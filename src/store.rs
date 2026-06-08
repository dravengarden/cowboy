//! Postgres-backed persistence for cowboy state.
//!
//! [`Store`] is a thin wrapper around an `sqlx::PgPool` exposing the small
//! surface the [`Hub`] (and one background writer task in
//! [`crate::server`]) needs:
//!
//! - load all sessions + their events on daemon startup (warm restore);
//! - append a new session;
//! - append an event under a session;
//! - update a session's status;
//! - delete a session (cascades events).
//!
//! Writes from the hot path (`Hub::push`) go through an mpsc channel into the
//! background writer task, so a slow DB never blocks WS fan-out. That's
//! the write-behind tradeoff: in-memory and DB can diverge for a brief
//! window if the daemon crashes mid-flush. v0 accepts that — WS clients
//! already had no durability guarantees, and the alternative (sync writes)
//! couples broadcast latency to DB round-trips.
//!
//! Embedded migrations live next to `Cargo.toml` in `./migrations/`. Run
//! [`Store::migrate`] once on startup.

use std::time::Duration;

use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::core::{Envelope, Event, QueuedMessage, SessionMeta, SessionOrigin, Status};

/// All persistent state needed to rehydrate a single session after restart.
pub struct LoadedSession {
    pub meta: SessionMeta,
    pub events: Vec<Envelope>,
    /// Highest `seq + 1` for this session — what Hub uses to stamp the next
    /// event in line.
    pub next_seq: u64,
    /// Persisted send-queue + drafts (cross-terminal sync survives restart).
    pub queue: Vec<QueuedMessage>,
    pub drafts: Vec<QueuedMessage>,
}

