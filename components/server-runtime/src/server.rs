//! HTTP / WebSocket server (design §5).
//!
//! Every frontend is an **equal subscriber** to one WebSocket stream. On
//! browsers negotiate a lightweight global session index, then hydrate the
//! focused session over HTTP while the socket carries live events. Legacy and
//! ACP bridge clients retain the complete bootstrap unless they opt into lazy
//! mode, preserving wire compatibility.
//!
//! Product `/ws` and `/api/*` require a product cookie or Bearer token.
//! Machine connections use one-time enrollment plus an OpenSSH Ed25519
//! challenge before WebSocket protocol negotiation.

pub mod agent_tools;
pub mod cache_metrics_api;
pub mod config_revert;
pub mod model_catalog;
pub use agent_tools::launch_extensions;
pub use mcp_bindings::launch_mcp;
pub mod dormant_pins;
pub mod folder_defaults;
pub mod mcp_bindings;
pub mod state;
pub use state::AppState;
pub mod memory_reclaim;
pub mod memory_relay;
pub use cowboy_server_auth::server::secure_transport;
pub mod send_delivery;
pub mod session_configuration;
pub mod session_context;
pub mod session_create_replay;
pub mod session_maintenance;
pub mod session_provider_updates;
pub mod session_reclaim;
pub mod session_trash;
pub mod web_roots;
pub mod workspace_preparation;

pub use std::collections::{BTreeMap, BTreeSet, HashMap};
pub use std::net::SocketAddr;
pub use std::path::{Component, Path as FsPath, PathBuf};
pub use std::sync::Arc;
pub use std::sync::atomic::{AtomicU64, Ordering};
pub use tokio::sync::{mpsc, watch};

pub use anyhow::Context as _;
pub use axum::Router;
pub use axum::body::Body;
pub use axum::extract::DefaultBodyLimit;
pub use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
pub use axum::extract::{ConnectInfo, Extension, Json, Path, Query, State};
#[cfg(test)]
pub use axum::http::Method;
pub use axum::http::{HeaderMap, StatusCode, Uri, header};
pub use axum::middleware::{self, Next};
pub use axum::response::{IntoResponse, Response};
pub use axum::routing::{any, delete, get, post, put};
pub use base64::Engine as _;
pub use futures::{SinkExt, StreamExt};
pub use serde::{Deserialize, Serialize};
pub use sha2::{Digest as _, Sha256};

pub use agent_client_protocol::schema::v1::ContentBlock;

pub use crate::acp::AgentCommand;
pub use crate::core::{
    CodeReadScope, CommandOutcome, DispatchReq, Envelope, Event, FanoutFrame, Hub, Inbound,
    Outbound, PersistenceHealth, RestoredSession, SessionOrigin, Status, StoreReceiver, StoreSink,
    StoreWrite, project_sync_value,
};
pub use crate::diff_snapshot::DiffSnapshotCache;
pub use crate::machine_control::MachineControl;
pub use crate::machine_protocol::CONTROLLER_RECONNECT_MAX_BACKOFF_SECONDS;
pub use crate::observability::{Observability, SubmitReceipt, TelemetryBatch};
pub use crate::persistence::EventReducer;
pub use crate::product_auth::ProductPrincipal;
pub use crate::remote_runtime::{RemoteBootstrap, RemoteRuntime};
pub use crate::runtime::RuntimeHealth;
pub use crate::runtime_router::RuntimeRouter;
pub use crate::serve_args::ServeArgs;
pub use crate::session_folders::{FolderActor, project_folders_value};
pub use crate::store::Store;
pub use crate::supervisor::Supervisor;
pub use crate::usage::UsageService;
pub use crate::web_push::WebPushService;
pub use tokio_util::io::ReaderStream;

pub mod agent_api_bridge;
pub mod background_tasks;
#[cfg(test)]
mod background_tasks_tests;
#[cfg(test)]
mod c_auth_tests;
pub mod cardea_authorization;
pub use cowboy_server_code_buffers::server::code_buffers;
pub mod draft_documents;
pub mod execution;
#[cfg(unix)]
pub mod local_operator;
pub mod machine_lanes;
pub mod machine_post_edit;
pub mod managed_calls;
pub mod native_configuration;
pub mod notifications;
pub use cowboy_server_auth::server::operator_approval;
pub mod peer_notices;
#[cfg(test)]
mod persistence_tests;
pub use cowboy_server_plugins::server::plugin_install;
pub mod preferences;
pub use cowboy_server_accounts::server::{
    AccountsState, NamedReleaseMove, ProviderAuthExecutor, ProviderAuthStartRequest,
    ProviderDistributionOutcome, ResetRequest, ResolvedProviderGeneration,
    SchedulingAuthentication, catalogued_installations, distribute_and_mark_scope,
    provider_accounts, provider_auth_scope_for_machine, provider_reload_contract_compatible,
    reconcile_provider_auth_executors, resolve_provider_generation,
    resolve_provider_generation_with_auth, resumed_provider_authentication_response,
    sync_provider_account_auth_to_machine,
};
#[cfg(test)]
pub use cowboy_server_accounts::server::{
    provider_inventory_contract_matches, resolve_scheduling_auth_generation,
};
pub mod superagent;
pub use plugin_install::api_machine_plugin_install;
pub mod plugin_history;
pub use cowboy_server_plugins::server::plugin_uninstall;
#[cfg(test)]
pub use cowboy_server_plugins::server::provider_auth_sync_required_before_install;
pub use cowboy_server_plugins::server::{
    PluginInstallRequest, PluginUninstallRequest, PluginsState, agent_plugin_install_compatibility,
    plugin_install_compatibility, preview_machine_plugin_uninstall, service_managed_refusal,
};
pub mod projects;
pub use cowboy_server_auth::server::provider_auth_sync;
pub mod sync_dataset;
pub use cowboy_server_auth::server::web_security;
pub use cowboy_server_telemetry::server::telemetry_binding;
pub use plugin_uninstall::{
    api_machine_plugin_operation_receipt, api_machine_plugin_operations,
    api_machine_plugin_recovery_assessment, api_machine_plugin_uninstall,
};

