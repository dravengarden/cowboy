//! Durable transcript anchors (docs/background-tasks.md#durable-anchors).
//!
//! The table is append-only per `(session, key)`: the first anchor wins and is
//! never moved. Rows are written in the same transaction as their event batch
//! (see `upsert_event_batch_with_anchors`) and deleted with the transcript.

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use crate::core::{Envelope, Event};
use crate::transcript_anchors::{
    AnchorWrite, BACKFILL_SENTINEL, CALL_PREFIX, METRICS, SUBAGENT_PREFIX, SUBAGENT_UPDATE,
    snapshot_order,
};
use anyhow::{Context as _, Result};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

/// Rows read per page when scanning a session's anchored subagents.
const ANCHOR_PAGE_ROWS: usize = 500;

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
    /// `cowboy_subagent` row of each child becomes its anchor and takes that
    /// child's newest snapshot, because a writer restart or the old 4,096-child
    /// clear left later rows holding the current state. The superseded rows
    /// stay in history; clients merge them by id. Returns the number of anchors
    /// found, or `None` when the session was scanned before.
    pub async fn backfill_subagent_anchors(&self, session_id: &str) -> Result<Option<usize>> {
        let found = dispatch!(self, backfill_subagent_anchors(session_id))?;
        if let Some(count) = found {
            METRICS
                .backfills
                .fetch_add(u64::try_from(count).unwrap_or(0), Ordering::Relaxed);
        }
        Ok(found)
    }

    /// The anchored subagent rows of a session (the latest durable snapshot of
    /// each child, at its anchor) that `keep` retains. The whole set is scanned
    /// in pages, so a long session is never silently cut short.
    pub async fn subagent_anchor_rows(
        &self,
        session_id: &str,
        keep: impl Fn(&AnchoredRow) -> bool,
    ) -> Result<Vec<AnchoredRow>> {
        let mut rows = Vec::new();
        let mut after = -1;
        loop {
            let page = dispatch!(
                self,
                subagent_anchor_page(session_id, after, ANCHOR_PAGE_ROWS)
            )?;
            let full = page.len() == ANCHOR_PAGE_ROWS;
            for (key, seq, payload) in page {
                after = seq;
                if let Some(row) = anchored(session_id, key, seq, payload)
                    && keep(&row)
                {
                    rows.push(row);
                }
            }
            if !full {
                return Ok(rows);
            }
        }
    }

    /// The anchored rows of the named children only.
    pub async fn subagent_anchor_rows_for(
        &self,
        session_id: &str,
        children: &[String],
    ) -> Result<Vec<AnchoredRow>> {
        let mut rows = Vec::with_capacity(children.len());
        for child in children {
            let key = format!("{SUBAGENT_PREFIX}{child}");
            if let Some((seq, payload)) = dispatch!(self, subagent_anchor_row(session_id, &key))?
                && let Some(row) = anchored(session_id, key, seq, payload)
            {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    /// The latest durable snapshots across sessions, newest anchors first, as
    /// `(session, row)`. A restarted Hub fences replays against them.
    pub async fn recent_subagent_snapshots(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, AnchoredRow)>> {
        let rows = dispatch!(self, recent_subagent_snapshots(limit))?;
        Ok(rows
            .into_iter()
            .filter_map(|(session_id, key, seq, payload)| {
                let row = anchored(&session_id, key, seq, payload)?;
                Some((session_id, row))
            })
            .collect())
    }

    /// The newest admitted Call. Calls at or below it when anchors first ran
    /// never had one.
    pub async fn managed_call_admission_high_water(&self) -> Result<i64> {
        dispatch!(self, managed_call_admission_high_water())
    }

    /// Calls whose parent transcript should hold an anchor row and does not, as
    /// `(parent_session_id, call_id)`: not dismissed, and either admitted after
    /// `since_admission` or still active. Bounded; oldest first.
    pub async fn calls_missing_anchor(
        &self,
        since_admission: i64,
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        dispatch!(self, calls_missing_anchor(since_admission, limit))
    }
}

macro_rules! implementation {
    ($backend:ty, $subagent_rows_filter:literal) => {
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
                let rows: Vec<(i64, serde_json::Value)> = sqlx::query_as(concat!(
                    "SELECT seq, payload FROM events WHERE session_id = $1 AND ",
                    $subagent_rows_filter, " ORDER BY seq"
                ))
                .bind(session_id)
                .bind(SUBAGENT_UPDATE)
                .fetch_all(&mut *tx)
                .await
                .context("SELECT legacy subagent rows")?;
                // child -> (earliest seq, seq of the newest snapshot, its order)
                let mut children: BTreeMap<String, (i64, i64, (u64, u64))> = BTreeMap::new();
                for (seq, payload) in &rows {
                    let Some(update) = payload.get("update") else { continue };
                    let Some(child) = update
                        .get("sessionId")
                        .and_then(serde_json::Value::as_str)
                        .filter(|child| crate::transcript_anchors::valid_id(child))
                    else {
                        continue;
                    };
                    let order = snapshot_order(update);
                    children
                        .entry(child.to_owned())
                        .and_modify(|(_, newest, best)| {
                            if order >= *best {
                                *newest = *seq;
                                *best = order;
                            }
                        })
                        .or_insert((*seq, *seq, order));
                }
                let created = now_ms();
                let mut found = 0;
                for (child, (anchor, newest, _)) in children {
                    let inserted = sqlx::query(
                        "INSERT INTO transcript_anchors(session_id, anchor_key, seq, created_at_ms) \
                         VALUES ($1, $2, $3, $4) ON CONFLICT(session_id, anchor_key) DO NOTHING",
                    )
                    .bind(session_id)
                    .bind(format!("{SUBAGENT_PREFIX}{child}"))
                    .bind(anchor)
                    .bind(created)
                    .execute(&mut *tx)
                    .await
                    .context("INSERT backfilled anchor")?;
                    if inserted.rows_affected() != 1 {
                        continue;
                    }
                    found += 1;
                    if newest != anchor {
                        sqlx::query(
                            "UPDATE events SET payload = \
                             (SELECT newest.payload FROM events newest \
                              WHERE newest.session_id = $1 AND newest.seq = $2) \
                             WHERE session_id = $1 AND seq = $3",
                        )
                        .bind(session_id)
                        .bind(newest)
                        .bind(anchor)
                        .execute(&mut *tx)
                        .await
                        .context("UPDATE backfilled anchor row")?;
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

            async fn subagent_anchor_page(
                &self,
                session_id: &str,
                after: i64,
                limit: usize,
            ) -> Result<Vec<(String, i64, serde_json::Value)>> {
                sqlx::query_as(
                    "SELECT a.anchor_key, a.seq, e.payload FROM transcript_anchors a \
                     JOIN events e ON e.session_id = a.session_id AND e.seq = a.seq \
                     WHERE a.session_id = $1 AND a.anchor_key LIKE $2 AND a.seq > $3 \
                     ORDER BY a.seq LIMIT $4",
                )
                .bind(session_id)
                .bind(format!("{SUBAGENT_PREFIX}%"))
                .bind(after)
                .bind(i64::try_from(limit).unwrap_or(i64::MAX))
                .fetch_all(&self.pool)
                .await
                .context("SELECT anchored subagent rows")
            }

            async fn subagent_anchor_row(
                &self,
                session_id: &str,
                key: &str,
            ) -> Result<Option<(i64, serde_json::Value)>> {
                sqlx::query_as(
                    "SELECT a.seq, e.payload FROM transcript_anchors a \
                     JOIN events e ON e.session_id = a.session_id AND e.seq = a.seq \
                     WHERE a.session_id = $1 AND a.anchor_key = $2",
                )
                .bind(session_id)
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .context("SELECT anchored subagent row")
            }

            async fn recent_subagent_snapshots(
                &self,
                limit: usize,
            ) -> Result<Vec<(String, String, i64, serde_json::Value)>> {
                sqlx::query_as(
                    "SELECT a.session_id, a.anchor_key, a.seq, e.payload FROM transcript_anchors a \
                     JOIN events e ON e.session_id = a.session_id AND e.seq = a.seq \
                     WHERE a.anchor_key LIKE $1 ORDER BY a.created_at_ms DESC LIMIT $2",
                )
                .bind(format!("{SUBAGENT_PREFIX}%"))
                .bind(i64::try_from(limit).unwrap_or(i64::MAX))
                .fetch_all(&self.pool)
                .await
                .context("SELECT recent subagent snapshots")
            }

            async fn managed_call_admission_high_water(&self) -> Result<i64> {
                sqlx::query_scalar("SELECT COALESCE(MAX(admission_order), 0) FROM managed_agent_calls")
                    .fetch_one(&self.pool)
                    .await
                    .context("SELECT managed call admission high water")
            }

            async fn calls_missing_anchor(
                &self,
                since_admission: i64,
                limit: usize,
            ) -> Result<Vec<(String, String)>> {
                let rows: Vec<(String, String)> = sqlx::query_as(
                    "SELECT c.parent_session_id, c.call_id FROM managed_agent_calls c \
                     JOIN sessions s ON s.id = c.parent_session_id \
                     WHERE c.dismissed = FALSE \
                       AND (c.admission_order > $2 OR c.state IN \
                         ('queued', 'starting', 'running', 'waiting_input', 'stopping')) \
                       AND NOT EXISTS (SELECT 1 FROM transcript_anchors a \
                         WHERE a.session_id = c.parent_session_id AND a.anchor_key = $1 || c.call_id) \
                     ORDER BY c.admission_order LIMIT $3",
                )
                .bind(CALL_PREFIX)
                .bind(since_admission)
                .bind(i64::try_from(limit).unwrap_or(i64::MAX))
                .fetch_all(&self.pool)
                .await
                .context("SELECT calls missing transcript anchors")?;
                Ok(rows)
            }
        }
    };
}

implementation!(PostgresStorage, "payload->'update'->>'sessionUpdate' = $2");
implementation!(
    SqliteStorage,
    "json_extract(payload, '$.update.sessionUpdate') = $2"
);
