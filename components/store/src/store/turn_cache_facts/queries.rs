//! SQLite and PostgreSQL projection, pre-delete preservation, rollback-safe
//! reconciliation, retention, and SQL-aggregated query paths for
//! `turn_cache_facts`.

use anyhow::{Context as _, Result};
use serde_json::Value;
use sqlx::Row as _;

use super::{
    AccountCacheMetrics, BackfillProgress, CacheMetricsCoverage, CacheMetricsTotals,
    DEFAULT_RETENTION_DAYS, MAX_MODELS_IN_RESPONSE, MAX_RECENT_TURNS, MAX_SAFE_TOKENS,
    MAX_VERSION_BYTES, ModelCacheMetrics, SessionCacheMetrics, TurnCacheFact,
    classify_prompt_origin, extract_session_model, normalize_stop_reason, normalize_turn_usage,
    sanitize_text,
};
use crate::agent_model::{Event, TurnUsage};
use crate::model::Envelope;
use crate::store::{PostgresStorage, SqliteStorage, StorageBackend, Store};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectionMode {
    /// Live `TurnEnd` inside `upsert_event_batch`: captures live session
    /// version/generation/model and updates on batch retry.
    Live,
    /// Historical backfill or pre-Clear/pre-Purge preservation: inserts only
    /// missing keys (`ON CONFLICT DO NOTHING`) and leaves unverifiable
    /// historical version/generation/model attribution unknown (`""` / `NULL`).
    BackfillMissing,
}

#[derive(Debug, Clone)]
struct BackfillState {
    cursor_session_id: String,
    cursor_seq: i64,
    completed_at_ms: Option<i64>,
}

impl Default for BackfillState {
    fn default() -> Self {
        Self {
            cursor_session_id: String::new(),
            cursor_seq: -1,
            completed_at_ms: None,
        }
    }
}

#[derive(Debug)]
struct SessionSnapshot {
    owner_user_id: Option<String>,
    provider: String,
    provider_account_id: String,
    provider_version: String,
    provider_generation_digest: String,
    model: Option<String>,
}

/// Base for splitting non-negative `i64` token counters into two SQL `SUM`
/// limbs (`2^30 = 1_073_741_824`) so neither SQLite's 64-bit signed `SUM` nor
/// PostgreSQL's `BIGINT` cast can overflow even across millions of max-sized
/// (`2^53 - 1`) turns.
const TOKEN_LIMB_BASE: u128 = 1_073_741_824;

#[derive(Debug)]
struct AggRow {
    total_turns: i64,
    eligible_turns: i64,
    observed_usage_turns: i64,
    observed_cache_turns: i64,
    observed_cache_write_turns: i64,
    observed_complete_breakdown_turns: i64,
    observed_reasoning_turns: i64,
    renewed_turns: i64,
    compacted_turns: i64,
    inclusive_input_tokens: u128,
    observed_cache_input_tokens: u128,
    cached_read_tokens: u128,
    cache_write_tokens: u128,
    uncached_input_tokens: u128,
    other_uncached_input_tokens: u128,
    output_tokens: u128,
    reasoning_tokens: u128,
    oldest_fact_ms: Option<i64>,
    newest_fact_ms: Option<i64>,
}

fn combine_limbs(hi: i64, lo: i64) -> u128 {
    let hi_u = u128::try_from(hi).unwrap_or(0);
    let lo_u = u128::try_from(lo).unwrap_or(0);
    hi_u.saturating_mul(TOKEN_LIMB_BASE).saturating_add(lo_u)
}

fn clamp_safe_json_tokens(value: u128, saturated: &mut bool) -> u64 {
    if value > u128::from(MAX_SAFE_TOKENS) {
        *saturated = true;
        MAX_SAFE_TOKENS
    } else {
        u64::try_from(value).unwrap_or(MAX_SAFE_TOKENS)
    }
}

#[allow(clippy::cast_precision_loss)]
fn ratio(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

fn to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

#[allow(clippy::cast_precision_loss)]
fn agg_to_totals(row: &AggRow) -> CacheMetricsTotals {
    let observed_usage = to_u64(row.observed_usage_turns);
    let observed_cache = to_u64(row.observed_cache_turns);
    let observed_write = to_u64(row.observed_cache_write_turns);
    let observed_complete = to_u64(row.observed_complete_breakdown_turns);
    let observed_reasoning = to_u64(row.observed_reasoning_turns);

    // Compute shares from full 128-bit sums before safe-JSON clamping.
    let cached_input_share = if observed_cache > 0 && row.observed_cache_input_tokens > 0 {
        Some(row.cached_read_tokens as f64 / row.observed_cache_input_tokens as f64)
    } else {
        None
    };
    let uncached_input_share = if observed_cache > 0 && row.observed_cache_input_tokens > 0 {
        Some(row.uncached_input_tokens as f64 / row.observed_cache_input_tokens as f64)
    } else {
        None
    };

    let mut totals_saturated = false;
    let inclusive_input = (observed_usage > 0)
        .then(|| clamp_safe_json_tokens(row.inclusive_input_tokens, &mut totals_saturated));
    let observed_cache_input = (observed_cache > 0)
        .then(|| clamp_safe_json_tokens(row.observed_cache_input_tokens, &mut totals_saturated));
    let cached_read = (observed_cache > 0)
        .then(|| clamp_safe_json_tokens(row.cached_read_tokens, &mut totals_saturated));
    let cache_write = (observed_write > 0)
        .then(|| clamp_safe_json_tokens(row.cache_write_tokens, &mut totals_saturated));
    let uncached = (observed_cache > 0)
        .then(|| clamp_safe_json_tokens(row.uncached_input_tokens, &mut totals_saturated));
    // Only emit `other_uncached_input_tokens` when every observed-cache turn has
    // a complete disjoint breakdown AND no extra write-only turn diverges from
    // that cohort, so a 3-way stacked bar always sums to observed_cache_input.
    let other_uncached = if observed_cache > 0
        && observed_complete == observed_cache
        && observed_write == observed_cache
    {
        Some(clamp_safe_json_tokens(
            row.other_uncached_input_tokens,
            &mut totals_saturated,
        ))
    } else {
        None
    };
    let output = (observed_usage > 0)
        .then(|| clamp_safe_json_tokens(row.output_tokens, &mut totals_saturated));
    let reasoning = (observed_reasoning > 0)
        .then(|| clamp_safe_json_tokens(row.reasoning_tokens, &mut totals_saturated));

    CacheMetricsTotals {
        total_turns: to_u64(row.total_turns),
        eligible_turns: to_u64(row.eligible_turns),
        observed_usage_turns: observed_usage,
        observed_cache_turns: observed_cache,
        observed_cache_write_turns: observed_write,
        renewed_turns: to_u64(row.renewed_turns),
        compacted_turns: to_u64(row.compacted_turns),
        inclusive_input_tokens: inclusive_input,
        observed_cache_input_tokens: observed_cache_input,
        cached_read_tokens: cached_read,
        cache_write_tokens: cache_write,
        uncached_input_tokens: uncached,
        other_uncached_input_tokens: other_uncached,
        output_tokens: output,
        reasoning_tokens: reasoning,
        cached_input_share,
        uncached_input_share,
        totals_saturated,
    }
}

fn build_scoped_coverage(
    totals: &CacheMetricsTotals,
    agg: &AggRow,
    backfill: &BackfillState,
) -> CacheMetricsCoverage {
    CacheMetricsCoverage {
        accounting_source: "turn_end".to_owned(),
        granularity: "turn_total".to_owned(),
        backfill_complete: backfill.completed_at_ms.is_some(),
        oldest_fact_ms: agg.oldest_fact_ms,
        newest_fact_ms: agg.newest_fact_ms,
        usage_coverage_ratio: ratio(totals.observed_usage_turns, totals.eligible_turns),
        cache_coverage_ratio: ratio(totals.observed_cache_turns, totals.eligible_turns),
    }
}

async fn begin_write_tx_pg(pool: &sqlx::PgPool) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    pool.begin().await.context("begin PostgreSQL write tx")
}

