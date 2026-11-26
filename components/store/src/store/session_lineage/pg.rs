use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context as _, Result};
use chrono::Utc;
use futures::TryStreamExt as _;
use sqlx::{Postgres, Row as _, Transaction};

use super::{
    ArchivedLineageSegment, ChildDisplayLineage, LineageEdge, LineageHistoryOutcome,
    LineageHistoryPage, MAX_MATERIALIZED_HOPS, decode_lineage_events, empty_sealed_page,
    next_segment_id, to_i64_seq,
};
use crate::store::PostgresStorage;

#[derive(sqlx::FromRow)]
struct PgEdgeRow {
    child_session_id: String,
    hop_order: i32,
    parent_session_id: String,
    parent_is_root: bool,
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
            hop_order: u32::try_from(row.hop_order).unwrap_or(1),
            parent_session_id: row.parent_session_id,
            parent_is_root: row.parent_is_root,
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
SELECT child_session_id, hop_order, parent_session_id, parent_is_root, owner_user_id, \
       kind, before_seq, source_title, source_provider, segment_id, display_detached, \
       edge_version, created_at_ms \
FROM session_lineage_edges";

impl PostgresStorage {
    pub(super) async fn upsert_lineage_edge(&self, edge: &LineageEdge) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let hop_order = i32::try_from(edge.hop_order.max(1)).unwrap_or(1);
        sqlx::query(
            "INSERT INTO session_lineage_edges ( \
               child_session_id, hop_order, parent_session_id, parent_is_root, owner_user_id, \
               kind, before_seq, source_title, source_provider, segment_id, display_detached, \
               edge_version, created_at_ms \
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
             ON CONFLICT (child_session_id, hop_order) DO UPDATE SET \
               parent_session_id = EXCLUDED.parent_session_id, \
               parent_is_root = EXCLUDED.parent_is_root, \
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
        .bind(hop_order)
        .bind(&edge.parent_session_id)
        .bind(edge.parent_is_root)
        .bind(&edge.owner_user_id)
        .bind(&edge.kind)
        .bind(to_i64_seq(edge.before_seq)?)
        .bind(&edge.source_title)
        .bind(&edge.source_provider)
        .bind(edge.segment_id.as_deref())
        .bind(edge.display_detached)
        .bind(to_i64_seq(edge.edge_version.max(1))?)
        .bind(edge.created_at_ms)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("UPSERT lineage edge for {}", edge.child_session_id))?;

