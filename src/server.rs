//! HTTP / WebSocket server (design §5).
//!
//! Every frontend is an **equal subscriber** to one WebSocket stream. On
//! browsers negotiate a lightweight global session index, then hydrate the
//! focused session over HTTP while the socket carries live events. Legacy and
//! ACP bridge clients retain the complete bootstrap unless they opt into lazy
//! mode, preserving wire compatibility.
//!
//! v1 has **no auth** and binds `0.0.0.0` by deliberate choice (LAN-only use);
//! design §9 auth/pairing is a follow-up.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use axum::Router;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Json, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post, put};
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
use crate::persistence::EventReducer;
use crate::remote_runtime::{RemoteBootstrap, RemoteRuntime};
use crate::runtime::RuntimeHealth;
use crate::runtime_wire::StartSession;
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
    remote_runtime: Option<Arc<RemoteRuntime>>,
    web_root: PathBuf,
    usage: UsageService,
    diff_snapshots: DiffSnapshotCache,
    code_cache: crate::code_cache::CodeCache,
    zed_adapter_socket: Option<PathBuf>,
}

const STORE_QUEUE_CAPACITY: usize = 8_192;
const FORCE_CANCEL_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

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
    let usage = UsageService::new(args.codex_command.clone());
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
    // Acquire the controller lease before restoring Hub state. Its worker
    // snapshot is the authority for whether a persisted Busy turn actually
    // survived this control-plane restart.
    let mut runtime_bootstrap = match args.runtime_socket.clone() {
        Some(socket) => Some(
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                RemoteBootstrap::connect(socket),
            )
            .await
            .context("timed out connecting detached agent runtime")??,
        ),
        None => None,
    };

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
            match runtime_bootstrap.as_ref() {
                Some(runtime) => hub.restore_with_workers(restored, runtime.workers()),
                None => hub.restore(restored),
            }
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
    let remote_runtime = runtime_bootstrap.as_ref().map(|bootstrap| {
        RemoteRuntime::new(
            hub.clone(),
            bootstrap,
            args.worker_generation.clone(),
            args.runtime_worker_command
                .as_ref()
                .map(|path| path.display().to_string()),
        )
    });
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
    if let Some(runtime) = &remote_runtime {
        // Re-declare every adopted worker's launch metadata. This is
        // idempotent, and lets a newly restarted agentd reconstruct sessions
        // even when an older compatible worker snapshot lacks the additive
        // launch-spec field.
        for meta in hub.session_list() {
            if runtime.has_worker(&meta.id) {
                runtime.adopt(StartSession {
                    session_id: meta.id,
                    provider: meta.provider,
                    cwd: meta.cwd,
                    agent_session_id: meta.agent_session_id,
                    system: meta.system,
                    generation: String::new(),
                    fallback_for: None,
                    adopt_only: true,
                });
            }
        }
    }
    let supervisor = Arc::new(match &remote_runtime {
        Some(runtime) => Supervisor::new_remote(
            hub.clone(),
            args.workspace_root.clone(),
            session_id_floor,
            Arc::clone(runtime),
        ),
        None => Supervisor::new(hub.clone(), args.workspace_root.clone(), session_id_floor),
    });

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
    let mut dispatcher_task = tokio::spawn(async move {
        run_dispatcher(
            dispatcher_hub,
            dispatcher_supervisor,
            dispatch_rx,
            dispatcher_shutdown,
        )
        .await;
        dispatcher_health.set_dispatcher(false);
        tracing::error!("dispatcher exited unexpectedly");
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

    // Start replay only after scheduler/dispatcher sinks exist; otherwise a
    // ScheduleWakeup event buffered during the deploy could be ACKed while its
    // side effect was still unwired.
    if let (Some(runtime), Some(bootstrap)) = (&remote_runtime, runtime_bootstrap.take()) {
        runtime.start(bootstrap);
    }

    // Headless auto-resume. A turn cut off by THIS restart had its continuation
    // enqueued + the session marked Interrupted during `hub.restore` above — but
    // that continuation only drains once the agent revives, which used to wait for
    // a CLIENT to open the session. Revive the opted-in ones right here so an
    // interrupted turn resumes with NO client connected — the whole point of
    // auto-resume is surviving an unattended deploy. Gated on a non-empty queue so
    // we only wake sessions that actually have the continuation (or user-queued
    // prompts) to drain, never an idle Interrupted one (which would just spin a
    // misleading "working" state). The drain runs through the dispatcher wired above.
    for meta in hub.session_list() {
        if meta.status != Status::Interrupted || !hub.effective_auto_resume(&meta.id) {
            continue;
        }
        if hub
            .session_info(&meta.id)
            .is_none_or(|i| i.queue_count == 0)
        {
            continue;
        }
        match supervisor.ensure_alive(&meta.id) {
            Ok(revived) => {
                tracing::info!(session = %meta.id, revived, "auto-resume: reviving interrupted turn");
            }
            Err(e) => {
                tracing::warn!(session = %meta.id, error = %e, "auto-resume revive failed");
            }
        }
    }

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
            remote_runtime: remote_runtime.clone(),
            web_root: args.web_root,
            usage,
            diff_snapshots: DiffSnapshotCache::default(),
            code_cache,
            zed_adapter_socket: args.zed_adapter_socket,
        },
        shutdown_tx,
    )
    .await;
    judge_task.abort();
    scheduler_task.abort();
    reset_task.abort();
    match tokio::time::timeout(std::time::Duration::from_secs(5), &mut dispatcher_task).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::error!(%error, "dispatcher task failed during shutdown"),
        Err(_) => {
            tracing::error!("dispatcher did not drain within shutdown deadline");
            dispatcher_task.abort();
        }
    }
    if let Some(runtime) = &remote_runtime {
        runtime
            .graceful_shutdown(std::time::Duration::from_secs(10))
            .await;
    }
    if let Some(task) = purge_task {
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
            Ok(()) => return true,
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
        .route("/api/usage", get(api_usage).post(api_usage_refresh))
        .route("/api/usage/logs", get(api_usage_logs))
        .route("/api/usage/codex/reset", post(api_codex_reset))
        .route(
            "/api/usage/codex/reset/schedule",
            put(api_codex_reset_schedule).delete(api_codex_reset_cancel),
        )
        .route("/metrics", get(prometheus_metrics))
        .route("/api/workspaces", get(api_workspaces))
        .route("/api/machines", get(api_machines))
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
    } else if state
        .remote_runtime
        .as_ref()
        .is_some_and(|runtime| !runtime.connected())
    {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "agent runtime disconnected",
        )
            .into_response()
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
    let runtime = state
        .remote_runtime
        .as_ref()
        .map(|runtime| runtime.stats())
        .unwrap_or_default();
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
        runtime_connected: state
            .remote_runtime
            .as_ref()
            .is_none_or(|runtime| runtime.connected()),
        runtime_workers: runtime.workers,
        runtime_busy_workers: runtime.busy_workers,
        runtime_draining_workers: runtime.draining_workers,
        runtime_handoff_workers: runtime.handoff_workers,
        runtime_pending_commands: runtime.pending_commands,
        code_cache_bytes: code_cache.bytes,
        code_cache_hits: code_cache.hits,
        code_cache_misses: code_cache.misses,
        code_cache_evictions: code_cache.evictions,
    })
    .into_response()
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
    let runtime = state
        .remote_runtime
        .as_ref()
        .map(|runtime| runtime.stats())
        .unwrap_or_default();
    let runtime_connected = state
        .remote_runtime
        .as_ref()
        .is_none_or(|runtime| runtime.connected());
    let mut body = format!(
        "# TYPE cowboy_up gauge\ncowboy_up {}\n# TYPE cowboy_database_bytes gauge\ncowboy_database_bytes {db_bytes}\n# TYPE cowboy_events_rows gauge\ncowboy_events_rows {events_rows}\n# TYPE cowboy_sessions gauge\ncowboy_sessions{{state=\"live\"}} {sessions_live}\ncowboy_sessions{{state=\"deleted\"}} {sessions_deleted}\n# TYPE cowboy_daemon_rss_bytes gauge\ncowboy_daemon_rss_bytes {}\n# TYPE cowboy_persistence_pending gauge\ncowboy_persistence_pending {}\n# TYPE cowboy_persistence_dropped_total counter\ncowboy_persistence_dropped_total {}\n# TYPE cowboy_persistence_failed_batches_total counter\ncowboy_persistence_failed_batches_total {}\n# TYPE cowboy_persistence_healthy gauge\ncowboy_persistence_healthy {}\n# TYPE cowboy_runtime_connected gauge\ncowboy_runtime_connected {}\n# TYPE cowboy_runtime_workers gauge\ncowboy_runtime_workers {}\n# TYPE cowboy_runtime_busy_workers gauge\ncowboy_runtime_busy_workers {}\n# TYPE cowboy_runtime_draining_workers gauge\ncowboy_runtime_draining_workers {}\n# TYPE cowboy_runtime_handoff_workers gauge\ncowboy_runtime_handoff_workers {}\n# TYPE cowboy_runtime_pending_commands gauge\ncowboy_runtime_pending_commands {}\n",
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
    );
    body.push_str("# TYPE cowboy_agent_memory_bytes gauge\n# TYPE cowboy_agent_pids gauge\n# TYPE cowboy_agent_cpu_seconds_total counter\n");
    for (session, stats) in state.supervisor.resource_stats() {
        let seconds = stats.cpu_usage_usec / 1_000_000;
        let micros = stats.cpu_usage_usec % 1_000_000;
        let _ = writeln!(
            body,
            "cowboy_agent_memory_bytes{{session=\"{session}\"}} {}\ncowboy_agent_pids{{session=\"{session}\"}} {}\ncowboy_agent_cpu_seconds_total{{session=\"{session}\"}} {seconds}.{micros:06}",
            stats.memory_bytes, stats.pids,
        );
    }
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
/// dialog: the two host roots (columbus, /etc/nixos) plus one entry per
/// columbus-managed project, read from `<workspace-root>/columbus/project-defs/*`
/// (the registry is the source of truth for which projects exist) and resolved
/// to each project's worktree. The frontend keeps a hard-coded fallback for when
/// this is unreachable.
async fn api_workspaces(State(state): State<Arc<AppState>>) -> Response {
    let columbus = state.supervisor.workspace_root().join("columbus");
    let work_items = projected_work_items(&columbus);
    let mut out = vec![
        Workspace {
            value: "columbus".to_owned(),
            label: "columbus".to_owned(),
            help: columbus.display().to_string(),
            project: None,
            active_work_items: Vec::new(),
        },
        Workspace {
            value: "/etc/nixos".to_owned(),
            label: "/etc/nixos".to_owned(),
            help: "NixOS host config".to_owned(),
            project: None,
            active_work_items: Vec::new(),
        },
    ];
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
    status: &'static str,
    local: bool,
}

