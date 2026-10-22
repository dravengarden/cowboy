//! Retained local exit/cleanup evidence; legacy receipts prove only leader exit.
use super::MachinePluginStore;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // Independent historical evidence, not lifecycle states.
pub struct Receipt {
    schema: u32,
    account: crate::provider_account::ProviderAccountId,
    provider: String,
    request: String,
    /// False means the owning task never spawned its native command.
    pub process_started: bool,
    /// True confirms only the owned leader, not every descendant.
    pub leader_exited: bool,
    pub candidate_removed: bool,
    #[serde(default)]
    pub process_tree_exited: bool,
}

impl MachinePluginStore {
    pub fn observe_login_stop(
        &self,
        request: &crate::provider_account::LoginStopRequest,
    ) -> Result<Option<crate::provider_account::LoginStopObservation>> {
        ensure!(
            request.account_id == self.account_id && !request.account_id.is_default(),
            "login stop account mismatch"
        );
        Ok(self
            .auth_candidate_stop(&request.provider_id, &request.login_request_id)?
            .map(|receipt| crate::provider_account::LoginStopObservation {
                request: request.clone(),
                process_started: receipt.process_started,
                leader_exited: receipt.leader_exited,
                candidate_removed: receipt.candidate_removed,
                process_tree_exited: receipt.process_tree_exited,
            }))
    }

    pub(super) fn recover_auth_candidate_stops(&self, provider: &str) -> Result<()> {
        let directory = self.auth_provider_root(provider).join("login-receipts");
        match fs::symlink_metadata(&directory) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            result => ensure!(result?.is_dir(), "invalid login receipt directory"),
        }
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            if name
                .to_str()
                .is_some_and(|name| name.starts_with('.') && name.ends_with(".partial"))
            {
                continue; // Uncommitted atomic-write scratch is never evidence.
            }
            let request = name
                .to_str()
                .and_then(|name| name.strip_suffix(".json"))
                .ok_or_else(|| anyhow::anyhow!("invalid login receipt name"))?;
            let receipt = self
                .auth_candidate_stop(provider, request)?
                .ok_or_else(|| anyhow::anyhow!("login stop receipt disappeared"))?;
            if !receipt.candidate_removed {
                self.discard_stopped_auth_candidate(provider, request, receipt.process_started)?;
            }
        }
        Ok(())
    }

    /// Absence is unknown. Never creates state or infers exit from a missing home.
    pub fn auth_candidate_stop(&self, provider: &str, request: &str) -> Result<Option<Receipt>> {
        super::super::validate_plugin_id(provider)?;
        super::super::validate_auth_candidate_id(request)?;
        let root = self.auth_provider_root(provider);
        let relative = format!("login-receipts/{request}.json");
        match fs::symlink_metadata(root.join(&relative)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            result => ensure!(result?.is_file(), "invalid login stop receipt"),
        }
        let receipt: Receipt = serde_json::from_slice(
            &super::super::native_identity::read_login_file(&root, &relative, 16 * 1024)?,
        )?;
        ensure!(
            receipt.schema == 1
                && receipt.account == self.account_id
                && receipt.provider == provider
                && receipt.request == request
                && receipt.leader_exited == receipt.process_started
                && (!receipt.process_tree_exited || receipt.leader_exited),
            "login stop receipt scope mismatch"
        );
        Ok(Some(receipt))
    }

    pub(super) fn discard_stopped_auth_candidate(
        &self,
        provider: &str,
        request: &str,
        process_started: bool,
    ) -> Result<()> {
        let mut receipt = Receipt {
            schema: 1,
            account: self.account_id.clone(),
            provider: provider.into(),
            request: request.into(),
            process_started,
            leader_exited: process_started,
            candidate_removed: false,
            process_tree_exited: false,
        };
        {
            let _guard = self.auth_candidates.lock();
            if let Some(old) = self.auth_candidate_stop(provider, request)? {
                receipt.candidate_removed = old.candidate_removed;
                receipt.process_tree_exited = old.process_tree_exited;
                ensure!(old == receipt, "login stop receipt conflict");
            } else {
                receipt.process_tree_exited =
                    process_started && self.auth_candidate_tree_exited(provider, request)?;
            }
            self.persist_candidate_stop(&receipt)?;
        }
        // The durable receipt fences reuse before the directory can disappear.
        // A retry repeats cleanup even if it was previously recorded, so a
        // recreated private home is never silently accepted as clean.
        self.discard_auth_candidate(provider, request)?;
        let _guard = self.auth_candidates.lock();
        let root = self.auth_candidate_root(provider);
        if root.exists() {
            fs::File::open(&root)?.sync_all()?;
        }
        receipt.candidate_removed = true;
        self.persist_candidate_stop(&receipt)
    }

    fn persist_candidate_stop(&self, receipt: &Receipt) -> Result<()> {
        super::super::validate_plugin_id(&receipt.provider)?;
        super::super::validate_auth_candidate_id(&receipt.request)?;
        let root = self.auth_provider_root(&receipt.provider);
        crate::provider_account::prepare_private_directory(&root)?;
        let directory = root.join("login-receipts");
        crate::provider_account::prepare_private_directory(&directory)?;
        super::super::atomic_write(
            &directory.join(format!("{}.json", receipt.request)),
            &serde_json::to_vec(receipt)?,
            0o600,
        )?;
        fs::File::open(&directory)?.sync_all()?;
        fs::File::open(&root)?.sync_all()?;
        Ok(())
    }
}
