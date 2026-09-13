use super::*;
use crate::machine_protocol::MachineEvent;
use crate::machine_protocol::telemetry_recovery::{fixture, observed_fixture};
use tokio::sync::mpsc;

#[tokio::test]
async fn recovery_audit_requires_protocol_eighteen_and_never_falls_back_to_recovery() {
    let query = RecoveryAuditQuery {
        schema: 1,
        step: fixture().step,
    };
    for protocol in 1..18 {
        let control = MachineControl::default();
        let (connection, mut commands) = connect(&control, protocol);
        assert_eq!(
            control
                .telemetry_recovery_audit(&connection, &query)
                .await
                .unwrap_err()
                .certainty,
            CommandFailure::NotSent
        );
        assert!(commands.try_recv().is_err());
        assert!(control.live.read().pending.is_empty());
    }
    let control = MachineControl::default();
    let (connection, mut commands) = connect(&control, 18);
    let mut foreign = query;
    foreign.step.machine_id = "another-machine".into();
    assert_eq!(
        control
            .telemetry_recovery_audit(&connection, &foreign)
            .await
            .unwrap_err()
            .certainty,
        CommandFailure::NotSent
    );
    assert!(commands.try_recv().is_err());
}

#[tokio::test]
async fn recovery_audit_has_distinct_reply_correlation_and_discards_late_or_changed_evidence() {
    use crate::machine_protocol::telemetry_recovery_audit::RecoveryAuditSnapshot;
    for boundary in ["exact", "digest", "reconnect"] {
        let control = MachineControl::default();
        let (connection, mut commands) = connect(&control, 18);
        let request = fixture();
        let query = RecoveryAuditQuery {
            schema: 1,
            step: request.step.clone(),
        };
        let read = control.telemetry_recovery_audit(&connection, &query);
        let reply = async {
            let MachineCommand::QueryTelemetryRecoveryAudit {
                request_id,
                query: received,
            } = commands.recv().await.unwrap()
            else {
                panic!()
            };
            assert_eq!(*received, query);
            control.record_remote(
                &connection,
                MachineEvent::TelemetryRecoveryObservation {
                    request_id: request_id.clone(),
                    observation: Box::new(observed_fixture(&request)),
                },
            );
            control.record_remote(
                &connection,
                MachineEvent::TelemetryBindingRecovered {
                    request_id: request_id.clone(),
                    result: Box::new(RecoveryResult::Observed {
                        observation: observed_fixture(&request),
                    }),
                },
            );
            assert_eq!(control.live.read().pending.len(), 1);
            let RecoveryObservation::Observed { snapshot } = observed_fixture(&request) else {
                panic!()
            };
            let observation = RecoveryAuditObservation::Observed {
                snapshot: Box::new(RecoveryAuditSnapshot {
                    query_digest: if boundary == "digest" {
                        crate::machine_protocol::telemetry_binding::binding_digest(b"wrong query")
                    } else {
                        query.digest().unwrap()
                    },
                    receipt: snapshot.receipt,
                    binding: snapshot.binding,
                }),
            };
            let _replacement = (boundary == "reconnect").then(|| connect(&control, 18));
            control.record_remote(
                &connection,
                MachineEvent::TelemetryRecoveryAuditObservation {
                    request_id,
                    observation: Box::new(observation),
                },
            );
        };
        let (result, ()) = tokio::join!(read, reply);
        assert_eq!(result.is_ok(), boundary == "exact");
        assert!(control.live.read().pending.is_empty());
        assert!(commands.try_recv().is_err());
        assert!(!control.events("machine-test").iter().any(|event| matches!(
            event,
            MachineEvent::TelemetryRecoveryAuditObservation { .. }
        )));
    }
}

#[tokio::test]
async fn ending_an_audit_observer_removes_only_its_waiter_without_sending_another_command() {
    let control = MachineControl::default();
    let (connection, mut commands) = connect(&control, 18);
    let query = RecoveryAuditQuery {
        schema: 1,
        step: fixture().step,
    };
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(10),
            control.telemetry_recovery_audit(&connection, &query)
        )
        .await
        .is_err()
    );
    assert!(matches!(
        commands.try_recv().unwrap(),
        MachineCommand::QueryTelemetryRecoveryAudit { .. }
    ));
    assert!(commands.try_recv().is_err());
    assert!(control.live.read().pending.is_empty());
}

fn connect(
    control: &MachineControl,
    protocol: u16,
) -> (ConnectionToken, mpsc::UnboundedReceiver<MachineCommand>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (
        control.install(
            "machine-test".into(),
            "same-epoch".into(),
            false,
            protocol,
            tx,
        ),
        rx,
    )
}

