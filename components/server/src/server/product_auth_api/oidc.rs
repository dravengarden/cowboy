//! OIDC browser and native sign-in: start, callback, native handoff exchange,
//! events, polling and completion.

use super::*;

pub(in crate::server) async fn api_auth_oidc_start(
    State(state): State<ProductAuthState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    uri: Uri,
    headers: HeaderMap,
    Query(query): Query<OidcStartQuery>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let provider_id = authentication_provider_id(uri.path(), "start").unwrap_or("cardea");
    let Some(provider) = state
        .product_authentication
        .provider(provider_id)
        .map(AsRef::as_ref)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let target = match (
        query.client.as_deref(),
        query.code_challenge,
        query.handoff_challenge,
    ) {
        (None, None, None) => crate::oidc::AuthorizationTarget::Browser,
        (Some("macos-manager"), Some(code_challenge), None) => {
            crate::oidc::AuthorizationTarget::MacOs { code_challenge }
        }
        (Some("browser-shell"), Some(code_challenge), Some(handoff_challenge)) => {
            crate::oidc::AuthorizationTarget::BrowserShell {
                code_challenge,
                handoff_challenge,
            }
        }
        _ => return (StatusCode::BAD_REQUEST, "invalid OIDC client request").into_response(),
    };
    if instance_needs_setup(&state).await {
        return (StatusCode::CONFLICT, "complete Cowboy account setup first").into_response();
    }
    let user = match store.user_by_username(provider.account()).await {
        Ok(Some(user)) if user.disabled_at_ms.is_none() => user,
        Ok(_) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(error) => {
            tracing::error!(%error, "oidc_account_lookup");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if let Some(admin_account) = provider.admin_account()
        && !admin_identities(&state.hub)
            .accounts
            .iter()
            .any(|account| account.account == admin_account)
    {
        tracing::error!(admin_account, "oidc_admin_mapping_unavailable");
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let source_ip = crate::product_auth::client_ip(&headers, peer);
    let rate_key = format!("oidc:{provider_id}:start");
    let secure = crate::product_auth::request_is_https(&headers);
    if provider.requires_cross_site_post_cookie() && !secure {
        return (
            StatusCode::BAD_REQUEST,
            "form_post authentication requires HTTPS",
        )
            .into_response();
    }
    apply_rate_limit(&state, &rate_key, &source_ip.to_string()).await;
    let started = match state
        .oidc_transactions
        .begin(provider, source_ip, target.clone())
        .await
    {
        Ok(started) => started,
        Err(error) => {
            tracing::info!(%error, user_id = user.id, "oidc_start_rejected");
            return (StatusCode::TOO_MANY_REQUESTS, "try again later").into_response();
        }
    };
    if let crate::oidc::AuthorizationTarget::BrowserShell {
        code_challenge,
        handoff_challenge,
    } = &target
        && let Err(error) =
            state
                .oidc_native_handoffs
                .begin_browser(provider_id, code_challenge, handoff_challenge)
    {
        state.oidc_transactions.cancel(&started.cookie_token);
        tracing::info!(%error, "oidc_browser_handoff_rejected");
        return (StatusCode::BAD_REQUEST, "invalid OIDC client request").into_response();
    }
    state.rate_limits.reset(&rate_key, &source_ip.to_string());
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, started.location),
            (
                header::SET_COOKIE,
                crate::oidc::transaction_cookie(
                    &started.cookie_token,
                    secure,
                    provider.requires_cross_site_post_cookie(),
                ),
            ),
        ],
    )
        .into_response()
}

pub(in crate::server) async fn api_auth_oidc_callback(
    State(state): State<ProductAuthState>,
    uri: Uri,
    headers: HeaderMap,
    Query(query): Query<OidcCallbackQuery>,
) -> Response {
    api_auth_oidc_callback_inner(state, uri, headers, query).await
}

pub(in crate::server) async fn api_auth_oidc_callback_form(
    State(state): State<ProductAuthState>,
    uri: Uri,
    headers: HeaderMap,
    Form(query): Form<OidcCallbackQuery>,
) -> Response {
    api_auth_oidc_callback_inner(state, uri, headers, query).await
}

