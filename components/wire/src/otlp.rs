//! Standard OTLP protobuf messages. No gRPC transport or external Collector.
use anyhow::{Result, ensure};
use base64::Engine as _;
use opentelemetry_proto::tonic::{
    collector::{
        logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest,
        trace::v1::ExportTraceServiceRequest,
    },
    metrics::v1::metric,
};
use prost::Message as _;
use serde::{Deserialize, Serialize};

pub const MAX_BYTES: usize = 256 * 1024;
pub const MAX_ITEMS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    Logs,
    Metrics,
    Traces,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Export {
    pub signal: Signal,
    /// Base64 transports protobuf through the existing authenticated JSON RPC.
    pub protobuf: String,
}

// A protocol Debug dump must not copy telemetry bodies into diagnostics.
impl std::fmt::Debug for Export {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Export")
            .field("signal", &self.signal)
            .field("encoded_bytes", &self.protobuf.len())
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "machine-host")]
pub fn signal_rejected(signal: Signal, bytes: &[u8], sent: usize) -> Result<u64> {
    use opentelemetry_proto::tonic::collector::{
        logs::v1::ExportLogsServiceResponse, metrics::v1::ExportMetricsServiceResponse,
        trace::v1::ExportTraceServiceResponse,
    };
    ensure!(bytes.len() <= 64 * 1024, "OTLP response too large");
    let rejected = match signal {
        Signal::Logs => ExportLogsServiceResponse::decode(bytes)?
            .partial_success
            .map_or(0, |v| v.rejected_log_records),
        Signal::Metrics => ExportMetricsServiceResponse::decode(bytes)?
            .partial_success
            .map_or(0, |v| v.rejected_data_points),
        Signal::Traces => ExportTraceServiceResponse::decode(bytes)?
            .partial_success
            .map_or(0, |v| v.rejected_spans),
    };
    let rejected = u64::try_from(rejected)?;
    ensure!(
        rejected <= u64::try_from(sent)?,
        "invalid OTLP rejected count"
    );
    Ok(rejected)
}

pub enum Request {
    Logs(ExportLogsServiceRequest),
    Metrics(ExportMetricsServiceRequest),
    Traces(ExportTraceServiceRequest),
}

impl Export {
    pub fn decode(&self) -> Result<(Vec<u8>, usize)> {
        ensure!(
            self.protobuf.len() <= MAX_BYTES.div_ceil(3) * 4,
            "OTLP export too large"
        );
        let bytes = base64::engine::general_purpose::STANDARD.decode(&self.protobuf)?;
        let request = Request::decode(self.signal, &bytes)?;
        Ok((bytes, request.count()))
    }
}

impl Request {
    pub fn decode(signal: Signal, bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_BYTES, "OTLP request too large");
        let request = match signal {
            Signal::Logs => Self::Logs(ExportLogsServiceRequest::decode(bytes)?),
            Signal::Metrics => Self::Metrics(ExportMetricsServiceRequest::decode(bytes)?),
            Signal::Traces => Self::Traces(ExportTraceServiceRequest::decode(bytes)?),
        };
        ensure!(request.count() <= MAX_ITEMS, "too many OTLP items");
        Ok(request)
    }

    pub fn count(&self) -> usize {
        match self {
            Self::Logs(r) => r
                .resource_logs
                .iter()
                .flat_map(|r| &r.scope_logs)
                .map(|s| s.log_records.len())
                .sum(),
            Self::Traces(r) => r
                .resource_spans
                .iter()
                .flat_map(|r| &r.scope_spans)
                .map(|s| s.spans.len())
                .sum(),
            Self::Metrics(r) => r
                .resource_metrics
                .iter()
                .flat_map(|r| &r.scope_metrics)
                .flat_map(|s| &s.metrics)
                .map(|m| match &m.data {
                    Some(metric::Data::Sum(v)) => v.data_points.len(),
                    Some(metric::Data::Histogram(v)) => v.data_points.len(),
                    Some(metric::Data::Gauge(v)) => v.data_points.len(),
                    Some(metric::Data::ExponentialHistogram(v)) => v.data_points.len(),
                    Some(metric::Data::Summary(v)) => v.data_points.len(),
                    None => 0,
                })
                .sum(),
        }
    }

    #[cfg(feature = "full")]
    pub fn export(&self) -> Export {
        let (signal, bytes) = match self {
            Self::Logs(r) => (Signal::Logs, r.encode_to_vec()),
            Self::Metrics(r) => (Signal::Metrics, r.encode_to_vec()),
            Self::Traces(r) => (Signal::Traces, r.encode_to_vec()),
        };
        Export {
            signal,
            protobuf: base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }
}
