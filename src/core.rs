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

use serde::Serialize;
use tokio::sync::broadcast;

/// Provider/session status as shown in the session list.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
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
#[derive(Debug, Clone, Serialize)]
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
#[derive(Debug, Clone, Serialize)]
pub struct Envelope {
    pub session_id: String,
    pub seq: u64,
    #[serde(flatten)]
    pub event: Event,
}

/// Session metadata for the list view (no event log).
#[derive(Debug, Clone, Serialize)]
pub struct SessionMeta {
    pub id: String,
    pub provider: String,
    pub cwd: String,
    pub title: String,
    pub status: Status,
}

/// Per-session state: metadata + the seq-ordered event log.
struct Session {
    meta: SessionMeta,
    log: Vec<Envelope>,
    next_seq: u64,
}

/// What the server pushes to a WebSocket client.
#[derive(Debug, Clone, Serialize)]
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
    /// An error to surface to the user (bad command, unknown session, ...).
    /// Part of the wire protocol; not emitted in v1 (command errors are
    /// currently logged server-side only).
    #[allow(dead_code)]
    Error { message: String },
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
}

impl Hub {
    #[must_use]
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            inner: std::sync::Arc::new(HubInner {
                sessions: Mutex::new(HashMap::new()),
                order: Mutex::new(Vec::new()),
                tx,
            }),
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
    pub fn create_session(&self, id: String, provider: String, cwd: String, title: String) {
        {
            let mut sessions = self.inner.sessions.lock().unwrap();
            let mut order = self.inner.order.lock().unwrap();
            sessions.insert(
                id.clone(),
                Session {
                    meta: SessionMeta {
                        id: id.clone(),
                        provider,
                        cwd,
                        title,
                        status: Status::Starting,
                    },
                    log: Vec::new(),
                    next_seq: 0,
                },
            );
            order.push(id);
        }
        self.broadcast_sessions();
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
        // A send error just means no clients are connected — fine.
        let _ = self.inner.tx.send(Outbound::Event { envelope });
    }

    fn broadcast_sessions(&self) {
        let _ = self.inner.tx.send(Outbound::Sessions {
            sessions: self.session_list(),
        });
    }
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}
