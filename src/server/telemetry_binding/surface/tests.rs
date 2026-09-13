use super::super::resolution::surface::tests::Fixture;
use super::*;
use axum::http::Method;
use serde_json::{Value, json};

const PLAN: &str = "/api/telemetry/binding/plan";
const CONFIRM: &str = "/api/telemetry/binding/confirm";
const CHOICES: &str = "/api/telemetry/binding/choices";

#[cfg(feature = "machine-host")]
fn confirmation(plan: &Value) -> Value {
    json!({"plan_id":plan["plan_id"], "action":plan["action"]})
}

#[tokio::test]
async fn closed_gate_and_closed_requests_do_not_create_a_namespace() {
    let f = Fixture::new(false).await;
    for body in [
        json!({}),
        json!({"action":"select"}),
        json!({"action":"recover"}),
        json!({"action":"revoke","actor":"operator"}),
        json!({"action":"restore","operation_id":"old","force":true}),
    ] {
        let (status, value) = f.request(Method::POST, PLAN, Some(body)).await;
        assert!(status.is_client_error());
        assert_eq!(value["error"], "invalid_request");
    }
    let (status, value) = f
        .request(
            Method::POST,
            CONFIRM,
            Some(json!({"plan_id":"unknown-plan-id-123", "action":"select"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value["error"], "binding_admission_closed");
    let (_, choices) = f.request(Method::GET, CHOICES, None).await;
    assert_eq!(choices["targets"], json!([]));
    assert_eq!(choices["revoke_available"], false);
    assert!(f.state.ledger().await.unwrap().is_none());
    assert!(f.state.legacy_fence.allows_legacy());
    f.stop().await;
}

#[test]
fn public_contract_has_no_serialized_authority() {
    let mut intent = crate::telemetry_binding::fixture("binding-surface-fixture");
    intent.schema = 2;
    intent.expires_at_ms = 2_300_000_000_000;
    let plan = PlanView::new(&intent, false, None).unwrap();
    let receipt = ReceiptView::new(&Operation {
        intent,
        progress: Progress::Aborted,
    })
    .unwrap();
    let actual = json!({"plan":plan,"receipt":receipt});
    // This fixture is also decoded by the independent Web implementation.
    let expected: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/telemetry-binding-surface.json"
    ))
    .unwrap();
    assert_eq!(expected, actual);
}

#[cfg(feature = "machine-host")]
mod signed {
    use super::super::super::live::tests::Fixture as Machine;
    use super::*;
    use std::sync::atomic::Ordering;

    async fn setup(write: bool, auth: bool) -> (Machine, Fixture) {
        let machine = Machine::new(true, false).await;
        let http = Fixture::with_binding(
            write,
            auth,
            machine.control.clone(),
            machine.catalog.clone(),
            machine.fences.clone(),
        )
        .await;
        (machine, http)
    }

    fn select(machine: &Machine) -> Value {
        json!({"action":"select", "target":{
            "machine_id":"machine-test", "installation": {
                "plugin_id": machine.installed.plugin_id,
                "plugin_version": machine.installed.plugin_version,
                "generation_digest": machine.installed.generation_digest,
                "contract_fingerprint": machine.installed.contract_fingerprint,
                "installation_revision": machine.installed.installation_revision,
            }
        }})
    }

    #[tokio::test]
    async fn actual_cookie_logout_role_loss_and_disable_refuse_confirmation_and_durable_reads() {
        use crate::store::{ProductUser, ProductUserSession};
        for completed in [false, true] {
            for failure in ["logout", "role", "disabled"] {
                let (machine, f) = setup(true, true).await;
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
                    token_hash: crate::admin::hex_sha256(b"binding-cookie"),
                    session_id: "binding-cookie-session".into(),
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
                assert_eq!(
                    f.request(Method::GET, CHOICES, None).await.0,
                    StatusCode::UNAUTHORIZED
                );
                let response = f
                    .client
                    .post(format!("{}{PLAN}", f.base))
                    .header(header::COOKIE, "cowboy_user=binding-cookie")
                    .json(&select(&machine))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let plan: Value = response.json().await.unwrap();
                if completed {
                    let response = f
                        .client
                        .post(format!("{}{CONFIRM}", f.base))
                        .header(header::COOKIE, "cowboy_user=binding-cookie")
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
                                &session.session_id,
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
                let response = f
                    .client
                    .post(format!("{}{CONFIRM}", f.base))
                    .header(header::COOKIE, "cowboy_user=binding-cookie")
                    .json(&confirmation(&plan))
                    .send()
                    .await
                    .unwrap();
                assert!(matches!(
                    response.status(),
                    StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED
                ));
                let response = f
                    .client
                    .get(format!(
                        "{}/api/telemetry/binding/operations/{}/receipt",
                        f.base,
                        plan["plan_id"].as_str().unwrap()
                    ))
                    .header(header::COOKIE, "cowboy_user=binding-cookie")
                    .send()
                    .await
                    .unwrap();
                assert!(matches!(
                    response.status(),
                    StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED
                ));
                assert_eq!(
                    machine.sends.load(Ordering::Relaxed),
                    usize::from(completed)
                );
                f.stop().await;
            }
        }
    }

    #[tokio::test]
    async fn plans_are_bounded_and_foreign_actors_services_and_actions_cannot_consume_them() {
        let (machine, f) = setup(true, false).await;
        let (_, plan) = f.request(Method::POST, PLAN, Some(select(&machine))).await;
        let original = f
            .state
            .binding_plans
            .0
            .lock()
            .get(plan["plan_id"].as_str().unwrap())
            .unwrap()
            .intent
            .clone();
        let request = Confirm {
            plan_id: original.operation_id.clone(),
            action: Action::Select,
        };
        assert!(
            f.state
                .binding_plans
                .consume(
                    &Actor::Product {
                        user_id: "different".into()
                    },
                    &f.state.service,
                    &request
                )
                .is_err()
        );
        assert!(
            f.state
                .binding_plans
                .consume(&original.actor, "foreign-service", &request)
                .is_err()
        );
        for index in 1..=256 {
            let mut intent = original.clone();
            intent.operation_id = format!("bounded-binding-preview-{index}");
            let effects = f.state.binding_effects(&intent).unwrap();
            let inserted = f.state.binding_plans.insert(Preview {
                before: fingerprint(&None).unwrap(),
                budget: OperationBudget::new(
                    intent.expires_at_ms,
                    Duration::from_mins(1),
                    TimeSample::now(),
                ),
                intent,
                effects,
            });
            assert_eq!(inserted.is_ok(), index < 256);
        }
        assert_eq!(f.state.binding_plans.0.lock().len(), 256);
        let oversized = f
            .client
            .post(format!("{}{PLAN}", f.base))
            .header(header::CONTENT_TYPE, "application/json")
            .body("x".repeat(1025))
            .send()
            .await
            .unwrap();
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            oversized.json::<Value>().await.unwrap()["error"],
            "invalid_request"
        );
        assert_eq!(machine.sends.load(Ordering::Relaxed), 0);
        f.stop().await;
    }

    #[tokio::test]
    async fn lost_http_observer_detaches_one_real_machine_attempt_and_receipt_get_does_not_resend()
    {
        let machine = Machine::new(true, true).await;
        let f = Fixture::with_binding(
            true,
            false,
            machine.control.clone(),
            machine.catalog.clone(),
            machine.fences.clone(),
        )
        .await;
        let (_, plan) = f.request(Method::POST, PLAN, Some(select(&machine))).await;
        // Real Machine commits, then its ACK is dropped. The HTTP observer
        // times out well before the coordinator's real 45-second RPC timeout.
        let response = f
            .client
            .post(format!("{}{CONFIRM}", f.base))
            .timeout(Duration::from_secs(1))
            .json(&confirmation(&plan))
            .send()
            .await;
        assert!(response.unwrap_err().is_timeout());
        tokio::time::timeout(Duration::from_secs(65), async {
            loop {
                let ledger = f.state.ledger().await.unwrap().unwrap();
                if matches!(
                    ledger.operations.last().unwrap().progress,
                    Progress::Completed { .. }
                ) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap();
        let (_, receipt) = f
            .request(
                Method::GET,
                &format!(
                    "/api/telemetry/binding/operations/{}/receipt",
                    plan["plan_id"].as_str().unwrap()
                ),
                None,
            )
            .await;
        assert_eq!(receipt["operation"]["phase"], "completed");
        assert_eq!(receipt["request_digest"], plan["request_digest"]);
        assert_eq!(machine.sends.load(Ordering::Relaxed), 1);
        assert_eq!(machine.queries.load(Ordering::Relaxed), 3); // preview, preflight, ambiguous ACK
        f.stop().await;
    }

    async fn apply(f: &Fixture, request: Value) -> Value {
        let (status, plan) = f.request(Method::POST, PLAN, Some(request)).await;
        assert_eq!(status, StatusCode::OK, "{plan}");
        let (status, result) = f
            .request(Method::POST, CONFIRM, Some(confirmation(&plan)))
            .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["request_digest"], plan["request_digest"]);
        assert_eq!(result["operation"]["phase"], "completed");
        plan
    }

    #[tokio::test]
    async fn signed_selection_revoke_and_both_exact_restorations_use_real_http_wire_and_writer() {
        let (machine, f) = setup(true, false).await;
        let (_, choices) = f.request(Method::GET, CHOICES, None).await;
        assert_eq!(choices["targets"].as_array().unwrap().len(), 1);
        let selected = apply(&f, select(&machine)).await;
        assert!(!f.state.legacy_fence.allows_legacy());
        // Restoration to absence does not lower either axis.
        let restored = apply(
            &f,
            json!({"action":"restore","operation_id":selected["plan_id"]}),
        )
        .await;
        assert_eq!(restored["result_head"]["revision"], "2");
        assert_eq!(restored["result_head"]["selection"], Value::Null);
        assert_eq!(
            f.request(
                Method::POST,
                PLAN,
                Some(json!({"action":"restore","operation_id":selected["plan_id"]}))
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        apply(&f, select(&machine)).await;
        let revoked = apply(&f, json!({"action":"revoke"})).await;
        let restored = apply(
            &f,
            json!({"action":"restore","operation_id":revoked["plan_id"]}),
        )
        .await;
        assert_eq!(restored["result_head"]["revision"], "5");
        assert_eq!(restored["result_head"]["policy_epoch"], "5");
        assert!(restored["result_head"]["selection"].is_object());
        assert_eq!(machine.sends.load(Ordering::Relaxed), 5);
        // No process-local preview handles are needed for old durable receipts,
        // even when another operation owns the current head.
        f.state.binding_plans.0.lock().clear();
        let queries = machine.queries.load(Ordering::Relaxed);
        let (_, receipt) = f
            .request(
                Method::GET,
                &format!(
                    "/api/telemetry/binding/operations/{}/receipt",
                    selected["plan_id"].as_str().unwrap()
                ),
                None,
            )
            .await;
        assert_eq!(receipt["request_digest"], selected["request_digest"]);
        assert_eq!(receipt["operation"]["phase"], "completed");
        assert_eq!(machine.queries.load(Ordering::Relaxed), queries);
        f.stop().await;
    }

    #[tokio::test]
    async fn previews_cannot_enable_production_or_substitute_a_catalog_installation() {
        let (machine, f) = setup(false, false).await;
        for field in [
            "generation_digest",
            "contract_fingerprint",
            "installation_revision",
            "plugin_version",
        ] {
            let mut request = select(&machine);
            request["target"]["installation"][field] = match field {
                "installation_revision" => json!(format!("installation-{}", "a".repeat(64))),
                "plugin_version" => json!("9.9.9"),
                _ => json!(format!("sha256:{}", "a".repeat(64))),
            };
            assert_eq!(
                f.request(Method::POST, PLAN, Some(request)).await.0,
                StatusCode::CONFLICT
            );
        }
        assert_eq!(machine.queries.load(Ordering::Relaxed), 0);
        let (status, plan) = f.request(Method::POST, PLAN, Some(select(&machine))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(plan["confirmation_available"], false);
        assert_eq!(
            f.request(Method::POST, CONFIRM, Some(confirmation(&plan)))
                .await
                .0,
            StatusCode::CONFLICT
        );
        assert_eq!(machine.sends.load(Ordering::Relaxed), 0);
        assert!(f.state.ledger().await.unwrap().is_none());
        assert!(f.state.legacy_fence.allows_legacy());
        f.stop().await;
    }

    #[tokio::test]
    async fn original_connection_expiry_service_cas_and_fences_end_consumed_plans() {
        for failure in ["connection", "expiry", "service", "fence"] {
            let (machine, f) = setup(true, false).await;
            let (_, plan) = f.request(Method::POST, PLAN, Some(select(&machine))).await;
            match failure {
                "connection" => {
                    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
                    machine.control.install(
                        "machine-test".into(),
                        "fixture-epoch".into(),
                        false,
                        17,
                        tx,
                    );
                }
                "expiry" => {
                    let mut plans = f.state.binding_plans.0.lock();
                    plans
                        .get_mut(plan["plan_id"].as_str().unwrap())
                        .unwrap()
                        .budget =
                        OperationBudget::new(1, Duration::from_mins(1), TimeSample::now());
                }
                "service" => {
                    f.pending(false).await;
                }
                "fence" => {
                    machine.fences.write().insert(
                        ("machine-test".into(), "victoria".into()),
                        crate::server::PluginFenceState::Installing,
                    );
                }
                _ => unreachable!(),
            }
            assert_eq!(
                f.request(Method::POST, CONFIRM, Some(confirmation(&plan)))
                    .await
                    .0,
                StatusCode::CONFLICT
            );
            machine.fences.write().clear();
            assert_eq!(
                f.request(Method::POST, CONFIRM, Some(confirmation(&plan)))
                    .await
                    .0,
                StatusCode::CONFLICT
            );
            assert_eq!(machine.sends.load(Ordering::Relaxed), 0);
            f.stop().await;
        }
    }

    #[tokio::test]
    async fn concurrent_confirmation_has_one_dispatch_and_purpose_mismatch_does_not_consume() {
        let (machine, f) = setup(true, false).await;
        let (_, plan) = f.request(Method::POST, PLAN, Some(select(&machine))).await;
        assert_eq!(
            f.request(
                Method::POST,
                CONFIRM,
                Some(json!({"plan_id":plan["plan_id"],"action":"revoke"}))
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        let (a, b) = tokio::join!(
            f.request(Method::POST, CONFIRM, Some(confirmation(&plan))),
            f.request(Method::POST, CONFIRM, Some(confirmation(&plan)))
        );
        assert_eq!(
            [a.0, b.0]
                .into_iter()
                .filter(|s| *s == StatusCode::OK)
                .count(),
            1
        );
        assert_eq!(machine.sends.load(Ordering::Relaxed), 1);
        assert_eq!(f.state.ledger().await.unwrap().unwrap().operations.len(), 1);
        f.stop().await;
    }
}
