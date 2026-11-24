use super::{QueuedMessage, Store, session, test_support};
use crate::prompt_origin::{DeliveryOrigin, PromptDispatchRecord};

#[allow(clippy::too_many_lines)]
async fn assert_prompt_dispatches_contract(store: &Store) {
    let active = "pd-active";
    let other = "pd-other";
    let deleted = "pd-deleted";
    for id in [active, other, deleted] {
        store.insert_session(&session(id)).await.unwrap();
    }

    let records = [
        PromptDispatchRecord {
            session_id: active.into(),
            cmid: "cmid-owner".into(),
            origin: DeliveryOrigin::Owner,
            trigger: DeliveryOrigin::Owner,
            dispatched_at_ms: 100,
            undelivered_retired: false,
        },
        PromptDispatchRecord {
            session_id: active.into(),
            cmid: "cmid-auto".into(),
            origin: DeliveryOrigin::Automation,
            trigger: DeliveryOrigin::Automation,
            dispatched_at_ms: 200,
            undelivered_retired: false,
        },
        PromptDispatchRecord {
            session_id: active.into(),
            cmid: "cmid-peer".into(),
            origin: DeliveryOrigin::peer("parent-9"),
            trigger: DeliveryOrigin::peer("parent-9"),
            dispatched_at_ms: 300,
            undelivered_retired: false,
        },
        PromptDispatchRecord {
            session_id: active.into(),
            cmid: "cmid-cowboy".into(),
            origin: DeliveryOrigin::cowboy("schedule"),
            trigger: DeliveryOrigin::cowboy("schedule"),
            dispatched_at_ms: 400,
            undelivered_retired: false,
        },
        PromptDispatchRecord {
            session_id: other.into(),
            cmid: "cmid-owner".into(),
            origin: DeliveryOrigin::peer("other-agent"),
            trigger: DeliveryOrigin::peer("other-agent"),
            dispatched_at_ms: 250,
            undelivered_retired: false,
        },
        PromptDispatchRecord {
            session_id: deleted.into(),
            cmid: "cmid-del".into(),
            origin: DeliveryOrigin::Owner,
            trigger: DeliveryOrigin::Owner,
            dispatched_at_ms: 500,
            undelivered_retired: false,
        },
    ];
    for record in &records {
        assert!(
            store
                .record_prompt_dispatch_if_compatible(record)
                .await
                .unwrap()
        );
    }

    for record in &records {
        let fetched = store
            .prompt_dispatch(&record.session_id, &record.cmid)
            .await
            .unwrap()
            .expect("persisted prompt dispatch");
        assert_eq!(&fetched, record);
    }
    assert_eq!(
        store.prompt_dispatch(active, "missing").await.unwrap(),
        None
    );

    // Immutability & compatibility: identical retry returns Ok(true);
    // accelerating a non-owner record with a different trigger keeps the same
    // origin and returns Ok(true); a conflicting origin returns Ok(false).
    assert!(
        store
            .record_prompt_dispatch_if_compatible(&records[0])
            .await
            .unwrap()
    );
    let accelerated_notice = PromptDispatchRecord {
        session_id: active.into(),
        cmid: "cmid-cowboy".into(),
        origin: DeliveryOrigin::cowboy("schedule"),
        trigger: DeliveryOrigin::Owner,
        dispatched_at_ms: 450,
        undelivered_retired: false,
    };
    assert!(
        store
            .record_prompt_dispatch_if_compatible(&accelerated_notice)
            .await
            .unwrap()
    );
    let refreshed = store
        .prompt_dispatch(active, "cmid-cowboy")
        .await
        .unwrap()
        .expect("refreshed cowboy dispatch");
    assert_eq!(refreshed.origin, DeliveryOrigin::cowboy("schedule"));
    assert_eq!(refreshed.trigger, DeliveryOrigin::cowboy("schedule"));
    assert_eq!(refreshed.dispatched_at_ms, 450);
    let forged = PromptDispatchRecord {
        session_id: active.into(),
        cmid: "cmid-owner".into(),
        origin: DeliveryOrigin::peer("attacker"),
        trigger: DeliveryOrigin::peer("attacker"),
        dispatched_at_ms: 999,
        undelivered_retired: false,
    };
    assert!(
        !store
            .record_prompt_dispatch_if_compatible(&forged)
            .await
            .unwrap()
    );
    store.record_prompt_dispatch(&forged).await.unwrap();
    assert_eq!(
        store.prompt_dispatch(active, "cmid-owner").await.unwrap(),
        Some(records[0].clone())
    );

    let batch = store
        .prompt_dispatches_by_cmids(
            active,
            &[
                "cmid-owner".into(),
                "cmid-peer".into(),
                "cmid-del".into(),
                "missing".into(),
            ],
        )
        .await
        .unwrap();
    assert_eq!(batch.len(), 2);
    assert_eq!(batch.get("cmid-owner"), Some(&DeliveryOrigin::Owner));
    assert_eq!(
        batch.get("cmid-peer"),
        Some(&DeliveryOrigin::peer("parent-9"))
    );
    assert!(
        store
            .prompt_dispatches_by_cmids(active, &[])
            .await
            .unwrap()
            .is_empty()
    );

    assert_prompt_dispatches_hydration_and_queue_contract(store, active, other, deleted).await;
}