pub struct ProviderReloadFence {
    pub fences: PluginLifecycleFences,
    pub key: (String, String),
}

impl ProviderReloadFence {
    pub fn acquire(fences: &PluginLifecycleFences, key: (String, String)) -> Result<Self, String> {
        let mut active = fences.write();
        if active.contains_key(&key) {
            return Err("Provider is changing on the selected Machine".to_owned());
        }
        active.insert(key.clone(), PluginFenceState::Installing);
        Ok(Self {
            fences: Arc::clone(fences),
            key,
        })
    }
}

impl Drop for ProviderReloadFence {
    fn drop(&mut self) {
        self.fences.write().remove(&self.key);
    }
}

pub const MACHINE_RUNTIME_STARTUP_GRACE: std::time::Duration = std::time::Duration::from_secs(45);
// Legacy Machine hosts begin their heartbeat only after welcome-time component
// reconciliation. Keep that explicitly identified work out of the
// "unresponsive" state while still bounding a genuinely stuck update.
pub const MACHINE_COMPONENT_RECONCILIATION_GRACE: std::time::Duration =
    std::time::Duration::from_secs(330);

#[cfg(test)]
mod component_supersession_tests;

#[cfg(test)]
mod machine_health_tests;

pub const STORE_QUEUE_CAPACITY: usize = 8_192;
pub const FORCE_CANCEL_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
/// An execution-bound native interrupt first cancels its target processes over
/// the Machine execution link: several round trips, each a second or more on a
/// slow link, and the link can stall for 30-60 s. That is progress, not a hung
/// turn. Recycling instead forces a cold remote resume that crosses the same
/// link and costs far more than the cancellation it replaces.
pub const EXECUTION_FORCE_CANCEL_GRACE: std::time::Duration = std::time::Duration::from_secs(75);
pub const QUEUE_EDIT_RECONNECT_GRACE: std::time::Duration = std::time::Duration::from_secs(15);
/// How long a dropped Machine stays `reconnecting` before the sweeper marks it
/// offline and clients surface "not connected". A remote Machine reaches the
/// Controller over pooled cross-border streams: after a reset it backs off up
/// to `CONTROLLER_RECONNECT_MAX_BACKOFF_SECONDS`, then needs a fresh stream
/// (one dial of up to 10 s, or an overlay fallback of the same budget) and the
/// TLS/auth handshake over a lossy path. 15 s expired on most of OVH's
/// reconnects (11 expiries for 14 resets in one day) although the Machine was
/// back within a minute; cover one full backoff plus one stream dial.
pub const MACHINE_RECONNECT_GRACE_SECONDS: i32 = 60;
/// A connected Machine still needs time to replay its authoritative worker
/// snapshots. Cover the Machine's entire outbound reconnect backoff plus a
/// bounded settlement window before declaring persisted Busy turns dead.
pub const RUNTIME_SNAPSHOT_SETTLEMENT_SECONDS: u64 = 15;
pub const RUNTIME_RECONCILIATION_GRACE: std::time::Duration = std::time::Duration::from_secs(
    CONTROLLER_RECONNECT_MAX_BACKOFF_SECONDS + RUNTIME_SNAPSHOT_SETTLEMENT_SECONDS,
);
pub const MACHINE_RECONNECT_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Let restore, listener binding, and Machine reconnection settle before the
/// optional account collectors start their short-lived provider processes.

#[derive(Debug, PartialEq, Eq)]
pub enum ScheduledResetFailurePolicy {
    RetryPreflight,
    StopFailed,
    StopUnknown,
}

pub fn scheduled_reset_failure_policy(
    call_may_have_reached_provider: bool,
    prior_attempts: i32,
) -> ScheduledResetFailurePolicy {
    if call_may_have_reached_provider {
        ScheduledResetFailurePolicy::StopUnknown
    } else if prior_attempts < 2 {
        ScheduledResetFailurePolicy::RetryPreflight
    } else {
        ScheduledResetFailurePolicy::StopFailed
    }
}

/// How often the Controller compares every connected Machine with the desired
/// component set and re-reads the manifest behind it.
pub const MACHINE_CONVERGENCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// A freshly connected Machine is already reconciling the automatic set its
/// welcome carried. Convergence waits this long before adding its own dispatch,
/// so the same generation is never activated twice in a row.
pub const MACHINE_CONVERGENCE_CONNECTION_GRACE: std::time::Duration =
    std::time::Duration::from_secs(120);

