use super::*;
use crate::machine_protocol::telemetry_binding::{
    BindingCommitFailure, BindingOutcome, BindingUnavailable,
};
use crate::machine_protocol::telemetry_recovery::{
    RecoveryResult, RecoverySnapshot, observed_fixture,
};
use crate::machine_protocol::{MachineCommand, MachineEvent};
use crate::server::telemetry_binding::resolution::surface::tests::Fixture;
use crate::telemetry_binding::Attention;
use axum::http::Method;
use serde_json::{Value, json};
use std::sync::atomic::AtomicUsize;

fn path(before: &Operation, suffix: &str) -> String {
    format!(
        "/api/telemetry/binding/operations/{}/{suffix}",
        before.intent.operation_id
    )
}

async fn pending(f: &Fixture) -> Operation {
    let before = f.pending(true).await;
    crate::server::telemetry_binding::advance(
        f.state.store.as_ref().unwrap(),
        &before,
        Progress::NeedsAttention {
            reason: Attention::Uncertain,
            observation: None,
        },
    )
    .await
    .unwrap()
}

async fn inspect(f: &Fixture, before: &Operation) -> (StatusCode, Value) {
    f.request(
        Method::POST,
        &path(before, "machine-recovery-plan"),
        Some(json!({})),
    )
    .await
}

fn confirmation(plan: &Value) -> Value {
    json!({"plan_id":plan["plan_id"], "action":plan["action"]})
}

struct Wire {
    task: tokio::task::JoinHandle<()>,
    queries: Arc<AtomicUsize>,
    sends: Arc<AtomicUsize>,
}

impl Wire {
    fn new(f: &Fixture, mode: &'static str) -> Self {
        let queries = Arc::new(AtomicUsize::new(0));
        let sends = Arc::new(AtomicUsize::new(0));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        f.state.control.install(
            "machine-test".into(),
            "recovery-surface-wire".into(),
            false,
            if mode == "old" { 16 } else { 17 },
            tx,
        );
        let connection = f
            .state
            .control
            .operation_connection("machine-test")
            .unwrap();
        let control = f.state.control.clone();
        let q = queries.clone();
        let s = sends.clone();
        let task = tokio::spawn(async move {
            let mut saved: Option<RecoveryObservation> = None;
            while let Some(command) = rx.recv().await {
                match command {
                    MachineCommand::QueryTelemetryRecovery {
                        request_id,
                        recovery,
                    } => {
                        let count = q.fetch_add(1, Ordering::Relaxed) + 1;
                        let mut binding = prepared(&recovery.step).unwrap();
                        let mut receipt = None;
                        if let Some(RecoveryObservation::Observed { snapshot }) = &saved {
                            binding = snapshot.binding.clone();
                            receipt = snapshot.receipt.clone().filter(|r| r.request == *recovery);
                        }
                        if mode == "unknown" || (mode == "changed" && count >= 2) {
                            let BindingObservation::Observed { snapshot } = &mut binding else {
                                panic!()
                            };
                            snapshot.receipt.as_mut().unwrap().outcome = BindingOutcome::Unknown {};
                        }
                        let mut observation = RecoveryObservation::Observed {
                            snapshot: Box::new(RecoverySnapshot {
                                request_digest: recovery.digest().unwrap(),
                                receipt,
                                binding,
                            }),
                        };
                        if mode == "unavailable" {
                            observation = RecoveryObservation::Unavailable {
                                reason: BindingUnavailable::Storage,
                            };
                        }
                        control.record_remote(
                            &connection,
                            MachineEvent::TelemetryRecoveryObservation {
                                request_id,
                                observation: Box::new(observation),
                            },
                        );
                    }
                    MachineCommand::RecoverTelemetryBinding {
                        request_id,
                        recovery,
                    } => {
                        s.fetch_add(1, Ordering::Relaxed);
                        let observation = observed_fixture(&recovery);
                        saved = Some(observation.clone());
                        let result = if mode == "ambiguous" {
                            RecoveryResult::Unavailable {
                                failure: BindingCommitFailure::Unavailable(
                                    BindingUnavailable::Storage,
                                ),
                            }
                        } else {
                            RecoveryResult::Observed { observation }
                        };
                        control.record_remote(
                            &connection,
                            MachineEvent::TelemetryBindingRecovered {
                                request_id,
                                result: Box::new(result),
                            },
                        );
                    }
                    _ => panic!(
                        "Machine recovery cannot dispatch binding, export or Service resolution"
                    ),
                }
            }
        });
        Self {
            task,
            queries,
            sends,
        }
    }

