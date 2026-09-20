use super::*;
use crate::machine_protocol::{MachineCommand, MachineEvent};
use tokio::sync::mpsc;

fn root(id: &str, path: &str) -> MachineWorkspace {
    MachineWorkspace {
        id: id.into(),
        display_name: id.into(),
        canonical_path: path.into(),
    }
}

fn connect(
    control: &MachineControl,
    machine: &str,
) -> (ConnectionToken, mpsc::UnboundedReceiver<MachineCommand>) {
    connect_at(control, machine, 21)
}

/// Protocol 22 is the Machine that owns and enforces root identities.
fn connect_at(
    control: &MachineControl,
    machine: &str,
    protocol: u16,
) -> (ConnectionToken, mpsc::UnboundedReceiver<MachineCommand>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (
        control.install(machine.into(), "same-epoch".into(), false, protocol, tx),
        rx,
    )
}

fn identity(workspace_id: &str, incarnation: &str) -> WorkspaceRootIdentity {
    WorkspaceRootIdentity {
        workspace_id: workspace_id.into(),
        incarnation: incarnation.into(),
    }
}

fn observe(
    control: &MachineControl,
    connection: &ConnectionToken,
    roots: Option<Vec<MachineWorkspace>>,
) {
    observe_owned(control, connection, roots, None);
}

fn observe_owned(
    control: &MachineControl,
    connection: &ConnectionToken,
    roots: Option<Vec<MachineWorkspace>>,
    identities: Option<Vec<WorkspaceRootIdentity>>,
) {
    control.record_remote(
        connection,
        MachineEvent::Inventory {
            components: vec![],
            workspaces: roots,
            workspace_identities: identities,
            workspace_revision: Some("config-revision".into()),
            observed_at_ms: 0,
        },
    );
}

fn scope(control: &MachineControl, machine: &str, workspace: &str) -> Option<WorkspaceCodeScope> {
    control.workspace_code_scope("service-test", machine, workspace)
}

#[test]
fn workspace_observations_preserve_only_continuous_exact_slots() {
    let control = MachineControl::default();
    let (connection, _rx) = connect(&control, "machine");
    let a = root("a", "/a");
    let b = root("b", "/b");
    observe(&control, &connection, Some(vec![a.clone(), b.clone()]));
    let original = scope(&control, "machine", "a").unwrap();
    let unrelated = scope(&control, "machine", "b").unwrap();
    let mut renamed = a.clone();
    renamed.display_name = "new name".into();
    observe(&control, &connection, Some(vec![b.clone(), renamed]));
    observe(&control, &connection, None);
    assert_eq!(scope(&control, "machine", "a").unwrap(), original);
    assert!(control.workspace_scope_is_current(&original));
    for changed in [
        vec![b.clone()],
        vec![root("a", "/different"), b.clone()],
        vec![a.clone(), a.clone(), b.clone()],
    ] {
        let before = scope(&control, "machine", "a").unwrap();
        observe(&control, &connection, Some(changed));
        observe(&control, &connection, Some(vec![a.clone(), b.clone()]));
        assert!(!control.workspace_scope_is_current(&before));
        assert_ne!(scope(&control, "machine", "a").unwrap(), before);
        assert!(control.workspace_scope_is_current(&unrelated));
    }
    assert!(!control.workspace_scope_is_current(&original));
}

#[test]
fn workspace_observations_are_service_registry_machine_and_connection_owned() {
    let control = MachineControl::default();
    let (old, _rx) = connect(&control, "machine");
    observe(&control, &old, Some(vec![root("a", "/a")]));
    let original = scope(&control, "machine", "a").unwrap();
    assert!(
        control
            .workspace_code_scope("foreign-service", "machine", "a")
            .is_none()
    );
    let foreign = MachineControl::default();
    let (foreign_connection, _rx2) = connect(&foreign, "machine");
    observe(&foreign, &foreign_connection, Some(vec![root("a", "/a")]));
    assert!(!foreign.workspace_scope_is_current(&original));
    let (other, _rx3) = connect(&control, "other");
    observe(&control, &other, Some(vec![root("a", "/a")]));
    assert_ne!(scope(&control, "other", "a").unwrap(), original);
    let (replacement, _rx4) = connect(&control, "machine");
    assert!(scope(&control, "machine", "a").is_none());
    observe(&control, &replacement, Some(vec![root("a", "/a")]));
    observe(&control, &old, Some(vec![]));
    control.remove_if_current(&old);
    assert!(!control.workspace_scope_is_current(&original));
    let current = scope(&control, "machine", "a").unwrap();
    control.disconnect("machine");
    assert!(!control.workspace_scope_is_current(&current));
    assert!(control.workspace_scope_is_current(&scope(&control, "other", "a").unwrap()));
}

