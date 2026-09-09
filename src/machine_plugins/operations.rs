//! `MachinePluginStore`'s private uninstall journal. Never an `OTel` spool or a
//! second Plugin lifecycle. One process lock and the parent's lifecycle lock
//! serialize effects; evidence alone never authorizes replay on reopen.

use super::*;
use crate::machine_protocol::plugin_step::{
    StepLookup, StepObservation, StepOutcome, StepReceipt, StepRejection, StepUnavailable,
    StepUncertainty, UninstallStep, digest,
};

pub(super) mod installations;

const MAX_RECORDS: usize = 4096;
const MAX_RECORD_BYTES: u64 = 8192;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u16,
    receipt: StepReceipt,
    evidence_digest: String,
}

struct JournalState {
    receipts: BTreeMap<String, StepReceipt>,
    poisoned: bool,
}

pub(super) struct Journal {
    root: PathBuf,
    _owner: OwnerLock,
    state: parking_lot::Mutex<JournalState>,
    pub(super) installations: installations::Installations,
}

struct OwnerLock(fs::File);

impl Drop for OwnerLock {
    fn drop(&mut self) {
        // fork/dup temporarily shares an open-file description even with
        // CLOEXEC. Close alone can leave its flock held by an unrelated child.
        // A live store keeps this guard; graceful release explicitly unlocks.
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

impl Journal {
    pub(super) fn open(state_dir: &Path) -> Result<Self> {
        let root = state_dir.join("plugin-operations");
        fs::create_dir_all(&root)?;
        ensure!(
            root.symlink_metadata()?.file_type().is_dir(),
            "invalid Machine journal directory"
        );
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        fs::File::open(state_dir)?.sync_all()?;
        let owner = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(root.join("owner.lock"))?;
        fs2::FileExt::try_lock_exclusive(&owner).context("Machine Plugin journal already owned")?;
        let owner = OwnerLock(owner);
        let mut receipts = BTreeMap::new();
        let mut slots = BTreeSet::new();
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().context("invalid Machine journal entry")?;
            if name == "owner.lock"
                || name == installations::DIRECTORY
                || (name.starts_with('.') && name.ends_with(".partial"))
            {
                continue;
            }
            ensure!(
                receipts.len() < MAX_RECORDS,
                "Machine journal capacity exceeded"
            );
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(entry.path())?;
            ensure!(file.metadata()?.is_file(), "invalid Machine journal record");
            let mut bytes = Vec::new();
            file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= MAX_RECORD_BYTES,
                "Machine journal record too large"
            );
            let record: Record = serde_json::from_slice(&bytes)
                .map_err(|_| anyhow::anyhow!("invalid Machine journal evidence"))?;
            let receipt = record.receipt;
            ensure!(
                record.schema == 1
                    && receipt.matches(&receipt.step)
                    && record.evidence_digest == digest(&serde_json::to_vec(&receipt)?),
                "Machine journal evidence integrity failure"
            );
            let key = receipt.step.key()?;
            ensure!(
                name == format!("{key}.json"),
                "Machine journal identity mismatch"
            );
            if matches!(receipt.outcome, StepOutcome::Unknown { .. }) {
                ensure!(
                    slots.insert(receipt.step.plugin_id.clone()),
                    "conflicting Machine recovery slots"
                );
            }
            ensure!(
                receipts.insert(key, receipt).is_none(),
                "duplicate Machine step"
            );
        }
        Ok(Self {
            installations: installations::Installations::open(root.join(installations::DIRECTORY))?,
            root,
            _owner: owner,
            state: parking_lot::Mutex::new(JournalState {
                receipts,
                poisoned: false,
            }),
        })
    }

    fn persist(&self, key: &str, receipt: &StepReceipt) -> Result<()> {
        let record = Record {
            schema: 1,
            evidence_digest: digest(&serde_json::to_vec(receipt)?),
            receipt: receipt.clone(),
        };
        let bytes = serde_json::to_vec(&record)?;
        ensure!(
            bytes.len() as u64 <= MAX_RECORD_BYTES,
            "Machine receipt too large"
        );
        // atomic_write flushes/closes the file before rename. The directory
        // flush is essential: an acknowledged rename must survive power loss.
        atomic_write(&self.root.join(format!("{key}.json")), &bytes, 0o600)?;
        fs::File::open(&self.root)?.sync_all()?;
        Ok(())
    }

