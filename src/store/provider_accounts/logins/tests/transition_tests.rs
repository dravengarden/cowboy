use super::*;

#[allow(clippy::too_many_lines)] // One lifecycle fixture shared by both database backends.
pub(super) async fn contract(
    store: &Store,
    old: &NativeCompletion,
    meta: &crate::core::SessionMeta,
) {
    use crate::store::provider_accounts::transitions::{Intent, Request as Transition};
    let input = Request {
        account_id: old.profile.account_id.clone(),
        machine_id: old.machine_id.clone(),
        provider_version: "new-version".into(),
        generation_digest: "new-digest".into(),
        method: old.profile.method.clone(),
    };
    let claim = store
        .claim_provider_login("login-owner", "migration-login", &input)
        .await
        .unwrap();
    let mut target = old.clone();
    target.profile.request_id = claim.receipt.request_id.clone();
    target.profile.generation = claim.receipt.native_generation.unwrap();
    target.profile.provider_version = input.provider_version;
    target.profile.generation_digest = input.generation_digest;
    let intent = Intent {
        request: Transition {
            session_id: meta.id.clone(),
            account_id: meta.provider_account_id.clone(),
            machine_id: meta.machine_id.clone(),
            provider: meta.provider.clone(),
            from_version: meta.provider_version.clone(),
            from_digest: meta.provider_generation_digest.clone(),
            from_generation: meta.provider_auth_generation.unwrap(),
            target_login_operation: "migration-login".into(),
        },
        target: target.clone(),
    };
    assert!(
        store
            .plan_native_session_transition("login-owner", "move", &intent)
            .await
            .is_err()
    );
    assert!(
        store
            .native_session_transition("login-owner", "move")
            .await
            .unwrap()
            .is_none()
    );
    let revision = store
        .provider_accounts("login-owner")
        .await
        .unwrap()
        .revision;
    store
        .with_provider_login_dispatch(
            "login-owner",
            "migration-login",
            &claim,
            &DispatchScope {
                registry_revision: revision,
                recipient_fingerprint: target.recipient_fingerprint.clone(),
            },
            || Ok(()),
        )
        .await
        .unwrap();
    store
        .record_native_login_completion("login-owner", "migration-login", &target)
        .await
        .unwrap();
    store
        .bind_native_login_identity("login-owner", "migration-login", &target, revision)
        .await
        .unwrap();
    let mut wrong = intent.clone();
    wrong.request.from_digest = "stale".into();
    assert!(
        store
            .plan_native_session_transition("login-owner", "stale", &wrong)
            .await
            .is_err()
    );
    let (a, b) = tokio::join!(
        store.plan_native_session_transition("login-owner", "move", &intent),
        store.plan_native_session_transition("login-owner", "move", &intent),
    );
    a.unwrap();
    b.unwrap();
    assert_stage(store, "planned").await;
    assert_eq!(
        store
            .native_session_transition("login-owner", "move")
            .await
            .unwrap(),
        Some(intent.clone())
    );
    assert!(
        store
            .native_session_transition("other-owner", "move")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .plan_native_session_transition("login-owner", "move", &wrong)
            .await
            .is_err()
    );
    assert!(
        store
            .plan_native_session_transition("login-owner", "parallel-move", &intent)
            .await
            .is_err()
    );
    // Planning changes neither persisted session binding nor its launch authority.
    assert_eq!(
        store
            .native_login_for_registered_session("login-owner", &meta.id)
            .await
            .unwrap(),
        Some(("bind".into(), old.clone()))
    );
    store
        .with_native_session_binding("login-owner", "bind", old, &meta.id, || async { Ok(()) })
        .await
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/plugins/claude-code/provider.json"
    )))
    .unwrap();
    let commit = crate::store::provider_accounts::transitions::BindingCommit {
        behavior: serde_json::from_value(manifest["runtime"]["behavior"].clone()).unwrap(),
    };
    assert!(
        store
            .commit_native_session_transition("login-owner", "move", &commit)
            .await
            .is_err()
    );
    assert!(
        store
            .native_session_transition_commit("login-owner", "move")
            .await
            .unwrap()
            .is_none()
    );
    let stopped = stop_contract(store, &intent).await;
    assert_stage(store, "stop_confirmed").await;
    commit_contract(store, &intent, &commit).await;
    assert_stage(store, "binding_committed").await;
    finish_contract(store, &intent, &stopped).await;
    assert_stage(store, "completed").await;
    store
        .cancel_provider_login("login-owner", "migration-login")
        .await
        .unwrap();
    // Exact replay is historical and must not be mistaken for current admission.
    store
        .plan_native_session_transition("login-owner", "move", &intent)
        .await
        .unwrap();
    assert!(
        store
            .with_native_login_binding("login-owner", "migration-login", &target, || Ok(()))
            .await
            .is_err()
    );
    store
        .record_native_session_transition_stop("login-owner", "move", &stopped, || {
            panic!("historical evidence must not require a live connection")
        })
        .await
        .unwrap();
    assert!(
        store
            .dispatch_native_session_transition_stop("login-owner", "move", |_| -> Result<()> {
                panic!("cancelled target must not dispatch a stop");
            })
            .await
            .is_err()
    );
    store
        .commit_native_session_transition("login-owner", "move", &commit)
        .await
        .unwrap();
}

