use super::*;
use crate::plugin_operation::installation::fixture;

async fn advance(store: &Store, intent: &InstallIntent, target: InstallPhase) {
    let path = [
        InstallPhase::Prepared,
        InstallPhase::SyncingAuthentication,
        InstallPhase::Installing,
        InstallPhase::MachineAcknowledged,
        InstallPhase::Completed,
    ];
    for pair in path.windows(2) {
        if pair[0] == target {
            break;
        }
        store
            .advance_plugin_install(intent, pair[0], pair[1], None)
            .await
            .unwrap();
        if pair[1] == target {
            break;
        }
    }
}

async fn contract(store: &Store) {
    store.migrate().await.unwrap();
    let intent = fixture("contract");
    store.begin_plugin_install(&intent).await.unwrap();
    assert_eq!(
        store
            .plugin_install_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap()
            .intent,
        intent
    );
    assert!(
        store.begin_plugin_install(&intent).await.is_err(),
        "same ID is evidence, not authority"
    );
    let mut changed = intent.clone();
    changed.envelope_digest = format!("sha256:{}", "d".repeat(64));
    assert!(store.begin_plugin_install(&changed).await.is_err());
    assert!(
        store
            .advance_plugin_install(
                &changed,
                InstallPhase::Prepared,
                InstallPhase::Installing,
                None
            )
            .await
            .is_err()
    );
    assert!(
        store
            .begin_plugin_install(&fixture("another-id"))
            .await
            .is_err(),
        "open slot is unique"
    );
    let mut uninstall = crate::plugin_operation::fixture("cross-kind");
    uninstall.machine_id.clone_from(&intent.machine_id);
    assert!(
        store.begin_plugin_uninstall(&uninstall).await.is_err(),
        "uninstall cannot cross an install claim"
    );
    advance(store, &intent, InstallPhase::Completed).await;
    assert!(
        store
            .advance_plugin_install(
                &intent,
                InstallPhase::Completed,
                InstallPhase::Installing,
                None
            )
            .await
            .is_err()
    );
    store.begin_plugin_uninstall(&uninstall).await.unwrap();
    assert!(
        store
            .begin_plugin_install(&fixture("after-uninstall"))
            .await
            .is_err()
    );
    store
        .advance_plugin_uninstall(
            &uninstall.operation_id,
            crate::plugin_operation::Phase::Prepared,
            crate::plugin_operation::Phase::Aborted,
            None,
        )
        .await
        .unwrap();
    let next = fixture("after-uninstall");
    store.begin_plugin_install(&next).await.unwrap();
    recovery_contract(store, next).await;
}

