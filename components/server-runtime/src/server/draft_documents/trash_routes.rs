//! Optimistic permanent draft purge, session unsent draft archiving, and
//! durable session-draft restoration.

use super::*;
use crate::core::{DraftRestoreStage, PendingDraftTransferCommit};
use crate::store::draft_documents::{
    DraftArchiveOptions, DraftChange, DraftDocument, DraftKind, DraftMutation, DraftResult,
    PurgeDraftError, PurgeDraftTarget,
};

pub(super) fn verify_operator_dataset(
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

pub(super) async fn await_store_barrier(state: &Arc<AppState>) -> Result<(), (StatusCode, String)> {
    let Some(rx) = state.hub.store_barrier() else {
        if state.store.is_some() {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Durable storage writer is unavailable; try again".to_owned(),
            ));
        }
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

async fn finish_transfer_commit(
    state: &Arc<AppState>,
    mut commit: PendingDraftTransferCommit,
    rx: tokio::sync::oneshot::Receiver<Result<bool, String>>,
    failure_message: &str,
) -> Result<(), (StatusCode, String)> {
    match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
        Ok(Ok(Ok(true))) => {
            state.hub.commit_durable_draft_transfer(&mut commit);
            Ok(())
        }
        Ok(Ok(Ok(false))) | Ok(Ok(Err(_))) | Ok(Err(_)) | Err(_) => {
            state.hub.abort_durable_draft_transfer(&mut commit);
            Err((StatusCode::SERVICE_UNAVAILABLE, failure_message.to_owned()))
        }
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
    expected_content: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    expected_schedule: Option<crate::core::DraftSchedule>,
    #[serde(default)]
    expected_schedule_ms: Option<i64>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    attachments: Vec<serde_json::Value>,
}

fn schedule_matches(
    actual: Option<&crate::core::DraftSchedule>,
    expected: Option<&crate::core::DraftSchedule>,
    legacy_ms: Option<i64>,
) -> bool {
    if expected.is_some() || legacy_ms.is_none() {
        match (actual, expected) {
            (None, None) => true,
            (Some(a), Some(b)) => a.fire_at_ms == b.fire_at_ms && a.delivery == b.delivery,
            _ => false,
        }
    } else {
        actual.map(|s| s.fire_at_ms) == legacy_ms
    }
}

fn content_matches(
    actual: &[serde_json::Value],
    expected: Option<&[serde_json::Value]>,
    expected_text: &str,
) -> bool {
    if let Some(expected_blocks) = expected {
        return actual == expected_blocks;
    }
    if actual.is_empty() {
        return true;
    }
    actual.len() == 1
        && actual[0].get("type").and_then(serde_json::Value::as_str) == Some("text")
        && actual[0].get("text").and_then(serde_json::Value::as_str) == Some(expected_text)
}

async fn archive_and_remove_session_draft(
    state: &Arc<AppState>,
    store: &crate::store::Store,
    owner: &str,
    session_id: &str,
    row: &crate::core::QueuedMessage,
    request: TrashSessionDraftRequest,
) -> Response {
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
    let source_origin_json = match row.origin.as_ref().map(serde_json::to_string).transpose() {
        Ok(json) => json,
        Err(e) => {
            tracing::error!(error = %e, "serialize queue draft origin");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not archive draft to Trash",
            );
        }
    };
    let is_recovery = row.id.starts_with("recovery-");
    let doc_id = format!(
        "sd-{}",
        &crate::admin::hex_sha256(
            format!("{owner}:{session_id}:{}:{}", row.id, request.operation_id).as_bytes(),
        )[..32]
    );
    let archive_hash = crate::admin::hex_sha256(
        &serde_json::to_vec(&serde_json::json!({
            "session_id": session_id,
            "row_id": row.id,
            "text": row.text,
            "raw_content_json": raw_content_json,
            "schedule": row.schedule,
            "origin": row.origin,
            "is_recovery": is_recovery,
            "title": title,
            "attachments": request.attachments,
        }))
        .unwrap_or_default(),
    );
    let mutation = DraftMutation {
        operation_id: request.operation_id,
        document_id: doc_id,
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
    let opts = DraftArchiveOptions {
        raw_content_json: Some(&raw_content_json),
        source_origin_json: source_origin_json.as_deref(),
        source_is_recovery: is_recovery,
        request_hash_override: Some(&archive_hash),
        allow_session_provenance: true,
    };
    let document = match store
        .mutate_draft_document_with_options(owner, &mutation, &folders, &opts)
        .await
    {
        Ok(outcome) => match outcome.result {
            DraftResult::Applied(doc) => doc,
            DraftResult::Invalid(msg) => return error(StatusCode::UNPROCESSABLE_ENTITY, &msg),
            DraftResult::Conflict(_) | DraftResult::Purged { .. } => {
                return error(
                    StatusCode::CONFLICT,
                    "Archive operation conflicted; refresh and try again",
                );
            }
        },
        Err(e) => {
            tracing::error!(error = %e, "archive session draft to Trash");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not archive draft to Trash",
            );
        }
    };
    state.hub.announce_draft(owner, super::metadata(&document));
    match state.hub.begin_durable_draft_removal(session_id, &row.id) {
        Ok(Some((commit, rx))) => {
            if let Err((status, msg)) = finish_transfer_commit(
                state,
                commit,
                rx,
                "Archived in Trash, but could not durably remove the session draft; retry with the same operation",
            )
            .await
            {
                return error(status, &msg);
            }
        }
        Ok(None) => {}
        Err(msg) => return error(StatusCode::SERVICE_UNAVAILABLE, msg),
    }
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
    let session_lock = state.hub.draft_transfer_lock(&format!("sess:{session_id}"));
    let _session_guard = session_lock.lock().await;
    let (row, _reservation) = match state.hub.reserve_draft_for_transfer(&session_id, &draft_id) {
        Ok(Some(reserved)) => reserved,
        Ok(None) => {
            if let Ok(Some(existing_id)) = store
                .draft_operation_document(owner, &request.operation_id)
                .await
                && let Ok(Some(existing)) = store.draft_document(owner, &existing_id).await
                && existing.source_session_id.as_deref() == Some(session_id.as_str())
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
        || !content_matches(
            &row.content,
            request.expected_content.as_deref(),
            &request.expected_text,
        )
        || !schedule_matches(
            row.schedule.as_ref(),
            request.expected_schedule.as_ref(),
            request.expected_schedule_ms,
        )
    {
        return error(
            StatusCode::CONFLICT,
            "This draft was edited, attached, or rescheduled elsewhere; refresh and try again",
        );
    }
    if row.text.trim().is_empty() && row.content.is_empty() {
        match state.hub.begin_durable_draft_removal(&session_id, &row.id) {
            Ok(Some((commit, rx))) => {
                if let Err((status, msg)) = finish_transfer_commit(
                    &state,
                    commit,
                    rx,
                    "Could not durably remove empty draft; try again",
                )
                .await
                {
                    return error(status, &msg);
                }
            }
            Ok(None) => {}
            Err(msg) => return error(StatusCode::CONFLICT, msg),
        }
        return (
            [(header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "archived": false, "removed": true })),
        )
            .into_response();
    }
    archive_and_remove_session_draft(&state, store, owner, &session_id, &row, request).await
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

async fn stage_session_restore(
    state: &Arc<AppState>,
    session_id: &str,
    doc: &DraftDocument,
    content: Vec<serde_json::Value>,
    cmid: &str,
    origin: Option<crate::prompt_origin::DeliveryOrigin>,
    is_recovery: bool,
) -> Result<String, (StatusCode, String)> {
    match state.hub.begin_durable_draft_restore(
        session_id,
        doc.body.clone(),
        content,
        cmid.to_owned(),
        origin,
        is_recovery,
    ) {
        Ok(DraftRestoreStage::AlreadyPresent(id)) => {
            await_store_barrier(state).await?;
            Ok(id)
        }
        Ok(DraftRestoreStage::CommittedSync(id)) => {
            await_store_barrier(state).await?;
            Ok(id)
        }
        Ok(DraftRestoreStage::Pending(id, commit, rx)) => {
            finish_transfer_commit(
                state,
                commit,
                rx,
                "Could not durably restore draft to the session; try again",
            )
            .await?;
            Ok(id)
        }
        Err(msg) => Err((StatusCode::CONFLICT, msg)),
    }
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
        return error(
            StatusCode::FORBIDDEN,
            "Not allowed to restore into this session",
        );
    }
    // Acquire draft lock before session lock so concurrent restores never deadlock.
    let draft_lock = state
        .hub
        .draft_transfer_lock(&format!("draft:{owner}:{draft_id}"));
    let _draft_guard = draft_lock.lock().await;
    let session_lock = state
        .hub
        .draft_transfer_lock(&format!("sess:{}", request.session_id));
    let _session_guard = session_lock.lock().await;
    let cmid = format!(
        "restore-{}",
        &crate::admin::hex_sha256(
            format!("{owner}:{draft_id}:{}", request.operation_id).as_bytes(),
        )[..32]
    );
    match store.draft_tombstone(owner, &draft_id).await {
        Ok(Some(tomb)) if tomb.restore_op_id.as_deref() == Some(request.operation_id.as_str()) => {
            return (
                [(header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({
                    "session_id": request.session_id,
                    "restored_draft_id": tomb.restored_row_id,
                    "restored_cmid": cmid,
                    "purged_draft_id": draft_id,
                })),
            )
                .into_response();
        }
        Ok(Some(_)) => {
            return error(StatusCode::GONE, "This draft was permanently deleted");
        }
        Ok(None) => {}
        Err(e) => {
            tracing::error!(error = %e, "check draft restore tombstone");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "Could not restore draft");
        }
    }
    let record = match store.draft_trash_for_restore(owner, &draft_id).await {
        Ok(Some(record)) => record,
        Ok(None) => return error(StatusCode::NOT_FOUND, "Draft not found"),
        Err(e) => {
            tracing::error!(error = %e, "load draft for session restore");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "Could not restore draft");
        }
    };
    let doc = &record.document;
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
    let content: Vec<serde_json::Value> = record
        .raw_content_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<serde_json::Value>>(raw).ok())
        .unwrap_or_else(|| build_fallback_content(doc));
    let origin = match record.source_origin_json.as_deref() {
        Some(raw) => serde_json::from_str::<crate::prompt_origin::DeliveryOrigin>(raw).ok(),
        None => Some(crate::prompt_origin::DeliveryOrigin::from_caller(
            &auth.caller,
        )),
    };
    let restored_id = match stage_session_restore(
        &state,
        &request.session_id,
        doc,
        content,
        &cmid,
        origin,
        record.source_is_recovery,
    )
    .await
    {
        Ok(id) => id,
        Err((status, msg)) => return error(status, &msg),
    };
    let target = PurgeDraftTarget {
        id: draft_id.clone(),
        expected_revision: request.expected_revision,
        expected_trashed_at_ms: request.expected_trashed_at_ms,
    };
    match store
        .purge_trashed_drafts_with_restore_receipt(
            owner,
            &[target],
            Some((&request.operation_id, &restored_id)),
        )
        .await
    {
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