async fn commit_contract(
    store: &Store,
    intent: &crate::store::provider_accounts::transitions::Intent,
    commit: &crate::store::provider_accounts::transitions::BindingCommit,
) {
    store
        .update_agent_session_id(&intent.request.session_id, Some("old-native-thread"))
        .await
        .unwrap();
    assert!(
        store
            .commit_native_session_transition_checked("login-owner", "move", commit, || {
                anyhow::bail!("machine_connection_changed")
            })
            .await
            .is_err()
    );
    assert!(
        store
            .native_session_transition_commit("login-owner", "move")
            .await
            .unwrap()
            .is_none()
    );
    let (_, unchanged) = store
        .native_login_for_registered_session("login-owner", &intent.request.session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.profile.generation, intent.request.from_generation);
    assert_eq!(
        unchanged.profile.provider_version,
        intent.request.from_version
    );
    let (a, b) = tokio::join!(
        store.commit_native_session_transition("login-owner", "move", commit),
        store.commit_native_session_transition("login-owner", "move", commit)
    );
    a.unwrap();
    b.unwrap();
    store
        .commit_native_session_transition_checked("login-owner", "move", commit, || {
            panic!("historical receipt must not rerun live validation")
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .native_session_transition_commit("login-owner", "move")
            .await
            .unwrap(),
        Some(commit.clone())
    );
    assert!(
        store
            .native_session_transition_commit("other-owner", "move")
            .await
            .unwrap()
            .is_none()
    );
    let mut wrong = commit.clone();
    wrong.behavior.schema_version += 1;
    assert!(
        store
            .commit_native_session_transition("login-owner", "move", &wrong)
            .await
            .is_err()
    );
    assert_eq!(
        store
            .native_login_for_registered_session("login-owner", &intent.request.session_id)
            .await
            .unwrap(),
        Some((
            intent.request.target_login_operation.clone(),
            intent.target.clone()
        ))
    );
    let query = "SELECT status,agent_session_id,provider_behavior FROM sessions WHERE id=$1";
    let (status, native_id, behavior): (String, Option<String>, serde_json::Value) =
        match &store.backend {
            StorageBackend::Postgres(db) => sqlx::query_as(query)
                .bind(&intent.request.session_id)
                .fetch_one(&db.pool)
                .await
                .unwrap(),
            StorageBackend::Sqlite(db) => sqlx::query_as(query)
                .bind(&intent.request.session_id)
                .fetch_one(&db.pool)
                .await
                .unwrap(),
        };
    assert_eq!(status, "exited");
    assert_eq!(native_id, None);
    assert_eq!(behavior, serde_json::to_value(&commit.behavior).unwrap());
}

async fn assert_stage(store: &Store, expected: &str) {
    assert_eq!(
        store
            .native_session_transition_stage("login-owner", "move")
            .await
            .unwrap()
            .as_deref(),
        Some(expected)
    );
    for (owner, operation) in [("other-owner", "move"), ("login-owner", "missing")] {
        assert!(
            store
                .native_session_transition_stage(owner, operation)
                .await
                .unwrap()
                .is_none()
        );
    }
}

async fn finish_contract(
    store: &Store,
    intent: &crate::store::provider_accounts::transitions::Intent,
    stopped: &crate::store::provider_accounts::revocation::stops::Evidence,
) {
    let admitted = crate::provider_account::PreparedBinding {
        operation: "move".into(),
        stopped: stopped.tree_exit.clone().unwrap(),
        target: intent.target.profile.clone(),
    };
    assert!(
        !store
            .native_session_transition_finished("login-owner", "move")
            .await
            .unwrap()
    );
    let mut wrong = admitted.clone();
    wrong.stopped.launch_id = uuid::Uuid::new_v4();
    assert!(
        store
            .finish_native_session_transition("login-owner", "move", &wrong, || Ok(()))
            .await
            .is_err()
    );
    assert!(
        store
            .finish_native_session_transition("other-owner", "move", &admitted, || Ok(()))
            .await
            .is_err()
    );
    assert!(
        store
            .finish_native_session_transition("login-owner", "move", &admitted, || anyhow::bail!(
                "connection changed"
            ))
            .await
            .is_err()
    );
    assert!(
        !store
            .native_session_transition_finished("login-owner", "move")
            .await
            .unwrap()
    );
    store
        .finish_native_session_transition("login-owner", "move", &admitted, || Ok(()))
        .await
        .unwrap();
    assert!(
        store
            .native_session_transition_finished("login-owner", "move")
            .await
            .unwrap()
    );
    assert!(
        !store
            .native_session_transition_finished("other-owner", "move")
            .await
            .unwrap()
    );
    store
        .finish_native_session_transition("login-owner", "move", &admitted, || {
            panic!("historical finish must not dispatch")
        })
        .await
        .unwrap();
}

#[allow(clippy::too_many_lines)] // Paired-backend fixture covers dispatch, evidence admission and replay.
async fn stop_contract(
    store: &Store,
    intent: &crate::store::provider_accounts::transitions::Intent,
) -> crate::store::provider_accounts::revocation::stops::Evidence {
    use crate::store::provider_accounts::revocation::stops::Evidence;
    store
        .dispatch_native_session_transition_stop("login-owner", "move", |observed| {
            assert_eq!(observed, intent);
            Ok(())
        })
        .await
        .unwrap();
    for (owner, operation) in [("other-owner", "move"), ("login-owner", "missing")] {
        assert!(
            store
                .dispatch_native_session_transition_stop(owner, operation, |_| -> Result<()> {
                    panic!("missing or foreign intent must not dispatch");
                })
                .await
                .is_err()
        );
    }
    let mut evidence = Evidence {
        account_id: intent.request.account_id.clone(),
        session_id: intent.request.session_id.clone(),
        machine_id: intent.request.machine_id.clone(),
        recipient_fingerprint: intent.target.recipient_fingerprint.clone(),
        tree_exit: None,
    };
    assert!(
        store
            .record_native_session_transition_stop("login-owner", "move", &evidence, || true)
            .await
            .is_err()
    );
    evidence.tree_exit = Some(crate::provider_account::WorkerExitObservation {
        request: crate::provider_account::WorkerExitRequest {
            stop_command_id: Some(crate::provider_account::account_stop_command_id(
                "migration:move",
                &evidence.session_id,
                &evidence.account_id,
            )),
            account_id: evidence.account_id.clone(),
            session_id: evidence.session_id.clone(),
        },
        launch_id: uuid::Uuid::new_v4(),
    });
    assert!(
        store
            .record_native_session_transition_stop("login-owner", "move", &evidence, || false)
            .await
            .is_err()
    );
    assert!(
        store
            .native_session_transition_stop("login-owner", "move")
            .await
            .unwrap()
            .is_none()
    );
    for field in 0..6 {
        let mut wrong = evidence.clone();
        match field {
            0 => wrong.machine_id = "wrong".into(),
            1 => wrong.recipient_fingerprint = "wrong".into(),
            2 => wrong.tree_exit.as_mut().unwrap().launch_id = uuid::Uuid::nil(),
            3 => wrong.tree_exit.as_mut().unwrap().request.session_id = "wrong".into(),
            4 => wrong.tree_exit.as_mut().unwrap().request.stop_command_id = None,
            _ => {
                wrong.tree_exit.as_mut().unwrap().request.stop_command_id = Some("old-stop".into());
            }
        }
        assert!(
            store
                .record_native_session_transition_stop("login-owner", "move", &wrong, || true)
                .await
                .is_err()
        );
    }
    let (a, b) = tokio::join!(
        store.record_native_session_transition_stop("login-owner", "move", &evidence, || true),
        store.record_native_session_transition_stop("login-owner", "move", &evidence, || true)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(
        store
            .native_session_transition_stop("login-owner", "move")
            .await
            .unwrap(),
        Some(evidence.clone())
    );
    assert!(
        store
            .native_session_transition_stop("other-owner", "move")
            .await
            .unwrap()
            .is_none()
    );
    let mut wrong = evidence.clone();
    wrong.tree_exit.as_mut().unwrap().launch_id = uuid::Uuid::new_v4();
    assert!(
        store
            .record_native_session_transition_stop("login-owner", "move", &wrong, || true)
            .await
            .is_err()
    );
    evidence
}
