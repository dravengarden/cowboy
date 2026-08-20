//! Backend-neutral persistence for Cowboy state.
//!
//! [`Store`] is the stable storage API consumed by the rest of the controller.
//! `PostgreSQL` and `SQLite` are private implementations selected from the
//! connection URL; callers never branch on the database kind.
//!
//! - load all sessions + their recent event tails on daemon startup;
//! - append a new session;
//! - UPSERT reduced event batches under their sessions;
//! - update a session's status;
//! - delete a session (cascades events).
//!
//! Writes from the hot path (`Hub::push`) go through an mpsc channel into the
//! bounded background writer task, so a slow DB never blocks WS fan-out or grows
//! memory indefinitely. The writer retries, reports degraded health on loss,
//! and drains on graceful shutdown. A hard crash can still lose the current
//! batch; the alternative couples broadcast latency to DB round-trips.
//!
//! Backend-specific embedded migrations live next to `Cargo.toml` under
//! `./migrations/`. Run [`Store::migrate`] once on startup.

#![warn(clippy::pedantic)]

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::Read as _;
use std::time::Duration;

use anyhow::{Context as _, Result};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use futures::TryStreamExt as _;
use sha2::Digest as _;
use sqlx::Row as _;
use sqlx::postgres::{PgPool, PgPoolOptions};

mod sqlite;

use sqlite::SqliteStorage;

use crate::core::{
    Envelope, Event, QuestionPageSummary, QueuedMessage, SessionMeta, SessionOrigin, Status,
    bound_history_page, question_summary_title,
};

fn valid_machine_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn validate_encryption_public_key(value: &str) -> Result<()> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .context("decoding Machine encryption public key")?;
    anyhow::ensure!(
        bytes.len() == 32,
        "Machine encryption public key must be 32 bytes"
    );
    Ok(())
}

fn ensure_reset_provider(provider: &str) -> Result<()> {
    anyhow::ensure!(
        matches!(provider, "codex" | "xai"),
        "unsupported reset provider"
    );
    Ok(())
}

fn hex_sha256(value: &[u8]) -> String {
    let digest = sha2::Sha256::digest(value);
    digest
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        })
}

/// Strip NUL (`U+0000`) code points from every string (and object key) inside a
/// JSON value, in place.
///
/// Postgres `jsonb` cannot represent `U+0000`: an INSERT/UPDATE carrying one
/// fails with `ERROR: unsupported Unicode escape sequence`, and our write-behind
/// writer then drops the whole intent (the event or queue mutation is lost,
/// logged as "store writer failed an intent"). Agent stdout and pasted prompts
/// occasionally carry stray NUL bytes (terminal control noise); they carry no
/// meaning in our stored text, so we drop them rather than lose the row. Call
/// this on any value bound to a `jsonb` column.
fn strip_nul(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) if s.contains('\0') => s.retain(|c| c != '\0'),
        serde_json::Value::Array(arr) => arr.iter_mut().for_each(strip_nul),
        serde_json::Value::Object(map) => {
            // Object keys can also carry NUL; rebuild the map only if needed so
            // the common (clean) path stays allocation-free.
            if map.keys().any(|k| k.contains('\0')) {
                let cleaned: serde_json::Map<String, serde_json::Value> = std::mem::take(map)
                    .into_iter()
                    .map(|(k, mut v)| {
                        strip_nul(&mut v);
                        (k.replace('\0', ""), v)
                    })
                    .collect();
                *map = cleaned;
            } else {
                map.values_mut().for_each(strip_nul);
            }
        }
        _ => {}
    }
}

