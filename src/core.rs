//! Normalized session model + the in-memory `Hub` (design §5/§6).
//!
//! cowboy is the **single source of truth**: it assigns a monotonic `seq` per
//! session so ordering is global and unambiguous, keeps the per-session event
//! log, and fans every event out to all connected WebSocket clients equally.
//!
//! **Normalization shortcut.** ACP *is* the provider-agnostic model, and the
//! `agent-client-protocol` types are already `Serialize`. So rather than
//! re-modelling every variant, a passed-through agent update is carried as the
//! serialized `SessionUpdate` JSON ([`Event::Update`]); only the cowboy-specific
//! events (permission lifecycle, process lifecycle) get their own variants.
//!
//! **v1 storage.** The event log is in-memory (snapshot + live tail within the
//! process lifetime). `SQLite` persistence is deferred together with restart
//! `session/load` resume (design §7) — both land in the same follow-up.

use parking_lot::Mutex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

use crate::persistence::EventReducer;
use crate::runtime_wire::{WorkerSnapshot, WorkerState};

mod code_scope;
pub(crate) use code_scope::{CodeReadScope, SessionCodeScope};
mod product_permissions;
pub(crate) use product_permissions::ProductPermissionObservation;
mod persistence_queue;
pub(crate) use persistence_queue::NORMAL_BYTES as STORE_BATCH_MAX_BYTES;
pub use persistence_queue::{PersistenceHealth, StoreReceiver, StoreSink};
pub mod settings_keys;

/// How many recent events a fresh client gets over WS (the live tail). Older
/// history is paged in over HTTP. Sized to comfortably fill a few phone screens.
pub const SNAPSHOT_TAIL: usize = 200;
/// Maximum serialized event payload sent for one session during WebSocket
/// bootstrap. A few tool-heavy sessions can otherwise make every mobile
/// reconnect replay several megabytes before live fan-out starts. Older events
/// remain available through cursor-based HTTP history.
pub const SNAPSHOT_MAX_BYTES: usize = 128 * 1024;
/// Soft serialized-byte budget for one cursor history response. Event count
/// alone is not a useful bound: screenshots and large tool results can make a
/// 200-event page tens of megabytes and terminate an iOS WebContent process.
/// One oversized event is still returned so the cursor always advances.
pub const HISTORY_MAX_BYTES: usize = 512 * 1024;
/// Maximum persisted-history tail retained in the Hub. Older events stay in
/// Postgres and are fetched by `/api/history`.
pub const HOT_TAIL: usize = 1_000;
const HOT_TAIL_TRIM_BATCH: usize = 200;
/// Soft heap budget for one persisted session's canonical hot tail. A count
/// limit alone is ineffective for screenshots and multi-megabyte tool results.
/// Keep at least the newest event so the cursor always advances.
pub(crate) const HOT_TAIL_MAX_BYTES: usize = 1024 * 1024;
/// Idle sessions keep a smaller replay tail. Opening one pages older rows
/// through `/api/history`; a busy turn keeps the full 1 MiB so the focused
/// transcript does not stall mid-stream.
pub(crate) const HOT_TAIL_IDLE_MAX_BYTES: usize = 512 * 1024;
const BROADCAST_CAPACITY: usize = 1_024;
/// Event-count ceiling for the cursor-based HTTP history route. The byte budget
/// above is the primary bound; this limits render work for many tiny events.
pub const HISTORY_PAGE: usize = 64;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn estimated_json_bytes(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null => 4,
        serde_json::Value::Bool(_) => 5,
        serde_json::Value::Number(_) => 24,
        serde_json::Value::String(value) => value.len().saturating_add(2),
        serde_json::Value::Array(values) => values.iter().fold(2usize, |size, value| {
            size.saturating_add(estimated_json_bytes(value))
                .saturating_add(1)
        }),
        serde_json::Value::Object(values) => values.iter().fold(2usize, |size, (key, value)| {
            size.saturating_add(key.len())
                .saturating_add(estimated_json_bytes(value))
                .saturating_add(4)
        }),
    }
}

/// A cheap soft estimate used only for retention. Walking a JSON string is O(1)
/// (`String::len`), unlike serializing the ever-growing text on every token.
pub(crate) fn estimated_envelope_bytes(envelope: &Envelope) -> usize {
    let base = envelope
        .session_id
        .len()
        .saturating_add(envelope.cmid.as_deref().map_or(0, str::len))
        .saturating_add(64);
    match &envelope.event {
        Event::Update { update } => base.saturating_add(estimated_json_bytes(update)),
        _ => base
            .saturating_add(serde_json::to_vec(&envelope.event).map_or(256, |bytes| bytes.len())),
    }
}

pub(crate) fn hot_tail_budget_bytes(status: Status) -> usize {
    match status {
        Status::Busy => HOT_TAIL_MAX_BYTES,
        _ => HOT_TAIL_IDLE_MAX_BYTES,
    }
}

fn trim_hot_log(
    log: &mut Vec<Envelope>,
    log_bytes: &mut usize,
    trim_count_batch: bool,
    max_bytes: usize,
) -> bool {
    let mut drop_count = if trim_count_batch && log.len() > HOT_TAIL + HOT_TAIL_TRIM_BATCH {
        HOT_TAIL_TRIM_BATCH.min(log.len().saturating_sub(1))
    } else {
        0
    };
    let mut retained_bytes = *log_bytes;
    for envelope in &log[..drop_count] {
        retained_bytes = retained_bytes.saturating_sub(estimated_envelope_bytes(envelope));
    }
    while retained_bytes > max_bytes && drop_count + 1 < log.len() {
        retained_bytes = retained_bytes.saturating_sub(estimated_envelope_bytes(&log[drop_count]));
        drop_count += 1;
    }
    if drop_count == 0 {
        return false;
    }
    log.drain(..drop_count);
    *log_bytes = retained_bytes;
    true
}

/// Enforce the canonical in-memory hot-tail budget after restoring persisted
/// events. Storage applies an approximate serialized-byte bound before rows
/// cross the process boundary; this exact model-side pass accounts for decoded
/// strings and keeps the newest event when it alone exceeds the budget.
pub(crate) fn bound_restored_hot_log(log: &mut Vec<Envelope>) -> bool {
    // Older rows may predate canonical raw-output compaction. Normalize them
    // before computing the in-process budget so a duplicated image/command
    // result does not become the one oversized event retained after restart.
    for envelope in log.iter_mut() {
        crate::persistence::compact_canonical_tool_output(envelope);
    }
    let mut log_bytes = log.iter().fold(0usize, |size, envelope| {
        size.saturating_add(estimated_envelope_bytes(envelope))
    });
    trim_hot_log(log, &mut log_bytes, false, HOT_TAIL_MAX_BYTES)
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QuestionPageSummary {
    pub id: u64,
    pub title: String,
    pub ordinal: u64,
}

fn is_user_message_chunk(envelope: &Envelope) -> bool {
    matches!(
        &envelope.event,
        Event::Update { update }
            if update.get("sessionUpdate").and_then(serde_json::Value::as_str)
                == Some("user_message_chunk")
    )
}

fn is_context_cleared(envelope: &Envelope) -> bool {
    matches!(
        &envelope.event,
        Event::Update { update }
            if update.get("sessionUpdate").and_then(serde_json::Value::as_str)
                == Some("context_cleared")
    )
}

fn is_turn_end(envelope: &Envelope) -> bool {
    matches!(envelope.event, Event::TurnEnd { .. })
}

/// A typed refusal ends a turn, but leaves the native worker available. This
/// controller-owned detail holds queued work through idle snapshots without
/// inferring a refusal from ordinary assistant prose.
pub(crate) const MODEL_REFUSAL_DETAIL: &str = "The model declined this request. Review its message before editing the request, changing models, or starting a new session. Queued messages require an explicit send.";

/// Whether the current native-agent context has received a user turn.
///
/// Codex allocates a thread id at `session/new` but does not create a resumable
/// rollout until the first user turn. Stop at the latest clear marker so an old
/// conversation cannot make a newly-cleared, still-empty context look durable.
/// If the hot tail begins after both markers, conservatively preserve the id.
fn current_context_has_user_message(session: &Session) -> bool {
    for envelope in session.log.iter().rev() {
        if is_user_message_chunk(envelope) {
            return true;
        }
        if is_context_cleared(envelope) {
            return false;
        }
    }
    !session.reached_start
}

fn is_human_question_chunk(envelope: &Envelope) -> bool {
    matches!(
        &envelope.event,
        Event::Update { update }
            if update.get("sessionUpdate").and_then(serde_json::Value::as_str)
                == Some("user_message_chunk")
                && crate::prompt_origin::is_human_prompt_update(update)
                && !matches!(
                    update.pointer("/content/text").and_then(serde_json::Value::as_str)
                        .map(str::trim),
                    Some("/compact" | "/compress")
                )
    )
}

fn question_chunk_text(envelope: &Envelope) -> &str {
    match &envelope.event {
        Event::Update { update } => update
            .pointer("/content/text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        _ => "",
    }
}

pub(crate) fn question_summary_title(text: &str, ordinal: u64) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = compact.trim_matches(|character: char| {
        matches!(
            character,
            '#' | '>' | '*' | '+' | '-' | '.' | '`' | ' ' | '\t'
        ) || character.is_ascii_digit()
    });
    if trimmed.is_empty() {
        return format!("Page {ordinal}");
    }
    let mut chars = trimmed.chars();
    let title = chars.by_ref().take(72).collect::<String>();
    if chars.next().is_some() {
        format!("{}…", title.trim_end())
    } else {
        title
    }
}

#[cfg(test)]
mod question_summary_title_tests {
    use super::question_summary_title;

    #[test]
    fn empty_page_titles_use_page_view_terminology() {
        assert_eq!(question_summary_title("7", 7), "Page 7");
        assert_eq!(question_summary_title("###", 12), "Page 12");
    }
}

pub(crate) fn bound_history_page(mut events: Vec<Envelope>) -> Vec<Envelope> {
    let mut start = events.len();
    let mut serialized_bytes = 0usize;
    for index in (0..events.len()).rev() {
        let event_bytes = serde_json::to_vec(&events[index]).map_or(0, |event| event.len());
        if serialized_bytes > 0 && serialized_bytes.saturating_add(event_bytes) > HISTORY_MAX_BYTES
        {
            break;
        }
        start = index;
        serialized_bytes = serialized_bytes.saturating_add(event_bytes);
    }
    if start > 0 {
        events.drain(..start);
    }
    events
}

/// Who opened a session. Used by the UI to render an `origin` badge and
/// (eventually) to decide which sessions belong to which client surface.
/// `Web` = a browser/phone clicked "New session" on cowboy's own UI.
/// `Api` = a direct `POST /api/sessions` with no `origin` field (curl, tests,
/// future scripted callers).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionOrigin {
    #[default]
    Api,
    Web,
}

pub use crate::agent_model::{Event, Status};
use crate::agent_model::{LEGACY_AUTO_CONTINUE_PREFIX, SCHED_PREFIX};

/// One event stamped with its session + monotonic `seq`. This is the unit
/// stored in the log and streamed to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub session_id: String,
    pub seq: u64,
    #[serde(flatten)]
    pub event: Event,
    /// LIVE-only echo of the originating client's cmid on the `user_message_chunk`
    /// it dispatched, so that client reconciles its optimistic chat bubble by id.
    /// Not persisted (transient reconcile tag) and None for everything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmid: Option<String>,
}

/// Client message id prefix reserved for Controller-owned managed call turns.
pub const MANAGED_CALL_CMID_PREFIX: &str = "managed-call:";

/// One managed child turn as observed in the Hub's hot log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedTurn {
    /// The prompt is queued, in flight or already echoed into the transcript.
    pub prompted: bool,
    pub stop_reason: Option<String>,
    pub final_text: String,
    pub status: Status,
}

/// Extract `(user_prompt, assistant_partial)` for the LAST turn in a session's
/// log — the turn cut off by a restart. Walks to the last `user_message_chunk`
/// group (the prompt) and concatenates the `agent_message_chunk` text after it
/// (the partial output, since a cut-off turn has no `TurnEnd`). Text blocks only;
/// degrades to empty strings.
fn last_turn_texts(log: &[Envelope]) -> (String, String) {
    let chunk = |env: &Envelope| -> Option<(String, String)> {
        if let Event::Update { update } = &env.event {
            let kind = update
                .get("sessionUpdate")
                .and_then(serde_json::Value::as_str)?;
            let text = update
                .get("content")
                .and_then(|c| c.get("text"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            return Some((kind.to_owned(), text.to_owned()));
        }
        None
    };
    let last_user = log
        .iter()
        .rposition(|env| matches!(chunk(env), Some((ref k, _)) if k == "user_message_chunk"));
    let Some(start) = last_user else {
        return (String::new(), String::new());
    };
    let mut prompt = String::new();
    let mut partial = String::new();
    for env in &log[start..] {
        match chunk(env) {
            Some((k, t)) if k == "user_message_chunk" => prompt.push_str(&t),
            Some((k, t)) if k == "agent_message_chunk" => partial.push_str(&t),
            _ => {}
        }
    }
    (prompt, partial)
}

/// An in-progress switch to another installed Provider release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUpdate {
    pub from: String,
    pub to: String,
    /// Started by the idle auto-update policy rather than an explicit Reload.
    pub automatic: bool,
    /// Controller clock when the reload began. Clients estimate progress from
    /// it because a worker start reports no intermediate phases.
    pub started_at_ms: i64,
}

/// A newer, native-session-compatible Provider release installed on the
/// session's Device. Clients use it to offer the update; nothing changes
/// until the user or an idle policy applies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUpdateAvailable {
    pub version: String,
    pub digest: String,
    /// The user asked to update as soon as the current work finishes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub when_idle: bool,
    /// Estimated Controller epoch ms at which the idle policy updates this
    /// session unattended. Absent when no automatic policy applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic_at_ms: Option<i64>,
}

/// Controller-held offer behind [`SessionMeta::provider_update_available`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderUpdateOffer {
    pub version: String,
    pub digest: String,
    pub when_idle: bool,
    /// Idle period after which a policy updates the session unattended.
    pub automatic_after: Option<std::time::Duration>,
}

/// Session metadata for the list view (no event log).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub provider: String,
    /// Exact Agent Plugin release, changed only by an explicit idle reload.
    #[serde(default)]
    pub provider_version: String,
    #[serde(default)]
    pub provider_generation_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_auth_generation: Option<u64>,
    /// Signed host-integration interface selected by this exact Provider
    /// generation. `None` is reserved for package-less legacy sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_behavior: Option<cowboy_provider_sdk::ProviderBehaviorContract>,
    /// Stable machine placement. A session never silently migrates to another
    /// machine because its provider credentials, cwd, and native thread all
    /// belong to the selected host.
    #[serde(default = "local_machine_id")]
    pub machine_id: String,
    /// Stable advertised Machine workspace identity selected at creation.
    /// Kept separately because `cwd` is replaced with an isolated worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_source_path: Option<String>,
    /// Independent target identity. Absence preserves legacy runtime-local
    /// execution; an unknown or invalid record must never fall back to it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::execution_environment::deserialize_optional_binding"
    )]
    pub execution_binding: Option<crate::execution_environment::ExecutionBinding>,
    pub cwd: String,
    pub title: String,
    pub status: Status,
    /// Who opened the session (UI surface that called `new_session`).
    #[serde(default)]
    pub origin: SessionOrigin,
    /// The downstream agent's OWN session id (the ACP id it returns from
    /// `session/new`). Captured on first start; used by the supervisor to
    /// resume the prior conversation via `session/load` when reviving a
    /// session whose agent process is gone (design §7). `None` until the
    /// agent assigns one, and for providers that don't support resume.
    #[serde(default)]
    pub agent_session_id: Option<String>,
    /// User-set MANUAL PAUSE of the queue drain (the ⏸ toggle). While true the
    /// auto-drain is HELD — queued messages don't advance even after the current
    /// turn ends — but the running turn is NOT interrupted (it finishes). The
    /// user toggles it (`SetPaused`) and releases it to resume. A MANUAL send
    /// still overrides it. In-memory only (transient — resets to false on a
    /// daemon restart); `serde(default)` covers old clients + the restore path.
    #[serde(default)]
    pub paused: bool,
    /// True while an accepted deletion waits for the session's execution
    /// environment to confirm it stopped. Clients treat it as the deletion
    /// acknowledgement; the row disappears on confirmation or returns with a
    /// Crashed detail when the stop stays unconfirmed. Transient.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closing: bool,
    /// True for a machine-driven system session: visible and watchable in the
    /// UI but view-only. The composer is hidden and user turns are rejected;
    /// only the backend wake endpoint drives it. Persisted for compatibility.
    #[serde(default)]
    pub system: bool,
    /// Context-window usage the agent reports over ACP `usage_update`:
    /// `context_used` tokens of a `context_size`-token window (so the UI shows a
    /// "context X% full" ring — see the composer). `0`/`0` = not yet reported.
    /// Transient live state (intercepted onto the meta rather than bloating the
    /// timeline with a copy per turn — see acp.rs); `serde(default)` covers old
    /// clients + the restore path. Not persisted — a fresh `usage_update` re-seeds
    /// it right after any revive.
    #[serde(default)]
    pub context_used: u64,
    #[serde(default)]
    pub context_size: u64,
    /// Full latest ACP usage update, including optional cost and provider `_meta`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<crate::agent_model::SessionUsage>,
    /// Native background tasks the agent is still waiting on after its prompt
    /// turn ended (a backgrounded shell, a Monitor). Presentation only: it
    /// never holds the queue, because a background server may never finish.
    /// Transient live state restored from the worker snapshot on reconnect.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub background_tasks: u32,
    /// Provider release switch in progress while `status` is `Starting`.
    /// Lets clients present an unattended idle update as background
    /// maintenance instead of a cold start. Cleared on the next status change.
    /// Transient; never persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_update: Option<ProviderUpdate>,
    /// Newer installed Provider release this session can adopt. Derived from
    /// the Controller's update offers in `session_list`. Transient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_update_available: Option<ProviderUpdateAvailable>,
    /// Soonest fire time (epoch ms) across this session's SCHEDULED DRAFTS, or
    /// `None` if none are scheduled. Derived from the drafts in `session_list`
    /// (not stored on the struct proper) so the session-row clock badge can show
    /// "next fires at …" without shipping every draft to the list. Transient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_schedule_ms: Option<i64>,
    /// Product account that created this session. `None` is the pre-auth shared
    /// pool (legacy rows and unauthenticated creates).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<String>,
    /// Display username for `owner_user_id`. Not a column; stamped at create and
    /// joined from `users` on restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_username: Option<String>,
}

impl SessionMeta {
    /// Binding identity alone cannot authorize execution. The selected exact
    /// signed Provider must also accept this executor contract.
    pub(crate) fn require_runtime_launch(&self) -> Result<(), String> {
        if let Some(binding) = &self.execution_binding {
            if let Some(child) = binding.managed_child() {
                return if child.accepts(&self.id, &self.machine_id, &self.cwd)
                    && !self.provider_version.is_empty()
                    && !self.provider_generation_digest.is_empty()
                {
                    Ok(())
                } else {
                    Err("managed child does not match its pinned session".into())
                };
            }
            let binding = binding
                .for_runtime(&self.machine_id, &self.cwd)
                .map_err(str::to_owned)?;
            if self.provider_version.is_empty()
                || self.provider_generation_digest.is_empty()
                || !self
                    .provider_behavior
                    .as_ref()
                    .and_then(|value| value.execution.as_ref())
                    .is_some_and(|contract| {
                        contract.accepts(
                            binding.environment.protocol,
                            &binding.environment.executor_digest,
                        )
                    })
            {
                return Err(
                    "this Provider release does not support the bound execution environment".into(),
                );
            }
        }
        Ok(())
    }
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde skip_serializing_if passes a reference.
const fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

fn local_machine_id() -> String {
    "local".to_owned()
}

fn configuration_behavior(
    provider: &str,
    behavior: Option<&cowboy_provider_sdk::ProviderBehaviorContract>,
) -> cowboy_provider_sdk::ConfigurationBehavior {
    behavior.map_or_else(
        || crate::provider::legacy_behavior(provider).configuration,
        |behavior| behavior.configuration.clone(),
    )
}

fn default_config_preferences(
    provider: &str,
    behavior: Option<&cowboy_provider_sdk::ProviderBehaviorContract>,
) -> serde_json::Value {
    let defaults = behavior.map_or_else(
        || crate::provider::legacy_behavior(provider).default_preferences,
        |behavior| behavior.default_preferences.clone(),
    );
    serde_json::to_value(defaults).unwrap_or_else(|_| serde_json::json!({}))
}

fn projected_config_options(
    provider: &str,
    behavior: Option<&cowboy_provider_sdk::ProviderBehaviorContract>,
    preferences: &serde_json::Value,
    options: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    let configuration = configuration_behavior(provider, behavior);
    crate::managed_config::projected_options(&configuration, preferences, options)
}

/// Immutable attributes assigned when a Cowboy session is registered.
pub struct SessionRegistration {
    pub id: String,
    pub provider: String,
    pub provider_version: String,
    pub provider_generation_digest: String,
    pub provider_auth_generation: Option<u64>,
    pub provider_behavior: Option<cowboy_provider_sdk::ProviderBehaviorContract>,
    pub machine_id: String,
    pub workspace_id: Option<String>,
    pub workspace_name: Option<String>,
    pub workspace_source_path: Option<String>,
    pub execution_binding: Option<crate::execution_environment::ExecutionBinding>,
    pub cwd: String,
    pub title: String,
    pub origin: SessionOrigin,
    pub system: bool,
    pub owner_user_id: Option<String>,
    pub owner_username: Option<String>,
}

/// One staged message — either a QUEUED prompt (waiting for the current turn to
/// end) or a parked DRAFT. Server-authoritative so every connected terminal sees
/// the same queue/drafts (design follow-up: these used to be client-local
/// localStorage, which never synced across devices). `content` is the already-
/// built ACP content-block array exactly as a `Prompt` would carry it (empty for
/// a plain-text message); `text` is kept alongside for display / re-editing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedMessage {
    pub id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub content: Vec<serde_json::Value>,
    /// Client-generated message id (uuid), round-tripped UNCHANGED so the
    /// ORIGINATING client can reconcile its optimistic row and dedupe a retry.
    /// Purely a per-client tag — never used for cross-terminal sync, and absent
    /// for bridge/API sends. Lives inside the jsonb queue/drafts blob, so it
    /// needs no schema column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmid: Option<String>,
    /// Present only on a DRAFT that's been given a future fire time — the
    /// server-side scheduler auto-activates it then (see `Hub::schedule_draft`).
    /// `None` for a plain draft / any queued message. Rides the same jsonb
    /// drafts blob (no schema column), so it persists and survives a restart
    /// (the startup re-arm scans drafts for it). Queue items never carry one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<DraftSchedule>,
}

/// A draft's future auto-send instruction. Server-controlled (fires even with
/// every client offline). One-shot — cleared when it fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSchedule {
    /// Absolute epoch-ms fire time. Computed on the client at commit (so it can
    /// mean "9am tomorrow" without a delay clamp) and armed verbatim — unlike the
    /// agent's `ScheduleWakeup`, this is NOT clamped to the [60s,1h] wakeup band.
    pub fire_at_ms: i64,
    /// Where the prompt lands in the send-queue at fire time: tail (default) or
    /// head. BOTH always respect a paused queue (a fired draft never bypasses the
    /// ⏸ hold) and never interrupt a running turn.
    #[serde(default)]
    pub delivery: Delivery,
}

/// Fire-time queue position for a scheduled draft. The two modes differ ONLY in
/// where the fired prompt lands; both wait for any in-flight turn to finish and
/// both honour a paused queue (fire → enqueue-and-hold, never bypass).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// Append to the TAIL of the send-queue (default): runs after everything
    /// already queued. `queue` alias keeps drafts persisted by the first cut
    /// (which had a queue/now split) deserializing.
    #[default]
    #[serde(alias = "queue")]
    Back,
    /// Insert at the HEAD of the send-queue: runs before other queued prompts,
    /// but still lets a live turn finish and still respects the pause. `now` alias
    /// maps the retired bypass-pause mode onto plain front-insert.
    #[serde(alias = "now")]
    Front,
}

/// A request from the Hub to the background dispatcher task (in `crate::server`)
/// to actually send a queued prompt to its agent. The Hub owns the queue +
/// serialization state but cannot call the `Supervisor` (which holds the Hub),
/// so the drain decision happens under the Hub lock and the resulting dispatch
/// is handed off over this channel — breaking the Hub→Supervisor cycle.
#[derive(Debug, Clone)]
pub struct DispatchReq {
    pub session_id: String,
    pub text: String,
    pub content: Vec<serde_json::Value>,
    /// cmid of the originating submit (chat send) — carried so the agent's
    /// user-message echo can be tagged for optimistic reconcile. None for a
    /// drained queue item (its optimistic row already reconciled via `queues`).
    pub cmid: Option<String>,
}

/// One session's full persisted state, handed to
/// [`Hub::restore_reconciling_runtime`] at startup.
pub struct RestoredSession {
    pub meta: SessionMeta,
    pub log: Vec<Envelope>,
    pub event_count: u64,
    pub reached_start: bool,
    pub next_seq: u64,
    pub queue: Vec<QueuedMessage>,
    pub drafts: Vec<QueuedMessage>,
    /// Latest agent-advertised config options, retained so a new device can
    /// render the session controls before its worker is warm.
    pub config_options: Option<serde_json::Value>,
    /// User-selected values that the service must re-apply when the worker is
    /// recreated. Defaults are seeded for newly-created OpenAI sessions.
    pub config_preferences: serde_json::Value,
    pub mobile_review_state: serde_json::Value,
    /// Explicit sidebar folder placement (`sessions.folder_id`).
    pub folder_id: Option<String>,
}

/// Per-session info for the UI's session-info dialog — the metadata plus the
/// live in-memory counts (event log length + staged queue/drafts sizes).
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    #[serde(flatten)]
    pub meta: SessionMeta,
    pub event_count: u64,
    pub queue_count: usize,
    pub drafts_count: usize,
}

/// Admission and delivery are separate facts for each recent dispatch.
struct DispatchedPrompt {
    cmid: String,
    echoed: bool,
}

/// Per-session state: metadata + the seq-ordered event log.
struct Session {
    meta: SessionMeta,
    // A Controller-local observation lifetime, not the native worker lifetime.
    code_incarnation: code_scope::CodeIncarnation,
    /// The durable Session lineage the owning Machine last reported. `None`
    /// means that Machine reports none (no admitted writer, an older release, a
    /// local runtime), in which case only the process-local observation lifetime
    /// above fences this Session. Learned from snapshots, never constructed here.
    machine_lineage: Option<String>,
    /// Hot event tail when persistence is enabled; the full log in memory-only
    /// development mode.
    log: Vec<Envelope>,
    /// Soft heap estimate for `log`, maintained alongside canonical upserts so
    /// large tool payloads are bounded without serializing every text chunk.
    log_bytes: usize,
    event_count: u64,
    reached_start: bool,
    next_seq: u64,
    /// When this Controller last appended an event to the session. Monotonic
    /// and process-local: a restart treats every session as just active, so
    /// idle-based policies wait a full idle period before acting.
    last_activity: std::time::Instant,
    /// Last seen agent-advertised config options (raw ACP
    /// `configOptions` array — see acp.rs intercept). `None` until the agent
    /// fires its first `config_option_update` notification. Re-sent to every
    /// new client on connect so the composer dropdowns populate from a fresh
    /// reload.
    config_options: Option<serde_json::Value>,
    /// Session-owned config values. This is deliberately separate from the
    /// latest agent snapshot: an agent's startup defaults must not erase a
    /// user's choice before the service has re-applied it.
    config_preferences: serde_json::Value,
    /// Prompts waiting for the current turn to finish, in send order. Drained
    /// one-at-a-time on each turn-end (see `Hub::try_drain`).
    queue: Vec<QueuedMessage>,
    /// Parked messages the user composed but hasn't committed to send.
    drafts: Vec<QueuedMessage>,
    /// The queued-message id currently held open for editing, if any. A held
    /// head pauses the whole queue drain (the user is editing "don't send this
    /// or the ones behind it"). GLOBAL across terminals; cleared when the editing
    /// client releases or after a disconnected client's bounded reconnect
    /// grace. One hold per session (matches the original single client-side
    /// `editingHold` model).
    editing: Option<String>,
    /// Lease generation for `editing`. A reconnect reasserts the hold and bumps
    /// this value, so a delayed cleanup owned by the replaced WebSocket cannot
    /// release the new connection's edit transaction.
    editing_epoch: u64,
    /// True while a queue-dispatched prompt of ours is in flight but the session
    /// hasn't yet flipped back to idle. Guards the dispatch-before-`Busy` window
    /// so a same-tick re-drain can't double-send and overlap turns. Cleared once
    /// per completed turn or on death (see `complete_turn` / `set_status`).
    in_flight: bool,
    /// Monotonic identity for the current in-flight dispatch guard. Runtime
    /// reconciliation captures this before applying an idle lifecycle edge so
    /// it cannot clear a newer guard created while that edge drains the queue.
    in_flight_epoch: u64,
    /// Client message ids of prompts this Hub recently handed to a worker. A
    /// reconnecting client resends every unconfirmed submit; between the dispatch
    /// and the persisted user echo only this window knows the prompt already
    /// ran. Bounded; the echoed `cmid` in `log` covers the rest and survives a
    /// Controller restart.
    dispatched_cmids: VecDeque<DispatchedPrompt>,
    /// A worker reports the same completed turn through both `TurnEnded` and a
    /// trailing `Busy` -> `Running` lifecycle edge. The first edge may drain the
    /// next prompt before the second arrives. Latch that completion until the
    /// next authoritative `Busy` edge so the duplicate completion cannot clear
    /// the newly-created in-flight guard and dispatch a second queued prompt.
    turn_completion_latched: bool,
    /// Monotonic identity for the current lifecycle edge. Bumped only when the
    /// status actually changes, so duplicate worker snapshots do not disguise a
    /// stuck turn while a Busy -> Running -> Busy replacement invalidates every
    /// watchdog armed for the previous turn.
    lifecycle_epoch: u64,
    /// Mobile-only code-review workspace state. Desktop never consumes it.
    mobile_review: MobileReviewState,
}

fn latest_crash_detail_for_session(session: &Session) -> Option<&str> {
    if session.meta.status != Status::Crashed {
        return None;
    }
    for envelope in session.log.iter().rev() {
        let Event::Lifecycle { status, detail } = &envelope.event else {
            continue;
        };
        if *status != Status::Crashed {
            return None;
        }
        if let Some(detail) = detail.as_deref() {
            return Some(detail);
        }
    }
    None
}

/// Return the newest crash detail that has not been followed by a clean turn
/// completion. A live worker can project `Running` again after a recoverable
/// Provider failure, so current status alone is not enough to decide whether
/// refreshed credentials still need to be applied to that session.
fn unresolved_crash_detail_for_session(session: &Session) -> Option<&str> {
    for envelope in session.log.iter().rev() {
        match &envelope.event {
            Event::TurnEnd { .. } => return None,
            Event::Lifecycle {
                status: Status::Crashed,
                detail: Some(detail),
            } => return Some(detail),
            _ => {}
        }
    }
    None
}

const MOBILE_REVIEW_TAB_CAP: usize = 12;
const MOBILE_REVIEW_PROGRESS_CAP: usize = 512;
const MOBILE_REVIEW_POSITION_CAP: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MobileReviewTab {
    path: String,
    #[serde(default)]
    pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MobileReviewPosition {
    line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MobileReviewState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_review: Option<crate::workspace_extensions::ReviewBinding>,
    #[serde(default)]
    remote_selected: bool,
    #[serde(default = "default_mobile_review_mode")]
    mode: String,
    #[serde(default)]
    tabs: Vec<MobileReviewTab>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active: Option<String>,
    #[serde(default)]
    progress: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    positions: std::collections::BTreeMap<String, MobileReviewPosition>,
}

fn default_mobile_review_mode() -> String {
    "git".to_owned()
}

impl Default for MobileReviewState {
    fn default() -> Self {
        Self {
            remote_review: None,
            remote_selected: false,
            mode: default_mobile_review_mode(),
            tabs: Vec::new(),
            active: None,
            progress: std::collections::BTreeMap::new(),
            positions: std::collections::BTreeMap::new(),
        }
    }
}

impl MobileReviewState {
    fn from_stored(value: serde_json::Value) -> Self {
        let mut state = serde_json::from_value::<Self>(value).unwrap_or_default();
        if state
            .remote_review
            .as_ref()
            .is_some_and(|binding| !binding.valid())
        {
            state.remote_review = None;
            state.remote_selected = false;
        }
        if !matches!(state.mode.as_str(), "files" | "git") {
            state.mode = default_mobile_review_mode();
        }
        state.tabs.retain(|tab| valid_mobile_review_path(&tab.path));
        let mut seen = HashSet::new();
        state.tabs.retain(|tab| seen.insert(tab.path.clone()));
        if state.tabs.len() > MOBILE_REVIEW_TAB_CAP {
            state.tabs = state
                .tabs
                .split_off(state.tabs.len() - MOBILE_REVIEW_TAB_CAP);
        }
        if state
            .active
            .as_ref()
            .is_some_and(|path| !state.tabs.iter().any(|tab| &tab.path == path))
        {
            state.active = None;
        }
        state.progress.retain(|key, revision| {
            !key.is_empty() && key.len() <= 2048 && !revision.is_empty() && revision.len() <= 512
        });
        while state.progress.len() > MOBILE_REVIEW_PROGRESS_CAP {
            if let Some(key) = state.progress.keys().next().cloned() {
                state.progress.remove(&key);
            }
        }
        state.positions.retain(|path, position| {
            valid_mobile_review_path(path)
                && position.line > 0
                && position
                    .revision
                    .as_ref()
                    .is_none_or(|revision| !revision.is_empty() && revision.len() <= 512)
        });
        while state.positions.len() > MOBILE_REVIEW_POSITION_CAP {
            if let Some(path) = state.positions.keys().next().cloned() {
                state.positions.remove(&path);
            }
        }
        state
    }
}

fn valid_mobile_review_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.starts_with('/')
        && !path.split('/').any(|part| matches!(part, "" | "." | ".."))
        && !path.contains('\0')
}

fn mobile_review_string_arg(
    args: &serde_json::Value,
    name: &str,
    max_len: usize,
) -> Result<String, String> {
    let value = args
        .get(name)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("missing {name}"))?;
    if value.is_empty() || value.len() > max_len || value.contains('\0') {
        return Err(format!("invalid {name}"));
    }
    Ok(value.to_owned())
}

