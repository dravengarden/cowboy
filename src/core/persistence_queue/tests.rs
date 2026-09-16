use super::*;
use crate::core::{Envelope, QueuedMessage};
use std::sync::Barrier;
use std::time::Duration;

fn event(seq: u64, bytes: usize) -> StoreWrite {
    StoreWrite::AppendEvent(Envelope {
        session_id: "fixture".into(),
        seq,
        cmid: None,
        event: Event::Update {
            update: serde_json::json!({"sessionUpdate":"agent_message_chunk", "content":{"type":"text", "text":"x".repeat(bytes)}}),
        },
    })
}

fn control(title: &str) -> StoreWrite {
    StoreWrite::UpdateTitle {
        session_id: "fixture".into(),
        title: title.into(),
    }
}

fn channel(capacity: usize) -> (StoreSink, StoreReceiver, Arc<PersistenceHealth>) {
    let health = Arc::new(PersistenceHealth::default());
    let (sink, receiver) = StoreSink::channel(capacity, health.clone());
    (sink, receiver, health)
}

fn drain(receiver: &mut StoreReceiver) -> Vec<StoreWrite> {
    std::iter::from_fn(|| receiver.try_recv().ok()).collect()
}

#[test]
fn sixteen_mib_event_does_not_reject_following_small_events() {
    let (sink, mut receiver, health) = channel(16);
    let large = event(1, 16 * 1024 * 1024);
    let bytes = estimated_store_write_bytes(&large);
    assert!(sink.send(large));
    assert!(sink.send(event(2, 437)));
    assert!(sink.send(StoreWrite::AppendEvent(Envelope {
        session_id: "independent-session".into(),
        seq: 3,
        cmid: None,
        event: Event::TurnEnd {
            stop_reason: "done".into()
        },
    })));
    assert_eq!(health.pending(), 3);
    assert!(health.pending_bytes() > bytes);
    let writes = drain(&mut receiver);
    assert_eq!(writes.len(), 3);
    for (write, seq) in writes.iter().zip(1..=3) {
        assert!(matches!(write, StoreWrite::AppendEvent(envelope) if envelope.seq == seq));
    }
    let StoreWrite::AppendEvent(envelope) = &writes[0] else {
        unreachable!()
    };
    let Event::Update { update } = &envelope.event else {
        unreachable!()
    };
    assert_eq!(
        update["content"]["text"].as_str().unwrap().len(),
        16 * 1024 * 1024
    );
    assert_eq!(health.pending(), 0);
    assert_eq!(health.pending_bytes(), 0);
    assert_eq!(health.dropped(), 0);
    assert!(health.is_healthy());
}

#[test]
fn oversized_slot_is_independent_from_ordinary_queue_occupancy() {
    let (sink, mut receiver, health) = channel(8);
    assert!(sink.send(event(1, 1024)));
    assert!(sink.send(event(2, NORMAL_BYTES + 1)));
    assert!(sink.send(event(3, 1024)));
    assert_eq!(drain(&mut receiver).len(), 3);
    assert!(sink.send(event(4, NORMAL_BYTES + 1)));
    assert_eq!(drain(&mut receiver).len(), 1);
    assert!(health.is_healthy());
    assert_eq!(health.pending_bytes(), 0);
}

#[test]
fn concurrent_large_producers_cannot_each_borrow_an_oversized_slot() {
    let (sink, mut receiver, health) = channel(16);
    let barrier = Arc::new(Barrier::new(8));
    let admitted = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|seq| {
                let barrier = barrier.clone();
                let sink = sink.clone();
                scope.spawn(move || {
                    let write = event(seq, NORMAL_BYTES + 1);
                    barrier.wait();
                    sink.send(write)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| usize::from(handle.join().unwrap()))
            .sum::<usize>()
    });
    assert_eq!(admitted, 1);
    assert_eq!(health.dropped(), 7);
    assert_eq!(drain(&mut receiver).len(), 1);
    assert_eq!(health.pending_bytes(), 0);
    assert!(
        !health.is_healthy(),
        "draining cannot conceal rejected events"
    );
}

#[test]
fn concurrent_normal_admission_and_refunds_use_one_budget() {
    let (sink, mut receiver, health) = channel(32);
    let each = estimated_store_write_bytes(&event(0, 1024 * 1024));
    let admitted = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..16)
            .map(|seq| {
                let sink = sink.clone();
                scope.spawn(move || sink.send(event(seq, 1024 * 1024)))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| usize::from(handle.join().unwrap()))
            .sum::<usize>()
    });
    assert_eq!(admitted, NORMAL_BYTES / each);
    assert!(health.pending_bytes() <= NORMAL_BYTES);
    assert_eq!(drain(&mut receiver).len(), admitted);
    assert_eq!(health.pending(), 0);
    assert_eq!(health.pending_bytes(), 0);
    assert!(
        sink.send(event(20, 1024 * 1024)),
        "degradation is evidence, not a permanent admission lock"
    );
    assert_eq!(drain(&mut receiver).len(), 1);
    assert!(!health.is_healthy());
}

#[test]
fn reserved_control_slots_are_finite_and_need_no_runtime_or_detached_task() {
    let (sink, mut receiver, health) = channel(1);
    assert!(sink.send(event(0, 1)));
    for index in 0..RESERVED_SLOTS {
        assert!(sink.send(control(&index.to_string())));
    }
    assert!(!sink.send(control("over capacity")));
    let writes = drain(&mut receiver);
    assert_eq!(writes.len(), RESERVED_SLOTS + 1);
    for (index, write) in writes.iter().skip(1).enumerate() {
        assert!(
            matches!(write, StoreWrite::UpdateTitle { title, .. } if title == &index.to_string())
        );
    }
    assert_eq!(health.dropped(), 1);
    assert_eq!(health.pending_bytes(), 0);
}