#[test]
fn ambiguous_and_invalid_workspace_observations_do_not_get_read_scopes() {
    let control = MachineControl::default();
    let (connection, _rx) = connect(&control, "machine");
    for roots in [
        vec![root("a", "/a"), root("a", "/other")],
        vec![root("a", "relative")],
        vec![root("a", "/bad\0path")],
        vec![root("a", &format!("/{}", "x".repeat(4096)))],
    ] {
        observe(&control, &connection, Some(roots));
        assert!(scope(&control, "machine", "a").is_none());
    }
}

#[test]
fn workspace_observation_global_count_and_byte_budgets_cannot_multiply_by_machine() {
    let control = MachineControl::default();
    let mut connections = Vec::new();
    let mut receivers = Vec::new();
    for index in 0..4 {
        let machine = format!("machine-{index}");
        let (connection, rx) = connect(&control, &machine);
        observe(
            &control,
            &connection,
            Some(
                (0..MAX_WORKSPACES_PER_MACHINE)
                    .map(|id| root(&format!("w-{id}"), "/a"))
                    .collect(),
            ),
        );
        assert!(scope(&control, &machine, "w-0").is_some());
        connections.push(connection);
        receivers.push(rx);
    }
    let (extra, _extra_rx) = connect(&control, "extra");
    observe(&control, &extra, Some(vec![root("a", "/a")]));
    assert!(scope(&control, "extra", "a").is_none());
    let original = scope(&control, "machine-0", "w-0").unwrap();
    observe(
        &control,
        &connections[0],
        Some(
            (0..=MAX_WORKSPACES_PER_MACHINE)
                .map(|id| root(&format!("w-{id}"), "/a"))
                .collect(),
        ),
    );
    assert!(!control.workspace_scope_is_current(&original));
    assert!(scope(&control, "machine-1", "w-0").is_some());
    // Two full 4 KiB path sets plus other Machines exceed aggregate bytes.
    for index in [0, 1] {
        observe(
            &control,
            &connections[index],
            Some(
                (0..MAX_WORKSPACES_PER_MACHINE)
                    .map(|id| root(&format!("w-{id}"), &format!("/{}", "a".repeat(4095))))
                    .collect(),
            ),
        );
    }
    assert!(scope(&control, "machine-0", "w-0").is_some());
    assert!(scope(&control, "machine-1", "w-0").is_none());
    assert!(scope(&control, "machine-2", "w-0").is_some());
}

#[tokio::test]
async fn workspace_dispatch_refuses_ended_scope_before_registering_or_sending() {
    let control = MachineControl::default();
    let (connection, mut rx) = connect(&control, "machine");
    observe(&control, &connection, Some(vec![root("a", "/a")]));
    let original = scope(&control, "machine", "a").unwrap();
    observe(&control, &connection, Some(vec![]));
    observe(&control, &connection, Some(vec![root("a", "/a")]));
    assert!(
        control
            .code_request_in_workspace(&original, CodeOperation::Manifest)
            .await
            .is_err()
    );
    assert!(rx.try_recv().is_err());
    assert!(control.live.read().pending.is_empty());
    assert!(control.workspace_scope_is_colocated(&original).is_err());
}

#[tokio::test]
async fn workspace_dispatch_uses_original_root_and_rechecks_a_parked_reply() {
    for end_before_observation in [false, true] {
        let control = MachineControl::default();
        let (connection, mut rx) = connect(&control, "machine");
        observe(&control, &connection, Some(vec![root("a", "/original")]));
        let original = scope(&control, "machine", "a").unwrap();
        let mut request =
            Box::pin(control.code_request_in_workspace(&original, CodeOperation::Manifest));
        let command = tokio::select! {
            result = &mut request => panic!("unexpected result: {result:?}"),
            command = rx.recv() => command.unwrap(),
        };
        let MachineCommand::AdapterRequest {
            request_id,
            adapter,
            payload,
            workspace_incarnation,
        } = command
        else {
            panic!("wrong request")
        };
        // A protocol-21 Machine owns no identity, so none is invented here.
        assert!(workspace_incarnation.is_none());
        assert_eq!(adapter, "code");
        let decoded: CodeAdapterRequest = serde_json::from_value(payload).unwrap();
        assert_eq!(decoded.root, "/original");
        control.record_remote(
            &connection,
            MachineEvent::AdapterResponse {
                request_id,
                accepted: true,
                payload: Some(serde_json::json!({"original": true})),
                detail: None,
                refusal: None,
            },
        );
        if end_before_observation {
            observe(&control, &connection, Some(vec![]));
            observe(&control, &connection, Some(vec![root("a", "/original")]));
        }
        let result = request.await;
        assert_eq!(result.is_err(), end_before_observation);
        assert!(control.live.read().pending.is_empty());
        assert!(rx.try_recv().is_err());
    }
}

