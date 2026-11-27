//! Indexed turn-level prompt-cache telemetry projection (`turn_cache_facts`).
//!
//! Each completed `Event::TurnEnd` is projected into `turn_cache_facts` inside
//! the event-batch transaction, preserved transactionally before `Clear` or
//! session purge, and reconciled in bounded keyset batches on startup so
//! historical turns (including turns written during a Controller rollback) are
//! filled without ever overwriting live-captured turn-time attribution.
//!
//! Token accounting invariant: `TurnUsage.uncached_input_tokens` equals
//! `input_tokens - cached_input_tokens` and therefore already includes
//! `cache_write_tokens` (e.g. Claude Messages). Normalization stores disjoint
//! partitions (`cached_read_tokens`, `cache_write_tokens`,
//! `other_uncached_input_tokens`) and preserves `None` for unreported fields
//! instead of coercing unknown values to zero.

mod queries;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent_model::TurnUsage;

pub(in crate::store) use queries::{
    project_events_pg, project_events_sqlite, project_missing_for_session_pg,
    project_missing_for_session_sqlite,
};

/// Default retention window for `turn_cache_facts` rows (days).
pub const DEFAULT_RETENTION_DAYS: i64 = 90;
/// Maximum safe integer (JS `Number.MAX_SAFE_INTEGER`) matching `TurnUsage`.
pub(super) const MAX_SAFE_TOKENS: u64 = 9_007_199_254_740_991;
const MAX_MODEL_BYTES: usize = 120;
const MAX_PROMPT_ORIGIN_BYTES: usize = 32;
const MAX_VERSION_BYTES: usize = 128;
const MAX_MODELS_IN_RESPONSE: i64 = 16;
const MAX_RECENT_TURNS: usize = 100;

/// Disjoint token breakdown for one turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizedTurnCacheUsage {
    pub total_tokens: i64,
    pub inclusive_input_tokens: i64,
    pub cached_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub other_uncached_input_tokens: Option<i64>,
    pub output_tokens: i64,
    pub reasoning_tokens: Option<i64>,
}

/// Normalize one `TurnUsage` into disjoint, self-consistent token counters.
///
/// Returns `None` if input is absent or fails accounting invariants (e.g.
/// `cached_read > inclusive_input`, `cache_write > uncached_total`, or
/// inconsistent `uncached_input_tokens`). Missing cache fields stay `None`.
#[must_use]
pub fn normalize_turn_usage(usage: Option<&TurnUsage>) -> Option<NormalizedTurnCacheUsage> {
    let usage = usage?;
    if usage.total_tokens > MAX_SAFE_TOKENS
        || usage.input_tokens > MAX_SAFE_TOKENS
        || usage.output_tokens > MAX_SAFE_TOKENS
    {
        return None;
    }
    if usage.input_tokens.checked_add(usage.output_tokens) != Some(usage.total_tokens) {
        return None;
    }
    let inclusive_input = usage.input_tokens;
    let cached_read = usage.cached_input_tokens;
    if cached_read.is_some_and(|read| read > inclusive_input) {
        return None;
    }
    let derived_uncached = cached_read
        .map(|read| inclusive_input - read)
        .or(usage.uncached_input_tokens);
    if let (Some(derived), Some(raw)) = (derived_uncached, usage.uncached_input_tokens)
        && derived != raw
    {
        return None;
    }
    if derived_uncached.is_some_and(|uncached| uncached > inclusive_input) {
        return None;
    }
    let cache_write = usage.cache_write_tokens;
    if let (Some(write), Some(uncached)) = (cache_write, derived_uncached)
        && write > uncached
    {
        return None;
    }
    if cache_write.is_some_and(|write| write > inclusive_input) {
        return None;
    }
    if usage
        .reasoning_tokens
        .is_some_and(|reasoning| reasoning > usage.output_tokens)
    {
        return None;
    }
    let other_uncached = match (derived_uncached, cache_write) {
        (Some(uncached), Some(write)) => Some(uncached - write),
        _ => None,
    };

    Some(NormalizedTurnCacheUsage {
        total_tokens: i64::try_from(usage.total_tokens).ok()?,
        inclusive_input_tokens: i64::try_from(inclusive_input).ok()?,
        cached_read_tokens: cached_read.map(i64::try_from).transpose().ok()?,
        cache_write_tokens: cache_write.map(i64::try_from).transpose().ok()?,
        other_uncached_input_tokens: other_uncached.map(i64::try_from).transpose().ok()?,
        output_tokens: i64::try_from(usage.output_tokens).ok()?,
        reasoning_tokens: usage.reasoning_tokens.map(i64::try_from).transpose().ok()?,
    })
}

pub(super) fn sanitize_text(input: &str, max_bytes: usize) -> String {
    let cleaned = if input.contains('\0') {
        input.replace('\0', "")
    } else {
        input.to_owned()
    };
    let trimmed = cleaned.trim();
    if trimmed.len() <= max_bytes {
        return trimmed.to_owned();
    }
    let mut cut = max_bytes;
    while cut > 0 && !trimmed.is_char_boundary(cut) {
        cut -= 1;
    }
    trimmed[..cut].to_owned()
}