/// Same as [`strip_nul`] but for a plain `text` column — Postgres `text`/`varchar`
/// reject NUL just like `jsonb`. Allocates only when a NUL is actually present.
fn strip_nul_str(s: &str) -> std::borrow::Cow<'_, str> {
    if s.contains('\0') {
        std::borrow::Cow::Owned(s.replace('\0', ""))
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

/// All persistent state needed to rehydrate a single session after restart.
pub struct LoadedSession {
    pub meta: SessionMeta,
    pub events: Vec<Envelope>,
    /// Total durable rows, including history older than `events`.
    pub event_count: u64,
    /// Whether the loaded tail reaches the first durable row.
    pub reached_start: bool,
    /// Highest `seq + 1` for this session — what Hub uses to stamp the next
    /// event in line.
    pub next_seq: u64,
    /// Persisted send-queue + drafts (cross-terminal sync survives restart).
    pub queue: Vec<QueuedMessage>,
    pub drafts: Vec<QueuedMessage>,
    /// Latest agent-advertised config options, retained for a fresh device.
    pub config_options: Option<serde_json::Value>,
    /// User-selected config values that must survive worker recreation.
    pub config_preferences: serde_json::Value,
    /// Mobile-only code-review workspace state, shared across iPhone/iPad clients.
    pub mobile_review_state: serde_json::Value,
}

#[derive(Clone)]
pub struct Store {
    backend: StorageBackend,
}

#[derive(Clone)]
enum StorageBackend {
    Postgres(PostgresStorage),
    Sqlite(SqliteStorage),
}

#[derive(Clone)]
struct PostgresStorage {
    pool: PgPool,
    artifacts: crate::artifacts::ArtifactStore,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScheduledProviderAction {
    pub fire_at_ms: i64,
    pub idempotency_key: String,
    pub attempt_count: i32,
    pub next_attempt_at_ms: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderActionLog {
    pub id: i64,
    pub provider: String,
    pub action: String,
    pub trigger: String,
    pub status: String,
    pub phase: String,
    pub message: String,
    pub credit_id: Option<String>,
    pub idempotency_suffix: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct RuntimeIncidentWrite {
    pub id: String,
    pub occurred_at_ms: i64,
    pub source: String,
    pub classification: String,
    pub severity: String,
    pub state: String,
    pub summary: String,
    pub fingerprint: String,
    pub session_id: Option<String>,
    pub client_id: Option<String>,
    pub machine_id: Option<String>,
    pub trace_id: Option<String>,
    pub build: Option<String>,
    pub evidence_start_ms: i64,
    pub evidence_end_ms: i64,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct RuntimeIncident {
    pub id: String,
    pub occurred_at_ms: i64,
    pub updated_at_ms: i64,
    pub source: String,
    pub classification: String,
    pub severity: String,
    pub state: String,
    pub summary: String,
    pub fingerprint: String,
    pub session_id: Option<String>,
    pub client_id: Option<String>,
    pub machine_id: Option<String>,
    pub trace_id: Option<String>,
    pub build: Option<String>,
    pub evidence_start_ms: i64,
    pub evidence_end_ms: i64,
    pub detail: serde_json::Value,
    pub recovered_at_ms: Option<i64>,
    pub recovery_outcome: Option<String>,
}

/// Closed, bounded query accepted by the unified diagnostic log read model.
#[derive(Debug, Clone)]
pub struct DiagnosticLogFilter {
    pub since_ms: i64,
    pub until_ms: i64,
    pub kinds: Vec<String>,
    pub severities: Vec<String>,
    pub states: Vec<String>,
    pub agents: Vec<String>,
    pub session_ref: Option<String>,
    pub cursor_ms: Option<i64>,
    pub cursor_id: Option<String>,
    pub limit: i64,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct DiagnosticLogSummary {
    pub id: String,
    pub occurred_at_ms: i64,
    pub kind: String,
    pub severity: String,
    pub state: String,
    pub title: String,
    pub summary: String,
    pub session_ref: Option<String>,
    pub provider: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub classification: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticLogDetail {
    pub id: String,
    pub kind: String,
    pub occurred_at_ms: i64,
    pub title: String,
    pub summary: String,
    pub sections: Vec<DiagnosticLogSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticLogSection {
    pub title: String,
    pub fields: Vec<DiagnosticLogField>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticLogField {
    pub label: String,
    pub value: String,
    pub copyable: bool,
}

fn diagnostic_field(
    label: impl Into<String>,
    value: impl Into<String>,
    copyable: bool,
) -> DiagnosticLogField {
    DiagnosticLogField {
        label: label.into(),
        value: value.into(),
        copyable,
    }
}

fn optional_diagnostic_field(
    fields: &mut Vec<DiagnosticLogField>,
    label: &str,
    value: Option<impl Into<String>>,
    copyable: bool,
) {
    if let Some(value) = value {
        let value = value.into();
        if !value.is_empty() {
            fields.push(diagnostic_field(label, value, copyable));
        }
    }
}

fn diagnostic_title(value: &str) -> String {
    if value == "xai" {
        return "xAI".to_owned();
    }
    let mut words = value.split('_').filter(|word| !word.is_empty());
    let Some(first) = words.next() else {
        return "Diagnostic event".to_owned();
    };
    let mut title = first.to_owned();
    if let Some(initial) = title.get_mut(0..1) {
        initial.make_ascii_uppercase();
    }
    for word in words {
        title.push(' ');
        title.push_str(word);
    }
    title
}

fn parse_provider_diagnostic_id<'a>(id: &'a str, prefix: &str) -> Option<(&'a str, &'a str, i64)> {
    let rest = id.strip_prefix(prefix)?;
    let (identity, sequence) = rest.rsplit_once(':')?;
    let (machine_id, producer_id) = identity.split_once(':')?;
    let sequence = sequence.parse().ok()?;
    (!machine_id.is_empty() && !producer_id.is_empty()).then_some((
        machine_id,
        producer_id,
        sequence,
    ))
}

fn json_scalar(value: &serde_json::Value, key: &str) -> Option<String> {
    match value.get(key)? {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn json_i64(value: &serde_json::Value, key: &str) -> Option<i64> {
    value.get(key)?.as_i64()
}

fn cache_transition_cause(
    current: &serde_json::Value,
    previous: &serde_json::Value,
    intervening_provider_error: bool,
) -> &'static str {
    let changed = |key: &str| current.get(key) != previous.get(key);
    if json_scalar(current, "operation").as_deref() == Some("compact") {
        "client_compaction"
    } else if changed("gateway_build") {
        "gateway_build_changed"
    } else if changed("gateway_boot_id") {
        "post_gateway_restart"
    } else if changed("model_family") {
        "model_changed"
    } else if changed("model_revision") {
        "model_revision_changed"
    } else if changed("request_role") {
        "request_role_changed"
    } else if changed("upstream_protocol") {
        "protocol_changed"
    } else if changed("translation_mode") {
        "translation_changed"
    } else if changed("thinking_mode") || changed("reasoning_effort") {
        "reasoning_configuration_changed"
    } else if json_i64(current, "compatibility_fixes").is_some_and(|value| value > 0) {
        "compatibility_rewrite"
    } else if changed("static_prefix_fingerprint") {
        "static_prefix_changed"
    } else if (json_scalar(current, "agent").as_deref() != Some("codex")
        || current
            .get("has_previous_response_id")
            .and_then(serde_json::Value::as_bool)
            != Some(true))
        && json_i64(current, "input_item_count")
            .zip(json_i64(previous, "input_item_count"))
            .is_some_and(|(current, previous)| current < previous)
    {
        "history_rewrite"
    } else if intervening_provider_error {
        "post_provider_error"
    } else if json_scalar(current, "request_prefix_fingerprint").is_some()
        && current.get("request_prefix_fingerprint") == previous.get("request_prefix_fingerprint")
    {
        "unexpected_exact_prefix_miss"
    } else {
        "unexpected_active_cache_drop"
    }
}

fn cache_transition_title(cause: &str) -> &'static str {
    match cause {
        "static_prefix_changed" => "Cache prefix changed",
        "unexpected_exact_prefix_miss" => "Exact-prefix cache miss",
        "history_rewrite" => "Cached history rewritten",
        "post_gateway_restart" => "Cache lost after gateway restart",
        "gateway_build_changed" => "Cache lost after gateway update",
        "client_compaction" => "Cache changed after compaction",
        "model_changed" => "Cache lost after model change",
        "model_revision_changed" => "Cache lost after model revision",
        "request_role_changed" => "Cache lost after role change",
        "protocol_changed" => "Cache lost after protocol change",
        "translation_changed" => "Cache lost after translation change",
        "reasoning_configuration_changed" => "Cache lost after reasoning change",
        "compatibility_rewrite" => "Cache lost after compatibility rewrite",
        "post_provider_error" => "Cache lost after provider error",
        _ => "Unexplained active-session cache drop",
    }
}

fn provider_status_impact(status: i64) -> (&'static str, bool) {
    if matches!(status, 401..=403) {
        ("Critical blocker", true)
    } else if (400..=499).contains(&status) && !matches!(status, 408 | 425 | 429 | 499) {
        ("Blocking request failure", true)
    } else {
        ("Retryable provider attempt", false)
    }
}

fn cache_rate_label(value: &serde_json::Value) -> Option<String> {
    let hit = json_i64(value, "cache_hit_tokens")?;
    let miss = json_i64(value, "cache_miss_tokens")?;
    let total = hit.checked_add(miss)?;
    if total <= 0 {
        return None;
    }
    let tenths = hit.checked_mul(1_000)?.checked_div(total)?;
    Some(format!("{}.{:01}", tenths / 10, tenths % 10))
}

fn provider_detail_matches_kind(
    kind: &str,
    current: &serde_json::Value,
    previous: &serde_json::Value,
    gap_ms: Option<i64>,
) -> bool {
    if kind == "provider_error" {
        return json_i64(current, "status").is_some_and(|status| status >= 400);
    }
    if kind != "cache_anomaly"
        || json_i64(current, "schema_version").is_none_or(|version| version < 3)
        || json_scalar(current, "session_fingerprint").is_none()
        || !matches!(
            json_scalar(current, "session_attribution").as_deref(),
            Some("response_lineage" | "explicit")
        )
        || !matches!(
            json_scalar(current, "cache_observation").as_deref(),
            Some("explicit" | "derived")
        )
        || json_scalar(current, "static_prefix_fingerprint").is_none()
        || json_scalar(previous, "static_prefix_fingerprint").is_none()
        || !gap_ms.is_some_and(|gap| (0..=30 * 60 * 1_000).contains(&gap))
        || json_i64(current, "status").is_none_or(|status| status >= 400)
        || json_i64(previous, "status").is_none_or(|status| status >= 400)
        || json_i64(current, "input_tokens").is_none_or(|tokens| tokens < 8_000)
        || json_i64(previous, "input_tokens").is_none_or(|tokens| tokens < 8_000)
    {
        return false;
    }
    let Some((current_hit, current_miss, previous_hit, previous_miss)) =
        json_i64(current, "cache_hit_tokens")
            .zip(json_i64(current, "cache_miss_tokens"))
            .zip(
                json_i64(previous, "cache_hit_tokens").zip(json_i64(previous, "cache_miss_tokens")),
            )
            .map(
                |((current_hit, current_miss), (previous_hit, previous_miss))| {
                    (current_hit, current_miss, previous_hit, previous_miss)
                },
            )
    else {
        return false;
    };
    let current_total = i128::from(current_hit) + i128::from(current_miss);
    let previous_total = i128::from(previous_hit) + i128::from(previous_miss);
    current_total > 0
        && previous_total > 0
        && i128::from(current_hit) * 10 < current_total
        && i128::from(previous_hit) * 10 >= previous_total * 9
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MachineRecord {
    pub id: String,
    pub display_name: String,
    pub connection_mode: String,
    pub platform: String,
    pub architecture: String,
    pub status: String,
    pub inventory: serde_json::Value,
    pub last_seen_at_ms: Option<i64>,
    pub revoked: bool,
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EnrolledMachine {
    pub id: String,
    pub display_name: String,
    pub fingerprint: String,
}

#[derive(sqlx::FromRow)]
struct MachineRow {
    id: String,
    display_name: String,
    connection_mode: String,
    platform: String,
    architecture: String,
    status: String,
    inventory: serde_json::Value,
    last_seen_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    public_key: Option<String>,
}

macro_rules! dispatch_storage {
    ($store:expr, $method:ident($($argument:expr),* $(,)?)) => {
        match &$store.backend {
            StorageBackend::Postgres(backend) => backend.$method($($argument),*).await,
            StorageBackend::Sqlite(backend) => backend.$method($($argument),*).await,
        }
    };
}

impl Store {
    /// Connect to the durable store selected by the URL scheme.
    ///
    /// # Errors
    /// Returns when the URL scheme is unsupported or the selected database
    /// cannot be opened.
    pub async fn connect(url: &str, artifact_dir: std::path::PathBuf) -> Result<Self> {
        let backend = if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            StorageBackend::Postgres(PostgresStorage::connect(url, artifact_dir).await?)
        } else if url.starts_with("sqlite:") {
            StorageBackend::Sqlite(SqliteStorage::connect(url, artifact_dir).await?)
        } else {
            anyhow::bail!("unsupported database URL scheme")
        };
        Ok(Self { backend })
    }

    pub async fn migrate(&self) -> Result<()> {
        dispatch_storage!(self, migrate())
    }

    pub async fn create_machine_enrollment(
        &self,
        machine_id: &str,
        display_name: &str,
        ttl_seconds: i64,
    ) -> Result<String> {
        dispatch_storage!(
            self,
            create_machine_enrollment(machine_id, display_name, ttl_seconds)
        )
    }

    pub async fn consume_machine_enrollment(
        &self,
        token: &str,
        public_key: &str,
        encryption_public_key: &str,
    ) -> Result<EnrolledMachine> {
        dispatch_storage!(
            self,
            consume_machine_enrollment(token, public_key, encryption_public_key)
        )
    }

    pub async fn machine_public_key(&self, machine_id: &str) -> Result<Option<String>> {
        dispatch_storage!(self, machine_public_key(machine_id))
    }

    pub async fn machine_encryption_public_key(&self, machine_id: &str) -> Result<Option<String>> {
        dispatch_storage!(self, machine_encryption_public_key(machine_id))
    }

    /// Bind the first challenge-proven X25519 key for an already enrolled
    /// signing identity, or assert that a previously bound key is unchanged.
    pub async fn bind_machine_encryption_public_key(
        &self,
        machine_id: &str,
        encryption_public_key: &str,
    ) -> Result<()> {
        dispatch_storage!(
            self,
            bind_machine_encryption_public_key(machine_id, encryption_public_key)
        )
    }

    pub async fn list_machines(&self) -> Result<Vec<MachineRecord>> {
        dispatch_storage!(self, list_machines())
    }

    pub async fn machine_is_local(&self, machine_id: &str) -> Result<bool> {
        dispatch_storage!(self, machine_is_local(machine_id))
    }

    pub async fn revoke_machine(&self, machine_id: &str) -> Result<()> {
        dispatch_storage!(self, revoke_machine(machine_id))
    }

    pub async fn machine_connection_is_current(
        &self,
        machine_id: &str,
        connection_epoch: &str,
    ) -> Result<bool> {
        dispatch_storage!(
            self,
            machine_connection_is_current(machine_id, connection_epoch)
        )
    }

    pub async fn machine_connected(
        &self,
        machine_id: &str,
        connection_epoch: &str,
        platform: &str,
        architecture: &str,
        connection_mode: &str,
        inventory: &serde_json::Value,
    ) -> Result<()> {
        dispatch_storage!(
            self,
            machine_connected(
                machine_id,
                connection_epoch,
                platform,
                architecture,
                connection_mode,
                inventory
            )
        )
    }

    pub async fn machine_seen(
        &self,
        machine_id: &str,
        connection_epoch: &str,
        inventory: Option<&serde_json::Value>,
    ) -> Result<()> {
        dispatch_storage!(self, machine_seen(machine_id, connection_epoch, inventory))
    }

    pub async fn machine_disconnected(
        &self,
        machine_id: &str,
        connection_epoch: &str,
        grace_seconds: i32,
    ) -> Result<()> {
        dispatch_storage!(
            self,
            machine_disconnected(machine_id, connection_epoch, grace_seconds)
        )
    }

    pub async fn expire_machine_reconnects(&self) -> Result<u64> {
        dispatch_storage!(self, expire_machine_reconnects())
    }

    pub async fn upsert_provider_reset(
        &self,
        provider: &str,
        fire_at_ms: i64,
        idempotency_key: &str,
    ) -> Result<()> {
        ensure_reset_provider(provider)?;
        dispatch_storage!(
            self,
            upsert_provider_reset(provider, fire_at_ms, idempotency_key)
        )
    }

    pub async fn load_provider_reset(
        &self,
        provider: &str,
    ) -> Result<Option<ScheduledProviderAction>> {
        ensure_reset_provider(provider)?;
        dispatch_storage!(self, load_provider_reset(provider))
    }

    pub async fn defer_provider_reset(
        &self,
        provider: &str,
        idempotency_key: &str,
        next_attempt_at_ms: i64,
    ) -> Result<()> {
        ensure_reset_provider(provider)?;
        dispatch_storage!(
            self,
            defer_provider_reset(provider, idempotency_key, next_attempt_at_ms)
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn append_provider_action_log(
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
        ensure_reset_provider(provider)?;
        dispatch_storage!(
            self,
            append_provider_action_log(
                provider,
                trigger,
                status,
                phase,
                message,
                credit_id,
                idempotency_key,
                created_at_ms
            )
        )
    }

    pub async fn provider_action_logs(&self, limit: i64) -> Result<Vec<ProviderActionLog>> {
        dispatch_storage!(self, provider_action_logs(limit))
    }

    pub async fn delete_provider_reset(&self, provider: &str) -> Result<()> {
        ensure_reset_provider(provider)?;
        dispatch_storage!(self, delete_provider_reset(provider))
    }

    pub async fn claim_provider_reset(
        &self,
        provider: &str,
        idempotency_key: &str,
    ) -> Result<bool> {
        ensure_reset_provider(provider)?;
        dispatch_storage!(self, claim_provider_reset(provider, idempotency_key))
    }

    pub async fn next_session_number(&self) -> Result<u64> {
        dispatch_storage!(self, next_session_number())
    }

    pub async fn load_all(&self) -> Result<Vec<LoadedSession>> {
        dispatch_storage!(self, load_all())
    }

    pub async fn history_page(
        &self,
        session_id: &str,
        before_seq: u64,
        page_size: usize,
    ) -> Result<(Vec<Envelope>, Option<u64>, bool)> {
        dispatch_storage!(self, history_page(session_id, before_seq, page_size))
    }

    pub async fn question_page_before(
        &self,
        session_id: &str,
        before_seq: u64,
    ) -> Result<(Vec<Envelope>, Option<u64>, bool)> {
        dispatch_storage!(self, question_page_before(session_id, before_seq))
    }

    pub async fn question_page_summaries(
        &self,
        session_id: &str,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<QuestionPageSummary>, Option<u64>, u64)> {
        dispatch_storage!(self, question_page_summaries(session_id, before_seq, limit))
    }

    pub async fn question_page_at(
        &self,
        session_id: &str,
        root_seq: u64,
    ) -> Result<Option<Vec<Envelope>>> {
        dispatch_storage!(self, question_page_at(session_id, root_seq))
    }

    pub async fn insert_session(&self, meta: &SessionMeta) -> Result<()> {
        dispatch_storage!(self, insert_session(meta))
    }

    pub async fn update_status(&self, session_id: &str, status: Status) -> Result<()> {
        dispatch_storage!(self, update_status(session_id, status))
    }

    pub async fn update_agent_session_id(
        &self,
        session_id: &str,
        agent_session_id: Option<&str>,
    ) -> Result<()> {
        dispatch_storage!(self, update_agent_session_id(session_id, agent_session_id))
    }

    pub async fn update_config_options(
        &self,
        session_id: &str,
        options: &serde_json::Value,
    ) -> Result<()> {
        dispatch_storage!(self, update_config_options(session_id, options))
    }

    pub async fn update_config_preferences(
        &self,
        session_id: &str,
        preferences: &serde_json::Value,
    ) -> Result<()> {
        dispatch_storage!(self, update_config_preferences(session_id, preferences))
    }

    pub async fn update_title(&self, session_id: &str, title: &str) -> Result<()> {
        dispatch_storage!(self, update_title(session_id, title))
    }

    pub async fn update_cwd(&self, session_id: &str, cwd: &str, title: Option<&str>) -> Result<()> {
        dispatch_storage!(self, update_cwd(session_id, cwd, title))
    }

    pub async fn update_mobile_review_state(
        &self,
        session_id: &str,
        value: &serde_json::Value,
    ) -> Result<()> {
        dispatch_storage!(self, update_mobile_review_state(session_id, value))
    }

    pub async fn upsert_event_batch(
        &self,
        events: &[Envelope],
        highwaters: &HashMap<String, u64>,
    ) -> Result<()> {
        dispatch_storage!(self, upsert_event_batch(events, highwaters))
    }

    pub async fn clear_events(&self, session_id: &str) -> Result<()> {
        dispatch_storage!(self, clear_events(session_id))
    }

    pub fn artifact_path(&self, name: &str) -> Option<std::path::PathBuf> {
        match &self.backend {
            StorageBackend::Postgres(backend) => backend.artifact_path(name),
            StorageBackend::Sqlite(backend) => backend.artifact_path(name),
        }
    }

    pub async fn update_pending(
        &self,
        session_id: &str,
        queue: &[QueuedMessage],
        drafts: &[QueuedMessage],
    ) -> Result<()> {
        dispatch_storage!(self, update_pending(session_id, queue, drafts))
    }

    pub async fn upsert_wakeup(
        &self,
        session_id: &str,
        fire_at_ms: i64,
        prompt: &str,
    ) -> Result<()> {
        dispatch_storage!(self, upsert_wakeup(session_id, fire_at_ms, prompt))
    }

    pub async fn delete_wakeup(&self, session_id: &str) -> Result<()> {
        dispatch_storage!(self, delete_wakeup(session_id))
    }

    pub async fn load_wakeups(&self) -> Result<Vec<(String, i64, String)>> {
        dispatch_storage!(self, load_wakeups())
    }

    pub async fn update_session_order(&self, order: &[String]) -> Result<()> {
        dispatch_storage!(self, update_session_order(order))
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        dispatch_storage!(self, delete_session(session_id))
    }

    pub async fn soft_delete_sessions_until(
        &self,
        session_ids: &[String],
        purge_after_ms: i64,
    ) -> Result<()> {
        dispatch_storage!(
            self,
            soft_delete_sessions_until(session_ids, purge_after_ms)
        )
    }

    pub async fn purge_deleted(&self, retention_days: i64) -> Result<u64> {
        dispatch_storage!(self, purge_deleted(retention_days))
    }

    pub async fn upsert_runtime_incident(&self, incident: &RuntimeIncidentWrite) -> Result<()> {
        dispatch_storage!(self, upsert_runtime_incident(incident))
    }

    pub async fn recover_runtime_incident(
        &self,
        session_id: &str,
        recovered_at_ms: i64,
        outcome: &str,
    ) -> Result<u64> {
        dispatch_storage!(
            self,
            recover_runtime_incident(session_id, recovered_at_ms, outcome)
        )
    }

    pub async fn runtime_incidents(&self, limit: i64) -> Result<Vec<RuntimeIncident>> {
        dispatch_storage!(self, runtime_incidents(limit))
    }

    pub async fn diagnostic_logs(
        &self,
        filter: &DiagnosticLogFilter,
    ) -> Result<Vec<DiagnosticLogSummary>> {
        dispatch_storage!(self, diagnostic_logs(filter))
    }

    pub async fn diagnostic_log_detail(&self, id: &str) -> Result<Option<DiagnosticLogDetail>> {
        dispatch_storage!(self, diagnostic_log_detail(id))
    }

    pub async fn storage_metrics(&self) -> Result<(i64, i64, i64)> {
        dispatch_storage!(self, storage_metrics())
    }

    pub async fn ingest_provider_usage(
        &self,
        machine_id: &str,
        producer_id: &str,
        events: &[crate::machine_protocol::ProviderUsageEvent],
    ) -> Result<u64> {
        dispatch_storage!(self, ingest_provider_usage(machine_id, producer_id, events))
    }

    pub async fn provider_usage_summary(
        &self,
        provider: &str,
        days: i32,
        retention_days: i32,
    ) -> Result<serde_json::Value> {
        dispatch_storage!(self, provider_usage_summary(provider, days, retention_days))
    }

    pub async fn provider_usage_activity(
        &self,
        provider: &str,
        from_ms: i64,
        to_ms: i64,
        agents: &[String],
        model_families: &[String],
    ) -> Result<serde_json::Value> {
        dispatch_storage!(
            self,
            provider_usage_activity(provider, from_ms, to_ms, agents, model_families)
        )
    }

    pub async fn purge_provider_usage(&self, retention_days: i32) -> Result<u64> {
        dispatch_storage!(self, purge_provider_usage(retention_days))
    }
}

impl PostgresStorage {
    /// Create a short-lived, single-use Machine enrollment secret. Only its
    /// SHA-256 digest is persisted.
    ///
    /// # Errors
    /// Returns when secure randomness cannot be read or the database rejects
    /// the requested machine identity.
    pub async fn create_machine_enrollment(
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
            "SELECT EXISTS(SELECT 1 FROM machines WHERE id = $1 AND public_key IS NOT NULL AND revoked_at IS NULL)",
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
        sqlx::query(
            "INSERT INTO machine_enrollment_tokens \
             (token_hash, machine_id, display_name, expires_at) \
             VALUES ($1, $2, $3, now() + make_interval(secs => $4::double precision)) \
             ON CONFLICT (machine_id) DO UPDATE SET token_hash = EXCLUDED.token_hash, \
             display_name = EXCLUDED.display_name, expires_at = EXCLUDED.expires_at, \
             used_at = NULL, created_at = now()",
        )
        .bind(token_hash)
        .bind(machine_id)
        .bind(display_name.trim())
        .bind(ttl_seconds)
        .execute(&self.pool)
        .await
        .context("creating Machine enrollment")?;
        Ok(token)
    }

    /// Atomically consume an enrollment token and bind a public key to the
    /// requested stable machine id.
    ///
    /// # Errors
    /// Returns when the token is invalid/expired/used or persistence fails.
    pub async fn consume_machine_enrollment(
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
        let token_hash = hex_sha256(token.as_bytes());
        let row: Option<(String, String)> = sqlx::query_as(
            "UPDATE machine_enrollment_tokens SET used_at = now() \
             WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now() \
             RETURNING machine_id, display_name",
        )
        .bind(token_hash)
        .fetch_optional(&mut *transaction)
        .await
        .context("consuming Machine enrollment")?;
        let (id, display_name) =
            row.context("invalid, expired, or already used enrollment token")?;
        let result = sqlx::query(
            "INSERT INTO machines \
             (id, display_name, connection_mode, platform, architecture, status, public_key, \
              encryption_public_key, enrolled_at) \
             VALUES ($1, $2, 'outbound_wss', 'unknown', 'unknown', 'offline', $3, $4, now()) \
             ON CONFLICT (id) DO UPDATE SET display_name = EXCLUDED.display_name, \
             connection_mode = 'outbound_wss', public_key = EXCLUDED.public_key, \
             encryption_public_key = EXCLUDED.encryption_public_key, \
             enrolled_at = now(), revoked_at = NULL, status = 'offline', \
             connection_epoch = NULL, reconnect_deadline_at = NULL, updated_at = now() \
             WHERE machines.public_key IS NULL OR \
             (machines.revoked_at IS NOT NULL AND machines.public_key IS DISTINCT FROM EXCLUDED.public_key)",
        )
        .bind(&id)
        .bind(&display_name)
        .bind(public_key)
        .bind(encryption_public_key)
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

    /// Load the active public key for a Machine.
    ///
    /// # Errors
    /// Returns when the database query fails.
    pub async fn machine_public_key(&self, machine_id: &str) -> Result<Option<String>> {
        let value: Option<Option<String>> = sqlx::query_scalar(
            "SELECT public_key FROM machines WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(machine_id)
        .fetch_optional(&self.pool)
        .await
        .context("loading Machine public key")?;
        Ok(value.flatten())
    }

    pub async fn machine_encryption_public_key(&self, machine_id: &str) -> Result<Option<String>> {
        let value: Option<Option<String>> = sqlx::query_scalar(
            "SELECT encryption_public_key FROM machines WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(machine_id)
        .fetch_optional(&self.pool)
        .await
        .context("loading Machine encryption public key")?;
        Ok(value.flatten())
    }

    pub async fn bind_machine_encryption_public_key(
        &self,
        machine_id: &str,
        encryption_public_key: &str,
    ) -> Result<()> {
        validate_encryption_public_key(encryption_public_key)?;
        let result = sqlx::query(
            "UPDATE machines SET encryption_public_key = $2, updated_at = now() \
             WHERE id = $1 AND revoked_at IS NULL \
             AND (encryption_public_key IS NULL OR encryption_public_key = $2)",
        )
        .bind(machine_id)
        .bind(encryption_public_key)
        .execute(&self.pool)
        .await
        .context("binding Machine encryption public key")?;
        anyhow::ensure!(
            result.rows_affected() == 1,
            "Machine encryption public key changed; revoke and re-enroll the Machine"
        );
        Ok(())
    }

    /// List enrolled Machines, including revoked records for administrative UI.
    ///
    /// # Errors
    /// Returns when the database query fails.
    pub async fn list_machines(&self) -> Result<Vec<MachineRecord>> {
        let rows: Vec<MachineRow> = sqlx::query_as(
            "SELECT id, display_name, connection_mode, platform, architecture, status, \
             inventory, last_seen_at, revoked_at, public_key FROM machines ORDER BY id = 'local' DESC, display_name",
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
                last_seen_at_ms: row.last_seen_at.map(|value| value.timestamp_millis()),
                revoked: row.revoked_at.is_some(),
                fingerprint: row
                    .public_key
                    .as_deref()
                    .and_then(|key| crate::machine_auth::fingerprint(key).ok()),
            })
            .collect())
    }

    /// Whether a registered Machine is colocated with this controller. This is
    /// used only as a bounded fallback when its loopback adapter tunnel is
    /// temporarily unavailable; remote Machines must never fall through to the
    /// controller filesystem merely because they share a path spelling.
    pub async fn machine_is_local(&self, machine_id: &str) -> Result<bool> {
        let mode: Option<String> = sqlx::query_scalar(
            "SELECT connection_mode FROM machines WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(machine_id)
        .fetch_optional(&self.pool)
        .await
        .context("loading Machine connection mode")?;
        Ok(mode.as_deref() == Some("local"))
    }

    /// Revoke a remote Machine identity and fence its current connection.
    /// The active controller observes the cleared epoch on its next bounded
    /// revocation check and closes the socket.
    ///
    /// # Errors
    /// Returns when the id is reserved, unknown, or persistence fails.
    pub async fn revoke_machine(&self, machine_id: &str) -> Result<()> {
        anyhow::ensure!(machine_id != "local", "the local Machine cannot be revoked");
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("starting Machine revocation")?;
        let result = sqlx::query(
            "UPDATE machines SET revoked_at = now(), status = 'offline', \
             connection_epoch = NULL, reconnect_deadline_at = NULL, updated_at = now() \
             WHERE id = $1 AND public_key IS NOT NULL AND revoked_at IS NULL",
        )
        .bind(machine_id)
        .execute(&mut *transaction)
        .await
        .context("revoking Machine identity")?;
        anyhow::ensure!(
            result.rows_affected() == 1,
            "unknown or already revoked Machine"
        );
        sqlx::query("DELETE FROM machine_enrollment_tokens WHERE machine_id = $1")
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

    /// Test whether a connection epoch still owns an active Machine identity.
    ///
    /// # Errors
    /// Returns when persistence cannot be queried.
    pub async fn machine_connection_is_current(
        &self,
        machine_id: &str,
        connection_epoch: &str,
    ) -> Result<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM machines WHERE id = $1 \
             AND connection_epoch = $2 AND revoked_at IS NULL)",
        )
        .bind(machine_id)
        .bind(connection_epoch)
        .fetch_one(&self.pool)
        .await
        .context("checking Machine connection epoch")
    }

    /// Record an authenticated Machine connection and its current inventory.
    ///
    /// # Errors
    /// Returns when the database update fails.
    pub async fn machine_connected(
        &self,
        machine_id: &str,
        connection_epoch: &str,
        platform: &str,
        architecture: &str,
        connection_mode: &str,
        inventory: &serde_json::Value,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE machines SET connection_epoch = $2, platform = $3, architecture = $4, \
             connection_mode = $5, status = 'online', inventory = $6, last_seen_at = now(), \
             reconnect_deadline_at = NULL, updated_at = now() \
             WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(machine_id)
        .bind(connection_epoch)
        .bind(platform)
        .bind(architecture)
        .bind(connection_mode)
        .bind(inventory)
        .execute(&self.pool)
        .await
        .context("recording Machine connection")?;
        anyhow::ensure!(
            result.rows_affected() == 1,
            "Machine was revoked during authentication"
        );
        Ok(())
    }

    /// Refresh the liveness timestamp and optional inventory of a Machine.
    ///
    /// # Errors
    /// Returns when the database update fails.
    pub async fn machine_seen(
        &self,
        machine_id: &str,
        connection_epoch: &str,
        inventory: Option<&serde_json::Value>,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE machines SET status = 'online', last_seen_at = now(), \
             inventory = COALESCE($3, inventory), reconnect_deadline_at = NULL, updated_at = now() \
             WHERE id = $1 AND connection_epoch = $2 AND revoked_at IS NULL",
        )
        .bind(machine_id)
        .bind(connection_epoch)
        .bind(inventory)
        .execute(&self.pool)
        .await
        .context("refreshing Machine liveness")?;
        anyhow::ensure!(
            result.rows_affected() == 1,
            "Machine connection is no longer current"
        );
        Ok(())
    }

    /// Mark a disconnected Machine as reconnecting for a bounded grace period.
    ///
    /// # Errors
    /// Returns when the database update fails.
    pub async fn machine_disconnected(
        &self,
        machine_id: &str,
        connection_epoch: &str,
        grace_seconds: i32,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE machines SET status = 'reconnecting', connection_epoch = NULL, \
             reconnect_deadline_at = now() + $3::integer * interval '1 second', updated_at = now() \
             WHERE id = $1 AND connection_epoch = $2 AND revoked_at IS NULL",
        )
        .bind(machine_id)
        .bind(connection_epoch)
        .bind(grace_seconds)
        .execute(&self.pool)
        .await
        .context("recording Machine disconnect")?;
        Ok(())
    }

    /// Expire reconnect grace windows that were not superseded by a new epoch.
    ///
    /// Returns the number of Machines that became offline.
    ///
    /// # Errors
    /// Returns when the database update fails.
    pub async fn expire_machine_reconnects(&self) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE machines SET status = 'offline', reconnect_deadline_at = NULL, updated_at = now() \
             WHERE status = 'reconnecting' AND reconnect_deadline_at <= now() \
             AND connection_epoch IS NULL AND revoked_at IS NULL",
        )
        .execute(&self.pool)
        .await
        .context("expiring Machine reconnect grace")?;
        Ok(result.rows_affected())
    }

    pub async fn upsert_provider_reset(
        &self,
        provider: &str,
        fire_at_ms: i64,
        idempotency_key: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO scheduled_provider_actions (provider, action, fire_at_ms, idempotency_key) \
             VALUES ($1, 'rate_limit_reset', $2, $3) \
             ON CONFLICT (provider) DO UPDATE SET action = EXCLUDED.action, \
             fire_at_ms = EXCLUDED.fire_at_ms, idempotency_key = EXCLUDED.idempotency_key, \
             attempt_count = 0, next_attempt_at_ms = EXCLUDED.fire_at_ms",
        )
        .bind(provider)
        .bind(fire_at_ms)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPSERT scheduled {provider} reset"))?;
        Ok(())
    }

    pub async fn load_provider_reset(
        &self,
        provider: &str,
    ) -> Result<Option<ScheduledProviderAction>> {
        let row: Option<(i64, String, i32, Option<i64>)> = sqlx::query_as(
            "SELECT fire_at_ms, idempotency_key, attempt_count, next_attempt_at_ms FROM scheduled_provider_actions \
             WHERE provider = $1 AND action = 'rate_limit_reset'",
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

    pub async fn defer_provider_reset(
        &self,
        provider: &str,
        idempotency_key: &str,
        next_attempt_at_ms: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE scheduled_provider_actions SET attempt_count = attempt_count + 1, \
             next_attempt_at_ms = $3 WHERE provider = $1 AND action = 'rate_limit_reset' \
             AND idempotency_key = $2",
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
    pub async fn append_provider_action_log(
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
             VALUES ($1, 'rate_limit_reset', $2, $3, $4, $5, $6, $7, $8)",
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

    pub async fn provider_action_logs(&self, limit: i64) -> Result<Vec<ProviderActionLog>> {
        sqlx::query_as::<
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
             ORDER BY created_at_ms DESC, id DESC LIMIT $1",
        )
        .bind(limit.clamp(1, 200))
        .fetch_all(&self.pool)
        .await
        .context("list provider action logs")
        .map(|rows| {
            rows.into_iter()
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
                .collect()
        })
    }

    pub async fn delete_provider_reset(&self, provider: &str) -> Result<()> {
        sqlx::query("DELETE FROM scheduled_provider_actions WHERE provider = $1")
            .bind(provider)
            .execute(&self.pool)
            .await
            .with_context(|| format!("DELETE scheduled {provider} reset"))?;
        Ok(())
    }

    pub async fn claim_provider_reset(
        &self,
        provider: &str,
        idempotency_key: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM scheduled_provider_actions \
             WHERE provider = $1 AND idempotency_key = $2",
        )
        .bind(provider)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await
        .with_context(|| format!("claim scheduled {provider} reset"))?;
        Ok(result.rows_affected() == 1)
    }
    /// Open a pool against `url`.
    ///
    /// # Errors
    /// If the URL is malformed or the DB is unreachable within the connect
    /// timeout.
    pub async fn connect(url: &str, artifact_dir: std::path::PathBuf) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(5))
            .connect(url)
            .await
            .with_context(|| format!("connecting to postgres {url}"))?;
        Ok(Self {
            pool,
            artifacts: crate::artifacts::ArtifactStore::new(artifact_dir)?,
        })
    }

    /// Run embedded migrations under `./migrations/`. Idempotent.
    ///
    /// # Errors
    /// If a migration fails to apply.
    pub async fn migrate(&self) -> Result<()> {
        let mut migrator = sqlx::migrate!("./migrations");
        // Component rollback restores the previous controller binary but does
        // not roll back PostgreSQL. A predecessor must therefore tolerate an
        // already-applied additive migration from a failed candidate release.
        // Known migration checksums are still verified by SQLx.
        migrator.set_ignore_missing(true);
        migrator
            .run(&self.pool)
            .await
            .context("running migrations")?;
        Ok(())
    }

    /// Return the first numeric `sess-N` id not yet used by any durable row,
    /// including soft-deleted rows that [`Self::load_all`] intentionally hides.
    ///
    /// Seeding only from live/restored sessions can reuse a tombstoned id after
    /// restart. The in-memory Hub then clobbers the new session while Postgres
    /// rejects its INSERT, and later metadata updates corrupt the tombstone.
    pub async fn next_session_number(&self) -> Result<u64> {
        let max: Option<i64> = sqlx::query_scalar(
            "SELECT max((substring(id FROM 6))::bigint) FROM sessions \
             WHERE id ~ '^sess-[0-9]+$'",
        )
        .fetch_one(&self.pool)
        .await
        .context("SELECT max session id")?;
        Ok(max
            .and_then(|value| u64::try_from(value).ok())
            .map_or(1, |value| value.saturating_add(1)))
    }

    /// Load every session with only its recent event tail. Older history remains
    /// in Postgres and is fetched by [`Self::history_page`].
    ///
    /// # Errors
    /// If a query fails or a payload is unparseable.
    pub async fn load_all(&self) -> Result<Vec<LoadedSession>> {
        let session_rows: Vec<SessionRow> = sqlx::query_as::<_, SessionRow>(
            "SELECT id, provider, provider_version, provider_generation_digest, \
             provider_auth_generation, provider_behavior, machine_id, workspace_id, workspace_name, workspace_source_path, \
             cwd, title, origin, status, agent_session_id, \
             system, next_seq, queue, drafts, \
             config_options, config_preferences, mobile_review_state, created_at \
             FROM sessions WHERE deleted_at IS NULL ORDER BY position ASC NULLS LAST, created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .context("SELECT sessions")?;

        let mut out = Vec::with_capacity(session_rows.len());
        for row in session_rows {
            let id = row.id.clone();
            let event_rows: Vec<EventRow> = sqlx::query_as::<_, EventRow>(
                "WITH recent AS MATERIALIZED ( \
                     SELECT seq, payload FROM events \
                     WHERE session_id = $1 ORDER BY seq DESC LIMIT $2 \
                 ), sized AS ( \
                     SELECT seq, payload, \
                            row_number() OVER (ORDER BY seq DESC) AS recent_rank, \
                            sum(octet_length(payload::text) + $4) \
                                OVER (ORDER BY seq DESC) AS cumulative_bytes \
                     FROM recent \
                 ), totals AS ( \
                     SELECT count(*)::bigint AS total_count \
                     FROM events WHERE session_id = $1 \
                 ) \
                 SELECT sized.seq, sized.payload, totals.total_count \
                 FROM sized CROSS JOIN totals \
                 WHERE sized.recent_rank = 1 OR sized.cumulative_bytes <= $3 \
                 ORDER BY sized.seq DESC",
            )
            .bind(&id)
            .bind(i64::try_from(crate::core::HOT_TAIL).unwrap_or(i64::MAX))
            .bind(i64::try_from(crate::core::HOT_TAIL_MAX_BYTES).unwrap_or(i64::MAX))
            .bind(i64::try_from(id.len().saturating_add(64)).unwrap_or(i64::MAX))
            .fetch_all(&self.pool)
            .await
            .with_context(|| format!("SELECT events for {id}"))?;

            let event_count = event_rows
                .first()
                .and_then(|r| u64::try_from(r.total_count).ok())
                .unwrap_or(0);
            let mut reached_start =
                event_count <= u64::try_from(event_rows.len()).unwrap_or(u64::MAX);
            let mut events = Vec::with_capacity(event_rows.len());
            for er in event_rows.into_iter().rev() {
                let seq_for_log = er.seq;
                // Degrade a single undecodable event to a SKIP (with a warn), not a
                // hard error: one corrupt/legacy row must not fail the whole
                // `load_all` and so block the daemon from starting at all (that
                // bricks EVERY session — a blank UI for the user). Same
                // "tolerate one bad row" philosophy as queue/drafts
                // below. The skipped seq leaves a gap, which the client tolerates.
                let event: Event = match serde_json::from_value(er.payload) {
                    Ok(ev) => ev,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            session = %id,
                            seq = seq_for_log,
                            "skipping undecodable event during restore",
                        );
                        continue;
                    }
                };
                let seq = u64::try_from(er.seq).unwrap_or(0);
                events.push(Envelope {
                    session_id: id.clone(),
                    seq,
                    event,
                    // cmid is a live-only reconcile tag, never persisted.
                    cmid: None,
                });
            }
            if crate::core::bound_restored_hot_log(&mut events) {
                reached_start = false;
            }
            let next_seq = u64::try_from(row.next_seq).unwrap_or(0);
            // Tolerate a malformed/legacy payload by degrading to empty rather
            // than failing the whole restore for one bad row.
            let queue: Vec<QueuedMessage> =
                serde_json::from_value(row.queue.clone()).unwrap_or_default();
            let drafts: Vec<QueuedMessage> =
                serde_json::from_value(row.drafts.clone()).unwrap_or_default();
            let config_options = row.config_options.clone();
            let config_preferences = if row.config_preferences.is_object() {
                row.config_preferences.clone()
            } else {
                serde_json::json!({})
            };
            let mobile_review_state = row.mobile_review_state.clone();
            out.push(LoadedSession {
                meta: row.into_meta(),
                events,
                event_count,
                reached_start,
                next_seq,
                queue,
                drafts,
                config_options,
                config_preferences,
                mobile_review_state,
            });
        }
        Ok(out)
    }

    /// Read one cursor-addressed history page directly from Postgres.
    pub async fn history_page(
        &self,
        session_id: &str,
        before_seq: u64,
        page_size: usize,
    ) -> Result<(Vec<Envelope>, Option<u64>, bool)> {
        let before_i64 = i64::try_from(before_seq).context("history cursor overflow")?;
        let limit = i64::try_from(page_size).context("history page size overflow")?;
        let rows: Vec<EventRow> = sqlx::query_as::<_, EventRow>(
            "SELECT seq, payload, 0::bigint AS total_count FROM events \
             WHERE session_id = $1 AND seq < $2 ORDER BY seq DESC LIMIT $3",
        )
        .bind(session_id)
        .bind(before_i64)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT history page for {session_id}"))?;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows.into_iter().rev() {
            match serde_json::from_value::<Event>(row.payload) {
                Ok(event) => events.push(Envelope {
                    session_id: session_id.to_owned(),
                    seq: u64::try_from(row.seq).unwrap_or(0),
                    event,
                    cmid: None,
                }),
                Err(e) => tracing::warn!(
                    error = %e,
                    session = %session_id,
                    seq = row.seq,
                    "skipping undecodable history event",
                ),
            }
        }
        let events = bound_history_page(events);
        let oldest = events.first().map(|event| event.seq);
        let reached_start = match oldest {
            Some(seq) => !sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM events WHERE session_id = $1 AND seq < $2)",
            )
            .bind(session_id)
            .bind(i64::try_from(seq).unwrap_or(i64::MAX))
            .fetch_one(&self.pool)
            .await
            .with_context(|| format!("SELECT history start for {session_id}"))?,
            None => true,
        };
        let next_before_seq = (!reached_start).then_some(oldest.unwrap_or(before_seq));
        Ok((events, next_before_seq, reached_start))
    }

    /// Load the complete question page immediately preceding `before_seq`.
    /// Unlike scrollback paging this deliberately follows the durable user
    /// prompt boundary, so the reader does not repeatedly re-render partial
    /// answer batches while looking for the preceding question.
    pub async fn question_page_before(
        &self,
        session_id: &str,
        before_seq: u64,
    ) -> Result<(Vec<Envelope>, Option<u64>, bool)> {
        let before_i64 = i64::try_from(before_seq).context("question cursor overflow")?;
        let root = sqlx::query_scalar::<_, Option<i64>>(
            "WITH ordered AS ( \
               SELECT seq, payload, \
                 LAG(payload->'update'->>'sessionUpdate') OVER (ORDER BY seq) AS previous_update \
               FROM events WHERE session_id = $1 AND seq < $2 \
             ) \
             SELECT MAX(seq) FROM ordered \
             WHERE payload->>'kind' = 'update' \
               AND payload->'update'->>'sessionUpdate' = 'user_message_chunk' \
               AND COALESCE(payload->'update'->>'autoResumed', 'false') <> 'true' \
               AND COALESCE(payload->'update'->'promptOrigin'->>'actor', 'human') = 'human' \
               AND POSITION('<system-reminder' IN LOWER(COALESCE(payload->'update'->'content'->>'text', ''))) = 0 \
               AND BTRIM(COALESCE(payload->'update'->'content'->>'text', '')) \
                   NOT IN ('/compact', '/compress') \
               AND previous_update IS DISTINCT FROM 'user_message_chunk'",
        )
        .bind(session_id)
        .bind(before_i64)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("SELECT previous question root for {session_id}"))?;
        let Some(root) = root else {
            return Ok((Vec::new(), None, true));
        };
        let turn_end = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT MIN(seq) FROM events \
             WHERE session_id = $1 AND seq >= $2 AND seq < $3 \
               AND payload->>'kind' = 'turn_end'",
        )
        .bind(session_id)
        .bind(root)
        .bind(before_i64)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("SELECT question turn end for {session_id}"))?;
        let page_end = turn_end
            .and_then(|seq| seq.checked_add(1))
            .unwrap_or(before_i64);
        let rows: Vec<EventRow> = sqlx::query_as::<_, EventRow>(
            "SELECT seq, payload, 0::bigint AS total_count FROM events \
             WHERE session_id = $1 AND seq >= $2 AND seq < $3 ORDER BY seq",
        )
        .bind(session_id)
        .bind(root)
        .bind(page_end)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT complete question page for {session_id}"))?;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            match serde_json::from_value::<Event>(row.payload) {
                Ok(event) => events.push(Envelope {
                    session_id: session_id.to_owned(),
                    seq: u64::try_from(row.seq).unwrap_or(0),
                    event,
                    cmid: None,
                }),
                Err(error) => tracing::warn!(
                    %error,
                    session = %session_id,
                    seq = row.seq,
                    "skipping undecodable question-page event",
                ),
            }
        }
        let root_u64 = u64::try_from(root).unwrap_or(0);
        let has_earlier_root = sqlx::query_scalar::<_, bool>(
            "WITH ordered AS ( \
               SELECT seq, payload, \
                 LAG(payload->'update'->>'sessionUpdate') OVER (ORDER BY seq) AS previous_update \
               FROM events WHERE session_id = $1 AND seq < $2 \
             ) \
             SELECT EXISTS(SELECT 1 FROM ordered \
               WHERE payload->>'kind' = 'update' \
                 AND payload->'update'->>'sessionUpdate' = 'user_message_chunk' \
                 AND COALESCE(payload->'update'->>'autoResumed', 'false') <> 'true' \
                 AND COALESCE(payload->'update'->'promptOrigin'->>'actor', 'human') = 'human' \
                 AND POSITION('<system-reminder' IN LOWER(COALESCE(payload->'update'->'content'->>'text', ''))) = 0 \
                 AND BTRIM(COALESCE(payload->'update'->'content'->>'text', '')) \
                     NOT IN ('/compact', '/compress') \
                 AND previous_update IS DISTINCT FROM 'user_message_chunk')",
        )
        .bind(session_id)
        .bind(root)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("SELECT earlier question root for {session_id}"))?;
        Ok((
            events,
            has_earlier_root.then_some(root_u64),
            !has_earlier_root,
        ))
    }

    /// Return a cursor page of lightweight question roots. This query transfers
    /// only the prompt text needed for the directory; answer payloads remain in
    /// Postgres until the reader opens one page.
    pub async fn question_page_summaries(
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
               SELECT seq, payload, \
                 LAG(payload->'update'->>'sessionUpdate') OVER (ORDER BY seq) AS previous_update \
               FROM events WHERE session_id = $1 \
             ), roots AS ( \
               SELECT seq, COALESCE(payload->'update'->'content'->>'text', '') AS title \
               FROM ordered \
               WHERE payload->>'kind' = 'update' \
                 AND payload->'update'->>'sessionUpdate' = 'user_message_chunk' \
                 AND COALESCE(payload->'update'->>'autoResumed', 'false') <> 'true' \
                 AND COALESCE(payload->'update'->'promptOrigin'->>'actor', 'human') = 'human' \
                 AND POSITION('<system-reminder' IN LOWER(COALESCE(payload->'update'->'content'->>'text', ''))) = 0 \
                 AND BTRIM(COALESCE(payload->'update'->'content'->>'text', '')) \
                     NOT IN ('/compact', '/compress') \
                 AND previous_update IS DISTINCT FROM 'user_message_chunk' \
             ), numbered AS ( \
               SELECT seq, title, \
                 ROW_NUMBER() OVER (ORDER BY seq) AS ordinal, \
                 COUNT(*) OVER () AS total \
               FROM roots \
             ) \
             SELECT seq, title, ordinal, total FROM numbered \
             WHERE ($2::bigint IS NULL OR seq < $2) \
             ORDER BY seq DESC LIMIT $3",
        )
        .bind(session_id)
        .bind(before)
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT question summaries for {session_id}"))?;
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

    /// Load exactly one immutable question page by its user-message root.
    pub async fn question_page_at(
        &self,
        session_id: &str,
        root_seq: u64,
    ) -> Result<Option<Vec<Envelope>>> {
        let root = i64::try_from(root_seq).context("question root overflow")?;
        let bounds = sqlx::query_as::<_, (i64, Option<i64>)>(
            "WITH ordered AS ( \
               SELECT seq, payload, \
                 LAG(payload->'update'->>'sessionUpdate') OVER (ORDER BY seq) AS previous_update \
               FROM events WHERE session_id = $1 \
             ), roots AS ( \
               SELECT seq FROM ordered \
               WHERE payload->>'kind' = 'update' \
                 AND payload->'update'->>'sessionUpdate' = 'user_message_chunk' \
                 AND COALESCE(payload->'update'->>'autoResumed', 'false') <> 'true' \
                 AND COALESCE(payload->'update'->'promptOrigin'->>'actor', 'human') = 'human' \
                 AND POSITION('<system-reminder' IN LOWER(COALESCE(payload->'update'->'content'->>'text', ''))) = 0 \
                 AND BTRIM(COALESCE(payload->'update'->'content'->>'text', '')) \
                     NOT IN ('/compact', '/compress') \
                 AND previous_update IS DISTINCT FROM 'user_message_chunk' \
             ), bounded AS ( \
               SELECT seq, LEAD(seq) OVER (ORDER BY seq) AS next_seq FROM roots \
             ) \
             SELECT seq, next_seq FROM bounded WHERE seq = $2",
        )
        .bind(session_id)
        .bind(root)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("SELECT question page bounds for {session_id}:{root_seq}"))?;
        let Some((start, end)) = bounds else {
            return Ok(None);
        };
        let turn_end = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT MIN(seq) FROM events \
             WHERE session_id = $1 AND seq >= $2 \
               AND ($3::bigint IS NULL OR seq < $3) \
               AND payload->>'kind' = 'turn_end'",
        )
        .bind(session_id)
        .bind(start)
        .bind(end)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("SELECT question page turn end for {session_id}:{root_seq}"))?;
        let end = turn_end.and_then(|seq| seq.checked_add(1)).or(end);
        let rows = sqlx::query_as::<_, EventRow>(
            "SELECT seq, payload, 0::bigint AS total_count FROM events \
             WHERE session_id = $1 AND seq >= $2 AND ($3::bigint IS NULL OR seq < $3) \
             ORDER BY seq",
        )
        .bind(session_id)
        .bind(start)
        .bind(end)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT question page at {session_id}:{root_seq}"))?;
        let events = rows
            .into_iter()
            .filter_map(|row| {
                serde_json::from_value::<Event>(row.payload)
                    .map(|event| Envelope {
                        session_id: session_id.to_owned(),
                        seq: u64::try_from(row.seq).unwrap_or(0),
                        event,
                        cmid: None,
                    })
                    .map_err(|error| {
                        tracing::warn!(
                            %error,
                            session = %session_id,
                            seq = row.seq,
                            "skipping undecodable lazy question-page event",
                        );
                    })
                    .ok()
            })
            .collect();
        Ok(Some(events))
    }

    /// Insert a brand-new session. Caller is expected to set `next_seq = 0`
    /// (the row default does it too).
    ///
    /// # Errors
    /// If the row already exists or the INSERT fails.
    pub async fn insert_session(&self, m: &SessionMeta) -> Result<()> {
        sqlx::query(
            "INSERT INTO sessions(id, provider, provider_version, provider_generation_digest, \
             provider_auth_generation, provider_behavior, machine_id, workspace_id, workspace_name, \
             workspace_source_path, cwd, title, origin, status, next_seq, system) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, 0, $15)",
        )
        .bind(&m.id)
        .bind(&m.provider)
        .bind(&m.provider_version)
        .bind(&m.provider_generation_digest)
        .bind(m.provider_auth_generation.and_then(|value| i64::try_from(value).ok()))
        .bind(
            m.provider_behavior
                .as_ref()
                .map(serde_json::to_value)
                .transpose()?,
        )
        .bind(&m.machine_id)
        .bind(m.workspace_id.as_deref())
        .bind(m.workspace_name.as_deref())
        .bind(m.workspace_source_path.as_deref())
        .bind(&m.cwd)
        .bind(strip_nul_str(&m.title))
        .bind(origin_to_str(m.origin))
        .bind(status_to_str(m.status))
        .bind(m.system)
        .execute(&self.pool)
        .await
        .with_context(|| format!("INSERT session {}", m.id))?;
        Ok(())
    }

    /// Update only `status` and bump `updated_at`. Used when `Hub::set_status`
    /// fires; the event itself goes through `append_event` separately.
    ///
    /// # Errors
    /// If the UPDATE fails.
    pub async fn update_status(&self, session_id: &str, status: Status) -> Result<()> {
        sqlx::query("UPDATE sessions SET status = $1, updated_at = now() WHERE id = $2")
            .bind(status_to_str(status))
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE session status {session_id}"))?;
        Ok(())
    }

    /// Persist the downstream agent's own session id (the ACP id it returns
    /// from `session/new`). Stored so a revived agent can resume the prior
    /// conversation via `session/load` instead of starting blank. Mirrors the
    /// other `update_*` helpers — only this column and `updated_at` move.
    ///
    /// # Errors
    /// If the UPDATE fails.
    pub async fn update_agent_session_id(
        &self,
        session_id: &str,
        agent_session_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE sessions SET agent_session_id = $1, updated_at = now() WHERE id = $2")
            .bind(agent_session_id)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE session agent_session_id {session_id}"))?;
        Ok(())
    }

    /// Persist the latest agent-advertised config options. This is a display
    /// snapshot, not the source of the user's selected values.
    pub async fn update_config_options(
        &self,
        session_id: &str,
        options: &serde_json::Value,
    ) -> Result<()> {
        let mut options = options.clone();
        strip_nul(&mut options);
        sqlx::query("UPDATE sessions SET config_options = $1, updated_at = now() WHERE id = $2")
            .bind(options)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE session config_options {session_id}"))?;
        Ok(())
    }

    /// Persist the values selected by the user for this session. Keeping this
    /// separate from the agent snapshot prevents startup defaults from erasing
    /// a choice before it can be re-applied.
    pub async fn update_config_preferences(
        &self,
        session_id: &str,
        preferences: &serde_json::Value,
    ) -> Result<()> {
        let mut preferences = preferences.clone();
        strip_nul(&mut preferences);
        sqlx::query(
            "UPDATE sessions SET config_preferences = $1, updated_at = now() WHERE id = $2",
        )
        .bind(preferences)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPDATE session config_preferences {session_id}"))?;
        Ok(())
    }

    /// Persist a user-renamed title. Mirrors `update_status` — only the
    /// title and `updated_at` move; everything else stays.
    ///
    /// # Errors
    /// If the UPDATE fails.
    pub async fn update_title(&self, session_id: &str, title: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET title = $1, updated_at = now() WHERE id = $2")
            .bind(strip_nul_str(title))
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE session title {session_id}"))?;
        Ok(())
    }

    /// Persist a migrated session cwd and, when supplied, its untouched default
    /// title in one statement so the list cannot observe a half-retargeted row.
    pub async fn update_cwd(&self, session_id: &str, cwd: &str, title: Option<&str>) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET cwd = $1, title = COALESCE($2, title), updated_at = now() WHERE id = $3",
        )
        .bind(cwd)
        .bind(title.map(strip_nul_str))
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPDATE session cwd {session_id}"))?;
        Ok(())
    }

    /// Persist the Mobile-only code-review workspace state for one session.
    ///
    /// # Errors
    /// If the UPDATE fails.
    pub async fn update_mobile_review_state(
        &self,
        session_id: &str,
        value: &serde_json::Value,
    ) -> Result<()> {
        let mut value = value.clone();
        strip_nul(&mut value);
        sqlx::query(
            "UPDATE sessions SET mobile_review_state = $1, updated_at = now() WHERE id = $2",
        )
        .bind(&value)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPDATE mobile review state {session_id}"))?;
        Ok(())
    }

    /// Insert or replace a reduced batch of canonical events and advance every
    /// touched session's sequence watermark in one transaction. Replacing an
    /// existing `(session_id, seq)` is how the writer coalesces streaming text
    /// and tool updates without changing their stable timeline position.
    pub async fn upsert_event_batch(
        &self,
        events: &[Envelope],
        highwaters: &HashMap<String, u64>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.context("begin event batch")?;
        for env in events {
            let mut payload = serde_json::to_value(&env.event).context("serialize event")?;
            strip_nul(&mut payload);
            self.artifacts.externalize_images(&mut payload)?;
            let seq = i64::try_from(env.seq).context("seq i64 overflow")?;
            sqlx::query(
                "INSERT INTO events(session_id, seq, payload) VALUES ($1, $2, $3) \
                 ON CONFLICT(session_id, seq) DO UPDATE SET payload = EXCLUDED.payload",
            )
            .bind(&env.session_id)
            .bind(seq)
            .bind(&payload)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("UPSERT event {}/{}", env.session_id, env.seq))?;
        }
        for (session_id, next_seq) in highwaters {
            let next_seq = i64::try_from(*next_seq).context("next_seq i64 overflow")?;
            sqlx::query(
                "UPDATE sessions SET next_seq = GREATEST(next_seq, $1), updated_at = now() \
                 WHERE id = $2",
            )
            .bind(next_seq)
            .bind(session_id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("UPDATE next_seq for {session_id}"))?;
        }
        tx.commit().await.context("commit event batch")?;
        Ok(())
    }

    /// Delete one session's durable transcript while preserving its metadata
    /// and monotonic `next_seq` watermark.
    pub async fn clear_events(&self, session_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM events WHERE session_id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("DELETE events for {session_id}"))?;
        Ok(())
    }

    pub fn artifact_path(&self, name: &str) -> Option<std::path::PathBuf> {
        self.artifacts.path(name)
    }

    /// Persist a session's queue + drafts (whole lists, as JSONB). Called on
    /// every staged-message mutation so the cross-terminal queue/drafts survive
    /// a daemon restart. Whole-list overwrite (not row-level) keeps it simple;
    /// the lists are small (a handful of pending prompts at most).
    ///
    /// # Errors
    /// If serializing the lists fails or the UPDATE fails.
    pub async fn update_pending(
        &self,
        session_id: &str,
        queue: &[QueuedMessage],
        drafts: &[QueuedMessage],
    ) -> Result<()> {
        let mut queue_json = serde_json::to_value(queue).context("serialize queue")?;
        let mut drafts_json = serde_json::to_value(drafts).context("serialize drafts")?;
        strip_nul(&mut queue_json);
        strip_nul(&mut drafts_json);
        sqlx::query(
            "UPDATE sessions SET queue = $1, drafts = $2, updated_at = now() WHERE id = $3",
        )
        .bind(&queue_json)
        .bind(&drafts_json)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPDATE session pending {session_id}"))?;
        Ok(())
    }

    /// Upsert a session's pending `ScheduleWakeup` (migration 0011).
    ///
    /// # Errors
    /// If the query fails.
    pub async fn upsert_wakeup(
        &self,
        session_id: &str,
        fire_at_ms: i64,
        prompt: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO scheduled_wakeups (session_id, fire_at_ms, prompt) VALUES ($1, $2, $3) \
             ON CONFLICT (session_id) DO UPDATE SET fire_at_ms = $2, prompt = $3",
        )
        .bind(session_id)
        .bind(fire_at_ms)
        .bind(prompt)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPSERT wakeup {session_id}"))?;
        Ok(())
    }

    /// Drop a session's persisted wakeup (it fired, or was dropped).
    ///
    /// # Errors
    /// If the query fails.
    pub async fn delete_wakeup(&self, session_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM scheduled_wakeups WHERE session_id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("DELETE wakeup {session_id}"))?;
        Ok(())
    }

    /// Load every persisted pending wakeup as `(session_id, fire_at_ms, prompt)`,
    /// to re-arm the scheduler on startup. Overdue ones fire immediately (catch-up).
    ///
    /// # Errors
    /// If the query fails.
    pub async fn load_wakeups(&self) -> Result<Vec<(String, i64, String)>> {
        let rows: Vec<(String, i64, String)> =
            sqlx::query_as("SELECT session_id, fire_at_ms, prompt FROM scheduled_wakeups")
                .fetch_all(&self.pool)
                .await
                .context("SELECT scheduled_wakeups")?;
        Ok(rows)
    }

    /// Persist the manual session ordering: write each id's index as its
    /// `position`. `load_all` then restores the drag-arranged order (NULLS LAST
    /// + `created_at` keeps any unknown/never-reordered rows sensible). It writes
    ///   one update per id in a single transaction; the list is short.
    ///
    /// # Errors
    /// If the transaction or an UPDATE fails.
    pub async fn update_session_order(&self, order: &[String]) -> Result<()> {
        let mut tx = self.pool.begin().await.context("begin tx")?;
        for (i, id) in order.iter().enumerate() {
            let pos = i64::try_from(i).unwrap_or(i64::MAX);
            sqlx::query("UPDATE sessions SET position = $1, updated_at = now() WHERE id = $2")
                .bind(pos)
                .bind(id)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("UPDATE position for {id}"))?;
        }
        tx.commit().await.context("commit update_session_order")?;
        Ok(())
    }

    /// SOFT-delete a session: mark `deleted_at` so it vanishes from the UI (the
    /// in-memory Hub already dropped it, and `load_all` skips it) but its rows
    /// linger for the retention window before [`Self::purge_deleted`] hard-drops
    /// them (cascade → events). Idempotent; re-deleting keeps the first time.
    ///
    /// # Errors
    /// If the UPDATE fails.
    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET deleted_at = now() WHERE id = $1 AND deleted_at IS NULL")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("soft-delete session {session_id}"))?;
        Ok(())
    }

    pub async fn soft_delete_sessions_until(
        &self,
        session_ids: &[String],
        purge_after_ms: i64,
    ) -> Result<()> {
        let purge_after = chrono::DateTime::<Utc>::from_timestamp_millis(purge_after_ms)
            .context("Provider uninstall purge deadline is outside the supported range")?;
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("begin Provider uninstall")?;
        for session_id in session_ids {
            let result = sqlx::query(
                "UPDATE sessions SET deleted_at = now(), purge_after_at = $2 \
                 WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(session_id)
            .bind(purge_after)
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

    /// Hard-delete sessions soft-deleted more than `retention_days` ago (cascade
    /// → their events). The storage-reclaim half of soft-delete; run on startup
    /// and periodically. Returns the number of sessions purged.
    ///
    /// # Errors
    /// If the DELETE fails.
    pub async fn purge_deleted(&self, retention_days: i64) -> Result<u64> {
        let done = sqlx::query(
            "DELETE FROM sessions WHERE deleted_at IS NOT NULL \
             AND COALESCE( \
               purge_after_at, \
               deleted_at + make_interval(days => $1::int) \
             ) < now()",
        )
        .bind(i32::try_from(retention_days).unwrap_or(3))
        .execute(&self.pool)
        .await
        .context("purge soft-deleted sessions")?;
        let mut referenced = HashSet::new();
        // Extract only the small artifact URLs in Postgres. Pulling every
        // matching JSON payload across the driver made startup GC deserialize
        // hundreds of large transcript events and dominated controller peak
        // memory even though rows were consumed as a stream.
        let mut rows = sqlx::query(
            "SELECT DISTINCT value #>> ARRAY[]::text[] AS reference \
             FROM events \
             CROSS JOIN LATERAL jsonb_path_query( \
                 payload, \
                 'strict $.** ? (@.type() == \"string\" && @ starts with \"/api/artifacts/\")'::jsonpath \
             ) AS value",
        )
        .fetch(&self.pool);
        while let Some(row) = rows
            .try_next()
            .await
            .context("scan retained PostgreSQL artifact references")?
        {
            let reference: String = row.try_get("reference")?;
            crate::artifacts::collect_references(
                &serde_json::Value::String(reference),
                &mut referenced,
            );
        }
        let artifacts_removed = self
            .artifacts
            .prune_unreferenced(&referenced, Duration::from_hours(24))
            .context("prune unreferenced PostgreSQL event artifacts")?;
        if artifacts_removed > 0 {
            tracing::info!(artifacts_removed, "purged unreferenced event artifacts");
        }
        Ok(done.rows_affected())
    }

    /// Insert an incident idempotently. Raw evidence is retained by Victoria;
    /// this row is the durable index and recovery record.
    pub async fn upsert_runtime_incident(&self, incident: &RuntimeIncidentWrite) -> Result<()> {
        let mut detail = incident.detail.clone();
        strip_nul(&mut detail);
        sqlx::query(
            "INSERT INTO runtime_incidents (id, occurred_at, source, classification, severity, \
             state, summary, fingerprint, session_id, client_id, machine_id, trace_id, build, \
             evidence_start, evidence_end, detail) VALUES ( \
             $1, to_timestamp($2::double precision / 1000), $3, $4, $5, $6, $7, $8, $9, $10, \
             COALESCE($11, (SELECT machine_id FROM sessions WHERE id = $9)), \
             $12, $13, to_timestamp($14::double precision / 1000), \
             to_timestamp($15::double precision / 1000), $16) \
             ON CONFLICT (id) DO UPDATE SET updated_at = now(), \
             evidence_end = GREATEST(runtime_incidents.evidence_end, EXCLUDED.evidence_end), \
             detail = runtime_incidents.detail || EXCLUDED.detail",
        )
        .bind(strip_nul_str(&incident.id).as_ref())
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
        .execute(&self.pool)
        .await
        .context("UPSERT runtime incident")?;
        Ok(())
    }

    /// Mark every active controller runtime incident for a session as recovered.
    /// Terminal failed-turn and command-error records remain immutable evidence.
    pub async fn recover_runtime_incident(
        &self,
        session_id: &str,
        recovered_at_ms: i64,
        outcome: &str,
    ) -> Result<u64> {
        let done = sqlx::query(
            "UPDATE runtime_incidents SET state = 'recovered', \
             recovered_at = to_timestamp($2::double precision / 1000), \
             recovery_outcome = $3, updated_at = now() WHERE session_id = $1 \
               AND source = 'controller' AND state = 'active'",
        )
        .bind(session_id)
        .bind(recovered_at_ms)
        .bind(outcome)
        .execute(&self.pool)
        .await
        .with_context(|| format!("recover runtime incident for {session_id}"))?;
        Ok(done.rows_affected())
    }

    /// Query recent incident summaries, newest first.
    pub async fn runtime_incidents(&self, limit: i64) -> Result<Vec<RuntimeIncident>> {
        sqlx::query_as(
            "SELECT id, (extract(epoch FROM occurred_at) * 1000)::bigint AS occurred_at_ms, \
             (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at_ms, source, \
             classification, severity, state, summary, fingerprint, session_id, client_id, \
             machine_id, trace_id, build, \
             (extract(epoch FROM evidence_start) * 1000)::bigint AS evidence_start_ms, \
             (extract(epoch FROM evidence_end) * 1000)::bigint AS evidence_end_ms, detail, \
             (extract(epoch FROM recovered_at) * 1000)::bigint AS recovered_at_ms, \
             recovery_outcome FROM runtime_incidents ORDER BY occurred_at DESC LIMIT $1",
        )
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await
        .context("SELECT runtime incidents")
    }

    /// Query the unified diagnostic-log index without loading heavy incident
    /// detail. `PostgreSQL` remains the durable correlation layer; raw service
    /// evidence stays in `VictoriaLogs`.
    #[allow(clippy::too_many_lines)] // one SQL read model spanning four existing owners
    pub async fn diagnostic_logs(
        &self,
        filter: &DiagnosticLogFilter,
    ) -> Result<Vec<DiagnosticLogSummary>> {
        let query = r"
            WITH runtime_logs AS (
              SELECT 'runtime:' || incident.id AS id,
                (extract(epoch FROM incident.occurred_at) * 1000)::bigint AS occurred_at_ms,
                'session_error'::text AS kind, incident.severity, incident.state,
                initcap(replace(incident.classification, '_', ' ')) AS title,
                incident.summary, incident.session_id AS session_ref,
                session.provider,
                CASE
                  WHEN session.provider IN ('codex', 'codex-deepseek') THEN 'codex'
                  WHEN session.provider IN ('claude-code', 'claude-deepseek') THEN 'claude'
                  ELSE NULL
                END AS agent,
                NULL::text AS model, incident.classification
              FROM runtime_incidents incident
              LEFT JOIN sessions session ON session.id = incident.session_id
              WHERE incident.occurred_at >= to_timestamp($1::double precision / 1000)
                AND incident.occurred_at <= to_timestamp($2::double precision / 1000)
            ), provider_error_logs AS (
              SELECT 'provider:' || event.machine_id || ':' || event.producer_id || ':' ||
                  event.sequence::text AS id,
                (extract(epoch FROM event.occurred_at) * 1000)::bigint AS occurred_at_ms,
                'provider_error'::text AS kind,
                CASE
                  WHEN event.status IN (401, 402, 403) THEN 'critical'
                  WHEN event.status BETWEEN 400 AND 499
                    AND event.status NOT IN (408, 425, 429, 499) THEN 'error'
                  ELSE 'warning'
                END::text AS severity,
                'failed'::text AS state,
                'DeepSeek HTTP ' || event.status::text AS title,
                initcap(event.agent) || ' request ' || CASE
                  WHEN event.status BETWEEN 400 AND 499
                    AND event.status NOT IN (408, 425, 429, 499)
                    THEN 'was blocked with HTTP status '
                  ELSE 'attempt received retryable HTTP status '
                END || event.status::text AS summary,
                event.session_fingerprint AS session_ref, event.provider,
                event.agent, nullif(coalesce(event.resolved_model, event.model), '') AS model,
                CASE
                  WHEN event.status IN (401, 403) THEN 'provider_authentication_failure'
                  WHEN event.status = 402 THEN 'provider_balance_exhausted'
                  WHEN event.status BETWEEN 400 AND 499
                    AND event.status NOT IN (408, 425, 429, 499)
                    THEN 'provider_request_blocked'
                  ELSE 'provider_retryable_failure'
                END::text AS classification
              FROM provider_usage_events event
              WHERE event.occurred_at >= to_timestamp($1::double precision / 1000)
                AND event.occurred_at <= to_timestamp($2::double precision / 1000)
                AND event.request_purpose = 'interactive'
                AND event.status >= 400
            ), cache_keepalive_logs AS (
              SELECT 'keepalive:' || event.machine_id || ':' || event.producer_id || ':' ||
                  event.sequence::text AS id,
                (extract(epoch FROM event.occurred_at) * 1000)::bigint AS occurred_at_ms,
                'cache_anomaly'::text AS kind,
                CASE
                  WHEN event.cache_keepalive_outcome = 'terminal_error'
                    AND event.status IN (401, 402, 403) THEN 'critical'
                  WHEN event.cache_keepalive_outcome IN
                    ('miss', 'partial', 'retryable_error', 'terminal_error') THEN 'warning'
                  ELSE 'info'
                END::text AS severity,
                CASE event.cache_keepalive_outcome
                  WHEN 'hit' THEN 'succeeded'
                  WHEN 'preempted' THEN 'recovered'
                  WHEN 'retryable_error' THEN 'retrying'
                  ELSE 'failed'
                END::text AS state,
                CASE event.cache_keepalive_outcome
                  WHEN 'hit' THEN 'DeepSeek cache protected'
                  WHEN 'miss' THEN 'DeepSeek keepalive cache miss'
                  WHEN 'partial' THEN 'DeepSeek keepalive partial hit'
                  WHEN 'retryable_error' THEN 'DeepSeek keepalive will retry'
                  WHEN 'terminal_error' THEN 'DeepSeek keepalive stopped'
                  ELSE 'DeepSeek keepalive preempted'
                END::text AS title,
                CASE event.cache_keepalive_outcome
                  WHEN 'hit' THEN format('Protected %s cached tokens',
                    coalesce(event.cache_hit_tokens, 0))
                  WHEN 'miss' THEN format('Cache expired after %s ms; protection stopped',
                    coalesce(event.cache_keepalive_source_age_ms, 0))
                  WHEN 'partial' THEN format('Cache hit fell below 90%% after %s ms; protection stopped',
                    coalesce(event.cache_keepalive_source_age_ms, 0))
                  WHEN 'retryable_error' THEN format('Retryable HTTP %s; one bounded retry is allowed',
                    event.status)
                  WHEN 'terminal_error' THEN format('HTTP %s made this snapshot ineligible for further keepalives',
                    event.status)
                  ELSE 'A real agent request preempted the background keepalive'
                END::text AS summary,
                event.session_fingerprint AS session_ref, event.provider,
                event.agent, nullif(coalesce(event.resolved_model, event.model), '') AS model,
                'cache_keepalive_' || event.cache_keepalive_outcome AS classification
              FROM provider_usage_events event
              WHERE event.occurred_at >= to_timestamp($1::double precision / 1000)
                AND event.occurred_at <= to_timestamp($2::double precision / 1000)
                AND event.request_purpose = 'cache_keepalive'
            ), lineage AS (
              SELECT event.*,
                lag(event.status) OVER session_window AS previous_status,
                lag(event.model_family) OVER session_window AS previous_model_family,
                lag(event.model_revision) OVER session_window AS previous_model_revision,
                lag(event.request_role) OVER session_window AS previous_request_role,
                lag(event.upstream_protocol) OVER session_window AS previous_upstream_protocol,
                lag(event.translation_mode) OVER session_window AS previous_translation_mode,
                lag(event.thinking_mode) OVER session_window AS previous_thinking_mode,
                lag(event.reasoning_effort) OVER session_window AS previous_reasoning_effort,
                lag(event.static_prefix_fingerprint) OVER session_window AS previous_static_prefix,
                lag(event.request_prefix_fingerprint) OVER session_window AS previous_request_prefix,
                lag(event.input_tokens) OVER session_window AS previous_input_tokens,
                lag(event.input_item_count) OVER session_window AS previous_input_item_count,
                lag(event.gateway_build) OVER session_window AS previous_gateway_build,
                lag(event.gateway_boot_id) OVER session_window AS previous_gateway_boot_id,
                lag(event.cache_hit_tokens) OVER session_window AS previous_cache_hit_tokens,
                lag(event.cache_miss_tokens) OVER session_window AS previous_cache_miss_tokens,
                lag(event.sequence) OVER session_window AS previous_sequence,
                lag(event.occurred_at) OVER session_window AS previous_occurred_at
              FROM provider_usage_events event
              WHERE event.occurred_at >=
                  to_timestamp($1::double precision / 1000) - interval '30 minutes'
                AND event.occurred_at <= to_timestamp($2::double precision / 1000)
                AND event.schema_version >= 3
                AND event.request_purpose = 'interactive'
                AND event.session_fingerprint IS NOT NULL
                AND event.session_attribution <> 'prefix_root'
                AND event.status < 400
                AND event.cache_observation IN ('explicit', 'derived')
                AND coalesce(event.input_tokens, 0) >= 8000
                AND event.cache_hit_tokens + event.cache_miss_tokens > 0
                AND event.static_prefix_fingerprint IS NOT NULL
              WINDOW session_window AS (
                PARTITION BY event.machine_id, event.producer_id, event.account_fingerprint,
                  event.agent, event.session_fingerprint
                ORDER BY event.occurred_at, event.sequence
              )
            ), cache_transitions AS (
              SELECT lineage.*,
                CASE
                  WHEN operation = 'compact' THEN 'client_compaction'
                  WHEN gateway_build IS DISTINCT FROM previous_gateway_build
                    THEN 'gateway_build_changed'
                  WHEN gateway_boot_id IS DISTINCT FROM previous_gateway_boot_id
                    THEN 'post_gateway_restart'
                  WHEN model_family IS DISTINCT FROM previous_model_family THEN 'model_changed'
                  WHEN model_revision IS DISTINCT FROM previous_model_revision
                    THEN 'model_revision_changed'
                  WHEN request_role IS DISTINCT FROM previous_request_role
                    THEN 'request_role_changed'
                  WHEN upstream_protocol IS DISTINCT FROM previous_upstream_protocol
                    THEN 'protocol_changed'
                  WHEN translation_mode IS DISTINCT FROM previous_translation_mode
                    THEN 'translation_changed'
                  WHEN thinking_mode IS DISTINCT FROM previous_thinking_mode
                    OR reasoning_effort IS DISTINCT FROM previous_reasoning_effort
                    THEN 'reasoning_configuration_changed'
                  WHEN compatibility_fixes > 0 THEN 'compatibility_rewrite'
                  WHEN static_prefix_fingerprint IS DISTINCT FROM previous_static_prefix
                    THEN 'static_prefix_changed'
                  WHEN (agent <> 'codex' OR has_previous_response_id IS NOT TRUE)
                    AND input_item_count < previous_input_item_count THEN 'history_rewrite'
                  WHEN EXISTS (
                    SELECT 1 FROM provider_usage_events failed
                    WHERE failed.machine_id = lineage.machine_id
                      AND failed.producer_id = lineage.producer_id
                      AND failed.account_fingerprint = lineage.account_fingerprint
                      AND failed.agent = lineage.agent
                      AND failed.session_fingerprint IS NOT DISTINCT FROM lineage.session_fingerprint
                      AND failed.request_purpose = 'interactive'
                      AND (failed.occurred_at, failed.sequence) >
                        (lineage.previous_occurred_at, lineage.previous_sequence)
                      AND (failed.occurred_at, failed.sequence) <
                        (lineage.occurred_at, lineage.sequence)
                      AND failed.status >= 400
                  ) THEN 'post_provider_error'
                  WHEN request_prefix_fingerprint = previous_request_prefix
                    THEN 'unexpected_exact_prefix_miss'
                  ELSE 'unexpected_active_cache_drop'
                END AS cause
              FROM lineage
              WHERE occurred_at >= to_timestamp($1::double precision / 1000)
                AND occurred_at <= to_timestamp($2::double precision / 1000)
                AND schema_version >= 3
                AND session_fingerprint IS NOT NULL
                AND session_attribution <> 'prefix_root'
                AND previous_occurred_at IS NOT NULL
                AND occurred_at >= previous_occurred_at
                AND occurred_at - previous_occurred_at <= interval '30 minutes'
                AND status < 400
                AND previous_status < 400
                AND cache_observation IN ('explicit', 'derived')
                AND coalesce(input_tokens, 0) >= 8000
                AND cache_hit_tokens + cache_miss_tokens > 0
                AND cache_hit_tokens * 10 < cache_hit_tokens + cache_miss_tokens
                AND coalesce(previous_input_tokens, 0) >= 8000
                AND previous_cache_hit_tokens + previous_cache_miss_tokens > 0
                AND previous_cache_hit_tokens * 10 >=
                  9 * (previous_cache_hit_tokens + previous_cache_miss_tokens)
                AND static_prefix_fingerprint IS NOT NULL
                AND previous_static_prefix IS NOT NULL
            ), cache_logs AS (
              SELECT 'cache:' || machine_id || ':' || producer_id || ':' || sequence::text AS id,
                (extract(epoch FROM occurred_at) * 1000)::bigint AS occurred_at_ms,
                'cache_anomaly'::text AS kind, 'warning'::text AS severity,
                'observed'::text AS state,
                CASE cause
                  WHEN 'static_prefix_changed' THEN 'Cache prefix changed'
                  WHEN 'unexpected_exact_prefix_miss' THEN 'Exact-prefix cache miss'
                  WHEN 'history_rewrite' THEN 'Cached history rewritten'
                  WHEN 'post_gateway_restart' THEN 'Cache lost after gateway restart'
                  WHEN 'gateway_build_changed' THEN 'Cache lost after gateway update'
                  WHEN 'client_compaction' THEN 'Cache changed after compaction'
                  WHEN 'model_changed' THEN 'Cache lost after model change'
                  WHEN 'model_revision_changed' THEN 'Cache lost after model revision'
                  WHEN 'request_role_changed' THEN 'Cache lost after role change'
                  WHEN 'protocol_changed' THEN 'Cache lost after protocol change'
                  WHEN 'translation_changed' THEN 'Cache lost after translation change'
                  WHEN 'reasoning_configuration_changed' THEN 'Cache lost after reasoning change'
                  WHEN 'compatibility_rewrite' THEN 'Cache lost after compatibility rewrite'
                  WHEN 'post_provider_error' THEN 'Cache lost after provider error'
                  ELSE 'Unexplained active-session cache drop'
                END AS title,
                format('Cache hit rate fell from %s%% to %s%% within 30 minutes',
                  round(previous_cache_hit_tokens * 100.0 /
                    (previous_cache_hit_tokens + previous_cache_miss_tokens), 1),
                  round(cache_hit_tokens * 100.0 / (cache_hit_tokens + cache_miss_tokens), 1)
                ) AS summary,
                session_fingerprint AS session_ref, provider, agent,
                nullif(coalesce(resolved_model, model), '') AS model, cause AS classification
              FROM cache_transitions
            ), automation_logs AS (
              SELECT 'automation:' || log.id::text AS id, log.created_at_ms AS occurred_at_ms,
                'automation'::text AS kind,
                CASE
                  WHEN log.status = 'failed' THEN 'error'
                  WHEN log.status IN ('retrying', 'unknown') THEN 'warning'
                  ELSE 'info'
                END AS severity,
                log.status AS state,
                initcap(log.provider) || ' ' || replace(log.action, '_', ' ') AS title,
                log.message AS summary, NULL::text AS session_ref, log.provider,
                CASE WHEN log.provider = 'codex' THEN 'codex'::text ELSE NULL::text END AS agent,
                NULL::text AS model,
                'provider_automation'::text AS classification
              FROM provider_action_logs log
              WHERE log.created_at_ms >= $1 AND log.created_at_ms <= $2
            ), logs AS (
              SELECT * FROM runtime_logs
              UNION ALL SELECT * FROM provider_error_logs
              UNION ALL SELECT * FROM cache_keepalive_logs
              UNION ALL SELECT * FROM cache_logs
              UNION ALL SELECT * FROM automation_logs
            )
            SELECT id, occurred_at_ms, kind, severity, state, title, summary,
              session_ref, provider, agent, model, classification
            FROM logs
            WHERE (cardinality($3::text[]) = 0 OR kind = ANY($3))
              AND (cardinality($4::text[]) = 0 OR severity = ANY($4))
              AND (cardinality($5::text[]) = 0 OR state = ANY($5))
              AND (cardinality($6::text[]) = 0 OR agent = ANY($6))
              AND ($7::text IS NULL OR session_ref = $7)
              AND ($8::bigint IS NULL OR occurred_at_ms < $8 OR
                (occurred_at_ms = $8 AND id < $9))
            ORDER BY occurred_at_ms DESC, id DESC
            LIMIT $10
        ";
        sqlx::query_as(query)
            .bind(filter.since_ms)
            .bind(filter.until_ms)
            .bind(&filter.kinds)
            .bind(&filter.severities)
            .bind(&filter.states)
            .bind(&filter.agents)
            .bind(filter.session_ref.as_deref())
            .bind(filter.cursor_ms)
            .bind(filter.cursor_id.as_deref())
            .bind(filter.limit.clamp(1, 100).saturating_add(1))
            .fetch_all(&self.pool)
            .await
            .context("query diagnostic logs")
    }

    /// Load one diagnostic event after the user expands it. List requests never
    /// pay for incident JSON or provider lineage evidence.
    #[allow(clippy::too_many_lines)] // type-specific lazy detail stays at this storage boundary
    pub async fn diagnostic_log_detail(&self, id: &str) -> Result<Option<DiagnosticLogDetail>> {
        if let Some(incident_id) = id.strip_prefix("runtime:") {
            let incident = sqlx::query_as::<_, RuntimeIncident>(
                "SELECT id, (extract(epoch FROM occurred_at) * 1000)::bigint AS occurred_at_ms, \
                 (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at_ms, source, \
                 classification, severity, state, summary, fingerprint, session_id, client_id, \
                 machine_id, trace_id, build, \
                 (extract(epoch FROM evidence_start) * 1000)::bigint AS evidence_start_ms, \
                 (extract(epoch FROM evidence_end) * 1000)::bigint AS evidence_end_ms, detail, \
                 (extract(epoch FROM recovered_at) * 1000)::bigint AS recovered_at_ms, \
                 recovery_outcome FROM runtime_incidents WHERE id = $1",
            )
            .bind(incident_id)
            .fetch_optional(&self.pool)
            .await
            .context("load runtime diagnostic detail")?;
            let Some(incident) = incident else {
                return Ok(None);
            };
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
            return Ok(Some(DiagnosticLogDetail {
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
            }));
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
                 idempotency_suffix, created_at_ms FROM provider_action_logs WHERE id = $1",
            )
            .bind(action_id)
            .fetch_optional(&self.pool)
            .await
            .context("load automation diagnostic detail")?;
            let Some((
                provider,
                action,
                trigger,
                status,
                phase,
                message,
                credit_id,
                key,
                occurred_at_ms,
            )) = row
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
                occurred_at_ms,
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
        let row = sqlx::query_as::<_, (i64, serde_json::Value)>(
            r"
                WITH target AS (
                  SELECT * FROM provider_usage_events
                  WHERE machine_id = $1 AND producer_id = $2 AND sequence = $3
                ), previous AS (
                  SELECT candidate.* FROM provider_usage_events candidate, target
                  WHERE candidate.machine_id = target.machine_id
                    AND candidate.producer_id = target.producer_id
                    AND candidate.account_fingerprint = target.account_fingerprint
                    AND candidate.agent = target.agent
                    AND candidate.session_fingerprint IS NOT DISTINCT FROM target.session_fingerprint
                    AND candidate.schema_version >= 3
                    AND candidate.request_purpose = 'interactive'
                    AND candidate.session_attribution <> 'prefix_root'
                    AND candidate.status < 400
                    AND candidate.cache_observation IN ('explicit', 'derived')
                    AND coalesce(candidate.input_tokens, 0) >= 8000
                    AND candidate.cache_hit_tokens + candidate.cache_miss_tokens > 0
                    AND candidate.static_prefix_fingerprint IS NOT NULL
                    AND (candidate.occurred_at, candidate.sequence) <
                      (target.occurred_at, target.sequence)
                  ORDER BY candidate.occurred_at DESC, candidate.sequence DESC
                  LIMIT 1
                )
                SELECT (extract(epoch FROM target.occurred_at) * 1000)::bigint,
                  jsonb_build_object(
                    'current', to_jsonb(target),
                    'previous', CASE WHEN previous.sequence IS NULL THEN '{}'::jsonb
                      ELSE to_jsonb(previous) END,
                    'gap_ms', CASE WHEN previous.sequence IS NULL THEN NULL
                      ELSE (extract(epoch FROM target.occurred_at - previous.occurred_at) * 1000)::bigint END,
                    'intervening_provider_errors', CASE WHEN previous.sequence IS NULL THEN 0
                      ELSE (SELECT count(*) FROM provider_usage_events failed
                        WHERE failed.machine_id = target.machine_id
                          AND failed.producer_id = target.producer_id
                          AND failed.account_fingerprint = target.account_fingerprint
                          AND failed.agent = target.agent
                          AND failed.session_fingerprint IS NOT DISTINCT FROM target.session_fingerprint
                          AND failed.request_purpose = 'interactive'
                          AND (failed.occurred_at, failed.sequence) >
                            (previous.occurred_at, previous.sequence)
                          AND (failed.occurred_at, failed.sequence) <
                            (target.occurred_at, target.sequence)
                          AND failed.status >= 400) END
                  )
                FROM target LEFT JOIN previous ON true
            ",
        )
        .bind(machine_id)
        .bind(producer_id)
        .bind(sequence)
        .fetch_optional(&self.pool)
        .await
        .context("load provider diagnostic detail")?;
        let Some((occurred_at_ms, evidence)) = row else {
            return Ok(None);
        };
        let current = evidence.get("current").cloned().unwrap_or_default();
        let previous = evidence.get("previous").cloned().unwrap_or_default();
        let gap_ms = json_i64(&evidence, "gap_ms");
        if kind == "cache_keepalive" {
            if json_scalar(&current, "request_purpose").as_deref() != Some("cache_keepalive") {
                return Ok(None);
            }
            let outcome = json_scalar(&current, "cache_keepalive_outcome")
                .unwrap_or_else(|| "unknown".to_owned());
            let status = json_scalar(&current, "status").unwrap_or_else(|| "unknown".to_owned());
            let title = match outcome.as_str() {
                "hit" => "DeepSeek cache protected",
                "miss" => "DeepSeek keepalive cache miss",
                "partial" => "DeepSeek keepalive partial hit",
                "retryable_error" => "DeepSeek keepalive will retry",
                "terminal_error" => "DeepSeek keepalive stopped",
                "preempted" => "DeepSeek keepalive preempted",
                _ => "DeepSeek keepalive observation",
            }
            .to_owned();
            let summary = match outcome.as_str() {
                "hit" => format!(
                    "Protected {} cached tokens",
                    json_scalar(&current, "cache_hit_tokens")
                        .unwrap_or_else(|| "0".to_owned())
                ),
                "miss" => "The cached prefix expired; automatic protection stopped".to_owned(),
                "partial" => {
                    "The cached prefix fell below the 90% hit threshold; automatic protection stopped"
                        .to_owned()
                }
                "retryable_error" => {
                    format!("Retryable HTTP {status}; one bounded retry is allowed")
                }
                "terminal_error" => {
                    format!("HTTP {status} made this snapshot ineligible for further keepalives")
                }
                "preempted" => {
                    "A real agent request preempted the background keepalive".to_owned()
                }
                _ => "Unknown cache-protection outcome".to_owned(),
            };
            let mut identity = vec![
                diagnostic_field("Log ID", id, true),
                diagnostic_field("Machine ID", machine_id, true),
                diagnostic_field("Producer ID", producer_id, true),
                diagnostic_field("Sequence", sequence.to_string(), true),
            ];
            for (label, key, copyable) in [
                ("Provider", "provider", false),
                ("Agent", "agent", false),
                ("Model", "model", false),
                ("Resolved model", "resolved_model", false),
                ("Session fingerprint", "session_fingerprint", true),
                ("Account fingerprint", "account_fingerprint", true),
            ] {
                optional_diagnostic_field(
                    &mut identity,
                    label,
                    json_scalar(&current, key),
                    copyable,
                );
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
                optional_diagnostic_field(
                    &mut protection,
                    label,
                    json_scalar(&current, key),
                    copyable,
                );
            }
            return Ok(Some(DiagnosticLogDetail {
                id: id.to_owned(),
                kind: "cache_anomaly".to_owned(),
                occurred_at_ms,
                title,
                summary,
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
            }));
        }
        if !provider_detail_matches_kind(kind, &current, &previous, gap_ms) {
            return Ok(None);
        }
        let status_code = json_i64(&current, "status");
        let status = json_scalar(&current, "status").unwrap_or_else(|| "unknown".to_owned());
        let agent = json_scalar(&current, "agent").unwrap_or_else(|| "unknown".to_owned());
        let intervening_provider_errors =
            json_i64(&evidence, "intervening_provider_errors").unwrap_or_default();
        let cause = (kind == "cache_anomaly")
            .then(|| cache_transition_cause(&current, &previous, intervening_provider_errors > 0));
        let title = cause.map_or_else(
            || format!("DeepSeek HTTP {status}"),
            |cause| cache_transition_title(cause).to_owned(),
        );
        let summary = if kind == "cache_anomaly" {
            match (cache_rate_label(&previous), cache_rate_label(&current)) {
                (Some(previous), Some(current)) => {
                    format!("Cache hit rate fell from {previous}% to {current}% within 30 minutes")
                }
                _ => "Active-session cache hit rate fell below 10%".to_owned(),
            }
        } else if status_code.is_some_and(|status| provider_status_impact(status).1) {
            format!(
                "{} request was blocked with HTTP status {status}",
                diagnostic_title(&agent)
            )
        } else {
            format!(
                "{} request attempt received retryable HTTP status {status}",
                diagnostic_title(&agent)
            )
        };
        let mut identity = vec![
            diagnostic_field("Log ID", id, true),
            diagnostic_field("Machine ID", machine_id, true),
            diagnostic_field("Producer ID", producer_id, true),
            diagnostic_field("Sequence", sequence.to_string(), true),
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
        let mut request = Vec::new();
        if let Some(cause) = cause {
            request.push(diagnostic_field("Cache classification", cause, false));
        } else if let Some(status) = status_code {
            request.push(diagnostic_field(
                "Observed impact",
                provider_status_impact(status).0,
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
                json_scalar(&previous, key),
                true,
            );
        }
        optional_diagnostic_field(
            &mut cache_identity,
            "Previous cache hit tokens",
            json_scalar(&previous, "cache_hit_tokens"),
            false,
        );
        optional_diagnostic_field(
            &mut cache_identity,
            "Previous cache miss tokens",
            json_scalar(&previous, "cache_miss_tokens"),
            false,
        );
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
        Ok(Some(DiagnosticLogDetail {
            id: id.to_owned(),
            kind: kind.to_owned(),
            occurred_at_ms,
            title,
            summary,
            sections,
            evidence: Some(evidence),
        }))
    }

    /// Storage metrics for the info panel: `(db_bytes, events_rows,
    /// sessions_soft_deleted)`. One round-trip.
    ///
    /// # Errors
    /// If the query fails.
    pub async fn storage_metrics(&self) -> Result<(i64, i64, i64)> {
        let row: (i64, i64, i64) = sqlx::query_as(
            "SELECT pg_database_size(current_database())::bigint, \
             (SELECT count(*) FROM events)::bigint, \
             (SELECT count(*) FROM sessions WHERE deleted_at IS NOT NULL)::bigint",
        )
        .fetch_one(&self.pool)
        .await
        .context("storage metrics")?;
        Ok(row)
    }
}

// --- enum ↔ text helpers -----------------------------------------------------
//
// We store enums as text columns rather than postgres enums so adding a
// variant doesn't need a schema migration. The cost is a tiny match arm.

fn origin_to_str(o: SessionOrigin) -> &'static str {
    match o {
        SessionOrigin::Api => "api",
        SessionOrigin::Web => "web",
    }
}

fn origin_from_str(s: &str) -> SessionOrigin {
    // Legacy "zed" rows (the retired Zed bridge) fall through to the Api default.
    match s {
        "web" => SessionOrigin::Web,
        _ => SessionOrigin::Api,
    }
}

fn status_to_str(s: Status) -> &'static str {
    match s {
        Status::Starting => "starting",
        Status::Running => "running",
        Status::Busy => "busy",
        Status::Exited => "exited",
        Status::Crashed => "crashed",
        Status::Interrupted => "interrupted",
    }
}

fn status_from_str(s: &str) -> Status {
    match s {
        "running" => Status::Running,
        "busy" => Status::Busy,
        "exited" => Status::Exited,
        "crashed" => Status::Crashed,
        "interrupted" => Status::Interrupted,
        _ => Status::Starting,
    }
}

fn provider_usage_metrics_within_bounds(
    event: &crate::machine_protocol::ProviderUsageEvent,
) -> bool {
    ![
        event.input_tokens,
        event.output_tokens,
        event.reasoning_tokens,
        event.cache_hit_tokens,
        event.cache_miss_tokens,
    ]
    .into_iter()
    .flatten()
    .any(|value| value > crate::machine_protocol::PROVIDER_USAGE_MAX_TOKENS)
        && event
            .duration_ms
            .is_none_or(|value| value <= crate::machine_protocol::PROVIDER_USAGE_MAX_DURATION_MS)
        && event
            .request_bytes
            .is_none_or(|value| value <= crate::machine_protocol::PROVIDER_USAGE_MAX_REQUEST_BYTES)
        && ![
            event.input_item_count,
            event.tool_count,
            event.system_block_count,
            event.compatibility_fixes,
        ]
        .into_iter()
        .flatten()
        .any(|value| value > crate::machine_protocol::PROVIDER_USAGE_MAX_SHAPE_COUNT)
        && ![
            event.cache_keepalive_interval_ms,
            event.cache_keepalive_source_age_ms,
        ]
        .into_iter()
        .flatten()
        .any(|value| value > crate::machine_protocol::PROVIDER_USAGE_MAX_KEEPALIVE_MS)
}

fn valid_provider_usage_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_optional_provider_usage_hex(value: Option<&String>, length: usize) -> bool {
    value.is_none_or(|value| valid_provider_usage_hex(value, length))
}

fn valid_provider_usage_keepalive_algorithm(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn provider_usage_model_family(model: &str) -> &'static str {
    let normalized = model
        .trim()
        .to_ascii_lowercase()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    if normalized.starts_with("deepseek-v4-pro") {
        "pro"
    } else if normalized.starts_with("deepseek-v4-flash")
        || matches!(normalized.as_str(), "deepseek-chat" | "deepseek-reasoner")
    {
        "flash"
    } else {
        "unknown"
    }
}

fn valid_provider_usage_v3_dimensions(event: &crate::machine_protocol::ProviderUsageEvent) -> bool {
    let model = event.resolved_model.as_deref().unwrap_or(&event.model);
    let session_valid = match event.session_attribution.as_str() {
        "response_lineage" | "prefix_root" | "explicit" => event.session_fingerprint.is_some(),
        "unattributed" => event.session_fingerprint.is_none(),
        _ => false,
    };
    let lane_valid = match (event.producer_id.as_str(), event.agent.as_str()) {
        ("codex-deepseek", "codex") => {
            event.client_protocol == "responses"
                && matches!(event.operation.as_str(), "responses" | "compact")
                && matches!(
                    event.upstream_protocol.as_str(),
                    "responses" | "chat_completions"
                )
                && ((event.upstream_protocol == "responses" && event.translation_mode == "native")
                    || (event.upstream_protocol == "chat_completions"
                        && event.translation_mode == "responses_to_chat"))
        }
        ("claude-deepseek", "claude") => {
            event.operation == "messages"
                && event.client_protocol == "anthropic_messages"
                && event.upstream_protocol == "anthropic_messages"
                && matches!(
                    event.translation_mode.as_str(),
                    "native" | "anthropic_compat"
                )
        }
        _ => false,
    };
    let request_role_valid = matches!(
        event.request_role.as_str(),
        "unknown" | "executor" | "planner" | "subagent" | "reviewer"
    );
    event.protocol == event.upstream_protocol
        && event.model_family == provider_usage_model_family(model)
        && request_role_valid
        && matches!(event.thinking_mode.as_str(), "enabled" | "disabled")
        && matches!(
            event.reasoning_effort.as_str(),
            "default" | "low" | "high" | "max"
        )
        && matches!(event.traffic_source.as_str(), "unattributed" | "cowboy")
        && (event.traffic_source != "cowboy" || event.session_attribution == "explicit")
        && session_valid
        && valid_optional_provider_usage_hex(event.session_fingerprint.as_ref(), 32)
        && valid_optional_provider_usage_hex(event.static_prefix_fingerprint.as_ref(), 32)
        && valid_optional_provider_usage_hex(event.request_prefix_fingerprint.as_ref(), 32)
        && valid_optional_provider_usage_hex(event.gateway_build.as_ref(), 16)
        && valid_optional_provider_usage_hex(event.gateway_boot_id.as_ref(), 16)
        && event.static_prefix_fingerprint.is_some()
        && event.request_prefix_fingerprint.is_some()
        && event.gateway_build.is_some()
        && event.gateway_boot_id.is_some()
        && event
            .resolved_model
            .as_ref()
            .is_none_or(|value| !value.is_empty() && value.len() <= 128)
        && event
            .model_revision
            .as_ref()
            .is_none_or(|value| !value.is_empty() && value.len() <= 128)
        && lane_valid
}

fn valid_provider_usage_v4_dimensions(event: &crate::machine_protocol::ProviderUsageEvent) -> bool {
    if !valid_provider_usage_v3_dimensions(event) {
        return false;
    }
    match event.request_purpose.as_str() {
        "interactive" => {
            event.cache_keepalive_outcome == "not_applicable"
                && event.cache_keepalive_algorithm.is_none()
                && event.cache_keepalive_attempt.is_none_or(|value| value == 0)
                && event
                    .cache_keepalive_interval_ms
                    .is_none_or(|value| value == 0)
                && event
                    .cache_keepalive_source_age_ms
                    .is_none_or(|value| value == 0)
                && event.source_request_prefix_fingerprint.is_none()
        }
        "cache_keepalive" => {
            event.session_attribution == "explicit"
                && event.traffic_source == "cowboy"
                && matches!(
                    event.cache_keepalive_outcome.as_str(),
                    "hit" | "miss" | "partial" | "retryable_error" | "terminal_error" | "preempted"
                )
                && event
                    .cache_keepalive_algorithm
                    .as_deref()
                    .is_some_and(valid_provider_usage_keepalive_algorithm)
                && event
                    .cache_keepalive_attempt
                    .is_some_and(|value| (1..=1_000).contains(&value))
                && event
                    .cache_keepalive_interval_ms
                    .is_some_and(|value| value > 0)
                && event.cache_keepalive_source_age_ms.is_some()
                && valid_optional_provider_usage_hex(
                    event.source_request_prefix_fingerprint.as_ref(),
                    32,
                )
                && event.source_request_prefix_fingerprint.is_some()
        }
        _ => false,
    }
}

fn valid_provider_usage_token_algebra(event: &crate::machine_protocol::ProviderUsageEvent) -> bool {
    if event.schema_version < 2 {
        return true;
    }
    match event.usage_observed {
        Some(false) => {
            event.input_tokens.is_none()
                && event.output_tokens.is_none()
                && event.reasoning_tokens.is_none()
                && event.cache_hit_tokens.is_none()
                && event.cache_miss_tokens.is_none()
                && event.cache_observation == "absent"
        }
        Some(true) => {
            let (Some(input), Some(output), Some(reasoning)) = (
                event.input_tokens,
                event.output_tokens,
                event.reasoning_tokens,
            ) else {
                return false;
            };
            if reasoning > output {
                return false;
            }
            match event.cache_observation.as_str() {
                "absent" => event.cache_hit_tokens.is_none() && event.cache_miss_tokens.is_none(),
                "derived" | "explicit" => {
                    let (Some(hit), Some(miss)) = (event.cache_hit_tokens, event.cache_miss_tokens)
                    else {
                        return false;
                    };
                    hit.checked_add(miss) == Some(input)
                }
                _ => false,
            }
        }
        None => false,
    }
}

fn validate_provider_usage_event(
    producer_id: &str,
    event: &crate::machine_protocol::ProviderUsageEvent,
) -> Result<()> {
    if event.producer_id != producer_id
        || event.provider != "deepseek"
        || !matches!(event.agent.as_str(), "codex" | "claude")
        || event.account_fingerprint.len() != 16
        || !event
            .account_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || event.model.len() > 128
        || !(100..=599).contains(&event.status)
        || !matches!(event.schema_version, 1..=4)
        || !matches!(
            event.operation.as_str(),
            "legacy" | "responses" | "compact" | "messages" | "chat_completions"
        )
        || !matches!(
            event.protocol.as_str(),
            "legacy" | "responses" | "chat_completions" | "anthropic_messages"
        )
        || !matches!(
            event.cache_observation.as_str(),
            "legacy" | "absent" | "derived" | "explicit"
        )
        || !matches!(
            (
                producer_id,
                event.producer_id.as_str(),
                event.agent.as_str()
            ),
            ("codex-deepseek", "codex-deepseek", "codex")
                | ("claude-deepseek", "claude-deepseek", "claude")
        )
        || !provider_usage_metrics_within_bounds(event)
        || !valid_provider_usage_token_algebra(event)
    {
        anyhow::bail!("invalid provider usage event");
    }
    if event.schema_version >= 2
        && (event.operation == "legacy"
            || event.protocol == "legacy"
            || event.cache_observation == "legacy"
            || event.usage_observed.is_none()
            || event.completed.is_none()
            || event.streaming.is_none()
            || event.duration_ms.is_none()
            || event.request_bytes.is_none()
            || event.input_item_count.is_none()
            || event.tool_count.is_none()
            || event.system_block_count.is_none()
            || event.has_previous_response_id.is_none()
            || event.compatibility_fixes.is_none())
    {
        anyhow::bail!("incomplete provider usage event");
    }
    if event.schema_version == 2
        && !matches!(
            (
                event.agent.as_str(),
                event.operation.as_str(),
                event.protocol.as_str()
            ),
            (
                "codex",
                "responses" | "compact",
                "responses" | "chat_completions"
            ) | ("claude", "messages", "anthropic_messages")
        )
    {
        anyhow::bail!("inconsistent provider usage dimensions");
    }
    if event.schema_version >= 3 && !valid_provider_usage_v3_dimensions(event) {
        anyhow::bail!("invalid version three provider usage dimensions");
    }
    if event.schema_version == 4 && !valid_provider_usage_v4_dimensions(event) {
        anyhow::bail!("invalid version four provider usage dimensions");
    }
    match event.cache_observation.as_str() {
        "absent" if event.cache_hit_tokens.is_some() || event.cache_miss_tokens.is_some() => {
            anyhow::bail!("cache counters require a measured observation");
        }
        "derived" | "explicit"
            if event.usage_observed != Some(true)
                || event.cache_hit_tokens.is_none()
                || event.cache_miss_tokens.is_none() =>
        {
            anyhow::bail!("measured cache observations require complete counters");
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod provider_usage_validation_tests {
    use super::*;
    use crate::machine_protocol::ProviderUsageEvent;

    pub(super) fn event() -> ProviderUsageEvent {
        ProviderUsageEvent {
            schema_version: 3,
            producer_id: "codex-deepseek".to_owned(),
            sequence: 1,
            occurred_at_ms: 1_786_000_000_000,
            account_fingerprint: "0123456789abcdef".to_owned(),
            provider: "deepseek".to_owned(),
            agent: "codex".to_owned(),
            model: "deepseek-v4-flash".to_owned(),
            model_family: "flash".to_owned(),
            resolved_model: Some("deepseek-v4-flash".to_owned()),
            model_revision: Some("fp-v4".to_owned()),
            request_role: "executor".to_owned(),
            status: 200,
            input_tokens: Some(10),
            output_tokens: Some(4),
            reasoning_tokens: Some(1),
            cache_hit_tokens: Some(7),
            cache_miss_tokens: Some(3),
            operation: "responses".to_owned(),
            protocol: "responses".to_owned(),
            client_protocol: "responses".to_owned(),
            upstream_protocol: "responses".to_owned(),
            translation_mode: "native".to_owned(),
            thinking_mode: "enabled".to_owned(),
            reasoning_effort: "high".to_owned(),
            session_fingerprint: Some("11111111111111111111111111111111".to_owned()),
            session_attribution: "response_lineage".to_owned(),
            traffic_source: "unattributed".to_owned(),
            static_prefix_fingerprint: Some("22222222222222222222222222222222".to_owned()),
            request_prefix_fingerprint: Some("33333333333333333333333333333333".to_owned()),
            gateway_build: Some("4444444444444444".to_owned()),
            gateway_boot_id: Some("5555555555555555".to_owned()),
            cache_observation: "derived".to_owned(),
            usage_observed: Some(true),
            completed: Some(true),
            streaming: Some(true),
            duration_ms: Some(42),
            request_bytes: Some(123),
            input_item_count: Some(2),
            tool_count: Some(1),
            system_block_count: Some(1),
            has_previous_response_id: Some(true),
            compatibility_fixes: Some(0),
            request_purpose: "interactive".to_owned(),
            cache_keepalive_outcome: "not_applicable".to_owned(),
            cache_keepalive_algorithm: None,
            cache_keepalive_attempt: Some(0),
            cache_keepalive_interval_ms: Some(0),
            cache_keepalive_source_age_ms: Some(0),
            source_request_prefix_fingerprint: None,
        }
    }

    #[test]
    fn controller_rejects_unknown_usage_producer() {
        let mut candidate = event();
        candidate.producer_id = "custom-deepseek".to_owned();
        assert!(validate_provider_usage_event("custom-deepseek", &candidate).is_err());
    }

    #[test]
    fn controller_rejects_oversized_usage_metric() {
        let mut candidate = event();
        candidate.cache_hit_tokens = Some(crate::machine_protocol::PROVIDER_USAGE_MAX_TOKENS + 1);
        assert!(validate_provider_usage_event("codex-deepseek", &candidate).is_err());
    }

    #[test]
    fn controller_accepts_unknown_role_for_existing_agent_lanes() {
        let mut candidate = event();
        candidate.request_role = "unknown".to_owned();
        assert!(validate_provider_usage_event("codex-deepseek", &candidate).is_ok());
    }

    #[test]
    fn controller_rejects_inconsistent_usage_token_algebra() {
        let mut candidate = event();
        candidate.cache_miss_tokens = Some(4);
        assert!(validate_provider_usage_event("codex-deepseek", &candidate).is_err());

        let mut candidate = event();
        candidate.reasoning_tokens = Some(5);
        assert!(validate_provider_usage_event("codex-deepseek", &candidate).is_err());
    }

    #[test]
    fn controller_accepts_complete_cache_keepalive_and_rejects_missing_source_prefix() {
        let mut candidate = event();
        candidate.schema_version = 4;
        candidate.request_purpose = "cache_keepalive".to_owned();
        candidate.cache_keepalive_outcome = "hit".to_owned();
        candidate.cache_keepalive_algorithm = Some("adaptive-replay-v1".to_owned());
        candidate.cache_keepalive_attempt = Some(1);
        candidate.cache_keepalive_interval_ms = Some(19_800_000);
        candidate.cache_keepalive_source_age_ms = Some(19_801_000);
        candidate.source_request_prefix_fingerprint =
            Some("66666666666666666666666666666666".to_owned());
        candidate.session_attribution = "explicit".to_owned();
        candidate.traffic_source = "cowboy".to_owned();
        assert!(validate_provider_usage_event("codex-deepseek", &candidate).is_ok());

        candidate.source_request_prefix_fingerprint = None;
        assert!(validate_provider_usage_event("codex-deepseek", &candidate).is_err());
    }

    #[test]
    fn cache_transition_classification_identifies_prefix_instability() {
        let previous = serde_json::json!({
            "agent": "claude",
            "operation": "messages",
            "model_family": "flash",
            "model_revision": "v4",
            "request_role": "executor",
            "upstream_protocol": "anthropic_messages",
            "translation_mode": "anthropic_compat",
            "thinking_mode": "enabled",
            "reasoning_effort": "high",
            "gateway_build": "build-1",
            "gateway_boot_id": "boot-1",
            "static_prefix_fingerprint": "prefix-1",
            "request_prefix_fingerprint": "request-1",
            "input_item_count": 20,
            "compatibility_fixes": 0,
        });
        let mut current = previous.clone();
        current["static_prefix_fingerprint"] = serde_json::json!("prefix-2");
        assert_eq!(
            cache_transition_cause(&current, &previous, false),
            "static_prefix_changed"
        );

        current = previous.clone();
        assert_eq!(
            cache_transition_cause(&current, &previous, false),
            "unexpected_exact_prefix_miss"
        );
        assert_eq!(
            cache_transition_cause(&current, &previous, true),
            "post_provider_error"
        );
    }

    #[test]
    fn diagnostic_detail_ids_cannot_relabel_healthy_provider_rows() {
        let current = serde_json::json!({
            "schema_version": 3,
            "status": 200,
            "session_fingerprint": "session",
            "session_attribution": "explicit",
            "cache_observation": "explicit",
            "static_prefix_fingerprint": "prefix",
            "input_tokens": 10_000,
            "cache_hit_tokens": 0,
            "cache_miss_tokens": 10_000,
        });
        let previous = serde_json::json!({
            "status": 200,
            "static_prefix_fingerprint": "prefix",
            "input_tokens": 10_000,
            "cache_hit_tokens": 9_500,
            "cache_miss_tokens": 500,
        });
        assert!(!provider_detail_matches_kind(
            "provider_error",
            &current,
            &previous,
            Some(1_000),
        ));
        assert!(provider_detail_matches_kind(
            "cache_anomaly",
            &current,
            &previous,
            Some(1_000),
        ));
        assert!(!provider_detail_matches_kind(
            "cache_anomaly",
            &current,
            &previous,
            Some(31 * 60 * 1_000),
        ));
    }

    #[test]
    fn provider_status_impact_separates_blockers_from_retryable_attempts() {
        for status in [400, 401, 402, 403, 404, 422] {
            assert!(provider_status_impact(status).1, "{status} should block");
        }
        for status in [408, 425, 429, 499, 500, 502, 503] {
            assert!(
                !provider_status_impact(status).1,
                "{status} should be retryable"
            );
        }
    }

    #[test]
    fn provider_diagnostic_ids_preserve_colons_inside_producer_ids() {
        assert_eq!(
            parse_provider_diagnostic_id("cache:hawk:gateway:boot:42", "cache:"),
            Some(("hawk", "gateway:boot", 42))
        );
    }
}

fn provider_usage_metric(value: Option<u64>) -> Result<Option<i64>> {
    value
        .map(i64::try_from)
        .transpose()
        .context("provider usage metric overflow")
}

async fn insert_provider_usage_event(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    machine_id: &str,
    producer_id: &str,
    event: &crate::machine_protocol::ProviderUsageEvent,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO provider_usage_events (machine_id, producer_id, sequence, occurred_at, \
         account_fingerprint, provider, agent, model, model_family, resolved_model, \
         model_revision, request_role, status, input_tokens, output_tokens, reasoning_tokens, \
         cache_hit_tokens, cache_miss_tokens, schema_version, operation, protocol, \
         client_protocol, upstream_protocol, translation_mode, thinking_mode, \
         reasoning_effort, session_fingerprint, session_attribution, traffic_source, \
         static_prefix_fingerprint, request_prefix_fingerprint, gateway_build, \
         gateway_boot_id, cache_observation, usage_observed, completed, streaming, \
         duration_ms, request_bytes, input_item_count, tool_count, system_block_count, \
         has_previous_response_id, compatibility_fixes, request_purpose, \
         cache_keepalive_outcome, cache_keepalive_algorithm, cache_keepalive_attempt, \
         cache_keepalive_interval_ms, cache_keepalive_source_age_ms, \
         source_request_prefix_fingerprint) VALUES ( \
         $1, $2, $3, to_timestamp($4::double precision / 1000), $5, $6, $7, $8, $9, \
         $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, \
         $24, $25, $26, $27, $28, $29, $30, $31, $32, $33, $34, $35, $36, $37, \
         $38, $39, $40, $41, $42, $43, $44, $45, $46, $47, $48, $49, $50, \
         $51) ON CONFLICT DO NOTHING",
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
    .context("insert provider usage event")?;
    Ok(())
}

const PROVIDER_USAGE_AGGREGATE_COLUMNS: &str = "count(*) FILTER (WHERE request_purpose = 'interactive')::bigint AS requests, \
     count(*) FILTER (WHERE request_purpose = 'interactive' AND status >= 400)::bigint AS errors, \
     count(*) FILTER (WHERE request_purpose = 'interactive' AND status BETWEEN 400 AND 499 AND status NOT IN (408, 425, 429, 499))::bigint AS blocking_errors, \
     count(*) FILTER (WHERE request_purpose = 'interactive' AND (status IN (408, 425, 429, 499) OR status >= 500))::bigint AS transient_errors, \
     count(*) FILTER (WHERE request_purpose = 'interactive' AND completed IS TRUE)::bigint AS completed_requests, \
     count(completed) FILTER (WHERE request_purpose = 'interactive')::bigint AS completion_observations, \
     count(*) FILTER (WHERE request_purpose = 'interactive' AND usage_observed IS TRUE)::bigint AS usage_observations, \
     least(coalesce(sum(input_tokens::numeric) FILTER (WHERE request_purpose = 'interactive'), 0), 9223372036854775807)::bigint AS input_tokens, \
     least(coalesce(sum(output_tokens::numeric) FILTER (WHERE request_purpose = 'interactive'), 0), 9223372036854775807)::bigint AS output_tokens, \
     least(coalesce(sum(reasoning_tokens::numeric) FILTER (WHERE request_purpose = 'interactive'), 0), 9223372036854775807)::bigint AS reasoning_tokens, \
     least(coalesce(sum(cache_hit_tokens::numeric) FILTER (WHERE request_purpose = 'interactive' AND cache_observation IN ('explicit', 'derived')), 0), 9223372036854775807)::bigint AS cache_hit_tokens, \
     least(coalesce(sum(cache_miss_tokens::numeric) FILTER (WHERE request_purpose = 'interactive' AND cache_observation IN ('explicit', 'derived')), 0), 9223372036854775807)::bigint AS cache_miss_tokens, \
     count(*) FILTER (WHERE request_purpose = 'interactive' AND cache_observation IN ('explicit', 'derived') AND cache_hit_tokens IS NOT NULL AND cache_miss_tokens IS NOT NULL)::bigint AS cache_observations, \
     count(*) FILTER (WHERE request_purpose = 'interactive' AND cache_observation = 'explicit')::bigint AS explicit_cache_observations, \
     count(*) FILTER (WHERE request_purpose = 'interactive' AND cache_observation = 'derived')::bigint AS derived_cache_observations, \
     count(*) FILTER (WHERE request_purpose = 'interactive' AND cache_observation = 'absent')::bigint AS absent_cache_observations, \
     count(*) FILTER (WHERE request_purpose = 'interactive' AND cache_observation IN ('explicit', 'derived') AND cache_hit_tokens::numeric + cache_miss_tokens::numeric > 0 AND cache_hit_tokens::numeric * 10 < cache_hit_tokens::numeric + cache_miss_tokens::numeric)::bigint AS cold_cache_requests, \
     count(*) FILTER (WHERE request_purpose = 'interactive' AND cache_observation IN ('explicit', 'derived') AND cache_hit_tokens::numeric + cache_miss_tokens::numeric > 0 AND cache_hit_tokens::numeric * 10 >= 9 * (cache_hit_tokens::numeric + cache_miss_tokens::numeric))::bigint AS hot_cache_requests, \
     least(coalesce(sum(duration_ms::numeric) FILTER (WHERE request_purpose = 'interactive'), 0), 9223372036854775807)::bigint AS duration_ms, \
     count(duration_ms) FILTER (WHERE request_purpose = 'interactive')::bigint AS duration_observations, \
     least(coalesce(sum(request_bytes::numeric) FILTER (WHERE request_purpose = 'interactive'), 0), 9223372036854775807)::bigint AS request_bytes, \
     count(request_bytes) FILTER (WHERE request_purpose = 'interactive')::bigint AS request_shape_observations, \
     least(coalesce(sum(input_item_count::numeric) FILTER (WHERE request_purpose = 'interactive'), 0), 9223372036854775807)::bigint AS input_item_count, \
     least(coalesce(sum(tool_count::numeric) FILTER (WHERE request_purpose = 'interactive'), 0), 9223372036854775807)::bigint AS tool_count, \
     least(coalesce(sum(system_block_count::numeric) FILTER (WHERE request_purpose = 'interactive'), 0), 9223372036854775807)::bigint AS system_block_count, \
     count(*) FILTER (WHERE request_purpose = 'interactive' AND has_previous_response_id IS TRUE)::bigint AS previous_response_requests, \
     least(coalesce(sum(compatibility_fixes::numeric) FILTER (WHERE request_purpose = 'interactive'), 0), 9223372036854775807)::bigint AS compatibility_fixes, \
     count(*) FILTER (WHERE request_purpose = 'interactive' AND streaming IS TRUE)::bigint AS streaming_requests, \
     count(*) FILTER (WHERE request_purpose = 'cache_keepalive')::bigint AS cache_keepalive_requests, \
     count(*) FILTER (WHERE request_purpose = 'cache_keepalive' AND cache_keepalive_outcome = 'hit')::bigint AS cache_keepalive_hits, \
     count(*) FILTER (WHERE request_purpose = 'cache_keepalive' AND cache_keepalive_outcome = 'miss')::bigint AS cache_keepalive_misses, \
     count(*) FILTER (WHERE request_purpose = 'cache_keepalive' AND cache_keepalive_outcome = 'partial')::bigint AS cache_keepalive_partials, \
     count(*) FILTER (WHERE request_purpose = 'cache_keepalive' AND cache_keepalive_outcome = 'retryable_error')::bigint AS cache_keepalive_retryable_errors, \
     count(*) FILTER (WHERE request_purpose = 'cache_keepalive' AND cache_keepalive_outcome = 'terminal_error')::bigint AS cache_keepalive_terminal_errors, \
     count(*) FILTER (WHERE request_purpose = 'cache_keepalive' AND cache_keepalive_outcome = 'preempted')::bigint AS cache_keepalive_preemptions, \
     count(*) FILTER (WHERE request_purpose = 'cache_keepalive' AND usage_observed IS TRUE)::bigint AS cache_keepalive_usage_observations, \
     least(coalesce(sum(input_tokens::numeric) FILTER (WHERE request_purpose = 'cache_keepalive'), 0), 9223372036854775807)::bigint AS cache_keepalive_input_tokens, \
     least(coalesce(sum(output_tokens::numeric) FILTER (WHERE request_purpose = 'cache_keepalive'), 0), 9223372036854775807)::bigint AS cache_keepalive_output_tokens, \
     least(coalesce(sum(reasoning_tokens::numeric) FILTER (WHERE request_purpose = 'cache_keepalive'), 0), 9223372036854775807)::bigint AS cache_keepalive_reasoning_tokens, \
     least(coalesce(sum(cache_hit_tokens::numeric) FILTER (WHERE request_purpose = 'cache_keepalive' AND cache_observation IN ('explicit', 'derived')), 0), 9223372036854775807)::bigint AS cache_keepalive_hit_tokens, \
     least(coalesce(sum(cache_miss_tokens::numeric) FILTER (WHERE request_purpose = 'cache_keepalive' AND cache_observation IN ('explicit', 'derived')), 0), 9223372036854775807)::bigint AS cache_keepalive_miss_tokens, \
     least(coalesce(sum(duration_ms::numeric) FILTER (WHERE request_purpose = 'cache_keepalive'), 0), 9223372036854775807)::bigint AS cache_keepalive_duration_ms, \
     count(duration_ms) FILTER (WHERE request_purpose = 'cache_keepalive')::bigint AS cache_keepalive_duration_observations, \
     least(coalesce(sum(cache_keepalive_interval_ms::numeric) FILTER (WHERE request_purpose = 'cache_keepalive'), 0), 9223372036854775807)::bigint AS cache_keepalive_interval_ms, \
     count(cache_keepalive_interval_ms) FILTER (WHERE request_purpose = 'cache_keepalive')::bigint AS cache_keepalive_interval_observations, \
     least(coalesce(sum(cache_keepalive_source_age_ms::numeric) FILTER (WHERE request_purpose = 'cache_keepalive'), 0), 9223372036854775807)::bigint AS cache_keepalive_source_age_ms, \
     count(cache_keepalive_source_age_ms) FILTER (WHERE request_purpose = 'cache_keepalive')::bigint AS cache_keepalive_source_age_observations";

#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderUsageBreakdown {
    summary: UsageAggregate,
    by_agent: std::collections::BTreeMap<String, UsageAggregate>,
    by_agent_model:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, UsageAggregate>>,
    by_agent_model_family:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, UsageAggregate>>,
    by_agent_request_role:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, UsageAggregate>>,
    by_machine: std::collections::BTreeMap<String, UsageAggregate>,
    by_operation: std::collections::BTreeMap<String, UsageAggregate>,
    by_model: std::collections::BTreeMap<String, UsageAggregate>,
    by_resolved_model: std::collections::BTreeMap<String, UsageAggregate>,
    by_billing_model: std::collections::BTreeMap<String, UsageAggregate>,
    by_model_revision: std::collections::BTreeMap<String, UsageAggregate>,
    by_model_family: std::collections::BTreeMap<String, UsageAggregate>,
    by_request_role: std::collections::BTreeMap<String, UsageAggregate>,
    by_protocol: std::collections::BTreeMap<String, UsageAggregate>,
    by_client_protocol: std::collections::BTreeMap<String, UsageAggregate>,
    by_upstream_protocol: std::collections::BTreeMap<String, UsageAggregate>,
    by_translation_mode: std::collections::BTreeMap<String, UsageAggregate>,
    by_thinking_mode: std::collections::BTreeMap<String, UsageAggregate>,
    by_reasoning_effort: std::collections::BTreeMap<String, UsageAggregate>,
    by_session_attribution: std::collections::BTreeMap<String, UsageAggregate>,
    by_traffic_source: std::collections::BTreeMap<String, UsageAggregate>,
    by_gateway_build: std::collections::BTreeMap<String, UsageAggregate>,
    by_schema_version: std::collections::BTreeMap<String, UsageAggregate>,
    by_agent_operation:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, UsageAggregate>>,
    by_agent_billing_model:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, UsageAggregate>>,
}

impl ProviderUsageBreakdown {
    fn add_row(&mut self, row: &sqlx::postgres::PgRow) {
        let aggregate = UsageAggregate::from_row(row);
        let agent = row.get::<String, _>("agent");
        let model = row.get::<String, _>("model");
        let resolved_model = row.get::<String, _>("resolved_model");
        let billing_model = if resolved_model.is_empty() {
            model.clone()
        } else {
            resolved_model.clone()
        };
        let model_family = row.get::<String, _>("model_family");
        let request_role = row.get::<String, _>("request_role");
        let operation = row.get::<String, _>("operation");
        self.summary.add(&aggregate);
        for (map, key) in [
            (&mut self.by_agent, agent.clone()),
            (&mut self.by_machine, row.get("machine_id")),
            (&mut self.by_operation, operation.clone()),
            (&mut self.by_model, model.clone()),
            (&mut self.by_resolved_model, resolved_model),
            (&mut self.by_billing_model, billing_model.clone()),
            (&mut self.by_model_revision, row.get("model_revision")),
            (&mut self.by_model_family, model_family.clone()),
            (&mut self.by_request_role, request_role.clone()),
            (&mut self.by_protocol, row.get("protocol")),
            (&mut self.by_client_protocol, row.get("client_protocol")),
            (&mut self.by_upstream_protocol, row.get("upstream_protocol")),
            (&mut self.by_translation_mode, row.get("translation_mode")),
            (&mut self.by_thinking_mode, row.get("thinking_mode")),
            (&mut self.by_reasoning_effort, row.get("reasoning_effort")),
            (
                &mut self.by_session_attribution,
                row.get("session_attribution"),
            ),
            (&mut self.by_traffic_source, row.get("traffic_source")),
            (&mut self.by_gateway_build, row.get("gateway_build")),
            (
                &mut self.by_schema_version,
                row.get::<i32, _>("schema_version").to_string(),
            ),
        ] {
            map.entry(key).or_default().add(&aggregate);
        }
        self.by_agent_model
            .entry(agent.clone())
            .or_default()
            .entry(model)
            .or_default()
            .add(&aggregate);
        self.by_agent_billing_model
            .entry(agent.clone())
            .or_default()
            .entry(billing_model)
            .or_default()
            .add(&aggregate);
        self.by_agent_model_family
            .entry(agent.clone())
            .or_default()
            .entry(model_family)
            .or_default()
            .add(&aggregate);
        self.by_agent_request_role
            .entry(agent.clone())
            .or_default()
            .entry(request_role)
            .or_default()
            .add(&aggregate);
        self.by_agent_operation
            .entry(agent)
            .or_default()
            .entry(operation)
            .or_default()
            .add(&aggregate);
    }
}

async fn load_provider_usage_breakdown(
    pool: &PgPool,
    provider: &str,
    days: i32,
) -> Result<ProviderUsageBreakdown> {
    let query = format!(
        "SELECT agent, machine_id, coalesce(model, '') AS model, \
         coalesce(resolved_model, '') AS resolved_model, coalesce(model_revision, '') AS model_revision, \
         coalesce(gateway_build, '') AS gateway_build, model_family, request_role, \
         operation, protocol, client_protocol, upstream_protocol, translation_mode, \
         thinking_mode, reasoning_effort, session_attribution, traffic_source, schema_version, \
         {PROVIDER_USAGE_AGGREGATE_COLUMNS} FROM provider_usage_events WHERE provider = $1 \
         AND occurred_at >= now() - make_interval(days => $2::int) \
         GROUP BY agent, machine_id, model, resolved_model, model_revision, gateway_build, \
         model_family, request_role, operation, protocol, \
         client_protocol, upstream_protocol, translation_mode, thinking_mode, reasoning_effort, \
         session_attribution, traffic_source, schema_version"
    );
    let rows = sqlx::query(&query)
        .bind(provider)
        .bind(days)
        .fetch_all(pool)
        .await
        .context("aggregate provider usage")?;
    let mut breakdown = ProviderUsageBreakdown::default();
    for row in &rows {
        breakdown.add_row(row);
    }
    Ok(breakdown)
}

async fn load_provider_usage_breakdown_hours(
    pool: &PgPool,
    provider: &str,
    hours: i32,
) -> Result<ProviderUsageBreakdown> {
    let query = format!(
        "SELECT agent, machine_id, coalesce(model, '') AS model, \
         coalesce(resolved_model, '') AS resolved_model, coalesce(model_revision, '') AS model_revision, \
         coalesce(gateway_build, '') AS gateway_build, model_family, request_role, \
         operation, protocol, client_protocol, upstream_protocol, translation_mode, \
         thinking_mode, reasoning_effort, session_attribution, traffic_source, schema_version, \
         {PROVIDER_USAGE_AGGREGATE_COLUMNS} FROM provider_usage_events WHERE provider = $1 \
         AND occurred_at >= now() - make_interval(hours => $2::int) \
         GROUP BY agent, machine_id, model, resolved_model, model_revision, gateway_build, \
         model_family, request_role, operation, protocol, \
         client_protocol, upstream_protocol, translation_mode, thinking_mode, reasoning_effort, \
         session_attribution, traffic_source, schema_version"
    );
    let rows = sqlx::query(&query)
        .bind(provider)
        .bind(hours)
        .fetch_all(pool)
        .await
        .context("aggregate rolling provider usage")?;
    let mut breakdown = ProviderUsageBreakdown::default();
    for row in &rows {
        breakdown.add_row(row);
    }
    Ok(breakdown)
}

async fn load_daily_provider_usage(
    pool: &PgPool,
    provider: &str,
    days: i32,
) -> Result<Vec<serde_json::Value>> {
    let query = format!(
        "SELECT to_char(date_trunc('day', occurred_at), 'YYYY-MM-DD') AS day, \
         {PROVIDER_USAGE_AGGREGATE_COLUMNS} FROM provider_usage_events WHERE provider = $1 \
         AND occurred_at >= now() - make_interval(days => $2::int) \
         GROUP BY date_trunc('day', occurred_at) ORDER BY date_trunc('day', occurred_at)"
    );
    Ok(sqlx::query(&query)
        .bind(provider)
        .bind(days)
        .fetch_all(pool)
        .await
        .context("aggregate daily provider usage")?
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "day": row.get::<String, _>("day"),
                "totals": UsageAggregate::from_row(&row),
            })
        })
        .collect())
}

