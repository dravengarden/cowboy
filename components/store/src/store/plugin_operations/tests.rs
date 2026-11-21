use super::*;
use crate::plugin_operation::fixture;

mod resolution;

async fn advance_to_machine_commit(store: &Store, intent: &UninstallIntent) {
    for (from, to) in [
        (Phase::Prepared, Phase::StoppingSessions),
        (Phase::StoppingSessions, Phase::Uninstalling),
        (Phase::Uninstalling, Phase::MachineUninstalled),
    ] {
        store
            .advance_plugin_uninstall(&intent.operation_id, from, to, None)
            .await
            .unwrap();
    }
}

#[allow(clippy::too_many_lines)] // One atomicity, compensation-CAS and retained-reference story on both backends.
async fn contract(store: &Store) {
    store.migrate().await.unwrap();
    // Reader bridge accepts both retained schema-one and exact-incarnation
    // schema-two evidence on PostgreSQL and SQLite. No migration rewrite.
    let mut cas = fixture("installation-cas");
    cas.schema = 2;
    cas.installation_revision = Some(
        format!("installation-{}", "a".repeat(64))
            .try_into()
            .unwrap(),
    );
    store.begin_plugin_uninstall(&cas).await.unwrap();
    assert_eq!(
        store
            .plugin_uninstall_operation(&cas.operation_id)
            .await
            .unwrap()
            .unwrap()
            .intent,
        cas
    );
    let mut changed = cas.clone();
    changed.installation_revision = Some(
        format!("installation-{}", "b".repeat(64))
            .try_into()
            .unwrap(),
    );
    assert!(store.begin_plugin_uninstall(&changed).await.is_err());
    store
        .advance_plugin_uninstall(&cas.operation_id, Phase::Prepared, Phase::Aborted, None)
        .await
        .unwrap();
    let mut intent = fixture("atomic");
    intent.plugin_id = "codex".to_owned();
    intent.session_ids = vec!["sess-701".to_owned(), "sess-702".to_owned()];
    store
        .insert_session(&super::super::storage_contract_tests::session("sess-701"))
        .await
        .unwrap();
    store.begin_plugin_uninstall(&intent).await.unwrap();
    assert!(
        store.begin_plugin_uninstall(&intent).await.is_err(),
        "a repeated ID is evidence, not execution authority"
    );
    let mut conflict = intent.clone();
    conflict.actor = crate::plugin_operation::Actor::Admin {
        account: "another".to_owned(),
    };
    assert!(store.begin_plugin_uninstall(&conflict).await.is_err());
    conflict.operation_id = fixture("different").operation_id;
    assert!(
        store.begin_plugin_uninstall(&conflict).await.is_err(),
        "the open slot is unique"
    );
    assert!(store.commit_plugin_uninstall(&intent).await.is_err());
    advance_to_machine_commit(store, &intent).await;
    assert!(
        store.commit_plugin_uninstall(&intent).await.is_err(),
        "a missing session aborts the entire transaction"
    );
    assert_eq!(
        store.load_all().await.unwrap().len(),
        1,
        "an earlier UPDATE was rolled back"
    );
    assert_eq!(
        store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap()
            .phase,
        Phase::MachineUninstalled
    );
    store
        .insert_session(&super::super::storage_contract_tests::session("sess-702"))
        .await
        .unwrap();
    let mut expanded = intent.clone();
    expanded.session_ids.pop();
    assert!(
        store.commit_plugin_uninstall(&expanded).await.is_err(),
        "intent digest prevents changing the deletion set"
    );
    store.commit_plugin_uninstall(&intent).await.unwrap();
    assert!(store.load_all().await.unwrap().is_empty());
    assert_eq!(
        store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap()
            .phase,
        Phase::Completed
    );
    assert!(
        store
            .advance_plugin_uninstall(
                &intent.operation_id,
                Phase::MachineUninstalled,
                Phase::RestoringMachine,
                None
            )
            .await
            .is_err()
    );
    assert!(
        store
            .recover_plugin_uninstalls("service-test")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .plugin_uninstall_history("service-test", "hawk", "codex")
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .plugin_uninstall_history("other-service", "hawk", "codex")
            .await
            .unwrap()
            .is_empty()
    );
    store.begin_plugin_uninstall(&conflict).await.unwrap();
    // Retention must not erase the before-state references of an unfinished
    // operation, even after their original uninstall purge deadline.
    match &store.backend {
        StorageBackend::Postgres(db) => {
            sqlx::query(
                "UPDATE sessions SET purge_after_at = to_timestamp(0) WHERE deleted_at IS NOT NULL",
            )
            .execute(&db.pool)
            .await
            .unwrap();
        }
        StorageBackend::Sqlite(db) => {
            sqlx::query(
                "UPDATE sessions SET purge_after_at_ms = 0 WHERE deleted_at_ms IS NOT NULL",
            )
            .execute(&db.pool)
            .await
            .unwrap();
        }
    }
    assert_eq!(store.purge_deleted(3).await.unwrap(), 0);
    let recovered = store
        .recover_plugin_uninstalls("service-test")
        .await
        .unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].phase, Phase::NeedsAttention);
    assert_eq!(recovered[0].attention_from, Some(Phase::Prepared));
    assert!(store.begin_plugin_uninstall(&conflict).await.is_err());
    assert!(
        store
            .recover_plugin_uninstalls("another-service")
            .await
            .is_err()
    );
    let again = store
        .recover_plugin_uninstalls("service-test")
        .await
        .unwrap();
    assert_eq!(again[0].attention_from, recovered[0].attention_from);
    assert_eq!(again[0].updated_at_ms, recovered[0].updated_at_ms);
    assert_eq!(
        store.purge_deleted(3).await.unwrap(),
        0,
        "recovery fencing also retains expired rows"
    );
}

