//! Storage-level Explore question-page summaries and bounded detail reads for
//! ancestor lineage hops.
//!
//! Mirrors `Store::question_page_summaries` and `Store::question_page_at` in
//! SQL, strictly clamping `seq < edge.before_seq` on both `events` and
//! `archived_lineage_events` so arbitrarily long lineages keep exact ordinals,
//! exact totals, and clean cursor pagination without loading the full prefix.

use anyhow::{Context as _, Result};

use super::{LineageEdge, to_i64_seq};
use crate::agent_model::Event;
use crate::model::{
    Envelope, HISTORY_MAX_BYTES, QuestionPageSummary, estimated_envelope_bytes,
    question_summary_title,
};
use crate::store::{PostgresStorage, SqliteStorage, StorageBackend, Store};

const PG_LIVE_SUMMARIES_SQL: &str = "\
WITH ordered AS ( \
  SELECT seq, payload, \
    LAG(payload->'update'->>'sessionUpdate') OVER (ORDER BY seq) AS previous_update \
  FROM events WHERE session_id = $1 AND seq < $2 \
), roots AS ( \
  SELECT seq, COALESCE(payload->'update'->'content'->>'text', '') AS title \
  FROM ordered \
  WHERE payload->>'kind' = 'update' \
    AND payload->'update'->>'sessionUpdate' = 'user_message_chunk' \
    AND COALESCE(payload->'update'->'_meta'->>'cowboyDelivery', '') <> 'current_turn' \
    AND COALESCE(payload->'update'->>'autoResumed', 'false') <> 'true' \
    AND COALESCE(payload->'update'->'promptOrigin'->>'actor', 'human') = 'human' \
    AND POSITION('<system-reminder' IN LOWER(COALESCE(payload->'update'->'content'->>'text', ''))) = 0 \
    AND BTRIM(COALESCE(payload->'update'->'content'->>'text', '')) NOT IN ('/compact', '/compress') \
    AND previous_update IS DISTINCT FROM 'user_message_chunk' \
), numbered AS ( \
  SELECT seq, title, \
    ROW_NUMBER() OVER (ORDER BY seq) AS ordinal, \
    COUNT(*) OVER () AS total \
  FROM roots \
) \
SELECT seq, title, ordinal, total FROM numbered \
WHERE ($3::bigint IS NULL OR seq < $3) \
ORDER BY seq DESC LIMIT $4";

const PG_ARCHIVED_SUMMARIES_SQL: &str = "\
WITH ordered AS ( \
  SELECT seq, payload, \
    LAG(payload->'update'->>'sessionUpdate') OVER (ORDER BY seq) AS previous_update \
  FROM archived_lineage_events WHERE segment_id = $1 AND seq < $2 \
), roots AS ( \
  SELECT seq, COALESCE(payload->'update'->'content'->>'text', '') AS title \
  FROM ordered \
  WHERE payload->>'kind' = 'update' \
    AND payload->'update'->>'sessionUpdate' = 'user_message_chunk' \
    AND COALESCE(payload->'update'->'_meta'->>'cowboyDelivery', '') <> 'current_turn' \
    AND COALESCE(payload->'update'->>'autoResumed', 'false') <> 'true' \
    AND COALESCE(payload->'update'->'promptOrigin'->>'actor', 'human') = 'human' \
    AND POSITION('<system-reminder' IN LOWER(COALESCE(payload->'update'->'content'->>'text', ''))) = 0 \
    AND BTRIM(COALESCE(payload->'update'->'content'->>'text', '')) NOT IN ('/compact', '/compress') \
    AND previous_update IS DISTINCT FROM 'user_message_chunk' \
), numbered AS ( \
  SELECT seq, title, \
    ROW_NUMBER() OVER (ORDER BY seq) AS ordinal, \
    COUNT(*) OVER () AS total \
  FROM roots \
) \
SELECT seq, title, ordinal, total FROM numbered \
WHERE ($3::bigint IS NULL OR seq < $3) \
ORDER BY seq DESC LIMIT $4";

