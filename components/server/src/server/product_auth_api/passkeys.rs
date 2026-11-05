//! Passkey registration, assertion, external-browser ceremonies,
//! re-authentication and removal.

use super::*;

pub(in crate::server) async fn api_auth_list_passkeys(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    if !state.product_authentication.passkeys.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let user = match require_fresh_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let passkeys = match store.list_user_passkeys(&user.id).await {
        Ok(passkeys) => passkeys,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    let reauth_after_ms = store
        .user_passkey_policy(&user.id)
        .await
        .ok()
        .flatten()
        .map_or(crate::passkey::DEFAULT_PASSKEY_REAUTH_AFTER_MS, |policy| {
            policy
                .reauth_after_ms
                .min(state.product_authentication.session.passkey_max_age_ms)
        });
    Json(serde_json::json!({
        "passkeys": passkeys
            .into_iter()
            .map(|passkey| crate::passkey::PasskeyView {
                id: passkey.id,
                nickname: passkey.nickname,
                created_at_ms: passkey.created_at_ms,
                last_used_at_ms: passkey.last_used_at_ms,
            })
            .collect::<Vec<_>>(),
        "reauth_after_ms": reauth_after_ms,
    }))
    .into_response()
}

pub(in crate::server) async fn api_auth_passkey_register_options(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::passkey::RegisterStartRequest>,
) -> Response {
    if !state.product_authentication.passkeys.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let user = match require_recent_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let nickname = match crate::passkey::normalize_nickname(&request.nickname) {
        Ok(nickname) => nickname,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let existing = match store.list_user_passkeys(&user.id).await {
        Ok(existing) => existing,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    let webauthn = match crate::passkey::webauthn_for_request(&headers) {
        Ok(webauthn) => webauthn,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    match state.passkeys.start_registration(
        &user.id,
        &user.username,
        nickname,
        &existing,
        &webauthn,
    ) {
        Ok((challenge_id, public_key)) => Json(serde_json::json!({
            "challenge_id": challenge_id,
            "publicKey": public_key,
        }))
        .into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

pub(in crate::server) async fn api_auth_passkey_register_complete(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::passkey::RegisterCompleteRequest>,
) -> Response {
    if !state.product_authentication.passkeys.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let user = match require_recent_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let webauthn = match crate::passkey::webauthn_for_request(&headers) {
        Ok(webauthn) => webauthn,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let (nickname, credential_id, passkey_json) = match state.passkeys.finish_registration(
        &user.id,
        &request.challenge_id,
        request.credential,
        &webauthn,
    ) {
        Ok(created) => created,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    match persist_product_passkey_registration(store, &user, nickname, credential_id, passkey_json)
        .await
    {
        Ok(passkey) => Json(passkey).into_response(),
        Err(response) => response,
    }
}

#[expect(
    clippy::result_large_err,
    reason = "HTTP rejection is an owned Axum response returned directly by its handler"
)]
pub(in crate::server) async fn persist_product_passkey_registration(
    store: &Store,
    user: &crate::store::ProductUser,
    nickname: String,
    credential_id: String,
    passkey_json: String,
) -> Result<crate::passkey::PasskeyView, Response> {
    let now = auth_now_ms();
    match store.list_user_passkeys(&user.id).await {
        Ok(existing) => {
            if let Some(existing) = existing
                .into_iter()
                .find(|passkey| passkey.credential_id == credential_id)
            {
                return Ok(crate::passkey::PasskeyView {
                    id: existing.id,
                    nickname: existing.nickname,
                    created_at_ms: existing.created_at_ms,
                    last_used_at_ms: existing.last_used_at_ms,
                });
            }
        }
        Err(error) => {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response());
        }
    }
    let passkey = crate::passkey::UserPasskey {
        id: match crate::product_auth::new_user_id() {
            Ok(id) => id,
            Err(error) => {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response());
            }
        },
        user_id: user.id.clone(),
        credential_id,
        nickname,
        passkey_json,
        created_at_ms: now,
        last_used_at_ms: Some(now),
    };
    if let Err(error) = store.insert_user_passkey(&passkey).await {
        return Err((StatusCode::CONFLICT, error.to_string()).into_response());
    }
    let _ = store.touch_user_last_step_up(&user.id, now).await;
    Ok(crate::passkey::PasskeyView {
        id: passkey.id,
        nickname: passkey.nickname,
        created_at_ms: passkey.created_at_ms,
        last_used_at_ms: passkey.last_used_at_ms,
    })
}

pub(in crate::server) async fn api_auth_passkey_assert_options(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !state.product_authentication.passkeys.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let user = match require_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let existing = match store.list_user_passkeys(&user.id).await {
        Ok(existing) => existing,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    let webauthn = match crate::passkey::webauthn_for_request(&headers) {
        Ok(webauthn) => webauthn,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    match state
        .passkeys
        .start_assertion(&user.id, &existing, &webauthn)
    {
        Ok((challenge_id, public_key)) => Json(serde_json::json!({
            "challenge_id": challenge_id,
            "publicKey": public_key,
        }))
        .into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

pub(in crate::server) async fn api_auth_passkey_assert_complete(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::passkey::AssertCompleteRequest>,
) -> Response {
    if !state.product_authentication.passkeys.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let user = match require_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let existing = match store.list_user_passkeys(&user.id).await {
        Ok(existing) => existing,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    let webauthn = match crate::passkey::webauthn_for_request(&headers) {
        Ok(webauthn) => webauthn,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let (passkey_id, passkey_json) = match state.passkeys.finish_assertion(
        &user.id,
        &request.challenge_id,
        request.credential,
        &existing,
        &webauthn,
    ) {
        Ok(done) => done,
        Err(error) => return (StatusCode::UNAUTHORIZED, error.to_string()).into_response(),
    };
    match persist_product_passkey_assertion(
        &state,
        &headers,
        store,
        &user,
        passkey_id,
        passkey_json,
    )
    .await
    {
        Ok(result) => product_passkey_assertion_response(result),
        Err(response) => response,
    }
}

#[expect(
    clippy::result_large_err,
    reason = "HTTP rejection is an owned Axum response returned directly by its handler"
)]
pub(in crate::server) async fn persist_product_passkey_assertion(
    state: &ProductAuthState,
    headers: &HeaderMap,
    store: &Store,
    user: &crate::store::ProductUser,
    passkey_id: String,
    passkey_json: String,
) -> Result<ProductPasskeyAssertionResult, Response> {
    let now = auth_now_ms();
    if let Err(error) = store
        .update_user_passkey(&user.id, &passkey_id, &passkey_json, now)
        .await
    {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response());
    }
    let _ = store.touch_user_last_step_up(&user.id, now).await;
    let policy = store
        .user_passkey_policy(&user.id)
        .await
        .map_err(product_session_policy_unavailable)?
        .unwrap_or(crate::passkey::PasskeyPolicy {
            enabled: false,
            reauth_after_ms: crate::passkey::DEFAULT_PASSKEY_REAUTH_AFTER_MS,
            last_step_up_at_ms: None,
            passkey_count: 0,
        });
    if state
        .product_authentication
        .passkeys
        .session_refresh_enabled
        && policy.enabled
    {
        let Some(previous_token) = crate::product_auth::user_cookie_token(headers) else {
            return Err(StatusCode::UNAUTHORIZED.into_response());
        };
        let previous_token_hash = crate::admin::hex_sha256(previous_token.as_bytes());
        let previous_session = match store.user_session_by_token_hash(&previous_token_hash).await {
            Ok(Some(session)) => session,
            Ok(None) => return Err(StatusCode::UNAUTHORIZED.into_response()),
            Err(error) => {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response());
            }
        };
        let token = match crate::product_auth::new_session_token() {
            Ok(token) => token,
            Err(error) => {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response());
            }
        };
        let primary_due_at_ms = previous_session
            .primary_authenticated_at_ms
            .saturating_add(state.product_authentication.session.primary_max_age_ms);
        // Passkey rotates the current secret and refreshes only the local
        // user-verification proof. It can never move the primary-login cap.
        let expires_at_ms = primary_due_at_ms;
        let session = crate::store::ProductUserSession {
            token_hash: crate::admin::hex_sha256(token.as_bytes()),
            session_id: previous_session.session_id.clone(),
            user_id: user.id.clone(),
            client_kind: previous_session.client_kind.clone(),
            principal_class: previous_session.principal_class.clone(),
            created_at_ms: now,
            expires_at_ms,
            last_seen_at_ms: now,
            user_agent: headers
                .get(header::USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned),
            passkey_verified_at_ms: Some(now),
            primary_authenticated_at_ms: previous_session.primary_authenticated_at_ms,
            primary_auth_method: previous_session.primary_auth_method.clone(),
            revoked_at_ms: None,
            revoke_reason: None,
            auth_provider_id: previous_session.auth_provider_id.clone(),
            auth_issuer: previous_session.auth_issuer.clone(),
            auth_subject: previous_session.auth_subject.clone(),
            auth_sid: previous_session.auth_sid.clone(),
            id_token_ciphertext: previous_session.id_token_ciphertext.clone(),
        };
        if let Err(error) = store
            .rotate_user_session(&previous_token_hash, &session)
            .await
        {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response());
        }
        let me = product_me_for_user_with_policy(
            &state.hub,
            &state.product_authentication,
            user,
            Some(&session),
            &policy,
        );
        return Ok(ProductPasskeyAssertionResult {
            me,
            set_cookie: Some(crate::product_auth::session_cookie(
                &token,
                crate::product_auth::request_is_https(headers),
                expires_at_ms.saturating_sub(now) / 1_000,
            )),
        });
    }
    let current_session = match crate::product_auth::user_cookie_token(headers) {
        Some(token) => store
            .user_session_by_token_hash(&crate::admin::hex_sha256(token.as_bytes()))
            .await
            .ok()
            .flatten(),
        None => None,
    };
    let me = product_me_for_user_with_policy(
        &state.hub,
        &state.product_authentication,
        user,
        current_session.as_ref(),
        &policy,
    );
    Ok(ProductPasskeyAssertionResult {
        me,
        set_cookie: None,
    })
}

pub(in crate::server) fn product_passkey_assertion_response(
    result: ProductPasskeyAssertionResult,
) -> Response {
    match result.set_cookie {
        Some(cookie) => ([(header::SET_COOKIE, cookie)], Json(result.me)).into_response(),
        None => Json(result.me).into_response(),
    }
}

pub(in crate::server) async fn api_auth_passkey_external_start(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::passkey::ExternalStartRequest>,
) -> Response {
    if !state.product_authentication.passkeys.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let Some((session, user)) = product_session_and_user_from_store_cookie(store, &headers).await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if request.action == crate::passkey::ExternalPasskeyAction::Register {
        if let Err(response) =
            ensure_product_session_fresh(store, &session, &state.product_authentication).await
        {
            return response;
        }
        if !product_session_has_recent_step_up(&session, auth_now_ms()) {
            return (
                StatusCode::PRECONDITION_REQUIRED,
                "Recent login or Passkey verification required",
            )
                .into_response();
        }
    }
    let existing = match store.list_user_passkeys(&user.id).await {
        Ok(existing) => existing,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    let webauthn = match crate::passkey::webauthn_for_request(&headers) {
        Ok(webauthn) => webauthn,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let transaction_id = match request.action {
        crate::passkey::ExternalPasskeyAction::Register => {
            let Some(nickname) = request.nickname else {
                return (StatusCode::BAD_REQUEST, "passkey name is required").into_response();
            };
            let nickname = match crate::passkey::normalize_nickname(&nickname) {
                Ok(nickname) => nickname,
                Err(error) => {
                    return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
                }
            };
            state.passkeys.start_external_registration(
                crate::passkey::ExternalPasskeyBinding {
                    user_id: &user.id,
                    session_id: &session.session_id,
                    session_token_hash: &session.token_hash,
                    code_challenge: &request.code_challenge,
                },
                &user.username,
                nickname,
                &existing,
                &webauthn,
            )
        }
        crate::passkey::ExternalPasskeyAction::Assert => {
            if request.nickname.is_some() {
                return (StatusCode::BAD_REQUEST, "passkey name is not accepted").into_response();
            }
            state.passkeys.start_external_assertion(
                crate::passkey::ExternalPasskeyBinding {
                    user_id: &user.id,
                    session_id: &session.session_id,
                    session_token_hash: &session.token_hash,
                    code_challenge: &request.code_challenge,
                },
                &existing,
                &webauthn,
            )
        }
    };
    match transaction_id {
        Ok(transaction_id) => {
            if let Err(error) = persist_external_passkey_ceremony(&state, &transaction_id).await {
                let _ = state.passkeys.discard_external(&transaction_id);
                tracing::error!(%error, "external_passkey_start_persist");
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({
                        "code": "passkey_transaction_unavailable",
                        "message": "Cowboy could not start secure Passkey verification. Try again.",
                    })),
                )
                    .into_response();
            }
            Json(serde_json::json!({
                "transaction_id": transaction_id,
                "expires_in_seconds": crate::passkey::EXTERNAL_CEREMONY_TTL_SECS,
            }))
            .into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

pub(in crate::server) async fn api_auth_passkey_external_options(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::passkey::ExternalTransactionRequest>,
) -> Response {
    if !state.product_authentication.passkeys.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(rejected) = reject_bad_origin(&headers, peer_addr(peer), &state.public_origins) {
        return rejected;
    }
    if let Err(error) = restore_external_passkey_ceremony(&state, &request.transaction_id).await {
        return external_passkey_error_response(&error);
    }
    match state
        .passkeys
        .external_browser_state(&request.transaction_id)
    {
        Ok(crate::passkey::ExternalBrowserState::Ready { action, public_key }) => {
            Json(serde_json::json!({
                "status": "ready",
                "action": action,
                "publicKey": public_key,
            }))
            .into_response()
        }
        Ok(crate::passkey::ExternalBrowserState::Complete) => {
            Json(serde_json::json!({ "status": "complete" })).into_response()
        }
        Ok(crate::passkey::ExternalBrowserState::Failed) => {
            Json(serde_json::json!({ "status": "failed" })).into_response()
        }
        Err(error) => external_passkey_error_response(&error),
    }
}

pub(in crate::server) async fn api_auth_passkey_external_complete(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::passkey::ExternalCompleteRequest>,
) -> Response {
    if !state.product_authentication.passkeys.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(rejected) = reject_bad_origin(&headers, peer_addr(peer), &state.public_origins) {
        return rejected;
    }
    if let Err(error) = restore_external_passkey_ceremony(&state, &request.transaction_id).await {
        return external_passkey_error_response(&error);
    }
    if matches!(
        state
            .passkeys
            .external_browser_state(&request.transaction_id),
        Ok(crate::passkey::ExternalBrowserState::Complete)
    ) {
        return Json(serde_json::json!({ "status": "complete" })).into_response();
    }
    let (action, user_id) = match state.passkeys.external_subject(&request.transaction_id) {
        Ok(subject) => subject,
        Err(error) => return external_passkey_error_response(&error),
    };
    let webauthn = match crate::passkey::webauthn_for_request(&headers) {
        Ok(webauthn) => webauthn,
        Err(_) => return (StatusCode::BAD_REQUEST, "Passkey origin is invalid").into_response(),
    };
    let completed = match action {
        crate::passkey::ExternalPasskeyAction::Register => {
            match serde_json::from_value::<webauthn_rs::prelude::RegisterPublicKeyCredential>(
                request.credential,
            ) {
                Ok(credential) => state.passkeys.complete_external_registration(
                    &request.transaction_id,
                    credential,
                    &webauthn,
                ),
                Err(error) => Err(error.into()),
            }
        }
        crate::passkey::ExternalPasskeyAction::Assert => {
            let Some(store) = durable_store(&state.store) else {
                return missing_store();
            };
            let existing = match store.list_user_passkeys(&user_id).await {
                Ok(existing) => existing,
                Err(error) => {
                    tracing::error!(%error, "external_passkey_list");
                    return StatusCode::SERVICE_UNAVAILABLE.into_response();
                }
            };
            match serde_json::from_value::<webauthn_rs::prelude::PublicKeyCredential>(
                request.credential,
            ) {
                Ok(credential) => state.passkeys.complete_external_assertion(
                    &request.transaction_id,
                    credential,
                    &existing,
                    &webauthn,
                ),
                Err(error) => Err(error.into()),
            }
        }
    };
    match completed {
        Ok(()) => {
            if let Err(error) =
                persist_external_passkey_ceremony(&state, &request.transaction_id).await
            {
                tracing::error!(%error, "external_passkey_complete_persist");
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            Json(serde_json::json!({ "status": "complete" })).into_response()
        }
        Err(error) => {
            tracing::warn!(%error, "external_passkey_rejected");
            (StatusCode::UNAUTHORIZED, "Passkey verification failed").into_response()
        }
    }
}

pub(in crate::server) async fn api_auth_passkey_external_fail(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::passkey::ExternalTransactionRequest>,
) -> Response {
    if !state.product_authentication.passkeys.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(rejected) = reject_bad_origin(&headers, peer_addr(peer), &state.public_origins) {
        return rejected;
    }
    if let Err(error) = restore_external_passkey_ceremony(&state, &request.transaction_id).await {
        return external_passkey_error_response(&error);
    }
    match state.passkeys.fail_external(&request.transaction_id) {
        Ok(()) => {
            if let Err(error) =
                persist_external_passkey_ceremony(&state, &request.transaction_id).await
            {
                tracing::error!(%error, "external_passkey_fail_persist");
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Err(error) => external_passkey_error_response(&error),
    }
}

pub(in crate::server) async fn api_auth_passkey_external_events(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !state.product_authentication.passkeys.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let Some((session, user)) = product_session_and_user_from_store_cookie(store, &headers).await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    ws.max_message_size(4 * 1_024)
        .max_frame_size(4 * 1_024)
        .on_upgrade(move |socket| {
            handle_passkey_external_events(
                socket,
                state,
                user.id,
                session.session_id,
                session.token_hash,
            )
        })
}

pub(in crate::server) async fn handle_passkey_external_events(
    mut socket: WebSocket,
    state: ProductAuthState,
    user_id: String,
    session_id: String,
    session_token_hash: String,
) {
    let request = match tokio::time::timeout(
        PASSKEY_EXTERNAL_EVENTS_HANDSHAKE_TIMEOUT,
        socket.recv(),
    )
    .await
    {
        Ok(Some(Ok(Message::Text(text)))) => {
            serde_json::from_str::<crate::passkey::ExternalFinalizeRequest>(text.as_str()).ok()
        }
        _ => None,
    };
    let Some(request) = request else {
        let _ = socket.close().await;
        return;
    };
    if let Err(error) = restore_external_passkey_ceremony(&state, &request.transaction_id).await {
        tracing::info!(%error, "external_passkey_events_restore_rejected");
        let _ = send_json(
            &mut socket,
            &PasskeyExternalEventStatus {
                status: "unavailable",
            },
        )
        .await;
        let _ = socket.close().await;
        return;
    }
    let mut events = match state.passkeys.subscribe_external(
        &request.transaction_id,
        &user_id,
        &session_id,
        &session_token_hash,
        &request.code_verifier,
    ) {
        Ok(events) => events,
        Err(error) => {
            tracing::info!(%error, "external_passkey_events_rejected");
            let _ = send_json(
                &mut socket,
                &PasskeyExternalEventStatus {
                    status: "unavailable",
                },
            )
            .await;
            let _ = socket.close().await;
            return;
        }
    };
    let deadline = tokio::time::sleep(PASSKEY_EXTERNAL_EVENTS_TTL);
    tokio::pin!(deadline);
    let mut heartbeat = tokio::time::interval(PASSKEY_EXTERNAL_EVENTS_HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await;

    loop {
        let terminal = match *events.borrow_and_update() {
            crate::passkey::ExternalPasskeyEvent::Pending => None,
            crate::passkey::ExternalPasskeyEvent::Complete => Some("complete"),
            crate::passkey::ExternalPasskeyEvent::Failed => Some("failed"),
        };
        if let Some(status) = terminal {
            let _ = send_json(&mut socket, &PasskeyExternalEventStatus { status }).await;
            let _ = socket.close().await;
            return;
        }

        tokio::select! {
            changed = events.changed() => {
                if changed.is_err() {
                    let _ = send_json(
                        &mut socket,
                        &PasskeyExternalEventStatus { status: "unavailable" },
                    ).await;
                    let _ = socket.close().await;
                    return;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(Message::Ping(payload))) => {
                    if socket.send(Message::Pong(payload)).await.is_err() {
                        return;
                    }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(_)) => {
                    let _ = socket.close().await;
                    return;
                }
            },
            _ = heartbeat.tick() => {
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return;
                }
            }
            () = &mut deadline => {
                let _ = send_json(
                    &mut socket,
                    &PasskeyExternalEventStatus { status: "unavailable" },
                ).await;
                let _ = socket.close().await;
                return;
            }
        }
    }
}

pub(in crate::server) async fn api_auth_passkey_external_finalize(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::passkey::ExternalFinalizeRequest>,
) -> Response {
    if !state.product_authentication.passkeys.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(rejected) = reject_bad_origin(&headers, peer_addr(peer), &state.public_origins) {
        return rejected;
    }
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let Some((session, user)) = product_session_and_user_from_store_cookie(store, &headers).await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if let Err(error) = restore_external_passkey_ceremony(&state, &request.transaction_id).await {
        return external_passkey_error_response(&error);
    }
    let outcome = match state.passkeys.finalize_external(
        &request.transaction_id,
        &user.id,
        &session.session_id,
        &session.token_hash,
        &request.code_verifier,
    ) {
        Ok(outcome) => outcome,
        Err(error) => return external_passkey_error_response(&error),
    };
    match outcome {
        crate::passkey::ExternalFinalizeResult::Pending => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "status": "pending" })),
        )
            .into_response(),
        crate::passkey::ExternalFinalizeResult::Failed => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "code": "passkey_transaction_cancelled",
                "message": "Passkey verification was cancelled.",
            })),
        )
            .into_response(),
        crate::passkey::ExternalFinalizeResult::Registration {
            nickname,
            credential_id,
            passkey_json,
        } => match persist_product_passkey_registration(
            store,
            &user,
            nickname,
            credential_id,
            passkey_json,
        )
        .await
        {
            Ok(passkey) => {
                let response = serde_json::json!({
                    "status": "complete",
                    "passkey": passkey,
                });
                if let Err(error) = state.passkeys.mark_external_applied(
                    &request.transaction_id,
                    crate::passkey::ExternalPasskeyAction::Register,
                    response.clone(),
                ) {
                    tracing::error!(%error, "external_passkey_registration_mark_applied");
                } else if let Err(error) =
                    persist_external_passkey_ceremony(&state, &request.transaction_id).await
                {
                    tracing::error!(%error, "external_passkey_registration_result_persist");
                }
                Json(response).into_response()
            }
            Err(response) => response,
        },
        crate::passkey::ExternalFinalizeResult::Assertion {
            passkey_id,
            passkey_json,
        } => match persist_product_passkey_assertion(
            &state,
            &headers,
            store,
            &user,
            passkey_id,
            passkey_json,
        )
        .await
        {
            Ok(result) => {
                let response = serde_json::json!({
                    "status": "complete",
                    "me": result.me,
                });
                if let Err(error) = state.passkeys.mark_external_applied(
                    &request.transaction_id,
                    crate::passkey::ExternalPasskeyAction::Assert,
                    response.clone(),
                ) {
                    tracing::error!(%error, "external_passkey_assertion_mark_applied");
                } else if let Err(error) =
                    persist_external_passkey_ceremony(&state, &request.transaction_id).await
                {
                    tracing::error!(%error, "external_passkey_assertion_result_persist");
                }
                let body = Json(response);
                match result.set_cookie {
                    Some(cookie) => ([(header::SET_COOKIE, cookie)], body).into_response(),
                    None => body.into_response(),
                }
            }
            Err(response) => response,
        },
        crate::passkey::ExternalFinalizeResult::Applied { response } => {
            Json(response).into_response()
        }
    }
}

pub(in crate::server) async fn api_auth_passkey_reauth(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::passkey::ReauthSettingRequest>,
) -> Response {
    if !state.product_authentication.passkeys.enabled
        || !state
            .product_authentication
            .passkeys
            .session_refresh_enabled
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let user = match require_fresh_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let current = match store.user_passkey_policy(&user.id).await {
        Ok(Some(policy)) => policy,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    let reauth_after_ms = request.reauth_after_ms.unwrap_or(current.reauth_after_ms);
    if !crate::passkey::valid_reauth_interval(reauth_after_ms) {
        return (
            StatusCode::BAD_REQUEST,
            "verification frequency is unsupported",
        )
            .into_response();
    }
    if reauth_after_ms > state.product_authentication.session.passkey_max_age_ms {
        return (
            StatusCode::BAD_REQUEST,
            "verification frequency exceeds the Cowboy Service maximum",
        )
            .into_response();
    }
    if request.enabled && current.passkey_count == 0 {
        return (
            StatusCode::BAD_REQUEST,
            "add a Passkey before enabling refresh",
        )
            .into_response();
    }
    if let Err(error) = store
        .set_user_passkey_reauth(&user.id, request.enabled, reauth_after_ms)
        .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    let current_session = match crate::product_auth::user_cookie_token(&headers) {
        Some(token) => store
            .user_session_by_token_hash(&crate::admin::hex_sha256(token.as_bytes()))
            .await
            .ok()
            .flatten(),
        None => None,
    };
    match product_me_for_user(
        Some(store),
        &state.hub,
        &state.product_authentication,
        &user,
        current_session.as_ref(),
    )
    .await
    {
        Ok(me) => Json(me).into_response(),
        Err(error) => product_session_policy_unavailable(error),
    }
}

pub(in crate::server) async fn api_auth_delete_passkey(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !state.product_authentication.passkeys.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let user = match require_recent_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    match store.delete_user_passkey(&user.id, &id).await {
        Ok(0) => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}
