//! Bind existing finite Plugin operations to this core Service and the actual
//! authenticated Machine route. No proposed graph, wire identity or historical
//! receipt can create a local Service owner. This is not authorization.
use super::{ConnectionToken, MachineCommand, MachineControl};

/// Borrowed claims only. Field names keep the two identity axes distinct;
/// validation compares them with the established owner and live route.
struct ClaimedSite<'a> {
    service: &'a str,
    machine: &'a str,
}

fn claimed_site(command: &MachineCommand) -> Option<ClaimedSite<'_>> {
    let (service, machine) = match command {
        MachineCommand::ObservePluginInstallation { query, .. } => {
            (&query.service_id, &query.machine_id)
        }
        MachineCommand::InstallPluginStep { step, .. }
        | MachineCommand::QueryPluginInstallStep { step, .. } => {
            (&step.service_id, &step.machine_id)
        }
        MachineCommand::UninstallPluginStep { step, .. }
        | MachineCommand::QueryPluginUninstallStep { step, .. }
        | MachineCommand::QueryPluginUninstallRecovery { step, .. } => {
            (&step.service_id, &step.machine_id)
        }
        MachineCommand::QueryTelemetryBinding { step, .. }
        | MachineCommand::CommitTelemetryBinding { step, .. } => {
            (&step.service_id, &step.machine_id)
        }
        MachineCommand::RecoverTelemetryBinding { recovery, .. }
        | MachineCommand::QueryTelemetryRecovery { recovery, .. } => {
            (&recovery.step.service_id, &recovery.step.machine_id)
        }
        MachineCommand::QueryTelemetryRecoveryAudit { query, .. } => {
            (&query.step.service_id, &query.step.machine_id)
        }
        MachineCommand::ExportBoundTelemetry { attempt, .. } => {
            (&attempt.service_id, &attempt.machine_id)
        }
        // Existing unscoped wire commands still use their own authenticated
        // transport/domain checks. Never invent a Site by inspecting opaque
        // adapter payloads or encrypted Provider credentials. Exhaustive match:
        // adding a wire command requires an explicit core classification.
        MachineCommand::Reconcile { .. }
        | MachineCommand::BeginLogin { .. }
        | MachineCommand::CancelLogin { .. }
        | MachineCommand::SubmitLoginCode { .. }
        | MachineCommand::UpdateNpmComponent { .. }
        | MachineCommand::RefreshInventory { .. }
        | MachineCommand::InstallPlugin { .. }
        | MachineCommand::UninstallPlugin { .. }
        | MachineCommand::ReactivatePlugin { .. }
        | MachineCommand::ApplyProviderAuth { .. }
        | MachineCommand::FinalizeProviderAuthCandidate { .. }
        | MachineCommand::AdapterRequest { .. }
        | MachineCommand::InvokePluginHost { .. }
        | MachineCommand::ProviderUsageAck { .. } => return None,
    };
    Some(ClaimedSite { service, machine })
}

impl MachineControl {
    pub(super) fn matches_site(
        &self,
        connection: &ConnectionToken,
        service: &str,
        machine: &str,
    ) -> bool {
        self.service.as_str() == service && connection.0.machine_id == machine
    }

    /// Finite resolution captures this owner's original authenticated route.
    /// Reopening a Service retains its durable name, not its old connections.
    pub(crate) fn scoped_operation_connection(
        &self,
        service: &str,
        machine: &str,
    ) -> Result<ConnectionToken, String> {
        if self.service.as_str() != service {
            return Err("Machine operation Service owner mismatch".into());
        }
        self.operation_connection(machine)
    }

    // Called inside BOTH outgoing channel locks, before registering a waiter
    // or sending bytes. Generic command/send entrypoints cannot bypass this.
    pub(super) fn check_command_site(
        &self,
        connection: &ConnectionToken,
        command: &MachineCommand,
    ) -> Result<(), String> {
        if claimed_site(command)
            .is_some_and(|site| !self.matches_site(connection, site.service, site.machine))
        {
            return Err("Machine operation Site mismatch".into());
        }
        Ok(())
    }
}