#[test]
fn lifecycle_has_a_bounded_byte_reservation_not_an_unlimited_bypass() {
    let (sink, mut receiver, health) = channel(1024);
    assert!(sink.send(event(0, NORMAL_BYTES - 256)));
    let title = "c".repeat(1024);
    let mut accepted = 0;
    while sink.send(control(&title)) {
        accepted += 1;
    }
    assert!(accepted > 0 && accepted < 256);
    let state = sink.shared.state.lock();
    assert!(state.normal_bytes <= NORMAL_BYTES);
    assert!(state.reserved_bytes <= RESERVED_BYTES);
    assert!(!state.oversized);
    drop(state);
    assert_eq!(drain(&mut receiver).len(), accepted + 1);
    assert_eq!(health.pending_bytes(), 0);
}

#[test]
fn later_writes_cannot_overtake_a_reserved_control_write() {
    let (sink, mut receiver, health) = channel(2);
    assert!(sink.send(event(0, 1)));
    assert!(sink.send(event(1, 1)));
    assert!(sink.send(control("first")));
    assert!(receiver.try_recv().is_ok());
    assert!(receiver.try_recv().is_ok());
    assert!(sink.send(control("second")));
    let writes = drain(&mut receiver);
    assert!(matches!(&writes[0], StoreWrite::UpdateTitle { title, .. } if title == "first"));
    assert!(matches!(&writes[1], StoreWrite::UpdateTitle { title, .. } if title == "second"));
    assert!(health.is_healthy());
}

#[tokio::test]
async fn closing_admission_keeps_every_accepted_item_in_fifo_until_drained() {
    let (sink, mut receiver, health) = channel(1);
    assert!(sink.send(event(1, 1)));
    assert!(sink.send(control("accepted before close")));
    receiver.close();
    assert!(!sink.send(control("after close")));
    assert!(matches!(
        receiver.recv().await,
        Some(StoreWrite::AppendEvent(_))
    ));
    assert!(matches!(
        receiver.recv().await,
        Some(StoreWrite::UpdateTitle { .. })
    ));
    assert!(receiver.recv().await.is_none());
    assert_eq!(health.pending_bytes(), 0);
    assert_eq!(health.dropped(), 1);
    assert!(!health.is_healthy());
}

#[tokio::test]
async fn last_sender_closes_only_after_clones_drop_and_drains_accepted_writes() {
    let (sink, mut receiver, health) = channel(1);
    let retained = sink.clone();
    drop(sink);
    assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    assert!(retained.send(control("retained sender")));
    drop(retained);
    assert!(receiver.recv().await.is_some());
    assert!(receiver.recv().await.is_none());
    assert!(health.is_healthy());
}

#[tokio::test]
async fn cancelled_empty_receiver_does_not_lose_delivery_or_close_wakeups() {
    let (sink, mut receiver, health) = channel(1);
    assert!(
        tokio::time::timeout(Duration::from_millis(1), receiver.recv())
            .await
            .is_err()
    );
    let waiting = tokio::spawn(async move {
        assert!(receiver.recv().await.is_some());
        assert!(receiver.recv().await.is_none());
    });
    assert!(sink.send(event(1, 1)));
    drop(sink);
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap();
    assert!(health.is_healthy());
}

#[test]
fn dropping_a_nonempty_receiver_refunds_capacity_and_reports_lost_ownership() {
    let (sink, receiver, health) = channel(2);
    assert!(sink.send(event(1, NORMAL_BYTES + 1)));
    assert!(sink.send(control("pending")));
    drop(receiver);
    assert_eq!(health.pending(), 0);
    assert_eq!(health.pending_bytes(), 0);
    assert_eq!(health.dropped(), 2);
    assert!(!sink.send(event(3, 1)));
    assert_eq!(health.dropped(), 3);
    assert_eq!(health.pending_bytes(), 0);
    assert!(!health.is_healthy());
}

#[test]
fn pending_attachments_and_private_settings_are_not_counted_as_tiny_metadata() {
    let content = "x".repeat(2 * 1024 * 1024);
    let write = StoreWrite::UpdatePending {
        session_id: "fixture".into(),
        queue: vec![],
        drafts: vec![QueuedMessage {
            id: "draft".into(),
            text: String::new(),
            cmid: None,
            schedule: None,
            content: vec![serde_json::json!({"type":"resource", "data":content})],
        }],
    };
    assert!(estimated_store_write_bytes(&write) >= content.len());
    let setting = StoreWrite::PutSetting {
        key: "private-fixture".into(),
        value: serde_json::json!({"value":content}),
    };
    assert!(estimated_store_write_bytes(&setting) >= content.len());
    let (sink, mut receiver, health) = channel(2);
    assert!(sink.send(write));
    assert!(sink.send(setting));
    assert!(health.pending_bytes() >= 2 * content.len());
    assert_eq!(drain(&mut receiver).len(), 2);
    assert_eq!(health.pending_bytes(), 0);
}

#[test]
fn single_item_budget_rejects_overflow_even_for_control_writes() {
    let state = State::default();
    for critical in [false, true] {
        assert!(state.charge(MAX_SINGLE_BYTES + 1, critical, 4).is_err());
        assert!(state.charge(usize::MAX, critical, 4).is_err());
        assert!(matches!(
            state.charge(MAX_SINGLE_BYTES, critical, 4),
            Ok(Charge::Oversized)
        ));
    }
}
