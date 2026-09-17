//! Finite Service-owned navigation. Display paths and protocol support are not
//! acquisition grants; the private admission policy defaults closed.
use super::*;
use crate::machine_protocol::code_buffer_navigation::{
    Action as NativeAction, Content, Kind, Location, Phase, Point, Request as NativeRequest,
};
use axum::extract::State as AxumState;
use axum::http::HeaderValue;
use tokio::time::Instant;

pub(super) mod registry;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepare {
    content: Content,
    position: Point,
    query: Kind,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Destination {
    destination: u32,
    content: Content,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Prepared,
    Unknown,
    Retained,
    ReleaseUnknown,
    Released,
    // Inert local abandonment only, not an acknowledgement of native release.
    Expired,
}

impl From<Phase> for State {
    fn from(phase: Phase) -> Self {
        match phase {
            Phase::Prepared => Self::Prepared,
            Phase::Unknown => Self::Unknown,
            Phase::Retained => Self::Retained,
            Phase::ReleaseUnknown => Self::ReleaseUnknown,
            Phase::Released => Self::Released,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DestinationState {
    Pending,
    Unknown,
    Prepared,
    Expired,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DestinationSnapshot {
    destination: u32,
    state: DestinationState,
    // An ordinary Service lookup, never a native reference or implicit Open.
    resource_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    api_version: u8,
    navigation_id: String,
    source_resource_id: String,
    content: Content,
    position: Point,
    query: Kind,
    state: State,
    locations: Vec<Location>,
    destinations: Vec<DestinationSnapshot>,
    pending: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Action {
    Execute,
    Query,
    Release,
    Destination { destination: u32, content: Content },
}

impl Action {
    fn acquisition(&self) -> bool {
        matches!(self, Self::Execute | Self::Destination { .. })
    }
}

pub(super) fn routes() -> Router<Context> {
    Router::new()
        .route(
            "/api/code/buffers/{id}/navigations",
            post(prepare).layer(DefaultBodyLimit::max(512)),
        )
        .route(
            "/api/code/navigations/{id}",
            get(query)
                .put(execute)
                .delete(release)
                .layer(DefaultBodyLimit::max(128)),
        )
        .route(
            "/api/code/navigations/{id}/destinations",
            post(destination).layer(DefaultBodyLimit::max(512)),
        )
        .layer(middleware::map_response(|mut response: Response| async {
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
        Err(status) => (status, "owned navigation unavailable").into_response(),
    };
    ([(header::CACHE_CONTROL, "no-store")], response).into_response()
}

async fn check_user(
    context: &Context,
    approval: &OperatorApproval,
    user: &str,
) -> Result<(), StatusCode> {
    if approval
        .current_product_operator(context.auth())
        .await
        .is_none_or(|principal| principal.user_id != user)
        || *context.shutdown.borrow()
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

async fn check(
    context: &Context,
    approval: &OperatorApproval,
    binding: &Binding,
    acquisition: bool,
) -> Result<(), StatusCode> {
    if acquisition {
        if !context.code_navigation_admission {
            return Err(StatusCode::NOT_IMPLEMENTED);
        }
        check_open(context, approval, &binding.scope, &binding.user).await?;
    } else {
        check_user(context, approval, &binding.user).await?;
    }
    if !context.machine_control.is_current(&binding.connection) {
        return Err(StatusCode::CONFLICT);
    }
    if !context
        .machine_control
        .supports_code_buffer_navigation(&binding.connection)
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
    let result = tokio::time::timeout(super::registry::PREPARE_TTL, async {
        request
            .content
            .validate()
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        if request.position.row > request.content.utf8_bytes
            || request.position.column > request.content.utf8_bytes
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        let approval = approval(&context, &authenticated, &headers)?;
        let groups = &context.code_buffers.navigations;
        let reservation = groups.reserve()?;
        let source = context
            .code_buffers
            .navigation_source(&authenticated.principal.user_id, &id)?;
        check(&context, &approval, &source.binding, true).await?;
        let observed = context
            .machine_control
            .code_buffer_navigation(
                &source.binding.connection,
                NativeRequest {
                    service_id: context.service_id.clone(),
                    machine_id: source.binding.scope.machine_id().to_owned(),
                    action: NativeAction::Prepare {
                        lease: source.binding.native.clone(),
                        content: request.content.clone(),
                        position: request.position,
                        query: request.query,
                    },
                },
            )
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        check(&context, &approval, &source.binding, true).await?;
        if observed.phase != Phase::Prepared {
            return Err(StatusCode::BAD_GATEWAY);
        }
        groups.insert(
            reservation,
            id,
            source.binding.clone(),
            request,
            observed.navigation,
        )
    })
    .await
    .unwrap_or(Err(StatusCode::GATEWAY_TIMEOUT));
    response(result)
}

async fn execute(
    AxumState(context): AxumState<Context>,
    Path(id): Path<String>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Json(_request): Json<Empty>,
) -> Response {
    operate(context, id, authenticated, headers, Action::Execute).await
}

async fn query(
    AxumState(context): AxumState<Context>,
    Path(id): Path<String>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
) -> Response {
    operate(context, id, authenticated, headers, Action::Query).await
}

async fn release(
    AxumState(context): AxumState<Context>,
    Path(id): Path<String>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Json(_request): Json<Empty>,
) -> Response {
    operate(context, id, authenticated, headers, Action::Release).await
}

async fn destination(
    AxumState(context): AxumState<Context>,
    Path(id): Path<String>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Json(request): Json<Destination>,
) -> Response {
    operate(
        context,
        id,
        authenticated,
        headers,
        Action::Destination {
            destination: request.destination,
            content: request.content,
        },
    )
    .await
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
        let approval = approval(&context, &authenticated, &headers)?;
        check_user(&context, &approval, &authenticated.principal.user_id).await?;
        if action.acquisition() && !context.code_navigation_admission {
            return Err(StatusCode::NOT_IMPLEMENTED);
        }
        match context.code_buffers.navigations.admit(
            &context.code_buffers,
            &authenticated.principal.user_id,
            &id,
            action.clone(),
        )? {
            registry::Admission::Saved(snapshot, binding) => {
                if let Some(binding) = binding {
                    check(&context, &approval, &binding, action.acquisition()).await?;
                }
                Ok(*snapshot)
            }
            registry::Admission::Run(job) => {
                let worker = context.clone();
                let receiver = context.code_buffers.spawn(async move {
                    tokio::time::timeout_at(deadline, async {
                        check(&worker, &approval, &job.binding, job.action.acquisition()).await?;
                        let _source = if job.action == Action::Execute {
                            let source = worker
                                .code_buffers
                                .navigation_source(&job.binding.user, &job.resource)?;
                            if source.binding.native != job.binding.native
                                || !source.binding.connection.same(&job.binding.connection)
                            {
                                return Err(StatusCode::CONFLICT);
                            }
                            Some(source)
                        } else {
                            None
                        };
                        job.begin()?;
                        let observed = worker
                            .machine_control
                            .code_buffer_navigation(
                                &job.binding.connection,
                                NativeRequest {
                                    service_id: worker.service_id.clone(),
                                    machine_id: job.binding.scope.machine_id().to_owned(),
                                    action: job.native_action(),
                                },
                            )
                            .await
                            .map_err(|_| StatusCode::BAD_GATEWAY)?;
                        let snapshot = job.finish(&worker.code_buffers, observed)?;
                        Ok((snapshot, approval, job))
                    })
                    .await
                    .unwrap_or(Err(StatusCode::GATEWAY_TIMEOUT))
                })?;
                let (snapshot, approval, job) = receiver
                    .await
                    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)??;
                // Save effects even after revocation, but disclose no response
                // without rechecking the original request's credential.
                check(&context, &approval, &job.binding, action.acquisition()).await?;
                Ok(snapshot)
            }
        }
    };
    response(
        tokio::time::timeout_at(deadline, future)
            .await
            .unwrap_or(Err(StatusCode::GATEWAY_TIMEOUT)),
    )
}

#[cfg(test)]
mod tests;
