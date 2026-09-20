use super::*;
use crate::core::{Hub, SessionOrigin, SessionRegistration};
use crate::machine_protocol::{MachineCommand, MachineEvent};
use tokio::sync::mpsc;

fn session(machine: &str) -> SessionCodeScope {
    let hub = Hub::new();
    hub.create_session(SessionRegistration {
        id: "session".into(),
        provider: "codex".into(),
        provider_version: String::new(),
        provider_generation_digest: String::new(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: machine.into(),
        workspace_id: None,
        workspace_name: None,
        workspace_source_path: None,
        cwd: "/original".into(),
        title: "fixture".into(),
        origin: SessionOrigin::default(),
        system: false,
        owner_user_id: None,
        owner_username: None,
    });
    hub.session_code_scope("session").unwrap()
}

fn connect(
    control: &MachineControl,
    machine: &str,
    colocated: bool,
) -> (ConnectionToken, mpsc::UnboundedReceiver<MachineCommand>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (
        control.install(machine.into(), "same-epoch".into(), colocated, 21, tx),
        rx,
    )
}

fn scope(control: &MachineControl, session: &SessionCodeScope) -> SessionReadScope {
    control
        .session_read_scope("service-test", session.clone())
        .unwrap()
}

#[tokio::test]
async fn local_session_reads_require_the_original_core_owner_not_a_machine() {
    let control = MachineControl::default();
    let local = session("local");
    let original = scope(&control, &local);
    assert_eq!(original, scope(&control, &local));
    assert!(original.connection().is_none());
    assert!(control.session_read_scope_is_current(&original));
    assert!(control.session_read_is_colocated(&original).unwrap());
    assert!(
        control
            .code_request_in_session(&original, CodeOperation::Manifest)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(original.string_bytes(), local.string_bytes());
    assert!(control.live.read().pending.is_empty());
    assert!(
        control
            .session_read_scope("foreign-service", local.clone())
            .is_none()
    );
    let foreign = MachineControl::default();
    assert_ne!(scope(&foreign, &local), original);
    assert!(!foreign.session_read_scope_is_current(&original));
    assert!(foreign.session_read_is_colocated(&original).is_err());
    assert!(
        foreign
            .code_request_in_session(&original, CodeOperation::Manifest)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn session_read_dispatch_never_switches_original_remote_or_colocated_routes() {
    for originally_colocated in [false, true] {
        let control = MachineControl::default();
        let session = session("machine");
        assert!(
            control
                .session_read_scope("service-test", session.clone())
                .is_none()
        );
        let (_old, mut old_commands) = connect(&control, "machine", originally_colocated);
        let original = scope(&control, &session);
        assert_eq!(
            control.session_read_is_colocated(&original),
            Ok(originally_colocated)
        );
        let (_replacement, mut new_commands) = connect(&control, "machine", !originally_colocated);
        assert_ne!(scope(&control, &session), original);
        assert!(control.session_read_is_colocated(&original).is_err());
        assert!(
            control
                .code_request_in_session(&original, CodeOperation::Manifest)
                .await
                .is_err()
        );
        assert!(old_commands.try_recv().is_err());
        assert!(new_commands.try_recv().is_err());
        assert!(control.live.read().pending.is_empty());
        let current = scope(&control, &session);
        control.disconnect("machine");
        assert!(!control.session_read_scope_is_current(&current));
        assert!(control.session_read_is_colocated(&current).is_err());
        assert!(
            control
                .session_read_scope("service-test", session.clone())
                .is_none()
        );
    }
}

#[test]
fn session_read_routes_ignore_other_machines_and_cannot_cross_registries() {
    let control = MachineControl::default();
    let (_first, _rx) = connect(&control, "machine", false);
    let session = session("machine");
    let original = scope(&control, &session);
    let (_other, _other_rx) = connect(&control, "other", true);
    control.disconnect("other");
    assert_eq!(scope(&control, &session), original);
    assert!(control.session_read_scope_is_current(&original));
    assert!(original.string_bytes() > session.string_bytes());
    let foreign = MachineControl::default();
    let (_same_names, _foreign_rx) = connect(&foreign, "machine", false);
    assert_ne!(scope(&foreign, &session), original);
    assert!(!foreign.session_read_scope_is_current(&original));
}

#[tokio::test]
async fn session_read_dispatch_derives_original_root_and_rechecks_parked_replies() {
    for replace in [false, true] {
        let control = MachineControl::default();
        let (connection, mut commands) = connect(&control, "machine", false);
        let original = scope(&control, &session("machine"));
        let mut request =
            Box::pin(control.code_request_in_session(&original, CodeOperation::Manifest));
        let command = tokio::select! {
            result = &mut request => panic!("unexpected response: {result:?}"),
            command = commands.recv() => command.unwrap(),
        };
        let MachineCommand::AdapterRequest {
            request_id,
            adapter,
            payload,
            workspace_incarnation,
        } = command
        else {
            panic!("wrong command")
        };
        // A Session route executes in a session worktree, not an advertised
        // root, so it never borrows another observation's root identity.
        assert!(workspace_incarnation.is_none());
        assert_eq!(adapter, "code");
        let decoded: CodeAdapterRequest = serde_json::from_value(payload).unwrap();
        assert_eq!(decoded.root, "/original");
        assert!(matches!(decoded.operation, CodeOperation::Manifest));
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
        let replacement = replace.then(|| connect(&control, "machine", false));
        // Keep the observer parked until the real reply has completed AND the
        // replacement (if any) is installed; correlation alone is insufficient.
        let result = request.await;
        assert_eq!(result.is_err(), replace);
        if !replace {
            assert_eq!(result.unwrap(), Some(serde_json::json!({"original": true})));
        }
        assert!(control.live.read().pending.is_empty());
        assert!(commands.try_recv().is_err());
        if let Some((_, mut commands)) = replacement {
            assert!(commands.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn cancelled_session_read_drops_only_its_waiter_without_replay_or_cleanup() {
    let control = MachineControl::default();
    let (_connection, mut commands) = connect(&control, "machine", false);
    let original = scope(&control, &session("machine"));
    let mut request = Box::pin(control.code_request_in_session(&original, CodeOperation::Manifest));
    tokio::select! {
        result = &mut request => panic!("unexpected response: {result:?}"),
        command = commands.recv() => assert!(command.is_some()),
    }
    drop(request);
    assert!(control.live.read().pending.is_empty());
    assert!(control.session_read_scope_is_current(&original));
    assert!(commands.try_recv().is_err());
}
