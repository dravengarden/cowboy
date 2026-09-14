//! Query the actual database; HTTP admission is not delivery acceptance.
use super::super::super::super::victoria::QueryRound;
use super::*;
use opentelemetry_proto::tonic::{logs::v1::LogRecord, trace::v1::Span};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
struct Sample {
    name: String,
    labels: BTreeMap<String, String>,
    value: f64,
    timestamp: u64,
}

pub(super) struct Expected {
    log: LogRecord,
    span: Span,
    samples: Vec<Sample>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn labels(
    attributes: &[opentelemetry_proto::tonic::common::v1::KeyValue],
) -> Result<BTreeMap<String, String>, Failure> {
    use opentelemetry_proto::tonic::common::v1::any_value::Value;
    attributes
        .iter()
        .map(|attribute| {
            let value = match attribute
                .value
                .as_ref()
                .and_then(|value| value.value.as_ref())
            {
                Some(Value::StringValue(value)) => value.clone(),
                _ => return Err(Failure::Setup),
            };
            Ok((attribute.key.clone(), value))
        })
        .collect()
}

impl Expected {
    pub fn decode(requests: &[(Signal, Vec<u8>)]) -> Result<Self, Failure> {
        let mut logs = Vec::new();
        let mut spans = Vec::new();
        let mut samples = Vec::new();
        for (signal, bytes) in requests {
            match Request::decode(*signal, bytes).map_err(|_| Failure::FrameDecode)? {
                Request::Logs(request) => logs.extend(
                    request
                        .resource_logs
                        .into_iter()
                        .flat_map(|r| r.scope_logs)
                        .flat_map(|s| s.log_records),
                ),
                Request::Traces(request) => spans.extend(
                    request
                        .resource_spans
                        .into_iter()
                        .flat_map(|r| r.scope_spans)
                        .flat_map(|s| s.spans),
                ),
                Request::Metrics(request) => {
                    for metric in request
                        .resource_metrics
                        .into_iter()
                        .flat_map(|r| r.scope_metrics)
                        .flat_map(|s| s.metrics)
                    {
                        use opentelemetry_proto::tonic::metrics::v1::{
                            metric::Data, number_data_point,
                        };
                        match metric.data {
                            Some(Data::Sum(sum)) => {
                                check(sum.aggregation_temporality == 2 && sum.is_monotonic)?;
                                for point in sum.data_points {
                                    let value = match point.value {
                                        Some(number_data_point::Value::AsDouble(value)) => value,
                                        _ => return Err(Failure::Setup),
                                    };
                                    samples.push(Sample {
                                        name: metric.name.clone(),
                                        labels: labels(&point.attributes)?,
                                        value,
                                        timestamp: point.time_unix_nano / 1_000_000,
                                    });
                                }
                            }
                            Some(Data::Histogram(histogram)) => {
                                check(histogram.aggregation_temporality == 2)?;
                                for point in histogram.data_points {
                                    check(
                                        point.bucket_counts.len()
                                            == point.explicit_bounds.len() + 1,
                                    )?;
                                    let labels = labels(&point.attributes)?;
                                    let timestamp = point.time_unix_nano / 1_000_000;
                                    let count = f64::from(
                                        u32::try_from(point.count).map_err(|_| Failure::Setup)?,
                                    );
                                    for (suffix, value) in [
                                        ("count", count),
                                        ("sum", point.sum.ok_or(Failure::Setup)?),
                                    ] {
                                        samples.push(Sample {
                                            name: format!("{}_{suffix}", metric.name),
                                            labels: labels.clone(),
                                            value,
                                            timestamp,
                                        });
                                    }
                                    let mut count = 0_u64;
                                    for (index, bucket) in point.bucket_counts.iter().enumerate() {
                                        count = count.checked_add(*bucket).ok_or(Failure::Setup)?;
                                        let mut labels = labels.clone();
                                        labels.insert(
                                            "le".into(),
                                            point
                                                .explicit_bounds
                                                .get(index)
                                                .map_or_else(|| "+Inf".into(), ToString::to_string),
                                        );
                                        samples.push(Sample {
                                            name: format!("{}_bucket", metric.name),
                                            labels,
                                            value: f64::from(
                                                u32::try_from(count).map_err(|_| Failure::Setup)?,
                                            ),
                                            timestamp,
                                        });
                                    }
                                    check(count == point.count)?;
                                }
                            }
                            _ => return Err(Failure::Setup),
                        }
                    }
                }
            }
        }
        check(logs.len() == 1 && spans.len() == 1 && samples.len() > 3)?;
        let log = logs.remove(0);
        let span = spans.remove(0);
        check(
            log.trace_id == span.trace_id
                && log.span_id == span.span_id
                && span.trace_id.len() == 16
                && span.span_id.len() == 8,
        )?;
        Ok(Self { log, span, samples })
    }