/// The first multi-machine slice deliberately advertises only the operational
/// local scheduler. Enrolled outbound machines are added here only after their
/// authenticated connection and inventory have been accepted.
async fn api_machines() -> Json<Vec<MachineSummary>> {
    let display_name = std::env::var("HOSTNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "This machine".to_owned());
    Json(vec![MachineSummary {
        id: "local".to_owned(),
        display_name,
        platform: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
        status: "online",
        local: true,
    }])
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
/// WS `Inbound::NewSession` is fire-and-forget without a `sessionId` reply, so
/// an external HTTP caller would have to diff `Outbound::Sessions` broadcasts
/// to learn their id — racey. This endpoint exists so a single synchronous HTTP
/// request returns the assigned id directly. Web UI clients can keep using the
/// WS path; this is purely additive.
#[derive(Debug, Deserialize)]
struct NewSessionRequest {
    provider: String,
    /// Stable machine placement. Older clients omit it and remain local.
    #[serde(default = "default_machine_id")]
    machine_id: String,
    #[serde(default)]
    cwd: Option<String>,
    /// Which surface opened the session — defaults to `Api` for direct
    /// `curl`/test callers. The Web UI uses the WS `Inbound::NewSession` path
    /// (which always tags `Web`), not this endpoint.
    #[serde(default)]
    origin: SessionOrigin,
    /// Create a view-only machine-driven system session. Defaults false; the Web
    /// UI never sets it.
    #[serde(default)]
    system: bool,
}

fn default_machine_id() -> String {
    "local".to_owned()
}

/// Response body for `POST /api/sessions`.
#[derive(Debug, Serialize)]
struct NewSessionResponse {
    session_id: String,
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
    match state.supervisor.new_session_on(
        &req.provider,
        req.cwd,
        req.origin,
        req.system,
        &req.machine_id,
    ) {
        Ok(session_id) => {
            (StatusCode::CREATED, Json(NewSessionResponse { session_id })).into_response()
        }
        Err(message) => (StatusCode::BAD_REQUEST, message).into_response(),
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
struct CodeManifestResponse {
    api_version: u8,
    provider: &'static str,
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
    match serde_json::from_str::<ZedAdapterResponse>(&line)? {
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
    let files = tokio::task::spawn_blocking(move || {
        crate::files::search(std::path::Path::new(&cwd), &query.q, limit)
    })
    .await
    .unwrap_or_default();
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
    let files = tokio::task::spawn_blocking(move || {
        crate::code_review::LocalCodeProvider::new(cwd).search(&query.q, limit)
    })
    .await
    .unwrap_or_default();
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
    let language_ready = if let Some(socket) = &state.zed_adapter_socket {
        match ensure_zed_worktree(socket, &cwd).await {
            Ok(ready) => ready,
            Err(error) => {
                tracing::warn!(session = %session_id, %error, "Zed adapter unavailable");
                false
            }
        }
    } else {
        false
    };
    let manifest_cwd = cwd;
    let result = tokio::task::spawn_blocking(move || {
        crate::code_review::LocalCodeProvider::new(manifest_cwd).manifest()
    })
    .await;
    let Ok(Ok(manifest)) = result else {
        return (StatusCode::UNPROCESSABLE_ENTITY, "worktree unavailable").into_response();
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
    let result = tokio::task::spawn_blocking(move || {
        crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd)).changes()
    })
    .await;
    let Ok(Ok(result)) = result else {
        return (StatusCode::UNPROCESSABLE_ENTITY, "git changes unavailable").into_response();
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
        session_id,
        cwd: cwd.clone(),
        path: path.clone(),
        context: query.context,
        show_whitespace: query.show_whitespace,
        scope,
    };
    let page = state
        .diff_snapshots
        .first_page(key, || async move {
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
    let cache = state.code_cache.clone();
    let result = tokio::task::spawn_blocking(move || {
        if let Some(cached) = cache.get_or_load(FsPath::new(&cwd), &query.path)? {
            debug_assert_eq!(cached.size, cached.bytes.len() as u64);
            crate::code_review::cached_file_page(
                &query.path,
                cached.bytes,
                cached.revision,
                query.cursor.as_deref(),
            )
        } else {
            crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd))
                .file_page(&query.path, query.cursor.as_deref())
        }
    })
    .await;
    let Ok(result) = result else {
        return (StatusCode::BAD_REQUEST, "file unavailable").into_response();
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
    let Some(socket) = &state.zed_adapter_socket else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "language service unavailable",
        )
            .into_response();
    };
    if query.path.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid buffer path").into_response();
    }
    match zed_adapter_request(
        socket,
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
    let Some(socket) = &state.zed_adapter_socket else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "language service unavailable",
        )
            .into_response();
    };
    if query.path.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid buffer path").into_response();
    }
    match zed_adapter_request(
        socket,
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
    let Some(socket) = &state.zed_adapter_socket else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "language service unavailable",
        )
            .into_response();
    };
    if query.path.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid buffer path").into_response();
    }
    match zed_adapter_request(
        socket,
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
    let Some(socket) = &state.zed_adapter_socket else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "language service unavailable",
        )
            .into_response();
    };
    if query.path.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid buffer path").into_response();
    }
    match zed_adapter_request(
        socket,
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
    let Some(socket) = &state.zed_adapter_socket else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "language service unavailable",
        )
            .into_response();
    };
    if request.path.is_empty()
        || request.lease_id.is_empty()
        || request.lease_id.len() > 128
        || request.lease_id.chars().any(char::is_whitespace)
    {
        return (StatusCode::BAD_REQUEST, "invalid buffer lease").into_response();
    }
    if open && ensure_zed_worktree(socket, &cwd).await.is_err() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "language service unavailable",
        )
            .into_response();
    }
    let response = zed_adapter_request(
        socket,
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
        Inbound::NewSession { provider, cwd } => state
            .supervisor
            .new_session(&provider, cwd, SessionOrigin::Web, false)
            .map(|_| ()),
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
            if state.hub.status(&session_id) == Some(Status::Interrupted)
                && !state.hub.effective_auto_resume(&session_id)
            {
                // Interrupted + NOT opted into auto-resume → leave it for manual
                // recovery (the "last turn was interrupted" bar; submitting
                // revives it). An auto-resume session falls through to
                // ensure_alive: restore already enqueued the continuation, so
                // reviving here drains it the moment the session is opened.
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
            //    permanent delete path: agentd retains delete tombstones to
            //    reject stale launches for genuinely deleted sessions.
            state.hub.clear_agent_session_id(&session_id);
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

async fn send_json<S>(sink: &mut S, msg: &Outbound) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    sink.send(Message::Text(text.into())).await.map_err(|_| ())
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
