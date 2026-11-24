//! Durable completion-notice recovery helpers for [`super::RemoteRuntime`].
//!
//! Keeps `UndeliveredPrompt` ACKs behind the FIFO durable requeue + claim
//! release commit (validated against persisted `dispatch_command_id` and
//! recipient `worker_epoch`, rebuilding requeued payloads from the Call ledger,
//! and deduplicated by stable receipt keys), tracks runtime-teardown requeues
//! for graceful shutdown, and isolates execution-recovery send fences from
//! ordinary startup failures.

use super::{
    Arc, CoreCommand, HashMap, Ordering, Result, RuntimeEvent, Shared, StartSession, Status,
    WorkerSnapshot, WorkerState,
};
use crate::agent_model::CALL_NOTICE_PREFIX;
use crate::core::{NoticeRequeueRequest, UndeliveredRequeue};
use crate::store::NoticeDeliveryState;

mod teardown;
pub(super) use teardown::{
    is_ambiguous_prompt_rejection, park_ambiguous_prompt_command,
    park_and_retire_ambiguous_pending, recover_pending_prompts_on_disconnect,
    replacement_worker_superseded_epoch, requeue_rejected_command,
    requeue_unsent_notices_on_disconnect, retire_superseded_prompts_for_snapshot,
    revive_and_try_prompt, set_connected, set_connected_if_active,
};

