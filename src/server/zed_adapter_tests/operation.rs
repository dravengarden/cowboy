use super::*;
use crate::server::zed_session::{Operation, buffer_request};

fn ready() -> serde_json::Value {
    serde_json::json!({"type":"worktree", "api_version":1, "state":"ready"})
}

#[tokio::test]
async fn an_existing_operation_never_adopts_a_reconnected_machine() {
    let hub = Hub::new();
    scope::create(&hub, "machine-test");
    let scope = hub.session_code_scope("session").unwrap();
    let control = MachineControl::default();
    let (sender, mut original) = mpsc::unbounded_channel();
    let connection = control.install("machine-test".into(), "epoch".into(), false, 19, sender);
    let mut operation = Operation::connect(&hub, &control, None, &scope)
        .await
        .unwrap();
    let respond = async {
        buffer::respond(
            &control,
            &connection,
            original.recv().await.unwrap(),
            ready(),
        );
    };
    let (reply, ()) = tokio::join!(
        operation.request(serde_json::json!({"type":"ensureWorktree"})),
        respond
    );
    assert!(reply.is_ok());
    let (sender, mut replacement) = mpsc::unbounded_channel();
    control.install("machine-test".into(), "epoch".into(), false, 19, sender);
    assert_eq!(
        operation
            .request(serde_json::json!({"type":"openBuffer"}))
            .await
            .err()
            .unwrap()
            .to_string(),
        "Machine operation connection is no longer current"
    );
    assert!(replacement.try_recv().is_err());
    assert_eq!(
        operation
            .request(serde_json::json!({"type":"closeBuffer"}))
            .await
            .err()
            .unwrap()
            .to_string(),
        "Zed operation has ended"
    );
}

#[tokio::test]
async fn cancelled_remote_request_ends_the_operation_without_replay() {
    let hub = Hub::new();
    scope::create(&hub, "machine-test");
    let scope = hub.session_code_scope("session").unwrap();
    let control = MachineControl::default();
    let (sender, mut commands) = mpsc::unbounded_channel();
    let connection = control.install("machine-test".into(), "epoch".into(), false, 19, sender);
    let mut operation = Operation::connect(&hub, &control, None, &scope)
        .await
        .unwrap();
    let command = {
        let mut request =
            std::pin::pin!(operation.request(serde_json::json!({"type":"openBuffer"})));
        assert!(futures::poll!(&mut request).is_pending());
        commands.try_recv().unwrap()
    };
    // A late reply does not revive the cancelled operation or cause cleanup RPC.
    buffer::respond(&control, &connection, command, ready());
    assert_eq!(
        operation
            .request(serde_json::json!({"type":"closeBuffer"}))
            .await
            .err()
            .unwrap()
            .to_string(),
        "Zed operation has ended"
    );
    assert!(commands.try_recv().is_err());
}

#[tokio::test]
async fn failed_remote_exchange_is_terminal_even_if_the_channel_stays_live() {
    for payload in [
        serde_json::json!({"type":"error", "message":"fixture error"}),
        serde_json::json!({"type":"worktree", "api_version":2, "state":"ready"}),
        serde_json::json!({"type":"unknown"}),
        serde_json::Value::Null,
    ] {
        let hub = Hub::new();
        scope::create(&hub, "machine-test");
        let scope = hub.session_code_scope("session").unwrap();
        let control = MachineControl::default();
        let (sender, mut commands) = mpsc::unbounded_channel();
        let connection = control.install("machine-test".into(), "epoch".into(), false, 19, sender);
        let mut operation = Operation::connect(&hub, &control, None, &scope)
            .await
            .unwrap();
        let respond = async {
            buffer::respond(
                &control,
                &connection,
                commands.recv().await.unwrap(),
                payload,
            );
        };
        let (reply, ()) = tokio::join!(
            operation.request(serde_json::json!({"type":"health"})),
            respond
        );
        assert!(reply.is_err());
        assert_eq!(
            operation
                .request(serde_json::json!({"type":"openBuffer"}))
                .await
                .err()
                .unwrap()
                .to_string(),
            "Zed operation has ended"
        );
        assert!(commands.try_recv().is_err());
    }
}

#[tokio::test]
async fn local_buffer_sequence_keeps_its_connected_peer_after_socket_replacement() {
    let hub = Hub::new();
    scope::create(&hub, "local");
    let scope = hub.session_code_scope("session").unwrap();
    let control = MachineControl::default();
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("zed.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let respond = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line).unwrap()["type"],
            "ensureWorktree"
        );
        std::fs::rename(&socket, directory.path().join("retired.sock")).unwrap();
        let replacement = UnixListener::bind(&socket).unwrap();
        stream
            .get_mut()
            .write_all(b"{\"type\":\"worktree\",\"api_version\":1,\"state\":\"ready\"}\n")
            .await
            .unwrap();
        line.clear();
        tokio::select! {
            result = stream.read_line(&mut line) => { assert!(result.unwrap() > 0); }
            _ = replacement.accept() => panic!("operation adopted a replacement peer"),
        }
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line).unwrap(),
            serde_json::json!({"type":"openBuffer", "worktree":"/work/a", "path":"a.rs", "leaseId":"browser-lease"})
        );
        stream
            .get_mut()
            .write_all(b"{\"type\":\"buffer\",\"api_version\":1,\"path\":\"a.rs\",\"leases\":1}\n")
            .await
            .unwrap();
    };
    let (reply, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(
            buffer_request(
                &hub,
                &control,
                Some(&socket),
                &scope,
                buffer::open_request()
            ),
            respond
        )
    })
    .await
    .unwrap();
    assert!(matches!(
        reply.unwrap(),
        ZedAdapterResponse::Buffer { leases: 1, .. }
    ));
}

