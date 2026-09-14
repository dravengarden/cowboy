//! One process-local install/upgrade confirmation. Neither a serialized
//! Catalog envelope nor a later login can mint or renew this authority.

use super::*;
use crate::machine_protocol::DesiredPlugin;
use crate::plugin_operation::installation::InstallIntent;

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

impl OperatorApproval {
    pub(in crate::server) fn bind_installation(
        self,
        machine: &str,
        desired: &DesiredPlugin,
        operation_id: String,
    ) -> Result<InstallationAuthority> {
        let expires_at_ms = self.received.deadline_ms(Duration::from_mins(5));
        let intent = InstallIntent {
            schema: 1,
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
}

impl InstallationAuthority {
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
