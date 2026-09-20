//! Explicit finite disk-to-native synchronization, never a read-side reload.

use super::*;
use crate::machine_protocol::code_buffer_sync::{
    Action as NativeAction, Content, Purpose, Reason, Request as NativeRequest,
    State as NativeState, VersionEntry,
};
use axum::extract::State as AxumState;
use axum::http::HeaderValue;
use tokio::time::Instant;

pub(super) mod registry;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepare {
    purpose: Purpose,
    content: Content,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum State {
    Prepared {},
    Pending {},
    Unknown {},
    Applied {
        content: Content,
        version: Vec<VersionEntry>,
    },
    Refused {
        reason: Reason,
    },
    Retired {},
    // Local effect-free abandonment, not a claim of remote retirement.
    Expired {},
}

impl From<NativeState> for State {
    fn from(value: NativeState) -> Self {
        match value {
            NativeState::Prepared {} => Self::Prepared {},
            NativeState::Pending {} => Self::Pending {},
            NativeState::Unknown {} => Self::Unknown {},
            NativeState::Applied { content, version } => Self::Applied { content, version },
            NativeState::Refused { reason } => Self::Refused { reason },
            NativeState::Retired {} => Self::Retired {},
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    api_version: u8,
    operation_id: String,
    resource_id: String,
    purpose: Purpose,
    content: Content,
    state: State,
    pending: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Action {
    Apply,
    Query,
    Retire,
}

pub(super) fn routes() -> Router<Context> {
    Router::new()
        .route(
            "/api/code/buffers/{id}/synchronizations",
            post(prepare).layer(DefaultBodyLimit::max(512)),
        )
        .route(
            "/api/code/buffer-synchronizations/{id}",
            get(query)
                .put(apply)
                .delete(retire)
                .layer(DefaultBodyLimit::max(128)),
        )
        .layer(middleware::map_response(|mut response: Response| async {
            // Include extractor rejections, not just handler responses.
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }))
}

fn response(result: Result<Snapshot, StatusCode>) -> Response {
    let response = match result {
        Ok(snapshot) => (
            if snapshot.pending {
                StatusCode::ACCEPTED
            } else {
                StatusCode::OK
            },
            Json(snapshot),
        )
            .into_response(),
        Err(status) => (status, "owned buffer synchronization unavailable").into_response(),
    };
    ([(header::CACHE_CONTROL, "no-store")], response).into_response()
}

async fn check(
    context: &Context,
    approval: &OperatorApproval,
    binding: &Binding,
    action: Action,
) -> Result<(), StatusCode> {
    if action == Action::Apply {
        check_open(context, approval, &binding.scope, &binding.user).await?;
    } else {
        // Independent fresh permission to observe/retire the same user's
        // original operation after Session deletion, never to write a new one.
        check_user(context, approval, &binding.user).await?;
    }
    if !context.machine_control.is_current(&binding.connection) {
        return Err(StatusCode::CONFLICT);
    }
    if !context
        .machine_control
        .supports_code_buffer_sync(&binding.connection)
    {
        return Err(StatusCode::NOT_IMPLEMENTED);
    }
    Ok(())
}

async fn prepare(
    AxumState(context): AxumState<Context>,
    Path(id): Path<String>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Json(request): Json<Prepare>,
) -> Response {
    let result = tokio::time::timeout(
        super::registry::PREPARE_TTL,
        prepare_inner(&context, id, &authenticated, &headers, request),
    )
    .await
    .unwrap_or(Err(StatusCode::GATEWAY_TIMEOUT));
    response(result)
}

async fn prepare_inner(
    context: &Context,
    id: String,
    authenticated: &AuthenticatedProductRequest,
    headers: &HeaderMap,
    request: Prepare,
) -> Result<Snapshot, StatusCode> {
    request
        .content
        .validate()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let approval = approval(context, authenticated, headers)?;
    let operations = &context.code_buffers.synchronizations;
    let reservation = operations.reserve()?;
    let (binding, fence) = context
        .code_buffers
        .synchronization_owner(&authenticated.principal.user_id, &id)?;
    check(context, &approval, &binding, Action::Apply).await?;
    let observed = context
        .machine_control
        .code_buffer_sync(
            &binding.connection,
            NativeRequest {
                service_id: context.service_id.clone(),
                machine_id: binding.scope.machine_id().to_owned(),
                action: NativeAction::Prepare {
                    lease: binding.native.clone(),
                    purpose: request.purpose,
                    content: request.content.clone(),
                },
            },
            &request.content,
        )
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    check(context, &approval, &binding, Action::Apply).await?;
    if observed.state != (NativeState::Prepared {}) {
        return Err(StatusCode::BAD_GATEWAY);
    }
    operations.insert(
        reservation,
        registry::Prepared {
            resource: id,
            binding,
            fence,
            content: request.content,
            operation: observed.operation,
        },
    )
}

async fn apply(
    AxumState(context): AxumState<Context>,
    Path(id): Path<String>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Json(_request): Json<Empty>,
) -> Response {
    operate(context, id, authenticated, headers, Action::Apply).await
}

async fn query(
    AxumState(context): AxumState<Context>,
    Path(id): Path<String>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
) -> Response {
    operate(context, id, authenticated, headers, Action::Query).await
}

async fn retire(
    AxumState(context): AxumState<Context>,
    Path(id): Path<String>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Json(_request): Json<Empty>,
) -> Response {
    operate(context, id, authenticated, headers, Action::Retire).await
}

async fn operate(
    context: Context,
    id: String,
    authenticated: AuthenticatedProductRequest,
    headers: HeaderMap,
    action: Action,
) -> Response {
    let deadline = Instant::now() + super::registry::JOB_TIMEOUT;
    let future = async {
        let approval = Arc::new(approval(&context, &authenticated, &headers)?);
        check_user(&context, &approval, &authenticated.principal.user_id).await?;
        let result = async {
            match context.code_buffers.synchronizations.admit(
                &authenticated.principal.user_id,
                &id,
                action,
            )? {
                registry::Admission::Saved(snapshot) => Ok(snapshot),
                registry::Admission::Run(job) => {
                    let receiver = context.code_buffers.spawn(run_job(
                        context.clone(),
                        Arc::clone(&approval),
                        *job,
                        deadline,
                    ))?;
                    let (snapshot, job) = receiver
                        .await
                        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)??;
                    // Record exact effects even if the original permission expires,
                    // but do not disclose their response under revoked authority.
                    check(&context, &approval, &job.binding, action).await?;
                    Ok(snapshot)
                }
            }
        }
        .await;
        // Errors and saved snapshots must cross the same original-credential
        // boundary as successful replies. This never modifies effect evidence.
        check_user(&context, &approval, &authenticated.principal.user_id).await?;
        result
    };
    response(
        tokio::time::timeout_at(deadline, future)
            .await
            .unwrap_or(Err(StatusCode::GATEWAY_TIMEOUT)),
    )
}

async fn run_job(
    context: Context,
    approval: Arc<OperatorApproval>,
    job: registry::Job,
    deadline: Instant,
) -> Result<(Snapshot, registry::Job), StatusCode> {
    tokio::time::timeout_at(deadline, async {
        check(&context, &approval, &job.binding, job.action).await?;
        check_deadline(deadline)?;
        job.begin()?;
        let snapshot = dispatch(&context, &job).await?;
        Ok((snapshot, job))
    })
    .await
    .unwrap_or(Err(StatusCode::GATEWAY_TIMEOUT))
}

async fn dispatch(context: &Context, job: &registry::Job) -> Result<Snapshot, StatusCode> {
    let operation = job.operation.clone();
    let action = match job.action {
        Action::Apply => NativeAction::Apply { operation },
        Action::Query => NativeAction::Query { operation },
        Action::Retire => NativeAction::Retire { operation },
    };
    let observed = context
        .machine_control
        .code_buffer_sync(
            &job.binding.connection,
            NativeRequest {
                service_id: context.service_id.clone(),
                machine_id: job.binding.scope.machine_id().to_owned(),
                action,
            },
            &job.content,
        )
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    job.finish(observed.state)
}

#[cfg(test)]
mod tests;
