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

pub(crate) const MAX_BYTES: usize = 256 * 1024;
pub(crate) const MAX_ITEMS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Signal {
    Logs,
    Metrics,
    Traces,
}

impl Signal {
    #[cfg(feature = "machine-host")]
    pub(crate) fn rejected(self, bytes: &[u8], sent: usize) -> Result<u64> {
        use opentelemetry_proto::tonic::collector::{
            logs::v1::ExportLogsServiceResponse, metrics::v1::ExportMetricsServiceResponse,
            trace::v1::ExportTraceServiceResponse,
        };
        ensure!(bytes.len() <= 64 * 1024, "OTLP response too large");
        let rejected = match self {
            Self::Logs => ExportLogsServiceResponse::decode(bytes)?
                .partial_success
                .map_or(0, |v| v.rejected_log_records),
            Self::Metrics => ExportMetricsServiceResponse::decode(bytes)?
                .partial_success
                .map_or(0, |v| v.rejected_data_points),
            Self::Traces => ExportTraceServiceResponse::decode(bytes)?
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
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Export {
    pub signal: Signal,
    /// Base64 transports protobuf through the existing authenticated JSON RPC.
    pub protobuf: String,
}

pub(crate) enum Request {
    Logs(ExportLogsServiceRequest),
    Metrics(ExportMetricsServiceRequest),
    Traces(ExportTraceServiceRequest),
}

impl Export {
    #[cfg(feature = "machine-host")]
    pub(crate) fn decode(&self) -> Result<(Vec<u8>, usize)> {
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
    pub(crate) fn decode(signal: Signal, bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_BYTES, "OTLP request too large");
        let request = match signal {
            Signal::Logs => Self::Logs(ExportLogsServiceRequest::decode(bytes)?),
            Signal::Metrics => Self::Metrics(ExportMetricsServiceRequest::decode(bytes)?),
            Signal::Traces => Self::Traces(ExportTraceServiceRequest::decode(bytes)?),
        };
        ensure!(request.count() <= MAX_ITEMS, "too many OTLP items");
        Ok(request)
    }

    pub(crate) fn count(&self) -> usize {
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
    pub(crate) fn export(&self) -> Export {
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

#[cfg(feature = "full")]
mod controller {
    use super::*;
    use crate::observability::{redact_message, sanitize_attributes};
    use opentelemetry_proto::tonic::{
        common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value},
        metrics::v1::{self as metrics, number_data_point},
        resource::v1::Resource,
    };
    use sha2::{Digest as _, Sha256};
    use std::collections::{BTreeMap, BTreeSet};
    use std::fmt::Write as _;

    pub(crate) fn now_nanos() -> u64 {
        chrono::Utc::now()
            .timestamp_nanos_opt()
            .and_then(|v| u64::try_from(v).ok())
            .unwrap_or(0)
    }

    /// Conservative W3C v00 ingress: ignore unsampled, invalid or future
    /// contexts. Correlation never grants visibility or imports baggage.
    pub(crate) fn command_span(principal: &str, parent: &str, start: u64) -> Option<Request> {
        use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
        if parent.len() != 55
            || !parent.starts_with("00-")
            || parent.as_bytes()[35] != b'-'
            || parent.as_bytes()[52] != b'-'
        {
            return None;
        }
        let hex = |s: &str| -> Option<Vec<u8>> {
            if !s
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return None;
            }
            s.as_bytes()
                .chunks_exact(2)
                .map(|b| u8::from_str_radix(std::str::from_utf8(b).ok()?, 16).ok())
                .collect()
        };
        // Check ASCII before slicing an untrusted UTF-8 carrier.
        if !parent.is_ascii() {
            return None;
        }
        let trace_id = hex(&parent[3..35])?;
        let parent_span_id = hex(&parent[36..52])?;
        let flags = *hex(&parent[53..55])?.first()?;
        if flags & 1 == 0 || ids(&trace_id, &parent_span_id, true).is_err() {
            return None;
        }
        Some(Request::Traces(ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource {
                    attributes: vec![text("service.name", "cowboy-controller")],
                    ..Default::default()
                }),
                scope_spans: vec![ScopeSpans {
                    scope: Some(InstrumentationScope {
                        name: "cowboy.controller".into(),
                        version: "1".into(),
                        ..Default::default()
                    }),
                    spans: vec![Span {
                        trace_id,
                        parent_span_id,
                        span_id: uuid::Uuid::new_v4().as_bytes()[..8].to_vec(),
                        name: "cowboy.controller.dispatch".into(),
                        kind: 2,
                        flags: 1,
                        start_time_unix_nano: start,
                        end_time_unix_nano: now_nanos().max(start),
                        attributes: vec![
                            text(
                                "cowboy.owner",
                                &format!("{:x}", Sha256::digest(principal.as_bytes())),
                            ),
                            text("cowboy.parent_trust", "client"),
                            text("cowboy.stage", "websocket_dispatch"),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }))
    }

    pub(crate) fn text(key: &str, value: &str) -> KeyValue {
        KeyValue {
            key: key.into(),
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(value.into())),
            }),
            ..Default::default()
        }
    }

    fn value_text<'a>(attrs: &'a [KeyValue], key: &str) -> Option<&'a str> {
        attrs
            .iter()
            .find(|a| a.key == key)
            .and_then(|a| a.value.as_ref())
            .and_then(|v| match &v.value {
                Some(any_value::Value::StringValue(s)) => Some(s.as_str()),
                _ => None,
            })
    }

    fn scope(value: &mut Option<InstrumentationScope>) {
        *value = Some(InstrumentationScope {
            name: "cowboy.web".into(),
            version: "1".into(),
            ..Default::default()
        });
    }

    fn resource(value: &mut Option<Resource>) -> (String, String) {
        let attrs = value.as_ref().map_or(&[][..], |r| r.attributes.as_slice());
        let platform = match value_text(attrs, "cowboy.platform") {
            Some("ios") => "ios",
            Some("macos") => "macos",
            Some("web") => "web",
            _ => "other",
        }
        .to_owned();
        let surface = match value_text(attrs, "cowboy.surface") {
            Some("mobile") => "mobile",
            Some("desktop") => "desktop",
            _ => "other",
        }
        .to_owned();
        *value = Some(Resource {
            attributes: vec![
                text("service.name", "cowboy-web"),
                text("cowboy.platform", &platform),
                text("cowboy.surface", &surface),
            ],
            ..Default::default()
        });
        (platform, surface)
    }

    fn attributes(attrs: &mut Vec<KeyValue>) {
        let input = serde_json::Value::Object(
            attrs
                .iter()
                .take(64)
                .filter_map(|a| {
                    let value = match a.value.as_ref()?.value.as_ref()? {
                        any_value::Value::StringValue(s) => serde_json::Value::String(s.clone()),
                        any_value::Value::BoolValue(v) => (*v).into(),
                        any_value::Value::IntValue(v) => (*v).into(),
                        any_value::Value::DoubleValue(v) if v.is_finite() => serde_json::json!(v),
                        _ => return None,
                    };
                    Some((a.key.clone(), value))
                })
                .collect(),
        );
        *attrs = sanitize_attributes(&input)
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, value)| {
                let value = match value {
                    serde_json::Value::String(s) => any_value::Value::StringValue(s.clone()),
                    serde_json::Value::Bool(v) => any_value::Value::BoolValue(*v),
                    serde_json::Value::Number(v) if v.is_i64() => {
                        any_value::Value::IntValue(v.as_i64().unwrap())
                    }
                    _ => any_value::Value::DoubleValue(value.as_f64().unwrap_or(0.0)),
                };
                KeyValue {
                    key: key.clone(),
                    value: Some(AnyValue { value: Some(value) }),
                    ..Default::default()
                }
            })
            .collect();
    }