    fn logs_match(&self, rows: &[Value]) -> bool {
        use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValue;
        let Some(AnyValue::StringValue(body)) =
            self.log.body.as_ref().and_then(|body| body.value.as_ref())
        else {
            return false;
        };
        rows.len() == 1
            && rows[0]["_msg"] == *body
            && rows[0]["trace_id"] == hex(&self.log.trace_id)
            && rows[0]["span_id"] == hex(&self.log.span_id)
            && rows[0]["_time"]
                .as_str()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .and_then(|time| time.timestamp_nanos_opt())
                .and_then(|time| u64::try_from(time).ok())
                == Some(self.log.time_unix_nano)
    }

    fn metrics_match(&self, rows: &[Value]) -> bool {
        if rows.len() != self.samples.len() {
            return false;
        }
        let mut used = BTreeSet::new();
        for sample in &self.samples {
            let matches: Vec<_> = rows
                .iter()
                .enumerate()
                .filter(|(_, row)| {
                    let Some(metric) = row["metric"].as_object() else {
                        return false;
                    };
                    metric
                        .get("__name__")
                        .is_some_and(|name| name == &sample.name)
                        && sample.labels.iter().all(|(key, value)| {
                            metric.get(key).is_some_and(|actual| actual == value)
                        })
                        && metric.keys().all(|key| {
                            sample.labels.contains_key(key)
                                || [
                                    "__name__",
                                    "service.name",
                                    "cowboy.platform",
                                    "cowboy.surface",
                                    "otel_scope_name",
                                    "otel_scope_version",
                                ]
                                .contains(&key.as_str())
                        })
                        && row["values"].as_array().is_some_and(|values| {
                            values.len() == 1 && values[0].as_f64() == Some(sample.value)
                        })
                        && row["timestamps"].as_array().is_some_and(|values| {
                            values.len() == 1 && values[0].as_u64() == Some(sample.timestamp)
                        })
                })
                .map(|(index, _)| index)
                .collect();
            if matches.len() != 1 || !used.insert(matches[0]) {
                return false;
            }
        }
        used.len() == rows.len()
    }

    fn trace_matches(&self, response: &Value) -> bool {
        let Some(data) = response["data"].as_array().filter(|data| data.len() == 1) else {
            return false;
        };
        let Some(spans) = data[0]["spans"].as_array().filter(|spans| spans.len() == 1) else {
            return false;
        };
        let span = &spans[0];
        response["errors"].is_null()
            && data[0]["traceID"] == hex(&self.span.trace_id)
            && span["traceID"] == hex(&self.span.trace_id)
            && span["spanID"] == hex(&self.span.span_id)
            && span["operationName"] == self.span.name
            && span["startTime"].as_u64() == Some(self.span.start_time_unix_nano / 1000)
            && span["duration"].as_u64()
                == Some((self.span.end_time_unix_nano - self.span.start_time_unix_nano) / 1000)
    }

