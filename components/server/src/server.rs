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

mod agent_tools;
mod config_revert;
pub(crate) use agent_tools::launch_extensions;
mod dormant_pins;
mod folder_defaults;
mod memory_reclaim;
mod memory_relay;
mod secure_transport;
mod send_delivery;
mod session_configuration;
mod session_context;
mod session_create_replay;
mod session_maintenance;
mod session_provider_updates;
mod session_reclaim;
mod session_trash;
mod web_roots;
mod workspace_preparation;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read as _;
use std::net::SocketAddr;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Context as _;
use axum::Router;
use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Extension, Form, Json, Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post, put};
use base64::Engine as _;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use agent_client_protocol::schema::v1::ContentBlock;

use crate::acp::AgentCommand;
use crate::cli::ServeArgs;
use crate::code_adapter::CodeOperation;
use crate::code_review::CodeProvider as _;
use crate::core::{
    CodeReadScope, CommandOutcome, DispatchReq, Envelope, Event, FanoutFrame, Hub, Inbound,
    Outbound, PersistenceHealth, RestoredSession, SessionOrigin, Status, StoreReceiver, StoreSink,
    StoreWrite, project_sync_value,
};
use crate::diff_snapshot::{DiffSnapshotCache, DiffSnapshotKey};
use crate::machine_control::MachineControl;
use crate::machine_protocol::CONTROLLER_RECONNECT_MAX_BACKOFF_SECONDS;
use crate::observability::{Observability, SubmitReceipt, TelemetryBatch};
use crate::persistence::EventReducer;
use crate::product_auth::{ProductPrincipal, WS_AUTH_REQUIRED_CLOSE_CODE};
use crate::remote_runtime::{RemoteBootstrap, RemoteRuntime};
use crate::runtime::RuntimeHealth;
use crate::runtime_router::RuntimeRouter;
use crate::session_folders::{FolderActor, project_folders_value};
use crate::store::{ActiveClientRelease, Store};
use crate::supervisor::Supervisor;
use crate::usage::UsageService;
use crate::web_push::WebPushService;
#[cfg(test)]
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, watch};
use tokio_util::io::ReaderStream;

mod agent_api;
mod background_tasks;
#[cfg(test)]
mod background_tasks_tests;
mod cardea_authorization;
mod code_buffers;
mod code_reads;
mod draft_documents;
mod execution;
#[cfg(unix)]
mod local_operator;
mod machine_lanes;
mod managed_calls;
mod notifications;
mod operator_approval;
#[cfg(test)]
mod persistence_tests;
mod plugin_install;
mod preferences;
mod provider_accounts;
mod workspace_extensions;
use plugin_install::api_machine_plugin_install;
mod plugin_history;
mod plugin_uninstall;
mod product_continuation;
mod projects;
mod provider_auth_sync;
mod sync_dataset;
mod telemetry_binding;
mod web_security;
mod zed_session;
use plugin_uninstall::{
    api_machine_plugin_operation_receipt, api_machine_plugin_operations,
    api_machine_plugin_recovery_assessment, api_machine_plugin_uninstall,
};

