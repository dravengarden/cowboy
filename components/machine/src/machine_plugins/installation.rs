//! The finite installation path inside `MachinePluginStore`. The existing
//! installer owns all capability kinds; this adds authority and evidence, not
//! another installer or a generic replay engine.

use super::*;
use crate::machine_protocol::plugin_install::{
    InstallLookup, InstallObservation, InstallOutcome, InstallPhase, InstallRejection, InstallStep,
    InstallTarget, InstallTargetObservation, InstallTargetQuery, InstallUnavailable,
    InstallUncertainty,
};
use operations::install_attempts::PendingAttempt;
use operations::lease::InstallationLease;

pub(super) struct LeaseCheck(Option<InstallationLease>);

impl LeaseCheck {
    pub(super) fn none() -> Self {
        Self(None)
    }

    pub(super) fn check(&self) -> Result<()> {
        if let Some(lease) = &self.0 {
            lease
                .check()
                .map_err(|_| anyhow::anyhow!("installation execution lease ended"))?;
        }
        Ok(())
    }
}

pub(super) enum InstallGuard<'a> {
    Legacy,
    Durable(InstallAdmission<'a>),
}

/// Private fields: another installer module cannot assemble a durable guard
/// by pairing an unrelated receipt and still-live lease.
pub(super) struct InstallAdmission<'a> {
    pending: Option<Box<PendingAttempt>>,
    lease: &'a InstallationLease,
}

