//! One process-local install/upgrade confirmation. Neither a serialized
//! Catalog envelope nor a later login can mint or renew this authority.

use super::*;
use crate::machine_protocol::DesiredPlugin;

fn target_digest(machine: &str, desired: &DesiredPlugin) -> Result<String> {
    Ok(hex_sha256(&serde_json::to_vec(&(machine, desired))?))
}

pub(in crate::server) struct InstallationAuthority {
    approval: OperatorApproval,
    target_digest: String,
    budget: OperationBudget,
    revoked: AtomicBool,
}

impl OperatorApproval {
    pub(in crate::server) fn bind_installation(
        self,
        machine: &str,
        desired: &DesiredPlugin,
    ) -> Result<InstallationAuthority> {
        Ok(InstallationAuthority {
            target_digest: target_digest(machine, desired)?,
            budget: OperationBudget::new(
                auth_now_ms().saturating_add(300_000),
                Duration::from_mins(5),
                self.received,
            ),
            approval: self,
            revoked: AtomicBool::new(false),
        })
    }
}

impl InstallationAuthority {
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