    fn ids(trace: &[u8], span: &[u8], required: bool) -> Result<()> {
        ensure!(
            (!required && trace.is_empty()) || (trace.len() == 16 && trace.iter().any(|b| *b != 0)),
            "invalid OTLP trace id"
        );
        ensure!(
            (!required && span.is_empty()) || (span.len() == 8 && span.iter().any(|b| *b != 0)),
            "invalid OTLP span id"
        );
        ensure!(
            span.is_empty() || !trace.is_empty(),
            "span id without trace id"
        );
        Ok(())
    }

    fn time(value: u64, now: u64) -> u64 {
        if value == 0 {
            now
        } else {
            value.clamp(
                now.saturating_sub(7 * 86_400_000_000_000),
                now.saturating_add(300_000_000_000),
            )
        }
    }

    fn finite_metric_attributes(attrs: &mut Vec<KeyValue>, platform: &str, surface: &str) {
        let mut result = vec![text("platform", platform), text("surface", surface)];
        for key in ["connection", "transport", "reason", "operation"] {
            if let Some(value) = value_text(attrs, key) {
                let allowed: &[&str] = match key {
                    "connection" => &["initial", "reconnect"],
                    "transport" => &["websocket", "http"],
                    "reason" => &[
                        "initial",
                        "close",
                        "error",
                        "online",
                        "visibility",
                        "timeout",
                        "heartbeat",
                        "other",
                    ],
                    _ => &[
                        "submit",
                        "prompt",
                        "open_session",
                        "cancel",
                        "set_config_option",
                        "sync",
                    ],
                };
                result.push(text(
                    key,
                    if allowed.contains(&value) {
                        value
                    } else {
                        "other"
                    },
                ));
            }
        }
        *attrs = result;
    }

