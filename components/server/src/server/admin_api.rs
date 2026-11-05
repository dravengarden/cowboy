//! The /api/admin console handlers: setup, accounts, registration, permissions,
//! session limits, provider and plugin administration and admin passkeys.

use super::*;

pub(super) fn admin_identities(hub: &Hub) -> crate::admin::AdminIdentities {
    crate::admin::AdminIdentities::from_setting(
        hub.settings_snapshot()
            .get(crate::admin::ADMIN_IDENTITIES_SETTING),
    )
}

pub(super) async fn admin_session_response(
    store: Option<&Store>,
    hub: &Hub,
    headers: &HeaderMap,
    identities: crate::admin::AdminIdentities,
    token: String,
) -> Response {
    persist_admin_identities(hub, &identities);
    let secure = crate::product_auth::request_is_https(headers);
    let status = enrich_admin_status(
        store,
        &identities,
        identities.status(Some(&token), auth_now_ms()),
    )
    .await;
    (
        [(
            header::SET_COOKIE,
            crate::admin::session_cookie(&token, secure),
        )],
        Json(status),
    )
        .into_response()
}

pub(super) async fn api_admin_auth(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Json<crate::admin::AdminAuthStatus> {
    let identities = admin_identities(&state.hub);
    let mut status = identities.status(
        crate::admin::cookie_token(&headers).as_deref(),
        auth_now_ms(),
    );
    status.setup_pending = status.bootstrap_required
        && crate::admin::setup_cookie_token(&headers)
            .is_some_and(|token| state.setup.tickets.is_valid(&token));
    Json(enrich_admin_status(state.store.as_ref(), &identities, status).await)
}

pub(super) async fn api_admin_setup(
    State(state): State<ProductAuthState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::admin::AdminSetupRequest>,
) -> Response {
    let _ = (state, peer, headers, request);
    (StatusCode::FORBIDDEN, "complete setup on /").into_response()
}

pub(super) async fn api_admin_bootstrap(
    State(state): State<ProductAuthState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::admin::AdminCredentials>,
) -> Response {
    let _ = (state, peer, headers, request);
    (StatusCode::FORBIDDEN, "complete setup on /").into_response()
}

pub(super) async fn api_admin_login(
    State(state): State<ProductAuthState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::admin::AdminCredentials>,
) -> Response {
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    if let Some(rejected) = reject_insecure_admin(&headers, peer) {
        return rejected;
    }
    let ip = crate::product_auth::client_ip(&headers, peer).to_string();
    let rate_name = format!("admin:{}", request.account.trim().to_ascii_lowercase());
    apply_rate_limit(&state, &rate_name, &ip).await;
    let mut identities = admin_identities(&state.hub);
    let now = auth_now_ms();
    match tokio::task::spawn_blocking(move || {
        let token = identities.login(&request, now)?;
        Ok::<_, anyhow::Error>((identities, token))
    })
    .await
    {
        Ok(Ok((identities, token))) => {
            state.rate_limits.reset(&rate_name, &ip);
            tracing::info!("admin_login ok=true");
            admin_session_response(
                state.store.as_ref(),
                &state.hub,
                &headers,
                identities,
                token,
            )
            .await
        }
        Ok(Err(_)) => {
            state.rate_limits.record_failure(&rate_name, &ip);
            tracing::info!("admin_login ok=false");
            (StatusCode::UNAUTHORIZED, "invalid admin credentials").into_response()
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) async fn api_admin_logout(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    let mut identities = admin_identities(&state.hub);
    if let Some(token) = crate::admin::cookie_token(&headers) {
        identities.logout(&token);
        persist_admin_identities(&state.hub, &identities);
    }
    let secure = crate::product_auth::request_is_https(&headers);
    (
        [(
            header::SET_COOKIE,
            crate::admin::clear_session_cookie(secure),
        )],
        Json(identities.status(None, auth_now_ms())),
    )
        .into_response()
}

pub(super) async fn api_admin_accounts(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        return status.into_response();
    }
    Json(serde_json::json!({
        "accounts": admin_identities(&state.hub)
            .accounts
            .into_iter()
            .map(|account| serde_json::json!({
                "account": account.account,
                "role": account.role,
                "created_at_ms": account.created_at_ms,
            }))
            .collect::<Vec<_>>(),
    }))
    .into_response()
}

pub(super) async fn api_admin_registration(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        return status.into_response();
    }
    Json(
        serde_json::to_value(registration_policy(&state.hub).public_view())
            .unwrap_or_else(|_| serde_json::json!({})),
    )
    .into_response()
}

pub(super) async fn api_admin_create_account(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::admin::AdminCredentials>,
) -> Response {
    let _ = (state, peer, headers, request);
    (StatusCode::FORBIDDEN, "this instance is single-user").into_response()
}

pub(super) async fn api_admin_overview(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        return status.into_response();
    }
    let healthy = match &state.runtime_health {
        Some(health) => {
            health.is_healthy(state.store.is_some())
                && state
                    .persistence_health
                    .as_ref()
                    .is_none_or(|health| health.is_healthy())
        }
        None => true,
    };
    let (events_rows, sessions_deleted) = match &state.store {
        Some(store) => store
            .observed_storage_metrics()
            .await
            .map(|(_, events, deleted)| (events, deleted))
            .unwrap_or((0, 0)),
        None => (i64::try_from(state.hub.event_total()).unwrap_or(0), 0),
    };
    let (runtime_workers, runtime_busy_workers) = state
        .runtime_router
        .as_ref()
        .map(|router| {
            let stats = router.stats();
            (stats.workers, stats.busy_workers)
        })
        .unwrap_or((0, 0));
    Json(AdminOverview {
        healthy,
        persistence: if !healthy && state.store.is_some() {
            "degraded"
        } else if state.store.is_some() {
            "durable"
        } else {
            "memory"
        },
        backend: if state.store.is_some() {
            "store"
        } else {
            "memory"
        },
        sessions_live: state.hub.session_list().len(),
        sessions_deleted,
        events_rows,
        daemon_rss_bytes: daemon_memory().rss_bytes,
        runtime_workers,
        runtime_busy_workers,
        registration: registration_policy(&state.hub).public_view(),
    })
    .into_response()
}

