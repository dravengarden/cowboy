use super::*;
use std::time::Duration;

fn session(id: &str) -> crate::core::SessionMeta {
    let hub = Hub::new();
    hub.create_local_session(
        id.into(),
        "codex".into(),
        "/tmp".into(),
        "Persistence fixture".into(),
        SessionOrigin::Api,
        false,
    );
    let mut meta = hub.session_info(id).unwrap().meta;
    // Both immutable baselines seed this local Machine, not the Hub-only alias.
    meta.machine_id = "hawk".into();
    meta
}

fn text(seq: u64, bytes: usize) -> StoreWrite {
    StoreWrite::AppendEvent(Envelope {
        session_id: "fixture".into(),
        seq,
        cmid: None,
        event: Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "messageId": seq.to_string(),
                "content": {"type": "text", "text": "x".repeat(bytes)},
            }),
        },
    })
}

fn title(value: &str) -> StoreWrite {
    StoreWrite::UpdateTitle {
        session_id: "fixture".into(),
        title: value.into(),
    }
}

fn assert_healthy_drained(health: &PersistenceHealth) {
    assert_eq!(health.pending(), 0);
    assert_eq!(health.pending_bytes(), 0);
    assert_eq!(health.dropped(), 0);
    assert_eq!(health.failed_batches(), 0);
    assert!(health.is_healthy());
}

async fn large_event_contract(url: &str, root: &FsPath) {
    let store = Store::connect(url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();
    store.insert_session(&session("fixture")).await.unwrap();
    store.insert_session(&session("independent")).await.unwrap();
    let health = Arc::new(PersistenceHealth::default());
    let (sink, rx) = StoreSink::channel(4, health.clone());
    assert!(sink.send(text(0, 16 * 1024 * 1024)));
    assert!(sink.send(text(1, 437)));
    assert!(sink.send(StoreWrite::AppendEvent(Envelope {
        session_id: "independent".into(),
        seq: 0,
        cmid: None,
        event: Event::TurnEnd {
            usage: None,
            stop_reason: "done".into()
        },
    })));
    assert!(sink.send(title("old")));
    // These intents use the count reserve behind a completely full queue.
    assert!(sink.send(title("latest")));
    for value in ["old", "latest"] {
        assert!(sink.send(StoreWrite::PutSetting {
            key: crate::admin::REGISTRATION_SETTING.into(),
            value: serde_json::json!(value),
        }));
    }
    let (shutdown_tx, shutdown) = watch::channel(false);
    shutdown_tx.send(true).unwrap();
    tokio::time::timeout(
        Duration::from_secs(30),
        run_store_writer(store, rx, health.clone(), shutdown),
    )
    .await
    .expect("accepted writes must drain with senders still alive");
    assert_healthy_drained(&health);

    // Reopen through the real backend-neutral Store. Never inspect the live DB.
    let reopened = Store::connect(url, root.join("artifacts")).await.unwrap();
    reopened.migrate().await.unwrap();
    let sessions = reopened.load_all().await.unwrap();
    let primary = sessions
        .iter()
        .find(|row| row.meta.id == "fixture")
        .unwrap();
    assert_eq!(primary.meta.title, "latest");
    assert_eq!(primary.next_seq, 2);
    assert_eq!(primary.event_count, 2);
    let independent = sessions
        .iter()
        .find(|row| row.meta.id == "independent")
        .unwrap();
    assert_eq!(independent.next_seq, 1);
    assert_eq!(independent.event_count, 1);
    for (before, seq, expected_bytes) in [(1, 0, 16 * 1024 * 1024), (2, 1, 437)] {
        let (events, _, _) = reopened.history_page("fixture", before, 1).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].seq, seq);
        let Event::Update { update } = &events[0].event else {
            panic!("expected text")
        };
        let actual = update["content"]["text"].as_str().unwrap();
        assert_eq!(actual.len(), expected_bytes);
        assert!(actual.bytes().all(|byte| byte == b'x'));
    }
    let settings = reopened.load_settings().await.unwrap();
    assert!(
        settings
            .iter()
            .any(|(key, value)| { key == crate::admin::REGISTRATION_SETTING && value == "latest" })
    );
}

#[tokio::test]
async fn sqlite_large_then_small_and_reserved_writes_survive_shutdown_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("fixture.sqlite").display());
    large_event_contract(&url, root.path()).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_large_then_small_and_reserved_writes_survive_shutdown_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
        .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
    large_event_contract(&url, root.path()).await;
}