async fn begin_write_tx_sqlite(
    pool: &sqlx::SqlitePool,
) -> Result<sqlx::Transaction<'_, sqlx::Sqlite>> {
    pool.begin_with("BEGIN IMMEDIATE")
        .await
        .context("begin SQLite immediate write tx")
}

async fn begin_read_tx_pg(pool: &sqlx::PgPool) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    pool.begin_with("BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .context("begin PostgreSQL repeatable-read snapshot tx")
}

async fn begin_read_tx_sqlite(
    pool: &sqlx::SqlitePool,
) -> Result<sqlx::Transaction<'_, sqlx::Sqlite>> {
    pool.begin_with("BEGIN DEFERRED")
        .await
        .context("begin SQLite snapshot read tx")
}

macro_rules! limb_sum {
    ($cond:expr, $expr:expr, $hi:literal, $lo:literal) => {
        concat!(
            "CAST(COALESCE(SUM(CASE WHEN ",
            $cond,
            " THEN ((",
            $expr,
            ") / 1073741824) ELSE 0 END), 0) AS BIGINT) AS ",
            $hi,
            ", ",
            "CAST(COALESCE(SUM(CASE WHEN ",
            $cond,
            " THEN ((",
            $expr,
            ") % 1073741824) ELSE 0 END), 0) AS BIGINT) AS ",
            $lo
        )
    };
}

macro_rules! agg_select_exprs {
    ($bool_true:literal) => {
        concat!(
            "CAST(COUNT(*) AS BIGINT) AS total_turns, ",
            "CAST(COALESCE(SUM(CASE WHEN usage_observed = ", $bool_true, " OR stop_reason NOT IN ('cancelled', 'error') THEN 1 ELSE 0 END), 0) AS BIGINT) AS eligible_turns, ",
            "CAST(COALESCE(SUM(CASE WHEN usage_observed = ", $bool_true, " THEN 1 ELSE 0 END), 0) AS BIGINT) AS observed_usage_turns, ",
            "CAST(COALESCE(SUM(CASE WHEN inclusive_input_tokens IS NOT NULL AND inclusive_input_tokens >= 0 AND cached_read_tokens IS NOT NULL AND cached_read_tokens >= 0 AND cached_read_tokens <= inclusive_input_tokens THEN 1 ELSE 0 END), 0) AS BIGINT) AS observed_cache_turns, ",
            "CAST(COALESCE(SUM(CASE WHEN inclusive_input_tokens IS NOT NULL AND inclusive_input_tokens >= 0 AND cache_write_tokens IS NOT NULL AND cache_write_tokens >= 0 AND cache_write_tokens <= inclusive_input_tokens THEN 1 ELSE 0 END), 0) AS BIGINT) AS observed_cache_write_turns, ",
            "CAST(COALESCE(SUM(CASE WHEN inclusive_input_tokens IS NOT NULL AND inclusive_input_tokens >= 0 AND cached_read_tokens IS NOT NULL AND cached_read_tokens >= 0 AND cache_write_tokens IS NOT NULL AND cache_write_tokens >= 0 AND other_uncached_input_tokens IS NOT NULL AND other_uncached_input_tokens >= 0 AND cached_read_tokens + cache_write_tokens + other_uncached_input_tokens = inclusive_input_tokens THEN 1 ELSE 0 END), 0) AS BIGINT) AS observed_complete_breakdown_turns, ",
            "CAST(COALESCE(SUM(CASE WHEN reasoning_tokens IS NOT NULL AND reasoning_tokens >= 0 THEN 1 ELSE 0 END), 0) AS BIGINT) AS observed_reasoning_turns, ",
            "CAST(COALESCE(SUM(CASE WHEN renewed = ", $bool_true, " THEN 1 ELSE 0 END), 0) AS BIGINT) AS renewed_turns, ",
            "CAST(COALESCE(SUM(CASE WHEN compacted = ", $bool_true, " THEN 1 ELSE 0 END), 0) AS BIGINT) AS compacted_turns, ",
            limb_sum!(concat!("usage_observed = ", $bool_true, " AND inclusive_input_tokens IS NOT NULL AND inclusive_input_tokens >= 0"), "inclusive_input_tokens", "inclusive_input_hi", "inclusive_input_lo"), ", ",
            limb_sum!("inclusive_input_tokens IS NOT NULL AND inclusive_input_tokens >= 0 AND cached_read_tokens IS NOT NULL AND cached_read_tokens >= 0 AND cached_read_tokens <= inclusive_input_tokens", "inclusive_input_tokens", "observed_cache_input_hi", "observed_cache_input_lo"), ", ",
            limb_sum!("inclusive_input_tokens IS NOT NULL AND inclusive_input_tokens >= 0 AND cached_read_tokens IS NOT NULL AND cached_read_tokens >= 0 AND cached_read_tokens <= inclusive_input_tokens", "cached_read_tokens", "cached_read_hi", "cached_read_lo"), ", ",
            limb_sum!("inclusive_input_tokens IS NOT NULL AND inclusive_input_tokens >= 0 AND cache_write_tokens IS NOT NULL AND cache_write_tokens >= 0 AND cache_write_tokens <= inclusive_input_tokens", "cache_write_tokens", "cache_write_hi", "cache_write_lo"), ", ",
            limb_sum!("inclusive_input_tokens IS NOT NULL AND inclusive_input_tokens >= 0 AND cached_read_tokens IS NOT NULL AND cached_read_tokens >= 0 AND cached_read_tokens <= inclusive_input_tokens", "inclusive_input_tokens - cached_read_tokens", "uncached_input_hi", "uncached_input_lo"), ", ",
            limb_sum!("inclusive_input_tokens IS NOT NULL AND inclusive_input_tokens >= 0 AND cached_read_tokens IS NOT NULL AND cached_read_tokens >= 0 AND cache_write_tokens IS NOT NULL AND cache_write_tokens >= 0 AND other_uncached_input_tokens IS NOT NULL AND other_uncached_input_tokens >= 0 AND cached_read_tokens + cache_write_tokens + other_uncached_input_tokens = inclusive_input_tokens", "other_uncached_input_tokens", "other_uncached_hi", "other_uncached_lo"), ", ",
            limb_sum!(concat!("usage_observed = ", $bool_true, " AND output_tokens IS NOT NULL AND output_tokens >= 0"), "output_tokens", "output_hi", "output_lo"), ", ",
            limb_sum!("reasoning_tokens IS NOT NULL AND reasoning_tokens >= 0", "reasoning_tokens", "reasoning_hi", "reasoning_lo"), ", ",
            "MIN(occurred_at_ms) AS oldest_fact_ms, ",
            "MAX(occurred_at_ms) AS newest_fact_ms"
        )
    };
}

