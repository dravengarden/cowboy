//! Usage, executor and provider reset handlers.

use super::*;

pub(super) async fn api_usage(State(state): State<Arc<AppState>>) -> Response {
    let bindings = state.usage.plugin_bindings();
    let snapshot = crate::usage::with_session_usage(
        state.usage.snapshot().await,
        &state.hub.session_list(),
        &state.provider_catalog,
        &bindings,
    );
    Json(snapshot).into_response()
}

pub(super) async fn api_usage_executors(State(state): State<Arc<AppState>>) -> Response {
    match state.usage.execution_settings().await {
        Ok(settings) => Json(settings).into_response(),
        Err(error) => {
            tracing::warn!(%error, "usage execution settings read failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Usage execution settings are unavailable",
            )
                .into_response()
        }
    }
}

pub(super) async fn api_usage_executor_update(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Json(request): Json<UsageExecutorUpdate>,
) -> Response {
    match state
        .usage
        .set_execution_machine(&provider, request.machine_id)
        .await
    {
        Ok(()) => api_usage_executors(State(state)).await,
        Err(error) => {
            tracing::warn!(%error, "usage execution settings update failed");
            (StatusCode::BAD_REQUEST, "Could not save usage Machine. Select an enrolled Machine and an account with a usage collector.").into_response()
        }
    }
}

pub(super) fn usage_response(state: &AppState, snapshot: crate::usage::UsageSnapshot) -> Response {
    let bindings = state.usage.plugin_bindings();
    Json(crate::usage::with_session_usage(
        snapshot,
        &state.hub.session_list(),
        &state.provider_catalog,
        &bindings,
    ))
    .into_response()
}

pub(super) async fn api_usage_refresh(
    State(state): State<Arc<AppState>>,
    authenticated: Option<Extension<AuthenticatedProductRequest>>,
    Query(query): Query<UsageRefreshQuery>,
) -> Response {
    // One global refresh covers every account the caller sees, named ones
    // included, instead of one client request per account.
    if let Some(Extension(auth)) = &authenticated {
        provider_accounts::usage::refresh_owner_accounts(&state, &auth.principal.user_id);
    }
    if query.wait == Some(false) {
        return match state.usage.request_refresh(None).await {
            Ok(snapshot) => usage_response(&state, snapshot),
            Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        };
    }
    usage_response(&state, state.usage.refresh().await)
}

