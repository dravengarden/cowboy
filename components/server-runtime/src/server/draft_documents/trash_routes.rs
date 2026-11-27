//! Optimistic permanent draft purge, atomic folder-only Trash, session unsent
//! draft archiving, and durable session-draft restoration.

use super::*;
use crate::store::draft_documents::{
    DraftChange, DraftDocument, DraftKind, DraftMutation, DraftResult, PurgeDraftError,
    PurgeDraftTarget,
};

fn verify_operator_dataset(
    state: &Arc<AppState>,
    auth: &AuthenticatedProductRequest,
    headers: &HeaderMap,
    action_label: &str,
) -> Result<(), (StatusCode, String)> {
    if !auth.principal.can_reorder() {
        return Err((
            StatusCode::FORBIDDEN,
            format!("{action_label} requires an operator account"),
        ));
    }
    let Some(dataset) = headers
        .get("x-cowboy-dataset")
        .and_then(|v| v.to_str().ok())
    else {
        return Err((StatusCode::CONFLICT, "Draft dataset is required".to_owned()));
    };
    if !sync_dataset::matches(&state.service_id, &auth.principal, dataset) {
        return Err((
            StatusCode::CONFLICT,
            "Draft dataset changed; reload before continuing".to_owned(),
        ));
    }
    Ok(())
}

async fn await_store_barrier(state: &Arc<AppState>) -> Result<(), (StatusCode, String)> {
    let Some(rx) = state.hub.store_barrier() else {
        return Ok(());
    };
    match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
        Ok(Ok(Ok(_))) => Ok(()),
        Ok(Ok(Err(message))) => Err((StatusCode::SERVICE_UNAVAILABLE, message)),
        Ok(Err(_)) | Err(_) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage synchronization timed out; try again".to_owned(),
        )),
    }
}

fn sanitize_archive_title(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed: String = cleaned.chars().take(160).collect();
    if trimmed.is_empty() {
        "Draft".to_owned()
    } else {
        trimmed
    }
}

/// Serve `GET /api/drafts/purge` as a normal read of the document literally
/// named `"purge"`, so the literal `POST /api/drafts/purge` route never masks it.
pub async fn read_literal_purge(
    state: State<Arc<AppState>>,
    auth: Extension<AuthenticatedProductRequest>,
) -> Response {
    super::read(state, auth, Path("purge".to_owned())).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeDraftsRequest {
    targets: Vec<PurgeDraftTarget>,
}

pub async fn purge(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Json(request): Json<PurgeDraftsRequest>,
) -> Response {
    if let Err((status, msg)) = verify_operator_dataset(&state, &auth, &headers, "Deleting drafts")
    {
        return error(status, &msg);
    }
    if request.targets.is_empty() || request.targets.len() > 500 {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Provide between 1 and 500 draft targets",
        );
    }
    let Some(store) = &state.store else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
    let owner = &auth.principal.user_id;
    let mut folder_claims: Vec<(String, i64)> = Vec::new();
    for target in &request.targets {
        match store.draft_document(owner, &target.id).await {
            Ok(Some(doc)) if !doc.deleted => {
                if let Some(parent_id) = doc.parent_id {
                    folder_claims.push((parent_id, target.expected_trashed_at_ms));
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!(error = %e, "inspect draft before purge");
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Could not delete drafts permanently",
                );
            }
        }
    }
    let _claim_guard = match state.hub.claim_folder_draft_purge(owner, &folder_claims) {
        Ok(guard) => guard,
        Err(message) => return error(StatusCode::CONFLICT, &message),
    };
    if let Err((status, msg)) = await_store_barrier(&state).await {
        return error(status, &msg);
    }
    match store.purge_trashed_drafts(owner, &request.targets).await {
        Ok(Ok(purged)) => {
            state.hub.announce_draft_purged(owner, &purged);
            let entries: Vec<_> = purged
                .iter()
                .map(|(id, revision)| serde_json::json!({ "id": id, "revision": revision }))
                .collect();
            let ids: Vec<&str> = purged.iter().map(|(id, _)| id.as_str()).collect();
            (
                [(header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "purged": ids, "entries": entries })),
            )
                .into_response()
        }
        Ok(Err(PurgeDraftError::Conflict(message))) => error(StatusCode::CONFLICT, &message),
        Ok(Err(PurgeDraftError::Invalid(message))) => {
            error(StatusCode::UNPROCESSABLE_ENTITY, &message)
        }
        Err(e) => {
            tracing::error!(error = %e, "purge trashed drafts");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not delete drafts permanently",
            )
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrashSessionDraftRequest {
    operation_id: String,
    expected_text: String,
    #[serde(default)]
    expected_schedule_ms: Option<i64>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    attachments: Vec<serde_json::Value>,
}

struct SessionDraftArchiveCtx<'a> {
    owner: &'a str,
    session_id: &'a str,
    draft_id: &'a str,
    doc_id: &'a str,
    already_recorded: bool,
}

