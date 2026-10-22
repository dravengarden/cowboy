use super::*;
use crate::machine_protocol::code_buffer_navigation::{NavigationRef, Snapshot as NativeSnapshot};
use crate::machine_protocol::{MachineCommand, MachineEvent};
use crate::server::code_buffers::tests::{Fixture, authenticated, json_response, native};
use serde_json::{Value, json};

pub(super) fn fixture(protocol: u16) -> Fixture {
    let mut fixture = Fixture::new();
    let directory = tempfile::tempdir().unwrap();
    let identity = crate::service_identity::load_or_create(directory.path()).unwrap();
    fixture.context.service_id = identity.as_str().to_owned();
    fixture.context.machine_control = Arc::new(MachineControl::new(identity));
    fixture.context.code_navigation_admission = true;
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

pub(super) fn content() -> Content {
    Content {
        sha256: "a".repeat(64),
        utf8_bytes: 17,
    }
}
pub(super) fn request() -> Prepare {
    Prepare {
        content: content(),
        position: Point { row: 0, column: 4 },
        query: Kind::Definition,
    }
}
pub(super) fn navigation(number: u64) -> NavigationRef {
    NavigationRef {
        instance: "b".repeat(32),
        id: format!("navigation:{number:016x}"),
    }
}
pub(super) fn locations() -> Vec<Location> {
    vec![Location {
        path: "target.rs".into(),
        content: content(),
        start: Point { row: 0, column: 4 },
        end: Point { row: 0, column: 6 },
    }]
}
pub(super) fn observed(phase: Phase) -> NativeSnapshot {
    NativeSnapshot {
        api_version: 1,
        navigation: navigation(1),
        phase,
        locations: if matches!(
            phase,
            Phase::Retained | Phase::ReleaseUnknown | Phase::Released
        ) {
            locations()
        } else {
            vec![]
        },
        destinations: vec![],
    }
}

pub(super) fn opened(fixture: &Fixture) -> String {
    opened_as(fixture, "local")
}

fn opened_as(fixture: &Fixture, user: &str) -> String {
    let owners = &fixture.context.code_buffers;
    let id = owners
        .insert(
            owners.reserve().unwrap(),
            Binding {
                user: user.into(),
                scope: fixture.context.hub.session_code_scope("session").unwrap(),
                connection: fixture.connection.clone(),
                native: serde_json::from_value(native(1)).unwrap(),
            },
        )
        .unwrap()
        .resource_id;
    let super::super::registry::Admission::Run(job) =
        owners.admit(user, &id, remote::Action::Open).unwrap()
    else {
        panic!("open job");
    };
    job.begin().unwrap();
    job.finish(remote::LeaseState::Open).unwrap();
    id
}

mod authority;
mod client_wire;

async fn command(fixture: &mut Fixture) -> MachineCommand {
    tokio::time::timeout(std::time::Duration::from_secs(3), fixture.commands.recv())
        .await
        .unwrap()
        .unwrap()
}

fn reply(fixture: &Fixture, command: MachineCommand, value: Value) {
    let MachineCommand::CodeBufferNavigation {
        request_id,
        request,
    } = command
    else {
        panic!("navigation is not generic forwarding");
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
            refusal: None,
        },
    );
}

async fn prepared(fixture: &mut Fixture, resource: &str) -> String {
    let task = tokio::spawn(prepare(
        AxumState(fixture.context.clone()),
        Path(resource.into()),
        Extension(authenticated()),
        HeaderMap::new(),
        Json(request()),
    ));
    let sent = command(fixture).await;
    reply(
        fixture,
        sent,
        serde_json::to_value(observed(Phase::Prepared)).unwrap(),
    );
    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value = json_response(response).await;
    assert!(value.get("navigation").is_none());
    value["navigationId"].as_str().unwrap().into()
}

fn start(fixture: &Fixture, id: &str, action: Action) -> tokio::task::JoinHandle<Response> {
    tokio::spawn(operate(
        fixture.context.clone(),
        id.into(),
        authenticated(),
        HeaderMap::new(),
        action,
    ))
}

