use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Prepare {
    epoch: String,
}

pub(super) async fn prepare(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(session): Path<String>,
    Json(request): Json<Prepare>,
) -> Response {
    if !session_is_visible(&state.hub, &auth.principal, &session) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(info) = state.hub.session_info(&session) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !auth
        .principal
        .can_mutate(info.meta.owner_user_id.as_deref())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(_permit) = Arc::clone(&state.extensions.bindings.capacity).try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    match state
        .extensions
        .launches
        .prepare(&state, &session, &request.epoch)
        .await
    {
        Ok(()) => Json(
            json!({"schema":1,"epoch":request.epoch,"launch_prepared":true,"runtime_ready":false}),
        )
        .into_response(),
        Err(error) => {
            tracing::warn!(
                session = %session,
                error = %format!("{error:#}"),
                "MCP launch preparation unavailable"
            );
            (
                StatusCode::CONFLICT,
                Json(json!({"error":"mcp_launch_preparation_unavailable"})),
            )
                .into_response()
        }
    }
}