#[derive(Clone)]
struct PluginUninstallPlan {
    actor: crate::plugin_operation::Actor,
    machine_id: String,
    plugin_id: String,
    plugin_version: String,
    generation_digest: String,
    installation_revision:
        Option<crate::machine_protocol::installation_revision::InstallationRevision>,
    contract_fingerprint: String,
    session_ids: Vec<String>,
    active_session_ids: Vec<String>,
    purge_after_ms: i64,
    expires_at_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PluginFenceState {
    Installing,
    Uninstalling,
    Uninstalled,
    NeedsReconcile,
}

type PluginLifecycleFences = Arc<parking_lot::RwLock<HashMap<(String, String), PluginFenceState>>>;

/// Reserve the lifecycle before reading the active release, including across
/// awaits. Reuse the non-destructive installing fence; cancellation releases it.
struct ProviderReloadFence {
    fences: PluginLifecycleFences,
    key: (String, String),
}

impl ProviderReloadFence {
    fn acquire(fences: &PluginLifecycleFences, key: (String, String)) -> Result<Self, String> {
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

const fn provider_session_has_active_turn(status: crate::agent_model::Status) -> bool {
    matches!(status, crate::agent_model::Status::Busy)
}

struct ProviderAuthReconciliation {
    expired: Vec<(String, String)>,
    active: Option<(String, ProviderAuthExecutor)>,
}

impl ProviderAuthReconciliation {
    fn resumable(&self, active_request_id: Option<&str>) -> Option<(String, ProviderAuthExecutor)> {
        self.active
            .as_ref()
            .filter(|(request_id, _)| active_request_id == Some(request_id.as_str()))
            .cloned()
    }
}

struct AppState {
    service_id: String,
    service_config_path: PathBuf,
    /// The Service configuration file, with live fields following edits.
    service_config: crate::config::Handle,
    hub: Hub,
    supervisor: Arc<Supervisor>,
    /// Kept for read-only storage metrics (`/api/metrics`). `None` in-memory.
    store: Option<Store>,
    persistence_health: Option<Arc<PersistenceHealth>>,
    shutdown: watch::Receiver<bool>,
    runtime_health: Arc<RuntimeHealth>,
    runtime_router: Arc<RuntimeRouter>,
    machine_control: Arc<MachineControl>,
    machine_snapshots: MachineSnapshots,
    session_maintenance: session_maintenance::Coordinator,
    plugin_catalog: Arc<crate::plugin_catalog::PluginCatalog>,
    plugin_storage: crate::plugin_storage::PluginStorage,
    provider_catalog: Arc<crate::provider_catalog::ProviderCatalog>,
    provider_auth: Arc<crate::provider_service::ProviderAuthService>,
    provider_auth_sync: provider_auth_sync::Coordinator,
    provider_auth_executors: parking_lot::Mutex<HashMap<String, ProviderAuthExecutor>>,
    plugin_uninstall_plans: parking_lot::Mutex<HashMap<String, PluginUninstallPlan>>,
    plugin_resolution_plans: plugin_uninstall::resolution::ResolutionPlans,
    telemetry_resolution_plans: Arc<telemetry_binding::resolution::surface::Plans>,
    telemetry_recovery_plans: Arc<telemetry_binding::recovery::surface::Plans>,
    telemetry_binding_plans: Arc<telemetry_binding::surface::Plans>,
    telemetry_binding_fence: crate::telemetry_binding::LegacyFence,
    telemetry_writer_admission:
        Option<Arc<crate::telemetry_plugin::writer_admission::WriterAdmission>>,
    plugin_lifecycle_fences: PluginLifecycleFences,
    session_create_replays: Arc<session_create_replay::Replays>,
    desired_machine_components: Arc<crate::machine_convergence::DesiredComponentSource>,
    component_convergence: Arc<crate::machine_convergence::ConvergenceState>,
    convergence_freeze: Arc<crate::machine_convergence::ConvergenceFreeze>,
    desired_machine_plugins: Arc<crate::machine_convergence::PluginDesiredSource>,
    web_root: PathBuf,
    web_roots: web_roots::WebRoots,
    usage: UsageService,
    diff_snapshots: DiffSnapshotCache,
    file_cursors: code_reads::file_pages::PageCursors,
    code_cache: crate::code_cache::CodeCache,
    code_buffers: Arc<code_buffers::Owners>,
    code_navigation_admission: bool,
    project_placement: crate::project_placement::Store,
    execution_preparations: parking_lot::Mutex<std::collections::HashSet<String>>,
    workspace_preparations: parking_lot::Mutex<std::collections::HashSet<String>>,
    execution_readiness: execution::recovery::Readiness,
    lost_turns: execution::recovery::LostTurns,
    execution_closures:
        Arc<parking_lot::Mutex<std::collections::HashMap<String, execution::Maintenance>>>,
    managed_calls: Arc<managed_calls::Coordinator>,
    memory_relay: memory_relay::Relay,
    /// Machines permitted to have their Code reads executed on the
    /// Controller's own filesystem. A hello's declared connection mode is a
    /// request; this set is the permission.
    colocated_machines: std::collections::BTreeSet<String>,
    zed_adapter_socket: Option<PathBuf>,
    observability: Observability,
    web_push: Arc<WebPushService>,
    notifications: notifications::Presence,
    public_origins: Arc<Vec<String>>,
    product_auth_enabled: bool,
    product_authentication: Arc<crate::auth_plugins::ProductAuthentication>,
    oidc_transactions: Arc<crate::oidc::OidcTransactions>,
    oidc_native_handoffs: Arc<crate::oidc::NativeHandoffs>,
    device_authorizations: Arc<crate::client_auth::DeviceAuthorizations>,
    device_access: Arc<crate::client_auth::DeviceAccessSessions>,
    capacity_runtime: Arc<ActiveCapacityRuntime>,
}

#[derive(Default)]
struct ActiveCapacityRuntime {
    channels: parking_lot::Mutex<HashMap<String, LocalActiveClient>>,
    changed: tokio::sync::Notify,
}

#[derive(Debug, Default)]
struct LocalActiveClient {
    channels: u32,
    fencing_token: Option<i64>,
}

impl ActiveCapacityRuntime {
    fn open_channel(&self, client_id: &str, limit: u32, enforce: bool) -> Result<u32, u32> {
        let mut clients = self.channels.lock();
        let client = clients.entry(client_id.to_owned()).or_default();
        let next = client.channels.saturating_add(1);
        if enforce && next > limit {
            return Err(client.channels);
        }
        client.channels = next;
        Ok(next)
    }

    fn record_fence(&self, client_id: &str, fencing_token: i64) {
        if let Some(client) = self.channels.lock().get_mut(client_id) {
            client.fencing_token = Some(fencing_token);
        }
    }

    fn close_channel(&self, client_id: &str) -> Option<i64> {
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

    fn notify_changed(&self) {
        self.changed.notify_waiters();
    }
}

struct ActiveCapacityChannel {
    runtime: Arc<ActiveCapacityRuntime>,
    store: Option<Store>,
    client_id: String,
    reservation_ms: i64,
    per_user_limit: u32,
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

const MACHINE_RUNTIME_STARTUP_GRACE: std::time::Duration = std::time::Duration::from_secs(45);
// Legacy Machine hosts begin their heartbeat only after welcome-time component
// reconciliation. Keep that explicitly identified work out of the
// "unresponsive" state while still bounding a genuinely stuck update.
const MACHINE_COMPONENT_RECONCILIATION_GRACE: std::time::Duration =
    std::time::Duration::from_secs(330);

#[cfg(test)]
mod component_supersession_tests;

#[cfg(test)]
mod machine_health_tests;

const STORE_QUEUE_CAPACITY: usize = 8_192;
const FORCE_CANCEL_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
/// An execution-bound native interrupt first cancels its target processes over
/// the Machine execution link: several round trips, each a second or more on a
/// slow link, and the link can stall for 30-60 s. That is progress, not a hung
/// turn. Recycling instead forces a cold remote resume that crosses the same
/// link and costs far more than the cancellation it replaces.
const EXECUTION_FORCE_CANCEL_GRACE: std::time::Duration = std::time::Duration::from_secs(60);
const QUEUE_EDIT_RECONNECT_GRACE: std::time::Duration = std::time::Duration::from_secs(15);
/// How long a dropped Machine stays `reconnecting` before the sweeper marks it
/// offline and clients surface "not connected". A remote Machine reaches the
/// Controller over pooled cross-border streams: after a reset it backs off up
/// to `CONTROLLER_RECONNECT_MAX_BACKOFF_SECONDS`, then needs a fresh stream
/// (one dial of up to 10 s, or an overlay fallback of the same budget) and the
/// TLS/auth handshake over a lossy path. 15 s expired on most of OVH's
/// reconnects (11 expiries for 14 resets in one day) although the Machine was
/// back within a minute; cover one full backoff plus one stream dial.
const MACHINE_RECONNECT_GRACE_SECONDS: i32 = 60;
/// A connected Machine still needs time to replay its authoritative worker
/// snapshots. Cover the Machine's entire outbound reconnect backoff plus a
/// bounded settlement window before declaring persisted Busy turns dead.
const RUNTIME_SNAPSHOT_SETTLEMENT_SECONDS: u64 = 15;
const RUNTIME_RECONCILIATION_GRACE: std::time::Duration = std::time::Duration::from_secs(
    CONTROLLER_RECONNECT_MAX_BACKOFF_SECONDS + RUNTIME_SNAPSHOT_SETTLEMENT_SECONDS,
);
const MACHINE_RECONNECT_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(test)]
#[test]
fn runtime_reconciliation_covers_machine_retry_and_snapshot_settlement() {
    assert_eq!(
        RUNTIME_RECONCILIATION_GRACE,
        std::time::Duration::from_secs(45)
    );
    assert!(
        RUNTIME_RECONCILIATION_GRACE
            > std::time::Duration::from_secs(CONTROLLER_RECONNECT_MAX_BACKOFF_SECONDS)
    );
}
/// Let restore, listener binding, and Machine reconnection settle before the
/// optional account collectors start their short-lived provider processes.
const PROVIDER_UNINSTALL_RETENTION_MS: i64 = 3 * 24 * 60 * 60 * 1_000;
const PROVIDER_UNINSTALL_PLAN_TTL_MS: i64 = 10 * 60 * 1_000;

#[derive(Debug, PartialEq, Eq)]
enum ScheduledResetFailurePolicy {
    RetryPreflight,
    StopFailed,
    StopUnknown,
}

fn scheduled_reset_failure_policy(
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

/// Start the HTTP/WebSocket server and the agent supervisor.
pub async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    let runtime_restrictions = Arc::new(crate::project_placement::RuntimeRestrictions::parse(
        &args.provider_runtime_machines,
    )?);
    if args.check_plugin_catalog {
        let catalog = crate::plugin_catalog::PluginCatalog::inspect(
            &args.data_dir,
            args.plugin_catalog_dir.clone(),
        )?;
        let released = catalog
            .entries()
            .into_iter()
            .filter(|entry| {
                matches!(
                    entry.release_state,
                    crate::plugin_catalog::PluginReleaseState::Ready
                )
            })
            .map(|entry| {
                serde_json::json!({
                    "plugin_id": entry.plugin_id,
                    "plugin_version": entry.plugin_version,
                    "artifact_digest": entry.artifact_digest,
                })
            })
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema": "dravengarden.cowboy.catalog-reader-preflight/v1",
                "status": "readable",
                "supported_release_schema": cowboy_plugin_sdk::RELEASE_SCHEMA_VERSION,
                "supported_provider_sdk_version": cowboy_provider_sdk::PROVIDER_SDK_VERSION,
                "releases": released,
                "not_checked": ["runtime_artifact_bytes", "host_activation", "database_migrations", "real_login"],
            }))?
        );
        return Ok(());
    }
    let (plugin_catalog, product_authentication, core_security) =
        crate::plugin_activation::prepare_controller_hosts(&args)?;
    if args.check_plugin_hosts {
        let mut report = plugin_catalog.host_preflight_report()?;
        report.telemetry = Some(crate::telemetry_plugin::controller_policy::inspect(&args)?);
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    anyhow::ensure!(
        args.product_auth_enabled,
        "Device authentication is mandatory; authentication cannot be disabled"
    );
    // An invalid Service configuration refuses startup before any state is
    // touched, so a rolling update keeps the previous instance serving.
    let service_config_path = crate::config::Scope::Service.path_in(&args.data_dir);
    let service_config = crate::config::Handle::new(crate::config::load(
        crate::config::Scope::Service,
        &service_config_path,
    )?);
    // Lives for the whole process; it only reads the file and swaps values.
    let _service_config_watch =
        service_config.watch(crate::config::Scope::Service, service_config_path.clone());
    plugin_catalog.initialize()?;
    let plugin_dir =
        crate::plugin_dir::PluginDir::open(&args.data_dir).context("opening plugin directory")?;
    let _security_owner = crate::core_security::ControllerLock::acquire(&plugin_dir)?;
    let plugin_catalog = Arc::new(plugin_catalog);
    let product_authentication = Arc::new(product_authentication);
    let service_identity = crate::service_identity::load_or_create(&args.data_dir)
        .context("loading Cowboy Service identity")?;
    let service_id = service_identity.as_str().to_owned();
    let crate::telemetry_plugin::controller_policy::ControllerPolicy {
        writer: telemetry_writer_admission,
        background: managed_export_policy,
    } = crate::telemetry_plugin::controller_policy::ControllerPolicy::load(&args, &service_id)?;
    let desired_machine_components =
        Arc::new(crate::machine_convergence::DesiredComponentSource::load(
            args.machine_components_manifest.as_deref(),
            now_ms(),
        )?);
    let component_convergence = Arc::new(crate::machine_convergence::ConvergenceState::default());
    let convergence_freeze = Arc::new(crate::machine_convergence::ConvergenceFreeze::new(
        &args.data_dir,
    ));
    let desired_machine_plugins = Arc::new(crate::machine_convergence::PluginDesiredSource::new(
        &args.data_dir,
    ));
    init_tracing();
    let log_directory = crate::logs::directory(&args.data_dir);
    let log_guard = crate::logs::init(
        log_directory.clone(),
        crate::logs::Context::new("cowboy-controller"),
    )?
    .track_outcome();
    if args.cardea_oidc_config.is_some() && !args.product_auth_enabled {
        tracing::warn!("Cardea OIDC is configured but product authentication is disabled");
    }
    if args.auth_config.is_some() && !args.product_auth_enabled {
        tracing::warn!(
            "authentication Plugins are configured but product authentication is disabled"
        );
    }
    tracing::info!(
        compiled =
            crate::memory_observability::compiled_malloc_conf().unwrap_or("jemalloc defaults"),
        runtime_override = crate::memory_observability::runtime_malloc_conf_override()
            .as_deref()
            .unwrap_or("none"),
        "jemalloc configuration"
    );
    tracing::info!(
        tokio_workers = std::env::var("COWBOY_TOKIO_WORKERS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(4),
        "controller tokio runtime"
    );
    let code_cache =
        crate::code_cache::CodeCache::open(args.data_dir.join("code-cache"), args.code_cache_bytes)
            .map_err(anyhow::Error::msg)
            .context("opening code content cache")?;
    let web_push = WebPushService::open(&args.data_dir).context("opening Web Push service")?;
    let provider_catalog = Arc::new(crate::provider_catalog::ProviderCatalog::open(
        &args.data_dir,
        Arc::clone(&plugin_catalog),
    )?);
    let provider_auth = Arc::new(crate::provider_service::ProviderAuthService::open(
        &args.data_dir,
    )?);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    // Persistence closes only after dispatcher/runtime quiescence. Using the
    // HTTP shutdown signal directly would race a last-millisecond prompt
    // requeue against the writer closing its receiver.
    let (store_shutdown_tx, store_shutdown_rx) = watch::channel(false);
    let runtime_health = Arc::new(RuntimeHealth::default());
    // Phase 2: when --database-url is supplied, hook in the persistent store.
    // Migrations run on every start (sqlx tracks applied versions, so it's
    // idempotent); the in-memory Hub is then warmed from the DB before WS
    // clients can connect. Without a database URL the daemon falls back to
    // pure in-memory mode — same behaviour as before, useful for dev or for
    // running on a host without durable storage configured.
    let plugin_lifecycle_fences;
    let telemetry_binding_fence;
    let managed_export_activation;
    let (hub, mut store, persistence_health, writer_task, purge_task, session_id_floor) =
        if let Some(url) = args.database_url() {
            let store = Store::connect(url, args.data_dir.join("artifacts"))
                .await
                .context("connecting database")?;
            store.migrate().await.context("running migrations")?;
            store
                .initialize_core_security(core_security.as_ref(), &plugin_dir)
                .await
                .context("initializing core security authority")?;
            // Restore durable admission fences before any background writer,
            // sweeper, Plugin host or runtime can race the startup scan.
            plugin_lifecycle_fences = plugin_uninstall::recover_fences(Some(&store), &service_id)
                .await
                .context("restoring Plugin lifecycle fences")?;
            telemetry_binding_fence =
                crate::telemetry_binding::LegacyFence::recover(Some(&store), &service_id)
                    .await
                    .context("restoring Service telemetry binding fence")?;
            managed_export_activation = match managed_export_policy {
                Some(policy) => Some(policy.activate(&store).await),
                None => None,
            };
            let session_id_floor = store
                .next_session_number()
                .await
                .context("seeding session id counter")?;
            let health = Arc::new(PersistenceHealth::default());
            let (sink, rx) = StoreSink::channel(STORE_QUEUE_CAPACITY, Arc::clone(&health));
            let hub = Hub::with_store(Some(sink));
            let settings = store
                .load_settings()
                .await
                .context("loading persisted auth settings")?;
            hub.load_settings(settings);
            store
                .integrate_draft_folders()
                .await
                .context("integrating Draft directories")?;
            hub.restore_workspace_orders(store.load_workspace_orders().await?);
            let folders = store
                .load_session_folders()
                .await
                .context("loading session folders")?;
            hub.restore_session_folders(folders);
            // Warm restore — sessions + events come back exactly as the daemon
            // left them, so on a fresh process every WS client's first snapshot
            // is correct.
            let loaded = store.load_all().await.context("loading persisted state")?;
            let restored: Vec<_> = loaded
                .into_iter()
                .map(|ls| RestoredSession {
                    meta: ls.meta,
                    log: ls.events,
                    event_count: ls.event_count,
                    reached_start: ls.reached_start,
                    next_seq: ls.next_seq,
                    queue: ls.queue,
                    drafts: ls.drafts,
                    config_options: ls.config_options,
                    config_preferences: ls.config_preferences,
                    mobile_review_state: ls.mobile_review_state,
                    folder_id: ls.folder_id,
                    last_event_at_ms: ls.last_event_at_ms,
                })
                .collect();
            let restored_count = restored.len();
            // Adopt durable anchors before any reducer sees a new event, so a
            // restarted Controller never starts a second subagent/Call row.
            match store
                .recent_transcript_anchors(crate::transcript_anchors::HUB_WARM_ANCHORS)
                .await
            {
                Ok(anchors) => hub.warm_transcript_anchors(anchors),
                Err(error) => tracing::warn!(%error, "warming transcript anchors failed"),
            }
            match store
                .recent_subagent_snapshots(crate::transcript_anchors::WARM_SNAPSHOT_MARKS)
                .await
            {
                Ok(snapshots) => hub.warm_subagent_marks(snapshots),
                Err(error) => tracing::warn!(%error, "warming subagent marks failed"),
            }
            // Detached workers outlive this control-plane process. Keep
            // persisted Busy turns guarded until their Machine runtime has had
            // one bounded reconnect window to prove ownership; only then may a
            // missing worker become Interrupted.
            hub.restore_reconciling_runtime(restored);
            crate::session_context::restore_announcements(&hub);
            hub.restore_continuations();
            hub.restore_forks();
            hub.set_artifacts(store.artifacts());
            tracing::info!(restored = restored_count, "persistence wired",);
            // Background DB writer: dequeues StoreWrite intents and applies them.
            // Errors are logged but don't bring the daemon down — the in-memory
            // state remains authoritative for the current process.
            runtime_health.set_store_writer(true);
            let writer_health = Arc::clone(&runtime_health);
            let writer_store = store.clone();
            let writer_persistence_health = Arc::clone(&health);
            let writer_shutdown = store_shutdown_rx.clone();
            let writer_task = tokio::spawn(async move {
                run_store_writer(writer_store, rx, writer_persistence_health, writer_shutdown)
                    .await;
                writer_health.set_store_writer(false);
            });
            // Background sweeper: hard-delete sessions soft-deleted past the
            // retention window, reclaiming their event storage.
            runtime_health.set_purge_sweeper(true);
            let purge_health = Arc::clone(&runtime_health);
            let purge_store = store.clone();
            let purge_task = tokio::spawn(async move {
                run_purge_sweeper(purge_store).await;
                purge_health.set_purge_sweeper(false);
                tracing::error!("purge sweeper exited unexpectedly");
            });
            (
                hub,
                Some(store),
                Some(health),
                Some(writer_task),
                Some(purge_task),
                session_id_floor,
            )
        } else {
            tracing::info!("no --database-url: running in-memory only");
            plugin_lifecycle_fences = plugin_uninstall::recover_fences(None, &service_id).await?;
            telemetry_binding_fence =
                crate::telemetry_binding::LegacyFence::recover(None, &service_id).await?;
            managed_export_activation = None;
            (Hub::new(), None, None, None, None, 1)
        };
    let plugin_storage = match store.as_ref() {
        Some(store) => store.plugin_storage(plugin_dir.clone()),
        None => crate::plugin_storage::PluginStorage::sqlite_files(plugin_dir.clone()),
    };
    let plugin_runtime = match plugin_catalog.activate_runtime(&plugin_storage).await {
        Ok(runtime) => runtime,
        Err(error) if plugin_catalog.strict_host_activation() => {
            return Err(error).context("activating selected Plugin hosts");
        }
        Err(error) => {
            tracing::error!(%error, "plugin host activation failed");
            plugin_catalog.install_empty_runtime()
        }
    };
    if let Some(store) = store.as_mut().filter(|_| core_security.is_none()) {
        let namespace = plugin_runtime
            .namespace_for_capability(crate::core_passkeys::STORAGE_CAPABILITY)
            .context("WebAuthn plugin storage is unavailable")?;
        store
            .attach_passkey_storage(namespace)
            .await
            .context("binding core Passkey storage at the legacy namespace")?;
    }
    let machine_control = Arc::new(MachineControl::new(service_identity));
    let usage = UsageService::with_plugin_catalog(
        store.clone(),
        Some(args.data_dir.join("usage-snapshot.json")),
        Arc::clone(&plugin_catalog),
        Arc::clone(&machine_control),
    )
    .with_config(service_config.clone())
    .with_runtime_restrictions(Arc::clone(&runtime_restrictions));
    usage.restore_execution_settings().await?;
    let runtime_router = RuntimeRouter::new();
    // Usage collectors and authenticated Machine connections must share this
    // registry; a second instance leaves collectors permanently without routes.
    // Built here rather than through a constructor: every field is a clone of
    // something this scope already owns, and the list is long enough that
    // naming each one at the call site reads better than an argument order.
    let machine_snapshots = MachineSnapshots {
        store: store.clone(),
        hub: hub.clone(),
        runtime_router: Arc::clone(&runtime_router),
        machine_control: Arc::clone(&machine_control),
        desired_components: Arc::clone(&desired_machine_components),
        component_convergence: Arc::clone(&component_convergence),
        convergence_freeze: Arc::clone(&convergence_freeze),
        desired_plugins: Arc::clone(&desired_machine_plugins),
        product_auth_enabled: args.product_auth_enabled,
        revision: Arc::new(AtomicU64::new(0)),
        service_config: service_config.clone(),
    };
    let machine_presence_task = store.as_ref().map(|store| {
        let store = store.clone();
        let machine_snapshots = machine_snapshots.clone();
        let shutdown = shutdown_rx.clone();
        tokio::spawn(run_machine_presence_sweeper(
            store,
            machine_snapshots,
            shutdown,
        ))
    });
    // Reset credits belong to provider accounts, not sessions. Restore one
    // shared timer per provider and keep them independent from session queues.
    if let Some(store) = store.as_ref() {
        for provider in &usage.reset_provider_ids() {
            match store.load_provider_reset(provider).await {
                Ok(Some(action)) => {
                    usage
                        .set_reset_schedule(
                            provider.as_str(),
                            Some(crate::usage::ResetSchedule {
                                fire_at_ms: action.fire_at_ms,
                            }),
                        )
                        .await;
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, %provider, "loading scheduled provider reset"),
            }
        }
    }
    let reset_task = {
        let usage = usage.clone();
        let store = store.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            loop {
                for provider in &usage.reset_provider_ids() {
                    let action = match store.as_ref() {
                        Some(store) => store.load_provider_reset(provider).await.ok().flatten(),
                        None => None,
                    };
                    if let Some(action) = action.filter(|item| item.next_attempt_at_ms <= now_ms())
                    {
                        let key = action.idempotency_key;
                        // Plugins that cannot accept an idempotency key claim
                        // the one-shot timer before any provider call so a
                        // crash or ambiguous response cannot consume a later
                        // reset on an automatic retry.
                        // Pacing may delay a due reset; reserve it before an
                        // irreversible claim so pacing never discards it.
                        let reservation = match usage.reserve_reset(provider).await {
                            Ok(reservation) => reservation,
                            Err(error) => {
                                tracing::debug!(%error, %provider, "scheduled provider reset waits for usage pacing");
                                continue;
                            }
                        };
                        let claim_before = usage.reset_claims_before_attempt(provider);
                        if claim_before {
                            let Some(store) = store.as_ref() else {
                                continue;
                            };
                            match store.claim_provider_reset(provider, &key).await {
                                Ok(true) => {}
                                Ok(false) => continue,
                                Err(error) => {
                                    tracing::error!(%error, %provider, "claiming scheduled provider reset");
                                    continue;
                                }
                            }
                            usage.set_reset_schedule(provider, None).await;
                        }
                        if let Some(store) = store.as_ref() {
                            let _ = store
                                .append_provider_action_log(
                                    provider,
                                    "scheduled",
                                    "started",
                                    "preflight",
                                    "Scheduled reset attempt started",
                                    None,
                                    Some(&key),
                                    now_ms(),
                                )
                                .await;
                        }
                        match usage
                            .consume_reserved_reset(reservation, provider, &key)
                            .await
                        {
                            Ok(result) => {
                                tracing::info!(%provider, outcome = %result.outcome, "scheduled provider reset finished");
                                if let Some(store) = store.as_ref() {
                                    let _ = store
                                        .append_provider_action_log(
                                            provider,
                                            "scheduled",
                                            "succeeded",
                                            "provider_response",
                                            &result.outcome,
                                            result.credit_id.as_deref(),
                                            Some(&key),
                                            now_ms(),
                                        )
                                        .await;
                                }
                                if !claim_before && let Some(store) = store.as_ref() {
                                    match store.claim_provider_reset(provider, &key).await {
                                        Ok(true) => usage.set_reset_schedule(provider, None).await,
                                        Ok(false) => {}
                                        Err(error) => {
                                            tracing::warn!(%error, %provider, "clearing scheduled provider reset");
                                        }
                                    }
                                }
                            }
                            Err(error) => {
                                if claim_before {
                                    let (status, phase) = if error.call_may_have_reached_provider {
                                        ("unknown", "consume")
                                    } else {
                                        ("failed", "preflight")
                                    };
                                    tracing::error!(%error, %provider, "scheduled one-shot provider reset stopped without retry");
                                    if let Some(store) = store.as_ref() {
                                        let _ = store
                                            .append_provider_action_log(
                                                provider,
                                                "scheduled",
                                                status,
                                                phase,
                                                &error.to_string(),
                                                error.credit_id.as_deref(),
                                                Some(&key),
                                                now_ms(),
                                            )
                                            .await;
                                    }
                                    continue;
                                }
                                if scheduled_reset_failure_policy(
                                    error.call_may_have_reached_provider,
                                    action.attempt_count,
                                ) == ScheduledResetFailurePolicy::StopUnknown
                                {
                                    tracing::error!(%error, %provider, "scheduled provider reset outcome unknown; automatic retry disabled");
                                    if let Some(store) = store.as_ref() {
                                        let _ = store
                                            .append_provider_action_log(
                                                provider,
                                                "scheduled",
                                                "unknown",
                                                "consume",
                                                &error.to_string(),
                                                error.credit_id.as_deref(),
                                                Some(&key),
                                                now_ms(),
                                            )
                                            .await;
                                        match store.claim_provider_reset(provider, &key).await {
                                            Ok(true) => {
                                                usage.set_reset_schedule(provider, None).await;
                                            }
                                            Ok(false) => {}
                                            Err(clear_error) => {
                                                tracing::warn!(%clear_error, %provider, "clearing scheduled provider reset");
                                            }
                                        }
                                    }
                                } else if scheduled_reset_failure_policy(
                                    false,
                                    action.attempt_count,
                                ) == ScheduledResetFailurePolicy::RetryPreflight
                                {
                                    tracing::warn!(%error, %provider, "scheduled provider reset preflight failed; retrying safely");
                                    if let Some(store) = store.as_ref() {
                                        let _ = store
                                            .append_provider_action_log(
                                                provider,
                                                "scheduled",
                                                "retrying",
                                                "preflight",
                                                &error.to_string(),
                                                error.credit_id.as_deref(),
                                                Some(&key),
                                                now_ms(),
                                            )
                                            .await;
                                        let _ = store
                                            .defer_provider_reset(
                                                provider,
                                                &key,
                                                now_ms().saturating_add(60_000),
                                            )
                                            .await;
                                    }
                                } else {
                                    tracing::error!(%error, %provider, "scheduled provider reset preflight retry limit reached");
                                    if let Some(store) = store.as_ref() {
                                        let _ = store
                                            .append_provider_action_log(
                                                provider,
                                                "scheduled",
                                                "failed",
                                                "preflight",
                                                &error.to_string(),
                                                error.credit_id.as_deref(),
                                                Some(&key),
                                                now_ms(),
                                            )
                                            .await;
                                        match store.claim_provider_reset(provider, &key).await {
                                            Ok(true) => {
                                                usage.set_reset_schedule(provider, None).await;
                                            }
                                            Ok(false) => {}
                                            Err(clear_error) => {
                                                tracing::warn!(%clear_error, %provider, "clearing scheduled provider reset");
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(15)) => {}
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() { break; }
                    }
                }
            }
        })
    };
    let supervisor = Arc::new(
        Supervisor::new(
            hub.clone(),
            args.workspace_root.clone(),
            session_id_floor,
            Arc::clone(&runtime_router),
        )
        .with_runtime_restrictions(runtime_restrictions),
    );
    let mut local_sinks: Vec<Box<dyn crate::logs::EvidenceSink>> = Vec::new();
    if matches!(
        args.telemetry_local_backend,
        crate::logs::LocalBackend::Sqlite | crate::logs::LocalBackend::Both
    ) {
        local_sinks.push(Box::new(crate::logs::SqliteStore::open(
            log_directory,
            false,
        )?));
    }
    if matches!(
        args.telemetry_local_backend,
        crate::logs::LocalBackend::Jsonl | crate::logs::LocalBackend::Both
    ) {
        local_sinks.push(Box::new(
            crate::telemetry_file::TelemetryFile::open(
                args.telemetry_dir
                    .unwrap_or_else(|| crate::telemetry_file::default_directory(&args.data_dir)),
                args.telemetry_segment_bytes,
                args.telemetry_retained_files,
                chrono::Utc::now().timestamp_millis(),
            )
            .context("opening local telemetry")?
            .with_retention_seconds(args.telemetry_retain_seconds)?,
        ));
    }
    let observability = Observability::start(
        store.clone(),
        crate::logs::Fanout(local_sinks),
        match managed_export_activation {
            Some(crate::telemetry_plugin::background_policy::Activation::Active(policy)) => {
                Some(telemetry_binding::background::exporter(
                    policy,
                    store
                        .clone()
                        .context("managed telemetry requires a durable store")?,
                    Arc::clone(&machine_control),
                    Arc::clone(&plugin_catalog),
                    Arc::clone(&plugin_lifecycle_fences),
                ))
            }
            Some(crate::telemetry_plugin::background_policy::Activation::Stopped) => None,
            None => crate::telemetry_plugin::controller_exporter(
                args.telemetry_plugin_config.as_deref(),
                Arc::clone(&machine_control),
                Arc::clone(&plugin_catalog),
                telemetry_binding_fence.clone(),
            )?,
        },
    );
    let telemetry_shutdown = observability.clone();

    // Background dispatcher: the Hub owns each session's send-queue but can't
    // call the Supervisor (which holds the Hub) — that cycle is why the queue
    // used to live client-side. The Hub now makes the drain decision under its
    // lock and hands each ready prompt over this channel; we send it to the
    // agent here, off the lock. Wired before any client connects.
    let (dispatch_tx, dispatch_rx) = mpsc::channel::<DispatchReq>(1_024);
    hub.set_dispatch_tx(dispatch_tx);

    // Honor agent `ScheduleWakeup`s: fires a wake-prompt (via the same dispatch
    // path) at the scheduled time. Without this, an ACP-driven agent's scheduled
    // self-checks never fire and get consumed by the next user turn instead.
    let (sched_tx, sched_rx) = mpsc::channel::<crate::scheduler::ScheduleCmd>(1_024);
    hub.set_scheduler_tx(sched_tx);
    runtime_health.set_scheduler(true);
    let scheduler_health = Arc::clone(&runtime_health);
    let scheduler_hub = hub.clone();
    let scheduler_task = tokio::spawn(async move {
        crate::scheduler::run_scheduler(scheduler_hub, sched_rx).await;
        scheduler_health.set_scheduler(false);
        tracing::error!("scheduler exited unexpectedly");
    });
    // Re-arm wakeups that were pending across this restart; any already overdue
    // fire immediately (catch-up for the downtime).
    if let Some(store) = store.as_ref() {
        match store.load_wakeups().await {
            Ok(ws) => {
                let n = ws.len();
                for (sid, fire_at_ms, prompt) in ws {
                    hub.rearm_wakeup(&sid, fire_at_ms, prompt);
                }
                if n > 0 {
                    tracing::info!(rearmed = n, "re-armed persisted scheduled wakeups");
                }
            }
            Err(e) => tracing::warn!(error = %e, "loading scheduled wakeups (skipping re-arm)"),
        }
    }
    // Re-arm user-scheduled DRAFTS across the restart. These persist inside the
    // restored sessions' drafts jsonb (not a separate table), so re-arm scans the
    // now-restored in-memory sessions. An overdue one fires immediately (catch-up).
    hub.rearm_scheduled_drafts();

    // Machine WebSockets can only reconnect after Axum starts listening. Keep
    // this timer independent of the request task: real worker snapshots remove
    // their sessions from the reconciliation set, while the remainder become
    // genuine interruptions after the Machine's maximum reconnect delay plus
    // a bounded snapshot-settlement window. Interrupted turns stay stopped
    // until the user submits work.
    let runtime_reconciliation_task = {
        let hub = hub.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = tokio::time::sleep(RUNTIME_RECONCILIATION_GRACE) => {
                    let interrupted = hub.finalize_runtime_reconciliation();
                    if !interrupted.is_empty() {
                        tracing::warn!(
                            count = interrupted.len(),
                            grace_seconds = RUNTIME_RECONCILIATION_GRACE.as_secs(),
                            "runtime reconciliation grace expired without detached owners"
                        );
                    }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        tracing::info!("runtime reconciliation cancelled by shutdown");
                    }
                }
            }
        })
    };

    tracing::info!(
        workspace = %args.workspace_root.display(),
        data_dir = %args.data_dir.display(),
        plugin_dir = %plugin_dir.root().display(),
        plugin_storage = ?plugin_storage.kind(),
        "cowboy serving",
    );

    // Start only after runtime/storage setup. The observer owns no install
    // authority, stops on the HTTP signal and drains before storage teardown.
    let catalog_watcher = crate::plugin_catalog::spawn_catalog_watcher(
        Arc::clone(&plugin_catalog),
        plugin_storage.clone(),
        Arc::clone(&provider_catalog),
        shutdown_rx.clone(),
    );
    let state = Arc::new(AppState {
        service_id,
        service_config_path,
        service_config,
        hub,
        supervisor,
        store,
        persistence_health,
        shutdown: shutdown_rx,
        runtime_health,
        runtime_router,
        machine_control,
        machine_snapshots,
        session_maintenance: session_maintenance::Coordinator::default(),
        plugin_catalog,
        plugin_storage,
        provider_catalog,
        provider_auth,
        provider_auth_sync: provider_auth_sync::Coordinator::default(),
        provider_auth_executors: parking_lot::Mutex::new(HashMap::new()),
        plugin_uninstall_plans: parking_lot::Mutex::new(HashMap::new()),
        plugin_resolution_plans: plugin_uninstall::resolution::ResolutionPlans::default(),
        telemetry_resolution_plans: Arc::default(),
        telemetry_recovery_plans: Arc::default(),
        telemetry_binding_plans: Arc::default(),
        telemetry_binding_fence,
        telemetry_writer_admission,
        plugin_lifecycle_fences,
        session_create_replays: Arc::default(),
        desired_machine_components,
        component_convergence,
        convergence_freeze,
        desired_machine_plugins,
        web_roots: web_roots::WebRoots::new(args.web_root.clone()),
        web_root: args.web_root,
        usage,
        diff_snapshots: DiffSnapshotCache::default(),
        file_cursors: code_reads::file_pages::PageCursors::default(),
        code_cache,
        code_buffers: Arc::default(),
        code_navigation_admission: args.code_navigation_admission
            == crate::cli::CodeNavigationAdmission::Candidate,
        project_placement: crate::project_placement::Store::new(
            &args.data_dir,
            args.execution_runtime_machine,
        )?,
        execution_preparations: parking_lot::Mutex::default(),
        workspace_preparations: parking_lot::Mutex::default(),
        execution_readiness: Default::default(),
        lost_turns: Default::default(),
        execution_closures: Arc::default(),
        managed_calls: Arc::default(),
        memory_relay: memory_relay::Relay::default(),
        colocated_machines: args.colocated_machines.iter().cloned().collect(),
        zed_adapter_socket: args.zed_adapter_socket,
        observability,
        web_push,
        notifications: notifications::Presence::default(),
        public_origins: Arc::new(crate::product_auth::load_public_origins()),
        product_auth_enabled: args.product_auth_enabled,
        product_authentication,
        oidc_transactions: Arc::new(crate::oidc::OidcTransactions::default()),
        oidc_native_handoffs: Arc::new(crate::oidc::NativeHandoffs::default()),
        device_authorizations: Arc::new(crate::client_auth::DeviceAuthorizations::default()),
        device_access: Arc::new(crate::client_auth::DeviceAccessSessions::default()),
        capacity_runtime: Arc::new(ActiveCapacityRuntime::default()),
    });
    let web_push_task = tokio::spawn(notifications::run(
        Arc::clone(&state),
        state.shutdown.clone(),
    ));
    state.runtime_health.set_dispatcher(true);
    let dispatcher_health = Arc::clone(&state.runtime_health);
    let dispatcher_state = Arc::clone(&state);
    let dispatcher_shutdown = state.shutdown.clone();
    let dispatcher_exit_state = dispatcher_shutdown.clone();
    let mut dispatcher_task = tokio::spawn(async move {
        run_dispatcher(dispatcher_state, dispatch_rx, dispatcher_shutdown).await;
        dispatcher_health.set_dispatcher(false);
        if *dispatcher_exit_state.borrow() {
            tracing::info!("dispatcher stopped after shutdown");
        } else {
            tracing::error!("dispatcher exited while Cowboy was still serving");
        }
    });

    let runtime_router = Arc::clone(&state.runtime_router);
    let result = serve_axum(args.bind, args.data_dir.clone(), state, shutdown_tx).await;
    // Persistence closes below; events consumed after this point would be
    // acknowledged but never stored.
    runtime_router.disconnect_all();
    if let Err(error) = catalog_watcher.shutdown().await {
        tracing::error!(%error, "Catalog observer failed during shutdown");
    }
    scheduler_task.abort();
    reset_task.abort();
    runtime_reconciliation_task.abort();
    web_push_task.abort();
    match tokio::time::timeout(std::time::Duration::from_secs(5), &mut dispatcher_task).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::error!(%error, "dispatcher task failed during shutdown"),
        Err(_) => {
            tracing::warn!(
                "dispatcher still returning undelivered prompts; keeping persistence alive"
            );
            if let Err(error) = dispatcher_task.await {
                tracing::error!(%error, "dispatcher failed while finishing shutdown retention");
            }
        }
    }
    if let Some(task) = purge_task {
        task.abort();
    }
    if let Some(task) = machine_presence_task {
        task.abort();
    }
    telemetry_shutdown.drain().await;
    let _ = store_shutdown_tx.send(true);
    if let Some(task) = writer_task {
        match tokio::time::timeout(std::time::Duration::from_secs(10), task).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!(%error, "store writer task failed during shutdown"),
            Err(_) => tracing::error!("store writer did not drain within shutdown deadline"),
        }
    }
    log_guard.finish(result)
}

