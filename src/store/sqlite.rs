//! `SQLite` implementation of the stable [`super::Store`] API.

use std::io::Read as _;
use std::str::FromStr as _;
use std::time::Duration;

use anyhow::{Context as _, Result};
use base64::Engine as _;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Connection as _, SqlitePool};

use super::*;

const SQLITE_MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

#[derive(Clone)]
pub(super) struct SqliteStorage {
    pool: SqlitePool,
    artifacts: crate::artifacts::ArtifactStore,
    database_path: Option<std::path::PathBuf>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn merge_jsonb_values(
    existing: Option<serde_json::Value>,
    update: serde_json::Value,
) -> serde_json::Value {
    match (existing, update) {
        (None, update) => update,
        (Some(serde_json::Value::Object(mut existing)), serde_json::Value::Object(update)) => {
            existing.extend(update);
            serde_json::Value::Object(existing)
        }
        (Some(serde_json::Value::Array(mut existing)), serde_json::Value::Array(update)) => {
            existing.extend(update);
            serde_json::Value::Array(existing)
        }
        (Some(serde_json::Value::Array(mut existing)), update) => {
            existing.push(update);
            serde_json::Value::Array(existing)
        }
        (Some(existing), serde_json::Value::Array(mut update)) => {
            update.insert(0, existing);
            serde_json::Value::Array(update)
        }
        (Some(existing), update) => serde_json::Value::Array(vec![existing, update]),
    }
}

#[derive(sqlx::FromRow)]
struct SqliteMachineRow {
    id: String,
    display_name: String,
    connection_mode: String,
    platform: String,
    architecture: String,
    status: String,
    inventory: serde_json::Value,
    last_seen_at_ms: Option<i64>,
    revoked_at_ms: Option<i64>,
    public_key: Option<String>,
}

#[derive(sqlx::FromRow)]
struct SqliteSessionRow {
    id: String,
    provider: String,
    provider_version: String,
    provider_generation_digest: String,
    provider_auth_generation: Option<i64>,
    provider_behavior: Option<serde_json::Value>,
    machine_id: String,
    workspace_id: Option<String>,
    workspace_name: Option<String>,
    workspace_source_path: Option<String>,
    cwd: String,
    title: String,
    origin: String,
    status: String,
    agent_session_id: Option<String>,
    auto_resume: Option<bool>,
    awaiting_user: bool,
    done: bool,
    system: bool,
    next_seq: i64,
    queue: serde_json::Value,
    drafts: serde_json::Value,
    judge_runs: serde_json::Value,
    config_options: Option<serde_json::Value>,
    config_preferences: serde_json::Value,
    mobile_review_state: serde_json::Value,
    #[allow(dead_code)]
    created_at_ms: i64,
}

const PROVIDER_USAGE_COLUMNS: &str = "machine_id, producer_id, sequence, occurred_at_ms, \
    received_at_ms, account_fingerprint, provider, agent, model, model_family, resolved_model, \
    model_revision, request_role, status, input_tokens, output_tokens, reasoning_tokens, \
    cache_hit_tokens, cache_miss_tokens, schema_version, operation, protocol, client_protocol, \
    upstream_protocol, translation_mode, thinking_mode, reasoning_effort, session_fingerprint, \
    session_attribution, traffic_source, static_prefix_fingerprint, request_prefix_fingerprint, \
    gateway_build, gateway_boot_id, cache_observation, usage_observed, completed, streaming, \
    duration_ms, request_bytes, input_item_count, tool_count, system_block_count, \
    has_previous_response_id, compatibility_fixes, request_purpose, cache_keepalive_outcome, \
    cache_keepalive_algorithm, cache_keepalive_attempt, cache_keepalive_interval_ms, \
    cache_keepalive_source_age_ms, source_request_prefix_fingerprint";

#[derive(Clone, serde::Serialize, sqlx::FromRow)]
struct ProviderUsageRecord {
    machine_id: String,
    producer_id: String,
    sequence: i64,
    occurred_at_ms: i64,
    received_at_ms: i64,
    account_fingerprint: String,
    provider: String,
    agent: String,
    model: Option<String>,
    model_family: String,
    resolved_model: Option<String>,
    model_revision: Option<String>,
    request_role: String,
    status: i32,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    cache_hit_tokens: Option<i64>,
    cache_miss_tokens: Option<i64>,
    schema_version: i32,
    operation: String,
    protocol: String,
    client_protocol: String,
    upstream_protocol: String,
    translation_mode: String,
    thinking_mode: String,
    reasoning_effort: String,
    session_fingerprint: Option<String>,
    session_attribution: String,
    traffic_source: String,
    static_prefix_fingerprint: Option<String>,
    request_prefix_fingerprint: Option<String>,
    gateway_build: Option<String>,
    gateway_boot_id: Option<String>,
    cache_observation: String,
    usage_observed: Option<bool>,
    completed: Option<bool>,
    streaming: Option<bool>,
    duration_ms: Option<i64>,
    request_bytes: Option<i64>,
    input_item_count: Option<i64>,
    tool_count: Option<i64>,
    system_block_count: Option<i64>,
    has_previous_response_id: Option<bool>,
    compatibility_fixes: Option<i64>,
    request_purpose: String,
    cache_keepalive_outcome: String,
    cache_keepalive_algorithm: Option<String>,
    cache_keepalive_attempt: Option<i64>,
    cache_keepalive_interval_ms: Option<i64>,
    cache_keepalive_source_age_ms: Option<i64>,
    source_request_prefix_fingerprint: Option<String>,
}

impl ProviderUsageRecord {
    fn model(&self) -> &str {
        self.model.as_deref().unwrap_or_default()
    }

    fn resolved_model(&self) -> &str {
        self.resolved_model.as_deref().unwrap_or_default()
    }

    fn billing_model(&self) -> &str {
        self.resolved_model
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.model())
    }

    fn model_revision(&self) -> &str {
        self.model_revision.as_deref().unwrap_or_default()
    }

    fn gateway_build(&self) -> &str {
        self.gateway_build.as_deref().unwrap_or_default()
    }
}

fn add_optional_metric(target: &mut i64, value: Option<i64>) {
    *target = target.saturating_add(value.unwrap_or_default());
}

#[allow(clippy::too_many_lines)]
fn usage_aggregate_for(record: &ProviderUsageRecord) -> UsageAggregate {
    let mut aggregate = UsageAggregate::default();
    if record.request_purpose == "interactive" {
        aggregate.requests = 1;
        aggregate.errors = i64::from(record.status >= 400);
        aggregate.blocking_errors = i64::from(
            (400..=499).contains(&record.status) && !matches!(record.status, 408 | 425 | 429 | 499),
        );
        aggregate.transient_errors =
            i64::from(matches!(record.status, 408 | 425 | 429 | 499) || record.status >= 500);
        aggregate.completed_requests = i64::from(record.completed == Some(true));
        aggregate.completion_observations = i64::from(record.completed.is_some());
        aggregate.usage_observations = i64::from(record.usage_observed == Some(true));
        add_optional_metric(&mut aggregate.input_tokens, record.input_tokens);
        add_optional_metric(&mut aggregate.output_tokens, record.output_tokens);
        add_optional_metric(&mut aggregate.reasoning_tokens, record.reasoning_tokens);
        let measured_cache = matches!(record.cache_observation.as_str(), "explicit" | "derived");
        if measured_cache {
            add_optional_metric(&mut aggregate.cache_hit_tokens, record.cache_hit_tokens);
            add_optional_metric(&mut aggregate.cache_miss_tokens, record.cache_miss_tokens);
            aggregate.cache_observations =
                i64::from(record.cache_hit_tokens.is_some() && record.cache_miss_tokens.is_some());
            if let Some((hit, miss)) = record.cache_hit_tokens.zip(record.cache_miss_tokens) {
                let total = i128::from(hit) + i128::from(miss);
                aggregate.cold_cache_requests =
                    i64::from(total > 0 && i128::from(hit) * 10 < total);
                aggregate.hot_cache_requests =
                    i64::from(total > 0 && i128::from(hit) * 10 >= 9 * total);
            }
        }
        aggregate.explicit_cache_observations = i64::from(record.cache_observation == "explicit");
        aggregate.derived_cache_observations = i64::from(record.cache_observation == "derived");
        aggregate.absent_cache_observations = i64::from(record.cache_observation == "absent");
        add_optional_metric(&mut aggregate.duration_ms, record.duration_ms);
        aggregate.duration_observations = i64::from(record.duration_ms.is_some());
        add_optional_metric(&mut aggregate.request_bytes, record.request_bytes);
        aggregate.request_shape_observations = i64::from(record.request_bytes.is_some());
        add_optional_metric(&mut aggregate.input_item_count, record.input_item_count);
        add_optional_metric(&mut aggregate.tool_count, record.tool_count);
        add_optional_metric(&mut aggregate.system_block_count, record.system_block_count);
        aggregate.previous_response_requests =
            i64::from(record.has_previous_response_id == Some(true));
        add_optional_metric(
            &mut aggregate.compatibility_fixes,
            record.compatibility_fixes,
        );
        aggregate.streaming_requests = i64::from(record.streaming == Some(true));
    } else if record.request_purpose == "cache_keepalive" {
        aggregate.cache_keepalive_requests = 1;
        aggregate.cache_keepalive_hits = i64::from(record.cache_keepalive_outcome == "hit");
        aggregate.cache_keepalive_misses = i64::from(record.cache_keepalive_outcome == "miss");
        aggregate.cache_keepalive_partials = i64::from(record.cache_keepalive_outcome == "partial");
        aggregate.cache_keepalive_retryable_errors =
            i64::from(record.cache_keepalive_outcome == "retryable_error");
        aggregate.cache_keepalive_terminal_errors =
            i64::from(record.cache_keepalive_outcome == "terminal_error");
        aggregate.cache_keepalive_preemptions =
            i64::from(record.cache_keepalive_outcome == "preempted");
        aggregate.cache_keepalive_usage_observations =
            i64::from(record.usage_observed == Some(true));
        add_optional_metric(
            &mut aggregate.cache_keepalive_input_tokens,
            record.input_tokens,
        );
        add_optional_metric(
            &mut aggregate.cache_keepalive_output_tokens,
            record.output_tokens,
        );
        add_optional_metric(
            &mut aggregate.cache_keepalive_reasoning_tokens,
            record.reasoning_tokens,
        );
        if matches!(record.cache_observation.as_str(), "explicit" | "derived") {
            add_optional_metric(
                &mut aggregate.cache_keepalive_hit_tokens,
                record.cache_hit_tokens,
            );
            add_optional_metric(
                &mut aggregate.cache_keepalive_miss_tokens,
                record.cache_miss_tokens,
            );
        }
        add_optional_metric(
            &mut aggregate.cache_keepalive_duration_ms,
            record.duration_ms,
        );
        aggregate.cache_keepalive_duration_observations = i64::from(record.duration_ms.is_some());
        add_optional_metric(
            &mut aggregate.cache_keepalive_interval_ms,
            record.cache_keepalive_interval_ms,
        );
        aggregate.cache_keepalive_interval_observations =
            i64::from(record.cache_keepalive_interval_ms.is_some());
        add_optional_metric(
            &mut aggregate.cache_keepalive_source_age_ms,
            record.cache_keepalive_source_age_ms,
        );
        aggregate.cache_keepalive_source_age_observations =
            i64::from(record.cache_keepalive_source_age_ms.is_some());
    }
    aggregate
}

fn add_usage_record(breakdown: &mut ProviderUsageBreakdown, record: &ProviderUsageRecord) {
    let aggregate = usage_aggregate_for(record);
    breakdown.summary.add(&aggregate);
    for (map, key) in [
        (&mut breakdown.by_agent, record.agent.clone()),
        (&mut breakdown.by_machine, record.machine_id.clone()),
        (&mut breakdown.by_operation, record.operation.clone()),
        (&mut breakdown.by_model, record.model().to_owned()),
        (
            &mut breakdown.by_resolved_model,
            record.resolved_model().to_owned(),
        ),
        (
            &mut breakdown.by_billing_model,
            record.billing_model().to_owned(),
        ),
        (
            &mut breakdown.by_model_revision,
            record.model_revision().to_owned(),
        ),
        (&mut breakdown.by_model_family, record.model_family.clone()),
        (&mut breakdown.by_request_role, record.request_role.clone()),
        (&mut breakdown.by_protocol, record.protocol.clone()),
        (
            &mut breakdown.by_client_protocol,
            record.client_protocol.clone(),
        ),
        (
            &mut breakdown.by_upstream_protocol,
            record.upstream_protocol.clone(),
        ),
        (
            &mut breakdown.by_translation_mode,
            record.translation_mode.clone(),
        ),
        (
            &mut breakdown.by_thinking_mode,
            record.thinking_mode.clone(),
        ),
        (
            &mut breakdown.by_reasoning_effort,
            record.reasoning_effort.clone(),
        ),
        (
            &mut breakdown.by_session_attribution,
            record.session_attribution.clone(),
        ),
        (
            &mut breakdown.by_traffic_source,
            record.traffic_source.clone(),
        ),
        (
            &mut breakdown.by_gateway_build,
            record.gateway_build().to_owned(),
        ),
        (
            &mut breakdown.by_schema_version,
            record.schema_version.to_string(),
        ),
    ] {
        map.entry(key).or_default().add(&aggregate);
    }
    for (map, outer, inner) in [
        (
            &mut breakdown.by_agent_model,
            record.agent.clone(),
            record.model().to_owned(),
        ),
        (
            &mut breakdown.by_agent_billing_model,
            record.agent.clone(),
            record.billing_model().to_owned(),
        ),
        (
            &mut breakdown.by_agent_model_family,
            record.agent.clone(),
            record.model_family.clone(),
        ),
        (
            &mut breakdown.by_agent_request_role,
            record.agent.clone(),
            record.request_role.clone(),
        ),
        (
            &mut breakdown.by_agent_operation,
            record.agent.clone(),
            record.operation.clone(),
        ),
    ] {
        map.entry(outer)
            .or_default()
            .entry(inner)
            .or_default()
            .add(&aggregate);
    }
}

