//! Per-attempt installation evidence within the one Machine operation journal.
//! The parent owns the process lock; the store owns lifecycle serialization.
//! Reopening retains pending evidence verbatim and never resumes an installer.

use super::*;
use crate::machine_protocol::plugin_install::{
    InstallLookup, InstallOutcome, InstallPhase, InstallReceipt, InstallStep, InstallUnavailable,
};

pub(super) const DIRECTORY: &str = "install-attempts-v1";
const MAX_ATTEMPTS: usize = 4096;
const MAX_ATTEMPT_BYTES: u64 = 8192;

#[cfg(test)]
type WriteHook = Box<dyn Fn(&InstallReceipt) -> Result<()> + Send + Sync>;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u16,
    receipt: InstallReceipt,
    evidence_digest: String,
}

struct State {
    receipts: BTreeMap<String, InstallReceipt>,
    present: bool,
    poisoned: bool,
}

pub(in crate::machine_plugins) struct Attempts {
    root: PathBuf,
    owner: std::sync::Arc<()>,
    state: parking_lot::Mutex<State>,
    #[cfg(test)]
    before_write: parking_lot::Mutex<Option<WriteHook>>,
}

/// An in-process admitted continuation, not a receipt codec or replay handle.
pub(in crate::machine_plugins) struct PendingAttempt {
    receipt: InstallReceipt,
    owner: std::sync::Arc<()>,
}

impl PendingAttempt {
    pub(in crate::machine_plugins) fn receipt(&self) -> &InstallReceipt {
        &self.receipt
    }
}

impl Attempts {
    pub(super) fn open(root: PathBuf) -> Result<Self> {
        let present = match root.symlink_metadata() {
            Ok(metadata) => {
                ensure!(metadata.is_dir(), "invalid install attempt directory");
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        let mut receipts = BTreeMap::new();
        let mut pending = BTreeSet::new();
        if present {
            for entry in fs::read_dir(&root)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_str().context("invalid install attempt entry")?;
                if name.starts_with('.') && name.ends_with(".partial") {
                    continue;
                }
                ensure!(
                    receipts.len() < MAX_ATTEMPTS,
                    "install attempt capacity exceeded"
                );
                let file = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                    .open(entry.path())?;
                ensure!(file.metadata()?.is_file(), "invalid install attempt record");
                let mut bytes = Vec::new();
                file.take(MAX_ATTEMPT_BYTES + 1).read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() as u64 <= MAX_ATTEMPT_BYTES,
                    "install attempt too large"
                );
                let record: Record = serde_json::from_slice(&bytes)
                    .map_err(|_| anyhow::anyhow!("invalid install attempt evidence"))?;
                let receipt = record.receipt;
                ensure!(
                    record.schema == 1
                        && receipt.matches(&receipt.step)
                        && record.evidence_digest == digest(&serde_json::to_vec(&receipt)?),
                    "install attempt integrity failure"
                );
                let key = receipt.step.key()?;
                ensure!(
                    name == format!("{key}.json"),
                    "install attempt identity mismatch"
                );
                if receipt.outcome.fenced() {
                    ensure!(
                        pending.insert(receipt.step.plugin_id.clone()),
                        "conflicting install attempts"
                    );
                }
                ensure!(
                    receipts.insert(key, receipt).is_none(),
                    "duplicate install attempt"
                );
            }
        }
        Ok(Self {
            root,
            owner: std::sync::Arc::new(()),
            #[cfg(test)]
            before_write: parking_lot::Mutex::new(None),
            state: parking_lot::Mutex::new(State {
                receipts,
                present,
                poisoned: false,
            }),
        })
    }

    fn lookup(state: &State, step: &InstallStep) -> InstallLookup {
        let Ok(key) = step.key() else {
            return unavailable(InstallUnavailable::InvalidRequest);
        };
        if state.poisoned {
            return unavailable(InstallUnavailable::Storage);
        }
        match state.receipts.get(&key) {
            None => InstallLookup::NotFound {},
            Some(receipt) if receipt.matches(step) => InstallLookup::Found {
                receipt: Box::new(receipt.clone()),
            },
            Some(_) => unavailable(InstallUnavailable::IdentityConflict),
        }
    }

    pub(in crate::machine_plugins) fn query(&self, step: &InstallStep) -> InstallLookup {
        Self::lookup(&self.state.lock(), step)
    }

    pub(in crate::machine_plugins) fn has_capacity(&self) -> bool {
        let state = self.state.lock();
        !state.poisoned && state.receipts.len() < MAX_ATTEMPTS
    }

    pub(in crate::machine_plugins) fn tracked(&self, plugin: &str) -> bool {
        self.state
            .lock()
            .receipts
            .values()
            .any(|receipt| receipt.step.plugin_id == plugin)
    }

