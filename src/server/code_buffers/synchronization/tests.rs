use super::*;
use crate::machine_protocol::{MachineCommand, MachineEvent};
use crate::server::code_buffers::tests::{Fixture, authenticated, json_response, native};
use serde_json::{Value, json};

#[test]
fn browser_synchronization_fixture_matches_service_serialization() {
    let content = Content {
        sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
        utf8_bytes: 3,
    };
    let state = NativeState::Applied {
        content: content.clone(),
        version: vec![
            VersionEntry {
                replica_id: 0,
                timestamp: 1,
            },
            VersionEntry {
                replica_id: u16::MAX,
                timestamp: u32::MAX,
            },
        ],
    };
    state.validate(&content).unwrap();
    let snapshot = Snapshot {
        api_version: 1,
        operation_id: "sync-0123456789abcdef0123456789abcdef-0000000000000001".into(),
        resource_id: "0123456789abcdef0123456789abcdef-0000000000000001".into(),
        purpose: Purpose::RefreshFromDisk,
        content,
        state: State::from(state),
        pending: false,
    };
    let expected: Value = serde_json::from_str(include_str!(
        "../../../../contracts/code-buffer-sync.fixture.json"
    ))
    .unwrap();
    assert_eq!(serde_json::to_value(snapshot).unwrap(), expected);
}

pub(super) fn fixture(protocol: u16) -> Fixture {
    let mut fixture = Fixture::new();
    let directory = tempfile::tempdir().unwrap();
    let identity = crate::service_identity::load_or_create(directory.path()).unwrap();
    fixture.context.service_id = identity.as_str().to_owned();
    fixture.context.machine_control = Arc::new(MachineControl::new(identity));
    let (sender, commands) = mpsc::unbounded_channel();
    fixture.connection = fixture.context.machine_control.install(
        "machine".into(),
        "same-epoch".into(),
        false,
        protocol,
        sender,
    );
    fixture.commands = commands;
    fixture
}

pub(super) fn opened(fixture: &Fixture, number: u64) -> String {
    opened_as(fixture, number, "local")
}

fn opened_as(fixture: &Fixture, number: u64, user: &str) -> String {
    let owners = &fixture.context.code_buffers;
    let id = owners
        .insert(
            owners.reserve().unwrap(),
            Binding {
                user: user.into(),
                scope: fixture.context.hub.session_code_scope("session").unwrap(),
                connection: fixture.connection.clone(),
                native: serde_json::from_value(native(number)).unwrap(),
            },
        )
        .unwrap()
        .resource_id;
    let super::super::registry::Admission::Run(job) =
        owners.admit(user, &id, remote::Action::Open).unwrap()
    else {
        panic!("expected open");
    };
    job.begin().unwrap();
    job.finish(remote::LeaseState::Open).unwrap();
    id
}

pub(super) fn content() -> Content {
    Content {
        sha256: "a".repeat(64),
        utf8_bytes: 17,
    }
}

pub(super) fn applied() -> NativeState {
    NativeState::Applied {
        content: content(),
        version: vec![VersionEntry {
            replica_id: 1,
            timestamp: 7,
        }],
    }
}

pub(super) fn observation(state: NativeState) -> Value {
    json!({"api_version":1,"operation":native(1),"state":state})
}

pub(super) async fn command(fixture: &mut Fixture) -> MachineCommand {
    tokio::time::timeout(std::time::Duration::from_secs(5), fixture.commands.recv())
        .await
        .unwrap()
        .unwrap()
}

pub(super) fn reply(fixture: &Fixture, command: MachineCommand, value: Value) {
    let MachineCommand::CodeBufferSync {
        request_id,
        request,
    } = command
    else {
        panic!("sync must not use generic forwarding");
    };
    assert_eq!(request.service_id, fixture.context.service_id);
    assert_eq!(request.machine_id, "machine");
    fixture.context.machine_control.record_remote(
        &fixture.connection,
        MachineEvent::AdapterResponse {
            request_id,
            accepted: true,
            payload: Some(value),
            detail: None,
        },
    );
}

