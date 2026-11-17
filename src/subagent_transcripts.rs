//! Observed subagent transcripts (docs/subagents.md#observed-transcripts).
//!
//! A worker forwards each subagent's own ACP updates as
//! [`SUBAGENT_EVENT_UPDATE`] runtime updates of the parent session. They never
//! enter the parent timeline: [`intercept`] hands them to one bounded,
//! write-behind writer, which fences replays, coalesces streamed chunks of one
//! message into one row, bounds every row and child, and records gaps instead
//! of waiting. Everything here is presentation: a full queue, a slow store or
//! a failed write loses observed rows, never agent work.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::runtime_wire::{RuntimeEvent, SUBAGENT_EVENT_UPDATE};
use crate::store::Store;
use crate::store::subagent_events::ObservedRow;

/// Observed updates waiting for the writer; more are dropped and counted.
const QUEUE_CAPACITY: usize = 8192;
/// Rows written per store transaction.
const BATCH_ROWS: usize = 256;
/// How long the writer waits to fill a batch.
const BATCH_WINDOW: Duration = Duration::from_millis(50);
/// Serialized bytes one stored row may hold; larger updates are trimmed.
pub const MAX_ROW_BYTES: usize = 128 * 1024;
/// Rows kept per child; later updates are counted in one gap row.
pub const MAX_CHILD_ROWS: i64 = 20_000;
/// Children whose tail the writer keeps in memory before forgetting all.
const CACHED_CHILDREN: usize = 4096;
/// The row kind recording observed updates that were not kept.
pub const GAP_UPDATE: &str = "cowboy_observation_gap";

/// One observed child update, as the Controller received it.
#[derive(Debug, Clone, PartialEq)]
pub struct Observed {
    pub session_id: String,
    pub worker_epoch: String,
    pub runtime_seq: u64,
    pub child_id: String,
    pub update: Value,
    /// Updates of this child the worker dropped before this one.
    pub dropped: u64,
}

struct Sink {
    tx: mpsc::Sender<Observed>,
    /// `(session, child)` → updates dropped at this queue, reported with the
    /// child's next accepted update.
    dropped: Mutex<HashMap<(String, String), u64>>,
}

static SINK: OnceLock<Sink> = OnceLock::new();

/// Start the writer for this process. Without a store (in-memory mode)
/// observed updates are discarded.
pub fn spawn_writer(store: Store) {
    let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
    if SINK
        .set(Sink {
            tx,
            dropped: Mutex::default(),
        })
        .is_ok()
    {
        tokio::spawn(run_writer(store, rx));
    }
}

/// Divert an observed child update away from the parent timeline. Returns the
/// event unchanged when it is anything else.
#[must_use]
pub fn intercept(
    session_id: &str,
    worker_epoch: &str,
    runtime_seq: u64,
    event: RuntimeEvent,
) -> Option<RuntimeEvent> {
    let observed = match &event {
        RuntimeEvent::Update { update, .. }
            if update.get("sessionUpdate").and_then(Value::as_str)
                == Some(SUBAGENT_EVENT_UPDATE) =>
        {
            parse(session_id, worker_epoch, runtime_seq, update)
        }
        _ => return Some(event),
    };
    if let (Some(observed), Some(sink)) = (observed, SINK.get()) {
        offer(sink, observed);
    }
    None
}

fn parse(
    session_id: &str,
    worker_epoch: &str,
    runtime_seq: u64,
    update: &Value,
) -> Option<Observed> {
    let child_id = update
        .get("childSessionId")
        .and_then(Value::as_str)
        .filter(|child| crate::transcript_anchors::valid_id(child))?;
    let inner = update.get("update").filter(|inner| inner.is_object())?;
    inner.get("sessionUpdate").and_then(Value::as_str)?;
    Some(Observed {
        session_id: session_id.to_owned(),
        worker_epoch: worker_epoch.to_owned(),
        runtime_seq,
        child_id: child_id.to_owned(),
        update: inner.clone(),
        dropped: update.get("dropped").and_then(Value::as_u64).unwrap_or(0),
    })
}

fn offer(sink: &Sink, mut observed: Observed) {
    let key = (observed.session_id.clone(), observed.child_id.clone());
    let mut dropped = sink.dropped.lock();
    if let Some(count) = dropped.remove(&key) {
        observed.dropped = observed.dropped.saturating_add(count);
    }
    if let Err(error) = sink.tx.try_send(observed) {
        let observed = match error {
            mpsc::error::TrySendError::Full(observed)
            | mpsc::error::TrySendError::Closed(observed) => observed,
        };
        // Bounded too: a child that floods a stalled writer keeps one counter.
        if dropped.len() < CACHED_CHILDREN || dropped.contains_key(&key) {
            *dropped.entry(key).or_default() += observed.dropped.saturating_add(1);
        }
    }
}

