use super::*;
use crate::plugin_operation::resolution::{
    ResolutionIntent, ResolutionPermit, can_abort_before_effects,
};

fn resolution(operation: &Operation, id: &str) -> ResolutionIntent {
    ResolutionIntent::new(
        format!("resolution-{id:0>16}"),
        crate::plugin_operation::Actor::Admin {
            account: "new-confirming-operator".into(),
        },
        operation,
        chrono::Utc::now().timestamp_millis() + 120_000,
    )
    .unwrap()
}

async fn phase_abort_failure(store: &Store, enabled: bool) {
    match &store.backend {
        StorageBackend::Sqlite(db) => {
            let sql = if enabled {
                "CREATE TRIGGER resolution_abort_failure BEFORE UPDATE OF phase ON plugin_uninstall_operations \
                 WHEN NEW.phase = 'aborted' BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END"
            } else {
                "DROP TRIGGER resolution_abort_failure"
            };
            sqlx::query(sql).execute(&db.pool).await.unwrap();
        }
        StorageBackend::Postgres(db) => {
            if enabled {
                sqlx::query(
                    "CREATE FUNCTION resolution_abort_failure_fn() RETURNS trigger AS $$ \
                    BEGIN RAISE EXCEPTION 'fixture write failure'; END; $$ LANGUAGE plpgsql",
                )
                .execute(&db.pool)
                .await
                .unwrap();
                sqlx::query("CREATE TRIGGER resolution_abort_failure BEFORE UPDATE OF phase ON plugin_uninstall_operations \
                    FOR EACH ROW WHEN (NEW.phase = 'aborted') EXECUTE FUNCTION resolution_abort_failure_fn()")
                    .execute(&db.pool).await.unwrap();
            } else {
                sqlx::query("DROP TRIGGER resolution_abort_failure ON plugin_uninstall_operations")
                    .execute(&db.pool)
                    .await
                    .unwrap();
                sqlx::query("DROP FUNCTION resolution_abort_failure_fn()")
                    .execute(&db.pool)
                    .await
                    .unwrap();
            }
        }
    }
}

async fn old_migration_reader(store: &Store) {
    // The exact previous migration set opens the additive schema. As in the
    // shipped reader, ignore unknown versions, never known-byte mismatches.
    let root = tempfile::tempdir().unwrap();
    let (directory, floor) = match &store.backend {
        StorageBackend::Postgres(_) => ("migrations", 44),
        StorageBackend::Sqlite(_) => ("migrations/sqlite", 18),
    };
    for entry in std::fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_str().unwrap();
        if std::path::Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sql"))
            && name.split('_').next().unwrap().parse::<u32>().unwrap() <= floor
        {
            std::fs::copy(entry.path(), root.path().join(name)).unwrap();
        }
    }
    let mut reader = sqlx::migrate::Migrator::new(root.path()).await.unwrap();
    reader.set_ignore_missing(true);
    match &store.backend {
        StorageBackend::Postgres(db) => reader.run(&db.pool).await.unwrap(),
        StorageBackend::Sqlite(db) => reader.run(&db.pool).await.unwrap(),
    }
}

async fn session_snapshot(store: &Store) -> Vec<serde_json::Value> {
    store.load_all().await.unwrap().into_iter().map(|session| serde_json::json!({
        "meta": session.meta, "events": session.events,
        "event_count": session.event_count, "reached_start": session.reached_start,
        "next_seq": session.next_seq, "queue": session.queue, "drafts": session.drafts,
        "config_options": session.config_options, "config_preferences": session.config_preferences,
        "mobile_review_state": session.mobile_review_state,
    })).collect()
}