pub(in crate::server) async fn api_auth_oidc_callback_inner(
    state: ProductAuthState,
    uri: Uri,
    headers: HeaderMap,
    query: OidcCallbackQuery,
) -> Response {
    let secure = crate::product_auth::request_is_https(&headers);
    if !state.product_auth_enabled {
        return oidc_callback_error(StatusCode::NOT_FOUND, secure);
    }
    let provider_id = authentication_provider_id(uri.path(), "callback").unwrap_or("cardea");
    let Some(provider) = state
        .product_authentication
        .provider(provider_id)
        .map(AsRef::as_ref)
    else {
        return oidc_callback_error(StatusCode::NOT_FOUND, secure);
    };
    let Some(cookie) = crate::product_auth::cookie_value(&headers, crate::oidc::TRANSACTION_COOKIE)
    else {
        return oidc_callback_error(StatusCode::UNAUTHORIZED, secure);
    };
    let Some(returned_state) = query.state.as_deref() else {
        return oidc_callback_error(StatusCode::UNAUTHORIZED, secure);
    };
    let transaction = match state
        .oidc_transactions
        .consume(provider_id, &cookie, returned_state)
    {
        Ok(transaction) => transaction,
        Err(error) => {
            tracing::info!(%error, "oidc_callback_rejected");
            return oidc_callback_error(StatusCode::UNAUTHORIZED, secure);
        }
    };
    let target = transaction.target().clone();
    if query.user.as_ref().is_some_and(|user| user.len() > 8_192) {
        return oidc_callback_target_error(
            &state,
            provider_id,
            StatusCode::BAD_REQUEST,
            secure,
            &target,
        );
    }
    if query.error.is_some() || query.error_description.is_some() {
        tracing::info!("oidc_authorization_denied");
        return oidc_callback_target_error(
            &state,
            provider_id,
            StatusCode::UNAUTHORIZED,
            secure,
            &target,
        );
    }
    let Some(code) = query.code.as_deref() else {
        return oidc_callback_target_error(
            &state,
            provider_id,
            StatusCode::UNAUTHORIZED,
            secure,
            &target,
        );
    };
    let identity = match provider.exchange(&transaction, code).await {
        Ok(identity) => identity,
        Err(error) => {
            tracing::info!(%error, "oidc_exchange_rejected");
            return oidc_callback_target_error(
                &state,
                provider_id,
                StatusCode::UNAUTHORIZED,
                secure,
                &target,
            );
        }
    };
    let Some(store) = durable_store(&state.store).cloned() else {
        return oidc_callback_target_error(
            &state,
            provider_id,
            StatusCode::SERVICE_UNAVAILABLE,
            secure,
            &target,
        );
    };
    let user = match store.user_by_username(provider.account()).await {
        Ok(Some(user)) if user.disabled_at_ms.is_none() => user,
        Ok(_) => {
            return oidc_callback_target_error(
                &state,
                provider_id,
                StatusCode::UNAUTHORIZED,
                secure,
                &target,
            );
        }
        Err(error) => {
            tracing::error!(%error, "oidc_account_lookup");
            return oidc_callback_target_error(
                &state,
                provider_id,
                StatusCode::INTERNAL_SERVER_ERROR,
                secure,
                &target,
            );
        }
    };
    tracing::info!(
        provider_id,
        username = user.username,
        issuer = identity.issuer,
        subject = identity.subject,
        issued_at = identity.issued_at,
        authenticated_at = ?identity.authenticated_at,
        authentication_context = ?identity.authentication_context,
        authentication_methods = ?identity.authentication_methods,
        ok = true,
        "product_oidc_login"
    );
    match &target {
        crate::oidc::AuthorizationTarget::MacOs { code_challenge } => {
            let handoff = match state.oidc_native_handoffs.issue(
                provider_id,
                &user.id,
                code_challenge,
                &identity,
            ) {
                Ok(handoff) => handoff,
                Err(error) => {
                    tracing::error!(%error, "oidc_native_handoff_issue");
                    return oidc_callback_error(StatusCode::INTERNAL_SERVER_ERROR, secure);
                }
            };
            return (
                StatusCode::SEE_OTHER,
                [
                    (header::LOCATION, handoff.location),
                    (
                        header::SET_COOKIE,
                        crate::oidc::clear_transaction_cookie(secure),
                    ),
                ],
            )
                .into_response();
        }
        crate::oidc::AuthorizationTarget::BrowserShell {
            handoff_challenge, ..
        } => {
            if let Err(error) = state.oidc_native_handoffs.complete_browser(
                provider_id,
                handoff_challenge,
                &user.id,
                &identity,
            ) {
                tracing::error!(%error, "oidc_browser_handoff_complete");
                return oidc_callback_error(StatusCode::INTERNAL_SERVER_ERROR, secure);
            }
            return oidc_browser_shell_completion_response(secure);
        }
        crate::oidc::AuthorizationTarget::Browser => {}
    }
    let admin_cookie = match oidc_admin_session_cookie(&state, provider, &headers) {
        Ok(cookie) => cookie,
        Err(error) => {
            tracing::error!(%error, "oidc_admin_session_issue");
            return oidc_callback_error(StatusCode::SERVICE_UNAVAILABLE, secure);
        }
    };
    let mut response = issue_product_session(
        &state,
        &store,
        &user,
        &headers,
        provider_id,
        Some(&identity),
    )
    .await;
    if let Ok(value) = crate::oidc::clear_transaction_cookie(secure).parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    if !response.status().is_success() {
        return response;
    }
    *response.status_mut() = StatusCode::SEE_OTHER;
    response
        .headers_mut()
        .insert(header::LOCATION, header::HeaderValue::from_static("/"));
    if let Some(cookie) = admin_cookie
        && let Ok(value) = cookie.parse()
    {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

pub(in crate::server) fn oidc_callback_target_error(
    state: &ProductAuthState,
    provider_id: &str,
    status: StatusCode,
    secure: bool,
    target: &crate::oidc::AuthorizationTarget,
) -> Response {
    if matches!(target, crate::oidc::AuthorizationTarget::MacOs { .. }) {
        return (
            StatusCode::SEE_OTHER,
            [
                (
                    header::LOCATION,
                    crate::oidc::native_error_location().to_owned(),
                ),
                (
                    header::SET_COOKIE,
                    crate::oidc::clear_transaction_cookie(secure),
                ),
            ],
        )
            .into_response();
    }
    if let crate::oidc::AuthorizationTarget::BrowserShell {
        handoff_challenge, ..
    } = target
    {
        if let Err(error) = state
            .oidc_native_handoffs
            .fail_browser(provider_id, handoff_challenge)
        {
            tracing::info!(%error, "oidc_browser_handoff_fail");
        }
        return oidc_browser_shell_completion_response(secure);
    }
    oidc_callback_error(status, secure)
}

pub(in crate::server) fn oidc_browser_shell_completion_response(secure: bool) -> Response {
    (
        StatusCode::SEE_OTHER,
        [
            (
                header::LOCATION,
                "/api/auth/oidc/native/complete".to_owned(),
            ),
            (
                header::SET_COOKIE,
                crate::oidc::clear_transaction_cookie(secure),
            ),
        ],
    )
        .into_response()
}

pub(in crate::server) fn oidc_admin_session_cookie(
    state: &ProductAuthState,
    provider: &crate::oidc::OidcProvider,
    headers: &HeaderMap,
) -> anyhow::Result<Option<String>> {
    let Some(account) = provider.admin_account() else {
        return Ok(None);
    };
    let mut identities = admin_identities(&state.hub);
    let token = identities.login_federated(account, auth_now_ms())?;
    persist_admin_identities(&state.hub, &identities);
    Ok(Some(crate::admin::session_cookie(
        &token,
        crate::product_auth::request_is_https(headers),
    )))
}

pub(in crate::server) async fn api_auth_oidc_native_exchange(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    uri: Uri,
    headers: HeaderMap,
    Json(request): Json<OidcNativeExchangeRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let provider_id = authentication_provider_id(uri.path(), "native/exchange").unwrap_or("cardea");
    let Some(provider) = state
        .product_authentication
        .provider(provider_id)
        .map(AsRef::as_ref)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    if provider.admin_account().is_some()
        && let Some(rejected) = reject_insecure_admin(&headers, peer)
    {
        return rejected;
    }
    let Some(store) = durable_store(&state.store).cloned() else {
        return missing_store();
    };
    let source_ip = crate::product_auth::client_ip(&headers, peer).to_string();
    apply_rate_limit(&state, "oidc:native", &source_ip).await;
    let (user_id, identity) =
        match state
            .oidc_native_handoffs
            .consume(provider_id, &request.code, &request.code_verifier)
        {
            Ok(handoff) => handoff,
            Err(error) => {
                state.rate_limits.record_failure("oidc:native", &source_ip);
                tracing::info!(%error, "oidc_native_exchange_rejected");
                return (StatusCode::UNAUTHORIZED, "native authorization failed").into_response();
            }
        };
    let user = match store.user_by_id(&user_id).await {
        Ok(Some(user)) if user.disabled_at_ms.is_none() && user.username == provider.account() => {
            user
        }
        Ok(_) => {
            state.rate_limits.record_failure("oidc:native", &source_ip);
            return (StatusCode::UNAUTHORIZED, "native authorization failed").into_response();
        }
        Err(error) => {
            tracing::error!(%error, "oidc_native_account_lookup");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let admin_cookie = match oidc_admin_session_cookie(&state, provider, &headers) {
        Ok(cookie) => cookie,
        Err(error) => {
            tracing::error!(%error, "oidc_native_admin_session_issue");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let mut response = issue_product_session(
        &state,
        &store,
        &user,
        &headers,
        provider_id,
        Some(&identity),
    )
    .await;
    if !response.status().is_success() {
        return response;
    }
    if let Some(cookie) = admin_cookie
        && let Ok(value) = cookie.parse()
    {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    state.rate_limits.reset("oidc:native", &source_ip);
    tracing::info!(username = user.username, ok = true, "oidc_native_exchange");
    response
}

pub(in crate::server) async fn api_auth_oidc_native_events(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    uri: Uri,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let provider_id = authentication_provider_id(uri.path(), "native/events")
        .unwrap_or("cardea")
        .to_owned();
    let Some(provider) = state.product_authentication.provider(&provider_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    if provider.admin_account().is_some()
        && let Some(rejected) = reject_insecure_admin(&headers, peer)
    {
        return rejected;
    }
    ws.max_message_size(4 * 1_024)
        .max_frame_size(4 * 1_024)
        .on_upgrade(move |socket| handle_oidc_native_events(socket, state, provider_id))
}

pub(in crate::server) async fn handle_oidc_native_events(
    mut socket: WebSocket,
    state: ProductAuthState,
    provider_id: String,
) {
    let request =
        match tokio::time::timeout(OIDC_NATIVE_EVENTS_HANDSHAKE_TIMEOUT, socket.recv()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                serde_json::from_str::<OidcNativePollRequest>(text.as_str()).ok()
            }
            _ => None,
        };
    let Some(request) = request else {
        let _ = socket.close().await;
        return;
    };
    let mut events = match state.oidc_native_handoffs.subscribe_browser(
        &provider_id,
        &request.handoff_token,
        &request.code_verifier,
    ) {
        Ok(events) => events,
        Err(error) => {
            tracing::info!(%error, "oidc_browser_handoff_events_rejected");
            let _ = send_json(
                &mut socket,
                &OidcNativeEventStatus {
                    status: "unavailable",
                },
            )
            .await;
            let _ = socket.close().await;
            return;
        }
    };
    let deadline = tokio::time::sleep(OIDC_NATIVE_EVENTS_TTL);
    tokio::pin!(deadline);
    let mut heartbeat = tokio::time::interval(OIDC_NATIVE_EVENTS_HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await;

    loop {
        let status = *events.borrow_and_update();
        let terminal = match status {
            crate::oidc::BrowserHandoffEvent::Pending => None,
            crate::oidc::BrowserHandoffEvent::Ready => Some("ready"),
            crate::oidc::BrowserHandoffEvent::Failed => Some("failed"),
        };
        if let Some(status) = terminal {
            let _ = send_json(&mut socket, &OidcNativeEventStatus { status }).await;
            let _ = socket.close().await;
            return;
        }

        tokio::select! {
            changed = events.changed() => {
                if changed.is_err() {
                    let _ = send_json(
                        &mut socket,
                        &OidcNativeEventStatus { status: "unavailable" },
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
                    &OidcNativeEventStatus { status: "unavailable" },
                ).await;
                let _ = socket.close().await;
                return;
            }
        }
    }
}

pub(in crate::server) async fn api_auth_oidc_native_poll(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    uri: Uri,
    headers: HeaderMap,
    Json(request): Json<OidcNativePollRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let provider_id = authentication_provider_id(uri.path(), "native/poll").unwrap_or("cardea");
    let Some(provider) = state
        .product_authentication
        .provider(provider_id)
        .map(AsRef::as_ref)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    if provider.admin_account().is_some()
        && let Some(rejected) = reject_insecure_admin(&headers, peer)
    {
        return rejected;
    }
    let Some(store) = durable_store(&state.store).cloned() else {
        return missing_store();
    };
    let (user_id, identity) = match state.oidc_native_handoffs.poll_browser(
        provider_id,
        &request.handoff_token,
        &request.code_verifier,
    ) {
        Ok(crate::oidc::BrowserHandoffPoll::Pending) => {
            return (
                StatusCode::ACCEPTED,
                Json(OidcNativePollPending { status: "pending" }),
            )
                .into_response();
        }
        Ok(crate::oidc::BrowserHandoffPoll::Ready { user_id, identity }) => (user_id, identity),
        Ok(crate::oidc::BrowserHandoffPoll::Failed) => {
            return (StatusCode::GONE, "external authorization failed").into_response();
        }
        Err(error) => {
            tracing::info!(%error, "oidc_browser_handoff_poll_rejected");
            return (StatusCode::UNAUTHORIZED, "native authorization failed").into_response();
        }
    };
    let user = match store.user_by_id(&user_id).await {
        Ok(Some(user)) if user.disabled_at_ms.is_none() && user.username == provider.account() => {
            user
        }
        Ok(_) => {
            return (StatusCode::UNAUTHORIZED, "native authorization failed").into_response();
        }
        Err(error) => {
            tracing::error!(%error, "oidc_browser_handoff_account_lookup");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let admin_cookie = match oidc_admin_session_cookie(&state, provider, &headers) {
        Ok(cookie) => cookie,
        Err(error) => {
            tracing::error!(%error, "oidc_browser_handoff_admin_session_issue");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let mut response = issue_product_session(
        &state,
        &store,
        &user,
        &headers,
        provider_id,
        Some(&identity),
    )
    .await;
    if !response.status().is_success() {
        return response;
    }
    if let Some(cookie) = admin_cookie
        && let Ok(value) = cookie.parse()
    {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    tracing::info!(
        username = user.username,
        ok = true,
        "oidc_browser_handoff_complete"
    );
    response
}

pub(in crate::server) async fn api_auth_oidc_native_cancel(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    uri: Uri,
    headers: HeaderMap,
    Json(request): Json<OidcNativePollRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let provider_id = authentication_provider_id(uri.path(), "native/cancel").unwrap_or("cardea");
    let Some(provider) = state.product_authentication.provider(provider_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    if provider.admin_account().is_some()
        && let Some(rejected) = reject_insecure_admin(&headers, peer)
    {
        return rejected;
    }
    match state.oidc_native_handoffs.cancel_browser(
        provider_id,
        &request.handoff_token,
        &request.code_verifier,
    ) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => {
            tracing::info!(%error, "oidc_browser_handoff_cancel_rejected");
            (StatusCode::UNAUTHORIZED, "native authorization failed").into_response()
        }
    }
}

pub(in crate::server) async fn api_auth_oidc_native_complete() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .header("referrer-policy", "no-referrer")
        .header("x-content-type-options", "nosniff")
        .header("x-frame-options", "DENY")
        .header("content-security-policy", NATIVE_COMPLETE_CSP.as_str())
        .body(Body::from(NATIVE_COMPLETE_PAGE.as_str()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

pub(in crate::server) fn oidc_callback_error(status: StatusCode, secure: bool) -> Response {
    (
        status,
        [(
            header::SET_COOKIE,
            crate::oidc::clear_transaction_cookie(secure),
        )],
        "External login could not be completed",
    )
        .into_response()
}
