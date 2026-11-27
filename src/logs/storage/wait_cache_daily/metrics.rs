//! Export one closed UTC cohort as cumulative counters. Series are aggregated
//! again across the diagnostic dimensions and then passed through the same
//! allowlist as the signed VictoriaMetrics connector.
use super::{DAY_MS, RETENTION_MS, SqliteStore, utc_day};
use crate::logs::storage::{atomic_json, private_read};
use crate::logs::wait_cache as events;
use crate::metrics_egress::{self, DECISIONS, LOSSES, REQUESTS, TOKENS, WAITS, string};
use anyhow::{Context as _, Result, ensure};
use opentelemetry_proto::tonic::{
    collector::metrics::v1::ExportMetricsServiceRequest,
    common::v1::KeyValue,
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
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

type Series = BTreeMap<(&'static str, &'static str, &'static str, &'static str), u64>;

/// Settle margin after UTC midnight before multi-Machine CLI exports freeze a
/// closed day's single-writer claim (exceeds worker OTLP coalescing + flush lag).
const FINALIZE_GRACE_MS: i64 = 60 * 60 * 1000;
const MAX_WRITER_STATE_BYTES: usize = 4 * 1024 * 1024;
const MAX_WRITER_STATE_DAYS: usize = 46;

/// Verified store identities collected across one closed UTC day's reports.
pub struct EncodedWaitCacheOtlp {
    pub bytes: Vec<u8>,
    pub store_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportGroup {
    from_ms: i64,
    to_ms: i64,
    dimensions: serde_json::Map<String, Value>,
    totals: ReportTotals,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportTotals {
    records: u64,
    #[serde(default)]
    values: BTreeMap<String, [u64; 2]>,
}

/// Aggregate one or more closed-UTC-day `wait-cache-daily` reports into a single
/// single-writer OTLP cumulative counter payload. Fails closed on:
/// - unclosed, unfinalized, or expired (>45d) UTC days,
/// - wrong schema or day window,
/// - missing or duplicate `store_id` (preventing alias double-counting while
///   keeping `store_id` internal to collection),
/// - absent rollup database (`rollup_present != true`) or `capacity_floor_ms > day`,
/// - truncation, degraded days, or any `coverage_issues` (including writer loss),
/// - malformed, duplicate, or cross-day groups and invalid total tuples.
pub fn encode_wait_cache_otlp(
    day: i64,
    now: i64,
    reports: &[&Value],
    controller: bool,
) -> Result<EncodedWaitCacheOtlp> {
    let required_close = if controller {
        day.saturating_add(DAY_MS)
    } else {
        day.saturating_add(DAY_MS).saturating_add(FINALIZE_GRACE_MS)
    };
    ensure!(
        day >= 0 && day == utc_day(day) && now >= required_close,
        "wait/cache metric export requires a closed and finalized UTC day"
    );
    ensure!(
        day >= utc_day(now).saturating_sub(RETENTION_MS),
        "wait/cache cohort is outside the 45-day retention horizon"
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
        // On real wall-clock runs (not 1970 synthetic test epochs), require each
        // source's own clock to confirm the day is both finalized and retained.
        if !controller && now >= 1_000_000_000_000 {
            if let Some(source_now) = report["store_now_ms"].as_i64() {
                ensure!(
                    source_now >= required_close,
                    "wait/cache source clock has not passed the day finalization boundary"
                );
                ensure!(
                    day >= utc_day(source_now).saturating_sub(RETENTION_MS),
                    "wait/cache day is outside source's 45-day retention horizon"
                );
            }
        }
        if let Some(floor) = report["capacity_floor_ms"].as_i64() {
            ensure!(
                day >= floor,
                "wait/cache cohort predates source capacity_floor_ms"
            );
        }
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
        let raw_groups = report["groups"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("wait/cache cohort missing groups array"))?;
        let mut seen_dimensions = BTreeSet::new();
        for raw_group in raw_groups {
            let group: ReportGroup = serde_json::from_value(raw_group.clone())
                .context("wait/cache cohort contains malformed group or totals")?;
            ensure!(
                group.from_ms == day && group.to_ms == expected_to,
                "wait/cache cohort contains cross-day group"
            );
            validate_group_dimensions(&group.dimensions)?;
            let canonical_dim = serde_json::to_string(&group.dimensions)?;
            ensure!(
                seen_dimensions.insert(canonical_dim),
                "wait/cache cohort contains duplicate dimension group within one source"
            );
            for &[sum, count] in group.totals.values.values() {
                let _ = sum;
                ensure!(
                    count >= 1 && count <= group.totals.records,
                    "wait/cache cohort contains zero-sample or out-of-range value tuple"
                );
            }
            add_group(&mut series, &group);
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
struct CohortEffective {
    effective_day: String,
    store_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportedDay {
    payload_sha256: String,
    payload_hex: String,
    store_ids: Vec<String>,
    exported_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WriterState {
    schema: u16,
    cohort_history: Vec<CohortEffective>,
    exported_days: BTreeMap<String, ExportedDay>,
}

fn lock_path_for(state_path: &Path) -> Result<PathBuf> {
    let parent = state_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = state_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid wait-cache writer state filename"))?;
    Ok(parent.join(format!(".{name}.lock")))
}

struct FileLock(#[allow(dead_code)] File);

fn acquire_writer_lock(state_path: &Path) -> Result<FileLock> {
    let lock_path = lock_path_for(state_path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&lock_path)
        .with_context(|| format!("opening writer state lock {}", lock_path.display()))?;
    file.lock()
        .with_context(|| format!("locking wait-cache writer state {}", lock_path.display()))?;
    Ok(FileLock(file))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn decode_hex(hex: &str) -> Result<Vec<u8>> {
    ensure!(
        hex.len().is_multiple_of(2),
        "corrupt hex payload in writer state"
    );
    let mut out = Vec::with_capacity(hex.len() / 2);
    let (chunks, _) = hex.as_bytes().as_chunks::<2>();
    for pair in chunks {
        let s = std::str::from_utf8(pair).context("non-UTF8 hex payload")?;
        out.push(u8::from_str_radix(s, 16).context("invalid hex byte")?);
    }
    Ok(out)
}

/// Serialize access via an exclusive file lock beside `path`, validate `day`
/// against the effective-day cohort history, and atomically claim and persist
/// the exact OTLP payload bytes.
///
/// Re-running for an already-claimed day with the same cohort replays the
/// exact persisted payload bytes so a failed stdout delivery never strands a
/// day behind an unrecoverable hash. Cohort changes (`--update-cohort`) apply
/// only going forward from a strictly newer UTC day, never retroactively.
fn validate_group_dimensions(dim: &serde_json::Map<String, Value>) -> Result<()> {
    let event = dim
        .get("event")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("wait/cache group missing string event dimension"))?;
    match event {
        events::REQUEST => {
            for required in ["provider", "role", "outcome"] {
                ensure!(
                    dim.get(required)
                        .and_then(Value::as_str)
                        .is_some_and(|v| !v.is_empty()),
                    "wait/cache request group missing required dimension {required}"
                );
            }
        }
        events::SETTLED | events::DISPATCH | events::DECISION => {
            ensure!(
                dim.get("outcome")
                    .and_then(Value::as_str)
                    .is_some_and(|v| !v.is_empty()),
                "wait/cache {event} group missing required outcome dimension"
            );
        }
        events::TURN_DIAGNOSTICS
        | events::RECEIPT
        | events::NOTICE
        | events::LOSS
        | events::COVERAGE
        | "cowboy.context_rollover.history_read" => {}
        other => anyhow::bail!("unsupported wait/cache group event dimension {other}"),
    }
    Ok(())
}

/// Fast state-only replay path under exclusive writer lock: if `day` is already
/// claimed in `path` and its payload checksum verifies, returns the exact
/// persisted bytes without issuing any live source RPCs.
pub fn try_replay_claimed_writer_state(
    path: &Path,
    day: chrono::NaiveDate,
) -> Result<Option<Vec<u8>>> {
    let _lock = acquire_writer_lock(path)?;
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("reading wait-cache writer state"),
    }
    let raw = private_read(path, 8 * 1024 * 1024)?;
    let parsed: WriterState =
        serde_json::from_slice(&raw).context("invalid wait-cache writer state file")?;
    ensure!(
        parsed.schema == 1 && !parsed.cohort_history.is_empty(),
        "unsupported wait-cache writer state schema"
    );
    let day_key = day.format("%Y-%m-%d").to_string();
    let Some(existing) = parsed.exported_days.get(&day_key) else {
        return Ok(None);
    };
    let bytes = decode_hex(&existing.payload_hex)?;
    let actual_sha = format!("{:x}", Sha256::digest(&bytes));
    ensure!(
        actual_sha == existing.payload_sha256,
        "wait-cache writer state payload checksum mismatch for {day_key}"
    );
    Ok(Some(bytes))
}

pub fn claim_or_replay_writer_state(
    path: &Path,
    day: chrono::NaiveDate,
    now_ms: i64,
    encoded: &EncodedWaitCacheOtlp,
    update_cohort: bool,
) -> Result<Vec<u8>> {
    let _lock = acquire_writer_lock(path)?;
    let day_key = day.format("%Y-%m-%d").to_string();
    let payload_sha256 = format!("{:x}", Sha256::digest(&encoded.bytes));
    let mut state = match std::fs::symlink_metadata(path) {
        Ok(_) => {
            let raw = private_read(path, 8 * 1024 * 1024)?;
            let parsed: WriterState =
                serde_json::from_slice(&raw).context("invalid wait-cache writer state file")?;
            ensure!(
                parsed.schema == 1 && !parsed.cohort_history.is_empty(),
                "unsupported wait-cache writer state schema"
            );
            parsed
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => WriterState {
            schema: 1,
            cohort_history: vec![CohortEffective {
                effective_day: day_key.clone(),
                store_ids: encoded.store_ids.clone(),
            }],
            exported_days: BTreeMap::new(),
        },
        Err(error) => return Err(error).context("reading wait-cache writer state"),
    };

    if let Some(existing) = state.exported_days.get(&day_key) {
        ensure!(
            existing.store_ids == encoded.store_ids,
            "refusing to replay or alter already-exported day {day_key} with a different source cohort"
        );
        let bytes = decode_hex(&existing.payload_hex)?;
        let actual_sha = format!("{:x}", Sha256::digest(&bytes));
        ensure!(
            actual_sha == existing.payload_sha256,
            "wait-cache writer state payload checksum mismatch for {day_key}"
        );
        return Ok(bytes);
    }

    // Find the cohort effective on `day_key` (never apply newer cohort changes retroactively).
    let effective = state
        .cohort_history
        .iter()
        .rev()
        .find(|entry| entry.effective_day <= day_key)
        .or_else(|| state.cohort_history.first())
        .ok_or_else(|| anyhow::anyhow!("writer state missing cohort history"))?;

    if effective.store_ids != encoded.store_ids {
        ensure!(
            update_cohort,
            "wait/cache source cohort does not match bound cohort effective on {day_key}; pass --update-cohort only when intentionally changing the authoritative Machine set going forward"
        );
        let latest_effective = state
            .cohort_history
            .last()
            .map(|e| e.effective_day.as_str())
            .unwrap_or("");
        let latest_claimed = state
            .exported_days
            .keys()
            .next_back()
            .map(String::as_str)
            .unwrap_or("");
        let high_water = latest_effective.max(latest_claimed);
        ensure!(
            day_key.as_str() > high_water,
            "--update-cohort cannot apply retroactively on or before {high_water}"
        );
        state.cohort_history.push(CohortEffective {
            effective_day: day_key.clone(),
            store_ids: encoded.store_ids.clone(),
        });
    }

    state.exported_days.insert(
        day_key,
        ExportedDay {
            payload_sha256,
            payload_hex: encode_hex(&encoded.bytes),
            store_ids: encoded.store_ids.clone(),
            exported_at_ms: now_ms,
        },
    );
    // Prune claims to the 46-day rollup horizon and enforce the read budget
    // before replacing the state file so it can never outgrow private_read.
    while state.exported_days.len() > MAX_WRITER_STATE_DAYS {
        if let Some(oldest) = state.exported_days.keys().next().cloned() {
            state.exported_days.remove(&oldest);
        }
    }
    while serde_json::to_vec(&state)?.len() > MAX_WRITER_STATE_BYTES {
        ensure!(
            state.exported_days.len() > 1,
            "single wait-cache day claim exceeds writer state size bound"
        );
        if let Some(oldest) = state.exported_days.keys().next().cloned() {
            state.exported_days.remove(&oldest);
        }
    }
    atomic_json(path, &state)?;
    Ok(encoded.bytes.clone())
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

fn add_group(series: &mut Series, group: &ReportGroup) {
    let dim = &group.dimensions;
    let provider = events::provider(
        dim.get("provider")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let role = match dim.get("role").and_then(Value::as_str) {
        Some("parent" | "subagent" | "managed_child") => "interactive",
        Some("keepalive") => "keepalive",
        Some("auxiliary") => "auxiliary",
        _ => "unknown",
    };
    let outcome = events::outcome(
        dim.get("outcome")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let records = group.totals.records;
    let sum = |key: &str| group.totals.values.get(key).map(|tuple| tuple[0]);
    let mut add = |metric, provider, label, role, value: u64| {
        let total = series.entry((metric, provider, label, role)).or_default();
        *total = total.saturating_add(value);
    };
    match dim.get("event").and_then(Value::as_str) {
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
    // Keep the established last-millisecond UTC day timestamp for upgrade continuity.
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
            let group: ReportGroup = serde_json::from_value(serde_json::json!({
                "from_ms": 0,
                "to_ms": DAY_MS,
                "dimensions": {"event":events::REQUEST,"provider":"codex","role":role,"outcome":"usage"},
                "totals": {"records":1},
            }))
            .unwrap();
            add_group(&mut series, &group);
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
    fn multi_machine_encode_preserves_kind_labels_day_end_timestamp_and_strict_validation() {
        let day = 10 * DAY_MS;
        let now = day + DAY_MS + FINALIZE_GRACE_MS;
        let hawk_id = "11111111111111111111111111111111";
        let ovh_id = "22222222222222222222222222222222";
        let hawk = valid_report(
            day,
            hawk_id,
            serde_json::json!([
                {
                    "from_ms": day,
                    "to_ms": day + DAY_MS,
                    "dimensions": {"event": events::REQUEST, "provider": "codex", "role": "parent", "outcome": "usage"},
                    "totals": {"records": 10, "values": {"input_tokens": [1000, 10], "cache_read_input_tokens": [800, 10], "output_tokens": [250, 10]}}
                },
                {
                    "from_ms": day,
                    "to_ms": day + DAY_MS,
                    "dimensions": {"event": events::LOSS},
                    "totals": {"records": 2, "values": {"local_dropped_records": [3, 1], "local_write_failures": [1, 1]}}
                }
            ]),
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

        // Verify established last-ms timestamp (day + DAY_MS - 1)
        let expected_start = u64::try_from(day).unwrap() * 1_000_000;
        let expected_end = expected_start + u64::try_from(DAY_MS - 1).unwrap() * 1_000_000;

        // Verify distinct `kind` labels survive sanitization on TOKENS and LOSSES
        let tokens = metrics.iter().find(|m| m.name == TOKENS).unwrap();
        let Some(metric::Data::Sum(tok_sum)) = &tokens.data else {
            panic!("missing token sum");
        };
        let mut by_kind = BTreeMap::new();
        for p in &tok_sum.data_points {
            assert_eq!(p.start_time_unix_nano, expected_start);
            assert_eq!(p.time_unix_nano, expected_end);
            let kind = p
                .attributes
                .iter()
                .find(|a| a.key == "kind")
                .and_then(|a| a.value.as_ref())
                .and_then(|v| match &v.value {
                    Some(
                        opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s),
                    ) => Some(s.clone()),
                    _ => None,
                })
                .expect("sanitized token point must retain kind label");
            let val = match p.value {
                Some(number_data_point::Value::AsInt(v)) => v,
                _ => panic!("missing int"),
            };
            assert!(by_kind.insert(kind, val).is_none());
        }
        assert_eq!(by_kind.get("input"), Some(&1500));
        assert_eq!(by_kind.get("cache_read"), Some(&1100));
        assert_eq!(by_kind.get("output"), Some(&250));

        let losses = metrics.iter().find(|m| m.name == LOSSES).unwrap();
        let Some(metric::Data::Sum(loss_sum)) = &losses.data else {
            panic!("missing losses sum");
        };
        assert_eq!(loss_sum.data_points.len(), 2);
        assert!(
            loss_sum
                .data_points
                .iter()
                .any(|p| p.attributes.contains(&string("kind", "queue_drop")))
        );
        assert!(
            loss_sum
                .data_points
                .iter()
                .any(|p| p.attributes.contains(&string("kind", "write_failure")))
        );

        // Unfinalized midnight (inside 15m grace) and expired (>45d) both fail closed
        assert!(encode_wait_cache_otlp(day, day + DAY_MS, &[&hawk, &ovh], false).is_err());
        assert!(encode_wait_cache_otlp(day, day + 50 * DAY_MS, &[&hawk, &ovh], false).is_err());

        // Malformed group (missing records), bad tuple, and duplicate dimensions within one source fail closed
        let empty_totals = valid_report(
            day,
            ovh_id,
            serde_json::json!([{"from_ms": day, "to_ms": day + DAY_MS, "dimensions": {"event": events::REQUEST}, "totals": {}}]),
        );
        assert!(encode_wait_cache_otlp(day, now, &[&empty_totals], false).is_err());
        let dup_dim = valid_report(
            day,
            ovh_id,
            serde_json::json!([
                {"from_ms": day, "to_ms": day + DAY_MS, "dimensions": {"event": events::REQUEST, "provider": "codex"}, "totals": {"records": 1}},
                {"from_ms": day, "to_ms": day + DAY_MS, "dimensions": {"event": events::REQUEST, "provider": "codex"}, "totals": {"records": 1}}
            ]),
        );
        assert!(encode_wait_cache_otlp(day, now, &[&dup_dim], false).is_err());

        // Writer state: payload replay on retry, effective-day cohort non-retroactivity, and bare filename support
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("writer-state.json");
        let day19 = chrono::NaiveDate::from_ymd_opt(2026, 11, 19).unwrap();
        let day20 = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        let day21 = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        let committed =
            claim_or_replay_writer_state(&state_path, day19, now, &encoded, false).unwrap();
        assert_eq!(committed, encoded.bytes);

        // If source data mutates slightly after a failed stdout write, retry replays exact committed bytes
        let mutated = EncodedWaitCacheOtlp {
            bytes: vec![1, 2, 3, 4],
            store_ids: encoded.store_ids.clone(),
        };
        let replayed =
            claim_or_replay_writer_state(&state_path, day19, now, &mutated, false).unwrap();
        assert_eq!(replayed, committed);

        // Changing cohort on day21 via --update-cohort must NOT apply retroactively to unexported day20
        let hawk_only = encode_wait_cache_otlp(day, now, &[&hawk], false).unwrap();
        claim_or_replay_writer_state(&state_path, day21, now, &hawk_only, true).unwrap();
        assert!(claim_or_replay_writer_state(&state_path, day20, now, &hawk_only, false).is_err());
        assert!(claim_or_replay_writer_state(&state_path, day20, now, &hawk_only, true).is_err());
        // Original cohort is still valid for backfilling unexported day20
        claim_or_replay_writer_state(&state_path, day20, now, &encoded, false).unwrap();
        // Empty dimensions and [0, 0] zero-sample tuples both fail closed
        let empty_dim = valid_report(
            day,
            ovh_id,
            serde_json::json!([{"from_ms": day, "to_ms": day + DAY_MS, "dimensions": {}, "totals": {"records": 1}}]),
        );
        assert!(encode_wait_cache_otlp(day, now, &[&empty_dim], false).is_err());
        let zero_tuple = valid_report(
            day,
            ovh_id,
            serde_json::json!([{
                "from_ms": day,
                "to_ms": day + DAY_MS,
                "dimensions": {"event": events::REQUEST, "provider": "codex", "role": "parent", "outcome": "usage"},
                "totals": {"records": 1, "values": {"input_tokens": [0, 0]}}
            }]),
        );
        assert!(encode_wait_cache_otlp(day, now, &[&zero_tuple], false).is_err());

        // State-only replay returns exact committed bytes without live source RPCs
        assert_eq!(
            try_replay_claimed_writer_state(&state_path, day19).unwrap(),
            Some(committed.clone())
        );
        assert_eq!(
            try_replay_claimed_writer_state(
                &state_path,
                chrono::NaiveDate::from_ymd_opt(2026, 11, 25).unwrap()
            )
            .unwrap(),
            None
        );

        // Backdating --update-cohort between already-claimed day19 and day21 fails closed
        let state_path_bd = dir.path().join("writer-state-backdate.json");
        claim_or_replay_writer_state(&state_path_bd, day19, now, &encoded, false).unwrap();
        claim_or_replay_writer_state(&state_path_bd, day21, now, &encoded, false).unwrap();
        assert!(
            claim_or_replay_writer_state(&state_path_bd, day20, now, &hawk_only, true).is_err()
        );

        // Bare filename resolves parent "." for lock path
        assert_eq!(
            lock_path_for(Path::new("state.json")).unwrap(),
            PathBuf::from("./.state.json.lock")
        );
    }
}
