use super::*;
use crate::core::SessionRegistration;
use crate::machine_protocol::{MachineCommand, MachineEvent};

pub(super) fn create(hub: &Hub, machine: &str) {
    hub.create_session(SessionRegistration {
        id: "session".into(),
        provider: "codex".into(),
        provider_version: String::new(),
        provider_generation_digest: String::new(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: machine.into(),
        workspace_id: Some("workspace".into()),
        workspace_name: None,
        workspace_source_path: None,
        cwd: "/work/a".into(),
        title: "title".into(),
        origin: SessionOrigin::default(),
        system: false,
        owner_user_id: Some("fixture-user".into()),
        owner_username: None,
    });
}

#[tokio::test]
async fn remote_language_reply_is_discarded_after_retarget_without_followup() {
    let hub = Hub::new();
    create(&hub, "machine-test");
    let scope = hub.session_code_scope("session").unwrap();
    let control = MachineControl::default();
    let (sender, mut commands) = mpsc::unbounded_channel();
    let connection = control.install("machine-test".into(), "epoch".into(), false, 19, sender);
    let request = serde_json::json!({"type":"ensureWorktree", "path":scope.cwd(), "trusted":true});
    let respond = async {
        let MachineCommand::AdapterRequest {
            request_id,
            adapter,
            payload,
            workspace_incarnation: None,
        } = commands.recv().await.unwrap()
        else {
            panic!("adapter request expected")
        };
        assert_eq!(adapter, "zed");
        assert_eq!(payload["path"], "/work/a");
        hub.update_session_cwd("session", "/work/b".into()).unwrap();
        hub.update_session_cwd("session", "/work/a".into()).unwrap();
        control.record_remote(
            &connection,
            MachineEvent::AdapterResponse {
                request_id,
                accepted: true,
                payload: Some(
                    serde_json::json!({"type":"worktree", "api_version":1, "state":"ready"}),
                ),
                detail: None,
                refusal: None,
            },
        );
    };
    let (reply, ()) = tokio::join!(
        zed_request_in_scope(&hub, &control, None, &scope, request),
        respond
    );
    assert_eq!(
        reply.err().expect("scope must be stale").to_string(),
        "code context changed"
    );
    let followup = serde_json::json!({"type":"openBuffer", "worktree":scope.cwd(), "path":"a.rs", "leaseId":"fixture"});
    assert_eq!(
        zed_request_in_scope(&hub, &control, None, &scope, followup)
            .await
            .err()
            .expect("scope must be stale")
            .to_string(),
        "code context changed"
    );
    assert!(commands.try_recv().is_err());
}

#[tokio::test]
async fn an_observation_from_another_service_hub_sends_nothing() {
    let hub = Hub::new();
    let other = Hub::new();
    create(&hub, "machine-test");
    create(&other, "machine-test");
    let scope = other.session_code_scope("session").unwrap();
    let control = MachineControl::default();
    let (sender, mut commands) = mpsc::unbounded_channel();
    control.install("machine-test".into(), "epoch".into(), false, 19, sender);
    assert_eq!(
        zed_request_in_scope(
            &hub,
            &control,
            None,
            &scope,
            serde_json::json!({"type":"health"})
        )
        .await
        .err()
        .expect("scope must be foreign")
        .to_string(),
        "code context changed"
    );
    assert!(commands.try_recv().is_err());
}

#[tokio::test]
async fn local_language_reply_checks_the_same_scope_after_socket_io() {
    for change in [false, true] {
        let hub = Hub::new();
        create(&hub, "local");
        let scope = hub.session_code_scope("session").unwrap();
        let control = MachineControl::default();
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("zed.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let respond = async {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut request = String::new();
            BufReader::new(read).read_line(&mut request).await.unwrap();
            if change {
                hub.delete_session("session");
                create(&hub, "local");
            } else {
                hub.rename_session("session", "new title".into());
                hub.set_status("session", Status::Running, None);
            }
            write
                .write_all(b"{\"type\":\"worktree\",\"api_version\":1,\"state\":\"ready\"}\n")
                .await
                .unwrap();
        };
        let (reply, ()) = tokio::join!(
            zed_request_in_scope(
                &hub,
                &control,
                Some(&socket),
                &scope,
                serde_json::json!({"type":"ensureWorktree", "path":scope.cwd(), "trusted":true})
            ),
            respond
        );
        if change {
            assert_eq!(
                reply.err().expect("scope must be stale").to_string(),
                "code context changed"
            );
        } else {
            assert!(matches!(
                reply.unwrap(),
                ZedAdapterResponse::Worktree { .. }
            ));
        }
    }
}