#[tokio::test]
async fn navigation_requires_private_admission_original_open_and_protocol_21() {
    for (protocol, enabled) in [(20, true), (21, false)] {
        let mut fixture = fixture(protocol);
        fixture.context.code_navigation_admission = enabled;
        let resource = opened(&fixture);
        let response = prepare(
            AxumState(fixture.context.clone()),
            Path(resource),
            Extension(authenticated()),
            HeaderMap::new(),
            Json(request()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
        assert!(fixture.commands.try_recv().is_err());
    }
}

#[tokio::test]
async fn navigation_rejects_unadmitted_open_claim_and_changed_source_before_execute() {
    let mut fixture = fixture(21);
    let owners = &fixture.context.code_buffers;
    let resource = owners
        .insert(
            owners.reserve().unwrap(),
            Binding {
                user: "local".into(),
                scope: fixture.context.hub.session_code_scope("session").unwrap(),
                connection: fixture.connection.clone(),
                native: serde_json::from_value(native(4)).unwrap(),
            },
        )
        .unwrap()
        .resource_id;
    let super::super::registry::Admission::Run(job) = owners
        .admit("local", &resource, remote::Action::Query)
        .unwrap()
    else {
        panic!("query");
    };
    job.begin().unwrap();
    job.finish(remote::LeaseState::Open).unwrap();
    assert!(owners.navigation_source("local", &resource).is_err());
    let resource = opened(&fixture);
    let id = prepared(&mut fixture, &resource).await;
    let owners = &fixture.context.code_buffers;
    let (_, fence) = owners.synchronization_owner("local", &resource).unwrap();
    assert_eq!(
        start(&fixture, &id, Action::Execute)
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    drop(fence);
    let super::super::registry::Admission::Run(job) = owners
        .admit("local", &resource, remote::Action::Release)
        .unwrap()
    else {
        panic!("release");
    };
    job.begin().unwrap();
    job.finish(remote::LeaseState::Released).unwrap();
    assert_eq!(
        start(&fixture, &id, Action::Execute)
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert!(fixture.commands.try_recv().is_err());
}

#[tokio::test]
async fn execute_is_one_use_and_destination_enters_ordinary_ownership_without_open() {
    let mut fixture = fixture(21);
    let resource = opened(&fixture);
    let id = prepared(&mut fixture, &resource).await;
    let task = start(&fixture, &id, Action::Execute);
    let sent = command(&mut fixture).await;
    assert!(
        fixture
            .context
            .code_buffers
            .admit("local", &resource, remote::Action::Release)
            .is_ok_and(|admission| matches!(
                admission,
                super::super::registry::Admission::Saved(_)
            ))
    );
    reply(
        &fixture,
        sent,
        serde_json::to_value(observed(Phase::Retained)).unwrap(),
    );
    assert_eq!(task.await.unwrap().status(), StatusCode::OK);
    assert_eq!(
        start(&fixture, &id, Action::Execute)
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(fixture.commands.try_recv().is_err());
    let action = Action::Destination {
        destination: 0,
        content: content(),
    };
    let task = start(&fixture, &id, action.clone());
    let sent = command(&mut fixture).await;
    let mut value = observed(Phase::Retained);
    value.destinations.push(
        crate::machine_protocol::code_buffer_navigation::Destination {
            destination: 0,
            lease: serde_json::from_value(native(2)).unwrap(),
        },
    );
    reply(&fixture, sent, serde_json::to_value(&value).unwrap());
    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let saved = json_response(response).await;
    let target = saved["destinations"][0]["resourceId"].as_str().unwrap();
    assert_ne!(target, resource);
    assert_eq!(saved["destinations"][0]["state"], "prepared");
    assert_eq!(
        json_response(start(&fixture, &id, action).await.unwrap()).await,
        saved
    );
    assert!(fixture.commands.try_recv().is_err());
    assert!(
        fixture
            .context
            .code_buffers
            .admit_read("local", target)
            .is_err()
    );
    let super::super::registry::Admission::Run(job) = fixture
        .context
        .code_buffers
        .admit("local", target, remote::Action::Open)
        .unwrap()
    else {
        panic!("ordinary Open");
    };
    assert_eq!(
        job.binding.native,
        serde_json::from_value(native(2)).unwrap()
    );
    job.begin().unwrap();
    job.finish(remote::LeaseState::Open).unwrap();
    let task = start(&fixture, &id, Action::Release);
    let sent = command(&mut fixture).await;
    value.phase = Phase::Released;
    reply(&fixture, sent, serde_json::to_value(value).unwrap());
    assert_eq!(task.await.unwrap().status(), StatusCode::OK);
    assert!(
        fixture
            .context
            .code_buffers
            .admit_read("local", target)
            .is_ok()
    );
}

#[tokio::test]
async fn lost_observer_retains_execute_and_original_query_repairs_bad_reply() {
    let mut fixture = fixture(21);
    let source = opened(&fixture);
    let id = prepared(&mut fixture, &source).await;
    let task = start(&fixture, &id, Action::Execute);
    let sent = command(&mut fixture).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let mut invalid = serde_json::to_value(observed(Phase::Retained)).unwrap();
    invalid["navigation"]["id"] = json!("navigation:0000000000000002");
    reply(&fixture, sent, invalid);
    let saved = loop {
        let response = start(&fixture, &id, Action::Execute).await.unwrap();
        let value = json_response(response).await;
        if value["pending"] == false {
            break value;
        }
        tokio::task::yield_now().await;
    };
    assert_eq!(saved["state"], "unknown");
    assert!(fixture.commands.try_recv().is_err());
    assert_eq!(
        start(&fixture, &id, Action::Release)
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    let task = start(&fixture, &id, Action::Query);
    let sent = command(&mut fixture).await;
    reply(
        &fixture,
        sent,
        serde_json::to_value(observed(Phase::Retained)).unwrap(),
    );
    assert_eq!(
        json_response(task.await.unwrap()).await["state"],
        "retained"
    );
}

#[tokio::test]
async fn deleted_session_blocks_acquisition_not_original_cleanup_and_reconnect_cannot_adopt() {
    let mut fixture = fixture(21);
    let source = opened(&fixture);
    let id = prepared(&mut fixture, &source).await;
    fixture.context.hub.delete_session("session");
    assert_eq!(
        start(&fixture, &id, Action::Execute)
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert!(fixture.commands.try_recv().is_err());
    fixture.context.code_navigation_admission = false;
    let task = start(&fixture, &id, Action::Release);
    let sent = command(&mut fixture).await;
    let mut value = observed(Phase::Released);
    value.locations.clear();
    reply(&fixture, sent, serde_json::to_value(value).unwrap());
    assert_eq!(task.await.unwrap().status(), StatusCode::OK);
    let mut fixture = self::fixture(21);
    let source = opened(&fixture);
    let id = prepared(&mut fixture, &source).await;
    let (sender, mut commands) = mpsc::unbounded_channel();
    fixture.context.machine_control.install(
        "machine".into(),
        "same-epoch".into(),
        false,
        21,
        sender,
    );
    for action in [Action::Execute, Action::Query, Action::Release] {
        assert_eq!(
            start(&fixture, &id, action).await.unwrap().status(),
            StatusCode::CONFLICT
        );
    }
    assert!(commands.try_recv().is_err());
}

#[tokio::test]
async fn closed_http_codec_rejects_paths_grants_and_cacheable_errors() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let fixture = fixture(21);
    let router = super::super::router()
        .with_state(fixture.context)
        .layer(Extension(authenticated()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let client = reqwest::Client::new();
    for (path, value) in [
        (
            "/api/code/buffers/source/navigations",
            json!({"content":content(),"position":{"row":0,"column":0},"query":"definition","path":"target.rs"}),
        ),
        (
            "/api/code/navigations/group/destinations",
            json!({"destination":0,"content":content(),"lease":native(2)}),
        ),
        ("/api/code/navigations/group", json!({"authorized":true})),
    ] {
        let response = client
            .request(
                if path.ends_with("/group") {
                    reqwest::Method::PUT
                } else {
                    reqwest::Method::POST
                },
                format!("http://{address}{path}"),
            )
            .json(&value)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    server.abort();
}
