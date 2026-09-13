//! Closed, payload-free evidence for actual immutable managed OTLP transport.
use super::*;
use crate::machine_protocol::telemetry_export::ExportOutcome;
use crate::otlp::Signal;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in super::super) enum ResponseMode {
    Success,
    Partial,
    Unavailable,
    RateLimited,
    Redirect,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in super::super) enum Ack {
    #[default]
    Forward,
    Drop,
    Disconnect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in super::super) enum DeliveryStep {
    Unconfigured,
    BindingOnly,
    Delivered,
    Partial,
    Unavailable,
    RateLimited,
    Redirect,
    LostAck,
    Disconnected,
    Reconnected,
    Revoked,
    RestoredStalePolicy,
    ReopenedStalePolicy,
    Reactivated,
    ReopenedActive,
    MetricsOnly,
}

impl DeliveryStep {
    pub fn response(self) -> ResponseMode {
        match self {
            Self::Partial => ResponseMode::Partial,
            Self::Unavailable => ResponseMode::Unavailable,
            Self::RateLimited => ResponseMode::RateLimited,
            Self::Redirect => ResponseMode::Redirect,
            _ => ResponseMode::Success,
        }
    }

    pub fn exports(self, signal: Signal) -> bool {
        match self {
            Self::Unconfigured
            | Self::BindingOnly
            | Self::Revoked
            | Self::RestoredStalePolicy
            | Self::ReopenedStalePolicy => false,
            Self::MetricsOnly => signal == Signal::Metrics,
            _ => true,
        }
    }

    pub fn failed(self) -> bool {
        matches!(
            self,
            Self::Unavailable
                | Self::RateLimited
                | Self::Redirect
                | Self::LostAck
                | Self::Disconnected
                | Self::Revoked
                | Self::RestoredStalePolicy
        )
    }

    pub fn ack(self) -> Ack {
        match self {
            Self::LostAck => Ack::Drop,
            Self::Disconnected => Ack::Disconnect,
            _ => Ack::Forward,
        }
    }

    pub fn receipt(self) -> ExportOutcome {
        match self.response() {
            ResponseMode::Partial => ExportOutcome::Partial { rejected_items: 1 },
            ResponseMode::Unavailable | ResponseMode::RateLimited | ResponseMode::Redirect => {
                ExportOutcome::Unknown {}
            }
            ResponseMode::Success => ExportOutcome::Delivered {},
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(in super::super) struct HttpExport {
    pub signal: Signal,
    pub payload_sha256: String,
    pub items: usize,
    pub response: ResponseMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(in super::super) struct WireExport {
    pub signal: Signal,
    pub request_sha256: String,
    pub payload_sha256: String,
    pub items: usize,
    pub receipt: Option<ExportOutcome>,
    pub ack: Option<Ack>,
}

#[derive(Serialize)]
pub(in super::super) struct DeliveryRound {
    pub step: DeliveryStep,
    pub accepted: bool,
    pub submitted_batches: usize,
    pub elapsed_ms: u64,
    pub local_bytes_added: u64,
    pub local_records_added: usize,
    pub duplicate_batches_added: u64,
    pub remote_failures_added: [u64; 3],
    pub rejected_items_added: u64,
    pub export_commands_added: usize,
    pub http_requests_added: usize,
}

#[derive(Default, Serialize)]
pub(in super::super) struct DeliveryReport {
    pub rounds: Vec<DeliveryRound>,
    pub exports: Vec<WireExport>,
    pub http: Vec<HttpExport>,
    pub background_policy_sha256: Vec<String>,
}
