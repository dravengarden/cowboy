//! Request policy shared by the HTTP handlers: route classification, origin
//! and rate checks, admin and product-user guards, session visibility and the
//! small auth persistence helpers.

use super::*;

pub(super) fn registration_policy(hub: &Hub) -> crate::admin::RegistrationPolicy {
    crate::admin::RegistrationPolicy::from_setting(
        hub.settings_snapshot()
            .get(crate::admin::REGISTRATION_SETTING),
    )
}

pub(super) fn permission_policy(hub: &Hub) -> crate::admin::PermissionPolicy {
    crate::admin::PermissionPolicy::from_setting(
        hub.settings_snapshot()
            .get(crate::admin::PERMISSIONS_SETTING),
    )
}

pub(super) fn persist_admin_identities(hub: &Hub, identities: &crate::admin::AdminIdentities) {
    persist_admin_setting(hub, crate::admin::ADMIN_IDENTITIES_SETTING, identities);
}

pub(super) fn persist_admin_setting(hub: &Hub, key: &str, value: &impl Serialize) {
    hub.set_setting(
        key.to_owned(),
        serde_json::to_value(value).unwrap_or_else(|_| serde_json::json!({})),
    );
}

pub(super) async fn enrich_admin_status(
    store: Option<&Store>,
    identities: &crate::admin::AdminIdentities,
    mut status: crate::admin::AdminAuthStatus,
) -> crate::admin::AdminAuthStatus {
    let Some(account) = status.account.as_deref() else {
        return status;
    };
    let count = match store {
        Some(store) => store.count_admin_passkeys(account).await.unwrap_or(0),
        None => 0,
    };
    let policy = identities.passkey_policy(account, count);
    status.passkey_count = policy.passkey_count;
    status.passkey_reauth_enabled = policy.enabled;
    status.passkey_reauth_required = policy.reauth_eligible();
    status
}

pub(super) fn auth_now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(super) fn peer_addr(ConnectInfo(peer): ConnectInfo<SocketAddr>) -> SocketAddr {
    peer
}

pub(super) fn reject_bad_origin(
    headers: &HeaderMap,
    peer: SocketAddr,
    public_origins: &[String],
) -> Option<Response> {
    if crate::product_auth::origin_allowed(headers, peer, public_origins) {
        None
    } else {
        Some(StatusCode::FORBIDDEN.into_response())
    }
}

pub(super) fn reject_insecure_admin(headers: &HeaderMap, peer: SocketAddr) -> Option<Response> {
    if !peer.ip().is_loopback() {
        return Some((StatusCode::FORBIDDEN, "admin login requires HTTPS").into_response());
    }
    if crate::product_auth::request_is_https(headers)
        || !crate::product_auth::has_forwarded_client_headers(headers)
    {
        return None;
    }
    Some((StatusCode::FORBIDDEN, "admin login requires HTTPS").into_response())
}

pub(super) async fn apply_rate_limit(state: &ProductAuthState, username: &str, ip: &str) {
    if let Some(delay) = state.rate_limits.delay(username, ip) {
        tokio::time::sleep(delay).await;
    }
}

pub(super) fn require_admin(
    state: &ProductAuthState,
    headers: &HeaderMap,
    minimum: crate::admin::AdminRole,
) -> Result<crate::admin::AdminPrincipal, StatusCode> {
    require_admin_role(&state.hub, headers, minimum)
}

