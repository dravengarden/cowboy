//! Teardown/rejection notice requeue retries and atomic cold-revive + prompt
//! admission for [`super::super::RemoteRuntime`].

use super::{
    Arc, CALL_NOTICE_PREFIX, CoreCommand, NoticeRequeueRequest, Ordering, RecoveryDraftPark,
    Shared, StartSession, Status, UndeliveredRequeue, WorkerSnapshot, WorkerState,
    canonical_notice_payload,
};

pub(super) struct UnsentDisconnectNotice {
    pub(super) session_id: String,
    pub(super) command_id: String,
    pub(super) text: String,
    pub(super) content: Vec<serde_json::Value>,
    pub(super) cid: String,
    pub(super) was_optimistic_revival: bool,
}

fn extract_unsent_notices_on_disconnect(shared: &Shared) -> Vec<UnsentDisconnectNotice> {
    let raw = {
        let mut pending = shared.pending.lock();
        let attempted = shared.attempted_sends.lock();
        let mut sent = shared.sent.lock();
        let mut fenced = shared.fenced_commands.lock();
        let recovery_owned = shared.recovery_parking_keys.lock();
        let keys: Vec<String> = pending
            .iter_with_versions()
            .filter_map(|(key, version, cmd)| {
                if attempted.contains(key)
                    || recovery_owned.get(key).is_some_and(|(v, _)| *v == version)
                {
                    return None;
                }
                match cmd {
                    CoreCommand::Prompt {
                        cmid: Some(cid), ..
                    } if cid.starts_with(CALL_NOTICE_PREFIX) => Some(key.clone()),
                    _ => None,
                }
            })
            .collect();
        let mut staged_journal = Vec::with_capacity(keys.len());
        for key in &keys {
            if let Some(CoreCommand::Prompt {
                session_id,
                command_id,
                content,
                cmid: Some(cid),
                ..
            }) = pending.get(key)
            {
                let text = content
                    .iter()
                    .filter_map(|v| v.get("text").and_then(serde_json::Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                staged_journal.push(crate::core::JournalUnsentNotice {
                    session_id: session_id.clone(),
                    command_id: command_id.clone(),
                    text,
                    content: content.clone(),
                    completion_id: cid.clone(),
                });
            }
        }
        let _ = shared.hub.stage_unsent_notices_journal(&staged_journal);
        let mut extracted = Vec::with_capacity(keys.len());
        for key in keys {
            sent.remove(&key);
            fenced.remove(&key);
            if let Some(CoreCommand::Prompt {
                session_id,
                command_id,
                content,
                cmid: Some(cid),
                ..
            }) = pending.remove(&key)
            {
                let text = content
                    .iter()
                    .filter_map(|v| v.get("text").and_then(serde_json::Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                extracted.push((session_id, command_id, text, content, cid));
            }
        }
        extracted
    };
    raw.into_iter()
        .map(|(session_id, command_id, text, content, cid)| {
            let has_live_worker = shared.workers.lock().get(&session_id).is_some_and(|w| {
                w.has_connected_owner()
                    && !matches!(w.state, WorkerState::Exited | WorkerState::Crashed)
            });
            let was_optimistic_revival =
                !has_live_worker && shared.optimistic_revivals.lock().remove(&session_id);
            UnsentDisconnectNotice {
                session_id,
                command_id,
                text,
                content,
                cid,
                was_optimistic_revival,
            }
        })
        .collect()
}

/// Retry conditional FIFO requeue while the store writer is alive; if all
/// attempts exhaust under contention/error, attempt a direct durable claim
/// stamp clear before falling back to in-memory retention so shutdown never
/// leaves a provably unsent notice stamped as `dispatch_unconfirmed` (Round 3 #9).
enum TeardownRequeueOutcome {
    Retained,
    AlreadySettled,
    ShutdownTimedOut,
}

async fn commit_or_retry_teardown_requeue(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    req: &NoticeRequeueRequest,
) -> TeardownRequeueOutcome {
    let mut last_writer_closed = false;
    for (attempt, delay_ms) in [0_u64, 25, 50, 100].into_iter().enumerate() {
        if attempt > 0 {
            if !hub.has_store_writer() {
                break;
            }
            if hub
                .await_with_shutdown(tokio::time::sleep(std::time::Duration::from_millis(
                    delay_ms,
                )))
                .await
                .is_none()
            {
                return TeardownRequeueOutcome::ShutdownTimedOut;
            }
        }
        match hub.preview_and_send_undelivered_notice_requeue(req.clone()) {
            UndeliveredRequeue::NoStore => return TeardownRequeueOutcome::Retained,
            UndeliveredRequeue::Pending(mut preview, rx) => {
                match hub.await_with_shutdown(rx).await {
                    Some(Ok(Ok(true))) => {
                        hub.commit_undelivered_notice_requeue(&mut preview);
                        return TeardownRequeueOutcome::Retained;
                    }
                    Some(Ok(Ok(false))) => {
                        hub.abort_undelivered_notice_requeue(&mut preview, false);
                        return TeardownRequeueOutcome::AlreadySettled;
                    }
                    Some(Ok(Err(_))) => {
                        hub.abort_undelivered_notice_requeue(&mut preview, false);
                    }
                    Some(Err(_)) => {
                        last_writer_closed = true;
                        hub.fallback_undelivered_notice_requeue(&mut preview);
                        break;
                    }
                    None => {
                        hub.abandon_unknown_outcome_notice_requeue(&mut preview);
                        return TeardownRequeueOutcome::ShutdownTimedOut;
                    }
                }
            }
            UndeliveredRequeue::Rejected(_) => {}
        }
    }
    if let Some(store) = store {
        let now = chrono::Utc::now().timestamp_millis();
        if hub
            .await_with_shutdown(store.clear_call_notice_dispatching(
                &req.session_id,
                &req.cmid,
                None,
                req.expected_command_id.as_deref(),
                now,
            ))
            .await
            .is_none()
        {
            return TeardownRequeueOutcome::ShutdownTimedOut;
        }
    }
    if !last_writer_closed {
        hub.fallback_undelivered_notice_requeue_rejected(req);
    }
    TeardownRequeueOutcome::Retained
}

pub(in crate::remote_runtime) fn requeue_unsent_notices_on_disconnect(shared: &Arc<Shared>) {
    let unsent = extract_unsent_notices_on_disconnect(shared);
    if unsent.is_empty() {
        return;
    }
    spawn_teardown_notice_requeues(shared, unsent);
}

pub(super) fn spawn_teardown_notice_requeues(
    shared: &Arc<Shared>,
    unsent: Vec<UnsentDisconnectNotice>,
) {
    if unsent.is_empty() {
        return;
    }
    let wake_cb = shared.teardown_notice_wake.lock().clone();
    let store = shared
        .native_store
        .lock()
        .as_ref()
        .map(|(_, store)| store.clone());
    if tokio::runtime::Handle::try_current().is_ok() {
        let hub = shared.hub.clone();
        shared.hub.spawn_teardown_requeue(async move {
            for item in unsent {
                let Some(_commit_guard) = hub
                    .await_with_shutdown(hub.pending_commit_lock(&item.session_id))
                    .await
                else {
                    break;
                };
                let (text, content) = if let Some(store) = store.as_ref()
                    && let Some(Ok(Some(rebuilt))) = hub
                        .await_with_shutdown(canonical_notice_payload(
                            store,
                            &item.session_id,
                            &item.cid,
                        ))
                        .await
                {
                    rebuilt
                } else {
                    (item.text, item.content)
                };
                let req = NoticeRequeueRequest {
                    session_id: item.session_id.clone(),
                    text,
                    content,
                    cmid: item.cid.clone(),
                    expected_command_id: Some(item.command_id.clone()),
                    expected_worker_epoch: None,
                    require_worker_epoch_match: false,
                    receipt_key: format!("teardown:{}", item.command_id),
                    require_claim_release: true,
                };
                match commit_or_retry_teardown_requeue(&hub, store.as_ref(), &req).await {
                    TeardownRequeueOutcome::Retained => {
                        hub.prune_unsent_notice_journal(&item.session_id, &item.command_id);
                        if item.was_optimistic_revival
                            && hub.status(&item.session_id) == Some(Status::Starting)
                        {
                            hub.set_status(&item.session_id, Status::Exited, None);
                        }
                        if let Some(wake) = wake_cb.as_ref() {
                            wake(&item.session_id, &item.cid);
                        }
                    }
                    TeardownRequeueOutcome::AlreadySettled => {
                        hub.prune_unsent_notice_journal(&item.session_id, &item.command_id);
                    }
                    TeardownRequeueOutcome::ShutdownTimedOut => break,
                }
            }
        });
    } else {
        for item in unsent {
            let req = NoticeRequeueRequest {
                session_id: item.session_id.clone(),
                text: item.text,
                content: item.content,
                cmid: item.cid.clone(),
                expected_command_id: Some(item.command_id.clone()),
                expected_worker_epoch: None,
                require_worker_epoch_match: false,
                receipt_key: format!("teardown:{}", item.command_id),
                require_claim_release: true,
            };
            if shared.hub.has_store_writer() {
                shared
                    .hub
                    .fallback_undelivered_notice_requeue_rejected(&req);
            } else {
                let _ = shared.hub.preview_and_send_undelivered_notice_requeue(req);
            }
            if item.was_optimistic_revival
                && shared.hub.status(&item.session_id) == Some(Status::Starting)
            {
                shared
                    .hub
                    .set_status(&item.session_id, Status::Exited, None);
            }
            if let Some(wake) = wake_cb.as_ref() {
                wake(&item.session_id, &item.cid);
            }
        }
    }
}

fn requeue_rejected_without_cid(
    shared: &Shared,
    session_id: &str,
    command: Option<CoreCommand>,
    reason: Option<String>,
) {
    let provenance = match &command {
        Some(CoreCommand::Prompt { command_id, .. }) => Some(
            super::super::prompt_tracking::provenance_for_controller_command(
                shared, session_id, command_id, None,
            ),
        ),
        _ => None,
    };
    super::super::handle_rejected_command_with_provenance(
        &shared.hub,
        session_id,
        command,
        reason,
        provenance,
    );
}

fn requeue_rejected_ordinary_prompt(
    shared: &Shared,
    session_id: &str,
    command_id: String,
    content: Vec<serde_json::Value>,
    cid: String,
    reason: Option<String>,
) {
    let provenance = super::super::prompt_tracking::provenance_for_controller_command(
        shared,
        session_id,
        &command_id,
        Some(&cid),
    );
    super::super::handle_rejected_command_with_provenance(
        &shared.hub,
        session_id,
        Some(CoreCommand::Prompt {
            session_id: session_id.to_owned(),
            command_id,
            turn_id: String::new(),
            content,
            cmid: Some(cid),
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: None,
            acknowledged_uncertain_sequence: None,
        }),
        reason,
        Some(provenance),
    );
}

/// Route a rejected `CoreCommand` back to `Hub`: ordinary prompts use
/// `handle_rejected_command`, while `call-notice:` prompts use conditional
/// durable requeue under `pending_commit_lock` so a late rejection can never
/// reinsert an already-consumed or settled completion notice, and releases
/// the exact dispatch guard when SQL confirms settlement (Round 3 #14).
pub(in crate::remote_runtime) fn requeue_rejected_command(
    shared: &Shared,
    session_id: &str,
    command: Option<CoreCommand>,
    reason: Option<String>,
) {
    let Some(CoreCommand::Prompt {
        command_id,
        content,
        cmid: Some(cid),
        ..
    }) = command
    else {
        requeue_rejected_without_cid(shared, session_id, command, reason);
        return;
    };
    if !cid.starts_with(CALL_NOTICE_PREFIX) {
        requeue_rejected_ordinary_prompt(shared, session_id, command_id, content, cid, reason);
        return;
    }
    shared.in_flight_prompts.lock().retire_command(&command_id);
    let reason = reason.unwrap_or_else(|| "runtime rejected command".to_owned());
    let fallback_text = content
        .iter()
        .filter_map(|value| value.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    shared
        .hub
        .broadcast_error(Some(session_id.to_owned()), reason);
    let store = shared
        .native_store
        .lock()
        .as_ref()
        .map(|(_, store)| store.clone());
    if tokio::runtime::Handle::try_current().is_ok() {
        let hub = shared.hub.clone();
        let session_id = session_id.to_owned();
        shared.hub.spawn_teardown_requeue(async move {
            let _commit_guard = hub.pending_commit_lock(&session_id).await;
            let (text, content) = if let Some(store) = store.as_ref()
                && let Ok(Some(rebuilt)) = canonical_notice_payload(store, &session_id, &cid).await
            {
                rebuilt
            } else {
                (fallback_text, content)
            };
            let req = NoticeRequeueRequest {
                session_id: session_id.clone(),
                text,
                content,
                cmid: cid,
                expected_command_id: Some(command_id.clone()),
                expected_worker_epoch: None,
                require_worker_epoch_match: false,
                receipt_key: format!("rejected:{command_id}"),
                require_claim_release: true,
            };
            match hub.preview_and_send_undelivered_notice_requeue(req.clone()) {
                UndeliveredRequeue::NoStore => {}
                UndeliveredRequeue::Pending(mut preview, rx) => match rx.await {
                    Ok(Ok(true)) => hub.commit_undelivered_notice_requeue(&mut preview),
                    Ok(Ok(false)) => {
                        // Round 3 #14: SQL confirmed this notice is already
                        // settled (e.g. consumed by `calls.result`); do not
                        // reinsert it, but release the exact rejected dispatch
                        // guard so the parent queue is not blocked forever.
                        hub.release_rejected_settled_notice_dispatch(&mut preview);
                    }
                    Ok(Err(_)) | Err(_) => hub.fallback_undelivered_notice_requeue(&mut preview),
                },
                UndeliveredRequeue::Rejected(_) => {
                    hub.fallback_undelivered_notice_requeue_rejected(&req);
                }
            }
        });
    } else {
        let req = NoticeRequeueRequest {
            session_id: session_id.to_owned(),
            text: fallback_text,
            content,
            cmid: cid,
            expected_command_id: Some(command_id.clone()),
            expected_worker_epoch: None,
            require_worker_epoch_match: false,
            receipt_key: format!("rejected:{command_id}"),
            require_claim_release: true,
        };
        if shared.hub.has_store_writer() {
            shared
                .hub
                .fallback_undelivered_notice_requeue_rejected(&req);
        } else {
            let _ = shared.hub.preview_and_send_undelivered_notice_requeue(req);
        }
    }
}

/// Atomically revive a dormant session and enqueue its triggering prompt under
/// `projection_fence` so `disconnect()` cannot interleave between projecting
/// `Status::Starting` and inserting the prompt into `pending` (Round 3 #10).
/// Lock order: `projection_fence` -> `config_startups` -> `recovery_fences` ->
/// `pending` -> `sent`.
#[allow(clippy::too_many_arguments)]
pub(in crate::remote_runtime) fn revive_and_try_prompt(
    shared: &Shared,
    mut session: StartSession,
    command_id: String,
    turn_id: String,
    content: Vec<serde_json::Value>,
    cmid: Option<String>,
    trace: Option<crate::runtime_trace::TraceCarrier>,
) -> std::result::Result<String, String> {
    session.adopt_only = false;
    if session.generation.is_empty() {
        session.generation.clone_from(&shared.desired_generation);
    }
    let session_id = session.session_id.clone();
    let echo_artifacts = shared.hub.store_prompt_images(&content);
    let tracked = super::super::prompt_tracking::prepare_sent_prompt(
        &shared.hub,
        &session_id,
        &turn_id,
        cmid.as_deref(),
        &content,
    );
    {
        let fenced = shared.projection_fence.lock();
        if *fenced || shared.shutdown.load(Ordering::Acquire) {
            return Err("Machine runtime disconnected before prompt handoff".to_owned());
        }
        let has_worker = shared.workers.lock().contains_key(&session_id);
        let mut startups = shared.config_startups.lock();
        let recovery_fences = shared.recovery_fences.lock();
        if recovery_fences
            .get(&session_id)
            .is_some_and(|f| f.state == super::RecoveryFenceState::Failed)
        {
            return Err(
                "Execution recovery failed; prompt retained until recovery succeeds".to_owned(),
            );
        }
        let mut pending = shared.pending.lock();
        let mut sent = shared.sent.lock();
        if !has_worker {
            startups.insert(session_id.clone());
            shared.config_sync_epochs.lock().remove(&session_id);
        }
        shared.optimistic_revivals.lock().insert(session_id.clone());
        if !shared.resetting.lock().contains(&session_id) {
            let ensure_key = format!("ensure:{session_id}");
            sent.remove(&ensure_key);
            pending.insert(ensure_key, CoreCommand::EnsureSession { session });
        }
        shared
            .in_flight_prompts
            .lock()
            .record(command_id.clone(), tracked);
        sent.remove(&command_id);
        pending.insert(
            command_id.clone(),
            CoreCommand::Prompt {
                session_id: session_id.clone(),
                command_id: command_id.clone(),
                turn_id,
                content,
                cmid,
                trace,
                echo_artifacts,
                observe_subagents: true,
                expected_worker_epoch: None,
                acknowledged_uncertain_sequence: None,
            },
        );
        drop(sent);
        drop(pending);
        drop(recovery_fences);
        drop(startups);
        shared.hub.set_status(&session_id, Status::Starting, None);
    }
    super::super::queue_persisted_config_for_session(shared, &session_id, false);
    let _ = shared.notify.send(());
    Ok(command_id)
}

/// Classify whether a negative `CommandAck` for `command` represents an
/// ambiguous prompt delivery.
pub(in crate::remote_runtime) fn is_ambiguous_prompt_rejection(
    command: Option<&CoreCommand>,
    was_retransmitted: bool,
    reason: Option<&str>,
) -> bool {
    if !matches!(command, Some(CoreCommand::Prompt { .. })) {
        return false;
    }
    let explicit_ambiguous = reason.is_some_and(|message| {
        message.starts_with("delivery_ambiguous:")
            || message.starts_with("worker epoch changed before prompt admission")
    });
    let unproven_retry_refusal =
        was_retransmitted && !reason.is_some_and(|message| message.starts_with("unhanded:"));
    explicit_ambiguous || unproven_retry_refusal
}

/// True ONLY when an authoritative replacement worker epoch has superseded the
/// prompt's bound `expected_worker_epoch`. Same-epoch refusals and disconnects
/// must NEVER reconcile the idle guard from a stale cached snapshot.
pub(in crate::remote_runtime) fn replacement_worker_superseded_epoch(
    shared: &Shared,
    session_id: &str,
    command: &CoreCommand,
) -> Option<WorkerSnapshot> {
    let CoreCommand::Prompt {
        expected_worker_epoch: Some(bound),
        ..
    } = command
    else {
        return None;
    };
    shared
        .workers
        .lock()
        .get(session_id)
        .filter(|worker| {
            worker.worker_epoch != *bound
                && worker.has_connected_owner()
                && !matches!(worker.state, WorkerState::Exited | WorkerState::Crashed)
        })
        .cloned()
}

/// Durably park an ambiguous ordinary prompt via the acknowledged FIFO
/// `StoreWrite::UpdatePending` path BEFORE retiring `shared.pending`.
/// Returns `true` only once the draft is durable (or in-memory test mode has no
/// store writer); returns `false` on store failure so callers keep the command
/// fenced rather than losing the payload.
#[allow(clippy::too_many_lines)]
pub(in crate::remote_runtime) async fn park_ambiguous_prompt_durable(
    shared: &Arc<Shared>,
    session_id: &str,
    command: &CoreCommand,
    expected_version: Option<u64>,
    reason: &str,
) -> bool {
    let CoreCommand::Prompt {
        command_id,
        turn_id,
        content,
        cmid,
        ..
    } = command
    else {
        return false;
    };
    let is_call_notice = cmid
        .as_deref()
        .is_some_and(|cid| cid.starts_with(CALL_NOTICE_PREFIX));
    if !is_call_notice {
        let text = content
            .iter()
            .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        let _commit_guard = shared.hub.pending_commit_lock(session_id).await;
        let committed = match shared.hub.preview_park_recovery_prompt(
            session_id,
            text,
            content.clone(),
            cmid.clone(),
        ) {
            RecoveryDraftPark::NoStore | RecoveryDraftPark::Duplicate => true,
            RecoveryDraftPark::Pending(mut preview, rx) => {
                if let Ok(Ok(true)) = rx.await {
                    shared.hub.commit_park_recovery_prompt(&mut preview);
                    true
                } else {
                    shared.hub.abort_park_recovery_notices(&mut preview);
                    false
                }
            }
            RecoveryDraftPark::Rejected(_) => false,
        };
        if !committed {
            let text = content
                .iter()
                .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            // Finding 6: Carry exact command identity through retry completion
            // so once Hub commits the draft, the fenced command is retired and
            // an idle replacement worker can release the dispatch guard.
            let weak_shared = Arc::downgrade(shared);
            let retry_sid = session_id.to_owned();
            let retry_cmd_id = command_id.clone();
            let retry_turn_id = turn_id.clone();
            let retry_cmd = command.clone();
            let on_committed: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                let Some(shared) = weak_shared.upgrade() else {
                    return;
                };
                let removed = match expected_version {
                    Some(v) => {
                        let mut pending = shared.pending.lock();
                        let removed = pending.remove_if_version(&retry_cmd_id, v).is_some();
                        if removed {
                            shared.sent.lock().remove(&retry_cmd_id);
                            shared.attempted_sends.lock().remove(&retry_cmd_id);
                            shared.retransmitted_sends.lock().remove(&retry_cmd_id);
                            let mut fenced = shared.fenced_commands.lock();
                            if fenced.get(&retry_cmd_id) == Some(&v) {
                                fenced.remove(&retry_cmd_id);
                            }
                            shared
                                .in_flight_prompts
                                .lock()
                                .retire_command_for_turn(&retry_cmd_id, &retry_turn_id);
                        }
                        removed
                    }
                    None => false,
                };
                if !removed {
                    return;
                }
                let projection_guard = shared.projection_fence.lock();
                if !*projection_guard
                    && shared.connected.load(Ordering::Acquire)
                    && !shared.shutdown.load(Ordering::Acquire)
                    && let Some(replacement) =
                        replacement_worker_superseded_epoch(&shared, &retry_sid, &retry_cmd)
                {
                    reconcile_idle_guard_for_snapshot(&shared, &retry_sid, &replacement);
                }
            });
            let _ = shared.hub.enqueue_durable_recovery_park(
                session_id.to_owned(),
                text,
                content.clone(),
                cmid.clone(),
                reason.to_owned(),
                Some(on_committed),
            );
            shared.hub.broadcast_error(
                Some(session_id.to_owned()),
                format!("{reason} (durable draft commit queued for retry; prompt remains fenced)"),
            );
            return false;
        }
    }
    shared
        .hub
        .broadcast_error(Some(session_id.to_owned()), reason.to_owned());
    true
}

/// Durably park an ambiguous pending prompt BEFORE retiring `(key, version)`.
/// If durable commit fails, keeps the entry fenced in `shared.pending` so it is
/// neither lost nor replayed.
pub(in crate::remote_runtime) async fn park_and_retire_ambiguous_pending(
    shared: &Arc<Shared>,
    session_id: &str,
    key: &str,
    version: u64,
    command: CoreCommand,
    reason: &str,
    replacement_worker: Option<&WorkerSnapshot>,
) {
    if park_ambiguous_prompt_durable(shared, session_id, &command, Some(version), reason).await {
        let removed = {
            let mut pending = shared.pending.lock();
            let removed_cmd = pending.remove_if_version(key, version);
            if let Some(ref removed_cmd) = removed_cmd {
                shared.sent.lock().remove(key);
                shared.attempted_sends.lock().remove(key);
                shared.retransmitted_sends.lock().remove(key);
                let mut fenced = shared.fenced_commands.lock();
                if fenced.get(key) == Some(&version) {
                    fenced.remove(key);
                }
                if let CoreCommand::Prompt { turn_id, .. } = removed_cmd {
                    shared
                        .in_flight_prompts
                        .lock()
                        .retire_command_for_turn(key, turn_id);
                }
            }
            removed_cmd.is_some()
        };
        if removed && let Some(worker) = replacement_worker {
            let projection_guard = shared.projection_fence.lock();
            if !*projection_guard
                && shared.connected.load(Ordering::Acquire)
                && !shared.shutdown.load(Ordering::Acquire)
            {
                reconcile_idle_guard_for_snapshot(shared, session_id, worker);
            }
        }
    } else {
        shared
            .fenced_commands
            .lock()
            .insert(key.to_owned(), version);
    }
}

/// Re-evaluate Hub's idle dispatch guard against the exact admitted replacement
/// worker snapshot after retiring a parked prompt (Finding 13).
pub(in crate::remote_runtime) fn reconcile_idle_guard_for_snapshot(
    shared: &Shared,
    session_id: &str,
    worker: &WorkerSnapshot,
) {
    let guard = super::super::idle_snapshot_guard(shared, worker);
    super::super::reconcile_idle_guard(shared, session_id, guard);
}

/// Preserve unresolved ordinary (non-`call-notice:`) prompt payloads when a
/// `RemoteRuntime` disconnects or is replaced on Machine tunnel reconnect:
/// provably unattempted prompts roll back optimistic revivals and requeue into
/// Hub; already-attempted prompts commit durable recovery drafts BEFORE
/// retirement without reconciling idle guards from stale snapshots.
pub(in crate::remote_runtime) fn recover_pending_prompts_on_disconnect(shared: &Arc<Shared>) {
    let entries: Vec<(String, u64, CoreCommand, bool)> = {
        let pending = shared.pending.lock();
        let attempted = shared.attempted_sends.lock();
        let recovery_owned = shared.recovery_parking_keys.lock();
        pending
            .iter_with_versions()
            .filter_map(|(key, version, cmd)| {
                let CoreCommand::Prompt { cmid, .. } = cmd else {
                    return None;
                };
                if cmid
                    .as_deref()
                    .is_some_and(|cid| cid.starts_with(CALL_NOTICE_PREFIX))
                {
                    return None;
                }
                if recovery_owned.get(key).is_some_and(|(v, _)| *v == version) {
                    return None;
                }
                Some((key.clone(), version, cmd.clone(), attempted.contains(key)))
            })
            .collect()
    };
    let disconnect_reason =
        "Machine connection dropped while prompt delivery was unresolved; retained as draft";
    for (key, version, cmd, was_attempted) in entries {
        let Some(session_id) = cmd.session_id().map(str::to_owned) else {
            continue;
        };
        if was_attempted {
            recover_attempted_prompt_on_disconnect(
                shared,
                session_id,
                key,
                version,
                cmd,
                disconnect_reason,
            );
        } else {
            let removed = {
                let mut pending = shared.pending.lock();
                let removed = pending.remove_if_version(&key, version);
                if removed.is_some() {
                    shared.sent.lock().remove(&key);
                    let mut fenced = shared.fenced_commands.lock();
                    if fenced.get(&key) == Some(&version) {
                        fenced.remove(&key);
                    }
                }
                removed
            };
            if let Some(removed_cmd) = removed {
                let has_live_worker = shared.workers.lock().get(&session_id).is_some_and(|w| {
                    w.has_connected_owner()
                        && !matches!(w.state, WorkerState::Exited | WorkerState::Crashed)
                });
                let was_optimistic_revival =
                    !has_live_worker && shared.optimistic_revivals.lock().remove(&session_id);
                if was_optimistic_revival
                    && shared.hub.status(&session_id) == Some(Status::Starting)
                {
                    shared.hub.set_status(&session_id, Status::Exited, None);
                }
                requeue_rejected_command(
                    shared,
                    &session_id,
                    Some(removed_cmd),
                    Some(
                        "Machine connection dropped before prompt handoff; returned to queue"
                            .to_owned(),
                    ),
                );
            }
        }
    }
}

fn recover_attempted_prompt_on_disconnect(
    shared: &Arc<Shared>,
    session_id: String,
    key: String,
    version: u64,
    cmd: CoreCommand,
    disconnect_reason: &str,
) {
    let mut retire_now = true;
    if let CoreCommand::Prompt {
        command_id,
        turn_id,
        content,
        cmid,
        ..
    } = cmd
    {
        let text = content
            .iter()
            .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        let weak_shared = Arc::downgrade(shared);
        let retry_key = key.clone();
        let retry_turn = turn_id.clone();
        let on_committed: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let Some(shared) = weak_shared.upgrade() else {
                return;
            };
            let mut pending = shared.pending.lock();
            if pending.remove_if_version(&retry_key, version).is_some() {
                shared.sent.lock().remove(&retry_key);
                shared.attempted_sends.lock().remove(&retry_key);
                shared.retransmitted_sends.lock().remove(&retry_key);
                let mut fenced = shared.fenced_commands.lock();
                if fenced.get(&retry_key) == Some(&version) {
                    fenced.remove(&retry_key);
                }
                shared
                    .in_flight_prompts
                    .lock()
                    .retire_command_for_turn(&retry_key, &retry_turn);
            }
        });
        retire_now = shared.hub.enqueue_durable_recovery_park(
            session_id,
            text,
            content,
            cmid,
            disconnect_reason.to_owned(),
            Some(on_committed),
        );
        if retire_now {
            shared
                .in_flight_prompts
                .lock()
                .retire_command_for_turn(&command_id, &turn_id);
        }
    }
    let mut pending = shared.pending.lock();
    if retire_now {
        if pending.remove_if_version(&key, version).is_some() {
            shared.sent.lock().remove(&key);
            shared.attempted_sends.lock().remove(&key);
            shared.retransmitted_sends.lock().remove(&key);
            let mut fenced = shared.fenced_commands.lock();
            if fenced.get(&key) == Some(&version) {
                fenced.remove(&key);
            }
        }
    } else if pending.version(&key) == Some(version) {
        shared.fenced_commands.lock().insert(key, version);
    }
}

/// Park an already-attempted prompt whose delivery across a worker change is
/// ambiguous: retire `in_flight_prompts`, preserve non-`call-notice:` prompts
/// as durable non-dispatchable drafts, and surface the error.
pub(in crate::remote_runtime) fn park_ambiguous_prompt_command(
    shared: &Shared,
    session_id: &str,
    command: CoreCommand,
    reason: &str,
) {
    if let CoreCommand::Prompt {
        command_id,
        content,
        cmid,
        ..
    } = command
    {
        let is_call_notice = cmid
            .as_deref()
            .is_some_and(|cid| cid.starts_with(CALL_NOTICE_PREFIX));
        if !is_call_notice {
            let text = content
                .iter()
                .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            shared
                .hub
                .park_recovery_draft(session_id, text, content, cmid);
        }
        shared.in_flight_prompts.lock().retire_command(&command_id);
        shared
            .hub
            .broadcast_error(Some(session_id.to_owned()), reason.to_owned());
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn park_aborted_disconnected_prompts(
    shared: &Arc<Shared>,
    session_id: &str,
    disconnected_parks: Vec<(String, u64, bool, CoreCommand)>,
    message: &str,
    pending: &mut super::super::PendingCommands,
    sent: &mut std::collections::HashSet<String>,
    attempted: &mut std::collections::HashSet<String>,
    fenced: &mut std::collections::HashMap<String, u64>,
) -> Vec<UnsentDisconnectNotice> {
    let mut unsent_notices = Vec::new();
    for (key, version, was_attempted, cmd) in disconnected_parks {
        if let CoreCommand::Prompt {
            command_id,
            turn_id,
            content,
            cmid,
            ..
        } = cmd
        {
            let text = content
                .iter()
                .filter_map(|p| p.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            if let Some(cid) = cmid
                .as_deref()
                .filter(|c| c.starts_with(CALL_NOTICE_PREFIX))
            {
                if !was_attempted && !attempted.contains(&key) {
                    let journal_item = crate::core::JournalUnsentNotice {
                        session_id: session_id.to_owned(),
                        command_id: command_id.clone(),
                        text: text.clone(),
                        content: content.clone(),
                        completion_id: cid.to_owned(),
                    };
                    if shared.hub.stage_unsent_notices_journal(&[journal_item])
                        && pending.remove_if_version(&key, version).is_some()
                    {
                        sent.remove(&key);
                        if fenced.get(&key) == Some(&version) {
                            fenced.remove(&key);
                        }
                        shared
                            .in_flight_prompts
                            .lock()
                            .retire_command_for_turn(&command_id, &turn_id);
                        let has_live_worker =
                            shared.workers.lock().get(session_id).is_some_and(|w| {
                                w.has_connected_owner()
                                    && !matches!(
                                        w.state,
                                        WorkerState::Exited | WorkerState::Crashed
                                    )
                            });
                        let was_optimistic_revival = !has_live_worker
                            && shared.optimistic_revivals.lock().remove(session_id);
                        unsent_notices.push(UnsentDisconnectNotice {
                            session_id: session_id.to_owned(),
                            command_id,
                            text,
                            content,
                            cid: cid.to_owned(),
                            was_optimistic_revival,
                        });
                        continue;
                    }
                }
                fenced.insert(key, version);
            } else {
                let journal_ok = shared.hub.enqueue_durable_recovery_park(
                    session_id.to_owned(),
                    text,
                    content,
                    cmid,
                    message.to_owned(),
                    None,
                );
                if journal_ok && pending.remove_if_version(&key, version).is_some() {
                    sent.remove(&key);
                    attempted.remove(&key);
                    if fenced.get(&key) == Some(&version) {
                        fenced.remove(&key);
                    }
                    shared
                        .in_flight_prompts
                        .lock()
                        .retire_command_for_turn(&command_id, &turn_id);
                } else {
                    fenced.insert(key, version);
                }
            }
        } else {
            fenced.insert(key, version);
        }
    }
    unsent_notices
}

pub(super) fn cleanup_superseded_recovery_claims(
    shared: &Arc<Shared>,
    session_id: &str,
    fence_id: u64,
    pre_recovery_keys: &std::collections::HashMap<String, (u64, bool)>,
) {
    let disconnected =
        !shared.connected.load(Ordering::Acquire) || shared.shutdown.load(Ordering::Acquire);
    let unsent = {
        let mut pending = shared.pending.lock();
        let mut sent = shared.sent.lock();
        let mut attempted = shared.attempted_sends.lock();
        let mut fenced = shared.fenced_commands.lock();
        let mut owned = shared.recovery_parking_keys.lock();
        let mut disconnected_items = Vec::new();
        for (key, (version, was_attempted)) in pre_recovery_keys {
            if owned.get(key) != Some(&(*version, fence_id)) {
                continue;
            }
            owned.remove(key);
            if disconnected
                && pending.version(key) == Some(*version)
                && let Some(cmd) = pending.get(key).cloned()
            {
                disconnected_items.push((key.clone(), *version, *was_attempted, cmd));
            }
        }
        park_aborted_disconnected_prompts(
            shared,
            session_id,
            disconnected_items,
            "Machine disconnected while execution recovery was superseded; retained as draft",
            &mut pending,
            &mut sent,
            &mut attempted,
            &mut fenced,
        )
    };
    if !unsent.is_empty() {
        spawn_teardown_notice_requeues(shared, unsent);
    }
}

pub(in crate::remote_runtime) fn set_connected(shared: &Shared, value: bool) {
    let _projection_guard = shared.projection_fence.lock();
    shared.connected.store(value, Ordering::Release);
}

pub(in crate::remote_runtime) fn set_connected_if_active(shared: &Shared) {
    let projection_guard = shared.projection_fence.lock();
    if !*projection_guard && !shared.shutdown.load(Ordering::Acquire) {
        shared.connected.store(true, Ordering::Release);
    }
}

pub(super) fn release_claimed_recovery_keys(
    shared: &Shared,
    session_id: &str,
    fence_id: u64,
    pre_recovery_keys: &std::collections::HashMap<String, (u64, bool)>,
) {
    let mut recovery_fences = shared.recovery_fences.lock();
    if recovery_fences
        .get(session_id)
        .is_some_and(|f| f.id == fence_id)
    {
        recovery_fences.remove(session_id);
    }
    release_claimed_recovery_keys_only(shared, fence_id, pre_recovery_keys);
}

pub(super) fn release_claimed_recovery_keys_only(
    shared: &Shared,
    fence_id: u64,
    pre_recovery_keys: &std::collections::HashMap<String, (u64, bool)>,
) {
    let mut owned = shared.recovery_parking_keys.lock();
    for (key, (version, _)) in pre_recovery_keys {
        if owned.get(key) == Some(&(*version, fence_id)) {
            owned.remove(key);
        }
    }
}

pub(super) type RebuiltRecoveryBatch = (
    std::collections::HashMap<String, (u64, bool)>,
    Vec<(String, Vec<serde_json::Value>, String)>,
    Vec<(String, Vec<serde_json::Value>, Option<String>)>,
);

pub(super) fn rebuild_still_owned_recovery_batch(
    shared: &Shared,
    session_id: &str,
    fence_id: u64,
    claimed: &std::collections::HashMap<String, (u64, bool)>,
) -> RebuiltRecoveryBatch {
    let pending = shared.pending.lock();
    let attempted = shared.attempted_sends.lock();
    let owned = shared.recovery_parking_keys.lock();
    let mut active_keys = std::collections::HashMap::new();
    let mut notices = Vec::new();
    let mut others = Vec::new();
    for (key, (version, was_attempted)) in claimed {
        if owned.get(key) != Some(&(*version, fence_id)) || pending.version(key) != Some(*version) {
            continue;
        }
        let Some(command) = pending.get(key) else {
            continue;
        };
        if command.session_id() != Some(session_id) {
            continue;
        }
        active_keys.insert(
            key.clone(),
            (*version, *was_attempted || attempted.contains(key)),
        );
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
    (active_keys, notices, others)
}

#[cfg(test)]
mod r10_notice_recovery_tests {
    use super::*;
    use crate::core::{PersistenceHealth, StoreSink, StoreWrite};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn prompt_cmd(session_id: &str, command_id: &str, cmid: &str, text: &str) -> CoreCommand {
        CoreCommand::Prompt {
            session_id: session_id.to_owned(),
            command_id: command_id.to_owned(),
            turn_id: format!("turn-{command_id}"),
            content: vec![serde_json::json!({"type": "text", "text": text})],
            cmid: Some(cmid.to_owned()),
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: Some("epoch-old".to_owned()),
            acknowledged_uncertain_sequence: None,
        }
    }

    #[tokio::test]
    async fn explicit_recovery_exclusively_owns_pending_key_against_late_ack_and_disconnect() {
        let health = Arc::new(PersistenceHealth::default());
        let (sink, mut store_rx) = StoreSink::channel(16, health);
        let hub = crate::core::Hub::with_store(Some(sink));
        let acks = hub.store_ack_sink();
        hub.create_local_session(
            "sess-f2".into(),
            "codex".into(),
            "/tmp".into(),
            "f2".into(),
            crate::model::SessionOrigin::Web,
            false,
        );
        hub.set_status("sess-f2", Status::Running, None);
        let worker = super::super::super::tests::snapshot("sess-f2");
        let rt = super::super::super::RemoteRuntime::for_test(hub.clone(), vec![worker]);

        let cmd = prompt_cmd("sess-f2", "cmd-f2", "cmid-f2", "hello f2");
        let v1 = rt.shared.pending.lock().insert("cmd-f2".into(), cmd);
        rt.shared.attempted_sends.lock().insert("cmd-f2".into());

        // Trigger explicit durable recovery -> holds UpdatePending ack.
        let start: StartSession = serde_json::from_value(serde_json::json!({
            "session_id": "sess-f2",
            "provider": "codex",
            "cwd": "/tmp",
            "generation": ""
        }))
        .unwrap();
        super::super::recover_execution_durable(&rt.shared, start, None).unwrap();
        assert_eq!(
            rt.shared
                .recovery_parking_keys
                .lock()
                .get("cmd-f2")
                .map(|(v, _)| *v),
            Some(v1)
        );
        let ack_id = loop {
            if let StoreWrite::UpdatePending {
                ack_id: Some(id), ..
            } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                break id;
            }
        };

        // Late negative CommandAck MUST be ignored while explicit recovery owns `(cmd-f2, v1)`.
        assert!(
            super::super::take_acked_command(
                &rt.shared,
                "sess-f2",
                "cmd-f2",
                false,
                Some("worker_changed"),
            )
            .await
            .is_none()
        );
        assert_eq!(rt.shared.pending.lock().version("cmd-f2"), Some(v1));

        // Disconnect recovery MUST also skip `(cmd-f2, v1)` while explicit recovery owns it.
        recover_pending_prompts_on_disconnect(&rt.shared);
        assert_eq!(rt.shared.pending.lock().version("cmd-f2"), Some(v1));

        // Complete the explicit recovery batch park -> single draft committed, key retired.
        acks.complete(ack_id, Ok(true));
        for _ in 0..50 {
            if rt.shared.pending.lock().version("cmd-f2").is_none()
                && !rt
                    .shared
                    .recovery_parking_keys
                    .lock()
                    .contains_key("cmd-f2")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(rt.shared.pending.lock().version("cmd-f2").is_none());
        assert!(
            !rt.shared
                .recovery_parking_keys
                .lock()
                .contains_key("cmd-f2")
        );
        assert_eq!(
            hub.queue_and_drafts("sess-f2", |_| true).unwrap().1.len(),
            1
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn retry_park_uses_pre_await_version_and_skips_idle_reconcile_when_disconnected() {
        let health = Arc::new(PersistenceHealth::default());
        let (sink, mut store_rx) = StoreSink::channel(32, health);
        let hub = crate::core::Hub::with_store(Some(sink));
        let acks = hub.store_ack_sink();
        let (disp_tx, mut disp_rx) = mpsc::channel(8);
        hub.set_dispatch_tx(disp_tx);
        hub.create_local_session(
            "sess-f7".into(),
            "codex".into(),
            "/tmp".into(),
            "f7".into(),
            crate::model::SessionOrigin::Web,
            false,
        );
        hub.set_status("sess-f7", Status::Running, None);
        let mut replacement = super::super::super::tests::snapshot("sess-f7");
        replacement.worker_epoch = "epoch-new".into();
        replacement.state = WorkerState::Running;
        let rt = super::super::super::RemoteRuntime::for_test(hub.clone(), vec![replacement]);

        // 1. Finding 7: command_id reused with v2 while retry for v1 is queued -> v2 must NOT be removed.
        let cmd1 = prompt_cmd("sess-f7", "cmd-reuse", "cmid-f7-a", "first");
        let v1 = rt
            .shared
            .pending
            .lock()
            .insert("cmd-reuse".into(), cmd1.clone());
        let fail_once = tokio::spawn({
            let acks = acks.clone();
            async move {
                while let Some(write) = store_rx.recv().await {
                    if let StoreWrite::UpdatePending {
                        ack_id: Some(id), ..
                    } = write
                    {
                        acks.complete(id, Err("transient".into()));
                        break;
                    }
                }
                store_rx
            }
        });
        assert!(
            !park_ambiguous_prompt_durable(&rt.shared, "sess-f7", &cmd1, Some(v1), "stale").await
        );
        let mut store_rx = fail_once.await.unwrap();
        let cmd2 = prompt_cmd("sess-f7", "cmd-reuse", "cmid-f7-b", "second");
        let v2 = rt.shared.pending.lock().insert("cmd-reuse".into(), cmd2);
        assert!(v2 > v1);
        let ack_all = tokio::spawn({
            let acks = acks.clone();
            async move {
                while let Some(write) = store_rx.recv().await {
                    if let StoreWrite::UpdatePending {
                        ack_id: Some(id), ..
                    } = write
                    {
                        acks.complete(id, Ok(true));
                    }
                }
            }
        });
        for _ in 0..60 {
            if !hub
                .queue_and_drafts("sess-f7", |_| true)
                .unwrap()
                .1
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
        assert_eq!(
            rt.shared.pending.lock().version("cmd-reuse"),
            Some(v2),
            "retry callback for v1 must NEVER remove newer v2 entry"
        );

        // 2. Finding 8: disconnected runtime retry completion must NOT drain Hub from stale snapshot.
        hub.set_status("sess-f7", Status::Busy, None);
        hub.submit(
            "sess-f7",
            "wait-for-live-worker".into(),
            vec![],
            Some("cmid-q8".into()),
            &crate::prompt_origin::RequestCaller::Person,
        );
        assert_eq!(
            hub.queue_and_drafts("sess-f7", |_| true).unwrap().0.len(),
            1
        );
        rt.shared.connected.store(false, Ordering::Release);
        let cmd3 = prompt_cmd("sess-f7", "cmd-f8", "cmid-f8", "third");
        let v3 = rt
            .shared
            .pending
            .lock()
            .insert("cmd-f8".into(), cmd3.clone());
        let _ = super::super::take_acked_command(
            &rt.shared,
            "sess-f7",
            "cmd-f8",
            false,
            Some("worker_changed"),
        )
        .await;
        assert!(rt.shared.pending.lock().version("cmd-f8").is_none());
        let _ = v3;
        assert!(
            disp_rx.try_recv().is_err(),
            "disconnected runtime must not trigger drain from stale worker snapshot"
        );
        ack_all.abort();
    }

    #[tokio::test]
    async fn r11_failed_disconnected_recovery_requeues_provably_unsent_notice() {
        let dir = tempfile::tempdir().unwrap();
        let journal_path = dir.path().join("recovery-journal").join("parks.json");
        let health = Arc::new(PersistenceHealth::default());
        let (sink, mut store_rx) = StoreSink::channel(32, health);
        let hub = crate::core::Hub::with_store(Some(sink));
        let acks = hub.store_ack_sink();
        hub.create_local_session(
            "sess-r11-9".into(),
            "codex".into(),
            "/tmp".into(),
            "r11".into(),
            crate::model::SessionOrigin::Web,
            false,
        );
        hub.set_status("sess-r11-9", Status::Running, None);
        hub.attach_and_reconcile_recovery_journal(journal_path.clone());
        let worker = super::super::super::tests::snapshot("sess-r11-9");
        let rt = super::super::super::RemoteRuntime::for_test(hub.clone(), vec![worker]);

        // Unattempted call-notice command owned by explicit recovery.
        let notice_cmd = prompt_cmd(
            "sess-r11-9",
            "cmd-notice-9",
            "call-notice:call-r11-9",
            "completed child",
        );
        let v_notice = rt
            .shared
            .pending
            .lock()
            .insert("cmd-notice-9".into(), notice_cmd);
        let start: StartSession = serde_json::from_value(serde_json::json!({
            "session_id": "sess-r11-9",
            "provider": "codex",
            "cwd": "/tmp",
            "generation": ""
        }))
        .unwrap();
        super::super::recover_execution_durable(&rt.shared, start, None).unwrap();

        // Runtime disconnects while recovery batch park is in flight (so disconnect skips owned notice).
        set_connected(&rt.shared, false);
        requeue_unsent_notices_on_disconnect(&rt.shared);
        assert_eq!(
            rt.shared.pending.lock().version("cmd-notice-9"),
            Some(v_notice)
        );

        // Fail the batch park ACK -> abort sees disconnected && !was_attempted -> journals & requeues notice!
        let batch_ack = loop {
            if let StoreWrite::UpdatePending {
                ack_id: Some(id), ..
            } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                break id;
            }
        };
        acks.complete(batch_ack, Err("transient db failure".into()));
        // Next acked UpdatePending is the teardown notice requeue!
        let requeue_ack = loop {
            if let StoreWrite::UpdatePending {
                ack_id: Some(id),
                queue,
                ..
            } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                .await
                .unwrap()
                .unwrap()
                && queue
                    .iter()
                    .any(|m| m.cmid.as_deref() == Some("call-notice:call-r11-9"))
            {
                break id;
            }
        };
        acks.complete(requeue_ack, Ok(true));
        for _ in 0..50 {
            if hub
                .queue_and_drafts("sess-r11-9", |_| true)
                .unwrap()
                .0
                .iter()
                .any(|m| m.cmid.as_deref() == Some("call-notice:call-r11-9"))
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            hub.queue_and_drafts("sess-r11-9", |_| true)
                .unwrap()
                .0
                .iter()
                .any(|m| m.cmid.as_deref() == Some("call-notice:call-r11-9")),
            "failed disconnected recovery must requeue provably unsent call-notice"
        );
    }

    #[tokio::test]
    async fn r11_turn_checked_retire_preserves_reused_command_id() {
        let hub = crate::core::Hub::new();
        let worker = super::super::super::tests::snapshot("sess-r11-15");
        let rt = super::super::super::RemoteRuntime::for_test(hub.clone(), vec![worker]);
        let tracked_new = super::super::super::prompt_tracking::prepare_sent_prompt(
            &hub,
            "sess-r11-15",
            "turn-new",
            Some("cmid-new"),
            &[serde_json::json!({"type": "text", "text": "new"})],
        );
        rt.shared
            .in_flight_prompts
            .lock()
            .record("cmd-dup".into(), tracked_new);
        rt.shared
            .in_flight_prompts
            .lock()
            .retire_command_for_turn("cmd-dup", "turn-old");
        assert!(
            rt.shared
                .in_flight_prompts
                .lock()
                .take_matching("cmd-dup", "sess-r11-15", Some("cmid-new"))
                .is_some(),
            "retire_command_for_turn for turn-old must not erase turn-new tracking"
        );
    }
}
