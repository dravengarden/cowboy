//! Plain session model: the persisted session record, its transcript
//! envelope, queued messages and the hot-tail and history budgets. It names
//! no Hub, store or supervisor state, so the store and other layers can share
//! these types without depending on `core` (plan slice 5.3, before the
//! `cowboy-model` crate exists).

pub mod settings_keys;

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::agent_model::{Event, Status};

/// Soft serialized-byte budget for one cursor history response. Event count
/// alone is not a useful bound: screenshots and large tool results can make a
/// 200-event page tens of megabytes and terminate an iOS WebContent process.
/// One oversized event is still returned so the cursor always advances.
pub const HISTORY_MAX_BYTES: usize = 512 * 1024;

/// Maximum persisted-history tail retained in the Hub. Older events stay in
/// Postgres and are fetched by `/api/history`.
pub const HOT_TAIL: usize = 1_000;

pub(crate) const HOT_TAIL_TRIM_BATCH: usize = 200;

/// Soft heap budget for one persisted session's canonical hot tail. A count
/// limit alone is ineffective for screenshots and multi-megabyte tool results.
/// Keep at least the newest event so the cursor always advances.
pub const HOT_TAIL_MAX_BYTES: usize = 1024 * 1024;

pub(crate) fn estimated_json_bytes(value: &serde_json::Value) -> usize {
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

pub(crate) fn trim_hot_log(
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QuestionPageSummary {
    pub id: u64,
    pub title: String,
    pub ordinal: u64,
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

/// ACP updates a worker emits on its own whenever it starts or reconnects.
/// They, like lifecycle edges, say nothing about whether anyone used the
/// session, so they do not reset its idle clock.
pub(crate) const NON_ACTIVITY_SESSION_UPDATES: [&str; 5] = [
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAttention {
    pub approvals: usize,
    pub questions: usize,
    /// Native recovery knows the request is live but its description left the hot tail.
    #[serde(default)]
    pub other: usize,
}

impl SessionAttention {
    pub fn is_empty(&self) -> bool {
        self.approvals == 0 && self.questions == 0 && self.other == 0
    }
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde skip_serializing_if passes a reference.
pub(crate) const fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

pub(crate) fn local_machine_id() -> String {
    "local".to_owned()
}
