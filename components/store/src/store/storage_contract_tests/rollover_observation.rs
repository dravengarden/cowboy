use super::{Envelope, Event, HashMap, Store, session, test_support};
use serde_json::json;

async fn assert_durable_boundary(store: &Store) {
    let id = "rollover-history";
    store.insert_session(&session(id)).await.unwrap();
    let event = |seq, update| Envelope {
        session_id: id.into(),
        seq,
        event: Event::Update { update },
        cmid: None,
    };
    let append = |rows: Vec<Envelope>, next| async move {
        store
            .upsert_event_batch(&rows, &HashMap::from([(id.into(), next)]))
            .await
            .unwrap();
    };
    assert_eq!(store.latest_context_rollover(id).await.unwrap(), None);
    append(
        vec![
            event(17, json!({"sessionUpdate":"context_rolled_over", "before":16})),
            event(19, json!({"sessionUpdate":"context_rolled_over", "before":19})),
            event(32, json!({"sessionUpdate":"agent_message_chunk", "content":{"type":"text","text":"later"}})),
        ],
        33,
    ).await;
    assert_eq!(store.latest_context_rollover(id).await.unwrap(), Some(19));
    append(
        vec![event(38, json!({"sessionUpdate":"context_cleared"}))],
        39,
    )
    .await;
    assert_eq!(store.latest_context_rollover(id).await.unwrap(), None);
}

#[tokio::test]
async fn sqlite_latest_context_boundary_is_durable() {
    assert_durable_boundary(&test_support::sqlite().await.store).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_latest_context_boundary_is_durable() {
    assert_durable_boundary(&test_support::postgres().await.store).await;
}