const PG_LIVE_BOUNDS_SQL: &str = "\
WITH ordered AS ( \
  SELECT seq, payload, \
    LAG(payload->'update'->>'sessionUpdate') OVER (ORDER BY seq) AS previous_update \
  FROM events WHERE session_id = $1 AND seq < $2 \
), roots AS ( \
  SELECT seq FROM ordered \
  WHERE payload->>'kind' = 'update' \
    AND payload->'update'->>'sessionUpdate' = 'user_message_chunk' \
    AND COALESCE(payload->'update'->'_meta'->>'cowboyDelivery', '') <> 'current_turn' \
    AND COALESCE(payload->'update'->>'autoResumed', 'false') <> 'true' \
    AND COALESCE(payload->'update'->'promptOrigin'->>'actor', 'human') = 'human' \
    AND POSITION('<system-reminder' IN LOWER(COALESCE(payload->'update'->'content'->>'text', ''))) = 0 \
    AND BTRIM(COALESCE(payload->'update'->'content'->>'text', '')) NOT IN ('/compact', '/compress') \
    AND previous_update IS DISTINCT FROM 'user_message_chunk' \
), bounded AS ( \
  SELECT seq, LEAD(seq) OVER (ORDER BY seq) AS next_seq FROM roots \
) \
SELECT seq, next_seq FROM bounded WHERE seq = $3";

const PG_ARCHIVED_BOUNDS_SQL: &str = "\
WITH ordered AS ( \
  SELECT seq, payload, \
    LAG(payload->'update'->>'sessionUpdate') OVER (ORDER BY seq) AS previous_update \
  FROM archived_lineage_events WHERE segment_id = $1 AND seq < $2 \
), roots AS ( \
  SELECT seq FROM ordered \
  WHERE payload->>'kind' = 'update' \
    AND payload->'update'->>'sessionUpdate' = 'user_message_chunk' \
    AND COALESCE(payload->'update'->'_meta'->>'cowboyDelivery', '') <> 'current_turn' \
    AND COALESCE(payload->'update'->>'autoResumed', 'false') <> 'true' \
    AND COALESCE(payload->'update'->'promptOrigin'->>'actor', 'human') = 'human' \
    AND POSITION('<system-reminder' IN LOWER(COALESCE(payload->'update'->'content'->>'text', ''))) = 0 \
    AND BTRIM(COALESCE(payload->'update'->'content'->>'text', '')) NOT IN ('/compact', '/compress') \
    AND previous_update IS DISTINCT FROM 'user_message_chunk' \
), bounded AS ( \
  SELECT seq, LEAD(seq) OVER (ORDER BY seq) AS next_seq FROM roots \
) \
SELECT seq, next_seq FROM bounded WHERE seq = $3";

const SQLITE_LIVE_SUMMARIES_SQL: &str = "\
WITH ordered AS ( \
  SELECT seq, payload, \
    LAG(json_extract(payload, '$.update.sessionUpdate')) OVER (ORDER BY seq) AS previous_update \
  FROM events WHERE session_id = ?1 AND seq < ?2 \
), roots AS ( \
  SELECT seq, COALESCE(json_extract(payload, '$.update.content.text'), '') AS title \
  FROM ordered \
  WHERE json_extract(payload, '$.kind') = 'update' \
    AND json_extract(payload, '$.update.sessionUpdate') = 'user_message_chunk' \
    AND COALESCE(json_extract(payload, '$.update._meta.cowboyDelivery'), '') <> 'current_turn' \
    AND COALESCE(json_extract(payload, '$.update.autoResumed'), 0) <> 1 \
    AND COALESCE(json_extract(payload, '$.update.promptOrigin.actor'), 'human') = 'human' \
    AND instr(lower(COALESCE(json_extract(payload, '$.update.content.text'), '')), '<system-reminder') = 0 \
    AND trim(COALESCE(json_extract(payload, '$.update.content.text'), '')) NOT IN ('/compact', '/compress') \
    AND (previous_update IS NULL OR previous_update <> 'user_message_chunk') \
), numbered AS ( \
  SELECT seq, title, \
    ROW_NUMBER() OVER (ORDER BY seq) AS ordinal, \
    COUNT(*) OVER () AS total \
  FROM roots \
) \
SELECT seq, title, ordinal, total FROM numbered \
WHERE (?3 IS NULL OR seq < ?3) \
ORDER BY seq DESC LIMIT ?4";

