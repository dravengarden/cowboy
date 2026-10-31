//! Durable transcript anchors (docs/background-tasks.md#durable-anchors).
//!
//! The table is append-only per `(session, key)`: the first anchor wins and is
//! never moved. Rows are written in the same transaction as their event batch
//! (see `upsert_event_batch_with_anchors`) and deleted with the transcript.

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use crate::core::{Envelope, Event};
use crate::transcript_anchors::{
    AnchorWrite, BACKFILL_SENTINEL, CALL_PREFIX, METRICS, SUBAGENT_PREFIX, SUBAGENT_UPDATE,
};
use anyhow::{Context as _, Result};
use std::sync::atomic::Ordering;

macro_rules! dispatch {
    ($store:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match &$store.backend {
            StorageBackend::Postgres(db) => db.$method($($arg),*).await,
            StorageBackend::Sqlite(db) => db.$method($($arg),*).await,
        }
    };
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn decode(session_id: &str, seq: i64, mut payload: serde_json::Value) -> Option<Envelope> {
    let cmid = crate::persistence::take_persisted_cmid(&mut payload);
    match serde_json::from_value::<Event>(payload) {
        Ok(event) => Some(Envelope {
            session_id: session_id.to_owned(),
            seq: u64::try_from(seq).ok()?,
            event,
            cmid,
        }),
        Err(error) => {
            tracing::warn!(%error, session = %session_id, seq, "skipping undecodable anchor row");
            None
        }
    }
}

/// The current (in-place replaced) row of one anchored subagent.
#[derive(Debug, Clone)]
pub struct AnchoredRow {
    pub key: String,
    pub anchor_seq: u64,
    pub update: serde_json::Value,
}

fn anchored(
    session_id: &str,
    key: String,
    seq: i64,
    payload: serde_json::Value,
) -> Option<AnchoredRow> {
    let envelope = decode(session_id, seq, payload)?;
    let Event::Update { update } = envelope.event else {
        return None;
    };
    Some(AnchoredRow {
        key,
        anchor_seq: envelope.seq,
        update,
    })
}

impl Store {
    /// The durable anchor seq of one key.
    pub async fn transcript_anchor(&self, session_id: &str, key: &str) -> Result<Option<u64>> {
        METRICS.store_lookups.fetch_add(1, Ordering::Relaxed);
        dispatch!(self, transcript_anchor(session_id, key))
    }

    /// Anchors for the Hub to warm at start, newest first.
    pub async fn recent_transcript_anchors(&self, limit: usize) -> Result<Vec<AnchorWrite>> {
        dispatch!(self, recent_transcript_anchors(limit))
    }

    /// Scan a session's rows from before the table existed once: the earliest
    /// `cowboy_subagent` row of each child becomes its anchor. Returns the
    /// number of anchors found, or `None` when the session was scanned before.
    pub async fn backfill_subagent_anchors(&self, session_id: &str) -> Result<Option<usize>> {
        let found = dispatch!(self, backfill_subagent_anchors(session_id))?;
        if let Some(count) = found {
            METRICS
                .backfills
                .fetch_add(u64::try_from(count).unwrap_or(0), Ordering::Relaxed);
        }
        Ok(found)
    }

    /// Every anchored subagent row of a session (the latest durable snapshot of
    /// each child, at its anchor), bounded.
    pub async fn subagent_anchor_rows(&self, session_id: &str) -> Result<Vec<AnchoredRow>> {
        dispatch!(self, subagent_anchor_rows(session_id, None))
    }

    /// The anchored rows of the named children only.
    pub async fn subagent_anchor_rows_for(
        &self,
        session_id: &str,
        children: &[String],
    ) -> Result<Vec<AnchoredRow>> {
        if children.is_empty() {
            return Ok(Vec::new());
        }
        dispatch!(self, subagent_anchor_rows(session_id, Some(children)))
    }

    /// Non-dismissed Calls whose parent transcript has no anchor row yet, as
    /// `(parent_session_id, call_id)`. Bounded; oldest first.
    pub async fn calls_missing_anchor(&self, limit: usize) -> Result<Vec<(String, String)>> {
        dispatch!(self, calls_missing_anchor(limit))
    }
}

macro_rules! implementation {
    ($backend:ty, $subagent_rows_filter:literal, $child_expr:literal) => {
        impl $backend {
            async fn transcript_anchor(&self, session_id: &str, key: &str) -> Result<Option<u64>> {
                let seq: Option<i64> = sqlx::query_scalar(
                    "SELECT seq FROM transcript_anchors WHERE session_id = $1 AND anchor_key = $2",
                )
                .bind(session_id)
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .context("SELECT transcript anchor")?;
                Ok(seq.and_then(|seq| u64::try_from(seq).ok()))
            }

            async fn recent_transcript_anchors(&self, limit: usize) -> Result<Vec<AnchorWrite>> {
                let rows: Vec<(String, String, i64)> = sqlx::query_as(
                    "SELECT session_id, anchor_key, seq FROM transcript_anchors \
                     WHERE anchor_key <> $1 ORDER BY created_at_ms DESC LIMIT $2",
                )
                .bind(BACKFILL_SENTINEL)
                .bind(i64::try_from(limit).unwrap_or(i64::MAX))
                .fetch_all(&self.pool)
                .await
                .context("SELECT recent transcript anchors")?;
                Ok(rows
                    .into_iter()
                    .filter_map(|(session_id, key, seq)| {
                        Some(AnchorWrite { session_id, key, seq: u64::try_from(seq).ok()? })
                    })
                    .collect())
            }

            async fn backfill_subagent_anchors(&self, session_id: &str) -> Result<Option<usize>> {
                let mut tx = self.pool.begin().await.context("begin anchor backfill")?;
                let done: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM transcript_anchors WHERE session_id = $1 AND anchor_key = $2)",
                )
                .bind(session_id)
                .bind(BACKFILL_SENTINEL)
                .fetch_one(&mut *tx)
                .await
                .context("SELECT anchor backfill sentinel")?;
                if done {
                    return Ok(None);
                }
                let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = $1)")
                    .bind(session_id)
                    .fetch_one(&mut *tx)
                    .await?;
                if !exists {
                    return Ok(Some(0));
                }
                let rows: Vec<(Option<String>, i64)> = sqlx::query_as(concat!(
                    "SELECT ", $child_expr, " AS child, MIN(seq) FROM events WHERE session_id = $1 AND ",
                    $subagent_rows_filter, " GROUP BY 1"
                ))
                .bind(session_id)
                .bind(SUBAGENT_UPDATE)
                .fetch_all(&mut *tx)
                .await
                .context("SELECT legacy subagent rows")?;
                let created = now_ms();
                let mut found = 0;
                for (child, seq) in rows {
                    let Some(child) = child.filter(|child| crate::transcript_anchors::valid_id(child)) else {
                        continue;
                    };
                    let inserted = sqlx::query(
                        "INSERT INTO transcript_anchors(session_id, anchor_key, seq, created_at_ms) \
                         VALUES ($1, $2, $3, $4) ON CONFLICT(session_id, anchor_key) DO NOTHING",
                    )
                    .bind(session_id)
                    .bind(format!("{SUBAGENT_PREFIX}{child}"))
                    .bind(seq)
                    .bind(created)
                    .execute(&mut *tx)
                    .await
                    .context("INSERT backfilled anchor")?;
                    if inserted.rows_affected() == 1 {
                        found += 1;
                    }
                }
                sqlx::query(
                    "INSERT INTO transcript_anchors(session_id, anchor_key, seq, created_at_ms) \
                     VALUES ($1, $2, 0, $3) ON CONFLICT(session_id, anchor_key) DO NOTHING",
                )
                .bind(session_id)
                .bind(BACKFILL_SENTINEL)
                .bind(created)
                .execute(&mut *tx)
                .await
                .context("INSERT anchor backfill sentinel")?;
                tx.commit().await.context("commit anchor backfill")?;
                Ok(Some(found))
            }

            async fn subagent_anchor_rows(
                &self,
                session_id: &str,
                children: Option<&[String]>,
            ) -> Result<Vec<AnchoredRow>> {
                let rows: Vec<(String, i64, serde_json::Value)> = sqlx::query_as(
                    "SELECT a.anchor_key, a.seq, e.payload FROM transcript_anchors a \
                     JOIN events e ON e.session_id = a.session_id AND e.seq = a.seq \
                     WHERE a.session_id = $1 AND a.anchor_key LIKE $2 ORDER BY a.seq LIMIT 2000",
                )
                .bind(session_id)
                .bind(format!("{SUBAGENT_PREFIX}%"))
                .fetch_all(&self.pool)
                .await
                .context("SELECT anchored subagent rows")?;
                let wanted: Option<std::collections::HashSet<String>> = children.map(|children| {
                    children.iter().map(|child| format!("{SUBAGENT_PREFIX}{child}")).collect()
                });
                Ok(rows
                    .into_iter()
                    .filter(|(key, _, _)| wanted.as_ref().is_none_or(|wanted| wanted.contains(key)))
                    .filter_map(|(key, seq, payload)| anchored(session_id, key, seq, payload))
                    .collect())
            }

            async fn calls_missing_anchor(&self, limit: usize) -> Result<Vec<(String, String)>> {
                let rows: Vec<(String, String)> = sqlx::query_as(
                    "SELECT c.parent_session_id, c.call_id FROM managed_agent_calls c \
                     JOIN sessions s ON s.id = c.parent_session_id \
                     WHERE c.dismissed = FALSE AND NOT EXISTS (SELECT 1 FROM transcript_anchors a \
                       WHERE a.session_id = c.parent_session_id AND a.anchor_key = $1 || c.call_id) \
                     ORDER BY c.admission_order LIMIT $2",
                )
                .bind(CALL_PREFIX)
                .bind(i64::try_from(limit).unwrap_or(i64::MAX))
                .fetch_all(&self.pool)
                .await
                .context("SELECT calls missing transcript anchors")?;
                Ok(rows)
            }
        }
    };
}

implementation!(
    PostgresStorage,
    "payload->'update'->>'sessionUpdate' = $2",
    "payload->'update'->>'sessionId'"
);
implementation!(
    SqliteStorage,
    "json_extract(payload, '$.update.sessionUpdate') = $2",
    "json_extract(payload, '$.update.sessionId')"
);
