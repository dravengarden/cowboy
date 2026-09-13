use super::*;
use crate::machine_protocol::telemetry_binding::{
    BindingObservationSnapshot, BindingUnavailable, binding_digest, fixture,
};

fn connect(
    control: &MachineControl,
    protocol: u16,
) -> (ConnectionToken, mpsc::UnboundedReceiver<MachineCommand>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (
        control.install(
            "machine-test".into(),
            "reused-epoch".into(),
            false,
            protocol,
            tx,
        ),
        rx,
    )
}

fn missing(step: &BindingStep) -> BindingObservation {
    BindingObservation::Observed {
        snapshot: Box::new(BindingObservationSnapshot {
            request_digest: step.request_digest().unwrap(),
            receipt: None,
            current: None,
            unresolved: false,
        }),
    }
}

#[tokio::test]
async fn telemetry_binding_commit_requires_protocol_fifteen_and_namespace_cas() {
    use crate::machine_protocol::telemetry_binding::execution_fixture;
    for protocol in 1..15 {
        let control = MachineControl::default();
        let (connection, mut commands) = connect(&control, protocol);
        for step in [fixture(), execution_fixture()] {
            assert_eq!(
                control
                    .commit_telemetry_binding(&connection, &step)
                    .await
                    .unwrap_err()
                    .certainty,
                CommandFailure::NotSent
            );
        }
        assert_eq!(
            control
                .telemetry_binding_observation(&connection, &execution_fixture())
                .await
                .unwrap_err()
                .certainty,
            CommandFailure::NotSent
        );
        assert!(commands.try_recv().is_err());
        assert!(control.live.read().pending.is_empty());
    }
    let control = MachineControl::default();
    let (connection, mut commands) = connect(&control, 15);
    assert_eq!(
        control
            .commit_telemetry_binding(&connection, &fixture())
            .await
            .unwrap_err()
            .certainty,
        CommandFailure::NotSent
    );
    assert_eq!(
        control
            .commit_telemetry_binding(&connection, &execution_fixture())
            .await
            .unwrap_err()
            .certainty,
        CommandFailure::NotSent,
        "selection requires the exact active installation at enqueue"
    );
    assert!(commands.try_recv().is_err());
}

#[tokio::test]
async fn telemetry_binding_commit_replies_cannot_substitute_query_or_command_results() {
    use crate::machine_protocol::telemetry_binding::{
        BindingChange, BindingCommitResult, execution_fixture,
    };
    let control = MachineControl::default();
    let (connection, mut commands) = connect(&control, 15);
    let mut step = execution_fixture();
    step.change = BindingChange::Revoke {
        policy_epoch: "1".to_owned().try_into().unwrap(),
    };
    for forged in [false, true] {
        let commit = control.commit_telemetry_binding(&connection, &step);
        let reply = async {
            let MachineCommand::CommitTelemetryBinding {
                request_id,
                step: received,
            } = commands.recv().await.unwrap()
            else {
                panic!()
            };
            assert_eq!(*received, step);
            control.record_remote(
                &connection,
                MachineEvent::CommandResult {
                    request_id: request_id.clone(),
                    accepted: true,
                    detail: None,
                },
            );
            control.record_remote(
                &connection,
                MachineEvent::TelemetryBindingObservation {
                    request_id: request_id.clone(),
                    observation: Box::new(missing(&step)),
                },
            );
            assert_eq!(control.live.read().pending.len(), 1);
            let mut observation = missing(&step);
            if forged && let BindingObservation::Observed { snapshot } = &mut observation {
                snapshot.request_digest = binding_digest(b"not this original intent");
            }
            control.record_remote(
                &connection,
                MachineEvent::TelemetryBindingCommitted {
                    request_id,
                    result: Box::new(BindingCommitResult::Observed { observation }),
                },
            );
        };
        let (result, ()) = tokio::join!(commit, reply);
        assert_eq!(result.is_ok(), !forged);
        assert!(control.live.read().pending.is_empty());
        assert!(
            !control
                .events("machine-test")
                .iter()
                .any(|event| matches!(event, MachineEvent::TelemetryBindingCommitted { .. }))
        );
    }
}

