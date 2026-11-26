//! Take a continuation back while its first message still waits in its queue:
//! the message returns to the caller, the original comes back from Trash and
//! the unused destination goes there instead. See `docs/session-context.md`.
use super::*;

#[derive(Debug, Deserialize)]
pub struct CancelRequest {
    pub operation: String,
}

#[derive(Debug, Serialize)]
struct Cancelled {
    source_session_id: String,
    text: String,
    content: Vec<serde_json::Value>,
    session: Option<crate::core::SessionMeta>,
    /// The destination stayed out of Trash; it holds no message.
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

const CANCELLED: &str = "cancelled";

/// Shared eligibility predicate for whether `meta` (live OR in Trash!) can
/// still invoke Cancel Switch to return its sole bootstrap prompt to `source_id`.
pub(crate) fn is_cancellable_handoff_destination(
    state: &AppState,
    source_id: &str,
    meta: &crate::core::SessionMeta,
) -> bool {
    if meta.closing || meta.continued_from.as_deref() != Some(source_id) {
        return false;
    }
    let Ok(ctx) = crate::session_context::read(&state.hub, &meta.id) else {
        return false;
    };
    let accepted = ctx
        .delivery
        .as_ref()
        .is_some_and(|delivery| delivery.state == "accepted" && delivery.session_id == meta.id);
    if !accepted {
        return meta.status == crate::agent_model::Status::Starting
            && ctx
                .delivery
                .as_ref()
                .is_some_and(|d| d.state == "preparing");
    }
    let Some(cmid) = ctx.injection.as_ref().map(|inj| inj.cmid.as_str()) else {
        return false;
    };
    state
        .hub
        .session_info(&meta.id)
        .is_some_and(|info| info.queue_count == 1)
        && state.hub.has_queued_cmid(&meta.id, cmid)
}

fn replayed(state: &AppState, returned: &crate::session_context::Returned) -> Cancelled {
    Cancelled {
        session: state
            .hub
            .session_info(&returned.source_session_id)
            .map(|info| info.meta.for_client()),
        source_session_id: returned.source_session_id.clone(),
        text: returned.text.clone(),
        content: returned.content.clone(),
        note: None,
    }
}

fn replay_cancelled(
    state: &AppState,
    target: &crate::session_context::Context,
    operation: &str,
) -> Option<Cancelled> {
    target
        .delivery
        .as_ref()
        .filter(|delivery| delivery.operation == operation && delivery.state == CANCELLED)
        .and_then(|delivery| delivery.returned.as_ref())
        .map(|returned| replayed(state, returned))
}

fn restore_source_if_trashed(
    state: &Arc<AppState>,
    principal: &ProductPrincipal,
    source: &str,
) -> Result<(), String> {
    if state.hub.session_is_trashed(source) {
        super::super::session_trash::restore(state, principal, &[], &[source.to_owned()])?;
    }
    Ok(())
}

async fn take_back(
    state: Arc<AppState>,
    auth: AuthenticatedProductRequest,
    id: String,
    request: CancelRequest,
) -> Result<Cancelled, String> {
    current_authority(&state, &auth)?;
    let destination = owned(&state, &auth.principal, &id)?.meta;
    // A lost reply is replayed from the destination's receipt, which keeps
    // the message once it left the queue.
    if let Some(cancelled) = replay_cancelled(
        &state,
        &crate::session_context::read(&state.hub, &id)?,
        &request.operation,
    ) {
        return Ok(cancelled);
    }
    let source = destination
        .continued_from
        .ok_or("This session did not continue another one")?;
    // Same order as a send: source first, then its destination.
    let source_lock = write_lock(&source);
    let _source_guard = source_lock.lock().await;
    let lock = write_lock(&id);
    let _guard = lock.lock().await;
    owned(&state, &auth.principal, &source)?;
    let mut target = crate::session_context::read(&state.hub, &id)?;
    if let Some(cancelled) = replay_cancelled(&state, &target, &request.operation) {
        return Ok(cancelled);
    }
    let cmid = target
        .delivery
        .as_ref()
        .filter(|delivery| {
            delivery.operation == request.operation
                && delivery.session_id == id
                && delivery.state == "accepted"
        })
        .and(target.injection.as_ref())
        .map(|injection| injection.cmid.clone())
        .ok_or("This handoff can no longer be taken back")?;
    let mut context = crate::session_context::read(&state.hub, &source)?;
    if !context
        .delivery
        .as_ref()
        .is_some_and(|delivery| delivery.operation == request.operation)
    {
        return Err("The original session has moved on since this handoff".into());
    }
    let refused = "The new agent already has this message; stop it instead, or remove other queued messages first";
    if state
        .hub
        .session_info(&id)
        .is_none_or(|info| info.queue_count != 1)
        || !state.hub.has_queued_cmid(&id, &cmid)
    {
        return Err(refused.into());
    }
    restore_source_if_trashed(&state, &auth.principal, &source)?;
    // Flush any queued bootstrap UpdatePending to durable storage before
    // taking the pending-commit fence and CAS-committing cancellation.
    lineage::flush_store_barrier(&state).await?;
    // Fence dispatch and write-behind queue flushes without emptying the queue
    // yet, then CAS-commit empty queue, cancelled receipt, and lineage edge
    // deletion in one database transaction.
    let (taken, expected_queue, cancel_guard) =
        state.hub.begin_cancel_handoff(&id, &cmid).ok_or(refused)?;
    let (text, content) = (taken.text, taken.content);
    if let Some(delivery) = target.delivery.as_mut() {
        delivery.state = CANCELLED.into();
        delivery.returned = Some(crate::session_context::Returned {
            source_session_id: source.clone(),
            text: text.clone(),
            content: content.clone(),
        });
    }
    target.injection = None;
    advance_applied(&mut target)?;
    let target_key = crate::session_context::key(&id);
    let target_value = serde_json::to_value(&target).expect("session context serializes");
    if let Some(store) = state.store.as_ref() {
        let committed = store
            .commit_handoff_cancellation(&id, &expected_queue, &target_key, &target_value)
            .await
            .map_err(|error| format!("Could not cancel switch ({error})"))?;
        if !committed {
            return Err(refused.into());
        }
    }
    state
        .hub
        .commit_cancel_handoff_memory(&id, &cmid, target_key, target_value, cancel_guard);
    // The work did not move: the destination stops naming the original, so
    // restoring it later cannot turn the original into history again. Its
    // cancelled receipt keeps startup from recording the mark once more.
    state.hub.set_setting(
        format!(
            "{}{id}",
            crate::core::settings_keys::SESSION_CONTINUED_FROM_PREFIX
        ),
        serde_json::Value::Null,
    );
    state.hub.set_continued_from(&id, None);
    let note = super::super::session_trash::trash(
        &state,
        &auth.principal,
        &[],
        std::slice::from_ref(&id),
    )
    .err()
    .map(|reason| {
        tracing::info!(session = %id, %reason, "cancelled continuation stayed out of Trash");
        format!("The new session stayed open: {reason}")
    });
    if note.is_none() {
        release_launching_worker(&state, &id);
    }
    // Best effort: the destination's receipt already records the outcome.
    if let Some(delivery) = context.delivery.as_mut() {
        delivery.state = CANCELLED.into();
        delivery.source_trashed = false;
    }
    let saved = match advance_applied(&mut context) {
        Ok(()) => save(&state, &source, &context).await,
        Err(error) => Err(error),
    };
    if let Err(reason) = saved {
        tracing::warn!(session = %source, %reason, "cancelled handoff source receipt not recorded");
    }
    Ok(Cancelled {
        session: state
            .hub
            .session_info(&source)
            .map(|info| info.meta.for_client()),
        source_session_id: source,
        text,
        content,
        note,
    })
}

/// Trash hibernates an idle worker, but this one may still be launching for
/// the message just taken back. Release it once it is up; nothing can prompt
/// a trashed session meanwhile.
fn release_launching_worker(state: &Arc<AppState>, id: &str) {
    const POLL: std::time::Duration = std::time::Duration::from_secs(2);
    const ATTEMPTS: usize = 90;
    let state = Arc::downgrade(state);
    let id = id.to_owned();
    tokio::spawn(async move {
        for _ in 0..ATTEMPTS {
            tokio::time::sleep(POLL).await;
            let Some(state) = state.upgrade() else { return };
            let Some(status) = state.hub.status(&id) else {
                return;
            };
            if !state.hub.session_is_trashed(&id)
                || matches!(
                    status,
                    crate::agent_model::Status::Exited
                        | crate::agent_model::Status::Crashed
                        | crate::agent_model::Status::Interrupted
                )
            {
                return;
            }
            if status == crate::agent_model::Status::Running
                && state.supervisor.has_live_worker(&id)
            {
                if let Err(reason) = state.supervisor.hibernate_session(&id) {
                    tracing::info!(session = %id, %reason, "cancelled continuation kept its worker");
                }
                return;
            }
        }
    });
}

pub async fn cancel(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<String>,
    Json(request): Json<CancelRequest>,
) -> Response {
    match tokio::spawn(take_back(state, auth, id, request)).await {
        Ok(Ok(cancelled)) => (StatusCode::OK, Json(cancelled)).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => failure("Cancel outcome is unknown; check both sessions before retrying".into()),
    }
}
