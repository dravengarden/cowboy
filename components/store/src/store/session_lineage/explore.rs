//! Bounded question-page summaries and detail reads for ancestor lineage hops.
//!
//! Strictly enforces `seq < edge.before_seq` on the server so Explore and
//! Search never leak post-cutoff turns from a parent session.

use anyhow::{Context as _, Result};

use super::{LineageEdge, decode_lineage_events, to_i64_seq};
use crate::agent_model::Event;
use crate::model::{Envelope, QuestionPageSummary, question_summary_title};
use crate::store::{PostgresStorage, SqliteStorage, StorageBackend, Store};

fn root_user_question_text(event: &Event) -> Option<&str> {
    let Event::Update { update } = event else {
        return None;
    };
    if update
        .get("sessionUpdate")
        .and_then(serde_json::Value::as_str)
        != Some("user_message_chunk")
    {
        return None;
    }
    if update
        .pointer("/_meta/cowboyDelivery")
        .and_then(serde_json::Value::as_str)
        == Some("current_turn")
    {
        return None;
    }
    let text = update
        .pointer("/content/text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if text.starts_with("<system-reminder>")
        || !crate::prompt_origin::is_human_prompt_update(update)
    {
        return None;
    }
    Some(text)
}

fn summarize_questions(
    events: &[Envelope],
    before: Option<u64>,
    limit: usize,
) -> (Vec<QuestionPageSummary>, Option<u64>, u64) {
    let mut all = Vec::new();
    for env in events {
        if let Some(text) = root_user_question_text(&env.event) {
            all.push((env.seq, text));
        }
    }
    let total = u64::try_from(all.len()).unwrap_or(u64::MAX);
    let numbered: Vec<QuestionPageSummary> = all
        .into_iter()
        .enumerate()
        .map(|(idx, (id, text))| {
            let ordinal = u64::try_from(idx + 1).unwrap_or(u64::MAX);
            QuestionPageSummary {
                id,
                title: question_summary_title(text, ordinal),
                ordinal,
            }
        })
        .collect();
    let eligible: Vec<QuestionPageSummary> = match before {
        Some(cursor) => numbered.into_iter().filter(|p| p.id < cursor).collect(),
        None => numbered,
    };
    let split = eligible.len().saturating_sub(limit);
    let pages = eligible[split..].to_vec();
    let next_before = (split > 0).then(|| pages.first().map(|p| p.id)).flatten();
    (pages, next_before, total)
}

fn slice_question_page(events: &[Envelope], page_id: u64) -> Option<Vec<Envelope>> {
    let start = events.iter().position(|env| env.seq == page_id)?;
    root_user_question_text(&events[start].event)?;
    let end = events[start + 1..]
        .iter()
        .position(|env| root_user_question_text(&env.event).is_some())
        .map_or(events.len(), |rel| start + 1 + rel);
    Some(events[start..end].to_vec())
}

impl Store {
    /// Summarize question pages in an ancestor hop strictly `< edge.before_seq`.
    pub async fn lineage_question_pages(
        &self,
        edge: &LineageEdge,
        before: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<QuestionPageSummary>, Option<u64>, u64)> {
        let events = self.load_bounded_lineage_events(edge).await?;
        Ok(summarize_questions(&events, before, limit))
    }

    /// Read one question page in an ancestor hop, returning `None` if `page_id`
    /// is at or beyond `edge.before_seq` or not a question start.
    pub async fn lineage_question_page_at(
        &self,
        edge: &LineageEdge,
        page_id: u64,
    ) -> Result<Option<Vec<Envelope>>> {
        if page_id >= edge.before_seq {
            return Ok(None);
        }
        let events = self.load_bounded_lineage_events(edge).await?;
        Ok(slice_question_page(&events, page_id))
    }

    async fn load_bounded_lineage_events(&self, edge: &LineageEdge) -> Result<Vec<Envelope>> {
        if edge.before_seq == 0 {
            return Ok(Vec::new());
        }
        match &self.backend {
            StorageBackend::Postgres(db) => db.load_bounded_lineage_events(edge).await,
            StorageBackend::Sqlite(db) => db.load_bounded_lineage_events(edge).await,
        }
    }
}

/// Cap single-hop scan rows to stay bounded in memory on very long sessions.
const MAX_LINEAGE_EXPLORE_ROWS: i64 = 4_000;

impl PostgresStorage {
    pub(super) async fn load_bounded_lineage_events(
        &self,
        edge: &LineageEdge,
    ) -> Result<Vec<Envelope>> {
        let before_i64 = to_i64_seq(edge.before_seq)?;
        let rows: Vec<(i64, serde_json::Value)> =
            if let Some(segment_id) = edge.segment_id.as_deref() {
                sqlx::query_as(
                    "SELECT seq, payload FROM archived_lineage_events \
                 WHERE segment_id = $1 AND seq < $2 ORDER BY seq DESC LIMIT $3",
                )
                .bind(segment_id)
                .bind(before_i64)
                .bind(MAX_LINEAGE_EXPLORE_ROWS)
                .fetch_all(&self.pool)
                .await
                .context("SELECT archived lineage explore rows")?
            } else {
                sqlx::query_as(
                    "SELECT seq, payload FROM events \
                 WHERE session_id = $1 AND seq < $2 ORDER BY seq DESC LIMIT $3",
                )
                .bind(&edge.parent_session_id)
                .bind(before_i64)
                .bind(MAX_LINEAGE_EXPLORE_ROWS)
                .fetch_all(&self.pool)
                .await
                .context("SELECT live lineage explore rows")?
            };
        Ok(decode_lineage_events(&edge.parent_session_id, rows))
    }
}

impl SqliteStorage {
    pub(super) async fn load_bounded_lineage_events(
        &self,
        edge: &LineageEdge,
    ) -> Result<Vec<Envelope>> {
        let before_i64 = to_i64_seq(edge.before_seq)?;
        let raw: Vec<(i64, String)> = if let Some(segment_id) = edge.segment_id.as_deref() {
            sqlx::query_as(
                "SELECT seq, payload FROM archived_lineage_events \
                 WHERE segment_id = ?1 AND seq < ?2 ORDER BY seq DESC LIMIT ?3",
            )
            .bind(segment_id)
            .bind(before_i64)
            .bind(MAX_LINEAGE_EXPLORE_ROWS)
            .fetch_all(&self.pool)
            .await
            .context("SELECT SQLite archived lineage explore rows")?
        } else {
            sqlx::query_as(
                "SELECT seq, payload FROM events \
                 WHERE session_id = ?1 AND seq < ?2 ORDER BY seq DESC LIMIT ?3",
            )
            .bind(&edge.parent_session_id)
            .bind(before_i64)
            .bind(MAX_LINEAGE_EXPLORE_ROWS)
            .fetch_all(&self.pool)
            .await
            .context("SELECT SQLite live lineage explore rows")?
        };
        let rows = raw
            .into_iter()
            .filter_map(|(seq, text)| {
                serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .map(|val| (seq, val))
            })
            .collect();
        Ok(decode_lineage_events(&edge.parent_session_id, rows))
    }
}
