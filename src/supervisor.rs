//! Session supervisor (design §7).
//!
//! Owns each agent's lifetime, decoupled from any client connection. Because
//! the `agent-client-protocol` connection is `!Send` (single-threaded,
//! spawn-local), each session runs on its **own OS thread** with a
//! current-thread tokio runtime + `LocalSet` (see [`crate::acp::run_agent`]).
//! The supervisor talks to that thread only through a `Send` command channel.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;

use crate::acp::{self, AgentCommand};
use crate::core::{Hub, SessionOrigin, Status};
use crate::provider::{self, LaunchSpec};

/// Spawns and tracks agent sessions; routes commands to their threads.
pub struct Supervisor {
    hub: Hub,
    workspace_root: PathBuf,
    senders: Mutex<HashMap<String, mpsc::UnboundedSender<AgentCommand>>>,
    counter: AtomicU64,
}

fn initial_counter(hub: &Hub, persistent_floor: u64, clock_floor: u64) -> u64 {
    let live_floor = hub
        .session_list()
        .iter()
        .filter_map(|meta| {
            meta.id
                .strip_prefix("sess-")
                .and_then(|suffix| suffix.parse::<u64>().ok())
        })
        .max()
        .map_or(1, |value| value.saturating_add(1));
    live_floor.max(persistent_floor).max(clock_floor)
}