async fn persist_and_remove_session_draft(
    state: &Arc<AppState>,
    store: &crate::store::Store,
    ctx: &SessionDraftArchiveCtx<'_>,
    row: &crate::core::QueuedMessage,
    request: TrashSessionDraftRequest,
) -> Response {
    let SessionDraftArchiveCtx {
        owner,
        session_id,
        draft_id,
        doc_id,
        already_recorded,
    } = *ctx;
    let title = sanitize_archive_title(if request.title.trim().is_empty() {
        row.text
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("Draft")
    } else {
        &request.title
    });
    let raw_content_json = match serde_json::to_string(&row.content) {
        Ok(json) => json,
        Err(e) => {
            tracing::error!(error = %e, "serialize queue draft raw content");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not archive draft to Trash",
            );
        }
    };
    let mutation = DraftMutation {
        operation_id: request.operation_id,
        document_id: doc_id.to_owned(),
        expected_revision: 0,
        change: DraftChange::Create {
            kind: DraftKind::Document,
            title,
            parent_id: None,
            body: row.text.clone(),
            attachments: request.attachments,
            trashed: true,
            source_session_id: Some(session_id.to_owned()),
        },
    };
    let folders = super::owner_folders(state, owner);
    let document = match store
        .mutate_draft_document_with_raw_content(owner, &mutation, &folders, Some(&raw_content_json))
        .await
    {
        Ok(DraftResult::Applied(doc)) => doc,
        Ok(DraftResult::Invalid(msg)) => return error(StatusCode::UNPROCESSABLE_ENTITY, &msg),
        Ok(DraftResult::Conflict(_) | DraftResult::Purged { .. }) => {
            return error(
                StatusCode::CONFLICT,
                "Archive operation conflicted; refresh and try again",
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "archive session draft to Trash");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not archive draft to Trash",
            );
        }
    };
    if !state.hub.remove_draft_if_exact(
        session_id,
        draft_id,
        &row.text,
        &row.content,
        request.expected_schedule_ms,
    ) {
        if !already_recorded && let Some(trashed_at) = document.trashed_at_ms {
            let _ = store
                .purge_trashed_drafts(
                    owner,
                    &[PurgeDraftTarget {
                        id: document.id.clone(),
                        expected_revision: document.revision,
                        expected_trashed_at_ms: trashed_at,
                    }],
                )
                .await;
        }
        return error(
            StatusCode::CONFLICT,
            "This draft was edited, scheduled, or sent; it has been kept",
        );
    }
    state.hub.announce_draft(owner, super::metadata(&document));
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({
            "archived": true,
            "removed": true,
            "draft": super::metadata(&document),
        })),
    )
        .into_response()
}