#[allow(clippy::too_many_lines)] // One atomicity/CAS/rollback story, identical for both backends.
async fn contract(store: &Store) {
    store.migrate().await.unwrap();
    let mut intent = fixture("local-resolution");
    intent.plugin_id = "codex".into();
    intent.session_ids = vec!["sess-709".into()];
    intent.live_session_ids = intent.session_ids.clone();
    store
        .insert_session(&crate::store::storage_contract_tests::session("sess-709"))
        .await
        .unwrap();
    store.begin_plugin_uninstall(&intent).await.unwrap();
    let before = store
        .recover_plugin_uninstalls(&intent.service_id)
        .await
        .unwrap()
        .remove(0);
    assert!(can_abort_before_effects(&before));
    assert!(
        store
            .advance_plugin_uninstall(
                &intent.operation_id,
                Phase::NeedsAttention,
                Phase::Aborted,
                None
            )
            .await
            .is_err()
    );
    let approval = resolution(&before, "atomic");
    let mut wrong = approval.clone();
    wrong.operation_digest = format!("sha256:{}", "0".repeat(64));
    assert!(
        store
            .resolve_plugin_uninstall(&ResolutionPermit::for_test(wrong))
            .await
            .is_err()
    );
    // Fail AFTER the receipt INSERT. Both the receipt and state must roll back.
    phase_abort_failure(store, true).await;
    assert!(
        store
            .resolve_plugin_uninstall(&ResolutionPermit::for_test(approval.clone()))
            .await
            .is_err()
    );
    phase_abort_failure(store, false).await;
    assert!(
        store
            .plugin_uninstall_resolution(&intent.operation_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap(),
        Some(before.clone())
    );
    let permit = ResolutionPermit::for_test(approval.clone());
    permit.expire_for_test();
    assert!(store.resolve_plugin_uninstall(&permit).await.is_err());

    let sessions_before = session_snapshot(store).await;
    let first = ResolutionPermit::for_test(approval);
    let second = ResolutionPermit::for_test(resolution(&before, "racing"));
    let (a, b) = tokio::join!(
        store.resolve_plugin_uninstall(&first),
        store.resolve_plugin_uninstall(&second)
    );
    assert_ne!(
        a.is_ok(),
        b.is_ok(),
        "exactly one new confirmation can commit"
    );
    let receipt = a.or(b).unwrap();
    let completed = store
        .plugin_uninstall_operation(&intent.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert!(receipt.matches_completed(&completed).unwrap());
    assert_eq!(completed.intent, before.intent);
    assert_eq!(completed.problem, before.problem);
    assert_eq!(completed.attention_from, before.attention_from);
    assert_eq!(completed.cause, before.cause);
    assert_ne!(receipt.intent.actor, completed.intent.actor);
    assert_eq!(session_snapshot(store).await, sessions_before);
    assert_eq!(
        store
            .plugin_uninstall_resolution(&intent.operation_id)
            .await
            .unwrap(),
        Some(receipt.clone())
    );
    assert!(
        store
            .resolve_plugin_uninstall(&ResolutionPermit::for_test(receipt.intent.clone()))
            .await
            .is_err(),
        "repeated mutation is refused; callers may separately observe the saved receipt"
    );
    assert!(
        store
            .advance_plugin_uninstall(
                &intent.operation_id,
                Phase::Prepared,
                Phase::StoppingSessions,
                None
            )
            .await
            .is_err()
    );
    old_migration_reader(store).await;
    assert!(
        store
            .recover_plugin_uninstalls(&intent.service_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .plugin_uninstall_resolution(&intent.operation_id)
            .await
            .unwrap(),
        Some(receipt)
    );

    let mut later = fixture("after-resolution");
    later.plugin_id = intent.plugin_id;
    store.begin_plugin_uninstall(&later).await.unwrap();
    store
        .advance_plugin_uninstall(
            &later.operation_id,
            Phase::Prepared,
            Phase::StoppingSessions,
            None,
        )
        .await
        .unwrap();
    let pending = store
        .recover_plugin_uninstalls(&later.service_id)
        .await
        .unwrap()
        .remove(0);
    assert!(!can_abort_before_effects(&pending));
    let mut fake = pending.clone();
    fake.attention_from = Some(Phase::Prepared);
    let forged = resolution(&fake, "later-effects");
    assert!(
        store
            .resolve_plugin_uninstall(&ResolutionPermit::for_test(forged))
            .await
            .is_err()
    );
    assert_eq!(
        store
            .plugin_uninstall_operation(&later.operation_id)
            .await
            .unwrap(),
        Some(pending)
    );
}

#[tokio::test]
async fn sqlite_no_effect_resolution_contract() {
    let root = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}",
        root.path().join("resolution.sqlite").display()
    );
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    contract(&store).await;
    let operation_id = fixture("local-resolution").operation_id;
    let receipt = store
        .plugin_uninstall_resolution(&operation_id)
        .await
        .unwrap()
        .unwrap();
    let StorageBackend::Sqlite(db) = &store.backend else {
        unreachable!()
    };
    db.pool.close().await;
    let reopened = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    reopened.migrate().await.unwrap();
    assert_eq!(
        reopened
            .plugin_uninstall_resolution(&operation_id)
            .await
            .unwrap(),
        Some(receipt)
    );
    let StorageBackend::Sqlite(db) = &reopened.backend else {
        unreachable!()
    };
    sqlx::query("UPDATE plugin_uninstall_resolutions SET intent = '{}' WHERE operation_id = ?1")
        .bind(&operation_id)
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        reopened
            .plugin_uninstall_resolution(&operation_id)
            .await
            .is_err()
    );
    assert_eq!(
        reopened
            .plugin_uninstall_operation(&operation_id)
            .await
            .unwrap()
            .unwrap()
            .phase,
        Phase::Aborted
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture: just test-postgres"]
async fn postgres_no_effect_resolution_contract() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated PostgreSQL fixture");
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    contract(&store).await;
}

#[tokio::test]
async fn sqlite_resolution_rechecks_budget_after_waiting_for_the_write_lock() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let intent = fixture("queued-resolution");
    store.begin_plugin_uninstall(&intent).await.unwrap();
    let before = store
        .recover_plugin_uninstalls(&intent.service_id)
        .await
        .unwrap()
        .remove(0);
    let permit = std::sync::Arc::new(ResolutionPermit::for_test(resolution(&before, "queued")));
    let StorageBackend::Sqlite(db) = &store.backend else {
        unreachable!()
    };
    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("UPDATE plugin_uninstall_operations SET phase = phase WHERE operation_id = ?1")
        .bind(&intent.operation_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    let task_store = store.clone();
    let task_permit = std::sync::Arc::clone(&permit);
    let (started, waiting) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _ = started.send(());
        task_store.resolve_plugin_uninstall(&task_permit).await
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
    assert_eq!(
        store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap(),
        Some(before)
    );
    assert!(
        store
            .plugin_uninstall_resolution(&intent.operation_id)
            .await
            .unwrap()
            .is_none()
    );
}
