//! Launch preparation for `Broker::spawn_worker`: the exact Provider
//! generation, placement checks, the session environment and the previous
//! direct owner. Spawning, PID ownership and reaping stay in the caller.

use super::*;

use crate::machine_plugins::ProviderLaunchContext;
use cowboy_provider_sdk::ProviderBehaviorContract;
use std::os::unix::fs::DirBuilderExt as _;

pub(super) fn should_recycle_for_explicit_revive(
    worker: &WorkerSnapshot,
    desired_generation: &str,
) -> bool {
    matches!(worker.state, WorkerState::Exited | WorkerState::Crashed)
        || (!desired_generation.is_empty()
            && worker.generation != desired_generation
            && worker.drain_requested
            && worker.current_turn_id.is_none()
            && worker.pending_permissions.is_empty()
            && matches!(worker.state, WorkerState::Running | WorkerState::Draining))
}

impl Broker {
    /// The registered worker for this generation. A retained bundle may predate
    /// MCP launch plans, so an MCP launch is refused before it starts a worker
    /// that would exit on the unknown argument.
    pub(super) async fn launch_worker_command(
        &self,
        session: &StartSession,
        provider: Option<&ProviderLaunchContext>,
    ) -> Result<PathBuf> {
        let worker = self
            .generation_commands
            .lock()
            .get(&session.generation)
            .cloned()
            .with_context(|| {
                format!(
                    "no worker executable registered for generation {}",
                    session.generation
                )
            })?;
        if session.mcp_plan.is_some() {
            super::mcp_launch::ensure_worker_accepts_plan(&worker).await?;
        }
        if requires_managed_resume_worker(
            session,
            provider.map(|context| &context.managed_continue_profiles),
        ) {
            super::mcp_launch::ensure_worker_supports_managed_resume(&worker).await?;
        }
        Ok(worker)
    }

    /// Optional worker arguments that pin the authenticated account and MCP plan.
    pub(super) fn push_launch_identity_args(
        &self,
        command: &mut Command,
        session: &StartSession,
    ) -> Result<()> {
        if let Some(auth_generation) = session.provider_auth_generation {
            command
                .arg("--provider-auth-generation")
                .arg(auth_generation.to_string());
        }
        if !session.provider_account_id.is_default() {
            command
                .arg("--provider-account-id")
                .arg(session.provider_account_id.as_str());
            if self.args.spawn_mode == SpawnMode::SystemdUser {
                command.arg("--launch-gate");
            }
        }
        if let Some(plan) = &session.mcp_plan {
            command.arg("--mcp-plan").arg(serde_json::to_string(plan)?);
        }
        Ok(())
    }