impl InstallGuard<'_> {
    pub(super) fn validate_for(
        &self,
        store: &MachinePluginStore,
        desired: &DesiredPlugin,
    ) -> Result<()> {
        self.check()?;
        match self {
            Self::Legacy => {
                store.operations.install_attempts.ensure_legacy_allowed()?;
                store.operations.installations.ensure_writable()?;
                store
                    .operations
                    .ensure_unfenced(&desired.release.plugin_id)?;
            }
            Self::Durable(admission) => {
                ensure!(
                    admission
                        .pending
                        .as_ref()
                        .expect("install continuation present")
                        .receipt()
                        .step
                        .matches_envelope(desired),
                    "installation continuation targets another envelope"
                );
            }
        }
        Ok(())
    }
    pub(super) fn lease_check(&self) -> LeaseCheck {
        match self {
            Self::Legacy => LeaseCheck(None),
            Self::Durable(InstallAdmission { lease, .. }) => LeaseCheck(Some(lease.clone_lease())),
        }
    }

    /// Flush regular files and directory entries of the staged, signed runtime
    /// before publishing activation. Archive extraction alone does not fsync
    /// payload bytes. Never follow archive symlinks out of this generation.
    pub(super) async fn flush_staging(
        &self,
        content: &Path,
        lifecycle: &mut Option<LifecycleGuard>,
    ) -> Result<()> {
        if matches!(self, Self::Legacy) {
            return Ok(());
        }
        self.check()?;
        let content = content.to_owned();
        let lease_check = self.lease_check();
        MachinePluginStore::run_with_lifecycle(
            lifecycle,
            "staged installation flush task failed",
            move || {
                let mut pending = vec![content];
                let mut entries = 0_u64;
                while let Some(path) = pending.pop() {
                    lease_check.check()?;
                    entries += 1;
                    ensure!(
                        entries <= 1_000_000,
                        "staged installation exceeds flush limit"
                    );
                    let metadata = path.symlink_metadata()?;
                    if metadata.file_type().is_symlink() {
                        continue;
                    }
                    ensure!(
                        metadata.is_file() || metadata.is_dir(),
                        "invalid staged installation entry"
                    );
                    let file = OpenOptions::new()
                        .read(true)
                        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                        .open(&path)?;
                    if metadata.is_dir() {
                        for entry in fs::read_dir(&path)? {
                            ensure!(
                                pending.len() < 1_000_000,
                                "staged installation exceeds flush limit"
                            );
                            pending.push(entry?.path());
                        }
                    }
                    file.sync_all()?;
                }
                lease_check.check()
            },
        )
        .await?;
        self.check()
    }

    pub(super) fn check(&self) -> Result<()> {
        if let Self::Durable(InstallAdmission { lease, pending, .. }) = self {
            ensure!(
                lease.matches(
                    &pending
                        .as_ref()
                        .expect("install continuation present")
                        .receipt()
                        .step
                ),
                "installation continuation targets another request"
            );
            lease
                .check()
                .map_err(|_| anyhow::anyhow!("installation execution lease ended"))?;
        }
        Ok(())
    }

    pub(super) async fn bounded<T>(
        &self,
        future: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        self.check()?;
        let result = if let Self::Durable(InstallAdmission { lease, .. }) = self {
            let remaining = lease
                .remaining()
                .map_err(|_| anyhow::anyhow!("installation execution lease ended"))?;
            tokio::time::timeout(remaining, future)
                .await
                .context("installation execution deadline ended")?
        } else {
            future.await
        };
        self.check()?;
        result
    }

    pub(super) async fn phase(
        &mut self,
        store: &MachinePluginStore,
        phase: InstallPhase,
        lifecycle: &mut Option<LifecycleGuard>,
    ) -> Result<()> {
        self.check()?;
        if let Self::Durable(InstallAdmission { pending, lease, .. }) = self {
            let mut owned_pending = pending
                .take()
                .context("installation continuation already consumed")?;
            let store_for_adv = store.clone();
            let lease_check = LeaseCheck(Some(lease.clone_lease()));
            let (adv_res, returned_pending) = MachinePluginStore::run_with_lifecycle(
                lifecycle,
                "installation phase transition task failed",
                move || {
                    let result = lease_check.check().and_then(|()| {
                        store_for_adv
                            .operations
                            .install_attempts
                            .advance(&mut owned_pending, InstallOutcome::Pending { phase })?;
                        // A receipt fsync is not permitted to renew the original lease.
                        lease_check.check()
                    });
                    Ok((result, owned_pending))
                },
            )
            .await?;
            *pending = Some(returned_pending);
            return adv_res;
        }
        self.check()
    }

    pub(super) async fn before_activation(
        &mut self,
        store: &MachinePluginStore,
        lifecycle: &mut Option<LifecycleGuard>,
    ) -> Result<()> {
        self.check()?;
        if let Self::Durable(InstallAdmission { pending, .. }) = self {
            let step = pending
                .as_ref()
                .context("installation continuation already consumed")?
                .receipt()
                .step
                .clone();
            let store_for_target = store.clone();
            let target = MachinePluginStore::run_with_lifecycle(
                lifecycle,
                "installation target check task failed",
                move || store_for_target.install_target(&step.plugin_id),
            )
            .await?;
            self.check()?;
            ensure!(
                target == step.expected,
                "installation target changed during staging"
            );
        }
        self.phase(store, InstallPhase::Activating, lifecycle).await
    }

    async fn finish(
        &mut self,
        store: &MachinePluginStore,
        result: &Result<PluginInventory>,
        lifecycle: &mut Option<LifecycleGuard>,
    ) -> Result<()> {
        let Self::Durable(InstallAdmission { pending, lease, .. }) = self else {
            return Ok(());
        };
        let mut owned_pending = pending
            .take()
            .context("installation continuation already consumed")?;
        let InstallOutcome::Pending { phase } = owned_pending.receipt().outcome else {
            *pending = Some(owned_pending);
            bail!("installation continuation has already ended");
        };
        let outcome = match result {
            Ok(inventory) => {
                let step = &owned_pending.receipt().step;
                ensure!(
                    inventory.plugin_id == step.plugin_id
                        && inventory.plugin_kind == step.plugin_kind
                        && inventory.plugin_version == step.plugin_version
                        && inventory.generation_digest == step.generation_digest
                        && inventory.contract_fingerprint == step.contract_fingerprint
                        && inventory.state == PluginInstallationState::Active,
                    "installation completion identity mismatch"
                );
                InstallOutcome::Applied {
                    revision: inventory
                        .installation_revision
                        .clone()
                        .context("installation has no durable incarnation")?,
                }
            }
            Err(_) => match (phase, lease.check()) {
                (InstallPhase::Prepared, Err(reason)) => InstallOutcome::Rejected { reason },
                (_, status) => InstallOutcome::Unknown {
                    phase,
                    reason: match status {
                        Err(InstallRejection::AuthorizationEnded) => {
                            InstallUncertainty::AuthorizationEnded
                        }
                        Err(InstallRejection::Expired) => InstallUncertainty::Expired,
                        _ => InstallUncertainty::EffectFailure,
                    },
                },
            },
        };
        // Completion records an already observed effect. It grants no next
        // effect and may acknowledge it after the original lease has ended.
        let store_for_fin = store.clone();
        let (adv_res, returned_pending) = MachinePluginStore::run_with_lifecycle(
            lifecycle,
            "installation completion transition task failed",
            move || {
                let result = store_for_fin
                    .operations
                    .install_attempts
                    .advance(&mut owned_pending, outcome);
                Ok((result, owned_pending))
            },
        )
        .await?;
        *pending = Some(returned_pending);
        adv_res
    }
}

