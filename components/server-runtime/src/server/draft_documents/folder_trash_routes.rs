//! Single-transaction folder-only Trash, server-receipt Undo, and optimistic
//! whole-plan Trash permanent deletion (`POST /api/trash/purge-plan`).

use super::trash_routes::{await_store_barrier, verify_operator_dataset};
use super::*;
use crate::session_folders::MovedSessionDelta;
use crate::store::draft_documents::{MovedSessionReceipt, PurgeDraftError, PurgeDraftTarget};
use std::collections::{HashMap, HashSet};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoTrashFolderOnlyRequest {
    receipt_id: String,
}

async fn run_trash_folder_only(
    state: Arc<AppState>,
    store: crate::store::Store,
    actor: crate::session_folders::FolderActor,
    folder_id: String,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let lock = state.hub.folder_tx_lock();
    let _lock_guard = lock.lock().await;
    let _tx_guard = state
        .hub
        .begin_folder_tx()
        .map_err(|msg| (StatusCode::CONFLICT, msg))?;
    await_store_barrier(&state).await?;
    let now = chrono::Utc::now().timestamp_millis();
    let (candidate, delta, folders_by_owner) = state
        .hub
        .preview_trash_folder_only(&actor, &folder_id, now)
        .map_err(|msg| (StatusCode::CONFLICT, msg))?;
    let receipt_id = format!("fto-{}", uuid::Uuid::new_v4().simple());
    let moved_sessions: Vec<MovedSessionReceipt> = delta
        .moved_sessions
        .iter()
        .map(|item| MovedSessionReceipt {
            session_id: item.session_id.clone(),
            was_implicit: item.was_implicit,
        })
        .collect();
    let outcome = store
        .atomic_folder_trash_only(
            &receipt_id,
            &folder_id,
            delta.owner.as_deref(),
            delta.trashed_at_ms,
            delta.destination.as_deref(),
            delta.previous_project.as_deref(),
            &delta.moved_folders,
            &moved_sessions,
            &folders_by_owner,
            &delta.pins,
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, folder = %folder_id, "atomic folder-only trash");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not move folder to Trash".to_owned(),
            )
        })?;
    // Publish in Hub memory ONLY after the single SQL transaction committed.
    state.hub.commit_folder_tx(candidate);
    for (doc_owner, doc) in &outcome.moved_draft_docs {
        state.hub.announce_draft(doc_owner, super::metadata(doc));
    }
    Ok(serde_json::json!({
        "receipt_id": outcome.receipt.receipt_id,
        "folder_id": outcome.receipt.folder_id,
        "trashed_at_ms": outcome.receipt.trashed_at_ms,
        "destination": outcome.receipt.destination,
        "previous_project": outcome.receipt.previous_project,
        "moved_folders": outcome.receipt.moved_folders,
        "moved_sessions": outcome.receipt.moved_sessions,
        "moved_drafts": outcome.receipt.moved_drafts,
    }))
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
    let Some(store) = state.store.clone() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
    let actor = folder_actor(&state, &auth.principal);
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let res = run_trash_folder_only(state, store, actor, folder_id).await;
        let _ = tx.send(res);
    });
    match rx.await {
        Ok(Ok(body)) => ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response(),
        Ok(Err((status, msg))) => error(status, &msg),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Folder Trash task ended unexpectedly",
        ),
    }
}