/// Resolve a subagent or Call anchor this writer lifetime does not know yet
/// through the durable table (scanning a session's pre-table rows once), so a
/// restarted or evicting writer replaces the original row instead of starting
/// a second one. `Ok(None)` means the key is new.
pub async fn resolve_durable_anchor(
    store: &Store,
    session_id: &str,
    key: &str,
) -> anyhow::Result<Option<u64>> {
    if let Some(seq) = store.transcript_anchor(session_id, key).await? {
        return Ok(Some(seq));
    }
    if key.starts_with(crate::transcript_anchors::SUBAGENT_PREFIX)
        && store.backfill_subagent_anchors(session_id).await?.is_some()
    {
        return store.transcript_anchor(session_id, key).await;
    }
    Ok(None)
}

pub async fn flush_event_batch(
    store: &Store,
    reducer: &mut EventReducer,
    events: &mut BTreeMap<(String, u64), Envelope>,
    highwaters: &mut HashMap<String, u64>,
    anchors: &mut Vec<crate::transcript_anchors::AnchorWrite>,
    occurrences: &mut Vec<crate::task_notifications::Occurrence>,
) -> bool {
    if events.is_empty() && highwaters.is_empty() && anchors.is_empty() && occurrences.is_empty() {
        return true;
    }
    let rows: Vec<Envelope> = std::mem::take(events).into_values().collect();
    let watermarks = std::mem::take(highwaters);
    let new_anchors = std::mem::take(anchors);
    let new_occurrences = std::mem::take(occurrences);
    let mut last_error = None;
    for attempt in 0..4 {
        match store
            .upsert_event_batch_with_anchors(&rows, &watermarks, &new_anchors, &new_occurrences)
            .await
        {
            Ok(()) => {
                if let Err(error) = record_lifecycle_incidents(store, &rows).await {
                    tracing::error!(%error, "recording lifecycle incidents failed");
                    last_error = Some(error);
                } else {
                    return true;
                }
            }
            Err(e) => {
                last_error = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(50 * (1 << attempt))).await;
            }
        }
    }
    if let Some(error) = last_error {
        tracing::error!(%error, rows = rows.len(), "store writer exhausted event-batch retries");
    }
    // Restore uncommitted batch state so subsequent operations (especially Clear)
    // never treat a failed flush as empty or archive an incomplete prefix.
    for row in rows {
        events.insert((row.session_id.clone(), row.seq), row);
    }
    for (sid, seq) in watermarks {
        highwaters
            .entry(sid)
            .and_modify(|cur| *cur = (*cur).max(seq))
            .or_insert(seq);
    }
    for anchor in &new_anchors {
        reducer.forget_anchor(&anchor.session_id, &anchor.key);
    }
    anchors.extend(new_anchors);
    occurrences.extend(new_occurrences);
    false
}

pub const fn lifecycle_incident_recovery_outcome(status: Status) -> Option<&'static str> {
    match status {
        // An authoritative worker snapshot projects an in-flight turn as Busy.
        // It proves that the session is live just as strongly as the idle
        // Running projection, so a prior crash/interruption is no longer active.
        Status::Busy => Some("session_busy"),
        Status::Running => Some("session_running"),
        _ => None,
    }
}

#[cfg(test)]
mod incident_classification_tests;

#[cfg(test)]
mod store_writer_tests;

/// Retention (days) for soft-deleted sessions before the sweeper hard-deletes
/// them + their events.
pub const PURGE_RETENTION_DAYS: i64 = 3;
pub const PROVIDER_USAGE_RETENTION_DAYS: i32 = 30;

/// Periodically hard-delete sessions soft-deleted past [`PURGE_RETENTION_DAYS`],
/// reclaiming their event storage. `interval` fires immediately on the first
/// tick (clears any backlog accrued while the daemon was down), then every 6h.
/// Errors are logged, never fatal.
pub async fn run_purge_sweeper(store: Store) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(6 * 60 * 60));
    loop {
        tick.tick().await;
        match store.purge_deleted(PURGE_RETENTION_DAYS).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(purged = n, "swept soft-deleted sessions past retention"),
            Err(e) => tracing::warn!(error = %e, "purge sweep failed"),
        }
        match store.reclaim_free_pages().await {
            Ok(0) => {}
            Ok(bytes) => tracing::info!(bytes, "returned free database pages to the filesystem"),
            Err(e) => tracing::warn!(error = %e, "database page reclaim failed"),
        }
        match store
            .purge_provider_usage(PROVIDER_USAGE_RETENTION_DAYS)
            .await
        {
            Ok(0) => {}
            Ok(n) => tracing::info!(purged = n, "swept expired provider usage events"),
            Err(e) => tracing::warn!(error = %e, "provider usage purge failed"),
        }
        match store
            .prune_turn_cache_facts(crate::store::turn_cache_facts::DEFAULT_RETENTION_DAYS)
            .await
        {
            Ok(0) => {}
            Ok(n) => tracing::info!(purged = n, "swept expired turn cache facts"),
            Err(e) => tracing::warn!(error = %e, "turn cache facts purge failed"),
        }
    }
}