/// A command sent by a client (Web UI, native shell, API / test harnesses)
/// to the daemon over the WebSocket. Tag is `type`, snake-cased.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inbound {
    /// Coarse, user-gesture-only browser activity used to slide the idle
    /// deadline. Heartbeats, agent output, and background refreshes never send it.
    AuthActivity,
    /// Foreground liveness check; never extends the user session idle deadline.
    ConnectionProbe { nonce: u64 },
    /// Start a new agent session.
    NewSession {
        provider: String,
        #[serde(default)]
        cwd: Option<String>,
    },
    /// Send a user turn to a session. Two shapes:
    ///
    /// - **Web UI** sends `text: "..."` (legacy text-only path; daemon wraps
    ///   it in a single ACP `Text` content block).
    /// - **API / direct callers** send `content: [...ACP ContentBlock JSON]` to
    ///   carry rich content (e.g. pasted images). When both are present,
    ///   `content` wins. At least one must be non-empty; otherwise the prompt is
    ///   dropped server-side with a warn log.
    Prompt {
        session_id: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        content: Vec<serde_json::Value>,
    },
    /// Cancel a session's current turn.
    Cancel { session_id: String },
    /// Cancel a prompt submitted by an ACP bridge before it becomes the active
    /// turn. `cmid` is the bridge-generated correlation id. This deliberately
    /// removes only that queued prompt and never disturbs another surface's
    /// active turn.
    CancelSubmitted { session_id: String, cmid: String },
    /// Answer a pending permission request.
    Permission {
        session_id: String,
        request_id: String,
        #[serde(default)]
        option_id: Option<String>,
    },
    /// Tear down a session: cancel any in-flight turn, drop the agent thread,
    /// remove the entry from the Hub, and broadcast the updated session list
    /// to every connected client (so other surfaces auto-clear).
    DeleteSession { session_id: String },
    /// Rename a session — the user-customizable title shown in the `AppBar`
    /// and (post-rename) in the sidebar list. Empty title is rejected at
    /// the server before this point.
    RenameSession { session_id: String, title: String },
    /// Compatibility tombstone for pre-removal Web clients. The command is
    /// accepted and ignored so a stale installed PWA cannot re-enable the retired
    /// behavior or surface a protocol error before it updates.
    SetSessionAutoResume {
        session_id: String,
        #[serde(default)]
        value: Option<bool>,
    },
    /// User toggle: manually pause/resume the queue drain. Holds the auto-drain
    /// without interrupting the running turn (see [`Hub::set_paused`]).
    SetPaused { session_id: String, paused: bool },
    /// Compatibility tombstone for the retired synthetic continuation action.
    ResumeTurn { session_id: String },
    /// Overlay action: retry an errored/crashed turn (re-run the last prompt).
    RetryTurn { session_id: String },
    /// Compatibility tombstone for retired auto-resume settings.
    SetSetting {
        key: String,
        value: serde_json::Value,
    },
    /// Generic optimistic-sync mutation (Cowboy state-sync). The client applies
    /// it locally for an INSTANT update, then sends it here; the daemon (the
    /// arbiter) linearizes it per `state`, version-stamps, and broadcasts an
    /// [`Outbound::SyncPatch`] every terminal folds. `id` is the client-minted
    /// mutation id (the `cmid` generator) — it makes a retry idempotent (the
    /// arbiter dedupes on it). `state` selects the synced value (`"title"`,
    /// `"order"`, …); `name`+`args` are the mutator + its JSON-plain args, applied
    /// by the typed handler in `Hub::sync_apply`. Supersedes the bespoke
    /// rename/reorder commands for the web client.
    Sync {
        state: String,
        id: String,
        name: String,
        #[serde(default)]
        args: serde_json::Value,
    },
    /// Set one config option on the session (mode / model / effort / future).
    /// ACP exposes a unified typed `session/set_config_option` request that
    /// handles all three via the same shape. The agent answers with the
    /// refreshed `configOptions` array, which the daemon then re-broadcasts as
    /// [`Outbound::ConfigOptions`].
    SetConfigOption {
        session_id: String,
        config_id: String,
        /// Free-form value — typically a string variant id (`"sonnet"`,
        /// `"high"`, `"bypassPermissions"`), but the protocol allows
        /// booleans too. Forwarded verbatim.
        value: serde_json::Value,
    },
    /// Client opened/selected a session — revive its agent if it died with a
    /// daemon restart, WITHOUT sending a turn. Idempotent (a no-op when the
    /// agent is already alive), so it's safe to send on every open / reconnect.
    /// Lets a reopened session warm up before the user types (design §7).
    /// Handled in server.rs via [`crate::supervisor::Supervisor::ensure_alive`].
    OpenSession { session_id: String },

    /// Reset a session's agent context ("clear conversation"). Over ACP, clearing
    /// is the CLIENT's job — Claude/Codex/Gemini expose no `clear` agent command
    /// (only `compact`), so this can't be a slash command. The daemon tears the
    /// agent down and respawns it with a FRESH `session/new` (dropping the prior
    /// `agent_session_id` so it does NOT `session/load`), then drops a
    /// `context_cleared` marker into a fresh timeline. Clear is intentionally a
    /// destructive boundary: both agent context and prior transcript are discarded.
    ResetSession { session_id: String },

    // --- Server-authoritative queue + drafts (synced across all terminals) ----
    //
    // The Web UI sends these instead of dispatching prompts itself: the daemon
    // owns the per-session queue/drafts and the drain (next-on-turn-end), so
    // every connected terminal sees identical state and only one turn ever runs.
    /// Send a user turn the queue-aware way: dispatch immediately if the session
    /// is idle and nothing is queued/in-flight, otherwise append to the queue.
    /// (The API keeps using `Prompt` for a direct, un-queued dispatch.)
    Submit {
        session_id: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        content: Vec<serde_json::Value>,
        /// Optional client message id for optimistic reconcile + idempotent
        /// retry (Phase 2 uses it for the chat/queue path). See QueuedMessage.
        #[serde(default)]
        cmid: Option<String>,
        /// "Force push" a busy session: instead of appending to the back of the
        /// queue, jump this prompt to the FRONT and interrupt the running turn so
        /// it runs next (the long-press-send affordance). No-op on an idle session
        /// — it just sends normally. Old clients omit it (defaults false).
        #[serde(default)]
        force: bool,
        /// "Jump to front" WITHOUT interrupting: insert at the FRONT of the queue
        /// (runs next after the current turn, ahead of the rest of the queue) but
        /// do NOT cancel the running turn. Distinct from `force`. No-op on an
        /// idle/empty-queue session. Old clients omit it (defaults false).
        #[serde(default)]
        front: bool,
    },
    /// Drop one queued prompt. `cmid` is the client's durable outbox id for
    /// this command, so a replay whose row already ran receives an addressed
    /// `stale` result instead of retrying forever.
    RemoveQueued {
        session_id: String,
        id: String,
        #[serde(default)]
        cmid: Option<String>,
    },
    /// Edit a queued prompt in place (text + content). Empty both → removed.
    EditQueued {
        session_id: String,
        id: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        content: Vec<serde_json::Value>,
        #[serde(default)]
        cmid: Option<String>,
    },
    /// Drop a session's whole queue.
    ClearQueue { session_id: String },
    /// "Send now": move a queued prompt to the front and drain it if the session
    /// can take a turn this instant; otherwise it just becomes next in line.
    RequestSendQueued { session_id: String, id: String },
    /// "Force push": interrupt the running turn and make this prompt run next.
    ForcePushQueued { session_id: String, id: String },
    /// Move a queued prompt back to drafts.
    QueuedToDraft { session_id: String, id: String },
    /// Hold (or release, with `id: null`) the queue head for editing — pauses the
    /// drain on every terminal while one client edits.
    SetQueueEditing {
        session_id: String,
        #[serde(default)]
        id: Option<String>,
    },
    /// Park the composer's content as a new draft.
    AddDraft {
        session_id: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        content: Vec<serde_json::Value>,
        /// Client message id → optimistic draft reconcile + idempotent retry.
        #[serde(default)]
        cmid: Option<String>,
    },
    /// Edit a draft in place. Empty both → removed.
    EditDraft {
        session_id: String,
        id: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        content: Vec<serde_json::Value>,
        #[serde(default)]
        cmid: Option<String>,
    },
    /// Drop one draft.
    RemoveDraft {
        session_id: String,
        id: String,
        #[serde(default)]
        cmid: Option<String>,
    },
    /// Drop a session's whole draft list.
    ClearDrafts { session_id: String },
    /// Activate one draft: submit it (send-or-queue) and remove it from drafts.
    ActivateDraft {
        session_id: String,
        id: String,
        #[serde(default)]
        cmid: Option<String>,
    },
    /// Activate every draft, front-to-back.
    ActivateAllDrafts { session_id: String },
    /// Attach/replace a future fire time on a draft (create it if `id`/`cmid`
    /// match nothing). The server-side scheduler auto-activates it at `fire_at_ms`
    /// — fires even with every client offline. `text`/`content` overwrite the
    /// target only when non-empty. See `Hub::schedule_draft`.
    ScheduleDraft {
        session_id: String,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        cmid: Option<String>,
        #[serde(default)]
        text: String,
        #[serde(default)]
        content: Vec<serde_json::Value>,
        fire_at_ms: i64,
        #[serde(default)]
        delivery: Delivery,
    },
    /// Strip the schedule off a draft (it stays a plain parked draft).
    UnscheduleDraft { session_id: String, id: String },
    /// Move a draft to another session's drafts (the "parked it in the wrong
    /// session" fix). The whole message — text + attachments — relocates to the
    /// END of `to_session`'s drafts. `session_id` is the SOURCE.
    MoveDraft {
        session_id: String,
        id: String,
        to_session: String,
    },

    // --- Reorder (drag-to-arrange, server-authoritative + synced) -------------
    /// Reorder the session list to match `order` (a full list of session ids;
    /// any omitted ids keep their relative order at the end). Persisted +
    /// broadcast so every terminal shows the same arrangement.
    ReorderSessions { order: Vec<String> },
    /// Reorder one session's send-queue to match `order` (queued message ids).
    ReorderQueue {
        session_id: String,
        order: Vec<String>,
    },
    /// Reorder one session's drafts to match `order` (draft ids).
    ReorderDrafts {
        session_id: String,
        order: Vec<String>,
    },
}

/// What the server pushes to a WebSocket client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outbound {
    /// Per-cookie session deadlines. The server emits this only to the socket
    /// authenticated by that cookie; it never enters Hub broadcast history.
    AuthSession { session: serde_json::Value },
    /// Admission state for this logical interactive client. This is sent
    /// directly while a client waits for, acquires, or loses an active seat;
    /// it is never broadcast to another account or client.
    ClientCapacity { capacity: serde_json::Value },
    /// Full session list (sent on connect and whenever it changes).
    Sessions { sessions: Vec<SessionMeta> },
    /// Full enrolled-Machine projection. `resync` marks the deterministic
    /// connect snapshot; live revisions are monotonic within one Controller
    /// process so a delayed async projection cannot overwrite newer state.
    Machines {
        revision: u64,
        machines: Vec<crate::machine_protocol::MachineSummary>,
        #[serde(default)]
        resync: bool,
    },
    /// Marks the end of the deterministic WebSocket connect snapshot. Thin
    /// protocol bridges wait for this before accepting client requests, so
    /// `session/list` cannot race an incomplete session cache.
    BootstrapComplete,
    /// Replay of one session's RECENT log tail. Lazy browser clients request it
    /// over HTTP when focused; legacy/bridge WebSockets receive it at connect.
    /// Capped by count and serialized bytes; older pages are fetched on demand.
    Snapshot {
        session_id: String,
        events: Vec<Envelope>,
        reached_start: bool,
    },
    /// A single live event.
    Event { envelope: Envelope },
    /// Application-level heartbeat, sent to each client on a fixed interval.
    /// Browsers don't expose WS protocol ping/pong to JS, so a client can't tell
    /// a live-but-silent (idle) connection from a HALF-OPEN one (TCP alive, no
    /// data — common on mobile/5G, where `onclose` never fires and the status
    /// silently freezes). This gives the client a steady signal: no message —
    /// heartbeat included — for a couple of intervals means the socket is dead,
    /// so reconnect (→ fresh snapshot). A failed send also reaps a dead client
    /// server-side. Carries no data; the client only reads its arrival time.
    Ping,
    /// Addressed reply on the requesting socket, never broadcast or persisted.
    ConnectionProbe { nonce: u64 },
    /// Agent-advertised per-session config options (mode / model / effort and
    /// whatever else upstream adds). Sent (a) on client connect for every
    /// session whose options were captured during this daemon's lifetime,
    /// and (b) live whenever the agent fires `config_option_update`. The
    /// payload is the raw ACP array — see acp.rs intercept.
    ConfigOptions {
        session_id: String,
        options: serde_json::Value,
    },
    // (Queue + drafts now flow on the generic SyncPatch channel as state
    // "queue:<session_id>", not a dedicated variant — see Hub::emit_pending.)
    /// A generic snapshot patch for one synced `state` (Cowboy state-sync): the
    /// ABSOLUTE `value` at `version`, plus the mutation ids newly confirmed. Sent
    /// on connect as a resync (`confirmed` = every applied id, to seed/heal a
    /// client) and after each accepted [`Inbound::Sync`] (`confirmed` = just that
    /// mutation). The client keeps the highest `version` per state, drops
    /// confirmed pending, and folds `value`. `fromVersion` is implicitly 0
    /// (absolute snapshot). `value` is the state's derived JSON (title map / order
    /// array / queues).
    SyncPatch {
        state: String,
        version: u64,
        value: serde_json::Value,
        confirmed: Vec<String>,
        /// True for a connect/reconnect RESYNC: the client adopts `value` as
        /// ground truth regardless of version (the daemon's version clock resets
        /// on restart, so a reconnecting client must not ignore the lower
        /// post-restart version). False for a live patch (version-gated).
        #[serde(default)]
        resync: bool,
    },
    /// Compatibility tombstone for cached clients from before automatic resume
    /// was retired. New clients ignore this empty snapshot.
    Settings {
        settings: std::collections::HashMap<String, serde_json::Value>,
    },
    /// The Controller's account usage snapshot, including which accounts are
    /// refreshing. Sent whenever it changes so every client shows one state.
    Usage { snapshot: serde_json::Value },
    /// An error to surface to the user (bad command, unknown session, ...).
    /// Broadcast to every connected client — cowboy's "one shared progress"
    /// design means any window watching the same session should see why a
    /// command was rejected, not just the originator.
    Error {
        /// Session the error belongs to, if any. `None` for daemon-level
        /// errors (malformed inbound frame, unknown session id, ...).
        #[serde(default)]
        session_id: Option<String>,
        message: String,
    },
    /// Addressed outcome for a client-authored command that will never take
    /// effect. Unlike `Error` it names the originating `cmid`, so the device
    /// holding that outbox entry can stop retrying and show why; every other
    /// client ignores an id it does not own.
    CommandResult {
        session_id: String,
        cmid: String,
        outcome: CommandOutcome,
        message: String,
    },
}

/// Why a client-authored command was refused for good.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOutcome {
    /// The target session no longer exists for this principal.
    NotFound,
    /// The session exists but refuses this command.
    Rejected,
    /// The session exists but the targeted row already left the queue or
    /// drafts, so the command has nothing left to act on.
    Stale,
}

/// One immutable live frame shared by every WebSocket and the Web Push observer.
/// Structured `Outbound` stays available for observers; JSON is serialized at
/// most once and reused for every socket write.
#[derive(Debug)]
pub struct FanoutFrame {
    outbound: Outbound,
    json: OnceLock<String>,
}

impl FanoutFrame {
    fn new(outbound: Outbound) -> Arc<Self> {
        Arc::new(Self {
            outbound,
            json: OnceLock::new(),
        })
    }

    #[must_use]
    pub fn outbound(&self) -> &Outbound {
        &self.outbound
    }

    /// Cached JSON for this frame. Concurrent first writers may serialize twice;
    /// `OnceLock` keeps one buffer for the ring's lifetime.
    pub fn json(&self) -> Result<&str, serde_json::Error> {
        if let Some(json) = self.json.get() {
            return Ok(json);
        }
        let encoded = serde_json::to_string(&self.outbound)?;
        Ok(self.json.get_or_init(|| encoded))
    }
}

impl std::ops::Deref for FanoutFrame {
    type Target = Outbound;

