//! Export one closed UTC cohort as cumulative counters. Series are aggregated
//! again across the diagnostic dimensions and then passed through the same
//! allowlist as the signed VictoriaMetrics connector.
use super::{DAY_MS, SqliteStore, utc_day};
use crate::logs::wait_cache as events;
use crate::metrics_egress::{self, DECISIONS, LOSSES, REQUESTS, TOKENS, WAITS, string};
use anyhow::{Result, ensure};
use opentelemetry_proto::tonic::{
    collector::metrics::v1::ExportMetricsServiceRequest,
    common::v1::KeyValue,
    metrics::v1::{
        Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum, metric, number_data_point,
    },
    resource::v1::Resource,
};
use prost::Message as _;
use serde_json::Value;
use std::collections::BTreeMap;

type Series = BTreeMap<(&'static str, &'static str, &'static str, &'static str), u64>;

impl SqliteStore {
    /// Counters only, one complete UTC day per request. The directory is one
    /// source of truth. Callers collecting multiple Machines aggregate by
    /// provider first rather than treating host identities as metric labels.
    pub fn wait_cache_otlp(&self, day: i64, now: i64, controller: bool) -> Result<Vec<u8>> {
        ensure!(
            day >= 0 && day == utc_day(day) && day <= now.saturating_sub(DAY_MS),
            "wait/cache metric export requires a closed UTC day"
        );
        let report = self.wait_cache_daily(day, day + DAY_MS)?;
        ensure!(
            report["truncated"] == false,
            "wait/cache cohort exceeded the rollup bound"
        );
        ensure!(
            report["degraded_days_ms"]
                .as_array()
                .is_some_and(Vec::is_empty),
            "wait/cache cohort has incomplete rollup coverage"
        );
        let mut series = Series::new();
        if let Some(groups) = report["groups"].as_array() {
            for group in groups {
                add_group(&mut series, group);
            }
        }
        let request = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource {
                    attributes: vec![string(
                        "cowboy.source",
                        if controller { "controller" } else { "machine" },
                    )],
                    ..Default::default()
                }),
                scope_metrics: vec![ScopeMetrics {
                    metrics: counters(series, day),
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let (protobuf, _) = metrics_egress::sanitize(&request.encode_to_vec())?;
        Ok(protobuf)
    }
}

fn add_group(series: &mut Series, group: &Value) {
    let dim = &group["dimensions"];
    let provider = events::provider(dim["provider"].as_str().unwrap_or_default());
    let role = match dim["role"].as_str() {
        Some("parent" | "subagent" | "managed_child") => "interactive",
        Some("keepalive") => "keepalive",
        Some("auxiliary") => "auxiliary",
        _ => "unknown",
    };
    let outcome = events::outcome(dim["outcome"].as_str().unwrap_or_default());
    let records = group["totals"]["records"].as_u64().unwrap_or_default();
    let sum = |key| group["totals"]["values"][key][0].as_u64();
    let mut add = |metric, provider, label, role, value: u64| {
        let total = series.entry((metric, provider, label, role)).or_default();
        *total = total.saturating_add(value);
    };
    match dim["event"].as_str() {
        Some(events::REQUEST) => {
            let outcome = match outcome {
                "usage" | "error" | "incomplete" | "no_usage" => outcome,
                _ => "other",
            };
            add(REQUESTS, provider, outcome, role, records);
            for (key, kind) in [
                ("input_tokens", "input"),
                ("output_tokens", "output"),
                ("cache_read_input_tokens", "cache_read"),
                ("cache_write_input_tokens", "cache_write"),
            ] {
                if let Some(value) = sum(key) {
                    add(TOKENS, provider, kind, role, value);
                }
            }
        }
        Some(events::SETTLED | events::DISPATCH) => {
            let outcome = match outcome {
                "admitted" | "completed" | "failed" | "cancelled" => outcome,
                _ => "other",
            };
            add(WAITS, "", outcome, "", records);
        }
        Some(events::DECISION) => {
            let outcome = match outcome {
                "renewed" | "notify_held" => outcome,
                _ => "other",
            };
            add(DECISIONS, "", outcome, "", records);
        }
        Some(events::LOSS) => {
            for (key, kind) in [
                ("local_dropped_records", "queue_drop"),
                ("local_write_failures", "write_failure"),
            ] {
                if let Some(value) = sum(key) {
                    add(LOSSES, "", kind, "", value);
                }
            }
        }
        _ => {}
    }
}

fn counters(series: Series, day: i64) -> Vec<Metric> {
    let start = u64::try_from(day)
        .unwrap_or_default()
        .saturating_mul(1_000_000);
    let end = start.saturating_add(u64::try_from(DAY_MS - 1).unwrap_or_default() * 1_000_000);
    let mut instruments: BTreeMap<&str, Vec<NumberDataPoint>> = BTreeMap::new();
    for ((metric, provider, label, role), value) in series {
        let key = if matches!(metric, TOKENS | LOSSES) {
            "kind"
        } else {
            "outcome"
        };
        let attributes = [(key, label), ("provider", provider), ("role", role)]
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(key, value)| string(key, value))
            .collect::<Vec<KeyValue>>();
        instruments
            .entry(metric)
            .or_default()
            .push(NumberDataPoint {
                attributes,
                start_time_unix_nano: start,
                time_unix_nano: end,
                value: Some(number_data_point::Value::AsInt(
                    i64::try_from(value).unwrap_or(i64::MAX),
                )),
                ..Default::default()
            });
    }
    instruments
        .into_iter()
        .map(|(name, data_points)| Metric {
            name: name.to_owned(),
            unit: "1".into(),
            data: Some(metric::Data::Sum(Sum {
                data_points,
                aggregation_temporality: 2,
                is_monotonic: true,
            })),
            ..Default::default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noninteractive_requests_keep_separate_finite_series() {
        let mut series = Series::new();
        for role in ["parent", "keepalive", "auxiliary", "unknown", "unexpected"] {
            add_group(
                &mut series,
                &serde_json::json!({
                    "dimensions": {"event":events::REQUEST,"provider":"codex","role":role,"outcome":"usage"},
                    "totals": {"records":1},
                }),
            );
        }
        assert_eq!(series[&(REQUESTS, "codex", "usage", "interactive")], 1);
        assert_eq!(series[&(REQUESTS, "codex", "usage", "keepalive")], 1);
        assert_eq!(series[&(REQUESTS, "codex", "usage", "auxiliary")], 1);
        assert_eq!(series[&(REQUESTS, "codex", "usage", "unknown")], 2);
        let request = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                scope_metrics: vec![ScopeMetrics {
                    metrics: counters(series, 0),
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let (safe, count) = metrics_egress::sanitize(&request.encode_to_vec()).unwrap();
        assert_eq!(count, 4);
        let projected = ExportMetricsServiceRequest::decode(safe.as_slice()).unwrap();
        let Some(metric::Data::Sum(sum)) =
            &projected.resource_metrics[0].scope_metrics[0].metrics[0].data
        else {
            panic!("missing request counter");
        };
        for (point, role) in
            sum.data_points
                .iter()
                .zip(["auxiliary", "interactive", "keepalive", "unknown"])
        {
            assert!(point.attributes.contains(&string("role", role)));
        }
    }
}
