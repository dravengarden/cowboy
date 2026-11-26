use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context as _, Result};
use chrono::Utc;
use futures::TryStreamExt as _;
use sqlx::{Postgres, Row as _, Transaction};

use super::{
    ArchivedLineageSegment, LineageEdge, LineageHistoryPage, decode_lineage_events,
    empty_sealed_page, next_segment_id, to_i64_seq,
};
use crate::store::PostgresStorage;

#[derive(sqlx::FromRow)]
struct PgEdgeRow {
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

impl From<PgEdgeRow> for LineageEdge {
    fn from(row: PgEdgeRow) -> Self {
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
struct PgSegmentRow {
    segment_id: String,
    source_session_id: String,
    owner_user_id: String,
    title: String,
    provider: String,
    before_seq: i64,
    reason: String,
    created_at_ms: i64,
}

impl From<PgSegmentRow> for ArchivedLineageSegment {
    fn from(row: PgSegmentRow) -> Self {
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

impl PostgresStorage {
    pub(super) async fn upsert_lineage_edge(&self, edge: &LineageEdge) -> Result<()> {
        sqlx::query(
            "INSERT INTO session_lineage_edges ( \
               child_session_id, parent_session_id, owner_user_id, kind, before_seq, \
               source_title, source_provider, segment_id, display_detached, edge_version, \
               created_at_ms \
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
             ON CONFLICT (child_session_id) DO UPDATE SET \
               parent_session_id = EXCLUDED.parent_session_id, \
               owner_user_id = EXCLUDED.owner_user_id, \
               kind = EXCLUDED.kind, \
               before_seq = EXCLUDED.before_seq, \
               source_title = EXCLUDED.source_title, \
               source_provider = EXCLUDED.source_provider, \
               segment_id = EXCLUDED.segment_id, \
               display_detached = EXCLUDED.display_detached, \
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
        .with_context(|| format!("UPSERT lineage edge for {}", edge.child_session_id))?;
        Ok(())
    }

    pub(super) async fn delete_lineage_edge(&self, child_session_id: &str) -> Result<bool> {
        let done = sqlx::query("DELETE FROM session_lineage_edges WHERE child_session_id = $1")
            .bind(child_session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("DELETE lineage edge for {child_session_id}"))?;
        Ok(done.rows_affected() > 0)
    }

    pub(super) async fn lineage_edge(&self, child_session_id: &str) -> Result<Option<LineageEdge>> {
        let row =
            sqlx::query_as::<_, PgEdgeRow>(&format!("{EDGE_SELECT} WHERE child_session_id = $1"))
                .bind(child_session_id)
                .fetch_optional(&self.pool)
                .await
                .with_context(|| format!("SELECT lineage edge for {child_session_id}"))?;
        Ok(row.map(LineageEdge::from))
    }

    pub(super) async fn load_all_lineage_edges(&self) -> Result<Vec<LineageEdge>> {
        let rows = sqlx::query_as::<_, PgEdgeRow>(EDGE_SELECT)
            .fetch_all(&self.pool)
            .await
            .context("SELECT all lineage edges")?;
        Ok(rows.into_iter().map(LineageEdge::from).collect())
    }

    pub(super) async fn lineage_children(
        &self,
        parent_session_id: &str,
    ) -> Result<Vec<LineageEdge>> {
        let rows = sqlx::query_as::<_, PgEdgeRow>(&format!(
            "{EDGE_SELECT} WHERE parent_session_id = $1 ORDER BY created_at_ms DESC"
        ))
        .bind(parent_session_id)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT lineage children for {parent_session_id}"))?;
        Ok(rows.into_iter().map(LineageEdge::from).collect())
    }

    pub(super) async fn lineage_segment(
        &self,
        segment_id: &str,
    ) -> Result<Option<ArchivedLineageSegment>> {
        let row = sqlx::query_as::<_, PgSegmentRow>(
            "SELECT segment_id, source_session_id, owner_user_id, title, provider, \
                    before_seq, reason, created_at_ms \
             FROM archived_lineage_segments WHERE segment_id = $1",
        )
        .bind(segment_id)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("SELECT archived lineage segment {segment_id}"))?;
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
        let rows: Vec<(i64, serde_json::Value)> = sqlx::query_as(
            "SELECT seq, payload FROM events \
             WHERE session_id = $1 AND seq < $2 ORDER BY seq DESC LIMIT $3",
        )
        .bind(&edge.parent_session_id)
        .bind(before_i64)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT live lineage page for {}", edge.parent_session_id))?;
        let events = decode_lineage_events(&edge.parent_session_id, rows);
        let oldest = events.first().map(|event| event.seq);
        let reached_start = match oldest {
            Some(seq) => !sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM events WHERE session_id = $1 AND seq < $2)",
            )
            .bind(&edge.parent_session_id)
            .bind(to_i64_seq(seq)?)
            .fetch_one(&self.pool)
            .await
            .context("SELECT live lineage start")?,
            None => true,
        };
        let session_next_seq: Option<i64> =
            sqlx::query_scalar("SELECT next_seq FROM sessions WHERE id = $1")
                .bind(&edge.parent_session_id)
                .fetch_optional(&self.pool)
                .await
                .context("SELECT parent next_seq for lineage seal")?;
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
        let rows: Vec<(i64, serde_json::Value)> = sqlx::query_as(
            "SELECT seq, payload FROM archived_lineage_events \
             WHERE segment_id = $1 AND seq < $2 ORDER BY seq DESC LIMIT $3",
        )
        .bind(segment_id)
        .bind(before_i64)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT archived lineage page for {segment_id}"))?;
        let events = decode_lineage_events(source_session_id, rows);
        let oldest = events.first().map(|event| event.seq);
        let reached_start = match oldest {
            Some(seq) => !sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM archived_lineage_events \
                 WHERE segment_id = $1 AND seq < $2)",
            )
            .bind(segment_id)
            .bind(to_i64_seq(seq)?)
            .fetch_one(&self.pool)
            .await
            .context("SELECT archived lineage start")?,
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

/// Seal any live unsealed child prefixes referencing `session_id` BEFORE Clear
/// deletes its `events`, and detach `session_id`'s own display edge while
/// preserving `parent_session_id` for auth ancestry.
pub(in crate::store) async fn seal_lineage_on_clear_pg(
    tx: &mut Transaction<'_, Postgres>,
    session_id: &str,
) -> Result<()> {
    let now_ms = Utc::now().timestamp_millis();
    sqlx::query(
        "UPDATE session_lineage_edges \
         SET display_detached = TRUE, segment_id = NULL, edge_version = edge_version + 1 \
         WHERE child_session_id = $1 AND (NOT display_detached OR segment_id IS NOT NULL)",
    )
    .bind(session_id)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("detach lineage display edge on clear for {session_id}"))?;

    sqlx::query("DELETE FROM session_turn_completions WHERE session_id = $1")
        .bind(session_id)
        .execute(&mut **tx)
        .await
        .with_context(|| format!("clear turn completions for {session_id}"))?;

    let max_before: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(before_seq) FROM session_lineage_edges \
         WHERE parent_session_id = $1 AND segment_id IS NULL AND NOT display_detached",
    )
    .bind(session_id)
    .fetch_one(&mut **tx)
    .await
    .with_context(|| format!("check child lineage prefixes for {session_id}"))?;
    let Some(max_before) = max_before else {
        return Ok(());
    };
    seal_session_prefix_pg(tx, session_id, max_before, "clear", now_ms).await
}

async fn seal_session_prefix_pg(
    tx: &mut Transaction<'_, Postgres>,
    session_id: &str,
    max_before: i64,
    reason: &str,
    now_ms: i64,
) -> Result<()> {
    let meta: Option<(String, String, String)> = sqlx::query_as(
        "SELECT COALESCE(owner_user_id, ''), title, provider FROM sessions WHERE id = $1",
    )
    .bind(session_id)
    .fetch_optional(&mut **tx)
    .await
    .with_context(|| format!("read session metadata for lineage archive {session_id}"))?;
    let Some((owner_user_id, title, provider)) = meta else {
        return Ok(());
    };
    let segment_id = next_segment_id(session_id, now_ms, reason);
    sqlx::query(
        "INSERT INTO archived_lineage_segments \
         (segment_id, source_session_id, owner_user_id, title, provider, before_seq, reason, created_at_ms) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
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
    .with_context(|| format!("INSERT archived_lineage_segments {segment_id}"))?;

    if max_before > 0 {
        sqlx::query(
            "INSERT INTO archived_lineage_events (segment_id, seq, ts_ms, payload) \
             SELECT $1, seq, (extract(epoch FROM ts) * 1000)::bigint, payload \
             FROM events WHERE session_id = $2 AND seq < $3",
        )
        .bind(&segment_id)
        .bind(session_id)
        .bind(max_before)
        .execute(&mut **tx)
        .await
        .with_context(|| format!("copy events into archived_lineage_events {segment_id}"))?;
    }

    sqlx::query(
        "UPDATE session_lineage_edges \
         SET segment_id = $1, edge_version = edge_version + 1 \
         WHERE parent_session_id = $2 AND segment_id IS NULL AND NOT display_detached",
    )
    .bind(&segment_id)
    .bind(session_id)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("bind archived segment {segment_id} onto edges"))?;
    Ok(())
}

/// Purge expired soft-deleted sessions in one transaction: archive transitively
/// reachable ancestor prefixes first, delete the expired `sessions` rows so
/// auth generations and worktrees are never pinned, GC unreachable lineage
/// edges/segments, and then prune unreferenced event artifacts.
pub(in crate::store) async fn purge_deleted_pg(
    storage: &PostgresStorage,
    retention_days: i64,
) -> Result<u64> {
    let days = i32::try_from(retention_days).unwrap_or(3);
    let now_ms = Utc::now().timestamp_millis();
    let mut tx = storage.pool.begin().await?;
    let purging: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM sessions WHERE deleted_at IS NOT NULL \
         AND COALESCE(purge_after_at, deleted_at + make_interval(days => $1::int)) < now() \
         AND NOT EXISTS (SELECT 1 FROM managed_agent_calls AS call \
             WHERE (call.parent_session_id = sessions.id OR call.child_session_id = sessions.id) \
             AND call.state NOT IN ('completed', 'failed', 'cancelled')) \
         AND NOT EXISTS (SELECT 1 FROM plugin_uninstall_operations AS operation \
             WHERE operation.machine_id = sessions.machine_id AND operation.plugin_id = sessions.provider \
             AND operation.phase NOT IN ('completed', 'compensated', 'aborted'))",
    )
    .bind(days)
    .fetch_all(&mut *tx)
    .await
    .context("select expired soft-deleted sessions")?;