    fn deref(&self) -> &Self::Target {
        &self.outbound
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HubMemoryStats {
    pub session_count: usize,
    pub hot_log_bytes: usize,
    pub broadcast_last_bytes: usize,
}

/// Persistence intent sent on the write-behind channel from `Hub` to the
/// background DB writer task in `crate::server`. Each variant maps 1:1 to a
/// [`crate::store::Store`] call.
#[derive(Debug, Clone, Serialize)]
pub enum StoreWrite {
    InsertSession(Box<SessionMeta>),
    AppendEvent(Envelope),
    UpdateStatus {
        session_id: String,
        status: Status,
    },
    /// Persist one session-scoped error independently of the lifecycle stream
    /// (for example, a rejected command or an ACP failure reported as text).
    RecordSessionError {
        id: String,
        session_id: String,
        occurred_at_ms: i64,
        message: String,
    },
    UpdateTitle {
        session_id: String,
        title: String,
    },
    UpdateCwd {
        session_id: String,
        cwd: String,
        title: Option<String>,
    },
    ReloadProvider(Box<SessionMeta>),
    SetAgentSessionId {
        session_id: String,
        agent_session_id: Option<String>,
    },
    /// Persist the latest agent-advertised config option snapshot so a fresh
    /// device can render session controls before the worker is warm.
    UpdateConfigOptions {
        session_id: String,
        options: serde_json::Value,
    },
    /// Persist user-selected session config values independently from the
    /// provider's current capability snapshot.
    UpdateConfigPreferences {
        session_id: String,
        preferences: serde_json::Value,
    },
    ClearEvents {
        session_id: String,
    },
    DeleteSession(String),
    /// Persist a session's queue + drafts (whole lists, as JSONB) so staged
    /// messages survive a daemon restart — matching the durability the old
    /// client-side localStorage gave them.
    UpdatePending {
        session_id: String,
        queue: Vec<QueuedMessage>,
        drafts: Vec<QueuedMessage>,
    },
    /// Persist the manual session ordering (a `position` per id) so a drag-
    /// arranged list survives a daemon restart.
    UpdateWorkspaceOrder {
        owner: String,
        order: Vec<String>,
    },
    UpdateSessionOrder {
        order: Vec<String>,
    },
    /// Persist one owner's whole sidebar folder set (bounded by
    /// `session_folders::FOLDER_CAP`, so a whole-set write beats deltas).
    ReplaceSessionFolders {
        owner_user_id: crate::session_folders::FolderOwner,
        folders: Vec<crate::session_folders::SessionFolder>,
    },
    /// Persist explicit sidebar placements (`None` = back to the root).
    UpdateSessionPlacement {
        placements: Vec<(String, Option<String>)>,
    },
    /// Persist one session's Mobile-only code-review workspace state.
    UpdateMobileReviewState {
        session_id: String,
        value: serde_json::Value,
    },
    /// Upsert a session's pending `ScheduleWakeup` so an armed
    /// wakeup survives a daemon restart and still fires.
    UpsertWakeup {
        session_id: String,
        fire_at_ms: i64,
        prompt: String,
    },
    /// Drop a session's persisted wakeup once it has fired (or been dropped).
    DeleteWakeup {
        session_id: String,
    },
    /// Persist one internal auth/admin setting. Never projected to product clients.
    PutSetting {
        key: String,
        value: serde_json::Value,
    },
}

pub(crate) fn estimated_store_write_bytes(write: &StoreWrite) -> usize {
    match write {
        StoreWrite::AppendEvent(envelope) => estimated_envelope_bytes(envelope),
        StoreWrite::UpdateConfigOptions { options, .. }
        | StoreWrite::UpdateConfigPreferences {
            preferences: options,
            ..
        }
        | StoreWrite::UpdateMobileReviewState { value: options, .. } => {
            estimated_json_bytes(options).saturating_add(128)
        }
        StoreWrite::UpdatePending {
            session_id,
            queue,
            drafts,
        } => queue.iter().chain(drafts.iter()).fold(
            session_id.len().saturating_add(64),
            |size, message| {
                size.saturating_add(message.id.len())
                    .saturating_add(message.text.len())
                    .saturating_add(message.cmid.as_deref().map_or(0, str::len))
                    .saturating_add(message.content.iter().fold(64usize, |size, block| {
                        size.saturating_add(estimated_json_bytes(block))
                    }))
            },
        ),
        StoreWrite::UpsertWakeup {
            session_id, prompt, ..
        } => session_id
            .len()
            .saturating_add(prompt.len())
            .saturating_add(32),
        StoreWrite::RecordSessionError {
            id,
            session_id,
            message,
            ..
        } => message
            .len()
            .saturating_add(id.len())
            .saturating_add(session_id.len())
            .saturating_add(64),
        // Metadata and private settings can carry payloads too. Count their
        // complete representation without allocating a second serialized copy.
        _ => {
            let mut count = SerializedBytes(0);
            if serde_json::to_writer(&mut count, write).is_err() {
                return usize::MAX;
            }
            count.0.saturating_add(64)
        }
    }
}

struct SerializedBytes(usize);

impl std::io::Write for SerializedBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Live arbiter state for the title-sync channel (the Cowboy state-sync
/// reference arbiter, in Rust). Coordinates optimistic cross-terminal renames:
/// each accepted mutation bumps `version` and yields a snapshot patch (the whole
/// `titles` override map + the confirmed id). EPHEMERAL by design — durability
/// rides on the existing per-title persistence (`StoreWrite::UpdateTitle`) and
/// the `SessionMeta.title` mirror, so this map holds only live rename overrides
/// and resets (empty, version 0) on restart, while the persisted title reloads
/// into `SessionMeta`. `seen` dedupes a retried mutation so it never double-
/// patches (the arbiter's idempotency half; the client's confirmed-drop is the
/// other half).
/// Per-state bookkeeping for the generic optimistic-sync channel (the
/// Cowboy state-sync reference arbiter, in Rust). One entry per synced state
/// (`"title"`, `"order"`, `"queue:<session>"`, …). `version` is the state's
/// monotonic clock; `seen` dedupes a retried mutation so it never double-patches
/// (the arbiter's idempotency half; the client's confirmed-drop is the other).
/// EPHEMERAL: the VALUE itself is always derived from the typed source of truth
/// (SessionMeta / the order list / the queues), which is what's persisted — so
/// this holds no value, only the clock + dedupe set, and resets on restart while
/// the derived value reloads from pg.
#[derive(Default)]
struct SyncArbiter {
    version: u64,
    seen: HashSet<String>,
}

/// The single source of truth. Cloneable handle (`Arc` inside) shared by the
/// server, the supervisor, and every agent thread's ACP client.
#[derive(Clone)]
pub struct Hub {
    inner: std::sync::Arc<HubInner>,
}

/// Unanswered client-set config values of one session, with when each was set.
type InFlightConfig = HashMap<String, (serde_json::Value, std::time::Instant)>;

struct HubInner {
    sessions: Mutex<HashMap<String, Session>>,
    /// Internal auth/admin state restored from the durable settings table.
    settings: Mutex<HashMap<String, serde_json::Value>>,
    product_permissions: Mutex<product_permissions::Observations>,
    /// Persisted Busy sessions awaiting an authoritative, connected worker
    /// snapshot after the control plane restarts. Broker registry placeholders
    /// do not settle this set; a bounded server-side grace timer finalizes the
    /// remainder as genuine interruptions.
    runtime_reconciliation: Mutex<HashSet<String>>,
    /// Config values a client set that the agent has not yet reported back,
    /// per session. A preset sends several options in a row and the agent
    /// answers each with a full snapshot; the answer to the first still
    /// carries the old values of the rest. Overlaying these keeps that stale
    /// snapshot from flipping the selection back. Lock after `sessions`.
    config_in_flight: Mutex<HashMap<String, InFlightConfig>>,
    /// Newer installed Provider releases sessions may adopt, republished by
    /// the Provider update pass. Never held together with `sessions`.
    provider_update_offers: Mutex<HashMap<String, ProviderUpdateOffer>>,
    /// Canonicalizes the raw ACP stream for the in-memory replay tail. The DB
    /// writer still reduces compact deltas so streaming text coalesces without
    /// enqueueing the accumulated string on every token.
    history_reducer: Mutex<EventReducer>,
    /// Optional content-addressed image store. When present, inline ACP images
    /// are replaced with `/api/artifacts/…` URLs before the Hub clones the
    /// envelope into history, the persistence queue, and live fan-out.
    artifacts: Mutex<Option<crate::artifacts::ArtifactStore>>,
    /// Insertion order of session ids, so the list view is stable.
    order: Mutex<Vec<String>>,
    /// Sessions-sidebar folder tree + explicit placements, the typed truth
    /// behind the `"folders"` sync state (`docs/sessions-folders.md`).
    folders: Mutex<crate::session_folders::SessionFolders>,
    workspace_orders: Mutex<HashMap<String, Vec<String>>>,
    /// Metadata of each owner's draft documents changed this lifetime, behind
    /// the `"drafts"` announcement state. Documents converge over HTTP; this
    /// only tells the owner's other devices which revision to fetch.
    draft_announcements: Mutex<HashMap<String, serde_json::Map<String, serde_json::Value>>>,
    /// Live fan-out to all connected clients. Lagging receivers are dropped by
    /// `broadcast` and simply miss events until their next reconnect snapshot.
    /// One immutable frame is shared by the Web Push observer and every socket:
    /// cloning an `Outbound` per receiver would otherwise duplicate a
    /// multi-megabyte tool result once per connected device.
    tx: broadcast::Sender<Arc<FanoutFrame>>,
    broadcast_last_bytes: AtomicUsize,
    /// Optional write-behind channel to the DB writer. `None` ⇒ in-memory
    /// only (no `--database-url` configured).
    store_tx: Option<StoreSink>,
    /// Hand-off to the background dispatcher task that owns the `Supervisor`.
    /// Set once at startup via [`Hub::set_dispatch_tx`]; `None` until then (and
    /// in tests), in which case a drain decision is computed but no prompt is
    /// actually sent. See [`DispatchReq`].
    dispatch_tx: Mutex<Option<mpsc::Sender<DispatchReq>>>,
    /// Hand-off to the background scheduler task that fires agent-armed
    /// `ScheduleWakeup`s. Set once at startup via [`Hub::set_scheduler_tx`];
    /// `None` until then (and in tests) ⇒ wakeups are simply not honored.
    scheduler_tx: Mutex<Option<mpsc::Sender<crate::scheduler::ScheduleCmd>>>,
    /// Per-state arbiters for the generic optimistic-sync channel, keyed by
    /// state name (`"title"`, `"order"`, …). See [`SyncArbiter`].
    sync: Mutex<HashMap<String, SyncArbiter>>,
    /// Monotonic source of queued/draft message ids (`q1`, `q2`, …). Seeded from
    /// the wall-clock-free counter; uniqueness across a daemon lifetime is all
    /// that's required (ids are list-local keys, not persisted-across-restart
    /// identities — restored lists keep whatever ids they were saved with).
    next_qid: AtomicU64,
    /// Monotonic suffix for durable session-error ids created outside the
    /// transcript sequence. Wall-clock milliseconds make ids restart-safe;
    /// this counter disambiguates multiple errors in the same millisecond.
    next_error_id: AtomicU64,
}

fn set_config_option_current_value(
    options: &mut serde_json::Value,
    config_id: &str,
    value: &serde_json::Value,
) -> bool {
    let Some(options) = options.as_array_mut() else {
        return false;
    };
    let Some(option) = options.iter_mut().find_map(|option| {
        (option.get("id").and_then(serde_json::Value::as_str) == Some(config_id)).then_some(option)
    }) else {
        return false;
    };
    let Some(option) = option.as_object_mut() else {
        return false;
    };
    let key = if option.contains_key("current_value") && !option.contains_key("currentValue") {
        "current_value"
    } else {
        "currentValue"
    };
    option.insert(key.to_owned(), value.clone());
    true
}

fn config_current_value(option: &serde_json::Value) -> Option<&serde_json::Value> {
    option
        .get("currentValue")
        .or_else(|| option.get("current_value"))
}

pub(crate) fn config_option_accepts(option: &serde_json::Value, value: &serde_json::Value) -> bool {
    option
        .get("options")
        .is_none_or(|choices| config_value_list_contains(choices, value))
}

fn config_value_list_contains(options: &serde_json::Value, value: &serde_json::Value) -> bool {
    match options {
        serde_json::Value::Array(options) => options
            .iter()
            .any(|option| config_value_list_contains(option, value)),
        serde_json::Value::Object(option) => {
            option.get("value") == Some(value)
                || option
                    .get("options")
                    .is_some_and(|nested| config_value_list_contains(nested, value))
        }
        _ => options == value,
    }
}

/// Reconcile durable selections with the provider's authoritative option
/// snapshot. A selected value may legitimately differ from `currentValue`
/// while the provider is applying it, but a value that disappeared from the
/// advertised choices must never be replayed into a recreated worker.
fn reconcile_config_preferences(
    options: Option<&serde_json::Value>,
    preferences: &mut serde_json::Value,
) -> bool {
    let Some(options) = options.and_then(serde_json::Value::as_array) else {
        return false;
    };
    let Some(preferences) = preferences.as_object_mut() else {
        return false;
    };
    let changing_model = options.iter().any(|option| {
        option.get("category").and_then(serde_json::Value::as_str) == Some("model")
            && option
                .get("id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| {
                    preferences.get(id).is_some_and(|selected| {
                        config_current_value(option) != Some(selected)
                            && config_option_accepts(option, selected)
                    })
                })
    });
    let mut changed = false;
    for option in options {
        // A queued model change can still receive snapshots for the prior
        // model. Its reasoning limits cannot retire the next model's preset.
        if changing_model
            && option.get("category").and_then(serde_json::Value::as_str) == Some("thought_level")
        {
            continue;
        }
        let Some(config_id) = option.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(selected) = preferences.get(config_id).cloned() else {
            continue;
        };
        if config_option_accepts(option, &selected) {
            continue;
        }
        let replacement = config_current_value(option)
            .filter(|value| {
                matches!(
                    value,
                    serde_json::Value::String(_) | serde_json::Value::Bool(_)
                )
            })
            .cloned();
        match replacement {
            Some(replacement) if replacement != selected => {
                preferences.insert(config_id.to_owned(), replacement);
                changed = true;
            }
            Some(_) => {}
            None => {
                preferences.remove(config_id);
                changed = true;
            }
        }
    }
    changed
}

impl Hub {
    #[must_use]
    pub fn new() -> Self {
        Self::with_store(None)
    }

    /// Hub plus a write-behind channel. The receiver half is owned by the
    /// DB writer task (spawned in `crate::server`).
    #[must_use]
    pub fn with_store(store_tx: Option<StoreSink>) -> Self {
        // Shared fan-out buffer. A client that falls this many events behind
        // LAGS and the broadcast drops its missed events (the server then closes
        // it to force a resync — see server.rs). One long autonomous turn (a book
        // chapter) can emit hundreds of chunks, so a roomy buffer keeps a briefly
        // slow mobile client from lagging on a normal blip. It is a single shared
        // ring, but tool and image events are not guaranteed to be small.
        // A slow/backgrounded client is closed and resnapshotted on lag, so
        // retaining thousands of potentially multi-megabyte tool/image events
        // only pins heap without improving correctness. 1,024 still absorbs a
        // long burst while bounding the shared ring's retained payload.
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            inner: std::sync::Arc::new(HubInner {
                sessions: Mutex::new(HashMap::new()),
                settings: Mutex::new(HashMap::new()),
                product_permissions: Mutex::new(product_permissions::Observations::default()),
                runtime_reconciliation: Mutex::new(HashSet::new()),
                config_in_flight: Mutex::new(HashMap::new()),
                provider_update_offers: Mutex::new(HashMap::new()),
                history_reducer: Mutex::new(EventReducer::default()),
                artifacts: Mutex::new(None),
                order: Mutex::new(Vec::new()),
                folders: Mutex::new(crate::session_folders::SessionFolders::default()),
                workspace_orders: Mutex::new(HashMap::new()),
                draft_announcements: Mutex::new(HashMap::new()),
                tx,
                broadcast_last_bytes: AtomicUsize::new(0),
                store_tx,
                dispatch_tx: Mutex::new(None),
                scheduler_tx: Mutex::new(None),
                sync: Mutex::new(HashMap::new()),
                next_qid: AtomicU64::new(1),
                next_error_id: AtomicU64::new(1),
            }),
        }
    }

    /// Wire the background dispatcher's hand-off channel. Called once at startup
    /// (in `crate::server`) after the dispatcher task is spawned, before any
    /// client connects. Until set, drains compute but dispatch nothing.
    pub fn set_dispatch_tx(&self, tx: mpsc::Sender<DispatchReq>) {
        *self.inner.dispatch_tx.lock() = Some(tx);
    }

    /// Wire the background scheduler's hand-off channel (mirrors
    /// [`Self::set_dispatch_tx`]). Until set, [`Self::schedule_wakeup`] is a no-op.
    pub fn set_scheduler_tx(&self, tx: mpsc::Sender<crate::scheduler::ScheduleCmd>) {
        *self.inner.scheduler_tx.lock() = Some(tx);
    }

    pub fn set_artifacts(&self, artifacts: crate::artifacts::ArtifactStore) {
        *self.inner.artifacts.lock() = Some(artifacts);
    }

    /// Store a prompt's large images ahead of dispatch. `true` lets a remote
    /// worker echo them by digest, which [`Self::push_tagged`] resolves.
    pub fn store_prompt_images(&self, content: &[serde_json::Value]) -> bool {
        let Some(artifacts) = self.inner.artifacts.lock().clone() else {
            return false;
        };
        artifacts
            .store_prompt_images(content)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "prompt images stay inline in the echo");
                false
            })
    }

    #[must_use]
    pub fn memory_stats(&self) -> HubMemoryStats {
        let sessions = self.inner.sessions.lock();
        HubMemoryStats {
            session_count: sessions.len(),
            hot_log_bytes: sessions.values().map(|session| session.log_bytes).sum(),
            broadcast_last_bytes: self.inner.broadcast_last_bytes.load(Ordering::Relaxed),
        }
    }

    fn fanout(&self, outbound: Outbound) {
        // A send error just means no clients or internal observers are
        // connected. The canonical hot tail remains authoritative.
        let bytes = match &outbound {
            Outbound::Event { envelope } => estimated_envelope_bytes(envelope),
            _ => 256,
        };
        self.inner
            .broadcast_last_bytes
            .store(bytes, Ordering::Relaxed);
        let _ = self.inner.tx.send(FanoutFrame::new(outbound));
    }

    /// Publish the current account usage snapshot to every client.
    pub fn broadcast_usage(&self, snapshot: serde_json::Value) {
        self.fanout(Outbound::Usage { snapshot });
    }

    /// Publish one authoritative Machine registry revision over the existing
    /// product WebSocket. Machine state remains owned by the durable Store;
    /// Hub is transport only and retains no second copy.
    pub fn broadcast_machines(
        &self,
        revision: u64,
        machines: Vec<crate::machine_protocol::MachineSummary>,
    ) {
        self.fanout(Outbound::Machines {
            revision,
            machines,
            resync: false,
        });
    }

    /// Arm (replace) a session's pending `ScheduleWakeup` — `acp.rs` calls this
    /// when it intercepts the tool. `delay_seconds` is the agent-requested delay
    /// (clamped by the scheduler); the wakeup fires `prompt` as its own turn.
    /// Also persisted in the durable baseline so it survives a restart.
    pub fn schedule_wakeup(&self, session_id: &str, delay_seconds: i64, prompt: String) {
        let fire_at_ms = crate::scheduler::fire_at_from_delay(delay_seconds);
        if let Some(tx) = self.inner.scheduler_tx.lock().as_ref() {
            let _ = tx.try_send(crate::scheduler::ScheduleCmd::Arm {
                session_id: session_id.to_owned(),
                fire_at_ms,
                prompt: prompt.clone(),
            });
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpsertWakeup {
                session_id: session_id.to_owned(),
                fire_at_ms,
                prompt,
            });
        }
    }

    /// Re-arm a persisted wakeup on startup (absolute `fire_at_ms`, no re-persist
    /// — it's already in the DB). An already-overdue one fires immediately
    /// (catch-up for time the daemon was down).
    pub fn rearm_wakeup(&self, session_id: &str, fire_at_ms: i64, prompt: String) {
        if let Some(tx) = self.inner.scheduler_tx.lock().as_ref() {
            let _ = tx.try_send(crate::scheduler::ScheduleCmd::Arm {
                session_id: session_id.to_owned(),
                fire_at_ms,
                prompt,
            });
        }
    }

    /// Drop a session's persisted wakeup — called by the scheduler once it has
    /// consumed (fired or dropped) the pending wakeup, so it won't re-fire on the
    /// next restart.
    pub fn clear_persisted_wakeup(&self, session_id: &str) {
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::DeleteWakeup {
                session_id: session_id.to_owned(),
            });
        }
    }

    /// Tell the scheduler a human turn arrived for a session, resetting its
    /// consecutive-wakeup runaway guard. No-op if the scheduler isn't wired.
    fn notify_human_turn(&self, session_id: &str) {
        if let Some(tx) = self.inner.scheduler_tx.lock().as_ref() {
            let _ = tx.try_send(crate::scheduler::ScheduleCmd::HumanTurn {
                session_id: session_id.to_owned(),
            });
        }
    }

    /// Arm (or replace) a scheduled DRAFT's timer at absolute `fire_at_ms`. The
    /// draft itself (with its `schedule`) is the persisted record — this only
    /// drives the in-memory timer, so it's used both for a fresh schedule and
    /// the startup re-arm (an overdue time fires immediately, catch-up).
    fn arm_draft_timer(&self, session_id: &str, draft_id: &str, fire_at_ms: i64) {
        if let Some(tx) = self.inner.scheduler_tx.lock().as_ref() {
            let _ = tx.try_send(crate::scheduler::ScheduleCmd::ArmDraft {
                session_id: session_id.to_owned(),
                draft_id: draft_id.to_owned(),
                fire_at_ms,
            });
        }
    }

    /// Cancel a scheduled draft's timer — called whenever the draft leaves its
    /// scheduled state (unscheduled, removed, manually activated, moved, cleared)
    /// so a dropped draft can't still fire. No-op if the scheduler isn't wired.
    fn cancel_draft_timer(&self, session_id: &str, draft_id: &str) {
        if let Some(tx) = self.inner.scheduler_tx.lock().as_ref() {
            let _ = tx.try_send(crate::scheduler::ScheduleCmd::CancelDraft {
                session_id: session_id.to_owned(),
                draft_id: draft_id.to_owned(),
            });
        }
    }

    /// Re-arm every persisted scheduled draft on startup (absolute fire times, no
    /// re-persist — they're already in the drafts jsonb). Scans in-memory sessions
    /// AFTER restore. An already-overdue schedule fires immediately (catch-up for
    /// downtime), mirroring [`Self::rearm_wakeup`].
    pub fn rearm_scheduled_drafts(&self) {
        let arms: Vec<(String, String, i64)> = {
            let sessions = self.inner.sessions.lock();
            sessions
                .values()
                .flat_map(|s| {
                    let sid = s.meta.id.clone();
                    s.drafts.iter().filter_map(move |m| {
                        m.schedule
                            .as_ref()
                            .map(|sc| (sid.clone(), m.id.clone(), sc.fire_at_ms))
                    })
                })
                .collect()
        };
        for (sid, did, fire_at_ms) in arms {
            self.arm_draft_timer(&sid, &did, fire_at_ms);
        }
    }

    /// Populate the in-memory state from a previously-stored snapshot.
    /// Should be called once at startup, BEFORE any client connects, so the
    /// `Sessions` broadcast on first connect already includes everything.
    /// Skips ordinary write-behind: these rows are already in the DB. Durable
    /// compatibility repairs discovered during restore are written back.
    ///
    /// Without runtime reconciliation, restored sessions are forced to a dead
    /// state. Production startup instead uses
    /// [`Self::restore_reconciling_runtime`], because detached Machine workers
    /// can outlive the controller and authoritatively reclaim a persisted Busy
    /// turn during the bounded reconnect window.
    ///
    /// The persisted status still tells us what it was doing when we died,
    /// and we keep that one bit: a session that was `Busy` (a turn in flight)
    /// becomes [`Status::Interrupted`] — "your last turn never finished" — while
    /// an idle/alive one just becomes `Exited` (dormant, nothing unfinished).
    /// The write-behind store applies the `Busy` write within ms of a turn
    /// starting, so for any turn that ran more than an instant the bit is
    /// durable before a restart (store.rs accepts the sub-ms crash window).
    /// Restore persisted state before Machine runtimes reconnect.
    ///
    /// Persisted Busy sessions retain their Busy/in-flight guard during the
    /// bounded reconnect window. A connected worker snapshot adopts them; the
    /// server later calls [`Self::finalize_runtime_reconciliation`] for any
    /// session that still has no owner. This ordering is what makes normal
    /// control-plane deployment transparent to detached workers.
    pub fn restore_reconciling_runtime(&self, sessions: Vec<RestoredSession>) {
        self.restore_impl(sessions, &[], true);
    }

    /// Restore the persisted sidebar folder tree. Session placements ride on
    /// each [`RestoredSession::folder_id`] instead.
    pub(crate) fn restore_workspace_orders(&self, orders: Vec<(String, Vec<String>)>) {
        *self.inner.workspace_orders.lock() = orders.into_iter().collect();
    }

    /// Announce one owner's changed draft `metadata` (no body) to that owner's
    /// devices. The live patch carries only this document; a reconnect resync
    /// carries every document announced this lifetime.
    pub(crate) fn announce_draft(&self, owner: &str, metadata: serde_json::Value) {
        let Some(id) = metadata.get("id").and_then(serde_json::Value::as_str) else {
            return;
        };
        let id = id.to_owned();
        {
            let mut announcements = self.inner.draft_announcements.lock();
            let documents = announcements.entry(owner.to_owned()).or_default();
            let newer = documents
                .get(&id)
                .and_then(|previous| previous.get("revision"))
                .and_then(serde_json::Value::as_i64)
                .is_none_or(|previous| {
                    metadata
                        .get("revision")
                        .and_then(serde_json::Value::as_i64)
                        .is_some_and(|revision| revision > previous)
                });
            if !newer {
                return;
            }
            documents.insert(id.clone(), metadata.clone());
        }
        self.sync_emit(
            "drafts",
            serde_json::json!({ owner: { id: metadata } }),
            Vec::new(),
        );
    }

    pub fn restore_session_folders(&self, folders: Vec<crate::session_folders::SessionFolder>) {
        self.inner.folders.lock().set_folders(folders);
    }

    /// Restore persisted sessions while reconciling detached runtime workers.
    /// A persisted `Busy` row is interrupted only when no matching live worker
    /// exists. This prevents a control-plane deploy from generating a false
    /// interruption marker while the original ACP prompt is still running in
    /// its detached worker.
    #[cfg(test)]
    fn restore_with_workers(&self, sessions: Vec<RestoredSession>, workers: &[WorkerSnapshot]) {
        self.restore_impl(sessions, workers, false);
    }

    fn restore_impl(
        &self,
        sessions: Vec<RestoredSession>,
        workers: &[WorkerSnapshot],
        defer_missing_busy: bool,
    ) {
        let live: HashMap<&str, &WorkerSnapshot> = workers
            .iter()
            .filter(|worker| worker.has_connected_owner())
            .map(|worker| (worker.session_id.as_str(), worker))
            .collect();
        // Seed the qid counter PAST every restored id. The counter (`next_qid`)
        // is in-memory and resets to 1 on each daemon restart, so without this a
        // draft/queued message created after a restart reuses q1, q2, … and
        // collides with a pre-restart one — duplicate ids, which the client keys
        // rows by, so a "3 Drafts" header renders only 2 distinct rows. Done
        // before the loop so the dedup below can mint fresh ids past the max.
        let mut max_qid = 0u64;
        for r in &sessions {
            for m in r.queue.iter().chain(r.drafts.iter()) {
                if let Some(n) = m.id.strip_prefix('q').and_then(|s| s.parse::<u64>().ok()) {
                    max_qid = max_qid.max(n);
                }
            }
        }
        self.inner.next_qid.store(max_qid + 1, Ordering::Relaxed);

        // Sessions whose turn was cut off by the restart (persisted `Busy`).
        // Collected under the lock, marked after it's released — `push` below
        // re-locks `sessions`, so holding the lock here would deadlock.
        let mut interrupted: Vec<String> = Vec::new();
        // Sessions whose pending lists changed during compatibility repair →
        // persist after restore. This includes duplicate-id healing and removal
        // of retired synthetic continuations left by an older controller.
        let mut pending_dirty: Vec<String> = Vec::new();
        // Provider option snapshots are authoritative over persisted selections.
        // Heal values retired by a model/provider upgrade before any worker can
        // replay them, then write the repaired preference object back to storage.
        let mut config_preferences_dirty: Vec<(String, serde_json::Value)> = Vec::new();
        // Ids already seen across ALL sessions — ids must be globally unique so a
        // later cross-session move can't collide. The first occurrence keeps its
        // id; a duplicate (corruption from the old counter-reset bug) gets a fresh
        // one past `max_qid`.
        let mut seen: HashSet<String> = HashSet::new();
        let mut placements: Vec<(String, String)> = Vec::new();
        {
            let mut sessions_lock = self.inner.sessions.lock();
            let mut order = self.inner.order.lock();
            let mut history_reducer = self.inner.history_reducer.lock();
            for r in sessions {
                let RestoredSession {
                    mut meta,
                    mut log,
                    event_count,
                    mut reached_start,
                    next_seq,
                    mut queue,
                    mut drafts,
                    config_options,
                    mut config_preferences,
                    mobile_review_state,
                    folder_id,
                } = r;
                let mut healed = false;
                let queue_len = queue.len();
                let drafts_len = drafts.len();
                queue.retain(|message| {
                    !message
                        .cmid
                        .as_deref()
                        .is_some_and(|cmid| cmid.starts_with(LEGACY_AUTO_CONTINUE_PREFIX))
                });
                drafts.retain(|message| {
                    !message
                        .cmid
                        .as_deref()
                        .is_some_and(|cmid| cmid.starts_with(LEGACY_AUTO_CONTINUE_PREFIX))
                });
                let removed_legacy_continuation =
                    queue.len() != queue_len || drafts.len() != drafts_len;
                for m in queue.iter_mut().chain(drafts.iter_mut()) {
                    if !seen.insert(m.id.clone()) {
                        m.id = self.next_qid();
                        seen.insert(m.id.clone());
                        healed = true;
                    }
                }
                let id = meta.id.clone();
                if reconcile_config_preferences(config_options.as_ref(), &mut config_preferences) {
                    config_preferences_dirty.push((id.clone(), config_preferences.clone()));
                }
                let runtime = live.get(id.as_str()).copied();
                let was_busy = meta.status == Status::Busy && runtime.is_none();
                meta.status = match runtime.map(|worker| worker.state) {
                    Some(WorkerState::Starting) => Status::Starting,
                    Some(WorkerState::Running) => Status::Running,
                    Some(WorkerState::Busy) => Status::Busy,
                    Some(WorkerState::Draining) => {
                        if runtime.is_some_and(|worker| worker.current_turn_id.is_some()) {
                            Status::Busy
                        } else {
                            Status::Running
                        }
                    }
                    Some(WorkerState::Exited) => Status::Exited,
                    Some(WorkerState::Crashed) => Status::Crashed,
                    None => match meta.status {
                        // No detached owner survived: preserve the original
                        // restart-recovery behavior unless production startup
                        // is still inside its bounded runtime reconnect window.
                        Status::Busy if defer_missing_busy => Status::Busy,
                        Status::Busy => Status::Interrupted,
                        Status::Exited | Status::Crashed | Status::Interrupted => meta.status,
                        Status::Running | Status::Starting => Status::Exited,
                    },
                };
                if let Some(agent_session_id) =
                    runtime.and_then(|worker| worker.agent_session_id.clone())
                {
                    meta.agent_session_id = Some(agent_session_id);
                }
                if was_busy {
                    if defer_missing_busy {
                        self.inner.runtime_reconciliation.lock().insert(id.clone());
                    } else {
                        interrupted.push(id.clone());
                    }
                }
                if healed || removed_legacy_continuation {
                    pending_dirty.push(id.clone());
                }
                for envelope in &mut log {
                    crate::persistence::compact_canonical_tool_output(envelope);
                }
                let mut log_bytes = log.iter().fold(0usize, |size, envelope| {
                    size.saturating_add(estimated_envelope_bytes(envelope))
                });
                if self.inner.store_tx.is_some()
                    && trim_hot_log(
                        &mut log,
                        &mut log_bytes,
                        false,
                        hot_tail_budget_bytes(meta.status),
                    )
                {
                    reached_start = false;
                }
                for envelope in &log {
                    let _ = history_reducer.reduce(envelope.clone());
                }
                sessions_lock.insert(
                    id.clone(),
                    Session {
                        meta,
                        code_incarnation: code_scope::CodeIncarnation::default(),
                        machine_lineage: None,
                        log,
                        log_bytes,
                        event_count,
                        reached_start,
                        next_seq,
                        last_activity: std::time::Instant::now(),
                        config_options,
                        config_preferences,
                        queue,
                        drafts,
                        editing: None,
                        editing_epoch: 0,
                        in_flight: (defer_missing_busy && was_busy)
                            || runtime.is_some_and(|worker| {
                                worker.current_turn_id.is_some() || worker.pending_prompt_count > 0
                            }),
                        in_flight_epoch: 0,
                        dispatched_cmids: VecDeque::new(),
                        turn_completion_latched: false,
                        lifecycle_epoch: 0,
                        mobile_review: MobileReviewState::from_stored(mobile_review_state),
                    },
                );
                if let Some(folder_id) = folder_id {
                    placements.push((id.clone(), folder_id));
                }
                order.push(id);
            }
        }
        {
            let mut folders = self.inner.folders.lock();
            for (session_id, folder_id) in placements {
                folders.restore_placement(session_id, folder_id);
            }
        }
        // For each interrupted session: persist the corrected status AND append a
        // permanent timeline marker. The live status is ephemeral — a resume
        // overwrites it — but this Lifecycle entry stays in the log forever, so
        // "this turn was cut off" is visible after the fact too. Idempotent across
        // repeated restarts: the status write-back flips the persisted value off
        // `busy`, so the next restore reads `interrupted` and adds no second marker
        // (only a fresh `busy` → interrupt does).
        for id in interrupted {
            self.record_restart_interruption(&id);
        }
        // Persist repaired pending lists so a retired continuation cannot return
        // after another restart and healed ids remain globally unique.
        for id in pending_dirty {
            self.emit_pending(&id);
        }
        for (session_id, preferences) in config_preferences_dirty {
            if let Some(tx) = self.inner.store_tx.as_ref() {
                let _ = tx.send(StoreWrite::UpdateConfigPreferences {
                    session_id,
                    preferences,
                });
            }
        }
    }

    /// Reader-first fence shared by snapshots and streamed events. A native
    /// id, config update or permission request cannot adopt an unsupported
    /// environment through the event path while snapshots reject it.
    pub(crate) fn accepts_runtime_projection(&self, session_id: &str) -> bool {
        !self
            .inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|session| session.meta.require_runtime_launch().is_err())
    }

    /// Decide whether an incoming runtime snapshot may project lifecycle state
    /// into the Hub. A real connected owner atomically settles startup
    /// reconciliation. A broker-only placeholder is ignored while a persisted
    /// Busy turn is still waiting for its owner, so it cannot overwrite Busy
    /// with a speculative Starting/Running state.
    pub fn accept_runtime_snapshot(&self, worker: &WorkerSnapshot) -> bool {
        if !self.accepts_runtime_projection(&worker.session_id) {
            return false;
        }
        if self.session_info(&worker.session_id).is_some_and(|info| {
            info.meta.execution_binding.is_some()
                && !worker.launch.as_ref().is_some_and(|launch| {
                    launch.execution_binding == info.meta.execution_binding
                        && launch.provider == info.meta.provider
                        && launch.provider_version == info.meta.provider_version
                        && launch.provider_generation_digest == info.meta.provider_generation_digest
                        && launch.provider_auth_generation == info.meta.provider_auth_generation
                        && launch.cwd == info.meta.cwd
                })
        }) {
            return false;
        }
        if worker.has_connected_owner() {
            let settling = self
                .inner
                .runtime_reconciliation
                .lock()
                .remove(&worker.session_id);
            if settling {
                if worker.owns_in_flight_turn() {
                    tracing::info!(
                        session = %worker.session_id,
                        worker_epoch = %worker.worker_epoch,
                        "detached worker adopted restored in-flight turn"
                    );
                } else {
                    tracing::warn!(
                        session = %worker.session_id,
                        worker_epoch = %worker.worker_epoch,
                        state = ?worker.state,
                        "detached worker reconnected without the restored turn"
                    );
                    self.record_restart_interruption(&worker.session_id);
                }
            }
            true
        } else {
            !self
                .inner
                .runtime_reconciliation
                .lock()
                .contains(&worker.session_id)
        }
    }

    /// Project a worker lifecycle into the Hub. An idle Running snapshot must
    /// not hide a restart-interruption that just recorded the lost prompt.
    pub fn project_runtime_status(&self, session_id: &str, status: Status, detail: Option<String>) {
        if status == Status::Running && self.status(session_id) == Some(Status::Interrupted) {
            return;
        }
        self.set_status(session_id, status, detail);
    }

    /// Finalize persisted Busy sessions whose detached owner did not reconnect
    /// within the server's bounded grace period. Returns exactly the sessions
    /// newly marked Interrupted for observability.
    pub fn finalize_runtime_reconciliation(&self) -> Vec<String> {
        let pending = std::mem::take(&mut *self.inner.runtime_reconciliation.lock());
        let interrupted: Vec<String> = pending
            .into_iter()
            .filter(|id| self.status(id) == Some(Status::Busy))
            .collect();
        for id in &interrupted {
            self.record_restart_interruption(id);
        }
        interrupted
    }

    fn record_restart_interruption(&self, session_id: &str) {
        self.set_status(
            session_id,
            Status::Interrupted,
            Some("turn cut off by a cowboy restart — it never finished".to_owned()),
        );
    }

    /// The newest event sequence already assigned in a session, used as a
    /// managed call's cursor before its prompt is submitted.
    #[must_use]
    pub fn last_event_seq(&self, session_id: &str) -> Option<u64> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .map(|s| s.next_seq.saturating_sub(1))
    }

    /// Observe one managed child turn after `after_seq` from the hot log.
    /// The final assistant text is the agent message after the turn's last
    /// non-message update, which is what a native final response contains.
    #[must_use]
    pub fn managed_turn(&self, session_id: &str, after_seq: u64) -> Option<ManagedTurn> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        let mut turn = ManagedTurn {
            prompted: s.in_flight
                || s.queue.iter().any(|m| {
                    m.cmid
                        .as_deref()
                        .is_some_and(|c| c.starts_with(MANAGED_CALL_CMID_PREFIX))
                }),
            stop_reason: None,
            final_text: String::new(),
            status: s.meta.status,
        };
        let mut final_text = String::new();
        for entry in s.log.iter().filter(|entry| entry.seq > after_seq) {
            match &entry.event {
                Event::Update { update } => {
                    match update
                        .get("sessionUpdate")
                        .and_then(serde_json::Value::as_str)
                    {
                        Some("user_message_chunk") => turn.prompted = true,
                        Some("agent_message_chunk") => {
                            if let Some(text) = update
                                .get("content")
                                .and_then(|content| content.get("text"))
                                .and_then(serde_json::Value::as_str)
                            {
                                final_text.push_str(text);
                            }
                        }
                        Some(
                            "agent_thought_chunk"
                            | "usage_update"
                            | "available_commands_update"
                            | "current_mode_update"
                            | "config_option_update",
                        )
                        | None => {}
                        Some(_) => final_text.clear(),
                    }
                }
                Event::TurnEnd { stop_reason } => {
                    turn.stop_reason = Some(stop_reason.clone());
                    break;
                }
                _ => {}
            }
        }
        turn.final_text = final_text;
        Some(turn)
    }

    /// Submit a managed call's prompt to its child. Client entry points refuse
    /// managed children; only the Controller-owned call runner reaches here.
    pub fn submit_managed_prompt(&self, session_id: &str, call_id: &str, text: String) {
        self.submit_inner(
            session_id,
            text,
            Vec::new(),
            Some(format!("{MANAGED_CALL_CMID_PREFIX}{call_id}")),
            true,
        );
    }

    fn is_managed_child(&self, session_id: &str) -> bool {
        self.inner.sessions.lock().get(session_id).is_some_and(|s| {
            s.meta
                .execution_binding
                .as_ref()
                .is_some_and(|binding| binding.managed().is_some())
        })
    }

    /// Subscribe to the live event stream.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<FanoutFrame>> {
        self.inner.tx.subscribe()
    }

    /// Current session list (insertion order).
    #[must_use]
    pub fn session_list(&self) -> Vec<SessionMeta> {
        self.session_list_filtered(|_| true)
    }

    /// Session list after applying `keep` to each row's `owner_user_id`.
    #[must_use]
    pub fn session_list_filtered(&self, keep: impl Fn(Option<&str>) -> bool) -> Vec<SessionMeta> {
        let offers = self.inner.provider_update_offers.lock().clone();
        let now = now_ms();
        let sessions = self.inner.sessions.lock();
        let order = self.inner.order.lock();
        order
            .iter()
            .filter_map(|id| {
                sessions.get(id).and_then(|s| {
                    keep(s.meta.owner_user_id.as_deref()).then(|| {
                        let mut meta = s.meta.clone();
                        // Surface the soonest scheduled-draft fire so the session-row
                        // clock badge can show it, without shipping the drafts here.
                        meta.next_schedule_ms = s
                            .drafts
                            .iter()
                            .filter_map(|m| m.schedule.as_ref().map(|sc| sc.fire_at_ms))
                            .min();
                        meta.provider_update_available = offers
                            .get(id)
                            .filter(|offer| {
                                meta.provider_update.is_none()
                                    && offer.digest != meta.provider_generation_digest
                            })
                            .map(|offer| ProviderUpdateAvailable {
                                version: offer.version.clone(),
                                digest: offer.digest.clone(),
                                when_idle: offer.when_idle,
                                automatic_at_ms: offer.automatic_after.map(|after| {
                                    let remaining = after.saturating_sub(s.last_activity.elapsed());
                                    now.saturating_add(
                                        i64::try_from(remaining.as_millis()).unwrap_or(i64::MAX),
                                    )
                                }),
                            });
                        meta
                    })
                })
            })
            .collect()
    }

    /// Product user id stamped on this session. `None` if the session is
    /// unknown or still in the unowned shared pool.
    #[must_use]
    pub fn session_owner_user_id(&self, session_id: &str) -> Option<String> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .and_then(|session| session.meta.owner_user_id.clone())
    }

    /// Whether a live session exists and is stamped for `user_id`.
    #[must_use]
    pub fn owned_by_product_user(&self, session_id: &str, user_id: &str) -> bool {
        self.session_owner_user_id(session_id).as_deref() == Some(user_id)
    }

    /// Per-session info (metadata + live event/queue/draft counts) for the
    /// session-info dialog. `None` for an unknown session.
    #[must_use]
    pub fn session_info(&self, session_id: &str) -> Option<SessionInfo> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        Some(SessionInfo {
            meta: s.meta.clone(),
            event_count: s.event_count,
            queue_count: s.queue.len(),
            drafts_count: s.drafts.len(),
        })
    }

    /// Whether a session is machine-driven and view-only. The WS dispatch
    /// rejects user-driven turns for these; only the backend wake endpoint
    /// (`POST /api/sessions/{id}/prompt`) drives them.
    #[must_use]
    pub fn session_is_system(&self, session_id: &str) -> bool {
        let sessions = self.inner.sessions.lock();
        sessions.get(session_id).is_some_and(|s| s.meta.system)
    }

    /// Replace every session's Provider update offer. Clients see the change
    /// through the ordinary session list broadcast.
    pub fn publish_provider_update_offers(&self, offers: HashMap<String, ProviderUpdateOffer>) {
        let changed = {
            let mut current = self.inner.provider_update_offers.lock();
            let changed = *current != offers;
            *current = offers;
            changed
        };
        if changed {
            self.broadcast_sessions();
        }
    }

    /// Drop one offer after its update was applied, together with the
    /// one-shot "update when idle" request that may have caused it.
    pub fn settle_provider_update_offer(&self, session_id: &str) {
        self.inner.provider_update_offers.lock().remove(session_id);
        self.set_provider_update_when_idle(session_id, false);
    }

    /// Whether the user asked this session to adopt the installed Provider
    /// release as soon as it is idle. Persisted; consumed by the update pass.
    #[must_use]
    pub fn provider_update_when_idle(&self, session_id: &str) -> bool {
        self.inner
            .settings
            .lock()
            .get(&format!(
                "{}{session_id}",
                settings_keys::SESSION_PROVIDER_UPDATE_WHEN_IDLE_PREFIX
            ))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    }

    /// Record or cancel the one-shot request and reflect it in the offer.
    pub fn set_provider_update_when_idle(&self, session_id: &str, when_idle: bool) {
        if self.provider_update_when_idle(session_id) != when_idle {
            self.set_setting(
                format!(
                    "{}{session_id}",
                    settings_keys::SESSION_PROVIDER_UPDATE_WHEN_IDLE_PREFIX
                ),
                serde_json::json!(when_idle),
            );
        }
        let changed = self
            .inner
            .provider_update_offers
            .lock()
            .get_mut(session_id)
            .is_some_and(|offer| {
                let changed = offer.when_idle != when_idle;
                offer.when_idle = when_idle;
                changed
            });
        if changed {
            self.broadcast_sessions();
        }
    }

    /// How long ago this Controller last appended an event to the session.
    #[must_use]
    pub fn session_idle_for(&self, session_id: &str) -> Option<std::time::Duration> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .map(|session| session.last_activity.elapsed())
    }

    #[must_use]
    pub fn session_has_in_flight_prompt(&self, session_id: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|session| session.in_flight)
    }

    /// Identity of the prompt guard currently held by this session.
    #[must_use]
    pub fn in_flight_prompt_epoch(&self, session_id: &str) -> Option<u64> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .filter(|session| session.in_flight)
            .map(|session| session.in_flight_epoch)
    }

    /// Total events held in memory across all live sessions — the event-count
    /// metric for the info panel.
    #[must_use]
    pub fn event_total(&self) -> u64 {
        let sessions = self.inner.sessions.lock();
        sessions.values().map(|s| s.event_count).sum()
    }

    /// Recent log TAIL for a fresh client (last [`SNAPSHOT_TAIL`] events) plus
    /// `reached_start` = whether the tail IS the whole log. Older pages are
    /// fetched on demand over HTTP (`history_page`), not shipped here — a long
    /// session must not re-send its entire history on every connect/reconnect.
    #[must_use]
    pub fn snapshot(&self, session_id: &str) -> Option<(Vec<Envelope>, bool)> {
        let sessions = self.inner.sessions.lock();
        sessions.get(session_id).map(|s| {
            let len = s.log.len();
            let count_start = len.saturating_sub(SNAPSHOT_TAIL);
            let mut start = len;
            let mut serialized_bytes = 0usize;
            for index in (count_start..len).rev() {
                let event_bytes = serde_json::to_vec(&s.log[index]).map_or(0, |event| event.len());
                if serialized_bytes > 0
                    && serialized_bytes.saturating_add(event_bytes) > SNAPSHOT_MAX_BYTES
                {
                    break;
                }
                start = index;
                serialized_bytes = serialized_bytes.saturating_add(event_bytes);
            }
            // A rich user prompt is echoed as one consecutive event per ACP
            // content block (typically image, then text). The byte budget may
            // otherwise cut between those blocks, making the fresh client show
            // only the text while the image remains stranded in the previous
            // history page. Keep the prompt atomic at the snapshot boundary;
            // one user attachment is allowed to exceed the soft bootstrap
            // budget so the transcript never misrepresents what was sent.
            if s.log.get(start).is_some_and(is_user_message_chunk) {
                while start > count_start && is_user_message_chunk(&s.log[start - 1]) {
                    start -= 1;
                }
            }
            (s.log[start..].to_vec(), s.reached_start && start == 0)
        })
    }

    /// Up to [`HISTORY_PAGE`] events older than `before_seq`. Cursor pagination
    /// remains efficient when canonicalization leaves gaps in the durable seqs.
    /// Returns ascending events plus the next cursor and whether the beginning
    /// of the retained in-memory window was reached.
    #[must_use]
    pub fn history_page(
        &self,
        session_id: &str,
        before_seq: u64,
    ) -> Option<(Vec<Envelope>, Option<u64>, bool)> {
        let sessions = self.inner.sessions.lock();
        sessions.get(session_id).map(|s| {
            let end = s.log.partition_point(|e| e.seq < before_seq);
            let count_start = end.saturating_sub(HISTORY_PAGE);
            let candidates = s.log[count_start..end].to_vec();
            let candidate_count = candidates.len();
            let events = bound_history_page(candidates);
            let reached_start =
                s.reached_start && count_start == 0 && events.len() == candidate_count;
            let next_before_seq =
                (!reached_start).then(|| events.first().map_or(before_seq, |event| event.seq));
            (events, next_before_seq, reached_start)
        })
    }

    #[must_use]
    pub fn question_page_before(
        &self,
        session_id: &str,
        before_seq: u64,
    ) -> Option<(Vec<Envelope>, Option<u64>, bool)> {
        let sessions = self.inner.sessions.lock();
        sessions.get(session_id).map(|session| {
            let end = session.log.partition_point(|event| event.seq < before_seq);
            let roots = session.log[..end]
                .iter()
                .enumerate()
                .filter_map(|(index, event)| {
                    let previous_was_user =
                        index > 0 && is_user_message_chunk(&session.log[index - 1]);
                    (is_human_question_chunk(event) && !previous_was_user).then_some(index)
                })
                .collect::<Vec<_>>();
            let Some(&root_index) = roots.last() else {
                return (Vec::new(), None, true);
            };
            // A question page describes one conversational turn. Background
            // terminals may continue to emit after TurnEnd; including that
            // unbounded tail makes a page grow forever and can strand the next
            // bootstrap behind thousands of non-renderable tool deltas.
            let page_end = session.log[root_index..end]
                .iter()
                .position(is_turn_end)
                .map_or(end, |offset| root_index + offset + 1);
            let events = session.log[root_index..page_end].to_vec();
            let reached_start = roots.len() == 1 && session.reached_start;
            let next_before_seq = (!reached_start).then_some(session.log[root_index].seq);
            (events, next_before_seq, reached_start)
        })
    }

    #[must_use]
    pub fn question_page_summaries(
        &self,
        session_id: &str,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Option<(Vec<QuestionPageSummary>, Option<u64>, usize, bool)> {
        let sessions = self.inner.sessions.lock();
        sessions.get(session_id).map(|session| {
            let roots = session
                .log
                .iter()
                .enumerate()
                .filter_map(|(index, envelope)| {
                    let previous_was_user =
                        index > 0 && is_user_message_chunk(&session.log[index - 1]);
                    (is_human_question_chunk(envelope) && !previous_was_user).then_some(envelope)
                })
                .collect::<Vec<_>>();
            let end = before_seq.map_or(roots.len(), |cursor| {
                roots.partition_point(|envelope| envelope.seq < cursor)
            });
            let start = end.saturating_sub(limit);
            let pages = roots[start..end]
                .iter()
                .enumerate()
                .map(|(offset, envelope)| {
                    let ordinal = u64::try_from(start + offset + 1).unwrap_or(u64::MAX);
                    QuestionPageSummary {
                        id: envelope.seq,
                        title: question_summary_title(question_chunk_text(envelope), ordinal),
                        ordinal,
                    }
                })
                .collect();
            let next_before_seq = (start > 0).then_some(roots[start].seq);
            (pages, next_before_seq, roots.len(), session.reached_start)
        })
    }

    #[must_use]
    pub fn question_page_at(&self, session_id: &str, root_seq: u64) -> Option<Vec<Envelope>> {
        let sessions = self.inner.sessions.lock();
        sessions.get(session_id).and_then(|session| {
            let root_index = session
                .log
                .iter()
                .position(|envelope| envelope.seq == root_seq)?;
            let envelope = &session.log[root_index];
            let previous_was_user =
                root_index > 0 && is_user_message_chunk(&session.log[root_index - 1]);
            if !is_human_question_chunk(envelope) || previous_was_user {
                return None;
            }
            let next_root = (root_index + 1..session.log.len())
                .find(|&index| {
                    is_human_question_chunk(&session.log[index])
                        && !is_user_message_chunk(&session.log[index - 1])
                })
                .unwrap_or(session.log.len());
            let end = session.log[root_index..next_root]
                .iter()
                .position(is_turn_end)
                .map_or(next_root, |offset| root_index + offset + 1);
            Some(session.log[root_index..end].to_vec())
        })
    }

    /// Register a new session in `Starting` state and broadcast the new list.
    #[cfg(test)]
    pub fn create_local_session(
        &self,
        id: String,
        provider: String,
        cwd: String,
        title: String,
        origin: SessionOrigin,
        system: bool,
    ) {
        self.create_session(SessionRegistration {
            id,
            provider,
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "local".to_owned(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd,
            title,
            origin,
            system,
            owner_user_id: None,
            owner_username: None,
        });
    }

    /// Register a session on a specific stable machine identity.
    pub fn create_session(&self, registration: SessionRegistration) {
        let SessionRegistration {
            id,
            provider,
            provider_version,
            provider_generation_digest,
            provider_auth_generation,
            provider_behavior,
            machine_id,
            workspace_id,
            workspace_name,
            workspace_source_path,
            execution_binding,
            cwd,
            title,
            origin,
            system,
            owner_user_id,
            owner_username,
        } = registration;
        let config_preferences = default_config_preferences(&provider, provider_behavior.as_ref());
        let meta = SessionMeta {
            id: id.clone(),
            provider,
            provider_version,
            provider_generation_digest,
            provider_auth_generation,
            provider_behavior,
            machine_id,
            workspace_id,
            workspace_name,
            workspace_source_path,
            execution_binding,
            cwd,
            title,
            status: Status::Starting,
            origin,
            agent_session_id: None,
            paused: false,
            closing: false,
            system,
            context_used: 0,
            context_size: 0,
            usage: None,
            background_tasks: 0,
            provider_update: None,
            provider_update_available: None,
            next_schedule_ms: None,
            owner_user_id,
            owner_username,
        };
        {
            let mut sessions = self.inner.sessions.lock();
            let mut order = self.inner.order.lock();
            sessions.insert(
                id.clone(),
                Session {
                    meta: meta.clone(),
                    code_incarnation: code_scope::CodeIncarnation::default(),
                    machine_lineage: None,
                    log: Vec::new(),
                    log_bytes: 0,
                    event_count: 0,
                    reached_start: true,
                    next_seq: 0,
                    last_activity: std::time::Instant::now(),
                    config_options: None,
                    config_preferences: config_preferences.clone(),
                    queue: Vec::new(),
                    drafts: Vec::new(),
                    editing: None,
                    editing_epoch: 0,
                    in_flight: false,
                    in_flight_epoch: 0,
                    dispatched_cmids: VecDeque::new(),
                    turn_completion_latched: false,
                    lifecycle_epoch: 0,
                    mobile_review: MobileReviewState::default(),
                },
            );
            order.push(id.clone());
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::InsertSession(Box::new(meta)));
            if config_preferences
                .as_object()
                .is_some_and(|preferences| !preferences.is_empty())
            {
                let _ = tx.send(StoreWrite::UpdateConfigPreferences {
                    session_id: id,
                    preferences: config_preferences,
                });
            }
        }
        self.broadcast_sessions();
    }

    /// Remove a session entirely. Drops its event log and broadcasts the
    /// updated session list. Returns `true` if a session was actually
    /// removed. Note: this does NOT touch the supervisor — callers must
    /// also call [`crate::supervisor::Supervisor::delete_session`] (or the
    /// agent thread will linger uselessly until its rx is dropped on
    /// process shutdown).
    pub fn delete_session(&self, session_id: &str) -> bool {
        self.remove_session(session_id, true)
    }

    /// Remove a session after a caller-owned durable transaction has already
    /// recorded its deletion and absolute purge deadline.
    pub fn detach_session(&self, session_id: &str) -> bool {
        self.remove_session(session_id, false)
    }

    fn remove_session(&self, session_id: &str, persist: bool) -> bool {
        let removed = {
            let mut sessions = self.inner.sessions.lock();
            let mut order = self.inner.order.lock();
            let removed = sessions.remove(session_id).is_some();
            order.retain(|id| id != session_id);
            self.inner.folders.lock().forget_session(session_id);
            if removed {
                self.inner.history_reducer.lock().clear_session(session_id);
            }
            removed
        };
        if removed {
            // The per-session sync states die with the session. Their dedupe
            // sets must not keep growing for ids no client can replay any more.
            {
                let mut sync = self.inner.sync.lock();
                sync.remove(&format!("queue:{session_id}"));
                sync.remove(&format!("mobile-review:{session_id}"));
            }
            if persist && let Some(tx) = self.inner.store_tx.as_ref() {
                let _ = tx.send(StoreWrite::DeleteSession(session_id.to_owned()));
            }
            self.broadcast_sessions();
        }
        removed
    }

    /// Rename a session. Updates the in-memory `title`, persists, and
    /// re-broadcasts the session list so every connected surface sees the
    /// new label. Unknown ids are silently ignored (matches `set_status`).
    pub fn rename_session(&self, session_id: &str, title: String) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.meta.title.clone_from(&title);
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateTitle {
                session_id: session_id.to_owned(),
                title,
            });
        }
        self.broadcast_sessions();
    }

    /// Run `f` while holding the internal settings mutex. Callers must not await.
    pub fn with_settings_mut<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, serde_json::Value>) -> R,
    ) -> R {
        let mut settings = self.inner.settings.lock();
        product_permissions::mutate(self, &mut settings, f)
    }

    /// Snapshot internal settings for authenticated admin reads.
    #[must_use]
    pub fn settings_snapshot(&self) -> HashMap<String, serde_json::Value> {
        self.inner.settings.lock().clone()
    }

    /// One internal setting; JSON null means absent.
    pub fn setting(&self, key: &str) -> Option<serde_json::Value> {
        self.inner
            .settings
            .lock()
            .get(key)
            .filter(|value| !value.is_null())
            .cloned()
    }

    /// Restore internal auth/admin state before the HTTP server starts.
    pub fn load_settings(&self, entries: Vec<(String, serde_json::Value)>) {
        self.with_settings_mut(|settings| settings.extend(entries));
    }

    /// Insert one setting while the caller holds the settings mutex.
    pub fn commit_setting_locked(
        settings: &mut HashMap<String, serde_json::Value>,
        key: String,
        value: serde_json::Value,
    ) -> HashMap<String, serde_json::Value> {
        settings.insert(key, value);
        settings.clone()
    }

    /// Persist an internal setting after the settings lock has been dropped.
    pub fn publish_setting(
        &self,
        key: String,
        value: serde_json::Value,
        _snapshot: HashMap<String, serde_json::Value>,
    ) {
        // The store rejects unregistered keys deterministically; enqueueing
        // one would only exhaust retries and mark persistence degraded.
        let persisted = settings_keys::is_persisted_setting_key(&key);
        debug_assert!(
            persisted,
            "Hub setting {key:?} is not registered in core::settings_keys"
        );
        if !persisted {
            tracing::error!(key, "refusing to persist an unregistered Hub setting");
        } else if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::PutSetting { key, value });
        }
        // Settings is a compatibility tombstone. Never expose internal auth or
        // admin state to product clients.
        self.fanout(Outbound::Settings {
            settings: HashMap::new(),
        });
    }

    /// Persist one internal setting and publish the empty compatibility snapshot.
    pub fn set_setting(&self, key: String, value: serde_json::Value) {
        let snapshot = self.with_settings_mut(|settings| {
            Self::commit_setting_locked(settings, key.clone(), value.clone())
        });
        self.publish_setting(key, value, snapshot);
    }

    /// Publish whether an accepted deletion is still waiting for its execution
    /// environment. Broadcast-only; deletion itself is [`Self::delete_session`].
    pub fn set_closing(&self, session_id: &str, closing: bool) {
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            match sessions.get_mut(session_id) {
                Some(s) if s.meta.closing != closing => {
                    s.meta.closing = closing;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.broadcast_sessions();
        }
    }

    /// Manually PAUSE / RESUME the queue drain (the user's ⏸ toggle). Pausing
    /// holds the auto-drain (`drain_head` returns early on `paused`) WITHOUT
    /// touching the running turn — it finishes normally; only the next queued
    /// message is held. Resuming kicks the drain (which still waits for any
    /// in-flight turn to end, then advances). In-memory only (not persisted) +
    /// broadcast so every terminal reflects the state. No-op when unchanged.
    pub fn set_paused(&self, session_id: &str, paused: bool) {
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            match sessions.get_mut(session_id) {
                Some(s) if s.meta.paused != paused => {
                    s.meta.paused = paused;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.broadcast_sessions();
            // Resuming → try to advance now (an idle session with a queue drains
            // immediately; a busy one drains on the next turn-end as usual).
            if !paused {
                self.try_drain(session_id);
            }
        }
    }

    /// Record the agent-reported context-window usage (ACP `usage_update`):
    /// `used` tokens of a `size`-token window. Broadcast-only (transient, not
    /// persisted). Deduped — the agent re-emits identical usage several times per
    /// turn, so we only broadcast when the numbers actually move, keeping the
    /// session-list churn (and mobile bandwidth) down.
    pub fn set_session_usage(&self, session_id: &str, usage: crate::agent_model::SessionUsage) {
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            match sessions.get_mut(session_id) {
                Some(s) if s.meta.usage.as_ref() != Some(&usage) => {
                    s.meta.context_used = usage.used;
                    s.meta.context_size = usage.size;
                    s.meta.usage = Some(usage);
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.broadcast_sessions();
        }
    }

    /// Record the worker's live background-task level. Broadcast-only and
    /// deduped; it never changes dispatch or the transcript.
    pub fn set_background_tasks(&self, session_id: &str, count: u32) {
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            match sessions.get_mut(session_id) {
                Some(s) if s.meta.background_tasks != count => {
                    s.meta.background_tasks = count;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.broadcast_sessions();
        }
    }

    // --- Generic optimistic-sync channel (Cowboy state-sync arbiter) --------
    // The daemon is the arbiter for each synced `state`. A mutation is applied to
    // the TYPED source of truth (SessionMeta / order list); the patch carries the
    // state's DERIVED json value. No bespoke per-state wire — one Sync/SyncPatch.

    /// Apply a rename to the typed truth (NO `Sessions` re-broadcast — the sync
    /// channel carries the title now). Persists so fresh-connect + restart show
    /// it. Mirror of [`Self::rename_session`] minus the broadcast.
    fn apply_rename(&self, session_id: &str, title: String) {
        {
            let mut sessions = self.inner.sessions.lock();
            if let Some(s) = sessions.get_mut(session_id) {
                s.meta.title.clone_from(&title);
            }
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateTitle {
                session_id: session_id.to_owned(),
                title,
            });
        }
    }

    /// Apply a session reorder to the order list (NO broadcast — the sync channel
    /// carries it). Mirror of [`Self::reorder_sessions`] minus the broadcast.
    /// Submitted ids only permute names they include; every existing id survives.
    fn apply_reorder(&self, order: &[String]) {
        {
            let mut list = self.inner.order.lock();
            *list = merge_session_order(&list, order);
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let order = self.inner.order.lock().clone();
            let _ = tx.send(StoreWrite::UpdateSessionOrder { order });
        }
    }

    /// Apply a sidebar-folders mutation to the typed tree and persist what it
    /// changed: the owner's whole folder set and any explicit placements.
    /// Mirror of [`Self::apply_reorder`]; the sync channel carries the value.
    fn apply_folders(
        &self,
        actor: &crate::session_folders::FolderActor,
        mutation: &str,
        args: &serde_json::Value,
    ) -> Result<(), String> {
        let (effects, folders) = {
            let mut state = self.inner.folders.lock();
            let effects = state.apply(actor, mutation, args)?;
            let folders = effects
                .replaced_owner
                .as_ref()
                .map(|owner| state.folders_of(owner.as_deref()));
            (effects, folders)
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            if let (Some(owner_user_id), Some(folders)) = (effects.replaced_owner, folders) {
                let _ = tx.send(StoreWrite::ReplaceSessionFolders {
                    owner_user_id,
                    folders,
                });
            }
            if !effects.placements.is_empty() {
                let _ = tx.send(StoreWrite::UpdateSessionPlacement {
                    placements: effects.placements,
                });
            }
        }
        Ok(())
    }

    fn apply_mobile_review(
        &self,
        session_id: &str,
        mutation: &str,
        args: &serde_json::Value,
    ) -> Result<(), String> {
        let persisted = {
            let mut sessions = self.inner.sessions.lock();
            let session = sessions
                .get_mut(session_id)
                .ok_or_else(|| "unknown mobile review session".to_owned())?;
            let state = &mut session.mobile_review;
            match mutation {
                "setRemoteReview" => {
                    let binding = match args.get("binding") {
                        Some(serde_json::Value::Null) => None,
                        Some(value) => {
                            let binding: crate::workspace_extensions::ReviewBinding =
                                serde_json::from_value(value.clone())
                                    .map_err(|_| "invalid PR association")?;
                            if !binding.valid() {
                                return Err("invalid PR association".to_owned());
                            }
                            Some(binding)
                        }
                        None => return Err("missing PR association".to_owned()),
                    };
                    state.remote_review = binding;
                }
                "selectRemoteReview" => {
                    state.remote_selected = args
                        .get("selected")
                        .and_then(serde_json::Value::as_bool)
                        .ok_or("invalid PR selection")?;
                }
                "open" => {
                    let path = mobile_review_string_arg(args, "path", 4096)?;
                    if !valid_mobile_review_path(&path) {
                        return Err("invalid mobile review path".to_owned());
                    }
                    if !state.tabs.iter().any(|tab| tab.path == path) {
                        if state.tabs.len() >= MOBILE_REVIEW_TAB_CAP {
                            let evict = state.tabs.iter().position(|tab| !tab.pinned).unwrap_or(0);
                            let removed = state.tabs.remove(evict);
                            if state.active.as_deref() == Some(&removed.path) {
                                state.active = None;
                            }
                        }
                        state.tabs.push(MobileReviewTab {
                            path: path.clone(),
                            pinned: false,
                        });
                    }
                    state.active = Some(path);
                    state.mode = "files".to_owned();
                }
                "close" => {
                    let path = mobile_review_string_arg(args, "path", 4096)?;
                    state.tabs.retain(|tab| tab.path != path);
                    if state.active.as_deref() == Some(&path) {
                        state.active = state.tabs.last().map(|tab| tab.path.clone());
                    }
                }
                "reorder" => {
                    let order = args
                        .get("paths")
                        .and_then(serde_json::Value::as_array)
                        .ok_or("reorder: missing paths")?
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .filter(|path| valid_mobile_review_path(path))
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    sort_by_id_order(&mut state.tabs, &order, |tab| &tab.path);
                }
                "setPinned" => {
                    let path = mobile_review_string_arg(args, "path", 4096)?;
                    let pinned = args
                        .get("pinned")
                        .and_then(serde_json::Value::as_bool)
                        .ok_or("setPinned: missing pinned")?;
                    if let Some(tab) = state.tabs.iter_mut().find(|tab| tab.path == path) {
                        tab.pinned = pinned;
                    }
                }
                "activate" => {
                    let path = args.get("path").and_then(serde_json::Value::as_str);
                    state.active = path
                        .filter(|path| state.tabs.iter().any(|tab| tab.path == *path))
                        .map(str::to_owned);
                }
                "setMode" => {
                    let mode = mobile_review_string_arg(args, "mode", 16)?;
                    if !matches!(mode.as_str(), "files" | "git") {
                        return Err("invalid mobile review mode".to_owned());
                    }
                    state.mode = mode;
                }
                "markReviewed" => {
                    let key = mobile_review_string_arg(args, "key", 2048)?;
                    match args.get("revision").and_then(serde_json::Value::as_str) {
                        Some(revision) if !revision.is_empty() && revision.len() <= 512 => {
                            if state.progress.len() >= MOBILE_REVIEW_PROGRESS_CAP
                                && !state.progress.contains_key(&key)
                                && let Some(oldest) = state.progress.keys().next().cloned()
                            {
                                state.progress.remove(&oldest);
                            }
                            state.progress.insert(key, revision.to_owned());
                        }
                        None => {
                            state.progress.remove(&key);
                        }
                        _ => return Err("invalid review revision".to_owned()),
                    }
                }
                "setPosition" => {
                    let path = mobile_review_string_arg(args, "path", 4096)?;
                    if !valid_mobile_review_path(&path) {
                        return Err("invalid mobile review path".to_owned());
                    }
                    let line = args
                        .get("line")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|line| u32::try_from(line).ok())
                        .filter(|line| *line > 0)
                        .ok_or("setPosition: invalid line")?;
                    let revision = match args.get("revision") {
                        None | Some(serde_json::Value::Null) => None,
                        Some(value) => Some(
                            value
                                .as_str()
                                .filter(|revision| !revision.is_empty() && revision.len() <= 512)
                                .map(str::to_owned)
                                .ok_or("setPosition: invalid revision")?,
                        ),
                    };
                    if state.positions.len() >= MOBILE_REVIEW_POSITION_CAP
                        && !state.positions.contains_key(&path)
                        && let Some(oldest) = state.positions.keys().next().cloned()
                    {
                        state.positions.remove(&oldest);
                    }
                    state
                        .positions
                        .insert(path, MobileReviewPosition { line, revision });
                }
                _ => return Err(format!("unknown mobile review mutation {mutation}")),
            }
            serde_json::to_value(state).map_err(|error| error.to_string())?
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateMobileReviewState {
                session_id: session_id.to_owned(),
                value: persisted,
            });
        }
        Ok(())
    }

    /// The derived JSON value of one synced state — what a `SyncPatch` carries and
    /// the client folds. Always read live from the typed truth (so it's durable by
    /// derivation, no shadow copy to drift).
    #[must_use]
    pub fn sync_value(&self, state: &str) -> serde_json::Value {
        match state {
            "title" => {
                let sessions = self.inner.sessions.lock();
                let map: serde_json::Map<String, serde_json::Value> = sessions
                    .values()
                    .map(|s| {
                        (
                            s.meta.id.clone(),
                            serde_json::Value::String(s.meta.title.clone()),
                        )
                    })
                    .collect();
                serde_json::Value::Object(map)
            }
            "order" => {
                let list = self.inner.order.lock();
                serde_json::Value::Array(
                    list.iter()
                        .map(|id| serde_json::Value::String(id.clone()))
                        .collect(),
                )
            }
            "folders" => self.inner.folders.lock().value(),
            "workspace-order" => serde_json::to_value(&*self.inner.workspace_orders.lock())
                .expect("workspace orders serialize"),
            "drafts" => serde_json::to_value(&*self.inner.draft_announcements.lock())
                .expect("draft announcements serialize"),
            _ if state.starts_with("mobile-review:") => {
                let session_id = &state["mobile-review:".len()..];
                let sessions = self.inner.sessions.lock();
                sessions
                    .get(session_id)
                    .and_then(|session| serde_json::to_value(&session.mobile_review).ok())
                    .unwrap_or(serde_json::Value::Null)
            }
            _ => serde_json::Value::Null,
        }
    }

    /// Whether `id` was already applied for `state` (a retried delivery), without
    /// consuming it.
    fn sync_already_seen(&self, state: &str, id: &str) -> bool {
        self.inner
            .sync
            .lock()
            .get(state)
            .is_some_and(|entry| entry.seen.contains(id))
    }

    /// Record `id` as seen for `state`; returns true if it's NEW (first delivery).
    fn sync_first_seen(&self, state: &str, id: &str) -> bool {
        let mut reg = self.inner.sync.lock();
        reg.entry(state.to_owned())
            .or_default()
            .seen
            .insert(id.to_owned())
    }

    /// Cmids carried inside a queue/drafts value — the confirm set for the queue
    /// sync state (the client drops an optimistic add the moment its cmid lands).
    fn cmids_of(queue: &[QueuedMessage], drafts: &[QueuedMessage]) -> Vec<String> {
        queue
            .iter()
            .chain(drafts.iter())
            .filter_map(|m| m.cmid.clone())
            .collect()
    }

    /// Bump `state`'s version and broadcast a LIVE (version-gated) SyncPatch with
    /// the given absolute value + confirm set.
    fn sync_emit(&self, state: &str, value: serde_json::Value, confirmed: Vec<String>) {
        let version = {
            let mut reg = self.inner.sync.lock();
            let e = reg.entry(state.to_owned()).or_default();
            e.version += 1;
            e.version
        };
        // Op-log: every AUTHORITATIVE state change, one line → journald → vector
        // → VictoriaLogs. Lets you (or an AI) replay "how state X reached version
        // N" via LogsQL. Low volume (user-paced changes, not per-agent-event).
        tracing::info!(target: "cowboy::oplog", op = "change", %state, version, confirmed = ?confirmed);
        self.fanout(Outbound::SyncPatch {
            state: state.to_owned(),
            version,
            value,
            confirmed,
            resync: false,
        });
    }

    /// Version-stamp `state` and broadcast its derived value, confirming the given
    /// mutation ids.
    fn sync_broadcast(&self, state: &str, confirmed: Vec<String>) {
        let value = self.sync_value(state);
        self.sync_emit(state, value, confirmed);
    }

    /// Generic arbiter apply — see [`Inbound::Sync`]. Validate+parse (rejects a
    /// bad/unknown mutation before burning the id), dedupe (retry = no-op), apply
    /// the typed mutation, then version-stamp + broadcast. Returns an error string
    /// the server surfaces to the user.
    pub fn sync_apply(
        &self,
        state: &str,
        id: String,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(), String> {
        self.sync_apply_as(
            &crate::session_folders::FolderActor::local(),
            state,
            id,
            name,
            args,
        )
    }

    /// [`Self::sync_apply`] on behalf of a product principal: `actor` scopes
    /// the `"folders"` state to the principal's own tree.
    pub fn sync_apply_as(
        &self,
        actor: &crate::session_folders::FolderActor,
        state: &str,
        id: String,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(), String> {
        enum Op {
            WorkspaceOrder {
                owner: String,
                order: Vec<String>,
            },
            Rename {
                session_id: String,
                title: String,
            },
            Reorder {
                order: Vec<String>,
            },
            Folders {
                mutation: String,
                args: serde_json::Value,
            },
            MobileReview {
                session_id: String,
                mutation: String,
                args: serde_json::Value,
            },
        }
        let op = match (state, name) {
            ("workspace-order", "reorder") => {
                let values = args
                    .get("order")
                    .and_then(serde_json::Value::as_array)
                    .ok_or("workspace order is missing")?;
                if values.len() > 12000 {
                    return Err("workspace order is too large".to_owned());
                }
                let mut order = Vec::new();
                for value in values {
                    let key = value.as_str().ok_or("invalid workspace item")?;
                    let valid = key
                        .strip_prefix("draft:")
                        .or_else(|| key.strip_prefix("session:"));
                    if valid.is_none_or(|id| {
                        id.is_empty()
                            || id.len() > 160
                            || !id
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                    }) {
                        return Err("invalid workspace item".to_owned());
                    }
                    if !order.iter().any(|existing| existing == key) {
                        order.push(key.to_owned());
                    }
                }
                Op::WorkspaceOrder {
                    owner: actor.user_id.clone().unwrap_or_default(),
                    order,
                }
            }
            ("title", "rename") => {
                let session_id = args
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("rename: missing session_id")?
                    .to_owned();
                let title = args
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                if title.is_empty() {
                    return Err("title cannot be empty".to_owned());
                }
                Op::Rename { session_id, title }
            }
            ("order", "reorder") => {
                let order = args
                    .get("order")
                    .and_then(serde_json::Value::as_array)
                    .ok_or("reorder: missing order")?
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
                Op::Reorder { order }
            }
            ("folders", name) => {
                // A retried delivery must not be re-validated against the state
                // it already changed ("folder id already exists"). Confirm it
                // again: the resending client may have missed the first patch.
                if self.sync_already_seen(state, &id) {
                    self.sync_broadcast(state, vec![id]);
                    return Ok(());
                }
                // The dedupe set does not survive a Controller restart, but a
                // create that already produced exactly this folder is the same
                // retried delivery. Confirm it instead of rejecting the replay.
                if name == "create" && self.inner.folders.lock().is_replayed_create(actor, args) {
                    self.sync_first_seen(state, &id);
                    self.sync_broadcast(state, vec![id]);
                    return Ok(());
                }
                // Dry-run against a copy so a rejected mutation never consumes
                // its id: the client keeps the error, not a silent no-op retry.
                self.inner.folders.lock().clone().apply(actor, name, args)?;
                Op::Folders {
                    mutation: name.to_owned(),
                    args: args.clone(),
                }
            }
            (state, name) if state.starts_with("mobile-review:") => {
                let session_id = state["mobile-review:".len()..].to_owned();
                if session_id.is_empty() || !self.inner.sessions.lock().contains_key(&session_id) {
                    return Err("unknown mobile review session".to_owned());
                }
                if !matches!(
                    name,
                    "open"
                        | "setRemoteReview"
                        | "selectRemoteReview"
                        | "close"
                        | "reorder"
                        | "setPinned"
                        | "activate"
                        | "setMode"
                        | "markReviewed"
                        | "setPosition"
                ) {
                    return Err(format!("unknown mobile review mutation {name}"));
                }
                Op::MobileReview {
                    session_id,
                    mutation: name.to_owned(),
                    args: args.clone(),
                }
            }
            _ => return Err(format!("unknown sync mutation {state}/{name}")),
        };
        if !self.sync_first_seen(state, &id) {
            // Duplicate delivery/retry: already applied. Confirm it again so a
            // client that missed the original patch retires its outbox entry.
            self.sync_broadcast(state, vec![id]);
            return Ok(());
        }
        match op {
            Op::WorkspaceOrder { owner, order } => {
                let mut orders = self.inner.workspace_orders.lock();
                let current = orders.entry(owner.clone()).or_default();
                // Honor newly introduced rows at their submitted positions.
                // Hidden rows omitted by a folded client retain their relative order.
                let named: HashSet<&str> = order.iter().map(String::as_str).collect();
                let retained = current
                    .iter()
                    .filter(|key| !named.contains(key.as_str()))
                    .cloned();
                *current = order.iter().cloned().chain(retained).take(12_000).collect();
                if let Some(tx) = self.inner.store_tx.as_ref() {
                    let _ = tx.send(StoreWrite::UpdateWorkspaceOrder {
                        owner,
                        order: current.clone(),
                    });
                }
            }
            Op::Rename { session_id, title } => self.apply_rename(&session_id, title),
            Op::Reorder { order } => self.apply_reorder(&order),
            Op::Folders { mutation, args } => self.apply_folders(actor, &mutation, &args)?,
            Op::MobileReview {
                session_id,
                mutation,
                args,
            } => self.apply_mobile_review(&session_id, &mutation, &args)?,
        }
        // Op-log: the client INTENT behind a state change (who/what), paired with
        // the `op=change` line sync_emit writes for the authoritative version bump.
        tracing::info!(target: "cowboy::oplog", op = "mutation", %state, name, args = %args);
        self.sync_broadcast(state, vec![id]);
        Ok(())
    }

    /// Resync `SyncPatch`es for the GLOBAL states on connect, `resync: true` so the
    /// client adopts them as ground truth. `title` and `order` are ALWAYS emitted
    /// (even if never mutated this lifetime), because a client may carry a locally
    /// persisted (`state/sync-idb`) cache of these states across reloads:
    /// without an unconditional authoritative seed, a stale cached override would
    /// overlay the fresh `Sessions` titles with nothing to correct it. Any other
    /// state mutated this lifetime is included too. Per-session queue states resync
    /// separately via [`Self::queue_resync`].
    #[must_use]
    pub fn sync_resync(&self) -> Vec<Outbound> {
        let snapshot: Vec<(String, u64, Vec<String>)> = {
            let reg = self.inner.sync.lock();
            let mut out: Vec<(String, u64, Vec<String>)> = reg
                .iter()
                .filter(|(s, _)| !s.starts_with("queue:") && !s.starts_with("mobile-review:"))
                .map(|(s, e)| (s.clone(), e.version, e.seen.iter().cloned().collect()))
                .collect();
            // Guarantee title + order + folders are present even when untouched
            // this lifetime.
            for state in ["title", "order", "folders", "workspace-order"] {
                if !out.iter().any(|(s, _, _)| s == state) {
                    let version = reg.get(state).map_or(0, |e| e.version);
                    out.push((state.to_owned(), version, Vec::new()));
                }
            }
            for session_id in self.inner.sessions.lock().keys() {
                let state = format!("mobile-review:{session_id}");
                if !out.iter().any(|(existing, _, _)| existing == &state) {
                    let version = reg.get(&state).map_or(0, |entry| entry.version);
                    out.push((state, version, Vec::new()));
                }
            }
            out
        };
        snapshot
            .into_iter()
            .map(|(state, version, confirmed)| Outbound::SyncPatch {
                value: self.sync_value(&state),
                state,
                version,
                confirmed,
                resync: true,
            })
            .collect()
    }

    /// A resync `SyncPatch` for one session's queue+drafts state — `resync: true`
    /// so a (re)connecting client adopts it as ground truth. Confirmed = the cmids
    /// present in the value, so any optimistic add that landed while the client was
    /// away is dropped from its pending. `None` for an unknown session.
    #[must_use]
    pub fn queue_resync(&self, session_id: &str) -> Option<Outbound> {
        let (queue, drafts) = {
            let sessions = self.inner.sessions.lock();
            let s = sessions.get(session_id)?;
            (s.queue.clone(), s.drafts.clone())
        };
        let state = format!("queue:{session_id}");
        let version = self.inner.sync.lock().get(&state).map_or(0, |e| e.version);
        let confirmed = Self::cmids_of(&queue, &drafts);
        let value = serde_json::json!({ "queue": queue, "drafts": drafts });
        Some(Outbound::SyncPatch {
            state,
            version,
            value,
            confirmed,
            resync: true,
        })
    }

    /// Auto-name a session from its first prompt, but ONLY while the title is
    /// still the creation-time default (`provider · cwd`). The agent never
    /// pushes a title over ACP, so cowboy derives one itself; gating on the
    /// default makes this fire once (a later prompt sees a non-default title)
    /// and never clobber a manual rename. Check + write under one lock so a
    /// concurrent rename can't race it.
    pub fn auto_title(&self, session_id: &str, title: String) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let default = format!("{} · {}", s.meta.provider, s.meta.cwd);
            if s.meta.title != default {
                return; // manually renamed or already auto-titled
            }
            s.meta.title.clone_from(&title);
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateTitle {
                session_id: session_id.to_owned(),
                title,
            });
        }
        self.broadcast_sessions();
    }

    /// Record the downstream agent's own session id for a session. Codex creates
    /// the id before it creates the rollout, so keep it in memory until the
    /// current context receives its first user turn; only then is it safe to
    /// persist for a future `session/load`. Unknown ids are ignored.
    pub fn set_agent_session_id(&self, session_id: &str, agent_session_id: String) {
        let persist = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.meta.agent_session_id = Some(agent_session_id.clone());
            current_context_has_user_message(s)
        };
        if persist && let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::SetAgentSessionId {
                session_id: session_id.to_owned(),
                agent_session_id: Some(agent_session_id),
            });
        }
    }

    /// Return the native id only when Codex has had a user turn in the current
    /// context generation. An id allocated by `session/new` alone has no rollout
    /// and must not be handed to `session/load` after a restart.
    #[must_use]
    pub fn agent_session_id_for_resume(&self, session_id: &str) -> Option<String> {
        let sessions = self.inner.sessions.lock();
        let session = sessions.get(session_id)?;
        current_context_has_user_message(session)
            .then(|| session.meta.agent_session_id.clone())
            .flatten()
    }

    /// Reserve an idle session with an unresolved authentication failure for
    /// recovery after a newer compatible Service-auth generation reaches its
    /// Machine. The session deliberately retains its original auth generation:
    /// that generation owns the Provider runtime home containing the native
    /// session database and rollout. Machine credential reconciliation updates
    /// only the declared credential files in that home.
    pub fn begin_provider_auth_recovery(
        &self,
        session_id: &str,
        expected_status_revision: (Status, u64),
        expected_crash_detail: &str,
        expected_generation: u64,
        next_generation: u64,
    ) -> Result<bool, String> {
        if next_generation <= expected_generation {
            return Err("Provider auth generation must advance".to_owned());
        }
        let reserved = {
            let mut sessions = self.inner.sessions.lock();
            let session = sessions
                .get_mut(session_id)
                .ok_or_else(|| format!("unknown session {session_id:?}"))?;
            if (session.meta.status, session.lifecycle_epoch) != expected_status_revision
                || unresolved_crash_detail_for_session(session) != Some(expected_crash_detail)
                || session.meta.provider_auth_generation != Some(expected_generation)
                || session.in_flight
                || !matches!(session.meta.status, Status::Crashed | Status::Running)
            {
                false
            } else {
                session.meta.status = Status::Starting;
                session.lifecycle_epoch = session.lifecycle_epoch.wrapping_add(1);
                true
            }
        };
        if !reserved {
            return Ok(false);
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateStatus {
                session_id: session_id.to_owned(),
                status: Status::Starting,
            });
        }
        self.push(
            session_id,
            Event::Lifecycle {
                status: Status::Starting,
                detail: Some("reloading synchronized Provider credentials".to_owned()),
            },
        );
        self.broadcast_sessions();
        Ok(true)
    }

    /// Move a dormant (exited, workerless) session's binding to another
    /// installed Provider generation without starting anything. The next open
    /// launches that generation with the same native resume as an explicit
    /// reload. Refuses unless the session is still exactly `expected` and
    /// still exited, so a racing open or prompt always wins.
    pub fn repin_dormant_provider(
        &self,
        expected: &SessionMeta,
        version: &str,
        digest: &str,
        behavior: &cowboy_provider_sdk::ProviderBehaviorContract,
    ) -> Result<(), String> {
        let meta = {
            let mut sessions = self.inner.sessions.lock();
            let session = sessions
                .get_mut(&expected.id)
                .ok_or_else(|| "session no longer exists".to_owned())?;
            if session.in_flight || session.meta.status != Status::Exited {
                return Err("session is no longer dormant".to_owned());
            }
            if session.meta.provider != expected.provider
                || session.meta.provider_version != expected.provider_version
                || session.meta.provider_generation_digest != expected.provider_generation_digest
                || session.meta.provider_auth_generation != expected.provider_auth_generation
                || session.meta.agent_session_id != expected.agent_session_id
                || session.meta.machine_id != expected.machine_id
                || session.meta.cwd != expected.cwd
                || session.meta.execution_binding != expected.execution_binding
            {
                return Err("session changed while preparing to re-pin; try again".to_owned());
            }
            if !current_context_has_user_message(session) || session.meta.agent_session_id.is_none()
            {
                return Err(
                    "a saved native session is required to change Provider version".to_owned(),
                );
            }
            if session.meta.execution_binding.is_some() {
                let mut candidate = session.meta.clone();
                candidate.provider_version = version.to_owned();
                candidate.provider_generation_digest = digest.to_owned();
                candidate.provider_behavior = Some(behavior.clone());
                candidate.require_runtime_launch()?;
            }
            session.meta.provider_version = version.to_owned();
            session.meta.provider_generation_digest = digest.to_owned();
            session.meta.provider_behavior = Some(behavior.clone());
            session.lifecycle_epoch = session.lifecycle_epoch.wrapping_add(1);
            session.meta.clone()
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::ReloadProvider(Box::new(meta)));
        }
        self.broadcast_sessions();
        Ok(())
    }

    /// Reserve an idle session for an explicit Provider reload. The lock also
    /// fences prompt submission: a racing prompt either wins and rejects the
    /// reload, or stays queued until the replacement runtime is ready.
    pub fn begin_provider_reload(
        &self,
        expected: &SessionMeta,
        version: &str,
        digest: &str,
        behavior: &cowboy_provider_sdk::ProviderBehaviorContract,
        automatic: bool,
    ) -> Result<(), String> {
        let meta = {
            let mut sessions = self.inner.sessions.lock();
            let session = sessions
                .get_mut(&expected.id)
                .ok_or_else(|| "session no longer exists".to_owned())?;
            if session.in_flight || matches!(session.meta.status, Status::Busy | Status::Starting) {
                return Err(
                    "wait for the current turn to finish before loading a new Provider version"
                        .to_owned(),
                );
            }
            if session.meta.provider != expected.provider
                || session.meta.provider_version != expected.provider_version
                || session.meta.provider_generation_digest != expected.provider_generation_digest
                || session.meta.provider_auth_generation != expected.provider_auth_generation
                || session.meta.agent_session_id != expected.agent_session_id
                || session.meta.machine_id != expected.machine_id
                || session.meta.cwd != expected.cwd
                || session.meta.execution_binding != expected.execution_binding
            {
                return Err("session changed while preparing reload; try again".to_owned());
            }
            if !current_context_has_user_message(session) || session.meta.agent_session_id.is_none()
            {
                return Err(
                    "a saved native session is required to reload a new Provider version"
                        .to_owned(),
                );
            }
            if session.meta.execution_binding.is_some() {
                let mut candidate = session.meta.clone();
                candidate.provider_version = version.to_owned();
                candidate.provider_generation_digest = digest.to_owned();
                candidate.provider_behavior = Some(behavior.clone());
                candidate.require_runtime_launch()?;
            }
            session.meta.provider_update = Some(ProviderUpdate {
                from: session.meta.provider_version.clone(),
                to: version.to_owned(),
                automatic,
                started_at_ms: now_ms(),
            });
            session.meta.provider_version = version.to_owned();
            session.meta.provider_generation_digest = digest.to_owned();
            session.meta.provider_behavior = Some(behavior.clone());
            // Auth generation owns the native runtime home. Never replace it
            // with the active installation's credential generation here.
            session.meta.status = Status::Starting;
            session.lifecycle_epoch = session.lifecycle_epoch.wrapping_add(1);
            session.meta.clone()
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::ReloadProvider(Box::new(meta)));
        }
        self.push(
            &expected.id,
            Event::Lifecycle {
                status: Status::Starting,
                detail: Some(format!(
                    "reloading Provider {} -> {version}; preserving native session",
                    expected.provider_version
                )),
            },
        );
        self.broadcast_sessions();
        Ok(())
    }

    /// Settle an interrupted Provider reload against the surviving native
    /// worker. The reset and write-behind persistence travel independently;
    /// after reconnect the actual owner, not an unacknowledged intent, defines
    /// the running release. Never use this to migrate native identity or home.
    pub fn reconcile_provider_release(&self, worker: &WorkerSnapshot) {
        let Some(launch) = worker.launch.as_ref().filter(|launch| {
            worker.has_connected_owner() && !launch.provider_generation_digest.is_empty()
        }) else {
            return;
        };
        let reconciled = {
            let mut sessions = self.inner.sessions.lock();
            let Some(session) = sessions.get_mut(&worker.session_id) else {
                return;
            };
            if session.meta.provider != launch.provider
                || session.meta.require_runtime_launch().is_err()
                || session.meta.cwd != launch.cwd
                || session.meta.provider_auth_generation != launch.provider_auth_generation
                || session.meta.agent_session_id.is_none()
                || session.meta.agent_session_id != worker.agent_session_id
                || session.meta.provider_generation_digest.is_empty()
                || (session.meta.provider_generation_digest == launch.provider_generation_digest
                    && session.meta.provider_version == launch.provider_version)
            {
                return;
            }
            session
                .meta
                .provider_version
                .clone_from(&launch.provider_version);
            session
                .meta
                .provider_generation_digest
                .clone_from(&launch.provider_generation_digest);
            session
                .meta
                .provider_behavior
                .clone_from(&launch.provider_behavior);
            session.meta.clone()
        };
        tracing::warn!(session = %worker.session_id, version = %reconciled.provider_version,
            digest = %reconciled.provider_generation_digest,
            "reconciled interrupted Provider reload with surviving native worker");
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::ReloadProvider(Box::new(reconciled)));
        }
        self.broadcast_sessions();
    }

    /// Retarget a Cowboy session to a replacement checkout while preserving its
    /// transcript, queue, Cowboy id, and native agent session id. A creation-time
    /// default title follows the cwd; user-authored and auto-derived titles do not.
    pub fn update_session_cwd(&self, session_id: &str, cwd: String) -> Result<(), String> {
        let title = {
            let mut sessions = self.inner.sessions.lock();
            let session = sessions
                .get_mut(session_id)
                .ok_or_else(|| format!("unknown session {session_id:?}"))?;
            if session.meta.cwd == cwd {
                return Ok(());
            }
            if session.meta.execution_binding.is_some() {
                return Err(
                    "bound execution environment requires an atomic workspace change".into(),
                );
            }
            let default_title = format!("{} · {}", session.meta.provider, session.meta.cwd);
            let title = (session.meta.title == default_title)
                .then(|| format!("{} · {cwd}", session.meta.provider));
            session.meta.cwd.clone_from(&cwd);
            session.code_incarnation = code_scope::CodeIncarnation::default();
            if let Some(title) = &title {
                session.meta.title.clone_from(title);
            }
            title
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateCwd {
                session_id: session_id.to_owned(),
                cwd,
                title,
            });
        }
        self.broadcast_sessions();
        Ok(())
    }

    /// Publish a prepared binding only after its compare-and-set has committed
    /// to storage. No intermediate runtime-local metadata is ever observable.
    pub(crate) fn accept_execution_binding(
        &self,
        session_id: &str,
        expected: &crate::execution_environment::ExecutionBinding,
        prepared: crate::execution_environment::ExecutionBinding,
    ) -> Result<(), String> {
        {
            let mut sessions = self.inner.sessions.lock();
            let session = sessions
                .get_mut(session_id)
                .ok_or("session no longer exists")?;
            if session.meta.execution_binding.as_ref() != Some(expected) {
                return Err("execution preparation changed".into());
            }
            let binding = prepared.for_runtime(&session.meta.machine_id, &session.meta.cwd)?;
            if !expected
                .preparation()
                .is_some_and(|intent| intent.accepts(&binding))
            {
                return Err("prepared execution environment does not match the session".into());
            }
            let mut candidate = session.meta.clone();
            candidate.execution_binding = Some(prepared);
            candidate.require_runtime_launch()?;
            session.meta = candidate;
            session.code_incarnation = code_scope::CodeIncarnation::default();
        }
        self.broadcast_sessions();
        Ok(())
    }

    /// Publish a storage-CASed maintenance transition without changing the
    /// native conversation, prompts, Provider, authentication or workspace.
    pub(crate) fn accept_execution_recovery(
        &self,
        session_id: &str,
        expected: &crate::execution_environment::ExecutionBinding,
        next: crate::execution_environment::ExecutionBinding,
    ) -> Result<(), String> {
        {
            let mut sessions = self.inner.sessions.lock();
            let session = sessions
                .get_mut(session_id)
                .ok_or("session no longer exists")?;
            if session.meta.execution_binding.as_ref() != Some(expected) {
                return Err("execution recovery changed".into());
            }
            let valid = match (
                expected.decode(),
                next.recovery(),
                expected.recovery(),
                next.decode(),
            ) {
                (Ok(previous), Some(intent), _, _) => intent.previous == previous,
                (_, _, Some(intent), Ok(binding)) => intent.accepts(&binding),
                _ => false,
            };
            if !valid {
                return Err("invalid execution recovery transition".into());
            }
            session.meta.execution_binding = Some(next);
            session.code_incarnation = code_scope::CodeIncarnation::default();
        }
        self.broadcast_sessions();
        Ok(())
    }

    /// Prepare a session for a fresh-context worker replacement.
    ///
    /// Forget the resumable agent id so the next spawn uses `session/new`, and
    /// release the old worker's in-flight guard. The replacement has its own
    /// lifecycle fence; carrying this guard across the reset would leave the new
    /// idle worker permanently unable to dispatch a queued prompt because the
    /// normal `Starting` -> `Running` edge deliberately does not clear it.
    pub fn prepare_context_reset(&self, session_id: &str) {
        {
            let mut sessions = self.inner.sessions.lock();
            if let Some(s) = sessions.get_mut(session_id) {
                s.meta.agent_session_id = None;
                s.in_flight = false;
            }
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::SetAgentSessionId {
                session_id: session_id.to_owned(),
                agent_session_id: None,
            });
        }
    }

    /// Destructively clear one session's transcript while keeping its sequence
    /// watermark monotonic, so delayed clients cannot collide with old seq ids.
    pub fn clear_transcript(&self, session_id: &str) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(session) = sessions.get_mut(session_id) else {
                return;
            };
            session.log.clear();
            session.log_bytes = 0;
            session.event_count = 0;
            session.reached_start = true;
            self.inner.history_reducer.lock().clear_session(session_id);
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::ClearEvents {
                session_id: session_id.to_owned(),
            });
        }
    }

    /// Drop a `context_cleared` marker into a session's timeline so every client
    /// renders a "conversation cleared" divider. Pushed as a normal ACP-shaped
    /// `update` (the frontend's `derive` maps `sessionUpdate: "context_cleared"`
    /// to a divider item); `at` is unix-ms for the divider's timestamp.
    pub fn mark_context_cleared(&self, session_id: &str) {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        self.push(
            session_id,
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "context_cleared",
                    "at": at,
                }),
            },
        );
    }

    /// Update a session's status, emit a `Lifecycle` event, refresh the list.
    pub fn set_status(&self, session_id: &str, status: Status, detail: Option<String>) {
        let _ = self.set_status_if_revision(session_id, None, status, detail);
    }

    /// Return the status plus the monotonic identity of its current lifecycle
    /// edge. A force-cancel watchdog captures this before sending Cancel.
    #[must_use]
    pub fn status_revision(&self, session_id: &str) -> Option<(Status, u64)> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .map(|session| (session.meta.status, session.lifecycle_epoch))
    }

    /// Update a session only if both its status and lifecycle identity still
    /// match `expected`. Passing `None` accepts any current revision.
    pub fn set_status_if_revision(
        &self,
        session_id: &str,
        expected: Option<(Status, u64)>,
        status: Status,
        detail: Option<String>,
    ) -> bool {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            // A completed turn is reported twice: first as `TurnEnded`, then as
            // Busy -> Running. Whichever arrives first owns releasing the guard;
            // the completion latch keeps the duplicate edge from releasing the
            // NEXT prompt that the first edge may already have drained. A fresh
            // Busy edge is the authoritative start of the next turn and rearms
            // the latch. Starting -> Running must never release a guard because a
            // revived worker can still have our prompt queued downstream.
            let was = s.meta.status;
            if expected.is_some_and(|expected| expected != (was, s.lifecycle_epoch)) {
                return false;
            }
            if status == Status::Busy && was != Status::Busy {
                s.turn_completion_latched = false;
            }
            let completed_turn =
                was == Status::Busy && status == Status::Running && !s.turn_completion_latched;
            let terminated = matches!(
                status,
                Status::Exited | Status::Crashed | Status::Interrupted
            );
            if completed_turn || terminated {
                s.in_flight = false;
                s.turn_completion_latched = true;
            }
            if was != status {
                s.lifecycle_epoch = s.lifecycle_epoch.wrapping_add(1);
            }
            // Background work belongs to one live worker. A recoverable Crashed
            // hold keeps that worker, so only a restart or exit clears it.
            if matches!(
                status,
                Status::Starting | Status::Exited | Status::Interrupted
            ) {
                s.meta.background_tasks = 0;
            }
            if status != Status::Starting {
                s.meta.provider_update = None;
            }
            s.meta.status = status;
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateStatus {
                session_id: session_id.to_owned(),
                status,
            });
        }
        self.push(session_id, Event::Lifecycle { status, detail });
        self.broadcast_sessions();
        // A turn-end / death may make the session drainable — try the next
        // queued prompt now (no-op if still busy, held, or nothing queued).
        self.try_drain(session_id);
        true
    }

    /// Append an event to a session's log under the next `seq` and fan it out.
    /// Unknown sessions are ignored (a race with teardown).
    pub fn push(&self, session_id: &str, event: Event) {
        self.push_tagged(session_id, event, None);
    }

    /// Like [`Self::push`] but stamps a live `cmid` on the broadcast envelope —
    /// used to tag a dispatched prompt's user-message echo so the originating
    /// client reconciles its optimistic bubble (see Envelope::cmid).
    pub fn push_tagged(&self, session_id: &str, event: Event, cmid: Option<String>) {
        let mut event = event;
        crate::persistence::compact_inbound_event(&mut event);
        if let Some(artifacts) = self.inner.artifacts.lock().clone()
            && let Event::Update { update } = &mut event
            && let Err(error) = artifacts.externalize_images(update)
        {
            tracing::warn!(%error, session_id, "live image externalize failed");
        }
        let (envelope, durable_agent_session_id) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            // A permission answered on two devices at once resolves once. The
            // second answer is the same fact: the first row stays and no
            // duplicate is appended (docs/offline-first-sync.md, conflict 7).
            if let Event::PermissionResolved { request_id, .. } = &event
                && s.log.iter().rev().any(|entry| {
                    matches!(
                        &entry.event,
                        Event::PermissionResolved { request_id: seen, .. } if seen == request_id
                    )
                })
            {
                return;
            }
            let seq = s.next_seq;
            s.next_seq += 1;
            s.last_activity = std::time::Instant::now();
            let envelope = Envelope {
                session_id: session_id.to_owned(),
                seq,
                event,
                cmid,
            };
            if is_user_message_chunk(&envelope)
                && let Some(cmid) = envelope.cmid.as_deref()
                && let Some(dispatched) =
                    s.dispatched_cmids.iter_mut().find(|seen| seen.cmid == cmid)
            {
                // The bounded hot transcript can evict this echo before a
                // disconnected sender retries. Keep delivery evidence for as
                // long as its dispatch record still prevents re-execution.
                dispatched.echoed = true;
            }
            if let Some(canonical) = self.inner.history_reducer.lock().reduce(envelope.clone()) {
                match s
                    .log
                    .binary_search_by_key(&canonical.seq, |entry| entry.seq)
                {
                    Ok(index) => {
                        s.log_bytes = s
                            .log_bytes
                            .saturating_sub(estimated_envelope_bytes(&s.log[index]))
                            .saturating_add(estimated_envelope_bytes(&canonical));
                        s.log[index] = canonical;
                    }
                    Err(_) if canonical.seq == seq => {
                        s.log_bytes = s
                            .log_bytes
                            .saturating_add(estimated_envelope_bytes(&canonical));
                        s.log.push(canonical);
                        s.event_count = s.event_count.saturating_add(1);
                    }
                    // The canonical row was already trimmed from the hot tail.
                    // Its durable UPSERT still lands below, but re-inserting an
                    // old seq here would break the tail's sorted cursor contract.
                    Err(_) => {}
                }
            }
            if self.inner.store_tx.is_some()
                && trim_hot_log(
                    &mut s.log,
                    &mut s.log_bytes,
                    true,
                    hot_tail_budget_bytes(s.meta.status),
                )
            {
                s.reached_start = false;
            }
            let durable_agent_session_id = is_user_message_chunk(&envelope)
                .then(|| s.meta.agent_session_id.clone())
                .flatten();
            (envelope, durable_agent_session_id)
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::AppendEvent(envelope.clone()));
            if let Some(agent_session_id) = durable_agent_session_id {
                let _ = tx.send(StoreWrite::SetAgentSessionId {
                    session_id: session_id.to_owned(),
                    agent_session_id: Some(agent_session_id),
                });
            }
        }
        self.fanout(Outbound::Event { envelope });
    }

    fn broadcast_sessions(&self) {
        self.fanout(Outbound::Sessions {
            sessions: self.session_list(),
        });
    }

    /// Snapshot the captured config options for one session — used by the
    /// WS connect handler to replay the agent's last-seen `configOptions`
    /// to a freshly-connected client (so its composer dropdowns hydrate
    /// without waiting for the next `config_option_update`).
    #[must_use]
    pub fn config_options(&self, session_id: &str) -> Option<serde_json::Value> {
        let sessions = self.inner.sessions.lock();
        sessions.get(session_id).and_then(|session| {
            projected_config_options(
                &session.meta.provider,
                session.meta.provider_behavior.as_ref(),
                &session.config_preferences,
                session.config_options.clone(),
            )
        })
    }

    /// Return only the last agent-backed options snapshot. Unlike
    /// [`Self::config_options`], this does not synthesize host-owned controls
    /// when the agent has not advertised any options yet.
    pub(crate) fn persisted_config_options(&self, session_id: &str) -> Option<serde_json::Value> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .and_then(|session| session.config_options.clone())
    }

    /// Return the durable values selected for a session. The returned object is
    /// safe to pass across the Machine boundary because it contains only ACP
    /// option ids and scalar values, never provider credentials.
    #[must_use]
    pub fn config_preferences(&self, session_id: &str) -> Option<serde_json::Value> {
        let sessions = self.inner.sessions.lock();
        sessions
            .get(session_id)
            .map(|session| session.config_preferences.clone())
    }

    /// Record one user-selected config value and immediately fan out the
    /// optimistic selection. The agent's later authoritative option snapshot
    /// will correct it if the provider normalizes or rejects the value.
    pub fn set_config_preference(
        &self,
        session_id: &str,
        config_id: String,
        value: serde_json::Value,
    ) -> Result<(), String> {
        if config_id.is_empty() || config_id.len() > 128 {
            return Err("configuration id is invalid".to_owned());
        }
        if !matches!(
            &value,
            serde_json::Value::String(_) | serde_json::Value::Bool(_)
        ) {
            return Err("configuration values must be a string id or boolean".to_owned());
        }
        let (preferences, options) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(session) = sessions.get_mut(session_id) else {
                return Err(format!("unknown session {session_id:?}"));
            };
            if !session.config_preferences.is_object() {
                session.config_preferences = serde_json::json!({});
            }
            session
                .config_preferences
                .as_object_mut()
                .expect("config preferences are an object")
                .insert(config_id.clone(), value.clone());
            self.inner
                .config_in_flight
                .lock()
                .entry(session_id.to_owned())
                .or_default()
                .insert(
                    config_id.clone(),
                    (value.clone(), std::time::Instant::now()),
                );
            let mut options = projected_config_options(
                &session.meta.provider,
                session.meta.provider_behavior.as_ref(),
                &session.config_preferences,
                session.config_options.clone(),
            );
            let options = options.as_mut().and_then(|options| {
                set_config_option_current_value(options, &config_id, &value)
                    .then(|| options.clone())
            });
            if let Some(options) = &options {
                session.config_options = Some(options.clone());
            }
            (session.config_preferences.clone(), options)
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateConfigPreferences {
                session_id: session_id.to_owned(),
                preferences,
            });
        }
        if let Some(options) = options {
            if let Some(tx) = self.inner.store_tx.as_ref() {
                let _ = tx.send(StoreWrite::UpdateConfigOptions {
                    session_id: session_id.to_owned(),
                    options: options.clone(),
                });
            }
            self.fanout(Outbound::ConfigOptions {
                session_id: session_id.to_owned(),
                options,
            });
        }
        Ok(())
    }

    /// Keep client-set values the agent has not answered yet in an agent
    /// snapshot. Pending values settle together, only when one snapshot
    /// reports every one of them: during rapid preset taps a stale answer can
    /// match one latest value by coincidence while the rest are still queued,
    /// and settling that one alone let the next stale answer flip it back.
    /// Values the agent no longer offers are dropped (unless a model change is
    /// pending, whose old-model snapshots cannot judge the new model's
    /// options), and everything expires so a lost command cannot pin a value
    /// the agent never applied.
    fn overlay_config_in_flight(&self, session_id: &str, options: &mut serde_json::Value) {
        const SETTLE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);
        let mut in_flight = self.inner.config_in_flight.lock();
        let Some(pending) = in_flight.get_mut(session_id) else {
            return;
        };
        let find = |options: &serde_json::Value, config_id: &str| {
            options.as_array().and_then(|options| {
                options
                    .iter()
                    .find(|option| {
                        option.get("id").and_then(serde_json::Value::as_str) == Some(config_id)
                    })
                    .cloned()
            })
        };
        let model_pending = pending.keys().any(|config_id| {
            find(options, config_id).is_some_and(|option| {
                option.get("category").and_then(serde_json::Value::as_str) == Some("model")
            })
        });
        pending.retain(|config_id, (value, set_at)| {
            find(options, config_id).is_some_and(|option| {
                set_at.elapsed() <= SETTLE_DEADLINE
                    && (model_pending || config_option_accepts(&option, value))
            })
        });
        let settled = pending.iter().all(|(config_id, (value, _))| {
            find(options, config_id)
                .is_some_and(|option| config_current_value(&option) == Some(value))
        });
        if settled {
            in_flight.remove(session_id);
            return;
        }
        for (config_id, (value, _)) in pending.iter() {
            set_config_option_current_value(options, config_id, value);
        }
    }

    /// Store the latest agent-advertised config options for a session and
    /// fan them out to every client. Called from acp.rs when the upstream
    /// emits a `config_option_update` notification, and from the
    /// `SetConfigOption` reply path (the agent's authoritative response
    /// refreshes the same array).
    pub fn set_config_options(&self, session_id: &str, options: serde_json::Value) {
        let (options, corrected_preferences) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let raw_options = options;
            let mut options = projected_config_options(
                &s.meta.provider,
                s.meta.provider_behavior.as_ref(),
                &s.config_preferences,
                Some(raw_options.clone()),
            )
            .expect("agent config options remain present after projection");
            let corrected = reconcile_config_preferences(Some(&options), &mut s.config_preferences);
            if corrected {
                options = projected_config_options(
                    &s.meta.provider,
                    s.meta.provider_behavior.as_ref(),
                    &s.config_preferences,
                    Some(raw_options),
                )
                .expect("agent config options remain present after projection");
            }
            self.overlay_config_in_flight(session_id, &mut options);
            s.config_options = Some(options.clone());
            (options, corrected.then(|| s.config_preferences.clone()))
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateConfigOptions {
                session_id: session_id.to_owned(),
                options: options.clone(),
            });
            if let Some(preferences) = corrected_preferences {
                let _ = tx.send(StoreWrite::UpdateConfigPreferences {
                    session_id: session_id.to_owned(),
                    preferences,
                });
            }
        }
        self.fanout(Outbound::ConfigOptions {
            session_id: session_id.to_owned(),
            options,
        });
    }

    /// Record a session-scoped failure without changing the transcript or
    /// emitting another client notification. Some ACP agents report failures
    /// as ordinary message chunks, so their original transcript event remains
    /// visible while this durable incident feeds the diagnostic log.
    pub(crate) fn record_session_error(&self, session_id: &str, message: &str) {
        let occurred_at_ms = now_ms();
        let suffix = self.inner.next_error_id.fetch_add(1, Ordering::Relaxed);
        let incident_id = format!("session-error:{session_id}:{occurred_at_ms}:{suffix}");
        let mut message_end = message.len().min(4 * 1024);
        while !message.is_char_boundary(message_end) {
            message_end = message_end.saturating_sub(1);
        }
        let persisted_message = message[..message_end].to_owned();
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::RecordSessionError {
                id: incident_id.clone(),
                session_id: session_id.to_owned(),
                occurred_at_ms,
                message: persisted_message,
            });
        }
        tracing::error!(incident_id, session = %session_id, error = %message, "session error recorded");
    }

    /// Surface a command failure to every connected client so the UI can show
    /// a toast. Replaces the previous behaviour of silently logging to
    /// `tracing::warn` — that left the user staring at an unchanged page
    /// wondering why nothing happened.
    pub fn broadcast_error(&self, session_id: Option<String>, message: String) {
        if let Some(session_id) = session_id.as_ref() {
            self.record_session_error(session_id, &message);
        }
        self.fanout(Outbound::Error {
            session_id,
            message,
        });
    }

    /// Publish an addressed refusal for one client-authored command. Only the
    /// device that owns `cmid` acts on it; no session error is recorded because
    /// nothing went wrong with the agent.
    pub fn command_result(
        &self,
        session_id: &str,
        cmid: &str,
        outcome: CommandOutcome,
        message: &str,
    ) {
        self.fanout(Outbound::CommandResult {
            session_id: session_id.to_owned(),
            cmid: cmid.to_owned(),
            outcome,
            message: message.to_owned(),
        });
    }

    // --- Queue + drafts (server-authoritative, synced to every terminal) ------

    /// Current status of a session, if it exists. Lets the server decide
    /// busy-vs-idle for the force-push path without reaching into `Session`.
    #[must_use]
    pub fn status(&self, session_id: &str) -> Option<Status> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .map(|s| s.meta.status)
    }

    /// Latest explanatory crash detail for the current dead edge. Status-only
    /// worker snapshots may append a detail-less duplicate after the worker's
    /// richer ACP lifecycle event, so skip empty crash records while walking
    /// back, but never cross a non-crash lifecycle boundary.
    #[must_use]
    pub fn latest_crash_detail(&self, session_id: &str) -> Option<String> {
        let sessions = self.inner.sessions.lock();
        let session = sessions.get(session_id)?;
        latest_crash_detail_for_session(session).map(str::to_owned)
    }

    /// Newest crash detail which has not been superseded by a clean turn end.
    /// Unlike [`Self::latest_crash_detail`], this survives an idle worker
    /// snapshot projecting the session back to `Running` after the failure.
    #[must_use]
    pub fn unresolved_crash_detail(&self, session_id: &str) -> Option<String> {
        let sessions = self.inner.sessions.lock();
        let session = sessions.get(session_id)?;
        unresolved_crash_detail_for_session(session).map(str::to_owned)
    }

    fn next_qid(&self) -> String {
        format!("q{}", self.inner.next_qid.fetch_add(1, Ordering::Relaxed))
    }

    /// Re-broadcast (and persist) a session's queue + drafts after any change.
    /// Re-locks `sessions`, so callers MUST NOT hold the lock when calling.
    fn emit_pending(&self, session_id: &str) {
        let (queue, drafts) = {
            let sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get(session_id) else {
                return;
            };
            (s.queue.clone(), s.drafts.clone())
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdatePending {
                session_id: session_id.to_owned(),
                queue: queue.clone(),
                drafts: drafts.clone(),
            });
        }
        // Broadcast on the generic optimistic-sync channel as state "queue:<sid>".
        // Confirmed = the cmids in the value, so a client drops its optimistic add
        // the moment its cmid lands here (the cross-terminal reconcile).
        let confirmed = Self::cmids_of(&queue, &drafts);
        let value = serde_json::json!({ "queue": queue, "drafts": drafts });
        self.sync_emit(&format!("queue:{session_id}"), value, confirmed);
    }

    fn send_dispatch(&self, req: DispatchReq) {
        if let Some(tx) = self.inner.dispatch_tx.lock().as_ref()
            && let Err(error) = tx.try_send(req)
        {
            let req = error.into_inner();
            tracing::error!(session = %req.session_id, "dispatch queue rejected a prompt");
            self.clear_in_flight(&req.session_id);
            self.requeue_prompt(&req.session_id, req.text, req.content, req.cmid);
            self.broadcast_error(
                Some(req.session_id),
                "dispatch queue is full; prompt remains queued".to_owned(),
            );
        }
    }

    /// Whether a queued prompt can be dispatched now. `allow_revive` is the
    /// crucial distinction between the two callers:
    ///
    /// - **AUTO-drain** (turn-end / a queue mutation while idle) passes `false`,
    ///   so it ONLY fires into an alive, idle agent (`Running`). It must never
    ///   revive a dead one — otherwise an agent that exits or crashes MID-TASK
    ///   clears the in-flight guard on its death edge and the auto-drain would
    ///   immediately send the next queued prompt into a freshly-revived agent
    ///   that has lost the unfinished task. That is the "a task wasn't done but
    ///   the queue auto-sent" bug.
    /// - **EXPLICIT** sends (submit a new message, "send now") pass `true`,
    ///   deliberately reviving an exited/crashed session via `session/load`.
    ///
    /// Both require that nothing of ours is already in flight.
    fn ready(s: &Session, allow_revive: bool) -> bool {
        let can = if allow_revive {
            matches!(
                s.meta.status,
                Status::Running | Status::Exited | Status::Crashed | Status::Interrupted
            )
        } else {
            s.meta.status == Status::Running
        };
        can && !s.in_flight
    }

    /// Dispatch the head of a session's queue if it can take a turn and the head
    /// isn't held for editing. Pops the head, marks in-flight, hands the prompt
    /// to the dispatcher task, and re-broadcasts the shrunken queue. No-op
    /// otherwise. `allow_revive` is forwarded to [`Self::ready`].
    fn drain_head(&self, session_id: &str, allow_revive: bool, manual: bool) {
        // Without a dispatcher wired we must not pop (the prompt would be lost).
        if self.inner.dispatch_tx.lock().is_none() {
            return;
        }
        let req = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if !Self::ready(s, allow_revive) {
                return;
            }
            // The user MANUALLY paused the drain (the ⏸ toggle) → hold the queue
            // after the running turn finishes. A manual send still
            // overrides (the user can force a specific message through).
            if !manual && s.meta.paused {
                return;
            }
            if s.queue.is_empty() {
                return;
            }
            // ANY message being edited pauses the WHOLE auto-drain — not just when
            // the head is the one open (the old `== head` check let the head fire
            // out from under you while you edited message #2). The hold lifts on
            // Save/Cancel (set_queue_editing(None)). A MANUAL "send now" still
            // overrides (the user explicitly chose to send), so the queue is never
            // permanently trapped.
            if !manual && s.editing.is_some() {
                return;
            }
            let head = s.queue.remove(0);
            s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
            s.in_flight = true;
            Self::note_dispatched(s, head.cmid.as_deref());
            DispatchReq {
                session_id: session_id.to_owned(),
                text: head.text,
                content: head.content,
                cmid: head.cmid,
            }
        };
        self.emit_pending(session_id);
        self.send_dispatch(req);
    }

    /// The AUTO-drain: only fires into an alive idle agent (never revives a dead
    /// one). Called after every queue mutation and on every status change.
    fn try_drain(&self, session_id: &str) {
        self.drain_head(session_id, false, false);
    }

    /// MANUAL drain of the queue head: bypasses the paused hold
    /// (the user explicitly chose "send now") and revives a dormant session. Used
    /// by force-push so a ⚡ on a PAUSED queue runs the front message immediately
    /// WITHOUT resuming the rest of the held queue.
    pub fn drain_now(&self, session_id: &str) {
        self.drain_head(session_id, true, true);
    }

    /// Clear the in-flight guard (used by the dispatcher when a send fails) and
    /// try the next queued prompt.
    pub fn clear_in_flight(&self, session_id: &str) {
        {
            let mut sessions = self.inner.sessions.lock();
            if let Some(s) = sessions.get_mut(session_id) {
                s.in_flight = false;
            }
        }
        self.try_drain(session_id);
    }

    /// Complete one agent turn and release at most one dispatch guard.
    ///
    /// Runtime transports publish both `TurnEnded` and a trailing idle status.
    /// The first completion drains the next prompt; the second must be a no-op
    /// until that prompt reports its own `Busy` start, otherwise two queued
    /// prompts can be handed to the same worker at once.
    pub fn complete_turn(&self, session_id: &str) {
        let first_completion = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if s.turn_completion_latched {
                false
            } else {
                s.in_flight = false;
                s.turn_completion_latched = true;
                true
            }
        };
        if first_completion {
            self.try_drain(session_id);
        }
    }

    /// Reconcile an authoritative runtime snapshot that says the worker is idle.
    ///
    /// Remote worker lifecycle events can straddle a Machine broker reconnect.
    /// If Cowboy missed the Busy -> Running edge, the Hub may still retain the
    /// dispatch guard even though the worker snapshot proves that no turn is
    /// active. Callers must first prove that no prompt command is still pending
    /// in the controller; otherwise a Running snapshot can merely predate a
    /// prompt that is still travelling to the worker.
    pub fn reconcile_runtime_idle(&self, session_id: &str, expected_epoch: u64) {
        let released = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if s.meta.status != Status::Running
                || !s.in_flight
                || s.in_flight_epoch != expected_epoch
            {
                false
            } else {
                s.in_flight = false;
                s.turn_completion_latched = true;
                true
            }
        };
        if released {
            tracing::warn!(
                session = %session_id,
                "authoritative idle runtime snapshot released stale in-flight guard"
            );
            self.try_drain(session_id);
        }
    }

    /// Put a dispatched-but-never-run prompt BACK on the queue front.
    ///
    /// A prompt sent to a session that had to REVIVE rides `cmd_rx` into the
    /// freshly-spawned agent thread. If that agent dies during cold-start (e.g.
    /// the `npx` adapter fails to install) it returns BEFORE the command loop
    /// consumes `cmd_rx`, so the prompt is never logged and — without this —
    /// evaporates with the dead thread: gone from the composer (cleared on send),
    /// the queue (it was dispatched straight through), the transcript (never
    /// echoed), and even Retry (which reads the log). `run_agent` salvages the
    /// un-consumed prompt here so it lands back in the durable queue — visible
    /// again, and re-drained the moment the session next reaches `Running`.
    /// Idempotent on `cmid` (a racing re-revive must not double-queue it).
    pub fn requeue_prompt(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            // The in-flight turn is over (it crashed); free the guard so the queue
            // can drain again once an agent is alive.
            s.in_flight = false;
            // The prompt never ran: it is no longer "delivered", so the drain (or
            // a client resend) may hand it to a worker again.
            if let Some(c) = cmid.as_deref() {
                s.dispatched_cmids.retain(|seen| seen.cmid != c);
            }
            if let Some(c) = cmid.as_deref()
                && s.queue.iter().any(|m| m.cmid.as_deref() == Some(c))
            {
                return;
            }
            let id = self.next_qid();
            s.queue.insert(
                0,
                QueuedMessage {
                    id,
                    text,
                    content,
                    cmid,
                    schedule: None,
                },
            );
        }
        self.emit_pending(session_id);
    }

    /// Queue-aware send: dispatch immediately when the session is idle and
    /// nothing is queued/in-flight; otherwise append to the queue. The single
    /// entry point the Web composer uses (the bridge/API still use `Prompt`).
    pub fn submit(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) {
        self.submit_inner(session_id, text, content, cmid, false);
    }

    fn submit_inner(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        managed: bool,
    ) {
        // One controller owns a managed child conversation: its parent call.
        if !managed && self.is_managed_child(session_id) {
            self.broadcast_error(
                Some(session_id.to_owned()),
                "This child conversation is controlled by its parent's managed call".to_owned(),
            );
            return;
        }
        // A human (or any non-wakeup) submit resets the scheduler's runaway guard
        // — the autonomous-fire streak only counts unattended iterations.
        if !cmid
            .as_deref()
            .is_some_and(|c| c.starts_with(crate::scheduler::WAKEUP_PREFIX))
        {
            self.notify_human_turn(session_id);
        }
        let wired = self.inner.dispatch_tx.lock().is_some();
        let mut dispatch = None;
        let duplicate;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            // Idempotent on cmid: a retry whose original landed in the queue, was
            // handed to a worker, or already echoed into the transcript must not
            // run again. The reconnecting client resends every unconfirmed submit.
            duplicate = cmid
                .as_deref()
                .filter(|c| Self::prompt_already_accepted(s, c))
                .map(str::to_owned);
            if duplicate.is_some() {
                // fall through to the addressed confirmation below
            } else if wired && Self::ready(s, true) && s.queue.is_empty() {
                s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
                s.in_flight = true;
                Self::note_dispatched(s, cmid.as_deref());
                dispatch = Some(DispatchReq {
                    session_id: session_id.to_owned(),
                    text,
                    content,
                    cmid,
                });
            } else {
                let id = self.next_qid();
                s.queue.push(QueuedMessage {
                    id,
                    text,
                    content,
                    cmid,
                    schedule: None,
                });
            }
        }
        if let Some(cmid) = duplicate {
            self.confirm_delivered(session_id, &cmid);
            return;
        }
        match dispatch {
            // Dispatched straight through — never touched a list, so no flicker
            // of the prompt appearing-then-leaving the queue.
            Some(req) => self.send_dispatch(req),
            None => self.emit_pending(session_id),
        }
    }

    /// Force-push a fresh prompt (the long-press-send affordance). When a turn is
    /// in flight, the prompt jumps to the FRONT of the queue and this returns
    /// `true` so the caller interrupts the running turn — the cancelled turn ends,
    /// the drain then runs this prompt next. On an idle session there's nothing to
    /// jump ahead of, so it behaves exactly like `submit` (dispatches straight
    /// through) and returns `false`. Same cmid-idempotency as `submit`.
    #[must_use]
    pub fn force_submit(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        // `true` = also interrupt the running turn (force push). `false` = just
        // jump to the front of the queue and let the current turn finish first
        // ("jump to front" / `submit { front: true }`).
        interrupt_on_busy: bool,
    ) -> bool {
        if self.is_managed_child(session_id) {
            return false;
        }
        let wired = self.inner.dispatch_tx.lock().is_some();
        let mut dispatch = None;
        let mut interrupt = false;
        let duplicate;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            duplicate = cmid
                .as_deref()
                .filter(|c| Self::prompt_already_accepted(s, c))
                .map(str::to_owned);
            if duplicate.is_some() {
                // fall through to the addressed confirmation below
            } else if wired && Self::ready(s, true) && s.queue.is_empty() {
                // Idle + nothing queued → straight dispatch, identical to submit.
                s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
                s.in_flight = true;
                Self::note_dispatched(s, cmid.as_deref());
                dispatch = Some(DispatchReq {
                    session_id: session_id.to_owned(),
                    text,
                    content,
                    cmid,
                });
            } else {
                // Busy / draining / queued ahead → jump to the FRONT so it runs
                // next; ask the caller to interrupt the in-flight turn only when
                // this is a force push (not a no-interrupt "jump to front").
                let id = self.next_qid();
                s.queue.insert(
                    0,
                    QueuedMessage {
                        id,
                        text,
                        content,
                        cmid,
                        schedule: None,
                    },
                );
                interrupt = interrupt_on_busy;
            }
        }
        if let Some(cmid) = duplicate {
            self.confirm_delivered(session_id, &cmid);
            return false;
        }
        match dispatch {
            Some(req) => self.send_dispatch(req),
            None => self.emit_pending(session_id),
        }
        interrupt
    }

    /// Whether `cmid` names a prompt a browser client minted. Hub-synthesized
    /// ids (scheduler wakeups, scheduled drafts, retry, legacy continuation)
    /// are deliberately reusable and never participate in delivery dedupe.
    fn is_client_cmid(cmid: &str) -> bool {
        !cmid.is_empty() && !cmid.starts_with("__") && !cmid.starts_with("cowboy-")
    }

    /// True once this Hub has accepted the prompt: still queued, handed to a
    /// worker, or echoed into the transcript (which survives
    /// a Controller restart because the echo persists its `cmid`).
    fn prompt_already_accepted(s: &Session, cmid: &str) -> bool {
        if s.queue.iter().any(|m| m.cmid.as_deref() == Some(cmid)) {
            return true;
        }
        Self::is_client_cmid(cmid)
            && (s.dispatched_cmids.iter().any(|seen| seen.cmid == cmid)
                || s.log
                    .iter()
                    .rev()
                    .any(|entry| entry.cmid.as_deref() == Some(cmid)))
    }

    const DISPATCHED_CMID_WINDOW: usize = 64;

    fn note_dispatched(s: &mut Session, cmid: Option<&str>) {
        let Some(cmid) = cmid.filter(|c| Self::is_client_cmid(c)) else {
            return;
        };
        if s.dispatched_cmids.iter().any(|seen| seen.cmid == cmid) {
            return;
        }
        if s.dispatched_cmids.len() >= Self::DISPATCHED_CMID_WINDOW {
            s.dispatched_cmids.pop_front();
        }
        s.dispatched_cmids.push_back(DispatchedPrompt {
            cmid: cmid.to_owned(),
            echoed: false,
        });
    }

    /// Tell every client that `cmid` is already folded into this session, without
    /// changing or re-persisting the queue. Confirmations are monotonic facts the
    /// sync client accepts from any patch, so the resending device retires its
    /// outbox entry instead of waiting out an acknowledgement timeout.
    fn confirm_delivered(&self, session_id: &str, cmid: &str) {
        let (queue, drafts) = {
            let sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get(session_id) else {
                return;
            };
            // Dispatch admission deduplicates execution but does not prove
            // delivery. Until a queue row or user echo owns the prompt, the
            // browser must retain its durable outbox (including attachments).
            if !s
                .queue
                .iter()
                .chain(&s.drafts)
                .any(|m| m.cmid.as_deref() == Some(cmid))
                && !s
                    .dispatched_cmids
                    .iter()
                    .any(|seen| seen.cmid == cmid && seen.echoed)
                && !s.log.iter().any(|entry| {
                    entry.cmid.as_deref() == Some(cmid) && is_user_message_chunk(entry)
                })
            {
                return;
            }
            (s.queue.clone(), s.drafts.clone())
        };
        let mut confirmed = Self::cmids_of(&queue, &drafts);
        if !confirmed.iter().any(|seen| seen == cmid) {
            confirmed.push(cmid.to_owned());
        }
        let value = serde_json::json!({ "queue": queue, "drafts": drafts });
        self.sync_emit(&format!("queue:{session_id}"), value, confirmed);
    }

    /// Drop one queued prompt. Returns whether the row was still queued.
    pub fn remove_queued(&self, session_id: &str, id: &str) -> bool {
        let removed = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            let before = s.queue.len();
            s.queue.retain(|m| m.id != id);
            if s.editing.as_deref() == Some(id) {
                s.editing = None;
            }
            before != s.queue.len()
        };
        self.emit_pending(session_id);
        self.try_drain(session_id);
        removed
    }

    /// Remove exactly one still-queued prompt by its client correlation id.
    ///
    /// ACP `session/cancel` is session-scoped, but a bridge prompt can be
    /// waiting behind a turn started by another surface. Cancelling that
    /// request must not interrupt the unrelated active turn. Returns whether a
    /// queued prompt was removed; an already-dispatched prompt is absent and
    /// must instead be cancelled through the provider.
    pub fn remove_queued_by_cmid(&self, session_id: &str, cmid: &str) -> bool {
        let removed = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            let before = s.queue.len();
            s.queue.retain(|m| m.cmid.as_deref() != Some(cmid));
            before != s.queue.len()
        };
        if removed {
            self.emit_pending(session_id);
            self.try_drain(session_id);
        }
        removed
    }

    /// Edit a queued prompt in place. Empty text + content removes it. Returns
    /// whether the row was still queued.
    pub fn edit_queued(
        &self,
        session_id: &str,
        id: &str,
        text: String,
        content: Vec<serde_json::Value>,
    ) -> bool {
        let found = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if text.trim().is_empty() && content.is_empty() {
                let before = s.queue.len();
                s.queue.retain(|m| m.id != id);
                if s.editing.as_deref() == Some(id) {
                    s.editing = None;
                }
                before != s.queue.len()
            } else if let Some(m) = s.queue.iter_mut().find(|m| m.id == id) {
                m.text = text;
                m.content = content;
                true
            } else {
                false
            }
        };
        self.emit_pending(session_id);
        self.try_drain(session_id);
        found
    }

    /// Drop a session's whole queue.
    pub fn clear_queue(&self, session_id: &str) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.queue.clear();
            s.editing = None;
        }
        self.emit_pending(session_id);
    }

    /// "Send now": move a queued prompt to the front, then dispatch it. This is
    /// an EXPLICIT user action, so it may revive an exited/crashed session
    /// (`allow_revive` = true) — unlike the auto-drain. If the agent is mid-turn it
    /// just becomes next in line.
    pub fn request_send_queued(&self, session_id: &str, id: &str) {
        if self.is_managed_child(session_id) {
            return;
        }
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if let Some(pos) = s.queue.iter().position(|m| m.id == id) {
                let m = s.queue.remove(pos);
                s.queue.insert(0, m);
            } else {
                return;
            }
        }
        self.emit_pending(session_id);
        // Explicit user "send now" → manual drain, bypassing the awaiting hold.
        self.drain_head(session_id, true, true);
    }

    /// Overlay "Retry" for an errored/crashed turn: re-run the last user prompt
    /// (reviving the session). No-op if there's no prior prompt.
    pub fn retry_turn(&self, session_id: &str) {
        if self.is_managed_child(session_id) {
            return;
        }
        let (prompt, status, retry_cmid, already_queued) = {
            let sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get(session_id) else {
                tracing::warn!(session = %session_id, "retry_turn: unknown session — no-op");
                return;
            };
            let Some(last_user_seq) = s.log.iter().rev().find_map(|envelope| {
                let Event::Update { update } = &envelope.event else {
                    return None;
                };
                (update
                    .get("sessionUpdate")
                    .and_then(serde_json::Value::as_str)
                    == Some("user_message_chunk"))
                .then_some(envelope.seq)
            }) else {
                tracing::warn!(session = %session_id, "retry_turn: no prior user event — no-op");
                return;
            };
            let retry_cmid = format!("cowboy-retry:{session_id}:{last_user_seq}");
            let prompt = last_turn_texts(&s.log).0;
            if s.in_flight
                || s.queue
                    .iter()
                    .any(|message| message.cmid.as_deref() == Some(retry_cmid.as_str()))
            {
                tracing::info!(session = %session_id, "retry_turn: identical retry already pending — no-op");
                return;
            }
            let already_queued = s.queue.iter().any(|message| message.text == prompt);
            (prompt, s.meta.status, retry_cmid, already_queued)
        };
        if prompt.trim().is_empty() {
            // The "no response" report points here first: a crashed turn whose
            // user prompt never made it into the log leaves nothing to re-run.
            tracing::warn!(session = %session_id, ?status, "retry_turn: no prior prompt to retry — no-op");
            return;
        }
        if already_queued {
            tracing::info!(session = %session_id, "retry_turn: rejected prompt already queued — draining without duplication");
            self.drain_head(session_id, true, true);
            return;
        }
        tracing::info!(session = %session_id, ?status, prompt_len = prompt.len(), "retry_turn: re-submitting last prompt");
        let _ = self.force_submit(session_id, prompt, Vec::new(), Some(retry_cmid), true);
        // force_submit DISPATCHES only when the queue is empty AND the session is
        // ready; with messages already queued — or a crashed/exited session — it
        // just parks the prompt at the queue FRONT and emits pending. That left
        // Retry looking like "added to the top of the queue, now send it yourself".
        // Drain the head WITH revive so Retry runs
        // the prompt immediately, reviving a dead session — no manual send. Safe
        // after a direct dispatch too: force_submit set `in_flight`, so `ready`
        // returns false and this drain no-ops (no double send).
        self.drain_head(session_id, true, true);
    }

    /// Move a queued prompt back to drafts.
    pub fn queued_to_draft(&self, session_id: &str, id: &str) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if let Some(pos) = s.queue.iter().position(|m| m.id == id) {
                let m = s.queue.remove(pos);
                if s.editing.as_deref() == Some(id) {
                    s.editing = None;
                }
                s.drafts.push(m);
            } else {
                return;
            }
        }
        self.emit_pending(session_id);
        self.try_drain(session_id);
    }

    /// Hold (`Some`) or release (`None`) the queue head for editing. A held head
    /// pauses the drain on every terminal; releasing tries the drain again.
    pub fn set_queue_editing(&self, session_id: &str, id: Option<String>) -> u64 {
        let released = id.is_none();
        let epoch = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return 0;
            };
            s.editing_epoch = s.editing_epoch.wrapping_add(1);
            s.editing = id;
            s.editing_epoch
        };
        if released {
            self.try_drain(session_id);
        }
        epoch
    }

    /// Release a disconnected editor only if no replacement connection has
    /// renewed the same transaction during the reload grace period.
    pub fn release_queue_editing_if_epoch(&self, session_id: &str, id: &str, epoch: u64) -> bool {
        let released = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if s.editing_epoch != epoch || s.editing.as_deref() != Some(id) {
                return false;
            }
            s.editing = None;
            s.editing_epoch = s.editing_epoch.wrapping_add(1);
            true
        };
        if released {
            self.try_drain(session_id);
        }
        released
    }

    /// Park a new draft.
    pub fn add_draft(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) {
        let duplicate;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            // Idempotent on cmid: when a send looked failed the client resends
            // with the SAME cmid, but the original may actually have landed — so
            // a matching cmid means "already staged", don't double-add. Confirm
            // it again instead: the resending client missed the first patch.
            duplicate = cmid
                .as_deref()
                .filter(|c| s.drafts.iter().any(|m| m.cmid.as_deref() == Some(*c)))
                .map(str::to_owned);
            if duplicate.is_none() {
                let id = self.next_qid();
                s.drafts.push(QueuedMessage {
                    id,
                    text,
                    content,
                    cmid,
                    schedule: None,
                });
            }
        }
        if let Some(cmid) = duplicate {
            self.confirm_delivered(session_id, &cmid);
            return;
        }
        self.emit_pending(session_id);
    }

    /// Attach (or update) a future fire time on a draft — the user-driven analog
    /// of the agent's `ScheduleWakeup`. Targets an existing draft by `id`, else
    /// by `cmid`, else creates a fresh draft carrying the schedule. `text`/
    /// `content` overwrite the target only when non-empty (a reschedule-in-place
    /// from the chip passes the current text; a bare time-change can pass empty).
    /// Persists via the drafts jsonb and arms the server-side timer. Broadcasts
    /// the session list too so the row clock badge updates promptly.
    #[allow(clippy::too_many_arguments)]
    pub fn schedule_draft(
        &self,
        session_id: &str,
        id: Option<String>,
        cmid: Option<String>,
        text: String,
        content: Vec<serde_json::Value>,
        fire_at_ms: i64,
        delivery: Delivery,
    ) {
        let draft_id = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let pos = id
                .as_deref()
                .and_then(|i| s.drafts.iter().position(|m| m.id == i))
                .or_else(|| {
                    cmid.as_deref()
                        .and_then(|c| s.drafts.iter().position(|m| m.cmid.as_deref() == Some(c)))
                });
            let schedule = Some(DraftSchedule {
                fire_at_ms,
                delivery,
            });
            match pos {
                Some(p) => {
                    let m = &mut s.drafts[p];
                    if !text.trim().is_empty() || !content.is_empty() {
                        m.text = text;
                        m.content = content;
                    }
                    m.schedule = schedule;
                    m.id.clone()
                }
                None => {
                    let did = self.next_qid();
                    s.drafts.push(QueuedMessage {
                        id: did.clone(),
                        text,
                        content,
                        cmid,
                        schedule,
                    });
                    did
                }
            }
        };
        self.emit_pending(session_id);
        self.broadcast_sessions();
        self.arm_draft_timer(session_id, &draft_id, fire_at_ms);
    }

    /// Strip the schedule off a draft, leaving it a plain parked draft, and cancel
    /// its timer. No-op if the draft is gone or wasn't scheduled.
    pub fn unschedule_draft(&self, session_id: &str, id: &str) {
        let cleared = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            match s.drafts.iter_mut().find(|m| m.id == id) {
                Some(m) if m.schedule.is_some() => {
                    m.schedule = None;
                    true
                }
                _ => false,
            }
        };
        if cleared {
            self.emit_pending(session_id);
            self.broadcast_sessions();
            self.cancel_draft_timer(session_id, id);
        }
    }

    /// Fire a scheduled draft (called by the scheduler at its fire time): remove
    /// it from drafts and submit it per its `delivery`. Tagged with `SCHED_PREFIX`
    /// so the echo renders as a "↻ scheduled" note. No-op if the draft is gone
    /// (the user removed/activated it before it fired — the timer was cancelled,
    /// but a fire already in-flight is harmless here).
    pub fn fire_scheduled_draft(&self, session_id: &str, draft_id: &str) {
        if self.is_managed_child(session_id) {
            return;
        }
        let fired = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.drafts
                .iter()
                .position(|m| m.id == draft_id)
                .map(|pos| s.drafts.remove(pos))
        };
        let Some(m) = fired else {
            return;
        };
        // The draft left the drafts list → refresh the pending panel and the
        // session list (its next_schedule_ms just changed).
        self.emit_pending(session_id);
        self.broadcast_sessions();

        let front = m
            .schedule
            .as_ref()
            .is_some_and(|sc| sc.delivery == Delivery::Front);
        let cmid = Some(format!("{SCHED_PREFIX}{session_id}-{draft_id}"));

        // Land it in the queue. A scheduled fire ALWAYS respects a paused queue —
        // it never bypasses the ⏸ hold — and never interrupts a live turn. It
        // dispatches straight through ONLY when the session is idle, unpaused, and
        // the queue is empty; otherwise it enqueues (head for Front, tail for Back)
        // and the normal drain runs it when the queue resumes / the turn ends.
        let wired = self.inner.dispatch_tx.lock().is_some();
        let mut dispatch = None;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if let Some(c) = cmid.as_deref()
                && s.queue.iter().any(|q| q.cmid.as_deref() == Some(c))
            {
                return;
            }
            if wired && !s.meta.paused && Self::ready(s, true) && s.queue.is_empty() {
                s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
                s.in_flight = true;
                dispatch = Some(DispatchReq {
                    session_id: session_id.to_owned(),
                    text: m.text,
                    content: m.content,
                    cmid,
                });
            } else {
                let id = self.next_qid();
                let msg = QueuedMessage {
                    id,
                    text: m.text,
                    content: m.content,
                    cmid,
                    schedule: None,
                };
                if front {
                    s.queue.insert(0, msg);
                } else {
                    s.queue.push(msg);
                }
            }
        }
        match dispatch {
            Some(req) => self.send_dispatch(req),
            None => self.emit_pending(session_id),
        }
    }

    /// Edit a draft in place. Empty text + content removes it.
    /// Edit a draft in place. Empty text + content removes it. Returns whether
    /// the draft still existed.
    pub fn edit_draft(
        &self,
        session_id: &str,
        id: &str,
        text: String,
        content: Vec<serde_json::Value>,
    ) -> bool {
        let (found, unscheduled) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if text.trim().is_empty() && content.is_empty() {
                let had = s.drafts.iter().any(|m| m.id == id && m.schedule.is_some());
                let before = s.drafts.len();
                s.drafts.retain(|m| m.id != id);
                (before != s.drafts.len(), had)
            } else if let Some(m) = s.drafts.iter_mut().find(|m| m.id == id) {
                m.text = text;
                m.content = content;
                (true, false)
            } else {
                (false, false)
            }
        };
        self.emit_pending(session_id);
        if unscheduled {
            self.cancel_draft_timer(session_id, id);
            self.broadcast_sessions();
        }
        found
    }

    /// Compare and remove under one lock. A delayed Undo may never remove a
    /// row another device has edited, scheduled or already sent.
    pub fn remove_draft_if_unchanged(
        &self,
        session_id: &str,
        identity: &str,
        text: &str,
        content: &[serde_json::Value],
    ) -> bool {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(session) = sessions.get_mut(session_id) else {
                return false;
            };
            let Some(index) = session
                .drafts
                .iter()
                .position(|row| row.cmid.as_deref() == Some(identity) || row.id == identity)
            else {
                return false;
            };
            let row = &session.drafts[index];
            if row.schedule.is_some() || row.text != text || row.content != content {
                return false;
            }
            session.drafts.remove(index);
        }
        self.emit_pending(session_id);
        true
    }

    /// Drop one draft. Returns whether the draft still existed.
    pub fn remove_draft(&self, session_id: &str, id: &str) -> bool {
        let (removed, unscheduled) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            let had = s.drafts.iter().any(|m| m.id == id && m.schedule.is_some());
            let before = s.drafts.len();
            s.drafts.retain(|m| m.id != id);
            (before != s.drafts.len(), had)
        };
        self.emit_pending(session_id);
        if unscheduled {
            self.cancel_draft_timer(session_id, id);
            self.broadcast_sessions();
        }
        removed
    }

    /// Move a draft out of `from`'s draft list and onto the END of `to`'s. The
    /// "parked in the wrong session" fix. No-op if `from == to`, either session
    /// is unknown, or the id isn't a draft of `from`. Crucially, the draft is
    /// pulled out ONLY when `to` exists, so a bad destination can never drop the
    /// message. Both sessions are persisted + broadcast.
    pub fn move_draft(&self, from: &str, id: &str, to: &str) {
        if from == to {
            return;
        }
        let scheduled = {
            let mut sessions = self.inner.sessions.lock();
            // Take the draft out of `from`, but only if `to` exists to receive it.
            let Some(msg) = (if sessions.contains_key(to) {
                sessions.get_mut(from).and_then(|s| {
                    s.drafts
                        .iter()
                        .position(|m| m.id == id)
                        .map(|pos| s.drafts.remove(pos))
                })
            } else {
                None
            }) else {
                return;
            };
            let scheduled = msg.schedule.as_ref().map(|sc| sc.fire_at_ms);
            // `to` existed at the top of this lock and we still hold it, so this
            // can't miss; the `if let` just avoids an unwrap.
            if let Some(dst) = sessions.get_mut(to) {
                dst.drafts.push(msg);
            }
            scheduled
        };
        self.emit_pending(from);
        self.emit_pending(to);
        // A scheduled draft keeps its fire time across the move — retarget the
        // timer from the source session to the destination (same draft id).
        if let Some(fire_at_ms) = scheduled {
            self.cancel_draft_timer(from, id);
            self.arm_draft_timer(to, id, fire_at_ms);
            self.broadcast_sessions();
        }
    }

    /// Drop a session's whole draft list.
    pub fn clear_drafts(&self, session_id: &str) {
        let scheduled_ids: Vec<String> = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let ids = s
                .drafts
                .iter()
                .filter(|m| m.schedule.is_some())
                .map(|m| m.id.clone())
                .collect();
            s.drafts.clear();
            ids
        };
        self.emit_pending(session_id);
        for did in &scheduled_ids {
            self.cancel_draft_timer(session_id, did);
        }
        if !scheduled_ids.is_empty() {
            self.broadcast_sessions();
        }
    }

    /// Activate one draft: remove it from drafts and submit it (send-or-queue).
    pub fn activate_draft(&self, session_id: &str, id: &str, cmid: Option<String>) {
        if self.is_managed_child(session_id) {
            return;
        }
        let msg = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.drafts
                .iter()
                .position(|m| m.id == id)
                .map(|pos| s.drafts.remove(pos))
        };
        if let Some(m) = msg {
            self.emit_pending(session_id);
            // Manually sending a scheduled draft now → cancel its pending fire.
            if m.schedule.is_some() {
                self.cancel_draft_timer(session_id, id);
                self.broadcast_sessions();
            }
            // Echo the explicit send identity so the sender can retire its
            // optimistic bubble. Older clients retain the draft's identity.
            self.submit(session_id, m.text, m.content, cmid.or(m.cmid));
        }
    }

    /// Activate every draft, front-to-back, then clear them.
    pub fn activate_all_drafts(&self, session_id: &str) {
        if self.is_managed_child(session_id) {
            return;
        }
        let msgs = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            std::mem::take(&mut s.drafts)
        };
        if msgs.is_empty() {
            return;
        }
        self.emit_pending(session_id);
        let mut had_scheduled = false;
        for m in msgs {
            // Manually sending a scheduled draft now → cancel its pending fire.
            if m.schedule.is_some() {
                self.cancel_draft_timer(session_id, &m.id);
                had_scheduled = true;
            }
            // Bulk activation preserves the same identity as a single send.
            self.submit(session_id, m.text, m.content, m.cmid);
        }
        if had_scheduled {
            self.broadcast_sessions();
        }
    }

    // --- Reorder --------------------------------------------------------------

    /// Reorder one session's queue to the given id order, then re-broadcast +
    /// persist. Ids not in `order` keep their relative order at the end (a
    /// stable sort), so a stale/partial order can't drop messages. Also re-tries
    /// the drain in case the new head is now dispatchable.
    pub fn reorder_queue(&self, session_id: &str, order: &[String]) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            sort_by_id_order(&mut s.queue, order, |m| &m.id);
        }
        self.emit_pending(session_id);
        self.try_drain(session_id);
    }

    /// Reorder one session's drafts to the given id order (see `reorder_queue`).
    pub fn reorder_drafts(&self, session_id: &str, order: &[String]) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            sort_by_id_order(&mut s.drafts, order, |m| &m.id);
        }
        self.emit_pending(session_id);
    }

    /// Reorder the session list to the given id order, then persist + broadcast.
    /// Submitted ids only permute names they include; existing ids are never dropped.
    pub fn reorder_sessions(&self, order: &[String]) {
        {
            let mut list = self.inner.order.lock();
            *list = merge_session_order(&list, order);
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateSessionOrder {
                order: self.inner.order.lock().clone(),
            });
        }
        self.broadcast_sessions();
    }
}

