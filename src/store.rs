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

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::core::{Envelope, Event, JudgeRun, QueuedMessage, SessionMeta, SessionOrigin, Status};

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
        serde_json::Value::String(_) => {}
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
}

#[derive(Clone)]
pub struct Store {
    pool: PgPool,
    artifacts: crate::artifacts::ArtifactStore,
}

impl Store {
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
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .context("running migrations")?;
        Ok(())
    }

    /// Load every session with only its recent event tail. Older history remains
    /// in Postgres and is fetched by [`Self::history_page`].
    ///
    /// # Errors
    /// If a query fails or a payload is unparseable.
    pub async fn load_all(&self) -> Result<Vec<LoadedSession>> {
        let session_rows: Vec<SessionRow> = sqlx::query_as::<_, SessionRow>(
            "SELECT id, provider, cwd, title, origin, status, agent_session_id, auto_resume, \
             awaiting_user, done, system, next_seq, queue, drafts, judge_runs, created_at \
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
            out.push(LoadedSession {
                meta: row.into_meta(),
                events,
                event_count,
                reached_start,
                next_seq,
                queue,
                drafts,
                judge_runs,
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

    /// Insert a brand-new session. Caller is expected to set `next_seq = 0`
    /// (the row default does it too).
    ///
    /// # Errors
    /// If the row already exists or the INSERT fails.
    pub async fn insert_session(&self, m: &SessionMeta) -> Result<()> {
        sqlx::query(
            "INSERT INTO sessions(id, provider, cwd, title, origin, status, next_seq, system) \
             VALUES ($1, $2, $3, $4, $5, $6, 0, $7)",
        )
        .bind(&m.id)
        .bind(&m.provider)
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
            .bind(strip_nul_str(title))
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
    awaiting_user: bool,
    done: bool,
    system: bool,
    next_seq: i64,
    queue: serde_json::Value,
    drafts: serde_json::Value,
    judge_runs: serde_json::Value,
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
