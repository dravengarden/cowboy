//! Read-only, point-in-time recovery evidence. A matching removal is NOT a
//! recovery grant, a verified retained executable, or a restored session.

use super::installation_revision::InstallationRevision;
#[cfg(any(feature = "full", test))]
use super::plugin_step::{StepOutcome, UninstallStep};
use super::plugin_step::{StepReceipt, StepRejection, StepUnavailable};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationUnavailable {
    Storage,
    ActiveLinkMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstallationEvidence {
    Untracked {},
    Installed {
        revision: InstallationRevision,
        generation_digest: String,
    },
    Removed {
        revision: InstallationRevision,
        previous_revision: InstallationRevision,
        uninstall_request_digest: String,
    },
    Pending {
        revision: InstallationRevision,
    },
    Unavailable {
        reason: InstallationUnavailable,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverySnapshot {
    /// Binds even a missing receipt to the complete original request.
    pub request_digest: String,
    pub receipt: Option<Box<StepReceipt>>,
    pub installation: InstallationEvidence,
    /// Includes unresolved OTHER steps on this slot, not just this request.
    pub slot_fenced: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryObservation {
    Observed { snapshot: Box<RecoverySnapshot> },
    Unavailable { reason: StepUnavailable },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryUncertainty {
    MissingForwardReceipt,
    ForwardOutcome,
    InstallationPending,
    InstallationUnavailable,
}

/// Derived by the Controller from validated evidence, never accepted from a
/// Machine's declaration of recovery readiness. Every variant is observational.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RecoveryBasis {
    MatchingRemoval {
        tombstone_revision: InstallationRevision,
    },
    ForwardRejected {
        reason: StepRejection,
    },
    Unknown {
        reason: RecoveryUncertainty,
    },
    LegacyUntracked {},
    InstallationChanged {},
    SlotFenced {},
    Unavailable {
        reason: StepUnavailable,
    },
}

#[cfg(any(feature = "full", test))]
fn canonical_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

#[cfg(any(feature = "full", test))]
impl RecoveryObservation {
    pub(crate) fn matches(&self, step: &UninstallStep) -> bool {
        let Ok(expected) = step.request_digest() else {
            return false;
        };
        let Self::Observed { snapshot } = self else {
            return true;
        };
        if snapshot.request_digest != expected
            || snapshot
                .receipt
                .as_ref()
                .is_some_and(|receipt| !receipt.matches(step))
        {
            return false;
        }
        if snapshot
            .receipt
            .as_ref()
            .is_some_and(|r| matches!(r.outcome, StepOutcome::Unknown { .. }))
            && !snapshot.slot_fenced
        {
            return false;
        }
        match &snapshot.installation {
            InstallationEvidence::Installed {
                generation_digest, ..
            } => canonical_digest(generation_digest),
            InstallationEvidence::Removed {
                revision,
                previous_revision,
                uninstall_request_digest,
            } => revision != previous_revision && canonical_digest(uninstall_request_digest),
            InstallationEvidence::Pending { .. } => snapshot.slot_fenced,
            InstallationEvidence::Untracked {} | InstallationEvidence::Unavailable { .. } => true,
        }
    }

    /// The caller must still re-authorize and re-verify a FUTURE effect. In
    /// particular, neither an absent link nor a tombstone resolves Unknown.
    pub(crate) fn basis(&self, step: &UninstallStep) -> RecoveryBasis {
        if !self.matches(step) {
            return RecoveryBasis::Unavailable {
                reason: StepUnavailable::InvalidRequest,
            };
        }
        let snapshot = match self {
            Self::Unavailable { reason } => return RecoveryBasis::Unavailable { reason: *reason },
            Self::Observed { snapshot } => snapshot,
        };
        let unknown = |reason| RecoveryBasis::Unknown { reason };
        let Some(receipt) = &snapshot.receipt else {
            return unknown(RecoveryUncertainty::MissingForwardReceipt);
        };
        match receipt.outcome {
            StepOutcome::Unknown { .. } => return unknown(RecoveryUncertainty::ForwardOutcome),
            StepOutcome::Rejected { reason } => return RecoveryBasis::ForwardRejected { reason },
            StepOutcome::Applied {} => {}
        }
        let Some(expected_revision) = &step.installation_revision else {
            return RecoveryBasis::LegacyUntracked {};
        };
        match &snapshot.installation {
            InstallationEvidence::Pending { .. } => {
                return unknown(RecoveryUncertainty::InstallationPending);
            }
            InstallationEvidence::Unavailable { .. } => {
                return unknown(RecoveryUncertainty::InstallationUnavailable);
            }
            _ => {}
        }
        if snapshot.slot_fenced {
            return RecoveryBasis::SlotFenced {};
        }
        match &snapshot.installation {
            InstallationEvidence::Removed {
                revision,
                previous_revision,
                uninstall_request_digest,
            } if previous_revision == expected_revision
                && uninstall_request_digest == &snapshot.request_digest =>
            {
                RecoveryBasis::MatchingRemoval {
                    tombstone_revision: revision.clone(),
                }
            }
            _ => RecoveryBasis::InstallationChanged {},
        }
    }
}

#[cfg(test)]
mod tests;
