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
use std::sync::Mutex;

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
/// `Zed` = the `acp-bridge` translated an ACP `session/new` from Zed.
/// `Api` = a direct `POST /api/sessions` with no `origin` field (curl, tests,
/// future scripted callers).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionOrigin {
    #[default]
    Api,
    Web,
    Zed,
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
}

/// A command sent by a client (Web UI, `acp-bridge`, future test harnesses)
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
    /// - **Bridge** sends `content: [...ACP ContentBlock JSON]` to carry rich
    ///   content (e.g. pasted images). When both are present, `content`
    ///   wins. At least one must be non-empty; otherwise the prompt is
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

    // --- Server-authoritative queue + drafts (synced across all terminals) ----
    //
    // The Web UI sends these instead of dispatching prompts itself: the daemon
    // owns the per-session queue/drafts and the drain (next-on-turn-end), so
    // every connected terminal sees identical state and only one turn ever runs.
    /// Send a user turn the queue-aware way: dispatch immediately if the session
    /// is idle and nothing is queued/in-flight, otherwise append to the queue.
    /// (The bridge/API keep using `Prompt` for a direct, un-queued dispatch.)
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
    /// Per-state arbiters for the generic optimistic-sync channel, keyed by
    /// state name (`"title"`, `"order"`, …). See [`SyncArbiter`].
    sync: Mutex<HashMap<String, SyncArbiter>>,
    /// Monotonic source of queued/draft message ids (`q1`, `q2`, …). Seeded from
    /// the wall-clock-free counter; uniqueness across a daemon lifetime is all
    /// that's required (ids are list-local keys, not persisted-across-restart
    /// identities — restored lists keep whatever ids they were saved with).
    next_qid: AtomicU64,
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
                sync: Mutex::new(HashMap::new()),
                next_qid: AtomicU64::new(1),
            }),
        }
    }

    /// Wire the background dispatcher's hand-off channel. Called once at startup
    /// (in `crate::server`) after the dispatcher task is spawned, before any
    /// client connects. Until set, drains compute but dispatch nothing.
    pub fn set_dispatch_tx(&self, tx: mpsc::UnboundedSender<DispatchReq>) {
        *self.inner.dispatch_tx.lock().unwrap() = Some(tx);
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
            let mut sessions_lock = self.inner.sessions.lock().unwrap();
            let mut order = self.inner.order.lock().unwrap();
            for r in sessions {
                let RestoredSession {
                    mut meta,
                    log,
                    next_seq,
                    mut queue,
                    mut drafts,
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
        let sessions = self.inner.sessions.lock().unwrap();
        let order = self.inner.order.lock().unwrap();
        order
            .iter()
            .filter_map(|id| sessions.get(id).map(|s| s.meta.clone()))
            .collect()
    }

    /// Per-session info (metadata + live event/queue/draft counts) for the
    /// session-info dialog. `None` for an unknown session.
    #[must_use]
    pub fn session_info(&self, session_id: &str) -> Option<SessionInfo> {
        let sessions = self.inner.sessions.lock().unwrap();
        let s = sessions.get(session_id)?;
        Some(SessionInfo {
            meta: s.meta.clone(),
            event_count: u64::try_from(s.log.len()).unwrap_or(u64::MAX),
            queue_count: s.queue.len(),
            drafts_count: s.drafts.len(),
        })
    }

    /// Total events held in memory across all live sessions — the event-count
    /// metric for the info panel.
    #[must_use]
    pub fn event_total(&self) -> u64 {
        let sessions = self.inner.sessions.lock().unwrap();
        sessions.values().map(|s| u64::try_from(s.log.len()).unwrap_or(u64::MAX)).sum()
    }

    /// Recent log TAIL for a fresh client (last [`SNAPSHOT_TAIL`] events) plus
    /// `reached_start` = whether the tail IS the whole log. Older pages are
    /// fetched on demand over HTTP (`history_page`), not shipped here — a long
    /// session must not re-send its entire history on every connect/reconnect.
    #[must_use]
    pub fn snapshot(&self, session_id: &str) -> Option<(Vec<Envelope>, bool)> {
        let sessions = self.inner.sessions.lock().unwrap();
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
        let sessions = self.inner.sessions.lock().unwrap();
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
    ) {
        let meta = SessionMeta {
            id: id.clone(),
            provider,
            cwd,
            title,
            status: Status::Starting,
            origin,
            agent_session_id: None,
        };
        {
            let mut sessions = self.inner.sessions.lock().unwrap();
            let mut order = self.inner.order.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
            let mut order = self.inner.order.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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

    // --- Generic optimistic-sync channel (@shared-utils/sync arbiter) --------
    // The daemon is the arbiter for each synced `state`. A mutation is applied to
    // the TYPED source of truth (SessionMeta / order list); the patch carries the
    // state's DERIVED json value. No bespoke per-state wire — one Sync/SyncPatch.

    /// Apply a rename to the typed truth (NO `Sessions` re-broadcast — the sync
    /// channel carries the title now). Persists so fresh-connect + restart show
    /// it. Mirror of [`Self::rename_session`] minus the broadcast.
    fn apply_rename(&self, session_id: &str, title: String) {
        {
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut list = self.inner.order.lock().unwrap();
            list.sort_by_key(|id| order.iter().position(|o| o == id).unwrap_or(usize::MAX));
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let order = self.inner.order.lock().unwrap().clone();
            let _ = tx.send(StoreWrite::UpdateSessionOrder { order });
        }
    }

    /// The derived JSON value of one synced state — what a `SyncPatch` carries and
    /// the client folds. Always read live from the typed truth (so it's durable by
    /// derivation, no shadow copy to drift).
    fn sync_value(&self, state: &str) -> serde_json::Value {
        match state {
            "title" => {
                let sessions = self.inner.sessions.lock().unwrap();
                let map: serde_json::Map<String, serde_json::Value> = sessions
                    .values()
                    .map(|s| (s.meta.id.clone(), serde_json::Value::String(s.meta.title.clone())))
                    .collect();
                serde_json::Value::Object(map)
            }
            "order" => {
                let list = self.inner.order.lock().unwrap();
                serde_json::Value::Array(list.iter().map(|id| serde_json::Value::String(id.clone())).collect())
            }
            _ => serde_json::Value::Null,
        }
    }

    /// Record `id` as seen for `state`; returns true if it's NEW (first delivery).
    fn sync_first_seen(&self, state: &str, id: &str) -> bool {
        let mut reg = self.inner.sync.lock().unwrap();
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
            let mut reg = self.inner.sync.lock().unwrap();
            let e = reg.entry(state.to_owned()).or_default();
            e.version += 1;
            e.version
        };
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
        self.sync_broadcast(state, vec![id]);
        Ok(())
    }

    /// Resync `SyncPatch`es for the GLOBAL states (title / order) mutated this
    /// lifetime — at their version, confirming all seen ids, `resync: true` so the
    /// client adopts them across a restart. Per-session queue states resync
    /// separately via [`Self::queue_resync`]. States never mutated aren't here;
    /// the client uses the `Sessions`-derived default.
    #[must_use]
    pub fn sync_resync(&self) -> Vec<Outbound> {
        let snapshot: Vec<(String, u64, Vec<String>)> = {
            let reg = self.inner.sync.lock().unwrap();
            reg.iter()
                .filter(|(s, _)| !s.starts_with("queue:"))
                .map(|(s, e)| (s.clone(), e.version, e.seen.iter().cloned().collect()))
                .collect()
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
            let sessions = self.inner.sessions.lock().unwrap();
            let s = sessions.get(session_id)?;
            (s.queue.clone(), s.drafts.clone())
        };
        let state = format!("queue:{session_id}");
        let version = self.inner.sync.lock().unwrap().get(&state).map_or(0, |e| e.version);
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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

    /// Update a session's status, emit a `Lifecycle` event, refresh the list.
    pub fn set_status(&self, session_id: &str, status: Status, detail: Option<String>) {
        {
            let mut sessions = self.inner.sessions.lock().unwrap();
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
        // queued prompt now (no-op if still busy or nothing queued).
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
        let sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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

    // --- Queue + drafts (server-authoritative, synced to every terminal) ------

    /// Current status of a session, if it exists. Lets the server decide
    /// busy-vs-idle for the force-push path without reaching into `Session`.
    #[must_use]
    pub fn status(&self, session_id: &str) -> Option<Status> {
        self.inner
            .sessions
            .lock()
            .unwrap()
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
            let sessions = self.inner.sessions.lock().unwrap();
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
        if let Some(tx) = self.inner.dispatch_tx.lock().unwrap().as_ref() {
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
    fn drain_head(&self, session_id: &str, allow_revive: bool) {
        // Without a dispatcher wired we must not pop (the prompt would be lost).
        if self.inner.dispatch_tx.lock().unwrap().is_none() {
            return;
        }
        let req = {
            let mut sessions = self.inner.sessions.lock().unwrap();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if !Self::ready(s, allow_revive) {
                return;
            }
            let Some(head) = s.queue.first() else {
                return;
            };
            if s.editing.as_deref() == Some(head.id.as_str()) {
                return; // head held for edit → whole queue pauses
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
        self.drain_head(session_id, false);
    }

    /// Clear the in-flight guard (used by the dispatcher when a send fails) and
    /// try the next queued prompt.
    pub fn clear_in_flight(&self, session_id: &str) {
        {
            let mut sessions = self.inner.sessions.lock().unwrap();
            if let Some(s) = sessions.get_mut(session_id) {
                s.in_flight = false;
            }
        }
        self.try_drain(session_id);
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
        let wired = self.inner.dispatch_tx.lock().unwrap().is_some();
        let mut dispatch = None;
        {
            let mut sessions = self.inner.sessions.lock().unwrap();
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
        match dispatch {
            // Dispatched straight through — never touched a list, so no flicker
            // of the prompt appearing-then-leaving the queue.
            Some(req) => self.send_dispatch(req),
            None => self.emit_pending(session_id),
        }
    }

    /// Drop one queued prompt.
    pub fn remove_queued(&self, session_id: &str, id: &str) {
        {
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
        self.drain_head(session_id, true);
    }

    /// Move a queued prompt back to drafts.
    pub fn queued_to_draft(&self, session_id: &str, id: &str) {
        {
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut sessions = self.inner.sessions.lock().unwrap();
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
            let mut list = self.inner.order.lock().unwrap();
            list.sort_by_key(|id| {
                order
                    .iter()
                    .position(|o| o == id)
                    .unwrap_or(usize::MAX)
            });
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateSessionOrder {
                order: self.inner.order.lock().unwrap().clone(),
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
