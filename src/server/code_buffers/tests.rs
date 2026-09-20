use super::*;
use crate::core::SessionRegistration;
use crate::machine_control::ConnectionToken;
use crate::machine_protocol::{MachineCommand, MachineEvent};
use serde_json::{Value, json};

mod authority;

#[tokio::test]
async fn review_selection_is_effect_free_and_requires_the_connected_machine_floor() {
    let mut fixture = Fixture::new();
    let control = &fixture.context.machine_control;
    let capture = || {
        CodeReadScope::Session(
            control
                .session_read_scope(
                    "service-test",
                    fixture.context.hub.session_code_scope("session").unwrap(),
                )
                .unwrap(),
        )
    };
    let scope = capture();
    assert_eq!(review_mode(control, &scope), ReviewMode::Legacy);
    for (protocol, mode, wire) in [
        (20, ReviewMode::Owned, "owned"),
        (19, ReviewMode::Legacy, "legacy"),
    ] {
        let (sender, mut commands) = mpsc::unbounded_channel();
        let connection = control.install(
            "machine".into(),
            "same-epoch".into(),
            false,
            protocol,
            sender,
        );
        assert_eq!(review_mode(control, &scope), ReviewMode::Unavailable);
        assert_eq!(review_mode(control, &capture()), mode);
        assert_eq!(serde_json::to_value(mode).unwrap(), json!(wire));
        assert!(commands.try_recv().is_err());
        drop(connection);
    }
    assert!(fixture.commands.try_recv().is_err());
    assert_eq!(
        review_mode(&MachineControl::default(), &scope),
        ReviewMode::Unavailable
    );
    let hub = Hub::new();
    create(&hub, "local");
    let local = CodeReadScope::Session(
        control
            .session_read_scope("service-test", hub.session_code_scope("session").unwrap())
            .unwrap(),
    );
    assert_eq!(review_mode(control, &local), ReviewMode::Legacy);
}

#[tokio::test]
async fn lifecycle_matches_browser_wire_fixture() {
    let contract: Value = serde_json::from_str(include_str!(
        "../../../contracts/code-buffer-client.fixture.json"
    ))
    .unwrap();
    let mut fixture = Fixture::new();
    let prepared = fixture.prepare(1).await;
    let id = prepared.resource_id.clone();
    let mut expected = contract["prepared"].clone();
    expected["resourceId"] = json!(id);
    assert_eq!(serde_json::to_value(prepared).unwrap(), expected);
    for (action, state) in [(Action::Open, "open"), (Action::Release, "released")] {
        let response = fixture.operation(&id, action, state).await;
        assert_eq!(response.status(), StatusCode::OK);
        expected["state"] = json!(state);
        assert_eq!(json_response(response).await, expected);
    }
}

pub(super) struct Fixture {
    pub context: Context,
    pub connection: ConnectionToken,
    pub commands: mpsc::UnboundedReceiver<MachineCommand>,
    _shutdown: watch::Sender<bool>,
}

pub(super) fn create(hub: &Hub, machine: &str) {
    create_owned(hub, machine, "local");
}

pub(super) fn create_owned(hub: &Hub, machine: &str, user: &str) {
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
        cwd: "/original/worktree".into(),
        title: "fixture".into(),
        origin: SessionOrigin::default(),
        system: false,
        owner_user_id: Some(user.into()),
        owner_username: None,
    });
}

pub(super) fn authenticated() -> AuthenticatedProductRequest {
    AuthenticatedProductRequest {
        permissions: None,
        principal: crate::product_auth::local_product_principal(),
        cookie_session: None,
        device_identity: None,
    }
}

pub(super) fn native(id: u64) -> Value {
    json!({"instance":"a".repeat(32), "id":format!("{id:016x}")})
}

fn native_reply(id: u64, state: &str) -> Value {
    json!({"type":"bufferLease", "api_version":1, "lease":native(id), "state":state})
}

pub(super) fn payload(command: &MachineCommand) -> &Value {
    let MachineCommand::AdapterRequest {
        adapter, payload, ..
    } = command
    else {
        panic!("expected adapter command");
    };
    assert_eq!(adapter, "zed");
    payload
}

