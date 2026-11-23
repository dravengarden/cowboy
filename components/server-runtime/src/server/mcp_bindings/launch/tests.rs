use super::super::tests::{binding, fixture, update};
use super::*;
use crate::{
    core::Hub,
    machine_control::MachineControl,
    machine_protocol::{MachineCommand, MachineEvent},
    mcp_binding::TransportRequest,
};
use serde_json::json;
use tokio::sync::{mpsc, watch};

struct Fixture {
    hub: Hub,
    meta: SessionMeta,
    policies: super::super::Policies,
    store: crate::store::Store,
    root: tempfile::TempDir,
    control: Arc<MachineControl>,
    execution: ConnectionToken,
    commands: tokio::sync::Mutex<mpsc::UnboundedReceiver<MachineCommand>>,
    _runtime_commands: mpsc::UnboundedReceiver<MachineCommand>,
    runtimes: Arc<crate::runtime_router::RuntimeRouter>,
    service: String,
    shutdown: watch::Receiver<bool>,
    _shutdown: watch::Sender<bool>,
    epoch: String,
}
impl Fixture {
    async fn new() -> Self {
        let (_, mut meta, policies, store, root) = fixture().await;
        let binding = crate::execution_environment::fixture();
        let decoded = binding.decode().unwrap();
        let mut behavior = crate::provider::legacy_behavior("codex");
        behavior.execution = Some(cowboy_provider_sdk::ExecutionBehavior::JsonrpcV1 {
            executor_digests: [decoded.environment.executor_digest.clone()].into(),
        });
        meta.provider_behavior = Some(behavior);
        meta.provider_version = "1.0.0".into();
        meta.provider_generation_digest = format!("sha256:{}", "a".repeat(64));
        meta.machine_id = decoded.runtime.machine_id.clone();
        meta.cwd = decoded.runtime.cwd;
        meta.workspace_id = Some(decoded.workspace.id);
        meta.execution_binding = Some(binding);
        meta.require_runtime_launch().unwrap();
        let hub = Hub::new();
        hub.project_persisted_session(meta.clone());
        let stored = policies
            .replace(
                &hub,
                &store,
                &meta,
                update(None, vec![super::super::tests::binding()]),
            )
            .await
            .unwrap();
        let identity =
            crate::service_identity::load_or_create(&root.path().join("service")).unwrap();
        let service = identity.as_str().to_owned();
        let control = Arc::new(MachineControl::new(identity));
        let (tx, commands) = mpsc::unbounded_channel();
        let execution = control.install(
            "hawk".into(),
            "epoch".into(),
            false,
            crate::machine_protocol::MCP_NATIVE_CONFIRMATION_PROTOCOL_VERSION,
            tx,
        );
        let (tx, runtime_commands) = mpsc::unbounded_channel();
        control.install(
            "ovh".into(),
            "epoch".into(),
            false,
            crate::machine_protocol::MCP_NATIVE_CONFIRMATION_PROTOCOL_VERSION,
            tx,
        );
        let runtimes = crate::runtime_router::RuntimeRouter::new();
        let runtime = crate::remote_runtime::RemoteRuntime::for_test(hub.clone(), vec![]);
        runtime.connect_for_test();
        runtimes.install("ovh".into(), runtime);
        let (shutdown_tx, shutdown) = watch::channel(false);
        Self {
            hub,
            meta,
            policies,
            store,
            root,
            control,
            execution,
            commands: tokio::sync::Mutex::new(commands),
            _runtime_commands: runtime_commands,
            runtimes,
            service,
            shutdown,
            _shutdown: shutdown_tx,
            epoch: stored.epoch,
        }
    }
    fn context(&self) -> Context<'_> {
        Context {
            hub: &self.hub,
            policies: &self.policies,
            control: &self.control,
            runtimes: &self.runtimes,
            service: &self.service,
            shutdown: &self.shutdown,
        }
    }
    async fn respond(&self) -> Result<()> {
        let mut commands = self.commands.lock().await;
        let MachineCommand::Execution { request_id, .. } = commands.recv().await.unwrap() else {
            panic!("describe")
        };
        let scope = crate::execution_protocol::Scope::from_binding(
            &self
                .meta
                .execution_binding
                .as_ref()
                .unwrap()
                .decode()
                .unwrap(),
        );
        self.control.record_remote(&self.execution, MachineEvent::ExecutionResponse { request_id, response: Box::new(crate::machine_protocol::execution::Response::Call { response: crate::execution_protocol::Response::Ready {
            scope, initialization: json!({"cowboyAgentCli":{"command":"/nix/store/fixture/bin/cowboy","sha256":"a".repeat(64),"version":"1.0.0"}}), event_cursor: 0,
        }}) });
        let MachineCommand::McpTransport {
            request_id,
            request: TransportRequest::PrepareNative { profile },
        } = commands.recv().await.unwrap()
        else {
            panic!("prepare")
        };
        self.control.record_remote(&self.execution, MachineEvent::McpTransportResponse { request_id, response: json!({"handle":format!("native-{}", "e".repeat(32)),"path":"/private/suger.json","descriptor":{"schema":1,"binding":profile.binding,"epoch":profile.epoch,"server_version":"1.0.0","tools":[{"name":"suger_status","description":"Status","inputSchema":{"type":"object"}},{"name":"suger_read","description":"Read","inputSchema":{"type":"object"}}]}}) });
        let MachineCommand::McpTransport {
            request_id,
            request: TransportRequest::ConfirmNative { .. },
        } = commands.recv().await.unwrap()
        else {
            panic!("confirm")
        };
        self.control.record_remote(
            &self.execution,
            MachineEvent::McpTransportResponse {
                request_id,
                response: json!({"confirmed":true}),
            },
        );
        Ok(())
    }
    fn worker(&self, plan: &crate::mcp_binding::NativeLaunchPlan, epoch: &str) {
        let runtime = self.runtime(plan, epoch);
        runtime.connect_for_test();
        runtime
            .mcp_launch_authority(&self.service, &self.meta, plan)
            .unwrap();
    }
    /// Install a not-yet-connected runtime that reports this worker.
    fn runtime(
        &self,
        plan: &crate::mcp_binding::NativeLaunchPlan,
        epoch: &str,
    ) -> Arc<crate::remote_runtime::RemoteRuntime> {
        let snapshot = serde_json::from_value(json!({"session_id":self.meta.id,"worker_epoch":epoch,"generation":"gen","state":"running","launch":{
            "session_id":self.meta.id,"provider":self.meta.provider,"provider_version":self.meta.provider_version,"provider_generation_digest":self.meta.provider_generation_digest,
            "provider_behavior":self.meta.provider_behavior,"cwd":self.meta.cwd,"system":false,"generation":"gen","execution_binding":self.meta.execution_binding,"mcp_plan":plan
        }})).unwrap();
        let runtime =
            crate::remote_runtime::RemoteRuntime::for_test(self.hub.clone(), vec![snapshot]);
        self.runtimes.install("ovh".into(), Arc::clone(&runtime));
        runtime
    }
}

