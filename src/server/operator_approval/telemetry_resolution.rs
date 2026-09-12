//! New purpose-bound authority. An expired mutation is never renewed here.
#![cfg_attr(not(test), allow(dead_code))]
use super::*;
use crate::machine_protocol::telemetry_binding::{BindingDigest, BindingObservation};
use crate::telemetry_binding::{
    Operation,
    resolution::{ResolutionIntent, ResolutionPermit},
};

pub(in crate::server) struct TelemetryResolutionAuthority {
    approval: OperatorApproval,
    digest: BindingDigest,
    budget: OperationBudget,
    revoked: AtomicBool,
}

impl OperatorApproval {
    pub(in crate::server) fn bind_telemetry_resolution(
        self,
        intent: &ResolutionIntent,
    ) -> Result<TelemetryResolutionAuthority> {
        ensure!(
            self.service == intent.service_id && self.actor == intent.actor,
            "binding resolution confirmation owner changed"
        );
        Ok(TelemetryResolutionAuthority {
            digest: intent.digest()?,
            budget: OperationBudget::new(
                intent.expires_at_ms,
                Duration::from_mins(1),
                self.received,
            ),
            approval: self,
            revoked: AtomicBool::new(false),
        })
    }
}

impl TelemetryResolutionAuthority {
    pub(in crate::server) fn remaining(&self) -> Duration {
        if self.revoked.load(Ordering::Acquire) {
            Duration::ZERO
        } else {
            self.budget.remaining()
        }
    }

    pub(in crate::server) async fn check(
        &self,
        auth: ProductRequestAuth<'_>,
        intent: &ResolutionIntent,
    ) -> bool {
        let valid = !self.remaining().is_zero()
            && self.approval.service == intent.service_id
            && self.approval.actor == intent.actor
            && intent.digest().is_ok_and(|digest| digest == self.digest)
            && self.approval.current_operator(auth).await.as_ref() == Some(&self.approval.actor)
            && !self.remaining().is_zero();
        if !valid {
            self.revoked.store(true, Ordering::Release);
        }
        valid
    }

    pub(in crate::server) async fn into_permit(
        self,
        auth: ProductRequestAuth<'_>,
        intent: ResolutionIntent,
        before: Operation,
        observation: Option<BindingObservation>,
    ) -> Result<ResolutionPermit> {
        ensure!(
            self.check(auth, &intent).await,
            "binding resolution authorization ended"
        );
        ResolutionPermit::new(intent, before, observation, self.budget)
    }
}
