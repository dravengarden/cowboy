//! Session supervisor (design §7).
//!
//! Owns each agent's lifetime, decoupled from any client connection. Because
//! the `agent-client-protocol` connection is `!Send` (single-threaded,
//! spawn-local), each session runs on its **own OS thread** with a
//! current-thread tokio runtime + `LocalSet` (see [`crate::acp::run_agent`]).
//! The supervisor talks to that thread only through a `Send` command channel.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use tokio::sync::mpsc;

use crate::acp::{self, AgentCommand};
use crate::core::{Hub, SessionOrigin};
use crate::provider;

/// Spawns and tracks agent sessions; routes commands to their threads.
pub struct Supervisor {
    hub: Hub,
    workspace_root: PathBuf,
    senders: Mutex<HashMap<String, mpsc::UnboundedSender<AgentCommand>>>,
    counter: AtomicU64,
}

impl Supervisor {
    #[must_use]
    pub fn new(hub: Hub, workspace_root: PathBuf) -> Self {
        Self {
            hub,
            workspace_root,
            senders: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(1),
        }
    }

    /// Create a new session for `provider`, optionally rooted at `cwd`
    /// (resolved under the workspace root), tagged with the surface
    /// (`origin`) that opened it. Returns the cowboy session id.
    ///
    /// # Errors
    /// If the provider is unknown or the agent thread cannot be spawned.
    pub fn new_session(
        &self,
        provider: &str,
        cwd: Option<String>,
        origin: SessionOrigin,
    ) -> Result<String, String> {
        let spec =
            provider::lookup(provider).ok_or_else(|| format!("unknown provider {provider:?}"))?;

        // Resolve cwd. Relative paths join the workspace_root; absolute
        // paths are honoured as-is. We dropped the starts_with(root) clamp
        // (v1) because cowboy is LAN-only + runs as the human user, and the
        // user explicitly wants to open sessions in workspaces outside the
        // default root (e.g. `/etc/nixos`). The agent already inherits the
        // user's full filesystem permissions, so the clamp was security
        // theatre rather than a real boundary.
        let cwd = match cwd {
            Some(rel) => {
                let p = PathBuf::from(&rel);
                if p.is_absolute() {
                    p
                } else {
                    self.workspace_root.join(p)
                }
            }
            None => self.workspace_root.clone(),
        };

        let id = format!("sess-{}", self.counter.fetch_add(1, Ordering::Relaxed));
        let title = format!("{provider} · {}", cwd.display());
        self.hub.create_session(
            id.clone(),
            provider.to_owned(),
            cwd.display().to_string(),
            title,
            origin,
        );

        let (tx, rx) = mpsc::unbounded_channel();
        self.senders.lock().unwrap().insert(id.clone(), tx);

        let hub = self.hub.clone();
        let thread_id = id.clone();
        std::thread::Builder::new()
            .name(format!("agent-{id}"))
            .spawn(move || acp::run_agent(&spec, &thread_id, cwd, rx, &hub))
            .map_err(|e| format!("spawning agent thread: {e}"))?;

        Ok(id)
    }

    /// Forward a command to a session's agent thread.
    ///
    /// # Errors
    /// If the session is unknown or its thread has already ended.
    pub fn send(&self, session_id: &str, cmd: AgentCommand) -> Result<(), String> {
        let senders = self.senders.lock().unwrap();
        let tx = senders
            .get(session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        tx.send(cmd).map_err(|_| "session ended".to_owned())
    }

    /// Tear down a session's agent thread. Sends `Cancel` (best-effort, so an
    /// in-flight turn returns to its caller cleanly), then drops the tx so
    /// the agent's command loop terminates on next poll. Hub state is the
    /// caller's responsibility — pair with [`Hub::delete_session`].
    ///
    /// Returns `true` if the session had a live sender (= the thread was
    /// alive). Unknown / already-torn-down sessions are a no-op and return
    /// `false`.
    pub fn delete_session(&self, session_id: &str) -> bool {
        let tx = self.senders.lock().unwrap().remove(session_id);
        match tx {
            Some(tx) => {
                let _ = tx.send(AgentCommand::Cancel);
                drop(tx);
                true
            }
            None => false,
        }
    }
}
