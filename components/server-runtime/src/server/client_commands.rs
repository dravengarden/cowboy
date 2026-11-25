//! The Inbound command steps that handle_command dispatches to: admission
//! refusal, field extraction, and the larger per-command bodies.

use super::*;
use std::ops::ControlFlow;

/// Refuse a command for a missing, foreign, view-only, trashed or
/// context-pending session, reporting why. Returns whether it was refused.
pub fn refuse_command(
    state: &Arc<AppState>,
    principal: &ProductPrincipal,
    cmd: &Inbound,
    session_id_for_err: &Option<String>,
    authored_cmid: &Option<String>,
) -> bool {
    if let Some(cmid) = authored_cmid
        .as_deref()
        .filter(|id| crate::agent_model::is_reserved_call_notice_cmid(id))
    {
        let message = "reserved completion notice message id is not allowed";
        if let Some(sid) = session_id_for_err {
            state
                .hub
                .command_result(sid, cmid, CommandOutcome::Rejected, message);
        } else {
            state.hub.broadcast_error(None, message.to_owned());
        }
        return true;
    }
    if let (Some(sid), Some(cmid)) = (&session_id_for_err, &authored_cmid)
        && state.hub.session_info(sid).is_none()
    {
        state.hub.command_result(
            sid,
            cmid,
            CommandOutcome::NotFound,
            "this session no longer exists",
        );
        return true;
    }
    if let Some(sid) = &session_id_for_err {
        let owner = state
            .hub
            .session_info(sid)
            .and_then(|info| info.meta.owner_user_id);
        if !principal.can_mutate(owner.as_deref()) {
            state.hub.broadcast_error(
                Some(sid.clone()),
                "not allowed to mutate this session".to_owned(),
            );
            if let Some(cmid) = &authored_cmid {
                state.hub.command_result(
                    sid,
                    cmid,
                    CommandOutcome::Rejected,
                    "not allowed to mutate this session",
                );
            }
            return true;
        }
    }
    // A view-only system session rejects user-driven turns; only the backend
    // wake endpoint (POST /api/sessions/{id}/prompt) drives it.
    if let Inbound::MoveDraft {
        session_id,
        to_session,
        ..
    } = cmd
        && refuse_move_draft_target(state, principal, session_id, to_session, authored_cmid)
    {
        return true;
    }
    if let Some(sid) = &session_id_for_err
        && matches!(&cmd, Inbound::Prompt { .. } | Inbound::Submit { .. })
        && state.hub.session_is_system(sid)
    {
        state.hub.broadcast_error(
            Some(sid.clone()),
            "view-only system session: input is disabled".to_owned(),
        );
        if let Some(cmid) = &authored_cmid {
            state.hub.command_result(
                sid,
                cmid,
                CommandOutcome::Rejected,
                "view-only system session: input is disabled",
            );
        }
        return true;
    }
    if let Some(sid) = &session_id_for_err
        && !allowed_on_trashed_session(cmd)
        && state.hub.session_is_trashed(sid)
    {
        if let Some(cmid) = &authored_cmid {
            state.hub.command_result(
                sid,
                cmid,
                CommandOutcome::Rejected,
                session_trash::TRASHED_REFUSAL,
            );
        } else {
            state
                .hub
                .broadcast_error(Some(sid.clone()), session_trash::TRASHED_REFUSAL.into());
        }
        return true;
    }
    if let Some(sid) = &session_id_for_err
        && matches!(
            cmd,
            Inbound::Prompt { .. }
                | Inbound::Submit { .. }
                | Inbound::ActivateDraft { .. }
                | Inbound::ActivateAllDrafts { .. }
                | Inbound::RequestSendQueued { .. }
                | Inbound::ForcePushQueued { .. }
        )
        && crate::session_context::pending(&state.hub, sid)
    {
        let message = "Context changes are pending. Send from the Composer with the selected context, or cancel the switch.";
        if let Some(cmid) = &authored_cmid {
            state
                .hub
                .command_result(sid, cmid, CommandOutcome::Rejected, message);
        } else {
            state.hub.broadcast_error(Some(sid.clone()), message.into());
        }
        return true;
    }
    false
}

/// The client message id of an authored command, if it carries one.
pub fn command_authored_cmid(cmd: &Inbound) -> Option<String> {
    match cmd {
        Inbound::Submit { cmid, .. }
        | Inbound::AddDraft { cmid, .. }
        | Inbound::ActivateDraft { cmid, .. }
        | Inbound::RequestSendQueued { cmid, .. }
        | Inbound::ForcePushQueued { cmid, .. }
        | Inbound::ScheduleDraft { cmid, .. }
        | Inbound::EditQueued { cmid, .. }
        | Inbound::RemoveQueued { cmid, .. }
        | Inbound::EditDraft { cmid, .. }
        | Inbound::RemoveDraft { cmid, .. } => cmid.clone(),
        _ => None,
    }
}

/// The session a command addresses, for refusal and error attribution.
pub fn command_session_id(cmd: &Inbound) -> Option<String> {
    match cmd {
        Inbound::Prompt { session_id, .. }
        | Inbound::Cancel { session_id }
        | Inbound::CancelSubagent { session_id, .. }
        | Inbound::CancelSubmitted { session_id, .. }
        | Inbound::Permission { session_id, .. }
        | Inbound::DeleteSession { session_id }
        | Inbound::RenameSession { session_id, .. }
        | Inbound::SetSessionAutoResume { session_id, .. }
        | Inbound::SetPaused { session_id, .. }
        | Inbound::ResumeTurn { session_id }
        | Inbound::RetryTurn { session_id }
        | Inbound::SetConfigOption { session_id, .. }
        | Inbound::OpenSession { session_id }
        | Inbound::ResetSession { session_id }
        | Inbound::Submit { session_id, .. }
        | Inbound::RemoveQueued { session_id, .. }
        | Inbound::EditQueued { session_id, .. }
        | Inbound::ClearQueue { session_id }
        | Inbound::RequestSendQueued { session_id, .. }
        | Inbound::ForcePushQueued { session_id, .. }
        | Inbound::QueuedToDraft { session_id, .. }
        | Inbound::SetQueueEditing { session_id, .. }
        | Inbound::AddDraft { session_id, .. }
        | Inbound::EditDraft { session_id, .. }
        | Inbound::RemoveDraft { session_id, .. }
        | Inbound::ClearDrafts { session_id }
        | Inbound::ActivateDraft { session_id, .. }
        | Inbound::ActivateAllDrafts { session_id }
        | Inbound::MoveDraft { session_id, .. }
        | Inbound::ScheduleDraft { session_id, .. }
        | Inbound::UnscheduleDraft { session_id, .. }
        | Inbound::ReorderQueue { session_id, .. }
        | Inbound::ReorderDrafts { session_id, .. } => Some(session_id.clone()),
        // Sync mutations are state-scoped (title/order), not session-scoped — a
        // failure surfaces as a daemon-level error (None).
        Inbound::AuthActivity
        | Inbound::ConnectionProbe { .. }
        | Inbound::NewSession { .. }
        | Inbound::ReorderSessions { .. }
        | Inbound::Sync { .. }
        | Inbound::TrashItems { .. }
        | Inbound::RestoreItems { .. }
        | Inbound::PurgeItems { .. }
        | Inbound::SetSetting { .. } => None,
    }
}

