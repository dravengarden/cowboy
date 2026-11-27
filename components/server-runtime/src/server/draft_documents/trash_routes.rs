//! Optimistic permanent draft purge, single-transaction session unsent draft
//! archiving, and single-transaction durable session-draft restoration.

use super::*;
use crate::store::draft_documents::{
    AtomicArchiveDraftArgs, AtomicRestoreDraftArgs, AtomicRestoreOutcome, DraftArchiveOptions,
    DraftChange, DraftDocument, DraftKind, DraftMutation, DraftResult, PurgeDraftError,
    PurgeDraftTarget,
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
    match rx.await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(message)) => Err((StatusCode::SERVICE_UNAVAILABLE, message)),
        Err(_) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage synchronization was interrupted; try again".to_owned(),
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
    expected_content: Vec<serde_json::Value>,
    expected_schedule: Option<crate::core::DraftSchedule>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    attachments: Vec<serde_json::Value>,
}

fn schedule_exact_eq(
    actual: Option<&crate::core::DraftSchedule>,
    expected: Option<&crate::core::DraftSchedule>,
) -> bool {
    match (actual, expected) {
        (None, None) => true,
        (Some(a), Some(b)) => a.fire_at_ms == b.fire_at_ms && a.delivery == b.delivery,
        _ => false,
    }
}

/// Keep only attachment projections whose embedded ACP block matches a real
/// non-text block from the authoritative session row (R3-12).
fn filter_verified_attachments(
    row_content: &[serde_json::Value],
    caller_attachments: Vec<serde_json::Value>,
) -> Vec<serde_json::Value> {
    let mut available: Vec<&serde_json::Value> = row_content
        .iter()
        .filter(|b| {
            matches!(
                b.get("type").and_then(serde_json::Value::as_str),
                Some("image" | "resource")
            )
        })
        .collect();
    let mut verified = Vec::with_capacity(caller_attachments.len());
    for att in caller_attachments {
        if let Some(block) = att.get("block")
            && let Some(pos) = available.iter().position(|b| *b == block)
        {
            available.swap_remove(pos);
            verified.push(att);
        }
    }
    verified
}

struct PreparedArchive {
    mutation: DraftMutation,
    raw_content_json: String,
    source_origin_json: Option<String>,
    is_recovery: bool,
    archive_hash: String,
}

fn prepare_archive_payload(
    owner: &str,
    session_id: &str,
    row: &crate::core::QueuedMessage,
    request: TrashSessionDraftRequest,
) -> Result<PreparedArchive, (StatusCode, String)> {
    let title = sanitize_archive_title(if request.title.trim().is_empty() {
        row.text
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("Draft")
    } else {
        &request.title
    });
    let raw_content_json = serde_json::to_string(&row.content).map_err(|e| {
        tracing::error!(error = %e, "serialize queue draft raw content");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not archive draft to Trash".to_owned(),
        )
    })?;
    let source_origin_json = row
        .origin
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| {
            tracing::error!(error = %e, "serialize queue draft origin");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not archive draft to Trash".to_owned(),
            )
        })?;
    let is_recovery = row.id.starts_with("recovery-");
    let durable_row_key = row.cmid.as_deref().unwrap_or(&row.id);
    let doc_id = format!(
        "sd-{}",
        &crate::admin::hex_sha256(
            format!(
                "{owner}:{session_id}:{durable_row_key}:{}",
                request.operation_id
            )
            .as_bytes()
        )[..32]
    );
    let archive_hash = crate::admin::hex_sha256(
        &serde_json::to_vec(&serde_json::json!({
            "session_id": session_id,
            "row_key": durable_row_key,
            "text": row.text,
            "raw_content_json": raw_content_json,
            "schedule": row.schedule,
            "origin": row.origin,
            "is_recovery": is_recovery,
        }))
        .unwrap_or_default(),
    );
    let verified_attachments = filter_verified_attachments(&row.content, request.attachments);
    let mutation = DraftMutation {
        operation_id: request.operation_id,
        document_id: doc_id,
        expected_revision: 0,
        change: DraftChange::Create {
            kind: DraftKind::Document,
            title,
            parent_id: None,
            body: row.text.clone(),
            attachments: verified_attachments,
            trashed: true,
            source_session_id: Some(session_id.to_owned()),
        },
    };
    Ok(PreparedArchive {
        mutation,
        raw_content_json,
        source_origin_json,
        is_recovery,
        archive_hash,
    })
}