    async fn stop(self) {
        self.task.abort();
        assert!(self.task.await.unwrap_err().is_cancelled());
    }
}

#[tokio::test]
async fn recovery_http_has_a_separate_closed_production_gate_and_read_only_preview() {
    let f = Fixture::with_recovery(false, false).await;
    let before = pending(&f).await;
    let wire = Wire::new(&f, "prepared");
    let original = f.state.ledger().await.unwrap();
    let (status, plan) = inspect(&f, &before).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(plan["confirmation_available"], false);
    assert_eq!(plan["operation"]["phase"], "needs_attention");
    assert_eq!(plan["machine_head"]["revision"], "0");
    let result = f
        .request(
            Method::POST,
            &path(&before, "recover-machine"),
            Some(confirmation(&plan)),
        )
        .await;
    assert_eq!(
        result,
        (
            StatusCode::CONFLICT,
            json!({"schema":1,"error":"recovery_admission_closed"})
        )
    );
    assert_eq!(wire.queries.load(Ordering::Relaxed), 1);
    assert_eq!(wire.sends.load(Ordering::Relaxed), 0);
    assert_eq!(f.state.ledger().await.unwrap(), original);
    assert!(original.as_ref().unwrap().resolutions.is_empty());
    wire.stop().await;
    f.stop().await;
}