async fn recovery_contract(store: &Store, next: InstallIntent) {
    assert!(
        store
            .advance_plugin_install(&next, InstallPhase::Prepared, InstallPhase::Completed, None)
            .await
            .is_err()
    );
    store
        .advance_plugin_install(
            &next,
            InstallPhase::Prepared,
            InstallPhase::Installing,
            None,
        )
        .await
        .unwrap();
    assert!(
        store
            .advance_plugin_install(
                &next,
                InstallPhase::Installing,
                InstallPhase::Aborted,
                Some(InstallProblem::MachineRejected)
            )
            .await
            .is_err(),
        "generic rejection cannot prove no effect"
    );
    let recovered = store.recover_plugin_installs("service-test").await.unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].phase, InstallPhase::NeedsAttention);
    assert_eq!(recovered[0].attention_from, Some(InstallPhase::Installing));
    assert_eq!(recovered[0].problem, Some(InstallProblem::Interrupted));
    assert_eq!(
        store.recover_plugin_installs("service-test").await.unwrap(),
        recovered
    );
    assert!(
        store
            .recover_plugin_installs("other-service")
            .await
            .is_err()
    );
    assert!(
        store
            .advance_plugin_install(
                &next,
                InstallPhase::NeedsAttention,
                InstallPhase::Installing,
                None
            )
            .await
            .is_err()
    );
    assert!(
        store
            .begin_plugin_install(&fixture("fresh-id"))
            .await
            .is_err()
    );
    assert_eq!(
        store
            .plugin_install_history("service-test", "machine-test", "victoria")
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(
        store
            .plugin_install_history("other-service", "machine-test", "victoria")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn sqlite_install_journal_contract() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    contract(&store).await;
    let StorageBackend::Sqlite(db) = &store.backend else {
        unreachable!()
    };
    let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(synchronous, 2);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture: just test-postgres"]
async fn postgres_install_journal_contract() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated PostgreSQL fixture");
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    contract(&store).await;
}

#[tokio::test]
async fn reopen_reconstructs_every_interrupted_phase_without_replaying_or_claiming_success() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("install.sqlite").display());
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let phases = [
        InstallPhase::Prepared,
        InstallPhase::SyncingAuthentication,
        InstallPhase::Installing,
        InstallPhase::MachineAcknowledged,
        InstallPhase::Completed,
    ];
    for (index, phase) in phases.iter().enumerate() {
        let mut intent = fixture(&index.to_string());
        intent.plugin_id = format!("plugin-{index}");
        store.begin_plugin_install(&intent).await.unwrap();
        advance(&store, &intent, *phase).await;
    }
    let StorageBackend::Sqlite(db) = &store.backend else {
        unreachable!()
    };
    db.pool.close().await;
    drop(store);
    let reopened = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    reopened.migrate().await.unwrap();
    let recovered = reopened
        .recover_plugin_installs("service-test")
        .await
        .unwrap();
    assert_eq!(recovered.len(), 4);
    for op in &recovered {
        assert_eq!(op.phase, InstallPhase::NeedsAttention);
        assert!(phases[..4].contains(&op.attention_from.unwrap()));
    }
    assert_eq!(
        reopened
            .recover_plugin_installs("service-test")
            .await
            .unwrap(),
        recovered
    );
}

#[tokio::test]
async fn contended_recovery_reserves_sqlite_writer_before_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect(
        &format!("sqlite://{}", root.path().join("install.sqlite").display()),
        root.path().join("artifacts"),
    )
    .await
    .unwrap();
    store.migrate().await.unwrap();
    store
        .begin_plugin_install(&fixture("contention"))
        .await
        .unwrap();
    let StorageBackend::Sqlite(db) = &store.backend else {
        unreachable!()
    };
    let mut writer = db.pool.begin().await.unwrap();
    sqlx::query("UPDATE plugin_install_operations SET phase = 'installing'")
        .execute(&mut *writer)
        .await
        .unwrap();
    let pending = store.recover_plugin_installs("service-test");
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut pending)
            .await
            .is_err()
    );
    writer.commit().await.unwrap();
    assert_eq!(
        pending.await.unwrap()[0].attention_from,
        Some(InstallPhase::Installing)
    );
}

#[tokio::test]
async fn unknown_schema_or_corrupt_evidence_aborts_recovery_transaction() {
    for corruption in ["digest", "schema", "owner", "phase"] {
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let mut intent = fixture("corrupt");
        store.begin_plugin_install(&intent).await.unwrap();
        let StorageBackend::Sqlite(db) = &store.backend else {
            unreachable!()
        };
        let statement = match corruption {
            "digest" => "UPDATE plugin_install_operations SET intent_sha256 = 'invalid'",
            "owner" => "UPDATE plugin_install_operations SET service_id = 'foreign'",
            "phase" => "UPDATE plugin_install_operations SET phase = 'needs_attention'",
            _ => {
                intent.schema = 99;
                let document = serde_json::to_string(&intent).unwrap();
                sqlx::query("UPDATE plugin_install_operations SET intent = $1, intent_sha256 = $2")
                    .bind(&document)
                    .bind(format!("{:x}", Sha256::digest(document.as_bytes())))
                    .execute(&db.pool)
                    .await
                    .unwrap();
                "SELECT 1"
            }
        };
        sqlx::query(statement).execute(&db.pool).await.unwrap();
        assert!(
            store.recover_plugin_installs("service-test").await.is_err(),
            "{corruption}"
        );
        let problem: Option<String> =
            sqlx::query_scalar("SELECT problem FROM plugin_install_operations")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert!(
            problem.is_none(),
            "invalid evidence must not be rewritten as an interruption"
        );
    }
}