/// The writer's view of one child's newest row.
#[derive(Debug, Clone, Default)]
struct ChildState {
    next_seq: i64,
    rows: i64,
    capped: bool,
    /// The newest row, kept while further chunks may extend it.
    open: Option<ObservedRow>,
}

/// The write-behind writer's state; one per process (tests make their own).
#[derive(Default)]
pub(crate) struct Writer {
    children: HashMap<(String, String), ChildState>,
    highwaters: HashMap<(String, String), i64>,
}

async fn run_writer(store: Store, mut rx: mpsc::Receiver<Observed>) {
    let mut writer = Writer::default();
    while let Some(first) = rx.recv().await {
        let mut batch = vec![first];
        let deadline = tokio::time::Instant::now() + BATCH_WINDOW;
        while batch.len() < BATCH_ROWS {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(next)) => batch.push(next),
                Ok(None) | Err(_) => break,
            }
        }
        writer.apply(&store, batch).await;
    }
}

impl Writer {
    pub(crate) async fn apply(&mut self, store: &Store, batch: Vec<Observed>) {
        let mut rows: Vec<ObservedRow> = Vec::new();
        let mut touched = HashSet::new();
        for observed in batch {
            let fence = (observed.session_id.clone(), observed.worker_epoch.clone());
            let highwater = match self.highwaters.get(&fence) {
                Some(highwater) => *highwater,
                None => match store
                    .subagent_events_highwater(&observed.session_id, &observed.worker_epoch)
                    .await
                {
                    Ok(highwater) => highwater.unwrap_or(0),
                    Err(error) => {
                        tracing::warn!(%error, "reading observed subagent highwater");
                        continue;
                    }
                },
            };
            let runtime_seq = i64::try_from(observed.runtime_seq).unwrap_or(i64::MAX);
            self.highwaters.insert(fence, highwater.max(runtime_seq));
            if runtime_seq <= highwater {
                continue; // A replay after a Controller restart.
            }
            let key = (observed.session_id.clone(), observed.child_id.clone());
            if !self.children.contains_key(&key) {
                let Some(state) = load_child(store, &observed).await else {
                    continue;
                };
                if self.children.len() >= CACHED_CHILDREN {
                    self.children.clear();
                }
                self.children.insert(key.clone(), state);
            }
            let Some(state) = self.children.get_mut(&key) else {
                continue;
            };
            for row in fold(state, observed, chrono::Utc::now().timestamp_millis()) {
                touched.insert(key.clone());
                rows.retain(|kept| {
                    (&kept.session_id, &kept.child_id, kept.seq)
                        != (&row.session_id, &row.child_id, row.seq)
                });
                rows.push(row);
            }
        }
        if let Err(error) = store.upsert_subagent_events(&rows).await {
            tracing::warn!(%error, rows = rows.len(), "writing observed subagent events");
            // The store is authoritative: reload these children next time.
            for key in touched {
                self.children.remove(&key);
            }
            self.highwaters.clear();
        }
        if self.highwaters.len() > CACHED_CHILDREN {
            self.highwaters.clear();
        }
    }
}

async fn load_child(store: &Store, observed: &Observed) -> Option<ChildState> {
    match store
        .subagent_events_tail(&observed.session_id, &observed.child_id)
        .await
    {
        Ok(None) => Some(ChildState {
            next_seq: 1,
            ..ChildState::default()
        }),
        Ok(Some(tail)) => Some(ChildState {
            next_seq: tail.seq + 1,
            rows: tail.rows,
            capped: tail.rows > MAX_CHILD_ROWS,
            open: None,
        }),
        Err(error) => {
            tracing::warn!(%error, "reading observed subagent tail");
            None
        }
    }
}

/// Fold one observed update into its child's rows: the rows to write now.
fn fold(state: &mut ChildState, observed: Observed, now_ms: i64) -> Vec<ObservedRow> {
    let runtime_seq = i64::try_from(observed.runtime_seq).unwrap_or(i64::MAX);
    let mut out = Vec::new();
    let append = |state: &mut ChildState, payload: Value| {
        let stored = ObservedRow {
            session_id: observed.session_id.clone(),
            child_id: observed.child_id.clone(),
            seq: state.next_seq,
            ts_ms: now_ms,
            worker_epoch: observed.worker_epoch.clone(),
            runtime_seq,
            payload,
        };
        state.next_seq += 1;
        state.rows += 1;
        state.open = None;
        stored
    };
    if state.capped {
        return out;
    }
    if observed.dropped > 0 && state.rows < MAX_CHILD_ROWS {
        let gap =
            json!({"sessionUpdate": GAP_UPDATE, "reason": "dropped", "dropped": observed.dropped});
        out.push(append(state, gap));
    }
    if state.rows >= MAX_CHILD_ROWS {
        // One final row past the limit says the rest was not kept.
        state.capped = true;
        out.push(append(
            state,
            json!({"sessionUpdate": GAP_UPDATE, "reason": "limit"}),
        ));
        return out;
    }
    let update = bounded(observed.update.clone());
    if let Some(open) = state.open.as_mut()
        && let Some(merged) = coalesce(&open.payload, &update)
    {
        open.payload = merged;
        open.ts_ms = now_ms;
        open.runtime_seq = runtime_seq;
        open.worker_epoch.clone_from(&observed.worker_epoch);
        out.push(open.clone());
        return out;
    }
    let stored = append(state, update);
    if chunk_text(&stored.payload).is_some() {
        state.open = Some(stored.clone());
    }
    out.push(stored);
    out
}

