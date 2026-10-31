//! Durable transcript anchors for background tasks (docs/background-tasks.md).
//!
//! A subagent child (`cowboy_subagent`) and an accepted Call
//! (`cowboy_call_anchor`) each own exactly one transcript row: every later
//! snapshot replaces that row in place. The row's position is its *anchor*.
//! Both reducers (the Hub's hot log and the store writer) resolve an anchor
//! through an [`AnchorIndex`]: a bounded LRU per process whose misses fall
//! back to the durable `transcript_anchors` table. Eviction therefore never
//! makes a known child look new, and a Controller restart never starts a
//! second row.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

/// `sessionUpdate` of the per-Call transcript anchor row.
pub const CALL_ANCHOR_UPDATE: &str = "cowboy_call_anchor";
/// `sessionUpdate` of a subagent snapshot row (see `runtime_wire`).
pub const SUBAGENT_UPDATE: &str = "cowboy_subagent";
pub const SUBAGENT_PREFIX: &str = "subagent:";
pub const CALL_PREFIX: &str = "call:";
/// Marks a session whose rows from before the table existed were scanned.
pub const BACKFILL_SENTINEL: &str = "backfill:v1";
/// Anchors the Hub warms from the table at start, newest first.
pub const HUB_WARM_ANCHORS: usize = 16_384;
/// Anchors each process keeps in memory before the least recently used is
/// forgotten (and later resolved through the store again).
pub const INDEX_CAPACITY: usize = 16_384;
/// Child ids accepted by one lookup request.
pub const LOOKUP_MAX_IDS: usize = 200;
const ID_MAX_BYTES: usize = 256;

/// The anchor key an update owns, if it is an anchored row.
#[must_use]
pub fn anchor_key(update: &Value) -> Option<String> {
    let kind = update.get("sessionUpdate").and_then(Value::as_str)?;
    let (field, prefix) = match kind {
        SUBAGENT_UPDATE => ("sessionId", SUBAGENT_PREFIX),
        CALL_ANCHOR_UPDATE => ("callId", CALL_PREFIX),
        _ => return None,
    };
    update
        .get(field)
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
        .map(|id| format!("{prefix}{id}"))
}

/// A child or call id that may name an anchor.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= ID_MAX_BYTES && !id.chars().any(char::is_control)
}

/// One durable anchor row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorWrite {
    pub session_id: String,
    pub key: String,
    pub seq: u64,
}

/// Bounded LRU of `(session, anchor key) -> seq`. Never cleared wholesale.
#[derive(Debug)]
pub struct AnchorIndex {
    capacity: usize,
    tick: u64,
    entries: HashMap<(String, String), (u64, u64)>,
    order: BTreeMap<u64, (String, String)>,
}

impl Default for AnchorIndex {
    fn default() -> Self {
        Self::with_capacity(INDEX_CAPACITY)
    }
}

