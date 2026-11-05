//! Product login, sessions, passkeys, OIDC, device authorization and API
//! tokens: the /api/auth handlers and the principal resolution they share.

use super::*;

pub(super) fn product_auth_router(state: ProductAuthState) -> Router {
    Router::new()
        .route("/api/auth/status", get(api_auth_status))
        .route("/api/auth/setup", post(api_auth_setup))
        .route("/api/auth/register", post(api_auth_register))
        .route("/api/auth/login", post(api_auth_login))
        .route("/api/auth/oidc/start", get(api_auth_oidc_start))
        .route(
            "/api/auth/oidc/callback",
            get(api_auth_oidc_callback).post(api_auth_oidc_callback_form),
        )
        .route(
            "/api/auth/oidc/native/exchange",
            post(api_auth_oidc_native_exchange),
        )
        .route(
            "/api/auth/oidc/native/poll",
            post(api_auth_oidc_native_poll),
        )
        .route(
            "/api/auth/oidc/native/events",
            get(api_auth_oidc_native_events),
        )
        .route(
            "/api/auth/oidc/native/cancel",
            post(api_auth_oidc_native_cancel),
        )
        .route(
            "/api/auth/oidc/native/complete",
            get(api_auth_oidc_native_complete),
        )
        .route(
            "/api/auth/providers/{provider_id}/start",
            get(api_auth_oidc_start),
        )
        .route(
            "/api/auth/providers/{provider_id}/callback",
            get(api_auth_oidc_callback).post(api_auth_oidc_callback_form),
        )
        .route(
            "/api/auth/providers/{provider_id}/native/exchange",
            post(api_auth_oidc_native_exchange),
        )
        .route(
            "/api/auth/providers/{provider_id}/native/poll",
            post(api_auth_oidc_native_poll),
        )
        .route(
            "/api/auth/providers/{provider_id}/native/events",
            get(api_auth_oidc_native_events),
        )
        .route(
            "/api/auth/providers/{provider_id}/native/cancel",
            post(api_auth_oidc_native_cancel),
        )
        .route(
            "/api/auth/providers/{provider_id}/backchannel-logout",
            post(api_auth_oidc_backchannel_logout),
        )
        .route("/api/auth/logout", post(api_auth_logout))
        .route("/api/auth/logout/complete", get(api_auth_logout_complete))
        .route("/api/auth/me", get(api_auth_me))
        .route(
            "/api/auth/automation/credentials",
            post(api_auth_create_automation_credential),
        )
        .route(
            "/api/auth/device/authorizations",
            post(api_auth_device_authorization_start),
        )
        .route(
            "/api/auth/device/authorizations/inspect",
            post(api_auth_device_authorization_inspect),
        )
        .route(
            "/api/auth/device/authorizations/approve",
            post(api_auth_device_authorization_approve),
        )
        .route(
            "/api/auth/device/authorizations/deny",
            post(api_auth_device_authorization_deny),
        )
        .route(
            "/api/auth/device/authorizations/events",
            get(api_auth_device_authorization_events),
        )
        .route("/api/auth/device/exchange", post(api_auth_device_exchange))
        .route(
            "/api/auth/device/cardea",
            get(api_auth_cardea_device_configuration).post(api_auth_cardea_device_exchange),
        )
        .route("/api/auth/device/refresh", post(api_auth_device_refresh))
        .route("/api/auth/sessions", get(api_auth_list_sessions))
        .route("/api/auth/sessions/{id}", delete(api_auth_delete_session))
        .route(
            "/api/auth/active-clients/{id}",
            delete(api_auth_release_active_client),
        )
        .route("/api/auth/devices", get(api_auth_list_devices))
        .route("/api/auth/devices/{id}", delete(api_auth_delete_device))
        .route(
            "/api/auth/tokens",
            get(api_auth_list_tokens).post(api_auth_create_token),
        )
        .route("/api/auth/tokens/{id}", delete(api_auth_delete_token))
        .route("/api/auth/passkeys", get(api_auth_list_passkeys))
        .route(
            "/api/auth/passkeys/register/options",
            post(api_auth_passkey_register_options),
        )
        .route(
            "/api/auth/passkeys/register/complete",
            post(api_auth_passkey_register_complete),
        )
        .route(
            "/api/auth/passkeys/assert/options",
            post(api_auth_passkey_assert_options),
        )
        .route(
            "/api/auth/passkeys/assert/complete",
            post(api_auth_passkey_assert_complete),
        )
        .route(
            "/api/auth/passkeys/external/start",
            post(api_auth_passkey_external_start),
        )
        .route(
            "/api/auth/passkeys/external/options",
            post(api_auth_passkey_external_options),
        )
        .route(
            "/api/auth/passkeys/external/complete",
            post(api_auth_passkey_external_complete),
        )
        .route(
            "/api/auth/passkeys/external/fail",
            post(api_auth_passkey_external_fail),
        )
        .route(
            "/api/auth/passkeys/external/events",
            get(api_auth_passkey_external_events),
        )
        .route(
            "/api/auth/passkeys/external/finalize",
            post(api_auth_passkey_external_finalize),
        )
        .route("/api/auth/passkeys/reauth", put(api_auth_passkey_reauth))
        .route("/api/auth/passkeys/{id}", delete(api_auth_delete_passkey))
        .route(
            "/api/admin/users",
            get(api_admin_users).post(api_admin_create_user),
        )
        .route(
            "/api/admin/users/{id}/disable",
            post(api_admin_disable_user),
        )
        .route(
            "/api/admin/users/{id}/password",
            post(api_admin_set_password),
        )
        .route("/api/admin/auth", get(api_admin_auth))
        .route("/api/admin/auth/setup", post(api_admin_setup))
        .route("/api/admin/auth/bootstrap", post(api_admin_bootstrap))
        .route("/api/admin/auth/login", post(api_admin_login))
        .route("/api/admin/auth/logout", post(api_admin_logout))
        .route(
            "/api/admin/accounts",
            get(api_admin_accounts).post(api_admin_create_account),
        )
        .route("/api/admin/overview", get(api_admin_overview))
        .route("/api/admin/sessions", get(api_admin_sessions))
        .route("/api/admin/machines", get(api_admin_machines))
        .route(
            "/api/admin/registration",
            get(api_admin_registration).put(api_admin_registration_put),
        )
        .route(
            "/api/admin/registration/tokens",
            post(api_admin_registration_token),
        )
        .route(
            "/api/admin/registration/tokens/{id}",
            delete(api_admin_registration_token_delete),
        )
        .route(
            "/api/admin/permissions",
            get(api_admin_permissions).put(api_admin_permissions_put),
        )
        .route(
            "/api/admin/session-limits",
            get(api_admin_session_limits).put(api_admin_session_limits_put),
        )
        .route("/api/admin/providers", get(api_admin_providers))
        .route("/api/admin/plugins", get(api_admin_plugins))
        .route("/api/admin/passkeys", get(api_admin_list_passkeys))
        .route(
            "/api/admin/passkeys/register/options",
            post(api_admin_passkey_register_options),
        )
        .route(
            "/api/admin/passkeys/register/complete",
            post(api_admin_passkey_register_complete),
        )
        .route(
            "/api/admin/passkeys/assert/options",
            post(api_admin_passkey_assert_options),
        )
        .route(
            "/api/admin/passkeys/assert/complete",
            post(api_admin_passkey_assert_complete),
        )
        .route("/api/admin/passkeys/reauth", put(api_admin_passkey_reauth))
        .route("/api/admin/passkeys/{id}", delete(api_admin_delete_passkey))
        .route(
            "/api/admin/providers/refresh",
            post(api_admin_providers_refresh),
        )
        .route(
            "/api/admin/plugins/refresh",
            post(api_admin_plugins_refresh),
        )
        .with_state(state)
}

pub(super) fn product_me(hub: &Hub, username: &str) -> ProductMe {
    ProductMe {
        account: username.to_owned(),
        user_id: None,
        role: permission_policy(hub).role_for(username),
        passkey_count: 0,
        passkey_reauth_enabled: false,
        passkey_reauth_required: false,
        passkey_reauth_after_ms: crate::passkey::DEFAULT_PASSKEY_REAUTH_AFTER_MS,
        passkey_reauth_due_at_ms: None,
        passkey_reauth_warn_at_ms: None,
        primary_reauth_due_at_ms: None,
        primary_reauth_warn_at_ms: None,
        session_idle_due_at_ms: None,
        session_expires_at_ms: None,
        session_server_now_ms: None,
        session_reauth_kind: None,
        primary_auth_method: None,
    }
}

