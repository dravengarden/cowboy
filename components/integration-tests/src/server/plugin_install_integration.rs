use crate::machine_control::{CommandFailure, MachineControl};
use crate::machine_protocol::plugin_install::{
    InstallLookup, InstallObservation, InstallOutcome, InstallReceipt, InstallStep,
};
use crate::server::plugin_install::dispatch;

fn applied() -> InstallOutcome {
    InstallOutcome::Applied {
        revision: format!("installation-{}", "d".repeat(64))
            .try_into()
            .unwrap(),
    }
}

fn machine_receipt(step: &InstallStep, outcome: InstallOutcome) -> InstallReceipt {
    InstallReceipt {
        step: step.clone(),
        request_digest: step.request_digest().unwrap(),
        outcome,
    }
}

#[cfg(feature = "machine-host")]
#[tokio::test]
async fn transport_never_moves_installation_to_a_replacement_connection_or_retries_a_lost_ack() {
    use crate::machine_protocol::{MachineCommand, MachineEvent};
    let root = tempfile::tempdir().unwrap();
    let publisher = crate::machine_auth::MachineIdentity::load_or_create(root.path()).unwrap();
    let desired = crate::machine_plugins::telemetry_release_for_test(&publisher, "1.1.0");
    let mut step = crate::machine_protocol::plugin_install::fixture();
    step.plugin_version
        .clone_from(&desired.release.plugin_version);
    step.generation_digest
        .clone_from(&desired.release.artifact_digest);
    step.contract_fingerprint
        .clone_from(&desired.release.contract_fingerprint);
    step.envelope_digest =
        crate::machine_protocol::plugin_step::digest(&serde_json::to_vec(&desired).unwrap());
    for change in ["before", "lost", "rejected", "applied"] {
        let control = MachineControl::default();
        let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
        let original = control.install("machine-test".into(), "same-epoch".into(), false, 19, tx);
        let (replacement, mut other_commands) = tokio::sync::mpsc::unbounded_channel();
        if change == "before" {
            control.install(
                "machine-test".into(),
                "same-epoch".into(),
                false,
                19,
                replacement,
            );
            assert_eq!(
                dispatch(&control, &original, &desired, &step)
                    .await
                    .unwrap_err()
                    .certainty,
                CommandFailure::NotSent
            );
        } else {
            let (result, ()) = tokio::join!(
                dispatch(&control, &original, &desired, &step),
                async {
                    let MachineCommand::InstallPluginStep {
                        request_id,
                        plugin,
                        step: sent,
                    } = commands.recv().await.unwrap()
                    else {
                        panic!("only an exact install may be sent");
                    };
                    assert_eq!(*plugin, desired);
                    assert_eq!(*sent, step);
                    assert_eq!(request_id, format!("plugin-install-{}", step.operation_id));
                    if change == "lost" {
                        control.install(
                            "machine-test".into(),
                            "same-epoch".into(),
                            false,
                            19,
                            replacement,
                        );
                    }
                    // Even a late successful reply from the old connection cannot
                    // certify the result after its incarnation has been replaced.
                    control.record_remote(
                        &original,
                        MachineEvent::PluginInstallationStep { request_id, observation: Box::new(InstallObservation { admission_enabled: false, result: InstallLookup::Found { receipt: Box::new(machine_receipt(&step, if change == "rejected" { InstallOutcome::Rejected { reason: crate::machine_protocol::plugin_install::InstallRejection::TargetChanged } } else { applied() })) } }) },
                    );
                }
            );
            match change {
                "lost" => assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown),
                _ => assert!(matches!(
                    result.unwrap().result,
                    InstallLookup::Found { .. }
                )),
            }
        }
        assert!(commands.try_recv().is_err());
        assert!(
            other_commands.try_recv().is_err(),
            "no rebind, retry or inverse"
        );
    }
}
