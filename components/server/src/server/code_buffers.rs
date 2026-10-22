//! Additive original-owner buffer API. The legacy Review consumer and its
//! path-based language reads are not switched by publishing these endpoints.

use super::*;
use crate::core::SessionCodeScope;
use operator_approval::OperatorApproval;
use registry::{Admission, Binding, Snapshot};
use remote::Action;

mod navigation;
mod reads;
mod registry;
mod remote;
mod synchronization;
pub(super) use registry::Owners;

/// Consumer selection only, never a native capability or an execution grant.
/// Protocol 20 is the independently activated synchronization routing floor.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReviewMode {
    Legacy,
    Owned,
    Unavailable,
}

pub(super) fn review_mode(control: &MachineControl, scope: &CodeReadScope) -> ReviewMode {
    let CodeReadScope::Session(scope) = scope else {
        return ReviewMode::Legacy;
    };
    if !control.session_read_scope_is_current(scope) {
        return ReviewMode::Unavailable;
    }
    let Some(connection) = scope.connection() else {
        return ReviewMode::Legacy;
    };
    if control.supports_code_buffer_sync(connection) {
        ReviewMode::Owned
    } else {
        ReviewMode::Legacy
    }
}

#[derive(Clone)]
struct Context {
    service_id: String,
    hub: Hub,
    machine_control: Arc<MachineControl>,
    code_buffers: Arc<Owners>,
    code_navigation_admission: bool,
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
        code_navigation_admission: state.code_navigation_admission,
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
            post(reads::read).layer(DefaultBodyLimit::max(512)),
        )
        .merge(synchronization::routes())
        .merge(navigation::routes())
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
        || scope.string_bytes() > 16 * 1024
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

/// Fresh permission to observe this user's retained outcome, not a grant to
/// acquire work in its current/replacement Session. Also used by sync/navigation.
async fn check_user(
    state: &Context,
    approval: &OperatorApproval,
    user: &str,
) -> Result<(), StatusCode> {
    if approval
        .current_product_operator(state.auth())
        .await
        .is_none_or(|principal| principal.user_id != user)
        || *state.shutdown.borrow()
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

/// A ready authority future must not beat an already-expired timeout's poll.
/// All stages use the original deadline; ownership handoff cannot renew it.
fn check_deadline(deadline: tokio::time::Instant) -> Result<(), StatusCode> {
    if tokio::time::Instant::now() >= deadline {
        return Err(StatusCode::GATEWAY_TIMEOUT);
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
    let deadline = tokio::time::Instant::now() + registry::JOB_TIMEOUT;
    let future = async {
        // Share one original continuation with the owned job and its observer.
        // This neither captures a second credential nor renews its deadline.
        let approval = Arc::new(approval(&state, &authenticated, &headers)?);
        check_user(&state, &approval, &authenticated.principal.user_id).await?;
        let result =
            match state
                .code_buffers
                .admit(&authenticated.principal.user_id, &id, action)?
            {
                Admission::Saved(snapshot) => Ok(snapshot),
                Admission::Run(job) => {
                    let owners = Arc::clone(&state.code_buffers);
                    let receiver = owners.spawn(run_job(
                        state.clone(),
                        Arc::clone(&approval),
                        *job,
                        deadline,
                    ))?;
                    receiver
                        .await
                        .unwrap_or(Err(StatusCode::SERVICE_UNAVAILABLE))
                }
            };
        // Cover saved observations and errors too. This is only permission to
        // disclose an outcome: it does not reopen, release, rearm, or confer a
        // new Session/native-use grant. The job has already recorded its effect.
        check_user(&state, &approval, &authenticated.principal.user_id).await?;
        result
    };
    response(
        tokio::time::timeout_at(deadline, future)
            .await
            .unwrap_or(Err(StatusCode::GATEWAY_TIMEOUT)),
    )
}

/// The task owner commits observations independently of the response observer.
async fn run_job(
    state: Context,
    approval: Arc<OperatorApproval>,
    job: registry::Job,
    deadline: tokio::time::Instant,
) -> Result<Snapshot, StatusCode> {
    tokio::time::timeout_at(deadline, async {
        if job.action == Action::Open {
            check_open(&state, &approval, &job.binding.scope, &job.binding.user).await?;
        } else {
            // Original-resource observation/cleanup survives Session removal.
            check_user(&state, &approval, &job.binding.user).await?;
        }
        // Do not rely on timer poll ordering when authority becomes ready at
        // the deadline. Expiry cannot start a fresh native effect.
        check_deadline(deadline)?;
        job.begin()?;
        let observed = remote::request(
            &state.machine_control,
            &job.binding.connection,
            &job.binding.native,
            job.action,
        )
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
        // Save even after logout/cancellation. Response refusal is not undo.
        job.finish(observed)
    })
    .await
    .unwrap_or(Err(StatusCode::GATEWAY_TIMEOUT))
}

#[cfg(test)]
mod tests;