pub(super) fn product_session_deadlines(
    server: crate::auth_plugins::SessionServerPolicy,
    policy: &crate::passkey::PasskeyPolicy,
    passkey_refresh_enabled: bool,
    session: &crate::store::ProductUserSession,
    now_ms: i64,
) -> ProductSessionDeadlines {
    let primary_hard_due = session
        .primary_authenticated_at_ms
        .saturating_add(server.primary_max_age_ms);
    let primary_due_at_ms = primary_hard_due.min(session.expires_at_ms);
    let primary_warn_at_ms = primary_due_at_ms.saturating_sub(server.primary_warning_ms);
    let idle_due_at_ms = server.activity_sliding_enabled.then(|| {
        session
            .last_seen_at_ms
            .saturating_add(server.idle_timeout_ms)
    });
    let passkey_eligible = passkey_refresh_enabled && policy.reauth_eligible();
    let passkey_due_at_ms = if passkey_eligible {
        session.passkey_verified_at_ms.map(|verified_at| {
            verified_at.saturating_add(policy.reauth_after_ms.min(server.passkey_max_age_ms))
        })
    } else {
        None
    };
    let passkey_warn_at_ms =
        passkey_due_at_ms.map(|due_at| due_at.saturating_sub(server.passkey_warning_ms));
    let required_kind = if primary_due_at_ms <= now_ms {
        Some(ProductSessionReauthKind::Primary)
    } else if passkey_due_at_ms.is_some_and(|due_at| due_at <= now_ms)
        || idle_due_at_ms.is_some_and(|due_at| due_at <= now_ms)
    {
        Some(if passkey_eligible {
            ProductSessionReauthKind::Passkey
        } else {
            ProductSessionReauthKind::Primary
        })
    } else {
        None
    };
    ProductSessionDeadlines {
        idle_due_at_ms,
        passkey_due_at_ms,
        passkey_warn_at_ms,
        primary_due_at_ms,
        primary_warn_at_ms,
        required_kind,
    }
}

pub(super) fn product_me_for_user_with_policy(
    hub: &Hub,
    authentication: &crate::auth_plugins::ProductAuthentication,
    user: &crate::store::ProductUser,
    session: Option<&crate::store::ProductUserSession>,
    policy: &crate::passkey::PasskeyPolicy,
) -> ProductMe {
    let mut me = product_me(hub, &user.username);
    me.user_id = Some(user.id.clone());
    let passkey_refresh_enabled =
        authentication.passkeys.enabled && authentication.passkeys.session_refresh_enabled;
    me.passkey_count = policy.passkey_count;
    me.passkey_reauth_enabled = passkey_refresh_enabled && policy.enabled;
    me.passkey_reauth_after_ms = policy
        .reauth_after_ms
        .min(authentication.session.passkey_max_age_ms);
    if let Some(session) = session {
        me.primary_auth_method = session.primary_auth_method.clone();
        let now = auth_now_ms();
        let deadlines = product_session_deadlines(
            authentication.session,
            policy,
            passkey_refresh_enabled,
            session,
            now,
        );
        me.passkey_reauth_due_at_ms = deadlines.passkey_due_at_ms;
        me.passkey_reauth_warn_at_ms = deadlines.passkey_warn_at_ms;
        me.passkey_reauth_required =
            deadlines.required_kind == Some(ProductSessionReauthKind::Passkey);
        me.primary_reauth_due_at_ms = Some(deadlines.primary_due_at_ms);
        me.primary_reauth_warn_at_ms = Some(deadlines.primary_warn_at_ms);
        me.session_idle_due_at_ms = deadlines.idle_due_at_ms;
        me.session_expires_at_ms = Some(session.expires_at_ms);
        me.session_server_now_ms = Some(now);
        me.session_reauth_kind = deadlines.required_kind;
    }
    me
}

pub(super) async fn product_me_for_user(
    store: Option<&Store>,
    hub: &Hub,
    authentication: &crate::auth_plugins::ProductAuthentication,
    user: &crate::store::ProductUser,
    session: Option<&crate::store::ProductUserSession>,
) -> anyhow::Result<ProductMe> {
    let Some(store) = store else {
        let mut me = product_me(hub, &user.username);
        me.user_id = Some(user.id.clone());
        return Ok(me);
    };
    let policy = store
        .user_passkey_policy(&user.id)
        .await
        .context("load product session Passkey policy")?
        .unwrap_or(crate::passkey::PasskeyPolicy {
            enabled: false,
            reauth_after_ms: crate::passkey::DEFAULT_PASSKEY_REAUTH_AFTER_MS,
            last_step_up_at_ms: None,
            passkey_count: 0,
        });
    Ok(product_me_for_user_with_policy(
        hub,
        authentication,
        user,
        session,
        &policy,
    ))
}

pub(super) fn product_session_policy_unavailable(error: anyhow::Error) -> Response {
    tracing::error!(%error, "product_session_policy_unavailable");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "session protection is temporarily unavailable",
    )
        .into_response()
}

pub(super) async fn product_user_from_cookie(
    state: &ProductAuthState,
    headers: &HeaderMap,
) -> Option<crate::store::ProductUser> {
    product_user_from_store_cookie(state.store.as_ref()?, headers).await
}

pub(super) async fn product_session_and_user_from_cookie(
    state: &ProductAuthState,
    headers: &HeaderMap,
) -> Option<(crate::store::ProductUserSession, crate::store::ProductUser)> {
    product_session_and_user_from_store_cookie(state.store.as_ref()?, headers).await
}

#[cfg(test)]
pub(super) async fn resolve_product_principal(
    product_auth_enabled: bool,
    store: Option<&Store>,
    hub: &Hub,
    headers: &HeaderMap,
) -> Option<ProductPrincipal> {
    resolve_product_request_principal(product_auth_enabled, store, hub, headers)
        .await
        .map(|(principal, _)| principal)
}

pub(super) async fn resolve_product_request_principal(
    product_auth_enabled: bool,
    store: Option<&Store>,
    hub: &Hub,
    headers: &HeaderMap,
) -> Option<(ProductPrincipal, Option<crate::store::ProductUserSession>)> {
    if !product_auth_enabled {
        return Some((crate::product_auth::local_product_principal(), None));
    }
    let store = store?;
    if let Some(token) = crate::product_auth::bearer_token(headers) {
        return product_from_bearer(store, hub, &token)
            .await
            .map(|principal| (principal, None));
    }
    let (session, user) = product_session_and_user_from_store_cookie(store, headers).await?;
    Some((product_principal(hub, &user), Some(session)))
}

pub(super) async fn resolve_product_api_request_principal(
    state: ProductRequestAuth<'_>,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<Option<AuthenticatedProductRequest>, ()> {
    if !state.product_auth_enabled {
        return Ok(Some(AuthenticatedProductRequest {
            principal: crate::product_auth::local_product_principal(),
            cookie_session: None,
            device_identity: None,
            permissions: None,
        }));
    }
    let bearer = crate::product_auth::bearer_token(headers);
    if bearer.as_deref().is_some_and(|token| {
        token.starts_with(crate::client_auth::ACCESS_TOKEN_PREFIX)
            || token.starts_with(crate::client_auth::AUTOMATION_ACCESS_TOKEN_PREFIX)
    }) {
        let path_and_query = uri
            .path_and_query()
            .map_or(uri.path(), axum::http::uri::PathAndQuery::as_str);
        let identity = state
            .device_access
            .authenticate(headers, method, path_and_query, auth_now_ms())
            .map_err(|error| {
                tracing::info!(%error, "device_access_rejected");
            })?
            .ok_or(())?;
        if identity.is_automation()
            && (!state.product_authentication.automation.enabled
                || !automation_route_allowed(&identity, method, uri.path()))
        {
            tracing::info!(
                credential_id = identity.device_id,
                method = %method,
                path = uri.path(),
                "automation_access_scope_rejected"
            );
            return Err(());
        }
        let Some(store) = state.store else {
            return Ok(None);
        };
        let user = store
            .user_by_id(&identity.user_id)
            .await
            .map_err(|error| {
                tracing::error!(%error, "device_access_user_lookup");
            })?
            .filter(|user| user.disabled_at_ms.is_none());
        let Some(user) = user else {
            state.device_access.revoke_device(&identity.device_id);
            return Ok(None);
        };
        if !identity.is_automation() {
            let active = store
                .list_user_devices_for_user(&identity.user_id)
                .await
                .map_err(|_| ())?
                .into_iter()
                .any(|device| device.id == identity.device_id && device.revoked_at_ms.is_none());
            if !active {
                state.device_access.revoke_device(&identity.device_id);
                return Ok(None);
            }
            let store = store.clone();
            let device_id = identity.device_id.clone();
            tokio::spawn(async move {
                let _ = store
                    .touch_user_device_last_used(&device_id, auth_now_ms())
                    .await;
            });
        }
        return AuthenticatedProductRequest {
            principal: product_principal(state.hub, &user),
            cookie_session: None,
            device_identity: Some(identity),
            permissions: None,
        }
        .observe_permissions(state.hub)
        .map(Some);
    }
    resolve_product_request_principal(state.product_auth_enabled, state.store, state.hub, headers)
        .await
        .map(|(principal, cookie_session)| {
            AuthenticatedProductRequest {
                principal,
                cookie_session,
                device_identity: None,
                permissions: None,
            }
            .observe_permissions(state.hub)
        })
        .transpose()
}

