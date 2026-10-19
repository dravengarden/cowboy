//! Native logout reservation and historical leader outcome. Dispatch stays gated.
#![allow(dead_code)]
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub request: String,
    pub succeeded: Option<bool>,
    #[serde(default)]
    pub process_tree_exited: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<processes::Reservation>,
}

pub(crate) enum LogoutPreparation {
    Execute(Box<PreparedNativeLogin>),
    /// None is unknown, never permission to launch the same operation again.
    Observed(Option<bool>),
}

impl MachinePluginStore {
    /// Caller must separately establish session-stop and login-cancel authority.
    /// This never reads, copies or exports native credentials.
    pub(crate) fn prepare_native_logout(
        &self,
        provider: &str,
        generation: u64,
        login: &str,
        operation: &str,
    ) -> Result<LogoutPreparation> {
        let _guard = self.auth_candidates.lock();
        validate_plugin_id(provider)?;
        validate_auth_candidate_id(operation)?;
        let (directory, mut profile) =
            self.read_native_profile_for_cleanup(provider, generation)?;
        ensure!(
            profile.request == login && profile.process_tree_exited,
            "native login exit or scope unconfirmed"
        );
        if let Some(record) = &profile.logout {
            ensure!(
                record.request == operation,
                "native logout operation conflict"
            );
            return Ok(LogoutPreparation::Observed(record.succeeded));
        }
        ensure_no_revocation(&self.native_profile_root(provider))?;
        ensure!(!profile.revoked, "native profile was revoked");
        let (package, path) = self.package_for_generation(provider, &profile.digest)?;
        ensure!(
            package.manifest.version == profile.version
                && package.manifest.compatibility.auth_contract_fingerprint == profile.contract,
            "native logout package changed"
        );
        let mut method = package
            .manifest
            .authentication
            .methods
            .iter()
            .find(|method| method.id == profile.method)
            .cloned()
            .context("native logout method missing")?;
        ensure!(method.native_identity.is_some(), "native identity required");
        let Some(cowboy_provider_sdk::NativeLogoutRecipe::CommandV1 {
            arguments: logout_arguments,
        }) = &method.native_logout
        else {
            anyhow::bail!("native logout recipe missing");
        };
        let cowboy_provider_sdk::AuthExecutor::CommandV1 {
            component,
            arguments,
            terminal,
            preflight,
            ..
        } = &mut method.executor
        else {
            anyhow::bail!("native logout executor missing")
        };
        arguments.clone_from(logout_arguments);
        *terminal = cowboy_provider_sdk::AuthTerminal::Pipes;
        // Login setup can overwrite settings; it must not run during logout.
        preflight.clear();
        let payload = matching_payload(&package, &self.platform, &self.architecture)?;
        let requirement = payload
            .private_components
            .iter()
            .find(|candidate| candidate.kind == component.kind && candidate.slot == component.slot)
            .context("native logout component missing")?;
        let command = runtime_command(&path, &requirement.command)?;
        let home = directory.join("home");
        ensure!(
            fs::symlink_metadata(&home)?.is_dir(),
            "native logout home missing or linked"
        );
        profile.logout = Some(Record {
            request: operation.into(),
            succeeded: None,
            process_tree_exited: false,
            process: Some(processes::Reservation::new()?),
        });
        persist(&directory, &profile)?;
        Ok(LogoutPreparation::Execute(Box::new(PreparedNativeLogin {
            home,
            command,
            method,
        })))
    }

    pub(crate) fn record_native_logout_exit(
        &self,
        provider: &str,
        generation: u64,
        operation: &str,
        succeeded: bool,
    ) -> Result<()> {
        self.record_native_logout_outcome(provider, generation, operation, succeeded, false)
    }

    pub(crate) fn record_native_logout_tree_exit(
        &self,
        provider: &str,
        generation: u64,
        operation: &str,
        succeeded: bool,
    ) -> Result<()> {
        self.record_native_logout_outcome(provider, generation, operation, succeeded, true)
    }

    fn record_native_logout_outcome(
        &self,
        provider: &str,
        generation: u64,
        operation: &str,
        succeeded: bool,
        process_tree_exited: bool,
    ) -> Result<()> {
        let _guard = self.auth_candidates.lock();
        validate_plugin_id(provider)?;
        let (directory, mut profile) =
            self.read_native_profile_for_cleanup(provider, generation)?;
        let record = profile
            .logout
            .as_mut()
            .context("native logout not reserved")?;
        ensure!(
            !profile.revoked || record.succeeded == Some(succeeded),
            "revoked profile cannot acquire new logout evidence"
        );
        ensure!(record.request == operation, "native logout scope mismatch");
        ensure!(
            record
                .succeeded
                .is_none_or(|previous| previous == succeeded),
            "native logout outcome conflict"
        );
        ensure!(
            record.succeeded.is_none() || record.process_tree_exited == process_tree_exited,
            "native logout containment evidence conflict"
        );
        record.succeeded = Some(succeeded);
        record.process_tree_exited = process_tree_exited;
        persist(&directory, &profile)
    }

    /// Read both durable outcomes, including after private-home deletion.
    pub(crate) fn native_auth_trees_exited(
        &self,
        provider: &str,
        generation: u64,
        login: &str,
        operation: &str,
    ) -> Result<bool> {
        let _guard = self.auth_candidates.lock();
        validate_plugin_id(provider)?;
        let (_, profile) = self.read_native_profile_for_cleanup(provider, generation)?;
        ensure!(profile.request == login, "native login scope mismatch");
        let record = profile
            .logout
            .as_ref()
            .context("native logout not reserved")?;
        ensure!(record.request == operation, "native logout scope mismatch");
        Ok(profile.process_tree_exited && record.process_tree_exited)
    }

    /// Delete only the exact logged-out generation. The retained tombstone
    /// prevents reusing the original login, including after a partial deletion.
    pub(crate) fn cleanup_logged_out_native_profile(
        &self,
        provider: &str,
        generation: u64,
        login: &str,
        operation: &str,
    ) -> Result<()> {
        let _guard = self.auth_candidates.lock();
        validate_plugin_id(provider)?;
        validate_auth_candidate_id(operation)?;
        let (directory, mut profile) =
            self.read_native_profile_for_cleanup(provider, generation)?;
        ensure!(
            profile.request == login && profile.process_tree_exited,
            "native login scope or exit unconfirmed"
        );
        ensure!(
            profile
                .logout
                .as_ref()
                .is_some_and(|record| record.request == operation
                    && record.succeeded == Some(true)
                    && record.process_tree_exited),
            "native logout success unconfirmed"
        );
        profile.revoked = true;
        persist(&directory, &profile)?;
        remove_projection::remove(&directory.join("home"))?;
        fs::File::open(&directory)?.sync_all()?;
        Ok(())
    }
}

pub(super) fn persist(directory: &Path, profile: &Profile) -> Result<()> {
    atomic_write(
        &directory.join("profile.json"),
        &serde_json::to_vec(profile)?,
        0o600,
    )?;
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}