const SQLITE_ARCHIVED_SUMMARIES_SQL: &str = "\
WITH ordered AS ( \
  SELECT seq, payload, \
    LAG(json_extract(payload, '$.update.sessionUpdate')) OVER (ORDER BY seq) AS previous_update \
  FROM archived_lineage_events WHERE segment_id = ?1 AND seq < ?2 \
), roots AS ( \
  SELECT seq, COALESCE(json_extract(payload, '$.update.content.text'), '') AS title \
  FROM ordered \
  WHERE json_extract(payload, '$.kind') = 'update' \
    AND json_extract(payload, '$.update.sessionUpdate') = 'user_message_chunk' \
    AND COALESCE(json_extract(payload, '$.update._meta.cowboyDelivery'), '') <> 'current_turn' \
    AND COALESCE(json_extract(payload, '$.update.autoResumed'), 0) <> 1 \
    AND COALESCE(json_extract(payload, '$.update.promptOrigin.actor'), 'human') = 'human' \
    AND instr(lower(COALESCE(json_extract(payload, '$.update.content.text'), '')), '<system-reminder') = 0 \
    AND trim(COALESCE(json_extract(payload, '$.update.content.text'), '')) NOT IN ('/compact', '/compress') \
    AND (previous_update IS NULL OR previous_update <> 'user_message_chunk') \
), numbered AS ( \
  SELECT seq, title, \
    ROW_NUMBER() OVER (ORDER BY seq) AS ordinal, \
    COUNT(*) OVER () AS total \
  FROM roots \
) \
SELECT seq, title, ordinal, total FROM numbered \
WHERE (?3 IS NULL OR seq < ?3) \
ORDER BY seq DESC LIMIT ?4";

const SQLITE_LIVE_BOUNDS_SQL: &str = "\
WITH ordered AS ( \
  SELECT seq, payload, \
    LAG(json_extract(payload, '$.update.sessionUpdate')) OVER (ORDER BY seq) AS previous_update \
  FROM events WHERE session_id = ?1 AND seq < ?2 \
), roots AS ( \
  SELECT seq FROM ordered \
  WHERE json_extract(payload, '$.kind') = 'update' \
    AND json_extract(payload, '$.update.sessionUpdate') = 'user_message_chunk' \
    AND COALESCE(json_extract(payload, '$.update._meta.cowboyDelivery'), '') <> 'current_turn' \
    AND COALESCE(json_extract(payload, '$.update.autoResumed'), 0) <> 1 \
    AND COALESCE(json_extract(payload, '$.update.promptOrigin.actor'), 'human') = 'human' \
    AND instr(lower(COALESCE(json_extract(payload, '$.update.content.text'), '')), '<system-reminder') = 0 \
    AND trim(COALESCE(json_extract(payload, '$.update.content.text'), '')) NOT IN ('/compact', '/compress') \
    AND (previous_update IS NULL OR previous_update <> 'user_message_chunk') \
), bounded AS ( \
  SELECT seq, LEAD(seq) OVER (ORDER BY seq) AS next_seq FROM roots \
) \
SELECT seq, next_seq FROM bounded WHERE seq = ?3";

const SQLITE_ARCHIVED_BOUNDS_SQL: &str = "\
WITH ordered AS ( \
  SELECT seq, payload, \
    LAG(json_extract(payload, '$.update.sessionUpdate')) OVER (ORDER BY seq) AS previous_update \
  FROM archived_lineage_events WHERE segment_id = ?1 AND seq < ?2 \
), roots AS ( \
  SELECT seq FROM ordered \
  WHERE json_extract(payload, '$.kind') = 'update' \
    AND json_extract(payload, '$.update.sessionUpdate') = 'user_message_chunk' \
    AND COALESCE(json_extract(payload, '$.update._meta.cowboyDelivery'), '') <> 'current_turn' \
    AND COALESCE(json_extract(payload, '$.update.autoResumed'), 0) <> 1 \
    AND COALESCE(json_extract(payload, '$.update.promptOrigin.actor'), 'human') = 'human' \
    AND instr(lower(COALESCE(json_extract(payload, '$.update.content.text'), '')), '<system-reminder') = 0 \
    AND trim(COALESCE(json_extract(payload, '$.update.content.text'), '')) NOT IN ('/compact', '/compress') \
    AND (previous_update IS NULL OR previous_update <> 'user_message_chunk') \
), bounded AS ( \
  SELECT seq, LEAD(seq) OVER (ORDER BY seq) AS next_seq FROM roots \
) \
SELECT seq, next_seq FROM bounded WHERE seq = ?3";

