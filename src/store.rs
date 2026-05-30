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

use crate::core::{Envelope, Event, SessionMeta, SessionOrigin, Status};

/// All persistent state needed to rehydrate a single session after restart.
pub struct LoadedSession {
    pub meta: SessionMeta,
    pub events: Vec<Envelope>,
    /// Highest `seq + 1` for this session — what Hub uses to stamp the next
    /// event in line.
    pub next_seq: u64,
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
            "SELECT id, provider, cwd, title, origin, status, next_seq, created_at \
             FROM sessions ORDER BY created_at ASC",
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
                });
            }
            let next_seq = u64::try_from(row.next_seq).unwrap_or(0);
            out.push(LoadedSession {
                meta: row.into_meta(),
                events,
                next_seq,
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

    /// Remove a session. ON DELETE CASCADE on the FK drops every row in
    /// `events` for the same id.
    ///
    /// # Errors
    /// If the DELETE fails.
    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("DELETE session {session_id}"))?;
        Ok(())
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
    }
}

fn status_from_str(s: &str) -> Status {
    match s {
        "running" => Status::Running,
        "busy" => Status::Busy,
        "exited" => Status::Exited,
        "crashed" => Status::Crashed,
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
    next_seq: i64,
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
        }
    }
}

#[derive(sqlx::FromRow)]
struct EventRow {
    seq: i64,
    payload: serde_json::Value,
}