pub(super) async fn prepared(fixture: &mut Fixture, resource: &str) -> Snapshot {
    let context = fixture.context.clone();
    let principal = authenticated();
    let headers = HeaderMap::new();
    let future = prepare_inner(
        &context,
        resource.to_owned(),
        &principal,
        &headers,
        Prepare {
            purpose: Purpose::RefreshFromDisk,
            content: content(),
        },
    );
    tokio::pin!(future);
    assert!(futures::poll!(&mut future).is_pending());
    let command = command(fixture).await;
    let MachineCommand::CodeBufferSync { request, .. } = &command else {
        panic!();
    };
    assert_eq!(
        request.action,
        NativeAction::Prepare {
            lease: serde_json::from_value(native(1)).unwrap(),
            purpose: Purpose::RefreshFromDisk,
            content: content(),
        }
    );
    reply(fixture, command, observation(NativeState::Prepared {}));
    future.await.unwrap()
}

pub(super) fn start(
    fixture: &Fixture,
    id: &str,
    action: Action,
) -> tokio::task::JoinHandle<Response> {
    tokio::spawn(operate(
        fixture.context.clone(),
        id.into(),
        authenticated(),
        HeaderMap::new(),
        action,
    ))
}

#[tokio::test]
async fn explicit_confirmation_is_once_and_terminal_cleanup_uses_original_id() {
    let mut fixture = fixture(20);
    let resource = opened(&fixture, 1);
    let prepared = prepared(&mut fixture, &resource).await;
    let id = &prepared.operation_id;
    assert_ne!(id, &resource);
    let body = serde_json::to_value(&prepared).unwrap();
    assert_eq!(body["resourceId"], resource);
    assert_eq!(body["purpose"], "refresh_from_disk");
    assert_eq!(body["content"], json!(content()));
    assert!(body.get("operation").is_none());
    assert!(body.get("lease").is_none());
    let owners = &fixture.context.code_buffers;
    assert!(matches!(
        owners.admit_read("local", &resource),
        Err(StatusCode::CONFLICT)
    ));
    assert!(matches!(
        owners.admit("local", &resource, remote::Action::Release),
        Err(StatusCode::CONFLICT)
    ));
    let task = start(&fixture, id, Action::Apply);
    let sent = command(&mut fixture).await;
    let MachineCommand::CodeBufferSync { request, .. } = &sent else {
        panic!();
    };
    assert_eq!(
        request.action,
        NativeAction::Apply {
            operation: serde_json::from_value(native(1)).unwrap()
        }
    );
    let pending = operate(
        fixture.context.clone(),
        id.into(),
        authenticated(),
        HeaderMap::new(),
        Action::Apply,
    )
    .await;
    assert_eq!(pending.status(), StatusCode::ACCEPTED);
    assert_eq!(json_response(pending).await["state"]["kind"], "unknown");
    reply(&fixture, sent, observation(applied()));
    assert_eq!(
        json_response(task.await.unwrap()).await["state"]["kind"],
        "applied"
    );
    for action in [Action::Apply, Action::Query] {
        let saved = start(&fixture, id, action).await.unwrap();
        assert_eq!(json_response(saved).await["state"]["kind"], "applied");
    }
    assert!(fixture.commands.try_recv().is_err());
    drop(
        fixture
            .context
            .code_buffers
            .admit_read("local", &resource)
            .unwrap(),
    );
    // Cleanup cannot require a replacement Session or its path.
    fixture.context.hub.delete_session("session");
    let task = start(&fixture, id, Action::Retire);
    let sent = command(&mut fixture).await;
    reply(&fixture, sent, observation(NativeState::Retired {}));
    assert_eq!(
        json_response(task.await.unwrap()).await["state"]["kind"],
        "retired"
    );
    assert_eq!(
        start(&fixture, id, Action::Apply).await.unwrap().status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        json_response(start(&fixture, id, Action::Retire).await.unwrap()).await["state"]["kind"],
        "retired"
    );
    assert!(fixture.commands.try_recv().is_err());
}