/// How often the Controller compares every connected Machine with the desired
/// component set and re-reads the manifest behind it.
const MACHINE_CONVERGENCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// A freshly connected Machine is already reconciling the automatic set its
/// welcome carried. Convergence waits this long before adding its own dispatch,
/// so the same generation is never activated twice in a row.
const MACHINE_CONVERGENCE_CONNECTION_GRACE: std::time::Duration =
    std::time::Duration::from_secs(120);

/// Resolve a subagent or Call anchor this writer lifetime does not know yet
/// through the durable table (scanning a session's pre-table rows once), so a
/// restarted or evicting writer replaces the original row instead of starting
/// a second one. `Ok(None)` means the key is new.
async fn resolve_durable_anchor(
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

async fn flush_event_batch(
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
    // These anchors were never committed; do not treat them as durable.
    for anchor in &new_anchors {
        reducer.forget_anchor(&anchor.session_id, &anchor.key);
    }
    false
}

const fn lifecycle_incident_recovery_outcome(status: Status) -> Option<&'static str> {
    match status {
        // An authoritative worker snapshot projects an in-flight turn as Busy.
        // It proves that the session is live just as strongly as the idle
        // Running projection, so a prior crash/interruption is no longer active.
        Status::Busy => Some("session_busy"),
        Status::Running => Some("session_running"),
        _ => None,
    }
}

