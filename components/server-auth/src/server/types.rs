//! Core authentication, policy, and shared lifecycle state types.

use super::*;
use std::io::Read as _;

#[derive(Clone, Copy)]
pub struct ProductApiEnforcement<'a> {
    pub product_auth_enabled: bool,
    pub store: Option<&'a Store>,
    pub hub: &'a Hub,
    pub device_access: &'a crate::client_auth::DeviceAccessSessions,
    pub product_authentication: &'a crate::auth_plugins::ProductAuthentication,
    pub public_origins: &'a [String],
}

impl<'a> ProductApiEnforcement<'a> {
    pub fn auth(&self) -> ProductRequestAuth<'a> {
        ProductRequestAuth {
            product_auth_enabled: self.product_auth_enabled,
            store: self.store,
            hub: self.hub,
            device_access: self.device_access,
            product_authentication: self.product_authentication,
        }
    }
}

pub fn machine_refresh_path(path: &str) -> bool {
    path.strip_prefix("/api/machines/")
        .and_then(|rest| rest.strip_suffix("/refresh"))
        .is_some_and(|machine| !machine.is_empty() && !machine.contains('/'))
}

/// Same rule as the metrics scrape: a direct same-host caller, never a request
/// that crossed the public reverse proxy.
pub fn internal_machine_refresh(
    method: &Method,
    path: &str,
    peer: SocketAddr,
    headers: &HeaderMap,
) -> bool {
    method == Method::POST
        && machine_refresh_path(path)
        && crate::product_auth::metrics_scrape_allowed(peer, headers)
}