async fn load_provider_usage_coverage(
    pool: &PgPool,
    provider: &str,
    days: i32,
) -> Result<Vec<serde_json::Value>> {
    Ok(sqlx::query(
        "SELECT machine_id, agent, last_sequence, \
         (extract(epoch FROM last_received_at) * 1000)::bigint AS last_received_at_ms \
         FROM provider_usage_producers WHERE provider = $1 AND last_received_at >= \
         now() - make_interval(days => $2::int) ORDER BY machine_id, agent",
    )
    .bind(provider)
    .bind(days)
    .fetch_all(pool)
    .await
    .context("load provider usage coverage")?
    .into_iter()
    .map(|row| {
        serde_json::json!({
            "machine": row.get::<String, _>("machine_id"),
            "agent": row.get::<String, _>("agent"),
            "lastSequence": row.get::<i64, _>("last_sequence"),
            "lastReceivedAtMs": row.get::<i64, _>("last_received_at_ms"),
        })
    })
    .collect())
}

async fn load_provider_usage_available_agents(
    pool: &PgPool,
    provider: &str,
    days: i32,
) -> Result<Vec<String>> {
    sqlx::query_scalar(
        "SELECT DISTINCT agent FROM provider_usage_events WHERE provider = $1 AND occurred_at >= \
         now() - make_interval(days => $2::int) ORDER BY agent",
    )
    .bind(provider)
    .bind(days)
    .fetch_all(pool)
    .await
    .context("load provider usage available agents")
}