fn finish_summary_rows(
    rows: Vec<(i64, String, i64, i64)>,
) -> (Vec<QuestionPageSummary>, Option<u64>, u64) {
    let total = rows
        .first()
        .map_or(0, |row| u64::try_from(row.3).unwrap_or(0));
    let next_before_seq = rows
        .last()
        .and_then(|row| (row.2 > 1).then(|| u64::try_from(row.0).unwrap_or(0)));
    let mut pages: Vec<QuestionPageSummary> = rows
        .into_iter()
        .map(|(seq, title, ordinal, _)| {
            let ordinal = u64::try_from(ordinal).unwrap_or(u64::MAX);
            QuestionPageSummary {
                id: u64::try_from(seq).unwrap_or(0),
                title: question_summary_title(&title, ordinal),
                ordinal,
            }
        })
        .collect();
    pages.reverse();
    (pages, next_before_seq, total)
}

impl Store {
    /// Summarize question pages in an ancestor hop strictly `< edge.before_seq`.
    pub async fn lineage_question_pages(
        &self,
        edge: &LineageEdge,
        before: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<QuestionPageSummary>, Option<u64>, u64)> {
        if edge.before_seq == 0 {
            return Ok((Vec::new(), None, 0));
        }
        match &self.backend {
            StorageBackend::Postgres(db) => db.lineage_question_pages(edge, before, limit).await,
            StorageBackend::Sqlite(db) => db.lineage_question_pages(edge, before, limit).await,
        }
    }

    /// Read one bounded question page in an ancestor hop strictly `< edge.before_seq`,
    /// preserving the opening question root, honoring the turn-end boundary, and
    /// returning a forward continuation cursor when truncated by row/byte budget.
    pub async fn lineage_question_page_at(
        &self,
        edge: &LineageEdge,
        page_id: u64,
        after_seq: Option<u64>,
    ) -> Result<Option<LineageQuestionPageDetail>> {
        if page_id >= edge.before_seq {
            return Ok(None);
        }
        match &self.backend {
            StorageBackend::Postgres(db) => {
                db.lineage_question_page_at(edge, page_id, after_seq).await
            }
            StorageBackend::Sqlite(db) => {
                db.lineage_question_page_at(edge, page_id, after_seq).await
            }
        }
    }
}

/// Bounded question-page detail preserving the question root and returning a
/// continuation cursor (`next_after_seq`) when a long turn exceeds one page.
#[derive(Debug, Clone)]
pub struct LineageQuestionPageDetail {
    pub events: Vec<Envelope>,
    pub next_after_seq: Option<u64>,
    pub reached_start: bool,
}

const MAX_QUESTION_PAGE_ROWS: i64 = 2000;

fn decode_question_page_asc(
    session_id: &str,
    rows: Vec<(i64, serde_json::Value)>,
    has_more_rows: bool,
    reached_start: bool,
) -> LineageQuestionPageDetail {
    let mut events = Vec::with_capacity(rows.len());
    let mut bytes = 0usize;
    let mut truncated_by_bytes = false;
    for (seq, mut payload) in rows {
        let Ok(seq_u64) = u64::try_from(seq) else {
            continue;
        };
        let cmid = payload
            .as_object_mut()
            .and_then(|obj| obj.remove("cmid"))
            .and_then(|value| value.as_str().map(str::to_owned));
        let Ok(event) = serde_json::from_value::<Event>(payload) else {
            continue;
        };
        let env = Envelope {
            session_id: session_id.to_owned(),
            seq: seq_u64,
            event,
            cmid,
        };
        let size = estimated_envelope_bytes(&env);
        if !events.is_empty() && bytes.saturating_add(size) > HISTORY_MAX_BYTES {
            truncated_by_bytes = true;
            break;
        }
        bytes = bytes.saturating_add(size);
        events.push(env);
    }
    let next_after_seq = if truncated_by_bytes || has_more_rows {
        events.last().map(|env| env.seq.saturating_add(1))
    } else {
        None
    };
    LineageQuestionPageDetail {
        events,
        next_after_seq,
        reached_start,
    }
}

impl PostgresStorage {
    pub(super) async fn lineage_question_pages(
        &self,
        edge: &LineageEdge,
        before: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<QuestionPageSummary>, Option<u64>, u64)> {
        let cutoff = to_i64_seq(edge.before_seq)?;
        let cursor = before.map(to_i64_seq).transpose()?;
        let lim = i64::try_from(limit.clamp(1, 200)).unwrap_or(64);
        let (sql, key) = match edge.segment_id.as_deref() {
            Some(seg) => (PG_ARCHIVED_SUMMARIES_SQL, seg),
            None => (PG_LIVE_SUMMARIES_SQL, edge.parent_session_id.as_str()),
        };
        let rows: Vec<(i64, String, i64, i64)> = sqlx::query_as(sql)
            .bind(key)
            .bind(cutoff)
            .bind(cursor)
            .bind(lim)
            .fetch_all(&self.pool)
            .await
            .context("SELECT PG lineage question page summaries")?;
        Ok(finish_summary_rows(rows))
    }

