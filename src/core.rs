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

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use parking_lot::Mutex;

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

/// How many recent events a fresh client gets over WS (the live tail). Older
/// history is paged in over HTTP. Sized to comfortably fill a few phone screens.
pub const SNAPSHOT_TAIL: usize = 200;
/// Fixed history page size (events) for the HTTP `/api/history` route. Fixed +
/// seq-aligned so each page has a STABLE url → safe to cache `immutable`.
pub const HISTORY_PAGE: usize = 200;

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

/// Provider/session status as shown in the session list.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Agent subprocess spawning / ACP handshake in flight.
    Starting,
    /// Session established; idle, ready for a prompt.
    Running,
    /// A prompt turn is currently being processed.
    Busy,
    /// Agent exited cleanly (or was stopped).
    Exited,
    /// Agent crashed / the ACP connection failed.
    Crashed,
    /// A turn was in flight when the daemon went down (detected at restore from a
    /// persisted `Busy`). The agent subprocess is gone — like `Exited`/`Crashed`
    /// this is a settled, resumable-via-new-turn state — but it carries the extra
    /// fact that the last turn never finished, so the UI can say so instead of
    /// showing a plain "dormant". Only ever set by [`Hub::restore`].
    Interrupted,
}

/// A normalized session event fanned out to clients.
///
/// `Update` is a pass-through of an ACP `SessionUpdate` (message/thought
/// chunks, tool calls, plan, available commands, mode). The remaining variants
/// are cowboy-specific control events the protocol doesn't model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A serialized ACP `SessionUpdate` (see module docs).
    Update { update: serde_json::Value },
    /// The agent is asking to proceed; carries the options for the UI to
    /// render. `first response wins` (design §5): the first answer resolves it.
    PermissionRequest {
        request_id: String,
        tool_call: serde_json::Value,
        options: serde_json::Value,
    },
    /// A permission request was answered, so other clients clear their buttons.
    PermissionResolved {
        request_id: String,
        option_id: Option<String>,
    },
    /// Process lifecycle transition (status + optional human detail).
    Lifecycle {
        status: Status,
        detail: Option<String>,
    },
    /// A prompt turn finished (carries the ACP stop reason as a string).
    TurnEnd { stop_reason: String },
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

// --- Auto-resume interrupted turns (tasks/active/session-auto-resume) ---------

/// Settings key: the global default for auto-resuming interrupted turns (bool).
const AUTO_RESUME_DEFAULT_KEY: &str = "session.autoResume.default";
/// Settings key: the continuation-message template (string with `{{var}}` holes).
const AUTO_RESUME_TEMPLATE_KEY: &str = "session.autoResume.template";
/// cmid prefix tagging an auto-enqueued continuation, so it's deduped (never
/// stacked) and recognizable. (The cmid isn't persisted across restart — it only
/// guards stacking WITHIN the queue at enqueue time, which is where the pile-up
/// risk is.) Also read in `acp.rs` to flag the echo as `autoResumed` so the UI
/// renders it as a continuation note rather than a user bubble.
pub(crate) const AUTO_CONTINUE_PREFIX: &str = "__cont__";
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
            let kind = update.get("sessionUpdate").and_then(serde_json::Value::as_str)?;
            let text = update
                .get("content")
                .and_then(|c| c.get("text"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            return Some((kind.to_owned(), text.to_owned()));
        }
        None
    };
    let last_user = log.iter().rposition(|env| {
        matches!(chunk(env), Some((ref k, _)) if k == "user_message_chunk")
    });
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
    /// never (explicit opt-out). See tasks/active/session-auto-resume.
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
    /// True for the machine-driven memory janitor SYSTEM session: visible and
    /// watchable in the UI but VIEW-ONLY — the composer is hidden and user turns
    /// are rejected; only the backend wake endpoint drives it. Persisted
    /// (migration 0010) so it survives a daemon restart. See mnemosyne.
    #[serde(default)]
    pub system: bool,
    /// True while the agent has a **background process still running between
    /// turns** — e.g. a `run_in_background` build it ended its turn to wait on.
    /// ACP gives no signal for this; it's derived by the proc-watcher reading the
    /// agent's cgroup (see [`crate::procwatch`]). Like `working`, it suppresses
    /// the "Waiting for your reply" / "Queue paused" overlay (the agent isn't
    /// idle, it's waiting on its own work) and drives a "background task" widget.
    /// Transient — never persisted, recomputed live; `serde(default)` covers old
    /// clients + the restore path.
    #[serde(default)]
    pub background_task: bool,
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

