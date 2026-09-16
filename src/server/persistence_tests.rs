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
        session_id: "fixture".into()
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