async fn run_archive_session_draft(
    state: Arc<AppState>,
    store: crate::store::Store,
    owner: String,
    session_id: String,
    draft_id: String,
    request: TrashSessionDraftRequest,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let session_lock = state.hub.draft_transfer_lock(&format!("sess:{session_id}"));
    let _session_guard = session_lock.lock().await;
    let (row, _reservation) = match state.hub.reserve_draft_for_transfer(&session_id, &draft_id) {
        Ok(Some(reserved)) => reserved,
        Ok(None) => {
            if let Ok(Some(existing_id)) = store
                .draft_operation_document(&owner, &request.operation_id)
                .await
                && let Ok(Some(existing)) = store.draft_document(&owner, &existing_id).await
                && existing.source_session_id.as_deref() == Some(session_id.as_str())
            {
                return Ok(
                    serde_json::json!({ "archived": true, "removed": true, "draft": super::metadata(&existing) }),
                );
            }
            return Err((StatusCode::NOT_FOUND, "Draft no longer exists".to_owned()));
        }
        Err(msg) => return Err((StatusCode::CONFLICT, msg.to_owned())),
    };
    if row.text != request.expected_text
        || row.content != request.expected_content
        || !schedule_exact_eq(row.schedule.as_ref(), request.expected_schedule.as_ref())
    {
        return Err((
            StatusCode::CONFLICT,
            "This draft was edited, attached, or rescheduled elsewhere; refresh and try again"
                .to_owned(),
        ));
    }
    let (_, _, mut commit) = state
        .hub
        .stage_atomic_draft_removal(&session_id, &row.id)
        .map_err(|msg| (StatusCode::CONFLICT, msg.to_owned()))?;
    if let Err(err) = await_store_barrier(&state).await {
        state.hub.abort_durable_draft_transfer(&mut commit);
        return Err(err);
    }
    let (next_queue, next_drafts) = match state.hub.snapshot_for_staged_transfer(&commit) {
        Ok(pair) => pair,
        Err(msg) => {
            state.hub.abort_durable_draft_transfer(&mut commit);
            return Err((StatusCode::CONFLICT, msg.to_owned()));
        }
    };
    if row.text.trim().is_empty() && row.content.is_empty() {
        if let Err(e) = store
            .update_pending(&session_id, &next_queue, &next_drafts)
            .await
        {
            tracing::error!(error = %e, "remove empty session draft");
            state.hub.abort_durable_draft_transfer(&mut commit);
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Could not durably remove empty draft; try again".to_owned(),
            ));
        }
        state.hub.commit_durable_draft_transfer(&mut commit);
        return Ok(serde_json::json!({ "archived": false, "removed": true }));
    }
    let prepared = match prepare_archive_payload(&owner, &session_id, &row, request) {
        Ok(p) => p,
        Err(err) => {
            state.hub.abort_durable_draft_transfer(&mut commit);
            return Err(err);
        }
    };
    let folders = super::owner_folders(&state, &owner);
    let opts = DraftArchiveOptions {
        raw_content_json: Some(&prepared.raw_content_json),
        source_origin_json: prepared.source_origin_json.as_deref(),
        source_is_recovery: prepared.is_recovery,
        request_hash_override: Some(&prepared.archive_hash),
        allow_session_provenance: true,
    };
    match store
        .atomic_archive_session_draft(AtomicArchiveDraftArgs {
            owner: &owner,
            session_id: &session_id,
            mutation: &prepared.mutation,
            shared_folders: &folders,
            opts: &opts,
            next_queue: &next_queue,
            next_drafts: &next_drafts,
        })
        .await
    {
        Ok(DraftResult::Applied(doc)) => {
            state.hub.commit_durable_draft_transfer(&mut commit);
            state.hub.announce_draft(&owner, super::metadata(&doc));
            Ok(
                serde_json::json!({ "archived": true, "removed": true, "draft": super::metadata(&doc) }),
            )
        }
        Ok(DraftResult::Invalid(msg)) => {
            state.hub.abort_durable_draft_transfer(&mut commit);
            Err((StatusCode::UNPROCESSABLE_ENTITY, msg))
        }
        Ok(DraftResult::Conflict(_) | DraftResult::Purged { .. }) => {
            state.hub.abort_durable_draft_transfer(&mut commit);
            Err((
                StatusCode::CONFLICT,
                "Archive operation conflicted; refresh and try again".to_owned(),
            ))
        }
        Err(e) => {
            tracing::error!(error = %e, "atomic archive session draft");
            state.hub.abort_durable_draft_transfer(&mut commit);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not archive draft to Trash".to_owned(),
            ))
        }
    }
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
    let Some(store) = state.store.clone() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
    let Some(info) = state.hub.session_info(&session_id) else {
        return error(StatusCode::NOT_FOUND, "Session no longer exists");
    };
    if !auth
        .principal
        .can_mutate(info.meta.owner_user_id.as_deref())
    {
        return error(StatusCode::FORBIDDEN, "Not allowed to change this session");
    }
    // Run in a request-independent task so client cancellation or timeout never
    // drops locks/fences while the atomic SQL commit is in flight (R3-5).
    let (tx, rx) = tokio::sync::oneshot::channel();
    let owner = auth.principal.user_id.clone();
    tokio::spawn(async move {
        let res =
            run_archive_session_draft(state, store, owner, session_id, draft_id, request).await;
        let _ = tx.send(res);
    });
    match rx.await {
        Ok(Ok(body)) => ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response(),
        Ok(Err((status, msg))) => error(status, &msg),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Draft archive task ended unexpectedly",
        ),
    }
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

