//! Reader floor for Machine-local telemetry binding authority. The enclosing
//! Plugin journal owns the process lock. Opening/querying NEVER writes this
//! namespace, adopts private policy, starts an exporter or replays an intent.
//! The finite wire command cannot open the independently closed admission gate.

use super::*;
use crate::machine_protocol::telemetry_binding::{
    BindingChange, BindingDigest, BindingNamespace, BindingObservation, BindingObservationSnapshot,
    BindingOutcome, BindingReceipt, BindingSnapshot, BindingStep, BindingUnavailable,
    binding_digest, valid_service,
};
use crate::machine_protocol::telemetry_recovery::RecoveryReceipt;

pub(super) const FILE: &str = "telemetry-bindings-v1.json";
const MAX_BINDING_RECORDS: usize = 1024;
const MAX_BINDING_BYTES: u64 = 4 * 1024 * 1024;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LedgerFile {
    ledger: Ledger,
    evidence_digest: BindingDigest,
}

// One finite Machine export slot, scoped to its pinned Service. Receipt and
// binding-state updates belong in one atomic durable replacement, not separate
// files whose agreement is guessed after restart. No endpoint or token here.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    schema: u16,
    service_id: String,
    machine_id: String,
    current: BindingSnapshot,
    receipts: Vec<BindingReceipt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    resolutions: Vec<RecoveryReceipt>,
}

impl Ledger {
    fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.schema, 1 | 2)
                && valid_service(&self.service_id)
                && !self.machine_id.is_empty()
                && self.machine_id.len() <= 128
                && self
                    .machine_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
            "invalid telemetry binding journal owner"
        );
        ensure!(
            self.receipts.len() <= MAX_BINDING_RECORDS
                && self.resolutions.len() <= self.receipts.len()
                && (self.schema == 1) == self.resolutions.is_empty(),
            "telemetry binding journal capacity exceeded"
        );
        let mut current = BindingSnapshot::initial();
        let mut ids = BTreeSet::new();
        let mut applied: BTreeMap<String, &BindingReceipt> = BTreeMap::new();
        let mut unresolved = false;
        for (index, receipt) in self.receipts.iter().enumerate() {
            ensure!(
                !unresolved
                    && receipt.matches(&receipt.step)
                    && receipt.step.service_id == self.service_id
                    && receipt.step.machine_id == self.machine_id
                    && ids.insert(&receipt.step.operation_id),
                "invalid telemetry binding receipt chain"
            );
            if let Some(namespace) = receipt.step.expected_namespace {
                ensure!(
                    receipt.step.expected == current
                        && (namespace != BindingNamespace::Unmanaged || index == 0),
                    "binding namespace predecessor mismatch"
                );
            }
            if matches!(receipt.outcome, BindingOutcome::Rejected { .. }) {
                continue;
            }
            ensure!(
                receipt.step.expected == current,
                "telemetry binding predecessor mismatch"
            );
            if let BindingChange::Restore {
                forward_request_digest,
                selection,
                ..
            } = &receipt.step.change
            {
                let key: String = forward_request_digest.clone().into();
                let forward = applied
                    .get(&key)
                    .context("telemetry binding restoration source is missing")?;
                ensure!(
                    matches!(&forward.outcome, BindingOutcome::Applied { after } if *after == current)
                        && forward.step.expected.selection == *selection,
                    "telemetry binding restoration conflicts with later state"
                );
            }
            match &receipt.outcome {
                BindingOutcome::Applied { after } => {
                    current = after.clone();
                    applied.insert(String::from(receipt.request_digest.clone()), receipt);
                }
                BindingOutcome::Prepared {} | BindingOutcome::Unknown {} => unresolved = true,
                BindingOutcome::Rejected { .. } => unreachable!(),
            }
        }
        ensure!(self.current == current, "telemetry binding head mismatch");
        let mut resolved = BTreeSet::new();
        let mut resolution_ids = BTreeSet::new();
        let mut previous = None;
        for resolution in &self.resolutions {
            resolution.validate()?;
            let (index, receipt) = self
                .receipts
                .iter()
                .enumerate()
                .find(|(_, receipt)| {
                    receipt.step.operation_id == resolution.request.step.operation_id
                })
                .context("missing resolved Machine binding")?;
            ensure!(
                resolved.insert(&receipt.step.operation_id)
                    && resolution_ids.insert(&resolution.request.resolution_id)
                    && previous.is_none_or(|previous| previous < index)
                    && receipt == &resolution.binding,
                "Machine binding resolution chain changed"
            );
            previous = Some(index);
        }
        self.current.validate()
    }
}

