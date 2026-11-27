//! Atomic folder-only Trash and exact Undo endpoints.

use super::trash_routes::{await_store_barrier, verify_operator_dataset};
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoMovedDraft {
    owner: String,
    id: String,
    metadata_revision: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoTrashFolderOnlyRequest {
    expected_trashed_at_ms: i64,
    #[serde(default)]
    destination: Option<String>,
    #[serde(default)]
    previous_project: Option<String>,
    #[serde(default)]
    moved_folders: Vec<String>,
    #[serde(default)]
    moved_sessions: Vec<String>,
    #[serde(default)]
    moved_drafts: Vec<UndoMovedDraft>,
}

pub async fn trash_folder_only(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Path(folder_id): Path<String>,
) -> Response {
    if let Err((status, msg)) =
        verify_operator_dataset(&state, &auth, &headers, "Moving folders to Trash")
    {
        return error(status, &msg);
    }
    let Some(store) = &state.store else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
    let actor = folder_actor(&state, &auth.principal);
    let guard = match state.hub.begin_folder_trash_only(&actor, &folder_id) {
        Ok(guard) => guard,
        Err(msg) => return error(StatusCode::CONFLICT, &msg),
    };
    // Reparent drafts in DB BEFORE moving the folder to Trash while the fence
    // rejects concurrent folder mutations or draft filings into `folder_id`.
    let moved_drafts = match store
        .reparent_drafts_out_of_folder(
            guard.owner.as_deref(),
            &folder_id,
            guard.destination.as_deref(),
        )
        .await
    {
        Ok(docs) => docs,
        Err(e) => {
            tracing::error!(error = %e, folder = %folder_id, "reparent drafts out of folder");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not move drafts out of folder before trashing",
            );
        }
    };
    let now = chrono::Utc::now().timestamp_millis();
    let delta = match state.hub.trash_folder_only(&actor, &folder_id, now) {
        Ok(delta) => delta,
        Err(msg) => {
            let rollback_targets: Vec<(String, String, i64)> = moved_drafts
                .iter()
                .map(|(doc_owner, doc)| (doc_owner.clone(), doc.id.clone(), doc.metadata_revision))
                .collect();
            let _ = store
                .undo_reparent_drafts_to_folder(
                    &folder_id,
                    guard.destination.as_deref(),
                    &rollback_targets,
                )
                .await;
            return error(StatusCode::CONFLICT, &msg);
        }
    };
    if let Err((status, msg)) = await_store_barrier(&state).await {
        return error(status, &msg);
    }
    drop(guard);
    for (doc_owner, doc) in &moved_drafts {
        state.hub.announce_draft(doc_owner, super::metadata(doc));
    }
    let draft_summaries: Vec<_> = moved_drafts
        .iter()
        .map(|(doc_owner, doc)| {
            serde_json::json!({
                "owner": doc_owner,
                "id": doc.id,
                "revision": doc.revision,
                "metadata_revision": doc.metadata_revision,
            })
        })
        .collect();
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({
            "folder_id": delta.folder_id,
            "trashed_at_ms": delta.trashed_at_ms,
            "destination": delta.destination,
            "previous_project": delta.previous_project,
            "moved_folders": delta.moved_folders,
            "moved_sessions": delta.moved_sessions,
            "moved_drafts": draft_summaries,
        })),
    )
        .into_response()
}

pub async fn undo_trash_folder_only(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Path(folder_id): Path<String>,
    Json(request): Json<UndoTrashFolderOnlyRequest>,
) -> Response {
    if let Err((status, msg)) = verify_operator_dataset(&state, &auth, &headers, "Undoing Trash") {
        return error(status, &msg);
    }
    let Some(store) = &state.store else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
    let actor = folder_actor(&state, &auth.principal);
    if !actor.sees_all
        && request
            .moved_drafts
            .iter()
            .any(|draft| draft.owner != auth.principal.user_id)
    {
        return error(
            StatusCode::FORBIDDEN,
            "Not allowed to restore another user's drafts",
        );
    }
    if let Err(msg) = state.hub.undo_trash_folder_only(
        &actor,
        &folder_id,
        request.expected_trashed_at_ms,
        request.destination.as_deref(),
        request.previous_project.as_deref(),
        &request.moved_folders,
        &request.moved_sessions,
    ) {
        return error(StatusCode::CONFLICT, &msg);
    }
    if let Err((status, msg)) = await_store_barrier(&state).await {
        return error(status, &msg);
    }
    let targets: Vec<(String, String, i64)> = request
        .moved_drafts
        .into_iter()
        .map(|draft| (draft.owner, draft.id, draft.metadata_revision))
        .collect();
    let restored_drafts = match store
        .undo_reparent_drafts_to_folder(&folder_id, request.destination.as_deref(), &targets)
        .await
    {
        Ok(docs) => docs,
        Err(e) => {
            tracing::error!(error = %e, folder = %folder_id, "undo reparent drafts into folder");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Folder restored, but could not move all drafts back into it",
            );
        }
    };
    for (doc_owner, doc) in &restored_drafts {
        state.hub.announce_draft(doc_owner, super::metadata(doc));
    }
    let restored_ids: Vec<&str> = restored_drafts
        .iter()
        .map(|(_, doc)| doc.id.as_str())
        .collect();
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({
            "restored_folder_id": folder_id,
            "restored_drafts": restored_ids,
        })),
    )
        .into_response()
}