async fn insert_provider_usage_event(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    machine_id: &str,
    producer_id: &str,
    event: &crate::machine_protocol::ProviderUsageEvent,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO provider_usage_events (machine_id, producer_id, sequence, occurred_at_ms, \
         account_fingerprint, provider, agent, model, model_family, resolved_model, \
         model_revision, request_role, status, input_tokens, output_tokens, reasoning_tokens, \
         cache_hit_tokens, cache_miss_tokens, schema_version, operation, protocol, \
         client_protocol, upstream_protocol, translation_mode, thinking_mode, reasoning_effort, \
         session_fingerprint, session_attribution, traffic_source, static_prefix_fingerprint, \
         request_prefix_fingerprint, gateway_build, gateway_boot_id, cache_observation, \
         usage_observed, completed, streaming, duration_ms, request_bytes, input_item_count, \
         tool_count, system_block_count, has_previous_response_id, compatibility_fixes, \
         request_purpose, cache_keepalive_outcome, cache_keepalive_algorithm, \
         cache_keepalive_attempt, cache_keepalive_interval_ms, cache_keepalive_source_age_ms, \
         source_request_prefix_fingerprint) VALUES ( \
         ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
         ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT DO NOTHING",
    )
    .bind(machine_id)
    .bind(producer_id)
    .bind(i64::try_from(event.sequence).context("usage sequence overflow")?)
    .bind(event.occurred_at_ms)
    .bind(&event.account_fingerprint)
    .bind(&event.provider)
    .bind(&event.agent)
    .bind(&event.model)
    .bind(&event.model_family)
    .bind(&event.resolved_model)
    .bind(&event.model_revision)
    .bind(&event.request_role)
    .bind(i32::from(event.status))
    .bind(provider_usage_metric(event.input_tokens)?)
    .bind(provider_usage_metric(event.output_tokens)?)
    .bind(provider_usage_metric(event.reasoning_tokens)?)
    .bind(provider_usage_metric(event.cache_hit_tokens)?)
    .bind(provider_usage_metric(event.cache_miss_tokens)?)
    .bind(i32::from(event.schema_version))
    .bind(&event.operation)
    .bind(&event.protocol)
    .bind(&event.client_protocol)
    .bind(&event.upstream_protocol)
    .bind(&event.translation_mode)
    .bind(&event.thinking_mode)
    .bind(&event.reasoning_effort)
    .bind(&event.session_fingerprint)
    .bind(&event.session_attribution)
    .bind(&event.traffic_source)
    .bind(&event.static_prefix_fingerprint)
    .bind(&event.request_prefix_fingerprint)
    .bind(&event.gateway_build)
    .bind(&event.gateway_boot_id)
    .bind(&event.cache_observation)
    .bind(event.usage_observed)
    .bind(event.completed)
    .bind(event.streaming)
    .bind(provider_usage_metric(event.duration_ms)?)
    .bind(provider_usage_metric(event.request_bytes)?)
    .bind(provider_usage_metric(event.input_item_count)?)
    .bind(provider_usage_metric(event.tool_count)?)
    .bind(provider_usage_metric(event.system_block_count)?)
    .bind(event.has_previous_response_id)
    .bind(provider_usage_metric(event.compatibility_fixes)?)
    .bind(&event.request_purpose)
    .bind(&event.cache_keepalive_outcome)
    .bind(&event.cache_keepalive_algorithm)
    .bind(provider_usage_metric(event.cache_keepalive_attempt)?)
    .bind(provider_usage_metric(event.cache_keepalive_interval_ms)?)
    .bind(provider_usage_metric(event.cache_keepalive_source_age_ms)?)
    .bind(&event.source_request_prefix_fingerprint)
    .execute(&mut **transaction)
    .await
    .context("insert SQLite provider usage event")?;
    Ok(())
}

async fn load_provider_usage_records(
    pool: &SqlitePool,
    provider: &str,
    from_ms: i64,
    to_ms: i64,
    agent: Option<&str>,
    model_family: Option<&str>,
) -> Result<Vec<ProviderUsageRecord>> {
    let query = format!(
        "SELECT {PROVIDER_USAGE_COLUMNS} FROM provider_usage_events \
         WHERE provider = ?1 AND occurred_at_ms >= ?2 AND occurred_at_ms <= ?3 \
         AND (?4 IS NULL OR agent = ?4) AND (?5 IS NULL OR model_family = ?5) \
         ORDER BY occurred_at_ms, sequence"
    );
    sqlx::query_as(&query)
        .bind(provider)
        .bind(from_ms)
        .bind(to_ms)
        .bind(agent)
        .bind(model_family)
        .fetch_all(pool)
        .await
        .context("load SQLite provider usage records")
}

fn usage_breakdown(records: &[ProviderUsageRecord]) -> ProviderUsageBreakdown {
    let mut breakdown = ProviderUsageBreakdown::default();
    for record in records {
        add_usage_record(&mut breakdown, record);
    }
    breakdown
}

fn usage_day(timestamp_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_ms).map_or_else(
        || "1970-01-01".to_owned(),
        |value| value.format("%F").to_string(),
    )
}

fn daily_usage(records: &[ProviderUsageRecord]) -> Vec<serde_json::Value> {
    let mut days = std::collections::BTreeMap::<String, UsageAggregate>::new();
    for record in records {
        days.entry(usage_day(record.occurred_at_ms))
            .or_default()
            .add(&usage_aggregate_for(record));
    }
    days.into_iter()
        .map(|(day, totals)| serde_json::json!({ "day": day, "totals": totals }))
        .collect()
}

fn timeline_usage(records: &[ProviderUsageRecord], bucket: &str) -> Result<Vec<serde_json::Value>> {
    let bucket_ms = match bucket {
        "hour" => 3_600_000,
        "day" => 86_400_000,
        _ => anyhow::bail!("invalid provider usage timeline bucket"),
    };
    let mut buckets = std::collections::BTreeMap::<i64, UsageAggregate>::new();
    for record in records {
        let start_ms = record.occurred_at_ms.div_euclid(bucket_ms) * bucket_ms;
        buckets
            .entry(start_ms)
            .or_default()
            .add(&usage_aggregate_for(record));
    }
    Ok(buckets
        .into_iter()
        .map(|(start_ms, totals)| serde_json::json!({ "startMs": start_ms, "totals": totals }))
        .collect())
}

fn cache_is_high(record: &ProviderUsageRecord) -> bool {
    record
        .cache_hit_tokens
        .zip(record.cache_miss_tokens)
        .is_some_and(|(hit, miss)| {
            let total = i128::from(hit) + i128::from(miss);
            total > 0 && i128::from(hit) * 10 >= 9 * total
        })
}

fn cache_is_low(record: &ProviderUsageRecord) -> bool {
    record
        .cache_hit_tokens
        .zip(record.cache_miss_tokens)
        .is_some_and(|(hit, miss)| {
            let total = i128::from(hit) + i128::from(miss);
            total > 0 && i128::from(hit) * 10 < total
        })
}

fn low_hit_cause(
    current: &ProviderUsageRecord,
    previous: Option<&ProviderUsageRecord>,
) -> &'static str {
    if current.schema_version < 3 {
        return "legacy_unattributed";
    }
    if current.session_fingerprint.is_none() && current.has_previous_response_id == Some(true) {
        return "session_lineage_unavailable";
    }
    if current.session_fingerprint.is_none() {
        return "unattributed";
    }
    if current.session_attribution == "prefix_root" {
        return "prefix_lineage_ambiguous";
    }
    let Some(previous) = previous else {
        return "first_session_observation";
    };
    if current.operation == "compact" {
        "client_compaction"
    } else if current
        .occurred_at_ms
        .saturating_sub(previous.occurred_at_ms)
        >= 21_600_000
        && cache_is_high(previous)
    {
        "probable_cache_eviction"
    } else if current.gateway_build != previous.gateway_build {
        "gateway_build_changed"
    } else if current.gateway_boot_id != previous.gateway_boot_id {
        "post_gateway_restart"
    } else if current.model_family != previous.model_family {
        "model_changed"
    } else if current.model_revision != previous.model_revision {
        "model_revision_changed"
    } else if current.request_role != previous.request_role {
        "request_role_changed"
    } else if current.upstream_protocol != previous.upstream_protocol {
        "protocol_changed"
    } else if current.translation_mode != previous.translation_mode {
        "translation_changed"
    } else if current.thinking_mode != previous.thinking_mode
        || current.reasoning_effort != previous.reasoning_effort
    {
        "reasoning_configuration_changed"
    } else if current.compatibility_fixes.is_some_and(|value| value > 0) {
        "compatibility_rewrite"
    } else if current.static_prefix_fingerprint != previous.static_prefix_fingerprint {
        "static_prefix_changed"
    } else if (current.agent != "codex" || current.has_previous_response_id != Some(true))
        && current
            .input_item_count
            .zip(previous.input_item_count)
            .is_some_and(|(current, previous)| current < previous)
    {
        "history_rewrite"
    } else if current.request_prefix_fingerprint.is_some()
        && current.request_prefix_fingerprint == previous.request_prefix_fingerprint
        && cache_is_high(previous)
    {
        "unexpected_exact_prefix_miss"
    } else {
        "unexplained_low_hit"
    }
}

fn low_hit_breakdown(
    records: &[ProviderUsageRecord],
    from_ms: i64,
    model_family: Option<&str>,
) -> LowHitBreakdown {
    type SessionKey = (String, String, String, String, String);
    let mut previous = std::collections::HashMap::<SessionKey, ProviderUsageRecord>::new();
    let mut result = LowHitBreakdown::default();
    for record in records {
        let lineage = record
            .session_fingerprint
            .clone()
            .unwrap_or_else(|| format!("{}:{}", record.producer_id, record.sequence));
        let key = (
            record.machine_id.clone(),
            record.producer_id.clone(),
            record.account_fingerprint.clone(),
            record.agent.clone(),
            lineage,
        );
        let prior = previous.get(&key);
        let eligible = record.occurred_at_ms >= from_ms
            && model_family.is_none_or(|value| record.model_family == value)
            && matches!(record.cache_observation.as_str(), "explicit" | "derived")
            && record.input_tokens.is_some_and(|value| value >= 8_000)
            && cache_is_low(record);
        if eligible {
            let cause = low_hit_cause(record, prior).to_owned();
            let model = record.billing_model().to_owned();
            let aggregate = usage_aggregate_for(record);
            result.summary.add(&aggregate);
            result
                .by_cause
                .entry(cause.clone())
                .or_default()
                .add(&aggregate);
            result
                .by_cause_model
                .entry(cause)
                .or_default()
                .entry(model)
                .or_default()
                .add(&aggregate);
        }
        previous.insert(key, record.clone());
    }
    result
}

#[derive(sqlx::FromRow)]
struct ProviderCoverageRow {
    machine_id: String,
    producer_id: String,
    agent: String,
    last_sequence: i64,
    last_received_at_ms: i64,
}

#[derive(sqlx::FromRow)]
struct RuntimeIncidentWithProvider {
    id: String,
    occurred_at_ms: i64,
    updated_at_ms: i64,
    source: String,
    classification: String,
    severity: String,
    state: String,
    summary: String,
    fingerprint: String,
    session_id: Option<String>,
    client_id: Option<String>,
    machine_id: Option<String>,
    trace_id: Option<String>,
    build: Option<String>,
    evidence_start_ms: i64,
    evidence_end_ms: i64,
    detail: serde_json::Value,
    recovered_at_ms: Option<i64>,
    recovery_outcome: Option<String>,
    provider: Option<String>,
}

impl RuntimeIncidentWithProvider {
    fn into_parts(self) -> (RuntimeIncident, Option<String>) {
        (
            RuntimeIncident {
                id: self.id,
                occurred_at_ms: self.occurred_at_ms,
                updated_at_ms: self.updated_at_ms,
                source: self.source,
                classification: self.classification,
                severity: self.severity,
                state: self.state,
                summary: self.summary,
                fingerprint: self.fingerprint,
                session_id: self.session_id,
                client_id: self.client_id,
                machine_id: self.machine_id,
                trace_id: self.trace_id,
                build: self.build,
                evidence_start_ms: self.evidence_start_ms,
                evidence_end_ms: self.evidence_end_ms,
                detail: self.detail,
                recovered_at_ms: self.recovered_at_ms,
                recovery_outcome: self.recovery_outcome,
            },
            self.provider,
        )
    }
}

async fn provider_usage_coverage(
    pool: &SqlitePool,
    provider: &str,
    records: &[ProviderUsageRecord],
    agent: Option<&str>,
) -> Result<Vec<serde_json::Value>> {
    let covered = records
        .iter()
        .map(|record| (record.machine_id.as_str(), record.producer_id.as_str()))
        .collect::<std::collections::HashSet<_>>();
    let rows: Vec<ProviderCoverageRow> = sqlx::query_as(
        "SELECT machine_id, producer_id, agent, last_sequence, last_received_at_ms \
         FROM provider_usage_producers WHERE provider = ?1 AND (?2 IS NULL OR agent = ?2) \
         ORDER BY machine_id, agent",
    )
    .bind(provider)
    .bind(agent)
    .fetch_all(pool)
    .await
    .context("load SQLite provider usage coverage")?;
    Ok(rows
        .into_iter()
        .filter(|row| covered.contains(&(row.machine_id.as_str(), row.producer_id.as_str())))
        .map(|row| {
            serde_json::json!({
                "machine": row.machine_id,
                "agent": row.agent,
                "lastSequence": row.last_sequence,
                "lastReceivedAtMs": row.last_received_at_ms,
            })
        })
        .collect())
}

async fn recent_provider_usage_coverage(
    pool: &SqlitePool,
    provider: &str,
    since_ms: i64,
) -> Result<Vec<serde_json::Value>> {
    let rows: Vec<ProviderCoverageRow> = sqlx::query_as(
        "SELECT machine_id, producer_id, agent, last_sequence, last_received_at_ms \
         FROM provider_usage_producers WHERE provider = ?1 AND last_received_at_ms >= ?2 \
         ORDER BY machine_id, agent",
    )
    .bind(provider)
    .bind(since_ms)
    .fetch_all(pool)
    .await
    .context("load recent SQLite provider usage coverage")?;
    Ok(rows
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "machine": row.machine_id,
                "agent": row.agent,
                "lastSequence": row.last_sequence,
                "lastReceivedAtMs": row.last_received_at_ms,
            })
        })
        .collect())
}

fn provider_record_json(record: &ProviderUsageRecord) -> serde_json::Value {
    serde_json::to_value(record).unwrap_or_else(|_| serde_json::json!({}))
}

fn provider_record_session_key(record: &ProviderUsageRecord) -> Option<(&str, &str, &str, &str)> {
    Some((
        record.machine_id.as_str(),
        record.producer_id.as_str(),
        record.account_fingerprint.as_str(),
        record.session_fingerprint.as_deref()?,
    ))
}

fn diagnostic_lineage_eligible(record: &ProviderUsageRecord) -> bool {
    record.schema_version >= 3
        && record.request_purpose == "interactive"
        && record.session_attribution != "prefix_root"
        && record.status < 400
        && record.session_fingerprint.is_some()
        && record.static_prefix_fingerprint.is_some()
        && matches!(record.cache_observation.as_str(), "explicit" | "derived")
        && record.input_tokens.is_some_and(|tokens| tokens >= 8_000)
        && record
            .cache_hit_tokens
            .zip(record.cache_miss_tokens)
            .is_some_and(|(hit, miss)| hit.saturating_add(miss) > 0)
}

