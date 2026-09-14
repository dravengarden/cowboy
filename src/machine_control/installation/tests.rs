use super::*;
use crate::machine_protocol::MachineEvent;
use crate::machine_protocol::plugin_install::{InstallUnavailable, fixture};
use tokio::sync::mpsc;

#[tokio::test]
async fn old_protocol_and_foreign_machine_requests_are_not_sent() {
    let control = MachineControl::default();
    let (tx, mut commands) = mpsc::unbounded_channel();
    let token = control.install("machine-test".into(), "epoch".into(), false, 18, tx);
    let step = fixture();
    assert_eq!(
        control
            .plugin_installation_step(&token, &step, None)
            .await
            .unwrap_err()
            .certainty,
        CommandFailure::NotSent
    );
    assert_eq!(
        control
            .plugin_installation_target(&token, &step.target_query())
            .await
            .unwrap_err()
            .certainty,
        CommandFailure::NotSent
    );
    assert!(commands.try_recv().is_err());
    assert!(control.live.read().pending.is_empty());
    let (tx, mut commands) = mpsc::unbounded_channel();
    let token = control.install("other-machine".into(), "epoch".into(), false, 19, tx);
    assert_eq!(
        control
            .plugin_installation_step(&token, &step, None)
            .await
            .unwrap_err()
            .certainty,
        CommandFailure::NotSent
    );
    assert!(commands.try_recv().is_err());
}

#[tokio::test]
async fn query_replies_use_exact_kind_and_never_enter_event_history() {
    let control = MachineControl::default();
    let (tx, mut commands) = mpsc::unbounded_channel();
    let token = control.install("machine-test".into(), "epoch".into(), false, 19, tx);
    let step = fixture();
    let send = async {
        let MachineCommand::QueryPluginInstallStep { request_id, .. } =
            commands.recv().await.unwrap()
        else {
            panic!();
        };
        control.record_remote(
            &token,
            MachineEvent::PluginInstallationTarget {
                request_id: request_id.clone(),
                observation: Box::new(InstallTargetObservation::Unavailable {
                    reason: InstallUnavailable::ReaderOnly,
                }),
            },
        );
        assert!(control.live.read().pending.contains_key(&request_id));
        control.record_remote(
            &token,
            MachineEvent::PluginInstallationStep {
                request_id,
                observation: Box::new(InstallObservation {
                    admission_enabled: false,
                    result: InstallLookup::Unavailable {
                        reason: InstallUnavailable::ReaderOnly,
                    },
                }),
            },
        );
    };
    let (result, ()) = tokio::join!(control.plugin_installation_step(&token, &step, None), send);
    assert_eq!(
        result.unwrap().result,
        InstallLookup::Unavailable {
            reason: InstallUnavailable::ReaderOnly
        }
    );
    assert!(control.live.read().events.values().all(Vec::is_empty));
}

#[tokio::test]
async fn an_ack_for_changed_input_is_unknown_not_success_and_never_retried() {
    let control = MachineControl::default();
    let (tx, mut commands) = mpsc::unbounded_channel();
    let token = control.install("machine-test".into(), "epoch".into(), false, 19, tx);
    let step = fixture();
    let send = async {
        let MachineCommand::QueryPluginInstallStep {
            request_id,
            mut step,
        } = commands.recv().await.unwrap()
        else {
            panic!();
        };
        step.plan_digest = crate::machine_protocol::plugin_step::digest(b"another actor");
        let receipt = crate::machine_protocol::plugin_install::InstallReceipt {
            request_digest: step.request_digest().unwrap(),
            step: *step,
            outcome: crate::machine_protocol::plugin_install::InstallOutcome::Rejected {
                reason: crate::machine_protocol::plugin_install::InstallRejection::Expired,
            },
        };
        control.record_remote(
            &token,
            MachineEvent::PluginInstallationStep {
                request_id,
                observation: Box::new(InstallObservation {
                    admission_enabled: false,
                    result: InstallLookup::Found {
                        receipt: Box::new(receipt),
                    },
                }),
            },
        );
    };
    let (result, ()) = tokio::join!(control.plugin_installation_step(&token, &step, None), send);
    assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown);
    assert!(commands.try_recv().is_err());
    assert!(control.live.read().pending.is_empty());
}