/// Take or release this connection's queue edit hold.
#[allow(clippy::too_many_arguments)]
pub fn set_queue_editing_command(
    state: &Arc<AppState>,
    held: &mut HashMap<String, (String, u64)>,
    session_id: String,
    id: Option<String>,
    token: Option<String>,
) -> Result<(), String> {
    // Track the hold per-connection so a mid-edit disconnect releases it
    // (the hold is global server state — see handle_ws teardown).
    match &id {
        Some(mid) => {
            let epoch =
                state
                    .hub
                    .set_queue_editing_with_token(&session_id, Some(mid.clone()), token);
            held.insert(session_id.clone(), (mid.clone(), epoch));
        }
        None => {
            // A stale socket can deliver its final release after a
            // replacement socket has already renewed the edit hold.
            // Release only the epoch owned by THIS connection; the
            // replacement's newer epoch must keep the queue pinned.
            if let Some((mid, epoch)) = held.remove(&session_id) {
                state
                    .hub
                    .release_queue_editing_if_epoch(&session_id, &mid, epoch);
            }
        }
    }
    Ok(())
}

/// Queue an authored prompt: plain, jump to the front, or force-push.
#[allow(clippy::too_many_arguments)]
fn submit_wrap_up_command(
    state: &Arc<AppState>,
    caller: &crate::prompt_origin::RequestCaller,
    session_id: &str,
    text: String,
    content: Vec<serde_json::Value>,
    cmid: Option<String>,
    force: bool,
    wrap_up_target: Option<&crate::send_delivery::DeliveryTarget>,
) -> ControlFlow<bool, Result<(), String>> {
    let reject_wrap_up = |cmid_ref: Option<&str>, message: &str| {
        if let Some(id) = cmid_ref.filter(|id| !id.is_empty()) {
            state
                .hub
                .command_result(session_id, id, CommandOutcome::Rejected, message);
        }
        ControlFlow::Continue(Err(message.to_owned()))
    };
    if force {
        return reject_wrap_up(cmid.as_deref(), "wrap_up conflicts with force");
    }
    if !caller.is_person() {
        return reject_wrap_up(
            cmid.as_deref(),
            "wrap_up delivery is restricted to the session owner",
        );
    }
    if !cmid.as_deref().is_some_and(|id| {
        crate::wrap_up::is_valid_wrap_up_operation_id(id) && crate::core::Hub::is_authored_cmid(id)
    }) {
        return reject_wrap_up(
            cmid.as_deref(),
            "wrap_up requires a valid authored client message id",
        );
    }
    let Some(built_blocks) = build_prompt_blocks(&text, &content) else {
        return reject_wrap_up(
            cmid.as_deref(),
            "empty prompt: no substantive text or content",
        );
    };
    let json_blocks: Vec<serde_json::Value> = built_blocks
        .iter()
        .map(|b| serde_json::to_value(b).unwrap_or_default())
        .collect();
    if !crate::wrap_up::has_substantive_prompt_blocks(&json_blocks) {
        return reject_wrap_up(
            cmid.as_deref(),
            "empty prompt: no substantive text or content",
        );
    }
    let cmid_for_reject = cmid.clone();
    match state
        .hub
        .front_submit_with_outcome(session_id, text, content, cmid, caller)
    {
        crate::core::FrontSubmitOutcome::Refused => reject_wrap_up(
            cmid_for_reject.as_deref(),
            "session unavailable for wrap_up delivery",
        ),
        crate::core::FrontSubmitOutcome::Duplicate => ControlFlow::Break(true),
        crate::core::FrontSubmitOutcome::DispatchedImmediately
        | crate::core::FrontSubmitOutcome::QueuedFallback => ControlFlow::Continue(Ok(())),
        crate::core::FrontSubmitOutcome::QueuedFront => {
            let _ = send_delivery::try_wrap_up_hint(state, session_id, wrap_up_target);
            ControlFlow::Continue(Ok(()))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn submit_command(
    state: &Arc<AppState>,
    principal: &ProductPrincipal,
    caller: &crate::prompt_origin::RequestCaller,
    command_trace: Option<&crate::runtime_trace::SpanTimer>,
    session_id: String,
    text: String,
    content: Vec<serde_json::Value>,
    cmid: Option<String>,
    force: bool,
    front: bool,
    wrap_up: bool,
    wrap_up_target: Option<crate::send_delivery::DeliveryTarget>,
) -> ControlFlow<bool, Result<(), String>> {
    ControlFlow::Continue({
        // Only an authorized, exact session can create runtime correlation
        // authority. The ephemeral key never enters durable queue rows.
        if let (Some(trace), Some(cmid), Some(info)) = (
            command_trace,
            cmid.as_deref(),
            state.hub.session_info(&session_id),
        ) {
            state.observability.bind_runtime_trace(
                &principal.user_id,
                &info.meta.machine_id,
                &session_id,
                cmid,
                trace,
            );
        }
        if wrap_up {
            return submit_wrap_up_command(
                state,
                caller,
                &session_id,
                text,
                content,
                cmid,
                force,
                wrap_up_target.as_ref(),
            );
        }
        if force
            && let Some(refusal) =
                provider_accounts::retirement::session_refusal(&state.accounts, &session_id)
        {
            // Nothing queued, nothing cancelled: a retired session only parks.
            return ControlFlow::Continue(Err(refusal.to_owned()));
        }
        if force {
            // Long-press send: jump to the front of the queue and interrupt the
            // running turn so it runs next (same end-state as a queued row's
            // force-push). force_submit returns true when it queued (vs a direct
            // idle dispatch); only then, and only on a live turn, do we Cancel.
            let queued =
                state
                    .hub
                    .force_submit_once(&session_id, text, content, cmid, true, caller);
            if queued.is_none() {
                return ControlFlow::Break(true);
            }
            if queued == Some(true)
                && matches!(
                    state.hub.status(&session_id),
                    Some(Status::Busy | Status::Starting)
                )
            {
                force_cancel_with_watchdog(state, &session_id)
            } else {
                // Not busy: force_submit front-inserted the prompt but nothing
                // dispatched it — a PAUSED queue HOLDS the auto-drain. A
                // force-push is an explicit "send this now", so
                // drain the head MANUALLY here: bypass the hold and run it now,
                // WITHOUT resuming the rest of the held queue. No-op when the
                // head already dispatched (idle + empty queue).
                state.hub.drain_now(&session_id, caller);
                Ok(())
            }
        } else if front {
            // "Jump to front" without interrupting: front-insert so it runs next
            // after the current turn, ahead of the rest of the queue. Same
            // front placement as force, but no Cancel.
            let _ = state
                .hub
                .force_submit(&session_id, text, content, cmid, false, caller);
            Ok(())
        } else {
            state.hub.submit(&session_id, text, content, cmid, caller);
            Ok(())
        }
    })
}

/// Clear the conversation, through execution recovery when the session is
/// bound to a live execution.
#[allow(clippy::too_many_arguments)]
pub fn reset_session_command(
    state: &Arc<AppState>,
    session_id: String,
) -> ControlFlow<bool, Result<(), String>> {
    // Before either path clears anything: a retired card's session could
    // never start again, so its history stays and the guidance is returned.
    if let Some(refusal) =
        provider_accounts::retirement::session_refusal(&state.accounts, &session_id)
    {
        return ControlFlow::Continue(Err(refusal.to_owned()));
    }
    {
        if state.hub.session_info(&session_id).is_some_and(|info| {
            info.meta.execution_binding.is_some()
                && !(info.meta.status == Status::Exited
                    && !state
                        .runtime_router
                        .runtime(&info.meta.machine_id)
                        .is_some_and(|runtime| runtime.has_worker(&session_id)))
        }) {
            let state = Arc::clone(state);
            let revision = state.hub.status_revision(&session_id);
            let in_flight = state.hub.session_has_in_flight_prompt(&session_id);
            let turn = state.hub.in_flight_prompt_epoch(&session_id);
            let mut shutdown = state.shutdown.clone();
            tokio::spawn(async move {
                let result = dispatch_preflight(
                    &mut shutdown,
                    execution::recovery::before_reset(&state, &session_id),
                )
                .await;
                match result {
                        Ok(recovery) if state.hub.status_revision(&session_id) == revision
                            && state.hub.session_has_in_flight_prompt(&session_id) == in_flight
                            && state.hub.in_flight_prompt_epoch(&session_id) == turn
                            && !state.hub.session_is_trashed(&session_id) => {
                            if let Err(error) = clear_conversation(&state, &session_id).await {
                                state.hub.broadcast_error(Some(session_id), error);
                                return;
                            }
                            let result = match recovery {
                                Some(intent) => state.supervisor.recover_execution_session(&session_id, intent),
                                None => state.supervisor.reset_session(&session_id),
                            };
                            state.hub.end_clear_conversation(&session_id);
                            if let Err(error) = result {
                                state.hub.broadcast_error(Some(session_id), error);
                            }
                        }
                        Ok(_) => state.hub.broadcast_error(Some(session_id),
                            "Session changed while preparing Clear; retry after its current operation finishes".into()),
                        Err(error) => state.hub.broadcast_error(Some(session_id), error),
                    }
            });
            return ControlFlow::Break(true);
        }
        // "Clear conversation" (see Inbound::ResetSession). Order matters:
        // 1. Durably delete the transcript and forget the resumable agent id
        //    with its restart fence, in one acknowledged transaction; a failure
        //    leaves the session and its worker untouched.
        // 2. Apply it to the Hub (fresh context, divider marker), so the
        //    respawn does a FRESH session/new instead of session/load.
        // 3. Atomically fence + replace the worker. This must not use the
        //    permanent delete path: the Machine broker retains delete tombstones to
        //    reject stale launches for genuinely deleted sessions.
        let state = Arc::clone(state);
        tokio::spawn(async move {
            if let Err(error) = clear_conversation(&state, &session_id).await {
                state.hub.broadcast_error(Some(session_id), error);
                return;
            }
            if let Err(e) = state.supervisor.reset_session(&session_id) {
                tracing::warn!(session_id = %session_id, error = %e, "reset: respawn failed");
            }
            state.hub.end_clear_conversation(&session_id);
        });
        ControlFlow::Break(true)
    }
}

/// Commit a user Clear durably, then apply it to the Hub. Nothing changes when
/// the store refuses or fails the combined write.
async fn clear_conversation(state: &AppState, session_id: &str) -> Result<(), String> {
    let commit = match state.hub.begin_clear_conversation(session_id)? {
        None => None,
        Some((ack, commit)) => match ack.await {
            Ok(Ok(_)) => Some(commit),
            failed => {
                state.hub.end_clear_conversation(session_id);
                return Err(match failed {
                    Ok(Err(error)) => format!("Clear was not saved: {error}"),
                    _ => "Clear was not saved: the store writer stopped".to_owned(),
                });
            }
        },
    };
    crate::subagent_transcripts::fence(session_id);
    state.hub.finish_clear_conversation(session_id, commit);
    Ok(())
}

/// Open a session, reviving it unless its last turn was interrupted or it
/// failed with a terminal Provider error.
#[allow(clippy::too_many_arguments)]
pub fn open_session_command(state: &Arc<AppState>, session_id: String) -> Result<(), String> {
    // Do NOT revive an INTERRUPTED session on open. Reviving would flip it
    // to Starting→Running, hiding the fact that its last turn was cut off
    // by a restart — and a stale in-flight tool from that turn would then
    // drive a misleading "working" spinner (the reported bug: after a
    // deploy + reload it looked like the agent was still thinking). Left
    // interrupted, the client shows the "last turn was interrupted — send
    // a message to start a new one" bar and no spinner; submitting a
    // message revives it via the drain. Exited sessions also stay asleep.
    let meta = state
        .hub
        .session_list()
        .into_iter()
        .find(|meta| meta.id == session_id);
    let terminal_provider_error = meta.as_ref().is_some_and(|meta| {
        meta.status == Status::Crashed
            && state
                .hub
                .latest_crash_detail(&session_id)
                .as_deref()
                .is_some_and(|detail| {
                    let behavior = meta
                        .provider_behavior
                        .clone()
                        .unwrap_or_else(|| crate::provider::legacy_behavior(&meta.provider));
                    behavior
                        .matching_error_rule(detail)
                        .is_some_and(|rule| rule.user_detail.is_some())
                })
    });
    if terminal_provider_error || state.hub.status(&session_id) == Some(Status::Interrupted) {
        // Interrupted sessions remain stopped until the user submits a
        // message or explicitly sends queued work.
        if terminal_provider_error {
            tracing::info!(
                session_id = %session_id,
                "not auto-reviving session with terminal provider startup error"
            );
        }
        Ok(())
    } else {
        // A client opens the focus it restored from localStorage on
        // reload. If that session is gone (deleted while the client was
        // away), this is NOT an error condition: the client already pops a
        // one-shot *warning* snackbar and falls back to another session.
        // Log a server-side warning and swallow the error so no error
        // toast is broadcast (which would otherwise read as a hard
        // failure).
        match state.supervisor.open(&session_id) {
            Ok(revived) => {
                if revived {
                    tokio::spawn(session_reclaim::rebalance_after_wake(
                        Arc::clone(state),
                        session_id.clone(),
                    ));
                } else if let Some(meta) = meta.as_ref() {
                    session_provider_updates::request_on_open(state, meta);
                }
                Ok(())
            }
            Err(e) => {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "open of unknown/gone session ignored — client will fall back",
                );
                Ok(())
            }
        }
    }
}

/// A direct API prompt: auto-title, block parsing and dispatch.
#[allow(clippy::too_many_arguments)]
pub fn prompt_command(
    state: &Arc<AppState>,
    caller: &crate::prompt_origin::RequestCaller,
    session_id: String,
    text: String,
    content: Vec<serde_json::Value>,
) -> ControlFlow<bool, Result<(), String>> {
    // Never wake or feed a worker of a retired legacy card.
    if let Some(refusal) =
        provider_accounts::retirement::session_refusal(&state.accounts, &session_id)
    {
        return ControlFlow::Continue(Err(refusal.to_owned()));
    }
    ControlFlow::Continue({
        // Derive an auto-title from the first prompt before text/content are
        // consumed below. auto_title no-ops unless the title is still the
        // cwd default, so this only "takes" on a session's first prompt and
        // never overrides a manual rename.
        let auto = first_prompt_title(&text, &content);
        if !(text.is_empty() && content.is_empty()) {
            config_revert::clear(&state.hub, &session_id);
        }
        if content.is_empty() && text.is_empty() {
            tracing::warn!("Prompt with neither text nor content; dropping");
            state.hub.broadcast_error(
                Some(session_id),
                "empty prompt: no text or content blocks".to_owned(),
            );
            return ControlFlow::Break(false);
        }
        if state.hub.has_dispatch_tx() {
            // Route through Hub's single-in-flight queue guard so a legacy
            // WebSocket `Prompt` can never race with a draining completion notice.
            state.hub.submit(&session_id, text, content, None, caller);
            return ControlFlow::Break(true);
        }
        let blocks: Vec<ContentBlock> = if content.is_empty() {
            vec![ContentBlock::from(text)]
        } else {
            content
                .into_iter()
                .filter_map(|v| match serde_json::from_value::<ContentBlock>(v) {
                    Ok(b) => Some(b),
                    Err(e) => {
                        tracing::warn!(error = %e, "skipping unparseable Prompt content block");
                        None
                    }
                })
                .collect()
        };
        let cmid = format!("cowboy-dispatch:{}", uuid::Uuid::new_v4().simple());
        let origin = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        state.hub.record_prompt_dispatch(
            &session_id,
            cmid.clone(),
            origin.clone(),
            origin,
            chrono::Utc::now().timestamp_millis(),
        );
        let result = state
            .supervisor
            .send(&session_id, AgentCommand::Prompt(blocks, Some(cmid), None));
        if result.is_ok()
            && let Some(title) = auto
        {
            state.hub.auto_title(&session_id, title);
        }
        result
    })
}

/// Derive a short session title from the first prompt: the first non-empty
/// line, whitespace-collapsed and truncated. Prefers the legacy `text` field;
/// falls back to the first text block in `content` (attachment prompts carry
/// their text there). Returns None for an attachment-only / empty prompt.
pub fn first_prompt_title(text: &str, content: &[serde_json::Value]) -> Option<String> {
    // Cap length on a char boundary so a long first line stays a label, not a
    // paragraph.
    const MAX: usize = 60;
    let raw = if text.trim().is_empty() {
        content.iter().find_map(|v| {
            if v.get("type").and_then(serde_json::Value::as_str) == Some("text") {
                v.get("text")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            } else {
                None
            }
        })?
    } else {
        text.to_owned()
    };
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    // Append an ellipsis when the first line is cut to MAX.
    if collapsed.chars().count() > MAX {
        let head: String = collapsed.chars().take(MAX).collect();
        Some(format!("{head}…"))
    } else {
        Some(collapsed)
    }
}

#[allow(clippy::too_many_lines)] // one cohesive command-dispatch match
/// Address a replayed edit or removal whose row already left the queue or
/// drafts. The session itself still exists, so the outcome is `stale` rather
/// than `not_found`; a command without a client id has no outbox entry to
/// retire and stays a silent no-op.
pub fn report_stale_row(state: &AppState, session_id: &str, cmid: Option<&str>) {
    if let Some(cmid) = cmid {
        state.hub.command_result(
            session_id,
            cmid,
            CommandOutcome::Stale,
            "this message no longer exists",
        );
    }
}

/// Address an edit blocked by an in-flight steer reservation on that row.
/// Keeps the row alive and reports `rejected` (never `stale`) so the client
/// retains the edit for retry instead of parking a duplicate draft.
pub fn report_reserved_row(state: &AppState, session_id: &str, cmid: Option<&str>) {
    if let Some(cmid) = cmid {
        state.hub.command_result(
            session_id,
            cmid,
            CommandOutcome::Rejected,
            "this message is currently being delivered; try again once delivery settles",
        );
    }
}

/// The folder tree a principal organizes: their own while product auth is on,
/// the shared unowned tree while it is off (mirroring session ownership).
pub fn folder_actor(state: &AppState, principal: &ProductPrincipal) -> FolderActor {
    FolderActor {
        user_id: state
            .product_auth_enabled
            .then(|| principal.user_id.clone()),
        sees_all: principal.sees_every_session(),
    }
}

pub fn apply_visible_reorder(
    state: &AppState,
    principal: &ProductPrincipal,
    order: &[String],
) -> Result<(), String> {
    if !principal.can_reorder() {
        return Err("viewers cannot reorder sessions".to_owned());
    }
    let submitted = order
        .iter()
        .map(|id| serde_json::Value::String(id.clone()))
        .collect::<Vec<_>>();
    let filtered = filter_submitted_order(state, principal, &submitted);
    state.hub.reorder_sessions(&filtered);
    Ok(())
}

pub fn filter_submitted_order(
    state: &AppState,
    principal: &ProductPrincipal,
    submitted: &[serde_json::Value],
) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    submitted
        .iter()
        .filter_map(serde_json::Value::as_str)
        .filter_map(|id| {
            let info = state.hub.session_info(id)?;
            principal
                .can_see(info.meta.owner_user_id.as_deref())
                .then(|| id.to_owned())
        })
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

pub fn force_cancel_grace(state: &AppState, session_id: &str) -> std::time::Duration {
    if state
        .hub
        .session_info(session_id)
        .is_some_and(|info| info.meta.execution_binding.is_some())
    {
        EXECUTION_FORCE_CANCEL_GRACE
    } else {
        FORCE_CANCEL_GRACE
    }
}

const HELD_SEND_REPLY: &str =
    "This session cannot send yet. The message stays first in the queue and sends once it can.";
const SEND_NOW_EXPIRED_REPLY: &str = "This Send now was made too long ago (or the device clock is off) to resend. The message stays in the queue; send it again.";
const SEND_NOW_HANDLED_REPLY: &str = "This Send now was already handled.";
const SEND_NOW_REFUSED_REPLY: &str =
    "This session cannot take another Send now right now. The message stays in the queue.";

/// A queued row's Send now. While the session's account parks dispatch (a
/// rate limit, a block, an unacknowledged effect) or its execution Machine is
/// not connected, the row only moves to the front and waits there, retried as
/// an explicit send once the account admits it and the Machine is back.
/// Dispatching it would take the row out of the queue just to requeue it under
/// a new id, which a client editing the row reads as a sent message and saves
/// the edit as a draft beside the returned original. The sending device gets
/// an addressed refusal for `cmid` once the hold is durable, so it retires its
/// optimistic send only when a restart can no longer lose it. A device that
/// lost that reply replays the command: it gets the reply again and sends
/// nothing, whether the wait still holds the row or already ended. A command
/// issued outside the window those receipts are kept for (`issued_ms`) is
/// refused without effect, so no replay can outlive its receipt. Whether
/// nothing more is to be done: the send was held, refused, or its row is gone.
pub fn request_send_queued(
    state: &Arc<AppState>,
    caller: &crate::prompt_origin::RequestCaller,
    session_id: &str,
    id: &str,
    cmid: Option<&str>,
    issued_ms: Option<i64>,
) -> bool {
    let reply = |message: &'static str| {
        if let Some(cmid) = cmid {
            state
                .hub
                .command_result(session_id, cmid, CommandOutcome::Rejected, message);
        }
    };
    if let Some(cmid) = cmid
        && let Some(waiting) = state.hub.held_send_replay(session_id, cmid)
    {
        reply(if waiting {
            HELD_SEND_REPLY
        } else {
            SEND_NOW_HANDLED_REPLY
        });
        return true;
    }
    if send_now_outside_window(issued_ms, chrono::Utc::now().timestamp_millis()) {
        reply(SEND_NOW_EXPIRED_REPLY);
        return true;
    }
    // Every command may end as a receipt: an oversized id or a full ledger
    // would make the ledger a write the store refuses.
    if cmid.is_some_and(|cmid| {
        cmid.len() > crate::core::MAX_HELD_COMMAND_BYTES || !state.hub.held_sends_admit(session_id)
    }) {
        reply(SEND_NOW_REFUSED_REPLY);
        return true;
    }
    let held = state.hub.session_info(session_id).and_then(|info| {
        provider_accounts::controls::parked_reason(&state.accounts, &info.meta).or_else(|| {
            (!session_maintenance::execution_target_connected(state, &info.meta))
                .then(|| session_maintenance::TARGET_WAIT_REASON.to_owned())
        })
    });
    let Some(reason) = held else {
        return !state
            .hub
            .request_send_queued_as(session_id, id, caller, cmid);
    };
    // The wait, the row's position and its intent are installed in one
    // serialized section: an automatic drain that sees the held row already
    // finds the session waiting, a completion cannot clear the wait in
    // between, a duplicate of this command is recognized there, and a request
    // for a row another terminal removed leaves the previous wait as it was.
    match state.hub.hold_queued(session_id, id, caller, cmid, &reason) {
        crate::core::HoldOutcome::Held => {
            // Command results reach every socket: the wait reason stays in
            // the session's own state, which only its viewers receive.
            if let Some(cmid) = cmid {
                let barrier = state.hub.store_barrier();
                let (hub, session, cmid) =
                    (state.hub.clone(), session_id.to_owned(), cmid.to_owned());
                tokio::spawn(async move {
                    if let Some(durable) = barrier {
                        let _ = durable.await;
                    }
                    hub.command_result(&session, &cmid, CommandOutcome::Rejected, HELD_SEND_REPLY);
                });
            }
        }
        crate::core::HoldOutcome::Replayed { waiting } => reply(if waiting {
            HELD_SEND_REPLY
        } else {
            SEND_NOW_HANDLED_REPLY
        }),
        crate::core::HoldOutcome::Missing => {}
    }
    true
}

/// Whether a Send now issued at `issued_ms` falls outside the window the
/// Controller keeps held-send receipts for: older than the replay horizon, or
/// stamped further ahead than a client clock may run. A command without a
/// time (from a client before it was sent) is never refused for it.
fn send_now_outside_window(issued_ms: Option<i64>, now_ms: i64) -> bool {
    issued_ms.is_some_and(|issued| {
        now_ms.saturating_sub(issued) > crate::core::HELD_SEND_REPLAY_HORIZON_MS
            || issued.saturating_sub(now_ms) > crate::core::HELD_SEND_CLOCK_SKEW_MS
    })
}

/// A queued row's force push: promote it and interrupt the running turn so
/// it runs next; on an idle session there's nothing to cancel, so it is just
/// sent now. A retired session's row stays where it is and no turn is
/// cancelled, and a row its account parks never cancels the running turn.
pub fn force_push_queued(
    state: &Arc<AppState>,
    caller: &crate::prompt_origin::RequestCaller,
    session_id: &str,
    id: &str,
    cmid: Option<&str>,
    issued_ms: Option<i64>,
) -> Result<(), String> {
    if let Some(refusal) =
        provider_accounts::retirement::session_refusal(&state.accounts, session_id)
    {
        return Err(refusal.to_owned());
    }
    // Promote to front either way.
    if request_send_queued(state, caller, session_id, id, cmid, issued_ms) {
        return Ok(());
    }
    if matches!(
        state.hub.status(session_id),
        Some(Status::Busy | Status::Starting)
    ) {
        force_cancel_with_watchdog(state, session_id)
    } else {
        Ok(())
    }
}

pub fn force_cancel_with_watchdog(state: &AppState, session_id: &str) -> Result<(), String> {
    // Never cancel a retired session's live turn for a send that will only
    // park; an explicit Stop takes its own path.
    if let Some(refusal) =
        provider_accounts::retirement::session_refusal(&state.accounts, session_id)
    {
        return Err(refusal.to_owned());
    }
    let Some(cancelled_revision @ (Status::Busy | Status::Starting, _)) =
        state.hub.status_revision(session_id)
    else {
        return Ok(());
    };
    let grace = force_cancel_grace(state, session_id);
    state.supervisor.request_cancel_for_recycle(session_id)?;
    let hub = state.hub.clone();
    let supervisor = Arc::clone(&state.supervisor);
    let session_id = session_id.to_owned();
    tokio::spawn(async move {
        tokio::time::sleep(grace).await;
        if !hub.set_status_if_revision(
            &session_id,
            Some(cancelled_revision),
            Status::Interrupted,
            Some("force cancel timed out; recycling session worker".to_owned()),
        ) {
            return;
        }
        tracing::error!(
            session = %session_id,
            grace_seconds = grace.as_secs(),
            "force cancel did not end turn; recycling only this session worker"
        );
        // The interrupted edge frees Hub's in-flight guard, but its automatic
        // drain waits for the replacement worker to become Running.
        if let Err(error) = supervisor.recycle_session(&session_id) {
            tracing::error!(session = %session_id, %error, "force-cancel recycle failed");
            hub.set_status(&session_id, Status::Crashed, Some(error));
        }
    });
    Ok(())
}

/// Build the ACP prompt blocks for a queued message: parse the stored content
/// blocks, or fall back to a single text block. Mirrors the `Inbound::Prompt`
/// handler's logic. Returns `None` for a genuinely empty prompt.
pub fn build_prompt_blocks(text: &str, content: &[serde_json::Value]) -> Option<Vec<ContentBlock>> {
    if content.is_empty() {
        if text.is_empty() {
            return None;
        }
        return Some(vec![ContentBlock::from(text.to_owned())]);
    }
    let blocks: Vec<ContentBlock> = content
        .iter()
        .filter_map(
            |v| match serde_json::from_value::<ContentBlock>(v.clone()) {
                Ok(b) => Some(b),
                Err(e) => {
                    tracing::warn!(error = %e, "skipping unparseable queued content block");
                    None
                }
            },
        )
        .collect();
    if blocks.is_empty() {
        None
    } else {
        Some(blocks)
    }
}

/// Combined admission refusal, durable retired-submit dedupe, durable recovery
/// activation, and durable notice-withdrawal pre-step for `handle_command`.
pub async fn prehandle_command(
    state: &Arc<AppState>,
    principal: &ProductPrincipal,
    caller: &crate::prompt_origin::RequestCaller,
    cmd: &Inbound,
    session_id_for_err: &Option<String>,
) -> Option<bool> {
    // A client-authored prompt or draft carries its `cmid` from a durable
    // outbox that is replayed after every reconnect. A refusal that can never
    // change must name that id, otherwise the device retries it forever and the
    // user only ever sees an unaddressed toast.
    let authored_cmid: Option<String> = command_authored_cmid(cmd);
    if refuse_command(state, principal, cmd, session_id_for_err, &authored_cmid) {
        return Some(false);
    }
    if let Inbound::Submit {
        session_id, cmid, ..
    } = cmd
        && let Some(done) = check_durably_retired_submit(state, session_id, cmid.as_deref()).await
    {
        return Some(done);
    }
    if let Some(done) = try_commit_recovery_draft_activation(state, caller, cmd).await {
        return Some(done);
    }
    try_commit_owner_notice_withdrawal(state, cmd, &authored_cmid).await
}

/// On an authored submit cache miss, consult durable `prompt_dispatches` so an
/// offline browser replaying a previously parked/retired operation ID receives
/// an addressed delivery confirmation instead of executing the retired prompt again.
pub async fn check_durably_retired_submit(
    state: &AppState,
    session_id: &str,
    cmid: Option<&str>,
) -> Option<bool> {
    let op = cmid.filter(|id| crate::core::Hub::is_authored_cmid(id))?;
    if state.hub.is_cmid_known_in_hub(session_id, op) {
        return None;
    }
    let store = state.store.as_ref()?;
    match store.prompt_dispatch(session_id, op).await {
        Ok(Some(row)) if row.undelivered_retired => {
            state.hub.confirm_durably_retired_cmid(session_id, op);
            Some(true)
        }
        Ok(_) => None,
        Err(error) => {
            tracing::warn!(%error, session = %session_id, "failed to check durable prompt retirement");
            let msg = "Could not verify prompt delivery state; retry shortly";
            state
                .hub
                .command_result(session_id, op, CommandOutcome::Rejected, msg);
            state
                .hub
                .broadcast_error(Some(session_id.to_owned()), msg.to_owned());
            Some(false)
        }
    }
}

/// Atomically reserve the fresh client operation ID, check durable history,
/// serialize against per-session pending commits (`pending_commit_lock`), and
/// durably commit a `recovery-*` draft activation before mutating Hub.
async fn try_commit_recovery_draft_activation(
    state: &Arc<AppState>,
    caller: &crate::prompt_origin::RequestCaller,
    cmd: &Inbound,
) -> Option<bool> {
    let Inbound::ActivateDraft {
        session_id,
        id,
        cmid,
        front,
    } = cmd
    else {
        return None;
    };
    if !id.starts_with("recovery-") {
        return None;
    }
    let Some(op) = cmid
        .as_deref()
        .filter(|op| crate::core::Hub::is_authored_cmid(op))
    else {
        let msg = "Resending a parked recovery draft requires a new client operation id";
        if let Some(raw) = cmid.as_deref().filter(|op| !op.is_empty()) {
            state
                .hub
                .command_result(session_id, raw, CommandOutcome::Rejected, msg);
        }
        state
            .hub
            .broadcast_error(Some(session_id.clone()), msg.to_owned());
        return Some(false);
    };
    let reservation = match state
        .hub
        .reserve_recovery_draft_activation(session_id, id, op)
    {
        Ok(r) => r,
        Err(crate::core::RecoveryReserveErr::NotFound) => return Some(true),
        Err(
            crate::core::RecoveryReserveErr::InvalidOperationId(msg)
            | crate::core::RecoveryReserveErr::OperationInUse(msg)
            | crate::core::RecoveryReserveErr::AlreadyActivating(msg),
        ) => {
            state
                .hub
                .command_result(session_id, op, CommandOutcome::Rejected, msg);
            state
                .hub
                .broadcast_error(Some(session_id.clone()), msg.to_owned());
            return Some(false);
        }
    };
    if let Some(store) = state.store.as_ref() {
        match store.operation_id_used_in_history(session_id, op).await {
            Ok(false) => {}
            Ok(true) => {
                let msg = "Activation operation id is already in use; draft retained";
                state
                    .hub
                    .command_result(session_id, op, CommandOutcome::Rejected, msg);
                state
                    .hub
                    .broadcast_error(Some(session_id.clone()), msg.to_owned());
                return Some(false);
            }
            Err(error) => {
                tracing::warn!(%error, session = %session_id, "failed to verify recovery activation op id");
                let msg = "Could not verify activation operation id; draft retained";
                state
                    .hub
                    .command_result(session_id, op, CommandOutcome::Rejected, msg);
                state
                    .hub
                    .broadcast_error(Some(session_id.clone()), msg.to_owned());
                return Some(false);
            }
        }
    }
    let _guard = state.hub.pending_commit_lock(session_id).await;
    let (mut commit, rx) =
        match state
            .hub
            .begin_durable_recovery_draft_activation(reservation, *front, caller)
        {
            Ok(crate::core::RecoveryActivationCommitStep::CommittedSync) => return Some(true),
            Ok(crate::core::RecoveryActivationCommitStep::Pending(commit, rx)) => (commit, rx),
            Err(msg) => {
                state
                    .hub
                    .command_result(session_id, op, CommandOutcome::Rejected, msg);
                state
                    .hub
                    .broadcast_error(Some(session_id.clone()), msg.to_owned());
                return Some(false);
            }
        };
    match rx.await {
        Ok(Ok(true)) => {
            state
                .hub
                .commit_recovery_draft_activation(&mut commit, caller);
            Some(true)
        }
        _ => {
            state.hub.abort_recovery_draft_activation(&mut commit);
            let msg = "Failed to persist recovery draft activation; draft retained";
            state
                .hub
                .command_result(session_id, op, CommandOutcome::Rejected, msg);
            state
                .hub
                .broadcast_error(Some(session_id.clone()), msg.to_owned());
            Some(false)
        }
    }
}

/// Durably commit owner queue mutations that remove, clear, park, edit, or
/// cancel a still-queued `call-notice:` row before mutating `Hub` or
/// acknowledging the command. Returns `None` when the command does not affect
/// any queued completion notice (so the caller falls through to the fast sync
/// path), or `Some(accepted)` once handled.
pub async fn try_commit_owner_notice_withdrawal(
    state: &Arc<AppState>,
    cmd: &Inbound,
    authored_cmid: &Option<String>,
) -> Option<bool> {
    let (session_id, op) = match cmd {
        Inbound::RemoveQueued { session_id, id, .. } => (
            session_id.as_str(),
            crate::core::OwnerNoticeWithdrawalOp::RemoveQueued { id },
        ),
        Inbound::EditQueued {
            session_id,
            id,
            text,
            content,
            ..
        } => (
            session_id.as_str(),
            crate::core::OwnerNoticeWithdrawalOp::EditQueued { id, text, content },
        ),
        Inbound::ClearQueue { session_id } => (
            session_id.as_str(),
            crate::core::OwnerNoticeWithdrawalOp::ClearQueue,
        ),
        Inbound::QueuedToDraft { session_id, id } => (
            session_id.as_str(),
            crate::core::OwnerNoticeWithdrawalOp::QueuedToDraft { id },
        ),
        Inbound::CancelSubmitted { session_id, cmid } => (
            session_id.as_str(),
            crate::core::OwnerNoticeWithdrawalOp::CancelSubmitted { cmid },
        ),
        _ => return None,
    };
    let _guard = state.hub.pending_commit_lock(session_id).await;
    let (mut preview, ack_rx) = match state
        .hub
        .apply_or_preview_owner_notice_withdrawal(session_id, &op)
    {
        crate::core::OwnerQueueMutation::AppliedSync(changed) => {
            report_owner_queue_mutation_outcome(state, cmd, changed);
            return Some(true);
        }
        crate::core::OwnerQueueMutation::CommitPending(preview, rx) => (preview, rx),
        crate::core::OwnerQueueMutation::Rejected(message) => {
            report_withdrawal_failure(state, session_id, authored_cmid.as_deref(), message);
            return Some(false);
        }
    };
    if state.store.is_none() {
        state.hub.abort_owner_notice_withdrawal(&mut preview);
        report_withdrawal_failure(
            state,
            session_id,
            authored_cmid.as_deref(),
            "durable store unavailable; completion notice withdrawal was not committed",
        );
        return Some(false);
    }
    match ack_rx.await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            state.hub.abort_owner_notice_withdrawal(&mut preview);
            tracing::warn!(
                session_id,
                %error,
                "failed to durably commit completion notice owner withdrawal"
            );
            report_withdrawal_failure(
                state,
                session_id,
                authored_cmid.as_deref(),
                "failed to durably commit completion notice withdrawal",
            );
            return Some(false);
        }
        Err(_) => {
            state.hub.abort_owner_notice_withdrawal(&mut preview);
            report_withdrawal_failure(
                state,
                session_id,
                authored_cmid.as_deref(),
                "durable store writer closed before committing completion notice withdrawal",
            );
            return Some(false);
        }
    }
    let changed = state.hub.commit_owner_notice_withdrawal(&mut preview, &op);
    report_owner_queue_mutation_outcome(state, cmd, changed);
    Some(true)
}

