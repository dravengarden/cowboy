//! Public authentication status, automation credentials, first-run setup,
//! registration and password login.

use super::*;

pub(in crate::server) async fn api_auth_status(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    if !state.product_auth_enabled {
        return Json(serde_json::json!({
            "registration": crate::admin::RegistrationPublicStatus {
                enabled: false,
                mode: crate::admin::RegistrationMode::Disabled,
                accepts_registration: false,
            },
            "setup_required": false,
            "setup_pending": false,
            "login_method_order": [],
            "providers": [],
            "me": {
                "account": "local",
                "user_id": crate::product_auth::local_product_principal().user_id,
                "role": "owner",
                "auth_enabled": false,
            },
        }))
        .into_response();
    }
    let setup_required = instance_needs_setup(&state).await;
    let setup_pending = setup_required
        && crate::admin::setup_cookie_token(&headers)
            .is_some_and(|token| state.setup.tickets.is_valid(&token));
    let host_plugins = state
        .plugin_catalog
        .as_ref()
        .and_then(|catalog| catalog.runtime())
        .map_or_else(Vec::new, |runtime| {
            state.product_authentication.public_host_plugins(&runtime)
        });
    let mut body = serde_json::json!({
        "registration": crate::admin::RegistrationPublicStatus {
            enabled: false,
            mode: crate::admin::RegistrationMode::Disabled,
            accepts_registration: false,
        },
        "setup_required": setup_required,
        "setup_pending": setup_pending,
        "password_enabled": state.product_authentication.password_enabled,
        "login_method_order": state.product_authentication.login_method_order(),
        "passkeys": {
            "enabled": state.product_authentication.passkeys.enabled,
            "prompt_after_login": state.product_authentication.passkeys.prompt_after_login,
            "session_refresh_enabled": state.product_authentication.passkeys.session_refresh_enabled,
        },
        "session": state.product_authentication.session,
        "capacity": state.product_authentication.capacity,
        "logout": state.product_authentication.logout,
        "automation": state.product_authentication.automation,
        "providers": state.product_authentication.public_providers(),
        "host_plugins": host_plugins,
    });
    if let Some((session, user)) = product_session_and_user_from_cookie(&state, &headers).await {
        let me = match product_me_for_user(
            state.store.as_ref(),
            &state.hub,
            &state.product_authentication,
            &user,
            Some(&session),
        )
        .await
        {
            Ok(me) => me,
            Err(error) => return product_session_policy_unavailable(error),
        };
        body["me"] = serde_json::to_value(me).unwrap_or_default();
    }
    Json(body).into_response()
}

