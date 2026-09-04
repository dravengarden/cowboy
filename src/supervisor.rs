//! Session supervisor (design §7).
//!
//! Owns each agent's lifetime, decoupled from any client connection. Because
//! the `agent-client-protocol` connection is `!Send` (single-threaded,
//! spawn-local), each session runs on its **own OS thread** with a
//! current-thread tokio runtime + `LocalSet` (see [`crate::acp::run_agent`]).
//! The supervisor talks to that thread only through a `Send` command channel.

#![warn(clippy::pedantic)]

use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::acp::AgentCommand;
use crate::core::{Hub, SessionOrigin, SessionRegistration, Status};
use crate::provider::{self, LaunchSpec};
use crate::remote_runtime::RemoteRuntime;
use crate::runtime_router::RuntimeRouter;
use crate::runtime_wire::StartSession;
use crate::workspace::{
    current_project_checkout, resolve_session_workspace, session_belongs_to_project,
};

/// Stable Machine placement selected for a newly registered session.
#[derive(Clone, Copy)]
pub struct SessionPlacement<'a> {
    pub machine_id: &'a str,
    pub workspace: Option<&'a crate::machine_protocol::MachineWorkspace>,
}

/// Exact immutable Agent Plugin release and Service-auth projection selected by
/// the Controller before a session is registered.
#[derive(Clone, Copy)]
pub struct ProviderGeneration<'a> {
    pub version: &'a str,
    pub digest: &'a str,
    pub auth_generation: Option<u64>,
    pub behavior: Option<&'a cowboy_provider_sdk::ProviderBehaviorContract>,
}

/// Product-account stamp written at session creation. Absent keeps the row in
/// the unowned shared pool.
#[derive(Clone, Copy)]
pub struct SessionOwner<'a> {
    pub user_id: &'a str,
    pub username: Option<&'a str>,
}

/// Spawns and tracks agent sessions; routes commands to their threads.
pub struct Supervisor {
    hub: Hub,
    workspace_root: PathBuf,
    router: Arc<RuntimeRouter>,
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

fn session_configuration(
    meta: &crate::core::SessionMeta,
) -> cowboy_provider_sdk::ConfigurationBehavior {
    meta.provider_behavior.as_ref().map_or_else(
        || provider::legacy_behavior(&meta.provider).configuration,
        |behavior| behavior.configuration.clone(),
    )
}

impl Supervisor {
    #[must_use]
    #[cfg(test)]
    pub fn new_remote(
        hub: Hub,
        workspace_root: PathBuf,
        persistent_floor: u64,
        runtime: Arc<RemoteRuntime>,
    ) -> Self {
        let router = RuntimeRouter::new();
        router.install("local".to_owned(), runtime);
        Self::new(hub, workspace_root, persistent_floor, router)
    }

    #[must_use]
    pub fn new(
        hub: Hub,
        workspace_root: PathBuf,
        persistent_floor: u64,
        router: Arc<RuntimeRouter>,
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
            router,
            counter: AtomicU64::new(initial),
            lifecycle: Mutex::new(()),
        }
    }

    /// The configured root against which relative session paths resolve.
    /// Exposed so the server can enumerate Machine-owned workspaces beneath it.
    #[must_use]
    pub fn workspace_root(&self) -> &std::path::Path {
        &self.workspace_root
    }

    /// Reserve the stable Cowboy id before a selected Machine prepares its
    /// session-specific workspace.
    pub fn reserve_session_id(&self) -> String {
        format!("sess-{}", self.counter.fetch_add(1, Ordering::Relaxed))
    }

    /// Register a session before Machine-local workspace preparation begins.
    /// The caller may return this stable id to the UI immediately; the session
    /// remains `Starting` until [`Self::start_registered_session`] installs its
    /// worker, or becomes `Crashed` with the preparation error.
    #[allow(clippy::too_many_arguments)]
    pub fn register_session_on_with_id(
        &self,
        id: &str,
        provider: &str,
        cwd: Option<String>,
        origin: SessionOrigin,
        system: bool,
        placement: SessionPlacement<'_>,
        provider_generation: ProviderGeneration<'_>,
        owner: Option<SessionOwner<'_>>,
    ) -> Result<String, String> {
        let SessionPlacement {
            machine_id,
            workspace,
        } = placement;
        let ProviderGeneration {
            version,
            digest,
            auth_generation,
            behavior,
        } = provider_generation;
        if !self.router.connected(machine_id) {
            return Err(format!("machine {machine_id:?} is not connected"));
        }
        if digest.is_empty() {
            provider::lookup(provider).ok_or_else(|| format!("unknown provider {provider:?}"))?;
        } else {
            provider::remote_generation(provider)
                .ok_or_else(|| format!("invalid Provider id {provider:?}"))?;
        }

        // Resolve cwd. Relative paths join the workspace_root; absolute
        // paths are honoured as-is. Remote paths have already been validated
        // and prepared by the selected Machine; local legacy sessions retain
        // the original full-user filesystem boundary.
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

        let title = format!("{provider} · {}", cwd.display());
        self.hub.create_session(SessionRegistration {
            id: id.to_owned(),
            provider: provider.to_owned(),
            provider_version: version.to_owned(),
            provider_generation_digest: digest.to_owned(),
            provider_auth_generation: auth_generation,
            provider_behavior: behavior.cloned(),
            machine_id: machine_id.to_owned(),
            workspace_id: workspace.map(|value| value.id.clone()),
            workspace_name: workspace.map(|value| value.display_name.clone()),
            workspace_source_path: workspace.map(|value| value.canonical_path.clone()),
            cwd: cwd.display().to_string(),
            title,
            origin,
            system,
            owner_user_id: owner.map(|owner| owner.user_id.to_owned()),
            owner_username: owner.and_then(|owner| owner.username.map(str::to_owned)),
        });

        Ok(id.to_owned())
    }

