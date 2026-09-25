//! One process-local install/upgrade confirmation. Neither a serialized
//! Catalog envelope nor a later login can mint or renew this authority.

use super::*;
use crate::machine_protocol::DesiredPlugin;
use crate::machine_protocol::plugin_install::{InstallStep, InstallTarget};
use crate::plugin_operation::installation::{InstallIntent, InstallOperation};

fn target_digest(machine: &str, desired: &DesiredPlugin) -> Result<String> {
    Ok(hex_sha256(&serde_json::to_vec(&(machine, desired))?))
}

pub(in crate::server) struct InstallationAuthority {
    approval: OperatorApproval,
    target_digest: String,
    budget: OperationBudget,
    intent: InstallIntent,
    revoked: AtomicBool,
}

/// Fresh, closed authority to observe and commit the terminal receipt of one
/// already-fenced installation. It cannot be converted into installation
/// authority and carries no signed release envelope.
pub(in crate::server) struct InstallationReconciliationAuthority {
    approval: OperatorApproval,
    operation_digest: String,
    budget: OperationBudget,
    revoked: AtomicBool,
}

impl OperatorApproval {
    pub(in crate::server) fn bind_installation(
        self,
        machine: &str,
        desired: &DesiredPlugin,
        operation_id: String,
        target: InstallTarget,
    ) -> Result<InstallationAuthority> {
        let expires_at_ms = self.received.deadline_ms(Duration::from_mins(5));
        let intent = InstallIntent {
            schema: 2,
            request_id: format!("plugin-install-{operation_id}"),
            operation_id,
            service_id: self.service.clone(),
            actor: self.actor.clone(),
            machine_id: machine.to_owned(),
            plugin_id: desired.release.plugin_id.clone(),
            plugin_kind: desired.release.plugin_kind,
            plugin_version: desired.release.plugin_version.clone(),
            generation_digest: desired.release.artifact_digest.clone(),
            contract_fingerprint: desired.release.contract_fingerprint.clone(),
            envelope_digest: format!("sha256:{}", hex_sha256(&serde_json::to_vec(desired)?)),
            machine_target: Some(target),
            expires_at_ms,
        };
        intent.validate()?;
        Ok(InstallationAuthority {
            target_digest: target_digest(machine, desired)?,
            budget: OperationBudget::new(expires_at_ms, Duration::from_mins(5), self.received),
            approval: self,
            intent,
            revoked: AtomicBool::new(false),
        })
    }

    pub(in crate::server) fn bind_installation_reconciliation(
        self,
        operation: &InstallOperation,
    ) -> Result<InstallationReconciliationAuthority> {
        operation.validate()?;
        let expires_at_ms = self.received.deadline_ms(Duration::from_mins(2));
        let received = self.received;
        Ok(InstallationReconciliationAuthority {
            approval: self,
            operation_digest: hex_sha256(&serde_json::to_vec(operation)?),
            budget: OperationBudget::new(expires_at_ms, Duration::from_mins(2), received),
            revoked: AtomicBool::new(false),
        })
    }
}

impl InstallationAuthority {
    /// Final synchronous identity/budget check at dispatch; this does not
    /// replace the continuous credential/connection checks in the coordinator.
    pub(in crate::server) fn matches_live_step(&self, step: &InstallStep) -> bool {
        let valid = self.within_budget()
            && self
                .intent
                .machine_step()
                .is_ok_and(|expected| &expected == step);
        if !valid {
            self.revoke();
        }
        valid
    }

    pub(in crate::server) fn intent(&self) -> &InstallIntent {
        &self.intent
    }

    pub(in crate::server) fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }

    pub(in crate::server) fn within_budget(&self) -> bool {
        !self.revoked.load(Ordering::Acquire) && !self.budget.expired()
    }

    pub(in crate::server) async fn check(
        &self,
        auth: ProductRequestAuth<'_>,
        service: &str,
        machine: &str,
        desired: &DesiredPlugin,
    ) -> bool {
        let valid = self.within_budget()
            && self.approval.service == service
            && target_digest(machine, desired).is_ok_and(|digest| digest == self.target_digest)
            && self.approval.current_operator(auth).await.as_ref() == Some(&self.approval.actor)
            && self.within_budget();
        if !valid {
            self.revoke();
        }
        valid
    }
}

impl InstallationReconciliationAuthority {
    pub(in crate::server) async fn check(
        &self,
        auth: ProductRequestAuth<'_>,
        service: &str,
        operation: &InstallOperation,
    ) -> bool {
        let valid = !self.revoked.load(Ordering::Acquire)
            && !self.budget.expired()
            && self.approval.service == service
            && operation.intent.service_id == service
            && serde_json::to_vec(operation)
                .is_ok_and(|bytes| hex_sha256(&bytes) == self.operation_digest)
            && self.approval.current_operator(auth).await.as_ref() == Some(&self.approval.actor)
            && !self.budget.expired();
        if !valid {
            self.revoked.store(true, Ordering::Release);
        }
        valid
    }
}
