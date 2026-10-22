use super::super::tests::{path, pending};
use super::*;
use crate::machine_protocol::telemetry_binding::BindingUnavailable;
use crate::machine_protocol::telemetry_recovery::{RecoveryRequest, observed_fixture};
use crate::machine_protocol::telemetry_recovery_audit::RecoveryAuditSnapshot;
use crate::machine_protocol::{MachineCommand, MachineEvent};
use crate::server::telemetry_binding::resolution::surface::tests::Fixture;
use axum::http::Method;

fn evidence(query: &RecoveryAuditQuery, request: &RecoveryRequest) -> RecoveryAuditObservation {
    let RecoveryObservation::Observed { snapshot } = observed_fixture(request) else {
        panic!()
    };
    RecoveryAuditObservation::Observed {
        snapshot: Box::new(RecoveryAuditSnapshot {
            query_digest: query.digest().unwrap(),
            receipt: snapshot.receipt,
            binding: snapshot.binding,
        }),
    }
}

#[tokio::test]
async fn audit_http_is_read_only_strictly_correlated_and_does_not_require_a_plan() {
    for mode in [
        "recorded",
        "absent",
        "old",
        "service-digest",
        "step",
        "storage",
        "connection",
    ] {
        let f = Fixture::new(false).await;
        let before = pending(&f).await;
        let saved = f.state.ledger().await.unwrap();
        let mut request = crate::server::telemetry_binding::recovery::request_fixture(
            &before,
            &before.intent.actor,
        );
        if mode == "service-digest" {
            request.service_operation_digest = binding_digest(b"another NeedsAttention operation");
        }
        if mode == "step" {
            request.step.plan_digest = binding_digest(b"foreign intent");
            request.expected_observation_digest = binding_digest(
                &serde_json::to_vec(
                    &crate::machine_protocol::telemetry_recovery::prepared(&request.step).unwrap(),
                )
                .unwrap(),
            );
        }
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let connection = f.state.control.install(
            "machine-test".into(),
            "audit-wire".into(),
            false,
            if mode == "old" { 17 } else { 18 },
            tx,
        );
        let control = f.state.control.clone();
        let task = tokio::spawn(async move {
            if mode == "old" {
                return;
            }
            let MachineCommand::QueryTelemetryRecoveryAudit { request_id, query } =
                rx.recv().await.unwrap()
            else {
                panic!("only a query may be sent")
            };
            let mut observation = evidence(&query, &request);
            if mode == "absent" {
                let RecoveryAuditObservation::Observed { snapshot } = &mut observation else {
                    panic!()
                };
                snapshot.receipt = None;
            }
            if mode == "storage" {
                observation = RecoveryAuditObservation::Unavailable {
                    reason: BindingUnavailable::Storage,
                };
            }
            if mode == "connection" {
                let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
                control.install("machine-test".into(), "audit-wire".into(), false, 18, tx);
            }
            control.record_remote(
                &connection,
                MachineEvent::TelemetryRecoveryAuditObservation {
                    request_id,
                    observation: Box::new(observation),
                },
            );
            assert!(rx.try_recv().is_err());
        });
        let (status, view) = f
            .request(Method::GET, &path(&before, "machine-recovery-audit"), None)
            .await;
        assert_eq!(
            status,
            if matches!(mode, "recorded" | "absent") {
                StatusCode::OK
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            },
            "{mode}: {view}"
        );
        if status == StatusCode::OK {
            assert_eq!(
                view["operation"],
                serde_json::to_value(OperationView::new(&before).unwrap()).unwrap()
            );
            assert_eq!(view["recovery"].is_null(), mode == "absent");
            if mode == "recorded" {
                assert_eq!(
                    view["recovery"]["receipt"]["operation_digest"],
                    view["recovery"]["before"]["operation_digest"]
                );
            }
        }
        task.await.unwrap();
        assert_eq!(f.state.ledger().await.unwrap(), saved);
        assert!(f.state.recovery_plans.0.lock().is_empty());
        f.stop().await;
    }
}

