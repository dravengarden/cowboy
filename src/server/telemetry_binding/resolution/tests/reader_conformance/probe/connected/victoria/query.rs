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

fn fixture_labels(
    attributes: &[opentelemetry_proto::tonic::common::v1::KeyValue],
    dimension: (&str, &str),
) -> Result<BTreeMap<String, String>, Failure> {
    // Independent fixture contract: do not accept arbitrary labels or values
    // merely because the Controller forwarded them to the database.
    let mut expected: BTreeMap<_, _> = [("platform", "ios"), ("surface", "mobile"), dimension]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    check(attributes.len() == expected.len() && labels(attributes)? == expected)?;
    // VictoriaMetrics v1.148.0 promotes native OTel scope metadata with these
    // literal names (lib/protoparser/opentelemetry/pb/pb.go).
    expected.extend(
        [
            ("service.name", "cowboy-client-aggregate"),
            ("scope.name", "cowboy.client.aggregate"),
            ("scope.version", "1"),
        ]
        .map(|(key, value)| (key.to_owned(), value.to_owned())),
    );
    Ok(expected)
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
                                check(
                                    metric.name == "cowboy.client.websocket.reconnects"
                                        && metric.unit == "{event}"
                                        && sum.aggregation_temporality == 2
                                        && sum.is_monotonic
                                        && sum.data_points.len() == 1,
                                )?;
                                for point in sum.data_points {
                                    let value = match point.value {
                                        Some(number_data_point::Value::AsDouble(value)) => value,
                                        _ => return Err(Failure::Setup),
                                    };
                                    check(value == 1.0)?;
                                    samples.push(Sample {
                                        name: metric.name.clone(),
                                        labels: fixture_labels(
                                            &point.attributes,
                                            ("reason", "online"),
                                        )?,
                                        value,
                                        timestamp: point.time_unix_nano / 1_000_000,
                                    });
                                }
                            }
                            Some(Data::Histogram(histogram)) => {
                                check(
                                    metric.name == "cowboy.client.websocket.connect.duration"
                                        && metric.unit == "s"
                                        && histogram.aggregation_temporality == 2
                                        && histogram.data_points.len() == 1,
                                )?;
                                for point in histogram.data_points {
                                    check(
                                        point.count == 1
                                            && point.sum == Some(0.125)
                                            && point.explicit_bounds
                                                == [
                                                    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0,
                                                    2.5, 5.0, 10.0, 30.0, 60.0, 120.0,
                                                ]
                                            && point.bucket_counts
                                                == [0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                                    )?;
                                    let labels = fixture_labels(
                                        &point.attributes,
                                        ("connection", "initial"),
                                    )?;
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
        check(logs.len() == 1 && spans.len() == 1 && samples.len() == 18)?;
        let log = logs.remove(0);
        let span = spans.remove(0);
        check(
            log.trace_id == span.trace_id
                && log.span_id == span.span_id
                && span.trace_id.len() == 16
                && span.span_id.len() == 8
                && span.end_time_unix_nano >= span.start_time_unix_nano,
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
                        && metric.len() == sample.labels.len() + 1
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
        // VictoriaTraces 0.9.3's default insert.indexFlushInterval is 20s.
        // Retain that real profile: an accepted span need not yet be visible
        // through the Jaeger trace-ID index. Never turn a timeout into success.
        while started.elapsed() < Duration::from_secs(35) {
            match self.once(databases, restarted).await {
                Ok(result) => return Ok(result),
                Err(failure) => last = failure,
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Err(last)
    }
}

#[test]
fn database_queries_refuse_success_status_without_exact_signal_values() {
    let requests = forwarded_fixture();
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
    for key in ["service.name", "scope.name", "scope.version", "platform"] {
        let mut changed = rows.clone();
        changed[0]["metric"].as_object_mut().unwrap().remove(key);
        assert!(!expected.metrics_match(&changed));
        changed[0]["metric"][key] = "wrong".into();
        assert!(!expected.metrics_match(&changed));
    }
    let mut changed = rows.clone();
    changed.push(rows[0].clone());
    assert!(!expected.metrics_match(&changed));
}

fn forwarded_fixture() -> Vec<(Signal, Vec<u8>)> {
    let mut aggregate = crate::otlp::Aggregator::default();
    fresh_fixtures()
        .unwrap()
        .into_iter()
        .map(|(signal, bytes)| {
            let mut request = Request::decode(signal, &bytes).unwrap();
            request.sanitize("isolated-query-test").unwrap();
            aggregate.aggregate(&mut request).unwrap();
            (signal, request.export().decode().unwrap().0)
        })
        .collect()
}

#[test]
fn query_oracle_rejects_changed_or_duplicate_fixture_metrics() {
    use opentelemetry_proto::tonic::metrics::v1::{metric::Data, number_data_point::Value};
    let original = forwarded_fixture();
    assert!(Expected::decode(&original).is_ok());
    for histogram in [false, true] {
        let mut changed = original.clone();
        let mut altered = false;
        for (signal, bytes) in &mut changed {
            let mut request = Request::decode(*signal, bytes).unwrap();
            if let Request::Metrics(metrics) = &mut request {
                let data = &mut metrics.resource_metrics[0].scope_metrics[0].metrics[0].data;
                match data {
                    Some(Data::Sum(sum)) if !histogram => {
                        sum.data_points[0].value = Some(Value::AsDouble(2.0));
                        altered = true;
                    }
                    Some(Data::Histogram(histogram_data)) if histogram => {
                        histogram_data.data_points[0].count = 2;
                        altered = true;
                    }
                    _ => {}
                }
            }
            *bytes = request.export().decode().unwrap().0;
        }
        assert!(altered && Expected::decode(&changed).is_err());
    }
    let mut duplicate = original.clone();
    duplicate.push(original.last().unwrap().clone());
    assert!(Expected::decode(&duplicate).is_err());
}

#[test]
fn log_and_trace_queries_require_exact_correlated_records() {
    use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValue;
    let expected = Expected::decode(&forwarded_fixture()).unwrap();
    let Some(AnyValue::StringValue(body)) = expected
        .log
        .body
        .as_ref()
        .and_then(|body| body.value.as_ref())
    else {
        panic!("fixture text log");
    };
    let timestamp =
        chrono::DateTime::from_timestamp_nanos(i64::try_from(expected.log.time_unix_nano).unwrap())
            .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
    let log = json!({
        "_msg":body,"_time":timestamp,
        "trace_id":hex(&expected.log.trace_id),"span_id":hex(&expected.log.span_id),
    });
    assert!(expected.logs_match(std::slice::from_ref(&log)));
    for key in ["_msg", "_time", "trace_id", "span_id"] {
        let mut changed = log.clone();
        changed[key] = "wrong".into();
        assert!(!expected.logs_match(&[changed]));
    }
    assert!(!expected.logs_match(&[log.clone(), log]));
    let trace_id = hex(&expected.span.trace_id);
    let trace = json!({"errors":null,"data":[{"traceID":trace_id,"spans":[{
        "traceID":trace_id,"spanID":hex(&expected.span.span_id),
        "operationName":expected.span.name,
        "startTime":expected.span.start_time_unix_nano / 1000,
        "duration":(expected.span.end_time_unix_nano - expected.span.start_time_unix_nano) / 1000,
    }]}]});
    assert!(expected.trace_matches(&trace));
    for key in [
        "traceID",
        "spanID",
        "operationName",
        "startTime",
        "duration",
    ] {
        let mut changed = trace.clone();
        changed["data"][0]["spans"][0][key] = Value::Null;
        assert!(!expected.trace_matches(&changed));
    }
    let mut changed = trace.clone();
    changed["errors"] = json!([{"code":500}]);
    assert!(!expected.trace_matches(&changed));
    let mut changed = trace.clone();
    changed["data"][0]["spans"]
        .as_array_mut()
        .unwrap()
        .push(trace["data"][0]["spans"][0].clone());
    assert!(!expected.trace_matches(&changed));
}
