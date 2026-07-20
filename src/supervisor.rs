//! Session supervisor (design §7).
//!
//! Owns each agent's lifetime, decoupled from any client connection. Because
//! the `agent-client-protocol` connection is `!Send` (single-threaded,
//! spawn-local), each session runs on its **own OS thread** with a
//! current-thread tokio runtime + `LocalSet` (see [`crate::acp::run_agent`]).
//! The supervisor talks to that thread only through a `Send` command channel.

#![warn(clippy::pedantic)]

use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;

use crate::acp::{self, AgentCommand};
use crate::core::{Hub, SessionOrigin, Status};
use crate::provider::{self, LaunchSpec};
use crate::remote_runtime::RemoteRuntime;
use crate::runtime_wire::StartSession;
use crate::workspace::{
    current_project_checkout, resolve_session_workspace, session_belongs_to_project,
};

enum Backend {
    Local(Mutex<HashMap<String, mpsc::UnboundedSender<AgentCommand>>>),
    Remote(Arc<RemoteRuntime>),
}

/// Spawns and tracks agent sessions; routes commands to their threads.
pub struct Supervisor {
    hub: Hub,
    workspace_root: PathBuf,
    backend: Backend,
    counter: AtomicU64,
    lifecycle: Mutex<()>,
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
            backend: Backend::Local(Mutex::new(HashMap::new())),
            counter: AtomicU64::new(initial),
            lifecycle: Mutex::new(()),
        }
    }

    #[must_use]
    pub fn new_remote(
        hub: Hub,
        workspace_root: PathBuf,
        persistent_floor: u64,
        runtime: Arc<RemoteRuntime>,
    ) -> Self {
        let clock_floor = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| u64::try_from(duration.as_millis()).ok())
            .unwrap_or(1);
        let initial = initial_counter(&hub, persistent_floor, clock_floor);
        Self {
            hub,
            workspace_root,
            backend: Backend::Remote(runtime),
            counter: AtomicU64::new(initial),
            lifecycle: Mutex::new(()),
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
        match &self.backend {
            Backend::Local(senders) => senders
                .lock()
                .keys()
                .filter_map(|id| crate::cgroup::stats(id).map(|stats| (id.clone(), stats)))
                .collect(),
            // Detached worker cgroups are sibling user units. Agentd exports
            // their resource telemetry separately; the legacy in-process path
            // remains available for dev/tests.
            Backend::Remote(_) => Vec::new(),
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
        let Backend::Local(senders) = &self.backend else {
            let Backend::Remote(runtime) = &self.backend else {
                unreachable!();
            };
            runtime.ensure(StartSession {
                session_id: session_id.to_owned(),
                provider: spec.id.to_owned(),
                cwd: cwd.display().to_string(),
                agent_session_id: resume,
                system: self.hub.session_is_system(session_id),
                generation: String::new(),
                fallback_for: None,
                adopt_only: false,
            });
            return Ok(());
        };
        let mut senders = senders.lock();
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
    pub fn send(&self, session_id: &str, command: AgentCommand) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        self.prepare_session_inner(session_id)?;
        if let Backend::Remote(runtime) = &self.backend {
            if !self
                .hub
                .session_list()
                .iter()
                .any(|meta| meta.id == session_id)
            {
                return Err(format!("unknown session {session_id:?}"));
            }
            match command {
                AgentCommand::Prompt(blocks, cmid, completion) => {
                    if let Some(completion) = completion {
                        let _ = completion.send(Err(
                            "direct completion capture is unavailable through detached runtime"
                                .to_owned(),
                        ));
                        return Err(
                            "detached runtime does not support completion capture".to_owned()
                        );
                    }
                    let content = blocks
                        .into_iter()
                        .map(|block| serde_json::to_value(block).unwrap_or(serde_json::Value::Null))
                        .collect();
                    runtime.ensure(self.start_session(session_id)?);
                    runtime.prompt(session_id, content, cmid);
                }
                AgentCommand::Cancel => runtime.cancel(session_id),
                AgentCommand::Permission {
                    request_id,
                    option_id,
                } => runtime.permission(session_id, request_id, option_id),
                AgentCommand::SetConfigOption { config_id, value } => {
                    runtime.set_config_option(session_id, config_id, value);
                }
            }
            return Ok(());
        }
        let Backend::Local(senders) = &self.backend else {
            unreachable!();
        };
        // A failed `send` means the receiver (agent thread) is gone: drop the
        // stale sender and fall through to revive, recovering the command from
        // the `SendError` so we needn't require `AgentCommand: Clone`.
        let command = {
            let mut senders = senders.lock();
            match senders.get(session_id) {
                Some(tx) => match tx.send(command) {
                    Ok(()) => return Ok(()),
                    Err(error) => {
                        senders.remove(session_id);
                        error.0
                    }
                },
                None => command,
            }
        };
        self.revive(session_id)?;
        let Backend::Local(senders) = &self.backend else {
            unreachable!();
        };
        senders
            .lock()
            .get(session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?
            .send(command)
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
        let _lifecycle = self.lifecycle.lock();
        if self.prepare_session_inner(session_id)? {
            return Ok(true);
        }
        match &self.backend {
            Backend::Remote(runtime) => {
                if runtime.has_worker(session_id) {
                    return Ok(false);
                }
                runtime.ensure(self.start_session(session_id)?);
                return Ok(true);
            }
            Backend::Local(senders) if senders.lock().contains_key(session_id) => return Ok(false),
            Backend::Local(_) => {}
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
        self.resolve_and_persist_cwd(session_id)?;
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

    fn start_session(&self, session_id: &str) -> Result<StartSession, String> {
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        Ok(StartSession {
            session_id: meta.id,
            provider: meta.provider,
            cwd: meta.cwd,
            agent_session_id: meta.agent_session_id,
            system: meta.system,
            generation: String::new(),
            fallback_for: None,
            adopt_only: false,
        })
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
        if let Backend::Remote(runtime) = &self.backend {
            let existed = runtime.has_worker(session_id);
            runtime.stop(session_id);
            return existed;
        }
        let Backend::Local(senders) = &self.backend else {
            unreachable!();
        };
        let tx = senders.lock().remove(session_id);
        match tx {
            Some(tx) => {
                let _ = tx.send(AgentCommand::Cancel);
                drop(tx);
                true
            }
            None => false,
        }
    }

    /// Replace a session's agent with a fresh context without deleting the
    /// Cowboy session. The remote broker needs an explicit reset operation so
    /// its permanent-delete tombstone cannot poison the replacement launch.
    pub fn reset_session(&self, session_id: &str) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        if self.prepare_session_inner(session_id)? {
            Ok(())
        } else {
            self.recycle_session_inner(session_id)
        }
    }

    /// Fence and replace one session's agent while preserving its resumable ACP
    /// id. Used after a force-cancel grace period expires; no other worker or
    /// session is touched.
    pub fn recycle_session(&self, session_id: &str) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        self.resolve_and_persist_cwd(session_id)?;
        self.recycle_session_inner(session_id)
    }

    /// Reconcile and recycle every session rooted in a Columbus project after
    /// its checkout was replaced. This is the migration fence for the case
    /// where the pathname stayed stable but a running worker still owns the old
    /// directory inode. It preserves Cowboy and native agent session ids.
    pub fn reconcile_project_sessions(
        &self,
        project: &str,
        dry_run: bool,
    ) -> Result<Vec<String>, String> {
        let _lifecycle = self.lifecycle.lock();
        let columbus = self.workspace_root.join("columbus");
        current_project_checkout(&columbus, project).ok_or_else(|| {
            format!(
                "cannot reconcile Cowboy sessions for Columbus project {project:?}: no usable current checkout; run `harness-cli --root {} project path {project}` first",
                columbus.display()
            )
        })?;
        let sessions: Vec<_> = self
            .hub
            .session_list()
            .into_iter()
            .filter(|meta| {
                session_belongs_to_project(&columbus, PathBuf::from(&meta.cwd).as_path(), project)
            })
            .collect();
        let active: Vec<_> = sessions
            .iter()
            .filter(|meta| {
                matches!(meta.status, Status::Busy | Status::Starting)
                    || self.hub.session_has_in_flight_prompt(&meta.id)
            })
            .map(|meta| format!("{} ({:?})", meta.id, meta.status))
            .collect();
        if !active.is_empty() {
            return Err(format!(
                "cannot migrate Columbus project {project:?} while Cowboy sessions are active: {}. Wait for those turns to finish and retry the migration",
                active.join(", ")
            ));
        }
        let session_ids: Vec<_> = sessions.into_iter().map(|meta| meta.id).collect();
        if dry_run {
            return Ok(session_ids);
        }

        for session_id in &session_ids {
            self.resolve_and_persist_cwd(session_id)?;
            self.recycle_session_inner(session_id)?;
        }
        Ok(session_ids)
    }

    /// Validate a session's persisted cwd before any operation that may reuse a
    /// worker. A migrated cwd or crashed worker fences and replaces exactly that
    /// session's worker; the Cowboy id and native agent session id remain
    /// unchanged. Recycling crashed workers is important when a checkout was
    /// replaced at the same pathname: the metadata still looks valid while the
    /// old app-server process holds the deleted directory inode.
    pub fn prepare_session(&self, session_id: &str) -> Result<bool, String> {
        let _lifecycle = self.lifecycle.lock();
        self.prepare_session_inner(session_id)
    }

    fn prepare_session_inner(&self, session_id: &str) -> Result<bool, String> {
        let migrated = self.resolve_and_persist_cwd(session_id)?;
        let crashed = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .is_some_and(|meta| meta.status == Status::Crashed);
        if !migrated && !crashed {
            return Ok(false);
        }
        if crashed && !migrated {
            tracing::warn!(
                session = session_id,
                "recycling crashed worker before session resume"
            );
        }
        self.recycle_session_inner(session_id)?;
        Ok(true)
    }

    fn resolve_and_persist_cwd(&self, session_id: &str) -> Result<bool, String> {
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        let stored_cwd = PathBuf::from(&meta.cwd);
        let resolved = resolve_session_workspace(&self.workspace_root, &stored_cwd)?;
        if !resolved.changed {
            return Ok(false);
        }
        self.hub
            .update_session_cwd(session_id, resolved.path.display().to_string())?;
        tracing::warn!(
            session = session_id,
            old_cwd = %meta.cwd,
            new_cwd = %resolved.path.display(),
            project = ?resolved.project,
            resume = ?meta.agent_session_id,
            "retargeting session after workspace migration"
        );
        Ok(true)
    }

    fn recycle_session_inner(&self, session_id: &str) -> Result<(), String> {
        match &self.backend {
            Backend::Remote(runtime) => {
                self.hub.set_status(session_id, Status::Starting, None);
                runtime.reset(self.start_session(session_id)?);
                Ok(())
            }
            Backend::Local(_) => {
                self.delete_session(session_id);
                self.revive(session_id)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_wire::{CoreCommand, WorkerSnapshot, WorkerState};
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "cowboy-supervisor-test-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("tempdir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn worker_snapshot(cwd: &str) -> WorkerSnapshot {
        WorkerSnapshot {
            session_id: "s".to_owned(),
            worker_epoch: "old-worker".to_owned(),
            generation: "test-generation".to_owned(),
            executable: Some("/bin/false".to_owned()),
            launch: Some(StartSession {
                session_id: "s".to_owned(),
                provider: "codex".to_owned(),
                cwd: cwd.to_owned(),
                agent_session_id: Some("codex-thread-1".to_owned()),
                system: false,
                generation: "test-generation".to_owned(),
                fallback_for: None,
                adopt_only: false,
            }),
            state: WorkerState::Crashed,
            agent_session_id: Some("codex-thread-1".to_owned()),
            current_turn_id: None,
            last_runtime_seq: 1,
            pending_permissions: Vec::new(),
            config_options: None,
            context_used: None,
            context_size: None,
            pending_prompt_count: 0,
            drain_requested: false,
        }
    }

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

    #[tokio::test]
    async fn crashed_session_recycles_worker_and_resumes_native_thread() {
        let root = TestDir::new();
        let cwd = root.path().join("checkout");
        std::fs::create_dir_all(&cwd).expect("checkout");
        let hub = Hub::new();
        hub.create_session(
            "s".to_owned(),
            "codex".to_owned(),
            cwd.display().to_string(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.set_agent_session_id("s", "codex-thread-1".to_owned());
        hub.set_status("s", Status::Crashed, Some("deleted cwd".to_owned()));
        let runtime = RemoteRuntime::for_test(
            hub.clone(),
            vec![worker_snapshot(cwd.to_string_lossy().as_ref())],
        );
        let supervisor = Supervisor::new_remote(hub.clone(), root.0.clone(), 0, runtime.clone());

        assert!(supervisor.prepare_session("s").expect("prepare"));

        let pending = runtime.pending_for_test();
        assert!(pending.iter().any(|command| {
            matches!(
                command,
                CoreCommand::EnsureSession { session }
                    if session.cwd == cwd.to_string_lossy()
                        && session.agent_session_id.as_deref() == Some("codex-thread-1")
            )
        }));
        assert!(pending.iter().any(|command| {
            matches!(
                command,
                CoreCommand::StopSession { command_id, .. }
                    if command_id.starts_with("reset-")
            )
        }));
        let meta = hub.session_info("s").expect("session").meta;
        assert_eq!(meta.id, "s");
        assert_eq!(meta.agent_session_id.as_deref(), Some("codex-thread-1"));
        assert_eq!(meta.status, Status::Starting);
    }

    #[test]
    fn project_reconcile_dry_run_blocks_active_sessions_without_mutation() {
        let root = TestDir::new();
        let columbus = root.path().join("columbus");
        let definition = columbus.join("project-defs/corsair/project.toml");
        std::fs::create_dir_all(definition.parent().expect("definition parent"))
            .expect("definition dir");
        std::fs::write(
            definition,
            "kind = \"external\"\nrepo = \"git@example/corsair\"\ndefault_branch = \"main\"\n",
        )
        .expect("definition");
        let checkout = columbus.join("projects/corsair/main");
        std::fs::create_dir_all(&checkout).expect("checkout");
        let hub = Hub::new();
        hub.create_session(
            "s".to_owned(),
            "codex".to_owned(),
            checkout.display().to_string(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.set_status("s", Status::Busy, None);
        let supervisor = Supervisor::new(hub.clone(), root.0.clone(), 0);

        let error = supervisor
            .reconcile_project_sessions("corsair", true)
            .expect_err("busy session must block migration");
        assert!(error.contains("Wait for those turns to finish"));
        assert_eq!(hub.status("s"), Some(Status::Busy));

        hub.set_status("s", Status::Running, None);
        assert_eq!(
            supervisor
                .reconcile_project_sessions("corsair", true)
                .expect("idle dry run"),
            vec!["s"]
        );
        assert_eq!(hub.status("s"), Some(Status::Running));
    }
}
