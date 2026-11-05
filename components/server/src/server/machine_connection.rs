//! The authenticated Machine connection: handshake, writer and runtime tunnel
//! tasks, per-frame handling, provider authentication sync and presence.

use super::*;

mod frames;
use frames::*;

#[derive(Clone)]
pub(super) struct MachineSnapshots {
    pub(super) store: Option<Store>,
    pub(super) hub: Hub,
    pub(super) runtime_router: Arc<RuntimeRouter>,
    pub(super) machine_control: Arc<MachineControl>,
    pub(super) desired_components: Arc<crate::machine_convergence::DesiredComponentSource>,
    pub(super) component_convergence: Arc<crate::machine_convergence::ConvergenceState>,
    pub(super) convergence_freeze: Arc<crate::machine_convergence::ConvergenceFreeze>,
    pub(super) desired_plugins: Arc<crate::machine_convergence::PluginDesiredSource>,
    pub(super) product_auth_enabled: bool,
    pub(super) revision: Arc<AtomicU64>,
    /// Whether a full Machine can still free a slot (`sessions.reclaim_*`).
    pub(super) service_config: crate::config::Handle,
}

pub(super) fn project_machine_health(
    status: &str,
    runtime_connected: bool,
    control_connection_age: Option<std::time::Duration>,
    automatic_update_pending: bool,
    observed_at_ms: i64,
    last_seen_at_ms: Option<i64>,
) -> crate::machine_protocol::MachineHealth {
    use crate::machine_protocol::{MachineHealth, MachineHealthReason, MachineHealthState};

    let (state, reason) = match status {
        "online" if runtime_connected => (
            MachineHealthState::Ready,
            MachineHealthReason::RuntimeConnected,
        ),
        "online"
            if automatic_update_pending
                && control_connection_age
                    .is_some_and(|age| age < MACHINE_COMPONENT_RECONCILIATION_GRACE) =>
        {
            (
                MachineHealthState::Updating,
                MachineHealthReason::MachineUpdating,
            )
        }
        "online"
            if control_connection_age.is_some_and(|age| age < MACHINE_RUNTIME_STARTUP_GRACE) =>
        {
            (
                MachineHealthState::Starting,
                MachineHealthReason::RuntimeStarting,
            )
        }
        "online" if control_connection_age.is_some() => (
            MachineHealthState::Degraded,
            MachineHealthReason::RuntimeUnavailable,
        ),
        "online" => (
            MachineHealthState::Degraded,
            MachineHealthReason::TransportUnavailable,
        ),
        "reconnecting" => (
            MachineHealthState::Reconnecting,
            MachineHealthReason::TransportReconnecting,
        ),
        "updating" => (
            MachineHealthState::Updating,
            MachineHealthReason::MachineUpdating,
        ),
        "degraded" => (
            MachineHealthState::Degraded,
            MachineHealthReason::MachineReportedDegraded,
        ),
        "offline" => (
            MachineHealthState::Offline,
            MachineHealthReason::MachineOffline,
        ),
        _ => (
            MachineHealthState::Degraded,
            MachineHealthReason::UnknownMachineStatus,
        ),
    };
    MachineHealth {
        state,
        reason,
        observed_at_ms,
        last_seen_at_ms,
    }
}

/// Annotates reported components with their live lease counts, any Plugin
/// that supersedes them, and the update the signed manifest offers.
fn annotate_components(
    components: &mut [crate::machine_protocol::ComponentInventory],
    open_sessions: u32,
    provider_sessions: Option<&HashMap<String, u64>>,
    plugins: &[crate::machine_protocol::PluginInventory],
    desired: &crate::machine_convergence::DesiredComponents,
    checked_at_ms: i64,
) {
    for component in components {
        if matches!(
            component.id.kind,
            crate::machine_protocol::ComponentKind::AcpRuntime
                | crate::machine_protocol::ComponentKind::ProviderAdapter
                | crate::machine_protocol::ComponentKind::ProviderCli
        ) {
            component.active_leases = match component.id.kind {
                crate::machine_protocol::ComponentKind::ProviderAdapter
                | crate::machine_protocol::ComponentKind::ProviderCli => {
                    let slot = component.id.slot.as_str();
                    let occupants = crate::plugin_runtime_args::occupancy_provider_ids(slot);
                    if occupants.is_empty() {
                        provider_sessions
                            .and_then(|providers| providers.get(slot))
                            .copied()
                            .unwrap_or(0)
                    } else {
                        occupants.iter().fold(0, |total, provider| {
                            total.saturating_add(
                                provider_sessions
                                    .and_then(|providers| providers.get(*provider))
                                    .copied()
                                    .unwrap_or(0),
                            )
                        })
                    }
                }
                _ => u64::from(open_sessions),
            };
        }
        component.superseded_by = superseding_plugin(&component.id, plugins);
        if let Some(desired) = desired.find(&component.id) {
            let available = component.state != crate::machine_protocol::ComponentState::Active
                || !component.digest.eq_ignore_ascii_case(&desired.digest);
            component.update = Some(crate::machine_protocol::ComponentUpdate {
                latest_version: desired.version.clone(),
                available,
                source: "signed Cowboy manifest".to_owned(),
                checked_at_ms,
                installable: available,
            });
        }
    }
}

