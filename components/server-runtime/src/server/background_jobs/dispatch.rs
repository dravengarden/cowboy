//! Queued prompt dispatcher and dispatch fencing.

use super::super::*;

/// Drain the Hub→dispatcher channel: each [`DispatchReq`] is a queued prompt the
/// Hub decided is ready to send. We forward it to the session's agent. On
/// success, derive the auto-title from the first prompt (a no-op after the
/// first); on failure, clear the in-flight guard (so the queue can keep
/// draining) and surface the error to every client.
pub async fn run_dispatcher(
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

pub async fn dispatch_preflight<T>(
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

pub async fn dispatch_one(state: Arc<AppState>, req: DispatchReq) {
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
                origin,
                initiator,
                explicit,
                ownership: _ownership,
                ..
            } = req;
            // A paused send refused by a limit the gate just refreshed keeps
            // its intent, like any later transient refusal.
            retain_failed_dispatch(
                &state.hub, session_id, text, content, cmid, origin, initiator, &reason, explicit,
            );
        }
    }
}

enum PreflightOutcome {
    Ready(Option<Box<crate::execution_environment::RecoveryV1>>),
    Retain(String),
    WorkspaceWait(String),
}

async fn prepare_dispatch_preflight(
    state: &Arc<AppState>,
    session_id: &str,
    text: &str,
    content: &[serde_json::Value],
    cmid: Option<&str>,
    dispatch_origin: &crate::prompt_origin::DeliveryOrigin,
) -> PreflightOutcome {
    let mut shutdown = state.shutdown.clone();
    let recovery = match dispatch_preflight(
        &mut shutdown,
        execution::recovery::before_dispatch(state, session_id),
    )
    .await
    {
        Ok(recovery) => recovery,
        Err(error) => return PreflightOutcome::Retain(error),
    };
    // A failed or interrupted Web workspace preparation left this session on
    // its source checkout; prepare it again before any worker can start.
    // Completion drains the queue; an explicit intent would re-drain it on
    // every maintenance tick while the Machine is still preparing.
    if let Some(wait) = workspace_preparation::resume(state, session_id) {
        return PreflightOutcome::WorkspaceWait(wait);
    }
    // A directory MCP default must be part of the first launch.
    if let Err(error) = dispatch_preflight(&mut shutdown, async {
        mcp_bindings::defaults::apply(state, session_id).await;
        Ok(())
    })
    .await
    {
        return PreflightOutcome::Retain(error);
    }
    let is_command_or_compaction = crate::session_context::is_command_blocks(content)
        || crate::wrap_up::is_compaction_prompt_blocks(content);
    session_context::rollover::before_dispatch(
        state,
        session_id,
        text,
        content,
        cmid,
        dispatch_origin,
    )
    .await;
    // Links wait for a real message: a preamble would break a slash command or compaction.
    if !is_command_or_compaction
        && let Err(error) = session_context::claim_carrier(state, session_id, cmid).await
    {
        return PreflightOutcome::Retain(error);
    }
    PreflightOutcome::Ready(recovery.map(Box::new))
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
        origin,
        initiator,
        dispatch_origin,
        trigger,
    } = req;
    let retain =
        |sid: String, text, content, cmid: Option<String>, origin, initiator, error: &str| {
            retain_failed_dispatch(
                hub, sid, text, content, cmid, origin, initiator, error, explicit,
            );
        };
    // A user send retries a preparation that failed permanently; the
    // preflight below then holds the prompt until the target is ready.
    if explicit {
        execution::sessions::retry_blocked(&state, &session_id);
    }
    let Some(blocks) = build_prompt_blocks(&text, &content) else {
        tracing::warn!(session = %session_id, "queued prompt had no content; dropping");
        hub.clear_in_flight(&session_id);
        return;
    };
    let canonical_content: Vec<serde_json::Value> = blocks
        .iter()
        .map(|block| serde_json::to_value(block).expect("ACP content serializes"))
        .collect();
    let recovery = match prepare_dispatch_preflight(
        &state,
        &session_id,
        &text,
        &canonical_content,
        cmid.as_deref(),
        &dispatch_origin,
    )
    .await
    {
        PreflightOutcome::Ready(recovery) => recovery.map(|r| *r),
        PreflightOutcome::Retain(error) => {
            retain(session_id, text, content, cmid, origin, initiator, &error);
            return;
        }
        PreflightOutcome::WorkspaceWait(wait) => {
            // Preparation drains the queue itself; an explicit intent would
            // re-drain it on every maintenance tick meanwhile.
            retain_failed_dispatch(
                hub, session_id, text, content, cmid, origin, initiator, &wait, false,
            );
            return;
        }
    };
    let plugin_key = provider_fence_key_for_session(hub, &session_id);
    if let Some(other) = notice_preflight_gate(
        &state,
        plugin_key.as_ref(),
        &session_id,
        cmid.as_deref(),
        recovery.is_none(),
    ) {
        finish_deferred_dispatch(
            hub, other, session_id, text, content, cmid, origin, initiator, explicit,
        );
        return;
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
                retain(session_id, text, content, cmid, origin, initiator, &reason);
                return;
            }
        };
    let dispatch_cmid =
        match record_dispatch_provenance(&state, &session_id, cmid, dispatch_origin, trigger).await
        {
            Ok((dispatch_cmid, _)) => dispatch_cmid,
            Err((retired_cmid, "prompt operation id was durably retired")) => {
                managed_calls::after_notice_dispatch_error(
                    &state,
                    &session_id,
                    Some(retired_cmid.as_str()),
                    claim_token.as_ref(),
                )
                .await;
                hub.confirm_durably_retired_cmid(&session_id, &retired_cmid);
                hub.clear_in_flight(&session_id);
                return;
            }
            Err((failed_cmid, reason)) => {
                managed_calls::after_notice_dispatch_error(
                    &state,
                    &session_id,
                    Some(failed_cmid.as_str()),
                    claim_token.as_ref(),
                )
                .await;
                retain(
                    session_id,
                    text,
                    content,
                    Some(failed_cmid),
                    origin,
                    initiator,
                    reason,
                );
                return;
            }
        };
    let cmid = Some(dispatch_cmid);
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
    finish_dispatch_outcome(
        &state,
        outcome,
        _ownership,
        session_id,
        text,
        content,
        cmid,
        origin,
        initiator,
        explicit,
        claim_token,
        notice_rt,
    )
    .await;
}