        if hop_order == 1 {
            // Rebuild materialized deeper hops (`hop_order >= 2`) from the
            // parent's active display hops so clearing the parent later never
            // strips inherited history from this child.
            sqlx::query(
                "DELETE FROM session_lineage_edges WHERE child_session_id = $1 AND hop_order > 1",
            )
            .bind(&edge.child_session_id)
            .execute(&mut *tx)
            .await
            .context("clear prior materialized lineage hops")?;

            if !edge.display_detached {
                let parent_hops: Vec<PgEdgeRow> = sqlx::query_as(&format!(
                    "{EDGE_SELECT} WHERE child_session_id = $1 AND NOT display_detached \
                     ORDER BY hop_order ASC LIMIT $2"
                ))
                .bind(&edge.parent_session_id)
                .bind(i64::from(MAX_MATERIALIZED_HOPS.saturating_sub(1)))
                .fetch_all(&mut *tx)
                .await
                .context("read parent display hops for materialization")?;
                let mut seen = HashSet::from([
                    edge.child_session_id.clone(),
                    edge.parent_session_id.clone(),
                ]);
                for (next_hop, inherited) in (2_i32..).zip(parent_hops) {
                    if !seen.insert(inherited.parent_session_id.clone()) {
                        break;
                    }
                    sqlx::query(
                        "INSERT INTO session_lineage_edges ( \
                           child_session_id, hop_order, parent_session_id, parent_is_root, \
                           owner_user_id, kind, before_seq, source_title, source_provider, \
                           segment_id, display_detached, edge_version, created_at_ms \
                         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, FALSE, 1, $11)",
                    )
                    .bind(&edge.child_session_id)
                    .bind(next_hop)
                    .bind(&inherited.parent_session_id)
                    .bind(inherited.parent_is_root)
                    .bind(&edge.owner_user_id)
                    .bind(&inherited.kind)
                    .bind(inherited.before_seq)
                    .bind(&inherited.source_title)
                    .bind(&inherited.source_provider)
                    .bind(inherited.segment_id.as_deref())
                    .bind(inherited.created_at_ms)
                    .execute(&mut *tx)
                    .await
                    .context("materialize inherited lineage hop")?;
                }
            }
        }
        tx.commit().await?;
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
        let row = sqlx::query_as::<_, PgEdgeRow>(&format!(
            "{EDGE_SELECT} WHERE child_session_id = $1 AND hop_order = 1"
        ))
        .bind(child_session_id)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("SELECT lineage edge for {child_session_id}"))?;
        Ok(row.map(LineageEdge::from))
    }

    pub(super) async fn child_display_lineage(
        &self,
        child_session_id: &str,
    ) -> Result<ChildDisplayLineage> {
        let rows = sqlx::query_as::<_, PgEdgeRow>(&format!(
            "{EDGE_SELECT} WHERE child_session_id = $1 ORDER BY hop_order ASC"
        ))
        .bind(child_session_id)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT child display lineage for {child_session_id}"))?;
        let has_durable_edge = !rows.is_empty();
        let hops = rows
            .into_iter()
            .map(LineageEdge::from)
            .filter(|edge| !edge.display_detached)
            .collect();
        Ok(ChildDisplayLineage {
            has_durable_edge,
            hops,
        })
    }

    pub(super) async fn load_all_lineage_edges(&self) -> Result<Vec<LineageEdge>> {
        let rows = sqlx::query_as::<_, PgEdgeRow>(&format!(
            "{EDGE_SELECT} ORDER BY child_session_id, hop_order ASC"
        ))
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
            "{EDGE_SELECT} WHERE parent_session_id = $1 AND hop_order = 1 ORDER BY created_at_ms DESC"
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

    pub(super) async fn session_has_live_events_before(
        &self,
        session_id: &str,
        before_seq: u64,
    ) -> Result<bool> {
        if before_seq == 0 {
            return Ok(false);
        }
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM events WHERE session_id = $1 AND seq < $2)",
        )
        .bind(session_id)
        .bind(to_i64_seq(before_seq)?)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("check live events for {session_id}"))
    }

    pub(super) async fn lineage_history_page(
        &self,
        edge: &LineageEdge,
        before_seq: u64,
        page_size: usize,
    ) -> Result<LineageHistoryPage> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let page = read_history_page_in_tx(&mut tx, edge, before_seq, page_size).await?;
        tx.commit().await?;
        Ok(page)
    }

    pub(super) async fn lineage_history_page_checked(
        &self,
        child_session_id: &str,
        hop_parent_id: &str,
        owner_user_id: Option<&str>,
        expected_edge_version: Option<u64>,
        before_seq: u64,
        page_size: usize,
    ) -> Result<LineageHistoryOutcome> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query_as::<_, PgEdgeRow>(&format!(
            "{EDGE_SELECT} WHERE child_session_id = $1 AND parent_session_id = $2 \
             AND NOT display_detached ORDER BY hop_order ASC LIMIT 1"
        ))
        .bind(child_session_id)
        .bind(hop_parent_id)
        .fetch_optional(&mut *tx)
        .await
        .context("SELECT snapshot lineage edge")?;
        let Some(edge) = row.map(LineageEdge::from) else {
            return Ok(LineageHistoryOutcome::NotFound);
        };
        if let Some(owner) = owner_user_id
            && edge.owner_user_id != owner
        {
            return Ok(LineageHistoryOutcome::NotFound);
        }
        if let Some(expected) = expected_edge_version
            && expected != edge.edge_version
        {
            return Ok(LineageHistoryOutcome::VersionConflict {
                current_version: edge.edge_version,
            });
        }
        let page = read_history_page_in_tx(&mut tx, &edge, before_seq, page_size).await?;
        tx.commit().await?;
        Ok(LineageHistoryOutcome::Page(page))
    }
}

async fn read_history_page_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    edge: &LineageEdge,
    before_seq: u64,
    page_size: usize,
) -> Result<LineageHistoryPage> {
    let effective_before = before_seq.min(edge.before_seq);
    if effective_before == 0 {
        return Ok(empty_sealed_page(
            edge.segment_id.clone(),
            edge.edge_version,
        ));
    }
    let before_i64 = to_i64_seq(effective_before)?;
    let limit = i64::try_from(page_size).context("lineage page size overflow")?;
    if let Some(segment_id) = edge.segment_id.as_deref() {
        let rows: Vec<(i64, serde_json::Value)> = sqlx::query_as(
            "SELECT seq, payload FROM archived_lineage_events \
             WHERE segment_id = $1 AND seq < $2 ORDER BY seq DESC LIMIT $3",
        )
        .bind(segment_id)
        .bind(before_i64)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await
        .with_context(|| format!("SELECT archived lineage page for {segment_id}"))?;
        let events = decode_lineage_events(&edge.parent_session_id, rows);
        let oldest = events.first().map(|event| event.seq);
        let reached_start = match oldest {
            Some(seq) => !sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM archived_lineage_events \
                 WHERE segment_id = $1 AND seq < $2)",
            )
            .bind(segment_id)
            .bind(to_i64_seq(seq)?)
            .fetch_one(&mut **tx)
            .await
            .context("SELECT archived lineage start")?,
            None => true,
        };
        let next_before_seq = (!reached_start).then_some(oldest.unwrap_or(0));
        return Ok(LineageHistoryPage {
            events,
            next_before_seq,
            reached_start,
            is_sealed: true,
            segment_id: Some(segment_id.to_owned()),
            edge_version: edge.edge_version,
        });
    }
    let rows: Vec<(i64, serde_json::Value)> = sqlx::query_as(
        "SELECT seq, payload FROM events \
         WHERE session_id = $1 AND seq < $2 ORDER BY seq DESC LIMIT $3",
    )
    .bind(&edge.parent_session_id)
    .bind(before_i64)
    .bind(limit)
    .fetch_all(&mut **tx)
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
        .fetch_one(&mut **tx)
        .await
        .context("SELECT live lineage start")?,
        None => true,
    };
    let session_next_seq: Option<i64> =
        sqlx::query_scalar("SELECT next_seq FROM sessions WHERE id = $1")
            .bind(&edge.parent_session_id)
            .fetch_optional(&mut **tx)
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
        edge_version: edge.edge_version,
    })
}

