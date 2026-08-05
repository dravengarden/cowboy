//! HTTP / WebSocket server (design §5).
//!
//! Every frontend is an **equal subscriber** to one WebSocket stream. On
//! browsers negotiate a lightweight global session index, then hydrate the
//! focused session over HTTP while the socket carries live events. Legacy and
//! ACP bridge clients retain the complete bootstrap unless they opt into lazy
//! mode, preserving wire compatibility.
//!
//! Browser clients still use the original LAN trust model. Machine connections
//! use one-time enrollment plus an OpenSSH Ed25519 challenge before WebSocket
//! protocol negotiation.

use std::collections::{BTreeMap, HashMap};
use std::io::Read as _;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use axum::Router;
use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Json, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post, put};
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
    DispatchReq, Envelope, Event, Hub, Inbound, JudgeReq, Outbound, PersistenceHealth,
    RestoredSession, SessionOrigin, Status, StoreSink, StoreWrite,
};
use crate::diff_snapshot::{DiffSnapshotCache, DiffSnapshotKey};
use crate::machine_control::MachineControl;
use crate::observability::{Observability, SubmitReceipt, TelemetryBatch};
use crate::persistence::EventReducer;
use crate::remote_runtime::{RemoteBootstrap, RemoteRuntime};
use crate::runtime::RuntimeHealth;
use crate::runtime_router::RuntimeRouter;
use crate::store::Store;
use crate::supervisor::Supervisor;
use crate::usage::UsageService;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, watch};

struct AppState {
    hub: Hub,
    supervisor: Arc<Supervisor>,
    /// Kept for read-only storage metrics (`/api/metrics`). `None` in-memory.
    store: Option<Store>,
    persistence_health: Option<Arc<PersistenceHealth>>,
    shutdown: watch::Receiver<bool>,
    runtime_health: Arc<RuntimeHealth>,
    runtime_router: Arc<RuntimeRouter>,
    machine_control: Arc<MachineControl>,
    desired_machine_components: Arc<Vec<crate::machine_protocol::DesiredComponent>>,
    web_root: PathBuf,
    usage: UsageService,
    diff_snapshots: DiffSnapshotCache,
    code_cache: crate::code_cache::CodeCache,
    zed_adapter_socket: Option<PathBuf>,
    observability: Observability,
}

