//! Watch the Controller's canonical top-level native subagent snapshots.
//! Terminal snapshots can prove a settlement despite a coverage gap.
use super::AppState;
use crate::core::{Event, Hub, Outbound};
use crate::logs::wait_cache::{self, Fields};
use crate::transcript_anchors::{SUBAGENT_UPDATE, subagent_terminal};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

const MAX_PENDING: usize = 2048;

struct Pending {
    started: Option<Instant>,
    seq: u64,
    parent_ended: Option<bool>,
}

struct Observation {
    event: &'static str,
    identity: u64,
    outcome: &'static str,
    start_seq: Option<u64>,
    parent_ended: Option<bool>,
    duration: Option<u64>,
}

#[derive(Default)]
struct Waits {
    sessions: HashMap<String, HashMap<(String, String), Pending>>,
    count: usize,
    settled: HashSet<(String, u64)>,
    settled_order: VecDeque<(String, u64)>,
}

impl Waits {
    fn parent_boundary(&mut self, session: &str, seq: u64) {
        if let Some(tasks) = self.sessions.get_mut(session) {
            for pending in tasks.values_mut().filter(|pending| pending.seq < seq) {
                pending.parent_ended = Some(true);
            }
        }
    }

    fn forget(&mut self, session: &str) {
        if let Some(tasks) = self.sessions.remove(session) {
            self.count -= tasks.len();
        }
    }

    fn settle(
        &mut self,
        session: &str,
        key: &(String, String),
        terminal_seq: u64,
    ) -> Option<Observation> {
        // A terminal seq is unique in its parent timeline. Keep recent keys
        // through gaps, and use the same durable evidence ID for every replay.
        let occurrence = (session.to_owned(), terminal_seq);
        if !self.settled.insert(occurrence.clone()) {
            return None;
        }
        self.settled_order.push_back(occurrence);
        if self.settled.len() > MAX_PENDING
            && let Some(oldest) = self.settled_order.pop_front()
        {
            self.settled.remove(&oldest);
        }
        let pending = self.take_pending(session, key, terminal_seq);
        let started = pending.as_ref().and_then(|p| p.started);
        Some(Observation {
            event: wait_cache::SETTLED,
            identity: terminal_seq,
            outcome: if started.is_some() {
                "observed"
            } else {
                "partial"
            },
            start_seq: pending.as_ref().and_then(|p| p.started.map(|_| p.seq)),
            parent_ended: pending.and_then(|p| p.parent_ended),
            duration: started
                .map(|start| u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)),
        })
    }

    fn take_pending(
        &mut self,
        session: &str,
        key: &(String, String),
        terminal_seq: u64,
    ) -> Option<Pending> {
        let tasks = self.sessions.get_mut(session)?;
        if tasks.get(key)?.seq >= terminal_seq {
            return None;
        }
        let pending = tasks.remove(key)?;
        self.count -= 1;
        if tasks.is_empty() {
            self.sessions.remove(session);
        }
        Some(pending)
    }

    fn running(
        &mut self,
        session: &str,
        key: (String, String),
        seq: u64,
        start_observed: bool,
    ) -> Option<Observation> {
        if let Some(pending) = self
            .sessions
            .get_mut(session)
            .and_then(|tasks| tasks.get_mut(&key))
        {
            if !start_observed || seq <= pending.seq {
                return None;
            }
            // A genuine boundary supersedes a partially observed incarnation.
            *pending = Self::pending(seq, true);
        } else {
            if self.count == MAX_PENDING {
                Self::coverage(1);
                return None;
            }
            self.sessions
                .entry(session.to_owned())
                .or_default()
                .insert(key, Self::pending(seq, start_observed));
            self.count += 1;
        }
        start_observed.then_some(Observation {
            event: wait_cache::DISPATCH,
            identity: seq,
            outcome: "admitted",
            start_seq: Some(seq),
            parent_ended: None,
            duration: None,
        })
    }

    fn pending(seq: u64, start_observed: bool) -> Pending {
        Pending {
            seq,
            started: start_observed.then(Instant::now),
            parent_ended: start_observed.then_some(false),
        }
    }

    fn observe_snapshot(
        &mut self,
        session: &str,
        key: (String, String),
        update: &serde_json::Value,
        start_observed: bool,
    ) -> Option<Observation> {
        let seq = update["observedSeq"].as_u64()?;
        if subagent_terminal(update) {
            let terminal = update["terminalSeq"].as_u64()?;
            if terminal > seq {
                return None;
            }
            self.settle(session, &key, terminal)
        } else if update["state"] == "running" {
            self.running(session, key, seq, start_observed)
        } else {
            None
        }
    }

    fn native_update(
        &mut self,
        hub: &Hub,
        session: &str,
        update: &serde_json::Value,
        start_observed: bool,
    ) {
        if update["sessionUpdate"] != SUBAGENT_UPDATE || !update["parentSessionId"].is_null() {
            return;
        }
        let (Some(child), Some(incarnation)) =
            (update["sessionId"].as_str(), update["incarnation"].as_str())
        else {
            return;
        };
        let key = (child.to_owned(), incarnation.to_owned());
        let Some(observation) = self.observe_snapshot(session, key, update, start_observed) else {
            return;
        };
        let settled = observation.event == wait_cache::SETTLED;
        let mut fields = Fields::default()
            .string("cowboy.session.id", session)
            .string("native_child_id", child)
            .string("wait_path", "native_subagent")
            .string("outcome", observation.outcome)
            .integer("native_wait_started_seq", observation.start_seq)
            .boolean("parent_ended_turn", observation.parent_ended)
            // Native task notifications have no Cowboy Call notice/wake.
            .boolean("wake_needed", settled.then_some(false))
            .integer("wait_duration_ms", observation.duration)
            .integer("cowboy.duration_ms", observation.duration);
        if let Some(info) = hub.session_info(session) {
            fields = fields
                .string("provider", wait_cache::provider(&info.meta.provider))
                .integer(
                    "verified_context_used",
                    hub.verified_context_usage(&info.meta),
                );
        }
        let seq = observation.identity.to_string();
        let terminal_key = [session, seq.as_str()];
        let dispatch_key = [session, child, incarnation, seq.as_str()];
        let key: &[&str] = if settled {
            &terminal_key
        } else {
            &dispatch_key
        };
        wait_cache::record(observation.event, key, None, fields);
    }

    fn gap(&mut self) {
        let dropped = self.count;
        self.sessions.clear();
        self.count = 0;
        Self::coverage(dropped);
    }

    fn coverage(dropped: usize) {
        wait_cache::record(
            wait_cache::COVERAGE,
            &[],
            None,
            Fields::default()
                .string("producer", "native_wait")
                .string("outcome", "partial")
                .integer("producer_dropped", dropped as u64),
        );
    }
}