pub(super) async fn product_user_from_store_cookie(
    store: &Store,
    headers: &HeaderMap,
) -> Option<crate::store::ProductUser> {
    product_session_and_user_from_store_cookie(store, headers)
        .await
        .map(|(_, user)| user)
}

pub(super) async fn product_session_and_user_from_store_cookie(
    store: &Store,
    headers: &HeaderMap,
) -> Option<(crate::store::ProductUserSession, crate::store::ProductUser)> {
    let token = crate::product_auth::user_cookie_token(headers)?;
    let session = store
        .user_session_by_token_hash(&crate::admin::hex_sha256(token.as_bytes()))
        .await
        .ok()
        .flatten()?;
    if session.expires_at_ms <= auth_now_ms() {
        return None;
    }
    let user = store.user_by_id(&session.user_id).await.ok().flatten()?;
    user.disabled_at_ms.is_none().then_some((session, user))
}

pub(super) async fn product_from_bearer(
    store: &Store,
    hub: &Hub,
    token: &str,
) -> Option<ProductPrincipal> {
    let record = store
        .user_api_token_by_hash(&crate::admin::hex_sha256(token.as_bytes()))
        .await
        .ok()
        .flatten()?;
    if record.revoked_at_ms.is_some() {
        return None;
    }
    if record
        .expires_at_ms
        .is_some_and(|expires| expires <= auth_now_ms())
    {
        return None;
    }
    let user = store.user_by_id(&record.user_id).await.ok().flatten()?;
    if user.disabled_at_ms.is_some() {
        return None;
    }
    let store = store.clone();
    let token_id = record.id.clone();
    let now_ms = auth_now_ms();
    tokio::spawn(async move {
        let _ = store
            .touch_user_api_token_last_used(&token_id, now_ms)
            .await;
    });
    Some(product_principal(hub, &user))
}

pub(super) fn product_principal(hub: &Hub, user: &crate::store::ProductUser) -> ProductPrincipal {
    ProductPrincipal {
        user_id: user.id.clone(),
        username: user.username.clone(),
        role: permission_policy(hub).role_for(&user.username),
    }
}

pub(super) async fn enforce_product_api(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    if internal_machine_refresh(&method, &path, peer, request.headers()) {
        return next.run(request).await;
    }
    match classify_route(&method, &path) {
        RouteAuth::Public | RouteAuth::MetricsScrape => return next.run(request).await,
        RouteAuth::AdminViewer => {
            if let Err(status) = require_admin_role(
                &state.hub,
                request.headers(),
                crate::admin::AdminRole::Viewer,
            ) {
                return status.into_response();
            }
            return next.run(request).await;
        }
        RouteAuth::AdminOperator => {
            if let Err(status) = require_admin_role(
                &state.hub,
                request.headers(),
                crate::admin::AdminRole::Operator,
            ) {
                return status.into_response();
            }
            if matches!(method, Method::POST | Method::PUT | Method::DELETE)
                && let Some(rejected) =
                    reject_bad_origin(request.headers(), peer, &state.public_origins)
            {
                return rejected;
            }
            return next.run(request).await;
        }
        RouteAuth::AdminOwner => {
            if let Err(status) = require_admin_role(
                &state.hub,
                request.headers(),
                crate::admin::AdminRole::Owner,
            ) {
                return status.into_response();
            }
            if matches!(method, Method::POST | Method::PUT | Method::DELETE)
                && let Some(rejected) =
                    reject_bad_origin(request.headers(), peer, &state.public_origins)
            {
                return rejected;
            }
            return next.run(request).await;
        }
        RouteAuth::ProductOrAdminOperator
            if require_admin_role(
                &state.hub,
                request.headers(),
                crate::admin::AdminRole::Operator,
            )
            .is_ok() =>
        {
            if matches!(method, Method::POST | Method::PUT | Method::DELETE)
                && crate::product_auth::bearer_token(request.headers()).is_none()
                && let Some(rejected) =
                    reject_bad_origin(request.headers(), peer, &state.public_origins)
            {
                return rejected;
            }
            return next.run(request).await;
        }
        _ => {}
    }
    let resolved = match resolve_product_api_request_principal(
        ProductRequestAuth::from(state.as_ref()),
        &method,
        request.uri(),
        request.headers(),
    )
    .await
    {
        Ok(resolved) => resolved,
        Err(()) => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let Some(authenticated) = resolved else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let principal = authenticated.principal.clone();
    let cookie_session = authenticated.cookie_session.clone();
    // Let the WebSocket handler report an expired product session over the
    // authenticated protocol before closing with 4001. Rejecting the HTTP
    // upgrade here surfaces as an opaque 1006 in browsers and is
    // indistinguishable from a network outage. `handle_ws` performs the same
    // freshness check before sending any bootstrap or session data.
    if enforce_product_session_freshness_before_dispatch(&path)
        && let (Some(store), Some(session)) = (state.store.as_ref(), cookie_session.as_ref())
        && let Err(response) =
            ensure_product_session_fresh(store, session, &state.product_authentication).await
    {
        return response;
    }
    let via_cookie = state.product_auth_enabled
        && crate::product_auth::user_cookie_token(request.headers()).is_some()
        && crate::product_auth::bearer_token(request.headers()).is_none();
    if via_cookie
        && (path == "/ws" || matches!(method, Method::POST | Method::PUT | Method::DELETE))
        && let Some(rejected) = reject_bad_origin(request.headers(), peer, &state.public_origins)
    {
        return rejected;
    }
    match classify_route(&method, &path) {
        RouteAuth::ProductOperator | RouteAuth::ProductOrAdminOperator
            if !principal.role.at_least(crate::admin::AdminRole::Operator) =>
        {
            return StatusCode::FORBIDDEN.into_response();
        }
        RouteAuth::ProductSessionSee | RouteAuth::ProductSessionMutate => {
            if let Err(status) = authorize_session_route(&state.hub, &principal, &method, &path) {
                return status.into_response();
            }
        }
        _ => {}
    }
    request.extensions_mut().insert(authenticated);
    next.run(request).await
}

pub(super) fn enforce_product_session_freshness_before_dispatch(path: &str) -> bool {
    path != "/ws"
}

pub(super) async fn issue_product_session(
    state: &ProductAuthState,
    store: &Store,
    user: &crate::store::ProductUser,
    headers: &HeaderMap,
    auth_method: &str,
    external_identity: Option<&crate::oidc::VerifiedIdentity>,
) -> Response {
    let previous_token_hash = crate::product_auth::user_cookie_token(headers)
        .map(|token| crate::admin::hex_sha256(token.as_bytes()));
    let previous_session = if let Some(token_hash) = previous_token_hash.as_deref() {
        match store.user_session_by_token_hash(token_hash).await {
            Ok(session) => session,
            Err(error) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
            }
        }
    } else {
        None
    };
    if let Some(previous) = previous_session.as_ref() {
        if previous.user_id != user.id {
            return (
                StatusCode::CONFLICT,
                "Sign out before changing the account for this browser session.",
            )
                .into_response();
        }
        if previous
            .primary_auth_method
            .as_deref()
            .is_some_and(|method| method != auth_method)
        {
            return (
                StatusCode::CONFLICT,
                "Sign out before switching this browser session's sign-in method.",
            )
                .into_response();
        }
        if let Some(identity) = external_identity
            && (previous
                .auth_issuer
                .as_deref()
                .is_some_and(|issuer| issuer != identity.issuer)
                || previous
                    .auth_subject
                    .as_deref()
                    .is_some_and(|subject| subject != identity.subject))
        {
            return (
                StatusCode::CONFLICT,
                "Sign out before changing this browser session's external identity.",
            )
                .into_response();
        }
    }
    let password_login = auth_method == crate::auth_plugins::PASSWORD_LOGIN_METHOD;
    if password_login != external_identity.is_none() {
        tracing::error!(auth_method, "product session identity binding mismatch");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if let Some(identity) = external_identity {
        let Some(provider) = state.product_authentication.provider(auth_method) else {
            return StatusCode::UNAUTHORIZED.into_response();
        };
        if !provider.accepts_identity(identity) {
            tracing::warn!(auth_method, "product session external identity mismatch");
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    let token = match crate::product_auth::new_session_token() {
        Ok(token) => token,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    let now = auth_now_ms();
    let session_ttl_ms = state.product_authentication.session.primary_max_age_ms;
    let session_id = match previous_session.as_ref() {
        Some(previous) => previous.session_id.clone(),
        None => match crate::product_auth::new_user_id() {
            Ok(session_id) => session_id,
            Err(error) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
            }
        },
    };
    let session = crate::store::ProductUserSession {
        token_hash: crate::admin::hex_sha256(token.as_bytes()),
        session_id,
        user_id: user.id.clone(),
        client_kind: previous_session.as_ref().map_or_else(
            || "browser".to_owned(),
            |session| session.client_kind.clone(),
        ),
        principal_class: "human".to_owned(),
        created_at_ms: now,
        expires_at_ms: now.saturating_add(session_ttl_ms),
        last_seen_at_ms: now,
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned),
        passkey_verified_at_ms: None,
        primary_authenticated_at_ms: now,
        primary_auth_method: Some(auth_method.to_owned()),
        revoked_at_ms: None,
        revoke_reason: None,
        auth_provider_id: (auth_method != crate::auth_plugins::PASSWORD_LOGIN_METHOD)
            .then(|| auth_method.to_owned()),
        auth_issuer: external_identity.map(|identity| identity.issuer.clone()),
        auth_subject: external_identity.map(|identity| identity.subject.clone()),
        auth_sid: external_identity.and_then(|identity| identity.session_id.clone()),
        id_token_ciphertext: previous_session
            .as_ref()
            .and_then(|session| session.id_token_ciphertext.clone()),
    };
    let persisted = if let (Some(previous), Some(previous_token_hash)) =
        (previous_session.as_ref(), previous_token_hash.as_deref())
    {
        debug_assert_eq!(previous.user_id, session.user_id);
        store
            .replace_user_session(previous_token_hash, &session)
            .await
            .map(|()| None)
    } else {
        store
            .admit_user_session(
                &session,
                state
                    .product_authentication
                    .capacity
                    .signed_in_sessions_per_user,
                state.product_authentication.capacity.enforcement
                    == crate::auth_plugins::CapacityEnforcement::Enforce,
                state.product_authentication.capacity.single_session_mode
                    == crate::auth_plugins::SingleSessionMode::NewestWins,
            )
            .await
            .map(Some)
    };
    let admission = match persisted {
        Ok(admission) => admission,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    match admission {
        Some(crate::store::ProductSessionAdmission::CapacityFull {
            active_sessions,
            limit,
        }) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "code": "signed_in_session_capacity_full",
                    "active": active_sessions,
                    "limit": limit,
                    "retryable": true,
                })),
            )
                .into_response();
        }
        Some(crate::store::ProductSessionAdmission::Admitted {
            observed_overflow,
            revoked_session_ids,
            ..
        }) => {
            if observed_overflow {
                tracing::warn!(
                    user_id = user.id,
                    "signed-in session capacity exceeded in observation mode"
                );
            }
            for revoked_session_id in revoked_session_ids {
                tracing::info!(
                    user_id = user.id,
                    session_id = revoked_session_id,
                    "product_session_replaced"
                );
            }
            state.capacity_runtime.notify_changed();
        }
        None => {}
    }
    let me = match product_me_for_user(
        Some(store),
        &state.hub,
        &state.product_authentication,
        user,
        Some(&session),
    )
    .await
    {
        Ok(me) => me,
        Err(error) => return product_session_policy_unavailable(error),
    };
    (
        [(
            header::SET_COOKIE,
            crate::product_auth::session_cookie(
                &token,
                crate::product_auth::request_is_https(headers),
                session_ttl_ms / 1_000,
            ),
        )],
        Json(me),
    )
        .into_response()
}

