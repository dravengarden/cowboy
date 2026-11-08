//! Mixed-generation command admission.
use crate::machine_protocol::*;

impl MachineCommand {
    /// Old Machines may still negotiate an earlier wire version. Keep the
    /// capability check beside the tagged command so the Controller never
    /// sends a Service-auth or Provider-lifecycle message an older peer could
    /// deserialize incorrectly.
    #[must_use]
    #[allow(clippy::too_many_lines)] // One exhaustive table keeps mixed-generation admission auditable.
    pub fn minimum_protocol(&self) -> u16 {
        match self {
            Self::McpTransport {
                request:
                    crate::mcp_binding::TransportRequest::PrepareNative { .. }
                    | crate::mcp_binding::TransportRequest::ConfirmNative { .. },
                ..
            } => MCP_NATIVE_CONFIRMATION_PROTOCOL_VERSION,
            Self::McpTransport { .. } => MCP_TRANSPORT_PROTOCOL_VERSION,
            Self::CollectNativeUsage { .. } => NATIVE_USAGE_PROTOCOL_VERSION,
            Self::BeginNativeLogin { .. }
            | Self::QueryNativeProfile { .. }
            | Self::NativeLogout { .. }
            | Self::QueryLoginStop { .. }
            | Self::QueryWorkerExit { .. }
            | Self::PrepareNativeBinding { .. }
            | Self::CancelLoginRequest { .. } => PROVIDER_ACCOUNT_PROTOCOL_VERSION,
            Self::ApplyProviderAuth { envelope, .. }
                if envelope.envelope_schema >= 2 || !envelope.account_id.is_default() =>
            {
                PROVIDER_ACCOUNT_PROTOCOL_VERSION
            }
            Self::BeginLogin { account_id, .. }
            | Self::FinalizeProviderAuthCandidate { account_id, .. }
                if !account_id.is_default() =>
            {
                PROVIDER_ACCOUNT_PROTOCOL_VERSION
            }
            Self::Execution { request, .. }
                if matches!(
                    &request.action,
                    execution::Action::DeliverManagedArtifact { .. }
                ) || matches!(&request.action, execution::Action::PrepareManagedRound { round }
                        if round.profile == cowboy_provider_sdk::ManagedRuntimeProfile::ImageV1) =>
            {
                MANAGED_IMAGE_PROTOCOL_VERSION
            }
            Self::AdapterRequest { adapter, .. } if adapter == session_code::ADAPTER => {
                session_code::PROTOCOL_VERSION
            }
            Self::AdapterRequest { adapter, .. } if adapter == code_observer::ADAPTER => {
                code_observer::PROTOCOL_VERSION
            }
            Self::Projects { .. } => PROJECT_REGISTRY_PROTOCOL_VERSION,
            Self::Execution { request, .. }
                if matches!(request.action, execution::Action::Recover { .. }) =>
            {
                EXECUTION_RECOVERY_PROTOCOL_VERSION
            }
            Self::Execution { request, .. }
                if matches!(
                    request.action,
                    execution::Action::PrepareManagedEnvironment { .. }
                ) =>
            {
                MANAGED_ENVIRONMENT_PROTOCOL_VERSION
            }
            Self::Execution { request, .. }
                if matches!(&request.action,
                execution::Action::InstallCallGateway { grant } | execution::Action::RevokeCallGateway { grant } if grant.mcp) =>
            {
                MCP_TRANSPORT_PROTOCOL_VERSION
            }
            Self::Execution { request, .. } if request.action.managed_call() => {
                MANAGED_CALL_PROTOCOL_VERSION
            }
            Self::ManagedCallReply { .. } => MANAGED_CALL_PROTOCOL_VERSION,
            Self::MemoryRelay { .. } | Self::MemoryRelayReply { .. } => {
                MEMORY_RELAY_PROTOCOL_VERSION
            }
            Self::Execution { .. } => EXECUTION_ENVIRONMENT_PROTOCOL_VERSION,
            // Only a carried root identity needs the newer Machine. Ordinary
            // adapter traffic keeps its original floor below.
            Self::AdapterRequest {
                workspace_incarnation: Some(_),
                ..
            } => CODE_WORKSPACE_ROOT_IDENTITY_PROTOCOL_VERSION,
            Self::CodeBufferNavigation { .. } => CODE_BUFFER_NAVIGATION_PROTOCOL_VERSION,
            Self::CodeBufferSync { .. } => CODE_BUFFER_SYNC_PROTOCOL_VERSION,
            Self::QueryTelemetryRecoveryAudit { .. } => TELEMETRY_RECOVERY_AUDIT_PROTOCOL_VERSION,
            Self::ExportBoundTelemetry { .. } => TELEMETRY_BOUND_EXPORT_PROTOCOL_VERSION,
            Self::RecoverTelemetryBinding { .. } | Self::QueryTelemetryRecovery { .. } => {
                TELEMETRY_BINDING_RECOVERY_PROTOCOL_VERSION
            }
            Self::CommitTelemetryBinding { .. } => TELEMETRY_BINDING_COMMIT_PROTOCOL_VERSION,
            Self::QueryTelemetryBinding { step, .. } => {
                if step.schema == 1 {
                    TELEMETRY_BINDING_OBSERVATION_PROTOCOL_VERSION
                } else {
                    TELEMETRY_BINDING_COMMIT_PROTOCOL_VERSION
                }
            }
            Self::ObservePluginInstallation { .. }
            | Self::InstallPluginStep { .. }
            | Self::QueryPluginInstallStep { .. } => PLUGIN_INSTALL_ATTEMPT_PROTOCOL_VERSION,
            Self::QueryPluginUninstallRecovery { .. } => {
                PLUGIN_RECOVERY_OBSERVATION_PROTOCOL_VERSION
            }
            Self::UninstallPluginStep { step, .. }
            | Self::QueryPluginUninstallStep { step, .. } => {
                if step.schema == 1 {
                    PLUGIN_STEP_PROTOCOL_VERSION
                } else {
                    PLUGIN_INSTALLATION_PROTOCOL_VERSION
                }
            }
            Self::InvokePluginHost {
                operation: PluginHostOperation::ExportOtlp,
                ..
            } => OTLP_PLUGIN_PROTOCOL_VERSION,
            Self::InvokePluginHost {
                operation: PluginHostOperation::ExportTelemetry,
                ..
            } => TELEMETRY_PLUGIN_PROTOCOL_VERSION,
            // Uninstall is destructive and its Controller saga relies on
            // exact-generation reactivation for compensation. Never let a
            // protocol-three Machine begin removal that it cannot undo.
            Self::InstallPlugin { .. } | Self::InvokePluginHost { .. } => {
                PLUGIN_HOST_EXECUTION_PROTOCOL_VERSION
            }
            Self::UninstallPlugin { .. } | Self::ReactivatePlugin { .. } => 5,
            Self::BeginLogin { .. }
            | Self::CancelLogin { .. }
            | Self::SubmitLoginCode { .. }
            | Self::ApplyProviderAuth { .. }
            | Self::FinalizeProviderAuthCandidate { .. } => 3,
            Self::Reconcile { .. }
            | Self::UpdateNpmComponent { .. }
            | Self::RefreshInventory { .. }
            | Self::AdapterRequest { .. }
            | Self::ProviderUsageAck { .. } => 1,
        }
    }
}
