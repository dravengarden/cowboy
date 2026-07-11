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
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use agent_client_protocol::schema::v1::ContentBlock;

use crate::acp::AgentCommand;
use crate::cli::ServeArgs;
use crate::core::{
    DispatchReq, Envelope, Hub, Inbound, Outbound, PersistenceHealth, RestoredSession,
    SessionOrigin, Status, StoreSink, StoreWrite,
};
use crate::persistence::EventReducer;
use crate::store::Store;
use crate::supervisor::Supervisor;
use tokio::sync::{mpsc, watch};

struct AppState {
    hub: Hub,
    supervisor: Arc<Supervisor>,
    /// Kept for read-only storage metrics (`/api/metrics`). `None` in-memory.
    store: Option<Store>,
    /// The memory debounce queue — the enqueue handle the `/api/memory/*`
    /// handlers reach. `None` when `--memory-enabled` is off (the endpoints then
    /// 404), so nothing memory-related runs by default.
    memory_queue: Option<Arc<crate::memory::Queue>>,
    persistence_health: Option<Arc<PersistenceHealth>>,
    shutdown: watch::Receiver<bool>,
}

const STORE_QUEUE_CAPACITY: usize = 8_192;

/// Start the HTTP/WebSocket server and the agent supervisor.
pub async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    init_tracing();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Phase 2: when --postgres-url is supplied, hook in the persistent store.
    // Migrations run on every start (sqlx tracks applied versions, so it's
    // idempotent); the in-memory Hub is then warmed from the DB before WS
    // clients can connect. Without --postgres-url the daemon falls back to
    // pure in-memory mode — same behaviour as before, useful for dev or for
    // running on a host that doesn't have the cowboy-private postgres yet.
    let (hub, store, persistence_health, writer_task) =
        if let Some(url) = args.postgres_url.as_deref() {
            let store = Store::connect(url).await.context("connecting postgres")?;
            store.migrate().await.context("running migrations")?;
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
            // Inference provider configs + keys (the judge reads the key from memory).
            let inf_cfg = store.load_inference_config().await.unwrap_or_else(|e| {
                tracing::warn!(error = %e, "loading inference config");
                Vec::new()
            });
            let inf_keys = store.load_inference_secrets().await.unwrap_or_else(|e| {
                tracing::warn!(error = %e, "loading inference secrets");
                Vec::new()
            });
            hub.load_inference(inf_cfg, inf_keys);
            hub.restore(restored);
            tracing::info!(
                postgres = url,
                restored = restored_count,
                "persistence wired",
            );
            // Background DB writer: dequeues StoreWrite intents and applies them.
            // Errors are logged but don't bring the daemon down — the in-memory
            // state remains authoritative for the current process.
            let writer_task = tokio::spawn(run_store_writer(
                store.clone(),
                rx,
                Arc::clone(&health),
                shutdown_rx.clone(),
            ));
            // Background sweeper: hard-delete sessions soft-deleted past the
            // retention window, reclaiming their event storage.
            tokio::spawn(run_purge_sweeper(store.clone()));
            (hub, Some(store), Some(health), Some(writer_task))
        } else {
            tracing::info!("no --postgres-url: running in-memory only");
            (Hub::new(), None, None, None)
        };
    let supervisor = Arc::new(Supervisor::new(hub.clone(), args.workspace_root.clone()));

    // Background dispatcher: the Hub owns each session's send-queue but can't
    // call the Supervisor (which holds the Hub) — that cycle is why the queue
    // used to live client-side. The Hub now makes the drain decision under its
    // lock and hands each ready prompt over this channel; we send it to the
    // agent here, off the lock. Wired before any client connects.
    let (dispatch_tx, dispatch_rx) = mpsc::unbounded_channel::<DispatchReq>();
    hub.set_dispatch_tx(dispatch_tx);
    tokio::spawn(run_dispatcher(
        hub.clone(),
        Arc::clone(&supervisor),
        dispatch_rx,
    ));

    // Honor agent `ScheduleWakeup`s: fires a wake-prompt (via the same dispatch
    // path) at the scheduled time. Without this, an ACP-driven agent's scheduled
    // self-checks never fire and get consumed by the next user turn instead.
    let (sched_tx, sched_rx) = mpsc::unbounded_channel::<crate::scheduler::ScheduleCmd>();
    hub.set_scheduler_tx(sched_tx);
    tokio::spawn(crate::scheduler::run_scheduler(hub.clone(), sched_rx));
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

    // Memory subsystem (p5). GATED: when --memory-enabled is off this whole
    // block is skipped, so the daemon's behaviour is byte-for-byte unchanged
    // (no store, no janitor session, no queue, no endpoints).
    let memory_queue = if args.memory_enabled {
        match setup_memory(&args, hub.clone(), Arc::clone(&supervisor)) {
            Ok(q) => Some(q),
            Err(e) => {
                // A memory-setup failure must NOT take the daemon down — the
                // core chat surface stays up; memory simply stays inert.
                tracing::error!(error = %e, "memory subsystem failed to start (disabled for this run)");
                None
            }
        }
    } else {
        None
    };

    tracing::info!(
        workspace = %args.workspace_root.display(),
        data_dir = %args.data_dir.display(),
        memory_enabled = args.memory_enabled,
        "cowboy serving",
    );

    let result = serve_axum(
        args.bind,
        AppState {
            hub,
            supervisor,
            store,
            memory_queue,
            persistence_health,
            shutdown: shutdown_rx,
        },
        shutdown_tx,
    )
    .await;
    if let Some(task) = writer_task {
        match tokio::time::timeout(std::time::Duration::from_secs(10), task).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!(%error, "store writer task failed during shutdown"),
            Err(_) => tracing::error!("store writer did not drain within shutdown deadline"),
        }
    }
    result
}

