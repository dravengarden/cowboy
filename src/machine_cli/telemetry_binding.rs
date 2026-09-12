//! Actual finite command admission, shared by the socket dispatcher and tests.

use crate::machine_plugins::{MachinePluginStore, PluginExecutionScope};
use crate::machine_protocol::MachineEvent;
use crate::machine_protocol::telemetry_binding::{
    BindingCommitFailure, BindingCommitResult, BindingStep, BindingUnavailable,
};
use std::sync::Arc;

pub(crate) fn commit(
    request_id: String,
    step: BindingStep,
    store: Arc<MachinePluginStore>,
    execution: &PluginExecutionScope,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
) {
    let lease = step
        .validate_commit()
        .map_err(|_| BindingUnavailable::InvalidRequest)
        .and_then(|()| execution.telemetry_binding(&step));
    let lease = match lease {
        Ok(lease) => lease,
        Err(reason) => {
            let _ = events.send(MachineEvent::TelemetryBindingCommitted {
                request_id,
                result: Box::new(BindingCommitResult::Unavailable {
                    failure: BindingCommitFailure::Unavailable(reason),
                }),
            });
            return;
        }
    };
    // The scope, deadline and full request have already been captured. A later
    // connection, deadline or queue wait cannot mint a replacement lease.
    tokio::spawn(async move {
        let result = store.commit_telemetry_binding_command(&step, lease).await;
        let _ = events.send(MachineEvent::TelemetryBindingCommitted {
            request_id,
            result: Box::new(result),
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::telemetry_binding::{
        BindingChange, BindingRejection, execution_fixture,
    };

    #[tokio::test]
    async fn cli_admission_rejects_old_schema_foreign_owner_and_ended_queued_scope() {
        for boundary in ["schema", "owner", "disconnect"] {
            let root = tempfile::tempdir().unwrap();
            let store = Arc::new(
                MachinePluginStore::new(
                    root.path(),
                    crate::machine_protocol::Platform::Linux,
                    "x86_64".into(),
                )
                .unwrap(),
            );
            store.enable_binding_writer_for_test();
            let mut step = execution_fixture();
            step.change = BindingChange::Revoke {
                policy_epoch: "1".to_owned().try_into().unwrap(),
            };
            if boundary == "schema" {
                step.schema = 1;
                step.expected_namespace = None;
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
            commit("finite-rpc".into(), step, store, &scope, events);
            drop(scope); // spawned work has not run on this single-thread runtime
            let MachineEvent::TelemetryBindingCommitted { result, .. } = rx.recv().await.unwrap()
            else {
                panic!()
            };
            let expected = match boundary {
                "schema" => BindingCommitFailure::Unavailable(BindingUnavailable::InvalidRequest),
                "owner" => BindingCommitFailure::Unavailable(BindingUnavailable::WrongOwner),
                "disconnect" => {
                    BindingCommitFailure::Rejected(BindingRejection::AuthorizationEnded)
                }
                _ => unreachable!(),
            };
            assert_eq!(
                *result,
                BindingCommitResult::Unavailable { failure: expected }
            );
            assert!(
                !root
                    .path()
                    .join("plugin-operations/telemetry-bindings-v1.json")
                    .exists()
            );
        }
    }
}