#[tokio::test]
async fn cancelling_workspace_observation_releases_only_its_waiter_without_replay() {
    let control = MachineControl::default();
    let (connection, mut rx) = connect(&control, "machine");
    observe(&control, &connection, Some(vec![root("a", "/a")]));
    let original = scope(&control, "machine", "a").unwrap();
    let mut request =
        Box::pin(control.code_request_in_workspace(&original, CodeOperation::Manifest));
    tokio::select! {
        result = &mut request => panic!("unexpected result: {result:?}"),
        command = rx.recv() => assert!(command.is_some()),
    }
    drop(request);
    assert!(control.live.read().pending.is_empty());
    assert!(control.workspace_scope_is_current(&original));
    assert!(rx.try_recv().is_err());
}

/// Fails against the previous implementation: an unchanged id, path and
/// connection preserved the identical scope across a replaced root object.
#[test]
fn a_new_machine_owned_incarnation_ends_the_old_observation() {
    let control = MachineControl::default();
    let (connection, _rx) = connect_at(&control, "machine", 22);
    let a = root("a", "/a");
    let b = root("b", "/b");
    let roots = Some(vec![a.clone(), b.clone()]);
    observe_owned(
        &control,
        &connection,
        roots.clone(),
        Some(vec![identity("a", "first"), identity("b", "stable")]),
    );
    let original = scope(&control, "machine", "a").unwrap();
    let unrelated = scope(&control, "machine", "b").unwrap();
    // The same observation again preserves both.
    observe_owned(
        &control,
        &connection,
        roots.clone(),
        Some(vec![identity("a", "first"), identity("b", "stable")]),
    );
    assert_eq!(scope(&control, "machine", "a").unwrap(), original);
    // Only the replaced root loses its identity.
    observe_owned(
        &control,
        &connection,
        roots.clone(),
        Some(vec![identity("a", "second"), identity("b", "stable")]),
    );
    assert!(!control.workspace_scope_is_current(&original));
    assert_ne!(scope(&control, "machine", "a").unwrap(), original);
    assert!(control.workspace_scope_is_current(&unrelated));
    // Returning to the original value is a new identity, never a revival.
    let replaced = scope(&control, "machine", "a").unwrap();
    observe_owned(
        &control,
        &connection,
        roots,
        Some(vec![identity("a", "first"), identity("b", "stable")]),
    );
    assert!(!control.workspace_scope_is_current(&original));
    assert!(!control.workspace_scope_is_current(&replaced));
}

#[test]
fn an_owning_machine_without_a_usable_identity_gets_no_read_scope() {
    let control = MachineControl::default();
    let (connection, _rx) = connect_at(&control, "machine", 22);
    let roots = Some(vec![root("a", "/a"), root("b", "/b")]);
    for identities in [
        // Absent entirely, absent for one root, duplicated, and structurally
        // invalid values all end that root instead of choosing a value.
        None,
        Some(vec![identity("b", "stable")]),
        Some(vec![
            identity("a", "one"),
            identity("a", "two"),
            identity("b", "stable"),
        ]),
        Some(vec![identity("a", ""), identity("b", "stable")]),
        Some(vec![identity("a", "has space"), identity("b", "stable")]),
        Some(vec![
            identity("a", &"x".repeat(129)),
            identity("b", "stable"),
        ]),
    ] {
        observe_owned(&control, &connection, roots.clone(), identities);
        assert!(scope(&control, "machine", "a").is_none());
        // An unrelated, well-formed root stays readable.
        assert_eq!(
            scope(&control, "machine", "b").is_some(),
            control.live.read().workspace_inventory["machine"].contains_key("b")
        );
    }
}

