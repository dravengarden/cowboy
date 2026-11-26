use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context as _, Result};
use chrono::Utc;
use futures::TryStreamExt as _;
use sqlx::{Row as _, Sqlite, Transaction};

use super::{
    ArchivedLineageSegment, LineageEdge, LineageHistoryPage, decode_lineage_events,
    empty_sealed_page, next_segment_id, to_i64_seq,
};
use crate::store::sqlite::SqliteStorage;

#[derive(sqlx::FromRow)]
struct SqliteEdgeRow {
    child_session_id: String,
    parent_session_id: String,
    owner_user_id: String,
    kind: String,
    before_seq: i64,
    source_title: String,
    source_provider: String,
    segment_id: Option<String>,
    display_detached: bool,
    edge_version: i64,
    created_at_ms: i64,
}

impl From<SqliteEdgeRow> for LineageEdge {
    fn from(row: SqliteEdgeRow) -> Self {
        Self {
            child_session_id: row.child_session_id,
            parent_session_id: row.parent_session_id,
            owner_user_id: row.owner_user_id,
            kind: row.kind,
            before_seq: u64::try_from(row.before_seq).unwrap_or(0),
            source_title: row.source_title,
            source_provider: row.source_provider,
            segment_id: row.segment_id,
            display_detached: row.display_detached,
            edge_version: u64::try_from(row.edge_version).unwrap_or(1),
            created_at_ms: row.created_at_ms,
        }
    }
}

#[derive(sqlx::FromRow)]
struct SqliteSegmentRow {
    segment_id: String,
    source_session_id: String,
    owner_user_id: String,
    title: String,
    provider: String,
    before_seq: i64,
    reason: String,
    created_at_ms: i64,
}

impl From<SqliteSegmentRow> for ArchivedLineageSegment {
    fn from(row: SqliteSegmentRow) -> Self {
        Self {
            segment_id: row.segment_id,
            source_session_id: row.source_session_id,
            owner_user_id: row.owner_user_id,
            title: row.title,
            provider: row.provider,
            before_seq: u64::try_from(row.before_seq).unwrap_or(0),
            reason: row.reason,
            created_at_ms: row.created_at_ms,
        }
    }
}

const EDGE_SELECT: &str = "\
SELECT child_session_id, parent_session_id, owner_user_id, kind, before_seq, \
       source_title, source_provider, segment_id, display_detached, edge_version, created_at_ms \
FROM session_lineage_edges";

fn decode_json_rows(raw: Vec<(i64, String)>) -> Vec<(i64, serde_json::Value)> {
    raw.into_iter()
        .filter_map(|(seq, text)| match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(value) => Some((seq, value)),
            Err(error) => {
                tracing::warn!(%error, seq, "skipping malformed SQLite lineage event JSON");
                None
            }
        })
        .collect()
}

impl SqliteStorage {
    pub(super) async fn upsert_lineage_edge(&self, edge: &LineageEdge) -> Result<()> {
        sqlx::query(
            "INSERT INTO session_lineage_edges ( \
               child_session_id, parent_session_id, owner_user_id, kind, before_seq, \
               source_title, source_provider, segment_id, display_detached, edge_version, \
               created_at_ms \
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
             ON CONFLICT (child_session_id) DO UPDATE SET \
               parent_session_id = excluded.parent_session_id, \
               owner_user_id = excluded.owner_user_id, \
               kind = excluded.kind, \
               before_seq = excluded.before_seq, \
               source_title = excluded.source_title, \
               source_provider = excluded.source_provider, \
               segment_id = excluded.segment_id, \
               display_detached = excluded.display_detached, \
               edge_version = session_lineage_edges.edge_version + 1",
        )
        .bind(&edge.child_session_id)
        .bind(&edge.parent_session_id)
        .bind(&edge.owner_user_id)
        .bind(&edge.kind)
        .bind(to_i64_seq(edge.before_seq)?)
        .bind(&edge.source_title)
        .bind(&edge.source_provider)
        .bind(edge.segment_id.as_deref())
        .bind(edge.display_detached)
        .bind(to_i64_seq(edge.edge_version.max(1))?)
        .bind(edge.created_at_ms)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPSERT SQLite lineage edge for {}", edge.child_session_id))?;
        Ok(())
    }

    pub(super) async fn delete_lineage_edge(&self, child_session_id: &str) -> Result<bool> {
        let done = sqlx::query("DELETE FROM session_lineage_edges WHERE child_session_id = ?1")
            .bind(child_session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("DELETE SQLite lineage edge for {child_session_id}"))?;
        Ok(done.rows_affected() > 0)
    }