pub async fn trash_session_draft(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Path((session_id, draft_id)): Path<(String, String)>,
    Json(request): Json<TrashSessionDraftRequest>,
) -> Response {
    if let Err((status, msg)) =
        verify_operator_dataset(&state, &auth, &headers, "Moving drafts to Trash")
    {
        return error(status, &msg);
    }
    let Some(store) = &state.store else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
    let owner = &auth.principal.user_id;
    let Some(info) = state.hub.session_info(&session_id) else {
        return error(StatusCode::NOT_FOUND, "Session no longer exists");
    };
    if !auth
        .principal
        .can_mutate(info.meta.owner_user_id.as_deref())
    {
        return error(StatusCode::FORBIDDEN, "Not allowed to change this session");
    }
    let doc_id = format!(
        "sd-{}",
        &crate::admin::hex_sha256(
            format!("{owner}:{session_id}:{}", request.operation_id).as_bytes(),
        )[..32]
    );
    let already_recorded = store
        .draft_operation_document(owner, &request.operation_id)
        .await
        .ok()
        .flatten()
        .is_some();
    let row = match state.hub.inspect_draft(&session_id, &draft_id) {
        Ok(Some(row)) => row,
        Ok(None) => {
            if already_recorded
                && let Ok(Some(existing)) = store.draft_document(owner, &doc_id).await
            {
                return (
                    [(header::CACHE_CONTROL, "no-store")],
                    Json(serde_json::json!({
                        "archived": true,
                        "removed": true,
                        "draft": super::metadata(&existing),
                    })),
                )
                    .into_response();
            }
            return error(StatusCode::NOT_FOUND, "Draft no longer exists");
        }
        Err(msg) => return error(StatusCode::CONFLICT, msg),
    };
    if row.text != request.expected_text
        || row.schedule.as_ref().map(|s| s.fire_at_ms) != request.expected_schedule_ms
    {
        return error(
            StatusCode::CONFLICT,
            "This draft was edited or rescheduled elsewhere; refresh and try again",
        );
    }
    if row.text.trim().is_empty() && row.content.is_empty() {
        if !state.hub.remove_draft_if_exact(
            &session_id,
            &draft_id,
            &row.text,
            &row.content,
            request.expected_schedule_ms,
        ) {
            return error(
                StatusCode::CONFLICT,
                "This draft changed before it could be removed",
            );
        }
        return (
            [(header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "archived": false, "removed": true })),
        )
            .into_response();
    }
    let ctx = SessionDraftArchiveCtx {
        owner,
        session_id: &session_id,
        draft_id: &draft_id,
        doc_id: &doc_id,
        already_recorded,
    };
    persist_and_remove_session_draft(&state, store, &ctx, &row, request).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreToSessionRequest {
    operation_id: String,
    session_id: String,
    expected_revision: i64,
    expected_trashed_at_ms: i64,
}

fn build_fallback_content(doc: &DraftDocument) -> Vec<serde_json::Value> {
    let mut blocks = Vec::new();
    if !doc.body.trim().is_empty() {
        blocks.push(serde_json::json!({ "type": "text", "text": doc.body }));
    }
    for att in &doc.attachments {
        if let Some(block) = att.get("block") {
            blocks.push(block.clone());
        }
    }
    blocks
}

