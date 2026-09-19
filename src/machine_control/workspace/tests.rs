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
    let (tx, rx) = mpsc::unbounded_channel();
    (
        control.install(machine.into(), "same-epoch".into(), false, 21, tx),
        rx,
    )
}

fn observe(
    control: &MachineControl,
    connection: &ConnectionToken,
    roots: Option<Vec<MachineWorkspace>>,
) {
    control.record_remote(
        connection,
        MachineEvent::Inventory {
            components: vec![],
            workspaces: roots,
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
        } = command
        else {
            panic!("wrong request")
        };
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
