//! New Operator purpose for Machine-only bookkeeping; not Service resolution.
#![cfg_attr(not(test), allow(dead_code))]
use super::*;
use crate::machine_protocol::telemetry_binding::{BindingDigest, binding_digest};
use crate::machine_protocol::telemetry_recovery::{RecoveryActor, RecoveryRequest};
use crate::telemetry_binding::{Operation, Progress};

pub(in crate::server) fn recovery_actor(actor: &Actor) -> RecoveryActor {
    match actor {
        Actor::Product { user_id } => RecoveryActor::Product {
            user_id: user_id.clone(),
        },
        Actor::Admin { account } => RecoveryActor::Admin {
            account: account.clone(),
        },
    }
}

pub(in crate::server) struct TelemetryRecoveryAuthority {
    approval: OperatorApproval,
    before: Operation,
    digest: BindingDigest,
    budget: OperationBudget,
    revoked: AtomicBool,
}

impl OperatorApproval {
    pub(in crate::server) fn bind_telemetry_recovery(
        self,
        request: &RecoveryRequest,
        before: &Operation,
    ) -> Result<TelemetryRecoveryAuthority> {
        self.require_general_purpose()?;
        ensure!(
            self.service == request.step.service_id
                && recovery_actor(&self.actor) == request.actor
                && matches!(before.progress, Progress::NeedsAttention { .. })
                && before.intent.machine_step()? == request.step
                && binding_digest(&serde_json::to_vec(before)?) == request.service_operation_digest,
            "Machine recovery confirmation owner or Service operation changed"
        );
        Ok(TelemetryRecoveryAuthority {
            digest: request.digest()?,
            budget: OperationBudget::new(
                request.expires_at_ms,
                Duration::from_mins(1),
                self.received,
            ),
            approval: self,
            before: before.clone(),
            revoked: AtomicBool::new(false),
        })
    }
}

impl TelemetryRecoveryAuthority {
    pub(in crate::server) fn constrain_to_preview(mut self, preview: OperationBudget) -> Self {
        self.budget = self.budget.intersect(preview);
        self
    }

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
        store: &crate::store::Store,
        request: &RecoveryRequest,
    ) -> bool {
        let valid = !self.remaining().is_zero()
            && request.digest().is_ok_and(|digest| digest == self.digest)
            && self.approval.current_operator(auth).await.as_ref() == Some(&self.approval.actor)
            && store
                .telemetry_binding_ledger(&self.approval.service)
                .await
                .is_ok_and(|ledger| {
                    ledger.is_some_and(|ledger| ledger.operations.last() == Some(&self.before))
                })
            && !self.remaining().is_zero();
        if !valid {
            self.revoked.store(true, Ordering::Release);
        }
        valid
    }
}