    pub(super) async fn lineage_edge(&self, child_session_id: &str) -> Result<Option<LineageEdge>> {
        let row = sqlx::query_as::<_, SqliteEdgeRow>(&format!(
            "{EDGE_SELECT} WHERE child_session_id = ?1"
        ))
        .bind(child_session_id)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite lineage edge for {child_session_id}"))?;
        Ok(row.map(LineageEdge::from))
    }

    pub(super) async fn load_all_lineage_edges(&self) -> Result<Vec<LineageEdge>> {
        let rows = sqlx::query_as::<_, SqliteEdgeRow>(EDGE_SELECT)
            .fetch_all(&self.pool)
            .await
            .context("SELECT all SQLite lineage edges")?;
        Ok(rows.into_iter().map(LineageEdge::from).collect())
    }

    pub(super) async fn lineage_children(&self, parent_session_id: &str) -> Result<Vec<LineageEdge>> {
        let rows = sqlx::query_as::<_, SqliteEdgeRow>(&format!(
            "{EDGE_SELECT} WHERE parent_session_id = ?1 ORDER BY created_at_ms DESC"
        ))
        .bind(parent_session_id)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite lineage children for {parent_session_id}"))?;
        Ok(rows.into_iter().map(LineageEdge::from).collect())
    }

    pub(super) async fn lineage_segment(
        &self,
        segment_id: &str,
    ) -> Result<Option<ArchivedLineageSegment>> {
        let row = sqlx::query_as::<_, SqliteSegmentRow>(
            "SELECT segment_id, source_session_id, owner_user_id, title, provider, \
                    before_seq, reason, created_at_ms \
             FROM archived_lineage_segments WHERE segment_id = ?1",
        )
        .bind(segment_id)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite archived lineage segment {segment_id}"))?;
        Ok(row.map(ArchivedLineageSegment::from))
    }

    pub(super) async fn lineage_history_page(
        &self,
        edge: &LineageEdge,
        before_seq: u64,
        page_size: usize,
    ) -> Result<LineageHistoryPage> {
        let effective_before = before_seq.min(edge.before_seq);
        if effective_before == 0 {
            return Ok(empty_sealed_page(edge.segment_id.clone()));
        }
        let before_i64 = to_i64_seq(effective_before)?;
        let limit = i64::try_from(page_size).context("lineage page size overflow")?;
        if let Some(segment_id) = edge.segment_id.as_deref() {
            return self
                .archived_history_page(segment_id, &edge.parent_session_id, before_i64, limit)
                .await;
        }
        let raw: Vec<(i64, String)> = sqlx::query_as(
            "SELECT seq, payload FROM events \
             WHERE session_id = ?1 AND seq < ?2 ORDER BY seq DESC LIMIT ?3",
        )
        .bind(&edge.parent_session_id)
        .bind(before_i64)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite live lineage page for {}", edge.parent_session_id))?;
        let events = decode_lineage_events(&edge.parent_session_id, decode_json_rows(raw));
        let oldest = events.first().map(|event| event.seq);
        let reached_start = match oldest {
            Some(seq) => {
                let exists: i64 = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM events WHERE session_id = ?1 AND seq < ?2)",
                )
                .bind(&edge.parent_session_id)
                .bind(to_i64_seq(seq)?)
                .fetch_one(&self.pool)
                .await
                .context("SELECT SQLite live lineage start")?;
                exists == 0
            }
            None => true,
        };
        let session_next_seq: Option<i64> =
            sqlx::query_scalar("SELECT next_seq FROM sessions WHERE id = ?1")
                .bind(&edge.parent_session_id)
                .fetch_optional(&self.pool)
                .await
                .context("SELECT SQLite parent next_seq for lineage seal")?;
        let is_sealed = session_next_seq
            .and_then(|value| u64::try_from(value).ok())
            .is_some_and(|next_seq| next_seq >= edge.before_seq);
        let next_before_seq = (!reached_start).then_some(oldest.unwrap_or(effective_before));
        Ok(LineageHistoryPage {
            events,
            next_before_seq,
            reached_start,
            is_sealed,
            segment_id: None,
        })
    }

    async fn archived_history_page(
        &self,
        segment_id: &str,
        source_session_id: &str,
        before_i64: i64,
        limit: i64,
    ) -> Result<LineageHistoryPage> {
        let raw: Vec<(i64, String)> = sqlx::query_as(
            "SELECT seq, payload FROM archived_lineage_events \
             WHERE segment_id = ?1 AND seq < ?2 ORDER BY seq DESC LIMIT ?3",
        )
        .bind(segment_id)
        .bind(before_i64)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite archived lineage page for {segment_id}"))?;
        let events = decode_lineage_events(source_session_id, decode_json_rows(raw));
        let oldest = events.first().map(|event| event.seq);
        let reached_start = match oldest {
            Some(seq) => {
                let exists: i64 = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM archived_lineage_events \
                     WHERE segment_id = ?1 AND seq < ?2)",
                )
                .bind(segment_id)
                .bind(to_i64_seq(seq)?)
                .fetch_one(&self.pool)
                .await
                .context("SELECT SQLite archived lineage start")?;
                exists == 0
            }
            None => true,
        };
        let next_before_seq = (!reached_start).then_some(oldest.unwrap_or(0));
        Ok(LineageHistoryPage {
            events,
            next_before_seq,
            reached_start,
            is_sealed: true,
            segment_id: Some(segment_id.to_owned()),
        })
    }
}

