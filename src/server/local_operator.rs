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
        .route("/v1/plugins", get(api_plugins))
        .route("/v1/plugins/refresh", post(api_plugin_catalog_refresh))
        .route("/v1/machines/{id}/plugins", get(api_machine_plugins))
        .route("/v1/machines/{id}/plugins/{plugin}/install", post(install))
        .route(
            "/v1/machines/{id}/plugins/{plugin}/operations",
            get(plugin_install::api_machine_plugin_install_operations),
        )
        .route("/v1/usage", get(api_usage))
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