pub(super) fn require_admin_role(
    hub: &Hub,
    headers: &HeaderMap,
    minimum: crate::admin::AdminRole,
) -> Result<crate::admin::AdminPrincipal, StatusCode> {
    let Some(token) = crate::admin::cookie_token(headers) else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    let Some(principal) = admin_identities(hub).principal(&token, auth_now_ms()) else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    if principal.role.at_least(minimum) {
        Ok(principal)
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

pub(super) fn classify_route(method: &Method, path: &str) -> RouteAuth {
    if public_route(path) {
        return RouteAuth::Public;
    }
    if path == "/metrics" {
        return RouteAuth::MetricsScrape;
    }
    if path == "/api/telemetry/binding" || path.starts_with("/api/telemetry/binding/") {
        return RouteAuth::ProductOrAdminOperator;
    }
    if path == "/api/code/buffers"
        || path.starts_with("/api/code/buffers/")
        || path.starts_with("/api/code/buffer-synchronizations/")
        || path.starts_with("/api/code/navigations/")
    {
        return RouteAuth::ProductOperator;
    }
    if path == "/api/preferences"
        || path.starts_with("/api/preferences/")
        || path == "/api/superagent/activity"
    {
        return RouteAuth::Product;
    }
    if path == "/api/provider-accounts" || path.starts_with("/api/provider-accounts/") {
        return RouteAuth::ProductOperator;
    }
    if path == "/api/drafts"
        || path.starts_with("/api/drafts/")
        || path == "/api/folders"
        || path.starts_with("/api/folders/")
    {
        return if matches!(*method, Method::GET | Method::HEAD) {
            RouteAuth::Product
        } else {
            RouteAuth::ProductOperator
        };
    }
    if path == "/ws" {
        return RouteAuth::Product;
    }
    if path == "/api/auth/tokens" {
        return if method == Method::POST {
            RouteAuth::ProductOperator
        } else {
            RouteAuth::Product
        };
    }
    if path == "/api/auth/automation/credentials" {
        return RouteAuth::ProductOperator;
    }
    if path.starts_with("/api/auth/tokens/") {
        return RouteAuth::Product;
    }
    if path == "/api/auth/sessions"
        || path.starts_with("/api/auth/sessions/")
        || path.starts_with("/api/auth/active-clients/")
    {
        return RouteAuth::Product;
    }
    if matches!(
        path,
        "/api/auth/device/authorizations"
            | "/api/auth/device/authorizations/inspect"
            | "/api/auth/device/authorizations/events"
            | "/api/auth/device/exchange"
            | "/api/auth/device/cardea"
            | "/api/auth/device/refresh"
    ) {
        return RouteAuth::Public;
    }
    if matches!(
        path,
        "/api/auth/device/authorizations/approve"
            | "/api/auth/device/authorizations/deny"
            | "/api/auth/devices"
    ) || path.starts_with("/api/auth/devices/")
    {
        return RouteAuth::Product;
    }
    if matches!(
        path,
        "/api/auth/passkeys/external/options"
            | "/api/auth/passkeys/external/complete"
            | "/api/auth/passkeys/external/fail"
    ) {
        return RouteAuth::Public;
    }
    if path.starts_with("/api/auth/passkeys") {
        return RouteAuth::Product;
    }
    if path.starts_with("/api/machine/") {
        return RouteAuth::Public;
    }
    // Refreshing republishes a Machine's read-only inventory. The Web and
    // Operators reach it signed in; host deployment checks reach it through
    // `internal_machine_refresh` before any principal is resolved.
    if method == Method::POST && machine_refresh_path(path) {
        return RouteAuth::ProductOrAdminOperator;
    }
    // Release transactions need a narrow, credential-free liveness signal on
    // both the Controller host and remote Machines. Keep the full inventory
    // protected; this endpoint exposes only connection state and the active
    // ACP generation required to prove a rollout.
    if method == Method::GET
        && path.starts_with("/api/machines/")
        && path.ends_with("/deployment-health")
    {
        return RouteAuth::Public;
    }
    if matches!(path, "/api/sessions" | "/api/execution-sessions") && method == Method::POST {
        return RouteAuth::ProductOperator;
    }
    // Content-free durable-state counts of one Machine, for operators and AI
    // diagnosis. It exposes no Session, lineage or path.
    if matches!(*method, Method::GET | Method::HEAD)
        && path.starts_with("/api/machines/")
        && path.ends_with("/durable-state")
    {
        return RouteAuth::ProductOrAdminOperator;
    }
    if matches!(path, "/api/project-policies" | "/api/project-placements")
        || (path.starts_with("/api/machines/")
            && (path.ends_with("/projects") || path.ends_with("/project-policy")))
    {
        return if matches!(*method, Method::GET | Method::HEAD) {
            RouteAuth::Product
        } else {
            RouteAuth::ProductOperator
        };
    }
    if path == "/api/sessions/reconcile-project" {
        return RouteAuth::AdminOperator;
    }
    if path == "/api/artifacts" || path.starts_with("/api/artifacts/") {
        return RouteAuth::Product;
    }
    if session_id_from_path(path).is_some() {
        return if matches!(*method, Method::GET | Method::HEAD) {
            RouteAuth::ProductSessionSee
        } else {
            RouteAuth::ProductSessionMutate
        };
    }
    // Notification enrollment belongs to the signed-in product user. Leaving
    // these routes to the API fallback incorrectly requires an admin cookie.
    // Product authentication still enforces session freshness and mutation origin.
    if path == "/api/notifications/presence" {
        return RouteAuth::Product;
    }
    if (path == "/api/web-push/config" && matches!(*method, Method::GET | Method::HEAD))
        || (path == "/api/web-push/subscription" && matches!(*method, Method::PUT | Method::DELETE))
    {
        return RouteAuth::Product;
    }
    // Per-agent Tools defaults are a product Settings page. The API fallback
    // demanded an admin cookie, so every product user got 401. Saving a default
    // still requires the Owner role inside the handler.
    if path == "/api/agent-tools" || path.starts_with("/api/agent-tools/") {
        return if matches!(*method, Method::GET | Method::HEAD) {
            RouteAuth::Product
        } else {
            RouteAuth::ProductOperator
        };
    }
    if matches!(
        path,
        "/api/usage"
            | "/api/sync/dataset"
            | "/api/usage/logs"
            | "/api/workspaces"
            | "/api/execution-environments"
            | "/api/plugins"
            | "/api/providers"
            | "/api/machines"
            | "/api/session-maintenance"
            | "/api/observability/batches"
            | "/api/telemetry/v1/logs"
            | "/api/telemetry/v1/metrics"
            | "/api/telemetry/v1/traces"
    ) {
        return RouteAuth::Product;
    }
    // Manual and scheduled usage resets are driven from the product UI, so a
    // product operator must be able to reach them with the product cookie the
    // rest of that screen already uses. The legacy admin console keeps its own
    // path through the admin cookie.
    if path.starts_with("/api/usage/")
        && (path.ends_with("/reset")
            || path.ends_with("/reset/schedule")
            || (path.ends_with("/executor") && method == Method::PUT))
    {
        return RouteAuth::ProductOrAdminOperator;
    }
    if path.starts_with("/api/usage/") {
        return RouteAuth::Product;
    }
    if path.starts_with("/api/machines/") && path.ends_with("/maintenance") && method == Method::PUT
    {
        return RouteAuth::ProductOperator;
    }
    if path.starts_with("/api/machines/")
        && (path.ends_with("/events") || path.ends_with("/plugins"))
        && method == Method::GET
    {
        return RouteAuth::Product;
    }
    if path == "/api/machines/enrollment" {
        return RouteAuth::ProductOrAdminOperator;
    }
    if path.starts_with("/api/machines/") && path.ends_with("/revoke") {
        return RouteAuth::ProductOrAdminOperator;
    }
    if path.starts_with("/api/machines/")
        && (path.ends_with("/components/reconcile")
            || path.ends_with("/components/reconcile-one")
            || path.ends_with("/components/update-npm"))
    {
        return RouteAuth::AdminOperator;
    }
    if path.starts_with("/api/machines/") && path.contains("/providers/") {
        return RouteAuth::ProductOrAdminOperator;
    }
    if path.starts_with("/api/machines/") && path.contains("/plugins/") {
        return RouteAuth::ProductOrAdminOperator;
    }
    if path == "/api/plugins/catalog/refresh" {
        return RouteAuth::AdminOperator;
    }
    // Plugin RPC is a mutation surface: the Agent API catalog already requires
    // an Operator for the same command.
    if plugin_call_path(path) {
        return RouteAuth::ProductOperator;
    }
    if path == "/api/providers/catalog/refresh" {
        return RouteAuth::AdminOperator;
    }
    if provider_auth_path(path) {
        return RouteAuth::ProductOperator;
    }
    if matches!(
        path,
        "/api/metrics" | "/api/logs" | "/api/observability/incidents"
    ) || path.starts_with("/api/logs/")
    {
        return RouteAuth::ProductOrAdminOperator;
    }
    if path == "/api/auth/setup"
        || path == "/api/auth/oidc/start"
        || path == "/api/auth/oidc/callback"
        || path == "/api/auth/oidc/native/exchange"
        || path == "/api/auth/oidc/native/poll"
        || path == "/api/auth/oidc/native/events"
        || path == "/api/auth/oidc/native/cancel"
        || path == "/api/auth/oidc/native/complete"
        || authentication_provider_id(path, "start").is_some()
        || authentication_provider_id(path, "callback").is_some()
        || authentication_provider_id(path, "native/exchange").is_some()
        || authentication_provider_id(path, "native/poll").is_some()
        || authentication_provider_id(path, "native/events").is_some()
        || authentication_provider_id(path, "native/cancel").is_some()
        || authentication_provider_id(path, "backchannel-logout").is_some()
        || path == "/api/auth/logout"
        || path == "/api/auth/logout/complete"
        || path == "/api/admin/auth"
        || path == "/api/admin/auth/setup"
        || path == "/api/admin/auth/bootstrap"
        || path == "/api/admin/auth/login"
        || path == "/api/admin/auth/logout"
    {
        return RouteAuth::Public;
    }
    if path == "/api/admin/permissions" || path.starts_with("/api/admin/accounts") {
        return if matches!(*method, Method::GET | Method::HEAD) {
            RouteAuth::AdminViewer
        } else {
            RouteAuth::AdminOwner
        };
    }
    if path.starts_with("/api/admin/users/") && path.ends_with("/password") {
        return RouteAuth::AdminOwner;
    }
    if path.starts_with("/api/admin/") {
        return if matches!(*method, Method::GET | Method::HEAD) {
            RouteAuth::AdminViewer
        } else {
            RouteAuth::AdminOperator
        };
    }
    if path.starts_with("/api/") {
        return RouteAuth::AdminOperator;
    }
    RouteAuth::Public
}

fn public_route(path: &str) -> bool {
    matches!(path, "/healthz" | "/version")
        || path.starts_with("/plugin-artifacts/")
        || path.starts_with("/provider-artifacts/")
        || !path.starts_with("/api/") && path != "/ws" && path != "/metrics"
}

pub(super) fn plugin_call_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/api/plugins/") else {
        return false;
    };
    let mut segments = rest.split('/');
    let Some(plugin_id) = segments.next() else {
        return false;
    };
    crate::plugin_host::validate_plugin_id(plugin_id).is_ok()
        && segments.next() == Some("call")
        && segments.next().is_none()
}