impl MachineSnapshots {
    pub(super) async fn load(
        &self,
    ) -> anyhow::Result<Vec<crate::machine_protocol::MachineSummary>> {
        let Some(store) = self.store.as_ref() else {
            return Ok(Vec::new());
        };
        let machines = store.list_machines().await?;
        // (open sessions, sessions holding a worker slot, open sessions per
        // Provider). Component occupancy keeps counting every open session;
        // capacity counts only live Agent processes.
        let mut session_loads: HashMap<String, (u32, u32, HashMap<String, u64>)> = HashMap::new();
        for session in self
            .hub
            .session_list()
            .into_iter()
            .filter(|session| session.status != crate::agent_model::Status::Exited)
        {
            let live = holds_worker_slot(&self.runtime_router, &session);
            let (open_sessions, live_sessions, providers) = session_loads
                .entry(session.machine_id)
                .or_insert_with(|| (0, 0, HashMap::new()));
            *open_sessions = open_sessions.saturating_add(1);
            if live {
                *live_sessions = live_sessions.saturating_add(1);
            }
            let provider_sessions = providers.entry(session.provider).or_default();
            *provider_sessions = provider_sessions.saturating_add(1);
        }
        let checked_at_ms = now_ms();
        let desired = self.desired_components.current();
        let frozen = self.convergence_freeze.is_frozen();
        let desired_plugins = self.desired_plugins.current();
        Ok(machines
            .into_iter()
            .filter(|machine| product_machine_is_visible(machine, self.product_auth_enabled))
            .map(|machine| {
                let workspaces: Vec<crate::machine_protocol::MachineWorkspace> = machine
                    .inventory
                    .get("workspaces")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
                let workspace_revision = machine
                    .inventory
                    .get("workspace_revision")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                let mut components: Vec<crate::machine_protocol::ComponentInventory> = machine
                    .inventory
                    .get("components")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
                let plugins: Vec<crate::machine_protocol::PluginInventory> = machine
                    .inventory
                    .get("plugins")
                    .or_else(|| machine.inventory.get("providers"))
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
                let capacity: crate::machine_protocol::MachineCapacity = machine
                    .inventory
                    .get("capacity")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
                let provider_contracts = machine
                    .inventory
                    .get("provider_contracts")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok());
                let plugin_contracts = machine
                    .inventory
                    .get("plugin_contracts")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok());
                let (open_sessions, active_sessions, provider_sessions) = session_loads
                    .get(&machine.id)
                    .map_or((0, 0, None), |(open, live, providers)| {
                        (*open, *live, Some(providers))
                    });
                let local = machine.connection_mode == "local";
                annotate_components(
                    &mut components,
                    open_sessions,
                    provider_sessions,
                    &plugins,
                    &desired,
                    checked_at_ms,
                );
                let pending_updates: Vec<crate::machine_protocol::ComponentId> = desired
                    .iter()
                    .filter(|desired| {
                        !components.iter().any(|current| {
                            current.id == desired.id
                                && current.digest.eq_ignore_ascii_case(&desired.digest)
                                && current.state == crate::machine_protocol::ComponentState::Active
                        })
                    })
                    .map(|desired| desired.id.clone())
                    .collect();
                let automatic_update_pending = desired.iter().any(|component| {
                    component.automatic && pending_updates.contains(&component.id)
                });
                let connected = self.runtime_router.connected(&machine.id);
                // Projects belong to the execution target; an AI-only Machine
                // can accept Remote sessions with an empty project registry.
                // A full Machine stays schedulable while a create can free a
                // slot by hibernating an idle session, as the create does.
                let schedulable = connected
                    && !capacity.draining
                    && (active_sessions < capacity.max_sessions
                        || session_reclaim::reclaimable(
                            session_maintenance::policy(
                                &self.hub,
                                &self.service_config,
                                &machine.id,
                            )
                            .reclaim_on_capacity
                                == Some(true),
                            self.service_config
                                .get(&crate::config::schema::SESSIONS_RECLAIM_MIN_IDLE),
                            &session_reclaim::candidates_in(
                                &self.hub,
                                &self.runtime_router,
                                &machine.id,
                            ),
                        ));
                let health = project_machine_health(
                    &machine.status,
                    connected,
                    self.machine_control.connection_age(&machine.id),
                    automatic_update_pending,
                    checked_at_ms,
                    machine.last_seen_at_ms,
                );
                let machine_id = machine.id.clone();
                let mut summary = crate::machine_protocol::MachineSummary {
                    local,
                    connected,
                    schedulable,
                    health,
                    id: machine.id,
                    display_name: machine.display_name,
                    platform: machine.platform,
                    architecture: machine.architecture,
                    status: machine.status,
                    fingerprint: machine.fingerprint,
                    workspaces,
                    workspace_revision,
                    workspace_owner: machine
                        .inventory
                        .get("workspace_owner")
                        .cloned()
                        .and_then(|v| serde_json::from_value(v).ok()),
                    components,
                    plugins,
                    provider_contracts,
                    plugin_contracts,
                    capacity,
                    active_sessions,
                    resources: self.machine_control.host_resources(&machine_id),
                    capabilities: crate::machine_protocol::MachineCapabilities {
                        hibernation: self.machine_control.machine_supports(
                            &machine_id,
                            crate::machine_protocol::SESSION_HIBERNATION_PROTOCOL_VERSION,
                        ),
                        native_fork: self.machine_control.machine_supports(
                            &machine_id,
                            crate::machine_protocol::NATIVE_FORK_PROTOCOL_VERSION,
                        ),
                    },
                    pending_updates,
                    convergence: Vec::new(),
                    plugin_lifecycle: if desired_plugins.value.manages(&machine_id) {
                        crate::machine_protocol::PluginLifecycleOwner::Managed
                    } else {
                        crate::machine_protocol::PluginLifecycleOwner::Manual
                    },
                };
                summary.convergence = crate::machine_convergence::plan_machine(
                    &summary,
                    &desired,
                    &self.component_convergence.attempts(&machine_id),
                    frozen,
                    checked_at_ms,
                )
                .reported;
                summary
            })
            .collect())
    }

    pub(super) async fn connect_message(&self) -> anyhow::Result<Outbound> {
        let revision = self.revision.load(Ordering::Acquire);
        Ok(Outbound::Machines {
            revision,
            machines: self.load().await?,
            resync: true,
        })
    }

    pub(super) async fn publish(&self) {
        let revision = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        match self.load().await {
            Ok(machines) => self.hub.broadcast_machines(revision, machines),
            Err(error) => tracing::warn!(%error, revision, "publishing Machine snapshot"),
        }
    }
}

