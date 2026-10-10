//! Product observations use the same parent visibility boundary as transcripts.

mod approval;
mod coordinator;
mod runner;

pub(super) use coordinator::{Coordinator, delete_child, handle, parent_deleted};

use super::{AppState, AuthenticatedProductRequest, session_is_visible};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Page {
    before: Option<String>,
}

pub(super) async fn list(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Path(parent): Path<String>,
    Query(page): Query<Page>,
) -> Response {
    if !session_is_visible(&state.hub, &authenticated.principal, &parent) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if page
        .before
        .as_deref()
        .is_some_and(|id| !crate::managed_calls::valid_id(id))
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(store) = &state.store else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    // Readiness is an observation, never a cached grant. Dispatch must derive
    // authority again; reconnect or Reload may invalidate this immediately.
    let parent_runtime_ready = state.hub.session_info(&parent).is_some_and(|info| {
        state
            .runtime_router
            .runtime(&info.meta.machine_id)
            .is_some_and(|runtime| {
                runtime
                    .managed_call_authority(&state.service_id, &info.meta)
                    .is_ok()
            })
    });
    let placement = state.hub.session_info(&parent).and_then(|info| {
        crate::managed_calls::service::Ledger::for_parent(
            store.clone(),
            &state.service_id,
            &info.meta,
        )
        .ok()
        .map(|ledger| ledger.placement().clone())
    });
    match store.managed_calls(&parent, page.before.as_deref()).await {
        Ok(calls) => {
            let next = (calls.len() == 100).then(|| calls.last().unwrap().call_id.clone());
            let summaries: Vec<_> = calls.iter().map(|call| call.summary()).collect();
            Json(json!({"schema":1,"calls":summaries,"next_before":next,
                "current_placement":placement,"parent_runtime_ready":parent_runtime_ready}))
            .into_response()
        }
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

pub(super) async fn inspect(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Path((parent, call)): Path<(String, String)>,
) -> Response {
    if !session_is_visible(&state.hub, &authenticated.principal, &parent) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(store) = &state.store else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match store.managed_call(&parent, &call).await {
        Ok(Some(record)) => Json(json!({"schema":1,"call":record})).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// A product user may stop a visible parent's call. Recording the request is
/// not proof that the child stopped; the runner observes the actual outcome.
pub(super) async fn cancel(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Path((parent, call)): Path<(String, String)>,
) -> Response {
    if !session_is_visible(&state.hub, &authenticated.principal, &parent)
        || !crate::managed_calls::valid_id(&call)
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match coordinator::cancel_call(&state, &parent, &call).await {
        Some(record) => Json(json!({"schema":1,"call":record.summary()})).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ApprovalDecision {
    decision: approval::Decision,
}

/// A person's answer to the calls its session's agent is waiting for.
pub(super) async fn decide_approval(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Path(parent): Path<String>,
    Json(body): Json<ApprovalDecision>,
) -> Response {
    if !session_is_visible(&state.hub, &authenticated.principal, &parent) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(info) = state.hub.session_info(&parent) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !authenticated
        .principal
        .can_mutate(info.meta.owner_user_id.as_deref())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let approvals = &state.managed_calls.approvals;
    // Nothing waiting: the agent already stopped asking.
    let Some(agents) = approvals.decide(&parent, body.decision) else {
        state.hub.broadcast_call_approval(&parent, None);
        return StatusCode::CONFLICT.into_response();
    };
    if body.decision == approval::Decision::Session {
        super::agent_tools::allow_session(&state, &info.meta, &agents);
    }
    state
        .hub
        .broadcast_call_approval(&parent, approvals.view(&parent, &info.meta.provider));
    StatusCode::NO_CONTENT.into_response()
}
