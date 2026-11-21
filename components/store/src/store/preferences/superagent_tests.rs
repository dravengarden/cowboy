use super::*;
#[tokio::test]
async fn superagent_unsent_retry_is_atomic_and_candidate_pages_do_not_lose_ties() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", dir.path().into())
        .await
        .unwrap();
    store.migrate().await.unwrap();
    contract(&store).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
async fn postgres_superagent_contract() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect(&url, dir.path().into()).await.unwrap();
    store.migrate().await.unwrap();
    contract(&store).await;
}

async fn contract(store: &Store) {
    assert_eq!(store.superagent_projects("owner").await.unwrap(), json!({}));
    let broken = json!({"scope":"superagent","revision":1,"values":{"configuration":{
        "sources":[{"unknown_future_field":true}], "projects":{"project":{"coordinator":"coordinator"}}, "sessions":{}
    }}}).to_string();
    match &store.backend {
        StorageBackend::Postgres(db) => {
            sqlx::query("INSERT INTO user_preferences(owner_user_id,scope,value) VALUES ($1,'superagent',$2)").bind("owner").bind(&broken).execute(&db.pool).await.unwrap();
        }
        StorageBackend::Sqlite(db) => {
            sqlx::query("INSERT INTO user_preferences(owner_user_id,scope,value) VALUES ($1,'superagent',$2)").bind("owner").bind(&broken).execute(&db.pool).await.unwrap();
        }
    }
    assert_eq!(
        store.superagent_projects("owner").await.unwrap()["project"]["coordinator"],
        "coordinator"
    );
    let repair = crate::store::preferences::PreferenceChange {
        scope: "superagent".into(),
        expected_revision: 1,
        changes: json!({"configuration":{"sources":[],"projects":{},"sessions":{}}}),
    };
    assert!(matches!(
        store
            .update_preferences("owner", "repair", &repair)
            .await
            .unwrap(),
        crate::store::preferences::PreferenceResult::Applied(_)
    ));
    assert_eq!(store.superagent_projects("owner").await.unwrap(), json!({}));
    for i in 0..101 {
        store
            .admit_agent_operation("owner", "worker", &format!("sa-candidate-{i:03}"), "hash")
            .await
            .unwrap();
    }
    let first = store
        .superagent_activity("owner", &(i64::MAX, String::new(), String::new()))
        .await
        .unwrap();
    assert_eq!(first.len(), 100);
    let last = first.last().unwrap();
    let cursor = (
        last["created_at_ms"].as_i64().unwrap(),
        last["operation"].as_str().unwrap().into(),
        last["session_id"].as_str().unwrap().into(),
    );
    let rest = store.superagent_activity("owner", &cursor).await.unwrap();
    assert_eq!(rest.len(), 1);
    assert!(
        !first
            .iter()
            .any(|row| row["operation"] == rest[0]["operation"])
    );
    let previous = json!({"status":"not_dispatched","turn_seq":1});
    assert!(
        !store
            .retry_unsent_superagent_operation(
                "owner",
                "worker",
                "sa-candidate-000",
                &previous,
                "new"
            )
            .await
            .unwrap()
    );
    store
        .finish_agent_operation("owner", "worker", "sa-candidate-000", &previous)
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        store.retry_unsent_superagent_operation(
            "owner",
            "worker",
            "sa-candidate-000",
            &previous,
            "new"
        ),
        store.retry_unsent_superagent_operation(
            "owner",
            "worker",
            "sa-candidate-000",
            &previous,
            "new"
        )
    );
    assert_ne!(a.unwrap(), b.unwrap());
    assert_eq!(
        store
            .agent_operation_receipt("owner", "worker", "sa-candidate-000")
            .await
            .unwrap(),
        Some(None)
    );
}