impl AnchorIndex {
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            tick: 0,
            entries: HashMap::new(),
            order: BTreeMap::new(),
        }
    }

    fn touch(&mut self, key: &(String, String)) -> Option<u64> {
        let tick = self.tick.wrapping_add(1);
        let (seq, previous) = {
            let entry = self.entries.get_mut(key)?;
            let previous = entry.1;
            entry.1 = tick;
            (entry.0, previous)
        };
        self.tick = tick;
        self.order.remove(&previous);
        self.order.insert(tick, key.clone());
        Some(seq)
    }

    /// The anchor's seq, refreshing its recency.
    pub fn get(&mut self, session_id: &str, key: &str) -> Option<u64> {
        let found = self.touch(&(session_id.to_owned(), key.to_owned()));
        if found.is_some() {
            METRICS.cache_hits.fetch_add(1, Ordering::Relaxed);
        } else {
            METRICS.cache_misses.fetch_add(1, Ordering::Relaxed);
        }
        found
    }

    /// Whether the index holds the anchor, without counting a lookup.
    #[must_use]
    pub fn contains(&self, session_id: &str, key: &str) -> bool {
        self.entries
            .contains_key(&(session_id.to_owned(), key.to_owned()))
    }

    pub fn insert(&mut self, session_id: &str, key: &str, seq: u64) {
        let id = (session_id.to_owned(), key.to_owned());
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.0 = seq;
            self.touch(&id);
            return;
        }
        while self.entries.len() >= self.capacity {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            self.entries.remove(&oldest);
            METRICS.evictions.fetch_add(1, Ordering::Relaxed);
        }
        self.tick = self.tick.wrapping_add(1);
        self.order.insert(self.tick, id.clone());
        self.entries.insert(id, (seq, self.tick));
    }

    /// Forget one anchor: the next write resolves it through the store again.
    pub fn remove(&mut self, session_id: &str, key: &str) {
        if let Some((_, tick)) = self
            .entries
            .remove(&(session_id.to_owned(), key.to_owned()))
        {
            self.order.remove(&tick);
        }
    }

    pub fn remove_session(&mut self, session_id: &str) {
        self.entries.retain(|(session, _), _| session != session_id);
        self.order.retain(|_, (session, _)| session != session_id);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Process-wide diagnostic counters, exported on `/metrics`.
#[derive(Debug, Default)]
pub struct AnchorMetrics {
    pub cache_hits: AtomicU64,
    pub cache_misses: AtomicU64,
    pub evictions: AtomicU64,
    pub store_lookups: AtomicU64,
    pub backfills: AtomicU64,
    pub revision_conflicts: AtomicU64,
    pub revision_stale: AtomicU64,
    pub duplicates: AtomicU64,
    pub epoch_changes: AtomicU64,
    pub lookup_requests: AtomicU64,
    pub lookup_ids: AtomicU64,
    pub call_anchor_repairs: AtomicU64,
}

pub static METRICS: AnchorMetrics = AnchorMetrics {
    cache_hits: AtomicU64::new(0),
    cache_misses: AtomicU64::new(0),
    evictions: AtomicU64::new(0),
    store_lookups: AtomicU64::new(0),
    backfills: AtomicU64::new(0),
    revision_conflicts: AtomicU64::new(0),
    revision_stale: AtomicU64::new(0),
    duplicates: AtomicU64::new(0),
    epoch_changes: AtomicU64::new(0),
    lookup_requests: AtomicU64::new(0),
    lookup_ids: AtomicU64::new(0),
    call_anchor_repairs: AtomicU64::new(0),
};

impl AnchorMetrics {
    /// Prometheus text exposition of every counter.
    #[must_use]
    pub fn prometheus(&self) -> String {
        let counters = [
            (
                "cowboy_transcript_anchor_cache_hits_total",
                &self.cache_hits,
            ),
            (
                "cowboy_transcript_anchor_cache_misses_total",
                &self.cache_misses,
            ),
            ("cowboy_transcript_anchor_evictions_total", &self.evictions),
            (
                "cowboy_transcript_anchor_store_lookups_total",
                &self.store_lookups,
            ),
            ("cowboy_transcript_anchor_backfills_total", &self.backfills),
            (
                "cowboy_transcript_anchor_call_repairs_total",
                &self.call_anchor_repairs,
            ),
            (
                "cowboy_subagent_revision_conflicts_total",
                &self.revision_conflicts,
            ),
            ("cowboy_subagent_revision_stale_total", &self.revision_stale),
            ("cowboy_subagent_duplicates_total", &self.duplicates),
            ("cowboy_subagent_epoch_changes_total", &self.epoch_changes),
            (
                "cowboy_subagent_lookup_requests_total",
                &self.lookup_requests,
            ),
            ("cowboy_subagent_lookup_ids_total", &self.lookup_ids),
        ];
        let mut output = String::new();
        for (name, value) in counters {
            output.push_str(&format!(
                "# TYPE {name} counter\n{name} {}\n",
                value.load(Ordering::Relaxed)
            ));
        }
        output
    }
}

/// A subagent snapshot in a terminal foreground state. `idle` is the only
/// state the ACP RFD defines as no longer working; `unknown` is not terminal.
#[must_use]
pub fn subagent_terminal(update: &Value) -> bool {
    update.get("state").and_then(Value::as_str) == Some("idle")
}

/// The Controller receipt sequence of a snapshot, else its producer revision
/// (rows from before stamping). Higher wins.
#[must_use]
pub fn snapshot_order(update: &Value) -> (u64, u64) {
    (
        update
            .get("observedSeq")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        update.get("revision").and_then(Value::as_u64).unwrap_or(0),
    )
}

/// What the Controller does with one incoming subagent snapshot (I4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotVerdict {
    Accept,
    /// Same incarnation and revision, identical payload.
    Duplicate,
    /// Same incarnation and revision, different payload: accepted and counted.
    Conflict,
    /// An older revision of the same incarnation: never wins.
    Stale,
}

/// The identity and content digest of the last accepted snapshot of a child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMark {
    pub incarnation: Option<String>,
    pub revision: u64,
    pub digest: [u8; 32],
}

