//! Exact trusted installation and original authenticated Machine connection.
//! Neither this transport nor a negotiated protocol supplies Operator authority.

use super::*;
use crate::machine_control::{ConnectionToken, MachineControl};
use crate::plugin_catalog::PluginCatalog;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) const BINDING_WRITE_ADMISSION: bool = false;

pub(super) struct LiveEffects {
    control: Arc<MachineControl>,
    catalog: Arc<PluginCatalog>,
    fences: crate::server::PluginLifecycleFences,
    connection: ConnectionToken,
    step: BindingStep,
    ended: AtomicBool,
}

impl LiveEffects {
    fn capture(
        control: Arc<MachineControl>,
        catalog: Arc<PluginCatalog>,
        fences: crate::server::PluginLifecycleFences,
        intent: &Intent,
    ) -> Result<Self> {
        ensure!(
            BINDING_WRITE_ADMISSION,
            "telemetry binding writer admission is closed"
        );
        Self::bind(control, catalog, fences, intent)
    }

    pub(super) fn bind(
        control: Arc<MachineControl>,
        catalog: Arc<PluginCatalog>,
        fences: crate::server::PluginLifecycleFences,
        intent: &Intent,
    ) -> Result<Self> {
        let step = intent.machine_step()?;
        step.validate_commit()?;
        let connection = control
            .operation_connection(&step.machine_id)
            .map_err(|_| anyhow::anyhow!("binding Machine is not connected"))?;
        let effects = Self {
            control,
            catalog,
            fences,
            connection,
            step,
            ended: AtomicBool::new(false),
        };
        ensure!(
            effects.current(),
            "binding target is not currently authorized"
        );
        Ok(effects)
    }

    pub(super) fn current(&self) -> bool {
        let valid = !self.ended.load(Ordering::Acquire)
            && self
                .control
                .telemetry_binding_target_current(&self.connection, &self.step)
            && self.step.after().is_ok_and(|after| {
                after.selection.is_none_or(|target| {
                    // Revocation needs no removed installation, old policy or
                    // Catalog entry. Selecting/restoring an installation does.
                    !self
                        .fences
                        .read()
                        .contains_key(&(self.step.machine_id.clone(), target.plugin_id.clone()))
                        && self
                            .catalog
                            .resolve_telemetry_backend(
                                &target.plugin_id,
                                &target.plugin_version,
                                &String::from(target.generation_digest.clone()),
                            )
                            .is_ok_and(|release| {
                                self.control
                                    .connected_plugin_inventory()
                                    .iter()
                                    .any(|entry| {
                                        entry.machine_id == self.step.machine_id
                                            && release.matches_inventory(&entry.plugin)
                                            && entry.plugin.installation_revision.as_ref()
                                                == Some(&target.installation_revision)
                                            && entry.plugin.contract_fingerprint
                                                == String::from(target.contract_fingerprint.clone())
                                    })
                            })
                })
            });
        if !valid {
            self.ended.store(true, Ordering::Release);
        }
        valid
    }
}

impl Effects for LiveEffects {
    async fn authorized(&self, intent: &Intent) -> bool {
        let valid = intent.machine_step().is_ok_and(|step| step == self.step) && self.current();
        if !valid {
            self.ended.store(true, Ordering::Release);
        }
        valid
    }

    fn within_budget(&self) -> bool {
        // The independently required Confirmation owns the original time
        // budget; this transport cannot renew it while checking its connection.
        !self.ended.load(Ordering::Acquire) && self.control.is_current(&self.connection)
    }

    async fn dispatch(&self, step: &BindingStep) -> Result<BindingObservation> {
        ensure!(
            step == &self.step && self.current(),
            "binding transport ended"
        );
        self.control
            .commit_telemetry_binding(&self.connection, step)
            .await
            .map_err(|_| anyhow::anyhow!("binding mutation receipt unavailable"))
    }

    async fn observe(&self, step: &BindingStep) -> Result<BindingObservation> {
        ensure!(step == &self.step, "binding observation identity changed");
        self.control
            .telemetry_binding_observation(&self.connection, step)
            .await
            .map_err(|_| anyhow::anyhow!("binding observation unavailable"))
    }
}

#[cfg(all(test, feature = "machine-host"))]
pub(super) mod tests;