fn unavailable(reason: InstallUnavailable) -> InstallLookup {
    InstallLookup::Unavailable { reason }
}

impl MachinePluginStore {
    fn install_target(&self, plugin: &str) -> Result<InstallTarget> {
        let target = self
            .operations
            .installations
            .install_target(plugin, self.recovery_active_digest(plugin))?;
        if let InstallTarget::Installed {
            generation_digest, ..
        } = &target
        {
            // Incarnation CAS is not permission to repair an unverified or
            // damaged current generation. Retained signed legacy Agents keep
            // their existing verifier; non-Agent cache JSON is not evidence.
            let active = self
                .inventory_one_untracked(plugin)?
                .context("installed target unavailable")?;
            ensure!(
                active.generation_digest == *generation_digest,
                "installed target changed"
            );
            if active.plugin_kind != cowboy_plugin_sdk::PluginKind::AgentProvider {
                self.verified_plugin_generation(plugin, generation_digest)?;
            }
        }
        Ok(target)
    }

    fn install_admission(&self, admitted: bool) -> bool {
        admitted && self.operations.installations.writer_enabled()
    }

    pub async fn installation_target(
        &self,
        query: &InstallTargetQuery,
        service: Option<&str>,
        machine: &str,
        admitted: bool,
    ) -> InstallTargetObservation {
        let lifecycle = self.lock_lifecycle_owned().await;
        let store = self.clone();
        let query = query.clone();
        let service = service.map(str::to_owned);
        let machine = machine.to_owned();
        tokio::task::spawn_blocking(move || {
            let _lifecycle = lifecycle;
            let result = (|| {
                let query_digest = query
                    .digest()
                    .map_err(|_| InstallUnavailable::InvalidRequest)?;
                if service.as_deref() != Some(query.service_id.as_str())
                    || machine != query.machine_id
                {
                    return Err(InstallUnavailable::WrongOwner);
                }
                store
                    .operations
                    .ensure_unfenced(&query.plugin_id)
                    .map_err(|_| InstallUnavailable::SlotFenced)?;
                let target = store
                    .install_target(&query.plugin_id)
                    .map_err(|_| InstallUnavailable::Untracked)?;
                Ok(InstallTargetObservation::Observed {
                    query_digest,
                    admission_enabled: store.install_admission(admitted),
                    target,
                })
            })();
            result.unwrap_or_else(|reason| InstallTargetObservation::Unavailable { reason })
        })
        .await
        .unwrap_or(InstallTargetObservation::Unavailable {
            reason: InstallUnavailable::Untracked,
        })
    }

