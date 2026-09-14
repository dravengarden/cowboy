//! Exact trusted installation and original authenticated Machine connection.
//! Neither this transport nor a negotiated protocol supplies Operator authority.

use super::*;
use crate::composition::telemetry::ResolvedBinding;
#[cfg(all(test, feature = "machine-host"))]
use crate::machine_control::ConnectionToken;
use crate::machine_control::MachineControl;
use crate::plugin_catalog::PluginCatalog;
use crate::telemetry_plugin::writer_admission::{BindingWrites, WriteScope};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct LiveEffects {
    control: Arc<MachineControl>,
    catalog: Arc<PluginCatalog>,
    fences: crate::server::PluginLifecycleFences,
    resolved: ResolvedBinding,
    ended: AtomicBool,
    admission: Option<WriteScope<BindingWrites>>,
}

impl LiveEffects {
    pub(super) fn admit(mut self, scope: Option<WriteScope<BindingWrites>>) -> Self {
        self.admission = scope;
        self
    }

    fn admitted(&self) -> bool {
        self.admission.as_ref().is_some_and(|scope| {
            let step = self.resolved.step();
            scope.check_for(&step.service_id, &step.machine_id)
        })
    }

    pub(super) fn bind(
        control: Arc<MachineControl>,
        catalog: Arc<PluginCatalog>,
        fences: crate::server::PluginLifecycleFences,
        intent: &Intent,
    ) -> Result<Self> {
        let resolved = ResolvedBinding::resolve(&catalog, &control, intent.machine_step()?)?;
        let effects = Self {
            control,
            catalog,
            fences,
            resolved,
            ended: AtomicBool::new(false),
            admission: None,
        };
        ensure!(
            effects.current(),
            "binding target is not currently authorized"
        );
        Ok(effects)
    }

    pub(super) fn current(&self) -> bool {
        let valid = !self.ended.load(Ordering::Acquire)
            && self.resolved.current(&self.catalog, &self.control)
            && self.resolved.installation().is_none_or(|target| {
                // Revocation needs no removed installation, old policy or
                // Catalog entry. Selecting/restoring an installation does.
                !self.fences.read().contains_key(&(
                    self.resolved.step().machine_id.clone(),
                    target.plugin_id.clone(),
                ))
            });
        if !valid {
            self.ended.store(true, Ordering::Release);
        }
        valid
    }
}

impl Effects for LiveEffects {
    async fn authorized(&self, intent: &Intent) -> bool {
        let valid = intent
            .machine_step()
            .is_ok_and(|step| step == *self.resolved.step())
            && self.admitted()
            && self.current();
        if !valid {
            self.ended.store(true, Ordering::Release);
        }
        valid
    }

    fn within_budget(&self) -> bool {
        // The independently required Confirmation owns the original time
        // budget; this transport cannot renew it while checking its connection.
        self.admitted()
            && !self.ended.load(Ordering::Acquire)
            && self.control.is_current(self.resolved.connection())
    }

    async fn dispatch(&self, step: &BindingStep) -> Result<BindingObservation> {
        ensure!(
            step == self.resolved.step() && self.admitted() && self.current(),
            "binding transport ended"
        );
        self.resolved
            .dispatch(&self.catalog, &self.control)
            .await
            .map_err(|_| anyhow::anyhow!("binding mutation receipt unavailable"))
    }

    async fn observe(&self, step: &BindingStep) -> Result<BindingObservation> {
        ensure!(
            step == self.resolved.step(),
            "binding observation identity changed"
        );
        self.control
            .telemetry_binding_observation(self.resolved.connection(), step)
            .await
            .map_err(|_| anyhow::anyhow!("binding observation unavailable"))
    }
}

#[cfg(all(test, feature = "machine-host"))]
pub(super) mod tests;