#[tokio::test]
async fn recovery_rejects_all_old_protocols_foreign_targets_and_never_falls_back() {
    let request = fixture();
    for protocol in 1..17 {
        let control = MachineControl::default();
        let (connection, mut commands) = connect(&control, protocol);
        assert_eq!(
            control
                .recover_telemetry_binding(&connection, &request)
                .await
                .unwrap_err()
                .certainty,
            CommandFailure::NotSent
        );
        assert_eq!(
            control
                .telemetry_recovery_observation(&connection, &request)
                .await
                .unwrap_err()
                .certainty,
            CommandFailure::NotSent
        );
        assert!(commands.try_recv().is_err());
        assert!(control.live.read().pending.is_empty());
    }
    let control = MachineControl::default();
    let (connection, mut commands) = connect(&control, 17);
    let mut foreign = request;
    foreign.step.machine_id = "another-machine".into();
    assert!(!control.telemetry_recovery_target_current(&connection, &foreign));
    assert_eq!(
        control
            .recover_telemetry_binding(&connection, &foreign)
            .await
            .unwrap_err()
            .certainty,
        CommandFailure::NotSent
    );
    assert!(commands.try_recv().is_err());
}

#[tokio::test]
async fn recovery_reply_kind_and_complete_digest_are_distinct_and_not_in_event_history() {
    let control = MachineControl::default();
    let (connection, mut commands) = connect(&control, 17);
    let request = fixture();
    for forged in [false, true] {
        let commit = control.recover_telemetry_binding(&connection, &request);
        let reply = async {
            let MachineCommand::RecoverTelemetryBinding {
                request_id,
                recovery,
            } = commands.recv().await.unwrap()
            else {
                panic!()
            };
            assert_eq!(*recovery, request);
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
                MachineEvent::TelemetryRecoveryObservation {
                    request_id: request_id.clone(),
                    observation: Box::new(observed_fixture(&request)),
                },
            );
            assert_eq!(control.live.read().pending.len(), 1);
            let mut changed = request.clone();
            if forged {
                changed.resolution_id.push('x');
            }
            control.record_remote(
                &connection,
                MachineEvent::TelemetryBindingRecovered {
                    request_id,
                    result: Box::new(RecoveryResult::Observed {
                        observation: observed_fixture(&changed),
                    }),
                },
            );
        };
        let (result, ()) = tokio::join!(commit, reply);
        assert_eq!(result.is_ok(), !forged);
        assert!(control.live.read().pending.is_empty());
    }
    let query = control.telemetry_recovery_observation(&connection, &request);
    let reply = async {
        let MachineCommand::QueryTelemetryRecovery {
            request_id,
            recovery,
        } = commands.recv().await.unwrap()
        else {
            panic!()
        };
        assert_eq!(*recovery, request);
        control.record_remote(
            &connection,
            MachineEvent::TelemetryBindingRecovered {
                request_id: request_id.clone(),
                result: Box::new(RecoveryResult::Observed {
                    observation: observed_fixture(&request),
                }),
            },
        );
        assert_eq!(control.live.read().pending.len(), 1);
        control.record_remote(
            &connection,
            MachineEvent::TelemetryRecoveryObservation {
                request_id,
                observation: Box::new(observed_fixture(&request)),
            },
        );
    };
    let (result, ()) = tokio::join!(query, reply);
    assert!(result.unwrap().matches(&request));
    assert!(!control.events("machine-test").iter().any(|event| matches!(
        event,
        MachineEvent::TelemetryBindingRecovered { .. }
            | MachineEvent::TelemetryRecoveryObservation { .. }
    )));
}

#[tokio::test]
async fn storage_failure_is_uncertain_and_same_epoch_replacement_cannot_accept_late_recovery() {
    for replace in [false, true] {
        let control = MachineControl::default();
        let (connection, mut commands) = connect(&control, 17);
        let request = fixture();
        let commit = control.recover_telemetry_binding(&connection, &request);
        let reply = async {
            let MachineCommand::RecoverTelemetryBinding { request_id, .. } =
                commands.recv().await.unwrap()
            else {
                panic!()
            };
            let _replacement = replace.then(|| connect(&control, 17));
            control.record_remote(&connection, MachineEvent::TelemetryBindingRecovered { request_id, result: Box::new(RecoveryResult::Unavailable {
                failure: crate::machine_protocol::telemetry_binding::BindingCommitFailure::Unavailable(crate::machine_protocol::telemetry_binding::BindingUnavailable::Storage),
            }) });
        };
        let (result, ()) = tokio::join!(commit, reply);
        assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown);
        assert!(control.live.read().pending.is_empty());
        assert!(commands.try_recv().is_err());
        if replace {
            assert!(!control.telemetry_recovery_target_current(&connection, &request));
        }
    }
}