    pub async fn query_installation_step(
        &self,
        step: &InstallStep,
        service: Option<&str>,
        machine: &str,
        admitted: bool,
    ) -> InstallObservation {
        let lifecycle = self.lock_lifecycle_owned().await;
        let store = self.clone();
        let step = step.clone();
        let service = service.map(str::to_owned);
        let machine = machine.to_owned();
        let admission_enabled = service.is_some() && store.install_admission(admitted);
        let result = tokio::task::spawn_blocking(move || {
            let _lifecycle = lifecycle;
            if step.validate().is_err() {
                unavailable(InstallUnavailable::InvalidRequest)
            } else if service.as_deref() != Some(step.service_id.as_str())
                || machine != step.machine_id
            {
                unavailable(InstallUnavailable::WrongOwner)
            } else {
                store.installation_preflight(&step)
            }
        })
        .await
        .unwrap_or_else(|_| unavailable(InstallUnavailable::Storage));
        InstallObservation {
            admission_enabled,
            result,
        }
    }

    fn installation_preflight(&self, step: &InstallStep) -> InstallLookup {
        match self.operations.install_attempts.query(step) {
            InstallLookup::NotFound {} => {
                if self.operations.ensure_unfenced(&step.plugin_id).is_err() {
                    unavailable(InstallUnavailable::SlotFenced)
                } else if !self
                    .install_target(&step.plugin_id)
                    .is_ok_and(|target| target == step.expected)
                {
                    unavailable(InstallUnavailable::TargetChanged)
                } else {
                    InstallLookup::NotFound {}
                }
            }
            found => found,
        }
    }

    pub async fn install_step(
        &self,
        step: &InstallStep,
        desired: &DesiredPlugin,
        lease: &InstallationLease,
        admitted: bool,
    ) -> InstallObservation {
        let mut lifecycle = Some(self.lock_lifecycle_owned().await);
        let admission_enabled = self.install_admission(admitted);
        let result = if !step.matches_envelope(desired) || !lease.matches(step) {
            unavailable(InstallUnavailable::InvalidRequest)
        } else {
            let store = self.clone();
            let preflight_step = step.clone();
            let preflight = Self::run_with_lifecycle(
                &mut lifecycle,
                "installation preflight task failed",
                move || Ok(store.installation_preflight(&preflight_step)),
            )
            .await
            .unwrap_or_else(|_| unavailable(InstallUnavailable::Storage));
            match preflight {
                InstallLookup::NotFound {} if !admission_enabled => {
                    unavailable(InstallUnavailable::ReaderOnly)
                }
                InstallLookup::NotFound {} => {
                    self.execute_installation(step, desired, lease, &mut lifecycle)
                        .await
                }
                found => found,
            }
        };
        InstallObservation {
            admission_enabled,
            result,
        }
    }