fn diagnostic_cache_transition(
    current: &ProviderUsageRecord,
    previous: &ProviderUsageRecord,
) -> bool {
    diagnostic_lineage_eligible(current)
        && diagnostic_lineage_eligible(previous)
        && current.occurred_at_ms >= previous.occurred_at_ms
        && current.occurred_at_ms - previous.occurred_at_ms <= 30 * 60 * 1_000
        && cache_is_low(current)
        && cache_is_high(previous)
}

fn diagnostic_cache_cause(
    current: &ProviderUsageRecord,
    previous: &ProviderUsageRecord,
    intervening_provider_error: bool,
) -> &'static str {
    let current_json = provider_record_json(current);
    let previous_json = provider_record_json(previous);
    cache_transition_cause(&current_json, &previous_json, intervening_provider_error)
}

fn diagnostic_cache_summary(
    current: &ProviderUsageRecord,
    previous: &ProviderUsageRecord,
) -> String {
    let current_json = provider_record_json(current);
    let previous_json = provider_record_json(previous);
    match (
        cache_rate_label(&previous_json),
        cache_rate_label(&current_json),
    ) {
        (Some(previous), Some(current)) => {
            format!("Cache hit rate fell from {previous}% to {current}% within 30 minutes")
        }
        _ => "Active-session cache hit rate fell below 10%".to_owned(),
    }
}

fn provider_error_summary(record: &ProviderUsageRecord) -> String {
    if provider_status_impact(i64::from(record.status)).1 {
        format!(
            "{} request was blocked with HTTP status {}",
            diagnostic_title(&record.agent),
            record.status
        )
    } else {
        format!(
            "{} request attempt received retryable HTTP status {}",
            diagnostic_title(&record.agent),
            record.status
        )
    }
}

fn provider_error_classification(status: i32) -> &'static str {
    match status {
        401 | 403 => "provider_authentication_failure",
        402 => "provider_balance_exhausted",
        value if (400..=499).contains(&value) && !matches!(value, 408 | 425 | 429 | 499) => {
            "provider_request_blocked"
        }
        _ => "provider_retryable_failure",
    }
}

fn provider_error_severity(status: i32) -> &'static str {
    if matches!(status, 401..=403) {
        "critical"
    } else if (400..=499).contains(&status) && !matches!(status, 408 | 425 | 429 | 499) {
        "error"
    } else {
        "warning"
    }
}

fn keepalive_title(outcome: &str) -> &'static str {
    match outcome {
        "hit" => "DeepSeek cache protected",
        "miss" => "DeepSeek keepalive cache miss",
        "partial" => "DeepSeek keepalive partial hit",
        "retryable_error" => "DeepSeek keepalive will retry",
        "terminal_error" => "DeepSeek keepalive stopped",
        "preempted" => "DeepSeek keepalive preempted",
        _ => "DeepSeek keepalive observation",
    }
}

fn keepalive_summary(record: &ProviderUsageRecord) -> String {
    match record.cache_keepalive_outcome.as_str() {
        "hit" => format!(
            "Protected {} cached tokens",
            record.cache_hit_tokens.unwrap_or_default()
        ),
        "miss" => format!(
            "Cache expired after {} ms; protection stopped",
            record.cache_keepalive_source_age_ms.unwrap_or_default()
        ),
        "partial" => format!(
            "Cache hit fell below 90% after {} ms; protection stopped",
            record.cache_keepalive_source_age_ms.unwrap_or_default()
        ),
        "retryable_error" => format!(
            "Retryable HTTP {}; one bounded retry is allowed",
            record.status
        ),
        "terminal_error" => format!(
            "HTTP {} made this snapshot ineligible for further keepalives",
            record.status
        ),
        "preempted" => "A real agent request preempted the background keepalive".to_owned(),
        _ => "Unknown cache-protection outcome".to_owned(),
    }
}

fn keepalive_severity(record: &ProviderUsageRecord) -> &'static str {
    if record.cache_keepalive_outcome == "terminal_error" && matches!(record.status, 401..=403) {
        "critical"
    } else if matches!(
        record.cache_keepalive_outcome.as_str(),
        "miss" | "partial" | "retryable_error" | "terminal_error"
    ) {
        "warning"
    } else {
        "info"
    }
}

fn keepalive_state(outcome: &str) -> &'static str {
    match outcome {
        "hit" => "succeeded",
        "preempted" => "recovered",
        "retryable_error" => "retrying",
        _ => "failed",
    }
}

fn matches_diagnostic_filter(summary: &DiagnosticLogSummary, filter: &DiagnosticLogFilter) -> bool {
    (filter.kinds.is_empty() || filter.kinds.contains(&summary.kind))
        && (filter.severities.is_empty() || filter.severities.contains(&summary.severity))
        && (filter.states.is_empty() || filter.states.contains(&summary.state))
        && (filter.agents.is_empty()
            || summary
                .agent
                .as_ref()
                .is_some_and(|agent| filter.agents.contains(agent)))
        && filter
            .session_ref
            .as_ref()
            .is_none_or(|session| summary.session_ref.as_ref() == Some(session))
        && filter.cursor_ms.is_none_or(|cursor_ms| {
            summary.occurred_at_ms < cursor_ms
                || (summary.occurred_at_ms == cursor_ms
                    && filter
                        .cursor_id
                        .as_ref()
                        .is_none_or(|cursor_id| summary.id < *cursor_id))
        })
}

fn runtime_diagnostic_detail(id: &str, incident: RuntimeIncident) -> DiagnosticLogDetail {
    let mut identity = vec![diagnostic_field("Log ID", id, true)];
    optional_diagnostic_field(
        &mut identity,
        "Session ID",
        incident.session_id.clone(),
        true,
    );
    optional_diagnostic_field(
        &mut identity,
        "Machine ID",
        incident.machine_id.clone(),
        true,
    );
    optional_diagnostic_field(&mut identity, "Client ID", incident.client_id.clone(), true);
    optional_diagnostic_field(&mut identity, "Trace ID", incident.trace_id.clone(), true);
    let mut lifecycle = vec![
        diagnostic_field("Source", incident.source.clone(), false),
        diagnostic_field("Classification", incident.classification.clone(), false),
        diagnostic_field("Severity", incident.severity.clone(), false),
        diagnostic_field("State", incident.state.clone(), false),
        diagnostic_field("Fingerprint", incident.fingerprint.clone(), true),
        diagnostic_field(
            "Evidence start",
            incident.evidence_start_ms.to_string(),
            false,
        ),
        diagnostic_field("Evidence end", incident.evidence_end_ms.to_string(), false),
    ];
    optional_diagnostic_field(&mut lifecycle, "Build", incident.build.clone(), true);
    optional_diagnostic_field(
        &mut lifecycle,
        "Recovered at",
        incident.recovered_at_ms.map(|value| value.to_string()),
        false,
    );
    optional_diagnostic_field(
        &mut lifecycle,
        "Recovery outcome",
        incident.recovery_outcome.clone(),
        false,
    );
    DiagnosticLogDetail {
        id: id.to_owned(),
        kind: "session_error".to_owned(),
        occurred_at_ms: incident.occurred_at_ms,
        title: diagnostic_title(&incident.classification),
        summary: incident.summary,
        sections: vec![
            DiagnosticLogSection {
                title: "Identity".to_owned(),
                fields: identity,
            },
            DiagnosticLogSection {
                title: "Lifecycle".to_owned(),
                fields: lifecycle,
            },
        ],
        evidence: (!incident
            .detail
            .as_object()
            .is_some_and(serde_json::Map::is_empty))
        .then_some(incident.detail),
    }
}

#[allow(clippy::too_many_lines)] // mirrors the existing backend-neutral diagnostic detail shape
fn provider_diagnostic_detail(
    id: &str,
    kind: &str,
    record: &ProviderUsageRecord,
    previous: Option<&ProviderUsageRecord>,
    intervening_provider_errors: i64,
) -> Option<DiagnosticLogDetail> {
    let current = provider_record_json(record);
    let previous_json = previous.map_or_else(|| serde_json::json!({}), provider_record_json);
    let gap_ms = previous.map(|value| record.occurred_at_ms.saturating_sub(value.occurred_at_ms));
    let evidence = serde_json::json!({
        "current": current,
        "previous": previous_json,
        "gap_ms": gap_ms,
        "intervening_provider_errors": intervening_provider_errors,
    });
    if kind != "cache_keepalive"
        && !provider_detail_matches_kind(
            kind,
            evidence.get("current")?,
            evidence.get("previous")?,
            gap_ms,
        )
    {
        return None;
    }
    let mut identity = vec![
        diagnostic_field("Log ID", id, true),
        diagnostic_field("Machine ID", &record.machine_id, true),
        diagnostic_field("Producer ID", &record.producer_id, true),
        diagnostic_field("Sequence", record.sequence.to_string(), true),
    ];
    for (label, key, copyable) in [
        ("Provider", "provider", false),
        ("Agent", "agent", false),
        ("Model", "model", false),
        ("Resolved model", "resolved_model", false),
        ("Model family", "model_family", false),
        ("Model revision", "model_revision", true),
        ("Session fingerprint", "session_fingerprint", true),
        ("Account fingerprint", "account_fingerprint", true),
    ] {
        optional_diagnostic_field(&mut identity, label, json_scalar(&current, key), copyable);
    }
    if kind == "cache_keepalive" {
        if record.request_purpose != "cache_keepalive" {
            return None;
        }
        let mut protection = Vec::new();
        for (label, key, copyable) in [
            ("Outcome", "cache_keepalive_outcome", false),
            ("Algorithm", "cache_keepalive_algorithm", false),
            ("Attempt", "cache_keepalive_attempt", false),
            (
                "Scheduled interval ms",
                "cache_keepalive_interval_ms",
                false,
            ),
            ("Source age ms", "cache_keepalive_source_age_ms", false),
            ("HTTP status", "status", false),
            ("Duration ms", "duration_ms", false),
            ("Input tokens", "input_tokens", false),
            ("Output tokens", "output_tokens", false),
            ("Cache hit tokens", "cache_hit_tokens", false),
            ("Cache miss tokens", "cache_miss_tokens", false),
            (
                "Source request prefix",
                "source_request_prefix_fingerprint",
                true,
            ),
            ("Replay request prefix", "request_prefix_fingerprint", true),
            ("Static prefix", "static_prefix_fingerprint", true),
            ("Gateway build", "gateway_build", true),
            ("Gateway boot", "gateway_boot_id", true),
        ] {
            optional_diagnostic_field(&mut protection, label, json_scalar(&current, key), copyable);
        }
        return Some(DiagnosticLogDetail {
            id: id.to_owned(),
            kind: "cache_anomaly".to_owned(),
            occurred_at_ms: record.occurred_at_ms,
            title: keepalive_title(&record.cache_keepalive_outcome).to_owned(),
            summary: keepalive_summary(record),
            sections: vec![
                DiagnosticLogSection {
                    title: "Identity".to_owned(),
                    fields: identity,
                },
                DiagnosticLogSection {
                    title: "Cache protection".to_owned(),
                    fields: protection,
                },
            ],
            evidence: Some(evidence),
        });
    }
    let cause = (kind == "cache_anomaly")
        .then(|| cache_transition_cause(&current, &previous_json, intervening_provider_errors > 0));
    let title = cause.map_or_else(
        || format!("DeepSeek HTTP {}", record.status),
        |cause| cache_transition_title(cause).to_owned(),
    );
    let summary = if kind == "cache_anomaly" {
        previous.map_or_else(
            || "Active-session cache hit rate fell below 10%".to_owned(),
            |previous| diagnostic_cache_summary(record, previous),
        )
    } else {
        provider_error_summary(record)
    };
    let mut request = Vec::new();
    if let Some(cause) = cause {
        request.push(diagnostic_field("Cache classification", cause, false));
    } else {
        request.push(diagnostic_field(
            "Observed impact",
            provider_status_impact(i64::from(record.status)).0,
            false,
        ));
    }
    for (label, key) in [
        ("HTTP status", "status"),
        ("Operation", "operation"),
        ("Request role", "request_role"),
        ("Client protocol", "client_protocol"),
        ("Upstream protocol", "upstream_protocol"),
        ("Translation mode", "translation_mode"),
        ("Thinking mode", "thinking_mode"),
        ("Reasoning effort", "reasoning_effort"),
        ("Duration ms", "duration_ms"),
        ("Completed", "completed"),
        ("Streaming", "streaming"),
        ("Input tokens", "input_tokens"),
        ("Output tokens", "output_tokens"),
        ("Reasoning tokens", "reasoning_tokens"),
        ("Cache hit tokens", "cache_hit_tokens"),
        ("Cache miss tokens", "cache_miss_tokens"),
        ("Request bytes", "request_bytes"),
        ("Input items", "input_item_count"),
        ("Tools", "tool_count"),
        ("System blocks", "system_block_count"),
        ("Compatibility fixes", "compatibility_fixes"),
    ] {
        optional_diagnostic_field(&mut request, label, json_scalar(&current, key), false);
    }
    let mut cache_identity = Vec::new();
    for (label, key) in [
        ("Static prefix", "static_prefix_fingerprint"),
        ("Request prefix", "request_prefix_fingerprint"),
        ("Gateway build", "gateway_build"),
        ("Gateway boot", "gateway_boot_id"),
    ] {
        optional_diagnostic_field(&mut cache_identity, label, json_scalar(&current, key), true);
        optional_diagnostic_field(
            &mut cache_identity,
            &format!("Previous {label}"),
            json_scalar(&previous_json, key),
            true,
        );
    }
    optional_diagnostic_field(
        &mut cache_identity,
        "Gap ms",
        gap_ms.map(|value| value.to_string()),
        false,
    );
    optional_diagnostic_field(
        &mut cache_identity,
        "Intervening provider errors",
        (intervening_provider_errors > 0).then(|| intervening_provider_errors.to_string()),
        false,
    );
    let mut sections = vec![
        DiagnosticLogSection {
            title: "Identity".to_owned(),
            fields: identity,
        },
        DiagnosticLogSection {
            title: "Request".to_owned(),
            fields: request,
        },
    ];
    if !cache_identity.is_empty() {
        sections.push(DiagnosticLogSection {
            title: "Cache lineage".to_owned(),
            fields: cache_identity,
        });
    }
    Some(DiagnosticLogDetail {
        id: id.to_owned(),
        kind: kind.to_owned(),
        occurred_at_ms: record.occurred_at_ms,
        title,
        summary,
        sections,
        evidence: Some(evidence),
    })
}