async fn load_filtered_provider_usage_breakdown(
    pool: &PgPool,
    provider: &str,
    from_ms: i64,
    to_ms: i64,
    agent: Option<&str>,
    model_family: Option<&str>,
) -> Result<ProviderUsageBreakdown> {
    let query = format!(
        "SELECT agent, machine_id, coalesce(model, '') AS model, \
         coalesce(resolved_model, '') AS resolved_model, coalesce(model_revision, '') AS model_revision, \
         coalesce(gateway_build, '') AS gateway_build, model_family, request_role, \
         operation, protocol, client_protocol, upstream_protocol, translation_mode, \
         thinking_mode, reasoning_effort, session_attribution, traffic_source, schema_version, \
         {PROVIDER_USAGE_AGGREGATE_COLUMNS} FROM provider_usage_events WHERE provider = $1 \
         AND occurred_at >= to_timestamp($2::double precision / 1000) \
         AND occurred_at <= to_timestamp($3::double precision / 1000) \
         AND ($4::text IS NULL OR agent = $4) AND ($5::text IS NULL OR model_family = $5) \
         GROUP BY agent, machine_id, model, resolved_model, model_revision, gateway_build, \
         model_family, request_role, operation, protocol, \
         client_protocol, upstream_protocol, translation_mode, thinking_mode, reasoning_effort, \
         session_attribution, traffic_source, schema_version"
    );
    let rows = sqlx::query(&query)
        .bind(provider)
        .bind(from_ms)
        .bind(to_ms)
        .bind(agent)
        .bind(model_family)
        .fetch_all(pool)
        .await
        .context("aggregate filtered provider usage")?;
    let mut breakdown = ProviderUsageBreakdown::default();
    for row in &rows {
        breakdown.add_row(row);
    }
    Ok(breakdown)
}