pub async fn observe(state: Arc<AppState>) {
    let mut stream = state.hub.subscribe();
    let mut shutdown = state.shutdown.clone();
    let mut waits = Waits::default();
    loop {
        let frame = tokio::select! {
            _ = shutdown.wait_for(|stopping| *stopping) => return,
            frame = stream.recv() => frame,
        };
        let frame = match frame {
            Ok(frame) => frame,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                waits.gap();
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        };
        let Outbound::Event { envelope } = frame.outbound() else {
            continue;
        };
        match &envelope.event {
            Event::Update { update } => waits.native_update(
                &state.hub,
                &envelope.session_id,
                update,
                frame.native_wait_started(),
            ),
            Event::TurnEnd { stop_reason, .. }
                if matches!(
                    stop_reason.as_str(),
                    "EndTurn" | "MaxTokens" | "MaxTurnRequests"
                ) =>
            {
                waits.parent_boundary(&envelope.session_id, envelope.seq);
            }
            Event::Lifecycle {
                status:
                    crate::core::Status::Starting
                    | crate::core::Status::Exited
                    | crate::core::Status::Crashed,
                ..
            } => {
                waits.forget(&envelope.session_id);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_and_overflow_leave_pending_waits_at_capacity() {
        let mut waits = Waits::default();
        let key = |n: usize| (n.to_string(), "worker".to_owned());
        for n in 0..MAX_PENDING {
            assert!(waits.running("parent", key(n), 1, true).is_some());
        }
        assert!(waits.running("parent", key(0), 2, false).is_none());
        assert!(waits.running("parent", key(0), 1, true).is_none());
        assert!(waits.running("parent", key(MAX_PENDING), 2, true).is_none());
        assert_eq!(waits.count, MAX_PENDING);
        let settled = waits.settle("parent", &key(0), 3).unwrap();
        assert_eq!(
            (settled.event, settled.identity, settled.outcome),
            (wait_cache::SETTLED, 3, "observed")
        );
        assert_eq!(settled.start_seq, Some(1));
        assert_eq!(settled.parent_ended, Some(false));
        assert!(settled.duration.is_some());
    }

    #[test]
    fn rediscovered_wait_is_partial_until_a_new_start_is_proven() {
        let mut waits = Waits::default();
        let key = ("child".into(), "worker".into());
        assert!(waits.running("parent", key.clone(), 1, true).is_some());
        waits.gap();
        assert!(waits.running("parent", key.clone(), 7, false).is_none());
        let partial = waits.settle("parent", &key, 8).unwrap();
        assert_eq!((partial.identity, partial.outcome), (8, "partial"));
        assert_eq!((partial.parent_ended, partial.duration), (None, None));
        assert!(waits.running("parent", key.clone(), 9, false).is_none());
        assert!(waits.running("parent", key.clone(), 10, true).is_some());
        waits.parent_boundary("parent", 11);
        let settled = waits.settle("parent", &key, 12).unwrap();
        assert_eq!((settled.identity, settled.parent_ended), (12, Some(true)));
        assert_eq!(settled.start_seq, Some(10));
    }

    #[test]
    fn terminal_first_after_a_gap_records_one_partial_occurrence() {
        let mut waits = Waits::default();
        let key = ("child".into(), "worker".into());
        assert!(waits.running("parent", key.clone(), 1, true).is_some());
        waits.gap();
        let snapshot = |seq, terminal| serde_json::json!({"state": "idle", "observedSeq": seq, "terminalSeq": terminal});
        // The first frame after lag can already be a repeated terminal snapshot.
        let partial = waits
            .observe_snapshot("parent", key.clone(), &snapshot(5, 4), false)
            .unwrap();
        assert_eq!(
            (partial.event, partial.identity, partial.outcome),
            (wait_cache::SETTLED, 4, "partial")
        );
        assert_eq!(
            (partial.start_seq, partial.parent_ended, partial.duration),
            (None, None, None)
        );
        waits.gap();
        assert!(
            waits
                .observe_snapshot("parent", key.clone(), &snapshot(6, 4), false)
                .is_none()
        );
        assert!(waits.running("parent", key.clone(), 7, true).is_some());
        assert!(
            waits
                .observe_snapshot("parent", key.clone(), &snapshot(8, 4), false)
                .is_none()
        );
        let complete = waits
            .observe_snapshot("parent", key.clone(), &snapshot(9, 9), false)
            .unwrap();
        assert_eq!((complete.identity, complete.start_seq), (9, Some(7)));
        assert!(
            waits
                .observe_snapshot("parent", key, &snapshot(10, 9), false)
                .is_none()
        );
    }

    #[test]
    fn terminal_updates_without_canonical_terminal_seq_do_not_double_settle() {
        let mut waits = Waits::default();
        let key = ("child".into(), "worker".into());
        assert!(waits.running("parent", key.clone(), 1, true).is_some());
        let unstamped = |seq| serde_json::json!({"state": "idle", "observedSeq": seq});
        assert!(
            waits
                .observe_snapshot("parent", key.clone(), &unstamped(2), false)
                .is_none()
        );
        assert!(
            waits
                .observe_snapshot("parent", key.clone(), &unstamped(3), false)
                .is_none()
        );
        let stamped = |seq, terminal| serde_json::json!({"state": "idle", "observedSeq": seq, "terminalSeq": terminal});
        let settled = waits
            .observe_snapshot("parent", key.clone(), &stamped(4, 2), false)
            .unwrap();
        assert_eq!((settled.identity, settled.outcome), (2, "observed"));
        assert!(
            waits
                .observe_snapshot("parent", key, &stamped(5, 2), false)
                .is_none()
        );
    }
}
