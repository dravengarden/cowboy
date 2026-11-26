//! Hub-side lineage index, freeze-floor coordination, Clear serialization, and
//! exact sequence cursors.

use std::sync::Arc;

use super::context_rollover::HistoryBoundary;
use super::hub_managed_calls::StoreAckRx;
use super::{Hub, HubInner, Session, StoreWrite};
use crate::agent_model::Event;

/// Durable `child -> parent` entry held in `HubInner::auth_parents`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthParentEntry {
    pub parent_session_id: String,
    /// `true` iff `parent_session_id` was positively verified to be a root
    /// (no parent of its own) when the edge was recorded.
    pub parent_is_root: bool,
}

/// Three-state authorization ancestry probe for fail-closed `lineage_root`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthParentStatus {
    Parent {
        parent_id: String,
        parent_is_root: bool,
    },
    KnownRoot,
    Unknown,
}

/// RAII guard pinning `source_session_id` against concurrent `Clear` while its
/// cutoff is frozen and its durable lineage edge is committed.
pub struct LineageCaptureGuard {
    inner: Arc<HubInner>,
    session_id: String,
}

impl Drop for LineageCaptureGuard {
    fn drop(&mut self) {
        if let Some(session) = self.inner.sessions.lock().get_mut(&self.session_id) {
            session.lineage_pins = session.lineage_pins.saturating_sub(1);
        }
    }
}

