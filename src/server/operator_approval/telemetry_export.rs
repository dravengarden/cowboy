//! One fresh export confirmation, distinct from binding mutation/recovery.
#![cfg_attr(not(test), allow(dead_code))]
use super::*;
use crate::machine_protocol::telemetry_export::{ATTEMPT_BUDGET, ExportAttempt};

pub(in crate::server) struct TelemetryExportAuthority {
    approval: OperatorApproval,
    digest: crate::machine_protocol::telemetry_binding::BindingDigest,
    budget: OperationBudget,
    revoked: AtomicBool,
}

impl OperatorApproval {
    pub(in crate::server) fn bind_telemetry_export(
        self,
        attempt: &ExportAttempt,
    ) -> Result<TelemetryExportAuthority> {
        ensure!(
            self.service == attempt.service_id,
            "export confirmation owner changed"
        );
        Ok(TelemetryExportAuthority {
            digest: attempt.request_digest()?,
            budget: OperationBudget::new(attempt.expires_at_ms, ATTEMPT_BUDGET, self.received),
            approval: self,
            revoked: AtomicBool::new(false),
        })
    }
}

impl TelemetryExportAuthority {
    pub(in crate::server) fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
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
        attempt: &ExportAttempt,
    ) -> bool {
        let valid = !self.remaining().is_zero()
            && self.approval.service == attempt.service_id
            && attempt.request_digest().is_ok_and(|d| d == self.digest)
            && self.approval.current_operator(auth).await.as_ref() == Some(&self.approval.actor)
            && !self.remaining().is_zero();
        if !valid {
            self.revoke();
        }
        valid
    }
}
