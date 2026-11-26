//! Durable session lineage edges and decoupled archived history segments.
//!
//! Each forked or replaced session records its immediate parent (`hop_order = 1`)
//! plus a materialized snapshot of its parent's active display hops
//! (`hop_order = 2..=MAX_DISPLAY_HOPS`). Clearing an intermediate branch detaches
//! only that branch's own display view while preserving existing descendants'
//! materialized hops. Auth ancestry walks `hop_order = 1` edges with explicit
//! `parent_is_root` tracking so purged pre-migration intermediates fail closed.

mod explore;
mod pg;
mod sqlite;
#[cfg(test)]
mod tests;
mod turns_and_receipts;

use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use super::{StorageBackend, Store};
use crate::agent_model::Event;
use crate::model::{Envelope, bound_history_page};

static SEGMENT_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Maximum display hops materialized onto a child session.
pub const MAX_MATERIALIZED_HOPS: u32 = 16;

pub use explore::LineageQuestionPageDetail;
pub(super) use pg::{purge_deleted_pg, seal_lineage_on_clear_pg, seal_lineage_on_delete_pg};
pub(super) use sqlite::{
    purge_deleted_sqlite, seal_lineage_on_clear_sqlite, seal_lineage_on_delete_sqlite,
};
pub use turns_and_receipts::{DuplicateReceipt, ReserveOutcome, TurnCompletion};

/// Durable provenance edge from a child session to an ancestor (`hop_order = 1`
/// is the immediate parent and authorization link; `hop_order >= 2` are
/// materialized display hops copied at branch creation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageEdge {
    pub child_session_id: String,
    pub hop_order: u32,
    pub parent_session_id: String,
    pub parent_is_root: bool,
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

/// Durable display lineage for one child session, distinguishing a legacy
/// session (no rows at all) from an explicitly detached session (Clear or
/// excluded Replace).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildDisplayLineage {
    pub has_durable_edge: bool,
    pub hops: Vec<LineageEdge>,
}

/// Decoupled archived prefix copied from a parent session on Clear, logical
/// delete, or Trash purge.
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
    pub edge_version: u64,
}

/// Atomic snapshot outcome for `GET /api/sessions/{id}/lineage/{hop}/history`.
#[derive(Debug, Clone)]
pub enum LineageHistoryOutcome {
    Page(LineageHistoryPage),
    NotFound,
    VersionConflict { current_version: u64 },
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

pub(super) fn empty_sealed_page(
    segment_id: Option<String>,
    edge_version: u64,
) -> LineageHistoryPage {
    LineageHistoryPage {
        events: Vec::new(),
        next_before_seq: None,
        reached_start: true,
        is_sealed: true,
        segment_id,
        edge_version,
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
    /// Insert or update the immediate (`hop_order = 1`) lineage edge for
    /// `edge.child_session_id` and snapshot the parent's active display hops
    /// onto the child inside one transaction.
    pub async fn upsert_lineage_edge(&self, edge: &LineageEdge) -> Result<()> {
        dispatch!(self, upsert_lineage_edge(edge))
    }

    /// Delete every lineage edge row owned by `child_session_id` (Cancel Switch).
    pub async fn delete_lineage_edge(&self, child_session_id: &str) -> Result<bool> {
        dispatch!(self, delete_lineage_edge(child_session_id))
    }

    /// Fetch the immediate (`hop_order = 1`) lineage edge for `child_session_id`.
    pub async fn lineage_edge(&self, child_session_id: &str) -> Result<Option<LineageEdge>> {
        dispatch!(self, lineage_edge(child_session_id))
    }

    /// Fetch the materialized active display hops for `child_session_id` and
    /// whether any durable edge row exists at all (distinguishing detached from
    /// legacy pre-migration sessions).
    pub async fn child_display_lineage(
        &self,
        child_session_id: &str,
    ) -> Result<ChildDisplayLineage> {
        dispatch!(self, child_display_lineage(child_session_id))
    }

    /// Load every persisted lineage edge (startup auth ancestry & freeze floors).
    pub async fn load_all_lineage_edges(&self) -> Result<Vec<LineageEdge>> {
        dispatch!(self, load_all_lineage_edges())
    }

    /// List direct (`hop_order = 1`) children of `parent_session_id`, newest first.
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

    /// Whether durable live `events` rows exist for `session_id` `< before_seq`.
    pub async fn session_has_live_events_before(
        &self,
        session_id: &str,
        before_seq: u64,
    ) -> Result<bool> {
        dispatch!(self, session_has_live_events_before(session_id, before_seq))
    }

    /// Read one page of ancestor events strictly bounded by `edge.before_seq`
    /// in a single consistent database snapshot.
    pub async fn lineage_history_page(
        &self,
        edge: &LineageEdge,
        before_seq: u64,
        page_size: usize,
    ) -> Result<LineageHistoryPage> {
        dispatch!(self, lineage_history_page(edge, before_seq, page_size))
    }

    /// Read one page of ancestor history for `(child_session_id, hop_parent_id)`
    /// inside a single database snapshot, validating ownership, display
    /// attachment, and `expected_edge_version` atomically with events and seal.
    pub async fn lineage_history_page_checked(
        &self,
        child_session_id: &str,
        hop_parent_id: &str,
        owner_user_id: Option<&str>,
        expected_edge_version: Option<u64>,
        before_seq: u64,
        page_size: usize,
    ) -> Result<LineageHistoryOutcome> {
        dispatch!(
            self,
            lineage_history_page_checked(
                child_session_id,
                hop_parent_id,
                owner_user_id,
                expected_edge_version,
                before_seq,
                page_size,
            )
        )
    }
}