    async fn execute_installation(
        &self,
        step: &InstallStep,
        desired: &DesiredPlugin,
        lease: &InstallationLease,
        lifecycle: &mut Option<LifecycleGuard>,
    ) -> InstallLookup {
        enum PrepareDecision {
            Ready(Box<VerifiedInstallPackage>, Box<PendingAttempt>),
            Rejected(InstallUnavailable),
        }
        let store = self.clone();
        let desired_owned = desired.clone();
        let step_owned = step.clone();
        let lease_owned = lease.clone_lease();
        let prepared = Self::run_with_lifecycle(
            lifecycle,
            "installation preparation task failed",
            move || {
                if lease_owned.check().is_err() {
                    return Ok(PrepareDecision::Rejected(InstallUnavailable::WrongOwner));
                }
                let Ok(verified) = store.verified_install_package(&desired_owned) else {
                    return Ok(PrepareDecision::Rejected(
                        InstallUnavailable::InvalidRequest,
                    ));
                };
                if lease_owned.check().is_err() {
                    return Ok(PrepareDecision::Rejected(InstallUnavailable::WrongOwner));
                }
                let journal = &store.operations.install_attempts;
                if !journal.has_capacity() {
                    return Ok(PrepareDecision::Rejected(InstallUnavailable::Capacity));
                }
                let Ok(pending) = journal.begin(&step_owned) else {
                    return Ok(PrepareDecision::Rejected(InstallUnavailable::Storage));
                };
                Ok(PrepareDecision::Ready(
                    Box::new(verified),
                    Box::new(pending),
                ))
            },
        )
        .await;
        let (verified, pending) = match prepared {
            Ok(PrepareDecision::Ready(verified, pending)) => (*verified, pending),
            Ok(PrepareDecision::Rejected(reason)) => return unavailable(reason),
            Err(_) => return unavailable(InstallUnavailable::Storage),
        };
        let mut guard = InstallGuard::Durable(InstallAdmission {
            pending: Some(pending),
            lease,
        });
        let result = self
            .install_inner(desired, verified, &mut guard, lifecycle)
            .await;
        if guard.finish(self, &result, lifecycle).await.is_err() {
            return unavailable(InstallUnavailable::Storage);
        }
        let store_for_query = self.clone();
        let step_for_query = step.clone();
        MachinePluginStore::run_with_lifecycle(
            lifecycle,
            "installation receipt query task failed",
            move || {
                Ok(store_for_query
                    .operations
                    .install_attempts
                    .query(&step_for_query))
            },
        )
        .await
        .unwrap_or_else(|_| unavailable(InstallUnavailable::Storage))
    }
    #[allow(clippy::too_many_lines)] // One fenced Agent Provider install flow.
    pub(super) async fn install_agent_plugin(
        &self,
        plugin_package: &PluginPackage,
        desired: &DesiredPlugin,
        host_bundle: Option<&PluginHostBundle>,
        host_bundle_bytes: Option<&[u8]>,
        guard: &mut InstallGuard<'_>,
        lifecycle: &mut Option<LifecycleGuard>,
    ) -> Result<PluginInventory> {
        let package = plugin_package
            .agent_provider()
            .context("Machine Agent installer received a non-Agent Plugin")?
            .clone();
        let provider_release = desired.release.agent_provider_binding(plugin_package)?;
        let payload = matching_payload(&package, &self.platform, &self.architecture)?;
        let runtime_artifacts =
            matching_runtime_artifacts(&provider_release, &self.platform, &self.architecture)?;
        let plugin_root = self.plugin_root(&package.manifest.id);
        let generation_name = digest_generation_name(&desired.release.artifact_digest)?;
        let generation = plugin_root.join("generations").join(&generation_name);
        let content = generation.join("content");
        let (auth_envelope, prepared_auth) = self
            .prestage_agent_generation_async(
                &package,
                plugin_package,
                desired,
                host_bundle,
                host_bundle_bytes,
                &generation,
                &content,
                guard,
                lifecycle,
            )
            .await?;

        let started = Instant::now();
        let mut prepared = runtime_staging::prepare_provider_runtime(
            &content,
            runtime_artifacts,
            guard,
            self.root.parent(),
            account_sandbox::sandboxed(&package),
            lifecycle,
            Some(std::sync::Arc::clone(&self.lifecycle)),
        )
        .await?;
        let runtime_ms = started.elapsed().as_millis();
        let probe_started = Instant::now();
        if !prepared.is_already_installed() {
            let launch_command = match prepared.command_path(&payload.launch_command) {
                Ok(cmd) => cmd,
                Err(error) => {
                    prepared.discard(lifecycle).await;
                    return Err(error);
                }
            };
            prepared = prepared.verify_before_outer_probe(lifecycle).await?;
            if let Err(error) = guard.check() {
                prepared.discard(lifecycle).await;
                return Err(error);
            }
            let exit_confirmed = match guard
                .bounded(probe_provider_runtime(
                    &package.manifest.runtime,
                    &launch_command,
                ))
                .await
            {
                Ok(confirmed) => confirmed,
                Err(error) => {
                    prepared.discard(lifecycle).await;
                    return Err(error);
                }
            };
            prepared.record_outer_probe_exit_confirmed(exit_confirmed);
        }
        let probe_ms = probe_started.elapsed().as_millis();
        let _runtime = prepared.finalize(guard, lifecycle).await?;
        let flush_started = Instant::now();
        guard.flush_staging(&content, lifecycle).await?;
        tracing::info!(
            plugin_id = %package.manifest.id,
            runtime_ms,
            probe_ms,
            flush_ms = flush_started.elapsed().as_millis(),
            staging_ms = started.elapsed().as_millis(),
            "Provider generation staged"
        );
        // Last, so a worker selected while staging is the one that is checked.
        guard
            .bounded(self.ensure_worker_reads_package(
                &content.join("package.cowboy-provider"),
                &package.manifest.runtime.required_capabilities,
            ))
            .await?;
        guard.before_activation(self, lifecycle).await?;
        let pending = self
            .begin_installation_async(
                &package.manifest.id,
                &desired.release.artifact_digest,
                guard,
                lifecycle,
            )
            .await?;
        let activation = {
            let plugin_root_for_activate = plugin_root.clone();
            let generation_name_for_activate = generation_name.clone();
            let lease_check = guard.lease_check();
            Self::run_with_lifecycle(lifecycle, "Provider activation task failed", move || {
                Self::activate_checked(
                    &plugin_root_for_activate,
                    &generation_name_for_activate,
                    &lease_check,
                )
            })
            .await?
        };
        // A sealed Service replica may predate installation. Materialize it as
        // part of activation so installation never asks for another login.
        if let (Some(envelope), Some(prepared)) = (auth_envelope.as_ref(), prepared_auth) {
            guard
                .phase(self, InstallPhase::ProjectingAuthentication, lifecycle)
                .await?;
            self.commit_install_auth_async(
                envelope,
                &package,
                prepared,
                &plugin_root,
                activation,
                guard,
                lifecycle,
            )
            .await?;
        }
        self.finish_installation_async(&package.manifest.id, pending, lifecycle)
            .await?;
        self.inventory_one_async(&package.manifest.id, lifecycle)
            .await?
            .context("activated Provider is missing from inventory")
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit_install_auth_async(
        &self,
        envelope: &SealedProviderAuth,
        package: &ProviderPackage,
        prepared: PreparedProviderAuth,
        plugin_root: &Path,
        activation: ProviderActivationSnapshot,
        guard: &InstallGuard<'_>,
        lifecycle: &mut Option<LifecycleGuard>,
    ) -> Result<()> {
        let store = self.clone();
        let envelope = envelope.clone();
        let package = package.clone();
        let plugin_root = plugin_root.to_owned();
        let lease_check = guard.lease_check();
        Self::run_with_lifecycle(
            lifecycle,
            "Provider authentication activation task failed",
            move || -> Result<()> {
                lease_check.check()?;
                if let Err(error) = store.commit_prepared_auth(&envelope, Some(&package), prepared) {
                    lease_check.check()?;
                    if let Err(rollback_error) = Self::restore_activation(&plugin_root, &activation)
                    {
                        bail!(
                            "Provider authentication activation failed: {error:#}; restoring the previous Provider generation also failed: {rollback_error:#}"
                        );
                    }
                    return Err(error.context(
                        "Provider authentication activation failed; previous generation restored",
                    ));
                }
                Ok(())
            },
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn prestage_agent_generation_async(
        &self,
        package: &ProviderPackage,
        plugin_package: &PluginPackage,
        desired: &DesiredPlugin,
        host_bundle: Option<&PluginHostBundle>,
        host_bundle_bytes: Option<&[u8]>,
        generation: &Path,
        content: &Path,
        guard: &InstallGuard<'_>,
        lifecycle: &mut Option<LifecycleGuard>,
    ) -> Result<(Option<SealedProviderAuth>, Option<PreparedProviderAuth>)> {
        let store = self.clone();
        let package = package.clone();
        let plugin_package = plugin_package.clone();
        let desired = desired.clone();
        let host_bundle = host_bundle.cloned();
        let host_bundle_bytes = host_bundle_bytes.map(ToOwned::to_owned);
        let generation = generation.to_owned();
        let content = content.to_owned();
        let lease_check = guard.lease_check();
        Self::run_with_lifecycle(
            lifecycle,
            "Provider generation pre-staging task failed",
            move || -> Result<_> {
                lease_check.check()?;
                let auth_envelope = store.latest_auth_envelope(&package.manifest.id)?;
                let prepared_auth = auth_envelope
                    .as_ref()
                    .map(|envelope| store.prepare_auth_inner(envelope, Some(&package)))
                    .transpose()?;
                lease_check.check()?;
                fs::create_dir_all(&content).with_context(|| {
                    format!("creating Provider generation {}", generation.display())
                })?;
                fs::set_permissions(&generation, fs::Permissions::from_mode(0o700))?;
                write_provider_generation_files(
                    &content,
                    &package,
                    &plugin_package,
                    &desired,
                    host_bundle.as_ref(),
                    host_bundle_bytes.as_deref(),
                )?;
                Ok((auth_envelope, prepared_auth))
            },
        )
        .await
    }

    #[allow(clippy::too_many_lines)] // One fenced non-Agent Plugin install flow.
    pub(super) async fn install_non_agent_plugin(
        &self,
        package: &PluginPackage,
        desired: &DesiredPlugin,
        host_bundle: Option<&PluginHostBundle>,
        host_bundle_bytes: Option<&[u8]>,
        guard: &mut InstallGuard<'_>,
        lifecycle: &mut Option<LifecycleGuard>,
    ) -> Result<PluginInventory> {
        let artifacts = matching_plugin_runtime_artifacts(
            &desired.release.runtime_artifacts,
            &self.platform,
            &self.architecture,
        )?;
        if matches!(package.payload, PluginPayload::CodeIntelligence(_)) {
            ensure!(
                artifacts.components.iter().any(|component| {
                    component.kind == PluginComponentKind::CodeIntelligenceAdapter
                }),
                "code-intelligence Plugin has no adapter component"
            );
        }
        let runtime_artifacts = provider_staging_projection(artifacts);
        let plugin_root = self.plugin_root(&package.manifest.id);
        let generation_name = digest_generation_name(&desired.release.artifact_digest)?;
        let generation = plugin_root.join("generations").join(&generation_name);
        let content = generation.join("content");
        self.prestage_non_agent_generation_async(
            package,
            desired,
            host_bundle,
            host_bundle_bytes,
            &generation,
            &content,
            guard,
            lifecycle,
        )
        .await?;
        let mut prepared = runtime_staging::prepare_provider_runtime(
            &content,
            &runtime_artifacts,
            guard,
            self.root.parent(),
            false,
            lifecycle,
            Some(std::sync::Arc::clone(&self.lifecycle)),
        )
        .await?;
        if !prepared.is_already_installed() {
            let (returned, exit_confirmed) = self
                .probe_staged_code_runtime(
                    package,
                    &desired.release.artifact_digest,
                    prepared,
                    guard,
                    lifecycle,
                )
                .await?;
            prepared = returned;
            prepared.record_outer_probe_exit_confirmed(exit_confirmed);
        }
        prepared.finalize(guard, lifecycle).await?;
        let inventory = PluginInventory {
            plugin_id: package.manifest.id.clone(),
            plugin_version: package.manifest.version.clone(),
            plugin_kind: package.manifest.kind,
            generation_digest: desired.release.artifact_digest.clone(),
            installation_revision: None,
            contract_fingerprint: package.contract_fingerprint.clone(),
            state: PluginInstallationState::Active,
            rollback_generation_digest: None,
            active_session_leases: 0,
            auth_generation: None,
            replica_state: ProviderReplicaState::Absent,
            materialization_state: ProviderMaterializationState::NotInstalled,
            detail: None,
        };
        {
            let content_for_inv = content.clone();
            let lease_check = guard.lease_check();
            Self::run_with_lifecycle(
                lifecycle,
                "Plugin inventory staging task failed",
                move || {
                    lease_check.check()?;
                    atomic_write(
                        &content_for_inv.join("plugin-inventory.json"),
                        &serde_json::to_vec(&inventory)?,
                        0o600,
                    )
                },
            )
            .await?;
        }
        guard.flush_staging(&content, lifecycle).await?;
        guard.before_activation(self, lifecycle).await?;
        let pending = self
            .begin_installation_async(
                &package.manifest.id,
                &desired.release.artifact_digest,
                guard,
                lifecycle,
            )
            .await?;
        {
            let store = self.clone();
            let package_for_act = package.clone();
            let plugin_root_for_act = plugin_root.clone();
            let generation_name_for_act = generation_name.clone();
            let lease_check = guard.lease_check();
            Self::run_with_lifecycle(lifecycle, "Plugin activation task failed", move || {
                if matches!(package_for_act.payload, PluginPayload::CodeIntelligence(_)) {
                    store.activate_code_generation_checked(
                        &package_for_act,
                        &generation_name_for_act,
                        &lease_check,
                    )
                } else {
                    Self::activate_checked(
                        &plugin_root_for_act,
                        &generation_name_for_act,
                        &lease_check,
                    )?;
                    Ok(())
                }
            })
            .await?;
        }
        self.finish_installation_async(&package.manifest.id, pending, lifecycle)
            .await?;
        self.inventory_one_async(&package.manifest.id, lifecycle)
            .await?
            .context("activated Plugin is missing from inventory")
    }

    #[allow(clippy::too_many_arguments)]
    async fn prestage_non_agent_generation_async(
        &self,
        package: &PluginPackage,
        desired: &DesiredPlugin,
        host_bundle: Option<&PluginHostBundle>,
        host_bundle_bytes: Option<&[u8]>,
        generation: &Path,
        content: &Path,
        guard: &InstallGuard<'_>,
        lifecycle: &mut Option<LifecycleGuard>,
    ) -> Result<()> {
        let store = self.clone();
        let package = package.clone();
        let desired = desired.clone();
        let host_bundle = host_bundle.cloned();
        let host_bundle_bytes = host_bundle_bytes.map(ToOwned::to_owned);
        let generation = generation.to_owned();
        let content = content.to_owned();
        let lease_check = guard.lease_check();
        Self::run_with_lifecycle(
            lifecycle,
            "Plugin generation pre-staging task failed",
            move || -> Result<()> {
                lease_check.check()?;
                store.validate_extension_dependencies(&package)?;
                lease_check.check()?;
                fs::create_dir_all(&content).with_context(|| {
                    format!("creating Plugin generation {}", generation.display())
                })?;
                fs::set_permissions(&generation, fs::Permissions::from_mode(0o700))?;
                atomic_write(
                    &content.join("package.cowboy-plugin"),
                    &package.canonical_bytes()?,
                    0o600,
                )?;
                atomic_write(
                    &content.join("plugin-release.json"),
                    &serde_json::to_vec(&desired.release)?,
                    0o600,
                )?;
                atomic_write(
                    &content.join("publisher.pub"),
                    desired.publisher_public_key.as_bytes(),
                    0o600,
                )?;
                stage_plugin_host_bundle(
                    &content,
                    host_bundle.as_ref(),
                    host_bundle_bytes.as_deref(),
                )?;
                Ok(())
            },
        )
        .await
    }
}

#[cfg(test)]
pub(super) mod tests;

pub(super) fn ensure_machine_installable_kind(package: &PluginPackage) -> Result<()> {
    ensure!(
        matches!(
            package.manifest.kind,
            cowboy_plugin_sdk::PluginKind::AgentProvider
                | cowboy_plugin_sdk::PluginKind::CodeIntelligence
                | cowboy_plugin_sdk::PluginKind::TelemetryBackend
                | cowboy_plugin_sdk::PluginKind::WorkspaceExtension
                | cowboy_plugin_sdk::PluginKind::MemoryBackend
                | cowboy_plugin_sdk::PluginKind::CallProfile
                | cowboy_plugin_sdk::PluginKind::McpServer
                | cowboy_plugin_sdk::PluginKind::MachineBrowser
        ),
        "Machine installer received a Controller-only Plugin kind"
    );
    Ok(())
}