#[tokio::test]
async fn cancelled_observer_does_not_cancel_or_repeat_admitted_apply() {
    let mut fixture = fixture(20);
    let resource = opened(&fixture, 1);
    let prepared = prepared(&mut fixture, &resource).await;
    let task = start(&fixture, &prepared.operation_id, Action::Apply);
    let sent = command(&mut fixture).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    reply(&fixture, sent, observation(applied()));
    for _ in 0..100 {
        let value = json_response(
            start(&fixture, &prepared.operation_id, Action::Apply)
                .await
                .unwrap(),
        )
        .await;
        if value["pending"] == false {
            assert_eq!(value["state"]["kind"], "applied");
            assert!(fixture.commands.try_recv().is_err());
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("owned task did not settle");
}

#[tokio::test]
async fn invalid_outcomes_keep_exclusion_and_never_rearm_apply() {
    let mut wrong_owner = observation(applied());
    wrong_owner["operation"] = native(2);
    let mut wrong_content = observation(applied());
    wrong_content["state"]["content"]["utf8Bytes"] = json!(18);
    let mut unknown_field = observation(NativeState::Unknown {});
    unknown_field["state"]["permission"] = json!(true);
    for value in [
        json!(null),
        wrong_owner,
        wrong_content,
        unknown_field,
        observation(NativeState::Prepared {}),
        observation(NativeState::Retired {}),
    ] {
        let mut fixture = fixture(20);
        let resource = opened(&fixture, 1);
        let prepared = prepared(&mut fixture, &resource).await;
        let id = &prepared.operation_id;
        let task = start(&fixture, id, Action::Apply);
        let sent = command(&mut fixture).await;
        reply(&fixture, sent, value);
        assert_eq!(task.await.unwrap().status(), StatusCode::BAD_GATEWAY);
        assert_eq!(
            json_response(start(&fixture, id, Action::Apply).await.unwrap()).await["state"]["kind"],
            "unknown"
        );
        assert!(matches!(
            fixture.context.code_buffers.admit_read("local", &resource),
            Err(StatusCode::CONFLICT)
        ));
        assert_eq!(
            start(&fixture, id, Action::Retire).await.unwrap().status(),
            StatusCode::CONFLICT
        );
        assert!(fixture.commands.try_recv().is_err());
        let task = start(&fixture, id, Action::Query);
        let sent = command(&mut fixture).await;
        reply(&fixture, sent, observation(applied()));
        assert_eq!(
            json_response(task.await.unwrap()).await["state"]["kind"],
            "applied"
        );
    }
}

#[tokio::test]
async fn session_aba_and_connection_replacement_cannot_authorize_an_old_apply() {
    for change in ["session", "cwd", "connection"] {
        let mut fixture = fixture(20);
        let resource = opened(&fixture, 1);
        let prepared = prepared(&mut fixture, &resource).await;
        let (sender, mut replacement) = mpsc::unbounded_channel();
        match change {
            "session" => {
                fixture.context.hub.delete_session("session");
                crate::server::code_buffers::tests::create(&fixture.context.hub, "machine");
            }
            "cwd" => {
                fixture
                    .context
                    .hub
                    .update_session_cwd("session", "/elsewhere".into())
                    .unwrap();
                fixture
                    .context
                    .hub
                    .update_session_cwd("session", "/original/worktree".into())
                    .unwrap();
            }
            _ => {
                fixture.context.machine_control.install(
                    "machine".into(),
                    "same-epoch".into(),
                    false,
                    20,
                    sender,
                );
            }
        }
        assert_eq!(
            start(&fixture, &prepared.operation_id, Action::Apply)
                .await
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );
        assert!(fixture.commands.try_recv().is_err());
        assert!(replacement.try_recv().is_err());
    }
}

#[tokio::test]
async fn protocol_floor_unknown_fields_and_foreign_users_fail_before_effects() {
    let fixture = fixture(19);
    let resource = opened(&fixture, 1);
    let result = prepare_inner(
        &fixture.context,
        resource.clone(),
        &authenticated(),
        &HeaderMap::new(),
        Prepare {
            purpose: Purpose::RefreshFromDisk,
            content: content(),
        },
    )
    .await;
    assert!(matches!(result, Err(StatusCode::NOT_IMPLEMENTED)));
    drop(
        fixture
            .context
            .code_buffers
            .admit_read("local", &resource)
            .unwrap(),
    );
    let app = routes()
        .with_state(fixture.context.clone())
        .layer(Extension(authenticated()));
    struct Server(tokio::task::JoinHandle<()>);
    impl Drop for Server {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let _server = Server(tokio::spawn(async {
        axum::serve(listener, app).await.unwrap();
    }));
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    for body in [
        json!({"purpose":"restore","content":content()}),
        json!({"purpose":"refresh_from_disk","content":content(),"lease":native(1)}),
        json!({"purpose":"refresh_from_disk","content":{"sha256":"a".repeat(64),"utf8Bytes":17,"grant":true}}),
    ] {
        let result = client
            .post(format!(
                "{base}/api/code/buffers/{resource}/synchronizations"
            ))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(result.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(result.headers()[header::CACHE_CONTROL], "no-store");
    }
}

mod authority;
