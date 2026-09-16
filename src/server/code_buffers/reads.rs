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
    request.validate().map_err(|_| StatusCode::BAD_REQUEST)?;
    let approval = approval(&state, &authenticated, &headers)?;
    let owners = Arc::clone(&state.code_buffers);
    let job = owners.admit_read(&authenticated.principal.user_id, &id)?;
    let worker_state = state.clone();
    let deadline = tokio::time::Instant::now() + registry::JOB_TIMEOUT;
    let receiver = owners.spawn(async move {
        check_read(&worker_state, &approval, &job.binding).await?;
        remote::read_support(
            &worker_state.machine_control,
            &job.binding.connection,
            request.support_kind(),
        )
        .await
        .map_err(|_| StatusCode::NOT_IMPLEMENTED)?;
        check_read(&worker_state, &approval, &job.binding).await?;
        let reply = remote::read(
            &worker_state.machine_control,
            &job.binding.connection,
            &job.binding.native,
            request,
        )
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
        Ok((reply, job, approval))
    })?;
    tokio::time::timeout_at(deadline, async {
        let (reply, job, approval) = receiver
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)??;
        // Session removal/ABA, account changes or revoked credentials discard
        // the result at the response boundary. The borrow and original approval
        // survive delivery from the owned task; a dropped observer grants nothing.
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
