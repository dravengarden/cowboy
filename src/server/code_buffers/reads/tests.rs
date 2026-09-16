use super::super::tests::{Fixture, authenticated, create, json_response, native, payload, reply};
use super::*;
use crate::machine_protocol::MachineCommand;
use serde_json::{Value, json};

mod authority;

#[tokio::test]
async fn nonempty_observations_match_browser_wire_fixture() {
    let contract: Value = serde_json::from_str(include_str!(
        "../../../../contracts/code-buffer-client.fixture.json"
    ))
    .unwrap();
    let (mut fixture, id) = opened().await;
    for (request, kind) in [
        (Request::Language {}, "language"),
        (Request::Symbols {}, "symbols"),
    ] {
        let task = start(&fixture, &id, request);
        probe(&mut fixture).await;
        let command = command(&mut fixture).await;
        let mut native = result(kind);
        native["result"] = contract[kind]["result"].clone();
        native["opened_version"] = contract[kind]["openedVersion"].clone();
        reply(&fixture.context, &fixture.connection, command, native);
        let response = task.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut expected = contract[kind].clone();
        expected["resourceId"] = json!(id);
        assert_eq!(json_response(response).await, expected);
    }
}

async fn opened() -> (Fixture, String) {
    let mut fixture = Fixture::new();
    let id = fixture.prepare(1).await.resource_id;
    assert_eq!(
        fixture.operation(&id, Action::Open, "open").await.status(),
        StatusCode::OK
    );
    (fixture, id)
}

fn start(fixture: &Fixture, id: &str, request: Request) -> tokio::task::JoinHandle<Response> {
    tokio::spawn(read(
        State(fixture.context.clone()),
        Path(id.to_owned()),
        Extension(authenticated()),
        HeaderMap::new(),
        Json(request),
    ))
}