#[allow(clippy::type_complexity)]
async fn load_verified_restore_record(
    store: &crate::store::Store,
    owner: &str,
    draft_id: &str,
    cmid: &str,
    request: &RestoreToSessionRequest,
) -> Result<
    Result<crate::store::draft_documents::DraftTrashRestoreRecord, serde_json::Value>,
    (StatusCode, String),
> {
    if let Some(tomb) = store.draft_tombstone(owner, draft_id).await.map_err(|e| {
        tracing::error!(error = %e, "check draft restore tombstone");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not restore draft".to_owned(),
        )
    })? {
        if tomb.restore_op_id.as_deref() == Some(request.operation_id.as_str())
            && tomb.restore_session_id.as_deref() == Some(request.session_id.as_str())
            && tomb.restore_expected_revision == Some(request.expected_revision)
            && tomb.restore_expected_trashed_at_ms == Some(request.expected_trashed_at_ms)
        {
            return Ok(Err(serde_json::json!({
                "session_id": request.session_id,
                "restored_draft_id": tomb.restored_row_id,
                "restored_cmid": cmid,
                "purged_draft_id": draft_id,
            })));
        }
        return Err((
            StatusCode::GONE,
            "This draft was permanently deleted".to_owned(),
        ));
    }
    let Some(record) = store
        .draft_trash_for_restore(owner, draft_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "load draft for session restore");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not restore draft".to_owned(),
            )
        })?
    else {
        return Err((StatusCode::NOT_FOUND, "Draft not found".to_owned()));
    };
    let doc = &record.document;
    if !doc.deleted
        || doc.revision != request.expected_revision
        || doc.trashed_at_ms != Some(request.expected_trashed_at_ms)
        || doc.source_session_id.as_deref() != Some(request.session_id.as_str())
    {
        return Err((
            StatusCode::CONFLICT,
            "Draft changed or was already restored elsewhere; refresh and try again".to_owned(),
        ));
    }
    Ok(Ok(record))
}

