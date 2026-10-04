//! Bounded in-process effect ownership. Tombstones live for the incarnation.
//! Losing this process loses execution authority; the launch marker forbids an
//! automatic same-incarnation restart. No timeout grants permission to replay.

use crate::execution_protocol::{Event, Invocation, Outcome, Refusal};
use sha2::{Digest as _, Sha256};
use std::collections::{HashMap, VecDeque};
use tokio::sync::watch;

const MAX_OPERATIONS: usize = 65_536;
const MAX_PENDING: usize = 64;
const MAX_PENDING_BYTES: usize = 16 * 1024 * 1024;
const MAX_RESULTS_BYTES: usize = 16 * 1024 * 1024;
const MAX_EVENTS_BYTES: usize = 4 * 1024 * 1024;
const MAX_EVENT_BATCH_BYTES: usize = 2 * 1024 * 1024;

pub(super) enum EventError {
    Full(serde_json::Value),
    Invalid,
}

impl std::fmt::Debug for EventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Native messages may contain source, credentials or command output.
        f.write_str(match self {
            Self::Full(_) => "EventBackpressure",
            Self::Invalid => "InvalidEvent",
        })
    }
}

struct Entry {
    digest: [u8; 32],
    outcome: watch::Sender<Outcome>,
    bytes: usize,
    request_bytes: usize,
}

pub(super) enum Admission {
    Existing(watch::Receiver<Outcome>),
    New {
        id: u64,
        receiver: watch::Receiver<Outcome>,
    },
}

pub(super) struct Ledger {
    operations: HashMap<String, Entry>,
    pending: HashMap<u64, String>,
    next_id: u64,
    pending_bytes: usize,
    completed: VecDeque<String>,
    result_bytes: usize,
    pub lost: bool,
    events: VecDeque<(Event, usize)>,
    event_bytes: usize,
    sequence: u64,
    acknowledged: u64,
    pub event_changed: watch::Sender<u64>,
    pub event_space: watch::Sender<u64>,
}

impl Ledger {
    pub fn new() -> Self {
        Self {
            operations: HashMap::new(),
            pending: HashMap::new(),
            next_id: 1,
            pending_bytes: 0,
            completed: VecDeque::new(),
            result_bytes: 0,
            lost: false,
            events: VecDeque::new(),
            event_bytes: 0,
            sequence: 0,
            acknowledged: 0,
            event_changed: watch::channel(0).0,
            event_space: watch::channel(0).0,
        }
    }

    pub fn admit(&mut self, invocation: &Invocation) -> Result<Admission, Refusal> {
        if !invocation.validate() {
            return Err(Refusal::InvalidRequest);
        }
        let serialized = serde_json::to_vec(invocation).map_err(|_| Refusal::InvalidRequest)?;
        let digest: [u8; 32] = Sha256::digest(&serialized).into();
        if let Some(entry) = self.operations.get(&invocation.operation_id) {
            if entry.digest != digest {
                return Err(Refusal::OperationConflict);
            }
            return Ok(Admission::Existing(entry.outcome.subscribe()));
        }
        if self.lost {
            return Err(Refusal::EnvironmentLost);
        }
        if self.pending.len() >= MAX_PENDING
            || self.operations.len() >= MAX_OPERATIONS
            || self.pending_bytes + serialized.len() > MAX_PENDING_BYTES
        {
            return Err(Refusal::Capacity);
        }
        let (outcome, receiver) = watch::channel(Outcome::Pending);
        let id = self.next_id;
        self.next_id += 1;
        self.pending_bytes += serialized.len();
        self.pending.insert(id, invocation.operation_id.clone());
        self.operations.insert(
            invocation.operation_id.clone(),
            Entry {
                digest,
                outcome,
                bytes: 0,
                request_bytes: serialized.len(),
            },
        );
        Ok(Admission::New { id, receiver })
    }

    pub fn observe(&self, id: &str) -> Option<watch::Receiver<Outcome>> {
        self.operations
            .get(id)
            .map(|entry| entry.outcome.subscribe())
    }

