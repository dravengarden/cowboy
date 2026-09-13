//! Capture this new purpose's connection and deadline BEFORE detached work.
use crate::machine_plugins::{MachinePluginStore, PluginExecutionScope};
use crate::machine_protocol::telemetry_recovery::{RecoveryRequest, RecoveryResult};
use crate::machine_protocol::{MachineEvent, telemetry_binding::BindingCommitFailure};
use std::sync::Arc;

pub(crate) fn recover(
    request_id: String,
    request: RecoveryRequest,
    store: Arc<MachinePluginStore>,
    execution: &PluginExecutionScope,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
) {
    let lease = match execution.telemetry_recovery(&request) {
        Ok(lease) => lease,
        Err(reason) => {
            let _ = events.send(MachineEvent::TelemetryBindingRecovered {
                request_id,
                result: Box::new(RecoveryResult::Unavailable {
                    failure: BindingCommitFailure::Unavailable(reason),
                }),
            });
            return;
        }
    };
    tokio::spawn(async move {
        let result = store.recover_telemetry_binding(&request, lease).await;
        let _ = events.send(MachineEvent::TelemetryBindingRecovered {
            request_id,
            result: Box::new(result),
        });
    });
}

pub(crate) fn query(
    request_id: String,
    request: RecoveryRequest,
    store: Arc<MachinePluginStore>,
    service: Option<String>,
    machine: String,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
) {
    tokio::spawn(async move {
        let observation = store
            .telemetry_recovery_observation(&request, service.as_deref(), &machine)
            .await;
        let _ = events.send(MachineEvent::TelemetryRecoveryObservation {
            request_id,
            observation: Box::new(observation),
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::telemetry_binding::{BindingRejection, BindingUnavailable};
    use crate::machine_protocol::telemetry_recovery::fixture;

    #[tokio::test]
    async fn recovery_cli_rejects_legacy_owner_expiry_and_disconnected_queued_work() {
        for boundary in ["schema", "owner", "expired", "disconnect"] {
            let root = tempfile::tempdir().unwrap();
            let store = Arc::new(
                MachinePluginStore::new(
                    root.path(),
                    crate::machine_protocol::Platform::Linux,
                    "x86_64".into(),
                )
                .unwrap(),
            );
            store.enable_binding_recovery_for_test();
            let mut request = fixture();
            if boundary == "schema" {
                request.step.schema = 1;
                request.step.expected_namespace = None;
            }
            if boundary == "expired" {
                request.expires_at_ms = 1;
            }
            let scope = PluginExecutionScope::new(
                Some(if boundary == "owner" {
                    "foreign-service"
                } else {
                    "service-test"
                }),
                "machine-test",
            );
            let (events, mut rx) = tokio::sync::mpsc::unbounded_channel();
            recover("recovery-rpc".into(), request, store, &scope, events);
            if boundary == "disconnect" {
                drop(scope);
            }
            let MachineEvent::TelemetryBindingRecovered { request_id, result } =
                rx.recv().await.unwrap()
            else {
                panic!()
            };
            assert_eq!(request_id, "recovery-rpc");
            let failure = match boundary {
                "schema" => BindingCommitFailure::Unavailable(BindingUnavailable::InvalidRequest),
                "owner" => BindingCommitFailure::Unavailable(BindingUnavailable::WrongOwner),
                "expired" => BindingCommitFailure::Rejected(BindingRejection::Expired),
                _ => BindingCommitFailure::Rejected(BindingRejection::AuthorizationEnded),
            };
            assert_eq!(*result, RecoveryResult::Unavailable { failure });
            assert!(
                !root
                    .path()
                    .join("plugin-operations/telemetry-bindings-v1.json")
                    .exists()
            );
        }
    }
}
