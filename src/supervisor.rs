//! Session supervisor (design §7).
//!
//! Owns each agent's lifetime, decoupled from any client connection. Because
//! the `agent-client-protocol` connection is `!Send` (single-threaded,
//! spawn-local), each session runs on its **own OS thread** with a
//! current-thread tokio runtime + `LocalSet` (see [`crate::acp::run_agent`]).
//! The supervisor talks to that thread only through a `Send` command channel.

#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

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
    pub execution_binding: Option<&'a crate::execution_environment::ExecutionBinding>,
}

/// Exact immutable Agent Plugin release and Service-auth projection selected by
/// the Controller before a session is registered.
#[derive(Clone, Copy)]
pub struct ProviderGeneration<'a> {
    pub account_id: Option<&'a crate::provider_account::ProviderAccountId>,
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
    runtime_restrictions: Arc<crate::project_placement::RuntimeRestrictions>,
    /// Told the id of every dormant session a prompt woke, so the server can
    /// return its Device to capacity. Opening a session no longer wakes it.
    wake_listener: parking_lot::Mutex<Option<WakeListener>>,
    wake_admission: parking_lot::Mutex<Option<WakeAdmission>>,
    recovery_admission: parking_lot::Mutex<Option<WakeAdmission>>,
}

/// Receives the id of a dormant session that a prompt is waking.
pub type WakeListener = Arc<dyn Fn(&str) + Send + Sync>;
/// A cold prompt holds its admission lease until Starting has been published.
pub type WakeAdmission = Arc<dyn Fn(&str) -> Result<Box<dyn Send>, String> + Send + Sync>;

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
    #[cfg(any(test, feature = "test-support"))]
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
            runtime_restrictions: Arc::default(),
            wake_listener: parking_lot::Mutex::new(None),
            wake_admission: parking_lot::Mutex::new(None),
            recovery_admission: parking_lot::Mutex::new(None),
        }
    }

    pub fn set_wake_listener(&self, listener: WakeListener) {
        *self.wake_listener.lock() = Some(listener);
    }

    pub fn set_wake_admission(&self, admission: WakeAdmission) {
        *self.wake_admission.lock() = Some(admission);
    }

    pub fn set_recovery_admission(&self, admission: WakeAdmission) {
        *self.recovery_admission.lock() = Some(admission);
    }

    /// The isolated workspace is ready, but no AI process is needed until use.
    pub fn finish_session_preparation(
        &self,
        session_id: &str,
        complete: impl FnOnce(&Hub) -> Result<(), String>,
    ) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        let runtime = self.runtime_for_session(session_id)?;
        if runtime.has_worker(session_id) {
            return Err("session already has a worker during preparation".into());
        }
        // Publish the prepared path/binding and dormant state under the same
        // fence as open/send. No observer can launch between these transitions.
        complete(&self.hub)?;
        self.hub.set_status(
            session_id,
            Status::Exited,
            Some("Ready; starts when a message is sent".into()),
        );
        // Messages submitted while preparation was in progress already belong
        // to the durable queue. Empty, paused and edited queues stay dormant.
        self.hub.retry_resource_wait(session_id);
        Ok(())
    }

    /// A dormant session holds no worker and has cleanly exited: it is
    /// resumable, and only a prompt (or an explicit reload) launches it.
    fn is_dormant(&self, runtime: &RemoteRuntime, session_id: &str) -> bool {
        !runtime.has_worker(session_id) && self.hub.status(session_id) == Some(Status::Exited)
    }

    /// The Controller re-runs Machine workspace preparation before dispatch;
    /// never fall through to a worker on an unprepared source checkout.
    fn refuse_unprepared_workspace(&self, session_id: &str) -> Result<(), String> {
        if self
            .hub
            .session_info(session_id)
            .is_some_and(|info| info.meta.awaits_workspace_preparation())
            && !self.has_worker(session_id)
        {
            return Err("Waiting for resources: preparing the workspace".to_owned());
        }
        Ok(())
    }

    fn has_worker(&self, session_id: &str) -> bool {
        self.runtime_for_session(session_id)
            .is_ok_and(|runtime| runtime.has_worker(session_id))
    }

    /// Editing preferences is not a request to start or recover a process.
    /// This also covers workspace preparation and stopped/offline Machines.
    fn defer_configuration(&self, session_id: &str) -> bool {
        matches!(
            self.hub.status(session_id),
            Some(Status::Exited | Status::Crashed | Status::Interrupted)
        ) || (self.hub.status(session_id) == Some(Status::Starting)
            && self
                .runtime_for_session(session_id)
                .is_ok_and(|runtime| !runtime.has_worker(session_id)))
    }

    #[must_use]
    pub fn with_runtime_restrictions(
        mut self,
        restrictions: Arc<crate::project_placement::RuntimeRestrictions>,
    ) -> Self {
        self.runtime_restrictions = restrictions;
        self
    }

    pub fn runtime_allowed(&self, provider: &str, machine: &str) -> bool {
        self.runtime_restrictions.allows(provider, machine)
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
        // This synchronous compatibility path cannot lock and validate a
        // durable native login binding. Named callers must prepare metadata
        // and use the account-aware Store registration before publication.
        if provider_generation
            .account_id
            .is_some_and(|account| !account.is_default())
        {
            return Err("named_account_requires_verified_registration".into());
        }
        let registration = self.prepare_session_on_with_id(
            id,
            provider,
            cwd,
            origin,
            system,
            placement,
            provider_generation,
            owner,
        )?;
        self.hub.create_session(registration);
        Ok(id.to_owned())
    }

    /// Validate and construct registration without publishing or starting it.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_session_on_with_id(
        &self,
        id: &str,
        provider: &str,
        cwd: Option<String>,
        origin: SessionOrigin,
        system: bool,
        placement: SessionPlacement<'_>,
        provider_generation: ProviderGeneration<'_>,
        owner: Option<SessionOwner<'_>>,
    ) -> Result<SessionRegistration, String> {
        let SessionPlacement {
            machine_id,
            workspace,
            execution_binding,
        } = placement;
        self.runtime_restrictions.check(provider, machine_id)?;
        let ProviderGeneration {
            account_id,
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
        Ok(SessionRegistration {
            id: id.to_owned(),
            provider: provider.to_owned(),
            provider_version: version.to_owned(),
            provider_generation_digest: digest.to_owned(),
            provider_auth_generation: auth_generation,
            provider_account_id: account_id.cloned().unwrap_or_default(),
            provider_behavior: behavior.cloned(),
            machine_id: machine_id.to_owned(),
            workspace_id: workspace.map(|value| value.id.clone()),
            workspace_name: workspace.map(|value| value.display_name.clone()),
            workspace_source_path: workspace.map(|value| value.canonical_path.clone()),
            execution_binding: execution_binding.cloned(),
            cwd: cwd.display().to_string(),
            title,
            origin,
            system,
            owner_user_id: owner.map(|owner| owner.user_id.to_owned()),
            owner_username: owner.and_then(|owner| owner.username.map(str::to_owned)),
        })
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
        meta.require_runtime_launch()?;
        self.runtime_restrictions
            .check(&meta.provider, &meta.machine_id)?;
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
            // A managed-call continuation restarts its hibernated child here;
            // it must resume the retained native thread, not open a blank one.
            let resume = self.hub.agent_session_id_for_resume(session_id);
            self.ensure_worker(session_id, &spec, std::path::Path::new(&meta.cwd), resume)
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
                execution_binding: None,
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
        meta.require_runtime_launch()?;
        self.runtime_restrictions
            .check(&meta.provider, &meta.machine_id)?;
        let configuration = session_configuration(&meta);
        let budget = self.managed_context_budget(session_id, &configuration);
        let cache_protection = self.managed_cache_protection(session_id, &configuration);
        runtime.ensure(StartSession {
            session_id: session_id.to_owned(),
            provider: spec.id.clone(),
            provider_version: meta.provider_version,
            provider_generation_digest: meta.provider_generation_digest,
            provider_account_id: meta.provider_account_id.clone(),
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
            execution_binding: meta.execution_binding,
            extensions: None,
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
    /// stayed empty). We lazily spawn an agent and resume the retained native
    /// thread through `session/resume` or `session/load`. Failed restoration
    /// never falls back to a blank session. Persisted configuration is restored
    /// before the first queued prompt reaches that thread.
    ///
    /// [`Outbound::Error`]: crate::core::Outbound::Error
    ///
    /// # Errors
    /// If the session is unknown to the Hub, its provider is no longer
    /// registered, or the agent thread cannot be spawned.
    pub fn send(&self, session_id: &str, command: AgentCommand) -> Result<(), String> {
        self.send_traced(session_id, command, None)
    }

    pub fn start_steer(
        &self,
        session_id: &str,
        content: Vec<serde_json::Value>,
        cmid: String,
        target: Option<&crate::send_delivery::DeliveryTarget>,
        next_checkpoint: bool,
    ) -> Result<crate::remote_runtime::SteeringDelivery, String> {
        if self.hub.is_managed_child(session_id) {
            return Err("managed child conversations do not accept steering".into());
        }
        // Do not prepare, wake or replace a worker for this operation.
        self.runtime_for_session(session_id)?.start_steer_targeted(
            session_id,
            content,
            cmid,
            target,
            next_checkpoint,
        )
    }

    pub fn start_cancel_turn(
        &self,
        session_id: &str,
        target: &crate::send_delivery::DeliveryTarget,
        cmid: String,
    ) -> Result<crate::remote_runtime::SteeringDelivery, String> {
        if self.hub.is_managed_child(session_id) {
            return Err("managed child is read-only".into());
        }
        self.runtime_for_session(session_id)?
            .start_cancel_turn(session_id, target, cmid)
    }

    pub fn delivery_turn_finished(
        &self,
        session_id: &str,
        target: &crate::send_delivery::DeliveryTarget,
    ) -> Result<bool, String> {
        self.runtime_for_session(session_id)?
            .delivery_turn_finished(session_id, target)
    }

    pub fn delivery_controls(&self, session_id: &str) -> crate::runtime_wire::DeliveryControls {
        self.runtime_for_session(session_id)
            .map(|runtime| runtime.delivery_controls(session_id))
            .unwrap_or_default()
    }

    pub fn delivery_target(
        &self,
        session_id: &str,
    ) -> Option<crate::send_delivery::DeliveryTarget> {
        if self.hub.is_managed_child(session_id) {
            return None;
        }
        self.runtime_for_session(session_id)
            .ok()?
            .delivery_target(session_id)
    }

    pub fn send_traced(
        &self,
        session_id: &str,
        command: AgentCommand,
        trace: Option<crate::runtime_trace::TraceCarrier>,
    ) -> Result<(), String> {
        self.send_traced_after_execution_recovery(session_id, command, trace, None)
    }

    fn delivery_admission(
        &self,
        session_id: &str,
        command: &AgentCommand,
        recovering: bool,
    ) -> Result<Option<Box<dyn Send>>, String> {
        let admission = if recovering {
            self.recovery_admission.lock().clone()
        } else if matches!(command, AgentCommand::Prompt(..))
            && !self.runtime_for_session(session_id)?.has_worker(session_id)
        {
            self.wake_admission.lock().clone()
        } else {
            None
        };
        admission.map(|admit| admit(session_id)).transpose()
    }

    pub fn send_traced_after_execution_recovery(
        &self,
        session_id: &str,
        command: AgentCommand,
        trace: Option<crate::runtime_trace::TraceCarrier>,
        recovery: Option<crate::execution_environment::RecoveryV1>,
    ) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        if recovery.is_some() && !matches!(command, AgentCommand::Prompt(..)) {
            return Err("execution recovery requires prompt delivery".into());
        }
        if matches!(
            command,
            AgentCommand::Steer { .. } | AgentCommand::CancelTurn { .. }
        ) {
            return Err("current-turn delivery requires durable asynchronous admission".into());
        }
        if matches!(command, AgentCommand::Cancel) {
            // Stopping a retained turn must never require starting or repairing
            // a worker, including after host policy has denied its placement.
            self.runtime_for_session(session_id)?.cancel(session_id);
            return Ok(());
        }
        if matches!(command, AgentCommand::SetConfigOption { .. })
            && self.defer_configuration(session_id)
        {
            // The caller already saved the preference. Do this before cwd
            // repair or runtime admission: neither belongs to a settings edit.
            return Ok(());
        }
        // A verified target replacement changes the failed restore's execution
        // conditions. Unchanged targets retain the explicit Reload boundary.
        if matches!(
            command,
            AgentCommand::Prompt(..) | AgentCommand::SetConfigOption { .. }
        ) && recovery.is_none()
            && self.should_hold_native_restore_failure(session_id)
        {
            tracing::info!(
                session = session_id,
                "refusing prompt until Reload after native ACP restore timeout"
            );
            return Err(crate::provider_behavior::NATIVE_RESTORE_TIMEOUT_HOLD.to_owned());
        }
        if let AgentCommand::Prompt(_, cmid, _) = &command
            && !cmid
                .as_deref()
                .is_some_and(|cmid| cmid.starts_with(crate::core::MANAGED_CALL_CMID_PREFIX))
            && self.hub.is_managed_child(session_id)
        {
            // Only the parent's managed call controls a child conversation.
            return Err(
                "this child conversation is controlled by its parent's managed call".into(),
            );
        }
        if matches!(command, AgentCommand::Prompt(..))
            && self
                .runtime_for_session(session_id)?
                .hibernation_pending(session_id)
        {
            return Err("Waiting for resources: session is finishing hibernation".into());
        }
        let _admission = self.delivery_admission(session_id, &command, recovery.is_some())?;
        if let Some(intent) = recovery {
            self.recover_execution_session_inner(session_id, intent)?;
        }
        self.prepare_session_inner(session_id)?;
        self.refuse_unprepared_workspace(session_id)?;
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
                let content = crate::session_context::prompt_content(
                    &self.hub,
                    session_id,
                    cmid.as_deref(),
                    blocks,
                )?;
                let session = self.start_session(session_id)?;
                if self.is_dormant(&runtime, session_id) {
                    // Project Starting at once, as an open used to, so the
                    // client shows the wake for the whole launch.
                    runtime.revive(session);
                    let listener = self.wake_listener.lock().clone();
                    if let Some(listener) = listener {
                        listener(session_id);
                    }
                } else {
                    runtime.ensure(session);
                }
                runtime.prompt_traced(session_id, content, cmid, trace);
            }
            AgentCommand::Steer { .. } | AgentCommand::CancelTurn { .. } => {
                unreachable!("steering has separate admission")
            }
            AgentCommand::Cancel => runtime.cancel(session_id),
            AgentCommand::Permission {
                request_id,
                option_id,
            } => runtime.permission(session_id, request_id, option_id),
            AgentCommand::SetConfigOption { config_id, value } => {
                // The caller saved the value as a session preference, and every
                // launch replays preferences before its first prompt. A dormant
                // session therefore keeps sleeping: the change costs its Device
                // nothing until the next prompt wakes it.
                if self.is_dormant(&runtime, session_id) {
                    return Ok(());
                }
                // A live (or crashed) session gets the value now; ensuring first
                // coalesces it with the persisted preference replay.
                runtime.ensure(self.start_session(session_id)?);
                runtime.set_config_option(session_id, &config_id, value);
            }
        }
        Ok(())
    }

    /// Reconcile a session the client opened without waking it when it is
    /// dormant: its transcript lives in the Controller, so reading history
    /// needs no worker, and a hibernated session costs its Device nothing
    /// until a prompt arrives. A live or disagreeing owner is still
    /// reconciled exactly as [`Self::ensure_alive`] does.
    ///
    /// Returns `true` if it launched a worker.
    ///
    /// # Errors
    /// If the session is unknown or its Machine runtime is disconnected.
    pub fn open(&self, session_id: &str) -> Result<bool, String> {
        let _lifecycle = self.lifecycle.lock();
        if matches!(
            self.hub.status(session_id),
            Some(Status::Exited | Status::Crashed | Status::Interrupted)
        ) {
            return Ok(false);
        }
        self.ensure_alive_inner(session_id)
    }

    /// Ensure a session has a live detached worker without sending a turn.
    ///
    /// Returns `true` if it revived, `false` if the agent was already alive.
    ///
    /// # Errors
    /// If the session is unknown or its Machine runtime is disconnected.
    #[cfg(test)]
    fn ensure_alive(&self, session_id: &str) -> Result<bool, String> {
        let _lifecycle = self.lifecycle.lock();
        self.ensure_alive_inner(session_id)
    }

    fn ensure_alive_inner(&self, session_id: &str) -> Result<bool, String> {
        refuse_trashed(&self.hub, session_id)?;
        // Native restore timeouts are session-local hydrate failures. Open,
        // reconnect, send, and Retry must not recycle the worker and pay the
        // same unbounded resume again. Reload remains the explicit retry.
        if self.should_hold_native_restore_failure(session_id) {
            tracing::info!(
                session = session_id,
                "not auto-reviving session after native ACP restore timeout"
            );
            return Ok(false);
        }
        // A split-runtime session holds a durable preparation intent until its
        // execution environment is accepted. Preparation recovery owns that
        // first launch; the intent can never decode as a runnable binding.
        if self.hub.session_list().into_iter().any(|meta| {
            meta.id == session_id
                && meta
                    .execution_binding
                    .as_ref()
                    .is_some_and(|binding| binding.preparation().is_some())
        }) {
            return Ok(false);
        }
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
        if meta.awaits_workspace_preparation() && !self.has_worker(session_id) {
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
        let session = self.start_session(session_id)?;
        if !has_worker && meta.status == Status::Exited {
            runtime.revive(session);
        } else {
            runtime.ensure(session);
        }
        Ok(!has_worker)
    }

    /// Build the idempotent worker launch declaration for a persisted session.
    fn start_session(&self, session_id: &str) -> Result<StartSession, String> {
        // Every launch, revive, reset, reload and Provider upgrade builds its
        // declaration here, so this is the one boundary Trash must hold.
        refuse_trashed(&self.hub, session_id)?;
        self.session_declaration(session_id, None)
    }

    /// Internal migration declaration only; never activates a worker. Fresh
    /// account admission remains the Controller/transport caller's obligation.
    pub fn readmission_session(
        &self,
        session_id: &str,
        target: &crate::provider_account::NativeProfileReceipt,
    ) -> Result<StartSession, String> {
        let mut declaration = self.session_declaration(session_id, Some(target))?;
        declaration.adopt_only = true;
        declaration.agent_session_id = None;
        Ok(declaration)
    }

    fn session_declaration(
        &self,
        session_id: &str,
        readmission: Option<&crate::provider_account::NativeProfileReceipt>,
    ) -> Result<StartSession, String> {
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        if let Some(target) = readmission {
            meta.require_native_readmission(target)?;
        } else {
            meta.require_runtime_launch()?;
        }
        self.runtime_restrictions
            .check(&meta.provider, &meta.machine_id)?;
        let configuration = session_configuration(&meta);
        let budget = self.managed_context_budget(session_id, &configuration);
        let cache_protection = self.managed_cache_protection(session_id, &configuration);
        let extensions = Some(crate::controller_hooks::launch_extensions(&self.hub, &meta));
        Ok(StartSession {
            session_id: meta.id,
            provider: meta.provider,
            provider_version: meta.provider_version,
            provider_generation_digest: meta.provider_generation_digest,
            provider_account_id: meta.provider_account_id.clone(),
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
            execution_binding: meta.execution_binding,
            extensions,
        })
    }

    fn managed_context_budget(
        &self,
        session_id: &str,
        configuration: &cowboy_provider_sdk::ConfigurationBehavior,
    ) -> Option<crate::deepseek_context::ContextBudget> {
        let preferences = self.hub.config_preferences(session_id)?;
        crate::managed_config::context_budget(configuration, &preferences)
    }

    fn managed_cache_protection(
        &self,
        session_id: &str,
        configuration: &cowboy_provider_sdk::ConfigurationBehavior,
    ) -> Option<bool> {
        let preferences = self.hub.config_preferences(session_id)?;
        crate::managed_config::cache_protection(configuration, &preferences)
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

    /// Snapshot only. A destructive coordinator must record its intent before
    /// asking the runtime to stop; this does not grant permission to restart it.
    pub fn has_live_worker(&self, session_id: &str) -> bool {
        self.runtime_for_session(session_id)
            .is_ok_and(|runtime| runtime.has_worker(session_id))
    }

    pub fn observed_configuration(&self, session_id: &str) -> Option<serde_json::Value> {
        self.runtime_for_session(session_id)
            .ok()?
            .observed_configuration(session_id)
    }

    /// Release an idle session's worker to free its Machine memory and slot.
    /// The session, transcript, worktree and native id remain; opening or
    /// prompting it later resumes through the ordinary revive path. The Machine
    /// re-checks idleness atomically and refuses if work started meanwhile.
    ///
    /// # Errors
    /// If the session is unknown, has no live worker, or has unfinished work.
    pub fn hibernate_session(&self, session_id: &str) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        let runtime = self.runtime_for_session(session_id)?;
        if !runtime.has_worker(session_id) {
            return Err("session is already hibernated".to_owned());
        }
        if meta.status != Status::Running || self.hub.session_has_in_flight_prompt(session_id) {
            return Err("wait for the current turn to finish before hibernating".to_owned());
        }
        if meta.background_tasks > 0 {
            return Err("session has background work in progress".to_owned());
        }
        runtime.hibernate(session_id);
        Ok(())
    }

    /// Replace a session's agent with a fresh context without deleting the
    /// Cowboy session. The remote broker needs an explicit reset operation so
    /// its permanent-delete tombstone cannot poison the replacement launch.
    pub fn reset_session(&self, session_id: &str) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        refuse_trashed(&self.hub, session_id)?;
        // A dormant session has no worker to replace. Its native id is already
        // forgotten, so the next prompt's wake starts a fresh context.
        if self.hub.status(session_id) == Some(Status::Exited)
            && !self
                .runtime_for_session(session_id)
                .is_ok_and(|runtime| runtime.has_worker(session_id))
        {
            return Ok(());
        }
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
        refuse_trashed(&self.hub, session_id)?;
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        self.runtime_restrictions
            .check(&meta.provider, &meta.machine_id)?;
        if meta.awaits_workspace_preparation() && !self.has_worker(session_id) {
            return Err(if meta.status == Status::Starting {
                "session workspace is still being prepared; reload it after preparation finishes"
            } else {
                "session workspace was never prepared; send a message or Retry to prepare it"
            }
            .to_owned());
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

    pub fn recover_execution_session(
        &self,
        session_id: &str,
        intent: crate::execution_environment::RecoveryV1,
    ) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        let _admission = self
            .recovery_admission
            .lock()
            .clone()
            .map(|admit| admit(session_id))
            .transpose()?;
        self.recover_execution_session_inner(session_id, intent)
    }

    fn recover_execution_session_inner(
        &self,
        session_id: &str,
        intent: crate::execution_environment::RecoveryV1,
    ) -> Result<(), String> {
        let runtime = self.runtime_for_session(session_id)?;
        let session = self.start_session(session_id)?;
        if !session
            .execution_binding
            .as_ref()
            .and_then(|b| b.decode().ok())
            .is_some_and(|b| intent.accepts(&b))
        {
            return Err("execution recovery binding changed before worker replacement".into());
        }
        self.hub.set_status(session_id, Status::Starting, None);
        let binding = session
            .execution_binding
            .as_ref()
            .and_then(|b| b.decode().ok())
            .ok_or("missing execution binding")?;
        if runtime.worker_matches_execution(session_id, &binding) {
            runtime.retry_execution_recovery(session);
        } else {
            runtime.recover_execution(session, intent);
        }
        Ok(())
    }

    /// Request native cancellation before arming a replacement watchdog.
    /// Old Machine protocols have no non-destructive hard-stop-only command:
    /// ordinary `StopSession` permanently deletes the session and its worktree.
    /// A denied historical placement therefore keeps its state and requires
    /// Machine-owned process maintenance if native cancellation does not finish.
    pub fn request_cancel_for_recycle(&self, session_id: &str) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        self.runtime_for_session(session_id)?.cancel(session_id);
        self.start_session(session_id).map(|_| ()).map_err(|error| {
            format!(
                "native cancellation requested, but force-restart is unavailable: {error}. \
                 If cancellation does not finish, stop the worker on its original Machine \
                 without deleting the Session"
            )
        })
    }

    /// Fence and replace one session's agent while preserving its resumable ACP
    /// id. Used after a force-cancel grace period expires; no other worker or
    /// session is touched.
    pub fn recycle_session(&self, session_id: &str) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        self.resolve_and_persist_cwd(session_id)?;
        self.recycle_session_inner(session_id)
    }

    /// Explicitly adopt a trusted installed Provider release without changing
    /// native identity, credential home, workspace, or saved user preferences.
    /// Unlike a same-version reload, this operation never interrupts a turn.
    /// `automatic` marks an unattended idle update for client presentation.
    pub fn reload_session_provider(
        &self,
        expected: &crate::core::SessionMeta,
        generation: ProviderGeneration<'_>,
        automatic: bool,
        carry: Option<&crate::core::NamedReleaseCarry>,
    ) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        let runtime = self.runtime_for_session(&expected.id)?;
        if !runtime.connected() {
            return Err("session Machine is not connected".to_owned());
        }
        if runtime.hibernation_pending(&expected.id) {
            return Err("session is finishing hibernation".into());
        }
        let behavior = generation
            .behavior
            .ok_or_else(|| "missing signed Provider behavior".to_owned())?;
        let mut launch = self.start_session(&expected.id)?;
        self.hub.begin_provider_reload(
            expected,
            generation.version,
            generation.digest,
            behavior,
            automatic,
            carry,
        )?;
        generation.version.clone_into(&mut launch.provider_version);
        generation
            .digest
            .clone_into(&mut launch.provider_generation_digest);
        launch.provider_behavior = Some(behavior.clone());
        runtime.reset(launch);
        Ok(())
    }

    /// Apply a Cowboy-managed configuration effect selected by the session's
    /// signed Provider behavior. Managed settings change process startup rather
    /// than ACP state, so recycle only this idle worker while preserving both
    /// the Cowboy and native agent session ids. Returns `false` when the option
    /// is ordinary adapter-owned protocol data.
    pub fn set_managed_config_option(
        &self,
        session_id: &str,
        config_id: &str,
        value: serde_json::Value,
    ) -> Result<bool, String> {
        let _lifecycle = self.lifecycle.lock();
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        let configuration = session_configuration(&meta);
        let Some(effect) = crate::managed_config::effect_for_id(&configuration, config_id) else {
            return Ok(false);
        };
        let deferred = self.defer_configuration(session_id);
        if !deferred
            && (matches!(meta.status, Status::Busy | Status::Starting)
                || self.hub.session_has_in_flight_prompt(session_id))
        {
            return Err(
                "wait for the current turn to finish before changing this managed setting"
                    .to_owned(),
            );
        }
        let preferences = self
            .hub
            .config_preferences(session_id)
            .unwrap_or_else(|| serde_json::json!({}));
        let unchanged = crate::managed_config::change_is_unchanged(
            effect,
            &configuration,
            &preferences,
            &value,
        )?;
        self.hub
            .set_config_preference(session_id, config_id.to_owned(), value)?;
        // A dormant session's next launch reads the new preference.
        if unchanged || deferred {
            return Ok(true);
        }
        self.resolve_and_persist_cwd(session_id)?;
        self.recycle_session_inner(session_id)?;
        Ok(true)
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
    /// Split-runtime retries defer worker preparation to dispatch preflight,
    /// which must verify the target lifetime before launching any replacement.
    pub fn prepare_session(&self, session_id: &str) -> Result<bool, String> {
        let _lifecycle = self.lifecycle.lock();
        if self.should_hold_native_restore_failure(session_id) {
            return Err(crate::provider_behavior::NATIVE_RESTORE_TIMEOUT_HOLD.to_owned());
        }
        refuse_trashed(&self.hub, session_id)?;
        if self
            .hub
            .session_info(session_id)
            .is_some_and(|info| info.meta.execution_binding.is_some())
        {
            // Retry must enqueue before launching a split-runtime worker. The
            // dispatch preflight first asks the target whether its lifetime
            // survived; only target-proven loss permits a new incarnation.
            // Eagerly recycling here launches against the stale binding and
            // can crash before dispatch gets a chance to recover it.
            return Ok(false);
        }
        self.prepare_session_inner(session_id)
    }

    fn prepare_session_inner(&self, session_id: &str) -> Result<bool, String> {
        refuse_trashed(&self.hub, session_id)?;
        let migrated = self.resolve_and_persist_cwd(session_id)?;
        let session = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id);
        // A session whose workspace preparation failed or was interrupted
        // still points at the stable source checkout. Recycling would launch
        // a worker there, which the Machine refuses, and leave the session
        // permanently crashed. Workspace preparation owns its first launch.
        if session
            .as_ref()
            .is_some_and(crate::core::SessionMeta::awaits_workspace_preparation)
            && !self.has_worker(session_id)
        {
            return Ok(false);
        }
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
                        detail == crate::core::MODEL_REFUSAL_DETAIL
                            || crate::provider::keeps_worker_alive_for_behavior(&behavior, detail)
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
        meta.require_runtime_launch()?;
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

    fn should_hold_native_restore_failure(&self, session_id: &str) -> bool {
        let Some(meta) = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
        else {
            return false;
        };
        if meta.status != Status::Crashed {
            return false;
        }
        self.hub
            .latest_crash_detail(session_id)
            .as_deref()
            .is_some_and(crate::provider_behavior::is_native_session_restore_timeout)
    }

    fn recycle_session_inner(&self, session_id: &str) -> Result<(), String> {
        let runtime = self.runtime_for_session(session_id)?;
        let session = self.start_session(session_id)?;
        self.hub.set_status(session_id, Status::Starting, None);
        runtime.reset(session);
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

/// A trashed session never launches, resumes or replaces a worker: every
/// wake path (open, send, queue drain, schedule, wakeup, reload) passes here.
fn refuse_trashed(hub: &crate::core::Hub, session_id: &str) -> Result<(), String> {
    if hub.session_is_trashed(session_id) {
        return Err(crate::core::TRASHED_SESSION_REFUSAL.to_owned());
    }
    Ok(())
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
                provider_account_id: crate::provider_account::ProviderAccountId::default(),
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
                execution_binding: None,
                extensions: None,
            }),
            state: WorkerState::Crashed,
            agent_session_id: Some("codex-thread-1".to_owned()),
            native_thread_materialized: None,
            current_turn_id: None,
            supports_current_turn: false,
            delivery_controls: crate::runtime_wire::DeliveryControls::default(),
            last_runtime_seq: 1,
            pending_permissions: Vec::new(),
            config_options: None,
            context_used: None,
            context_size: None,
            pending_prompt_count: 0,
            drain_requested: false,
            exit_detail: None,
            background_tasks: None,
            incarnation: None,
        }
    }

    fn preparing_web_session(hub: &Hub, cwd: &str) {
        hub.create_session(SessionRegistration {
            id: "s".to_owned(),
            provider: "codex".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "hawk".to_owned(),
            workspace_id: Some("columbus".to_owned()),
            workspace_name: Some("columbus".to_owned()),
            workspace_source_path: Some(cwd.to_owned()),
            execution_binding: None,
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

    #[tokio::test]
    async fn migrated_native_declaration_keeps_placement_and_never_starts_a_worker() {
        let root = TestDir::new();
        let seed = Hub::new();
        preparing_web_session(&seed, &root.path().display().to_string());
        let mut original = seed.session_info("s").unwrap().meta;
        original.provider = "claude-code".into();
        original.provider_account_id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        original.owner_user_id = Some("owner".into());
        original.provider_version = "old".into();
        original.provider_generation_digest = "old-digest".into();
        original.provider_auth_generation = Some(1);
        original.agent_session_id = Some("old-thread".into());
        original.status = Status::Exited;
        let hub = Hub::new();
        assert!(hub.project_persisted_session(original.clone()));
        let target: crate::provider_account::NativeProfileReceipt = serde_json::from_value(serde_json::json!({
            "account_id":original.provider_account_id,"provider_id":"claude-code","request_id":"login","generation":2,
            "method":"native","provider_version":"new","generation_digest":"new-digest","auth_contract_fingerprint":"fixture",
            "identity":{"authority":"fixture","subject":"private","tenant":null}
        })).unwrap();
        let behavior = crate::provider::legacy_behavior("claude-code");
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        runtime.connect_for_test();
        let supervisor = remote_supervisor(hub.clone(), Arc::clone(&runtime), root.0.clone());
        assert!(supervisor.readmission_session("s", &target).is_err());
        hub.project_native_binding(&original, &target, &behavior)
            .unwrap();
        let declaration = supervisor.readmission_session("s", &target).unwrap();
        assert!(declaration.adopt_only);
        assert!(declaration.agent_session_id.is_none());
        assert_eq!(declaration.cwd, original.cwd);
        assert_eq!(declaration.execution_binding, original.execution_binding);
        assert_eq!(declaration.provider_behavior.as_ref(), Some(&behavior));
        assert_eq!(declaration.provider_auth_generation, Some(2));
        assert!(runtime.pending_for_test().is_empty());
        let restricted = remote_supervisor(hub.clone(), Arc::clone(&runtime), root.0.clone())
            .with_runtime_restrictions(Arc::new(
                crate::project_placement::RuntimeRestrictions::parse(&["claude-code=ovh".into()])
                    .unwrap(),
            ));
        assert!(restricted.readmission_session("s", &target).is_err());
        assert!(restricted.start_registered_session("s").is_err());
        let admission = crate::provider_account::PreparedBinding {
            operation: "move".into(),
            target,
            stopped: crate::provider_account::WorkerExitObservation {
                request: crate::provider_account::WorkerExitRequest {
                    session_id: "s".into(),
                    account_id: original.provider_account_id,
                    stop_command_id: Some("stop".into()),
                },
                launch_id: uuid::Uuid::new_v4(),
            },
        };
        let waiting = runtime
            .readmit_account_session(declaration, admission)
            .unwrap();
        assert!(
            matches!(runtime.pending_for_test().as_slice(), [CoreCommand::ReadmitAccountSession { session, .. }] if session.adopt_only)
        );
        drop(waiting);
        assert_eq!(hub.status("s"), Some(Status::Exited));
        assert!(!runtime.has_worker("s"));
        supervisor.start_registered_session("s").unwrap();
        assert!(runtime.pending_for_test().iter().any(
            |command| matches!(command, CoreCommand::EnsureSession { session }
                if !session.adopt_only && session.provider_auth_generation == Some(2)
                && session.agent_session_id.is_none() && session.cwd == original.cwd)
        ));
        // Queueing is not admission: this fixture has no Store or transport.
        assert!(!runtime.has_worker("s"));
    }

    #[tokio::test]
    async fn runtime_restriction_blocks_restored_launches_and_prompts_but_keeps_cancel() {
        let root = TestDir::new();
        let cwd = root.path().display().to_string();
        let hub = Hub::new();
        preparing_web_session(&hub, &cwd);
        hub.set_status("s", Status::Busy, None);
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub.clone(), Arc::clone(&runtime), root.0.clone())
            .with_runtime_restrictions(Arc::new(
                crate::project_placement::RuntimeRestrictions::parse(&["codex=ovh".into()])
                    .unwrap(),
            ));
        for result in [
            supervisor.start_registered_session("s"),
            supervisor.ensure_alive("s").map(|_| ()),
            supervisor.send("s", AgentCommand::Prompt(Vec::new(), None, None)),
            supervisor.ensure_worker(
                "s",
                &provider::remote_generation("codex").unwrap(),
                root.path(),
                None,
            ),
        ] {
            assert!(result.unwrap_err().contains("not permitted"));
        }
        assert!(runtime.pending_for_test().is_empty());
        supervisor.send("s", AgentCommand::Cancel).unwrap();
        assert!(
            runtime
                .pending_for_test()
                .iter()
                .all(|command| !matches!(command, CoreCommand::EnsureSession { .. }))
        );
        assert_eq!(hub.session_list().len(), 1);
    }

    #[tokio::test]
    async fn compatibility_registration_rejects_named_accounts_without_side_effects() {
        let root = TestDir::new();
        let hub = Hub::new();
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub.clone(), Arc::clone(&runtime), root.0.clone());
        let account: crate::provider_account::ProviderAccountId =
            "00000000-0000-0000-0000-000000000001".parse().unwrap();
        for eager in [false, true] {
            let generation = ProviderGeneration {
                account_id: Some(&account),
                version: "1.0.0",
                digest: "digest",
                auth_generation: Some(1),
                behavior: None,
            };
            let result = if eager {
                supervisor.new_session_on_with_id(
                    "refused",
                    "claude-code",
                    None,
                    SessionOrigin::Web,
                    false,
                    "hawk",
                    generation,
                    None,
                )
            } else {
                supervisor.register_session_on_with_id(
                    "refused",
                    "claude-code",
                    None,
                    SessionOrigin::Web,
                    false,
                    SessionPlacement {
                        machine_id: "hawk",
                        workspace: None,
                        execution_binding: None,
                    },
                    generation,
                    None,
                )
            };
            assert_eq!(
                result.unwrap_err(),
                "named_account_requires_verified_registration"
            );
            assert!(hub.session_list().is_empty());
            assert!(runtime.pending_for_test().is_empty());
        }
    }

    #[tokio::test]
    async fn runtime_restriction_refuses_new_registration_before_mutating_sessions() {
        let root = TestDir::new();
        let hub = Hub::new();
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub.clone(), runtime, root.0.clone())
            .with_runtime_restrictions(Arc::new(
                crate::project_placement::RuntimeRestrictions::parse(&["codex=ovh".into()])
                    .unwrap(),
            ));
        let result = supervisor.new_session_on_with_id(
            "refused",
            "codex",
            Some(root.path().display().to_string()),
            SessionOrigin::Web,
            false,
            "hawk",
            ProviderGeneration {
                account_id: None,
                version: "",
                digest: "",
                auth_generation: None,
                behavior: None,
            },
            None,
        );
        assert!(result.unwrap_err().contains("not permitted"));
        assert!(hub.session_list().is_empty());
    }

    #[tokio::test]
    async fn runtime_restriction_preserves_refused_reload_and_native_cancel_only() {
        let root = TestDir::new();
        let hub = Hub::new();
        preparing_web_session(&hub, &root.path().display().to_string());
        hub.set_status("s", Status::Busy, None);
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub.clone(), Arc::clone(&runtime), root.0.clone())
            .with_runtime_restrictions(Arc::new(
                crate::project_placement::RuntimeRestrictions::parse(&["codex=ovh".into()])
                    .unwrap(),
            ));
        let original = hub.status_revision("s");
        assert!(
            supervisor
                .reload_session("s", true)
                .unwrap_err()
                .contains("not permitted")
        );
        assert!(
            supervisor
                .reset_session("s")
                .unwrap_err()
                .contains("not permitted")
        );
        assert_eq!(hub.status_revision("s"), original);
        assert!(runtime.pending_for_test().is_empty());

        // A denied historical runtime still receives native Cancel. Never arm
        // a replacement watchdog or use permanent deletion as a hard stop.
        assert!(
            supervisor
                .request_cancel_for_recycle("s")
                .unwrap_err()
                .contains("native cancellation requested, but force-restart is unavailable")
        );
        assert!(matches!(runtime.pending_for_test().as_slice(),
            [CoreCommand::Cancel { session_id, .. }] if session_id == "s"));
        assert_eq!(hub.status_revision("s"), original);
        assert_eq!(hub.session_list().len(), 1);
    }

    fn dormant_codex_session(root: &TestDir) -> Hub {
        let cwd = root.path().join("workspace");
        std::fs::create_dir_all(&cwd).expect("workspace");
        let hub = Hub::new();
        hub.create_session(SessionRegistration {
            id: "s".to_owned(),
            provider: "codex".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "hawk".to_owned(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: cwd.display().to_string(),
            title: "test".to_owned(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
        hub.set_status("s", Status::Exited, None);
        hub
    }

    fn recovered_session_fixture(
        root: &TestDir,
    ) -> (Hub, crate::execution_environment::RecoveryV1) {
        let cwd = root.path().join("workspace");
        std::fs::create_dir_all(&cwd).unwrap();
        let mut previous = crate::execution_environment::fixture().decode().unwrap();
        previous.runtime.machine_id = "hawk".into();
        previous.runtime.cwd = cwd.display().to_string();
        previous.workspace.worktree_id = "s".into();
        let mut replacement = previous.clone();
        replacement.revision += 1;
        replacement.environment.id = "55555555555555555555555555555555".into();
        replacement.environment.incarnation = "44444444444444444444444444444444".into();
        let intent = crate::execution_environment::RecoveryV1 {
            schema: 1,
            phase: "recovering".into(),
            operation_id: "execution-wake-fixture-1".into(),
            previous,
        };
        let hub = Hub::new();
        hub.create_session(SessionRegistration {
            id: "s".into(),
            provider: "codex".into(),
            provider_version: "1.0.0".into(),
            provider_generation_digest: "fixture".into(),
            provider_auth_generation: None,
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_behavior: Some(serde_json::from_value(serde_json::json!({
                "schema_version": 1, "permission": "acp_config_full_access_v1",
                "session": "portable_v1", "turn_end": "portable_v1", "configuration": "acp_config_options_v1",
                "execution": {"interface": "jsonrpc_v1", "executor_digests": [intent.previous.environment.executor_digest.clone()]}
            })).unwrap()),
            machine_id: "hawk".into(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: Some(crate::execution_environment::ExecutionBinding::from_record(
                serde_json::to_value(replacement).unwrap(),
            )),
            cwd: cwd.display().to_string(),
            title: "test".into(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
        hub.push("s", crate::core::Event::Update {
            update: serde_json::json!({"sessionUpdate":"user_message_chunk", "content":{"type":"text", "text":"existing durable history"}}),
        });
        hub.set_agent_session_id("s", "native-original".into());
        hub.set_status("s", Status::Exited, None);
        (hub, intent)
    }

    #[tokio::test]
    async fn clear_after_target_recovery_launches_fresh_context_on_the_recovered_binding() {
        let root = TestDir::new();
        let (hub, intent) = recovered_session_fixture(&root);
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub.clone(), Arc::clone(&runtime), root.0.clone());
        hub.set_paused("s", true);
        hub.requeue_prompt("s", "retained".into(), vec![], Some("original".into()));

        hub.prepare_context_reset("s");
        hub.clear_transcript("s");
        hub.mark_context_cleared("s");
        supervisor
            .recover_execution_session("s", intent.clone())
            .unwrap();

        let pending = runtime.pending_for_test();
        let launches = pending
            .iter()
            .filter_map(|command| match command {
                CoreCommand::RecoverExecutionSession { session, .. } => Some(session),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(launches.len(), 1);
        assert!(launches[0].agent_session_id.is_none());
        assert!(
            intent.accepts(
                &launches[0]
                    .execution_binding
                    .as_ref()
                    .unwrap()
                    .decode()
                    .unwrap()
            )
        );
        assert!(!pending.iter().any(|command| matches!(
            command,
            CoreCommand::Prompt { .. } | CoreCommand::StopSession { .. }
        )));
        assert_eq!(hub.session_info("s").unwrap().queue_count, 1);
        assert!(hub.session_info("s").unwrap().meta.paused);
    }

    #[tokio::test]
    async fn retry_of_crashed_execution_session_waits_for_dispatch_preflight() {
        let root = TestDir::new();
        let (hub, _) = recovered_session_fixture(&root);
        hub.set_status(
            "s",
            Status::Crashed,
            Some("execution binding failed".into()),
        );
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub.clone(), Arc::clone(&runtime), root.0.clone());
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        hub.set_dispatch_tx(tx);

        // Exercise the RetryTurn ordering: preparation must leave the session
        // crashed and enqueue its retry without starting the stale worker.
        assert!(!supervisor.prepare_session("s").unwrap());
        assert_eq!(hub.status("s"), Some(Status::Crashed));
        assert!(runtime.pending_for_test().is_empty());
        hub.retry_turn("s");
        hub.retry_turn("s");
        let request = rx.try_recv().expect("retry reaches dispatch preflight");
        assert_eq!(request.text, "existing durable history");
        assert!(rx.try_recv().is_err(), "retry must not duplicate delivery");
        assert!(runtime.pending_for_test().is_empty());
        assert_eq!(
            hub.session_info("s")
                .unwrap()
                .meta
                .agent_session_id
                .as_deref(),
            Some("native-original")
        );
    }

    #[tokio::test]
    async fn recovered_prompt_uses_one_admission_before_any_worker_launch() {
        let root = TestDir::new();
        let (hub, intent) = recovered_session_fixture(&root);
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub.clone(), Arc::clone(&runtime), root.0.clone());
        supervisor.set_wake_admission(Arc::new(|_| {
            panic!("recovery must not pay a second wake admission")
        }));
        supervisor.set_recovery_admission(Arc::new(|_| Err("Waiting for resources: full".into())));
        let prompt = || AgentCommand::Prompt(Vec::new(), Some("new-prompt".into()), None);
        assert!(
            supervisor
                .send_traced_after_execution_recovery("s", prompt(), None, Some(intent.clone()))
                .unwrap_err()
                .contains("Waiting for resources")
        );
        assert!(runtime.pending_for_test().is_empty());
        assert_eq!(hub.status("s"), Some(Status::Exited));
        assert!(
            supervisor
                .recover_execution_session("s", intent.clone())
                .unwrap_err()
                .contains("Waiting for resources")
        );
        assert!(runtime.pending_for_test().is_empty());
        let admissions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = Arc::clone(&admissions);
        supervisor.set_recovery_admission(Arc::new(move |_| {
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Box::new(()))
        }));
        supervisor
            .send_traced_after_execution_recovery("s", prompt(), None, Some(intent))
            .unwrap();
        assert_eq!(admissions.load(std::sync::atomic::Ordering::SeqCst), 1);
        let pending = runtime.pending_for_test();
        assert_eq!(
            pending
                .iter()
                .filter(|c| matches!(c, CoreCommand::RecoverExecutionSession { .. }))
                .count(),
            1
        );
        assert_eq!(
            pending
                .iter()
                .filter(|c| matches!(c, CoreCommand::Prompt { .. }))
                .count(),
            1
        );
        assert!(
            !pending
                .iter()
                .any(|c| matches!(c, CoreCommand::EnsureSession { .. }))
        );
        assert_eq!(
            hub.agent_session_id_for_resume("s").as_deref(),
            Some("native-original")
        );
    }

    #[tokio::test]
    async fn registered_start_resumes_a_hibernated_native_thread() {
        let root = TestDir::new();
        let hub = dormant_codex_session(&root);
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub.clone(), Arc::clone(&runtime), root.0.clone());
        // A fresh child has no native rollout and must open a new thread.
        hub.set_agent_session_id("s", "native-unprompted".into());
        supervisor.start_registered_session("s").unwrap();
        assert!(runtime.pending_for_test().iter().any(|command| matches!(
            command,
            CoreCommand::EnsureSession { session } if session.agent_session_id.is_none()
        )));

        // A managed-call continuation restarts the same child after its first
        // turn hibernated it; the retained thread carries the earlier context.
        let root = TestDir::new();
        let hub = dormant_codex_session(&root);
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub.clone(), Arc::clone(&runtime), root.0.clone());
        hub.push("s", crate::core::Event::Update {
            update: serde_json::json!({"sessionUpdate":"user_message_chunk", "content":{"type":"text", "text":"generate"}}),
        });
        hub.set_agent_session_id("s", "native-original".into());
        supervisor.start_registered_session("s").unwrap();
        assert!(runtime.pending_for_test().iter().any(|command| matches!(
            command,
            CoreCommand::EnsureSession { session }
                if session.agent_session_id.as_deref() == Some("native-original")
        )));
    }

    #[tokio::test]
    async fn prepared_sessions_stay_workerless_and_resource_wait_precedes_every_launch() {
        let root = TestDir::new();
        let hub = dormant_codex_session(&root);
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub.clone(), Arc::clone(&runtime), root.0.clone());
        hub.set_status("s", Status::Starting, None);
        supervisor
            .finish_session_preparation("s", |_| Ok(()))
            .unwrap();
        assert_eq!(hub.status("s"), Some(Status::Exited));
        assert!(!supervisor.open("s").unwrap());
        assert!(runtime.pending_for_test().is_empty());
        supervisor.set_wake_admission(Arc::new(|_| Err("Waiting for resources: full".into())));
        let send = || AgentCommand::Prompt(Vec::new(), Some("once".into()), None);
        assert!(
            supervisor
                .send("s", send())
                .unwrap_err()
                .contains("Waiting for resources")
        );
        assert_eq!(hub.status("s"), Some(Status::Exited));
        assert!(
            runtime.pending_for_test().is_empty(),
            "refused wake must not allocate or send"
        );
        supervisor.set_wake_admission(Arc::new(|_| Ok(Box::new(()))));
        supervisor.send("s", send()).unwrap();
        assert_eq!(hub.status("s"), Some(Status::Starting));
        assert_eq!(
            runtime
                .pending_for_test()
                .iter()
                .filter(|command| matches!(command, CoreCommand::Prompt { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn preparation_completion_dispatches_messages_submitted_during_preparation() {
        let root = TestDir::new();
        let hub = dormant_codex_session(&root);
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub.clone(), Arc::clone(&runtime), root.0.clone());
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        hub.set_dispatch_tx(tx);
        hub.set_status("s", Status::Starting, None);
        hub.requeue_prompt("s", "while preparing".into(), vec![], Some("first".into()));
        supervisor
            .finish_session_preparation("s", |_| Ok(()))
            .unwrap();
        assert_eq!(rx.try_recv().unwrap().cmid.as_deref(), Some("first"));
        assert!(rx.try_recv().is_err());
        assert!(
            runtime.pending_for_test().is_empty(),
            "only the dispatcher may admit the launch"
        );
    }

    #[tokio::test]
    async fn opening_a_dormant_session_reads_history_without_launching_a_worker() {
        let root = TestDir::new();
        let hub = dormant_codex_session(&root);
        let router = RuntimeRouter::new();
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        router.install("hawk".to_owned(), Arc::clone(&runtime));
        let supervisor = Supervisor::new(hub.clone(), root.0.clone(), 0, router);
        let woken = Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
        let observed = Arc::clone(&woken);
        supervisor.set_wake_listener(Arc::new(move |id: &str| {
            observed.lock().push(id.to_owned());
        }));

        assert!(!supervisor.open("s").expect("open dormant"));
        assert!(
            runtime.pending_for_test().is_empty(),
            "opening launched a worker"
        );
        assert_eq!(hub.status("s"), Some(Status::Exited));
        assert!(woken.lock().is_empty());

        // The prompt is what wakes it, with Starting projected at once.
        supervisor
            .send(
                "s",
                AgentCommand::Prompt(Vec::new(), Some("c1".to_owned()), None),
            )
            .expect("prompt wakes the dormant session");
        assert!(runtime.pending_for_test().iter().any(|command| matches!(
            command,
            CoreCommand::EnsureSession { session } if session.session_id == "s"
        )));
        assert_eq!(hub.status("s"), Some(Status::Starting));
        assert_eq!(woken.lock().as_slice(), ["s".to_owned()]);
    }

    #[tokio::test]
    async fn settings_and_clear_on_a_dormant_session_do_not_wake_it() {
        let root = TestDir::new();
        let hub = dormant_codex_session(&root);
        let router = RuntimeRouter::new();
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        router.install("hawk".to_owned(), Arc::clone(&runtime));
        let supervisor = Supervisor::new(hub.clone(), root.0.clone(), 0, router);

        hub.set_config_preference("s", "model".to_owned(), serde_json::json!("gpt-test"))
            .expect("save preference");
        supervisor
            .send(
                "s",
                AgentCommand::SetConfigOption {
                    config_id: "model".to_owned(),
                    value: serde_json::json!("gpt-test"),
                },
            )
            .expect("config change on a dormant session");
        hub.prepare_context_reset("s");
        supervisor
            .reset_session("s")
            .expect("clear a dormant session");

        assert!(
            runtime.pending_for_test().is_empty(),
            "a setting or clear launched a worker"
        );
        assert_eq!(hub.status("s"), Some(Status::Exited));
        assert_eq!(
            hub.config_preferences("s").unwrap()["model"],
            serde_json::json!("gpt-test")
        );

        // The next prompt wakes it fresh and replays the preference first.
        supervisor
            .send(
                "s",
                AgentCommand::Prompt(Vec::new(), Some("c1".to_owned()), None),
            )
            .expect("prompt wakes the session");
        let pending = runtime.pending_for_test();
        assert!(pending.iter().any(|command| matches!(
            command,
            CoreCommand::EnsureSession { session }
                if session.session_id == "s" && session.agent_session_id.is_none()
        )));
    }

    #[tokio::test]
    async fn inspecting_a_crashed_session_preserves_the_failure_without_restarting() {
        let root = TestDir::new();
        let hub = dormant_codex_session(&root);
        hub.set_status("s", Status::Crashed, None);
        let router = RuntimeRouter::new();
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        router.install("hawk".to_owned(), Arc::clone(&runtime));
        let supervisor = Supervisor::new(hub.clone(), root.0.clone(), 0, router);
        supervisor.open("s").expect("open crashed");
        assert!(runtime.pending_for_test().is_empty());
        assert_eq!(hub.status("s"), Some(Status::Crashed));
    }

    #[tokio::test]
    async fn stopped_configuration_and_clear_work_without_a_connected_machine() {
        let root = TestDir::new();
        let hub = dormant_codex_session(&root);
        let supervisor = Supervisor::new(hub.clone(), root.0.clone(), 0, RuntimeRouter::new());
        for status in [Status::Exited, Status::Crashed, Status::Interrupted] {
            hub.set_status("s", status, None);
            assert!(!supervisor.open("s").unwrap());
            hub.set_config_preference("s", "model".into(), serde_json::json!("chosen"))
                .unwrap();
            supervisor
                .send(
                    "s",
                    AgentCommand::SetConfigOption {
                        config_id: "model".into(),
                        value: serde_json::json!("chosen"),
                    },
                )
                .unwrap();
            assert_eq!(hub.status("s"), Some(status));
        }
        hub.set_status("s", Status::Exited, None);
        hub.prepare_context_reset("s");
        supervisor.reset_session("s").unwrap();
        assert_eq!(hub.status("s"), Some(Status::Exited));
        assert_eq!(hub.config_preferences("s").unwrap()["model"], "chosen");
    }

    #[tokio::test]
    async fn send_queues_while_machine_runtime_handshake_is_still_in_flight() {
        let root = TestDir::new();
        let cwd = root.path().join("workspace");
        std::fs::create_dir_all(&cwd).expect("workspace");
        let hub = Hub::new();
        hub.create_session(SessionRegistration {
            id: "s".to_owned(),
            provider: "codex".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "hawk".to_owned(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: cwd.display().to_string(),
            title: "test".to_owned(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
        hub.set_status("s", Status::Exited, None);
        let router = RuntimeRouter::new();
        let supervisor = Supervisor::new(hub.clone(), root.0.clone(), 0, Arc::clone(&router));
        assert!(
            supervisor
                .ensure_alive("s")
                .unwrap_err()
                .contains("not connected")
        );

        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        router.install("hawk".to_owned(), Arc::clone(&runtime));
        assert!(supervisor.ensure_alive("s").expect("queued ensure"));
        assert!(runtime.pending_for_test().iter().any(|command| {
            matches!(
                command,
                CoreCommand::EnsureSession { session } if session.session_id == "s"
            )
        }));
        // Progress is visible before the Machine verifies and spawns.
        assert_eq!(hub.status("s"), Some(Status::Starting));
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
    async fn failed_workspace_preparation_never_launches_from_source_checkout() {
        // A Controller restart or Git timeout can end Machine preparation and
        // leave the session Crashed on the stable source root. Resume must not
        // recycle a worker there (the Machine refuses it as untrusted); the
        // Controller re-runs workspace preparation instead.
        let root = TestDir::new();
        let source = root.path().join("columbus");
        std::fs::create_dir_all(&source).expect("source");
        let hub = Hub::new();
        preparing_web_session(&hub, source.to_string_lossy().as_ref());
        hub.set_status(
            "s",
            Status::Crashed,
            Some("Machine could not prepare an isolated workspace: git timed out".into()),
        );
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub.clone(), runtime.clone(), root.0.clone());

        assert!(!supervisor.prepare_session("s").expect("retry preflight"));
        assert!(!supervisor.ensure_alive("s").expect("open"));
        assert!(
            supervisor
                .send("s", AgentCommand::Prompt(vec![], Some("c".into()), None))
                .is_err()
        );
        assert!(supervisor.reload_session("s", false).is_err());
        assert!(runtime.pending_for_test().is_empty());
        assert_eq!(hub.status("s"), Some(Status::Crashed));
    }

    #[tokio::test]
    async fn opening_split_runtime_session_waits_for_execution_preparation() {
        let root = TestDir::new();
        let hub = Hub::new();
        let intent = crate::execution_environment::PreparationV1 {
            schema: 1,
            phase: "preparing".into(),
            runtime: crate::execution_environment::RuntimeLocation {
                machine_id: "hawk".into(),
                cwd: "/runtime/s".into(),
            },
            machine_id: "ovh".into(),
            workspace_id: "columbus".into(),
            source_path: "/sources/columbus".into(),
            executor_digest: format!("sha256:{}", "ab".repeat(32)),
        };
        let binding = crate::execution_environment::ExecutionBinding::from_record(
            serde_json::to_value(&intent).unwrap(),
        );
        hub.create_session(SessionRegistration {
            id: "s".to_owned(),
            provider: "codex".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "hawk".to_owned(),
            workspace_id: Some("columbus".to_owned()),
            workspace_name: Some("columbus".to_owned()),
            workspace_source_path: Some("/sources/columbus".to_owned()),
            execution_binding: Some(binding),
            cwd: "/runtime/s".to_owned(),
            title: "test".to_owned(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = remote_supervisor(hub, runtime.clone(), root.0.clone());

        assert!(
            !supervisor
                .ensure_alive("s")
                .expect("open session awaiting its execution environment")
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
        let supervisor = remote_supervisor(hub.clone(), runtime.clone(), root.0.clone());

        assert!(
            !supervisor
                .ensure_alive("s")
                .expect("open divergent exited session")
        );
        // A live worker may be kept as-is; leave its status to the Machine.
        assert_eq!(hub.status("s"), Some(Status::Exited));

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
    async fn opening_a_resume_timeout_crash_does_not_auto_revive() {
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
        hub.set_agent_session_id("s", "codex-thread-1".to_owned());
        hub.set_status(
            "s",
            Status::Crashed,
            Some("agent did not complete ACP session/resume within 240s".to_owned()),
        );
        hub.set_status("s", Status::Crashed, None);
        let runtime = RemoteRuntime::for_test(
            hub.clone(),
            vec![worker_snapshot(cwd.to_string_lossy().as_ref())],
        );
        let supervisor = Supervisor::new_remote(hub.clone(), root.0.clone(), 0, runtime.clone());

        assert!(!supervisor.ensure_alive("s").expect("hold crashed restore"));
        assert!(!runtime.pending_for_test().iter().any(|command| {
            matches!(
                command,
                CoreCommand::EnsureSession { session } if session.session_id == "s"
            ) || matches!(command, CoreCommand::StopSession { .. })
        }));
        assert_eq!(hub.status("s"), Some(Status::Crashed));
        assert_eq!(
            hub.latest_crash_detail("s").as_deref(),
            Some("agent did not complete ACP session/resume within 240s")
        );
    }

    #[tokio::test]
    async fn sending_after_a_resume_timeout_crash_does_not_retry_hydrate() {
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
        hub.set_agent_session_id("s", "codex-thread-1".to_owned());
        hub.set_status(
            "s",
            Status::Crashed,
            Some("agent did not complete ACP session/resume within 240s".to_owned()),
        );
        let runtime = RemoteRuntime::for_test(
            hub.clone(),
            vec![worker_snapshot(cwd.to_string_lossy().as_ref())],
        );
        let supervisor = Supervisor::new_remote(hub.clone(), root.0.clone(), 0, runtime.clone());

        let error = supervisor
            .send(
                "s",
                AgentCommand::Prompt(
                    vec![agent_client_protocol::schema::v1::ContentBlock::from(
                        "retry me".to_owned(),
                    )],
                    None,
                    None,
                ),
            )
            .expect_err("send must refuse restore-timeout crash");
        assert!(error.contains("Reload"));
        assert!(supervisor.prepare_session("s").is_err());
        assert!(!runtime.pending_for_test().iter().any(|command| {
            matches!(
                command,
                CoreCommand::EnsureSession { session } if session.session_id == "s"
            ) || matches!(command, CoreCommand::StopSession { .. })
        }));
        assert_eq!(hub.status("s"), Some(Status::Crashed));

        supervisor
            .reload_session("s", false)
            .expect("Reload remains the explicit hydrate retry");
        assert!(runtime.pending_for_test().iter().any(|command| {
            matches!(
                command,
                CoreCommand::EnsureSession { session } if session.session_id == "s"
            )
        }));
        assert_eq!(hub.status("s"), Some(Status::Starting));
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
    async fn provider_reload_redeclares_only_the_target_and_keeps_native_home() {
        let root = TestDir::new();
        let hub = Hub::new();
        hub.create_session(crate::core::SessionRegistration {
            id: "s".to_owned(),
            provider: "codex".to_owned(),
            provider_version: "old".to_owned(),
            provider_generation_digest: "old-digest".to_owned(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: Some(7),
            provider_behavior: Some(provider::legacy_behavior("codex")),
            machine_id: "local".to_owned(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: root.path().display().to_string(),
            title: "test".to_owned(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
        hub.push("s", crate::core::Event::Update {
            update: serde_json::json!({"sessionUpdate": "user_message_chunk", "content": {"text": "keep"}}),
        });
        hub.set_agent_session_id("s", "native-thread".to_owned());
        hub.set_status("s", Status::Running, None);
        let before = hub.session_info("s").unwrap().meta;
        assert_eq!(before.provider_auth_generation, Some(7));
        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let supervisor = Supervisor::new_remote(hub.clone(), root.0.clone(), 0, runtime.clone());
        let behavior = provider::legacy_behavior("codex");
        let target = ProviderGeneration {
            account_id: None,
            version: "new",
            digest: "new-digest",
            auth_generation: Some(999),
            behavior: Some(&behavior),
        };
        assert!(
            supervisor
                .reload_session_provider(&before, target, false, None)
                .unwrap_err()
                .contains("not connected")
        );
        assert!(runtime.pending_for_test().is_empty());
        assert_eq!(hub.status("s"), Some(Status::Running));
        runtime.connect_for_test();
        supervisor
            .reload_session_provider(&before, target, false, None)
            .expect("reload");
        let pending = runtime.pending_for_test();
        assert!(pending.iter().any(
            |command| matches!(command, CoreCommand::EnsureSession { session }
            if session.session_id == "s" && session.provider_version == "new"
                && session.provider_generation_digest == "new-digest"
                && session.agent_session_id.as_deref() == Some("native-thread")
                && session.provider_auth_generation == before.provider_auth_generation
                && session.cwd == before.cwd && session.adopt_only)
        ));
        assert!(pending.iter().any(
            |command| matches!(command, CoreCommand::StopSession {session_id, command_id}
            if session_id == "s" && command_id.starts_with("reset-"))
        ));
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
    async fn turn_rejection_reuses_live_workers_without_resume() {
        let context_limit = "API Error: 400 This model's maximum context length is 1048576 tokens. However, you requested 1048875 tokens";
        for (provider, detail) in [
            ("claude-code", context_limit),
            ("claude-deepseek", context_limit),
            ("claude-code", crate::core::MODEL_REFUSAL_DETAIL),
            ("codex", crate::core::MODEL_REFUSAL_DETAIL),
        ] {
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
    async fn changing_a_managed_context_budget_recycles_only_the_idle_session() {
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
                .set_managed_config_option("s", "deepseek_context", serde_json::json!("830k"),)
                .expect_err("busy session must reject a process-level config change")
                .contains("current turn")
        );
        assert_eq!(
            hub.config_preferences("s").unwrap()["deepseek_context"],
            "680k"
        );

        hub.set_status("s", Status::Running, None);
        assert!(
            supervisor
                .set_managed_config_option("s", "deepseek_context", serde_json::json!("830k"),)
                .expect("idle context change")
        );

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
