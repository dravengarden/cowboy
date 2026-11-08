//! Typed replies correlated to Machine requests.

use super::AdapterFailure;
use crate::machine_protocol::machine_browser;
use crate::machine_protocol::plugin_recovery::RecoveryObservation;
use crate::machine_protocol::plugin_step::StepObservation;
use crate::machine_protocol::telemetry_binding::{BindingCommitResult, BindingObservation};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    NativeBinding,
    NativeLogout,
    LoginStop,
    WorkerExit,
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
    NativeBinding(Option<Box<crate::provider_account::PreparedBinding>>),
    NativeLogout(Option<crate::provider_account::NativeLogoutObservation>),
    LoginStop(Option<crate::provider_account::LoginStopObservation>),
    WorkerExit(Option<crate::provider_account::WorkerExitObservation>),
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
            Self::NativeProfile(_) => ReplyKind::NativeProfile,
            Self::NativeLogout(_) => ReplyKind::NativeLogout,
            Self::LoginStop(_) => ReplyKind::LoginStop,
            Self::WorkerExit(_) => ReplyKind::WorkerExit,
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
