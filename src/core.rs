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

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

use crate::persistence::EventReducer;
use crate::runtime_wire::{WorkerSnapshot, WorkerState};

mod code_scope;
mod review_defaults;
mod session_continuation;
mod session_fork;
pub use session_fork::{FORK_ORIGIN_UPDATE, ForkPoint, ForkRecord, ForkState};
mod hub_config;
mod hub_events;
mod hub_provider_pins;
mod hub_queue;
mod hub_superagent;
mod hub_sync;
mod session_attention;
pub use session_attention::SessionAttention;
mod session_trash;
pub use code_scope::{CodeReadScope, SessionCodeScope};
pub use session_trash::TRASHED_SESSION_REFUSAL;
mod product_permissions;
pub use product_permissions::ProductPermissionObservation;
mod persistence_queue;
pub use persistence_queue::NORMAL_BYTES as STORE_BATCH_MAX_BYTES;
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
pub const HOT_TAIL_MAX_BYTES: usize = 1024 * 1024;
/// Idle sessions keep a smaller replay tail. Opening one pages older rows
/// through `/api/history`; a busy turn keeps the full 1 MiB so the focused
/// transcript does not stall mid-stream.
pub const HOT_TAIL_IDLE_MAX_BYTES: usize = 512 * 1024;
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
pub fn estimated_envelope_bytes(envelope: &Envelope) -> usize {
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

pub fn hot_tail_budget_bytes(status: Status) -> usize {
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
pub fn bound_restored_hot_log(log: &mut Vec<Envelope>) -> bool {
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

/// Whether `request_id`'s latest request already has its answer in `log`.
/// Walks back to the nearest row about that id: a resolution means answered,
/// the request itself (or no row at all) means this answer is the first.
fn permission_already_resolved<'a>(
    log: impl DoubleEndedIterator<Item = &'a Event>,
    request_id: &str,
) -> bool {
    log.rev()
        .find_map(|event| match event {
            Event::PermissionResolved {
                request_id: seen, ..
            } if seen == request_id => Some(true),
            Event::PermissionRequest {
                request_id: seen, ..
            } if seen == request_id => Some(false),
            _ => None,
        })
        .unwrap_or(false)
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
pub const MODEL_REFUSAL_DETAIL: &str = "The model declined this request. Review its message before editing the request, changing models, or starting a new session. Queued messages require an explicit send.";

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
        if matches!(&envelope.event, Event::Update { update } if session_fork::is_fork_origin(update))
        {
            // A forked native thread holds the copied conversation.
            return true;
        }
        if is_context_cleared(envelope) {
            return false;
        }
    }
    !session.reached_start
}

/// Whether the session may move to another Provider version: its current
/// context holds a saved native session to resume, or nothing to lose (never
/// prompted, or cleared), so the next turn starts a fresh native session on
/// either release.
fn provider_version_portable(session: &Session) -> bool {
    !current_context_has_user_message(session) || session.meta.agent_session_id.is_some()
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

pub fn question_summary_title(text: &str, ordinal: u64) -> String {
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
mod question_summary_title_tests;

pub fn bound_history_page(mut events: Vec<Envelope>) -> Vec<Envelope> {
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
    pub generated_images: Vec<serde_json::Value>,
    pub usage: Option<crate::agent_model::TurnUsage>,
    pub status: Status,
}

/// The LAST user prompt in a session's log, rebuilt for a retry.
#[derive(Debug, Default, PartialEq, Eq)]
struct LastUserPrompt {
    /// Concatenated text of the prompt's text blocks.
    text: String,
    /// The complete block list when the prompt carried anything other than
    /// text (images, resources); empty for a text-only prompt, exactly like a
    /// plain text send.
    content: Vec<serde_json::Value>,
    /// The originating client's cmid when the echo still carries it (live
    /// envelopes only; the tag is not persisted).
    cmid: Option<String>,
}

/// Rebuild the LAST user prompt in a session's log — the turn cut off by a
/// crash or restart — from its `user_message_chunk` group. The ACP worker echoes
/// one chunk per content block, so the group is the maximal run of consecutive
/// user chunks ending at the last one. Degrades to empty values.
fn last_user_prompt(log: &[Envelope]) -> LastUserPrompt {
    let user_block = |env: &Envelope| -> Option<serde_json::Value> {
        let Event::Update { update } = &env.event else {
            return None;
        };
        (update
            .get("sessionUpdate")
            .and_then(serde_json::Value::as_str)
            == Some("user_message_chunk"))
        .then(|| {
            update
                .get("content")
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        })
    };
    let Some(last) = log.iter().rposition(|env| user_block(env).is_some()) else {
        return LastUserPrompt::default();
    };
    let start = log[..last]
        .iter()
        .rposition(|env| user_block(env).is_none())
        .map_or(0, |index| index + 1);
    let blocks: Vec<serde_json::Value> = log[start..=last].iter().filter_map(user_block).collect();
    let text: String = blocks
        .iter()
        .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
        .collect();
    let text_only = blocks.iter().all(|block| {
        block
            .get("type")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|kind| kind == "text")
    });
    LastUserPrompt {
        text,
        content: if text_only { Vec::new() } else { blocks },
        cmid: log[start..=last].iter().find_map(|env| env.cmid.clone()),
    }
}

/// `(type, mimeType)` of every block that is not text: the identity of an
/// attachment-only prompt, whose text is empty and whose image bytes may live
/// inline in one copy and behind an artifact URL in another.
fn attachment_signature(content: &[serde_json::Value]) -> Vec<(String, String)> {
    content
        .iter()
        .filter(|block| {
            block
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind != "text")
        })
        .map(|block| {
            let field = |name: &str| {
                block
                    .get(name)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            (field("type"), field("mimeType"))
        })
        .collect()
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
    /// The idle policy no longer restarts this long-unused session for new
    /// releases; it adopts the installed one when next opened.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on_open: bool,
}

/// Controller-held offer behind [`SessionMeta::provider_update_available`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderUpdateOffer {
    pub version: String,
    pub digest: String,
    pub when_idle: bool,
    /// Idle period after which a policy updates the session unattended.
    pub automatic_after: Option<std::time::Duration>,
    /// See [`ProviderUpdateAvailable::on_open`].
    pub on_open: bool,
}

/// Session metadata for the list view (no event log).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    #[serde(default, skip_serializing_if = "SessionAttention::is_empty")]
    pub attention: SessionAttention,
    pub id: String,
    pub provider: String,
    /// The session this one continues as a new agent. Persisted as the
    /// `session_continued_from:` setting and restored onto the projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continued_from: Option<String>,
    /// The session this one forks natively. Persisted in the `session_fork:`
    /// setting and restored onto the projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<String>,
    /// Resolved once at creation; independent of runtime Machine and auth generation.
    #[serde(default)]
    pub provider_account_id: crate::provider_account::ProviderAccountId,
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
    /// still overrides it. Persisted as a UI-owned setting; `serde(default)`
    /// covers older clients and sessions with no explicit pause.
    #[serde(default)]
    pub paused: bool,
    /// True while an accepted deletion waits for the session's execution
    /// environment to confirm it stopped. Clients treat it as the deletion
    /// acknowledgement; the row disappears on confirmation or returns with a
    /// Crashed detail when the stop stays unconfirmed. Transient.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closing: bool,
    /// Epoch ms when the session was moved to Trash, `None` while it is live.
    /// A trashed session keeps its transcript, worktree and native thread but
    /// accepts no work and never launches a worker until it is restored; the
    /// Controller deletes it permanently once the owner's retention elapses
    /// (`docs/sessions-folders.md#trash`). Persisted as `sessions.trashed_at_ms`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trashed_at_ms: Option<i64>,
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
    /// Live worker observation. Storage restores this as false until the
    /// owning runtime reports its capabilities again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub supports_current_turn: bool,
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
    /// Why an explicitly sent message remains in the durable queue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_wait: Option<String>,
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

/// Controller-verified proof that one named session's login stays valid on
/// another release: the target's signed authentication is identical to the
/// release the login signed in under (`provider_account::native_release_compatible`),
/// exactly as new-session admission carries a login over. It binds the
/// session, its login generation and the exact target release. Runtime
/// admission (Store and Machine) still rechecks the login before any launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedReleaseCarry {
    pub session_id: String,
    pub auth_generation: u64,
    pub version: String,
    pub digest: String,
}