/// Persist Controller dispatch provenance before handoff. If a client-supplied
/// `cmid` collides with an earlier dispatch carrying a different `(origin,
/// trigger)`, mint a fresh dispatch cmid so a caller can never inherit an
/// earlier owner dispatch's provenance.
pub async fn record_dispatch_provenance(
    state: &AppState,
    session_id: &str,
    preferred_cmid: Option<String>,
    dispatch_origin: crate::prompt_origin::DeliveryOrigin,
    trigger: crate::prompt_origin::DeliveryOrigin,
) -> Result<(String, i64), (String, &'static str)> {
    record_dispatch_provenance_in(
        &state.hub,
        state.store.as_ref(),
        session_id,
        preferred_cmid,
        dispatch_origin,
        trigger,
    )
    .await
}

pub(crate) async fn record_dispatch_provenance_in(
    hub: &Hub,
    store: Option<&Store>,
    session_id: &str,
    preferred_cmid: Option<String>,
    dispatch_origin: crate::prompt_origin::DeliveryOrigin,
    trigger: crate::prompt_origin::DeliveryOrigin,
) -> Result<(String, i64), (String, &'static str)> {
    let is_reserved_cmid = |id: &str| {
        crate::agent_model::is_reserved_call_notice_cmid(id)
            || id.starts_with(crate::core::MANAGED_CALL_CMID_PREFIX)
            || id.starts_with(crate::agent_model::PEER_NOTICE_PREFIX)
            || id.starts_with("cowboy-superagent:")
    };
    let fresh_cmid = || format!("cowboy-dispatch:{}", uuid::Uuid::new_v4().simple());
    let mut dispatch_cmid = preferred_cmid.unwrap_or_else(fresh_cmid);
    if hub
        .prompt_dispatch_record(session_id, &dispatch_cmid)
        .is_some_and(|existing| existing.origin != dispatch_origin)
    {
        if is_reserved_cmid(&dispatch_cmid) {
            return Err((
                dispatch_cmid,
                "conflicting provenance for reserved message id",
            ));
        }
        dispatch_cmid = fresh_cmid();
    }
    let dispatched_at_ms = chrono::Utc::now().timestamp_millis();
    if let Some(store) = store {
        let mut record = crate::prompt_origin::PromptDispatchRecord {
            session_id: session_id.to_owned(),
            cmid: dispatch_cmid.clone(),
            origin: dispatch_origin.clone(),
            trigger: trigger.clone(),
            dispatched_at_ms,
            undelivered_retired: false,
            dispatch_seq: 0,
        };
        let Some(_op_guard) = hub.reserve_in_flight_dispatch_cmid(session_id, &dispatch_cmid)
        else {
            return Err((
                dispatch_cmid,
                "operation id is reserved by an active recovery draft activation",
            ));
        };
        match store.record_prompt_dispatch_if_compatible(&record).await {
            Ok(true) => {}
            Ok(false) => {
                if store
                    .prompt_dispatch(session_id, &dispatch_cmid)
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|row| row.undelivered_retired)
                {
                    return Err((dispatch_cmid, "prompt operation id was durably retired"));
                }
                if is_reserved_cmid(&dispatch_cmid) {
                    return Err((
                        dispatch_cmid,
                        "conflicting provenance for reserved message id",
                    ));
                }
                dispatch_cmid = fresh_cmid();
                record.cmid = dispatch_cmid.clone();
                if let Err(error) = store.record_prompt_dispatch(&record).await {
                    tracing::error!(%error, session = %session_id, "prompt_dispatches write failed");
                    return Err((dispatch_cmid, "dispatch provenance store unavailable"));
                }
            }
            Err(error) => {
                tracing::error!(%error, session = %session_id, "prompt_dispatches write failed");
                return Err((dispatch_cmid, "dispatch provenance store unavailable"));
            }
        }
    }
    hub.record_prompt_dispatch(
        session_id,
        dispatch_cmid.clone(),
        dispatch_origin,
        trigger,
        dispatched_at_ms,
    );
    Ok((dispatch_cmid, dispatched_at_ms))
}