#[tokio::test]
async fn telemetry_binding_commit_late_reply_never_crosses_a_connection_incarnation() {
    use crate::machine_protocol::telemetry_binding::{
        BindingChange, BindingCommitResult, execution_fixture,
    };
    let control = MachineControl::default();
    let (connection, mut commands) = connect(&control, 15);
    let mut step = execution_fixture();
    step.change = BindingChange::Revoke {
        policy_epoch: "1".to_owned().try_into().unwrap(),
    };
    let commit = control.commit_telemetry_binding(&connection, &step);
    let replace = async {
        let MachineCommand::CommitTelemetryBinding { request_id, .. } =
            commands.recv().await.unwrap()
        else {
            panic!()
        };
        let (_replacement, _commands) = connect(&control, 15);
        control.record_remote(
            &connection,
            MachineEvent::TelemetryBindingCommitted {
                request_id,
                result: Box::new(BindingCommitResult::Observed {
                    observation: missing(&step),
                }),
            },
        );
    };
    let (result, ()) = tokio::join!(commit, replace);
    assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown);
    assert!(control.live.read().pending.is_empty());
    assert_eq!(
        control
            .commit_telemetry_binding(&connection, &step)
            .await
            .unwrap_err()
            .certainty,
        CommandFailure::NotSent
    );
    assert!(control.events("machine-test").is_empty());
}

#[tokio::test]
async fn telemetry_binding_commit_storage_failure_is_not_proof_of_no_effect() {
    use crate::machine_protocol::telemetry_binding::{
        BindingChange, BindingCommitFailure, BindingCommitResult, execution_fixture,
    };
    let control = MachineControl::default();
    let (connection, mut commands) = connect(&control, 15);
    let mut step = execution_fixture();
    step.change = BindingChange::Revoke {
        policy_epoch: "1".to_owned().try_into().unwrap(),
    };
    let commit = control.commit_telemetry_binding(&connection, &step);
    let reply = async {
        let MachineCommand::CommitTelemetryBinding { request_id, .. } =
            commands.recv().await.unwrap()
        else {
            panic!()
        };
        control.record_remote(
            &connection,
            MachineEvent::TelemetryBindingCommitted {
                request_id,
                result: Box::new(BindingCommitResult::Unavailable {
                    failure: BindingCommitFailure::Unavailable(BindingUnavailable::Storage),
                }),
            },
        );
    };
    let (result, ()) = tokio::join!(commit, reply);
    assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown);
}

#[tokio::test]
async fn telemetry_binding_query_checks_protocol_owner_and_syntax_before_enqueue() {
    for protocol in 1..14 {
        let control = MachineControl::default();
        let (connection, mut commands) = connect(&control, protocol);
        let error = control
            .telemetry_binding_observation(&connection, &fixture())
            .await
            .unwrap_err();
        assert_eq!(error.certainty, CommandFailure::NotSent);
        assert!(commands.try_recv().is_err());
        assert!(control.live.read().pending.is_empty());
    }
    for invalid in 0..3 {
        let control = MachineControl::default();
        let (connection, mut commands) = connect(&control, 14);
        let mut step = fixture();
        match invalid {
            0 => step.machine_id = "other-machine".into(),
            1 => step.schema = 2,
            _ => step.operation_id.clear(),
        }
        assert_eq!(
            control
                .telemetry_binding_observation(&connection, &step)
                .await
                .unwrap_err()
                .certainty,
            CommandFailure::NotSent
        );
        assert!(commands.try_recv().is_err());
    }
}

