//! Hub-side lineage index, freeze-floor coordination, and exact sequence cursors.
//!
//! `auth_parents` tracks `child_session_id -> parent_session_id` independently
//! of display detachment or parent archival/purge so Tools widening checks can
//! never be shed by Clear or Trash retention.

use super::{Hub, StoreWrite};
use crate::agent_model::Event;

impl Hub {
    /// Exclusive sequence cursor (`next_seq`) for a session: `0` when empty,
    /// `last_seq + 1` once at least one event exists. Unlike
    /// `last_event_seq().saturating_add(1)`, this never turns an empty session
    /// into cutoff `1`.
    #[must_use]
    pub fn next_event_seq(&self, session_id: &str) -> Option<u64> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .map(|session| session.next_seq)
    }

    /// Durable authorization parent of `session_id`, surviving Clear and
    /// archived/purged intermediates, with fallback to live legacy marks.
    #[must_use]
    pub fn auth_parent(&self, session_id: &str) -> Option<String> {
        if let Some(parent) = self.inner.auth_parents.lock().get(session_id).cloned() {
            return Some(parent);
        }
        let info = self.session_info(session_id)?;
        info.meta.forked_from.or(info.meta.continued_from)
    }

    /// Record a durable `child -> parent` authorization edge in memory.
    pub fn record_auth_parent(&self, child_session_id: &str, parent_session_id: &str) {
        self.inner
            .auth_parents
            .lock()
            .insert(child_session_id.to_owned(), parent_session_id.to_owned());
    }

    /// Remove a rolled-back `child -> parent` edge (Cancel Switch).
    pub fn remove_auth_parent(&self, child_session_id: &str) {
        self.inner.auth_parents.lock().remove(child_session_id);
    }

    /// Freeze canonical rows `< before_seq` on both the Hub reducer and the
    /// background store writer so post-branch streaming or tool updates never
    /// mutate an inherited prefix in place.
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
            });
        }
    }

    /// Warm authorization ancestry, live unsealed freeze floors, and linked-fork
    /// `forked_from` projections from persisted lineage edges at startup.
    pub fn warm_lineage_edges(&self, edges: &[crate::store::LineageEdge]) {
        {
            let mut parents = self.inner.auth_parents.lock();
            let mut reducer = self.inner.history_reducer.lock();
            for edge in edges {
                parents.insert(
                    edge.child_session_id.clone(),
                    edge.parent_session_id.clone(),
                );
                if edge.segment_id.is_none() && !edge.display_detached && edge.before_seq > 0 {
                    reducer.set_freeze_floor(&edge.parent_session_id, edge.before_seq);
                }
            }
        }
        for edge in edges {
            if edge.kind == "linked_fork" || edge.kind == "native_fork" {
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
        Some(session.next_seq)
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
    fn warm_lineage_edges_preserves_auth_parent_across_missing_intermediate_and_sets_freeze_floor() {
        let hub = Hub::new();
        add_session(&hub, "c");
        let edges = vec![
            LineageEdge {
                child_session_id: "b".to_owned(),
                parent_session_id: "a".to_owned(),
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
                parent_session_id: "b".to_owned(),
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
        assert_eq!(hub.auth_parent("c").as_deref(), Some("b"));
        assert_eq!(hub.auth_parent("b").as_deref(), Some("a"));
        assert_eq!(
            hub.session_info("c").unwrap().meta.forked_from.as_deref(),
            Some("b")
        );
        let reducer = hub.inner.history_reducer.lock();
        assert_eq!(reducer.freeze_floor("b"), 9);
        assert_eq!(reducer.freeze_floor("a"), 0);
    }
}