pub(super) async fn api_auth_status(
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

pub(super) async fn api_auth_create_automation_credential(
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

pub(super) async fn api_auth_setup(
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

pub(super) async fn api_auth_register(
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

pub(super) async fn api_auth_login(
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

pub(super) async fn api_auth_oidc_start(
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

pub(super) async fn api_auth_oidc_callback(
    State(state): State<ProductAuthState>,
    uri: Uri,
    headers: HeaderMap,
    Query(query): Query<OidcCallbackQuery>,
) -> Response {
    api_auth_oidc_callback_inner(state, uri, headers, query).await
}

pub(super) async fn api_auth_oidc_callback_form(
    State(state): State<ProductAuthState>,
    uri: Uri,
    headers: HeaderMap,
    Form(query): Form<OidcCallbackQuery>,
) -> Response {
    api_auth_oidc_callback_inner(state, uri, headers, query).await
}

pub(super) async fn api_auth_oidc_callback_inner(
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

pub(super) fn oidc_callback_target_error(
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

pub(super) fn oidc_browser_shell_completion_response(secure: bool) -> Response {
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

pub(super) fn oidc_admin_session_cookie(
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

pub(super) async fn api_auth_oidc_native_exchange(
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

pub(super) async fn api_auth_oidc_native_events(
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

pub(super) async fn handle_oidc_native_events(
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

pub(super) async fn api_auth_oidc_native_poll(
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

pub(super) async fn api_auth_oidc_native_cancel(
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

pub(super) async fn api_auth_oidc_native_complete() -> Response {
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

pub(super) fn oidc_callback_error(status: StatusCode, secure: bool) -> Response {
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

pub(super) async fn api_auth_logout(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    request: Option<Json<LogoutRequest>>,
) -> Response {
    let peer = peer_addr(peer);
    if crate::product_auth::user_cookie_token(&headers).is_some()
        && let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins)
    {
        return rejected;
    }
    let request = request.map_or_else(LogoutRequest::default, |Json(request)| request);
    let secure = crate::product_auth::request_is_https(&headers);
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "authentication storage is unavailable",
        )
            .into_response();
    };
    let Some(token) = crate::product_auth::user_cookie_token(&headers) else {
        return (
            [(
                header::SET_COOKIE,
                crate::product_auth::clear_session_cookie(secure),
            )],
            Json(serde_json::json!({
                "ok": true,
                "scope": request.scope,
                "revoked_sessions": 0,
                "provider_logout_url": null,
            })),
        )
            .into_response();
    };
    let session = match store
        .user_session_by_token_hash(&crate::admin::hex_sha256(token.as_bytes()))
        .await
    {
        Ok(session) => session,
        Err(error) => {
            tracing::error!(%error, "product_logout_session_lookup_failed");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "authentication storage is unavailable",
            )
                .into_response();
        }
    };
    let Some(session) = session else {
        return (
            [(
                header::SET_COOKIE,
                crate::product_auth::clear_session_cookie(secure),
            )],
            Json(serde_json::json!({
                "ok": true,
                "scope": request.scope,
                "revoked_sessions": 0,
                "provider_logout_url": null,
            })),
        )
            .into_response();
    };
    // This cookie session's enrollments end with it; a later account on the
    // same installation must not receive them.
    if let Err(error) = store
        .remove_web_push_subscriptions_of_session(&session.session_id)
        .await
    {
        tracing::warn!(%error, "removing Web Push subscriptions at logout");
    }
    let provider_id = session.auth_provider_id.as_deref();
    let offer_provider_logout = match state.product_authentication.logout.provider_logout {
        crate::auth_plugins::ProviderLogoutPolicy::Never => false,
        crate::auth_plugins::ProviderLogoutPolicy::Offer => request.provider_logout,
        crate::auth_plugins::ProviderLogoutPolicy::Always => provider_id.is_some(),
    };
    let provider_logout_url = offer_provider_logout
        .then_some(provider_id)
        .flatten()
        .and_then(|provider_id| state.product_authentication.provider(provider_id))
        .and_then(|provider| provider.rp_logout_location());
    let effective_scope =
        if provider_logout_url.is_some() && matches!(request.scope, LogoutScope::Current) {
            LogoutScope::Provider
        } else {
            request.scope
        };
    if matches!(effective_scope, LogoutScope::Provider | LogoutScope::All)
        && !product_session_has_recent_step_up(&session, auth_now_ms())
    {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            "Recent login or Passkey verification required",
        )
            .into_response();
    }
    let revoked_sessions = match effective_scope {
        LogoutScope::Current => store
            .delete_user_session(&session.token_hash)
            .await
            .map(|()| 1),
        LogoutScope::All => store.delete_user_sessions_for_user(&session.user_id).await,
        LogoutScope::Provider => {
            let Some(provider_id) = provider_id else {
                return (
                    StatusCode::BAD_REQUEST,
                    "the current session does not belong to an external provider",
                )
                    .into_response();
            };
            store
                .revoke_user_sessions_for_provider(
                    &session.user_id,
                    provider_id,
                    "provider_logout",
                    auth_now_ms(),
                )
                .await
        }
    };
    let revoked_sessions = match revoked_sessions {
        Ok(revoked_sessions) => revoked_sessions,
        Err(error) => {
            tracing::error!(%error, "product_logout_failed");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "authentication storage is unavailable",
            )
                .into_response();
        }
    };
    state.capacity_runtime.notify_changed();
    tracing::info!(
        user_id = session.user_id,
        ?effective_scope,
        revoked_sessions,
        provider_logout = provider_logout_url.is_some(),
        "product_logout"
    );
    (
        [(
            header::SET_COOKIE,
            crate::product_auth::clear_session_cookie(secure),
        )],
        Json(serde_json::json!({
            "ok": true,
            "scope": effective_scope,
            "revoked_sessions": revoked_sessions,
            "provider_logout_url": provider_logout_url,
        })),
    )
        .into_response()
}

pub(super) async fn api_auth_logout_complete(headers: HeaderMap) -> Response {
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, "/".to_owned()),
            (header::CACHE_CONTROL, "no-store".to_owned()),
            (
                header::SET_COOKIE,
                crate::product_auth::clear_session_cookie(crate::product_auth::request_is_https(
                    &headers,
                )),
            ),
        ],
    )
        .into_response()
}

pub(super) async fn api_auth_oidc_backchannel_logout(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    uri: Uri,
    Form(request): Form<OidcBackchannelLogoutRequest>,
) -> Response {
    if !state.product_auth_enabled || !state.product_authentication.logout.backchannel_logout {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(provider_id) = authentication_provider_id(uri.path(), "backchannel-logout") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(provider) = state.product_authentication.provider(provider_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let source_ip = peer_addr(peer).ip().to_string();
    let rate_name = format!("oidc:logout:{provider_id}");
    apply_rate_limit(&state, &rate_name, &source_ip).await;
    let now_ms = auth_now_ms();
    let Some(verified) = provider
        .verify_logout_token(
            &request.logout_token,
            u64::try_from(now_ms.div_euclid(1_000)).unwrap_or_default(),
        )
        .await
    else {
        state.rate_limits.record_failure(&rate_name, &source_ip);
        tracing::warn!(provider_id, "oidc_backchannel_logout_rejected");
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(store) = state.store.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let expires_at_ms = i64::try_from(verified.expires_at)
        .unwrap_or(i64::MAX)
        .saturating_mul(1_000);
    match store
        .apply_oidc_backchannel_logout(
            provider_id,
            &verified.issuer,
            verified.subject.as_deref(),
            verified.session_id.as_deref(),
            &verified.token_id,
            expires_at_ms,
            now_ms,
        )
        .await
    {
        Ok(crate::store::OidcBackchannelLogout::Applied { revoked_sessions }) => {
            state.rate_limits.reset(&rate_name, &source_ip);
            state.capacity_runtime.notify_changed();
            tracing::info!(
                provider_id,
                issued_at = verified.issued_at,
                revoked_sessions,
                "oidc_backchannel_logout_applied"
            );
            StatusCode::OK.into_response()
        }
        Ok(crate::store::OidcBackchannelLogout::Replay) => {
            state.rate_limits.record_failure(&rate_name, &source_ip);
            tracing::warn!(provider_id, "oidc_backchannel_logout_replay");
            StatusCode::BAD_REQUEST.into_response()
        }
        Err(error) => {
            tracing::error!(provider_id, %error, "oidc_backchannel_logout_failed");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

pub(super) async fn api_auth_me(
    State(state): State<ProductAuthState>,
    authenticated: Option<Extension<AuthenticatedProductRequest>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    if !state.product_auth_enabled {
        return Json(serde_json::json!({
            "account": "local",
            "user_id": crate::product_auth::local_product_principal().user_id,
            "role": "owner",
            "auth_enabled": false,
        }))
        .into_response();
    }
    let authenticated = match authenticated {
        Some(Extension(authenticated)) => authenticated,
        None => match resolve_product_api_request_principal(
            ProductRequestAuth::from(&state),
            &method,
            &uri,
            &headers,
        )
        .await
        {
            Ok(Some(authenticated)) => authenticated,
            Ok(None) | Err(()) => return StatusCode::UNAUTHORIZED.into_response(),
        },
    };
    let principal = authenticated.principal;
    let cookie_session = authenticated.cookie_session;
    let user = match state.store.as_ref() {
        Some(store) => store.user_by_id(&principal.user_id).await.ok().flatten(),
        None => None,
    };
    match user {
        Some(user) => match product_me_for_user(
            state.store.as_ref(),
            &state.hub,
            &state.product_authentication,
            &user,
            cookie_session.as_ref(),
        )
        .await
        {
            Ok(me) => no_store_json(StatusCode::OK, me),
            Err(error) => product_session_policy_unavailable(error),
        },
        None => {
            let mut me = product_me(&state.hub, &principal.username);
            me.user_id = Some(principal.user_id);
            no_store_json(StatusCode::OK, me)
        }
    }
}

pub(super) fn product_session_has_recent_step_up(
    session: &crate::store::ProductUserSession,
    now_ms: i64,
) -> bool {
    let verified_at = session
        .passkey_verified_at_ms
        .unwrap_or(session.created_at_ms);
    verified_at <= now_ms.saturating_add(60_000)
        && verified_at.saturating_add(PASSKEY_MANAGEMENT_STEP_UP_MAX_AGE_MS) > now_ms
}

pub(super) async fn api_auth_list_passkeys(
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

pub(super) async fn api_auth_passkey_register_options(
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

pub(super) async fn api_auth_passkey_register_complete(
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
pub(super) async fn persist_product_passkey_registration(
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

pub(super) async fn api_auth_passkey_assert_options(
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

pub(super) async fn api_auth_passkey_assert_complete(
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
pub(super) async fn persist_product_passkey_assertion(
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

pub(super) fn product_passkey_assertion_response(
    result: ProductPasskeyAssertionResult,
) -> Response {
    match result.set_cookie {
        Some(cookie) => ([(header::SET_COOKIE, cookie)], Json(result.me)).into_response(),
        None => Json(result.me).into_response(),
    }
}

pub(super) async fn api_auth_passkey_external_start(
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

pub(super) async fn api_auth_passkey_external_options(
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

pub(super) async fn api_auth_passkey_external_complete(
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

pub(super) async fn api_auth_passkey_external_fail(
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

pub(super) async fn api_auth_passkey_external_events(
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

pub(super) async fn handle_passkey_external_events(
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

pub(super) async fn api_auth_passkey_external_finalize(
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

pub(super) async fn api_auth_passkey_reauth(
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

pub(super) async fn api_auth_delete_passkey(
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

pub(super) fn device_authorization_error(error: &anyhow::Error) -> Response {
    tracing::info!(%error, "device_authorization_rejected");
    let message = error.to_string();
    let status = if message.contains("too many") {
        StatusCode::TOO_MANY_REQUESTS
    } else if message.contains("unavailable") || message.contains("no longer") {
        StatusCode::GONE
    } else {
        StatusCode::BAD_REQUEST
    };
    (status, "device authorization is unavailable").into_response()
}

pub(super) fn device_verification_url(
    public_origins: &[String],
    request_id: &str,
    approval_token: &str,
) -> String {
    let path = format!("/auth/device#request_id={request_id}&approval_token={approval_token}");
    public_origins
        .first()
        .map_or(path.clone(), |origin| format!("{origin}{path}"))
}

pub(super) async fn api_auth_device_authorization_start(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::client_auth::StartAuthorizationRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if durable_store(&state.store).is_none() {
        return missing_store();
    }
    let source_ip = crate::product_auth::client_ip(&headers, peer_addr(peer));
    match state
        .device_authorizations
        .start(request, source_ip, auth_now_ms())
    {
        Ok((request_id, approval_token, expires_at_ms)) => {
            let verification_url = device_verification_url(
                state.public_origins.as_ref(),
                &request_id,
                &approval_token,
            );
            no_store_json(
                StatusCode::CREATED,
                crate::client_auth::StartAuthorizationResponse {
                    request_id,
                    verification_url,
                    expires_at_ms,
                },
            )
        }
        Err(error) => device_authorization_error(&error),
    }
}

pub(super) async fn api_auth_device_authorization_inspect(
    State(state): State<ProductAuthState>,
    Json(request): Json<crate::client_auth::BrowserAuthorizationRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    match state
        .device_authorizations
        .browser_info(&request.request_id, &request.approval_token)
    {
        Ok(info) => no_store_json(StatusCode::OK, info),
        Err(error) => device_authorization_error(&error),
    }
}

pub(super) async fn api_auth_device_authorization_approve(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::client_auth::BrowserAuthorizationRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(rejected) = reject_bad_origin(&headers, peer_addr(peer), &state.public_origins) {
        return rejected;
    }
    let user = match require_recent_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state.device_authorizations.approve(&request, &user.id) {
        Ok(()) => {
            tracing::info!(
                user_id = user.id,
                request_id = request.request_id,
                "device_authorization_approved"
            );
            no_store_json(StatusCode::OK, serde_json::json!({ "ok": true }))
        }
        Err(error) => device_authorization_error(&error),
    }
}

pub(super) async fn api_auth_device_authorization_deny(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::client_auth::BrowserAuthorizationRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(rejected) = reject_bad_origin(&headers, peer_addr(peer), &state.public_origins) {
        return rejected;
    }
    if let Err(response) = require_product_user(&state, &headers).await {
        return response;
    }
    match state.device_authorizations.deny(&request) {
        Ok(()) => no_store_json(StatusCode::OK, serde_json::json!({ "ok": true })),
        Err(error) => device_authorization_error(&error),
    }
}

pub(super) async fn api_auth_device_authorization_events(
    State(state): State<ProductAuthState>,
    ws: WebSocketUpgrade,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    ws.max_message_size(4 * 1_024)
        .max_frame_size(4 * 1_024)
        .on_upgrade(move |socket| handle_device_authorization_events(socket, state))
}

pub(super) async fn handle_device_authorization_events(
    mut socket: WebSocket,
    state: ProductAuthState,
) {
    let handshake =
        match tokio::time::timeout(DEVICE_AUTH_EVENTS_HANDSHAKE_TIMEOUT, socket.recv()).await {
            Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<
                crate::client_auth::AuthorizationEventsHandshake,
            >(text.as_str())
            .ok(),
            _ => None,
        };
    let Some(handshake) = handshake else {
        let _ = socket.close().await;
        return;
    };
    let mut events = match state
        .device_authorizations
        .subscribe(&handshake.request_id, &handshake.code_verifier)
    {
        Ok(events) => events,
        Err(error) => {
            tracing::info!(%error, "device_authorization_events_rejected");
            let _ = send_json(
                &mut socket,
                &crate::client_auth::AuthorizationEventStatus {
                    status: "unavailable".to_owned(),
                },
            )
            .await;
            let _ = socket.close().await;
            return;
        }
    };
    let deadline = tokio::time::sleep(DEVICE_AUTH_EVENTS_TTL);
    tokio::pin!(deadline);
    let mut heartbeat = tokio::time::interval(DEVICE_AUTH_EVENTS_HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await;

    loop {
        let terminal = match *events.borrow_and_update() {
            crate::client_auth::AuthorizationEvent::Pending => None,
            crate::client_auth::AuthorizationEvent::Approved => Some("approved"),
            crate::client_auth::AuthorizationEvent::Denied => Some("denied"),
        };
        if let Some(status) = terminal {
            let _ = send_json(
                &mut socket,
                &crate::client_auth::AuthorizationEventStatus {
                    status: status.to_owned(),
                },
            )
            .await;
            let _ = socket.close().await;
            return;
        }
        tokio::select! {
            changed = events.changed() => {
                if changed.is_err() {
                    let _ = send_json(
                        &mut socket,
                        &crate::client_auth::AuthorizationEventStatus {
                            status: "unavailable".to_owned(),
                        },
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
                    &crate::client_auth::AuthorizationEventStatus {
                        status: "unavailable".to_owned(),
                    },
                ).await;
                let _ = socket.close().await;
                return;
            }
        }
    }
}

pub(super) async fn api_auth_cardea_device_configuration(
    State(state): State<ProductAuthState>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(provider) = state.product_authentication.provider("cardea") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match provider.cardea_device_configuration() {
        Ok(configuration) => Json(configuration).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(super) async fn api_auth_cardea_device_exchange(
    State(state): State<ProductAuthState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<CardeaDeviceExchangeRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(provider) = state.product_authentication.provider("cardea") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let source_ip = crate::product_auth::client_ip(&headers, peer);
    apply_rate_limit(&state, "cardea-device-exchange", &source_ip.to_string()).await;
    if request.name.is_empty()
        || request.name.chars().count() > 80
        || request
            .name
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>'))
        || crate::client_auth::decode_public_key(&request.public_key).is_err()
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let user = match store.user_by_username(provider.account()).await {
        Ok(Some(user)) if user.disabled_at_ms.is_none() => user,
        Ok(_) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let identity = match provider
        .exchange_cardea_device(
            &request.access_token,
            &request.assertion,
            &request.public_key,
        )
        .await
    {
        Ok(identity) => identity,
        Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
    };
    // One stable Cowboy device per Cardea grant; refresh cannot bypass local
    // device revocation or allocate a new capacity slot on every request.
    let id = match provider.cardea_product_device_id(&identity.grant_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let devices = match store.list_user_devices_for_user(&user.id).await {
        Ok(devices) => devices,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let now = auth_now_ms();
    if let Some(device) = devices.iter().find(|d| d.id == id) {
        if device.public_key != identity.public_key {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    } else {
        let device = crate::store::ProductDevice {
            id: id.clone(),
            user_id: user.id.clone(),
            name: request.name,
            public_key: identity.public_key.clone(),
            created_at_ms: now,
            last_used_at_ms: Some(now),
            revoked_at_ms: None,
        };
        // Admission owns capacity and its transactional lock. No product
        // refresh bearer is returned: subsequent mints recheck Cardea.
        let secret = match crate::client_auth::new_refresh_token() {
            Ok(v) => v,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        let refresh = crate::store::ProductDeviceRefreshToken {
            token_hash: crate::admin::hex_sha256(secret.as_bytes()),
            device_id: id.clone(),
            family_id: id.clone(),
            created_at_ms: now,
            expires_at_ms: now + 1,
            used_at_ms: None,
            revoked_at_ms: Some(now),
        };
        match store
            .admit_user_device(
                &device,
                &refresh,
                state
                    .product_authentication
                    .capacity
                    .authorized_clients_per_user,
                state.product_authentication.capacity.enforcement
                    == crate::auth_plugins::CapacityEnforcement::Enforce,
            )
            .await
        {
            Ok(crate::store::ProductDeviceAdmission::Admitted { .. }) => {}
            Ok(crate::store::ProductDeviceAdmission::CapacityFull { .. }) => {
                return StatusCode::CONFLICT.into_response();
            }
            // A revoked device keeps its primary key: insertion fails closed.
            // Concurrent first admissions can safely retry with fresh evidence.
            Err(_) => return StatusCode::CONFLICT.into_response(),
        }
    }
    let deadline = match i64::try_from(identity.expires_at)
        .ok()
        .and_then(|t| t.checked_mul(1000))
    {
        Some(t) => t,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    }
    .min(now.saturating_add(300_000));
    match state.device_access.issue_identity_device(
        &id,
        &user.id,
        &identity.public_key,
        now,
        deadline,
    ) {
        Ok((token, expiry)) => no_store_json(
            StatusCode::OK,
            serde_json::json!({
                "schema":"dravengarden.cowboy.cardea-device-token/v1", "device_id":id,
                "access_token":token, "access_expires_at_ms":expiry,
            }),
        ),
        Err(_) => StatusCode::UNAUTHORIZED.into_response(),
    }
}

pub(super) async fn api_auth_device_exchange(
    State(state): State<ProductAuthState>,
    Json(request): Json<crate::client_auth::ExchangeRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let approved = match state
        .device_authorizations
        .begin_exchange(&request, auth_now_ms())
    {
        Ok(approved) => approved,
        Err(error) => return device_authorization_error(&error),
    };
    let finish_failed = || {
        state
            .device_authorizations
            .finish_exchange(&request.request_id, false)
    };
    let user = match store.user_by_id(&approved.user_id).await {
        Ok(Some(user)) if user.disabled_at_ms.is_none() => user,
        Ok(_) => {
            finish_failed();
            return StatusCode::UNAUTHORIZED.into_response();
        }
        Err(error) => {
            finish_failed();
            tracing::error!(%error, "device_authorization_user_lookup");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let now = auth_now_ms();
    let device_id = match crate::product_auth::new_user_id() {
        Ok(id) => id,
        Err(error) => {
            finish_failed();
            tracing::error!(%error, "device_id_generation");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let family_id = match crate::product_auth::new_user_id() {
        Ok(id) => id,
        Err(error) => {
            finish_failed();
            tracing::error!(%error, "device_refresh_family_generation");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let refresh_token = match crate::client_auth::new_refresh_token() {
        Ok(token) => token,
        Err(error) => {
            finish_failed();
            tracing::error!(%error, "device_refresh_generation");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let refresh_expires_at_ms = now.saturating_add(crate::client_auth::REFRESH_TOKEN_TTL_MS);
    let device = crate::store::ProductDevice {
        id: device_id.clone(),
        user_id: user.id.clone(),
        name: approved.name,
        public_key: approved.public_key,
        created_at_ms: now,
        last_used_at_ms: Some(now),
        revoked_at_ms: None,
    };
    let refresh = crate::store::ProductDeviceRefreshToken {
        token_hash: crate::admin::hex_sha256(refresh_token.as_bytes()),
        device_id: device_id.clone(),
        family_id,
        created_at_ms: now,
        expires_at_ms: refresh_expires_at_ms,
        used_at_ms: None,
        revoked_at_ms: None,
    };
    let admission = store
        .admit_user_device(
            &device,
            &refresh,
            state
                .product_authentication
                .capacity
                .authorized_clients_per_user,
            state.product_authentication.capacity.enforcement
                == crate::auth_plugins::CapacityEnforcement::Enforce,
        )
        .await;
    match admission {
        Ok(crate::store::ProductDeviceAdmission::Admitted {
            observed_overflow, ..
        }) => {
            if observed_overflow {
                tracing::warn!(
                    user_id = user.id,
                    "authorized-client capacity exceeded in observation mode"
                );
            }
        }
        Ok(crate::store::ProductDeviceAdmission::CapacityFull {
            active_clients,
            limit,
        }) => {
            finish_failed();
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "code": "authorized_client_capacity_full",
                    "active": active_clients,
                    "limit": limit,
                    "retryable": false,
                    "action": "revoke_an_authorized_client",
                })),
            )
                .into_response();
        }
        Err(error) => {
            finish_failed();
            tracing::error!(%error, "device_authorization_persist");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    let (access_token, access_expires_at_ms) =
        match state
            .device_access
            .issue(&device.id, &device.user_id, &device.public_key, now)
        {
            Ok(access) => access,
            Err(error) => {
                let _ = store.revoke_user_device(&device.id, now).await;
                finish_failed();
                tracing::error!(%error, "device_access_issue");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
    state
        .device_authorizations
        .finish_exchange(&request.request_id, true);
    tracing::info!(
        user_id = user.id,
        device_id,
        "device_authorization_complete"
    );
    no_store_json(
        StatusCode::OK,
        crate::client_auth::DeviceTokenResponse {
            device_id,
            access_token,
            access_expires_at_ms,
            refresh_token,
            refresh_expires_at_ms,
        },
    )
}

pub(super) async fn api_auth_device_refresh(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let Some(refresh_token) = refresh_bearer(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let token_hash = crate::admin::hex_sha256(refresh_token.as_bytes());
    let (device, previous) = match store.user_device_refresh_by_hash(&token_hash).await {
        Ok(Some(value)) => value,
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(error) => {
            tracing::error!(%error, "device_refresh_lookup");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let now = auth_now_ms();
    // Refresh tokens are sender-constrained. Authenticate the key before
    // treating reuse as theft; possession of an obsolete token alone must not
    // let an attacker revoke the legitimate device as a denial of service.
    if let Err(error) = state.device_access.verify_token_proof(
        &headers,
        crate::client_auth::TokenProofContext {
            method: &Method::POST,
            path_and_query: "/api/auth/device/refresh",
            token: &refresh_token,
            device_id: &device.id,
            public_key: &device.public_key,
            now_ms: now,
        },
    ) {
        tracing::info!(%error, device_id = device.id, "device_refresh_proof_rejected");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if previous.used_at_ms.is_some() {
        tracing::warn!(device_id = device.id, "device_refresh_replay_detected");
        revoke_replayed_device(&state, store, &device.id, now).await;
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if previous.revoked_at_ms.is_some()
        || device.revoked_at_ms.is_some()
        || previous.expires_at_ms <= now
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let user = match store.user_by_id(&device.user_id).await {
        Ok(Some(user)) if user.disabled_at_ms.is_none() => user,
        Ok(_) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(error) => {
            tracing::error!(%error, "device_refresh_user_lookup");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let replacement = match crate::client_auth::new_refresh_token() {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "device_refresh_generation");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let next = crate::store::ProductDeviceRefreshToken {
        token_hash: crate::admin::hex_sha256(replacement.as_bytes()),
        device_id: device.id.clone(),
        family_id: previous.family_id,
        created_at_ms: now,
        expires_at_ms: previous.expires_at_ms,
        used_at_ms: None,
        revoked_at_ms: None,
    };
    match store
        .rotate_user_device_refresh(&token_hash, &next, now)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(device_id = device.id, "device_refresh_rotation_race");
            revoke_replayed_device(&state, store, &device.id, now).await;
            return StatusCode::UNAUTHORIZED.into_response();
        }
        Err(error) => {
            tracing::error!(%error, "device_refresh_rotate");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    let (access_token, access_expires_at_ms) =
        match state
            .device_access
            .issue(&device.id, &user.id, &device.public_key, now)
        {
            Ok(access) => access,
            Err(error) => {
                tracing::error!(%error, "device_access_refresh_issue");
                revoke_replayed_device(&state, store, &device.id, now).await;
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
    no_store_json(
        StatusCode::OK,
        crate::client_auth::DeviceTokenResponse {
            device_id: device.id,
            access_token,
            access_expires_at_ms,
            refresh_token: replacement,
            refresh_expires_at_ms: next.expires_at_ms,
        },
    )
}

pub(super) fn device_public_json(device: crate::store::ProductDevice) -> serde_json::Value {
    serde_json::json!({
        "id": device.id,
        "name": device.name,
        "created_at_ms": device.created_at_ms,
        "last_used_at_ms": device.last_used_at_ms,
    })
}

pub(super) async fn api_auth_list_sessions(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let Some((session, user)) = product_session_and_user_from_store_cookie(store, &headers).await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if let Err(response) =
        ensure_product_session_fresh(store, &session, &state.product_authentication).await
    {
        return response;
    }
    let now_ms = auth_now_ms();
    match tokio::try_join!(
        store.list_user_sessions_for_user(&user.id, now_ms),
        store.list_active_clients_for_user(&user.id, now_ms),
        store.list_user_devices_for_user(&user.id),
    ) {
        Ok((sessions, active_clients, authorized_clients)) => no_store_json(
            StatusCode::OK,
            serde_json::json!({
                "sessions": sessions
                    .into_iter()
                    .map(|item| session_public_json(item, Some(&session.session_id)))
                    .collect::<Vec<_>>(),
                "active_clients": active_clients,
                "authorized_clients": authorized_clients.len(),
                "limit": state.product_authentication.capacity.signed_in_sessions_per_user,
                "active_limit": state.product_authentication.capacity.active_clients_per_user,
                "enforcement": state.product_authentication.capacity.enforcement,
            }),
        ),
        Err(error) => {
            tracing::error!(%error, "user_session_list");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

pub(super) async fn api_auth_delete_session(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(rejected) = reject_bad_origin(&headers, peer_addr(peer), &state.public_origins) {
        return rejected;
    }
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let Some((current_session, user)) =
        product_session_and_user_from_store_cookie(store, &headers).await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if let Err(response) =
        ensure_product_session_fresh(store, &current_session, &state.product_authentication).await
    {
        return response;
    }
    if !product_session_has_recent_step_up(&current_session, auth_now_ms()) {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            "Recent login or Passkey verification required",
        )
            .into_response();
    }
    match store
        .revoke_user_session_for_user(&user.id, &id, "user_revoked", auth_now_ms())
        .await
    {
        Ok(0) => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => {
            tracing::info!(
                user_id = user.id,
                session_id = id,
                current = id == current_session.session_id,
                "user_session_revoked"
            );
            state.capacity_runtime.notify_changed();
            let mut response = no_store_json(StatusCode::OK, serde_json::json!({ "ok": true }));
            if id == current_session.session_id
                && let Ok(value) = crate::product_auth::clear_session_cookie(
                    crate::product_auth::request_is_https(&headers),
                )
                .parse()
            {
                response.headers_mut().append(header::SET_COOKIE, value);
            }
            response
        }
        Err(error) => {
            tracing::error!(%error, "user_session_revoke");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

pub(super) async fn api_auth_release_active_client(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    Query(query): Query<ActiveClientReleaseQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(rejected) = reject_bad_origin(&headers, peer_addr(peer), &state.public_origins) {
        return rejected;
    }
    if query.fencing_token <= 0 {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let user = match require_fresh_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    match store
        .release_active_client_for_user(
            &user.id,
            &ActiveClientRelease {
                client_id: &id,
                fencing_token: query.fencing_token,
                reason: "user_reclaimed",
                now_ms: auth_now_ms(),
                reservation_ms: state.product_authentication.capacity.reservation_ms,
                per_user_limit: state
                    .product_authentication
                    .capacity
                    .active_clients_per_user,
            },
        )
        .await
    {
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Ok(true) => {
            tracing::info!(user_id = user.id, client_id = id, "active_client_reclaimed");
            state.capacity_runtime.notify_changed();
            no_store_json(StatusCode::OK, serde_json::json!({ "ok": true }))
        }
        Err(error) => {
            tracing::error!(%error, "active_client_reclaim");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

pub(super) async fn api_auth_list_devices(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    let user = match require_fresh_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    match store.list_user_devices_for_user(&user.id).await {
        Ok(devices) => no_store_json(
            StatusCode::OK,
            serde_json::json!({
                "devices": devices.into_iter().map(device_public_json).collect::<Vec<_>>(),
            }),
        ),
        Err(error) => {
            tracing::error!(%error, "device_list");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

pub(super) async fn api_auth_delete_device(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(rejected) = reject_bad_origin(&headers, peer_addr(peer), &state.public_origins) {
        return rejected;
    }
    let user = match require_recent_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    match store
        .revoke_user_device_for_user(&user.id, &id, auth_now_ms())
        .await
    {
        Ok(0) => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => {
            state.device_access.revoke_device(&id);
            tracing::info!(user_id = user.id, device_id = id, "user_device_revoked");
            no_store_json(StatusCode::OK, serde_json::json!({ "ok": true }))
        }
        Err(error) => {
            tracing::error!(%error, "device_revoke");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

pub(super) async fn api_auth_list_tokens(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    let Some((principal, cookie_session)) = resolve_product_request_principal(
        state.product_auth_enabled,
        state.store.as_ref(),
        &state.hub,
        &headers,
    )
    .await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    if let Some(session) = cookie_session.as_ref()
        && let Err(response) =
            ensure_product_session_fresh(store, session, &state.product_authentication).await
    {
        return response;
    }
    match store
        .list_user_api_tokens_for_user(&principal.user_id)
        .await
    {
        Ok(tokens) => Json(serde_json::json!({
            "tokens": tokens.iter().map(token_public_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) async fn api_auth_create_token(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<CreateApiTokenRequest>,
) -> Response {
    let peer = peer_addr(peer);
    if crate::product_auth::bearer_token(&headers).is_none()
        && let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins)
    {
        return rejected;
    }
    let Some((principal, cookie_session)) = resolve_product_request_principal(
        state.product_auth_enabled,
        state.store.as_ref(),
        &state.hub,
        &headers,
    )
    .await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if !principal.role.at_least(crate::admin::AdminRole::Operator) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(store) = durable_store(&state.store).cloned() else {
        return missing_store();
    };
    if let Some(session) = cookie_session.as_ref()
        && let Err(response) =
            ensure_product_session_fresh(&store, session, &state.product_authentication).await
    {
        return response;
    }
    let name = match crate::product_auth::ensure_api_token_name(&request.name) {
        Ok(name) => name,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let ttl_secs = match crate::product_auth::api_token_ttl_secs(request.ttl_seconds) {
        Ok(ttl) => ttl,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let secret = match crate::product_auth::new_api_token_secret() {
        Ok(secret) => secret,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    let now = auth_now_ms();
    let token = crate::store::ProductApiToken {
        id: match crate::product_auth::new_user_id() {
            Ok(id) => id,
            Err(error) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
            }
        },
        user_id: principal.user_id.clone(),
        name,
        token_prefix: crate::product_auth::api_token_prefix(&secret),
        token_hash: crate::admin::hex_sha256(secret.as_bytes()),
        created_at_ms: now,
        expires_at_ms: Some(now.saturating_add(ttl_secs.saturating_mul(1_000))),
        last_used_at_ms: None,
        revoked_at_ms: None,
    };
    if let Err(error) = store.insert_user_api_token(&token).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    tracing::info!(
        user_id = principal.user_id,
        token_id = token.id,
        token_prefix = %token.token_prefix,
        "product_api_token_created"
    );
    let mut body = token_public_json(&token);
    body["token"] = serde_json::Value::String(secret);
    (StatusCode::CREATED, Json(body)).into_response()
}

pub(super) async fn api_auth_delete_token(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let peer = peer_addr(peer);
    if crate::product_auth::bearer_token(&headers).is_none()
        && let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins)
    {
        return rejected;
    }
    let Some((principal, cookie_session)) = resolve_product_request_principal(
        state.product_auth_enabled,
        state.store.as_ref(),
        &state.hub,
        &headers,
    )
    .await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    if let Some(session) = cookie_session.as_ref()
        && let Err(response) =
            ensure_product_session_fresh(store, session, &state.product_authentication).await
    {
        return response;
    }
    match store
        .revoke_user_api_token_for_user(&principal.user_id, &id)
        .await
    {
        Ok(0) => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => {
            tracing::info!(
                user_id = principal.user_id,
                token_id = id,
                "product_api_token_revoked"
            );
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) fn product_machine_is_visible(
    machine: &crate::store::MachineRecord,
    product_auth_enabled: bool,
) -> bool {
    !machine.revoked && (!product_auth_enabled || machine.fingerprint.is_some())
}

pub(super) fn product_session_owner(
    product_auth_enabled: bool,
    principal: &ProductPrincipal,
) -> Option<crate::supervisor::SessionOwner<'_>> {
    product_auth_enabled.then_some(crate::supervisor::SessionOwner {
        user_id: &principal.user_id,
        username: Some(&principal.username),
    })
}

pub(super) fn product_auth_deadline_instant(due_at_ms: i64) -> tokio::time::Instant {
    const MAX_DELAY_MS: u64 = 91 * 24 * 60 * 60 * 1_000;
    let delay_ms = (due_at_ms.saturating_sub(auth_now_ms()).max(1) as u64).min(MAX_DELAY_MS);
    tokio::time::Instant::now() + std::time::Duration::from_millis(delay_ms)
}

pub(super) async fn product_auth_session_message(
    state: &AppState,
    cookie_token: &str,
) -> Option<ProductAuthSessionSnapshot> {
    let store = state.store.as_ref()?;
    let session = store
        .user_session_by_token_hash(&crate::admin::hex_sha256(cookie_token.as_bytes()))
        .await
        .ok()??;
    if session.expires_at_ms <= auth_now_ms() {
        return None;
    }
    let user = store.user_by_id(&session.user_id).await.ok()??;
    if user.disabled_at_ms.is_some() {
        return None;
    }
    let user_id = user.id.clone();
    let me = product_me_for_user(
        Some(store),
        &state.hub,
        &state.product_authentication,
        &user,
        Some(&session),
    )
    .await
    .ok()?;
    let valid = me.session_reauth_kind.is_none();
    let next_due_at_ms = [
        me.primary_reauth_due_at_ms,
        me.passkey_reauth_due_at_ms,
        me.session_idle_due_at_ms,
    ]
    .into_iter()
    .flatten()
    .min()?;
    let session = serde_json::to_value(me).ok()?;
    Some(ProductAuthSessionSnapshot {
        message: Outbound::AuthSession { session },
        valid,
        user_id,
        next_due_at_ms,
    })
}