fn report_owner_queue_mutation_outcome(state: &AppState, cmd: &Inbound, changed: bool) {
    match cmd {
        Inbound::CancelSubmitted { session_id, cmid } if changed => {
            state.hub.push(
                session_id,
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "cowboy_prompt_cancelled",
                        "cmid": cmid,
                    }),
                },
            );
        }
        Inbound::RemoveQueued {
            session_id, cmid, ..
        }
        | Inbound::EditQueued {
            session_id, cmid, ..
        } if !changed => {
            report_stale_row(state, session_id, cmid.as_deref());
        }
        _ => {}
    }
}

fn report_withdrawal_failure(
    state: &AppState,
    session_id: &str,
    authored_cmid: Option<&str>,
    message: &str,
) {
    state
        .hub
        .broadcast_error(Some(session_id.to_owned()), message.to_owned());
    if let Some(cmid) = authored_cmid {
        state
            .hub
            .command_result(session_id, cmid, CommandOutcome::Rejected, message);
    }
}

/// Authorize the destination session of a WebSocket `MoveDraft` command so a
/// caller cannot move drafts into a foreign, missing, trashed, system, or
/// managed-child session.
fn refuse_move_draft_target(
    state: &AppState,
    principal: &ProductPrincipal,
    from_session: &str,
    to_session: &str,
    authored_cmid: &Option<String>,
) -> bool {
    refuse_move_draft_target_in(
        &state.hub,
        principal,
        from_session,
        to_session,
        authored_cmid,
    )
}