/// Normalize arbitrary stop-reason strings (including multi-line provider error
/// payloads or strings containing embedded NUL bytes) onto a closed, safe label
/// set so malformed runtime stop reasons can never abort PostgreSQL writes.
#[must_use]
pub fn normalize_stop_reason(raw: &str) -> &'static str {
    let cleaned = if raw.contains('\0') {
        raw.replace('\0', "")
    } else {
        raw.to_owned()
    };
    let trimmed = cleaned.trim();
    if trimmed.eq_ignore_ascii_case("endturn") || trimmed.eq_ignore_ascii_case("end_turn") {
        "end_turn"
    } else if trimmed.eq_ignore_ascii_case("cancelled") || trimmed.eq_ignore_ascii_case("canceled")
    {
        "cancelled"
    } else if trimmed.eq_ignore_ascii_case("maxtokens")
        || trimmed.eq_ignore_ascii_case("max_tokens")
    {
        "max_tokens"
    } else if trimmed.eq_ignore_ascii_case("maxturnrequests")
        || trimmed.eq_ignore_ascii_case("max_turn_requests")
    {
        "max_turn_requests"
    } else if trimmed.eq_ignore_ascii_case("refusal") {
        "refusal"
    } else if trimmed.eq_ignore_ascii_case("error") || trimmed.starts_with("error:") {
        "error"
    } else if trimmed.is_empty() {
        "unknown"
    } else {
        "other"
    }
}

fn model_identifiers_compatible(selected: &str, observed: &str) -> bool {
    let sel = selected.trim().to_ascii_lowercase();
    let obs = observed.trim().to_ascii_lowercase();
    if sel.is_empty() || obs.is_empty() {
        return false;
    }
    if sel == obs {
        return true;
    }
    let contains_token = |haystack: &str, needle: &str| -> bool {
        haystack
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|tok| tok == needle)
    };
    contains_token(&obs, &sel)
        || contains_token(&sel, &obs)
        || obs.starts_with(&format!("{sel}-"))
        || obs.starts_with(&format!("{sel}["))
        || sel.starts_with(&format!("{obs}-"))
}

fn extract_selected_model(
    config_preferences: Option<&Value>,
    config_options: Option<&Value>,
) -> Option<String> {
    if let Some(model) = config_preferences
        .and_then(|value| value.get("model"))
        .and_then(Value::as_str)
        .map(normalize_model_value)
        .filter(|value| !value.is_empty())
    {
        return Some(model);
    }
    let options = config_options.and_then(Value::as_array)?;
    for option in options {
        if option.get("id").and_then(Value::as_str) != Some("model") {
            continue;
        }
        if let Some(current) = option.get("currentValue").and_then(Value::as_str) {
            let normalized = normalize_model_value(current);
            if !normalized.is_empty() {
                return Some(normalized);
            }
        }
    }
    None
}

/// Extract the best unambiguous live turn-time model identifier from session
/// metadata.
///
/// `last_usage.raw._cowboy.modelUsage` is cumulative and flushed asynchronously
/// by the write-behind worker, so on the first turn after a model switch A -> B
/// `modelUsage` may still contain only `[A]` while `config_preferences` holds
/// `B` (or vice-versa right after a switch). Whenever cumulative `modelUsage`
/// conflicts with the active selected model (or has multiple models without a
/// unique match for the selected model), return `None` (`unknown`) instead of
/// misattributing the turn to the wrong model.
#[must_use]
pub fn extract_session_model(
    last_usage: Option<&Value>,
    config_preferences: Option<&Value>,
    config_options: Option<&Value>,
) -> Option<String> {
    let selected = extract_selected_model(config_preferences, config_options);
    let usage_models: Vec<String> = last_usage
        .and_then(|value| value.pointer("/raw/_cowboy/modelUsage"))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    row.get("model")
                        .and_then(Value::as_str)
                        .map(|value| sanitize_text(value, MAX_MODEL_BYTES))
                        .filter(|value| !value.is_empty())
                })
                .collect()
        })
        .unwrap_or_default();

    match usage_models.as_slice() {
        [] => selected,
        [only] => match selected.as_deref() {
            None => Some(only.clone()),
            Some(sel) if model_identifiers_compatible(sel, only) => Some(only.clone()),
            Some(_) => None,
        },
        many => {
            let sel = selected.as_deref()?;
            let mut matches = many
                .iter()
                .filter(|observed| model_identifiers_compatible(sel, observed));
            let first = matches.next()?;
            if matches.next().is_some() {
                return None;
            }
            Some(first.clone())
        }
    }
}

fn normalize_model_value(raw: &str) -> String {
    let cleaned = sanitize_text(raw, MAX_MODEL_BYTES * 2);
    if cleaned.starts_with('[')
        && let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&cleaned)
        && let Some(last) = items.last().and_then(Value::as_str)
    {
        return sanitize_text(last, MAX_MODEL_BYTES);
    }
    sanitize_text(&cleaned, MAX_MODEL_BYTES)
}