async fn run_undo_trash_folder_only(
    state: Arc<AppState>,
    store: crate::store::Store,
    actor: crate::session_folders::FolderActor,
    caller_user_id: String,
    folder_id: String,
    receipt_id: String,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let lock = state.hub.folder_tx_lock();
    let _lock_guard = lock.lock().await;
    let _tx_guard = state
        .hub
        .begin_folder_tx()
        .map_err(|msg| (StatusCode::CONFLICT, msg))?;
    await_store_barrier(&state).await?;
    let Some((receipt, undone_at)) = store
        .load_folder_trash_only_receipt(&receipt_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, receipt = %receipt_id, "load folder trash receipt");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not undo folder deletion".to_owned(),
            )
        })?
    else {
        return Err((
            StatusCode::CONFLICT,
            "This folder Trash undo receipt is no longer available".to_owned(),
        ));
    };
    if receipt.folder_id != folder_id {
        return Err((
            StatusCode::CONFLICT,
            "Undo receipt does not match this folder".to_owned(),
        ));
    }
    if undone_at.is_some() {
        return Err((
            StatusCode::CONFLICT,
            "This folder Trash action was already undone".to_owned(),
        ));
    }
    if !actor.sees_all
        && receipt
            .moved_drafts
            .iter()
            .any(|draft| draft.owner != caller_user_id)
    {
        return Err((
            StatusCode::FORBIDDEN,
            "Not allowed to restore another user's drafts".to_owned(),
        ));
    }
    let moved_sessions: Vec<MovedSessionDelta> = receipt
        .moved_sessions
        .iter()
        .map(|s| MovedSessionDelta {
            session_id: s.session_id.clone(),
            was_implicit: s.was_implicit,
        })
        .collect();
    let (candidate, folders_by_owner, pins) = state
        .hub
        .preview_undo_trash_folder_only(
            &actor,
            &receipt.folder_id,
            receipt.trashed_at_ms,
            receipt.destination.as_deref(),
            receipt.previous_project.as_deref(),
            &receipt.moved_folders,
            &moved_sessions,
        )
        .map_err(|msg| (StatusCode::CONFLICT, msg))?;
    let restored_drafts = match store
        .atomic_undo_folder_trash_only(&receipt, &folders_by_owner, &pins)
        .await
    {
        Ok(Ok(docs)) => docs,
        Ok(Err(msg)) => return Err((StatusCode::CONFLICT, msg)),
        Err(e) => {
            tracing::error!(error = %e, folder = %folder_id, "atomic undo folder trash");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not undo folder deletion".to_owned(),
            ));
        }
    };
    state.hub.commit_folder_tx(candidate);
    for (doc_owner, doc) in &restored_drafts {
        state.hub.announce_draft(doc_owner, super::metadata(doc));
    }
    let restored_ids: Vec<&str> = restored_drafts
        .iter()
        .map(|(_, doc)| doc.id.as_str())
        .collect();
    Ok(serde_json::json!({
        "restored_folder_id": folder_id,
        "restored_drafts": restored_ids,
    }))
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
    let Some(store) = state.store.clone() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
    let actor = folder_actor(&state, &auth.principal);
    let caller_user_id = auth.principal.user_id.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let res = run_undo_trash_folder_only(
            state,
            store,
            actor,
            caller_user_id,
            folder_id,
            request.receipt_id,
        )
        .await;
        let _ = tx.send(res);
    });
    match rx.await {
        Ok(Ok(body)) => ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response(),
        Ok(Err((status, msg))) => error(status, &msg),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Folder undo task ended unexpectedly",
        ),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedTrashedFolder {
    id: String,
    expected_trashed_at_ms: i64,
    #[serde(default)]
    expected_subfolders: Vec<String>,
    #[serde(default)]
    expected_sessions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedTrashedSession {
    id: String,
    expected_trashed_at_ms: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeTrashPlanRequest {
    #[serde(default)]
    folders: Vec<ExpectedTrashedFolder>,
    #[serde(default)]
    sessions: Vec<ExpectedTrashedSession>,
    #[serde(default)]
    drafts: Vec<PurgeDraftTarget>,
}

/// Validate the complete confirmed Trash deletion plan (exact folder
/// timestamps, subfolder membership, session timestamps, and draft optimistic
/// targets) before deleting ANY item (R3-13).
#[allow(clippy::type_complexity)]
fn validate_purge_plan_snapshot(
    state: &Arc<AppState>,
    actor: &crate::session_folders::FolderActor,
    principal: &ProductPrincipal,
    request: &PurgeTrashPlanRequest,
) -> Result<(Vec<(String, i64)>, Vec<(String, i64)>), (StatusCode, String)> {
    let folder_ids: Vec<String> = request.folders.iter().map(|f| f.id.clone()).collect();
    if !folder_ids.is_empty()
        && let Err(msg) = state.hub.check_trashed_folders(actor, &folder_ids)
    {
        return Err((StatusCode::CONFLICT, msg));
    }
    let (trashed_sessions, trashed_folders) = state.hub.trash_contents();
    let session_ts: HashMap<&str, i64> = trashed_sessions
        .iter()
        .map(|(id, _, ts)| (id.as_str(), *ts))
        .collect();
    let folder_ts: HashMap<&str, i64> = trashed_folders
        .iter()
        .filter_map(|f| f.trashed_at_ms.map(|ts| (f.id.as_str(), ts)))
        .collect();
    let mut confirmed_sessions: HashMap<String, i64> = HashMap::new();
    for s in &request.sessions {
        if session_ts.get(s.id.as_str()) != Some(&s.expected_trashed_at_ms) {
            return Err((
                StatusCode::CONFLICT,
                "A session in Trash changed since you opened it; refresh and try again".to_owned(),
            ));
        }
        confirmed_sessions.insert(s.id.clone(), s.expected_trashed_at_ms);
    }
    let mut folder_claims: Vec<(String, i64)> = Vec::new();
    for f in &request.folders {
        if folder_ts.get(f.id.as_str()) != Some(&f.expected_trashed_at_ms) {
            return Err((
                StatusCode::CONFLICT,
                "A folder in Trash changed or was restored elsewhere; refresh and try again"
                    .to_owned(),
            ));
        }
        let expected_subs: HashSet<&str> =
            f.expected_subfolders.iter().map(String::as_str).collect();
        let mut subtree: HashSet<String> = HashSet::from([f.id.clone()]);
        for candidate in &trashed_folders {
            if candidate.id != f.id
                && super::super::session_trash::folder_is_below_trashed(
                    &trashed_folders,
                    &candidate.id,
                    &f.id,
                )
            {
                if !expected_subs.contains(candidate.id.as_str()) {
                    return Err((StatusCode::CONFLICT, "Folder contents in Trash changed since you confirmed; refresh and try again".to_owned()));
                }
                subtree.insert(candidate.id.clone());
            }
        }
        let expected_sess: HashSet<&str> = f.expected_sessions.iter().map(String::as_str).collect();
        for (sid, _, ts) in &trashed_sessions {
            if state
                .hub
                .session_folder_placement(sid)
                .is_some_and(|placed| subtree.contains(&placed))
            {
                if !expected_sess.contains(sid.as_str()) {
                    return Err((StatusCode::CONFLICT, "Sessions inside this trashed folder changed since you confirmed; refresh and try again".to_owned()));
                }
                confirmed_sessions.entry(sid.clone()).or_insert(*ts);
            }
        }
        folder_claims.push((f.id.clone(), f.expected_trashed_at_ms));
    }
    let session_list: Vec<(String, i64)> = confirmed_sessions.into_iter().collect();
    if let Err(msg) =
        super::super::session_trash::validate_purge_sessions(state, principal, &session_list)
    {
        return Err((StatusCode::CONFLICT, msg));
    }
    Ok((folder_claims, session_list))
}

pub async fn purge_trash_plan(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Json(request): Json<PurgeTrashPlanRequest>,
) -> Response {
    if let Err((status, msg)) = verify_operator_dataset(&state, &auth, &headers, "Deleting Trash") {
        return error(status, &msg);
    }
    if request.folders.is_empty() && request.sessions.is_empty() && request.drafts.is_empty() {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Provide at least one Trash item to delete",
        );
    }
    let Some(store) = &state.store else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
    let actor = folder_actor(&state, &auth.principal);
    let owner = &auth.principal.user_id;
    let (mut folder_claims, session_list) =
        match validate_purge_plan_snapshot(&state, &actor, &auth.principal, &request) {
            Ok(pair) => pair,
            Err((status, msg)) => return error(status, &msg),
        };
    for target in &request.drafts {
        if let Ok(Some(doc)) = store.draft_document(owner, &target.id).await
            && !doc.deleted
            && let Some(pid) = doc.parent_id
        {
            folder_claims.push((pid, target.expected_trashed_at_ms));
        }
    }
    let _claim_guard = match state.hub.claim_folder_draft_purge(owner, &folder_claims) {
        Ok(guard) => guard,
        Err(msg) => return error(StatusCode::CONFLICT, &msg),
    };
    if let Err((status, msg)) = await_store_barrier(&state).await {
        return error(status, &msg);
    }
    let purged_drafts = if request.drafts.is_empty() {
        Vec::new()
    } else {
        match store.purge_trashed_drafts(owner, &request.drafts).await {
            Ok(Ok(purged)) => {
                state.hub.announce_draft_purged(owner, &purged);
                purged
            }
            Ok(Err(PurgeDraftError::Conflict(msg))) => return error(StatusCode::CONFLICT, &msg),
            Ok(Err(PurgeDraftError::Invalid(msg))) => {
                return error(StatusCode::UNPROCESSABLE_ENTITY, &msg);
            }
            Err(e) => {
                tracing::error!(error = %e, "purge drafts in Trash plan");
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Could not delete drafts permanently",
                );
            }
        }
    };
    let expected_folders: Vec<(String, i64)> = request
        .folders
        .iter()
        .map(|f| (f.id.clone(), f.expected_trashed_at_ms))
        .collect();
    if let Err(msg) = super::super::session_trash::execute_guarded_purge(
        &state,
        &actor,
        &expected_folders,
        &session_list,
    ) {
        return error(StatusCode::CONFLICT, &msg);
    }
    let folder_ids: Vec<&str> = request.folders.iter().map(|f| f.id.as_str()).collect();
    let draft_entries: Vec<_> = purged_drafts
        .iter()
        .map(|(id, revision)| serde_json::json!({ "id": id, "revision": revision }))
        .collect();
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({
            "purged_drafts": draft_entries,
            "purged_folders": folder_ids,
            "purged_sessions": session_list.iter().map(|(id, _)| id).collect::<Vec<_>>(),
        })),
    )
        .into_response()
}