/// See `seal_lineage_on_clear_pg`.
pub(in crate::store) async fn seal_lineage_on_clear_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE session_lineage_edges \
         SET display_detached = TRUE, segment_id = NULL, edge_version = edge_version + 1 \
         WHERE child_session_id = ?1 AND (NOT display_detached OR segment_id IS NOT NULL)",
    )
    .bind(session_id)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("detach SQLite lineage display edge on clear for {session_id}"))?;

    sqlx::query("DELETE FROM session_turn_completions WHERE session_id = ?1")
        .bind(session_id)
        .execute(&mut **tx)
        .await
        .with_context(|| format!("clear SQLite turn completions for {session_id}"))?;

    let max_before: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(before_seq) FROM session_lineage_edges \
         WHERE parent_session_id = ?1 AND segment_id IS NULL AND NOT display_detached",
    )
    .bind(session_id)
    .fetch_one(&mut **tx)
    .await
    .with_context(|| format!("check SQLite child lineage prefixes for {session_id}"))?;
    let Some(max_before) = max_before else {
        return Ok(());
    };
    seal_session_prefix_sqlite(tx, session_id, max_before, "clear", now_ms).await
}

async fn seal_session_prefix_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
    max_before: i64,
    reason: &str,
    now_ms: i64,
) -> Result<()> {
    let meta: Option<(String, String, String)> = sqlx::query_as(
        "SELECT COALESCE(owner_user_id, ''), title, provider FROM sessions WHERE id = ?1",
    )
    .bind(session_id)
    .fetch_optional(&mut **tx)
    .await
    .with_context(|| format!("read SQLite session metadata for lineage archive {session_id}"))?;
    let Some((owner_user_id, title, provider)) = meta else {
        return Ok(());
    };
    let segment_id = next_segment_id(session_id, now_ms, reason);
    sqlx::query(
        "INSERT INTO archived_lineage_segments \
         (segment_id, source_session_id, owner_user_id, title, provider, before_seq, reason, created_at_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(&segment_id)
    .bind(session_id)
    .bind(&owner_user_id)
    .bind(&title)
    .bind(&provider)
    .bind(max_before)
    .bind(reason)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("INSERT SQLite archived_lineage_segments {segment_id}"))?;

    if max_before > 0 {
        sqlx::query(
            "INSERT INTO archived_lineage_events (segment_id, seq, ts_ms, payload) \
             SELECT ?1, seq, ts_ms, payload FROM events WHERE session_id = ?2 AND seq < ?3",
        )
        .bind(&segment_id)
        .bind(session_id)
        .bind(max_before)
        .execute(&mut **tx)
        .await
        .with_context(|| format!("copy SQLite events into archived_lineage_events {segment_id}"))?;
    }

    sqlx::query(
        "UPDATE session_lineage_edges \
         SET segment_id = ?1, edge_version = edge_version + 1 \
         WHERE parent_session_id = ?2 AND segment_id IS NULL AND NOT display_detached",
    )
    .bind(&segment_id)
    .bind(session_id)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("bind SQLite archived segment {segment_id} onto edges"))?;
    Ok(())
}

