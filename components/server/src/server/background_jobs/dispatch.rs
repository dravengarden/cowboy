//! Queued prompt dispatcher and dispatch fencing.

use super::super::*;

/// Drain the Hub→dispatcher channel: each [`DispatchReq`] is a queued prompt the
/// Hub decided is ready to send. We forward it to the session's agent. On
/// success, derive the auto-title from the first prompt (a no-op after the
/// first); on failure, clear the in-flight guard (so the queue can keep
/// draining) and surface the error to every client.
pub(in crate::server) async fn run_dispatcher(
    state: Arc<AppState>,
    mut rx: mpsc::Receiver<DispatchReq>,
    mut shutdown: watch::Receiver<bool>,
) {
    // A slow cold target must not hold up prompts for other sessions. Hub
    // admission still permits only one outstanding dispatch per session.
    let mut dispatches = tokio::task::JoinSet::new();
    loop {
        let req = tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { rx.close(); }
                rx.recv().await
            }
            result = dispatches.join_next(), if !dispatches.is_empty() => {
                if let Some(Err(error)) = result { tracing::error!(%error, "dispatch task failed"); }
                continue;
            }
            req = rx.recv(), if dispatches.len() < 32 => req,
        };
        let Some(req) = req else { break };
        dispatches.spawn(dispatch_one(Arc::clone(&state), req));
    }
    while let Some(result) = dispatches.join_next().await {
        if let Err(error) = result {
            tracing::error!(%error, "dispatch task failed while draining");
        }
    }
    tracing::info!("dispatcher shutting down (channel closed)");
}

pub(in crate::server) async fn dispatch_preflight<T>(
    shutdown: &mut watch::Receiver<bool>,
    preparation: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::select! {
        biased;
        _ = shutdown.wait_for(|stopping| *stopping) => {
            Err("Controller disconnected for shutdown; prompt remains queued".into())
        }
        result = preparation => result,
    }
}

pub(in crate::server) async fn dispatch_one(state: Arc<AppState>, req: DispatchReq) {
    if req.ownership.supervision().is_some() {
        superagent::dispatch(state, req).await;
        return;
    }
    // Named-only accounts: admission, account state and uncertain effects,
    // before anything can revive a worker.
    let gate = provider_accounts::controls::before_dispatch(&state.accounts, &req.session_id);
    match gate.await {
        Ok(()) => dispatch_admitted(state, req).await,
        Err(reason) => {
            // The dispatch ownership is held until the prompt is requeued.
            let DispatchReq {
                session_id,
                text,
                content,
                cmid,
                ownership: _ownership,
                ..
            } = req;
            retain_failed_dispatch(&state.hub, session_id, text, content, cmid, &reason);
        }
    }
}

async fn dispatch_admitted(state: Arc<AppState>, req: DispatchReq) {
    let hub = &state.hub;
    let DispatchReq {
        ownership: _ownership,
        session_id,
        text,
        content,
        cmid,
        explicit,
    } = req;
    let retain = |session_id: String, text, content, cmid, error: &str| {
        let session = session_id.clone();
        retain_failed_dispatch(hub, session_id, text, content, cmid, error);
        if explicit {
            retain_explicit_intent(hub, &session, error);
        }
    };
    // A user send retries a preparation that failed permanently; the
    // preflight below then holds the prompt until the target is ready.
    if explicit {
        execution::sessions::retry_blocked(&state, &session_id);
    }
    let mut shutdown = state.shutdown.clone();
    let recovery = match dispatch_preflight(
        &mut shutdown,
        execution::recovery::before_dispatch(&state, &session_id),
    )
    .await
    {
        Ok(recovery) => recovery,
        Err(error) => {
            retain(session_id, text, content, cmid, &error);
            return;
        }
    };
    // A failed or interrupted Web workspace preparation left this session on
    // its source checkout; prepare it again before any worker can start.
    // Completion drains the queue; an explicit intent would re-drain it on
    // every maintenance tick while the Machine is still preparing.
    if let Some(wait) = workspace_preparation::resume(&state, &session_id) {
        retain_failed_dispatch(hub, session_id, text, content, cmid, &wait);
        return;
    }
    // A directory MCP default must be part of the first launch.
    if let Err(error) = dispatch_preflight(&mut shutdown, async {
        mcp_bindings::defaults::apply(&state, &session_id).await;
        Ok(())
    })
    .await
    {
        retain(session_id, text, content, cmid, &error);
        return;
    }
    session_context::rollover::before_dispatch(&state, &session_id, &text, cmid.as_deref()).await;
    // Links wait for a real message: a preamble would break a slash command.
    if !crate::session_context::is_command(&text)
        && let Err(error) =
            session_context::claim_carrier(&state, &session_id, cmid.as_deref()).await
    {
        retain(session_id, text, content, cmid, &error);
        return;
    }
    let Some(blocks) = build_prompt_blocks(&text, &content) else {
        tracing::warn!(session = %session_id, "queued prompt had no content; dropping");
        hub.clear_in_flight(&session_id);
        return;
    };
    let plugin_key = provider_fence_key_for_session(hub, &session_id);
    if cmid
        .as_deref()
        .is_some_and(|id| id.starts_with(crate::agent_model::CALL_NOTICE_PREFIX))
    {
        match check_sync_dispatch_gates(
            &state,
            plugin_key.as_ref(),
            &session_id,
            cmid.as_deref(),
            recovery.is_none(),
        ) {
            SyncSendOutcome::Ready => {}
            other => {
                finish_deferred_dispatch(hub, other, session_id, text, content, cmid, explicit);
                return;
            }
        }
    }
    let notice_rt = notice_runtime_before_send(&state, &session_id, cmid.as_deref());
    let claim_token =
        match managed_calls::before_notice_dispatch(&state, &session_id, cmid.as_deref()).await {
            managed_calls::DispatchGate::Proceed(token) => token,
            managed_calls::DispatchGate::DropDuplicate => {
                hub.clear_in_flight(&session_id);
                return;
            }
            managed_calls::DispatchGate::Requeue(reason) => {
                retain(session_id, text, content, cmid, &reason);
                return;
            }
        };
    let explicit_command_id = claim_token.as_ref().map(|token| token.command_id.clone());
    let outcome = send_under_dispatch_fence(
        &state,
        plugin_key.as_ref(),
        &session_id,
        &text,
        &content,
        blocks,
        cmid.as_ref(),
        recovery,
        explicit_command_id,
    );
    match outcome {
        SyncSendOutcome::Sent { title } => {
            // The message reached the agent: the latest in-place agent change
            // is now part of the conversation, so Settings loses its Back.
            config_revert::clear(hub, &session_id);
            // Successful sends synchronously enter the runtime pending set.
            // Failed sends retain dispatch ownership until requeue completes.
            drop(_ownership);
            session_maintenance::clear_wait(hub, &session_id);
            if let Some(t) = title {
                hub.auto_title(&session_id, t);
            }
        }
        other => {
            let transient_disconnect = matches!(
                &other,
                SyncSendOutcome::SendError(error) if is_transient_machine_disconnect(error)
            );
            managed_calls::after_notice_dispatch_error(
                &state,
                &session_id,
                cmid.as_deref(),
                claim_token.as_ref(),
            )
            .await;
            finish_deferred_dispatch(hub, other, session_id, text, content, cmid, explicit);
            rewake_notice_if_reconnected(&state, transient_disconnect, notice_rt).await;
        }
    }
}