/// Checkpointed background reconciliation from historical `events` into
/// `turn_cache_facts`. Resets the scan cursor on startup so turns written
/// during a temporary Controller rollback are repaired (`ON CONFLICT DO NOTHING`),
/// yields between bounded batches, and retries transient DB errors with capped
/// exponential backoff until completion or `shutdown`.
pub async fn run_turn_cache_backfill(store: Store, mut shutdown: watch::Receiver<bool>) {
    const SCAN_BATCH: usize = 1_000;
    const MAX_BACKOFF_SECS: u64 = 60;
    let mut backoff_secs = 2_u64;
    loop {
        if *shutdown.borrow() {
            return;
        }
        match store.begin_turn_cache_reconcile().await {
            Ok(()) => {
                backoff_secs = 2;
                break;
            }
            Err(error) => {
                tracing::warn!(%error, backoff_secs, "turn_cache_facts reconcile reset failed; retrying");
                tokio::select! {
                    _ = shutdown.changed() => return,
                    () = tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)) => {}
                }
                backoff_secs = backoff_secs.saturating_mul(2).min(MAX_BACKOFF_SECS);
            }
        }
    }
    loop {
        if *shutdown.borrow() {
            return;
        }
        match store
            .backfill_turn_cache_facts_batch(
                SCAN_BATCH,
                crate::store::turn_cache_facts::DEFAULT_RETENTION_DAYS,
            )
            .await
        {
            Ok(progress) if progress.completed => {
                if progress.rows_projected > 0 {
                    tracing::info!(
                        rows = progress.rows_projected,
                        "completed turn_cache_facts historical reconciliation"
                    );
                }
                return;
            }
            Ok(_) => {
                backoff_secs = 2;
                tokio::select! {
                    _ = shutdown.changed() => return,
                    () = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
                }
            }
            Err(error) => {
                tracing::warn!(%error, backoff_secs, "turn_cache_facts backfill batch failed; retrying");
                tokio::select! {
                    _ = shutdown.changed() => return,
                    () = tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)) => {}
                }
                backoff_secs = backoff_secs.saturating_mul(2).min(MAX_BACKOFF_SECS);
            }
        }
    }
}

/// First retention pass after start, once Machines have reconnected and every
/// restored session's pinned generation is known; then every
/// `plugins.generation_retention_interval` (re-read live before each wait).
pub const GENERATION_RETENTION_DELAY: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// How often the retention task compares each Device's reported disk with its
/// declared `disk.low_watermark` between scheduled passes.
pub const URGENT_RETENTION_POLL: std::time::Duration = std::time::Duration::from_secs(60);

#[cfg(test)]
mod dispatcher_failure_tests;

/// Why a Provider lifecycle fence refuses session runtime admission, or `None`
/// when the fence (an install or reload in progress) leaves sessions usable.
/// `NeedsReconcile` covers an install or uninstall whose Machine outcome is
/// unknown, so it must not be reported as an uninstall.
pub const fn provider_fence_refusal(fence: PluginFenceState) -> Option<&'static str> {
    match fence {
        PluginFenceState::Installing => None,
        PluginFenceState::Uninstalling => {
            Some("the session Provider is uninstalling from its Machine")
        }
        PluginFenceState::Uninstalled => {
            Some("the session Provider was uninstalled from its Machine")
        }
        PluginFenceState::NeedsReconcile => Some(
            "the session Provider's last install or uninstall on its Machine has an unconfirmed \
             outcome; reconcile it in Extensions before continuing",
        ),
    }
}

#[cfg(test)]
mod provider_fence_refusal_tests;

pub async fn prepare_admin_setup(
    state: &Arc<AppState>,
    data_dir: PathBuf,
) -> anyhow::Result<Arc<crate::admin::AdminSetupState>> {
    cowboy_server_auth::server::admin_api::prepare_admin_setup_for(
        state.store.as_ref(),
        &state.hub,
        data_dir,
    )
    .await
}

impl<'a> From<&'a AppState> for ProductRequestAuth<'a> {
    fn from(state: &'a AppState) -> Self {
        Self {
            product_auth_enabled: state.product_auth_enabled,
            store: state.store.as_ref(),
            hub: &state.hub,
            device_access: &state.device_access,
            product_authentication: &state.product_authentication,
        }
    }
}

pub async fn enforce_product_api(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    cowboy_server_auth::server::product_auth_api::enforce_product_api_with_state(
        cowboy_server_auth::server::types::ProductApiEnforcement {
            product_auth_enabled: state.product_auth_enabled,
            store: state.store.as_ref(),
            hub: &state.hub,
            device_access: &state.device_access,
            product_authentication: &state.product_authentication,
            public_origins: state.public_origins.as_slice(),
        },
        peer,
        request,
        next,
    )
    .await
}

pub async fn product_auth_session_message(
    state: &AppState,
    cookie_token: &str,
) -> Option<ProductAuthSessionSnapshot> {
    cowboy_server_auth::server::product_auth_api::product_auth_session_message_for(
        state.store.as_ref(),
        &state.hub,
        &state.product_authentication,
        cookie_token,
    )
    .await
}