pub(in crate::machine_plugins) struct Bindings {
    path: PathBuf,
    state: parking_lot::Mutex<BindingState>,
}

struct BindingState {
    ledger: Option<Ledger>,
    poisoned: bool,
    // There is intentionally no production setter. Service coordination and
    // accepted live/cold readers must precede any managed-namespace creation.
    writer: bool,
    recovery_writer: bool,
    /// Process-local proof of validated reopen, never reconstructed from a
    /// live failed write or a serialized request. Unknown/schema-one stay fenced.
    reopened_prepared: Option<BindingDigest>,
}

impl Bindings {
    /// Called under the Machine lifecycle lock before each managed emission.
    /// Re-read the owned evidence: a missing/tampered file must not make cached
    /// authority usable, nor may restoring its bytes revive this process.
    pub(in crate::machine_plugins) fn ensure_export_current(
        &self,
        request: &crate::machine_protocol::telemetry_export::ExportAttempt,
    ) -> Result<()> {
        let mut state = self.state.lock();
        ensure!(
            self.retained_current(&mut state),
            "telemetry binding evidence is unavailable"
        );
        let ledger = state
            .ledger
            .as_ref()
            .context("managed telemetry binding is absent")?;
        ensure!(
            ledger.service_id == request.service_id
                && ledger.machine_id == request.machine_id
                && ledger.current == request.binding
                && ledger.current.selection.is_some()
                && !ledger.receipts.last().is_some_and(|r| matches!(
                    r.outcome,
                    BindingOutcome::Prepared {} | BindingOutcome::Unknown {}
                )),
            "managed telemetry binding is not current"
        );
        Ok(())
    }

    #[allow(clippy::verbose_bit_mask)] // Keep the conventional Unix group/other permission mask.
    pub(super) fn open(path: &Path) -> Result<Self> {
        let ledger = Self::read(path)?;
        let reopened_prepared = ledger
            .as_ref()
            .and_then(|ledger| ledger.receipts.last())
            .filter(|receipt| {
                receipt.step.schema == 2 && matches!(receipt.outcome, BindingOutcome::Prepared {})
            })
            .map(|receipt| receipt.request_digest.clone());
        Ok(Self {
            path: path.to_owned(),
            state: parking_lot::Mutex::new(BindingState {
                ledger,
                poisoned: false,
                writer: false,
                recovery_writer: false,
                reopened_prepared,
            }),
        })
    }