/// See `purge_deleted_pg`.
pub(in crate::store) async fn purge_deleted_sqlite(
    storage: &SqliteStorage,
    retention_days: i64,
) -> Result<u64> {
    let retention_ms = retention_days.saturating_mul(86_400_000);
    let now_ms = Utc::now().timestamp_millis();
    let mut tx = storage.pool.begin().await?;
    let purging: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM sessions WHERE deleted_at_ms IS NOT NULL \
         AND COALESCE(purge_after_at_ms, deleted_at_ms + ?1) <= ?2 \
         AND NOT EXISTS (SELECT 1 FROM managed_agent_calls AS call \
             WHERE (call.parent_session_id = sessions.id OR call.child_session_id = sessions.id) \
             AND call.state NOT IN ('completed', 'failed', 'cancelled')) \
         AND NOT EXISTS (SELECT 1 FROM plugin_uninstall_operations AS operation \
             WHERE operation.machine_id = sessions.machine_id AND operation.plugin_id = sessions.provider \
             AND operation.phase NOT IN ('completed', 'compensated', 'aborted'))",
    )
    .bind(retention_ms)
    .bind(now_ms)
    .fetch_all(&mut *tx)
    .await
    .context("select expired soft-deleted SQLite sessions")?;

    let purging_json = serde_json::to_string(&purging)?;
    let to_archive: Vec<(String, i64)> = if purging.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(
            "WITH RECURSIVE \
               purging(id) AS (SELECT value FROM json_each(?1)), \
               display_reachable(id) AS ( \
                 SELECT id FROM sessions WHERE id NOT IN (SELECT id FROM purging) \
                 UNION \
                 SELECT e.parent_session_id FROM session_lineage_edges AS e \
                 JOIN display_reachable AS r ON e.child_session_id = r.id \
                 WHERE NOT e.display_detached \
               ) \
             SELECT e.parent_session_id, MAX(e.before_seq) \
             FROM session_lineage_edges AS e \
             WHERE e.parent_session_id IN (SELECT id FROM purging) \
               AND e.segment_id IS NULL \
               AND NOT e.display_detached \
               AND e.child_session_id IN (SELECT id FROM display_reachable) \
             GROUP BY e.parent_session_id",
        )
        .bind(&purging_json)
        .fetch_all(&mut *tx)
        .await
        .context("compute reachable SQLite lineage ancestors before purge")?
    };

    for (session_id, max_before) in to_archive {
        seal_session_prefix_sqlite(&mut tx, &session_id, max_before, "purge", now_ms).await?;
    }

    let purged_count = if purging.is_empty() {
        0
    } else {
        sqlx::query("DELETE FROM sessions WHERE id IN (SELECT value FROM json_each(?1))")
            .bind(&purging_json)
            .execute(&mut *tx)
            .await
            .context("purge soft-deleted SQLite sessions")?
            .rows_affected()
    };

    sqlx::query(
        "WITH RECURSIVE auth_reachable(id) AS ( \
           SELECT id FROM sessions \
           UNION \
           SELECT e.parent_session_id FROM session_lineage_edges AS e \
           JOIN auth_reachable AS r ON e.child_session_id = r.id \
         ) \
         DELETE FROM session_lineage_edges \
         WHERE child_session_id NOT IN (SELECT id FROM auth_reachable)",
    )
    .execute(&mut *tx)
    .await
    .context("GC unreachable SQLite lineage edges")?;

    sqlx::query(
        "DELETE FROM archived_lineage_segments \
         WHERE segment_id NOT IN ( \
           SELECT segment_id FROM session_lineage_edges WHERE segment_id IS NOT NULL \
         )",
    )
    .execute(&mut *tx)
    .await
    .context("GC unreferenced SQLite archived lineage segments")?;

    tx.commit().await?;
    prune_sqlite_artifacts(storage).await?;
    Ok(purged_count)
}

async fn prune_sqlite_artifacts(storage: &SqliteStorage) -> Result<()> {
    let mut referenced = HashSet::new();
    let mut rows = sqlx::query(
        "SELECT DISTINCT tree.value AS reference \
         FROM events \
         CROSS JOIN json_tree(events.payload) AS tree \
         WHERE instr(events.payload, '/api/artifacts/') > 0 \
         AND tree.type = 'text' AND tree.value LIKE '/api/artifacts/%' \
         UNION SELECT DISTINCT tree.value AS reference \
         FROM archived_lineage_events \
         CROSS JOIN json_tree(archived_lineage_events.payload) AS tree \
         WHERE instr(archived_lineage_events.payload, '/api/artifacts/') > 0 \
         AND tree.type = 'text' AND tree.value LIKE '/api/artifacts/%' \
         UNION SELECT json_extract(image.value, '$.url') AS reference \
         FROM managed_agent_calls CROSS JOIN json_each(document, '$.result.artifacts') AS image \
         WHERE json_type(image.value, '$.url') = 'text'",
    )
    .fetch(&storage.pool);
    while let Some(row) = rows
        .try_next()
        .await
        .context("scan retained SQLite artifact references")?
    {
        let reference: String = row.try_get("reference")?;
        crate::artifacts::collect_references(&serde_json::Value::String(reference), &mut referenced);
    }
    let artifacts_removed = storage
        .artifacts
        .prune_unreferenced(&referenced, Duration::from_hours(24))
        .context("prune unreferenced SQLite event artifacts")?;
    if artifacts_removed > 0 {
        tracing::info!(
            artifacts_removed,
            "purged unreferenced SQLite event artifacts"
        );
    }
    Ok(())
}