#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    /// Open a pool against `url`.
    ///
    /// # Errors
    /// If the URL is malformed or the DB is unreachable within the connect
    /// timeout.
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(5))
            .connect(url)
            .await
            .with_context(|| format!("connecting to postgres {url}"))?;
        Ok(Self { pool })
    }

    /// Run embedded migrations under `./migrations/`. Idempotent.
    ///
    /// # Errors
    /// If a migration fails to apply.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .context("running migrations")?;
        Ok(())
    }

    /// Load every session with its full event log, restoring insertion
    /// order via `created_at`.
    ///
    /// # Errors
    /// If a query fails or a payload is unparseable.
    pub async fn load_all(&self) -> Result<Vec<LoadedSession>> {
        let session_rows: Vec<SessionRow> = sqlx::query_as::<_, SessionRow>(
            "SELECT id, provider, cwd, title, origin, status, agent_session_id, auto_resume, \
             next_seq, queue, drafts, created_at \
             FROM sessions WHERE deleted_at IS NULL ORDER BY position ASC NULLS LAST, created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .context("SELECT sessions")?;

        let mut out = Vec::with_capacity(session_rows.len());
        for row in session_rows {
            let id = row.id.clone();
            let event_rows: Vec<EventRow> = sqlx::query_as::<_, EventRow>(
                "SELECT seq, payload FROM events WHERE session_id = $1 ORDER BY seq ASC",
            )
            .bind(&id)
            .fetch_all(&self.pool)
            .await
            .with_context(|| format!("SELECT events for {id}"))?;

            let mut events = Vec::with_capacity(event_rows.len());
            for er in event_rows {
                let event: Event = serde_json::from_value(er.payload)
                    .with_context(|| format!("decoding event seq={} for {id}", er.seq))?;
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
            out.push(LoadedSession {
                meta: row.into_meta(),
                events,
                next_seq,
                queue,
                drafts,
            });
        }
        Ok(out)
    }

    /// Insert a brand-new session. Caller is expected to set `next_seq = 0`
    /// (the row default does it too).
    ///
    /// # Errors
    /// If the row already exists or the INSERT fails.
    pub async fn insert_session(&self, m: &SessionMeta) -> Result<()> {
        sqlx::query(
            "INSERT INTO sessions(id, provider, cwd, title, origin, status, next_seq) \
             VALUES ($1, $2, $3, $4, $5, $6, 0)",
        )
        .bind(&m.id)
        .bind(&m.provider)
        .bind(&m.cwd)
        .bind(&m.title)
        .bind(origin_to_str(m.origin))
        .bind(status_to_str(m.status))
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
        agent_session_id: &str,
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
            .bind(title)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE session title {session_id}"))?;
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
        sqlx::query(
            "INSERT INTO settings(key, value, updated_at) VALUES ($1, $2, now()) \
             ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPSERT setting {key}"))?;
        Ok(())
    }

    // --- Inference providers (design: tasks/active/confirm-detect-skills §H) ---
    // Config (model + params) and the API key live in SEPARATE tables so the key
    // never rides along a non-secret read: the web is only ever told "is a key
    // set?" (`inference_secret_set`), never the key itself.

    /// All stored inference configs: `(provider, model, params)`.
    ///
    /// # Errors
    /// If the SELECT fails.
    pub async fn load_inference_config(&self) -> Result<Vec<(String, String, serde_json::Value)>> {
        let rows = sqlx::query_as("SELECT provider, model, params FROM inference_config")
            .fetch_all(&self.pool)
            .await
            .context("SELECT inference_config")?;
        Ok(rows)
    }

    /// Upsert an inference provider's non-secret config.
    ///
    /// # Errors
    /// If the UPSERT fails.
    pub async fn put_inference_config(
        &self,
        provider: &str,
        model: &str,
        params: &serde_json::Value,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO inference_config(provider, model, params, updated_at) \
             VALUES ($1, $2, $3, now()) \
             ON CONFLICT (provider) DO UPDATE SET model = $2, params = $3, updated_at = now()",
        )
        .bind(provider)
        .bind(model)
        .bind(params)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPSERT inference_config {provider}"))?;
        Ok(())
    }

    /// Upsert an inference provider's API key.
    ///
    /// # Errors
    /// If the UPSERT fails.
    pub async fn put_inference_secret(&self, provider: &str, api_key: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO inference_secrets(provider, api_key, updated_at) \
             VALUES ($1, $2, now()) \
             ON CONFLICT (provider) DO UPDATE SET api_key = $2, updated_at = now()",
        )
        .bind(provider)
        .bind(api_key)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPSERT inference_secret {provider}"))?;
        Ok(())
    }

    /// The API key — INTERNAL ONLY (the judge call reads it). NEVER exposed to a
    /// web client; pair `inference_secret_set` for the client-facing fact.
    ///
    /// # Errors
    /// If the SELECT fails.
    pub async fn load_inference_secret(&self, provider: &str) -> Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT api_key FROM inference_secrets WHERE provider = $1")
                .bind(provider)
                .fetch_optional(&self.pool)
                .await
                .context("SELECT inference_secret")?;
        Ok(row.map(|(k,)| k))
    }

    /// Whether a key is set for `provider` — the ONLY secret fact the web is told.
    ///
    /// # Errors
    /// If the SELECT fails.
    pub async fn inference_secret_set(&self, provider: &str) -> Result<bool> {
        let row: Option<(i32,)> =
            sqlx::query_as("SELECT 1 FROM inference_secrets WHERE provider = $1")
                .bind(provider)
                .fetch_optional(&self.pool)
                .await
                .context("EXISTS inference_secret")?;
        Ok(row.is_some())
    }

    /// All API keys — INTERNAL ONLY, restore-time bulk load into the Hub's memory
    /// (so the judge can call without a per-turn DB read). Never reaches the web.
    ///
    /// # Errors
    /// If the SELECT fails.
    pub async fn load_inference_secrets(&self) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query_as("SELECT provider, api_key FROM inference_secrets")
            .fetch_all(&self.pool)
            .await
            .context("SELECT inference_secrets")?;
        Ok(rows)
    }

    /// Append an event under its session. Also bumps `sessions.next_seq` so
    /// the high-water mark survives restart. Single transaction so seq
    /// stays monotonic from any concurrent appender.
    ///
    /// # Errors
    /// If serializing the event fails, the transaction fails, or the seq
    /// conflicts with an existing row.
    pub async fn append_event(&self, env: &Envelope) -> Result<()> {
        let mut tx = self.pool.begin().await.context("begin tx")?;
        let payload = serde_json::to_value(&env.event).context("serialize event")?;
        let seq_i64 = i64::try_from(env.seq).context("seq i64 overflow")?;
        sqlx::query("INSERT INTO events(session_id, seq, payload) VALUES ($1, $2, $3)")
            .bind(&env.session_id)
            .bind(seq_i64)
            .bind(&payload)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("INSERT event {}/{}", env.session_id, env.seq))?;
        sqlx::query(
            "UPDATE sessions SET next_seq = GREATEST(next_seq, $1 + 1), updated_at = now() \
             WHERE id = $2",
        )
        .bind(seq_i64)
        .bind(&env.session_id)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("UPDATE next_seq for {}", env.session_id))?;
        tx.commit().await.context("commit append_event")?;
        Ok(())
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
        let queue_json = serde_json::to_value(queue).context("serialize queue")?;
        let drafts_json = serde_json::to_value(drafts).context("serialize drafts")?;
        sqlx::query("UPDATE sessions SET queue = $1, drafts = $2, updated_at = now() WHERE id = $3")
            .bind(&queue_json)
            .bind(&drafts_json)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE session pending {session_id}"))?;
        Ok(())
    }

    /// Persist the manual session ordering: write each id's index as its
    /// `position`. `load_all` then restores the drag-arranged order (NULLS LAST
    /// + created_at keeps any unknown/never-reordered rows sensible). One UPDATE
    /// per id in a single transaction — the list is short (a handful of
    /// sessions).
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
        SessionOrigin::Zed => "zed",
    }
}

fn origin_from_str(s: &str) -> SessionOrigin {
    match s {
        "zed" => SessionOrigin::Zed,
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

// --- row types ---------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    provider: String,
    cwd: String,
    title: String,
    origin: String,
    status: String,
    agent_session_id: Option<String>,
    auto_resume: Option<bool>,
    next_seq: i64,
    queue: serde_json::Value,
    drafts: serde_json::Value,
    #[allow(dead_code)]
    created_at: DateTime<Utc>,
}

impl SessionRow {
    fn into_meta(self) -> SessionMeta {
        SessionMeta {
            id: self.id,
            provider: self.provider,
            cwd: self.cwd,
            title: self.title,
            status: status_from_str(&self.status),
            origin: origin_from_str(&self.origin),
            agent_session_id: self.agent_session_id,
            auto_resume: self.auto_resume,
            // Never restored from the DB — a turn-end hold is transient; the next
            // turn re-judges. Always start cleared.
            awaiting_user: false,
        }
    }
}

#[derive(sqlx::FromRow)]
struct EventRow {
    seq: i64,
    payload: serde_json::Value,
}
