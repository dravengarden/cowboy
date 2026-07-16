//! HTTP / WebSocket server (design §5).
//!
//! Every frontend is an **equal subscriber** to one WebSocket stream. On
//! connect a client receives the session list plus a full snapshot of each
//! session's event log, then a live tail of all events — so "new session shows
//! everywhere" and "permission resolves everywhere" are just broadcasts. The
//! same socket carries inbound commands.
//!
//! v1 has **no auth** and binds `0.0.0.0` by deliberate choice (LAN-only use);
//! design §9 auth/pairing is a follow-up.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Json, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::Router;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use agent_client_protocol::schema::v1::ContentBlock;

use crate::acp::AgentCommand;
use crate::cli::ServeArgs;
use crate::core::{
    DispatchReq, Envelope, Event, Hub, Inbound, JudgeReq, Outbound, PersistenceHealth,
    RestoredSession, SessionOrigin, Status, StoreSink, StoreWrite,
};
use crate::persistence::EventReducer;
use crate::remote_runtime::{RemoteBootstrap, RemoteRuntime};
use crate::runtime::RuntimeHealth;
use crate::runtime_wire::StartSession;
use crate::store::Store;
use crate::supervisor::Supervisor;
use crate::usage::UsageService;
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
}

const STORE_QUEUE_CAPACITY: usize = 8_192;
const FORCE_CANCEL_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Start the HTTP/WebSocket server and the agent supervisor.
pub async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    init_tracing();
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
        },
        shutdown_tx,
    )
    .await;
    judge_task.abort();
    scheduler_task.abort();
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
        StoreWrite::SetAgentSessionId {
            session_id,
            agent_session_id,
        } => {
            store
                .update_agent_session_id(session_id, agent_session_id)
                .await
        }
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
        assert!(reducer
            .reduce(update(
                30,
                serde_json::json!({"sessionUpdate": "usage_update", "used": 1, "size": 2}),
            ))
            .is_none());
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
    state.supervisor.send(session_id, AgentCommand::Cancel)?;
    let hub = state.hub.clone();
    let supervisor = Arc::clone(&state.supervisor);
    let session_id = session_id.to_owned();
    tokio::spawn(async move {
        tokio::time::sleep(FORCE_CANCEL_GRACE).await;
        let Some(stuck_status @ (Status::Busy | Status::Starting)) = hub.status(&session_id) else {
            return;
        };
        if !hub.set_status_if_current(
            &session_id,
            Some(stuck_status),
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
        .route("/metrics", get(prometheus_metrics))
        .route("/api/workspaces", get(api_workspaces))
        .route("/api/sessions", post(api_new_session))
        .route("/api/sessions/{id}/files", get(api_search_files))
        .route("/api/sessions/{id}/info", get(api_session_info))
        .route("/api/sessions/{id}/prompt", post(api_session_prompt))
        .route("/api/history/{id}", get(api_history))
        .route("/api/artifacts/{name}", get(api_artifact))
        .route("/ws", any(ws_upgrade))
        // Everything else: the separately deployed SPA, with index.html
        // fallback for client-side routes.
        .fallback(static_handler)
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
            stats.memory_bytes,
            stats.pids,
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

/// Resolve a registered project's session cwd from its on-disk layout:
/// `projects/<name>/main` for an external worktree, else `projects/<name>`
/// itself for a subdir project. `None` for a registered-but-not-checked-out
/// project (no `main` worktree and nothing on disk but a bare clone).
fn project_worktree(columbus: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let proj = columbus.join("projects").join(name);
    let main = proj.join("main");
    if main.is_dir() {
        return Some(main);
    }
    // Subdir project: the dir itself holds the checkout. Skip a dir that is
    // only a bare clone (`.bare` with no worktree, e.g. an uncloned external).
    let has_content = std::fs::read_dir(&proj)
        .ok()?
        .filter_map(Result::ok)
        .any(|e| e.file_name() != ".bare");
    has_content.then_some(proj)
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

/// Response body for `POST /api/sessions`.
#[derive(Debug, Serialize)]
struct NewSessionResponse {
    session_id: String,
}

async fn api_new_session(
    State(state): State<Arc<AppState>>,
    Json(req): Json<NewSessionRequest>,
) -> Response {
    match state
        .supervisor
        .new_session(&req.provider, req.cwd, req.origin, req.system)
    {
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

#[derive(Debug, Serialize)]
struct HistoryResponse {
    events: Vec<Envelope>,
    next_before_seq: Option<u64>,
    reached_start: bool,
}

#[derive(Debug, Deserialize)]
struct HistoryQuery {
    before_seq: u64,
}

/// One seq-aligned page of a session's history (events `[k·HISTORY_PAGE,
/// (k+1)·HISTORY_PAGE)`). The client pages UP from the WS tail; older pages
/// arrive here. A COMPLETE past page never changes again, so it's served
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
            store
                .history_page(&session_id, query.before_seq, crate::core::HISTORY_PAGE)
                .await
        }
        None => Ok(state
            .hub
            .history_page(&session_id, query.before_seq)
            .unwrap_or_default()),
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
    {
        if inm.contains(etag.as_str()) {
            return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag.as_str())]).into_response();
        }
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

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: Arc<AppState>) {
    let (mut sink, mut stream) = socket.split();

    // Subscribe BEFORE snapshotting so no event slips through the gap; the
    // client dedups by (session_id, seq), so a brief overlap is harmless.
    let mut rx = state.hub.subscribe();
    let mut shutdown = state.shutdown.clone();

    if send_json(
        &mut sink,
        &Outbound::Sessions {
            sessions: state.hub.session_list(),
        },
    )
    .await
    .is_err()
    {
        return;
    }
    // Seed the global settings (auto-resume default + continuation template) so
    // the client's Settings UI + per-session badge render on first paint.
    if send_json(
        &mut sink,
        &Outbound::Settings {
            settings: state.hub.settings_snapshot(),
        },
    )
    .await
    .is_err()
    {
        return;
    }
    // Seed the static skill registry (prompt + extract) for the Info sheet.
    if send_json(
        &mut sink,
        &Outbound::Skills {
            skills: state.hub.skills_snapshot(),
        },
    )
    .await
    .is_err()
    {
        return;
    }
    // Seed every optimistic-sync state (@shared-utils/sync): one absolute
    // snapshot patch per state mutated this lifetime, so each of this client's
    // sync clients starts at the arbiter's version and folds any live overrides.
    for patch in state.hub.sync_resync() {
        if send_json(&mut sink, &patch).await.is_err() {
            return;
        }
    }
    for meta in state.hub.session_list() {
        if let Some((events, reached_start)) = state.hub.snapshot(&meta.id) {
            if send_json(
                &mut sink,
                &Outbound::Snapshot {
                    session_id: meta.id.clone(),
                    events,
                    reached_start,
                },
            )
            .await
            .is_err()
            {
                return;
            }
        }
        // Replay the last-seen agent config options so the composer's
        // mode / model / effort dropdowns hydrate on first paint instead of
        // waiting for the next `config_option_update` to fire upstream.
        if let Some(options) = state.hub.config_options(&meta.id) {
            if send_json(
                &mut sink,
                &Outbound::ConfigOptions {
                    session_id: meta.id.clone(),
                    options,
                },
            )
            .await
            .is_err()
            {
                return;
            }
        }
        // Replay the server-authoritative queue + drafts as a resync SyncPatch
        // (state "queue:<id>") so a freshly-opened terminal renders the same
        // staged messages and adopts them across a daemon restart.
        if let Some(patch) = state.hub.queue_resync(&meta.id) {
            if send_json(&mut sink, &patch).await.is_err() {
                return;
            }
        }
        // Seed the confirm-detect judge-run history so the inspector widget
        // (long-press the turn-status pill) hydrates with the persisted runs on
        // first paint instead of waiting for the next turn-end judge.
        if send_json(
            &mut sink,
            &Outbound::JudgeHistory {
                session_id: meta.id.clone(),
                runs: state.hub.judge_history(&meta.id),
            },
        )
        .await
        .is_err()
        {
            return;
        }
    }

    if send_json(&mut sink, &Outbound::BootstrapComplete)
        .await
        .is_err()
    {
        return;
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
    if let Some(sid) = &session_id_for_err {
        if matches!(&cmd, Inbound::Prompt { .. } | Inbound::Submit { .. })
            && state.hub.session_is_system(sid)
        {
            state.hub.broadcast_error(
                Some(sid.clone()),
                "view-only system session: input is disabled".to_owned(),
            );
            return;
        }
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
            if result.is_ok() {
                if let Some(title) = auto {
                    state.hub.auto_title(&session_id, title);
                }
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
            state.hub.resume_turn(&session_id);
            Ok(())
        }
        Inbound::RetryTurn { session_id } => {
            state.hub.retry_turn(&session_id);
            Ok(())
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
            // 2. Drop the timeline divider marker (kept history, agent forgets).
            // 3. Atomically fence + replace the worker. This must not use the
            //    permanent delete path: agentd retains delete tombstones to
            //    reject stale launches for genuinely deleted sessions.
            state.hub.clear_agent_session_id(&session_id);
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
