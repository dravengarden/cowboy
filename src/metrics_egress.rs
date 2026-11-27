//! Signed OTLP metrics egress. A backend can promote resource and point
//! attributes to series labels, so both use finite values. Native and client
//! telemetry can keep richer attributes in local evidence.
use anyhow::Result;
use cowboy_wire::otlp::{Request, Signal};
use opentelemetry_proto::tonic::{
    common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value},
    metrics::v1::{HistogramDataPoint, Metric, metric},
    resource::v1::Resource,
};
use prost::Message as _;

pub(crate) mod legacy;

const CLIENT_HISTOGRAM_BOUNDS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0,
];

pub(crate) fn valid_client_histogram(point: &HistogramDataPoint) -> bool {
    point.explicit_bounds == CLIENT_HISTOGRAM_BOUNDS
        && point.bucket_counts.len() == CLIENT_HISTOGRAM_BOUNDS.len() + 1
        && point
            .bucket_counts
            .iter()
            .try_fold(0u64, |count, bucket| count.checked_add(*bucket))
            == Some(point.count)
}

pub(crate) const REQUESTS: &str = "cowboy.wait_cache.requests";
pub(crate) const WAITS: &str = "cowboy.wait_cache.waits";
pub(crate) const DECISIONS: &str = "cowboy.wait_cache.decisions";
pub(crate) const TOKENS: &str = "cowboy.wait_cache.tokens";
pub(crate) const LOSSES: &str = "cowboy.wait_cache.losses";

#[derive(Clone, Copy)]
enum Family {
    Client,
    WaitCache,
}

fn instrument(metric: &mut Metric) -> Option<Family> {
    let (family, sum, unit) = match metric.name.as_str() {
        "cowboy.client.websocket.reconnects" | "cowboy.client.long_tasks" => {
            (Family::Client, true, "{event}")
        }
        "cowboy.client.websocket.connect.duration"
        | "cowboy.client.websocket.reconnect.duration"
        | "cowboy.client.long_task.duration"
        | "cowboy.client.navigation.duration"
        | "cowboy.client.command.duration"
        | "cowboy.client.first_output.duration" => (Family::Client, false, "s"),
        REQUESTS | WAITS | DECISIONS | TOKENS | LOSSES => (Family::WaitCache, true, "1"),
        _ => return None,
    };
    metric.description.clear();
    metric.metadata.clear();
    metric.unit = unit.to_owned();
    match &mut metric.data {
        Some(metric::Data::Sum(sum_data)) if sum && sum_data.is_monotonic => {
            for p in &mut sum_data.data_points {
                p.attributes = labels(family, &p.attributes);
                p.exemplars.clear();
            }
        }
        Some(metric::Data::Histogram(histogram)) if !sum => {
            for p in &mut histogram.data_points {
                if !valid_client_histogram(p) {
                    return None;
                }
                p.attributes = labels(family, &p.attributes);
                p.exemplars.clear();
            }
        }
        _ => return None,
    }
    Some(family)
}

fn allowed(attrs: &[KeyValue], key: &str, values: &[&str]) -> Option<KeyValue> {
    let value = attrs
        .iter()
        .find(|attr| attr.key == key)?
        .value
        .as_ref()?
        .value
        .as_ref()?;
    let any_value::Value::StringValue(value) = value else {
        return None;
    };
    values.contains(&value.as_str()).then(|| string(key, value))
}

pub(crate) fn string(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.to_owned(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(value.to_owned())),
        }),
        ..Default::default()
    }
}

fn labels(family: Family, attrs: &[KeyValue]) -> Vec<KeyValue> {
    let allow: &[(&str, &[&str])] = match family {
        Family::Client => &[
            ("platform", &["ios", "macos", "web", "other"]),
            ("surface", &["mobile", "desktop", "other"]),
            ("connection", &["initial", "reconnect", "other"]),
            ("transport", &["websocket", "http", "other"]),
            (
                "reason",
                &[
                    "initial",
                    "close",
                    "error",
                    "online",
                    "visibility",
                    "timeout",
                    "heartbeat",
                    "other",
                ],
            ),
            (
                "operation",
                &[
                    "submit",
                    "prompt",
                    "open_session",
                    "cancel",
                    "set_config_option",
                    "sync",
                    "other",
                ],
            ),
        ],
        Family::WaitCache => &[
            (
                "provider",
                &[
                    "codex",
                    "claude-code",
                    "antigravity",
                    "codex-deepseek",
                    "claude-deepseek",
                    "deepseek-harness",
                    "grok",
                    "other",
                ],
            ),
            (
                "role",
                &["keepalive", "interactive", "auxiliary", "unknown"],
            ),
            (
                "kind",
                &[
                    "input",
                    "output",
                    "cache_read",
                    "cache_write",
                    "queue_drop",
                    "write_failure",
                ],
            ),
            (
                "outcome",
                &[
                    "usage",
                    "no_usage",
                    "error",
                    "incomplete",
                    "admitted",
                    "completed",
                    "failed",
                    "cancelled",
                    "renewed",
                    "notify_held",
                    "other",
                ],
            ),
        ],
    };
    allow
        .iter()
        .filter_map(|(key, values)| allowed(attrs, key, values))
        .collect()
}

