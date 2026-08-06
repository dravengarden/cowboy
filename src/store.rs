//! Postgres-backed persistence for cowboy state.
//!
//! [`Store`] is a thin wrapper around an `sqlx::PgPool` exposing the small
//! surface the [`Hub`] (and one background writer task in
//! [`crate::server`]) needs:
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
//! Embedded migrations live next to `Cargo.toml` in `./migrations/`. Run
//! [`Store::migrate`] once on startup.

#![warn(clippy::pedantic)]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Read as _;
use std::time::Duration;

use anyhow::{Context as _, Result};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use sha2::Digest as _;
use sqlx::Row as _;
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::core::{
    Envelope, Event, JudgeRun, QuestionPageSummary, QueuedMessage, SessionMeta, SessionOrigin,
    Status, bound_history_page, question_summary_title,
};

fn valid_machine_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
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
/// writer then drops the whole intent (the event / queue / judge-run is lost,
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
    /// Persisted confirm-detect judge-run history (newest first), capped.
    pub judge_runs: Vec<JudgeRun>,
    /// Mobile-only code-review workspace state, shared across iPhone/iPad clients.
    pub mobile_review_state: serde_json::Value,
}

#[derive(Clone)]
pub struct Store {
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

impl Store {
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
    ) -> Result<EnrolledMachine> {
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
             (id, display_name, connection_mode, platform, architecture, status, public_key, enrolled_at) \
             VALUES ($1, $2, 'outbound_wss', 'unknown', 'unknown', 'offline', $3, now()) \
             ON CONFLICT (id) DO UPDATE SET display_name = EXCLUDED.display_name, \
             connection_mode = 'outbound_wss', public_key = EXCLUDED.public_key, \
             enrolled_at = now(), revoked_at = NULL, status = 'offline', \
             connection_epoch = NULL, reconnect_deadline_at = NULL, updated_at = now() \
             WHERE machines.public_key IS NULL OR \
             (machines.revoked_at IS NOT NULL AND machines.public_key IS DISTINCT FROM EXCLUDED.public_key)",
        )
        .bind(&id)
        .bind(&display_name)
        .bind(public_key)
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

