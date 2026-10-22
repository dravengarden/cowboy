//! Prepared binding evidence only. Preparation never removes a stop fence.
#![allow(dead_code)] // Activated only with the complete broker readmission protocol.
use super::{
    Admission, MachinePluginStore, atomic_write, marker_name, native_identity,
    prepare_private_directory,
};
use crate::provider_account::{PreparedBinding, account_stop_command_id};
use anyhow::{Result, ensure};
use std::fs;

impl MachinePluginStore {
    pub fn readmit_native_session(
        &self,
        binding: &PreparedBinding,
        session: &crate::runtime_wire::StartSession,
    ) -> Result<()> {
        let target = &binding.target;
        ensure!(
            session.session_id == binding.stopped.request.session_id
                && session.provider_account_id == target.account_id
                && session.provider == target.provider_id
                && session.provider_version == target.provider_version
                && session.provider_generation_digest == target.generation_digest
                && session.provider_auth_generation == Some(target.generation)
                && session.agent_session_id.is_none()
                && session.adopt_only,
            "readmission declaration mismatch"
        );
        let (package, _) =
            self.package_for_generation(&target.provider_id, &target.generation_digest)?;
        ensure!(
            session.provider_behavior.as_ref() == Some(&package.manifest.runtime.behavior),
            "readmission behavior mismatch"
        );
        self.readmit_account_binding(binding, &package)
    }
    pub fn prepare_native_binding(&self, binding: &PreparedBinding) -> Result<()> {
        let (package, _) = self.package_for_generation(
            &binding.target.provider_id,
            &binding.target.generation_digest,
        )?;
        self.prepare_account_binding(binding, &package)
    }
    /// Revalidate even identical retries: a historical preparation is not
    /// current admission. Serialize with stop, worker and login mutations.
    pub fn prepare_account_binding(
        &self,
        binding: &PreparedBinding,
        package: &super::super::ProviderPackage,
    ) -> Result<()> {
        let _guard = self.auth_candidates.lock();
        self.prepare_account_binding_locked(binding, package)
    }

    fn prepare_account_binding_locked(
        &self,
        binding: &PreparedBinding,
        package: &super::super::ProviderPackage,
    ) -> Result<()> {
        let request = &binding.stopped.request;
        ensure!(
            self.account_id.is_default()
                && crate::managed_calls::valid_id(&binding.operation)
                && !request.account_id.is_default()
                && binding.target.account_id == request.account_id
                && binding.target.provider_id == "claude-code",
            "invalid prepared account binding scope"
        );
        let command = account_stop_command_id(
            &format!("migration:{}", binding.operation),
            &request.session_id,
            &request.account_id,
        );
        ensure!(
            request.stop_command_id.as_ref() == Some(&command)
                && self.account_stop_command(&request.session_id, &request.account_id)?
                    == Some(command),
            "prepared binding stop command mismatch"
        );
        let exited = self.account_worker_exit(&request.session_id)?;
        ensure!(
            exited.is_some_and(|exit| exit.account == request.account_id
                && exit.tree_exited
                && exit.launch == binding.stopped.launch_id),
            "prepared binding worker exit mismatch"
        );
        let scoped = self.for_account(&request.account_id)?;
        let target = &binding.target;
        ensure!(
            package.manifest.id == target.provider_id
                && scoped.native_profile_receipt(
                    &target.provider_id,
                    target.generation,
                    &target.request_id
                )? == *target,
            "prepared binding native receipt mismatch"
        );
        scoped.ensure_native_release(package, target.generation, &target.generation_digest)?;
        scoped.native_profile_home(package, target.generation)?;
        let root = self.auth_base.join("session-account-preparations");
        prepare_private_directory(&root)?;
        let name = marker_name(&serde_json::to_string(&(
            &request.session_id,
            &binding.operation,
        ))?);
        let path = root.join(&name);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                atomic_write(&path, &serde_json::to_vec(&(1u32, binding))?, 0o600)?;
            }
            result => {
                ensure!(result?.is_file(), "invalid prepared binding file");
                let (schema, previous): (u32, PreparedBinding) = serde_json::from_slice(
                    &native_identity::read_login_file(&root, &name, 16384)?,
                )?;
                ensure!(
                    schema == 1 && previous == *binding,
                    "prepared binding conflict"
                );
            }
        }
        fs::File::open(root)?.sync_all()?;
        fs::File::open(&self.auth_base)?.sync_all()?;
        Ok(())
    }

    /// Caller must hold the broker lifecycle gate and verify current Controller
    /// authorization. This local step changes no processes or broker maps.
    pub fn readmit_account_binding(
        &self,
        binding: &PreparedBinding,
        package: &super::super::ProviderPackage,
    ) -> Result<()> {
        let _guard = self.auth_candidates.lock();
        let request = &binding.stopped.request;
        let target = &binding.target;
        let admitted = Admission {
            operation: binding.operation.clone(),
            launch: binding.stopped.launch_id,
            binding: super::worker_exit::WorkerBinding {
                provider: target.provider_id.clone(),
                version: target.provider_version.clone(),
                digest: target.generation_digest.clone(),
                generation: target.generation,
            },
        };
        ensure!(
            self.account_id.is_default()
                && !target.account_id.is_default()
                && target.account_id == request.account_id
                && target.provider_id == "claude-code"
                && package.manifest.id == target.provider_id
                && crate::managed_calls::valid_id(&binding.operation),
            "invalid readmission scope"
        );
        admitted.binding.validate()?;
        let scoped = self.for_account(&target.account_id)?;
        ensure!(
            scoped.native_profile_receipt(
                &target.provider_id,
                target.generation,
                &target.request_id
            )? == *target,
            "readmission target changed"
        );
        scoped.ensure_native_release(package, target.generation, &target.generation_digest)?;
        scoped.native_profile_home(package, target.generation)?;
        let root = self.account_stop_root();
        let name = marker_name(&request.session_id);
        let bytes = native_identity::read_login_file(&root, &name, 4096)?;
        let (session, account, command) = super::decode_marker(&bytes)?;
        ensure!(
            session == request.session_id
                && account == request.account_id
                && command == request.stop_command_id,
            "readmission stop mismatch"
        );
        if let Some((previous, stopped)) = super::managed_marker(&bytes)? {
            if !stopped {
                ensure!(previous == admitted, "readmission conflict");
                fs::File::open(&root)?.sync_all()?;
                return Ok(());
            }
            ensure!(
                target.generation > previous.binding.generation,
                "readmission generation already consumed"
            );
        }
        self.prepare_account_binding_locked(binding, package)?;
        let mut worker = self
            .account_worker_exit(&request.session_id)?
            .ok_or_else(|| anyhow::anyhow!("worker exit missing"))?;
        let old = worker
            .binding
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("worker binding unknown"))?;
        ensure!(
            old.provider == target.provider_id
                && (old.generation < target.generation
                    || (old == &admitted.binding
                        && worker.readmission.as_ref() == Some(&binding.operation))),
            "readmission must advance the worker binding"
        );
        worker.binding = Some(admitted.binding.clone());
        worker.readmission = Some(binding.operation.clone());
        // Durable binding first, release second. A crash in between remains
        // fenced and can only retry this exact operation and target.
        self.persist_worker_exit(&worker)?;
        atomic_write(
            &root.join(name),
            &serde_json::to_vec(&(3u32, session, account, command, admitted, false))?,
            0o600,
        )?;
        fs::File::open(&root)?.sync_all()?;
        Ok(())
    }
}