#[tokio::test]
async fn session_owner_survives_preparation_and_releases_on_revocation() {
    let f = Fixture::new().await;
    let launches = Launches::default();
    let context = f.context();
    let result = tokio::try_join!(
        launches.prepare_context(&context, &f.meta.id, &f.epoch),
        f.respond()
    );
    result.unwrap();
    assert!(launches.resolve(&context, &f.meta).is_ok());
    let mut stale = f.meta.clone();
    stale.provider_version = "old".into();
    assert!(launches.resolve(&context, &stale).is_err());
    assert!(f.commands.lock().await.try_recv().is_err());
    let revoked = f
        .policies
        .replace(
            &f.hub,
            &f.store,
            &f.meta,
            update(Some(f.epoch.clone()), vec![]),
        )
        .await
        .unwrap();
    f.policies
        .replace(
            &f.hub,
            &f.store,
            &f.meta,
            update(Some(revoked.epoch), vec![binding()]),
        )
        .await
        .unwrap();
    assert!(launches.resolve(&context, &f.meta).is_err());
    launches.sweep(&context);
    assert!(launches.entries.lock().is_empty());
    assert!(matches!(
        f.commands.lock().await.try_recv().unwrap(),
        MachineCommand::McpTransport {
            request: TransportRequest::ReleaseNative { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn revocation_during_keeper_read_cancels_preparation() {
    let f = Fixture::new().await;
    let launches = Launches::default();
    let context = f.context();
    let revoke = async {
        assert!(matches!(
            f.commands.lock().await.recv().await.unwrap(),
            MachineCommand::Execution { .. }
        ));
        f.policies
            .replace(
                &f.hub,
                &f.store,
                &f.meta,
                update(Some(f.epoch.clone()), vec![]),
            )
            .await
            .unwrap();
        Ok::<(), anyhow::Error>(())
    };
    let result = tokio::try_join!(
        launches.prepare_context(&context, &f.meta.id, &f.epoch),
        revoke
    );
    assert!(result.is_err());
    assert!(launches.entries.lock().is_empty());
    assert!(f.commands.lock().await.try_recv().is_err());
}

#[tokio::test]
async fn worker_replacement_cannot_reuse_a_ready_launch_owner() {
    let f = Fixture::new().await;
    let launches = Launches::default();
    let context = f.context();
    let result = tokio::try_join!(
        launches.prepare_context(&context, &f.meta.id, &f.epoch),
        f.respond()
    );
    result.unwrap();
    let plan = launches.resolve(&context, &f.meta).unwrap();
    f.worker(&plan, "worker-one");
    launches.resolve(&context, &f.meta).unwrap();
    f.worker(&plan, "worker-two");
    assert!(launches.resolve(&context, &f.meta).is_err());
    launches.sweep(&context);
    assert!(launches.entries.lock().is_empty());
}

#[tokio::test]
async fn runtime_reconnect_keeps_the_plan_of_a_surviving_worker() {
    let f = Fixture::new().await;
    let launches = Launches::default();
    let context = f.context();
    tokio::try_join!(
        launches.prepare_context(&context, &f.meta.id, &f.epoch),
        f.respond()
    )
    .unwrap();
    let plan = launches.resolve(&context, &f.meta).unwrap();
    f.worker(&plan, "worker-one");
    launches.resolve(&context, &f.meta).unwrap();
    let kept = |launches: &Launches| {
        launches.sweep(&context);
        assert_eq!(launches.entries.lock().len(), 1);
        assert_eq!(launches.resolve(&context, &f.meta).unwrap(), plan);
    };
    // WebSocket teardown: the runtime leaves the router and the Machine
    // control connection is replaced.
    let old = f.runtimes.runtime("ovh").unwrap();
    f.runtimes.remove_if_current("ovh", &old);
    let (tx, _replacement) = mpsc::unbounded_channel();
    f.control.install(
        "ovh".into(),
        "epoch".into(),
        false,
        crate::machine_protocol::MCP_NATIVE_CONFIRMATION_PROTOCOL_VERSION,
        tx,
    );
    kept(&launches);
    // The next runtime is installed while still connecting.
    let next = f.runtime(&plan, "worker-one");
    kept(&launches);
    // A broker registry placeholder hides the same worker without replacing it.
    next.disconnect_for_test();
    let placeholder = f.runtime(&plan, "broker-placeholder");
    placeholder.connect_for_test();
    kept(&launches);
    let next = f.runtime(&plan, "worker-one");
    next.connect_for_test();
    kept(&launches);
    assert!(launches.ready_authority(&context, &f.meta).is_ok());
    assert!(f.commands.lock().await.try_recv().is_err());
    // A connected runtime without that worker is a real change.
    let empty = crate::remote_runtime::RemoteRuntime::for_test(f.hub.clone(), vec![]);
    empty.connect_for_test();
    f.runtimes.install("ovh".into(), empty);
    assert!(launches.resolve(&context, &f.meta).is_err());
}

#[tokio::test]
async fn controller_hook_delivers_owned_plan_and_refuses_connection_replacement() {
    let f = Arc::new(Fixture::new().await);
    let launches = Arc::new(Launches::default());
    let context = f.context();
    let result = tokio::try_join!(
        launches.prepare_context(&context, &f.meta.id, &f.epoch),
        f.respond()
    );
    result.unwrap();
    let weak = Arc::downgrade(&f);
    let owners = Arc::downgrade(&launches);
    f.hub.set_mcp_launch_resolver(Arc::new(move |meta| {
        let f = weak.upgrade().context("fixture stopped")?;
        owners
            .upgrade()
            .context("owners stopped")?
            .resolve(&f.context(), meta)
    }));
    let stored = f.hub.setting(&super::super::key(&f.meta.id));
    assert!(super::super::launch_mcp(&f.hub, &f.meta).unwrap().is_some());
    assert_eq!(f.hub.setting(&super::super::key(&f.meta.id)), stored);
    let (tx, mut replacement) = mpsc::unbounded_channel();
    f.control.install(
        "hawk".into(),
        "epoch".into(),
        false,
        crate::machine_protocol::MCP_NATIVE_CONFIRMATION_PROTOCOL_VERSION,
        tx,
    );
    assert!(super::super::launch_mcp(&f.hub, &f.meta).is_err());
    launches.sweep(&context);
    assert!(launches.entries.lock().is_empty());
    assert!(replacement.try_recv().is_err());
}

#[tokio::test]
async fn fresh_supervisor_launch_carries_the_owned_plan_and_extensions() {
    crate::install_controller_hooks();
    let f = Arc::new(Fixture::new().await);
    let launches = Arc::new(Launches::default());
    let context = f.context();
    tokio::try_join!(
        launches.prepare_context(&context, &f.meta.id, &f.epoch),
        f.respond()
    )
    .unwrap();
    let weak = Arc::downgrade(&f);
    let owners = Arc::downgrade(&launches);
    f.hub.set_mcp_launch_resolver(Arc::new(move |meta| {
        let f = weak.upgrade().context("fixture stopped")?;
        owners
            .upgrade()
            .context("owners stopped")?
            .resolve(&f.context(), meta)
    }));
    let supervisor = crate::supervisor::Supervisor::new(
        f.hub.clone(),
        f.root.path().into(),
        0,
        Arc::clone(&f.runtimes),
    );
    supervisor.start_registered_session(&f.meta.id).unwrap();
    let runtime = f.runtimes.runtime("ovh").unwrap();
    let queued = runtime.pending_for_test();
    let launch = queued
        .iter()
        .find_map(|command| match command {
            crate::runtime_wire::CoreCommand::EnsureSession { session } => Some(session),
            _ => None,
        })
        .expect("fresh launch queued");
    assert_eq!(
        launch.mcp_plan,
        Some(launches.resolve(&context, &f.meta).unwrap())
    );
    assert!(launch.extensions.is_some());
    assert!(f.commands.lock().await.try_recv().is_err());
}

#[tokio::test]
async fn recovery_rebuilds_owner_without_rewriting_policy_and_reuses_it() {
    let f = Fixture::new().await;
    let launches = Launches::default();
    let context = f.context();
    let before = Stored::load(&f.hub, &f.meta.id).unwrap();
    let mut cursor = String::new();
    tokio::try_join!(launches.recover_next(&context, &mut cursor), f.respond()).unwrap();
    assert!(launches.resolve(&context, &f.meta).is_ok());
    launches.recover_next(&context, &mut cursor).await.unwrap();
    launches
        .prepare_context(&context, &f.meta.id, &f.epoch)
        .await
        .unwrap();
    assert_eq!(Stored::load(&f.hub, &f.meta.id).unwrap(), before);
    assert!(f.commands.lock().await.try_recv().is_err());
}

#[tokio::test]
async fn recovery_skips_revoked_policy_and_serializes_source_preparation() {
    let f = Fixture::new().await;
    let launches = Launches::default();
    let context = f.context();
    let guard = launches.preparation.lock().await;
    assert!(
        launches
            .recover_next(&context, &mut String::new())
            .await
            .is_err()
    );
    drop(guard);
    f.policies
        .replace(
            &f.hub,
            &f.store,
            &f.meta,
            update(Some(f.epoch.clone()), vec![]),
        )
        .await
        .unwrap();
    launches
        .recover_next(&context, &mut String::new())
        .await
        .unwrap();
    assert!(launches.entries.lock().is_empty());
    assert!(f.commands.lock().await.try_recv().is_err());
}

#[tokio::test]
async fn recovery_advances_past_a_session_that_cannot_launch() {
    let f = Fixture::new().await;
    let mut blocked = f.meta.clone();
    blocked.id = "000-unavailable".into();
    blocked.provider_version.clear();
    f.hub.project_persisted_session(blocked.clone());
    f.policies
        .replace(&f.hub, &f.store, &blocked, update(None, vec![binding()]))
        .await
        .unwrap();
    let launches = Launches::default();
    let context = f.context();
    let mut cursor = String::new();
    assert!(launches.recover_next(&context, &mut cursor).await.is_err());
    assert_eq!(cursor, blocked.id);
    tokio::try_join!(launches.recover_next(&context, &mut cursor), f.respond()).unwrap();
    assert!(launches.resolve(&context, &f.meta).is_ok());
}

#[tokio::test]
async fn readiness_requires_a_current_owner_and_exact_worker() {
    let f = Fixture::new().await;
    let launches = Launches::default();
    let context = f.context();
    assert!(launches.ready_authority(&context, &f.meta).is_err());
    tokio::try_join!(
        launches.prepare_context(&context, &f.meta.id, &f.epoch),
        f.respond()
    )
    .unwrap();
    assert!(launches.ready_authority(&context, &f.meta).is_err());
    let plan = launches.resolve(&context, &f.meta).unwrap();
    f.worker(&plan, "worker-one");
    assert!(launches.ready_authority(&context, &f.meta).is_ok());
    f.worker(&plan, "worker-two");
    assert!(launches.ready_authority(&context, &f.meta).is_err());
}

#[tokio::test]
async fn grant_epoch_requires_exact_authority_and_never_survives_revocation() {
    let f = Fixture::new().await;
    let launches = Launches::default();
    let context = f.context();
    tokio::try_join!(
        launches.prepare_context(&context, &f.meta.id, &f.epoch),
        f.respond()
    )
    .unwrap();
    let plan = launches.resolve(&context, &f.meta).unwrap();
    f.worker(&plan, "worker-one");
    let authority = launches.ready_authority(&context, &f.meta).unwrap();
    assert_eq!(
        launches.grant_epoch_context(&context, &f.meta, &authority),
        Some(f.epoch.clone())
    );
    let revoked = f
        .policies
        .replace(
            &f.hub,
            &f.store,
            &f.meta,
            update(Some(f.epoch.clone()), vec![]),
        )
        .await
        .unwrap();
    assert!(
        launches
            .grant_epoch_context(&context, &f.meta, &authority)
            .is_none()
    );
    f.policies
        .replace(
            &f.hub,
            &f.store,
            &f.meta,
            update(Some(revoked.epoch), vec![binding()]),
        )
        .await
        .unwrap();
    assert!(
        launches
            .grant_epoch_context(&context, &f.meta, &authority)
            .is_none()
    );
}
