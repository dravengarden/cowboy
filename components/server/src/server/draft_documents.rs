//! Product writing routes deliberately have no session/Machine extractor.
use super::*;
use crate::store::draft_documents::{DraftMutation, DraftResult};

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({"error": message}))).into_response()
}

/// A document without its authored content: what the library lists and the
/// `drafts` push announces.
fn metadata(document: &crate::store::draft_documents::DraftDocument) -> serde_json::Value {
    let mut metadata = serde_json::to_value(document).expect("document serializes");
    let object = metadata.as_object_mut().expect("document object");
    object.remove("body");
    object.remove("attachments");
    metadata
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
            let entries: Vec<_> = documents.iter().map(metadata).collect();
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

/// The History list without content (`draft_history_index`); a client
/// fetches a version's text and attachments only when it is opened.
pub(super) async fn history_index(
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
    match store
        .draft_history_index(&auth.principal.user_id, &id)
        .await
    {
        Ok(index) => ([(header::CACHE_CONTROL, "no-store")], Json(index)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "read draft recovery index");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not load recovery history",
            )
        }
    }
}

pub(super) async fn history_snapshot(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((id, revision)): Path<(String, i64)>,
) -> Response {
    let Some(store) = &state.store else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
    match store
        .draft_history_snapshot(&auth.principal.user_id, &id, revision)
        .await
    {
        // A kept snapshot never changes: a client may cache it for good.
        Ok(Some(snapshot)) => (
            [(
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable",
            )],
            Json(snapshot),
        )
            .into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "This version is no longer kept"),
        Err(e) => {
            tracing::error!(error = %e, "read draft recovery snapshot");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not load this version",
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
    let folders = state.hub.sync_value("folders")["folders"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| {
            serde_json::from_value::<crate::session_folders::SessionFolder>(value).ok()
        })
        .filter(|folder| {
            folder
                .owner_user_id
                .as_deref()
                .is_none_or(|owner| owner == auth.principal.user_id)
        })
        .collect::<Vec<_>>();
    match store.mutate_draft_document_in_workspace(&auth.principal.user_id, &mutation, &folders).await {
        Ok(DraftResult::Applied(document)) => {
            state.hub.announce_draft(&auth.principal.user_id, metadata(&document));
            Json(document).into_response()
        }
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

#[cfg(test)]
mod route_tests {
    use axum::{Router, extract::Path, routing::get};

    /// The History index and a numbered version share one path position; the
    /// literal segment must win, and the full list must stay where old
    /// clients read it.
    #[tokio::test]
    async fn history_routes_coexist() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let app = Router::new()
            .route("/api/drafts/{id}/history", get(|| async { "list" }))
            .route("/api/drafts/{id}/history/index", get(|| async { "index" }))
            .route(
                "/api/drafts/{id}/history/{revision}",
                get(|Path((_, revision)): Path<(String, i64)>| async move {
                    format!("version {revision}")
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let read = |path: &'static str| async move {
            let response = reqwest::get(format!("http://{address}{path}"))
                .await
                .unwrap();
            (response.status().as_u16(), response.text().await.unwrap())
        };
        assert_eq!(
            read("/api/drafts/doc/history").await,
            (200, "list".to_owned())
        );
        assert_eq!(
            read("/api/drafts/doc/history/index").await,
            (200, "index".to_owned())
        );
        assert_eq!(
            read("/api/drafts/doc/history/7").await,
            (200, "version 7".to_owned())
        );
        assert_eq!(read("/api/drafts/doc/history/latest").await.0, 400);
    }
}