/// Digest of a snapshot payload without the Controller's own stamps.
#[must_use]
pub fn snapshot_digest(update: &Value) -> [u8; 32] {
    use sha2::Digest as _;
    let mut copy = update.clone();
    if let Some(object) = copy.as_object_mut() {
        object.remove("observedSeq");
        object.remove("incarnation");
    }
    sha2::Sha256::digest(copy.to_string().as_bytes()).into()
}

#[must_use]
pub fn mark_of(update: &Value) -> SnapshotMark {
    SnapshotMark {
        incarnation: update
            .get("incarnation")
            .and_then(Value::as_str)
            .map(str::to_owned),
        revision: update.get("revision").and_then(Value::as_u64).unwrap_or(0),
        digest: snapshot_digest(update),
    }
}

#[must_use]
pub fn judge_snapshot(previous: Option<&SnapshotMark>, next: &SnapshotMark) -> SnapshotVerdict {
    let Some(previous) = previous else {
        return SnapshotVerdict::Accept;
    };
    if previous.incarnation != next.incarnation {
        return SnapshotVerdict::Accept;
    }
    match next.revision.cmp(&previous.revision) {
        std::cmp::Ordering::Greater => SnapshotVerdict::Accept,
        std::cmp::Ordering::Less => SnapshotVerdict::Stale,
        std::cmp::Ordering::Equal if previous.digest == next.digest => SnapshotVerdict::Duplicate,
        std::cmp::Ordering::Equal => SnapshotVerdict::Conflict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_name_subagents_and_calls_only() {
        assert_eq!(
            anchor_key(&json!({"sessionUpdate":"cowboy_subagent","sessionId":"c1"})).as_deref(),
            Some("subagent:c1")
        );
        assert_eq!(
            anchor_key(&json!({"sessionUpdate":"cowboy_call_anchor","callId":"call-1"})).as_deref(),
            Some("call:call-1")
        );
        assert_eq!(
            anchor_key(&json!({"sessionUpdate":"cowboy_subagent"})),
            None
        );
        assert_eq!(
            anchor_key(&json!({"sessionUpdate":"tool_call","toolCallId":"t"})),
            None
        );
    }

    #[test]
    fn the_index_evicts_least_recently_used_without_clearing() {
        let mut index = AnchorIndex::with_capacity(3);
        index.insert("s", "subagent:a", 1);
        index.insert("s", "subagent:b", 2);
        index.insert("s", "subagent:c", 3);
        assert_eq!(index.get("s", "subagent:a"), Some(1));
        index.insert("s", "subagent:d", 4);
        assert_eq!(index.len(), 3, "one entry was evicted, never all of them");
        assert_eq!(
            index.get("s", "subagent:b"),
            None,
            "b was least recently used"
        );
        assert_eq!(index.get("s", "subagent:a"), Some(1));
        assert_eq!(index.get("s", "subagent:d"), Some(4));
        index.remove_session("s");
        assert!(index.is_empty());
    }

    // T5: reordered and duplicate updates.
    #[test]
    fn t5_duplicates_are_ignored_older_revisions_never_win_conflicts_are_counted() {
        let snapshot = |revision: u64, state: &str| {
            json!({"sessionUpdate":"cowboy_subagent","sessionId":"c","incarnation":"w1",
                "revision":revision,"state":state})
        };
        let first = mark_of(&snapshot(10, "running"));
        assert_eq!(judge_snapshot(None, &first), SnapshotVerdict::Accept);
        let mut stamped = snapshot(10, "running");
        stamped["observedSeq"] = json!(99);
        assert_eq!(
            judge_snapshot(Some(&first), &mark_of(&stamped)),
            SnapshotVerdict::Duplicate,
            "the Controller's own stamp is not content"
        );
        assert_eq!(
            judge_snapshot(Some(&first), &mark_of(&snapshot(9, "idle"))),
            SnapshotVerdict::Stale
        );
        assert_eq!(
            judge_snapshot(Some(&first), &mark_of(&snapshot(10, "idle"))),
            SnapshotVerdict::Conflict
        );
        assert_eq!(
            judge_snapshot(Some(&first), &mark_of(&snapshot(11, "idle"))),
            SnapshotVerdict::Accept
        );
        let mut other_worker = snapshot(3, "running");
        other_worker["incarnation"] = json!("w2");
        assert_eq!(
            judge_snapshot(Some(&first), &mark_of(&other_worker)),
            SnapshotVerdict::Accept,
            "a new incarnation restarts the producer revision clock"
        );
    }
}
