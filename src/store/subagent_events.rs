//! Observed subagent transcripts (docs/subagents.md#observed-transcripts).
//!
//! One append-only timeline per `(session, child)`, written only by the
//! observation writer (`crate::subagent_transcripts`) and read in pages by the
//! Web. Rows never enter the parent's `events` timeline.

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use anyhow::{Context as _, Result};

macro_rules! dispatch {
    ($store:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match &$store.backend {
            StorageBackend::Postgres(db) => db.$method($($arg),*).await,
            StorageBackend::Sqlite(db) => db.$method($($arg),*).await,
        }
    };
}

/// One stored row of a child's observed timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedRow {
    pub session_id: String,
    pub child_id: String,
    pub seq: i64,
    pub ts_ms: i64,
    pub worker_epoch: String,
    pub runtime_seq: i64,
    pub payload: serde_json::Value,
}

/// The newest row of one child and how many rows it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct ChildTail {
    pub seq: i64,
    pub rows: i64,
    pub payload: serde_json::Value,
}

/// A page of rows in ascending `seq`, and whether older rows remain.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ObservedPage {
    pub rows: Vec<(i64, i64, serde_json::Value)>,
    pub more_before: bool,
    /// A forward page stopped at its row limit.
    pub more_after: bool,
}

impl Store {
    /// Insert or replace rows (a coalesced message keeps its seq). Rows of a
    /// session deleted meanwhile are skipped, not an error.
    pub async fn upsert_subagent_events(&self, rows: &[ObservedRow]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        dispatch!(self, upsert_subagent_events(rows))
    }

    /// Delete one session's observed transcripts (the writer's Clear barrier).
    pub async fn delete_subagent_events(&self, session_id: &str) -> Result<()> {
        dispatch!(self, delete_subagent_events(session_id))
    }

    /// The newest `runtime_seq` stored for one worker incarnation.
    pub async fn subagent_events_highwater(
        &self,
        session_id: &str,
        worker_epoch: &str,
    ) -> Result<Option<i64>> {
        dispatch!(self, subagent_events_highwater(session_id, worker_epoch))
    }

    pub async fn subagent_events_tail(
        &self,
        session_id: &str,
        child_id: &str,
    ) -> Result<Option<ChildTail>> {
        dispatch!(self, subagent_events_tail(session_id, child_id))
    }

    /// Rows after `after_seq` (oldest first) when given, else the newest
    /// rows before `before_seq` (or the newest overall).
    pub async fn subagent_events_page(
        &self,
        session_id: &str,
        child_id: &str,
        after_seq: Option<i64>,
        before_seq: Option<i64>,
        limit: usize,
    ) -> Result<ObservedPage> {
        dispatch!(
            self,
            subagent_events_page(session_id, child_id, after_seq, before_seq, limit)
        )
    }
}

