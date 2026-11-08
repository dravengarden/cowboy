//! `MachineCommand` family routed by `handle_machine_command`: Telemetry binding, recovery and export.

use super::*;

pub(super) fn handle(
    command: MachineCommand,
    context: MachineCommandContext,
    execution: &PluginExecutionScope,
) {
    let MachineCommandContext {
        service_id,
        machine_id,
        events,
        providers,
        ..
    } = context;
    match command {
        MachineCommand::RecoverTelemetryBinding {
            request_id,
            recovery,
        } => {
            telemetry_recovery::recover(request_id, *recovery, providers, execution, events);
        }
        MachineCommand::QueryTelemetryRecovery {
            request_id,
            recovery,
        } => {
            telemetry_recovery::query(
                request_id, *recovery, providers, service_id, machine_id, events,
            );
        }
        MachineCommand::QueryTelemetryRecoveryAudit { request_id, query } => {
            telemetry_recovery::query_audit(
                request_id, *query, providers, service_id, machine_id, events,
            );
        }
        MachineCommand::ExportBoundTelemetry {
            request_id,
            attempt,
        } => {
            telemetry_export::export(request_id, *attempt, providers, execution, events);
        }
        MachineCommand::CommitTelemetryBinding { request_id, step } => {
            telemetry_binding::commit(request_id, *step, providers, execution, events);
        }
        MachineCommand::QueryTelemetryBinding { request_id, step } => {
            tokio::spawn(async move {
                let observation = providers
                    .telemetry_binding_observation(&step, service_id.as_deref(), &machine_id)
                    .await;
                let _ = events.send(MachineEvent::TelemetryBindingObservation {
                    request_id,
                    observation: Box::new(observation),
                });
            });
        }
        other => tracing::error!(
            command = ?std::mem::discriminant(&other),
            "machine command routed to the wrong family (command_telemetry)"
        ),
    }
}