pub(super) async fn run_machine_presence_sweeper(
    store: Store,
    machine_snapshots: MachineSnapshots,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        match store.expire_machine_reconnects().await {
            Ok(expired) if expired > 0 => {
                tracing::warn!(expired, "Machine reconnect grace expired");
                machine_snapshots.publish().await;
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "expiring Machine reconnect grace"),
        }
        tokio::select! {
            _ = tokio::time::sleep(MACHINE_RECONNECT_SWEEP_INTERVAL) => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
}

pub(super) fn is_transient_machine_disconnect(error: &str) -> bool {
    error.contains("is not connected") || error.contains("disconnected")
}

/// Same rule as the metrics scrape: a direct same-host caller, never a request
/// that crossed the public reverse proxy.
pub(super) fn internal_machine_refresh(
    method: &Method,
    path: &str,
    peer: SocketAddr,
    headers: &HeaderMap,
) -> bool {
    method == Method::POST
        && machine_refresh_path(path)
        && crate::product_auth::metrics_scrape_allowed(peer, headers)
}

pub(super) async fn await_machine_command(
    state: &AppState,
    machine_id: &str,
    request_id: String,
    command: crate::machine_protocol::MachineCommand,
    operation: &'static str,
    timeout: std::time::Duration,
) -> Response {
    let started = std::time::Instant::now();
    let result = state
        .machine_control
        .command_request_with_timeout(machine_id, request_id.clone(), command, timeout)
        .await;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match result {
        Ok(()) => {
            tracing::info!(
                %machine_id,
                %request_id,
                operation,
                elapsed_ms,
                "Machine command completed"
            );
            Json(MachineCommandResponse { request_id }).into_response()
        }
        Err(error) => {
            tracing::warn!(
                %machine_id,
                %request_id,
                operation,
                elapsed_ms,
                timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
                %error,
                "Machine command failed"
            );
            (machine_command_error_status(&error), error).into_response()
        }
    }
}

pub(super) async fn sync_provider_auth_to_machine(
    state: &Arc<AppState>,
    machine_id: &str,
    provider_id: &str,
) -> Result<u64, String> {
    sync_provider_account_auth_to_machine(
        state,
        machine_id,
        provider_id,
        &crate::provider_account::ProviderAccountId::default(),
    )
    .await
}

pub(super) async fn sync_provider_account_auth_to_machine(
    state: &Arc<AppState>,
    machine_id: &str,
    provider_id: &str,
    account_id: &crate::provider_account::ProviderAccountId,
) -> Result<u64, String> {
    let connection = state.machine_control.operation_connection(machine_id)?;
    let envelope =
        provider_account_auth_envelope_for_machine(state, machine_id, provider_id, account_id)
            .await?;
    state
        .provider_auth_sync
        .apply(&state.machine_control, &connection, envelope)
        .await
}

pub(super) async fn reconcile_machine_account_grants(
    state: &Arc<AppState>,
    machine_id: &str,
) -> Result<(), String> {
    let store = state
        .store
        .as_ref()
        .ok_or("Provider authentication requires persistence")?;
    let key = store
        .machine_encryption_public_key(machine_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("Machine has no enrolled encryption key")?;
    let statuses = state
        .provider_auth
        .machine_account_replica_statuses(&key)
        .map_err(|error| error.to_string())?;
    for status in statuses {
        if let Err(error) =
            sync_and_mark_account_grant(state, machine_id, &status.provider_id, &status.account_id)
                .await
        {
            tracing::warn!(%error, %machine_id, account = status.account_id.as_str(), provider = %status.provider_id,
                "account credential reconciliation remains pending");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn accept_service_auth_candidate(
    state: &Arc<AppState>,
    machine_id: &str,
    request_id: &str,
    provider_id: &str,
    account_id: &crate::provider_account::ProviderAccountId,
    native_identity: Option<&crate::provider_account::NativeIdentity>,
    auth_method: &str,
    provider_version: &str,
    generation_digest: &str,
    auth_contract_fingerprint: &str,
    portable_schema: &str,
    values: BTreeMap<String, String>,
    account_label: Option<String>,
) -> Result<(), String> {
    if !account_id.is_default() {
        return provider_accounts::login::accept(
            state,
            provider_accounts::login::Candidate {
                machine: machine_id,
                request: request_id,
                provider: provider_id,
                account: account_id,
                identity: native_identity,
                version: provider_version,
                digest: generation_digest,
                contract: auth_contract_fingerprint,
                bundle: crate::machine_protocol::PortableCredentialBundle {
                    portable_schema: portable_schema.into(),
                    method_id: auth_method.into(),
                    values,
                },
            },
        )
        .await;
    }
    let (executor, statuses) = {
        let mut executors = state.provider_auth_executors.lock();
        let executor = executors
            .get_mut(request_id)
            .ok_or_else(|| "authentication executor is no longer active".to_owned())?;
        if !executor.active(&state.machine_control, now_ms()) {
            return Err("authentication executor expired or disconnected".to_owned());
        }
        if !executor.accepts_candidate(
            machine_id,
            account_id,
            provider_id,
            auth_method,
            provider_version,
            generation_digest,
            auth_contract_fingerprint,
        ) {
            return Err("authentication candidate does not match its executor".to_owned());
        }
        if executor.promotion_started {
            return Ok(());
        }
        let package = state
            .provider_catalog
            .package(provider_id, provider_version, generation_digest)
            .ok_or_else(|| {
                "authentication candidate references an untrusted Agent Plugin release".to_owned()
            })?;
        if package.manifest.compatibility.auth_contract_fingerprint != auth_contract_fingerprint {
            return Err("authentication candidate contract fingerprint mismatch".to_owned());
        }
        if package.manifest.authentication.portable_schema != portable_schema {
            return Err("authentication candidate portable schema mismatch".to_owned());
        }
        let method = package
            .manifest
            .authentication
            .methods
            .iter()
            .find(|method| method.id == auth_method)
            .ok_or_else(|| "authentication method is not declared".to_owned())?;
        provider_accounts::validate_native_evidence(
            provider_id,
            method.native_identity.as_ref(),
            native_identity,
        )
        .map_err(str::to_owned)?;
        let bundle = crate::machine_protocol::PortableCredentialBundle {
            portable_schema: portable_schema.to_owned(),
            method_id: auth_method.to_owned(),
            values,
        };
        let packages = state
            .provider_catalog
            .packages_for_authentication_scope(portable_schema);
        let statuses = executor.promote_once(|expected_generation| {
            (if packages.len() == 1 {
                state
                    .provider_auth
                    .commit(
                        &packages[0],
                        &bundle,
                        account_label.clone(),
                        Some(expected_generation),
                    )
                    .map(|status| vec![status])
            } else {
                state.provider_auth.commit_shared(
                    &packages,
                    &bundle,
                    account_label.clone(),
                    provider_id,
                    Some(expected_generation),
                )
            })
            .map_err(|error| error.to_string())
        })?;
        let Some(statuses) = statuses else {
            return Ok(());
        };
        (executor.clone(), statuses)
    };
    let (statuses, replicas) = distribute_and_mark_provider_auth(state, statuses).await;
    recover_failed_provider_sessions(state, &statuses, &replicas);
    usage_credentials_changed(state, &statuses);
    let mut warnings = Vec::new();
    if !statuses
        .iter()
        .any(|status| status.provider_id == provider_id)
    {
        warnings.push("shared Provider authentication status is incomplete".to_owned());
    }
    state.machine_control.record_service_login(
        machine_id,
        crate::machine_control::ServiceLoginNotice {
            request_id: request_id.to_owned(),
            provider: provider_id.to_owned(),
            state: crate::machine_protocol::AuthState::SignedIn,
            account_label: account_label.clone(),
            detail: Some(if warnings.is_empty() {
                "authentication is owned and synchronized by Cowboy Service".to_owned()
            } else {
                format!(
                    "authentication is owned by Cowboy Service; {}",
                    warnings.join("; ")
                )
            }),
        },
    );

    // Promotion and Machine distribution are the security boundary. Report
    // success before cleaning the disposable executor home so a slow cleanup
    // cannot leave the UI looking as though credential ownership is unresolved.
    let finalize_request_id = machine_request_id("provider-auth-finalize");
    if let Err(error) = state
        .machine_control
        .command_on_connection(
            &executor.connection,
            finalize_request_id.clone(),
            crate::machine_protocol::MachineCommand::FinalizeProviderAuthCandidate {
                request_id: finalize_request_id,
                account_id: executor.account_id.clone(),
                provider_id: provider_id.to_owned(),
                auth_method: auth_method.to_owned(),
                candidate_request_id: request_id.to_owned(),
            },
        )
        .await
        .map_err(|error| error.detail)
    {
        tracing::warn!(%error, %provider_id, %request_id, "cleaning temporary Provider authentication home");
        state.machine_control.record_service_login(
            machine_id,
            crate::machine_control::ServiceLoginNotice {
                request_id: request_id.to_owned(),
                provider: provider_id.to_owned(),
                state: crate::machine_protocol::AuthState::SignedIn,
                account_label,
                detail: Some(format!(
                    "authentication is owned by Cowboy Service; temporary executor cleanup is pending: {error}"
                )),
            },
        );
    }
    Ok(())
}

pub(super) async fn write_machine_messages<S>(
    sink: S,
    messages: mpsc::UnboundedReceiver<Message>,
    features: impl Into<crate::machine_transport::Features>,
    credit: Option<Arc<crate::machine_transport::Credit>>,
) -> anyhow::Result<()>
where
    S: SinkExt<Message> + Unpin,
{
    crate::machine_transport::write(sink, messages, features, credit).await
}

pub(super) fn queue_machine_message(
    tx: &mpsc::UnboundedSender<Message>,
    message: Message,
) -> Result<(), ()> {
    tx.send(message).map_err(|_| ())
}

pub(super) fn queue_machine_json<T: Serialize>(
    tx: &mpsc::UnboundedSender<Message>,
    message: &T,
) -> Result<(), ()> {
    let text = serde_json::to_string(message).map_err(|_| ())?;
    queue_machine_message(tx, Message::Text(text.into()))
}

pub(super) async fn forward_machine_runtime_frames<R>(
    mut reader: crate::runtime_wire::FrameReader<R>,
    tx: mpsc::Sender<crate::runtime_wire::Frame>,
) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    loop {
        let frame = reader
            .next()
            .await
            .context("reading Controller runtime frame")?
            .ok_or_else(|| anyhow::anyhow!("Controller runtime tunnel closed"))?;
        tx.send(frame)
            .await
            .map_err(|_| anyhow::anyhow!("Machine WebSocket runtime bridge closed"))?;
    }
}

pub(super) async fn write_machine_runtime_frames<W>(
    mut writer: W,
    mut rx: mpsc::UnboundedReceiver<crate::runtime_wire::Frame>,
) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    while let Some(frame) = rx.recv().await {
        tokio::time::timeout(
            WEBSOCKET_FRAME_SEND_TIMEOUT,
            crate::runtime_wire::write_frame(&mut writer, &frame),
        )
        .await
        .map_err(|_| anyhow::anyhow!("writing Machine runtime frame timed out"))?
        .context("writing Machine runtime frame")?;
    }
    Ok(())
}

pub(super) fn random_machine_token() -> anyhow::Result<String> {
    let mut random = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .context("opening OS randomness")?
        .read_exact(&mut random)
        .context("reading OS randomness")?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random))
}

pub(super) async fn handle_machine_ws(
    mut socket: WebSocket,
    state: Arc<AppState>,
    (chunked, compressed, paced, stream): (bool, bool, bool, bool),
    lane_token: Option<String>,
) {
    let Some(MachineHandshake {
        store,
        hello,
        challenge_id,
        protocol,
        connection_mode,
    }) = machine_handshake(&mut socket, &state).await
    else {
        return;
    };
    tracing::info!(machine = %hello.machine_id, chunked, compressed, paced, stream, "Machine connected");
    // Keep WebSocket writes out of the Machine read loop. A runtime replay or
    // command response may fill the socket while the Machine is still sending
    // heartbeats; the read side must continue to make progress independently.
    // Finite runtime replays can exceed a small frame-count bound, so let the
    // per-frame writer timeout fence a genuinely stalled connection instead of
    // disconnecting a healthy Machine merely because the queue briefly bursts.
    let (socket_sink, mut socket_stream) = socket.split();
    let (machine_write_tx, machine_write_rx) = mpsc::unbounded_channel();
    let execution_calls = Arc::new(tokio::sync::Semaphore::new(64));
    let credit = Arc::new(crate::machine_transport::Credit::default());
    let mut socket_writer = tokio::spawn(write_machine_messages(
        socket_sink,
        machine_write_rx,
        (chunked, compressed, paced, stream),
        Some(Arc::clone(&credit)),
    ));
    let (machine_command_tx, mut machine_command_rx) = mpsc::unbounded_channel();
    // A declared connection mode is a request to be read on this host, not
    // evidence of running on it: the transport is the same TCP socket either
    // way, and a reverse proxy makes every peer look local. Require an
    // explicit permission as well, so an enrolled remote Machine cannot name
    // Controller-host paths and have them read back.
    let colocated = colocated_execution_permitted(
        connection_mode == "local",
        &hello.machine_id,
        &state.colocated_machines,
    );
    if connection_mode == "local" && !colocated {
        tracing::warn!(
            machine = %hello.machine_id,
            "Machine declared local mode without colocation permission; \
             its reads will execute on the Machine"
        );
    }
    let connection = state.machine_control.install(
        hello.machine_id.clone(),
        challenge_id.clone(),
        colocated,
        protocol,
        machine_command_tx,
    );
    state
        .session_maintenance
        .observe_capacity(&hello.machine_id, hello.capacity.clone());
    let lanes = Arc::new(crate::machine_lanes::LaneSet::default());
    // Dropped when this connection ends, which refuses further lanes.
    let _lane_registration = lane_token.map(|token| {
        machine_lanes::register(
            token,
            machine_lanes::LaneTarget {
                connection: connection.clone(),
                machine_id: hello.machine_id.clone(),
                execution_calls: Arc::clone(&execution_calls),
                primary: machine_write_tx.clone(),
                lanes: Arc::clone(&lanes),
                features: (chunked, compressed, paced, stream),
            },
        )
    });
    state.machine_control.record_remote(
        &connection,
        crate::machine_protocol::MachineEvent::Inventory {
            components: hello.components.clone(),
            workspaces: Some(hello.workspaces.clone()),
            workspace_identities: Some(hello.workspace_identities.clone()),
            workspace_revision: hello.workspace_revision.clone(),
            workspace_owner: hello.workspace_owner,
            observed_at_ms: now_ms(),
        },
    );
    state.machine_snapshots.publish().await;
    state.machine_control.record_remote(
        &connection,
        crate::machine_protocol::MachineEvent::PluginInventory {
            plugins: hello.plugins.clone(),
            observed_at_ms: now_ms(),
        },
    );
    spawn_machine_reconnect_sync(&state, &hello, &connection, protocol);
    let (runtime_core, runtime_tunnel) = match UnixStream::pair() {
        Ok(pair) => pair,
        Err(error) => {
            tracing::error!(%error, machine = %hello.machine_id, "creating Machine runtime tunnel");
            socket_writer.abort();
            state.machine_control.remove_if_current(&connection);
            return;
        }
    };
    let (runtime_reader, runtime_writer) = runtime_tunnel.into_split();
    let runtime_reader = crate::runtime_wire::FrameReader::new(runtime_reader);
    // Drain Controller -> Machine runtime frames independently from the main
    // Machine WebSocket loop. Without this pump, a large frame arriving from
    // Machine can fill runtime_writer while RemoteRuntime simultaneously fills
    // the opposite half of the same socketpair; both tasks then await write
    // capacity and neither returns to its read branch.
    let (runtime_frame_tx, mut runtime_frame_rx) = mpsc::channel(MACHINE_RUNTIME_OUTBOUND_CAPACITY);
    let mut runtime_forwarder = tokio::spawn(forward_machine_runtime_frames(
        runtime_reader,
        runtime_frame_tx,
    ));
    // Keep Machine -> Controller writes out of the WebSocket loop as well. A
    // blocked internal consumer must not stop that loop from forwarding the
    // opposing direction, which is what lets the consumer make progress.
    // Startup can replay every unacknowledged worker event before
    // RemoteRuntime finishes its handshake. Do not reject that finite replay
    // based on frame count: the writer's per-frame timeout bounds a genuinely
    // stalled consumer, and disconnect cleanup drops the remaining queue.
    let (runtime_write_tx, runtime_write_rx) = mpsc::unbounded_channel();
    let mut runtime_writer = tokio::spawn(write_machine_runtime_frames(
        runtime_writer,
        runtime_write_rx,
    ));
    let (runtime_tx, mut runtime_rx) = tokio::sync::oneshot::channel();
    install_machine_runtime(&state, &hello, protocol, runtime_core, runtime_tx);
    let mut connected_runtime: Option<Arc<RemoteRuntime>> = None;
    let mut runtime_registration_pending = true;
    let mut inventory = MachineInventory::from_hello(&hello);
    let link = MachineLink {
        state: &state,
        store: &store,
        hello: &hello,
        challenge_id: &challenge_id,
        connection: &connection,
        machine_write_tx: &machine_write_tx,
        execution_calls: &execution_calls,
        lanes: &lanes,
        runtime_write_tx: &runtime_write_tx,
    };
    // The last usage batch this connection declined, reported once per batch.
    let mut declined_usage_batch: Option<(String, u64)> = None;
    let mut revocation_check = tokio::time::interval(std::time::Duration::from_secs(2));
    revocation_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    revocation_check.tick().await;
    let mut heartbeat_watchdog = MachineHeartbeatWatchdog::default();
    let mut decoder = crate::machine_transport::Decoder::new((chunked, compressed, paced, stream));
    // Why the socket ended, and how long it had been silent: a peer close, a
    // transport error or EOF after a stall look alike otherwise.
    let connected_at = std::time::Instant::now();
    let mut last_inbound = connected_at;
    let socket_ended = |cause: &str, error: Option<String>, silent: std::time::Duration| {
        tracing::warn!(
            machine = %hello.machine_id,
            cause,
            error = error.as_deref().unwrap_or(""),
            connected_ms = connected_at.elapsed().as_millis(),
            inbound_idle_ms = silent.as_millis(),
            "Machine WebSocket ended"
        );
    };
    loop {
        let message = tokio::select! {
            message = socket_stream.next() => Some(message),
            runtime = &mut runtime_rx, if runtime_registration_pending => {
                runtime_registration_pending = false;
                if let Ok(runtime) = runtime {
                    connected_runtime = Some(runtime);
                }
                None
            }
            frame = runtime_frame_rx.recv() => {
                let Some(frame) = frame else { break };
                if queue_machine_json(
                    &machine_write_tx,
                    &crate::machine_protocol::MachineFrame::Runtime { frame },
                ).is_err() {
                    break;
                }
                continue;
            }
            forwarded = &mut runtime_forwarder => {
                match forwarded {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(%error, machine = %hello.machine_id, "Machine runtime forwarding stopped");
                    }
                    Err(error) => {
                        tracing::warn!(%error, machine = %hello.machine_id, "Machine runtime forwarding task failed");
                    }
                }
                break;
            }
            written = &mut runtime_writer => {
                match written {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(%error, machine = %hello.machine_id, "Machine runtime writer stopped");
                    }
                    Err(error) => {
                        tracing::warn!(%error, machine = %hello.machine_id, "Machine runtime writer task failed");
                    }
                }
                break;
            }
            written = &mut socket_writer => {
                match written {
                    Ok(Ok(())) => {
                        tracing::warn!(machine = %hello.machine_id, "Machine WebSocket writer stopped");
                    }
                    Ok(Err(error)) => {
                        tracing::warn!(%error, machine = %hello.machine_id, "Machine WebSocket writer failed");
                    }
                    Err(error) => {
                        tracing::warn!(%error, machine = %hello.machine_id, "Machine WebSocket writer task failed");
                    }
                }
                break;
            }
            command = machine_command_rx.recv() => {
                let Some(command) = command else { break };
                if queue_machine_json(
                    &machine_write_tx,
                    &crate::machine_protocol::MachineFrame::Command { command },
                ).is_err() {
                    break;
                }
                continue;
            }
            _ = revocation_check.tick() => {
                if heartbeat_watchdog.expired_at(std::time::Instant::now()) {
                    tracing::warn!(
                        machine = %hello.machine_id,
                        timeout_ms = MACHINE_HEARTBEAT_TIMEOUT.as_millis(),
                        "Machine heartbeat timed out"
                    );
                    break;
                }
                match store.machine_connection_is_current(&hello.machine_id, &challenge_id).await {
                    Ok(true) => continue,
                    Ok(false) => {
                        tracing::info!(machine = %hello.machine_id, "Machine connection fenced");
                        let _ = queue_machine_message(&machine_write_tx, Message::Close(None));
                        break;
                    }
                    Err(error) => {
                        tracing::warn!(%error, machine = %hello.machine_id, "checking Machine revocation");
                        break;
                    }
                }
            }
        };
        let Some(message) = message else {
            continue;
        };
        let silent = last_inbound.elapsed();
        last_inbound = std::time::Instant::now();
        let Some(message) = message else {
            socket_ended("eof", None, silent);
            break;
        };
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                socket_ended("read_error", Some(error.to_string()), silent);
                break;
            }
        };
        let message = match message {
            Message::Binary(bytes) => {
                let decoded = decoder.chunk((chunked, compressed), &bytes).await;
                if let Some(credit) = decoder.credit()
                    && queue_machine_message(&machine_write_tx, credit).is_err()
                {
                    break;
                }
                match decoded {
                    Ok(Some(text)) => Message::Text(text.into()),
                    Ok(None) => continue,
                    Err(_) => break,
                }
            }
            Message::Text(text) => {
                if decoder.text(&text).is_err() {
                    break;
                }
                Message::Text(text)
            }
            other => other,
        };
        let Message::Text(text) = message else {
            match message {
                Message::Ping(value) => {
                    if queue_machine_message(&machine_write_tx, Message::Pong(value)).is_err() {
                        break;
                    }
                }
                Message::Close(_) => {
                    socket_ended("close", None, silent);
                    break;
                }
                Message::Pong(payload) => {
                    credit.observe(&payload);
                }
                _ => {}
            }
            continue;
        };
        let Ok(frame) = serde_json::from_str::<crate::machine_protocol::MachineFrame>(&text) else {
            break;
        };
        if !state.machine_control.is_current(&connection) {
            break;
        }
        if link
            .handle_frame(
                &mut inventory,
                &mut declined_usage_batch,
                &mut heartbeat_watchdog,
                frame,
            )
            .await
            .is_break()
        {
            break;
        }
    }
    socket_writer.abort();
    runtime_forwarder.abort();
    runtime_writer.abort();
    if let Some(runtime) = connected_runtime.as_ref() {
        state
            .runtime_router
            .remove_if_current(&hello.machine_id, runtime);
    }
    state.machine_control.remove_if_current(&connection);
    if let Err(error) = store
        .machine_disconnected(
            &hello.machine_id,
            &challenge_id,
            MACHINE_RECONNECT_GRACE_SECONDS,
        )
        .await
    {
        tracing::warn!(%error, machine = %hello.machine_id, "marking Machine offline");
    }
    state.machine_snapshots.publish().await;
}