pub(super) fn session_id_from_path(path: &str) -> Option<&str> {
    for prefix in ["/api/sessions/", "/api/code/sessions/", "/api/history/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            let id = rest.split('/').next().unwrap_or_default();
            if !id.is_empty() && id != "reconcile-project" {
                return Some(id);
            }
        }
    }
    None
}

pub(super) fn session_id_from_sync_state(state: &str) -> Option<&str> {
    state
        .strip_prefix("queue:")
        .or_else(|| state.strip_prefix("mobile-review:"))
        .filter(|id| !id.is_empty())
}

/// `/api/sessions/{id}/widgets/{widget}/interactions` and
/// `/api/sessions/{id}/widgets/{widget}/apps/{node}/messages`.
fn human_only_session_route(path: &str) -> bool {
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    matches!(
        parts.as_slice(),
        ["", "api", "sessions", _, "widgets", _, "interactions"]
            | [
                "",
                "api",
                "sessions",
                _,
                "widgets",
                _,
                "apps",
                _,
                "messages"
            ]
    )
}

pub(super) fn automation_route_allowed(
    identity: &crate::client_auth::DeviceAccessIdentity,
    method: &Method,
    path: &str,
) -> bool {
    if !identity.is_automation() {
        return true;
    }
    if path == "/ws" {
        return identity.has_scope("websocket");
    }
    if matches!(*method, Method::GET | Method::HEAD)
        && identity.has_scope("api:read")
        && (matches!(path, "/api/auth/status" | "/api/auth/me")
            || (!path.starts_with("/api/auth/") && !path.starts_with("/api/admin/")))
    {
        return true;
    }
    // A widget decision or an app's message speaks for the person who read it:
    // no automation credential may submit one, whatever its scopes.
    if human_only_session_route(path) {
        return false;
    }
    identity.has_scope("sessions:write")
        && ((method == Method::POST && matches!(path, "/api/sessions" | "/api/execution-sessions"))
            || session_id_from_path(path).is_some()
            || path == "/api/artifacts"
            || path.starts_with("/api/artifacts/"))
}

