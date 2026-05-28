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
use crate::core::Hub;
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
    /// (resolved under the workspace root). Returns the cowboy session id.
    ///
    /// # Errors
    /// If the provider is unknown or the agent thread cannot be spawned.
    pub fn new_session(&self, provider: &str, cwd: Option<String>) -> Result<String, String> {
        let spec =
            provider::lookup(provider).ok_or_else(|| format!("unknown provider {provider:?}"))?;

        // Resolve cwd within the workspace root. A relative request joins the
        // root; an absolute request that escapes the root falls back to it
        // (v1 scoping — design §9 workspace-root scoping).
        let cwd = match cwd {
            Some(rel) => {
                let p = PathBuf::from(&rel);
                let joined = if p.is_absolute() {
                    p
                } else {
                    self.workspace_root.join(p)
                };
                if joined.starts_with(&self.workspace_root) {
                    joined
                } else {
                    self.workspace_root.clone()
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
}
