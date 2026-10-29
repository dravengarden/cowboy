//! Private host transport. This router is never merged into the TCP router.

use super::*;
use crate::local_operator::{Grant, SOCKET_NAME, directory, private_file, uid};
use axum::extract::connect_info::Connected;
use std::fs::File;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::UnixListener;

#[derive(Clone)]
struct Peer(Option<u32>);

impl Connected<axum::serve::IncomingStream<'_, UnixListener>> for Peer {
    fn connect_info(stream: axum::serve::IncomingStream<'_, UnixListener>) -> Self {
        Self(
            stream
                .io()
                .peer_cred()
                .ok()
                .map(|credential| credential.uid()),
        )
    }
}

#[derive(Clone)]
struct Boundary {
    directory: PathBuf,
    live: Arc<AtomicBool>,
}

async fn authenticate(
    State(boundary): State<Boundary>,
    ConnectInfo(Peer(peer_uid)): ConnectInfo<Peer>,
    mut request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    let grant = peer_uid.and_then(|peer_uid| {
        Grant::capture(&boundary.directory, peer_uid, Arc::clone(&boundary.live)).ok()
    });
    let Some(grant) = grant else {
        return (
            StatusCode::FORBIDDEN,
            "Local Operator delegation is disabled or this host identity is not authorized",
        )
            .into_response();
    };
    request.extensions_mut().insert(Arc::new(grant));
    next.run(request).await
}

async fn status(
    State(state): State<Arc<AppState>>,
    Extension(grant): Extension<Arc<Grant>>,
) -> Response {
    no_store_json(
        StatusCode::OK,
        serde_json::json!({
            "schema":1, "service_id":state.service_id, "actor":grant.actor(), "transport":"unix-peer-credentials",
        }),
    )
}