    /// Verify and resolve the exact Provider generation this launch runs.
    pub(super) async fn resolve_launch_provider(
        &self,
        session: &StartSession,
    ) -> Result<Option<ProviderLaunchContext>> {
        // Verification reads and hashes the retained runtime; keep that file
        // work off the broker's async workers so other sessions keep flowing.
        let verification_started = Instant::now();
        anyhow::ensure!(
            session.provider_account_id.is_default()
                || (!session.provider_generation_digest.is_empty()
                    && session.provider_auth_generation.is_some()),
            "Named account requires an exact authenticated Provider generation"
        );
        let provider = if session.provider_generation_digest.is_empty() {
            None
        } else {
            let store = Arc::new(
                self.args
                    .provider_store
                    .for_account(&session.provider_account_id)?,
            );
            let (provider_id, digest, auth_generation) = (
                session.provider.clone(),
                session.provider_generation_digest.clone(),
                session.provider_auth_generation,
            );
            // An Operator install fences the Plugin until it resolves (about a
            // minute for a Provider upgrade). A start in that window waits for
            // the install rather than reporting a crashed Agent; a fence with
            // no live installer still fails at once for reconciliation.
            let install_deadline = Instant::now() + PROVIDER_INSTALL_LAUNCH_WAIT;
            loop {
                let (launch, install_in_progress) = {
                    let (store, provider_id, digest) =
                        (Arc::clone(&store), provider_id.clone(), digest.clone());
                    tokio::task::spawn_blocking(move || {
                        let launch = store.launch_context(&provider_id, &digest, auth_generation);
                        let in_progress =
                            launch.is_err() && store.install_in_progress(&provider_id);
                        (launch, in_progress)
                    })
                    .await
                    .context("Provider launch verification task failed")?
                };
                match launch {
                    Ok(launch) => break Some(launch),
                    Err(error)
                        if install_in_progress
                            && Instant::now() < install_deadline
                            && !self.cancelled_sessions.lock().contains(&session.session_id) =>
                    {
                        tracing::info!(
                            session = %session.session_id,
                            provider = %provider_id,
                            %error,
                            "worker launch waiting for Provider installation"
                        );
                        tokio::time::sleep(PROVIDER_INSTALL_LAUNCH_POLL).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        };
        tracing::info!(
            session = %session.session_id,
            provider = %session.provider,
            verification_ms = verification_started.elapsed().as_millis(),
            "worker launch context prepared"
        );
        Ok(provider)
    }

    /// Resolve Provider behavior, validate launch placement, and build the
    /// session environment every launch mode passes to the worker.
    pub(super) async fn prepare_launch_environment(
        &self,
        session: &StartSession,
        provider: &Option<ProviderLaunchContext>,
    ) -> Result<Vec<(&'static str, String)>> {
        let behavior = provider.as_ref().map_or_else(
            || {
                session
                    .provider_behavior
                    .clone()
                    .unwrap_or_else(|| crate::provider_behavior::legacy_behavior(&session.provider))
            },
            |provider| provider.behavior.clone(),
        );
        validate_launch_placement(session, provider, &behavior)?;
        self.launch_session_environment(session, provider, &behavior)
            .await
    }

    /// The session environment every launch mode passes to the worker.
    pub(super) async fn launch_session_environment(
        &self,
        session: &StartSession,
        provider: &Option<ProviderLaunchContext>,
        behavior: &ProviderBehaviorContract,
    ) -> Result<Vec<(&'static str, String)>> {
        let mut session_environment =
            session_context_environment(session, &behavior.configuration)?;
        // Local parents run their tools in this worker's process tree. Remote
        // parents receive the context from their target keeper instead, and
        // managed children never receive one: delegation is not recursive.
        if session.execution_binding.is_none()
            && !session.system
            && let Some(path) = self.args.call_context_root.as_deref().and_then(|root| {
                crate::managed_calls::host::context_file(root, &session.session_id)
            })
        {
            session_environment.push((
                crate::execution_protocol::CALL_CONTEXT_ENV,
                path.display().to_string(),
            ));
        }
        if let Some(value) = self.launch_extensions_environment(session).await? {
            session_environment.push((crate::machine_plugins::SESSION_EXTENSIONS_ENV, value));
        }
        if let Some(provider) = &provider {
            session_environment.push((
                "COWBOY_PROVIDER_PACKAGE_PATH",
                provider.package_path.display().to_string(),
            ));
            session_environment.push(("COWBOY_PROVIDER_ENTRYPOINT", provider.command.clone()));
            if let Some(home) = &provider.home {
                session_environment.push(("HOME", home.display().to_string()));
            }
            if let Some(launch) = &provider.account_sandbox {
                session_environment.push((
                    crate::account_sandbox::LAUNCH_ENV,
                    serde_json::to_string(launch)?,
                ));
            }
            if session.execution_binding.is_some()
                && (provider.account_sandbox.is_some() || session.provider == "antigravity")
            {
                session_environment.push((
                    crate::account_sandbox::EXECUTION_LEDGER_ENV,
                    self.execution_ledger(&session.session_id)?
                        .display()
                        .to_string(),
                ));
            }
        }
        Ok(session_environment)
    }

    /// The durable ledger of a sandboxed session's execution binding. It
    /// lives under the Machine state root, keyed by Session, so the session's
    /// uncertain effect survives a dead worker or a reboot; processes are
    /// never inherited (design "Revision 5 simplification").
    fn execution_ledger(&self, session_id: &str) -> Result<PathBuf> {
        let root = self
            .args
            .call_context_root
            .as_deref()
            .context("a sandboxed execution binding needs the Machine state root")?;
        let path = crate::account_sandbox::execution_ledger_file(root, session_id)
            .context("invalid session id for an execution ledger")?;
        let directory = path.parent().context("execution ledger has no directory")?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
        Ok(path)
    }

    /// A Direct launch waits until the previous worker for this session has
    /// no live process owner.
    pub(super) async fn await_previous_direct_worker(&self, session: &StartSession) -> Result<()> {
        let workers = Arc::clone(&self.direct_worker_pids);
        let existing_pid = workers.lock().get(&session.session_id).copied();
        if let Some(existing_pid) = existing_pid {
            tracing::warn!(
                session = %session.session_id,
                pid = existing_pid,
                "waiting for previous direct worker owner before launch"
            );
            ensure!(
                enforce_direct_worker_exit(
                    Arc::clone(&workers),
                    session.session_id.clone(),
                    existing_pid,
                    DIRECT_WORKER_GRACEFUL_STOP_TIMEOUT,
                    DIRECT_WORKER_TERM_TIMEOUT,
                    DIRECT_WORKER_KILL_TIMEOUT,
                )
                .await,
                "previous direct worker {} pid {} did not exit before replacement",
                session.session_id,
                existing_pid
            );
        }
        ensure!(
            !workers.lock().contains_key(&session.session_id),
            "direct worker {} still has a live process owner",
            session.session_id
        );
        Ok(())
    }
}

/// The installed generation must match what the session declared, and a
/// managed child or execution binding must be launchable by it.
pub(super) fn validate_launch_placement(
    session: &StartSession,
    provider: &Option<ProviderLaunchContext>,
    behavior: &ProviderBehaviorContract,
) -> Result<()> {
    if let (Some(declared), Some(installed)) =
        (session.provider_behavior.as_ref(), provider.as_ref())
    {
        ensure!(
            declared == &installed.behavior,
            "session Provider behavior does not match the installed generation"
        );
    }
    super::mcp_launch::validate(session, provider.as_ref())?;
    if let Some(installed) = &provider {
        ensure!(
            session.provider_version == installed.version,
            "session Provider version does not match the installed generation"
        );
    }
    let managed_child = session
        .execution_binding
        .as_ref()
        .and_then(|binding| binding.managed_child());
    if let Some(child) = &managed_child {
        ensure!(
            child.session_id == session.session_id && child.cwd == session.cwd,
            "managed child placement mismatch"
        );
        let installed = provider
            .as_ref()
            .context("managed child requires an exact Provider generation")?;
        let bytes = std::fs::read(&installed.package_path)?;
        let package = cowboy_provider_sdk::ProviderPackage::from_bytes(&bytes)?;
        package
            .manifest
            .runtime
            .launch_arguments(Some(child.profile))?;
    }
    if let Some(binding) = &session.execution_binding
        && managed_child.is_none()
    {
        let binding = binding.decode().map_err(anyhow::Error::msg)?;
        if let Some(managed) = &binding.managed {
            // A remote managed child launches only its signed profile.
            ensure!(
                binding.workspace.worktree_id == session.session_id,
                "managed child placement mismatch"
            );
            let installed = provider
                .as_ref()
                .context("managed child requires an exact Provider generation")?;
            let bytes = std::fs::read(&installed.package_path)?;
            let package = cowboy_provider_sdk::ProviderPackage::from_bytes(&bytes)?;
            package
                .manifest
                .runtime
                .launch_arguments(Some(managed.profile))?;
        }
        ensure!(
            behavior
                .execution
                .as_ref()
                .is_some_and(|execution| execution.accepts(
                    binding.environment.protocol,
                    &binding.environment.executor_digest
                )),
            "Provider has not accepted this exact execution component"
        );
        ensure!(
            binding.runtime.cwd == session.cwd
                && provider
                    .as_ref()
                    .is_some_and(|provider| provider.execution_jsonrpc),
            "Provider generation cannot launch this execution binding"
        );
    }
    Ok(())
}

/// Best-effort cleanup of a deleted Antigravity session's local conversation
/// artifacts when no other live session on the same account references it.
pub(super) fn cleanup_deleted_antigravity_session(broker: &Broker, deleted: &StartSession) {
    if deleted.provider != "antigravity" || deleted.provider_account_id.is_default() {
        return;
    }
    let Some(native_id) = deleted.agent_session_id.as_deref() else {
        return;
    };
    let referenced_elsewhere = broker.sessions.lock().values().any(|other| {
        other.provider == "antigravity"
            && other.provider_account_id == deleted.provider_account_id
            && other.agent_session_id.as_deref() == Some(native_id)
    });
    if !referenced_elsewhere
        && let Ok(scoped) = broker
            .args
            .provider_store
            .for_account(&deleted.provider_account_id)
    {
        scoped.cleanup_antigravity_conversation(native_id);
    }
}

/// The `systemd-run` property enforcing a signed Provider network policy on
/// the whole worker unit, including the native agent and its MCP children.
pub(super) fn network_property(policy: cowboy_provider_sdk::RuntimeNetworkPolicy) -> &'static str {
    match policy {
        cowboy_provider_sdk::RuntimeNetworkPolicy::Ipv4OnlyV1 => {
            "--property=RestrictAddressFamilies=AF_INET AF_UNIX AF_NETLINK"
        }
    }
}

#[cfg(test)]
mod network_policy_tests {
    #[test]
    fn ipv4_only_policy_excludes_ipv6_sockets() {
        let property =
            super::network_property(cowboy_provider_sdk::RuntimeNetworkPolicy::Ipv4OnlyV1);
        assert!(property.contains("AF_INET "));
        assert!(!property.contains("AF_INET6"));
    }
}

/// Whether launching `session` requires a worker that binds `managed_session_id`
/// in the account-sandbox execution descriptor (`--check-capability managed-resume-v1`).
/// Only signed managed profiles declaring `--cowboy-managed-continue=v1` require it;
/// Codex and Claude managed children resume natively on retained workers.
pub(super) fn requires_managed_resume_worker(
    session: &StartSession,
    managed_continue_profiles: Option<&BTreeSet<cowboy_provider_sdk::ManagedRuntimeProfile>>,
) -> bool {
    if !session
        .agent_session_id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
    {
        return false;
    }
    let Some(profile) = session.execution_binding.as_ref().and_then(|binding| {
        binding
            .managed_child()
            .map(|child| child.profile)
            .or_else(|| {
                binding
                    .decode()
                    .ok()
                    .and_then(|decoded| decoded.managed.map(|managed| managed.profile))
            })
    }) else {
        return false;
    };
    managed_continue_profiles.is_some_and(|profiles| profiles.contains(&profile))
}

#[cfg(test)]
mod managed_resume_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    fn managed_session(provider: &str) -> StartSession {
        let mut session: StartSession = serde_json::from_value(serde_json::json!({
            "session_id": "child-1",
            "provider": provider,
            "cwd": "/work",
            "generation": "gen-1",
            "agent_session_id": "native-1",
        }))
        .unwrap();
        let record = serde_json::to_value(crate::execution_environment::ManagedChildV1 {
            schema: 1,
            phase: "managed_child".into(),
            session_id: "child-1".into(),
            parent_session_id: "parent-1".into(),
            machine_id: "hawk".into(),
            workspace_id: "w".into(),
            cwd: "/work".into(),
            profile: cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
        })
        .unwrap();
        session.execution_binding = Some(
            crate::execution_environment::ExecutionBinding::from_record(record),
        );
        session
    }

    #[tokio::test]
    async fn codex_and_claude_managed_continuations_remain_compatible_with_retained_workers() {
        let root = tempfile::tempdir().unwrap();
        let retained = root.path().join("retained-worker");
        std::fs::write(&retained, "#!/bin/sh\nexit 2\n").unwrap();
        std::fs::set_permissions(&retained, std::fs::Permissions::from_mode(0o755)).unwrap();

        let empty_profiles = BTreeSet::new();
        for provider in ["codex", "claude-code"] {
            let session = managed_session(provider);
            assert!(
                !requires_managed_resume_worker(&session, Some(&empty_profiles)),
                "{provider} must not require managed-resume-v1 probe on retained workers"
            );
        }

        let agy_session = managed_session("antigravity");
        let agy_profiles: BTreeSet<_> = [cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1]
            .into_iter()
            .collect();
        assert!(requires_managed_resume_worker(
            &agy_session,
            Some(&agy_profiles)
        ));
        assert!(
            super::super::mcp_launch::ensure_worker_supports_managed_resume(&retained)
                .await
                .is_err()
        );
    }
}