pub(super) async fn api_admin_sessions(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        return status.into_response();
    }
    let sessions = state
        .hub
        .session_list()
        .into_iter()
        .map(|session| AdminSessionRow {
            id: session.id,
            title: session.title,
            provider: session.provider,
            machine_id: session.machine_id,
            status: session.status,
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({ "sessions": sessions })).into_response()
}

pub(super) async fn api_admin_machines(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        return status.into_response();
    }
    let machines = if let Some(store) = state.store.as_ref() {
        match store.list_machines().await {
            Ok(machines) => machines,
            Err(error) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
            }
        }
    } else {
        Vec::new()
    };
    Json(serde_json::json!({ "machines": machines })).into_response()
}

pub(super) async fn api_admin_registration_put(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(patch): Json<crate::admin::RegistrationPolicyPatch>,
) -> Response {
    let _ = (state, peer, headers, patch);
    (StatusCode::FORBIDDEN, "this instance is single-user").into_response()
}

pub(super) async fn api_admin_registration_token(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::admin::CreateRegistrationToken>,
) -> Response {
    let _ = (state, peer, headers, request);
    (StatusCode::FORBIDDEN, "this instance is single-user").into_response()
}

pub(super) async fn api_admin_registration_token_delete(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    Path(token_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let _ = (state, peer, token_id, headers);
    (StatusCode::FORBIDDEN, "this instance is single-user").into_response()
}

pub(super) async fn api_admin_permissions(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        return status.into_response();
    }
    let policy = permission_policy(&state.hub);
    Json(serde_json::json!({
        "default_role": policy.default_role,
        "grants": policy.grants,
        "anonymous_role": policy.role_for(""),
    }))
    .into_response()
}