impl SqliteSessionRow {
    fn into_meta(self) -> SessionMeta {
        SessionMeta {
            id: self.id,
            provider: self.provider,
            provider_version: self.provider_version,
            provider_generation_digest: self.provider_generation_digest,
            provider_auth_generation: self
                .provider_auth_generation
                .and_then(|value| u64::try_from(value).ok()),
            provider_behavior: self
                .provider_behavior
                .and_then(|value| serde_json::from_value(value).ok()),
            machine_id: self.machine_id,
            workspace_id: self.workspace_id,
            workspace_name: self.workspace_name,
            workspace_source_path: self.workspace_source_path,
            cwd: self.cwd,
            title: self.title,
            status: status_from_str(&self.status),
            origin: origin_from_str(&self.origin),
            agent_session_id: self.agent_session_id,
            auto_resume: self.auto_resume,
            awaiting_user: self.awaiting_user,
            done: self.done,
            system: self.system,
            judging: false,
            paused: false,
            context_used: 0,
            context_size: 0,
            usage: None,
            next_schedule_ms: None,
        }
    }
}

fn decode_event_row(row: EventRow, session_id: &str, operation: &str) -> Option<Envelope> {
    match serde_json::from_value::<Event>(row.payload) {
        Ok(event) => Some(Envelope {
            session_id: session_id.to_owned(),
            seq: u64::try_from(row.seq).unwrap_or(0),
            event,
            cmid: None,
        }),
        Err(error) => {
            tracing::warn!(
                %error,
                session = %session_id,
                seq = row.seq,
                operation,
                "skipping undecodable SQLite event",
            );
            None
        }
    }
}

impl SqliteStorage {
    pub(super) async fn connect(url: &str, artifact_dir: std::path::PathBuf) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(url)
            .with_context(|| format!("parsing SQLite URL {url}"))?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        // A plain in-memory SQLite database belongs to one connection. File
        // databases retain a small read pool while WAL serializes writers.
        let max_connections = if url.contains(":memory:") { 1 } else { 4 };
        let database_path = (max_connections > 1).then(|| options.get_filename().to_owned());
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await
            .with_context(|| format!("connecting to SQLite {url}"))?;
        Ok(Self {
            pool,
            artifacts: crate::artifacts::ArtifactStore::new(artifact_dir)?,
            database_path,
        })
    }

    pub(super) async fn migrate(&self) -> Result<()> {
        let mut migrator = SQLITE_MIGRATIONS;
        // Component rollback restores the previous controller binary but does
        // not roll back SQLite. Match the PostgreSQL backend's additive
        // migration contract while retaining checksum validation for every
        // migration known to this binary.
        migrator.set_ignore_missing(true);
        migrator
            .run(&self.pool)
            .await
            .context("running SQLite migrations")
    }

    pub(super) async fn create_machine_enrollment(
        &self,
        machine_id: &str,
        display_name: &str,
        ttl_seconds: i64,
    ) -> Result<String> {
        anyhow::ensure!(
            valid_machine_id(machine_id),
            "machine id must use 1-64 lowercase ASCII letters, digits, or hyphens"
        );
        anyhow::ensure!(machine_id != "local", "the local Machine id is reserved");
        anyhow::ensure!(
            !display_name.trim().is_empty(),
            "display name cannot be empty"
        );
        anyhow::ensure!(
            (60..=3600).contains(&ttl_seconds),
            "enrollment TTL must be 60-3600s"
        );
        let active_key_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM machines \
             WHERE id = ?1 AND public_key IS NOT NULL AND revoked_at_ms IS NULL)",
        )
        .bind(machine_id)
        .fetch_one(&self.pool)
        .await
        .context("checking existing Machine identity")?;
        anyhow::ensure!(
            !active_key_exists,
            "Machine already has an active identity; revoke it before re-enrollment"
        );
        let mut random = [0_u8; 32];
        std::fs::File::open("/dev/urandom")
            .context("opening OS randomness")?
            .read_exact(&mut random)
            .context("reading OS randomness")?;
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random);
        let token_hash = hex_sha256(token.as_bytes());
        let expires_at_ms = now_ms()
            .checked_add(ttl_seconds.saturating_mul(1_000))
            .context("Machine enrollment expiry overflow")?;
        sqlx::query(
            "INSERT INTO machine_enrollment_tokens \
             (token_hash, machine_id, display_name, expires_at_ms) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (machine_id) DO UPDATE SET token_hash = excluded.token_hash, \
             display_name = excluded.display_name, expires_at_ms = excluded.expires_at_ms, \
             used_at_ms = NULL, created_at_ms = ?5",
        )
        .bind(token_hash)
        .bind(machine_id)
        .bind(display_name.trim())
        .bind(expires_at_ms)
        .bind(now_ms())
        .execute(&self.pool)
        .await
        .context("creating Machine enrollment")?;
        Ok(token)
    }

    pub(super) async fn consume_machine_enrollment(
        &self,
        token: &str,
        public_key: &str,
        encryption_public_key: &str,
    ) -> Result<EnrolledMachine> {
        validate_encryption_public_key(encryption_public_key)?;
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("starting enrollment transaction")?;
        let timestamp = now_ms();
        let token_hash = hex_sha256(token.as_bytes());
        let row: Option<(String, String)> = sqlx::query_as(
            "UPDATE machine_enrollment_tokens SET used_at_ms = ?2 \
             WHERE token_hash = ?1 AND used_at_ms IS NULL AND expires_at_ms > ?2 \
             RETURNING machine_id, display_name",
        )
        .bind(token_hash)
        .bind(timestamp)
        .fetch_optional(&mut *transaction)
        .await
        .context("consuming Machine enrollment")?;
        let (id, display_name) =
            row.context("invalid, expired, or already used enrollment token")?;
        let result = sqlx::query(
            "INSERT INTO machines \
             (id, display_name, connection_mode, platform, architecture, status, public_key, \
              encryption_public_key, enrolled_at_ms) \
             VALUES (?1, ?2, 'outbound_wss', 'unknown', 'unknown', 'offline', ?3, ?4, ?5) \
             ON CONFLICT (id) DO UPDATE SET display_name = excluded.display_name, \
             connection_mode = 'outbound_wss', public_key = excluded.public_key, \
             encryption_public_key = excluded.encryption_public_key, \
             enrolled_at_ms = ?5, revoked_at_ms = NULL, status = 'offline', \
             connection_epoch = NULL, reconnect_deadline_at_ms = NULL, updated_at_ms = ?5 \
             WHERE machines.public_key IS NULL OR \
             (machines.revoked_at_ms IS NOT NULL AND machines.public_key IS NOT excluded.public_key)",
        )
        .bind(&id)
        .bind(&display_name)
        .bind(public_key)
        .bind(encryption_public_key)
        .bind(timestamp)
        .execute(&mut *transaction)
        .await
        .context("binding enrolled Machine public key")?;
        anyhow::ensure!(
            result.rows_affected() == 1,
            "Machine already has this identity or an active identity"
        );
        transaction
            .commit()
            .await
            .context("committing Machine enrollment")?;
        let fingerprint = crate::machine_auth::fingerprint(public_key)?;
        Ok(EnrolledMachine {
            id,
            display_name,
            fingerprint,
        })
    }

    pub(super) async fn machine_public_key(&self, machine_id: &str) -> Result<Option<String>> {
        let value: Option<Option<String>> = sqlx::query_scalar(
            "SELECT public_key FROM machines WHERE id = ?1 AND revoked_at_ms IS NULL",
        )
        .bind(machine_id)
        .fetch_optional(&self.pool)
        .await
        .context("loading Machine public key")?;
        Ok(value.flatten())
    }

    pub(super) async fn machine_encryption_public_key(
        &self,
        machine_id: &str,
    ) -> Result<Option<String>> {
        let value: Option<Option<String>> = sqlx::query_scalar(
            "SELECT encryption_public_key FROM machines WHERE id = ?1 AND revoked_at_ms IS NULL",
        )
        .bind(machine_id)
        .fetch_optional(&self.pool)
        .await
        .context("loading SQLite Machine encryption public key")?;
        Ok(value.flatten())
    }

    pub(super) async fn bind_machine_encryption_public_key(
        &self,
        machine_id: &str,
        encryption_public_key: &str,
    ) -> Result<()> {
        validate_encryption_public_key(encryption_public_key)?;
        let result = sqlx::query(
            "UPDATE machines SET encryption_public_key = ?2, updated_at_ms = ?3 \
             WHERE id = ?1 AND revoked_at_ms IS NULL \
             AND (encryption_public_key IS NULL OR encryption_public_key = ?2)",
        )
        .bind(machine_id)
        .bind(encryption_public_key)
        .bind(now_ms())
        .execute(&self.pool)
        .await
        .context("binding Machine encryption public key")?;
        anyhow::ensure!(
            result.rows_affected() == 1,
            "Machine encryption public key changed; revoke and re-enroll the Machine"
        );
        Ok(())
    }

    pub(super) async fn list_machines(&self) -> Result<Vec<MachineRecord>> {
        let rows: Vec<SqliteMachineRow> = sqlx::query_as(
            "SELECT id, display_name, connection_mode, platform, architecture, status, \
             inventory, last_seen_at_ms, revoked_at_ms, public_key FROM machines \
             ORDER BY id = 'local' DESC, display_name",
        )
        .fetch_all(&self.pool)
        .await
        .context("listing Machines")?;
        Ok(rows
            .into_iter()
            .map(|row| MachineRecord {
                id: row.id,
                display_name: row.display_name,
                connection_mode: row.connection_mode,
                platform: row.platform,
                architecture: row.architecture,
                status: row.status,
                inventory: row.inventory,
                last_seen_at_ms: row.last_seen_at_ms,
                revoked: row.revoked_at_ms.is_some(),
                fingerprint: row
                    .public_key
                    .as_deref()
                    .and_then(|key| crate::machine_auth::fingerprint(key).ok()),
            })
            .collect())
    }

    pub(super) async fn machine_is_local(&self, machine_id: &str) -> Result<bool> {
        let mode: Option<String> = sqlx::query_scalar(
            "SELECT connection_mode FROM machines WHERE id = ?1 AND revoked_at_ms IS NULL",
        )
        .bind(machine_id)
        .fetch_optional(&self.pool)
        .await
        .context("loading Machine connection mode")?;
        Ok(mode.as_deref() == Some("local"))
    }

    pub(super) async fn revoke_machine(&self, machine_id: &str) -> Result<()> {
        anyhow::ensure!(machine_id != "local", "the local Machine cannot be revoked");
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("starting Machine revocation")?;
        let timestamp = now_ms();
        let result = sqlx::query(
            "UPDATE machines SET revoked_at_ms = ?2, status = 'offline', \
             connection_epoch = NULL, reconnect_deadline_at_ms = NULL, updated_at_ms = ?2 \
             WHERE id = ?1 AND public_key IS NOT NULL AND revoked_at_ms IS NULL",
        )
        .bind(machine_id)
        .bind(timestamp)
        .execute(&mut *transaction)
        .await
        .context("revoking Machine identity")?;
        anyhow::ensure!(
            result.rows_affected() == 1,
            "unknown or already revoked Machine"
        );
        sqlx::query("DELETE FROM machine_enrollment_tokens WHERE machine_id = ?1")
            .bind(machine_id)
            .execute(&mut *transaction)
            .await
            .context("discarding Machine enrollment tokens")?;
        transaction
            .commit()
            .await
            .context("committing Machine revocation")?;
        Ok(())
    }

    pub(super) async fn machine_connection_is_current(
        &self,
        machine_id: &str,
        connection_epoch: &str,
    ) -> Result<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM machines WHERE id = ?1 \
             AND connection_epoch = ?2 AND revoked_at_ms IS NULL)",
        )
        .bind(machine_id)
        .bind(connection_epoch)
        .fetch_one(&self.pool)
        .await
        .context("checking Machine connection epoch")
    }

    pub(super) async fn machine_connected(
        &self,
        machine_id: &str,
        connection_epoch: &str,
        platform: &str,
        architecture: &str,
        connection_mode: &str,
        inventory: &serde_json::Value,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE machines SET connection_epoch = ?2, platform = ?3, architecture = ?4, \
             connection_mode = ?5, status = 'online', inventory = ?6, last_seen_at_ms = ?7, \
             reconnect_deadline_at_ms = NULL, updated_at_ms = ?7 \
             WHERE id = ?1 AND revoked_at_ms IS NULL",
        )
        .bind(machine_id)
        .bind(connection_epoch)
        .bind(platform)
        .bind(architecture)
        .bind(connection_mode)
        .bind(inventory)
        .bind(now_ms())
        .execute(&self.pool)
        .await
        .context("recording Machine connection")?;
        anyhow::ensure!(
            result.rows_affected() == 1,
            "Machine was revoked during authentication"
        );
        Ok(())
    }

    pub(super) async fn machine_seen(
        &self,
        machine_id: &str,
        connection_epoch: &str,
        inventory: Option<&serde_json::Value>,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE machines SET status = 'online', last_seen_at_ms = ?4, \
             inventory = COALESCE(?3, inventory), reconnect_deadline_at_ms = NULL, updated_at_ms = ?4 \
             WHERE id = ?1 AND connection_epoch = ?2 AND revoked_at_ms IS NULL",
        )
        .bind(machine_id)
        .bind(connection_epoch)
        .bind(inventory)
        .bind(now_ms())
        .execute(&self.pool)
        .await
        .context("refreshing Machine liveness")?;
        anyhow::ensure!(
            result.rows_affected() == 1,
            "Machine connection is no longer current"
        );
        Ok(())
    }

    pub(super) async fn machine_disconnected(
        &self,
        machine_id: &str,
        connection_epoch: &str,
        grace_seconds: i32,
    ) -> Result<()> {
        let timestamp = now_ms();
        let reconnect_deadline_at_ms = timestamp.saturating_add(i64::from(grace_seconds) * 1_000);
        sqlx::query(
            "UPDATE machines SET status = 'reconnecting', connection_epoch = NULL, \
             reconnect_deadline_at_ms = ?3, updated_at_ms = ?4 \
             WHERE id = ?1 AND connection_epoch = ?2 AND revoked_at_ms IS NULL",
        )
        .bind(machine_id)
        .bind(connection_epoch)
        .bind(reconnect_deadline_at_ms)
        .bind(timestamp)
        .execute(&self.pool)
        .await
        .context("recording Machine disconnect")?;
        Ok(())
    }

    pub(super) async fn expire_machine_reconnects(&self) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE machines SET status = 'offline', reconnect_deadline_at_ms = NULL, \
             updated_at_ms = ?1 WHERE status = 'reconnecting' \
             AND reconnect_deadline_at_ms <= ?1 AND connection_epoch IS NULL \
             AND revoked_at_ms IS NULL",
        )
        .bind(now_ms())
        .execute(&self.pool)
        .await
        .context("expiring Machine reconnect grace")?;
        Ok(result.rows_affected())
    }

    pub(super) async fn upsert_provider_reset(
        &self,
        provider: &str,
        fire_at_ms: i64,
        idempotency_key: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO scheduled_provider_actions \
             (provider, action, fire_at_ms, idempotency_key) \
             VALUES (?1, 'rate_limit_reset', ?2, ?3) \
             ON CONFLICT (provider) DO UPDATE SET action = excluded.action, \
             fire_at_ms = excluded.fire_at_ms, idempotency_key = excluded.idempotency_key, \
             attempt_count = 0, next_attempt_at_ms = excluded.fire_at_ms",
        )
        .bind(provider)
        .bind(fire_at_ms)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPSERT scheduled {provider} reset"))?;
        Ok(())
    }

    pub(super) async fn load_provider_reset(
        &self,
        provider: &str,
    ) -> Result<Option<ScheduledProviderAction>> {
        let row: Option<(i64, String, i32, Option<i64>)> = sqlx::query_as(
            "SELECT fire_at_ms, idempotency_key, attempt_count, next_attempt_at_ms \
             FROM scheduled_provider_actions \
             WHERE provider = ?1 AND action = 'rate_limit_reset'",
        )
        .bind(provider)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("SELECT scheduled {provider} reset"))?;
        Ok(row.map(
            |(fire_at_ms, idempotency_key, attempt_count, next_attempt_at_ms)| {
                ScheduledProviderAction {
                    fire_at_ms,
                    idempotency_key,
                    attempt_count,
                    next_attempt_at_ms: next_attempt_at_ms.unwrap_or(fire_at_ms),
                }
            },
        ))
    }

    pub(super) async fn defer_provider_reset(
        &self,
        provider: &str,
        idempotency_key: &str,
        next_attempt_at_ms: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE scheduled_provider_actions SET attempt_count = attempt_count + 1, \
             next_attempt_at_ms = ?3 WHERE provider = ?1 AND action = 'rate_limit_reset' \
             AND idempotency_key = ?2",
        )
        .bind(provider)
        .bind(idempotency_key)
        .bind(next_attempt_at_ms)
        .execute(&self.pool)
        .await
        .with_context(|| format!("defer scheduled {provider} reset"))?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn append_provider_action_log(
        &self,
        provider: &str,
        trigger: &str,
        status: &str,
        phase: &str,
        message: &str,
        credit_id: Option<&str>,
        idempotency_key: Option<&str>,
        created_at_ms: i64,
    ) -> Result<()> {
        let suffix = idempotency_key.map(|key| {
            key.chars()
                .rev()
                .take(8)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        });
        sqlx::query(
            "INSERT INTO provider_action_logs \
             (provider, action, trigger, status, phase, message, credit_id, idempotency_suffix, created_at_ms) \
             VALUES (?1, 'rate_limit_reset', ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(provider)
        .bind(trigger)
        .bind(status)
        .bind(phase)
        .bind(message)
        .bind(credit_id)
        .bind(suffix)
        .bind(created_at_ms)
        .execute(&self.pool)
        .await
        .context("append provider action log")?;
        Ok(())
    }

    pub(super) async fn provider_action_logs(&self, limit: i64) -> Result<Vec<ProviderActionLog>> {
        let rows = sqlx::query_as::<
            _,
            (
                i64,
                String,
                String,
                String,
                String,
                String,
                String,
                Option<String>,
                Option<String>,
                i64,
            ),
        >(
            "SELECT id, provider, action, trigger, status, phase, message, credit_id, \
             idempotency_suffix, created_at_ms FROM provider_action_logs \
             ORDER BY created_at_ms DESC, id DESC LIMIT ?1",
        )
        .bind(limit.clamp(1, 200))
        .fetch_all(&self.pool)
        .await
        .context("list provider action logs")?;
        Ok(rows
            .into_iter()
            .map(|row| ProviderActionLog {
                id: row.0,
                provider: row.1,
                action: row.2,
                trigger: row.3,
                status: row.4,
                phase: row.5,
                message: row.6,
                credit_id: row.7,
                idempotency_suffix: row.8,
                created_at_ms: row.9,
            })
            .collect())
    }

    pub(super) async fn delete_provider_reset(&self, provider: &str) -> Result<()> {
        sqlx::query("DELETE FROM scheduled_provider_actions WHERE provider = ?1")
            .bind(provider)
            .execute(&self.pool)
            .await
            .with_context(|| format!("DELETE scheduled {provider} reset"))?;
        Ok(())
    }

    pub(super) async fn claim_provider_reset(
        &self,
        provider: &str,
        idempotency_key: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM scheduled_provider_actions \
             WHERE provider = ?1 AND idempotency_key = ?2",
        )
        .bind(provider)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await
        .with_context(|| format!("claim scheduled {provider} reset"))?;
        Ok(result.rows_affected() == 1)
    }
}