pub fn random_machine_token() -> anyhow::Result<String> {
    let mut random = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .context("opening OS randomness")?
        .read_exact(&mut random)
        .context("reading OS randomness")?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginFenceState {
    Installing,
    Uninstalling,
    Uninstalled,
    NeedsReconcile,
}

pub type PluginLifecycleFences =
    Arc<parking_lot::RwLock<HashMap<(String, String), PluginFenceState>>>;

/// Reserve the lifecycle before reading the active release, including across
/// awaits. Reuse the non-destructive installing fence; cancellation releases it.

#[derive(Default)]
pub struct ActiveCapacityRuntime {
    pub channels: parking_lot::Mutex<HashMap<String, LocalActiveClient>>,
    pub changed: tokio::sync::Notify,
}

#[derive(Debug, Default)]
pub struct LocalActiveClient {
    pub channels: u32,
    pub fencing_token: Option<i64>,
}

impl ActiveCapacityRuntime {
    pub fn open_channel(&self, client_id: &str, limit: u32, enforce: bool) -> Result<u32, u32> {
        let mut clients = self.channels.lock();
        let client = clients.entry(client_id.to_owned()).or_default();
        let next = client.channels.saturating_add(1);
        if enforce && next > limit {
            return Err(client.channels);
        }
        client.channels = next;
        Ok(next)
    }

    pub fn record_fence(&self, client_id: &str, fencing_token: i64) {
        if let Some(client) = self.channels.lock().get_mut(client_id) {
            client.fencing_token = Some(fencing_token);
        }
    }

    pub fn close_channel(&self, client_id: &str) -> Option<i64> {
        let mut clients = self.channels.lock();
        let client = clients.get_mut(client_id)?;
        client.channels = client.channels.saturating_sub(1);
        if client.channels > 0 {
            return None;
        }
        clients
            .remove(client_id)
            .and_then(|client| client.fencing_token)
    }

    pub fn notify_changed(&self) {
        self.changed.notify_waiters();
    }
}

pub struct ActiveCapacityChannel {
    pub runtime: Arc<ActiveCapacityRuntime>,
    pub store: Option<Store>,
    pub client_id: String,
    pub reservation_ms: i64,
    pub per_user_limit: u32,
}

impl Drop for ActiveCapacityChannel {
    fn drop(&mut self) {
        let Some(fencing_token) = self.runtime.close_channel(&self.client_id) else {
            return;
        };
        let Some(store) = self.store.clone() else {
            return;
        };
        let runtime = self.runtime.clone();
        let client_id = self.client_id.clone();
        let reservation_ms = self.reservation_ms;
        let per_user_limit = self.per_user_limit;
        tokio::spawn(async move {
            if let Err(error) = store
                .release_active_client(&ActiveClientRelease {
                    client_id: &client_id,
                    fencing_token,
                    reason: "channel_closed",
                    now_ms: auth_now_ms(),
                    reservation_ms,
                    per_user_limit,
                })
                .await
            {
                tracing::warn!(%error, %client_id, "releasing active-client lease");
            }
            runtime.notify_changed();
        });
    }
}

#[derive(Clone)]
pub struct ProductAuthState {
    pub hub: Hub,
    pub store: Option<Store>,
    pub rate_limits: Arc<crate::product_auth::AuthRateLimiter>,
    pub public_origins: Arc<Vec<String>>,
    pub runtime_health: Option<Arc<RuntimeHealth>>,
    pub persistence_health: Option<Arc<PersistenceHealth>>,
    pub runtime_router: Option<Arc<RuntimeRouter>>,
    pub plugin_catalog: Option<Arc<crate::plugin_catalog::PluginCatalog>>,
    pub plugin_storage: Option<crate::plugin_storage::PluginStorage>,
    pub provider_catalog: Option<Arc<crate::provider_catalog::ProviderCatalog>>,
    pub passkeys: Arc<crate::passkey::PasskeyCeremonies>,
    pub setup: Arc<crate::admin::AdminSetupState>,
    pub setup_lock: Arc<tokio::sync::Mutex<()>>,
    pub product_auth_enabled: bool,
    pub product_authentication: Arc<crate::auth_plugins::ProductAuthentication>,
    pub oidc_transactions: Arc<crate::oidc::OidcTransactions>,
    pub oidc_native_handoffs: Arc<crate::oidc::NativeHandoffs>,
    pub device_authorizations: Arc<crate::client_auth::DeviceAuthorizations>,
    pub device_access: Arc<crate::client_auth::DeviceAccessSessions>,
    pub capacity_runtime: Arc<ActiveCapacityRuntime>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub account: String,
    pub password: String,
    #[allow(dead_code)]
    pub token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub account: String,
    pub password: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogoutScope {
    #[default]
    Current,
    Provider,
    All,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogoutRequest {
    #[serde(default)]
    pub scope: LogoutScope,
    #[serde(default)]
    pub provider_logout: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcBackchannelLogoutRequest {
    pub logout_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationCredentialRequest {
    pub name: String,
    pub public_key: String,
    pub ttl_seconds: Option<i64>,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveClientReleaseQuery {
    pub fencing_token: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcStartQuery {
    pub client: Option<String>,
    pub code_challenge: Option<String>,
    pub handoff_challenge: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcNativeExchangeRequest {
    pub code: String,
    pub code_verifier: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcNativePollRequest {
    pub handoff_token: String,
    pub code_verifier: String,
}

#[derive(Debug, Serialize)]
pub struct OidcNativePollPending {
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct OidcNativeEventStatus {
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct PasskeyExternalEventStatus {
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ProductMe {
    pub account: String,
    pub user_id: Option<String>,
    pub role: crate::admin::AdminRole,
    #[serde(default)]
    pub primary_auth_method: Option<String>,
    #[serde(default)]
    pub passkey_count: u32,
    #[serde(default)]
    pub passkey_reauth_enabled: bool,
    #[serde(default)]
    pub passkey_reauth_required: bool,
    #[serde(default)]
    pub passkey_reauth_after_ms: i64,
    #[serde(default)]
    pub passkey_reauth_due_at_ms: Option<i64>,
    #[serde(default)]
    pub passkey_reauth_warn_at_ms: Option<i64>,
    #[serde(default)]
    pub primary_reauth_due_at_ms: Option<i64>,
    #[serde(default)]
    pub primary_reauth_warn_at_ms: Option<i64>,
    #[serde(default)]
    pub session_idle_due_at_ms: Option<i64>,
    #[serde(default)]
    pub session_expires_at_ms: Option<i64>,
    #[serde(default)]
    pub session_server_now_ms: Option<i64>,
    #[serde(default)]
    pub session_reauth_kind: Option<ProductSessionReauthKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductSessionReauthKind {
    Passkey,
    Primary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductSessionDeadlines {
    pub idle_due_at_ms: Option<i64>,
    pub passkey_due_at_ms: Option<i64>,
    pub passkey_warn_at_ms: Option<i64>,
    pub primary_due_at_ms: i64,
    pub primary_warn_at_ms: i64,
    pub required_kind: Option<ProductSessionReauthKind>,
}

#[derive(Debug, Deserialize)]
pub struct CreateApiTokenRequest {
    pub name: String,
    pub ttl_seconds: Option<i64>,
}

pub fn durable_store(store: &Option<Store>) -> Option<&Store> {
    store.as_ref()
}

pub fn missing_store() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "product accounts require a durable store",
    )
        .into_response()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteAuth {
    Public,
    Product,
    ProductOperator,
    ProductSessionSee,
    ProductSessionMutate,
    ProductOrAdminOperator,
    AdminViewer,
    AdminOperator,
    AdminOwner,
    MetricsScrape,
}

/// The same authentication context serves API middleware and the standalone
/// authentication router. A verified request extension is reused by handlers:
/// sender-constrained proof nonces must never be consumed twice.
#[derive(Clone, Copy)] // Borrowed core services, not an approval or credential.
pub struct ProductRequestAuth<'a> {
    pub product_auth_enabled: bool,
    pub store: Option<&'a Store>,
    pub hub: &'a Hub,
    pub device_access: &'a crate::client_auth::DeviceAccessSessions,
    pub product_authentication: &'a crate::auth_plugins::ProductAuthentication,
}

impl<'a> From<&'a ProductAuthState> for ProductRequestAuth<'a> {
    fn from(state: &'a ProductAuthState) -> Self {
        Self {
            product_auth_enabled: state.product_auth_enabled,
            store: state.store.as_ref(),
            hub: &state.hub,
            device_access: &state.device_access,
            product_authentication: &state.product_authentication,
        }
    }
}

#[derive(Clone)]
pub struct AuthenticatedProductRequest {
    pub principal: ProductPrincipal,
    pub cookie_session: Option<crate::store::ProductUserSession>,
    pub device_identity: Option<crate::client_auth::DeviceAccessIdentity>,
    pub permissions: Option<crate::core::ProductPermissionObservation>,
}

impl AuthenticatedProductRequest {
    /// Complete authentication by atomically observing current permissions.
    /// Handlers may share this observation, but must never recapture it.
    pub fn observe_permissions(mut self, hub: &Hub) -> Result<Self, ()> {
        let permissions = hub
            .observe_product_permissions(&self.principal.username)
            .ok_or(())?;
        self.principal.role = permissions.role();
        self.permissions = Some(permissions);
        Ok(self)
    }
}

pub const OIDC_NATIVE_EVENTS_HANDSHAKE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(10);
pub const OIDC_NATIVE_EVENTS_TTL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
pub const OIDC_NATIVE_EVENTS_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(25);

/// Close the window Cowboy opened for an external sign-in, from inside it.
///
/// The opener cannot be relied on to do this. A Provider that serves
/// `Cross-Origin-Opener-Policy: same-origin` — Cardea does — moves this window
/// into a new browsing context group the moment the authorization request
/// reaches it. That severs Cowboy's handle for the rest of the flow: `closed`
/// reports `true` even while the window is on screen, and `close()` and any
/// navigation through that handle are dropped without an error. Only this
/// document can still end the window, so it closes itself and offers a link
/// for an engine that refuses. The link stays hidden until the close has had
/// its chance, both to avoid a flash in the ordinary case and because the iOS
/// shell dismisses its own authentication sheet: loading Cowboy inside that
/// sheet is a choice for the reader to make, never an automatic redirect.
pub const NATIVE_COMPLETE_SCRIPT: &str = r#"
try { window.close(); } catch (error) { /* Recovered by the link below. */ }
setTimeout(function () { document.body.className = "stranded"; }, 700);
"#;

pub static NATIVE_COMPLETE_PAGE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Return to Cowboy</title>
  <style>
    :root {{ color-scheme: light dark; font-family: -apple-system, BlinkMacSystemFont, sans-serif; }}
    body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; background: Canvas; color: CanvasText; }}
    main {{ max-width: 30rem; padding: 2rem; text-align: center; }}
    h1 {{ font-size: 1.5rem; }}
    p {{ line-height: 1.5; opacity: .72; }}
    a {{ display: none; }}
    body.stranded a {{ display: inline-block; }}
  </style>
</head>
<body><main><h1>Return to Cowboy</h1><p>Cowboy will show the authorization result and close this window, or you can close it now.</p><p><a href="/">Open Cowboy in this window</a></p></main><script>{NATIVE_COMPLETE_SCRIPT}</script></body>
</html>"#
    )
});

pub static NATIVE_COMPLETE_CSP: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let digest = base64::engine::general_purpose::STANDARD
        .encode(Sha256::digest(NATIVE_COMPLETE_SCRIPT.as_bytes()));
    format!(
        "default-src 'none'; style-src 'unsafe-inline'; script-src 'sha256-{digest}'; \
         base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    )
});

pub const PASSKEY_MANAGEMENT_STEP_UP_MAX_AGE_MS: i64 = 5 * 60 * 1_000;

pub struct ProductPasskeyAssertionResult {
    pub me: ProductMe,
    pub set_cookie: Option<String>,
}

pub const PASSKEY_EXTERNAL_EVENTS_HANDSHAKE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(10);
pub const PASSKEY_EXTERNAL_EVENTS_TTL: std::time::Duration =
    std::time::Duration::from_secs(crate::passkey::EXTERNAL_CEREMONY_TTL_SECS);
pub const PASSKEY_EXTERNAL_EVENTS_HEARTBEAT: std::time::Duration =
    std::time::Duration::from_secs(25);

pub const DEVICE_AUTH_EVENTS_HANDSHAKE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(10);
pub const DEVICE_AUTH_EVENTS_TTL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
pub const DEVICE_AUTH_EVENTS_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(25);

pub fn no_store_json<T: Serialize>(status: StatusCode, value: T) -> Response {
    let mut response = (status, Json(value)).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardeaDeviceExchangeRequest {
    pub name: String,
    pub public_key: String,
    pub access_token: String,
    pub assertion: String,
}

pub struct ProductAuthSessionSnapshot {
    pub message: Outbound,
    pub valid: bool,
    pub user_id: String,
    pub next_due_at_ms: i64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct AdminCreateUserRequest {
    pub account: String,
    pub password: String,
    pub role: Option<crate::admin::AdminRole>,
}

#[derive(Debug, Deserialize)]
pub struct AdminSetPasswordRequest {
    pub password: String,
}

#[derive(Serialize)]
pub struct AdminOverview {
    pub healthy: bool,
    pub persistence: &'static str,
    pub backend: &'static str,
    pub sessions_live: usize,
    pub sessions_deleted: i64,
    pub events_rows: i64,
    pub daemon_rss_bytes: u64,
    pub runtime_workers: usize,
    pub runtime_busy_workers: usize,
    pub registration: crate::admin::RegistrationPolicyView,
}

#[derive(Serialize)]
pub struct AdminSessionRow {
    pub id: String,
    pub title: String,
    pub provider: String,
    pub machine_id: String,
    pub status: crate::agent_model::Status,
}

pub const WEBSOCKET_FRAME_SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub async fn send_json<S, T>(sink: &mut S, msg: &T) -> Result<(), ()>
where
    S: futures::SinkExt<Message> + Unpin,
    T: Serialize,
{
    send_json_with_timeout(sink, msg, WEBSOCKET_FRAME_SEND_TIMEOUT).await
}

pub async fn send_json_with_timeout<S, T>(
    sink: &mut S,
    msg: &T,
    timeout: std::time::Duration,
) -> Result<(), ()>
where
    S: futures::SinkExt<Message> + Unpin,
    T: Serialize,
{
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    tokio::time::timeout(timeout, sink.send(Message::Text(text.into())))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

pub fn authorize_ws_upgrade(
    headers: &HeaderMap,
    peer: SocketAddr,
    public_origins: &[String],
    principal: Option<ProductPrincipal>,
) -> Result<ProductPrincipal, StatusCode> {
    let via_cookie = crate::product_auth::user_cookie_token(headers).is_some()
        && crate::product_auth::bearer_token(headers).is_none();
    if via_cookie && !crate::product_auth::origin_allowed(headers, peer, public_origins) {
        return Err(StatusCode::FORBIDDEN);
    }
    match principal {
        Some(principal) => Ok(principal),
        None => {
            tracing::info!(reason = "auth_required", "ws_rejected");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DaemonMemory {
    pub rss_bytes: u64,
    pub rss_peak_bytes: u64,
}

/// Current and lifetime-high resident set of THIS process (not its agent or
/// usage-collector children). `/proc/self/status` reports KiB directly, avoiding
/// the incorrect 4 KiB page-size assumption `statm` would make on some hosts.
pub fn daemon_memory() -> DaemonMemory {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .map_or_else(DaemonMemory::default, |status| {
            parse_proc_status_memory(&status)
        })
}

pub fn parse_proc_status_memory(status: &str) -> DaemonMemory {
    fn kib_value(line: &str, key: &str) -> Option<u64> {
        line.strip_prefix(key)?
            .split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
            .map(|kib| kib.saturating_mul(1024))
    }

    let mut memory = DaemonMemory::default();
    for line in status.lines() {
        if let Some(bytes) = kib_value(line, "VmRSS:") {
            memory.rss_bytes = bytes;
        } else if let Some(bytes) = kib_value(line, "VmHWM:") {
            memory.rss_peak_bytes = bytes;
        }
    }
    memory
}

pub fn provider_auth_path(path: &str) -> bool {
    ["/api/plugins/", "/api/providers/"].iter().any(|prefix| {
        let Some(rest) = path.strip_prefix(prefix) else {
            return false;
        };
        let mut segments = rest.split('/');
        if !segments.next().is_some_and(|segment| !segment.is_empty()) {
            return false;
        }
        if segments.next() != Some("auth") {
            return false;
        }
        match (segments.next(), segments.next()) {
            (None, None) => true,
            (Some(segment), None) => !segment.is_empty(),
            _ => false,
        }
    })
}
