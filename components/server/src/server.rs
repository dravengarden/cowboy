//! HTTP / WebSocket server composition and Agent API surface.
//!
//! Runtime state, session/machine handlers, managed calls, and MCP bindings
//! live in `cowboy-server-runtime` and are re-exported here so `routes.rs`,
//! `serve()`, and `agent_api` compile in parallel via Cargo pipelining.

pub use cowboy_server_runtime::server::*;

use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use cowboy_server_agent_api::server::agent_api;

/// Start the HTTP/WebSocket server and the agent supervisor.
pub async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    let runtime_restrictions = Arc::new(crate::project_placement::RuntimeRestrictions::parse(
        &args.provider_runtime_machines,
    )?);
    if args.check_plugin_catalog {
        return print_plugin_catalog_preflight(&args);
    }
    let (plugin_catalog, product_authentication, core_security) =
        crate::plugin_activation::prepare_controller_hosts(&args)?;
    if args.check_plugin_hosts {
        let mut report = plugin_catalog.host_preflight_report()?;
        report.telemetry = Some(crate::telemetry_plugin::controller_policy::inspect(&args)?);
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    anyhow::ensure!(
        args.product_auth_enabled,
        "Device authentication is mandatory; authentication cannot be disabled"
    );
    // An invalid Service configuration refuses startup before any state is
    // touched, so a rolling update keeps the previous instance serving.
    let service_config_path = crate::config::Scope::Service.path_in(&args.data_dir);
    let service_config = crate::config::Handle::new(crate::config::load(
        crate::config::Scope::Service,
        &service_config_path,
    )?);
    // Lives for the whole process; it only reads the file and swaps values.
    let _service_config_watch =
        service_config.watch(crate::config::Scope::Service, service_config_path.clone());
    plugin_catalog.initialize()?;
    let plugin_dir =
        crate::plugin_dir::PluginDir::open(&args.data_dir).context("opening plugin directory")?;
    let _security_owner = crate::core_security::ControllerLock::acquire(&plugin_dir)?;
    let plugin_catalog = Arc::new(plugin_catalog);
    let product_authentication = Arc::new(product_authentication);
    let service_identity = crate::service_identity::load_or_create(&args.data_dir)
        .context("loading Cowboy Service identity")?;
    let service_id = service_identity.as_str().to_owned();
    let crate::telemetry_plugin::controller_policy::ControllerPolicy {
        writer: telemetry_writer_admission,
        background: managed_export_policy,
    } = crate::telemetry_plugin::controller_policy::ControllerPolicy::load(&args, &service_id)?;
    let desired_machine_components =
        Arc::new(crate::machine_convergence::DesiredComponentSource::load(
            args.machine_components_manifest.as_deref(),
            now_ms(),
        )?);
    let component_convergence = Arc::new(crate::machine_convergence::ConvergenceState::default());
    let convergence_freeze = Arc::new(crate::machine_convergence::ConvergenceFreeze::new(
        &args.data_dir,
    ));
    let desired_machine_plugins = Arc::new(crate::machine_convergence::PluginDesiredSource::new(
        &args.data_dir,
    ));
    init_tracing();
    let log_directory = crate::logs::directory(&args.data_dir);
    let log_guard = crate::logs::init(
        log_directory.clone(),
        crate::logs::Context::new("cowboy-controller"),
    )?
    .track_outcome();
    if args.cardea_oidc_config.is_some() && !args.product_auth_enabled {
        tracing::warn!("Cardea OIDC is configured but product authentication is disabled");
    }
    if args.auth_config.is_some() && !args.product_auth_enabled {
        tracing::warn!(
            "authentication Plugins are configured but product authentication is disabled"
        );
    }
    log_runtime_configuration();
    let code_cache =
        crate::code_cache::CodeCache::open(args.data_dir.join("code-cache"), args.code_cache_bytes)
            .map_err(anyhow::Error::msg)
            .context("opening code content cache")?;
    let web_push = WebPushService::open(&args.data_dir).context("opening Web Push service")?;
    let provider_catalog = Arc::new(crate::provider_catalog::ProviderCatalog::open(
        &args.data_dir,
        Arc::clone(&plugin_catalog),
    )?);
    let provider_auth = provider_accounts::retirement::open_provider_auth(&args.data_dir)?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    // Persistence closes only after dispatcher/runtime quiescence. Using the
    // HTTP shutdown signal directly would race a last-millisecond prompt
    // requeue against the writer closing its receiver.
    let (store_shutdown_tx, store_shutdown_rx) = watch::channel(false);
    let runtime_health = Arc::new(RuntimeHealth::default());
    // Durable admission fences are restored before any writer, sweeper,
    // Plugin host or runtime starts (see open_persistence).
    let Persistence {
        hub,
        mut store,
        health: persistence_health,
        writer_task,
        purge_task,
        session_id_floor,
        plugin_lifecycle_fences,
        telemetry_binding_fence,
        managed_export_activation,
    } = open_persistence(
        &args,
        core_security.as_ref(),
        &plugin_dir,
        (&service_id, &provider_auth),
        managed_export_policy,
        &runtime_health,
        &store_shutdown_rx,
    )
    .await?;
    let plugin_storage = match store.as_ref() {
        Some(store) => store.plugin_storage(plugin_dir.clone()),
        None => crate::plugin_storage::PluginStorage::sqlite_files(plugin_dir.clone()),
    };
    let plugin_runtime = match plugin_catalog.activate_runtime(&plugin_storage).await {
        Ok(runtime) => runtime,
        Err(error) if plugin_catalog.strict_host_activation() => {
            return Err(error).context("activating selected Plugin hosts");
        }
        Err(error) => {
            tracing::error!(%error, "plugin host activation failed");
            plugin_catalog.install_empty_runtime()
        }
    };
    if let Some(store) = store.as_mut().filter(|_| core_security.is_none()) {
        let namespace = plugin_runtime
            .namespace_for_capability(crate::core_passkeys::STORAGE_CAPABILITY)
            .context("WebAuthn plugin storage is unavailable")?;
        store
            .attach_passkey_storage(namespace)
            .await
            .context("binding core Passkey storage at the legacy namespace")?;
    }
    let machine_control = Arc::new(MachineControl::new(service_identity));
    let usage = UsageService::with_plugin_catalog(
        store.clone(),
        Some(args.data_dir.join("usage-snapshot.json")),
        Arc::clone(&plugin_catalog),
        Arc::clone(&machine_control),
    )
    .with_config(service_config.clone())
    .with_runtime_restrictions(Arc::clone(&runtime_restrictions))
    .with_retired_service_accounts(crate::store::provider_accounts::legacy_cards::CARDS);
    usage.restore_execution_settings().await?;
    let runtime_router = RuntimeRouter::new();
    // Usage collectors and authenticated Machine connections must share this
    // registry; a second instance leaves collectors permanently without routes.
    // Built here rather than through a constructor: every field is a clone of
    // something this scope already owns, and the list is long enough that
    // naming each one at the call site reads better than an argument order.
    let machine_snapshots = MachineSnapshots {
        store: store.clone(),
        hub: hub.clone(),
        runtime_router: Arc::clone(&runtime_router),
        machine_control: Arc::clone(&machine_control),
        desired_components: Arc::clone(&desired_machine_components),
        component_convergence: Arc::clone(&component_convergence),
        convergence_freeze: Arc::clone(&convergence_freeze),
        desired_plugins: Arc::clone(&desired_machine_plugins),
        product_auth_enabled: args.product_auth_enabled,
        revision: Arc::new(AtomicU64::new(0)),
        service_config: service_config.clone(),
    };
    let machine_presence_task =
        spawn_machine_presence_sweeper(store.as_ref(), &machine_snapshots, &shutdown_rx);
    restore_provider_reset_schedules(&usage, store.as_ref()).await?;
    let reset_task = spawn_provider_reset_timer(usage.clone(), store.clone(), shutdown_rx.clone());
    let supervisor = Arc::new(
        Supervisor::new(
            hub.clone(),
            args.workspace_root.clone(),
            session_id_floor,
            Arc::clone(&runtime_router),
        )
        .with_runtime_restrictions(runtime_restrictions),
    );
    let observability = Observability::start_with_wait_cache(
        store.clone(),
        crate::logs::Fanout(local_evidence_sinks(&args, log_directory.clone())?),
        match managed_export_activation {
            Some(crate::telemetry_plugin::background_policy::Activation::Active(policy)) => {
                Some(telemetry_binding::background::exporter(
                    policy,
                    store
                        .clone()
                        .context("managed telemetry requires a durable store")?,
                    Arc::clone(&machine_control),
                    Arc::clone(&plugin_catalog),
                    Arc::clone(&plugin_lifecycle_fences),
                ))
            }
            Some(crate::telemetry_plugin::background_policy::Activation::Stopped) => None,
            None => crate::telemetry_plugin::controller_exporter(
                args.telemetry_plugin_config.as_deref(),
                Arc::clone(&machine_control),
                Arc::clone(&plugin_catalog),
                telemetry_binding_fence.clone(),
            )?,
        },
        Some(log_directory),
    );
    let telemetry_shutdown = observability.clone();

    // Background dispatcher: the Hub owns each session's send-queue but can't
    // call the Supervisor (which holds the Hub) — that cycle is why the queue
    // used to live client-side. The Hub now makes the drain decision under its
    // lock and hands each ready prompt over this channel; we send it to the
    // agent here, off the lock. Wired before any client connects.
    let (dispatch_tx, dispatch_rx) = mpsc::channel::<DispatchReq>(1_024);
    hub.set_dispatch_tx(dispatch_tx);

    let scheduler_task = start_scheduler(&hub, store.as_ref(), &runtime_health).await;

    let runtime_reconciliation_task =
        spawn_runtime_reconciliation(&hub, &machine_control, &shutdown_rx);

    tracing::info!(
        workspace = %args.workspace_root.display(),
        data_dir = %args.data_dir.display(),
        plugin_dir = %plugin_dir.root().display(),
        plugin_storage = ?plugin_storage.kind(),
        "cowboy serving",
    );

    // Start only after runtime/storage setup. The observer owns no install
    // authority, stops on the HTTP signal and drains before storage teardown.
    let catalog_watcher = crate::plugin_catalog::spawn_catalog_watcher(
        Arc::clone(&plugin_catalog),
        plugin_storage.clone(),
        Arc::clone(&provider_catalog),
        shutdown_rx.clone(),
    );
    let state = AppState::build(state::AppStateInputs {
        service_id,
        service_config_path,
        service_config,
        hub,
        supervisor,
        store,
        persistence_health,
        shutdown: shutdown_rx,
        runtime_health,
        runtime_router,
        machine_control,
        machine_snapshots,
        plugin_catalog,
        plugin_storage,
        provider_catalog,
        provider_auth,
        telemetry_binding_fence,
        telemetry_writer_admission,
        plugin_lifecycle_fences,
        desired_machine_components,
        component_convergence,
        convergence_freeze,
        desired_machine_plugins,
        web_root: args.web_root,
        usage,
        code_cache,
        code_navigation_admission: args.code_navigation_admission
            == crate::serve_args::CodeNavigationAdmission::Candidate,
        project_placement: crate::project_placement::Store::new(
            &args.data_dir,
            args.execution_runtime_machine,
        )?,
        colocated_machines: args.colocated_machines.iter().cloned().collect(),
        zed_adapter_socket: args.zed_adapter_socket,
        observability,
        web_push,
        product_auth_enabled: args.product_auth_enabled,
        product_authentication,
        agent_api: Arc::new(agent_api::Bridge),
    });
    let web_push_task = spawn_notifications(&state);
    let dispatcher_task = spawn_dispatcher(&state, dispatch_rx);
    let code_holdings_task = tokio::spawn(agent_api::maintain_holdings(Arc::clone(&state)));

    let (runtime_router, hub) = (Arc::clone(&state.runtime_router), state.hub.clone());
    let result = serve_axum(args.bind, args.data_dir.clone(), state, shutdown_tx).await;
    code_holdings_task.abort();
    let _ = code_holdings_task.await;
    // Persistence closes below; events consumed after this point would be
    // acknowledged but never stored.
    runtime_router.disconnect_all();
    if let Err(error) = catalog_watcher.shutdown().await {
        tracing::error!(%error, "Catalog observer failed during shutdown");
    }
    ControllerTasks {
        hub,
        scheduler_task,
        reset_task,
        runtime_reconciliation_task,
        web_push_task,
        dispatcher_task,
        purge_task,
        machine_presence_task,
        telemetry_shutdown,
        store_shutdown_tx,
        writer_task,
    }
    .shutdown()
    .await;
    log_guard.finish(result)
}

