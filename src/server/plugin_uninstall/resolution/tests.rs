use super::*;

fn preview() -> ResolutionIntent {
    let operation = crate::plugin_operation::resolution::fixture();
    ResolutionIntent::new(
        "resolution-00000001".into(),
        operation.intent.actor.clone(),
        &operation,
        now_ms() + 120_000,
    )
    .unwrap()
}

#[test]
fn previews_bind_new_actor_action_target_and_are_bounded_one_use() {
    let plans = ResolutionPlans::default();
    let intent = preview();
    let target = (
        intent.machine_id.clone(),
        intent.plugin_id.clone(),
        intent.operation_id.clone(),
    );
    let request = Confirmation {
        plan_id: intent.resolution_id.clone(),
        action: intent.action,
    };
    plans.insert(intent.clone()).unwrap();
    assert!(plans.insert(intent.clone()).is_err());
    assert!(
        plans
            .consume(
                &Actor::Admin {
                    account: "not-the-confirming-operator".into()
                },
                &intent.service_id,
                &target,
                &request
            )
            .is_err()
    );
    assert!(
        plans
            .consume(&intent.actor, "other-service", &target, &request)
            .is_err()
    );
    let mut other = target.clone();
    other.2 = "operation-different".into();
    assert!(
        plans
            .consume(&intent.actor, &intent.service_id, &other, &request)
            .is_err()
    );
    assert_eq!(
        plans
            .consume(&intent.actor, &intent.service_id, &target, &request)
            .unwrap(),
        intent
    );
    assert!(
        plans
            .consume(&intent.actor, &intent.service_id, &target, &request)
            .is_err()
    );
    plans.insert(intent.clone()).unwrap();
    plans
        .plans
        .lock()
        .get(&intent.resolution_id)
        .unwrap()
        .budget
        .expire_for_test();
    assert!(
        plans
            .consume(&intent.actor, &intent.service_id, &target, &request)
            .is_err()
    );
    for i in 0..256 {
        let mut next = intent.clone();
        next.resolution_id = format!("resolution-{i:016}");
        plans.insert(next).unwrap();
    }
    let mut extra = intent;
    extra.resolution_id = "resolution-capacity-extra".into();
    assert!(plans.insert(extra).is_err());
    for path in ["resolution-plan", "resolve", "resolution"] {
        let route =
            format!("/api/machines/hawk/plugins/victoria/operations/operation-00000001/{path}");
        assert_eq!(
            classify_route(&axum::http::Method::POST, &route),
            RouteAuth::ProductOrAdminOperator
        );
        assert_eq!(
            classify_route(&axum::http::Method::GET, &route),
            RouteAuth::ProductOrAdminOperator
        );
    }
    for body in [
        r#"{"plan_id":"resolution-00000001","action":"clear_fence"}"#,
        r#"{"plan_id":"resolution-00000001","action":"abort_before_effects","force":true}"#,
        r#"{"plan_id":"resolution-00000001"}"#,
    ] {
        assert!(serde_json::from_str::<Confirmation>(body).is_err());
    }
}

#[test]
fn resolution_guard_never_steals_live_work_and_keeps_uncertainty() {
    let fences = Arc::new(parking_lot::RwLock::new(HashMap::new()));
    let key = ("hawk".into(), "victoria".into());
    assert!(OperationFence::acquire_resolution(&fences, key.clone()).is_err());
    for state in [
        PluginFenceState::Installing,
        PluginFenceState::Uninstalling,
        PluginFenceState::Uninstalled,
    ] {
        fences.write().insert(key.clone(), state);
        assert!(OperationFence::acquire_resolution(&fences, key.clone()).is_err());
        assert_eq!(fences.read().get(&key), Some(&state));
    }
    fences
        .write()
        .insert(key.clone(), PluginFenceState::NeedsReconcile);
    let fence = OperationFence::acquire_resolution(&fences, key.clone()).unwrap();
    assert!(OperationFence::acquire_resolution(&fences, key.clone()).is_err());
    drop(fence);
    assert_eq!(
        fences.read().get(&key),
        Some(&PluginFenceState::NeedsReconcile)
    );
    let mut fence = OperationFence::acquire_resolution(&fences, key.clone()).unwrap();
    fence.finish(Phase::Aborted);
    drop(fence);
    assert!(!fences.read().contains_key(&key));
}