/// Merge a submitted session-id permutation into the global order.
///
/// Every id already in `existing` survives. `submitted` only permutes the
/// names it actually includes; omitted visible ids stay in place with hidden
/// ones. Brand-new ids (in `submitted` but not `existing`) are appended.
#[must_use]
pub fn merge_session_order(existing: &[String], submitted: &[String]) -> Vec<String> {
    let mut seen_submitted = HashSet::new();
    let submitted: Vec<String> = submitted
        .iter()
        .filter(|id| seen_submitted.insert((*id).as_str()))
        .cloned()
        .collect();
    let existing_set: HashSet<&str> = existing.iter().map(String::as_str).collect();
    let named: Vec<String> = submitted
        .iter()
        .filter(|id| existing_set.contains(id.as_str()))
        .cloned()
        .collect();
    let named_set: HashSet<String> = named.iter().cloned().collect();
    let mut named_iter = named.into_iter();
    let mut merged: Vec<String> = existing
        .iter()
        .map(|id| {
            if named_set.contains(id) {
                named_iter
                    .next()
                    .expect("named ids are drawn from existing")
            } else {
                id.clone()
            }
        })
        .collect();
    merged.extend(
        submitted
            .into_iter()
            .filter(|id| !existing_set.contains(id.as_str())),
    );
    merged
}