macro_rules! impl_turn_cache_backend {
    (
        $project_fn:ident,
        $project_missing_session_fn:ident,
        $project_one_fn:ident,
        $session_snapshot_fn:ident,
        $backfill_state_fn:ident,
        $decode_agg_fn:ident,
        $decode_turn_fn:ident,
        $begin_tx_fn:ident,
        $begin_read_tx_fn:ident,
        $db:ty,
        $storage:ty,
        $row_ty:ty,
        $ts_expr:literal,
        $e_ts_expr:literal,
        $kind_expr:literal,
        $e_kind_expr:literal,
        $update_expr:literal,
        $status_expr:literal,
        $delivery_expr:literal,
        $bool_true:literal,
        $owner_eq_expr:literal,
        $min_fn:literal
    ) => {
        fn $decode_agg_fn(row: &$row_ty) -> Result<AggRow> {
            Ok(AggRow {
                total_turns: row.try_get("total_turns")?,
                eligible_turns: row.try_get("eligible_turns")?,
                observed_usage_turns: row.try_get("observed_usage_turns")?,
                observed_cache_turns: row.try_get("observed_cache_turns")?,
                observed_cache_write_turns: row.try_get("observed_cache_write_turns")?,
                observed_complete_breakdown_turns: row
                    .try_get("observed_complete_breakdown_turns")?,
                observed_reasoning_turns: row.try_get("observed_reasoning_turns")?,
                renewed_turns: row.try_get("renewed_turns")?,
                compacted_turns: row.try_get("compacted_turns")?,
                inclusive_input_tokens: combine_limbs(
                    row.try_get("inclusive_input_hi")?,
                    row.try_get("inclusive_input_lo")?,
                ),
                observed_cache_input_tokens: combine_limbs(
                    row.try_get("observed_cache_input_hi")?,
                    row.try_get("observed_cache_input_lo")?,
                ),
                cached_read_tokens: combine_limbs(
                    row.try_get("cached_read_hi")?,
                    row.try_get("cached_read_lo")?,
                ),
                cache_write_tokens: combine_limbs(
                    row.try_get("cache_write_hi")?,
                    row.try_get("cache_write_lo")?,
                ),
                uncached_input_tokens: combine_limbs(
                    row.try_get("uncached_input_hi")?,
                    row.try_get("uncached_input_lo")?,
                ),
                other_uncached_input_tokens: combine_limbs(
                    row.try_get("other_uncached_hi")?,
                    row.try_get("other_uncached_lo")?,
                ),
                output_tokens: combine_limbs(
                    row.try_get("output_hi")?,
                    row.try_get("output_lo")?,
                ),
                reasoning_tokens: combine_limbs(
                    row.try_get("reasoning_hi")?,
                    row.try_get("reasoning_lo")?,
                ),
                oldest_fact_ms: row.try_get("oldest_fact_ms")?,
                newest_fact_ms: row.try_get("newest_fact_ms")?,
            })
        }

        #[allow(clippy::cast_precision_loss)]
        fn $decode_turn_fn(row: &$row_ty) -> Result<TurnCacheFact> {
            let inclusive_raw: Option<i64> = row.try_get("inclusive_input_tokens")?;
            let cached_raw: Option<i64> = row.try_get("cached_read_tokens")?;
            let inclusive = inclusive_raw.and_then(|v| u64::try_from(v).ok());
            let cached = cached_raw.and_then(|v| u64::try_from(v).ok());
            let uncached = match (inclusive, cached) {
                (Some(inc), Some(read)) if read <= inc => Some(inc - read),
                _ => None,
            };
            let cached_input_share = match (inclusive, cached) {
                (Some(inc), Some(read)) if inc > 0 && read <= inc => {
                    Some(read as f64 / inc as f64)
                }
                _ => None,
            };
            Ok(TurnCacheFact {
                turn_end_seq: to_u64(row.try_get::<i64, _>("turn_end_seq")?),
                occurred_at_ms: row.try_get("occurred_at_ms")?,
                model: row.try_get("model")?,
                stop_reason: row.try_get("stop_reason")?,
                prompt_origin: row.try_get("prompt_origin")?,
                idle_gap_ms: row.try_get("idle_gap_ms")?,
                renewed: row.try_get("renewed")?,
                compacted: row.try_get("compacted")?,
                usage_observed: row.try_get("usage_observed")?,
                inclusive_input_tokens: inclusive,
                cached_read_tokens: cached,
                cache_write_tokens: row
                    .try_get::<Option<i64>, _>("cache_write_tokens")?
                    .and_then(|v| u64::try_from(v).ok()),
                uncached_input_tokens: uncached,
                other_uncached_input_tokens: row
                    .try_get::<Option<i64>, _>("other_uncached_input_tokens")?
                    .and_then(|v| u64::try_from(v).ok()),
                output_tokens: row
                    .try_get::<Option<i64>, _>("output_tokens")?
                    .and_then(|v| u64::try_from(v).ok()),
                reasoning_tokens: row
                    .try_get::<Option<i64>, _>("reasoning_tokens")?
                    .and_then(|v| u64::try_from(v).ok()),
                cached_input_share,
                accounting_source: row.try_get("accounting_source")?,
            })
        }

        async fn $session_snapshot_fn(
            tx: &mut sqlx::Transaction<'_, $db>,
            session_id: &str,
        ) -> Result<Option<SessionSnapshot>> {
            let row = sqlx::query(
                "SELECT owner_user_id, provider, provider_account_id, provider_version, \
                        provider_generation_digest, last_usage, config_preferences, config_options \
                 FROM sessions WHERE id = $1",
            )
            .bind(session_id)
            .fetch_optional(&mut **tx)
            .await
            .with_context(|| format!("read session metadata for cache projection {session_id}"))?;
            let Some(row) = row else {
                return Ok(None);
            };
            let last_usage: Option<Value> = row.try_get("last_usage").ok().flatten();
            let config_preferences: Option<Value> =
                row.try_get("config_preferences").ok().flatten();
            let config_options: Option<Value> = row.try_get("config_options").ok().flatten();
            let raw_version: String = row.try_get("provider_version")?;
            let raw_digest: String = row.try_get("provider_generation_digest")?;
            Ok(Some(SessionSnapshot {
                owner_user_id: row.try_get("owner_user_id")?,
                provider: row.try_get("provider")?,
                provider_account_id: row.try_get("provider_account_id")?,
                provider_version: sanitize_text(&raw_version, MAX_VERSION_BYTES),
                provider_generation_digest: sanitize_text(&raw_digest, MAX_VERSION_BYTES),
                model: extract_session_model(
                    last_usage.as_ref(),
                    config_preferences.as_ref(),
                    config_options.as_ref(),
                ),
            }))
        }

        #[allow(clippy::too_many_lines)]
        async fn $project_one_fn(
            tx: &mut sqlx::Transaction<'_, $db>,
            session_id: &str,
            seq: i64,
            occurred_at_ms: i64,
            stop_reason: &str,
            usage: Option<&TurnUsage>,
            mode: ProjectionMode,
        ) -> Result<bool> {
            let Some(session) = $session_snapshot_fn(tx, session_id).await? else {
                return Ok(false);
            };
            // Locate the immediate preceding canonical TurnEnd directly in
            // `events` by indexed `(session_id, seq)` so live projection never
            // depends on backfill ordering and automatically respects Clear.
            let prev_sql = concat!(
                "SELECT seq, ", $ts_expr, " AS ts_ms \
                 FROM events \
                 WHERE session_id = $1 AND seq < $2 AND ", $kind_expr, " = 'turn_end' \
                 ORDER BY seq DESC LIMIT 1"
            );
            let prev: Option<(i64, i64)> = sqlx::query_as(prev_sql)
                .bind(session_id)
                .bind(seq)
                .fetch_optional(&mut **tx)
                .await?;

            let mut window_start = prev.map_or(-1_i64, |(prev_seq, _)| prev_seq);
            let mut prev_turn_end_ms = prev.map(|(_, prev_ms)| prev_ms);

            let markers_sql = concat!(
                "SELECT \
                   CAST(COALESCE(MAX(CASE WHEN ", $update_expr, " = 'context_rolled_over' THEN 1 ELSE 0 END), 0) AS BIGINT), \
                   CAST(COALESCE(MAX(CASE WHEN ", $update_expr, " = 'compaction_update' \
                                     AND ", $status_expr, " = 'completed' THEN 1 ELSE 0 END), 0) AS BIGINT), \
                   MAX(CASE WHEN ", $update_expr, " = 'context_cleared' THEN seq ELSE NULL END) \
                 FROM events \
                 WHERE session_id = $1 AND seq > $2 AND seq < $3 \
                   AND ", $kind_expr, " = 'update' \
                   AND ", $update_expr, " IN ('context_rolled_over', 'compaction_update', 'context_cleared')"
            );
            let (renewed_i, compacted_i, cleared_seq): (i64, i64, Option<i64>) =
                sqlx::query_as(markers_sql)
                    .bind(session_id)
                    .bind(window_start)
                    .bind(seq)
                    .fetch_one(&mut **tx)
                    .await?;
            let renewed = renewed_i != 0;
            let compacted = compacted_i != 0;
            if let Some(cs) = cleared_seq {
                prev_turn_end_ms = None;
                window_start = cs;
            }

            let prompt_sql = concat!(
                "SELECT ", $ts_expr, " AS ts_ms, payload \
                 FROM events \
                 WHERE session_id = $1 AND seq > $2 AND seq < $3 \
                   AND ", $kind_expr, " = 'update' \
                   AND ", $update_expr, " = 'user_message_chunk' \
                   AND COALESCE(", $delivery_expr, ", '') != 'current_turn' \
                 ORDER BY seq ASC LIMIT 1"
            );
            let prompt_row = sqlx::query(prompt_sql)
                .bind(session_id)
                .bind(window_start)
                .bind(seq)
                .fetch_optional(&mut **tx)
                .await?;
            let (prompt_origin, idle_gap_ms) = match prompt_row {
                Some(row) => {
                    let prompt_ts_ms: i64 = row.try_get("ts_ms")?;
                    let payload: Value = row.try_get("payload")?;
                    let origin = classify_prompt_origin(&payload);
                    let gap = prev_turn_end_ms
                        .filter(|prev_ms| prompt_ts_ms >= *prev_ms)
                        .map(|prev_ms| prompt_ts_ms - prev_ms);
                    (origin, gap)
                }
                None => ("unknown".to_owned(), None),
            };

            let normalized = normalize_turn_usage(usage);
            let usage_observed = normalized.is_some();
            let stop_reason_norm = normalize_stop_reason(stop_reason);
            let (provider_version, provider_generation_digest, model) = match mode {
                ProjectionMode::Live => (
                    session.provider_version.as_str(),
                    session.provider_generation_digest.as_str(),
                    session.model.as_deref(),
                ),
                // Backfill cannot prove what generation or model ran on a past
                // turn after reloads/switches; leave historical attribution unknown.
                ProjectionMode::BackfillMissing => ("", "", None),
            };

            let upsert_sql = match mode {
                ProjectionMode::Live => {
                    "INSERT INTO turn_cache_facts ( \
                        session_id, turn_end_seq, occurred_at_ms, owner_user_id, provider, \
                        provider_account_id, provider_version, provider_generation_digest, \
                        model, stop_reason, prompt_origin, prev_turn_end_ms, idle_gap_ms, \
                        renewed, compacted, usage_observed, total_tokens, inclusive_input_tokens, \
                        cached_read_tokens, cache_write_tokens, other_uncached_input_tokens, \
                        output_tokens, reasoning_tokens, accounting_source \
                     ) VALUES ( \
                        $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                        $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, 'turn_end' \
                     ) ON CONFLICT (session_id, turn_end_seq) DO UPDATE SET \
                        occurred_at_ms = EXCLUDED.occurred_at_ms, \
                        owner_user_id = EXCLUDED.owner_user_id, \
                        provider = EXCLUDED.provider, \
                        provider_account_id = EXCLUDED.provider_account_id, \
                        provider_version = EXCLUDED.provider_version, \
                        provider_generation_digest = EXCLUDED.provider_generation_digest, \
                        model = COALESCE(EXCLUDED.model, turn_cache_facts.model), \
                        stop_reason = EXCLUDED.stop_reason, \
                        prompt_origin = EXCLUDED.prompt_origin, \
                        prev_turn_end_ms = EXCLUDED.prev_turn_end_ms, \
                        idle_gap_ms = EXCLUDED.idle_gap_ms, \
                        renewed = EXCLUDED.renewed, \
                        compacted = EXCLUDED.compacted, \
                        usage_observed = EXCLUDED.usage_observed, \
                        total_tokens = EXCLUDED.total_tokens, \
                        inclusive_input_tokens = EXCLUDED.inclusive_input_tokens, \
                        cached_read_tokens = EXCLUDED.cached_read_tokens, \
                        cache_write_tokens = EXCLUDED.cache_write_tokens, \
                        other_uncached_input_tokens = EXCLUDED.other_uncached_input_tokens, \
                        output_tokens = EXCLUDED.output_tokens, \
                        reasoning_tokens = EXCLUDED.reasoning_tokens"
                }
                ProjectionMode::BackfillMissing => {
                    "INSERT INTO turn_cache_facts ( \
                        session_id, turn_end_seq, occurred_at_ms, owner_user_id, provider, \
                        provider_account_id, provider_version, provider_generation_digest, \
                        model, stop_reason, prompt_origin, prev_turn_end_ms, idle_gap_ms, \
                        renewed, compacted, usage_observed, total_tokens, inclusive_input_tokens, \
                        cached_read_tokens, cache_write_tokens, other_uncached_input_tokens, \
                        output_tokens, reasoning_tokens, accounting_source \
                     ) VALUES ( \
                        $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                        $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, 'turn_end' \
                     ) ON CONFLICT (session_id, turn_end_seq) DO NOTHING"
                }
            };

            let done = sqlx::query(upsert_sql)
                .bind(session_id)
                .bind(seq)
                .bind(occurred_at_ms)
                .bind(&session.owner_user_id)
                .bind(&session.provider)
                .bind(&session.provider_account_id)
                .bind(provider_version)
                .bind(provider_generation_digest)
                .bind(model)
                .bind(stop_reason_norm)
                .bind(&prompt_origin)
                .bind(prev_turn_end_ms)
                .bind(idle_gap_ms)
                .bind(renewed)
                .bind(compacted)
                .bind(usage_observed)
                .bind(normalized.map(|n| n.total_tokens))
                .bind(normalized.map(|n| n.inclusive_input_tokens))
                .bind(normalized.and_then(|n| n.cached_read_tokens))
                .bind(normalized.and_then(|n| n.cache_write_tokens))
                .bind(normalized.and_then(|n| n.other_uncached_input_tokens))
                .bind(normalized.map(|n| n.output_tokens))
                .bind(normalized.and_then(|n| n.reasoning_tokens))
                .execute(&mut **tx)
                .await
                .with_context(|| format!("UPSERT turn_cache_facts {session_id}/{seq}"))?;
            Ok(done.rows_affected() > 0)
        }

        pub(in crate::store) async fn $project_fn(
            tx: &mut sqlx::Transaction<'_, $db>,
            events: &[Envelope],
            now_ms: i64,
        ) -> Result<()> {
            let mut turn_ends: Vec<&Envelope> = events
                .iter()
                .filter(|envelope| matches!(envelope.event, Event::TurnEnd { .. }))
                .collect();
            if turn_ends.is_empty() {
                return Ok(());
            }
            turn_ends.sort_by_key(|envelope| (&envelope.session_id, envelope.seq));
            let ts_sql = concat!(
                "SELECT ", $ts_expr, " AS ts_ms FROM events WHERE session_id = $1 AND seq = $2"
            );
            for envelope in turn_ends {
                let Event::TurnEnd { stop_reason, usage } = &envelope.event else {
                    continue;
                };
                let seq = i64::try_from(envelope.seq).context("turn_end seq i64 overflow")?;
                let occurred_at_ms = sqlx::query_scalar::<_, i64>(ts_sql)
                    .bind(&envelope.session_id)
                    .bind(seq)
                    .fetch_optional(&mut **tx)
                    .await?
                    .unwrap_or(now_ms);
                $project_one_fn(
                    tx,
                    &envelope.session_id,
                    seq,
                    occurred_at_ms,
                    stop_reason,
                    usage.as_ref(),
                    ProjectionMode::Live,
                )
                .await?;
            }
            Ok(())
        }

        /// Preserve any retained unprojected `TurnEnd` rows for `session_id`
        /// inside the caller's transaction before `Clear` or session purge
        /// removes `events`.
        pub(in crate::store) async fn $project_missing_session_fn(
            tx: &mut sqlx::Transaction<'_, $db>,
            session_id: &str,
        ) -> Result<usize> {
            let cutoff_ms = chrono::Utc::now()
                .timestamp_millis()
                .saturating_sub(DEFAULT_RETENTION_DAYS.saturating_mul(86_400_000));
            let sql = concat!(
                "SELECT e.seq, ", $e_ts_expr, " AS ts_ms, e.payload \
                 FROM events e \
                 LEFT JOIN turn_cache_facts f \
                   ON f.session_id = e.session_id AND f.turn_end_seq = e.seq \
                 WHERE e.session_id = $1 \
                   AND ", $e_kind_expr, " = 'turn_end' \
                   AND f.session_id IS NULL \
                   AND ", $e_ts_expr, " >= $2 \
                 ORDER BY e.seq ASC"
            );
            let missing = sqlx::query(sql)
                .bind(session_id)
                .bind(cutoff_ms)
                .fetch_all(&mut **tx)
                .await
                .with_context(|| format!("select unprojected TurnEnds before delete for {session_id}"))?;
            let mut inserted = 0usize;
            for row in missing {
                let seq: i64 = row.try_get("seq")?;
                let ts_ms: i64 = row.try_get("ts_ms")?;
                let payload: Value = row.try_get("payload")?;
                let stop_reason = payload
                    .get("stop_reason")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let usage: Option<TurnUsage> = payload
                    .get("usage")
                    .filter(|value| !value.is_null())
                    .and_then(|value| serde_json::from_value(value.clone()).ok());
                if $project_one_fn(
                    tx,
                    session_id,
                    seq,
                    ts_ms,
                    stop_reason,
                    usage.as_ref(),
                    ProjectionMode::BackfillMissing,
                )
                .await?
                {
                    inserted += 1;
                }
            }
            Ok(inserted)
        }

        async fn $backfill_state_fn<'e, E>(exec: E) -> Result<BackfillState>
        where
            E: sqlx::Executor<'e, Database = $db>,
        {
            let row = sqlx::query(
                "SELECT cursor_session_id, cursor_seq, completed_at_ms \
                 FROM turn_cache_backfill_state WHERE slot = 1",
            )
            .fetch_optional(exec)
            .await?;
            let Some(row) = row else {
                return Ok(BackfillState::default());
            };
            Ok(BackfillState {
                cursor_session_id: row.try_get("cursor_session_id")?,
                cursor_seq: row.try_get("cursor_seq")?,
                completed_at_ms: row.try_get("completed_at_ms")?,
            })
        }

        impl $storage {
            /// Reset the reconciliation cursor on Controller startup so any
            /// retained `TurnEnd` rows written during a temporary Controller
            /// rollback are discovered and inserted (`ON CONFLICT DO NOTHING`)
            /// without overwriting live-projected facts.
            pub(in crate::store) async fn begin_turn_cache_reconcile(&self) -> Result<()> {
                let now_ms = chrono::Utc::now().timestamp_millis();
                let mut tx = $begin_tx_fn(&self.pool).await?;
                sqlx::query(
                    "INSERT INTO turn_cache_backfill_state ( \
                        slot, cursor_session_id, cursor_seq, completed_at_ms, \
                        oldest_backfilled_ms, rows_backfilled, updated_at_ms \
                     ) VALUES (1, '', -1, NULL, NULL, 0, $1) \
                     ON CONFLICT (slot) DO UPDATE SET \
                        cursor_session_id = '', \
                        cursor_seq = -1, \
                        completed_at_ms = NULL, \
                        updated_at_ms = EXCLUDED.updated_at_ms",
                )
                .bind(now_ms)
                .execute(&mut *tx)
                .await
                .context("reset turn_cache_backfill_state for startup reconcile")?;
                tx.commit().await?;
                Ok(())
            }

            #[allow(clippy::too_many_lines)]
            pub(in crate::store) async fn backfill_turn_cache_facts_batch(
                &self,
                scan_limit: usize,
                retention_days: i64,
            ) -> Result<BackfillProgress> {
                let limit = i64::try_from(scan_limit.clamp(1, 5_000)).unwrap_or(1_000);
                let state = $backfill_state_fn(&self.pool).await?;
                if state.completed_at_ms.is_some() {
                    return Ok(BackfillProgress {
                        scanned_events: 0,
                        rows_projected: 0,
                        completed: true,
                        cursor_session_id: state.cursor_session_id,
                        cursor_seq: state.cursor_seq,
                    });
                }

                let now_ms = chrono::Utc::now().timestamp_millis();
                let cutoff_ms =
                    now_ms.saturating_sub(retention_days.max(1).saturating_mul(86_400_000));
                let scan_sql = concat!(
                    "SELECT e.session_id, e.seq, ", $e_ts_expr, " AS ts_ms, \
                            CASE WHEN ", $e_kind_expr, " = 'turn_end' AND f.session_id IS NULL \
                                 THEN e.payload ELSE NULL END AS turn_payload \
                     FROM events e \
                     LEFT JOIN turn_cache_facts f \
                       ON f.session_id = e.session_id AND f.turn_end_seq = e.seq \
                     WHERE (e.session_id, e.seq) > ($1, $2) \
                     ORDER BY e.session_id ASC, e.seq ASC \
                     LIMIT $3"
                );
                let scanned = sqlx::query(scan_sql)
                    .bind(&state.cursor_session_id)
                    .bind(state.cursor_seq)
                    .bind(limit)
                    .fetch_all(&self.pool)
                    .await
                    .context("scan events for turn_cache_facts backfill")?;

                let scanned_count = scanned.len();
                let mut last_session = state.cursor_session_id.clone();
                let mut last_seq = state.cursor_seq;
                let mut turn_candidates: Vec<(String, i64, i64, Value)> = Vec::new();
                for row in scanned {
                    let sid: String = row.try_get("session_id")?;
                    let seq: i64 = row.try_get("seq")?;
                    let ts_ms: i64 = row.try_get("ts_ms")?;
                    let payload: Option<Value> = row.try_get("turn_payload")?;
                    last_session.clone_from(&sid);
                    last_seq = seq;
                    if ts_ms >= cutoff_ms
                        && let Some(payload) = payload
                    {
                        turn_candidates.push((sid, seq, ts_ms, payload));
                    }
                }

                let completed = i64::try_from(scanned_count).unwrap_or(i64::MAX) < limit;
                let mut tx = $begin_tx_fn(&self.pool).await?;
                let mut projected = 0usize;
                let mut batch_oldest: Option<i64> = None;
                for (sid, seq, ts_ms, payload) in turn_candidates {
                    let stop_reason = payload
                        .get("stop_reason")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let usage: Option<TurnUsage> = payload
                        .get("usage")
                        .filter(|value| !value.is_null())
                        .and_then(|value| serde_json::from_value(value.clone()).ok());
                    if $project_one_fn(
                        &mut tx,
                        &sid,
                        seq,
                        ts_ms,
                        stop_reason,
                        usage.as_ref(),
                        ProjectionMode::BackfillMissing,
                    )
                    .await?
                    {
                        projected += 1;
                        batch_oldest = Some(match batch_oldest {
                            Some(current) => current.min(ts_ms),
                            None => ts_ms,
                        });
                    }
                }

                let completed_at_ms = completed.then_some(now_ms);
                let state_sql = concat!(
                    "INSERT INTO turn_cache_backfill_state ( \
                        slot, cursor_session_id, cursor_seq, completed_at_ms, \
                        oldest_backfilled_ms, rows_backfilled, updated_at_ms \
                     ) VALUES (1, $1, $2, $3, $4, $5, $6) \
                     ON CONFLICT (slot) DO UPDATE SET \
                        cursor_session_id = EXCLUDED.cursor_session_id, \
                        cursor_seq = EXCLUDED.cursor_seq, \
                        completed_at_ms = COALESCE(turn_cache_backfill_state.completed_at_ms, EXCLUDED.completed_at_ms), \
                        oldest_backfilled_ms = CASE \
                            WHEN turn_cache_backfill_state.oldest_backfilled_ms IS NULL THEN EXCLUDED.oldest_backfilled_ms \
                            WHEN EXCLUDED.oldest_backfilled_ms IS NULL THEN turn_cache_backfill_state.oldest_backfilled_ms \
                            ELSE ", $min_fn, "(turn_cache_backfill_state.oldest_backfilled_ms, EXCLUDED.oldest_backfilled_ms) \
                        END, \
                        rows_backfilled = turn_cache_backfill_state.rows_backfilled + EXCLUDED.rows_backfilled, \
                        updated_at_ms = EXCLUDED.updated_at_ms"
                );
                sqlx::query(state_sql)
                    .bind(&last_session)
                    .bind(last_seq)
                    .bind(completed_at_ms)
                    .bind(batch_oldest)
                    .bind(i64::try_from(projected).unwrap_or(0))
                    .bind(now_ms)
                    .execute(&mut *tx)
                    .await
                    .context("advance turn_cache_backfill_state")?;
                tx.commit().await?;

                Ok(BackfillProgress {
                    scanned_events: scanned_count,
                    rows_projected: projected,
                    completed,
                    cursor_session_id: last_session,
                    cursor_seq: last_seq,
                })
            }

            pub(in crate::store) async fn prune_turn_cache_facts(
                &self,
                retention_days: i64,
            ) -> Result<u64> {
                let cutoff_ms = chrono::Utc::now()
                    .timestamp_millis()
                    .saturating_sub(retention_days.max(1).saturating_mul(86_400_000));
                let result = sqlx::query("DELETE FROM turn_cache_facts WHERE occurred_at_ms < $1")
                    .bind(cutoff_ms)
                    .execute(&self.pool)
                    .await
                    .context("prune expired turn_cache_facts")?;
                Ok(result.rows_affected())
            }

            pub(in crate::store) async fn account_cache_metrics(
                &self,
                owner_user_id: &str,
                provider: &str,
                provider_account_id: &str,
                from_ms: i64,
                to_ms: i64,
                window: &str,
            ) -> Result<AccountCacheMetrics> {
                let mut tx = $begin_read_tx_fn(&self.pool).await?;
                let totals_sql = concat!(
                    "SELECT ", agg_select_exprs!($bool_true), " \
                     FROM turn_cache_facts \
                     WHERE owner_user_id = $1 \
                       AND provider = $2 \
                       AND provider_account_id = $3 \
                       AND occurred_at_ms >= $4 \
                       AND occurred_at_ms <= $5"
                );
                let agg_row = sqlx::query(totals_sql)
                    .bind(owner_user_id)
                    .bind(provider)
                    .bind(provider_account_id)
                    .bind(from_ms)
                    .bind(to_ms)
                    .fetch_one(&mut *tx)
                    .await
                    .context("aggregate account turn_cache_facts")?;
                let agg = $decode_agg_fn(&agg_row)?;

                let models_sql = concat!(
                    "SELECT * FROM (SELECT COALESCE(NULLIF(model, ''), 'unknown') AS model_key, ",
                    agg_select_exprs!($bool_true),
                    " FROM turn_cache_facts \
                     WHERE owner_user_id = $1 \
                       AND provider = $2 \
                       AND provider_account_id = $3 \
                       AND occurred_at_ms >= $4 \
                       AND occurred_at_ms <= $5 \
                     GROUP BY model_key) AS grouped \
                     ORDER BY (grouped.inclusive_input_hi * 1073741824.0 + grouped.inclusive_input_lo) DESC, grouped.total_turns DESC, grouped.model_key ASC \
                     LIMIT $6"
                );
                let model_rows = sqlx::query(models_sql)
                    .bind(owner_user_id)
                    .bind(provider)
                    .bind(provider_account_id)
                    .bind(from_ms)
                    .bind(to_ms)
                    .bind(MAX_MODELS_IN_RESPONSE)
                    .fetch_all(&mut *tx)
                    .await
                    .context("aggregate account turn_cache_facts by model")?;

                let mut by_model = Vec::with_capacity(model_rows.len());
                for row in &model_rows {
                    let model: String = row.try_get("model_key")?;
                    let model_agg = $decode_agg_fn(row)?;
                    by_model.push(ModelCacheMetrics {
                        model,
                        totals: agg_to_totals(&model_agg),
                    });
                }

                let backfill = $backfill_state_fn(&mut *tx).await?;
                let _ = tx.rollback().await;
                let totals = agg_to_totals(&agg);
                let coverage = build_scoped_coverage(&totals, &agg, &backfill);
                Ok(AccountCacheMetrics {
                    provider: provider.to_owned(),
                    provider_account_id: provider_account_id.to_owned(),
                    window: window.to_owned(),
                    from_ms,
                    to_ms,
                    totals,
                    by_model,
                    coverage,
                })
            }

            pub(in crate::store) async fn session_cache_metrics(
                &self,
                session_id: &str,
                owner_user_id: Option<&str>,
                provider: &str,
                provider_account_id: &str,
                recent_turn_limit: usize,
            ) -> Result<SessionCacheMetrics> {
                let mut tx = $begin_read_tx_fn(&self.pool).await?;
                let totals_sql = concat!(
                    "SELECT ", agg_select_exprs!($bool_true), " \
                     FROM turn_cache_facts \
                     WHERE session_id = $1 \
                       AND provider = $2 \
                       AND provider_account_id = $3 \
                       AND ", $owner_eq_expr
                );
                let agg_row = sqlx::query(totals_sql)
                    .bind(session_id)
                    .bind(provider)
                    .bind(provider_account_id)
                    .bind(owner_user_id)
                    .fetch_one(&mut *tx)
                    .await
                    .context("aggregate session turn_cache_facts")?;
                let agg = $decode_agg_fn(&agg_row)?;

                let models_sql = concat!(
                    "SELECT * FROM (SELECT COALESCE(NULLIF(model, ''), 'unknown') AS model_key, ",
                    agg_select_exprs!($bool_true),
                    " FROM turn_cache_facts \
                     WHERE session_id = $1 \
                       AND provider = $2 \
                       AND provider_account_id = $3 \
                       AND ", $owner_eq_expr, " \
                     GROUP BY model_key) AS grouped \
                     ORDER BY (grouped.inclusive_input_hi * 1073741824.0 + grouped.inclusive_input_lo) DESC, grouped.total_turns DESC, grouped.model_key ASC \
                     LIMIT $5"
                );
                let model_rows = sqlx::query(models_sql)
                    .bind(session_id)
                    .bind(provider)
                    .bind(provider_account_id)
                    .bind(owner_user_id)
                    .bind(MAX_MODELS_IN_RESPONSE)
                    .fetch_all(&mut *tx)
                    .await
                    .context("aggregate session turn_cache_facts by model")?;
                let mut by_model = Vec::with_capacity(model_rows.len());
                for row in &model_rows {
                    let model: String = row.try_get("model_key")?;
                    let model_agg = $decode_agg_fn(row)?;
                    by_model.push(ModelCacheMetrics {
                        model,
                        totals: agg_to_totals(&model_agg),
                    });
                }

                let limit = i64::try_from(recent_turn_limit.min(MAX_RECENT_TURNS)).unwrap_or(30);
                let recent_sql = concat!(
                    "SELECT turn_end_seq, occurred_at_ms, model, stop_reason, prompt_origin, \
                            idle_gap_ms, renewed, compacted, usage_observed, \
                            inclusive_input_tokens, cached_read_tokens, cache_write_tokens, \
                            other_uncached_input_tokens, output_tokens, reasoning_tokens, \
                            accounting_source \
                     FROM turn_cache_facts \
                     WHERE session_id = $1 \
                       AND provider = $2 \
                       AND provider_account_id = $3 \
                       AND ", $owner_eq_expr, " \
                     ORDER BY turn_end_seq DESC \
                     LIMIT $5"
                );
                let recent_rows = sqlx::query(recent_sql)
                    .bind(session_id)
                    .bind(provider)
                    .bind(provider_account_id)
                    .bind(owner_user_id)
                    .bind(limit)
                    .fetch_all(&mut *tx)
                    .await
                    .context("query recent session turn_cache_facts")?;
                let mut recent_turns = Vec::with_capacity(recent_rows.len());
                for row in &recent_rows {
                    recent_turns.push($decode_turn_fn(row)?);
                }

                let backfill = $backfill_state_fn(&mut *tx).await?;
                let _ = tx.rollback().await;
                let totals = agg_to_totals(&agg);
                let coverage = build_scoped_coverage(&totals, &agg, &backfill);
                Ok(SessionCacheMetrics {
                    session_id: session_id.to_owned(),
                    provider: provider.to_owned(),
                    provider_account_id: provider_account_id.to_owned(),
                    totals,
                    by_model,
                    recent_turns,
                    coverage,
                })
            }
        }
    };
}