impl SqliteStorage {
    pub(super) async fn next_session_number(&self) -> Result<u64> {
        let max: Option<i64> = sqlx::query_scalar(
            "SELECT max(CAST(substr(id, 6) AS INTEGER)) FROM sessions \
             WHERE id LIKE 'sess-%' AND substr(id, 6) <> '' \
             AND substr(id, 6) NOT GLOB '*[^0-9]*'",
        )
        .fetch_one(&self.pool)
        .await
        .context("SELECT max session id")?;
        Ok(max
            .and_then(|value| u64::try_from(value).ok())
            .map_or(1, |value| value.saturating_add(1)))
    }

    pub(super) async fn load_all(&self) -> Result<Vec<LoadedSession>> {
        let session_rows: Vec<SqliteSessionRow> = sqlx::query_as(
            "SELECT id, provider, provider_version, provider_generation_digest, \
             provider_auth_generation, provider_behavior, machine_id, workspace_id, workspace_name, workspace_source_path, \
             cwd, title, origin, status, agent_session_id, auto_resume, \
             awaiting_user, done, system, next_seq, queue, drafts, judge_runs, \
             config_options, config_preferences, mobile_review_state, created_at_ms \
             FROM sessions WHERE deleted_at_ms IS NULL \
             ORDER BY position IS NULL, position ASC, created_at_ms ASC",
        )
        .fetch_all(&self.pool)
        .await
        .context("SELECT SQLite sessions")?;

        let mut output = Vec::with_capacity(session_rows.len());
        for row in session_rows {
            let id = row.id.clone();
            let event_rows: Vec<EventRow> = sqlx::query_as(
                "SELECT seq, payload, count(*) OVER() AS total_count \
                 FROM events WHERE session_id = ?1 ORDER BY seq DESC LIMIT ?2",
            )
            .bind(&id)
            .bind(i64::try_from(crate::core::HOT_TAIL).unwrap_or(i64::MAX))
            .fetch_all(&self.pool)
            .await
            .with_context(|| format!("SELECT SQLite events for {id}"))?;
            let event_count = event_rows
                .first()
                .and_then(|event| u64::try_from(event.total_count).ok())
                .unwrap_or(0);
            let reached_start = event_count <= u64::try_from(event_rows.len()).unwrap_or(u64::MAX);
            let events = event_rows
                .into_iter()
                .rev()
                .filter_map(|event| decode_event_row(event, &id, "restore"))
                .collect();
            let next_seq = u64::try_from(row.next_seq).unwrap_or(0);
            let queue = serde_json::from_value(row.queue.clone()).unwrap_or_default();
            let drafts = serde_json::from_value(row.drafts.clone()).unwrap_or_default();
            let judge_runs = serde_json::from_value(row.judge_runs.clone()).unwrap_or_default();
            let config_options = row.config_options.clone();
            let config_preferences = if row.config_preferences.is_object() {
                row.config_preferences.clone()
            } else {
                serde_json::json!({})
            };
            let mobile_review_state = row.mobile_review_state.clone();
            output.push(LoadedSession {
                meta: row.into_meta(),
                events,
                event_count,
                reached_start,
                next_seq,
                queue,
                drafts,
                judge_runs,
                config_options,
                config_preferences,
                mobile_review_state,
            });
        }
        Ok(output)
    }

