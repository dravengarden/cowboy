//! The Inbound command steps that handle_command dispatches to: admission
//! refusal, field extraction, and the larger per-command bodies.

use super::*;
use std::ops::ControlFlow;

/// Refuse a command for a missing, foreign, view-only, trashed or
/// context-pending session, reporting why. Returns whether it was refused.
pub(super) fn refuse_command(
    state: &Arc<AppState>,
    principal: &ProductPrincipal,
    cmd: &Inbound,
    session_id_for_err: &Option<String>,
    authored_cmid: &Option<String>,
) -> bool {
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
        && matches!(&cmd, Inbound::Prompt { .. } | Inbound::Submit { .. })
        && crate::session_context::pending(&state.hub, sid)
    {
        let message = "Context changes are pending. Send with the current context revision or cancel those changes.";
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
pub(super) fn command_authored_cmid(cmd: &Inbound) -> Option<String> {
    match cmd {
        Inbound::Submit { cmid, .. }
        | Inbound::AddDraft { cmid, .. }
        | Inbound::ScheduleDraft { cmid, .. }
        | Inbound::EditQueued { cmid, .. }
        | Inbound::RemoveQueued { cmid, .. }
        | Inbound::EditDraft { cmid, .. }
        | Inbound::RemoveDraft { cmid, .. } => cmid.clone(),
        _ => None,
    }
}

/// The session a command addresses, for refusal and error attribution.
pub(super) fn command_session_id(cmd: &Inbound) -> Option<String> {
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
pub(super) fn set_queue_editing_command(
    state: &Arc<AppState>,
    held: &mut HashMap<String, (String, u64)>,
    session_id: String,
    id: Option<String>,
) -> Result<(), String> {
    // Track the hold per-connection so a mid-edit disconnect releases it
    // (the hold is global server state — see handle_ws teardown).
    match &id {
        Some(mid) => {
            let epoch = state.hub.set_queue_editing(&session_id, Some(mid.clone()));
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
pub(super) fn submit_command(
    state: &Arc<AppState>,
    principal: &ProductPrincipal,
    command_trace: Option<&crate::runtime_trace::SpanTimer>,
    session_id: String,
    text: String,
    content: Vec<serde_json::Value>,
    cmid: Option<String>,
    force: bool,
    front: bool,
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
        if force {
            // Long-press send: jump to the front of the queue and interrupt the
            // running turn so it runs next (same end-state as a queued row's
            // force-push). force_submit returns true when it queued (vs a direct
            // idle dispatch); only then, and only on a live turn, do we Cancel.
            let queued = state
                .hub
                .force_submit_once(&session_id, text, content, cmid, true);
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
                state.hub.drain_now(&session_id);
                Ok(())
            }
        } else if front {
            // "Jump to front" without interrupting: front-insert so it runs next
            // after the current turn, ahead of the rest of the queue. Same
            // front placement as force, but no Cancel.
            let _ = state
                .hub
                .force_submit(&session_id, text, content, cmid, false);
            Ok(())
        } else {
            state.hub.submit(&session_id, text, content, cmid);
            Ok(())
        }
    })
}

/// Clear the conversation, through execution recovery when the session is
/// bound to a live execution.
#[allow(clippy::too_many_arguments)]
pub(super) fn reset_session_command(
    state: &Arc<AppState>,
    session_id: String,
) -> ControlFlow<bool, Result<(), String>> {
    ControlFlow::Continue({
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
                            state.hub.prepare_context_reset(&session_id);
                            state.hub.clear_transcript(&session_id);
                            state.hub.mark_context_cleared(&session_id);
                            let result = match recovery {
                                Some(intent) => state.supervisor.recover_execution_session(&session_id, intent),
                                None => state.supervisor.reset_session(&session_id),
                            };
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
        // 1. Forget the resumable agent id so the respawn does a FRESH
        //    session/new (clean context) instead of session/load.
        // 2. Destructively clear the old in-memory + durable transcript.
        // 3. Drop the new timeline boundary marker.
        // 4. Atomically fence + replace the worker. This must not use the
        //    permanent delete path: the Machine broker retains delete tombstones to
        //    reject stale launches for genuinely deleted sessions.
        state.hub.prepare_context_reset(&session_id);
        state.hub.clear_transcript(&session_id);
        state.hub.mark_context_cleared(&session_id);
        match state.supervisor.reset_session(&session_id) {
            Ok(()) => Ok(()),
            Err(e) => {
                tracing::warn!(session_id = %session_id, error = %e, "reset: respawn failed");
                Ok(())
            }
        }
    })
}

/// Open a session, reviving it unless its last turn was interrupted or it
/// failed with a terminal Provider error.
#[allow(clippy::too_many_arguments)]
pub(super) fn open_session_command(
    state: &Arc<AppState>,
    session_id: String,
) -> Result<(), String> {
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
pub(super) fn prompt_command(
    state: &Arc<AppState>,
    session_id: String,
    text: String,
    content: Vec<serde_json::Value>,
) -> ControlFlow<bool, Result<(), String>> {
    ControlFlow::Continue({
        // Derive an auto-title from the first prompt before text/content are
        // consumed below. auto_title no-ops unless the title is still the
        // cwd default, so this only "takes" on a session's first prompt and
        // never overrides a manual rename.
        let auto = first_prompt_title(&text, &content);
        if !(text.is_empty() && content.is_empty()) {
            config_revert::clear(&state.hub, &session_id);
        }
        let blocks: Vec<ContentBlock> = if content.is_empty() {
            if text.is_empty() {
                tracing::warn!("Prompt with neither text nor content; dropping");
                state.hub.broadcast_error(
                    Some(session_id),
                    "empty prompt: no text or content blocks".to_owned(),
                );
                return ControlFlow::Break(false);
            }
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
        // API direct prompt — no optimistic client, so no cmid.
        let result = state
            .supervisor
            .send(&session_id, AgentCommand::Prompt(blocks, None, None));
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
pub(super) fn first_prompt_title(text: &str, content: &[serde_json::Value]) -> Option<String> {
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
pub(super) fn report_stale_row(state: &AppState, session_id: &str, cmid: Option<&str>) {
    if let Some(cmid) = cmid {
        state.hub.command_result(
            session_id,
            cmid,
            CommandOutcome::Stale,
            "this message no longer exists",
        );
    }
}

/// The folder tree a principal organizes: their own while product auth is on,
/// the shared unowned tree while it is off (mirroring session ownership).
pub(super) fn folder_actor(state: &AppState, principal: &ProductPrincipal) -> FolderActor {
    FolderActor {
        user_id: state
            .product_auth_enabled
            .then(|| principal.user_id.clone()),
        sees_all: principal.sees_every_session(),
    }
}

pub(super) fn apply_visible_reorder(
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

pub(super) fn filter_submitted_order(
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