fn notice_runtime_before_send(
    state: &AppState,
    session_id: &str,
    cmid: Option<&str>,
) -> Option<(String, Option<Arc<RemoteRuntime>>)> {
    let machine_id = cmid
        .filter(|id| id.starts_with(crate::agent_model::CALL_NOTICE_PREFIX))
        .and_then(|_| state.hub.session_info(session_id))
        .map(|info| info.meta.machine_id)?;
    let runtime = state.runtime_router.runtime(&machine_id);
    Some((machine_id, runtime))
}

async fn rewake_notice_if_reconnected(
    state: &AppState,
    transient_disconnect: bool,
    notice_rt: Option<(String, Option<Arc<RemoteRuntime>>)>,
) {
    // If the Machine reconnected while claim release was awaited, the reconnect
    // scan saw an empty queue; wake the requeued notice now.
    if transient_disconnect
        && let Some((machine_id, runtime_before_send)) = notice_rt
        && let Some(current_runtime) = state.runtime_router.runtime(&machine_id)
        && runtime_before_send
            .as_ref()
            .is_none_or(|prev| !Arc::ptr_eq(prev, &current_runtime))
    {
        managed_calls::wake_notices_for_machine(&state.hub, state.store.as_ref(), &machine_id)
            .await;
    }
}

enum SyncSendOutcome {
    Ready,
    RequeueByFence,
    WakeForAgent(Result<(), String>),
    HandoffRefusal(String),
    Sent { title: Option<String> },
    SendError(String),
}

fn check_sync_dispatch_gates(
    state: &Arc<AppState>,
    plugin_key: Option<&(String, String)>,
    session_id: &str,
    cmid: Option<&str>,
    recovery_none: bool,
) -> SyncSendOutcome {
    let dispatch_fence = state.plugin_lifecycle_fences.read();
    check_sync_dispatch_gates_locked(
        state,
        &dispatch_fence,
        plugin_key,
        session_id,
        cmid,
        recovery_none,
    )
}

fn check_sync_dispatch_gates_locked(
    state: &Arc<AppState>,
    dispatch_fence: &std::collections::HashMap<(String, String), PluginFenceState>,
    plugin_key: Option<&(String, String)>,
    session_id: &str,
    cmid: Option<&str>,
    recovery_none: bool,
) -> SyncSendOutcome {
    if plugin_key
        .and_then(|key| dispatch_fence.get(key))
        .is_some_and(|fence| *fence != PluginFenceState::Installing)
    {
        return SyncSendOutcome::RequeueByFence;
    }
    // A continuation's first message stays queued (visible and cancellable)
    // while its agent launches; reaching Running drains it to that agent. The
    // Provider lifecycle fence above stays held across the launch.
    if recovery_none
        && crate::session_context::waits_for_agent(&state.hub, session_id, cmid)
        && !state.supervisor.has_live_worker(session_id)
    {
        return SyncSendOutcome::WakeForAgent(state.supervisor.wake_for_queued_prompt(session_id));
    }
    // The account gate ran before the asynchronous recovery above; an
    // uncertain effect, signal or block that arrived meanwhile parks the
    // prompt here, with nothing awaited between this check and the send.
    if let Some(wait) = provider_accounts::controls::handoff_refusal(&state.accounts, session_id) {
        return SyncSendOutcome::HandoffRefusal(wait);
    }
    SyncSendOutcome::Ready
}

