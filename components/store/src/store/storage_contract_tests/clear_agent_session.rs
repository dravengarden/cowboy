use super::{Store, session, test_support};
use serde_json::json;

/// Clear forgets the native id and records its fence in one write.
async fn assert_clear_is_one_write(store: &Store) {
    let id = "clear-agent-session";
    let mut meta = session(id);
    meta.agent_session_id = Some("native-1".into());
    store.insert_session(&meta).await.unwrap();
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
