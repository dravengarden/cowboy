//! Discover retained audit under the same lifecycle and sticky evidence fences.
use super::*;
use crate::machine_protocol::telemetry_recovery_audit::{
    RecoveryAuditObservation, RecoveryAuditQuery, RecoveryAuditSnapshot,
};

impl Bindings {
    fn query_recovery_audit(&self, query: &RecoveryAuditQuery) -> RecoveryAuditObservation {
        let unavailable = |reason| RecoveryAuditObservation::Unavailable { reason };
        let Ok(query_digest) = query.digest() else {
            return unavailable(BindingUnavailable::InvalidRequest);
        };
        let mut state = self.state.lock();
        if !self.retained_current(&mut state) {
            return unavailable(BindingUnavailable::Storage);
        }
        let binding = Self::lookup(&state, &query.step);
        if let BindingObservation::Unavailable { reason } = binding {
            return unavailable(reason);
        }
        // Ledger validation proves at most one audit for a complete step and
        // links it to the retained binding receipt. Never match an ID alone.
        let receipt = state.ledger.as_ref().and_then(|ledger| {
            ledger
                .resolutions
                .iter()
                .find(|receipt| receipt.request.step == query.step)
        });
        let observation = RecoveryAuditObservation::Observed {
            snapshot: Box::new(RecoveryAuditSnapshot {
                query_digest,
                receipt: receipt.cloned().map(Box::new),
                binding,
            }),
        };
        if observation.matches(query) {
            observation
        } else {
            unavailable(BindingUnavailable::Storage)
        }
    }
}

impl MachinePluginStore {
    pub(crate) async fn telemetry_recovery_audit(
        &self,
        query: &RecoveryAuditQuery,
        service: Option<&str>,
        machine: &str,
    ) -> RecoveryAuditObservation {
        let unavailable = |reason| RecoveryAuditObservation::Unavailable { reason };
        if query.digest().is_err() {
            return unavailable(BindingUnavailable::InvalidRequest);
        }
        if service != Some(query.step.service_id.as_str()) || machine != query.step.machine_id {
            return unavailable(BindingUnavailable::WrongOwner);
        }
        let Ok(_lifecycle) =
            tokio::time::timeout(Duration::from_secs(10), self.lifecycle.lock()).await
        else {
            return unavailable(BindingUnavailable::Storage);
        };
        if self.operations.state.lock().poisoned {
            return unavailable(BindingUnavailable::Storage);
        }
        self.operations
            .telemetry_bindings
            .query_recovery_audit(query)
    }
}

#[cfg(test)]
mod tests;
