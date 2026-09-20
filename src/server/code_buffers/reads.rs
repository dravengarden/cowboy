//! Observation borrows an original owner; neither a path nor a native handle
//! comes from the browser. Reads do not admit, recover or replay native effects.

use super::*;
use crate::code_buffer_read::{Output, Request, VersionEntry};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadResponse {
    api_version: u8,
    resource_id: String,
    opened_version: Vec<VersionEntry>,
    result: Output,
}

pub(super) async fn read(
    State(state): State<Context>,
    Path(id): Path<String>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Response {
    let result = read_inner(state, id, authenticated, headers, request).await;
    let response = match result {
        Ok(value) => Json(value).into_response(),
        Err(status) => (status, "owned buffer read unavailable").into_response(),
    };
    ([(header::CACHE_CONTROL, "no-store")], response).into_response()
}

async fn read_inner(
    state: Context,
    id: String,
    authenticated: AuthenticatedProductRequest,
    headers: HeaderMap,
    request: Request,
) -> Result<ReadResponse, StatusCode> {
    let deadline = tokio::time::Instant::now() + registry::JOB_TIMEOUT;
    request.validate().map_err(|_| StatusCode::BAD_REQUEST)?;
    let approval = Arc::new(approval(&state, &authenticated, &headers)?);
    let owners = Arc::clone(&state.code_buffers);
    let job = owners.admit_read(&authenticated.principal.user_id, &id)?;
    let receiver = owners.spawn(run_read(
        state.clone(),
        Arc::clone(&approval),
        job,
        request,
        deadline,
    ))?;
    tokio::time::timeout_at(deadline, async {
        let result = receiver
            .await
            .unwrap_or(Err(StatusCode::SERVICE_UNAVAILABLE));
        // Keep the original approval independently of task success. A failed
        // support/read response cannot bypass original-user authorization.
        check_user(&state, &approval, &authenticated.principal.user_id).await?;
        let (reply, job) = result?;
        check_read(&state, &approval, &job.binding).await?;
        Ok(ReadResponse {
            api_version: 1,
            resource_id: id,
            opened_version: reply.opened_version,
            result: reply.result,
        })
    })
    .await
    .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?
}

async fn run_read(
    state: Context,
    approval: Arc<OperatorApproval>,
    job: registry::ReadJob,
    request: Request,
    deadline: tokio::time::Instant,
) -> Result<
    (
        crate::code_buffer_read::Reply<remote::NativeRef>,
        registry::ReadJob,
    ),
    StatusCode,
> {
    // Cancellation drops only the observer; the actual borrow drains by this
    // same original deadline, including both support and native read waits.
    tokio::time::timeout_at(deadline, async {
        check_read(&state, &approval, &job.binding).await?;
        check_deadline(deadline)?;
        remote::read_support(
            &state.machine_control,
            &job.binding.connection,
            request.support_kind(),
        )
        .await
        .map_err(|_| StatusCode::NOT_IMPLEMENTED)?;
        check_read(&state, &approval, &job.binding).await?;
        check_deadline(deadline)?;
        let reply = remote::read(
            &state.machine_control,
            &job.binding.connection,
            &job.binding.native,
            request,
        )
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
        Ok((reply, job))
    })
    .await
    .unwrap_or(Err(StatusCode::GATEWAY_TIMEOUT))
}

async fn check_read(
    state: &Context,
    approval: &OperatorApproval,
    binding: &Binding,
) -> Result<(), StatusCode> {
    check_open(state, approval, &binding.scope, &binding.user).await?;
    if !state.machine_control.is_current(&binding.connection) {
        return Err(StatusCode::CONFLICT);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
