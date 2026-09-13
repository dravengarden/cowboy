use super::*;
use crate::server::telemetry_binding::recovery::request_fixture;
use crate::telemetry_binding::{Attention, Progress, writer::Change};

#[tokio::test]
async fn machine_recovery_binds_fresh_actor_original_credential_full_operation_and_budget() {
    for boundary in [
        "none",
        "logout",
        "role",
        "disabled",
        "actor",
        "operation",
        "step",
        "deadline",
        "queued",
        "retained",
        "preview_expired",
    ] {
        let h = Harness::new().await;
        let (headers, verified) = h.cookie().await;
        let mut approval =
            OperatorApproval::capture(h.context(), "service-test", Some(&verified), &headers)
                .unwrap();
        let mut intent = crate::telemetry_binding::fixture("machine-recovery-auth");
        intent.schema = 2;
        let before = h
            .store
            .change_telemetry_binding(&Change::Begin(&intent), &|| true)
            .await
            .unwrap()
            .operation;
        let before = h
            .store
            .change_telemetry_binding(
                &Change::Advance {
                    expected: &before,
                    progress: Progress::NeedsAttention {
                        reason: Attention::Uncertain,
                        observation: None,
                    },
                },
                &|| true,
            )
            .await
            .unwrap()
            .operation;
        let mut captured = before.clone();
        if boundary == "retained" {
            captured.intent.expires_at_ms += 1;
        }
        let request = request_fixture(&captured, approval.actor());
        assert_ne!(request.actor, (&before.intent.actor).into());
        if boundary == "queued" {
            approval.received = TimeSample::for_test(
                std::time::Instant::now() - Duration::from_secs(61),
                auth_now_ms(),
            );
        }
        let preview = OperationBudget::new(
            request.expires_at_ms,
            Duration::from_mins(1),
            TimeSample::now(),
        );
        if boundary == "preview_expired" {
            preview.expire_for_test();
        }
        let authority = approval
            .bind_telemetry_recovery(&request, &captured)
            .unwrap()
            .constrain_to_preview(preview);
        let mut changed = request.clone();
        match boundary {
            "logout" => {
                h.store
                    .revoke_user_session_for_user(
                        &h.user.id,
                        "session-approval",
                        "logout",
                        auth_now_ms(),
                    )
                    .await
                    .unwrap();
            }
            "role" => h.role(AdminRole::Viewer),
            "disabled" => h
                .store
                .set_user_disabled_at(&h.user.id, Some(auth_now_ms()))
                .await
                .unwrap(),
            "actor" => {
                changed.actor = crate::machine_protocol::telemetry_recovery::RecoveryActor::Admin {
                    account: "other".into(),
                }
            }
            "operation" => {
                changed.service_operation_digest =
                    crate::machine_protocol::telemetry_binding::binding_digest(b"changed")
            }
            "step" => changed.step.expires_at_ms += 1,
            "deadline" => changed.expires_at_ms += 1,
            _ => {}
        }
        assert_eq!(
            authority.check(h.context(), &h.store, &changed).await,
            boundary == "none",
            "{boundary}"
        );
        h.role(AdminRole::Operator);
        h.store
            .set_user_disabled_at(&h.user.id, None)
            .await
            .unwrap();
        assert_eq!(
            authority.check(h.context(), &h.store, &request).await,
            boundary == "none",
            "repair cannot renew {boundary}"
        );
        assert_eq!(
            h.store
                .telemetry_binding_ledger("service-test")
                .await
                .unwrap()
                .unwrap()
                .operations
                .last(),
            Some(&before)
        );
    }
}

#[tokio::test]
async fn confirmation_cannot_reuse_another_owner_or_an_unresolved_phase_it_did_not_confirm() {
    for boundary in ["actor", "service", "prepared", "step", "operation"] {
        let h = Harness::new().await;
        let (headers, verified) = h.cookie().await;
        let approval =
            OperatorApproval::capture(h.context(), "service-test", Some(&verified), &headers)
                .unwrap();
        let mut intent = crate::telemetry_binding::fixture("machine-recovery-purpose");
        intent.schema = 2;
        let before = crate::telemetry_binding::Operation {
            intent,
            progress: Progress::NeedsAttention {
                reason: Attention::Uncertain,
                observation: None,
            },
        };
        let mut request = request_fixture(&before, approval.actor());
        let mut before = before;
        match boundary {
            "actor" => {
                request.actor = crate::machine_protocol::telemetry_recovery::RecoveryActor::Admin {
                    account: "other".into(),
                }
            }
            "service" => request.step.service_id = "foreign".into(),
            "prepared" => before.progress = Progress::Prepared,
            "step" => before.intent.expires_at_ms += 1,
            _ => {
                request.service_operation_digest =
                    crate::machine_protocol::telemetry_binding::binding_digest(b"different")
            }
        }
        assert!(
            approval.bind_telemetry_recovery(&request, &before).is_err(),
            "{boundary}"
        );
    }
}
