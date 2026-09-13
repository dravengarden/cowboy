//! A single managed emission, owned by the original connection and deadline.
//! Reopening a binding ledger never constructs this non-serializable value.

use super::*;
use crate::machine_protocol::telemetry_export::{
    ATTEMPT_BUDGET, ExportAttempt, ExportOutcome, ExportReceipt,
};
use crate::operation_budget::{OperationBudget, TimeSample};

pub(crate) struct ManagedExportInvocation {
    request: ExportAttempt,
    connected: Arc<AtomicBool>,
    budget: OperationBudget,
}

impl ManagedExportInvocation {
    pub(in crate::machine_plugins) fn new(
        request: ExportAttempt,
        connected: Arc<AtomicBool>,
        received: TimeSample,
    ) -> Result<Self> {
        request.validate()?;
        Ok(Self {
            budget: OperationBudget::new(request.expires_at_ms, ATTEMPT_BUDGET, received),
            request,
            connected,
        })
    }

    fn check(&self) -> Result<()> {
        ensure!(
            self.connected.load(Ordering::Acquire) && !self.budget.expired(),
            "managed export lease ended"
        );
        Ok(())
    }

    async fn lock<'a>(
        &self,
        store: &'a MachinePluginStore,
    ) -> Result<tokio::sync::MutexGuard<'a, ()>> {
        self.check()?;
        let guard = tokio::time::timeout(self.budget.remaining(), store.lifecycle.lock())
            .await
            .context("managed export admission budget ended")?;
        self.check()?;
        Ok(guard)
    }
}

impl MachinePluginStore {
    pub(crate) async fn export_bound_telemetry(
        &self,
        invocation: ManagedExportInvocation,
    ) -> ExportReceipt {
        // The constructor checked the exact immutable request before scheduling.
        let request_digest = invocation
            .request
            .request_digest()
            .expect("validated managed export");
        let outcome = self
            .export_bound_attempt(&invocation)
            .await
            .unwrap_or(ExportOutcome::NotAdmitted {});
        ExportReceipt {
            request_digest,
            outcome,
        }
    }

    async fn export_bound_attempt(
        &self,
        invocation: &ManagedExportInvocation,
    ) -> Result<ExportOutcome> {
        let _export = self
            .telemetry_export
            .try_lock()
            .context("telemetry exporter is busy")?;
        let request = &invocation.request;
        let target = request
            .binding
            .selection
            .as_ref()
            .context("no managed selection")?;
        let selection = crate::telemetry_plugin::PluginSelection {
            plugin_id: target.plugin_id.clone(),
            plugin_version: target.plugin_version.clone(),
            generation_digest: target.generation_digest.clone().into(),
        };
        let policy_path = self
            .root
            .parent()
            .context("Machine state directory is missing")?
            .join("telemetry.json");
        let check_binding = || -> Result<_> {
            self.operations
                .telemetry_bindings
                .ensure_export_current(request)?;
            self.operations.ensure_unfenced(&target.plugin_id)?;
            let inventory = self.telemetry_inventory(&selection)?;
            ensure!(
                inventory.installation_revision.as_ref() == Some(&target.installation_revision)
                    && inventory.contract_fingerprint
                        == String::from(target.contract_fingerprint.clone()),
                "managed telemetry installation changed"
            );
            self.telemetry_contract(&selection, &inventory)
        };
        let (contract, policy, activation) = {
            let _guard = invocation.lock(self).await?;
            let contract = check_binding()?;
            let policy = crate::telemetry_plugin::prepare_policy(&policy_path, &selection)?;
            let activation = ActivationObservation::read(self, &target.plugin_id)?;
            invocation.check()?;
            (contract, policy, activation)
        };
        let admitted = AtomicBool::new(false);
        let admit = || async {
            let valid = async {
                let _guard = invocation.lock(self).await?;
                check_binding()?;
                ensure!(
                    ActivationObservation::read(self, &target.plugin_id)? == activation
                        && policy.unchanged(&policy_path),
                    "managed telemetry policy or activation changed"
                );
                invocation.check()?;
                // Defense in depth if an exporter ever gains another attempt.
                ensure!(
                    !admitted.swap(true, Ordering::AcqRel),
                    "managed attempt already consumed"
                );
                Ok::<_, anyhow::Error>(())
            }
            .await;
            valid.is_ok()
        };
        // The lifecycle lock ends at admission, never across external I/O.
        // Already-admitted HTTP may finish even if its scope ends meanwhile.
        let result = crate::telemetry_plugin::export_once(
            &contract,
            &policy,
            request.payload.clone(),
            &admit,
        )
        .await;
        Ok(if !result.enabled {
            ExportOutcome::Disabled {}
        } else if !admitted.load(Ordering::Acquire) {
            ExportOutcome::NotAdmitted {}
        } else if result.delivered {
            ExportOutcome::Delivered {}
        } else if result.rejected_items > 0 {
            ExportOutcome::Partial {
                rejected_items: result.rejected_items,
            }
        } else {
            ExportOutcome::Unknown {}
        })
    }
}

#[cfg(test)]
mod tests;
