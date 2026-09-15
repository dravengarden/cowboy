//! Additive original-owner buffer API. The legacy Review consumer and its
//! path-based language reads are not switched by publishing these endpoints.

use super::*;
use crate::core::SessionCodeScope;
use operator_approval::OperatorApproval;
use registry::{Admission, Binding, Snapshot};
use remote::Action;

mod reads;
mod registry;
mod remote;
pub(super) use registry::Owners;

#[derive(Clone)]
struct Context {
    service_id: String,
    hub: Hub,
    machine_control: Arc<MachineControl>,
    code_buffers: Arc<Owners>,
    shutdown: watch::Receiver<bool>,
    product_auth_enabled: bool,
    store: Option<Store>,
    device_access: Arc<crate::client_auth::DeviceAccessSessions>,
    product_authentication: Arc<crate::auth_plugins::ProductAuthentication>,
}

impl Context {
    fn auth(&self) -> ProductRequestAuth<'_> {
        ProductRequestAuth {
            product_auth_enabled: self.product_auth_enabled,
            store: self.store.as_ref(),
            hub: &self.hub,
            device_access: &self.device_access,
            product_authentication: &self.product_authentication,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PrepareRequest {
    session_id: String,
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

pub(super) fn routes(state: &AppState) -> Router<Arc<AppState>> {
    router().with_state(Context {
        service_id: state.service_id.clone(),
        hub: state.hub.clone(),
        machine_control: Arc::clone(&state.machine_control),
        code_buffers: Arc::clone(&state.code_buffers),
        shutdown: state.shutdown.clone(),
        product_auth_enabled: state.product_auth_enabled,
        store: state.store.clone(),
        device_access: Arc::clone(&state.device_access),
        product_authentication: Arc::clone(&state.product_authentication),
    })
}

fn router() -> Router<Context> {
    Router::new()
        .route(
            "/api/code/buffers",
            post(prepare).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/code/buffers/{id}",
            get(query)
                .put(open)
                .delete(release)
                .layer(DefaultBodyLimit::max(128)),
        )
        .route(
            "/api/code/buffers/{id}/read",
            post(reads::read).layer(DefaultBodyLimit::max(128)),
        )
}

fn response(result: Result<Snapshot, StatusCode>) -> Response {
    let response = match result {
        Ok(value) => (
            if value.pending {
                StatusCode::ACCEPTED
            } else {
                StatusCode::OK
            },
            Json(value),
        )
            .into_response(),
        Err(status) => (status, "owned buffer unavailable").into_response(),
    };
    ([(header::CACHE_CONTROL, "no-store")], response).into_response()
}

fn approval(
    state: &Context,
    authenticated: &AuthenticatedProductRequest,
    headers: &HeaderMap,
) -> Result<OperatorApproval, StatusCode> {
    if *state.shutdown.borrow() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    OperatorApproval::capture_product(
        state.auth(),
        &state.service_id,
        Some(authenticated),
        headers,
    )
}

async fn prepare(
    State(state): State<Context>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Json(request): Json<PrepareRequest>,
) -> Response {
    response(prepare_inner(&state, &authenticated, &headers, request).await)
}

async fn prepare_inner(
    state: &Context,
    authenticated: &AuthenticatedProductRequest,
    headers: &HeaderMap,
    request: PrepareRequest,
) -> Result<Snapshot, StatusCode> {
    let approval = approval(state, authenticated, headers)?;
    let scope = state
        .hub
        .session_code_scope(&request.session_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    // Core scope identity, both user IDs, and the path are bounded before any
    // reservation/I/O. A resource retains no client-provided paths after prepare.
    if request.path.len() > 4_096
        || authenticated.principal.user_id.len() > 256
        || CodeReadScope::Session(scope.clone()).string_bytes() > 16 * 1024
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if scope.machine_id() == "local" {
        return Err(StatusCode::NOT_IMPLEMENTED);
    }
    let (worktree, path) = zed_language_target(scope.machine_id(), scope.cwd(), &request.path)
        .filter(|(worktree, _)| worktree.len() <= 4_096)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let reservation = state.code_buffers.reserve()?;
    let connection = state
        .machine_control
        .operation_connection(scope.machine_id())
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let prepare = async {
        check_open(state, &approval, &scope, &authenticated.principal.user_id).await?;
        remote::support(&state.machine_control, &connection)
            .await
            .map_err(|_| StatusCode::NOT_IMPLEMENTED)?;
        check_open(state, &approval, &scope, &authenticated.principal.user_id).await?;
        // Preparation requires the existing worktree to be ready. It never
        // opens a buffer, falls back to legacy, or invents readiness via health.
        let native = remote::prepare(&state.machine_control, &connection, &worktree, &path)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        check_open(state, &approval, &scope, &authenticated.principal.user_id).await?;
        state.code_buffers.insert(
            reservation,
            Binding {
                user: authenticated.principal.user_id.clone(),
                scope,
                connection,
                native,
            },
        )
    };
    tokio::time::timeout(registry::PREPARE_TTL, prepare)
        .await
        .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?
}

async fn check_open(
    state: &Context,
    approval: &OperatorApproval,
    scope: &SessionCodeScope,
    user: &str,
) -> Result<(), StatusCode> {
    let principal = approval
        .current_product_operator(state.auth())
        .await
        .filter(|principal| principal.user_id == user)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if *state.shutdown.borrow() || !state.hub.code_scope_is_current(scope) {
        return Err(StatusCode::CONFLICT);
    }
    if !principal.can_mutate(scope.owner_user_id()) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}

async fn open(
    State(state): State<Context>,
    Path(id): Path<String>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Json(_request): Json<Empty>,
) -> Response {
    operate(state, id, authenticated, headers, Action::Open).await
}

async fn query(
    State(state): State<Context>,
    Path(id): Path<String>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
) -> Response {
    operate(state, id, authenticated, headers, Action::Query).await
}

async fn release(
    State(state): State<Context>,
    Path(id): Path<String>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Json(_request): Json<Empty>,
) -> Response {
    operate(state, id, authenticated, headers, Action::Release).await
}

async fn operate(
    state: Context,
    id: String,
    authenticated: AuthenticatedProductRequest,
    headers: HeaderMap,
    action: Action,
) -> Response {
    let result = async {
        let approval = approval(&state, &authenticated, &headers)?;
        match state
            .code_buffers
            .admit(&authenticated.principal.user_id, &id, action)?
        {
            Admission::Saved(snapshot) => Ok(snapshot),
            Admission::Run(job) => {
                let owners = Arc::clone(&state.code_buffers);
                let receiver = owners.spawn(async move {
                    let observed = {
                        if action == Action::Open {
                            check_open(&state, &approval, &job.binding.scope, &job.binding.user)
                                .await?;
                        } else {
                            // Fresh permission to observe/release this original
                            // user's resource, even after Session removal/ABA.
                            let current = approval.current_product_operator(state.auth()).await;
                            if current.is_none_or(|principal| principal.user_id != job.binding.user)
                                || *state.shutdown.borrow()
                            {
                                return Err(StatusCode::UNAUTHORIZED);
                            }
                        }
                        job.begin()?;
                        remote::request(
                            &state.machine_control,
                            &job.binding.connection,
                            &job.binding.native,
                            job.action,
                        )
                        .await
                        .map_err(|_| StatusCode::BAD_GATEWAY)?
                    };
                    job.finish(observed)
                })?;
                receiver
                    .await
                    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            }
        }
    }
    .await;
    response(result)
}

#[cfg(test)]
mod tests;