    fn lookup(state: &JournalState, step: &UninstallStep) -> StepLookup {
        let Ok(key) = step.key() else {
            return unavailable(StepUnavailable::InvalidRequest);
        };
        if state.poisoned {
            return unavailable(StepUnavailable::Storage);
        }
        match state.receipts.get(&key) {
            None => StepLookup::NotFound {},
            Some(receipt) if receipt.matches(step) => StepLookup::Found {
                receipt: Box::new(receipt.clone()),
            },
            Some(_) => unavailable(StepUnavailable::IdentityConflict),
        }
    }

    pub(super) fn query(&self, step: &UninstallStep) -> StepLookup {
        Self::lookup(&self.state.lock(), step)
    }

    pub(super) fn ensure_unfenced(&self, plugin: &str) -> Result<()> {
        self.installations.ensure_unfenced(plugin)?;
        let state = self.state.lock();
        ensure!(
            !state.poisoned
                && !state.receipts.values().any(|receipt| {
                    receipt.step.plugin_id == plugin
                        && matches!(receipt.outcome, StepOutcome::Unknown { .. })
                }),
            "Plugin requires Machine operation reconciliation"
        );
        Ok(())
    }

    pub(super) fn ensure_legacy_allowed(&self, plugin: &str) -> Result<()> {
        ensure!(
            !self.installations.requires_cas(),
            "Plugin lifecycle requires installation CAS"
        );
        let state = self.state.lock();
        // Once a slot has durable operation authority, an older Controller
        // cannot bypass its receipts through an unjournaled mutation.
        ensure!(
            !state.poisoned && !state.receipts.values().any(|r| r.step.plugin_id == plugin),
            "Plugin lifecycle requires the durable Machine step protocol"
        );
        Ok(())
    }

    // Artifact verification can take time; the two wall-clock reads intentionally differ.
    #[allow(clippy::same_functions_in_if_condition)]
    fn execute(
        &self,
        step: &UninstallStep,
        precondition: impl FnOnce() -> bool,
        effect: impl FnOnce() -> Result<()>,
    ) -> StepLookup {
        let mut state = self.state.lock();
        match Self::lookup(&state, step) {
            StepLookup::NotFound {} => {}
            found => return found, // including Unknown: never replay the effect.
        }
        if state.receipts.len() >= MAX_RECORDS {
            return unavailable(StepUnavailable::Capacity);
        }
        if state.receipts.values().any(|r| {
            r.step.plugin_id == step.plugin_id && matches!(r.outcome, StepOutcome::Unknown { .. })
        }) {
            return unavailable(StepUnavailable::SlotFenced);
        }
        let key = step.key().expect("validated by lookup");
        let mut receipt = StepReceipt {
            step: step.clone(),
            request_digest: step.request_digest().expect("validated by lookup"),
            // Intent is already uncertain evidence on a new process. A crash
            // before the effect is indistinguishable from a crash after it.
            outcome: StepOutcome::Unknown {
                reason: StepUncertainty::Interrupted,
            },
        };
        state.receipts.insert(key.clone(), receipt.clone());
        if self.persist(&key, &receipt).is_err() {
            state.poisoned = true;
            return unavailable(StepUnavailable::Storage);
        }
        receipt.outcome = if chrono::Utc::now().timestamp_millis() > step.expires_at_ms {
            StepOutcome::Rejected {
                reason: StepRejection::Expired,
            }
        } else if !precondition() {
            StepOutcome::Rejected {
                reason: StepRejection::TargetChanged,
            }
        } else if chrono::Utc::now().timestamp_millis() > step.expires_at_ms {
            StepOutcome::Rejected {
                reason: StepRejection::Expired,
            }
        } else if effect().is_ok() {
            StepOutcome::Applied {}
        } else {
            StepOutcome::Unknown {
                reason: StepUncertainty::EffectFailure,
            }
        };
        if self.persist(&key, &receipt).is_err() {
            state.poisoned = true;
            return unavailable(StepUnavailable::Storage);
        }
        state.receipts.insert(key, receipt.clone());
        StepLookup::Found {
            receipt: Box::new(receipt),
        }
    }
}

