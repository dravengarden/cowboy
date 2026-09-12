use super::*;
use crate::telemetry_binding::{LegacyFence, Progress, fixture, tests::applied};
use std::sync::atomic::{AtomicUsize, Ordering};

#[allow(clippy::too_many_lines)] // One atomic head/evidence/CAS story exercised on both real backends.
async fn contract(store: &Store) {
    store.migrate().await.unwrap();
    let intent = fixture("storage");
    let fence = LegacyFence::recover(Some(store), &intent.service_id)
        .await
        .unwrap();
    assert!(fence.allows_legacy());
    assert!(
        store
            .telemetry_binding_ledger(&intent.service_id)
            .await
            .unwrap()
            .is_none()
    );
    // Failure at either admission checkpoint rolls back even the first row.
    for expire_at in 0..2 {
        let checks = AtomicUsize::new(0);
        assert!(
            store
                .change_telemetry_binding(&Change::Begin(&intent), &|| checks
                    .fetch_add(1, Ordering::Relaxed)
                    < expire_at)
                .await
                .is_err()
        );
        assert!(
            store
                .telemetry_binding_ledger(&intent.service_id)
                .await
                .unwrap()
                .is_none()
        );
    }
    let prepared = store
        .change_telemetry_binding(&Change::Begin(&intent), &|| true)
        .await
        .unwrap()
        .operation;
    assert!(
        !LegacyFence::recover(Some(store), &intent.service_id)
            .await
            .unwrap()
            .allows_legacy()
    );
    assert!(
        !store
            .change_telemetry_binding(&Change::Begin(&intent), &|| true)
            .await
            .unwrap()
            .admitted
    );
    let mut conflict = intent.clone();
    conflict.actor = crate::plugin_operation::Actor::Admin {
        account: "other".into(),
    };
    assert!(
        store
            .change_telemetry_binding(&Change::Begin(&conflict), &|| true)
            .await
            .is_err()
    );
    conflict = fixture("conflicting-operation");
    assert!(
        store
            .change_telemetry_binding(&Change::Begin(&conflict), &|| true)
            .await
            .is_err()
    );
    assert!(
        store
            .telemetry_binding_ledger("other-service")
            .await
            .is_err()
    );
    let dispatching = store
        .change_telemetry_binding(
            &Change::Advance {
                expected: &prepared,
                progress: Progress::Dispatching,
            },
            &|| true,
        )
        .await
        .unwrap()
        .operation;
    let progress = Progress::Completed {
        observation: applied(&intent),
    };
    let checks = AtomicUsize::new(0);
    assert!(
        store
            .change_telemetry_binding(
                &Change::Advance {
                    expected: &dispatching,
                    progress: progress.clone()
                },
                &|| checks.fetch_add(1, Ordering::Relaxed) == 0
            )
            .await
            .is_err()
    );
    let retained = store
        .telemetry_binding_ledger(&intent.service_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        retained.current.is_none(),
        "failed completion never commits just the head"
    );
    assert_eq!(retained.operations.last(), Some(&dispatching));
    let completed = store
        .change_telemetry_binding(
            &Change::Advance {
                expected: &dispatching,
                progress,
            },
            &|| true,
        )
        .await
        .unwrap()
        .operation;
    let retained = store
        .telemetry_binding_ledger(&intent.service_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        retained.current,
        Some(intent.machine_step().unwrap().after().unwrap())
    );
    assert_eq!(retained.operations.last(), Some(&completed));
    assert!(
        store
            .change_telemetry_binding(
                &Change::Advance {
                    expected: &dispatching,
                    progress: Progress::Aborted
                },
                &|| true
            )
            .await
            .is_err()
    );
    let repeated = store
        .change_telemetry_binding(&Change::Begin(&intent), &|| true)
        .await
        .unwrap();
    assert!(!repeated.admitted);
    assert_eq!(repeated.operation, completed);

    let mut revoke = fixture("revoke-race");
    revoke.expected = retained.current;
    revoke.change = crate::machine_protocol::telemetry_binding::BindingChange::Revoke {
        policy_epoch: revoke
            .expected
            .as_ref()
            .unwrap()
            .policy_epoch
            .next()
            .unwrap(),
    };
    let mut competing = revoke.clone();
    competing.operation_id.push_str("-other");
    let left = Change::Begin(&revoke);
    let right = Change::Begin(&competing);
    let (a, b) = tokio::join!(
        store.change_telemetry_binding(&left, &|| true),
        store.change_telemetry_binding(&right, &|| true)
    );
    assert_ne!(a.is_ok(), b.is_ok(), "only one exact slot CAS may win");
    let retained = store
        .telemetry_binding_ledger(&intent.service_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.operations.len(), 2);
    assert_eq!(retained.operations[0], completed);
}

#[tokio::test]
async fn sqlite_binding_journal_contract() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    contract(&store).await;
}

#[tokio::test]
#[ignore = "requires the owned isolated PostgreSQL fixture"]
async fn postgres_binding_journal_contract() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated PostgreSQL fixture");
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    contract(&store).await;
}

#[tokio::test]
async fn sqlite_restart_retains_prepared_fence_and_corruption_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("bindings.sqlite").display());
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let intent = fixture("restart");
    let prepared = store
        .change_telemetry_binding(&Change::Begin(&intent), &|| true)
        .await
        .unwrap()
        .operation;
    drop(store);
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    assert!(
        !LegacyFence::recover(Some(&store), &intent.service_id)
            .await
            .unwrap()
            .allows_legacy()
    );
    let retained = store
        .telemetry_binding_ledger(&intent.service_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.operations, vec![prepared]);
    assert!(
        retained.current.is_none(),
        "startup cannot adopt private legacy selection or replay Prepared"
    );
    let StorageBackend::Sqlite(db) = &store.backend else {
        unreachable!()
    };
    sqlx::query("UPDATE telemetry_binding_journal SET document = '{}' WHERE slot = 'telemetry'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        LegacyFence::recover(Some(&store), &intent.service_id)
            .await
            .is_err()
    );
    assert!(
        store
            .change_telemetry_binding(&Change::Begin(&intent), &|| true)
            .await
            .is_err(),
        "corruption is not overwritten from a cached head"
    );
}