impl SessionMeta {
    fn require_provider_release_transition(
        &self,
        version: &str,
        digest: &str,
        carry: Option<&NamedReleaseCarry>,
    ) -> Result<(), String> {
        if !self.provider_account_id.is_default()
            && (self.provider_version != version || self.provider_generation_digest != digest)
            && !carry.is_some_and(|carry| {
                carry.session_id == self.id
                    && self.provider_auth_generation == Some(carry.auth_generation)
                    && carry.version == version
                    && carry.digest == digest
            })
        {
            return Err("named account release change requires a verified login transition".into());
        }
        Ok(())
    }

    /// Web creation registers a remote session against the stable source root
    /// and only later publishes the Machine-prepared session worktree as its
    /// `cwd`. A non-Git root legitimately stays shared, so this structural
    /// check alone does not prove the workspace is unprepared.
    #[must_use]
    pub fn on_workspace_source_root(&self) -> bool {
        self.origin == SessionOrigin::Web
            && self.machine_id != "local"
            && self.execution_binding.is_none()
            && self.workspace_source_path.as_deref() == Some(self.cwd.as_str())
    }

    /// A session whose Machine workspace preparation is still pending, was
    /// interrupted (Controller restart, dropped Machine connection) or failed
    /// (for example a Git timeout): it is on its source root and no native
    /// agent thread ever started. Callers that can observe a live worker must
    /// also exclude one, which covers the first launch on a shared non-Git
    /// root. Its source checkout is never a valid worker cwd for a Git root,
    /// and re-running preparation is idempotent for both kinds.
    #[must_use]
    pub fn awaits_workspace_preparation(&self) -> bool {
        self.on_workspace_source_root()
            && self.agent_session_id.is_none()
            && matches!(
                self.status,
                crate::agent_model::Status::Starting | crate::agent_model::Status::Crashed
            )
    }

    /// Binding identity alone cannot authorize execution. The selected exact
    /// signed Provider must also accept this executor contract. This is a
    /// structural check only: named dispatch and projection require the fresh
    /// Store admission enforced by RemoteRuntime, never this cached metadata.
    pub fn require_runtime_launch(&self) -> Result<(), String> {
        if !self.provider_account_id.is_default()
            && (self.closing
                || self.provider != "claude-code"
                || self.machine_id.is_empty()
                || self.machine_id == "local"
                || self.owner_user_id.as_ref().is_none_or(String::is_empty)
                || self.provider_version.is_empty()
                || self.provider_generation_digest.is_empty()
                || self
                    .provider_auth_generation
                    .is_none_or(|generation| generation == 0)
                || self.provider_behavior.is_none())
        {
            return Err("Named account requires a complete enrolled native binding".into());
        }
        self.require_runtime_environment()
    }

    /// Structural check only. The caller must separately hold current native
    /// account/session admission before dispatching this dormant declaration.
    pub fn require_native_readmission(
        &self,
        target: &crate::provider_account::NativeProfileReceipt,
    ) -> Result<(), String> {
        if self.provider_account_id.is_default()
            || self.closing
            || self.status != Status::Exited
            || self.owner_user_id.is_none()
            || self.agent_session_id.is_some()
            || self.provider_account_id != target.account_id
            || self.provider != target.provider_id
            || self.provider_version != target.provider_version
            || self.provider_generation_digest != target.generation_digest
            || self.provider_auth_generation != Some(target.generation)
            || target.generation == 0
        {
            return Err("native readmission does not match the dormant session".into());
        }
        self.require_runtime_environment()
    }

    fn require_runtime_environment(&self) -> Result<(), String> {
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

fn display_config_options(session: &Session) -> Option<serde_json::Value> {
    if let Some(options) = &session.config_options {
        return Some(options.clone());
    }
    let mut options = session.initial_config_options.clone()?;
    if let Some(preferences) = session.config_preferences.as_object() {
        for (id, value) in preferences {
            set_config_option_current_value(&mut options, id, value);
        }
    }
    Some(options)
}

/// Immutable attributes assigned when a Cowboy session is registered.
pub struct SessionRegistration {
    pub id: String,
    pub provider: String,
    pub provider_account_id: crate::provider_account::ProviderAccountId,
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
    /// Held across the channel and async preflight until runtime handoff.
    pub ownership: std::sync::Arc<DispatchOwnership>,
    pub session_id: String,
    pub text: String,
    pub content: Vec<serde_json::Value>,
    /// cmid of the originating submit (chat send) — carried so the agent's
    /// user-message echo can be tagged for optimistic reconcile. None for a
    /// drained queue item (its optimistic row already reconciled via `queues`).
    pub cmid: Option<String>,
    /// A user-requested send (chat submit, send now, force push). If it is
    /// returned for a transient resource wait, maintenance retries it once the
    /// resource returns even if the queue is paused or the session crashed.
    pub explicit: bool,
}

pub struct DispatchOwnership {
    hub: std::sync::Weak<HubInner>,
    session: String,
    epoch: u64,
    supervision: Option<crate::superagent::DispatchContext>,
}

impl std::fmt::Debug for DispatchOwnership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchOwnership")
            .field("session", &self.session)
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}

impl DispatchOwnership {
    fn new(hub: &Hub, session: &str, epoch: u64) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            hub: std::sync::Arc::downgrade(&hub.inner),
            session: session.into(),
            epoch,
            supervision: None,
        })
    }
}

impl Drop for DispatchOwnership {
    fn drop(&mut self) {
        if let Some(hub) = self.hub.upgrade()
            && let Some(session) = hub.sessions.lock().get_mut(&self.session)
            && session.dispatch_pending == Some(self.epoch)
        {
            session.dispatch_pending = None;
        }
    }
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
    /// Wall-clock epoch ms of the newest durable event that counts as
    /// activity, so idle-based policies keep measuring across restarts.
    pub last_event_at_ms: Option<i64>,
}

/// ACP updates a worker emits on its own whenever it starts or reconnects.
/// They, like lifecycle edges, say nothing about whether anyone used the
/// session, so they do not reset its idle clock.
const NON_ACTIVITY_SESSION_UPDATES: [&str; 5] = [
    "available_commands_update",
    "current_mode_update",
    "config_option_update",
    "usage_update",
    "session_info_update",
];