    async fn once(&self, databases: &Databases, restarted: bool) -> Result<QueryRound, Failure> {
        let logs = databases.rows(0).await?;
        if !self.logs_match(&logs) {
            return Err(Failure::DatabaseLogQuery);
        }
        let metrics = databases.rows(1).await?;
        if !self.metrics_match(&metrics) {
            return Err(Failure::DatabaseMetricQuery);
        }
        let bytes = databases
            .get(
                2,
                &format!("/select/jaeger/api/traces/{}", hex(&self.span.trace_id)),
                &[],
            )
            .await?;
        let trace: Value = serde_json::from_slice(&bytes).map_err(|_| Failure::FrameDecode)?;
        if !self.trace_matches(&trace) {
            return Err(Failure::DatabaseTraceQuery);
        }
        // Query ordering and flush partitions are not identities. Only the
        // verified expected semantics enter this normalized correlation digest.
        let digest = sha256(&serde_json::to_vec(&json!({
            "log":hex(&self.log.trace_id),"span":hex(&self.span.span_id),"time":self.log.time_unix_nano,
            "metrics":self.samples.iter().map(|s| json!({"name":s.name,"labels":s.labels,"value":s.value,"timestamp":s.timestamp})).collect::<Vec<_>>()
        })).map_err(|_| Failure::Setup)?);
        Ok(QueryRound {
            after_database_restart: restarted,
            log_records: logs.len(),
            trace_spans: 1,
            metric_series: metrics.len(),
            normalized_result_sha256: digest,
        })
    }

    pub async fn verify(
        &self,
        databases: &Databases,
        restarted: bool,
    ) -> Result<QueryRound, Failure> {
        let started = std::time::Instant::now();
        let mut last = Failure::Timeout;
        while started.elapsed() < Duration::from_secs(12) {
            match self.once(databases, restarted).await {
                Ok(result) => return Ok(result),
                Err(failure) => last = failure,
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(last)
    }
}

#[test]
fn database_queries_refuse_success_status_without_exact_signal_values() {
    let mut requests = fresh_fixtures().unwrap();
    // The Controller is responsible for cumulative conversion. Model that
    // declared wire form here solely to exercise query-oracle rejection.
    for (signal, bytes) in &mut requests {
        let mut request = Request::decode(*signal, bytes).unwrap();
        if let Request::Metrics(metrics) = &mut request {
            for metric in metrics
                .resource_metrics
                .iter_mut()
                .flat_map(|r| &mut r.scope_metrics)
                .flat_map(|s| &mut s.metrics)
            {
                use opentelemetry_proto::tonic::metrics::v1::metric::Data;
                match &mut metric.data {
                    Some(Data::Sum(sum)) => sum.aggregation_temporality = 2,
                    Some(Data::Histogram(histogram)) => histogram.aggregation_temporality = 2,
                    _ => panic!("fixture metric kind"),
                }
            }
        }
        *bytes = request.export().decode().unwrap().0;
    }
    let expected = Expected::decode(&requests).unwrap();
    assert!(!expected.logs_match(&[]));
    assert!(!expected.trace_matches(&json!({"data":[],"errors":null})));
    let rows: Vec<_> = expected
        .samples
        .iter()
        .map(|sample| {
            let mut labels = sample.labels.clone();
            labels.insert("__name__".into(), sample.name.clone());
            json!({"metric":labels,"values":[sample.value],"timestamps":[sample.timestamp]})
        })
        .collect();
    assert!(expected.metrics_match(&rows));
    for field in ["values", "timestamps", "metric"] {
        let mut changed = rows.clone();
        changed[0][field] = Value::Null;
        assert!(!expected.metrics_match(&changed));
    }
    let mut changed = rows.clone();
    changed[0]["values"] = json!([99]);
    assert!(!expected.metrics_match(&changed));
    let mut changed = rows.clone();
    changed[0]["metric"]["session_id"] = "forbidden".into();
    assert!(!expected.metrics_match(&changed));
    let mut changed = rows.clone();
    changed.push(rows[0].clone());
    assert!(!expected.metrics_match(&changed));
}
