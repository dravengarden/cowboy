//! An authenticated Machine name alone is not a complete execution Site.
use super::*;
use crate::machine_protocol::{plugin_install, telemetry_binding};

#[cfg(feature = "machine-host")]
#[path = "site_tests/matrix.rs"]
mod matrix;

#[test]
fn service_reopen_preserves_logical_identity_but_never_adopts_connection_handles() {
    let root = tempfile::tempdir().unwrap();
    let identity = crate::service_identity::load_or_create(root.path()).unwrap();
    let name = identity.as_str().to_owned();
    let first = MachineControl::new(identity);
    let (tx, _rx) = mpsc::unbounded_channel();
    let old = first.install("machine-test".into(), "same-epoch".into(), false, 19, tx);
    assert!(
        first
            .scoped_operation_connection(&name, "machine-test")
            .unwrap()
            .same(&old)
    );
    assert!(
        first
            .scoped_operation_connection("service-test", "machine-test")
            .is_err()
    );
    drop(first);

    let identity = crate::service_identity::load_or_create(root.path()).unwrap();
    assert_eq!(identity.as_str(), name);
    let reopened = MachineControl::new(identity);
    let (tx, mut commands) = mpsc::unbounded_channel();
    let current = reopened.install("machine-test".into(), "same-epoch".into(), false, 19, tx);
    assert!(!reopened.is_current(&old));
    assert!(!old.same(&current));
    assert!(
        reopened
            .scoped_operation_connection(&name, "machine-test")
            .unwrap()
            .same(&current)
    );
    let mut step = plugin_install::fixture();
    step.service_id = name;
    assert!(
        reopened
            .begin_request(
                "machine-test",
                "retained-handle",
                MachineCommand::QueryPluginInstallStep {
                    request_id: "retained-handle".into(),
                    step: Box::new(step),
                },
                ReplyKind::InstallationStep,
                Some(RequestBinding::Connection(&old)),
            )
            .is_err()
    );
    assert!(commands.try_recv().is_err());
    assert!(reopened.live.read().pending.is_empty());
}

#[test]
fn independent_services_with_identically_named_machines_cannot_exchange_routes() {
    let roots = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let first =
        MachineControl::new(crate::service_identity::load_or_create(roots[0].path()).unwrap());
    let second =
        MachineControl::new(crate::service_identity::load_or_create(roots[1].path()).unwrap());
    assert_ne!(first.service.as_str(), second.service.as_str());
    let (tx, _rx) = mpsc::unbounded_channel();
    let old = first.install("machine-test".into(), "same-epoch".into(), false, 19, tx);
    let (tx, _rx) = mpsc::unbounded_channel();
    let current = second.install("machine-test".into(), "same-epoch".into(), false, 19, tx);
    assert!(!second.is_current(&old));
    assert!(
        second
            .scoped_operation_connection(first.service.as_str(), "machine-test")
            .is_err()
    );
    first.disconnect("machine-test");
    assert!(second.is_current(&current));
}

#[test]
fn foreign_service_is_rejected_at_both_outgoing_boundaries() {
    let control = MachineControl::default();
    let (tx, mut commands) = mpsc::unbounded_channel();
    let token = control.install("machine-test".into(), "epoch".into(), false, 19, tx);
    let mut step = telemetry_binding::execution_fixture();
    step.service_id = "another-service".into();
    step.change = telemetry_binding::BindingChange::Revoke {
        policy_epoch: "1".to_owned().try_into().unwrap(),
    };
    step.validate_commit().unwrap();
    let command = MachineCommand::CommitTelemetryBinding {
        request_id: "foreign-site".into(),
        step: Box::new(step),
    };
    assert!(control.send("machine-test", command.clone()).is_err());
    assert!(
        control
            .begin_request(
                "machine-test",
                "foreign-site",
                command,
                ReplyKind::TelemetryBindingCommit,
                Some(RequestBinding::Connection(&token)),
            )
            .is_err()
    );
    assert!(commands.try_recv().is_err());
    assert!(control.live.read().pending.is_empty());
}