#[tokio::test]
async fn queued_clear_and_later_events_keep_fifo_through_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    store.insert_session(&session("fixture")).await.unwrap();
    let health = Arc::new(PersistenceHealth::default());
    let (sink, rx) = StoreSink::channel(3, health.clone());
    assert!(sink.send(text(0, 128)));
    assert!(sink.send(StoreWrite::ClearEvents {
        session_id: "fixture".into(),
        calls_before_ms: 1,
    }));
    assert!(sink.send(text(1, 12)));
    // Reserved control writes cannot overtake the earlier clear/new event pair.
    assert!(sink.send(title("after clear")));
    assert!(sink.send(title("latest")));
    let (shutdown_tx, shutdown) = watch::channel(false);
    shutdown_tx.send(true).unwrap();
    tokio::time::timeout(
        Duration::from_secs(5),
        run_store_writer(store.clone(), rx, health.clone(), shutdown),
    )
    .await
    .unwrap();
    assert_healthy_drained(&health);
    let loaded = store.load_all().await.unwrap();
    assert_eq!(loaded[0].meta.title, "latest");
    assert_eq!(loaded[0].event_count, 1);
    assert_eq!(loaded[0].events[0].seq, 1);
    assert_eq!(loaded[0].next_seq, 2);
}

#[tokio::test]
async fn initially_signalled_or_dropped_shutdown_drains_without_spinning() {
    for initially_signalled in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        store.insert_session(&session("fixture")).await.unwrap();
        let health = Arc::new(PersistenceHealth::default());
        let (sink, rx) = StoreSink::channel(1, health.clone());
        assert!(sink.send(title("drained")));
        let (shutdown_tx, shutdown) = watch::channel(initially_signalled);
        if !initially_signalled {
            drop(shutdown_tx);
        }
        tokio::time::timeout(
            Duration::from_secs(5),
            run_store_writer(store.clone(), rx, health.clone(), shutdown),
        )
        .await
        .unwrap();
        assert_healthy_drained(&health);
        assert_eq!(store.load_all().await.unwrap()[0].meta.title, "drained");
    }
}

// ---------------------------------------------------------------------------
// Background tasks Phase 1 (docs/background-tasks.md): one durable transcript
// anchor per subagent child, shared by every store-writer lifetime.

fn subagent_write(session: &str, seq: u64, child: &str, state: &str, revision: u64) -> StoreWrite {
    StoreWrite::AppendEvent(Envelope {
        session_id: session.into(),
        seq,
        cmid: None,
        event: Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "cowboy_subagent",
                "sessionId": child,
                "revision": revision,
                "state": state,
                "stopReason": if state == "idle" { serde_json::json!("end_turn") } else { serde_json::Value::Null },
                "title": "Explore",
            }),
        },
    })
}

/// Every durable `cowboy_subagent` row of `child`, oldest first.
async fn durable_subagent_rows(store: &Store, session: &str, child: &str) -> Vec<(u64, String)> {
    let (events, _, _) = store
        .history_page(session, u64::MAX >> 2, 100_000)
        .await
        .unwrap();
    let mut rows: Vec<(u64, String)> = events
        .into_iter()
        .filter_map(|envelope| match envelope.event {
            Event::Update { update }
                if update["sessionUpdate"] == "cowboy_subagent" && update["sessionId"] == child =>
            {
                Some((
                    envelope.seq,
                    update["state"].as_str().unwrap_or_default().to_owned(),
                ))
            }
            _ => None,
        })
        .collect();
    rows.sort();
    rows
}

/// T3: the store writer restarts between a child's first row and its terminal
/// snapshot (a Controller restart). The terminal snapshot must replace the
/// original anchor row instead of starting a second one.
async fn writer_restart_keeps_one_anchor_contract(url: &str, root: &FsPath) {
    let store = Store::connect(url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();
    store.insert_session(&session("fixture")).await.unwrap();
    let mut first_lifetime = EventReducer::default();
    assert!(
        apply_store_batch(
            &store,
            &mut first_lifetime,
            vec![subagent_write("fixture", 5, "child-a", "running", 10)],
        )
        .await
    );
    // A new Controller process starts with a fresh writer reducer.
    let mut second_lifetime = EventReducer::default();
    assert!(
        apply_store_batch(
            &store,
            &mut second_lifetime,
            vec![subagent_write("fixture", 400, "child-a", "idle", 20)],
        )
        .await
    );
    assert_eq!(
        durable_subagent_rows(&store, "fixture", "child-a").await,
        vec![(5, "idle".to_owned())],
        "one durable row, at the original anchor, holding the terminal state"
    );
}

#[tokio::test]
async fn sqlite_t3_writer_restart_keeps_one_subagent_anchor() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("fixture.sqlite").display());
    writer_restart_keeps_one_anchor_contract(&url, root.path()).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_t3_writer_restart_keeps_one_subagent_anchor() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
        .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
    writer_restart_keeps_one_anchor_contract(&url, root.path()).await;
}