    /// Start the worker for an already registered fresh session.
    ///
    /// A client can open the projected session before Machine workspace
    /// preparation completes. If that raced an older controller into starting
    /// a worker from the stable source checkout, fence that worker here so the
    /// prepared session-owned cwd is authoritative.
    pub fn start_registered_session(&self, session_id: &str) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        let spec = if meta.provider_generation_digest.is_empty() {
            provider::lookup(&meta.provider)
        } else {
            provider::remote_generation(&meta.provider)
        }
        .ok_or_else(|| format!("unknown provider {:?}", meta.provider))?;
        let runtime = self.runtime_for_session(session_id)?;
        if runtime.has_worker(session_id) && !runtime.worker_matches_cwd(session_id, &meta.cwd) {
            self.recycle_session_inner(session_id)
        } else {
            self.ensure_worker(session_id, &spec, std::path::Path::new(&meta.cwd), None)
        }
    }

    /// Compatibility path for callers that already own a prepared workspace.
    #[allow(clippy::too_many_arguments)]
    pub fn new_session_on_with_id(
        &self,
        id: &str,
        provider: &str,
        cwd: Option<String>,
        origin: SessionOrigin,
        system: bool,
        machine_id: &str,
        provider_generation: ProviderGeneration<'_>,
        owner: Option<SessionOwner<'_>>,
    ) -> Result<String, String> {
        let id = self.register_session_on_with_id(
            id,
            provider,
            cwd,
            origin,
            system,
            SessionPlacement {
                machine_id,
                workspace: None,
            },
            provider_generation,
            owner,
        )?;
        self.start_registered_session(&id)?;
        Ok(id)
    }

    /// Ensure the selected Machine owns a detached worker for `session_id`.
    ///
    /// # Errors
    /// If the Machine runtime is disconnected.
    fn ensure_worker(
        &self,
        session_id: &str,
        spec: &LaunchSpec,
        cwd: &std::path::Path,
        resume: Option<String>,
    ) -> Result<(), String> {
        let runtime = self.runtime_for_session(session_id)?;
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        let configuration = session_configuration(&meta);
        let budget = self.deepseek_context_budget(session_id, &configuration);
        let cache_protection = self.deepseek_cache_protection(session_id, &configuration);
        runtime.ensure(StartSession {
            session_id: session_id.to_owned(),
            provider: spec.id.clone(),
            provider_version: meta.provider_version,
            provider_generation_digest: meta.provider_generation_digest,
            provider_auth_generation: meta.provider_auth_generation,
            provider_behavior: meta.provider_behavior,
            cwd: cwd.display().to_string(),
            agent_session_id: resume,
            system: self.hub.session_is_system(session_id),
            context_window: budget.map(|value| value.context_window),
            auto_compact_token_limit: budget.map(|value| value.auto_compact_token_limit),
            cache_protection,
            generation: String::new(),
            fallback_for: None,
            adopt_only: false,
        });
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
        let runtime = self.runtime_for_session(session_id)?;
        match command {
            AgentCommand::Prompt(blocks, cmid, completion) => {
                if let Some(completion) = completion {
                    let _ = completion.send(Err(
                        "direct completion capture is unavailable through detached runtime"
                            .to_owned(),
                    ));
                    return Err("detached runtime does not support completion capture".to_owned());
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
                // A config change may arrive from a second device while this
                // session is dormant after a restart. Ensure the worker exists
                // before routing the command; the runtime will coalesce this
                // with the persisted session preference replay.
                runtime.ensure(self.start_session(session_id)?);
                runtime.set_config_option(session_id, &config_id, value);
            }
        }
        Ok(())
    }

    /// Ensure a session has a live detached worker without sending a turn.
    ///
    /// Returns `true` if it revived, `false` if the agent was already alive.
    ///
    /// # Errors
    /// If the session is unknown or its Machine runtime is disconnected.
    pub fn ensure_alive(&self, session_id: &str) -> Result<bool, String> {
        let _lifecycle = self.lifecycle.lock();
        if self.prepare_session_inner(session_id)? {
            return Ok(true);
        }
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        // Web creation registers the durable id against the stable source root
        // before Machine asynchronously returns the isolated worktree. Opening
        // that projected session must not spawn a worker from the source root.
        // The preparation task updates `cwd` and calls start_registered_session.
        if meta.status == Status::Starting
            && meta.machine_id != "local"
            && meta.origin == SessionOrigin::Web
            && meta.workspace_source_path.as_deref() == Some(meta.cwd.as_str())
        {
            return Ok(false);
        }
        let runtime = self.runtime_for_session(session_id)?;
        let has_worker = runtime.has_worker(session_id);
        if has_worker
            && meta.status != Status::Busy
            && !runtime.worker_matches_cwd(session_id, &meta.cwd)
        {
            self.recycle_session_inner(session_id)?;
            return Ok(true);
        }
        // An explicit open is also a state-reconciliation request. The
        // Controller and Machine can briefly disagree after reconnect (for
        // example, a wedged worker may look live here after its durable
        // lifecycle already reached Exited). Always let the Machine broker
        // make the final idempotent decision: a healthy worker is a no-op,
        // while a terminal owner is fenced and replaced.
        runtime.ensure(self.start_session(session_id)?);
        Ok(!has_worker)
    }

    /// Build the idempotent worker launch declaration for a persisted session.
    fn start_session(&self, session_id: &str) -> Result<StartSession, String> {
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        let configuration = session_configuration(&meta);
        let budget = self.deepseek_context_budget(session_id, &configuration);
        let cache_protection = self.deepseek_cache_protection(session_id, &configuration);
        Ok(StartSession {
            session_id: meta.id,
            provider: meta.provider,
            provider_version: meta.provider_version,
            provider_generation_digest: meta.provider_generation_digest,
            provider_auth_generation: meta.provider_auth_generation,
            provider_behavior: meta.provider_behavior,
            cwd: meta.cwd,
            agent_session_id: self.hub.agent_session_id_for_resume(session_id),
            system: meta.system,
            context_window: budget.map(|value| value.context_window),
            auto_compact_token_limit: budget.map(|value| value.auto_compact_token_limit),
            cache_protection,
            generation: String::new(),
            fallback_for: None,
            adopt_only: false,
        })
    }

    fn deepseek_context_budget(
        &self,
        session_id: &str,
        configuration: &cowboy_provider_sdk::ConfigurationBehavior,
    ) -> Option<crate::deepseek_context::ContextBudget> {
        let preferences = self.hub.config_preferences(session_id)?;
        let model = preferences.get("model").and_then(serde_json::Value::as_str);
        let requested = preferences
            .get(crate::deepseek_context::CONFIG_ID)
            .and_then(serde_json::Value::as_str);
        crate::deepseek_context::launch_budget(configuration, model, requested)
    }

    fn deepseek_cache_protection(
        &self,
        session_id: &str,
        configuration: &cowboy_provider_sdk::ConfigurationBehavior,
    ) -> Option<bool> {
        let preferences = self.hub.config_preferences(session_id)?;
        crate::deepseek_cache::selected(&preferences, configuration)
    }

    /// Tear down a session's detached worker. Hub state is the caller's
    /// responsibility — pair with [`Hub::delete_session`].
    ///
    /// Returns `true` if the session had a live worker.
    pub fn delete_session(&self, session_id: &str) -> bool {
        let Ok(runtime) = self.runtime_for_session(session_id) else {
            return false;
        };
        let existed = runtime.has_worker(session_id);
        runtime.stop(session_id);
        existed
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

    /// Reload one session's runtime while preserving its Cowboy state and
    /// resumable native agent id. Unlike [`Self::reset_session`], this does not
    /// clear transcript history or start a fresh agent context. Persisted config
    /// preferences are replayed by [`RemoteRuntime::reset`] before the replacement
    /// worker can accept another prompt.
    pub fn reload_session(
        &self,
        session_id: &str,
        confirm_active_turn: bool,
    ) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        if meta.status == Status::Starting
            && meta.machine_id != "local"
            && meta.origin == SessionOrigin::Web
            && meta.workspace_source_path.as_deref() == Some(meta.cwd.as_str())
        {
            return Err(
                "session workspace is still being prepared; reload it after preparation finishes"
                    .to_owned(),
            );
        }

        let active_turn = matches!(meta.status, Status::Busy | Status::Starting)
            && self.hub.session_has_in_flight_prompt(session_id);
        if active_turn && !confirm_active_turn {
            return Err(
                "session has an active turn; confirm reload to stop it before retrying".to_owned(),
            );
        }

        self.resolve_and_persist_cwd(session_id)?;
        if active_turn {
            self.hub.set_status(
                session_id,
                Status::Interrupted,
                Some("current turn stopped by session reload".to_owned()),
            );
        }
        self.recycle_session_inner(session_id)
    }

    /// Fence and replace one session's agent while preserving its resumable ACP
    /// id. Used after a force-cancel grace period expires; no other worker or
    /// session is touched.
    pub fn recycle_session(&self, session_id: &str) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        self.resolve_and_persist_cwd(session_id)?;
        self.recycle_session_inner(session_id)
    }

    /// Apply one Cowboy-owned `DeepSeek` context profile. This setting changes
    /// process startup rather than an ACP option, so recycle only this idle
    /// worker while preserving both the Cowboy and native agent session ids.
    pub fn set_deepseek_context_profile(
        &self,
        session_id: &str,
        value: serde_json::Value,
    ) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        let profile = value
            .as_str()
            .ok_or_else(|| "DeepSeek context profile must be a string id".to_owned())?;
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        if matches!(meta.status, Status::Busy | Status::Starting)
            || self.hub.session_has_in_flight_prompt(session_id)
        {
            return Err(
                "wait for the current turn to finish before changing the context budget".to_owned(),
            );
        }
        let preferences = self
            .hub
            .config_preferences(session_id)
            .unwrap_or_else(|| serde_json::json!({}));
        let model = preferences.get("model").and_then(serde_json::Value::as_str);
        let configuration = session_configuration(&meta);
        crate::deepseek_context::resolve(&configuration, model, profile)?;
        self.runtime_for_session(session_id)?;
        let unchanged = preferences
            .get(crate::deepseek_context::CONFIG_ID)
            .and_then(serde_json::Value::as_str)
            == Some(profile);
        self.hub.set_config_preference(
            session_id,
            crate::deepseek_context::CONFIG_ID.to_owned(),
            value,
        )?;
        if unchanged {
            return Ok(());
        }
        self.resolve_and_persist_cwd(session_id)?;
        self.recycle_session_inner(session_id)
    }

    /// Enable or disable Cowboy-owned `DeepSeek` cache protection. The policy is
    /// carried in a local HTTP header, never in the model prompt. Recycle only
    /// an idle worker so an active agent turn is never interrupted.
    pub fn set_deepseek_cache_protection(
        &self,
        session_id: &str,
        value: serde_json::Value,
    ) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        let enabled = value
            .as_bool()
            .ok_or_else(|| "DeepSeek cache protection must be a boolean".to_owned())?;
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        let configuration = session_configuration(&meta);
        if !crate::deepseek_cache::supported_behavior(&configuration) {
            return Err("cache protection is available only for DeepSeek sessions".to_owned());
        }
        if matches!(meta.status, Status::Busy | Status::Starting)
            || self.hub.session_has_in_flight_prompt(session_id)
        {
            return Err(
                "wait for the current turn to finish before changing cache protection".to_owned(),
            );
        }
        let preferences = self
            .hub
            .config_preferences(session_id)
            .unwrap_or_else(|| serde_json::json!({}));
        let unchanged =
            crate::deepseek_cache::selected(&preferences, &configuration) == Some(enabled);
        self.hub.set_config_preference(
            session_id,
            crate::deepseek_cache::CONFIG_ID.to_owned(),
            value,
        )?;
        if unchanged {
            return Ok(());
        }
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
        let session = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id);
        let crashed = session
            .as_ref()
            .is_some_and(|meta| meta.status == Status::Crashed);
        if !migrated && !crashed {
            return Ok(false);
        }
        let live_recoverable_turn_failure = crashed
            && !migrated
            && session.is_some_and(|meta| {
                self.hub
                    .latest_crash_detail(session_id)
                    .as_deref()
                    .is_some_and(|detail| {
                        let behavior = meta
                            .provider_behavior
                            .clone()
                            .unwrap_or_else(|| provider::legacy_behavior(&meta.provider));
                        crate::provider::keeps_worker_alive_for_behavior(&behavior, detail)
                    })
            })
            && self.runtime_for_session(session_id)?.has_worker(session_id);
        if live_recoverable_turn_failure {
            tracing::info!(
                session = session_id,
                "reusing live ACP worker after recoverable provider turn failure"
            );
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
        // Remote paths belong to the selected Machine and are validated by
        // its trusted-workspace boundary. Never reinterpret them against the
        // controller's local workspace layout during resume.
        if meta.machine_id != "local" {
            return Ok(false);
        }
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
        let runtime = self.runtime_for_session(session_id)?;
        self.hub.set_status(session_id, Status::Starting, None);
        runtime.reset(self.start_session(session_id)?);
        Ok(())
    }

    fn runtime_for_session(&self, session_id: &str) -> Result<Arc<RemoteRuntime>, String> {
        let machine_id = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .map(|meta| meta.machine_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        self.router
            .runtime(&machine_id)
            .ok_or_else(|| format!("machine {machine_id:?} is not connected"))
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
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: cwd.to_owned(),
                agent_session_id: Some("codex-thread-1".to_owned()),
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
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

    fn preparing_web_session(hub: &Hub, cwd: &str) {
        hub.create_session(SessionRegistration {
            id: "s".to_owned(),
            provider: "codex".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "hawk".to_owned(),
            workspace_id: Some("columbus".to_owned()),
            workspace_name: Some("columbus".to_owned()),
            workspace_source_path: Some(cwd.to_owned()),
            cwd: cwd.to_owned(),
            title: "test".to_owned(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
    }

    fn remote_supervisor(hub: Hub, runtime: Arc<RemoteRuntime>, root: PathBuf) -> Supervisor {
        let router = RuntimeRouter::new();
        router.install("hawk".to_owned(), runtime);
        Supervisor::new(hub, root, 0, router)
    }

    #[test]
    fn session_counter_honors_live_persistent_and_clock_floors() {
        let hub = Hub::new();
        hub.create_local_session(
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
    async fn opening_registered_web_session_waits_for_isolated_workspace() {
        let root = TestDir::new();
        let source = root.path().join("columbus");
        std::fs::create_dir_all(&source).expect("source");
        let hub = Hub::new();
        preparing_web_session(&hub, source.to_string_lossy().as_ref());
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub, runtime.clone(), root.0.clone());

        assert!(
            !supervisor
                .ensure_alive("s")
                .expect("open preparing session")
        );
        assert!(runtime.pending_for_test().is_empty());
    }

    #[tokio::test]
    async fn prepared_workspace_recycles_worker_started_from_source_checkout() {
        let root = TestDir::new();
        let source = root.path().join("columbus");
        let prepared = root.path().join("worktrees/s");
        std::fs::create_dir_all(&source).expect("source");
        std::fs::create_dir_all(&prepared).expect("prepared");
        let hub = Hub::new();
        preparing_web_session(&hub, source.to_string_lossy().as_ref());
        let mut worker = worker_snapshot(source.to_string_lossy().as_ref());
        worker.state = WorkerState::Running;
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![worker]);
        let supervisor = remote_supervisor(hub.clone(), runtime.clone(), root.0.clone());
        hub.update_session_cwd("s", prepared.display().to_string())
            .expect("prepared cwd");

        supervisor
            .start_registered_session("s")
            .expect("start prepared session");

        let pending = runtime.pending_for_test();
        assert!(pending.iter().any(|command| {
            matches!(
                command,
                CoreCommand::EnsureSession { session }
                    if session.cwd == prepared.to_string_lossy()
            )
        }));
        assert!(pending.iter().any(|command| {
            matches!(command, CoreCommand::StopSession { command_id, .. } if command_id.starts_with("reset-"))
        }));
    }

    #[tokio::test]
    async fn duplicate_prepared_workspace_start_remains_idempotent() {
        let root = TestDir::new();
        let prepared = root.path().join("worktrees/s");
        std::fs::create_dir_all(&prepared).expect("prepared");
        let hub = Hub::new();
        preparing_web_session(&hub, prepared.to_string_lossy().as_ref());
        let mut worker = worker_snapshot(prepared.to_string_lossy().as_ref());
        worker.state = WorkerState::Running;
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![worker]);
        let supervisor = remote_supervisor(hub, runtime.clone(), root.0.clone());

        supervisor
            .start_registered_session("s")
            .expect("repeat prepared start");

        let pending = runtime.pending_for_test();
        assert!(pending.iter().any(|command| {
            matches!(command, CoreCommand::EnsureSession { session } if session.cwd == prepared.to_string_lossy())
        }));
        assert!(
            !pending
                .iter()
                .any(|command| matches!(command, CoreCommand::StopSession { .. }))
        );
    }

    #[tokio::test]
    async fn opening_idle_session_repairs_a_stale_worker_cwd() {
        let root = TestDir::new();
        let source = root.path().join("columbus");
        let prepared = root.path().join("worktrees/s");
        std::fs::create_dir_all(&source).expect("source");
        std::fs::create_dir_all(&prepared).expect("prepared");
        let hub = Hub::new();
        preparing_web_session(&hub, source.to_string_lossy().as_ref());
        hub.update_session_cwd("s", prepared.display().to_string())
            .expect("prepared cwd");
        hub.set_status("s", Status::Running, None);
        let mut worker = worker_snapshot(source.to_string_lossy().as_ref());
        worker.state = WorkerState::Running;
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![worker]);
        let supervisor = remote_supervisor(hub, runtime.clone(), root.0.clone());

        assert!(supervisor.ensure_alive("s").expect("open stale worker"));

        let pending = runtime.pending_for_test();
        assert!(pending.iter().any(|command| {
            matches!(command, CoreCommand::EnsureSession { session } if session.cwd == prepared.to_string_lossy())
        }));
        assert!(
            pending
                .iter()
                .any(|command| matches!(command, CoreCommand::StopSession { .. }))
        );
    }

    #[tokio::test]
    async fn opening_busy_session_defers_stale_worker_repair() {
        let root = TestDir::new();
        let source = root.path().join("columbus");
        let prepared = root.path().join("worktrees/s");
        std::fs::create_dir_all(&source).expect("source");
        std::fs::create_dir_all(&prepared).expect("prepared");
        let hub = Hub::new();
        preparing_web_session(&hub, source.to_string_lossy().as_ref());
        hub.update_session_cwd("s", prepared.display().to_string())
            .expect("prepared cwd");
        hub.set_status("s", Status::Busy, None);
        let mut worker = worker_snapshot(source.to_string_lossy().as_ref());
        worker.state = WorkerState::Busy;
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![worker]);
        let supervisor = remote_supervisor(hub, runtime.clone(), root.0.clone());

        assert!(!supervisor.ensure_alive("s").expect("open busy worker"));
        let pending = runtime.pending_for_test();
        assert!(pending.iter().any(|command| {
            matches!(command, CoreCommand::EnsureSession { session } if session.session_id == "s")
        }));
        assert!(
            !pending
                .iter()
                .any(|command| matches!(command, CoreCommand::StopSession { .. }))
        );
    }

    #[tokio::test]
    async fn opening_exited_session_reasserts_ensure_for_divergent_runtime_snapshot() {
        let root = TestDir::new();
        let prepared = root.path().join("worktrees/s");
        std::fs::create_dir_all(&prepared).expect("prepared");
        let hub = Hub::new();
        preparing_web_session(&hub, prepared.to_string_lossy().as_ref());
        hub.set_status("s", Status::Exited, None);
        let mut worker = worker_snapshot(prepared.to_string_lossy().as_ref());
        worker.state = WorkerState::Running;
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![worker]);
        let supervisor = remote_supervisor(hub, runtime.clone(), root.0.clone());

        assert!(
            !supervisor
                .ensure_alive("s")
                .expect("open divergent exited session")
        );

        let pending = runtime.pending_for_test();
        assert!(pending.iter().any(|command| {
            matches!(command, CoreCommand::EnsureSession { session } if session.session_id == "s")
        }));
        assert!(
            !pending
                .iter()
                .any(|command| matches!(command, CoreCommand::StopSession { .. }))
        );
    }

    #[tokio::test]
    async fn crashed_session_recycles_worker_and_resumes_native_thread() {
        let root = TestDir::new();
        let cwd = root.path().join("checkout");
        std::fs::create_dir_all(&cwd).expect("checkout");
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex".to_owned(),
            cwd.display().to_string(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.push(
            "s",
            crate::core::Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": "durable prompt"},
                }),
            },
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

    #[tokio::test]
    async fn reload_preserves_history_pending_state_config_and_native_thread() {
        let root = TestDir::new();
        let cwd = root.path().join("checkout");
        std::fs::create_dir_all(&cwd).expect("checkout");
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex".to_owned(),
            cwd.display().to_string(),
            "keep this title".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.push(
            "s",
            crate::core::Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": "keep this history"},
                }),
            },
        );
        hub.set_agent_session_id("s", "codex-thread-1".to_owned());
        hub.set_config_preference("s", "model".to_owned(), serde_json::json!("gpt-test"))
            .expect("config preference");
        hub.set_status("s", Status::Running, None);
        let (dispatch_tx, mut dispatch_rx) = tokio::sync::mpsc::channel(4);
        hub.set_dispatch_tx(dispatch_tx);
        hub.submit("s", "active turn".to_owned(), Vec::new(), None);
        assert_eq!(
            dispatch_rx.recv().await.expect("active dispatch").text,
            "active turn"
        );
        hub.set_status("s", Status::Busy, None);
        hub.submit("s", "keep queued".to_owned(), Vec::new(), None);
        hub.add_draft("s", "keep draft".to_owned(), Vec::new(), None);

        let info_before = hub.session_info("s").expect("session before reload");
        let (history_before, _) = hub.snapshot("s").expect("history before reload");
        let history_before = serde_json::to_value(history_before).expect("serialize history");
        let preferences_before = hub
            .config_preferences("s")
            .expect("preferences before reload");
        let mut worker = worker_snapshot(cwd.to_string_lossy().as_ref());
        worker.state = WorkerState::Busy;
        worker.current_turn_id = Some("turn-1".to_owned());
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![worker]);
        let supervisor = Supervisor::new_remote(hub.clone(), root.0.clone(), 0, runtime.clone());

        supervisor
            .reload_session("s", true)
            .expect("reload session");

        let info_after = hub.session_info("s").expect("session after reload");
        assert_eq!(info_after.meta.id, info_before.meta.id);
        assert_eq!(info_after.meta.title, info_before.meta.title);
        assert_eq!(info_after.meta.cwd, info_before.meta.cwd);
        assert_eq!(
            info_after.meta.agent_session_id,
            info_before.meta.agent_session_id
        );
        assert_eq!(info_after.queue_count, info_before.queue_count);
        assert_eq!(info_after.drafts_count, info_before.drafts_count);
        assert_eq!(hub.config_preferences("s"), Some(preferences_before));
        assert_eq!(info_after.meta.status, Status::Starting);

        let (history_after, _) = hub.snapshot("s").expect("history after reload");
        let retained_event_count =
            usize::try_from(info_before.event_count).expect("test event count fits in usize");
        assert!(history_after.len() > retained_event_count);
        assert_eq!(
            serde_json::to_value(&history_after[..retained_event_count])
                .expect("serialize preserved history"),
            history_before
        );

        let pending = runtime.pending_for_test();
        assert!(pending.iter().any(|command| {
            matches!(
                command,
                CoreCommand::EnsureSession { session }
                    if session.agent_session_id.as_deref() == Some("codex-thread-1")
            )
        }));
        assert!(pending.iter().any(|command| {
            matches!(
                command,
                CoreCommand::SetConfigOption { config_id, value, .. }
                    if config_id == "model" && value == &serde_json::json!("gpt-test")
            )
        }));
        assert!(pending.iter().any(|command| {
            matches!(command, CoreCommand::StopSession { command_id, .. } if command_id.starts_with("reset-"))
        }));
    }

    #[tokio::test]
    async fn reload_rejects_an_active_turn_without_explicit_confirmation() {
        let root = TestDir::new();
        let cwd = root.path().join("checkout");
        std::fs::create_dir_all(&cwd).expect("checkout");
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex".to_owned(),
            cwd.display().to_string(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.set_status("s", Status::Running, None);
        let (dispatch_tx, mut dispatch_rx) = tokio::sync::mpsc::channel(1);
        hub.set_dispatch_tx(dispatch_tx);
        hub.submit("s", "active turn".to_owned(), Vec::new(), None);
        assert_eq!(
            dispatch_rx.recv().await.expect("active dispatch").text,
            "active turn"
        );
        hub.set_status("s", Status::Busy, None);
        let mut worker = worker_snapshot(cwd.to_string_lossy().as_ref());
        worker.state = WorkerState::Busy;
        worker.current_turn_id = Some("turn-1".to_owned());
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![worker]);
        let supervisor = Supervisor::new_remote(hub.clone(), root.0.clone(), 0, runtime.clone());

        let error = supervisor
            .reload_session("s", false)
            .expect_err("unconfirmed active reload must be rejected");

        assert!(error.contains("active turn"));
        assert_eq!(hub.status("s"), Some(Status::Busy));
        assert!(hub.session_has_in_flight_prompt("s"));
        assert!(runtime.pending_for_test().is_empty());
    }

    #[tokio::test]
    async fn reload_allows_an_idle_turn_without_confirmation() {
        let root = TestDir::new();
        let cwd = root.path().join("checkout");
        std::fs::create_dir_all(&cwd).expect("checkout");
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex".to_owned(),
            cwd.display().to_string(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.set_status("s", Status::Running, None);
        let runtime = RemoteRuntime::for_test(
            hub.clone(),
            vec![worker_snapshot(cwd.to_string_lossy().as_ref())],
        );
        let supervisor = Supervisor::new_remote(hub.clone(), root.0.clone(), 0, runtime.clone());

        supervisor
            .reload_session("s", false)
            .expect("idle reload does not need active-turn confirmation");

        assert_eq!(hub.status("s"), Some(Status::Starting));
        assert!(runtime.pending_for_test().iter().any(|command| {
            matches!(command, CoreCommand::StopSession { command_id, .. } if command_id.starts_with("reset-"))
        }));
    }

    #[tokio::test]
    async fn reload_waits_for_remote_web_workspace_preparation() {
        let root = TestDir::new();
        let source = root.path().join("columbus");
        std::fs::create_dir_all(&source).expect("source");
        let hub = Hub::new();
        preparing_web_session(&hub, source.to_string_lossy().as_ref());
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub.clone(), runtime.clone(), root.0.clone());

        let error = supervisor
            .reload_session("s", false)
            .expect_err("preparing workspace must reject reload");

        assert!(error.contains("workspace is still being prepared"));
        assert_eq!(hub.status("s"), Some(Status::Starting));
        assert!(runtime.pending_for_test().is_empty());
    }

    #[tokio::test]
    async fn context_rejection_reuses_live_claude_workers_without_resume() {
        for provider in ["claude-code", "claude-deepseek"] {
            let root = TestDir::new();
            let cwd = root.path().join("checkout");
            std::fs::create_dir_all(&cwd).expect("checkout");
            let hub = Hub::new();
            hub.create_local_session(
                "s".to_owned(),
                provider.to_owned(),
                cwd.display().to_string(),
                "test".to_owned(),
                SessionOrigin::Web,
                false,
            );
            let detail = "API Error: 400 This model's maximum context length is 1048576 tokens. However, you requested 1048875 tokens";
            hub.set_status("s", Status::Crashed, Some(detail.to_owned()));
            let mut worker = worker_snapshot(cwd.to_string_lossy().as_ref());
            worker.state = WorkerState::Running;
            worker.launch.as_mut().expect("launch").provider = provider.to_owned();
            let runtime = RemoteRuntime::for_test(hub.clone(), vec![worker]);
            let supervisor =
                Supervisor::new_remote(hub.clone(), root.0.clone(), 0, runtime.clone());

            assert!(!supervisor.prepare_session("s").expect("prepare"));
            assert!(runtime.has_worker("s"));
            assert!(runtime.pending_for_test().is_empty());
            assert_eq!(hub.status("s"), Some(Status::Crashed));
            assert_eq!(hub.latest_crash_detail("s").as_deref(), Some(detail));
        }
    }

    #[tokio::test]
    async fn empty_claude_stream_reuses_live_worker_without_resume() {
        let root = TestDir::new();
        let cwd = root.path().join("checkout");
        std::fs::create_dir_all(&cwd).expect("checkout");
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "claude-code".to_owned(),
            cwd.display().to_string(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        let detail =
            "API Error: Stream ended without receiving any events {\"errorKind\":\"unknown\"}";
        hub.set_status("s", Status::Crashed, Some(detail.to_owned()));
        let mut worker = worker_snapshot(cwd.to_string_lossy().as_ref());
        worker.state = WorkerState::Running;
        worker.launch.as_mut().expect("launch").provider = "claude-code".to_owned();
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![worker]);
        let supervisor = Supervisor::new_remote(hub.clone(), root.0.clone(), 0, runtime.clone());

        assert!(!supervisor.prepare_session("s").expect("prepare"));
        assert!(runtime.has_worker("s"));
        assert!(runtime.pending_for_test().is_empty());
        assert_eq!(hub.status("s"), Some(Status::Crashed));
        assert_eq!(hub.latest_crash_detail("s").as_deref(), Some(detail));
    }

    #[tokio::test]
    async fn crashed_empty_context_recycles_worker_without_unresumable_native_id() {
        let root = TestDir::new();
        let cwd = root.path().join("checkout");
        std::fs::create_dir_all(&cwd).expect("checkout");
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex".to_owned(),
            cwd.display().to_string(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.mark_context_cleared("s");
        hub.set_agent_session_id("s", "codex-thread-without-rollout".to_owned());
        hub.set_status("s", Status::Crashed, Some("no rollout found".to_owned()));
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
                        && session.agent_session_id.is_none()
            )
        }));
    }

    #[tokio::test]
    async fn changing_deepseek_context_recycles_only_the_idle_session() {
        let root = TestDir::new();
        let cwd = root.path().join("checkout");
        std::fs::create_dir_all(&cwd).expect("checkout");
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex-deepseek".to_owned(),
            cwd.display().to_string(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.push(
            "s",
            crate::core::Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": "durable prompt"},
                }),
            },
        );
        hub.set_agent_session_id("s", "deepseek-thread-1".to_owned());
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = Supervisor::new_remote(hub.clone(), root.0.clone(), 0, runtime.clone());

        hub.set_status("s", Status::Busy, None);
        assert!(
            supervisor
                .set_deepseek_context_profile("s", serde_json::json!("830k"))
                .expect_err("busy session must reject a process-level config change")
                .contains("current turn")
        );
        assert_eq!(
            hub.config_preferences("s").unwrap()["deepseek_context"],
            "680k"
        );

        hub.set_status("s", Status::Running, None);
        supervisor
            .set_deepseek_context_profile("s", serde_json::json!("830k"))
            .expect("idle context change");

        assert_eq!(hub.status("s"), Some(Status::Starting));
        assert_eq!(
            hub.config_preferences("s").unwrap()["deepseek_context"],
            "830k"
        );
        assert!(runtime.pending_for_test().iter().any(|command| {
            matches!(
                command,
                CoreCommand::EnsureSession { session }
                    if session.agent_session_id.as_deref() == Some("deepseek-thread-1")
                        && session.context_window == Some(830_000)
                        && session.auto_compact_token_limit == Some(788_500)
            )
        }));
        assert!(runtime.pending_for_test().iter().any(|command| {
            matches!(command, CoreCommand::StopSession { command_id, .. } if command_id.starts_with("reset-"))
        }));
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
        std::fs::create_dir_all(checkout.join(".git")).expect("checkout");
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex".to_owned(),
            checkout.display().to_string(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.set_status("s", Status::Busy, None);
        let supervisor = Supervisor::new(hub.clone(), root.0.clone(), 0, RuntimeRouter::new());

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