/// Rebuild resource and scope even for metrics that already passed web ingress
/// validation. Controlled name/units and label values also prevent hidden series
/// dimensions via scope attributes, metric metadata and exemplar labels.
pub(crate) fn sanitize(bytes: &[u8]) -> Result<(Vec<u8>, usize)> {
    let Request::Metrics(mut request) = Request::decode(Signal::Metrics, bytes)? else {
        unreachable!("metrics signal decodes to a metric request");
    };
    for resource in &mut request.resource_metrics {
        let attrs = resource.resource.take().unwrap_or_default().attributes;
        let mut family = None;
        for scope in &mut resource.scope_metrics {
            scope
                .metrics
                .retain_mut(|metric| match (family, instrument(metric)) {
                    (_, None)
                    | (Some(Family::Client), Some(Family::WaitCache))
                    | (Some(Family::WaitCache), Some(Family::Client)) => false,
                    (_, Some(value)) => {
                        family = Some(value);
                        true
                    }
                });
            scope.schema_url.clear();
            let scope_name = scope.scope.as_ref().map(|scope| scope.name.as_str());
            scope.scope = Some(InstrumentationScope {
                name: match (family, scope_name) {
                    (Some(Family::Client), Some("cowboy.web")) => "cowboy.web",
                    (Some(Family::Client), _) => "cowboy.client.aggregate",
                    _ => "cowboy.wait_cache",
                }
                .into(),
                version: "1".into(),
                ..Default::default()
            });
        }
        resource
            .scope_metrics
            .retain(|scope| !scope.metrics.is_empty());
        let service = match family {
            Some(Family::Client) => allowed(
                &attrs,
                "service.name",
                &["cowboy-web", "cowboy-client-aggregate"],
            )
            .unwrap_or_else(|| string("service.name", "cowboy-web")),
            _ => string("service.name", "cowboy-wait-cache"),
        };
        let mut result = vec![service];
        if matches!(family, Some(Family::Client)) {
            result.extend(allowed(
                &attrs,
                "cowboy.platform",
                &["ios", "macos", "web", "other"],
            ));
            result.extend(allowed(
                &attrs,
                "cowboy.surface",
                &["mobile", "desktop", "other"],
            ));
        } else {
            result.extend(allowed(&attrs, "cowboy.source", &["controller", "machine"]));
        }
        resource.resource = Some(Resource {
            attributes: result,
            ..Default::default()
        });
        resource.schema_url.clear();
    }
    request
        .resource_metrics
        .retain(|resource| !resource.scope_metrics.is_empty());
    let request = Request::Metrics(request);
    let count = request.count();
    let Request::Metrics(metrics) = request else {
        unreachable!()
    };
    Ok((metrics.encode_to_vec(), count))
}

