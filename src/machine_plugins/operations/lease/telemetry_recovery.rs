//! Different purpose and deadline from the interrupted binding mutation.
use super::{
    Arc, AtomicBool, BindingDigest, BindingRejection, BindingUnavailable, Duration,
    MAX_EXECUTION_TIME, OperationBudget, Ordering, PluginExecutionScope, TimeSample,
};
use crate::machine_protocol::telemetry_recovery::RecoveryRequest;

pub(crate) struct BindingRecoveryLease {
    connected: Arc<AtomicBool>,
    digest: BindingDigest,
    budget: OperationBudget,
}

impl PluginExecutionScope {
    pub(crate) fn telemetry_recovery(
        &self,
        request: &RecoveryRequest,
    ) -> Result<BindingRecoveryLease, BindingUnavailable> {
        let received = TimeSample::now();
        let digest = request
            .digest()
            .map_err(|_| BindingUnavailable::InvalidRequest)?;
        if !self.connected.load(Ordering::Acquire)
            || self.service.as_deref() != Some(request.step.service_id.as_str())
            || self.machine != request.step.machine_id
        {
            return Err(BindingUnavailable::WrongOwner);
        }
        Ok(BindingRecoveryLease {
            connected: self.connected.clone(),
            digest,
            budget: OperationBudget::new(request.expires_at_ms, MAX_EXECUTION_TIME, received),
        })
    }
}

impl BindingRecoveryLease {
    pub(in crate::machine_plugins::operations) fn matches(
        &self,
        request: &RecoveryRequest,
    ) -> bool {
        request.digest().is_ok_and(|digest| digest == self.digest)
    }
    pub(in crate::machine_plugins::operations) fn check(&self) -> Result<(), BindingRejection> {
        if !self.connected.load(Ordering::Acquire) {
            Err(BindingRejection::AuthorizationEnded)
        } else if self.budget.expired() {
            Err(BindingRejection::Expired)
        } else {
            Ok(())
        }
    }
    pub(in crate::machine_plugins::operations) fn remaining(
        &self,
    ) -> Result<Duration, BindingRejection> {
        self.check()?;
        let remaining = self.budget.remaining();
        if remaining.is_zero() {
            Err(BindingRejection::Expired)
        } else {
            Ok(remaining)
        }
    }
    #[cfg(test)]
    pub(in crate::machine_plugins::operations) fn expire_for_test(&self) {
        self.budget.expire_for_test();
    }
}
