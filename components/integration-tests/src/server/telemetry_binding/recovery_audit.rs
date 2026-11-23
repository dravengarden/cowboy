use crate::machine_protocol::telemetry_binding::binding_digest;
use crate::machine_protocol::telemetry_recovery::RecoveryObservation;
use crate::machine_protocol::{MachineCommand, MachineEvent};
use crate::operation_budget::{OperationBudget, TimeSample};
use crate::server::telemetry_binding::recovery::surface::ReceiptView;
use crate::server::telemetry_binding::resolution::surface::OperationView;
use crate::server::telemetry_binding::test_fixtures::HttpFixture as Fixture;
use crate::telemetry_binding::{LegacyFence, Operation, Progress, writer::Change};
use axum::http::{Method, StatusCode};
use std::sync::Arc;
use std::time::Duration;

fn path(before: &Operation, suffix: &str) -> String {
    format!(
        "/api/telemetry/binding/operations/{}/{suffix}",
        before.intent.operation_id
    )
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