pub async fn restore_to_session(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Path(draft_id): Path<String>,
    Json(request): Json<RestoreToSessionRequest>,
) -> Response {
    if let Err((status, msg)) = verify_operator_dataset(&state, &auth, &headers, "Restoring drafts")
    {
        return error(status, &msg);
    }
    let Some(store) = &state.store else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
    let owner = &auth.principal.user_id;
    let Some(info) = state.hub.session_info(&request.session_id) else {
        return error(StatusCode::CONFLICT, "Source session no longer exists");
    };
    if !auth
        .principal
        .can_mutate(info.meta.owner_user_id.as_deref())
    {
        return error(StatusCode::FORBIDDEN, "Not allowed to change this session");
    }
    let loaded = match store.draft_trash_for_restore(owner, &draft_id).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            return error(
                StatusCode::CONFLICT,
                "Draft is no longer in Trash; refresh and try again",
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "load trashed draft for restore");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "Could not restore draft");
        }
    };
    let (doc, raw_content_json) = loaded;
    if !doc.deleted
        || doc.revision != request.expected_revision
        || doc.trashed_at_ms != Some(request.expected_trashed_at_ms)
        || doc.source_session_id.as_deref() != Some(request.session_id.as_str())
    {
        return error(
            StatusCode::CONFLICT,
            "Draft changed or was already restored elsewhere; refresh and try again",
        );
    }
    let content: Vec<serde_json::Value> = raw_content_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<serde_json::Value>>(raw).ok())
        .unwrap_or_else(|| build_fallback_content(&doc));
    let cmid = format!(
        "restore-{}",
        &crate::admin::hex_sha256(
            format!("{owner}:{draft_id}:{}", request.operation_id).as_bytes(),
        )[..32]
    );
    let caller = auth.caller.clone();
    let restored_id = match state.hub.add_draft_if_live(
        &request.session_id,
        doc.body.clone(),
        content,
        cmid.clone(),
        &caller,
    ) {
        Ok(id) => id,
        Err(msg) => return error(StatusCode::CONFLICT, &msg),
    };
    // Wait until the restored draft is durably persisted to the destination
    // session BEFORE consuming the Trash archive.
    if let Err((status, msg)) = await_store_barrier(&state).await {
        return error(status, &msg);
    }
    let target = PurgeDraftTarget {
        id: draft_id.clone(),
        expected_revision: request.expected_revision,
        expected_trashed_at_ms: request.expected_trashed_at_ms,
    };
    match store.purge_trashed_drafts(owner, &[target]).await {
        Ok(Ok(purged)) => {
            state.hub.announce_draft_purged(owner, &purged);
            (
                [(header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({
                    "session_id": request.session_id,
                    "restored_draft_id": restored_id,
                    "restored_cmid": cmid,
                    "purged_draft_id": draft_id,
                })),
            )
                .into_response()
        }
        Ok(Err(PurgeDraftError::Conflict(msg))) => error(StatusCode::CONFLICT, &msg),
        Ok(Err(PurgeDraftError::Invalid(msg))) => error(StatusCode::UNPROCESSABLE_ENTITY, &msg),
        Err(e) => {
            tracing::error!(error = %e, "consume restored draft archive");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Restored to session, but could not remove Trash archive; refresh Trash",
            )
        }
    }
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
    let owner = &auth.principal.user_id;
    let actor = folder_actor(&state, &auth.principal);
    let now = chrono::Utc::now().timestamp_millis();
    let delta = match state.hub.trash_folder_only(&actor, &folder_id, now) {
        Ok(delta) => delta,
        Err(msg) => return error(StatusCode::CONFLICT, &msg),
    };
    if let Err((status, msg)) = await_store_barrier(&state).await {
        return error(status, &msg);
    }
    let moved_drafts = match store
        .reparent_drafts_out_of_folder(owner, &folder_id, delta.destination.as_deref())
        .await
    {
        Ok(docs) => docs,
        Err(e) => {
            tracing::error!(error = %e, folder = %folder_id, "reparent drafts out of folder");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Folder moved to Trash, but could not move all drafts out of it",
            );
        }
    };
    for doc in &moved_drafts {
        state.hub.announce_draft(owner, super::metadata(doc));
    }
    let draft_summaries: Vec<_> = moved_drafts
        .iter()
        .map(|doc| {
            serde_json::json!({
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
            "destination": delta.destination,
            "previous_project": delta.previous_project,
            "moved_folders": delta.moved_folders,
            "moved_sessions": delta.moved_sessions,
            "moved_drafts": draft_summaries,
        })),
    )
        .into_response()
}