pub(super) fn reply(
    context: &Context,
    connection: &ConnectionToken,
    command: MachineCommand,
    value: Value,
) {
    let MachineCommand::AdapterRequest { request_id, .. } = command else {
        panic!("expected adapter command");
    };
    context.machine_control.record_remote(
        connection,
        MachineEvent::AdapterResponse {
            request_id,
            accepted: true,
            payload: Some(value),
            detail: None,
            refusal: None,
        },
    );
}

impl Fixture {
    pub fn new() -> Self {
        let (shutdown, receiver) = watch::channel(false);
        let context = Context {
            service_id: "fixture-service".into(),
            hub: Hub::new(),
            machine_control: Arc::default(),
            code_buffers: Arc::default(),
            code_navigation_admission: false,
            shutdown: receiver,
            product_auth_enabled: false,
            store: None,
            device_access: Arc::default(),
            product_authentication: Arc::new(
                crate::auth_plugins::ProductAuthentication::test_default(None),
            ),
        };
        create(&context.hub, "machine");
        let (sender, commands) = mpsc::unbounded_channel();
        let connection = context.machine_control.install(
            "machine".into(),
            "same-epoch".into(),
            false,
            19,
            sender,
        );
        Self {
            context,
            connection,
            commands,
            _shutdown: shutdown,
        }
    }

    pub async fn prepare(&mut self, id: u64) -> Snapshot {
        let context = self.context.clone();
        let task = async {
            let probe = self.commands.recv().await.unwrap();
            assert_eq!(payload(&probe), &json!({"type":"bufferLeaseSupport"}));
            reply(
                &context,
                &self.connection,
                probe,
                json!({"type":"bufferLeaseSupport", "api_version":1}),
            );
            let command = self.commands.recv().await.unwrap();
            assert_eq!(
                payload(&command),
                &json!({"type":"prepareBuffer", "worktree":"/original/worktree", "path":"src/main.rs"})
            );
            reply(
                &context,
                &self.connection,
                command,
                native_reply(id, "prepared"),
            );
        };
        let request = PrepareRequest {
            session_id: "session".into(),
            path: "src/main.rs".into(),
        };
        let principal = authenticated();
        let headers = HeaderMap::new();
        let (result, ()) =
            tokio::join!(prepare_inner(&context, &principal, &headers, request), task);
        result.unwrap()
    }

    pub(super) async fn operation(
        &mut self,
        resource: &str,
        action: Action,
        observed: &str,
    ) -> Response {
        let context = self.context.clone();
        let commands = &mut self.commands;
        let connection = &self.connection;
        let task = async {
            let command = commands.recv().await.unwrap();
            let kind = match action {
                Action::Open => "openBufferLease",
                Action::Query => "queryBufferLease",
                Action::Release => "releaseBufferLease",
            };
            assert_eq!(payload(&command), &json!({"type":kind, "lease":native(1)}));
            reply(&context, connection, command, native_reply(1, observed));
        };
        let (response, ()) = tokio::join!(
            operate(
                context.clone(),
                resource.into(),
                authenticated(),
                HeaderMap::new(),
                action
            ),
            task
        );
        response
    }
}