    pub(super) async fn history_page(
        &self,
        session_id: &str,
        before_seq: u64,
        page_size: usize,
    ) -> Result<(Vec<Envelope>, Option<u64>, bool)> {
        let before = i64::try_from(before_seq).context("history cursor overflow")?;
        let limit = i64::try_from(page_size).context("history page size overflow")?;
        let rows: Vec<EventRow> = sqlx::query_as(
            "SELECT seq, payload, 0 AS total_count FROM events \
             WHERE session_id = ?1 AND seq < ?2 ORDER BY seq DESC LIMIT ?3",
        )
        .bind(session_id)
        .bind(before)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite history page for {session_id}"))?;
        let events = bound_history_page(
            rows.into_iter()
                .rev()
                .filter_map(|row| decode_event_row(row, session_id, "history page"))
                .collect(),
        );
        let oldest = events.first().map(|event| event.seq);
        let reached_start = match oldest {
            Some(seq) => {
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM events \
                     WHERE session_id = ?1 AND seq < ?2)",
                )
                .bind(session_id)
                .bind(i64::try_from(seq).unwrap_or(i64::MAX))
                .fetch_one(&self.pool)
                .await
                .with_context(|| format!("SELECT SQLite history start for {session_id}"))?;
                !exists
            }
            None => true,
        };
        let next_before_seq = (!reached_start).then_some(oldest.unwrap_or(before_seq));
        Ok((events, next_before_seq, reached_start))
    }

    pub(super) async fn question_page_before(
        &self,
        session_id: &str,
        before_seq: u64,
    ) -> Result<(Vec<Envelope>, Option<u64>, bool)> {
        let before = i64::try_from(before_seq).context("question cursor overflow")?;
        let root: Option<i64> = sqlx::query_scalar(
            "WITH ordered AS ( \
               SELECT seq, payload, LAG(json_extract(payload, '$.update.sessionUpdate')) \
                 OVER (ORDER BY seq) AS previous_update \
               FROM events WHERE session_id = ?1 AND seq < ?2 \
             ) SELECT MAX(seq) FROM ordered \
             WHERE json_extract(payload, '$.kind') = 'update' \
               AND json_extract(payload, '$.update.sessionUpdate') = 'user_message_chunk' \
               AND COALESCE(json_extract(payload, '$.update.autoResumed'), 0) <> 1 \
               AND COALESCE(json_extract(payload, '$.update.promptOrigin.actor'), 'human') = 'human' \
               AND instr(lower(COALESCE(json_extract(payload, '$.update.content.text'), '')), '<system-reminder') = 0 \
               AND trim(COALESCE(json_extract(payload, '$.update.content.text'), '')) \
                   NOT IN ('/compact', '/compress') \
               AND previous_update IS NOT 'user_message_chunk'",
        )
        .bind(session_id)
        .bind(before)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite previous question root for {session_id}"))?;
        let Some(root) = root else {
            return Ok((Vec::new(), None, true));
        };
        let turn_end: Option<i64> = sqlx::query_scalar(
            "SELECT MIN(seq) FROM events WHERE session_id = ?1 AND seq >= ?2 AND seq < ?3 \
             AND json_extract(payload, '$.kind') = 'turn_end'",
        )
        .bind(session_id)
        .bind(root)
        .bind(before)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite question turn end for {session_id}"))?;
        let page_end = turn_end
            .and_then(|seq| seq.checked_add(1))
            .unwrap_or(before);
        let rows: Vec<EventRow> = sqlx::query_as(
            "SELECT seq, payload, 0 AS total_count FROM events \
             WHERE session_id = ?1 AND seq >= ?2 AND seq < ?3 ORDER BY seq",
        )
        .bind(session_id)
        .bind(root)
        .bind(page_end)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite complete question page for {session_id}"))?;
        let events = rows
            .into_iter()
            .filter_map(|row| decode_event_row(row, session_id, "question page"))
            .collect();
        let has_earlier_root: bool = sqlx::query_scalar(
            "WITH ordered AS ( \
               SELECT seq, payload, LAG(json_extract(payload, '$.update.sessionUpdate')) \
                 OVER (ORDER BY seq) AS previous_update \
               FROM events WHERE session_id = ?1 AND seq < ?2 \
             ) SELECT EXISTS(SELECT 1 FROM ordered \
               WHERE json_extract(payload, '$.kind') = 'update' \
                 AND json_extract(payload, '$.update.sessionUpdate') = 'user_message_chunk' \
                 AND COALESCE(json_extract(payload, '$.update.autoResumed'), 0) <> 1 \
                 AND COALESCE(json_extract(payload, '$.update.promptOrigin.actor'), 'human') = 'human' \
                 AND instr(lower(COALESCE(json_extract(payload, '$.update.content.text'), '')), '<system-reminder') = 0 \
                 AND trim(COALESCE(json_extract(payload, '$.update.content.text'), '')) \
                     NOT IN ('/compact', '/compress') \
                 AND previous_update IS NOT 'user_message_chunk')",
        )
        .bind(session_id)
        .bind(root)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite earlier question root for {session_id}"))?;
        let root = u64::try_from(root).unwrap_or(0);
        Ok((events, has_earlier_root.then_some(root), !has_earlier_root))
    }

    pub(super) async fn question_page_summaries(
        &self,
        session_id: &str,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<QuestionPageSummary>, Option<u64>, u64)> {
        let before = before_seq
            .map(i64::try_from)
            .transpose()
            .context("question summary cursor overflow")?;
        let rows = sqlx::query_as::<_, (i64, String, i64, i64)>(
            "WITH ordered AS ( \
               SELECT seq, payload, LAG(json_extract(payload, '$.update.sessionUpdate')) \
                 OVER (ORDER BY seq) AS previous_update \
               FROM events WHERE session_id = ?1 \
             ), roots AS ( \
               SELECT seq, COALESCE(json_extract(payload, '$.update.content.text'), '') AS title \
               FROM ordered WHERE json_extract(payload, '$.kind') = 'update' \
                 AND json_extract(payload, '$.update.sessionUpdate') = 'user_message_chunk' \
                 AND COALESCE(json_extract(payload, '$.update.autoResumed'), 0) <> 1 \
                 AND COALESCE(json_extract(payload, '$.update.promptOrigin.actor'), 'human') = 'human' \
                 AND instr(lower(COALESCE(json_extract(payload, '$.update.content.text'), '')), '<system-reminder') = 0 \
                 AND trim(COALESCE(json_extract(payload, '$.update.content.text'), '')) \
                     NOT IN ('/compact', '/compress') \
                 AND previous_update IS NOT 'user_message_chunk' \
             ), numbered AS ( \
               SELECT seq, title, ROW_NUMBER() OVER (ORDER BY seq) AS ordinal, \
                 COUNT(*) OVER () AS total FROM roots \
             ) SELECT seq, title, ordinal, total FROM numbered \
             WHERE (?2 IS NULL OR seq < ?2) ORDER BY seq DESC LIMIT ?3",
        )
        .bind(session_id)
        .bind(before)
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite question summaries for {session_id}"))?;
        let total = rows
            .first()
            .map_or(0, |row| u64::try_from(row.3).unwrap_or(0));
        let next_before_seq = rows
            .last()
            .and_then(|row| (row.2 > 1).then(|| u64::try_from(row.0).unwrap_or(0)));
        let mut pages = rows
            .into_iter()
            .map(|(seq, title, ordinal, _)| {
                let ordinal = u64::try_from(ordinal).unwrap_or(u64::MAX);
                QuestionPageSummary {
                    id: u64::try_from(seq).unwrap_or(0),
                    title: question_summary_title(&title, ordinal),
                    ordinal,
                }
            })
            .collect::<Vec<_>>();
        pages.reverse();
        Ok((pages, next_before_seq, total))
    }

    pub(super) async fn question_page_at(
        &self,
        session_id: &str,
        root_seq: u64,
    ) -> Result<Option<Vec<Envelope>>> {
        let root = i64::try_from(root_seq).context("question root overflow")?;
        let bounds = sqlx::query_as::<_, (i64, Option<i64>)>(
            "WITH ordered AS ( \
               SELECT seq, payload, LAG(json_extract(payload, '$.update.sessionUpdate')) \
                 OVER (ORDER BY seq) AS previous_update \
               FROM events WHERE session_id = ?1 \
             ), roots AS ( \
               SELECT seq FROM ordered WHERE json_extract(payload, '$.kind') = 'update' \
                 AND json_extract(payload, '$.update.sessionUpdate') = 'user_message_chunk' \
                 AND COALESCE(json_extract(payload, '$.update.autoResumed'), 0) <> 1 \
                 AND COALESCE(json_extract(payload, '$.update.promptOrigin.actor'), 'human') = 'human' \
                 AND instr(lower(COALESCE(json_extract(payload, '$.update.content.text'), '')), '<system-reminder') = 0 \
                 AND trim(COALESCE(json_extract(payload, '$.update.content.text'), '')) \
                     NOT IN ('/compact', '/compress') \
                 AND previous_update IS NOT 'user_message_chunk' \
             ), bounded AS ( \
               SELECT seq, LEAD(seq) OVER (ORDER BY seq) AS next_seq FROM roots \
             ) SELECT seq, next_seq FROM bounded WHERE seq = ?2",
        )
        .bind(session_id)
        .bind(root)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| {
            format!("SELECT SQLite question page bounds for {session_id}:{root_seq}")
        })?;
        let Some((start, end)) = bounds else {
            return Ok(None);
        };
        let turn_end: Option<i64> = sqlx::query_scalar(
            "SELECT MIN(seq) FROM events WHERE session_id = ?1 AND seq >= ?2 \
             AND (?3 IS NULL OR seq < ?3) AND json_extract(payload, '$.kind') = 'turn_end'",
        )
        .bind(session_id)
        .bind(start)
        .bind(end)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite question turn end for {session_id}:{root_seq}"))?;
        let end = turn_end.and_then(|seq| seq.checked_add(1)).or(end);
        let rows: Vec<EventRow> = sqlx::query_as(
            "SELECT seq, payload, 0 AS total_count FROM events \
             WHERE session_id = ?1 AND seq >= ?2 AND (?3 IS NULL OR seq < ?3) ORDER BY seq",
        )
        .bind(session_id)
        .bind(start)
        .bind(end)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite question page at {session_id}:{root_seq}"))?;
        Ok(Some(
            rows.into_iter()
                .filter_map(|row| decode_event_row(row, session_id, "question page at"))
                .collect(),
        ))
    }

    pub(super) async fn insert_session(&self, meta: &SessionMeta) -> Result<()> {
        sqlx::query(
            "INSERT INTO sessions(id, provider, provider_version, provider_generation_digest, \
             provider_auth_generation, provider_behavior, machine_id, workspace_id, workspace_name, \
             workspace_source_path, cwd, title, origin, status, next_seq, system) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 0, ?15)",
        )
        .bind(&meta.id)
        .bind(&meta.provider)
        .bind(&meta.provider_version)
        .bind(&meta.provider_generation_digest)
        .bind(
            meta.provider_auth_generation
                .and_then(|value| i64::try_from(value).ok()),
        )
        .bind(
            meta.provider_behavior
                .as_ref()
                .map(serde_json::to_value)
                .transpose()?,
        )
        .bind(&meta.machine_id)
        .bind(meta.workspace_id.as_deref())
        .bind(meta.workspace_name.as_deref())
        .bind(meta.workspace_source_path.as_deref())
        .bind(&meta.cwd)
        .bind(strip_nul_str(&meta.title))
        .bind(origin_to_str(meta.origin))
        .bind(status_to_str(meta.status))
        .bind(meta.system)
        .execute(&self.pool)
        .await
        .with_context(|| format!("INSERT SQLite session {}", meta.id))?;
        Ok(())
    }

    pub(super) async fn update_status(&self, session_id: &str, status: Status) -> Result<()> {
        sqlx::query("UPDATE sessions SET status = ?1, updated_at_ms = ?2 WHERE id = ?3")
            .bind(status_to_str(status))
            .bind(now_ms())
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE SQLite session status {session_id}"))?;
        Ok(())
    }

    pub(super) async fn update_verdict(
        &self,
        session_id: &str,
        awaiting_user: bool,
        done: bool,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET awaiting_user = ?1, done = ?2, updated_at_ms = ?3 WHERE id = ?4",
        )
        .bind(awaiting_user)
        .bind(done)
        .bind(now_ms())
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPDATE SQLite session verdict {session_id}"))?;
        Ok(())
    }

    pub(super) async fn update_agent_session_id(
        &self,
        session_id: &str,
        agent_session_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE sessions SET agent_session_id = ?1, updated_at_ms = ?2 WHERE id = ?3")
            .bind(agent_session_id)
            .bind(now_ms())
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE SQLite session agent_session_id {session_id}"))?;
        Ok(())
    }

    pub(super) async fn update_config_options(
        &self,
        session_id: &str,
        options: &serde_json::Value,
    ) -> Result<()> {
        let mut options = options.clone();
        strip_nul(&mut options);
        sqlx::query("UPDATE sessions SET config_options = ?1, updated_at_ms = ?2 WHERE id = ?3")
            .bind(options)
            .bind(now_ms())
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE SQLite session config_options {session_id}"))?;
        Ok(())
    }

    pub(super) async fn update_config_preferences(
        &self,
        session_id: &str,
        preferences: &serde_json::Value,
    ) -> Result<()> {
        let mut preferences = preferences.clone();
        strip_nul(&mut preferences);
        sqlx::query(
            "UPDATE sessions SET config_preferences = ?1, updated_at_ms = ?2 WHERE id = ?3",
        )
        .bind(preferences)
        .bind(now_ms())
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPDATE SQLite session config_preferences {session_id}"))?;
        Ok(())
    }

    pub(super) async fn update_title(&self, session_id: &str, title: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET title = ?1, updated_at_ms = ?2 WHERE id = ?3")
            .bind(strip_nul_str(title))
            .bind(now_ms())
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE SQLite session title {session_id}"))?;
        Ok(())
    }

    pub(super) async fn update_cwd(
        &self,
        session_id: &str,
        cwd: &str,
        title: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET cwd = ?1, title = COALESCE(?2, title), \
             updated_at_ms = ?3 WHERE id = ?4",
        )
        .bind(cwd)
        .bind(title.map(strip_nul_str))
        .bind(now_ms())
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPDATE SQLite session cwd {session_id}"))?;
        Ok(())
    }

    pub(super) async fn update_auto_resume(
        &self,
        session_id: &str,
        value: Option<bool>,
    ) -> Result<()> {
        sqlx::query("UPDATE sessions SET auto_resume = ?1, updated_at_ms = ?2 WHERE id = ?3")
            .bind(value)
            .bind(now_ms())
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE SQLite session auto_resume {session_id}"))?;
        Ok(())
    }

    pub(super) async fn load_settings(&self) -> Result<Vec<(String, serde_json::Value)>> {
        sqlx::query_as("SELECT key, value FROM settings")
            .fetch_all(&self.pool)
            .await
            .context("SELECT SQLite settings")
    }

    pub(super) async fn put_setting(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        let mut value = value.clone();
        strip_nul(&mut value);
        sqlx::query(
            "INSERT INTO settings(key, value, updated_at_ms) VALUES (?1, ?2, ?3) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(key)
        .bind(&value)
        .bind(now_ms())
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPSERT SQLite setting {key}"))?;
        Ok(())
    }

    pub(super) async fn update_mobile_review_state(
        &self,
        session_id: &str,
        value: &serde_json::Value,
    ) -> Result<()> {
        let mut value = value.clone();
        strip_nul(&mut value);
        sqlx::query(
            "UPDATE sessions SET mobile_review_state = ?1, updated_at_ms = ?2 WHERE id = ?3",
        )
        .bind(&value)
        .bind(now_ms())
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPDATE SQLite mobile review state {session_id}"))?;
        Ok(())
    }

    pub(super) async fn upsert_event_batch(
        &self,
        events: &[Envelope],
        highwaters: &HashMap<String, u64>,
    ) -> Result<()> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("begin SQLite event batch")?;
        for envelope in events {
            let mut payload =
                serde_json::to_value(&envelope.event).context("serialize SQLite event")?;
            strip_nul(&mut payload);
            self.artifacts.externalize_images(&mut payload)?;
            let sequence = i64::try_from(envelope.seq).context("seq i64 overflow")?;
            sqlx::query(
                "INSERT INTO events(session_id, seq, payload) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(session_id, seq) DO UPDATE SET payload = excluded.payload",
            )
            .bind(&envelope.session_id)
            .bind(sequence)
            .bind(&payload)
            .execute(&mut *transaction)
            .await
            .with_context(|| {
                format!(
                    "UPSERT SQLite event {}/{}",
                    envelope.session_id, envelope.seq
                )
            })?;
        }
        let timestamp = now_ms();
        for (session_id, next_seq) in highwaters {
            let next_seq = i64::try_from(*next_seq).context("next_seq i64 overflow")?;
            sqlx::query(
                "UPDATE sessions SET next_seq = max(next_seq, ?1), updated_at_ms = ?2 WHERE id = ?3",
            )
            .bind(next_seq)
            .bind(timestamp)
            .bind(session_id)
            .execute(&mut *transaction)
            .await
            .with_context(|| format!("UPDATE SQLite next_seq for {session_id}"))?;
        }
        transaction
            .commit()
            .await
            .context("commit SQLite event batch")?;
        Ok(())
    }

    pub(super) async fn clear_events(&self, session_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM events WHERE session_id = ?1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("DELETE SQLite events for {session_id}"))?;
        Ok(())
    }

    pub(super) fn artifact_path(&self, name: &str) -> Option<std::path::PathBuf> {
        self.artifacts.path(name)
    }

    pub(super) async fn update_pending(
        &self,
        session_id: &str,
        queue: &[QueuedMessage],
        drafts: &[QueuedMessage],
    ) -> Result<()> {
        let mut queue_json = serde_json::to_value(queue).context("serialize SQLite queue")?;
        let mut drafts_json = serde_json::to_value(drafts).context("serialize SQLite drafts")?;
        strip_nul(&mut queue_json);
        strip_nul(&mut drafts_json);
        sqlx::query(
            "UPDATE sessions SET queue = ?1, drafts = ?2, updated_at_ms = ?3 WHERE id = ?4",
        )
        .bind(&queue_json)
        .bind(&drafts_json)
        .bind(now_ms())
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPDATE SQLite session pending {session_id}"))?;
        Ok(())
    }

    pub(super) async fn update_judge_runs(
        &self,
        session_id: &str,
        runs: &[JudgeRun],
    ) -> Result<()> {
        let mut runs_json = serde_json::to_value(runs).context("serialize SQLite judge_runs")?;
        strip_nul(&mut runs_json);
        sqlx::query("UPDATE sessions SET judge_runs = ?1, updated_at_ms = ?2 WHERE id = ?3")
            .bind(&runs_json)
            .bind(now_ms())
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE SQLite session judge_runs {session_id}"))?;
        Ok(())
    }

    pub(super) async fn upsert_wakeup(
        &self,
        session_id: &str,
        fire_at_ms: i64,
        prompt: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO scheduled_wakeups(session_id, fire_at_ms, prompt) VALUES (?1, ?2, ?3) \
             ON CONFLICT (session_id) DO UPDATE SET fire_at_ms = excluded.fire_at_ms, \
             prompt = excluded.prompt",
        )
        .bind(session_id)
        .bind(fire_at_ms)
        .bind(prompt)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPSERT SQLite wakeup {session_id}"))?;
        Ok(())
    }

    pub(super) async fn delete_wakeup(&self, session_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM scheduled_wakeups WHERE session_id = ?1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("DELETE SQLite wakeup {session_id}"))?;
        Ok(())
    }

    pub(super) async fn load_wakeups(&self) -> Result<Vec<(String, i64, String)>> {
        sqlx::query_as("SELECT session_id, fire_at_ms, prompt FROM scheduled_wakeups")
            .fetch_all(&self.pool)
            .await
            .context("SELECT SQLite scheduled_wakeups")
    }

    pub(super) async fn update_session_order(&self, order: &[String]) -> Result<()> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("begin SQLite session-order transaction")?;
        let timestamp = now_ms();
        for (index, id) in order.iter().enumerate() {
            let position = i64::try_from(index).unwrap_or(i64::MAX);
            sqlx::query("UPDATE sessions SET position = ?1, updated_at_ms = ?2 WHERE id = ?3")
                .bind(position)
                .bind(timestamp)
                .bind(id)
                .execute(&mut *transaction)
                .await
                .with_context(|| format!("UPDATE SQLite position for {id}"))?;
        }
        transaction
            .commit()
            .await
            .context("commit SQLite session order")?;
        Ok(())
    }

    pub(super) async fn delete_session(&self, session_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET deleted_at_ms = ?2 WHERE id = ?1 AND deleted_at_ms IS NULL",
        )
        .bind(session_id)
        .bind(now_ms())
        .execute(&self.pool)
        .await
        .with_context(|| format!("soft-delete SQLite session {session_id}"))?;
        Ok(())
    }

    pub(super) async fn soft_delete_sessions_until(
        &self,
        session_ids: &[String],
        purge_after_ms: i64,
    ) -> Result<()> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("begin Provider uninstall")?;
        let timestamp = now_ms();
        for session_id in session_ids {
            let result = sqlx::query(
                "UPDATE sessions SET deleted_at_ms = ?2, purge_after_at_ms = ?3 \
                 WHERE id = ?1 AND deleted_at_ms IS NULL",
            )
            .bind(session_id)
            .bind(timestamp)
            .bind(purge_after_ms)
            .execute(&mut *transaction)
            .await
            .with_context(|| format!("soft-delete Provider session {session_id}"))?;
            anyhow::ensure!(
                result.rows_affected() == 1,
                "Provider uninstall session set changed; refresh the uninstall plan"
            );
        }
        transaction
            .commit()
            .await
            .context("commit Provider uninstall")?;
        Ok(())
    }

    pub(super) async fn purge_deleted(&self, retention_days: i64) -> Result<u64> {
        let retention_ms = retention_days.saturating_mul(86_400_000);
        let result = sqlx::query(
            "DELETE FROM sessions WHERE deleted_at_ms IS NOT NULL \
             AND COALESCE(purge_after_at_ms, deleted_at_ms + ?1) <= ?2",
        )
        .bind(retention_ms)
        .bind(now_ms())
        .execute(&self.pool)
        .await
        .context("purge soft-deleted SQLite sessions")?;
        let mut referenced = HashSet::new();
        let mut rows =
            sqlx::query("SELECT payload FROM events WHERE payload LIKE '%/api/artifacts/%'")
                .fetch(&self.pool);
        while let Some(row) = rows
            .try_next()
            .await
            .context("scan retained SQLite artifact references")?
        {
            let payload: serde_json::Value = row.try_get("payload")?;
            crate::artifacts::collect_references(&payload, &mut referenced);
        }
        let artifacts_removed = self
            .artifacts
            .prune_unreferenced(&referenced, Duration::from_hours(24))
            .context("prune unreferenced SQLite event artifacts")?;
        if artifacts_removed > 0 {
            tracing::info!(
                artifacts_removed,
                "purged unreferenced SQLite event artifacts"
            );
        }
        Ok(result.rows_affected())
    }

    pub(super) async fn upsert_runtime_incident(
        &self,
        incident: &RuntimeIncidentWrite,
    ) -> Result<()> {
        let mut detail = incident.detail.clone();
        strip_nul(&mut detail);
        let id = strip_nul_str(&incident.id);
        let mut connection = self
            .pool
            .acquire()
            .await
            .context("acquire SQLite incident connection")?;
        let mut transaction = (*connection)
            .begin_with("BEGIN IMMEDIATE")
            .await
            .context("begin SQLite incident transaction")?;
        let existing = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT detail FROM runtime_incidents WHERE id = ?1",
        )
        .bind(id.as_ref())
        .fetch_optional(&mut *transaction)
        .await
        .context("load existing SQLite incident detail")?;
        let detail = merge_jsonb_values(existing, detail);
        let timestamp = now_ms();
        sqlx::query(
            "INSERT INTO runtime_incidents (id, occurred_at_ms, source, classification, severity, \
             state, summary, fingerprint, session_id, client_id, machine_id, trace_id, build, \
             evidence_start_ms, evidence_end_ms, detail) VALUES ( \
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, \
             COALESCE(?11, (SELECT machine_id FROM sessions WHERE id = ?9)), \
             ?12, ?13, ?14, ?15, ?16) \
             ON CONFLICT (id) DO UPDATE SET updated_at_ms = ?17, \
             evidence_end_ms = max(runtime_incidents.evidence_end_ms, excluded.evidence_end_ms), \
             detail = excluded.detail",
        )
        .bind(id.as_ref())
        .bind(incident.occurred_at_ms)
        .bind(strip_nul_str(&incident.source).as_ref())
        .bind(strip_nul_str(&incident.classification).as_ref())
        .bind(strip_nul_str(&incident.severity).as_ref())
        .bind(strip_nul_str(&incident.state).as_ref())
        .bind(strip_nul_str(&incident.summary).as_ref())
        .bind(strip_nul_str(&incident.fingerprint).as_ref())
        .bind(incident.session_id.as_deref())
        .bind(incident.client_id.as_deref())
        .bind(incident.machine_id.as_deref())
        .bind(incident.trace_id.as_deref())
        .bind(incident.build.as_deref())
        .bind(incident.evidence_start_ms)
        .bind(incident.evidence_end_ms)
        .bind(detail)
        .bind(timestamp)
        .execute(&mut *transaction)
        .await
        .context("UPSERT SQLite runtime incident")?;
        transaction
            .commit()
            .await
            .context("commit SQLite runtime incident")?;
        Ok(())
    }

    pub(super) async fn recover_runtime_incident(
        &self,
        session_id: &str,
        recovered_at_ms: i64,
        outcome: &str,
    ) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE runtime_incidents SET state = 'recovered', recovered_at_ms = ?2, \
             recovery_outcome = ?3, updated_at_ms = ?4 WHERE session_id = ?1 \
             AND source = 'controller' AND state = 'active'",
        )
        .bind(session_id)
        .bind(recovered_at_ms)
        .bind(outcome)
        .bind(now_ms())
        .execute(&self.pool)
        .await
        .with_context(|| format!("recover SQLite runtime incident for {session_id}"))?;
        Ok(result.rows_affected())
    }

    pub(super) async fn runtime_incidents(&self, limit: i64) -> Result<Vec<RuntimeIncident>> {
        sqlx::query_as(
            "SELECT id, occurred_at_ms, updated_at_ms, source, classification, severity, state, \
             summary, fingerprint, session_id, client_id, machine_id, trace_id, build, \
             evidence_start_ms, evidence_end_ms, detail, recovered_at_ms, recovery_outcome \
             FROM runtime_incidents ORDER BY occurred_at_ms DESC LIMIT ?1",
        )
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await
        .context("SELECT SQLite runtime incidents")
    }

    pub(super) async fn storage_metrics(&self) -> Result<(i64, i64, i64)> {
        let page_count: i64 = sqlx::query_scalar("PRAGMA page_count")
            .fetch_one(&self.pool)
            .await
            .context("read SQLite page count")?;
        let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
            .fetch_one(&self.pool)
            .await
            .context("read SQLite page size")?;
        let mut database_bytes = page_count.saturating_mul(page_size);
        if let Some(path) = &self.database_path {
            let wal = path.with_file_name(format!(
                "{}-wal",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            if let Ok(metadata) = std::fs::metadata(wal) {
                database_bytes = database_bytes
                    .saturating_add(i64::try_from(metadata.len()).unwrap_or(i64::MAX));
            }
        }
        let (events, deleted): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM events), \
             (SELECT count(*) FROM sessions WHERE deleted_at_ms IS NOT NULL)",
        )
        .fetch_one(&self.pool)
        .await
        .context("read SQLite storage metrics")?;
        Ok((database_bytes, events, deleted))
    }
}