/// [`NON_ACTIVITY_SESSION_UPDATES`] as a SQL list for the restore queries.
pub const NON_ACTIVITY_SESSION_UPDATES_SQL: &str = "'available_commands_update', \
'current_mode_update', 'config_option_update', 'usage_update', 'session_info_update'";

/// Whether an event shows the session being used: prompts, agent output,
/// tool calls and permissions. Lifecycle edges and worker metadata replayed
/// by a restart or reconnect are not.
pub fn counts_as_activity(event: &Event) -> bool {
    match event {
        Event::Lifecycle { .. } => false,
        Event::Update { update } => !update
            .get("sessionUpdate")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| NON_ACTIVITY_SESSION_UPDATES.contains(&kind)),
        _ => true,
    }
}

/// When a restored session was last active, from its newest durable event.
/// Without a record (or with a clock in the future) it counts as just active,
/// the conservative choice for every idle-based policy.
fn restored_last_activity(last_event_at_ms: Option<i64>, now_ms: i64) -> std::time::Instant {
    let now = std::time::Instant::now();
    last_event_at_ms
        .and_then(|at| u64::try_from(now_ms.saturating_sub(at)).ok())
        .and_then(|idle| now.checked_sub(std::time::Duration::from_millis(idle)))
        .unwrap_or(now)
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
    epoch: u64,
    echoed: bool,
}

/// Per-session state: metadata + the seq-ordered event log.
struct Session {
    pending_actions: session_attention::PendingActions,
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
    /// When the session last showed activity (see [`counts_as_activity`]).
    /// Monotonic; a restart restores it from the newest such durable event,
    /// so idle-based policies do not restart their wait on every deploy.
    last_activity: std::time::Instant,
    /// Last seen agent-advertised config options (raw ACP
    /// `configOptions` array — see acp.rs intercept). `None` until the agent
    /// fires its first `config_option_update` notification. Re-sent to every
    /// new client on connect so the composer dropdowns populate from a fresh
    /// reload.
    config_options: Option<serde_json::Value>,
    /// Signed defaults/presets before the first ACP snapshot. A preview is
    /// never persisted or treated as an agent acknowledgement.
    initial_config_options: Option<serde_json::Value>,
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
    /// A durable interrupt/send admission owns automatic queue dispatch until
    /// its outcome is known. Unknown effects survive restart without replay.
    delivery_hold: Option<String>,
    /// The queue head is an explicit send returned for a transient resource
    /// wait. Maintenance redelivers it as that explicit send; any dispatch of
    /// the head consumes the intent.
    explicit_wait: bool,
    /// A channel/preflight-owned prompt has not reached its runtime yet.
    dispatch_pending: Option<u64>,
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
    /// The Machine rejected the last launch because the pinned Provider
    /// generation has no directory there any more (retired under a surviving
    /// worker, or never installed). The Provider update pass moves the session
    /// to the installed release; nothing else can open it again. Transient.
    provider_generation_missing: bool,
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
const MOBILE_REVIEW_BINDING_CAP: usize = 16;
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
    /// Reopening the active path is still a new navigation intent.
    #[serde(default)]
    open_revision: u64,
    #[serde(default)]
    reveal_revision: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::review_directory::deserialize_override"
    )]
    directory: Option<Option<String>>,
    // Preserve opaque settings when an older Controller reads and rewrites
    // state produced by a newer release. Mutations remain a closed protocol.
    #[serde(default, flatten)]
    future: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_review: Option<crate::workspace_extensions::ReviewBinding>,
    #[serde(default)]
    remote_selected: bool,
    /// The chosen comparison: `""` is uncommitted work, otherwise a Git
    /// reference. Absent means the repository's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    comparison: Option<String>,
    /// Code contexts this session also works in, as `review_directory` values.
    /// A binding names an existing context and grants no file access.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    bindings: Vec<String>,
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
            open_revision: 0,
            reveal_revision: 0,
            directory: None,
            future: std::collections::BTreeMap::new(),
            remote_review: None,
            remote_selected: false,
            comparison: None,
            bindings: Vec::new(),
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
            .directory
            .as_ref()
            .and_then(Option::as_deref)
            .is_some_and(|value| !crate::review_directory::valid(value))
        {
            state.directory = None;
        }
        if state
            .remote_review
            .as_ref()
            .is_some_and(|binding| !binding.valid())
        {
            state.remote_review = None;
            state.remote_selected = false;
        }
        if state
            .comparison
            .as_deref()
            .is_some_and(|value| !valid_mobile_review_comparison(value))
        {
            state.comparison = None;
        }
        let mut bound = HashSet::new();
        state.bindings.retain(|context| {
            valid_mobile_review_binding(context) && bound.insert(context.clone())
        });
        state.bindings.truncate(MOBILE_REVIEW_BINDING_CAP);
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

fn valid_mobile_review_binding(context: &str) -> bool {
    context != "project" && crate::review_directory::valid(context)
}

