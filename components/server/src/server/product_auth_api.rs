//! Product login, sessions, passkeys, OIDC, device authorization and API
//! tokens: the /api/auth handlers and the principal resolution they share.

use super::*;

mod login;
pub(super) use login::*;
mod oidc;
pub(super) use oidc::*;
mod passkeys;
pub(super) use passkeys::*;
mod devices;
pub(super) use devices::*;
mod credentials;
pub(super) use credentials::*;

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