impl_turn_cache_backend!(
    project_events_pg,
    project_missing_for_session_pg,
    project_turn_end_pg,
    session_snapshot_pg,
    backfill_state_pg,
    decode_agg_pg,
    decode_turn_pg,
    begin_write_tx_pg,
    begin_read_tx_pg,
    sqlx::Postgres,
    PostgresStorage,
    sqlx::postgres::PgRow,
    "round(extract(epoch FROM ts) * 1000)::bigint",
    "round(extract(epoch FROM e.ts) * 1000)::bigint",
    "(payload ->> 'kind')",
    "(e.payload ->> 'kind')",
    "(payload #>> '{update,sessionUpdate}')",
    "(payload #>> '{update,status}')",
    "(payload #>> '{update,_meta,cowboyDelivery}')",
    "TRUE",
    "owner_user_id IS NOT DISTINCT FROM $4",
    "LEAST"
);

impl_turn_cache_backend!(
    project_events_sqlite,
    project_missing_for_session_sqlite,
    project_turn_end_sqlite,
    session_snapshot_sqlite,
    backfill_state_sqlite,
    decode_agg_sqlite,
    decode_turn_sqlite,
    begin_write_tx_sqlite,
    begin_read_tx_sqlite,
    sqlx::Sqlite,
    SqliteStorage,
    sqlx::sqlite::SqliteRow,
    "ts_ms",
    "e.ts_ms",
    "json_extract(payload, '$.kind')",
    "json_extract(e.payload, '$.kind')",
    "json_extract(payload, '$.update.sessionUpdate')",
    "json_extract(payload, '$.update.status')",
    "json_extract(payload, '$.update._meta.cowboyDelivery')",
    "1",
    "owner_user_id IS $4",
    "min"
);