#[allow(clippy::too_many_arguments)]
fn send_under_dispatch_fence(
    state: &Arc<AppState>,
    plugin_key: Option<&(String, String)>,
    session_id: &str,
    text: &str,
    content: &[serde_json::Value],
    blocks: Vec<ContentBlock>,
    cmid: Option<&String>,
    recovery: Option<crate::execution_environment::RecoveryV1>,
    explicit_command_id: Option<String>,
) -> SyncSendOutcome {
    let dispatch_fence = state.plugin_lifecycle_fences.read();
    match check_sync_dispatch_gates_locked(
        state,
        &dispatch_fence,
        plugin_key,
        session_id,
        cmid.map(String::as_str),
        recovery.is_none(),
    ) {
        SyncSendOutcome::Ready => {}
        blocked => return blocked,
    }
    let title = first_prompt_title(text, content);
    let trace = state
        .observability
        .dispatch_trace(session_id, cmid.map(String::as_str));
    match state.supervisor.send_traced_after_execution_recovery(
        session_id,
        AgentCommand::Prompt(blocks, cmid.cloned(), None),
        trace.clone(),
        recovery,
        explicit_command_id,
    ) {
        Ok(()) => SyncSendOutcome::Sent { title },
        Err(error) => {
            if let Some(trace) = &trace {
                state.observability.finish_delivery_trace(trace, false);
            }
            SyncSendOutcome::SendError(error)
        }
    }
}

fn finish_deferred_dispatch(
    hub: &Hub,
    outcome: SyncSendOutcome,
    session_id: String,
    text: String,
    content: Vec<serde_json::Value>,
    cmid: Option<String>,
    explicit: bool,
) {
    match outcome {
        SyncSendOutcome::RequeueByFence => {
            hub.requeue_prompt(&session_id, text, content, cmid);
        }
        SyncSendOutcome::WakeForAgent(Ok(())) => {
            hub.requeue_prompt(&session_id, text, content, cmid);
            // The launch may already have reported Running.
            hub.context_state_changed(&session_id);
        }
        SyncSendOutcome::WakeForAgent(Err(error)) => {
            let session = session_id.clone();
            retain_failed_dispatch(hub, session_id, text, content, cmid, &error);
            if explicit {
                retain_explicit_intent(hub, &session, &error);
            }
        }
        SyncSendOutcome::HandoffRefusal(wait) => {
            retain_failed_dispatch(hub, session_id, text, content, cmid, &wait);
        }
        SyncSendOutcome::SendError(error) => {
            tracing::warn!(session = %session_id, error = %error, "queued dispatch failed");
            let session = session_id.clone();
            retain_failed_dispatch(hub, session_id, text, content, cmid, &error);
            if explicit {
                retain_explicit_intent(hub, &session, &error);
            }
        }
        SyncSendOutcome::Ready | SyncSendOutcome::Sent { .. } => {}
    }
}

/// Restore a prompt that left the durable queue but never reached its runtime.
///
/// A remote machine can disconnect between queue drain and dispatch (notably
/// while the controller is restarting). Keeping the original `cmid` makes the
/// restore idempotent and lets every client reconcile the same queued item.
pub(in crate::server) fn retain_failed_dispatch(
    hub: &Hub,
    session_id: String,
    text: String,
    content: Vec<serde_json::Value>,
    cmid: Option<String>,
    error: &str,
) {
    hub.requeue_prompt(&session_id, text, content, cmid);
    if let Some(reason) = error.strip_prefix(session_maintenance::WAIT_PREFIX) {
        session_maintenance::wait(hub, &session_id, reason);
        return;
    }
    // A Controller restart authenticates the Machine WebSocket before the
    // ACP runtime handshake finishes. The prompt is durable; do not toast a
    // transient "not connected" as a user-facing send failure.
    if is_transient_machine_disconnect(error) {
        tracing::warn!(
            session = %session_id,
            error = %error,
            "queued dispatch deferred until Machine runtime is ready"
        );
        return;
    }
    // A refusal that is not a wait ends an earlier retry: the prompt stays
    // queued for explicit intervention instead of failing every two seconds.
    session_maintenance::clear_wait(hub, &session_id);
    // Restore-timeout holds already explain the queue and the next step.
    // Prefixing "send failed:" makes a protective refusal look like a
    // transport crash.
    let message = if crate::provider_behavior::is_session_restore_timeout_error(error) {
        error.to_owned()
    } else {
        format!("send failed: {error}")
    };
    hub.broadcast_error(Some(session_id), message);
}
