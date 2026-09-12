//! Reader floor for Machine-local telemetry binding authority. The enclosing
//! Plugin journal owns the process lock. Opening/querying NEVER writes this
//! namespace, adopts private policy, starts an exporter or replays an intent.
//! The finite writer is staged behind a closed admission gate, not a wire API.

use super::*;
use crate::machine_protocol::telemetry_binding::{
    BindingChange, BindingDigest, BindingObservation, BindingObservationSnapshot, BindingOutcome,
    BindingReceipt, BindingSnapshot, BindingStep, BindingUnavailable, binding_digest,
    valid_service,
};

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
}

impl Ledger {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1
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
            self.receipts.len() <= MAX_BINDING_RECORDS,
            "telemetry binding journal capacity exceeded"
        );
        let mut current = BindingSnapshot::initial();
        let mut ids = BTreeSet::new();
        let mut applied: BTreeMap<String, &BindingReceipt> = BTreeMap::new();
        let mut unresolved = false;
        for receipt in &self.receipts {
            ensure!(
                !unresolved
                    && receipt.matches(&receipt.step)
                    && receipt.step.service_id == self.service_id
                    && receipt.step.machine_id == self.machine_id
                    && ids.insert(&receipt.step.operation_id),
                "invalid telemetry binding receipt chain"
            );
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
}

impl Bindings {
    #[allow(clippy::verbose_bit_mask)] // Keep the conventional Unix group/other permission mask.
    pub(super) fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            path: path.to_owned(),
            state: parking_lot::Mutex::new(BindingState {
                ledger: Self::read(path)?,
                poisoned: false,
                writer: false,
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
        let state = self.state.lock();
        ensure!(
            !state.poisoned && state.ledger.is_none(),
            "telemetry binding authority is reader-only"
        );
        Ok(())
    }

    pub(super) fn query(&self, step: &BindingStep) -> BindingObservation {
        Self::lookup(&self.state.lock(), step)
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

// Production compiles the same finite transaction exercised by fixtures, but
// no command/coordinator can enable it in this reader-floor release.
#[cfg_attr(not(test), allow(dead_code))]
mod writer;

fn unavailable(reason: BindingUnavailable) -> BindingObservation {
    BindingObservation::Unavailable { reason }
}

impl MachinePluginStore {
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