/// Converges a reconnected Machine's account grants, interrupted plugin
/// staging and Provider authentication replicas off the connection loop.
fn spawn_machine_reconnect_sync(
    state: &Arc<AppState>,
    hello: &crate::machine_protocol::MachineHello,
    connection: &crate::machine_control::ConnectionToken,
    protocol: u16,
) {
    if protocol >= crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION {
        let sync_state = Arc::clone(state);
        let machine_id = hello.machine_id.clone();
        tokio::spawn(async move {
            if let Err(error) = reconcile_machine_account_grants(&sync_state, &machine_id).await {
                tracing::warn!(%error, %machine_id, "cannot reconcile account credential grants");
            }
        });
    }
    // A Machine that restarted mid-install proves an interrupted staging
    // attempt durably at startup; release its Service fence without an Operator.
    tokio::spawn(plugin_install::recover_interrupted_staging(
        Arc::clone(state),
        hello.machine_id.clone(),
        connection.clone(),
    ));
    for authentication in state.provider_auth.replica_statuses() {
        let sync_state = Arc::clone(state);
        let machine_id = hello.machine_id.clone();
        tokio::spawn(async move {
            let outcome = distribute_provider_auth(&sync_state, &authentication.provider_id).await;
            match sync_state.provider_auth.mark_distribution(
                &authentication.provider_id,
                authentication.auth_generation,
                outcome.state(),
            ) {
                Ok(marked) => {
                    let replicas = BTreeMap::from([(marked.provider_id.clone(), outcome)]);
                    recover_failed_provider_sessions(&sync_state, &[marked], &replicas);
                }
                Err(error) => {
                    tracing::warn!(%error, %machine_id, provider_id = %authentication.provider_id, "recording reconnected Machine Provider auth convergence");
                }
            }
        });
    }
}