impl SqliteStorage {
    #[allow(clippy::too_many_lines)]
    pub(super) async fn diagnostic_logs(
        &self,
        filter: &DiagnosticLogFilter,
    ) -> Result<Vec<DiagnosticLogSummary>> {
        let incidents: Vec<RuntimeIncidentWithProvider> = sqlx::query_as(
            "SELECT incident.id, incident.occurred_at_ms, incident.updated_at_ms, \
             incident.source, incident.classification, incident.severity, incident.state, \
             incident.summary, incident.fingerprint, incident.session_id, incident.client_id, \
             incident.machine_id, incident.trace_id, incident.build, incident.evidence_start_ms, \
             incident.evidence_end_ms, incident.detail, incident.recovered_at_ms, \
             incident.recovery_outcome, session.provider \
             FROM runtime_incidents incident LEFT JOIN sessions session \
             ON session.id = incident.session_id \
             WHERE incident.occurred_at_ms >= ?1 AND incident.occurred_at_ms <= ?2",
        )
        .bind(filter.since_ms)
        .bind(filter.until_ms)
        .fetch_all(&self.pool)
        .await
        .context("load SQLite runtime diagnostics")?;
        let mut summaries = incidents
            .into_iter()
            .map(RuntimeIncidentWithProvider::into_parts)
            .map(|(incident, provider)| {
                let agent = provider.as_deref().and_then(|provider| match provider {
                    "codex" | "codex-deepseek" => Some("codex".to_owned()),
                    "claude-code" | "claude-deepseek" => Some("claude".to_owned()),
                    _ => None,
                });
                DiagnosticLogSummary {
                    id: format!("runtime:{}", incident.id),
                    occurred_at_ms: incident.occurred_at_ms,
                    kind: "session_error".to_owned(),
                    severity: incident.severity,
                    state: incident.state,
                    title: diagnostic_title(&incident.classification),
                    summary: incident.summary,
                    session_ref: incident.session_id,
                    provider,
                    agent,
                    model: None,
                    classification: Some(incident.classification),
                }
            })
            .collect::<Vec<_>>();

        let provider_records = load_provider_usage_records(
            &self.pool,
            "deepseek",
            filter.since_ms.saturating_sub(30 * 60 * 1_000),
            filter.until_ms,
            None,
            None,
        )
        .await?;
        let mut previous = std::collections::HashMap::<
            (String, String, String, String),
            ProviderUsageRecord,
        >::new();
        for record in &provider_records {
            let key = provider_record_session_key(record).map(|key| {
                (
                    key.0.to_owned(),
                    key.1.to_owned(),
                    key.2.to_owned(),
                    key.3.to_owned(),
                )
            });
            if record.occurred_at_ms >= filter.since_ms {
                if record.request_purpose == "interactive" && record.status >= 400 {
                    summaries.push(DiagnosticLogSummary {
                        id: format!(
                            "provider:{}:{}:{}",
                            record.machine_id, record.producer_id, record.sequence
                        ),
                        occurred_at_ms: record.occurred_at_ms,
                        kind: "provider_error".to_owned(),
                        severity: provider_error_severity(record.status).to_owned(),
                        state: "failed".to_owned(),
                        title: format!("DeepSeek HTTP {}", record.status),
                        summary: provider_error_summary(record),
                        session_ref: record.session_fingerprint.clone(),
                        provider: Some(record.provider.clone()),
                        agent: Some(record.agent.clone()),
                        model: Some(record.billing_model().to_owned())
                            .filter(|value| !value.is_empty()),
                        classification: Some(
                            provider_error_classification(record.status).to_owned(),
                        ),
                    });
                }
                if record.request_purpose == "cache_keepalive" {
                    summaries.push(DiagnosticLogSummary {
                        id: format!(
                            "keepalive:{}:{}:{}",
                            record.machine_id, record.producer_id, record.sequence
                        ),
                        occurred_at_ms: record.occurred_at_ms,
                        kind: "cache_anomaly".to_owned(),
                        severity: keepalive_severity(record).to_owned(),
                        state: keepalive_state(&record.cache_keepalive_outcome).to_owned(),
                        title: keepalive_title(&record.cache_keepalive_outcome).to_owned(),
                        summary: keepalive_summary(record),
                        session_ref: record.session_fingerprint.clone(),
                        provider: Some(record.provider.clone()),
                        agent: Some(record.agent.clone()),
                        model: Some(record.billing_model().to_owned())
                            .filter(|value| !value.is_empty()),
                        classification: Some(format!(
                            "cache_keepalive_{}",
                            record.cache_keepalive_outcome
                        )),
                    });
                }
                if let Some(key) = &key
                    && let Some(prior) = previous.get(key)
                    && diagnostic_cache_transition(record, prior)
                {
                    let intervening_error = provider_records.iter().any(|candidate| {
                        candidate.machine_id == record.machine_id
                            && candidate.producer_id == record.producer_id
                            && candidate.account_fingerprint == record.account_fingerprint
                            && candidate.agent == record.agent
                            && candidate.session_fingerprint == record.session_fingerprint
                            && candidate.request_purpose == "interactive"
                            && candidate.occurred_at_ms > prior.occurred_at_ms
                            && candidate.occurred_at_ms < record.occurred_at_ms
                            && candidate.status >= 400
                    });
                    let cause = diagnostic_cache_cause(record, prior, intervening_error);
                    summaries.push(DiagnosticLogSummary {
                        id: format!(
                            "cache:{}:{}:{}",
                            record.machine_id, record.producer_id, record.sequence
                        ),
                        occurred_at_ms: record.occurred_at_ms,
                        kind: "cache_anomaly".to_owned(),
                        severity: "warning".to_owned(),
                        state: "observed".to_owned(),
                        title: cache_transition_title(cause).to_owned(),
                        summary: diagnostic_cache_summary(record, prior),
                        session_ref: record.session_fingerprint.clone(),
                        provider: Some(record.provider.clone()),
                        agent: Some(record.agent.clone()),
                        model: Some(record.billing_model().to_owned())
                            .filter(|value| !value.is_empty()),
                        classification: Some(cause.to_owned()),
                    });
                }
            }
            if diagnostic_lineage_eligible(record)
                && let Some(key) = key
            {
                previous.insert(key, record.clone());
            }
        }

        let action_logs = self.provider_action_logs(200).await?;
        summaries.extend(
            action_logs
                .into_iter()
                .filter(|log| (filter.since_ms..=filter.until_ms).contains(&log.created_at_ms))
                .map(|log| DiagnosticLogSummary {
                    id: format!("automation:{}", log.id),
                    occurred_at_ms: log.created_at_ms,
                    kind: "automation".to_owned(),
                    severity: match log.status.as_str() {
                        "failed" => "error",
                        "retrying" | "unknown" => "warning",
                        _ => "info",
                    }
                    .to_owned(),
                    state: log.status,
                    title: format!(
                        "{} {}",
                        diagnostic_title(&log.provider),
                        log.action.replace('_', " ")
                    ),
                    summary: log.message,
                    session_ref: None,
                    provider: Some(log.provider),
                    agent: None,
                    model: None,
                    classification: Some("provider_automation".to_owned()),
                }),
        );
        summaries.retain(|summary| matches_diagnostic_filter(summary, filter));
        summaries.sort_by(|left, right| {
            right
                .occurred_at_ms
                .cmp(&left.occurred_at_ms)
                .then_with(|| right.id.cmp(&left.id))
        });
        summaries.truncate(usize::try_from(filter.limit.clamp(1, 200)).unwrap_or(200));
        Ok(summaries)
    }

