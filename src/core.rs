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
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

use crate::persistence::EventReducer;
use crate::runtime_wire::{WorkerSnapshot, WorkerState};

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
const HOT_TAIL_MAX_BYTES: usize = 8 * 1024 * 1024;
const BROADCAST_CAPACITY: usize = 1_024;
/// Event-count ceiling for the cursor-based HTTP history route. The byte budget
/// above is the primary bound; this limits render work for many tiny events.
pub const HISTORY_PAGE: usize = 64;

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
fn estimated_envelope_bytes(envelope: &Envelope) -> usize {
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

fn trim_hot_log(log: &mut Vec<Envelope>, log_bytes: &mut usize, trim_count_batch: bool) -> bool {
    let mut drop_count = if trim_count_batch && log.len() > HOT_TAIL + HOT_TAIL_TRIM_BATCH {
        HOT_TAIL_TRIM_BATCH.min(log.len().saturating_sub(1))
    } else {
        0
    };
    let mut retained_bytes = *log_bytes;
    for envelope in &log[..drop_count] {
        retained_bytes = retained_bytes.saturating_sub(estimated_envelope_bytes(envelope));
    }
    while retained_bytes > HOT_TAIL_MAX_BYTES && drop_count + 1 < log.len() {
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
                && update.get("autoResumed").and_then(serde_json::Value::as_bool)
                    != Some(true)
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

use crate::agent_model::{AUTO_CONTINUE_PREFIX, SCHED_PREFIX};
pub use crate::agent_model::{Event, Status};

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

// --- Auto-resume interrupted turns (tasks/archive/2026/07/session-auto-resume) ---

/// Settings key: the global default for auto-resuming interrupted turns (bool).
const AUTO_RESUME_DEFAULT_KEY: &str = "session.autoResume.default";
/// Settings key: the continuation-message template (string with `{{var}}` holes).
const AUTO_RESUME_TEMPLATE_KEY: &str = "session.autoResume.template";
/// Built-in continuation template used when the operator hasn't customized one.
/// `{{partial}}` is the assistant output cowboy captured before the cut-off — the
/// one source of truth the revived agent's own store lacks. It MUST self-identify
/// as a SYSTEM auto-resume (not a fresh user request) and guard against re-running
/// side-effectful work: a revived agent that re-does already-done steps can loop
/// (re-deploy → another restart → resume → …) or double its side effects. The
/// "verify before re-running" line is the best-effort idempotency guard.
const DEFAULT_CONTINUATION_TEMPLATE: &str = "[系统自动续接,非用户重新提问] 你上一轮回复在完成前被 cowboy 重启打断,系统现自动恢复该轮。请**从中断处接着完成**,不要从头重做整个任务;尤其在重新执行任何有副作用的操作(写/改文件、部署、git 提交、发网络请求等)之前,先确认它是否已经做过,避免重复执行导致循环或副作用叠加。以下是你被打断前已产出的内容:\n\n{{partial}}";

/// Empty-partial framing: the turn was cut off BEFORE producing anything, so
/// there's nothing to "continue from" and we re-issue the original prompt. But it
/// must STILL be framed as a system auto-retry — a bare verbatim re-send (the old
/// behaviour) reads to the agent as a brand-new user request, so it re-runs
/// side-effectful work (re-deploy → another restart → resume → …): the exact loop
/// the user hit. `{{prompt}}` is the original request.
const DEFAULT_RETRY_TEMPLATE: &str = "[系统自动续接,非用户重新提问] 你上一轮还没产出任何内容就被 cowboy 重启打断,系统现自动重试该轮。请重新处理下面这条原始请求;但在执行其中任何有副作用的操作(写/改文件、部署、git 提交、发网络请求等)之前,先确认它是否已经做过,避免重复执行导致循环或副作用叠加:\n\n{{prompt}}";

/// Render a `{{var}}` template by literal substitution. Unknown vars are left
/// verbatim (the UI flags them); a var absent from the map is simply not
/// replaced.
fn render_template(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (k, v) in vars {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
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

/// Extract only the final agent answer for L2 classification. Codex labels its
/// commentary/final phases explicitly; other ACP providers fall back to the
/// last message-id group. Long answers retain both the opening result and the
/// closing question, where the useful classification signals usually live.
fn last_judge_text(log: &[Envelope]) -> String {
    #[derive(Clone)]
    struct Chunk {
        id: Option<String>,
        final_answer: bool,
        text: String,
    }

    let last_user = log.iter().rposition(|env| {
        matches!(
            &env.event,
            Event::Update { update }
                if update.get("sessionUpdate").and_then(serde_json::Value::as_str)
                    == Some("user_message_chunk")
        )
    });
    let Some(start) = last_user else {
        return String::new();
    };
    let chunks: Vec<Chunk> = log[start..]
        .iter()
        .filter_map(|env| {
            let Event::Update { update } = &env.event else {
                return None;
            };
            if update
                .get("sessionUpdate")
                .and_then(serde_json::Value::as_str)
                != Some("agent_message_chunk")
            {
                return None;
            }
            Some(Chunk {
                id: update
                    .get("messageId")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                final_answer: update
                    .pointer("/_meta/codex/phase")
                    .and_then(serde_json::Value::as_str)
                    == Some("final_answer"),
                text: update
                    .pointer("/content/text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            })
        })
        .collect();
    let selected = chunks
        .iter()
        .rev()
        .find(|c| c.final_answer)
        .or_else(|| chunks.last());
    let Some(selected) = selected else {
        return String::new();
    };
    let mut text = if let Some(id) = selected.id.as_deref() {
        chunks
            .iter()
            .filter(|c| c.id.as_deref() == Some(id) && (!selected.final_answer || c.final_answer))
            .map(|c| c.text.as_str())
            .collect::<String>()
    } else {
        selected.text.clone()
    };
    text.retain(|c| c != '\0');
    truncate_judge_text(text.trim())
}

fn truncate_judge_text(text: &str) -> String {
    const MAX: usize = 4_096;
    const HEAD: usize = 1_536;
    const MARKER: &str = "\n[…中间已截断…]\n";
    const MARKER_CHARS: usize = 11;
    let count = text.chars().count();
    if count <= MAX {
        return text.to_owned();
    }
    let head: String = text.chars().take(HEAD).collect();
    let tail: String = text
        .chars()
        .rev()
        .take(MAX - HEAD - MARKER_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}{MARKER}{tail}")
}

/// The ACP stop-reason string from the most recent `TurnEnd` event, if any. Read
/// by the confirm-detect L1 (a non-`EndTurn` stop ⇒ the turn was cut off, not a
/// question). `acp.rs` pushes `TurnEnd` to the log just before flipping the
/// status, so it's already present when the turn-end judge runs.
fn last_stop_reason(log: &[Envelope]) -> Option<String> {
    log.iter().rev().find_map(|e| match &e.event {
        Event::TurnEnd { stop_reason } => Some(stop_reason.clone()),
        _ => None,
    })
}

/// Session metadata for the list view (no event log).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub provider: String,
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
    /// Per-session OVERRIDE of the global auto-resume-interrupted-turns default.
    /// `None` = inherit `settings['session.autoResume.default']`; `Some(true)` =
    /// always auto-continue an interrupted turn for this session; `Some(false)` =
    /// never (explicit opt-out). See tasks/archive/2026/07/session-auto-resume.
    #[serde(default)]
    pub auto_resume: Option<bool>,
    /// True when the confirm-detect skill judged the agent's last turn as
    /// "awaiting the user" (a question/confirmation). Like `editing`, it PAUSES
    /// the whole drain so the next queued message isn't auto-sent as a wrong
    /// answer; it also drives the awaiting widget. Lives in the broadcast meta so
    /// clients see it. Persisted (migration 0008) so a held session survives a
    /// daemon restart; `serde(default)` only covers old clients omitting it.
    #[serde(default)]
    pub awaiting_user: bool,
    /// True when the confirm-detect skill judged the agent's last turn as having
    /// COMPLETED the task (drives the green "Task complete" overlay + a future
    /// notification). Persisted (migration 0008) so a finished session keeps it
    /// across a restart; re-judged each turn, cleared when the user sends / a new
    /// turn starts (a stale value is harmless — busy/crashed take overlay priority).
    #[serde(default)]
    pub done: bool,
    /// True while the async confirm-detect L2 judge is IN FLIGHT for the last
    /// turn (between the provisional hold and the verdict landing). Drives the
    /// pill's "Judging…" loading state so the purple "Waiting for your reply"
    /// doesn't flash prematurely. Transient — never persisted, resets to false on
    /// restart; `serde(default)` covers old clients + the restore path.
    #[serde(default)]
    pub judging: bool,
    /// User-set MANUAL PAUSE of the queue drain (the ⏸ toggle). While true the
    /// auto-drain is HELD — queued messages don't advance even after the current
    /// turn ends — but the running turn is NOT interrupted (it finishes). The
    /// user toggles it (`SetPaused`) and releases it to resume. A MANUAL send
    /// still overrides it. In-memory only (transient — resets to false on a
    /// daemon restart); `serde(default)` covers old clients + the restore path.
    #[serde(default)]
    pub paused: bool,
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
    /// Soonest fire time (epoch ms) across this session's SCHEDULED DRAFTS, or
    /// `None` if none are scheduled. Derived from the drafts in `session_list`
    /// (not stored on the struct proper) so the session-row clock badge can show
    /// "next fires at …" without shipping every draft to the list. Transient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_schedule_ms: Option<i64>,
}

fn local_machine_id() -> String {
    "local".to_owned()
}

fn default_config_preferences(provider: &str) -> serde_json::Value {
    if provider == "codex" {
        serde_json::json!({
            "model": "gpt-5.6-luna",
            "reasoning_effort": "max",
        })
    } else if provider == "codex-deepseek" {
        serde_json::json!({
            "model": "deepseek-v4-flash",
            "deepseek_context": "680k",
            "deepseek_cache_protection": true,
            "collaboration_mode": "default",
            "reasoning_effort": "max",
        })
    } else if provider == "claude-deepseek" {
        serde_json::json!({
            "model": "deepseek-v4-flash[1m]",
            "deepseek_context": "830k",
            "deepseek_cache_protection": true,
            "effort": "max",
            "agent": "default",
        })
    } else if provider == "grok" {
        // Keep the model id dynamic so a CLI update can change its catalog,
        // while making Cowboy's High + full-access defaults durable.
        serde_json::json!({
            "permission_mode": "always-approve",
            "reasoning_effort": "high",
        })
    } else {
        serde_json::json!({})
    }
}

fn projected_config_options(
    provider: &str,
    preferences: &serde_json::Value,
    options: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    let had_options = options.is_some();
    let mut options = options.unwrap_or_else(|| serde_json::json!([]));
    let Some(array) = options.as_array_mut() else {
        return Some(options);
    };
    array.retain(|option| {
        let id = option.get("id").and_then(serde_json::Value::as_str);
        id != Some(crate::deepseek_context::CONFIG_ID)
            && id != Some(crate::deepseek_cache::CONFIG_ID)
    });
    let model = preferences.get("model").and_then(serde_json::Value::as_str);
    let requested = preferences
        .get(crate::deepseek_context::CONFIG_ID)
        .and_then(serde_json::Value::as_str);
    if let Some(option) = crate::deepseek_context::config_option(provider, model, requested) {
        let insert_at = array
            .iter()
            .position(|candidate| {
                candidate.get("id").and_then(serde_json::Value::as_str) == Some("model")
            })
            .map_or(array.len(), |index| index.saturating_add(1));
        array.insert(insert_at, option);
    }
    if let Some(enabled) = crate::deepseek_cache::selected(preferences, provider)
        && let Some(option) = crate::deepseek_cache::config_option(provider, enabled)
    {
        let insert_at = array
            .iter()
            .position(|candidate| {
                candidate.get("id").and_then(serde_json::Value::as_str)
                    == Some(crate::deepseek_context::CONFIG_ID)
            })
            .map_or(array.len(), |index| index.saturating_add(1));
        array.insert(insert_at, option);
    }
    if had_options || crate::deepseek_cache::supported_provider(provider) {
        Some(options)
    } else {
        None
    }
}

/// Immutable attributes assigned when a Cowboy session is registered.
pub struct SessionRegistration {
    pub id: String,
    pub provider: String,
    pub machine_id: String,
    pub workspace_id: Option<String>,
    pub workspace_name: Option<String>,
    pub workspace_source_path: Option<String>,
    pub cwd: String,
    pub title: String,
    pub origin: SessionOrigin,
    pub system: bool,
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

/// One ambiguous normal turn handed to the shared Codex classifier worker.
#[derive(Debug, Clone)]
pub struct JudgeReq {
    pub session_id: String,
    pub seq: u64,
    pub final_text: String,
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
    /// Persisted confirm-detect judge-run history (newest first), capped.
    pub judge_runs: Vec<JudgeRun>,
    /// Latest agent-advertised config options, retained so a new device can
    /// render the session controls before its worker is warm.
    pub config_options: Option<serde_json::Value>,
    /// User-selected values that the service must re-apply when the worker is
    /// recreated. Defaults are seeded for newly-created OpenAI sessions.
    pub config_preferences: serde_json::Value,
    pub mobile_review_state: serde_json::Value,
}

/// One persisted confirm-detect judge run — the verdict PLUS the raw LLM I/O,
/// kept as a per-session history that backs the inspector widget (long-press the
/// turn-status pill). This is the durable superset of the live `JudgeResult`
/// broadcast: same fields, plus an `id` for delete and an `at` for display/sort.
///
/// `id` is server-minted as `<at>-<seq>` (unix-ms + the turn's judge_seq). The
/// timestamp makes it monotonic ACROSS restarts, so a run minted after a restart
/// (when `judge_seq` resets to 0) can never collide with a persisted one — the
/// only key the client deletes by.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeRun {
    pub id: String,
    /// Unix-ms when the verdict landed.
    pub at: i64,
    /// "L1" (deterministic stop-reason) or "L2" (the Codex judge).
    pub layer: String,
    pub awaiting_user: bool,
    pub done: bool,
    pub confidence: f32,
    pub reason: String,
    /// The model id (L2) or empty (L1).
    pub model: String,
    /// What the judge looked at — the agent's final text.
    pub input: String,
    /// The model's raw output (L2) or the L1 reason.
    pub output: String,
    pub cache_hit: u32,
    pub cache_miss: u32,
    pub latency_ms: u64,
}

/// How many judge runs to keep per session. The raw input (the agent's full final
/// message) can be a few KB, and the whole array is rewritten as JSONB on every
/// turn, so this caps both the wire/broadcast size and the DB write.
const JUDGE_HISTORY_CAP: usize = 30;

/// Unix-ms now — the `at`/id stamp for a judge run.
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
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

/// Per-session state: metadata + the seq-ordered event log.
struct Session {
    meta: SessionMeta,
    /// Hot event tail when persistence is enabled; the full log in memory-only
    /// development mode.
    log: Vec<Envelope>,
    /// Soft heap estimate for `log`, maintained alongside canonical upserts so
    /// large tool payloads are bounded without serializing every text chunk.
    log_bytes: usize,
    event_count: u64,
    reached_start: bool,
    next_seq: u64,
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
    /// client releases or disconnects. One hold per session (matches the
    /// original single client-side `editingHold` model).
    editing: Option<String>,
    /// True while a queue-dispatched prompt of ours is in flight but the session
    /// hasn't yet flipped back to idle. Guards the dispatch-before-`Busy` window
    /// so a same-tick re-drain can't double-send and overlap turns. Cleared on
    /// the `Busy`→`Running` turn-end edge or on death (see `set_status`).
    in_flight: bool,
    /// Monotonic identity for the current lifecycle edge. Bumped only when the
    /// status actually changes, so duplicate worker snapshots do not disguise a
    /// stuck turn while a Busy -> Running -> Busy replacement invalidates every
    /// watchdog armed for the previous turn.
    lifecycle_epoch: u64,
    /// Monotonic counter bumped on every turn-end judge dispatch. The async
    /// confirm-detect verdict carries the seq it was issued under and is applied
    /// ONLY if still current — a newer turn-end supersedes a stale verdict (the
    /// stale-clear race in plan.md Risks). Not persisted; resets to 0 on restart.
    judge_seq: u64,
    /// Confirm-detect judge-run history (newest first), capped at
    /// [`JUDGE_HISTORY_CAP`]. Server-authoritative + persisted (migration 0009);
    /// backs the inspector widget. Broadcast as [`Outbound::JudgeHistory`].
    judge_runs: Vec<JudgeRun>,
    /// Mobile-only code-review workspace state. Desktop never consumes it.
    mobile_review: MobileReviewState,
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
    /// Set a session's auto-resume OVERRIDE (`value: null` = inherit the global
    /// default, `true`/`false` = force on/off). Persisted + re-broadcast on
    /// `SessionMeta`. See tasks/archive/2026/07/session-auto-resume.
    SetSessionAutoResume {
        session_id: String,
        #[serde(default)]
        value: Option<bool>,
    },
    /// Clear/set a session's confirm-detect "awaiting user" hold from the awaiting
    /// widget. The user dismissing it (`false`) means "not a question" → the hold
    /// lifts and the queue drains. Broadcasts the updated session list.
    SetAwaiting { session_id: String, awaiting: bool },
    /// User toggle: manually pause/resume the queue drain. Holds the auto-drain
    /// without interrupting the running turn (see [`Hub::set_paused`]).
    SetPaused { session_id: String, paused: bool },
    /// Overlay action: resume an interrupted turn (inject the continuation + run).
    ResumeTurn { session_id: String },
    /// Overlay action: retry an errored/crashed turn (re-run the last prompt).
    RetryTurn { session_id: String },
    /// Set one global setting (`session.autoResume.default` flag /
    /// `session.autoResume.template` string). Persisted + broadcast to every
    /// surface as [`Outbound::Settings`].
    SetSetting {
        key: String,
        value: serde_json::Value,
    },
    /// Generic optimistic-sync mutation (@shared-utils/sync). The client applies
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
    /// Drop one queued prompt.
    RemoveQueued { session_id: String, id: String },
    /// Edit a queued prompt in place (text + content). Empty both → removed.
    EditQueued {
        session_id: String,
        id: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        content: Vec<serde_json::Value>,
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
    },
    /// Drop one draft.
    RemoveDraft { session_id: String, id: String },
    /// Drop a session's whole draft list.
    ClearDrafts { session_id: String },
    /// Activate one draft: submit it (send-or-queue) and remove it from drafts.
    ActivateDraft { session_id: String, id: String },
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
    /// Delete one run from a session's confirm-detect judge history (the
    /// inspector widget's per-item delete).
    RemoveJudgeRun { session_id: String, id: String },
    /// Clear a session's entire confirm-detect judge history.
    ClearJudgeRuns { session_id: String },
}

/// What the server pushes to a WebSocket client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outbound {
    /// Full session list (sent on connect and whenever it changes).
    Sessions { sessions: Vec<SessionMeta> },
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
    /// A generic snapshot patch for one synced `state` (@shared-utils/sync): the
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
    /// The global key-value settings (auto-resume default flag + continuation
    /// template). Sent on connect and re-broadcast whenever an edit lands, so
    /// every surface renders the same Settings UI + computes the same effective
    /// auto-resume.
    Settings {
        settings: std::collections::HashMap<String, serde_json::Value>,
    },
    /// The registered skills (id/title/description + the prompt template + the
    /// extraction rule), so the Info sheet can render each skill's prompt verbatim.
    /// Static — sent once on connect.
    Skills { skills: Vec<SkillView> },
    /// The confirm-detect judge's full result for a turn — the verdict PLUS the
    /// observability detail the overlay's "raw data" expand shows. Sent after each
    /// judge; the client keeps the latest per session. NOT persisted.
    JudgeResult {
        session_id: String,
        /// "L1" (deterministic stop-reason) or "L2" (the Codex judge).
        layer: String,
        awaiting_user: bool,
        done: bool,
        confidence: f32,
        reason: String,
        /// The model id (L2) or empty (L1).
        model: String,
        /// What the judge looked at — the agent's final text.
        input: String,
        /// The model's raw output (L2) or the L1 reason.
        output: String,
        cache_hit: u32,
        cache_miss: u32,
        latency_ms: u64,
    },
    /// A session's confirm-detect judge-run HISTORY (newest first), capped. The
    /// durable, server-authoritative superset of `JudgeResult` that backs the
    /// inspector widget. Sent per session on connect + re-broadcast on every new
    /// run / per-item delete / clear.
    JudgeHistory {
        session_id: String,
        runs: Vec<JudgeRun>,
    },
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
}

/// Persistence intent sent on the write-behind channel from `Hub` to the
/// background DB writer task in `crate::server`. Each variant maps 1:1 to a
/// [`crate::store::Store`] call.
#[derive(Debug, Clone)]
pub enum StoreWrite {
    InsertSession(SessionMeta),
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
    /// Persist the confirm-detect turn-end verdict (so a done/awaiting session
    /// survives a daemon restart — migration 0008).
    UpdateVerdict {
        session_id: String,
        awaiting_user: bool,
        done: bool,
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
    UpdateSessionOrder {
        order: Vec<String>,
    },
    /// Persist a session's auto-resume OVERRIDE (`None` = inherit global default).
    UpdateAutoResume {
        session_id: String,
        value: Option<bool>,
    },
    /// Persist a session's confirm-detect judge-run history (whole list, as JSONB
    /// — migration 0009). Written on every add / delete / clear, like
    /// [`StoreWrite::UpdatePending`].
    UpdateJudgeRuns {
        session_id: String,
        runs: Vec<JudgeRun>,
    },
    /// Upsert one global setting (auto-resume default flag / continuation template).
    PutSetting {
        key: String,
        value: serde_json::Value,
    },
    /// Persist one session's Mobile-only code-review workspace state.
    UpdateMobileReviewState {
        session_id: String,
        value: serde_json::Value,
    },
    /// Upsert a session's pending `ScheduleWakeup` (migration 0011) so an armed
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
}

/// Shared operational state for the bounded write-behind queue.
#[derive(Debug, Default)]
pub struct PersistenceHealth {
    pending: AtomicUsize,
    dropped: AtomicU64,
    failed_batches: AtomicU64,
    degraded: AtomicBool,
    last_error: Mutex<Option<String>>,
}

impl PersistenceHealth {
    #[must_use]
    pub fn pending(&self) -> usize {
        self.pending.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn failed_batches(&self) -> u64 {
        self.failed_batches.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn is_healthy(&self) -> bool {
        !self.degraded.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }

    pub(crate) fn consumed(&self, count: usize) {
        self.pending.fetch_sub(count, Ordering::Relaxed);
    }

    pub(crate) fn mark_failed_batch(&self) {
        self.failed_batches.fetch_add(1, Ordering::Relaxed);
        self.degraded.store(true, Ordering::Relaxed);
        *self.last_error.lock() = Some("database write retries exhausted".to_owned());
    }

    fn mark_rejected(&self, error: &str) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
        self.degraded.store(true, Ordering::Relaxed);
        *self.last_error.lock() = Some(error.to_owned());
    }
}

/// Non-blocking producer for the bounded persistence queue. Queue exhaustion
/// is deliberately visible through health/metrics instead of growing memory
/// without bound or silently discarding an intent.
#[derive(Clone)]
pub struct StoreSink {
    tx: mpsc::Sender<StoreWrite>,
    health: std::sync::Arc<PersistenceHealth>,
}

impl StoreSink {
    #[must_use]
    pub fn new(tx: mpsc::Sender<StoreWrite>, health: std::sync::Arc<PersistenceHealth>) -> Self {
        Self { tx, health }
    }

    pub fn send(&self, write: StoreWrite) -> bool {
        self.health.pending.fetch_add(1, Ordering::Relaxed);
        match self.tx.try_send(write) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(write))
                if !matches!(write, StoreWrite::AppendEvent(_)) =>
            {
                // Low-volume state (queue, drafts, settings, lifecycle) must not
                // disappear merely because a burst of stream events filled the
                // bounded queue. Wait for capacity off the synchronous Hub path.
                let tx = self.tx.clone();
                let health = Arc::clone(&self.health);
                match tokio::runtime::Handle::try_current() {
                    Ok(runtime) => {
                        runtime.spawn(async move {
                        if let Err(error) = tx.send(write).await {
                            health.consumed(1);
                            health.mark_rejected(&error.to_string());
                            tracing::error!(%error, "critical persistence intent was not accepted");
                        }
                    });
                        true
                    }
                    _ => {
                        self.health.consumed(1);
                        self.health
                            .mark_rejected("persistence queue full outside Tokio runtime");
                        false
                    }
                }
            }
            Err(error) => {
                self.health.pending.fetch_sub(1, Ordering::Relaxed);
                self.health.mark_rejected(&error.to_string());
                tracing::error!(%error, "persistence queue rejected an intent");
                false
            }
        }
    }
}

/// Client-facing, owned view of a registered skill (the static `SkillMeta` has
/// `&'static str` fields, which `Outbound`'s `Deserialize` can't target — so we
/// snapshot it into owned strings for the wire).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillView {
    pub id: String,
    pub title: String,
    pub description: String,
    pub prompt_template: String,
    pub extract: String,
}

/// Live arbiter state for the title-sync channel (the @shared-utils/sync
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
/// @shared-utils/sync reference arbiter, in Rust). One entry per synced state
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

struct HubInner {
    sessions: Mutex<HashMap<String, Session>>,
    /// Persisted Busy sessions awaiting an authoritative, connected worker
    /// snapshot after the control plane restarts. Broker registry placeholders
    /// do not settle this set; a bounded server-side grace timer finalizes the
    /// remainder as genuine interruptions.
    runtime_reconciliation: Mutex<HashSet<String>>,
    /// Canonicalizes the raw ACP stream for the in-memory replay tail. The DB
    /// writer owns a separate reducer because it consumes the same raw stream
    /// asynchronously; live WebSocket subscribers continue to receive raw frames.
    history_reducer: Mutex<EventReducer>,
    /// Insertion order of session ids, so the list view is stable.
    order: Mutex<Vec<String>>,
    /// Live fan-out to all connected clients. Lagging receivers are dropped by
    /// `broadcast` and simply miss events until their next reconnect snapshot.
    tx: broadcast::Sender<Outbound>,
    /// Optional write-behind channel to the DB writer. `None` ⇒ in-memory
    /// only (no `--database-url` configured).
    store_tx: Option<StoreSink>,
    /// Hand-off to the background dispatcher task that owns the `Supervisor`.
    /// Set once at startup via [`Hub::set_dispatch_tx`]; `None` until then (and
    /// in tests), in which case a drain decision is computed but no prompt is
    /// actually sent. See [`DispatchReq`].
    dispatch_tx: Mutex<Option<mpsc::Sender<DispatchReq>>>,
    /// Hand-off to the single shared Codex classifier worker. Each normal
    /// `EndTurn` judgment gets an isolated Luna thread on one app-server process.
    judge_tx: Mutex<Option<mpsc::Sender<JudgeReq>>>,
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
    /// Global key-value settings (auto-resume default flag + continuation
    /// template), mirrored from the `settings` table on restore. Authoritative
    /// in-memory; every edit also write-behinds via `StoreWrite::PutSetting`.
    settings: Mutex<HashMap<String, serde_json::Value>>,
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
                runtime_reconciliation: Mutex::new(HashSet::new()),
                history_reducer: Mutex::new(EventReducer::default()),
                order: Mutex::new(Vec::new()),
                tx,
                store_tx,
                dispatch_tx: Mutex::new(None),
                judge_tx: Mutex::new(None),
                scheduler_tx: Mutex::new(None),
                sync: Mutex::new(HashMap::new()),
                next_qid: AtomicU64::new(1),
                next_error_id: AtomicU64::new(1),
                settings: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Wire the background dispatcher's hand-off channel. Called once at startup
    /// (in `crate::server`) after the dispatcher task is spawned, before any
    /// client connects. Until set, drains compute but dispatch nothing.
    pub fn set_dispatch_tx(&self, tx: mpsc::Sender<DispatchReq>) {
        *self.inner.dispatch_tx.lock() = Some(tx);
    }

    /// Wire the shared Codex classifier queue. Called once at server startup.
    pub fn set_judge_tx(&self, tx: mpsc::Sender<JudgeReq>) {
        *self.inner.judge_tx.lock() = Some(tx);
    }

    /// Wire the background scheduler's hand-off channel (mirrors
    /// [`Self::set_dispatch_tx`]). Until set, [`Self::schedule_wakeup`] is a no-op.
    pub fn set_scheduler_tx(&self, tx: mpsc::Sender<crate::scheduler::ScheduleCmd>) {
        *self.inner.scheduler_tx.lock() = Some(tx);
    }

    /// Arm (replace) a session's pending `ScheduleWakeup` — `acp.rs` calls this
    /// when it intercepts the tool. `delay_seconds` is the agent-requested delay
    /// (clamped by the scheduler); the wakeup fires `prompt` as its own turn.
    /// Also persisted (migration 0011) so it survives a restart.
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
    /// Skips the write-behind side: these rows are already in the DB.
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

    /// Restore persisted sessions while reconciling detached runtime workers.
    /// A persisted `Busy` row is interrupted only when no matching live worker
    /// exists. This prevents a control-plane deploy from generating a false
    /// interruption marker and duplicate auto-resume while the original ACP
    /// prompt is still running in its detached worker.
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
        // Sessions whose ids we HEALED (re-id'd a duplicate) → persist after.
        let mut reid_dirty: Vec<String> = Vec::new();
        // Ids already seen across ALL sessions — ids must be globally unique so a
        // later cross-session move can't collide. The first occurrence keeps its
        // id; a duplicate (corruption from the old counter-reset bug) gets a fresh
        // one past `max_qid`.
        let mut seen: HashSet<String> = HashSet::new();
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
                    judge_runs,
                    config_options,
                    config_preferences,
                    mobile_review_state,
                } = r;
                let mut healed = false;
                for m in queue.iter_mut().chain(drafts.iter_mut()) {
                    if !seen.insert(m.id.clone()) {
                        m.id = self.next_qid();
                        seen.insert(m.id.clone());
                        healed = true;
                    }
                }
                let id = meta.id.clone();
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
                if healed {
                    reid_dirty.push(id.clone());
                }
                let mut log_bytes = log.iter().fold(0usize, |size, envelope| {
                    size.saturating_add(estimated_envelope_bytes(envelope))
                });
                if self.inner.store_tx.is_some() && trim_hot_log(&mut log, &mut log_bytes, false) {
                    reached_start = false;
                }
                for envelope in &log {
                    let _ = history_reducer.reduce(envelope.clone());
                }
                sessions_lock.insert(
                    id.clone(),
                    Session {
                        meta,
                        log,
                        log_bytes,
                        event_count,
                        reached_start,
                        next_seq,
                        config_options,
                        config_preferences,
                        queue,
                        drafts,
                        editing: None,
                        in_flight: (defer_missing_busy && was_busy)
                            || runtime.is_some_and(|worker| {
                                worker.current_turn_id.is_some() || worker.pending_prompt_count > 0
                            }),
                        lifecycle_epoch: 0,
                        judge_seq: 0,
                        judge_runs,
                        mobile_review: MobileReviewState::from_stored(mobile_review_state),
                    },
                );
                order.push(id);
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
        // Persist any session whose duplicate ids we healed, so the corrected
        // (unique-id) lists reach the DB + every client.
        for id in reid_dirty {
            self.emit_pending(&id);
        }
    }

    /// Decide whether an incoming runtime snapshot may project lifecycle state
    /// into the Hub. A real connected owner atomically settles startup
    /// reconciliation. A broker-only placeholder is ignored while a persisted
    /// Busy turn is still waiting for its owner, so it cannot overwrite Busy
    /// with a speculative Starting/Running state.
    pub fn accept_runtime_snapshot(&self, worker: &WorkerSnapshot) -> bool {
        if worker.has_connected_owner() {
            if self
                .inner
                .runtime_reconciliation
                .lock()
                .remove(&worker.session_id)
            {
                tracing::info!(
                    session = %worker.session_id,
                    worker_epoch = %worker.worker_epoch,
                    "detached worker adopted restored in-flight turn"
                );
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

    /// Finalize persisted Busy sessions whose detached owner did not reconnect
    /// within the server's bounded grace period. Returns exactly the sessions
    /// newly marked Interrupted so the server can revive opted-in continuations.
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
        // Auto-resume (opted in, globally or per-session): enqueue a
        // continuation built from the cut-off turn's partial output. It stays
        // queued behind the Interrupted marker until the server revives it.
        if self.effective_auto_resume(session_id) {
            self.enqueue_continuation(session_id);
        }
    }

    /// Subscribe to the live event stream.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Outbound> {
        self.inner.tx.subscribe()
    }

    /// Current session list (insertion order).
    #[must_use]
    pub fn session_list(&self) -> Vec<SessionMeta> {
        let sessions = self.inner.sessions.lock();
        let order = self.inner.order.lock();
        order
            .iter()
            .filter_map(|id| {
                sessions.get(id).map(|s| {
                    let mut meta = s.meta.clone();
                    // Surface the soonest scheduled-draft fire so the session-row
                    // clock badge can show it, without shipping the drafts here.
                    meta.next_schedule_ms = s
                        .drafts
                        .iter()
                        .filter_map(|m| m.schedule.as_ref().map(|sc| sc.fire_at_ms))
                        .min();
                    meta
                })
            })
            .collect()
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

    #[must_use]
    pub fn session_has_in_flight_prompt(&self, session_id: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|session| session.in_flight)
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
            machine_id: "local".to_owned(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            cwd,
            title,
            origin,
            system,
        });
    }

    /// Register a session on a specific stable machine identity.
    pub fn create_session(&self, registration: SessionRegistration) {
        let SessionRegistration {
            id,
            provider,
            machine_id,
            workspace_id,
            workspace_name,
            workspace_source_path,
            cwd,
            title,
            origin,
            system,
        } = registration;
        let config_preferences = default_config_preferences(&provider);
        let meta = SessionMeta {
            id: id.clone(),
            provider,
            machine_id,
            workspace_id,
            workspace_name,
            workspace_source_path,
            cwd,
            title,
            status: Status::Starting,
            origin,
            agent_session_id: None,
            auto_resume: None, // inherit the global default until overridden
            awaiting_user: false,
            done: false,
            judging: false,
            paused: false,
            system,
            context_used: 0,
            context_size: 0,
            usage: None,
            next_schedule_ms: None,
        };
        {
            let mut sessions = self.inner.sessions.lock();
            let mut order = self.inner.order.lock();
            sessions.insert(
                id.clone(),
                Session {
                    meta: meta.clone(),
                    log: Vec::new(),
                    log_bytes: 0,
                    event_count: 0,
                    reached_start: true,
                    next_seq: 0,
                    config_options: None,
                    config_preferences: config_preferences.clone(),
                    queue: Vec::new(),
                    drafts: Vec::new(),
                    editing: None,
                    in_flight: false,
                    lifecycle_epoch: 0,
                    judge_seq: 0,
                    judge_runs: Vec::new(),
                    mobile_review: MobileReviewState::default(),
                },
            );
            order.push(id.clone());
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::InsertSession(meta));
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
        let removed = {
            let mut sessions = self.inner.sessions.lock();
            let mut order = self.inner.order.lock();
            let removed = sessions.remove(session_id).is_some();
            order.retain(|id| id != session_id);
            if removed {
                self.inner.history_reducer.lock().clear_session(session_id);
            }
            removed
        };
        if removed {
            if let Some(tx) = self.inner.store_tx.as_ref() {
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

    /// Set a session's auto-resume OVERRIDE (`None` = inherit the global
    /// default). Updates the typed truth (`SessionMeta.auto_resume`), persists,
    /// and re-broadcasts the session list — the override rides on `SessionMeta`,
    /// so the client recomputes its badge from `override ?? global default`.
    /// Mirror of [`Self::rename_session`].
    pub fn set_auto_resume(&self, session_id: &str, value: Option<bool>) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.meta.auto_resume = value;
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateAutoResume {
                session_id: session_id.to_owned(),
                value,
            });
        }
        self.broadcast_sessions();
    }

    /// Snapshot of all global settings — for the connect-time push.
    #[must_use]
    pub fn settings_snapshot(&self) -> HashMap<String, serde_json::Value> {
        self.inner.settings.lock().clone()
    }

    /// Set one global setting (auto-resume default / continuation template),
    /// persist it, and broadcast the new full map to every connected surface.
    pub fn set_setting(&self, key: String, value: serde_json::Value) {
        let snapshot = {
            let mut s = self.inner.settings.lock();
            s.insert(key.clone(), value.clone());
            s.clone()
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::PutSetting { key, value });
        }
        let _ = self
            .inner
            .tx
            .send(Outbound::Settings { settings: snapshot });
    }

    /// Seed the in-memory settings map from the persisted table (restore only).
    pub fn load_settings(&self, entries: Vec<(String, serde_json::Value)>) {
        let mut s = self.inner.settings.lock();
        for (k, v) in entries {
            s.insert(k, v);
        }
    }

    /// Snapshot the static skill registry into owned wire views (Info sheet).
    #[must_use]
    pub fn skills_snapshot(&self) -> Vec<SkillView> {
        crate::skills::registry()
            .into_iter()
            .map(|m| SkillView {
                id: m.id.to_owned(),
                title: m.title.to_owned(),
                description: m.description.to_owned(),
                prompt_template: m.prompt_template.to_owned(),
                extract: m.extract.to_owned(),
            })
            .collect()
    }

    /// Whether a session is currently holding for "awaiting user".
    #[must_use]
    #[cfg(test)]
    pub fn is_awaiting(&self, session_id: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|s| s.meta.awaiting_user)
    }

    /// Set/clear a session's "awaiting user" hold. Broadcasts the session list so
    /// the awaiting widget updates; clearing also resumes the drain.
    /// Write-behind the current turn-end verdict so it survives a restart (0008).
    fn persist_verdict(&self, session_id: &str, awaiting_user: bool, done: bool) {
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateVerdict {
                session_id: session_id.to_owned(),
                awaiting_user,
                done,
            });
        }
    }

    pub fn set_awaiting(&self, session_id: &str, awaiting: bool) {
        // `Some(done)` when the awaiting flag actually flipped — carries the
        // unchanged `done` so the persisted pair stays consistent.
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            match sessions.get_mut(session_id) {
                Some(s) if s.meta.awaiting_user != awaiting => {
                    s.meta.awaiting_user = awaiting;
                    Some(s.meta.done)
                }
                _ => None,
            }
        };
        if let Some(done) = changed {
            self.persist_verdict(session_id, awaiting, done);
            self.broadcast_sessions();
            if !awaiting {
                self.try_drain(session_id);
            }
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

    /// Mark a session as mid-judge — the async L2 confirm-detect is in flight, so
    /// the pill shows "Judging…" instead of prematurely flashing the provisional
    /// "Waiting for your reply". Broadcast-only (transient, not persisted); no-op
    /// if the flag is unchanged.
    fn set_judging(&self, session_id: &str, judging: bool) {
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            match sessions.get_mut(session_id) {
                Some(s) if s.meta.judging != judging => {
                    s.meta.judging = judging;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.broadcast_sessions();
        }
    }

    fn judge_is_current(&self, session_id: &str, seq: u64) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|s| s.judge_seq == seq)
    }

    fn set_judging_if_current(&self, session_id: &str, seq: u64, judging: bool) {
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            match sessions.get_mut(session_id) {
                Some(s) if s.judge_seq == seq && s.meta.judging != judging => {
                    s.meta.judging = judging;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.broadcast_sessions();
        }
    }

    /// Apply a confirm-detect verdict under the `judge_seq` stale-guard: set the
    /// hold to `v.awaiting_user` only if no newer turn-end has superseded `seq`.
    /// Broadcasts and resumes the drain when the hold clears.
    fn apply_verdict(&self, session_id: &str, seq: u64, v: &crate::skills::Verdict) -> bool {
        let resume = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if s.judge_seq != seq {
                return false; // a newer turn-end already re-judged — drop this verdict
            }
            let resume = s.meta.awaiting_user && !v.awaiting_user;
            s.meta.awaiting_user = v.awaiting_user;
            s.meta.done = v.done;
            resume
        };
        self.persist_verdict(session_id, v.awaiting_user, v.done);
        self.broadcast_sessions();
        if resume {
            self.try_drain(session_id);
        }
        true
    }

    /// Turn-end hook: classify the agent's last message with the confirm-detect
    /// skill. L1 (the agent provider's deterministic stop-reason rule) runs inline
    /// and short-circuits; only an ambiguous `EndTurn` spawns the async L2 judge.
    ///
    /// Recall-first (design §I): before the async judge, set `awaiting_user=true`
    /// synchronously so the drain can't release a queued prompt as a wrong answer.
    /// The verdict is applied under the `judge_seq` guard — `false` clears + drains,
    /// `true` keeps the hold; a repeated classifier failure clears it so the queue
    /// cannot deadlock. `done` is logged for completion notifications.
    fn judge_turn_end(&self, session_id: &str) {
        let (final_text, seq, provider, stop_reason) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.judge_seq = s.judge_seq.wrapping_add(1);
            (
                last_judge_text(&s.log),
                s.judge_seq,
                s.meta.provider.clone(),
                last_stop_reason(&s.log),
            )
        };
        // L1: deterministic, no LLM. A cut-off/cancelled turn settles here; only a
        // normal `EndTurn` returns None and falls through to L2.
        if let Some(v) = crate::provider::confirm::l1(
            &provider,
            &crate::provider::confirm::TurnEndCtx {
                stop_reason: stop_reason.as_deref(),
            },
        ) {
            let at = now_ms();
            self.emit_and_record_judge(
                session_id,
                JudgeRun {
                    id: format!("{at}-{seq}"),
                    at,
                    layer: "L1".to_owned(),
                    awaiting_user: v.awaiting_user,
                    done: v.done,
                    confidence: v.confidence,
                    reason: v.reason.clone(),
                    model: String::new(),
                    input: final_text.clone(),
                    output: v.reason.clone(),
                    cache_hit: 0,
                    cache_miss: 0,
                    latency_ms: 0,
                },
            );
            let _ = self.apply_verdict(session_id, seq, &v);
            return;
        }
        // Nothing the agent said this turn → nothing to judge; don't hold.
        if final_text.trim().is_empty() {
            self.set_awaiting(session_id, false);
            return;
        }
        // Provisional hold while the async L2 judge runs + flag it judging so the
        // pill shows "Judging…" rather than the provisional "Waiting for your reply".
        self.set_awaiting(session_id, true);
        self.set_judging(session_id, true);
        let req = JudgeReq {
            session_id: session_id.to_owned(),
            seq,
            final_text,
        };
        let sent = self
            .inner
            .judge_tx
            .lock()
            .as_ref()
            .is_some_and(|tx| tx.try_send(req.clone()).is_ok());
        if !sent {
            self.finish_judge_failure(req, "classifier worker unavailable");
        }
    }

    fn finish_judge_failure(&self, req: JudgeReq, error: &str) {
        if !self.judge_is_current(&req.session_id, req.seq) {
            return;
        }
        tracing::warn!(session = %req.session_id, %error, "confirm-detect judge failed; clearing provisional hold");
        let at = now_ms();
        let reason = format!("judge failed: {error}");
        let verdict = crate::skills::Verdict {
            reason: reason.clone(),
            ..Default::default()
        };
        if !self.apply_verdict(&req.session_id, req.seq, &verdict) {
            return;
        }
        self.emit_and_record_judge(
            &req.session_id,
            JudgeRun {
                id: format!("{at}-{}", req.seq),
                at,
                layer: "L2".to_owned(),
                awaiting_user: false,
                done: false,
                confidence: 0.0,
                reason: reason.clone(),
                model: crate::inference::codex::MODEL.to_owned(),
                input: req.final_text,
                output: String::new(),
                cache_hit: 0,
                cache_miss: 0,
                latency_ms: 0,
            },
        );
        self.set_judging_if_current(&req.session_id, req.seq, false);
    }

    /// Broadcast a judge run for the LIVE overlay (`JudgeResult`, latest-per-
    /// session — the pill's quick-peek expand) AND append it to the durable,
    /// per-session history (the inspector widget). One call from both judge
    /// layers so the two surfaces never diverge.
    fn emit_and_record_judge(&self, session_id: &str, run: JudgeRun) {
        self.broadcast(Outbound::JudgeResult {
            session_id: session_id.to_owned(),
            layer: run.layer.clone(),
            awaiting_user: run.awaiting_user,
            done: run.done,
            confidence: run.confidence,
            reason: run.reason.clone(),
            model: run.model.clone(),
            input: run.input.clone(),
            output: run.output.clone(),
            cache_hit: run.cache_hit,
            cache_miss: run.cache_miss,
            latency_ms: run.latency_ms,
        });
        let runs = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.judge_runs.insert(0, run); // newest first
            s.judge_runs.truncate(JUDGE_HISTORY_CAP);
            s.judge_runs.clone()
        };
        self.persist_and_emit_judge_runs(session_id, runs);
    }

    /// Write-behind the whole history + broadcast it. Shared by add/delete/clear.
    fn persist_and_emit_judge_runs(&self, session_id: &str, runs: Vec<JudgeRun>) {
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateJudgeRuns {
                session_id: session_id.to_owned(),
                runs: runs.clone(),
            });
        }
        self.broadcast(Outbound::JudgeHistory {
            session_id: session_id.to_owned(),
            runs,
        });
    }

    /// Delete one run from a session's judge history (inspector per-item delete).
    /// No-op (no broadcast) if the id isn't present.
    pub fn remove_judge_run(&self, session_id: &str, id: &str) {
        let runs = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let before = s.judge_runs.len();
            s.judge_runs.retain(|r| r.id != id);
            if s.judge_runs.len() == before {
                return; // nothing removed → don't churn
            }
            s.judge_runs.clone()
        };
        self.persist_and_emit_judge_runs(session_id, runs);
    }

    /// Clear a session's entire judge history. No-op if already empty.
    pub fn clear_judge_runs(&self, session_id: &str) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if s.judge_runs.is_empty() {
                return;
            }
            s.judge_runs.clear();
        }
        self.persist_and_emit_judge_runs(session_id, Vec::new());
    }

    /// Snapshot a session's judge history for the connect seed. Empty for an
    /// unknown session.
    #[must_use]
    pub fn judge_history(&self, session_id: &str) -> Vec<JudgeRun> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .map_or_else(Vec::new, |s| s.judge_runs.clone())
    }

    /// The global default for auto-resuming interrupted turns (off when unset).
    #[must_use]
    pub fn auto_resume_default(&self) -> bool {
        self.inner
            .settings
            .lock()
            .get(AUTO_RESUME_DEFAULT_KEY)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    /// Effective auto-resume for a session: its override, else the global
    /// default. `false` for an unknown session.
    #[must_use]
    pub fn effective_auto_resume(&self, session_id: &str) -> bool {
        let over = self
            .inner
            .sessions
            .lock()
            .get(session_id)
            .map(|s| s.meta.auto_resume);
        match over {
            Some(Some(v)) => v,                       // explicit per-session override
            Some(None) => self.auto_resume_default(), // inherit
            None => false,                            // unknown session
        }
    }

    /// The continuation-message template (the customizable string with `{{var}}`
    /// holes), falling back to the built-in default.
    fn continuation_template(&self) -> String {
        self.inner
            .settings
            .lock()
            .get(AUTO_RESUME_TEMPLATE_KEY)
            .and_then(|v| v.as_str())
            .map_or_else(|| DEFAULT_CONTINUATION_TEMPLATE.to_owned(), str::to_owned)
    }

    /// Build the continuation prompt for an interrupted turn (template rendered
    /// with the turn's partial output / original prompt / cwd) and ENQUEUE it, so
    /// it auto-drains the moment the agent revives (via `session/load`). No-op if
    /// the last turn left nothing to continue. The continuation is a fresh turn
    /// carrying cowboy's partial output (the agent's own store is unreliable after
    /// a mid-turn crash); the template tells it to continue, not redo.
    fn enqueue_continuation(&self, session_id: &str) {
        // Read the template before taking the sessions lock (avoid nesting the
        // settings lock under it).
        let template = self.continuation_template();
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            // RUNAWAY GUARD: never stack a second auto-continuation. A session
            // interrupted across several restarts while never opened (so the
            // continuation never drains) must not accrue a pile of them — the
            // bound that keeps "continue" from running away.
            if s.queue.iter().any(|m| {
                m.cmid
                    .as_deref()
                    .is_some_and(|c| c.starts_with(AUTO_CONTINUE_PREFIX))
            }) {
                return;
            }
            let (prompt, partial) = last_turn_texts(&s.log);
            // EMPTY-RESULT case: the turn was cut off before producing anything, so
            // there's nothing to "continue from" — re-issue the ORIGINAL prompt, but
            // WRAPPED in the auto-retry framing (NOT verbatim: a bare re-send reads
            // as a fresh user request → the agent re-runs side effects → loop). A
            // "here's what you produced: <nothing>" message would be nonsense, hence
            // the separate retry template. Nothing at all (no prompt either) → don't
            // enqueue, so a content-less interruption can't seed a continue.
            let text = if partial.trim().is_empty() {
                if prompt.trim().is_empty() {
                    return;
                }
                render_template(
                    DEFAULT_RETRY_TEMPLATE,
                    &[("prompt", &prompt), ("cwd", &s.meta.cwd)],
                )
            } else {
                render_template(
                    &template,
                    &[
                        ("partial", &partial),
                        ("prompt", &prompt),
                        ("cwd", &s.meta.cwd),
                    ],
                )
            };
            let id = self.next_qid();
            let cmid = format!("{AUTO_CONTINUE_PREFIX}{id}");
            // FRONT of the queue: the interrupted turn was running BEFORE anything
            // queued behind it, so its continuation must run first (the queue was
            // *waiting on* that turn), not after the backlog.
            s.queue.insert(
                0,
                QueuedMessage {
                    id,
                    text,
                    content: Vec::new(),
                    cmid: Some(cmid),
                    schedule: None,
                },
            );
            // A continuation is the system telling the agent to finish its OWN work
            // — never an answer to a question — so it must not sit behind a stale
            // confirm-detect hold. (The death-edge clear above usually handles this;
            // this is the belt-and-braces for any path that enqueues without one.)
            s.meta.awaiting_user = false;
        }
        self.emit_pending(session_id);
    }

    // --- Generic optimistic-sync channel (@shared-utils/sync arbiter) --------
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
    fn apply_reorder(&self, order: &[String]) {
        {
            let mut list = self.inner.order.lock();
            list.sort_by_key(|id| order.iter().position(|o| o == id).unwrap_or(usize::MAX));
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let order = self.inner.order.lock().clone();
            let _ = tx.send(StoreWrite::UpdateSessionOrder { order });
        }
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
    fn sync_value(&self, state: &str) -> serde_json::Value {
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
        let _ = self.inner.tx.send(Outbound::SyncPatch {
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
        enum Op {
            Rename {
                session_id: String,
                title: String,
            },
            Reorder {
                order: Vec<String>,
            },
            MobileReview {
                session_id: String,
                mutation: String,
                args: serde_json::Value,
            },
        }
        let op = match (state, name) {
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
            (state, name) if state.starts_with("mobile-review:") => {
                let session_id = state["mobile-review:".len()..].to_owned();
                if session_id.is_empty() || !self.inner.sessions.lock().contains_key(&session_id) {
                    return Err("unknown mobile review session".to_owned());
                }
                if !matches!(
                    name,
                    "open"
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
            return Ok(()); // duplicate delivery/retry — already applied + broadcast
        }
        match op {
            Op::Rename { session_id, title } => self.apply_rename(&session_id, title),
            Op::Reorder { order } => self.apply_reorder(&order),
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
    /// persisted (`@shared-utils/sync-idb`) cache of these states across reloads:
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
            // Guarantee title + order are present even when untouched this lifetime.
            for state in ["title", "order"] {
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
            let default_title = format!("{} · {}", session.meta.provider, session.meta.cwd);
            let title = (session.meta.title == default_title)
                .then(|| format!("{} · {cwd}", session.meta.provider));
            session.meta.cwd.clone_from(&cwd);
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
        let turn_ended;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            // Clear the in-flight guard on a true turn-end (Busy → Running) or on
            // death — NOT on Starting → Running (a revive passes through that
            // edge while our dispatched prompt is still queued downstream, so
            // clearing there would release the next prompt early and overlap
            // turns). Mirrors the old client-side drain edge logic.
            let was = s.meta.status;
            if expected.is_some_and(|expected| expected != (was, s.lifecycle_epoch)) {
                return false;
            }
            if (was == Status::Busy && status == Status::Running)
                || matches!(
                    status,
                    Status::Exited | Status::Crashed | Status::Interrupted
                )
            {
                s.in_flight = false;
            }
            // A turn that died/was cut off is NOT "awaiting your reply" — the agent
            // didn't ask a question, it got interrupted. Clear the confirm-detect
            // hold so it doesn't block the auto-resume continuation (which inserts
            // at the queue FRONT and must drain to revive the turn). The judge only
            // runs on a clean Busy→Running end, so these death edges need the
            // explicit clear.
            if matches!(
                status,
                Status::Exited | Status::Crashed | Status::Interrupted
            ) {
                s.meta.awaiting_user = false;
                s.meta.done = false;
            }
            // A true turn-end (Busy → Running) is where the agent handed control
            // back — the point to ask the confirm-detect skill "is it waiting on
            // me?". Captured here under the lock; the judge runs after we release.
            turn_ended = was == Status::Busy && status == Status::Running;
            if was != status {
                s.lifecycle_epoch = s.lifecycle_epoch.wrapping_add(1);
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
        // On a turn-end, judge whether the agent is awaiting the user BEFORE the
        // drain can release the next queued prompt. The judge sets a provisional
        // hold synchronously, so try_drain below is a no-op until the shared Codex
        // worker either clears it (drains) or confirms it (stays held).
        if turn_ended {
            self.judge_turn_end(session_id);
        }
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
        let (envelope, durable_agent_session_id) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let seq = s.next_seq;
            s.next_seq += 1;
            let envelope = Envelope {
                session_id: session_id.to_owned(),
                seq,
                event,
                cmid,
            };
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
            if self.inner.store_tx.is_some() && trim_hot_log(&mut s.log, &mut s.log_bytes, true) {
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
        // A send error just means no clients are connected — fine.
        let _ = self.inner.tx.send(Outbound::Event { envelope });
    }

    fn broadcast_sessions(&self) {
        let _ = self.inner.tx.send(Outbound::Sessions {
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
                &session.config_preferences,
                session.config_options.clone(),
            )
        })
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
            let mut options = projected_config_options(
                &session.meta.provider,
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
            let _ = self.inner.tx.send(Outbound::ConfigOptions {
                session_id: session_id.to_owned(),
                options,
            });
        }
        Ok(())
    }

    /// Store the latest agent-advertised config options for a session and
    /// fan them out to every client. Called from acp.rs when the upstream
    /// emits a `config_option_update` notification, and from the
    /// `SetConfigOption` reply path (the agent's authoritative response
    /// refreshes the same array).
    pub fn set_config_options(&self, session_id: &str, options: serde_json::Value) {
        let options = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let options =
                projected_config_options(&s.meta.provider, &s.config_preferences, Some(options))
                    .expect("agent config options remain present after projection");
            s.config_options = Some(options.clone());
            options
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateConfigOptions {
                session_id: session_id.to_owned(),
                options: options.clone(),
            });
        }
        let _ = self.inner.tx.send(Outbound::ConfigOptions {
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
        let _ = self.inner.tx.send(Outbound::Error {
            session_id,
            message,
        });
    }

    /// Broadcast an arbitrary outbound message to every connected client.
    pub fn broadcast(&self, msg: Outbound) {
        let _ = self.inner.tx.send(msg);
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
            if detail.is_some() {
                return detail.clone();
            }
        }
        None
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
        // The awaiting-user hold below applies only to automatic drain. An
        // explicit manual send always gets through, so a queue cannot remain
        // trapped behind a stale classifier verdict.
        let req = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if !Self::ready(s, allow_revive) {
                return;
            }
            // The agent's last turn was judged "awaiting the user" → hold the whole
            // queue so the next message isn't auto-sent as a wrong answer. A manual
            // send overrides this (the user chose to send anyway).
            if !manual && s.meta.awaiting_user {
                return;
            }
            // The user MANUALLY paused the drain (the ⏸ toggle) → hold the queue
            // exactly like awaiting_user: the running turn still finishes, but no
            // queued message auto-advances until they resume. A manual send still
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
            s.in_flight = true;
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

    /// MANUAL drain of the queue head: bypasses the paused / awaiting-user holds
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

    /// Reconcile an authoritative runtime snapshot that says the worker is idle.
    ///
    /// Remote worker lifecycle events can straddle a Machine broker reconnect.
    /// If Cowboy missed the Busy -> Running edge, the Hub may still retain the
    /// dispatch guard even though the worker snapshot proves that no turn is
    /// active. Callers must first prove that no prompt command is still pending
    /// in the controller; otherwise a Running snapshot can merely predate a
    /// prompt that is still travelling to the worker.
    pub fn reconcile_runtime_idle(&self, session_id: &str) {
        let released = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if s.meta.status != Status::Running || !s.in_flight {
                false
            } else {
                s.in_flight = false;
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
        let mut cleared_awaiting = false;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            // Idempotent on cmid (a retry whose original actually landed in the
            // queue must not double-add). The dispatch branch (chat) doesn't
            // store a QueuedMessage, so cmid reconciliation there is Phase 2.
            if let Some(c) = cmid.as_deref()
                && s.queue.iter().any(|m| m.cmid.as_deref() == Some(c))
            {
                return;
            }
            // The user is actively sending → they've engaged with whatever the
            // agent asked, so the "awaiting your reply" state is resolved. Clear it
            // now so the widget vanishes immediately (the next turn re-judges).
            if s.meta.awaiting_user || s.meta.done {
                s.meta.awaiting_user = false;
                s.meta.done = false;
                cleared_awaiting = true;
            }
            if wired && Self::ready(s, true) && s.queue.is_empty() {
                s.in_flight = true;
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
        if cleared_awaiting {
            self.broadcast_sessions();
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
        let wired = self.inner.dispatch_tx.lock().is_some();
        let mut dispatch = None;
        let mut interrupt = false;
        let mut cleared_awaiting = false;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if let Some(c) = cmid.as_deref()
                && s.queue.iter().any(|m| m.cmid.as_deref() == Some(c))
            {
                return false;
            }
            // Explicit send → the "awaiting your reply" state is resolved; clear it.
            if s.meta.awaiting_user || s.meta.done {
                s.meta.awaiting_user = false;
                s.meta.done = false;
                cleared_awaiting = true;
            }
            if wired && Self::ready(s, true) && s.queue.is_empty() {
                // Idle + nothing queued → straight dispatch, identical to submit.
                s.in_flight = true;
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
        if cleared_awaiting {
            self.broadcast_sessions();
        }
        match dispatch {
            Some(req) => self.send_dispatch(req),
            None => self.emit_pending(session_id),
        }
        interrupt
    }

    /// Drop one queued prompt.
    pub fn remove_queued(&self, session_id: &str, id: &str) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.queue.retain(|m| m.id != id);
            if s.editing.as_deref() == Some(id) {
                s.editing = None;
            }
        }
        self.emit_pending(session_id);
        self.try_drain(session_id);
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

    /// Edit a queued prompt in place. Empty text + content removes it.
    pub fn edit_queued(
        &self,
        session_id: &str,
        id: &str,
        text: String,
        content: Vec<serde_json::Value>,
    ) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if text.trim().is_empty() && content.is_empty() {
                s.queue.retain(|m| m.id != id);
                if s.editing.as_deref() == Some(id) {
                    s.editing = None;
                }
            } else if let Some(m) = s.queue.iter_mut().find(|m| m.id == id) {
                m.text = text;
                m.content = content;
            }
        }
        self.emit_pending(session_id);
        self.try_drain(session_id);
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

    /// Overlay "Resume" for an interrupted turn: inject the auto-resume
    /// continuation and drain it now (manual → revives + bypasses holds). A no-op
    /// if there's nothing to continue from.
    pub fn resume_turn(&self, session_id: &str) {
        self.enqueue_continuation(session_id);
        self.drain_head(session_id, true, true);
    }

    /// Overlay "Retry" for an errored/crashed turn: re-run the last user prompt
    /// (reviving the session). No-op if there's no prior prompt.
    pub fn retry_turn(&self, session_id: &str) {
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
        // Drain the head WITH revive (the same path resume_turn uses) so Retry runs
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
    pub fn set_queue_editing(&self, session_id: &str, id: Option<String>) {
        let released = id.is_none();
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.editing = id;
        }
        if released {
            self.try_drain(session_id);
        }
    }

    /// Park a new draft.
    pub fn add_draft(
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
            // Idempotent on cmid: when a send looked failed the client resends
            // with the SAME cmid, but the original may actually have landed — so
            // a matching cmid means "already staged", don't double-add.
            if let Some(c) = cmid.as_deref()
                && s.drafts.iter().any(|m| m.cmid.as_deref() == Some(c))
            {
                return;
            }
            let id = self.next_qid();
            s.drafts.push(QueuedMessage {
                id,
                text,
                content,
                cmid,
                schedule: None,
            });
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
        let mut cleared_awaiting = false;
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
            // A fired schedule is the user's pre-committed "reply" → clear any
            // awaiting-user hold so it isn't stranded behind that widget. (Pause is
            // a separate, deliberate hold and is NOT cleared here.)
            if s.meta.awaiting_user || s.meta.done {
                s.meta.awaiting_user = false;
                s.meta.done = false;
                cleared_awaiting = true;
            }
            if wired && !s.meta.paused && Self::ready(s, true) && s.queue.is_empty() {
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
        if cleared_awaiting {
            self.broadcast_sessions();
        }
        match dispatch {
            Some(req) => self.send_dispatch(req),
            None => self.emit_pending(session_id),
        }
    }

    /// Edit a draft in place. Empty text + content removes it.
    pub fn edit_draft(
        &self,
        session_id: &str,
        id: &str,
        text: String,
        content: Vec<serde_json::Value>,
    ) {
        let unscheduled = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if text.trim().is_empty() && content.is_empty() {
                let had = s.drafts.iter().any(|m| m.id == id && m.schedule.is_some());
                s.drafts.retain(|m| m.id != id);
                had
            } else if let Some(m) = s.drafts.iter_mut().find(|m| m.id == id) {
                m.text = text;
                m.content = content;
                false
            } else {
                false
            }
        };
        self.emit_pending(session_id);
        if unscheduled {
            self.cancel_draft_timer(session_id, id);
            self.broadcast_sessions();
        }
    }

    /// Drop one draft.
    pub fn remove_draft(&self, session_id: &str, id: &str) {
        let unscheduled = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let had = s.drafts.iter().any(|m| m.id == id && m.schedule.is_some());
            s.drafts.retain(|m| m.id != id);
            had
        };
        self.emit_pending(session_id);
        if unscheduled {
            self.cancel_draft_timer(session_id, id);
            self.broadcast_sessions();
        }
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
    pub fn activate_draft(&self, session_id: &str, id: &str) {
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
            // Activating a draft is a server-side move, not a fresh client
            // optimistic send — no cmid.
            self.submit(session_id, m.text, m.content, None);
        }
    }

    /// Activate every draft, front-to-back, then clear them.
    pub fn activate_all_drafts(&self, session_id: &str) {
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
            // Activating a draft is a server-side move, not a fresh client
            // optimistic send — no cmid.
            self.submit(session_id, m.text, m.content, None);
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
    /// Ids not in `order` keep their relative order at the end.
    pub fn reorder_sessions(&self, order: &[String]) {
        {
            let mut list = self.inner.order.lock();
            list.sort_by_key(|id| order.iter().position(|o| o == id).unwrap_or(usize::MAX));
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateSessionOrder {
                order: self.inner.order.lock().clone(),
            });
        }
        self.broadcast_sessions();
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
mod config_preference_tests {
    use super::*;

    #[test]
    fn new_codex_sessions_start_with_luna_max_preferences() {
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
                "model": "gpt-5.6-luna",
                "reasoning_effort": "max",
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
                "model": "deepseek-v4-flash[1m]",
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
                "model": "deepseek-v4-flash",
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

/// Run the single shared Luna classifier worker. The app-server process stays
/// warm, while each judgment gets an isolated ephemeral thread with an identical
/// static prefix. A failed or timed-out request discards the process; the next
/// retry recalibrates it. The queue stays bounded by the server-side channel.
pub async fn run_judge_worker(hub: Hub, mut rx: mpsc::Receiver<JudgeReq>, command: String) {
    let mut judge = crate::inference::codex::CodexJudge::new(command);
    let mut circuit_opened_at: Option<std::time::Instant> = None;
    while let Some(req) = rx.recv().await {
        // The judge is an advisory turn-end classifier, never part of the agent's
        // critical path. When the model endpoint is unhealthy, one slow request
        // must not hold this session for a minute and then serially do the same to
        // every session behind it. Probe once with a short deadline; after a
        // failure, fail open for a cooldown and drain queued requests immediately.
        if circuit_opened_at
            .is_some_and(|opened| opened.elapsed() < std::time::Duration::from_mins(1))
        {
            hub.finish_judge_failure(req, "classifier circuit open");
            continue;
        }
        circuit_opened_at = None;

        let started = std::time::Instant::now();
        let call = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            judge.complete(&req.final_text),
        )
        .await;
        let response = match call {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                let error = error.to_string();
                judge.reset();
                circuit_opened_at = Some(std::time::Instant::now());
                tracing::warn!(session = %req.session_id, %error, "Codex judge failed; opening circuit");
                hub.finish_judge_failure(req, &error);
                continue;
            }
            Err(_) => {
                let error = "Codex judge timed out after 8s";
                judge.reset();
                circuit_opened_at = Some(std::time::Instant::now());
                tracing::warn!(session = %req.session_id, "Codex judge timed out; opening circuit");
                hub.finish_judge_failure(req, error);
                continue;
            }
        };
        match crate::skills::confirm::classify_response(response) {
            Ok(outcome) => {
                if !hub.judge_is_current(&req.session_id, req.seq) {
                    continue;
                }
                let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let (hit, miss) = outcome
                    .usage
                    .as_ref()
                    .map_or((0, 0), |u| (u.cache_hit_tokens, u.cache_miss_tokens));
                tracing::info!(
                    session = %req.session_id,
                    awaiting = outcome.verdict.awaiting_user,
                    done = outcome.verdict.done,
                    confidence = outcome.verdict.confidence,
                    reason = %outcome.verdict.reason,
                    latency_ms,
                    cache_hit = hit,
                    cache_miss = miss,
                    "confirm-detect verdict"
                );
                let at = now_ms();
                if !hub.apply_verdict(&req.session_id, req.seq, &outcome.verdict) {
                    continue;
                }
                hub.emit_and_record_judge(
                    &req.session_id,
                    JudgeRun {
                        id: format!("{at}-{}", req.seq),
                        at,
                        layer: outcome.layer.to_owned(),
                        awaiting_user: outcome.verdict.awaiting_user,
                        done: outcome.verdict.done,
                        confidence: outcome.verdict.confidence,
                        reason: outcome.verdict.reason.clone(),
                        model: crate::inference::codex::MODEL.to_owned(),
                        input: req.final_text.clone(),
                        output: outcome.raw_output,
                        cache_hit: hit,
                        cache_miss: miss,
                        latency_ms,
                    },
                );
                hub.set_judging_if_current(&req.session_id, req.seq, false);
            }
            Err(error) => {
                let error = error.to_string();
                judge.reset();
                circuit_opened_at = Some(std::time::Instant::now());
                tracing::warn!(session = %req.session_id, %error, "parse Codex judge verdict failed; opening circuit");
                hub.finish_judge_failure(req, &error);
            }
        }
    }
}

#[cfg(test)]
mod runtime_reconciliation_tests {
    use super::*;

    fn restored_busy(id: &str) -> RestoredSession {
        RestoredSession {
            meta: SessionMeta {
                id: id.to_owned(),
                provider: "codex".to_owned(),
                machine_id: "hawk".to_owned(),
                workspace_id: None,
                workspace_name: None,
                workspace_source_path: None,
                cwd: "/tmp".to_owned(),
                title: "test".to_owned(),
                status: Status::Busy,
                origin: SessionOrigin::Web,
                agent_session_id: Some("agent-1".to_owned()),
                auto_resume: Some(false),
                awaiting_user: false,
                done: false,
                judging: false,
                paused: false,
                system: false,
                context_used: 0,
                context_size: 0,
                usage: None,
                next_schedule_ms: None,
            },
            log: Vec::new(),
            event_count: 0,
            reached_start: true,
            next_seq: 0,
            queue: Vec::new(),
            drafts: Vec::new(),
            judge_runs: Vec::new(),
            config_options: None,
            config_preferences: serde_json::json!({}),
            mobile_review_state: serde_json::Value::Null,
        }
    }

    fn worker_snapshot(session_id: &str, worker_epoch: &str) -> WorkerSnapshot {
        WorkerSnapshot {
            session_id: session_id.to_owned(),
            worker_epoch: worker_epoch.to_owned(),
            generation: "gen-1".to_owned(),
            executable: None,
            launch: None,
            state: WorkerState::Busy,
            agent_session_id: Some("agent-1".to_owned()),
            current_turn_id: Some("turn-1".to_owned()),
            last_runtime_seq: 7,
            pending_permissions: Vec::new(),
            config_options: None,
            context_used: None,
            context_size: None,
            pending_prompt_count: 0,
            drain_requested: false,
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
    fn immediate_restore_does_not_treat_broker_placeholder_as_live_owner() {
        let hub = Hub::new();
        hub.restore_with_workers(
            vec![restored_busy("session-3")],
            &[worker_snapshot("session-3", "broker-session-3")],
        );

        assert_eq!(hub.status("session-3"), Some(Status::Interrupted));
    }
}

#[cfg(test)]
mod confirm_hold_tests {
    use super::*;

    fn update(seq: u64, kind: &str, text: &str, id: &str, phase: Option<&str>) -> Envelope {
        let mut value = serde_json::json!({
            "sessionUpdate": kind,
            "messageId": id,
            "content": { "type": "text", "text": text }
        });
        if let Some(phase) = phase {
            value["_meta"] = serde_json::json!({ "codex": { "phase": phase } });
        }
        Envelope {
            session_id: "s".to_owned(),
            seq,
            event: Event::Update { update: value },
            cmid: None,
        }
    }

    #[test]
    fn judge_input_prefers_only_codex_final_answer() {
        let log = vec![
            update(1, "user_message_chunk", "task", "u", None),
            update(2, "agent_message_chunk", "working", "a", Some("commentary")),
            update(3, "agent_message_chunk", "done ", "a", Some("final_answer")),
            update(4, "agent_message_chunk", "now", "a", Some("final_answer")),
        ];
        assert_eq!(last_judge_text(&log), "done now");
    }

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

    #[test]
    fn judge_input_falls_back_to_last_agent_message_group() {
        let log = vec![
            update(1, "user_message_chunk", "task", "u", None),
            update(2, "agent_message_chunk", "old", "a", None),
            update(3, "agent_message_chunk", "new ", "b", None),
            update(4, "agent_message_chunk", "answer", "b", None),
        ];
        assert_eq!(last_judge_text(&log), "new answer");
    }

    #[test]
    fn judge_input_truncates_unicode_and_keeps_both_ends() {
        let text = format!("START{}END", "界".repeat(5_000));
        let truncated = truncate_judge_text(&text);
        assert!(truncated.starts_with("START"));
        assert!(truncated.ends_with("END"));
        assert!(truncated.contains("中间已截断"));
        assert_eq!(truncated.chars().count(), 4_096);
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
    fn awaiting_hold_toggles() {
        let hub = hub_with_session("s2");
        assert!(!hub.is_awaiting("s2"));
        hub.set_awaiting("s2", true);
        assert!(hub.is_awaiting("s2"), "judge held the queue");
        hub.set_awaiting("s2", false);
        assert!(!hub.is_awaiting("s2"), "clearing resumes the drain");
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
        let (tx, _rx) = mpsc::channel(HOT_TAIL + HOT_TAIL_TRIM_BATCH + 2);
        let health = std::sync::Arc::new(PersistenceHealth::default());
        let hub = Hub::with_store(Some(StoreSink::new(tx, health)));
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
            let Outbound::Event { envelope } = live.try_recv().expect("raw live frame") else {
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
    fn persisted_hub_bounds_canonical_hot_history_by_payload_bytes() {
        let (tx, _rx) = mpsc::channel(32);
        let health = std::sync::Arc::new(PersistenceHealth::default());
        let hub = Hub::with_store(Some(StoreSink::new(tx, health)));
        hub.create_local_session(
            "byte-hot-tail".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "byte-hot-tail".to_owned(),
            SessionOrigin::Api,
            false,
        );
        let payload = "x".repeat(1024 * 1024);
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

        hub.reconcile_runtime_idle("runtime-reconnect");

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

    #[tokio::test]
    async fn critical_persistence_waits_behind_a_full_event_queue() {
        let (tx, mut rx) = mpsc::channel(1);
        let health = Arc::new(PersistenceHealth::default());
        let sink = StoreSink::new(tx, Arc::clone(&health));
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
        health.consumed(2);
        assert_eq!(health.dropped(), 0);
        assert_eq!(health.pending(), 0);
    }

    #[tokio::test]
    async fn session_broadcast_errors_are_persisted_outside_the_transcript() {
        let (tx, mut rx) = mpsc::channel(4);
        let health = Arc::new(PersistenceHealth::default());
        let hub = Hub::with_store(Some(StoreSink::new(tx, health)));
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
        assert!(matches!(
            live.recv().await,
            Ok(Outbound::Error {
                session_id: Some(session_id),
                message,
            }) if session_id == "session-1" && message == "runtime rejected command"
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
