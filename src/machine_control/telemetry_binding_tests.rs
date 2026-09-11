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