    #[allow(clippy::too_many_lines)]
    pub(super) async fn diagnostic_log_detail(
        &self,
        id: &str,
    ) -> Result<Option<DiagnosticLogDetail>> {
        if let Some(incident_id) = id.strip_prefix("runtime:") {
            let incident: Option<RuntimeIncident> = sqlx::query_as(
                "SELECT id, occurred_at_ms, updated_at_ms, source, classification, severity, \
                 state, summary, fingerprint, session_id, client_id, machine_id, trace_id, build, \
                 evidence_start_ms, evidence_end_ms, detail, recovered_at_ms, recovery_outcome \
                 FROM runtime_incidents WHERE id = ?1",
            )
            .bind(incident_id)
            .fetch_optional(&self.pool)
            .await
            .context("load SQLite runtime diagnostic detail")?;
            return Ok(incident.map(|incident| runtime_diagnostic_detail(id, incident)));
        }
        if let Some(action_id) = id.strip_prefix("automation:") {
            let Ok(action_id) = action_id.parse::<i64>() else {
                return Ok(None);
            };
            let row = sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    String,
                    String,
                    String,
                    Option<String>,
                    Option<String>,
                    i64,
                ),
            >(
                "SELECT provider, action, trigger, status, phase, message, credit_id, \
                 idempotency_suffix, created_at_ms FROM provider_action_logs WHERE id = ?1",
            )
            .bind(action_id)
            .fetch_optional(&self.pool)
            .await
            .context("load SQLite automation diagnostic detail")?;
            let Some((provider, action, trigger, status, phase, message, credit_id, key, at)) = row
            else {
                return Ok(None);
            };
            let mut fields = vec![
                diagnostic_field("Log ID", id, true),
                diagnostic_field("Provider", provider.clone(), false),
                diagnostic_field("Action", action.clone(), false),
                diagnostic_field("Trigger", trigger, false),
                diagnostic_field("Status", status, false),
                diagnostic_field("Phase", phase, false),
            ];
            optional_diagnostic_field(&mut fields, "Credit ID", credit_id, true);
            optional_diagnostic_field(&mut fields, "Idempotency suffix", key, true);
            return Ok(Some(DiagnosticLogDetail {
                id: id.to_owned(),
                kind: "automation".to_owned(),
                occurred_at_ms: at,
                title: format!(
                    "{} {}",
                    diagnostic_title(&provider),
                    action.replace('_', " ")
                ),
                summary: message,
                sections: vec![DiagnosticLogSection {
                    title: "Automation".to_owned(),
                    fields,
                }],
                evidence: None,
            }));
        }
        let (kind, prefix) = if id.starts_with("provider:") {
            ("provider_error", "provider:")
        } else if id.starts_with("cache:") {
            ("cache_anomaly", "cache:")
        } else if id.starts_with("keepalive:") {
            ("cache_keepalive", "keepalive:")
        } else {
            return Ok(None);
        };
        let Some((machine_id, producer_id, sequence)) = parse_provider_diagnostic_id(id, prefix)
        else {
            return Ok(None);
        };
        let query = format!(
            "SELECT {PROVIDER_USAGE_COLUMNS} FROM provider_usage_events \
             WHERE machine_id = ?1 AND producer_id = ?2 AND sequence = ?3"
        );
        let record: Option<ProviderUsageRecord> = sqlx::query_as(&query)
            .bind(machine_id)
            .bind(producer_id)
            .bind(sequence)
            .fetch_optional(&self.pool)
            .await
            .context("load SQLite provider diagnostic detail")?;
        let Some(record) = record else {
            return Ok(None);
        };
        let previous_query = format!(
            "SELECT {PROVIDER_USAGE_COLUMNS} FROM provider_usage_events \
             WHERE machine_id = ?1 AND producer_id = ?2 AND account_fingerprint = ?3 \
             AND agent = ?4 AND session_fingerprint IS ?5 AND schema_version >= 3 \
             AND request_purpose = 'interactive' AND session_attribution <> 'prefix_root' \
             AND status < 400 AND cache_observation IN ('explicit', 'derived') \
             AND COALESCE(input_tokens, 0) >= 8000 \
             AND cache_hit_tokens + cache_miss_tokens > 0 \
             AND static_prefix_fingerprint IS NOT NULL \
             AND (occurred_at_ms < ?6 OR (occurred_at_ms = ?6 AND sequence < ?7)) \
             ORDER BY occurred_at_ms DESC, sequence DESC LIMIT 1"
        );
        let previous: Option<ProviderUsageRecord> = sqlx::query_as(&previous_query)
            .bind(machine_id)
            .bind(producer_id)
            .bind(&record.account_fingerprint)
            .bind(&record.agent)
            .bind(&record.session_fingerprint)
            .bind(record.occurred_at_ms)
            .bind(record.sequence)
            .fetch_optional(&self.pool)
            .await
            .context("load previous SQLite provider diagnostic event")?;
        let intervening_provider_errors = if let Some(previous) = &previous {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM provider_usage_events WHERE machine_id = ?1 \
                 AND producer_id = ?2 AND account_fingerprint = ?3 AND agent = ?4 \
                 AND session_fingerprint IS ?5 AND request_purpose = 'interactive' \
                 AND occurred_at_ms > ?6 AND occurred_at_ms < ?7 AND status >= 400",
            )
            .bind(machine_id)
            .bind(producer_id)
            .bind(&record.account_fingerprint)
            .bind(&record.agent)
            .bind(&record.session_fingerprint)
            .bind(previous.occurred_at_ms)
            .bind(record.occurred_at_ms)
            .fetch_one(&self.pool)
            .await
            .context("count intervening SQLite provider errors")?
        } else {
            0
        };
        Ok(provider_diagnostic_detail(
            id,
            kind,
            &record,
            previous.as_ref(),
            intervening_provider_errors,
        ))
    }

    pub(super) async fn ingest_provider_usage(
        &self,
        machine_id: &str,
        producer_id: &str,
        events: &[crate::machine_protocol::ProviderUsageEvent],
    ) -> Result<u64> {
        if !valid_machine_id(machine_id)
            || producer_id.is_empty()
            || producer_id.len() > 128
            || events.is_empty()
            || events.len() > 200
        {
            anyhow::bail!("invalid provider usage batch");
        }
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("begin SQLite provider usage batch")?;
        for event in events {
            validate_provider_usage_event(producer_id, event)?;
            insert_provider_usage_event(&mut transaction, machine_id, producer_id, event).await?;
        }
        let last = events.iter().map(|event| event.sequence).max().unwrap_or(0);
        let newest = events
            .iter()
            .max_by_key(|event| event.occurred_at_ms)
            .context("empty provider usage batch")?;
        sqlx::query(
            "INSERT INTO provider_usage_producers (machine_id, producer_id, provider, \
             account_fingerprint, agent, last_sequence, last_occurred_at_ms, last_received_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT (machine_id, producer_id) DO UPDATE SET \
             provider = excluded.provider, account_fingerprint = excluded.account_fingerprint, \
             agent = excluded.agent, last_sequence = max(provider_usage_producers.last_sequence, \
             excluded.last_sequence), last_occurred_at_ms = max( \
             provider_usage_producers.last_occurred_at_ms, excluded.last_occurred_at_ms), \
             last_received_at_ms = excluded.last_received_at_ms",
        )
        .bind(machine_id)
        .bind(producer_id)
        .bind(&newest.provider)
        .bind(&newest.account_fingerprint)
        .bind(&newest.agent)
        .bind(i64::try_from(last).context("usage watermark overflow")?)
        .bind(newest.occurred_at_ms)
        .bind(now_ms())
        .execute(&mut *transaction)
        .await
        .context("upsert SQLite provider usage producer")?;
        transaction
            .commit()
            .await
            .context("commit SQLite provider usage batch")?;
        Ok(last)
    }

    pub(super) async fn provider_usage_summary(
        &self,
        provider: &str,
        days: i32,
        retention_days: i32,
    ) -> Result<serde_json::Value> {
        let timestamp = now_ms();
        let from_ms = timestamp.saturating_sub(i64::from(days).saturating_mul(86_400_000));
        let records =
            load_provider_usage_records(&self.pool, provider, from_ms, timestamp, None, None)
                .await?;
        let breakdown = usage_breakdown(&records);
        let last_day_records = load_provider_usage_records(
            &self.pool,
            provider,
            timestamp.saturating_sub(86_400_000),
            timestamp,
            None,
            None,
        )
        .await?;
        let last_24_hours = usage_breakdown(&last_day_records);
        let daily = daily_usage(&records);
        let producers = recent_provider_usage_coverage(&self.pool, provider, from_ms).await?;
        let retention_from =
            timestamp.saturating_sub(i64::from(retention_days).saturating_mul(86_400_000));
        let available_agents = load_provider_usage_records(
            &self.pool,
            provider,
            retention_from,
            timestamp,
            None,
            None,
        )
        .await?
        .into_iter()
        .map(|record| record.agent)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
        Ok(serde_json::json!({
            "source": "cowboy", "windowField": "occurred_at", "windowDays": days,
            "retentionDays": retention_days, "availableAgents": available_agents,
            "summary": breakdown.summary, "byAgent": breakdown.by_agent,
            "byAgentModel": breakdown.by_agent_model,
            "byAgentBillingModel": breakdown.by_agent_billing_model,
            "byAgentModelFamily": breakdown.by_agent_model_family,
            "byAgentRequestRole": breakdown.by_agent_request_role,
            "byMachine": breakdown.by_machine, "byOperation": breakdown.by_operation,
            "byModel": breakdown.by_model, "byResolvedModel": breakdown.by_resolved_model,
            "byBillingModel": breakdown.by_billing_model,
            "byModelRevision": breakdown.by_model_revision,
            "byModelFamily": breakdown.by_model_family,
            "byRequestRole": breakdown.by_request_role, "byProtocol": breakdown.by_protocol,
            "byClientProtocol": breakdown.by_client_protocol,
            "byUpstreamProtocol": breakdown.by_upstream_protocol,
            "byTranslationMode": breakdown.by_translation_mode,
            "byThinkingMode": breakdown.by_thinking_mode,
            "byReasoningEffort": breakdown.by_reasoning_effort,
            "bySessionAttribution": breakdown.by_session_attribution,
            "byTrafficSource": breakdown.by_traffic_source,
            "byGatewayBuild": breakdown.by_gateway_build,
            "bySchemaVersion": breakdown.by_schema_version,
            "byAgentOperation": breakdown.by_agent_operation, "daily": daily,
            "last24Hours": {
                "summary": last_24_hours.summary,
                "byModel": last_24_hours.by_model,
                "byAgentModel": last_24_hours.by_agent_model,
                "byBillingModel": last_24_hours.by_billing_model,
                "byAgentBillingModel": last_24_hours.by_agent_billing_model,
                "byModelFamily": last_24_hours.by_model_family,
                "byAgentModelFamily": last_24_hours.by_agent_model_family,
            },
            "coverage": { "producers": producers },
        }))
    }

    pub(super) async fn provider_usage_activity(
        &self,
        provider: &str,
        from_ms: i64,
        to_ms: i64,
        agents: &[String],
        model_families: &[String],
    ) -> Result<serde_json::Value> {
        let window_ms = to_ms.saturating_sub(from_ms);
        if provider != "deepseek"
            || !(60_000..=i64::from(30 * 86_400) * 1_000).contains(&window_ms)
            || agents.len() > 2
            || model_families.len() > 2
            || agents
                .iter()
                .any(|value| !matches!(value.as_str(), "codex" | "claude"))
            || model_families
                .iter()
                .any(|value| !matches!(value.as_str(), "flash" | "pro"))
            || agents
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != agents.len()
            || model_families
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != model_families.len()
        {
            anyhow::bail!("invalid provider usage activity filter");
        }
        let agent = (agents.len() == 1).then(|| agents[0].as_str());
        let model_family = (model_families.len() == 1).then(|| model_families[0].as_str());
        let window_seconds = window_ms / 1_000;
        let bucket = if window_seconds <= 86_400 {
            "hour"
        } else {
            "day"
        };
        let records =
            load_provider_usage_records(&self.pool, provider, from_ms, to_ms, agent, model_family)
                .await?;
        let breakdown = usage_breakdown(&records);
        let timeline = timeline_usage(&records, bucket)?;
        let lineage_records = load_provider_usage_records(
            &self.pool,
            provider,
            from_ms.saturating_sub(21_600_000),
            to_ms,
            agent,
            None,
        )
        .await?
        .into_iter()
        .filter(|record| record.request_purpose == "interactive")
        .collect::<Vec<_>>();
        let low_hit = low_hit_breakdown(&lineage_records, from_ms, model_family);
        let producers = provider_usage_coverage(&self.pool, provider, &records, agent).await?;
        Ok(serde_json::json!({
            "source": "cowboy", "windowField": "occurred_at", "retentionDays": 30,
            "windowSeconds": window_seconds, "fromMs": from_ms, "toMs": to_ms,
            "bucket": bucket,
            "filters": { "agents": agents, "modelFamilies": model_families },
            "summary": breakdown.summary, "byAgent": breakdown.by_agent,
            "byAgentModel": breakdown.by_agent_model,
            "byAgentBillingModel": breakdown.by_agent_billing_model,
            "byAgentModelFamily": breakdown.by_agent_model_family,
            "byAgentRequestRole": breakdown.by_agent_request_role,
            "byMachine": breakdown.by_machine, "byOperation": breakdown.by_operation,
            "byModel": breakdown.by_model, "byResolvedModel": breakdown.by_resolved_model,
            "byBillingModel": breakdown.by_billing_model,
            "byModelRevision": breakdown.by_model_revision,
            "byModelFamily": breakdown.by_model_family,
            "byRequestRole": breakdown.by_request_role, "byProtocol": breakdown.by_protocol,
            "byClientProtocol": breakdown.by_client_protocol,
            "byUpstreamProtocol": breakdown.by_upstream_protocol,
            "byTranslationMode": breakdown.by_translation_mode,
            "byThinkingMode": breakdown.by_thinking_mode,
            "byReasoningEffort": breakdown.by_reasoning_effort,
            "bySessionAttribution": breakdown.by_session_attribution,
            "byTrafficSource": breakdown.by_traffic_source,
            "byGatewayBuild": breakdown.by_gateway_build,
            "bySchemaVersion": breakdown.by_schema_version,
            "byAgentOperation": breakdown.by_agent_operation,
            "timeline": timeline,
            "lowHit": {
                "definition": { "minimumInputTokens": 8000, "maximumHitRatePercent": 10 },
                "summary": low_hit.summary,
                "byCause": low_hit.by_cause,
                "byCauseModel": low_hit.by_cause_model,
            },
            "coverage": { "producers": producers },
        }))
    }

    pub(super) async fn purge_provider_usage(&self, retention_days: i32) -> Result<u64> {
        let cutoff = now_ms().saturating_sub(i64::from(retention_days) * 86_400_000);
        let result = sqlx::query("DELETE FROM provider_usage_events WHERE received_at_ms < ?1")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .context("purge SQLite provider usage ledger")?;
        Ok(result.rows_affected())
    }
}