fn is_context_window_rejection(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    detail.contains("maximum context length")
        || detail.contains("prompt is too long")
        || (detail.contains("context window")
            && (detail.contains("exceed")
                || detail.contains("full")
                || detail.contains("limit reached")))
}

fn is_empty_stream_failure(detail: &str) -> bool {
    detail
        .to_ascii_lowercase()
        .contains("stream ended without receiving any events")
}

fn classify_session_error(detail: &str) -> &'static str {
    match classify_crash_detail(Some(detail)) {
        "runtime_failure" => "session_command_error",
        classification => classification,
    }
}

fn session_error_severity(classification: &str) -> &'static str {
    match classification {
        "worker_startup_failure" | "process_exit" | "resource_exhaustion" | "runtime_failure" => {
            "critical"
        }
        _ => "error",
    }
}

fn classify_interruption_detail(detail: Option<&str>) -> &'static str {
    let detail = detail.unwrap_or_default().to_ascii_lowercase();
    if detail.contains("deploy")
        || detail.contains("shutdown")
        || detail.contains("controller restart")
    {
        "expected_interruption"
    } else {
        "runtime_interruption"
    }
}

#[cfg(test)]
mod incident_classification_tests;

#[cfg(test)]
mod store_writer_tests;

/// Retention (days) for soft-deleted sessions before the sweeper hard-deletes
/// them + their events.
const PURGE_RETENTION_DAYS: i64 = 3;
const PROVIDER_USAGE_RETENTION_DAYS: i32 = 30;

/// Periodically hard-delete sessions soft-deleted past [`PURGE_RETENTION_DAYS`],
/// reclaiming their event storage. `interval` fires immediately on the first
/// tick (clears any backlog accrued while the daemon was down), then every 6h.
/// Errors are logged, never fatal.
async fn run_purge_sweeper(store: Store) {
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
    }
}

/// First retention pass after start, once Machines have reconnected and every
/// restored session's pinned generation is known; then every
/// `plugins.generation_retention_interval` (re-read live before each wait).
const GENERATION_RETENTION_DELAY: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// How often the retention task compares each Device's reported disk with its
/// declared `disk.low_watermark` between scheduled passes.
const URGENT_RETENTION_POLL: std::time::Duration = std::time::Duration::from_secs(60);

fn force_cancel_grace(state: &AppState, session_id: &str) -> std::time::Duration {
    if state
        .hub
        .session_info(session_id)
        .is_some_and(|info| info.meta.execution_binding.is_some())
    {
        EXECUTION_FORCE_CANCEL_GRACE
    } else {
        FORCE_CANCEL_GRACE
    }
}

fn force_cancel_with_watchdog(state: &AppState, session_id: &str) -> Result<(), String> {
    let Some(cancelled_revision @ (Status::Busy | Status::Starting, _)) =
        state.hub.status_revision(session_id)
    else {
        return Ok(());
    };
    let grace = force_cancel_grace(state, session_id);
    state.supervisor.request_cancel_for_recycle(session_id)?;
    let hub = state.hub.clone();
    let supervisor = Arc::clone(&state.supervisor);
    let session_id = session_id.to_owned();
    tokio::spawn(async move {
        tokio::time::sleep(grace).await;
        if !hub.set_status_if_revision(
            &session_id,
            Some(cancelled_revision),
            Status::Interrupted,
            Some("force cancel timed out; recycling session worker".to_owned()),
        ) {
            return;
        }
        tracing::error!(
            session = %session_id,
            grace_seconds = grace.as_secs(),
            "force cancel did not end turn; recycling only this session worker"
        );
        // The interrupted edge frees Hub's in-flight guard, but its automatic
        // drain waits for the replacement worker to become Running.
        if let Err(error) = supervisor.recycle_session(&session_id) {
            tracing::error!(session = %session_id, %error, "force-cancel recycle failed");
            hub.set_status(&session_id, Status::Crashed, Some(error));
        }
    });
    Ok(())
}

/// Reboot convergence: an explicit send returned only because a Machine or
/// resource was not ready yet must be delivered once it is, even when the
/// queue is paused or the session crashed while its host rebooted.
fn retain_explicit_intent(hub: &Hub, session_id: &str, error: &str) {
    if error.starts_with(session_maintenance::WAIT_PREFIX) {
        hub.set_explicit_wait(session_id, true);
    } else if is_transient_machine_disconnect(error) {
        session_maintenance::wait(
            hub,
            session_id,
            session_maintenance::MACHINE_CONNECTION_WAIT,
        );
        hub.set_explicit_wait(session_id, true);
    }
}

#[cfg(test)]
mod dispatcher_failure_tests;

/// Why a Provider lifecycle fence refuses session runtime admission, or `None`
/// when the fence (an install or reload in progress) leaves sessions usable.
/// `NeedsReconcile` covers an install or uninstall whose Machine outcome is
/// unknown, so it must not be reported as an uninstall.
const fn provider_fence_refusal(fence: PluginFenceState) -> Option<&'static str> {
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

/// Build the ACP prompt blocks for a queued message: parse the stored content
/// blocks, or fall back to a single text block. Mirrors the `Inbound::Prompt`
/// handler's logic. Returns `None` for a genuinely empty prompt.
fn build_prompt_blocks(text: &str, content: &[serde_json::Value]) -> Option<Vec<ContentBlock>> {
    if content.is_empty() {
        if text.is_empty() {
            return None;
        }
        return Some(vec![ContentBlock::from(text.to_owned())]);
    }
    let blocks: Vec<ContentBlock> = content
        .iter()
        .filter_map(
            |v| match serde_json::from_value::<ContentBlock>(v.clone()) {
                Ok(b) => Some(b),
                Err(e) => {
                    tracing::warn!(error = %e, "skipping unparseable queued content block");
                    None
                }
            },
        )
        .collect();
    if blocks.is_empty() {
        None
    } else {
        Some(blocks)
    }
}