pub(crate) fn refuse_move_draft_target_in(
    hub: &Hub,
    principal: &ProductPrincipal,
    from_session: &str,
    to_session: &str,
    authored_cmid: &Option<String>,
) -> bool {
    let reject = |outcome: CommandOutcome, message: &str| {
        hub.broadcast_error(Some(from_session.to_owned()), message.to_owned());
        if let Some(cmid) = authored_cmid.as_deref() {
            hub.command_result(from_session, cmid, outcome, message);
        }
        true
    };
    let Some(info) = hub.session_info(to_session) else {
        return reject(
            CommandOutcome::NotFound,
            "destination session no longer exists",
        );
    };
    if !principal.can_mutate(info.meta.owner_user_id.as_deref()) {
        return reject(
            CommandOutcome::Rejected,
            "not allowed to mutate destination session",
        );
    }
    if info.meta.closing || info.meta.system || hub.is_managed_child(to_session) {
        return reject(
            CommandOutcome::Rejected,
            "cannot move drafts into this destination session",
        );
    }
    if info.meta.trashed_at_ms.is_some() {
        return reject(CommandOutcome::Rejected, session_trash::TRASHED_REFUSAL);
    }
    false
}

/// Resolve the original prompt's durable `(cmid, origin)` before acquiring
/// synchronous runtime fences for `Inbound::RetryTurn`. Missing or unreadable
/// records fail closed as non-owner.
pub async fn resolve_retry_original(
    state: &AppState,
    cmd: &Inbound,
) -> Option<(String, crate::prompt_origin::DeliveryOrigin)> {
    resolve_retry_original_in(&state.hub, state.store.as_ref(), cmd).await
}