/// Project a global title map or order array down to `visible` ids only.
#[must_use]
pub fn project_sync_value(
    state: &str,
    value: serde_json::Value,
    visible: &HashSet<String>,
) -> serde_json::Value {
    match state {
        "title" => {
            let Some(map) = value.as_object() else {
                return value;
            };
            serde_json::Value::Object(
                map.iter()
                    .filter(|(id, _)| visible.contains(id.as_str()))
                    .map(|(id, title)| (id.clone(), title.clone()))
                    .collect(),
            )
        }
        "order" => {
            let Some(list) = value.as_array() else {
                return value;
            };
            serde_json::Value::Array(
                list.iter()
                    .filter(|id| {
                        id.as_str()
                            .is_some_and(|session_id| visible.contains(session_id))
                    })
                    .cloned()
                    .collect(),
            )
        }
        _ => value,
    }
}

/// Stable reorder of `items` to match `order` (by each item's id). Items whose
/// id isn't in `order` sort to the end keeping their prior relative order, so a
/// partial / stale order never drops or duplicates anything.
fn sort_by_id_order<T>(items: &mut [T], order: &[String], id_of: impl Fn(&T) -> &str) {
    items.sort_by_key(|item| {
        order
            .iter()
            .position(|o| o == id_of(item))
            .unwrap_or(usize::MAX)
    });
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod managed_call_tests {
    use super::*;

    fn child(id: &str) -> SessionRegistration {
        let binding = crate::execution_environment::ManagedChildV1 {
            schema: 1,
            phase: "managed_child".into(),
            session_id: id.into(),
            parent_session_id: "parent".into(),
            machine_id: "hawk".into(),
            workspace_id: "project".into(),
            cwd: "/owned/managed/child/workspace".into(),
            profile: cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
        };
        SessionRegistration {
            id: id.to_owned(),
            provider: "codex".to_owned(),
            provider_version: "3.4.0".to_owned(),
            provider_generation_digest: "generation".to_owned(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "hawk".to_owned(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: Some(crate::execution_environment::ExecutionBinding::from_record(
                serde_json::to_value(binding).unwrap(),
            )),
            cwd: "/owned/managed/child/workspace".to_owned(),
            title: "child".to_owned(),
            origin: SessionOrigin::Api,
            system: false,
            owner_user_id: None,
            owner_username: None,
        }
    }

    fn update(kind: &str, text: &str) -> Event {
        Event::Update {
            update: serde_json::json!({"sessionUpdate": kind, "content": {"type":"text","text": text}}),
        }
    }

    #[test]
    fn client_entry_points_cannot_prompt_a_managed_child() {
        let hub = Hub::new();
        hub.create_session(child("child"));
        hub.submit(
            "child",
            "injected".into(),
            Vec::new(),
            Some("client-1".into()),
        );
        assert!(!hub.force_submit("child", "forced".into(), Vec::new(), None, true));
        hub.add_draft("child", "draft".into(), Vec::new(), None);
        hub.activate_all_drafts("child");
        let turn = hub.managed_turn("child", 0).unwrap();
        assert!(!turn.prompted);
        hub.submit_managed_prompt("child", "call-1", "review".into());
        assert!(hub.managed_turn("child", 0).unwrap().prompted);
    }

    #[test]
    fn managed_turn_returns_the_final_message_after_its_last_tool_update() {
        let hub = Hub::new();
        hub.create_session(child("child"));
        hub.push("child", update("agent_message_chunk", "previous call"));
        hub.push(
            "child",
            Event::TurnEnd {
                stop_reason: "end_turn".into(),
            },
        );
        let cursor = hub.last_event_seq("child").unwrap();
        hub.push("child", update("user_message_chunk", "review"));
        hub.push("child", update("agent_message_chunk", "I will inspect"));
        hub.push(
            "child",
            Event::Update {
                update: serde_json::json!({"sessionUpdate":"tool_call","toolCallId":"t"}),
            },
        );
        hub.push("child", update("agent_message_chunk", "{\"verdict\":"));
        hub.push("child", update("agent_message_chunk", "\"approve\"}"));
        let running = hub.managed_turn("child", cursor).unwrap();
        assert!(running.prompted && running.stop_reason.is_none());
        hub.push(
            "child",
            Event::TurnEnd {
                stop_reason: "end_turn".into(),
            },
        );
        let ended = hub.managed_turn("child", cursor).unwrap();
        assert_eq!(ended.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(ended.final_text, "{\"verdict\":\"approve\"}");
        assert!(hub.managed_turn("missing", 0).is_none());
    }
}

#[cfg(test)]
mod session_owner_tests {
    use super::*;

    fn registration(id: &str) -> SessionRegistration {
        SessionRegistration {
            id: id.to_owned(),
            provider: "codex".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "local".to_owned(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: "/tmp".to_owned(),
            title: "owner stamp".to_owned(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        }
    }

    #[test]
    fn new_session_stamps_optional_owner_and_broadcasts_it() {
        let hub = Hub::new();
        let mut registration = registration("sess-owned");
        registration.owner_user_id = Some("0123456789abcdef0123456789abcdef".to_owned());
        registration.owner_username = Some("draven".to_owned());
        hub.create_session(registration);

        let meta = hub
            .session_list()
            .into_iter()
            .find(|session| session.id == "sess-owned")
            .expect("created session");
        assert_eq!(
            meta.owner_user_id.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert_eq!(meta.owner_username.as_deref(), Some("draven"));
    }

    #[test]
    fn unauthenticated_create_leaves_owner_null() {
        let hub = Hub::new();
        hub.create_session(registration("sess-shared"));

        let meta = hub
            .session_list()
            .into_iter()
            .find(|session| session.id == "sess-shared")
            .expect("created session");
        assert!(meta.owner_user_id.is_none());
        assert!(meta.owner_username.is_none());
        assert!(!hub.owned_by_product_user("sess-shared", "anyone"));
        assert_eq!(hub.session_owner_user_id("sess-shared"), None);
    }
}

#[cfg(test)]
mod session_order_merge_tests {
    use super::{Hub, SessionOrigin, merge_session_order, project_sync_value};
    use std::collections::HashSet;

    #[test]
    fn stale_omit_keeps_the_missing_visible_id() {
        let merged = merge_session_order(
            &["A".to_owned(), "B".to_owned(), "C".to_owned()],
            &["C".to_owned(), "A".to_owned()],
        );
        assert_eq!(merged, vec!["C".to_owned(), "B".to_owned(), "A".to_owned()]);
    }

    #[test]
    fn hidden_id_stays_in_its_held_slot() {
        let merged = merge_session_order(
            &["A".to_owned(), "hidden".to_owned(), "B".to_owned()],
            &["B".to_owned(), "A".to_owned()],
        );
        assert_eq!(
            merged,
            vec!["B".to_owned(), "hidden".to_owned(), "A".to_owned()]
        );
    }

    #[test]
    fn hub_reorder_never_drops_omitted_ids() {
        let hub = Hub::new();
        for id in ["A", "B", "C"] {
            hub.create_local_session(
                id.to_owned(),
                "codex".to_owned(),
                "/tmp".to_owned(),
                id.to_owned(),
                SessionOrigin::Web,
                false,
            );
        }
        hub.reorder_sessions(&["C".to_owned(), "A".to_owned()]);
        let order: Vec<String> = hub.session_list().into_iter().map(|meta| meta.id).collect();
        assert_eq!(order, vec!["C".to_owned(), "B".to_owned(), "A".to_owned()]);
    }

    #[test]
    fn project_sync_value_drops_hidden_title_and_order_ids() {
        let visible = HashSet::from(["A".to_owned(), "C".to_owned()]);
        let titles = project_sync_value(
            "title",
            serde_json::json!({ "A": "one", "B": "hidden", "C": "three" }),
            &visible,
        );
        assert_eq!(titles, serde_json::json!({ "A": "one", "C": "three" }));
        let order = project_sync_value("order", serde_json::json!(["A", "B", "C"]), &visible);
        assert_eq!(order, serde_json::json!(["A", "C"]));
    }
}

#[cfg(test)]
mod config_preference_tests {
    use super::*;

    #[test]
    fn new_codex_sessions_start_with_sol_6_1_medium_preferences() {
        let hub = Hub::new();
        hub.create_local_session(
            "codex-session".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );

        assert_eq!(
            hub.config_preferences("codex-session"),
            Some(serde_json::json!({
                "model": "gpt-6.1-sol",
                "reasoning_effort": "medium",
            }))
        );
    }

    #[test]
    fn new_grok_sessions_start_with_high_reasoning_and_full_access_without_pinning_a_model() {
        let hub = Hub::new();
        hub.create_local_session(
            "grok-session".to_owned(),
            "grok".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );

        assert_eq!(
            hub.config_preferences("grok-session"),
            Some(serde_json::json!({
                "permission_mode": "always-approve",
                "reasoning_effort": "high",
            }))
        );
    }

    #[test]
    fn new_claude_deepseek_sessions_start_with_flash_max_default_agent_preferences() {
        let hub = Hub::new();
        hub.create_local_session(
            "claude-deepseek-session".to_owned(),
            "claude-deepseek".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );

        assert_eq!(
            hub.config_preferences("claude-deepseek-session"),
            Some(serde_json::json!({
                "model": "deepseek-flash[1m]",
                "deepseek_context": "830k",
                "deepseek_cache_protection": true,
                "effort": "max",
                "agent": "default",
            }))
        );
    }

    #[test]
    fn new_codex_deepseek_sessions_start_with_flash_default_collaboration_max_preferences() {
        let hub = Hub::new();
        hub.create_local_session(
            "codex-deepseek-session".to_owned(),
            "codex-deepseek".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );

        assert_eq!(
            hub.config_preferences("codex-deepseek-session"),
            Some(serde_json::json!({
                "model": "deepseek-flash",
                "deepseek_context": "680k",
                "deepseek_cache_protection": true,
                "collaboration_mode": "default",
                "reasoning_effort": "max",
            }))
        );
    }

    #[test]
    fn selecting_a_config_value_updates_the_shared_snapshot() {
        let hub = Hub::new();
        hub.create_local_session(
            "codex-session".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.set_config_options(
            "codex-session",
            serde_json::json!([{
                "id": "model",
                "currentValue": "gpt-5.6-sol",
                "options": [{"value": "gpt-5.6-sol"}, {"value": "gpt-5.6-luna"}],
            }]),
        );

        hub.set_config_preference(
            "codex-session",
            "model".to_owned(),
            serde_json::json!("gpt-5.6-luna"),
        )
        .expect("valid config preference");

        assert_eq!(
            hub.config_preferences("codex-session")
                .and_then(|value| value.get("model").cloned()),
            Some(serde_json::json!("gpt-5.6-luna"))
        );
        assert_eq!(
            hub.config_options("codex-session")
                .and_then(|value| value[0].get("currentValue").cloned()),
            Some(serde_json::json!("gpt-5.6-luna"))
        );
    }

    #[test]
    fn a_preset_does_not_flip_back_on_the_agents_answer_to_its_first_option() {
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "claude-code".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        let snapshot = |model: &str, effort: &str| {
            serde_json::json!([
                {"id": "model", "currentValue": model,
                 "options": [{"value": "opus"}, {"value": "sonnet"}]},
                {"id": "effort", "currentValue": effort,
                 "options": [{"value": "default"}, {"value": "high"}]},
            ])
        };
        let current = |id: usize| {
            hub.config_options("s")
                .and_then(|options| options[id].get("currentValue").cloned())
        };
        hub.set_config_options("s", snapshot("opus", "default"));
        // A preset sends both options before the agent answers either.
        for (id, value) in [("model", "sonnet"), ("effort", "high")] {
            hub.set_config_preference("s", id.to_owned(), serde_json::json!(value))
                .expect("preference");
        }
        // The answer to the model change still carries the old effort.
        hub.set_config_options("s", snapshot("sonnet", "default"));
        assert_eq!(current(1), Some(serde_json::json!("high")));
        hub.set_config_options("s", snapshot("sonnet", "high"));
        assert_eq!(current(1), Some(serde_json::json!("high")));
        // Settled: a later agent-side change is authoritative again.
        hub.set_config_options("s", snapshot("sonnet", "default"));
        assert_eq!(current(1), Some(serde_json::json!("default")));
        assert_eq!(current(0), Some(serde_json::json!("sonnet")));
    }

    #[test]
    fn rapid_preset_taps_never_show_a_stale_intermediate_selection() {
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "claude-code".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        let snapshot = |model: &str, effort: &str| {
            serde_json::json!([
                {"id": "model", "category": "model", "currentValue": model,
                 "options": [{"value": "opus"}, {"value": "sonnet"}]},
                {"id": "effort", "category": "thought_level", "currentValue": effort,
                 "options": [{"value": "default"}, {"value": "high"}]},
            ])
        };
        let shown = || {
            let options = hub.config_options("s").expect("options");
            (
                options[0]["currentValue"].clone(),
                options[1]["currentValue"].clone(),
            )
        };
        let target = (serde_json::json!("opus"), serde_json::json!("default"));
        hub.set_config_options("s", snapshot("opus", "default"));
        // Tap Sonnet·High, then immediately back to Opus·Default.
        for (id, value) in [
            ("model", "sonnet"),
            ("effort", "high"),
            ("model", "opus"),
            ("effort", "default"),
        ] {
            hub.set_config_preference("s", id.to_owned(), serde_json::json!(value))
                .expect("preference");
        }
        // The agent answers each command in order. The first answer matches
        // the final effort by coincidence; it must not settle it alone.
        for (model, effort) in [
            ("sonnet", "default"),
            ("sonnet", "high"),
            ("opus", "high"),
            ("opus", "default"),
        ] {
            hub.set_config_options("s", snapshot(model, effort));
            assert_eq!(shown(), target, "after agent answer ({model}, {effort})");
        }
        hub.set_config_options("s", snapshot("opus", "high"));
        assert_eq!(shown().1, serde_json::json!("high"), "settled values yield");
    }

    #[test]
    fn authoritative_options_replace_a_retired_persisted_value() {
        let health = Arc::new(PersistenceHealth::default());
        let (sink, mut rx) = StoreSink::channel(16, health);
        let hub = Hub::with_store(Some(sink));
        hub.create_local_session(
            "codex-session".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.set_config_preference(
            "codex-session",
            "model".to_owned(),
            serde_json::json!("gpt-5.3-codex-spark"),
        )
        .expect("model preference");
        hub.set_config_preference(
            "codex-session",
            "reasoning_effort".to_owned(),
            serde_json::json!("max"),
        )
        .expect("reasoning preference");
        while rx.try_recv().is_ok() {}

        hub.set_config_options(
            "codex-session",
            serde_json::json!([
                {
                    "id": "model",
                    "currentValue": "gpt-5.3-codex-spark",
                    "options": [{"value": "gpt-5.3-codex-spark"}],
                },
                {
                    "id": "reasoning_effort",
                    "currentValue": "low",
                    "options": [
                        {"value": "low"},
                        {"value": "medium"},
                        {"value": "high"},
                        {"value": "xhigh"},
                    ],
                },
            ]),
        );

        assert_eq!(
            hub.config_preferences("codex-session"),
            Some(serde_json::json!({
                "model": "gpt-5.3-codex-spark",
                "reasoning_effort": "low",
            }))
        );
        assert_eq!(
            hub.config_options("codex-session")
                .and_then(|value| value[1].get("currentValue").cloned()),
            Some(serde_json::json!("low"))
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(StoreWrite::UpdateConfigOptions { .. })
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(StoreWrite::UpdateConfigPreferences { preferences, .. })
                if preferences["reasoning_effort"] == serde_json::json!("low")
        ));
    }

    #[test]
    fn previous_model_snapshot_does_not_retire_a_pending_presets_reasoning() {
        let mut preferences = serde_json::json!({"model": "astra", "reasoning_effort": "max"});
        let mut options = serde_json::json!([
            {"id": "model", "category": "model", "currentValue": "spark",
             "options": [{"value": "spark"}, {"value": "astra"}]},
            {"id": "reasoning_effort", "category": "thought_level", "currentValue": "low",
             "options": [{"value": "low"}, {"value": "medium"}]}
        ]);
        assert!(!reconcile_config_preferences(
            Some(&options),
            &mut preferences
        ));
        assert_eq!(preferences["reasoning_effort"], "max");
        options[0]["currentValue"] = serde_json::json!("astra");
        options[1]["options"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"value": "max"}));
        assert!(!reconcile_config_preferences(
            Some(&options),
            &mut preferences
        ));
        assert_eq!(preferences["reasoning_effort"], "max");
        // Once the selected model itself retires the effort, normal repair
        // still applies; a genuinely removed value never survives indefinitely.
        options[1]["options"].as_array_mut().unwrap().pop();
        assert!(reconcile_config_preferences(
            Some(&options),
            &mut preferences
        ));
        assert_eq!(preferences["reasoning_effort"], "low");
    }

    #[test]
    fn deepseek_context_option_is_projected_after_the_model() {
        let hub = Hub::new();
        hub.create_local_session(
            "deepseek-session".to_owned(),
            "codex-deepseek".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.set_config_options(
            "deepseek-session",
            serde_json::json!([
                {"id": "model", "currentValue": "deepseek-v4-flash", "options": []},
                {"id": "reasoning_effort", "currentValue": "max", "options": []},
            ]),
        );

        let options = hub.config_options("deepseek-session").unwrap();
        assert_eq!(options[0]["id"], "model");
        assert_eq!(options[1]["id"], "deepseek_context");
        assert_eq!(options[1]["currentValue"], "680k");
        assert_eq!(options[2]["id"], "deepseek_cache_protection");
        assert_eq!(options[2]["currentValue"], true);
        assert_eq!(options[3]["id"], "reasoning_effort");
    }
}

#[cfg(test)]
mod runtime_reconciliation_tests {
    use super::*;

    #[test]
    fn restore_rehydrates_folder_placements_into_the_live_tree() {
        let hub = Hub::new();
        hub.restore_session_folders(vec![crate::session_folders::SessionFolder {
            id: "f-a".to_owned(),
            owner_user_id: None,
            name: "A".to_owned(),
            parent: None,
            position: 0,
            project: None,
        }]);
        let mut filed = restored_busy("filed");
        filed.folder_id = Some("f-a".to_owned());
        let mut loose = restored_busy("loose");
        loose.folder_id = Some("f-gone".to_owned());
        hub.restore_reconciling_runtime(vec![filed, loose]);
        let value = hub.sync_value("folders");
        assert_eq!(value["folders"][0]["id"], "f-a");
        assert_eq!(value["placement"]["filed"], "f-a");
        // A placement into a vanished folder degrades to the root.
        assert!(value["placement"].get("loose").is_none());
    }

    fn restored_busy(id: &str) -> RestoredSession {
        RestoredSession {
            meta: SessionMeta {
                id: id.to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                machine_id: "hawk".to_owned(),
                workspace_id: None,
                workspace_name: None,
                workspace_source_path: None,
                execution_binding: None,
                cwd: "/tmp".to_owned(),
                title: "test".to_owned(),
                status: Status::Busy,
                origin: SessionOrigin::Web,
                agent_session_id: Some("agent-1".to_owned()),
                paused: false,
                closing: false,
                system: false,
                context_used: 0,
                context_size: 0,
                usage: None,
                background_tasks: 0,
                provider_update: None,
                provider_update_available: None,
                next_schedule_ms: None,
                owner_user_id: None,
                owner_username: None,
            },
            log: Vec::new(),
            event_count: 0,
            reached_start: true,
            next_seq: 0,
            queue: Vec::new(),
            drafts: Vec::new(),
            config_options: None,
            config_preferences: serde_json::json!({}),
            mobile_review_state: serde_json::Value::Null,
            folder_id: None,
        }
    }

    pub(super) fn worker_snapshot(session_id: &str, worker_epoch: &str) -> WorkerSnapshot {
        WorkerSnapshot {
            session_id: session_id.to_owned(),
            worker_epoch: worker_epoch.to_owned(),
            generation: "gen-1".to_owned(),
            executable: None,
            launch: None,
            state: WorkerState::Busy,
            agent_session_id: Some("agent-1".to_owned()),
            native_thread_materialized: None,
            current_turn_id: Some("turn-1".to_owned()),
            last_runtime_seq: 7,
            pending_permissions: Vec::new(),
            config_options: None,
            context_used: None,
            context_size: None,
            pending_prompt_count: 0,
            drain_requested: false,
            exit_detail: None,
            background_tasks: None,
            incarnation: None,
        }
    }

    fn pending(id: &str, text: &str, cmid: &str) -> QueuedMessage {
        QueuedMessage {
            id: id.to_owned(),
            text: text.to_owned(),
            content: Vec::new(),
            cmid: Some(cmid.to_owned()),
            schedule: None,
        }
    }

    #[test]
    fn placeholder_cannot_settle_restored_busy_turn() {
        let hub = Hub::new();
        hub.restore_reconciling_runtime(vec![restored_busy("session-1")]);

        assert_eq!(hub.status("session-1"), Some(Status::Busy));
        assert!(!hub.accept_runtime_snapshot(&worker_snapshot("session-1", "broker-session-1")));
        assert_eq!(hub.status("session-1"), Some(Status::Busy));

        assert_eq!(
            hub.finalize_runtime_reconciliation(),
            vec!["session-1".to_owned()]
        );
        assert_eq!(hub.status("session-1"), Some(Status::Interrupted));
        assert!(hub.finalize_runtime_reconciliation().is_empty());
    }

    #[test]
    fn connected_worker_adopts_restored_busy_turn() {
        let hub = Hub::new();
        hub.restore_reconciling_runtime(vec![restored_busy("session-2")]);

        assert!(hub.accept_runtime_snapshot(&worker_snapshot("session-2", "worker-epoch-2")));
        assert!(hub.finalize_runtime_reconciliation().is_empty());
        assert_eq!(hub.status("session-2"), Some(Status::Busy));
    }

    #[test]
    fn idle_worker_cannot_silently_settle_a_restored_busy_turn() {
        let hub = Hub::new();
        hub.restore_reconciling_runtime(vec![restored_busy("session-idle")]);
        let mut idle = worker_snapshot("session-idle", "worker-epoch-idle");
        idle.state = WorkerState::Running;
        idle.current_turn_id = None;

        assert!(hub.accept_runtime_snapshot(&idle));
        assert_eq!(hub.status("session-idle"), Some(Status::Interrupted));
        assert!(!hub.session_has_in_flight_prompt("session-idle"));
        let (log, _) = hub.snapshot("session-idle").expect("snapshot");
        assert!(log.iter().any(|envelope| matches!(
            envelope.event,
            Event::Lifecycle {
                status: Status::Interrupted,
                ..
            }
        )));
        hub.project_runtime_status("session-idle", Status::Running, None);
        assert_eq!(hub.status("session-idle"), Some(Status::Interrupted));
        assert!(hub.finalize_runtime_reconciliation().is_empty());
    }

    #[test]
    fn restore_heals_a_preference_retired_by_the_persisted_option_snapshot() {
        let health = Arc::new(PersistenceHealth::default());
        let (sink, mut rx) = StoreSink::channel(4, health);
        let hub = Hub::with_store(Some(sink));
        let mut restored = restored_busy("session-stale-config");
        restored.config_preferences = serde_json::json!({
            "model": "gpt-5.3-codex-spark",
            "reasoning_effort": "max",
        });
        restored.config_options = Some(serde_json::json!([
            {
                "id": "model",
                "currentValue": "gpt-5.3-codex-spark",
                "options": [{"value": "gpt-5.3-codex-spark"}],
            },
            {
                "id": "reasoning_effort",
                "currentValue": "low",
                "options": [
                    {"value": "low"},
                    {"value": "medium"},
                    {"value": "high"},
                    {"value": "xhigh"},
                ],
            },
        ]));

        hub.restore_reconciling_runtime(vec![restored]);

        assert_eq!(
            hub.config_preferences("session-stale-config")
                .and_then(|value| value.get("reasoning_effort").cloned()),
            Some(serde_json::json!("low"))
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(StoreWrite::UpdateConfigPreferences {
                session_id,
                preferences,
            }) if session_id == "session-stale-config"
                && preferences["reasoning_effort"] == serde_json::json!("low")
        ));
    }

    #[test]
    fn immediate_restore_does_not_treat_broker_placeholder_as_live_owner() {
        let hub = Hub::new();
        hub.restore_with_workers(
            vec![restored_busy("session-3")],
            &[worker_snapshot("session-3", "broker-session-3")],
        );

        assert_eq!(hub.status("session-3"), Some(Status::Interrupted));
    }

    #[test]
    fn restore_discards_retired_continuations_but_keeps_user_work() {
        let hub = Hub::new();
        let mut restored = restored_busy("session-4");
        restored.queue = vec![
            pending("q1", "legacy queue continuation", "__cont__old"),
            pending("q2", "user queued message", "user-cmid"),
        ];
        restored.drafts = vec![
            pending("q3", "legacy draft continuation", "__cont__draft"),
            pending("q4", "user draft", "draft-cmid"),
        ];

        hub.restore_with_workers(vec![restored], &[]);

        assert_eq!(hub.status("session-4"), Some(Status::Interrupted));
        let Some(Outbound::SyncPatch { value, .. }) = hub.queue_resync("session-4") else {
            panic!("queue resync missing");
        };
        assert_eq!(value["queue"].as_array().unwrap().len(), 1);
        assert_eq!(value["queue"][0]["text"], "user queued message");
        assert_eq!(value["drafts"].as_array().unwrap().len(), 1);
        assert_eq!(value["drafts"][0]["text"], "user draft");
    }
}

#[cfg(test)]
mod core_tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn watchdog_revision_does_not_claim_the_replacement_turn() {
        let hub = hub_with_session("status-cas");
        hub.set_status("status-cas", Status::Busy, None);
        let cancelled_turn = hub.status_revision("status-cas").expect("busy revision");

        // The cancelled turn ends and the force-pushed replacement starts. Its
        // status is also Busy, but it is not the turn the watchdog was armed for.
        hub.set_status("status-cas", Status::Running, None);
        hub.set_status("status-cas", Status::Busy, None);

        assert!(!hub.set_status_if_revision(
            "status-cas",
            Some(cancelled_turn),
            Status::Interrupted,
            Some("watchdog".to_owned()),
        ));
        assert_eq!(hub.status("status-cas"), Some(Status::Busy));
    }

    #[test]
    fn latest_crash_detail_survives_a_detail_less_runtime_projection() {
        let hub = hub_with_session("terminal-crash");
        hub.set_status(
            "terminal-crash",
            Status::Crashed,
            Some("provider retired this login flow".to_owned()),
        );
        hub.set_status("terminal-crash", Status::Crashed, None);
        assert_eq!(
            hub.latest_crash_detail("terminal-crash").as_deref(),
            Some("provider retired this login flow")
        );

        hub.set_status("terminal-crash", Status::Starting, None);
        hub.set_status("terminal-crash", Status::Crashed, None);
        assert_eq!(hub.latest_crash_detail("terminal-crash"), None);
    }

    #[test]
    fn watchdog_revision_ignores_duplicate_busy_snapshots_on_a_stuck_turn() {
        let hub = hub_with_session("status-stuck");
        hub.set_status("status-stuck", Status::Busy, None);
        let stuck_turn = hub.status_revision("status-stuck").expect("busy revision");
        hub.set_status("status-stuck", Status::Busy, None);

        assert!(hub.set_status_if_revision(
            "status-stuck",
            Some(stuck_turn),
            Status::Interrupted,
            None,
        ));
        assert_eq!(hub.status("status-stuck"), Some(Status::Interrupted));
    }

    fn hub_with_session(id: &str) -> Hub {
        let hub = Hub::new();
        hub.create_local_session(
            id.to_owned(),
            "claude-code".to_owned(),
            "/tmp".to_owned(),
            "t".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub
    }

    #[test]
    fn mobile_review_sync_is_session_scoped_and_idempotent() {
        let hub = hub_with_session("mobile");
        hub.sync_apply(
            "mobile-review:mobile",
            "m1".to_owned(),
            "open",
            &serde_json::json!({"path": "strategies/README.md"}),
        )
        .unwrap();
        hub.sync_apply(
            "mobile-review:mobile",
            "m1".to_owned(),
            "open",
            &serde_json::json!({"path": "ignored/by-retry.rs"}),
        )
        .unwrap();
        hub.sync_apply(
            "mobile-review:mobile",
            "m2".to_owned(),
            "setPinned",
            &serde_json::json!({"path": "strategies/README.md", "pinned": true}),
        )
        .unwrap();
        hub.sync_apply(
            "mobile-review:mobile",
            "m3".to_owned(),
            "markReviewed",
            &serde_json::json!({"key": "combined:strategies/README.md", "revision": "abc123"}),
        )
        .unwrap();
        hub.sync_apply(
            "mobile-review:mobile",
            "m4".to_owned(),
            "setPosition",
            &serde_json::json!({
                "path": "strategies/README.md",
                "line": 47,
                "revision": "abc123"
            }),
        )
        .unwrap();

        let value = hub.sync_value("mobile-review:mobile");
        assert_eq!(value["mode"], "files");
        assert_eq!(value["active"], "strategies/README.md");
        assert_eq!(value["tabs"].as_array().unwrap().len(), 1);
        assert_eq!(value["tabs"][0]["pinned"], true);
        assert_eq!(value["progress"]["combined:strategies/README.md"], "abc123");
        assert_eq!(value["positions"]["strategies/README.md"]["line"], 47);
        assert_eq!(
            value["positions"]["strategies/README.md"]["revision"],
            "abc123"
        );
    }

    #[test]
    fn remote_review_binding_survives_local_navigation_and_is_validated() {
        let hub = hub_with_session("remote-review");
        let state = "mobile-review:remote-review";
        let binding = serde_json::json!({"pluginId":"fixture", "view":"pull-requests", "host":"github.com",
            "owner":"owner", "repository":"repo", "repositoryId":"123", "number":"12"});
        hub.sync_apply(
            state,
            "bind".into(),
            "setRemoteReview",
            &serde_json::json!({"binding":binding}),
        )
        .unwrap();
        hub.sync_apply(
            state,
            "select".into(),
            "selectRemoteReview",
            &serde_json::json!({"selected":true}),
        )
        .unwrap();
        hub.sync_apply(
            state,
            "open".into(),
            "open",
            &serde_json::json!({"path":"README.md"}),
        )
        .unwrap();
        hub.sync_apply(
            state,
            "close".into(),
            "close",
            &serde_json::json!({"path":"README.md"}),
        )
        .unwrap();
        let value = hub.sync_value(state);
        assert_eq!(value["remote_review"], binding);
        assert_eq!(value["remote_selected"], true);
        let restored = super::MobileReviewState::from_stored(value);
        assert_eq!(restored.remote_review.unwrap().number, "12");
        let mut bad = binding.clone();
        bad["host"] = serde_json::json!("token@github.com");
        assert!(
            hub.sync_apply(
                state,
                "invalid".into(),
                "setRemoteReview",
                &serde_json::json!({"binding":bad})
            )
            .is_err()
        );
        assert_eq!(hub.sync_value(state)["remote_review"], binding);
        hub.sync_apply(
            state,
            "unbind".into(),
            "setRemoteReview",
            &serde_json::json!({"binding":null}),
        )
        .unwrap();
        assert!(hub.sync_value(state).get("remote_review").is_none());
    }

    #[test]
    fn folders_sync_is_arbitrated_idempotent_and_seeded_on_resync() {
        let hub = hub_with_session("filed");
        hub.sync_apply(
            "folders",
            "f1".to_owned(),
            "create",
            &serde_json::json!({"id": "f-a", "name": "Cowboy", "project": "cowboy"}),
        )
        .unwrap();
        // A retried delivery is a no-op, not a second folder.
        hub.sync_apply(
            "folders",
            "f1".to_owned(),
            "create",
            &serde_json::json!({"id": "f-a", "name": "Retry"}),
        )
        .unwrap();
        hub.sync_apply(
            "folders",
            "f2".to_owned(),
            "place",
            &serde_json::json!({"session_ids": ["filed"], "folder": "f-a"}),
        )
        .unwrap();
        let value = hub.sync_value("folders");
        assert_eq!(value["folders"].as_array().unwrap().len(), 1);
        assert_eq!(value["folders"][0]["name"], "Cowboy");
        assert_eq!(value["folders"][0]["project"], "cowboy");
        assert_eq!(value["placement"]["filed"], "f-a");

        // A rejected mutation leaves its id unconsumed, so the corrected retry
        // under the same id still applies.
        assert_eq!(
            hub.sync_apply(
                "folders",
                "f3".to_owned(),
                "create",
                &serde_json::json!({"id": "f-b", "name": "   "}),
            )
            .unwrap_err(),
            "folder name cannot be empty"
        );
        hub.sync_apply(
            "folders",
            "f3".to_owned(),
            "create",
            &serde_json::json!({"id": "f-b", "name": "B", "parent": "f-a"}),
        )
        .unwrap();
        assert_eq!(hub.sync_value("folders")["folders"][1]["parent"], "f-a");
        assert_eq!(
            hub.sync_apply(
                "folders",
                "f4".to_owned(),
                "explode",
                &serde_json::json!({})
            )
            .unwrap_err(),
            "unknown folders mutation explode"
        );

        assert!(hub.sync_resync().iter().any(|message| matches!(
            message,
            super::Outbound::SyncPatch { state, resync: true, .. } if state == "folders"
        )));
        assert!(hub.delete_session("filed"));
        assert!(
            hub.sync_value("folders")["placement"]
                .get("filed")
                .is_none()
        );
    }

    #[test]
    fn mobile_review_sync_rejects_escaping_paths() {
        let hub = hub_with_session("mobile-invalid");
        let error = hub
            .sync_apply(
                "mobile-review:mobile-invalid",
                "m1".to_owned(),
                "open",
                &serde_json::json!({"path": "../secret"}),
            )
            .unwrap_err();
        assert_eq!(error, "invalid mobile review path");
    }

    #[test]
    fn queued_prompt_can_be_cancelled_by_exact_cmid() {
        let hub = hub_with_session("s");
        hub.submit(
            "s",
            "from ACP".to_owned(),
            Vec::new(),
            Some("acp-1".to_owned()),
        );
        assert_eq!(hub.session_info("s").unwrap().queue_count, 1);
        assert!(!hub.remove_queued_by_cmid("s", "another-client"));
        assert!(hub.remove_queued_by_cmid("s", "acp-1"));
        assert_eq!(hub.session_info("s").unwrap().queue_count, 0);
        assert!(!hub.remove_queued_by_cmid("s", "acp-1"));
    }

    #[test]
    fn renewed_queue_edit_hold_survives_replaced_socket_cleanup() {
        let hub = hub_with_session("edit-reload");
        let replaced_epoch = hub.set_queue_editing("edit-reload", Some("queued-1".to_owned()));
        let replacement_epoch = hub.set_queue_editing("edit-reload", Some("queued-1".to_owned()));

        assert_ne!(replaced_epoch, replacement_epoch);
        assert!(!hub.release_queue_editing_if_epoch("edit-reload", "queued-1", replaced_epoch,));
        assert_eq!(
            hub.inner
                .sessions
                .lock()
                .get("edit-reload")
                .and_then(|session| session.editing.as_deref()),
            Some("queued-1"),
        );
        assert!(hub.release_queue_editing_if_epoch("edit-reload", "queued-1", replacement_epoch,));
    }

    // The queue texts as clients would see them (via the resync patch).
    fn queue_texts(hub: &Hub, id: &str) -> Vec<String> {
        match hub.queue_resync(id) {
            Some(Outbound::SyncPatch { value, .. }) => value["queue"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|m| m["text"].as_str().unwrap_or_default().to_owned())
                        .collect()
                })
                .unwrap_or_default(),
            _ => vec![],
        }
    }

    // A prompt salvaged from a crashed cold-start lands back on the queue (so it's
    // never lost) and is idempotent on cmid (a racing re-revive can't double it).
    #[test]
    fn requeue_prompt_restores_and_dedupes() {
        let hub = hub_with_session("r1");
        assert!(queue_texts(&hub, "r1").is_empty());
        hub.requeue_prompt(
            "r1",
            "hello agent".to_owned(),
            vec![],
            Some("c1".to_owned()),
        );
        assert_eq!(queue_texts(&hub, "r1"), vec!["hello agent".to_owned()]);
        // Same cmid (the delivery raced a re-revive) → not double-queued.
        hub.requeue_prompt(
            "r1",
            "hello agent".to_owned(),
            vec![],
            Some("c1".to_owned()),
        );
        assert_eq!(
            queue_texts(&hub, "r1").len(),
            1,
            "same cmid must not double-queue"
        );
        // A different message DOES stack (front-inserted).
        hub.requeue_prompt("r1", "second".to_owned(), vec![], Some("c2".to_owned()));
        assert_eq!(
            queue_texts(&hub, "r1"),
            vec!["second".to_owned(), "hello agent".to_owned()]
        );
    }

    #[test]
    fn copy_undo_requires_the_original_unsent_row() {
        let hub = hub_with_session("undo-copy");
        let content = vec![serde_json::json!({"type":"text", "text":"original"})];
        hub.add_draft(
            "undo-copy",
            "original".into(),
            content.clone(),
            Some("copy-1".into()),
        );
        let id = hub.inner.sessions.lock()["undo-copy"].drafts[0].id.clone();
        assert!(!hub.remove_draft_if_unchanged("missing", "copy-1", "original", &content));
        assert!(!hub.remove_draft_if_unchanged("undo-copy", "copy-1", "original", &[]));
        hub.edit_draft("undo-copy", &id, "edited".into(), content.clone());
        assert!(!hub.remove_draft_if_unchanged("undo-copy", "copy-1", "original", &content));
        assert_eq!(hub.inner.sessions.lock()["undo-copy"].drafts.len(), 1);
        assert!(hub.remove_draft_if_unchanged("undo-copy", "copy-1", "edited", &content));
        assert!(!hub.remove_draft_if_unchanged("undo-copy", "copy-1", "edited", &content));
        hub.add_draft(
            "undo-copy",
            "original".into(),
            content.clone(),
            Some("copy-2".into()),
        );
        hub.inner
            .sessions
            .lock()
            .get_mut("undo-copy")
            .unwrap()
            .drafts[0]
            .schedule = Some(DraftSchedule {
            fire_at_ms: i64::MAX,
            delivery: Delivery::default(),
        });
        assert!(!hub.remove_draft_if_unchanged("undo-copy", "copy-2", "original", &content));
    }

    #[tokio::test]
    async fn activated_image_draft_keeps_its_echo_identity_and_replay_is_deduped() {
        let hub = hub_with_session("draft-send");
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("draft-send", Status::Running, None);
        let content = vec![
            serde_json::json!({"type": "image", "data": "c2hvdA==", "mimeType": "image/png"}),
            serde_json::json!({"type": "text", "text": "caption"}),
        ];
        hub.add_draft(
            "draft-send",
            "caption".to_owned(),
            content.clone(),
            Some("draft-cmid".to_owned()),
        );
        let id = hub.inner.sessions.lock()["draft-send"].drafts[0].id.clone();
        hub.activate_draft("draft-send", &id, Some("send-cmid".to_owned()));
        let dispatched = rx.recv().await.expect("draft dispatch");
        assert_eq!(dispatched.cmid.as_deref(), Some("send-cmid"));
        assert_eq!(dispatched.content, content);
        hub.activate_draft("draft-send", &id, Some("send-cmid".to_owned()));
        assert!(
            rx.try_recv().is_err(),
            "replayed activation must not dispatch twice"
        );
        for (index, block) in content.into_iter().enumerate() {
            hub.push_tagged(
                "draft-send",
                Event::Update {
                    update: serde_json::json!({"sessionUpdate": "user_message_chunk", "content": block}),
                },
                (index == 0).then(|| dispatched.cmid.clone()).flatten(),
            );
        }
        let (events, _) = hub.snapshot("draft-send").expect("transcript");
        let echo = events
            .iter()
            .find(|env| is_user_message_chunk(env))
            .expect("image echo");
        assert_eq!(echo.cmid.as_deref(), Some("send-cmid"));
        assert_eq!(
            crate::persistence::persisted_event_payload(echo).expect("persisted echo")["cmid"],
            "send-cmid"
        );
        let mut live = hub.subscribe();
        hub.submit(
            "draft-send",
            "caption".to_owned(),
            vec![],
            Some("send-cmid".to_owned()),
        );
        assert!(
            rx.try_recv().is_err(),
            "replayed submit must not dispatch twice"
        );
        let frame = live.try_recv().expect("delivery confirmation");
        assert!(
            matches!(frame.outbound(), Outbound::SyncPatch { confirmed, .. }
            if confirmed == &vec!["send-cmid".to_owned()])
        );
    }

    #[tokio::test]
    async fn bulk_draft_activation_preserves_ids_in_dispatch_and_queue() {
        let hub = hub_with_session("bulk-drafts");
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("bulk-drafts", Status::Running, None);
        for cmid in [Some("first"), Some("second"), None] {
            hub.add_draft(
                "bulk-drafts",
                "prompt".to_owned(),
                vec![],
                cmid.map(str::to_owned),
            );
        }
        hub.activate_all_drafts("bulk-drafts");
        assert_eq!(
            rx.recv().await.expect("first dispatch").cmid.as_deref(),
            Some("first")
        );
        let sessions = hub.inner.sessions.lock();
        let session = &sessions["bulk-drafts"];
        assert!(session.drafts.is_empty());
        assert_eq!(session.queue.len(), 2);
        assert_eq!(session.queue[0].cmid.as_deref(), Some("second"));
        assert_eq!(session.queue[1].cmid, None);
    }

    #[tokio::test]
    async fn legacy_draft_activation_keeps_the_source_identity_when_available() {
        for cmid in [Some("legacy-draft"), None] {
            let hub = hub_with_session("legacy-draft-send");
            let (tx, mut rx) = mpsc::channel(4);
            hub.set_dispatch_tx(tx);
            hub.set_status("legacy-draft-send", Status::Running, None);
            hub.add_draft(
                "legacy-draft-send",
                "prompt".to_owned(),
                vec![],
                cmid.map(str::to_owned),
            );
            let id = hub.inner.sessions.lock()["legacy-draft-send"].drafts[0]
                .id
                .clone();
            let request: Inbound = serde_json::from_value(serde_json::json!({
                "type": "activate_draft", "session_id": "legacy-draft-send", "id": id,
            }))
            .expect("legacy request");
            let Inbound::ActivateDraft {
                session_id,
                id,
                cmid: send_cmid,
            } = request
            else {
                panic!("expected draft activation");
            };
            assert!(send_cmid.is_none());
            hub.activate_draft(&session_id, &id, send_cmid);
            assert_eq!(
                rx.recv().await.expect("draft dispatch").cmid.as_deref(),
                cmid
            );
        }
    }

    // Dispatch admission prevents duplicate execution, but it is not proof
    // that a queued row or transcript exists to replace the browser's outbox.
    #[tokio::test]
    async fn replayed_submit_before_echo_stays_unconfirmed_and_is_not_rerun() {
        let hub = hub_with_session("replay");
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("replay", Status::Running, None);
        hub.submit("replay", "hello".to_owned(), vec![], Some("c-1".to_owned()));
        assert_eq!(rx.recv().await.expect("first dispatch").text, "hello");

        let mut live = hub.subscribe();
        hub.submit("replay", "hello".to_owned(), vec![], Some("c-1".to_owned()));
        assert!(!hub.force_submit(
            "replay",
            "hello".to_owned(),
            vec![],
            Some("c-1".to_owned()),
            true,
        ));
        assert!(
            rx.try_recv().is_err(),
            "a replayed submit must not dispatch again"
        );
        assert!(queue_texts(&hub, "replay").is_empty());
        assert!(
            live.try_recv().is_err(),
            "dispatch admission must not erase a prompt before its user echo"
        );

        hub.push_tagged(
            "replay",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": "hello"}
                }),
            },
            Some("c-1".to_owned()),
        );
        // A fresh reconnect can now retire the outbox: transcript replay owns
        // the prompt. The original dispatch remains the only execution.
        let mut live = hub.subscribe();
        hub.submit("replay", "hello".to_owned(), vec![], Some("c-1".to_owned()));
        assert!(rx.try_recv().is_err());
        let frame = live.try_recv().expect("echo-backed confirmation");
        let Outbound::SyncPatch {
            state, confirmed, ..
        } = frame.outbound()
        else {
            panic!("expected a queue confirmation");
        };
        assert_eq!(state, "queue:replay");
        assert_eq!(confirmed, &vec!["c-1".to_owned()]);

        hub.push(
            "replay",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "subsequent output"}
                }),
            },
        );
        {
            let mut sessions = hub.inner.sessions.lock();
            let s = sessions.get_mut("replay").unwrap();
            assert!(trim_hot_log(&mut s.log, &mut s.log_bytes, false, 1));
            assert!(!s.log.iter().any(is_user_message_chunk));
        }
        let mut live = hub.subscribe();
        hub.submit("replay", "hello".to_owned(), vec![], Some("c-1".to_owned()));
        assert!(rx.try_recv().is_err(), "evicted echoes must not run again");
        let frame = live.try_recv().expect("retained delivery evidence");
        assert!(
            matches!(frame.outbound(), Outbound::SyncPatch { confirmed, .. }
            if confirmed == &vec!["c-1".to_owned()])
        );
    }

    // After a Controller restart the dispatched-id window is empty, but the user
    // echo restored from storage still carries the cmid of the prompt it ran.
    #[tokio::test]
    async fn replayed_submit_is_recognised_from_the_persisted_echo() {
        let hub = hub_with_session("restored");
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("restored", Status::Running, None);
        let echo = || Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": {"type": "text", "text": "hello"},
            }),
        };
        hub.push_tagged("restored", echo(), Some("c-2".to_owned()));

        hub.submit(
            "restored",
            "hello".to_owned(),
            vec![],
            Some("c-2".to_owned()),
        );
        assert!(
            rx.try_recv().is_err(),
            "an echoed prompt must not run again"
        );
        assert!(queue_texts(&hub, "restored").is_empty());

        // Hub-synthesized ids are reusable on purpose and never deduped by the log.
        let synthesized = "cowboy-retry:restored:1".to_owned();
        hub.push_tagged("restored", echo(), Some(synthesized.clone()));
        hub.submit("restored", "again".to_owned(), vec![], Some(synthesized));
        assert_eq!(
            rx.recv().await.expect("synthesized id dispatches").text,
            "again"
        );
    }

    #[tokio::test]
    async fn replayed_image_submit_retains_payload_until_requeued() {
        let hub = hub_with_session("image-retry");
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("image-retry", Status::Running, None);
        let content = vec![serde_json::json!({
            "type": "image", "mimeType": "image/png", "data": "synthetic-image"
        })];
        hub.submit(
            "image-retry",
            "explain".into(),
            content.clone(),
            Some("image-1".into()),
        );
        let dispatched = rx.recv().await.unwrap();
        assert_eq!(dispatched.content, content);
        // A tagged diagnostic is not a user echo and must not acknowledge it.
        hub.push_tagged(
            "image-retry",
            Event::Update {
                update: serde_json::json!({"sessionUpdate": "config_option_update"}),
            },
            Some("image-1".into()),
        );
        let mut live = hub.subscribe();
        hub.submit(
            "image-retry",
            "explain".into(),
            content.clone(),
            Some("image-1".into()),
        );
        assert!(rx.try_recv().is_err());
        assert!(live.try_recv().is_err());

        // A failed transport returns the exact prompt to an authoritative row.
        hub.requeue_prompt(
            "image-retry",
            dispatched.text,
            dispatched.content,
            dispatched.cmid,
        );
        let mut live = hub.subscribe();
        hub.submit(
            "image-retry",
            "explain".into(),
            content.clone(),
            Some("image-1".into()),
        );
        let frame = live.try_recv().expect("queue-backed acknowledgement");
        let Outbound::SyncPatch {
            value, confirmed, ..
        } = frame.outbound()
        else {
            panic!("expected queue patch");
        };
        assert_eq!(confirmed, &vec!["image-1".to_owned()]);
        assert_eq!(value["queue"][0]["content"], serde_json::json!(content));
        assert!(rx.try_recv().is_err());
    }

    // A replayed draft is confirmed again so a client that missed the first
    // patch retires its outbox entry instead of waiting out a timeout.
    #[test]
    fn replayed_add_draft_is_confirmed_not_duplicated() {
        let hub = hub_with_session("draft-replay");
        hub.add_draft(
            "draft-replay",
            "later".to_owned(),
            vec![],
            Some("d-1".to_owned()),
        );
        let mut live = hub.subscribe();
        hub.add_draft(
            "draft-replay",
            "later".to_owned(),
            vec![],
            Some("d-1".to_owned()),
        );
        let frame = live.try_recv().expect("replayed draft confirmation");
        let Outbound::SyncPatch {
            state,
            value,
            confirmed,
            ..
        } = frame.outbound()
        else {
            panic!("expected a queue patch");
        };
        assert_eq!(state, "queue:draft-replay");
        assert_eq!(confirmed, &vec!["d-1".to_owned()]);
        assert_eq!(value["drafts"].as_array().unwrap().len(), 1);
    }

    // A duplicate sync delivery is confirmed again, so a client that missed the
    // original patch can retire its outbox entry.
    #[test]
    fn duplicate_sync_delivery_is_confirmed_again() {
        let hub = hub_with_session("dup");
        hub.sync_apply(
            "title",
            "m-1".to_owned(),
            "rename",
            &serde_json::json!({"session_id": "dup", "title": "First"}),
        )
        .unwrap();
        let mut live = hub.subscribe();
        hub.sync_apply(
            "title",
            "m-1".to_owned(),
            "rename",
            &serde_json::json!({"session_id": "dup", "title": "Ignored"}),
        )
        .unwrap();
        let frame = live.try_recv().expect("replayed delivery confirmation");
        let Outbound::SyncPatch {
            state,
            value,
            confirmed,
            ..
        } = frame.outbound()
        else {
            panic!("expected a sync patch");
        };
        assert_eq!(state, "title");
        assert_eq!(confirmed, &vec!["m-1".to_owned()]);
        assert_eq!(value["dup"], "First");
    }

    // The arbiter's dedupe set does not survive a restart; a create that already
    // produced exactly this folder is still the same retried delivery.
    #[test]
    fn folder_create_replayed_across_restart_is_confirmed() {
        let hub = hub_with_session("filed-replay");
        let args = serde_json::json!({"id": "f-r", "name": "Cowboy", "project": "cowboy"});
        hub.sync_apply("folders", "m-1".to_owned(), "create", &args)
            .unwrap();
        let folders = hub.inner.folders.lock().folders().to_vec();

        let restarted = hub_with_session("filed-replay");
        restarted.restore_session_folders(folders);
        let mut live = restarted.subscribe();
        restarted
            .sync_apply("folders", "m-1".to_owned(), "create", &args)
            .unwrap();
        let frame = live.try_recv().expect("replayed create confirmation");
        let Outbound::SyncPatch {
            state,
            value,
            confirmed,
            ..
        } = frame.outbound()
        else {
            panic!("expected a folders patch");
        };
        assert_eq!(state, "folders");
        assert_eq!(confirmed, &vec!["m-1".to_owned()]);
        assert_eq!(value["folders"].as_array().unwrap().len(), 1);
        // A different folder under the same id is still a conflict.
        assert_eq!(
            restarted
                .sync_apply(
                    "folders",
                    "m-2".to_owned(),
                    "create",
                    &serde_json::json!({"id": "f-r", "name": "Other"}),
                )
                .unwrap_err(),
            "folder id already exists"
        );
    }

    // A replayed edit or removal must be able to tell the client when its row
    // already left the queue or drafts, so the outbox entry can be retired.
    #[test]
    fn queue_and_draft_edits_report_whether_the_row_still_exists() {
        let hub = hub_with_session("rows");
        hub.submit("rows", "queued".to_owned(), vec![], Some("q-1".to_owned()));
        hub.add_draft("rows", "draft".to_owned(), vec![], Some("d-1".to_owned()));
        let Some(Outbound::SyncPatch { value, .. }) = hub.queue_resync("rows") else {
            panic!("expected a queue resync");
        };
        let queued = value["queue"][0]["id"].as_str().unwrap().to_owned();
        let draft = value["drafts"][0]["id"].as_str().unwrap().to_owned();

        assert!(hub.edit_queued("rows", &queued, "edited".to_owned(), vec![]));
        assert!(!hub.edit_queued("rows", "missing", "edited".to_owned(), vec![]));
        assert!(hub.remove_queued("rows", &queued));
        assert!(!hub.remove_queued("rows", &queued));
        assert!(!hub.edit_queued("rows", &queued, String::new(), vec![]));

        assert!(hub.edit_draft("rows", &draft, "edited".to_owned(), vec![]));
        assert!(!hub.edit_draft("rows", "missing", "edited".to_owned(), vec![]));
        assert!(hub.remove_draft("rows", &draft));
        assert!(!hub.remove_draft("rows", &draft));
        assert!(!hub.edit_draft("rows", &draft, String::new(), vec![]));

        assert!(!hub.remove_queued("gone", "x"));
        assert!(!hub.remove_draft("gone", "x"));
    }

    // A permission answered on two devices resolves once in the transcript;
    // the first answer wins and the second appends no row.
    #[test]
    fn duplicate_permission_resolution_appends_no_second_row() {
        let hub = hub_with_session("perm");
        hub.push(
            "perm",
            Event::PermissionRequest {
                request_id: "r-1".to_owned(),
                tool_call: serde_json::json!({}),
                options: serde_json::json!([]),
            },
        );
        hub.push(
            "perm",
            Event::PermissionResolved {
                request_id: "r-1".to_owned(),
                option_id: Some("allow".to_owned()),
            },
        );
        hub.push(
            "perm",
            Event::PermissionResolved {
                request_id: "r-1".to_owned(),
                option_id: Some("deny".to_owned()),
            },
        );
        let resolutions = |hub: &Hub| -> Vec<Option<String>> {
            hub.snapshot("perm")
                .expect("snapshot")
                .0
                .into_iter()
                .filter_map(|entry| match entry.event {
                    Event::PermissionResolved { option_id, .. } => Some(option_id),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(resolutions(&hub), vec![Some("allow".to_owned())]);
        hub.push(
            "perm",
            Event::PermissionResolved {
                request_id: "r-2".to_owned(),
                option_id: None,
            },
        );
        assert_eq!(resolutions(&hub), vec![Some("allow".to_owned()), None]);
    }

    // Per-session sync dedupe sets are dropped with the session.
    #[test]
    fn deleting_a_session_drops_its_sync_states() {
        let hub = hub_with_session("gone");
        hub.sync_apply(
            "mobile-review:gone",
            "m-1".to_owned(),
            "open",
            &serde_json::json!({"path": "README.md"}),
        )
        .unwrap();
        hub.add_draft("gone", "draft".to_owned(), vec![], Some("d-1".to_owned()));
        assert!(hub.inner.sync.lock().contains_key("mobile-review:gone"));
        assert!(hub.inner.sync.lock().contains_key("queue:gone"));
        assert!(hub.delete_session("gone"));
        assert!(!hub.inner.sync.lock().contains_key("mobile-review:gone"));
        assert!(!hub.inner.sync.lock().contains_key("queue:gone"));
    }

    // A pending environment stop is visible on the wire only while it lasts.
    #[test]
    fn closing_flag_is_listed_only_while_set() {
        let hub = hub_with_session("closing");
        let listed =
            |hub: &Hub| serde_json::to_value(&hub.session_list()[0]).expect("serialize session");
        assert!(listed(&hub).get("closing").is_none());
        hub.set_closing("closing", true);
        assert_eq!(listed(&hub)["closing"], true);
        hub.set_closing("closing", false);
        assert!(listed(&hub).get("closing").is_none());
    }

    #[test]
    fn session_cwd_retarget_preserves_native_thread_and_transcript() {
        let hub = hub_with_session("migrated");
        hub.set_agent_session_id("migrated", "codex-thread-1".to_owned());
        hub.push(
            "migrated",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"text": "preserved"}
                }),
            },
        );
        let before = hub.snapshot("migrated").expect("snapshot").0;

        hub.update_session_cwd("migrated", "/new/checkout".to_owned())
            .expect("retarget");

        let meta = hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == "migrated")
            .expect("session");
        assert_eq!(meta.cwd, "/new/checkout");
        assert_eq!(meta.agent_session_id.as_deref(), Some("codex-thread-1"));
        let after = hub.snapshot("migrated").expect("snapshot").0;
        assert_eq!(after.len(), before.len());
        assert_eq!(after[0].seq, before[0].seq);
        assert_eq!(
            serde_json::to_value(&after[0].event).expect("serialize after"),
            serde_json::to_value(&before[0].event).expect("serialize before")
        );
    }

    #[tokio::test]
    async fn repeated_retry_dispatches_original_prompt_once() {
        let hub = hub_with_session("retry-once");
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.push(
            "retry-once",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"text": "do the thing"}
                }),
            },
        );
        hub.set_status("retry-once", Status::Crashed, Some("boom".to_owned()));

        hub.retry_turn("retry-once");
        hub.retry_turn("retry-once");

        let first = rx.recv().await.expect("first retry");
        assert_eq!(first.text, "do the thing");
        assert!(
            first
                .cmid
                .as_deref()
                .is_some_and(|id| id.starts_with("cowboy-retry:"))
        );
        assert!(rx.try_recv().is_err(), "duplicate retry was dispatched");
    }

    #[tokio::test]
    async fn retry_drains_rejected_prompt_without_enqueuing_a_copy() {
        let hub = hub_with_session("retry-requeued");
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.push(
            "retry-requeued",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"text": "preserve this prompt"}
                }),
            },
        );
        hub.set_status(
            "retry-requeued",
            Status::Crashed,
            Some("stale cwd".to_owned()),
        );
        hub.requeue_prompt(
            "retry-requeued",
            "preserve this prompt".to_owned(),
            vec![serde_json::json!({"type": "text", "text": "preserve this prompt"})],
            Some("original-cmid".to_owned()),
        );

        hub.retry_turn("retry-requeued");

        let dispatched = rx.recv().await.expect("requeued prompt");
        assert_eq!(dispatched.text, "preserve this prompt");
        assert_eq!(dispatched.cmid.as_deref(), Some("original-cmid"));
        assert!(rx.try_recv().is_err(), "retry inserted a duplicate prompt");
        assert_eq!(
            hub.session_info("retry-requeued")
                .expect("session")
                .queue_count,
            0
        );
    }

    #[test]
    fn persisted_hub_bounds_hot_history_but_keeps_total_count() {
        let health = std::sync::Arc::new(PersistenceHealth::default());
        let (sink, _rx) = StoreSink::channel(HOT_TAIL + HOT_TAIL_TRIM_BATCH + 2, health);
        let hub = Hub::with_store(Some(sink));
        hub.create_local_session(
            "bounded".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "bounded".to_owned(),
            SessionOrigin::Api,
            false,
        );
        for n in 0..=HOT_TAIL + HOT_TAIL_TRIM_BATCH {
            hub.push(
                "bounded",
                Event::Update {
                    update: serde_json::json!({"sessionUpdate": "plan", "n": n}),
                },
            );
        }
        assert_eq!(
            hub.event_total(),
            u64::try_from(HOT_TAIL + HOT_TAIL_TRIM_BATCH + 1).unwrap()
        );
        let (snapshot, reached_start) = hub.snapshot("bounded").expect("session snapshot");
        assert_eq!(snapshot.len(), SNAPSHOT_TAIL);
        assert!(!reached_start);
    }

    #[test]
    fn hub_hot_history_reduces_stream_chunks_but_broadcasts_raw_frames() {
        let hub = hub_with_session("canonical-hot-tail");
        let mut live = hub.subscribe();
        for (seq, text) in ["hello ", "world"].into_iter().enumerate() {
            hub.push(
                "canonical-hot-tail",
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "agent_message_chunk",
                        "messageId": "answer",
                        "content": {"type": "text", "text": text},
                    }),
                },
            );
            let frame = live.try_recv().expect("raw live frame");
            let Outbound::Event { envelope } = &**frame else {
                panic!("expected event");
            };
            assert_eq!(envelope.seq, u64::try_from(seq).unwrap());
        }

        let (snapshot, reached_start) = hub.snapshot("canonical-hot-tail").expect("snapshot");
        assert!(reached_start);
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].seq, 0);
        let Event::Update { update } = &snapshot[0].event else {
            panic!("expected canonical update");
        };
        assert_eq!(
            update
                .pointer("/content/text")
                .and_then(serde_json::Value::as_str),
            Some("hello world")
        );
        assert_eq!(hub.event_total(), 1);
    }

    #[test]
    fn fanout_shares_compact_live_frame_while_hot_history_is_compact() {
        let hub = hub_with_session("shared-fanout");
        let mut first = hub.subscribe();
        let mut second = hub.subscribe();
        hub.push(
            "shared-fanout",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "image",
                    "status": "completed",
                    "content": [{"type": "content", "content": {"type": "text", "text": "saved"}}],
                    "rawOutput": {"result": "x".repeat(128 * 1024)},
                }),
            },
        );

        let first = first.try_recv().expect("first compact frame");
        let second = second.try_recv().expect("second compact frame");
        assert!(Arc::ptr_eq(&first, &second));
        let Outbound::Event { envelope } = &**first else {
            panic!("expected event");
        };
        let Event::Update { update } = &envelope.event else {
            panic!("expected compact update");
        };
        assert!(update.get("rawOutput").is_none());
        assert_eq!(
            first.json().expect("shared json").len(),
            second.json().expect("shared json").len()
        );

        let (snapshot, _) = hub.snapshot("shared-fanout").expect("snapshot");
        let Event::Update { update } = &snapshot[0].event else {
            panic!("expected canonical update");
        };
        assert!(update.get("rawOutput").is_none());
    }

    #[test]
    fn persist_queue_receives_compact_tool_events() {
        let health = Arc::new(PersistenceHealth::default());
        let (sink, mut rx) = StoreSink::channel(4, Arc::clone(&health));
        let hub = Hub::with_store(Some(sink));
        hub.create_local_session(
            "queued-compact".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "queued-compact".to_owned(),
            SessionOrigin::Api,
            false,
        );
        while rx.try_recv().is_ok() {}
        hub.push(
            "queued-compact",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "image",
                    "status": "completed",
                    "content": [{"type": "content", "content": {"type": "text", "text": "saved"}}],
                    "rawOutput": {"result": "x".repeat(128 * 1024)},
                }),
            },
        );
        assert!(health.pending_bytes() > 0);
        let write = rx.try_recv().expect("compact persist intent");
        let StoreWrite::AppendEvent(envelope) = &write else {
            panic!("expected compact persist intent");
        };
        let Event::Update { update } = &envelope.event else {
            panic!("expected update");
        };
        assert!(update.get("rawOutput").is_none());
        assert_eq!(health.pending_bytes(), 0);
    }

    #[test]
    fn ingest_externalizes_live_images_before_fanout() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-live-artifacts-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let hub = hub_with_session("live-image");
        hub.set_artifacts(crate::artifacts::ArtifactStore::new(root.clone()).unwrap());
        let mut live = hub.subscribe();
        let data = base64::engine::general_purpose::STANDARD.encode(vec![7_u8; 40_000]);
        hub.push(
            "live-image",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {
                        "type": "image",
                        "mimeType": "image/png",
                        "data": data,
                    }
                }),
            },
        );
        let frame = live.try_recv().expect("live image");
        let Outbound::Event { envelope } = &**frame else {
            panic!("expected event");
        };
        let Event::Update { update } = &envelope.event else {
            panic!("expected update");
        };
        assert!(update.pointer("/content/data").is_none());
        let url = update
            .pointer("/content/url")
            .and_then(serde_json::Value::as_str)
            .expect("artifact url");
        assert!(url.starts_with("/api/artifacts/"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn idle_hot_tail_is_tighter_than_a_busy_turn() {
        let health = Arc::new(PersistenceHealth::default());
        let (sink, _rx) = StoreSink::channel(32, health);
        let hub = Hub::with_store(Some(sink));
        hub.create_local_session(
            "idle-tail".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "idle-tail".to_owned(),
            SessionOrigin::Api,
            false,
        );
        let payload = "x".repeat(200 * 1024);
        for n in 0..8 {
            hub.push(
                "idle-tail",
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "plan",
                        "n": n,
                        "payload": payload,
                    }),
                },
            );
        }
        let sessions = hub.inner.sessions.lock();
        let session = sessions.get("idle-tail").expect("session");
        assert!(session.log_bytes <= HOT_TAIL_IDLE_MAX_BYTES);
        assert!(session.log.len() < 8);
    }

    #[test]
    fn do_shaped_working_set_keeps_shared_compact_frames() {
        let hub = Hub::new();
        for index in 0..17 {
            hub.create_local_session(
                format!("session-{index}"),
                "codex".to_owned(),
                "/tmp".to_owned(),
                format!("session-{index}"),
                SessionOrigin::Api,
                false,
            );
        }
        hub.set_status("session-0", Status::Busy, None);
        let mut terminals: Vec<_> = (0..3).map(|_| hub.subscribe()).collect();
        hub.push(
            "session-0",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "read",
                    "status": "completed",
                    "content": [{"type": "raw_output", "text": "ok"}],
                    "rawOutput": {"result": "x".repeat(2_580_000)},
                }),
            },
        );
        let first = terminals[0].try_recv().expect("terminal 0");
        let second = terminals[1].try_recv().expect("terminal 1");
        let third = terminals[2].try_recv().expect("terminal 2");
        assert!(Arc::ptr_eq(&first, &second) && Arc::ptr_eq(&second, &third));
        let json = first.json().expect("shared json");
        assert!(json.len() < 64 * 1024, "compact live JSON {}", json.len());
        assert!(!json.contains("rawOutput"));
        let stats = hub.memory_stats();
        assert_eq!(stats.session_count, 17);
        assert!(stats.hot_log_bytes < HOT_TAIL_MAX_BYTES);
        assert!(stats.broadcast_last_bytes < 64 * 1024);
    }

    fn production_shaped_do_fixture() -> serde_json::Value {
        let artifact_root = std::env::temp_dir().join(format!(
            "cowboy-do-fixture-artifacts-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_dir_all(&artifact_root);
        let hub = Hub::new();
        hub.set_artifacts(crate::artifacts::ArtifactStore::new(artifact_root.clone()).unwrap());
        for index in 0..17 {
            hub.create_local_session(
                format!("session-{index}"),
                "codex".to_owned(),
                "/tmp".to_owned(),
                format!("session-{index}"),
                SessionOrigin::Api,
                false,
            );
        }
        for index in 1..17 {
            for n in 0..4 {
                hub.push(
                    &format!("session-{index}"),
                    Event::Update {
                        update: serde_json::json!({
                            "sessionUpdate": "plan",
                            "n": n,
                            "title": format!("idle-{index}-{n}"),
                        }),
                    },
                );
            }
        }
        hub.set_status("session-0", Status::Busy, None);
        for text in ["The ", "quick ", "brown ", "fox "] {
            for _ in 0..20 {
                hub.push(
                    "session-0",
                    Event::Update {
                        update: serde_json::json!({
                            "sessionUpdate": "agent_message_chunk",
                            "messageId": "turn",
                            "content": {"type": "text", "text": text},
                        }),
                    },
                );
            }
        }
        let mut terminals: Vec<_> = (0..3).map(|_| hub.subscribe()).collect();
        hub.push(
            "session-0",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "read",
                    "status": "completed",
                    "content": [{"type": "raw_output", "text": "ok"}],
                    "rawOutput": {"result": "x".repeat(2_580_000)},
                }),
            },
        );
        let image = base64::engine::general_purpose::STANDARD.encode(vec![7_u8; 40_000]);
        hub.push(
            "session-0",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {
                        "type": "image",
                        "mimeType": "image/png",
                        "data": image,
                    }
                }),
            },
        );
        let live_frames: Vec<String> = terminals
            .iter_mut()
            .map(|rx| {
                let mut last = String::new();
                while let Ok(frame) = rx.try_recv() {
                    last = frame.json().expect("live json").to_owned();
                }
                last
            })
            .collect();
        let sessions = (0..17)
            .map(|index| {
                let id = format!("session-{index}");
                let (hot_tail, _) = hub.snapshot(&id).expect("snapshot");
                serde_json::json!({
                    "id": id,
                    "status": format!("{:?}", hub.status(&id).expect("status")).to_lowercase(),
                    "hotTail": hot_tail,
                    "hotTailBytes": hot_tail.iter().fold(0usize, |size, envelope| {
                        size.saturating_add(estimated_envelope_bytes(envelope))
                    }),
                })
            })
            .collect::<Vec<_>>();
        let stats = hub.memory_stats();
        let _ = std::fs::remove_dir_all(artifact_root);
        serde_json::json!({
            "generatedBy": "cowboy Hub compact ingest",
            "terminals": 3,
            "rawFanoutWouldHaveBeen": 2_580_000 * 3
                + base64::engine::general_purpose::STANDARD.encode(vec![7_u8; 40_000]).len() * 3,
            "hubHotLogBytes": stats.hot_log_bytes,
            "hubBroadcastLastBytes": stats.broadcast_last_bytes,
            "liveFrames": live_frames,
            "sessions": sessions,
        })
    }

    #[test]
    fn exports_production_shaped_do_fixture() {
        let fixture = production_shaped_do_fixture();
        let live = fixture["liveFrames"].as_array().expect("live frames");
        assert_eq!(live.len(), 3);
        assert!(live.iter().all(|frame| frame == &live[0]));
        let live_json = live[0].as_str().expect("live json");
        assert!(!live_json.contains("rawOutput"));
        assert!(
            live_json.contains("/api/artifacts/") || live_json.contains("\"type\":\"image\""),
            "live image should be externalized or still an image block: {live_json}"
        );
        let sessions = fixture["sessions"].as_array().expect("sessions");
        assert_eq!(sessions.len(), 17);
        let hot_log_bytes = sessions.iter().fold(0usize, |size, session| {
            size.saturating_add(session["hotTailBytes"].as_u64().unwrap_or(0) as usize)
        });
        assert!(hot_log_bytes < HOT_TAIL_MAX_BYTES + HOT_TAIL_IDLE_MAX_BYTES * 16);
        assert!(
            fixture["hubHotLogBytes"].as_u64().unwrap() < 64 * 1024,
            "compact hub tails should stay tiny, got {}",
            fixture["hubHotLogBytes"]
        );
        if let Ok(path) = std::env::var("COWBOY_DO_FIXTURE_OUT") {
            std::fs::write(&path, serde_json::to_vec_pretty(&fixture).unwrap())
                .unwrap_or_else(|error| panic!("write {path}: {error}"));
        }
    }

    fn env_usize(name: &str, default: usize) -> usize {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    }

    /// Extreme DO fixture: dozens of sessions, filled compact tails, many
    /// terminals, fat raw payloads stripped at ingest, plus a count of
    /// SQLite-only archive rows for the worker to synthesize. The Hub still
    /// trims idle/busy tails so the exported `hotTail` is what a focused
    /// isolate should load — not the full durable log.
    fn extreme_do_fixture() -> serde_json::Value {
        let sessions = env_usize("COWBOY_DO_SESSIONS", 50);
        let focused = env_usize("COWBOY_DO_FOCUSED", 4).min(sessions);
        let terminals = env_usize("COWBOY_DO_TERMINALS", 8);
        let idle_chunk_bytes = env_usize("COWBOY_DO_IDLE_CHUNK_BYTES", 8 * 1024);
        let idle_chunks = env_usize("COWBOY_DO_IDLE_CHUNKS", 8);
        let busy_chunks = env_usize("COWBOY_DO_BUSY_CHUNKS", 24);
        let tools = env_usize("COWBOY_DO_TOOLS", 200);
        let fat_events = env_usize("COWBOY_DO_FAT_EVENTS", 1);
        let archive_rows = env_usize("COWBOY_DO_ARCHIVE_ROWS", 8_000);
        let archive_bytes = env_usize("COWBOY_DO_ARCHIVE_BYTES", 512);

        let artifact_root = std::env::temp_dir().join(format!(
            "cowboy-do-extreme-artifacts-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_dir_all(&artifact_root);
        let health = Arc::new(PersistenceHealth::default());
        let (sink, _rx) = StoreSink::channel(64_000, health);
        let hub = Hub::with_store(Some(sink));
        hub.set_artifacts(crate::artifacts::ArtifactStore::new(artifact_root.clone()).unwrap());

        for index in 0..sessions {
            hub.create_local_session(
                format!("session-{index}"),
                "codex".to_owned(),
                "/tmp".to_owned(),
                format!("session-{index}"),
                SessionOrigin::Api,
                false,
            );
        }
        let pad = "x".repeat(idle_chunk_bytes);
        for index in focused..sessions {
            let id = format!("session-{index}");
            for n in 0..idle_chunks {
                hub.push(
                    &id,
                    Event::Update {
                        update: serde_json::json!({
                            "sessionUpdate": "plan",
                            "n": n,
                            "payload": pad,
                        }),
                    },
                );
            }
        }
        for index in 0..focused {
            let id = format!("session-{index}");
            hub.set_status(&id, Status::Busy, None);
            for n in 0..busy_chunks {
                hub.push(
                    &id,
                    Event::Update {
                        update: serde_json::json!({
                            "sessionUpdate": "plan",
                            "n": n,
                            "payload": pad,
                        }),
                    },
                );
            }
        }
        for n in 0..tools {
            hub.push(
                "session-0",
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": format!("tool-{n}"),
                        "status": "completed",
                        "content": [{"type": "raw_output", "text": format!("done-{n}")}],
                    }),
                },
            );
        }
        let mut live_rx: Vec<_> = (0..terminals).map(|_| hub.subscribe()).collect();
        let mut raw_fanout = 0usize;
        for n in 0..fat_events {
            raw_fanout = raw_fanout.saturating_add(2_580_000 * terminals);
            hub.push(
                "session-0",
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": format!("fat-{n}"),
                        "status": "completed",
                        "content": [{"type": "raw_output", "text": "ok"}],
                        "rawOutput": {"result": "x".repeat(2_580_000)},
                    }),
                },
            );
        }
        let image = base64::engine::general_purpose::STANDARD.encode(vec![7_u8; 40_000]);
        raw_fanout = raw_fanout.saturating_add(image.len() * terminals);
        hub.push(
            "session-0",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {
                        "type": "image",
                        "mimeType": "image/png",
                        "data": image,
                    }
                }),
            },
        );
        let live_frames: Vec<String> = live_rx
            .iter_mut()
            .map(|rx| {
                let mut last = String::new();
                while let Ok(frame) = rx.try_recv() {
                    last = frame.json().expect("live json").to_owned();
                }
                last
            })
            .collect();

        let session_rows = {
            let locked = hub.inner.sessions.lock();
            (0..sessions)
                .map(|index| {
                    let id = format!("session-{index}");
                    let session = locked.get(&id).expect("session");
                    serde_json::json!({
                        "id": id,
                        "status": format!("{:?}", session.meta.status).to_lowercase(),
                        "focused": index < focused,
                        "hotTail": session.log,
                        "hotTailBytes": session.log_bytes,
                    })
                })
                .collect::<Vec<_>>()
        };
        let stats = hub.memory_stats();
        let _ = std::fs::remove_dir_all(artifact_root);
        serde_json::json!({
            "generatedBy": "cowboy Hub extreme compact ingest",
            "profile": "extreme",
            "terminals": terminals,
            "focusedSessionIds": (0..focused).map(|index| format!("session-{index}")).collect::<Vec<_>>(),
            "rawFanoutWouldHaveBeen": raw_fanout,
            "hubHotLogBytes": stats.hot_log_bytes,
            "hubBroadcastLastBytes": stats.broadcast_last_bytes,
            "archiveRows": archive_rows,
            "archivePayloadBytes": archive_bytes,
            "liveFrames": live_frames,
            "sessions": session_rows,
        })
    }

    #[test]
    fn exports_extreme_do_fixture() {
        let fixture = extreme_do_fixture();
        let sessions = fixture["sessions"].as_array().expect("sessions");
        assert!(
            sessions.len() >= 20,
            "extreme mock needs dozens of sessions"
        );
        let live = fixture["liveFrames"].as_array().expect("live frames");
        assert!(live.len() >= 5);
        assert!(live.iter().all(|frame| frame == &live[0]));
        let live_json = live[0].as_str().expect("live json");
        assert!(!live_json.contains("rawOutput"));
        assert!(live_json.contains("/api/artifacts/"));
        let focused = sessions
            .iter()
            .filter(|session| session["focused"].as_bool() == Some(true))
            .count();
        assert!(focused >= 2);
        let hub_bytes = fixture["hubHotLogBytes"].as_u64().unwrap();
        assert!(
            hub_bytes < 80 * 1024 * 1024,
            "even extreme compact tails must stay under the DO isolate, got {hub_bytes}"
        );
        if let Ok(path) = std::env::var("COWBOY_DO_FIXTURE_OUT") {
            std::fs::write(&path, serde_json::to_vec(&fixture).unwrap())
                .unwrap_or_else(|error| panic!("write {path}: {error}"));
        }
    }

    #[test]
    fn persisted_hub_bounds_canonical_hot_history_by_payload_bytes() {
        let health = std::sync::Arc::new(PersistenceHealth::default());
        let (sink, _rx) = StoreSink::channel(32, health);
        let hub = Hub::with_store(Some(sink));
        hub.create_local_session(
            "byte-hot-tail".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "byte-hot-tail".to_owned(),
            SessionOrigin::Api,
            false,
        );
        let payload = "x".repeat(384 * 1024);
        for n in 0..12 {
            hub.push(
                "byte-hot-tail",
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "plan",
                        "n": n,
                        "payload": payload,
                    }),
                },
            );
        }

        let sessions = hub.inner.sessions.lock();
        let session = sessions.get("byte-hot-tail").expect("session");
        assert!(session.log.len() < 12);
        assert!(session.log_bytes <= HOT_TAIL_MAX_BYTES);
        assert!(!session.reached_start);
        assert_eq!(session.event_count, 12);
    }

    #[test]
    fn persisted_hub_keeps_one_oversized_newest_event() {
        let health = std::sync::Arc::new(PersistenceHealth::default());
        let (sink, _rx) = StoreSink::channel(4, health);
        let hub = Hub::with_store(Some(sink));
        hub.create_local_session(
            "oversized-hot-tail".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "oversized-hot-tail".to_owned(),
            SessionOrigin::Api,
            false,
        );
        hub.push(
            "oversized-hot-tail",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "tool_call_update",
                    "payload": "x".repeat(HOT_TAIL_MAX_BYTES + 1),
                }),
            },
        );

        let sessions = hub.inner.sessions.lock();
        let session = sessions.get("oversized-hot-tail").expect("session");
        assert_eq!(session.log.len(), 1);
        assert!(session.log_bytes > HOT_TAIL_MAX_BYTES);
        assert_eq!(session.event_count, 1);
    }

    #[test]
    fn session_snapshot_is_byte_bounded_and_keeps_the_newest_event() {
        let hub = hub_with_session("byte-bounded");
        let payload = "x".repeat(96 * 1024);
        for n in 0..4 {
            hub.push(
                "byte-bounded",
                Event::Update {
                    update: serde_json::json!({"sessionUpdate": "tool_call_update", "n": n, "payload": payload}),
                },
            );
        }

        let (snapshot, reached_start) = hub.snapshot("byte-bounded").expect("session snapshot");
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].seq, 3);
        assert!(!reached_start);
        assert!(serde_json::to_vec(&snapshot[0]).unwrap().len() < SNAPSHOT_MAX_BYTES);
    }

    #[test]
    fn session_snapshot_does_not_split_a_rich_user_prompt() {
        let hub = hub_with_session("rich-prompt-boundary");
        let payload = "x".repeat(96 * 1024);
        hub.push(
            "rich-prompt-boundary",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": { "type": "image", "data": payload, "mimeType": "image/jpeg" }
                }),
            },
        );
        hub.push(
            "rich-prompt-boundary",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": { "type": "text", "text": "adjust this image" }
                }),
            },
        );
        hub.push(
            "rich-prompt-boundary",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "tool_call_update",
                    "payload": payload
                }),
            },
        );

        let (snapshot, reached_start) = hub
            .snapshot("rich-prompt-boundary")
            .expect("session snapshot");
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot[0].seq, 0);
        assert!(is_user_message_chunk(&snapshot[0]));
        assert!(is_user_message_chunk(&snapshot[1]));
        assert!(reached_start);
        assert!(
            serde_json::to_vec(&snapshot).unwrap().len() > SNAPSHOT_MAX_BYTES,
            "message atomicity may exceed the soft byte budget"
        );
    }

    #[test]
    fn cursor_history_ignores_sequence_gaps() {
        let hub = hub_with_session("cursor");
        {
            let mut sessions = hub.inner.sessions.lock();
            let session = sessions.get_mut("cursor").expect("session");
            session.log = (0..450)
                .map(|index| Envelope {
                    session_id: "cursor".to_owned(),
                    seq: u64::try_from(index * 3).unwrap(),
                    event: Event::TurnEnd {
                        stop_reason: "done".to_owned(),
                    },
                    cmid: None,
                })
                .collect();
            session.reached_start = true;
        }
        let mut cursor = Some(u64::MAX);
        let mut seen = Vec::new();
        while let Some(before_seq) = cursor {
            let (page, next, reached_start) = hub.history_page("cursor", before_seq).unwrap();
            assert!(!page.is_empty());
            assert!(page.len() <= HISTORY_PAGE);
            seen.splice(0..0, page.iter().map(|event| event.seq));
            cursor = next;
            if reached_start {
                assert_eq!(cursor, None);
                break;
            }
        }
        assert_eq!(seen.len(), 450);
        assert!(seen.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn cursor_history_is_byte_bounded_and_always_advances() {
        let hub = hub_with_session("history-bytes");
        {
            let mut sessions = hub.inner.sessions.lock();
            let session = sessions.get_mut("history-bytes").expect("session");
            session.log = (0..3)
                .map(|seq| Envelope {
                    session_id: "history-bytes".to_owned(),
                    seq,
                    event: Event::Update {
                        update: serde_json::json!({
                            "sessionUpdate": "tool_call",
                            "toolCallId": format!("tool-{seq}"),
                            "content": "x".repeat(300 * 1024),
                        }),
                    },
                    cmid: None,
                })
                .collect();
            session.reached_start = true;
        }

        let (newest, cursor, reached_start) = hub.history_page("history-bytes", u64::MAX).unwrap();
        assert_eq!(newest.len(), 1);
        assert_eq!(newest[0].seq, 2);
        assert!(!reached_start);

        let (middle, next, reached_start) =
            hub.history_page("history-bytes", cursor.unwrap()).unwrap();
        assert_eq!(middle.len(), 1);
        assert_eq!(middle[0].seq, 1);
        assert!(!reached_start);
        assert!(next.unwrap() < 2);
    }

    #[test]
    fn question_history_returns_one_complete_prompt_rooted_page() {
        let hub = hub_with_session("question-page");
        for (kind, text) in [
            ("user_message_chunk", "first question"),
            ("agent_message_chunk", "first answer"),
            ("tool_call_update", "first tool"),
            ("user_message_chunk", "second question"),
            ("agent_message_chunk", "second answer"),
        ] {
            hub.push(
                "question-page",
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": kind,
                        "content": {"text": text},
                    }),
                },
            );
        }

        let (page, cursor, reached_start) = hub.question_page_before("question-page", 3).unwrap();
        assert_eq!(
            page.iter().map(|event| event.seq).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(cursor, None);
        assert!(reached_start);

        let (latest, before, total, exact) = hub
            .question_page_summaries("question-page", None, 1)
            .expect("latest question summary");
        assert_eq!(latest[0].title, "second question");
        assert_eq!(latest[0].ordinal, 2);
        assert_eq!(before, Some(3));
        assert_eq!(total, 2);
        assert!(exact);

        let (earlier, before, _, _) = hub
            .question_page_summaries("question-page", before, 1)
            .expect("earlier question summary");
        assert_eq!(earlier[0].title, "first question");
        assert_eq!(earlier[0].ordinal, 1);
        assert_eq!(before, None);

        let lazy_page = hub
            .question_page_at("question-page", 0)
            .expect("lazy question page");
        assert_eq!(
            lazy_page.iter().map(|event| event.seq).collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }

    #[test]
    fn question_history_stops_before_background_output_after_turn_end() {
        let hub = hub_with_session("question-tail");
        hub.push(
            "question-tail",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"text": "show me the logs"},
                }),
            },
        );
        hub.push(
            "question-tail",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"text": "the watcher is running"},
                }),
            },
        );
        hub.push(
            "question-tail",
            Event::TurnEnd {
                stop_reason: "end_turn".to_owned(),
            },
        );
        for index in 0..100 {
            hub.push(
                "question-tail",
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": "watcher",
                        "line": index,
                    }),
                },
            );
        }

        let (page, _, _) = hub
            .question_page_before("question-tail", 103)
            .expect("question page");
        assert_eq!(
            page.iter().map(|event| event.seq).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        let lazy = hub
            .question_page_at("question-tail", 0)
            .expect("lazy question page");
        assert_eq!(
            lazy.iter().map(|event| event.seq).collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }

    #[test]
    fn context_management_commands_are_not_question_pages() {
        let hub = hub_with_session("question-commands");
        for text in ["first question", "/compact", "second question"] {
            hub.push(
                "question-commands",
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "user_message_chunk",
                        "content": {"text": text},
                    }),
                },
            );
            hub.push(
                "question-commands",
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"text": "response"},
                    }),
                },
            );
        }

        let (pages, next, total, exact) = hub
            .question_page_summaries("question-commands", None, 64)
            .expect("question summaries");
        assert_eq!(pages.len(), 2);
        assert_eq!(next, None);
        assert_eq!(total, 2);
        assert!(exact);
    }

    #[test]
    fn native_thread_becomes_resumable_only_after_a_user_turn_in_current_context() {
        let hub = hub_with_session("native-durability");
        hub.set_agent_session_id("native-durability", "thread-empty".to_owned());
        assert_eq!(hub.agent_session_id_for_resume("native-durability"), None);

        hub.push(
            "native-durability",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": "first prompt"},
                }),
            },
        );
        assert_eq!(
            hub.agent_session_id_for_resume("native-durability")
                .as_deref(),
            Some("thread-empty")
        );

        hub.mark_context_cleared("native-durability");
        hub.set_agent_session_id("native-durability", "thread-after-clear".to_owned());
        assert_eq!(hub.agent_session_id_for_resume("native-durability"), None);

        hub.push(
            "native-durability",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": "new context prompt"},
                }),
            },
        );
        assert_eq!(
            hub.agent_session_id_for_resume("native-durability")
                .as_deref(),
            Some("thread-after-clear")
        );
    }

    #[test]
    fn clear_transcript_discards_history_before_the_new_boundary() {
        let hub = hub_with_session("clear-history");
        hub.push(
            "clear-history",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "old"},
                }),
            },
        );

        hub.clear_transcript("clear-history");
        hub.mark_context_cleared("clear-history");

        let (events, reached_start) = hub.snapshot("clear-history").expect("snapshot");
        assert!(reached_start);
        assert_eq!(events.len(), 1);
        assert!(is_context_cleared(&events[0]));
        assert_eq!(
            hub.session_info("clear-history")
                .expect("session")
                .event_count,
            1
        );
    }

    fn provider_reload_fixture() -> (Hub, SessionMeta) {
        let hub = hub_with_session("provider-reload");
        hub.push("provider-reload", Event::Update {
            update: serde_json::json!({"sessionUpdate": "user_message_chunk", "content": {"text": "keep history"}}),
        });
        hub.set_agent_session_id("provider-reload", "native-thread".to_owned());
        hub.set_status("provider-reload", Status::Running, None);
        hub.set_config_preference(
            "provider-reload",
            "model".to_owned(),
            serde_json::json!("saved-model"),
        )
        .expect("preference");
        let meta = hub.session_info("provider-reload").expect("session").meta;
        (hub, meta)
    }

    #[tokio::test]
    async fn provider_reload_preserves_identity_and_fences_new_prompts() {
        let (hub, before) = provider_reload_fixture();
        let behavior = crate::provider::legacy_behavior("codex");
        hub.add_draft(&before.id, "saved draft".to_owned(), vec![], None);
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        let (history, _) = hub.snapshot(&before.id).expect("history");
        hub.begin_provider_reload(&before, "new-version", "new-digest", &behavior, false)
            .expect("reload");
        hub.submit(&before.id, "racing prompt".to_owned(), vec![], None);
        assert!(rx.try_recv().is_err(), "prompt must wait for replacement");
        let after = hub.session_info(&before.id).expect("session");
        assert_eq!(after.meta.provider_version, "new-version");
        assert_eq!(after.meta.provider_generation_digest, "new-digest");
        assert_eq!(after.meta.provider_behavior, Some(behavior));
        assert_eq!(after.meta.agent_session_id, before.agent_session_id);
        assert_eq!(
            after.meta.provider_auth_generation,
            before.provider_auth_generation
        );
        assert_eq!(after.meta.cwd, before.cwd);
        assert_eq!(after.meta.title, before.title);
        assert_eq!(after.meta.machine_id, before.machine_id);
        assert_eq!(after.meta.status, Status::Starting);
        assert_eq!(after.drafts_count, 1);
        assert_eq!(after.queue_count, 1);
        assert_eq!(
            hub.config_preferences(&before.id).unwrap()["model"],
            "saved-model"
        );
        assert_eq!(
            serde_json::to_value(&hub.snapshot(&before.id).unwrap().0[..history.len()]).unwrap(),
            serde_json::to_value(history).unwrap()
        );
    }

    #[test]
    fn provider_reload_publishes_its_update_until_the_worker_settles() {
        let (hub, before) = provider_reload_fixture();
        let behavior = crate::provider::legacy_behavior("codex");
        hub.begin_provider_reload(&before, "new-version", "new-digest", &behavior, true)
            .expect("reload");
        let starting = hub.session_info(&before.id).expect("session").meta;
        let update = starting.provider_update.clone().expect("published update");
        assert_eq!(update.from, before.provider_version);
        assert_eq!(update.to, "new-version");
        assert!(update.automatic);
        assert!(update.started_at_ms > 0);
        let wire = serde_json::to_value(&starting).expect("serialize");
        assert_eq!(wire["provider_update"]["automatic"], true);
        hub.set_status(&before.id, Status::Starting, None);
        assert!(
            hub.session_info(&before.id)
                .unwrap()
                .meta
                .provider_update
                .is_some()
        );
        hub.set_status(&before.id, Status::Running, None);
        let settled = hub.session_info(&before.id).expect("session").meta;
        assert_eq!(settled.provider_update, None);
        assert!(
            serde_json::to_value(&settled)
                .unwrap()
                .get("provider_update")
                .is_none()
        );
    }

    #[tokio::test]
    async fn provider_reload_rejects_a_prompt_that_won_the_race() {
        let (hub, before) = provider_reload_fixture();
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.submit(&before.id, "active prompt".to_owned(), vec![], None);
        assert!(rx.recv().await.is_some());
        let behavior = crate::provider::legacy_behavior("codex");
        assert!(
            hub.begin_provider_reload(&before, "new", "new", &behavior, false)
                .unwrap_err()
                .contains("current turn")
        );
        assert_eq!(
            hub.session_info(&before.id)
                .unwrap()
                .meta
                .provider_generation_digest,
            before.provider_generation_digest
        );
        assert!(hub.session_has_in_flight_prompt(&before.id));
    }

    #[test]
    fn provider_reload_reconciles_interrupted_reset_without_migrating_native_identity() {
        let (hub, before) = provider_reload_fixture();
        let behavior = crate::provider::legacy_behavior("codex");
        hub.begin_provider_reload(&before, "intended", "intended-digest", &behavior, false)
            .unwrap();
        let mut worker =
            super::runtime_reconciliation_tests::worker_snapshot(&before.id, "surviving-worker");
        worker.agent_session_id.clone_from(&before.agent_session_id);
        worker.launch = Some(crate::runtime_wire::StartSession {
            session_id: before.id.clone(),
            provider: before.provider.clone(),
            provider_version: "actual".to_owned(),
            provider_generation_digest: "actual-digest".to_owned(),
            provider_auth_generation: before.provider_auth_generation,
            provider_behavior: Some(behavior),
            cwd: before.cwd.clone(),
            agent_session_id: before.agent_session_id.clone(),
            system: false,
            context_window: None,
            auto_compact_token_limit: None,
            cache_protection: None,
            generation: "worker-generation".to_owned(),
            fallback_for: None,
            adopt_only: false,
            execution_binding: None,
        });
        let history = serde_json::to_value(hub.snapshot(&before.id).unwrap().0).unwrap();
        for mismatch in ["native", "home", "cwd", "placeholder"] {
            let mut invalid = worker.clone();
            match mismatch {
                "native" => invalid.agent_session_id = Some("other-thread".to_owned()),
                "home" => invalid.launch.as_mut().unwrap().provider_auth_generation = Some(999),
                "cwd" => invalid.launch.as_mut().unwrap().cwd = "/other".to_owned(),
                _ => invalid.worker_epoch = format!("broker-{}", before.id),
            }
            hub.reconcile_provider_release(&invalid);
            assert_eq!(
                hub.session_info(&before.id).unwrap().meta.provider_version,
                "intended"
            );
        }
        hub.reconcile_provider_release(&worker);
        let actual = hub.session_info(&before.id).unwrap().meta;
        assert_eq!(actual.provider_version, "actual");
        assert_eq!(actual.provider_generation_digest, "actual-digest");
        assert_eq!(actual.agent_session_id, before.agent_session_id);
        assert_eq!(
            actual.provider_auth_generation,
            before.provider_auth_generation
        );
        assert_eq!(
            serde_json::to_value(hub.snapshot(&before.id).unwrap().0).unwrap(),
            history
        );
        // The opposite crash ordering (new worker, old persisted binding) uses
        // the same actual-owner reconciliation without an automatic restart.
        worker.launch.as_mut().unwrap().provider_version = "replacement".to_owned();
        worker.launch.as_mut().unwrap().provider_generation_digest =
            "replacement-digest".to_owned();
        hub.reconcile_provider_release(&worker);
        assert_eq!(
            hub.session_info(&before.id).unwrap().meta.provider_version,
            "replacement"
        );
    }

    #[test]
    fn dormant_repin_changes_only_the_binding_and_only_while_exited() {
        let (hub, running) = provider_reload_fixture();
        let behavior = crate::provider::legacy_behavior("codex");
        // A live session is not dormant.
        assert!(
            hub.repin_dormant_provider(&running, "new", "new-digest", &behavior)
                .unwrap_err()
                .contains("dormant")
        );
        hub.set_status(&running.id, Status::Exited, None);
        let before = hub.session_info(&running.id).unwrap().meta;
        let mut stale = before.clone();
        stale.provider_generation_digest = "other".to_owned();
        assert!(
            hub.repin_dormant_provider(&stale, "new", "new-digest", &behavior)
                .unwrap_err()
                .contains("changed")
        );
        hub.repin_dormant_provider(&before, "new", "new-digest", &behavior)
            .expect("re-pin");
        let after = hub.session_info(&before.id).unwrap().meta;
        assert_eq!(after.provider_version, "new");
        assert_eq!(after.provider_generation_digest, "new-digest");
        assert_eq!(after.provider_behavior, Some(behavior));
        assert_eq!(after.status, Status::Exited, "nothing is started");
        assert_eq!(after.agent_session_id, before.agent_session_id);
        assert_eq!(
            after.provider_auth_generation,
            before.provider_auth_generation
        );
        assert_eq!(after.cwd, before.cwd);
        assert_eq!(after.machine_id, before.machine_id);
    }

    #[test]
    fn provider_update_offer_follows_binding_and_one_shot_request() {
        let (hub, meta) = provider_reload_fixture();
        let offer = |when_idle| ProviderUpdateOffer {
            version: "new".to_owned(),
            digest: "new-digest".to_owned(),
            when_idle,
            automatic_after: Some(std::time::Duration::from_hours(1)),
        };
        let listed = |hub: &Hub| {
            hub.session_list()
                .into_iter()
                .find(|listed| listed.id == meta.id)
                .unwrap()
                .provider_update_available
        };
        assert_eq!(listed(&hub), None);
        hub.publish_provider_update_offers(HashMap::from([(meta.id.clone(), offer(false))]));
        let available = listed(&hub).expect("offer listed");
        assert_eq!(available.version, "new");
        assert!(!available.when_idle);
        let at = available.automatic_at_ms.expect("idle policy time");
        assert!(at > now_ms() + 3_500_000 && at <= now_ms() + 3_600_000);

        hub.set_provider_update_when_idle(&meta.id, true);
        assert!(hub.provider_update_when_idle(&meta.id));
        assert!(listed(&hub).unwrap().when_idle);

        // Once the session is bound to the offered release it is not offered.
        let behavior = crate::provider::legacy_behavior("codex");
        hub.set_status(&meta.id, Status::Exited, None);
        let exited = hub.session_info(&meta.id).unwrap().meta;
        hub.repin_dormant_provider(&exited, "new", "new-digest", &behavior)
            .expect("re-pin");
        assert_eq!(listed(&hub), None);

        hub.settle_provider_update_offer(&meta.id);
        assert!(!hub.provider_update_when_idle(&meta.id));
    }

    #[test]
    fn provider_reload_rejects_stale_identity_and_unsaved_context() {
        let (hub, before) = provider_reload_fixture();
        let behavior = crate::provider::legacy_behavior("codex");
        let mut stale = before.clone();
        stale.provider_generation_digest = "other".to_owned();
        assert!(
            hub.begin_provider_reload(&stale, "new", "new", &behavior, false)
                .unwrap_err()
                .contains("changed")
        );
        hub.prepare_context_reset(&before.id);
        let cleared = hub.session_info(&before.id).unwrap().meta;
        assert!(
            hub.begin_provider_reload(&cleared, "new", "new", &behavior, false)
                .unwrap_err()
                .contains("saved native")
        );
        assert_eq!(hub.status(&before.id), Some(Status::Running));
    }

    #[tokio::test]
    async fn context_reset_releases_stale_in_flight_guard_for_queued_send() {
        let hub = hub_with_session("reset-queue");
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("reset-queue", Status::Running, None);

        hub.submit("reset-queue", "old turn".to_owned(), vec![], None);
        let old_turn = rx.recv().await.expect("old turn dispatch");
        assert_eq!(old_turn.text, "old turn");
        hub.submit("reset-queue", "after reset".to_owned(), vec![], None);
        assert_eq!(queue_texts(&hub, "reset-queue"), vec!["after reset"]);

        hub.prepare_context_reset("reset-queue");
        hub.set_status("reset-queue", Status::Starting, None);
        hub.set_status("reset-queue", Status::Running, None);

        let dispatched = rx.recv().await.expect("queued dispatch after reset");
        assert_eq!(dispatched.text, "after reset");
        assert!(rx.try_recv().is_err());
        assert!(queue_texts(&hub, "reset-queue").is_empty());
    }

    #[tokio::test]
    async fn clean_turn_end_drains_next_message_without_classifier_hold() {
        let hub = hub_with_session("queue-after-turn");
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("queue-after-turn", Status::Running, None);

        hub.submit("queue-after-turn", "first".to_owned(), vec![], None);
        assert_eq!(rx.recv().await.expect("first dispatch").text, "first");
        hub.submit("queue-after-turn", "second".to_owned(), vec![], None);
        assert_eq!(queue_texts(&hub, "queue-after-turn"), vec!["second"]);

        hub.set_status("queue-after-turn", Status::Busy, None);
        hub.set_status("queue-after-turn", Status::Running, None);

        assert_eq!(rx.recv().await.expect("turn-end dispatch").text, "second");
        assert!(queue_texts(&hub, "queue-after-turn").is_empty());
    }

    #[tokio::test]
    async fn turn_end_then_idle_status_dispatches_only_the_force_pushed_prompt() {
        let hub = hub_with_session("force-turn-end-first");
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("force-turn-end-first", Status::Running, None);

        hub.submit(
            "force-turn-end-first",
            "active turn".to_owned(),
            vec![],
            None,
        );
        assert_eq!(
            rx.recv().await.expect("active dispatch").text,
            "active turn"
        );
        hub.set_status("force-turn-end-first", Status::Busy, None);
        hub.submit(
            "force-turn-end-first",
            "old queue head".to_owned(),
            vec![],
            None,
        );
        assert!(hub.force_submit(
            "force-turn-end-first",
            "forced prompt".to_owned(),
            vec![],
            None,
            true,
        ));
        assert_eq!(
            queue_texts(&hub, "force-turn-end-first"),
            vec!["forced prompt", "old queue head"]
        );

        hub.complete_turn("force-turn-end-first");
        hub.set_status("force-turn-end-first", Status::Running, None);
        assert_eq!(
            rx.recv().await.expect("forced dispatch").text,
            "forced prompt"
        );

        assert!(rx.try_recv().is_err(), "old queue head must remain parked");
        assert_eq!(
            queue_texts(&hub, "force-turn-end-first"),
            vec!["old queue head"]
        );
        assert!(hub.session_has_in_flight_prompt("force-turn-end-first"));

        hub.set_status("force-turn-end-first", Status::Busy, None);
        hub.complete_turn("force-turn-end-first");
        hub.set_status("force-turn-end-first", Status::Running, None);
        assert_eq!(
            rx.recv().await.expect("old head dispatch").text,
            "old queue head"
        );
        assert!(queue_texts(&hub, "force-turn-end-first").is_empty());
    }

    #[tokio::test]
    async fn idle_status_then_turn_end_dispatches_only_the_force_pushed_prompt() {
        let hub = hub_with_session("force-idle-first");
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("force-idle-first", Status::Running, None);

        hub.submit("force-idle-first", "active turn".to_owned(), vec![], None);
        assert_eq!(
            rx.recv().await.expect("active dispatch").text,
            "active turn"
        );
        hub.set_status("force-idle-first", Status::Busy, None);
        hub.submit(
            "force-idle-first",
            "old queue head".to_owned(),
            vec![],
            None,
        );
        assert!(hub.force_submit(
            "force-idle-first",
            "forced prompt".to_owned(),
            vec![],
            None,
            true,
        ));

        hub.set_status("force-idle-first", Status::Running, None);
        assert_eq!(
            rx.recv().await.expect("forced dispatch").text,
            "forced prompt"
        );
        hub.complete_turn("force-idle-first");

        assert!(rx.try_recv().is_err(), "old queue head must remain parked");
        assert_eq!(
            queue_texts(&hub, "force-idle-first"),
            vec!["old queue head"]
        );
        assert!(hub.session_has_in_flight_prompt("force-idle-first"));

        hub.set_status("force-idle-first", Status::Busy, None);
        hub.set_status("force-idle-first", Status::Running, None);
        assert_eq!(
            rx.recv().await.expect("old head dispatch").text,
            "old queue head"
        );
        hub.complete_turn("force-idle-first");
        assert!(
            rx.try_recv().is_err(),
            "duplicate turn end must stay a no-op"
        );
        assert!(queue_texts(&hub, "force-idle-first").is_empty());
    }

    #[tokio::test]
    async fn authoritative_runtime_idle_releases_missed_turn_lifecycle_guard() {
        let hub = hub_with_session("runtime-reconnect");
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("runtime-reconnect", Status::Running, None);

        hub.submit(
            "runtime-reconnect",
            "lost lifecycle turn".to_owned(),
            vec![],
            None,
        );
        let first = rx.recv().await.expect("first dispatch");
        assert_eq!(first.text, "lost lifecycle turn");
        hub.submit("runtime-reconnect", "queued turn".to_owned(), vec![], None);
        assert_eq!(queue_texts(&hub, "runtime-reconnect"), vec!["queued turn"]);

        let guard_epoch = hub
            .in_flight_prompt_epoch("runtime-reconnect")
            .expect("stale guard epoch");
        hub.reconcile_runtime_idle("runtime-reconnect", guard_epoch);

        let dispatched = rx
            .recv()
            .await
            .expect("queued dispatch after runtime reconciliation");
        assert_eq!(dispatched.text, "queued turn");
        assert!(queue_texts(&hub, "runtime-reconnect").is_empty());
    }

    #[test]
    fn truncated_hot_tail_conservatively_preserves_native_thread() {
        let hub = hub_with_session("truncated-native-history");
        hub.set_agent_session_id("truncated-native-history", "thread-existing".to_owned());
        {
            let mut sessions = hub.inner.sessions.lock();
            let session = sessions
                .get_mut("truncated-native-history")
                .expect("session");
            session.reached_start = false;
        }

        assert_eq!(
            hub.agent_session_id_for_resume("truncated-native-history")
                .as_deref(),
            Some("thread-existing")
        );
    }

    #[test]
    fn idle_failed_sessions_recover_without_switching_runtime_generation() {
        let hub = Hub::new();
        hub.create_session(SessionRegistration {
            id: "auth-refresh".to_owned(),
            provider: "grok".to_owned(),
            provider_version: "1.1.8".to_owned(),
            provider_generation_digest: "sha256:provider".to_owned(),
            provider_auth_generation: Some(2),
            provider_behavior: None,
            machine_id: "hawk".to_owned(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: "/tmp".to_owned(),
            title: "test".to_owned(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
        hub.set_status(
            "auth-refresh",
            Status::Crashed,
            Some("login required".to_owned()),
        );
        let crashed_revision = hub.status_revision("auth-refresh").unwrap();
        hub.set_status(
            "auth-refresh",
            Status::Crashed,
            Some("new crash edge".to_owned()),
        );
        assert!(
            !hub.begin_provider_auth_recovery(
                "auth-refresh",
                crashed_revision,
                "login required",
                2,
                3,
            )
            .expect("stale lifecycle edge")
        );
        let crashed_revision = hub.status_revision("auth-refresh").unwrap();

        assert!(
            hub.begin_provider_auth_recovery(
                "auth-refresh",
                crashed_revision,
                "new crash edge",
                2,
                3,
            )
            .expect("safe recovery")
        );
        assert_eq!(
            hub.session_info("auth-refresh")
                .unwrap()
                .meta
                .provider_auth_generation,
            Some(2),
            "the original runtime generation owns the native rollout"
        );
        assert_eq!(hub.status("auth-refresh"), Some(Status::Starting));

        hub.set_agent_session_id("auth-refresh", "native-thread".to_owned());
        hub.set_status(
            "auth-refresh",
            Status::Crashed,
            Some("second login failure".to_owned()),
        );
        hub.set_status("auth-refresh", Status::Running, None);
        let idle_revision = hub.status_revision("auth-refresh").unwrap();
        assert!(
            hub.begin_provider_auth_recovery(
                "auth-refresh",
                idle_revision,
                "second login failure",
                2,
                4,
            )
            .expect("idle native thread follows explicit reauthorization")
        );
        assert_eq!(
            hub.session_info("auth-refresh")
                .unwrap()
                .meta
                .provider_auth_generation,
            Some(2)
        );
        assert_eq!(
            hub.session_info("auth-refresh")
                .unwrap()
                .meta
                .agent_session_id
                .as_deref(),
            Some("native-thread")
        );

        hub.set_status("auth-refresh", Status::Exited, None);
        let exited_revision = hub.status_revision("auth-refresh").unwrap();
        assert!(
            !hub.begin_provider_auth_recovery(
                "auth-refresh",
                exited_revision,
                "second login failure",
                2,
                5,
            )
            .expect("an explicitly exited session remains stopped")
        );
        hub.set_status(
            "auth-refresh",
            Status::Crashed,
            Some("third login failure".to_owned()),
        );
        hub.set_status("auth-refresh", Status::Running, None);

        hub.push(
            "auth-refresh",
            Event::TurnEnd {
                stop_reason: "end_turn".to_owned(),
            },
        );
        let completed_revision = hub.status_revision("auth-refresh").unwrap();
        assert!(
            !hub.begin_provider_auth_recovery(
                "auth-refresh",
                completed_revision,
                "third login failure",
                2,
                5,
            )
            .expect("a completed turn clears the unresolved failure")
        );
    }

    #[test]
    fn newer_crash_without_detail_preserves_an_older_auth_failure() {
        let hub = hub_with_session("newer-detail-less-crash");
        hub.set_status(
            "newer-detail-less-crash",
            Status::Crashed,
            Some("login required".to_owned()),
        );
        hub.set_status("newer-detail-less-crash", Status::Crashed, None);

        assert_eq!(
            hub.unresolved_crash_detail("newer-detail-less-crash")
                .as_deref(),
            Some("login required")
        );
    }

    #[tokio::test]
    async fn critical_persistence_waits_behind_a_full_event_queue() {
        let health = Arc::new(PersistenceHealth::default());
        let (sink, mut rx) = StoreSink::channel(1, Arc::clone(&health));
        assert!(sink.send(StoreWrite::AppendEvent(Envelope {
            session_id: "s".to_owned(),
            seq: 1,
            event: Event::TurnEnd {
                stop_reason: "done".to_owned(),
            },
            cmid: None,
        })));
        assert!(sink.send(StoreWrite::UpdateTitle {
            session_id: "s".to_owned(),
            title: "durable".to_owned(),
        }));
        assert!(matches!(rx.recv().await, Some(StoreWrite::AppendEvent(_))));
        assert!(matches!(
            rx.recv().await,
            Some(StoreWrite::UpdateTitle { .. })
        ));
        assert_eq!(health.dropped(), 0);
        assert_eq!(health.pending(), 0);
    }

    #[tokio::test]
    async fn session_broadcast_errors_are_persisted_outside_the_transcript() {
        let health = Arc::new(PersistenceHealth::default());
        let (sink, mut rx) = StoreSink::channel(4, health);
        let hub = Hub::with_store(Some(sink));
        let mut live = hub.subscribe();

        hub.broadcast_error(
            Some("session-1".to_owned()),
            "runtime rejected command".to_owned(),
        );

        let Some(StoreWrite::RecordSessionError {
            id,
            session_id,
            message,
            ..
        }) = rx.recv().await
        else {
            panic!("expected a durable session error");
        };
        assert!(id.starts_with("session-error:session-1:"));
        assert_eq!(session_id, "session-1");
        assert_eq!(message, "runtime rejected command");
        let frame = live.recv().await.expect("error frame");
        assert!(matches!(
            &**frame,
            Outbound::Error {
                session_id: Some(session_id),
                message,
            } if session_id == "session-1" && message == "runtime rejected command"
        ));
        assert!(hub.snapshot("session-1").is_none());

        hub.broadcast_error(Some("session-1".to_owned()), "你".repeat(2_000));
        let Some(StoreWrite::RecordSessionError { message, .. }) = rx.recv().await else {
            panic!("expected a bounded unicode error");
        };
        assert!(message.len() <= 4 * 1024);
        assert_eq!(message, "你".repeat(message.chars().count()));
    }
}