/// Stand up the memory subsystem and return the enqueue handle (the Queue) for
/// the `/api/memory/*` handlers. Idempotent w.r.t. the janitor session: reuses an
/// existing system session rooted at the janitor cwd if one was restored, else
/// creates one. Spawns the reconcile loop (queue → debounced batch → janitor) and
/// the tidy timer. Returns the Queue or an error if the janitor session can't be
/// ensured.
fn setup_memory(
    args: &ServeArgs,
    hub: Hub,
    supervisor: Arc<Supervisor>,
) -> anyhow::Result<Arc<crate::memory::Queue>> {
    use crate::memory::{Index, Janitor, Queue, QueueConfig, Store as MemStore};

    let root = expand_tilde(&args.memory_root);
    let store = MemStore::new(root.clone());
    store.ensure_git_repo().context("init memory git repo")?;

    // The janitor session's cwd + the marker used to find/reuse its restored
    // system session: the memory root itself. (It used to be ~/mnemosyne, but
    // that path matched stale pre-fold janitor sessions — a claude-code one got
    // picked as the "codex janitor" and its revived turn hung past the timeout,
    // and ~/mnemosyne also carries an obsolete memory-tidy skill codex fails to
    // load. The store root has neither.)
    let janitor_cwd_str = root.display().to_string();

    // Reuse a restored memory-janitor system session — flagged `system`, at the
    // janitor cwd, AND of the configured provider. The provider check is what
    // stops a stale WRONG-provider session from being mistaken for the janitor
    // (the bug above). Else create one. Hub::restore brings persisted system
    // sessions back across a daemon restart, so without reuse we'd spawn a second
    // janitor every restart.
    let session_id = hub
        .session_list()
        .into_iter()
        .find(|m| {
            m.system && m.cwd == janitor_cwd_str && m.provider == args.memory_janitor_provider
        })
        .map(|m| m.id)
        .map_or_else(
            || {
                supervisor
                    .new_session(
                        &args.memory_janitor_provider,
                        Some(janitor_cwd_str.clone()),
                        SessionOrigin::Api,
                        true, // system: view-only janitor session
                    )
                    .map_err(|e| anyhow::anyhow!("creating memory-janitor session: {e}"))
            },
            Ok,
        )?;
    tracing::info!(session = %session_id, cwd = %janitor_cwd_str, provider = %args.memory_janitor_provider, "memory janitor session");

    // Build the initial recall/dedup index from whatever the store already holds.
    let index = Index::from_store(&store).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "building initial memory index (starting empty)");
        Index::new(Vec::new())
    });
    let janitor = Janitor {
        supervisor,
        store,
        session_id,
        index: Arc::new(parking_lot::Mutex::new(index)),
    };

    // Cross the sync→async boundary: the Queue's `wake` is a SYNC callback, but
    // the janitor drive is async. The callback just forwards the coalesced batch
    // over an mpsc channel; an async task awaits it and runs the janitor. (The
    // wake fires under the queue lock and MUST NOT block — an unbounded send is
    // non-blocking, so this is safe.)
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<crate::memory::Mutation>>();
    let queue = Arc::new(Queue::new(QueueConfig::default_config(), move |batch| {
        if tx.send(batch).is_err() {
            tracing::warn!("memory janitor channel closed; dropping batch");
        }
    }));

    // The reconcile loop: one debounced batch per wake → run the janitor.
    let reconcile_janitor = janitor.clone();
    tokio::spawn(async move {
        while let Some(batch) = rx.recv().await {
            let mut applied = false;
            for attempt in 0..3 {
                if reconcile_janitor.run_janitor(batch.clone()).await {
                    applied = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
            }
            if !applied {
                tracing::error!(
                    mutations = batch.len(),
                    "memory batch exhausted retries; leaving unwritten"
                );
            }
        }
        tracing::info!("memory reconcile loop shutting down (channel closed)");
    });

    // The tidy timer: a conservative scheduled survey/soft-archive pass every
    // 12h (mirrors run_purge_sweeper). The first tick fires immediately; skip it
    // so a fresh daemon doesn't tidy an empty store on boot.
    let tidy_janitor = janitor;
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(12 * 60 * 60));
        tick.tick().await; // immediate first tick — consume it (no boot tidy)
        loop {
            tick.tick().await;
            let _ = tidy_janitor.tidy().await;
        }
    });

    Ok(queue)
}