    /// Closed instruments prevent unbounded state even for an authenticated client.
    fn instrument(metric: &metrics::Metric) -> Result<bool> {
        let counter = matches!(
            metric.name.as_str(),
            "cowboy.client.websocket.reconnects" | "cowboy.client.long_tasks"
        );
        let histogram = matches!(
            metric.name.as_str(),
            "cowboy.client.websocket.connect.duration"
                | "cowboy.client.websocket.reconnect.duration"
                | "cowboy.client.long_task.duration"
                | "cowboy.client.navigation.duration"
                | "cowboy.client.command.duration"
                | "cowboy.client.first_output.duration"
        );
        ensure!(
            (counter && metric.unit == "{event}") || (histogram && metric.unit == "s"),
            "unknown OTLP instrument or unit"
        );
        Ok(counter)
    }

    impl Request {
        pub(crate) fn sanitize(&mut self, principal: &str) -> Result<()> {
            let now = now_nanos();
            let owner = format!("{:x}", Sha256::digest(principal.as_bytes()));
            let mut groups = 0;
            match self {
                Self::Logs(r) => {
                    ensure!(r.resource_logs.len() <= 4, "too many OTLP resources");
                    for r in &mut r.resource_logs {
                        resource(&mut r.resource);
                        r.schema_url.clear();
                        groups += r.scope_logs.len();
                        ensure!(groups <= 8, "too many OTLP scopes");
                        for s in &mut r.scope_logs {
                            scope(&mut s.scope);
                            s.schema_url.clear();
                            for log in &mut s.log_records {
                                ids(&log.trace_id, &log.span_id, false)?;
                                log.time_unix_nano = time(log.time_unix_nano, now);
                                log.observed_time_unix_nano = now;
                                log.flags &= 1;
                                log.severity_number = log.severity_number.clamp(0, 24);
                                log.severity_text = match log.severity_number {
                                    1..=4 => "TRACE",
                                    5..=8 => "DEBUG",
                                    9..=12 => "INFO",
                                    13..=16 => "WARN",
                                    17..=20 => "ERROR",
                                    21..=24 => "FATAL",
                                    _ => "",
                                }
                                .into();
                                let body = log
                                    .body
                                    .as_ref()
                                    .and_then(|v| match &v.value {
                                        Some(any_value::Value::StringValue(s)) => Some(s.as_str()),
                                        _ => None,
                                    })
                                    .unwrap_or("[non-text body omitted]");
                                log.body = Some(AnyValue {
                                    value: Some(any_value::Value::StringValue(redact_message(
                                        body,
                                    ))),
                                });
                                log.event_name = if valid_name(&log.event_name) {
                                    log.event_name.clone()
                                } else {
                                    "client.log".into()
                                };
                                attributes(&mut log.attributes);
                                log.attributes.retain(|a| !reserved(&a.key));
                                log.attributes.push(text("cowboy.owner", &owner));
                            }
                        }
                    }
                }
                Self::Traces(r) => {
                    ensure!(r.resource_spans.len() <= 4, "too many OTLP resources");
                    for r in &mut r.resource_spans {
                        resource(&mut r.resource);
                        r.schema_url.clear();
                        groups += r.scope_spans.len();
                        ensure!(groups <= 8, "too many OTLP scopes");
                        for s in &mut r.scope_spans {
                            scope(&mut s.scope);
                            s.schema_url.clear();
                            for span in &mut s.spans {
                                ids(&span.trace_id, &span.span_id, true)?;
                                ensure!(
                                    span.parent_span_id.is_empty()
                                        || (span.parent_span_id.len() == 8
                                            && span.parent_span_id.iter().any(|b| *b != 0)),
                                    "invalid parent span id"
                                );
                                ensure!(
                                    matches!(
                                        span.name.as_str(),
                                        "cowboy.client.connect"
                                            | "cowboy.client.command"
                                            | "cowboy.client.first_output"
                                    ),
                                    "unknown client span"
                                );
                                ensure!(
                                    span.end_time_unix_nano >= span.start_time_unix_nano
                                        && span
                                            .end_time_unix_nano
                                            .saturating_sub(span.start_time_unix_nano)
                                            <= 1_800_000_000_000,
                                    "invalid span duration"
                                );
                                span.start_time_unix_nano = time(span.start_time_unix_nano, now);
                                span.end_time_unix_nano = time(span.end_time_unix_nano, now)
                                    .max(span.start_time_unix_nano);
                                span.trace_state.clear();
                                span.flags &= 1;
                                span.kind = span.kind.clamp(0, 5);
                                if let Some(status) = &mut span.status {
                                    status.code = status.code.clamp(0, 2);
                                    status.message = redact_message(&status.message);
                                }
                                attributes(&mut span.attributes);
                                span.attributes.retain(|a| !reserved(&a.key));
                                span.attributes.push(text("cowboy.owner", &owner));
                                span.attributes.push(text("cowboy.trust", "client"));
                                span.events.truncate(8);
                                for event in &mut span.events {
                                    event.name = if valid_name(&event.name) {
                                        event.name.clone()
                                    } else {
                                        "client.event".into()
                                    };
                                    event.time_unix_nano = time(event.time_unix_nano, now);
                                    attributes(&mut event.attributes);
                                    event.attributes.retain(|a| !reserved(&a.key));
                                    event.attributes.truncate(4);
                                }
                                // Do not forward arbitrary remote links or baggage from a browser.
                                span.links.clear();
                            }
                        }
                    }
                }
                Self::Metrics(r) => {
                    ensure!(r.resource_metrics.len() <= 4, "too many OTLP resources");
                    for r in &mut r.resource_metrics {
                        let (platform, surface) = resource(&mut r.resource);
                        r.schema_url.clear();
                        groups += r.scope_metrics.len();
                        ensure!(groups <= 8, "too many OTLP scopes");
                        for s in &mut r.scope_metrics {
                            scope(&mut s.scope);
                            s.schema_url.clear();
                            ensure!(s.metrics.len() <= 32, "too many OTLP instruments");
                            for m in &mut s.metrics {
                                let counter = instrument(m)?;
                                m.description.clear();
                                m.metadata.clear();
                                match &mut m.data {
                                    Some(metric::Data::Sum(v)) if counter => {
                                        ensure!(
                                            v.aggregation_temporality == 1 && v.is_monotonic,
                                            "client counters must be monotonic delta"
                                        );
                                        for p in &mut v.data_points {
                                            let n = number(p)?;
                                            ensure!(
                                                (0.0..=1e9).contains(&n),
                                                "invalid counter value"
                                            );
                                            p.value = Some(number_data_point::Value::AsDouble(n));
                                            ensure!(
                                                p.time_unix_nano >= p.start_time_unix_nano,
                                                "invalid metric interval"
                                            );
                                            p.time_unix_nano = time(p.time_unix_nano, now);
                                            p.start_time_unix_nano =
                                                time(p.start_time_unix_nano, now)
                                                    .min(p.time_unix_nano);
                                            p.exemplars.clear();
                                            p.flags = 0;
                                            finite_metric_attributes(
                                                &mut p.attributes,
                                                &platform,
                                                &surface,
                                            );
                                        }
                                    }
                                    Some(metric::Data::Histogram(v)) if !counter => {
                                        ensure!(
                                            v.aggregation_temporality == 1,
                                            "client histograms must be delta"
                                        );
                                        for p in &mut v.data_points {
                                            ensure!(
                                                p.explicit_bounds
                                                    == [
                                                        0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5,
                                                        1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0
                                                    ],
                                                "unexpected histogram boundaries"
                                            );
                                            ensure!(
                                                p.explicit_bounds.len() <= 32
                                                    && p.bucket_counts.len()
                                                        == p.explicit_bounds.len() + 1
                                                    && p.explicit_bounds
                                                        .iter()
                                                        .all(|v| v.is_finite() && *v >= 0.0)
                                                    && p.explicit_bounds
                                                        .windows(2)
                                                        .all(|w| w[0] < w[1]),
                                                "invalid histogram buckets"
                                            );
                                            ensure!(
                                                p.count <= 1_000_000_000
                                                    && p.bucket_counts
                                                        .iter()
                                                        .try_fold(0u64, |n, v| n.checked_add(*v))
                                                        == Some(p.count),
                                                "invalid histogram count"
                                            );
                                            ensure!(
                                                p.sum
                                                    .is_some_and(|n| n.is_finite()
                                                        && (0.0..=1e12).contains(&n)),
                                                "invalid histogram sum"
                                            );
                                            ensure!(
                                                [p.min, p.max]
                                                    .into_iter()
                                                    .flatten()
                                                    .all(|n| n.is_finite()
                                                        && (0.0..=1800.0).contains(&n)),
                                                "invalid histogram extrema"
                                            );
                                            ensure!(
                                                !matches!((p.min, p.max), (Some(min), Some(max)) if min > max),
                                                "reversed histogram extrema"
                                            );
                                            ensure!(
                                                p.time_unix_nano >= p.start_time_unix_nano,
                                                "invalid metric interval"
                                            );
                                            p.time_unix_nano = time(p.time_unix_nano, now);
                                            p.start_time_unix_nano =
                                                time(p.start_time_unix_nano, now)
                                                    .min(p.time_unix_nano);
                                            p.exemplars.clear();
                                            p.flags = 0;
                                            finite_metric_attributes(
                                                &mut p.attributes,
                                                &platform,
                                                &surface,
                                            );
                                        }
                                    }
                                    _ => anyhow::bail!("unsupported client metric aggregation"),
                                }
                            }
                        }
                    }
                }
            }
            Ok(())
        }