#[expect(
    clippy::result_large_err,
    reason = "HTTP rejection is an owned Axum response returned directly by its handler"
)]
pub(super) async fn ensure_product_session_fresh(
    store: &Store,
    session: &crate::store::ProductUserSession,
    authentication: &crate::auth_plugins::ProductAuthentication,
) -> Result<(), Response> {
    let policy = store
        .user_passkey_policy(&session.user_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, user_id = session.user_id, "passkey_policy_lookup");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        })?;
    let policy = policy.unwrap_or(crate::passkey::PasskeyPolicy {
        enabled: false,
        reauth_after_ms: crate::passkey::DEFAULT_PASSKEY_REAUTH_AFTER_MS,
        last_step_up_at_ms: None,
        passkey_count: 0,
    });
    let deadlines = product_session_deadlines(
        authentication.session,
        &policy,
        authentication.passkeys.enabled && authentication.passkeys.session_refresh_enabled,
        session,
        auth_now_ms(),
    );
    if let Some(kind) = deadlines.required_kind {
        let kind = match kind {
            ProductSessionReauthKind::Passkey => "passkey",
            ProductSessionReauthKind::Primary => "primary",
        };
        Err((
            StatusCode::PRECONDITION_REQUIRED,
            Json(serde_json::json!({
                "code": "session_reauthentication_required",
                "kind": kind,
            })),
        )
            .into_response())
    } else {
        Ok(())
    }
}

