//! Typed replies correlated to Machine requests.

use super::AdapterFailure;
use crate::machine_protocol::MachineEvent;
use crate::machine_protocol::machine_browser;
use crate::machine_protocol::plugin_recovery::RecoveryObservation;
use crate::machine_protocol::plugin_step::StepObservation;
use crate::machine_protocol::telemetry_binding::{BindingCommitResult, BindingObservation};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    Mcp,
    NativeBinding,
    NativeLogout,
    LoginStop,
    WorkerExit,
    AccountFence,
    NativeProfile,
    Execution,
    Adapter,
    Command,
    PluginHost,
    PluginStep,
    InstallationTarget,
    InstallationStep,
    PluginRecovery,
    TelemetryBinding,
    TelemetryBindingCommit,
    TelemetryExport,
    TelemetryRecovery,
    TelemetryRecoveryCommit,
    TelemetryRecoveryAudit,
    Browser,
}

pub enum Reply {
    Mcp(serde_json::Value),
    NativeBinding(Option<Box<crate::provider_account::PreparedBinding>>),
    NativeLogout(Option<crate::provider_account::NativeLogoutObservation>),
    LoginStop(Option<crate::provider_account::LoginStopObservation>),
    WorkerExit(Option<crate::provider_account::WorkerExitObservation>),
    /// The daemon's confirmation, or the Machine's refusal detail.
    AccountFence(Result<crate::provider_account::AccountFenceObservation, String>),
    NativeProfile(Option<Box<crate::provider_account::NativeProfileReceipt>>),
    Execution(Box<crate::machine_protocol::execution::Response>),
    InstallationTarget(Box<crate::machine_protocol::plugin_install::InstallTargetObservation>),
    InstallationStep(Box<crate::machine_protocol::plugin_install::InstallObservation>),
    PluginStep(Box<StepObservation>),
    PluginRecovery(Box<RecoveryObservation>),
    TelemetryBinding(Box<BindingObservation>),
    TelemetryBindingCommit(Box<BindingCommitResult>),
    TelemetryExport(Option<Box<crate::machine_protocol::telemetry_export::ExportReceipt>>),
    TelemetryRecovery(Box<crate::machine_protocol::telemetry_recovery::RecoveryObservation>),
    TelemetryRecoveryCommit(Box<crate::machine_protocol::telemetry_recovery::RecoveryResult>),
    TelemetryRecoveryAudit(
        Box<crate::machine_protocol::telemetry_recovery_audit::RecoveryAuditObservation>,
    ),
    Adapter(Result<serde_json::Value, AdapterFailure>),
    Command(Result<(), String>),
    /// The Machine's browser state answering a browser command, or the
    /// generic rejection it sent instead.
    Browser(Result<Box<machine_browser::StateEvent>, String>),
    PluginHost {
        accepted: bool,
        started: bool,
        payload: Option<serde_json::Value>,
        detail: Option<String>,
    },
}

impl Reply {
    pub(super) const fn kind(&self) -> ReplyKind {
        match self {
            Self::Mcp(_) => ReplyKind::Mcp,
            Self::NativeProfile(_) => ReplyKind::NativeProfile,
            Self::NativeLogout(_) => ReplyKind::NativeLogout,
            Self::LoginStop(_) => ReplyKind::LoginStop,
            Self::WorkerExit(_) => ReplyKind::WorkerExit,
            Self::AccountFence(_) => ReplyKind::AccountFence,
            Self::NativeBinding(_) => ReplyKind::NativeBinding,
            Self::Execution(_) => ReplyKind::Execution,
            Self::InstallationTarget(_) => ReplyKind::InstallationTarget,
            Self::InstallationStep(_) => ReplyKind::InstallationStep,
            Self::PluginStep(_) => ReplyKind::PluginStep,
            Self::PluginRecovery(_) => ReplyKind::PluginRecovery,
            Self::TelemetryBinding(_) => ReplyKind::TelemetryBinding,
            Self::TelemetryBindingCommit(_) => ReplyKind::TelemetryBindingCommit,
            Self::TelemetryExport(_) => ReplyKind::TelemetryExport,
            Self::TelemetryRecovery(_) => ReplyKind::TelemetryRecovery,
            Self::TelemetryRecoveryCommit(_) => ReplyKind::TelemetryRecoveryCommit,
            Self::TelemetryRecoveryAudit(_) => ReplyKind::TelemetryRecoveryAudit,
            Self::Adapter(_) => ReplyKind::Adapter,
            Self::Command(_) => ReplyKind::Command,
            Self::Browser(_) => ReplyKind::Browser,
            Self::PluginHost { .. } => ReplyKind::PluginHost,
        }
    }
}

impl Reply {
    /// The typed reply a Machine observation event carries for its request,
    /// or the event itself when it is not one.
    #[allow(clippy::result_large_err)] // The unmatched event returns to its caller unchanged.
    pub(super) fn from_observation(event: MachineEvent) -> Result<(String, Self), MachineEvent> {
        Ok(match event {
            MachineEvent::NativeBindingPrepared {
                request_id,
                binding,
            } => (request_id, Self::NativeBinding(binding)),
            MachineEvent::WorkerExitObserved {
                request_id,
                observation,
            } => (request_id, Self::WorkerExit(observation)),
            MachineEvent::AccountSandboxFenceObserved {
                request_id,
                observation,
                detail,
            } => (
                request_id,
                Self::AccountFence(observation.ok_or_else(|| detail.unwrap_or_default())),
            ),
            MachineEvent::LoginStopObserved {
                request_id,
                observation,
            } => (request_id, Self::LoginStop(observation)),
            MachineEvent::NativeLogoutObserved {
                request_id,
                observation,
            } => (request_id, Self::NativeLogout(observation)),
            MachineEvent::NativeProfileObserved {
                request_id,
                profile,
            } => (request_id, Self::NativeProfile(profile)),
            MachineEvent::TelemetryRecoveryAuditObservation {
                request_id,
                observation,
            } => (request_id, Self::TelemetryRecoveryAudit(observation)),
            MachineEvent::TelemetryBindingRecovered { request_id, result } => {
                (request_id, Self::TelemetryRecoveryCommit(result))
            }
            MachineEvent::TelemetryRecoveryObservation {
                request_id,
                observation,
            } => (request_id, Self::TelemetryRecovery(observation)),
            MachineEvent::TelemetryExported {
                request_id,
                receipt,
            } => (request_id, Self::TelemetryExport(receipt)),
            MachineEvent::TelemetryBindingCommitted { request_id, result } => {
                (request_id, Self::TelemetryBindingCommit(result))
            }
            MachineEvent::TelemetryBindingObservation {
                request_id,
                observation,
            } => (request_id, Self::TelemetryBinding(observation)),
            MachineEvent::PluginUninstallRecovery {
                request_id,
                observation,
            } => (request_id, Self::PluginRecovery(observation)),
            MachineEvent::PluginInstallationTarget {
                request_id,
                observation,
            } => (request_id, Self::InstallationTarget(observation)),
            MachineEvent::PluginInstallationStep {
                request_id,
                observation,
            } => (request_id, Self::InstallationStep(observation)),
            MachineEvent::PluginUninstallStep {
                request_id,
                observation,
            } => (request_id, Self::PluginStep(observation)),
            event => return Err(event),
        })
    }
}