/// Seal any live unsealed child prefixes referencing `session_id` BEFORE Clear
/// deletes its `events`, and detach ONLY `session_id`'s own display edges while
/// preserving existing descendants' materialized ancestor hops and `hop_order = 1`
/// auth ancestry.
pub(in crate::store) async fn seal_lineage_on_clear_pg(
    tx: &mut Transaction<'_, Postgres>,
    session_id: &str,
) -> Result<()> {
    let now_ms = Utc::now().timestamp_millis();
    sqlx::query("DELETE FROM session_lineage_edges WHERE child_session_id = $1 AND hop_order > 1")
        .bind(session_id)
        .execute(&mut **tx)
        .await
        .with_context(|| format!("drop deeper display hops on clear for {session_id}"))?;

    sqlx::query(
        "UPDATE session_lineage_edges \
         SET display_detached = TRUE, segment_id = NULL, edge_version = edge_version + 1 \
         WHERE child_session_id = $1 AND hop_order = 1 \
           AND (NOT display_detached OR segment_id IS NOT NULL)",
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
    if let Some(max_before) = max_before {
        seal_session_prefix_pg(tx, session_id, max_before, "clear", now_ms).await?;
    }
    gc_unreferenced_segments_pg(tx).await
}

/// Seal referenced child prefixes and mark duplicate receipts terminal when a
/// session is logically deleted from Trash (eliminating any gap during the
/// 3-day purge grace window).
pub(in crate::store) async fn seal_lineage_on_delete_pg(
    tx: &mut Transaction<'_, Postgres>,
    session_id: &str,
) -> Result<()> {
    let now_ms = Utc::now().timestamp_millis();
    sqlx::query(
        "UPDATE session_duplicate_receipts SET state = 'deleted', updated_at_ms = $2 \
         WHERE destination_session_id = $1 AND state <> 'deleted'",
    )
    .bind(session_id)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("mark deleted duplicate receipt for {session_id}"))?;

    let max_before: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(before_seq) FROM session_lineage_edges \
         WHERE parent_session_id = $1 AND segment_id IS NULL AND NOT display_detached",
    )
    .bind(session_id)
    .fetch_one(&mut **tx)
    .await
    .with_context(|| format!("check child lineage prefixes on delete for {session_id}"))?;
    if let Some(max_before) = max_before {
        seal_session_prefix_pg(tx, session_id, max_before, "delete", now_ms).await?;
    }
    Ok(())
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
         WHERE parent_session_id = $2 AND segment_id IS NULL AND NOT display_detached \
           AND before_seq <= $3",
    )
    .bind(&segment_id)
    .bind(session_id)
    .bind(max_before)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("attach archived segment {segment_id} to lineage edges"))?;
    Ok(())
}

async fn gc_unreferenced_segments_pg(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    sqlx::query(
        "DELETE FROM archived_lineage_segments \
         WHERE segment_id NOT IN ( \
           SELECT segment_id FROM session_lineage_edges WHERE segment_id IS NOT NULL \
         )",
    )
    .execute(&mut **tx)
    .await
    .context("GC unreferenced archived lineage segments")?;
    Ok(())
}

