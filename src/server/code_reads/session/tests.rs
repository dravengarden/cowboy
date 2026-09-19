use super::*;
use crate::core::{SessionOrigin, SessionRegistration};
use crate::machine_control::ConnectionToken;
use crate::machine_protocol::MachineCommand;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse as _;
use tokio::sync::mpsc;

fn create(hub: &Hub, id: &str, machine: &str) {
    hub.create_session(SessionRegistration {
        id: id.into(),
        provider: "codex".into(),
        provider_version: String::new(),
        provider_generation_digest: String::new(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: machine.into(),
        workspace_id: Some("workspace".into()),
        workspace_name: None,
        workspace_source_path: None,
        cwd: "/original/worktree".into(),
        title: "fixture".into(),
        origin: SessionOrigin::default(),
        system: false,
        owner_user_id: Some("user".into()),
        owner_username: None,
    });
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

fn scope(hub: &Hub, control: &MachineControl, id: &str) -> CodeReadScope {
    resolve(hub, control, "service-test", id).unwrap()
}

#[test]
fn session_read_route_cannot_adopt_an_identical_replacement_connection() {
    let hub = Hub::new();
    let control = MachineControl::default();
    create(&hub, "session", "machine");
    let (_connection, _commands) = connect(&control, "machine", false);
    let original = scope(&hub, &control, "session");
    let (_replacement, _new_commands) = connect(&control, "machine", false);
    assert_ne!(scope(&hub, &control, "session"), original);
}

#[tokio::test]
async fn session_read_route_discards_buffered_responses_after_reconnect() {
    for status in [
        StatusCode::OK,
        StatusCode::NOT_MODIFIED,
        StatusCode::BAD_GATEWAY,
    ] {
        let hub = Hub::new();
        let control = MachineControl::default();
        create(&hub, "session", "machine");
        let (_connection, _commands) = connect(&control, "machine", false);
        let CodeReadScope::Session(original) = scope(&hub, &control, "session") else {
            unreachable!()
        };
        let response = super::super::guarded_response(
            || async { current(&hub, &control, &original) },
            || async {
                let (_replacement, _new_commands) = connect(&control, "machine", false);
                (status, [(header::ETAG, "\"old\"")], "old bytes").into_response()
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::GONE);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(!response.headers().contains_key(header::ETAG));
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap(),
            "code context changed"
        );
    }
}

#[test]
fn session_read_resolution_requires_the_actual_service_hub_and_live_route() {
    let hub = Hub::new();
    let control = MachineControl::default();
    create(&hub, "session", "machine");
    assert!(resolve(&hub, &control, "service-test", "session").is_none());
    let (_connection, _commands) = connect(&control, "machine", true);
    assert!(resolve(&hub, &control, "other-service", "session").is_none());
    let CodeReadScope::Session(original) = scope(&hub, &control, "session") else {
        unreachable!()
    };
    assert!(current(&hub, &control, &original));
    let foreign = Hub::new();
    create(&foreign, "session", "machine");
    assert!(!current(&foreign, &control, &original));
    assert!(!current(&hub, &MachineControl::default(), &original));
    control.disconnect("machine");
    assert!(!current(&hub, &control, &original));
    assert!(resolve(&hub, &control, "service-test", "session").is_none());
    // A stopped detached Session still exists. Losing its read route does not
    // delete the Session or convert its root into a Controller-local path.
    assert!(hub.session_code_scope("session").is_some());
    create(&hub, "standalone", "local");
    let CodeReadScope::Session(local) = scope(&hub, &control, "standalone") else {
        unreachable!()
    };
    assert!(current(&hub, &control, &local));
    assert!(local.connection().is_none());
}

#[test]
fn session_read_routes_keep_metadata_and_independent_sessions_but_not_cwd_aba() {
    let hub = Hub::new();
    let control = MachineControl::default();
    let (_connection, _commands) = connect(&control, "machine", false);
    for id in ["one", "two"] {
        create(&hub, id, "machine");
    }
    let first = scope(&hub, &control, "one");
    let second = scope(&hub, &control, "two");
    assert_ne!(first, second);
    hub.rename_session("one", "renamed".into());
    hub.set_status("one", crate::core::Status::Running, None);
    assert_eq!(scope(&hub, &control, "one"), first);
    hub.update_session_cwd("one", "/other".into()).unwrap();
    hub.update_session_cwd("one", "/original/worktree".into())
        .unwrap();
    assert_ne!(scope(&hub, &control, "one"), first);
    let CodeReadScope::Session(first) = first else {
        unreachable!()
    };
    assert!(!current(&hub, &control, &first));
    assert_eq!(scope(&hub, &control, "two"), second);
    hub.delete_session("one");
    create(&hub, "one", "machine");
    assert!(!current(&hub, &control, &first));
    assert_eq!(scope(&hub, &control, "two"), second);
}

#[tokio::test]
async fn stale_session_read_route_cannot_start_a_reader_or_choose_local_io() {
    let hub = Hub::new();
    let control = MachineControl::default();
    create(&hub, "session", "machine");
    let (_old, _commands) = connect(&control, "machine", false);
    let CodeReadScope::Session(original) = scope(&hub, &control, "session") else {
        unreachable!()
    };
    let (_new, mut commands) = connect(&control, "machine", true);
    let invoked = std::cell::Cell::new(false);
    let response = super::super::guarded_response(
        || async { current(&hub, &control, &original) },
        || {
            invoked.set(true);
            async { "invalid".into_response() }
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::GONE);
    assert!(!invoked.get());
    assert!(control.session_read_is_colocated(&original).is_err());
    assert!(commands.try_recv().is_err());
}

#[tokio::test]
async fn session_read_file_and_diff_cursors_cannot_cross_route_replacement() {
    use super::super::file_pages::{CursorError, PageCursors};
    use crate::code_review::{DiffDocument, DiffScope, FileDocument};
    use crate::diff_snapshot::{DiffSnapshotCache, DiffSnapshotKey};
    let hub = Hub::new();
    let control = MachineControl::default();
    create(&hub, "session", "machine");
    let (_connection, _commands) = connect(&control, "machine", false);
    let original = scope(&hub, &control, "session");
    let pages = PageCursors::default();
    let cursor = pages
        .project(
            &original,
            "a.txt",
            None,
            FileDocument {
                path: "a.txt".into(),
                revision: "a".repeat(64),
                text: "abc".into(),
                size: 100,
                truncated: true,
                next_cursor: Some(format!("{}:3", "a".repeat(64))),
                limited: false,
            },
        )
        .unwrap()
        .next_cursor
        .unwrap();
    let diffs = DiffSnapshotCache::default();
    let key = DiffSnapshotKey {
        owner: original.clone(),
        path: "a.txt".into(),
        context: 6,
        show_whitespace: true,
        scope: DiffScope::Unstaged,
    };
    let diff = diffs
        .first_page(key, || async {
            Ok(DiffDocument {
                path: "a.txt".into(),
                text: "+line\n".repeat(50_000),
                added: 50_000,
                removed: 0,
                truncated: false,
            })
        })
        .await
        .unwrap()
        .next_cursor
        .unwrap();
    hub.rename_session("session", "Renamed".into());
    let unchanged = scope(&hub, &control, "session");
    assert!(pages.resolve(&unchanged, "a.txt", Some(&cursor)).is_ok());
    assert!(diffs.next_page(&unchanged, &diff).await.is_ok());
    let (_replacement, _new_commands) = connect(&control, "machine", false);
    let replaced = scope(&hub, &control, "session");
    assert_eq!(
        pages
            .resolve(&replaced, "a.txt", Some(&cursor))
            .unwrap_err(),
        CursorError::Expired
    );
    assert_eq!(
        diffs.next_page(&replaced, &diff).await.unwrap_err(),
        "diff snapshot expired"
    );
}

#[tokio::test]
async fn manifest_readiness_uses_the_original_session_route_without_renewal() {
    use crate::machine_protocol::MachineEvent;
    for replacement in ["before", "parked", "none"] {
        let hub = Hub::new();
        let control = MachineControl::default();
        create(&hub, "session", "machine");
        let (connection, mut commands) = connect(&control, "machine", false);
        let CodeReadScope::Session(original) = scope(&hub, &control, "session") else {
            unreachable!()
        };
        if replacement == "before" {
            let (_new, mut new_commands) = connect(&control, "machine", false);
            assert!(
                worktree_ready(&hub, &control, None, &original)
                    .await
                    .is_err()
            );
            assert!(commands.try_recv().is_err());
            assert!(new_commands.try_recv().is_err());
            continue;
        }
        let mut read = Box::pin(worktree_ready(&hub, &control, None, &original));
        let command = tokio::select! {
            result = &mut read => panic!("unexpected readiness: {result:?}"),
            command = commands.recv() => command.unwrap(),
        };
        let MachineCommand::AdapterRequest {
            request_id,
            adapter,
            payload,
        } = command
        else {
            panic!("wrong command")
        };
        assert_eq!(adapter, "zed");
        assert_eq!(
            payload,
            serde_json::json!({"type":"ensureWorktree", "path":"/original/worktree", "trusted":true})
        );
        control.record_remote(
            &connection,
            MachineEvent::AdapterResponse {
                request_id,
                accepted: true,
                detail: None,
                payload: Some(
                    serde_json::json!({"type":"worktree", "api_version":1, "state":"ready"}),
                ),
            },
        );
        let new = (replacement == "parked").then(|| connect(&control, "machine", false));
        let result = read.await;
        if replacement == "none" {
            assert!(result.unwrap());
        } else {
            assert!(result.is_err());
        }
        if let Some((_, mut new_commands)) = new {
            assert!(new_commands.try_recv().is_err());
        }
        assert!(commands.try_recv().is_err());
    }
}

#[tokio::test]
async fn standalone_local_readiness_retains_the_existing_unix_socket_contract() {
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
    let hub = Hub::new();
    let control = MachineControl::default();
    create(&hub, "session", "local");
    let CodeReadScope::Session(original) = scope(&hub, &control, "session") else {
        unreachable!()
    };
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("zed.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut line = String::new();
        BufReader::new(read).read_line(&mut line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["path"], "/original/worktree");
        write
            .write_all(b"{\"type\":\"worktree\",\"api_version\":1,\"state\":\"ready\"}\n")
            .await
            .unwrap();
    });
    assert!(
        worktree_ready(&hub, &control, Some(&socket), &original)
            .await
            .unwrap()
    );
    server.await.unwrap();
}