pub async fn shutdown_signal(shutdown: watch::Sender<bool>) {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    tracing::warn!(%error, "failed to listen for ctrl-c");
                }
            }
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(%error, "failed to listen for ctrl-c");
    }
    tracing::info!("shutdown requested; draining connections and persistence");
    let _ = shutdown.send(true);
}

pub fn init_tracing() {
    crate::logs::init_stderr();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebPushConfigResponse<'a> {
    pub application_server_key: &'a str,
}

/// Response body for `GET /version`.
#[derive(Debug, Serialize)]
pub struct VersionResponse {
    pub version: String,
}

/// Storage/runtime metrics for the Settings info panel — the capacity dashboard
/// for the unbounded growers (events) + the deleted-session purge backlog.
#[derive(Debug, Serialize)]
pub struct Metrics {
    /// postgres database size (bytes).
    pub db_bytes: i64,
    /// total rows in the events log (the unbounded grower).
    pub events_rows: i64,
    /// live (non-deleted) sessions.
    pub sessions_live: usize,
    /// sessions soft-deleted, awaiting the 3-day purge.
    pub sessions_deleted: i64,
    /// daemon resident memory (bytes), excluding agent subprocesses.
    pub daemon_rss_bytes: u64,
    /// lifetime high-water RSS for the daemon process (bytes).
    pub daemon_rss_peak_bytes: u64,
    /// Current memory charged to the complete Cowboy service cgroup (bytes).
    pub cgroup_memory_bytes: u64,
    /// Lifetime high-water memory for the complete service cgroup (bytes).
    pub cgroup_memory_peak_bytes: u64,
    pub persistence_pending: usize,
    pub persistence_pending_bytes: usize,
    pub persistence_dropped: u64,
    pub persistence_failed_batches: u64,
    pub persistence_last_error: Option<String>,
    pub runtime_connected: bool,
    pub runtime_workers: usize,
    pub runtime_busy_workers: usize,
    pub runtime_draining_workers: usize,
    pub runtime_handoff_workers: usize,
    pub runtime_pending_commands: usize,
    pub code_cache_bytes: u64,
    pub code_cache_hits: u64,
    pub code_cache_misses: u64,
    pub code_cache_evictions: u64,
    pub observability_pending: usize,
    pub observability_pending_bytes: usize,
    pub observability_duplicate_batches: u64,
    pub observability_failed_file_batches: u64,
    pub observability_failed_incident_batches: u64,
    pub observability_dropped_export_batches: u64,
    pub observability_accepted_batches: u64,
    pub observability_dropped_batches: u64,
    pub observability_failed_log_batches: u64,
    pub observability_failed_metric_batches: u64,
    pub observability_failed_trace_batches: u64,
    pub observability_rejected_export_items: u64,
    pub hub_session_count: usize,
    pub hub_hot_log_bytes: usize,
    pub hub_broadcast_last_bytes: usize,
}

#[cfg(test)]
mod daemon_memory_tests;

