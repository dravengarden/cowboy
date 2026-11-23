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
    compaction_with(hub, id, "c-1", status);
}

fn compaction_with(hub: &Hub, id: &str, compaction_id: &str, status: &str) {
    hub.push(
        id,
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "compaction_update",
                "compactionId": compaction_id,
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
    // Nor does one with other counters: only a live report measures the
    // compacted context.
    hub.set_session_usage(
        "agy",
        SessionUsage {
            raw: serde_json::Value::Null,
            observed_at_ms: 0,
            ..live(300_000)
        },
    );
    let kept = meta(&hub, "agy");
    assert!(kept.context_usage_stale());
    assert_eq!(kept.context_used, 492_536);
    assert!(persisted(&mut rx).is_empty());

    // The next real report replaces the reading and clears the mark, even
    // when its counters happen to repeat the old ones.
    let report = SessionUsage {
        observed_at_ms: 2,
        ..live(492_536)
    };
    hub.set_session_usage("agy", report.clone());
    let fresh = meta(&hub, "agy");
    assert!(!fresh.context_usage_stale());
    assert_eq!(fresh.usage, Some(report.clone()));
    assert_eq!(persisted(&mut rx), vec![Some(report)]);
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

#[test]
fn usage_an_older_antigravity_proxy_relays_during_compact_stays_history() {
    // Before cowboy-antigravity-acp 1.0.25 the proxy relayed the usage of the
    // `/compact` prompt itself, which measures the summarized history.
    let (hub, mut rx) = hub_with("agy-old");
    hub.set_session_usage("agy-old", live(480_000));
    compaction_with(&hub, "agy-old", "agy-compact-1", "in_progress");
    hub.set_session_usage("agy-old", live(488_059));
    compaction_with(&hub, "agy-old", "agy-compact-1", "completed");
    turn_end(&hub, "agy-old");
    let stale = meta(&hub, "agy-old");
    assert!(stale.context_usage_stale());
    assert_eq!(stale.context_used, 488_059);
    persisted(&mut rx);
    hub.set_session_usage("agy-old", live(61_000));
    assert!(!meta(&hub, "agy-old").context_usage_stale());
}

#[test]
fn clear_and_compaction_writes_reach_storage_in_their_hub_order() {
    let (hub, mut rx) = hub_with("ordered");
    hub.set_session_usage("ordered", live(400_000));
    compaction(&hub, "ordered", "completed");
    hub.prepare_context_reset("ordered");
    let writes = persisted(&mut rx);
    assert_eq!(writes.last(), Some(&None), "Clear's reset is the last word");
}

#[test]
fn an_older_antigravity_proxy_measures_the_context_a_failed_compaction_keeps() {
    // Its `/compact` prompt's usage measures the context the session holds
    // until the compaction completes: current when it fails or is cancelled,
    // history once it completes.
    let (hub, _rx) = hub_with("agy-again");
    hub.set_session_usage("agy-again", live(480_000));
    compaction_with(&hub, "agy-again", "agy-compact-1", "completed");
    assert!(meta(&hub, "agy-again").context_usage_stale());

    for (id, end, used) in [
        ("agy-compact-2", "failed", 61_000),
        ("agy-compact-3", "cancelled", 62_000),
    ] {
        compaction_with(&hub, "agy-again", id, "in_progress");
        hub.set_session_usage("agy-again", live(used));
        compaction_with(&hub, "agy-again", id, end);
        let after = meta(&hub, "agy-again");
        assert!(
            !after.context_usage_stale(),
            "{end} keeps the context it measured"
        );
        assert_eq!(after.context_used, used);
    }
    compaction_with(&hub, "agy-again", "agy-compact-4", "in_progress");
    hub.set_session_usage("agy-again", live(63_000));
    compaction_with(&hub, "agy-again", "agy-compact-4", "completed");
    assert!(meta(&hub, "agy-again").context_usage_stale());
}

#[test]
fn republished_accounting_is_no_fresh_reading() {
    // Per-model quota rows at a turn end republish the last reading with its
    // observation time and new accounting fields.
    let (hub, mut rx) = hub_with("acct");
    hub.set_session_usage("acct", live(400_000));
    let mut republished = live(400_000);
    republished.raw["_meta"] = serde_json::json!({"model_usage": [{"model": "m", "turns": 1}]});

    compaction(&hub, "acct", "completed");
    persisted(&mut rx);
    hub.set_session_usage("acct", republished.clone());
    let kept = meta(&hub, "acct");
    assert!(
        kept.context_usage_stale(),
        "accounting keeps the stale mark"
    );
    assert_eq!(
        kept.usage.expect("reading").raw["_meta"],
        republished.raw["_meta"]
    );
    assert!(
        persisted(&mut rx)
            .iter()
            .all(|w| w.as_ref().is_some_and(SessionUsage::stale_after_compaction))
    );

    let (hub, _rx) = hub_with("acct-running");
    hub.set_session_usage("acct-running", live(400_000));
    compaction(&hub, "acct-running", "in_progress");
    hub.set_session_usage("acct-running", republished);
    compaction(&hub, "acct-running", "completed");
    assert!(
        meta(&hub, "acct-running").context_usage_stale(),
        "accounting during a compaction reports nothing about the compacted context"
    );
}