pub(super) async fn api_admin_permissions_put(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(patch): Json<crate::admin::PermissionPolicy>,
) -> Response {
    let _ = (state, peer, headers, patch);
    (StatusCode::FORBIDDEN, "this instance is single-user").into_response()
}

pub(super) async fn api_admin_session_limits(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        return status.into_response();
    }
    let limits = crate::admin::SessionLimits::from_setting(
        state
            .hub
            .settings_snapshot()
            .get(crate::admin::SESSION_LIMITS_SETTING),
    );
    Json(serde_json::json!({
        "max_sessions": limits.max_sessions,
        "max_retention_days": limits.max_retention_days,
        "last_n": limits.last_n,
        "last_time_hours": limits.last_time_hours,
        "keeps_latest": limits.keeps_event(0, 0),
        "or_rule": true,
    }))
    .into_response()
}

pub(super) async fn api_admin_session_limits_put(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(patch): Json<crate::admin::SessionLimits>,
) -> Response {
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Operator) {
        return status.into_response();
    }
    let mut limits = crate::admin::SessionLimits::from_setting(
        state
            .hub
            .settings_snapshot()
            .get(crate::admin::SESSION_LIMITS_SETTING),
    );
    if let Err(error) = limits.apply_patch(patch) {
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    persist_admin_setting(&state.hub, crate::admin::SESSION_LIMITS_SETTING, &limits);
    Json(limits).into_response()
}

pub(super) async fn api_admin_providers(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        return status.into_response();
    }
    let Some(catalog) = state.provider_catalog.as_ref() else {
        return Json(serde_json::json!({
            "providers": [],
            "catalog_root": serde_json::Value::Null,
        }))
        .into_response();
    };
    Json(serde_json::json!({
        "providers": catalog.entries(),
        "catalog_root": catalog.catalog_root(),
    }))
    .into_response()
}

pub(super) async fn api_admin_plugins(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        return status.into_response();
    }
    let Some(catalog) = state.plugin_catalog.as_ref() else {
        return Json(serde_json::json!({
            "plugins": [],
            "catalog_root": serde_json::Value::Null,
        }))
        .into_response();
    };
    Json(serde_json::json!({
        "plugins": catalog.entries(),
        "catalog_root": catalog.catalog_root(),
    }))
    .into_response()
}