    pub fn finish(&mut self, id: u64, reply: serde_json::Value) -> Result<(), ()> {
        let operation_id = self.pending.remove(&id).ok_or(())?;
        let entry = self.operations.get_mut(&operation_id).ok_or(())?;
        self.pending_bytes -= entry.request_bytes;
        entry.request_bytes = 0;
        let bytes = serde_json::to_vec(&reply).map_err(|_| ())?.len();
        entry.bytes = bytes;
        entry.outcome.send_replace(Outcome::Completed { reply });
        self.result_bytes += bytes;
        self.completed.push_back(operation_id);
        while self.result_bytes > MAX_RESULTS_BYTES {
            let expired = self.completed.pop_front().ok_or(())?;
            let entry = self.operations.get_mut(&expired).ok_or(())?;
            self.result_bytes -= entry.bytes;
            entry.bytes = 0;
            entry.outcome.send_replace(Outcome::ResultExpired);
        }
        Ok(())
    }

    pub fn lose(&mut self) {
        self.lost = true;
        self.pending_bytes = 0;
        for (_, operation) in self.pending.drain() {
            if let Some(entry) = self.operations.get(&operation) {
                entry.outcome.send_replace(Outcome::Unknown);
            }
        }
        self.event_changed.send_replace(self.sequence);
    }

    pub fn push_event(&mut self, message: serde_json::Value) -> Result<(), EventError> {
        let bytes = serde_json::to_vec(&message)
            .map_err(|_| EventError::Invalid)?
            .len();
        if bytes > MAX_EVENT_BATCH_BYTES {
            return Err(EventError::Invalid);
        }
        if self.event_bytes + bytes > MAX_EVENTS_BYTES || self.events.len() >= 8192 {
            // Never evict an event the consumer has not acknowledged. The
            // keeper pauses its bounded native reader until Events advances.
            // Backpressure then reaches the native executor and its processes.
            return Err(EventError::Full(message));
        }
        self.sequence += 1;
        self.events.push_back((
            Event {
                sequence: self.sequence,
                message,
            },
            bytes,
        ));
        self.event_bytes += bytes;
        self.event_changed.send_replace(self.sequence);
        Ok(())
    }

    pub fn events(&mut self, after: u64) -> Result<(Vec<Event>, u64), Refusal> {
        if after > self.sequence {
            return Err(Refusal::InvalidRequest);
        }
        if after < self.acknowledged {
            return Err(Refusal::CursorExpired);
        }
        // Events has one consumer per bound worker. Its next cursor is an
        // acknowledgement of the previous batch, not permission to drop any
        // newer output. Repeating the current cursor remains idempotent.
        if after > self.acknowledged {
            while self
                .events
                .front()
                .is_some_and(|(event, _)| event.sequence <= after)
            {
                self.event_bytes -= self.events.pop_front().expect("front exists").1;
            }
            self.acknowledged = after;
            self.event_space.send_replace(after);
        }
        let mut bytes = 0;
        let mut events = Vec::new();
        let mut through = after;
        for (event, size) in &self.events {
            if event.sequence <= after {
                continue;
            }
            if bytes + size > MAX_EVENT_BATCH_BYTES {
                break;
            }
            events.push(event.clone());
            through = event.sequence;
            bytes += size;
        }
        Ok((events, through))
    }