fn valid_mobile_review_comparison(value: &str) -> bool {
    let reference = value.strip_prefix("committed:").unwrap_or(value);
    value.is_empty()
        || (reference.starts_with("refs/")
            && value.len() <= 512
            && !value.chars().any(char::is_control))
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
    /// Stop one ACP subagent of the session (its child session
    /// `subagent_session_id`) while the turn and other subagents continue.
    CancelSubagent {
        session_id: String,
        subagent_session_id: String,
    },
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
    /// Move sessions and folders to Trash (`docs/sessions-folders.md#trash`).
    /// A folder takes everything filed in it. All-or-nothing: a working
    /// session refuses the whole request.
    TrashItems {
        #[serde(default)]
        session_ids: Vec<String>,
        #[serde(default)]
        folder_ids: Vec<String>,
    },
    /// Restore trashed sessions and folders to where they were.
    RestoreItems {
        #[serde(default)]
        session_ids: Vec<String>,
        #[serde(default)]
        folder_ids: Vec<String>,
    },
    /// Permanently delete trashed sessions and folders now, before their
    /// retention ends. Live items are refused; use `trash_items` first.
    PurgeItems {
        #[serde(default)]
        session_ids: Vec<String>,
        #[serde(default)]
        folder_ids: Vec<String>,
    },
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
    /// is the CLIENT's job — Claude/Codex expose no `clear` agent command
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
    /// Owner-scoped invalidation of native account observations and progress.
    /// Clients share one cached read; no credentials enter this notification.
    AccountUsageChanged { owner_user_id: String },
    /// Agent calls a session's agent is waiting for a person to approve, or
    /// `None` once nothing waits. Re-sent while the agent keeps asking, so a
    /// reconnecting client recovers it; it expires after `ttl_ms` otherwise.
    CallApproval {
        session_id: String,
        #[serde(default)]
        approval: Option<serde_json::Value>,
    },
    /// An error to surface to the user (bad command, unknown session, ...).
    /// Broadcast to every connected client — cowboy's "one shared progress"
    /// design means any window watching the same session should see why a
    /// command was rejected, not just the originator.
    Error {
        /// Session the error belongs to; absent (never `null`) for daemon-level
        /// errors such as a malformed inbound frame or unknown session id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
    /// Record one notification occurrence (docs/task-notifications.md).
    RecordOccurrence(crate::task_notifications::Occurrence),
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
        calls_before_ms: i64,
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
    /// Persist the Trash state of sessions (`None` = restored).
    SetSessionsTrashed {
        session_ids: Vec<String>,
        trashed_at_ms: Option<i64>,
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

pub fn estimated_store_write_bytes(write: &StoreWrite) -> usize {
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

pub struct ContextEditGuard {
    hub: Hub,
    id: String,
}

impl Drop for ContextEditGuard {
    fn drop(&mut self) {
        self.hub.inner.context_edits.lock().remove(&self.id);
        self.hub.try_drain(&self.id);
    }
}

/// Unanswered client-set config values of one session, with when each was set.
type InFlightConfig = HashMap<String, (serde_json::Value, std::time::Instant)>;

struct HubInner {
    sessions: Mutex<HashMap<String, Session>>,
    context_edits: Mutex<HashSet<String>>,
    /// Internal auth/admin state restored from the durable settings table.
    settings: Mutex<HashMap<String, serde_json::Value>>,
    product_permissions: Mutex<product_permissions::Observations>,
    /// Persisted Busy sessions awaiting an authoritative, connected worker
    /// snapshot after the control plane restarts. Broker registry placeholders
    /// do not settle this set; a bounded server-side grace timer finalizes the
    /// remainder as genuine interruptions.
    runtime_reconciliation: Mutex<HashSet<String>>,
    /// Sessions marked Interrupted because their restored turn was absent
    /// from the reconnecting worker's snapshot.
    restart_interrupted: Mutex<HashSet<String>>,
    /// Config values a client set that the agent has not yet reported back,
    /// per session. A preset sends several options in a row and the agent
    /// answers each with a full snapshot; the answer to the first still
    /// carries the old values of the rest. Overlaying these keeps that stale
    /// snapshot from flipping the selection back. Lock after `sessions`.
    config_in_flight: Mutex<HashMap<String, InFlightConfig>>,
    /// Newer installed Provider releases sessions may adopt, republished by
    /// the Provider update pass. Never held together with `sessions`.
    provider_update_offers: Mutex<HashMap<String, ProviderUpdateOffer>>,
    /// When an unattended Provider update last left a session crashed, by
    /// (Machine, Provider, generation digest). Never held with `sessions`.
    automatic_update_failures: Mutex<HashMap<(String, String, String), std::time::Instant>>,
    /// When an unattended Provider update last left each session crashed.
    /// Never held with `sessions`.
    automatic_update_failed_sessions: Mutex<HashMap<String, std::time::Instant>>,
    /// Canonicalizes the raw ACP stream for the in-memory replay tail. The DB
    /// writer still reduces compact deltas so streaming text coalesces without
    /// enqueueing the accumulated string on every token.
    history_reducer: Mutex<EventReducer>,
    /// Latest subagent observations per session (docs/background-tasks.md):
    /// the bootstrap `subagent_state` and lookups read it beside the store.
    /// Never held with `sessions`.
    background: Mutex<HashMap<String, BackgroundLedger>>,
    /// Identifies this Controller process to clients. A new epoch tells a
    /// client to replace, never merge, its cached subagent ordering.
    boot_epoch: String,
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
    preference_announcements: Mutex<HashMap<String, serde_json::Map<String, serde_json::Value>>>,
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
    /// Serialize read/compare/apply and ordered write-behind snapshots.
    sync_apply_gate: Mutex<()>,
    review_default_receipts: Mutex<review_defaults::Receipts>,
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

pub fn config_option_accepts(option: &serde_json::Value, value: &serde_json::Value) -> bool {
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

/// Children of one session whose latest observation the Hub keeps for the
/// bootstrap watermark. Older terminal children fall back to the store.
const BACKGROUND_LATEST_PER_SESSION: usize = 512;

/// Per-session subagent observation state (docs/background-tasks.md I2–I5).
#[derive(Default)]
struct BackgroundLedger {
    /// Last accepted snapshot identity per child, for duplicate/stale fencing.
    marks: HashMap<String, crate::transcript_anchors::SnapshotMark>,
    /// Latest accepted snapshot per child with the Hub's anchor seq.
    latest: HashMap<String, (u64, serde_json::Value)>,
    /// The worker incarnation (Machine worker epoch) last seen for the session.
    incarnation: Option<String>,
}

/// Where an accepted subagent snapshot leaves its terminal occurrence.
#[derive(Debug, Clone, Copy)]
enum SubagentEnding {
    Working,
    /// A new terminal occurrence starts at this snapshot.
    Ended,
    /// Metadata of an occurrence that already ended, with its identity.
    AlreadyEnded(Option<u64>),
}

/// The Hub's half of a `subagent_state` bootstrap.
#[derive(Debug, Clone, Default)]
pub struct SubagentObservations {
    /// `(anchor seq, latest update)` per child observed by this process.
    pub items: Vec<(u64, serde_json::Value)>,
    /// Highest seq allocated for the session when read.
    pub watermark_seq: u64,
    pub incarnation: Option<String>,
    /// The Hub holds the whole transcript (no store): its view is complete.
    pub memory_only: bool,
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
                context_edits: Mutex::new(HashSet::new()),
                product_permissions: Mutex::new(product_permissions::Observations::default()),
                runtime_reconciliation: Mutex::new(HashSet::new()),
                restart_interrupted: Mutex::new(HashSet::new()),
                config_in_flight: Mutex::new(HashMap::new()),
                provider_update_offers: Mutex::new(HashMap::new()),
                automatic_update_failures: Mutex::new(HashMap::new()),
                automatic_update_failed_sessions: Mutex::new(HashMap::new()),
                history_reducer: Mutex::new(EventReducer::default()),
                background: Mutex::new(HashMap::new()),
                boot_epoch: {
                    let mut bytes = [0u8; 8];
                    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
                    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
                },
                artifacts: Mutex::new(None),
                order: Mutex::new(Vec::new()),
                folders: Mutex::new(crate::session_folders::SessionFolders::default()),
                workspace_orders: Mutex::new(HashMap::new()),
                draft_announcements: Mutex::new(HashMap::new()),
                preference_announcements: Mutex::new(HashMap::new()),
                tx,
                broadcast_last_bytes: AtomicUsize::new(0),
                store_tx,
                dispatch_tx: Mutex::new(None),
                scheduler_tx: Mutex::new(None),
                sync: Mutex::new(HashMap::new()),
                sync_apply_gate: Mutex::new(()),
                review_default_receipts: Mutex::new(review_defaults::Receipts::default()),
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

    /// Put the bytes back into image blocks whose history copy only holds an
    /// `/api/artifacts/…` URL ([`Self::push_tagged`] externalizes large images
    /// before the log keeps them). A retry resubmits history, and the worker
    /// needs inline `data`. Fails with the missing reference when an artifact
    /// is gone or no store is configured; blocks are left untouched then.
    fn restore_externalized_images(
        &self,
        content: &mut [serde_json::Value],
    ) -> std::result::Result<(), String> {
        let store = self.inner.artifacts.lock().clone();
        for block in content.iter_mut() {
            let Some(object) = block.as_object_mut() else {
                continue;
            };
            if object.get("type").and_then(serde_json::Value::as_str) != Some("image")
                || object.contains_key("data")
            {
                continue;
            }
            let Some(url) = object
                .get("url")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
            else {
                continue;
            };
            let Some(name) = url.strip_prefix("/api/artifacts/") else {
                continue;
            };
            let path = store
                .as_ref()
                .and_then(|store| store.path(name))
                .ok_or_else(|| url.clone())?;
            let bytes = std::fs::read(&path).map_err(|_| url.clone())?;
            let data = base64::engine::general_purpose::STANDARD.encode(bytes);
            object.remove("url");
            object.insert("data".to_owned(), serde_json::Value::String(data));
        }
        Ok(())
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

    /// Publish a session's pending agent-call approval to its viewers.
    pub fn broadcast_call_approval(&self, session_id: &str, approval: Option<serde_json::Value>) {
        self.fanout(Outbound::CallApproval {
            session_id: session_id.to_owned(),
            approval,
        });
    }

    /// Publish the current account usage snapshot to every client.
    pub fn broadcast_usage(&self, snapshot: serde_json::Value) {
        self.fanout(Outbound::Usage { snapshot });
    }

    pub fn broadcast_account_usage_changed(&self, owner: &str) {
        self.fanout(Outbound::AccountUsageChanged {
            owner_user_id: owner.to_owned(),
        });
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
    pub fn restore_workspace_orders(&self, orders: Vec<(String, Vec<String>)>) {
        *self.inner.workspace_orders.lock() = orders.into_iter().collect();
    }

    /// Announce one owner's changed draft `metadata` (no body) to that owner's
    /// devices. The live patch carries only this document; a reconnect resync
    /// carries every document announced this lifetime.
    pub fn announce_preferences(&self, owner: &str, scope: &str, revision: i64) {
        {
            let mut all = self.inner.preference_announcements.lock();
            let scopes = all.entry(owner.to_owned()).or_default();
            if scopes
                .get(scope)
                .and_then(serde_json::Value::as_i64)
                .is_some_and(|old| old >= revision)
            {
                return;
            }
            scopes.insert(scope.to_owned(), revision.into());
        }
        self.sync_emit(
            "preferences",
            serde_json::json!({owner:{scope:revision}}),
            Vec::new(),
        );
    }

    pub fn announce_draft(&self, owner: &str, metadata: serde_json::Value) {
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

    pub fn folder_defaults(
        &self,
        actor: &crate::session_folders::FolderActor,
        id: &str,
    ) -> Result<crate::session_folders::defaults::ResolvedDefaults, String> {
        self.inner.folders.lock().resolve_defaults(actor, id)
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
                    last_event_at_ms,
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
                meta.paused = self
                    .setting(&format!(
                        "{}{id}",
                        settings_keys::SESSION_QUEUE_PAUSED_PREFIX
                    ))
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
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
                let log_bytes = session_attention::prepare_restored_log(
                    &mut log,
                    meta.status,
                    self.inner.store_tx.is_some(),
                    &mut reached_start,
                );
                for envelope in &log {
                    let _ = history_reducer.reduce(envelope.clone());
                }
                sessions_lock.insert(
                    id.clone(),
                    Session {
                        pending_actions: session_attention::PendingActions::restored(
                            &log, runtime, &mut meta,
                        ),
                        meta,
                        code_incarnation: code_scope::CodeIncarnation::default(),
                        machine_lineage: None,
                        log,
                        log_bytes,
                        event_count,
                        reached_start,
                        next_seq,
                        last_activity: restored_last_activity(last_event_at_ms, now_ms()),
                        config_options,
                        initial_config_options: None,
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
                        delivery_hold: self
                            .setting(&format!(
                                "{}{id}",
                                settings_keys::SESSION_DELIVERY_HOLD_PREFIX
                            ))
                            .and_then(|value| value.as_str().map(str::to_owned)),
                        explicit_wait: self
                            .setting(&format!(
                                "{}{id}",
                                settings_keys::SESSION_RESOURCE_WAIT_PREFIX
                            ))
                            .and_then(|value| {
                                value.get("explicit").and_then(serde_json::Value::as_bool)
                            })
                            .unwrap_or(false),
                        dispatch_pending: None,
                        dispatched_cmids: VecDeque::new(),
                        turn_completion_latched: false,
                        provider_generation_missing: false,
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
    pub fn accepts_runtime_projection(&self, session_id: &str) -> bool {
        !self
            .inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|session| session.meta.require_runtime_launch().is_err())
    }

    /// A known session whose runtime Machine is not `machine_id`. Unknown IDs
    /// stay false: a Machine may hold workers for sessions the Hub has dropped.
    pub fn session_runs_elsewhere(&self, session_id: &str, machine_id: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|session| session.meta.machine_id != machine_id)
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
            (info.meta.execution_binding.is_some() || !info.meta.provider_account_id.is_default())
                && !worker.launch.as_ref().is_some_and(|launch| {
                    launch.execution_binding == info.meta.execution_binding
                        && launch.provider == info.meta.provider
                        && launch.provider_account_id == info.meta.provider_account_id
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
        // A reconnect restates the level of a worker that already crashed.
        // Appending it again would report the recorded crash as a new one.
        if status == Status::Crashed
            && self.status(session_id) == Some(Status::Crashed)
            && detail.as_deref().is_none_or(|detail| {
                self.latest_crash_detail(session_id).as_deref() == Some(detail)
            })
        {
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
        self.inner
            .restart_interrupted
            .lock()
            .insert(session_id.to_owned());
        self.set_status(
            session_id,
            Status::Interrupted,
            Some("turn cut off by a cowboy restart — it never finished".to_owned()),
        );
    }

    /// A worker snapshot reflects events the Controller may not have
    /// received yet. When the restored turn's own end is replayed after
    /// the snapshot, the turn did finish: undo the restart interruption
    /// instead of reporting a completed answer as cut off.
    pub fn settle_restart_interruption(&self, session_id: &str) {
        if self.inner.restart_interrupted.lock().remove(session_id)
            && self.status(session_id) == Some(Status::Interrupted)
        {
            self.set_status(session_id, Status::Running, None);
        }
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
    /// Keep the last assistant message, using native message identity where
    /// available. Providers without IDs retain the tool-boundary fallback.
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
            generated_images: Vec::new(),
            usage: None,
            status: s.meta.status,
        };
        let mut final_text = String::new();
        let mut message_id: Option<&str> = None;
        for entry in s.log.iter().filter(|entry| entry.seq > after_seq) {
            match &entry.event {
                Event::Update { update } => {
                    if crate::managed_calls::media::generated_update(update) {
                        turn.generated_images.push(update.clone());
                    }
                    match update
                        .get("sessionUpdate")
                        .and_then(serde_json::Value::as_str)
                    {
                        Some("user_message_chunk") => turn.prompted = true,
                        Some("agent_message_chunk") => {
                            let next_id =
                                update.get("messageId").and_then(serde_json::Value::as_str);
                            if let Some(next_id) = next_id {
                                if message_id != Some(next_id) {
                                    final_text.clear();
                                }
                                message_id = Some(next_id);
                            }
                            if let Some(text) = update
                                .get("content")
                                .and_then(|content| content.get("text"))
                                .and_then(serde_json::Value::as_str)
                            {
                                final_text.push_str(text);
                            }
                        }
                        // Advisory notices are not part of the reply and may
                        // arrive after it.
                        Some(
                            "agent_thought_chunk"
                            | "usage_update"
                            | "available_commands_update"
                            | "current_mode_update"
                            | "config_option_update"
                            | "notice",
                        )
                        | None => {}
                        Some(_) => {
                            final_text.clear();
                            message_id = None;
                        }
                    }
                }
                Event::TurnEnd { stop_reason, usage } => {
                    turn.stop_reason = Some(stop_reason.clone());
                    turn.usage = *usage;
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
            false,
        );
    }

    /// Queue the parent's notice that one of its opted-in managed calls ended.
    /// An idle parent starts a turn; a busy one sees it at its next turn. The
    /// call-scoped id makes a repeated notice a no-op.
    pub fn submit_call_notice(&self, session_id: &str, call_id: &str, text: String) {
        self.submit_inner(
            session_id,
            text,
            Vec::new(),
            Some(format!(
                "{}{call_id}",
                crate::agent_model::CALL_NOTICE_PREFIX
            )),
            false,
            false,
        );
    }

    pub fn is_managed_child(&self, session_id: &str) -> bool {
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
                                on_open: offer.on_open,
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

    /// Whether an unattended update to this exact release left a session on
    /// the Machine crashed within `window`. The update pass holds further
    /// unattended updates to it meanwhile, so one bad release cannot crash
    /// every idle session in turn.
    #[must_use]
    pub fn automatic_update_failed_within(
        &self,
        machine_id: &str,
        provider: &str,
        digest: &str,
        window: std::time::Duration,
    ) -> bool {
        self.inner
            .automatic_update_failures
            .lock()
            .get(&(
                machine_id.to_owned(),
                provider.to_owned(),
                digest.to_owned(),
            ))
            .is_some_and(|failed| failed.elapsed() < window)
    }

    /// Whether this session's last unattended Provider update left it crashed
    /// and nothing has happened in it since. The idle policy leaves such a
    /// session alone until it is used or opened, instead of retrying the same
    /// resume on every release while nobody is there to see it fail.
    #[must_use]
    pub fn automatic_update_failed_since_activity(&self, session_id: &str) -> bool {
        let failed_at = self
            .inner
            .automatic_update_failed_sessions
            .lock()
            .get(session_id)
            .copied();
        failed_at.is_some_and(|failed_at| {
            // Compare the recorded instants. Two elapsed() calls observe
            // different "now"s; a scheduling pause between them could lift
            // this hold even though no session activity occurred.
            self.inner
                .sessions
                .lock()
                .get(session_id)
                .is_some_and(|session| session.last_activity <= failed_at)
        })
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

    /// Runtime-owned prompt guard, excluding channel/preflight ownership.
    #[must_use]
    pub fn in_flight_prompt_epoch(&self, session_id: &str) -> Option<u64> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .filter(|session| session.in_flight && session.dispatch_pending.is_none())
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
    #[cfg(any(test, feature = "test-support"))]
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
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
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
        self.project_new_session(Self::prepare_session(registration), true);
    }

    /// Construct metadata without publishing it or enqueuing a database write.
    pub fn prepare_session(registration: SessionRegistration) -> SessionMeta {
        let SessionRegistration {
            id,
            provider,
            provider_account_id,
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
        SessionMeta {
            continued_from: None,
            forked_from: None,
            id: id.clone(),
            provider,
            provider_account_id,
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
            attention: Default::default(),
            background_tasks: 0,
            supports_current_turn: false,
            provider_update: None,
            provider_update_available: None,
            resource_wait: None,
            next_schedule_ms: None,
            owner_user_id,
            owner_username,
            trashed_at_ms: None,
        }
    }

    /// Only for a committed row, with current account admission held by caller.
    /// Refuse a collision rather than replacing live transcript/runtime state.
    pub fn project_persisted_session(&self, meta: SessionMeta) -> bool {
        self.project_new_session(meta, false)
    }

    fn project_new_session(&self, meta: SessionMeta, persist: bool) -> bool {
        let id = meta.id.clone();
        // Managed children take only their parent's preset.
        let config_preferences = if meta
            .execution_binding
            .as_ref()
            .is_some_and(|binding| binding.managed().is_some())
        {
            serde_json::json!({})
        } else {
            default_config_preferences(&meta.provider, meta.provider_behavior.as_ref())
        };
        {
            let mut sessions = self.inner.sessions.lock();
            if sessions.contains_key(&id) {
                return false;
            }
            let mut order = self.inner.order.lock();
            sessions.insert(
                id.clone(),
                Session {
                    pending_actions: session_attention::PendingActions::default(),
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
                    initial_config_options: None,
                    config_preferences: config_preferences.clone(),
                    queue: Vec::new(),
                    drafts: Vec::new(),
                    editing: None,
                    editing_epoch: 0,
                    in_flight: false,
                    in_flight_epoch: 0,
                    delivery_hold: None,
                    explicit_wait: false,
                    dispatch_pending: None,
                    dispatched_cmids: VecDeque::new(),
                    turn_completion_latched: false,
                    provider_generation_missing: false,
                    lifecycle_epoch: 0,
                    mobile_review: MobileReviewState::default(),
                },
            );
            order.push(id.clone());
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            if persist {
                let _ = tx.send(StoreWrite::InsertSession(Box::new(meta)));
            }
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
        true
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
            self.inner.background.lock().remove(session_id);
        }
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
            for prefix in [
                settings_keys::SESSION_RESOURCE_WAIT_PREFIX,
                settings_keys::SESSION_QUEUE_PAUSED_PREFIX,
                settings_keys::SESSION_DELIVERY_POLICY_PREFIX,
                settings_keys::SESSION_DELIVERY_HOLD_PREFIX,
                settings_keys::SESSION_CONTEXT_PREFIX,
                settings_keys::SESSION_CONTEXT_CARRIER_PREFIX,
                settings_keys::SESSION_CONTINUED_FROM_PREFIX,
                settings_keys::SESSION_FORK_PREFIX,
                settings_keys::SESSION_CONFIG_REVERT_PREFIX,
            ] {
                let key = format!("{prefix}{session_id}");
                if self.setting(&key).is_some() {
                    self.set_setting(key, serde_json::Value::Null);
                }
            }
            self.broadcast_sessions();
        }
        removed
    }

    /// Rename a session. Updates the in-memory `title`, persists, and
    /// re-broadcasts the session list so every connected surface sees the
    /// new label. Unknown ids are silently ignored (matches `set_status`).
    /// Mark `session_id` as continuing `source` (or restore that mark).
    pub fn set_continued_from(&self, session_id: &str, source: Option<String>) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if s.meta.continued_from == source {
                return;
            }
            s.meta.continued_from = source;
        }
        self.broadcast_sessions();
    }

    /// Project persisted continuation marks after settings and sessions load.
    pub fn restore_continuations(&self) {
        let sessions = self.session_list();
        let key = |id: &str| format!("{}{id}", settings_keys::SESSION_CONTINUED_FROM_PREFIX);
        // Continuations created before the mark existed are named by their
        // source's accepted send receipt. Record them once, so a later send
        // from that source cannot erase the relationship.
        for source in &sessions {
            let Some(delivery) = crate::session_context::read(self, &source.id)
                .ok()
                .and_then(|context| context.delivery)
                .filter(|delivery| {
                    delivery.state == "accepted" && delivery.session_id != source.id
                })
            else {
                continue;
            };
            if sessions.iter().any(|meta| meta.id == delivery.session_id)
                && self.setting(&key(&delivery.session_id)).is_none()
            {
                self.set_setting(key(&delivery.session_id), serde_json::json!(source.id));
            }
        }
        for meta in &sessions {
            if let Some(source) = self
                .setting(&key(&meta.id))
                .and_then(|value| value.as_str().map(str::to_owned))
            {
                self.set_continued_from(&meta.id, Some(source));
            }
        }
    }

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

    /// Run `f` while holding the internal settings mutex. Callers must not await
    /// or lock sessions: queue admission orders sessions before settings.
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

    /// Close the queue admission race before awaiting a durable context edit.
    /// Messages arriving during the write remain queued, including schedules.
    pub fn begin_context_edit(
        &self,
        id: &str,
        allow_queued: bool,
    ) -> Result<ContextEditGuard, String> {
        let sessions = self.inner.sessions.lock();
        let session = sessions.get(id).ok_or("Session unavailable")?;
        if (!allow_queued && !session.queue.is_empty())
            || session.dispatch_pending.is_some()
            || session.delivery_hold.is_some()
        {
            return Err("Wait for the current submission, or clear the pending context to release queued messages".into());
        }
        if !self.inner.context_edits.lock().insert(id.to_owned()) {
            return Err("A context edit is already being saved".into());
        }
        Ok(ContextEditGuard {
            hub: self.clone(),
            id: id.to_owned(),
        })
    }

    fn context_allows_dispatch(&self, id: &str, cmid: Option<&str>) -> bool {
        !self.inner.context_edits.lock().contains(id)
            && crate::session_context::allows_dispatch(self, id, cmid)
    }

    pub fn has_queued_cmid(&self, id: &str, cmid: &str) -> bool {
        self.inner.sessions.lock().get(id).is_some_and(|session| {
            session
                .queue
                .iter()
                .any(|entry| entry.cmid.as_deref() == Some(cmid))
        })
    }

    pub fn context_state_changed(&self, id: &str) {
        self.try_drain(id);
    }

    pub fn set_resource_wait(&self, session_id: &str, reason: Option<String>) {
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            if let Some(s) = sessions.get_mut(session_id)
                && s.meta.resource_wait != reason
            {
                s.meta.resource_wait = reason;
                true
            } else {
                false
            }
        };
        if changed {
            self.broadcast_sessions();
        }
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
    /// in-flight turn to end, then advances). Persisted and broadcast so every
    /// terminal and a restarted Controller retain the user's decision.
    pub fn set_paused(&self, session_id: &str, paused: bool) {
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            match sessions.get_mut(session_id) {
                Some(s) if s.meta.paused != paused => {
                    s.meta.paused = paused;
                    // Keep persistence ordered with concurrent pause/resume.
                    self.set_setting(
                        format!("{}{session_id}", settings_keys::SESSION_QUEUE_PAUSED_PREFIX),
                        serde_json::Value::Bool(paused),
                    );
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

    /// Record support from the exact live worker, without adding transcript rows.
    pub fn set_current_turn_support(&self, session_id: &str, supported: bool) {
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            match sessions.get_mut(session_id) {
                Some(s) if s.meta.supports_current_turn != supported => {
                    s.meta.supports_current_turn = supported;
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
        // A fork's first thread holds the copied conversation before any prompt.
        let forking = self.native_fork_pending(session_id);
        let persist = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.meta.agent_session_id = Some(agent_session_id.clone());
            forking || current_context_has_user_message(s)
        };
        if persist && let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::SetAgentSessionId {
                session_id: session_id.to_owned(),
                agent_session_id: Some(agent_session_id),
            });
        }
        // Queued after the native ID on the same writer: a crash between the two
        // leaves a pending fork whose ID is already saved, which never re-forks.
        if forking {
            self.settle_native_fork(session_id);
        }
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
    pub fn accept_execution_binding(
        &self,
        session_id: &str,
        expected: &crate::execution_environment::ExecutionBinding,
        prepared: crate::execution_environment::ExecutionBinding,
        resolved_source: Option<&str>,
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
            if !expected.preparation().is_some_and(|mut intent| {
                if intent.validate().is_err() {
                    return false;
                }
                // Only the owning Machine's discovery reply may provide
                // this physical source. The durable intent remains the
                // exact CAS identity and every other field stays fenced.
                if let Some(source) = resolved_source {
                    intent.source_path = source.to_owned();
                }
                intent.accepts(&binding)
            }) {
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
    pub fn accept_execution_recovery(
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
    /// release the old worker's in-flight guard. A prompt still owned by the
    /// dispatcher has not reached that worker: retain its guard across reset so
    /// a newer queued prompt cannot overtake asynchronous preflight. Once handed
    /// to a runtime, its normal completion releases the guard.
    ///
    /// Context usage and background work describe the discarded agent context.
    /// A fresh agent reports usage only after its first model turn, and a
    /// dormant session has no worker snapshot to re-seed it, so leaving them
    /// here would keep the old context ring and Usage panel after Clear.
    pub fn prepare_context_reset(&self, session_id: &str) {
        // Clearing a fork, even before its first launch, discards the copy.
        self.discard_native_fork(session_id);
        let usage_changed = {
            let mut sessions = self.inner.sessions.lock();
            sessions.get_mut(session_id).is_some_and(|s| {
                s.meta.agent_session_id = None;
                s.in_flight = s.dispatch_pending.is_some();
                let changed = s.meta.context_used != 0
                    || s.meta.context_size != 0
                    || s.meta.usage.is_some()
                    || s.meta.background_tasks != 0;
                s.meta.context_used = 0;
                s.meta.context_size = 0;
                s.meta.usage = None;
                s.meta.background_tasks = 0;
                changed
            })
        };
        if usage_changed {
            self.broadcast_sessions();
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
        if let Some(ledger) = self.inner.background.lock().get_mut(session_id) {
            ledger.marks.clear();
            ledger.latest.clear();
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::ClearEvents {
                session_id: session_id.to_owned(),
                calls_before_ms: chrono::Utc::now().timestamp_millis(),
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
        let settled_update = {
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
            if (completed_turn || terminated) && s.dispatch_pending.is_none() {
                s.in_flight = false;
                s.dispatch_pending = None;
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
            let settled_update = (status != Status::Starting)
                .then(|| s.meta.provider_update.take())
                .flatten()
                .map(|update| {
                    (
                        update,
                        s.meta.machine_id.clone(),
                        s.meta.provider.clone(),
                        s.meta.provider_generation_digest.clone(),
                    )
                });
            s.meta.status = status;
            settled_update
        };
        if let Some((update, machine, provider, digest)) = settled_update {
            let elapsed_ms = now_ms().saturating_sub(update.started_at_ms);
            if status == Status::Crashed {
                tracing::warn!(
                    session = %session_id, %machine, from = %update.from, to = %update.to,
                    automatic = update.automatic, elapsed_ms,
                    detail = detail.as_deref().unwrap_or_default(),
                    "Provider update left the session crashed"
                );
                if update.automatic {
                    let now = std::time::Instant::now();
                    self.inner
                        .automatic_update_failures
                        .lock()
                        .insert((machine, provider, digest), now);
                    self.inner
                        .automatic_update_failed_sessions
                        .lock()
                        .insert(session_id.to_owned(), now);
                }
            } else {
                tracing::info!(
                    session = %session_id, %machine, from = %update.from, to = %update.to,
                    automatic = update.automatic, elapsed_ms, ?status,
                    "Provider update finished"
                );
                self.inner
                    .automatic_update_failed_sessions
                    .lock()
                    .remove(session_id);
            }
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

    /// Record a notification occurrence a non-event producer observed.
    pub fn record_occurrence(&self, occurrence: crate::task_notifications::Occurrence) {
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::RecordOccurrence(occurrence));
        }
    }

    /// Whether the permission request at `seq` still waits. A log that no longer
    /// holds it cannot say otherwise: delivery then relies on expiry.
    #[must_use]
    pub fn permission_pending(&self, session_id: &str, seq: u64) -> bool {
        let sessions = self.inner.sessions.lock();
        let Some(session) = sessions.get(session_id) else {
            return false;
        };
        let mut request = None;
        for entry in &session.log {
            match (&entry.event, request) {
                (Event::PermissionRequest { request_id, .. }, None) if entry.seq == seq => {
                    request = Some(request_id);
                }
                (Event::PermissionResolved { request_id, .. }, Some(asked))
                    if request_id == asked =>
                {
                    return false;
                }
                _ => {}
            }
        }
        true
    }

    /// I4: decide whether a subagent snapshot is a new observation, and whether
    /// it starts a terminal occurrence. `None` drops it.
    fn accept_subagent_snapshot(
        &self,
        session_id: &str,
        child: &str,
        update: &serde_json::Value,
    ) -> Option<SubagentEnding> {
        use crate::transcript_anchors::{METRICS, SnapshotVerdict, judge_snapshot, mark_of};
        let mark = mark_of(update);
        let mut background = self.inner.background.lock();
        let ledger = background.entry(session_id.to_owned()).or_default();
        let previous = ledger.marks.get(child);
        let ending = match (mark.terminal, previous) {
            (false, _) => SubagentEnding::Working,
            (true, Some(previous)) if previous.terminal => {
                SubagentEnding::AlreadyEnded(previous.terminal_seq)
            }
            (true, _) => SubagentEnding::Ended,
        };
        match judge_snapshot(previous, &mark) {
            SnapshotVerdict::Duplicate => {
                METRICS.duplicates.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            SnapshotVerdict::Stale => {
                METRICS.revision_stale.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(session = %session_id, child, "dropping an older subagent revision");
                return None;
            }
            SnapshotVerdict::Conflict => {
                METRICS.revision_conflicts.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(session = %session_id, child, revision = mark.revision,
                    "subagent revision conflict: equal revision, different payload");
            }
            SnapshotVerdict::Accept => {}
        }
        ledger.marks.insert(child.to_owned(), mark);
        Some(ending)
    }

    /// Keep the terminal identity the Hub stamped on the accepted snapshot.
    fn settle_subagent_mark(&self, session_id: &str, child: &str, update: &serde_json::Value) {
        let mut background = self.inner.background.lock();
        if let Some(mark) = background
            .get_mut(session_id)
            .and_then(|ledger| ledger.marks.get_mut(child))
        {
            mark.terminal_seq = update
                .get("terminalSeq")
                .and_then(serde_json::Value::as_u64);
        }
    }

    fn record_subagent_observation(
        &self,
        session_id: &str,
        child: String,
        anchor_seq: u64,
        update: serde_json::Value,
    ) {
        let mut background = self.inner.background.lock();
        let ledger = background.entry(session_id.to_owned()).or_default();
        ledger.latest.insert(child, (anchor_seq, update));
        if ledger.latest.len() > BACKGROUND_LATEST_PER_SESSION {
            // Forget the oldest terminal observation first: the store keeps it.
            let victim = ledger
                .latest
                .iter()
                .min_by_key(|(_, (_, update))| {
                    (
                        !crate::transcript_anchors::subagent_terminal(update),
                        crate::transcript_anchors::snapshot_order(update),
                    )
                })
                .map(|(child, _)| child.clone());
            if let Some(victim) = victim {
                ledger.latest.remove(&victim);
            }
        }
    }

    /// The Machine worker epoch that produced a session's latest events. A
    /// change is a load boundary: cached child state and grants are invalid
    /// until the new worker reports again (docs/background-tasks.md I3).
    pub fn note_worker_incarnation(&self, session_id: &str, incarnation: &str) {
        let mut background = self.inner.background.lock();
        let ledger = background.entry(session_id.to_owned()).or_default();
        if ledger.incarnation.as_deref() == Some(incarnation) {
            return;
        }
        if ledger.incarnation.is_some() {
            crate::transcript_anchors::METRICS
                .epoch_changes
                .fetch_add(1, Ordering::Relaxed);
        }
        ledger.incarnation = Some(incarnation.to_owned());
    }

    /// This Controller process's bootstrap epoch.
    #[must_use]
    pub fn boot_epoch(&self) -> &str {
        &self.inner.boot_epoch
    }

    /// Adopt durable transcript anchors before any new event is reduced, so a
    /// restarted Hub replaces the original subagent or Call row.
    pub fn warm_transcript_anchors(&self, anchors: Vec<crate::transcript_anchors::AnchorWrite>) {
        let mut reducer = self.inner.history_reducer.lock();
        // Newest first from the store: adopt oldest first so the newest stay
        // most recently used in the bounded index.
        for anchor in anchors.into_iter().rev() {
            reducer.adopt_anchor(&anchor.session_id, &anchor.key, anchor.seq);
        }
    }

    /// Restore the identity of each child's durable snapshot, so a replay after
    /// a restart is fenced like one in the same process (I4). The process's own
    /// marks always win.
    pub fn warm_subagent_marks(
        &self,
        snapshots: Vec<(String, crate::store::transcript_anchors::AnchoredRow)>,
    ) {
        let mut background = self.inner.background.lock();
        for (session_id, row) in snapshots {
            let Some(child) = row
                .key
                .strip_prefix(crate::transcript_anchors::SUBAGENT_PREFIX)
            else {
                continue;
            };
            background
                .entry(session_id)
                .or_default()
                .marks
                .entry(child.to_owned())
                .or_insert_with(|| crate::transcript_anchors::mark_of(&row.update));
        }
    }

    /// The Hub's subagent observations for a session's bootstrap, or `None`
    /// for an unknown session.
    #[must_use]
    pub fn subagent_observations(&self, session_id: &str) -> Option<SubagentObservations> {
        let watermark_seq = {
            let sessions = self.inner.sessions.lock();
            sessions.get(session_id)?.next_seq.saturating_sub(1)
        };
        let background = self.inner.background.lock();
        let ledger = background.get(session_id);
        Some(SubagentObservations {
            items: ledger
                .map(|ledger| ledger.latest.values().cloned().collect())
                .unwrap_or_default(),
            watermark_seq,
            incarnation: ledger.and_then(|ledger| ledger.incarnation.clone()),
            memory_only: self.inner.store_tx.is_none(),
        })
    }

    fn broadcast_sessions(&self) {
        self.fanout(Outbound::Sessions {
            sessions: self.session_list(),
        });
    }

    /// Record a session-scoped failure without changing the transcript or
    /// emitting another client notification. Some ACP agents report failures
    /// as ordinary message chunks, so their original transcript event remains
    /// visible while this durable incident feeds the diagnostic log.
    pub fn record_session_error(&self, session_id: &str, message: &str) {
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
mod managed_call_tests;

#[cfg(test)]
mod session_owner_tests;

#[cfg(test)]
mod session_order_merge_tests;

#[cfg(test)]
mod config_preference_tests;

#[cfg(test)]
mod runtime_reconciliation_tests;

#[cfg(test)]
mod core_tests;

#[cfg(all(test, feature = "full"))]
mod session_meta_wire_tests;