/// The signed export receipt binds the original payload. Reject unsafe
/// metrics instead of sending different bytes under that receipt. Require
/// Cowboy's canonical wire encoding. Unknown fields and overwritten values
/// can disappear on decode even when the decoded message is admissible.
#[cfg(feature = "machine-host")]
pub(crate) fn admitted(bytes: &[u8]) -> bool {
    sanitize(bytes).is_ok_and(|(safe, _)| safe == bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::{
        collector::metrics::v1::ExportMetricsServiceRequest,
        metrics::v1::{
            Exemplar, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum, number_data_point,
        },
    };

    fn sample(time: u64) -> ExportMetricsServiceRequest {
        let prohibited = [
            "session_id",
            "call_id",
            "session_fingerprint",
            "process.executable.path",
        ];
        let hostile = prohibited.map(|key| string(key, "private-fixture-value"));
        ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource {
                    attributes: hostile
                        .iter()
                        .cloned()
                        .chain([
                            string("service.name", "private-fixture-value"),
                            string("cowboy.source", "controller"),
                        ])
                        .collect(),
                    ..Default::default()
                }),
                scope_metrics: vec![ScopeMetrics {
                    scope: Some(InstrumentationScope {
                        name: "private-fixture-value".into(),
                        attributes: hostile.to_vec(),
                        ..Default::default()
                    }),
                    metrics: vec![Metric {
                        name: REQUESTS.into(),
                        metadata: hostile.to_vec(),
                        data: Some(metric::Data::Sum(Sum {
                            is_monotonic: true,
                            aggregation_temporality: 2,
                            data_points: vec![NumberDataPoint {
                                attributes: hostile
                                    .iter()
                                    .cloned()
                                    .chain([
                                        string("provider", "codex"),
                                        string("outcome", "private-fixture-value"),
                                    ])
                                    .collect(),
                                exemplars: vec![Exemplar {
                                    filtered_attributes: hostile.to_vec(),
                                    ..Default::default()
                                }],
                                value: Some(number_data_point::Value::AsInt(37)),
                                time_unix_nano: time,
                                start_time_unix_nano: time.saturating_sub(1_000_000_000),
                                ..Default::default()
                            }],
                        })),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    #[test]
    fn victoria_only_gets_finite_resource_labels_and_point_labels() {
        let bytes = sample(1_700_000_000_000_000_000).encode_to_vec();
        #[cfg(feature = "machine-host")]
        assert!(!admitted(&bytes));
        let (bytes, count) = sanitize(&bytes).unwrap();
        #[cfg(feature = "machine-host")]
        assert!(admitted(&bytes));
        assert_eq!(count, 1);
        let filtered = ExportMetricsServiceRequest::decode(bytes.as_slice()).unwrap();
        let resource = &filtered.resource_metrics[0];
        assert_eq!(
            resource.resource.as_ref().unwrap().attributes,
            [
                string("service.name", "cowboy-wait-cache"),
                string("cowboy.source", "controller"),
            ]
        );
        let metric = &resource.scope_metrics[0].metrics[0];
        let Some(metric::Data::Sum(value)) = &metric.data else {
            panic!("missing counter");
        };
        assert_eq!(
            value.data_points[0].attributes,
            [string("provider", "codex")]
        );
        assert!(value.data_points[0].exemplars.is_empty());
        assert!(
            !serde_json::to_string(&filtered)
                .unwrap()
                .contains("private-fixture-value")
        );

        for provider in ["antigravity", "deepseek-harness", "grok"] {
            let mut req = sample(1_700_000_000_000_000_000);
            let Some(metric::Data::Sum(sum)) =
                &mut req.resource_metrics[0].scope_metrics[0].metrics[0].data
            else {
                panic!("missing sum");
            };
            sum.data_points[0].attributes = vec![string("provider", provider)];
            let (clean, c) = sanitize(&req.encode_to_vec()).unwrap();
            assert_eq!(c, 1);
            #[cfg(feature = "machine-host")]
            assert!(admitted(&clean), "{provider}");
            let decoded = ExportMetricsServiceRequest::decode(clean.as_slice()).unwrap();
            let Some(metric::Data::Sum(clean_sum)) =
                &decoded.resource_metrics[0].scope_metrics[0].metrics[0].data
            else {
                panic!("missing decoded sum");
            };
            assert_eq!(
                clean_sum.data_points[0].attributes,
                [string("provider", provider)]
            );
        }
    }

    #[cfg(feature = "machine-host")]
    #[test]
    fn rejects_wire_fields_discarded_by_the_decoder() {
        #[derive(prost::Message)]
        struct RawResources {
            #[prost(bytes = "vec", repeated, tag = "1")]
            resources: Vec<Vec<u8>>,
        }

        let (canonical, _) = sanitize(&sample(1).encode_to_vec()).unwrap();
        let expected = ExportMetricsServiceRequest::decode(canonical.as_slice()).unwrap();
        let base = expected.resource_metrics[0].encode_to_vec();
        let secret = "private-fixture-value".to_owned();
        for field in [200, 3] {
            let mut resource = base.clone();
            prost::encoding::string::encode(field, &secret, &mut resource);
            if field == 3 {
                // The later schema_url overwrites the private value on decode.
                prost::encoding::string::encode(field, &String::new(), &mut resource);
            }
            let raw = RawResources {
                resources: vec![resource],
            }
            .encode_to_vec();
            assert_eq!(
                ExportMetricsServiceRequest::decode(raw.as_slice()).unwrap(),
                expected
            );
            assert!(!admitted(&raw));
        }
        assert!(admitted(&canonical));
    }

    #[cfg(feature = "machine-host")]
    #[test]
    fn victoria_histograms_require_fixed_boundaries_and_consistent_bucket_counts() {
        use opentelemetry_proto::tonic::metrics::v1::Histogram;

        let mut raw = sample(1);
        let mut point = HistogramDataPoint {
            explicit_bounds: CLIENT_HISTOGRAM_BOUNDS.to_vec(),
            bucket_counts: vec![1; CLIENT_HISTOGRAM_BOUNDS.len() + 1],
            count: CLIENT_HISTOGRAM_BOUNDS.len() as u64 + 1,
            ..Default::default()
        };
        let data = |p| {
            Some(metric::Data::Histogram(Histogram {
                data_points: vec![p],
                ..Default::default()
            }))
        };
        let instrument = &mut raw.resource_metrics[0].scope_metrics[0].metrics[0];
        instrument.name = "cowboy.client.command.duration".into();
        instrument.data = data(point.clone());
        let (safe, count) = sanitize(&raw.encode_to_vec()).unwrap();
        assert_eq!(count, 1);
        assert!(admitted(&safe));

        let mut invalid_boundary = point.clone();
        invalid_boundary.explicit_bounds[0] = 0.006;
        let mut missing_bucket = point.clone();
        missing_bucket.bucket_counts.pop();
        point.bucket_counts[0] = u64::MAX;
        for bad in [invalid_boundary, missing_bucket, point] {
            let mut candidate = ExportMetricsServiceRequest::decode(safe.as_slice()).unwrap();
            candidate.resource_metrics[0].scope_metrics[0].metrics[0].data = data(bad);
            assert!(!admitted(&candidate.encode_to_vec()));
        }
    }

    /// Local synthetic query-back with the pinned executable, no live service.
    #[tokio::test]
    #[ignore = "requires COWBOY_TEST_VICTORIA_METRICS (isolated test instance)"]
    async fn victoria_query_back() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let executable =
            std::env::var_os("COWBOY_TEST_VICTORIA_METRICS").expect("test VictoriaMetrics binary");
        let storage = tempfile::tempdir().unwrap();
        let address = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap();
        let mut process = tokio::process::Command::new(executable);
        process
            .env("GOMAXPROCS", "2")
            .kill_on_drop(true)
            .arg(format!("-storageDataPath={}", storage.path().display()))
            .arg(format!("-httpListenAddr={address}"))
            .args([
                "-retentionPeriod=1d",
                "-memory.allowedBytes=128MiB",
                "-selfScrapeInterval=0s",
                "-search.disableCache",
                "-inmemoryDataFlushInterval=1s",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut process = process.spawn().unwrap();
        let base = format!("http://{address}");
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        let healthy = async {
            loop {
                if http
                    .get(format!("{base}/health"))
                    .send()
                    .await
                    .is_ok_and(|r| r.status().is_success())
                {
                    break;
                }
                assert!(
                    process.try_wait().unwrap().is_none(),
                    "test VictoriaMetrics quit"
                );
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(8), healthy)
            .await
            .unwrap();
        let time = u64::try_from(chrono::Utc::now().timestamp_nanos_opt().unwrap())
            .unwrap()
            .saturating_sub(1_000_000_000);
        let (bytes, _) = sanitize(&sample(time).encode_to_vec()).unwrap();
        let posted = http
            .post(format!("{base}/opentelemetry/v1/metrics"))
            .header("Content-Type", "application/x-protobuf")
            .body(bytes)
            .send()
            .await
            .unwrap();
        assert!(posted.status().is_success(), "{}", posted.status());
        let observed = async {
            loop {
                let body = http
                    .get(format!("{base}/api/v1/export"))
                    .query(&[
                        ("match[]", "{__name__!=\"\"}"),
                        ("max_rows_per_line", "100"),
                    ])
                    .send()
                    .await
                    .unwrap()
                    .error_for_status()
                    .unwrap()
                    .text()
                    .await
                    .unwrap();
                if !body.trim().is_empty() {
                    break serde_json::from_str::<serde_json::Value>(body.trim()).unwrap();
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        };
        let payload = tokio::time::timeout(std::time::Duration::from_secs(35), observed)
            .await
            .unwrap();
        assert_eq!(
            payload["metric"],
            serde_json::json!({
                "__name__": REQUESTS,
                "cowboy.source": "controller",
                "provider": "codex",
                "scope.name": "cowboy.wait_cache",
                "scope.version": "1",
                "service.name": "cowboy-wait-cache",
            })
        );
        assert_eq!(payload["values"], serde_json::json!([37]));
        assert_eq!(payload["timestamps"], serde_json::json!([time / 1_000_000]));
        process.kill().await.unwrap();
    }
}