    pub(super) async fn lineage_question_page_at(
        &self,
        edge: &LineageEdge,
        page_id: u64,
        after_seq: Option<u64>,
    ) -> Result<Option<LineageQuestionPageDetail>> {
        let cutoff = to_i64_seq(edge.before_seq)?;
        let root = to_i64_seq(page_id)?;
        let (bounds_sql, key) = match edge.segment_id.as_deref() {
            Some(seg) => (PG_ARCHIVED_BOUNDS_SQL, seg),
            None => (PG_LIVE_BOUNDS_SQL, edge.parent_session_id.as_str()),
        };
        let bounds: Option<(i64, Option<i64>)> = sqlx::query_as(bounds_sql)
            .bind(key)
            .bind(cutoff)
            .bind(root)
            .fetch_optional(&self.pool)
            .await
            .context("SELECT PG lineage question page bounds")?;
        let Some((start, next_root)) = bounds else {
            return Ok(None);
        };
        let bound_upper = next_root.map_or(cutoff, |n| n.min(cutoff));
        let turn_end: Option<i64> = if let Some(seg) = edge.segment_id.as_deref() {
            sqlx::query_scalar(
                "SELECT MIN(seq) FROM archived_lineage_events                  WHERE segment_id = $1 AND seq >= $2 AND seq < $3 AND payload->>'kind' = 'turn_end'",
            )
            .bind(seg)
            .bind(start)
            .bind(bound_upper)
            .fetch_one(&self.pool)
            .await
            .context("SELECT PG archived question turn end")?
        } else {
            sqlx::query_scalar(
                "SELECT MIN(seq) FROM events                  WHERE session_id = $1 AND seq >= $2 AND seq < $3 AND payload->>'kind' = 'turn_end'",
            )
            .bind(&edge.parent_session_id)
            .bind(start)
            .bind(bound_upper)
            .fetch_one(&self.pool)
            .await
            .context("SELECT PG live question turn end")?
        };
        let upper = turn_end
            .and_then(|s| s.checked_add(1))
            .map_or(bound_upper, |e| e.min(bound_upper));
        let lower = after_seq
            .map(to_i64_seq)
            .transpose()?
            .map_or(start, |a| a.max(start));
        let mut rows: Vec<(i64, serde_json::Value)> = if let Some(seg) = edge.segment_id.as_deref()
        {
            sqlx::query_as(
                "SELECT seq, payload FROM archived_lineage_events                  WHERE segment_id = $1 AND seq >= $2 AND seq < $3                  ORDER BY seq ASC LIMIT $4",
            )
            .bind(seg)
            .bind(lower)
            .bind(upper)
            .bind(MAX_QUESTION_PAGE_ROWS + 1)
            .fetch_all(&self.pool)
            .await
            .context("SELECT PG archived lineage question page")?
        } else {
            sqlx::query_as(
                "SELECT seq, payload FROM events                  WHERE session_id = $1 AND seq >= $2 AND seq < $3                  ORDER BY seq ASC LIMIT $4",
            )
            .bind(&edge.parent_session_id)
            .bind(lower)
            .bind(upper)
            .bind(MAX_QUESTION_PAGE_ROWS + 1)
            .fetch_all(&self.pool)
            .await
            .context("SELECT PG live lineage question page")?
        };
        let has_more = i64::try_from(rows.len()).unwrap_or(0) > MAX_QUESTION_PAGE_ROWS;
        if has_more {
            rows.truncate(usize::try_from(MAX_QUESTION_PAGE_ROWS).unwrap_or(2000));
        }
        Ok(Some(decode_question_page_asc(
            &edge.parent_session_id,
            rows,
            has_more,
            lower == start,
        )))
    }
}

