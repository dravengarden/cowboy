//! Durable session lineage edges and decoupled archived history segments.
//!
//! Each forked or replaced session records one durable `LineageEdge` pointing
//! to its immediate parent and the exclusive sequence cursor (`before_seq`)
//! captured at branch time. Clearing or purging a parent archives only the
//! referenced prefix (`seq < MAX(before_seq)`) into `archived_lineage_segments`
//! and `archived_lineage_events`, releasing the parent's `sessions` row and
//! retired provider auth generations cleanly.

mod explore;
mod pg;
mod sqlite;
mod turns_and_receipts;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use super::{StorageBackend, Store};
use crate::agent_model::Event;
use crate::model::{Envelope, bound_history_page};

static SEGMENT_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(super) use pg::{purge_deleted_pg, seal_lineage_on_clear_pg};
pub(super) use sqlite::{purge_deleted_sqlite, seal_lineage_on_clear_sqlite};
pub use turns_and_receipts::{DuplicateReceipt, TurnCompletion};

/// Durable provenance edge from a child session to its immediate parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageEdge {
    pub child_session_id: String,
    pub parent_session_id: String,
    pub owner_user_id: String,
    pub kind: String,
    pub before_seq: u64,
    pub source_title: String,
    pub source_provider: String,
    pub segment_id: Option<String>,
    pub display_detached: bool,
    pub edge_version: u64,
    pub created_at_ms: i64,
}

/// Decoupled archived prefix copied from a parent session on Clear or Purge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchivedLineageSegment {
    pub segment_id: String,
    pub source_session_id: String,
    pub owner_user_id: String,
    pub title: String,
    pub provider: String,
    pub before_seq: u64,
    pub reason: String,
    pub created_at_ms: i64,
}

/// One immutable page of ancestor history clamped to `edge.before_seq`.
#[derive(Debug, Clone)]
pub struct LineageHistoryPage {
    pub events: Vec<Envelope>,
    pub next_before_seq: Option<u64>,
    pub reached_start: bool,
    /// `true` iff the prefix `< edge.before_seq` is durably flushed/sealed and
    /// safe for immutable HTTP/Service Worker caching.
    pub is_sealed: bool,
    pub segment_id: Option<String>,
}

pub(super) fn next_segment_id(source_session_id: &str, now_ms: i64, reason: &str) -> String {
    let seq = SEGMENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("lseg-{source_session_id}-{now_ms}-{seq}-{reason}")
}

pub(super) fn decode_lineage_events(
    source_session_id: &str,
    rows: Vec<(i64, serde_json::Value)>,
) -> Vec<Envelope> {
    let mut events = Vec::with_capacity(rows.len());
    for (seq, mut payload) in rows.into_iter().rev() {
        let cmid = crate::persistence::take_persisted_cmid(&mut payload);
        match serde_json::from_value::<Event>(payload) {
            Ok(event) => events.push(Envelope {
                session_id: source_session_id.to_owned(),
                seq: u64::try_from(seq).unwrap_or(0),
                event,
                cmid,
            }),
            Err(error) => tracing::warn!(
                %error,
                session = %source_session_id,
                seq,
                "skipping undecodable lineage history event"
            ),
        }
    }
    bound_history_page(events)
}

pub(super) fn empty_sealed_page(segment_id: Option<String>) -> LineageHistoryPage {
    LineageHistoryPage {
        events: Vec::new(),
        next_before_seq: None,
        reached_start: true,
        is_sealed: true,
        segment_id,
    }
}

pub(super) fn to_i64_seq(seq: u64) -> Result<i64> {
    i64::try_from(seq).context("lineage sequence cursor overflow")
}

macro_rules! dispatch {
    ($store:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match &$store.backend {
            StorageBackend::Postgres(db) => db.$method($($arg),*).await,
            StorageBackend::Sqlite(db) => db.$method($($arg),*).await,
        }
    };
}

impl Store {
    /// Insert or update the lineage edge for `edge.child_session_id`.
    pub async fn upsert_lineage_edge(&self, edge: &LineageEdge) -> Result<()> {
        dispatch!(self, upsert_lineage_edge(edge))
    }

    /// Delete the lineage edge for `child_session_id` (used when Cancel Switch
    /// rolls back a newly created destination).
    pub async fn delete_lineage_edge(&self, child_session_id: &str) -> Result<bool> {
        dispatch!(self, delete_lineage_edge(child_session_id))
    }

    /// Fetch the immediate lineage edge for `child_session_id`, if any.
    pub async fn lineage_edge(&self, child_session_id: &str) -> Result<Option<LineageEdge>> {
        dispatch!(self, lineage_edge(child_session_id))
    }

    /// Load every persisted lineage edge (used at startup to warm auth ancestry
    /// and `EventReducer` freeze floors).
    pub async fn load_all_lineage_edges(&self) -> Result<Vec<LineageEdge>> {
        dispatch!(self, load_all_lineage_edges())
    }

    /// List direct children of `parent_session_id`, newest first.
    pub async fn lineage_children(&self, parent_session_id: &str) -> Result<Vec<LineageEdge>> {
        dispatch!(self, lineage_children(parent_session_id))
    }

    /// Fetch one archived lineage segment's metadata.
    pub async fn lineage_segment(
        &self,
        segment_id: &str,
    ) -> Result<Option<ArchivedLineageSegment>> {
        dispatch!(self, lineage_segment(segment_id))
    }

    /// Read one page of ancestor events strictly bounded by `edge.before_seq`.
    pub async fn lineage_history_page(
        &self,
        edge: &LineageEdge,
        before_seq: u64,
        page_size: usize,
    ) -> Result<LineageHistoryPage> {
        dispatch!(self, lineage_history_page(edge, before_seq, page_size))
    }
}
