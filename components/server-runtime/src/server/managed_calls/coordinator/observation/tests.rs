use super::*;
use crate::core::{Hub, SessionOrigin, SessionRegistration};
use crate::machine_control::MachineControl;
use crate::machine_protocol::{MachineCommand, MachineEvent};
use crate::runtime_router::RuntimeRouter;
use tokio::sync::mpsc;

struct Fixture {
    hub: Hub,
    control: MachineControl,
    router: Arc<RuntimeRouter>,
    coordinator: Coordinator,
    connection: ConnectionToken,
    commands: mpsc::UnboundedReceiver<MachineCommand>,
}

impl Fixture {
    fn new() -> Self {
        let hub = Hub::new();
        let binding = crate::execution_environment::fixture();
        let mut behavior = crate::provider::legacy_behavior("claude-code");
        behavior.execution = Some(cowboy_provider_sdk::ExecutionBehavior::JsonrpcV1 {
            executor_digests: [binding.decode().unwrap().environment.executor_digest].into(),
        });
        hub.create_session(SessionRegistration {
            id: "parent".into(),
            provider: "claude-code".into(),
            provider_version: "1.0.0".into(),
            provider_generation_digest: "generation".into(),
            provider_account_id: Default::default(),
            provider_auth_generation: None,
            provider_behavior: Some(behavior),
            machine_id: "ovh".into(),
            workspace_id: Some("cowboy".into()),
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: Some(binding),
            cwd: "/runtime/session".into(),
            title: "fixture".into(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
        hub.sync_apply(
            "folders",
            "create".into(),
            "create",
            &json!({"id":"folder","name":"F","project":null}),
        )
        .unwrap();
        hub.sync_apply(
            "folders",
            "place".into(),
            "place",
            &json!({"session_ids":["parent"],"folder":"folder"}),
        )
        .unwrap();
        let control = MachineControl::default();
        let (tx, commands) = mpsc::unbounded_channel();
        let connection = control.install(
            "hawk".into(),
            "epoch".into(),
            false,
            code_observer::PROTOCOL_VERSION,
            tx,
        );
        let router = RuntimeRouter::new();
        let mut fixture = Self {
            hub,
            control,
            router,
            coordinator: Coordinator::default(),
            connection,
            commands,
        };
        fixture.worker("worker-one");
        let authority = fixture.context().authority("parent").unwrap();
        fixture.coordinator.grants.lock().insert(
            "parent".into(),
            GrantEntry {
                grant: Grant {
                    grant_id: "grant".into(),
                    parent_session_id: "parent".into(),
                    agent_api: true,
                    agent_actions: false,
                    mcp: false,
                },
                authority,
                mcp_epoch: None,
                installed_on: Some(fixture.connection.clone()),
                retry_after: None,
            },
        );
        fixture
    }

    fn context(&self) -> Context<'_> {
        Context {
            hub: &self.hub,
            machine_control: &self.control,
            runtime_router: &self.router,
            service_id: "service-test",
            managed_calls: &self.coordinator,
        }
    }

    fn scope(&self) -> SessionReadScope {
        self.control
            .session_read_scope(
                "service-test",
                self.hub.session_code_scope("parent").unwrap(),
            )
            .unwrap()
    }

    fn worker(&mut self, epoch: &str) {
        let parent = self.hub.session_info("parent").unwrap().meta;
        let worker = serde_json::from_value(json!({
            "session_id":parent.id,"worker_epoch":epoch,"generation":"worker-build","state":"busy",
            "agent_session_id":null,"current_turn_id":"turn","last_runtime_seq":1,"pending_permissions":[],
            "pending_prompt_count":0,"drain_requested":false,
            "launch":{"session_id":parent.id,"provider":parent.provider,"provider_version":parent.provider_version,
                "provider_generation_digest":parent.provider_generation_digest,"provider_behavior":parent.provider_behavior,
                "cwd":parent.cwd,"system":false,"generation":"worker-build","execution_binding":parent.execution_binding}
        })).unwrap();
        let runtime =
            crate::remote_runtime::RemoteRuntime::for_test(self.hub.clone(), vec![worker]);
        runtime.connect_for_test();
        self.router.install("ovh".into(), runtime);
    }
}

#[tokio::test]
async fn policy_aba_and_worker_replacement_end_the_original_continuation() {
    let mut fixture = Fixture::new();
    let original = reserve(&fixture.context(), &fixture.scope()).unwrap();
    assert!(original.current(&fixture.context()));
    for value in ["off", "on"] {
        fixture
            .hub
            .sync_apply(
                "folders",
                value.into(),
                "defaults",
                &json!({"id":"folder","values":{"language_servers":value}}),
            )
            .unwrap();
    }
    assert!(fixture.hub.language_servers_allowed("parent"));
    assert!(!original.current(&fixture.context()));
    assert!(reserve(&fixture.context(), &fixture.scope()).is_none());
    maintain(&fixture.context()).await;
    let replacement = reserve(&fixture.context(), &fixture.scope()).unwrap();
    assert_ne!(replacement.id, original.id);
    fixture.worker("worker-two");
    assert!(!replacement.current(&fixture.context()));
    assert!(fixture.commands.try_recv().is_err());
}

fn reply(
    control: &MachineControl,
    connection: &ConnectionToken,
    command: MachineCommand,
    response: Value,
) -> Value {
    let MachineCommand::AdapterRequest {
        request_id,
        payload,
        ..
    } = command
    else {
        panic!("unexpected command");
    };
    control.record_remote(
        connection,
        MachineEvent::AdapterResponse {
            request_id,
            accepted: true,
            payload: Some(response),
            detail: None,
            refusal: None,
        },
    );
    payload
}

#[tokio::test]
async fn lost_start_reply_retries_exactly_and_close_never_adopts_a_reconnection() {
    let mut fixture = Fixture::new();
    let context = Context {
        hub: &fixture.hub,
        machine_control: &fixture.control,
        runtime_router: &fixture.router,
        service_id: "service-test",
        managed_calls: &fixture.coordinator,
    };
    let entry = reserve(&context, &fixture.scope()).unwrap();
    let mut start = Box::pin(entry.start(&context));
    for response in [
        json!({"incarnation":"b".repeat(32)}),
        json!({"current":true}),
    ] {
        let command = tokio::select! { result = &mut start => panic!("early result: {result:?}"), command = fixture.commands.recv() => command.unwrap() };
        reply(&fixture.control, &fixture.connection, command, response);
    }
    let first = tokio::select! { result = &mut start => panic!("early result: {result:?}"), command = fixture.commands.recv() => command.unwrap() };
    let MachineCommand::AdapterRequest { payload: first, .. } = first else {
        panic!("expected Start");
    };
    drop(start); // reply lost after dispatch
    let mut retry = Box::pin(entry.start(&context));
    let verify = tokio::select! { result = &mut retry => panic!("early result: {result:?}"), command = fixture.commands.recv() => command.unwrap() };
    let verify = reply(
        &fixture.control,
        &fixture.connection,
        verify,
        json!({"current":true}),
    );
    assert_eq!(verify["action"], "verify");
    let next = tokio::select! { result = &mut retry => panic!("early result: {result:?}"), command = fixture.commands.recv() => command.unwrap() };
    assert_eq!(
        reply(
            &fixture.control,
            &fixture.connection,
            next,
            json!({"state":"started"})
        ),
        first
    );
    retry.await.unwrap();
    entry.retiring.store(true, Ordering::Release);
    let mut close = Box::pin(retire(&context, &entry));
    let command = tokio::select! { result = &mut close => panic!("early close: {result:?}"), command = fixture.commands.recv() => command.unwrap() };
    let closed = reply(
        &fixture.control,
        &fixture.connection,
        command,
        json!({"state":"closing"}),
    );
    assert_eq!(closed["continuation"], first["continuation"]);
    assert!(!close.await);
    let (tx, mut new_commands) = mpsc::unbounded_channel();
    let _new = fixture.control.install(
        "hawk".into(),
        "epoch".into(),
        false,
        code_observer::PROTOCOL_VERSION,
        tx,
    );
    assert!(retire(&context, &entry).await);
    assert!(!entry.current(&context));
    assert!(new_commands.try_recv().is_err());
}