impl Store {
    /// Reset the reconciliation cursor on startup so turns written during a
    /// temporary Controller rollback are repaired on redeploy without
    /// overwriting live-projected facts.
    pub async fn begin_turn_cache_reconcile(&self) -> Result<()> {
        match &self.backend {
            StorageBackend::Postgres(db) => db.begin_turn_cache_reconcile().await,
            StorageBackend::Sqlite(db) => db.begin_turn_cache_reconcile().await,
        }
    }

    /// Run one bounded keyset-paginated backfill batch from `events` into
    /// `turn_cache_facts`. Safe to invoke repeatedly until `progress.completed`.
    pub async fn backfill_turn_cache_facts_batch(
        &self,
        scan_limit: usize,
        retention_days: i64,
    ) -> Result<BackfillProgress> {
        match &self.backend {
            StorageBackend::Postgres(db) => {
                db.backfill_turn_cache_facts_batch(scan_limit, retention_days)
                    .await
            }
            StorageBackend::Sqlite(db) => {
                db.backfill_turn_cache_facts_batch(scan_limit, retention_days)
                    .await
            }
        }
    }

    /// Delete `turn_cache_facts` older than `retention_days`.
    pub async fn prune_turn_cache_facts(&self, retention_days: i64) -> Result<u64> {
        match &self.backend {
            StorageBackend::Postgres(db) => db.prune_turn_cache_facts(retention_days).await,
            StorageBackend::Sqlite(db) => db.prune_turn_cache_facts(retention_days).await,
        }
    }