pub(super) fn visible_session_ids(
    hub: &Hub,
    principal: &ProductPrincipal,
) -> std::collections::HashSet<String> {
    hub.session_list_filtered(|owner| principal.can_see(owner))
        .into_iter()
        .map(|meta| meta.id)
        .collect()
}

pub(super) fn session_is_visible(
    hub: &Hub,
    principal: &ProductPrincipal,
    session_id: &str,
) -> bool {
    hub.session_info(session_id).is_some()
        && (principal.sees_every_session()
            || hub.session_owner_user_id(session_id).is_none()
            || hub.owned_by_product_user(session_id, &principal.user_id))
}

/// Ownership of the session a route names, on the identity its handler will
/// see. Axum percent-decodes `Path`, so authorizing the raw segment would let
/// `%73ess-5` miss here and reach handlers that trust this check. Only code
/// routes admit other IDs: they also address workspace contexts, which are not
/// Sessions and are authorized by `code_reads::Authority`.
pub(super) fn authorize_session_route(
    hub: &Hub,
    principal: &ProductPrincipal,
    method: &Method,
    path: &str,
) -> Result<(), StatusCode> {
    let Some(raw) = session_id_from_path(path) else {
        return Ok(());
    };
    let session_id = percent_encoding::percent_decode_str(raw)
        .decode_utf8()
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let Some(info) = hub.session_info(&session_id) else {
        return if path.starts_with("/api/code/sessions/") {
            Ok(())
        } else {
            Err(StatusCode::NOT_FOUND)
        };
    };
    let owner = info.meta.owner_user_id.as_deref();
    if !principal.can_see(owner) {
        return Err(StatusCode::NOT_FOUND);
    }
    if matches!(
        classify_route(method, path),
        RouteAuth::ProductSessionMutate
    ) && !principal.can_mutate(owner)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}

pub(super) async fn hash_product_password(password: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || crate::product_auth::hash_password(&password))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