/// Every session, as the Web UI's list shows it to an owner.
async fn sessions(State(state): State<Arc<AppState>>) -> Response {
    no_store_json(
        StatusCode::OK,
        serde_json::json!({"schema":1, "sessions": state.hub.session_list()}),
    )
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct SessionProviderUpdate {
    /// Cancel a running turn instead of refusing a busy session.
    #[serde(default)]
    force: bool,
    /// Update once the current work finishes.
    #[serde(default)]
    when_idle: bool,
}

/// Move one session to its Machine's newest installed Provider release
/// through the same gates as the Web UI's Reload. An exited session is only
/// re-pinned; a busy one is updated after its turn (`when_idle`) or after
/// cancelling it (`force`).
async fn session_provider_update(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<SessionProviderUpdate>,
) -> Response {
    let Some(info) = state.hub.session_info(&session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    // A managed child serves exactly one call on its parent's binding and is
    // retired once that call finishes. Reloading it cannot help the call and,
    // once its execution lifetime is gone, only leaves it crashed.
    if state.hub.is_managed_child(&session_id) {
        return (
            StatusCode::CONFLICT,
            "managed child sessions keep their Provider release until they are retired",
        )
            .into_response();
    }
    let meta = info.meta;
    let target = match session_upgrade_target(&state, &meta).await {
        Ok(target) => target,
        Err(error) => return (StatusCode::CONFLICT, error).into_response(),
    };
    let reply = |code, state_name: &str| {
        no_store_json(
            code,
            serde_json::json!({
                "schema":1, "session":session_id, "state":state_name,
                "from":meta.provider_version, "to":target.version,
            }),
        )
    };
    if target.digest == meta.provider_generation_digest {
        return reply(StatusCode::OK, "current");
    }
    let busy = matches!(meta.status, Status::Busy | Status::Starting)
        || state.hub.session_has_in_flight_prompt(&session_id);
    if busy && !(request.force || request.when_idle) {
        return (
            StatusCode::CONFLICT,
            "session is busy; pass force to cancel its turn or when_idle to update after it",
        )
            .into_response();
    }
    if busy || request.when_idle {
        // The update pass applies a requested update as soon as the session
        // is idle, through the same fenced reload.
        state.hub.set_provider_update_when_idle(&session_id, true);
        if busy && request.force {
            if let Err(error) = state.supervisor.send(&session_id, AgentCommand::Cancel) {
                state.hub.set_provider_update_when_idle(&session_id, false);
                return (
                    StatusCode::CONFLICT,
                    format!("cancelling the turn failed: {error}"),
                )
                    .into_response();
            }
            tracing::info!(session = %session_id, "local Operator cancelled a turn to update its Provider");
            return reply(StatusCode::ACCEPTED, "cancelled_then_update");
        }
        return reply(StatusCode::ACCEPTED, "after_turn");
    }
    let live_worker = state
        .runtime_router
        .runtime(&meta.machine_id)
        .is_some_and(|runtime| runtime.has_worker(&meta.id));
    // Host operators share the same maintenance boundary as UI and automatic
    // updates. A dormant re-pin only changes metadata; recovery may allocate.
    let _maintenance: Box<dyn Send> = if meta.status == Status::Exited && !live_worker {
        match session_reclaim::decisions(&meta.machine_id).try_lock_owned() {
            Ok(lease) => Box::new(lease),
            Err(_) => return (StatusCode::CONFLICT, "maintenance is finishing").into_response(),
        }
    } else {
        match session_maintenance::reload_lease(&state, &session_id) {
            Ok(lease) => lease,
            Err(error) => return (StatusCode::CONFLICT, error).into_response(),
        }
    };
    let _fence = match ProviderReloadFence::acquire(
        &state.plugin_lifecycle_fences,
        (meta.machine_id.clone(), meta.provider.clone()),
    ) {
        Ok(fence) => fence,
        Err(error) => return (StatusCode::CONFLICT, error).into_response(),
    };
    // An exited session whose worker survives is reloaded, never re-pinned:
    // the worker's release would reconcile a re-pin straight back.
    let result = if meta.status == Status::Exited && !live_worker {
        state
            .hub
            .repin_dormant_provider(
                &meta,
                &target.version,
                &target.digest,
                &target.behavior,
                target.named_carry.as_ref(),
            )
            .map(|()| "repinned")
    } else {
        apply_session_provider_reload(&state, &meta, &target, false).map(|()| "updating")
    };
    match result {
        Ok(state_name) => {
            tracing::info!(session = %session_id, version = %target.version, state = state_name,
                "local Operator updated a session's Provider");
            reply(StatusCode::ACCEPTED, state_name)
        }
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

async fn install(
    State(state): State<Arc<AppState>>,
    Path((machine, plugin)): Path<(String, String)>,
    Extension(grant): Extension<Arc<Grant>>,
    Json(request): Json<PluginInstallRequest>,
) -> Response {
    let approval = match operator_approval::OperatorApproval::capture_host(&state.service_id, grant)
    {
        Ok(approval) => approval,
        Err(status) => return status.into_response(),
    };
    tracing::info!(actor = ?approval.actor(), "local Operator requested Plugin installation");
    plugin_install::confirmed_install(state, machine, plugin, request, approval).await
}

async fn project_request(
    State(state): State<Arc<AppState>>,
    Path(machine): Path<String>,
    Extension(grant): Extension<Arc<Grant>>,
    Json(request): Json<crate::machine_protocol::projects::Request>,
) -> Response {
    if !grant.current() {
        return StatusCode::FORBIDDEN.into_response();
    }
    projects::request(State(state), Path(machine), Json(request)).await
}

async fn project_policy(
    State(state): State<Arc<AppState>>,
    Path(machine): Path<String>,
    Extension(grant): Extension<Arc<Grant>>,
    Json(request): Json<projects::PolicyUpdate>,
) -> Response {
    if !grant.current() {
        return StatusCode::FORBIDDEN.into_response();
    }
    projects::update_policy(State(state), Path(machine), Json(request)).await
}

async fn reconcile_install(
    State(state): State<Arc<AppState>>,
    Path((machine, plugin, operation)): Path<(String, String, String)>,
    Extension(grant): Extension<Arc<Grant>>,
) -> Response {
    let approval = match operator_approval::OperatorApproval::capture_host(&state.service_id, grant)
    {
        Ok(approval) => approval,
        Err(status) => return status.into_response(),
    };
    tracing::info!(
        actor = ?approval.actor(),
        %machine,
        %plugin,
        %operation,
        "local Operator requested Plugin installation receipt reconciliation"
    );
    plugin_install::confirmed_reconcile_install(state, machine, plugin, operation, approval).await
}

async fn reconcile_uninstall(
    State(state): State<Arc<AppState>>,
    Path((machine, plugin, operation)): Path<(String, String, String)>,
    Extension(grant): Extension<Arc<Grant>>,
) -> Response {
    let approval = match operator_approval::OperatorApproval::capture_host(&state.service_id, grant)
    {
        Ok(approval) => approval,
        Err(status) => return status.into_response(),
    };
    plugin_uninstall::confirmed_reconcile_uninstall(state, machine, plugin, operation, approval)
        .await
}

/// The same preview the browser gets, with the host grant as its actor. It
/// lists the sessions an uninstall would take with it; nothing is removed here.
async fn uninstall_plan(
    State(state): State<Arc<AppState>>,
    Path((machine, plugin)): Path<(String, String)>,
    Extension(grant): Extension<Arc<Grant>>,
) -> Response {
    if !grant.current() {
        return StatusCode::FORBIDDEN.into_response();
    }
    super::preview_machine_plugin_uninstall(state, machine, plugin, grant.actor()).await
}

/// Consume a preview this same grant created. The preview's owner, target and
/// expiry are checked by the shared transaction, and a plan with active
/// sessions still requires the explicit second confirmation.
async fn uninstall(
    State(state): State<Arc<AppState>>,
    Path((machine, plugin)): Path<(String, String)>,
    Extension(grant): Extension<Arc<Grant>>,
    Json(request): Json<PluginUninstallRequest>,
) -> Response {
    let approval = match operator_approval::OperatorApproval::capture_host(&state.service_id, grant)
    {
        Ok(approval) => approval,
        Err(status) => return status.into_response(),
    };
    tracing::info!(actor = ?approval.actor(), "local Operator requested Plugin uninstall");
    plugin_uninstall::confirmed_uninstall(state, machine, plugin, request, approval).await
}

struct OwnerLock(File);

impl Drop for OwnerLock {
    fn drop(&mut self) {
        // A child between fork and exec can retain this open-file description.
        // Closing our descriptor alone would leave its flock held by that child.
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

struct SocketOwner {
    path: PathBuf,
    inode: u64,
    _lock: OwnerLock,
    live: Arc<AtomicBool>,
}

impl Drop for SocketOwner {
    fn drop(&mut self) {
        self.live.store(false, Ordering::Release);
        if std::fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.file_type().is_socket()
                && metadata.uid() == uid()
                && metadata.ino() == self.inode
        }) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub(super) struct Server {
    task: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    live: Arc<AtomicBool>,
}

pub(super) fn start(data_dir: &std::path::Path, state: Arc<AppState>) -> anyhow::Result<Server> {
    let directory = directory(data_dir)?;
    let live = Arc::new(AtomicBool::new(true));
    let (listener, owner) = bind_socket(&directory, Arc::clone(&live))?;
    let boundary = Boundary {
        directory,
        live: Arc::clone(&live),
    };
    let mut shutdown = state.shutdown.clone();
    let app = Router::new()
        .route("/v1/status", get(status))
        .route("/v1/sessions", get(sessions))
        .route(
            "/v1/sessions/{id}/provider-update",
            post(session_provider_update),
        )
        .route("/v1/execution-sessions", get(execution::recovery::list))
        .route(
            "/v1/execution-sessions/{id}/recover",
            get(execution::recovery::plan).post(execution::recovery::recover),
        )
        .route("/v1/plugins", get(api_plugins))
        .route("/v1/plugins/refresh", post(api_plugin_catalog_refresh))
        .route("/v1/machines", get(api_machines))
        .route("/v1/machines/{id}/plugins", get(api_machine_plugins))
        .route(
            "/v1/machines/{id}/durable-state",
            get(api_machine_durable_state),
        )
        .route("/v1/machines/{id}/refresh", post(api_machine_refresh))
        .route("/v1/project-policies", get(projects::policies))
        .route("/v1/project-placements", get(projects::placements))
        .route(
            "/v1/machines/{id}/projects",
            get(projects::list).post(project_request),
        )
        .route(
            "/v1/machines/{id}/project-policy",
            axum::routing::put(project_policy),
        )
        .route("/v1/machines/{id}/plugins/{plugin}/install", post(install))
        .route(
            "/v1/machines/{id}/plugins/{plugin}/uninstall-plan",
            post(uninstall_plan),
        )
        .route(
            "/v1/machines/{id}/plugins/{plugin}/uninstall",
            post(uninstall),
        )
        .route(
            "/v1/machines/{id}/plugins/{plugin}/operations",
            get(plugin_install::api_machine_plugin_install_operations),
        )
        .route(
            "/v1/machines/{id}/plugins/{plugin}/operations/{operation}/reconcile",
            post(reconcile_install),
        )
        .route("/v1/usage", get(api_usage))
        .route("/v1/usage/executors", get(api_usage_executors))
        .route(
            "/v1/usage/{provider}/executor",
            axum::routing::put(api_usage_executor_update),
        )
        .route(
            "/v1/machines/{id}/plugins/{plugin}/uninstall-operations",
            get(plugin_uninstall::api_machine_plugin_operations),
        )
        .route(
            "/v1/machines/{id}/plugins/{plugin}/uninstall-operations/{operation}/reconcile",
            post(reconcile_uninstall),
        )
        .route("/v1/usage/{provider}", post(api_usage_provider_refresh))
        .with_state(state)
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn_with_state(boundary, authenticate));
    let stopping = Arc::clone(&live);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _owner = owner;
        axum::serve(listener, app.into_make_service_with_connect_info::<Peer>())
            .with_graceful_shutdown(async move {
                if !*shutdown.borrow() {
                    tokio::select! { _ = shutdown.changed() => {}, _ = stopped => {} }
                }
                stopping.store(false, Ordering::Release);
            })
            .await
    });
    Ok(Server {
        task: Some(task),
        stop: Some(stop),
        live,
    })
}

fn bind_socket(
    directory: &std::path::Path,
    live: Arc<AtomicBool>,
) -> anyhow::Result<(UnixListener, SocketOwner)> {
    let lock = private_file(&directory.join("owner.lock"), true)?;
    fs2::FileExt::try_lock_exclusive(&lock).context("local Operator endpoint is already owned")?;
    let lock = OwnerLock(lock);
    let path = directory.join(SOCKET_NAME);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) => {
            anyhow::ensure!(
                metadata.file_type().is_socket() && metadata.uid() == uid(),
                "refusing to remove a foreign local Operator path"
            );
            std::fs::remove_file(&path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let owner = SocketOwner {
        inode: std::fs::symlink_metadata(&path)?.ino(),
        path,
        _lock: lock,
        live,
    };
    Ok((listener, owner))
}

impl Server {
    pub(super) async fn shutdown(mut self) {
        self.live.store(false, Ordering::Release);
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let Some(task) = self.task.take() else {
            return;
        };
        match task.await {
            Ok(Ok(())) => {}
            _ => tracing::error!("local Operator listener stopped unexpectedly"),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.live.store(false, Ordering::Release);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests;