pub(crate) async fn resolve_retry_original_in(
    hub: &Hub,
    store: Option<&Store>,
    cmd: &Inbound,
) -> Option<(String, crate::prompt_origin::DeliveryOrigin)> {
    let Inbound::RetryTurn { session_id } = cmd else {
        return None;
    };
    let cmid = hub.last_user_prompt_cmid(session_id)?;
    if let Some(origin) = hub.prompt_dispatch_origin(session_id, &cmid) {
        return Some((cmid, origin));
    }
    let store = store?;
    let record = store.prompt_dispatch(session_id, &cmid).await.ok()??;
    Some((cmid, record.origin))
}

/// Retry the last prompt using its proven original dispatch origin (if
/// recorded) and the retrying caller as trigger.
pub fn retry_turn_command(
    state: &AppState,
    caller: &crate::prompt_origin::RequestCaller,
    session_id: &str,
    resolved_original: Option<(String, crate::prompt_origin::DeliveryOrigin)>,
) -> Result<(), String> {
    state.supervisor.prepare_session(session_id).map(|_| {
        state.hub.retry_turn(session_id, caller, resolved_original);
    })
}

#[cfg(test)]
mod send_now_expiry_tests {
    use super::send_now_outside_window;
    use crate::core::{HELD_SEND_CLOCK_SKEW_MS, HELD_SEND_REPLAY_HORIZON_MS};

    #[test]
    fn only_a_send_now_outside_the_receipt_window_is_refused() {
        let now = 10 * HELD_SEND_REPLAY_HORIZON_MS;
        assert!(!send_now_outside_window(None, now));
        assert!(!send_now_outside_window(Some(now), now));
        // A client clock somewhat ahead is not refused.
        assert!(!send_now_outside_window(
            Some(now + HELD_SEND_CLOCK_SKEW_MS),
            now
        ));
        assert!(send_now_outside_window(
            Some(now + HELD_SEND_CLOCK_SKEW_MS + 1),
            now
        ));
        assert!(!send_now_outside_window(
            Some(now - HELD_SEND_REPLAY_HORIZON_MS),
            now
        ));
        assert!(send_now_outside_window(
            Some(now - HELD_SEND_REPLAY_HORIZON_MS - 1),
            now
        ));
    }
}
