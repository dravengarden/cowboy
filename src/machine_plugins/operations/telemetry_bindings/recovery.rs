//! Close only a validated reopened Prepared record. Never change its head.
use super::*;
use crate::machine_plugins::operations::lease::BindingRecoveryLease;
use crate::machine_protocol::telemetry_binding::{
    BindingCommitFailure as Failure, BindingRejection,
};
use crate::machine_protocol::telemetry_recovery::{
    RecoveryAction, RecoveryObservation, RecoveryRequest, RecoveryResult, RecoverySnapshot,
};

impl Bindings {
    fn recovery_lookup(state: &BindingState, request: &RecoveryRequest) -> RecoveryObservation {
        let unavailable = |reason| RecoveryObservation::Unavailable { reason };
        let Ok(digest) = request.digest() else {
            return unavailable(BindingUnavailable::InvalidRequest);
        };
        let binding = Self::lookup(state, &request.step);
        if let BindingObservation::Unavailable { reason } = binding {
            return unavailable(reason);
        }
        let receipt = state.ledger.as_ref().and_then(|ledger| {
            ledger
                .resolutions
                .iter()
                .find(|receipt| receipt.request.resolution_id == request.resolution_id)
        });
        if receipt.is_some_and(|receipt| !receipt.matches(request)) {
            return unavailable(BindingUnavailable::IdentityConflict);
        }
        let result = RecoveryObservation::Observed {
            snapshot: Box::new(RecoverySnapshot {
                request_digest: digest,
                receipt: receipt.cloned().map(Box::new),
                binding,
            }),
        };
        if result.matches(request) {
            result
        } else {
            unavailable(BindingUnavailable::Storage)
        }
    }

    fn query_recovery(&self, request: &RecoveryRequest) -> RecoveryObservation {
        let mut state = self.state.lock();
        if !self.retained_current(&mut state) {
            return RecoveryObservation::Unavailable {
                reason: BindingUnavailable::Storage,
            };
        }
        Self::recovery_lookup(&state, request)
    }

    fn recover(
        &self,
        request: &RecoveryRequest,
        lease: &BindingRecoveryLease,
    ) -> std::result::Result<RecoveryObservation, Failure> {
        self.recover_with_io(request, lease, |bytes| {
            atomic_write(&self.path, bytes, 0o600)?;
            fs::File::open(self.path.parent().context("binding journal parent")?)?.sync_all()?;
            Ok(())
        })
    }

    fn recover_with_io(
        &self,
        request: &RecoveryRequest,
        lease: &BindingRecoveryLease,
        mut persist: impl FnMut(&[u8]) -> Result<()>,
    ) -> std::result::Result<RecoveryObservation, Failure> {
        if !lease.matches(request) {
            return Err(Failure::Unavailable(BindingUnavailable::InvalidRequest));
        }
        let mut state = self.state.lock();
        if !self.retained_current(&mut state) {
            return Err(Failure::Unavailable(BindingUnavailable::Storage));
        }
        let observation = Self::recovery_lookup(&state, request);
        let snapshot = match &observation {
            RecoveryObservation::Unavailable { reason } => {
                return Err(Failure::Unavailable(*reason));
            }
            RecoveryObservation::Observed { snapshot } if snapshot.receipt.is_some() => {
                return Ok(observation);
            }
            RecoveryObservation::Observed { snapshot } => snapshot,
        };
        if !state.recovery_writer {
            return Err(Failure::ReaderOnly);
        }
        if !request.expects(&snapshot.binding) {
            return Err(Failure::Rejected(BindingRejection::TargetChanged));
        }
        if state.reopened_prepared.as_ref()
            != Some(&request.step.request_digest().expect("validated request"))
        {
            return Err(Failure::Fenced);
        }
        lease.check().map_err(Failure::Rejected)?;
        let mut ledger = state.ledger.clone().ok_or(Failure::Fenced)?;
        let last = ledger.receipts.last_mut().ok_or(Failure::Fenced)?;
        if !last.matches(&request.step) || !matches!(last.outcome, BindingOutcome::Prepared {}) {
            return Err(Failure::Fenced);
        }
        // A reopened owner cannot hold the original connection's execution
        // lease. This rejects that old attempt, not the retained namespace.
        last.outcome = match request.action {
            RecoveryAction::RejectInterruptedPrepared => BindingOutcome::Rejected {
                reason: BindingRejection::AuthorizationEnded,
            },
        };
        let receipt = RecoveryReceipt {
            request: request.clone(),
            binding: last.clone(),
            resolved_at_ms: chrono::Utc::now().timestamp_millis(),
        };
        ledger.resolutions.push(receipt);
        ledger.schema = 2;
        let bytes = ledger.encode()?;
        lease.check().map_err(Failure::Rejected)?;
        if persist(&bytes).is_err() {
            state.poisoned = true;
            return Err(Failure::Unavailable(BindingUnavailable::Storage));
        }
        state.ledger = Some(ledger);
        state.reopened_prepared = None;
        Ok(Self::recovery_lookup(&state, request))
    }
}

impl MachinePluginStore {
    pub(crate) async fn recover_telemetry_binding(
        &self,
        request: &RecoveryRequest,
        lease: BindingRecoveryLease,
    ) -> RecoveryResult {
        let result = async {
            let _lifecycle = tokio::time::timeout(
                lease.remaining().map_err(Failure::Rejected)?,
                self.lifecycle.lock(),
            )
            .await
            .map_err(|_| Failure::Rejected(BindingRejection::Expired))?;
            lease.check().map_err(Failure::Rejected)?;
            if self.operations.state.lock().poisoned {
                return Err(Failure::Unavailable(BindingUnavailable::Storage));
            }
            self.operations.telemetry_bindings.recover(request, &lease)
        }
        .await;
        match result {
            Ok(observation) => RecoveryResult::Observed { observation },
            Err(failure) => RecoveryResult::Unavailable { failure },
        }
    }

    pub(crate) async fn telemetry_recovery_observation(
        &self,
        request: &RecoveryRequest,
        service: Option<&str>,
        machine: &str,
    ) -> RecoveryObservation {
        let unavailable = |reason| RecoveryObservation::Unavailable { reason };
        if request.validate().is_err() {
            return unavailable(BindingUnavailable::InvalidRequest);
        }
        if service != Some(request.step.service_id.as_str()) || machine != request.step.machine_id {
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
        self.operations.telemetry_bindings.query_recovery(request)
    }
}

#[cfg(test)]
mod tests;