async fn load_provider_usage_timeline(
    pool: &PgPool,
    provider: &str,
    from_ms: i64,
    to_ms: i64,
    bucket: &str,
    agent: Option<&str>,
    model_family: Option<&str>,
) -> Result<Vec<serde_json::Value>> {
    let truncation = match bucket {
        "hour" => "hour",
        "day" => "day",
        _ => anyhow::bail!("invalid provider usage timeline bucket"),
    };
    let query = format!(
        "SELECT (extract(epoch FROM date_trunc('{truncation}', occurred_at)) * 1000)::bigint \
         AS start_ms, {PROVIDER_USAGE_AGGREGATE_COLUMNS} FROM provider_usage_events \
         WHERE provider = $1 AND occurred_at >= to_timestamp($2::double precision / 1000) \
         AND occurred_at <= to_timestamp($3::double precision / 1000) \
         AND ($4::text IS NULL OR agent = $4) AND ($5::text IS NULL OR model_family = $5) \
         GROUP BY date_trunc('{truncation}', occurred_at) ORDER BY date_trunc('{truncation}', occurred_at)"
    );
    Ok(sqlx::query(&query)
        .bind(provider)
        .bind(from_ms)
        .bind(to_ms)
        .bind(agent)
        .bind(model_family)
        .fetch_all(pool)
        .await
        .context("aggregate provider usage timeline")?
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "startMs": row.get::<i64, _>("start_ms"),
                "totals": UsageAggregate::from_row(&row),
            })
        })
        .collect())
}

