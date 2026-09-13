//! Read-only discovery of retained Machine audit, not a recovery request.
//! No Actor, new expiry or conversion to an execution lease belongs here.
use super::telemetry_binding::{
    BindingDigest, BindingObservation, BindingStep, BindingUnavailable, binding_digest,
};
use super::telemetry_recovery::{MAX_RECOVERY_BYTES, RecoveryReceipt};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryAuditQuery {
    pub schema: u16,
    pub step: BindingStep,
}

impl RecoveryAuditQuery {
    pub(crate) fn digest(&self) -> Result<BindingDigest> {
        // The original deadline is evidence, never revived authority.
        self.step.validate_commit()?;
        let bytes = serde_json::to_vec(self)?;
        ensure!(
            self.schema == 1 && bytes.len() <= 16 * 1024,
            "invalid recovery audit query"
        );
        Ok(binding_digest(&bytes))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryAuditSnapshot {
    pub query_digest: BindingDigest,
    pub receipt: Option<Box<RecoveryReceipt>>,
    /// Historical receipt and current head remain distinct.
    pub binding: BindingObservation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryAuditObservation {
    Observed {
        snapshot: Box<RecoveryAuditSnapshot>,
    },
    Unavailable {
        reason: BindingUnavailable,
    },
}

impl RecoveryAuditObservation {
    pub(crate) fn matches(&self, query: &RecoveryAuditQuery) -> bool {
        let Ok(digest) = query.digest() else {
            return false;
        };
        let Self::Observed { snapshot } = self else {
            return true;
        };
        let BindingObservation::Observed { snapshot: binding } = &snapshot.binding else {
            return false;
        };
        snapshot.query_digest == digest
            && snapshot.binding.matches(&query.step)
            && snapshot.receipt.as_ref().is_none_or(|receipt| {
                receipt.validate().is_ok()
                    && receipt.request.step == query.step
                    && binding.receipt.as_deref() == Some(&receipt.binding)
            })
            && serde_json::to_vec(self)
                .is_ok_and(|bytes| bytes.len() <= MAX_RECOVERY_BYTES + 16 * 1024)
    }
}

#[cfg(test)]
mod tests;
