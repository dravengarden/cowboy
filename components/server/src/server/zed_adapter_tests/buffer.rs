use super::*;
use crate::machine_protocol::{MachineCommand, MachineEvent};
use crate::server::zed_session::{BufferError, BufferRequest, buffer_request};

pub(super) fn open_request() -> BufferRequest<'static> {
    BufferRequest {
        worktree: "/work/a",
        path: "a.rs",
        lease_id: "browser-lease",
        open: true,
    }
}

pub(super) fn respond(
    control: &MachineControl,
    connection: &crate::machine_control::ConnectionToken,
    command: MachineCommand,
    payload: serde_json::Value,
) {
    let MachineCommand::AdapterRequest { request_id, .. } = command else {
        panic!("adapter command expected")
    };
    control.record_remote(
        connection,
        MachineEvent::AdapterResponse {
            request_id,
            accepted: true,
            payload: Some(payload),
            detail: None,
            refusal: None,
        },
    );
}

#[tokio::test]
async fn buffer_open_never_continues_after_a_same_epoch_reconnect() {
    let hub = Hub::new();
    scope::create(&hub, "machine-test");
    let scope = hub.session_code_scope("session").unwrap();
    let control = MachineControl::default();
    let (sender, mut original) = mpsc::unbounded_channel();
    let connection = control.install("machine-test".into(), "epoch".into(), false, 19, sender);
    let mut operation =
        std::pin::pin!(buffer_request(&hub, &control, None, &scope, open_request()));
    assert!(futures::poll!(&mut operation).is_pending());
    respond(
        &control,
        &connection,
        original.try_recv().unwrap(),
        serde_json::json!({"type":"worktree", "api_version":1, "state":"ready"}),
    );
    // Complete a reply, then replace the connection before its caller resumes.
    let (sender, mut replacement) = mpsc::unbounded_channel();
    control.install("machine-test".into(), "epoch".into(), false, 19, sender);
    assert!(matches!(
        futures::poll!(&mut operation),
        std::task::Poll::Ready(Err(_))
    ));
    assert!(replacement.try_recv().is_err());
    assert!(original.try_recv().is_err());
}

#[tokio::test]
async fn buffer_open_requires_positive_worktree_readiness() {
    for state in ["loading", "failed", "", "READY"] {
        let hub = Hub::new();
        scope::create(&hub, "machine-test");
        let scope = hub.session_code_scope("session").unwrap();
        let control = MachineControl::default();
        let (sender, mut commands) = mpsc::unbounded_channel();
        let connection = control.install("machine-test".into(), "epoch".into(), false, 19, sender);
        let mut operation =
            std::pin::pin!(buffer_request(&hub, &control, None, &scope, open_request()));
        assert!(futures::poll!(&mut operation).is_pending());
        respond(
            &control,
            &connection,
            commands.try_recv().unwrap(),
            serde_json::json!({"type":"worktree", "api_version":1, "state":state}),
        );
        assert!(matches!(
            futures::poll!(&mut operation),
            std::task::Poll::Ready(Err(BufferError::Unavailable(_)))
        ));
        assert!(commands.try_recv().is_err());
    }
}

#[tokio::test]
async fn buffer_open_rejects_wrong_variant_and_version_before_followup() {
    for payload in [
        serde_json::json!({"type":"buffer", "api_version":1, "path":"a.rs", "leases":1}),
        serde_json::json!({"type":"worktree", "api_version":2, "state":"ready"}),
        serde_json::json!({"type":"error", "message":"unavailable"}),
    ] {
        let hub = Hub::new();
        scope::create(&hub, "machine-test");
        let scope = hub.session_code_scope("session").unwrap();
        let control = MachineControl::default();
        let (sender, mut commands) = mpsc::unbounded_channel();
        let connection = control.install("machine-test".into(), "epoch".into(), false, 19, sender);
        let mut operation =
            std::pin::pin!(buffer_request(&hub, &control, None, &scope, open_request()));
        assert!(futures::poll!(&mut operation).is_pending());
        respond(&control, &connection, commands.try_recv().unwrap(), payload);
        assert!(matches!(
            futures::poll!(&mut operation),
            std::task::Poll::Ready(Err(BufferError::Unavailable(_)))
        ));
        assert!(commands.try_recv().is_err());
    }
}

#[tokio::test]
async fn stable_remote_buffer_open_and_close_preserve_the_wire_contract() {
    let hub = Hub::new();
    scope::create(&hub, "machine-test");
    let scope = hub.session_code_scope("session").unwrap();
    let control = MachineControl::default();
    let (sender, mut commands) = mpsc::unbounded_channel();
    let connection = control.install("machine-test".into(), "epoch".into(), false, 19, sender);
    for open in [true, false] {
        let respond = async {
            if open {
                let command = commands.recv().await.unwrap();
                assert!(
                    matches!(&command, MachineCommand::AdapterRequest { payload, .. } if *payload == serde_json::json!({"type":"ensureWorktree", "path":"/work/a", "trusted":true}))
                );
                respond(
                    &control,
                    &connection,
                    command,
                    serde_json::json!({"type":"worktree", "api_version":1, "state":"ready"}),
                );
            }
            let command = commands.recv().await.unwrap();
            assert!(
                matches!(&command, MachineCommand::AdapterRequest { payload, .. } if *payload == serde_json::json!({"type": if open {"openBuffer"} else {"closeBuffer"}, "worktree":"/work/a", "path":"a.rs", "leaseId":"browser-lease"}))
            );
            respond(
                &control,
                &connection,
                command,
                serde_json::json!({"type":"buffer", "api_version":1, "path":"a.rs", "leases":usize::from(open)}),
            );
        };
        let (reply, ()) = tokio::join!(
            buffer_request(
                &hub,
                &control,
                None,
                &scope,
                BufferRequest {
                    open,
                    ..open_request()
                }
            ),
            respond
        );
        assert!(
            matches!(reply.unwrap(), ZedAdapterResponse::Buffer {path, leases, ..} if path == "a.rs" && leases == usize::from(open))
        );
        assert!(commands.try_recv().is_err());
    }
}
