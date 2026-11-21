//! A queued telemetry-binding resolution against the real host writer policy,
//! which lives in this crate's `telemetry_plugin`.

use cowboy_store::store::{Store, TestPool};
use cowboy_store::telemetry_binding::resolution::tests::{intent as resolution_intent, permit};
use cowboy_store::telemetry_binding::{fixture, writer::Change};

use crate::telemetry_plugin::writer_admission::{ServiceResolution, tests::policy};

#[tokio::test]
async fn sqlite_queued_resolution_rechecks_original_host_policy_inside_transaction() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let intent = fixture("resolution-policy-lock-wait");
    let before = store
        .change_telemetry_binding(&Change::Begin(&intent), &|| true)
        .await
        .unwrap()
        .operation;
    let request = resolution_intent(&before, None);
    let permit = permit(&request, &before, None);
    let path = root.path().join("writer.json");
    let policy = policy(
        &path,
        &intent.service_id,
        &intent.machine_id,
        [false, false, true],
    );
    let scope = policy
        .scope::<ServiceResolution>(&intent.service_id, &intent.machine_id)
        .unwrap();
    let TestPool::Sqlite(pool) = store.test_pool() else {
        unreachable!()
    };
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE telemetry_binding_journal SET slot = slot WHERE slot = 'telemetry'")
        .execute(&mut *tx)
        .await
        .unwrap();
    let task_store = store.clone();
    let (started, waiting) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        assert!(scope.check());
        let _ = started.send(());
        task_store
            .change_telemetry_binding(&Change::Resolve(&permit), &|| scope.check())
            .await
    });
    waiting.await.unwrap();
    assert!(!task.is_finished());
    std::fs::remove_file(&path).unwrap();
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
