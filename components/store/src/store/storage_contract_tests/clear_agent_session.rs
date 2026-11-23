use super::{Store, session, test_support};
use crate::model::settings_keys;
use serde_json::json;

/// Clear forgets the native id and records its fence in one write.
async fn assert_clear_is_one_write(store: &Store) {
    let id = "clear-agent-session";
    let meta = session(id);
    store.insert_session(&meta).await.unwrap();
    store
        .update_agent_session_id(id, Some("native-1"))
        .await
        .unwrap();
    let key = format!(
        "{}{id}",
        crate::model::settings_keys::SESSION_CLEARED_NATIVE_PREFIX
    );
    store
        .clear_agent_session_id(id, &key, &json!(["native-1"]))
        .await
        .unwrap();
    let row = store
        .load_all()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.meta.id == id)
        .unwrap();
    assert_eq!(row.meta.agent_session_id, None);
    let settings = store.load_settings().await.unwrap();
    assert!(settings.contains(&(key.clone(), json!(["native-1"]))));
    assert!(
        store
            .clear_agent_session_id(id, "unregistered", &json!([]))
            .await
            .is_err()
    );

    // Clear conversation: transcript, native id and fence in one write.
    let id = "clear-conversation";
    let meta = session(id);
    store.insert_session(&meta).await.unwrap();
    store
        .update_agent_session_id(id, Some("native-2"))
        .await
        .unwrap();
    let event = super::Envelope {
        session_id: id.into(),
        seq: 1,
        event: super::Event::Update {
            update: json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old"}}),
        },
        cmid: None,
    };
    store
        .upsert_event_batch(&[event], &std::collections::HashMap::from([(id.into(), 2)]))
        .await
        .unwrap();
    let key = format!(
        "{}{id}",
        crate::model::settings_keys::SESSION_CLEARED_NATIVE_PREFIX
    );
    let lost = format!("{}{id}", settings_keys::SESSION_OWNER_TURN_LOST_PREFIX);
    // A refused fence keeps the transcript and the native id.
    assert!(
        store
            .clear_conversation(id, i64::MAX, Some(("unregistered", &json!([]))), None)
            .await
            .is_err()
    );
    let native = |rows: Vec<super::LoadedSession>| {
        rows.into_iter()
            .find(|row| row.meta.id == id)
            .unwrap()
            .meta
            .agent_session_id
    };
    assert_eq!(
        native(store.load_all().await.unwrap()).as_deref(),
        Some("native-2")
    );
    store
        .clear_conversation(
            id,
            i64::MAX,
            Some((&key, &json!(["native-2"]))),
            Some((&lost, &json!(41))),
        )
        .await
        .unwrap();
    assert_eq!(native(store.load_all().await.unwrap()), None);
    // The lost owner turn is written by the same transaction.
    let settings = store.load_settings().await.unwrap();
    assert!(settings.contains(&(key, json!(["native-2"]))));
    assert!(settings.contains(&(lost, json!(41))));
}

#[tokio::test]
async fn sqlite_clear_agent_session_is_one_write() {
    assert_clear_is_one_write(&test_support::sqlite().await.store).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_clear_agent_session_is_one_write() {
    assert_clear_is_one_write(&test_support::postgres().await.store).await;
}
