use super::*;
use crate::telemetry_binding::resolution::tests::{intent as resolution_intent, permit};

async fn contract(store: &Store) {
    store.migrate().await.unwrap();
    let intent = fixture("resolution-storage");
    let before = store
        .change_telemetry_binding(&Change::Begin(&intent), &|| true)
        .await
        .unwrap()
        .operation;
    let request = resolution_intent(&before, None);
    let permit = permit(&request, &before, None);
    for expire_at in 0..2 {
        let checks = AtomicUsize::new(0);
        assert!(
            store
                .change_telemetry_binding(&Change::Resolve(&permit), &|| {
                    checks.fetch_add(1, Ordering::Relaxed) < expire_at
                })
                .await
                .is_err()
        );
        let saved = store
            .telemetry_binding_ledger(&intent.service_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.operations, vec![before.clone()]);
        assert!(saved.resolutions.is_empty());
    }
    let expired = crate::telemetry_binding::resolution::tests::permit(&request, &before, None);
    expired.expire_for_test();
    assert!(
        store
            .change_telemetry_binding(&Change::Resolve(&expired), &|| true)
            .await
            .is_err(),
        "a caller cannot bypass the permit's own original budget"
    );
    let advance = Change::Advance {
        expected: &before,
        progress: Progress::Dispatching,
    };
    let resolution = Change::Resolve(&permit);
    let (dispatched, resolved) = tokio::join!(
        store.change_telemetry_binding(&advance, &|| true),
        store.change_telemetry_binding(&resolution, &|| true),
    );
    assert_ne!(
        dispatched.is_ok(),
        resolved.is_ok(),
        "only one full operation CAS wins"
    );
    let mut saved = store
        .telemetry_binding_ledger(&intent.service_id)
        .await
        .unwrap()
        .unwrap();
    if let Ok(dispatched) = dispatched {
        let observation = applied(&intent);
        let request = resolution_intent(&dispatched.operation, Some(&observation));
        let permit = crate::telemetry_binding::resolution::tests::permit(
            &request,
            &dispatched.operation,
            Some(observation),
        );
        store
            .change_telemetry_binding(&Change::Resolve(&permit), &|| true)
            .await
            .unwrap();
        saved = store
            .telemetry_binding_ledger(&intent.service_id)
            .await
            .unwrap()
            .unwrap();
    }
    assert_eq!(saved.resolutions.len(), 1);
    assert!(
        !LegacyFence::recover(Some(store), &intent.service_id)
            .await
            .unwrap()
            .allows_legacy()
    );
    assert!(
        store
            .change_telemetry_binding(&advance, &|| true)
            .await
            .is_err()
    );
    reject_corrupt_audit(store, &saved, &resolution).await;
}

async fn reject_corrupt_audit(store: &Store, saved: &Ledger, resolution: &Change<'_>) {
    let mut corrupted = serde_json::to_value(saved).unwrap();
    corrupted["resolutions"][0]["before"]["intent"]["actor"] =
        serde_json::to_value(crate::plugin_operation::Actor::Product {
            user_id: "changed".into(),
        })
        .unwrap();
    let document = corrupted.to_string();
    let sql = "UPDATE telemetry_binding_journal SET document = $1, document_sha256 = $2 WHERE slot = 'telemetry'";
    match &store.backend {
        StorageBackend::Postgres(db) => {
            sqlx::query(sql)
                .bind(&document)
                .bind(checksum(&document))
                .execute(&db.pool)
                .await
                .unwrap();
        }
        StorageBackend::Sqlite(db) => {
            sqlx::query(sql)
                .bind(&document)
                .bind(checksum(&document))
                .execute(&db.pool)
                .await
                .unwrap();
        }
    }
    assert!(
        store
            .telemetry_binding_ledger(resolution.service())
            .await
            .is_err()
    );
    assert!(
        LegacyFence::recover(Some(store), resolution.service())
            .await
            .is_err()
    );
    assert!(
        store
            .change_telemetry_binding(resolution, &|| true)
            .await
            .is_err(),
        "rechecksummed malformed audit is never repaired by a write"
    );
}

#[tokio::test]
async fn sqlite_resolution_atomicity_races_and_corruption() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    contract(&store).await;
}

#[tokio::test]
#[ignore = "requires the owned isolated PostgreSQL fixture"]
async fn postgres_resolution_atomicity_races_and_corruption() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated PostgreSQL fixture");
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    contract(&store).await;
}

#[tokio::test]
async fn sqlite_restart_retains_schema_two_audit_without_clearing_namespace() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("bindings.sqlite").display());
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let intent = fixture("resolution-restart");
    let before = store
        .change_telemetry_binding(&Change::Begin(&intent), &|| true)
        .await
        .unwrap()
        .operation;
    let request = resolution_intent(&before, None);
    store
        .change_telemetry_binding(&Change::Resolve(&permit(&request, &before, None)), &|| true)
        .await
        .unwrap();
    let saved = store
        .telemetry_binding_ledger(&intent.service_id)
        .await
        .unwrap()
        .unwrap();
    drop(store);
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    assert_eq!(
        saved,
        store
            .telemetry_binding_ledger(&intent.service_id)
            .await
            .unwrap()
            .unwrap()
    );
    assert!(saved.current.is_none());
    assert_eq!(saved.operations[0].progress, Progress::Aborted);
    assert!(
        !LegacyFence::recover(Some(&store), &intent.service_id)
            .await
            .unwrap()
            .allows_legacy()
    );
}

#[tokio::test]
async fn sqlite_queued_resolution_cannot_renew_its_permit_after_lock_wait() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let intent = fixture("resolution-lock-wait");
    let before = store
        .change_telemetry_binding(&Change::Begin(&intent), &|| true)
        .await
        .unwrap()
        .operation;
    let request = resolution_intent(&before, None);
    let permit = std::sync::Arc::new(permit(&request, &before, None));
    let StorageBackend::Sqlite(db) = &store.backend else {
        unreachable!()
    };
    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("UPDATE telemetry_binding_journal SET slot = slot WHERE slot = 'telemetry'")
        .execute(&mut *tx)
        .await
        .unwrap();
    let task_store = store.clone();
    let task_permit = permit.clone();
    let (started, waiting) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _ = started.send(());
        task_store
            .change_telemetry_binding(&Change::Resolve(&task_permit), &|| true)
            .await
    });
    waiting.await.unwrap();
    assert!(!task.is_finished());
    permit.expire_for_test();
    tx.commit().await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let saved = store
        .telemetry_binding_ledger(&intent.service_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.operations, vec![before]);
    assert!(saved.resolutions.is_empty());
}
