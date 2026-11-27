//! Routing of notifications that concern ACP subagents (docs/subagents.md):
//! the bounded parent-row snapshot, and the passive observation of each
//! child's own updates for its full transcript.

use std::sync::atomic::Ordering;
use std::time::Instant;

use agent_client_protocol::schema::v1::{
    CreateElicitationRequest, ElicitationMode, RequestPermissionRequest, SessionNotification,
    SessionUpdate,
};

use super::{ClientState, observed_at_ms, report_background_tasks};
use crate::agent_model::Event;

/// Route a notification that concerns subagents: a child session's own
/// update, or the root's `subagent_update` / linked `session_message`. Returns
/// whether the notification still belongs to the root timeline.
pub(super) fn route_subagent_notification(
    state: &ClientState,
    notif: &SessionNotification,
) -> bool {
    let session = notif.session_id.0.as_ref();
    let concerns_subagents = matches!(
        notif.update,
        SessionUpdate::SubagentUpdate(_)
            | SessionUpdate::SessionMessage(_)
            | SessionUpdate::SessionMessageChunk(_)
    ) || state.subagents.lock().owns(session);
    let now = Instant::now();
    let now_ms = u64::try_from(observed_at_ms()).unwrap_or_default();
    if !concerns_subagents {
        let due = state.subagents.lock().due(now, now_ms);
        publish_subagents(state, due);
        return true;
    }
    let update = match serde_json::to_value(&notif.update) {
        Ok(update) => update,
        Err(error) => {
            tracing::warn!(%error, "serializing subagent update");
            return false;
        }
    };
    let (routed, observed) = {
        let mut subagents = state.subagents.lock();
        let routed = subagents.route(session, &update, now, now_ms);
        let observed = !routed.root && subagents.observes(session, &update);
        (routed, observed)
    };
    publish_subagents(state, routed.snapshots);
    if !routed.root {
        if update["sessionUpdate"] == "tool_call_update"
            && matches!(update["status"].as_str(), Some("completed" | "failed"))
            && let Some(tool_id) = update["toolCallId"].as_str()
        {
            retire_child_tool_requests(state, session, tool_id);
        }
        // A working child proves the turn is alive even while the parent is
        // silent: the watchdog must not cancel a parent that waits on it, and
        // a prompt whose child already acted must never be replayed.
        state.backoff.lock().take();
        *state.last_progress.lock() = Instant::now();
        if let Some(prompt) = state.current_prompt() {
            prompt.visible_update.store(true, Ordering::SeqCst);
        }
    }
    if observed {
        // Passive and after every decision above: the agent's own handling
        // of this notification is already complete and never waits on it.
        state
            .sink
            .observe_subagent(&state.session_id, session, update);
    }
    routed.root
}

pub(super) fn publish_subagents(state: &ClientState, snapshots: Vec<serde_json::Value>) {
    if snapshots.is_empty() {
        return;
    }
    for update in snapshots {
        state.sink.push(&state.session_id, Event::Update { update });
    }
    let working = state.subagents.lock().working();
    let mut activity = state.activity.lock();
    activity.working_subagents = working;
    report_background_tasks(state, &mut activity);
}

/// Retire any pending permission or elicitation request tied to `(child_session_id,
/// tool_call_id)` when the child's terminal `tool_call_update` is diverted out
/// of the root timeline. Emits `PermissionResolved` and cancels the waiting
/// responder without disturbing parent or sibling-child requests.

pub(super) fn track_permission_tool(
    state: &ClientState,
    request_id: &str,
    req: &RequestPermissionRequest,
) {
    state.pending_tools.lock().insert(
        request_id.to_owned(),
        (
            req.session_id.0.to_string(),
            req.tool_call.tool_call_id.0.to_string(),
        ),
    );
}

pub(super) fn track_elicitation_tool(
    state: &ClientState,
    request_id: &str,
    req: &CreateElicitationRequest,
) {
    if let Some(session) = super::elicitation_session_id(req)
        && let Some(tool_id) = match &req.mode {
            ElicitationMode::Form(form) => super::elicitation_tool_call_id(&form.scope),
            ElicitationMode::Url(url) => super::elicitation_tool_call_id(&url.scope),
            _ => None,
        }
    {
        state
            .pending_tools
            .lock()
            .insert(request_id.to_owned(), (session.to_owned(), tool_id));
    }
}

pub(super) fn retire_child_tool_requests(
    state: &ClientState,
    child_session_id: &str,
    tool_call_id: &str,
) {
    let matched: Vec<String> = {
        let mut tools = state.pending_tools.lock();
        let ids: Vec<String> = tools
            .iter()
            .filter(|(_, (sid, tid))| sid == child_session_id && tid == tool_call_id)
            .map(|(req_id, _)| req_id.clone())
            .collect();
        for id in &ids {
            tools.remove(id);
        }
        ids
    };
    for request_id in matched {
        let tx = state.pending.lock().remove(&request_id);
        state.elicitations.lock().remove(&request_id);
        super::forget_url_elicitation(state, &request_id);
        if let Some(tx) = tx {
            let _ = tx.send(None);
            state.sink.push(
                &state.session_id,
                Event::PermissionResolved {
                    request_id,
                    option_id: None,
                },
            );
        }
    }
}