#[tokio::test]
async fn telemetry_binding_query_keeps_reply_kinds_separate_and_validates_missing_evidence() {
    let control = MachineControl::default();
    let (connection, mut commands) = connect(&control, 14);
    let step = fixture();
    for forged in [false, true] {
        let query = control.telemetry_binding_observation(&connection, &step);
        let respond = async {
            let MachineCommand::QueryTelemetryBinding {
                request_id,
                step: received,
            } = commands.recv().await.unwrap()
            else {
                panic!()
            };
            assert_eq!(*received, step);
            control.record_remote(
                &connection,
                MachineEvent::CommandResult {
                    request_id: request_id.clone(),
                    accepted: true,
                    detail: None,
                },
            );
            control.record_remote(
                &connection,
                MachineEvent::PluginUninstallStep {
                    request_id: request_id.clone(),
                    observation: Box::new(StepObservation {
                        admission_enabled: true,
                        result: StepLookup::NotFound {},
                    }),
                },
            );
            assert_eq!(control.live.read().pending.len(), 1);
            let mut observation = missing(&step);
            if forged {
                let BindingObservation::Observed { snapshot } = &mut observation else {
                    panic!()
                };
                snapshot.request_digest = binding_digest(b"different original request");
            }
            control.record_remote(
                &connection,
                MachineEvent::TelemetryBindingObservation {
                    request_id,
                    observation: Box::new(observation),
                },
            );
        };
        let (result, ()) = tokio::join!(query, respond);
        assert_eq!(result.is_ok(), !forged);
        assert!(control.live.read().pending.is_empty());
        assert!(
            !control
                .events("machine-test")
                .iter()
                .any(|event| matches!(event, MachineEvent::TelemetryBindingObservation { .. }))
        );
    }
}

#[tokio::test]
async fn telemetry_binding_query_connection_replacement_and_cancellation_release_waiters() {
    let control = MachineControl::default();
    let (connection, mut commands) = connect(&control, 14);
    let step = fixture();
    let query = control.telemetry_binding_observation(&connection, &step);
    let replace = async {
        let MachineCommand::QueryTelemetryBinding { request_id, .. } =
            commands.recv().await.unwrap()
        else {
            panic!()
        };
        let (_replacement, _commands) = connect(&control, 14);
        control.record_remote(
            &connection,
            MachineEvent::TelemetryBindingObservation {
                request_id,
                observation: Box::new(missing(&step)),
            },
        );
    };
    let (result, ()) = tokio::join!(query, replace);
    assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown);
    assert!(control.live.read().pending.is_empty());
    let (connection, mut commands) = connect(&control, 14);
    let mut query = Box::pin(control.telemetry_binding_observation(&connection, &step));
    assert!(futures::poll!(&mut query).is_pending());
    let MachineCommand::QueryTelemetryBinding { request_id, .. } = commands.recv().await.unwrap()
    else {
        panic!()
    };
    drop(query);
    assert!(control.live.read().pending.is_empty());
    control.record_remote(
        &connection,
        MachineEvent::TelemetryBindingObservation {
            request_id,
            observation: Box::new(BindingObservation::Unavailable {
                reason: BindingUnavailable::Storage,
            }),
        },
    );
    assert!(control.events("machine-test").is_empty());
}

#[tokio::test]
#[cfg(feature = "machine-host")]
async fn telemetry_binding_query_uses_actual_machine_reader_without_creating_binding_state() {
    let root = tempfile::tempdir().unwrap();
    let store = crate::machine_plugins::MachinePluginStore::new(
        root.path(),
        crate::machine_protocol::Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    let control = MachineControl::default();
    let (connection, mut commands) = connect(&control, 14);
    let step = fixture();
    let query = control.telemetry_binding_observation(&connection, &step);
    let machine = async {
        let wire = serde_json::to_vec(&commands.recv().await.unwrap()).unwrap();
        let MachineCommand::QueryTelemetryBinding { request_id, step } =
            serde_json::from_slice(&wire).unwrap()
        else {
            panic!()
        };
        let observation = store
            .telemetry_binding_observation(&step, Some("service-test"), "machine-test")
            .await;
        let response = MachineEvent::TelemetryBindingObservation {
            request_id,
            observation: Box::new(observation),
        };
        control.record_remote(
            &connection,
            serde_json::from_slice(&serde_json::to_vec(&response).unwrap()).unwrap(),
        );
    };
    let (result, ()) = tokio::join!(query, machine);
    assert_eq!(result.unwrap(), missing(&step));
    assert!(
        !root
            .path()
            .join("plugin-operations/telemetry-bindings-v1.json")
            .exists()
    );
    assert!(!root.path().join("telemetry.json").exists());
    assert!(control.events("machine-test").is_empty());
}