#[tokio::test]
async fn local_response_framing_and_byte_limit_are_enforced() {
    const LIMIT: usize = 4 * 1024 * 1024;
    let valid = b"{\"type\":\"worktree\",\"api_version\":1,\"state\":\"ready\"}";
    for (size, newline, accepted) in [
        (LIMIT, true, true),
        (LIMIT + 1, true, false),
        (valid.len(), false, false),
        (0, false, false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("zed.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let respond = async {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            BufReader::new(read)
                .read_line(&mut String::new())
                .await
                .unwrap();
            let mut response = vec![b' '; size];
            if size >= valid.len() {
                response[..valid.len()].copy_from_slice(valid);
            }
            if newline {
                *response.last_mut().unwrap() = b'\n';
            }
            let _ = write.write_all(&response).await;
        };
        let (reply, ()) = tokio::join!(
            zed_adapter_request(&socket, serde_json::json!({"type":"health"})),
            respond
        );
        assert_eq!(reply.is_ok(), accepted, "size={size}, newline={newline}");
        if size > LIMIT {
            assert_eq!(
                reply.err().unwrap().to_string(),
                "Zed adapter response exceeds byte limit"
            );
        } else if !newline {
            assert_eq!(
                reply.err().unwrap().to_string(),
                "Zed adapter response is incomplete"
            );
        }
    }
}

#[tokio::test]
async fn cancelled_local_request_drops_the_peer_and_ends_the_operation() {
    let hub = Hub::new();
    scope::create(&hub, "local");
    let scope = hub.session_code_scope("session").unwrap();
    let control = MachineControl::default();
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("zed.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let mut operation = Operation::connect(&hub, &control, Some(&socket), &scope)
        .await
        .unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    let mut peer = BufReader::new(stream);
    {
        let mut line = String::new();
        let mut request =
            std::pin::pin!(operation.request(serde_json::json!({"type":"openBuffer"})));
        tokio::select! {
            _ = &mut request => panic!("peer did not reply"),
            result = peer.read_line(&mut line) => { assert!(result.unwrap() > 0); }
        }
    }
    assert_eq!(peer.read_line(&mut String::new()).await.unwrap(), 0);
    assert_eq!(
        operation
            .request(serde_json::json!({"type":"closeBuffer"}))
            .await
            .err()
            .unwrap()
            .to_string(),
        "Zed operation has ended"
    );
}

#[tokio::test(start_paused = true)]
async fn local_write_and_read_deadlines_end_the_operation() {
    for blocked_writer in [true, false] {
        let hub = Hub::new();
        scope::create(&hub, "local");
        let scope = hub.session_code_scope("session").unwrap();
        let control = MachineControl::default();
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("zed.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let mut operation = Operation::connect(&hub, &control, Some(&socket), &scope)
            .await
            .unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let mut peer = BufReader::new(stream);
        let payload = if blocked_writer {
            serde_json::json!({"padding":"x".repeat(3 * 1024 * 1024)})
        } else {
            serde_json::json!({"type":"health"})
        };
        {
            let mut request = std::pin::pin!(operation.request(payload));
            assert!(futures::poll!(&mut request).is_pending());
            if !blocked_writer {
                let mut line = String::new();
                tokio::select! {
                    _ = &mut request => panic!("peer did not reply"),
                    result = peer.read_line(&mut line) => { assert!(result.unwrap() > 0); }
                }
            }
            tokio::time::advance(std::time::Duration::from_secs(36)).await;
            assert_eq!(
                request.await.err().unwrap().to_string(),
                "Zed adapter exchange timed out"
            );
        }
        assert_eq!(
            operation
                .request(serde_json::json!({"type":"openBuffer"}))
                .await
                .err()
                .unwrap()
                .to_string(),
            "Zed operation has ended"
        );
    }
}

#[tokio::test]
async fn local_oversized_request_sends_nothing_and_ends_the_operation() {
    let hub = Hub::new();
    scope::create(&hub, "local");
    let scope = hub.session_code_scope("session").unwrap();
    let control = MachineControl::default();
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("zed.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let mut operation = Operation::connect(&hub, &control, Some(&socket), &scope)
        .await
        .unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    let payload = serde_json::json!({"padding":"x".repeat(4 * 1024 * 1024)});
    assert_eq!(
        operation.request(payload).await.err().unwrap().to_string(),
        "Zed adapter request exceeds byte limit"
    );
    assert_eq!(
        BufReader::new(stream)
            .read_line(&mut String::new())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        operation
            .request(serde_json::json!({"type":"health"}))
            .await
            .err()
            .unwrap()
            .to_string(),
        "Zed operation has ended"
    );
}