const STORE_QUEUE_CAPACITY: usize = 8_192;
const FORCE_CANCEL_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
const MACHINE_RECONNECT_GRACE_SECONDS: i32 = 15;
const RUNTIME_RECONCILIATION_GRACE: std::time::Duration = std::time::Duration::from_secs(15);
const MACHINE_RECONNECT_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

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
    init_tracing();
    let code_cache =
        crate::code_cache::CodeCache::open(args.data_dir.join("code-cache"), args.code_cache_bytes)
            .map_err(anyhow::Error::msg)
            .context("opening code content cache")?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    // Persistence closes only after dispatcher/runtime quiescence. Using the
    // HTTP shutdown signal directly would race a last-millisecond prompt
    // requeue against the writer closing its receiver.
    let (store_shutdown_tx, store_shutdown_rx) = watch::channel(false);
    let runtime_health = Arc::new(RuntimeHealth::default());
    // Phase 2: when --postgres-url is supplied, hook in the persistent store.
    // Migrations run on every start (sqlx tracks applied versions, so it's
    // idempotent); the in-memory Hub is then warmed from the DB before WS
    // clients can connect. Without --postgres-url the daemon falls back to
    // pure in-memory mode — same behaviour as before, useful for dev or for
    // running on a host that doesn't have the cowboy-private postgres yet.
    let (hub, store, persistence_health, writer_task, purge_task, session_id_floor) =
        if let Some(url) = args.postgres_url.as_deref() {
            let store = Store::connect(url, args.data_dir.join("artifacts"))
                .await
                .context("connecting postgres")?;
            store.migrate().await.context("running migrations")?;
            let session_id_floor = store
                .next_session_number()
                .await
                .context("seeding session id counter")?;
            let (tx, rx) = mpsc::channel::<StoreWrite>(STORE_QUEUE_CAPACITY);
            let health = Arc::new(PersistenceHealth::default());
            let hub = Hub::with_store(Some(StoreSink::new(tx, Arc::clone(&health))));
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
                    judge_runs: ls.judge_runs,
                    mobile_review_state: ls.mobile_review_state,
                })
                .collect();
            let restored_count = restored.len();
            // Seed the global settings (auto-resume default + continuation template)
            // BEFORE restore, so restore can compute each session's effective
            // auto-resume (override ?? default) and enqueue a continuation for the
            // opted-in interrupted ones.
            match store.load_settings().await {
                Ok(entries) => hub.load_settings(entries),
                Err(e) => tracing::warn!(error = %e, "loading settings (degrading to defaults)"),
            }
            // Detached workers outlive this control-plane process. Keep
            // persisted Busy turns guarded until their Machine runtime has had
            // one bounded reconnect window to prove ownership; only then may a
            // missing worker become Interrupted.
            hub.restore_reconciling_runtime(restored);
            tracing::info!(
                postgres = url,
                restored = restored_count,
                "persistence wired",
            );
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
            tracing::info!("no --postgres-url: running in-memory only");
            (Hub::new(), None, None, None, None, 1)
        };
    let usage = UsageService::new(args.codex_command.clone(), store.clone());
    let initial_usage = usage.clone();
    let mut usage_shutdown = shutdown_rx.clone();
    tokio::spawn(async move {
        loop {
            initial_usage.refresh().await;
            tokio::select! {
                _ = tokio::time::sleep(crate::usage::AUTO_REFRESH_INTERVAL) => {}
                changed = usage_shutdown.changed() => {
                    if changed.is_err() || *usage_shutdown.borrow() { break; }
                }
            }
        }
    });
    let machine_presence_task = store.as_ref().map(|store| {
        let store = store.clone();
        let shutdown = shutdown_rx.clone();
        tokio::spawn(run_machine_presence_sweeper(store, shutdown))
    });
    let runtime_router = RuntimeRouter::new();
    // Reset credits belong to the Codex account, not a session. Restore one
    // shared provider-level timer and keep it independent from session queues.
    if let Some(store) = store.as_ref() {
        match store.load_codex_reset().await {
            Ok(Some(action)) => {
                usage
                    .set_reset_schedule(Some(crate::usage::CodexResetSchedule {
                        fire_at_ms: action.fire_at_ms,
                    }))
                    .await;
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "loading scheduled Codex reset"),
        }
    }
    let reset_task = {
        let usage = usage.clone();
        let store = store.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            loop {
                let action = match store.as_ref() {
                    Some(store) => store.load_codex_reset().await.ok().flatten(),
                    None => None,
                };
                if let Some(action) = action.filter(|item| item.next_attempt_at_ms <= now_ms()) {
                    let key = action.idempotency_key;
                    if let Some(store) = store.as_ref() {
                        let _ = store
                            .append_provider_action_log(
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
                    match usage.consume_nearest_reset(&key, None).await {
                        Ok(result) => {
                            tracing::info!(outcome = %result.outcome, "scheduled Codex reset finished");
                            if let Some(store) = store.as_ref() {
                                let _ = store
                                    .append_provider_action_log(
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
                            if let Some(store) = store.as_ref()
                                && let Err(error) = store.delete_codex_reset().await
                            {
                                tracing::warn!(%error, "clearing scheduled Codex reset");
                            }
                            usage.set_reset_schedule(None).await;
                        }
                        Err(error) => {
                            if scheduled_reset_failure_policy(
                                error.call_may_have_reached_provider,
                                action.attempt_count,
                            ) == ScheduledResetFailurePolicy::StopUnknown
                            {
                                tracing::error!(%error, "scheduled Codex reset outcome unknown; automatic retry disabled");
                                if let Some(store) = store.as_ref() {
                                    let _ = store
                                        .append_provider_action_log(
                                            "scheduled",
                                            "unknown",
                                            "consume",
                                            &error.to_string(),
                                            error.credit_id.as_deref(),
                                            Some(&key),
                                            now_ms(),
                                        )
                                        .await;
                                    let _ = store.delete_codex_reset().await;
                                }
                                usage.set_reset_schedule(None).await;
                            } else if scheduled_reset_failure_policy(false, action.attempt_count)
                                == ScheduledResetFailurePolicy::RetryPreflight
                            {
                                tracing::warn!(%error, "scheduled Codex reset preflight failed; retrying safely");
                                if let Some(store) = store.as_ref() {
                                    let _ = store
                                        .append_provider_action_log(
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
                                        .defer_codex_reset(now_ms().saturating_add(60_000))
                                        .await;
                                }
                            } else {
                                tracing::error!(%error, "scheduled Codex reset preflight retry limit reached");
                                if let Some(store) = store.as_ref() {
                                    let _ = store
                                        .append_provider_action_log(
                                            "scheduled",
                                            "failed",
                                            "preflight",
                                            &error.to_string(),
                                            error.credit_id.as_deref(),
                                            Some(&key),
                                            now_ms(),
                                        )
                                        .await;
                                    let _ = store.delete_codex_reset().await;
                                }
                                usage.set_reset_schedule(None).await;
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
    runtime_health.set_dispatcher(true);
    let dispatcher_health = Arc::clone(&runtime_health);
    let dispatcher_hub = hub.clone();
    let dispatcher_supervisor = Arc::clone(&supervisor);
    let dispatcher_shutdown = shutdown_rx.clone();
    let dispatcher_exit_state = dispatcher_shutdown.clone();
    let mut dispatcher_task = tokio::spawn(async move {
        run_dispatcher(
            dispatcher_hub,
            dispatcher_supervisor,
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

    // One bounded queue feeds one long-lived Codex app-server process; every
    // judgment uses a fresh ephemeral Luna thread. The worker starts/calibrates
    // lazily, so daemon readiness never waits on an external model call.
    let (judge_tx, judge_rx) = mpsc::channel::<JudgeReq>(256);
    hub.set_judge_tx(judge_tx);
    let judge_hub = hub.clone();
    let judge_command = args.codex_command.clone();
    let judge_task = tokio::spawn(async move {
        crate::core::run_judge_worker(judge_hub, judge_rx, judge_command).await;
        tracing::error!("Codex judge worker exited unexpectedly");
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

    // Resume interruptions already finalized before this process. Newly restored
    // Busy turns are deliberately absent here until runtime reconciliation ends.
    for meta in hub.session_list() {
        revive_auto_resume_session(&hub, &supervisor, &meta.id);
    }

    // Machine WebSockets can only reconnect after Axum starts listening. Keep
    // this timer independent of the request task: real worker snapshots remove
    // their sessions from the reconciliation set, while the remainder become
    // genuine interruptions after the same bounded grace used for Machine
    // presence. Auto-resume starts only after that decision, never before it.
    let runtime_reconciliation_task = {
        let hub = hub.clone();
        let supervisor = Arc::clone(&supervisor);
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
                    for session_id in interrupted {
                        revive_auto_resume_session(&hub, &supervisor, &session_id);
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
        AppState {
            hub,
            supervisor,
            store,
            persistence_health,
            shutdown: shutdown_rx,
            runtime_health,
            runtime_router,
            machine_control: Arc::new(MachineControl::default()),
            desired_machine_components: Arc::new(desired_machine_components),
            web_root: args.web_root,
            usage,
            diff_snapshots: DiffSnapshotCache::default(),
            code_cache,
            zed_adapter_socket: args.zed_adapter_socket,
            observability,
        },
        shutdown_tx,
    )
    .await;
    judge_task.abort();
    scheduler_task.abort();
    reset_task.abort();
    runtime_reconciliation_task.abort();
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

async fn run_machine_presence_sweeper(store: Store, mut shutdown: watch::Receiver<bool>) {
    loop {
        match store.expire_machine_reconnects().await {
            Ok(expired) if expired > 0 => {
                tracing::warn!(expired, "Machine reconnect grace expired");
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
        let count = batch.len();
        if !apply_store_batch(&store, &mut reducer, batch).await {
            health.mark_failed_batch();
        }
        health.consumed(count);
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
        let summary = detail.as_deref().unwrap_or(if *status == Status::Crashed {
            "Runtime crashed without a diagnostic detail"
        } else {
            "Runtime was interrupted"
        });
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(format!("{classification}:{summary}").as_bytes())
        );
        let incident_id = format!("lifecycle:{}:{}", envelope.session_id, envelope.seq);
        store
            .upsert_runtime_incident(&crate::store::RuntimeIncidentWrite {
                id: incident_id.clone(),
                occurred_at_ms,
                source: "controller".to_owned(),
                classification: classification.to_owned(),
                severity: if *status == Status::Crashed {
                    "error".to_owned()
                } else {
                    "warning".to_owned()
                },
                state: "active".to_owned(),
                summary: summary.to_owned(),
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
                    "detail": detail,
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
    let detail = detail.unwrap_or_default().to_ascii_lowercase();
    if detail.contains("oom") || detail.contains("out of memory") || detail.contains("signal: 9") {
        "resource_exhaustion"
    } else if detail.contains("protocol") || detail.contains("frame") || detail.contains("json-rpc")
    {
        "protocol_failure"
    } else if detail.contains("connection")
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
    use super::{classify_crash_detail, classify_interruption_detail};

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
        assert_eq!(classify_crash_detail(None), "runtime_failure");
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
        StoreWrite::UpdateVerdict {
            session_id,
            awaiting_user,
            done,
        } => {
            store
                .update_verdict(session_id, *awaiting_user, *done)
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
        StoreWrite::ClearEvents { session_id } => store.clear_events(session_id).await,
        StoreWrite::DeleteSession(id) => store.delete_session(id).await,
        StoreWrite::UpdatePending {
            session_id,
            queue,
            drafts,
        } => store.update_pending(session_id, queue, drafts).await,
        StoreWrite::UpdateSessionOrder { order } => store.update_session_order(order).await,
        StoreWrite::UpdateJudgeRuns { session_id, runs } => {
            store.update_judge_runs(session_id, runs).await
        }
        StoreWrite::UpdateAutoResume { session_id, value } => {
            store.update_auto_resume(session_id, *value).await
        }
        StoreWrite::PutSetting { key, value } => store.put_setting(key, value).await,
        StoreWrite::UpdateMobileReviewState { session_id, value } => {
            store.update_mobile_review_state(session_id, value).await
        }
        StoreWrite::UpsertWakeup {
            session_id,
            fire_at_ms,
            prompt,
        } => store.upsert_wakeup(session_id, *fire_at_ms, prompt).await,
        StoreWrite::DeleteWakeup { session_id } => store.delete_wakeup(session_id).await,
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
        let Some(blocks) = build_prompt_blocks(&text, &content) else {
            tracing::warn!(session = %session_id, "queued prompt had no content; dropping");
            hub.clear_in_flight(&session_id);
            continue;
        };
        let title = first_prompt_title(&text, &content);
        match supervisor.send(&session_id, AgentCommand::Prompt(blocks, cmid, None)) {
            Ok(()) => {
                if let Some(t) = title {
                    hub.auto_title(&session_id, t);
                }
            }
            Err(e) => {
                tracing::warn!(session = %session_id, error = %e, "queued dispatch failed");
                hub.clear_in_flight(&session_id);
                hub.broadcast_error(Some(session_id), format!("send failed: {e}"));
            }
        }
    }
    tracing::info!("dispatcher shutting down (channel closed)");
}

/// Revive one opted-in interrupted session only when it has durable work ready
/// to drain. This is shared by startup recovery and the delayed detached-worker
/// reconciliation path so neither requires a browser to open the session.
fn revive_auto_resume_session(hub: &Hub, supervisor: &Supervisor, session_id: &str) {
    if hub.status(session_id) != Some(Status::Interrupted)
        || !hub.effective_auto_resume(session_id)
        || hub
            .session_info(session_id)
            .is_none_or(|info| info.queue_count == 0)
    {
        return;
    }
    match supervisor.ensure_alive(session_id) {
        Ok(revived) => {
            tracing::info!(session = %session_id, revived, "auto-resume: reviving interrupted turn");
        }
        Err(error) => {
            tracing::warn!(session = %session_id, %error, "auto-resume revive failed");
        }
    }
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

async fn serve_axum(
    bind: std::net::SocketAddr,
    state: AppState,
    shutdown_tx: watch::Sender<bool>,
) -> anyhow::Result<()> {
    let state = Arc::new(state);

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
        .route("/api/usage", get(api_usage).post(api_usage_refresh))
        .route("/api/usage/{provider}", post(api_usage_provider_refresh))
        .route("/api/usage/logs", get(api_usage_logs))
        .route("/api/usage/codex/reset", post(api_codex_reset))
        .route(
            "/api/usage/codex/reset/schedule",
            put(api_codex_reset_schedule).delete(api_codex_reset_cancel),
        )
        .route("/metrics", get(prometheus_metrics))
        .route("/api/workspaces", get(api_workspaces))
        .route("/api/machines", get(api_machines))
        .route(
            "/api/machines/enrollment",
            post(api_machine_create_enrollment),
        )
        .route("/api/machines/{id}/events", get(api_machine_events))
        .route("/api/machines/{id}/refresh", post(api_machine_refresh))
        .route("/api/machines/{id}/login", post(api_machine_login))
        .route(
            "/api/machines/{id}/login/{request_id}",
            post(api_machine_login_submit).delete(api_machine_login_cancel),
        )
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
        .route("/api/machine/enroll", post(api_machine_enroll))
        .route("/api/machine/connect", any(machine_ws_upgrade))
        .route("/api/sessions", post(api_new_session))
        .route(
            "/api/sessions/reconcile-project",
            post(api_reconcile_project_sessions),
        )
        .route("/api/sessions/{id}/files", get(api_search_files))
        .route("/api/sessions/{id}/file-tree", get(api_file_tree))
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
        .route("/api/sessions/{id}/question-pages", get(api_question_pages))
        .route(
            "/api/sessions/{id}/question-pages/{page_id}",
            get(api_question_page),
        )
        .route("/api/sessions/{id}/bootstrap", get(api_session_bootstrap))
        .route("/api/sessions/{id}/prompt", post(api_session_prompt))
        .route("/api/history/{id}", get(api_history))
        .route("/api/artifacts/{name}", get(api_artifact))
        .route("/ws", any(ws_upgrade))
        // Everything else: the separately deployed SPA, with index.html
        // fallback for client-side routes.
        .fallback(static_handler)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(addr = %bind, "WS/HTTP listening");

    axum::serve(listener, app)
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
    persistence_pending: usize,
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
}

/// Resident set size of THIS process (the daemon, not its agent children) from
/// `/proc/self/statm` — field 2 is resident pages. 0 if unreadable.
fn daemon_rss_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .nth(1)
                .and_then(|f| f.parse::<u64>().ok())
        })
        .map_or(0, |pages| pages.saturating_mul(4096))
}

async fn api_metrics(State(state): State<Arc<AppState>>) -> Response {
    let sessions_live = state.hub.session_list().len();
    let (db_bytes, events_rows, sessions_deleted) = match &state.store {
        Some(s) => s.storage_metrics().await.unwrap_or((0, 0, 0)),
        None => (0, i64::try_from(state.hub.event_total()).unwrap_or(0), 0),
    };
    let runtime = state.runtime_router.stats();
    let code_cache = state.code_cache.metrics();
    Json(Metrics {
        db_bytes,
        events_rows,
        sessions_live,
        sessions_deleted,
        daemon_rss_bytes: daemon_rss_bytes(),
        persistence_pending: state.persistence_health.as_ref().map_or(0, |h| h.pending()),
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
    let snapshot =
        crate::usage::with_session_usage(state.usage.snapshot().await, &state.hub.session_list());
    Json(snapshot).into_response()
}

async fn api_usage_refresh(State(state): State<Arc<AppState>>) -> Response {
    let snapshot =
        crate::usage::with_session_usage(state.usage.refresh().await, &state.hub.session_list());
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
        ))
        .into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct CodexResetScheduleRequest {
    fire_at_ms: i64,
    confirm: String,
}

#[derive(Deserialize)]
struct CodexResetRequest {
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

async fn api_codex_reset(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CodexResetRequest>,
) -> Response {
    if request.confirm != "confirm" {
        return (StatusCode::BAD_REQUEST, "confirmation must be confirm").into_response();
    }
    let key = new_reset_idempotency_key();
    if let Some(store) = state.store.as_ref() {
        let _ = store
            .append_provider_action_log(
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
        .consume_nearest_reset(&key, Some(&request.expected_credit_id))
        .await
    {
        Ok(result) => {
            if let Some(store) = state.store.as_ref() {
                let _ = store
                    .append_provider_action_log(
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

async fn api_codex_reset_schedule(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CodexResetScheduleRequest>,
) -> Response {
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
    if let Err(error) = store.upsert_codex_reset(request.fire_at_ms, &key).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    let _ = store
        .append_provider_action_log(
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
        .set_reset_schedule(Some(crate::usage::CodexResetSchedule {
            fire_at_ms: request.fire_at_ms,
        }))
        .await;
    Json(state.usage.snapshot().await).into_response()
}

async fn api_codex_reset_cancel(State(state): State<Arc<AppState>>) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "persistent scheduling unavailable",
        )
            .into_response();
    };
    if let Err(error) = store.delete_codex_reset().await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    let _ = store
        .append_provider_action_log(
            "scheduled",
            "cancelled",
            "timer",
            "Scheduled reset cancelled",
            None,
            None,
            now_ms(),
        )
        .await;
    state.usage.set_reset_schedule(None).await;
    StatusCode::NO_CONTENT.into_response()
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

async fn prometheus_metrics(State(state): State<Arc<AppState>>) -> Response {
    let sessions_live = state.hub.session_list().len();
    let (db_bytes, events_rows, sessions_deleted) = match &state.store {
        Some(store) => store.storage_metrics().await.unwrap_or((0, 0, 0)),
        None => (0, i64::try_from(state.hub.event_total()).unwrap_or(0), 0),
    };
    let health = state.persistence_health.as_ref();
    let runtime = state.runtime_router.stats();
    let runtime_connected = state.runtime_router.has_connected_runtime();
    let body = format!(
        "# TYPE cowboy_up gauge\ncowboy_up {}\n# TYPE cowboy_database_bytes gauge\ncowboy_database_bytes {db_bytes}\n# TYPE cowboy_events_rows gauge\ncowboy_events_rows {events_rows}\n# TYPE cowboy_sessions gauge\ncowboy_sessions{{state=\"live\"}} {sessions_live}\ncowboy_sessions{{state=\"deleted\"}} {sessions_deleted}\n# TYPE cowboy_daemon_rss_bytes gauge\ncowboy_daemon_rss_bytes {}\n# TYPE cowboy_persistence_pending gauge\ncowboy_persistence_pending {}\n# TYPE cowboy_persistence_dropped_total counter\ncowboy_persistence_dropped_total {}\n# TYPE cowboy_persistence_failed_batches_total counter\ncowboy_persistence_failed_batches_total {}\n# TYPE cowboy_persistence_healthy gauge\ncowboy_persistence_healthy {}\n# TYPE cowboy_runtime_connected gauge\ncowboy_runtime_connected {}\n# TYPE cowboy_runtime_workers gauge\ncowboy_runtime_workers {}\n# TYPE cowboy_runtime_busy_workers gauge\ncowboy_runtime_busy_workers {}\n# TYPE cowboy_runtime_draining_workers gauge\ncowboy_runtime_draining_workers {}\n# TYPE cowboy_runtime_handoff_workers gauge\ncowboy_runtime_handoff_workers {}\n# TYPE cowboy_runtime_pending_commands gauge\ncowboy_runtime_pending_commands {}\n# TYPE cowboy_observability_pending gauge\ncowboy_observability_pending {}\n# TYPE cowboy_observability_accepted_batches_total counter\ncowboy_observability_accepted_batches_total {}\n# TYPE cowboy_observability_dropped_batches_total counter\ncowboy_observability_dropped_batches_total {}\n# TYPE cowboy_observability_failed_log_batches_total counter\ncowboy_observability_failed_log_batches_total {}\n# TYPE cowboy_observability_failed_metric_batches_total counter\ncowboy_observability_failed_metric_batches_total {}\n",
        u8::from(state.runtime_health.is_healthy(state.store.is_some()) && runtime_connected),
        daemon_rss_bytes(),
        health.map_or(0, |h| h.pending()),
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

#[derive(Debug, Serialize)]
struct MachineSummary {
    id: String,
    display_name: String,
    platform: String,
    architecture: String,
    status: String,
    local: bool,
    schedulable: bool,
    fingerprint: Option<String>,
    workspaces: Vec<crate::machine_protocol::MachineWorkspace>,
    components: Vec<crate::machine_protocol::ComponentInventory>,
    capacity: crate::machine_protocol::MachineCapacity,
    active_sessions: u32,
    pending_updates: Vec<crate::machine_protocol::ComponentId>,
}

async fn api_machines(State(state): State<Arc<AppState>>) -> Response {
    if let Some(store) = state.store.as_ref() {
        return match store.list_machines().await {
            Ok(machines) => Json(
                machines
                    .into_iter()
                    .filter(|machine| !machine.revoked)
                    .map(|machine| {
                        let workspaces: Vec<crate::machine_protocol::MachineWorkspace> = machine
                            .inventory
                            .get("workspaces")
                            .cloned()
                            .and_then(|value| serde_json::from_value(value).ok())
                            .unwrap_or_default();
                        let mut components: Vec<crate::machine_protocol::ComponentInventory> =
                            machine
                                .inventory
                                .get("components")
                                .cloned()
                                .and_then(|value| serde_json::from_value(value).ok())
                                .unwrap_or_default();
                        let capacity: crate::machine_protocol::MachineCapacity = machine
                            .inventory
                            .get("capacity")
                            .cloned()
                            .and_then(|value| serde_json::from_value(value).ok())
                            .unwrap_or_default();
                        let live_sessions: Vec<_> = state
                            .hub
                            .session_list()
                            .into_iter()
                            .filter(|session| {
                                session.machine_id == machine.id
                                    && session.status != crate::agent_model::Status::Exited
                            })
                            .collect();
                        let active_sessions =
                            u32::try_from(live_sessions.len()).unwrap_or(u32::MAX);
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
                                        u64::try_from(
                                            live_sessions
                                                .iter()
                                                .filter(|session| {
                                                    let provider = session.provider.as_str();
                                                    provider == slot
                                                        || (slot == "claude"
                                                            && matches!(
                                                                provider,
                                                                "claude-code" | "claude-deepseek"
                                                            ))
                                                })
                                                .count(),
                                        )
                                        .unwrap_or(u64::MAX)
                                    }
                                    _ => u64::from(active_sessions),
                                };
                            }
                            if let Some(desired) = state
                                .desired_machine_components
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
                                    checked_at_ms: now_ms(),
                                    installable: available,
                                });
                            }
                        }
                        let pending_updates = state
                            .desired_machine_components
                            .iter()
                            .filter(|desired| {
                                !components.iter().any(|current| {
                                    current.id == desired.id
                                        && current.digest.eq_ignore_ascii_case(&desired.digest)
                                        && current.state
                                            == crate::machine_protocol::ComponentState::Active
                                })
                            })
                            .map(|desired| desired.id.clone())
                            .collect();
                        let schedulable = state.runtime_router.connected(&machine.id)
                            && !workspaces.is_empty()
                            && !capacity.draining
                            && active_sessions < capacity.max_sessions;
                        MachineSummary {
                            local,
                            schedulable,
                            id: machine.id,
                            display_name: machine.display_name,
                            platform: machine.platform,
                            architecture: machine.architecture,
                            status: machine.status,
                            fingerprint: machine.fingerprint,
                            workspaces,
                            components,
                            capacity,
                            active_sessions,
                            pending_updates,
                        }
                    })
                    .collect::<Vec<_>>(),
            )
            .into_response(),
            Err(error) => {
                tracing::error!(%error, "listing Machines");
                (StatusCode::INTERNAL_SERVER_ERROR, "could not list machines").into_response()
            }
        };
    }
    Json(Vec::<MachineSummary>::new()).into_response()
}

async fn api_machine_events(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    Json(state.machine_control.events(&machine_id)).into_response()
}

#[derive(Debug, Deserialize)]
struct MachineLoginRequest {
    provider: String,
    #[serde(default)]
    auth_method: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MachineLoginCodeRequest {
    code: String,
}

#[derive(Debug, Deserialize)]
struct MachineEnrollmentRequest {
    machine_id: String,
    display_name: String,
}

#[derive(Debug, Serialize)]
struct MachineEnrollmentResponse {
    machine_id: String,
    display_name: String,
    token: String,
    expires_in_seconds: i64,
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
    match store
        .create_machine_enrollment(&request.machine_id, &request.display_name, TTL_SECONDS)
        .await
    {
        Ok(token) => Json(MachineEnrollmentResponse {
            machine_id: request.machine_id,
            display_name: request.display_name,
            token,
            expires_in_seconds: TTL_SECONDS,
        })
        .into_response(),
        Err(error) => (StatusCode::CONFLICT, error.to_string()).into_response(),
    }
}

#[derive(Debug, Serialize)]
struct MachineCommandResponse {
    request_id: String,
}

fn machine_request_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        random_machine_token().unwrap_or_else(|_| now_ms().to_string())
    )
}

async fn api_machine_refresh(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    let request_id = machine_request_id("refresh");
    match state.machine_control.send(
        &machine_id,
        crate::machine_protocol::MachineCommand::RefreshInventory {
            request_id: request_id.clone(),
        },
    ) {
        Ok(()) => Json(MachineCommandResponse { request_id }).into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

async fn api_machine_login(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
    Json(request): Json<MachineLoginRequest>,
) -> Response {
    if !matches!(request.provider.as_str(), "codex" | "claude" | "gemini") {
        return (StatusCode::BAD_REQUEST, "unknown provider").into_response();
    }
    if request.provider == "gemini"
        && !matches!(
            request.auth_method.as_deref(),
            Some("api_key" | "code_assist")
        )
    {
        return (StatusCode::BAD_REQUEST, "Gemini auth method is required").into_response();
    }
    if request.provider != "gemini" && request.auth_method.is_some() {
        return (
            StatusCode::BAD_REQUEST,
            "auth method is not supported for this provider",
        )
            .into_response();
    }
    if request.provider == "gemini" {
        let supports_current_auth = match state.store.as_ref() {
            Some(store) => store
                .list_machines()
                .await
                .ok()
                .and_then(|machines| {
                    machines
                        .into_iter()
                        .find(|machine| machine.id == machine_id)
                })
                .and_then(|machine| machine.inventory.get("components").cloned())
                .and_then(|value| {
                    serde_json::from_value::<Vec<crate::machine_protocol::ComponentInventory>>(
                        value,
                    )
                    .ok()
                })
                .is_some_and(|components| gemini_machine_auth_is_current(&components)),
            None => false,
        };
        if !supports_current_auth {
            return (
                StatusCode::CONFLICT,
                "This Machine must update cowboy-machine before using the current Gemini authentication flows",
            ).into_response();
        }
    }
    let request_id = machine_request_id("login");
    match state.machine_control.send(
        &machine_id,
        crate::machine_protocol::MachineCommand::BeginLogin {
            request_id: request_id.clone(),
            provider: request.provider,
            auth_method: request.auth_method,
        },
    ) {
        Ok(()) => Json(MachineCommandResponse { request_id }).into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

async fn api_machine_login_submit(
    State(state): State<Arc<AppState>>,
    Path((machine_id, request_id)): Path<(String, String)>,
    Json(request): Json<MachineLoginCodeRequest>,
) -> Response {
    let code = request.code.trim();
    if code.is_empty() || code.len() > 16_384 {
        return (StatusCode::BAD_REQUEST, "authorization code is invalid").into_response();
    }
    match state.machine_control.send(
        &machine_id,
        crate::machine_protocol::MachineCommand::SubmitLoginCode {
            request_id: request_id.clone(),
            code: code.to_owned(),
        },
    ) {
        Ok(()) => Json(MachineCommandResponse { request_id }).into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
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
    match state.machine_control.send(
        &machine_id,
        crate::machine_protocol::MachineCommand::Reconcile {
            request_id: request_id.clone(),
            components: state.desired_machine_components.as_ref().clone(),
        },
    ) {
        Ok(()) => Json(MachineCommandResponse { request_id }).into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
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
    match state.machine_control.send(
        &machine_id,
        crate::machine_protocol::MachineCommand::Reconcile {
            request_id: request_id.clone(),
            components: vec![component],
        },
    ) {
        Ok(()) => Json(MachineCommandResponse { request_id }).into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

async fn api_machine_update_npm(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
    Json(component): Json<crate::machine_protocol::ComponentId>,
) -> Response {
    let request_id = machine_request_id("update-npm");
    match state.machine_control.send(
        &machine_id,
        crate::machine_protocol::MachineCommand::UpdateNpmComponent {
            request_id: request_id.clone(),
            component,
        },
    ) {
        Ok(()) => Json(MachineCommandResponse { request_id }).into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

async fn api_machine_login_cancel(
    State(state): State<Arc<AppState>>,
    Path((machine_id, request_id)): Path<(String, String)>,
) -> Response {
    match state.machine_control.send(
        &machine_id,
        crate::machine_protocol::MachineCommand::CancelLogin {
            request_id: request_id.clone(),
        },
    ) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

async fn api_machine_revoke(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "persistence unavailable").into_response();
    };
    match store.revoke_machine(&machine_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct MachineEnrollRequest {
    token: String,
    public_key: String,
}

#[derive(Debug, Serialize)]
struct MachineEnrollResponse {
    machine_id: String,
    display_name: String,
    fingerprint: String,
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
        .consume_machine_enrollment(&request.token, &public_key)
        .await
    {
        Ok(machine) => (
            StatusCode::CREATED,
            Json(MachineEnrollResponse {
                machine_id: machine.id,
                display_name: machine.display_name,
                fingerprint: machine.fingerprint,
            }),
        )
            .into_response(),
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

const MACHINE_HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const MACHINE_HEARTBEAT_MS: u64 = 15_000;
const WEBSOCKET_FRAME_SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

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
    };
    if send_json(&mut socket, &challenge).await.is_err() {
        return;
    }
    let incoming = match tokio::time::timeout(MACHINE_HANDSHAKE_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => text,
        _ => return,
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
    let proof =
        crate::machine_protocol::challenge_proof_v1(&challenge_id, &nonce, expires_at_ms, &hello);
    let signature = signature.to_owned();
    let verified = tokio::task::spawn_blocking(move || {
        crate::machine_auth::verify(&public_key, &proof, &signature)
    })
    .await;
    if !matches!(verified, Ok(Ok(true))) {
        tracing::warn!(machine = %hello.machine_id, "Machine challenge verification failed");
        return;
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
        "workspaces": &hello.workspaces,
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
        return;
    }
    tracing::info!(machine = %hello.machine_id, "Machine connected");
    let (machine_command_tx, mut machine_command_rx) = mpsc::unbounded_channel();
    state.machine_control.install(
        hello.machine_id.clone(),
        challenge_id.clone(),
        connection_mode == "local",
        machine_command_tx,
    );
    state.machine_control.record(
        &hello.machine_id,
        crate::machine_protocol::MachineEvent::Inventory {
            components: hello.components.clone(),
            observed_at_ms: now_ms(),
        },
    );
    let (runtime_core, runtime_tunnel) = match UnixStream::pair() {
        Ok(pair) => pair,
        Err(error) => {
            tracing::error!(%error, machine = %hello.machine_id, "creating Machine runtime tunnel");
            return;
        }
    };
    let (runtime_reader, mut runtime_writer) = runtime_tunnel.into_split();
    let mut runtime_reader = crate::runtime_wire::FrameReader::new(runtime_reader);
    let (runtime_tx, mut runtime_rx) = tokio::sync::oneshot::channel();
    {
        let router = Arc::clone(&state.runtime_router);
        let hub = state.hub.clone();
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
                    let runtime = RemoteRuntime::new(hub, &bootstrap, generation, None);
                    router.install(machine_id, Arc::clone(&runtime));
                    runtime.start(bootstrap);
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
            frame = runtime_reader.next() => {
                match frame {
                    Ok(Some(frame)) => {
                        if send_json(
                            &mut socket,
                            &crate::machine_protocol::MachineFrame::Runtime { frame },
                        ).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    Ok(None) | Err(_) => break,
                }
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
                event: crate::machine_protocol::MachineEvent::Inventory { components, .. },
            } => {
                state.machine_control.record(
                    &hello.machine_id,
                    crate::machine_protocol::MachineEvent::Inventory {
                        components: components.clone(),
                        observed_at_ms: now_ms(),
                    },
                );
                let inventory = serde_json::json!({
                    "components": components,
                    "workspaces": &hello.workspaces,
                    "capacity": &hello.capacity,
                });
                store
                    .machine_seen(&hello.machine_id, &challenge_id, Some(&inventory))
                    .await
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
            crate::machine_protocol::MachineFrame::Event { event } => {
                state.machine_control.record(&hello.machine_id, event);
                store
                    .machine_seen(&hello.machine_id, &challenge_id, None)
                    .await
            }
            crate::machine_protocol::MachineFrame::Runtime { frame } => {
                if let Err(error) =
                    crate::runtime_wire::write_frame(&mut runtime_writer, &frame).await
                {
                    tracing::warn!(%error, machine = %hello.machine_id, "writing Machine runtime frame");
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

/// Response body for `POST /api/sessions`.
#[derive(Debug, Serialize)]
struct NewSessionResponse {
    session_id: String,
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

async fn api_new_session(
    State(state): State<Arc<AppState>>,
    Json(req): Json<NewSessionRequest>,
) -> Response {
    let mut cwd = req.cwd;
    let session_id = state.supervisor.reserve_session_id();
    if web_session_is_missing_machine(&req.machine_id, &req.origin) {
        return (
            StatusCode::CONFLICT,
            "Web session creation requires a connected Machine so its workspace can be isolated",
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
        let supported = machine
            .inventory
            .get("components")
            .cloned()
            .and_then(|value| {
                serde_json::from_value::<Vec<crate::machine_protocol::ComponentInventory>>(value)
                    .ok()
            })
            .is_some_and(|components| machine_supports_provider(&components, &req.provider));
        if !supported {
            return (
                StatusCode::CONFLICT,
                format!(
                    "provider {:?} is not installed and authenticated on machine {:?}",
                    req.provider, req.machine_id
                ),
            )
                .into_response();
        }

        let Some(source_path) = cwd.as_deref() else {
            return (
                StatusCode::BAD_REQUEST,
                "remote session requires a trusted workspace",
            )
                .into_response();
        };
        if let Err(message) = state.supervisor.register_session_on_with_id(
            &session_id,
            &req.provider,
            Some(source_path.to_owned()),
            req.origin,
            req.system,
            crate::supervisor::SessionPlacement {
                machine_id: &req.machine_id,
                workspace: Some(&selected_workspace),
            },
        ) {
            return (StatusCode::BAD_REQUEST, message).into_response();
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
        return (StatusCode::CREATED, Json(NewSessionResponse { session_id })).into_response();
    }
    match state.supervisor.new_session_on_with_id(
        &session_id,
        &req.provider,
        cwd,
        req.origin,
        req.system,
        &req.machine_id,
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
            (StatusCode::CREATED, Json(NewSessionResponse { session_id })).into_response()
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

fn machine_supports_provider(
    components: &[crate::machine_protocol::ComponentInventory],
    provider: &str,
) -> bool {
    use crate::machine_protocol::{AuthState, ComponentKind, ComponentState};
    let cli_slot = match provider {
        "claude-code" | "claude-deepseek" => "claude",
        "codex-deepseek" => "codex",
        provider => provider,
    };
    let cli_ready = components.iter().any(|component| {
        component.id.kind == ComponentKind::ProviderCli
            && matches!(component.id.slot.as_str(), candidate if candidate == cli_slot || candidate == provider)
            && component.state == ComponentState::Active
            && (matches!(provider, "codex-deepseek" | "claude-deepseek")
                || component.auth == Some(AuthState::SignedIn))
    });
    if !cli_ready {
        return false;
    }
    if provider == "gemini" {
        return gemini_machine_auth_is_current(components);
    }
    let adapter_active = |slot: &str| {
        components.iter().any(|component| {
            component.id.kind == ComponentKind::ProviderAdapter
                && component.id.slot == slot
                && component.state == ComponentState::Active
        })
    };
    if provider == "codex-deepseek" {
        adapter_active("codex") && adapter_active("codex-deepseek")
    } else if provider == "claude-deepseek" {
        adapter_active("claude") && adapter_active("claude-deepseek")
    } else {
        adapter_active(cli_slot) || adapter_active(provider)
    }
}

fn gemini_machine_auth_is_current(
    components: &[crate::machine_protocol::ComponentInventory],
) -> bool {
    components.iter().any(|component| {
        component.id.kind == crate::machine_protocol::ComponentKind::ProviderCli
            && component.id.slot == "gemini"
            && component.detail.is_some()
    })
}

#[cfg(test)]
mod machine_provider_tests {
    use super::{
        machine_supports_provider, resolve_machine_workspace, web_session_is_missing_machine,
    };
    use crate::core::SessionOrigin;
    use crate::machine_protocol::{
        AuthState, ComponentId, ComponentInventory, ComponentKind, ComponentState,
    };

    fn component(kind: ComponentKind, slot: &str, auth: Option<AuthState>) -> ComponentInventory {
        ComponentInventory {
            id: ComponentId {
                kind,
                slot: slot.to_owned(),
            },
            state: ComponentState::Active,
            version: "v1".to_owned(),
            generation: "v1".to_owned(),
            digest: "digest".to_owned(),
            rollback_generation: None,
            active_leases: 0,
            auth,
            detail: None,
            update: None,
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
    fn codex_requires_authenticated_cli_and_adapter() {
        let cli = component(
            ComponentKind::ProviderCli,
            "codex",
            Some(AuthState::SignedIn),
        );
        assert!(!machine_supports_provider(
            std::slice::from_ref(&cli),
            "codex"
        ));
        let adapter = component(ComponentKind::ProviderAdapter, "codex", None);
        assert!(machine_supports_provider(&[cli, adapter], "codex"));
    }

    #[test]
    fn claude_provider_id_maps_to_managed_claude_slot() {
        let components = [
            component(
                ComponentKind::ProviderCli,
                "claude",
                Some(AuthState::SignedIn),
            ),
            component(ComponentKind::ProviderAdapter, "claude", None),
        ];
        assert!(machine_supports_provider(&components, "claude-code"));
    }

    #[test]
    fn deepseek_runtime_reuses_managed_codex_components() {
        let codex_only = [
            component(
                ComponentKind::ProviderCli,
                "codex",
                Some(AuthState::SignedIn),
            ),
            component(ComponentKind::ProviderAdapter, "codex", None),
        ];
        assert!(!machine_supports_provider(&codex_only, "codex-deepseek"));
        let components = [
            component(
                ComponentKind::ProviderCli,
                "codex",
                Some(AuthState::SignedIn),
            ),
            component(ComponentKind::ProviderAdapter, "codex", None),
            component(ComponentKind::ProviderAdapter, "codex-deepseek", None),
        ];
        assert!(machine_supports_provider(&components, "codex-deepseek"));

        let signed_out_components = [
            component(
                ComponentKind::ProviderCli,
                "codex",
                Some(AuthState::SignedOut),
            ),
            component(ComponentKind::ProviderAdapter, "codex", None),
            component(ComponentKind::ProviderAdapter, "codex-deepseek", None),
        ];
        assert!(machine_supports_provider(
            &signed_out_components,
            "codex-deepseek"
        ));
    }

    #[test]
    fn deepseek_runtime_reuses_claude_adapter_without_claude_login() {
        let claude_only = [
            component(
                ComponentKind::ProviderCli,
                "claude",
                Some(AuthState::SignedIn),
            ),
            component(ComponentKind::ProviderAdapter, "claude", None),
        ];
        assert!(!machine_supports_provider(&claude_only, "claude-deepseek"));
        let components = [
            component(
                ComponentKind::ProviderCli,
                "claude",
                Some(AuthState::SignedOut),
            ),
            component(ComponentKind::ProviderAdapter, "claude", None),
            component(ComponentKind::ProviderAdapter, "claude-deepseek", None),
        ];
        assert!(machine_supports_provider(&components, "claude-deepseek"));
    }

    #[test]
    fn gemini_cli_is_its_acp_entrypoint_only_after_login_is_confirmed() {
        let cli = component(
            ComponentKind::ProviderCli,
            "gemini",
            Some(AuthState::SignedOut),
        );
        assert!(!machine_supports_provider(
            std::slice::from_ref(&cli),
            "gemini"
        ));
        let authenticated = ComponentInventory {
            auth: Some(AuthState::SignedIn),
            detail: Some("Gemini API key".to_owned()),
            ..cli
        };
        assert!(machine_supports_provider(&[authenticated], "gemini"));
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
    session_id: &str,
    cwd: &str,
    operation: serde_json::Value,
) -> anyhow::Result<Option<crate::code_adapter::CodeAdapterResponse>> {
    let machine_id = state
        .hub
        .session_list()
        .into_iter()
        .find(|meta| meta.id == session_id)
        .map(|meta| meta.machine_id)
        .context("unknown session")?;
    if machine_id == "local" {
        return Ok(None);
    }
    let colocated = match state.machine_control.is_colocated(&machine_id) {
        Some(value) => value,
        None => match state.store.as_ref() {
            Some(store) => store.machine_is_local(&machine_id).await.unwrap_or(false),
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
        .adapter_request(&machine_id, "code", request)
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
    let Some(cwd) = state
        .hub
        .session_list()
        .into_iter()
        .find(|m| m.id == session_id)
        .map(|m| m.cwd)
    else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let limit = query.limit.clamp(1, 100);
    let files = match remote_code_request(
        &state,
        &session_id,
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
    let Some(cwd) = session_cwd(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let limit = query.limit.clamp(1, 100);
    let files = match remote_code_request(
        &state,
        &session_id,
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
    let Some(cwd) = state
        .hub
        .session_list()
        .into_iter()
        .find(|m| m.id == session_id)
        .map(|m| m.cwd)
    else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let limit = query.limit.clamp(20, 500);
    let path = query.path;
    let requested_path = path.clone();
    let remote_page = match remote_code_request(
        &state,
        &session_id,
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

fn session_cwd(state: &AppState, session_id: &str) -> Option<String> {
    state
        .hub
        .session_list()
        .into_iter()
        .find(|meta| meta.id == session_id)
        .map(|meta| meta.cwd)
}

async fn api_code_manifest(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(cwd) = session_cwd(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let language_ready = match ensure_zed_worktree_for_session(&state, &session_id, &cwd).await {
        Ok(ready) => ready,
        Err(error) => {
            tracing::warn!(session = %session_id, %error, "Zed adapter unavailable");
            false
        }
    };
    let manifest = match remote_code_request(
        &state,
        &session_id,
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
    let Some(cwd) = session_cwd(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let result = match remote_code_request(
        &state,
        &session_id,
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
) -> Response {
    let Some(cwd) = session_cwd(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let result = match remote_code_request(
        &state,
        &session_id,
        &cwd,
        serde_json::json!({ "type": "repository" }),
    )
    .await
    {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::Repository(repository))) => repository,
        Ok(Some(_)) | Err(_) => {
            return (StatusCode::BAD_GATEWAY, "remote git history unavailable").into_response();
        }
        Ok(None) => {
            let result = tokio::task::spawn_blocking(move || {
                crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd)).repository()
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
    let Some(cwd) = session_cwd(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let oid = query.oid;
    let result = match remote_code_request(
        &state,
        &session_id,
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
    let Some(cwd) = session_cwd(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let oid = query.oid;
    let path = query.path;
    let result = match remote_code_request(
        &state,
        &session_id,
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
    let Some(cwd) = session_cwd(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
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
        &session_id,
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
    let Some(cwd) = session_cwd(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let result = match remote_code_request(
        &state,
        &session_id,
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
        Err(_) => return (StatusCode::BAD_REQUEST, "file unavailable").into_response(),
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
    let Some(cwd) = session_cwd(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if query.path.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid buffer path").into_response();
    }
    match zed_adapter_request_for_session(
        &state,
        &session_id,
        serde_json::json!({
            "type": "bufferLanguage",
            "worktree": cwd,
            "path": query.path,
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
    let Some(cwd) = session_cwd(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if query.path.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid buffer path").into_response();
    }
    match zed_adapter_request_for_session(
        &state,
        &session_id,
        serde_json::json!({
            "type": "bufferHover",
            "worktree": cwd,
            "path": query.path,
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
    let Some(cwd) = session_cwd(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if query.path.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid buffer path").into_response();
    }
    match zed_adapter_request_for_session(
        &state,
        &session_id,
        serde_json::json!({
            "type": "bufferNavigate",
            "worktree": cwd,
            "path": query.path,
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
    let Some(cwd) = session_cwd(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if query.path.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid buffer path").into_response();
    }
    match zed_adapter_request_for_session(
        &state,
        &session_id,
        serde_json::json!({
            "type": "bufferSymbols",
            "worktree": cwd,
            "path": query.path,
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
    let Some(cwd) = session_cwd(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if request.path.is_empty()
        || request.lease_id.is_empty()
        || request.lease_id.len() > 128
        || request.lease_id.chars().any(char::is_whitespace)
    {
        return (StatusCode::BAD_REQUEST, "invalid buffer lease").into_response();
    }
    if open
        && ensure_zed_worktree_for_session(&state, &session_id, &cwd)
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
            "worktree": cwd,
            "path": request.path,
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
/// config option, queue, and judge history made mobile reconnects multi-megabyte
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
    messages.push(Outbound::JudgeHistory {
        session_id: session_id.to_owned(),
        runs: hub.judge_history(session_id),
    });
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

async fn api_artifact(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
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

/// Serve a separately deployed asset by path, falling back to `index.html` so
/// the SPA owns client-side routing. Missing hashed assets never fall back: a
/// module request must receive a real 404 rather than HTML with a 200 status.
/// Missing `index.html` (UI not built) → 404.
///
/// Caching: a per-file SHA256 is used as a content `ETag` (stable across
/// rollouts when the bytes are unchanged). The
/// cache policy is split by whether the filename is content-addressed:
///   - `/assets/*` — Vite emits content-hashed names, so the bytes behind a name
///     never change → `immutable` with a one-year max-age, never revalidated.
///   - everything else (index.html, sw.js, manifest, favicon, icons) —
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
    // SPA handles the route. Vite's /assets names are content-addressed files,
    // never client-side routes. Returning index.html for a missing old chunk
    // makes browsers report the opaque "Importing a module script failed"
    // error because a JS import received text/html.
    let requested_path = state.web_root.join(requested);
    let (name, content) = match tokio::fs::read(&requested_path).await {
        Ok(bytes) => (requested, bytes),
        Err(request_error) if requested.starts_with("assets/") => {
            tracing::info!(
                requested = %requested_path.display(),
                %request_error,
                "requested web asset is no longer deployed"
            );
            return StatusCode::NOT_FOUND.into_response();
        }
        Err(request_error) => match tokio::fs::read(state.web_root.join("index.html")).await {
            Ok(bytes) => ("index.html", bytes),
            Err(index_error) => {
                tracing::warn!(
                    requested = %requested_path.display(),
                    %request_error,
                    %index_error,
                    "reading web asset failed"
                );
                return (StatusCode::NOT_FOUND, "UI not built").into_response();
            }
        },
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
    let mime = mime_guess::from_path(name).first_or_octet_stream();
    (
        [
            (header::CONTENT_TYPE, mime.as_ref()),
            (header::CACHE_CONTROL, cache_control),
            (header::ETAG, etag.as_str()),
        ],
        Body::from(content),
    )
        .into_response()
}

/// App-level WS heartbeat interval. 25s stays under the common 60s proxy/idle
/// timeout (the 75% rule) and keeps NAT mappings warm; the client treats missing
/// ~2 of these as a dead socket. See [`crate::core::Outbound::Ping`].
const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(25);

#[derive(Debug, Default, Deserialize)]
struct WebSocketQuery {
    bootstrap: Option<String>,
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Query(query): Query<WebSocketQuery>,
) -> impl IntoResponse {
    let lazy_bootstrap = query.bootstrap.as_deref() == Some("lazy");
    ws.on_upgrade(move |socket| handle_ws(socket, state, lazy_bootstrap))
}

fn global_bootstrap(hub: &Hub) -> Vec<Outbound> {
    let mut messages = vec![
        Outbound::Sessions {
            sessions: hub.session_list(),
        },
        Outbound::Settings {
            settings: hub.settings_snapshot(),
        },
        Outbound::Skills {
            skills: hub.skills_snapshot(),
        },
    ];
    messages.extend(hub.sync_resync());
    messages
}

fn connect_bootstrap(hub: &Hub, lazy: bool) -> Vec<Outbound> {
    let mut messages = global_bootstrap(hub);
    if !lazy {
        for session in hub.session_list() {
            if let Some(session_messages) = focused_session_bootstrap(hub, &session.id) {
                messages.extend(session_messages);
            }
        }
    }
    messages.push(Outbound::BootstrapComplete);
    messages
}

async fn handle_ws(socket: WebSocket, state: Arc<AppState>, lazy_bootstrap: bool) {
    let (mut sink, mut stream) = socket.split();

    // Subscribe BEFORE snapshotting so no event slips through the gap; the
    // client dedups by (session_id, seq), so a brief overlap is harmless.
    let mut rx = state.hub.subscribe();
    let mut shutdown = state.shutdown.clone();

    for message in connect_bootstrap(&state.hub, lazy_bootstrap) {
        if send_json(&mut sink, &message).await.is_err() {
            return;
        }
    }

    // Fan-out task: broadcast events → this socket, plus a periodic app-level
    // heartbeat (Outbound::Ping) so a client can detect a HALF-OPEN socket that
    // never fires `onclose` (see Outbound::Ping). Per-client interval — a failed
    // heartbeat send reaps a dead client here too.
    let mut fanout = tokio::spawn(async move {
        let mut heartbeat = tokio::time::interval(HEARTBEAT);
        // The first tick fires immediately; consume it so the first ping waits a
        // full interval (the connect snapshot is fresh traffic already).
        heartbeat.tick().await;
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
                msg = rx.recv() => match msg {
                    Ok(msg) => {
                        if send_json(&mut sink, &msg).await.is_err() {
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
                _ = heartbeat.tick() => {
                    if send_json(&mut sink, &Outbound::Ping).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Edit-holds this connection set (session_id → held queued-message id). The
    // editing hold is GLOBAL server state, so a client that disconnects mid-edit
    // would otherwise leave the head pinned and stall the queue forever. We
    // track what this socket held and release it on teardown.
    let mut held: HashMap<String, String> = HashMap::new();

    // Inbound command loop.
    loop {
        tokio::select! {
            _ = &mut fanout => break,
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => handle_command(&state, &text, &mut held),
                    // Other frame types (ping/pong/binary) are ignored.
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_))) => {}
                    // Close, transport error, or stream end: tear down.
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                }
            }
        }
    }
    // Release any edit-holds this connection still owns so the queue can drain.
    for session_id in held.keys() {
        state.hub.set_queue_editing(session_id, None);
    }
    fanout.abort();
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
fn handle_command(state: &AppState, text: &str, held: &mut HashMap<String, String>) {
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
        | Inbound::SetAwaiting { session_id, .. }
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
        | Inbound::ReorderDrafts { session_id, .. }
        | Inbound::RemoveJudgeRun { session_id, .. }
        | Inbound::ClearJudgeRuns { session_id } => Some(session_id.clone()),
        // Sync mutations are state-scoped (title/order), not session-scoped — a
        // failure surfaces as a daemon-level error (None).
        Inbound::NewSession { .. }
        | Inbound::ReorderSessions { .. }
        | Inbound::Sync { .. }
        | Inbound::SetSetting { .. } => None,
    };
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
    let result = match cmd {
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
        } => {
            // Generic arbiter apply (title/order/…); validates, dedupes by id,
            // applies the typed mutation, version-stamps + broadcasts the patch.
            state.hub.sync_apply(&sync_state, id, &name, &args)
        }
        Inbound::SetSessionAutoResume { session_id, value } => {
            state.hub.set_auto_resume(&session_id, value);
            Ok(())
        }
        Inbound::SetAwaiting {
            session_id,
            awaiting,
        } => {
            state.hub.set_awaiting(&session_id, awaiting);
            Ok(())
        }
        Inbound::SetPaused { session_id, paused } => {
            state.hub.set_paused(&session_id, paused);
            Ok(())
        }
        Inbound::ResumeTurn { session_id } => {
            state.supervisor.prepare_session(&session_id).map(|_| {
                state.hub.resume_turn(&session_id);
            })
        }
        Inbound::RetryTurn { session_id } => {
            state.supervisor.prepare_session(&session_id).map(|_| {
                state.hub.retry_turn(&session_id);
            })
        }
        Inbound::SetSetting { key, value } => {
            state.hub.set_setting(key, value);
            Ok(())
        }
        Inbound::SetConfigOption {
            session_id,
            config_id,
            value,
        } => state.supervisor.send(
            &session_id,
            AgentCommand::SetConfigOption { config_id, value },
        ),
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
                    && meta.provider == "gemini"
                    && state
                        .hub
                        .latest_crash_detail(&session_id)
                        .as_deref()
                        .is_some_and(crate::provider::gemini::is_retired_consumer_error)
            });
            if terminal_provider_error
                || (state.hub.status(&session_id) == Some(Status::Interrupted)
                    && !state.hub.effective_auto_resume(&session_id))
            {
                // Interrupted + NOT opted into auto-resume → leave it for manual
                // recovery (the "last turn was interrupted" bar; submitting
                // revives it). An auto-resume session falls through to
                // ensure_alive: restore already enqueued the continuation, so
                // reviving here drains it the moment the session is opened.
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
                    // dispatched it — a PAUSED (or awaiting-user) queue HOLDS the
                    // auto-drain. A force-push is an explicit "send this now", so
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
                    held.insert(session_id.clone(), mid.clone());
                }
                None => {
                    held.remove(&session_id);
                }
            }
            state.hub.set_queue_editing(&session_id, id);
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
        Inbound::ReorderSessions { order } => {
            state.hub.reorder_sessions(&order);
            Ok(())
        }
        Inbound::ReorderQueue { session_id, order } => {
            state.hub.reorder_queue(&session_id, &order);
            Ok(())
        }
        Inbound::ReorderDrafts { session_id, order } => {
            state.hub.reorder_drafts(&session_id, &order);
            Ok(())
        }
        Inbound::RemoveJudgeRun { session_id, id } => {
            state.hub.remove_judge_run(&session_id, &id);
            Ok(())
        }
        Inbound::ClearJudgeRuns { session_id } => {
            state.hub.clear_judge_runs(&session_id);
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

async fn send_json<S, T>(sink: &mut S, msg: &T) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
    T: Serialize,
{
    send_json_with_timeout(sink, msg, WEBSOCKET_FRAME_SEND_TIMEOUT).await
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
        let messages = connect_bootstrap(&hub_with_sessions(), true);
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
        assert!(!messages.iter().any(|message| matches!(
            message,
            Outbound::Snapshot { .. }
                | Outbound::ConfigOptions { .. }
                | Outbound::JudgeHistory { .. }
        )));
        assert!(!messages.iter().any(|message| matches!(
            message,
            Outbound::SyncPatch { state, .. } if state.starts_with("queue:")
        )));
    }

    #[test]
    fn legacy_websocket_bootstrap_remains_complete() {
        let messages = connect_bootstrap(&hub_with_sessions(), false);
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
            Outbound::Snapshot { session_id, .. }
            | Outbound::ConfigOptions { session_id, .. }
            | Outbound::JudgeHistory { session_id, .. } => session_id == "focused",
            Outbound::SyncPatch { state, .. } => state == "queue:focused",
            _ => true,
        }));
    }
}