pub(super) async fn json_response(response: Response) -> Value {
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn original_owner_releases_after_session_deletion_without_path_lookup_or_replay() {
    let mut fixture = Fixture::new();
    let prepared = fixture.prepare(1).await;
    assert_eq!(
        fixture
            .operation(&prepared.resource_id, Action::Open, "open")
            .await
            .status(),
        StatusCode::OK
    );
    fixture.context.hub.delete_session("session");
    let closed = fixture
        .operation(&prepared.resource_id, Action::Release, "released")
        .await;
    assert_eq!(json_response(closed).await["state"], "released");
    for action in [Action::Query, Action::Release] {
        let result = operate(
            fixture.context.clone(),
            prepared.resource_id.clone(),
            authenticated(),
            HeaderMap::new(),
            action,
        )
        .await;
        assert_eq!(json_response(result).await["state"], "released");
    }
    let result = operate(
        fixture.context.clone(),
        prepared.resource_id,
        authenticated(),
        HeaderMap::new(),
        Action::Open,
    )
    .await;
    assert_eq!(result.status(), StatusCode::CONFLICT);
    assert!(fixture.commands.try_recv().is_err());
}

#[tokio::test]
async fn retarget_and_delete_recreate_reject_new_open_but_preserve_original_cleanup() {
    for recreate in [false, true] {
        let mut fixture = Fixture::new();
        let prepared = fixture.prepare(1).await;
        if recreate {
            fixture.context.hub.delete_session("session");
            create(&fixture.context.hub, "machine");
        } else {
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
        let result = operate(
            fixture.context.clone(),
            prepared.resource_id.clone(),
            authenticated(),
            HeaderMap::new(),
            Action::Open,
        )
        .await;
        assert_eq!(result.status(), StatusCode::CONFLICT);
        assert!(fixture.commands.try_recv().is_err());
        assert_eq!(
            fixture
                .operation(&prepared.resource_id, Action::Release, "released")
                .await
                .status(),
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn http_observer_cancellation_does_not_cancel_the_admitted_open() {
    let mut fixture = Fixture::new();
    let prepared = fixture.prepare(1).await;
    let mut observer = Box::pin(operate(
        fixture.context.clone(),
        prepared.resource_id.clone(),
        authenticated(),
        HeaderMap::new(),
        Action::Open,
    ));
    assert!(futures::poll!(&mut observer).is_pending());
    let command = fixture.commands.recv().await.unwrap();
    assert_eq!(payload(&command)["type"], "openBufferLease");
    drop(observer);
    reply(
        &fixture.context,
        &fixture.connection,
        command,
        native_reply(1, "open"),
    );
    // Await the independently owned task, not a guessed scheduling delay.
    let mut tasks_done = false;
    for _ in 0..100 {
        let result = operate(
            fixture.context.clone(),
            prepared.resource_id.clone(),
            authenticated(),
            HeaderMap::new(),
            Action::Open,
        )
        .await;
        let body = json_response(result).await;
        if body["pending"] == false {
            assert_eq!(body["state"], "open");
            tasks_done = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(tasks_done);
    assert!(
        fixture.commands.try_recv().is_err(),
        "duplicate must not open again"
    );
    assert_eq!(
        fixture
            .operation(&prepared.resource_id, Action::Release, "released")
            .await
            .status(),
        StatusCode::OK
    );
    fixture.context.code_buffers.shutdown().await;
}

#[tokio::test]
async fn changed_machine_connection_cannot_receive_an_old_resource() {
    let mut fixture = Fixture::new();
    let prepared = fixture.prepare(1).await;
    let (sender, mut replacement) = mpsc::unbounded_channel();
    fixture.context.machine_control.install(
        "machine".into(),
        "same-epoch".into(),
        false,
        19,
        sender,
    );
    let result = operate(
        fixture.context.clone(),
        prepared.resource_id.clone(),
        authenticated(),
        HeaderMap::new(),
        Action::Open,
    )
    .await;
    assert_eq!(result.status(), StatusCode::BAD_GATEWAY);
    let repeated = operate(
        fixture.context.clone(),
        prepared.resource_id.clone(),
        authenticated(),
        HeaderMap::new(),
        Action::Open,
    )
    .await;
    assert_eq!(json_response(repeated).await["state"], "unknown");
    assert_eq!(
        operate(
            fixture.context.clone(),
            prepared.resource_id,
            authenticated(),
            HeaderMap::new(),
            Action::Release
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    assert!(replacement.try_recv().is_err());
}

#[tokio::test]
async fn malformed_or_wrong_owner_open_replies_leave_queryable_evidence() {
    for value in [
        json!(null),
        native_reply(2, "open"),
        native_reply(1, "prepared"),
        json!({"type":"bufferLease", "api_version":2, "lease":native(1), "state":"open"}),
        json!({"type":"bufferLease", "api_version":1, "lease":native(1), "state":"open", "extra":true}),
    ] {
        let mut fixture = Fixture::new();
        let prepared = fixture.prepare(1).await;
        let context = fixture.context.clone();
        let reply_task = async {
            let command = fixture.commands.recv().await.unwrap();
            reply(&context, &fixture.connection, command, value);
        };
        let (response, ()) = tokio::join!(
            operate(
                context.clone(),
                prepared.resource_id.clone(),
                authenticated(),
                HeaderMap::new(),
                Action::Open
            ),
            reply_task
        );
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let repeated = operate(
            context,
            prepared.resource_id.clone(),
            authenticated(),
            HeaderMap::new(),
            Action::Open,
        )
        .await;
        assert_eq!(json_response(repeated).await["state"], "unknown");
        assert!(fixture.commands.try_recv().is_err());
        assert_eq!(
            json_response(
                fixture
                    .operation(&prepared.resource_id, Action::Query, "open")
                    .await
            )
            .await["state"],
            "open"
        );
        assert_eq!(
            fixture
                .operation(&prepared.resource_id, Action::Release, "released")
                .await
                .status(),
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn missing_release_receipt_is_observed_without_resending_the_release() {
    let mut fixture = Fixture::new();
    let prepared = fixture.prepare(1).await;
    fixture
        .operation(&prepared.resource_id, Action::Open, "open")
        .await;
    let context = fixture.context.clone();
    let reply_task = async {
        let command = fixture.commands.recv().await.unwrap();
        // An unparseable reply does not establish that the release failed.
        reply(&context, &fixture.connection, command, Value::Null);
    };
    let (response, ()) = tokio::join!(
        operate(
            context.clone(),
            prepared.resource_id.clone(),
            authenticated(),
            HeaderMap::new(),
            Action::Release
        ),
        reply_task
    );
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let repeated = operate(
        context,
        prepared.resource_id.clone(),
        authenticated(),
        HeaderMap::new(),
        Action::Release,
    )
    .await;
    assert_eq!(json_response(repeated).await["state"], "unknown");
    assert!(fixture.commands.try_recv().is_err());
    assert_eq!(
        json_response(
            fixture
                .operation(&prepared.resource_id, Action::Query, "released")
                .await
        )
        .await["state"],
        "released"
    );
}

#[tokio::test]
async fn only_the_exact_core_support_reply_allows_preparation() {
    for value in [
        Value::Null,
        json!({"type":"health", "buffer_lease_api":1}),
        json!({"type":"bufferLeaseSupport", "api_version":2}),
        json!({"type":"bufferLeaseSupport", "api_version":1, "worktree":"/other"}),
    ] {
        let mut fixture = Fixture::new();
        let context = fixture.context.clone();
        let principal = authenticated();
        let headers = HeaderMap::new();
        let task = async {
            let probe = fixture.commands.recv().await.unwrap();
            reply(&context, &fixture.connection, probe, value);
        };
        let (result, ()) = tokio::join!(
            prepare_inner(
                &context,
                &principal,
                &headers,
                PrepareRequest {
                    session_id: "session".into(),
                    path: "src/main.rs".into()
                }
            ),
            task
        );
        assert_eq!(result.unwrap_err(), StatusCode::NOT_IMPLEMENTED);
        assert!(fixture.commands.try_recv().is_err());
    }
}

#[tokio::test]
async fn support_probe_cannot_authorize_a_replacement_machine_or_retargeted_session() {
    for reconnect in [true, false] {
        let mut fixture = Fixture::new();
        let context = fixture.context.clone();
        let principal = authenticated();
        let headers = HeaderMap::new();
        let mut request = Box::pin(prepare_inner(
            &context,
            &principal,
            &headers,
            PrepareRequest {
                session_id: "session".into(),
                path: "src/main.rs".into(),
            },
        ));
        assert!(futures::poll!(&mut request).is_pending());
        let probe = fixture.commands.try_recv().unwrap();
        reply(
            &context,
            &fixture.connection,
            probe,
            json!({"type":"bufferLeaseSupport", "api_version":1}),
        );
        let (sender, mut replacement) = mpsc::unbounded_channel();
        if reconnect {
            context.machine_control.install(
                "machine".into(),
                "same-epoch".into(),
                false,
                19,
                sender,
            );
        } else {
            context
                .hub
                .update_session_cwd("session", "/changed".into())
                .unwrap();
        }
        assert!(request.await.is_err());
        assert!(fixture.commands.try_recv().is_err());
        assert!(replacement.try_recv().is_err());
    }
}

#[tokio::test]
async fn cancelled_preparation_and_invalid_paths_never_open_a_buffer() {
    let mut fixture = Fixture::new();
    let context = fixture.context.clone();
    let principal = authenticated();
    let headers = HeaderMap::new();
    let mut request = Box::pin(prepare_inner(
        &context,
        &principal,
        &headers,
        PrepareRequest {
            session_id: "session".into(),
            path: "src/main.rs".into(),
        },
    ));
    assert!(futures::poll!(&mut request).is_pending());
    let probe = fixture.commands.try_recv().unwrap();
    drop(request);
    reply(
        &context,
        &fixture.connection,
        probe,
        json!({"type":"bufferLeaseSupport", "api_version":1}),
    );
    for path in [
        "".into(),
        "../escape".into(),
        "/absolute".into(),
        "a\\b".into(),
        "a".repeat(4_097),
    ] {
        assert_eq!(
            prepare_inner(
                &context,
                &principal,
                &headers,
                PrepareRequest {
                    session_id: "session".into(),
                    path
                }
            )
            .await
            .unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }
    assert!(fixture.commands.try_recv().is_err());
    context.hub.delete_session("session");
    create(&context.hub, "local");
    assert_eq!(
        prepare_inner(
            &context,
            &principal,
            &headers,
            PrepareRequest {
                session_id: "session".into(),
                path: "a".into()
            }
        )
        .await
        .unwrap_err(),
        StatusCode::NOT_IMPLEMENTED
    );
}

#[test]
fn request_schemas_and_product_route_ownership_are_closed() {
    assert!(
        serde_json::from_value::<PrepareRequest>(json!({"sessionId":"session", "path":"file"}))
            .is_ok()
    );
    for value in [
        json!({"sessionId":"session", "path":"file", "lease":native(1)}),
        json!({"sessionId":"session", "path":"file", "machineId":"other"}),
        json!({"sessionId":"session"}),
    ] {
        assert!(serde_json::from_value::<PrepareRequest>(value).is_err());
    }
    assert!(serde_json::from_value::<Empty>(json!({})).is_ok());
    assert!(serde_json::from_value::<Empty>(json!({"path":"retarget"})).is_err());
    for method in [Method::GET, Method::POST, Method::PUT, Method::DELETE] {
        for path in [
            "/api/code/buffers",
            "/api/code/buffers/resource",
            "/api/code/buffers/resource/read",
        ] {
            assert_eq!(classify_route(&method, path), RouteAuth::ProductOperator);
        }
    }
}

#[tokio::test]
async fn http_routes_reject_retargeting_and_unbounded_bodies_without_native_calls() {
    struct Server(tokio::task::JoinHandle<()>);
    impl Drop for Server {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let mut fixture = Fixture::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    // This is a router/body contract fixture with explicit local product
    // authority, not real password, device-auth or production admission proof.
    let app = router()
        .with_state(fixture.context.clone())
        .layer(Extension(authenticated()));
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
    let response = client
        .post(format!("{base}/api/code/buffers"))
        .json(&json!({"sessionId":"session", "path":"file", "lease":native(1)}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = client
        .post(format!("{base}/api/code/buffers"))
        .json(&json!({"sessionId":"session", "path":"a".repeat(20_000)}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    for method in [Method::PUT, Method::DELETE] {
        let response = client
            .request(method.clone(), format!("{base}/api/code/buffers/unknown"))
            .json(&json!({"path":"retarget"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let response = client
            .request(method, format!("{base}/api/code/buffers/unknown"))
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    for (body, status) in [
        (
            json!({"kind":"language", "path":"retarget"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"kind":"hover", "offset":0}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"kind":"symbols", "path":"x".repeat(512)}),
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (json!({"kind":"language"}), StatusCode::NOT_FOUND),
        (
            serde_json::from_str::<Value>(include_str!(
                "../../../plugins/zed/adapter/fixtures/content.json"
            ))
            .unwrap()["request"]
                .clone(),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let response = client
            .post(format!("{base}/api/code/buffers/unknown/read"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    assert!(fixture.commands.try_recv().is_err());
}