#[allow(clippy::too_many_arguments)]
async fn finish_dispatch_outcome(
    state: &Arc<AppState>,
    outcome: SyncSendOutcome,
    ownership: Arc<crate::core::DispatchOwnership>,
    session_id: String,
    text: String,
    content: Vec<serde_json::Value>,
    cmid: Option<String>,
    origin: Option<crate::prompt_origin::DeliveryOrigin>,
    initiator: Option<crate::prompt_origin::DeliveryOrigin>,
    explicit: bool,
    claim_token: Option<managed_calls::NoticeDispatchToken>,
    notice_rt: Option<(String, Option<Arc<RemoteRuntime>>)>,
) {
    let hub = &state.hub;
    match outcome {
        SyncSendOutcome::Sent { title } => {
            // The message reached the agent: the latest in-place agent change
            // is now part of the conversation, so Settings loses its Back.
            config_revert::clear(hub, &session_id);
            // Successful sends synchronously enter the runtime pending set.
            // Failed sends retain dispatch ownership until requeue completes.
            drop(ownership);
            // A Send now held meanwhile for another row keeps its wait;
            // checked under the section that installs a hold.
            hub.clear_resource_wait_unless_held(&session_id);
            if let Some(t) = title {
                hub.auto_title(&session_id, t);
            }
        }
        other => {
            if let Some(id) = cmid.as_deref() {
                hub.disarm_pending_echo(&session_id, id);
            }
            let transient_disconnect = matches!(
                &other,
                SyncSendOutcome::SendError(error) if is_transient_machine_disconnect(error)
            );
            managed_calls::after_notice_dispatch_error(
                state,
                &session_id,
                cmid.as_deref(),
                claim_token.as_ref(),
            )
            .await;
            finish_deferred_dispatch(
                hub, other, session_id, text, content, cmid, origin, initiator, explicit,
            );
            rewake_notice_if_reconnected(state, transient_disconnect, notice_rt).await;
        }
    }
}