    #[allow(clippy::verbose_bit_mask)]
    fn read(path: &Path) -> Result<Option<Ledger>> {
        use std::os::unix::fs::MetadataExt as _;
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(error).context("opening telemetry binding evidence"),
        };
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.mode() & 0o077 == 0
                && metadata.nlink() == 1
                && metadata.uid() == rustix::process::geteuid().as_raw()
                && metadata.len() <= MAX_BINDING_BYTES,
            "telemetry binding evidence must be bounded, private and owned"
        );
        let mut bytes = Vec::new();
        (&file)
            .take(MAX_BINDING_BYTES + 1)
            .read_to_end(&mut bytes)?;
        let after = file.metadata()?;
        ensure!(
            bytes.len() as u64 <= MAX_BINDING_BYTES
                && metadata.len() == after.len()
                && metadata.mode() == after.mode()
                && metadata.nlink() == after.nlink()
                && metadata.uid() == after.uid()
                && metadata.ctime() == after.ctime()
                && metadata.ctime_nsec() == after.ctime_nsec(),
            "telemetry binding evidence changed during read"
        );
        let record: LedgerFile = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("invalid telemetry binding evidence"))?;
        ensure!(
            record.evidence_digest == binding_digest(&serde_json::to_vec(&record.ledger)?),
            "telemetry binding evidence integrity failure"
        );
        record.ledger.validate()?;
        Ok(Some(record.ledger))
    }

    pub(in crate::machine_plugins) fn ensure_legacy_allowed(&self) -> Result<()> {
        let mut state = self.state.lock();
        ensure!(
            self.retained_current(&mut state) && state.ledger.is_none(),
            "telemetry binding authority is reader-only"
        );
        Ok(())
    }

    pub(super) fn query(&self, step: &BindingStep) -> BindingObservation {
        let mut state = self.state.lock();
        if !self.retained_current(&mut state) {
            return unavailable(BindingUnavailable::Storage);
        }
        Self::lookup(&state, step)
    }

    fn retained_current(&self, state: &mut BindingState) -> bool {
        if state.poisoned || !Self::read(&self.path).is_ok_and(|retained| retained == state.ledger)
        {
            state.poisoned = true;
            return false;
        }
        true
    }

    fn lookup(state: &BindingState, step: &BindingStep) -> BindingObservation {
        if state.poisoned {
            return unavailable(BindingUnavailable::Storage);
        }
        let Ok(request_digest) = step.request_digest() else {
            return unavailable(BindingUnavailable::InvalidRequest);
        };
        let Some(ledger) = &state.ledger else {
            return BindingObservation::Observed {
                snapshot: Box::new(BindingObservationSnapshot {
                    request_digest,
                    receipt: None,
                    current: None,
                    unresolved: false,
                }),
            };
        };
        if ledger.service_id != step.service_id || ledger.machine_id != step.machine_id {
            return unavailable(BindingUnavailable::WrongOwner);
        }
        let receipt = ledger
            .receipts
            .iter()
            .find(|receipt| receipt.step.operation_id == step.operation_id);
        if receipt.is_some_and(|receipt| !receipt.matches(step)) {
            return unavailable(BindingUnavailable::IdentityConflict);
        }
        BindingObservation::Observed {
            snapshot: Box::new(BindingObservationSnapshot {
                request_digest,
                receipt: receipt.cloned().map(Box::new),
                current: Some(ledger.current.clone()),
                unresolved: ledger
                    .receipts
                    .last()
                    .is_some_and(BindingReceipt::unresolved),
            }),
        }
    }
}

// Protocol support does not enable the independent, currently closed writer.
mod recovery;
mod writer;

fn unavailable(reason: BindingUnavailable) -> BindingObservation {
    BindingObservation::Unavailable { reason }
}

impl MachinePluginStore {
    #[cfg(test)]
    pub(crate) fn enable_binding_writer_for_test(&self) {
        self.operations.telemetry_bindings.state.lock().writer = true;
    }

    #[cfg(test)]
    pub(crate) fn enable_binding_recovery_for_test(&self) {
        self.operations
            .telemetry_bindings
            .state
            .lock()
            .recovery_writer = true;
    }

    pub(crate) async fn telemetry_binding_observation(
        &self,
        step: &BindingStep,
        service: Option<&str>,
        machine: &str,
    ) -> BindingObservation {
        if step.validate().is_err() {
            return unavailable(BindingUnavailable::InvalidRequest);
        }
        if service != Some(step.service_id.as_str()) || machine != step.machine_id {
            return unavailable(BindingUnavailable::WrongOwner);
        }
        let Ok(_lifecycle) =
            tokio::time::timeout(Duration::from_secs(10), self.lifecycle.lock()).await
        else {
            return unavailable(BindingUnavailable::Storage);
        };
        // A poisoned enclosing journal cannot supply a trustworthy snapshot.
        if self.operations.state.lock().poisoned {
            return unavailable(BindingUnavailable::Storage);
        }
        self.operations.telemetry_bindings.query(step)
    }
}

#[cfg(test)]
mod tests;
