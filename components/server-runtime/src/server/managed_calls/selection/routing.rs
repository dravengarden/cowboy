//! Review and analysis routing from `models/routing.json` (docs/review-routing.md):
//! which reviewer preset a review gets automatically (with dynamic quota-pace
//! priority switching when configured on `high`), which explicit presets a
//! routine review may name, and which cheap preset an unnamed analysis or
//! context-ask call defaults to. Rows match resolved model values, never preset
//! IDs, because one signed ID can resolve to different models per generation.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    schema: u16,
    pub evidence: String,
    #[allow(dead_code, reason = "documentation for table editors")]
    note: String,
    providers: BTreeMap<String, Tiers>,
    costs: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tiers {
    #[serde(default)]
    high: Option<Tier>,
    #[serde(default)]
    routine: Option<Tier>,
    #[serde(default)]
    economy: Option<Tier>,
    #[serde(default)]
    analysis: Option<AnalysisTiers>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tier {
    auto: Vec<Row>,
    #[serde(default)]
    allow: Vec<Row>,
    #[serde(default)]
    quota_ample: Option<QuotaAmpleTier>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuotaAmpleTier {
    min_remaining_percent: f64,
    comfort_remaining_percent: f64,
    auto: Vec<Row>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnalysisTiers {
    auto: Vec<Row>,
    #[serde(default)]
    context_ask: Vec<Row>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    family: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    efforts: Vec<String>,
}

pub static TABLE: LazyLock<Option<Table>> = LazyLock::new(|| {
    let table: Table = serde_json::from_str(include_str!("../../../../../../models/routing.json"))
        .inspect_err(|error| tracing::error!(%error, "review routing table is invalid"))
        .ok()?;
    (table.schema == 1).then_some(table)
});

/// Outcome of evaluating cached quota headroom relative to reset time.
#[derive(Clone, Debug, PartialEq)]
pub struct QuotaAssessment {
    pub ample: bool,
    pub mode: &'static str,
    pub reason: &'static str,
    pub min_remaining_percent: Option<f64>,
    pub remaining_time_percent: Option<f64>,
}

impl QuotaAssessment {
    fn conserve(
        reason: &'static str,
        min_remaining_percent: Option<f64>,
        remaining_time_percent: Option<f64>,
    ) -> Self {
        Self {
            ample: false,
            mode: "conserve",
            reason,
            min_remaining_percent,
            remaining_time_percent,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "mode": self.mode,
            "reason": self.reason,
            "min_remaining_percent": self.min_remaining_percent,
            "remaining_time_percent": self.remaining_time_percent,
        })
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn collect_quota_windows(limits: &Value) -> Vec<&Value> {
    let mut windows = Vec::new();
    if let Some(by_id) = limits.get("rateLimitsByLimitId").and_then(Value::as_object) {
        let mut keys: Vec<&String> = by_id.keys().collect();
        keys.sort();
        for key in keys {
            let Some(detail) = by_id.get(key) else {
                continue;
            };
            for slot in ["primary", "secondary"] {
                if let Some(window) = detail.get(slot).filter(|v| v.is_object()) {
                    windows.push(window);
                }
            }
        }
    } else {
        for slot in ["primary", "secondary"] {
            if let Some(window) = limits.get(slot).filter(|v| v.is_object()) {
                windows.push(window);
            }
        }
    }
    windows
}

fn evaluate_quota(quota: Option<&Value>, now_ms: i64, policy: &QuotaAmpleTier) -> QuotaAssessment {
    let Some(quota) = quota else {
        return QuotaAssessment::conserve("quota_unavailable", None, None);
    };
    if quota["status"].as_str() != Some("available") {
        return QuotaAssessment::conserve("quota_unavailable", None, None);
    }
    if quota["stale"].as_bool().unwrap_or(true) {
        return QuotaAssessment::conserve("quota_stale", None, None);
    }
    let Some(limits) = quota.get("limits").filter(|v| v.is_object()) else {
        return QuotaAssessment::conserve("limits_missing", None, None);
    };
    let now_secs = (now_ms as f64) / 1000.0;
    let mut saw_window = false;
    let mut all_ample = true;
    let mut tightest_remaining: Option<f64> = None;
    let mut tightest_time: Option<f64> = None;
    let mut failure_reason = "behind_pace";

    for window in collect_quota_windows(limits) {
        let Some(used) = window.get("usedPercent").and_then(Value::as_f64) else {
            continue;
        };
        saw_window = true;
        let remaining = (100.0 - used).clamp(0.0, 100.0);
        let window_mins = window
            .get("windowDurationMins")
            .and_then(Value::as_f64)
            .filter(|&m| m > 0.0);
        let resets_at_secs = window
            .get("resetsAt")
            .and_then(Value::as_f64)
            .filter(|&t| t > 0.0)
            .map(|t| if t > 1e12 { t / 1000.0 } else { t });
        let remaining_time = match (window_mins, resets_at_secs) {
            (Some(mins), Some(reset)) if now_secs > 0.0 => {
                let left_secs = (reset - now_secs).max(0.0);
                Some((left_secs / (mins * 60.0) * 100.0).clamp(0.0, 100.0))
            }
            _ => None,
        };
        let meets_floor = remaining >= policy.min_remaining_percent;
        let meets_pace = remaining >= policy.comfort_remaining_percent
            || remaining_time.is_some_and(|time_left| remaining >= time_left);
        if !meets_floor || !meets_pace {
            if all_ample || !meets_floor {
                failure_reason = if !meets_floor {
                    "below_safety_floor"
                } else {
                    "behind_pace"
                };
            }
            all_ample = false;
        }
        if tightest_remaining.is_none_or(|prev| remaining < prev) {
            tightest_remaining = Some(round1(remaining));
            tightest_time = remaining_time.map(round1);
        }
    }

    if !saw_window {
        return QuotaAssessment::conserve("no_windows", None, None);
    }
    if all_ample {
        QuotaAssessment {
            ample: true,
            mode: "ample",
            reason: "headroom_ok",
            min_remaining_percent: tightest_remaining,
            remaining_time_percent: tightest_time,
        }
    } else {
        QuotaAssessment::conserve(failure_reason, tightest_remaining, tightest_time)
    }
}

/// Evaluate the `high` tier's dynamic quota headroom policy for `provider`,
/// when `models/routing.json` declares `quota_ample` for that provider.
pub fn high_quota_assessment(
    table: &Table,
    provider: &str,
    quota: Option<&Value>,
    now_ms: i64,
) -> Option<QuotaAssessment> {
    let policy = table
        .providers
        .get(provider)?
        .high
        .as_ref()?
        .quota_ample
        .as_ref()?;
    Some(evaluate_quota(quota, now_ms, policy))
}

/// The model a preset resolves to, without a context suffix such as `[1m]`.
pub fn model(preset: &Value) -> Option<&str> {
    let model = preset["values"]["model"].as_str()?;
    Some(model.split_once('[').map_or(model, |(base, _)| base))
}

pub fn effort(preset: &Value) -> &str {
    preset["values"]["reasoning_effort"]
        .as_str()
        .or(preset["values"]["effort"].as_str())
        .or(preset["values"]["thought_level"].as_str())
        .unwrap_or("default")
}

/// Model family vocabulary for `analysis` / `context_ask` routing. Kept apart
/// from `selection::family` so non-reviewer models never enter reviewer pools.
fn analysis_family(model: &str) -> Option<&'static str> {
    if let Some(reviewer) = super::family(model) {
        return Some(reviewer);
    }
    let base = model.split_once('[').map_or(model, |(base, _)| base);
    if base == "sonnet" || base.starts_with("claude-sonnet-") {
        Some("sonnet")
    } else if base.starts_with("gpt-") && base.ends_with("-luna") {
        Some("luna")
    } else if base.starts_with("gemini-") && base.ends_with("-hydrogen") {
        Some("gemini-hydrogen")
    } else if base.starts_with("gemini-") && base.ends_with("-neon") {
        Some("gemini-neon")
    } else if base.starts_with("deepseek-") {
        Some("deepseek")
    } else if base.starts_with("grok-") {
        Some("grok")
    } else {
        None
    }
}

impl Row {
    fn matches_with(&self, preset: &Value, family_of: fn(&str) -> Option<&'static str>) -> bool {
        let Some(model) = model(preset) else {
            return false;
        };
        family_of(model) == Some(self.family.as_str())
            && self
                .model
                .as_deref()
                .is_none_or(|expected| expected == model)
            && (self.efforts.is_empty() || self.efforts.iter().any(|e| e == effort(preset)))
    }

    fn matches(&self, preset: &Value) -> bool {
        self.matches_with(preset, super::family)
    }

    fn matches_analysis(&self, preset: &Value) -> bool {
        self.matches_with(preset, analysis_family)
    }
}

/// Which tier a fresh review resolves to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    High,
    Routine,
    Economy,
}

/// The automatic preset first, then every other preset the tier admits, in
/// `pool` (strongest-first) order. `None` when the table has no review rows
/// for this Provider or generation; the caller then keeps the legacy order.
pub fn order(
    table: &Table,
    provider: &str,
    level: Level,
    pool: &[Value],
    coverage: &[Value],
    quota: Option<&Value>,
    now_ms: i64,
) -> Option<Ranked> {
    let tiers = table.providers.get(provider)?;
    let high = tiers.high.as_ref()?;
    let routine = tiers.routine.as_ref()?;
    let tier = match level {
        Level::High => high,
        Level::Routine => routine,
        Level::Economy => tiers.economy.as_ref().unwrap_or(routine),
    };
    let auto_rows: &[Row] = match (level, tier.quota_ample.as_ref()) {
        (Level::High, Some(qa)) if evaluate_quota(quota, now_ms, qa).ample => &qa.auto,
        _ => &tier.auto,
    };
    let bounds = if level == Level::High {
        None
    } else {
        Some(routine)
    };
    let admitted = |preset: &&Value| {
        bounds.is_none_or(|routine| {
            routine.allow.is_empty()
                || routine
                    .allow
                    .iter()
                    .chain(&routine.auto)
                    .chain(auto_rows.iter())
                    .any(|row| row.matches(preset))
        })
    };
    let rows = || {
        [high, routine]
            .into_iter()
            .chain(tiers.economy.as_ref())
            .flat_map(|tier| {
                tier.auto
                    .iter()
                    .chain(&tier.allow)
                    .chain(tier.quota_ample.iter().flat_map(|qa| qa.auto.iter()))
            })
    };
    // Rows naming an exact model bind the table to known generations; an
    // unknown generation keeps the legacy order.
    // Coverage is by model identity over the whole runnable generation, so
    // excluding the caller's model, a family or an effort never turns a known
    // generation into an unknown one.
    let known = !rows().any(|row| row.model.is_some())
        || rows().any(|row| {
            row.model.as_deref().is_some_and(|expected| {
                coverage
                    .iter()
                    .any(|preset| model(preset) == Some(expected))
            })
        });
    if !known {
        return None;
    }
    // The tier's automatic row, else (for example when the caller's own model
    // was excluded) the strongest preset the tier still admits.
    let automatic = auto_rows
        .iter()
        .find_map(|row| pool.iter().find(|preset| row.matches(preset)))
        // Never for the high tier: its explicit-only reviewers (Fable) need
        // the caller to name them.
        .or_else(|| {
            (level != Level::High)
                .then(|| pool.iter().find(|preset| admitted(preset)))
                .flatten()
        });
    let mut ordered: Vec<Value> = automatic.into_iter().cloned().collect();
    ordered.extend(
        pool.iter()
            .filter(admitted)
            .filter(|preset| automatic.is_none_or(|chosen| preset["id"] != chosen["id"]))
            .cloned(),
    );
    Some(Ranked {
        list: ordered,
        automatic: automatic.is_some(),
    })
}

/// First runnable preset matching `provider`'s `analysis` (or `context_ask`)
/// routing rows. Returns `None` when the table does not cover this Provider or
/// generation, leaving the Provider's own default untouched.
pub fn analysis_choice(
    table: &Table,
    provider: &str,
    runnable: &Value,
    context_ask: bool,
) -> Option<Value> {
    let analysis = table.providers.get(provider)?.analysis.as_ref()?;
    let pool = runnable.as_array()?;
    let rows = if context_ask && !analysis.context_ask.is_empty() {
        analysis.context_ask.iter().chain(&analysis.auto)
    } else {
        analysis.auto.iter().chain(&analysis.context_ask)
    };
    rows.clone()
        .find_map(|row| {
            if row.efforts.is_empty() {
                pool.iter().find(|preset| row.matches_analysis(preset))
            } else {
                row.efforts.iter().find_map(|wanted| {
                    pool.iter()
                        .find(|preset| effort(preset) == wanted && row.matches_analysis(preset))
                })
            }
        })
        .cloned()
}

/// Capabilities projection of `provider`'s automatic analysis / context-ask
/// choices with measured costs, or `null` when uncovered.
pub fn analysis_view(table: &Table, provider: &str, runnable: &Value) -> Value {
    if table
        .providers
        .get(provider)
        .and_then(|t| t.analysis.as_ref())
        .is_none()
    {
        return Value::Null;
    }
    let entry = |preset: Value| json!({"preset": preset["id"], "cost": cost(table, &preset)});
    json!({
        "evidence": table.evidence,
        "automatic": analysis_choice(table, provider, runnable, false).map(entry),
        "context_ask": analysis_choice(table, provider, runnable, true).map(entry),
    })
}

/// A tier's presets, the automatic choice first when one exists. Without
/// one, `list` holds only what a caller may name explicitly.
pub(in crate::server::managed_calls) struct Ranked {
    pub(in crate::server::managed_calls) list: Vec<Value>,
    pub(in crate::server::managed_calls) automatic: bool,
}

/// Measured cost of a preset's resolved model and effort, or `unknown`.
pub fn cost(table: &Table, preset: &Value) -> Value {
    model(preset)
        .and_then(|model| table.costs.get(&format!("{model}:{}", effort(preset))))
        .cloned()
        .unwrap_or_else(|| json!("unknown"))
}