#[tokio::test]
async fn detached_resolution_closes_only_its_slot_and_replay_is_evidence_only() {
    let (root, store, intent) = super::super::tests::setup("resolved-observer").await;
    let fences = recover_fences(Some(&store), &intent.service_id)
        .await
        .unwrap();
    let key = (intent.machine_id.clone(), intent.plugin_id.clone());
    fences.write().insert(
        ("other-machine".into(), intent.plugin_id.clone()),
        PluginFenceState::NeedsReconcile,
    );
    let before = store
        .plugin_uninstall_operation(&intent.operation_id)
        .await
        .unwrap()
        .unwrap();
    let resolution = ResolutionIntent::new(
        "resolution-detached-0001".into(),
        Actor::Admin {
            account: "fresh-operator".into(),
        },
        &before,
        now_ms() + 120_000,
    )
    .unwrap();
    let fence = OperationFence::acquire_resolution(&fences, key.clone()).unwrap();
    let permit = ResolutionPermit::for_test(resolution.clone());
    let (done, observed) = tokio::sync::oneshot::channel();
    let task_store = store.clone();
    let observer = tokio::spawn(async move {
        let result = resolve_no_effect(task_store, permit, fence).await;
        let _ = done.send(result);
    });
    drop(observer);
    let receipt = observed.await.unwrap().unwrap();
    assert_eq!(receipt.intent, resolution);
    assert!(!fences.read().contains_key(&key));
    assert_eq!(fences.read().len(), 1);
    let operation = store
        .plugin_uninstall_operation(&intent.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(operation.intent, before.intent);
    assert!(receipt.matches_completed(&operation).unwrap());
    assert!(
        store
            .recover_plugin_uninstalls(&intent.service_id)
            .await
            .unwrap()
            .is_empty()
    );
    // Model a lost COMMIT response/retained memory fence. A repeated storage
    // mutation is refused; only the exact already-committed receipt resolves it.
    fences
        .write()
        .insert(key.clone(), PluginFenceState::NeedsReconcile);
    let replay = resolve_no_effect(
        store.clone(),
        ResolutionPermit::for_test(resolution.clone()),
        OperationFence::acquire_resolution(&fences, key.clone()).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(replay, receipt);
    assert_eq!(
        store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap(),
        Some(operation)
    );
    fences
        .write()
        .insert(key.clone(), PluginFenceState::NeedsReconcile);
    let mut wrong = resolution;
    wrong.resolution_id = "resolution-different-0001".into();
    assert!(
        resolve_no_effect(
            store,
            ResolutionPermit::for_test(wrong),
            OperationFence::acquire_resolution(&fences, key.clone()).unwrap()
        )
        .await
        .is_err()
    );
    assert_eq!(
        fences.read().get(&key),
        Some(&PluginFenceState::NeedsReconcile)
    );
    let body = axum::body::to_bytes(receipt_response(&receipt).into_body(), 4096)
        .await
        .unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(!body.contains("fresh-operator"));
    assert!(!body.contains("session_ids"));
    assert!(!body.contains("operation_digest"));
    drop(root);
}

#[tokio::test]
async fn changed_or_expired_resolution_never_clears_a_fence() {
    for expired in [true, false] {
        let (_root, store, intent) = super::super::tests::setup("refused-resolution").await;
        let fences = recover_fences(Some(&store), &intent.service_id)
            .await
            .unwrap();
        let before = store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap();
        let mut resolution = ResolutionIntent::new(
            "resolution-refused-0001".into(),
            intent.actor.clone(),
            &before,
            now_ms() + 120_000,
        )
        .unwrap();
        if !expired {
            resolution.operation_digest = format!("sha256:{}", "0".repeat(64));
        }
        let permit = ResolutionPermit::for_test(resolution);
        if expired {
            permit.expire_for_test();
        }
        let key = (intent.machine_id.clone(), intent.plugin_id.clone());
        assert!(
            resolve_no_effect(
                store.clone(),
                permit,
                OperationFence::acquire_resolution(&fences, key.clone()).unwrap()
            )
            .await
            .is_err()
        );
        assert_eq!(
            fences.read().get(&key),
            Some(&PluginFenceState::NeedsReconcile)
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
}