impl Hub {
    /// Exclusive sequence cursor (`next_seq`) for a session: `0` when empty,
    /// `last_seq + 1` once at least one event exists.
    #[must_use]
    pub fn next_event_seq(&self, session_id: &str) -> Option<u64> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .map(|session| session.next_seq)
    }

    /// Bounded snapshot of hot-log events strictly `< before_seq`, paired with
    /// `(title, provider, events, reached_start)` for Turn-0 handoff digests.
    #[must_use]
    pub fn hot_events_before(
        &self,
        session_id: &str,
        before_seq: u64,
    ) -> Option<(String, String, Vec<crate::model::Envelope>, bool)> {
        let sessions = self.inner.sessions.lock();
        let session = sessions.get(session_id)?;
        let events: Vec<crate::model::Envelope> = session
            .log
            .iter()
            .filter(|env| env.seq < before_seq)
            .cloned()
            .collect();
        Some((
            session.meta.title.clone(),
            session.meta.provider.clone(),
            events,
            session.reached_start,
        ))
    }

    /// Current native context generation (`0` before any rollover, or the
    /// rollover boundary seq after renewal).
    #[must_use]
    pub fn context_generation(&self, session_id: &str) -> Option<u64> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .and_then(Self::session_context_generation)
    }

    pub(super) fn session_context_generation(session: &Session) -> Option<u64> {
        match session.history_boundary {
            HistoryBoundary::Known(Some(boundary)) => Some(boundary),
            HistoryBoundary::Known(None) => Some(0),
            HistoryBoundary::Unknown => None,
        }
    }

    /// Three-state authorization parent lookup distinguishing a positively
    /// known root from a purged pre-migration parent whose ancestry is unknown.
    #[must_use]
    pub fn auth_parent_status(&self, session_id: &str) -> AuthParentStatus {
        if let Some(entry) = self.inner.auth_parents.lock().get(session_id).cloned() {
            return AuthParentStatus::Parent {
                parent_id: entry.parent_session_id,
                parent_is_root: entry.parent_is_root,
            };
        }
        let Some(info) = self.session_info(session_id) else {
            return AuthParentStatus::Unknown;
        };
        match info.meta.forked_from.or(info.meta.continued_from) {
            Some(parent_id) => AuthParentStatus::Parent {
                parent_id,
                parent_is_root: false,
            },
            None => AuthParentStatus::KnownRoot,
        }
    }

    /// Immediate durable authorization parent of `session_id`, if any.
    #[must_use]
    pub fn auth_parent(&self, session_id: &str) -> Option<String> {
        match self.auth_parent_status(session_id) {
            AuthParentStatus::Parent { parent_id, .. } => Some(parent_id),
            AuthParentStatus::KnownRoot | AuthParentStatus::Unknown => None,
        }
    }

    /// Record a durable `child -> parent` authorization edge in memory.
    pub fn record_auth_parent(
        &self,
        child_session_id: &str,
        parent_session_id: &str,
        parent_is_root: bool,
    ) {
        self.inner.auth_parents.lock().insert(
            child_session_id.to_owned(),
            AuthParentEntry {
                parent_session_id: parent_session_id.to_owned(),
                parent_is_root,
            },
        );
    }

    /// Remove a rolled-back `child -> parent` edge (Cancel Switch).
    pub fn remove_auth_parent(&self, child_session_id: &str) {
        self.inner.auth_parents.lock().remove(child_session_id);
    }

    /// Atomically pin `source_session_id` against concurrent `Clear`, capture
    /// its exclusive cutoff (or clamp an explicit cutoff `<= next_seq`), freeze
    /// the Hub reducer while holding `sessions.lock()`, and enqueue an
    /// acknowledged FIFO `FreezeLineagePrefix` write for the store writer.
    pub fn pin_and_freeze_source(
        &self,
        source_session_id: &str,
        requested_before: Option<u64>,
    ) -> Result<(u64, Option<StoreAckRx>, LineageCaptureGuard), String> {
        let (before_seq, ack_rx) = {
            let mut sessions = self.inner.sessions.lock();
            let session = sessions
                .get_mut(source_session_id)
                .ok_or("Source session no longer exists")?;
            if session.clear_pending {
                return Err(
                    "This session is clearing its conversation; retry once Clear finishes".into(),
                );
            }
            let before_seq = match requested_before {
                Some(explicit) => explicit.min(session.next_seq),
                None => session.next_seq,
            };
            session.lineage_pins = session.lineage_pins.saturating_add(1);
            if before_seq > 0 {
                self.inner
                    .history_reducer
                    .lock()
                    .set_freeze_floor(source_session_id, before_seq);
            }
            let ack_rx = if before_seq > 0
                && let Some(tx) = self.inner.store_tx.as_ref()
            {
                let (ack_id, rx) = self.inner.store_acks.allocate();
                if tx.send(StoreWrite::FreezeLineagePrefix {
                    session_id: source_session_id.to_owned(),
                    before_seq,
                    ack_id: Some(ack_id),
                }) {
                    Some(rx)
                } else {
                    self.inner.store_acks.cancel(ack_id);
                    session.lineage_pins = session.lineage_pins.saturating_sub(1);
                    return Err("Durable store queue is unavailable; retry shortly".into());
                }
            } else {
                None
            };
            (before_seq, ack_rx)
        };
        Ok((
            before_seq,
            ack_rx,
            LineageCaptureGuard {
                inner: Arc::clone(&self.inner),
                session_id: source_session_id.to_owned(),
            },
        ))
    }

    /// Freeze canonical rows `< before_seq` on both the Hub reducer and the
    /// background store writer (best-effort helper when no Clear pin is held).
    pub fn freeze_lineage_prefix(&self, session_id: &str, before_seq: u64) {
        if before_seq == 0 {
            return;
        }
        self.inner
            .history_reducer
            .lock()
            .set_freeze_floor(session_id, before_seq);
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::FreezeLineagePrefix {
                session_id: session_id.to_owned(),
                before_seq,
                ack_id: None,
            });
        }
    }

    /// Snapshot all active freeze floors from the Hub's reducer (passed into
    /// the background store writer at startup).
    #[must_use]
    pub fn snapshot_freeze_floors(&self) -> Vec<(String, u64)> {
        self.inner.history_reducer.lock().snapshot_freeze_floors()
    }

    /// Warm authorization ancestry, live unsealed lineage freeze floors, and
    /// linked-fork `forked_from` projections at startup.
    pub fn warm_lineage_edges(&self, edges: &[crate::store::LineageEdge]) {
        {
            let mut parents = self.inner.auth_parents.lock();
            let mut reducer = self.inner.history_reducer.lock();
            for edge in edges {
                if edge.hop_order == 1 {
                    parents.insert(
                        edge.child_session_id.clone(),
                        AuthParentEntry {
                            parent_session_id: edge.parent_session_id.clone(),
                            parent_is_root: edge.parent_is_root,
                        },
                    );
                }
                if edge.segment_id.is_none() && !edge.display_detached && edge.before_seq > 0 {
                    reducer.set_freeze_floor(&edge.parent_session_id, edge.before_seq);
                }
            }
        }
        for edge in edges {
            if edge.hop_order == 1 && (edge.kind == "linked_fork" || edge.kind == "native_fork") {
                self.set_forked_from(&edge.child_session_id, Some(edge.parent_session_id.clone()));
            }
        }
    }

    /// Exclusive sequence cursor immediately after the completed turn ending at
    /// `message_id`, if that turn is present in the Hub's hot log.
    #[must_use]
    pub fn fork_turn_before_seq(&self, source: &str, message_id: &str) -> Option<u64> {
        let sessions = self.inner.sessions.lock();
        let session = sessions.get(source)?;
        let index = session.log.iter().rposition(|envelope| {
            let Event::Update { update } = &envelope.event else {
                return false;
            };
            update
                .get("sessionUpdate")
                .and_then(serde_json::Value::as_str)
                == Some("agent_message_chunk")
                && update.get("messageId").and_then(serde_json::Value::as_str) == Some(message_id)
        })?;
        for envelope in &session.log[index + 1..] {
            match &envelope.event {
                Event::TurnEnd { .. } => return Some(envelope.seq.saturating_add(1)),
                Event::Update { update }
                    if update
                        .get("sessionUpdate")
                        .and_then(serde_json::Value::as_str)
                        == Some("user_message_chunk")
                        && update
                            .pointer("/_meta/cowboyDelivery")
                            .and_then(serde_json::Value::as_str)
                            != Some("current_turn") =>
                {
                    return Some(envelope.seq);
                }
                _ => {}
            }
        }
        None
    }

    /// Atomically extract `(message_id, before_seq, context_generation)` under
    /// `sessions.lock()` right as `Event::TurnEnd` commits to `session.log`.
    pub(super) fn extract_turn_completion_in_session(
        session_id: &str,
        session: &Session,
        turn_end_seq: u64,
    ) -> Option<crate::store::TurnCompletion> {
        let context_generation = Self::session_context_generation(session)?;
        let mut message_id = None;
        for envelope in session.log.iter().rev() {
            if envelope.seq >= turn_end_seq {
                continue;
            }
            if matches!(envelope.event, Event::TurnEnd { .. }) {
                break;
            }
            let Event::Update { update } = &envelope.event else {
                continue;
            };
            match update
                .get("sessionUpdate")
                .and_then(serde_json::Value::as_str)
            {
                Some("agent_message_chunk") => {
                    if let Some(id) = update.get("messageId").and_then(serde_json::Value::as_str)
                        && !id.is_empty()
                    {
                        message_id = Some(id.to_owned());
                        break;
                    }
                }
                Some(
                    "user_message_chunk" | "context_cleared" | crate::context_rollover::MARKER,
                ) => break,
                _ => {}
            }
        }
        Some(crate::store::TurnCompletion {
            session_id: session_id.to_owned(),
            context_generation,
            message_id: message_id?,
            before_seq: turn_end_seq.saturating_add(1),
            completed_at_ms: chrono::Utc::now().timestamp_millis(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::SessionOrigin;
    use crate::store::LineageEdge;

    fn add_session(hub: &Hub, id: &str) {
        hub.create_local_session(
            id.to_owned(),
            "claude-code".to_owned(),
            "/tmp".to_owned(),
            id.to_owned(),
            SessionOrigin::Web,
            false,
        );
    }

    #[test]
    fn next_event_seq_is_zero_when_empty_and_exclusive_after_events() {
        let hub = Hub::new();
        add_session(&hub, "empty");
        assert_eq!(hub.next_event_seq("empty"), Some(0));
        hub.push(
            "empty",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": "hi"},
                }),
            },
        );
        assert_eq!(hub.next_event_seq("empty"), Some(1));
    }

    #[test]
    fn fork_turn_before_seq_stops_exclusively_after_target_turn() {
        let hub = Hub::new();
        add_session(&hub, "src");
        hub.push(
            "src",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": "q1"},
                }),
            },
        );
        hub.push(
            "src",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "messageId": "a1",
                    "content": {"type": "text", "text": "ans1"},
                }),
            },
        );
        hub.push(
            "src",
            Event::TurnEnd {
                stop_reason: "end_turn".to_owned(),
                usage: None,
            },
        );
        hub.push(
            "src",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": "q2"},
                }),
            },
        );
        assert_eq!(hub.fork_turn_before_seq("src", "a1"), Some(3));
    }

    #[test]
    fn warm_lineage_edges_distinguishes_known_root_from_unknown_and_sets_freeze_floors() {
        let hub = Hub::new();
        add_session(&hub, "c");
        let edges = vec![
            LineageEdge {
                child_session_id: "b".to_owned(),
                hop_order: 1,
                parent_session_id: "a".to_owned(),
                parent_is_root: true,
                owner_user_id: String::new(),
                kind: "replace".to_owned(),
                before_seq: 5,
                source_title: "A".to_owned(),
                source_provider: "claude-code".to_owned(),
                segment_id: Some("seg-a".to_owned()),
                display_detached: true,
                edge_version: 2,
                created_at_ms: 1,
            },
            LineageEdge {
                child_session_id: "c".to_owned(),
                hop_order: 1,
                parent_session_id: "b".to_owned(),
                parent_is_root: false,
                owner_user_id: String::new(),
                kind: "linked_fork".to_owned(),
                before_seq: 9,
                source_title: "B".to_owned(),
                source_provider: "codex".to_owned(),
                segment_id: None,
                display_detached: false,
                edge_version: 1,
                created_at_ms: 2,
            },
        ];
        hub.warm_lineage_edges(&edges);
        assert_eq!(
            hub.auth_parent_status("c"),
            AuthParentStatus::Parent {
                parent_id: "b".to_owned(),
                parent_is_root: false,
            }
        );
        assert_eq!(
            hub.auth_parent_status("b"),
            AuthParentStatus::Parent {
                parent_id: "a".to_owned(),
                parent_is_root: true,
            }
        );
        assert_eq!(
            hub.auth_parent_status("missing-legacy"),
            AuthParentStatus::Unknown
        );
        let reducer = hub.inner.history_reducer.lock();
        assert_eq!(reducer.freeze_floor("b"), 9);
        assert_eq!(reducer.freeze_floor("a"), 0);
    }

    #[test]
    fn pin_and_freeze_source_blocks_concurrent_clear() {
        let hub = Hub::new();
        add_session(&hub, "src");
        hub.push(
            "src",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": "hello"},
                }),
            },
        );
        let (before, _rx, guard) = hub.pin_and_freeze_source("src", None).unwrap();
        assert_eq!(before, 1);
        assert!(hub.begin_clear_conversation("src").is_err());
        drop(guard);
        assert!(hub.begin_clear_conversation("src").is_ok());
    }
}