async fn command(fixture: &mut Fixture) -> MachineCommand {
    tokio::time::timeout(std::time::Duration::from_secs(5), fixture.commands.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn probe(fixture: &mut Fixture) {
    let command = command(fixture).await;
    assert_eq!(payload(&command), &json!({"type":"bufferLeaseReadSupport"}));
    reply(
        &fixture.context,
        &fixture.connection,
        command,
        json!({"type":"bufferLeaseReadSupport", "api_version":1}),
    );
}

fn result(kind: &str) -> Value {
    let result = if kind == "language" {
        json!({"kind":"language","diagnosticsState":"unobserved","diagnostics":[],"inlayHints":[],"semanticTokens":[]})
    } else {
        json!({"kind":"symbols","symbols":[]})
    };
    json!({"type":"bufferLeaseRead","api_version":1,"lease":native(1),"opened_version":[],"result":result})
}

#[tokio::test]
async fn reads_are_pathless_typed_and_do_not_export_native_authority() {
    let (mut fixture, id) = opened().await;
    for (request, kind) in [
        (Request::Language {}, "language"),
        (Request::Symbols {}, "symbols"),
    ] {
        let task = start(&fixture, &id, request);
        probe(&mut fixture).await;
        let command = command(&mut fixture).await;
        assert_eq!(
            payload(&command),
            &json!({"type":"readBufferLease","lease":native(1),"request":{"kind":kind}})
        );
        reply(&fixture.context, &fixture.connection, command, result(kind));
        let response = task.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            json_response(response).await,
            json!({"apiVersion":1,"resourceId":id,"openedVersion":[],"result":result(kind)["result"]})
        );
    }
    assert_eq!(
        fixture
            .operation(&id, Action::Release, "released")
            .await
            .status(),
        StatusCode::OK
    );
    assert!(fixture.commands.try_recv().is_err());
}

#[tokio::test]
async fn old_machines_or_forged_health_cannot_enable_reads() {
    for unsupported in [
        Value::Null,
        json!({"type":"health","buffer_lease_api":1}),
        json!({"type":"bufferLeaseReadSupport","api_version":2}),
        json!({"type":"bufferLeaseReadSupport","api_version":1,"extra":true}),
    ] {
        let (mut fixture, id) = opened().await;
        let task = start(&fixture, &id, Request::Symbols {});
        let probe = command(&mut fixture).await;
        reply(&fixture.context, &fixture.connection, probe, unsupported);
        assert_eq!(task.await.unwrap().status(), StatusCode::NOT_IMPLEMENTED);
        assert!(fixture.commands.try_recv().is_err());
        assert_eq!(
            fixture
                .operation(&id, Action::Release, "released")
                .await
                .status(),
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn stale_scope_discards_results_and_preserves_original_cleanup() {
    for stage in ["probe", "read"] {
        for recreate in [false, true] {
            let (mut fixture, id) = opened().await;
            let task = start(&fixture, &id, Request::Language {});
            if stage == "read" {
                probe(&mut fixture).await;
            }
            let command = command(&mut fixture).await;
            if recreate {
                fixture.context.hub.delete_session("session");
                create(&fixture.context.hub, "machine");
            } else {
                fixture
                    .context
                    .hub
                    .update_session_cwd("session", "/other".into())
                    .unwrap();
                fixture
                    .context
                    .hub
                    .update_session_cwd("session", "/original/worktree".into())
                    .unwrap();
            }
            let value = if stage == "read" {
                result("language")
            } else {
                json!({"type":"bufferLeaseReadSupport","api_version":1})
            };
            reply(&fixture.context, &fixture.connection, command, value);
            assert_eq!(task.await.unwrap().status(), StatusCode::CONFLICT);
            assert!(fixture.commands.try_recv().is_err());
            assert_eq!(
                fixture
                    .operation(&id, Action::Release, "released")
                    .await
                    .status(),
                StatusCode::OK
            );
        }
    }
}

#[tokio::test]
async fn reconnect_discards_old_reads_without_forwarding_to_replacement() {
    let (mut fixture, id) = opened().await;
    let task = start(&fixture, &id, Request::Language {});
    probe(&mut fixture).await;
    let command = command(&mut fixture).await;
    let (sender, mut replacement) = mpsc::unbounded_channel();
    fixture.context.machine_control.install(
        "machine".into(),
        "same-epoch".into(),
        false,
        19,
        sender,
    );
    reply(
        &fixture.context,
        &fixture.connection,
        command,
        result("language"),
    );
    assert!(matches!(
        task.await.unwrap().status(),
        StatusCode::BAD_GATEWAY | StatusCode::CONFLICT
    ));
    assert!(replacement.try_recv().is_err());
    assert!(fixture.commands.try_recv().is_err());
    assert_eq!(
        start(&fixture, &id, Request::Symbols {})
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn invalid_results_do_not_change_open_evidence_or_replay_mutations() {
    for (pointer, value) in [
        ("/lease", native(2)),
        ("/result", json!({"kind":"symbols","symbols":[]})),
        ("/api_version", json!(2)),
    ] {
        let (mut fixture, id) = opened().await;
        let task = start(&fixture, &id, Request::Language {});
        probe(&mut fixture).await;
        let command = command(&mut fixture).await;
        let mut wrong = result("language");
        *wrong.pointer_mut(pointer).unwrap() = value;
        reply(&fixture.context, &fixture.connection, command, wrong);
        assert_eq!(task.await.unwrap().status(), StatusCode::BAD_GATEWAY);
        assert!(fixture.commands.try_recv().is_err());
        assert_eq!(
            fixture
                .operation(&id, Action::Release, "released")
                .await
                .status(),
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn observer_cancellation_does_not_release_a_still_borrowed_owner() {
    let (mut fixture, id) = opened().await;
    let task = start(&fixture, &id, Request::Language {});
    probe(&mut fixture).await;
    let command = command(&mut fixture).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let pending = operate(
        fixture.context.clone(),
        id.clone(),
        authenticated(),
        HeaderMap::new(),
        Action::Release,
    )
    .await;
    assert_eq!(pending.status(), StatusCode::ACCEPTED);
    assert!(fixture.commands.try_recv().is_err());
    reply(
        &fixture.context,
        &fixture.connection,
        command,
        result("language"),
    );
    let mut drained = false;
    for _ in 0..100 {
        let observed = operate(
            fixture.context.clone(),
            id.clone(),
            authenticated(),
            HeaderMap::new(),
            Action::Open,
        )
        .await;
        if json_response(observed).await["pending"] == false {
            drained = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(drained);
    assert_eq!(
        fixture
            .operation(&id, Action::Release, "released")
            .await
            .status(),
        StatusCode::OK
    );
}