pub(super) async fn api_admin_plugins_refresh(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Operator) {
        return status.into_response();
    }
    let Some(catalog) = state.plugin_catalog.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "plugin catalog unavailable",
        )
            .into_response();
    };
    let Some(storage) = state.plugin_storage.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "plugin runtime storage unavailable",
        )
            .into_response();
    };
    match catalog.refresh_with_runtime(storage).await {
        Ok(count) => {
            if let Some(providers) = state.provider_catalog.as_ref()
                && let Err(error) = providers.refresh_external()
            {
                return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
            }
            Json(serde_json::json!({ "external_releases": count })).into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

pub(super) async fn api_admin_providers_refresh(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Operator) {
        return status.into_response();
    }
    let Some(catalog) = state.provider_catalog.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "provider catalog unavailable",
        )
            .into_response();
    };
    let (Some(plugins), Some(storage)) =
        (state.plugin_catalog.as_ref(), state.plugin_storage.as_ref())
    else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "plugin runtime storage unavailable",
        )
            .into_response();
    };
    match plugins.refresh_with_runtime(storage).await {
        Ok(_) => match catalog.refresh_external() {
            Ok(count) => Json(serde_json::json!({ "external_releases": count })).into_response(),
            Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        },
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

pub(super) fn admin_passkey_subject(account: &str) -> String {
    format!("admin:{account}")
}

pub(super) async fn api_admin_list_passkeys(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    let principal = match require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        Ok(principal) => principal,
        Err(status) => return status.into_response(),
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    match store.list_admin_passkeys(&principal.account).await {
        Ok(passkeys) => Json(serde_json::json!({
            "passkeys": passkeys
                .into_iter()
                .map(|passkey| crate::passkey::PasskeyView {
                    id: passkey.id,
                    nickname: passkey.nickname,
                    created_at_ms: passkey.created_at_ms,
                    last_used_at_ms: passkey.last_used_at_ms,
                })
                .collect::<Vec<_>>(),
            "reauth_after_ms": crate::passkey::ADMIN_PASSKEY_REAUTH_AFTER_MS,
        }))
        .into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) async fn api_admin_passkey_register_options(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::passkey::RegisterStartRequest>,
) -> Response {
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let principal = match require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        Ok(principal) => principal,
        Err(status) => return status.into_response(),
    };
    let nickname = match crate::passkey::normalize_nickname(&request.nickname) {
        Ok(nickname) => nickname,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let existing = match store.list_admin_passkeys(&principal.account).await {
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
        &admin_passkey_subject(&principal.account),
        &principal.account,
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

pub(super) async fn api_admin_passkey_register_complete(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::passkey::RegisterCompleteRequest>,
) -> Response {
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let principal = match require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        Ok(principal) => principal,
        Err(status) => return status.into_response(),
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let webauthn = match crate::passkey::webauthn_for_request(&headers) {
        Ok(webauthn) => webauthn,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let (nickname, credential_id, passkey_json) = match state.passkeys.finish_registration(
        &admin_passkey_subject(&principal.account),
        &request.challenge_id,
        request.credential,
        &webauthn,
    ) {
        Ok(created) => created,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let now = auth_now_ms();
    let passkey = crate::passkey::UserPasskey {
        id: match crate::product_auth::new_user_id() {
            Ok(id) => id,
            Err(error) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
            }
        },
        user_id: principal.account.clone(),
        credential_id,
        nickname,
        passkey_json,
        created_at_ms: now,
        last_used_at_ms: Some(now),
    };
    if let Err(error) = store.insert_admin_passkey(&passkey).await {
        return (StatusCode::CONFLICT, error.to_string()).into_response();
    }
    let mut identities = admin_identities(&state.hub);
    identities.touch_last_step_up(&principal.account, now);
    persist_admin_identities(&state.hub, &identities);
    Json(crate::passkey::PasskeyView {
        id: passkey.id,
        nickname: passkey.nickname,
        created_at_ms: passkey.created_at_ms,
        last_used_at_ms: passkey.last_used_at_ms,
    })
    .into_response()
}

pub(super) async fn api_admin_passkey_assert_options(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let principal = match require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        Ok(principal) => principal,
        Err(status) => return status.into_response(),
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let existing = match store.list_admin_passkeys(&principal.account).await {
        Ok(existing) => existing,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    let webauthn = match crate::passkey::webauthn_for_request(&headers) {
        Ok(webauthn) => webauthn,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    match state.passkeys.start_assertion(
        &admin_passkey_subject(&principal.account),
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

pub(super) async fn api_admin_passkey_assert_complete(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::passkey::AssertCompleteRequest>,
) -> Response {
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let principal = match require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        Ok(principal) => principal,
        Err(status) => return status.into_response(),
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let existing = match store.list_admin_passkeys(&principal.account).await {
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
        &admin_passkey_subject(&principal.account),
        &request.challenge_id,
        request.credential,
        &existing,
        &webauthn,
    ) {
        Ok(done) => done,
        Err(error) => return (StatusCode::UNAUTHORIZED, error.to_string()).into_response(),
    };
    let now = auth_now_ms();
    if let Err(error) = store
        .update_admin_passkey(&principal.account, &passkey_id, &passkey_json, now)
        .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    let mut identities = admin_identities(&state.hub);
    identities.touch_last_step_up(&principal.account, now);
    persist_admin_identities(&state.hub, &identities);
    let status = identities.status(crate::admin::cookie_token(&headers).as_deref(), now);
    Json(enrich_admin_status(Some(store), &identities, status).await).into_response()
}

pub(super) async fn api_admin_passkey_reauth(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::passkey::ReauthSettingRequest>,
) -> Response {
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let principal = match require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        Ok(principal) => principal,
        Err(status) => return status.into_response(),
    };
    let mut identities = admin_identities(&state.hub);
    if let Err(error) = identities.set_passkey_reauth(&principal.account, request.enabled) {
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    persist_admin_identities(&state.hub, &identities);
    let status = identities.status(
        crate::admin::cookie_token(&headers).as_deref(),
        auth_now_ms(),
    );
    Json(enrich_admin_status(state.store.as_ref(), &identities, status).await).into_response()
}

pub(super) async fn api_admin_delete_passkey(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let principal = match require_admin(&state, &headers, crate::admin::AdminRole::Viewer) {
        Ok(principal) => principal,
        Err(status) => return status.into_response(),
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    match store.delete_admin_passkey(&principal.account, &id).await {
        Ok(0) => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) async fn api_admin_users(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Operator) {
        return status.into_response();
    }
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    match store.list_users().await {
        Ok(users) => Json(serde_json::json!({
            "users": users
                .iter()
                .map(|user| user_json(&state.hub, user))
                .collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) async fn api_admin_create_user(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<AdminCreateUserRequest>,
) -> Response {
    let _ = (state, peer, headers, request);
    (StatusCode::FORBIDDEN, "this instance is single-user").into_response()
}

pub(super) async fn api_admin_disable_user(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Operator) {
        return status.into_response();
    }
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let now = auth_now_ms();
    if let Err(error) = store.set_user_disabled_at(&id, Some(now)).await {
        let message = error.to_string();
        if message.contains("not found") {
            return StatusCode::NOT_FOUND.into_response();
        }
        return (StatusCode::INTERNAL_SERVER_ERROR, message).into_response();
    }
    let _ = store.delete_user_sessions_for_user(&id).await;
    let _ = store.revoke_user_api_tokens_for_user(&id).await;
    let _ = store.revoke_user_devices_for_user(&id, now).await;
    state.device_access.revoke_user(&id);
    match store.user_by_id(&id).await {
        Ok(Some(user)) => Json(user_json(&state.hub, &user)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) async fn api_admin_set_password(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<AdminSetPasswordRequest>,
) -> Response {
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Owner) {
        return status.into_response();
    }
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let user = match store.user_by_id(&id).await {
        Ok(Some(user)) => user,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    if let Err(error) = crate::product_auth::ensure_password(&request.password, &user.username) {
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    let password_hash = match hash_product_password(request.password).await {
        Ok(hash) => hash,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response();
        }
    };
    if let Err(error) = store
        .update_user_password(
            &id,
            crate::product_auth::PASSWORD_ALGO_ARGON2ID,
            &password_hash,
        )
        .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    state.device_access.revoke_user(&id);
    StatusCode::NO_CONTENT.into_response()
}

/// Creates the admin setup state and, while no user exists, publishes the
/// one-time setup token.
pub(super) async fn prepare_admin_setup(
    state: &Arc<AppState>,
    data_dir: PathBuf,
) -> anyhow::Result<Arc<crate::admin::AdminSetupState>> {
    let setup = Arc::new(crate::admin::AdminSetupState::new(data_dir));
    let setup_needed = match state.store.as_ref() {
        Some(store) => store
            .list_users()
            .await
            .map(|users| users.is_empty())
            .unwrap_or(true),
        None => true,
    };
    {
        let mut identities = admin_identities(&state.hub);
        match crate::admin::ensure_admin_setup_token(&setup.data_dir, &mut identities, setup_needed)
        {
            Ok(Some(token)) => {
                persist_admin_identities(&state.hub, &identities);
                tracing::warn!(
                    path = %setup.token_path().display(),
                    token,
                    "admin_setup_token"
                );
            }
            Ok(None) => {
                persist_admin_identities(&state.hub, &identities);
                if setup_needed {
                    tracing::info!(
                        path = %setup.token_path().display(),
                        "admin_setup_token_ready"
                    );
                }
            }
            Err(error) => {
                return Err(error).context("preparing admin setup token");
            }
        }
    }
    Ok(setup)
}
