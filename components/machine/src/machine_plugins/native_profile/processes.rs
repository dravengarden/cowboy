//! Crash recovery proves termination, never native command success. The owner
//! stamp also covers the interval between profile reservation and containment.
use super::*;
use crate::cgroup::{ContainmentIdentity, ProcessOwner};

#[cfg(all(test, target_os = "linux"))]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reservation {
    owner: ProcessOwner,
    containment: Option<ContainmentIdentity>,
}

impl Reservation {
    pub(super) fn new() -> Result<Self> {
        Ok(Self {
            owner: ProcessOwner::current()?,
            containment: None,
        })
    }

    fn bind(&mut self, identity: &ContainmentIdentity) -> Result<()> {
        ensure!(
            self.owner == ProcessOwner::current()? && identity.owner() == &self.owner,
            "native process owner mismatch"
        );
        ensure!(self.containment.is_none(), "native process already bound");
        self.containment = Some(identity.clone());
        Ok(())
    }

    async fn recover(&self) -> Result<bool> {
        if let Some(identity) = &self.containment {
            ensure!(
                identity.owner() == &self.owner,
                "native containment owner mismatch"
            );
        }
        if !self.owner.exited()? {
            return Ok(false);
        }
        if let Some(identity) = &self.containment {
            identity.terminate().await?;
        }
        // No containment means the old owner never reached spawn. Only new
        // records carry this reservation; a missing legacy record proves nothing.
        Ok(true)
    }
}

impl MachinePluginStore {
    pub fn bind_native_login_process(
        &self,
        provider: &str,
        generation: u64,
        request: &str,
        identity: &ContainmentIdentity,
    ) -> Result<()> {
        let _guard = self.auth_candidates.lock();
        let (directory, mut profile) = self.read_native_profile_metadata(provider, generation)?;
        ensure!(
            profile.request == request
                && !profile.revoked
                && !profile.process_tree_exited
                && profile.logout.is_none(),
            "native login process scope mismatch"
        );
        profile
            .process
            .as_mut()
            .context("native login reservation missing")?
            .bind(identity)?;
        logout::persist(&directory, &profile)
    }

    pub fn bind_native_logout_process(
        &self,
        provider: &str,
        generation: u64,
        operation: &str,
        identity: &ContainmentIdentity,
    ) -> Result<()> {
        let _guard = self.auth_candidates.lock();
        let (directory, mut profile) =
            self.read_native_profile_for_cleanup(provider, generation)?;
        ensure!(
            !profile.revoked && profile.process_tree_exited,
            "native login stop unconfirmed"
        );
        let record = profile
            .logout
            .as_mut()
            .context("native logout reservation missing")?;
        ensure!(
            record.request == operation
                && record.succeeded.is_none()
                && !record.process_tree_exited,
            "native logout process scope mismatch"
        );
        record
            .process
            .as_mut()
            .context("native logout process reservation missing")?
            .bind(identity)?;
        logout::persist(&directory, &profile)
    }

    /// Called while reconciling the original logout, never a new operation.
    /// An active owner (including another concurrent retry) is not an orphan.
    pub async fn recover_native_auth_processes(
        &self,
        provider: &str,
        generation: u64,
        login: &str,
        operation: &str,
    ) -> Result<()> {
        validate_auth_candidate_id(login)?;
        validate_auth_candidate_id(operation)?;
        let snapshot = {
            let _guard = self.auth_candidates.lock();
            let (_, profile) = self.read_native_profile_for_cleanup(provider, generation)?;
            ensure!(profile.request == login, "native recovery scope mismatch");
            ensure!(
                profile
                    .logout
                    .as_ref()
                    .is_none_or(|record| record.request == operation),
                "native logout recovery scope mismatch"
            );
            profile
        };
        let login_stopped = !snapshot.process_tree_exited
            && match &snapshot.process {
                Some(process) => process.recover().await?,
                None => false,
            };
        let logout_stopped = match &snapshot.logout {
            Some(record) if !record.process_tree_exited => match &record.process {
                Some(process) => process.recover().await?,
                None => false,
            },
            _ => false,
        };
        if !login_stopped && !logout_stopped {
            return Ok(());
        }
        let _guard = self.auth_candidates.lock();
        let (directory, mut profile) =
            self.read_native_profile_for_cleanup(provider, generation)?;
        ensure!(
            profile.request == snapshot.request && profile.process == snapshot.process,
            "native recovery reservation changed"
        );
        if login_stopped {
            profile.process_tree_exited = true;
        }
        if logout_stopped {
            let record = profile
                .logout
                .as_mut()
                .context("native logout reservation disappeared")?;
            let original = snapshot
                .logout
                .as_ref()
                .context("native logout snapshot missing")?;
            ensure!(
                record.request == original.request && record.process == original.process,
                "native logout reservation changed"
            );
            record.process_tree_exited = true;
        }
        // Do not set leader_exited, login_succeeded, identity or logout.succeeded.
        logout::persist(&directory, &profile)
    }

    /// Before a new login, end logins abandoned by an exited owner (for
    /// example a Machine replaced mid-login). Termination of the contained
    /// tree is proved first; the abandoned generation never produced an
    /// identity, so it is recorded as failed and the next preparation retires
    /// it. A live owner, a completed identity or any logout is left untouched.
    pub async fn recover_abandoned_native_logins(
        &self,
        provider: &str,
        request: &str,
    ) -> Result<()> {
        validate_plugin_id(provider)?;
        validate_auth_candidate_id(request)?;
        let root = self.native_profile_root(provider);
        let generations = match fs::read_dir(root.join("generations")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            result => result?,
        };
        let mut abandoned = Vec::new();
        {
            let _guard = self.auth_candidates.lock();
            for entry in generations {
                let name = entry?.file_name().to_string_lossy().into_owned();
                let Ok(generation) = name.parse::<u64>() else {
                    continue;
                };
                let (_, profile) = self.read_native_profile_for_cleanup(provider, generation)?;
                if profile.request != request
                    && !profile.revoked
                    && !profile.leader_exited
                    && !profile.process_tree_exited
                    && profile.identity.is_none()
                    && profile.logout.is_none()
                    && profile.process.is_some()
                {
                    abandoned.push((generation, profile));
                }
            }
        }
        for (generation, snapshot) in abandoned {
            let Some(process) = &snapshot.process else {
                continue;
            };
            if !process.recover().await? {
                continue;
            }
            let _guard = self.auth_candidates.lock();
            let (directory, mut profile) =
                self.read_native_profile_for_cleanup(provider, generation)?;
            ensure!(profile == snapshot, "native login reservation changed");
            profile.leader_exited = true;
            profile.process_tree_exited = true;
            profile.login_succeeded = false;
            logout::persist(&directory, &profile)?;
        }
        Ok(())
    }
}
