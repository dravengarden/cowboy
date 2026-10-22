use super::*;
use crate::machine_plugins::PluginExecutionScope;
use crate::machine_protocol::telemetry_recovery::{RecoveryResult, fixture, prepared};

fn open(root: &Path) -> MachinePluginStore {
    MachinePluginStore::new(
        root,
        crate::machine_protocol::Platform::Linux,
        "x86_64".into(),
    )
    .unwrap()
}

#[tokio::test]
async fn audit_discovery_survives_reopen_and_later_heads_without_any_execution_handle() {
    let root = tempfile::tempdir().unwrap();
    let mut request = fixture();
    request.step.change = BindingChange::Revoke {
        policy_epoch: "1".to_owned().try_into().unwrap(),
    };
    request.expected_observation_digest =
        binding_digest(&serde_json::to_vec(&prepared(&request.step).unwrap()).unwrap());
    let query = RecoveryAuditQuery {
        schema: 1,
        step: request.step.clone(),
    };
    let read = |store: &MachinePluginStore| {
        store
            .operations
            .telemetry_bindings
            .query_recovery_audit(&query)
    };
    let store = open(root.path());
    assert!(
        matches!(read(&store), RecoveryAuditObservation::Observed { snapshot } if snapshot.receipt.is_none())
    );
    assert!(!store.operations.telemetry_bindings.path.exists());
    store.enable_binding_writer_for_test();
    store.interrupt_binding_for_test(&request.step).await;
    assert!(matches!(
        read(&store),
        RecoveryAuditObservation::Unavailable {
            reason: BindingUnavailable::Storage
        }
    ));
    drop(store);
    let store = open(root.path());
    store.enable_binding_recovery_for_test();
    let scope = PluginExecutionScope::new(Some("service-test"), "machine-test");
    assert!(matches!(
        store
            .recover_telemetry_binding(&request, scope.telemetry_recovery(&request).unwrap())
            .await,
        RecoveryResult::Observed { .. }
    ));
    let first = read(&store);
    assert!(first.matches(&query));
    let RecoveryAuditObservation::Observed { snapshot: original } = first else {
        panic!()
    };
    assert!(original.receipt.is_some());
    let mut next = query.step.clone();
    next.operation_id.push_str("-later");
    next.expected_namespace = Some(BindingNamespace::Managed);
    store.enable_binding_writer_for_test();
    let result = store
        .commit_telemetry_binding_command(&next, scope.telemetry_binding(&next).unwrap())
        .await;
    assert!(matches!(
        result,
        crate::machine_protocol::telemetry_binding::BindingCommitResult::Observed { .. }
    ));
    drop(scope);
    drop(store);
    let store = open(root.path());
    let path = &store.operations.telemetry_bindings.path;
    let retained = fs::read(path).unwrap();
    let found = store
        .telemetry_recovery_audit(&query, Some("service-test"), "machine-test")
        .await;
    assert!(found.matches(&query));
    let RecoveryAuditObservation::Observed { snapshot } = found else {
        panic!()
    };
    assert_eq!(snapshot.receipt, original.receipt);
    assert!(
        matches!(snapshot.binding, BindingObservation::Observed { snapshot } if snapshot.current == Some(next.after().unwrap()))
    );
    assert_eq!(retained, fs::read(path).unwrap());
    assert!(
        store
            .operations
            .telemetry_bindings
            .ensure_legacy_allowed()
            .is_err()
    );
    let state = store.operations.telemetry_bindings.state.lock();
    assert!(!state.writer && !state.recovery_writer && state.reopened_prepared.is_none());
}

#[tokio::test]
async fn audit_discovery_rejects_owner_conflicts_corruption_and_stays_read_only() {
    let root = tempfile::tempdir().unwrap();
    let mut request = fixture();
    request.step.change = BindingChange::Revoke {
        policy_epoch: "1".to_owned().try_into().unwrap(),
    };
    let query = RecoveryAuditQuery {
        schema: 1,
        step: request.step.clone(),
    };
    let store = open(root.path());
    for (service, machine) in [
        (None, "machine-test"),
        (Some("foreign"), "machine-test"),
        (Some("service-test"), "foreign"),
    ] {
        assert!(matches!(
            store
                .telemetry_recovery_audit(&query, service, machine)
                .await,
            RecoveryAuditObservation::Unavailable {
                reason: BindingUnavailable::WrongOwner
            }
        ));
    }
    assert!(!store.operations.telemetry_bindings.path.exists());
    store.enable_binding_writer_for_test();
    store.interrupt_binding_for_test(&request.step).await;
    drop(store);
    let store = open(root.path());
    let mut foreign = query.clone();
    foreign.step.plan_digest = binding_digest(b"different original intent");
    assert!(matches!(
        store
            .operations
            .telemetry_bindings
            .query_recovery_audit(&foreign),
        RecoveryAuditObservation::Unavailable {
            reason: BindingUnavailable::IdentityConflict
        }
    ));
    let path = &store.operations.telemetry_bindings.path;
    let retained = fs::read(path).unwrap();
    atomic_write(path, b"{}", 0o600).unwrap();
    assert!(matches!(
        store
            .operations
            .telemetry_bindings
            .query_recovery_audit(&query),
        RecoveryAuditObservation::Unavailable {
            reason: BindingUnavailable::Storage
        }
    ));
    atomic_write(path, &retained, 0o600).unwrap();
    assert!(matches!(
        store
            .operations
            .telemetry_bindings
            .query_recovery_audit(&query),
        RecoveryAuditObservation::Unavailable {
            reason: BindingUnavailable::Storage
        }
    ));
    assert!(
        store
            .operations
            .telemetry_bindings
            .ensure_legacy_allowed()
            .is_err()
    );
}
