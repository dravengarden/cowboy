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
use tracing_subscriber::EnvFilter;

use agent_client_protocol::schema::v1::ContentBlock;

use crate::acp::AgentCommand;
use crate::cli::ServeArgs;
use crate::code_review::CodeProvider as _;
use crate::core::{
    DispatchReq, Envelope, Event, FanoutFrame, Hub, Inbound, Outbound, PersistenceHealth,
    RestoredSession, SessionOrigin, Status, StoreSink, StoreWrite, project_sync_value,
};
use crate::diff_snapshot::{DiffSnapshotCache, DiffSnapshotKey};
use crate::machine_control::MachineControl;
use crate::observability::{Observability, SubmitReceipt, TelemetryBatch};
use crate::persistence::EventReducer;
use crate::product_auth::{ProductPrincipal, WS_AUTH_REQUIRED_CLOSE_CODE};
use crate::remote_runtime::{RemoteBootstrap, RemoteRuntime};
use crate::runtime::RuntimeHealth;
use crate::runtime_router::RuntimeRouter;
use crate::store::Store;
use crate::supervisor::Supervisor;
use crate::usage::UsageService;
use crate::web_push::{NotificationCategory, WebPushService, WebPushSubscription};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, watch};
use tokio_util::io::ReaderStream;

#[derive(Clone)]
struct PluginUninstallPlan {
    machine_id: String,
    plugin_id: String,
    generation_digest: String,
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
}

type PluginLifecycleFences = Arc<parking_lot::RwLock<HashMap<(String, String), PluginFenceState>>>;

const fn provider_session_has_active_turn(status: crate::agent_model::Status) -> bool {
    matches!(status, crate::agent_model::Status::Busy)
}

#[derive(Clone)]
struct ProviderAuthExecutor {
    machine_id: String,
    provider_id: String,
    provider_version: String,
    generation_digest: String,
    auth_contract_fingerprint: String,
    auth_method: String,
    expected_generation: u64,
    promotion_started: bool,
    expires_at_ms: i64,
}

impl ProviderAuthExecutor {
    fn accepts_candidate(
        &self,
        machine_id: &str,
        provider_id: &str,
        auth_method: &str,
        provider_version: &str,
        generation_digest: &str,
        auth_contract_fingerprint: &str,
    ) -> bool {
        self.machine_id == machine_id
            && self.provider_id == provider_id
            && self.provider_version == provider_version
            && self.generation_digest == generation_digest
            && self.auth_contract_fingerprint == auth_contract_fingerprint
            && self.auth_method == auth_method
    }
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

fn reconcile_provider_auth_executors(
    executors: &mut HashMap<String, ProviderAuthExecutor>,
    provider_id: &str,
    timestamp: i64,
) -> ProviderAuthReconciliation {
    let expired: Vec<_> = executors
        .iter()
        .filter(|(_, executor)| executor.expires_at_ms < timestamp)
        .map(|(request_id, executor)| (executor.provider_id.clone(), request_id.clone()))
        .collect();
    executors.retain(|_, executor| executor.expires_at_ms >= timestamp);
    let active = executors
        .iter()
        .filter(|(_, executor)| executor.provider_id == provider_id)
        .max_by_key(|(_, executor)| executor.expires_at_ms)
        .map(|(request_id, executor)| (request_id.clone(), executor.clone()));
    ProviderAuthReconciliation { expired, active }
}

fn resumed_provider_authentication_response(
    request_id: String,
    executor: &ProviderAuthExecutor,
) -> Response {
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "request_id": request_id,
            "expires_at_ms": executor.expires_at_ms,
            "method": executor.auth_method,
            "resumed": true,
        })),
    )
        .into_response()
}

struct AppState {
    service_id: String,
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
    plugin_catalog: Arc<crate::plugin_catalog::PluginCatalog>,
    provider_catalog: Arc<crate::provider_catalog::ProviderCatalog>,
    provider_auth: Arc<crate::provider_service::ProviderAuthService>,
    provider_auth_executors: parking_lot::Mutex<HashMap<String, ProviderAuthExecutor>>,
    plugin_uninstall_plans: parking_lot::Mutex<HashMap<String, PluginUninstallPlan>>,
    plugin_lifecycle_fences: PluginLifecycleFences,
    desired_machine_components: Arc<Vec<crate::machine_protocol::DesiredComponent>>,
    web_root: PathBuf,
    usage: UsageService,
    diff_snapshots: DiffSnapshotCache,
    code_cache: crate::code_cache::CodeCache,
    zed_adapter_socket: Option<PathBuf>,
    observability: Observability,
    web_push: Arc<WebPushService>,
    public_origins: Arc<Vec<String>>,
    product_auth_enabled: bool,
    product_authentication: Arc<crate::auth_plugins::ProductAuthentication>,
    oidc_transactions: Arc<crate::oidc::OidcTransactions>,
    oidc_native_handoffs: Arc<crate::oidc::NativeHandoffs>,
    device_authorizations: Arc<crate::client_auth::DeviceAuthorizations>,
    device_access: Arc<crate::client_auth::DeviceAccessSessions>,
}

#[derive(Clone)]
struct MachineSnapshots {
    store: Option<Store>,
    hub: Hub,
    runtime_router: Arc<RuntimeRouter>,
    desired_components: Arc<Vec<crate::machine_protocol::DesiredComponent>>,
    product_auth_enabled: bool,
    revision: Arc<AtomicU64>,
}

impl MachineSnapshots {
    fn new(
        store: Option<Store>,
        hub: Hub,
        runtime_router: Arc<RuntimeRouter>,
        desired_components: Arc<Vec<crate::machine_protocol::DesiredComponent>>,
        product_auth_enabled: bool,
    ) -> Self {
        Self {
            store,
            hub,
            runtime_router,
            desired_components,
            product_auth_enabled,
            revision: Arc::new(AtomicU64::new(0)),
        }
    }

    async fn load(&self) -> anyhow::Result<Vec<crate::machine_protocol::MachineSummary>> {
        let Some(store) = self.store.as_ref() else {
            return Ok(Vec::new());
        };
        let machines = store.list_machines().await?;
        let mut session_loads: HashMap<String, (u32, HashMap<String, u64>)> = HashMap::new();
        for session in self
            .hub
            .session_list()
            .into_iter()
            .filter(|session| session.status != crate::agent_model::Status::Exited)
        {
            let (active_sessions, providers) = session_loads
                .entry(session.machine_id)
                .or_insert_with(|| (0, HashMap::new()));
            *active_sessions = active_sessions.saturating_add(1);
            let provider_sessions = providers.entry(session.provider).or_default();
            *provider_sessions = provider_sessions.saturating_add(1);
        }
        let checked_at_ms = now_ms();
        Ok(machines
            .into_iter()
            .filter(|machine| product_machine_is_visible(machine, self.product_auth_enabled))
            .map(|machine| {
                let workspaces: Vec<crate::machine_protocol::MachineWorkspace> = machine
                    .inventory
                    .get("workspaces")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
                let workspace_revision = machine
                    .inventory
                    .get("workspace_revision")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                let mut components: Vec<crate::machine_protocol::ComponentInventory> = machine
                    .inventory
                    .get("components")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
                let plugins: Vec<crate::machine_protocol::PluginInventory> = machine
                    .inventory
                    .get("plugins")
                    .or_else(|| machine.inventory.get("providers"))
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
                let capacity: crate::machine_protocol::MachineCapacity = machine
                    .inventory
                    .get("capacity")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
                let provider_contracts = machine
                    .inventory
                    .get("provider_contracts")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok());
                let (active_sessions, provider_sessions) = session_loads
                    .get(&machine.id)
                    .map_or((0, None), |(active, providers)| (*active, Some(providers)));
                let local = machine.connection_mode == "local";
                for component in &mut components {
                    if matches!(
                        component.id.kind,
                        crate::machine_protocol::ComponentKind::AcpRuntime
                            | crate::machine_protocol::ComponentKind::ProviderAdapter
                            | crate::machine_protocol::ComponentKind::ProviderCli
                    ) {
                        component.active_leases = match component.id.kind {
                            crate::machine_protocol::ComponentKind::ProviderAdapter
                            | crate::machine_protocol::ComponentKind::ProviderCli => {
                                let slot = component.id.slot.as_str();
                                let exact = provider_sessions
                                    .and_then(|providers| providers.get(slot))
                                    .copied()
                                    .unwrap_or(0);
                                if slot == "claude" {
                                    ["claude-code", "claude-deepseek"].iter().fold(
                                        exact,
                                        |total, provider| {
                                            total.saturating_add(
                                                provider_sessions
                                                    .and_then(|providers| providers.get(*provider))
                                                    .copied()
                                                    .unwrap_or(0),
                                            )
                                        },
                                    )
                                } else {
                                    exact
                                }
                            }
                            _ => u64::from(active_sessions),
                        };
                    }
                    if let Some(desired) = self
                        .desired_components
                        .iter()
                        .find(|desired| desired.id == component.id)
                    {
                        let available = component.state
                            != crate::machine_protocol::ComponentState::Active
                            || !component.digest.eq_ignore_ascii_case(&desired.digest);
                        component.update = Some(crate::machine_protocol::ComponentUpdate {
                            latest_version: desired.version.clone(),
                            available,
                            source: "signed Cowboy manifest".to_owned(),
                            checked_at_ms,
                            installable: available,
                        });
                    }
                }
                let pending_updates = self
                    .desired_components
                    .iter()
                    .filter(|desired| {
                        !components.iter().any(|current| {
                            current.id == desired.id
                                && current.digest.eq_ignore_ascii_case(&desired.digest)
                                && current.state == crate::machine_protocol::ComponentState::Active
                        })
                    })
                    .map(|desired| desired.id.clone())
                    .collect();
                let connected = self.runtime_router.connected(&machine.id);
                let schedulable = connected
                    && !workspaces.is_empty()
                    && !capacity.draining
                    && active_sessions < capacity.max_sessions;
                crate::machine_protocol::MachineSummary {
                    local,
                    connected,
                    schedulable,
                    id: machine.id,
                    display_name: machine.display_name,
                    platform: machine.platform,
                    architecture: machine.architecture,
                    status: machine.status,
                    fingerprint: machine.fingerprint,
                    workspaces,
                    workspace_revision,
                    components,
                    plugins,
                    provider_contracts,
                    capacity,
                    active_sessions,
                    pending_updates,
                }
            })
            .collect())
    }

    async fn connect_message(&self) -> anyhow::Result<Outbound> {
        let revision = self.revision.load(Ordering::Acquire);
        Ok(Outbound::Machines {
            revision,
            machines: self.load().await?,
            resync: true,
        })
    }

    async fn publish(&self) {
        let revision = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        match self.load().await {
            Ok(machines) => self.hub.broadcast_machines(revision, machines),
            Err(error) => tracing::warn!(%error, revision, "publishing Machine snapshot"),
        }
    }
}

const STORE_QUEUE_CAPACITY: usize = 8_192;
const FORCE_CANCEL_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
const QUEUE_EDIT_RECONNECT_GRACE: std::time::Duration = std::time::Duration::from_secs(15);
const MACHINE_RECONNECT_GRACE_SECONDS: i32 = 15;
const RUNTIME_RECONCILIATION_GRACE: std::time::Duration = std::time::Duration::from_secs(15);
const MACHINE_RECONNECT_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
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
    let service_id = crate::service_identity::load_or_create(&args.data_dir)
        .context("loading Cowboy Service identity")?;
    let desired_machine_components = if let Some(path) = &args.machine_components_manifest {
        serde_json::from_slice::<Vec<crate::machine_protocol::DesiredComponent>>(
            &std::fs::read(path).with_context(|| {
                format!("reading Machine component manifest {}", path.display())
            })?,
        )
        .with_context(|| format!("parsing Machine component manifest {}", path.display()))?
    } else {
        Vec::new()
    };
    let desired_machine_components = Arc::new(desired_machine_components);
    init_tracing();
    let legacy_oidc_provider = load_oidc_provider(
        args.product_auth_enabled,
        args.cardea_oidc_config.as_deref(),
    )?;
    if args.cardea_oidc_config.is_some() && !args.product_auth_enabled {
        tracing::warn!("Cardea OIDC is configured but product authentication is disabled");
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
    let plugin_catalog = Arc::new(crate::plugin_catalog::PluginCatalog::open(
        &args.data_dir,
        args.plugin_catalog_dir.clone(),
    )?);
    let product_authentication = Arc::new(if args.product_auth_enabled {
        crate::auth_plugins::ProductAuthentication::load(
            args.auth_config.as_deref(),
            &plugin_catalog,
            legacy_oidc_provider,
        )
        .context("loading product authentication methods")?
    } else {
        if args.auth_config.is_some() {
            tracing::warn!(
                "authentication Plugins are configured but product authentication is disabled"
            );
        }
        crate::auth_plugins::ProductAuthentication::disabled()
    });
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
    let (hub, store, persistence_health, writer_task, purge_task, session_id_floor) =
        if let Some(url) = args.database_url() {
            let store = Store::connect(url, args.data_dir.join("artifacts"))
                .await
                .context("connecting database")?;
            store.migrate().await.context("running migrations")?;
            let session_id_floor = store
                .next_session_number()
                .await
                .context("seeding session id counter")?;
            let (tx, rx) = mpsc::channel::<StoreWrite>(STORE_QUEUE_CAPACITY);
            let health = Arc::new(PersistenceHealth::default());
            let hub = Hub::with_store(Some(StoreSink::new(tx, Arc::clone(&health))));
            let settings = store
                .load_settings()
                .await
                .context("loading persisted auth settings")?;
            hub.load_settings(settings);
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
                })
                .collect();
            let restored_count = restored.len();
            // Detached workers outlive this control-plane process. Keep
            // persisted Busy turns guarded until their Machine runtime has had
            // one bounded reconnect window to prove ownership; only then may a
            // missing worker become Interrupted.
            hub.restore_reconciling_runtime(restored);
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
            (Hub::new(), None, None, None, None, 1)
        };
    let usage = UsageService::new(
        args.codex_command.clone(),
        store.clone(),
        Some(args.data_dir.join("usage-snapshot.json")),
    );
    let runtime_router = RuntimeRouter::new();
    let machine_snapshots = MachineSnapshots::new(
        store.clone(),
        hub.clone(),
        Arc::clone(&runtime_router),
        Arc::clone(&desired_machine_components),
        args.product_auth_enabled,
    );
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
    let web_push_task = tokio::spawn(run_web_push_notifications(
        hub.clone(),
        Arc::clone(&web_push),
        shutdown_rx.clone(),
    ));
    // Reset credits belong to provider accounts, not sessions. Restore one
    // shared timer per provider and keep them independent from session queues.
    if let Some(store) = store.as_ref() {
        for provider in crate::usage::RESET_PROVIDERS {
            match store.load_provider_reset(provider).await {
                Ok(Some(action)) => {
                    usage
                        .set_reset_schedule(
                            provider,
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
                for provider in crate::usage::RESET_PROVIDERS {
                    let action = match store.as_ref() {
                        Some(store) => store.load_provider_reset(provider).await.ok().flatten(),
                        None => None,
                    };
                    if let Some(action) = action.filter(|item| item.next_attempt_at_ms <= now_ms())
                    {
                        let key = action.idempotency_key;
                        // xAI does not accept an idempotency key. Claim its
                        // one-shot timer before any provider call so a crash or
                        // ambiguous response cannot consume a later reset on
                        // an automatic retry.
                        if provider == "xai" {
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
                        match usage.consume_nearest_reset(provider, &key, None).await {
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
                                if provider != "xai"
                                    && let Some(store) = store.as_ref()
                                {
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
                                if provider == "xai" {
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
    let supervisor = Arc::new(Supervisor::new(
        hub.clone(),
        args.workspace_root.clone(),
        session_id_floor,
        Arc::clone(&runtime_router),
    ));
    let observability = Observability::start(
        store.clone(),
        args.victoria_logs_url,
        args.victoria_metrics_url,
    );

    // Background dispatcher: the Hub owns each session's send-queue but can't
    // call the Supervisor (which holds the Hub) — that cycle is why the queue
    // used to live client-side. The Hub now makes the drain decision under its
    // lock and hands each ready prompt over this channel; we send it to the
    // agent here, off the lock. Wired before any client connects.
    let (dispatch_tx, dispatch_rx) = mpsc::channel::<DispatchReq>(1_024);
    hub.set_dispatch_tx(dispatch_tx);
    let plugin_lifecycle_fences: PluginLifecycleFences =
        Arc::new(parking_lot::RwLock::new(HashMap::new()));
    runtime_health.set_dispatcher(true);
    let dispatcher_health = Arc::clone(&runtime_health);
    let dispatcher_hub = hub.clone();
    let dispatcher_supervisor = Arc::clone(&supervisor);
    let dispatcher_plugin_fences = Arc::clone(&plugin_lifecycle_fences);
    let dispatcher_shutdown = shutdown_rx.clone();
    let dispatcher_exit_state = dispatcher_shutdown.clone();
    let mut dispatcher_task = tokio::spawn(async move {
        run_dispatcher(
            dispatcher_hub,
            dispatcher_supervisor,
            dispatcher_plugin_fences,
            dispatch_rx,
            dispatcher_shutdown,
        )
        .await;
        dispatcher_health.set_dispatcher(false);
        if *dispatcher_exit_state.borrow() {
            tracing::info!("dispatcher stopped after shutdown");
        } else {
            tracing::error!("dispatcher exited while Cowboy was still serving");
        }
    });

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
    // genuine interruptions after the same bounded grace used for Machine
    // presence. Interrupted turns stay stopped until the user submits work.
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
        "cowboy serving",
    );

    let result = serve_axum(
        args.bind,
        args.data_dir.clone(),
        AppState {
            service_id,
            hub,
            supervisor,
            store,
            persistence_health,
            shutdown: shutdown_rx,
            runtime_health,
            runtime_router,
            machine_control: Arc::new(MachineControl::default()),
            machine_snapshots,
            plugin_catalog,
            provider_catalog,
            provider_auth,
            provider_auth_executors: parking_lot::Mutex::new(HashMap::new()),
            plugin_uninstall_plans: parking_lot::Mutex::new(HashMap::new()),
            plugin_lifecycle_fences,
            desired_machine_components,
            web_root: args.web_root,
            usage,
            diff_snapshots: DiffSnapshotCache::default(),
            code_cache,
            zed_adapter_socket: args.zed_adapter_socket,
            observability,
            web_push,
            public_origins: Arc::new(crate::product_auth::load_public_origins()),
            product_auth_enabled: args.product_auth_enabled,
            product_authentication,
            oidc_transactions: Arc::new(crate::oidc::OidcTransactions::default()),
            oidc_native_handoffs: Arc::new(crate::oidc::NativeHandoffs::default()),
            device_authorizations: Arc::new(crate::client_auth::DeviceAuthorizations::default()),
            device_access: Arc::new(crate::client_auth::DeviceAccessSessions::default()),
        },
        shutdown_tx,
    )
    .await;
    scheduler_task.abort();
    reset_task.abort();
    runtime_reconciliation_task.abort();
    web_push_task.abort();
    match tokio::time::timeout(std::time::Duration::from_secs(5), &mut dispatcher_task).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::error!(%error, "dispatcher task failed during shutdown"),
        Err(_) => {
            tracing::error!("dispatcher did not drain within shutdown deadline");
            dispatcher_task.abort();
        }
    }
    if let Some(task) = purge_task {
        task.abort();
    }
    if let Some(task) = machine_presence_task {
        task.abort();
    }
    let _ = store_shutdown_tx.send(true);
    if let Some(task) = writer_task {
        match tokio::time::timeout(std::time::Duration::from_secs(10), task).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!(%error, "store writer task failed during shutdown"),
            Err(_) => tracing::error!("store writer did not drain within shutdown deadline"),
        }
    }
    result
}

fn load_oidc_provider(
    product_auth_enabled: bool,
    config_path: Option<&std::path::Path>,
) -> anyhow::Result<Option<Arc<crate::oidc::OidcProvider>>> {
    if !product_auth_enabled {
        return Ok(None);
    }
    config_path
        .map(crate::oidc::OidcProvider::load)
        .transpose()
        .context("loading Cardea OIDC consumer profile")
        .map(|provider| provider.map(Arc::new))
}

async fn run_machine_presence_sweeper(
    store: Store,
    machine_snapshots: MachineSnapshots,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        match store.expire_machine_reconnects().await {
            Ok(expired) if expired > 0 => {
                tracing::warn!(expired, "Machine reconnect grace expired");
                machine_snapshots.publish().await;
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "expiring Machine reconnect grace"),
        }
        tokio::select! {
            _ = tokio::time::sleep(MACHINE_RECONNECT_SWEEP_INTERVAL) => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
}

async fn run_web_push_notifications(
    hub: Hub,
    service: Arc<WebPushService>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut events = hub.subscribe();
    loop {
        let outbound = tokio::select! {
            result = events.recv() => match result {
                Ok(outbound) => outbound,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "Web Push observer lagged");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { break; }
                continue;
            }
        };
        let notification = match outbound.outbound() {
            Outbound::Event { envelope }
                if matches!(envelope.event, Event::PermissionRequest { .. }) =>
            {
                Some((
                    NotificationCategory::Permission,
                    envelope.session_id.clone(),
                ))
            }
            Outbound::Error {
                session_id: Some(session_id),
                ..
            } => Some((NotificationCategory::Error, session_id.clone())),
            _ => None,
        };
        let Some((category, session_id)) = notification else {
            continue;
        };
        let title = hub
            .session_info(&session_id)
            .map_or_else(|| "Session".to_owned(), |info| info.meta.title);
        let service = Arc::clone(&service);
        tokio::spawn(async move {
            service.notify(category, &session_id, &title).await;
        });
    }
}

/// Drain write-behind intents in small batches. Streaming text/tool updates are
/// reduced to stable rows before persistence, and transient usage/session-info
/// frames only advance the durable sequence watermark.
async fn run_store_writer(
    store: Store,
    mut rx: mpsc::Receiver<StoreWrite>,
    health: Arc<PersistenceHealth>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut reducer = EventReducer::default();
    loop {
        let first = tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_ok() && *shutdown.borrow() {
                    rx.close();
                    rx.recv().await
                } else {
                    continue;
                }
            }
            write = rx.recv() => write,
        };
        let Some(first) = first else { break };
        let mut batch = vec![first];
        while batch.len() < 256 {
            match tokio::time::timeout(std::time::Duration::from_millis(10), rx.recv()).await {
                Ok(Some(write)) => batch.push(write),
                Ok(None) | Err(_) => break,
            }
        }
        health.consumed_writes(&batch);
        if !apply_store_batch(&store, &mut reducer, batch).await {
            health.mark_failed_batch();
        }
    }
    tracing::info!("store writer shutting down (channel closed)");
}

async fn apply_store_batch(
    store: &Store,
    reducer: &mut EventReducer,
    writes: Vec<StoreWrite>,
) -> bool {
    let mut events: BTreeMap<(String, u64), Envelope> = BTreeMap::new();
    let mut highwaters: HashMap<String, u64> = HashMap::new();
    let mut ok = true;
    for write in writes {
        match write {
            StoreWrite::AppendEvent(env) => {
                highwaters
                    .entry(env.session_id.clone())
                    .and_modify(|seq| *seq = (*seq).max(env.seq.saturating_add(1)))
                    .or_insert_with(|| env.seq.saturating_add(1));
                if let Some(reduced) = reducer.reduce(env) {
                    events.insert((reduced.session_id.clone(), reduced.seq), reduced);
                }
            }
            StoreWrite::ClearEvents { ref session_id } => {
                ok &= flush_event_batch(store, &mut events, &mut highwaters).await;
                reducer.clear_session(session_id);
                ok &= retry_store_write(store, &write).await;
            }
            other => {
                ok &= flush_event_batch(store, &mut events, &mut highwaters).await;
                ok &= retry_store_write(store, &other).await;
            }
        }
    }
    ok &= flush_event_batch(store, &mut events, &mut highwaters).await;
    ok
}

async fn flush_event_batch(
    store: &Store,
    events: &mut BTreeMap<(String, u64), Envelope>,
    highwaters: &mut HashMap<String, u64>,
) -> bool {
    if events.is_empty() && highwaters.is_empty() {
        return true;
    }
    let rows: Vec<Envelope> = std::mem::take(events).into_values().collect();
    let watermarks = std::mem::take(highwaters);
    let mut last_error = None;
    for attempt in 0..4 {
        match store.upsert_event_batch(&rows, &watermarks).await {
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
    false
}

async fn record_lifecycle_incidents(store: &Store, rows: &[Envelope]) -> anyhow::Result<()> {
    for envelope in rows {
        if let Event::TurnEnd { stop_reason } = &envelope.event
            && let Some(raw_summary) = stop_reason.strip_prefix("error: ")
        {
            let occurred_at_ms = now_ms();
            let classification = classify_session_error(raw_summary);
            let (summary, truncated) = bounded_incident_summary(raw_summary);
            let fingerprint = format!(
                "{:x}",
                Sha256::digest(format!("{classification}:{raw_summary}").as_bytes())
            );
            let incident_id = format!("turn-error:{}:{}", envelope.session_id, envelope.seq);
            store
                .upsert_runtime_incident(&crate::store::RuntimeIncidentWrite {
                    id: incident_id.clone(),
                    occurred_at_ms,
                    source: "controller".to_owned(),
                    classification: classification.to_owned(),
                    severity: session_error_severity(classification).to_owned(),
                    state: "failed".to_owned(),
                    summary: summary.clone(),
                    fingerprint,
                    session_id: Some(envelope.session_id.clone()),
                    client_id: None,
                    machine_id: None,
                    trace_id: None,
                    build: Some(env!("CARGO_PKG_VERSION").to_owned()),
                    evidence_start_ms: occurred_at_ms.saturating_sub(30_000),
                    evidence_end_ms: occurred_at_ms.saturating_add(30_000),
                    detail: serde_json::json!({
                        "source": "turn_failure",
                        "turn_end_seq": envelope.seq,
                        "stop_reason": summary,
                        "truncated": truncated,
                    }),
                })
                .await?;
            tracing::error!(
                incident_id,
                session_id = %envelope.session_id,
                classification,
                error = %raw_summary,
                "agent turn failed"
            );
        }
        let Event::Lifecycle { status, detail } = &envelope.event else {
            continue;
        };
        if *status == Status::Running {
            let recovered = store
                .recover_runtime_incident(&envelope.session_id, now_ms(), "session_running")
                .await?;
            if recovered > 0 {
                tracing::info!(
                    session_id = %envelope.session_id,
                    recovery_outcome = "session_running",
                    "runtime incident recovered"
                );
            }
            continue;
        }
        if !matches!(status, Status::Crashed | Status::Interrupted) {
            continue;
        }
        let occurred_at_ms = now_ms();
        let classification = if *status == Status::Interrupted {
            classify_interruption_detail(detail.as_deref())
        } else {
            classify_crash_detail(detail.as_deref())
        };
        let raw_summary = detail.as_deref().unwrap_or(if *status == Status::Crashed {
            "Runtime crashed without a diagnostic detail"
        } else {
            "Runtime was interrupted"
        });
        let (summary, truncated) = bounded_incident_summary(raw_summary);
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(format!("{classification}:{raw_summary}").as_bytes())
        );
        let incident_id = format!("lifecycle:{}:{}", envelope.session_id, envelope.seq);
        store
            .upsert_runtime_incident(&crate::store::RuntimeIncidentWrite {
                id: incident_id.clone(),
                occurred_at_ms,
                source: "controller".to_owned(),
                classification: classification.to_owned(),
                severity: if *status == Status::Crashed {
                    "critical".to_owned()
                } else {
                    "warning".to_owned()
                },
                state: "active".to_owned(),
                summary: summary.clone(),
                fingerprint,
                session_id: Some(envelope.session_id.clone()),
                client_id: None,
                machine_id: None,
                trace_id: None,
                build: Some(env!("CARGO_PKG_VERSION").to_owned()),
                evidence_start_ms: occurred_at_ms.saturating_sub(30_000),
                evidence_end_ms: occurred_at_ms.saturating_add(30_000),
                detail: serde_json::json!({
                    "status": status,
                    "lifecycle_seq": envelope.seq,
                    "detail": summary,
                    "truncated": truncated,
                }),
            })
            .await?;
        tracing::error!(
            incident_id,
            session_id = %envelope.session_id,
            classification,
            lifecycle_seq = envelope.seq,
            detail = summary,
            "runtime incident opened"
        );
    }
    Ok(())
}

fn classify_crash_detail(detail: Option<&str>) -> &'static str {
    if detail.is_some_and(is_context_window_rejection) {
        return "provider_context_limit";
    }
    if detail.is_some_and(is_empty_stream_failure) {
        return "provider_empty_stream";
    }
    let detail = detail.unwrap_or_default().to_ascii_lowercase();
    if detail.contains("worker")
        && (detail.contains("before readiness")
            || detail.contains("did not become ready")
            || detail.contains("generation launch failed")
            || detail.contains("spawning worker")
            || detail.contains("transient worker unit"))
    {
        "worker_startup_failure"
    } else if detail.contains("oom")
        || detail.contains("out of memory")
        || detail.contains("signal: 9")
    {
        "resource_exhaustion"
    } else if detail.contains("protocol") || detail.contains("frame") || detail.contains("json-rpc")
    {
        "protocol_failure"
    } else if detail.contains("connection")
        || detail.contains("disconnected")
        || detail.contains("network error")
        || detail.contains("socket")
        || detail.contains("timed out")
    {
        "transport_failure"
    } else if detail.contains("exited")
        || detail.contains("exit status")
        || detail.contains("signal")
    {
        "process_exit"
    } else {
        "runtime_failure"
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

fn bounded_incident_summary(value: &str) -> (String, bool) {
    let mut end = value.len().min(4 * 1024);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    (value[..end].to_owned(), end < value.len())
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
mod incident_classification_tests {
    use super::{
        bounded_incident_summary, classify_crash_detail, classify_interruption_detail,
        classify_session_error, session_error_severity,
    };

    #[test]
    fn crash_details_map_to_stable_incident_classes() {
        assert_eq!(
            classify_crash_detail(Some("process exited with signal: 9")),
            "resource_exhaustion"
        );
        assert_eq!(
            classify_crash_detail(Some("runtime frame too large")),
            "protocol_failure"
        );
        assert_eq!(
            classify_crash_detail(Some("socket connection timed out")),
            "transport_failure"
        );
        assert_eq!(
            classify_crash_detail(Some("exit status 217")),
            "process_exit"
        );
        assert_eq!(
            classify_crash_detail(Some(
                "API Error: 400 This model's maximum context length is 1048576 tokens"
            )),
            "provider_context_limit"
        );
        assert_eq!(
            classify_crash_detail(Some("API Error: Stream ended without receiving any events")),
            "provider_empty_stream"
        );
        assert_eq!(classify_crash_detail(None), "runtime_failure");
        assert_eq!(
            classify_crash_detail(Some(
                "fallback after generation launch failed: worker sess-1 exited before readiness with exit status: 1"
            )),
            "worker_startup_failure"
        );
    }

    #[test]
    fn only_explicit_control_plane_edges_are_expected_interruptions() {
        assert_eq!(
            classify_interruption_detail(Some("controller restart during deploy")),
            "expected_interruption"
        );
        assert_eq!(
            classify_interruption_detail(Some("force cancel watchdog fired")),
            "runtime_interruption"
        );
    }

    #[test]
    fn session_errors_retain_provider_and_transport_classifications() {
        assert_eq!(
            classify_session_error(
                "API Error: 400 This model's maximum context length is 1048576 tokens"
            ),
            "provider_context_limit"
        );
        assert_eq!(
            classify_session_error("socket connection timed out"),
            "transport_failure"
        );
        assert_eq!(
            classify_session_error(
                "Error running remote compact task: stream disconnected before completion: Transport error: network error: error decoding response body"
            ),
            "transport_failure"
        );
        assert_eq!(
            classify_session_error("runtime rejected command"),
            "session_command_error"
        );
        assert_eq!(
            classify_session_error("worker sess-1 exited before readiness with exit status: 1"),
            "worker_startup_failure"
        );
        assert_eq!(session_error_severity("worker_startup_failure"), "critical");
        assert_eq!(session_error_severity("process_exit"), "critical");
        assert_eq!(session_error_severity("session_command_error"), "error");
    }

    #[test]
    fn incident_summaries_are_bounded_on_utf8_boundaries() {
        let (summary, truncated) = bounded_incident_summary(&"你".repeat(2_000));
        assert!(truncated);
        assert!(summary.len() <= 4 * 1024);
        assert_eq!(summary, "你".repeat(summary.chars().count()));
    }
}

async fn retry_store_write(store: &Store, write: &StoreWrite) -> bool {
    let mut last_error = None;
    for attempt in 0..4 {
        match apply_store_write(store, write).await {
            Ok(()) => return true,
            Err(e) => {
                last_error = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(50 * (1 << attempt))).await;
            }
        }
    }
    if let Some(error) = last_error {
        tracing::error!(%error, ?write, "store writer exhausted intent retries");
    }
    false
}

async fn apply_store_write(store: &Store, write: &StoreWrite) -> anyhow::Result<()> {
    match write {
        StoreWrite::InsertSession(meta) => store.insert_session(meta).await,
        StoreWrite::AppendEvent(_) => Ok(()),
        StoreWrite::UpdateStatus { session_id, status } => {
            store.update_status(session_id, *status).await
        }
        StoreWrite::RecordSessionError {
            id,
            session_id,
            occurred_at_ms,
            message,
        } => {
            let classification = classify_session_error(message);
            let fingerprint = format!(
                "{:x}",
                Sha256::digest(format!("{classification}:{message}").as_bytes())
            );
            store
                .upsert_runtime_incident(&crate::store::RuntimeIncidentWrite {
                    id: id.clone(),
                    occurred_at_ms: *occurred_at_ms,
                    source: "controller".to_owned(),
                    classification: classification.to_owned(),
                    severity: session_error_severity(classification).to_owned(),
                    state: "failed".to_owned(),
                    summary: message.clone(),
                    fingerprint,
                    session_id: Some(session_id.clone()),
                    client_id: None,
                    machine_id: None,
                    trace_id: None,
                    build: Some(env!("CARGO_PKG_VERSION").to_owned()),
                    evidence_start_ms: occurred_at_ms.saturating_sub(30_000),
                    evidence_end_ms: occurred_at_ms.saturating_add(30_000),
                    detail: serde_json::json!({ "source": "session_error" }),
                })
                .await
        }
        StoreWrite::UpdateTitle { session_id, title } => {
            store.update_title(session_id, title).await
        }
        StoreWrite::UpdateCwd {
            session_id,
            cwd,
            title,
        } => store.update_cwd(session_id, cwd, title.as_deref()).await,
        StoreWrite::SetAgentSessionId {
            session_id,
            agent_session_id,
        } => {
            store
                .update_agent_session_id(session_id, agent_session_id.as_deref())
                .await
        }
        StoreWrite::UpdateProviderAuthGeneration {
            session_id,
            provider_auth_generation,
        } => {
            store
                .update_provider_auth_generation(session_id, *provider_auth_generation)
                .await
        }
        StoreWrite::UpdateConfigOptions {
            session_id,
            options,
        } => store.update_config_options(session_id, options).await,
        StoreWrite::UpdateConfigPreferences {
            session_id,
            preferences,
        } => {
            store
                .update_config_preferences(session_id, preferences)
                .await
        }
        StoreWrite::ClearEvents { session_id } => store.clear_events(session_id).await,
        StoreWrite::DeleteSession(id) => store.delete_session(id).await,
        StoreWrite::UpdatePending {
            session_id,
            queue,
            drafts,
        } => store.update_pending(session_id, queue, drafts).await,
        StoreWrite::UpdateSessionOrder { order } => store.update_session_order(order).await,
        StoreWrite::UpdateMobileReviewState { session_id, value } => {
            store.update_mobile_review_state(session_id, value).await
        }
        StoreWrite::UpsertWakeup {
            session_id,
            fire_at_ms,
            prompt,
        } => store.upsert_wakeup(session_id, *fire_at_ms, prompt).await,
        StoreWrite::DeleteWakeup { session_id } => store.delete_wakeup(session_id).await,
        StoreWrite::PutSetting { key, value } => store.put_setting(key, value).await,
    }
}

#[cfg(test)]
mod store_writer_tests {
    use super::*;
    use crate::core::Event;

    fn update(seq: u64, value: serde_json::Value) -> Envelope {
        Envelope {
            session_id: "sess-test".to_owned(),
            seq,
            event: Event::Update { update: value },
            cmid: None,
        }
    }

    #[test]
    fn reducer_coalesces_text_at_the_first_seq() {
        let mut reducer = EventReducer::default();
        let first = reducer
            .reduce(update(
                10,
                serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "messageId": "m1",
                    "content": {"type": "text", "text": "hello "}
                }),
            ))
            .expect("first chunk persists");
        assert_eq!(first.seq, 10);
        let joined = reducer
            .reduce(update(
                11,
                serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "messageId": "m1",
                    "content": {"type": "text", "text": "world"}
                }),
            ))
            .expect("second chunk updates canonical row");
        assert_eq!(joined.seq, 10);
        let Event::Update { update } = joined.event else {
            panic!("update")
        };
        assert_eq!(update["content"]["text"], "hello world");
    }

    #[test]
    fn reducer_folds_tool_updates_into_the_original_call() {
        let mut reducer = EventReducer::default();
        reducer.reduce(update(
            20,
            serde_json::json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-1",
                "title": "run",
                "status": "pending"
            }),
        ));
        let folded = reducer
            .reduce(update(
                21,
                serde_json::json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "tool-1",
                    "status": "completed",
                    "content": [{"type": "text", "text": "ok"}]
                }),
            ))
            .expect("tool update folds");
        assert_eq!(folded.seq, 20);
        let Event::Update { update } = folded.event else {
            panic!("update")
        };
        assert_eq!(update["sessionUpdate"], "tool_call");
        assert_eq!(update["status"], "completed");
        assert_eq!(update["title"], "run");
    }

    #[test]
    fn reducer_drops_transient_frames() {
        let mut reducer = EventReducer::default();
        assert!(
            reducer
                .reduce(update(
                    30,
                    serde_json::json!({"sessionUpdate": "usage_update", "used": 1, "size": 2}),
                ))
                .is_none()
        );
    }
}

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

fn force_cancel_with_watchdog(state: &AppState, session_id: &str) -> Result<(), String> {
    let Some(cancelled_revision @ (Status::Busy | Status::Starting, _)) =
        state.hub.status_revision(session_id)
    else {
        return Ok(());
    };
    state.supervisor.send(session_id, AgentCommand::Cancel)?;
    let hub = state.hub.clone();
    let supervisor = Arc::clone(&state.supervisor);
    let session_id = session_id.to_owned();
    tokio::spawn(async move {
        tokio::time::sleep(FORCE_CANCEL_GRACE).await;
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
            grace_seconds = FORCE_CANCEL_GRACE.as_secs(),
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

/// Drain the Hub→dispatcher channel: each [`DispatchReq`] is a queued prompt the
/// Hub decided is ready to send. We forward it to the session's agent. On
/// success, derive the auto-title from the first prompt (a no-op after the
/// first); on failure, clear the in-flight guard (so the queue can keep
/// draining) and surface the error to every client.
async fn run_dispatcher(
    hub: Hub,
    supervisor: Arc<Supervisor>,
    plugin_lifecycle_fences: PluginLifecycleFences,
    mut rx: mpsc::Receiver<DispatchReq>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        let req = tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    rx.close();
                }
                rx.recv().await
            }
            req = rx.recv() => req,
        };
        let Some(req) = req else { break };
        let DispatchReq {
            session_id,
            text,
            content,
            cmid,
        } = req;
        if plugin_fence_state_for_session(&hub, &plugin_lifecycle_fences, &session_id)
            .is_some_and(|state| state != PluginFenceState::Installing)
        {
            hub.requeue_prompt(&session_id, text, content, cmid);
            continue;
        }
        let Some(blocks) = build_prompt_blocks(&text, &content) else {
            tracing::warn!(session = %session_id, "queued prompt had no content; dropping");
            hub.clear_in_flight(&session_id);
            continue;
        };
        let title = first_prompt_title(&text, &content);
        match supervisor.send(
            &session_id,
            AgentCommand::Prompt(blocks, cmid.clone(), None),
        ) {
            Ok(()) => {
                if let Some(t) = title {
                    hub.auto_title(&session_id, t);
                }
            }
            Err(e) => {
                tracing::warn!(session = %session_id, error = %e, "queued dispatch failed");
                retain_failed_dispatch(&hub, session_id, text, content, cmid, &e.to_string());
            }
        }
    }
    tracing::info!("dispatcher shutting down (channel closed)");
}

/// Restore a prompt that left the durable queue but never reached its runtime.
///
/// A remote machine can disconnect between queue drain and dispatch (notably
/// while the controller is restarting). Keeping the original `cmid` makes the
/// restore idempotent and lets every client reconcile the same queued item.
fn retain_failed_dispatch(
    hub: &Hub,
    session_id: String,
    text: String,
    content: Vec<serde_json::Value>,
    cmid: Option<String>,
    error: &str,
) {
    hub.requeue_prompt(&session_id, text, content, cmid);
    hub.broadcast_error(Some(session_id), format!("send failed: {error}"));
}

#[cfg(test)]
mod dispatcher_failure_tests {
    use super::*;

    #[test]
    fn failed_remote_dispatch_retains_the_original_prompt() {
        let hub = Hub::new();
        hub.create_local_session(
            "remote-session".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "Remote session".to_owned(),
            crate::core::SessionOrigin::Web,
            false,
        );

        retain_failed_dispatch(
            &hub,
            "remote-session".to_owned(),
            "current status?".to_owned(),
            Vec::new(),
            Some("client-message-1".to_owned()),
            "machine is not connected",
        );

        assert_eq!(hub.session_info("remote-session").unwrap().queue_count, 1);
        let Some(Outbound::SyncPatch { value, .. }) = hub.queue_resync("remote-session") else {
            panic!("queued prompt should be available to reconnecting clients");
        };
        assert_eq!(value["queue"][0]["text"], "current status?");
        assert_eq!(value["queue"][0]["cmid"], "client-message-1");
    }
}

fn plugin_fence_state_for_session(
    hub: &Hub,
    fences: &PluginLifecycleFences,
    session_id: &str,
) -> Option<PluginFenceState> {
    let key = provider_fence_key_for_session(hub, session_id)?;
    fences.read().get(&key).copied()
}

fn provider_fence_key_for_session(hub: &Hub, session_id: &str) -> Option<(String, String)> {
    hub.session_list()
        .into_iter()
        .find(|session| session.id == session_id)
        .map(|session| (session.machine_id, session.provider))
}

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

fn product_auth_router(state: ProductAuthState) -> Router {
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
        .route("/api/auth/logout", post(api_auth_logout))
        .route("/api/auth/me", get(api_auth_me))
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
        .route("/api/auth/device/refresh", post(api_auth_device_refresh))
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

fn registration_policy(hub: &Hub) -> crate::admin::RegistrationPolicy {
    crate::admin::RegistrationPolicy::from_setting(
        hub.settings_snapshot()
            .get(crate::admin::REGISTRATION_SETTING),
    )
}

fn permission_policy(hub: &Hub) -> crate::admin::PermissionPolicy {
    crate::admin::PermissionPolicy::from_setting(
        hub.settings_snapshot()
            .get(crate::admin::PERMISSIONS_SETTING),
    )
}

fn admin_identities(hub: &Hub) -> crate::admin::AdminIdentities {
    crate::admin::AdminIdentities::from_setting(
        hub.settings_snapshot()
            .get(crate::admin::ADMIN_IDENTITIES_SETTING),
    )
}

fn persist_admin_identities(hub: &Hub, identities: &crate::admin::AdminIdentities) {
    persist_admin_setting(hub, crate::admin::ADMIN_IDENTITIES_SETTING, identities);
}

#[allow(dead_code)]
fn persist_registration_policy(hub: &Hub, policy: &crate::admin::RegistrationPolicy) {
    persist_admin_setting(hub, crate::admin::REGISTRATION_SETTING, policy);
}

fn persist_admin_setting(hub: &Hub, key: &str, value: &impl Serialize) {
    hub.set_setting(
        key.to_owned(),
        serde_json::to_value(value).unwrap_or_else(|_| serde_json::json!({})),
    );
}

async fn enrich_admin_status(
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

async fn admin_session_response(
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

async fn api_admin_auth(
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

async fn api_admin_setup(
    State(state): State<ProductAuthState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::admin::AdminSetupRequest>,
) -> Response {
    let _ = (state, peer, headers, request);
    (StatusCode::FORBIDDEN, "complete setup on /").into_response()
}

async fn api_admin_bootstrap(
    State(state): State<ProductAuthState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::admin::AdminCredentials>,
) -> Response {
    let _ = (state, peer, headers, request);
    (StatusCode::FORBIDDEN, "complete setup on /").into_response()
}

async fn api_admin_login(
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

async fn api_admin_logout(State(state): State<ProductAuthState>, headers: HeaderMap) -> Response {
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

async fn api_admin_accounts(State(state): State<ProductAuthState>, headers: HeaderMap) -> Response {
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

async fn api_admin_registration(
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

async fn api_admin_create_account(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::admin::AdminCredentials>,
) -> Response {
    let _ = (state, peer, headers, request);
    (StatusCode::FORBIDDEN, "this instance is single-user").into_response()
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

async fn api_admin_overview(State(state): State<ProductAuthState>, headers: HeaderMap) -> Response {
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
            .storage_metrics()
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

async fn api_admin_sessions(State(state): State<ProductAuthState>, headers: HeaderMap) -> Response {
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

async fn api_admin_machines(State(state): State<ProductAuthState>, headers: HeaderMap) -> Response {
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

async fn api_admin_registration_put(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(patch): Json<crate::admin::RegistrationPolicyPatch>,
) -> Response {
    let _ = (state, peer, headers, patch);
    (StatusCode::FORBIDDEN, "this instance is single-user").into_response()
}

async fn api_admin_registration_token(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::admin::CreateRegistrationToken>,
) -> Response {
    let _ = (state, peer, headers, request);
    (StatusCode::FORBIDDEN, "this instance is single-user").into_response()
}

async fn api_admin_registration_token_delete(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    Path(token_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let _ = (state, peer, token_id, headers);
    (StatusCode::FORBIDDEN, "this instance is single-user").into_response()
}

async fn api_admin_permissions(
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

async fn api_admin_permissions_put(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(patch): Json<crate::admin::PermissionPolicy>,
) -> Response {
    let _ = (state, peer, headers, patch);
    (StatusCode::FORBIDDEN, "this instance is single-user").into_response()
}

#[allow(dead_code)]
async fn api_admin_permissions_put_continue(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(patch): Json<crate::admin::PermissionPolicy>,
) -> Response {
    let peer = peer_addr(peer);
    if let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins) {
        return rejected;
    }
    if let Err(status) = require_admin(&state, &headers, crate::admin::AdminRole::Owner) {
        return status.into_response();
    }
    let mut policy = permission_policy(&state.hub);
    if let Err(error) = policy.apply_patch(patch) {
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    persist_admin_setting(&state.hub, crate::admin::PERMISSIONS_SETTING, &policy);
    Json(policy).into_response()
}

async fn api_admin_session_limits(
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

async fn api_admin_session_limits_put(
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

async fn api_admin_providers(
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

async fn api_admin_plugins(State(state): State<ProductAuthState>, headers: HeaderMap) -> Response {
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

async fn api_admin_plugins_refresh(
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
    match catalog.refresh_external() {
        Ok(count) => Json(serde_json::json!({ "external_releases": count })).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

async fn api_admin_providers_refresh(
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
    match catalog.refresh_external() {
        Ok(count) => Json(serde_json::json!({ "external_releases": count })).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

fn admin_passkey_subject(account: &str) -> String {
    format!("admin:{account}")
}

async fn api_admin_list_passkeys(
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

async fn api_admin_passkey_register_options(
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

async fn api_admin_passkey_register_complete(
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

async fn api_admin_passkey_assert_options(
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

async fn api_admin_passkey_assert_complete(
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

async fn api_admin_passkey_reauth(
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

async fn api_admin_delete_passkey(
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

fn auth_now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn peer_addr(ConnectInfo(peer): ConnectInfo<SocketAddr>) -> SocketAddr {
    peer
}

fn reject_bad_origin(
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

fn reject_insecure_admin(headers: &HeaderMap, peer: SocketAddr) -> Option<Response> {
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

fn product_me(hub: &Hub, username: &str) -> ProductMe {
    ProductMe {
        account: username.to_owned(),
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

fn product_session_deadlines(
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

fn product_me_for_user_with_policy(
    hub: &Hub,
    authentication: &crate::auth_plugins::ProductAuthentication,
    user: &crate::store::ProductUser,
    session: Option<&crate::store::ProductUserSession>,
    policy: &crate::passkey::PasskeyPolicy,
) -> ProductMe {
    let mut me = product_me(hub, &user.username);
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

async fn product_me_for_user(
    store: Option<&Store>,
    hub: &Hub,
    authentication: &crate::auth_plugins::ProductAuthentication,
    user: &crate::store::ProductUser,
    session: Option<&crate::store::ProductUserSession>,
) -> anyhow::Result<ProductMe> {
    let Some(store) = store else {
        return Ok(product_me(hub, &user.username));
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

fn product_session_policy_unavailable(error: anyhow::Error) -> Response {
    tracing::error!(%error, "product_session_policy_unavailable");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "session protection is temporarily unavailable",
    )
        .into_response()
}

async fn apply_rate_limit(state: &ProductAuthState, username: &str, ip: &str) {
    if let Some(delay) = state.rate_limits.delay(username, ip) {
        tokio::time::sleep(delay).await;
    }
}

async fn product_user_from_cookie(
    state: &ProductAuthState,
    headers: &HeaderMap,
) -> Option<crate::store::ProductUser> {
    product_user_from_store_cookie(state.store.as_ref()?, headers).await
}

async fn product_session_and_user_from_cookie(
    state: &ProductAuthState,
    headers: &HeaderMap,
) -> Option<(crate::store::ProductUserSession, crate::store::ProductUser)> {
    product_session_and_user_from_store_cookie(state.store.as_ref()?, headers).await
}

fn require_admin(
    state: &ProductAuthState,
    headers: &HeaderMap,
    minimum: crate::admin::AdminRole,
) -> Result<crate::admin::AdminPrincipal, StatusCode> {
    require_admin_role(&state.hub, headers, minimum)
}

fn require_admin_role(
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

fn classify_route(method: &Method, path: &str) -> RouteAuth {
    if matches!(path, "/healthz" | "/version")
        || path.starts_with("/plugin-artifacts/")
        || path.starts_with("/provider-artifacts/")
        || !path.starts_with("/api/") && path != "/ws" && path != "/metrics"
    {
        return RouteAuth::Public;
    }
    if path == "/metrics" {
        return RouteAuth::MetricsScrape;
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
    if path.starts_with("/api/auth/tokens/") {
        return RouteAuth::Product;
    }
    if matches!(
        path,
        "/api/auth/device/authorizations"
            | "/api/auth/device/authorizations/inspect"
            | "/api/auth/device/authorizations/events"
            | "/api/auth/device/exchange"
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
    // Deployment health checks use refresh to ask a connected Machine to
    // republish its read-only inventory after a declarative workspace update.
    // It does not install, revoke, reconcile, or otherwise mutate the Machine.
    if method == Method::POST && path.starts_with("/api/machines/") && path.ends_with("/refresh") {
        return RouteAuth::Public;
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
    if path == "/api/sessions" && method == Method::POST {
        return RouteAuth::ProductOperator;
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
    if matches!(
        path,
        "/api/usage"
            | "/api/usage/logs"
            | "/api/usage/deepseek/activity"
            | "/api/workspaces"
            | "/api/plugins"
            | "/api/providers"
            | "/api/machines"
            | "/api/observability/batches"
    ) {
        return RouteAuth::Product;
    }
    if path.starts_with("/api/usage/")
        && (path.ends_with("/reset") || path.ends_with("/reset/schedule"))
    {
        return RouteAuth::AdminOperator;
    }
    if path.starts_with("/api/usage/") {
        return RouteAuth::Product;
    }
    if path.starts_with("/api/machines/")
        && (path.ends_with("/events") || path.ends_with("/plugins") || path.ends_with("/providers"))
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
        && (path.ends_with("/refresh")
            || path.ends_with("/components/reconcile")
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

fn provider_auth_path(path: &str) -> bool {
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

fn session_id_from_path(path: &str) -> Option<&str> {
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

fn session_id_from_sync_state(state: &str) -> Option<&str> {
    state
        .strip_prefix("queue:")
        .or_else(|| state.strip_prefix("mobile-review:"))
        .filter(|id| !id.is_empty())
}

async fn resolve_product_principal(
    product_auth_enabled: bool,
    store: Option<&Store>,
    hub: &Hub,
    headers: &HeaderMap,
) -> Option<ProductPrincipal> {
    resolve_product_request_principal(product_auth_enabled, store, hub, headers)
        .await
        .map(|(principal, _)| principal)
}

async fn resolve_product_request_principal(
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

async fn resolve_product_api_request_principal(
    state: &AppState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<Option<AuthenticatedProductRequest>, ()> {
    if !state.product_auth_enabled {
        return Ok(Some(AuthenticatedProductRequest {
            principal: crate::product_auth::local_product_principal(),
            cookie_session: None,
            device_identity: None,
        }));
    }
    let bearer = crate::product_auth::bearer_token(headers);
    if bearer
        .as_deref()
        .is_some_and(|token| token.starts_with(crate::client_auth::ACCESS_TOKEN_PREFIX))
    {
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
        let Some(store) = state.store.as_ref() else {
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
        let store = store.clone();
        let device_id = identity.device_id.clone();
        tokio::spawn(async move {
            let _ = store
                .touch_user_device_last_used(&device_id, auth_now_ms())
                .await;
        });
        return Ok(Some(AuthenticatedProductRequest {
            principal: product_principal(&state.hub, &user),
            cookie_session: None,
            device_identity: Some(identity),
        }));
    }
    Ok(resolve_product_request_principal(
        state.product_auth_enabled,
        state.store.as_ref(),
        &state.hub,
        headers,
    )
    .await
    .map(|(principal, cookie_session)| AuthenticatedProductRequest {
        principal,
        cookie_session,
        device_identity: None,
    }))
}

#[derive(Clone)]
struct AuthenticatedProductRequest {
    principal: ProductPrincipal,
    cookie_session: Option<crate::store::ProductUserSession>,
    device_identity: Option<crate::client_auth::DeviceAccessIdentity>,
}

async fn product_user_from_store_cookie(
    store: &Store,
    headers: &HeaderMap,
) -> Option<crate::store::ProductUser> {
    product_session_and_user_from_store_cookie(store, headers)
        .await
        .map(|(_, user)| user)
}

async fn product_session_and_user_from_store_cookie(
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

async fn ensure_product_session_fresh(
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

async fn product_from_bearer(store: &Store, hub: &Hub, token: &str) -> Option<ProductPrincipal> {
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

fn product_principal(hub: &Hub, user: &crate::store::ProductUser) -> ProductPrincipal {
    ProductPrincipal {
        user_id: user.id.clone(),
        username: user.username.clone(),
        role: permission_policy(hub).role_for(&user.username),
    }
}

fn visible_session_ids(
    hub: &Hub,
    principal: &ProductPrincipal,
) -> std::collections::HashSet<String> {
    hub.session_list_filtered(|owner| principal.can_see(owner))
        .into_iter()
        .map(|meta| meta.id)
        .collect()
}

fn session_is_visible(hub: &Hub, principal: &ProductPrincipal, session_id: &str) -> bool {
    hub.session_info(session_id).is_some()
        && (principal.sees_every_session()
            || hub.session_owner_user_id(session_id).is_none()
            || hub.owned_by_product_user(session_id, &principal.user_id))
}

fn authorize_ws_upgrade(
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

async fn enforce_product_api(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
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
        &state,
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
    if let (Some(store), Some(session)) = (state.store.as_ref(), cookie_session.as_ref())
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
            if let Some(session_id) = session_id_from_path(&path) {
                match state.hub.session_info(session_id) {
                    None => {}
                    Some(info) => {
                        let owner = info.meta.owner_user_id.as_deref();
                        if !principal.can_see(owner) {
                            return StatusCode::NOT_FOUND.into_response();
                        }
                        if matches!(
                            classify_route(&method, &path),
                            RouteAuth::ProductSessionMutate
                        ) && !principal.can_mutate(owner)
                        {
                            return StatusCode::FORBIDDEN.into_response();
                        }
                    }
                }
            }
        }
        _ => {}
    }
    request.extensions_mut().insert(authenticated);
    next.run(request).await
}

async fn issue_product_session(
    state: &ProductAuthState,
    store: &Store,
    user: &crate::store::ProductUser,
    headers: &HeaderMap,
    auth_method: &str,
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
    }
    let token = match crate::product_auth::new_session_token() {
        Ok(token) => token,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    let now = auth_now_ms();
    let session_ttl_ms = state.product_authentication.session.primary_max_age_ms;
    let session = crate::store::ProductUserSession {
        token_hash: crate::admin::hex_sha256(token.as_bytes()),
        user_id: user.id.clone(),
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
    };
    let persisted = if let (Some(previous), Some(previous_token_hash)) =
        (previous_session.as_ref(), previous_token_hash.as_deref())
    {
        debug_assert_eq!(previous.user_id, session.user_id);
        store
            .replace_user_session(previous_token_hash, &session)
            .await
    } else {
        store.insert_user_session(&session).await
    };
    if let Err(error) = persisted {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
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

async fn hash_product_password(password: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || crate::product_auth::hash_password(&password))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

fn user_json(hub: &Hub, user: &crate::store::ProductUser) -> serde_json::Value {
    serde_json::json!({
        "id": user.id,
        "username": user.username,
        "created_at_ms": user.created_at_ms,
        "updated_at_ms": user.updated_at_ms,
        "disabled_at_ms": user.disabled_at_ms,
        "role": permission_policy(hub).role_for(&user.username),
    })
}

async fn instance_needs_setup(state: &ProductAuthState) -> bool {
    match durable_store(&state.store) {
        Some(store) => store
            .list_users()
            .await
            .map(|users| users.is_empty())
            .unwrap_or(true),
        None => true,
    }
}

async fn api_auth_status(State(state): State<ProductAuthState>, headers: HeaderMap) -> Response {
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
        "providers": state.product_authentication.public_providers(),
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

async fn api_auth_setup(
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

async fn api_auth_register(
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
    )
    .await;
    let secure = crate::product_auth::request_is_https(&headers);
    if let Ok(value) = crate::admin::clear_setup_cookie(secure).parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

async fn api_auth_login(
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
    )
    .await
}

async fn api_auth_oidc_start(
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

async fn api_auth_oidc_callback(
    State(state): State<ProductAuthState>,
    uri: Uri,
    headers: HeaderMap,
    Query(query): Query<OidcCallbackQuery>,
) -> Response {
    api_auth_oidc_callback_inner(state, uri, headers, query).await
}

async fn api_auth_oidc_callback_form(
    State(state): State<ProductAuthState>,
    uri: Uri,
    headers: HeaderMap,
    Form(query): Form<OidcCallbackQuery>,
) -> Response {
    api_auth_oidc_callback_inner(state, uri, headers, query).await
}

async fn api_auth_oidc_callback_inner(
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
            let handoff =
                match state
                    .oidc_native_handoffs
                    .issue(provider_id, &user.id, code_challenge)
                {
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
    let mut response = issue_product_session(&state, &store, &user, &headers, provider_id).await;
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

fn oidc_callback_target_error(
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

fn oidc_browser_shell_completion_response(secure: bool) -> Response {
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

fn oidc_admin_session_cookie(
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

async fn api_auth_oidc_native_exchange(
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
    let user_id =
        match state
            .oidc_native_handoffs
            .consume(provider_id, &request.code, &request.code_verifier)
        {
            Ok(user_id) => user_id,
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
    let mut response = issue_product_session(&state, &store, &user, &headers, provider_id).await;
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

const OIDC_NATIVE_EVENTS_HANDSHAKE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(10);
const OIDC_NATIVE_EVENTS_TTL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const OIDC_NATIVE_EVENTS_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(25);

async fn api_auth_oidc_native_events(
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

async fn handle_oidc_native_events(
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

async fn api_auth_oidc_native_poll(
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
    let user_id = match state.oidc_native_handoffs.poll_browser(
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
        Ok(crate::oidc::BrowserHandoffPoll::Ready { user_id }) => user_id,
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
    let mut response = issue_product_session(&state, &store, &user, &headers, provider_id).await;
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

async fn api_auth_oidc_native_cancel(
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

async fn api_auth_oidc_native_complete() -> Response {
    const PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Return to Cowboy</title>
  <style>
    :root { color-scheme: light dark; font-family: -apple-system, BlinkMacSystemFont, sans-serif; }
    body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: Canvas; color: CanvasText; }
    main { max-width: 30rem; padding: 2rem; text-align: center; }
    h1 { font-size: 1.5rem; }
    p { line-height: 1.5; opacity: .72; }
  </style>
</head>
<body><main><h1>Return to Cowboy</h1><p>Cowboy will show the authorization result and close this window, or you can close it now.</p></main></body>
</html>"#;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .header("referrer-policy", "no-referrer")
        .header("x-content-type-options", "nosniff")
        .header("x-frame-options", "DENY")
        .header(
            "content-security-policy",
            "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        )
        .body(Body::from(PAGE))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn oidc_callback_error(status: StatusCode, secure: bool) -> Response {
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

fn authentication_provider_id<'a>(path: &'a str, suffix: &str) -> Option<&'a str> {
    let middle = path.strip_prefix("/api/auth/providers/")?;
    let provider_id = middle.strip_suffix(&format!("/{suffix}"))?;
    (!provider_id.is_empty()
        && !provider_id.contains('/')
        && provider_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
    .then_some(provider_id)
}

async fn api_auth_logout(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let peer = peer_addr(peer);
    if crate::product_auth::user_cookie_token(&headers).is_some()
        && let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins)
    {
        return rejected;
    }
    if let (Some(store), Some(token)) = (
        state.store.as_ref(),
        crate::product_auth::user_cookie_token(&headers),
    ) && let Ok(Some(session)) = store
        .user_session_by_token_hash(&crate::admin::hex_sha256(token.as_bytes()))
        .await
    {
        let _ = store.delete_user_session(&session.token_hash).await;
        tracing::info!(user_id = session.user_id, "product_logout");
    }
    (
        [(
            header::SET_COOKIE,
            crate::product_auth::clear_session_cookie(crate::product_auth::request_is_https(
                &headers,
            )),
        )],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

async fn api_auth_me(State(state): State<ProductAuthState>, headers: HeaderMap) -> Response {
    if !state.product_auth_enabled {
        return Json(serde_json::json!({
            "account": "local",
            "role": "owner",
            "auth_enabled": false,
        }))
        .into_response();
    }
    let cookie_session = if crate::product_auth::bearer_token(&headers).is_none() {
        product_session_and_user_from_cookie(&state, &headers)
            .await
            .map(|(session, _)| session)
    } else {
        None
    };
    let Some(principal) = resolve_product_principal(
        state.product_auth_enabled,
        state.store.as_ref(),
        &state.hub,
        &headers,
    )
    .await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
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
            Ok(me) => Json(me).into_response(),
            Err(error) => product_session_policy_unavailable(error),
        },
        None => Json(product_me(&state.hub, &principal.username)).into_response(),
    }
}

async fn require_product_user(
    state: &ProductAuthState,
    headers: &HeaderMap,
) -> Result<crate::store::ProductUser, Response> {
    product_user_from_cookie(state, headers)
        .await
        .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())
}

async fn require_fresh_product_user(
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

const PASSKEY_MANAGEMENT_STEP_UP_MAX_AGE_MS: i64 = 5 * 60 * 1_000;

fn product_session_has_recent_step_up(
    session: &crate::store::ProductUserSession,
    now_ms: i64,
) -> bool {
    let verified_at = session
        .passkey_verified_at_ms
        .unwrap_or(session.created_at_ms);
    verified_at <= now_ms.saturating_add(60_000)
        && verified_at.saturating_add(PASSKEY_MANAGEMENT_STEP_UP_MAX_AGE_MS) > now_ms
}

async fn require_recent_product_user(
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

async fn api_auth_list_passkeys(
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

async fn api_auth_passkey_register_options(
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

async fn api_auth_passkey_register_complete(
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

async fn persist_product_passkey_registration(
    store: &Store,
    user: &crate::store::ProductUser,
    nickname: String,
    credential_id: String,
    passkey_json: String,
) -> Result<crate::passkey::PasskeyView, Response> {
    let now = auth_now_ms();
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

async fn api_auth_passkey_assert_options(
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

async fn api_auth_passkey_assert_complete(
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

struct ProductPasskeyAssertionResult {
    me: ProductMe,
    set_cookie: Option<String>,
}

async fn persist_product_passkey_assertion(
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
            user_id: user.id.clone(),
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

fn product_passkey_assertion_response(result: ProductPasskeyAssertionResult) -> Response {
    match result.set_cookie {
        Some(cookie) => ([(header::SET_COOKIE, cookie)], Json(result.me)).into_response(),
        None => Json(result.me).into_response(),
    }
}

async fn api_auth_passkey_external_start(
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
                    session_token_hash: &session.token_hash,
                    code_challenge: &request.code_challenge,
                },
                &existing,
                &webauthn,
            )
        }
    };
    match transaction_id {
        Ok(transaction_id) => Json(serde_json::json!({
            "transaction_id": transaction_id,
            "expires_in_seconds": crate::passkey::EXTERNAL_CEREMONY_TTL_SECS,
        }))
        .into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

async fn api_auth_passkey_external_options(
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
        Err(_) => (StatusCode::GONE, "Passkey setup expired").into_response(),
    }
}

async fn api_auth_passkey_external_complete(
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
    let (action, user_id) = match state.passkeys.external_subject(&request.transaction_id) {
        Ok(subject) => subject,
        Err(_) => return (StatusCode::GONE, "Passkey setup expired").into_response(),
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
        Ok(()) => Json(serde_json::json!({ "status": "complete" })).into_response(),
        Err(error) => {
            tracing::warn!(%error, "external_passkey_rejected");
            (StatusCode::UNAUTHORIZED, "Passkey verification failed").into_response()
        }
    }
}

async fn api_auth_passkey_external_fail(
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
    match state.passkeys.fail_external(&request.transaction_id) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(_) => (StatusCode::GONE, "Passkey setup expired").into_response(),
    }
}

const PASSKEY_EXTERNAL_EVENTS_HANDSHAKE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(10);
const PASSKEY_EXTERNAL_EVENTS_TTL: std::time::Duration = std::time::Duration::from_secs(2 * 60);
const PASSKEY_EXTERNAL_EVENTS_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(25);

async fn api_auth_passkey_external_events(
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
            handle_passkey_external_events(socket, state, user.id, session.token_hash)
        })
}

async fn handle_passkey_external_events(
    mut socket: WebSocket,
    state: ProductAuthState,
    user_id: String,
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
    let mut events = match state.passkeys.subscribe_external(
        &request.transaction_id,
        &user_id,
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

async fn api_auth_passkey_external_finalize(
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
    let outcome = match state.passkeys.finalize_external(
        &request.transaction_id,
        &user.id,
        &session.token_hash,
        &request.code_verifier,
    ) {
        Ok(outcome) => outcome,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                "Passkey setup expired or did not complete",
            )
                .into_response();
        }
    };
    match outcome {
        crate::passkey::ExternalFinalizeResult::Pending => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "status": "pending" })),
        )
            .into_response(),
        crate::passkey::ExternalFinalizeResult::Failed => {
            (StatusCode::BAD_REQUEST, "Passkey setup was cancelled").into_response()
        }
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
            Ok(passkey) => Json(serde_json::json!({
                "status": "complete",
                "passkey": passkey,
            }))
            .into_response(),
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
                let body = Json(serde_json::json!({
                    "status": "complete",
                    "me": result.me,
                }));
                match result.set_cookie {
                    Some(cookie) => ([(header::SET_COOKIE, cookie)], body).into_response(),
                    None => body.into_response(),
                }
            }
            Err(response) => response,
        },
    }
}

async fn api_auth_passkey_reauth(
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

async fn api_auth_delete_passkey(
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

fn device_authorization_error(error: &anyhow::Error) -> Response {
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

fn device_verification_url(
    public_origins: &[String],
    request_id: &str,
    approval_token: &str,
) -> String {
    let path = format!("/auth/device#request_id={request_id}&approval_token={approval_token}");
    public_origins
        .first()
        .map_or(path.clone(), |origin| format!("{origin}{path}"))
}

async fn api_auth_device_authorization_start(
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

async fn api_auth_device_authorization_inspect(
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

async fn api_auth_device_authorization_approve(
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

async fn api_auth_device_authorization_deny(
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

async fn api_auth_device_authorization_events(
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

async fn handle_device_authorization_events(mut socket: WebSocket, state: ProductAuthState) {
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

async fn api_auth_device_exchange(
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
    if let Err(error) = store.insert_user_device(&device, &refresh).await {
        finish_failed();
        tracing::error!(%error, "device_authorization_persist");
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
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

fn refresh_bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?
        .trim();
    token
        .starts_with(crate::client_auth::REFRESH_TOKEN_PREFIX)
        .then(|| token.to_owned())
}

async fn revoke_replayed_device(
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

async fn api_auth_device_refresh(
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

fn device_public_json(device: crate::store::ProductDevice) -> serde_json::Value {
    serde_json::json!({
        "id": device.id,
        "name": device.name,
        "created_at_ms": device.created_at_ms,
        "last_used_at_ms": device.last_used_at_ms,
    })
}

async fn api_auth_list_devices(
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

async fn api_auth_delete_device(
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

fn token_public_json(token: &crate::store::ProductApiToken) -> serde_json::Value {
    serde_json::json!({
        "id": token.id,
        "name": token.name,
        "token_prefix": token.token_prefix,
        "created_at_ms": token.created_at_ms,
        "expires_at_ms": token.expires_at_ms,
        "last_used_at_ms": token.last_used_at_ms,
    })
}

async fn api_auth_list_tokens(
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

async fn api_auth_create_token(
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

async fn api_auth_delete_token(
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

async fn api_admin_users(State(state): State<ProductAuthState>, headers: HeaderMap) -> Response {
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

async fn api_admin_create_user(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<AdminCreateUserRequest>,
) -> Response {
    let _ = (state, peer, headers, request);
    (StatusCode::FORBIDDEN, "this instance is single-user").into_response()
}

async fn api_admin_disable_user(
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

async fn api_admin_set_password(
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

async fn serve_axum(
    bind: std::net::SocketAddr,
    data_dir: PathBuf,
    state: AppState,
    shutdown_tx: watch::Sender<bool>,
) -> anyhow::Result<()> {
    let state = Arc::new(state);
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
    let auth_state = ProductAuthState {
        hub: state.hub.clone(),
        store: state.store.clone(),
        rate_limits: Arc::new(crate::product_auth::AuthRateLimiter::default()),
        public_origins: state.public_origins.clone(),
        runtime_health: Some(state.runtime_health.clone()),
        persistence_health: state.persistence_health.clone(),
        runtime_router: Some(state.runtime_router.clone()),
        plugin_catalog: Some(state.plugin_catalog.clone()),
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
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .route("/api/metrics", get(api_metrics))
        .route(
            "/api/observability/batches",
            post(api_observability_batch).layer(DefaultBodyLimit::max(256 * 1024)),
        )
        .route(
            "/api/observability/incidents",
            get(api_observability_incidents),
        )
        .route("/api/logs", get(api_diagnostic_logs))
        .route("/api/logs/{id}", get(api_diagnostic_log_detail))
        .route("/api/usage", get(api_usage).post(api_usage_refresh))
        .route(
            "/api/usage/deepseek/activity",
            get(api_deepseek_usage_activity),
        )
        .route("/api/usage/{provider}", post(api_usage_provider_refresh))
        .route("/api/usage/logs", get(api_usage_logs))
        .route("/api/usage/{provider}/reset", post(api_provider_reset))
        .route(
            "/api/usage/{provider}/reset/schedule",
            put(api_provider_reset_schedule).delete(api_provider_reset_cancel),
        )
        .route("/metrics", get(prometheus_metrics))
        .route("/api/workspaces", get(api_workspaces))
        .route("/api/web-push/config", get(api_web_push_config))
        .route(
            "/api/web-push/subscription",
            put(api_web_push_subscribe).delete(api_web_push_unsubscribe),
        )
        .route("/api/providers", get(api_providers))
        .route("/api/plugins", get(api_plugins))
        .route(
            "/api/plugins/catalog/refresh",
            post(api_plugin_catalog_refresh),
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
        .route("/api/sessions", post(api_new_session))
        .route(
            "/api/sessions/reconcile-project",
            post(api_reconcile_project_sessions),
        )
        .route("/api/sessions/{id}/files", get(api_search_files))
        .route("/api/code/sessions/{id}/tree", get(api_file_tree))
        .route("/api/code/sessions/{id}/search", get(api_code_search))
        .route("/api/code/sessions/{id}/manifest", get(api_code_manifest))
        .route("/api/code/sessions/{id}/changes", get(api_code_changes))
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
        .route("/api/sessions/{id}/reload", post(api_session_reload))
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
        .route("/api/sessions/{id}/prompt", post(api_session_prompt))
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
        .layer(middleware::from_fn_with_state(state, enforce_product_api))
        .merge(product_auth_router(auth_state))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(addr = %bind, "WS/HTTP listening");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(shutdown_tx))
    .await
    .context("axum serve")?;
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
    tracing_subscriber::fmt()
        // ACP is newline-delimited JSON-RPC over stdout. A single log line on
        // stdout corrupts the transport, so keep every command's diagnostics
        // on stderr (which systemd and Zed both capture separately).
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

async fn healthz(State(state): State<Arc<AppState>>) -> Response {
    if !state.runtime_health.is_healthy(state.store.is_some()) {
        return (StatusCode::SERVICE_UNAVAILABLE, "background task degraded").into_response();
    }
    if state
        .persistence_health
        .as_ref()
        .is_some_and(|health| !health.is_healthy())
    {
        (StatusCode::SERVICE_UNAVAILABLE, "persistence degraded").into_response()
    } else {
        "ok".into_response()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebPushConfigResponse<'a> {
    application_server_key: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WebPushUnsubscribeRequest {
    endpoint: String,
}

async fn api_web_push_config(State(state): State<Arc<AppState>>) -> Response {
    Json(WebPushConfigResponse {
        application_server_key: state.web_push.public_key(),
    })
    .into_response()
}

async fn api_web_push_subscribe(
    State(state): State<Arc<AppState>>,
    Json(subscription): Json<WebPushSubscription>,
) -> Response {
    match state.web_push.upsert(subscription) {
        Ok(()) => Json(serde_json::json!({ "subscribed": true })).into_response(),
        Err(error) => {
            tracing::warn!(%error, "rejecting Web Push subscription");
            (StatusCode::BAD_REQUEST, "invalid Web Push subscription").into_response()
        }
    }
}

async fn api_web_push_unsubscribe(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WebPushUnsubscribeRequest>,
) -> Response {
    match state.web_push.remove(&request.endpoint) {
        Ok(()) => Json(serde_json::json!({ "subscribed": false })).into_response(),
        Err(error) => {
            tracing::warn!(%error, "removing Web Push subscription");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Response body for `GET /version`.
#[derive(Debug, Serialize)]
struct VersionResponse {
    version: String,
}

/// A build identifier the SPA polls to detect a frontend rollout. `index.html`
/// references the content-hashed JS/CSS bundles, so hashing it keeps `/version`
/// aligned with the files currently exposed through the stable web-root path.
async fn version(State(state): State<Arc<AppState>>) -> Response {
    match tokio::fs::read(state.web_root.join("index.html")).await {
        Ok(bytes) => Json(VersionResponse {
            version: content_hash(&bytes),
        })
        .into_response(),
        Err(error) => {
            tracing::warn!(%error, web_root = %state.web_root.display(), "reading web version failed");
            (StatusCode::NOT_FOUND, "UI not built").into_response()
        }
    }
}

/// Render the first 16 bytes of SHA256 as a compact content identifier. Used
/// by both static-asset ETags and `/version` so the two cannot drift.
fn content_hash(content: &[u8]) -> String {
    let hash: [u8; 32] = Sha256::digest(content).into();
    format!(
        "{:016x}{:016x}",
        u64::from_be_bytes(hash[0..8].try_into().expect("8 bytes")),
        u64::from_be_bytes(hash[8..16].try_into().expect("8 bytes")),
    )
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
    observability_accepted_batches: u64,
    observability_dropped_batches: u64,
    observability_failed_log_batches: u64,
    observability_failed_metric_batches: u64,
    hub_session_count: usize,
    hub_hot_log_bytes: usize,
    hub_broadcast_last_bytes: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct DaemonMemory {
    rss_bytes: u64,
    rss_peak_bytes: u64,
}

/// Current and lifetime-high resident set of THIS process (not its agent or
/// usage-collector children). `/proc/self/status` reports KiB directly, avoiding
/// the incorrect 4 KiB page-size assumption `statm` would make on some hosts.
fn daemon_memory() -> DaemonMemory {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .map_or_else(DaemonMemory::default, |status| {
            parse_proc_status_memory(&status)
        })
}

fn parse_proc_status_memory(status: &str) -> DaemonMemory {
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

#[cfg(test)]
mod daemon_memory_tests {
    use super::parse_proc_status_memory;

    #[test]
    fn proc_status_reports_current_and_peak_bytes() {
        let memory = parse_proc_status_memory(
            "Name:\tcowboy\nVmPeak:\t 999999 kB\nVmHWM:\t 258180 kB\nVmRSS:\t  72372 kB\n",
        );
        assert_eq!(memory.rss_bytes, 72_372 * 1024);
        assert_eq!(memory.rss_peak_bytes, 258_180 * 1024);
    }
}

async fn api_metrics(State(state): State<Arc<AppState>>) -> Response {
    let sessions_live = state.hub.session_list().len();
    let (db_bytes, events_rows, sessions_deleted) = match &state.store {
        Some(s) => s.storage_metrics().await.unwrap_or((0, 0, 0)),
        None => (0, i64::try_from(state.hub.event_total()).unwrap_or(0), 0),
    };
    let runtime = state.runtime_router.stats();
    let code_cache = state.code_cache.metrics();
    let daemon_memory = daemon_memory();
    let cgroup_memory = crate::memory_observability::own_cgroup_memory().unwrap_or_default();
    let hub_memory = state.hub.memory_stats();
    Json(Metrics {
        db_bytes,
        events_rows,
        sessions_live,
        sessions_deleted,
        daemon_rss_bytes: daemon_memory.rss_bytes,
        daemon_rss_peak_bytes: daemon_memory.rss_peak_bytes,
        cgroup_memory_bytes: cgroup_memory.current_bytes,
        cgroup_memory_peak_bytes: cgroup_memory.peak_bytes,
        persistence_pending: state.persistence_health.as_ref().map_or(0, |h| h.pending()),
        persistence_pending_bytes: state
            .persistence_health
            .as_ref()
            .map_or(0, |h| h.pending_bytes()),
        persistence_dropped: state.persistence_health.as_ref().map_or(0, |h| h.dropped()),
        persistence_failed_batches: state
            .persistence_health
            .as_ref()
            .map_or(0, |h| h.failed_batches()),
        persistence_last_error: state
            .persistence_health
            .as_ref()
            .and_then(|h| h.last_error()),
        runtime_connected: state.runtime_router.has_connected_runtime(),
        runtime_workers: runtime.workers,
        runtime_busy_workers: runtime.busy_workers,
        runtime_draining_workers: runtime.draining_workers,
        runtime_handoff_workers: runtime.handoff_workers,
        runtime_pending_commands: runtime.pending_commands,
        code_cache_bytes: code_cache.bytes,
        code_cache_hits: code_cache.hits,
        code_cache_misses: code_cache.misses,
        code_cache_evictions: code_cache.evictions,
        observability_pending: state.observability.health().pending(),
        observability_accepted_batches: state.observability.health().accepted_batches(),
        observability_dropped_batches: state.observability.health().dropped_batches(),
        observability_failed_log_batches: state.observability.health().failed_log_batches(),
        observability_failed_metric_batches: state.observability.health().failed_metric_batches(),
        hub_session_count: hub_memory.session_count,
        hub_hot_log_bytes: hub_memory.hot_log_bytes,
        hub_broadcast_last_bytes: hub_memory.broadcast_last_bytes,
    })
    .into_response()
}

async fn api_observability_batch(
    State(state): State<Arc<AppState>>,
    Json(batch): Json<TelemetryBatch>,
) -> Response {
    match state.observability.submit(batch) {
        Ok(()) => (StatusCode::ACCEPTED, Json(SubmitReceipt { accepted: true })).into_response(),
        Err(message) if message == "observability queue full" => {
            (StatusCode::SERVICE_UNAVAILABLE, message).into_response()
        }
        Err(message) => (StatusCode::BAD_REQUEST, message).into_response(),
    }
}

async fn api_observability_incidents(State(state): State<Arc<AppState>>) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "incident ledger unavailable",
        )
            .into_response();
    };
    match store.runtime_incidents(200).await {
        Ok(incidents) => Json(incidents).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn api_usage(State(state): State<Arc<AppState>>) -> Response {
    let snapshot = crate::usage::with_session_usage(
        state.usage.snapshot().await,
        &state.hub.session_list(),
        &state.provider_catalog,
    );
    Json(snapshot).into_response()
}

async fn api_usage_refresh(State(state): State<Arc<AppState>>) -> Response {
    let snapshot = crate::usage::with_session_usage(
        state.usage.refresh().await,
        &state.hub.session_list(),
        &state.provider_catalog,
    );
    Json(snapshot).into_response()
}

async fn api_usage_provider_refresh(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(provider): axum::extract::Path<String>,
) -> Response {
    match state.usage.refresh_provider(&provider).await {
        Ok(snapshot) => Json(crate::usage::with_session_usage(
            snapshot,
            &state.hub.session_list(),
            &state.provider_catalog,
        ))
        .into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

#[derive(Default, Deserialize)]
struct DeepSeekActivityQuery {
    window: Option<String>,
    model: Option<String>,
    agent: Option<String>,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
}

#[derive(Debug, Eq, PartialEq)]
struct DeepSeekActivityFilter {
    window: String,
    from_ms: i64,
    to_ms: i64,
    models: Vec<String>,
    agents: Vec<String>,
}

fn selected_activity_filters(
    value: Option<&str>,
    allowed: &[&str],
) -> Result<Vec<String>, &'static str> {
    let Some(value) = value.filter(|value| !value.is_empty() && *value != "all") else {
        return Ok(Vec::new());
    };
    let mut selected = Vec::new();
    for item in value.split(',') {
        if item == "all" || !allowed.contains(&item) {
            return Err("invalid activity filter");
        }
        if !selected.iter().any(|existing| existing == item) {
            selected.push(item.to_owned());
        }
    }
    Ok(selected)
}

fn parse_deepseek_activity_filter(
    query: &DeepSeekActivityQuery,
) -> Result<DeepSeekActivityFilter, &'static str> {
    let now = now_ms();
    let (window, from_ms, to_ms) = match (query.from_ms, query.to_ms) {
        (Some(from_ms), Some(to_ms))
            if from_ms > 0
                && to_ms > from_ms
                && to_ms <= now.saturating_add(5 * 60_000)
                && to_ms.saturating_sub(from_ms) <= 30 * 86_400_000 =>
        {
            ("custom".to_owned(), from_ms, to_ms)
        }
        (Some(_), Some(_)) => return Err("invalid activity time range"),
        (None, None) => {
            let window = query.window.as_deref().unwrap_or("24h");
            let window_seconds = match window {
                "1h" => 3_600,
                "2h" => 2 * 3_600,
                "4h" => 4 * 3_600,
                "6h" => 6 * 3_600,
                "8h" => 8 * 3_600,
                "12h" => 12 * 3_600,
                "24h" => 24 * 3_600,
                "7d" => 7 * 86_400,
                "14d" => 14 * 86_400,
                "30d" => 30 * 86_400,
                _ => return Err("invalid activity window"),
            };
            (
                window.to_owned(),
                now.saturating_sub(i64::from(window_seconds) * 1_000),
                now,
            )
        }
        _ => return Err("activity time range requires both boundaries"),
    };
    Ok(DeepSeekActivityFilter {
        window,
        from_ms,
        to_ms,
        models: selected_activity_filters(query.model.as_deref(), &["flash", "pro"])?,
        agents: selected_activity_filters(query.agent.as_deref(), &["codex", "claude"])?,
    })
}

async fn api_deepseek_usage_activity(
    State(state): State<Arc<AppState>>,
    Query(query): Query<DeepSeekActivityQuery>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "provider usage persistence is unavailable",
        )
            .into_response();
    };
    let filter = match parse_deepseek_activity_filter(&query) {
        Ok(filter) => filter,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    match store
        .provider_usage_activity(
            "deepseek",
            filter.from_ms,
            filter.to_ms,
            &filter.agents,
            &filter.models,
        )
        .await
    {
        Ok(mut activity) => {
            if let Some(object) = activity.as_object_mut() {
                object.insert("window".to_owned(), filter.window.into());
                object.insert("observedAtMs".to_owned(), crate::usage::now_ms().into());
            }
            crate::provider_info::decorate_deepseek_activity(&mut activity);
            Json(activity).into_response()
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

#[cfg(test)]
mod deepseek_activity_filter_tests {
    use super::{DeepSeekActivityQuery, parse_deepseek_activity_filter};

    #[test]
    fn defaults_to_bounded_unfiltered_activity() {
        let filter = parse_deepseek_activity_filter(&DeepSeekActivityQuery::default())
            .expect("default filter");
        assert_eq!(filter.window, "24h");
        assert_eq!(filter.to_ms - filter.from_ms, 86_400_000);
        assert!(filter.models.is_empty());
        assert!(filter.agents.is_empty());
    }

    #[test]
    fn accepts_agent_and_model_family_filters() {
        let filter = parse_deepseek_activity_filter(&DeepSeekActivityQuery {
            window: Some("30d".to_owned()),
            model: Some("pro,flash".to_owned()),
            agent: Some("codex,claude".to_owned()),
            ..DeepSeekActivityQuery::default()
        })
        .expect("multi filter");
        assert_eq!(filter.to_ms - filter.from_ms, 30 * 86_400_000);
        assert_eq!(filter.models, ["pro", "flash"]);
        assert_eq!(filter.agents, ["codex", "claude"]);
    }

    #[test]
    fn accepts_short_rolling_windows() {
        for (window, window_seconds) in [
            ("2h", 2 * 3_600),
            ("4h", 4 * 3_600),
            ("8h", 8 * 3_600),
            ("12h", 12 * 3_600),
        ] {
            let filter = parse_deepseek_activity_filter(&DeepSeekActivityQuery {
                window: Some(window.to_owned()),
                ..DeepSeekActivityQuery::default()
            })
            .expect("rolling window");
            assert_eq!(filter.window, window);
            assert_eq!(
                filter.to_ms - filter.from_ms,
                i64::from(window_seconds) * 1_000
            );
        }
    }

    #[test]
    fn accepts_an_exact_bounded_time_range() {
        let to_ms = super::now_ms();
        let filter = parse_deepseek_activity_filter(&DeepSeekActivityQuery {
            from_ms: Some(to_ms - 2 * 3_600_000),
            to_ms: Some(to_ms),
            ..DeepSeekActivityQuery::default()
        })
        .expect("custom range");
        assert_eq!(filter.window, "custom");
        assert_eq!(filter.from_ms, to_ms - 2 * 3_600_000);
        assert_eq!(filter.to_ms, to_ms);
    }

    #[test]
    fn rejects_open_ended_dimensions() {
        for query in [
            DeepSeekActivityQuery {
                window: Some("forever".to_owned()),
                ..DeepSeekActivityQuery::default()
            },
            DeepSeekActivityQuery {
                model: Some("deepseek-v4-pro[1m]".to_owned()),
                ..DeepSeekActivityQuery::default()
            },
            DeepSeekActivityQuery {
                agent: Some("unknown-runtime".to_owned()),
                ..DeepSeekActivityQuery::default()
            },
        ] {
            assert!(parse_deepseek_activity_filter(&query).is_err());
        }
    }
}

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

fn new_reset_idempotency_key() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "cowboy-reset-{}-{}-{}",
        std::process::id(),
        now_ms(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    )
}

fn reset_provider_supported(provider: &str) -> bool {
    crate::usage::RESET_PROVIDERS.contains(&provider)
}

async fn api_provider_reset(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Json(request): Json<ResetRequest>,
) -> Response {
    if !reset_provider_supported(&provider) {
        return (
            StatusCode::BAD_REQUEST,
            "provider does not support usage resets",
        )
            .into_response();
    }
    if request.confirm != "confirm" {
        return (StatusCode::BAD_REQUEST, "confirmation must be confirm").into_response();
    }
    let key = new_reset_idempotency_key();
    if let Some(store) = state.store.as_ref() {
        let _ = store
            .append_provider_action_log(
                &provider,
                "manual",
                "started",
                "preflight",
                "Manual reset attempt started",
                Some(&request.expected_credit_id),
                Some(&key),
                now_ms(),
            )
            .await;
    }
    match state
        .usage
        .consume_nearest_reset(&provider, &key, Some(&request.expected_credit_id))
        .await
    {
        Ok(result) => {
            if let Some(store) = state.store.as_ref() {
                let _ = store
                    .append_provider_action_log(
                        &provider,
                        "manual",
                        "succeeded",
                        "provider_response",
                        &result.outcome,
                        result.credit_id.as_deref(),
                        Some(&key),
                        now_ms(),
                    )
                    .await;
            }
            Json(result).into_response()
        }
        Err(error) => {
            if let Some(store) = state.store.as_ref() {
                let status = if error.call_may_have_reached_provider {
                    "unknown"
                } else {
                    "failed"
                };
                let phase = if error.call_may_have_reached_provider {
                    "consume"
                } else {
                    "preflight"
                };
                let _ = store
                    .append_provider_action_log(
                        &provider,
                        "manual",
                        status,
                        phase,
                        &error.to_string(),
                        error.credit_id.as_deref(),
                        Some(&key),
                        now_ms(),
                    )
                    .await;
            }
            (StatusCode::CONFLICT, error.to_string()).into_response()
        }
    }
}

async fn api_provider_reset_schedule(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Json(request): Json<ResetScheduleRequest>,
) -> Response {
    if !reset_provider_supported(&provider) {
        return (
            StatusCode::BAD_REQUEST,
            "provider does not support usage resets",
        )
            .into_response();
    }
    if request.confirm != "confirm" {
        return (StatusCode::BAD_REQUEST, "confirmation must be confirm").into_response();
    }
    if request.fire_at_ms < now_ms().saturating_add(60_000) {
        return (
            StatusCode::BAD_REQUEST,
            "schedule must be at least one minute in the future",
        )
            .into_response();
    }
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "persistent scheduling unavailable",
        )
            .into_response();
    };
    let key = new_reset_idempotency_key();
    if let Err(error) = store
        .upsert_provider_reset(&provider, request.fire_at_ms, &key)
        .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    let _ = store
        .append_provider_action_log(
            &provider,
            "scheduled",
            "scheduled",
            "timer",
            "Reset scheduled",
            None,
            Some(&key),
            now_ms(),
        )
        .await;
    state
        .usage
        .set_reset_schedule(
            &provider,
            Some(crate::usage::ResetSchedule {
                fire_at_ms: request.fire_at_ms,
            }),
        )
        .await;
    Json(state.usage.snapshot().await).into_response()
}

async fn api_provider_reset_cancel(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
) -> Response {
    if !reset_provider_supported(&provider) {
        return (
            StatusCode::BAD_REQUEST,
            "provider does not support usage resets",
        )
            .into_response();
    }
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "persistent scheduling unavailable",
        )
            .into_response();
    };
    if let Err(error) = store.delete_provider_reset(&provider).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    let _ = store
        .append_provider_action_log(
            &provider,
            "scheduled",
            "cancelled",
            "timer",
            "Scheduled reset cancelled",
            None,
            None,
            now_ms(),
        )
        .await;
    state.usage.set_reset_schedule(&provider, None).await;
    StatusCode::NO_CONTENT.into_response()
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

fn selected_log_filters(
    value: Option<&str>,
    allowed: &[&str],
) -> Result<Vec<String>, &'static str> {
    match value {
        None | Some("") | Some("all") => Ok(Vec::new()),
        Some(value) => {
            let mut selected = Vec::new();
            for item in value.split(',') {
                if item == "all" || !allowed.contains(&item) {
                    return Err("invalid diagnostic log filter");
                }
                if !selected.iter().any(|existing| existing == item) {
                    selected.push(item.to_owned());
                }
            }
            Ok(selected)
        }
    }
}

fn decode_log_cursor(value: Option<&str>) -> Result<Option<DiagnosticLogCursor>, &'static str> {
    let Some(value) = value else {
        return Ok(None);
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| "invalid diagnostic log cursor")?;
    let cursor: DiagnosticLogCursor =
        serde_json::from_slice(&bytes).map_err(|_| "invalid diagnostic log cursor")?;
    if cursor.occurred_at_ms <= 0 || cursor.id.is_empty() || cursor.id.len() > 512 {
        return Err("invalid diagnostic log cursor");
    }
    Ok(Some(cursor))
}

fn encode_log_cursor(item: &crate::store::DiagnosticLogSummary) -> Option<String> {
    serde_json::to_vec(&DiagnosticLogCursor {
        occurred_at_ms: item.occurred_at_ms,
        id: item.id.clone(),
    })
    .ok()
    .map(|value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value))
}

fn parse_diagnostic_log_filter(
    query: &DiagnosticLogsQuery,
) -> Result<crate::store::DiagnosticLogFilter, &'static str> {
    let now = now_ms();
    let (since_ms, until_ms) = match (query.from_ms, query.to_ms) {
        (Some(from_ms), Some(to_ms))
            if from_ms > 0
                && to_ms > from_ms
                && to_ms <= now.saturating_add(5 * 60_000)
                && to_ms.saturating_sub(from_ms) <= 365 * 86_400_000 =>
        {
            (from_ms, to_ms)
        }
        (Some(_), Some(_)) => return Err("invalid diagnostic log time range"),
        (None, None) => {
            let window_seconds = match query.window.as_deref().unwrap_or("7d") {
                "1h" => 3_600,
                "24h" => 86_400,
                "7d" => 7 * 86_400,
                "30d" => 30 * 86_400,
                _ => return Err("invalid diagnostic log window"),
            };
            (now.saturating_sub(i64::from(window_seconds) * 1_000), now)
        }
        _ => return Err("diagnostic log time range requires both boundaries"),
    };
    let session_ref = query
        .session
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if session_ref.as_ref().is_some_and(|value| {
        value.len() > 128
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')
            })
    }) {
        return Err("invalid diagnostic log session");
    }
    let cursor = decode_log_cursor(query.cursor.as_deref())?;
    Ok(crate::store::DiagnosticLogFilter {
        since_ms,
        until_ms,
        kinds: selected_log_filters(
            query.kind.as_deref(),
            &[
                "session_error",
                "provider_error",
                "cache_anomaly",
                "automation",
            ],
        )?,
        severities: selected_log_filters(
            query.severity.as_deref(),
            &["info", "warning", "error", "critical"],
        )?,
        states: selected_log_filters(
            query.state.as_deref(),
            &[
                "active",
                "recovered",
                "failed",
                "succeeded",
                "observed",
                "scheduled",
                "started",
                "retrying",
                "unknown",
                "cancelled",
            ],
        )?,
        agents: selected_log_filters(query.agent.as_deref(), &["codex", "claude"])?,
        session_ref,
        cursor_ms: cursor.as_ref().map(|value| value.occurred_at_ms),
        cursor_id: cursor.map(|value| value.id),
        limit: query.limit.unwrap_or(25).clamp(1, 100),
    })
}

async fn api_diagnostic_logs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<DiagnosticLogsQuery>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "diagnostic logs unavailable",
        )
            .into_response();
    };
    let filter = match parse_diagnostic_log_filter(&query) {
        Ok(filter) => filter,
        Err(message) => return (StatusCode::BAD_REQUEST, message).into_response(),
    };
    match store.diagnostic_logs(&filter).await {
        Ok(mut items) => {
            let has_more = items.len() > usize::try_from(filter.limit).unwrap_or(100);
            items.truncate(usize::try_from(filter.limit).unwrap_or(100));
            let next_cursor = has_more && !items.is_empty();
            Json(DiagnosticLogPage {
                next_cursor: next_cursor
                    .then(|| items.last().and_then(encode_log_cursor))
                    .flatten(),
                items,
            })
            .into_response()
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn api_diagnostic_log_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "diagnostic logs unavailable",
        )
            .into_response();
    };
    if id.is_empty() || id.len() > 512 {
        return (StatusCode::BAD_REQUEST, "invalid diagnostic log id").into_response();
    }
    match store.diagnostic_log_detail(&id).await {
        Ok(Some(detail)) => Json(detail).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

#[cfg(test)]
mod diagnostic_log_query_tests {
    use super::{
        DiagnosticLogsQuery, decode_log_cursor, encode_log_cursor, now_ms,
        parse_diagnostic_log_filter,
    };

    #[test]
    fn diagnostic_log_filters_are_closed_and_bounded() {
        let filter = parse_diagnostic_log_filter(&DiagnosticLogsQuery {
            kind: Some("cache_anomaly,provider_error".to_owned()),
            severity: Some("warning,error".to_owned()),
            state: Some("observed,failed".to_owned()),
            agent: Some("claude,codex".to_owned()),
            window: Some("30d".to_owned()),
            from_ms: None,
            to_ms: None,
            session: Some("0123456789abcdef0123456789abcdef".to_owned()),
            limit: Some(5_000),
            cursor: None,
        })
        .expect("valid diagnostic filters");
        assert_eq!(filter.kinds, ["cache_anomaly", "provider_error"]);
        assert_eq!(filter.severities, ["warning", "error"]);
        assert_eq!(filter.states, ["observed", "failed"]);
        assert_eq!(filter.agents, ["claude", "codex"]);
        assert_eq!(filter.limit, 100);

        assert!(
            parse_diagnostic_log_filter(&DiagnosticLogsQuery {
                kind: Some("raw_prompt".to_owned()),
                ..DiagnosticLogsQuery::default()
            })
            .is_err()
        );
    }

    #[test]
    fn diagnostic_log_time_ranges_require_two_bounded_boundaries() {
        let until_ms = now_ms();
        let filter = parse_diagnostic_log_filter(&DiagnosticLogsQuery {
            from_ms: Some(until_ms - 6 * 3_600_000),
            to_ms: Some(until_ms),
            ..DiagnosticLogsQuery::default()
        })
        .expect("valid explicit range");
        assert_eq!(filter.since_ms, until_ms - 6 * 3_600_000);
        assert_eq!(filter.until_ms, until_ms);
        assert!(
            parse_diagnostic_log_filter(&DiagnosticLogsQuery {
                from_ms: Some(until_ms - 3_600_000),
                ..DiagnosticLogsQuery::default()
            })
            .is_err()
        );
    }

    #[test]
    fn diagnostic_log_cursor_round_trips_the_stable_sort_key() {
        let item = crate::store::DiagnosticLogSummary {
            id: "provider:hawk:producer-1:42".to_owned(),
            occurred_at_ms: 1_786_035_044_709,
            kind: "provider_error".to_owned(),
            severity: "error".to_owned(),
            state: "failed".to_owned(),
            title: "DeepSeek HTTP 400".to_owned(),
            summary: "request failed".to_owned(),
            session_ref: None,
            provider: Some("deepseek".to_owned()),
            agent: Some("claude".to_owned()),
            model: Some("deepseek-v4-flash".to_owned()),
            classification: Some("provider_http_error".to_owned()),
        };
        let encoded = encode_log_cursor(&item).expect("encode cursor");
        let decoded = decode_log_cursor(Some(&encoded))
            .expect("decode cursor")
            .expect("cursor present");
        assert_eq!(decoded.occurred_at_ms, item.occurred_at_ms);
        assert_eq!(decoded.id, item.id);
    }
}

async fn api_usage_logs(State(state): State<Arc<AppState>>) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "persistent logs unavailable",
        )
            .into_response();
    };
    match store.provider_action_logs(100).await {
        Ok(logs) => Json(logs).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn prometheus_metrics(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !crate::product_auth::metrics_scrape_allowed(peer, &headers) {
        tracing::info!(reason = "forwarded", "metrics_rejected");
        return StatusCode::NOT_FOUND.into_response();
    }
    let sessions_live = state.hub.session_list().len();
    let (db_bytes, events_rows, sessions_deleted) = match &state.store {
        Some(store) => store.storage_metrics().await.unwrap_or((0, 0, 0)),
        None => (0, i64::try_from(state.hub.event_total()).unwrap_or(0), 0),
    };
    let health = state.persistence_health.as_ref();
    let runtime = state.runtime_router.stats();
    let runtime_connected = state.runtime_router.has_connected_runtime();
    let daemon_memory = daemon_memory();
    let cgroup_memory = crate::memory_observability::own_cgroup_memory().unwrap_or_default();
    let hub_memory = state.hub.memory_stats();
    let body = format!(
        "# TYPE cowboy_up gauge\ncowboy_up {}\n# TYPE cowboy_database_bytes gauge\ncowboy_database_bytes {db_bytes}\n# TYPE cowboy_events_rows gauge\ncowboy_events_rows {events_rows}\n# TYPE cowboy_sessions gauge\ncowboy_sessions{{state=\"live\"}} {sessions_live}\ncowboy_sessions{{state=\"deleted\"}} {sessions_deleted}\n# TYPE cowboy_daemon_rss_bytes gauge\ncowboy_daemon_rss_bytes {}\n# TYPE cowboy_daemon_rss_peak_bytes gauge\ncowboy_daemon_rss_peak_bytes {}\n# TYPE cowboy_cgroup_memory_bytes gauge\ncowboy_cgroup_memory_bytes {}\n# TYPE cowboy_cgroup_memory_peak_bytes gauge\ncowboy_cgroup_memory_peak_bytes {}\n# TYPE cowboy_persistence_pending gauge\ncowboy_persistence_pending {}\n# TYPE cowboy_persistence_pending_bytes gauge\ncowboy_persistence_pending_bytes {}\n# TYPE cowboy_hub_hot_log_bytes gauge\ncowboy_hub_hot_log_bytes {}\n# TYPE cowboy_hub_broadcast_last_bytes gauge\ncowboy_hub_broadcast_last_bytes {}\n# TYPE cowboy_persistence_dropped_total counter\ncowboy_persistence_dropped_total {}\n# TYPE cowboy_persistence_failed_batches_total counter\ncowboy_persistence_failed_batches_total {}\n# TYPE cowboy_persistence_healthy gauge\ncowboy_persistence_healthy {}\n# TYPE cowboy_runtime_connected gauge\ncowboy_runtime_connected {}\n# TYPE cowboy_runtime_workers gauge\ncowboy_runtime_workers {}\n# TYPE cowboy_runtime_busy_workers gauge\ncowboy_runtime_busy_workers {}\n# TYPE cowboy_runtime_draining_workers gauge\ncowboy_runtime_draining_workers {}\n# TYPE cowboy_runtime_handoff_workers gauge\ncowboy_runtime_handoff_workers {}\n# TYPE cowboy_runtime_pending_commands gauge\ncowboy_runtime_pending_commands {}\n# TYPE cowboy_observability_pending gauge\ncowboy_observability_pending {}\n# TYPE cowboy_observability_accepted_batches_total counter\ncowboy_observability_accepted_batches_total {}\n# TYPE cowboy_observability_dropped_batches_total counter\ncowboy_observability_dropped_batches_total {}\n# TYPE cowboy_observability_failed_log_batches_total counter\ncowboy_observability_failed_log_batches_total {}\n# TYPE cowboy_observability_failed_metric_batches_total counter\ncowboy_observability_failed_metric_batches_total {}\n",
        u8::from(state.runtime_health.is_healthy(state.store.is_some()) && runtime_connected),
        daemon_memory.rss_bytes,
        daemon_memory.rss_peak_bytes,
        cgroup_memory.current_bytes,
        cgroup_memory.peak_bytes,
        health.map_or(0, |h| h.pending()),
        health.map_or(0, |h| h.pending_bytes()),
        hub_memory.hot_log_bytes,
        hub_memory.broadcast_last_bytes,
        health.map_or(0, |h| h.dropped()),
        health.map_or(0, |h| h.failed_batches()),
        u8::from(health.is_none_or(|h| h.is_healthy())),
        u8::from(runtime_connected),
        runtime.workers,
        runtime.busy_workers,
        runtime.draining_workers,
        runtime.handoff_workers,
        runtime.pending_commands,
        state.observability.health().pending(),
        state.observability.health().accepted_batches(),
        state.observability.health().dropped_batches(),
        state.observability.health().failed_log_batches(),
        state.observability.health().failed_metric_batches(),
    );
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

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

fn projected_work_items(columbus: &std::path::Path) -> Vec<WorkspaceWorkItem> {
    let output = std::process::Command::new("harness-cli")
        .args([
            "--root",
            &columbus.display().to_string(),
            "work-item",
            "list",
            "--format=json",
        ])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    serde_json::from_slice(&output.stdout).unwrap_or_default()
}

/// Resolve a registered project's current stable checkout through Columbus,
/// with a registry-backed fallback when the harness CLI is not on `PATH`.
/// Returns `None` for a registered project whose checkout has not been cloned.
fn project_worktree(columbus: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    crate::workspace::current_project_checkout(columbus, name)
}

/// `GET /api/workspaces` — the selectable session roots for the New Session
/// dialog: Columbus plus one entry per
/// columbus-managed project, read from `<workspace-root>/columbus/project-defs/*`
/// (the registry is the source of truth for which projects exist) and resolved
/// to each project's stable checkout. The selected Machine prepares an isolated
/// session worktree before the worker starts. The frontend keeps a fallback for when
/// this is unreachable.
async fn api_workspaces(State(state): State<Arc<AppState>>) -> Response {
    let columbus = state.supervisor.workspace_root().join("columbus");
    let work_items = projected_work_items(&columbus);
    let mut out = vec![Workspace {
        value: "columbus".to_owned(),
        label: "columbus".to_owned(),
        help: columbus.display().to_string(),
        project: None,
        active_work_items: Vec::new(),
    }];
    if let Ok(entries) = std::fs::read_dir(columbus.join("project-defs")) {
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "schema" && n != "cue.mod")
            .collect();
        names.sort();
        for name in names {
            if let Some(dir) = project_worktree(&columbus, &name) {
                let path = dir.display().to_string();
                out.push(Workspace {
                    value: path.clone(),
                    label: name.clone(),
                    help: path,
                    project: Some(name.clone()),
                    active_work_items: work_items
                        .iter()
                        .filter(|item| item.projects.contains(&name))
                        .cloned()
                        .collect(),
                });
            }
        }
    }
    Json(out).into_response()
}

async fn api_machines(State(state): State<Arc<AppState>>) -> Response {
    match state.machine_snapshots.load().await {
        Ok(machines) => Json(machines).into_response(),
        Err(error) => {
            tracing::error!(%error, "listing Machines");
            (StatusCode::INTERNAL_SERVER_ERROR, "could not list machines").into_response()
        }
    }
}

async fn api_machine_events(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    Json(state.machine_control.events(&machine_id)).into_response()
}

#[derive(Debug, Serialize)]
struct MachineDeploymentHealthResponse {
    connected: bool,
    status: String,
    active_acp_generation: Option<String>,
}

async fn api_machine_deployment_health(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "machine registry unavailable",
        )
            .into_response();
    };
    let machines = match store.list_machines().await {
        Ok(machines) => machines,
        Err(error) => {
            tracing::error!(%error, "reading Machine deployment health");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not read Machine deployment health",
            )
                .into_response();
        }
    };
    let Some(machine) = machines
        .into_iter()
        .find(|machine| machine.id == machine_id && !machine.revoked)
    else {
        return (StatusCode::NOT_FOUND, "unknown or revoked Machine").into_response();
    };
    let active_acp_generation = machine
        .inventory
        .get("components")
        .cloned()
        .and_then(|value| {
            serde_json::from_value::<Vec<crate::machine_protocol::ComponentInventory>>(value).ok()
        })
        .and_then(|components| {
            components.into_iter().find_map(|component| {
                (component.id.kind == crate::machine_protocol::ComponentKind::AcpRuntime
                    && component.state == crate::machine_protocol::ComponentState::Active)
                    .then_some(component.generation)
            })
        });
    Json(MachineDeploymentHealthResponse {
        connected: state.runtime_router.connected(&machine.id),
        status: machine.status,
        active_acp_generation,
    })
    .into_response()
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

async fn api_machine_create_enrollment(
    State(state): State<Arc<AppState>>,
    Json(request): Json<MachineEnrollmentRequest>,
) -> Response {
    const TTL_SECONDS: i64 = 900;
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Machine enrollment requires persistence",
        )
            .into_response();
    };
    let display_name = request
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Computer")
        .to_owned();
    let requested_id = request
        .machine_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned);
    let mut last_error = None;
    for _ in 0..8 {
        let machine_id = if let Some(id) = requested_id.as_deref() {
            id.to_owned()
        } else {
            match crate::store::generate_machine_id() {
                Ok(id) => id,
                Err(error) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
                }
            }
        };
        match store
            .create_machine_enrollment(&machine_id, &display_name, TTL_SECONDS)
            .await
        {
            Ok(token) => {
                return Json(MachineEnrollmentResponse {
                    machine_id,
                    display_name,
                    token,
                    expires_in_seconds: TTL_SECONDS,
                })
                .into_response();
            }
            Err(error) if requested_id.is_some() => {
                return (StatusCode::CONFLICT, error.to_string()).into_response();
            }
            Err(error) => last_error = Some(error),
        }
    }
    (
        StatusCode::CONFLICT,
        last_error.map_or_else(
            || "could not allocate a Machine id".to_owned(),
            |error| error.to_string(),
        ),
    )
        .into_response()
}

async fn api_machine_cancel_enrollment(
    State(state): State<Arc<AppState>>,
    Json(request): Json<MachineEnrollmentCancelRequest>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Machine enrollment requires persistence",
        )
            .into_response();
    };
    if request.token.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "enrollment token is required").into_response();
    }
    match store.cancel_machine_enrollment(&request.token).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

#[derive(Debug, Serialize)]
struct MachineCommandResponse {
    request_id: String,
}

const MACHINE_INVENTORY_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
const MACHINE_COMPONENT_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

fn machine_command_error_status(error: &str) -> StatusCode {
    if error == "Machine command timed out" {
        StatusCode::GATEWAY_TIMEOUT
    } else if error == "Machine command response channel closed" {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::CONFLICT
    }
}

async fn await_machine_command(
    state: &AppState,
    machine_id: &str,
    request_id: String,
    command: crate::machine_protocol::MachineCommand,
    operation: &'static str,
    timeout: std::time::Duration,
) -> Response {
    let started = std::time::Instant::now();
    let result = state
        .machine_control
        .command_request_with_timeout(machine_id, request_id.clone(), command, timeout)
        .await;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match result {
        Ok(()) => {
            tracing::info!(
                %machine_id,
                %request_id,
                operation,
                elapsed_ms,
                "Machine command completed"
            );
            Json(MachineCommandResponse { request_id }).into_response()
        }
        Err(error) => {
            tracing::warn!(
                %machine_id,
                %request_id,
                operation,
                elapsed_ms,
                timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
                %error,
                "Machine command failed"
            );
            (machine_command_error_status(&error), error).into_response()
        }
    }
}

fn machine_request_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        random_machine_token().unwrap_or_else(|_| now_ms().to_string())
    )
}

fn apply_workspace_inventory(
    current: &mut Vec<crate::machine_protocol::MachineWorkspace>,
    current_revision: &mut Option<String>,
    workspaces: Option<Vec<crate::machine_protocol::MachineWorkspace>>,
    revision: Option<String>,
) {
    if let Some(workspaces) = workspaces {
        *current = workspaces;
        *current_revision = revision;
    }
}

async fn api_machine_refresh(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    let request_id = machine_request_id("refresh");
    await_machine_command(
        state.as_ref(),
        &machine_id,
        request_id.clone(),
        crate::machine_protocol::MachineCommand::RefreshInventory {
            request_id: request_id.clone(),
        },
        "refresh_inventory",
        MACHINE_INVENTORY_COMMAND_TIMEOUT,
    )
    .await
}

async fn api_machine_reconcile(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    if state.desired_machine_components.is_empty() {
        return (
            StatusCode::PRECONDITION_FAILED,
            "no signed Machine component manifest is configured",
        )
            .into_response();
    }
    let request_id = machine_request_id("reconcile");
    await_machine_command(
        state.as_ref(),
        &machine_id,
        request_id.clone(),
        crate::machine_protocol::MachineCommand::Reconcile {
            request_id: request_id.clone(),
            components: state.desired_machine_components.as_ref().clone(),
        },
        "reconcile_components",
        MACHINE_COMPONENT_COMMAND_TIMEOUT,
    )
    .await
}

async fn api_machine_reconcile_one(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
    Json(component_id): Json<crate::machine_protocol::ComponentId>,
) -> Response {
    let Some(component) = state
        .desired_machine_components
        .iter()
        .find(|component| component.id == component_id)
        .cloned()
    else {
        return (StatusCode::NOT_FOUND, "no signed update for this component").into_response();
    };
    let request_id = machine_request_id("reconcile-one");
    await_machine_command(
        state.as_ref(),
        &machine_id,
        request_id.clone(),
        crate::machine_protocol::MachineCommand::Reconcile {
            request_id: request_id.clone(),
            components: vec![component],
        },
        "reconcile_component",
        MACHINE_COMPONENT_COMMAND_TIMEOUT,
    )
    .await
}

async fn api_machine_update_npm(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
    Json(component): Json<crate::machine_protocol::ComponentId>,
) -> Response {
    let request_id = machine_request_id("update-npm");
    await_machine_command(
        state.as_ref(),
        &machine_id,
        request_id.clone(),
        crate::machine_protocol::MachineCommand::UpdateNpmComponent {
            request_id: request_id.clone(),
            component,
        },
        "update_npm_component",
        MACHINE_COMPONENT_COMMAND_TIMEOUT,
    )
    .await
}

async fn api_machine_revoke(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "persistence unavailable").into_response();
    };
    match store.revoke_machine(&machine_id).await {
        Ok(()) => {
            state.machine_control.disconnect(&machine_id);
            state.runtime_router.remove(&machine_id);
            state.machine_snapshots.publish().await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

fn product_machine_is_visible(
    machine: &crate::store::MachineRecord,
    product_auth_enabled: bool,
) -> bool {
    !machine.revoked && (!product_auth_enabled || machine.fingerprint.is_some())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct ProviderAuthenticationExecutorIdentity {
    provider_id: String,
    provider_version: String,
    generation_digest: String,
}

async fn connected_provider_authentication_executors(
    state: &AppState,
) -> Vec<ProviderAuthenticationExecutorIdentity> {
    let Some(store) = state.store.as_ref() else {
        return Vec::new();
    };
    let Ok(machines) = store.list_machines().await else {
        return Vec::new();
    };
    let connected = state.machine_control.connected_machine_ids();
    let mut executors = BTreeSet::new();
    for machine in machines
        .into_iter()
        .filter(|machine| !machine.revoked && connected.contains(&machine.id))
    {
        let Ok(providers) = serde_json::from_value::<Vec<crate::machine_protocol::PluginInventory>>(
            machine
                .inventory
                .get("plugins")
                .or_else(|| machine.inventory.get("providers"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        ) else {
            continue;
        };
        for installed in providers.into_iter().filter(|provider| {
            provider.state == crate::machine_protocol::PluginInstallationState::Active
        }) {
            if state
                .provider_catalog
                .package(
                    &installed.plugin_id,
                    &installed.plugin_version,
                    &installed.generation_digest,
                )
                .is_none()
            {
                continue;
            }
            executors.insert(ProviderAuthenticationExecutorIdentity {
                provider_id: installed.plugin_id,
                provider_version: installed.plugin_version,
                generation_digest: installed.generation_digest,
            });
        }
    }
    executors.into_iter().collect()
}

async fn api_providers(State(state): State<Arc<AppState>>) -> Response {
    let providers = state.provider_catalog.entries();
    let authentication_executors = connected_provider_authentication_executors(&state).await;
    Json(serde_json::json!({
        "providers": providers,
        "authentications": state.provider_auth.statuses(),
        "authentication_executors": authentication_executors,
    }))
    .into_response()
}

async fn api_plugins(State(state): State<Arc<AppState>>) -> Response {
    let authentication_executors = connected_provider_authentication_executors(&state).await;
    Json(serde_json::json!({
        "component_release": crate::plugin::active_component_release(),
        "plugins": state.plugin_catalog.entries(),
        "providers": state.provider_catalog.entries(),
        "authentications": state.provider_auth.statuses(),
        "authentication_executors": authentication_executors,
    }))
    .into_response()
}

async fn api_plugin_catalog_refresh(State(state): State<Arc<AppState>>) -> Response {
    match state.plugin_catalog.refresh_external() {
        Ok(count) => Json(serde_json::json!({ "external_releases": count })).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

async fn api_provider_catalog_refresh(State(state): State<Arc<AppState>>) -> Response {
    match state.provider_catalog.refresh_external() {
        Ok(count) => Json(serde_json::json!({ "external_releases": count })).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

async fn api_machine_plugins(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "machine registry unavailable",
        )
            .into_response();
    };
    match store.list_machines().await {
        Ok(machines) => machines
            .into_iter()
            .find(|machine| machine.id == machine_id && !machine.revoked)
            .map_or_else(
                || (StatusCode::NOT_FOUND, "unknown or revoked Machine").into_response(),
                |machine| {
                    Json(
                        machine
                            .inventory
                            .get("plugins")
                            .or_else(|| machine.inventory.get("providers"))
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!([])),
                    )
                    .into_response()
                },
            ),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn api_machine_provider_inventory_compat(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "machine registry unavailable",
        )
            .into_response();
    };
    match store.list_machines().await {
        Ok(machines) => {
            let Some(machine) = machines
                .into_iter()
                .find(|machine| machine.id == machine_id && !machine.revoked)
            else {
                return (StatusCode::NOT_FOUND, "unknown or revoked Machine").into_response();
            };
            let plugins = machine
                .inventory
                .get("plugins")
                .or_else(|| machine.inventory.get("providers"))
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            let providers = plugins
                .into_iter()
                .filter_map(|mut plugin| {
                    let object = plugin.as_object_mut()?;
                    if object
                        .get("plugin_kind")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|kind| kind != "agent_provider")
                    {
                        return None;
                    }
                    if let Some(id) = object.remove("plugin_id") {
                        object.insert("provider_id".to_owned(), id);
                    }
                    if let Some(version) = object.remove("plugin_version") {
                        object.insert("provider_version".to_owned(), version);
                    }
                    object.remove("plugin_kind");
                    Some(plugin)
                })
                .collect::<Vec<_>>();
            Json(providers).into_response()
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct PluginInstallRequest {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    digest: Option<String>,
}

async fn agent_plugin_install_compatibility(
    state: &AppState,
    machine_id: &str,
    desired: &crate::machine_protocol::DesiredPlugin,
) -> Result<Option<cowboy_provider_sdk::ProviderCompatibilityProblem>, String> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(|| "Provider lifecycle requires persistence".to_owned())?;
    let machine = store
        .list_machines()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|machine| machine.id == machine_id && !machine.revoked)
        .ok_or_else(|| format!("unknown or revoked Machine {machine_id:?}"))?;
    let Some(raw_contracts) = machine.inventory.get("provider_contracts").cloned() else {
        return Ok(Some(
            cowboy_provider_sdk::ProviderCompatibilityProblem::capability_inventory_unavailable(),
        ));
    };
    let contracts = match serde_json::from_value::<cowboy_provider_sdk::ProviderContractInventory>(
        raw_contracts,
    ) {
        Ok(contracts) => contracts,
        Err(_) => {
            return Ok(Some(
                cowboy_provider_sdk::ProviderCompatibilityProblem::new(
                    cowboy_provider_sdk::ProviderCompatibilityCode::CapabilityInventoryInvalid,
                    "Cowboy Machine reported an invalid Provider capability inventory. Update Cowboy Machine before installing or upgrading Providers.",
                ),
            ));
        }
    };
    let target = serde_json::from_value::<cowboy_provider_sdk::PlatformTarget>(serde_json::json!({
        "os": machine.platform,
        "architecture": machine.architecture,
    }))
    .map_err(|error| format!("Machine Provider platform is invalid: {error}"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&desired.package_base64)
        .map_err(|error| format!("Catalog Provider package is invalid: {error}"))?;
    let plugin_package = desired
        .release
        .validate_bytes(&bytes)
        .map_err(|error| format!("Catalog Plugin release is invalid: {error}"))?;
    let package = plugin_package
        .agent_provider()
        .ok_or_else(|| "Plugin is not an Agent Provider".to_owned())?;
    let binding = desired
        .release
        .agent_provider_binding(&plugin_package)
        .map_err(|error| format!("Agent Plugin binding is invalid: {error}"))?;
    Ok(contracts.compatibility_problem(package, &binding, &target))
}

fn provider_compatibility_response(
    problem: cowboy_provider_sdk::ProviderCompatibilityProblem,
) -> Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "error": "provider_incompatible",
            "code": problem.code,
            "detail": problem.detail,
        })),
    )
        .into_response()
}

async fn api_machine_plugin_install(
    State(state): State<Arc<AppState>>,
    Path((machine_id, provider_id)): Path<(String, String)>,
    Json(request): Json<PluginInstallRequest>,
) -> Response {
    let desired = match state.plugin_catalog.resolve(
        &provider_id,
        request.version.as_deref(),
        request.digest.as_deref(),
    ) {
        Ok(desired) => desired,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let is_agent_plugin =
        desired.release.plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider;
    if is_agent_plugin {
        match agent_plugin_install_compatibility(&state, &machine_id, &desired).await {
            Ok(None) => {}
            Ok(Some(problem)) => return provider_compatibility_response(problem),
            Err(error) => return (StatusCode::CONFLICT, error).into_response(),
        }
    }
    let fence = (machine_id.clone(), provider_id.clone());
    let previous_fence = {
        let mut fences = state.plugin_lifecycle_fences.write();
        match fences.get(&fence).copied() {
            Some(PluginFenceState::Installing | PluginFenceState::Uninstalling) => {
                return (
                    StatusCode::CONFLICT,
                    "another Provider lifecycle operation is already in progress",
                )
                    .into_response();
            }
            previous => {
                fences.insert(fence.clone(), PluginFenceState::Installing);
                previous
            }
        }
    };
    // A replica can be stored before the Provider exists. Synchronize it first
    // so Machine activation validates and materializes the current Service
    // generation in the same local lifecycle transaction. An upgrade whose
    // old package cannot materialize the new authentication contract may
    // already hold that exact sealed replica; retrying the pre-sync would
    // deadlock the upgrade before the compatible package can be activated.
    let authentication = is_agent_plugin
        .then(|| state.provider_auth.status(&provider_id))
        .flatten();
    let installed = current_machine_plugin(&state, &machine_id, &provider_id)
        .await
        .ok();
    if provider_auth_sync_required_before_install(authentication.as_ref(), installed.as_ref())
        && let Err(error) = sync_provider_auth_to_machine(&state, &machine_id, &provider_id).await
    {
        let mut fences = state.plugin_lifecycle_fences.write();
        if previous_fence == Some(PluginFenceState::Uninstalled) {
            fences.insert(fence, PluginFenceState::Uninstalled);
        } else {
            fences.remove(&fence);
        }
        return (
            StatusCode::CONFLICT,
            format!("Service authentication sync failed before Provider installation: {error}"),
        )
            .into_response();
    }
    let request_id = machine_request_id("provider-install");
    let command = crate::machine_protocol::MachineCommand::InstallPlugin {
        request_id: request_id.clone(),
        plugin: Box::new(desired),
    };
    if let Err(error) = state
        .machine_control
        .command_request(&machine_id, request_id, command)
        .await
    {
        let mut fences = state.plugin_lifecycle_fences.write();
        if previous_fence == Some(PluginFenceState::Uninstalled) {
            fences.insert(fence, PluginFenceState::Uninstalled);
        } else {
            fences.remove(&fence);
        }
        return (StatusCode::CONFLICT, error).into_response();
    }
    // Re-read after activation to close a concurrent refresh/logout window. A
    // failure leaves a valid installed generation unschedulable until normal
    // reconciliation catches up; it never launches with stale credentials.
    let auth_sync = if is_agent_plugin && state.provider_auth.status(&provider_id).is_some() {
        sync_provider_auth_to_machine(&state, &machine_id, &provider_id)
            .await
            .map(|_| ())
    } else {
        Ok(())
    };
    state.plugin_lifecycle_fences.write().remove(&fence);
    if let Err(error) = auth_sync {
        return (
            StatusCode::CONFLICT,
            format!("Provider installed but Service authentication sync failed: {error}"),
        )
            .into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

fn provider_auth_sync_required_before_install(
    authentication: Option<&crate::provider_service::ProviderAuthenticationStatus>,
    installed: Option<&crate::machine_protocol::PluginInventory>,
) -> bool {
    let Some(authentication) = authentication else {
        return false;
    };
    !installed.is_some_and(|provider| {
        provider.auth_generation == Some(authentication.auth_generation)
            && provider.replica_state == crate::machine_protocol::ProviderReplicaState::Current
    })
}

async fn current_machine_plugin(
    state: &Arc<AppState>,
    machine_id: &str,
    provider_id: &str,
) -> Result<crate::machine_protocol::PluginInventory, String> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(|| "Provider lifecycle requires persistence".to_owned())?;
    let machine = store
        .list_machines()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|machine| machine.id == machine_id && !machine.revoked)
        .ok_or_else(|| format!("unknown or revoked Machine {machine_id:?}"))?;
    let providers: Vec<crate::machine_protocol::PluginInventory> = machine
        .inventory
        .get("plugins")
        .or_else(|| machine.inventory.get("providers"))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    providers
        .into_iter()
        .find(|provider| {
            provider.plugin_id == provider_id
                && provider.state == crate::machine_protocol::PluginInstallationState::Active
        })
        .ok_or_else(|| {
            format!("Provider {provider_id:?} is not installed on Machine {machine_id:?}")
        })
}

async fn api_machine_plugin_uninstall_plan(
    State(state): State<Arc<AppState>>,
    Path((machine_id, provider_id)): Path<(String, String)>,
) -> Response {
    let installed = match current_machine_plugin(&state, &machine_id, &provider_id).await {
        Ok(installed) => installed,
        Err(error) => return (StatusCode::CONFLICT, error).into_response(),
    };
    let mut affected: Vec<_> = state
        .hub
        .session_list()
        .into_iter()
        .filter(|session| session.machine_id == machine_id && session.provider == provider_id)
        .collect();
    affected.sort_by(|left, right| left.id.cmp(&right.id));
    let session_ids: Vec<_> = affected.iter().map(|session| session.id.clone()).collect();
    let active_session_ids: Vec<_> = affected
        .iter()
        .filter(|session| provider_session_has_active_turn(session.status))
        .map(|session| session.id.clone())
        .collect();
    let timestamp = now_ms();
    let purge_after_ms = timestamp.saturating_add(PROVIDER_UNINSTALL_RETENTION_MS);
    let expires_at_ms = timestamp.saturating_add(PROVIDER_UNINSTALL_PLAN_TTL_MS);
    let plan_id = match random_machine_token() {
        Ok(value) => value,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    state
        .plugin_uninstall_plans
        .lock()
        .retain(|_, plan| plan.expires_at_ms >= timestamp);
    state.plugin_uninstall_plans.lock().insert(
        plan_id.clone(),
        PluginUninstallPlan {
            machine_id: machine_id.clone(),
            plugin_id: provider_id.clone(),
            generation_digest: installed.generation_digest.clone(),
            session_ids,
            active_session_ids: active_session_ids.clone(),
            purge_after_ms,
            expires_at_ms,
        },
    );
    Json(serde_json::json!({
        "plan_id": plan_id,
        "machine_id": machine_id,
        "plugin_id": provider_id,
        "plugin_version": installed.plugin_version,
        "generation_digest": installed.generation_digest,
        "affected_sessions": affected,
        "active_session_ids": active_session_ids,
        "purge_after_ms": purge_after_ms,
        "expires_at_ms": expires_at_ms,
        "warning": "Uninstalling this Provider removes it from this Machine and soft-deletes every affected session. Session data remains recoverable only until the stated purge deadline.",
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
struct PluginUninstallRequest {
    plan_id: String,
    #[serde(default)]
    confirm_active_sessions: bool,
}

async fn compensate_plugin_uninstall(
    state: &Arc<AppState>,
    plan: &PluginUninstallPlan,
    stopped_live_sessions: &[String],
    cause: String,
) -> String {
    let request_id = machine_request_id("provider-reactivate");
    let compensation = state
        .machine_control
        .command_request(
            &plan.machine_id,
            request_id.clone(),
            crate::machine_protocol::MachineCommand::ReactivatePlugin {
                request_id,
                plugin_id: plan.plugin_id.clone(),
                generation_digest: plan.generation_digest.clone(),
            },
        )
        .await;
    let Err(compensation_error) = compensation else {
        let mut reload_errors = Vec::new();
        for session_id in stopped_live_sessions {
            if let Err(error) = state.supervisor.reload_session(session_id) {
                reload_errors.push(format!("{session_id}: {error}"));
            }
        }
        return if reload_errors.is_empty() {
            format!("{cause}; the previous Provider generation was restored")
        } else {
            format!(
                "{cause}; the previous Provider generation was restored, but live session reload failed: {}",
                reload_errors.join("; ")
            )
        };
    };
    format!(
        "{cause}; automatic Provider restoration failed: {compensation_error}. The Machine requires Provider lifecycle reconciliation"
    )
}

async fn api_machine_plugin_uninstall(
    State(state): State<Arc<AppState>>,
    Path((machine_id, provider_id)): Path<(String, String)>,
    Json(request): Json<PluginUninstallRequest>,
) -> Response {
    let plan = state.plugin_uninstall_plans.lock().remove(&request.plan_id);
    let Some(plan) = plan else {
        return (
            StatusCode::CONFLICT,
            "uninstall plan is missing or already consumed",
        )
            .into_response();
    };
    if plan.machine_id != machine_id
        || plan.plugin_id != provider_id
        || plan.expires_at_ms < now_ms()
    {
        return (
            StatusCode::CONFLICT,
            "uninstall plan is stale or does not match this target",
        )
            .into_response();
    }
    if !plan.active_session_ids.is_empty() && !request.confirm_active_sessions {
        return (
            StatusCode::CONFLICT,
            "active sessions require an explicit second confirmation",
        )
            .into_response();
    }
    let fence = (machine_id.clone(), provider_id.clone());
    let acquired = {
        let mut fences = state.plugin_lifecycle_fences.write();
        if fences.contains_key(&fence) {
            false
        } else {
            fences.insert(fence.clone(), PluginFenceState::Uninstalling);
            true
        }
    };
    if !acquired {
        return (
            StatusCode::CONFLICT,
            "another Provider lifecycle operation is already in progress",
        )
            .into_response();
    }
    let result = async {
        let current = current_machine_plugin(&state, &machine_id, &provider_id).await?;
        if current.generation_digest != plan.generation_digest {
            return Err("Provider generation changed; refresh the uninstall plan".to_owned());
        }
        let mut current_sessions: Vec<_> = state
            .hub
            .session_list()
            .into_iter()
            .filter(|session| session.machine_id == machine_id && session.provider == provider_id)
            .collect();
        current_sessions.sort_by(|left, right| left.id.cmp(&right.id));
        let current_session_ids: Vec<_> = current_sessions
            .iter()
            .map(|session| session.id.clone())
            .collect();
        if current_session_ids != plan.session_ids {
            return Err("affected session set changed; refresh the uninstall plan".to_owned());
        }
        let current_active_session_ids: Vec<_> = current_sessions
            .iter()
            .filter(|session| provider_session_has_active_turn(session.status))
            .map(|session| session.id.clone())
            .collect();
        if current_active_session_ids != plan.active_session_ids {
            return Err("active turn set changed; refresh the uninstall plan".to_owned());
        }
        let stopped_live_sessions: Vec<_> = plan
            .session_ids
            .iter()
            .filter(|session_id| state.supervisor.delete_session(session_id))
            .cloned()
            .collect();
        let request_id = machine_request_id("provider-uninstall");
        if let Err(error) = state
            .machine_control
            .command_request(
                &machine_id,
                request_id.clone(),
                crate::machine_protocol::MachineCommand::UninstallPlugin {
                    request_id,
                    plugin_id: provider_id.clone(),
                    generation_digest: plan.generation_digest.clone(),
                },
            )
            .await
        {
            return Err(compensate_plugin_uninstall(
                &state,
                &plan,
                &stopped_live_sessions,
                format!("Provider uninstall failed: {error}"),
            )
            .await);
        }
        let store = state
            .store
            .as_ref()
            .ok_or_else(|| "Provider uninstall requires persistence".to_owned())?;
        if let Err(error) = store
            .soft_delete_sessions_until(&plan.session_ids, plan.purge_after_ms)
            .await
        {
            return Err(compensate_plugin_uninstall(
                &state,
                &plan,
                &stopped_live_sessions,
                format!("durable Provider session deletion failed: {error}"),
            )
            .await);
        }
        for session_id in &plan.session_ids {
            state.hub.detach_session(session_id);
        }
        Ok::<(), String>(())
    }
    .await;
    match result {
        Ok(()) => {
            state
                .plugin_lifecycle_fences
                .write()
                .insert(fence, PluginFenceState::Uninstalled);
            Json(serde_json::json!({
                "provider_id": provider_id,
                "machine_id": machine_id,
                "deleted_session_ids": plan.session_ids,
                "purge_after_ms": plan.purge_after_ms,
            }))
            .into_response()
        }
        Err(error) => {
            state.plugin_lifecycle_fences.write().remove(&fence);
            (StatusCode::CONFLICT, error).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
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

async fn api_provider_auth_commit(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Json(request): Json<ProviderAuthCommitRequest>,
) -> Response {
    let Some((_provider_version, _generation_digest, package)) =
        state.provider_catalog.latest_package(&provider_id)
    else {
        return (
            StatusCode::BAD_REQUEST,
            "Provider is known, but no signed runtime release is published in the Catalog",
        )
            .into_response();
    };
    let bundle = crate::machine_protocol::PortableCredentialBundle {
        portable_schema: package.manifest.authentication.portable_schema.clone(),
        method_id: request.method,
        values: request.values,
    };
    let packages = state
        .provider_catalog
        .packages_for_authentication_scope(&package.manifest.authentication.portable_schema);
    let statuses = match if packages.len() == 1 {
        state
            .provider_auth
            .commit(
                &packages[0],
                &bundle,
                request.account_label,
                request.expected_generation,
            )
            .map(|status| vec![status])
    } else {
        state.provider_auth.commit_shared(
            &packages,
            &bundle,
            request.account_label,
            &provider_id,
            request.expected_generation,
        )
    } {
        Ok(statuses) => statuses,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let (statuses, replicas) = distribute_and_mark_provider_auth(&state, statuses).await;
    rebind_unstarted_provider_sessions(&state, &statuses, &replicas);
    let Some(status) = statuses
        .iter()
        .find(|status| status.provider_id == provider_id)
        .cloned()
    else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "shared Provider authentication commit omitted requested Provider",
        )
            .into_response();
    };
    let replicas_succeeded: usize = replicas.values().map(|value| value.succeeded).sum();
    let replicas_failed: usize = replicas.values().map(|value| value.failed).sum();
    let replicas_pending: usize = replicas.values().map(|value| value.pending).sum();
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "authentication": status,
            "authentications": statuses,
            "replicas": replicas,
            "replicas_succeeded": replicas_succeeded,
            "replicas_failed": replicas_failed,
            "replicas_pending": replicas_pending,
        })),
    )
        .into_response()
}

async fn api_provider_auth_logout(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
) -> Response {
    let scope = match state.provider_auth.status(&provider_id) {
        Some(status) => status.authentication_scope,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "Provider is not authenticated at Cowboy Service scope",
            )
                .into_response();
        }
    };
    let provider_ids: Vec<_> = state
        .provider_catalog
        .provider_ids_for_authentication_scope(&scope);
    let provider_ids: Vec<_> = provider_ids
        .into_iter()
        .filter(|candidate| state.provider_auth.status(candidate).is_some())
        .collect();
    if provider_ids.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "authentication scope has no durable Provider state",
        )
            .into_response();
    }
    let statuses = match if provider_ids.len() == 1 {
        state
            .provider_auth
            .logout(&provider_ids[0])
            .map(|status| vec![status])
    } else {
        state.provider_auth.logout_shared(&provider_ids)
    } {
        Ok(statuses) => statuses,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let (statuses, replicas) = distribute_and_mark_provider_auth(&state, statuses).await;
    let Some(status) = statuses
        .iter()
        .find(|status| status.provider_id == provider_id)
        .cloned()
    else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "shared Provider authentication logout omitted requested Provider",
        )
            .into_response();
    };
    let replicas_succeeded: usize = replicas.values().map(|value| value.succeeded).sum();
    let replicas_failed: usize = replicas.values().map(|value| value.failed).sum();
    let replicas_pending: usize = replicas.values().map(|value| value.pending).sum();
    Json(serde_json::json!({
        "authentication": status,
        "authentications": statuses,
        "replicas": replicas,
        "replicas_succeeded": replicas_succeeded,
        "replicas_failed": replicas_failed,
        "replicas_pending": replicas_pending,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
struct ProviderAuthStartRequest {
    method: String,
    provider_version: String,
    generation_digest: String,
}

async fn api_provider_auth_start(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Json(request): Json<ProviderAuthStartRequest>,
) -> Response {
    let Some(package) = state.provider_catalog.package(
        &provider_id,
        &request.provider_version,
        &request.generation_digest,
    ) else {
        return (
            StatusCode::BAD_REQUEST,
            "Provider release is not in the Catalog",
        )
            .into_response();
    };
    if !package
        .manifest
        .authentication
        .methods
        .iter()
        .any(|method| method.id == request.method)
    {
        return (
            StatusCode::BAD_REQUEST,
            "authentication method is not declared by this Provider",
        )
            .into_response();
    }
    let timestamp = now_ms();
    let reconciliation = {
        let mut executors = state.provider_auth_executors.lock();
        reconcile_provider_auth_executors(&mut executors, &provider_id, timestamp)
    };
    let active_request_id = state
        .provider_auth
        .active_authentication_request(&provider_id);
    let resumable = reconciliation.resumable(active_request_id.as_deref());
    for (expired_provider, expired_request) in reconciliation.expired {
        state
            .provider_auth
            .cancel_authentication(&expired_provider, &expired_request);
    }
    if let Some((request_id, executor)) = resumable {
        return resumed_provider_authentication_response(request_id, &executor);
    }
    let candidates = state.machine_control.connected_machine_ids();
    let mut executor = None;
    for machine_id in candidates {
        if current_machine_plugin(&state, &machine_id, &provider_id)
            .await
            .is_ok_and(|installed| {
                installed.plugin_version == request.provider_version
                    && installed.generation_digest == request.generation_digest
            })
        {
            executor = Some(machine_id);
            break;
        }
    }
    let Some(machine_id) = executor else {
        return (
            StatusCode::CONFLICT,
            "No connected Machine has this exact Agent Plugin release installed for temporary authentication",
        )
            .into_response();
    };
    let request_id = machine_request_id("provider-login");
    let expected_generation = state
        .provider_auth
        .status(&provider_id)
        .map_or(0, |status| status.auth_generation);
    let timestamp = now_ms();
    let expires_at_ms = timestamp.saturating_add(15 * 60 * 1_000);
    let mut executors = state.provider_auth_executors.lock();
    let reconciliation = reconcile_provider_auth_executors(&mut executors, &provider_id, timestamp);
    let active_request_id = state
        .provider_auth
        .active_authentication_request(&provider_id);
    if let Some((active_request_id, active_executor)) =
        reconciliation.resumable(active_request_id.as_deref())
    {
        drop(executors);
        for (expired_provider, expired_request) in reconciliation.expired {
            state
                .provider_auth
                .cancel_authentication(&expired_provider, &expired_request);
        }
        return resumed_provider_authentication_response(active_request_id, &active_executor);
    }
    executors.insert(
        request_id.clone(),
        ProviderAuthExecutor {
            machine_id: machine_id.clone(),
            provider_id: provider_id.clone(),
            provider_version: request.provider_version.clone(),
            generation_digest: request.generation_digest.clone(),
            auth_contract_fingerprint: package
                .manifest
                .compatibility
                .auth_contract_fingerprint
                .clone(),
            auth_method: request.method.clone(),
            expected_generation,
            promotion_started: false,
            expires_at_ms,
        },
    );
    drop(executors);
    for (expired_provider, expired_request) in reconciliation.expired {
        state
            .provider_auth
            .cancel_authentication(&expired_provider, &expired_request);
    }
    if let Err(error) = state
        .provider_auth
        .begin_authentication(&package, &request_id)
    {
        state.provider_auth_executors.lock().remove(&request_id);
        return (StatusCode::CONFLICT, error.to_string()).into_response();
    }
    if let Err(error) = state.machine_control.send(
        &machine_id,
        crate::machine_protocol::MachineCommand::BeginLogin {
            request_id: request_id.clone(),
            provider: provider_id.clone(),
            auth_method: Some(request.method.clone()),
        },
    ) {
        state.provider_auth_executors.lock().remove(&request_id);
        state
            .provider_auth
            .cancel_authentication(&provider_id, &request_id);
        return (StatusCode::CONFLICT, error).into_response();
    }
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "request_id": request_id,
            "expires_at_ms": expires_at_ms,
            "method": request.method,
            "resumed": false,
        })),
    )
        .into_response()
}

async fn api_provider_auth_events(
    State(state): State<Arc<AppState>>,
    Path((provider_id, request_id)): Path<(String, String)>,
) -> Response {
    let executor = state
        .provider_auth_executors
        .lock()
        .get(&request_id)
        .cloned();
    let Some(executor) = executor.filter(|value| value.provider_id == provider_id) else {
        return (
            StatusCode::NOT_FOUND,
            "authentication request is not active",
        )
            .into_response();
    };
    if executor.expires_at_ms < now_ms() {
        state.provider_auth_executors.lock().remove(&request_id);
        state
            .provider_auth
            .cancel_authentication(&provider_id, &request_id);
        let _ = state.machine_control.send(
            &executor.machine_id,
            crate::machine_protocol::MachineCommand::CancelLogin {
                request_id: request_id.clone(),
            },
        );
        return (StatusCode::NOT_FOUND, "authentication request expired").into_response();
    }
    let events: Vec<_> = state
        .machine_control
        .events(&executor.machine_id)
        .into_iter()
        .filter(|event| match event {
            crate::machine_protocol::MachineEvent::LoginChallenge { request_id: id, .. }
            | crate::machine_protocol::MachineEvent::LoginState { request_id: id, .. }
            | crate::machine_protocol::MachineEvent::CommandResult { request_id: id, .. } => {
                id == &request_id
            }
            _ => false,
        })
        .collect();
    Json(serde_json::json!({ "request_id": request_id, "events": events })).into_response()
}

#[derive(Debug, Deserialize)]
struct ProviderAuthSubmitRequest {
    code: String,
}

async fn api_provider_auth_submit(
    State(state): State<Arc<AppState>>,
    Path((provider_id, request_id)): Path<(String, String)>,
    Json(request): Json<ProviderAuthSubmitRequest>,
) -> Response {
    let executor = state
        .provider_auth_executors
        .lock()
        .get(&request_id)
        .cloned();
    let Some(executor) = executor
        .filter(|value| value.provider_id == provider_id && value.expires_at_ms >= now_ms())
    else {
        return (
            StatusCode::NOT_FOUND,
            "authentication request is not active",
        )
            .into_response();
    };
    match state.machine_control.send(
        &executor.machine_id,
        crate::machine_protocol::MachineCommand::SubmitLoginCode {
            request_id,
            code: request.code,
        },
    ) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

async fn api_provider_auth_cancel(
    State(state): State<Arc<AppState>>,
    Path((provider_id, request_id)): Path<(String, String)>,
) -> Response {
    let executor = {
        let mut executors = state.provider_auth_executors.lock();
        if executors
            .get(&request_id)
            .is_some_and(|value| value.provider_id == provider_id)
        {
            executors.remove(&request_id)
        } else {
            None
        }
    };
    let Some(executor) = executor else {
        return (
            StatusCode::NOT_FOUND,
            "authentication request is not active",
        )
            .into_response();
    };
    state
        .provider_auth
        .cancel_authentication(&provider_id, &request_id);
    match state.machine_control.send(
        &executor.machine_id,
        crate::machine_protocol::MachineCommand::CancelLogin { request_id },
    ) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

#[derive(Debug, Default, Serialize)]
struct ProviderDistributionOutcome {
    succeeded: usize,
    failed: usize,
    pending: usize,
    #[serde(skip)]
    synchronized_generations: BTreeMap<String, u64>,
}

async fn distribute_and_mark_provider_auth(
    state: &Arc<AppState>,
    statuses: Vec<crate::provider_service::ProviderAuthenticationStatus>,
) -> (
    Vec<crate::provider_service::ProviderAuthenticationStatus>,
    BTreeMap<String, ProviderDistributionOutcome>,
) {
    let mut next = Vec::with_capacity(statuses.len());
    let mut replicas = BTreeMap::new();
    for status in statuses {
        let provider_id = status.provider_id.clone();
        let outcome = distribute_provider_auth(state, &provider_id).await;
        match state.provider_auth.mark_distribution(
            &provider_id,
            status.auth_generation,
            outcome.state(),
        ) {
            Ok(marked) => {
                replicas.insert(provider_id, outcome);
                next.push(marked);
            }
            Err(error) => {
                tracing::warn!(%error, %provider_id, "ignoring stale Provider auth distribution result");
                next.push(state.provider_auth.status(&provider_id).unwrap_or(status));
            }
        }
    }
    (next, replicas)
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

fn provider_auth_rebind_generations(
    session: &crate::core::SessionMeta,
    status: &crate::provider_service::ProviderAuthenticationStatus,
    synchronized_generations: &BTreeMap<String, u64>,
    pinned_auth_contract_fingerprint: &str,
) -> Option<(u64, u64)> {
    let expected_generation = session.provider_auth_generation?;
    (status.provider_id == session.provider
        && status.authentication_state
            == crate::provider_service::ServiceAuthenticationState::Ready
        && status.auth_generation > expected_generation
        && synchronized_generations.get(&session.machine_id) == Some(&status.auth_generation)
        && status.auth_contract_fingerprint == pinned_auth_contract_fingerprint)
        .then_some((expected_generation, status.auth_generation))
}

fn rebindable_provider_auth_failure(
    hub: &Hub,
    session_id: &str,
) -> Option<((Status, u64), String)> {
    let revision @ (Status::Crashed, _) = hub.status_revision(session_id)? else {
        return None;
    };
    let detail = hub.latest_crash_detail(session_id)?;
    crate::provider_behavior::is_provider_auth_required_error(&detail).then_some((revision, detail))
}

fn rebind_unstarted_provider_sessions(
    state: &AppState,
    statuses: &[crate::provider_service::ProviderAuthenticationStatus],
    replicas: &BTreeMap<String, ProviderDistributionOutcome>,
) -> Vec<String> {
    let mut rebound = Vec::new();
    for session in state.hub.session_list() {
        let Some((status_revision, crash_detail)) =
            rebindable_provider_auth_failure(&state.hub, &session.id)
        else {
            continue;
        };
        let Some(status) = statuses
            .iter()
            .find(|status| status.provider_id == session.provider)
        else {
            continue;
        };
        let Some(outcome) = replicas.get(&session.provider) else {
            continue;
        };
        let Some(package) = state.provider_catalog.package(
            &session.provider,
            &session.provider_version,
            &session.provider_generation_digest,
        ) else {
            continue;
        };
        let Some((expected_generation, next_generation)) = provider_auth_rebind_generations(
            &session,
            status,
            &outcome.synchronized_generations,
            &package.manifest.compatibility.auth_contract_fingerprint,
        ) else {
            continue;
        };
        match state.hub.rebind_provider_auth_generation(
            &session.id,
            status_revision,
            &crash_detail,
            expected_generation,
            next_generation,
        ) {
            Ok(true) => rebound.push(session.id),
            Ok(false) => {}
            Err(error) => tracing::warn!(
                session = %session.id,
                provider = %session.provider,
                %error,
                "rebinding refreshed Provider authentication"
            ),
        }
    }
    if !rebound.is_empty() {
        tracing::info!(sessions = ?rebound, "rebound unstarted sessions to refreshed Provider authentication");
    }
    rebound
}

async fn distribute_provider_auth(
    state: &Arc<AppState>,
    provider_id: &str,
) -> ProviderDistributionOutcome {
    let connected = state.machine_control.connected_machine_ids();
    let Some(store) = state.store.as_ref() else {
        return ProviderDistributionOutcome {
            failed: 1,
            ..ProviderDistributionOutcome::default()
        };
    };
    let enrolled = match store.list_machines().await {
        Ok(machines) => machines,
        Err(error) => {
            tracing::warn!(%error, %provider_id, "listing enrolled Machines for Provider auth sync");
            return ProviderDistributionOutcome {
                failed: 1,
                ..ProviderDistributionOutcome::default()
            };
        }
    };
    let mut outcome = ProviderDistributionOutcome::default();
    for machine in enrolled.into_iter().filter(|machine| !machine.revoked) {
        let machine_id = machine.id;
        if !connected.contains(&machine_id) {
            outcome.pending += 1;
            continue;
        }
        match sync_provider_auth_to_machine(state, &machine_id, provider_id).await {
            Ok(auth_generation) => {
                outcome.succeeded += 1;
                outcome
                    .synchronized_generations
                    .insert(machine_id, auth_generation);
            }
            Err(error) => {
                outcome.failed += 1;
                tracing::warn!(%error, %machine_id, %provider_id, "Provider auth replica sync failed");
            }
        }
    }
    outcome
}

async fn sync_provider_auth_to_machine(
    state: &Arc<AppState>,
    machine_id: &str,
    provider_id: &str,
) -> Result<u64, String> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(|| "Provider authentication sync requires persistence".to_owned())?;
    let public_key = store
        .machine_encryption_public_key(machine_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Machine {machine_id:?} has no enrolled encryption key"))?;
    let envelope = state
        .provider_auth
        .seal_for_machine(provider_id, &public_key)
        .map_err(|error| error.to_string())?;
    let auth_generation = envelope.auth_generation;
    let request_id = machine_request_id("provider-auth");
    state
        .machine_control
        .command_request(
            machine_id,
            request_id.clone(),
            crate::machine_protocol::MachineCommand::ApplyProviderAuth {
                request_id,
                envelope: Box::new(envelope),
            },
        )
        .await?;
    Ok(auth_generation)
}

fn failed_provider_auth_projection_ids(
    plugins: &[crate::machine_protocol::PluginInventory],
) -> Vec<String> {
    plugins
        .iter()
        .filter(|plugin| {
            plugin.state == crate::machine_protocol::PluginInstallationState::Active
                && plugin.replica_state == crate::machine_protocol::ProviderReplicaState::Current
                && plugin.materialization_state
                    == crate::machine_protocol::ProviderMaterializationState::Failed
        })
        .map(|plugin| plugin.plugin_id.clone())
        .collect()
}

#[allow(clippy::too_many_arguments)]
async fn accept_provider_auth_refresh_candidate(
    state: &Arc<AppState>,
    machine_id: &str,
    installed: Option<crate::machine_protocol::PluginInventory>,
    request_id: &str,
    provider_id: &str,
    expected_generation: u64,
    provider_version: &str,
    generation_digest: &str,
    auth_contract_fingerprint: &str,
    portable_schema: &str,
    auth_method: &str,
    values: BTreeMap<String, String>,
) -> Result<(), String> {
    let installed = installed
        .filter(|installed| {
            installed.plugin_id == provider_id
                && installed.plugin_version == provider_version
                && installed.generation_digest == generation_digest
                && installed.state == crate::machine_protocol::PluginInstallationState::Active
                && installed.auth_generation == Some(expected_generation)
                && installed.replica_state == crate::machine_protocol::ProviderReplicaState::Current
        })
        .ok_or_else(|| {
            "credential refresh does not match the Machine's active Provider generation".to_owned()
        })?;
    let package = state
        .provider_catalog
        .package(provider_id, provider_version, generation_digest)
        .ok_or_else(|| {
            "credential refresh references an untrusted Agent Plugin release".to_owned()
        })?;
    if package.manifest.authentication.refresh
        != cowboy_provider_sdk::RefreshOwnership::CompareAndSwap
    {
        return Err("Provider does not allow Machine credential refresh".to_owned());
    }
    if package.manifest.compatibility.auth_contract_fingerprint != auth_contract_fingerprint
        || package.manifest.authentication.portable_schema != portable_schema
        || installed.auth_generation != Some(expected_generation)
    {
        return Err("credential refresh contract does not match the active Provider".to_owned());
    }
    let bundle = crate::machine_protocol::PortableCredentialBundle {
        portable_schema: portable_schema.to_owned(),
        method_id: auth_method.to_owned(),
        values,
    };
    let packages = state
        .provider_catalog
        .packages_for_authentication_scope(portable_schema);
    match state
        .provider_auth
        .compare_and_swap_refresh(&packages, &bundle, provider_id, expected_generation)
        .map_err(|error| error.to_string())?
    {
        crate::provider_service::ProviderAuthRefreshResult::Stale { current_generation } => {
            tracing::info!(
                %request_id,
                %machine_id,
                %provider_id,
                expected_generation,
                current_generation,
                "discarding stale Provider credential refresh and restoring Service generation"
            );
            sync_provider_auth_to_machine(state, machine_id, provider_id).await?;
        }
        crate::provider_service::ProviderAuthRefreshResult::Unchanged(_) => {
            tracing::debug!(
                %request_id,
                %machine_id,
                %provider_id,
                expected_generation,
                "ignoring unchanged Provider credential refresh"
            );
        }
        crate::provider_service::ProviderAuthRefreshResult::Updated(statuses) => {
            let next_generation = statuses
                .iter()
                .map(|status| status.auth_generation)
                .max()
                .unwrap_or(expected_generation);
            tracing::info!(
                %request_id,
                %machine_id,
                %provider_id,
                expected_generation,
                next_generation,
                "promoted Machine-refreshed Provider credentials"
            );
            let (statuses, replicas) = distribute_and_mark_provider_auth(state, statuses).await;
            rebind_unstarted_provider_sessions(state, &statuses, &replicas);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn accept_service_auth_candidate(
    state: &Arc<AppState>,
    machine_id: &str,
    request_id: &str,
    provider_id: &str,
    auth_method: &str,
    provider_version: &str,
    generation_digest: &str,
    auth_contract_fingerprint: &str,
    portable_schema: &str,
    values: BTreeMap<String, String>,
    account_label: Option<String>,
) -> Result<(), String> {
    let executor = {
        let mut executors = state.provider_auth_executors.lock();
        let executor = executors
            .get_mut(request_id)
            .ok_or_else(|| "authentication executor is no longer active".to_owned())?;
        if !executor.accepts_candidate(
            machine_id,
            provider_id,
            auth_method,
            provider_version,
            generation_digest,
            auth_contract_fingerprint,
        ) {
            return Err("authentication candidate does not match its executor".to_owned());
        }
        if executor.promotion_started {
            return Ok(());
        }
        executor.promotion_started = true;
        executor.clone()
    };
    let package = state
        .provider_catalog
        .package(provider_id, provider_version, generation_digest)
        .ok_or_else(|| {
            "authentication candidate references an untrusted Agent Plugin release".to_owned()
        })?;
    if package.manifest.compatibility.auth_contract_fingerprint != auth_contract_fingerprint {
        return Err("authentication candidate contract fingerprint mismatch".to_owned());
    }
    if package.manifest.authentication.portable_schema != portable_schema {
        return Err("authentication candidate portable schema mismatch".to_owned());
    }
    let bundle = crate::machine_protocol::PortableCredentialBundle {
        portable_schema: portable_schema.to_owned(),
        method_id: auth_method.to_owned(),
        values,
    };
    let packages = state
        .provider_catalog
        .packages_for_authentication_scope(portable_schema);
    let statuses = (if packages.len() == 1 {
        state
            .provider_auth
            .commit(
                &packages[0],
                &bundle,
                account_label.clone(),
                Some(executor.expected_generation),
            )
            .map(|status| vec![status])
    } else {
        state.provider_auth.commit_shared(
            &packages,
            &bundle,
            account_label.clone(),
            provider_id,
            Some(executor.expected_generation),
        )
    })
    .map_err(|error| error.to_string())?;
    let (statuses, replicas) = distribute_and_mark_provider_auth(state, statuses).await;
    rebind_unstarted_provider_sessions(state, &statuses, &replicas);
    let mut warnings = Vec::new();
    if !statuses
        .iter()
        .any(|status| status.provider_id == provider_id)
    {
        warnings.push("shared Provider authentication status is incomplete".to_owned());
    }
    let finalize_request_id = machine_request_id("provider-auth-finalize");
    if let Err(error) = state
        .machine_control
        .command_request(
            machine_id,
            finalize_request_id.clone(),
            crate::machine_protocol::MachineCommand::FinalizeProviderAuthCandidate {
                request_id: finalize_request_id,
                provider_id: provider_id.to_owned(),
                auth_method: auth_method.to_owned(),
                candidate_request_id: request_id.to_owned(),
            },
        )
        .await
    {
        tracing::warn!(%error, %provider_id, %request_id, "cleaning temporary Provider authentication home");
        warnings.push(format!("temporary executor cleanup is pending: {error}"));
    }
    state.machine_control.record(
        machine_id,
        crate::machine_protocol::MachineEvent::LoginState {
            request_id: request_id.to_owned(),
            provider: provider_id.to_owned(),
            state: crate::machine_protocol::AuthState::SignedIn,
            account_label,
            detail: Some(if warnings.is_empty() {
                "authentication is owned and synchronized by Cowboy Service".to_owned()
            } else {
                format!(
                    "authentication is owned by Cowboy Service; {}",
                    warnings.join("; ")
                )
            }),
        },
    );
    Ok(())
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

async fn api_machine_service(State(state): State<Arc<AppState>>) -> Json<MachineServiceResponse> {
    Json(MachineServiceResponse {
        service_id: state.service_id.clone(),
    })
}

async fn api_machine_enroll(
    State(state): State<Arc<AppState>>,
    Json(request): Json<MachineEnrollRequest>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Machine enrollment requires persistence",
        )
            .into_response();
    };
    let public_key = match crate::machine_auth::validate_public_key(&request.public_key) {
        Ok(public_key) => public_key,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    match store
        .consume_machine_enrollment(&request.token, &public_key, &request.encryption_public_key)
        .await
    {
        Ok(machine) => {
            let response = MachineEnrollResponse {
                service_id: state.service_id.clone(),
                machine_id: machine.id,
                display_name: machine.display_name,
                fingerprint: machine.fingerprint,
            };
            state.machine_snapshots.publish().await;
            (StatusCode::CREATED, Json(response)).into_response()
        }
        Err(error) => {
            tracing::warn!(%error, "Machine enrollment rejected");
            (
                StatusCode::UNAUTHORIZED,
                "invalid or expired enrollment token",
            )
                .into_response()
        }
    }
}

// Existing Machine clients collect their bounded CLI/auth inventory after
// receiving the challenge. Those probes can legitimately exceed 15 seconds on
// a loaded macOS host, so keep the nonce window above their worst-case budget.
const MACHINE_HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
const MACHINE_HEARTBEAT_MS: u64 = 15_000;
const WEBSOCKET_FRAME_SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const MACHINE_RUNTIME_OUTBOUND_CAPACITY: usize = 16;

async fn forward_machine_runtime_frames<R>(
    mut reader: crate::runtime_wire::FrameReader<R>,
    tx: mpsc::Sender<crate::runtime_wire::Frame>,
) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    loop {
        let frame = reader
            .next()
            .await
            .context("reading Controller runtime frame")?
            .ok_or_else(|| anyhow::anyhow!("Controller runtime tunnel closed"))?;
        tx.send(frame)
            .await
            .map_err(|_| anyhow::anyhow!("Machine WebSocket runtime bridge closed"))?;
    }
}

async fn write_machine_runtime_frames<W>(
    mut writer: W,
    mut rx: mpsc::UnboundedReceiver<crate::runtime_wire::Frame>,
) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    while let Some(frame) = rx.recv().await {
        tokio::time::timeout(
            WEBSOCKET_FRAME_SEND_TIMEOUT,
            crate::runtime_wire::write_frame(&mut writer, &frame),
        )
        .await
        .map_err(|_| anyhow::anyhow!("writing Machine runtime frame timed out"))?
        .context("writing Machine runtime frame")?;
    }
    Ok(())
}

#[cfg(test)]
mod machine_runtime_bridge_tests {
    use super::{forward_machine_runtime_frames, write_machine_runtime_frames};
    use crate::runtime_wire::{CoreCommand, Frame, FrameReader, read_frame, write_frame};
    use std::time::Duration;

    #[tokio::test]
    async fn runtime_bridge_makes_full_duplex_progress_under_backpressure() {
        // A tiny transport makes two multi-kilobyte frames deterministically
        // reproduce the full-duplex write cycle seen on Hawk.
        let (core, tunnel) = tokio::io::duplex(128);
        let (mut core_reader, mut core_writer) = tokio::io::split(core);
        let (tunnel_reader, tunnel_writer) = tokio::io::split(tunnel);
        let controller_frame = Frame::CoreCommand {
            command: CoreCommand::SetDesiredGeneration {
                generation: "controller".repeat(1_024),
                worker_command: None,
            },
        };
        let machine_frame = Frame::Reject {
            reason: "machine".repeat(1_024),
        };
        let expected_controller_frame = controller_frame.clone();
        let expected_machine_frame = machine_frame.clone();
        let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel(1);
        let forwarder = tokio::spawn(forward_machine_runtime_frames(
            FrameReader::new(tunnel_reader),
            outbound_tx,
        ));
        let (inbound_tx, inbound_rx) = tokio::sync::mpsc::unbounded_channel();
        let writer = tokio::spawn(write_machine_runtime_frames(tunnel_writer, inbound_rx));
        let controller = tokio::spawn(async move {
            write_frame(&mut core_writer, &controller_frame)
                .await
                .expect("write Controller frame");
            read_frame(&mut core_reader)
                .await
                .expect("read Machine frame")
                .expect("Machine frame")
        });

        let (forwarded, received) = tokio::time::timeout(Duration::from_secs(1), async {
            inbound_tx.send(machine_frame).expect("queue Machine frame");
            let forwarded = outbound_rx
                .recv()
                .await
                .expect("forwarded Controller frame");
            let received = controller.await.expect("Controller task");
            (forwarded, received)
        })
        .await
        .expect("full-duplex bridge must make progress");

        assert_eq!(forwarded, expected_controller_frame);
        assert_eq!(received, expected_machine_frame);
        forwarder.abort();
        writer.abort();
    }
}

async fn machine_ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_machine_ws(socket, state))
}

fn random_machine_token() -> anyhow::Result<String> {
    let mut random = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .context("opening OS randomness")?
        .read_exact(&mut random)
        .context("reading OS randomness")?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random))
}

async fn handle_machine_ws(mut socket: WebSocket, state: Arc<AppState>) {
    let Some(store) = state.store.as_ref().cloned() else {
        let _ = send_json(
            &mut socket,
            &crate::machine_protocol::MachineFrame::Reject {
                reason: "Machine connections require persistence".to_owned(),
            },
        )
        .await;
        return;
    };
    let challenge_id = match random_machine_token() {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(%error, "creating Machine challenge");
            return;
        }
    };
    let nonce = match random_machine_token() {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(%error, "creating Machine nonce");
            return;
        }
    };
    let expires_at_ms = now_ms().saturating_add(MACHINE_HANDSHAKE_TIMEOUT.as_millis() as i64);
    let challenge = crate::machine_protocol::MachineFrame::Challenge {
        challenge_id: challenge_id.clone(),
        nonce: nonce.clone(),
        expires_at_ms,
        proof_version: 2,
    };
    if send_json(&mut socket, &challenge).await.is_err() {
        return;
    }
    let incoming = match tokio::time::timeout(MACHINE_HANDSHAKE_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => text,
        Ok(Some(Ok(_))) => {
            tracing::warn!("Machine handshake received a non-text hello");
            return;
        }
        Ok(Some(Err(error))) => {
            tracing::warn!(%error, "reading Machine hello");
            return;
        }
        Ok(None) => {
            tracing::warn!("Machine disconnected before sending hello");
            return;
        }
        Err(_) => {
            tracing::warn!("Machine hello timed out while collecting host inventory");
            return;
        }
    };
    let crate::machine_protocol::MachineFrame::Hello { hello } =
        (match serde_json::from_str(&incoming) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::warn!(%error, "invalid Machine hello");
                return;
            }
        })
    else {
        return;
    };
    if hello.challenge_id.as_deref() != Some(&challenge_id) {
        return;
    }
    let Some(signature) = hello.challenge_signature.as_deref() else {
        return;
    };
    let public_key = match store.machine_public_key(&hello.machine_id).await {
        Ok(Some(value)) => value,
        Ok(None) => return,
        Err(error) => {
            tracing::error!(%error, machine = %hello.machine_id, "loading Machine identity");
            return;
        }
    };
    if now_ms() > expires_at_ms {
        return;
    }
    let proof = if hello.max_protocol >= 3 {
        crate::machine_protocol::challenge_proof_v2(&challenge_id, &nonce, expires_at_ms, &hello)
    } else {
        crate::machine_protocol::challenge_proof_v1(&challenge_id, &nonce, expires_at_ms, &hello)
    };
    let signature = signature.to_owned();
    let verified = tokio::task::spawn_blocking(move || {
        crate::machine_auth::verify(&public_key, &proof, &signature)
    })
    .await;
    if !matches!(verified, Ok(Ok(true))) {
        tracing::warn!(machine = %hello.machine_id, "Machine challenge verification failed");
        return;
    }
    if hello.max_protocol >= 3 {
        let Some(encryption_public_key) = hello.encryption_public_key.as_deref() else {
            tracing::warn!(machine = %hello.machine_id, "protocol-three Machine omitted encryption key");
            return;
        };
        if let Err(error) = store
            .bind_machine_encryption_public_key(&hello.machine_id, encryption_public_key)
            .await
        {
            tracing::warn!(%error, machine = %hello.machine_id, "Machine encryption key rejected");
            return;
        }
    }
    let Some(protocol) = crate::machine_protocol::negotiate(
        crate::machine_protocol::MIN_MACHINE_PROTOCOL_VERSION,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        hello.min_protocol,
        hello.max_protocol,
    ) else {
        let _ = send_json(
            &mut socket,
            &crate::machine_protocol::MachineFrame::Reject {
                reason: "no compatible Machine protocol".to_owned(),
            },
        )
        .await;
        return;
    };
    let platform = match hello.platform {
        crate::machine_protocol::Platform::Linux => "linux",
        crate::machine_protocol::Platform::Macos => "macos",
    };
    let connection_mode = match hello.connection_mode {
        crate::machine_protocol::ConnectionMode::LocalUds => "local",
        crate::machine_protocol::ConnectionMode::OutboundTls => "outbound_wss",
    };
    let inventory = serde_json::json!({
        "components": &hello.components,
        "plugins": &hello.plugins,
        "provider_contracts": &hello.provider_contracts,
        "workspaces": &hello.workspaces,
        "workspace_revision": &hello.workspace_revision,
        "capacity": &hello.capacity,
    });
    if let Err(error) = store
        .machine_connected(
            &hello.machine_id,
            &challenge_id,
            platform,
            &hello.arch,
            connection_mode,
            &inventory,
        )
        .await
    {
        tracing::error!(%error, machine = %hello.machine_id, "recording Machine connection");
        return;
    }
    if send_json(
        &mut socket,
        &crate::machine_protocol::MachineFrame::Welcome {
            protocol,
            controller_epoch: 0,
            heartbeat_interval_ms: MACHINE_HEARTBEAT_MS,
            desired_components: state
                .desired_machine_components
                .iter()
                .filter(|component| component.automatic)
                .cloned()
                .collect(),
        },
    )
    .await
    .is_err()
    {
        let _ = store
            .machine_disconnected(
                &hello.machine_id,
                &challenge_id,
                MACHINE_RECONNECT_GRACE_SECONDS,
            )
            .await;
        state.machine_snapshots.publish().await;
        return;
    }
    tracing::info!(machine = %hello.machine_id, "Machine connected");
    let (machine_command_tx, mut machine_command_rx) = mpsc::unbounded_channel();
    state.machine_control.install(
        hello.machine_id.clone(),
        challenge_id.clone(),
        connection_mode == "local",
        protocol,
        machine_command_tx,
    );
    state.machine_control.record(
        &hello.machine_id,
        crate::machine_protocol::MachineEvent::Inventory {
            components: hello.components.clone(),
            workspaces: Some(hello.workspaces.clone()),
            workspace_revision: hello.workspace_revision.clone(),
            observed_at_ms: now_ms(),
        },
    );
    state.machine_snapshots.publish().await;
    state.machine_control.record(
        &hello.machine_id,
        crate::machine_protocol::MachineEvent::PluginInventory {
            plugins: hello.plugins.clone(),
            observed_at_ms: now_ms(),
        },
    );
    for authentication in state.provider_auth.replica_statuses() {
        let sync_state = Arc::clone(&state);
        let machine_id = hello.machine_id.clone();
        tokio::spawn(async move {
            let outcome = distribute_provider_auth(&sync_state, &authentication.provider_id).await;
            match sync_state.provider_auth.mark_distribution(
                &authentication.provider_id,
                authentication.auth_generation,
                outcome.state(),
            ) {
                Ok(marked) => {
                    let replicas = BTreeMap::from([(marked.provider_id.clone(), outcome)]);
                    rebind_unstarted_provider_sessions(&sync_state, &[marked], &replicas);
                }
                Err(error) => {
                    tracing::warn!(%error, %machine_id, provider_id = %authentication.provider_id, "recording reconnected Machine Provider auth convergence");
                }
            }
        });
    }
    let (runtime_core, runtime_tunnel) = match UnixStream::pair() {
        Ok(pair) => pair,
        Err(error) => {
            tracing::error!(%error, machine = %hello.machine_id, "creating Machine runtime tunnel");
            return;
        }
    };
    let (runtime_reader, runtime_writer) = runtime_tunnel.into_split();
    let runtime_reader = crate::runtime_wire::FrameReader::new(runtime_reader);
    // Drain Controller -> Machine runtime frames independently from the main
    // Machine WebSocket loop. Without this pump, a large frame arriving from
    // Machine can fill runtime_writer while RemoteRuntime simultaneously fills
    // the opposite half of the same socketpair; both tasks then await write
    // capacity and neither returns to its read branch.
    let (runtime_frame_tx, mut runtime_frame_rx) = mpsc::channel(MACHINE_RUNTIME_OUTBOUND_CAPACITY);
    let mut runtime_forwarder = tokio::spawn(forward_machine_runtime_frames(
        runtime_reader,
        runtime_frame_tx,
    ));
    // Keep Machine -> Controller writes out of the WebSocket loop as well. A
    // blocked internal consumer must not stop that loop from forwarding the
    // opposing direction, which is what lets the consumer make progress.
    // Startup can replay every unacknowledged worker event before
    // RemoteRuntime finishes its handshake. Do not reject that finite replay
    // based on frame count: the writer's per-frame timeout bounds a genuinely
    // stalled consumer, and disconnect cleanup drops the remaining queue.
    let (runtime_write_tx, runtime_write_rx) = mpsc::unbounded_channel();
    let mut runtime_writer = tokio::spawn(write_machine_runtime_frames(
        runtime_writer,
        runtime_write_rx,
    ));
    let (runtime_tx, mut runtime_rx) = tokio::sync::oneshot::channel();
    {
        let router = Arc::clone(&state.runtime_router);
        let hub = state.hub.clone();
        let provider_auth = Arc::clone(&state.provider_auth);
        let machine_snapshots = state.machine_snapshots.clone();
        let machine_id = hello.machine_id.clone();
        let generation = hello
            .components
            .iter()
            .find(|component| {
                component.id.kind == crate::machine_protocol::ComponentKind::AcpRuntime
                    && component.state == crate::machine_protocol::ComponentState::Active
            })
            .map_or_else(
                || hello.host_build.clone(),
                |component| component.generation.clone(),
            );
        tokio::spawn(async move {
            let label = PathBuf::from(format!("machine://{machine_id}"));
            match RemoteBootstrap::from_stream(label, runtime_core).await {
                Ok(bootstrap) => {
                    // Executable paths are machine-local. The remote broker
                    // registered this generation from its own active
                    // content-addressed component before connecting.
                    let runtime = RemoteRuntime::new_with_provider_auth(
                        hub,
                        &bootstrap,
                        generation,
                        None,
                        provider_auth,
                    );
                    router.install(machine_id, Arc::clone(&runtime));
                    runtime.start(bootstrap);
                    machine_snapshots.publish().await;
                    let _ = runtime_tx.send(runtime);
                }
                Err(error) => {
                    tracing::warn!(%error, machine = %machine_id, "Machine runtime handshake failed");
                }
            }
        });
    }
    let mut connected_runtime: Option<Arc<RemoteRuntime>> = None;
    let mut runtime_registration_pending = true;
    let mut current_components = hello.components.clone();
    let mut current_workspaces = hello.workspaces.clone();
    let mut current_workspace_revision = hello.workspace_revision.clone();
    let mut current_providers = hello.plugins.clone();
    let mut revocation_check = tokio::time::interval(std::time::Duration::from_secs(2));
    revocation_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    revocation_check.tick().await;
    loop {
        let message = tokio::select! {
            message = socket.recv() => Some(message),
            runtime = &mut runtime_rx, if runtime_registration_pending => {
                runtime_registration_pending = false;
                if let Ok(runtime) = runtime {
                    connected_runtime = Some(runtime);
                }
                None
            }
            frame = runtime_frame_rx.recv() => {
                let Some(frame) = frame else { break };
                if send_json(
                    &mut socket,
                    &crate::machine_protocol::MachineFrame::Runtime { frame },
                ).await.is_err() {
                    break;
                }
                continue;
            }
            forwarded = &mut runtime_forwarder => {
                match forwarded {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(%error, machine = %hello.machine_id, "Machine runtime forwarding stopped");
                    }
                    Err(error) => {
                        tracing::warn!(%error, machine = %hello.machine_id, "Machine runtime forwarding task failed");
                    }
                }
                break;
            }
            written = &mut runtime_writer => {
                match written {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(%error, machine = %hello.machine_id, "Machine runtime writer stopped");
                    }
                    Err(error) => {
                        tracing::warn!(%error, machine = %hello.machine_id, "Machine runtime writer task failed");
                    }
                }
                break;
            }
            command = machine_command_rx.recv() => {
                let Some(command) = command else { break };
                if send_json(
                    &mut socket,
                    &crate::machine_protocol::MachineFrame::Command { command },
                ).await.is_err() {
                    break;
                }
                continue;
            }
            _ = revocation_check.tick() => {
                match store.machine_connection_is_current(&hello.machine_id, &challenge_id).await {
                    Ok(true) => continue,
                    Ok(false) => {
                        tracing::info!(machine = %hello.machine_id, "Machine connection fenced");
                        let _ = socket.send(Message::Close(None)).await;
                        break;
                    }
                    Err(error) => {
                        tracing::warn!(%error, machine = %hello.machine_id, "checking Machine revocation");
                        break;
                    }
                }
            }
        };
        let Some(message) = message else {
            continue;
        };
        let Some(message) = message else {
            break;
        };
        let Ok(Message::Text(text)) = message else {
            break;
        };
        let Ok(frame) = serde_json::from_str::<crate::machine_protocol::MachineFrame>(&text) else {
            break;
        };
        let result = match frame {
            crate::machine_protocol::MachineFrame::Heartbeat { .. } => {
                store
                    .machine_seen(&hello.machine_id, &challenge_id, None)
                    .await
            }
            crate::machine_protocol::MachineFrame::Event {
                event:
                    crate::machine_protocol::MachineEvent::Inventory {
                        components,
                        workspaces,
                        workspace_revision,
                        ..
                    },
            } => {
                current_components = components;
                apply_workspace_inventory(
                    &mut current_workspaces,
                    &mut current_workspace_revision,
                    workspaces,
                    workspace_revision,
                );
                state.machine_control.record(
                    &hello.machine_id,
                    crate::machine_protocol::MachineEvent::Inventory {
                        components: current_components.clone(),
                        workspaces: Some(current_workspaces.clone()),
                        workspace_revision: current_workspace_revision.clone(),
                        observed_at_ms: now_ms(),
                    },
                );
                let inventory = serde_json::json!({
                    "components": &current_components,
                    "providers": &current_providers,
                    "provider_contracts": &hello.provider_contracts,
                    "workspaces": &current_workspaces,
                    "workspace_revision": &current_workspace_revision,
                    "capacity": &hello.capacity,
                });
                let result = store
                    .machine_seen(&hello.machine_id, &challenge_id, Some(&inventory))
                    .await;
                if result.is_ok() {
                    state.machine_snapshots.publish().await;
                }
                result
            }
            crate::machine_protocol::MachineFrame::Event {
                event:
                    crate::machine_protocol::MachineEvent::PluginInventory {
                        plugins,
                        observed_at_ms,
                    },
            } => {
                current_providers = plugins;
                let failed_provider_auth = failed_provider_auth_projection_ids(&current_providers);
                state.machine_control.record(
                    &hello.machine_id,
                    crate::machine_protocol::MachineEvent::PluginInventory {
                        plugins: current_providers.clone(),
                        observed_at_ms,
                    },
                );
                let inventory = serde_json::json!({
                    "components": &current_components,
                    "plugins": &current_providers,
                    "provider_contracts": &hello.provider_contracts,
                    "workspaces": &current_workspaces,
                    "workspace_revision": &current_workspace_revision,
                    "capacity": &hello.capacity,
                });
                let result = store
                    .machine_seen(&hello.machine_id, &challenge_id, Some(&inventory))
                    .await;
                if result.is_ok() {
                    state.machine_snapshots.publish().await;
                    for provider_id in failed_provider_auth {
                        let repair_state = Arc::clone(&state);
                        let repair_machine_id = hello.machine_id.clone();
                        tokio::spawn(async move {
                            if let Err(error) = sync_provider_auth_to_machine(
                                &repair_state,
                                &repair_machine_id,
                                &provider_id,
                            )
                            .await
                            {
                                tracing::warn!(
                                    %error,
                                    machine = %repair_machine_id,
                                    %provider_id,
                                    "repairing failed Provider credential projection"
                                );
                            }
                        });
                    }
                }
                result
            }
            crate::machine_protocol::MachineFrame::Event {
                event:
                    crate::machine_protocol::MachineEvent::ProviderUsageBatch {
                        producer_id,
                        first_sequence,
                        last_sequence,
                        events,
                    },
            } => {
                let bounded = events.len() <= 200
                    && events
                        .first()
                        .is_some_and(|event| event.sequence == first_sequence)
                    && events
                        .last()
                        .is_some_and(|event| event.sequence == last_sequence)
                    && events
                        .windows(2)
                        .all(|pair| pair[0].sequence < pair[1].sequence);
                if !bounded {
                    Err(anyhow::anyhow!("invalid provider usage sequence envelope"))
                } else {
                    match store
                        .ingest_provider_usage(&hello.machine_id, &producer_id, &events)
                        .await
                    {
                        Ok(acknowledged) => {
                            let ack = crate::machine_protocol::MachineFrame::Command {
                                command:
                                    crate::machine_protocol::MachineCommand::ProviderUsageAck {
                                        producer_id,
                                        sequence: acknowledged,
                                    },
                            };
                            match send_json(&mut socket, &ack).await {
                                Ok(()) => {
                                    store
                                        .machine_seen(&hello.machine_id, &challenge_id, None)
                                        .await
                                }
                                Err(()) => Err(anyhow::anyhow!(
                                    "Machine disconnected while acknowledging provider usage"
                                )),
                            }
                        }
                        Err(error) => Err(error),
                    }
                }
            }
            crate::machine_protocol::MachineFrame::Event {
                event:
                    crate::machine_protocol::MachineEvent::ServiceAuthCandidate {
                        request_id,
                        provider_id,
                        auth_method,
                        provider_version,
                        generation_digest,
                        auth_contract_fingerprint,
                        portable_schema,
                        bundle,
                        account_label,
                    },
            } => {
                let candidate_state = Arc::clone(&state);
                let candidate_machine_id = hello.machine_id.clone();
                tokio::spawn(async move {
                    if let Err(error) = accept_service_auth_candidate(
                        &candidate_state,
                        &candidate_machine_id,
                        &request_id,
                        &provider_id,
                        &auth_method,
                        &provider_version,
                        &generation_digest,
                        &auth_contract_fingerprint,
                        &portable_schema,
                        bundle,
                        account_label,
                    )
                    .await
                    {
                        tracing::warn!(%error, %provider_id, "temporary Provider login could not be promoted");
                        candidate_state
                            .provider_auth
                            .fail_authentication(&provider_id, &request_id);
                        candidate_state.machine_control.record(
                            &candidate_machine_id,
                            crate::machine_protocol::MachineEvent::LoginState {
                                request_id,
                                provider: provider_id,
                                state: crate::machine_protocol::AuthState::Error,
                                account_label: None,
                                detail: Some(error),
                            },
                        );
                    }
                });
                store
                    .machine_seen(&hello.machine_id, &challenge_id, None)
                    .await
            }
            crate::machine_protocol::MachineFrame::Event {
                event:
                    crate::machine_protocol::MachineEvent::ProviderAuthRefreshCandidate {
                        request_id,
                        provider_id,
                        expected_generation,
                        provider_version,
                        generation_digest,
                        auth_contract_fingerprint,
                        portable_schema,
                        auth_method,
                        bundle,
                    },
            } => {
                let refresh_state = Arc::clone(&state);
                let refresh_machine_id = hello.machine_id.clone();
                let installed = current_providers
                    .iter()
                    .find(|installed| installed.plugin_id == provider_id)
                    .cloned();
                tokio::spawn(async move {
                    if let Err(error) = accept_provider_auth_refresh_candidate(
                        &refresh_state,
                        &refresh_machine_id,
                        installed,
                        &request_id,
                        &provider_id,
                        expected_generation,
                        &provider_version,
                        &generation_digest,
                        &auth_contract_fingerprint,
                        &portable_schema,
                        &auth_method,
                        bundle,
                    )
                    .await
                    {
                        tracing::warn!(
                            %error,
                            machine = %refresh_machine_id,
                            %provider_id,
                            expected_generation,
                            "Machine-refreshed Provider credentials were rejected"
                        );
                        if let Err(reconcile_error) = sync_provider_auth_to_machine(
                            &refresh_state,
                            &refresh_machine_id,
                            &provider_id,
                        )
                        .await
                        {
                            tracing::warn!(
                                error = %reconcile_error,
                                machine = %refresh_machine_id,
                                %provider_id,
                                "restoring authoritative Provider credentials after rejected refresh"
                            );
                        }
                    }
                });
                store
                    .machine_seen(&hello.machine_id, &challenge_id, None)
                    .await
            }
            crate::machine_protocol::MachineFrame::Event { event } => {
                if let crate::machine_protocol::MachineEvent::LoginState {
                    request_id,
                    provider,
                    state: auth_state,
                    ..
                } = &event
                    && matches!(
                        auth_state,
                        crate::machine_protocol::AuthState::SignedOut
                            | crate::machine_protocol::AuthState::Unsupported
                            | crate::machine_protocol::AuthState::Error
                    )
                {
                    state
                        .provider_auth
                        .fail_authentication(provider, request_id);
                }
                state.machine_control.record(&hello.machine_id, event);
                store
                    .machine_seen(&hello.machine_id, &challenge_id, None)
                    .await
            }
            crate::machine_protocol::MachineFrame::Runtime { frame } => {
                if let Err(error) = runtime_write_tx.send(frame) {
                    tracing::warn!(%error, machine = %hello.machine_id, "Machine runtime writer closed");
                    break;
                }
                continue;
            }
            _ => continue,
        };
        if let Err(error) = result {
            tracing::warn!(%error, machine = %hello.machine_id, "updating Machine state");
            break;
        }
    }
    runtime_forwarder.abort();
    runtime_writer.abort();
    if let Some(runtime) = connected_runtime.as_ref() {
        state
            .runtime_router
            .remove_if_current(&hello.machine_id, runtime);
    }
    state
        .machine_control
        .remove_if_current(&hello.machine_id, &challenge_id);
    if let Err(error) = store
        .machine_disconnected(
            &hello.machine_id,
            &challenge_id,
            MACHINE_RECONNECT_GRACE_SECONDS,
        )
        .await
    {
        tracing::warn!(%error, machine = %hello.machine_id, "marking Machine offline");
    }
    state.machine_snapshots.publish().await;
}

async fn api_session_info(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    match state.hub.session_info(&session_id) {
        Some(info) => Json(info).into_response(),
        None => (StatusCode::NOT_FOUND, "unknown session").into_response(),
    }
}

/// Rebuild one session's runtime without clearing its transcript, native agent
/// id, queue, drafts, title, cwd, or persisted config preferences. The
/// Supervisor owns the atomic worker fence/replacement and broadcasts the
/// resulting lifecycle edges to every connected client.
async fn api_session_reload(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    if state.hub.session_info(&session_id).is_none() {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    if plugin_fence_state_for_session(&state.hub, &state.plugin_lifecycle_fences, &session_id)
        .is_some_and(|fence| fence != PluginFenceState::Installing)
    {
        return (
            StatusCode::CONFLICT,
            "the session Provider is uninstalling from its Machine",
        )
            .into_response();
    }
    match state.supervisor.reload_session(&session_id) {
        Ok(()) => (StatusCode::ACCEPTED, "reloading").into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

async fn api_session_cache_protection(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    let Some(session) = state
        .hub
        .session_list()
        .into_iter()
        .find(|session| session.id == session_id)
    else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let configuration = session.provider_behavior.as_ref().map_or_else(
        || crate::provider::legacy_behavior(&session.provider).configuration,
        |behavior| behavior.configuration.clone(),
    );
    if !crate::deepseek_cache::supported_behavior(&configuration) {
        return (
            StatusCode::BAD_REQUEST,
            "cache protection is available only for DeepSeek sessions",
        )
            .into_response();
    }
    let enabled = state
        .hub
        .config_preferences(&session.id)
        .and_then(|preferences| crate::deepseek_cache::selected(&preferences, &configuration))
        .unwrap_or(true);
    if !enabled {
        return Json(serde_json::json!({
            "state": "disabled",
            "algorithm": "adaptive-replay-v1",
            "minimumHitTokens": crate::deepseek_cache::MINIMUM_HIT_TOKENS,
            "contextUsed": session.context_used,
        }))
        .into_response();
    }
    match state
        .machine_control
        .adapter_request(
            &session.machine_id,
            "deepseek-cache-status",
            serde_json::json!({
                "configuration": configuration,
                "sessionId": session.id,
            }),
        )
        .await
    {
        Ok(mut status) => {
            if let Some(object) = status.as_object_mut() {
                object.insert(
                    "minimumHitTokens".to_owned(),
                    crate::deepseek_cache::MINIMUM_HIT_TOKENS.into(),
                );
                object.insert("contextUsed".to_owned(), session.context_used.into());
            }
            Json(status).into_response()
        }
        Err(error) => {
            tracing::warn!(session = %session.id, machine = %session.machine_id, %error, "DeepSeek cache-protection status unavailable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "cache-protection status is temporarily unavailable",
            )
                .into_response()
        }
    }
}

/// Request body for the machine wake (`POST /api/sessions/{id}/prompt`).
#[derive(Debug, Deserialize)]
struct SessionPromptRequest {
    #[serde(default)]
    text: String,
    #[serde(default)]
    content: Vec<serde_json::Value>,
}

/// Inject a prompt into a session FROM THE BACKEND (machine-driven). The
/// This bypasses the WS user-input gate precisely because it is the backend,
/// not an interactive client. Works on any session, including `system` ones.
async fn api_session_prompt(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(req): Json<SessionPromptRequest>,
) -> Response {
    // Keep the read guard through dispatch. Provider uninstall acquires the
    // write guard before it snapshots active turns, so a prompt can never slip
    // between the fence check and the explicit active-turn confirmation.
    let provider_key = provider_fence_key_for_session(&state.hub, &session_id);
    let plugin_prompt_fence = state.plugin_lifecycle_fences.read();
    if provider_key
        .as_ref()
        .and_then(|key| plugin_prompt_fence.get(key))
        .is_some_and(|fence| *fence != PluginFenceState::Installing)
    {
        return (
            StatusCode::CONFLICT,
            "the session Provider is uninstalling from its Machine",
        )
            .into_response();
    }
    let blocks: Vec<ContentBlock> = if req.content.is_empty() {
        if req.text.is_empty() {
            return (StatusCode::BAD_REQUEST, "empty prompt: no text or content").into_response();
        }
        vec![ContentBlock::from(req.text)]
    } else {
        req.content
            .into_iter()
            .filter_map(|v| serde_json::from_value::<ContentBlock>(v).ok())
            .collect()
    };
    match state
        .supervisor
        .send(&session_id, AgentCommand::Prompt(blocks, None, None))
    {
        Ok(()) => (StatusCode::ACCEPTED, "queued").into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
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
    provider: String,
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

fn default_machine_id() -> String {
    "local".to_owned()
}

fn web_session_is_missing_machine(machine_id: &str, origin: &SessionOrigin) -> bool {
    machine_id == "local" && matches!(origin, SessionOrigin::Web)
}

struct ResolvedProviderGeneration {
    version: String,
    digest: String,
    auth_generation: Option<u64>,
    behavior: cowboy_provider_sdk::ProviderBehaviorContract,
}

fn resolve_scheduling_auth_generation(
    installed: &crate::machine_protocol::PluginInventory,
    authentication_required: bool,
    service_auth: Option<&crate::provider_service::ProviderAuthenticationStatus>,
) -> Result<Option<u64>, String> {
    if !authentication_required {
        return Ok(None);
    }
    let provider_id = &installed.plugin_id;
    let service_auth = service_auth.ok_or_else(|| {
        format!("Provider {provider_id:?} is not authenticated at Cowboy Service scope")
    })?;
    if service_auth.authentication_state
        != crate::provider_service::ServiceAuthenticationState::Ready
    {
        return Err(format!(
            "Provider {provider_id:?} authentication is not ready at Cowboy Service scope"
        ));
    }
    if installed.replica_state != crate::machine_protocol::ProviderReplicaState::Current
        || installed.materialization_state
            != crate::machine_protocol::ProviderMaterializationState::Current
    {
        return Err(format!(
            "Provider {provider_id:?} Service authentication is not synchronized to this Machine"
        ));
    }
    let installed_generation = installed
        .auth_generation
        .ok_or_else(|| "Provider auth generation is missing".to_owned())?;
    if installed_generation != service_auth.auth_generation {
        return Err(format!(
            "Provider {provider_id:?} Service authentication generation is not synchronized to this Machine"
        ));
    }
    Ok(Some(installed_generation))
}

fn resolve_provider_generation(
    catalog: &crate::provider_catalog::ProviderCatalog,
    inventory: &[crate::machine_protocol::PluginInventory],
    provider_id: &str,
    service_auth: Option<&crate::provider_service::ProviderAuthenticationStatus>,
) -> Result<ResolvedProviderGeneration, String> {
    let installed = inventory
        .iter()
        .find(|provider| {
            provider.plugin_id == provider_id
                && provider.state == crate::machine_protocol::PluginInstallationState::Active
        })
        .ok_or_else(|| format!("Provider {provider_id:?} is not installed on this Machine"))?;
    let package = catalog
        .package(
            provider_id,
            &installed.plugin_version,
            &installed.generation_digest,
        )
        .ok_or_else(|| {
            format!(
                "installed Provider generation {} is not trusted by this Catalog",
                installed.generation_digest
            )
        })?;
    if package.contract_fingerprint != installed.contract_fingerprint {
        return Err(
            "installed Provider contract fingerprint does not match the Catalog".to_owned(),
        );
    }
    let auth_generation = resolve_scheduling_auth_generation(
        installed,
        package.manifest.authentication.required,
        service_auth,
    )?;
    Ok(ResolvedProviderGeneration {
        version: installed.plugin_version.clone(),
        digest: installed.generation_digest.clone(),
        auth_generation,
        behavior: package.manifest.runtime.behavior,
    })
}

/// Response body for `POST /api/sessions`.
#[derive(Debug, Serialize)]
struct NewSessionResponse {
    session_id: String,
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

async fn api_reconcile_project_sessions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ReconcileProjectSessionsRequest>,
) -> Response {
    let project = req.project.trim();
    if project.is_empty() {
        return (StatusCode::BAD_REQUEST, "project cannot be empty").into_response();
    }
    match state
        .supervisor
        .reconcile_project_sessions(project, req.dry_run)
    {
        Ok(session_ids) => {
            let native_thread_ids = state
                .hub
                .session_list()
                .into_iter()
                .filter(|meta| session_ids.contains(&meta.id))
                .filter_map(|meta| meta.agent_session_id.map(|thread| (meta.id, thread)))
                .collect();
            Json(ReconcileProjectSessionsResponse {
                session_ids,
                native_thread_ids,
            })
            .into_response()
        }
        Err(message) => (StatusCode::CONFLICT, message).into_response(),
    }
}

fn product_session_owner(
    product_auth_enabled: bool,
    principal: &ProductPrincipal,
) -> Option<crate::supervisor::SessionOwner<'_>> {
    product_auth_enabled.then_some(crate::supervisor::SessionOwner {
        user_id: &principal.user_id,
        username: Some(&principal.username),
    })
}

async fn api_new_session(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Json(req): Json<NewSessionRequest>,
) -> Response {
    let principal = authenticated.principal;
    if !principal.role.at_least(crate::admin::AdminRole::Operator) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut cwd = req.cwd;
    let session_id = state.supervisor.reserve_session_id();
    if web_session_is_missing_machine(&req.machine_id, &req.origin) {
        return (
            StatusCode::CONFLICT,
            "Web session creation requires a connected Machine so its workspace can be isolated",
        )
            .into_response();
    }
    if state
        .plugin_lifecycle_fences
        .read()
        .contains_key(&(req.machine_id.clone(), req.provider.clone()))
    {
        return (
            StatusCode::CONFLICT,
            "Provider is changing on the selected Machine",
        )
            .into_response();
    }
    if req.machine_id != "local" {
        let Some(store) = state.store.as_ref() else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "machine registry unavailable",
            )
                .into_response();
        };
        let machine = store.list_machines().await.ok().and_then(|machines| {
            machines
                .into_iter()
                .find(|machine| machine.id == req.machine_id && !machine.revoked)
        });
        let Some(machine) = machine else {
            return (StatusCode::NOT_FOUND, "unknown or revoked machine").into_response();
        };
        let workspaces: Vec<crate::machine_protocol::MachineWorkspace> = machine
            .inventory
            .get("workspaces")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        let selected_workspace = match resolve_machine_workspace(&workspaces, cwd.as_deref()) {
            Ok(workspace) => workspace.clone(),
            Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
        };
        cwd = Some(selected_workspace.canonical_path.clone());
        let capacity: crate::machine_protocol::MachineCapacity = machine
            .inventory
            .get("capacity")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        let active_sessions = state
            .hub
            .session_list()
            .into_iter()
            .filter(|session| {
                session.machine_id == req.machine_id
                    && session.status != crate::agent_model::Status::Exited
            })
            .count();
        if capacity.draining || active_sessions >= capacity.max_sessions as usize {
            return (
                StatusCode::CONFLICT,
                format!("machine {:?} is draining or at capacity", req.machine_id),
            )
                .into_response();
        }
        let providers = machine
            .inventory
            .get("plugins")
            .or_else(|| machine.inventory.get("providers"))
            .cloned()
            .and_then(|value| {
                serde_json::from_value::<Vec<crate::machine_protocol::PluginInventory>>(value).ok()
            })
            .unwrap_or_default();
        let provider_auth = state.provider_auth.status(&req.provider);
        let provider_generation = match resolve_provider_generation(
            &state.provider_catalog,
            &providers,
            &req.provider,
            provider_auth.as_ref(),
        ) {
            Ok(generation) => generation,
            Err(error) => return (StatusCode::CONFLICT, error).into_response(),
        };

        let Some(source_path) = cwd.as_deref() else {
            return (
                StatusCode::BAD_REQUEST,
                "remote session requires a trusted workspace",
            )
                .into_response();
        };
        // Hold both guards through the synchronous durable registration. The
        // lifecycle read guard makes an uninstall snapshot include this new
        // session; the Service auth read guard prevents logout/refresh from
        // changing the generation after readiness was checked.
        let plugin_creation_fence = state.plugin_lifecycle_fences.read();
        if plugin_creation_fence.contains_key(&(req.machine_id.clone(), req.provider.clone())) {
            return (
                StatusCode::CONFLICT,
                "Provider is changing on the selected Machine",
            )
                .into_response();
        }
        let registration = state.provider_auth.with_scheduling_generation(
            &req.provider,
            provider_generation.auth_generation.is_some(),
            provider_generation.auth_generation,
            || {
                state.supervisor.register_session_on_with_id(
                    &session_id,
                    &req.provider,
                    Some(source_path.to_owned()),
                    req.origin,
                    req.system,
                    crate::supervisor::SessionPlacement {
                        machine_id: &req.machine_id,
                        workspace: Some(&selected_workspace),
                    },
                    crate::supervisor::ProviderGeneration {
                        version: &provider_generation.version,
                        digest: &provider_generation.digest,
                        auth_generation: provider_generation.auth_generation,
                        behavior: Some(&provider_generation.behavior),
                    },
                    product_session_owner(state.product_auth_enabled, &principal),
                )
            },
        );
        drop(plugin_creation_fence);
        match registration {
            Ok(Ok(_)) => {}
            Ok(Err(message)) => return (StatusCode::BAD_REQUEST, message).into_response(),
            Err(error) => return (StatusCode::CONFLICT, error.to_string()).into_response(),
        }

        let prepare_state = Arc::clone(&state);
        let prepare_session_id = session_id.clone();
        let prepare_machine_id = req.machine_id.clone();
        let prepare_source_path = source_path.to_owned();
        let initial_prompt = req.initial_prompt;
        tokio::spawn(async move {
            let result = prepare_state
                .machine_control
                .adapter_request(
                    &prepare_machine_id,
                    "workspace",
                    serde_json::json!({
                        "root": &prepare_source_path,
                        "session_id": &prepare_session_id,
                    }),
                )
                .await
                .map_err(|error| {
                    format!("Machine could not prepare an isolated workspace: {error}")
                })
                .and_then(|value| {
                    serde_json::from_value::<PreparedMachineWorkspace>(value).map_err(|error| {
                        format!("Machine returned invalid workspace metadata: {error}")
                    })
                });

            let prepared = match result {
                Ok(prepared) => prepared,
                Err(error) => {
                    prepare_state.hub.set_status(
                        &prepare_session_id,
                        crate::agent_model::Status::Crashed,
                        Some(error),
                    );
                    return;
                }
            };
            tracing::info!(
                session_id = %prepare_session_id,
                machine_id = %prepare_machine_id,
                source_path = %prepared.source_path,
                prepared_path = %prepared.path,
                revision = ?prepared.revision,
                isolated = prepared.isolated,
                created = prepared.created,
                "prepared Machine workspace for session"
            );
            loop {
                match plugin_fence_state_for_session(
                    &prepare_state.hub,
                    &prepare_state.plugin_lifecycle_fences,
                    &prepare_session_id,
                ) {
                    Some(PluginFenceState::Uninstalling) => {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                    Some(PluginFenceState::Uninstalled) => return,
                    Some(PluginFenceState::Installing) | None => break,
                }
            }
            let prepared_session = prepare_state
                .hub
                .update_session_cwd(&prepare_session_id, prepared.path)
                .and_then(|()| {
                    prepare_state
                        .supervisor
                        .start_registered_session(&prepare_session_id)
                });
            if let Err(error) = prepared_session {
                prepare_state.hub.set_status(
                    &prepare_session_id,
                    crate::agent_model::Status::Crashed,
                    Some(format!("Session preparation failed: {error}")),
                );
                return;
            }
            if let Some(prompt) = initial_prompt.filter(|prompt| !prompt.trim().is_empty())
                && let Err(error) = prepare_state.supervisor.send(
                    &prepare_session_id,
                    AgentCommand::Prompt(vec![ContentBlock::from(prompt)], None, None),
                )
            {
                prepare_state.hub.set_status(
                    &prepare_session_id,
                    crate::agent_model::Status::Crashed,
                    Some(format!("Initial prompt failed: {error}")),
                );
            }
        });
        return (
            StatusCode::CREATED,
            Json(NewSessionResponse {
                session_id,
                provider_version: provider_generation.version,
                provider_generation_digest: provider_generation.digest,
                provider_auth_generation: provider_generation.auth_generation,
            }),
        )
            .into_response();
    }
    match state.supervisor.new_session_on_with_id(
        &session_id,
        &req.provider,
        cwd,
        req.origin,
        req.system,
        &req.machine_id,
        crate::supervisor::ProviderGeneration {
            version: "",
            digest: "",
            auth_generation: None,
            behavior: None,
        },
        product_session_owner(state.product_auth_enabled, &principal),
    ) {
        Ok(session_id) => {
            if let Some(prompt) = req
                .initial_prompt
                .filter(|prompt| !prompt.trim().is_empty())
                && let Err(message) = state.supervisor.send(
                    &session_id,
                    AgentCommand::Prompt(vec![ContentBlock::from(prompt)], None, None),
                )
            {
                state.hub.set_status(
                    &session_id,
                    crate::agent_model::Status::Crashed,
                    Some(format!("Initial prompt failed: {message}")),
                );
            }
            (
                StatusCode::CREATED,
                Json(NewSessionResponse {
                    session_id,
                    provider_version: String::new(),
                    provider_generation_digest: String::new(),
                    provider_auth_generation: None,
                }),
            )
                .into_response()
        }
        Err(message) => (StatusCode::BAD_REQUEST, message).into_response(),
    }
}

fn resolve_machine_workspace<'a>(
    workspaces: &'a [crate::machine_protocol::MachineWorkspace],
    requested_id: Option<&str>,
) -> Result<&'a crate::machine_protocol::MachineWorkspace, String> {
    let requested_id = requested_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "remote session requires a trusted workspace id".to_owned())?;
    workspaces
        .iter()
        .find(|workspace| workspace.id == requested_id)
        .ok_or_else(|| format!("unknown trusted workspace {requested_id:?}"))
}

#[cfg(test)]
mod machine_provider_tests {
    use super::{
        ProviderAuthExecutor, apply_workspace_inventory, failed_provider_auth_projection_ids,
        provider_auth_rebind_generations, rebindable_provider_auth_failure,
        resolve_machine_workspace, resolve_scheduling_auth_generation,
        web_session_is_missing_machine,
    };
    use crate::core::{Hub, SessionOrigin, Status};
    use std::collections::BTreeMap;

    fn installed_auth(generation: u64) -> crate::machine_protocol::PluginInventory {
        crate::machine_protocol::PluginInventory {
            plugin_id: "gemini".to_owned(),
            plugin_version: "1.0.0".to_owned(),
            plugin_kind: cowboy_plugin_sdk::PluginKind::AgentProvider,
            generation_digest: format!("sha256:{}", "1".repeat(64)),
            contract_fingerprint: format!("sha256:{}", "2".repeat(64)),
            state: crate::machine_protocol::PluginInstallationState::Active,
            rollback_generation_digest: None,
            active_session_leases: 0,
            auth_generation: Some(generation),
            replica_state: crate::machine_protocol::ProviderReplicaState::Current,
            materialization_state: crate::machine_protocol::ProviderMaterializationState::Current,
            detail: None,
        }
    }

    #[test]
    fn authentication_candidate_is_bound_to_the_exact_executor_release() {
        let executor = ProviderAuthExecutor {
            machine_id: "hawk".to_owned(),
            provider_id: "gemini".to_owned(),
            provider_version: "1.0.0".to_owned(),
            generation_digest: "sha256:release".to_owned(),
            auth_contract_fingerprint: "sha256:auth".to_owned(),
            auth_method: "api-key".to_owned(),
            expected_generation: 0,
            promotion_started: false,
            expires_at_ms: i64::MAX,
        };
        assert!(executor.accepts_candidate(
            "hawk",
            "gemini",
            "api-key",
            "1.0.0",
            "sha256:release",
            "sha256:auth",
        ));
        assert!(!executor.accepts_candidate(
            "hawk",
            "gemini",
            "api-key",
            "1.0.0",
            "sha256:other-release",
            "sha256:auth",
        ));
    }

    fn service_auth(
        generation: u64,
        state: crate::provider_service::ServiceAuthenticationState,
    ) -> crate::provider_service::ProviderAuthenticationStatus {
        crate::provider_service::ProviderAuthenticationStatus {
            provider_id: "gemini".to_owned(),
            auth_generation: generation,
            authentication_state: state,
            distribution_state: crate::provider_service::ServiceDistributionState::Current,
            auth_contract_fingerprint: format!("sha256:{}", "3".repeat(64)),
            authentication_scope: "gemini-auth-v1".to_owned(),
            portable_schema: "gemini-auth-v1".to_owned(),
            projection_schema: "gemini-home-v1".to_owned(),
            account_label: None,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn web_creation_cannot_use_the_legacy_shared_local_workspace() {
        assert!(web_session_is_missing_machine("local", &SessionOrigin::Web));
        assert!(!web_session_is_missing_machine(
            "local",
            &SessionOrigin::Api
        ));
        assert!(!web_session_is_missing_machine("hawk", &SessionOrigin::Web));
    }

    #[test]
    fn remote_workspace_id_is_resolved_before_session_persistence() {
        let workspaces = [crate::machine_protocol::MachineWorkspace {
            id: "cowboy".to_owned(),
            display_name: "Cowboy".to_owned(),
            canonical_path: "/srv/cowboy".to_owned(),
        }];
        assert_eq!(
            resolve_machine_workspace(&workspaces, Some("cowboy"))
                .map(|workspace| { (workspace.id.as_str(), workspace.canonical_path.as_str()) }),
            Ok(("cowboy", "/srv/cowboy"))
        );
        assert!(resolve_machine_workspace(&workspaces, Some("/srv/cowboy")).is_err());
        assert!(resolve_machine_workspace(&workspaces, Some("unknown")).is_err());
    }

    #[test]
    fn session_scheduling_requires_the_exact_ready_service_auth_generation() {
        let installed = installed_auth(7);
        let ready = service_auth(
            7,
            crate::provider_service::ServiceAuthenticationState::Ready,
        );
        assert_eq!(
            resolve_scheduling_auth_generation(&installed, true, Some(&ready)),
            Ok(Some(7))
        );

        let stale = service_auth(
            8,
            crate::provider_service::ServiceAuthenticationState::Ready,
        );
        assert!(
            resolve_scheduling_auth_generation(&installed, true, Some(&stale))
                .unwrap_err()
                .contains("generation is not synchronized")
        );

        let signed_out = service_auth(
            7,
            crate::provider_service::ServiceAuthenticationState::SignedOut,
        );
        assert!(
            resolve_scheduling_auth_generation(&installed, true, Some(&signed_out))
                .unwrap_err()
                .contains("not ready")
        );
        assert_eq!(
            resolve_scheduling_auth_generation(&installed, false, None),
            Ok(None)
        );
    }

    #[test]
    fn only_failed_current_provider_projections_request_service_repair() {
        let mut failed = installed_auth(7);
        failed.materialization_state =
            crate::machine_protocol::ProviderMaterializationState::Failed;
        let mut refreshing = installed_auth(7);
        refreshing.plugin_id = "grok".to_owned();
        refreshing.materialization_state =
            crate::machine_protocol::ProviderMaterializationState::Applying;
        let mut absent = failed.clone();
        absent.plugin_id = "codex".to_owned();
        absent.replica_state = crate::machine_protocol::ProviderReplicaState::Absent;

        assert_eq!(
            failed_provider_auth_projection_ids(&[failed, refreshing, absent]),
            vec!["gemini".to_owned()]
        );
    }

    #[test]
    fn auth_refresh_rebind_requires_newer_ready_credentials_on_the_selected_machine() {
        let hub = Hub::new();
        hub.create_session(crate::core::SessionRegistration {
            id: "s".to_owned(),
            provider: "gemini".to_owned(),
            provider_version: "1.0.0".to_owned(),
            provider_generation_digest: "sha256:provider".to_owned(),
            provider_auth_generation: Some(7),
            provider_behavior: None,
            machine_id: "hawk".to_owned(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            cwd: "/tmp".to_owned(),
            title: "test".to_owned(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
        let session = hub.session_info("s").unwrap().meta;
        let ready = service_auth(
            8,
            crate::provider_service::ServiceAuthenticationState::Ready,
        );
        let synchronized = BTreeMap::from([("hawk".to_owned(), 8)]);

        hub.set_status("s", Status::Crashed, Some("unrelated failure".to_owned()));
        assert!(rebindable_provider_auth_failure(&hub, "s").is_none());
        hub.set_status(
            "s",
            Status::Crashed,
            Some(crate::provider::provider_auth_required_detail(
                "gemini", true,
            )),
        );
        assert!(rebindable_provider_auth_failure(&hub, "s").is_some());

        assert_eq!(
            provider_auth_rebind_generations(
                &session,
                &ready,
                &synchronized,
                &ready.auth_contract_fingerprint,
            ),
            Some((7, 8))
        );
        assert_eq!(
            provider_auth_rebind_generations(
                &session,
                &ready,
                &BTreeMap::new(),
                &ready.auth_contract_fingerprint,
            ),
            None
        );
        let stale_projection = BTreeMap::from([("hawk".to_owned(), 7)]);
        assert_eq!(
            provider_auth_rebind_generations(
                &session,
                &ready,
                &stale_projection,
                &ready.auth_contract_fingerprint,
            ),
            None
        );
        assert_eq!(
            provider_auth_rebind_generations(&session, &ready, &synchronized, "sha256:different",),
            None
        );
        let expired = service_auth(
            8,
            crate::provider_service::ServiceAuthenticationState::Expired,
        );
        assert_eq!(
            provider_auth_rebind_generations(
                &session,
                &expired,
                &synchronized,
                &expired.auth_contract_fingerprint,
            ),
            None
        );
    }

    #[test]
    fn component_only_inventory_preserves_workspaces_until_an_explicit_reload() {
        let workspace = |id: &str| crate::machine_protocol::MachineWorkspace {
            id: id.to_owned(),
            display_name: id.to_owned(),
            canonical_path: format!("/srv/{id}"),
        };
        let mut workspaces = vec![workspace("old")];
        let mut revision = Some("revision-old".to_owned());
        apply_workspace_inventory(&mut workspaces, &mut revision, None, None);
        assert_eq!(workspaces, vec![workspace("old")]);
        assert_eq!(revision.as_deref(), Some("revision-old"));

        apply_workspace_inventory(
            &mut workspaces,
            &mut revision,
            Some(vec![workspace("new")]),
            Some("revision-new".to_owned()),
        );
        assert_eq!(workspaces, vec![workspace("new")]);
        assert_eq!(revision.as_deref(), Some("revision-new"));
    }
}

/// Query string for `GET /api/sessions/{id}/files` — the composer's `@` picker.
#[derive(Debug, Deserialize)]
struct FileSearchQuery {
    /// Fuzzy query; empty returns the "most useful" files (shallow + recent).
    #[serde(default)]
    q: String,
    #[serde(default = "default_file_limit")]
    limit: usize,
}

fn default_file_limit() -> usize {
    20
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

fn default_file_tree_limit() -> usize {
    200
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

fn file_tree_revision(path: &str, entries: &[FileTreeEntry], truncated: bool) -> String {
    let mut digest = Sha256::new();
    digest.update(path.as_bytes());
    digest.update([u8::from(truncated)]);
    for entry in entries {
        digest.update(entry.name.as_bytes());
        digest.update([0]);
        digest.update(entry.path.as_bytes());
        digest.update([0]);
        digest.update(entry.kind.as_bytes());
        digest.update([0]);
        digest.update([u8::from(entry.ignored)]);
    }
    format!("{:x}", digest.finalize())
}

fn file_tree_http_response(headers: &HeaderMap, revision: &str, bytes: Vec<u8>) -> Response {
    const TREE_CACHE_CONTROL: &str = "private, max-age=15, stale-while-revalidate=120";
    let etag = format!("\"{revision}\"");
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains(etag.as_str()))
    {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag.as_str()),
                (header::CACHE_CONTROL, TREE_CACHE_CONTROL),
            ],
        )
            .into_response();
    }
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static(TREE_CACHE_CONTROL),
    );
    if let Ok(value) = etag.parse() {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeChangeResponse {
    path: String,
    old_path: Option<String>,
    status: &'static str,
    staged: bool,
    unstaged: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeChangesResponse {
    api_version: u8,
    head: Option<String>,
    revision: String,
    changes: Vec<CodeChangeResponse>,
    truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeRepositoryResponse {
    api_version: u8,
    commits: Vec<crate::code_review::GitCommitSummary>,
    history_truncated: bool,
    worktrees: Vec<crate::code_review::GitWorktreeSummary>,
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

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ZedAdapterResponse {
    Worktree {
        api_version: u8,
        state: String,
    },
    Buffer {
        api_version: u8,
        path: String,
        leases: usize,
    },
    BufferLanguage {
        api_version: u8,
        path: String,
        version: Vec<CodeBufferVersion>,
        diagnostics: Vec<CodeDiagnostic>,
        inlay_hints: Vec<CodeInlayHint>,
        semantic_tokens: Vec<u32>,
    },
    BufferHover {
        api_version: u8,
        path: String,
        contents: Vec<CodeHoverBlock>,
    },
    BufferNavigation {
        api_version: u8,
        path: String,
        locations: Vec<CodeLocation>,
    },
    BufferSymbols {
        api_version: u8,
        path: String,
        symbols: Vec<CodeDocumentSymbol>,
    },
    Error {
        message: String,
    },
    #[serde(other)]
    Other,
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

async fn zed_adapter_request(
    socket: &FsPath,
    request: serde_json::Value,
) -> anyhow::Result<ZedAdapterResponse> {
    let stream = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        UnixStream::connect(socket),
    )
    .await
    .context("Zed adapter connect timed out")??;
    let (read, mut write) = stream.into_split();
    write.write_all(request.to_string().as_bytes()).await?;
    write.write_all(b"\n").await?;
    write.shutdown().await?;
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(35),
        BufReader::new(read).read_line(&mut line),
    )
    .await
    .context("Zed adapter response timed out")??;
    validate_zed_adapter_response(serde_json::from_str::<ZedAdapterResponse>(&line)?)
}

fn validate_zed_adapter_response(
    response: ZedAdapterResponse,
) -> anyhow::Result<ZedAdapterResponse> {
    match response {
        response @ (ZedAdapterResponse::Worktree { api_version: 1, .. }
        | ZedAdapterResponse::Buffer { api_version: 1, .. }
        | ZedAdapterResponse::BufferLanguage { api_version: 1, .. }
        | ZedAdapterResponse::BufferHover { api_version: 1, .. }
        | ZedAdapterResponse::BufferNavigation { api_version: 1, .. }
        | ZedAdapterResponse::BufferSymbols { api_version: 1, .. }) => Ok(response),
        ZedAdapterResponse::Worktree { api_version, .. }
        | ZedAdapterResponse::Buffer { api_version, .. }
        | ZedAdapterResponse::BufferLanguage { api_version, .. }
        | ZedAdapterResponse::BufferHover { api_version, .. }
        | ZedAdapterResponse::BufferNavigation { api_version, .. }
        | ZedAdapterResponse::BufferSymbols { api_version, .. } => {
            anyhow::bail!("unsupported Zed adapter API version {api_version}")
        }
        ZedAdapterResponse::Error { message } => anyhow::bail!("{message}"),
        ZedAdapterResponse::Other => anyhow::bail!("unexpected Zed adapter response"),
    }
}

async fn zed_adapter_request_for_session(
    state: &AppState,
    session_id: &str,
    request: serde_json::Value,
) -> anyhow::Result<ZedAdapterResponse> {
    let machine_id = state
        .hub
        .session_list()
        .into_iter()
        .find(|meta| meta.id == session_id)
        .map(|meta| meta.machine_id)
        .context("unknown session")?;
    if machine_id == "local" {
        let socket = state
            .zed_adapter_socket
            .as_deref()
            .context("local Zed adapter is not configured")?;
        return zed_adapter_request(socket, request).await;
    }
    let value = state
        .machine_control
        .adapter_request(&machine_id, "zed", request)
        .await
        .map_err(anyhow::Error::msg)?;
    validate_zed_adapter_response(serde_json::from_value(value)?)
}

async fn remote_code_request(
    state: &AppState,
    machine_id: &str,
    cwd: &str,
    operation: serde_json::Value,
) -> anyhow::Result<Option<crate::code_adapter::CodeAdapterResponse>> {
    if machine_id == "local" {
        return Ok(None);
    }
    let colocated = match state.machine_control.is_colocated(machine_id) {
        Some(value) => value,
        None => match state.store.as_ref() {
            Some(store) => store.machine_is_local(machine_id).await.unwrap_or(false),
            None => false,
        },
    };
    if colocated {
        return Ok(None);
    }
    let mut request = operation;
    request
        .as_object_mut()
        .context("code adapter operation must be an object")?
        .insert("root".to_owned(), serde_json::Value::String(cwd.to_owned()));
    let value = state
        .machine_control
        .adapter_request(machine_id, "code", request)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(Some(serde_json::from_value(value)?))
}

async fn ensure_zed_worktree_for_session(
    state: &AppState,
    session_id: &str,
    cwd: &str,
) -> anyhow::Result<bool> {
    match zed_adapter_request_for_session(
        state,
        session_id,
        serde_json::json!({
            "type": "ensureWorktree",
            "path": cwd,
            "trusted": true,
        }),
    )
    .await?
    {
        ZedAdapterResponse::Worktree { state, .. } => Ok(state == "ready"),
        _ => anyhow::bail!("unexpected Zed adapter response"),
    }
}

#[cfg(test)]
async fn ensure_zed_worktree(socket: &FsPath, cwd: &str) -> anyhow::Result<bool> {
    match zed_adapter_request(
        socket,
        serde_json::json!({
            "type": "ensureWorktree",
            "path": cwd,
            "trusted": true,
        }),
    )
    .await?
    {
        ZedAdapterResponse::Worktree { state, .. } => Ok(state == "ready"),
        _ => anyhow::bail!("unexpected Zed adapter response"),
    }
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
}

#[derive(Debug, Default, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum CodeDiffScope {
    #[default]
    Combined,
    Staged,
    Unstaged,
}

fn default_code_context() -> usize {
    6
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

/// Rank files under a session's working directory for the `@` reference picker.
///
/// The cwd comes from the session itself (never from the client) so a browser
/// can't walk arbitrary paths. The walk + fuzzy match is blocking, so it runs
/// on a blocking thread; a missing session is `404`, an empty tree is `200`
/// with `[]`.
async fn api_search_files(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<FileSearchQuery>,
) -> Response {
    let Some(context) = resolve_code_context(&state, &session_id).await else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let cwd = context.cwd;
    let limit = query.limit.clamp(1, 100);
    let files = match remote_code_request(
        &state,
        &context.machine_id,
        &cwd,
        serde_json::json!({ "type": "search", "query": query.q, "limit": limit }),
    )
    .await
    {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::Search(files))) => files,
        Ok(Some(_)) | Err(_) => {
            return (StatusCode::BAD_GATEWAY, "remote code search unavailable").into_response();
        }
        Ok(None) => tokio::task::spawn_blocking(move || {
            crate::files::search(std::path::Path::new(&cwd), &query.q, limit)
        })
        .await
        .unwrap_or_default(),
    };
    Json(FileSearchResponse { files }).into_response()
}

async fn api_code_search(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<FileSearchQuery>,
) -> Response {
    let Some(context) = resolve_code_context(&state, &session_id).await else {
        return (StatusCode::NOT_FOUND, "unknown code context").into_response();
    };
    let cwd = context.cwd;
    let limit = query.limit.clamp(1, 100);
    let files = match remote_code_request(
        &state,
        &context.machine_id,
        &cwd,
        serde_json::json!({ "type": "search", "query": query.q, "limit": limit }),
    )
    .await
    {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::Search(files))) => files,
        Ok(Some(_)) | Err(_) => {
            return (StatusCode::BAD_GATEWAY, "remote code search unavailable").into_response();
        }
        Ok(None) => tokio::task::spawn_blocking(move || {
            crate::code_review::LocalCodeProvider::new(cwd).search(&query.q, limit)
        })
        .await
        .unwrap_or_default(),
    };
    Json(CodeSearchResponse {
        api_version: 1,
        files,
    })
    .into_response()
}

/// Return one filesystem directory page for the mobile review tree.
///
/// The root is resolved from the session rather than a client-provided root.
/// Relative paths are validated and cannot escape it. Gitignored children are
/// intentionally visible here because they may be independent repositories;
/// Git Changes remains scoped to the owning repository.
async fn api_file_tree(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<FileTreeQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(context) = resolve_code_context(&state, &session_id).await else {
        return (StatusCode::NOT_FOUND, "unknown code context").into_response();
    };
    let cwd = context.cwd;
    let limit = query.limit.clamp(20, 500);
    let path = query.path;
    let requested_path = path.clone();
    let remote_page = match remote_code_request(
        &state,
        &context.machine_id,
        &cwd,
        serde_json::json!({ "type": "directory", "path": path.clone(), "limit": limit }),
    )
    .await
    {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::Directory(page))) => Some(page),
        Ok(Some(_)) | Err(_) => {
            return (StatusCode::BAD_GATEWAY, "remote directory unavailable").into_response();
        }
        Ok(None) => None,
    };
    if let Some(page) = remote_page {
        let entries = page
            .entries
            .into_iter()
            .map(|entry| FileTreeEntry {
                name: entry.name,
                path: entry.path,
                kind: if entry.is_directory {
                    "directory"
                } else {
                    "file"
                },
                ignored: entry.ignored,
            })
            .collect::<Vec<_>>();
        let revision = file_tree_revision(&requested_path, &entries, page.truncated);
        let body = serde_json::to_vec(&FileTreeResponse {
            api_version: 1,
            path: requested_path,
            revision: revision.clone(),
            entries,
            truncated: page.truncated,
        })
        .expect("file tree response serializes");
        return file_tree_http_response(&headers, &revision, body);
    }
    let cache = state.code_cache.clone();
    let cache_root = cwd.clone();
    let cache_path = requested_path.clone();
    let cached = tokio::task::spawn_blocking(move || {
        cache.get_directory(FsPath::new(&cache_root), &cache_path, limit)
    })
    .await;
    if let Ok(Ok(Some(cached))) = cached {
        return file_tree_http_response(&headers, &cached.revision, cached.bytes);
    }
    let scan_root = cwd.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::code_review::LocalCodeProvider::new(scan_root).directory(&path, limit)
    })
    .await;
    let Ok(Ok(page)) = result else {
        return (StatusCode::BAD_REQUEST, "invalid directory").into_response();
    };
    let entries = page
        .entries
        .into_iter()
        .map(|entry| FileTreeEntry {
            name: entry.name,
            path: entry.path,
            kind: if entry.is_directory {
                "directory"
            } else {
                "file"
            },
            ignored: entry.ignored,
        })
        .collect::<Vec<_>>();
    let truncated = page.truncated;
    let revision = file_tree_revision(&requested_path, &entries, truncated);
    let body = serde_json::to_vec(&FileTreeResponse {
        api_version: 1,
        path: requested_path.clone(),
        revision: revision.clone(),
        entries,
        truncated,
    })
    .expect("file tree response serializes");
    let cache = state.code_cache.clone();
    let cache_revision = revision.clone();
    let cache_body = body.clone();
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            cache.put_directory(
                FsPath::new(&cwd),
                &requested_path,
                limit,
                &cache_revision,
                &cache_body,
            )
        })
        .await;
        if let Ok(Err(error)) = result {
            tracing::warn!(%error, "persisting lazy directory cache failed");
        } else if let Err(error) = result {
            tracing::warn!(%error, "lazy directory cache task failed");
        }
    });
    file_tree_http_response(&headers, &revision, body)
}

const WORKSPACE_CODE_CONTEXT_PREFIX: &str = "workspace::";

struct ResolvedCodeContext {
    machine_id: String,
    cwd: String,
    session_id: Option<String>,
}

fn parse_workspace_code_context(value: &str) -> Option<(&str, &str)> {
    let value = value.strip_prefix(WORKSPACE_CODE_CONTEXT_PREFIX)?;
    let (machine_id, workspace_id) = value.split_once("::")?;
    (!machine_id.is_empty() && !workspace_id.is_empty() && !workspace_id.contains("::"))
        .then_some((machine_id, workspace_id))
}

#[cfg(test)]
mod workspace_code_context_tests {
    use super::parse_workspace_code_context;

    #[test]
    fn only_explicit_machine_workspace_pairs_are_code_contexts() {
        assert_eq!(
            parse_workspace_code_context("workspace::hawk::cowboy"),
            Some(("hawk", "cowboy"))
        );
        assert_eq!(parse_workspace_code_context("sess-123"), None);
        assert_eq!(parse_workspace_code_context("workspace::::cowboy"), None);
        assert_eq!(
            parse_workspace_code_context("workspace::hawk::cowboy::extra"),
            None
        );
    }
}

async fn resolve_code_context(state: &AppState, id: &str) -> Option<ResolvedCodeContext> {
    if let Some((machine_id, workspace_id)) = parse_workspace_code_context(id) {
        let store = state.store.as_ref()?;
        let machines = store.list_machines().await.ok()?;
        let machine = machines
            .into_iter()
            .find(|machine| machine.id == machine_id && !machine.revoked)?;
        let workspaces: Vec<crate::machine_protocol::MachineWorkspace> = machine
            .inventory
            .get("workspaces")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())?;
        let workspace = workspaces
            .into_iter()
            .find(|workspace| workspace.id == workspace_id)?;
        return Some(ResolvedCodeContext {
            machine_id: machine.id,
            cwd: workspace.canonical_path,
            session_id: None,
        });
    }
    session_code_context(state, id)
}

fn session_code_context(state: &AppState, session_id: &str) -> Option<ResolvedCodeContext> {
    state
        .hub
        .session_list()
        .into_iter()
        .find(|meta| meta.id == session_id)
        .map(|meta| ResolvedCodeContext {
            machine_id: meta.machine_id,
            cwd: meta.cwd,
            session_id: Some(meta.id),
        })
}

async fn api_code_manifest(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(context) = resolve_code_context(&state, &session_id).await else {
        return (StatusCode::NOT_FOUND, "unknown code context").into_response();
    };
    let cwd = context.cwd;
    let language_ready = if context.session_id.is_some() {
        match ensure_zed_worktree_for_session(&state, &session_id, &cwd).await {
            Ok(ready) => ready,
            Err(error) => {
                tracing::warn!(session = %session_id, %error, "Zed adapter unavailable");
                false
            }
        }
    } else {
        false
    };
    let manifest = match remote_code_request(
        &state,
        &context.machine_id,
        &cwd,
        serde_json::json!({ "type": "manifest" }),
    )
    .await
    {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::Manifest(manifest))) => manifest,
        Ok(Some(_)) | Err(_) => {
            return (StatusCode::BAD_GATEWAY, "remote worktree unavailable").into_response();
        }
        Ok(None) => {
            let result = tokio::task::spawn_blocking(move || {
                crate::code_review::LocalCodeProvider::new(cwd).manifest()
            })
            .await;
            let Ok(Ok(manifest)) = result else {
                return (StatusCode::UNPROCESSABLE_ENTITY, "worktree unavailable").into_response();
            };
            manifest
        }
    };
    let language_state = if language_ready {
        "ready"
    } else {
        "unavailable"
    };
    // Capability fields are part of this cached representation. Bump the
    // contract tag whenever that shape grows so installed Mobile clients do
    // not retain an older 304-backed manifest after a deploy.
    let etag = format!(
        "\"code-manifest-v3-{}-{language_state}\"",
        manifest.revision
    );
    const MANIFEST_CACHE_CONTROL: &str = "private, max-age=0, must-revalidate";
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains(etag.as_str()))
    {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag.as_str()),
                (header::CACHE_CONTROL, MANIFEST_CACHE_CONTROL),
            ],
        )
            .into_response();
    }
    let mut response = Json(CodeManifestResponse {
        api_version: 1,
        provider: manifest.provider,
        revision: manifest.revision,
        head: manifest.head,
        project: manifest.project,
        branch: manifest.branch,
        worktree: manifest.worktree,
        change_count: manifest.change_count,
        language: CodeLanguageCapabilities {
            provider: if language_ready { "zed" } else { "none" },
            state: language_state,
            diagnostics: language_ready,
            inlay_hints: language_ready,
            semantic_tokens: language_ready,
            hover: language_ready,
            navigation: language_ready,
            outline: language_ready,
        },
    })
    .into_response();
    response
        .headers_mut()
        .insert(header::ETAG, etag.parse().expect("SHA256 ETag is valid"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static(MANIFEST_CACHE_CONTROL),
    );
    response
}

async fn api_code_changes(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    let Some(context) = resolve_code_context(&state, &session_id).await else {
        return (StatusCode::NOT_FOUND, "unknown code context").into_response();
    };
    let cwd = context.cwd;
    let result = match remote_code_request(
        &state,
        &context.machine_id,
        &cwd,
        serde_json::json!({ "type": "changes" }),
    )
    .await
    {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::Changes(changes))) => changes,
        Ok(Some(_)) | Err(_) => {
            return (StatusCode::BAD_GATEWAY, "remote git changes unavailable").into_response();
        }
        Ok(None) => {
            let result = tokio::task::spawn_blocking(move || {
                crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd)).changes()
            })
            .await;
            let Ok(Ok(changes)) = result else {
                return (StatusCode::UNPROCESSABLE_ENTITY, "git changes unavailable")
                    .into_response();
            };
            changes
        }
    };
    Json(CodeChangesResponse {
        api_version: 1,
        head: result.head,
        revision: result.revision,
        changes: result
            .changes
            .into_iter()
            .map(|change| CodeChangeResponse {
                path: change.path,
                old_path: change.old_path,
                staged: change.staged,
                unstaged: change.unstaged,
                status: match change.status {
                    crate::code_review::ChangeStatus::Modified => "modified",
                    crate::code_review::ChangeStatus::Added => "added",
                    crate::code_review::ChangeStatus::Deleted => "deleted",
                    crate::code_review::ChangeStatus::Renamed => "renamed",
                    crate::code_review::ChangeStatus::Untracked => "untracked",
                    crate::code_review::ChangeStatus::Conflicted => "conflicted",
                },
            })
            .collect(),
        truncated: result.truncated,
    })
    .into_response()
}

async fn api_code_repository(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<CodeRepositoryQuery>,
) -> Response {
    let Some(context) = resolve_code_context(&state, &session_id).await else {
        return (StatusCode::NOT_FOUND, "unknown code context").into_response();
    };
    let cwd = context.cwd;
    let after = query.after;
    let result = match remote_code_request(
        &state,
        &context.machine_id,
        &cwd,
        match after.as_deref() {
            Some(oid) => serde_json::json!({ "type": "repository", "after": oid }),
            None => serde_json::json!({ "type": "repository" }),
        },
    )
    .await
    {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::Repository(repository))) => repository,
        Ok(Some(_)) | Err(_) => {
            return (StatusCode::BAD_GATEWAY, "remote git history unavailable").into_response();
        }
        Ok(None) => {
            let result = tokio::task::spawn_blocking(move || {
                crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd))
                    .repository(after.as_deref())
            })
            .await;
            let Ok(Ok(repository)) = result else {
                return (StatusCode::UNPROCESSABLE_ENTITY, "git history unavailable")
                    .into_response();
            };
            repository
        }
    };
    Json(CodeRepositoryResponse {
        api_version: 1,
        commits: result.commits,
        history_truncated: result.history_truncated,
        worktrees: result.worktrees,
    })
    .into_response()
}

async fn api_code_commit(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<CodeCommitQuery>,
) -> Response {
    let Some(context) = resolve_code_context(&state, &session_id).await else {
        return (StatusCode::NOT_FOUND, "unknown code context").into_response();
    };
    let cwd = context.cwd;
    let oid = query.oid;
    let result = match remote_code_request(
        &state,
        &context.machine_id,
        &cwd,
        serde_json::json!({ "type": "commit", "oid": oid.clone() }),
    )
    .await
    {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::Commit(commit))) => commit,
        Ok(Some(_)) | Err(_) => {
            return (StatusCode::BAD_GATEWAY, "remote commit unavailable").into_response();
        }
        Ok(None) => {
            let result = tokio::task::spawn_blocking(move || {
                crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd)).commit(&oid)
            })
            .await;
            let Ok(Ok(commit)) = result else {
                return (StatusCode::UNPROCESSABLE_ENTITY, "commit unavailable").into_response();
            };
            commit
        }
    };
    Json(CodeCommitResponse {
        api_version: 1,
        commit: result,
    })
    .into_response()
}

async fn api_code_commit_diff(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<CodeCommitDiffQuery>,
) -> Response {
    let Some(context) = resolve_code_context(&state, &session_id).await else {
        return (StatusCode::NOT_FOUND, "unknown code context").into_response();
    };
    let cwd = context.cwd;
    let oid = query.oid;
    let path = query.path;
    let result = match remote_code_request(
        &state,
        &context.machine_id,
        &cwd,
        serde_json::json!({ "type": "commit_diff", "oid": oid.clone(), "path": path.clone() }),
    )
    .await
    {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::CommitDiff(diff))) => diff,
        Ok(Some(_)) | Err(_) => {
            return (StatusCode::BAD_GATEWAY, "remote commit diff unavailable").into_response();
        }
        Ok(None) => {
            let commit_oid = oid.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd))
                    .commit_diff(&commit_oid, &path)
            })
            .await;
            let Ok(Ok(diff)) = result else {
                return (StatusCode::UNPROCESSABLE_ENTITY, "commit diff unavailable")
                    .into_response();
            };
            diff
        }
    };
    Json(CodeDiffResponse {
        api_version: 1,
        path: result.path,
        revision: oid,
        text: result.text,
        added: result.added,
        removed: result.removed,
        truncated: result.truncated,
        next_cursor: None,
        limited: result.truncated,
    })
    .into_response()
}

async fn api_code_diff(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<CodeDiffQuery>,
) -> Response {
    if let Some(cursor) = query.cursor.as_deref() {
        return match state.diff_snapshots.next_page(&session_id, cursor).await {
            Ok(page) => Json(CodeDiffResponse {
                api_version: 1,
                path: page.path,
                revision: page.revision,
                text: page.text,
                added: page.added,
                removed: page.removed,
                truncated: page.next_cursor.is_some() || page.limited,
                next_cursor: page.next_cursor,
                limited: page.limited,
            })
            .into_response(),
            Err(error) if error == "diff snapshot expired" => {
                (StatusCode::GONE, error).into_response()
            }
            Err(error) => (StatusCode::BAD_REQUEST, error).into_response(),
        };
    }
    let Some(context) = resolve_code_context(&state, &session_id).await else {
        return (StatusCode::NOT_FOUND, "unknown code context").into_response();
    };
    let cwd = context.cwd;
    let Some(path) = query.path else {
        return (StatusCode::BAD_REQUEST, "path is required").into_response();
    };
    let scope = match query.scope {
        CodeDiffScope::Combined => crate::code_review::DiffScope::Combined,
        CodeDiffScope::Staged => crate::code_review::DiffScope::Staged,
        CodeDiffScope::Unstaged => crate::code_review::DiffScope::Unstaged,
    };
    let key = DiffSnapshotKey {
        session_id: session_id.clone(),
        cwd: cwd.clone(),
        path: path.clone(),
        context: query.context,
        show_whitespace: query.show_whitespace,
        scope,
    };
    let remote_document = match remote_code_request(
        &state,
        &context.machine_id,
        &cwd,
        serde_json::json!({
            "type": "diff",
            "path": path.clone(),
            "context": query.context,
            "show_whitespace": query.show_whitespace,
            "scope": scope,
        }),
    )
    .await
    {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::Diff(document))) => Some(document),
        Ok(Some(_)) | Err(_) => {
            return (StatusCode::BAD_GATEWAY, "remote diff unavailable").into_response();
        }
        Ok(None) => None,
    };
    let page = state
        .diff_snapshots
        .first_page(key, || async move {
            if let Some(document) = remote_document {
                return Ok(document);
            }
            tokio::task::spawn_blocking(move || {
                crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd)).diff_snapshot(
                    &path,
                    query.context,
                    query.show_whitespace,
                    scope,
                )
            })
            .await
            .map_err(|error| error.to_string())?
        })
        .await;
    let Ok(page) = page else {
        return (StatusCode::BAD_REQUEST, "diff unavailable").into_response();
    };
    Json(CodeDiffResponse {
        api_version: 1,
        path: page.path,
        revision: page.revision,
        text: page.text,
        added: page.added,
        removed: page.removed,
        truncated: page.next_cursor.is_some() || page.limited,
        next_cursor: page.next_cursor,
        limited: page.limited,
    })
    .into_response()
}

async fn api_code_file(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<CodeFileQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(context) = resolve_code_context(&state, &session_id).await else {
        return (StatusCode::NOT_FOUND, "unknown code context").into_response();
    };
    let cwd = context.cwd;
    let result = match remote_code_request(
        &state,
        &context.machine_id,
        &cwd,
        serde_json::json!({ "type": "file", "path": query.path.clone(), "cursor": query.cursor.clone() }),
    )
    .await
    {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::File(file))) => Ok(file),
        Ok(Some(_)) | Err(_) => return (StatusCode::BAD_GATEWAY, "remote file unavailable").into_response(),
        Ok(None) => {
            let cache = state.code_cache.clone();
            let result = tokio::task::spawn_blocking(move || {
                match cache.get_or_load(FsPath::new(&cwd), &query.path) {
                    Ok(Some(cached)) => {
                        debug_assert_eq!(cached.size, cached.bytes.len() as u64);
                        crate::code_review::cached_file_page(
                            &query.path,
                            cached.bytes,
                            cached.revision,
                            query.cursor.as_deref(),
                        )
                    }
                    // The cache is deliberately bounded to the physical session
                    // worktree. Registered aggregate projects are a read-only Code
                    // projection outside that root, so let the provider resolve and
                    // validate those paths instead of treating a cache miss as an
                    // authorization failure.
                    Ok(None) | Err(_) => crate::code_review::LocalCodeProvider::new(
                        FsPath::new(&cwd),
                    )
                    .file_page(&query.path, query.cursor.as_deref()),
                }
            }).await;
            let Ok(result) = result else {
                return (StatusCode::BAD_REQUEST, "file unavailable").into_response();
            };
            result
        }
    };
    let result = match result {
        Ok(result) => result,
        Err(error) if error == "file snapshot changed" => {
            return (StatusCode::CONFLICT, error).into_response();
        }
        Err(error) => return code_file_error_response(&error),
    };
    let etag = format!("\"{}\"", result.revision);
    const FILE_CACHE_CONTROL: &str = "private, max-age=0, must-revalidate";
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains(etag.as_str()))
    {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag.as_str()),
                (header::CACHE_CONTROL, FILE_CACHE_CONTROL),
            ],
        )
            .into_response();
    }
    let mut response = Json(CodeFileResponse {
        api_version: 1,
        path: result.path,
        revision: result.revision,
        text: result.text,
        size: result.size,
        truncated: result.truncated,
        next_cursor: result.next_cursor,
        limited: result.limited,
    })
    .into_response();
    if let Ok(value) = etag.parse() {
        response.headers_mut().insert(header::ETAG, value);
    }
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static(FILE_CACHE_CONTROL),
    );
    response
}

async fn api_code_file_raw(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<CodeFileQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(context) = resolve_code_context(&state, &session_id).await else {
        return (StatusCode::NOT_FOUND, "unknown code context").into_response();
    };
    let cwd = context.cwd;
    let result = match remote_code_request(
        &state,
        &context.machine_id,
        &cwd,
        serde_json::json!({ "type": "file_raw", "path": query.path.clone() }),
    )
    .await
    {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::FileRaw(file))) => Ok(file),
        Ok(Some(_)) => return (StatusCode::BAD_GATEWAY, "remote file unavailable").into_response(),
        Err(_) => return (StatusCode::BAD_GATEWAY, "remote file unavailable").into_response(),
        Ok(None) => {
            let cache = state.code_cache.clone();
            let path = query.path.clone();
            let result = tokio::task::spawn_blocking(move || {
                let media_type = crate::code_review::preview_media_type(&path)
                    .ok_or_else(|| "file is not a previewable media type".to_owned())?;
                match cache.get_or_load(FsPath::new(&cwd), &path) {
                    Ok(Some(cached)) => Ok(crate::code_review::RawFileDocument {
                        path: path.replace('\\', "/"),
                        revision: cached.revision,
                        media_type: media_type.to_owned(),
                        bytes: cached.bytes,
                        size: cached.size,
                    }),
                    Ok(None) | Err(_) => {
                        crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd))
                            .file_raw(&path)
                    }
                }
            })
            .await;
            let Ok(result) = result else {
                return (StatusCode::BAD_REQUEST, "file unavailable").into_response();
            };
            result
        }
    };
    let result = match result {
        Ok(result) => result,
        Err(error) if error == "file snapshot changed" => {
            return (StatusCode::CONFLICT, error).into_response();
        }
        Err(error) => return code_file_error_response(&error),
    };
    let etag = format!("\"{}\"", result.revision);
    const FILE_CACHE_CONTROL: &str = "private, max-age=0, must-revalidate";
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains(etag.as_str()))
    {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag.as_str()),
                (header::CACHE_CONTROL, FILE_CACHE_CONTROL),
            ],
        )
            .into_response();
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, result.media_type),
            (header::ETAG, etag),
            (header::CACHE_CONTROL, FILE_CACHE_CONTROL.to_owned()),
        ],
        result.bytes,
    )
        .into_response()
}

fn code_file_error_response(error: &str) -> Response {
    match error {
        "file not found" => (StatusCode::NOT_FOUND, "file not found").into_response(),
        "binary file" | "file is not UTF-8" | "file is not a previewable media type" => (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "file is not previewable text",
        )
            .into_response(),
        _ => (StatusCode::BAD_REQUEST, "file unavailable").into_response(),
    }
}

async fn api_code_buffer_open(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<CodeBufferLeaseRequest>,
) -> Response {
    api_code_buffer_lease(state, session_id, request, true).await
}

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

#[derive(Debug, Deserialize, Serialize)]
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

async fn api_code_language(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<CodeLanguageQuery>,
) -> Response {
    let Some(context) = session_code_context(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if query.path.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid buffer path").into_response();
    }
    let Some((worktree, path)) =
        zed_language_target(&context.machine_id, &context.cwd, &query.path)
    else {
        return (StatusCode::UNPROCESSABLE_ENTITY, "buffer lease unavailable").into_response();
    };
    match zed_adapter_request_for_session(
        &state,
        &session_id,
        serde_json::json!({
            "type": "bufferLanguage",
            "worktree": worktree,
            "path": path,
        }),
    )
    .await
    {
        Ok(ZedAdapterResponse::BufferLanguage {
            path,
            version,
            diagnostics,
            inlay_hints,
            semantic_tokens,
            ..
        }) => Json(CodeLanguageResponse {
            api_version: 1,
            path,
            version,
            diagnostics,
            inlay_hints,
            semantic_tokens,
        })
        .into_response(),
        Ok(_) => (
            StatusCode::BAD_GATEWAY,
            "unexpected language service response",
        )
            .into_response(),
        Err(error) => {
            tracing::warn!(session = %session_id, %error, "Zed language query failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "language intelligence unavailable",
            )
                .into_response()
        }
    }
}

async fn api_code_hover(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<CodeHoverQuery>,
) -> Response {
    let Some(context) = session_code_context(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if query.path.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid buffer path").into_response();
    }
    let Some((worktree, path)) =
        zed_language_target(&context.machine_id, &context.cwd, &query.path)
    else {
        return (StatusCode::UNPROCESSABLE_ENTITY, "buffer lease unavailable").into_response();
    };
    match zed_adapter_request_for_session(
        &state,
        &session_id,
        serde_json::json!({
            "type": "bufferHover",
            "worktree": worktree,
            "path": path,
            "row": query.row,
            "column": query.column,
        }),
    )
    .await
    {
        Ok(ZedAdapterResponse::BufferHover { path, contents, .. }) => Json(CodeHoverResponse {
            api_version: 1,
            path,
            contents,
        })
        .into_response(),
        Ok(_) => (
            StatusCode::BAD_GATEWAY,
            "unexpected language service response",
        )
            .into_response(),
        Err(error) => {
            tracing::debug!(session = %session_id, %error, "Zed hover query failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "symbol information unavailable",
            )
                .into_response()
        }
    }
}

async fn api_code_navigation(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<CodeNavigationQuery>,
) -> Response {
    let Some(context) = session_code_context(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if query.path.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid buffer path").into_response();
    }
    let Some((worktree, path)) =
        zed_language_target(&context.machine_id, &context.cwd, &query.path)
    else {
        return (StatusCode::UNPROCESSABLE_ENTITY, "buffer lease unavailable").into_response();
    };
    match zed_adapter_request_for_session(
        &state,
        &session_id,
        serde_json::json!({
            "type": "bufferNavigate",
            "worktree": worktree,
            "path": path,
            "row": query.row,
            "column": query.column,
            "kind": query.kind,
        }),
    )
    .await
    {
        Ok(ZedAdapterResponse::BufferNavigation {
            path, locations, ..
        }) => Json(CodeNavigationResponse {
            api_version: 1,
            path,
            locations,
        })
        .into_response(),
        Ok(_) => (
            StatusCode::BAD_GATEWAY,
            "unexpected language service response",
        )
            .into_response(),
        Err(error) => {
            tracing::debug!(session = %session_id, %error, "Zed navigation query failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "symbol navigation unavailable",
            )
                .into_response()
        }
    }
}

async fn api_code_outline(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<CodeLanguageQuery>,
) -> Response {
    let Some(context) = session_code_context(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if query.path.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid buffer path").into_response();
    }
    let Some((worktree, path)) =
        zed_language_target(&context.machine_id, &context.cwd, &query.path)
    else {
        return (StatusCode::UNPROCESSABLE_ENTITY, "buffer lease unavailable").into_response();
    };
    match zed_adapter_request_for_session(
        &state,
        &session_id,
        serde_json::json!({
            "type": "bufferSymbols",
            "worktree": worktree,
            "path": path,
        }),
    )
    .await
    {
        Ok(ZedAdapterResponse::BufferSymbols { path, symbols, .. }) => Json(CodeOutlineResponse {
            api_version: 1,
            path,
            symbols,
        })
        .into_response(),
        Ok(_) => (
            StatusCode::BAD_GATEWAY,
            "unexpected language service response",
        )
            .into_response(),
        Err(error) => {
            tracing::debug!(session = %session_id, %error, "Zed outline query failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "document outline unavailable",
            )
                .into_response()
        }
    }
}

async fn api_code_buffer_close(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<CodeBufferLeaseRequest>,
) -> Response {
    api_code_buffer_lease(state, session_id, request, false).await
}

async fn api_code_buffer_lease(
    state: Arc<AppState>,
    session_id: String,
    request: CodeBufferLeaseRequest,
    open: bool,
) -> Response {
    let Some(context) = session_code_context(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if request.path.is_empty()
        || request.lease_id.is_empty()
        || request.lease_id.len() > 128
        || request.lease_id.chars().any(char::is_whitespace)
    {
        return (StatusCode::BAD_REQUEST, "invalid buffer lease").into_response();
    }
    let Some((worktree, path)) =
        zed_language_target(&context.machine_id, &context.cwd, &request.path)
    else {
        // Missing files, path escapes, and unregistered aggregate projections
        // are deterministic lease misses, not adapter outages. Registered
        // `projects/<name>/...` files lease against that checkout so hover can
        // run rust-analyzer there.
        return (StatusCode::UNPROCESSABLE_ENTITY, "buffer lease unavailable").into_response();
    };
    if open
        && ensure_zed_worktree_for_session(&state, &session_id, &worktree)
            .await
            .is_err()
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "language service unavailable",
        )
            .into_response();
    }
    let response = zed_adapter_request_for_session(
        &state,
        &session_id,
        serde_json::json!({
            "type": if open { "openBuffer" } else { "closeBuffer" },
            "worktree": worktree,
            "path": path,
            "leaseId": request.lease_id,
        }),
    )
    .await;
    match response {
        Ok(ZedAdapterResponse::Buffer { path, leases, .. }) => Json(CodeBufferLeaseResponse {
            api_version: 1,
            path,
            leases,
        })
        .into_response(),
        Ok(_) => (
            StatusCode::BAD_GATEWAY,
            "unexpected language service response",
        )
            .into_response(),
        Err(error) => {
            tracing::warn!(
                session = %session_id,
                operation = if open { "open" } else { "close" },
                %error,
                "Zed buffer lease failed"
            );
            (
                if open {
                    StatusCode::UNPROCESSABLE_ENTITY
                } else {
                    StatusCode::CONFLICT
                },
                "buffer lease unavailable",
            )
                .into_response()
        }
    }
}

/// Resolve a browser buffer path on the host that owns the session.
///
/// Local sessions can be canonicalized here, including Columbus aggregate
/// projections. A remote Machine path does not exist on the Controller, so the
/// Controller only applies the portable relative-path boundary and leaves the
/// authoritative canonical/trusted-root check to that Machine's Zed adapter.
fn zed_language_target(machine_id: &str, cwd: &str, relative: &str) -> Option<(String, String)> {
    if machine_id != "local" {
        if relative.is_empty()
            || relative.starts_with('/')
            || relative.contains(['\\', '\0'])
            || relative
                .split('/')
                .any(|component| matches!(component, "" | "." | ".."))
        {
            return None;
        }
        return Some((cwd.to_owned(), relative.to_owned()));
    }
    crate::code_review::LocalCodeProvider::new(std::path::Path::new(cwd))
        .language_buffer_key(relative)
        .map(|(worktree, path)| {
            (
                worktree.to_string_lossy().into_owned(),
                path.to_string_lossy().replace('\\', "/"),
            )
        })
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
}

/// Hydrate only the session the reader actually opened. The WebSocket connect
/// path deliberately carries global metadata only; replaying every transcript,
/// config option and queue state made mobile reconnects multi-megabyte
/// affairs. Live events can overlap this response and are deduplicated by seq.
async fn api_session_bootstrap(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    let Some(messages) = focused_session_bootstrap(&state.hub, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(SessionBootstrapResponse { messages }),
    )
        .into_response()
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
/// past page never changes again, so it's served
/// `immutable` (one year) — the browser + service worker then satisfy any
/// re-fetch (scroll back, reload, post-recycle reload) with ZERO network. The
/// still-growing latest page is `no-store`, but the client never asks for it
/// (it has the tail over WS). Unknown session → 404; out-of-range page → `[]`.
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
    let cache = "public, max-age=31536000, immutable";
    (
        [(header::CACHE_CONTROL, cache)],
        Json(HistoryResponse {
            events,
            next_before_seq,
            reached_start,
        }),
    )
        .into_response()
}

async fn api_question_pages(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<QuestionPagesQuery>,
) -> Response {
    if state.hub.session_info(&session_id).is_none() {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    let limit = query.limit.unwrap_or(64).clamp(1, 100);
    let result = match &state.store {
        Some(store) => store
            .question_page_summaries(&session_id, query.before, limit)
            .await
            .map(|(pages, next_before_seq, total)| QuestionPagesResponse {
                total,
                exact: true,
                pages,
                next_before_seq,
            }),
        None => Ok(state
            .hub
            .question_page_summaries(&session_id, query.before, limit)
            .map_or(
                QuestionPagesResponse {
                    total: 0,
                    exact: false,
                    pages: Vec::new(),
                    next_before_seq: None,
                },
                |(pages, next_before_seq, total, exact)| QuestionPagesResponse {
                    total: u64::try_from(total).unwrap_or(u64::MAX),
                    exact,
                    pages,
                    next_before_seq,
                },
            )),
    };
    match result {
        Ok(response) => Json(response).into_response(),
        Err(error) => {
            tracing::warn!(
                session = %session_id,
                error = %error,
                "question page count query failed"
            );
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "question pages unavailable",
            )
                .into_response()
        }
    }
}

async fn api_question_page(
    State(state): State<Arc<AppState>>,
    Path((session_id, page_id)): Path<(String, u64)>,
) -> Response {
    if state.hub.session_info(&session_id).is_none() {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    let result = match &state.store {
        Some(store) => store.question_page_at(&session_id, page_id).await,
        None => Ok(state.hub.question_page_at(&session_id, page_id)),
    };
    match result {
        Ok(Some(events)) => (
            // The newest question page can still grow after its root prompt is
            // persisted. A reader may request it while the agent is producing
            // the answer, so treating this route as immutable can pin that
            // partial response on one device for a year. Cursor history remains
            // immutable; explicit question-page reads always revalidate.
            [(header::CACHE_CONTROL, "no-store")],
            Json(HistoryResponse {
                events,
                next_before_seq: None,
                reached_start: false,
            }),
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(
                session = %session_id,
                page_id,
                %error,
                "question page query failed"
            );
            (StatusCode::SERVICE_UNAVAILABLE, "question page unavailable").into_response()
        }
    }
}

async fn api_artifact(
    State(state): State<Arc<AppState>>,
    Extension(_authenticated): Extension<AuthenticatedProductRequest>,
    Path(name): Path<String>,
) -> Response {
    let Some(path) = state
        .store
        .as_ref()
        .and_then(|store| store.artifact_path(&name))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let content_type = mime_guess::from_path(&name)
                .first_or_octet_stream()
                .to_string();
            (
                [
                    (header::CONTENT_TYPE, content_type),
                    (
                        header::CACHE_CONTROL,
                        "public, max-age=31536000, immutable".to_owned(),
                    ),
                ],
                bytes,
            )
                .into_response()
        }
        Err(error) => {
            tracing::warn!(%error, artifact = %name, "reading artifact failed");
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

async fn plugin_release_artifact(
    State(state): State<Arc<AppState>>,
    Path((digest, name)): Path<(String, String)>,
) -> Response {
    let Some(path) = state
        .provider_catalog
        .published_artifact_path(&digest, &name)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(error) => {
            tracing::info!(%error, artifact = %path.display(), "Plugin artifact is unavailable");
            return StatusCode::NOT_FOUND.into_response();
        }
    };
    let metadata = match file.metadata().await {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(%error, artifact = %path.display(), "reading Plugin artifact metadata failed");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let digest = digest
        .strip_prefix("sha256:")
        .unwrap_or(&digest)
        .to_ascii_lowercase();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header("x-content-type-options", "nosniff")
        .header(header::CONTENT_LENGTH, metadata.len())
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .header(header::ETAG, format!("\"sha256:{digest}\""))
        .body(Body::from_stream(ReaderStream::new(file)))
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "building Plugin artifact response failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })
}

fn is_admin_console_path(requested: &str) -> bool {
    requested == "admin" || requested.starts_with("admin/")
}

/// Resolve the on-disk document for a static request.
///
/// `/admin` and `/admin/*` never fall back to the session PWA `index.html`.
/// When `admin.html` is present they return that document; otherwise 404.
async fn resolve_static_document(
    web_root: &FsPath,
    requested: &str,
) -> Result<(String, Vec<u8>), StatusCode> {
    let requested_path = web_root.join(requested);
    match tokio::fs::read(&requested_path).await {
        Ok(bytes) => Ok((requested.to_owned(), bytes)),
        Err(_) if is_admin_console_path(requested) => {
            match tokio::fs::read(web_root.join("admin.html")).await {
                Ok(bytes) => Ok(("admin.html".to_owned(), bytes)),
                Err(_) => Err(StatusCode::NOT_FOUND),
            }
        }
        Err(_) if requested.starts_with("assets/") => Err(StatusCode::NOT_FOUND),
        Err(_) => match tokio::fs::read(web_root.join("index.html")).await {
            Ok(bytes) => Ok(("index.html".to_owned(), bytes)),
            Err(_) => Err(StatusCode::NOT_FOUND),
        },
    }
}

fn static_content_type(name: &str) -> String {
    if name == ".well-known/apple-app-site-association" {
        "application/json".to_owned()
    } else {
        mime_guess::from_path(name)
            .first_or_octet_stream()
            .to_string()
    }
}

/// Serve a separately deployed asset by path, falling back to `index.html` so
/// the SPA owns client-side routing. Missing hashed assets never fall back: a
/// module request must receive a real 404 rather than HTML with a 200 status.
/// Missing `index.html` (UI not built) → 404.
/// `/admin` and `/admin/*` serve `admin.html` and never the session shell.
///
/// Caching: a per-file SHA256 is used as a content `ETag` (stable across
/// rollouts when the bytes are unchanged). The
/// cache policy is split by whether the filename is content-addressed:
///   - `/assets/*` — Vite emits content-hashed names, so the bytes behind a name
///     never change → `immutable` with a one-year max-age, never revalidated.
///   - everything else (index.html, admin.html, sw.js, manifest, favicon, icons) —
///     `no-cache`: the browser may store it but MUST revalidate via the `ETag` on
///     every use, so a redeploy is picked up immediately while unchanged files
///     cost only a 304. This is what stops a redeployed favicon/icon from being
///     pinned to a stale copy in the browser's HTTP cache.
async fn static_handler(
    State(state): State<Arc<AppState>>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let requested = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };

    // Never allow a URI to escape the configured asset root. Percent-encoded
    // traversal remains a literal filename at this layer and is harmless.
    if !FsPath::new(requested)
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Serve the asset if it exists; otherwise fall back to index.html so the
    // SPA handles the route. Admin routes never take that fallback. Vite's
    // /assets names are content-addressed files, never client-side routes.
    // Returning index.html for a missing old chunk makes browsers report the
    // opaque "Importing a module script failed" error because a JS import
    // received text/html.
    let requested_path = state.web_root.join(requested);
    let (name, content) = match resolve_static_document(&state.web_root, requested).await {
        Ok(document) => document,
        Err(status) if is_admin_console_path(requested) => {
            tracing::info!(
                requested = %requested_path.display(),
                "admin console is not in the web root"
            );
            return status.into_response();
        }
        Err(status) if requested.starts_with("assets/") => {
            tracing::info!(
                requested = %requested_path.display(),
                "requested web asset is no longer deployed"
            );
            return status.into_response();
        }
        Err(status) => {
            tracing::warn!(
                requested = %requested_path.display(),
                "reading web asset failed"
            );
            return (status, "UI not built").into_response();
        }
    };

    // Content ETag uses the same hash function as `/version`.
    let etag = format!("\"{}\"", content_hash(&content));

    // Conditional request: the browser echoes our ETag in If-None-Match; if it
    // still matches, skip the body. `contains` (not strict equality) tolerates a
    // comma-list or a `W/` weak prefix some clients send.
    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        && inm.contains(etag.as_str())
    {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag.as_str())]).into_response();
    }

    let cache_control = if name.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        // `no-store`, NOT `no-cache`. iOS WKWebView (the native Tauri shell) caches
        // the HTML shell + sw.js under NSURLCache and serves it stale even with
        // `no-cache`/ETag revalidation — so a redeploy never reached the app until
        // a manual cache wipe. `no-store` forbids storing it at all, so every cold
        // start re-fetches index.html → its new hashed asset url → the fresh
        // bundle. The files this covers are tiny (HTML, sw.js, manifest, icons).
        "no-store"
    };
    let content_type = static_content_type(&name);
    if name == "passkey.html" {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type.as_str())
            .header(header::CACHE_CONTROL, cache_control)
            .header(header::ETAG, etag.as_str())
            .header("referrer-policy", "no-referrer")
            .header("x-content-type-options", "nosniff")
            .header("x-frame-options", "DENY")
            .header(
                "content-security-policy",
                "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                 connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
            )
            .header(
                "permissions-policy",
                "publickey-credentials-create=(self), publickey-credentials-get=(self)",
            )
            .body(Body::from(content))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, cache_control.to_owned()),
            (header::ETAG, etag),
        ],
        Body::from(content),
    )
        .into_response()
}

#[cfg(test)]
mod static_asset_tests {
    use super::{is_admin_console_path, resolve_static_document, static_content_type};
    use axum::http::StatusCode;

    #[test]
    fn admin_console_paths_are_distinct_from_the_session_spa() {
        assert!(is_admin_console_path("admin"));
        assert!(is_admin_console_path("admin/accounts"));
        assert!(!is_admin_console_path("admin.html"));
        assert!(!is_admin_console_path("index.html"));
        assert!(!is_admin_console_path("sessions"));
    }

    #[test]
    fn apple_app_site_association_is_served_as_json() {
        assert_eq!(
            static_content_type(".well-known/apple-app-site-association"),
            "application/json"
        );
        assert_eq!(static_content_type("index.html"), "text/html");
    }

    #[tokio::test]
    async fn admin_paths_serve_admin_html_not_the_session_spa() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-admin-static-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), b"session-spa").unwrap();
        std::fs::write(root.join("admin.html"), b"admin-console").unwrap();

        let (name, body) = resolve_static_document(&root, "admin").await.unwrap();
        assert_eq!(name, "admin.html");
        assert_eq!(body, b"admin-console");

        let (name, body) = resolve_static_document(&root, "admin/accounts")
            .await
            .unwrap();
        assert_eq!(name, "admin.html");
        assert_eq!(body, b"admin-console");

        let (name, body) = resolve_static_document(&root, "index.html").await.unwrap();
        assert_eq!(name, "index.html");
        assert_eq!(body, b"session-spa");

        let (name, body) = resolve_static_document(&root, "sessions").await.unwrap();
        assert_eq!(name, "index.html");
        assert_eq!(body, b"session-spa");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn missing_admin_html_is_not_the_session_spa() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-admin-static-missing-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), b"session-spa").unwrap();

        let status = resolve_static_document(&root, "admin").await.unwrap_err();
        assert_eq!(status, StatusCode::NOT_FOUND);

        std::fs::remove_dir_all(&root).unwrap();
    }
}

/// App-level WS heartbeat interval. 25s stays under the common 60s proxy/idle
/// timeout (the 75% rule) and keeps NAT mappings warm; the client treats missing
/// ~2 of these as a dead socket. See [`crate::core::Outbound::Ping`].
const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(25);

#[derive(Debug, Default, Deserialize)]
struct WebSocketQuery {
    bootstrap: Option<String>,
}

#[derive(Clone)]
struct WebSocketAuthentication {
    principal: ProductPrincipal,
    via_cookie: bool,
    cookie_token: Option<String>,
    bearer: Option<String>,
    device_identity: Option<crate::client_auth::DeviceAccessIdentity>,
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Query(query): Query<WebSocketQuery>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_ws_upgrade(
        &headers,
        peer,
        &state.public_origins,
        Some(authenticated.principal),
    ) {
        Ok(principal) => principal,
        Err(status) => return status.into_response(),
    };
    let lazy_bootstrap = query.bootstrap.as_deref() == Some("lazy");
    let via_cookie = state.product_auth_enabled
        && crate::product_auth::user_cookie_token(&headers).is_some()
        && crate::product_auth::bearer_token(&headers).is_none();
    let cookie_token = crate::product_auth::user_cookie_token(&headers);
    let bearer = crate::product_auth::bearer_token(&headers);
    let device_identity = authenticated.device_identity;
    ws.on_upgrade(move |socket| {
        handle_ws(
            socket,
            state,
            lazy_bootstrap,
            WebSocketAuthentication {
                principal,
                via_cookie,
                cookie_token,
                bearer,
                device_identity,
            },
        )
    })
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

fn project_outbound(
    hub: &Hub,
    principal: &ProductPrincipal,
    visible: &std::collections::HashSet<String>,
    message: Outbound,
) -> Option<Outbound> {
    match message {
        Outbound::Sessions { sessions } => Some(Outbound::Sessions {
            sessions: sessions
                .into_iter()
                .filter(|meta| principal.can_see(meta.owner_user_id.as_deref()))
                .collect(),
        }),
        Outbound::Snapshot { ref session_id, .. }
        | Outbound::ConfigOptions { ref session_id, .. } => {
            session_is_visible(hub, principal, session_id).then_some(message)
        }
        Outbound::Event { ref envelope } => {
            session_is_visible(hub, principal, &envelope.session_id).then_some(message)
        }
        Outbound::Error {
            session_id: Some(ref session_id),
            ..
        } => session_is_visible(hub, principal, session_id).then_some(message),
        Outbound::Error {
            session_id: None, ..
        }
        | Outbound::Machines { .. }
        | Outbound::AuthSession { .. }
        | Outbound::Ping
        | Outbound::BootstrapComplete
        | Outbound::Settings { .. } => Some(message),
        Outbound::SyncPatch {
            state,
            version,
            value,
            confirmed,
            resync,
        } => {
            if state == "title" || state == "order" {
                Some(Outbound::SyncPatch {
                    value: project_sync_value(&state, value, visible),
                    state,
                    version,
                    confirmed,
                    resync,
                })
            } else if let Some(session_id) = session_id_from_sync_state(&state) {
                session_is_visible(hub, principal, session_id).then_some(Outbound::SyncPatch {
                    state,
                    version,
                    value,
                    confirmed,
                    resync,
                })
            } else {
                Some(Outbound::SyncPatch {
                    state,
                    version,
                    value,
                    confirmed,
                    resync,
                })
            }
        }
    }
}

fn ws_close_auth_required() -> Message {
    Message::Close(Some(CloseFrame {
        code: WS_AUTH_REQUIRED_CLOSE_CODE,
        reason: "auth_required".into(),
    }))
}

async fn principal_still_valid(
    state: &AppState,
    via_cookie: bool,
    cookie_token: Option<&str>,
    bearer: Option<&str>,
    device_identity: Option<&crate::client_auth::DeviceAccessIdentity>,
    expected: &ProductPrincipal,
) -> bool {
    if !state.product_auth_enabled {
        return true;
    }
    let Some(store) = state.store.as_ref() else {
        return false;
    };
    let current = if let Some(identity) = device_identity {
        let Some(token) = bearer else {
            return false;
        };
        if !state
            .device_access
            .token_still_valid(token, identity, auth_now_ms())
        {
            return false;
        }
        match store.user_by_id(&identity.user_id).await {
            Ok(Some(user)) if user.disabled_at_ms.is_none() => {
                Some(product_principal(&state.hub, &user))
            }
            _ => None,
        }
    } else if via_cookie {
        let Some(token) = cookie_token else {
            return false;
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("{}={token}", crate::product_auth::USER_SESSION_COOKIE)
                .parse()
                .unwrap_or_else(|_| header::HeaderValue::from_static("")),
        );
        match product_session_and_user_from_store_cookie(store, &headers).await {
            Some((session, user))
                if ensure_product_session_fresh(store, &session, &state.product_authentication)
                    .await
                    .is_ok() =>
            {
                Some(product_principal(&state.hub, &user))
            }
            _ => None,
        }
    } else if let Some(token) = bearer {
        product_from_bearer(store, &state.hub, token).await
    } else {
        None
    };
    current.is_some_and(|principal| {
        principal.user_id == expected.user_id && !principal.username.is_empty()
    })
}

struct ProductAuthSessionSnapshot {
    message: Outbound,
    valid: bool,
    user_id: String,
    next_due_at_ms: i64,
}

fn product_auth_deadline_instant(due_at_ms: i64) -> tokio::time::Instant {
    const MAX_DELAY_MS: u64 = 91 * 24 * 60 * 60 * 1_000;
    let delay_ms = (due_at_ms.saturating_sub(auth_now_ms()).max(1) as u64).min(MAX_DELAY_MS);
    tokio::time::Instant::now() + std::time::Duration::from_millis(delay_ms)
}

async fn product_auth_session_message(
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

async fn handle_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    lazy_bootstrap: bool,
    authentication: WebSocketAuthentication,
) {
    let WebSocketAuthentication {
        principal,
        via_cookie,
        cookie_token,
        bearer,
        device_identity,
    } = authentication;
    let (mut sink, mut stream) = socket.split();

    // A cookie session must pass its current idle, Passkey, and primary-login
    // deadlines before any transcript or Machine snapshot is disclosed. Push
    // the required proof to the client first, then close without bootstrapping
    // when the session is no longer fresh.
    let initial_auth_deadline_ms = if let Some(token) = cookie_token.as_deref() {
        let Some(snapshot) = product_auth_session_message(&state, token).await else {
            let _ = sink.send(ws_close_auth_required()).await;
            return;
        };
        if send_json(&mut sink, &snapshot.message).await.is_err() {
            return;
        }
        if !snapshot.valid || snapshot.user_id != principal.user_id {
            let _ = sink.send(ws_close_auth_required()).await;
            return;
        }
        Some(snapshot.next_due_at_ms)
    } else {
        None
    };

    // Subscribe BEFORE snapshotting so no event slips through the gap; the
    // client dedups by (session_id, seq), so a brief overlap is harmless.
    let mut rx = state.hub.subscribe();
    let mut shutdown = state.shutdown.clone();

    let machine_snapshot = match state.machine_snapshots.connect_message().await {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            // Never project an unavailable registry as an empty one: the
            // browser keeps its last authoritative snapshot across reconnects.
            tracing::warn!(%error, "loading Machine WebSocket bootstrap");
            None
        }
    };
    for message in connect_bootstrap(&state.hub, lazy_bootstrap, &principal, machine_snapshot) {
        if send_json(&mut sink, &message).await.is_err() {
            return;
        }
    }
    // Fan-out task: broadcast events → this socket, plus a periodic app-level
    // heartbeat (Outbound::Ping) so a client can detect a HALF-OPEN socket that
    // never fires `onclose` (see Outbound::Ping). Per-client interval — a failed
    // heartbeat send reaps a dead client here too.
    let fanout_state = Arc::clone(&state);
    let fanout_principal = principal.clone();
    let fanout_cookie = cookie_token.clone();
    let fanout_bearer = bearer.clone();
    let fanout_device = device_identity.clone();
    let (direct_tx, mut direct_rx) =
        tokio::sync::mpsc::unbounded_channel::<Option<ProductAuthSessionSnapshot>>();
    let mut fanout = tokio::spawn(async move {
        let mut heartbeat = tokio::time::interval(HEARTBEAT);
        let auth_deadline = tokio::time::sleep_until(product_auth_deadline_instant(
            initial_auth_deadline_ms.unwrap_or(i64::MAX),
        ));
        tokio::pin!(auth_deadline);
        // The first tick fires immediately; consume it so the first ping waits a
        // full interval (the connect snapshot is fresh traffic already).
        heartbeat.tick().await;
        loop {
            tokio::select! {
                Some(update) = direct_rx.recv() => {
                    let Some(snapshot) = update else {
                        let _ = sink.send(ws_close_auth_required()).await;
                        break;
                    };
                    if send_json(&mut sink, &snapshot.message).await.is_err() {
                            break;
                    }
                    if !snapshot.valid || snapshot.user_id != fanout_principal.user_id {
                        let _ = sink.send(ws_close_auth_required()).await;
                        break;
                    }
                    auth_deadline
                        .as_mut()
                        .reset(product_auth_deadline_instant(snapshot.next_due_at_ms));
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
                msg = rx.recv() => match msg {
                    Ok(msg) => {
                        let result = if fanout_principal.sees_every_session() {
                            send_frame(&mut sink, msg.as_ref()).await
                        } else {
                            let visible = visible_session_ids(&fanout_state.hub, &fanout_principal);
                            let Some(projected) = project_outbound(
                                &fanout_state.hub,
                                &fanout_principal,
                                &visible,
                                msg.outbound().clone(),
                            ) else {
                                continue;
                            };
                            send_json(&mut sink, &projected).await
                        };
                        if result.is_err() {
                            break;
                        }
                    }
                    // Lagged: a slow client (mobile/5G, or backgrounded) fell
                    // >1024 events behind, so the broadcast DROPPED events for it.
                    // Its timeline is now permanently inconsistent — e.g. it missed
                    // the tool_call_update that completed a tool, so the UI shows a
                    // stuck "pending" tool / "working" spinner on an idle session
                    // (the observed bug). Continuing would keep serving newer events
                    // over that hole forever. Instead CLOSE the socket: the client
                    // reconnects and rebuilds a consistent state from a fresh
                    // snapshot (the connect path re-sends sessions + snapshots).
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(dropped = n, "WS client lagged; closing to force a resync");
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = &mut auth_deadline, if via_cookie => {
                    let Some(token) = fanout_cookie.as_deref() else {
                        let _ = sink.send(ws_close_auth_required()).await;
                        break;
                    };
                    let Some(snapshot) = product_auth_session_message(&fanout_state, token).await else {
                        let _ = sink.send(ws_close_auth_required()).await;
                        break;
                    };
                    if send_json(&mut sink, &snapshot.message).await.is_err() {
                        break;
                    }
                    if !snapshot.valid || snapshot.user_id != fanout_principal.user_id {
                        let _ = sink.send(ws_close_auth_required()).await;
                        break;
                    }
                    auth_deadline
                        .as_mut()
                        .reset(product_auth_deadline_instant(snapshot.next_due_at_ms));
                }
                _ = heartbeat.tick() => {
                    let valid = if via_cookie {
                        match fanout_cookie.as_deref() {
                            Some(token) => match product_auth_session_message(&fanout_state, token).await {
                                Some(snapshot) => {
                                    if send_json(&mut sink, &snapshot.message).await.is_err() {
                                        break;
                                    }
                                    auth_deadline
                                        .as_mut()
                                        .reset(product_auth_deadline_instant(snapshot.next_due_at_ms));
                                    snapshot.valid && snapshot.user_id == fanout_principal.user_id
                                }
                                None => false,
                            },
                            None => false,
                        }
                    } else {
                        principal_still_valid(
                            &fanout_state,
                            false,
                            None,
                            fanout_bearer.as_deref(),
                            fanout_device.as_ref(),
                            &fanout_principal,
                        )
                        .await
                    };
                    if !valid {
                        tracing::info!(reason = "disabled", "ws_rejected");
                        let _ = sink.send(ws_close_auth_required()).await;
                        break;
                    }
                    if send_json(&mut sink, &Outbound::Ping).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Edit-holds this connection set (session_id → message id + lease epoch). The
    // editing hold is GLOBAL server state, so a client that disconnects mid-edit
    // would otherwise leave the head pinned and stall the queue forever. We
    // track what this socket held and release it after a short reload grace.
    let mut held: HashMap<String, (String, u64)> = HashMap::new();
    let mut wait_for_auth_close = false;

    // Inbound command loop.
    loop {
        tokio::select! {
            _ = &mut fanout => break,
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if !principal_still_valid(
                            &state,
                            via_cookie,
                            cookie_token.as_deref(),
                            bearer.as_deref(),
                            device_identity.as_ref(),
                            &principal,
                        )
                        .await
                        {
                            tracing::info!(reason = "expired_or_step_up_required", "ws_rejected");
                            let _ = direct_tx.send(None);
                            wait_for_auth_close = true;
                            break;
                        }
                        if matches!(
                            serde_json::from_str::<Inbound>(&text),
                            Ok(Inbound::AuthActivity)
                        ) {
                            if let (Some(store), Some(token)) =
                                (state.store.as_ref(), cookie_token.as_deref())
                            {
                                let token_hash = crate::admin::hex_sha256(token.as_bytes());
                                if let Err(error) = store
                                    .touch_user_session_activity(&token_hash, auth_now_ms())
                                    .await
                                {
                                    tracing::warn!(%error, "touching user session activity");
                                }
                                let direct = product_auth_session_message(&state, token).await;
                                let _ = direct_tx.send(direct);
                            }
                            continue;
                        }
                        handle_command(
                            &state,
                            &principal,
                            &text,
                            &mut held,
                        );
                    }
                    // Other frame types (ping/pong/binary) are ignored.
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_))) => {}
                    // Close, transport error, or stream end: tear down.
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                }
            }
        }
    }
    // A service-worker update or iOS WebView restart briefly replaces the socket.
    // Keep the queue pinned while the restored local edit reconnects and reasserts
    // its hold. Epoch matching prevents this old socket's timer from releasing a
    // replacement connection; a genuine abandoned edit drains after the grace.
    for (session_id, (id, epoch)) in held {
        let hub = state.hub.clone();
        tokio::spawn(async move {
            tokio::time::sleep(QUEUE_EDIT_RECONNECT_GRACE).await;
            hub.release_queue_editing_if_epoch(&session_id, &id, epoch);
        });
    }
    if wait_for_auth_close {
        let _ = fanout.await;
    } else {
        fanout.abort();
    }
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
fn handle_command(
    state: &AppState,
    principal: &ProductPrincipal,
    text: &str,
    held: &mut HashMap<String, (String, u64)>,
) {
    let cmd: Inbound = match serde_json::from_str(text) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "bad inbound command");
            state
                .hub
                .broadcast_error(None, format!("bad inbound command: {e}"));
            return;
        }
    };
    // Capture session_id ahead of the match for error attribution. Most
    // commands carry one; NewSession doesn't (the session id is assigned
    // by the daemon after success).
    let session_id_for_err: Option<String> = match &cmd {
        Inbound::Prompt { session_id, .. }
        | Inbound::Cancel { session_id }
        | Inbound::CancelSubmitted { session_id, .. }
        | Inbound::Permission { session_id, .. }
        | Inbound::DeleteSession { session_id }
        | Inbound::RenameSession { session_id, .. }
        | Inbound::SetSessionAutoResume { session_id, .. }
        | Inbound::SetPaused { session_id, .. }
        | Inbound::ResumeTurn { session_id }
        | Inbound::RetryTurn { session_id }
        | Inbound::SetConfigOption { session_id, .. }
        | Inbound::OpenSession { session_id }
        | Inbound::ResetSession { session_id }
        | Inbound::Submit { session_id, .. }
        | Inbound::RemoveQueued { session_id, .. }
        | Inbound::EditQueued { session_id, .. }
        | Inbound::ClearQueue { session_id }
        | Inbound::RequestSendQueued { session_id, .. }
        | Inbound::ForcePushQueued { session_id, .. }
        | Inbound::QueuedToDraft { session_id, .. }
        | Inbound::SetQueueEditing { session_id, .. }
        | Inbound::AddDraft { session_id, .. }
        | Inbound::EditDraft { session_id, .. }
        | Inbound::RemoveDraft { session_id, .. }
        | Inbound::ClearDrafts { session_id }
        | Inbound::ActivateDraft { session_id, .. }
        | Inbound::ActivateAllDrafts { session_id }
        | Inbound::MoveDraft { session_id, .. }
        | Inbound::ScheduleDraft { session_id, .. }
        | Inbound::UnscheduleDraft { session_id, .. }
        | Inbound::ReorderQueue { session_id, .. }
        | Inbound::ReorderDrafts { session_id, .. } => Some(session_id.clone()),
        // Sync mutations are state-scoped (title/order), not session-scoped — a
        // failure surfaces as a daemon-level error (None).
        Inbound::AuthActivity
        | Inbound::NewSession { .. }
        | Inbound::ReorderSessions { .. }
        | Inbound::Sync { .. }
        | Inbound::SetSetting { .. } => None,
    };
    if let Some(sid) = &session_id_for_err {
        let owner = state
            .hub
            .session_info(sid)
            .and_then(|info| info.meta.owner_user_id);
        if !principal.can_mutate(owner.as_deref()) {
            state.hub.broadcast_error(
                Some(sid.clone()),
                "not allowed to mutate this session".to_owned(),
            );
            return;
        }
    }
    // A view-only system session rejects user-driven turns; only the backend
    // wake endpoint (POST /api/sessions/{id}/prompt) drives it.
    if let Some(sid) = &session_id_for_err
        && matches!(&cmd, Inbound::Prompt { .. } | Inbound::Submit { .. })
        && state.hub.session_is_system(sid)
    {
        state.hub.broadcast_error(
            Some(sid.clone()),
            "view-only system session: input is disabled".to_owned(),
        );
        return;
    }
    // Serialize prompt admission against Provider lifecycle changes. Holding
    // this read guard through the command match closes the check-then-dispatch
    // race with uninstall's active-turn snapshot.
    let provider_prompt_key = session_id_for_err
        .as_deref()
        .filter(|_| matches!(&cmd, Inbound::Prompt { .. } | Inbound::Submit { .. }))
        .and_then(|sid| provider_fence_key_for_session(&state.hub, sid));
    let plugin_prompt_fence = provider_prompt_key
        .as_ref()
        .map(|_| state.plugin_lifecycle_fences.read());
    if provider_prompt_key
        .as_ref()
        .zip(plugin_prompt_fence.as_ref())
        .and_then(|(key, fences)| fences.get(key))
        .is_some_and(|fence| *fence != PluginFenceState::Installing)
    {
        state.hub.broadcast_error(
            session_id_for_err,
            "the session Provider is uninstalling from its Machine".to_owned(),
        );
        return;
    }
    let result = match cmd {
        Inbound::AuthActivity => Ok(()),
        Inbound::NewSession { .. } => Err(
            "legacy WebSocket session creation is disabled; use POST /api/sessions with a connected Machine"
                .to_owned(),
        ),
        Inbound::Prompt {
            session_id,
            text,
            content,
        } => {
            // Derive an auto-title from the first prompt before text/content are
            // consumed below. auto_title no-ops unless the title is still the
            // cwd default, so this only "takes" on a session's first prompt and
            // never overrides a manual rename.
            let auto = first_prompt_title(&text, &content);
            let blocks: Vec<ContentBlock> = if content.is_empty() {
                if text.is_empty() {
                    tracing::warn!("Prompt with neither text nor content; dropping");
                    state.hub.broadcast_error(
                        Some(session_id),
                        "empty prompt: no text or content blocks".to_owned(),
                    );
                    return;
                }
                vec![ContentBlock::from(text)]
            } else {
                content
                    .into_iter()
                    .filter_map(|v| match serde_json::from_value::<ContentBlock>(v) {
                        Ok(b) => Some(b),
                        Err(e) => {
                            tracing::warn!(error = %e, "skipping unparseable Prompt content block");
                            None
                        }
                    })
                    .collect()
            };
            // API direct prompt — no optimistic client, so no cmid.
            let result = state
                .supervisor
                .send(&session_id, AgentCommand::Prompt(blocks, None, None));
            if result.is_ok()
                && let Some(title) = auto
            {
                state.hub.auto_title(&session_id, title);
            }
            result
        }
        Inbound::Cancel { session_id } => state.supervisor.send(&session_id, AgentCommand::Cancel),
        Inbound::CancelSubmitted { session_id, cmid } => {
            if state.hub.remove_queued_by_cmid(&session_id, &cmid) {
                // Explicit acknowledgement for the ACP bridge. Absence of this
                // event means the prompt crossed the queue→dispatch boundary;
                // the bridge then waits for its cmid echo and cancels the now-
                // active turn, avoiding both a false cancellation response and
                // interruption of another surface's turn.
                state.hub.push(
                    &session_id,
                    Event::Update {
                        update: serde_json::json!({
                            "sessionUpdate": "cowboy_prompt_cancelled",
                            "cmid": cmid,
                        }),
                    },
                );
            }
            Ok(())
        }
        Inbound::Permission {
            session_id,
            request_id,
            option_id,
        } => state.supervisor.send(
            &session_id,
            AgentCommand::Permission {
                request_id,
                option_id,
            },
        ),
        Inbound::DeleteSession { session_id } => {
            // Order: tear down agent thread first (so it doesn't push more
            // events into a soon-to-be-gone Hub session), then drop Hub state
            // + broadcast updated list.
            state.supervisor.delete_session(&session_id);
            state.hub.delete_session(&session_id);
            Ok(())
        }
        Inbound::RenameSession { session_id, title } => {
            // Empty title is a UI bug; reject server-side so the toast lands.
            let trimmed = title.trim().to_owned();
            if trimmed.is_empty() {
                Err("title cannot be empty".to_owned())
            } else {
                state.hub.rename_session(&session_id, trimmed);
                Ok(())
            }
        }
        Inbound::Sync {
            state: sync_state,
            id,
            name,
            args,
        } => apply_inbound_sync(state, principal, &sync_state, id, &name, &args),
        Inbound::SetSessionAutoResume { .. }
        | Inbound::ResumeTurn { .. }
        | Inbound::SetSetting { .. } => Ok(()),
        Inbound::SetPaused { session_id, paused } => {
            state.hub.set_paused(&session_id, paused);
            Ok(())
        }
        Inbound::RetryTurn { session_id } => {
            state.supervisor.prepare_session(&session_id).map(|_| {
                state.hub.retry_turn(&session_id);
            })
        }
        Inbound::SetConfigOption {
            session_id,
            config_id,
            value,
        } => {
            if config_id == crate::deepseek_context::CONFIG_ID {
                state
                    .supervisor
                    .set_deepseek_context_profile(&session_id, value)
            } else if config_id == crate::deepseek_cache::CONFIG_ID {
                state
                    .supervisor
                    .set_deepseek_cache_protection(&session_id, value)
            } else {
                state
                    .hub
                    .set_config_preference(&session_id, config_id.clone(), value.clone())
                    .and_then(|()| {
                        state.supervisor.send(
                            &session_id,
                            AgentCommand::SetConfigOption { config_id, value },
                        )
                    })
            }
        }
        // Revive on open (design §7): warm the agent when the client selects
        // the session, not only on the first prompt. No-op if already alive.
        Inbound::OpenSession { session_id } => {
            // Do NOT revive an INTERRUPTED session on open. Reviving would flip it
            // to Starting→Running, hiding the fact that its last turn was cut off
            // by a restart — and a stale in-flight tool from that turn would then
            // drive a misleading "working" spinner (the reported bug: after a
            // deploy + reload it looked like the agent was still thinking). Left
            // interrupted, the client shows the "last turn was interrupted — send
            // a message to start a new one" bar and no spinner; submitting a
            // message revives it via the drain. Exited/dormant sessions (nothing
            // unfinished) still pre-revive on open so they're ready to type into.
            let meta = state
                .hub
                .session_list()
                .into_iter()
                .find(|meta| meta.id == session_id);
            let terminal_provider_error = meta.as_ref().is_some_and(|meta| {
                meta.status == Status::Crashed
                    && state
                        .hub
                        .latest_crash_detail(&session_id)
                        .as_deref()
                        .is_some_and(|detail| {
                            let behavior = meta
                                .provider_behavior
                                .clone()
                                .unwrap_or_else(|| crate::provider::legacy_behavior(&meta.provider));
                            behavior
                                .matching_error_rule(detail)
                                .is_some_and(|rule| rule.user_detail.is_some())
                        })
            });
            if terminal_provider_error
                || state.hub.status(&session_id) == Some(Status::Interrupted)
            {
                // Interrupted sessions remain stopped until the user submits a
                // message or explicitly sends queued work.
                if terminal_provider_error {
                    tracing::info!(
                        session_id = %session_id,
                        "not auto-reviving session with terminal provider startup error"
                    );
                }
                Ok(())
            } else {
                // A client opens the focus it restored from localStorage on
                // reload. If that session is gone (deleted while the client was
                // away), this is NOT an error condition: the client already pops a
                // one-shot *warning* snackbar and falls back to another session.
                // Log a server-side warning and swallow the error so no error
                // toast is broadcast (which would otherwise read as a hard
                // failure).
                match state.supervisor.ensure_alive(&session_id) {
                    Ok(_) => Ok(()),
                    Err(e) => {
                        tracing::warn!(
                            session_id = %session_id,
                            error = %e,
                            "open of unknown/gone session ignored — client will fall back",
                        );
                        Ok(())
                    }
                }
            }
        }
        Inbound::ResetSession { session_id } => {
            // "Clear conversation" (see Inbound::ResetSession). Order matters:
            // 1. Forget the resumable agent id so the respawn does a FRESH
            //    session/new (clean context) instead of session/load.
            // 2. Destructively clear the old in-memory + durable transcript.
            // 3. Drop the new timeline boundary marker.
            // 4. Atomically fence + replace the worker. This must not use the
            //    permanent delete path: the Machine broker retains delete tombstones to
            //    reject stale launches for genuinely deleted sessions.
            state.hub.prepare_context_reset(&session_id);
            state.hub.clear_transcript(&session_id);
            state.hub.mark_context_cleared(&session_id);
            match state.supervisor.reset_session(&session_id) {
                Ok(()) => Ok(()),
                Err(e) => {
                    tracing::warn!(session_id = %session_id, error = %e, "reset: respawn failed");
                    Ok(())
                }
            }
        }

        // --- Server-authoritative queue + drafts ------------------------------
        // These mutate Hub state, which broadcasts the new queue/drafts to every
        // terminal. They never fail in a way worth a toast, so all return Ok.
        Inbound::Submit {
            session_id,
            text,
            content,
            cmid,
            force,
            front,
        } => {
            if force {
                // Long-press send: jump to the front of the queue and interrupt the
                // running turn so it runs next (same end-state as a queued row's
                // force-push). force_submit returns true when it queued (vs a direct
                // idle dispatch); only then, and only on a live turn, do we Cancel.
                let queued = state
                    .hub
                    .force_submit(&session_id, text, content, cmid, true);
                if queued
                    && matches!(
                        state.hub.status(&session_id),
                        Some(Status::Busy | Status::Starting)
                    )
                {
                    force_cancel_with_watchdog(state, &session_id)
                } else {
                    // Not busy: force_submit front-inserted the prompt but nothing
                    // dispatched it — a PAUSED queue HOLDS the auto-drain. A
                    // force-push is an explicit "send this now", so
                    // drain the head MANUALLY here: bypass the hold and run it now,
                    // WITHOUT resuming the rest of the held queue. No-op when the
                    // head already dispatched (idle + empty queue).
                    state.hub.drain_now(&session_id);
                    Ok(())
                }
            } else if front {
                // "Jump to front" without interrupting: front-insert so it runs next
                // after the current turn, ahead of the rest of the queue. Same
                // front placement as force, but no Cancel.
                let _ = state
                    .hub
                    .force_submit(&session_id, text, content, cmid, false);
                Ok(())
            } else {
                state.hub.submit(&session_id, text, content, cmid);
                Ok(())
            }
        }
        Inbound::RemoveQueued { session_id, id } => {
            state.hub.remove_queued(&session_id, &id);
            Ok(())
        }
        Inbound::EditQueued {
            session_id,
            id,
            text,
            content,
        } => {
            state.hub.edit_queued(&session_id, &id, text, content);
            Ok(())
        }
        Inbound::ClearQueue { session_id } => {
            state.hub.clear_queue(&session_id);
            Ok(())
        }
        Inbound::RequestSendQueued { session_id, id } => {
            state.hub.request_send_queued(&session_id, &id);
            Ok(())
        }
        Inbound::ForcePushQueued { session_id, id } => {
            // Interrupt the running turn so the promoted prompt runs next; on an
            // idle session there's nothing to cancel, so just send it now.
            state.hub.request_send_queued(&session_id, &id); // promote to front either way
            if matches!(
                state.hub.status(&session_id),
                Some(Status::Busy | Status::Starting)
            ) {
                force_cancel_with_watchdog(state, &session_id)
            } else {
                Ok(())
            }
        }
        Inbound::QueuedToDraft { session_id, id } => {
            state.hub.queued_to_draft(&session_id, &id);
            Ok(())
        }
        Inbound::SetQueueEditing { session_id, id } => {
            // Track the hold per-connection so a mid-edit disconnect releases it
            // (the hold is global server state — see handle_ws teardown).
            match &id {
                Some(mid) => {
                    let epoch = state
                        .hub
                        .set_queue_editing(&session_id, Some(mid.clone()));
                    held.insert(session_id.clone(), (mid.clone(), epoch));
                }
                None => {
                    // A stale socket can deliver its final release after a
                    // replacement socket has already renewed the edit hold.
                    // Release only the epoch owned by THIS connection; the
                    // replacement's newer epoch must keep the queue pinned.
                    if let Some((mid, epoch)) = held.remove(&session_id) {
                        state
                            .hub
                            .release_queue_editing_if_epoch(&session_id, &mid, epoch);
                    }
                }
            }
            Ok(())
        }
        Inbound::AddDraft {
            session_id,
            text,
            content,
            cmid,
        } => {
            state.hub.add_draft(&session_id, text, content, cmid);
            Ok(())
        }
        Inbound::EditDraft {
            session_id,
            id,
            text,
            content,
        } => {
            state.hub.edit_draft(&session_id, &id, text, content);
            Ok(())
        }
        Inbound::RemoveDraft { session_id, id } => {
            state.hub.remove_draft(&session_id, &id);
            Ok(())
        }
        Inbound::ClearDrafts { session_id } => {
            state.hub.clear_drafts(&session_id);
            Ok(())
        }
        Inbound::ActivateDraft { session_id, id } => {
            state.hub.activate_draft(&session_id, &id);
            Ok(())
        }
        Inbound::ActivateAllDrafts { session_id } => {
            state.hub.activate_all_drafts(&session_id);
            Ok(())
        }
        Inbound::MoveDraft {
            session_id,
            id,
            to_session,
        } => {
            state.hub.move_draft(&session_id, &id, &to_session);
            Ok(())
        }
        Inbound::ScheduleDraft {
            session_id,
            id,
            cmid,
            text,
            content,
            fire_at_ms,
            delivery,
        } => {
            state
                .hub
                .schedule_draft(&session_id, id, cmid, text, content, fire_at_ms, delivery);
            Ok(())
        }
        Inbound::UnscheduleDraft { session_id, id } => {
            state.hub.unschedule_draft(&session_id, &id);
            Ok(())
        }
        Inbound::ReorderSessions { order } => apply_visible_reorder(state, principal, &order),
        Inbound::ReorderQueue { session_id, order } => {
            state.hub.reorder_queue(&session_id, &order);
            Ok(())
        }
        Inbound::ReorderDrafts { session_id, order } => {
            state.hub.reorder_drafts(&session_id, &order);
            Ok(())
        }
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, "command failed");
        state
            .hub
            .broadcast_error(session_id_for_err, format!("command failed: {e}"));
    }
}

fn apply_inbound_sync(
    state: &AppState,
    principal: &ProductPrincipal,
    sync_state: &str,
    id: String,
    name: &str,
    args: &serde_json::Value,
) -> Result<(), String> {
    if sync_state == "order" {
        if !principal.can_reorder() {
            return Err("viewers cannot reorder sessions".to_owned());
        }
        let submitted = args
            .get("order")
            .and_then(serde_json::Value::as_array)
            .ok_or("reorder: missing order")?;
        let filtered = filter_submitted_order(state, principal, submitted);
        return state.hub.sync_apply(
            sync_state,
            id,
            name,
            &serde_json::json!({ "order": filtered }),
        );
    }
    if sync_state == "title" {
        let session_id = args
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .ok_or("rename: missing session_id")?;
        let owner = state
            .hub
            .session_info(session_id)
            .and_then(|info| info.meta.owner_user_id);
        if !principal.can_mutate(owner.as_deref()) {
            return Err("not allowed to rename this session".to_owned());
        }
        return state.hub.sync_apply(sync_state, id, name, args);
    }
    if let Some(session_id) = session_id_from_sync_state(sync_state) {
        let owner = state
            .hub
            .session_info(session_id)
            .and_then(|info| info.meta.owner_user_id);
        if !principal.can_mutate(owner.as_deref()) {
            return Err("not allowed to mutate this session".to_owned());
        }
        return state.hub.sync_apply(sync_state, id, name, args);
    }
    Err(format!("unknown sync mutation {sync_state}/{name}"))
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
async fn send_json<S, T>(sink: &mut S, msg: &T) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
    T: Serialize,
{
    send_json_with_timeout(sink, msg, WEBSOCKET_FRAME_SEND_TIMEOUT).await
}

async fn send_frame<S>(sink: &mut S, frame: &FanoutFrame) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let text = frame.json().map_err(|_| ())?;
    tokio::time::timeout(
        WEBSOCKET_FRAME_SEND_TIMEOUT,
        sink.send(Message::Text(text.to_owned().into())),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())
}

async fn send_json_with_timeout<S, T>(
    sink: &mut S,
    msg: &T,
    timeout: std::time::Duration,
) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
    T: Serialize,
{
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    tokio::time::timeout(timeout, sink.send(Message::Text(text.into())))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

#[cfg(test)]
mod code_file_policy_tests {
    use super::{code_file_error_response, zed_language_target};
    use axum::http::StatusCode;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn file_errors_distinguish_missing_binary_and_invalid_requests() {
        assert_eq!(
            code_file_error_response("file not found").status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            code_file_error_response("binary file").status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        assert_eq!(
            code_file_error_response("file is not UTF-8").status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        assert_eq!(
            code_file_error_response("invalid file cursor").status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn local_buffer_leases_require_a_real_file_inside_the_session_worktree() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("cowboy-buffer-policy-{unique}"));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

        let root_text = root.to_string_lossy();
        assert!(zed_language_target("local", &root_text, "src/main.rs").is_some());
        assert!(zed_language_target("local", &root_text, "src/missing.rs").is_none());
        assert!(zed_language_target("local", &root_text, "../outside.rs").is_none());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remote_buffer_leases_are_resolved_on_the_session_machine() {
        let remote_root = "/Users/example/.local/state/cowboy-machine/worktrees/session";
        assert_eq!(
            zed_language_target(
                "macbook-air",
                remote_root,
                "service/workflow/core/webhook_helpers.go",
            ),
            Some((
                remote_root.to_owned(),
                "service/workflow/core/webhook_helpers.go".to_owned(),
            ))
        );
        assert!(zed_language_target("macbook-air", remote_root, "../outside.go").is_none());
        assert!(zed_language_target("macbook-air", remote_root, "/etc/passwd").is_none());
        assert!(zed_language_target("macbook-air", remote_root, "src\\outside.go").is_none());
    }
}

#[cfg(test)]
mod reset_policy_tests {
    use super::{ScheduledResetFailurePolicy, scheduled_reset_failure_policy};

    #[test]
    fn ambiguous_consume_is_never_retried() {
        assert_eq!(
            scheduled_reset_failure_policy(true, 0),
            ScheduledResetFailurePolicy::StopUnknown
        );
    }

    #[test]
    fn only_preflight_failures_receive_two_bounded_retries() {
        assert_eq!(
            scheduled_reset_failure_policy(false, 0),
            ScheduledResetFailurePolicy::RetryPreflight
        );
        assert_eq!(
            scheduled_reset_failure_policy(false, 1),
            ScheduledResetFailurePolicy::RetryPreflight
        );
        assert_eq!(
            scheduled_reset_failure_policy(false, 2),
            ScheduledResetFailurePolicy::StopFailed
        );
    }
}

#[cfg(test)]
mod websocket_send_tests {
    use super::send_json_with_timeout;
    use std::time::Duration;

    #[tokio::test]
    async fn stalled_websocket_write_times_out_so_the_connection_can_reconcile() {
        let sink = futures::sink::unfold((), |(), _message| async move {
            std::future::pending::<Result<(), std::io::Error>>().await
        });
        futures::pin_mut!(sink);

        let result = send_json_with_timeout(
            &mut sink,
            &serde_json::json!({ "type": "runtime" }),
            Duration::from_millis(10),
        )
        .await;

        assert!(result.is_err(), "a stalled WebSocket write must be fenced");
    }
}

#[cfg(test)]
mod code_tree_cache_tests {
    use super::{FileTreeEntry, file_tree_revision};

    #[test]
    fn revision_is_stable_and_covers_visible_tree_state() {
        let entries = vec![FileTreeEntry {
            name: "src".to_owned(),
            path: "src".to_owned(),
            kind: "directory",
            ignored: false,
        }];
        let revision = file_tree_revision("", &entries, false);
        assert_eq!(revision, file_tree_revision("", &entries, false));
        assert_ne!(revision, file_tree_revision("", &entries, true));
        assert_ne!(revision, file_tree_revision("nested", &entries, false));
        let renamed = vec![FileTreeEntry {
            name: "source".to_owned(),
            path: "source".to_owned(),
            kind: "directory",
            ignored: false,
        }];
        assert_ne!(revision, file_tree_revision("", &renamed, false));
        let ignored = vec![FileTreeEntry {
            name: "src".to_owned(),
            path: "src".to_owned(),
            kind: "directory",
            ignored: true,
        }];
        assert_ne!(revision, file_tree_revision("", &ignored, false));
    }
}

#[cfg(test)]
mod zed_adapter_tests {
    use super::*;
    use tokio::net::UnixListener;

    fn test_socket(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "cowboy-zed-{label}-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn ensure_worktree_uses_the_stable_adapter_contract() {
        let socket = test_socket("worktree-client");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut request = String::new();
            BufReader::new(read).read_line(&mut request).await.unwrap();
            let request: serde_json::Value = serde_json::from_str(&request).unwrap();
            assert_eq!(request["type"], "ensureWorktree");
            assert_eq!(request["path"], "/tmp/worktree");
            assert_eq!(request["trusted"], true);
            write
                .write_all(b"{\"type\":\"worktree\",\"api_version\":1,\"state\":\"ready\"}\n")
                .await
                .unwrap();
        });

        assert!(ensure_zed_worktree(&socket, "/tmp/worktree").await.unwrap());
        server.await.unwrap();
        tokio::fs::remove_file(socket).await.unwrap();
    }

    #[tokio::test]
    async fn buffer_lease_uses_the_stable_adapter_contract() {
        let socket = test_socket("buffer-client");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut request = String::new();
            BufReader::new(read).read_line(&mut request).await.unwrap();
            let request: serde_json::Value = serde_json::from_str(&request).unwrap();
            assert_eq!(request["type"], "openBuffer");
            assert_eq!(request["worktree"], "/tmp/worktree");
            assert_eq!(request["path"], "src/main.rs");
            assert_eq!(request["leaseId"], "browser-tab-1");
            write
                .write_all(
                    b"{\"type\":\"buffer\",\"api_version\":1,\"path\":\"src/main.rs\",\"leases\":1}\n",
                )
                .await
                .unwrap();
        });

        let response = zed_adapter_request(
            &socket,
            serde_json::json!({
                "type": "openBuffer",
                "worktree": "/tmp/worktree",
                "path": "src/main.rs",
                "leaseId": "browser-tab-1",
            }),
        )
        .await
        .unwrap();
        assert!(matches!(
            response,
            ZedAdapterResponse::Buffer {
                path,
                leases: 1,
                ..
            } if path == "src/main.rs"
        ));
        server.await.unwrap();
        tokio::fs::remove_file(socket).await.unwrap();
    }
}

#[cfg(test)]
mod bootstrap_tests {
    use super::{connect_bootstrap, focused_session_bootstrap};
    use crate::core::{Event, Hub, Outbound, SessionOrigin};
    use crate::product_auth::ProductPrincipal;

    fn test_owner_principal() -> ProductPrincipal {
        ProductPrincipal {
            user_id: "owner".to_owned(),
            username: "owner".to_owned(),
            role: crate::admin::AdminRole::Owner,
        }
    }

    fn hub_with_sessions() -> Hub {
        let hub = Hub::new();
        for id in ["focused", "inactive"] {
            hub.create_local_session(
                id.to_owned(),
                "codex".to_owned(),
                "/tmp".to_owned(),
                id.to_owned(),
                SessionOrigin::Web,
                false,
            );
            hub.push(
                id,
                Event::Update {
                    update: serde_json::json!({"sessionUpdate": "agent_message_chunk", "messageId": id, "content": {"type": "text", "text": id}}),
                },
            );
        }
        hub
    }

    #[test]
    fn websocket_bootstrap_contains_only_global_state() {
        let messages = connect_bootstrap(
            &hub_with_sessions(),
            true,
            &test_owner_principal(),
            Some(Outbound::Machines {
                revision: 7,
                machines: Vec::new(),
                resync: true,
            }),
        );
        assert!(
            messages
                .iter()
                .any(|message| matches!(message, Outbound::Sessions { .. }))
        );
        assert!(
            messages
                .iter()
                .any(|message| matches!(message, Outbound::BootstrapComplete))
        );
        assert!(messages.iter().any(|message| matches!(
            message,
            Outbound::Machines {
                revision: 7,
                resync: true,
                ..
            }
        )));
        let machines_index = messages
            .iter()
            .position(|message| matches!(message, Outbound::Machines { .. }))
            .expect("Machine snapshot");
        let complete_index = messages
            .iter()
            .position(|message| matches!(message, Outbound::BootstrapComplete))
            .expect("bootstrap completion");
        assert!(machines_index < complete_index);
        assert!(!messages.iter().any(|message| matches!(
            message,
            Outbound::Snapshot { .. } | Outbound::ConfigOptions { .. }
        )));
        assert!(!messages.iter().any(|message| matches!(
            message,
            Outbound::SyncPatch { state, .. } if state.starts_with("queue:")
        )));
    }

    #[test]
    fn legacy_websocket_bootstrap_remains_complete() {
        let messages =
            connect_bootstrap(&hub_with_sessions(), false, &test_owner_principal(), None);
        let snapshots = messages
            .iter()
            .filter(|message| matches!(message, Outbound::Snapshot { .. }))
            .count();
        assert_eq!(snapshots, 2);
        assert!(matches!(messages.last(), Some(Outbound::BootstrapComplete)));
    }

    #[test]
    fn focused_bootstrap_does_not_replay_another_session() {
        let messages = focused_session_bootstrap(&hub_with_sessions(), "focused")
            .expect("focused session bootstrap");
        let snapshots: Vec<_> = messages
            .iter()
            .filter_map(|message| match message {
                Outbound::Snapshot {
                    session_id, events, ..
                } => Some((session_id, events)),
                _ => None,
            })
            .collect();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].0, "focused");
        assert_eq!(snapshots[0].1.len(), 1);
        assert!(messages.iter().all(|message| match message {
            Outbound::Snapshot { session_id, .. } | Outbound::ConfigOptions { session_id, .. } =>
                session_id == "focused",
            Outbound::SyncPatch { state, .. } => state == "queue:focused",
            _ => true,
        }));
    }
}

#[cfg(test)]
mod provider_uninstall_tests {
    use super::provider_session_has_active_turn;
    use crate::agent_model::Status;

    #[test]
    fn only_an_in_flight_turn_requires_active_uninstall_confirmation() {
        assert!(provider_session_has_active_turn(Status::Busy));
        for idle_or_terminal in [
            Status::Starting,
            Status::Running,
            Status::Exited,
            Status::Crashed,
            Status::Interrupted,
        ] {
            assert!(!provider_session_has_active_turn(idle_or_terminal));
        }
    }
}

#[cfg(test)]
mod provider_install_tests {
    use super::provider_auth_sync_required_before_install;
    use crate::machine_protocol::{
        PluginInstallationState, PluginInventory, ProviderMaterializationState,
        ProviderReplicaState,
    };
    use crate::provider_service::{
        ProviderAuthenticationStatus, ServiceAuthenticationState, ServiceDistributionState,
    };

    fn authentication(auth_generation: u64) -> ProviderAuthenticationStatus {
        ProviderAuthenticationStatus {
            provider_id: "codex".to_owned(),
            auth_generation,
            authentication_state: ServiceAuthenticationState::Ready,
            distribution_state: ServiceDistributionState::Failed,
            auth_contract_fingerprint: "sha256:new-auth-contract".to_owned(),
            authentication_scope: "codex-auth-v1".to_owned(),
            portable_schema: "codex-auth-v1".to_owned(),
            projection_schema: "codex-home-v1".to_owned(),
            account_label: None,
            updated_at_ms: 1,
        }
    }

    fn installed(
        auth_generation: Option<u64>,
        replica_state: ProviderReplicaState,
    ) -> PluginInventory {
        PluginInventory {
            plugin_id: "codex".to_owned(),
            plugin_version: "1.1.1".to_owned(),
            plugin_kind: cowboy_plugin_sdk::PluginKind::AgentProvider,
            generation_digest: "sha256:old-provider".to_owned(),
            contract_fingerprint: "sha256:old-contract".to_owned(),
            state: PluginInstallationState::Active,
            rollback_generation_digest: None,
            active_session_leases: 0,
            auth_generation,
            replica_state,
            materialization_state: ProviderMaterializationState::Failed,
            detail: None,
        }
    }

    #[test]
    fn current_sealed_replica_allows_an_incompatible_provider_upgrade() {
        let authentication = authentication(2);
        let installed = installed(Some(2), ProviderReplicaState::Current);

        assert!(!provider_auth_sync_required_before_install(
            Some(&authentication),
            Some(&installed),
        ));
    }

    #[test]
    fn missing_or_stale_replica_still_requires_pre_install_sync() {
        let authentication = authentication(2);

        assert!(provider_auth_sync_required_before_install(
            Some(&authentication),
            None,
        ));
        assert!(provider_auth_sync_required_before_install(
            Some(&authentication),
            Some(&installed(Some(1), ProviderReplicaState::Current)),
        ));
        assert!(provider_auth_sync_required_before_install(
            Some(&authentication),
            Some(&installed(Some(2), ProviderReplicaState::Failed)),
        ));
    }

    #[test]
    fn signed_out_provider_does_not_require_auth_sync() {
        assert!(!provider_auth_sync_required_before_install(None, None));
    }
}

#[cfg(test)]
mod provider_auth_resume_tests {
    use super::{ProviderAuthExecutor, reconcile_provider_auth_executors};
    use std::collections::HashMap;

    fn executor(provider_id: &str, method: &str, expires_at_ms: i64) -> ProviderAuthExecutor {
        ProviderAuthExecutor {
            machine_id: "machine".to_owned(),
            provider_id: provider_id.to_owned(),
            provider_version: "1.0.0".to_owned(),
            generation_digest: "sha256:generation".to_owned(),
            auth_contract_fingerprint: "sha256:contract".to_owned(),
            auth_method: method.to_owned(),
            expected_generation: 0,
            promotion_started: false,
            expires_at_ms,
        }
    }

    #[test]
    fn page_reload_recovers_the_existing_provider_authentication() {
        let mut executors = HashMap::from([
            (
                "claude-request".to_owned(),
                executor("claude-code", "claude-account", 2_000),
            ),
            (
                "codex-request".to_owned(),
                executor("codex", "chatgpt-account", 3_000),
            ),
        ]);

        let reconciliation =
            reconcile_provider_auth_executors(&mut executors, "claude-code", 1_000);

        assert!(reconciliation.expired.is_empty());
        let (request_id, active) = reconciliation
            .active
            .as_ref()
            .expect("existing Claude authentication");
        assert_eq!(request_id, "claude-request");
        assert_eq!(active.auth_method, "claude-account");
        assert!(reconciliation.resumable(Some("claude-request")).is_some());
        assert!(reconciliation.resumable(None).is_none());
        assert!(reconciliation.resumable(Some("failed-request")).is_none());
        assert_eq!(executors.len(), 2);
    }

    #[test]
    fn expired_authentication_is_removed_instead_of_resumed() {
        let mut executors = HashMap::from([
            (
                "expired-claude".to_owned(),
                executor("claude-code", "claude-account", 999),
            ),
            (
                "current-codex".to_owned(),
                executor("codex", "chatgpt-account", 2_000),
            ),
        ]);

        let reconciliation =
            reconcile_provider_auth_executors(&mut executors, "claude-code", 1_000);

        assert_eq!(
            reconciliation.expired,
            vec![("claude-code".to_owned(), "expired-claude".to_owned())]
        );
        assert!(reconciliation.active.is_none());
        assert_eq!(executors.len(), 1);
        assert!(executors.contains_key("current-codex"));
    }
}

#[cfg(test)]
mod product_auth_api_tests {
    use super::*;
    use crate::admin::{
        ADMIN_IDENTITIES_SETTING, AdminCredentials, AdminIdentities, REGISTRATION_SETTING,
        RegistrationMode, RegistrationPolicy,
    };
    use crate::product_auth::USER_SESSION_COOKIE;
    use crate::store::Store;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use ed25519_dalek::SigningKey;
    use std::os::unix::fs::PermissionsExt as _;
    use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

    fn auth_state(hub: Hub, store: Option<Store>) -> ProductAuthState {
        let data_dir = std::env::temp_dir().join(format!(
            "cowboy-admin-setup-state-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&data_dir).unwrap();
        let setup = Arc::new(crate::admin::AdminSetupState::new(data_dir));
        let mut identities = AdminIdentities::from_setting(
            hub.settings_snapshot()
                .get(crate::admin::ADMIN_IDENTITIES_SETTING),
        );
        if let Ok(Some(_)) =
            crate::admin::ensure_admin_setup_token(&setup.data_dir, &mut identities, true)
        {
            hub.set_setting(
                ADMIN_IDENTITIES_SETTING.to_owned(),
                serde_json::to_value(&identities).unwrap(),
            );
        }
        ProductAuthState {
            hub,
            store,
            rate_limits: Arc::new(crate::product_auth::AuthRateLimiter::default()),
            public_origins: Arc::new(Vec::new()),
            runtime_health: None,
            persistence_health: None,
            runtime_router: None,
            plugin_catalog: None,
            provider_catalog: None,
            passkeys: Arc::new(crate::passkey::PasskeyCeremonies::default()),
            setup,
            setup_lock: Arc::new(tokio::sync::Mutex::new(())),
            product_auth_enabled: true,
            product_authentication: Arc::new(
                crate::auth_plugins::ProductAuthentication::test_default(None),
            ),
            oidc_transactions: Arc::new(crate::oidc::OidcTransactions::default()),
            oidc_native_handoffs: Arc::new(crate::oidc::NativeHandoffs::default()),
            device_authorizations: Arc::new(crate::client_auth::DeviceAuthorizations::default()),
            device_access: Arc::new(crate::client_auth::DeviceAccessSessions::default()),
        }
    }

    #[test]
    fn auth_off_does_not_read_oidc_secret_files() {
        let missing = std::env::temp_dir().join(format!(
            "cowboy-missing-oidc-config-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        assert!(load_oidc_provider(false, Some(&missing)).unwrap().is_none());
        assert!(load_oidc_provider(true, Some(&missing)).is_err());
    }

    async fn test_store() -> (Store, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "cowboy-product-auth-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let url = format!("sqlite://{}", root.join("cowboy.sqlite3").display());
        let store = Store::connect(&url, root.join("artifacts")).await.unwrap();
        store.migrate().await.unwrap();
        (store, root)
    }

    fn test_oidc_provider(
        root: &std::path::Path,
        product_account: &str,
        admin_account: Option<&str>,
    ) -> Arc<crate::oidc::OidcProvider> {
        let client_key = SigningKey::from_bytes(&[7_u8; 32]);
        let id_token_key = SigningKey::from_bytes(&[9_u8; 32]);
        let private_key_path = root.join("oidc-client-private.jwk");
        std::fs::write(
            &private_key_path,
            serde_json::to_vec(&serde_json::json!({
                "kty": "OKP",
                "crv": "Ed25519",
                "d": URL_SAFE_NO_PAD.encode(client_key.to_bytes()),
                "x": URL_SAFE_NO_PAD.encode(client_key.verifying_key().as_bytes()),
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(&private_key_path, std::fs::Permissions::from_mode(0o600))
            .unwrap();
        let config_path = root.join("oidc.json");
        std::fs::write(
            &config_path,
            serde_json::to_vec(&serde_json::json!({
                "schema": "dravengarden.cowboy.cardea-oidc/v1",
                "display_name": "Cardea",
                "issuer": "https://cardea.example",
                "client_id": "cowboy-test",
                "client_key_id": "primary",
                "client_private_key_file": private_key_path,
                "id_token_key_id": "issuer-primary",
                "id_token_public_key_jwk": {
                    "kty": "OKP",
                    "crv": "Ed25519",
                    "x": URL_SAFE_NO_PAD.encode(id_token_key.verifying_key().as_bytes()),
                },
                "subject": "draven",
                "account": product_account,
                "admin_account": admin_account,
                "redirect_uri": "https://cowboy.example/api/auth/oidc/callback",
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        Arc::new(crate::oidc::OidcProvider::load(&config_path).unwrap())
    }

    fn install_rustls() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    async fn spawn_auth(state: ProductAuthState) -> (String, tokio::task::JoinHandle<()>) {
        install_rustls();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(
                listener,
                product_auth_router(state).into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    fn origin_for(base: &str) -> String {
        base.trim_end_matches('/').to_owned()
    }

    async fn post_json(
        url: &str,
        origin: &str,
        cookie: Option<&str>,
        body: serde_json::Value,
    ) -> reqwest::Response {
        let mut request = reqwest::Client::new()
            .post(url)
            .header(header::ORIGIN, origin)
            .json(&body);
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        request.send().await.unwrap()
    }

    async fn put_json(
        url: &str,
        origin: &str,
        cookie: Option<&str>,
        body: serde_json::Value,
    ) -> reqwest::Response {
        let mut request = reqwest::Client::new()
            .put(url)
            .header(header::ORIGIN, origin)
            .json(&body);
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        request.send().await.unwrap()
    }

    fn set_cookie(response: &reqwest::Response, name: &str) -> Option<String> {
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find(|value| value.starts_with(&format!("{name}=")))
            .map(ToOwned::to_owned)
    }

    fn cookie_header(set_cookie: &str) -> String {
        set_cookie
            .split(';')
            .next()
            .unwrap_or(set_cookie)
            .trim()
            .to_owned()
    }

    #[allow(dead_code)]
    fn enable_open(hub: &Hub) {
        hub.set_setting(
            REGISTRATION_SETTING.to_owned(),
            serde_json::to_value(RegistrationPolicy {
                enabled: true,
                mode: RegistrationMode::Open,
                tokens: Vec::new(),
            })
            .unwrap(),
        );
    }

    fn seed_admin(hub: &Hub) -> String {
        let mut identities = AdminIdentities::default();
        let token = identities
            .bootstrap(
                &AdminCredentials {
                    account: "owner".to_owned(),
                    password: "Correct-horse-bat1".to_owned(),
                },
                1_900_000_000_000,
            )
            .unwrap();
        hub.set_setting(
            ADMIN_IDENTITIES_SETTING.to_owned(),
            serde_json::to_value(&identities).unwrap(),
        );
        format!("cowboy_admin={token}")
    }

    async fn prove_setup(base: &str, origin: &str, token: &str) -> String {
        let prepared = post_json(
            &format!("{base}/api/auth/setup"),
            origin,
            None,
            serde_json::json!({ "token": token }),
        )
        .await;
        assert_eq!(prepared.status(), StatusCode::OK);
        cookie_header(
            &set_cookie(&prepared, crate::admin::ADMIN_SETUP_COOKIE).expect("setup cookie"),
        )
    }

    fn setup_token_from(state: &ProductAuthState) -> String {
        std::fs::read_to_string(state.setup.token_path())
            .unwrap()
            .trim()
            .to_owned()
    }

    async fn spawn_enforcement(state: ProductAuthState) -> (String, tokio::task::JoinHandle<()>) {
        install_rustls();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let app = Router::new()
                .route("/ws", any(test_ws_upgrade))
                .route("/metrics", get(test_metrics))
                .with_state(state);
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn browser_approved_device_rotates_sender_constrained_credentials() {
        let (store, root) = test_store().await;
        let now = auth_now_ms();
        let user = crate::store::ProductUser {
            id: "a".repeat(32),
            username: "draven".to_owned(),
            password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
            password_hash: crate::product_auth::hash_password("Correct-horse-bat1").unwrap(),
            created_at_ms: now,
            updated_at_ms: now,
            disabled_at_ms: None,
        };
        store.insert_user(&user).await.unwrap();
        let cookie_token = crate::product_auth::new_session_token().unwrap();
        store
            .insert_user_session(&crate::store::ProductUserSession {
                token_hash: crate::admin::hex_sha256(cookie_token.as_bytes()),
                user_id: user.id.clone(),
                created_at_ms: now,
                expires_at_ms: now
                    + crate::auth_plugins::SessionServerPolicy::default().primary_max_age_ms,
                last_seen_at_ms: now,
                user_agent: Some("device-authorization-test".to_owned()),
                passkey_verified_at_ms: None,
                primary_authenticated_at_ms: now,
                primary_auth_method: Some(crate::auth_plugins::PASSWORD_LOGIN_METHOD.to_owned()),
            })
            .await
            .unwrap();
        let cookie = format!("{USER_SESSION_COOKIE}={cookie_token}");
        let state = auth_state(Hub::new(), Some(store.clone()));
        let device_access = state.device_access.clone();
        let (base, server) = spawn_auth(state).await;
        let origin = origin_for(&base);

        let signing_key = crate::client_auth::new_signing_key().unwrap();
        let code_verifier = crate::client_auth::new_code_verifier().unwrap();
        let started = reqwest::Client::new()
            .post(format!("{base}/api/auth/device/authorizations"))
            .json(&crate::client_auth::StartAuthorizationRequest {
                name: "Zed on Hawk".to_owned(),
                public_key: crate::client_auth::public_key_to_base64(&signing_key),
                code_challenge: crate::client_auth::code_challenge(&code_verifier).unwrap(),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(started.status(), StatusCode::CREATED);
        assert_eq!(
            started.headers()[header::CACHE_CONTROL],
            header::HeaderValue::from_static("no-store")
        );
        let started = started
            .json::<crate::client_auth::StartAuthorizationResponse>()
            .await
            .unwrap();
        let verification = url::Url::parse(&format!("{base}{}", started.verification_url)).unwrap();
        assert_eq!(verification.path(), "/auth/device");
        let capability = url::form_urlencoded::parse(
            verification
                .fragment()
                .expect("authorization fragment")
                .as_bytes(),
        )
        .into_owned()
        .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(capability.get("request_id"), Some(&started.request_id));
        let approval_token = capability
            .get("approval_token")
            .expect("approval token")
            .clone();
        let browser_request = serde_json::json!({
            "request_id": started.request_id,
            "approval_token": approval_token,
        });

        let inspected = reqwest::Client::new()
            .post(format!("{base}/api/auth/device/authorizations/inspect"))
            .json(&browser_request)
            .send()
            .await
            .unwrap();
        assert_eq!(inspected.status(), StatusCode::OK);
        let inspected: serde_json::Value = inspected.json().await.unwrap();
        assert_eq!(inspected["name"], "Zed on Hawk");
        assert_eq!(inspected["status"], "pending");
        assert!(
            inspected["fingerprint"]
                .as_str()
                .unwrap()
                .starts_with("SHA256:")
        );

        let socket_url = format!(
            "{}/api/auth/device/authorizations/events",
            base.replacen("http://", "ws://", 1)
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(socket_url).await.unwrap();
        socket
            .send(TungsteniteMessage::Text(
                serde_json::to_string(&crate::client_auth::AuthorizationEventsHandshake {
                    request_id: started.request_id.clone(),
                    code_verifier: code_verifier.clone(),
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();

        let missing_login = post_json(
            &format!("{base}/api/auth/device/authorizations/approve"),
            &origin,
            None,
            browser_request.clone(),
        )
        .await;
        assert_eq!(missing_login.status(), StatusCode::UNAUTHORIZED);
        let approved = post_json(
            &format!("{base}/api/auth/device/authorizations/approve"),
            &origin,
            Some(&cookie),
            browser_request,
        )
        .await;
        assert_eq!(approved.status(), StatusCode::OK);
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .expect("device approval event timed out")
            .expect("device approval socket closed")
            .expect("device approval event failed");
        let TungsteniteMessage::Text(event) = event else {
            panic!("expected a device authorization text event");
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&event).unwrap()["status"],
            "approved"
        );

        let exchange = crate::client_auth::signed_exchange_request(
            &signing_key,
            started.request_id.clone(),
            code_verifier,
            auth_now_ms(),
        )
        .unwrap();
        let exchanged = reqwest::Client::new()
            .post(format!("{base}/api/auth/device/exchange"))
            .json(&exchange)
            .send()
            .await
            .unwrap();
        assert_eq!(exchanged.status(), StatusCode::OK);
        let tokens = exchanged
            .json::<crate::client_auth::DeviceTokenResponse>()
            .await
            .unwrap();
        assert!(
            tokens
                .access_token
                .starts_with(crate::client_auth::ACCESS_TOKEN_PREFIX)
        );
        assert!(
            tokens
                .refresh_token
                .starts_with(crate::client_auth::REFRESH_TOKEN_PREFIX)
        );

        let mut proof_headers = HeaderMap::new();
        proof_headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", tokens.access_token).parse().unwrap(),
        );
        for (name, value) in crate::client_auth::signed_proof_headers(
            &signing_key,
            &tokens.device_id,
            &tokens.access_token,
            "GET",
            "/api/sessions",
            auth_now_ms(),
        )
        .unwrap()
        {
            proof_headers.insert(
                header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        let identity = device_access
            .authenticate(&proof_headers, &Method::GET, "/api/sessions", auth_now_ms())
            .unwrap()
            .expect("device access identity");
        assert_eq!(identity.user_id, user.id);
        assert!(
            device_access
                .authenticate(&proof_headers, &Method::GET, "/api/sessions", auth_now_ms(),)
                .is_err()
        );

        let listed = reqwest::Client::new()
            .get(format!("{base}/api/auth/devices"))
            .header(header::COOKIE, &cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        assert_eq!(
            listed.json::<serde_json::Value>().await.unwrap()["devices"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let refresh_path = "/api/auth/device/refresh";
        let mut refresh_request = reqwest::Client::new()
            .post(format!("{base}{refresh_path}"))
            .bearer_auth(&tokens.refresh_token);
        for (name, value) in crate::client_auth::signed_proof_headers(
            &signing_key,
            &tokens.device_id,
            &tokens.refresh_token,
            "POST",
            refresh_path,
            auth_now_ms(),
        )
        .unwrap()
        {
            refresh_request = refresh_request.header(name, value);
        }
        let refreshed = refresh_request.send().await.unwrap();
        assert_eq!(refreshed.status(), StatusCode::OK);
        let refreshed = refreshed
            .json::<crate::client_auth::DeviceTokenResponse>()
            .await
            .unwrap();
        assert_ne!(refreshed.refresh_token, tokens.refresh_token);

        let unsigned_replay = reqwest::Client::new()
            .post(format!("{base}{refresh_path}"))
            .bearer_auth(&tokens.refresh_token)
            .send()
            .await
            .unwrap();
        assert_eq!(unsigned_replay.status(), StatusCode::UNAUTHORIZED);
        assert!(device_access.token_still_valid(&refreshed.access_token, &identity, auth_now_ms()));

        let mut replay = reqwest::Client::new()
            .post(format!("{base}{refresh_path}"))
            .bearer_auth(&tokens.refresh_token);
        for (name, value) in crate::client_auth::signed_proof_headers(
            &signing_key,
            &tokens.device_id,
            &tokens.refresh_token,
            "POST",
            refresh_path,
            auth_now_ms(),
        )
        .unwrap()
        {
            replay = replay.header(name, value);
        }
        assert_eq!(
            replay.send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        assert!(!device_access.token_still_valid(
            &refreshed.access_token,
            &identity,
            auth_now_ms()
        ));
        assert!(
            store
                .list_user_devices_for_user(&user.id)
                .await
                .unwrap()
                .is_empty()
        );

        let replayed_exchange = reqwest::Client::new()
            .post(format!("{base}/api/auth/device/exchange"))
            .json(&exchange)
            .send()
            .await
            .unwrap();
        assert_eq!(replayed_exchange.status(), StatusCode::GONE);

        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    async fn test_ws_upgrade(
        ws: WebSocketUpgrade,
        State(state): State<ProductAuthState>,
        ConnectInfo(peer): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
    ) -> Response {
        let principal = resolve_product_principal(
            state.product_auth_enabled,
            state.store.as_ref(),
            &state.hub,
            &headers,
        )
        .await;
        match authorize_ws_upgrade(&headers, peer, &state.public_origins, principal) {
            Ok(_) => {
                drop(ws);
                StatusCode::SWITCHING_PROTOCOLS.into_response()
            }
            Err(status) => status.into_response(),
        }
    }

    async fn test_metrics(
        ConnectInfo(peer): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
    ) -> Response {
        if crate::product_auth::metrics_scrape_allowed(peer, &headers) {
            StatusCode::OK.into_response()
        } else {
            StatusCode::NOT_FOUND.into_response()
        }
    }

    #[test]
    fn classify_route_table_matches_capability_matrix() {
        assert_eq!(classify_route(&Method::GET, "/healthz"), RouteAuth::Public);
        assert_eq!(classify_route(&Method::GET, "/ws"), RouteAuth::Product);
        assert_eq!(
            classify_route(&Method::GET, "/metrics"),
            RouteAuth::MetricsScrape
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/sessions"),
            RouteAuth::ProductOperator
        );
        for path in [
            "/api/plugins/codex/auth",
            "/api/plugins/codex/auth/start",
            "/api/plugins/codex/auth/request-1",
            "/api/providers/codex/auth",
            "/api/providers/codex/auth/start",
            "/api/providers/codex/auth/request-1",
        ] {
            assert_eq!(
                classify_route(&Method::POST, path),
                RouteAuth::ProductOperator,
                "Provider authentication route {path} must accept a product operator",
            );
        }
        assert_eq!(
            classify_route(&Method::POST, "/api/plugins/catalog/refresh"),
            RouteAuth::AdminOperator,
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/sessions/abc/info"),
            RouteAuth::ProductSessionSee
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/sessions/abc/prompt"),
            RouteAuth::ProductSessionMutate
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/artifacts/hash"),
            RouteAuth::Product
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/machines/enrollment"),
            RouteAuth::ProductOrAdminOperator
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/machine/service"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::DELETE, "/api/machines/enrollment"),
            RouteAuth::ProductOrAdminOperator
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/machines/m-123/revoke"),
            RouteAuth::ProductOrAdminOperator
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/machines/m-123/refresh"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/machines/m-123/deployment-health"),
            RouteAuth::Public
        );
        for path in [
            "/api/metrics",
            "/api/logs",
            "/api/logs/log-123",
            "/api/observability/incidents",
        ] {
            assert_eq!(
                classify_route(&Method::GET, path),
                RouteAuth::ProductOrAdminOperator,
                "observability route {path} must accept product and legacy admin operators",
            );
        }
        assert_eq!(
            classify_route(&Method::GET, "/api/auth/tokens"),
            RouteAuth::Product
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/auth/tokens"),
            RouteAuth::ProductOperator
        );
        assert_eq!(
            classify_route(&Method::DELETE, "/api/auth/tokens/abc"),
            RouteAuth::Product
        );
        for path in [
            "/api/auth/device/authorizations",
            "/api/auth/device/authorizations/inspect",
            "/api/auth/device/exchange",
            "/api/auth/device/refresh",
        ] {
            assert_eq!(classify_route(&Method::POST, path), RouteAuth::Public);
        }
        assert_eq!(
            classify_route(&Method::GET, "/api/auth/device/authorizations/events"),
            RouteAuth::Public
        );
        for path in [
            "/api/auth/device/authorizations/approve",
            "/api/auth/device/authorizations/deny",
        ] {
            assert_eq!(classify_route(&Method::POST, path), RouteAuth::Product);
        }
        assert_eq!(
            classify_route(&Method::GET, "/api/auth/devices"),
            RouteAuth::Product
        );
        assert_eq!(
            classify_route(&Method::DELETE, "/api/auth/devices/device-1"),
            RouteAuth::Product
        );
        for path in [
            "/api/auth/passkeys/external/options",
            "/api/auth/passkeys/external/complete",
            "/api/auth/passkeys/external/fail",
        ] {
            assert_eq!(classify_route(&Method::POST, path), RouteAuth::Public);
        }
        for path in [
            "/api/auth/passkeys/external/start",
            "/api/auth/passkeys/external/finalize",
        ] {
            assert_eq!(classify_route(&Method::POST, path), RouteAuth::Product);
        }
        assert_eq!(
            classify_route(&Method::GET, "/api/auth/passkeys/external/events"),
            RouteAuth::Product
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/auth/setup"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/auth/oidc/start"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/auth/oidc/callback"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/auth/oidc/native/exchange"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/auth/oidc/native/poll"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/auth/oidc/native/events"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/auth/oidc/native/cancel"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/auth/oidc/native/complete"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/auth/providers/cardea/native/poll"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/auth/providers/cardea/native/events"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/auth/providers/cardea/native/cancel"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/admin/auth"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/admin/auth/setup"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/admin/auth/bootstrap"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/admin/auth/login"),
            RouteAuth::Public
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/admin/overview"),
            RouteAuth::AdminViewer
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/admin/accounts"),
            RouteAuth::AdminViewer
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/admin/accounts"),
            RouteAuth::AdminOwner
        );
        assert_eq!(
            classify_route(&Method::PUT, "/api/admin/registration"),
            RouteAuth::AdminOperator
        );
        assert_eq!(
            classify_route(&Method::PUT, "/api/admin/permissions"),
            RouteAuth::AdminOwner
        );
        assert!(!matches!(
            classify_route(&Method::GET, "/api/unknown"),
            RouteAuth::Public | RouteAuth::Product
        ));
    }

    #[test]
    fn device_authorization_prefers_the_configured_public_browser_origin() {
        assert_eq!(
            device_verification_url(
                &["https://cowboy.example".to_owned()],
                "request",
                "approval",
            ),
            "https://cowboy.example/auth/device#request_id=request&approval_token=approval",
        );
        assert_eq!(
            device_verification_url(&[], "request", "approval"),
            "/auth/device#request_id=request&approval_token=approval",
        );
    }

    #[tokio::test]
    async fn legacy_browser_session_binds_once_to_its_next_primary_method() {
        let (store, root) = test_store().await;
        let now = auth_now_ms();
        let user = crate::store::ProductUser {
            id: "a".repeat(32),
            username: "owner".to_owned(),
            password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
            password_hash: crate::product_auth::hash_password("Correct-horse-bat1").unwrap(),
            created_at_ms: now,
            updated_at_ms: now,
            disabled_at_ms: None,
        };
        store.insert_user(&user).await.unwrap();
        let legacy_token = "b".repeat(64);
        let legacy_hash = crate::admin::hex_sha256(legacy_token.as_bytes());
        store
            .insert_user_session(&crate::store::ProductUserSession {
                token_hash: legacy_hash.clone(),
                user_id: user.id.clone(),
                created_at_ms: now,
                expires_at_ms: now
                    + crate::auth_plugins::SessionServerPolicy::default().primary_max_age_ms,
                last_seen_at_ms: now,
                user_agent: Some("legacy-method-test".to_owned()),
                passkey_verified_at_ms: None,
                primary_authenticated_at_ms: now,
                primary_auth_method: None,
            })
            .await
            .unwrap();
        let state = auth_state(Hub::new(), Some(store.clone()));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("{USER_SESSION_COOKIE}={legacy_token}")
                .parse()
                .unwrap(),
        );

        let bound = issue_product_session(
            &state,
            &store,
            &user,
            &headers,
            crate::auth_plugins::PASSWORD_LOGIN_METHOD,
        )
        .await;
        assert_eq!(bound.status(), StatusCode::OK);
        assert!(
            store
                .user_session_by_token_hash(&legacy_hash)
                .await
                .unwrap()
                .is_none()
        );
        let replacement_cookie = bound
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find(|value| value.starts_with(&format!("{USER_SESSION_COOKIE}=")))
            .expect("replacement product cookie")
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let replacement_token = replacement_cookie
            .strip_prefix(&format!("{USER_SESSION_COOKIE}="))
            .unwrap();
        assert_eq!(
            store
                .user_session_by_token_hash(&crate::admin::hex_sha256(replacement_token.as_bytes()))
                .await
                .unwrap()
                .expect("bound replacement session")
                .primary_auth_method
                .as_deref(),
            Some("password")
        );

        headers.insert(header::COOKIE, replacement_cookie.parse().unwrap());
        let switched = issue_product_session(&state, &store, &user, &headers, "cardea").await;
        assert_eq!(switched.status(), StatusCode::CONFLICT);

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn native_oidc_handoff_requires_origin_pkce_and_sets_both_account_cookies() {
        install_rustls();
        let (store, root) = test_store().await;
        let now = auth_now_ms();
        let user = crate::store::ProductUser {
            id: "a".repeat(32),
            username: "owner".to_owned(),
            password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
            password_hash: crate::product_auth::hash_password("Correct-horse-bat1").unwrap(),
            created_at_ms: now,
            updated_at_ms: now,
            disabled_at_ms: None,
        };
        store.insert_user(&user).await.unwrap();
        let hub = Hub::new();
        let _ = seed_admin(&hub);
        let mut state = auth_state(hub, Some(store));
        state.product_authentication =
            Arc::new(crate::auth_plugins::ProductAuthentication::test_default(
                Some(test_oidc_provider(&root, "owner", Some("owner"))),
            ));
        let verifier = "v".repeat(64);
        let challenge = crate::oidc::pkce_challenge(&verifier).unwrap();
        let handoff = state
            .oidc_native_handoffs
            .issue("cardea", &user.id, &challenge)
            .unwrap();
        let callback = url::Url::parse(&handoff.location).unwrap();
        let code = callback
            .query_pairs()
            .find(|(name, _)| name == "code")
            .map(|(_, value)| value.into_owned())
            .unwrap();
        let (base, handle) = spawn_auth(state).await;
        let body = serde_json::json!({
            "code": code,
            "code_verifier": verifier,
        });

        let missing_origin = reqwest::Client::new()
            .post(format!("{base}/api/auth/oidc/native/exchange"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(missing_origin.status(), StatusCode::FORBIDDEN);

        let exchanged = post_json(
            &format!("{base}/api/auth/oidc/native/exchange"),
            &origin_for(&base),
            None,
            body.clone(),
        )
        .await;
        assert_eq!(exchanged.status(), StatusCode::OK);
        let user_cookie = set_cookie(&exchanged, crate::product_auth::USER_SESSION_COOKIE)
            .expect("product cookie");
        let admin_cookie =
            set_cookie(&exchanged, crate::admin::ADMIN_SESSION_COOKIE).expect("admin cookie");
        assert!(user_cookie.contains("Max-Age=2592000"));
        assert!(admin_cookie.contains("Max-Age=43200"));

        let replayed = post_json(
            &format!("{base}/api/auth/oidc/native/exchange"),
            &origin_for(&base),
            None,
            body,
        )
        .await;
        assert_eq!(replayed.status(), StatusCode::UNAUTHORIZED);

        let cookies = format!(
            "{}; {}",
            cookie_header(&user_cookie),
            cookie_header(&admin_cookie)
        );
        let product_status: serde_json::Value = reqwest::Client::new()
            .get(format!("{base}/api/auth/status"))
            .header(header::COOKIE, &cookies)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let admin_status: serde_json::Value = reqwest::Client::new()
            .get(format!("{base}/api/admin/auth"))
            .header(header::COOKIE, &cookies)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(product_status["me"]["account"], "owner");
        assert_eq!(product_status["me"]["primary_auth_method"], "cardea");
        assert_eq!(
            product_status["login_method_order"],
            serde_json::json!(["cardea", "password"])
        );
        assert_eq!(admin_status["authenticated"], true);

        let cardea_cookie = cookie_header(&user_cookie);
        let switched_without_sign_out = post_json(
            &format!("{base}/api/auth/login"),
            &origin_for(&base),
            Some(&cardea_cookie),
            serde_json::json!({
                "account": "owner",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(switched_without_sign_out.status(), StatusCode::CONFLICT);
        assert!(
            switched_without_sign_out
                .text()
                .await
                .unwrap()
                .contains("Sign out before switching")
        );

        let signed_out = post_json(
            &format!("{base}/api/auth/logout"),
            &origin_for(&base),
            Some(&cardea_cookie),
            serde_json::json!({}),
        )
        .await;
        assert!(signed_out.status().is_success());
        let password_login = post_json(
            &format!("{base}/api/auth/login"),
            &origin_for(&base),
            None,
            serde_json::json!({
                "account": "owner",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(password_login.status(), StatusCode::OK);
        let password_body: serde_json::Value = password_login.json().await.unwrap();
        assert_eq!(password_body["primary_auth_method"], "password");

        handle.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn browser_shell_oidc_poll_is_origin_bound_pending_and_single_use() {
        install_rustls();
        let (store, root) = test_store().await;
        let now = auth_now_ms();
        let user = crate::store::ProductUser {
            id: "a".repeat(32),
            username: "owner".to_owned(),
            password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
            password_hash: crate::product_auth::hash_password("Correct-horse-bat1").unwrap(),
            created_at_ms: now,
            updated_at_ms: now,
            disabled_at_ms: None,
        };
        store.insert_user(&user).await.unwrap();
        let hub = Hub::new();
        let _ = seed_admin(&hub);
        let mut state = auth_state(hub, Some(store));
        state.product_authentication =
            Arc::new(crate::auth_plugins::ProductAuthentication::test_default(
                Some(test_oidc_provider(&root, "owner", Some("owner"))),
            ));
        let verifier = "v".repeat(64);
        let handoff_token = "h".repeat(64);
        let code_challenge = crate::oidc::pkce_challenge(&verifier).unwrap();
        let handoff_challenge = crate::oidc::pkce_challenge(&handoff_token).unwrap();
        let handoffs = state.oidc_native_handoffs.clone();
        let (base, handle) = spawn_auth(state).await;
        let mut start_url = url::Url::parse(&format!("{base}/api/auth/oidc/start")).unwrap();
        start_url.query_pairs_mut().extend_pairs([
            ("client", "browser-shell"),
            ("code_challenge", code_challenge.as_str()),
            ("handoff_challenge", handoff_challenge.as_str()),
        ]);
        let start = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .get(start_url)
            .send()
            .await
            .unwrap();
        assert_eq!(start.status(), StatusCode::SEE_OTHER);
        assert!(
            start
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|location| location.starts_with("https://cardea.example/"))
        );
        assert!(
            set_cookie(&start, crate::oidc::TRANSACTION_COOKIE)
                .is_some_and(|cookie| cookie.contains("HttpOnly"))
        );
        let body = serde_json::json!({
            "handoff_token": handoff_token,
            "code_verifier": verifier,
        });

        let missing_origin = reqwest::Client::new()
            .post(format!("{base}/api/auth/oidc/native/poll"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(missing_origin.status(), StatusCode::FORBIDDEN);

        let pending = post_json(
            &format!("{base}/api/auth/oidc/native/poll"),
            &origin_for(&base),
            None,
            body.clone(),
        )
        .await;
        assert_eq!(pending.status(), StatusCode::ACCEPTED);
        assert_eq!(
            pending.json::<serde_json::Value>().await.unwrap()["status"],
            "pending"
        );

        handoffs
            .complete_browser("cardea", &handoff_challenge, &user.id)
            .unwrap();
        let exchanged = post_json(
            &format!("{base}/api/auth/providers/cardea/native/poll"),
            &origin_for(&base),
            None,
            body.clone(),
        )
        .await;
        assert_eq!(exchanged.status(), StatusCode::OK);
        let user_cookie = set_cookie(&exchanged, crate::product_auth::USER_SESSION_COOKIE)
            .expect("product cookie");
        let admin_cookie =
            set_cookie(&exchanged, crate::admin::ADMIN_SESSION_COOKIE).expect("admin cookie");
        assert!(user_cookie.contains("Max-Age=2592000"));
        assert!(admin_cookie.contains("Max-Age=43200"));

        let replayed = post_json(
            &format!("{base}/api/auth/oidc/native/poll"),
            &origin_for(&base),
            None,
            body,
        )
        .await;
        assert_eq!(replayed.status(), StatusCode::UNAUTHORIZED);

        let cancelled_verifier = "c".repeat(64);
        let cancelled_handoff_token = "x".repeat(64);
        handoffs
            .begin_browser(
                "cardea",
                &crate::oidc::pkce_challenge(&cancelled_verifier).unwrap(),
                &crate::oidc::pkce_challenge(&cancelled_handoff_token).unwrap(),
            )
            .unwrap();
        let cancelled_body = serde_json::json!({
            "handoff_token": cancelled_handoff_token,
            "code_verifier": cancelled_verifier,
        });
        let cancelled = post_json(
            &format!("{base}/api/auth/providers/cardea/native/cancel"),
            &origin_for(&base),
            None,
            cancelled_body.clone(),
        )
        .await;
        assert_eq!(cancelled.status(), StatusCode::NO_CONTENT);
        let cancelled_poll = post_json(
            &format!("{base}/api/auth/oidc/native/poll"),
            &origin_for(&base),
            None,
            cancelled_body,
        )
        .await;
        assert_eq!(cancelled_poll.status(), StatusCode::UNAUTHORIZED);

        let denied_verifier = "d".repeat(64);
        let denied_handoff_token = "n".repeat(64);
        let denied_code_challenge = crate::oidc::pkce_challenge(&denied_verifier).unwrap();
        let denied_handoff_challenge = crate::oidc::pkce_challenge(&denied_handoff_token).unwrap();
        let mut denied_start_url = url::Url::parse(&format!("{base}/api/auth/oidc/start")).unwrap();
        denied_start_url.query_pairs_mut().extend_pairs([
            ("client", "browser-shell"),
            ("code_challenge", denied_code_challenge.as_str()),
            ("handoff_challenge", denied_handoff_challenge.as_str()),
        ]);
        let denied_start = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .get(denied_start_url)
            .send()
            .await
            .unwrap();
        assert_eq!(denied_start.status(), StatusCode::SEE_OTHER);
        let denied_transaction_cookie = cookie_header(
            &set_cookie(&denied_start, crate::oidc::TRANSACTION_COOKIE)
                .expect("OIDC transaction cookie"),
        );
        let denied_state = url::Url::parse(
            denied_start
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .expect("authorization location"),
        )
        .unwrap()
        .query_pairs()
        .find(|(name, _)| name == "state")
        .map(|(_, value)| value.into_owned())
        .expect("OIDC state");
        let denied_callback = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .get(format!(
                "{base}/api/auth/oidc/callback?state={denied_state}&error=access_denied"
            ))
            .header(header::COOKIE, denied_transaction_cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(denied_callback.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            denied_callback
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some("/api/auth/oidc/native/complete")
        );
        let denied_poll = post_json(
            &format!("{base}/api/auth/oidc/native/poll"),
            &origin_for(&base),
            None,
            serde_json::json!({
                "handoff_token": denied_handoff_token,
                "code_verifier": denied_verifier,
            }),
        )
        .await;
        assert_eq!(denied_poll.status(), StatusCode::GONE);

        let completion = reqwest::Client::new()
            .get(format!("{base}/api/auth/oidc/native/complete"))
            .send()
            .await
            .unwrap();
        assert_eq!(completion.status(), StatusCode::OK);
        assert_eq!(
            completion
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
        assert!(completion.headers().contains_key("content-security-policy"));

        handle.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn browser_shell_oidc_events_are_origin_bound_and_do_not_consume_the_handoff() {
        install_rustls();
        let (store, root) = test_store().await;
        let mut state = auth_state(Hub::new(), Some(store));
        state.product_authentication =
            Arc::new(crate::auth_plugins::ProductAuthentication::test_default(
                Some(test_oidc_provider(&root, "owner", Some("owner"))),
            ));
        let verifier = "v".repeat(64);
        let handoff_token = "h".repeat(64);
        let handoff_challenge = crate::oidc::pkce_challenge(&handoff_token).unwrap();
        state
            .oidc_native_handoffs
            .begin_browser(
                "cardea",
                &crate::oidc::pkce_challenge(&verifier).unwrap(),
                &handoff_challenge,
            )
            .unwrap();
        let handoffs = state.oidc_native_handoffs.clone();
        let (base, handle) = spawn_auth(state).await;
        let socket_url = format!(
            "{}/api/auth/oidc/native/events",
            base.replacen("http://", "ws://", 1)
        );

        let rejected = tokio_tungstenite::connect_async(&socket_url)
            .await
            .expect_err("a native handoff socket without Origin must fail");
        assert!(matches!(
            rejected,
            tokio_tungstenite::tungstenite::Error::Http(response)
                if response.status() == StatusCode::FORBIDDEN
        ));

        let mut request = socket_url.into_client_request().unwrap();
        request
            .headers_mut()
            .insert(header::ORIGIN, origin_for(&base).parse().unwrap());
        let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        socket
            .send(TungsteniteMessage::Text(
                serde_json::json!({
                    "handoff_token": handoff_token,
                    "code_verifier": verifier,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        handoffs
            .complete_browser("cardea", &handoff_challenge, &"a".repeat(32))
            .unwrap();
        let message = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .expect("native handoff event timed out")
            .expect("native handoff socket closed")
            .expect("native handoff event failed");
        let TungsteniteMessage::Text(message) = message else {
            panic!("expected a native handoff text event");
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&message).unwrap(),
            serde_json::json!({ "status": "ready" })
        );
        assert!(matches!(
            handoffs
                .poll_browser("cardea", &"h".repeat(64), &"v".repeat(64))
                .unwrap(),
            crate::oidc::BrowserHandoffPoll::Ready { .. }
        ));

        let _ = socket.close(None).await;
        handle.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn product_machine_list_includes_enrolled_local_and_excludes_unenrolled_records() {
        let machine = |connection_mode: &str, fingerprint: Option<&str>, revoked: bool| {
            crate::store::MachineRecord {
                id: "machine".to_owned(),
                display_name: "Machine".to_owned(),
                connection_mode: connection_mode.to_owned(),
                platform: "linux".to_owned(),
                architecture: "x86_64".to_owned(),
                status: "offline".to_owned(),
                inventory: serde_json::json!({}),
                last_seen_at_ms: None,
                revoked,
                fingerprint: fingerprint.map(str::to_owned),
            }
        };
        assert!(product_machine_is_visible(
            &machine("outbound_wss", Some("SHA256:key"), false),
            true,
        ));
        assert!(product_machine_is_visible(
            &machine("local", Some("SHA256:key"), false),
            true,
        ));
        assert!(!product_machine_is_visible(
            &machine("local", None, false),
            true,
        ));
        assert!(!product_machine_is_visible(
            &machine("outbound_wss", None, false,),
            true
        ));
        assert!(!product_machine_is_visible(
            &machine("outbound_wss", Some("SHA256:key"), true),
            true,
        ));
        assert!(product_machine_is_visible(
            &machine("local", None, false),
            false,
        ));
        assert!(!product_machine_is_visible(
            &machine("local", None, true),
            false,
        ));
    }

    #[test]
    fn authorize_ws_upgrade_requires_a_principal() {
        let headers = HeaderMap::new();
        let err = authorize_ws_upgrade(
            &headers,
            SocketAddr::from(([127, 0, 0, 1], 3333)),
            &[],
            None,
        )
        .unwrap_err();
        assert_eq!(err, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn disabled_product_auth_allows_anonymous_ws_as_local_owner() {
        let headers = HeaderMap::new();
        let principal = resolve_product_principal(false, None, &Hub::new(), &headers)
            .await
            .expect("disabled product auth should create a local principal");
        assert!(product_session_owner(false, &principal).is_none());
        let principal = authorize_ws_upgrade(
            &headers,
            SocketAddr::from(([127, 0, 0, 1], 3333)),
            &[],
            Some(principal),
        )
        .expect("local principal should be accepted");
        assert_eq!(principal.user_id, "local");
        assert_eq!(principal.role, crate::admin::AdminRole::Owner);
    }

    #[test]
    fn enabled_product_auth_stamps_the_durable_user_as_session_owner() {
        let principal = ProductPrincipal {
            user_id: "0123456789abcdef0123456789abcdef".to_owned(),
            username: "draven".to_owned(),
            role: crate::admin::AdminRole::Owner,
        };
        let owner = product_session_owner(true, &principal).expect("session owner");
        assert_eq!(owner.user_id, principal.user_id);
        assert_eq!(owner.username, Some(principal.username.as_str()));
    }

    #[tokio::test]
    async fn unauthenticated_ws_is_rejected_before_upgrade() {
        let (store, root) = test_store().await;
        let (base, server) = spawn_enforcement(auth_state(Hub::new(), Some(store))).await;
        let response = reqwest::Client::new()
            .get(format!("{base}/ws"))
            .header(header::CONNECTION, "Upgrade")
            .header(header::UPGRADE, "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn metrics_with_forwarded_for_is_rejected() {
        let (store, root) = test_store().await;
        let (base, server) = spawn_enforcement(auth_state(Hub::new(), Some(store))).await;
        let rejected = reqwest::Client::new()
            .get(format!("{base}/metrics"))
            .header("x-forwarded-for", "1.2.3.4")
            .send()
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::NOT_FOUND);
        let allowed = reqwest::Client::new()
            .get(format!("{base}/metrics"))
            .send()
            .await
            .unwrap();
        assert_eq!(allowed.status(), StatusCode::OK);
        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn default_policy_rejects_register() {
        let (store, root) = test_store().await;
        let (base, server) = spawn_auth(auth_state(Hub::new(), Some(store))).await;
        let origin = origin_for(&base);
        let response = post_json(
            &format!("{base}/api/auth/register"),
            &origin,
            None,
            serde_json::json!({
                "account": "draven",
                "password": "long-enough-password",
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.text().await.unwrap(), "setup token required");
        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn in_memory_store_returns_503() {
        let (base, server) = spawn_auth(auth_state(Hub::new(), None)).await;
        let origin = origin_for(&base);
        let response = post_json(
            &format!("{base}/api/auth/register"),
            &origin,
            None,
            serde_json::json!({
                "account": "draven",
                "password": "long-enough-password",
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let login = post_json(
            &format!("{base}/api/auth/login"),
            &origin,
            None,
            serde_json::json!({
                "account": "draven",
                "password": "long-enough-password",
            }),
        )
        .await;
        assert_eq!(login.status(), StatusCode::SERVICE_UNAVAILABLE);
        server.abort();
    }

    #[tokio::test]
    async fn open_mode_ignores_token_and_login_sets_cookie() {
        let (store, root) = test_store().await;
        let hub = Hub::new();
        let state = auth_state(hub, Some(store));
        let token = setup_token_from(&state);
        let (base, server) = spawn_auth(state).await;
        let origin = origin_for(&base);
        let setup_cookie = prove_setup(&base, &origin, &token).await;
        let registered = post_json(
            &format!("{base}/api/auth/register"),
            &origin,
            Some(&setup_cookie),
            serde_json::json!({
                "account": "Draven",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(registered.status(), StatusCode::OK);
        let cookie = set_cookie(&registered, USER_SESSION_COOKIE).expect("register cookie");
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        let body: serde_json::Value = registered.json().await.unwrap();
        assert_eq!(body["account"], "draven");
        assert_eq!(body["role"], "owner");

        let _ = post_json(
            &format!("{base}/api/auth/logout"),
            &origin,
            Some(&cookie_header(&cookie)),
            serde_json::json!({}),
        )
        .await;

        let logged_in = post_json(
            &format!("{base}/api/auth/login"),
            &origin,
            None,
            serde_json::json!({
                "account": "draven",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(logged_in.status(), StatusCode::OK);
        let login_cookie = set_cookie(&logged_in, USER_SESSION_COOKIE).expect("login cookie");
        assert!(login_cookie.starts_with(&format!("{USER_SESSION_COOKIE}=")));
        assert!(login_cookie.contains("Max-Age=2592000"));

        let me = reqwest::Client::new()
            .get(format!("{base}/api/auth/me"))
            .header(header::COOKIE, cookie_header(&login_cookie))
            .send()
            .await
            .unwrap();
        assert_eq!(me.status(), StatusCode::OK);
        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn operator_can_create_list_and_revoke_own_api_tokens() {
        let (store, root) = test_store().await;
        let state = auth_state(Hub::new(), Some(store));
        let token = setup_token_from(&state);
        let (base, server) = spawn_auth(state).await;
        let origin = origin_for(&base);
        let setup_cookie = prove_setup(&base, &origin, &token).await;
        let created = post_json(
            &format!("{base}/api/auth/register"),
            &origin,
            Some(&setup_cookie),
            serde_json::json!({
                "account": "draven",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(created.status(), StatusCode::OK);

        let logged_in = post_json(
            &format!("{base}/api/auth/login"),
            &origin,
            None,
            serde_json::json!({
                "account": "draven",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        let cookie = cookie_header(&set_cookie(&logged_in, USER_SESSION_COOKIE).unwrap());

        let minted = post_json(
            &format!("{base}/api/auth/tokens"),
            &origin,
            Some(&cookie),
            serde_json::json!({ "name": "zed" }),
        )
        .await;
        assert_eq!(minted.status(), StatusCode::CREATED);
        let minted_body: serde_json::Value = minted.json().await.unwrap();
        let secret = minted_body["token"].as_str().unwrap().to_owned();
        let token_id = minted_body["id"].as_str().unwrap().to_owned();
        assert!(secret.starts_with("cow_"));
        assert_eq!(minted_body["token_prefix"].as_str().unwrap(), &secret[..8]);
        assert!(minted_body.get("token_hash").is_none());

        let listed = reqwest::Client::new()
            .get(format!("{base}/api/auth/tokens"))
            .header(header::COOKIE, &cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        let listed_body: serde_json::Value = listed.json().await.unwrap();
        assert_eq!(listed_body["tokens"][0]["id"], token_id);
        assert!(listed_body["tokens"][0].get("token").is_none());
        assert!(listed_body["tokens"][0].get("token_hash").is_none());

        let me = reqwest::Client::new()
            .get(format!("{base}/api/auth/me"))
            .header(header::AUTHORIZATION, format!("Bearer {secret}"))
            .send()
            .await
            .unwrap();
        assert_eq!(me.status(), StatusCode::OK);
        let me_body: serde_json::Value = me.json().await.unwrap();
        assert_eq!(me_body["account"], "draven");
        assert_eq!(me_body["role"], "owner");

        let bearer_create = reqwest::Client::new()
            .post(format!("{base}/api/auth/tokens"))
            .header(header::AUTHORIZATION, format!("Bearer {secret}"))
            .header(header::ORIGIN, "https://evil.example")
            .json(&serde_json::json!({ "name": "curl" }))
            .send()
            .await
            .unwrap();
        assert_eq!(bearer_create.status(), StatusCode::CREATED);

        let revoked = reqwest::Client::new()
            .delete(format!("{base}/api/auth/tokens/{token_id}"))
            .header(header::COOKIE, &cookie)
            .header(header::ORIGIN, &origin)
            .send()
            .await
            .unwrap();
        assert_eq!(revoked.status(), StatusCode::OK);
        let rejected = reqwest::Client::new()
            .get(format!("{base}/api/auth/me"))
            .header(header::AUTHORIZATION, format!("Bearer {secret}"))
            .send()
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn admin_created_user_is_operator() {
        let (store, root) = test_store().await;
        let hub = Hub::new();
        let admin = seed_admin(&hub);
        let (base, server) = spawn_auth(auth_state(hub.clone(), Some(store))).await;
        let origin = origin_for(&base);
        let created = post_json(
            &format!("{base}/api/admin/users"),
            &origin,
            Some(&admin),
            serde_json::json!({
                "account": "draven",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(created.status(), StatusCode::FORBIDDEN);
        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn concurrent_single_use_token_keeps_one_user() {
        let (store, root) = test_store().await;
        let state = auth_state(Hub::new(), Some(store.clone()));
        let token = setup_token_from(&state);
        let (base, server) = spawn_auth(state).await;
        let origin = origin_for(&base);
        let setup_cookie = prove_setup(&base, &origin, &token).await;
        let register_url = format!("{base}/api/auth/register");
        let first = post_json(
            &register_url,
            &origin,
            Some(&setup_cookie),
            serde_json::json!({
                "account": "alice",
                "password": "Correct-horse-bat1",
            }),
        );
        let second = post_json(
            &register_url,
            &origin,
            Some(&setup_cookie),
            serde_json::json!({
                "account": "bob",
                "password": "Correct-horse-bat1",
            }),
        );
        let (first, second) = tokio::join!(first, second);
        let statuses = [first.status(), second.status()];
        assert!(statuses.contains(&StatusCode::OK));
        assert!(
            statuses.contains(&StatusCode::FORBIDDEN) || statuses.contains(&StatusCode::CONFLICT)
        );
        let users = store.list_users().await.unwrap();
        assert_eq!(users.len(), 1);
        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn admin_bootstrap_creates_owner_then_product_user_can_login() {
        let (store, root) = test_store().await;
        let state = auth_state(Hub::new(), Some(store));
        let setup_token = std::fs::read_to_string(state.setup.token_path())
            .unwrap()
            .trim()
            .to_owned();
        assert!(setup_token.starts_with("cow_setup_"));
        let (base, server) = spawn_auth(state).await;
        let origin = origin_for(&base);

        let status = reqwest::Client::new()
            .get(format!("{base}/api/auth/status"))
            .send()
            .await
            .unwrap();
        assert_eq!(status.status(), StatusCode::OK);
        let body: serde_json::Value = status.json().await.unwrap();
        assert_eq!(body["setup_required"], true);
        assert_eq!(body["setup_pending"], false);
        assert_eq!(body["login_method_order"], serde_json::json!(["password"]));
        assert_eq!(body["session"]["activity_sliding_enabled"], true);
        assert_eq!(body["session"]["idle_timeout_ms"], 86_400_000_i64);
        assert_eq!(body["session"]["primary_max_age_ms"], 2_592_000_000_i64);
        assert_eq!(body["session"]["primary_warning_ms"], 86_400_000_i64);
        assert_eq!(body["registration"]["accepts_registration"], false);
        assert!(body.get("setup_token").is_none());
        assert!(!body.to_string().contains(&setup_token));

        let skipped = post_json(
            &format!("{base}/api/auth/register"),
            &origin,
            None,
            serde_json::json!({
                "account": "draven",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(skipped.status(), StatusCode::FORBIDDEN);

        let rejected = post_json(
            &format!("{base}/api/auth/setup"),
            &origin,
            None,
            serde_json::json!({ "token": "cow_setup_nope" }),
        )
        .await;
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

        let prepared = post_json(
            &format!("{base}/api/auth/setup"),
            &origin,
            None,
            serde_json::json!({ "token": setup_token }),
        )
        .await;
        assert_eq!(prepared.status(), StatusCode::OK);
        let setup_set_cookie =
            set_cookie(&prepared, crate::admin::ADMIN_SETUP_COOKIE).expect("setup cookie");
        assert!(setup_set_cookie.contains("SameSite=Strict"));
        assert!(setup_set_cookie.contains("HttpOnly"));
        let setup_cookie = cookie_header(&setup_set_cookie);
        let prepared_body: serde_json::Value = prepared.json().await.unwrap();
        assert_eq!(prepared_body["setup_pending"], true);
        assert!(!prepared_body.to_string().contains(&setup_token));

        let created = post_json(
            &format!("{base}/api/auth/register"),
            &origin,
            Some(&setup_cookie),
            serde_json::json!({
                "account": "draven",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(created.status(), StatusCode::OK);

        let logged_in = post_json(
            &format!("{base}/api/auth/login"),
            &origin,
            None,
            serde_json::json!({
                "account": "draven",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(logged_in.status(), StatusCode::OK);
        let user_set_cookie = set_cookie(&logged_in, USER_SESSION_COOKIE).unwrap();
        assert!(user_set_cookie.contains("Max-Age=2592000"));

        let first_cookie = cookie_header(&user_set_cookie);
        let reauthenticated = post_json(
            &format!("{base}/api/auth/login"),
            &origin,
            Some(&first_cookie),
            serde_json::json!({
                "account": "draven",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(reauthenticated.status(), StatusCode::OK);
        let replacement_set_cookie =
            set_cookie(&reauthenticated, USER_SESSION_COOKIE).expect("replacement user cookie");
        let replacement_cookie = cookie_header(&replacement_set_cookie);

        let stale = reqwest::Client::new()
            .get(format!("{base}/api/auth/me"))
            .header(header::COOKIE, first_cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);

        let me = reqwest::Client::new()
            .get(format!("{base}/api/auth/me"))
            .header(header::COOKIE, replacement_cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(me.status(), StatusCode::OK);
        let me_body: serde_json::Value = me.json().await.unwrap();
        assert_eq!(me_body["account"], "draven");
        assert_eq!(me_body["role"], "owner");
        assert_eq!(me_body["primary_auth_method"], "password");

        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn disabled_product_auth_reports_a_local_owner_without_setup() {
        let (store, root) = test_store().await;
        let mut state = auth_state(Hub::new(), Some(store));
        state.product_auth_enabled = false;
        let (base, server) = spawn_auth(state).await;

        let status = reqwest::Client::new()
            .get(format!("{base}/api/auth/status"))
            .send()
            .await
            .unwrap();
        assert_eq!(status.status(), StatusCode::OK);
        let body: serde_json::Value = status.json().await.unwrap();
        assert_eq!(body["setup_required"], false);
        assert_eq!(body["setup_pending"], false);
        assert_eq!(body["login_method_order"], serde_json::json!([]));
        assert_eq!(body["me"]["account"], "local");
        assert_eq!(body["me"]["role"], "owner");
        assert_eq!(body["me"]["auth_enabled"], false);

        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn admin_login_rejects_forwarded_cleartext() {
        let (store, root) = test_store().await;
        let hub = Hub::new();
        let _admin = seed_admin(&hub);
        let (base, server) = spawn_auth(auth_state(hub, Some(store))).await;
        let origin = origin_for(&base);
        let rejected = reqwest::Client::new()
            .post(format!("{base}/api/admin/auth/login"))
            .header(header::ORIGIN, &origin)
            .header("x-forwarded-for", "203.0.113.9")
            .header("x-forwarded-proto", "http")
            .json(&serde_json::json!({
                "account": "owner",
                "password": "Correct-horse-bat1",
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
        assert_eq!(rejected.text().await.unwrap(), "admin login requires HTTPS");
        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn admin_passkey_lock_is_off_until_a_credential_exists() {
        let (store, root) = test_store().await;
        let hub = Hub::new();
        let admin = seed_admin(&hub);
        let (base, server) = spawn_auth(auth_state(hub, Some(store))).await;
        let origin = origin_for(&base);
        let listed = reqwest::Client::new()
            .get(format!("{base}/api/admin/passkeys"))
            .header(header::COOKIE, &admin)
            .send()
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        let listed_body: serde_json::Value = listed.json().await.unwrap();
        assert_eq!(listed_body["passkeys"].as_array().unwrap().len(), 0);

        let disabled = put_json(
            &format!("{base}/api/admin/passkeys/reauth"),
            &origin,
            Some(&admin),
            serde_json::json!({ "enabled": false }),
        )
        .await;
        assert_eq!(disabled.status(), StatusCode::OK);
        let body: serde_json::Value = disabled.json().await.unwrap();
        assert_eq!(body["passkey_reauth_enabled"], false);
        assert_eq!(body["passkey_reauth_required"], false);

        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn admin_overview_requires_cookie() {
        let (store, root) = test_store().await;
        let hub = Hub::new();
        let admin = seed_admin(&hub);
        let (base, server) = spawn_auth(auth_state(hub, Some(store))).await;

        let denied = reqwest::Client::new()
            .get(format!("{base}/api/admin/overview"))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

        let overview = reqwest::Client::new()
            .get(format!("{base}/api/admin/overview"))
            .header(header::COOKIE, &admin)
            .send()
            .await
            .unwrap();
        assert_eq!(overview.status(), StatusCode::OK);
        let body: serde_json::Value = overview.json().await.unwrap();
        assert_eq!(body["healthy"], true);
        assert_eq!(body["backend"], "store");
        assert_eq!(body["registration"]["accepts_registration"], false);

        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn extra_users_and_admin_first_run_fail_closed() {
        let (store, root) = test_store().await;
        let hub = Hub::new();
        let admin = seed_admin(&hub);
        let (base, server) = spawn_auth(auth_state(hub.clone(), Some(store))).await;
        let origin = origin_for(&base);

        let saved = put_json(
            &format!("{base}/api/admin/registration"),
            &origin,
            Some(&admin),
            serde_json::json!({
                "enabled": true,
                "mode": "token",
            }),
        )
        .await;
        assert_eq!(saved.status(), StatusCode::FORBIDDEN);
        assert_eq!(saved.text().await.unwrap(), "this instance is single-user");

        let extra_admin = post_json(
            &format!("{base}/api/admin/accounts"),
            &origin,
            Some(&admin),
            serde_json::json!({
                "account": "ops",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(extra_admin.status(), StatusCode::FORBIDDEN);

        let admin_setup = post_json(
            &format!("{base}/api/admin/auth/setup"),
            &origin,
            None,
            serde_json::json!({ "token": "cow_setup_nope" }),
        )
        .await;
        assert_eq!(admin_setup.status(), StatusCode::FORBIDDEN);
        assert_eq!(admin_setup.text().await.unwrap(), "complete setup on /");

        let admin_bootstrap = post_json(
            &format!("{base}/api/admin/auth/bootstrap"),
            &origin,
            None,
            serde_json::json!({
                "account": "ops",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(admin_bootstrap.status(), StatusCode::FORBIDDEN);

        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn passkey_viewing_lock_is_off_until_a_credential_exists() {
        let (store, root) = test_store().await;
        let state = auth_state(Hub::new(), Some(store));
        let token = setup_token_from(&state);
        let (base, server) = spawn_auth(state).await;
        let origin = origin_for(&base);
        let setup_cookie = prove_setup(&base, &origin, &token).await;
        let registered = post_json(
            &format!("{base}/api/auth/register"),
            &origin,
            Some(&setup_cookie),
            serde_json::json!({
                "account": "draven",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(registered.status(), StatusCode::OK);
        let cookie = cookie_header(&set_cookie(&registered, USER_SESSION_COOKIE).unwrap());
        let me: serde_json::Value = reqwest::Client::new()
            .get(format!("{base}/api/auth/me"))
            .header(header::COOKIE, &cookie)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(me["passkey_count"], 0);
        assert_eq!(me["passkey_reauth_required"], false);
        assert_eq!(me["passkey_reauth_enabled"], false);
        assert_eq!(
            me["passkey_reauth_after_ms"],
            crate::passkey::DEFAULT_PASSKEY_REAUTH_AFTER_MS
        );

        let listed = reqwest::Client::new()
            .get(format!("{base}/api/auth/passkeys"))
            .header(header::COOKIE, &cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        let listed_body: serde_json::Value = listed.json().await.unwrap();
        assert_eq!(listed_body["passkeys"].as_array().unwrap().len(), 0);

        let rejected_enable = put_json(
            &format!("{base}/api/auth/passkeys/reauth"),
            &origin,
            Some(&cookie),
            serde_json::json!({ "enabled": true }),
        )
        .await;
        assert_eq!(rejected_enable.status(), StatusCode::BAD_REQUEST);

        let disabled = put_json(
            &format!("{base}/api/auth/passkeys/reauth"),
            &origin,
            Some(&cookie),
            serde_json::json!({ "enabled": false }),
        )
        .await;
        assert_eq!(disabled.status(), StatusCode::OK);
        let disabled_body: serde_json::Value = disabled.json().await.unwrap();
        assert_eq!(disabled_body["passkey_reauth_enabled"], false);

        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn external_passkey_handoff_is_public_only_in_safari_and_bound_to_one_session() {
        use sha2::Digest as _;

        let (store, root) = test_store().await;
        let mut state = auth_state(Hub::new(), Some(store));
        state.public_origins = Arc::new(vec!["https://cowboy.example".to_owned()]);
        let token = setup_token_from(&state);
        let (base, server) = spawn_auth(state).await;
        let origin = "https://cowboy.example".to_owned();
        let setup_cookie = prove_setup(&base, &origin, &token).await;
        let registered = post_json(
            &format!("{base}/api/auth/register"),
            &origin,
            Some(&setup_cookie),
            serde_json::json!({
                "account": "draven",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(registered.status(), StatusCode::OK);
        let first_cookie = cookie_header(&set_cookie(&registered, USER_SESSION_COOKIE).unwrap());
        let second_login = post_json(
            &format!("{base}/api/auth/login"),
            &origin,
            None,
            serde_json::json!({
                "account": "draven",
                "password": "Correct-horse-bat1",
            }),
        )
        .await;
        assert_eq!(second_login.status(), StatusCode::OK);
        let second_cookie = cookie_header(&set_cookie(&second_login, USER_SESSION_COOKIE).unwrap());
        let verifier = "a".repeat(64);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let started = post_json(
            &format!("{base}/api/auth/passkeys/external/start"),
            &origin,
            Some(&first_cookie),
            serde_json::json!({
                "action": "register",
                "nickname": "iPhone",
                "code_challenge": challenge,
            }),
        )
        .await;
        let started_status = started.status();
        let started_text = started.text().await.unwrap();
        assert_eq!(started_status, StatusCode::OK, "{started_text}");
        let started_body: serde_json::Value = serde_json::from_str(&started_text).unwrap();
        let transaction_id = started_body["transaction_id"].as_str().unwrap();
        assert_eq!(transaction_id.len(), 64);
        assert_eq!(started_body["expires_in_seconds"], 120);

        let evil_options = post_json(
            &format!("{base}/api/auth/passkeys/external/options"),
            "https://evil.example",
            None,
            serde_json::json!({ "transaction_id": transaction_id }),
        )
        .await;
        assert_eq!(evil_options.status(), StatusCode::FORBIDDEN);
        let options = post_json(
            &format!("{base}/api/auth/passkeys/external/options"),
            &origin,
            None,
            serde_json::json!({ "transaction_id": transaction_id }),
        )
        .await;
        assert_eq!(options.status(), StatusCode::OK);
        let options_body: serde_json::Value = options.json().await.unwrap();
        assert_eq!(options_body["status"], "ready");
        assert_eq!(options_body["action"], "register");
        let public_key = options_body["publicKey"].as_object().unwrap();
        assert!(
            public_key
                .get("challenge")
                .is_some_and(|value| value.is_string())
        );
        assert!(public_key.get("rp").is_some_and(|value| value.is_object()));
        assert!(
            public_key
                .get("user")
                .is_some_and(|value| value.is_object())
        );
        assert!(public_key.get("publicKey").is_none());

        let wrong_session = post_json(
            &format!("{base}/api/auth/passkeys/external/finalize"),
            &origin,
            Some(&second_cookie),
            serde_json::json!({
                "transaction_id": transaction_id,
                "code_verifier": verifier,
            }),
        )
        .await;
        assert_eq!(wrong_session.status(), StatusCode::BAD_REQUEST);
        let pending = post_json(
            &format!("{base}/api/auth/passkeys/external/finalize"),
            &origin,
            Some(&first_cookie),
            serde_json::json!({
                "transaction_id": transaction_id,
                "code_verifier": verifier,
            }),
        )
        .await;
        assert_eq!(pending.status(), StatusCode::ACCEPTED);

        let socket_url = format!(
            "{}/api/auth/passkeys/external/events",
            base.replacen("http://", "ws://", 1)
        );
        let rejected = tokio_tungstenite::connect_async(&socket_url)
            .await
            .expect_err("an external Passkey socket without Origin must fail");
        assert!(matches!(
            rejected,
            tokio_tungstenite::tungstenite::Error::Http(response)
                if response.status() == StatusCode::FORBIDDEN
        ));
        let mut socket_request = socket_url.into_client_request().unwrap();
        socket_request
            .headers_mut()
            .insert(header::ORIGIN, origin.parse().unwrap());
        socket_request
            .headers_mut()
            .insert(header::COOKIE, first_cookie.parse().unwrap());
        let (mut socket, response) = tokio_tungstenite::connect_async(socket_request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        socket
            .send(TungsteniteMessage::Text(
                serde_json::json!({
                    "transaction_id": transaction_id,
                    "code_verifier": verifier,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();

        let cancelled = post_json(
            &format!("{base}/api/auth/passkeys/external/fail"),
            &origin,
            None,
            serde_json::json!({ "transaction_id": transaction_id }),
        )
        .await;
        assert_eq!(cancelled.status(), StatusCode::OK);
        let message = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .expect("external Passkey event timed out")
            .expect("external Passkey socket closed")
            .expect("external Passkey event failed");
        let TungsteniteMessage::Text(message) = message else {
            panic!("expected an external Passkey text event");
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&message).unwrap(),
            serde_json::json!({ "status": "failed" })
        );
        let finalized = post_json(
            &format!("{base}/api/auth/passkeys/external/finalize"),
            &origin,
            Some(&first_cookie),
            serde_json::json!({
                "transaction_id": transaction_id,
                "code_verifier": verifier,
            }),
        )
        .await;
        assert_eq!(finalized.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            finalized.text().await.unwrap(),
            "Passkey setup was cancelled"
        );
        let _ = socket.close(None).await;

        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn overdue_extended_session_is_rejected_server_side_but_base_login_can_recover() {
        let (store, root) = test_store().await;
        let now = auth_now_ms();
        let user = crate::store::ProductUser {
            id: "a".repeat(32),
            username: "draven".to_owned(),
            password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
            password_hash: crate::product_auth::hash_password("Correct-horse-bat1").unwrap(),
            created_at_ms: now,
            updated_at_ms: now,
            disabled_at_ms: None,
        };
        store.insert_user(&user).await.unwrap();
        store
            .insert_user_passkey(&crate::passkey::UserPasskey {
                id: "b".repeat(32),
                user_id: user.id.clone(),
                credential_id: "credential".to_owned(),
                nickname: "Test Passkey".to_owned(),
                passkey_json: "{}".to_owned(),
                created_at_ms: now,
                last_used_at_ms: Some(now),
            })
            .await
            .unwrap();
        store
            .set_user_passkey_reauth(
                &user.id,
                true,
                crate::passkey::DEFAULT_PASSKEY_REAUTH_AFTER_MS,
            )
            .await
            .unwrap();

        let extended_token = "c".repeat(64);
        let extended = crate::store::ProductUserSession {
            token_hash: crate::admin::hex_sha256(extended_token.as_bytes()),
            user_id: user.id.clone(),
            created_at_ms: now - 8 * 24 * 60 * 60 * 1_000,
            expires_at_ms: now + 22 * 24 * 60 * 60 * 1_000,
            last_seen_at_ms: now,
            user_agent: None,
            passkey_verified_at_ms: Some(now - 8 * 24 * 60 * 60 * 1_000),
            primary_authenticated_at_ms: now - 8 * 24 * 60 * 60 * 1_000,
            primary_auth_method: Some(crate::auth_plugins::PASSWORD_LOGIN_METHOD.to_owned()),
        };
        store.insert_user_session(&extended).await.unwrap();
        let base_token = "d".repeat(64);
        let base_session = crate::store::ProductUserSession {
            token_hash: crate::admin::hex_sha256(base_token.as_bytes()),
            created_at_ms: now,
            expires_at_ms: now
                + crate::auth_plugins::SessionServerPolicy::default().primary_max_age_ms,
            passkey_verified_at_ms: None,
            ..extended.clone()
        };
        store.insert_user_session(&base_session).await.unwrap();

        let (base, server) = spawn_auth(auth_state(Hub::new(), Some(store))).await;
        let extended_cookie = format!("{USER_SESSION_COOKIE}={extended_token}");
        let status = reqwest::Client::new()
            .get(format!("{base}/api/auth/me"))
            .header(header::COOKIE, &extended_cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(status.status(), StatusCode::OK);
        let status: serde_json::Value = status.json().await.unwrap();
        assert_eq!(status["passkey_reauth_required"], true);

        let blocked = reqwest::Client::new()
            .get(format!("{base}/api/auth/tokens"))
            .header(header::COOKIE, &extended_cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(blocked.status(), StatusCode::PRECONDITION_REQUIRED);

        let base_cookie = format!("{USER_SESSION_COOKIE}={base_token}");
        let recoverable = reqwest::Client::new()
            .get(format!("{base}/api/auth/tokens"))
            .header(header::COOKIE, base_cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(recoverable.status(), StatusCode::OK);

        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn session_deadlines_keep_passkey_and_primary_proofs_independent() {
        let policy = crate::passkey::PasskeyPolicy {
            enabled: true,
            reauth_after_ms: 7 * 24 * 60 * 60 * 1_000,
            last_step_up_at_ms: Some(99_000),
            passkey_count: 1,
        };
        let server = crate::auth_plugins::SessionServerPolicy::default();
        let mut session = crate::store::ProductUserSession {
            token_hash: "aa".repeat(32),
            user_id: "user-1".to_owned(),
            created_at_ms: 100_000,
            expires_at_ms: 100_000 + server.primary_max_age_ms + 10_000,
            last_seen_at_ms: 100_000,
            user_agent: None,
            passkey_verified_at_ms: None,
            primary_authenticated_at_ms: 100_000,
            primary_auth_method: Some(crate::auth_plugins::PASSWORD_LOGIN_METHOD.to_owned()),
        };

        let deadlines = product_session_deadlines(server, &policy, true, &session, 100_000);
        assert_eq!(deadlines.passkey_due_at_ms, None);
        session.passkey_verified_at_ms = Some(100_000);
        let deadlines = product_session_deadlines(server, &policy, true, &session, 100_000);
        assert_eq!(
            deadlines.passkey_due_at_ms,
            Some(100_000 + server.passkey_max_age_ms)
        );
        assert_eq!(
            deadlines.primary_due_at_ms,
            100_000 + server.primary_max_age_ms
        );
        let original = deadlines;
        session.last_seen_at_ms += 10_000;
        let active = product_session_deadlines(server, &policy, true, &session, 110_000);
        assert_eq!(
            active.idle_due_at_ms,
            original.idle_due_at_ms.map(|due| due + 10_000)
        );
        assert_eq!(active.passkey_due_at_ms, original.passkey_due_at_ms);
        assert_eq!(active.primary_due_at_ms, original.primary_due_at_ms);
        assert_eq!(
            product_session_deadlines(server, &policy, true, &session, active.primary_due_at_ms,)
                .required_kind,
            Some(ProductSessionReauthKind::Primary),
            "activity and Passkey proof cannot move the primary-login cap"
        );

        let disabled = crate::passkey::PasskeyPolicy {
            enabled: false,
            ..policy
        };
        assert_eq!(
            product_session_deadlines(server, &disabled, true, &session, 100_000).passkey_due_at_ms,
            None
        );
        assert_eq!(
            product_session_deadlines(server, &policy, false, &session, 100_000).passkey_due_at_ms,
            None,
            "a persisted user toggle cannot bypass the server feature flag"
        );
    }

    #[test]
    fn passkey_management_requires_a_recent_session_local_step_up() {
        let now = 1_000_000;
        let mut session = crate::store::ProductUserSession {
            token_hash: "aa".repeat(32),
            user_id: "user-1".to_owned(),
            created_at_ms: now - PASSKEY_MANAGEMENT_STEP_UP_MAX_AGE_MS + 1,
            expires_at_ms: now + 1_000,
            last_seen_at_ms: now,
            user_agent: None,
            passkey_verified_at_ms: None,
            primary_authenticated_at_ms: now - PASSKEY_MANAGEMENT_STEP_UP_MAX_AGE_MS + 1,
            primary_auth_method: Some(crate::auth_plugins::PASSWORD_LOGIN_METHOD.to_owned()),
        };
        assert!(product_session_has_recent_step_up(&session, now));
        session.created_at_ms = now - PASSKEY_MANAGEMENT_STEP_UP_MAX_AGE_MS;
        assert!(!product_session_has_recent_step_up(&session, now));
        session.passkey_verified_at_ms = Some(now - 1);
        assert!(product_session_has_recent_step_up(&session, now));
        session.passkey_verified_at_ms = Some(now + 60_001);
        assert!(!product_session_has_recent_step_up(&session, now));
    }
}
