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

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

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
}

/// What the server pushes to a WebSocket client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outbound {
    /// Full session list (sent on connect and whenever it changes).
    Sessions { sessions: Vec<SessionMeta> },
    /// Replay of one session's whole log (sent on connect, after `Sessions`).
    Snapshot {
        session_id: String,
        events: Vec<Envelope>,
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
            }),
        }
    }

    /// Populate the in-memory state from a previously-stored snapshot.
    /// Should be called once at startup, BEFORE any client connects, so the
    /// `Sessions` broadcast on first connect already includes everything.
    /// Skips the write-behind side: these rows are already in the DB.
    ///
    /// **Restored sessions are forced to [`Status::Exited`].** The agent
    /// subprocess does not come back across a daemon restart — the postgres
    /// state is metadata + history only, not a live ACP connection. Letting
    /// a restored row keep its persisted `Running`/`Busy` status creates the
    /// trap of a UI that looks alive but rejects every prompt with `unknown
    /// session` (no `agent_tx` in the supervisor). Mark them dead so the UI
    /// shows them as ended and disables the composer; resume via
    /// session/load is a future follow-up (design §7).
    pub fn restore(&self, sessions: Vec<(SessionMeta, Vec<Envelope>, u64)>) {
        let mut sessions_lock = self.inner.sessions.lock().unwrap();
        let mut order = self.inner.order.lock().unwrap();
        for (mut meta, log, next_seq) in sessions {
            meta.status = Status::Exited;
            let id = meta.id.clone();
            sessions_lock.insert(
                id.clone(),
                Session {
                    meta,
                    log,
                    next_seq,
                    config_options: None,
                },
            );
            order.push(id);
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

    /// Full event log for one session (for a fresh client's snapshot).
    #[must_use]
    pub fn snapshot(&self, session_id: &str) -> Option<Vec<Envelope>> {
        let sessions = self.inner.sessions.lock().unwrap();
        sessions.get(session_id).map(|s| s.log.clone())
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
    }

    /// Append an event to a session's log under the next `seq` and fan it out.
    /// Unknown sessions are ignored (a race with teardown).
    pub fn push(&self, session_id: &str, event: Event) {
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
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}
