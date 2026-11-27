//! Export one closed UTC cohort as cumulative counters. Series are aggregated
//! again across the diagnostic dimensions and then passed through the same
//! allowlist as the signed VictoriaMetrics connector.
use super::{DAY_MS, SqliteStore, utc_day};
use crate::logs::storage::{atomic_json, private_read};
use crate::logs::wait_cache as events;
use crate::metrics_egress::{self, DECISIONS, LOSSES, REQUESTS, TOKENS, WAITS, string};
use anyhow::{Context as _, Result, ensure};
use opentelemetry_proto::tonic::{
    collector::metrics::v1::ExportMetricsServiceRequest,
    metrics::v1::{
        Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum, metric, number_data_point,
    },
    resource::v1::Resource,
};
use prost::Message as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

type Series = BTreeMap<(&'static str, &'static str, &'static str, &'static str), u64>;

/// Verified store identities collected across one closed UTC day's reports.
pub struct EncodedWaitCacheOtlp {
    pub bytes: Vec<u8>,
    pub store_ids: Vec<String>,
}

/// Aggregate one or more closed-UTC-day `wait-cache-daily` reports into a single
/// single-writer OTLP cumulative counter payload. Fails closed on:
/// - wrong schema or day window,
/// - missing or duplicate `store_id` (preventing alias double-counting while
///   keeping `store_id` internal to collection),
/// - absent rollup database (`rollup_present != true`),
/// - truncation, degraded days, or any `coverage_issues` (including writer loss),
/// - malformed or cross-day group rows.
pub fn encode_wait_cache_otlp(
    day: i64,
    now: i64,
    reports: &[&Value],
    controller: bool,
) -> Result<EncodedWaitCacheOtlp> {
    ensure!(
        day >= 0 && day == utc_day(day) && day <= now.saturating_sub(DAY_MS),
        "wait/cache metric export requires a closed UTC day"
    );
    ensure!(
        !reports.is_empty(),
        "wait/cache metric export requires at least one source report"
    );
    let mut series = Series::new();
    let mut store_ids = BTreeSet::new();
    let expected_to = day
        .checked_add(DAY_MS)
        .ok_or_else(|| anyhow::anyhow!("wait/cache day overflow"))?;
    for report in reports {
        ensure!(
            report["schema"].as_str() == Some("cowboy.wait_cache.daily/v1"),
            "wait/cache cohort has unsupported report schema"
        );
        ensure!(
            report["from_ms"].as_i64() == Some(day)
                && report["to_ms"].as_i64() == Some(expected_to),
            "wait/cache cohort report window does not match requested UTC day"
        );
        let store_id = report["store_id"]
            .as_str()
            .filter(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| anyhow::anyhow!("wait/cache cohort missing valid internal store_id"))?;
        ensure!(
            store_ids.insert(store_id.to_owned()),
            "duplicate log store_id in wait/cache metrics aggregation"
        );
        ensure!(
            report["rollup_present"].as_bool() == Some(true),
            "wait/cache cohort source lacks an initialized rollup database"
        );
        ensure!(
            report["truncated"].as_bool() == Some(false),
            "wait/cache cohort exceeded the rollup bound"
        );
        ensure!(
            report["degraded_days_ms"]
                .as_array()
                .is_some_and(Vec::is_empty),
            "wait/cache cohort has incomplete rollup coverage"
        );
        ensure!(
            report["coverage_issues"]
                .as_array()
                .is_some_and(Vec::is_empty),
            "wait/cache cohort has unresolved coverage or writer-loss issues"
        );
        let groups = report["groups"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("wait/cache cohort missing groups array"))?;
        for group in groups {
            ensure!(
                group["from_ms"].as_i64() == Some(day)
                    && group["to_ms"].as_i64() == Some(expected_to)
                    && group["dimensions"].is_object()
                    && group["totals"].is_object(),
                "wait/cache cohort contains malformed or cross-day group"
            );
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
    Ok(EncodedWaitCacheOtlp {
        bytes: protobuf,
        store_ids: store_ids.into_iter().collect(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WriterState {
    schema: u16,
    cohort_store_ids: Vec<String>,
    exported_days: BTreeMap<String, ExportedDay>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportedDay {
    payload_sha256: String,
    store_ids: Vec<String>,
    exported_at_ms: i64,
}

/// Bind `cowboy logs wait-cache-metrics` to a single writer, one authoritative
/// sorted `store_id` cohort, and immutable per-day payload hashes so subset
/// invocations or stale retries can never emit conflicting samples for the
/// same `cowboy.source="machine"` day-end series.
pub fn verify_and_record_writer_state(
    path: &Path,
    day: chrono::NaiveDate,
    now_ms: i64,
    encoded: &EncodedWaitCacheOtlp,
    update_cohort: bool,
) -> Result<()> {
    let day_key = day.format("%Y-%m-%d").to_string();
    let payload_sha256 = format!("{:x}", Sha256::digest(&encoded.bytes));
    let mut state = match std::fs::symlink_metadata(path) {
        Ok(_) => {
            let raw = private_read(path, 512 * 1024)?;
            let parsed: WriterState =
                serde_json::from_slice(&raw).context("invalid wait-cache writer state file")?;
            ensure!(
                parsed.schema == 1,
                "unsupported wait-cache writer state schema"
            );
            parsed
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => WriterState {
            schema: 1,
            cohort_store_ids: encoded.store_ids.clone(),
            exported_days: BTreeMap::new(),
        },
        Err(error) => return Err(error).context("reading wait-cache writer state"),
    };

    if let Some(existing) = state.exported_days.get(&day_key) {
        ensure!(
            existing.payload_sha256 == payload_sha256 && existing.store_ids == encoded.store_ids,
            "refusing to publish conflicting wait/cache metrics for already-exported day {day_key}"
        );
        return Ok(());
    }

    if state.cohort_store_ids != encoded.store_ids {
        ensure!(
            update_cohort,
            "wait/cache source cohort does not match bound writer state; pass --update-cohort only when intentionally changing the authoritative Machine set"
        );
        state.cohort_store_ids = encoded.store_ids.clone();
    }

    state.exported_days.insert(
        day_key,
        ExportedDay {
            payload_sha256,
            store_ids: encoded.store_ids.clone(),
            exported_at_ms: now_ms,
        },
    );
    // Keep up to 180 most recent exported days so state stays bounded.
    while state.exported_days.len() > 180 {
        if let Some(oldest) = state.exported_days.keys().next().cloned() {
            state.exported_days.remove(&oldest);
        }
    }
    atomic_json(path, &state)
}

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
        Ok(encode_wait_cache_otlp(day, now, &[&report], controller)?.bytes)
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
    let mut by_name: BTreeMap<&'static str, Vec<NumberDataPoint>> = BTreeMap::new();
    let start = u64::try_from(day)
        .unwrap_or_default()
        .saturating_mul(1_000_000);
    let end = u64::try_from(day + DAY_MS)
        .unwrap_or_default()
        .saturating_mul(1_000_000);
    for ((name, provider, label, role), value) in series {
        let mut attributes = Vec::new();
        if !provider.is_empty() {
            attributes.push(string("provider", provider));
        }
        let key = match name {
            TOKENS => "token",
            LOSSES => "reason",
            _ => "outcome",
        };
        attributes.push(string(key, label));
        if !role.is_empty() {
            attributes.push(string("role", role));
        }
        attributes.sort_by(|a, b| a.key.cmp(&b.key));
        let point = NumberDataPoint {
            attributes,
            start_time_unix_nano: start,
            time_unix_nano: end,
            value: Some(number_data_point::Value::AsInt(
                i64::try_from(value).unwrap_or(i64::MAX),
            )),
            ..Default::default()
        };
        by_name.entry(name).or_default().push(point);
    }
    by_name
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

    fn valid_report(day: i64, store_id: &str, groups: Value) -> Value {
        serde_json::json!({
            "schema": "cowboy.wait_cache.daily/v1",
            "store_id": store_id,
            "rollup_present": true,
            "from_ms": day,
            "to_ms": day + DAY_MS,
            "truncated": false,
            "degraded_days_ms": [],
            "coverage_issues": [],
            "groups": groups,
        })
    }

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

    #[test]
    fn multi_machine_encode_validates_store_uniqueness_rollup_presence_day_and_coverage() {
        let day = 10 * DAY_MS;
        let now = day + 2 * DAY_MS;
        let hawk_id = "11111111111111111111111111111111";
        let ovh_id = "22222222222222222222222222222222";
        let hawk = valid_report(
            day,
            hawk_id,
            serde_json::json!([{
                "from_ms": day,
                "to_ms": day + DAY_MS,
                "dimensions": {"event": events::REQUEST, "provider": "codex", "role": "parent", "outcome": "usage"},
                "totals": {"records": 10, "values": {"input_tokens": [1000, 10], "cache_read_input_tokens": [800, 10]}}
            }]),
        );
        let ovh = valid_report(
            day,
            ovh_id,
            serde_json::json!([
                {
                    "from_ms": day,
                    "to_ms": day + DAY_MS,
                    "dimensions": {"event": events::REQUEST, "provider": "codex", "role": "parent", "outcome": "usage"},
                    "totals": {"records": 5, "values": {"input_tokens": [500, 5], "cache_read_input_tokens": [300, 5]}}
                },
                {
                    "from_ms": day,
                    "to_ms": day + DAY_MS,
                    "dimensions": {"event": events::TURN_DIAGNOSTICS, "provider": "antigravity", "role": "parent", "outcome": "usage"},
                    "totals": {"records": 7}
                }
            ]),
        );
        let encoded = encode_wait_cache_otlp(day, now, &[&hawk, &ovh], false).unwrap();
        assert_eq!(encoded.store_ids, vec![hawk_id, ovh_id]);
        #[cfg(feature = "machine-host")]
        assert!(metrics_egress::admitted(&encoded.bytes));
        let decoded = ExportMetricsServiceRequest::decode(encoded.bytes.as_slice()).unwrap();
        let metrics = &decoded.resource_metrics[0].scope_metrics[0].metrics;
        let reqs = metrics.iter().find(|m| m.name == REQUESTS).unwrap();
        let Some(metric::Data::Sum(sum)) = &reqs.data else {
            panic!("missing request sum");
        };
        // Antigravity turn_diagnostics must NOT increment HTTP request counter
        assert_eq!(sum.data_points.len(), 1);
        assert_eq!(
            sum.data_points[0].value,
            Some(number_data_point::Value::AsInt(15))
        );

        // Duplicate store_id fails closed
        assert!(encode_wait_cache_otlp(day, now, &[&hawk, &hawk], false).is_err());

        // Missing rollup DB fails closed (never passes as zero)
        let mut no_rollup = ovh.clone();
        no_rollup["rollup_present"] = serde_json::json!(false);
        assert!(encode_wait_cache_otlp(day, now, &[&hawk, &no_rollup], false).is_err());

        // Wrong day fails closed
        let wrong_day = valid_report(day - DAY_MS, ovh_id, serde_json::json!([]));
        assert!(encode_wait_cache_otlp(day, now, &[&hawk, &wrong_day], false).is_err());

        // Writer-loss coverage issue fails closed
        let mut loss = ovh.clone();
        loss["coverage_issues"] = serde_json::json!(["local_dropped_records"]);
        assert!(encode_wait_cache_otlp(day, now, &[&hawk, &loss], false).is_err());

        // Single-writer cohort & day immutability
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("writer-state.json");
        let date = chrono::NaiveDate::from_ymd_opt(2026, 11, 19).unwrap();
        verify_and_record_writer_state(&state_path, date, now, &encoded, false).unwrap();
        // Idempotent retry of exact same day & payload succeeds
        verify_and_record_writer_state(&state_path, date, now, &encoded, false).unwrap();
        // Subset (hawk-only) for same day fails closed
        let hawk_only = encode_wait_cache_otlp(day, now, &[&hawk], false).unwrap();
        assert!(verify_and_record_writer_state(&state_path, date, now, &hawk_only, false).is_err());
        // Subset for next day fails closed without --update-cohort, succeeds with --update-cohort
        let next_date = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        assert!(
            verify_and_record_writer_state(&state_path, next_date, now, &hawk_only, false).is_err()
        );
        verify_and_record_writer_state(&state_path, next_date, now, &hawk_only, true).unwrap();
    }
}