fn unavailable(reason: StepUnavailable) -> StepLookup {
    StepLookup::Unavailable { reason }
}

impl MachinePluginStore {
    pub(crate) async fn uninstall_step(
        &self,
        step: &UninstallStep,
        service: Option<&str>,
        machine: &str,
        admission_enabled: bool,
        query_only: bool,
    ) -> StepObservation {
        let _lifecycle = self.lifecycle.lock().await;
        let result = if step.validate().is_err() {
            unavailable(StepUnavailable::InvalidRequest)
        } else if service != Some(step.service_id.as_str()) || machine != step.machine_id {
            unavailable(StepUnavailable::WrongOwner)
        } else if query_only {
            let found = self.operations.query(step);
            if found == (StepLookup::NotFound {})
                && self.operations.ensure_unfenced(&step.plugin_id).is_err()
            {
                unavailable(StepUnavailable::SlotFenced)
            } else if found == (StepLookup::NotFound {})
                && step.installation_revision.is_some()
                && !self.step_target_matches(step).unwrap_or(false)
            {
                // Read-only preflight can reject a stale preview before the
                // Service stops workers; execution still repeats the CAS.
                unavailable(StepUnavailable::InvalidRequest)
            } else {
                found
            }
        } else if !admission_enabled {
            unavailable(StepUnavailable::ReaderOnly)
        } else if self.operations.installations.requires_cas()
            && step.installation_revision.is_none()
        {
            // Old receipts remain queryable, but a fresh digest-only command
            // cannot remove an incarnation-tracked installation.
            match self.operations.query(step) {
                StepLookup::NotFound {} => unavailable(StepUnavailable::InvalidRequest),
                found => found,
            }
        } else if !self.operations.installations.admits(step) {
            unavailable(StepUnavailable::ReaderOnly)
        } else if self
            .operations
            .installations
            .ensure_unfenced(&step.plugin_id)
            .is_err()
        {
            match self.operations.query(step) {
                StepLookup::NotFound {} => unavailable(StepUnavailable::SlotFenced),
                found => found,
            }
        } else {
            self.operations.execute(
                step,
                || self.step_target_matches(step).unwrap_or(false),
                || {
                    let pending = self.operations.installations.begin(
                        &step.plugin_id,
                        Some(&step.generation_digest),
                        None,
                        installations::Effect::Uninstall,
                        Some(step.request_digest()?),
                    )?;
                    self.uninstall_inner(&step.plugin_id, &step.generation_digest)?;
                    // Flush every removed directory entry before acknowledging.
                    fs::File::open(self.plugin_root(&step.plugin_id))?.sync_all()?;
                    let auth = self.auth_provider_root(&step.plugin_id);
                    if auth.exists() {
                        fs::File::open(auth)?.sync_all()?;
                    }
                    self.operations.installations.finish(pending)?;
                    Ok(())
                },
            )
        };
        StepObservation {
            admission_enabled: admission_enabled
                && service.is_some()
                && self.operations.installations.admits(step),
            result,
        }
    }

    fn step_target_matches(&self, step: &UninstallStep) -> Result<bool> {
        let Some(active) = self.inventory_one(&step.plugin_id)? else {
            return Ok(false);
        };
        if active.state != PluginInstallationState::Active
            || active.plugin_version != step.plugin_version
            || active.generation_digest != step.generation_digest
            || active.contract_fingerprint != step.contract_fingerprint
            || active.installation_revision != step.installation_revision
        {
            return Ok(false);
        }
        let (package, _, _) =
            self.verified_plugin_generation(&step.plugin_id, &step.generation_digest)?;
        Ok(package.manifest.id == step.plugin_id
            && package.manifest.version == step.plugin_version
            && package.contract_fingerprint == step.contract_fingerprint)
    }
}

#[cfg(test)]
mod tests;