impl Supervisor {
    #[must_use]
    pub fn new(hub: Hub, workspace_root: PathBuf, persistent_floor: u64) -> Self {
        // The wall-clock floor prevents reuse even after old tombstones are
        // purged. `persistent_floor` covers clock rollback and, critically,
        // includes soft-deleted rows that Hub::restore does not load.
        let clock_floor = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| u64::try_from(duration.as_millis()).ok())
            .unwrap_or(1);
        let initial = initial_counter(&hub, persistent_floor, clock_floor);
        Self {
            hub,
            workspace_root,
            senders: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(initial),
        }
    }

    /// The workspace root sessions resolve against (default `/home/draven`).
    /// Exposed so the server can enumerate selectable workspaces beneath it
    /// (the columbus project registry lives at `<root>/columbus/project-defs`).
    #[must_use]
    pub fn workspace_root(&self) -> &std::path::Path {
        &self.workspace_root
    }

    #[must_use]
    pub fn resource_stats(&self) -> Vec<(String, crate::cgroup::Stats)> {
        self.senders
            .lock()
            .keys()
            .filter_map(|id| crate::cgroup::stats(id).map(|stats| (id.clone(), stats)))
            .collect()
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
        system: bool,
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
            system,
        );

        // Fresh session — no agent id to resume.
        self.spawn_agent(&id, &spec, cwd, None)?;
        Ok(id)
    }

    /// Spawn an agent thread for `session_id` and register its command sender.
    /// The single place that starts an [`acp::run_agent`] OS thread — both the
    /// fresh [`Self::new_session`] path and the [`Self::revive`] path go
    /// through here, differing only in `resume` (the agent's prior id to
    /// re-attach via `session/load`, or `None` for a blank session).
    ///
    /// Idempotent: if a live sender already exists (a concurrent caller won the
    /// race), this is a no-op. The Hub session must already exist.
    ///
    /// # Errors
    /// If the OS thread cannot be spawned.
    fn spawn_agent(
        &self,
        session_id: &str,
        spec: &LaunchSpec,
        cwd: PathBuf,
        resume: Option<String>,
    ) -> Result<(), String> {
        let mut senders = self.senders.lock();
        if senders.contains_key(session_id) {
            return Ok(()); // already live
        }
        // One in-flight turn per session is enforced by Hub, so this channel is
        // logically bounded even though Tokio's transport is unbounded.
        let (tx, rx) = mpsc::unbounded_channel();
        let hub = self.hub.clone();
        let spec = spec.clone();
        let thread_id = session_id.to_owned();
        std::thread::Builder::new()
            .name(format!("agent-{session_id}"))
            .spawn(move || acp::run_agent(&spec, &thread_id, cwd, resume, rx, &hub))
            .map_err(|e| format!("spawning agent thread: {e}"))?;
        senders.insert(session_id.to_owned(), tx);
        Ok(())
    }

    /// Forward a command to a session's agent thread.
    ///
    /// Fast path: a live agent thread owns the session → deliver directly.
    ///
    /// Slow path — **post-restart resume**: after a daemon restart,
    /// [`Hub::restore`] brings session metadata + event history back from
    /// postgres, but `senders` starts empty, so a restored session has no
    /// agent process. Without reviving it here the first command returned
    /// `unknown session`, which the WS layer turns into a fire-and-forget
    /// [`Outbound::Error`], so the prompt never reached an agent (the Web UI
    /// stayed empty). We lazily spawn a fresh
    /// agent for the restored session and deliver the command to it. The new
    /// agent starts without the prior turn's in-agent context — full
    /// `session/load` replay is the deferred design §7 follow-up — but the
    /// session continues and every surface stays in sync.
    ///
    /// [`Outbound::Error`]: crate::core::Outbound::Error
    ///
    /// # Errors
    /// If the session is unknown to the Hub, its provider is no longer
    /// registered, or the agent thread cannot be spawned.
    pub fn send(&self, session_id: &str, cmd: AgentCommand) -> Result<(), String> {
        // A failed `send` means the receiver (agent thread) is gone: drop the
        // stale sender and fall through to revive, recovering the command from
        // the `SendError` so we needn't require `AgentCommand: Clone`.
        let cmd = {
            let mut senders = self.senders.lock();
            match senders.get(session_id) {
                Some(tx) => match tx.send(cmd) {
                    Ok(()) => return Ok(()),
                    Err(error) => {
                        senders.remove(session_id);
                        error.0
                    }
                },
                None => cmd,
            }
        };
        self.revive(session_id)?;
        self.senders
            .lock()
            .get(session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?
            .send(cmd)
            .map_err(|_| "session ended".to_owned())
    }

    /// Ensure a session has a live agent **without sending a turn** — the
    /// "revive on open" path (design §7). Called when a client selects/opens a
    /// session: revives one whose agent died with a daemon restart (handing it
    /// the prior `agent_session_id` for `session/load` resume), so it's already
    /// warming up before the user types. Idempotent and cheap: a no-op when a
    /// sender is already registered (the steady-state case — agents outlive
    /// client connections), so it's safe to call on every open / reconnect.
    ///
    /// A stale sender left by a crashed-but-not-deleted agent is NOT detected
    /// here (we don't probe by sending); that rarer case is still recovered by
    /// the next [`Self::send`], which drops the dead sender and revives.
    ///
    /// Returns `true` if it revived, `false` if the agent was already alive.
    ///
    /// # Errors
    /// If the session is unknown to the Hub, its provider is no longer
    /// registered, or the agent thread cannot be spawned.
    pub fn ensure_alive(&self, session_id: &str) -> Result<bool, String> {
        if self.senders.lock().contains_key(session_id) {
            return Ok(false);
        }
        self.revive(session_id)?;
        Ok(true)
    }

    /// Spawn an agent thread for a session that exists in the Hub but has no
    /// live sender (one restored after a restart, or whose agent crashed).
    /// Reuses the persisted provider + cwd, and hands the agent its prior
    /// `agent_session_id` so it resumes the conversation via `session/load`
    /// (design §7) — the prior context is restored, not dropped, when the
    /// provider supports it. Idempotent: a no-op if a concurrent caller
    /// revived it first.
    ///
    /// # Errors
    /// If the session id is unknown to the Hub, the provider is no longer
    /// registered, or the OS thread cannot be spawned.
    fn revive(&self, session_id: &str) -> Result<(), String> {
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|m| m.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        let spec = provider::lookup(&meta.provider)
            .ok_or_else(|| format!("unknown provider {:?}", meta.provider))?;
        let cwd = PathBuf::from(&meta.cwd);

        // Reflect the reconnect immediately so the UI shows "starting" rather
        // than a stale "exited" while the agent re-handshakes (a few seconds).
        self.hub.set_status(session_id, Status::Starting, None);
        self.spawn_agent(session_id, &spec, cwd, meta.agent_session_id.clone())?;
        tracing::info!(
            session = session_id,
            resume = ?meta.agent_session_id,
            "revived session (session/load when the agent's prior id is known)"
        );
        Ok(())
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
        let tx = self.senders.lock().remove(session_id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_counter_honors_live_persistent_and_clock_floors() {
        let hub = Hub::new();
        hub.create_session(
            "sess-99".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            SessionOrigin::Api,
            false,
        );

        assert_eq!(initial_counter(&hub, 50, 25), 100);
        assert_eq!(initial_counter(&hub, 196, 25), 196);
        assert_eq!(initial_counter(&hub, 196, 10_000), 10_000);
    }
}