fn notice_preflight_gate(
    state: &Arc<AppState>,
    plugin_key: Option<&(String, String)>,
    session_id: &str,
    cmid: Option<&str>,
    recovery_none: bool,
) -> Option<SyncSendOutcome> {
    if !cmid.is_some_and(|id| id.starts_with(crate::agent_model::CALL_NOTICE_PREFIX)) {
        return None;
    }
    match check_sync_dispatch_gates(state, plugin_key, session_id, cmid, recovery_none) {
        SyncSendOutcome::Ready => None,
        other => Some(other),
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

#[allow(clippy::too_many_arguments)]
fn finish_deferred_dispatch(
    hub: &Hub,
    outcome: SyncSendOutcome,
    session_id: String,
    text: String,
    content: Vec<serde_json::Value>,
    cmid: Option<String>,
    origin: Option<crate::prompt_origin::DeliveryOrigin>,
    initiator: Option<crate::prompt_origin::DeliveryOrigin>,
    explicit: bool,
) {
    match outcome {
        // Both return the prompt for a condition that clears by itself; an
        // explicit one keeps its intent on its row.
        SyncSendOutcome::RequeueByFence => {
            hub.requeue_dispatch(
                &session_id,
                text,
                content,
                cmid,
                origin,
                initiator,
                explicit,
            );
        }
        SyncSendOutcome::WakeForAgent(Ok(())) => {
            hub.requeue_dispatch(
                &session_id,
                text,
                content,
                cmid,
                origin,
                initiator,
                explicit,
            );
            // The launch may already have reported Running.
            hub.context_state_changed(&session_id);
        }
        SyncSendOutcome::WakeForAgent(Err(error)) => {
            retain_failed_dispatch(
                hub, session_id, text, content, cmid, origin, initiator, &error, explicit,
            );
        }
        SyncSendOutcome::HandoffRefusal(wait) => {
            retain_failed_dispatch(
                hub, session_id, text, content, cmid, origin, initiator, &wait, explicit,
            );
        }
        SyncSendOutcome::SendError(error) => {
            tracing::warn!(session = %session_id, error = %error, "queued dispatch failed");
            retain_failed_dispatch(
                hub, session_id, text, content, cmid, origin, initiator, &error, explicit,
            );
        }
        SyncSendOutcome::Ready | SyncSendOutcome::Sent { .. } => {}
    }
}

/// Restore a prompt that left the durable queue but never reached its runtime.
///
/// Reboot convergence: an `explicit` send returned only because a Machine or
/// resource was not ready yet must be delivered once it is, even when the
/// queue is paused or the session crashed while its host rebooted. Its wait
/// is written first, then the row with its intent (and a held Send now's
/// command) in one queue write, so a crash between them never leaves a
/// paused row without the retry that would send it.
#[allow(clippy::too_many_arguments)]
pub fn retain_failed_dispatch(
    hub: &Hub,
    session_id: String,
    text: String,
    content: Vec<serde_json::Value>,
    cmid: Option<String>,
    origin: Option<crate::prompt_origin::DeliveryOrigin>,
    initiator: Option<crate::prompt_origin::DeliveryOrigin>,
    error: &str,
    explicit: bool,
) {
    if let Some(reason) = error.strip_prefix(session_maintenance::WAIT_PREFIX) {
        session_maintenance::wait(hub, &session_id, reason);
        hub.requeue_dispatch(
            &session_id,
            text,
            content,
            cmid,
            origin,
            initiator,
            explicit,
        );
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
        if explicit {
            session_maintenance::wait(
                hub,
                &session_id,
                session_maintenance::MACHINE_CONNECTION_WAIT,
            );
        }
        hub.requeue_dispatch(
            &session_id,
            text,
            content,
            cmid,
            origin,
            initiator,
            explicit,
        );
        return;
    }
    hub.requeue_prompt(&session_id, text, content, cmid, origin, initiator);
    // A refusal that is not a wait ends an earlier retry: the prompt stays
    // queued for explicit intervention instead of failing every two seconds.
    // A Send now held meanwhile for another row keeps the wait it needs.
    hub.clear_resource_wait_unless_held(&session_id);
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
