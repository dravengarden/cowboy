//! A completed compaction that no usage report follows leaves the session's
//! context reading as history: shown as unknown and read by no renewal
//! decision until the agent's next real `usage_update`.
use super::*;
use crate::agent_model::SessionUsage;

fn hub_with(id: &str) -> (Hub, StoreReceiver) {
    let health = std::sync::Arc::new(PersistenceHealth::default());
    let (sink, rx) = StoreSink::channel(64, health);
    let hub = Hub::with_store(Some(sink));
    hub.create_local_session(
        id.to_owned(),
        "antigravity".to_owned(),
        "/tmp".to_owned(),
        "compaction".to_owned(),
        SessionOrigin::Api,
        false,
    );
    (hub, rx)
}

fn live(used: u64) -> SessionUsage {
    SessionUsage {
        used,
        size: 1_048_576,
        raw: serde_json::json!({"sessionUpdate": "usage_update", "used": used, "size": 1_048_576}),
        observed_at_ms: 1,
        compacted_at_ms: None,
    }
}

fn compaction(hub: &Hub, id: &str, status: &str) {
    hub.push(
        id,
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "compaction_update",
                "compactionId": "c-1",
                "status": status,
            }),
        },
    );
}

fn turn_end(hub: &Hub, id: &str) {
    hub.push(
        id,
        Event::TurnEnd {
            usage: None,
            stop_reason: "end_turn".to_owned(),
        },
    );
}

fn meta(hub: &Hub, id: &str) -> SessionMeta {
    hub.session_info(id).expect("session").meta
}

fn persisted(rx: &mut StoreReceiver) -> Vec<Option<SessionUsage>> {
    let mut writes = Vec::new();
    while let Ok(write) = rx.try_recv() {
        if let StoreWrite::UpdateSessionUsage { usage, .. } = write {
            writes.push(usage);
        }
    }
    writes
}

#[test]
fn a_compaction_without_a_usage_report_leaves_the_reading_as_history() {
    let (hub, mut rx) = hub_with("agy");
    hub.set_session_usage("agy", live(492_536));
    compaction(&hub, "agy", "in_progress");
    persisted(&mut rx);

    compaction(&hub, "agy", "completed");
    turn_end(&hub, "agy");
    let stale = meta(&hub, "agy");
    assert!(stale.context_usage_stale());
    // The counters remain, as history only.
    assert_eq!(
        (stale.context_used, stale.context_size),
        (492_536, 1_048_576)
    );
    let usage = stale.usage.expect("usage");
    assert!(usage.compacted_at_ms.is_some());
    assert_eq!(
        persisted(&mut rx),
        vec![Some(usage.clone())],
        "the mark is persisted with the reading"
    );
    assert_eq!(hub.verified_context_usage(&meta(&hub, "agy")), None);

    // A reconnecting worker's counter-only snapshot repeats what it last saw.
    hub.set_session_usage(
        "agy",
        SessionUsage {
            raw: serde_json::Value::Null,
            observed_at_ms: 0,
            ..live(492_536)
        },
    );
    assert!(meta(&hub, "agy").context_usage_stale());
    assert!(persisted(&mut rx).is_empty());

    // The next real report replaces the reading and clears the mark, even
    // when its counters happen to repeat the old ones.
    hub.set_session_usage("agy", live(492_536));
    let fresh = meta(&hub, "agy");
    assert!(!fresh.context_usage_stale());
    assert_eq!(fresh.usage, Some(live(492_536)));
    assert_eq!(persisted(&mut rx), vec![Some(live(492_536))]);
}

#[test]
fn usage_reported_during_the_compaction_keeps_the_reading_current() {
    // Codex recomputes the compacted context and reports it before the
    // compaction's completion.
    let (hub, _rx) = hub_with("codex");
    hub.set_session_usage("codex", live(400_000));
    compaction(&hub, "codex", "in_progress");
    hub.set_session_usage("codex", live(42_000));
    compaction(&hub, "codex", "completed");
    turn_end(&hub, "codex");
    let current = meta(&hub, "codex");
    assert!(!current.context_usage_stale());
    assert_eq!(current.context_used, 42_000);
}

#[test]
fn usage_reported_right_after_the_completion_replaces_the_mark() {
    // Claude Code reports the retained tokens at its compaction boundary,
    // which can follow the typed completion.
    let (hub, _rx) = hub_with("claude");
    hub.set_session_usage("claude", live(400_000));
    compaction(&hub, "claude", "in_progress");
    compaction(&hub, "claude", "completed");
    assert!(meta(&hub, "claude").context_usage_stale());
    hub.set_session_usage("claude", live(30_000));
    turn_end(&hub, "claude");
    let current = meta(&hub, "claude");
    assert!(!current.context_usage_stale());
    assert_eq!(current.context_used, 30_000);
}

#[test]
fn a_compaction_that_did_not_complete_changes_nothing() {
    let (hub, mut rx) = hub_with("failed");
    hub.set_session_usage("failed", live(400_000));
    for status in ["failed", "cancelled"] {
        compaction(&hub, "failed", "in_progress");
        compaction(&hub, "failed", status);
        turn_end(&hub, "failed");
    }
    assert!(!meta(&hub, "failed").context_usage_stale());
    assert_eq!(persisted(&mut rx), vec![Some(live(400_000))]);
}

#[test]
fn an_ended_turn_forgets_usage_from_an_unfinished_compaction() {
    let (hub, _rx) = hub_with("ended");
    hub.set_session_usage("ended", live(400_000));
    compaction(&hub, "ended", "in_progress");
    hub.set_session_usage("ended", live(410_000));
    turn_end(&hub, "ended");
    // A later completion without its start saw no report of its own.
    compaction(&hub, "ended", "completed");
    assert!(meta(&hub, "ended").context_usage_stale());
}

#[test]
fn clear_discards_the_stale_reading() {
    let (hub, mut rx) = hub_with("clear");
    hub.set_session_usage("clear", live(400_000));
    compaction(&hub, "clear", "completed");
    assert!(meta(&hub, "clear").context_usage_stale());
    persisted(&mut rx);
    hub.prepare_context_reset("clear");
    let cleared = meta(&hub, "clear");
    assert!(cleared.usage.is_none());
    assert!(!cleared.context_usage_stale());
    assert!(persisted(&mut rx).contains(&None));
}

#[test]
fn a_session_without_a_reading_has_nothing_to_mark() {
    let (hub, mut rx) = hub_with("empty");
    compaction(&hub, "empty", "in_progress");
    compaction(&hub, "empty", "completed");
    assert!(meta(&hub, "empty").usage.is_none());
    assert!(persisted(&mut rx).is_empty());
}

#[test]
fn the_mark_survives_serialization_and_older_readings_have_none() {
    let mut usage = live(1);
    usage.compacted_at_ms = Some(5);
    let value = serde_json::to_value(&usage).expect("serialize");
    assert_eq!(value["compacted_at_ms"], 5);
    let restored: SessionUsage = serde_json::from_value(value).expect("deserialize");
    assert!(restored.stale_after_compaction());
    let older: SessionUsage =
        serde_json::from_value(serde_json::json!({"used": 1, "size": 2, "raw": null}))
            .expect("older reading");
    assert!(!older.stale_after_compaction());
    assert!(
        serde_json::to_value(live(1))
            .expect("serialize")
            .get("compacted_at_ms")
            .is_none()
    );
}