/// T4: more than 4096 children across sessions must not make a known child
/// look new (the reducer used to clear its whole map at that bound).
async fn reducer_eviction_keeps_one_anchor_contract(url: &str, root: &FsPath) {
    let store = Store::connect(url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();
    store.insert_session(&session("fixture")).await.unwrap();
    store.insert_session(&session("independent")).await.unwrap();
    let mut reducer = EventReducer::default();
    assert!(
        apply_store_batch(
            &store,
            &mut reducer,
            vec![subagent_write("fixture", 1, "child-a", "running", 1)],
        )
        .await
    );
    for chunk in 0..17u64 {
        let writes = (0..256u64)
            .map(|index| {
                let n = chunk * 256 + index;
                let session = if n % 2 == 0 { "fixture" } else { "independent" };
                subagent_write(session, 10 + n, &format!("other-{n}"), "running", 1)
            })
            .collect();
        assert!(apply_store_batch(&store, &mut reducer, writes).await);
    }
    assert!(
        apply_store_batch(
            &store,
            &mut reducer,
            vec![subagent_write("fixture", 9_000, "child-a", "idle", 2)],
        )
        .await
    );
    assert_eq!(
        durable_subagent_rows(&store, "fixture", "child-a").await,
        vec![(1, "idle".to_owned())],
        "eviction must resolve the anchor again instead of starting a new row"
    );

    // A writer whose LRU evicts the key resolves it through the store.
    let mut small = EventReducer::with_anchor_capacity(2);
    let writes = vec![
        subagent_write("fixture", 9_001, "child-b", "running", 1),
        subagent_write("fixture", 9_002, "child-c", "running", 1),
        subagent_write("fixture", 9_003, "child-d", "running", 1),
        subagent_write("fixture", 9_004, "child-b", "idle", 2),
    ];
    assert!(apply_store_batch(&store, &mut small, writes).await);
    assert_eq!(
        durable_subagent_rows(&store, "fixture", "child-b").await,
        vec![(9_001, "idle".to_owned())]
    );

    // Rows written before the anchor table existed are backfilled once.
    store.insert_session(&session("legacy")).await.unwrap();
    let legacy =
        |seq, state, revision| match subagent_write("legacy", seq, "child-old", state, revision) {
            StoreWrite::AppendEvent(envelope) => envelope,
            _ => unreachable!(),
        };
    store
        .upsert_event_batch(
            &[legacy(3, "running", 1)],
            &HashMap::from([("legacy".to_owned(), 4)]),
        )
        .await
        .unwrap();
    let backfills = crate::transcript_anchors::METRICS
        .backfills
        .load(std::sync::atomic::Ordering::Relaxed);
    let mut restarted = EventReducer::default();
    assert!(
        apply_store_batch(
            &store,
            &mut restarted,
            vec![StoreWrite::AppendEvent(legacy(50, "idle", 2))],
        )
        .await
    );
    assert_eq!(
        durable_subagent_rows(&store, "legacy", "child-old").await,
        vec![(3, "idle".to_owned())]
    );
    assert!(
        crate::transcript_anchors::METRICS
            .backfills
            .load(std::sync::atomic::Ordering::Relaxed)
            > backfills
    );
    assert_eq!(
        store.backfill_subagent_anchors("legacy").await.unwrap(),
        None,
        "scanned once"
    );
}

#[tokio::test]
async fn sqlite_t4_reducer_eviction_keeps_one_subagent_anchor() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("fixture.sqlite").display());
    reducer_eviction_keeps_one_anchor_contract(&url, root.path()).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_t4_reducer_eviction_keeps_one_subagent_anchor() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
        .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
    reducer_eviction_keeps_one_anchor_contract(&url, root.path()).await;
}
