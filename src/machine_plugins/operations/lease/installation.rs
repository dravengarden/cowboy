//! Original connection and original deadline for one exact installation.
use super::{
    Arc, AtomicBool, Duration, OperationBudget, Ordering, PluginExecutionScope, TimeSample,
};
use crate::machine_protocol::plugin_install::{InstallRejection, InstallStep, InstallUnavailable};

pub(crate) struct InstallationLease {
    connected: Arc<AtomicBool>,
    digest: String,
    budget: OperationBudget,
}

impl PluginExecutionScope {
    /// Capture synchronously on receipt, before scheduling or lock contention.
    pub(crate) fn installation(
        &self,
        step: &InstallStep,
    ) -> Result<InstallationLease, InstallUnavailable> {
        let received = TimeSample::now();
        let digest = step
            .request_digest()
            .map_err(|_| InstallUnavailable::InvalidRequest)?;
        if !self.connected.load(Ordering::Acquire)
            || self.service.as_deref() != Some(step.service_id.as_str())
            || self.machine != step.machine_id
        {
            return Err(InstallUnavailable::WrongOwner);
        }
        Ok(InstallationLease {
            connected: Arc::clone(&self.connected),
            digest,
            budget: OperationBudget::new(step.expires_at_ms, Duration::from_mins(5), received),
        })
    }
}

impl InstallationLease {
    pub(in crate::machine_plugins) fn matches(&self, step: &InstallStep) -> bool {
        step.request_digest()
            .is_ok_and(|digest| digest == self.digest)
    }

    pub(in crate::machine_plugins) fn check(&self) -> Result<(), InstallRejection> {
        if !self.connected.load(Ordering::Acquire) {
            Err(InstallRejection::AuthorizationEnded)
        } else if self.budget.expired() {
            Err(InstallRejection::Expired)
        } else {
            Ok(())
        }
    }

    pub(in crate::machine_plugins) fn remaining(&self) -> Result<Duration, InstallRejection> {
        self.check()?;
        let remaining = self.budget.remaining();
        if remaining.is_zero() {
            Err(InstallRejection::Expired)
        } else {
            Ok(remaining)
        }
    }

    #[cfg(test)]
    pub(in crate::machine_plugins) fn expire_for_test(&self) {
        self.budget.expire_for_test();
    }
}
