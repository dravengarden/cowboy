//! Finite Service -> Machine resolution for managed telemetry. Private fields,
//! no Clone/serde, and distinct binding/export results keep a checked report or
//! a deserialized receipt from becoming an executable port. These results are
//! still NOT grants: Operator/standing policy, time budget, durable namespace
//! CAS and Machine-private policy remain independently required by the caller.
#![warn(clippy::pedantic)]

use crate::machine_control::{
    CommandFailure, CommandRequestError, ConnectionToken, MachineControl,
    TelemetryInstallationLease,
};
use crate::machine_protocol::telemetry_binding::{
    BindingInstallation, BindingObservation, BindingStep,
};
use crate::machine_protocol::telemetry_export::{ExportAttempt, ExportReceipt};
use crate::plugin_catalog::{PluginCatalog, VerifiedTelemetryRelease};
use anyhow::{Result, ensure};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy)]
enum Requirement {
    Binding,
    Otlp(crate::otlp::Signal),
}

struct ResolvedPort {
    installation: BindingInstallation,
    release: VerifiedTelemetryRelease,
    lease: TelemetryInstallationLease,
    ended: AtomicBool,
}

impl ResolvedPort {
    fn resolve(
        catalog: &PluginCatalog,
        control: &MachineControl,
        connection: &ConnectionToken,
        installation: BindingInstallation,
        requirement: Requirement,
    ) -> Result<Self> {
        let release = catalog.resolve_telemetry_backend(
            &installation.plugin_id,
            &installation.plugin_version,
            &String::from(installation.generation_digest.clone()),
        )?;
        ensure!(
            release.matches_installation(&installation)
                && match requirement {
                    Requirement::Binding => true,
                    Requirement::Otlp(signal) => release.operation_for(Some(signal)).is_some(),
                },
            "telemetry resolution requires an exact verified contract"
        );
        let lease = control
            .lease_telemetry_installation(connection, &installation)
            .ok_or_else(|| anyhow::anyhow!("telemetry resolution installation unavailable"))?;
        Ok(Self {
            installation,
            release,
            lease,
            ended: AtomicBool::new(false),
        })
    }

    fn current(&self, catalog: &PluginCatalog, live: bool) -> bool {
        let valid = !self.ended.load(Ordering::Acquire) && live && self.release.current(catalog);
        if !valid {
            self.ended.store(true, Ordering::Release);
        }
        valid
    }
}

fn ended() -> CommandRequestError {
    CommandRequestError {
        certainty: CommandFailure::NotSent,
        detail: "resolved telemetry port ended".into(),
    }
}

pub(crate) struct ResolvedBinding {
    step: BindingStep,
    connection: ConnectionToken,
    port: Option<ResolvedPort>,
}

impl ResolvedBinding {
    pub(crate) fn resolve(
        catalog: &PluginCatalog,
        control: &MachineControl,
        step: BindingStep,
    ) -> Result<Self> {
        step.validate_commit()?;
        let connection = control
            .scoped_operation_connection(&step.service_id, &step.machine_id)
            .map_err(|_| anyhow::anyhow!("telemetry resolution Machine unavailable"))?;
        let port = step
            .after()?
            .selection
            .map(|target| {
                ResolvedPort::resolve(catalog, control, &connection, target, Requirement::Binding)
            })
            .transpose()?;
        let resolved = Self {
            step,
            connection,
            port,
        };
        ensure!(
            resolved.current(catalog, control),
            "telemetry resolution ended"
        );
        Ok(resolved)
    }

    pub(crate) fn step(&self) -> &BindingStep {
        &self.step
    }

    pub(crate) fn connection(&self) -> &ConnectionToken {
        &self.connection
    }

    pub(crate) fn installation(&self) -> Option<&BindingInstallation> {
        self.port.as_ref().map(|port| &port.installation)
    }

    pub(crate) fn current(&self, catalog: &PluginCatalog, control: &MachineControl) -> bool {
        let live = control.telemetry_binding_target_current(
            &self.connection,
            &self.step,
            self.port.as_ref().map(|port| &port.lease),
        );
        self.port
            .as_ref()
            .map_or(live, |port| port.current(catalog, live))
    }

    pub(crate) async fn dispatch(
        &self,
        catalog: &PluginCatalog,
        control: &MachineControl,
    ) -> Result<BindingObservation, CommandRequestError> {
        if !self.current(catalog, control) {
            return Err(ended());
        }
        // Carry the original lease all the way into atomic request enqueue.
        // Never recapture it after an await or accept a substitute wire step.
        control
            .commit_telemetry_binding(
                &self.connection,
                &self.step,
                self.port.as_ref().map(|port| &port.lease),
            )
            .await
    }
}

pub(crate) struct ResolvedExport {
    attempt: ExportAttempt,
    connection: ConnectionToken,
    port: ResolvedPort,
}

impl ResolvedExport {
    pub(crate) fn resolve(
        catalog: &PluginCatalog,
        control: &MachineControl,
        attempt: ExportAttempt,
    ) -> Result<Self> {
        attempt.validate()?;
        let connection = control
            .scoped_operation_connection(&attempt.service_id, &attempt.machine_id)
            .map_err(|_| anyhow::anyhow!("telemetry resolution Machine unavailable"))?;
        let target = attempt
            .binding
            .selection
            .clone()
            .ok_or_else(|| anyhow::anyhow!("telemetry resolution selection unavailable"))?;
        let port = ResolvedPort::resolve(
            catalog,
            control,
            &connection,
            target,
            Requirement::Otlp(attempt.payload.signal),
        )?;
        let resolved = Self {
            attempt,
            connection,
            port,
        };
        ensure!(
            resolved.current(catalog, control),
            "telemetry resolution ended"
        );
        Ok(resolved)
    }

    pub(crate) fn attempt(&self) -> &ExportAttempt {
        &self.attempt
    }

    pub(crate) fn installation(&self) -> &BindingInstallation {
        &self.port.installation
    }

    pub(crate) fn current(&self, catalog: &PluginCatalog, control: &MachineControl) -> bool {
        self.port.current(
            catalog,
            control.telemetry_export_target_current(
                &self.connection,
                &self.attempt,
                &self.port.lease,
            ),
        )
    }

    pub(crate) async fn dispatch(
        &self,
        catalog: &PluginCatalog,
        control: &MachineControl,
    ) -> Result<ExportReceipt, CommandRequestError> {
        if !self.current(catalog, control) {
            return Err(ended());
        }
        control
            .export_bound_telemetry(&self.connection, &self.attempt, &self.port.lease)
            .await
    }
}
