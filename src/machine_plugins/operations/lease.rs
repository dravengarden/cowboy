//! Ephemeral execution authority for one authenticated Machine connection.
//! Neither connection ownership nor a monotonic deadline can be deserialized
//! from a durable receipt. This does not grant recovery or Provider credentials.

use crate::machine_protocol::plugin_step::{StepUnavailable, UninstallStep};
use crate::machine_protocol::telemetry_binding::{
    BindingDigest, BindingRejection, BindingStep, BindingUnavailable,
};
use crate::operation_budget::{OperationBudget, TimeSample};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const MAX_EXECUTION_TIME: Duration = Duration::from_mins(1);

/// The connection task is the unique owner. Detached command tasks receive only
/// leases; retaining one must not keep a disconnected connection authorized.
pub(crate) struct PluginExecutionScope {
    connected: Arc<AtomicBool>,
    service: Option<String>,
    machine: String,
}

impl PluginExecutionScope {
    pub(crate) fn new(service: Option<&str>, machine: &str) -> Self {
        Self {
            connected: Arc::new(AtomicBool::new(true)),
            service: service.map(str::to_owned),
            machine: machine.to_owned(),
        }
    }

    /// Capture before spawn: the request and its original connection/time
    /// budget travel together and cannot be retargeted by an async caller.
    pub(crate) fn host(
        &self,
        request: crate::machine_plugins::PluginHostRequest,
    ) -> crate::machine_plugins::PluginHostInvocation {
        crate::machine_plugins::PluginHostInvocation::new(request, Arc::clone(&self.connected))
    }

    /// Called synchronously on command receipt, BEFORE spawning or waiting for
    /// a lifecycle lock. The wire supplies no duration or reusable capability.
    pub(crate) fn uninstall(
        &self,
        step: &UninstallStep,
    ) -> Result<UninstallExecutionLease, StepUnavailable> {
        self.uninstall_at(step, TimeSample::now())
    }

    /// Capture on protocol receipt, before detached scheduling. A negotiated
    /// command still cannot enable the independent local writer admission.
    pub(crate) fn telemetry_binding(
        &self,
        step: &BindingStep,
    ) -> Result<BindingExecutionLease, BindingUnavailable> {
        let received = TimeSample::now();
        let request_digest = step
            .request_digest()
            .map_err(|_| BindingUnavailable::InvalidRequest)?;
        if !self.connected.load(Ordering::Acquire)
            || self.service.as_deref() != Some(step.service_id.as_str())
            || self.machine != step.machine_id
        {
            return Err(BindingUnavailable::WrongOwner);
        }
        Ok(BindingExecutionLease {
            connected: Arc::clone(&self.connected),
            request_digest,
            budget: OperationBudget::new(step.expires_at_ms, MAX_EXECUTION_TIME, received),
        })
    }

    fn uninstall_at(
        &self,
        step: &UninstallStep,
        received: TimeSample,
    ) -> Result<UninstallExecutionLease, StepUnavailable> {
        let request_digest = step
            .request_digest()
            .map_err(|_| StepUnavailable::InvalidRequest)?;
        if !self.connected.load(Ordering::Acquire)
            || self.service.as_deref() != Some(step.service_id.as_str())
            || self.machine != step.machine_id
        {
            return Err(StepUnavailable::WrongOwner);
        }
        Ok(UninstallExecutionLease {
            connected: Arc::clone(&self.connected),
            request_digest,
            budget: OperationBudget::new(step.expires_at_ms, MAX_EXECUTION_TIME, received),
        })
    }
}

/// No deserialization, cloning, retargeting or renewal from a stored receipt.
pub(crate) struct BindingExecutionLease {
    connected: Arc<AtomicBool>,
    request_digest: BindingDigest,
    budget: OperationBudget,
}

impl BindingExecutionLease {
    pub(super) fn matches(&self, step: &BindingStep) -> bool {
        step.request_digest()
            .is_ok_and(|digest| digest == self.request_digest)
    }

    pub(super) fn check(&self) -> Result<(), BindingRejection> {
        if !self.connected.load(Ordering::Acquire) {
            Err(BindingRejection::AuthorizationEnded)
        } else if self.budget.expired() {
            Err(BindingRejection::Expired)
        } else {
            Ok(())
        }
    }

    pub(super) fn remaining(&self) -> Result<Duration, BindingRejection> {
        self.check()?;
        let remaining = self.budget.remaining();
        if remaining.is_zero() {
            Err(BindingRejection::Expired)
        } else {
            Ok(remaining)
        }
    }

    #[cfg(test)]
    pub(super) fn expire_for_test(&self) {
        self.budget.expire_for_test();
    }
}

impl Drop for PluginExecutionScope {
    fn drop(&mut self) {
        self.connected.store(false, Ordering::Release);
    }
}

/// No Clone, Serialize or Deserialize: one admitted command, exact request,
/// original connection and process-local deadline. A saved result is evidence,
/// not an instance of this type.
pub(crate) struct UninstallExecutionLease {
    connected: Arc<AtomicBool>,
    request_digest: String,
    budget: OperationBudget,
}

impl UninstallExecutionLease {
    pub(super) fn matches(&self, step: &UninstallStep) -> bool {
        step.request_digest()
            .is_ok_and(|digest| digest == self.request_digest)
    }

    pub(super) fn connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    pub(super) fn expired(&self) -> bool {
        self.budget.expired()
    }

    pub(super) fn before_effect(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.connected() && !self.expired(),
            "Plugin execution lease ended before removal"
        );
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn expire_for_test(&self) {
        self.budget.expire_for_test();
    }

    #[cfg(test)]
    fn expired_at(&self, now: TimeSample) -> bool {
        self.budget.expired_at(now)
    }
}

#[cfg(test)]
mod tests;
