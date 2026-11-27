//! SQLite and PostgreSQL projection, backfill, retention and query paths for
//! `turn_cache_facts`.

use anyhow::{Context as _, Result};
use serde_json::Value;
use sqlx::Row as _;

use super::{
    AccountCacheMetrics, BackfillProgress, CacheMetricsCoverage, CacheMetricsTotals,
    MAX_MODELS_IN_RESPONSE, MAX_RECENT_TURNS, ModelCacheMetrics, SessionCacheMetrics,
    TurnCacheFact, classify_prompt_origin, extract_session_model, is_eligible_turn,
    normalize_stop_reason, normalize_turn_usage,
};
use crate::agent_model::{Event, TurnUsage};
use crate::model::Envelope;
use crate::store::{PostgresStorage, SqliteStorage, StorageBackend, Store};

#[derive(Debug, Clone)]
struct BackfillState {
    cursor_session_id: String,
    cursor_seq: i64,
    completed_at_ms: Option<i64>,
    oldest_backfilled_ms: Option<i64>,
    rows_backfilled: i64,
}

impl Default for BackfillState {
    fn default() -> Self {
        Self {
            cursor_session_id: String::new(),
            cursor_seq: -1,
            completed_at_ms: None,
            oldest_backfilled_ms: None,
            rows_backfilled: 0,
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

#[derive(Debug)]
struct FactRow {
    turn_end_seq: i64,
    occurred_at_ms: i64,
    model: Option<String>,
    stop_reason: String,
    prompt_origin: String,
    idle_gap_ms: Option<i64>,
    renewed: bool,
    compacted: bool,
    usage_observed: bool,
    inclusive_input_tokens: Option<i64>,
    cached_read_tokens: Option<i64>,
    cache_write_tokens: Option<i64>,
    other_uncached_input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    accounting_source: String,
}

#[derive(Default)]
struct Accumulator {
    total_turns: u64,
    eligible_turns: u64,
    observed_usage_turns: u64,
    observed_cache_turns: u64,
    observed_cache_write_turns: u64,
    observed_other_uncached_turns: u64,
    observed_reasoning_turns: u64,
    renewed_turns: u64,
    compacted_turns: u64,
    inclusive_input_tokens: u64,
    observed_cache_input_tokens: u64,
    cached_read_tokens: u64,
    cache_write_tokens: u64,
    uncached_input_tokens: u64,
    other_uncached_input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
}

impl Accumulator {
    fn add(&mut self, row: &FactRow) {
        self.total_turns = self.total_turns.saturating_add(1);
        if is_eligible_turn(&row.stop_reason, row.usage_observed) {
            self.eligible_turns = self.eligible_turns.saturating_add(1);
        }
        if row.renewed {
            self.renewed_turns = self.renewed_turns.saturating_add(1);
        }
        if row.compacted {
            self.compacted_turns = self.compacted_turns.saturating_add(1);
        }
        if row.usage_observed {
            self.observed_usage_turns = self.observed_usage_turns.saturating_add(1);
        }
        if let Some(output) = row.output_tokens.and_then(|v| u64::try_from(v).ok()) {
            self.output_tokens = self.output_tokens.saturating_add(output);
        }
        if let Some(reasoning) = row.reasoning_tokens.and_then(|v| u64::try_from(v).ok()) {
            self.observed_reasoning_turns = self.observed_reasoning_turns.saturating_add(1);
            self.reasoning_tokens = self.reasoning_tokens.saturating_add(reasoning);
        }
        let Some(inclusive) = row
            .inclusive_input_tokens
            .and_then(|v| u64::try_from(v).ok())
        else {
            return;
        };
        self.inclusive_input_tokens = self.inclusive_input_tokens.saturating_add(inclusive);
        if let Some(cached) = row.cached_read_tokens.and_then(|v| u64::try_from(v).ok())
            && cached <= inclusive
        {
            self.observed_cache_turns = self.observed_cache_turns.saturating_add(1);
            self.observed_cache_input_tokens =
                self.observed_cache_input_tokens.saturating_add(inclusive);
            self.cached_read_tokens = self.cached_read_tokens.saturating_add(cached);
            self.uncached_input_tokens = self
                .uncached_input_tokens
                .saturating_add(inclusive - cached);
        }
        if let Some(write) = row.cache_write_tokens.and_then(|v| u64::try_from(v).ok()) {
            self.observed_cache_write_turns = self.observed_cache_write_turns.saturating_add(1);
            self.cache_write_tokens = self.cache_write_tokens.saturating_add(write);
        }
        if let Some(other) = row
            .other_uncached_input_tokens
            .and_then(|v| u64::try_from(v).ok())
        {
            self.observed_other_uncached_turns =
                self.observed_other_uncached_turns.saturating_add(1);
            self.other_uncached_input_tokens =
                self.other_uncached_input_tokens.saturating_add(other);
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn finish(self) -> CacheMetricsTotals {
        let cached_input_share = if self.observed_cache_input_tokens > 0 {
            Some(self.cached_read_tokens as f64 / self.observed_cache_input_tokens as f64)
        } else {
            None
        };
        let uncached_input_share = if self.observed_cache_input_tokens > 0 {
            Some(self.uncached_input_tokens as f64 / self.observed_cache_input_tokens as f64)
        } else {
            None
        };
        let other_uncached = if self.observed_cache_turns > 0
            && self.observed_other_uncached_turns == self.observed_cache_turns
        {
            Some(self.other_uncached_input_tokens)
        } else {
            None
        };
        CacheMetricsTotals {
            total_turns: self.total_turns,
            eligible_turns: self.eligible_turns,
            observed_usage_turns: self.observed_usage_turns,
            observed_cache_turns: self.observed_cache_turns,
            observed_cache_write_turns: self.observed_cache_write_turns,
            renewed_turns: self.renewed_turns,
            compacted_turns: self.compacted_turns,
            inclusive_input_tokens: self.inclusive_input_tokens,
            observed_cache_input_tokens: self.observed_cache_input_tokens,
            cached_read_tokens: (self.observed_cache_turns > 0).then_some(self.cached_read_tokens),
            cache_write_tokens: (self.observed_cache_write_turns > 0)
                .then_some(self.cache_write_tokens),
            uncached_input_tokens: (self.observed_cache_turns > 0)
                .then_some(self.uncached_input_tokens),
            other_uncached_input_tokens: other_uncached,
            output_tokens: self.output_tokens,
            reasoning_tokens: (self.observed_reasoning_turns > 0).then_some(self.reasoning_tokens),
            cached_input_share,
            uncached_input_share,
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn ratio(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

#[allow(clippy::cast_precision_loss)]
fn fact_row_to_public(row: &FactRow) -> TurnCacheFact {
    let inclusive = row
        .inclusive_input_tokens
        .and_then(|v| u64::try_from(v).ok());
    let cached = row.cached_read_tokens.and_then(|v| u64::try_from(v).ok());
    let uncached = match (inclusive, cached) {
        (Some(inc), Some(read)) if read <= inc => Some(inc - read),
        _ => None,
    };
    let cached_input_share = match (inclusive, cached) {
        (Some(inc), Some(read)) if inc > 0 && read <= inc => Some(read as f64 / inc as f64),
        _ => None,
    };
    TurnCacheFact {
        turn_end_seq: u64::try_from(row.turn_end_seq).unwrap_or(0),
        occurred_at_ms: row.occurred_at_ms,
        model: row.model.clone(),
        stop_reason: row.stop_reason.clone(),
        prompt_origin: row.prompt_origin.clone(),
        idle_gap_ms: row.idle_gap_ms,
        renewed: row.renewed,
        compacted: row.compacted,
        usage_observed: row.usage_observed,
        inclusive_input_tokens: inclusive,
        cached_read_tokens: cached,
        cache_write_tokens: row.cache_write_tokens.and_then(|v| u64::try_from(v).ok()),
        uncached_input_tokens: uncached,
        other_uncached_input_tokens: row
            .other_uncached_input_tokens
            .and_then(|v| u64::try_from(v).ok()),
        output_tokens: row.output_tokens.and_then(|v| u64::try_from(v).ok()),
        reasoning_tokens: row.reasoning_tokens.and_then(|v| u64::try_from(v).ok()),
        cached_input_share,
        accounting_source: row.accounting_source.clone(),
    }
}

fn aggregate_facts(rows: &[FactRow]) -> (CacheMetricsTotals, Vec<ModelCacheMetrics>) {
    let mut total = Accumulator::default();
    let mut per_model: std::collections::BTreeMap<String, Accumulator> =
        std::collections::BTreeMap::new();
    for row in rows {
        total.add(row);
        let model = row
            .model
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or("unknown");
        per_model.entry(model.to_owned()).or_default().add(row);
    }
    let mut models: Vec<ModelCacheMetrics> = per_model
        .into_iter()
        .map(|(model, acc)| ModelCacheMetrics {
            model,
            totals: acc.finish(),
        })
        .collect();
    models.sort_by(|left, right| {
        right
            .totals
            .inclusive_input_tokens
            .cmp(&left.totals.inclusive_input_tokens)
            .then_with(|| right.totals.total_turns.cmp(&left.totals.total_turns))
            .then_with(|| left.model.cmp(&right.model))
    });
    models.truncate(MAX_MODELS_IN_RESPONSE);
    (total.finish(), models)
}

fn build_coverage(totals: &CacheMetricsTotals, backfill: &BackfillState) -> CacheMetricsCoverage {
    CacheMetricsCoverage {
        accounting_source: "turn_end".to_owned(),
        granularity: "turn_total".to_owned(),
        backfill_complete: backfill.completed_at_ms.is_some(),
        oldest_backfilled_ms: backfill.oldest_backfilled_ms,
        rows_backfilled: u64::try_from(backfill.rows_backfilled).unwrap_or(0),
        usage_coverage_ratio: ratio(totals.observed_usage_turns, totals.eligible_turns),
        cache_coverage_ratio: ratio(totals.observed_cache_turns, totals.eligible_turns),
    }
}

macro_rules! impl_turn_cache_backend {
    (
        $project_fn:ident,
        $project_one_fn:ident,
        $session_snapshot_fn:ident,
        $backfill_state_fn:ident,
        $decode_row_fn:ident,
        $db:ty,
        $storage:ty,
        $row_ty:ty,
        $ts_expr:literal,
        $kind_expr:literal,
        $update_expr:literal,
        $status_expr:literal,
        $delivery_expr:literal,
        $min_fn:literal
    ) => {
        fn $decode_row_fn(row: &$row_ty) -> Result<FactRow> {
            Ok(FactRow {
                turn_end_seq: row.try_get("turn_end_seq")?,
                occurred_at_ms: row.try_get("occurred_at_ms")?,
                model: row.try_get("model")?,
                stop_reason: row.try_get("stop_reason")?,
                prompt_origin: row.try_get("prompt_origin")?,
                idle_gap_ms: row.try_get("idle_gap_ms")?,
                renewed: row.try_get("renewed")?,
                compacted: row.try_get("compacted")?,
                usage_observed: row.try_get("usage_observed")?,
                inclusive_input_tokens: row.try_get("inclusive_input_tokens")?,
                cached_read_tokens: row.try_get("cached_read_tokens")?,
                cache_write_tokens: row.try_get("cache_write_tokens")?,
                other_uncached_input_tokens: row.try_get("other_uncached_input_tokens")?,
                output_tokens: row.try_get("output_tokens")?,
                reasoning_tokens: row.try_get("reasoning_tokens")?,
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
            Ok(Some(SessionSnapshot {
                owner_user_id: row.try_get("owner_user_id")?,
                provider: row.try_get("provider")?,
                provider_account_id: row.try_get("provider_account_id")?,
                provider_version: row.try_get("provider_version")?,
                provider_generation_digest: row.try_get("provider_generation_digest")?,
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
        ) -> Result<bool> {
            let Some(session) = $session_snapshot_fn(tx, session_id).await? else {
                return Ok(false);
            };
            let prev: Option<(i64, i64)> = sqlx::query_as(
                "SELECT turn_end_seq, occurred_at_ms FROM turn_cache_facts \
                 WHERE session_id = $1 AND turn_end_seq < $2 \
                 ORDER BY turn_end_seq DESC LIMIT 1",
            )
            .bind(session_id)
            .bind(seq)
            .fetch_optional(&mut **tx)
            .await?;

            let mut window_start = prev.map_or(-1_i64, |(prev_seq, _)| prev_seq);
            let mut prev_turn_end_ms = None;
            if let Some((prev_seq, prev_ms)) = prev {
                let prev_present: Option<(i64,)> = sqlx::query_as(
                    "SELECT seq FROM events WHERE session_id = $1 AND seq = $2",
                )
                .bind(session_id)
                .bind(prev_seq)
                .fetch_optional(&mut **tx)
                .await?;
                if prev_present.is_some() {
                    prev_turn_end_ms = Some(prev_ms);
                }
            }

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

            sqlx::query(
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
                    reasoning_tokens = EXCLUDED.reasoning_tokens",
            )
            .bind(session_id)
            .bind(seq)
            .bind(occurred_at_ms)
            .bind(&session.owner_user_id)
            .bind(&session.provider)
            .bind(&session.provider_account_id)
            .bind(&session.provider_version)
            .bind(&session.provider_generation_digest)
            .bind(&session.model)
            .bind(&stop_reason_norm)
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
            Ok(true)
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
                )
                .await?;
            }
            Ok(())
        }

        async fn $backfill_state_fn(pool: &sqlx::Pool<$db>) -> Result<BackfillState> {
            let row = sqlx::query(
                "SELECT cursor_session_id, cursor_seq, completed_at_ms, \
                        oldest_backfilled_ms, rows_backfilled \
                 FROM turn_cache_backfill_state WHERE slot = 1",
            )
            .fetch_optional(pool)
            .await?;
            let Some(row) = row else {
                return Ok(BackfillState::default());
            };
            Ok(BackfillState {
                cursor_session_id: row.try_get("cursor_session_id")?,
                cursor_seq: row.try_get("cursor_seq")?,
                completed_at_ms: row.try_get("completed_at_ms")?,
                oldest_backfilled_ms: row.try_get("oldest_backfilled_ms")?,
                rows_backfilled: row.try_get("rows_backfilled")?,
            })
        }

        impl $storage {
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
                    "SELECT session_id, seq, ", $ts_expr, " AS ts_ms, \
                            CASE WHEN ", $kind_expr, " = 'turn_end' THEN payload ELSE NULL END AS turn_payload \
                     FROM events \
                     WHERE (session_id, seq) > ($1, $2) \
                     ORDER BY session_id ASC, seq ASC \
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
                let mut tx = self.pool.begin().await?;
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
                    if $project_one_fn(&mut tx, &sid, seq, ts_ms, stop_reason, usage.as_ref())
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
                let rows = sqlx::query(
                    "SELECT turn_end_seq, occurred_at_ms, model, stop_reason, prompt_origin, \
                            idle_gap_ms, renewed, compacted, usage_observed, \
                            inclusive_input_tokens, cached_read_tokens, cache_write_tokens, \
                            other_uncached_input_tokens, output_tokens, reasoning_tokens, \
                            accounting_source \
                     FROM turn_cache_facts \
                     WHERE owner_user_id = $1 \
                       AND provider = $2 \
                       AND provider_account_id = $3 \
                       AND occurred_at_ms >= $4 \
                       AND occurred_at_ms <= $5 \
                     ORDER BY occurred_at_ms DESC",
                )
                .bind(owner_user_id)
                .bind(provider)
                .bind(provider_account_id)
                .bind(from_ms)
                .bind(to_ms)
                .fetch_all(&self.pool)
                .await
                .context("query account turn_cache_facts")?;

                let mut facts = Vec::with_capacity(rows.len());
                for row in &rows {
                    facts.push($decode_row_fn(row)?);
                }
                let backfill = $backfill_state_fn(&self.pool).await?;
                let (totals, by_model) = aggregate_facts(&facts);
                let coverage = build_coverage(&totals, &backfill);
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
                provider: &str,
                provider_account_id: &str,
                recent_turn_limit: usize,
            ) -> Result<SessionCacheMetrics> {
                let rows = sqlx::query(
                    "SELECT turn_end_seq, occurred_at_ms, model, stop_reason, prompt_origin, \
                            idle_gap_ms, renewed, compacted, usage_observed, \
                            inclusive_input_tokens, cached_read_tokens, cache_write_tokens, \
                            other_uncached_input_tokens, output_tokens, reasoning_tokens, \
                            accounting_source \
                     FROM turn_cache_facts \
                     WHERE session_id = $1 \
                     ORDER BY turn_end_seq DESC",
                )
                .bind(session_id)
                .fetch_all(&self.pool)
                .await
                .context("query session turn_cache_facts")?;

                let mut facts = Vec::with_capacity(rows.len());
                for row in &rows {
                    facts.push($decode_row_fn(row)?);
                }
                let backfill = $backfill_state_fn(&self.pool).await?;
                let (totals, by_model) = aggregate_facts(&facts);
                let coverage = build_coverage(&totals, &backfill);
                let limit = recent_turn_limit.min(MAX_RECENT_TURNS);
                let recent_turns = facts.iter().take(limit).map(fact_row_to_public).collect();
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
    project_turn_end_pg,
    session_snapshot_pg,
    backfill_state_pg,
    decode_row_pg,
    sqlx::Postgres,
    PostgresStorage,
    sqlx::postgres::PgRow,
    "round(extract(epoch FROM ts) * 1000)::bigint",
    "(payload ->> 'kind')",
    "(payload #>> '{update,sessionUpdate}')",
    "(payload #>> '{update,status}')",
    "(payload #>> '{update,_meta,cowboyDelivery}')",
    "LEAST"
);

impl_turn_cache_backend!(
    project_events_sqlite,
    project_turn_end_sqlite,
    session_snapshot_sqlite,
    backfill_state_sqlite,
    decode_row_sqlite,
    sqlx::Sqlite,
    SqliteStorage,
    sqlx::sqlite::SqliteRow,
    "ts_ms",
    "json_extract(payload, '$.kind')",
    "json_extract(payload, '$.update.sessionUpdate')",
    "json_extract(payload, '$.update.status')",
    "json_extract(payload, '$.update._meta.cowboyDelivery')",
    "min"
);

impl Store {
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

    /// Read indexed turn-level cache efficiency metrics strictly scoped to one
    /// `(owner_user_id, provider, provider_account_id)` tuple.
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

    /// Read indexed turn-level cache efficiency metrics and recent turns for one
    /// session. Survives transcript `Clear`.
    pub async fn session_cache_metrics(
        &self,
        session_id: &str,
        provider: &str,
        provider_account_id: &str,
        recent_turn_limit: usize,
    ) -> Result<SessionCacheMetrics> {
        match &self.backend {
            StorageBackend::Postgres(db) => {
                db.session_cache_metrics(
                    session_id,
                    provider,
                    provider_account_id,
                    recent_turn_limit,
                )
                .await
            }
            StorageBackend::Sqlite(db) => {
                db.session_cache_metrics(
                    session_id,
                    provider,
                    provider_account_id,
                    recent_turn_limit,
                )
                .await
            }
        }
    }
}