async fn serve_axum(
    bind: std::net::SocketAddr,
    data_dir: PathBuf,
    state: Arc<AppState>,
    shutdown_tx: watch::Sender<bool>,
) -> anyhow::Result<()> {
    provider_accounts::browser::install_observer(&state.accounts);
    session_maintenance::start(&state);
    session_maintenance::crashed_workers::start(&state);
    session_reclaim::listen_for_wakes(&state);
    memory_reclaim::start(&state);
    let secure_transport = Arc::new(secure_transport::SecureTransport::new(
        state
            .store
            .clone()
            .context("Device authentication requires durable storage")?,
        state.public_origins.as_ref().clone(),
    )?);
    execution::sessions::start_recovery(&state);
    workspace_preparation::start_recovery(&state);
    managed_calls::Coordinator::start(&state);
    if let Some(store) = state.store.clone() {
        let hub = state.hub.clone();
        tokio::spawn(async move { background_tasks::repair_call_anchors(&hub, &store).await });
    }
    let setup = prepare_admin_setup(&state, data_dir.clone()).await?;
    let auth_state = ProductAuthState {
        hub: state.hub.clone(),
        store: state.store.clone(),
        rate_limits: Arc::new(crate::product_auth::AuthRateLimiter::default()),
        public_origins: state.public_origins.clone(),
        runtime_health: Some(state.runtime_health.clone()),
        persistence_health: state.persistence_health.clone(),
        runtime_router: Some(state.runtime_router.clone()),
        plugin_catalog: Some(state.plugin_catalog.clone()),
        plugin_storage: Some(state.plugin_storage.clone()),
        provider_catalog: Some(state.provider_catalog.clone()),
        passkeys: Arc::new(crate::passkey::PasskeyCeremonies::default()),
        setup,
        setup_lock: Arc::new(tokio::sync::Mutex::new(())),
        product_auth_enabled: state.product_auth_enabled,
        product_authentication: state.product_authentication.clone(),
        oidc_transactions: state.oidc_transactions.clone(),
        oidc_native_handoffs: state.oidc_native_handoffs.clone(),
        device_authorizations: state.device_authorizations.clone(),
        device_access: state.device_access.clone(),
        capacity_runtime: state.capacity_runtime.clone(),
    };

    let app = Router::new()
        .merge(cowboy_server_routes::platform_routes(&state))
        .merge(cowboy_server_routes::provider_routes(&state))
        .merge(cowboy_server_routes::machine_routes(&state))
        .merge(cowboy_server_routes::session_routes(&state))
        .merge(cowboy_server_routes::code_routes(&state))
        .merge(cowboy_server_agent_api::server::routes::widget_routes())
        .merge(cowboy_server_agent_api::server::routes::machine_post_edit_routes())
        // Everything else: the separately deployed SPA, with index.html
        // fallback for client-side routes.
        .fallback(static_handler)
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            enforce_product_api,
        ))
        .merge(product_auth_router(auth_state))
        .layer(middleware::from_fn_with_state(
            secure_transport,
            secure_transport::enforce,
        ))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(addr = %bind, "WS/HTTP listening");

    #[cfg(unix)]
    let local_operator = local_operator::start(&data_dir, Arc::clone(&state))?;

    // Converge signed automatic components and keep the Service-side Plugin
    // document fresh for as long as the Controller serves. A Controller with
    // neither configured simply finds nothing to do on each pass.
    let convergence_task = tokio::spawn(run_component_convergence(
        Arc::clone(&state),
        state.machine_snapshots.clone(),
        state.shutdown.clone(),
    ));

    let adoption_state = Arc::clone(&state);
    // Aborted together once the server stops serving.
    let background_tasks = [
        tokio::spawn(session_provider_updates::run(Arc::clone(&state))),
        tokio::spawn(execution::reconciler::run(Arc::clone(&state))),
        tokio::spawn(run_generation_retention(Arc::clone(&state))),
        tokio::spawn(async move {
            adopt_shared_provider_authentication(&adoption_state).await;
        }),
        tokio::spawn(session_trash::run_retention(Arc::clone(&state))),
        tokio::spawn(provider_accounts::run_recovery(Arc::clone(&state.accounts))),
        tokio::spawn(run_usage_broadcast(Arc::clone(&state))),
        tokio::spawn(superagent::run(Arc::clone(&state))),
        tokio::spawn(provider_accounts::usage::run(Arc::clone(&state.accounts))),
        tokio::spawn(provider_accounts::usage::reset::schedule::run(Arc::clone(
            &state.accounts,
        ))),
        tokio::spawn(session_context::rollover::observe(Arc::clone(&state))),
    ];
    let code_buffers = Arc::clone(&state.code_buffers);
    let result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(shutdown_tx))
    .await
    .context("axum serve");
    #[cfg(unix)]
    local_operator.shutdown().await;
    convergence_task.abort();
    for task in &background_tasks {
        task.abort();
    }
    code_buffers.shutdown().await;
    result?;
    Ok(())
}