pub(super) async fn api_usage_provider_refresh(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(provider): axum::extract::Path<String>,
    Query(query): Query<UsageRefreshQuery>,
) -> Response {
    let result = if query.wait == Some(false) {
        state.usage.request_refresh(Some(&provider)).await
    } else {
        state.usage.refresh_provider(&provider).await
    };
    match result {
        Ok(snapshot) => usage_response(&state, snapshot),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

pub(super) async fn api_usage_activity(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Query(query): Query<UsageActivityQuery>,
) -> Response {
    let bindings = state.usage.plugin_bindings();
    let Some(binding) = bindings
        .iter()
        .find(|binding| binding.account == provider && binding.activity)
    else {
        return (
            StatusCode::NOT_FOUND,
            "provider does not expose usage activity",
        )
            .into_response();
    };
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "provider usage persistence is unavailable",
        )
            .into_response();
    };
    let allowed_models = binding
        .activity_models
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<Vec<_>>();
    let allowed_agents = binding
        .activity_agents
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<Vec<_>>();
    let filter = match parse_usage_activity_filter(&query, &allowed_models, &allowed_agents) {
        Ok(filter) => filter,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    match store
        .provider_usage_activity(
            &provider,
            filter.from_ms,
            filter.to_ms,
            &filter.agents,
            &filter.models,
        )
        .await
    {
        Ok(mut activity) => {
            if let Some(object) = activity.as_object_mut() {
                object.insert("window".to_owned(), filter.window.into());
                object.insert("observedAtMs".to_owned(), crate::usage::now_ms().into());
            }
            Json(state.usage.decorate_activity(&provider, activity).await).into_response()
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) async fn api_provider_reset(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Json(request): Json<ResetRequest>,
) -> Response {
    if !reset_provider_supported(&state, &provider) {
        return (
            StatusCode::BAD_REQUEST,
            "provider does not support usage resets",
        )
            .into_response();
    }
    if request.confirm != "confirm" {
        return (StatusCode::BAD_REQUEST, "confirmation must be confirm").into_response();
    }
    let key = new_reset_idempotency_key();
    if let Some(store) = state.store.as_ref() {
        let _ = store
            .append_provider_action_log(
                &provider,
                "manual",
                "started",
                "preflight",
                "Manual reset attempt started",
                Some(&request.expected_credit_id),
                Some(&key),
                now_ms(),
            )
            .await;
    }
    match state
        .usage
        .consume_nearest_reset(&provider, &key, Some(&request.expected_credit_id))
        .await
    {
        Ok(result) => {
            if let Some(store) = state.store.as_ref() {
                let _ = store
                    .append_provider_action_log(
                        &provider,
                        "manual",
                        "succeeded",
                        "provider_response",
                        &result.outcome,
                        result.credit_id.as_deref(),
                        Some(&key),
                        now_ms(),
                    )
                    .await;
            }
            Json(result).into_response()
        }
        Err(error) => {
            if let Some(store) = state.store.as_ref() {
                let status = if error.call_may_have_reached_provider {
                    "unknown"
                } else {
                    "failed"
                };
                let phase = if error.call_may_have_reached_provider {
                    "consume"
                } else {
                    "preflight"
                };
                let _ = store
                    .append_provider_action_log(
                        &provider,
                        "manual",
                        status,
                        phase,
                        &error.to_string(),
                        error.credit_id.as_deref(),
                        Some(&key),
                        now_ms(),
                    )
                    .await;
            }
            (StatusCode::CONFLICT, error.to_string()).into_response()
        }
    }
}

pub(super) async fn api_provider_reset_schedule(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Json(request): Json<ResetScheduleRequest>,
) -> Response {
    if !reset_provider_supported(&state, &provider) {
        return (
            StatusCode::BAD_REQUEST,
            "provider does not support usage resets",
        )
            .into_response();
    }
    if request.confirm != "confirm" {
        return (StatusCode::BAD_REQUEST, "confirmation must be confirm").into_response();
    }
    if request.fire_at_ms < now_ms().saturating_add(60_000) {
        return (
            StatusCode::BAD_REQUEST,
            "schedule must be at least one minute in the future",
        )
            .into_response();
    }
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "persistent scheduling unavailable",
        )
            .into_response();
    };
    let key = new_reset_idempotency_key();
    if let Err(error) = store
        .upsert_provider_reset(&provider, request.fire_at_ms, &key)
        .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    let _ = store
        .append_provider_action_log(
            &provider,
            "scheduled",
            "scheduled",
            "timer",
            "Reset scheduled",
            None,
            Some(&key),
            now_ms(),
        )
        .await;
    state
        .usage
        .set_reset_schedule(
            &provider,
            Some(crate::usage::ResetSchedule {
                fire_at_ms: request.fire_at_ms,
            }),
        )
        .await;
    Json(state.usage.snapshot().await).into_response()
}

pub(super) async fn api_provider_reset_cancel(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
) -> Response {
    if !reset_provider_supported(&state, &provider) {
        return (
            StatusCode::BAD_REQUEST,
            "provider does not support usage resets",
        )
            .into_response();
    }
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "persistent scheduling unavailable",
        )
            .into_response();
    };
    if let Err(error) = store.delete_provider_reset(&provider).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    let _ = store
        .append_provider_action_log(
            &provider,
            "scheduled",
            "cancelled",
            "timer",
            "Scheduled reset cancelled",
            None,
            None,
            now_ms(),
        )
        .await;
    state.usage.set_reset_schedule(&provider, None).await;
    StatusCode::NO_CONTENT.into_response()
}

pub(super) async fn api_usage_logs(State(state): State<Arc<AppState>>) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "persistent logs unavailable",
        )
            .into_response();
    };
    match store.provider_action_logs(100).await {
        Ok(logs) => Json(logs).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

/// A committed Service credential supersedes a usage authentication failure.
/// Machine reconnects re-distribute an unchanged generation and must not call this.
pub(super) fn usage_credentials_changed(
    state: &Arc<AppState>,
    statuses: &[crate::provider_service::ProviderAuthenticationStatus],
) {
    for status in statuses
        .iter()
        .filter(|status| status.account_id.is_default())
    {
        let usage = state.usage.clone();
        let provider_id = status.provider_id.clone();
        tokio::spawn(async move { usage.credentials_changed(&provider_id).await });
    }
}