#[tokio::test]
async fn audit_http_does_not_reveal_any_history_without_current_operator_access() {
    let f = Fixture::with_auth(false, true).await;
    let before = pending(&f).await;
    for suffix in [
        "machine-recovery-audit",
        "machine-recoveries/nonexistent-resolution",
    ] {
        assert_eq!(
            f.request(Method::GET, &path(&before, suffix), None).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert!(f.state.recovery_plans.0.lock().is_empty());
    f.stop().await;
}

async fn end_access(state: &ApiState, mode: &str) {
    let now = chrono::Utc::now().timestamp_millis();
    let store = state.store.as_ref().unwrap();
    match mode {
        "logout" => {
            store
                .revoke_user_session_for_user(
                    &"c".repeat(32),
                    "audit-session-fixture",
                    "logout",
                    now,
                )
                .await
                .unwrap();
        }
        "role" => state.hub.set_setting(
            crate::admin::PERMISSIONS_SETTING.into(),
            serde_json::json!({"default_role":"viewer","grants":[]}),
        ),
        _ => {
            store
                .set_user_disabled_at(&"c".repeat(32), Some(now))
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn audit_read_rechecks_actual_cookie_logout_role_and_disable_before_and_after_machine_query()
{
    use crate::store::{ProductUser, ProductUserSession};
    use std::sync::atomic::{AtomicUsize, Ordering};
    for (mode, after_query) in ["logout", "role", "disabled"]
        .into_iter()
        .flat_map(|m| [false, true].map(move |after| (m, after)))
    {
        let f = Fixture::with_auth(false, true).await;
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
            serde_json::json!({"default_role":"operator","grants":[]}),
        );
        store
            .insert_user_session(&ProductUserSession {
                token_hash: crate::admin::hex_sha256(b"audit-cookie"),
                session_id: "audit-session-fixture".into(),
                user_id: user.id,
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
            })
            .await
            .unwrap();
        let before = pending(&f).await;
        let saved = f.state.ledger().await.unwrap();
        let request = crate::server::telemetry_binding::recovery::request_fixture(
            &before,
            &before.intent.actor,
        );
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let connection = f.state.control.install(
            "machine-test".into(),
            "audit-auth-wire".into(),
            false,
            18,
            tx,
        );
        let state = f.state.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let task = tokio::spawn(async move {
            let MachineCommand::QueryTelemetryRecoveryAudit { request_id, query } =
                rx.recv().await.unwrap()
            else {
                panic!()
            };
            count.fetch_add(1, Ordering::Relaxed);
            end_access(&state, mode).await;
            state.control.record_remote(
                &connection,
                MachineEvent::TelemetryRecoveryAuditObservation {
                    request_id,
                    observation: Box::new(evidence(&query, &request)),
                },
            );
        });
        if !after_query {
            end_access(&f.state, mode).await;
        }
        let response = f
            .client
            .get(format!(
                "{}{}",
                f.base,
                path(&before, "machine-recovery-audit")
            ))
            .header(header::COOKIE, "cowboy_user=audit-cookie")
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if mode == "role" && !after_query {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::UNAUTHORIZED
            },
            "{mode}/{after_query}"
        );
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(calls.load(Ordering::Relaxed), usize::from(after_query));
        if after_query {
            task.await.unwrap();
        } else {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        }
        assert_eq!(f.state.ledger().await.unwrap(), saved);
        assert!(f.state.recovery_plans.0.lock().is_empty());
        f.stop().await;
    }
}

#[cfg(feature = "machine-host")]
mod durable {
    use super::*;
    use crate::machine_plugins::{MachinePluginStore, PluginExecutionScope};
    use crate::machine_protocol::telemetry_binding::BindingChange;
    use crate::machine_protocol::telemetry_recovery::RecoveryResult;
    use crate::telemetry_binding::resolution::{
        ResolutionAction, ResolutionIntent, ResolutionPermit,
    };

    fn open(root: &std::path::Path) -> MachinePluginStore {
        MachinePluginStore::new(
            root,
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn new_controller_discovers_reopened_machine_audit_after_service_resolution_and_later_operation()
     {
        let f = Fixture::new(false).await;
        let root = tempfile::tempdir().unwrap();
        let mut intent = crate::telemetry_binding::fixture("durable-audit-original");
        intent.schema = 2;
        intent.change = BindingChange::Revoke {
            policy_epoch: "1".to_owned().try_into().unwrap(),
        };
        let store = f.state.store.as_ref().unwrap();
        let before = store
            .change_telemetry_binding(&Change::Begin(&intent), &|| true)
            .await
            .unwrap()
            .operation;
        let before =
            crate::server::telemetry_binding::advance(store, &before, Progress::Dispatching)
                .await
                .unwrap();
        let before = crate::server::telemetry_binding::advance(
            store,
            &before,
            Progress::NeedsAttention {
                reason: crate::telemetry_binding::Attention::Uncertain,
                observation: None,
            },
        )
        .await
        .unwrap();
        let request =
            crate::server::telemetry_binding::recovery::request_fixture(&before, &intent.actor);
        let machine = open(root.path());
        machine.enable_binding_writer_for_test();
        machine.interrupt_binding_for_test(&request.step).await;
        drop(machine);
        let machine = open(root.path());
        machine.enable_binding_recovery_for_test();
        let owner = PluginExecutionScope::new(Some("service-test"), "machine-test");
        let RecoveryResult::Observed {
            observation: RecoveryObservation::Observed { snapshot },
        } = machine
            .recover_telemetry_binding(&request, owner.telemetry_recovery(&request).unwrap())
            .await
        else {
            panic!()
        };
        let expected = serde_json::to_value(
            ReceiptView::from_audit(snapshot.receipt.as_ref().unwrap()).unwrap(),
        )
        .unwrap();
        let resolution = ResolutionIntent::new(
            "durable-service-resolution".into(),
            intent.actor.clone(),
            &before,
            ResolutionAction::RecordRejected {
                observation_digest: binding_digest(&serde_json::to_vec(&snapshot.binding).unwrap()),
            },
            chrono::Utc::now().timestamp_millis() + 60_000,
        )
        .unwrap();
        let permit = ResolutionPermit::new(
            resolution.clone(),
            before.clone(),
            Some(snapshot.binding),
            OperationBudget::new(
                resolution.expires_at_ms,
                Duration::from_mins(1),
                TimeSample::now(),
            ),
        )
        .unwrap();
        let terminal = store
            .change_telemetry_binding(&Change::Resolve(&permit), &|| true)
            .await
            .unwrap()
            .operation;
        let mut later = intent.clone();
        later.operation_id.push_str("-later");
        later.expected = Some(request.step.expected.clone());
        store
            .change_telemetry_binding(&Change::Begin(&later), &|| true)
            .await
            .unwrap();
        drop(owner);
        drop(machine);
        let f = f.restart().await;
        let machine = Arc::new(open(root.path()));
        let retained = std::fs::read(
            root.path()
                .join("plugin-operations/telemetry-bindings-v1.json"),
        )
        .unwrap();
        let ledger = f.state.ledger().await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let connection = f.state.control.install(
            "machine-test".into(),
            "reopened-audit".into(),
            false,
            18,
            tx,
        );
        let control = f.state.control.clone();
        let task = tokio::spawn(async move {
            for _ in 0..3 {
                let MachineCommand::QueryTelemetryRecoveryAudit { request_id, query } =
                    rx.recv().await.unwrap()
                else {
                    panic!("read must never become a mutation")
                };
                let observation = machine
                    .telemetry_recovery_audit(&query, Some("service-test"), "machine-test")
                    .await;
                control.record_remote(
                    &connection,
                    MachineEvent::TelemetryRecoveryAuditObservation {
                        request_id,
                        observation: Box::new(observation),
                    },
                );
            }
            assert!(rx.try_recv().is_err());
        });
        let (status, view) = f
            .request(Method::GET, &path(&before, "machine-recovery-audit"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(view["recovery"]["receipt"], expected);
        assert_eq!(
            view["recovery"]["before"],
            serde_json::to_value(OperationView::new(&before).unwrap()).unwrap()
        );
        assert_eq!(
            view["operation"],
            serde_json::to_value(OperationView::new(&terminal).unwrap()).unwrap()
        );
        let (status, view) = f
            .request(
                Method::GET,
                &path(
                    &before,
                    &format!("machine-recoveries/{}", request.resolution_id),
                ),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(view, expected);
        assert_eq!(
            f.request(
                Method::GET,
                &path(&before, "machine-recoveries/another-resolution"),
                None
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        task.await.unwrap();
        assert_eq!(f.state.ledger().await.unwrap(), ledger);
        assert_eq!(
            std::fs::read(
                root.path()
                    .join("plugin-operations/telemetry-bindings-v1.json")
            )
            .unwrap(),
            retained
        );
        assert!(f.state.recovery_plans.0.lock().is_empty());
        assert!(
            !LegacyFence::recover(f.state.store.as_ref(), "service-test")
                .await
                .unwrap()
                .allows_legacy()
        );
        f.stop().await;
    }
}
