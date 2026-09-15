use super::*;
use crate::machine_protocol::telemetry_recovery_audit::RecoveryAuditQuery;
use crate::machine_protocol::{DesiredPlugin, plugin_step, telemetry_export, telemetry_recovery};

fn commands(desired: &DesiredPlugin, service: &str, machine: &str) -> Vec<MachineCommand> {
    let mut install = plugin_install::fixture();
    install.service_id = service.into();
    install.machine_id = machine.into();
    install
        .plugin_version
        .clone_from(&desired.release.plugin_version);
    install
        .generation_digest
        .clone_from(&desired.release.artifact_digest);
    install
        .contract_fingerprint
        .clone_from(&desired.release.contract_fingerprint);
    install.envelope_digest = plugin_step::digest(&serde_json::to_vec(desired).unwrap());
    install.validate().unwrap();
    assert!(install.matches_envelope(desired));
    let mut uninstall = plugin_step::fixture();
    uninstall.service_id = service.into();
    uninstall.machine_id = machine.into();
    uninstall.validate().unwrap();
    let mut binding = telemetry_binding::execution_fixture();
    binding.service_id = service.into();
    binding.machine_id = machine.into();
    binding.validate_commit().unwrap();
    let mut recovery = telemetry_recovery::fixture();
    recovery.step = binding.clone();
    recovery.expected_observation_digest = telemetry_binding::binding_digest(
        &serde_json::to_vec(&telemetry_recovery::prepared(&binding).unwrap()).unwrap(),
    );
    recovery.validate().unwrap();
    let mut export = telemetry_export::fixture();
    export.service_id = service.into();
    export.machine_id = machine.into();
    export.validate().unwrap();
    let request_id = "site-fixture".to_owned();
    vec![
        MachineCommand::ObservePluginInstallation {
            request_id: request_id.clone(),
            query: Box::new(install.target_query()),
        },
        MachineCommand::InstallPluginStep {
            request_id: request_id.clone(),
            step: Box::new(install.clone()),
            plugin: Box::new(desired.clone()),
        },
        MachineCommand::QueryPluginInstallStep {
            request_id: request_id.clone(),
            step: Box::new(install),
        },
        MachineCommand::UninstallPluginStep {
            request_id: request_id.clone(),
            step: Box::new(uninstall.clone()),
        },
        MachineCommand::QueryPluginUninstallStep {
            request_id: request_id.clone(),
            step: Box::new(uninstall.clone()),
        },
        MachineCommand::QueryPluginUninstallRecovery {
            request_id: request_id.clone(),
            step: Box::new(uninstall),
        },
        MachineCommand::QueryTelemetryBinding {
            request_id: request_id.clone(),
            step: Box::new(binding.clone()),
        },
        MachineCommand::CommitTelemetryBinding {
            request_id: request_id.clone(),
            step: Box::new(binding.clone()),
        },
        MachineCommand::RecoverTelemetryBinding {
            request_id: request_id.clone(),
            recovery: Box::new(recovery.clone()),
        },
        MachineCommand::QueryTelemetryRecovery {
            request_id: request_id.clone(),
            recovery: Box::new(recovery),
        },
        MachineCommand::QueryTelemetryRecoveryAudit {
            request_id: request_id.clone(),
            query: Box::new(RecoveryAuditQuery {
                schema: 1,
                step: binding,
            }),
        },
        MachineCommand::ExportBoundTelemetry {
            request_id,
            attempt: Box::new(export),
        },
    ]
}

#[test]
fn all_scoped_commands_check_both_identity_axes_even_through_generic_entrypoints() {
    let root = tempfile::tempdir().unwrap();
    let publisher = crate::machine_auth::MachineIdentity::load_or_create(root.path()).unwrap();
    let desired = crate::machine_plugins::telemetry_release_for_test(&publisher, "1.0.0");
    let control = MachineControl::default();
    let (tx, mut incoming) = mpsc::unbounded_channel();
    let token = control.install("machine-test".into(), "epoch".into(), false, 19, tx);
    for service in ["service-test", "foreign-service"] {
        for machine in ["machine-test", "foreign-machine"] {
            let expected = service == "service-test" && machine == "machine-test";
            let cases = commands(&desired, service, machine);
            assert_eq!(cases.len(), 12);
            for command in cases {
                // A real wire round-trip remains a claim, never an owner.
                let bytes = serde_json::to_vec(&command).unwrap();
                let command: MachineCommand = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(
                    control.send("machine-test", command.clone()).is_ok(),
                    expected
                );
                assert_eq!(incoming.try_recv().is_ok(), expected);
                // Test both generic command calls and original-token calls.
                for binding in [None, Some(RequestBinding::Connection(&token))] {
                    let result = control.begin_request(
                        "machine-test",
                        "site-fixture",
                        command.clone(),
                        ReplyKind::Command,
                        binding,
                    );
                    assert_eq!(result.is_ok(), expected);
                    assert_eq!(incoming.try_recv().is_ok(), expected);
                    drop(result);
                    assert!(control.live.read().pending.is_empty());
                }
            }
        }
    }
    assert!(control.is_current(&token));
    assert!(control.live.read().events.is_empty());
}