#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LowHitBreakdown {
    summary: UsageAggregate,
    by_cause: std::collections::BTreeMap<String, UsageAggregate>,
    by_cause_model:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, UsageAggregate>>,
}

#[allow(clippy::too_many_lines)] // SQL lineage classification is clearest as one bounded query
async fn load_provider_usage_low_hit(
    pool: &PgPool,
    provider: &str,
    from_ms: i64,
    to_ms: i64,
    agent: Option<&str>,
    model_family: Option<&str>,
) -> Result<LowHitBreakdown> {
    let query = format!(
        "WITH lineage AS ( \
           SELECT events.*, \
             lag(model_family) OVER session_window AS previous_model_family, \
             lag(model_revision) OVER session_window AS previous_model_revision, \
             lag(request_role) OVER session_window AS previous_request_role, \
             lag(upstream_protocol) OVER session_window AS previous_upstream_protocol, \
             lag(translation_mode) OVER session_window AS previous_translation_mode, \
             lag(thinking_mode) OVER session_window AS previous_thinking_mode, \
             lag(reasoning_effort) OVER session_window AS previous_reasoning_effort, \
             lag(static_prefix_fingerprint) OVER session_window AS previous_static_prefix, \
             lag(request_prefix_fingerprint) OVER session_window AS previous_request_prefix, \
             lag(input_item_count) OVER session_window AS previous_input_item_count, \
             lag(gateway_build) OVER session_window AS previous_gateway_build, \
             lag(gateway_boot_id) OVER session_window AS previous_gateway_boot_id, \
             lag(cache_hit_tokens) OVER session_window AS previous_cache_hit_tokens, \
             lag(cache_miss_tokens) OVER session_window AS previous_cache_miss_tokens, \
             lag(occurred_at) OVER session_window AS previous_occurred_at \
           FROM provider_usage_events events \
           WHERE provider = $1 AND occurred_at >= \
             to_timestamp($2::double precision / 1000) - interval '6 hours' \
             AND occurred_at <= to_timestamp($3::double precision / 1000) \
             AND request_purpose = 'interactive' \
             AND ($4::text IS NULL OR agent = $4) \
           WINDOW session_window AS ( \
             PARTITION BY machine_id, producer_id, account_fingerprint, agent, \
               coalesce(session_fingerprint, producer_id || ':' || sequence::text) \
             ORDER BY occurred_at, sequence \
           ) \
         ), classified AS ( \
           SELECT lineage.*, CASE \
             WHEN schema_version < 3 THEN 'legacy_unattributed' \
             WHEN session_fingerprint IS NULL AND has_previous_response_id IS TRUE \
               THEN 'session_lineage_unavailable' \
             WHEN session_fingerprint IS NULL THEN 'unattributed' \
             WHEN session_attribution = 'prefix_root' THEN 'prefix_lineage_ambiguous' \
             WHEN previous_occurred_at IS NULL THEN 'first_session_observation' \
             WHEN operation = 'compact' THEN 'client_compaction' \
             WHEN occurred_at - previous_occurred_at >= interval '6 hours' \
               AND previous_cache_hit_tokens * 10 >= \
                 9 * (previous_cache_hit_tokens + previous_cache_miss_tokens) \
               THEN 'probable_cache_eviction' \
             WHEN gateway_build IS DISTINCT FROM previous_gateway_build THEN 'gateway_build_changed' \
             WHEN gateway_boot_id IS DISTINCT FROM previous_gateway_boot_id THEN 'post_gateway_restart' \
             WHEN model_family IS DISTINCT FROM previous_model_family THEN 'model_changed' \
             WHEN model_revision IS DISTINCT FROM previous_model_revision THEN 'model_revision_changed' \
             WHEN request_role IS DISTINCT FROM previous_request_role THEN 'request_role_changed' \
             WHEN upstream_protocol IS DISTINCT FROM previous_upstream_protocol THEN 'protocol_changed' \
             WHEN translation_mode IS DISTINCT FROM previous_translation_mode THEN 'translation_changed' \
             WHEN thinking_mode IS DISTINCT FROM previous_thinking_mode \
               OR reasoning_effort IS DISTINCT FROM previous_reasoning_effort \
               THEN 'reasoning_configuration_changed' \
             WHEN compatibility_fixes > 0 THEN 'compatibility_rewrite' \
             WHEN static_prefix_fingerprint IS DISTINCT FROM previous_static_prefix THEN 'static_prefix_changed' \
             WHEN (agent <> 'codex' OR has_previous_response_id IS NOT TRUE) \
               AND input_item_count < previous_input_item_count THEN 'history_rewrite' \
             WHEN request_prefix_fingerprint = previous_request_prefix \
               AND previous_cache_hit_tokens * 10 >= \
                 9 * (previous_cache_hit_tokens + previous_cache_miss_tokens) \
               THEN 'unexpected_exact_prefix_miss' \
             ELSE 'unexplained_low_hit' END AS cause \
           FROM lineage WHERE occurred_at >= to_timestamp($2::double precision / 1000) \
             AND occurred_at <= to_timestamp($3::double precision / 1000) \
             AND ($5::text IS NULL OR model_family = $5) \
             AND cache_observation IN ('explicit', 'derived') \
             AND coalesce(input_tokens, 0) >= 8000 \
             AND cache_hit_tokens + cache_miss_tokens > 0 \
             AND cache_hit_tokens * 10 < cache_hit_tokens + cache_miss_tokens \
         ) \
         SELECT cause, coalesce(resolved_model, model, '') AS model, {PROVIDER_USAGE_AGGREGATE_COLUMNS} \
         FROM classified GROUP BY cause, coalesce(resolved_model, model, '') \
         ORDER BY cause, coalesce(resolved_model, model, '')"
    );
    let rows = sqlx::query(&query)
        .bind(provider)
        .bind(from_ms)
        .bind(to_ms)
        .bind(agent)
        .bind(model_family)
        .fetch_all(pool)
        .await
        .context("classify low-hit provider usage")?;
    let mut result = LowHitBreakdown::default();
    for row in rows {
        let cause = row.get::<String, _>("cause");
        let model = row.get::<String, _>("model");
        let aggregate = UsageAggregate::from_row(&row);
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
    Ok(result)
}

async fn load_filtered_provider_usage_coverage(
    pool: &PgPool,
    provider: &str,
    from_ms: i64,
    to_ms: i64,
    agent: Option<&str>,
    model_family: Option<&str>,
) -> Result<Vec<serde_json::Value>> {
    Ok(sqlx::query(
        "SELECT producers.machine_id, producers.agent, producers.last_sequence, \
         (extract(epoch FROM producers.last_received_at) * 1000)::bigint AS last_received_at_ms \
         FROM provider_usage_producers producers WHERE producers.provider = $1 \
         AND ($4::text IS NULL OR producers.agent = $4) AND EXISTS ( \
           SELECT 1 FROM provider_usage_events events \
           WHERE events.machine_id = producers.machine_id \
             AND events.producer_id = producers.producer_id AND events.provider = $1 \
             AND events.occurred_at >= to_timestamp($2::double precision / 1000) \
             AND events.occurred_at <= to_timestamp($3::double precision / 1000) \
             AND ($5::text IS NULL OR events.model_family = $5) \
         ) ORDER BY producers.machine_id, producers.agent",
    )
    .bind(provider)
    .bind(from_ms)
    .bind(to_ms)
    .bind(agent)
    .bind(model_family)
    .fetch_all(pool)
    .await
    .context("load filtered provider usage coverage")?
    .into_iter()
    .map(|row| {
        serde_json::json!({
            "machine": row.get::<String, _>("machine_id"),
            "agent": row.get::<String, _>("agent"),
            "lastSequence": row.get::<i64, _>("last_sequence"),
            "lastReceivedAtMs": row.get::<i64, _>("last_received_at_ms"),
        })
    })
    .collect())
}

impl PostgresStorage {
    /// Persist one authenticated Machine usage batch idempotently and advance
    /// its producer watermark in the same transaction.
    pub async fn ingest_provider_usage(
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
            .context("begin provider usage batch")?;
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
             account_fingerprint, agent, last_sequence, last_occurred_at) VALUES \
             ($1, $2, $3, $4, $5, $6, to_timestamp($7::double precision / 1000)) \
             ON CONFLICT (machine_id, producer_id) \
             DO UPDATE SET provider = EXCLUDED.provider, \
             account_fingerprint = EXCLUDED.account_fingerprint, agent = EXCLUDED.agent, \
             last_sequence = GREATEST(provider_usage_producers.last_sequence, \
             EXCLUDED.last_sequence), last_occurred_at = GREATEST( \
             provider_usage_producers.last_occurred_at, EXCLUDED.last_occurred_at), \
             last_received_at = now()",
        )
        .bind(machine_id)
        .bind(producer_id)
        .bind(&newest.provider)
        .bind(&newest.account_fingerprint)
        .bind(&newest.agent)
        .bind(i64::try_from(last).context("usage watermark overflow")?)
        .bind(newest.occurred_at_ms)
        .execute(&mut *transaction)
        .await
        .context("upsert provider usage producer")?;
        transaction
            .commit()
            .await
            .context("commit provider usage batch")?;
        Ok(last)
    }

    /// Aggregate gateway-measured usage separately from provider-owned account facts.
    pub async fn provider_usage_summary(
        &self,
        provider: &str,
        days: i32,
        retention_days: i32,
    ) -> Result<serde_json::Value> {
        let breakdown = load_provider_usage_breakdown(&self.pool, provider, days).await?;
        let last_24_hours = load_provider_usage_breakdown_hours(&self.pool, provider, 24).await?;
        let daily = load_daily_provider_usage(&self.pool, provider, days).await?;
        let producers = load_provider_usage_coverage(&self.pool, provider, days).await?;
        let available_agents =
            load_provider_usage_available_agents(&self.pool, provider, retention_days).await?;
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

    /// Query one bounded, provider-owned telemetry view without refreshing
    /// account balance facts. Filters are closed enums at the HTTP boundary;
    /// this method repeats validation because persistence is the authority for
    /// long-lived diagnostic data.
    pub async fn provider_usage_activity(
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
        let (breakdown, timeline, low_hit, producers) = tokio::try_join!(
            load_filtered_provider_usage_breakdown(
                &self.pool,
                provider,
                from_ms,
                to_ms,
                agent,
                model_family,
            ),
            load_provider_usage_timeline(
                &self.pool,
                provider,
                from_ms,
                to_ms,
                bucket,
                agent,
                model_family,
            ),
            load_provider_usage_low_hit(&self.pool, provider, from_ms, to_ms, agent, model_family,),
            load_filtered_provider_usage_coverage(
                &self.pool,
                provider,
                from_ms,
                to_ms,
                agent,
                model_family,
            ),
        )?;
        Ok(serde_json::json!({
            "source": "cowboy", "windowField": "occurred_at", "retentionDays": 30,
            "windowSeconds": window_seconds, "fromMs": from_ms, "toMs": to_ms, "bucket": bucket,
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

    /// Bound the internal ledger independently from the shorter UI window.
    pub async fn purge_provider_usage(&self, retention_days: i32) -> Result<u64> {
        let result = sqlx::query(
            "DELETE FROM provider_usage_events WHERE received_at < \
             now() - make_interval(days => $1::int)",
        )
        .bind(retention_days)
        .execute(&self.pool)
        .await
        .context("purge provider usage ledger")?;
        Ok(result.rows_affected())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageAggregate {
    requests: i64,
    errors: i64,
    blocking_errors: i64,
    transient_errors: i64,
    completed_requests: i64,
    completion_observations: i64,
    usage_observations: i64,
    input_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
    cache_hit_tokens: i64,
    cache_miss_tokens: i64,
    cache_observations: i64,
    explicit_cache_observations: i64,
    derived_cache_observations: i64,
    absent_cache_observations: i64,
    cold_cache_requests: i64,
    hot_cache_requests: i64,
    duration_ms: i64,
    duration_observations: i64,
    request_bytes: i64,
    request_shape_observations: i64,
    input_item_count: i64,
    tool_count: i64,
    system_block_count: i64,
    previous_response_requests: i64,
    compatibility_fixes: i64,
    streaming_requests: i64,
    cache_keepalive_requests: i64,
    cache_keepalive_hits: i64,
    cache_keepalive_misses: i64,
    cache_keepalive_partials: i64,
    cache_keepalive_retryable_errors: i64,
    cache_keepalive_terminal_errors: i64,
    cache_keepalive_preemptions: i64,
    cache_keepalive_usage_observations: i64,
    cache_keepalive_input_tokens: i64,
    cache_keepalive_output_tokens: i64,
    cache_keepalive_reasoning_tokens: i64,
    cache_keepalive_hit_tokens: i64,
    cache_keepalive_miss_tokens: i64,
    cache_keepalive_duration_ms: i64,
    cache_keepalive_duration_observations: i64,
    cache_keepalive_interval_ms: i64,
    cache_keepalive_interval_observations: i64,
    cache_keepalive_source_age_ms: i64,
    cache_keepalive_source_age_observations: i64,
}

impl UsageAggregate {
    fn from_row(row: &sqlx::postgres::PgRow) -> Self {
        Self {
            requests: row.get("requests"),
            errors: row.get("errors"),
            blocking_errors: row.get("blocking_errors"),
            transient_errors: row.get("transient_errors"),
            completed_requests: row.get("completed_requests"),
            completion_observations: row.get("completion_observations"),
            usage_observations: row.get("usage_observations"),
            input_tokens: row.get("input_tokens"),
            output_tokens: row.get("output_tokens"),
            reasoning_tokens: row.get("reasoning_tokens"),
            cache_hit_tokens: row.get("cache_hit_tokens"),
            cache_miss_tokens: row.get("cache_miss_tokens"),
            cache_observations: row.get("cache_observations"),
            explicit_cache_observations: row.get("explicit_cache_observations"),
            derived_cache_observations: row.get("derived_cache_observations"),
            absent_cache_observations: row.get("absent_cache_observations"),
            cold_cache_requests: row.get("cold_cache_requests"),
            hot_cache_requests: row.get("hot_cache_requests"),
            duration_ms: row.get("duration_ms"),
            duration_observations: row.get("duration_observations"),
            request_bytes: row.get("request_bytes"),
            request_shape_observations: row.get("request_shape_observations"),
            input_item_count: row.get("input_item_count"),
            tool_count: row.get("tool_count"),
            system_block_count: row.get("system_block_count"),
            previous_response_requests: row.get("previous_response_requests"),
            compatibility_fixes: row.get("compatibility_fixes"),
            streaming_requests: row.get("streaming_requests"),
            cache_keepalive_requests: row.get("cache_keepalive_requests"),
            cache_keepalive_hits: row.get("cache_keepalive_hits"),
            cache_keepalive_misses: row.get("cache_keepalive_misses"),
            cache_keepalive_partials: row.get("cache_keepalive_partials"),
            cache_keepalive_retryable_errors: row.get("cache_keepalive_retryable_errors"),
            cache_keepalive_terminal_errors: row.get("cache_keepalive_terminal_errors"),
            cache_keepalive_preemptions: row.get("cache_keepalive_preemptions"),
            cache_keepalive_usage_observations: row.get("cache_keepalive_usage_observations"),
            cache_keepalive_input_tokens: row.get("cache_keepalive_input_tokens"),
            cache_keepalive_output_tokens: row.get("cache_keepalive_output_tokens"),
            cache_keepalive_reasoning_tokens: row.get("cache_keepalive_reasoning_tokens"),
            cache_keepalive_hit_tokens: row.get("cache_keepalive_hit_tokens"),
            cache_keepalive_miss_tokens: row.get("cache_keepalive_miss_tokens"),
            cache_keepalive_duration_ms: row.get("cache_keepalive_duration_ms"),
            cache_keepalive_duration_observations: row.get("cache_keepalive_duration_observations"),
            cache_keepalive_interval_ms: row.get("cache_keepalive_interval_ms"),
            cache_keepalive_interval_observations: row.get("cache_keepalive_interval_observations"),
            cache_keepalive_source_age_ms: row.get("cache_keepalive_source_age_ms"),
            cache_keepalive_source_age_observations: row
                .get("cache_keepalive_source_age_observations"),
        }
    }

    fn add(&mut self, other: &Self) {
        self.requests = self.requests.saturating_add(other.requests);
        self.errors = self.errors.saturating_add(other.errors);
        self.blocking_errors = self.blocking_errors.saturating_add(other.blocking_errors);
        self.transient_errors = self.transient_errors.saturating_add(other.transient_errors);
        self.completed_requests = self
            .completed_requests
            .saturating_add(other.completed_requests);
        self.completion_observations = self
            .completion_observations
            .saturating_add(other.completion_observations);
        self.usage_observations = self
            .usage_observations
            .saturating_add(other.usage_observations);
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(other.reasoning_tokens);
        self.cache_hit_tokens = self.cache_hit_tokens.saturating_add(other.cache_hit_tokens);
        self.cache_miss_tokens = self
            .cache_miss_tokens
            .saturating_add(other.cache_miss_tokens);
        self.cache_observations = self
            .cache_observations
            .saturating_add(other.cache_observations);
        self.explicit_cache_observations = self
            .explicit_cache_observations
            .saturating_add(other.explicit_cache_observations);
        self.derived_cache_observations = self
            .derived_cache_observations
            .saturating_add(other.derived_cache_observations);
        self.absent_cache_observations = self
            .absent_cache_observations
            .saturating_add(other.absent_cache_observations);
        self.cold_cache_requests = self
            .cold_cache_requests
            .saturating_add(other.cold_cache_requests);
        self.hot_cache_requests = self
            .hot_cache_requests
            .saturating_add(other.hot_cache_requests);
        self.duration_ms = self.duration_ms.saturating_add(other.duration_ms);
        self.duration_observations = self
            .duration_observations
            .saturating_add(other.duration_observations);
        self.request_bytes = self.request_bytes.saturating_add(other.request_bytes);
        self.request_shape_observations = self
            .request_shape_observations
            .saturating_add(other.request_shape_observations);
        self.input_item_count = self.input_item_count.saturating_add(other.input_item_count);
        self.tool_count = self.tool_count.saturating_add(other.tool_count);
        self.system_block_count = self
            .system_block_count
            .saturating_add(other.system_block_count);
        self.previous_response_requests = self
            .previous_response_requests
            .saturating_add(other.previous_response_requests);
        self.compatibility_fixes = self
            .compatibility_fixes
            .saturating_add(other.compatibility_fixes);
        self.streaming_requests = self
            .streaming_requests
            .saturating_add(other.streaming_requests);
        self.add_cache_keepalive(other);
    }

    fn add_cache_keepalive(&mut self, other: &Self) {
        self.cache_keepalive_requests = self
            .cache_keepalive_requests
            .saturating_add(other.cache_keepalive_requests);
        self.cache_keepalive_hits = self
            .cache_keepalive_hits
            .saturating_add(other.cache_keepalive_hits);
        self.cache_keepalive_misses = self
            .cache_keepalive_misses
            .saturating_add(other.cache_keepalive_misses);
        self.cache_keepalive_partials = self
            .cache_keepalive_partials
            .saturating_add(other.cache_keepalive_partials);
        self.cache_keepalive_retryable_errors = self
            .cache_keepalive_retryable_errors
            .saturating_add(other.cache_keepalive_retryable_errors);
        self.cache_keepalive_terminal_errors = self
            .cache_keepalive_terminal_errors
            .saturating_add(other.cache_keepalive_terminal_errors);
        self.cache_keepalive_preemptions = self
            .cache_keepalive_preemptions
            .saturating_add(other.cache_keepalive_preemptions);
        self.cache_keepalive_usage_observations = self
            .cache_keepalive_usage_observations
            .saturating_add(other.cache_keepalive_usage_observations);
        self.cache_keepalive_input_tokens = self
            .cache_keepalive_input_tokens
            .saturating_add(other.cache_keepalive_input_tokens);
        self.cache_keepalive_output_tokens = self
            .cache_keepalive_output_tokens
            .saturating_add(other.cache_keepalive_output_tokens);
        self.cache_keepalive_reasoning_tokens = self
            .cache_keepalive_reasoning_tokens
            .saturating_add(other.cache_keepalive_reasoning_tokens);
        self.cache_keepalive_hit_tokens = self
            .cache_keepalive_hit_tokens
            .saturating_add(other.cache_keepalive_hit_tokens);
        self.cache_keepalive_miss_tokens = self
            .cache_keepalive_miss_tokens
            .saturating_add(other.cache_keepalive_miss_tokens);
        self.cache_keepalive_duration_ms = self
            .cache_keepalive_duration_ms
            .saturating_add(other.cache_keepalive_duration_ms);
        self.cache_keepalive_duration_observations = self
            .cache_keepalive_duration_observations
            .saturating_add(other.cache_keepalive_duration_observations);
        self.cache_keepalive_interval_ms = self
            .cache_keepalive_interval_ms
            .saturating_add(other.cache_keepalive_interval_ms);
        self.cache_keepalive_interval_observations = self
            .cache_keepalive_interval_observations
            .saturating_add(other.cache_keepalive_interval_observations);
        self.cache_keepalive_source_age_ms = self
            .cache_keepalive_source_age_ms
            .saturating_add(other.cache_keepalive_source_age_ms);
        self.cache_keepalive_source_age_observations = self
            .cache_keepalive_source_age_observations
            .saturating_add(other.cache_keepalive_source_age_observations);
    }
}

// --- row types ---------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct SessionRow {
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
    system: bool,
    next_seq: i64,
    queue: serde_json::Value,
    drafts: serde_json::Value,
    config_options: Option<serde_json::Value>,
    config_preferences: serde_json::Value,
    mobile_review_state: serde_json::Value,
    #[allow(dead_code)]
    created_at: DateTime<Utc>,
}

impl SessionRow {
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
            // Restored from the DB (migration 0010) — a system session stays
            // view-only across a daemon restart.
            system: self.system,
            // Transient — the manual pause is in-memory only (not persisted), so a
            // restored session always comes back un-paused.
            paused: false,
            // Not persisted — a fresh usage_update re-seeds it right after revive.
            context_used: 0,
            context_size: 0,
            usage: None,
            // Derived from restored drafts in `session_list`, not stored here.
            next_schedule_ms: None,
        }
    }
}

#[derive(sqlx::FromRow)]
struct EventRow {
    seq: i64,
    payload: serde_json::Value,
    total_count: i64,
}

#[cfg(test)]
mod storage_contract_tests {
    use super::*;

    fn session(id: &str) -> SessionMeta {
        SessionMeta {
            id: id.to_owned(),
            provider: "codex".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "hawk".to_owned(),
            workspace_id: Some("cowboy".to_owned()),
            workspace_name: Some("Cowboy".to_owned()),
            workspace_source_path: Some("/tmp/cowboy".to_owned()),
            cwd: "/tmp/cowboy-worktree".to_owned(),
            title: "Storage contract".to_owned(),
            status: Status::Starting,
            origin: SessionOrigin::Web,
            agent_session_id: None,
            paused: false,
            system: false,
            context_used: 0,
            context_size: 0,
            usage: None,
            next_schedule_ms: None,
        }
    }

    async fn assert_machine_contract(store: &Store) -> Result<()> {
        assert!(store.machine_is_local("hawk").await?);
        let token = store
            .create_machine_enrollment("contract-machine", "Contract Machine", 60)
            .await?;
        let enrolled = store
            .consume_machine_enrollment(
                &token,
                "ssh-ed25519 QUJD",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            )
            .await?;
        assert_eq!(enrolled.id, "contract-machine");
        assert_eq!(enrolled.display_name, "Contract Machine");
        assert!(!enrolled.fingerprint.is_empty());
        assert_eq!(
            store
                .machine_public_key("contract-machine")
                .await?
                .as_deref(),
            Some("ssh-ed25519 QUJD")
        );
        assert_eq!(
            store
                .machine_encryption_public_key("contract-machine")
                .await?
                .as_deref(),
            Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
        );

        store
            .machine_connected(
                "contract-machine",
                "contract-epoch",
                "linux",
                "x86_64",
                "outbound_wss",
                &serde_json::json!([{"component": "worker", "version": "test"}]),
            )
            .await?;
        assert!(
            store
                .machine_connection_is_current("contract-machine", "contract-epoch")
                .await?
        );
        store
            .machine_seen(
                "contract-machine",
                "contract-epoch",
                Some(&serde_json::json!([{"component": "worker", "healthy": true}])),
            )
            .await?;
        let machines = store.list_machines().await?;
        let remote = machines
            .iter()
            .find(|machine| machine.id == "contract-machine")
            .context("enrolled Machine was not listed")?;
        assert_eq!(remote.status, "online");
        assert_eq!(remote.inventory[0]["healthy"], true);
        assert!(remote.last_seen_at_ms.is_some());

        store
            .machine_disconnected("contract-machine", "contract-epoch", 0)
            .await?;
        assert!(store.expire_machine_reconnects().await? >= 1);
        store.revoke_machine("contract-machine").await?;
        assert!(
            store
                .machine_public_key("contract-machine")
                .await?
                .is_none()
        );
        assert!(
            store
                .list_machines()
                .await?
                .iter()
                .any(|machine| machine.id == "contract-machine" && machine.revoked)
        );
        Ok(())
    }

    async fn assert_provider_action_contract(store: &Store) -> Result<()> {
        let fire_at_ms = 1_900_000_000_000;
        store
            .upsert_provider_reset("codex", fire_at_ms, "storage-contract-reset")
            .await?;
        let scheduled = store
            .load_provider_reset("codex")
            .await?
            .context("scheduled provider action was not restored")?;
        assert_eq!(scheduled.fire_at_ms, fire_at_ms);
        assert_eq!(scheduled.idempotency_key, "storage-contract-reset");
        assert_eq!(scheduled.attempt_count, 0);
        assert_eq!(scheduled.next_attempt_at_ms, fire_at_ms);

        store
            .defer_provider_reset("codex", "storage-contract-reset", fire_at_ms + 1_000)
            .await?;
        let deferred = store
            .load_provider_reset("codex")
            .await?
            .context("deferred provider action was not restored")?;
        assert_eq!(deferred.attempt_count, 1);
        assert_eq!(deferred.next_attempt_at_ms, fire_at_ms + 1_000);
        store
            .append_provider_action_log(
                "codex",
                "scheduled",
                "succeeded",
                "contract",
                "storage contract action",
                Some("credit-test"),
                Some("storage-contract-reset"),
                fire_at_ms,
            )
            .await?;
        let action_logs = store.provider_action_logs(10).await?;
        let action = action_logs
            .first()
            .context("provider action log was not restored")?;
        assert_eq!(action.status, "succeeded");
        assert_eq!(action.idempotency_suffix.as_deref(), Some("ct-reset"));
        let diagnostic_id = format!("automation:{}", action.id);
        assert!(store.diagnostic_log_detail(&diagnostic_id).await?.is_some());
        store.delete_provider_reset("codex").await?;
        assert!(store.load_provider_reset("codex").await?.is_none());

        store
            .upsert_provider_reset("xai", fire_at_ms + 2_000, "storage-contract-xai-reset")
            .await?;
        let xai = store
            .load_provider_reset("xai")
            .await?
            .context("scheduled xAI provider action was not restored")?;
        assert_eq!(xai.fire_at_ms, fire_at_ms + 2_000);
        assert!(
            !store
                .claim_provider_reset("xai", "stale-storage-contract-key")
                .await?
        );
        store
            .append_provider_action_log(
                "xai",
                "scheduled",
                "scheduled",
                "contract",
                "xAI storage contract action",
                None,
                Some("storage-contract-xai-reset"),
                fire_at_ms + 2_000,
            )
            .await?;
        let xai_log = store
            .provider_action_logs(10)
            .await?
            .into_iter()
            .find(|log| log.provider == "xai")
            .context("xAI provider action log was not restored")?;
        assert_eq!(xai_log.status, "scheduled");
        assert!(
            store
                .claim_provider_reset("xai", "storage-contract-xai-reset")
                .await?
        );
        assert!(store.load_provider_reset("xai").await?.is_none());
        Ok(())
    }

    async fn assert_cache_lineage_contract(store: &Store) -> Result<()> {
        let occurred_at_ms = chrono::Utc::now().timestamp_millis();
        let mut hot = super::provider_usage_validation_tests::event();
        hot.sequence = 10;
        hot.occurred_at_ms = occurred_at_ms;
        hot.input_tokens = Some(10_000);
        hot.cache_hit_tokens = Some(9_000);
        hot.cache_miss_tokens = Some(1_000);

        // This healthy but tiny request is outside the diagnostic lineage. It
        // must not hide the prior eligible hot observation from the next miss.
        let mut ineligible = super::provider_usage_validation_tests::event();
        ineligible.sequence = 11;
        ineligible.occurred_at_ms = occurred_at_ms + 1;

        let mut cold = hot.clone();
        cold.sequence = 12;
        cold.occurred_at_ms = occurred_at_ms + 2;
        cold.cache_hit_tokens = Some(0);
        cold.cache_miss_tokens = Some(10_000);
        assert_eq!(
            store
                .ingest_provider_usage("hawk", "codex-deepseek", &[hot, ineligible, cold],)
                .await?,
            12
        );
        let logs = store
            .diagnostic_logs(&DiagnosticLogFilter {
                since_ms: occurred_at_ms.saturating_sub(1_000),
                until_ms: occurred_at_ms.saturating_add(1_000),
                kinds: vec!["cache_anomaly".to_owned()],
                severities: Vec::new(),
                states: Vec::new(),
                agents: Vec::new(),
                session_ref: None,
                cursor_ms: None,
                cursor_id: None,
                limit: 20,
            })
            .await?;
        let anomaly = logs
            .iter()
            .find(|log| log.id.ends_with(":12"))
            .context("eligible cache transition was not listed")?;
        assert_eq!(
            anomaly.classification.as_deref(),
            Some("unexpected_exact_prefix_miss")
        );
        assert!(store.diagnostic_log_detail(&anomaly.id).await?.is_some());
        Ok(())
    }

    #[allow(clippy::too_many_lines)] // one shared end-to-end contract for every storage backend
    async fn run_storage_contract(store: &Store, session_id: &str) -> Result<()> {
        store.migrate().await?;
        assert_machine_contract(store).await?;
        assert_provider_action_contract(store).await?;
        store.insert_session(&session(session_id)).await?;
        let companion_id = format!("{session_id}-artifact");
        store.insert_session(&session(&companion_id)).await?;
        let numeric_id = session_id
            .strip_prefix("sess-")
            .context("storage contract needs a numeric session id")?
            .parse::<u64>()?;
        assert_eq!(store.next_session_number().await?, numeric_id + 1);
        store.update_status(session_id, Status::Running).await?;
        store
            .update_agent_session_id(session_id, Some("agent-thread"))
            .await?;
        store
            .update_config_options(session_id, &serde_json::json!({"model": "gpt-test"}))
            .await?;
        store
            .update_config_preferences(session_id, &serde_json::json!({"model": "gpt-test"}))
            .await?;
        store
            .update_title(session_id, "Storage contract renamed")
            .await?;
        store
            .update_cwd(
                session_id,
                "/tmp/cowboy-retargeted",
                Some("Storage contract retargeted"),
            )
            .await?;
        store
            .update_mobile_review_state(
                session_id,
                &serde_json::json!({"mode": "code", "tabs": ["src/store.rs"]}),
            )
            .await?;
        store
            .update_pending(
                session_id,
                &[QueuedMessage {
                    id: "queue-1".to_owned(),
                    text: "queued contract prompt".to_owned(),
                    content: Vec::new(),
                    cmid: Some("contract-cmid".to_owned()),
                    schedule: None,
                }],
                &[QueuedMessage {
                    id: "draft-1".to_owned(),
                    text: "draft contract prompt".to_owned(),
                    content: Vec::new(),
                    cmid: None,
                    schedule: None,
                }],
            )
            .await?;
        store
            .upsert_wakeup(session_id, 1_900_000_000_000, "continue")
            .await?;
        let events = vec![
            Envelope {
                session_id: session_id.to_owned(),
                seq: 0,
                event: Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "user_message_chunk",
                        "content": {"text": "verify both storage backends"},
                    }),
                },
                cmid: None,
            },
            Envelope {
                session_id: session_id.to_owned(),
                seq: 1,
                event: Event::TurnEnd {
                    stop_reason: "end_turn".to_owned(),
                },
                cmid: None,
            },
        ];
        store
            .upsert_event_batch(&events, &HashMap::from([(session_id.to_owned(), 2)]))
            .await?;

        let artifact_bytes = vec![7_u8; 40_000];
        let artifact_name = format!("{}.png", hex_sha256(&artifact_bytes));
        let artifact_event = Envelope {
            session_id: companion_id.clone(),
            seq: 0,
            event: Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {
                        "type": "image",
                        "data": base64::engine::general_purpose::STANDARD.encode(artifact_bytes),
                        "mimeType": "image/png",
                    },
                }),
            },
            cmid: None,
        };
        store
            .upsert_event_batch(
                &[artifact_event],
                &HashMap::from([(companion_id.clone(), 1)]),
            )
            .await?;
        assert!(store.artifact_path(&artifact_name).is_some());
        store
            .update_session_order(&[companion_id.clone(), session_id.to_owned()])
            .await?;

        let loaded = store.load_all().await?;
        assert_eq!(
            loaded.first().map(|loaded| loaded.meta.id.as_str()),
            Some(companion_id.as_str())
        );
        let restored = loaded
            .iter()
            .find(|loaded| loaded.meta.id == session_id)
            .context("storage contract session was not restored")?;
        assert_eq!(restored.meta.status, Status::Running);
        assert_eq!(
            restored.meta.agent_session_id.as_deref(),
            Some("agent-thread")
        );
        assert_eq!(restored.meta.title, "Storage contract retargeted");
        assert_eq!(restored.meta.cwd, "/tmp/cowboy-retargeted");
        assert_eq!(restored.events.len(), 2);
        assert_eq!(restored.next_seq, 2);
        assert_eq!(restored.queue[0].id, "queue-1");
        assert_eq!(restored.drafts[0].id, "draft-1");
        assert_eq!(
            restored.config_options.as_ref().unwrap()["model"],
            "gpt-test"
        );
        assert_eq!(restored.config_preferences["model"], "gpt-test");
        assert_eq!(restored.mobile_review_state["mode"], "code");

        let (history, _, reached_start) = store.history_page(session_id, 2, 200).await?;
        assert_eq!(history.len(), 2);
        assert!(reached_start);
        let (questions, _, total) = store.question_page_summaries(session_id, None, 20).await?;
        assert_eq!(total, 1);
        assert_eq!(questions.len(), 1);
        assert!(questions[0].title.contains("verify both storage backends"));
        let page = store
            .question_page_at(session_id, 0)
            .await?
            .context("question page missing")?;
        assert_eq!(page.len(), 2);
        let (previous_page, previous_cursor, previous_reached_start) =
            store.question_page_before(session_id, 2).await?;
        assert_eq!(previous_page.len(), 2);
        assert!(previous_cursor.is_none());
        assert!(previous_reached_start);

        assert!(
            store
                .load_wakeups()
                .await?
                .iter()
                .any(|(id, _, prompt)| id == session_id && prompt == "continue")
        );
        store.delete_wakeup(session_id).await?;
        assert!(
            store
                .load_wakeups()
                .await?
                .iter()
                .all(|(id, _, _)| id != session_id)
        );

        let incident_id = format!("storage-contract:{session_id}");
        store
            .upsert_runtime_incident(&RuntimeIncidentWrite {
                id: incident_id.clone(),
                occurred_at_ms: 1_900_000_000_000,
                source: "controller".to_owned(),
                classification: "storage_contract".to_owned(),
                severity: "warning".to_owned(),
                state: "active".to_owned(),
                summary: "storage contract incident".to_owned(),
                fingerprint: "contract".to_owned(),
                session_id: Some(session_id.to_owned()),
                client_id: None,
                machine_id: None,
                trace_id: None,
                build: Some("test".to_owned()),
                evidence_start_ms: 1_899_999_999_000,
                evidence_end_ms: 1_900_000_001_000,
                detail: serde_json::json!({"contract": true}),
            })
            .await?;
        store
            .upsert_runtime_incident(&RuntimeIncidentWrite {
                id: incident_id.clone(),
                occurred_at_ms: 1_900_000_000_000,
                source: "controller".to_owned(),
                classification: "storage_contract".to_owned(),
                severity: "warning".to_owned(),
                state: "active".to_owned(),
                summary: "storage contract incident".to_owned(),
                fingerprint: "contract".to_owned(),
                session_id: Some(session_id.to_owned()),
                client_id: None,
                machine_id: None,
                trace_id: None,
                build: Some("test".to_owned()),
                evidence_start_ms: 1_899_999_999_000,
                evidence_end_ms: 1_900_000_002_000,
                detail: serde_json::json!({"contract": null, "second": true}),
            })
            .await?;
        let incident = store
            .runtime_incidents(20)
            .await?
            .into_iter()
            .find(|incident| incident.id == incident_id)
            .context("runtime incident was not restored")?;
        assert_eq!(
            incident.detail.get("contract"),
            Some(&serde_json::Value::Null)
        );
        assert_eq!(incident.detail["second"], true);
        assert_eq!(incident.evidence_end_ms, 1_900_000_002_000);
        let logs = store
            .diagnostic_logs(&DiagnosticLogFilter {
                since_ms: 1_899_999_000_000,
                until_ms: 1_900_001_000_000,
                kinds: Vec::new(),
                severities: Vec::new(),
                states: Vec::new(),
                agents: Vec::new(),
                session_ref: Some(session_id.to_owned()),
                cursor_ms: None,
                cursor_id: None,
                limit: 20,
            })
            .await?;
        let log_id = format!("runtime:{incident_id}");
        assert!(logs.iter().any(|log| log.id == log_id));
        assert!(store.diagnostic_log_detail(&log_id).await?.is_some());
        assert_eq!(
            store
                .recover_runtime_incident(session_id, 1_900_000_002_000, "contract complete")
                .await?,
            1
        );
        let recovered = store
            .runtime_incidents(20)
            .await?
            .into_iter()
            .find(|incident| incident.id == incident_id)
            .context("recovered incident was not restored")?;
        assert_eq!(recovered.state, "recovered");
        assert_eq!(
            recovered.recovery_outcome.as_deref(),
            Some("contract complete")
        );

        let mut usage = super::provider_usage_validation_tests::event();
        usage.sequence = 1;
        usage.occurred_at_ms = chrono::Utc::now().timestamp_millis();
        store
            .ingest_provider_usage("hawk", "codex-deepseek", &[usage.clone()])
            .await?;
        // Replaying the same producer sequence is idempotent, and the producer
        // watermark remains monotonic.
        assert_eq!(
            store
                .ingest_provider_usage("hawk", "codex-deepseek", &[usage.clone()])
                .await?,
            1
        );
        let mut failed_usage = super::provider_usage_validation_tests::event();
        failed_usage.sequence = 2;
        failed_usage.occurred_at_ms = usage.occurred_at_ms;
        failed_usage.status = 401;
        failed_usage.completed = Some(false);
        assert_eq!(
            store
                .ingest_provider_usage("hawk", "codex-deepseek", &[failed_usage])
                .await?,
            2
        );
        let usage_summary = store.provider_usage_summary("deepseek", 1, 30).await?;
        assert_eq!(usage_summary["summary"]["requests"], 2);
        assert_eq!(usage_summary["summary"]["errors"], 1);
        assert_eq!(usage_summary["summary"]["inputTokens"], 20);
        let to_ms = chrono::Utc::now().timestamp_millis().saturating_add(1_000);
        let usage_activity = store
            .provider_usage_activity(
                "deepseek",
                to_ms.saturating_sub(60_000),
                to_ms,
                &["codex".to_owned()],
                &["flash".to_owned()],
            )
            .await?;
        assert_eq!(usage_activity["summary"]["requests"], 2);
        let usage_logs = store
            .diagnostic_logs(&DiagnosticLogFilter {
                since_ms: to_ms.saturating_sub(60_000),
                until_ms: to_ms,
                kinds: vec!["provider_error".to_owned()],
                severities: Vec::new(),
                states: Vec::new(),
                agents: Vec::new(),
                session_ref: None,
                cursor_ms: None,
                cursor_id: None,
                limit: 20,
            })
            .await?;
        let usage_log = usage_logs
            .iter()
            .find(|log| log.kind == "provider_error")
            .context("provider error diagnostic was not listed")?;
        assert!(store.diagnostic_log_detail(&usage_log.id).await?.is_some());
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert!(store.purge_provider_usage(0).await? >= 2);
        assert_cache_lineage_contract(store).await?;
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert!(store.purge_provider_usage(0).await? >= 3);

        let (_, event_count, _) = store.storage_metrics().await?;
        assert!(event_count >= 2);
        store.clear_events(&companion_id).await?;
        assert!(
            store
                .history_page(&companion_id, 1, 200)
                .await?
                .0
                .is_empty()
        );
        store.delete_session(session_id).await?;
        store.delete_session(&companion_id).await?;
        let (_, _, deleted_count) = store.storage_metrics().await?;
        assert!(deleted_count >= 2);
        assert!(
            store
                .load_all()
                .await?
                .iter()
                .all(|loaded| loaded.meta.id != session_id)
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert!(store.purge_deleted(0).await? >= 2);
        Ok(())
    }

    async fn assert_restore_hot_tail_is_byte_bounded(
        store: &Store,
        session_id: &str,
    ) -> Result<()> {
        store.migrate().await?;
        store.insert_session(&session(session_id)).await?;
        let payload = "x".repeat(384 * 1024);
        let events: Vec<_> = (0..5)
            .map(|seq| Envelope {
                session_id: session_id.to_owned(),
                seq,
                event: Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "tool_call_update",
                        "payload": payload,
                    }),
                },
                cmid: None,
            })
            .collect();
        store
            .upsert_event_batch(&events, &HashMap::from([(session_id.to_owned(), 5)]))
            .await?;

        let loaded = store
            .load_all()
            .await?
            .into_iter()
            .find(|loaded| loaded.meta.id == session_id)
            .context("byte-bounded restore session missing")?;
        assert_eq!(loaded.event_count, 5);
        assert!(!loaded.reached_start);
        assert_eq!(loaded.events.last().map(|event| event.seq), Some(4));
        assert!(loaded.events.len() < events.len());
        let restored_bytes = loaded.events.iter().fold(0usize, |size, event| {
            size.saturating_add(crate::core::estimated_envelope_bytes(event))
        });
        assert!(restored_bytes <= crate::core::HOT_TAIL_MAX_BYTES);
        Ok(())
    }

    #[tokio::test]
    async fn sqlite_implements_storage_contract() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-sqlite-contract-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let url = format!("sqlite://{}", root.join("cowboy.sqlite3").display());
        let store = Store::connect(&url, root.join("artifacts")).await.unwrap();
        run_storage_contract(&store, "sess-900000001")
            .await
            .unwrap();
        assert_restore_hot_tail_is_byte_bounded(&store, "sess-900000011")
            .await
            .unwrap();
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn sqlite_memory_implements_storage_contract() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-sqlite-memory-contract-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::connect("sqlite::memory:", root.join("artifacts"))
            .await
            .unwrap();
        run_storage_contract(&store, "sess-900000003")
            .await
            .unwrap();
        assert_restore_hot_tail_is_byte_bounded(&store, "sess-900000013")
            .await
            .unwrap();
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    #[ignore = "set COWBOY_TEST_POSTGRES_URL to an isolated empty database"]
    async fn postgres_implements_storage_contract() {
        let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
            .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
        let root =
            std::env::temp_dir().join(format!("cowboy-postgres-contract-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::connect(&url, root.join("artifacts")).await.unwrap();
        run_storage_contract(&store, "sess-900000002")
            .await
            .unwrap();
        assert_restore_hot_tail_is_byte_bounded(&store, "sess-900000012")
            .await
            .unwrap();
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