    pub(super) fn validate_installations(
        &self,
        installations: &installations::Installations,
    ) -> Result<()> {
        let state = self.state.lock();
        ensure!(
            !state.present || installations.requires_cas(),
            "install attempts lost installation authority"
        );
        for receipt in state.receipts.values() {
            let needs_slot = receipt.step.expected.revision().is_some()
                || matches!(
                    receipt.outcome,
                    InstallOutcome::Applied { .. }
                        | InstallOutcome::Pending {
                            phase: InstallPhase::ProjectingAuthentication
                        }
                        | InstallOutcome::Unknown {
                            phase: InstallPhase::ProjectingAuthentication,
                            ..
                        }
                );
            ensure!(
                !needs_slot || installations.tracked(&receipt.step.plugin_id),
                "install attempt lost installation slot"
            );
        }
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::machine_plugins) fn before_write_for_test(&self, hook: WriteHook) {
        *self.before_write.lock() = Some(hook);
    }

    pub(in crate::machine_plugins) fn ensure_legacy_allowed(&self) -> Result<()> {
        let state = self.state.lock();
        ensure!(
            !state.present && !state.poisoned,
            "installation requires durable attempts"
        );
        Ok(())
    }

    pub(in crate::machine_plugins) fn ensure_unfenced(&self, plugin: &str) -> Result<()> {
        let state = self.state.lock();
        ensure!(
            !state.poisoned && !Self::fenced(&state, plugin),
            "Plugin requires install attempt reconciliation"
        );
        Ok(())
    }

    fn fenced(state: &State, plugin: &str) -> bool {
        state
            .receipts
            .values()
            .any(|receipt| receipt.step.plugin_id == plugin && receipt.outcome.fenced())
    }

    /// The caller supplies fresh authority and checks target CAS under the
    /// lifecycle lock. Directory presence closes legacy entry points forever.
    pub(in crate::machine_plugins) fn begin(&self, step: &InstallStep) -> Result<PendingAttempt> {
        let key = step.key()?;
        let mut state = self.state.lock();
        ensure!(
            Self::lookup(&state, step) == (InstallLookup::NotFound {})
                && state.receipts.len() < MAX_ATTEMPTS
                && !Self::fenced(&state, &step.plugin_id),
            "install attempt admission changed"
        );
        let receipt = InstallReceipt {
            step: step.clone(),
            request_digest: step.request_digest()?,
            outcome: InstallOutcome::Pending {
                phase: InstallPhase::Prepared,
            },
        };
        self.persist(&mut state, &key, &receipt)?;
        Ok(PendingAttempt {
            receipt,
            owner: std::sync::Arc::clone(&self.owner),
        })
    }

    pub(in crate::machine_plugins) fn advance(
        &self,
        pending: &mut PendingAttempt,
        outcome: InstallOutcome,
    ) -> Result<()> {
        ensure!(
            std::sync::Arc::ptr_eq(&self.owner, &pending.owner)
                && outcome.follows(&pending.receipt.outcome),
            "install continuation is not owned by this process or phase"
        );
        let mut next = pending.receipt.clone();
        next.outcome = outcome;
        ensure!(
            next.matches(&next.step),
            "invalid install completion evidence"
        );
        let key = next.step.key()?;
        let mut state = self.state.lock();
        ensure!(
            !state.poisoned && state.receipts.get(&key) == Some(&pending.receipt),
            "install attempt compare-and-swap failed"
        );
        self.persist(&mut state, &key, &next)?;
        pending.receipt = next;
        Ok(())
    }

    fn persist(&self, state: &mut State, key: &str, receipt: &InstallReceipt) -> Result<()> {
        let result = (|| {
            ensure!(!state.poisoned, "install attempt storage unavailable");
            #[cfg(test)]
            if let Some(hook) = &*self.before_write.lock() {
                hook(receipt)?;
            }
            if !state.present {
                // Poisoning must also fence a failed first mkdir/parent flush.
                state.present = true;
                fs::DirBuilder::new().mode(0o700).create(&self.root)?;
                fs::File::open(self.root.parent().context("missing journal parent")?)?
                    .sync_all()?;
            }
            let record = Record {
                schema: 1,
                evidence_digest: digest(&serde_json::to_vec(receipt)?),
                receipt: receipt.clone(),
            };
            let bytes = serde_json::to_vec(&record)?;
            ensure!(
                bytes.len() as u64 <= MAX_ATTEMPT_BYTES,
                "install attempt too large"
            );
            atomic_write(&self.root.join(format!("{key}.json")), &bytes, 0o600)?;
            fs::File::open(&self.root)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            state.poisoned = true;
        } else {
            state.receipts.insert(key.to_owned(), receipt.clone());
        }
        result
    }
}

fn unavailable(reason: InstallUnavailable) -> InstallLookup {
    InstallLookup::Unavailable { reason }
}

#[cfg(test)]
mod tests;