/// One session's full persisted state, handed to [`Hub::restore`] at startup.
pub struct RestoredSession {
    pub meta: SessionMeta,
    pub log: Vec<Envelope>,
    pub next_seq: u64,
    pub queue: Vec<QueuedMessage>,
    pub drafts: Vec<QueuedMessage>,
    /// Persisted confirm-detect judge-run history (newest first), capped.
    pub judge_runs: Vec<JudgeRun>,
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
    /// "L1" (deterministic stop-reason) or "L2" (the DeepSeek judge).
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
    log: Vec<Envelope>,
    next_seq: u64,
    /// Last seen agent-advertised config options (raw ACP
    /// `configOptions` array — see acp.rs intercept). `None` until the agent
    /// fires its first `config_option_update` notification. Re-sent to every
    /// new client on connect so the composer dropdowns populate from a fresh
    /// reload.
    config_options: Option<serde_json::Value>,
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
    /// Monotonic counter bumped on every turn-end judge dispatch. The async
    /// confirm-detect verdict carries the seq it was issued under and is applied
    /// ONLY if still current — a newer turn-end supersedes a stale verdict (the
    /// stale-clear race in plan.md Risks). Not persisted; resets to 0 on restart.
    judge_seq: u64,
    /// Confirm-detect judge-run history (newest first), capped at
    /// [`JUDGE_HISTORY_CAP`]. Server-authoritative + persisted (migration 0009);
    /// backs the inspector widget. Broadcast as [`Outbound::JudgeHistory`].
    judge_runs: Vec<JudgeRun>,
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
    /// `SessionMeta`. See tasks/active/session-auto-resume.
    SetSessionAutoResume {
        session_id: String,
        #[serde(default)]
        value: Option<bool>,
    },
    /// Clear/set a session's confirm-detect "awaiting user" hold from the awaiting
    /// widget. The user dismissing it (`false`) means "not a question" → the hold
    /// lifts and the queue drains. Broadcasts the updated session list.
    SetAwaiting {
        session_id: String,
        awaiting: bool,
    },
    /// User toggle: manually pause/resume the queue drain. Holds the auto-drain
    /// without interrupting the running turn (see [`Hub::set_paused`]).
    SetPaused {
        session_id: String,
        paused: bool,
    },
    /// Overlay action: resume an interrupted turn (inject the continuation + run).
    ResumeTurn {
        session_id: String,
    },
    /// Overlay action: retry an errored/crashed turn (re-run the last prompt).
    RetryTurn {
        session_id: String,
    },
    /// Set one global setting (`session.autoResume.default` flag /
    /// `session.autoResume.template` string). Persisted + broadcast to every
    /// surface as [`Outbound::Settings`].
    SetSetting {
        key: String,
        value: serde_json::Value,
    },
    /// Set an inference provider's non-secret config (model + params). Persisted
    /// + broadcast as [`Outbound::InferenceConfig`].
    SetInferenceConfig {
        provider: String,
        model: String,
        #[serde(default)]
        params: serde_json::Value,
    },
    /// Set an inference provider's API key. Persisted to a SEPARATE table; the
    /// broadcast only flips `key_set` — the key itself never leaves the daemon.
    SetInferenceSecret {
        provider: String,
        api_key: String,
    },
    /// DEV probe: call the inference provider once with `prompt` and broadcast an
    /// [`Outbound::InferenceProbeResult`] (text + cache token counts). Proves the
    /// key/model/HTTP wiring end to end.
    InferenceProbe {
        provider: String,
        #[serde(default)]
        prompt: String,
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
    /// claude-agent-acp ≥ 0.31 exposes a unified `session/setSessionConfigOption`
    /// request that handles all three via the same shape. cowboy sends it as
    /// an ACP `ext_method` (the 0.4 crate lacks a typed wrapper); the agent
    /// answers with the refreshed `configOptions` array, which the daemon
    /// then re-broadcasts as [`Outbound::ConfigOptions`].
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
    /// `context_cleared` marker into the timeline for the UI's divider. The
    /// transcript history is kept (a scroll-back record); only the agent forgets.
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
    /// Replay of one session's RECENT log tail (sent on connect, after
    /// `Sessions`). Capped to the last [`SNAPSHOT_TAIL`] events so a long
    /// session doesn't ship its whole history on every connect; older pages are
    /// fetched on demand via `LoadHistory` → `History`. `reached_start` is true
    /// when these events ARE the whole log (nothing older to page to).
    Snapshot {
        session_id: String,
        events: Vec<Envelope>,
        reached_start: bool,
    },
    /// A single live event.
    Event { envelope: Envelope },
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
    /// The inference-provider configs (model + params + whether a key is set —
    /// NEVER the key). Sent on connect + re-broadcast on every edit.
    InferenceConfig {
        providers: Vec<InferenceView>,
    },
    /// The registered skills (id/title/description + the prompt template + the
    /// extraction rule), so the Info sheet can render each skill's prompt verbatim.
    /// Static — sent once on connect.
    Skills {
        skills: Vec<SkillView>,
    },
    /// The confirm-detect judge's full result for a turn — the verdict PLUS the
    /// observability detail the overlay's "raw data" expand shows. Sent after each
    /// judge; the client keeps the latest per session. NOT persisted.
    JudgeResult {
        session_id: String,
        /// "L1" (deterministic stop-reason) or "L2" (the DeepSeek judge).
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
    /// Result of an [`Inbound::InferenceProbe`] — surfaced in the Info sheet.
    InferenceProbeResult {
        provider: String,
        ok: bool,
        #[serde(default)]
        text: String,
        cache_hit: u32,
        cache_miss: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
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
/// [`crate::store::Store`] call. The channel is unbounded so the hot path
/// (`Hub::push`) never blocks; a slow DB causes memory growth, not WS lag.
#[derive(Debug, Clone)]
pub enum StoreWrite {
    InsertSession(SessionMeta),
    AppendEvent(Envelope),
    UpdateStatus { session_id: String, status: Status },
    /// Persist the confirm-detect turn-end verdict (so a done/awaiting session
    /// survives a daemon restart — migration 0008).
    UpdateVerdict { session_id: String, awaiting_user: bool, done: bool },
    UpdateTitle { session_id: String, title: String },
    SetAgentSessionId {
        session_id: String,
        agent_session_id: String,
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
    UpdateSessionOrder { order: Vec<String> },
    /// Persist a session's auto-resume OVERRIDE (`None` = inherit global default).
    UpdateAutoResume { session_id: String, value: Option<bool> },
    /// Persist a session's confirm-detect judge-run history (whole list, as JSONB
    /// — migration 0009). Written on every add / delete / clear, like
    /// [`StoreWrite::UpdatePending`].
    UpdateJudgeRuns { session_id: String, runs: Vec<JudgeRun> },
    /// Upsert one global setting (auto-resume default flag / continuation template).
    PutSetting { key: String, value: serde_json::Value },
    /// Upsert an inference provider's non-secret config.
    PutInferenceConfig { provider: String, model: String, params: serde_json::Value },
    /// Upsert an inference provider's API key (separate secrets table).
    PutInferenceSecret { provider: String, api_key: String },
    /// Upsert a session's pending `ScheduleWakeup` (migration 0011) so an armed
    /// wakeup survives a daemon restart and still fires.
    UpsertWakeup { session_id: String, fire_at_ms: i64, prompt: String },
    /// Drop a session's persisted wakeup once it has fired (or been dropped).
    DeleteWakeup { session_id: String },
}

/// Client-facing view of one inference provider's config — carries `key_set`
/// (whether an API key exists) but NEVER the key itself.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InferenceView {
    pub provider: String,
    pub model: String,
    pub params: serde_json::Value,
    pub key_set: bool,
    /// Selectable models (id + human label) for this provider — the UI renders the
    /// model dropdown from THIS, never hardcoding ids (Step 18). Sourced from the
    /// provider's `ModelSource`; empty for a provider with no known list.
    pub models: Vec<ModelOption>,
}

/// One selectable model for a provider's dropdown (the client-facing form of a
/// `ModelSource::Static` entry).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
}

/// The provider's selectable models as wire options. Reads the provider's
/// `ModelSource` (dynamic by design — ids churn); empty for an unknown provider.
/// The `/models`-endpoint hook lives on `ModelSource` for when a provider needs it.
fn provider_models(provider: &str) -> Vec<ModelOption> {
    let src = match provider {
        "deepseek" => crate::inference::deepseek::DeepSeek::model_list(),
        _ => Vec::new(),
    };
    src.into_iter().map(|(id, label)| ModelOption { id, label }).collect()
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

/// In-memory inference entry: the non-secret config PLUS the API key (read by the
/// judge call, never broadcast).
#[derive(Debug, Clone, Default)]
struct InferenceEntry {
    model: String,
    params: serde_json::Value,
    api_key: Option<String>,
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
    /// Insertion order of session ids, so the list view is stable.
    order: Mutex<Vec<String>>,
    /// Live fan-out to all connected clients. Lagging receivers are dropped by
    /// `broadcast` and simply miss events until their next reconnect snapshot.
    tx: broadcast::Sender<Outbound>,
    /// Optional write-behind channel to the DB writer. `None` ⇒ in-memory
    /// only (no `--postgres-url` configured).
    store_tx: Option<mpsc::UnboundedSender<StoreWrite>>,
    /// Hand-off to the background dispatcher task that owns the `Supervisor`.
    /// Set once at startup via [`Hub::set_dispatch_tx`]; `None` until then (and
    /// in tests), in which case a drain decision is computed but no prompt is
    /// actually sent. See [`DispatchReq`].
    dispatch_tx: Mutex<Option<mpsc::UnboundedSender<DispatchReq>>>,
    /// Hand-off to the background scheduler task that fires agent-armed
    /// `ScheduleWakeup`s. Set once at startup via [`Hub::set_scheduler_tx`];
    /// `None` until then (and in tests) ⇒ wakeups are simply not honored.
    scheduler_tx: Mutex<Option<mpsc::UnboundedSender<crate::scheduler::ScheduleCmd>>>,
    /// Per-state arbiters for the generic optimistic-sync channel, keyed by
    /// state name (`"title"`, `"order"`, …). See [`SyncArbiter`].
    sync: Mutex<HashMap<String, SyncArbiter>>,
    /// Monotonic source of queued/draft message ids (`q1`, `q2`, …). Seeded from
    /// the wall-clock-free counter; uniqueness across a daemon lifetime is all
    /// that's required (ids are list-local keys, not persisted-across-restart
    /// identities — restored lists keep whatever ids they were saved with).
    next_qid: AtomicU64,
    /// Global key-value settings (auto-resume default flag + continuation
    /// template), mirrored from the `settings` table on restore. Authoritative
    /// in-memory; every edit also write-behinds via `StoreWrite::PutSetting`.
    settings: Mutex<HashMap<String, serde_json::Value>>,
    /// In-memory inference-provider configs keyed by provider id (model/params +
    /// the API key the judge reads). Seeded from the DB on restore; every edit
    /// write-behinds + re-broadcasts (key_set only).
    inference: Mutex<HashMap<String, InferenceEntry>>,
}

impl Hub {
    #[must_use]
    pub fn new() -> Self {
        Self::with_store(None)
    }

    /// Hub plus a write-behind channel. The receiver half is owned by the
    /// DB writer task (spawned in `crate::server`).
    #[must_use]
    pub fn with_store(store_tx: Option<mpsc::UnboundedSender<StoreWrite>>) -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            inner: std::sync::Arc::new(HubInner {
                sessions: Mutex::new(HashMap::new()),
                order: Mutex::new(Vec::new()),
                tx,
                store_tx,
                dispatch_tx: Mutex::new(None),
                scheduler_tx: Mutex::new(None),
                sync: Mutex::new(HashMap::new()),
                next_qid: AtomicU64::new(1),
                settings: Mutex::new(HashMap::new()),
                inference: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Wire the background dispatcher's hand-off channel. Called once at startup
    /// (in `crate::server`) after the dispatcher task is spawned, before any
    /// client connects. Until set, drains compute but dispatch nothing.
    pub fn set_dispatch_tx(&self, tx: mpsc::UnboundedSender<DispatchReq>) {
        *self.inner.dispatch_tx.lock() = Some(tx);
    }

    /// Wire the background scheduler's hand-off channel (mirrors
    /// [`Self::set_dispatch_tx`]). Until set, [`Self::schedule_wakeup`] is a no-op.
    pub fn set_scheduler_tx(&self, tx: mpsc::UnboundedSender<crate::scheduler::ScheduleCmd>) {
        *self.inner.scheduler_tx.lock() = Some(tx);
    }

    /// Arm (replace) a session's pending `ScheduleWakeup` — `acp.rs` calls this
    /// when it intercepts the tool. `delay_seconds` is the agent-requested delay
    /// (clamped by the scheduler); the wakeup fires `prompt` as its own turn.
    /// Also persisted (migration 0011) so it survives a restart.
    pub fn schedule_wakeup(&self, session_id: &str, delay_seconds: i64, prompt: String) {
        let fire_at_ms = crate::scheduler::fire_at_from_delay(delay_seconds);
        if let Some(tx) = self.inner.scheduler_tx.lock().as_ref() {
            let _ = tx.send(crate::scheduler::ScheduleCmd::Arm {
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
            let _ = tx.send(crate::scheduler::ScheduleCmd::Arm {
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
            let _ = tx.send(crate::scheduler::ScheduleCmd::HumanTurn {
                session_id: session_id.to_owned(),
            });
        }
    }

    /// Populate the in-memory state from a previously-stored snapshot.
    /// Should be called once at startup, BEFORE any client connects, so the
    /// `Sessions` broadcast on first connect already includes everything.
    /// Skips the write-behind side: these rows are already in the DB.
    ///
    /// **Restored sessions are forced to a dead state.** The agent subprocess
    /// does not come back across a daemon restart — the postgres state is
    /// metadata + history only, not a live ACP connection. Letting a restored
    /// row keep its persisted `Running`/`Busy` status creates the trap of a UI
    /// that looks alive but rejects every prompt with `unknown session` (no
    /// `agent_tx` in the supervisor). So every restored session is dead +
    /// disabled; resume via session/load is a future follow-up (design §7).
    ///
    /// But the persisted status still tells us WHAT it was doing when we died,
    /// and we keep that one bit: a session that was `Busy` (a turn in flight)
    /// becomes [`Status::Interrupted`] — "your last turn never finished" — while
    /// an idle/alive one just becomes `Exited` (dormant, nothing unfinished).
    /// The write-behind store applies the `Busy` write within ms of a turn
    /// starting, so for any turn that ran more than an instant the bit is
    /// durable before a restart (store.rs accepts the sub-ms crash window).
    pub fn restore(&self, sessions: Vec<RestoredSession>) {
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
            for r in sessions {
                let RestoredSession {
                    mut meta,
                    log,
                    next_seq,
                    mut queue,
                    mut drafts,
                    judge_runs,
                } = r;
                let mut healed = false;
                for m in queue.iter_mut().chain(drafts.iter_mut()) {
                    if !seen.insert(m.id.clone()) {
                        m.id = self.next_qid();
                        seen.insert(m.id.clone());
                        healed = true;
                    }
                }
                let was_busy = meta.status == Status::Busy;
                meta.status = match meta.status {
                    // Mid-turn when we died → the work was cut off, unfinished.
                    Status::Busy => Status::Interrupted,
                    // Already a settled dead state (incl. a prior Interrupted that
                    // was never resumed) → keep it as recorded.
                    Status::Exited | Status::Crashed | Status::Interrupted => meta.status,
                    // Alive but idle, or still spinning up → just dormant.
                    Status::Running | Status::Starting => Status::Exited,
                };
                let id = meta.id.clone();
                if was_busy {
                    interrupted.push(id.clone());
                }
                if healed {
                    reid_dirty.push(id.clone());
                }
                sessions_lock.insert(
                    id.clone(),
                    Session {
                        meta,
                        log,
                        next_seq,
                        config_options: None,
                        queue,
                        drafts,
                        editing: None,
                        in_flight: false,
                        judge_seq: 0,
                        judge_runs,
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
            if let Some(tx) = self.inner.store_tx.as_ref() {
                let _ = tx.send(StoreWrite::UpdateStatus {
                    session_id: id.clone(),
                    status: Status::Interrupted,
                });
            }
            self.push(
                &id,
                Event::Lifecycle {
                    status: Status::Interrupted,
                    detail: Some(
                        "turn cut off by a cowboy restart — it never finished".to_owned(),
                    ),
                },
            );
            // Auto-resume (opted in, globally or per-session): enqueue a
            // continuation built from the cut-off turn's partial output. It stays
            // queued behind the `Interrupted` marker and auto-drains the instant
            // the agent revives (on open / reconnect), so the user finds the turn
            // already continuing instead of having to retype.
            if self.effective_auto_resume(&id) {
                self.enqueue_continuation(&id);
            }
        }
        // Persist any session whose duplicate ids we healed, so the corrected
        // (unique-id) lists reach the DB + every client.
        for id in reid_dirty {
            self.emit_pending(&id);
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
            .filter_map(|id| sessions.get(id).map(|s| s.meta.clone()))
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
            event_count: u64::try_from(s.log.len()).unwrap_or(u64::MAX),
            queue_count: s.queue.len(),
            drafts_count: s.drafts.len(),
        })
    }

    /// Whether a session is a machine-driven VIEW-ONLY system session (the
    /// mnemosyne memory janitor). The WS dispatch rejects user-driven turns for
    /// these; only the backend wake endpoint (`POST /api/sessions/{id}/prompt`)
    /// drives them.
    #[must_use]
    pub fn session_is_system(&self, session_id: &str) -> bool {
        let sessions = self.inner.sessions.lock();
        sessions.get(session_id).is_some_and(|s| s.meta.system)
    }

    /// Total events held in memory across all live sessions — the event-count
    /// metric for the info panel.
    #[must_use]
    pub fn event_total(&self) -> u64 {
        let sessions = self.inner.sessions.lock();
        sessions.values().map(|s| u64::try_from(s.log.len()).unwrap_or(u64::MAX)).sum()
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
            let start = len.saturating_sub(SNAPSHOT_TAIL);
            (s.log[start..].to_vec(), start == 0)
        })
    }

    /// One fixed-size, seq-aligned page of history for the HTTP history route:
    /// page `k` is events with seq in `[k·HISTORY_PAGE, (k+1)·HISTORY_PAGE)`.
    /// Since seqs are contiguous from 0, that's a direct index slice. Returns the
    /// events plus `immutable` = whether the page can never change again (the
    /// NEXT page has started, so nothing more will land in this one) — the HTTP
    /// handler turns that into a long-lived `Cache-Control: immutable`. An
    /// out-of-range page yields an empty slice.
    #[must_use]
    pub fn history_page(&self, session_id: &str, page: usize) -> Option<(Vec<Envelope>, bool)> {
        let sessions = self.inner.sessions.lock();
        sessions.get(session_id).map(|s| {
            // Page k = events whose SEQ is in [k·P, (k+1)·P). Sliced by SEQ, not
            // log index: seqs aren't always contiguous from 0 (some get assigned
            // without landing in the log), so an index slice returns a window
            // that doesn't line up with the client's seq-based paging and the
            // client never advances. The log is seq-sorted → binary-search bounds.
            let lo_seq = (page as u64).saturating_mul(HISTORY_PAGE as u64);
            let hi_seq = lo_seq.saturating_add(HISTORY_PAGE as u64);
            let lo = s.log.partition_point(|e| e.seq < lo_seq);
            let hi = s.log.partition_point(|e| e.seq < hi_seq);
            let events = s.log[lo..hi].to_vec();
            // Complete (immutable) once an event with seq ≥ hi_seq exists —
            // nothing more can land in this page. The latest page can still grow.
            let immutable = hi < s.log.len();
            (events, immutable)
        })
    }

    /// Register a new session in `Starting` state and broadcast the new list.
    pub fn create_session(
        &self,
        id: String,
        provider: String,
        cwd: String,
        title: String,
        origin: SessionOrigin,
        system: bool,
    ) {
        let meta = SessionMeta {
            id: id.clone(),
            provider,
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
            background_task: false,
        };
        {
            let mut sessions = self.inner.sessions.lock();
            let mut order = self.inner.order.lock();
            sessions.insert(
                id.clone(),
                Session {
                    meta: meta.clone(),
                    log: Vec::new(),
                    next_seq: 0,
                    config_options: None,
                    queue: Vec::new(),
                    drafts: Vec::new(),
                    editing: None,
                    in_flight: false,
                    judge_seq: 0,
                    judge_runs: Vec::new(),
                },
            );
            order.push(id);
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::InsertSession(meta));
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
        let _ = self.inner.tx.send(Outbound::Settings { settings: snapshot });
    }

    /// Seed the in-memory settings map from the persisted table (restore only).
    pub fn load_settings(&self, entries: Vec<(String, serde_json::Value)>) {
        let mut s = self.inner.settings.lock();
        for (k, v) in entries {
            s.insert(k, v);
        }
    }

    /// Snapshot of inference configs for the connect-time push + broadcast —
    /// `key_set` only, NEVER the key. Sorted by provider for a stable UI order.
    #[must_use]
    pub fn inference_snapshot(&self) -> Vec<InferenceView> {
        let m = self.inner.inference.lock();
        let mut v: Vec<InferenceView> = m
            .iter()
            .map(|(p, e)| InferenceView {
                provider: p.clone(),
                model: e.model.clone(),
                params: e.params.clone(),
                key_set: e.api_key.is_some(),
                models: provider_models(p),
            })
            .collect();
        // Always surface deepseek (the judge provider) even before it's configured,
        // so the Info sheet can render its model dropdown + the "set a key" state on
        // a fresh install. `key_set:false` until a key is stored.
        if !v.iter().any(|x| x.provider == "deepseek") {
            v.push(InferenceView {
                provider: "deepseek".to_owned(),
                model: crate::inference::deepseek::DEFAULT_MODEL.to_owned(),
                params: serde_json::json!({}),
                key_set: false,
                models: provider_models("deepseek"),
            });
        }
        v.sort_by(|a, b| a.provider.cmp(&b.provider));
        v
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

    /// Set a provider's non-secret config; persist + broadcast.
    pub fn set_inference_config(&self, provider: String, model: String, params: serde_json::Value) {
        {
            let mut m = self.inner.inference.lock();
            let e = m.entry(provider.clone()).or_default();
            e.model = model.clone();
            e.params = params.clone();
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::PutInferenceConfig { provider, model, params });
        }
        let _ = self.inner.tx.send(Outbound::InferenceConfig { providers: self.inference_snapshot() });
    }

    /// Set a provider's API key; persist (separate table) + broadcast (key_set only).
    pub fn set_inference_secret(&self, provider: String, api_key: String) {
        {
            let mut m = self.inner.inference.lock();
            m.entry(provider.clone()).or_default().api_key = Some(api_key.clone());
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::PutInferenceSecret { provider, api_key });
        }
        let _ = self.inner.tx.send(Outbound::InferenceConfig { providers: self.inference_snapshot() });
    }

    /// Seed inference state from the persisted tables (restore only).
    pub fn load_inference(
        &self,
        configs: Vec<(String, String, serde_json::Value)>,
        keys: Vec<(String, String)>,
    ) {
        let mut m = self.inner.inference.lock();
        for (provider, model, params) in configs {
            let e = m.entry(provider).or_default();
            e.model = model;
            e.params = params;
        }
        for (provider, api_key) in keys {
            m.entry(provider).or_default().api_key = Some(api_key);
        }
    }

    /// The API key for `provider` — INTERNAL (the judge call). Never broadcast.
    #[must_use]
    pub fn inference_key(&self, provider: &str) -> Option<String> {
        self.inner.inference.lock().get(provider).and_then(|e| e.api_key.clone())
    }

    /// The configured model for `provider` (the caller applies a default if unset).
    #[must_use]
    pub fn inference_model(&self, provider: &str) -> Option<String> {
        self.inner.inference.lock().get(provider).map(|e| e.model.clone())
    }

    /// Whether the confirm-detect judge can run — i.e. the `deepseek` provider has
    /// an API key. When false, the drain holds everything (§J no-token block).
    #[must_use]
    pub fn confirm_key_present(&self) -> bool {
        self.inner.inference.lock().get("deepseek").is_some_and(|e| e.api_key.is_some())
    }

    /// Whether a session is currently holding for "awaiting user".
    #[must_use]
    pub fn is_awaiting(&self, session_id: &str) -> bool {
        self.inner.sessions.lock().get(session_id).is_some_and(|s| s.meta.awaiting_user)
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

    /// Set whether a session has a background process running between turns —
    /// driven by [`crate::procwatch`] reading the agent's cgroup. Broadcast-only
    /// (transient, never persisted); no-op if unchanged. Drives the
    /// "background task" widget and suppresses the idle/awaiting overlay.
    pub fn set_background_task(&self, session_id: &str, background_task: bool) {
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            match sessions.get_mut(session_id) {
                Some(s) if s.meta.background_task != background_task => {
                    s.meta.background_task = background_task;
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

    /// Apply a confirm-detect verdict under the `judge_seq` stale-guard: set the
    /// hold to `v.awaiting_user` only if no newer turn-end has superseded `seq`.
    /// Broadcasts and resumes the drain when the hold clears.
    fn apply_verdict(&self, session_id: &str, seq: u64, v: &crate::skills::Verdict) {
        let resume = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if s.judge_seq != seq {
                return; // a newer turn-end already re-judged — drop this verdict
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
    }

    /// Turn-end hook: classify the agent's last message with the confirm-detect
    /// skill. L1 (the agent provider's deterministic stop-reason rule) runs inline
    /// and short-circuits; only an ambiguous `EndTurn` spawns the async L2 judge.
    ///
    /// Recall-first (design §I): before the async judge, set `awaiting_user=true`
    /// synchronously so the drain can't release a queued prompt as a wrong answer.
    /// The verdict is applied under the `judge_seq` guard — `false` clears + drains,
    /// `true`/error keeps the hold (continuity over a wrong drain). `done` is logged
    /// for a future "task complete" notification hook.
    fn judge_turn_end(&self, session_id: &str) {
        let (final_text, seq, provider, stop_reason) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.judge_seq = s.judge_seq.wrapping_add(1);
            (
                last_turn_texts(&s.log).1,
                s.judge_seq,
                s.meta.provider.clone(),
                last_stop_reason(&s.log),
            )
        };
        // L1: deterministic, no LLM. A cut-off/cancelled turn settles here; only a
        // normal `EndTurn` returns None and falls through to L2.
        if let Some(v) =
            crate::provider::confirm::l1(&provider, &crate::provider::confirm::TurnEndCtx {
                stop_reason: stop_reason.as_deref(),
                final_text: &final_text,
            })
        {
            let at = now_ms();
            self.emit_and_record_judge(session_id, JudgeRun {
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
            });
            self.apply_verdict(session_id, seq, &v);
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
        let Some(key) = self.inference_key("deepseek") else {
            self.set_judging(session_id, false); // never dispatched → not judging
            return; // confirm_key_present() gated us in, but be defensive
        };
        let model = self
            .inference_model("deepseek")
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| crate::inference::deepseek::DEFAULT_MODEL.to_owned());
        let hub = self.clone();
        let sid = session_id.to_owned();
        tokio::spawn(async move {
            let ds = crate::inference::deepseek::DeepSeek::new(key, model.clone());
            let started = std::time::Instant::now();
            match crate::skills::confirm::classify(&provider, Some("EndTurn"), &ds, &final_text).await
            {
                Ok(o) => {
                    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    let (hit, miss) = o.usage.as_ref().map_or((0, 0), |u| (u.cache_hit_tokens, u.cache_miss_tokens));
                    tracing::info!(
                        session = %sid, awaiting = o.verdict.awaiting_user, done = o.verdict.done,
                        confidence = o.verdict.confidence, reason = %o.verdict.reason, latency_ms,
                        "confirm-detect verdict"
                    );
                    let at = now_ms();
                    hub.emit_and_record_judge(&sid, JudgeRun {
                        id: format!("{at}-{seq}"),
                        at,
                        layer: o.layer.to_owned(),
                        awaiting_user: o.verdict.awaiting_user,
                        done: o.verdict.done,
                        confidence: o.verdict.confidence,
                        reason: o.verdict.reason.clone(),
                        model,
                        input: final_text,
                        output: o.raw_output,
                        cache_hit: hit,
                        cache_miss: miss,
                        latency_ms,
                    });
                    hub.apply_verdict(&sid, seq, &o.verdict);
                    hub.set_judging(&sid, false);
                }
                // Error → STAY held (the provisional true stands).
                Err(e) => {
                    tracing::warn!(session = %sid, error = %e, "confirm-detect judge failed; holding queue");
                    hub.set_judging(&sid, false);
                }
            }
        });
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
            if s
                .queue
                .iter()
                .any(|m| m.cmid.as_deref().is_some_and(|c| c.starts_with(AUTO_CONTINUE_PREFIX)))
            {
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
                render_template(DEFAULT_RETRY_TEMPLATE, &[("prompt", &prompt), ("cwd", &s.meta.cwd)])
            } else {
                render_template(
                    &template,
                    &[("partial", &partial), ("prompt", &prompt), ("cwd", &s.meta.cwd)],
                )
            };
            let id = self.next_qid();
            let cmid = format!("{AUTO_CONTINUE_PREFIX}{id}");
            // FRONT of the queue: the interrupted turn was running BEFORE anything
            // queued behind it, so its continuation must run first (the queue was
            // *waiting on* that turn), not after the backlog.
            s.queue.insert(0, QueuedMessage { id, text, content: Vec::new(), cmid: Some(cmid) });
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
            let _ = tx.send(StoreWrite::UpdateTitle { session_id: session_id.to_owned(), title });
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

    /// The derived JSON value of one synced state — what a `SyncPatch` carries and
    /// the client folds. Always read live from the typed truth (so it's durable by
    /// derivation, no shadow copy to drift).
    fn sync_value(&self, state: &str) -> serde_json::Value {
        match state {
            "title" => {
                let sessions = self.inner.sessions.lock();
                let map: serde_json::Map<String, serde_json::Value> = sessions
                    .values()
                    .map(|s| (s.meta.id.clone(), serde_json::Value::String(s.meta.title.clone())))
                    .collect();
                serde_json::Value::Object(map)
            }
            "order" => {
                let list = self.inner.order.lock();
                serde_json::Value::Array(list.iter().map(|id| serde_json::Value::String(id.clone())).collect())
            }
            _ => serde_json::Value::Null,
        }
    }

    /// Record `id` as seen for `state`; returns true if it's NEW (first delivery).
    fn sync_first_seen(&self, state: &str, id: &str) -> bool {
        let mut reg = self.inner.sync.lock();
        reg.entry(state.to_owned()).or_default().seen.insert(id.to_owned())
    }

    /// Cmids carried inside a queue/drafts value — the confirm set for the queue
    /// sync state (the client drops an optimistic add the moment its cmid lands).
    fn cmids_of(queue: &[QueuedMessage], drafts: &[QueuedMessage]) -> Vec<String> {
        queue.iter().chain(drafts.iter()).filter_map(|m| m.cmid.clone()).collect()
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
        let _ = self
            .inner
            .tx
            .send(Outbound::SyncPatch { state: state.to_owned(), version, value, confirmed, resync: false });
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
    pub fn sync_apply(&self, state: &str, id: String, name: &str, args: &serde_json::Value) -> Result<(), String> {
        enum Op {
            Rename { session_id: String, title: String },
            Reorder { order: Vec<String> },
        }
        let op = match (state, name) {
            ("title", "rename") => {
                let session_id = args
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("rename: missing session_id")?
                    .to_owned();
                let title = args.get("title").and_then(serde_json::Value::as_str).unwrap_or("").trim().to_owned();
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
            _ => return Err(format!("unknown sync mutation {state}/{name}")),
        };
        if !self.sync_first_seen(state, &id) {
            return Ok(()); // duplicate delivery/retry — already applied + broadcast
        }
        match op {
            Op::Rename { session_id, title } => self.apply_rename(&session_id, title),
            Op::Reorder { order } => self.apply_reorder(&order),
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
                .filter(|(s, _)| !s.starts_with("queue:"))
                .map(|(s, e)| (s.clone(), e.version, e.seen.iter().cloned().collect()))
                .collect();
            // Guarantee title + order are present even when untouched this lifetime.
            for state in ["title", "order"] {
                if !out.iter().any(|(s, _, _)| s == state) {
                    let version = reg.get(state).map_or(0, |e| e.version);
                    out.push((state.to_owned(), version, Vec::new()));
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
        Some(Outbound::SyncPatch { state, version, value, confirmed, resync: true })
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

    /// Record the downstream agent's own session id for a session, persisting
    /// it so a future revive can resume via `session/load`. Updates in-memory
    /// metadata + write-behind store; no broadcast (the id isn't rendered, and
    /// the status/lifecycle events already drive the UI). Unknown ids are
    /// ignored (matches `set_status`).
    pub fn set_agent_session_id(&self, session_id: &str, agent_session_id: String) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.meta.agent_session_id = Some(agent_session_id.clone());
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::SetAgentSessionId {
                session_id: session_id.to_owned(),
                agent_session_id,
            });
        }
    }

    /// Forget a session's resumable agent id so the NEXT spawn starts a fresh
    /// `session/new` (a clean agent context) instead of `session/load`. The
    /// "clear conversation" reset (see [`Inbound::ResetSession`]) calls this
    /// before respawning. In-memory only: the freshly-spawned agent's own
    /// `session/new` re-persists a new id within a second or two, so there's no
    /// separate NULL write-behind (a daemon restart in that tiny window just
    /// leaves the old context — rare, and re-clearable).
    pub fn clear_agent_session_id(&self, session_id: &str) {
        let mut sessions = self.inner.sessions.lock();
        if let Some(s) = sessions.get_mut(session_id) {
            s.meta.agent_session_id = None;
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
        let turn_ended;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            // Clear the in-flight guard on a true turn-end (Busy → Running) or on
            // death — NOT on Starting → Running (a revive passes through that
            // edge while our dispatched prompt is still queued downstream, so
            // clearing there would release the next prompt early and overlap
            // turns). Mirrors the old client-side drain edge logic.
            let was = s.meta.status;
            if (was == Status::Busy && status == Status::Running)
                || matches!(status, Status::Exited | Status::Crashed | Status::Interrupted)
            {
                s.in_flight = false;
            }
            // A turn that died/was cut off is NOT "awaiting your reply" — the agent
            // didn't ask a question, it got interrupted. Clear the confirm-detect
            // hold so it doesn't block the auto-resume continuation (which inserts
            // at the queue FRONT and must drain to revive the turn). The judge only
            // runs on a clean Busy→Running end, so these death edges need the
            // explicit clear.
            if matches!(status, Status::Exited | Status::Crashed | Status::Interrupted) {
                s.meta.awaiting_user = false;
                s.meta.done = false;
            }
            // A true turn-end (Busy → Running) is where the agent handed control
            // back — the point to ask the confirm-detect skill "is it waiting on
            // me?". Captured here under the lock; the judge runs after we release.
            turn_ended = was == Status::Busy && status == Status::Running;
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
        // drain can release the next queued prompt. Recall-first: the judge sets a
        // provisional hold synchronously, so try_drain below is a no-op until the
        // async verdict either clears it (drains) or confirms it (stays held). With
        // no judge key configured, drain_head already blocks wholesale (§J), so we
        // skip the judge entirely.
        if turn_ended && self.confirm_key_present() {
            self.judge_turn_end(session_id);
        }
        // A turn-end / death may make the session drainable — try the next
        // queued prompt now (no-op if still busy, held, or nothing queued).
        self.try_drain(session_id);
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
        let envelope = {
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
            s.log.push(envelope.clone());
            envelope
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::AppendEvent(envelope.clone()));
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
        sessions
            .get(session_id)
            .and_then(|s| s.config_options.clone())
    }

    /// Store the latest agent-advertised config options for a session and
    /// fan them out to every client. Called from acp.rs when the upstream
    /// emits a `config_option_update` notification, and from the
    /// `SetConfigOption` reply path (the agent's authoritative response
    /// refreshes the same array).
    pub fn set_config_options(&self, session_id: &str, options: serde_json::Value) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.config_options = Some(options.clone());
        }
        let _ = self.inner.tx.send(Outbound::ConfigOptions {
            session_id: session_id.to_owned(),
            options,
        });
    }

    /// Surface a command failure to every connected client so the UI can show
    /// a toast. Replaces the previous behaviour of silently logging to
    /// `tracing::warn` — that left the user staring at an unchanged page
    /// wondering why nothing happened.
    pub fn broadcast_error(&self, session_id: Option<String>, message: String) {
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
        if let Some(tx) = self.inner.dispatch_tx.lock().as_ref() {
            let _ = tx.send(req);
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
        // The two LLM-gated holds below apply ONLY to the automatic drain. A manual
        // send (an explicit "send this now") is the user-triggered fallback that
        // must ALWAYS get through — even with no judge key or while the agent is
        // judged "awaiting" — so the queue can never be permanently trapped.
        if !manual {
            // No inference key → we can't judge whether the agent is asking the
            // user, so never AUTO-drain (§J). The user can still send manually.
            if !self.confirm_key_present() {
                return;
            }
        }
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
            if let Some(c) = cmid.as_deref() {
                if s.queue.iter().any(|m| m.cmid.as_deref() == Some(c)) {
                    return;
                }
            }
            let id = self.next_qid();
            s.queue.insert(0, QueuedMessage { id, text, content, cmid });
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
            if let Some(c) = cmid.as_deref() {
                if s.queue.iter().any(|m| m.cmid.as_deref() == Some(c)) {
                    return;
                }
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
            if let Some(c) = cmid.as_deref() {
                if s.queue.iter().any(|m| m.cmid.as_deref() == Some(c)) {
                    return false;
                }
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
                dispatch = Some(DispatchReq { session_id: session_id.to_owned(), text, content, cmid });
            } else {
                // Busy / draining / queued ahead → jump to the FRONT so it runs
                // next; ask the caller to interrupt the in-flight turn only when
                // this is a force push (not a no-interrupt "jump to front").
                let id = self.next_qid();
                s.queue.insert(0, QueuedMessage { id, text, content, cmid });
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
        // Explicit user "send now" → manual drain, bypassing the no-key / awaiting
        // holds (the always-available fallback when the judge can't run).
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
        let (prompt, status) = {
            let sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get(session_id) else {
                tracing::warn!(session = %session_id, "retry_turn: unknown session — no-op");
                return;
            };
            (last_turn_texts(&s.log).0, s.meta.status)
        };
        if prompt.trim().is_empty() {
            // The "no response" report points here first: a crashed turn whose
            // user prompt never made it into the log leaves nothing to re-run.
            tracing::warn!(session = %session_id, ?status, "retry_turn: no prior prompt to retry — no-op");
            return;
        }
        tracing::info!(session = %session_id, ?status, prompt_len = prompt.len(), "retry_turn: re-submitting last prompt");
        let _ = self.force_submit(session_id, prompt, Vec::new(), None, true);
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
            if let Some(c) = cmid.as_deref() {
                if s.drafts.iter().any(|m| m.cmid.as_deref() == Some(c)) {
                    return;
                }
            }
            let id = self.next_qid();
            s.drafts.push(QueuedMessage {
                id,
                text,
                content,
                cmid,
            });
        }
        self.emit_pending(session_id);
    }

    /// Edit a draft in place. Empty text + content removes it.
    pub fn edit_draft(
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
                s.drafts.retain(|m| m.id != id);
            } else if let Some(m) = s.drafts.iter_mut().find(|m| m.id == id) {
                m.text = text;
                m.content = content;
            }
        }
        self.emit_pending(session_id);
    }

    /// Drop one draft.
    pub fn remove_draft(&self, session_id: &str, id: &str) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.drafts.retain(|m| m.id != id);
        }
        self.emit_pending(session_id);
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
        {
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
            // `to` existed at the top of this lock and we still hold it, so this
            // can't miss; the `if let` just avoids an unwrap.
            if let Some(dst) = sessions.get_mut(to) {
                dst.drafts.push(msg);
            }
        }
        self.emit_pending(from);
        self.emit_pending(to);
    }

    /// Drop a session's whole draft list.
    pub fn clear_drafts(&self, session_id: &str) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.drafts.clear();
        }
        self.emit_pending(session_id);
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
        for m in msgs {
            // Activating a draft is a server-side move, not a fresh client
            // optimistic send — no cmid.
            self.submit(session_id, m.text, m.content, None);
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
            list.sort_by_key(|id| {
                order
                    .iter()
                    .position(|o| o == id)
                    .unwrap_or(usize::MAX)
            });
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
mod confirm_hold_tests {
    use super::*;

    fn hub_with_session(id: &str) -> Hub {
        let hub = Hub::new();
        hub.create_session(
            id.to_owned(),
            "claude-code".to_owned(),
            "/tmp".to_owned(),
            "t".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub
    }

    // The two inputs `drain_head` reads before draining: a present judge key
    // (§J no-token block) and the per-session awaiting hold (§I). The drain is a
    // no-op without a registered dispatcher, so we assert the guard *state* the
    // drain branches on rather than the dispatch side effect (live-verified).
    #[test]
    fn no_key_blocks_then_key_unblocks() {
        let hub = hub_with_session("s1");
        assert!(!hub.confirm_key_present(), "no key → drain blocks");
        hub.set_inference_secret("deepseek".to_owned(), "sk-test".to_owned());
        assert!(hub.confirm_key_present(), "key present → drain may proceed");
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
        hub.requeue_prompt("r1", "hello agent".to_owned(), vec![], Some("c1".to_owned()));
        assert_eq!(queue_texts(&hub, "r1"), vec!["hello agent".to_owned()]);
        // Same cmid (the delivery raced a re-revive) → not double-queued.
        hub.requeue_prompt("r1", "hello agent".to_owned(), vec![], Some("c1".to_owned()));
        assert_eq!(queue_texts(&hub, "r1").len(), 1, "same cmid must not double-queue");
        // A different message DOES stack (front-inserted).
        hub.requeue_prompt("r1", "second".to_owned(), vec![], Some("c2".to_owned()));
        assert_eq!(queue_texts(&hub, "r1"), vec!["second".to_owned(), "hello agent".to_owned()]);
    }
}