#[derive(Deserialize)]
pub struct OtlpQuery {
    pub batch_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageExecutorUpdate {
    pub machine_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageRefreshQuery {
    /// `false` starts the refresh and returns at once; every client follows
    /// it through the usage broadcast. Absent keeps the waiting contract that
    /// the operator CLI and older cached clients rely on.
    pub wait: Option<bool>,
}

#[derive(Default, Deserialize)]
pub struct UsageActivityQuery {
    pub window: Option<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct UsageActivityFilter {
    pub window: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub models: Vec<String>,
    pub agents: Vec<String>,
}

#[cfg(test)]
mod usage_activity_filter_tests;

#[derive(Deserialize)]
pub struct ResetScheduleRequest {
    pub fire_at_ms: i64,
    pub confirm: String,
}

pub use crate::usage::now_ms;

#[derive(Debug, Default, Deserialize)]
pub struct DiagnosticLogsQuery {
    pub kind: Option<String>,
    pub severity: Option<String>,
    pub state: Option<String>,
    pub agent: Option<String>,
    pub window: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub session: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiagnosticLogCursor {
    pub occurred_at_ms: i64,
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct DiagnosticLogPage {
    pub items: Vec<crate::store::DiagnosticLogSummary>,
    pub next_cursor: Option<String>,
}

#[cfg(test)]
mod diagnostic_log_query_tests;

#[cfg(test)]
mod trashed_session_command_tests;

#[derive(Debug, Serialize)]
pub struct MachineDeploymentHealthResponse {
    pub connected: bool,
    pub status: String,
    pub active_acp_generation: Option<String>,
    pub workspace_revision: Option<String>,
    pub workspace_ids_sha256: Option<String>,
    pub workspace_owner: Option<crate::machine_protocol::projects::Owner>,
}

#[derive(Debug, Deserialize)]
pub struct MachineEnrollmentRequest {
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MachineEnrollmentResponse {
    pub machine_id: String,
    pub display_name: String,
    pub token: String,
    pub expires_in_seconds: i64,
}

#[derive(Debug, Deserialize)]
pub struct MachineEnrollmentCancelRequest {
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct MachineCommandResponse {
    pub request_id: String,
}

pub const MACHINE_INVENTORY_COMMAND_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(90);
pub const MACHINE_COMPONENT_COMMAND_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(300);

#[cfg(test)]
mod colocated_permission_tests;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ProviderAuthenticationExecutorIdentity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<String>,
    pub provider_id: String,
    pub provider_version: String,
    pub generation_digest: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCallIdentity {
    pub version: Option<String>,
    pub digest: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderAuthCommitRequest {
    /// Exact Provider-declared authentication method that produced this bundle.
    pub method: String,
    /// Provider-declared bundle keys mapped to standard-base64 credential values.
    pub values: BTreeMap<String, String>,
    #[serde(default)]
    pub account_label: Option<String>,
    #[serde(default)]
    pub expected_generation: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderAuthSubmitRequest {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct MachineEnrollRequest {
    pub token: String,
    pub public_key: String,
    pub encryption_public_key: String,
}

#[derive(Debug, Serialize)]
pub struct MachineEnrollResponse {
    pub service_id: String,
    pub machine_id: String,
    pub display_name: String,
    pub fingerprint: String,
}

#[derive(Debug, Serialize)]
pub struct MachineServiceResponse {
    pub service_id: String,
}

// Existing Machine clients collect their bounded CLI/auth inventory after
// receiving the challenge. Those probes can legitimately exceed 15 seconds on
// a loaded macOS host, so keep the nonce window above their worst-case budget.
pub const MACHINE_HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
pub const MACHINE_HEARTBEAT_MS: u64 = 15_000;
pub const MACHINE_HEARTBEAT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(MACHINE_HEARTBEAT_MS * 3);
pub const MACHINE_RUNTIME_OUTBOUND_CAPACITY: usize = 16;

#[derive(Default)]
pub struct MachineHeartbeatWatchdog {
    pub deadline: Option<std::time::Instant>,
}

impl MachineHeartbeatWatchdog {
    pub fn observe_at(&mut self, observed_at: std::time::Instant) {
        self.deadline = Some(observed_at + MACHINE_HEARTBEAT_TIMEOUT);
    }

    pub fn expired_at(&self, observed_at: std::time::Instant) -> bool {
        self.deadline
            .is_some_and(|deadline| observed_at >= deadline)
    }
}

#[cfg(test)]
mod machine_runtime_bridge_tests;

#[cfg(test)]
mod provider_usage_source_tests;

/// Rebuild one session's runtime without clearing its transcript, native agent
/// id, queue, drafts, title, cwd, or persisted config preferences. The
/// Supervisor owns the atomic worker fence/replacement and broadcasts the
/// resulting lifecycle edges to every connected client.
#[derive(Debug, Default, Deserialize)]
pub struct SessionReloadQuery {
    #[serde(default)]
    pub confirm_active_turn: bool,
    #[serde(default)]
    pub upgrade_provider: bool,
    pub expected_generation_digest: Option<String>,
}

/// Request body for the machine wake (`POST /api/sessions/{id}/prompt`).
#[derive(Debug, Deserialize)]
pub struct SessionPromptRequest {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub content: Vec<serde_json::Value>,
    #[serde(default)]
    pub delivery: Option<crate::send_delivery::SendDelivery>,
    #[serde(default)]
    pub front: bool,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub target: Option<crate::send_delivery::DeliveryTarget>,
    /// Optional source draft or queued row reserved and consumed atomically by
    /// this steer operation once the worker admits delivery.
    #[serde(default)]
    pub consume_source: Option<crate::core::SteerSource>,
}

/// Request body for `POST /api/sessions`.
///
/// The retired WS `Inbound::NewSession` was fire-and-forget without a
/// `sessionId` reply or Machine placement. This endpoint is the only Web
/// creation path. It returns a durable `Starting` session before remote
/// workspace preparation completes, so clients can navigate immediately and
/// observe the authoritative preparation lifecycle on the destination page.
#[derive(Debug, Deserialize)]
pub struct NewSessionRequest {
    #[serde(default)]
    pub context_sources: Vec<String>,
    /// Only the durable context transition may reserve a creation identity.
    #[serde(skip)]
    pub reserved_id: Option<String>,
    /// Only a native fork may keep its source's older auth generation.
    #[serde(skip)]
    pub retained_auth_generation: Option<u64>,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub provider_account_id: Option<crate::provider_account::ProviderAccountId>,
    #[serde(default)]
    pub folder_id: Option<String>,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default)]
    pub runtime_machine_id: Option<String>,
    /// Stable machine placement. API/ACP compatibility callers may omit it and
    /// retain their caller-owned local workspace; Web creation must select a
    /// registered Machine.
    #[serde(default = "default_machine_id")]
    pub machine_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Which surface opened the session — defaults to `Api` for direct
    /// `curl`/test callers. The Web UI sends `Web` through this endpoint.
    #[serde(default)]
    pub origin: SessionOrigin,
    /// Create a view-only machine-driven system session. Defaults false; the Web
    /// UI never sets it.
    #[serde(default)]
    pub system: bool,
    /// Optional first turn owned by the creation transaction (for example a
    /// Columbus work-item resume). It is dispatched only after the prepared
    /// workspace has been committed and the worker has started.
    #[serde(default)]
    pub initial_prompt: Option<String>,
}

/// Response body for `POST /api/sessions`.
#[derive(Debug, Serialize)]
pub struct NewSessionResponse {
    pub session_id: String,
    pub provider_account_id: crate::provider_account::ProviderAccountId,
    pub provider_version: String,
    pub provider_generation_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_auth_generation: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct PreparedMachineWorkspace {
    pub path: String,
    pub source_path: String,
    pub revision: Option<String>,
    pub isolated: bool,
    pub created: bool,
}

#[cfg(test)]
mod machine_provider_tests;

#[derive(Debug, Serialize)]
pub struct HistoryResponse {
    pub events: Vec<Envelope>,
    pub next_before_seq: Option<u64>,
    pub reached_start: bool,
}

#[derive(Debug, Serialize)]
pub struct QuestionPagesResponse {
    pub total: u64,
    pub exact: bool,
    pub pages: Vec<crate::core::QuestionPageSummary>,
    pub next_before_seq: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct QuestionPagesQuery {
    pub before: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct SessionBootstrapResponse {
    pub messages: Vec<Outbound>,
    /// Authoritative subagent observations (docs/background-tasks.md I5).
    /// A sibling of `messages`, not an `Outbound`, so protocol bridges and
    /// older Web clients are unaffected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_state: Option<serde_json::Value>,
    /// The account's task acknowledgements in this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_acknowledgements: Option<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SessionBootstrapQuery {
    /// The highest seq the client already holds; children observed after it
    /// are included even when terminal.
    #[serde(default)]
    pub since_seq: Option<u64>,
}

pub fn focused_session_bootstrap(hub: &Hub, session_id: &str) -> Option<Vec<Outbound>> {
    let (events, reached_start) = hub.snapshot(session_id)?;
    let mut messages = vec![Outbound::Snapshot {
        session_id: session_id.to_owned(),
        events,
        reached_start,
    }];
    if let Some(options) = hub.config_options(session_id) {
        messages.push(Outbound::ConfigOptions {
            session_id: session_id.to_owned(),
            options,
        });
    }
    if let Some(queue) = hub.queue_resync(session_id) {
        messages.push(queue);
    }
    Some(messages)
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub before_seq: u64,
    #[serde(default)]
    pub question_page: bool,
}

/// One cursor-addressed, event- and byte-bounded page of a session's history.
/// The client pages UP from the WS tail; older pages arrive here. A COMPLETE
/// past page never changes again, so `x-cowboy-history: complete` lets the
/// service worker satisfy any re-fetch (scroll back, reload, post-recycle
/// reload) with ZERO network. It stays `no-store` for the HTTP cache because
/// sign-out can clear the worker's cache but not that one. The still-growing
/// latest page is unmarked, and the client never asks for it (it has the tail
/// over WS). Unknown session → 404; out-of-range page → `[]`.
pub async fn api_history(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Response {
    if state.hub.session_info(&session_id).is_none() {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    let history = match &state.store {
        Some(store) => {
            if query.question_page {
                store
                    .question_page_before(&session_id, query.before_seq)
                    .await
            } else {
                store
                    .history_page(&session_id, query.before_seq, crate::core::HISTORY_PAGE)
                    .await
            }
        }
        None => Ok(if query.question_page {
            state
                .hub
                .question_page_before(&session_id, query.before_seq)
                .unwrap_or_default()
        } else {
            state
                .hub
                .history_page(&session_id, query.before_seq)
                .unwrap_or_default()
        }),
    };
    let (events, next_before_seq, reached_start) = match history {
        Ok(page) => page,
        Err(e) => {
            tracing::warn!(session = %session_id, before_seq = query.before_seq, error = %e, "history query failed");
            return (StatusCode::SERVICE_UNAVAILABLE, "history unavailable").into_response();
        }
    };
    (
        [
            (header::CACHE_CONTROL.as_str(), "private, no-store"),
            ("x-cowboy-history", "complete"),
        ],
        Json(HistoryResponse {
            events,
            next_before_seq,
            reached_start,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod refresh_identity_evidence_tests;

#[cfg(test)]
mod machine_refresh_tests;

#[cfg(test)]
mod session_route_authorization_tests;

#[cfg(test)]
mod static_asset_tests;

/// App-level WS heartbeat interval. 25s stays under the common 60s proxy/idle
/// timeout (the 75% rule) and keeps NAT mappings warm; the client treats missing
/// ~2 of these as a dead socket. See [`crate::core::Outbound::Ping`].
pub const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(25);

#[derive(Debug, Default, Deserialize)]
pub struct WebSocketQuery {
    pub bootstrap: Option<String>,
    pub session_deltas: Option<String>,
    pub client_id: Option<String>,
    pub client_kind: Option<String>,
    pub dataset: Option<String>,
}

#[derive(Clone)]
pub struct WebSocketAuthentication {
    pub principal: ProductPrincipal,
    pub caller: crate::prompt_origin::RequestCaller,
    pub via_cookie: bool,
    pub cookie_token: Option<String>,
    pub bearer: Option<String>,
    pub device_identity: Option<crate::client_auth::DeviceAccessIdentity>,
    pub capacity: WebSocketCapacityIdentity,
}

#[derive(Clone, Debug)]
pub struct WebSocketCapacityIdentity {
    pub client_id: String,
    pub user_id: Option<String>,
    pub principal_class: String,
    pub session_id: Option<String>,
    pub client_kind: String,
}

pub fn global_bootstrap_with_seq(hub: &Hub, principal: &ProductPrincipal) -> (Vec<Outbound>, u64) {
    let (sessions, sessions_seq, resyncs) =
        hub.global_bootstrap_snapshot_for_client(|owner| principal.can_see(owner));
    let visible = sessions.iter().map(|meta| meta.id.clone()).collect();
    let mut messages = vec![
        Outbound::Sessions { sessions },
        Outbound::Settings {
            // Compatibility tombstone: an older cached client treats an empty
            // settings snapshot as auto-resume disabled during rollout.
            settings: Default::default(),
        },
    ];
    messages.extend(
        resyncs
            .into_iter()
            .filter_map(|message| project_outbound(hub, principal, &visible, message)),
    );
    (messages, sessions_seq)
}

pub fn global_bootstrap(hub: &Hub, principal: &ProductPrincipal) -> Vec<Outbound> {
    global_bootstrap_with_seq(hub, principal).0
}

pub fn connect_bootstrap_with_seq(
    hub: &Hub,
    lazy: bool,
    principal: &ProductPrincipal,
    machine_snapshot: Option<Outbound>,
) -> (Vec<Outbound>, u64) {
    let (mut messages, sessions_seq) = global_bootstrap_with_seq(hub, principal);
    if !lazy {
        for session in hub.session_list_filtered(|owner| principal.can_see(owner)) {
            if let Some(session_messages) = focused_session_bootstrap(hub, &session.id) {
                messages.extend(session_messages);
            }
        }
    }
    messages.extend(machine_snapshot);
    messages.push(Outbound::BootstrapComplete);
    (messages, sessions_seq)
}

pub fn connect_bootstrap(
    hub: &Hub,
    lazy: bool,
    principal: &ProductPrincipal,
    machine_snapshot: Option<Outbound>,
) -> Vec<Outbound> {
    connect_bootstrap_with_seq(hub, lazy, principal, machine_snapshot).0
}

pub fn principal_owned_sync_state(state: &str) -> bool {
    state == "workspace-order" || state == "drafts" || state == "preferences"
}

pub const WS_CAPACITY_REQUIRED_CLOSE_CODE: u16 = 4429;

pub fn active_capacity_message(
    status: &str,
    capacity: Option<&crate::store::ActiveClientCapacity>,
    position: Option<u32>,
    retry_after_ms: Option<i64>,
    channel_count: Option<u32>,
    channel_limit: u32,
) -> Outbound {
    Outbound::ClientCapacity {
        capacity: serde_json::json!({
            "status": status,
            "position": position,
            "retry_after_ms": retry_after_ms,
            "channels": channel_count,
            "channel_limit": channel_limit,
            "active_for_user": capacity.map(|value| value.active_for_user),
            "active_for_service": capacity.map(|value| value.active_for_service),
            "active_automation": capacity.map(|value| value.active_automation),
            "per_user_limit": capacity.map(|value| value.per_user_limit),
            "service_limit": capacity.map(|value| value.service_limit),
            "automation_limit": capacity.map(|value| value.automation_limit),
        }),
    }
}

#[cfg(test)]
mod code_file_policy_tests;

#[cfg(test)]
mod reset_policy_tests;

#[cfg(test)]
mod websocket_send_tests;

#[cfg(test)]
mod code_tree_cache_tests;

#[cfg(test)]
mod bootstrap_tests;

#[cfg(test)]
mod provider_uninstall_tests;

#[cfg(test)]
mod provider_install_tests;

#[cfg(test)]
mod provider_auth_resume_tests;

#[cfg(test)]
mod auth_capacity_boundary_tests;

pub use cowboy_server_auth::server::product_auth_api;
pub use product_auth_api::*;

pub mod usage_api;
pub use usage_api::*;

pub use code_api::*;
pub use cowboy_server_code::server::{
    CodeState, code_api, code_context, code_reads, types::*, workspace_extensions, zed_session,
};

pub mod machine_api;
pub use machine_api::*;

pub mod provider_api;
pub use provider_api::*;

pub mod session_api;
pub use session_api::*;

pub mod machine_connection;
pub use machine_connection::*;

pub mod client_ws;
pub use client_ws::*;

pub mod background_jobs;
pub use background_jobs::*;

pub use cowboy_server_auth::server::http_policy;
pub use cowboy_server_auth::server::types::*;
pub use http_policy::*;

pub mod static_web;
pub use static_web::*;

pub mod observability_api;
pub use observability_api::*;

pub mod plugin_api;
pub use plugin_api::*;

pub mod provider_auth_flow;
pub use provider_auth_flow::*;

pub use code_context::*;
pub mod client_commands;
pub use client_commands::*;
pub mod machine_handshake;
pub use machine_handshake::*;

pub mod machine_events;
pub use machine_events::*;
pub mod provider_reset_timer;
pub use provider_reset_timer::*;

pub mod startup_restore;
pub mod startup_tasks;
pub use startup_restore::*;
pub use startup_tasks::*;