/// Installs the Machine's RemoteRuntime in the router and finishes its
/// broker handshake over the runtime tunnel in the background.
fn install_machine_runtime(
    state: &Arc<AppState>,
    hello: &crate::machine_protocol::MachineHello,
    protocol: u16,
    runtime_core: UnixStream,
    runtime_tx: tokio::sync::oneshot::Sender<Arc<RemoteRuntime>>,
) {
    let router = Arc::clone(&state.runtime_router);
    let hub = state.hub.clone();
    let provider_auth = Arc::clone(&state.provider_auth);
    let machine_snapshots = state.machine_snapshots.clone();
    let runtime_telemetry = state.observability.clone();
    let machine_id = hello.machine_id.clone();
    let generation = hello
        .components
        .iter()
        .find(|component| {
            component.id.kind == crate::machine_protocol::ComponentKind::AcpRuntime
                && component.state == crate::machine_protocol::ComponentState::Active
        })
        .map_or_else(
            || hello.host_build.clone(),
            |component| component.generation.clone(),
        );
    let label = PathBuf::from(format!("machine://{machine_id}"));
    // Install before the broker handshake finishes. Snapshot replay of
    // restored workers can take seconds; without a runtime in the router
    // every send/open in that window fails with "hawk is not connected".
    let runtime = RemoteRuntime::connecting(
        hub,
        machine_id.clone(),
        label.clone(),
        generation,
        provider_auth,
        protocol,
        if protocol >= crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION {
            hello.encryption_public_key.clone()
        } else {
            None
        },
    );
    runtime.attach_telemetry(machine_id.clone(), runtime_telemetry);
    if let Some(store) = state.store.as_ref() {
        runtime.attach_native_admission(machine_id.clone(), store.clone());
    }
    router.install(machine_id.clone(), Arc::clone(&runtime));
    tokio::spawn(async move {
        match RemoteBootstrap::from_stream(label, runtime_core).await {
            Ok(bootstrap) => {
                runtime.start(bootstrap);
                machine_snapshots.publish().await;
                let _ = runtime_tx.send(runtime);
            }
            Err(error) => {
                tracing::warn!(%error, machine = %machine_id, "Machine runtime handshake failed");
                router.remove_if_current(&machine_id, &runtime);
            }
        }
    });
}