        pub(crate) fn records(&self) -> String {
            let mut output = String::new();
            match self {
                Self::Logs(r) => {
                    for r in &r.resource_logs {
                        for s in &r.scope_logs {
                            for v in &s.log_records {
                                let _ = writeln!(
                                    output,
                                    "{}",
                                    serde_json::json!({"signal":"logs","resource":r.resource,"scope":s.scope,"record":v})
                                );
                            }
                        }
                    }
                }
                Self::Traces(r) => {
                    for r in &r.resource_spans {
                        for s in &r.scope_spans {
                            for v in &s.spans {
                                let _ = writeln!(
                                    output,
                                    "{}",
                                    serde_json::json!({"signal":"traces","resource":r.resource,"scope":s.scope,"span":v})
                                );
                            }
                        }
                    }
                }
                Self::Metrics(r) => {
                    for r in &r.resource_metrics {
                        for s in &r.scope_metrics {
                            for m in &s.metrics {
                                let points: Vec<serde_json::Value> = match &m.data {
                        Some(metric::Data::Sum(v)) => v.data_points.iter().map(|p| serde_json::json!({"sum":p,"aggregation_temporality":v.aggregation_temporality,"is_monotonic":v.is_monotonic})).collect(),
                        Some(metric::Data::Histogram(v)) => v.data_points.iter().map(|p| serde_json::json!({"histogram":p,"aggregation_temporality":v.aggregation_temporality})).collect(),
                        _ => Vec::new(),
                    };
                                for point in points {
                                    let _ = writeln!(
                                        output,
                                        "{}",
                                        serde_json::json!({"signal":"metrics","resource":r.resource,"scope":s.scope,"name":m.name,"unit":m.unit,"point":point})
                                    );
                                }
                            }
                        }
                    }
                }
            }
            output
        }
    }