/// A protocol-21 Machine owns no identity yet, so its behaviour is unchanged
/// and advertised identities from it are ignored rather than trusted.
#[test]
fn an_older_machine_keeps_its_previous_observation_behaviour() {
    let control = MachineControl::default();
    let (connection, _rx) = connect(&control, "machine");
    let roots = Some(vec![root("a", "/a")]);
    observe_owned(
        &control,
        &connection,
        roots.clone(),
        Some(vec![identity("a", "ignored")]),
    );
    let original = scope(&control, "machine", "a").unwrap();
    observe_owned(
        &control,
        &connection,
        roots,
        Some(vec![identity("a", "different")]),
    );
    assert!(control.workspace_scope_is_current(&original));
}

#[tokio::test]
async fn workspace_dispatch_carries_the_exact_machine_minted_incarnation() {
    let control = MachineControl::default();
    let (connection, mut rx) = connect_at(&control, "machine", 22);
    observe_owned(
        &control,
        &connection,
        Some(vec![root("a", "/original")]),
        Some(vec![identity("a", "minted-value")]),
    );
    let original = scope(&control, "machine", "a").unwrap();
    let mut request =
        Box::pin(control.code_request_in_workspace(&original, CodeOperation::Manifest));
    let command = tokio::select! {
        result = &mut request => panic!("unexpected result: {result:?}"),
        command = rx.recv() => command.unwrap(),
    };
    let MachineCommand::AdapterRequest {
        adapter,
        payload,
        workspace_incarnation,
        ..
    } = command
    else {
        panic!("wrong request")
    };
    assert_eq!(adapter, "code");
    assert_eq!(workspace_incarnation.as_deref(), Some("minted-value"));
    let decoded: CodeAdapterRequest = serde_json::from_value(payload).unwrap();
    assert_eq!(decoded.root, "/original");
    drop(request);
    assert!(control.live.read().pending.is_empty());
}

/// Carrying an identity to a Machine that does not enforce it would be an
/// unchecked read. The command floor refuses that dispatch outright.
#[test]
fn a_carried_incarnation_requires_an_owning_machine() {
    let carried = MachineCommand::AdapterRequest {
        request_id: "adapter-1".into(),
        adapter: "code".into(),
        payload: serde_json::json!({"root":"/a","type":"manifest"}),
        workspace_incarnation: Some("minted-value".into()),
    };
    assert_eq!(
        carried.minimum_protocol(),
        CODE_WORKSPACE_ROOT_IDENTITY_PROTOCOL_VERSION
    );
    let plain = MachineCommand::AdapterRequest {
        request_id: "adapter-2".into(),
        adapter: "code".into(),
        payload: serde_json::json!({"root":"/a","type":"manifest"}),
        workspace_incarnation: None,
    };
    assert_eq!(plain.minimum_protocol(), 1);
}

/// The Machine's typed refusal is the earliest evidence that the observation
/// ended. Fails against the previous implementation, which kept the scope —
/// and therefore its caches, `ETags` and continuations — alive until the next
/// inventory arrived. Retirement records an end, never a rollback.
#[tokio::test]
async fn a_machine_root_identity_refusal_retires_exactly_that_observation() {
    use crate::machine_protocol::AdapterRefusal;
    for (refusal, retired) in [
        (Some(AdapterRefusal::WorkspaceRootIdentityChanged), true),
        // An ordinary read failure is a diagnostic, not an ended observation.
        (None, false),
    ] {
        let control = MachineControl::default();
        let (connection, mut rx) = connect_at(&control, "machine", 22);
        observe_owned(
            &control,
            &connection,
            Some(vec![root("a", "/a"), root("b", "/b")]),
            Some(vec![identity("a", "first"), identity("b", "stable")]),
        );
        let original = scope(&control, "machine", "a").unwrap();
        let unrelated = scope(&control, "machine", "b").unwrap();
        let mut request =
            Box::pin(control.code_request_in_workspace(&original, CodeOperation::Manifest));
        let command = tokio::select! {
            result = &mut request => panic!("unexpected result: {result:?}"),
            command = rx.recv() => command.unwrap(),
        };
        let MachineCommand::AdapterRequest { request_id, .. } = command else {
            panic!("wrong request")
        };
        control.record_remote(
            &connection,
            MachineEvent::AdapterResponse {
                request_id,
                accepted: false,
                payload: None,
                detail: Some("workspace root identity is no longer current".into()),
                refusal,
            },
        );
        assert!(request.await.is_err());
        assert_eq!(!control.workspace_scope_is_current(&original), retired);
        assert_eq!(scope(&control, "machine", "a").is_none(), retired);
        // Only that slot ends: the Machine, its connection and every other
        // advertised root keep their own observations.
        assert!(control.workspace_scope_is_current(&unrelated));
        assert!(control.live.read().pending.is_empty());
        assert!(rx.try_recv().is_err());
    }
}
