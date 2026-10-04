use super::data::{Entry, Metrics, Series};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(crate) const BUCKETS: &[f64] = &[
    1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0, 30000.0,
    60000.0,
];

pub(crate) fn accumulate(groups: &mut BTreeMap<String, Series>, entry: &Entry) {
    let mut key = format!("{}/{}/{}", entry.service, entry.signal, entry.event);
    if groups.len() >= 128 && !groups.contains_key(&key) {
        key = "__other_groups__".into();
    }
    let series = groups.entry(key).or_insert_with(|| Series {
        duration_buckets: vec![0; BUCKETS.len() + 1],
        ..Default::default()
    });
    series.count += 1;
    series.errors += u64::from(entry.severity >= 17);
    series.warnings += u64::from((13..17).contains(&entry.severity));
    if let Some(ms) = entry.duration_ms {
        series.duration_count += 1;
        series.duration_sum_ms += ms;
        series.duration_max_ms = series.duration_max_ms.max(ms);
        let bucket = BUCKETS
            .iter()
            .position(|upper| ms <= *upper)
            .unwrap_or(BUCKETS.len());
        series.duration_buckets[bucket] += 1;
    }
    if series.evidence_ids.len() < 4 {
        series.evidence_ids.push(entry.id.clone());
    }
    if entry.severity >= 13 && series.failure_evidence_ids.len() < 4 {
        series.failure_evidence_ids.push(entry.id.clone());
    }
}

fn p95(series: &Series) -> Option<f64> {
    if series.duration_count == 0 {
        return None;
    }
    let threshold = (series.duration_count * 95).div_ceil(100);
    let mut sum = 0;
    for (i, count) in series.duration_buckets.iter().enumerate() {
        sum += count;
        if sum >= threshold {
            return Some(BUCKETS.get(i).copied().unwrap_or(series.duration_max_ms));
        }
    }
    None
}

pub(crate) fn analyze(current: Metrics, baseline: Option<Metrics>) -> Value {
    let mut findings = Vec::new();
    let mut comparisons = Vec::new();
    for (key, series) in &current.groups {
        let old = baseline.as_ref().and_then(|b| b.groups.get(key));
        let fraction = series.errors as f64 / series.count.max(1) as f64;
        let mean = if series.duration_count > 0 {
            Some(series.duration_sum_ms / series.duration_count as f64)
        } else {
            None
        };
        let old_mean = old
            .filter(|s| s.duration_count > 0)
            .map(|s| s.duration_sum_ms / s.duration_count as f64);
        comparisons.push(json!({"group":key,"observed_events":series.count,"events_per_second":series.count as f64/((current.to_ms-current.from_ms) as f64/1000.0),"observed_error_fraction":fraction,"mean_duration_ms":mean,"p95_histogram_upper_bound_ms":p95(series),"baseline_mean_duration_ms":old_mean,"baseline_count":old.map(|s|s.count)}));
        if series.errors > 0 || series.warnings > 0 {
            let hint = if key.contains("cursor_expired") {
                "Inspect the worker gap, keeper retention/backpressure and native process history; do not replay an uncertain effect."
            } else if key.contains("backpressure") {
                "Compare output bytes and consumer delay; check whether pressure clears. Pressure alone is not executor death."
            } else if key.contains("timeout") || key.contains("disconnected") {
                "Correlate the same session/environment on runtime and execution Machines; compare reconnect, keeper exit and transport evidence."
            } else {
                "Query the referenced records and their session/trace. The event count is evidence, not a root-cause conclusion."
            };
            findings.push(json!({"kind":if old.is_none(){"new_failure_group"}else{"observed_failure_group"},"group":key,"errors":series.errors,"warnings":series.warnings,"evidence_ids":series.failure_evidence_ids,"next_step":hint}));
        }
        if let (Some(mean), Some(old_mean), Some(old)) = (mean, old_mean, old)
            && key != "__other_groups__"
            && !current.coverage.truncated
            && current.coverage.issues.is_empty()
            && baseline
                .as_ref()
                .is_some_and(|b| !b.coverage.truncated && b.coverage.issues.is_empty())
            && series.duration_count >= 20
            && old.duration_count >= 20
            && mean > old_mean * 1.5
            && mean - old_mean > 10.0
        {
            findings.push(json!({"kind":"duration_regression_candidate","group":key,"ratio":mean/old_mean.max(0.001),"evidence_ids":series.evidence_ids,"next_step":"Compare workload and release generation before attributing this change to a regression."}));
        }
    }
    let incomplete = current.coverage.truncated
        || !current.coverage.issues.is_empty()
        || baseline
            .as_ref()
            .is_some_and(|b| b.coverage.truncated || !b.coverage.issues.is_empty());
    json!({"schema":"cowboy.logs.analysis/v1","status":if current.groups.is_empty(){"no_evidence"}else if incomplete{"incomplete_evidence"}else if findings.is_empty(){"no_findings_in_observed_data"}else{"findings"},"findings":findings,"comparisons":comparisons,"current":current,"baseline":baseline,"limits":"Deterministic evidence analysis; no model calls, no automatic remediation, no inference of health from missing logs. Counts describe retained observations, not all requests."})
}
