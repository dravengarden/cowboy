//! Product writing routes deliberately have no session/Machine extractor.
use super::*;
use crate::store::draft_documents::{DraftMutation, DraftResult};

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({"error": message}))).into_response()
}

pub(super) async fn list(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
) -> Response {
    let Some(store) = &state.store else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
    match store.draft_documents(&auth.principal.user_id).await {
        Ok(documents) => {
            let entries: Vec<_> = documents
                .into_iter()
                .map(|mut d| {
                    d.body.clear();
                    let mut metadata = serde_json::to_value(d).expect("document serializes");
                    metadata
                        .as_object_mut()
                        .expect("document object")
                        .remove("body");
                    metadata
                        .as_object_mut()
                        .expect("document object")
                        .remove("attachments");
                    metadata
                })
                .collect();
            (
                [(header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({"entries": entries})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "read draft library");
            error(StatusCode::INTERNAL_SERVER_ERROR, "Could not load drafts")
        }
    }
}

pub(super) async fn read(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<String>,
) -> Response {
    let Some(store) = &state.store else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
    match store.draft_document(&auth.principal.user_id, &id).await {
        Ok(Some(d)) => ([(header::CACHE_CONTROL, "no-store")], Json(d)).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "Draft not found"),
        Err(e) => {
            tracing::error!(error = %e, "read draft");
            error(StatusCode::INTERNAL_SERVER_ERROR, "Could not load draft")
        }
    }
}

pub(super) async fn history(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<String>,
) -> Response {
    let Some(store) = &state.store else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
    match store.draft_history(&auth.principal.user_id, &id).await {
        Ok(history) => ([(header::CACHE_CONTROL, "no-store")], Json(history)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "read draft recovery");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not load recovery history",
            )
        }
    }
}

pub(super) async fn mutate(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Json(mutation): Json<DraftMutation>,
) -> Response {
    if !auth.principal.can_reorder() {
        return error(
            StatusCode::FORBIDDEN,
            "Editing requires an operator account",
        );
    }
    let Some(dataset) = headers
        .get("x-cowboy-dataset")
        .and_then(|v| v.to_str().ok())
    else {
        return error(StatusCode::CONFLICT, "Draft dataset is required");
    };
    if !sync_dataset::matches(&state.service_id, &auth.principal, dataset) {
        return error(
            StatusCode::CONFLICT,
            "Draft dataset changed; reload before saving",
        );
    }
    let Some(store) = &state.store else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
    match store.mutate_draft_document(&auth.principal.user_id, &mutation).await {
        Ok(DraftResult::Applied(document)) => Json(document).into_response(),
        Ok(DraftResult::Conflict(document)) => (StatusCode::CONFLICT, Json(serde_json::json!({"error": "This draft changed elsewhere. Your local version has been kept.", "current": document}))).into_response(),
        Ok(DraftResult::Invalid(message)) => error(StatusCode::UNPROCESSABLE_ENTITY, &message),
        Err(e) => { tracing::error!(error = %e, "persist draft"); error(StatusCode::INTERNAL_SERVER_ERROR, "Could not save draft; retry with the same operation") }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CopyUndo {
    text: String,
    content: Vec<serde_json::Value>,
}

pub(super) async fn undo_copy(
    State(state): State<Arc<AppState>>,
    Path((id, cmid)): Path<(String, String)>,
    Json(request): Json<CopyUndo>,
) -> Response {
    if state
        .hub
        .remove_draft_if_unchanged(&id, &cmid, &request.text, &request.content)
    {
        StatusCode::NO_CONTENT.into_response()
    } else {
        error(
            StatusCode::CONFLICT,
            "This draft was edited, scheduled, sent or removed; it has been kept",
        )
    }
}