fn chunk_text(update: &Value) -> Option<&str> {
    matches!(
        update.get("sessionUpdate").and_then(Value::as_str),
        Some("agent_message_chunk" | "agent_thought_chunk" | "user_message_chunk")
    )
    .then(|| update.pointer("/content/text").and_then(Value::as_str))
    .flatten()
    .filter(|_| update.pointer("/content/type").and_then(Value::as_str) == Some("text"))
}

/// The open row extended by `next` when both are text chunks of the same
/// kind and message, within the row budget.
fn coalesce(open: &Value, next: &Value) -> Option<Value> {
    let (current, more) = (chunk_text(open)?, chunk_text(next)?);
    if open.get("sessionUpdate") != next.get("sessionUpdate")
        || open.get("messageId") != next.get("messageId")
        || current.len() + more.len() > MAX_ROW_BYTES
    {
        return None;
    }
    let mut merged = open.clone();
    merged["content"]["text"] = Value::String(format!("{current}{more}"));
    Some(merged)
}

/// Trim an update whose serialized form exceeds [`MAX_ROW_BYTES`], keeping
/// what identifies and summarizes it.
#[must_use]
pub fn bounded(mut update: Value) -> Value {
    let size = |value: &Value| serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len());
    if size(&update) <= MAX_ROW_BYTES {
        return update;
    }
    if let Some(text) = update.pointer_mut("/content/text")
        && let Some(value) = text.as_str()
    {
        let mut end = MAX_ROW_BYTES / 2;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        *text = Value::String(value[..end].to_owned());
    }
    if let Some(object) = update.as_object_mut() {
        for field in ["content", "rawOutput", "rawInput", "locations"] {
            if size(&Value::Object(object.clone())) <= MAX_ROW_BYTES {
                break;
            }
            object.remove(field);
        }
        object.insert("_cowboy".to_owned(), json!({"truncated": true}));
    }
    if size(&update) <= MAX_ROW_BYTES {
        return update;
    }
    let keep = |key: &str| update.get(key).cloned().unwrap_or(Value::Null);
    json!({
        "sessionUpdate": keep("sessionUpdate"),
        "toolCallId": keep("toolCallId"),
        "title": keep("title"),
        "status": keep("status"),
        "kind": keep("kind"),
        "_cowboy": {"truncated": true},
    })
}

/// The parent-timeline event for one runtime event of `(session, worker)`:
/// observed child updates are diverted to their own transcript, subagent
/// snapshots are stamped with their worker incarnation.
pub fn divert(
    hub: &crate::core::Hub,
    (session_id, worker_epoch): &(String, String),
    runtime_seq: u64,
    event: RuntimeEvent,
) -> Option<RuntimeEvent> {
    intercept(session_id, worker_epoch, runtime_seq, event)
        .map(|event| stamp_worker_incarnation(hub, session_id, worker_epoch, event))
}

/// Name the worker incarnation that reported a subagent snapshot, and note
/// the session's current incarnation (docs/background-tasks.md I3). A
/// snapshot from an older incarnation renders as not reporting and loses its
/// Stop grant until the new worker announces the child again.
pub fn stamp_worker_incarnation(
    hub: &crate::core::Hub,
    session_id: &str,
    worker_epoch: &str,
    mut event: RuntimeEvent,
) -> RuntimeEvent {
    if worker_epoch.is_empty() {
        return event;
    }
    hub.note_worker_incarnation(session_id, worker_epoch);
    if let RuntimeEvent::Update { update, .. } = &mut event
        && update
            .get("sessionUpdate")
            .and_then(serde_json::Value::as_str)
            == Some(crate::runtime_wire::SUBAGENT_UPDATE)
        && let Some(object) = update.as_object_mut()
    {
        object.insert(
            "incarnation".to_owned(),
            serde_json::Value::String(worker_epoch.to_owned()),
        );
    }
    event
}

#[cfg(test)]
#[path = "subagent_transcripts_tests.rs"]
mod tests;