/// Expand a leading `~` / `~/...` in a path to `$HOME`. cowboy has no `dirs`
/// dep, and the default `--memory-root` is `~/.agents/memory`, which clap stores
/// verbatim (no shell expansion for an env/default value).
fn expand_tilde(p: &std::path::Path) -> std::path::PathBuf {
    let Ok(home) = std::env::var("HOME") else {
        return p.to_path_buf();
    };
    let s = p.to_string_lossy();
    if s == "~" {
        std::path::PathBuf::from(home)
    } else if let Some(rest) = s.strip_prefix("~/") {
        std::path::Path::new(&home).join(rest)
    } else {
        p.to_path_buf()
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
        StoreWrite::PutInferenceConfig {
            provider,
            model,
            params,
        } => store.put_inference_config(provider, model, params).await,
        StoreWrite::PutInferenceSecret { provider, api_key } => {
            store.put_inference_secret(provider, api_key).await
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

/// Drain the Hub→dispatcher channel: each [`DispatchReq`] is a queued prompt the
/// Hub decided is ready to send. We forward it to the session's agent. On
/// success, derive the auto-title from the first prompt (a no-op after the
/// first); on failure, clear the in-flight guard (so the queue can keep
/// draining) and surface the error to every client.
async fn run_dispatcher(
    hub: Hub,
    supervisor: Arc<Supervisor>,
    mut rx: mpsc::UnboundedReceiver<DispatchReq>,
) {
    while let Some(req) = rx.recv().await {
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
        .route("/api/workspaces", get(api_workspaces))
        .route("/api/sessions", post(api_new_session))
        .route("/api/sessions/{id}/files", get(api_search_files))
        .route("/api/sessions/{id}/info", get(api_session_info))
        .route("/api/sessions/{id}/prompt", post(api_session_prompt))
        .route("/api/memory/record", post(api_memory_record))
        .route("/api/memory/forget", post(api_memory_forget))
        .route("/api/history/{id}/{page}", get(api_history))
        .route("/ws", any(ws_upgrade))
        // Everything else: the embedded SPA, with index.html fallback for
        // client-side routes.
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
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

async fn healthz(State(state): State<Arc<AppState>>) -> Response {
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

/// A build identifier the SPA polls to detect a redeploy. We reuse the embedded
/// `index.html`'s compile-time SHA256: it references the content-hashed JS/CSS
/// bundles, so any change to the shipped UI changes this hash, while an
/// unchanged build keeps it stable. The frontend captures it on first load and
/// re-checks after each WS reconnect — a mismatch means the daemon was
/// redeployed under a now-stale tab, which surfaces the "new version" banner.
async fn version() -> Response {
    match Assets::get("index.html") {
        Some(f) => Json(VersionResponse {
            version: content_hash_hex(&f.metadata.sha256_hash()),
        })
        .into_response(),
        None => (StatusCode::NOT_FOUND, "UI not built").into_response(),
    }
}

/// Render rust-embed's compile-time SHA256 as a 32-hex-char string (first 16
/// bytes — ample to avoid collisions across a handful of static files). Used
/// both for the static-asset `ETag` and the `/version` build id so the two
/// never drift.
fn content_hash_hex(hash: &[u8; 32]) -> String {
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
    })
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
    let mut out = vec![
        Workspace {
            value: "columbus".to_owned(),
            label: "columbus".to_owned(),
            help: columbus.display().to_string(),
        },
        Workspace {
            value: "/etc/nixos".to_owned(),
            label: "/etc/nixos".to_owned(),
            help: "NixOS host config".to_owned(),
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
                    label: name,
                    help: path,
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
/// mnemosyne daemon POSTs a batch here to drive the (view-only) memory session;
/// this bypasses the WS user-input gate precisely because it is the backend, not
/// a user. Works on any session, including `system` ones.
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

/// Request body for `POST /api/memory/record` — a validated memory WRITE
/// proposal from `cowboy mem record` (the CLI validates frontmatter first). The
/// daemon enqueues it to the debounce queue; the janitor dedups/judges/commits.
#[derive(Debug, Deserialize)]
struct MemoryRecordRequest {
    name: String,
    description: String,
    #[serde(rename = "type")]
    mem_type: String,
    /// Target tier slug (a project cwd-slug); omitted/empty → the machine tier.
    #[serde(default)]
    tier: Option<String>,
    #[serde(default)]
    body: String,
}

/// Enqueue an add proposal. Gated on `--memory-enabled`: a 404 when off (so the
/// CLI prints a clear "is memory enabled?" message and nothing memory-related
/// runs by default). Returns `{ok:true}` on enqueue.
async fn api_memory_record(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MemoryRecordRequest>,
) -> Response {
    let Some(queue) = state.memory_queue.as_ref() else {
        return (StatusCode::NOT_FOUND, "memory subsystem disabled").into_response();
    };
    let Some(mem_type) = crate::memory::MemoryType::parse(&req.mem_type) else {
        return (
            StatusCode::BAD_REQUEST,
            format!("unknown type {:?}", req.mem_type),
        )
            .into_response();
    };
    let mutation = crate::memory::Mutation {
        op: crate::memory::Op::Add,
        memory: crate::memory::Memory {
            name: req.name,
            description: req.description,
            mem_type,
            body: req.body,
        },
        slug: req.tier.unwrap_or_default(),
    };
    queue.enqueue(mutation);
    Json(serde_json::json!({ "ok": true })).into_response()
}

/// Request body for `POST /api/memory/forget` — a soft-archive proposal.
#[derive(Debug, Deserialize)]
struct MemoryForgetRequest {
    name: String,
    /// Target tier slug; omitted/empty → the machine tier.
    #[serde(default)]
    tier: Option<String>,
}

/// Enqueue a delete (soft-archive) proposal. Gated like `record`.
async fn api_memory_forget(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MemoryForgetRequest>,
) -> Response {
    let Some(queue) = state.memory_queue.as_ref() else {
        return (StatusCode::NOT_FOUND, "memory subsystem disabled").into_response();
    };
    if req.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "name is required").into_response();
    }
    // A Delete mutation carries only the name (apply soft-archives by name); the
    // other Memory fields are unused on the delete path, so minimal placeholders.
    let mutation = crate::memory::Mutation {
        op: crate::memory::Op::Delete,
        memory: crate::memory::Memory {
            name: req.name,
            description: String::new(),
            mem_type: crate::memory::MemoryType::Reference,
            body: String::new(),
        },
        slug: req.tier.unwrap_or_default(),
    };
    queue.enqueue(mutation);
    Json(serde_json::json!({ "ok": true })).into_response()
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
    /// Create a VIEW-ONLY machine-driven system session (the mnemosyne memory
    /// janitor). Defaults false; the Web UI never sets it.
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
    Path((session_id, page)): Path<(String, usize)>,
) -> Response {
    if state.hub.session_info(&session_id).is_none() {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    let history = match &state.store {
        Some(store) => {
            store
                .history_page(&session_id, page, crate::core::HISTORY_PAGE)
                .await
        }
        None => Ok(state
            .hub
            .history_page(&session_id, page)
            .unwrap_or_default()),
    };
    let (events, immutable) = match history {
        Ok(page) => page,
        Err(e) => {
            tracing::warn!(session = %session_id, page, error = %e, "history query failed");
            return (StatusCode::SERVICE_UNAVAILABLE, "history unavailable").into_response();
        }
    };
    let cache = if immutable {
        "public, max-age=31536000, immutable"
    } else {
        "no-store"
    };
    (
        [(header::CACHE_CONTROL, cache)],
        Json(HistoryResponse { events }),
    )
        .into_response()
}

/// The built web UI (Vite output), embedded at compile time. The flake builds
/// `web/dist` with deno before the cargo build so this folder exists.
#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

/// Serve an embedded asset by path, falling back to `index.html` so the SPA
/// owns client-side routing. Missing `index.html` (UI not built) → 404.
///
/// Caching: rust-embed computes a per-file SHA256 at compile time, which we use
/// as a content `ETag` (stable across rebuilds when the bytes are unchanged). The
/// cache policy is split by whether the filename is content-addressed:
///   - `/assets/*` — Vite emits content-hashed names, so the bytes behind a name
///     never change → `immutable` with a one-year max-age, never revalidated.
///   - everything else (index.html, sw.js, manifest, favicon, icons) —
///     `no-cache`: the browser may store it but MUST revalidate via the `ETag` on
///     every use, so a redeploy is picked up immediately while unchanged files
///     cost only a 304. This is what stops a redeployed favicon/icon from being
///     pinned to a stale copy in the browser's HTTP cache.
async fn static_handler(uri: Uri, headers: HeaderMap) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let requested = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };

    // Serve the asset if it exists; otherwise fall back to index.html so the
    // SPA handles the route. The mime is keyed off the *served* name so the
    // fallback is delivered as text/html, not octet-stream.
    let (name, file) = match Assets::get(requested) {
        Some(f) => (requested, Some(f)),
        None => ("index.html", Assets::get("index.html")),
    };
    let Some(content) = file else {
        return (StatusCode::NOT_FOUND, "UI not built").into_response();
    };

    // Content ETag from rust-embed's compile-time SHA256 (same hash the
    // `/version` build id uses, so the two stay in lockstep).
    let etag = format!("\"{}\"", content_hash_hex(&content.metadata.sha256_hash()));

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
        Body::from(content.data),
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
    // Seed inference-provider config (model + key_set, never the key).
    if send_json(
        &mut sink,
        &Outbound::InferenceConfig {
            providers: state.hub.inference_snapshot(),
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
        | Inbound::SetSetting { .. }
        | Inbound::SetInferenceConfig { .. }
        | Inbound::SetInferenceSecret { .. }
        | Inbound::InferenceProbe { .. } => None,
    };
    // A view-only system session (the mnemosyne memory janitor) rejects
    // user-driven turns; only the backend wake endpoint
    // (POST /api/sessions/{id}/prompt) drives it.
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
        Inbound::SetInferenceConfig {
            provider,
            model,
            params,
        } => {
            state.hub.set_inference_config(provider, model, params);
            Ok(())
        }
        Inbound::SetInferenceSecret { provider, api_key } => {
            state.hub.set_inference_secret(provider, api_key);
            Ok(())
        }
        Inbound::InferenceProbe { provider, prompt } => {
            // The probe is a network call — run it off the command path and
            // broadcast the result when it returns (the judge will be async too).
            let hub = state.hub.clone();
            tokio::spawn(async move {
                let result = inference_probe(&hub, &provider, &prompt).await;
                hub.broadcast(result);
            });
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
            // 1. Tear the current agent down (Cancel + drop its sender) so it
            //    stops and frees its slot before we respawn.
            // 2. Forget the resumable agent id so the respawn does a FRESH
            //    session/new (clean context) instead of session/load.
            // 3. Drop the timeline divider marker (kept history, agent forgets).
            // 4. Respawn a fresh agent so it's warm and ready to type into.
            state.supervisor.delete_session(&session_id);
            state.hub.clear_agent_session_id(&session_id);
            state.hub.mark_context_cleared(&session_id);
            match state.supervisor.ensure_alive(&session_id) {
                Ok(_) => Ok(()),
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
                    state.supervisor.send(&session_id, AgentCommand::Cancel)
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
                state.supervisor.send(&session_id, AgentCommand::Cancel)
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

/// DEV probe: call the inference provider once and shape the result message
/// (text + cache token counts, or an error). Proves the key/model/HTTP wiring.
async fn inference_probe(
    hub: &crate::core::Hub,
    provider: &str,
    prompt: &str,
) -> crate::core::Outbound {
    use crate::core::Outbound::InferenceProbeResult;
    use crate::inference::{
        deepseek::DeepSeek, CompleteRequest, InferenceProvider as _, Message as IMsg,
    };
    let err = |e: String| InferenceProbeResult {
        provider: provider.to_owned(),
        ok: false,
        text: String::new(),
        cache_hit: 0,
        cache_miss: 0,
        error: Some(e),
    };
    if provider != "deepseek" {
        return err(format!("unknown provider {provider}"));
    }
    let Some(key) = hub.inference_key(provider) else {
        return err("no API key set".to_owned());
    };
    let model = hub
        .inference_model(provider)
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| crate::inference::deepseek::DEFAULT_MODEL.to_owned());
    let ds = DeepSeek::new(key, model);
    let prompt = if prompt.is_empty() {
        "Reply with a JSON object {\"ok\": true}."
    } else {
        prompt
    };
    let req = CompleteRequest::json_judge(vec![IMsg::user(prompt)], 64);
    match ds.complete(req).await {
        Ok(r) => {
            tracing::info!(
                target: "cowboy::inference",
                provider,
                cache_hit = r.usage.cache_hit_tokens,
                cache_miss = r.usage.cache_miss_tokens,
                "inference probe ok"
            );
            InferenceProbeResult {
                provider: provider.to_owned(),
                ok: true,
                text: r.text,
                cache_hit: r.usage.cache_hit_tokens,
                cache_miss: r.usage.cache_miss_tokens,
                error: None,
            }
        }
        Err(e) => err(e.to_string()),
    }
}