pub(super) fn user_json(hub: &Hub, user: &crate::store::ProductUser) -> serde_json::Value {
    serde_json::json!({
        "id": user.id,
        "username": user.username,
        "created_at_ms": user.created_at_ms,
        "updated_at_ms": user.updated_at_ms,
        "disabled_at_ms": user.disabled_at_ms,
        "role": permission_policy(hub).role_for(&user.username),
    })
}

pub(super) async fn instance_needs_setup(state: &ProductAuthState) -> bool {
    match durable_store(&state.store) {
        Some(store) => store
            .list_users()
            .await
            .map(|users| users.is_empty())
            .unwrap_or(true),
        None => true,
    }
}

pub(super) fn authentication_provider_id<'a>(path: &'a str, suffix: &str) -> Option<&'a str> {
    let middle = path.strip_prefix("/api/auth/providers/")?;
    let provider_id = middle.strip_suffix(&format!("/{suffix}"))?;
    (!provider_id.is_empty()
        && !provider_id.contains('/')
        && provider_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
    .then_some(provider_id)
}

#[expect(
    clippy::result_large_err,
    reason = "HTTP rejection is an owned Axum response returned directly by its handler"
)]
pub(super) async fn require_product_user(
    state: &ProductAuthState,
    headers: &HeaderMap,
) -> Result<crate::store::ProductUser, Response> {
    product_user_from_cookie(state, headers)
        .await
        .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())
}

#[expect(
    clippy::result_large_err,
    reason = "HTTP rejection is an owned Axum response returned directly by its handler"
)]
pub(super) async fn require_fresh_product_user(
    state: &ProductAuthState,
    headers: &HeaderMap,
) -> Result<crate::store::ProductUser, Response> {
    let Some(store) = state.store.as_ref() else {
        return Err(missing_store());
    };
    let Some((session, user)) = product_session_and_user_from_store_cookie(store, headers).await
    else {
        return Err(StatusCode::UNAUTHORIZED.into_response());
    };
    ensure_product_session_fresh(store, &session, &state.product_authentication).await?;
    Ok(user)
}

#[expect(
    clippy::result_large_err,
    reason = "HTTP rejection is an owned Axum response returned directly by its handler"
)]
pub(super) async fn require_recent_product_user(
    state: &ProductAuthState,
    headers: &HeaderMap,
) -> Result<crate::store::ProductUser, Response> {
    let Some(store) = state.store.as_ref() else {
        return Err(missing_store());
    };
    let Some((session, user)) = product_session_and_user_from_store_cookie(store, headers).await
    else {
        return Err(StatusCode::UNAUTHORIZED.into_response());
    };
    ensure_product_session_fresh(store, &session, &state.product_authentication).await?;
    if !product_session_has_recent_step_up(&session, auth_now_ms()) {
        return Err((
            StatusCode::PRECONDITION_REQUIRED,
            "Recent login or Passkey verification required",
        )
            .into_response());
    }
    Ok(user)
}

pub(super) async fn restore_external_passkey_ceremony(
    state: &ProductAuthState,
    transaction_id: &str,
) -> anyhow::Result<()> {
    if state.passkeys.external_record(transaction_id).is_ok() {
        return Ok(());
    }
    let transaction_hash = crate::passkey::external_transaction_hash(transaction_id)?;
    let store = state
        .store
        .as_ref()
        .context("product store is unavailable")?;
    let record = store
        .external_passkey_ceremony(&transaction_hash, auth_now_ms())
        .await?
        .context("external passkey ceremony expired")?;
    state.passkeys.restore_external_record(&record)
}

pub(super) async fn persist_external_passkey_ceremony(
    state: &ProductAuthState,
    transaction_id: &str,
) -> anyhow::Result<()> {
    let store = state
        .store
        .as_ref()
        .context("product store is unavailable")?;
    let record = state.passkeys.external_record(transaction_id)?;
    store.upsert_external_passkey_ceremony(&record).await
}