pub(in crate::server) async fn api_auth_create_automation_credential(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<AutomationCredentialRequest>,
) -> Response {
    if !state.product_auth_enabled || !state.product_authentication.automation.enabled {
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
    if !product_principal(&state.hub, &user)
        .role
        .at_least(crate::admin::AdminRole::Operator)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(store) = state.store.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let name = request.name.trim();
    if name.is_empty()
        || name.chars().count() > 64
        || name.chars().any(char::is_control)
        || request.scopes.is_empty()
    {
        return (
            StatusCode::BAD_REQUEST,
            "invalid automation credential request",
        )
            .into_response();
    };
    let maximum_age_ms = state
        .product_authentication
        .automation
        .credential_max_age_ms;
    let ttl_ms = match request.ttl_seconds {
        Some(seconds) => seconds.checked_mul(1_000),
        None => Some(maximum_age_ms),
    };
    let Some(ttl_ms) = ttl_ms.filter(|ttl| (60_000..=maximum_age_ms).contains(ttl)) else {
        return (
            StatusCode::BAD_REQUEST,
            "automation credential lifetime exceeds the service policy",
        )
            .into_response();
    };
    let credential_id = match crate::product_auth::new_user_id() {
        Ok(credential_id) => credential_id,
        Err(error) => {
            tracing::error!(%error, "automation_credential_id_generation");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let mut scopes = request.scopes;
    scopes.sort();
    scopes.dedup();
    let now_ms = auth_now_ms();
    let (access_token, expires_at_ms) = match state.device_access.issue_automation(
        &credential_id,
        &user.id,
        &request.public_key,
        scopes.clone(),
        now_ms,
        ttl_ms,
    ) {
        Ok(credential) => credential,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    if let Err(error) = store
        .record_automation_credential_issued(
            &user.id,
            &credential_id,
            name,
            &scopes,
            expires_at_ms,
            now_ms,
        )
        .await
    {
        state.device_access.revoke_access_token(&access_token);
        tracing::error!(%error, user_id = user.id, credential_id, "automation credential audit");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    tracing::info!(
        user_id = user.id,
        credential_id,
        credential_name = name,
        expires_at_ms,
        scopes = ?scopes,
        "automation_credential_issued"
    );
    no_store_json(
        StatusCode::CREATED,
        serde_json::json!({
            "credential_id": credential_id,
            "access_token": access_token,
            "expires_at_ms": expires_at_ms,
            "scopes": scopes,
            "principal_class": "automation",
        }),
    )
}

pub(in crate::server) async fn api_auth_setup(
    State(state): State<ProductAuthState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::admin::AdminSetupRequest>,
) -> Response {
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    if let Some(rejected) = reject_insecure_admin(&headers, peer) {
        return rejected;
    }
    let ip = crate::product_auth::client_ip(&headers, peer).to_string();
    apply_rate_limit(&state, "auth:setup", &ip).await;
    if !instance_needs_setup(&state).await {
        state.rate_limits.record_failure("auth:setup", &ip);
        return (StatusCode::FORBIDDEN, "this instance already has its user").into_response();
    }
    let identities = admin_identities(&state.hub);
    if !identities.setup_token_matches(&request.token) {
        state.rate_limits.record_failure("auth:setup", &ip);
        tracing::info!(ok = false, "instance_setup");
        return (StatusCode::BAD_REQUEST, "invalid setup token").into_response();
    }
    match state.setup.tickets.issue() {
        Ok(ticket) => {
            state.rate_limits.reset("auth:setup", &ip);
            tracing::info!(ok = true, "instance_setup");
            let secure = crate::product_auth::request_is_https(&headers);
            (
                [(
                    header::SET_COOKIE,
                    crate::admin::setup_session_cookie(&ticket, secure),
                )],
                Json(serde_json::json!({
                    "setup_required": true,
                    "setup_pending": true,
                    "registration": {
                        "enabled": false,
                        "mode": "disabled",
                        "accepts_registration": false,
                    },
                })),
            )
                .into_response()
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(in crate::server) async fn api_auth_register(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<RegisterRequest>,
) -> Response {
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    if let Some(rejected) = reject_insecure_admin(&headers, peer) {
        return rejected;
    }
    let Some(store) = durable_store(&state.store).cloned() else {
        return missing_store();
    };
    let _setup_guard = state.setup_lock.lock().await;
    if !instance_needs_setup(&state).await {
        return (StatusCode::FORBIDDEN, "this instance already has its user").into_response();
    }
    let setup_ok = crate::admin::setup_cookie_token(&headers)
        .is_some_and(|token| state.setup.tickets.is_valid(&token));
    if !setup_ok {
        return (StatusCode::FORBIDDEN, "setup token required").into_response();
    }
    let username = match crate::product_auth::normalize_username(&request.account) {
        Ok(username) => username,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    if let Err(error) = crate::admin::ensure_admin_password(&request.password, &username) {
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    let ip = crate::product_auth::client_ip(&headers, peer).to_string();
    apply_rate_limit(&state, &username, &ip).await;
    let password_hash = match hash_product_password(request.password.clone()).await {
        Ok(hash) => hash,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response();
        }
    };
    let now = auth_now_ms();
    let user = crate::store::ProductUser {
        id: match crate::product_auth::new_user_id() {
            Ok(id) => id,
            Err(error) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
            }
        },
        username: username.clone(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
        password_hash,
        created_at_ms: now,
        updated_at_ms: now,
        disabled_at_ms: None,
    };
    if let Err(error) = store.insert_user(&user).await {
        let message = error.to_string();
        if message.contains("account already exists") {
            state.rate_limits.record_failure(&username, &ip);
            tracing::info!(username, ok = false, "product_register");
            return (StatusCode::CONFLICT, "account already exists").into_response();
        }
        return (StatusCode::INTERNAL_SERVER_ERROR, message).into_response();
    }
    let (permissions_value, permissions_snapshot) = state.hub.with_settings_mut(|settings| {
        let mut policy = crate::admin::PermissionPolicy::from_setting(
            settings.get(crate::admin::PERMISSIONS_SETTING),
        );
        policy.upsert_grant(&username, crate::admin::AdminRole::Owner);
        let value = serde_json::to_value(&policy).unwrap_or_else(|_| serde_json::json!({}));
        let snapshot = Hub::commit_setting_locked(
            settings,
            crate::admin::PERMISSIONS_SETTING.to_owned(),
            value.clone(),
        );
        (value, snapshot)
    });
    state.hub.publish_setting(
        crate::admin::PERMISSIONS_SETTING.to_owned(),
        permissions_value,
        permissions_snapshot,
    );
    let mut identities = admin_identities(&state.hub);
    if identities.bootstrap_required()
        && let Err(error) = identities.create_account(
            &crate::admin::AdminCredentials {
                account: username.clone(),
                password: request.password.clone(),
            },
            crate::admin::AdminRole::Owner,
            now,
        )
    {
        let _ = store.delete_user(&user.id).await;
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    identities.setup_token_hash = None;
    persist_admin_identities(&state.hub, &identities);
    state.setup.tickets.clear();
    if let Err(error) = crate::admin::consume_admin_setup_token_file(&state.setup.data_dir) {
        tracing::error!(%error, "admin_setup_token_file");
    }
    state.rate_limits.reset(&username, &ip);
    tracing::info!(username, ok = true, "instance_register");
    let mut response = issue_product_session(
        &state,
        &store,
        &user,
        &headers,
        crate::auth_plugins::PASSWORD_LOGIN_METHOD,
        None,
    )
    .await;
    let secure = crate::product_auth::request_is_https(&headers);
    if let Ok(value) = crate::admin::clear_setup_cookie(secure).parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

pub(in crate::server) async fn api_auth_login(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> Response {
    if !state.product_authentication.password_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    let Some(store) = durable_store(&state.store).cloned() else {
        return missing_store();
    };
    let normalized = crate::product_auth::normalize_username(&request.account);
    let rate_name = normalized
        .as_ref()
        .map_or("unknown", String::as_str)
        .to_owned();
    let ip = crate::product_auth::client_ip(&headers, peer).to_string();
    apply_rate_limit(&state, &rate_name, &ip).await;
    let user = match normalized {
        Ok(username) => store.user_by_username(&username).await.ok().flatten(),
        Err(_) => None,
    };
    let password = request.password.clone();
    let verified = match user.as_ref() {
        Some(user) => {
            let hash = user.password_hash.clone();
            tokio::task::spawn_blocking(move || {
                crate::product_auth::verify_password(&password, &hash)
            })
            .await
            .unwrap_or(false)
        }
        None => tokio::task::spawn_blocking(move || {
            crate::product_auth::verify_unknown_user_password(&password)
        })
        .await
        .unwrap_or(false),
    };
    let disabled = user
        .as_ref()
        .is_some_and(|user| user.disabled_at_ms.is_some());
    if !verified || disabled || user.is_none() {
        state.rate_limits.record_failure(&rate_name, &ip);
        let reason = if disabled { "disabled" } else { "invalid" };
        tracing::info!(username = rate_name, ok = false, reason, "product_login");
        return (StatusCode::UNAUTHORIZED, "invalid credentials").into_response();
    }
    let user = user.expect("present after credential check");
    state.rate_limits.reset(&rate_name, &ip);
    tracing::info!(username = user.username, ok = true, "product_login");
    issue_product_session(
        &state,
        &store,
        &user,
        &headers,
        crate::auth_plugins::PASSWORD_LOGIN_METHOD,
        None,
    )
    .await
}