    pub fn event_cursor(&self) -> u64 {
        self.sequence
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn invocation() -> Invocation {
        Invocation {
            operation_id: "write-once".into(),
            method: "fs/writeFile".into(),
            params: json!({"path": "file:///fixture", "dataBase64": "YQ=="}),
        }
    }

    #[test]
    fn admitted_effects_are_not_replayed_and_changed_arguments_refuse() {
        let mut ledger = Ledger::new();
        let original = invocation();
        let Admission::New { id, receiver } = ledger.admit(&original).unwrap() else {
            panic!("first admission");
        };
        assert!(matches!(
            ledger.admit(&original).unwrap(),
            Admission::Existing(_)
        ));
        let mut changed = original.clone();
        changed.params["dataBase64"] = json!("Yg==");
        assert!(matches!(
            ledger.admit(&changed),
            Err(Refusal::OperationConflict)
        ));
        ledger.finish(id, json!({"result": {}})).unwrap();
        assert!(matches!(*receiver.borrow(), Outcome::Completed { .. }));
        assert!(matches!(
            ledger.admit(&original).unwrap(),
            Admission::Existing(_)
        ));
    }

    #[test]
    fn loss_retains_completed_evidence_and_unknown_effects() {
        let mut ledger = Ledger::new();
        let original = invocation();
        let Admission::New { receiver, .. } = ledger.admit(&original).unwrap() else {
            panic!("first admission");
        };
        ledger.lose();
        assert_eq!(*receiver.borrow(), Outcome::Unknown);
        assert!(matches!(
            ledger.admit(&original).unwrap(),
            Admission::Existing(_)
        ));
        let mut new = original;
        new.operation_id = "new-id".into();
        assert!(matches!(ledger.admit(&new), Err(Refusal::EnvironmentLost)));
        assert!(ledger.observe("never-admitted").is_none());
    }

    #[test]
    fn result_eviction_keeps_effect_tombstones() {
        let mut ledger = Ledger::new();
        let original = invocation();
        let Admission::New { id, receiver } = ledger.admit(&original).unwrap() else {
            panic!("first admission");
        };
        ledger
            .finish(id, json!({"result": "a".repeat(MAX_RESULTS_BYTES + 1)}))
            .unwrap();
        assert_eq!(*receiver.borrow(), Outcome::ResultExpired);
        assert!(matches!(
            ledger.admit(&original).unwrap(),
            Admission::Existing(_)
        ));
    }

    #[test]
    fn event_backpressure_preserves_unacknowledged_output_and_terminal_events() {
        let mut ledger = Ledger::new();
        for _ in 0..3 {
            ledger
                .push_event(json!({"data": "a".repeat(1024 * 1024)}))
                .unwrap();
        }
        let fourth = json!({"data": "b".repeat(1024 * 1024)});
        let Err(EventError::Full(retained)) = ledger.push_event(fourth.clone()) else {
            panic!("full buffer must apply backpressure");
        };
        assert_eq!(retained, fourth);
        assert_eq!(ledger.event_cursor(), 3);
        let (events, through) = ledger.events(0).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(through, 1);
        assert_eq!(ledger.events(0).unwrap().0, events);
        assert_eq!(ledger.events(through).unwrap().1, 2);
        ledger.push_event(retained).unwrap();
        ledger
            .push_event(json!({"method":"process/exited"}))
            .unwrap();
        ledger
            .push_event(json!({"method":"process/closed"}))
            .unwrap();
        let mut cursor = through;
        let mut collected = events;
        while cursor < ledger.event_cursor() {
            let (batch, next) = ledger.events(cursor).unwrap();
            assert!(next > cursor);
            collected.extend(batch);
            cursor = next;
        }
        assert_eq!(
            collected
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            (1..=6).collect::<Vec<_>>()
        );
        assert_eq!(collected[3].message, fourth);
        assert_eq!(collected[5].message["method"], "process/closed");
        assert!(ledger.events(cursor).unwrap().0.is_empty());
        assert_eq!(ledger.event_bytes, 0);
        assert!(matches!(ledger.events(0), Err(Refusal::CursorExpired)));
        assert!(matches!(ledger.events(7), Err(Refusal::InvalidRequest)));
    }

    #[test]
    fn event_count_limit_applies_backpressure_without_losing_tiny_events() {
        let mut ledger = Ledger::new();
        for _ in 0..8192 {
            ledger.push_event(json!({"method":"tiny"})).unwrap();
        }
        assert!(matches!(
            ledger.push_event(json!({"method":"tail"})),
            Err(EventError::Full(_))
        ));
        assert_eq!(ledger.events(0).unwrap().0.len(), 8192);
        assert!(ledger.events(8192).unwrap().0.is_empty());
        ledger.push_event(json!({"method":"tail"})).unwrap();
        assert_eq!(ledger.events(8192).unwrap().0[0].sequence, 8193);
        assert!(matches!(
            ledger.push_event(json!({"data":"a".repeat(MAX_EVENT_BATCH_BYTES)})),
            Err(EventError::Invalid)
        ));
        assert!(matches!(ledger.events(9000), Err(Refusal::InvalidRequest)));
    }
}