    /// Read SQL-aggregated turn-level cache efficiency metrics strictly scoped
    /// to one `(owner_user_id, provider, provider_account_id)` tuple.
    pub async fn account_cache_metrics(
        &self,
        owner_user_id: &str,
        provider: &str,
        provider_account_id: &str,
        from_ms: i64,
        to_ms: i64,
        window: &str,
    ) -> Result<AccountCacheMetrics> {
        match &self.backend {
            StorageBackend::Postgres(db) => {
                db.account_cache_metrics(
                    owner_user_id,
                    provider,
                    provider_account_id,
                    from_ms,
                    to_ms,
                    window,
                )
                .await
            }
            StorageBackend::Sqlite(db) => {
                db.account_cache_metrics(
                    owner_user_id,
                    provider,
                    provider_account_id,
                    from_ms,
                    to_ms,
                    window,
                )
                .await
            }
        }
    }

    /// Read SQL-aggregated turn-level cache efficiency metrics and bounded
    /// recent turns for one session. Scoped by `(session_id, owner_user_id,
    /// provider, provider_account_id)` so retained facts can never cross owners.
    pub async fn session_cache_metrics(
        &self,
        session_id: &str,
        owner_user_id: Option<&str>,
        provider: &str,
        provider_account_id: &str,
        recent_turn_limit: usize,
    ) -> Result<SessionCacheMetrics> {
        match &self.backend {
            StorageBackend::Postgres(db) => {
                db.session_cache_metrics(
                    session_id,
                    owner_user_id,
                    provider,
                    provider_account_id,
                    recent_turn_limit,
                )
                .await
            }
            StorageBackend::Sqlite(db) => {
                db.session_cache_metrics(
                    session_id,
                    owner_user_id,
                    provider,
                    provider_account_id,
                    recent_turn_limit,
                )
                .await
            }
        }
    }
}