pub(super) fn classify_prompt_origin(payload: &Value) -> String {
    let Some(update) = payload.get("update") else {
        return "unknown".to_owned();
    };
    if let Some(actor) = update
        .pointer("/promptOrigin/actor")
        .and_then(Value::as_str)
        .map(|value| sanitize_text(&value.to_ascii_lowercase(), MAX_PROMPT_ORIGIN_BYTES))
        .filter(|value| !value.is_empty())
    {
        return actor;
    }
    if update.get("autoResumed").and_then(Value::as_bool) == Some(true) {
        return "auto_resume".to_owned();
    }
    if update
        .pointer("/content/text")
        .and_then(Value::as_str)
        .is_some_and(|text| text.starts_with("<system-reminder>"))
    {
        return "system_reminder".to_owned();
    }
    "human".to_owned()
}

/// Aggregated turn-level cache efficiency totals over a window or session.
///
/// Token sums accumulate in checked 128-bit arithmetic (split into 30-bit SQL
/// limbs so neither SQLite `SUM` nor PostgreSQL `BIGINT` can overflow) and
/// clamp to `MAX_SAFE_TOKENS` (`2^53 - 1`) for exact JavaScript JSON numbers;
/// `totals_saturated` is `true` if any token sum exceeded `2^53 - 1` (ratios
/// are always computed from full 128-bit sums before clamping).
///
/// Token sums stay `None` when no contributing turn reported valid usage/cache
/// counters, preserving `Some(0)` strictly for genuinely observed zero-token
/// turns. `observed_cache_write_turns` and `cache_write_tokens` track write
/// telemetry independently of `cached_read_tokens`, while
/// `other_uncached_input_tokens` is populated only when every observed-cache
/// turn has a complete disjoint `cached_read + cache_write + other_uncached`
/// breakdown and no unpaired write-only turn skews the cohort.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CacheMetricsTotals {
    pub total_turns: u64,
    pub eligible_turns: u64,
    pub observed_usage_turns: u64,
    pub observed_cache_turns: u64,
    pub observed_cache_write_turns: u64,
    pub renewed_turns: u64,
    pub compacted_turns: u64,
    pub inclusive_input_tokens: Option<u64>,
    pub observed_cache_input_tokens: Option<u64>,
    pub cached_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub uncached_input_tokens: Option<u64>,
    pub other_uncached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    /// `sum(cached_read_tokens) / sum(observed_cache_input_tokens)` strictly
    /// over turns that reported both inclusive input and cached read counters.
    pub cached_input_share: Option<f64>,
    /// `sum(uncached_input_tokens) / sum(observed_cache_input_tokens)`.
    pub uncached_input_share: Option<f64>,
    /// `true` when at least one token sum exceeded JS `Number.MAX_SAFE_INTEGER`
    /// (`2^53 - 1`) and was clamped for exact JSON serialization.
    #[serde(default)]
    pub totals_saturated: bool,
}

/// Per-model breakdown inside an account or session cache metrics response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelCacheMetrics {
    pub model: String,
    pub totals: CacheMetricsTotals,
}

/// Scoped coverage metadata accompanying every authorized cache response.
///
/// Timestamps are computed strictly from the authorized account or session's
/// own projected facts; installation-wide backfill counters are never exposed
/// on scoped endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheMetricsCoverage {
    pub accounting_source: String,
    pub granularity: String,
    pub backfill_complete: bool,
    pub oldest_fact_ms: Option<i64>,
    pub newest_fact_ms: Option<i64>,
    pub usage_coverage_ratio: Option<f64>,
    pub cache_coverage_ratio: Option<f64>,
}

/// One projected turn row exposed for session-level diagnostic disclosure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnCacheFact {
    pub turn_end_seq: u64,
    pub occurred_at_ms: i64,
    pub model: Option<String>,
    pub stop_reason: String,
    pub prompt_origin: String,
    pub idle_gap_ms: Option<i64>,
    pub renewed: bool,
    pub compacted: bool,
    pub usage_observed: bool,
    pub inclusive_input_tokens: Option<u64>,
    pub cached_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub uncached_input_tokens: Option<u64>,
    pub other_uncached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cached_input_share: Option<f64>,
    pub accounting_source: String,
}

/// Account-scoped cache efficiency read model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccountCacheMetrics {
    pub provider: String,
    pub provider_account_id: String,
    pub window: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub totals: CacheMetricsTotals,
    pub by_model: Vec<ModelCacheMetrics>,
    pub coverage: CacheMetricsCoverage,
}

/// Session-scoped cache efficiency read model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionCacheMetrics {
    pub session_id: String,
    pub provider: String,
    pub provider_account_id: String,
    pub totals: CacheMetricsTotals,
    pub by_model: Vec<ModelCacheMetrics>,
    pub recent_turns: Vec<TurnCacheFact>,
    pub coverage: CacheMetricsCoverage,
}

/// Result of one bounded background backfill/reconciliation batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillProgress {
    pub scanned_events: usize,
    pub rows_projected: usize,
    pub completed: bool,
    pub cursor_session_id: String,
    pub cursor_seq: i64,
}