pub(super) fn external_passkey_error_response(error: &anyhow::Error) -> Response {
    let detail = error.to_string();
    let (status, code, message) = if detail.contains("expired") {
        (
            StatusCode::GONE,
            "passkey_transaction_expired",
            "Passkey verification expired. Try again.",
        )
    } else if detail.contains("session mismatch") {
        (
            StatusCode::UNAUTHORIZED,
            "passkey_session_changed",
            "This Cowboy session changed. Start Passkey verification again.",
        )
    } else if detail.contains("PKCE mismatch") {
        (
            StatusCode::BAD_REQUEST,
            "passkey_transaction_mismatch",
            "Passkey verification could not be matched to this request.",
        )
    } else if detail.contains("unavailable") || detail.contains("type mismatch") {
        (
            StatusCode::CONFLICT,
            "passkey_transaction_unavailable",
            "Passkey verification is no longer active. Try again.",
        )
    } else {
        (
            StatusCode::BAD_REQUEST,
            "passkey_transaction_invalid",
            "Passkey verification did not complete. Try again.",
        )
    };
    tracing::info!(%error, code, "external_passkey_transaction_rejected");
    (
        status,
        Json(serde_json::json!({
            "code": code,
            "message": message,
        })),
    )
        .into_response()
}

#[cfg(test)]
pub(super) async fn persist_fixture_device(store: &Store, user: &str, device: &str, public: &str) {
    let now = auth_now_ms();
    store
        .insert_user_device(
            &crate::store::ProductDevice {
                id: device.into(),
                user_id: user.into(),
                name: "fixture".into(),
                public_key: public.into(),
                created_at_ms: now,
                last_used_at_ms: Some(now),
                revoked_at_ms: None,
            },
            &crate::store::ProductDeviceRefreshToken {
                token_hash: format!("fixture-{device}"),
                device_id: device.into(),
                family_id: device.into(),
                created_at_ms: now,
                expires_at_ms: now + 60000,
                used_at_ms: None,
                revoked_at_ms: Some(now),
            },
        )
        .await
        .unwrap();
}

pub(super) fn refresh_bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?
        .trim();
    token
        .starts_with(crate::client_auth::REFRESH_TOKEN_PREFIX)
        .then(|| token.to_owned())
}

pub(super) async fn revoke_replayed_device(
    state: &ProductAuthState,
    store: &Store,
    device_id: &str,
    now: i64,
) {
    if let Err(error) = store.revoke_user_device(device_id, now).await {
        tracing::error!(%error, device_id, "device_replay_revoke_failed");
    }
    state.device_access.revoke_device(device_id);
}

pub(super) fn session_public_json(
    session: crate::store::ProductUserSession,
    current_session_id: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "id": session.session_id,
        "current": current_session_id.is_some_and(|id| id == session.session_id),
        "client_kind": session.client_kind,
        "principal_class": session.principal_class,
        "created_at_ms": session.created_at_ms,
        "expires_at_ms": session.expires_at_ms,
        "last_seen_at_ms": session.last_seen_at_ms,
        "user_agent": session.user_agent,
        "primary_auth_method": session.primary_auth_method,
        "provider_id": session.auth_provider_id,
    })
}

pub(super) fn token_public_json(token: &crate::store::ProductApiToken) -> serde_json::Value {
    serde_json::json!({
        "id": token.id,
        "name": token.name,
        "token_prefix": token.token_prefix,
        "created_at_ms": token.created_at_ms,
        "expires_at_ms": token.expires_at_ms,
        "last_used_at_ms": token.last_used_at_ms,
    })
}

/// A trashed session is read-only until restored: it takes no work, no
/// configuration and never wakes. Deletion, cancel, open and rename stay
/// allowed, as does releasing a queue-edit hold: the Composer releases its hold
/// when it unmounts right after the user trashes the session, and refusing it
/// would raise an error toast and incident for the user's own action.
pub(super) fn allowed_on_trashed_session(cmd: &Inbound) -> bool {
    matches!(
        cmd,
        Inbound::DeleteSession { .. }
            | Inbound::Cancel { .. }
            | Inbound::CancelSubagent { .. }
            | Inbound::CancelSubmitted { .. }
            | Inbound::OpenSession { .. }
            | Inbound::RenameSession { .. }
            | Inbound::SetQueueEditing { id: None, .. }
    )
}

pub(super) fn is_admin_console_path(requested: &str) -> bool {
    requested == "admin" || requested.starts_with("admin/")
}