async fn run_restore_to_session(
    state: Arc<AppState>,
    store: crate::store::Store,
    owner: String,
    draft_id: String,
    request: RestoreToSessionRequest,
) -> Result<serde_json::Value, (StatusCode, String)> {
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
            format!("{owner}:{draft_id}:{}", request.operation_id).as_bytes()
        )[..32]
    );
    let record =
        match load_verified_restore_record(&store, &owner, &draft_id, &cmid, &request).await? {
            Ok(rec) => rec,
            Err(replayed) => return Ok(replayed),
        };
    let doc = &record.document;
    let content: Vec<serde_json::Value> = record
        .raw_content_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<serde_json::Value>>(raw).ok())
        .unwrap_or_else(|| build_fallback_content(doc));
    let origin = record
        .source_origin_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<crate::prompt_origin::DeliveryOrigin>(raw).ok());
    let (restored_id, _, _, mut commit) = state
        .hub
        .stage_atomic_draft_restore(
            &request.session_id,
            doc.body.clone(),
            content,
            cmid.clone(),
            origin,
            record.source_is_recovery,
        )
        .map_err(|msg| (StatusCode::CONFLICT, msg))?;
    if let Err(err) = await_store_barrier(&state).await {
        state.hub.abort_durable_draft_transfer(&mut commit);
        return Err(err);
    }
    let (next_queue, next_drafts) = match state.hub.snapshot_for_staged_transfer(&commit) {
        Ok(pair) => pair,
        Err(msg) => {
            state.hub.abort_durable_draft_transfer(&mut commit);
            return Err((StatusCode::CONFLICT, msg.to_owned()));
        }
    };
    match store
        .atomic_restore_session_draft(AtomicRestoreDraftArgs {
            owner: &owner,
            session_id: &request.session_id,
            draft_id: &draft_id,
            operation_id: &request.operation_id,
            expected_revision: request.expected_revision,
            expected_trashed_at_ms: request.expected_trashed_at_ms,
            restored_row_id: &restored_id,
            next_queue: &next_queue,
            next_drafts: &next_drafts,
        })
        .await
    {
        Ok(AtomicRestoreOutcome::Committed { purged_revision }) => {
            state.hub.commit_durable_draft_transfer(&mut commit);
            state
                .hub
                .announce_draft_purged(&owner, &[(draft_id.clone(), purged_revision)]);
            Ok(
                serde_json::json!({ "session_id": request.session_id, "restored_draft_id": restored_id, "restored_cmid": cmid, "purged_draft_id": draft_id }),
            )
        }
        Ok(AtomicRestoreOutcome::AlreadyRestored { restored_row_id }) => {
            state.hub.abort_durable_draft_transfer(&mut commit);
            Ok(
                serde_json::json!({ "session_id": request.session_id, "restored_draft_id": restored_row_id, "restored_cmid": cmid, "purged_draft_id": draft_id }),
            )
        }
        Ok(AtomicRestoreOutcome::Purged) => {
            state.hub.abort_durable_draft_transfer(&mut commit);
            Err((
                StatusCode::GONE,
                "This draft was permanently deleted".to_owned(),
            ))
        }
        Ok(AtomicRestoreOutcome::NotFound) => {
            state.hub.abort_durable_draft_transfer(&mut commit);
            Err((StatusCode::NOT_FOUND, "Draft not found".to_owned()))
        }
        Ok(AtomicRestoreOutcome::Conflict(msg)) => {
            state.hub.abort_durable_draft_transfer(&mut commit);
            Err((StatusCode::CONFLICT, msg))
        }
        Err(e) => {
            tracing::error!(error = %e, "atomic restore session draft");
            state.hub.abort_durable_draft_transfer(&mut commit);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not restore draft to session".to_owned(),
            ))
        }
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
    let Some(store) = state.store.clone() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable storage is unavailable",
        );
    };
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
    let (tx, rx) = tokio::sync::oneshot::channel();
    let owner = auth.principal.user_id.clone();
    tokio::spawn(async move {
        let res = run_restore_to_session(state, store, owner, draft_id, request).await;
        let _ = tx.send(res);
    });
    match rx.await {
        Ok(Ok(body)) => ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response(),
        Ok(Err((status, msg))) => error(status, &msg),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Draft restore task ended unexpectedly",
        ),
    }
}