/// Purge expired soft-deleted sessions while archiving referenced lineage
/// prefixes for surviving descendants, revalidating retention protection at
/// deletion time, and separating display GC from auth-only edge retention.
pub(in crate::store) async fn purge_deleted_pg(
    storage: &PostgresStorage,
    retention_days: i64,
) -> Result<u64> {
    let now_ms = Utc::now().timestamp_millis();
    let days = i32::try_from(retention_days).unwrap_or(3);
    let mut tx = storage.pool.begin().await?;
    let purging: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM sessions WHERE deleted_at IS NOT NULL \
         AND COALESCE(purge_after_at, deleted_at + make_interval(days => $1::int)) < now() \
         AND NOT EXISTS (SELECT 1 FROM managed_agent_calls AS call \
             WHERE (call.parent_session_id = sessions.id OR call.child_session_id = sessions.id) \
             AND call.state NOT IN ('completed', 'failed', 'cancelled')) \
         AND NOT EXISTS (SELECT 1 FROM plugin_uninstall_operations AS operation \
             WHERE operation.machine_id = sessions.machine_id AND operation.plugin_id = sessions.provider \
             AND operation.phase NOT IN ('completed', 'compensated', 'aborted')) \
         FOR UPDATE",
    )
    .bind(days)
    .fetch_all(&mut *tx)
    .await
    .context("select expired soft-deleted sessions")?;

    let to_archive: Vec<(String, i64)> = if purging.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(
            "SELECT e.parent_session_id, MAX(e.before_seq) \
             FROM session_lineage_edges AS e \
             WHERE e.parent_session_id = ANY($1::text[]) \
               AND e.segment_id IS NULL \
               AND NOT e.display_detached \
               AND e.child_session_id IN ( \
                 SELECT id FROM sessions WHERE NOT (id = ANY($1::text[])) \
               ) \
             GROUP BY e.parent_session_id",
        )
        .bind(&purging)
        .fetch_all(&mut *tx)
        .await
        .context("compute display-reachable lineage ancestors before purge")?
    };

    for (session_id, max_before) in to_archive {
        seal_session_prefix_pg(&mut tx, &session_id, max_before, "purge", now_ms).await?;
    }

    let actually_purged: Vec<String> = if purging.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar(
            "DELETE FROM sessions WHERE id = ANY($1::text[]) \
             AND deleted_at IS NOT NULL \
             AND COALESCE(purge_after_at, deleted_at + make_interval(days => $2::int)) < now() \
             AND NOT EXISTS (SELECT 1 FROM managed_agent_calls AS call \
                 WHERE (call.parent_session_id = sessions.id OR call.child_session_id = sessions.id) \
                 AND call.state NOT IN ('completed', 'failed', 'cancelled')) \
             AND NOT EXISTS (SELECT 1 FROM plugin_uninstall_operations AS operation \
                 WHERE operation.machine_id = sessions.machine_id AND operation.plugin_id = sessions.provider \
                 AND operation.phase NOT IN ('completed', 'compensated', 'aborted')) \
             RETURNING id",
        )
        .bind(&purging)
        .bind(days)
        .fetch_all(&mut *tx)
        .await
        .context("purge soft-deleted sessions")?
    };
    let purged_count = u64::try_from(actually_purged.len()).unwrap_or(0);

    if !actually_purged.is_empty() {
        sqlx::query(
            "UPDATE session_duplicate_receipts SET state = 'deleted', updated_at_ms = $2 \
             WHERE destination_session_id = ANY($1::text[]) AND state <> 'deleted'",
        )
        .bind(&actually_purged)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .context("mark purged duplicate receipts deleted")?;
    }

    // Separate display reachability from auth reachability: auth-only edges
    // keep parent IDs without pinning archived transcript segments/artifacts.
    gc_lineage_edges_pg(&mut tx).await?;
    tx.commit().await?;
    prune_pg_artifacts(storage).await?;
    Ok(purged_count)
}

async fn gc_lineage_edges_pg(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<()> {
    sqlx::query(
        "UPDATE session_lineage_edges \
         SET segment_id = NULL, edge_version = edge_version + 1 \
         WHERE segment_id IS NOT NULL \
           AND (display_detached OR child_session_id NOT IN (SELECT id FROM sessions))",
    )
    .execute(&mut **tx)
    .await
    .context("detach segments from non-display-reachable lineage edges")?;

    sqlx::query(
        "DELETE FROM session_lineage_edges \
         WHERE hop_order > 1 \
           AND (display_detached OR child_session_id NOT IN (SELECT id FROM sessions))",
    )
    .execute(&mut **tx)
    .await
    .context("GC unreachable deeper lineage display edges")?;

    sqlx::query(
        "WITH RECURSIVE auth_reachable(id) AS ( \
           SELECT id FROM sessions \
           UNION \
           SELECT e.parent_session_id FROM session_lineage_edges AS e \
           JOIN auth_reachable AS r ON e.child_session_id = r.id \
           WHERE e.hop_order = 1 \
         ) \
         DELETE FROM session_lineage_edges \
         WHERE hop_order = 1 AND child_session_id NOT IN (SELECT id FROM auth_reachable)",
    )
    .execute(&mut **tx)
    .await
    .context("GC unreachable auth lineage edges")?;

    gc_unreferenced_segments_pg(tx).await?;
    Ok(())
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