macro_rules! implementation {
    ($backend:ty, $begin:literal) => {
        impl $backend {
            async fn upsert_subagent_events(&self, rows: &[ObservedRow]) -> Result<()> {
                let mut tx = self
                    .pool
                    .begin_with($begin)
                    .await
                    .context("begin subagent events")?;
                for row in rows {
                    // PostgreSQL JSONB refuses U+0000; tool output may hold it.
                    let mut payload = row.payload.clone();
                    super::strip_nul(&mut payload);
                    sqlx::query(
                        "INSERT INTO subagent_events(session_id, child_id, seq, ts_ms, worker_epoch, runtime_seq, payload) \
                         SELECT $1, $2, $3, $4, $5, $6, $7 WHERE EXISTS(SELECT 1 FROM sessions WHERE id = $1) \
                         ON CONFLICT(session_id, child_id, seq) DO UPDATE SET ts_ms = EXCLUDED.ts_ms, \
                         worker_epoch = EXCLUDED.worker_epoch, runtime_seq = EXCLUDED.runtime_seq, \
                         payload = EXCLUDED.payload",
                    )
                    .bind(&row.session_id)
                    .bind(&row.child_id)
                    .bind(row.seq)
                    .bind(row.ts_ms)
                    .bind(&row.worker_epoch)
                    .bind(row.runtime_seq)
                    .bind(&payload)
                    .execute(&mut *tx)
                    .await
                    .with_context(|| {
                        format!("UPSERT subagent event {}/{}/{}", row.session_id, row.child_id, row.seq)
                    })?;
                }
                tx.commit().await.context("commit subagent events")
            }

            async fn delete_subagent_events(&self, session_id: &str) -> Result<()> {
                sqlx::query("DELETE FROM subagent_events WHERE session_id = $1")
                    .bind(session_id)
                    .execute(&self.pool)
                    .await
                    .context("DELETE observed subagent events")?;
                Ok(())
            }

            async fn subagent_events_highwater(
                &self,
                session_id: &str,
                worker_epoch: &str,
            ) -> Result<Option<i64>> {
                sqlx::query_scalar(
                    "SELECT MAX(runtime_seq) FROM subagent_events WHERE session_id = $1 AND worker_epoch = $2",
                )
                .bind(session_id)
                .bind(worker_epoch)
                .fetch_one(&self.pool)
                .await
                .context("SELECT subagent events highwater")
            }

            async fn subagent_events_tail(
                &self,
                session_id: &str,
                child_id: &str,
            ) -> Result<Option<ChildTail>> {
                let row: Option<(i64, serde_json::Value)> = sqlx::query_as(
                    "SELECT seq, payload FROM subagent_events WHERE session_id = $1 AND child_id = $2 \
                     ORDER BY seq DESC LIMIT 1",
                )
                .bind(session_id)
                .bind(child_id)
                .fetch_optional(&self.pool)
                .await
                .context("SELECT subagent events tail")?;
                let Some((seq, payload)) = row else {
                    return Ok(None);
                };
                let rows: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM subagent_events WHERE session_id = $1 AND child_id = $2",
                )
                .bind(session_id)
                .bind(child_id)
                .fetch_one(&self.pool)
                .await
                .context("COUNT subagent events")?;
                Ok(Some(ChildTail { seq, rows, payload }))
            }

            async fn subagent_events_page(
                &self,
                session_id: &str,
                child_id: &str,
                after_seq: Option<i64>,
                before_seq: Option<i64>,
                limit: usize,
            ) -> Result<ObservedPage> {
                let limit = i64::try_from(limit).unwrap_or(i64::MAX);
                if let Some(after) = after_seq {
                    let mut rows: Vec<(i64, i64, serde_json::Value)> = sqlx::query_as(
                        "SELECT seq, ts_ms, payload FROM subagent_events \
                         WHERE session_id = $1 AND child_id = $2 AND seq > $3 ORDER BY seq LIMIT $4",
                    )
                    .bind(session_id)
                    .bind(child_id)
                    .bind(after)
                    .bind(limit.saturating_add(1))
                    .fetch_all(&self.pool)
                    .await
                    .context("SELECT subagent events after")?;
                    let more_after = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
                    rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
                    return Ok(ObservedPage { rows, more_before: false, more_after });
                }
                let mut rows: Vec<(i64, i64, serde_json::Value)> = sqlx::query_as(
                    "SELECT seq, ts_ms, payload FROM subagent_events \
                     WHERE session_id = $1 AND child_id = $2 AND seq < $3 ORDER BY seq DESC LIMIT $4",
                )
                .bind(session_id)
                .bind(child_id)
                .bind(before_seq.unwrap_or(i64::MAX))
                .bind(limit.saturating_add(1))
                .fetch_all(&self.pool)
                .await
                .context("SELECT subagent events before")?;
                let more_before = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
                rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
                rows.reverse();
                Ok(ObservedPage { rows, more_before, more_after: false })
            }
        }
    };
}

implementation!(PostgresStorage, "BEGIN");
implementation!(SqliteStorage, "BEGIN IMMEDIATE");