    fn valid_name(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    }

    fn reserved(key: &str) -> bool {
        key.starts_with("cowboy.")
            || matches!(
                key,
                "service.name" | "session_id" | "machine_id" | "user_id" | "client_id"
            )
    }

    fn number(p: &metrics::NumberDataPoint) -> Result<f64> {
        match p.value {
            Some(number_data_point::Value::AsDouble(n)) if n.is_finite() => Ok(n),
            #[allow(clippy::cast_precision_loss)]
            Some(number_data_point::Value::AsInt(n)) => Ok(n as f64),
            _ => anyhow::bail!("invalid metric value"),
        }
    }

    /// One bounded Controller-owned writer, independent of browser resource IDs.
    #[derive(Default, Clone)]
    pub(crate) struct Aggregator {
        start: u64,
        streams: BTreeMap<String, metrics::Metric>,
    }

    impl Aggregator {
        pub(crate) fn aggregate(&mut self, request: &mut Request) -> Result<()> {
            let Request::Metrics(r) = request else {
                return Ok(());
            };
            // Commit all points together; a rejected request cannot half-increment counters.
            let mut candidate = self.clone();
            if candidate.start == 0 {
                candidate.start = now_nanos();
            }
            let now = now_nanos();
            let mut changed = BTreeSet::new();
            for m in r
                .resource_metrics
                .iter()
                .flat_map(|r| &r.scope_metrics)
                .flat_map(|s| &s.metrics)
            {
                let mut add = |attrs: &[KeyValue], data: metric::Data| -> Result<()> {
                    let key = format!("{}:{}", m.name, serde_json::to_string(attrs)?);
                    ensure!(
                        candidate.streams.contains_key(&key) || candidate.streams.len() < 512,
                        "metric stream limit"
                    );
                    let metric =
                        candidate
                            .streams
                            .entry(key.clone())
                            .or_insert_with(|| metrics::Metric {
                                name: m.name.clone(),
                                unit: m.unit.clone(),
                                ..Default::default()
                            });
                    match (&mut metric.data, data) {
                        (None, metric::Data::Sum(mut v)) => {
                            v.aggregation_temporality = 2;
                            v.data_points[0].start_time_unix_nano = candidate.start;
                            v.data_points[0].time_unix_nano = now;
                            metric.data = Some(metric::Data::Sum(v));
                        }
                        (None, metric::Data::Histogram(mut v)) => {
                            v.aggregation_temporality = 2;
                            v.data_points[0].start_time_unix_nano = candidate.start;
                            v.data_points[0].time_unix_nano = now;
                            metric.data = Some(metric::Data::Histogram(v));
                        }
                        (Some(metric::Data::Sum(old)), metric::Data::Sum(new)) => {
                            let n = number(&old.data_points[0])? + number(&new.data_points[0])?;
                            ensure!(n.is_finite(), "counter overflow");
                            old.data_points[0].value = Some(number_data_point::Value::AsDouble(n));
                            old.data_points[0].time_unix_nano = now;
                        }
                        (Some(metric::Data::Histogram(old)), metric::Data::Histogram(new)) => {
                            let a = &mut old.data_points[0];
                            let b = &new.data_points[0];
                            ensure!(
                                a.explicit_bounds == b.explicit_bounds,
                                "histogram boundary change"
                            );
                            a.count = a
                                .count
                                .checked_add(b.count)
                                .ok_or_else(|| anyhow::anyhow!("histogram count overflow"))?;
                            a.sum = Some(a.sum.unwrap_or(0.0) + b.sum.unwrap_or(0.0));
                            ensure!(a.sum.is_some_and(f64::is_finite), "histogram sum overflow");
                            for (a, b) in a.bucket_counts.iter_mut().zip(&b.bucket_counts) {
                                *a = a
                                    .checked_add(*b)
                                    .ok_or_else(|| anyhow::anyhow!("histogram bucket overflow"))?;
                            }
                            a.min = match (a.min, b.min) {
                                (Some(a), Some(b)) => Some(a.min(b)),
                                _ => None,
                            };
                            a.max = match (a.max, b.max) {
                                (Some(a), Some(b)) => Some(a.max(b)),
                                _ => None,
                            };
                            a.time_unix_nano = now;
                        }
                        _ => anyhow::bail!("metric type changed"),
                    }
                    changed.insert(key);
                    Ok(())
                };
                match &m.data {
                    Some(metric::Data::Sum(v)) => {
                        for p in &v.data_points {
                            add(
                                &p.attributes,
                                metric::Data::Sum(metrics::Sum {
                                    data_points: vec![p.clone()],
                                    ..v.clone()
                                }),
                            )?;
                        }
                    }
                    Some(metric::Data::Histogram(v)) => {
                        for p in &v.data_points {
                            add(
                                &p.attributes,
                                metric::Data::Histogram(metrics::Histogram {
                                    data_points: vec![p.clone()],
                                    ..v.clone()
                                }),
                            )?;
                        }
                    }
                    _ => anyhow::bail!("unsupported metric aggregation"),
                }
            }
            *r = ExportMetricsServiceRequest {
                resource_metrics: vec![metrics::ResourceMetrics {
                    resource: Some(Resource {
                        attributes: vec![text("service.name", "cowboy-client-aggregate")],
                        ..Default::default()
                    }),
                    scope_metrics: vec![metrics::ScopeMetrics {
                        scope: Some(InstrumentationScope {
                            name: "cowboy.client.aggregate".into(),
                            version: "1".into(),
                            ..Default::default()
                        }),
                        metrics: changed
                            .iter()
                            .filter_map(|k| candidate.streams.get(k).cloned())
                            .collect(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
            };
            *self = candidate;
            Ok(())
        }
    }
}

#[cfg(feature = "full")]
pub(crate) use controller::{Aggregator, command_span, now_nanos};

#[cfg(all(test, feature = "full"))]
pub(crate) fn client_fixtures() -> Vec<(Signal, Vec<u8>)> {
    let exports: Vec<Export> =
        serde_json::from_str(include_str!("../tests/fixtures/otel-client.json")).unwrap();
    exports
        .into_iter()
        .map(|v| {
            (
                v.signal,
                base64::engine::general_purpose::STANDARD
                    .decode(v.protobuf)
                    .unwrap(),
            )
        })
        .collect()
}

#[cfg(all(test, feature = "full"))]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value};
    use opentelemetry_proto::tonic::metrics::v1::number_data_point;

    #[test]
    fn sanitization_preserves_scalar_types_and_full_integer_precision() {
        let (_, bytes) = client_fixtures()
            .into_iter()
            .find(|(signal, _)| *signal == Signal::Logs)
            .unwrap();
        let mut request = Request::decode(Signal::Logs, &bytes).unwrap();
        let expected = vec![
            ("integer_max", any_value::Value::IntValue(i64::MAX)),
            ("integer_min", any_value::Value::IntValue(i64::MIN)),
            ("double", any_value::Value::DoubleValue(1.5)),
            ("boolean", any_value::Value::BoolValue(true)),
        ];
        let Request::Logs(ref mut logs) = request else {
            panic!("logs")
        };
        logs.resource_logs[0].scope_logs[0].log_records[0].attributes = expected
            .iter()
            .map(|(key, value)| KeyValue {
                key: (*key).into(),
                value: Some(AnyValue {
                    value: Some(value.clone()),
                }),
                ..Default::default()
            })
            .collect();
        request.sanitize("principal").unwrap();
        let Request::Logs(logs) = request else {
            panic!("logs")
        };
        let attrs = &logs.resource_logs[0].scope_logs[0].log_records[0].attributes;
        for (key, value) in expected {
            assert_eq!(
                attrs.iter().find(|attr| attr.key == key).unwrap().value,
                Some(AnyValue { value: Some(value) })
            );
        }
    }

    #[test]
    fn official_javascript_sdk_messages_decode_sanitize_and_aggregate() {
        let mut aggregator = Aggregator::default();
        let mut log_trace = Vec::new();
        let mut span_trace = Vec::new();
        for (signal, bytes) in client_fixtures() {
            let mut request = Request::decode(signal, &bytes).unwrap();
            assert_eq!(request.count(), 1);
            request.sanitize("test-user").unwrap();
            match &request {
                Request::Logs(r) => {
                    log_trace = r.resource_logs[0].scope_logs[0].log_records[0]
                        .trace_id
                        .clone()
                }
                Request::Traces(r) => {
                    span_trace = r.resource_spans[0].scope_spans[0].spans[0].trace_id.clone()
                }
                Request::Metrics(_) => aggregator.aggregate(&mut request).unwrap(),
            }
            let records = request.records();
            assert!(!records.contains("fixture-secret"));
            assert!(!records.contains("must-not-be-a-label"));
            assert!(!records.contains("test-user"));
            if signal == Signal::Metrics {
                assert!(records.contains("\"aggregation_temporality\":2"));
            }
        }
        assert_eq!(log_trace, span_trace);
        assert_eq!(log_trace.len(), 16);
    }

    #[test]
    fn delta_clients_share_one_cumulative_counter_and_invalid_batches_are_atomic() {
        let (signal, bytes) = client_fixtures().pop().unwrap();
        let mut aggregator = Aggregator::default();
        for (index, principal) in [
            "fixture-user-alpha",
            "fixture-user-beta",
            "fixture-user-alpha",
        ]
        .into_iter()
        .enumerate()
        {
            let mut request = Request::decode(signal, &bytes).unwrap();
            request.sanitize(principal).unwrap();
            aggregator.aggregate(&mut request).unwrap();
            let Request::Metrics(ref r) = request else {
                panic!("metrics")
            };
            let Some(metric::Data::Sum(sum)) =
                &r.resource_metrics[0].scope_metrics[0].metrics[0].data
            else {
                panic!("sum")
            };
            assert_eq!(sum.aggregation_temporality, 2);
            assert_eq!(
                sum.data_points[0].value,
                Some(number_data_point::Value::AsDouble(f64::from(
                    u32::try_from(index + 1).unwrap()
                )))
            );
            assert!(!request.records().contains(principal));
        }
        let mut bad = Request::decode(signal, &bytes).unwrap();
        let Request::Metrics(r) = &mut bad else {
            panic!("metrics")
        };
        let Some(metric::Data::Sum(sum)) =
            &mut r.resource_metrics[0].scope_metrics[0].metrics[0].data
        else {
            panic!("sum")
        };
        sum.aggregation_temporality = 2;
        assert!(bad.sanitize("client-a").is_err());
        sum_nan_rejected(signal, &bytes);
    }

    fn sum_nan_rejected(signal: Signal, bytes: &[u8]) {
        let mut bad = Request::decode(signal, bytes).unwrap();
        let Request::Metrics(r) = &mut bad else {
            panic!("metrics")
        };
        let Some(metric::Data::Sum(sum)) =
            &mut r.resource_metrics[0].scope_metrics[0].metrics[0].data
        else {
            panic!("sum")
        };
        sum.data_points[0].value = Some(number_data_point::Value::AsDouble(f64::NAN));
        assert!(bad.sanitize("client-a").is_err());
    }

    #[test]
    fn w3c_parent_creates_a_controller_child_without_trusting_browser_resources() {
        let parent = "00-11111111111111111111111111111111-2222222222222222-01";
        let Request::Traces(r) = command_span("principal", parent, now_nanos()).unwrap() else {
            panic!("trace")
        };
        let span = &r.resource_spans[0].scope_spans[0].spans[0];
        assert_eq!(span.trace_id, vec![0x11; 16]);
        assert_eq!(span.parent_span_id, vec![0x22; 8]);
        assert_ne!(span.span_id, span.parent_span_id);
        assert_eq!(span.name, "cowboy.controller.dispatch");
        for invalid in [
            "",
            "00-11111111111111111111111111111111-2222222222222222-00",
            "00-00000000000000000000000000000000-2222222222222222-01",
            "ff-11111111111111111111111111111111-2222222222222222-01",
            "00-11111111111111111111111111111111-2222222222222222-zz",
        ] {
            assert!(command_span("principal", invalid, now_nanos()).is_none());
        }
        assert!(Request::decode(Signal::Logs, &[255]).is_err());
        assert!(Request::decode(Signal::Logs, &vec![0; MAX_BYTES + 1]).is_err());
    }
}
