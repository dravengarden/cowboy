//! Product observations use the same parent visibility boundary as transcripts.

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
