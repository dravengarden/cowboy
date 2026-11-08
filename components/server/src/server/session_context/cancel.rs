//! Take a continuation back while its first message still waits in its queue:
//! the message returns to the caller, the original comes back from Trash and
//! the unused destination goes there instead. See `docs/session-context.md`.
use super::*;

#[derive(Debug, Deserialize)]
pub(in crate::server) struct CancelRequest {
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

async fn take_back(
    state: Arc<AppState>,
    auth: AuthenticatedProductRequest,
    id: String,
    request: CancelRequest,
) -> Result<Cancelled, String> {
    current_authority(&state, &auth)?;
    let destination = owned(&state, &auth.principal, &id)?.meta;
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
    let (text, content) = state.hub.take_sole_queued(&id, &cmid).ok_or(
        "The new agent already has this message; stop it instead, or remove other queued messages first",
    )?;
    if state.hub.session_is_trashed(&source)
        && let Err(error) = super::super::session_trash::restore(
            &state,
            &auth.principal,
            &[],
            std::slice::from_ref(&source),
        )
    {
        state.hub.requeue_prompt(&id, text, content, Some(cmid));
        return Err(error);
    }
    // The work did not move: the destination stops naming the original, so
    // restoring it later cannot turn the original into history again.
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
    // The message is already back in the caller's hands, so a failed receipt
    // update must not lose it. The receipt only stops naming the destination.
    for (session, context) in [(&id, &mut target), (&source, &mut context)] {
        if let Some(delivery) = context.delivery.as_mut() {
            delivery.state = CANCELLED.into();
            delivery.source_trashed = false;
        }
        if session == &id {
            context.injection = None;
        }
        let saved = match advance_applied(context) {
            Ok(()) => save(&state, session, context).await,
            Err(error) => Err(error),
        };
        if let Err(reason) = saved {
            tracing::warn!(%session, %reason, "cancelled handoff receipt not recorded");
        }
    }
    Ok(Cancelled {
        session: state.hub.session_info(&source).map(|info| info.meta),
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

pub(in crate::server) async fn cancel(
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