impl SqliteStorage {
    pub(super) async fn lineage_question_pages(
        &self,
        edge: &LineageEdge,
        before: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<QuestionPageSummary>, Option<u64>, u64)> {
        let cutoff = to_i64_seq(edge.before_seq)?;
        let cursor = before.map(to_i64_seq).transpose()?;
        let lim = i64::try_from(limit.clamp(1, 200)).unwrap_or(64);
        let (sql, key) = match edge.segment_id.as_deref() {
            Some(seg) => (SQLITE_ARCHIVED_SUMMARIES_SQL, seg),
            None => (SQLITE_LIVE_SUMMARIES_SQL, edge.parent_session_id.as_str()),
        };
        let rows: Vec<(i64, String, i64, i64)> = sqlx::query_as(sql)
            .bind(key)
            .bind(cutoff)
            .bind(cursor)
            .bind(lim)
            .fetch_all(&self.pool)
            .await
            .context("SELECT SQLite lineage question page summaries")?;
        Ok(finish_summary_rows(rows))
    }

    pub(super) async fn lineage_question_page_at(
        &self,
        edge: &LineageEdge,
        page_id: u64,
        after_seq: Option<u64>,
    ) -> Result<Option<LineageQuestionPageDetail>> {
        let cutoff = to_i64_seq(edge.before_seq)?;
        let root = to_i64_seq(page_id)?;
        let (bounds_sql, key) = match edge.segment_id.as_deref() {
            Some(seg) => (SQLITE_ARCHIVED_BOUNDS_SQL, seg),
            None => (SQLITE_LIVE_BOUNDS_SQL, edge.parent_session_id.as_str()),
        };
        let bounds: Option<(i64, Option<i64>)> = sqlx::query_as(bounds_sql)
            .bind(key)
            .bind(cutoff)
            .bind(root)
            .fetch_optional(&self.pool)
            .await
            .context("SELECT SQLite lineage question page bounds")?;
        let Some((start, next_root)) = bounds else {
            return Ok(None);
        };
        let bound_upper = next_root.map_or(cutoff, |n| n.min(cutoff));
        let turn_end: Option<i64> = if let Some(seg) = edge.segment_id.as_deref() {
            sqlx::query_scalar(
                "SELECT MIN(seq) FROM archived_lineage_events                  WHERE segment_id = ?1 AND seq >= ?2 AND seq < ?3                    AND json_extract(payload, '$.kind') = 'turn_end'",
            )
            .bind(seg)
            .bind(start)
            .bind(bound_upper)
            .fetch_one(&self.pool)
            .await
            .context("SELECT SQLite archived question turn end")?
        } else {
            sqlx::query_scalar(
                "SELECT MIN(seq) FROM events                  WHERE session_id = ?1 AND seq >= ?2 AND seq < ?3                    AND json_extract(payload, '$.kind') = 'turn_end'",
            )
            .bind(&edge.parent_session_id)
            .bind(start)
            .bind(bound_upper)
            .fetch_one(&self.pool)
            .await
            .context("SELECT SQLite live question turn end")?
        };
        let upper = turn_end
            .and_then(|s| s.checked_add(1))
            .map_or(bound_upper, |e| e.min(bound_upper));
        let lower = after_seq
            .map(to_i64_seq)
            .transpose()?
            .map_or(start, |a| a.max(start));
        let mut raw: Vec<(i64, String)> = if let Some(seg) = edge.segment_id.as_deref() {
            sqlx::query_as(
                "SELECT seq, payload FROM archived_lineage_events                  WHERE segment_id = ?1 AND seq >= ?2 AND seq < ?3                  ORDER BY seq ASC LIMIT ?4",
            )
            .bind(seg)
            .bind(lower)
            .bind(upper)
            .bind(MAX_QUESTION_PAGE_ROWS + 1)
            .fetch_all(&self.pool)
            .await
            .context("SELECT SQLite archived lineage question page")?
        } else {
            sqlx::query_as(
                "SELECT seq, payload FROM events                  WHERE session_id = ?1 AND seq >= ?2 AND seq < ?3                  ORDER BY seq ASC LIMIT ?4",
            )
            .bind(&edge.parent_session_id)
            .bind(lower)
            .bind(upper)
            .bind(MAX_QUESTION_PAGE_ROWS + 1)
            .fetch_all(&self.pool)
            .await
            .context("SELECT SQLite live lineage question page")?
        };
        let has_more = i64::try_from(raw.len()).unwrap_or(0) > MAX_QUESTION_PAGE_ROWS;
        if has_more {
            raw.truncate(usize::try_from(MAX_QUESTION_PAGE_ROWS).unwrap_or(2000));
        }
        let rows = raw
            .into_iter()
            .filter_map(|(seq, text)| {
                serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .map(|val| (seq, val))
            })
            .collect();
        Ok(Some(decode_question_page_asc(
            &edge.parent_session_id,
            rows,
            has_more,
            lower == start,
        )))
    }
}