    pub async fn upsert_codex_reset(&self, fire_at_ms: i64, idempotency_key: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO scheduled_provider_actions (provider, action, fire_at_ms, idempotency_key) \
             VALUES ('codex', 'rate_limit_reset', $1, $2) \
             ON CONFLICT (provider) DO UPDATE SET action = EXCLUDED.action, \
             fire_at_ms = EXCLUDED.fire_at_ms, idempotency_key = EXCLUDED.idempotency_key, \
             attempt_count = 0, next_attempt_at_ms = EXCLUDED.fire_at_ms",
        )
        .bind(fire_at_ms)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await
        .context("UPSERT scheduled Codex reset")?;
        Ok(())
    }

    pub async fn load_codex_reset(&self) -> Result<Option<ScheduledProviderAction>> {
        let row: Option<(i64, String, i32, Option<i64>)> = sqlx::query_as(
            "SELECT fire_at_ms, idempotency_key, attempt_count, next_attempt_at_ms FROM scheduled_provider_actions \
             WHERE provider = 'codex' AND action = 'rate_limit_reset'",
        )
        .fetch_optional(&self.pool)
        .await
        .context("SELECT scheduled Codex reset")?;
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

    pub async fn defer_codex_reset(&self, next_attempt_at_ms: i64) -> Result<()> {
        sqlx::query(
            "UPDATE scheduled_provider_actions SET attempt_count = attempt_count + 1, \
             next_attempt_at_ms = $1 WHERE provider = 'codex' AND action = 'rate_limit_reset'",
        )
        .bind(next_attempt_at_ms)
        .execute(&self.pool)
        .await
        .context("defer scheduled Codex reset")?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn append_provider_action_log(
        &self,
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
             VALUES ('codex', 'rate_limit_reset', $1, $2, $3, $4, $5, $6, $7)",
        )
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

    pub async fn delete_codex_reset(&self) -> Result<()> {
        sqlx::query("DELETE FROM scheduled_provider_actions WHERE provider = 'codex'")
            .execute(&self.pool)
            .await
            .context("DELETE scheduled Codex reset")?;
        Ok(())
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
            "SELECT id, provider, machine_id, workspace_id, workspace_name, workspace_source_path, \
             cwd, title, origin, status, agent_session_id, auto_resume, \
             awaiting_user, done, system, next_seq, queue, drafts, judge_runs, \
             mobile_review_state, created_at \
             FROM sessions WHERE deleted_at IS NULL ORDER BY position ASC NULLS LAST, created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .context("SELECT sessions")?;

        let mut out = Vec::with_capacity(session_rows.len());
        for row in session_rows {
            let id = row.id.clone();
            let event_rows: Vec<EventRow> = sqlx::query_as::<_, EventRow>(
                "SELECT seq, payload, count(*) OVER() AS total_count \
                 FROM events WHERE session_id = $1 ORDER BY seq DESC LIMIT $2",
            )
            .bind(&id)
            .bind(i64::try_from(crate::core::HOT_TAIL).unwrap_or(i64::MAX))
            .fetch_all(&self.pool)
            .await
            .with_context(|| format!("SELECT events for {id}"))?;

            let event_count = event_rows
                .first()
                .and_then(|r| u64::try_from(r.total_count).ok())
                .unwrap_or(0);
            let reached_start = event_count <= u64::try_from(event_rows.len()).unwrap_or(u64::MAX);
            let mut events = Vec::with_capacity(event_rows.len());
            for er in event_rows.into_iter().rev() {
                let seq_for_log = er.seq;
                // Degrade a single undecodable event to a SKIP (with a warn), not a
                // hard error: one corrupt/legacy row must not fail the whole
                // `load_all` and so block the daemon from starting at all (that
                // bricks EVERY session — a blank UI for the user). Same
                // "tolerate one bad row" philosophy as queue/drafts/judge_runs
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
            let next_seq = u64::try_from(row.next_seq).unwrap_or(0);
            // Tolerate a malformed/legacy payload by degrading to empty rather
            // than failing the whole restore for one bad row.
            let queue: Vec<QueuedMessage> =
                serde_json::from_value(row.queue.clone()).unwrap_or_default();
            let drafts: Vec<QueuedMessage> =
                serde_json::from_value(row.drafts.clone()).unwrap_or_default();
            let judge_runs: Vec<JudgeRun> =
                serde_json::from_value(row.judge_runs.clone()).unwrap_or_default();
            let mobile_review_state = row.mobile_review_state.clone();
            out.push(LoadedSession {
                meta: row.into_meta(),
                events,
                event_count,
                reached_start,
                next_seq,
                queue,
                drafts,
                judge_runs,
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
            "INSERT INTO sessions(id, provider, machine_id, workspace_id, workspace_name, \
             workspace_source_path, cwd, title, origin, status, next_seq, system) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 0, $11)",
        )
        .bind(&m.id)
        .bind(&m.provider)
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

    /// Persist the confirm-detect turn-end verdict so a finished/awaiting session
    /// survives a daemon restart (migration 0008).
    ///
    /// # Errors
    /// If the UPDATE fails.
    pub async fn update_verdict(
        &self,
        session_id: &str,
        awaiting_user: bool,
        done: bool,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET awaiting_user = $1, done = $2, updated_at = now() WHERE id = $3",
        )
        .bind(awaiting_user)
        .bind(done)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPDATE session verdict {session_id}"))?;
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

    /// Persist a session's auto-resume OVERRIDE (`None` = inherit the global
    /// default, `Some(true/false)` = force). Mirrors `update_status` — only this
    /// column + `updated_at` move.
    ///
    /// # Errors
    /// If the UPDATE fails.
    pub async fn update_auto_resume(&self, session_id: &str, value: Option<bool>) -> Result<()> {
        sqlx::query("UPDATE sessions SET auto_resume = $1, updated_at = now() WHERE id = $2")
            .bind(value)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE session auto_resume {session_id}"))?;
        Ok(())
    }

    /// Load the whole global key-value settings table (small; read once on
    /// startup). Returns `(key, value)` pairs.
    ///
    /// # Errors
    /// If the query fails.
    pub async fn load_settings(&self) -> Result<Vec<(String, serde_json::Value)>> {
        let rows: Vec<(String, serde_json::Value)> =
            sqlx::query_as("SELECT key, value FROM settings")
                .fetch_all(&self.pool)
                .await
                .context("SELECT settings")?;
        Ok(rows)
    }

    /// Upsert one global setting (`value` is JSONB).
    ///
    /// # Errors
    /// If the UPSERT fails.
    pub async fn put_setting(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        let mut value = value.clone();
        strip_nul(&mut value);
        sqlx::query(
            "INSERT INTO settings(key, value, updated_at) VALUES ($1, $2, now()) \
             ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()",
        )
        .bind(key)
        .bind(&value)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPSERT setting {key}"))?;
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

    /// Persist a session's confirm-detect judge-run history (the whole list, as
    /// `jsonb`; migration 0009). Whole-list overwrite like [`Self::update_pending`];
    /// the daemon caps the list, so it stays small.
    ///
    /// # Errors
    /// If serializing the list fails or the UPDATE fails.
    pub async fn update_judge_runs(&self, session_id: &str, runs: &[JudgeRun]) -> Result<()> {
        let mut runs_json = serde_json::to_value(runs).context("serialize judge_runs")?;
        strip_nul(&mut runs_json);
        sqlx::query("UPDATE sessions SET judge_runs = $1, updated_at = now() WHERE id = $2")
            .bind(&runs_json)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE session judge_runs {session_id}"))?;
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

    /// Hard-delete sessions soft-deleted more than `retention_days` ago (cascade
    /// → their events). The storage-reclaim half of soft-delete; run on startup
    /// and periodically. Returns the number of sessions purged.
    ///
    /// # Errors
    /// If the DELETE fails.
    pub async fn purge_deleted(&self, retention_days: i64) -> Result<u64> {
        let done = sqlx::query(
            "DELETE FROM sessions WHERE deleted_at IS NOT NULL \
             AND deleted_at < now() - make_interval(days => $1::int)",
        )
        .bind(i32::try_from(retention_days).unwrap_or(3))
        .execute(&self.pool)
        .await
        .context("purge soft-deleted sessions")?;
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

    /// Mark the newest unresolved incident for a session as recovered.
    pub async fn recover_runtime_incident(
        &self,
        session_id: &str,
        recovered_at_ms: i64,
        outcome: &str,
    ) -> Result<u64> {
        let done = sqlx::query(
            "UPDATE runtime_incidents SET state = 'recovered', \
             recovered_at = to_timestamp($2::double precision / 1000), \
             recovery_outcome = $3, updated_at = now() WHERE id = ( \
               SELECT id FROM runtime_incidents WHERE session_id = $1 \
               AND source = 'controller' AND state <> 'recovered' \
               ORDER BY occurred_at DESC LIMIT 1)",
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
        || !matches!(event.schema_version, 1 | 2)
        || !matches!(
            event.operation.as_str(),
            "legacy" | "responses" | "compact" | "messages"
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
    {
        anyhow::bail!("invalid provider usage event");
    }
    if event.schema_version == 2
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
        anyhow::bail!("incomplete version two provider usage event");
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

    fn event() -> ProviderUsageEvent {
        ProviderUsageEvent {
            schema_version: 2,
            producer_id: "codex-deepseek".to_owned(),
            sequence: 1,
            occurred_at_ms: 1_786_000_000_000,
            account_fingerprint: "0123456789abcdef".to_owned(),
            provider: "deepseek".to_owned(),
            agent: "codex".to_owned(),
            model: "deepseek-v4-flash".to_owned(),
            status: 200,
            input_tokens: Some(10),
            output_tokens: Some(4),
            reasoning_tokens: Some(1),
            cache_hit_tokens: Some(7),
            cache_miss_tokens: Some(3),
            operation: "responses".to_owned(),
            protocol: "responses".to_owned(),
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
         account_fingerprint, provider, agent, model, status, input_tokens, output_tokens, \
         reasoning_tokens, cache_hit_tokens, cache_miss_tokens, schema_version, operation, \
         protocol, cache_observation, usage_observed, completed, streaming, duration_ms, \
         request_bytes, input_item_count, tool_count, system_block_count, \
         has_previous_response_id, compatibility_fixes) VALUES ( \
         $1, $2, $3, to_timestamp($4::double precision / 1000), $5, $6, $7, $8, $9, \
         $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, \
         $24, $25, $26, $27, $28) ON CONFLICT DO NOTHING",
    )
    .bind(machine_id)
    .bind(producer_id)
    .bind(i64::try_from(event.sequence).context("usage sequence overflow")?)
    .bind(event.occurred_at_ms)
    .bind(&event.account_fingerprint)
    .bind(&event.provider)
    .bind(&event.agent)
    .bind(&event.model)
    .bind(i32::from(event.status))
    .bind(provider_usage_metric(event.input_tokens)?)
    .bind(provider_usage_metric(event.output_tokens)?)
    .bind(provider_usage_metric(event.reasoning_tokens)?)
    .bind(provider_usage_metric(event.cache_hit_tokens)?)
    .bind(provider_usage_metric(event.cache_miss_tokens)?)
    .bind(i32::from(event.schema_version))
    .bind(&event.operation)
    .bind(&event.protocol)
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
    .execute(&mut **transaction)
    .await
    .context("insert provider usage event")?;
    Ok(())
}

const PROVIDER_USAGE_AGGREGATE_COLUMNS: &str = "count(*)::bigint AS requests, \
     count(*) FILTER (WHERE status >= 400)::bigint AS errors, \
     count(*) FILTER (WHERE completed IS TRUE)::bigint AS completed_requests, \
     count(completed)::bigint AS completion_observations, \
     count(*) FILTER (WHERE usage_observed IS TRUE)::bigint AS usage_observations, \
     least(coalesce(sum(input_tokens::numeric), 0), 9223372036854775807)::bigint AS input_tokens, \
     least(coalesce(sum(output_tokens::numeric), 0), 9223372036854775807)::bigint AS output_tokens, \
     least(coalesce(sum(reasoning_tokens::numeric), 0), 9223372036854775807)::bigint AS reasoning_tokens, \
     least(coalesce(sum(cache_hit_tokens::numeric) FILTER (WHERE cache_observation IN ('explicit', 'derived')), 0), 9223372036854775807)::bigint AS cache_hit_tokens, \
     least(coalesce(sum(cache_miss_tokens::numeric) FILTER (WHERE cache_observation IN ('explicit', 'derived')), 0), 9223372036854775807)::bigint AS cache_miss_tokens, \
     count(*) FILTER (WHERE cache_observation IN ('explicit', 'derived') AND cache_hit_tokens IS NOT NULL AND cache_miss_tokens IS NOT NULL)::bigint AS cache_observations, \
     count(*) FILTER (WHERE cache_observation = 'explicit')::bigint AS explicit_cache_observations, \
     count(*) FILTER (WHERE cache_observation = 'derived')::bigint AS derived_cache_observations, \
     count(*) FILTER (WHERE cache_observation = 'absent')::bigint AS absent_cache_observations, \
     count(*) FILTER (WHERE cache_observation IN ('explicit', 'derived') AND cache_hit_tokens::numeric + cache_miss_tokens::numeric > 0 AND cache_hit_tokens::numeric * 10 < cache_hit_tokens::numeric + cache_miss_tokens::numeric)::bigint AS cold_cache_requests, \
     count(*) FILTER (WHERE cache_observation IN ('explicit', 'derived') AND cache_hit_tokens::numeric + cache_miss_tokens::numeric > 0 AND cache_hit_tokens::numeric * 10 >= 9 * (cache_hit_tokens::numeric + cache_miss_tokens::numeric))::bigint AS hot_cache_requests, \
     least(coalesce(sum(duration_ms::numeric), 0), 9223372036854775807)::bigint AS duration_ms, \
     count(duration_ms)::bigint AS duration_observations, \
     least(coalesce(sum(request_bytes::numeric), 0), 9223372036854775807)::bigint AS request_bytes, \
     count(request_bytes)::bigint AS request_shape_observations, \
     least(coalesce(sum(input_item_count::numeric), 0), 9223372036854775807)::bigint AS input_item_count, \
     least(coalesce(sum(tool_count::numeric), 0), 9223372036854775807)::bigint AS tool_count, \
     least(coalesce(sum(system_block_count::numeric), 0), 9223372036854775807)::bigint AS system_block_count, \
     count(*) FILTER (WHERE has_previous_response_id IS TRUE)::bigint AS previous_response_requests, \
     least(coalesce(sum(compatibility_fixes::numeric), 0), 9223372036854775807)::bigint AS compatibility_fixes, \
     count(*) FILTER (WHERE streaming IS TRUE)::bigint AS streaming_requests";

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderUsageBreakdown {
    summary: UsageAggregate,
    by_agent: std::collections::BTreeMap<String, UsageAggregate>,
    by_machine: std::collections::BTreeMap<String, UsageAggregate>,
    by_operation: std::collections::BTreeMap<String, UsageAggregate>,
    by_model: std::collections::BTreeMap<String, UsageAggregate>,
    by_protocol: std::collections::BTreeMap<String, UsageAggregate>,
    by_agent_operation:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, UsageAggregate>>,
}

async fn load_provider_usage_breakdown(
    pool: &PgPool,
    provider: &str,
    days: i32,
) -> Result<ProviderUsageBreakdown> {
    let query = format!(
        "SELECT agent, machine_id, coalesce(model, '') AS model, operation, protocol, \
         {PROVIDER_USAGE_AGGREGATE_COLUMNS} FROM provider_usage_events WHERE provider = $1 \
         AND occurred_at >= now() - make_interval(days => $2::int) \
         GROUP BY agent, machine_id, model, operation, protocol"
    );
    let rows = sqlx::query(&query)
        .bind(provider)
        .bind(days)
        .fetch_all(pool)
        .await
        .context("aggregate provider usage")?;
    let mut breakdown = ProviderUsageBreakdown {
        summary: UsageAggregate::default(),
        by_agent: std::collections::BTreeMap::new(),
        by_machine: std::collections::BTreeMap::new(),
        by_operation: std::collections::BTreeMap::new(),
        by_model: std::collections::BTreeMap::new(),
        by_protocol: std::collections::BTreeMap::new(),
        by_agent_operation: std::collections::BTreeMap::new(),
    };
    for row in rows {
        let aggregate = UsageAggregate::from_row(&row);
        let agent = row.get::<String, _>("agent");
        let operation = row.get::<String, _>("operation");
        breakdown.summary.add(&aggregate);
        breakdown
            .by_agent
            .entry(agent.clone())
            .or_default()
            .add(&aggregate);
        breakdown
            .by_machine
            .entry(row.get("machine_id"))
            .or_default()
            .add(&aggregate);
        breakdown
            .by_operation
            .entry(operation.clone())
            .or_default()
            .add(&aggregate);
        breakdown
            .by_model
            .entry(row.get("model"))
            .or_default()
            .add(&aggregate);
        breakdown
            .by_protocol
            .entry(row.get("protocol"))
            .or_default()
            .add(&aggregate);
        breakdown
            .by_agent_operation
            .entry(agent)
            .or_default()
            .entry(operation)
            .or_default()
            .add(&aggregate);
    }
    Ok(breakdown)
}

async fn load_provider_usage_breakdown_hours(
    pool: &PgPool,
    provider: &str,
    hours: i32,
) -> Result<ProviderUsageBreakdown> {
    let query = format!(
        "SELECT agent, machine_id, coalesce(model, '') AS model, operation, protocol, \
         {PROVIDER_USAGE_AGGREGATE_COLUMNS} FROM provider_usage_events WHERE provider = $1 \
         AND occurred_at >= now() - make_interval(hours => $2::int) \
         GROUP BY agent, machine_id, model, operation, protocol"
    );
    let rows = sqlx::query(&query)
        .bind(provider)
        .bind(hours)
        .fetch_all(pool)
        .await
        .context("aggregate rolling provider usage")?;
    let mut breakdown = ProviderUsageBreakdown {
        summary: UsageAggregate::default(),
        by_agent: std::collections::BTreeMap::new(),
        by_machine: std::collections::BTreeMap::new(),
        by_operation: std::collections::BTreeMap::new(),
        by_model: std::collections::BTreeMap::new(),
        by_protocol: std::collections::BTreeMap::new(),
        by_agent_operation: std::collections::BTreeMap::new(),
    };
    for row in rows {
        let aggregate = UsageAggregate::from_row(&row);
        let agent = row.get::<String, _>("agent");
        let operation = row.get::<String, _>("operation");
        breakdown.summary.add(&aggregate);
        breakdown
            .by_agent
            .entry(agent.clone())
            .or_default()
            .add(&aggregate);
        breakdown
            .by_machine
            .entry(row.get("machine_id"))
            .or_default()
            .add(&aggregate);
        breakdown
            .by_operation
            .entry(operation.clone())
            .or_default()
            .add(&aggregate);
        breakdown
            .by_model
            .entry(row.get("model"))
            .or_default()
            .add(&aggregate);
        breakdown
            .by_protocol
            .entry(row.get("protocol"))
            .or_default()
            .add(&aggregate);
        breakdown
            .by_agent_operation
            .entry(agent)
            .or_default()
            .entry(operation)
            .or_default()
            .add(&aggregate);
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

impl Store {
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

    /// Aggregate Cowboy-measured usage separately from provider-owned account facts.
    pub async fn provider_usage_summary(
        &self,
        provider: &str,
        days: i32,
    ) -> Result<serde_json::Value> {
        let breakdown = load_provider_usage_breakdown(&self.pool, provider, days).await?;
        let last_24_hours = load_provider_usage_breakdown_hours(&self.pool, provider, 24).await?;
        let daily = load_daily_provider_usage(&self.pool, provider, days).await?;
        let producers = load_provider_usage_coverage(&self.pool, provider, days).await?;
        Ok(serde_json::json!({
            "source": "cowboy", "windowField": "occurred_at", "retentionDays": days,
            "summary": breakdown.summary, "byAgent": breakdown.by_agent,
            "byMachine": breakdown.by_machine, "byOperation": breakdown.by_operation,
            "byModel": breakdown.by_model, "byProtocol": breakdown.by_protocol,
            "byAgentOperation": breakdown.by_agent_operation, "daily": daily,
            "last24Hours": {
                "summary": last_24_hours.summary,
                "byModel": last_24_hours.by_model,
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

#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageAggregate {
    requests: i64,
    errors: i64,
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
}

impl UsageAggregate {
    fn from_row(row: &sqlx::postgres::PgRow) -> Self {
        Self {
            requests: row.get("requests"),
            errors: row.get("errors"),
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
        }
    }

    fn add(&mut self, other: &Self) {
        self.requests = self.requests.saturating_add(other.requests);
        self.errors = self.errors.saturating_add(other.errors);
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
    }
}

// --- row types ---------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    provider: String,
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
    mobile_review_state: serde_json::Value,
    #[allow(dead_code)]
    created_at: DateTime<Utc>,
}

impl SessionRow {
    fn into_meta(self) -> SessionMeta {
        SessionMeta {
            id: self.id,
            provider: self.provider,
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
            // Restored from the DB (migration 0008) so a finished session keeps its
            // "Task complete" across a daemon restart — a done session has no next
            // turn to re-judge. `crashed`/`interrupted` status still takes overlay
            // precedence, and the next turn re-judges + re-persists.
            awaiting_user: self.awaiting_user,
            done: self.done,
            // Restored from the DB (migration 0010) — a system session stays
            // view-only across a daemon restart.
            system: self.system,
            // Transient — a restored session is never mid-judge.
            judging: false,
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