#[allow(clippy::too_many_lines)]
async fn assert_prompt_dispatches_hydration_and_queue_contract(
    store: &Store,
    active: &str,
    other: &str,
    deleted: &str,
) {
    // Soft-deleted sessions excluded; per-session limit retains newest ordered ASC.
    store.delete_session(deleted).await.unwrap();
    let recent = store.recent_prompt_dispatches(2).await.unwrap();
    let active_recent: Vec<&str> = recent
        .iter()
        .filter(|r| r.session_id == active)
        .map(|r| r.cmid.as_str())
        .collect();
    assert_eq!(active_recent, vec!["cmid-peer", "cmid-cowboy"]);
    assert!(recent.iter().any(|r| r.session_id == other));
    assert!(!recent.iter().any(|r| r.session_id == deleted));

    // QueuedMessage persists origin + initiator; legacy rows decode as None/None.
    let legacy_row: QueuedMessage = serde_json::from_value(serde_json::json!({
        "id": "q-legacy", "text": "old queued row", "content": [], "cmid": "legacy-cmid"
    }))
    .unwrap();
    assert_eq!(legacy_row.origin, None);
    assert_eq!(legacy_row.initiator, None);

    let typed_row = QueuedMessage {
        id: "q-typed".into(),
        text: "owner draft activated by agent".into(),
        content: vec![],
        cmid: Some("cmid-q".into()),
        schedule: None,
        origin: Some(DeliveryOrigin::Owner),
        initiator: Some(DeliveryOrigin::peer("agent-1")),
    };
    store
        .update_pending(active, &[legacy_row, typed_row], &[])
        .await
        .unwrap();
    let loaded = store
        .load_all()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.meta.id == active)
        .unwrap();
    assert_eq!(loaded.queue.len(), 2);
    assert_eq!(loaded.queue[0].origin, None);
    assert_eq!(loaded.queue[0].initiator, None);
    assert_eq!(loaded.queue[1].origin, Some(DeliveryOrigin::Owner));
    assert_eq!(
        loaded.queue[1].initiator,
        Some(DeliveryOrigin::peer("agent-1"))
    );

    // Write-behind race: recording dispatch provenance before InsertSession
    // flushes must succeed, survive purge_deleted of unrelated expired sessions,
    // and hydrate once InsertSession lands.
    let unflushed = "pd-unflushed";
    let now_ms = chrono::Utc::now().timestamp_millis();
    let early = PromptDispatchRecord {
        session_id: unflushed.into(),
        cmid: "initial:pd-unflushed".into(),
        origin: DeliveryOrigin::peer("creator-1"),
        trigger: DeliveryOrigin::peer("creator-1"),
        dispatched_at_ms: now_ms,
        undelivered_retired: false,
    };
    let stale_orphan = PromptDispatchRecord {
        session_id: "pd-never-flushed-old".into(),
        cmid: "initial:old-orphan".into(),
        origin: DeliveryOrigin::Owner,
        trigger: DeliveryOrigin::Owner,
        dispatched_at_ms: now_ms - crate::store::prompt_dispatches::ORPHAN_GRACE_MS - 60_000,
        undelivered_retired: false,
    };
    assert!(
        store
            .record_prompt_dispatch_if_compatible(&early)
            .await
            .unwrap()
    );
    assert!(
        store
            .record_prompt_dispatch_if_compatible(&stale_orphan)
            .await
            .unwrap()
    );
    // Purging drops expired soft-deleted sessions and aged orphans while
    // preserving fresh not-yet-flushed sessions inside the grace window.
    let _ = store.purge_deleted(0).await.unwrap();
    assert_eq!(
        store.prompt_dispatch(deleted, "cmid-del").await.unwrap(),
        None
    );
    assert_eq!(
        store
            .prompt_dispatch("pd-never-flushed-old", "initial:old-orphan")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .prompt_dispatch(unflushed, "initial:pd-unflushed")
            .await
            .unwrap(),
        Some(early.clone())
    );
    store.insert_session(&session(unflushed)).await.unwrap();
    let hydrated = store.recent_prompt_dispatches(10).await.unwrap();
    assert!(hydrated.iter().any(|r| r == &early));

    // Atomic UpdatePending retires exact prompt_dispatches row alongside draft persistence,
    // and operation_id_used_in_history detects durable dispatch/event IDs across restarts.
    assert!(
        store
            .operation_id_used_in_history(unflushed, "initial:pd-unflushed")
            .await
            .unwrap()
    );
    assert!(
        !store
            .operation_id_used_in_history(unflushed, "unused-op-id")
            .await
            .unwrap()
    );
    let recovery_draft = crate::model::QueuedMessage {
        id: "recovery-q1".into(),
        text: "parked".into(),
        content: vec![],
        cmid: None,
        schedule: None,
        origin: None,
        initiator: None,
    };
    store
        .update_pending_with_notice_ops(
            unflushed,
            &[],
            &[recovery_draft],
            &[],
            &[],
            &["initial:pd-unflushed".to_owned()],
        )
        .await
        .unwrap();
    let retired = store
        .prompt_dispatch(unflushed, "initial:pd-unflushed")
        .await
        .unwrap()
        .unwrap();
    assert!(retired.undelivered_retired);
    let hydrated_after = store.recent_prompt_dispatches(10).await.unwrap();
    assert!(hydrated_after.iter().any(|r| r.session_id == unflushed
        && r.cmid == "initial:pd-unflushed"
        && r.undelivered_retired));
}

#[tokio::test]
async fn sqlite_prompt_dispatches_contract() {
    assert_prompt_dispatches_contract(&test_support::sqlite().await.store).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_prompt_dispatches_contract() {
    assert_prompt_dispatches_contract(&test_support::postgres().await.store).await;
}