    let to_archive: Vec<(String, i64)> = if purging.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(
            "WITH RECURSIVE display_reachable(id) AS ( \
               SELECT id FROM sessions WHERE NOT (id = ANY($1::text[])) \
               UNION \
               SELECT e.parent_session_id FROM session_lineage_edges AS e \
               JOIN display_reachable AS r ON e.child_session_id = r.id \
               WHERE NOT e.display_detached \
             ) \
             SELECT e.parent_session_id, MAX(e.before_seq) \
             FROM session_lineage_edges AS e \
             WHERE e.parent_session_id = ANY($1::text[]) \
               AND e.segment_id IS NULL \
               AND NOT e.display_detached \
               AND e.child_session_id IN (SELECT id FROM display_reachable) \
             GROUP BY e.parent_session_id",
        )
        .bind(&purging)
        .fetch_all(&mut *tx)
        .await
        .context("compute reachable lineage ancestors before purge")?
    };

    for (session_id, max_before) in to_archive {
        seal_session_prefix_pg(&mut tx, &session_id, max_before, "purge", now_ms).await?;
    }

    let purged_count = if purging.is_empty() {
        0
    } else {
        sqlx::query("DELETE FROM sessions WHERE id = ANY($1::text[])")
            .bind(&purging)
            .execute(&mut *tx)
            .await
            .context("purge soft-deleted sessions")?
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
    .context("GC unreachable lineage edges")?;

    sqlx::query(
        "DELETE FROM archived_lineage_segments \
         WHERE segment_id NOT IN ( \
           SELECT segment_id FROM session_lineage_edges WHERE segment_id IS NOT NULL \
         )",
    )
    .execute(&mut *tx)
    .await
    .context("GC unreferenced archived lineage segments")?;

    tx.commit().await?;
    prune_pg_artifacts(storage).await?;
    Ok(purged_count)
}

async fn prune_pg_artifacts(storage: &PostgresStorage) -> Result<()> {
    let mut referenced = HashSet::new();
    let mut rows = sqlx::query(
        "SELECT DISTINCT value #>> ARRAY[]::text[] AS reference \
         FROM events \
         CROSS JOIN LATERAL jsonb_path_query( \
             payload, \
             'strict $.** ? (@.type() == \"string\" && @ starts with \"/api/artifacts/\")'::jsonpath \
         ) AS value \
         UNION SELECT DISTINCT value #>> ARRAY[]::text[] AS reference \
         FROM archived_lineage_events \
         CROSS JOIN LATERAL jsonb_path_query( \
             payload, \
             'strict $.** ? (@.type() == \"string\" && @ starts with \"/api/artifacts/\")'::jsonpath \
         ) AS value \
         UNION SELECT value #>> ARRAY[]::text[] AS reference \
         FROM managed_agent_calls \
         CROSS JOIN LATERAL jsonb_path_query(document::jsonb, 'lax $.result.artifacts[*].url') AS value \
         WHERE jsonb_typeof(value) = 'string'",
    )
    .fetch(&storage.pool);
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
    let artifacts_removed = storage
        .artifacts
        .prune_unreferenced(&referenced, Duration::from_hours(24))
        .context("prune unreferenced PostgreSQL event artifacts")?;
    if artifacts_removed > 0 {
        tracing::info!(artifacts_removed, "purged unreferenced event artifacts");
    }
    Ok(())
}