/// Per-session execution-recovery send fence owned exclusively by
/// [`recover_execution_durable`] (Finding 3 & Finding 8). Never cleared by
/// `fail_config_startup`; released only when the matching replacement worker
/// completes configuration readiness or a newer recovery supersedes it.
#[derive(Debug, Clone)]
pub(super) struct RecoveryFence {
    pub(super) id: u64,
    pub(super) pre_recovery_epoch: String,
    pub(super) state: RecoveryFenceState,
    /// Exact `RecoverExecutionSession` or `reset-*` `StopSession` command ID
    /// currently owning the `AwaitingReplacement` phase (Round 2 #4/#9).
    pub(super) command_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecoveryFenceState {
    /// Durable notice parking is running before issuing the replacement launch.
    Parking,
    /// Replacement launch (`RecoverExecutionSession` or reset) is queued;
    /// awaiting replacement worker `ConfigOptions` on a new worker epoch.
    AwaitingReplacement,
    /// Durable notice parking failed; reject late triggering prompts cleanly
    /// instead of sending them to the stale worker or stranding them in `Starting`.
    Failed,
}

pub(super) struct PendingUndeliveredNotice {
    session_id: String,
    worker_epoch: String,
    runtime_seq: u64,
    command_id: String,
    cmid: String,
    text: String,
    content: Vec<serde_json::Value>,
    error: String,
    admitted: bool,
}

/// Clear in-flight send/attempt/fence tracking once `CommandAck` arrives.
pub(super) fn clear_acked_command(shared: &Shared, command_id: &str) {
    shared.sent.lock().remove(command_id);
    shared.attempted_sends.lock().remove(command_id);
    shared.retransmitted_sends.lock().remove(command_id);
    shared.fenced_commands.lock().remove(command_id);
}

/// Validate session ownership, park-if-ambiguous, remove from `pending`, clear
/// steering waiters + tracking sets, and reconcile idle guard ONLY if a live
/// replacement worker epoch superseded the prompt's bound epoch.
/// Returns `None` when `command_id` belongs to a different session.
pub(super) fn take_acked_command(
    shared: &Arc<Shared>,
    session_id: &str,
    command_id: &str,
    accepted: bool,
    reason: Option<&str>,
) -> Option<(Option<CoreCommand>, bool)> {
    let was_retransmitted = shared.retransmitted_sends.lock().contains(command_id);
    let ambiguous_entry = {
        let pending = shared.pending.lock();
        if pending.get(command_id).is_some_and(|command| {
            command
                .session_id()
                .is_some_and(|expected| expected != session_id)
        }) {
            tracing::warn!(%command_id, "ignoring mismatched Machine command acknowledgement");
            return None;
        }
        let cmd = pending.get(command_id);
        if !accepted && is_ambiguous_prompt_rejection(cmd, was_retransmitted, reason) {
            pending
                .version(command_id)
                .and_then(|version| cmd.cloned().map(|command| (version, command)))
        } else {
            None
        }
    };
    if let Some((version, cmd)) = ambiguous_entry {
        let message = reason
            .unwrap_or("prompt delivery across worker change is unresolved; retained as draft");
        let replacement = replacement_worker_superseded_epoch(shared, session_id, &cmd);
        park_and_retire_ambiguous_pending(
            shared,
            session_id,
            command_id,
            version,
            cmd.clone(),
            message,
            replacement.as_ref(),
        );
        return Some((Some(cmd), true));
    }
    let command = shared.pending.lock().remove(command_id);
    if !accepted
        && let Some(CoreCommand::Steer { cmid, .. } | CoreCommand::CancelTurn { cmid, .. }) =
            &command
        && let Some(waiter) = shared
            .steering
            .lock()
            .remove(&(session_id.to_owned(), cmid.clone()))
    {
        let _ = waiter.send("unknown".into());
    }
    clear_acked_command(shared, command_id);
    Some((command, false))
}

/// Transition an active `AwaitingReplacement` recovery fence to `Failed` when
/// the Machine rejects the exact recovery command (`RecoverExecutionSession` or
/// `reset-*` `StopSession`), keeping the stale worker fenced while extracting
/// only pending `CoreCommand::Prompt` commands (Round 2 #4).
pub(super) fn fail_rejected_recovery_command(
    shared: &Shared,
    session_id: &str,
    command_id: &str,
    reason: &str,
) -> bool {
    let rejected_prompts = {
        let mut startups = shared.config_startups.lock();
        let mut fences = shared.recovery_fences.lock();
        let Some(fence) = fences.get_mut(session_id) else {
            return false;
        };
        if fence.state != RecoveryFenceState::AwaitingReplacement
            || fence.command_id.as_deref() != Some(command_id)
        {
            return false;
        }
        fence.state = RecoveryFenceState::Failed;
        startups.remove(session_id);
        let mut pending = shared.pending.lock();
        let mut sent = shared.sent.lock();
        let mut attempted = shared.attempted_sends.lock();
        let mut fenced_cmds = shared.fenced_commands.lock();
        let prompt_keys: Vec<String> = pending
            .iter()
            .filter(|(_, cmd)| {
                matches!(cmd, CoreCommand::Prompt { session_id: owner, .. } if owner == session_id)
            })
            .map(|(key, _)| key.clone())
            .collect();
        let mut extracted = Vec::with_capacity(prompt_keys.len());
        for key in prompt_keys {
            sent.remove(&key);
            attempted.remove(&key);
            fenced_cmds.remove(&key);
            if let Some(cmd) = pending.remove(&key) {
                extracted.push(cmd);
            }
        }
        extracted
    };
    for cmd in rejected_prompts {
        requeue_rejected_command(shared, session_id, Some(cmd), Some(reason.to_owned()));
    }
    true
}

/// If a session currently holds a recovery fence (including `Failed` after an
/// aborted or rejected recovery), supersede it with a fresh `AwaitingReplacement`
/// fence bound to the explicit reload/reset `command_id` so the replacement
/// worker's `ConfigOptions` clears the fence once ready (Round 2 #9).
pub(super) fn refresh_recovery_fence_for_reset(
    shared: &Shared,
    session_id: &str,
    command_id: &str,
) {
    let pre_recovery_epoch = shared
        .workers
        .lock()
        .get(session_id)
        .map(|worker| worker.worker_epoch.clone())
        .unwrap_or_default();
    let mut fences = shared.recovery_fences.lock();
    if fences.contains_key(session_id) {
        let id = shared
            .recovery_fence_counter
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        fences.insert(
            session_id.to_owned(),
            RecoveryFence {
                id,
                pre_recovery_epoch,
                state: RecoveryFenceState::AwaitingReplacement,
                command_id: Some(command_id.to_owned()),
            },
        );
    }
}

/// Handle a negative `CommandAck` (`accepted == false`) without inflating
/// `handle_frame`: fences rejected recovery commands, fails config startup when
/// appropriate, triggers workspace replacement reset, notes missing provider
/// generations, and requeues rejected commands through the notice-aware path.
pub(super) fn handle_rejected_ack(
    shared: &Shared,
    session_id: &str,
    command_id: &str,
    command: Option<CoreCommand>,
    reset_stop: bool,
    reason: Option<String>,
) {
    let rejected_recovery = reset_stop
        && fail_rejected_recovery_command(
            shared,
            session_id,
            command_id,
            reason.as_deref().unwrap_or("worker startup rejected"),
        );
    if !rejected_recovery
        && (matches!(&command, Some(CoreCommand::EnsureSession { .. })) || reset_stop)
    {
        super::fail_config_startup(
            shared,
            session_id,
            reason.as_deref().unwrap_or("worker startup rejected"),
        );
    }
    if reason
        .as_deref()
        .is_some_and(|message| message.starts_with("session workspace was replaced or removed:"))
    {
        super::reset_after_workspace_replacement(shared, session_id);
    }
    if reason
        .as_deref()
        .is_some_and(crate::generation_retention::is_not_installed_detail)
    {
        shared.hub.note_provider_generation_missing(session_id);
    }
    if is_ambiguous_prompt_rejection(command.as_ref(), false, reason.as_deref()) {
        let message = reason.unwrap_or_else(|| {
            "prompt delivery across worker change is unresolved; retained as draft".to_owned()
        });
        if let Some(cmd) = command {
            park_ambiguous_prompt_command(shared, session_id, cmd, &message);
        } else {
            shared
                .hub
                .broadcast_error(Some(session_id.to_owned()), message);
        }
    } else {
        requeue_rejected_command(shared, session_id, command, reason);
    }
    shared.in_flight_prompts.lock().retire_command(command_id);
}

/// Settle a `call-notice:` `UndeliveredPrompt` returned either from an admitted
/// worker event (`admitted == true`) or from an earlier worker epoch whose
/// snapshot was already replaced (`admitted == false`, requiring durable
/// `dispatch_worker_epoch` proof).
pub(super) async fn settle_event_undelivered(
    shared: &Shared,
    key: &(String, String),
    runtime_seq: u64,
    event: &RuntimeEvent,
    admitted: bool,
    pending: Option<PendingUndeliveredNotice>,
) -> Result<()> {
    let pending = if admitted {
        pending
    } else {
        suppressed_undelivered(key, runtime_seq, event, false)
    };
    settle_undelivered_notice(shared, pending).await
}

/// Whether a worker snapshot represents a live, non-draining recipient that
/// the Machine broker can actually route a prompt to (Round 3 #6).
fn worker_can_accept_notice(worker: &WorkerSnapshot) -> bool {
    worker.has_connected_owner()
        && !worker.drain_requested
        && matches!(
            worker.state,
            WorkerState::Starting | WorkerState::Running | WorkerState::Busy
        )
}

/// Record the live recipient worker epoch on `managed_call_notices` right before
/// writing a `call-notice:` `CoreCommand::Prompt` to the Machine broker so a
/// delayed `UndeliveredPrompt` from that exact worker remains verifiable even
/// after a replacement worker is adopted (Finding 2).
pub(super) async fn record_dispatch_worker_epoch_if_notice(shared: &Shared, command: &CoreCommand) {
    let CoreCommand::Prompt {
        session_id,
        command_id,
        cmid: Some(cid),
        ..
    } = command
    else {
        return;
    };
    if !cid.starts_with(CALL_NOTICE_PREFIX) {
        return;
    }
    if shared.resetting.lock().contains(session_id) {
        return;
    }
    let Some(worker_epoch) = shared
        .workers
        .lock()
        .get(session_id)
        .filter(|worker| worker_can_accept_notice(worker))
        .map(|worker| worker.worker_epoch.clone())
    else {
        return;
    };
    let Some((_, store)) = shared.native_store.lock().clone() else {
        return;
    };
    let _ = store
        .record_call_notice_dispatch_worker_epoch(session_id, cid, command_id, &worker_epoch)
        .await;
}

/// When a cold-revived worker's first admitted snapshot arrives while a
/// `call-notice:` prompt has already been handed off to the broker without a
/// prior worker epoch, record that admitted epoch on the active claim ONLY if
/// the worker is non-draining, non-terminal, and not currently resetting (Round 3 #6).
pub(super) fn spawn_record_snapshot_worker_epoch_if_notice(
    shared: &Shared,
    worker: &WorkerSnapshot,
) {
    if !worker_can_accept_notice(worker) || shared.resetting.lock().contains(&worker.session_id) {
        return;
    }
    let session_id = worker.session_id.as_str();
    let worker_epoch = worker.worker_epoch.as_str();
    let active_notices: Vec<(String, String)> = {
        let pending = shared.pending.lock();
        let attempted = shared.attempted_sends.lock();
        pending
            .iter()
            .filter_map(|(key, cmd)| {
                if !attempted.contains(key) {
                    return None;
                }
                match cmd {
                    CoreCommand::Prompt {
                        session_id: owner,
                        command_id,
                        cmid: Some(cid),
                        ..
                    } if owner == session_id && cid.starts_with(CALL_NOTICE_PREFIX) => {
                        Some((cid.clone(), command_id.clone()))
                    }
                    _ => None,
                }
            })
            .collect()
    };
    if active_notices.is_empty() {
        return;
    }
    let Some((_, store)) = shared.native_store.lock().clone() else {
        return;
    };
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let session_id = session_id.to_owned();
    let worker_epoch = worker_epoch.to_owned();
    handle.spawn(async move {
        for (cid, command_id) in active_notices {
            let _ = store
                .record_call_notice_dispatch_worker_epoch(
                    &session_id,
                    &cid,
                    &command_id,
                    &worker_epoch,
                )
                .await;
        }
    });
}

/// Project a `WorkerEvent`'s final branch (auto-permission or diverted Hub projection).
/// Defers `UndeliveredPrompt` for a `call-notice:` to [`settle_undelivered_notice`]
/// so durable validation, payload rebuild, deduplication, and FIFO store admission
/// run before mutating Hub or sending `Ack`.
pub(super) fn project_worker_outcome(
    shared: &Shared,
    key: &(String, String),
    session_id: &str,
    runtime_seq: u64,
    event: RuntimeEvent,
    idle_guard: Option<u64>,
    auto_permission: Option<(String, String)>,
) -> Option<PendingUndeliveredNotice> {
    if let Some((request_id, option_id)) = auto_permission {
        tracing::info!(
            session = %session_id,
            %request_id,
            %option_id,
            "auto-approving Codex full-access permission at runtime boundary"
        );
        super::queue_permission(shared, session_id, request_id, option_id);
        return None;
    }
    super::mark_projected_provider_auth_expired(shared, session_id, &event);
    super::prompt_tracking::observe_worker_event(shared, session_id, &event);
    let deferred =
        super::divert(&shared.hub, key, runtime_seq, event).and_then(|event| match event {
            RuntimeEvent::UndeliveredPrompt {
                command_id,
                cmid: Some(cid),
                content,
                error,
            } if cid.starts_with(CALL_NOTICE_PREFIX) => {
                shared.in_flight_prompts.lock().retire_command(&command_id);
                let text = content
                    .iter()
                    .filter_map(|value| value.get("text").and_then(serde_json::Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                Some(PendingUndeliveredNotice {
                    session_id: session_id.to_owned(),
                    worker_epoch: key.1.clone(),
                    runtime_seq,
                    command_id,
                    cmid: cid,
                    text,
                    content,
                    error,
                    admitted: true,
                })
            }
            RuntimeEvent::UndeliveredPrompt {
                command_id,
                cmid,
                content,
                error,
            } => {
                super::prompt_tracking::handle_undelivered_prompt(
                    shared,
                    session_id,
                    &command_id,
                    cmid.as_deref(),
                    content,
                    error,
                );
                None
            }
            other => {
                super::apply_event(&shared.hub, session_id, other);
                None
            }
        });
    super::reconcile_idle_guard(shared, session_id, idle_guard);
    deferred
}

/// Extract a `call-notice:` `UndeliveredPrompt` when transcript projection was
/// suppressed (e.g. during a reset) OR when an earlier worker epoch delivers a
/// delayed receipt after a replacement worker was adopted. `admitted` records
/// whether `native_admission::event` admitted this worker epoch; unadmitted
/// receipts MUST additionally match the persisted `dispatch_worker_epoch`.
pub(super) fn suppressed_undelivered(
    key: &(String, String),
    runtime_seq: u64,
    event: &RuntimeEvent,
    admitted: bool,
) -> Option<PendingUndeliveredNotice> {
    let RuntimeEvent::UndeliveredPrompt {
        command_id,
        cmid: Some(cid),
        content,
        error,
    } = event
    else {
        return None;
    };
    if !cid.starts_with(CALL_NOTICE_PREFIX) {
        return None;
    }
    let text = content
        .iter()
        .filter_map(|value| value.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    Some(PendingUndeliveredNotice {
        session_id: key.0.clone(),
        worker_epoch: key.1.clone(),
        runtime_seq,
        command_id: command_id.clone(),
        cmid: cid.clone(),
        text,
        content: content.clone(),
        error: error.clone(),
        admitted,
    })
}

/// Rebuild the canonical completion-notice payload from the authoritative Call
/// ledger record rather than trusting worker-echoed content (Finding 2).
async fn canonical_notice_payload(
    store: &crate::store::Store,
    session_id: &str,
    cmid: &str,
) -> Result<Option<(String, Vec<serde_json::Value>)>> {
    let Some(call_id) = cmid.strip_prefix(CALL_NOTICE_PREFIX) else {
        return Ok(None);
    };
    let Some(record) = store.managed_call(session_id, call_id).await? else {
        return Ok(None);
    };
    if !record.state.terminal() || !crate::managed_calls::notice::wants_notice(&record) {
        return Ok(None);
    }
    let text = crate::managed_calls::notice::call_notice(&record);
    let content = vec![serde_json::json!({ "type": "text", "text": text })];
    Ok(Some((text, content)))
}

async fn reconcile_already_committed_undelivered(
    shared: &Shared,
    store: &crate::store::Store,
    pending: &PendingUndeliveredNotice,
    delivery: &crate::store::CallNoticeDelivery,
    receipt_key: &str,
    legacy_unknown_cmd: bool,
) -> Result<()> {
    if delivery.state == NoticeDeliveryState::Enqueued
        && delivery.dispatch_started_at_ms.is_none()
        && let Some((text, content)) =
            canonical_notice_payload(store, &pending.session_id, &pending.cmid).await?
    {
        let replay_cmd = (!legacy_unknown_cmd).then_some(pending.command_id.as_str());
        match shared.hub.preview_and_send_reconcile_notice_requeue(
            &pending.session_id,
            text,
            content,
            &pending.cmid,
            replay_cmd,
            receipt_key,
        ) {
            UndeliveredRequeue::NoStore => return Ok(()),
            UndeliveredRequeue::Rejected(reason) => anyhow::bail!(
                "committed notice requeue reconciliation rejected for {}: {reason}",
                pending.session_id
            ),
            UndeliveredRequeue::Pending(mut preview, rx) => match rx.await {
                Ok(Ok(true)) => {
                    shared.hub.commit_undelivered_notice_requeue(&mut preview);
                    return Ok(());
                }
                Ok(Ok(false)) => {
                    shared
                        .hub
                        .abort_undelivered_notice_requeue(&mut preview, true);
                    return Ok(());
                }
                Ok(Err(error)) => {
                    shared
                        .hub
                        .abort_undelivered_notice_requeue(&mut preview, false);
                    anyhow::bail!(
                        "durable committed notice requeue reconciliation failed for {}: {error}",
                        pending.session_id
                    )
                }
                Err(_) => {
                    shared
                        .hub
                        .abort_undelivered_notice_requeue(&mut preview, false);
                    anyhow::bail!(
                        "store writer closed before confirming committed notice requeue reconciliation for {}",
                        pending.session_id
                    )
                }
            },
        }
    }
    shared
        .hub
        .mark_undelivered_key_seen(&pending.session_id, &pending.cmid, receipt_key);
    Ok(())
}

/// Validate a worker `UndeliveredPrompt` receipt against the persisted active
/// dispatch claim (`dispatch_started_at_ms`, `dispatch_command_id`, and
/// `dispatch_worker_epoch`). Returns `Some((expected_command_id, require_epoch_match))`
/// when authentic, or `None` when stale/unauthentic.
fn match_undelivered_claim(
    shared: &Shared,
    pending: &PendingUndeliveredNotice,
    delivery: &crate::store::CallNoticeDelivery,
    legacy_unknown_cmd: bool,
) -> Option<(Option<String>, bool)> {
    let reopenable = delivery.state == NoticeDeliveryState::Enqueued
        || (delivery.state == NoticeDeliveryState::Unknown
            && delivery.reason.as_deref() == Some("dispatch_unconfirmed"));
    if !reopenable || delivery.dispatch_started_at_ms.is_none() {
        return None;
    }
    let epoch_is_current = shared
        .workers
        .lock()
        .get(&pending.session_id)
        .is_some_and(|worker| worker.worker_epoch == pending.worker_epoch);
    let (epoch_matches, require_epoch_match) = match delivery.dispatch_worker_epoch.as_deref() {
        Some(persisted_epoch) => (persisted_epoch == pending.worker_epoch, true),
        None => (pending.admitted && epoch_is_current, false),
    };
    let (cmd_matches, expected_cmd) = if legacy_unknown_cmd {
        (
            pending.admitted && epoch_is_current,
            delivery.dispatch_command_id.clone(),
        )
    } else {
        (
            delivery.dispatch_command_id.as_deref() == Some(pending.command_id.as_str()),
            Some(pending.command_id.clone()),
        )
    };
    (cmd_matches && epoch_matches).then_some((expected_cmd, require_epoch_match))
}

/// Durably validate, deduplicate, requeue (rebuilt from the Call ledger), and
/// release an authoritative worker `UndeliveredPrompt` for a `call-notice:`
/// BEFORE advancing `highwaters` and sending `Frame::Ack`.
#[allow(clippy::too_many_lines)]
pub(super) async fn settle_undelivered_notice(
    shared: &Shared,
    pending: Option<PendingUndeliveredNotice>,
) -> Result<()> {
    let Some(pending) = pending else {
        return Ok(());
    };
    let _commit_guard = shared.hub.pending_commit_lock(&pending.session_id).await;
    let receipt_key = format!("worker:{}:{}", pending.worker_epoch, pending.runtime_seq);
    if shared
        .hub
        .is_undelivered_key_seen(&pending.session_id, &pending.cmid, &receipt_key)
    {
        return Ok(());
    }
    let store = shared
        .native_store
        .lock()
        .as_ref()
        .map(|(_, store)| store.clone());
    let legacy_unknown_cmd = pending.command_id == "unknown-command";
    let (requeue_text, requeue_content, expected_command_id, require_worker_epoch_match) =
        if let Some(store) = store.as_ref() {
            let Some(delivery) = store
                .call_notice_by_parent_completion_id(&pending.session_id, &pending.cmid)
                .await?
            else {
                return Ok(());
            };
            if delivery.last_undelivered_key.as_deref() == Some(receipt_key.as_str()) {
                return reconcile_already_committed_undelivered(
                    shared,
                    store,
                    &pending,
                    &delivery,
                    &receipt_key,
                    legacy_unknown_cmd,
                )
                .await;
            }
            let Some((expected_cmd, require_epoch_match)) =
                match_undelivered_claim(shared, &pending, &delivery, legacy_unknown_cmd)
            else {
                if pending.admitted {
                    shared.hub.mark_undelivered_key_seen(
                        &pending.session_id,
                        &pending.cmid,
                        &receipt_key,
                    );
                }
                return Ok(());
            };
            let Some((text, content)) =
                canonical_notice_payload(store, &pending.session_id, &pending.cmid).await?
            else {
                return Ok(());
            };
            (text, content, expected_cmd, require_epoch_match)
        } else {
            if !pending.admitted {
                return Ok(());
            }
            (
                pending.text,
                pending.content,
                (!legacy_unknown_cmd).then(|| pending.command_id.clone()),
                false,
            )
        };
    let req = NoticeRequeueRequest {
        session_id: pending.session_id.clone(),
        text: requeue_text,
        content: requeue_content,
        cmid: pending.cmid.clone(),
        expected_command_id,
        expected_worker_epoch: Some(pending.worker_epoch.clone()),
        require_worker_epoch_match,
        receipt_key,
        require_claim_release: true,
    };
    match shared.hub.preview_and_send_undelivered_notice_requeue(req) {
        UndeliveredRequeue::NoStore => {
            shared
                .hub
                .broadcast_error(Some(pending.session_id), pending.error);
            Ok(())
        }
        UndeliveredRequeue::Rejected(reason) => anyhow::bail!(
            "undelivered notice requeue rejected for {}: {reason}",
            pending.session_id
        ),
        UndeliveredRequeue::Pending(mut preview, rx) => match rx.await {
            Ok(Ok(true)) => {
                shared.hub.commit_undelivered_notice_requeue(&mut preview);
                shared
                    .hub
                    .broadcast_error(Some(pending.session_id), pending.error);
                Ok(())
            }
            Ok(Ok(false)) => {
                // SQL rejected as duplicate or already-settled/reclaimed without
                // touching `sessions.queue`; keep Hub state untouched and mark seen.
                shared
                    .hub
                    .abort_undelivered_notice_requeue(&mut preview, true);
                Ok(())
            }
            Ok(Err(error)) => {
                shared
                    .hub
                    .abort_undelivered_notice_requeue(&mut preview, false);
                anyhow::bail!(
                    "durable undelivered prompt requeue failed for {}: {error}",
                    pending.session_id
                )
            }
            Err(_) => {
                shared
                    .hub
                    .abort_undelivered_notice_requeue(&mut preview, false);
                anyhow::bail!(
                    "store writer closed before confirming undelivered prompt requeue for {}",
                    pending.session_id
                )
            }
        },
    }
}

/// Park pending prompts durably before issuing `RecoverExecutionSession` (when
/// `intent` is `Some`) or retrying recovery via `reset` (when `intent` is `None`).
#[allow(clippy::too_many_lines)]
pub(super) fn recover_execution_durable(
    shared: &Arc<Shared>,
    mut session: StartSession,
    intent: Option<crate::execution_environment::RecoveryV1>,
) -> std::result::Result<(), String> {
    if intent.is_some() {
        session.generation.clone_from(&shared.desired_generation);
    }
    let id = session.session_id.clone();
    let pre_recovery_epoch = shared
        .workers
        .lock()
        .get(&id)
        .map(|worker| worker.worker_epoch.clone())
        .unwrap_or_default();
    let fence_id = shared
        .recovery_fence_counter
        .fetch_add(1, Ordering::Relaxed)
        + 1;
    let (pre_recovery_keys, notice_prompts, other_prompts) =
        snapshot_pre_recovery_pending(shared, &id);
    if !notice_prompts.is_empty() && shared.hub.has_store_writer() {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            let msg = "Execution recovery aborted: async runtime unavailable to park pending completion notice";
            shared.hub.broadcast_error(Some(id), msg.into());
            return Err(msg.into());
        };
        // Install a dedicated recovery send fence synchronously BEFORE returning
        // (Finding 3). `fail_config_startup` never clears `recovery_fences`, so
        // a negative ACK for an earlier `EnsureSession` cannot reopen sends.
        super::begin_reset(shared, &id);
        shared
            .active_reset_commands
            .lock()
            .insert(id.clone(), format!("parking:{fence_id}"));
        shared.config_startups.lock().insert(id.clone());
        shared.recovery_fences.lock().insert(
            id.clone(),
            RecoveryFence {
                id: fence_id,
                pre_recovery_epoch,
                state: RecoveryFenceState::Parking,
                command_id: None,
            },
        );
        shared.config_sync_epochs.lock().remove(&id);
        let shared = Arc::clone(shared);
        handle.spawn(async move {
            let commit_guard = shared.hub.pending_commit_lock(&id).await;
            // Round 3 #2: if a newer recovery or reload already superseded this
            // fence before parking began, exit without creating duplicate drafts.
            if shared
                .recovery_fences
                .lock()
                .get(&id)
                .is_none_or(|f| f.id != fence_id)
            {
                return;
            }
            let Some((mut preview, rx)) = shared
                .hub
                .preview_park_recovery_notices(&id, &notice_prompts)
            else {
                drop(commit_guard);
                abort_failed_execution_recovery(
                    &shared,
                    &id,
                    fence_id,
                    &pre_recovery_keys,
                    "Execution recovery aborted: durable store writer unavailable to park pending completion notice",
                )
                .await;
                return;
            };
            match rx.await {
                Ok(Ok(true)) => {
                    shared.hub.commit_park_recovery_notices(&mut preview);
                    drop(commit_guard);
                    retire_parked_recovery_and_launch(
                        &shared,
                        &id,
                        fence_id,
                        &pre_recovery_keys,
                        other_prompts,
                        session,
                        intent,
                    );
                }
                Ok(Ok(false) | Err(_)) => {
                    shared.hub.abort_park_recovery_notices(&mut preview);
                    drop(commit_guard);
                    abort_failed_execution_recovery(
                        &shared,
                        &id,
                        fence_id,
                        &pre_recovery_keys,
                        "Execution recovery aborted: could not durably park pending completion notice",
                    )
                    .await;
                }
                Err(_) => {
                    shared.hub.abort_park_recovery_notices(&mut preview);
                    drop(commit_guard);
                    abort_failed_execution_recovery(
                        &shared,
                        &id,
                        fence_id,
                        &pre_recovery_keys,
                        "Execution recovery aborted: store writer closed before parking pending completion notice",
                    )
                    .await;
                }
            }
        });
        return Ok(());
    }
    park_recovery_prompts_shared(shared, &id);
    finish_recovery_sync(shared, session, intent, fence_id, pre_recovery_epoch);
    Ok(())
}

/// Retire every pre-recovery `(key, version)` that parking durably committed as
/// drafts + `execution_recovery_parked`, then launch replacement recovery if
/// `fence_id` is still the active fence owner.
fn retire_parked_recovery_and_launch(
    shared: &Shared,
    id: &str,
    fence_id: u64,
    pre_recovery_keys: &HashMap<String, u64>,
    other_prompts: Vec<(String, Vec<serde_json::Value>, Option<String>)>,
    session: StartSession,
    intent: Option<crate::execution_environment::RecoveryV1>,
) {
    let still_owner = {
        let mut recovery_fences = shared.recovery_fences.lock();
        let mut pending = shared.pending.lock();
        let mut sent = shared.sent.lock();
        let mut attempted = shared.attempted_sends.lock();
        let mut fenced = shared.fenced_commands.lock();
        for (key, expected_version) in pre_recovery_keys {
            if pending.version(key) == Some(*expected_version) {
                pending.remove(key);
                sent.remove(key);
                attempted.remove(key);
                fenced.remove(key);
            }
        }
        if let Some(fence) = recovery_fences.get_mut(id)
            && fence.id == fence_id
        {
            fence.state = RecoveryFenceState::AwaitingReplacement;
            true
        } else {
            false
        }
    };
    for (text, content, cmid) in other_prompts {
        shared.hub.park_recovery_draft(id, text, content, cmid);
    }
    shared.hub.broadcast_error(
        Some(id.to_owned()),
        "Execution recovery retained an unacknowledged prompt as a draft; inspect effects before resending"
            .into(),
    );
    if still_owner {
        finish_recovery_after_fence(shared, session, intent, fence_id);
    }
}

/// Clean up after durable notice parking failed during execution recovery:
/// - Atomically publish `RecoveryFenceState::Failed` under `recovery_fences -> pending`
///   so any late triggering prompt arriving via `try_prompt_traced` is rejected
///   synchronously rather than stranded behind a fence in `Starting` (Finding 8).
/// - Keep every uncertain pre-recovery command fenced in `fenced_commands` so
///   it is NEVER replayed to the old worker (even after reconnect).
/// - Extract ONLY post-recovery `CoreCommand::Prompt` commands (leaving unrelated
///   later control commands like `Cancel` or `Permission` under their owner!).
/// - Restore a `Starting` session back to `Interrupted` so execution reconciliation
///   can retry it cleanly.
#[allow(clippy::too_many_lines)]
async fn abort_failed_execution_recovery(
    shared: &Shared,
    session_id: &str,
    fence_id: u64,
    pre_recovery_keys: &HashMap<String, u64>,
    message: &str,
) {
    let post_recovery_prompts = {
        let mut recovery_fences = shared.recovery_fences.lock();
        let mut pending = shared.pending.lock();
        let mut sent = shared.sent.lock();
        let mut attempted = shared.attempted_sends.lock();
        let mut fenced = shared.fenced_commands.lock();
        // Round 2 #8: if a newer recovery or reload already superseded this
        // fence, return immediately without touching the newer recovery's
        // pending prompts, fenced commands, or session status.
        let Some(fence) = recovery_fences.get_mut(session_id) else {
            return;
        };
        if fence.id != fence_id {
            return;
        }
        fence.state = RecoveryFenceState::Failed;
        let parking_owner = format!("parking:{fence_id}");
        {
            let mut owners = shared.active_reset_commands.lock();
            if owners.get(session_id) == Some(&parking_owner) {
                owners.remove(session_id);
            }
        }
        let mut to_remove = Vec::new();
        for (key, version, cmd) in pending.iter_with_versions() {
            if cmd.session_id() != Some(session_id) {
                continue;
            }
            if pre_recovery_keys.get(key).is_some_and(|v| *v == version) {
                fenced.insert(key.clone());
            } else if matches!(cmd, CoreCommand::Prompt { .. }) {
                to_remove.push(key.clone());
            }
        }
        let mut prompts = Vec::new();
        for key in to_remove {
            sent.remove(&key);
            attempted.remove(&key);
            fenced.remove(&key);
            if let Some(CoreCommand::Prompt {
                command_id,
                content,
                cmid,
                ..
            }) = pending.remove(&key)
            {
                let text = content
                    .iter()
                    .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                prompts.push((command_id, text, content, cmid));
            }
        }
        prompts
    };
    if shared.hub.status(session_id) == Some(Status::Starting) {
        shared
            .hub
            .set_status(session_id, Status::Interrupted, Some(message.to_owned()));
    }
    for (command_id, text, content, cmid) in post_recovery_prompts {
        if let Some(cid) = cmid
            .as_deref()
            .filter(|id| id.starts_with(CALL_NOTICE_PREFIX))
            .map(str::to_owned)
        {
            let _commit_guard = shared.hub.pending_commit_lock(session_id).await;
            let req = NoticeRequeueRequest {
                session_id: session_id.to_owned(),
                text,
                content,
                cmid: cid,
                expected_command_id: Some(command_id.clone()),
                expected_worker_epoch: None,
                require_worker_epoch_match: false,
                receipt_key: format!("recovery-abort:{command_id}"),
                require_claim_release: true,
            };
            match shared
                .hub
                .preview_and_send_undelivered_notice_requeue(req.clone())
            {
                UndeliveredRequeue::NoStore => {}
                UndeliveredRequeue::Pending(mut preview, rx) => match rx.await {
                    Ok(Ok(true)) => shared.hub.commit_undelivered_notice_requeue(&mut preview),
                    Ok(Ok(false)) => shared
                        .hub
                        .abort_undelivered_notice_requeue(&mut preview, false),
                    Ok(Err(_)) | Err(_) => {
                        shared.hub.fallback_undelivered_notice_requeue(&mut preview);
                    }
                },
                UndeliveredRequeue::Rejected(_) => {
                    shared
                        .hub
                        .fallback_undelivered_notice_requeue_rejected(&req);
                }
            }
        } else {
            let (origin, initiator) = super::prompt_tracking::provenance_from_hub_or_reserved(
                &shared.hub,
                session_id,
                cmid.as_deref(),
            );
            shared
                .hub
                .requeue_prompt(session_id, text, content, cmid, origin, initiator);
        }
    }
    shared
        .hub
        .broadcast_error(Some(session_id.to_owned()), message.to_owned());
}

#[allow(clippy::type_complexity)]
fn snapshot_pre_recovery_pending(
    shared: &Shared,
    session_id: &str,
) -> (
    HashMap<String, u64>,
    Vec<(String, Vec<serde_json::Value>, String)>,
    Vec<(String, Vec<serde_json::Value>, Option<String>)>,
) {
    let pending = shared.pending.lock();
    let mut keys = HashMap::new();
    let mut notices = Vec::new();
    let mut others = Vec::new();
    for (key, version, command) in pending.iter_with_versions() {
        if command.session_id() != Some(session_id) {
            continue;
        }
        keys.insert(key.clone(), version);
        if let CoreCommand::Prompt { content, cmid, .. } = command {
            let text = content
                .iter()
                .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            if let Some(cid) = cmid
                .as_deref()
                .filter(|id| id.starts_with(CALL_NOTICE_PREFIX))
            {
                notices.push((text, content.clone(), cid.to_owned()));
            } else {
                others.push((text, content.clone(), cmid.clone()));
            }
        }
    }
    (keys, notices, others)
}

fn park_recovery_prompts_shared(shared: &Shared, id: &str) {
    for command in super::take_pending_prompts(shared, id) {
        if let CoreCommand::Prompt { content, cmid, .. } = command {
            let text = content
                .iter()
                .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            shared.hub.park_recovery_draft(id, text, content, cmid);
            shared.hub.broadcast_error(
                Some(id.to_owned()),
                "Execution recovery retained an unacknowledged prompt as a draft; inspect effects before resending"
                    .into(),
            );
        }
    }
}

fn finish_recovery_sync(
    shared: &Shared,
    session: StartSession,
    intent: Option<crate::execution_environment::RecoveryV1>,
    fence_id: u64,
    pre_recovery_epoch: String,
) {
    let id = session.session_id.clone();
    shared.recovery_fences.lock().insert(
        id.clone(),
        RecoveryFence {
            id: fence_id,
            pre_recovery_epoch,
            state: RecoveryFenceState::AwaitingReplacement,
            command_id: None,
        },
    );
    if let Some(intent) = intent {
        super::begin_reset(shared, &id);
        shared.config_startups.lock().insert(id.clone());
        shared.config_sync_epochs.lock().remove(&id);
        super::discard_pending_config(shared, &id);
        shared
            .pending
            .lock()
            .retain(|_, command| command.session_id() != Some(id.as_str()));
        shared
            .declarations
            .lock()
            .insert(id.clone(), session.clone());
        let command_id = next_id_shared(shared, "execution-recover");
        shared
            .active_reset_commands
            .lock()
            .insert(id.clone(), command_id.clone());
        if let Some(fence) = shared.recovery_fences.lock().get_mut(&id)
            && fence.id == fence_id
        {
            fence.command_id = Some(command_id.clone());
        }
        queue_shared(
            shared,
            command_id.clone(),
            CoreCommand::RecoverExecutionSession {
                session: Box::new(session),
                intent: Box::new(intent),
                command_id,
            },
        );
    } else {
        reset_shared(shared, session, Some(fence_id));
    }
}

/// Complete execution recovery after `begin_reset` + `recovery_fences` were
/// already installed synchronously and only `pre_recovery_keys` were removed,
/// preserving any post-recovery triggering prompt held in `shared.pending`.
fn finish_recovery_after_fence(
    shared: &Shared,
    session: StartSession,
    intent: Option<crate::execution_environment::RecoveryV1>,
    fence_id: u64,
) {
    if let Some(intent) = intent {
        let id = session.session_id.clone();
        super::discard_pending_config(shared, &id);
        shared
            .declarations
            .lock()
            .insert(id.clone(), session.clone());
        let command_id = next_id_shared(shared, "execution-recover");
        shared
            .active_reset_commands
            .lock()
            .insert(id.clone(), command_id.clone());
        if let Some(fence) = shared.recovery_fences.lock().get_mut(&id)
            && fence.id == fence_id
        {
            fence.command_id = Some(command_id.clone());
        }
        queue_shared(
            shared,
            command_id.clone(),
            CoreCommand::RecoverExecutionSession {
                session: Box::new(session),
                intent: Box::new(intent),
                command_id,
            },
        );
    } else {
        reset_shared(shared, session, Some(fence_id));
    }
}

fn queue_shared(shared: &Shared, key: String, command: CoreCommand) {
    super::insert_pending(shared, key, command);
    let _ = shared.notify.send(());
}

fn next_id_shared(shared: &Shared, prefix: &str) -> String {
    let n = shared.command_counter.fetch_add(1, Ordering::Relaxed) + 1;
    format!("{prefix}-{n}")
}

fn reset_shared(shared: &Shared, mut session: StartSession, fence_id: Option<u64>) {
    session.adopt_only = true;
    if session.generation.is_empty() {
        session.generation.clone_from(&shared.desired_generation);
    }
    let session_id = session.session_id.clone();
    let same_provider_release = shared
        .workers
        .lock()
        .get(&session_id)
        .is_some_and(|worker| {
            worker.launch.as_ref().is_some_and(|previous| {
                previous.provider_generation_digest == session.provider_generation_digest
                    && previous.provider_version == session.provider_version
            })
        });
    super::begin_reset(shared, &session_id);
    shared.config_startups.lock().insert(session_id.clone());
    shared.config_sync_epochs.lock().remove(&session_id);
    if !same_provider_release {
        super::discard_pending_config(shared, &session_id);
    }
    shared
        .declarations
        .lock()
        .insert(session_id.clone(), session.clone());
    let ensure_key = format!("ensure:{session_id}");
    queue_shared(shared, ensure_key, CoreCommand::EnsureSession { session });
    if same_provider_release {
        super::queue_persisted_config_for_session(shared, &session_id, true);
    }
    let command_id = next_id_shared(shared, "reset");
    shared
        .active_reset_commands
        .lock()
        .insert(session_id.clone(), command_id.clone());
    if let Some(target_fence_id) = fence_id
        && let Some(fence) = shared.recovery_fences.lock().get_mut(&session_id)
        && fence.id == target_fence_id
    {
        fence.command_id = Some(command_id.clone());
    }
    queue_shared(
        shared,
        command_id.clone(),
        CoreCommand::StopSession {
            session_id,
            command_id,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn recovery_fence_rejection_supersede_reload_and_cold_wake_disconnect_invariants() {
        let hub = crate::core::Hub::new();
        hub.create_session(crate::core::SessionRegistration {
            id: "sess-r2".into(),
            provider: "codex".into(),
            provider_version: "1.0.0".into(),
            provider_generation_digest: "sha256:test".into(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "hawk".into(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: "/tmp".into(),
            title: "t".into(),
            origin: crate::model::SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
        hub.set_status("sess-r2", Status::Starting, None);
        let rt = crate::remote_runtime::RemoteRuntime::for_test(hub.clone(), Vec::new());

        // High #4 & Medium #9: rejected recovery transitions fence to Failed
        // (keeping old worker fenced), and explicit reload refreshes it to
        // AwaitingReplacement so replacement readiness clears it.
        let fence_id = rt
            .shared
            .recovery_fence_counter
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        rt.shared.recovery_fences.lock().insert(
            "sess-r2".into(),
            RecoveryFence {
                id: fence_id,
                pre_recovery_epoch: "epoch-old".into(),
                state: RecoveryFenceState::AwaitingReplacement,
                command_id: Some("execution-recover-1".into()),
            },
        );
        assert!(fail_rejected_recovery_command(
            &rt.shared,
            "sess-r2",
            "execution-recover-1",
            "recovery refused",
        ));
        assert_eq!(
            rt.shared
                .recovery_fences
                .lock()
                .get("sess-r2")
                .unwrap()
                .state,
            RecoveryFenceState::Failed,
            "refused recovery must stay Failed so stale worker stays fenced"
        );
        refresh_recovery_fence_for_reset(&rt.shared, "sess-r2", "reset-2");
        let refreshed = rt
            .shared
            .recovery_fences
            .lock()
            .get("sess-r2")
            .unwrap()
            .clone();
        assert_eq!(refreshed.state, RecoveryFenceState::AwaitingReplacement);
        assert_eq!(refreshed.command_id.as_deref(), Some("reset-2"));
        assert!(refreshed.id > fence_id);

        // Medium #8: superseded recovery abort must NOT touch the newer recovery's
        // fence or pending prompts.
        rt.shared.pending.lock().insert(
            "prompt-new".into(),
            CoreCommand::Prompt {
                session_id: "sess-r2".into(),
                command_id: "prompt-new".into(),
                turn_id: "turn-new".into(),
                content: vec![serde_json::json!({"type": "text", "text": "keep me"})],
                cmid: None,
                trace: None,
                echo_artifacts: false,
                observe_subagents: false,
                expected_worker_epoch: None,
                acknowledged_uncertain_sequence: None,
            },
        );
        abort_failed_execution_recovery(&rt.shared, "sess-r2", 1, &HashMap::new(), "old abort")
            .await;
        assert_eq!(
            rt.shared.recovery_fences.lock().get("sess-r2").unwrap().id,
            refreshed.id
        );
        assert!(
            rt.shared.pending.lock().contains_key("prompt-new"),
            "superseded recovery abort must leave newer recovery's prompts intact"
        );
        rt.shared.pending.lock().clear();
        rt.shared.recovery_fences.lock().clear();

        // Medium #13: disconnect during optimistic cold wake restores Starting -> Exited
        // before invoking the wake callback.
        rt.shared
            .optimistic_revivals
            .lock()
            .insert("sess-r2".into());
        rt.shared.pending.lock().insert(
            "cmd-cold".into(),
            CoreCommand::Prompt {
                session_id: "sess-r2".into(),
                command_id: "cmd-cold".into(),
                turn_id: "turn-cold".into(),
                content: vec![serde_json::json!({"type": "text", "text": "notice"})],
                cmid: Some("call-notice:call-cold".into()),
                trace: None,
                echo_artifacts: false,
                observe_subagents: false,
                expected_worker_epoch: None,
                acknowledged_uncertain_sequence: None,
            },
        );
        let saw_exited_in_wake = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let saw_flag = Arc::clone(&saw_exited_in_wake);
        let wake_hub = hub.clone();
        *rt.shared.teardown_notice_wake.lock() = Some(Arc::new(move |sid, _cid| {
            if wake_hub.status(sid) == Some(Status::Exited) {
                saw_flag.store(true, Ordering::Release);
            }
        }));
        requeue_unsent_notices_on_disconnect(&rt.shared);
        hub.drain_teardown_requeues(std::time::Duration::from_secs(2))
            .await;
        assert_eq!(hub.status("sess-r2"), Some(Status::Exited));
        assert!(
            saw_exited_in_wake.load(Ordering::Acquire),
            "disconnect during optimistic cold wake must restore Exited before wake callback"
        );
    }

    #[tokio::test]
    async fn stale_reset_ack_during_parking_or_after_completion_is_ignored() {
        let hub = crate::core::Hub::new();
        hub.create_local_session(
            "sess-r4".into(),
            "codex".into(),
            "/tmp".into(),
            "test".into(),
            crate::model::SessionOrigin::Web,
            false,
        );
        hub.set_status("sess-r4", Status::Running, None);
        let worker = super::super::tests::snapshot("sess-r4");
        let rt = super::super::RemoteRuntime::for_test(hub.clone(), vec![worker]);

        // 1. During asynchronous recovery parking (`parking:7` placeholder), a
        //    stale `reset-old` ACK must NOT clear `resetting`, drop `workers`,
        //    or flip status to Starting/Crashed.
        super::super::begin_reset(&rt.shared, "sess-r4");
        rt.shared
            .active_reset_commands
            .lock()
            .insert("sess-r4".into(), "parking:7".into());
        rt.shared.pending.lock().insert(
            "reset-old".into(),
            CoreCommand::StopSession {
                session_id: "sess-r4".into(),
                command_id: "reset-old".into(),
            },
        );
        let mut sink = Vec::new();
        super::super::handle_frame(
            &rt.shared,
            crate::runtime_wire::Frame::CommandAck {
                session_id: "sess-r4".into(),
                command_id: "reset-old".into(),
                accepted: true,
                reason: None,
            },
            &mut sink,
        )
        .await
        .unwrap();
        assert!(rt.shared.resetting.lock().contains("sess-r4"));
        assert!(rt.shared.workers.lock().contains_key("sess-r4"));
        assert_eq!(
            rt.shared
                .active_reset_commands
                .lock()
                .get("sess-r4")
                .map(String::as_str),
            Some("parking:7")
        );
        assert_eq!(hub.status("sess-r4"), Some(Status::Running));
        assert!(
            rt.shared.pending.lock().version("reset-old").is_none(),
            "ACKed command and its version entry must be removed from PendingCommands"
        );

        // 2. After a reset completes or is cancelled (no entry in
        //    `active_reset_commands`), a late rejected stale reset ACK must
        //    also be ignored instead of flipping a live session to Crashed.
        rt.shared.resetting.lock().remove("sess-r4");
        rt.shared.active_reset_commands.lock().remove("sess-r4");
        rt.shared.pending.lock().insert(
            "reset-late".into(),
            CoreCommand::StopSession {
                session_id: "sess-r4".into(),
                command_id: "reset-late".into(),
            },
        );
        super::super::handle_frame(
            &rt.shared,
            crate::runtime_wire::Frame::CommandAck {
                session_id: "sess-r4".into(),
                command_id: "reset-late".into(),
                accepted: false,
                reason: Some("late refusal".into()),
            },
            &mut sink,
        )
        .await
        .unwrap();
        assert!(rt.shared.workers.lock().contains_key("sess-r4"));
        assert_eq!(hub.status("sess-r4"), Some(Status::Running));
        assert!(rt.shared.pending.lock().is_empty());
    }

    #[test]
    fn pending_commands_frees_version_entries_on_removal() {
        let mut pending = super::super::PendingCommands::default();
        let make = |id: &str| CoreCommand::StopSession {
            session_id: "s".into(),
            command_id: id.into(),
        };
        let v1 = pending.insert("cmd-1".into(), make("cmd-1"));
        let v2 = pending.insert("cmd-2".into(), make("cmd-2"));
        let v3 = pending.insert("cmd-3".into(), make("cmd-3"));
        assert!(v1 < v2 && v2 < v3);
        assert_eq!(pending.len(), 3);

        assert!(pending.remove("cmd-1").is_some());
        assert_eq!(pending.version("cmd-1"), None);
        pending.retain(|k, _| k != "cmd-2");
        assert_eq!(pending.version("cmd-2"), None);
        assert_eq!(pending.version("cmd-3"), Some(v3));
        assert_eq!(pending.len(), 1);

        let drained = pending.drain_commands();
        assert_eq!(drained.len(), 1);
        assert!(pending.is_empty());
        assert_eq!(pending.version("cmd-3"), None);

        let v4 = pending.insert("cmd-1".into(), make("cmd-1"));
        assert!(
            v4 > v3,
            "versions must remain strictly monotonic even after draining"
        );
    }
}