#[tokio::test]
async fn sqlite_recovery_waits_for_writer_before_reading_journal() {
    contended_sqlite_recovery(true).await;
}

#[tokio::test]
async fn sqlite_empty_recovery_waits_for_writer() {
    contended_sqlite_recovery(false).await;
}

async fn contended_sqlite_recovery(pending: bool) {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("journal.sqlite").display());
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let intent = fixture("contended-recovery");
    if pending {
        store.begin_plugin_uninstall(&intent).await.unwrap();
    }
    let StorageBackend::Sqlite(db) = &store.backend else {
        unreachable!()
    };
    let mut writer = db.pool.begin().await.unwrap();
    sqlx::query("UPDATE plugin_uninstall_operations SET phase = 'stopping_sessions'")
        .execute(&mut *writer)
        .await
        .unwrap();

    // A deferred reader can see Prepared while this writer is uncommitted,
    // but cannot upgrade that snapshot to a writer, even with busy_timeout.
    let recovery = store.recover_plugin_uninstalls("service-test");
    tokio::pin!(recovery);
    let early = tokio::time::timeout(std::time::Duration::from_millis(100), &mut recovery).await;
    writer.commit().await.unwrap();
    assert!(
        early.is_err(),
        "recovery must wait for the writer: {early:?}"
    );
    let operations = recovery.await.unwrap();
    if !pending {
        assert!(operations.is_empty());
        return;
    }
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].phase, Phase::NeedsAttention);
    assert_eq!(operations[0].attention_from, Some(Phase::StoppingSessions));
    assert_eq!(operations[0].problem, Some(Problem::Interrupted));
    let again = store
        .recover_plugin_uninstalls("service-test")
        .await
        .unwrap();
    assert_eq!(again[0].attention_from, operations[0].attention_from);
    assert_eq!(again[0].updated_at_ms, operations[0].updated_at_ms);
}

#[tokio::test]
async fn sqlite_plugin_uninstall_transaction_contract() {
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
    assert_eq!(
        synchronous, 2,
        "intent must survive power loss before external effects"
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture: just test-postgres"]
async fn postgres_plugin_uninstall_transaction_contract() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated PostgreSQL fixture");
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    contract(&store).await;
}

#[tokio::test]
async fn sqlite_reopen_preserves_every_interrupted_phase_without_replaying_it() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("journal.sqlite").display());
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let phases = [
        Phase::Prepared,
        Phase::StoppingSessions,
        Phase::Uninstalling,
        Phase::MachineUninstalled,
        Phase::RestoringMachine,
        Phase::RestoringSessions,
    ];
    for (index, phase) in phases.into_iter().enumerate() {
        let mut intent = fixture(&index.to_string());
        intent.plugin_id = format!("plugin-{index}");
        store.begin_plugin_uninstall(&intent).await.unwrap();
        let path = [
            Phase::Prepared,
            Phase::StoppingSessions,
            Phase::Uninstalling,
            Phase::MachineUninstalled,
            Phase::RestoringMachine,
            Phase::RestoringSessions,
        ];
        for pair in path.windows(2).take(index) {
            store
                .advance_plugin_uninstall(&intent.operation_id, pair[0], pair[1], None)
                .await
                .unwrap();
        }
        assert_eq!(
            store
                .plugin_uninstall_operation(&intent.operation_id)
                .await
                .unwrap()
                .unwrap()
                .phase,
            phase
        );
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
        .recover_plugin_uninstalls("service-test")
        .await
        .unwrap();
    assert_eq!(recovered.len(), phases.len());
    for op in recovered {
        assert_eq!(op.phase, Phase::NeedsAttention);
        assert_eq!(op.problem, Some(Problem::Interrupted));
        assert!(phases.contains(&op.attention_from.unwrap()));
        assert!(
            reopened
                .advance_plugin_uninstall(
                    &op.intent.operation_id,
                    Phase::NeedsAttention,
                    Phase::Uninstalling,
                    None
                )
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn tampered_or_unknown_evidence_fails_closed_before_recovery() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let intent = fixture("tamper");
    store.begin_plugin_uninstall(&intent).await.unwrap();
    let StorageBackend::Sqlite(db) = &store.backend else {
        unreachable!()
    };
    sqlx::query("UPDATE plugin_uninstall_operations SET intent = '{}' WHERE operation_id = ?1")
        .bind(&intent.operation_id)
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        store
            .recover_plugin_uninstalls("service-test")
            .await
            .is_err()
    );
    let phase: String = sqlx::query_scalar("SELECT phase FROM plugin_uninstall_operations")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        phase, "prepared",
        "failed recovery must not discard original evidence"
    );
}

#[tokio::test]
async fn concurrent_slot_claims_accept_one_intent() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let first = fixture("first");
    let second = fixture("second");
    let (a, b) = tokio::join!(
        store.begin_plugin_uninstall(&first),
        store.begin_plugin_uninstall(&second)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(
        store
            .recover_plugin_uninstalls("service-test")
            .await
            .unwrap()
            .len(),
        1
    );
}