#[tokio::test]
async fn installation_query_rejects_foreign_service_before_remote_observation() {
    let control = MachineControl::default();
    let (tx, mut commands) = mpsc::unbounded_channel();
    let token = control.install("machine-test".into(), "epoch".into(), false, 19, tx);
    let mut query = plugin_install::fixture().target_query();
    query.service_id = "another-service".into();
    query.validate().unwrap();
    // Poll once: a foreign read must fail locally, not wait for Machine to
    // reject it. This also bounds the regression on the old implementation.
    let mut request = std::pin::pin!(control.plugin_installation_target(&token, &query));
    assert!(matches!(
        futures::poll!(&mut request),
        std::task::Poll::Ready(Err(CommandRequestError {
            certainty: CommandFailure::NotSent,
            ..
        }))
    ));
    assert!(commands.try_recv().is_err());
    assert!(control.live.read().pending.is_empty());
}

#[test]
fn synchronization_cannot_cross_a_site_or_older_machine_protocol_boundary() {
    use crate::machine_protocol::code_buffer_sync;
    for protocol in [19, 20] {
        let root = tempfile::tempdir().unwrap();
        let control =
            MachineControl::new(crate::service_identity::load_or_create(root.path()).unwrap());
        let (tx, mut commands) = mpsc::unbounded_channel();
        let token = control.install("machine".into(), "epoch".into(), false, protocol, tx);
        for (service, machine) in [
            (control.service.as_str().to_owned(), "machine"),
            (format!("svc-{}", "f".repeat(32)), "machine"),
            (control.service.as_str().to_owned(), "other"),
        ] {
            let request: code_buffer_sync::Request = serde_json::from_value(serde_json::json!({
                "service_id":service,"machine_id":machine,"action":{"kind":"query",
                "operation":{"instance":"a".repeat(32),"id":"0000000000000001"}}
            }))
            .unwrap();
            request.validate().unwrap();
            let command = MachineCommand::CodeBufferSync {
                request_id: "sync-site".into(),
                request: Box::new(request),
            };
            let expected =
                protocol == 20 && service == control.service.as_str() && machine == "machine";
            assert_eq!(control.send("machine", command.clone()).is_ok(), expected);
            assert_eq!(commands.try_recv().is_ok(), expected);
            let result = control.begin_request(
                "machine",
                "sync-site",
                command,
                ReplyKind::Adapter,
                Some(RequestBinding::Connection(&token)),
            );
            assert_eq!(result.is_ok(), expected);
            assert_eq!(commands.try_recv().is_ok(), expected);
            drop(result);
            assert!(control.live.read().pending.is_empty());
        }
    }
}

#[test]
fn navigation_cannot_cross_a_site_or_older_machine_protocol_boundary() {
    for protocol in [20, 21] {
        let root = tempfile::tempdir().unwrap();
        let control =
            MachineControl::new(crate::service_identity::load_or_create(root.path()).unwrap());
        let (tx, mut commands) = mpsc::unbounded_channel();
        let token = control.install("machine".into(), "epoch".into(), false, protocol, tx);
        for (service, machine) in [
            (control.service.as_str().to_owned(), "machine"),
            (format!("svc-{}", "f".repeat(32)), "machine"),
            (control.service.as_str().to_owned(), "other"),
        ] {
            let request = serde_json::from_value(serde_json::json!({
                "service_id":service,"machine_id":machine,"action":{"kind":"query",
                "navigation":{"instance":"a".repeat(32),"id":"navigation:0000000000000001"}}
            }))
            .unwrap();
            let command = MachineCommand::CodeBufferNavigation {
                request_id: "navigation-site".into(),
                request: Box::new(request),
            };
            let expected =
                protocol == 21 && service == control.service.as_str() && machine == "machine";
            assert_eq!(control.send("machine", command.clone()).is_ok(), expected);
            assert_eq!(commands.try_recv().is_ok(), expected);
            let result = control.begin_request(
                "machine",
                "navigation-site",
                command,
                ReplyKind::Adapter,
                Some(RequestBinding::Connection(&token)),
            );
            assert_eq!(result.is_ok(), expected);
            assert_eq!(commands.try_recv().is_ok(), expected);
            drop(result);
            assert!(control.live.read().pending.is_empty());
        }
    }
}
