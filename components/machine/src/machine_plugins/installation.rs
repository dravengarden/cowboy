//! The finite installation path inside `MachinePluginStore`. The existing
//! installer owns all capability kinds; this adds authority and evidence, not
//! another installer or a generic replay engine.

use super::*;
use crate::machine_protocol::plugin_install::{
    InstallLookup, InstallObservation, InstallOutcome, InstallPhase, InstallRejection, InstallStep,
    InstallTarget, InstallTargetObservation, InstallTargetQuery, InstallUnavailable,
    InstallUncertainty,
};
use operations::install_attempts::{Attempts, PendingAttempt};
use operations::lease::InstallationLease;

pub(super) struct LeaseCheck(Option<InstallationLease>);

impl LeaseCheck {
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
    journal: &'a Attempts,
    pending: Box<PendingAttempt>,
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
                    admission.pending.receipt().step.matches_envelope(desired),
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
    pub(super) async fn flush_staging(&self, content: &Path) -> Result<()> {
        if matches!(self, Self::Legacy) {
            return Ok(());
        }
        self.check()?;
        let content = content.to_owned();
        let lease_check = self.lease_check();
        // Await the blocking task directly (never cancel via timeout) so no
        // detached filesystem walk outlives the store's lifecycle lock; lease
        // expiry still aborts promptly at each entry via `lease_check`.
        tokio::task::spawn_blocking(move || {
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
        })
        .await
        .context("staged installation flush task failed")??;
        self.check()
    }

    pub(super) fn check(&self) -> Result<()> {
        if let Self::Durable(InstallAdmission { lease, pending, .. }) = self {
            ensure!(
                lease.matches(&pending.receipt().step),
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

    pub(super) fn phase(&mut self, phase: InstallPhase) -> Result<()> {
        self.check()?;
        if let Self::Durable(InstallAdmission {
            journal, pending, ..
        }) = self
        {
            journal.advance(pending, InstallOutcome::Pending { phase })?;
        }
        // A receipt fsync is not permitted to renew the original lease.
        self.check()
    }

    pub(super) async fn before_activation(&mut self, store: &MachinePluginStore) -> Result<()> {
        self.check()?;
        if let Self::Durable(InstallAdmission { pending, .. }) = self {
            let step = pending.receipt().step.clone();
            let store = store.clone();
            let target = tokio::task::spawn_blocking(move || store.install_target(&step.plugin_id))
                .await
                .context("installation target check task failed")??;
            self.check()?;
            ensure!(
                target == step.expected,
                "installation target changed during staging"
            );
        }
        self.phase(InstallPhase::Activating)
    }

    fn finish(&mut self, result: &Result<PluginInventory>) -> Result<()> {
        let Self::Durable(InstallAdmission {
            journal,
            pending,
            lease,
        }) = self
        else {
            return Ok(());
        };
        let InstallOutcome::Pending { phase } = pending.receipt().outcome else {
            bail!("installation continuation has already ended");
        };
        let outcome = match result {
            Ok(inventory) => {
                let step = &pending.receipt().step;
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
        journal.advance(pending, outcome)
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
        let _lifecycle = self.lifecycle.lock().await;
        let store = self.clone();
        let query = query.clone();
        let service = service.map(str::to_owned);
        let machine = machine.to_owned();
        tokio::task::spawn_blocking(move || {
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
        let _lifecycle = self.lifecycle.lock().await;
        let store = self.clone();
        let step = step.clone();
        let service = service.map(str::to_owned);
        let machine = machine.to_owned();
        let admission_enabled = service.is_some() && store.install_admission(admitted);
        let result = tokio::task::spawn_blocking(move || {
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
        let _lifecycle = self.lifecycle.lock().await;
        let admission_enabled = self.install_admission(admitted);
        let result = if !step.matches_envelope(desired) || !lease.matches(step) {
            unavailable(InstallUnavailable::InvalidRequest)
        } else {
            let store = self.clone();
            let preflight_step = step.clone();
            let preflight =
                tokio::task::spawn_blocking(move || store.installation_preflight(&preflight_step))
                    .await
                    .unwrap_or_else(|_| unavailable(InstallUnavailable::Storage));
            match preflight {
                InstallLookup::NotFound {} if !admission_enabled => {
                    unavailable(InstallUnavailable::ReaderOnly)
                }
                InstallLookup::NotFound {} => self.execute_installation(step, desired, lease).await,
                found => found, // Even an expired duplicate is historical evidence, never replay.
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
    ) -> InstallLookup {
        // Validate the complete signed package before admitting storage/effects.
        if lease.check().is_err() {
            return unavailable(InstallUnavailable::WrongOwner);
        }
        let store = self.clone();
        let desired_check = desired.clone();
        let verified =
            tokio::task::spawn_blocking(move || store.verified_install_package(&desired_check))
                .await;
        if !matches!(verified, Ok(Ok(_))) {
            return unavailable(InstallUnavailable::InvalidRequest);
        }
        let journal = &self.operations.install_attempts;
        if !journal.has_capacity() {
            return unavailable(InstallUnavailable::Capacity);
        }
        let Ok(pending) = journal.begin(step) else {
            return unavailable(InstallUnavailable::Storage);
        };
        let mut guard = InstallGuard::Durable(InstallAdmission {
            journal,
            pending: Box::new(pending),
            lease,
        });
        let result = self.install_inner(desired, &mut guard).await;
        if guard.finish(&result).is_err() {
            return unavailable(InstallUnavailable::Storage);
        }
        journal.query(step)
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