#[derive(Clone)]
struct ProductAuthState {
    hub: Hub,
    store: Option<Store>,
    rate_limits: Arc<crate::product_auth::AuthRateLimiter>,
    public_origins: Arc<Vec<String>>,
    runtime_health: Option<Arc<RuntimeHealth>>,
    persistence_health: Option<Arc<PersistenceHealth>>,
    runtime_router: Option<Arc<RuntimeRouter>>,
    plugin_catalog: Option<Arc<crate::plugin_catalog::PluginCatalog>>,
    plugin_storage: Option<crate::plugin_storage::PluginStorage>,
    provider_catalog: Option<Arc<crate::provider_catalog::ProviderCatalog>>,
    passkeys: Arc<crate::passkey::PasskeyCeremonies>,
    setup: Arc<crate::admin::AdminSetupState>,
    setup_lock: Arc<tokio::sync::Mutex<()>>,
    product_auth_enabled: bool,
    product_authentication: Arc<crate::auth_plugins::ProductAuthentication>,
    oidc_transactions: Arc<crate::oidc::OidcTransactions>,
    oidc_native_handoffs: Arc<crate::oidc::NativeHandoffs>,
    device_authorizations: Arc<crate::client_auth::DeviceAuthorizations>,
    device_access: Arc<crate::client_auth::DeviceAccessSessions>,
    capacity_runtime: Arc<ActiveCapacityRuntime>,
}

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    account: String,
    password: String,
    #[allow(dead_code)]
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    account: String,
    password: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum LogoutScope {
    #[default]
    Current,
    Provider,
    All,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogoutRequest {
    #[serde(default)]
    scope: LogoutScope,
    #[serde(default)]
    provider_logout: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OidcBackchannelLogoutRequest {
    logout_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AutomationCredentialRequest {
    name: String,
    public_key: String,
    ttl_seconds: Option<i64>,
    #[serde(default)]
    scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActiveClientReleaseQuery {
    fencing_token: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OidcCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    #[serde(default)]
    user: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OidcStartQuery {
    client: Option<String>,
    code_challenge: Option<String>,
    handoff_challenge: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OidcNativeExchangeRequest {
    code: String,
    code_verifier: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OidcNativePollRequest {
    handoff_token: String,
    code_verifier: String,
}

#[derive(Debug, Serialize)]
struct OidcNativePollPending {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct OidcNativeEventStatus {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct PasskeyExternalEventStatus {
    status: &'static str,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AdminCreateUserRequest {
    account: String,
    password: String,
    role: Option<crate::admin::AdminRole>,
}

#[derive(Debug, Deserialize)]
struct AdminSetPasswordRequest {
    password: String,
}

#[derive(Debug, Serialize)]
struct ProductMe {
    account: String,
    user_id: Option<String>,
    role: crate::admin::AdminRole,
    #[serde(default)]
    primary_auth_method: Option<String>,
    #[serde(default)]
    passkey_count: u32,
    #[serde(default)]
    passkey_reauth_enabled: bool,
    #[serde(default)]
    passkey_reauth_required: bool,
    #[serde(default)]
    passkey_reauth_after_ms: i64,
    #[serde(default)]
    passkey_reauth_due_at_ms: Option<i64>,
    #[serde(default)]
    passkey_reauth_warn_at_ms: Option<i64>,
    #[serde(default)]
    primary_reauth_due_at_ms: Option<i64>,
    #[serde(default)]
    primary_reauth_warn_at_ms: Option<i64>,
    #[serde(default)]
    session_idle_due_at_ms: Option<i64>,
    #[serde(default)]
    session_expires_at_ms: Option<i64>,
    #[serde(default)]
    session_server_now_ms: Option<i64>,
    #[serde(default)]
    session_reauth_kind: Option<ProductSessionReauthKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProductSessionReauthKind {
    Passkey,
    Primary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProductSessionDeadlines {
    idle_due_at_ms: Option<i64>,
    passkey_due_at_ms: Option<i64>,
    passkey_warn_at_ms: Option<i64>,
    primary_due_at_ms: i64,
    primary_warn_at_ms: i64,
    required_kind: Option<ProductSessionReauthKind>,
}

#[derive(Debug, Deserialize)]
struct CreateApiTokenRequest {
    name: String,
    ttl_seconds: Option<i64>,
}

#[derive(Serialize)]
struct AdminOverview {
    healthy: bool,
    persistence: &'static str,
    backend: &'static str,
    sessions_live: usize,
    sessions_deleted: i64,
    events_rows: i64,
    daemon_rss_bytes: u64,
    runtime_workers: usize,
    runtime_busy_workers: usize,
    registration: crate::admin::RegistrationPolicyView,
}

#[derive(Serialize)]
struct AdminSessionRow {
    id: String,
    title: String,
    provider: String,
    machine_id: String,
    status: Status,
}

fn durable_store(store: &Option<Store>) -> Option<&Store> {
    store.as_ref()
}

fn missing_store() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "product accounts require a durable store",
    )
        .into_response()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RouteAuth {
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
struct ProductRequestAuth<'a> {
    product_auth_enabled: bool,
    store: Option<&'a Store>,
    hub: &'a Hub,
    device_access: &'a crate::client_auth::DeviceAccessSessions,
    product_authentication: &'a crate::auth_plugins::ProductAuthentication,
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
struct AuthenticatedProductRequest {
    principal: ProductPrincipal,
    cookie_session: Option<crate::store::ProductUserSession>,
    device_identity: Option<crate::client_auth::DeviceAccessIdentity>,
    permissions: Option<crate::core::ProductPermissionObservation>,
}

impl AuthenticatedProductRequest {
    /// Complete authentication by atomically observing current permissions.
    /// Handlers may share this observation, but must never recapture it.
    fn observe_permissions(mut self, hub: &Hub) -> Result<Self, ()> {
        let permissions = hub
            .observe_product_permissions(&self.principal.username)
            .ok_or(())?;
        self.principal.role = permissions.role();
        self.permissions = Some(permissions);
        Ok(self)
    }
}

const OIDC_NATIVE_EVENTS_HANDSHAKE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(10);
const OIDC_NATIVE_EVENTS_TTL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const OIDC_NATIVE_EVENTS_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(25);

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
const NATIVE_COMPLETE_SCRIPT: &str = r#"
try { window.close(); } catch (error) { /* Recovered by the link below. */ }
setTimeout(function () { document.body.className = "stranded"; }, 700);
"#;

static NATIVE_COMPLETE_PAGE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
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

static NATIVE_COMPLETE_CSP: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let digest = base64::engine::general_purpose::STANDARD
        .encode(Sha256::digest(NATIVE_COMPLETE_SCRIPT.as_bytes()));
    format!(
        "default-src 'none'; style-src 'unsafe-inline'; script-src 'sha256-{digest}'; \
         base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    )
});

const PASSKEY_MANAGEMENT_STEP_UP_MAX_AGE_MS: i64 = 5 * 60 * 1_000;

struct ProductPasskeyAssertionResult {
    me: ProductMe,
    set_cookie: Option<String>,
}

const PASSKEY_EXTERNAL_EVENTS_HANDSHAKE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(10);
const PASSKEY_EXTERNAL_EVENTS_TTL: std::time::Duration =
    std::time::Duration::from_secs(crate::passkey::EXTERNAL_CEREMONY_TTL_SECS);
const PASSKEY_EXTERNAL_EVENTS_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(25);

const DEVICE_AUTH_EVENTS_HANDSHAKE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(10);
const DEVICE_AUTH_EVENTS_TTL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const DEVICE_AUTH_EVENTS_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(25);

fn no_store_json<T: Serialize>(status: StatusCode, value: T) -> Response {
    let mut response = (status, Json(value)).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CardeaDeviceExchangeRequest {
    name: String,
    public_key: String,
    access_token: String,
    assertion: String,
}

async fn serve_axum(
    bind: std::net::SocketAddr,
    data_dir: PathBuf,
    state: Arc<AppState>,
    shutdown_tx: watch::Sender<bool>,
) -> anyhow::Result<()> {
    session_maintenance::start(&state);
    session_reclaim::listen_for_wakes(&state);
    memory_reclaim::start(&state);
    let secure_transport = Arc::new(secure_transport::SecureTransport::new(
        state
            .store
            .clone()
            .context("Device authentication requires durable storage")?,
        state.public_origins.as_ref().clone(),
    )?);
    execution::sessions::start_recovery(&state);
    workspace_preparation::start_recovery(&state);
    managed_calls::Coordinator::start(&state);
    if let Some(store) = state.store.clone() {
        let hub = state.hub.clone();
        tokio::spawn(async move { background_tasks::repair_call_anchors(&hub, &store).await });
    }
    let setup = Arc::new(crate::admin::AdminSetupState::new(data_dir.clone()));
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
    let auth_state = ProductAuthState {
        hub: state.hub.clone(),
        store: state.store.clone(),
        rate_limits: Arc::new(crate::product_auth::AuthRateLimiter::default()),
        public_origins: state.public_origins.clone(),
        runtime_health: Some(state.runtime_health.clone()),
        persistence_health: state.persistence_health.clone(),
        runtime_router: Some(state.runtime_router.clone()),
        plugin_catalog: Some(state.plugin_catalog.clone()),
        plugin_storage: Some(state.plugin_storage.clone()),
        provider_catalog: Some(state.provider_catalog.clone()),
        passkeys: Arc::new(crate::passkey::PasskeyCeremonies::default()),
        setup,
        setup_lock: Arc::new(tokio::sync::Mutex::new(())),
        product_auth_enabled: state.product_auth_enabled,
        product_authentication: state.product_authentication.clone(),
        oidc_transactions: state.oidc_transactions.clone(),
        oidc_native_handoffs: state.oidc_native_handoffs.clone(),
        device_authorizations: state.device_authorizations.clone(),
        device_access: state.device_access.clone(),
        capacity_runtime: state.capacity_runtime.clone(),
    };

    let app = Router::new()
        .merge(cardea_authorization::routes())
        .merge(code_buffers::routes(&state))
        .merge(telemetry_binding::resolution::surface::routes())
        .merge(telemetry_binding::recovery::surface::routes())
        .merge(telemetry_binding::surface::routes())
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .route("/api/metrics", get(api_metrics))
        .route("/api/sync/dataset", get(sync_dataset::get_dataset))
        .route("/api/drafts", get(draft_documents::list))
        .route("/api/preferences/{scope}", get(preferences::read))
        .route("/api/provider-accounts", get(provider_accounts::read).put(provider_accounts::update).layer(DefaultBodyLimit::max(16 * 1024)))
        .route("/api/provider-accounts/usage", get(provider_accounts::usage::list))
        .route("/api/provider-accounts/{account_id}/usage", get(provider_accounts::usage::read).post(provider_accounts::usage::refresh))
        .route("/api/provider-accounts/{account_id}/revocation", get(provider_accounts::lifecycle::http_status).post(provider_accounts::lifecycle::http_start).layer(DefaultBodyLimit::max(16 * 1024)))
        .route("/api/provider-accounts/{account_id}/revocation/reconcile", post(provider_accounts::lifecycle::http_resume).layer(DefaultBodyLimit::max(16 * 1024)))
        .route("/api/provider-accounts/login-operations/{operation}", get(provider_accounts::login::http_receipt).delete(provider_accounts::login::http_cancel))
        .route("/api/provider-accounts/{account_id}/logins", get(provider_accounts::login::http_logins))
        .route("/api/provider-accounts/{account_id}/logins/{request}", get(provider_accounts::login::http_continue).delete(provider_accounts::login::http_continue_cancel))
        .route("/api/provider-accounts/{account_id}/login", get(provider_accounts::login::options).post(provider_accounts::login::start).layer(DefaultBodyLimit::max(16 * 1024)))
        .route("/api/preferences", put(preferences::update).layer(DefaultBodyLimit::max(64 * 1024)))
        .route("/api/drafts/mutations", post(draft_documents::mutate).layer(DefaultBodyLimit::max(12 * 1024 * 1024)))
        .route("/api/drafts/{id}", get(draft_documents::read))
        .route("/api/drafts/{id}/history", get(draft_documents::history))
        .route("/api/drafts/{id}/history/index", get(draft_documents::history_index))
        .route("/api/drafts/{id}/history/{revision}", get(draft_documents::history_snapshot))
        .route("/api/sessions/{id}/draft-copies/{cmid}", delete(draft_documents::undo_copy).layer(DefaultBodyLimit::max(12 * 1024 * 1024)))
        .route(
            "/api/observability/batches",
            post(api_observability_batch).layer(DefaultBodyLimit::max(256 * 1024)),
        )
        .route(
            "/api/observability/incidents",
            get(api_observability_incidents),
        )
        .route(
            "/api/telemetry/v1/{signal}",
            post(api_otlp).layer(DefaultBodyLimit::max(crate::otlp::MAX_BYTES)),
        )
        .route("/api/logs", get(api_diagnostic_logs))
        .route("/api/logs/{id}", get(api_diagnostic_log_detail))
        .route("/api/usage", get(api_usage).post(api_usage_refresh))
        .route("/api/usage/executors", get(api_usage_executors))
        .route("/api/usage/{provider}/executor", axum::routing::put(api_usage_executor_update))
        .route("/api/usage/{provider}/activity", get(api_usage_activity))
        .route("/api/usage/{provider}", post(api_usage_provider_refresh))
        .route("/api/usage/logs", get(api_usage_logs))
        .route("/api/usage/{provider}/reset", post(api_provider_reset))
        .route(
            "/api/usage/{provider}/reset/schedule",
            put(api_provider_reset_schedule).delete(api_provider_reset_cancel),
        )
        .route("/metrics", get(prometheus_metrics))
        .route("/api/workspaces", get(api_workspaces))
        .route("/api/project-policies", get(projects::policies))
        .route("/api/project-placements", get(projects::placements))
        .route("/api/machines/{id}/projects", get(projects::list).post(projects::request))
        .route("/api/machines/{id}/project-policy", put(projects::update_policy))
        .route("/api/web-push/config", get(api_web_push_config))
        .route(
            "/api/notifications/presence",
            put(notifications::renew_presence).delete(notifications::release_presence),
        )
        .route(
            "/api/web-push/subscription",
            put(notifications::subscribe).delete(notifications::unsubscribe),
        )
        .route("/api/providers", get(api_providers))
        .route("/api/plugins", get(api_plugins))
        .route(
            "/api/plugins/catalog/refresh",
            post(api_plugin_catalog_refresh),
        )
        .route(
            "/api/plugins/{id}/call",
            post(api_plugin_call).layer(DefaultBodyLimit::max(
                crate::plugin_process::MAX_PLUGIN_COMMAND_INPUT_BYTES,
            )),
        )
        .route(
            "/api/providers/catalog/refresh",
            post(api_provider_catalog_refresh),
        )
        .route(
            "/api/providers/{id}/auth",
            post(api_provider_auth_commit).delete(api_provider_auth_logout),
        )
        .route(
            "/api/providers/{id}/auth/start",
            post(api_provider_auth_start),
        )
        .route(
            "/api/providers/{id}/auth/{request_id}",
            get(api_provider_auth_events)
                .post(api_provider_auth_submit)
                .delete(api_provider_auth_cancel),
        )
        .route(
            "/api/plugins/{id}/auth",
            post(api_provider_auth_commit).delete(api_provider_auth_logout),
        )
        .route(
            "/api/plugins/{id}/auth/start",
            post(api_provider_auth_start),
        )
        .route(
            "/api/plugins/{id}/auth/{request_id}",
            get(api_provider_auth_events)
                .post(api_provider_auth_submit)
                .delete(api_provider_auth_cancel),
        )
        .route("/api/machines", get(api_machines))
        .route(
            "/api/machines/enrollment",
            post(api_machine_create_enrollment).delete(api_machine_cancel_enrollment),
        )
        .route("/api/machines/{id}/events", get(api_machine_events))
        .route(
            "/api/machines/{id}/deployment-health",
            get(api_machine_deployment_health),
        )
        .route(
            "/api/machines/{id}/durable-state",
            get(api_machine_durable_state),
        )
        // Protocol-v4/web compatibility adapter. Delete after every supported
        // client consumes protocol-v5 Plugin inventory from `/plugins`.
        .route(
            "/api/machines/{id}/providers",
            get(api_machine_provider_inventory_compat),
        )
        .route("/api/machines/{id}/plugins", get(api_machine_plugins))
        .route(
            "/api/machines/{id}/providers/{provider_id}",
            post(api_machine_plugin_install),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}",
            post(api_machine_plugin_install),
        )
        .route(
            "/api/machines/{id}/providers/{provider_id}/uninstall-plan",
            post(api_machine_plugin_uninstall_plan),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/uninstall-plan",
            post(api_machine_plugin_uninstall_plan),
        )
        .route(
            "/api/machines/{id}/providers/{provider_id}/uninstall",
            post(api_machine_plugin_uninstall),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/uninstall",
            post(api_machine_plugin_uninstall),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/operations",
            get(api_machine_plugin_operations),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/installation-operations",
            get(plugin_install::api_machine_plugin_install_operations),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/lifecycle-history",
            get(plugin_history::get_history),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/machine-receipt",
            get(api_machine_plugin_operation_receipt),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/recovery-assessment",
            get(api_machine_plugin_recovery_assessment),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/resolution-plan",
            post(plugin_uninstall::resolution::api_resolution_plan),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/resolve",
            post(plugin_uninstall::resolution::api_resolve),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/resolution",
            get(plugin_uninstall::resolution::api_resolution_receipt),
        )
        .route("/api/machines/{id}/refresh", post(api_machine_refresh))
        .route(
            "/api/machines/{id}/components/reconcile",
            post(api_machine_reconcile),
        )
        .route(
            "/api/machines/{id}/components/reconcile-one",
            post(api_machine_reconcile_one),
        )
        .route(
            "/api/machines/{id}/components/update-npm",
            post(api_machine_update_npm),
        )
        .route("/api/machines/{id}/revoke", post(api_machine_revoke))
        .route("/api/machine/service", get(api_machine_service))
        .route("/api/machine/enroll", post(api_machine_enroll))
        .route("/api/machine/connect", any(machine_ws_upgrade))
        .route(crate::machine_lanes::LANE_PATH, any(machine_lanes::upgrade))
        .route(
            "/api/sessions",
            post(api_new_session).layer(middleware::from_fn_with_state(
                Arc::clone(&state.session_create_replays),
                session_create_replay::replay,
            )),
        )
        .route("/api/sessions/{id}/calls", get(managed_calls::list))
        .route("/api/sessions/{id}/calls/dismiss", post(managed_calls::dismiss))
        .route(
            "/api/sessions/{id}/calls/approval",
            post(managed_calls::decide_approval),
        )
        .route(
            "/api/sessions/{id}/tools",
            get(agent_tools::session).put(agent_tools::configure_session),
        )
        .route("/api/agent-tools", get(agent_tools::list))
        .route("/api/folders", get(folder_defaults::list).post(folder_defaults::create))
        .route("/api/folders/{id}/defaults", get(folder_defaults::get_defaults).patch(folder_defaults::update))
        .route("/api/agent-tools/{agent}", put(agent_tools::configure_agent))
        .route("/api/sessions/{id}/calls/{call_id}", get(managed_calls::inspect))
        .route(
            "/api/sessions/{id}/calls/{call_id}/cancel",
            post(managed_calls::cancel),
        )
        .route(
            "/api/execution-sessions",
            post(execution::sessions::create).layer(middleware::from_fn_with_state(
                Arc::clone(&state.session_create_replays),
                session_create_replay::replay,
            )),
        )
        .route("/api/execution-environments", get(execution::sessions::availability))
        .route(
            "/api/sessions/reconcile-project",
            post(api_reconcile_project_sessions),
        )
        .route("/api/sessions/{id}/files", get(api_search_files))
        .route("/api/code/sessions/{id}/tree", get(api_file_tree))
        .route("/api/code/sessions/{id}/search", get(api_code_search))
        .route("/api/code/sessions/{id}/manifest", get(api_code_manifest))
        .route("/api/code/sessions/{id}/changes", get(api_code_changes))
        .route("/api/code/sessions/{id}/extensions", get(workspace_extensions::inventory))
        .route("/api/code/sessions/{id}/extensions/resources", get(workspace_extensions::resources))
        .route(
            "/api/code/sessions/{id}/repository",
            get(api_code_repository),
        )
        .route("/api/code/sessions/{id}/commit", get(api_code_commit))
        .route(
            "/api/code/sessions/{id}/commit-diff",
            get(api_code_commit_diff),
        )
        .route("/api/code/sessions/{id}/diff", get(api_code_diff))
        .route("/api/code/sessions/{id}/file", get(api_code_file))
        .route("/api/code/sessions/{id}/file-raw", get(api_code_file_raw))
        .route("/api/code/sessions/{id}/language", get(api_code_language))
        .route(
            "/api/code/sessions/{id}/intelligence/hover",
            get(api_code_hover),
        )
        .route(
            "/api/code/sessions/{id}/intelligence/navigation",
            get(api_code_navigation),
        )
        .route(
            "/api/code/sessions/{id}/intelligence/outline",
            get(api_code_outline),
        )
        .route(
            "/api/code/sessions/{id}/buffer",
            put(api_code_buffer_open).delete(api_code_buffer_close),
        )
        .route("/api/sessions/{id}/info", get(api_session_info))
        .route(
            "/api/sessions/{id}/reload",
            get(api_session_reload_plan).post(api_session_reload).put(session_provider_updates::configure),
        )
        .route("/api/sessions/{id}/hibernate", post(api_session_hibernate))
        .route("/api/sessions/{id}/review-fetch", post(api_review_fetch))
        .route("/api/session-maintenance", get(session_maintenance::inventory))
        .route("/api/machines/{id}/maintenance", put(session_maintenance::configure))
        .route(
            "/api/sessions/{id}/cache-protection",
            get(api_session_cache_protection),
        )
        .route("/api/sessions/{id}/question-pages", get(api_question_pages))
        .route(
            "/api/sessions/{id}/question-pages/{page_id}",
            get(api_question_page),
        )
        .route("/api/sessions/{id}/bootstrap", get(api_session_bootstrap))
        .route(
            "/api/sessions/{id}/subagents/lookup",
            post(background_tasks::lookup),
        )
        .route(
            "/api/sessions/{id}/tasks/acknowledge",
            post(notifications::acknowledge),
        )
        .route("/api/sessions/{id}/prompt", post(api_session_prompt))
        .route("/api/sessions/{id}/context", get(session_context::get).put(session_context::set))
        .route("/api/sessions/{id}/context/send", post(session_context::send).layer(DefaultBodyLimit::max(64 * 1024 * 1024)))
        .route("/api/sessions/{id}/fork", post(session_context::fork))
        .route(
            "/api/sessions/{id}/duplicate",
            post(session_context::duplicate).layer(middleware::from_fn_with_state(
                Arc::clone(&state.session_create_replays),
                session_create_replay::replay,
            )),
        )
        .route("/api/sessions/{id}/config-revert", get(config_revert::get))
        .route(
            "/api/sessions/{id}/config-revert/restore",
            post(config_revert::restore),
        )
        .route("/api/sessions/{id}/delivery/{operation}", get(send_delivery::receipt))
        .route("/api/sessions/{id}/delivery-options", get(send_delivery::options))
        .route("/api/sessions/{id}/delivery-policy", put(send_delivery::set_policy))
        .route("/api/sessions/{id}/delivery-hold/release", post(send_delivery::release))
        .route("/api/history/{id}", get(api_history))
        .route("/api/artifacts/{name}", get(api_artifact))
        .route(
            "/plugin-artifacts/{digest}/{name}",
            get(plugin_release_artifact),
        )
        // Published URL compatibility for releases created before the Plugin
        // Catalog cutover. Delete after those immutable URLs age out.
        .route(
            "/provider-artifacts/{digest}/{name}",
            get(plugin_release_artifact),
        )
        .route("/ws", any(ws_upgrade))
        // Everything else: the separately deployed SPA, with index.html
        // fallback for client-side routes.
        .fallback(static_handler)
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state.clone(), enforce_product_api))
        .merge(product_auth_router(auth_state))
        .layer(middleware::from_fn_with_state(secure_transport, secure_transport::enforce))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(addr = %bind, "WS/HTTP listening");

    #[cfg(unix)]
    let local_operator = local_operator::start(&data_dir, Arc::clone(&state))?;

    // Converge signed automatic components and keep the Service-side Plugin
    // document fresh for as long as the Controller serves. A Controller with
    // neither configured simply finds nothing to do on each pass.
    let convergence_task = tokio::spawn(run_component_convergence(
        Arc::clone(&state),
        state.machine_snapshots.clone(),
        state.shutdown.clone(),
    ));

    let provider_update_task = tokio::spawn(session_provider_updates::run(Arc::clone(&state)));
    let generation_retention_task = tokio::spawn(run_generation_retention(Arc::clone(&state)));
    let adoption_state = Arc::clone(&state);
    let shared_authentication_task = tokio::spawn(async move {
        adopt_shared_provider_authentication(&adoption_state).await;
    });
    let trash_retention_task = tokio::spawn(session_trash::run_retention(Arc::clone(&state)));
    let account_recovery_task = tokio::spawn(provider_accounts::run_recovery(Arc::clone(&state)));
    let usage_broadcast_task = tokio::spawn(run_usage_broadcast(Arc::clone(&state)));
    let native_usage_task = tokio::spawn(provider_accounts::usage::run(Arc::clone(&state)));
    let code_buffers = Arc::clone(&state.code_buffers);
    let result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(shutdown_tx))
    .await
    .context("axum serve");
    #[cfg(unix)]
    local_operator.shutdown().await;
    convergence_task.abort();
    provider_update_task.abort();
    generation_retention_task.abort();
    shared_authentication_task.abort();
    trash_retention_task.abort();
    account_recovery_task.abort();
    usage_broadcast_task.abort();
    native_usage_task.abort();
    code_buffers.shutdown().await;
    result?;
    Ok(())
}

async fn shutdown_signal(shutdown: watch::Sender<bool>) {
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

pub(crate) fn init_tracing() {
    crate::logs::init_stderr();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebPushConfigResponse<'a> {
    application_server_key: &'a str,
}

/// Response body for `GET /version`.
#[derive(Debug, Serialize)]
struct VersionResponse {
    version: String,
}

/// Storage/runtime metrics for the Settings info panel — the capacity dashboard
/// for the unbounded growers (events) + the deleted-session purge backlog.
#[derive(Debug, Serialize)]
struct Metrics {
    /// postgres database size (bytes).
    db_bytes: i64,
    /// total rows in the events log (the unbounded grower).
    events_rows: i64,
    /// live (non-deleted) sessions.
    sessions_live: usize,
    /// sessions soft-deleted, awaiting the 3-day purge.
    sessions_deleted: i64,
    /// daemon resident memory (bytes), excluding agent subprocesses.
    daemon_rss_bytes: u64,
    /// lifetime high-water RSS for the daemon process (bytes).
    daemon_rss_peak_bytes: u64,
    /// Current memory charged to the complete Cowboy service cgroup (bytes).
    cgroup_memory_bytes: u64,
    /// Lifetime high-water memory for the complete service cgroup (bytes).
    cgroup_memory_peak_bytes: u64,
    persistence_pending: usize,
    persistence_pending_bytes: usize,
    persistence_dropped: u64,
    persistence_failed_batches: u64,
    persistence_last_error: Option<String>,
    runtime_connected: bool,
    runtime_workers: usize,
    runtime_busy_workers: usize,
    runtime_draining_workers: usize,
    runtime_handoff_workers: usize,
    runtime_pending_commands: usize,
    code_cache_bytes: u64,
    code_cache_hits: u64,
    code_cache_misses: u64,
    code_cache_evictions: u64,
    observability_pending: usize,
    observability_pending_bytes: usize,
    observability_duplicate_batches: u64,
    observability_failed_file_batches: u64,
    observability_failed_incident_batches: u64,
    observability_dropped_export_batches: u64,
    observability_accepted_batches: u64,
    observability_dropped_batches: u64,
    observability_failed_log_batches: u64,
    observability_failed_metric_batches: u64,
    observability_failed_trace_batches: u64,
    observability_rejected_export_items: u64,
    hub_session_count: usize,
    hub_hot_log_bytes: usize,
    hub_broadcast_last_bytes: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct DaemonMemory {
    rss_bytes: u64,
    rss_peak_bytes: u64,
}

#[cfg(test)]
mod daemon_memory_tests;

#[derive(Deserialize)]
struct OtlpQuery {
    batch_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageExecutorUpdate {
    machine_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageRefreshQuery {
    /// `false` starts the refresh and returns at once; every client follows
    /// it through the usage broadcast. Absent keeps the waiting contract that
    /// the operator CLI and older cached clients rely on.
    wait: Option<bool>,
}

#[derive(Default, Deserialize)]
struct UsageActivityQuery {
    window: Option<String>,
    model: Option<String>,
    agent: Option<String>,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
}

#[derive(Debug, Eq, PartialEq)]
struct UsageActivityFilter {
    window: String,
    from_ms: i64,
    to_ms: i64,
    models: Vec<String>,
    agents: Vec<String>,
}

#[cfg(test)]
mod usage_activity_filter_tests;

#[derive(Deserialize)]
struct ResetScheduleRequest {
    fire_at_ms: i64,
    confirm: String,
}

#[derive(Deserialize)]
struct ResetRequest {
    confirm: String,
    expected_credit_id: String,
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

#[derive(Debug, Default, Deserialize)]
struct DiagnosticLogsQuery {
    kind: Option<String>,
    severity: Option<String>,
    state: Option<String>,
    agent: Option<String>,
    window: Option<String>,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    session: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DiagnosticLogCursor {
    occurred_at_ms: i64,
    id: String,
}

#[derive(Debug, Serialize)]
struct DiagnosticLogPage {
    items: Vec<crate::store::DiagnosticLogSummary>,
    next_cursor: Option<String>,
}

#[cfg(test)]
mod diagnostic_log_query_tests;

#[cfg(test)]
mod trashed_session_command_tests;

/// One selectable working directory for the New Session dialog's dropdown.
#[derive(Debug, Serialize)]
struct Workspace {
    /// Sent to the daemon as `cwd` (absolute paths are honoured as-is).
    value: String,
    /// Short display name shown in the dropdown.
    label: String,
    /// Secondary line — the resolved absolute path or a description.
    help: String,
    /// Columbus registry id; absent for host-level roots.
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    /// Durable central work items projected onto this project.
    active_work_items: Vec<WorkspaceWorkItem>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct WorkspaceWorkItem {
    id: String,
    title: String,
    #[serde(default)]
    projects: Vec<String>,
    #[serde(default)]
    recipe: String,
    #[serde(default)]
    blocked: bool,
}

#[derive(Debug, Serialize)]
struct MachineDeploymentHealthResponse {
    connected: bool,
    status: String,
    active_acp_generation: Option<String>,
    workspace_revision: Option<String>,
    workspace_ids_sha256: Option<String>,
    workspace_owner: Option<crate::machine_protocol::projects::Owner>,
}

#[derive(Debug, Deserialize)]
struct MachineEnrollmentRequest {
    #[serde(default)]
    machine_id: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct MachineEnrollmentResponse {
    machine_id: String,
    display_name: String,
    token: String,
    expires_in_seconds: i64,
}

#[derive(Debug, Deserialize)]
struct MachineEnrollmentCancelRequest {
    token: String,
}

#[derive(Debug, Serialize)]
struct MachineCommandResponse {
    request_id: String,
}

const MACHINE_INVENTORY_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
const MACHINE_COMPONENT_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

#[cfg(test)]
mod colocated_permission_tests;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct ProviderAuthenticationExecutorIdentity {
    #[serde(skip_serializing_if = "Option::is_none")]
    machine_id: Option<String>,
    provider_id: String,
    provider_version: String,
    generation_digest: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginCallIdentity {
    version: Option<String>,
    digest: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginInstallRequest {
    operation_id: String,
    version: String,
    digest: String,
}

#[derive(Debug, Deserialize)]
struct PluginUninstallRequest {
    plan_id: String,
    #[serde(default)]
    confirm_active_sessions: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderAuthCommitRequest {
    /// Exact Provider-declared authentication method that produced this bundle.
    method: String,
    /// Provider-declared bundle keys mapped to standard-base64 credential values.
    values: BTreeMap<String, String>,
    #[serde(default)]
    account_label: Option<String>,
    #[serde(default)]
    expected_generation: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderAuthStartRequest {
    #[serde(default)]
    machine_id: Option<String>,
    method: String,
    provider_version: String,
    generation_digest: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderAuthSubmitRequest {
    code: String,
}

#[derive(Debug, Default, Serialize)]
struct ProviderDistributionOutcome {
    succeeded: usize,
    failed: usize,
    pending: usize,
    #[serde(skip)]
    synchronized_generations: BTreeMap<String, u64>,
}

impl ProviderDistributionOutcome {
    const fn state(&self) -> crate::provider_service::ServiceDistributionState {
        use crate::provider_service::ServiceDistributionState;
        if self.failed == 0 && self.pending == 0 {
            ServiceDistributionState::Current
        } else if self.succeeded == 0 && self.failed == 0 {
            ServiceDistributionState::Pending
        } else if self.succeeded == 0 && self.pending == 0 {
            ServiceDistributionState::Failed
        } else {
            ServiceDistributionState::Partial
        }
    }
}

#[derive(Debug, Deserialize)]
struct MachineEnrollRequest {
    token: String,
    public_key: String,
    encryption_public_key: String,
}

#[derive(Debug, Serialize)]
struct MachineEnrollResponse {
    service_id: String,
    machine_id: String,
    display_name: String,
    fingerprint: String,
}

#[derive(Debug, Serialize)]
struct MachineServiceResponse {
    service_id: String,
}

// Existing Machine clients collect their bounded CLI/auth inventory after
// receiving the challenge. Those probes can legitimately exceed 15 seconds on
// a loaded macOS host, so keep the nonce window above their worst-case budget.
const MACHINE_HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
const MACHINE_HEARTBEAT_MS: u64 = 15_000;
const MACHINE_HEARTBEAT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(MACHINE_HEARTBEAT_MS * 3);
const WEBSOCKET_FRAME_SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const MACHINE_RUNTIME_OUTBOUND_CAPACITY: usize = 16;

#[derive(Default)]
struct MachineHeartbeatWatchdog {
    deadline: Option<std::time::Instant>,
}

impl MachineHeartbeatWatchdog {
    fn observe_at(&mut self, observed_at: std::time::Instant) {
        self.deadline = Some(observed_at + MACHINE_HEARTBEAT_TIMEOUT);
    }

    fn expired_at(&self, observed_at: std::time::Instant) -> bool {
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
struct SessionReloadQuery {
    #[serde(default)]
    confirm_active_turn: bool,
    #[serde(default)]
    upgrade_provider: bool,
    expected_generation_digest: Option<String>,
}

/// Request body for the machine wake (`POST /api/sessions/{id}/prompt`).
#[derive(Debug, Deserialize)]
struct SessionPromptRequest {
    #[serde(default)]
    text: String,
    #[serde(default)]
    content: Vec<serde_json::Value>,
    #[serde(default)]
    delivery: Option<crate::send_delivery::SendDelivery>,
    #[serde(default)]
    front: bool,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    operation: Option<String>,
    #[serde(default)]
    target: Option<crate::send_delivery::DeliveryTarget>,
}

/// Request body for `POST /api/sessions`.
///
/// The retired WS `Inbound::NewSession` was fire-and-forget without a
/// `sessionId` reply or Machine placement. This endpoint is the only Web
/// creation path. It returns a durable `Starting` session before remote
/// workspace preparation completes, so clients can navigate immediately and
/// observe the authoritative preparation lifecycle on the destination page.
#[derive(Debug, Deserialize)]
struct NewSessionRequest {
    #[serde(default)]
    context_sources: Vec<String>,
    /// Only the durable context transition may reserve a creation identity.
    #[serde(skip)]
    reserved_id: Option<String>,
    /// Only a native fork may keep its source's older auth generation.
    #[serde(skip)]
    retained_auth_generation: Option<u64>,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    provider_account_id: Option<crate::provider_account::ProviderAccountId>,
    #[serde(default)]
    folder_id: Option<String>,
    #[serde(default)]
    preset: Option<String>,
    #[serde(default)]
    runtime_machine_id: Option<String>,
    /// Stable machine placement. API/ACP compatibility callers may omit it and
    /// retain their caller-owned local workspace; Web creation must select a
    /// registered Machine.
    #[serde(default = "default_machine_id")]
    machine_id: String,
    #[serde(default)]
    cwd: Option<String>,
    /// Which surface opened the session — defaults to `Api` for direct
    /// `curl`/test callers. The Web UI sends `Web` through this endpoint.
    #[serde(default)]
    origin: SessionOrigin,
    /// Create a view-only machine-driven system session. Defaults false; the Web
    /// UI never sets it.
    #[serde(default)]
    system: bool,
    /// Optional first turn owned by the creation transaction (for example a
    /// Columbus work-item resume). It is dispatched only after the prepared
    /// workspace has been committed and the worker has started.
    #[serde(default)]
    initial_prompt: Option<String>,
}

#[derive(Clone)]
struct ResolvedProviderGeneration {
    version: String,
    digest: String,
    auth_generation: Option<u64>,
    behavior: cowboy_provider_sdk::ProviderBehaviorContract,
    /// Set only by [`named_upgrade_target`]: authorizes moving one named
    /// session to this release on its existing login.
    named_carry: Option<NamedReleaseMove>,
}

/// A verified move of one named session to another release: the Hub proof
/// and the exact login whose runtime admission must hold around the change.
#[derive(Clone)]
struct NamedReleaseMove {
    carry: crate::core::NamedReleaseCarry,
    operation: String,
    completion: crate::store::provider_accounts::logins::NativeCompletion,
}

enum SchedulingAuthentication<'a> {
    /// Installation discovery only; never use this result to launch a session.
    Discovery,
    Service(Option<&'a crate::provider_service::ProviderAuthenticationStatus>),
    Native(&'a crate::provider_account::NativeProfileReceipt),
}

/// Response body for `POST /api/sessions`.
#[derive(Debug, Serialize)]
struct NewSessionResponse {
    session_id: String,
    provider_account_id: crate::provider_account::ProviderAccountId,
    provider_version: String,
    provider_generation_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_auth_generation: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct PreparedMachineWorkspace {
    path: String,
    source_path: String,
    revision: Option<String>,
    isolated: bool,
    created: bool,
}

/// Request body used by a Columbus checkout migration after the replacement
/// checkout has reached its stable path and before old worktree storage is
/// removed.
#[derive(Debug, Deserialize)]
struct ReconcileProjectSessionsRequest {
    project: String,
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Serialize)]
struct ReconcileProjectSessionsResponse {
    session_ids: Vec<String>,
    native_thread_ids: HashMap<String, String>,
}

#[cfg(test)]
mod machine_provider_tests;

/// Query string for `GET /api/sessions/{id}/files` — the composer's `@` picker.
#[derive(Debug, Deserialize)]
struct FileSearchQuery {
    /// Fuzzy query; empty returns the "most useful" files (shallow + recent).
    #[serde(default)]
    q: String,
    #[serde(default = "default_file_limit")]
    limit: usize,
}

#[derive(Debug, Serialize)]
struct FileSearchResponse {
    files: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeSearchResponse {
    api_version: u8,
    files: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct FileTreeQuery {
    #[serde(default)]
    path: String,
    #[serde(default = "default_file_tree_limit")]
    limit: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileTreeEntry {
    name: String,
    path: String,
    kind: &'static str,
    ignored: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileTreeResponse {
    api_version: u8,
    path: String,
    revision: String,
    entries: Vec<FileTreeEntry>,
    truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeChangeResponse {
    path: String,
    old_path: Option<String>,
    status: &'static str,
    staged: bool,
    unstaged: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    additions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deletions: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeChangesResponse {
    api_version: u8,
    head: Option<String>,
    revision: String,
    changes: Vec<CodeChangeResponse>,
    truncated: bool,
    comparison: Option<String>,
    default_comparison: Option<String>,
    comparisons: Vec<crate::code_review::GitComparison>,
    committed_comparisons: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    fetched_at_ms: Option<u64>,
    /// Review can fetch the compared branch here; Machine adapters cannot yet.
    fetch_available: bool,
}

#[derive(Debug, Deserialize)]
struct CodeFetchRequest {
    comparison: String,
}

#[derive(Debug, Deserialize)]
struct CodeChangesQuery {
    comparison: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeRepositoryResponse {
    api_version: u8,
    commits: Vec<crate::code_review::GitCommitSummary>,
    history_truncated: bool,
    worktrees: Vec<crate::code_review::GitWorktreeSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<crate::code_review::GitChangeTarget>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeCommitResponse {
    api_version: u8,
    #[serde(flatten)]
    commit: crate::code_review::GitCommitDetail,
}

#[derive(Debug, Deserialize)]
struct CodeRepositoryQuery {
    after: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodeCommitQuery {
    oid: String,
}

#[derive(Debug, Deserialize)]
struct CodeCommitDiffQuery {
    oid: String,
    path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeManifestResponse {
    api_version: u8,
    provider: String,
    revision: String,
    head: Option<String>,
    project: String,
    branch: Option<String>,
    worktree: Option<String>,
    change_count: usize,
    language: CodeLanguageCapabilities,
    buffer_mode: code_buffers::ReviewMode,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeLanguageCapabilities {
    provider: &'static str,
    state: &'static str,
    diagnostics: bool,
    inlay_hints: bool,
    semantic_tokens: bool,
    hover: bool,
    navigation: bool,
    outline: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeBufferVersion {
    replica_id: u32,
    timestamp: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodePoint {
    row: u32,
    column: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeDiagnostic {
    start: CodePoint,
    end: CodePoint,
    severity: i32,
    source: Option<String>,
    message: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeInlayHint {
    offset: u64,
    label: String,
    kind: Option<String>,
    padding_left: bool,
    padding_right: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeLanguageResponse {
    api_version: u8,
    path: String,
    version: Vec<CodeBufferVersion>,
    diagnostics: Vec<CodeDiagnostic>,
    inlay_hints: Vec<CodeInlayHint>,
    semantic_tokens: Vec<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeHoverBlock {
    text: String,
    language: Option<String>,
    markdown: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeHoverResponse {
    api_version: u8,
    path: String,
    contents: Vec<CodeHoverBlock>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeLocation {
    path: String,
    start: CodePoint,
    end: CodePoint,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeNavigationResponse {
    api_version: u8,
    path: String,
    locations: Vec<CodeLocation>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeDocumentSymbol {
    name: String,
    kind: i32,
    start: CodePoint,
    end: CodePoint,
    selection_start: CodePoint,
    selection_end: CodePoint,
    children: Vec<CodeDocumentSymbol>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeOutlineResponse {
    api_version: u8,
    path: String,
    symbols: Vec<CodeDocumentSymbol>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodeDiffQuery {
    path: Option<String>,
    cursor: Option<String>,
    #[serde(default = "default_code_context")]
    context: usize,
    #[serde(default = "default_true")]
    show_whitespace: bool,
    #[serde(default)]
    scope: CodeDiffScope,
    comparison: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum CodeDiffScope {
    #[default]
    Combined,
    Staged,
    Unstaged,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeDiffResponse {
    api_version: u8,
    path: String,
    revision: String,
    text: String,
    added: usize,
    removed: usize,
    truncated: bool,
    next_cursor: Option<String>,
    limited: bool,
}

#[derive(Debug, Deserialize)]
struct CodeFileQuery {
    path: String,
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodeBufferLeaseRequest {
    path: String,
    lease_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeBufferLeaseResponse {
    api_version: u8,
    path: String,
    leases: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeFileResponse {
    api_version: u8,
    path: String,
    revision: String,
    text: String,
    size: u64,
    truncated: bool,
    next_cursor: Option<String>,
    limited: bool,
}

const WORKSPACE_CODE_CONTEXT_PREFIX: &str = "workspace::";

struct ResolvedCodeContext {
    cwd: String,
    scope: CodeReadScope,
}

// Legacy resource calls retain their separate original-connection operation
// boundary. Buffered language reads use code_reads' product and route binding.
struct SessionCodeContext {
    machine_id: String,
    cwd: String,
    scope: crate::core::SessionCodeScope,
}

#[cfg(test)]
mod workspace_code_context_tests;

#[derive(Debug, Deserialize)]
struct CodeLanguageQuery {
    path: String,
}

#[derive(Debug, Deserialize)]
struct CodeHoverQuery {
    path: String,
    row: u32,
    column: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum CodeNavigationKind {
    Definition,
    Declaration,
    TypeDefinition,
    Implementation,
    References,
}

#[derive(Debug, Deserialize)]
struct CodeNavigationQuery {
    path: String,
    row: u32,
    column: u32,
    kind: CodeNavigationKind,
}

#[derive(Debug, Serialize)]
struct HistoryResponse {
    events: Vec<Envelope>,
    next_before_seq: Option<u64>,
    reached_start: bool,
}

#[derive(Debug, Serialize)]
struct QuestionPagesResponse {
    total: u64,
    exact: bool,
    pages: Vec<crate::core::QuestionPageSummary>,
    next_before_seq: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct QuestionPagesQuery {
    before: Option<u64>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct SessionBootstrapResponse {
    messages: Vec<Outbound>,
    /// Authoritative subagent observations (docs/background-tasks.md I5).
    /// A sibling of `messages`, not an `Outbound`, so protocol bridges and
    /// older Web clients are unaffected.
    #[serde(skip_serializing_if = "Option::is_none")]
    subagent_state: Option<serde_json::Value>,
    /// The account's task acknowledgements in this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    task_acknowledgements: Option<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
struct SessionBootstrapQuery {
    /// The highest seq the client already holds; children observed after it
    /// are included even when terminal.
    #[serde(default)]
    since_seq: Option<u64>,
}

fn focused_session_bootstrap(hub: &Hub, session_id: &str) -> Option<Vec<Outbound>> {
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
struct HistoryQuery {
    before_seq: u64,
    #[serde(default)]
    question_page: bool,
}

/// One cursor-addressed, event- and byte-bounded page of a session's history.
/// The client pages UP from the WS tail; older pages arrive here. A COMPLETE
/// past page never changes again, so `x-cowboy-history: complete` lets the
/// service worker satisfy any re-fetch (scroll back, reload, post-recycle
/// reload) with ZERO network. It stays `no-store` for the HTTP cache because
/// sign-out can clear the worker's cache but not that one. The still-growing
/// latest page is unmarked, and the client never asks for it (it has the tail
/// over WS). Unknown session → 404; out-of-range page → `[]`.
async fn api_history(
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
const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(25);

#[derive(Debug, Default, Deserialize)]
struct WebSocketQuery {
    bootstrap: Option<String>,
    client_id: Option<String>,
    client_kind: Option<String>,
    dataset: Option<String>,
}

#[derive(Clone)]
struct WebSocketAuthentication {
    principal: ProductPrincipal,
    via_cookie: bool,
    cookie_token: Option<String>,
    bearer: Option<String>,
    device_identity: Option<crate::client_auth::DeviceAccessIdentity>,
    capacity: WebSocketCapacityIdentity,
}

#[derive(Clone, Debug)]
struct WebSocketCapacityIdentity {
    client_id: String,
    user_id: Option<String>,
    principal_class: String,
    session_id: Option<String>,
    client_kind: String,
}

fn global_bootstrap(hub: &Hub, principal: &ProductPrincipal) -> Vec<Outbound> {
    let sessions = hub.session_list_filtered(|owner| principal.can_see(owner));
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
        hub.sync_resync()
            .into_iter()
            .filter_map(|message| project_outbound(hub, principal, &visible, message)),
    );
    messages
}

fn connect_bootstrap(
    hub: &Hub,
    lazy: bool,
    principal: &ProductPrincipal,
    machine_snapshot: Option<Outbound>,
) -> Vec<Outbound> {
    let mut messages = global_bootstrap(hub, principal);
    if !lazy {
        for session in hub.session_list_filtered(|owner| principal.can_see(owner)) {
            if let Some(session_messages) = focused_session_bootstrap(hub, &session.id) {
                messages.extend(session_messages);
            }
        }
    }
    messages.extend(machine_snapshot);
    messages.push(Outbound::BootstrapComplete);
    messages
}

fn principal_owned_sync_state(state: &str) -> bool {
    state == "workspace-order" || state == "drafts" || state == "preferences"
}

const WS_CAPACITY_REQUIRED_CLOSE_CODE: u16 = 4429;

fn active_capacity_message(
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

struct ProductAuthSessionSnapshot {
    message: Outbound,
    valid: bool,
    user_id: String,
    next_due_at_ms: i64,
}

/// Derive a short session title from the first prompt: the first non-empty
/// line, whitespace-collapsed and truncated. Prefers the legacy `text` field;
/// falls back to the first text block in `content` (attachment prompts carry
/// their text there). Returns None for an attachment-only / empty prompt.
fn first_prompt_title(text: &str, content: &[serde_json::Value]) -> Option<String> {
    // Cap length on a char boundary so a long first line stays a label, not a
    // paragraph.
    const MAX: usize = 60;
    let raw = if text.trim().is_empty() {
        content.iter().find_map(|v| {
            if v.get("type").and_then(serde_json::Value::as_str) == Some("text") {
                v.get("text")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            } else {
                None
            }
        })?
    } else {
        text.to_owned()
    };
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    // Append an ellipsis when the first line is cut to MAX.
    if collapsed.chars().count() > MAX {
        let head: String = collapsed.chars().take(MAX).collect();
        Some(format!("{head}…"))
    } else {
        Some(collapsed)
    }
}

#[allow(clippy::too_many_lines)] // one cohesive command-dispatch match
/// Address a replayed edit or removal whose row already left the queue or
/// drafts. The session itself still exists, so the outcome is `stale` rather
/// than `not_found`; a command without a client id has no outbox entry to
/// retire and stays a silent no-op.
fn report_stale_row(state: &AppState, session_id: &str, cmid: Option<&str>) {
    if let Some(cmid) = cmid {
        state.hub.command_result(
            session_id,
            cmid,
            CommandOutcome::Stale,
            "this message no longer exists",
        );
    }
}

/// The folder tree a principal organizes: their own while product auth is on,
/// the shared unowned tree while it is off (mirroring session ownership).
fn folder_actor(state: &AppState, principal: &ProductPrincipal) -> FolderActor {
    FolderActor {
        user_id: state
            .product_auth_enabled
            .then(|| principal.user_id.clone()),
        sees_all: principal.sees_every_session(),
    }
}

fn apply_visible_reorder(
    state: &AppState,
    principal: &ProductPrincipal,
    order: &[String],
) -> Result<(), String> {
    if !principal.can_reorder() {
        return Err("viewers cannot reorder sessions".to_owned());
    }
    let submitted = order
        .iter()
        .map(|id| serde_json::Value::String(id.clone()))
        .collect::<Vec<_>>();
    let filtered = filter_submitted_order(state, principal, &submitted);
    state.hub.reorder_sessions(&filtered);
    Ok(())
}

fn filter_submitted_order(
    state: &AppState,
    principal: &ProductPrincipal,
    submitted: &[serde_json::Value],
) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    submitted
        .iter()
        .filter_map(serde_json::Value::as_str)
        .filter_map(|id| {
            let info = state.hub.session_info(id)?;
            principal
                .can_see(info.meta.owner_user_id.as_deref())
                .then(|| id.to_owned())
        })
        .filter(|id| seen.insert(id.clone()))
        .collect()
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
mod zed_adapter_tests;

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

#[cfg(test)]
mod product_auth_api_tests;

mod product_auth_api;
use product_auth_api::*;

mod admin_api;
use admin_api::*;

mod usage_api;
use usage_api::*;

mod code_api;
use code_api::*;

mod machine_api;
use machine_api::*;

mod provider_api;
use provider_api::*;

mod session_api;
use session_api::*;

mod machine_connection;
use machine_connection::*;

mod client_ws;
use client_ws::*;

mod background_jobs;
use background_jobs::*;

mod http_policy;
use http_policy::*;

mod static_web;
use static_web::*;

mod observability_api;
use observability_api::*;

mod plugin_api;
use plugin_api::*;

mod provider_auth_flow;
use provider_auth_flow::*;

mod code_context;
use code_context::*;