#[tokio::test]
async fn recovery_http_is_one_use_with_exact_read_after_ambiguous_ack_and_no_service_change() {
    for mode in ["prepared", "ambiguous"] {
        let f = Fixture::with_recovery(true, false).await;
        let before = pending(&f).await;
        let wire = Wire::new(&f, mode);
        let original = f.state.ledger().await.unwrap();
        let (_, plan) = inspect(&f, &before).await;
        let uri = path(&before, "recover-machine");
        let (a, b) = tokio::join!(
            f.request(Method::POST, &uri, Some(confirmation(&plan))),
            f.request(Method::POST, &uri, Some(confirmation(&plan)))
        );
        let (status, receipt) = if a.0 == StatusCode::OK {
            assert_eq!(b.0, StatusCode::CONFLICT);
            a
        } else {
            assert_eq!(a.0, StatusCode::CONFLICT);
            b
        };
        assert_eq!(status, StatusCode::OK);
        assert_eq!(receipt["resolution_id"], plan["plan_id"]);
        assert_eq!(receipt["request_digest"], plan["request_digest"]);
        assert_eq!(
            wire.queries.load(Ordering::Relaxed),
            if mode == "ambiguous" { 3 } else { 2 }
        );
        assert_eq!(wire.sends.load(Ordering::Relaxed), 1);
        let id = plan["plan_id"].as_str().unwrap();
        let query = path(&before, &format!("machine-recoveries/{id}"));
        for _ in 0..2 {
            assert_eq!(
                f.request(Method::GET, &query, None).await,
                (StatusCode::OK, receipt.clone())
            );
        }
        assert_eq!(wire.sends.load(Ordering::Relaxed), 1);
        assert_eq!(f.state.ledger().await.unwrap(), original);
        assert_eq!(
            inspect(&f, &before).await.0,
            StatusCode::CONFLICT,
            "rejected Machine state is not a new Prepared recovery"
        );
        // Restart loses only the bounded query handle, not evidence or a permit.
        f.state.recovery_plans.0.lock().clear();
        assert_eq!(
            f.request(Method::GET, &query, None).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(wire.sends.load(Ordering::Relaxed), 1);
        wire.stop().await;
        f.stop().await;
    }
}

#[tokio::test]
async fn recovery_http_rejects_old_unknown_unavailable_or_changed_evidence() {
    for mode in ["old", "unknown", "unavailable", "changed"] {
        let f = Fixture::with_recovery(true, false).await;
        let before = pending(&f).await;
        let wire = Wire::new(&f, mode);
        let original = f.state.ledger().await.unwrap();
        let (status, plan) = inspect(&f, &before).await;
        if mode == "changed" {
            assert_eq!(status, StatusCode::OK);
            let uri = path(&before, "recover-machine");
            assert_eq!(
                f.request(Method::POST, &uri, Some(confirmation(&plan)))
                    .await
                    .0,
                StatusCode::CONFLICT
            );
            assert_eq!(
                f.request(Method::POST, &uri, Some(confirmation(&plan)))
                    .await
                    .0,
                StatusCode::CONFLICT
            );
            assert_eq!(wire.queries.load(Ordering::Relaxed), 2);
        } else {
            assert_eq!(status, StatusCode::CONFLICT);
        }
        assert_eq!(wire.sends.load(Ordering::Relaxed), 0);
        assert_eq!(f.state.ledger().await.unwrap(), original);
        wire.stop().await;
        f.stop().await;
    }
}

#[tokio::test]
async fn recovery_http_strict_bodies_and_foreign_operation_cannot_consume_preview() {
    let f = Fixture::with_recovery(true, false).await;
    let before = pending(&f).await;
    let wire = Wire::new(&f, "prepared");
    let (_, plan) = inspect(&f, &before).await;
    for field in ["force", "actor", "request", "expires_at_ms"] {
        let mut body = confirmation(&plan);
        body[field] = json!("forged");
        let (status, value) = f
            .request(Method::POST, &path(&before, "recover-machine"), Some(body))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(value, json!({"schema":1,"error":"invalid_request"}));
    }
    assert_eq!(
        f.request(
            Method::POST,
            &path(&before, "recover-machine"),
            Some(json!({"plan_id":plan["plan_id"],"action":"record_rejected"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        f.request(
            Method::POST,
            &path(&before, "machine-recovery-plan"),
            Some(json!({"action":"reject_interrupted_prepared"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        f.request(
            Method::POST,
            &path(&before, "machine-recovery-plan"),
            Some(json!({"extra":"x".repeat(1025)}))
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        f.request(
            Method::POST,
            "/api/telemetry/binding/operations/foreign-operation/recover-machine",
            Some(confirmation(&plan))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    // A Machine recovery reference cannot be used at the independent Service endpoint.
    assert_eq!(
        f.request(
            Method::POST,
            &path(&before, "resolve"),
            Some(confirmation(&plan))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    {
        let preview = f.state.recovery_plans.0.lock();
        assert!(
            preview
                .get(plan["plan_id"].as_str().unwrap())
                .unwrap()
                .budget
                .is_some()
        );
    }
    assert_eq!(wire.sends.load(Ordering::Relaxed), 0);
    wire.stop().await;
    f.stop().await;
}

#[tokio::test]
async fn recovery_http_rechecks_actual_cookie_role_and_account_before_confirmation_and_audit_read()
{
    use crate::store::{ProductUser, ProductUserSession};
    for (failure, submitted) in ["logout", "role", "disabled"]
        .into_iter()
        .flat_map(|f| [false, true].map(|s| (f, s)))
    {
        let f = Fixture::with_recovery(true, true).await;
        let now = chrono::Utc::now().timestamp_millis();
        let user = ProductUser {
            id: "c".repeat(32),
            username: "operator".into(),
            password_algo: "argon2id".into(),
            password_hash: "unused-fixture".into(),
            created_at_ms: now,
            updated_at_ms: now,
            disabled_at_ms: None,
        };
        let store = f.state.store.as_ref().unwrap();
        store.insert_user(&user).await.unwrap();
        f.state.hub.set_setting(
            crate::admin::PERMISSIONS_SETTING.into(),
            json!({"default_role":"operator","grants":[]}),
        );
        let session = ProductUserSession {
            token_hash: crate::admin::hex_sha256(b"surface-cookie"),
            session_id: "surface-session-fixture".into(),
            user_id: user.id.clone(),
            client_kind: "browser".into(),
            principal_class: "human".into(),
            created_at_ms: now,
            expires_at_ms: now + 300_000,
            last_seen_at_ms: now,
            user_agent: None,
            passkey_verified_at_ms: None,
            primary_authenticated_at_ms: now,
            primary_auth_method: Some("password".into()),
            revoked_at_ms: None,
            revoke_reason: None,
            auth_provider_id: None,
            auth_issuer: None,
            auth_subject: None,
            auth_sid: None,
            id_token_ciphertext: None,
        };
        store.insert_user_session(&session).await.unwrap();
        let before = pending(&f).await;
        let wire = Wire::new(&f, "prepared");
        assert_eq!(
            f.request(Method::GET, "/api/telemetry/binding", None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        let response = f
            .client
            .post(format!(
                "{}{}",
                f.base,
                path(&before, "machine-recovery-plan")
            ))
            .header(header::COOKIE, "cowboy_user=surface-cookie")
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let plan: Value = response.json().await.unwrap();
        if submitted {
            let response = f
                .client
                .post(format!("{}{}", f.base, path(&before, "recover-machine")))
                .header(header::COOKIE, "cowboy_user=surface-cookie")
                .json(&confirmation(&plan))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        match failure {
            "logout" => {
                store
                    .revoke_user_session_for_user(
                        &user.id,
                        "surface-session-fixture",
                        "logout",
                        now,
                    )
                    .await
                    .unwrap();
            }
            "role" => f.state.hub.set_setting(
                crate::admin::PERMISSIONS_SETTING.into(),
                json!({"default_role":"viewer","grants":[]}),
            ),
            _ => {
                store
                    .set_user_disabled_at(&user.id, Some(now))
                    .await
                    .unwrap();
            }
        }
        for (method, route, body) in [
            (
                Method::POST,
                path(&before, "recover-machine"),
                Some(confirmation(&plan)),
            ),
            (
                Method::GET,
                path(
                    &before,
                    &format!("machine-recoveries/{}", plan["plan_id"].as_str().unwrap()),
                ),
                None,
            ),
            (Method::GET, "/api/telemetry/binding".into(), None),
        ] {
            let mut request = f
                .client
                .request(method, format!("{}{route}", f.base))
                .header(header::COOKIE, "cowboy_user=surface-cookie");
            if let Some(body) = body {
                request = request.json(&body);
            }
            let response = request.send().await.unwrap();
            assert!(
                matches!(
                    response.status(),
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
                ),
                "{failure}"
            );
        }
        assert_eq!(
            f.state.ledger().await.unwrap().unwrap().operations,
            vec![before]
        );
        assert_eq!(wire.sends.load(Ordering::Relaxed), usize::from(submitted));
        wire.stop().await;
        f.stop().await;
    }
}

#[test]
fn recovery_preview_expiry_consumption_capacity_and_history_are_separate() {
    let plans = Plans::default();
    let request = crate::machine_protocol::telemetry_recovery::fixture();
    let insert = |r: RecoveryRequest| {
        plans.insert(
            r.clone(),
            OperationBudget::new(r.expires_at_ms, Duration::from_mins(1), TimeSample::now()),
        )
    };
    insert(request.clone()).unwrap();
    assert!(insert(request.clone()).is_err());
    let confirm = Confirmation {
        plan_id: request.resolution_id.clone(),
        action: request.action,
    };
    for (actor, service, op) in [
        (
            RecoveryActor::Admin {
                account: "foreign".into(),
            },
            request.step.service_id.as_str(),
            request.step.operation_id.as_str(),
        ),
        (
            request.actor.clone(),
            "foreign",
            request.step.operation_id.as_str(),
        ),
        (
            request.actor.clone(),
            request.step.service_id.as_str(),
            "foreign",
        ),
    ] {
        assert!(plans.consume(&actor, service, op, &confirm).is_err());
    }
    assert!(
        plans
            .submitted(
                &request.actor,
                &request.step.service_id,
                &request.step.operation_id,
                &request.resolution_id
            )
            .is_err()
    );
    let (saved, budget) = plans
        .consume(
            &request.actor,
            &request.step.service_id,
            &request.step.operation_id,
            &confirm,
        )
        .unwrap();
    assert_eq!(saved, request);
    budget.expire_for_test();
    assert!(
        plans
            .consume(
                &request.actor,
                &request.step.service_id,
                &request.step.operation_id,
                &confirm
            )
            .is_err()
    );
    assert_eq!(
        plans
            .submitted(
                &request.actor,
                &request.step.service_id,
                &request.step.operation_id,
                &request.resolution_id
            )
            .unwrap(),
        request
    );
    for i in 1..256 {
        let mut r = request.clone();
        r.resolution_id = format!("bounded-recovery-{i}");
        insert(r).unwrap();
    }
    let mut excess = request.clone();
    excess.resolution_id = "bounded-recovery-overflow".into();
    assert!(insert(excess).is_err());
    let mut lock = plans.0.lock();
    let p = lock.get_mut("bounded-recovery-1").unwrap();
    p.budget.as_ref().unwrap().expire_for_test();
    drop(lock);
    let expired = Confirmation {
        plan_id: "bounded-recovery-1".into(),
        action: request.action,
    };
    assert!(
        plans
            .consume(
                &request.actor,
                &request.step.service_id,
                &request.step.operation_id,
                &expired
            )
            .is_err()
    );
    plans
        .0
        .lock()
        .get_mut(&request.resolution_id)
        .unwrap()
        .retain_until = Instant::now();
    assert!(
        plans
            .submitted(
                &request.actor,
                &request.step.service_id,
                &request.step.operation_id,
                &request.resolution_id
            )
            .is_err()
    );
}

#[tokio::test]
async fn recovery_http_service_cas_and_original_preview_expiry_prevent_dispatch() {
    for changed in [true, false] {
        let f = Fixture::with_recovery(true, false).await;
        let before = pending(&f).await;
        let wire = Wire::new(&f, "prepared");
        let (_, plan) = inspect(&f, &before).await;
        if changed {
            use crate::telemetry_binding::resolution::{
                ResolutionAction, ResolutionIntent, ResolutionPermit,
            };
            let observation = crate::telemetry_binding::tests::applied(&before.intent);
            let expires = chrono::Utc::now().timestamp_millis() + 60_000;
            let intent = ResolutionIntent::new(
                "concurrent-service-resolution".into(),
                before.intent.actor.clone(),
                &before,
                ResolutionAction::AcceptApplied {
                    observation_digest: binding_digest(&serde_json::to_vec(&observation).unwrap()),
                },
                expires,
            )
            .unwrap();
            let permit = ResolutionPermit::new(
                intent,
                before.clone(),
                Some(observation),
                OperationBudget::new(expires, Duration::from_mins(1), TimeSample::now()),
            )
            .unwrap();
            f.state
                .store
                .as_ref()
                .unwrap()
                .change_telemetry_binding(&Change::Resolve(&permit), &|| true)
                .await
                .unwrap();
        } else {
            f.state
                .recovery_plans
                .0
                .lock()
                .get(plan["plan_id"].as_str().unwrap())
                .unwrap()
                .budget
                .as_ref()
                .unwrap()
                .expire_for_test();
        }
        let expected = f.state.ledger().await.unwrap();
        for _ in 0..2 {
            assert_eq!(
                f.request(
                    Method::POST,
                    &path(&before, "recover-machine"),
                    Some(confirmation(&plan))
                )
                .await
                .0,
                StatusCode::CONFLICT
            );
        }
        assert_eq!(wire.queries.load(Ordering::Relaxed), 1);
        assert_eq!(wire.sends.load(Ordering::Relaxed), 0);
        assert_eq!(f.state.ledger().await.unwrap(), expected);
        wire.stop().await;
        f.stop().await;
    }
}

#[test]
fn recovery_public_projection_matches_shared_web_fixture() {
    let source: Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/telemetry-resolution-surface.json"
    ))
    .unwrap();
    let mut before: Operation = serde_json::from_value(source["source_operation"].clone()).unwrap();
    before.progress = Progress::NeedsAttention {
        reason: Attention::Uncertain,
        observation: None,
    };
    let mut request = super::super::request_fixture(
        &before,
        &crate::plugin_operation::Actor::Product {
            user_id: "new-fixture-operator".into(),
        },
    );
    request.expires_at_ms = 2_300_000_000_000;
    request.resolution_id = "machine-recovery-contract-fixture".into();
    let mut observed = observed_fixture(&request);
    let RecoveryObservation::Observed { snapshot } = &mut observed else {
        panic!()
    };
    snapshot.receipt.as_mut().unwrap().resolved_at_ms = 2_200_000_000_000;
    let actual = json!({"plan":PlanView::new(&request,&before,false).unwrap(),"receipt":ReceiptView::new(&request,&observed).unwrap()});
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/telemetry-recovery-surface.json"
    ))
    .unwrap();
    assert_eq!(actual, fixture);
}
