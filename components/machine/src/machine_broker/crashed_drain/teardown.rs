//! Teardown confirmation, launch-UUID-scoped sandbox cleanup, and
//! capped-backoff background reconciliation for cooperative crashed-worker drain.

use super::*;

impl Broker {
    pub(in crate::machine_broker) async fn handle_crashed_drain_worker_frame(
        &self,
        session_id: &str,
        epoch: &str,
        connection_id: u64,
        frame: Frame,
    ) {
        match frame {
            Frame::CrashedDrainCompleted {
                session_id: drained_session,
                worker_epoch,
                command_id,
            } if drained_session == session_id && worker_epoch == epoch => {
                // Retry resolving durable launch identity if a post-restart
                // Hello lookup failed transiently before confirming teardown (R7-H6).
                let needs_launch = {
                    let state = self.crashed_drains.inner.lock();
                    state.active.get(session_id).is_some_and(|a| {
                        a.epoch == epoch
                            && a.connection_id == connection_id
                            && a.retired_launch == RetiredLaunch::Unresolved
                    })
                };
                if needs_launch {
                    let resolved = self.resolve_durable_launch_for_drain(session_id).await;
                    if resolved != RetiredLaunch::Unresolved {
                        let mut state = self.crashed_drains.inner.lock();
                        if let Some(active) = state.active.get_mut(session_id)
                            && active.epoch == epoch
                            && active.connection_id == connection_id
                            && active.retired_launch == RetiredLaunch::Unresolved
                        {
                            active.retired_launch = resolved;
                        }
                    }
                }
                self.confirm_crashed_drain(session_id, epoch, connection_id, command_id.as_deref());
            }
            Frame::CrashedDrainAborted {
                session_id: aborted_session,
                worker_epoch,
                command_id,
                uncertain_effect,
            } if aborted_session == session_id && worker_epoch == epoch => {
                self.handle_crashed_drain_aborted(
                    session_id,
                    epoch,
                    connection_id,
                    command_id.as_deref(),
                    uncertain_effect,
                );
            }
            _ => {}
        }
    }

    /// Handle explicit `Frame::CrashedDrainAborted` from a worker that observed
    /// an unacknowledged `uncertain_effect`: merges the authoritative effect,
    /// clears matching pending/active drain state and fence, surfaces the
    /// updated snapshot, and flushes pending commands (R6-M1).
    pub(in crate::machine_broker) fn handle_crashed_drain_aborted(
        &self,
        session_id: &str,
        epoch: &str,
        connection_id: u64,
        command_id: Option<&str>,
        uncertain_effect: Option<crate::runtime_wire::UncertainEffect>,
    ) {
        let cleared_fence = {
            // Hold canonical `cancelled_sessions -> permanently_deleted_sessions ->
            // controller -> sessions -> workers -> pending_commands -> crashed_drains`
            // through exact-peer validation, state mutation, and snapshot enqueue so
            // permanent deletion cannot race snapshot publication (R12-M2, R13-M1).
            // Serialize against `record_deletion` via `deletion_journal ->
            // cancelled_sessions -> permanently_deleted_sessions` so a committed
            // durable deletion is immediately visible before abort publication (R14-M1).
            let journal = self.deletion_journal.lock();
            let journal_deleted = journal
                .as_ref()
                .is_some_and(|j| j.deleted().contains(session_id));
            let _cancelled = self.cancelled_sessions.lock();
            let deleted = self.permanently_deleted_sessions.lock();
            drop(journal);
            if journal_deleted || deleted.contains(session_id) {
                return;
            }
            let ctrl_guard = self.controller.lock();
            let sessions = self.sessions.lock();
            let mut workers = self.workers.lock();
            let Some(worker) = workers
                .get_mut(session_id)
                .filter(|w| w.connection_id == connection_id && w.epoch == epoch)
            else {
                return;
            };
            let pending = self.pending_commands.lock();
            let mut state = self.crashed_drains.inner.lock();
            let matched_active = state.active.get(session_id).is_some_and(|a| {
                !a.confirmed
                    && a.epoch == epoch
                    && a.connection_id == connection_id
                    && (command_id.is_none()
                        || a.command_id.is_none()
                        || a.command_id.as_deref() == command_id)
            });
            let matched_pending = command_id.is_some_and(|cid| {
                state.pending.get(cid).is_some_and(|p| {
                    p.session_id == session_id
                        && p.epoch == epoch
                        && p.connection_id == connection_id
                })
            });
            let prev_effect = worker.snapshot.uncertain_effect.clone();
            if let Some(effect) = uncertain_effect {
                crate::runtime_wire::merge_uncertain_effect(
                    &mut worker.snapshot.uncertain_effect,
                    effect.clone(),
                );
                let mut merged = state.receiptless_uncertain_effects.get(session_id).cloned();
                crate::runtime_wire::merge_uncertain_effect(&mut merged, effect);
                if let Some(merged) = merged {
                    state
                        .receiptless_uncertain_effects
                        .insert(session_id.to_owned(), merged);
                }
            }
            let cleared = if !matched_active && !matched_pending {
                if worker.snapshot.uncertain_effect == prev_effect {
                    return;
                }
                false
            } else {
                if matched_active {
                    state.active.remove(session_id);
                }
                if matched_pending && let Some(cid) = command_id {
                    state.pending.remove(cid);
                }
                let still_pending = state
                    .pending
                    .values()
                    .any(|p| p.session_id == session_id && p.epoch == epoch);
                if !still_pending && !state.active.contains_key(session_id) {
                    worker.crashed_drain_fenced = false;
                    true
                } else {
                    false
                }
            };
            if let Some(ctrl) = ctrl_guard.as_ref() {
                let mut snap = worker.snapshot.clone();
                snap.pending_prompt_count = pending.get(session_id).map_or(0, |commands| {
                    u64::try_from(
                        commands
                            .iter()
                            .filter(|cmd| matches!(cmd, WorkerCommand::Prompt { .. }))
                            .count(),
                    )
                    .unwrap_or(u64::MAX)
                });
                snap.crashed_drain_active = snap.has_connected_owner()
                    && Self::has_crashed_drain_for_epoch_locked(&state, session_id, epoch);
                snap.incarnation = self.reported_lineage(session_id);
                let declared_provider = sessions.get(session_id).map(|s| s.provider.as_str());
                wrap_up::sanitize_snapshot_for_controller(
                    &mut snap,
                    worker.raw_wrap_up_steering,
                    worker.negotiated_protocol,
                    ctrl.outer_machine_protocol,
                    ctrl.protocol,
                    declared_provider,
                );
                let _ = ctrl.tx.send(Frame::Snapshot {
                    worker: Box::new(snap),
                });
            }
            cleared
        };
        if cleared_fence {
            self.flush_pending(session_id);
        }
        self.crashed_drains.settled.notify_waiters();
    }

    /// Called inside `ensure_session_in_lifecycle` (already holding
    /// `session_lifecycle_gate`): verify no crashed-worker drain is fencing
    /// `session_id`, then atomically inspect/remove any recyclable worker or
    /// reserve `launching` under `workers -> crashed_drains` so a late
    /// reconnecting draining worker can never race with explicit Ensure (R8-H4).
    pub(in crate::machine_broker) async fn claim_ensure_slot(
        &self,
        session_id: &str,
        adopt_only: bool,
        desired_generation: &str,
    ) -> Result<(Option<Result<WorkerPeer, WorkerSnapshot>>, bool), Option<String>> {
        if !adopt_only {
            self.gate_ensure_against_crashed_drain(session_id)
                .await
                .map_err(Some)?;
        }
        let mut workers = self.workers.lock();
        if !adopt_only && self.has_any_crashed_drain_fence_locked(&workers, session_id) {
            return Err(Some("crashed worker drain is in progress; retry".into()));
        }
        match workers.get(session_id) {
            Some(worker)
                if !adopt_only
                    && worker_launch::should_recycle_for_explicit_revive(
                        &worker.snapshot,
                        desired_generation,
                    ) =>
            {
                let removed = workers.remove(session_id);
                if let Some(effect) = removed
                    .as_ref()
                    .and_then(|peer| peer.snapshot.uncertain_effect.clone())
                {
                    self.record_session_uncertain_effect(session_id, effect);
                }
                Ok((removed.map(Ok), false))
            }
            Some(worker) => Ok((Some(Err(worker.snapshot.clone())), false)),
            None => {
                if !adopt_only && !self.launching.lock().insert(session_id.to_owned()) {
                    return Err(None);
                }
                Ok((None, !adopt_only))
            }
        }
    }

    pub(in crate::machine_broker) fn has_any_crashed_drain_fence_locked(
        &self,
        workers: &HashMap<String, WorkerPeer>,
        session_id: &str,
    ) -> bool {
        if workers
            .get(session_id)
            .is_some_and(|worker| worker.crashed_drain_fenced)
        {
            return true;
        }
        let state = self.crashed_drains.inner.lock();
        state.active.contains_key(session_id)
            || state
                .pending
                .values()
                .any(|pending| pending.session_id == session_id)
            || state.unconfirmed_teardown.contains_key(session_id)
    }

    pub(in crate::machine_broker) async fn resolve_durable_launch_for_drain(
        &self,
        session_id: &str,
    ) -> RetiredLaunch {
        match self.current_durable_launch_id(session_id).await {
            Ok(Some(launch)) => RetiredLaunch::Verified(launch),
            Ok(None) => RetiredLaunch::VerifiedNone,
            Err(_) => RetiredLaunch::Unresolved,
        }
    }

    pub(super) async fn resolve_or_refresh_retired_launch(
        &self,
        session_id: &str,
        worker_epoch: &str,
        current: RetiredLaunch,
    ) -> RetiredLaunch {
        if current != RetiredLaunch::Unresolved {
            return current;
        }
        let resolved = self.resolve_durable_launch_for_drain(session_id).await;
        if resolved == RetiredLaunch::Unresolved {
            return RetiredLaunch::Unresolved;
        }
        let workers = self.workers.lock();
        if workers.contains_key(session_id)
            || self.launching.lock().contains(session_id)
            || self.resetting_sessions.lock().contains_key(session_id)
        {
            return RetiredLaunch::Unresolved;
        }
        let mut state = self.crashed_drains.inner.lock();
        let mut matched = false;
        if let Some(entry) = state.unconfirmed_teardown.get_mut(session_id)
            && entry.epoch == worker_epoch
        {
            entry.retired_launch = resolved;
            matched = true;
        }
        if let Some(active) = state.active.get_mut(session_id)
            && active.epoch == worker_epoch
        {
            active.retired_launch = resolved;
            matched = true;
        }
        for pending in state.pending.values_mut() {
            if pending.session_id == session_id && pending.epoch == worker_epoch {
                pending.retired_launch = resolved;
                matched = true;
            }
        }
        if matched {
            resolved
        } else {
            RetiredLaunch::Unresolved
        }
    }

    pub(in crate::machine_broker) fn record_session_uncertain_effect(
        &self,
        session_id: &str,
        effect: crate::runtime_wire::UncertainEffect,
    ) {
        let mut state = self.crashed_drains.inner.lock();
        let mut merged = state.receiptless_uncertain_effects.get(session_id).cloned();
        crate::runtime_wire::merge_uncertain_effect(&mut merged, effect);
        if let Some(merged) = merged {
            state
                .receiptless_uncertain_effects
                .insert(session_id.to_owned(), merged);
        }
    }

    pub(in crate::machine_broker) fn retained_receiptless_uncertain(
        &self,
        session_id: &str,
    ) -> Option<crate::runtime_wire::UncertainEffect> {
        self.crashed_drains
            .inner
            .lock()
            .receiptless_uncertain_effects
            .get(session_id)
            .cloned()
    }

    /// Atomically enqueue reconciled catch-up snapshots (with broker-verified
    /// `crashed_drain_active`) and `CrashedWorkerDrained` retirements under
    /// `controller -> sessions -> workers -> pending_commands -> crashed_drains`
    /// BEFORE arming worker replay (R10-H6, R10-M3).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::machine_broker) fn enqueue_handover_reconciliation_locked(
        &self,
        ctrl: &Controller,
        sessions: &HashMap<String, StartSession>,
        session_states: &HashMap<String, WorkerState>,
        startup_failures: &HashMap<String, String>,
        generation_commands: &HashMap<String, PathBuf>,
        workers: &HashMap<String, WorkerPeer>,
        welcome: &[WorkerSnapshot],
    ) {
        let tx = ctrl.tx.clone();
        let outer = ctrl.outer_machine_protocol;
        let protocol = ctrl.protocol;
        let pending_guard = self.pending_commands.lock();
        let pending_prompts: HashMap<String, u64> = pending_guard
            .iter()
            .map(|(sid, cmds)| {
                let count = cmds
                    .iter()
                    .filter(|cmd| matches!(cmd, WorkerCommand::Prompt { .. }))
                    .count();
                (sid.clone(), u64::try_from(count).unwrap_or(u64::MAX))
            })
            .collect();
        let state = self.crashed_drains.inner.lock();
        for worker in workers.values() {
            let mut snap = worker.snapshot.clone();
            snap.pending_prompt_count = pending_prompts.get(&snap.session_id).copied().unwrap_or(0);
            if snap.launch.is_none() {
                snap.launch = sessions.get(&snap.session_id).cloned();
            }
            let has_drain = state
                .active
                .get(&snap.session_id)
                .is_some_and(|a| a.epoch == worker.epoch)
                || state
                    .pending
                    .values()
                    .any(|p| p.session_id == snap.session_id && p.epoch == worker.epoch);
            snap.crashed_drain_active = has_drain;
            snap.incarnation = self.reported_lineage(&snap.session_id);
            let declared_provider = sessions.get(&snap.session_id).map(|s| s.provider.as_str());
            wrap_up::sanitize_snapshot_for_controller(
                &mut snap,
                worker.raw_wrap_up_steering,
                worker.negotiated_protocol,
                outer,
                protocol,
                declared_provider,
            );
            if !welcome.contains(&snap) {
                let _ = tx.send(Frame::Snapshot {
                    worker: Box::new(snap),
                });
            }
        }
        let controller_capable = protocol >= CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
            && outer.is_some_and(|p| p >= CRASHED_WORKER_DRAIN_PROTOCOL_VERSION);
        let now = Instant::now();
        self.enqueue_dormant_handover_snapshots_locked(
            &tx,
            protocol,
            outer,
            controller_capable,
            now,
            sessions,
            session_states,
            startup_failures,
            generation_commands,
            workers,
            &pending_prompts,
            &state,
            welcome,
        );
        if controller_capable {
            for snap in welcome {
                if !snap.has_connected_owner() || workers.contains_key(&snap.session_id) {
                    continue;
                }
                if let Some((ep, confirmed_receipt, at)) =
                    state.recently_drained.get(&snap.session_id)
                    && *confirmed_receipt
                    && now.saturating_duration_since(*at) < RECENTLY_DRAINED_TTL
                    && ep == &snap.worker_epoch
                {
                    let _ = tx.send(Frame::CrashedWorkerDrained {
                        session_id: snap.session_id.clone(),
                        worker_epoch: ep.clone(),
                    });
                }
            }
        }
    }

    /// Reconcile dormant snapshots for still-declared sessions during handover
    /// (ordinary disconnects, hibernations, downgraded Controllers, receiptless
    /// drains) while skipping confirmed crashed-drain retirements delivered via
    /// `Frame::CrashedWorkerDrained` (R8-M5, R9-M3, R10-H6, R10-M3).
    #[allow(clippy::too_many_arguments)]
    fn enqueue_dormant_handover_snapshots_locked(
        &self,
        tx: &tokio::sync::mpsc::UnboundedSender<Frame>,
        protocol: u16,
        outer: Option<u16>,
        controller_capable: bool,
        now: Instant,
        sessions: &HashMap<String, StartSession>,
        session_states: &HashMap<String, WorkerState>,
        startup_failures: &HashMap<String, String>,
        generation_commands: &HashMap<String, PathBuf>,
        workers: &HashMap<String, WorkerPeer>,
        pending_prompts: &HashMap<String, u64>,
        state: &Inner,
        welcome: &[WorkerSnapshot],
    ) {
        for (session_id, session) in sessions.iter() {
            if workers.contains_key(session_id) {
                continue;
            }
            let replayed_drained = controller_capable
                && welcome.iter().any(|w| {
                    w.session_id == *session_id
                        && w.has_connected_owner()
                        && state.recently_drained.get(session_id).is_some_and(
                            |(ep, confirmed_receipt, at)| {
                                *confirmed_receipt
                                    && now.saturating_duration_since(*at) < RECENTLY_DRAINED_TTL
                                    && ep == &w.worker_epoch
                            },
                        )
                });
            if replayed_drained {
                continue;
            }
            // Match `snapshots()` default (`WorkerState::Starting`) so an
            // adopt-only declaration with no `session_states` entry never
            // invents a spurious `Crashed` transition during handover (R10-M3).
            let dormant_state = session_states
                .get(session_id)
                .copied()
                .unwrap_or(WorkerState::Starting);
            let exit_detail = (dormant_state == WorkerState::Crashed)
                .then(|| startup_failures.get(session_id).cloned())
                .flatten();
            let executable = generation_commands
                .get(&session.generation)
                .map(|path| path.display().to_string());
            let mut dormant = WorkerSnapshot {
                session_id: session_id.clone(),
                worker_epoch: format!("broker-{session_id}"),
                generation: session.generation.clone(),
                executable,
                launch: Some(session.clone()),
                state: dormant_state,
                agent_session_id: session.agent_session_id.clone(),
                native_thread_materialized: None,
                current_turn_id: None,
                supports_current_turn: false,
                delivery_controls: crate::runtime_wire::DeliveryControls::default(),
                wrap_up_steering: false,
                supports_subagent_cancel: false,
                last_runtime_seq: 0,
                pending_permissions: Vec::new(),
                config_options: None,
                context_used: None,
                context_size: None,
                pending_prompt_count: pending_prompts.get(session_id).copied().unwrap_or(0),
                drain_requested: false,
                exit_detail,
                background_tasks: None,
                native_cycle_id: None,
                native_cycle_active: false,
                incarnation: self.reported_lineage(session_id),
                account_unit: None,
                uncertain_effect: state.receiptless_uncertain_effects.get(session_id).cloned(),
                crashed_drain_active: false,
            };
            wrap_up::sanitize_snapshot_for_controller(
                &mut dormant,
                false,
                protocol,
                outer,
                protocol,
                Some(session.provider.as_str()),
            );
            if !welcome.contains(&dormant) {
                let _ = tx.send(Frame::Snapshot {
                    worker: Box::new(dormant),
                });
            }
        }
    }

    /// Validate that `(connection_id, worker_epoch)` is still the active peer
    /// and persist its `uncertain_effect` atomically under `workers ->
    /// crashed_drains` (caller holds `cancelled_sessions` and has checked
    /// `!permanently_deleted`, R10-H2, R11-H1, R12-M3).
    pub(in crate::machine_broker) fn record_uncertain_if_validated_peer(
        &self,
        session_id: &str,
        connection_id: u64,
        worker_epoch: &str,
        event: &crate::runtime_wire::RuntimeEvent,
    ) {
        let crate::runtime_wire::RuntimeEvent::Update { update, .. } = event else {
            return;
        };
        let Some(effect) = crate::runtime_wire::uncertain_effect(update) else {
            return;
        };
        let workers = self.workers.lock();
        if !workers
            .get(session_id)
            .is_some_and(|w| w.connection_id == connection_id && w.epoch == worker_epoch)
        {
            return;
        }
        self.record_session_uncertain_effect(session_id, effect);
    }

    /// Record turn-completion and monotonic uncertainty outcomes for a
    /// historical replay event below the current epoch's snapshot watermark
    /// without rolling back lifecycle state or lowering `last_runtime_seq` (R14-H1).
    pub(in crate::machine_broker) fn record_historical_replay_outcome(
        &self,
        session_id: &str,
        worker: &mut WorkerPeer,
        event: &RuntimeEvent,
    ) {
        if let RuntimeEvent::TurnEnded { turn_id, .. } = event {
            self.dispatched_prompts
                .lock()
                .mark_acked_by_turn(session_id, &worker.epoch, turn_id);
        } else if let RuntimeEvent::Update { update, .. } = event
            && let Some(effect) = crate::runtime_wire::uncertain_effect(update)
        {
            crate::runtime_wire::merge_uncertain_effect(
                &mut worker.snapshot.uncertain_effect,
                effect,
            );
        }
    }

    /// Flush queued rollout prompts immediately when a rollout-draining worker
    /// transitions to `Crashed` without a crashed-drain fence (R10-M1).
    pub(in crate::machine_broker) fn flush_if_crashed_rollout_event(
        &self,
        session_id: &str,
        crashed_event: bool,
    ) {
        if crashed_event
            && self.workers.lock().get(session_id).is_some_and(|w| {
                w.snapshot.state == WorkerState::Crashed
                    && w.snapshot.drain_requested
                    && !w.crashed_drain_fenced
            })
        {
            self.flush_pending(session_id);
        }
    }

    /// Atomically validate exact peer/binding/crashed-drain fence and enqueue
    /// `Frame::ExecutionRequest` (or send `IdentityMismatch` refusal to the
    /// worker) under canonical `controller -> cancelled_sessions -> sessions ->
    /// workers -> crashed_drains` locks so drain admission cannot race ahead
    /// of forwarding (R11-H2, R12-H1).
    pub(in crate::machine_broker) fn forward_or_refuse_execution_request(
        &self,
        session_id: &str,
        epoch: &str,
        connection_id: u64,
        request: Box<crate::execution_protocol::RuntimeRequest>,
    ) {
        // Acquire `cancelled_sessions` BEFORE `controller` to match
        // `wrap_up.rs::handle_worker_event` -> `admit_and_bind_controller_for_event`
        // and eliminate any broker-wide lock-order deadlock (R13-H1).
        let cancelled = self.cancelled_sessions.lock();
        let ctrl_guard = self.controller.lock();
        let sessions = self.sessions.lock();
        let workers = self.workers.lock();
        let Some(worker) = workers
            .get(session_id)
            .filter(|w| w.connection_id == connection_id && w.epoch == epoch)
        else {
            return;
        };
        let drain_state = self.crashed_drains.inner.lock();
        let draining_crashed = worker.crashed_drain_fenced
            || Self::has_crashed_drain_for_epoch_locked(&drain_state, session_id, epoch);
        let binding_matches = request.session_id == session_id
            && request.worker_epoch == epoch
            && !cancelled.contains(session_id)
            && sessions
                .get(session_id)
                .and_then(|launch| launch.execution_binding.as_ref())
                .and_then(|binding| binding.decode().ok())
                .is_some_and(|binding| binding == request.binding);
        let allowed = if request.session_id != session_id || request.worker_epoch != epoch {
            false
        } else if draining_crashed {
            let cleanup_only = match &request.command {
                crate::execution_protocol::Command::Invoke { invocation, .. } => matches!(
                    invocation.method.as_str(),
                    "process/terminate"
                        | "process/read"
                        | crate::execution_host::user_mcp::USER_MCP_STOP_METHOD
                ),
                crate::execution_protocol::Command::Observe { .. }
                | crate::execution_protocol::Command::Events { .. } => true,
                crate::execution_protocol::Command::Describe
                | crate::execution_protocol::Command::Shutdown => false,
            };
            (cleanup_only && binding_matches)
                || wrap_up::departing_cleanup_allows(self, session_id, epoch, &request)
        } else {
            binding_matches || wrap_up::departing_cleanup_allows(self, session_id, epoch, &request)
        };
        if allowed {
            if let Some(ctrl) = ctrl_guard.as_ref() {
                let _ = ctrl.tx.send(Frame::ExecutionRequest { request });
            }
        } else {
            let _ = worker.tx.send(Frame::ExecutionReply {
                reply: Box::new(crate::execution_protocol::RuntimeReply {
                    session_id: session_id.to_owned(),
                    worker_epoch: epoch.to_owned(),
                    request_id: request.request_id,
                    scope: crate::execution_protocol::Scope::from_binding(&request.binding),
                    response: crate::machine_protocol::execution::Response::Refused {
                        reason: crate::machine_protocol::execution::Refusal::IdentityMismatch,
                    },
                }),
            });
        }
    }

    /// Publish refreshed broker queue count after rejecting queued prompts for a
    /// `Crashed` worker so Controller's sighting clears `pending_prompt_count`
    /// and does not defer automatic drain (R11-M1).
    pub(in crate::machine_broker) fn publish_snapshot_after_crashed_flush(
        &self,
        session_id: &str,
        removed_prompts_for_crashed: bool,
    ) {
        if !removed_prompts_for_crashed {
            return;
        }
        if let Some(worker) = self
            .snapshots()
            .into_iter()
            .find(|w| w.session_id == session_id && w.has_connected_owner())
        {
            self.send_controller(Frame::Snapshot {
                worker: Box::new(worker),
            });
        }
    }

    pub(super) fn spawn_unconfirmed_sandbox_drain_retry(
        self: &Arc<Self>,
        lifecycle_guard: tokio::sync::OwnedMutexGuard<()>,
        session_id: String,
        command_id: String,
        entry: UnconfirmedTeardown,
    ) {
        let broker = Arc::clone(self);
        tokio::spawn(async move {
            let _guard = lifecycle_guard;
            if !broker.teardown_retry_still_valid(&session_id, &entry.epoch) {
                broker.command_rejected(
                    &session_id,
                    command_id,
                    "session state changed during teardown retry".into(),
                );
                return;
            }
            let launch = broker
                .resolve_or_refresh_retired_launch(&session_id, &entry.epoch, entry.retired_launch)
                .await;
            if !broker
                .confirm_retired_teardown_complete(&session_id, entry.peer_pid, launch)
                .await
            {
                broker.command_rejected(
                    &session_id,
                    command_id,
                    "crashed worker teardown is still unconfirmed".into(),
                );
                return;
            }
            let (cleared, rejections) = {
                let workers = broker.workers.lock();
                let mut pending_cmds = broker.pending_commands.lock();
                let mut dispatched = broker.dispatched_prompts.lock();
                let mut state = broker.crashed_drains.inner.lock();
                if workers.contains_key(&session_id) {
                    (false, Vec::new())
                } else if state
                    .unconfirmed_teardown
                    .get(&session_id)
                    .is_some_and(|recorded| recorded.epoch == entry.epoch)
                {
                    state.unconfirmed_teardown.remove(&session_id);
                    state.trusted_epoch_pids.remove(&session_id);
                    let rejections = classify_and_drain_queued_commands(
                        &mut pending_cmds,
                        &mut dispatched,
                        &session_id,
                    );
                    (true, rejections)
                } else {
                    (false, Vec::new())
                }
            };
            for (cid, reason) in rejections {
                broker.command_rejected(&session_id, cid, reason);
            }
            if cleared {
                broker.send_controller(Frame::CommandAck {
                    session_id: session_id.clone(),
                    command_id,
                    accepted: true,
                    reason: None,
                });
                broker.send_crashed_worker_drained(&session_id, &entry.epoch);
                broker.crashed_drains.settled.notify_waiters();
            } else {
                broker.command_rejected(
                    &session_id,
                    command_id,
                    "crashed worker teardown retry was superseded".into(),
                );
            }
        });
    }

    pub(super) fn teardown_retry_still_valid(&self, session_id: &str, worker_epoch: &str) -> bool {
        !self
            .permanently_deleted_sessions
            .lock()
            .contains(session_id)
            && !self.cancelled_sessions.lock().contains(session_id)
            && !self.launching.lock().contains(session_id)
            && !self.resetting_sessions.lock().contains_key(session_id)
            && !self.workers.lock().contains_key(session_id)
            && self
                .crashed_drains
                .inner
                .lock()
                .unconfirmed_teardown
                .get(session_id)
                .is_some_and(|recorded| recorded.epoch == worker_epoch)
    }

    /// Epoch-bound cleanup continuation with capped backoff; always revalidates
    /// ownership under `session_lifecycle_gate` before calling the daemon (R2-H4, R2-M5).
    pub(super) fn spawn_background_teardown_retry(
        self: &Arc<Self>,
        session_id: String,
        entry: UnconfirmedTeardown,
    ) {
        let broker = Arc::clone(self);
        tokio::spawn(async move {
            let mut attempt: u32 = 0;
            let mut retired_launch = entry.retired_launch;
            loop {
                let delay = Duration::from_secs((2_u64 << attempt.min(5)).min(60));
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(delay).await;
                if !broker.teardown_retry_still_valid(&session_id, &entry.epoch) {
                    return;
                }
                let gate = broker.session_lifecycle_gate(&session_id);
                let _guard = gate.lock().await;
                if !broker.teardown_retry_still_valid(&session_id, &entry.epoch) {
                    return;
                }
                retired_launch = broker
                    .resolve_or_refresh_retired_launch(&session_id, &entry.epoch, retired_launch)
                    .await;
                if !broker
                    .confirm_retired_teardown_complete(&session_id, entry.peer_pid, retired_launch)
                    .await
                {
                    continue;
                }
                let (cleared, session_for_cache, rejections) = {
                    let sessions = broker.sessions.lock();
                    let workers = broker.workers.lock();
                    let mut pending_cmds = broker.pending_commands.lock();
                    let mut dispatched = broker.dispatched_prompts.lock();
                    let mut state = broker.crashed_drains.inner.lock();
                    if !workers.contains_key(&session_id)
                        && state
                            .unconfirmed_teardown
                            .get(&session_id)
                            .is_some_and(|recorded| recorded.epoch == entry.epoch)
                    {
                        state.unconfirmed_teardown.remove(&session_id);
                        state.trusted_epoch_pids.remove(&session_id);
                        let rejections = classify_and_drain_queued_commands(
                            &mut pending_cmds,
                            &mut dispatched,
                            &session_id,
                        );
                        (true, sessions.get(&session_id).cloned(), rejections)
                    } else {
                        (false, None, Vec::new())
                    }
                };
                for (cid, reason) in rejections {
                    broker.command_rejected(&session_id, cid, reason);
                }
                if cleared {
                    if let Some(session) = session_for_cache {
                        broker.revoke_cache_protection(
                            &session,
                            &session_id,
                            "crashed_worker_drained",
                        );
                    }
                    broker.send_crashed_worker_drained(&session_id, &entry.epoch);
                    broker.crashed_drains.settled.notify_waiters();
                }
                return;
            }
        });
    }

    /// Process a worker `CommandAck`. Returns `false` when the ACK came from a
    /// superseded connection for a tracked crashed-drain command (caller must
    /// NOT forward it to Controller).
    pub(super) async fn wait_for_direct_exit_then_confirm_sandbox(
        &self,
        session_id: &str,
        recorded_pid: Option<u32>,
        retired_launch: RetiredLaunch,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + DIRECT_EXIT_GRACE;
        while !self
            .confirm_mid_drain_worker_exited(session_id, recorded_pid)
            .await
        {
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        self.confirm_retired_sandbox_units_gone(session_id, retired_launch)
            .await
    }

    pub(super) fn spawn_receiptless_drain_reconciler(
        self: &Arc<Self>,
        session_id: String,
        worker_epoch: String,
        recorded_pid: Option<u32>,
        mut retired_launch: RetiredLaunch,
    ) {
        let broker = Arc::clone(self);
        tokio::spawn(async move {
            let mut attempt: u32 = 0;
            loop {
                let delay = Duration::from_secs((2_u64 << attempt.min(5)).min(60));
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(delay).await;
                let gate = broker.session_lifecycle_gate(&session_id);
                let _guard = gate.lock().await;
                if broker
                    .permanently_deleted_sessions
                    .lock()
                    .contains(&session_id)
                    || broker.cancelled_sessions.lock().contains(&session_id)
                    || broker.launching.lock().contains(&session_id)
                    || broker.resetting_sessions.lock().contains_key(&session_id)
                    || broker.workers.lock().contains_key(&session_id)
                {
                    return;
                }
                let still_tracked = {
                    let state = broker.crashed_drains.inner.lock();
                    state
                        .active
                        .get(&session_id)
                        .is_some_and(|a| a.epoch == worker_epoch && !a.confirmed)
                        || state
                            .pending
                            .values()
                            .any(|p| p.session_id == session_id && p.epoch == worker_epoch)
                };
                if !still_tracked {
                    return;
                }
                retired_launch = broker
                    .resolve_or_refresh_retired_launch(&session_id, &worker_epoch, retired_launch)
                    .await;
                if !broker
                    .confirm_mid_drain_worker_exited(&session_id, recorded_pid)
                    .await
                    || !broker
                        .confirm_retired_sandbox_units_gone(&session_id, retired_launch)
                        .await
                {
                    continue;
                }
                let orphaned = {
                    let workers = broker.workers.lock();
                    let mut state = broker.crashed_drains.inner.lock();
                    if workers.contains_key(&session_id) {
                        return;
                    }
                    if state
                        .active
                        .get(&session_id)
                        .is_some_and(|a| a.epoch == worker_epoch && !a.confirmed)
                    {
                        state.active.remove(&session_id);
                    }
                    state.trusted_epoch_pids.remove(&session_id);
                    drain_pending_for_epoch(&mut state.pending, &session_id, &worker_epoch)
                };
                for (sid, cid) in orphaned {
                    broker.command_rejected(
                        &sid,
                        cid,
                        "worker exited during crashed drain without completion receipt".into(),
                    );
                }
                broker.record_recently_drained(&session_id, &worker_epoch, false);
                broker.publish_ownerless_crashed_snapshot(&session_id);
                broker.crashed_drains.settled.notify_waiters();
                return;
            }
        });
    }

    pub(super) async fn current_durable_launch_id(
        &self,
        session_id: &str,
    ) -> Result<Option<uuid::Uuid>, String> {
        let store = Arc::clone(&self.args.provider_store);
        let session = session_id.to_owned();
        tokio::task::spawn_blocking(move || {
            store
                .account_worker_exit(&session)
                .map(|opt| opt.map(|receipt| receipt.launch))
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?
    }

    /// Strict Direct-mode exit proof: requires BOTH no live broker-tracked PID/tree,
    /// a known PID whose process + process group are gone (never skipping when
    /// `recorded_pid` is `None`!), AND durable containment `tree_exited` for named
    /// Direct launches (R3-H4).
    pub(super) async fn confirm_direct_owner_gone(
        &self,
        session_id: &str,
        recorded_pid: Option<u32>,
    ) -> bool {
        if self.direct_worker_pids.lock().contains_key(session_id)
            || self.direct_worker_trees.lock().contains(session_id)
        {
            return false;
        }
        if !recorded_pid.is_some_and(direct_process_tree_gone) {
            return false;
        }
        let store = Arc::clone(&self.args.provider_store);
        let session = session_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let _ = store.reconcile_account_worker_boot(&session);
            match store.account_worker_exit(&session) {
                Ok(Some(receipt)) => receipt.tree_exited,
                Ok(None) => true,
                Err(_) => false,
            }
        })
        .await
        .unwrap_or(false)
    }

    pub(super) async fn confirm_retired_teardown_complete(
        &self,
        session_id: &str,
        recorded_pid: Option<u32>,
        retired_launch: RetiredLaunch,
    ) -> bool {
        if !self
            .confirm_mid_drain_worker_exited(session_id, recorded_pid)
            .await
        {
            return false;
        }
        self.confirm_retired_sandbox_units_gone(session_id, retired_launch)
            .await
    }

    /// Scoped durable sandbox cleanup: binds cleanup strictly to `expected_launch`
    /// (R3-M3). If `expected_launch` is `Some(u)`, only cleans when the durable
    /// record STILL names `u`; if `None`, verifies no durable named launch exists.
    pub(super) async fn confirm_retired_sandbox_units_gone(
        &self,
        session_id: &str,
        expected_launch: RetiredLaunch,
    ) -> bool {
        let store = Arc::clone(&self.args.provider_store);
        let session = session_id.to_owned();
        let confirmed = tokio::task::spawn_blocking(move || match expected_launch {
            RetiredLaunch::Verified(expected) => {
                store.confirm_session_account_units_gone_for_launch(&session, expected)
            }
            RetiredLaunch::VerifiedNone => match store.account_worker_exit(&session)? {
                None => Ok(()),
                Some(_) => {
                    // A named launch now owns this session; do not stop its sandbox.
                    Ok(())
                }
            },
            RetiredLaunch::Unresolved => Err(anyhow::anyhow!(
                "retired durable launch identity is unresolved; failing sandbox teardown check closed"
            )),
        })
        .await;
        match confirmed {
            Ok(Ok(())) => true,
            outcome => {
                tracing::error!(
                    session = session_id,
                    ?outcome,
                    "retired account sandbox unit teardown unconfirmed"
                );
                false
            }
        }
    }

    pub(super) async fn confirm_mid_drain_worker_exited(
        &self,
        session_id: &str,
        recorded_pid: Option<u32>,
    ) -> bool {
        match self.args.spawn_mode {
            SpawnMode::SystemdUser => {
                if systemd_exit::confirm_replaced(&self.args.provider_store, session_id)
                    .await
                    .is_err()
                {
                    return false;
                }
                let unit = worker_unit_name(&self.args.socket, session_id);
                systemd_exit::unit_inactive(&unit).await.unwrap_or(false)
            }
            SpawnMode::Direct => {
                self.confirm_direct_owner_gone(session_id, recorded_pid)
                    .await
            }
        }
    }
    pub(super) fn record_recently_drained(
        &self,
        session_id: &str,
        worker_epoch: &str,
        confirmed_receipt: bool,
    ) {
        let mut state = self.crashed_drains.inner.lock();
        let now = Instant::now();
        state
            .recently_drained
            .retain(|_, (_, _, at)| now.saturating_duration_since(*at) < RECENTLY_DRAINED_TTL);
        if state.recently_drained.len() >= MAX_RECENTLY_DRAINED
            && let Some(oldest) = state
                .recently_drained
                .iter()
                .min_by_key(|(_, (_, _, at))| *at)
                .map(|(k, _)| k.clone())
        {
            state.recently_drained.remove(&oldest);
        }
        state.recently_drained.insert(
            session_id.to_owned(),
            (worker_epoch.to_owned(), confirmed_receipt, now),
        );
    }

    pub(super) fn publish_ownerless_crashed_snapshot(&self, session_id: &str) {
        let snapshot = {
            let sessions = self.sessions.lock();
            let Some(session) = sessions.get(session_id).cloned() else {
                return;
            };
            let workers = self.workers.lock();
            if workers.contains_key(session_id) {
                return;
            }
            let state = self
                .session_states
                .lock()
                .get(session_id)
                .copied()
                .unwrap_or(WorkerState::Crashed);
            let exit_detail = (state == WorkerState::Crashed)
                .then(|| self.startup_failures.lock().get(session_id).cloned())
                .flatten();
            let executable = self
                .generation_commands
                .lock()
                .get(&session.generation)
                .map(|path| path.display().to_string());
            let pending_prompt_count =
                self.pending_commands
                    .lock()
                    .get(session_id)
                    .map_or(0, |commands| {
                        u64::try_from(
                            commands
                                .iter()
                                .filter(|cmd| matches!(cmd, WorkerCommand::Prompt { .. }))
                                .count(),
                        )
                        .unwrap_or(u64::MAX)
                    });
            WorkerSnapshot {
                session_id: session_id.to_owned(),
                worker_epoch: format!("broker-{session_id}"),
                generation: session.generation.clone(),
                executable,
                launch: Some(session.clone()),
                state,
                agent_session_id: session.agent_session_id,
                native_thread_materialized: None,
                current_turn_id: None,
                supports_current_turn: false,
                delivery_controls: crate::runtime_wire::DeliveryControls::default(),
                wrap_up_steering: false,
                supports_subagent_cancel: false,
                last_runtime_seq: 0,
                pending_permissions: Vec::new(),
                config_options: None,
                context_used: None,
                context_size: None,
                pending_prompt_count,
                drain_requested: false,
                exit_detail,
                background_tasks: None,
                native_cycle_id: None,
                native_cycle_active: false,
                incarnation: self.reported_lineage(session_id),
                account_unit: None,
                uncertain_effect: self.retained_receiptless_uncertain(session_id),
                crashed_drain_active: false,
            }
        };
        self.send_controller(Frame::Snapshot {
            worker: Box::new(snapshot),
        });
    }

    pub(super) fn send_crashed_worker_drained(&self, session_id: &str, worker_epoch: &str) {
        self.record_recently_drained(session_id, worker_epoch, true);
        let controller_outcome = {
            let guard = self.controller.lock();
            guard.as_ref().map(|controller| {
                let capable = controller.protocol >= CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
                    && controller
                        .outer_machine_protocol
                        .is_some_and(|proto| proto >= CRASHED_WORKER_DRAIN_PROTOCOL_VERSION);
                (capable, controller.tx.clone())
            })
        };
        match controller_outcome {
            Some((true, tx)) => {
                let _ = tx.send(Frame::CrashedWorkerDrained {
                    session_id: session_id.to_owned(),
                    worker_epoch: worker_epoch.to_owned(),
                });
            }
            Some((false, _)) => {
                // Downgraded Controller cannot read CrashedWorkerDrained: publish
                // compatible ownerless snapshot so it drops the exited worker (R5-M4).
                self.publish_ownerless_crashed_snapshot(session_id);
            }
            None => {}
        }
    }

    /// Close the Controller handover gap: if a worker appeared in `welcome`
    /// and completed crashed-drain retirement before the new Controller lease
    /// was installed, replay `CrashedWorkerDrained` to the new lease.
    #[cfg(test)]
    pub(in crate::machine_broker) fn replay_recently_drained_after_welcome(
        &self,
        welcome: &[WorkerSnapshot],
        lease: u64,
    ) {
        let replays: Vec<(String, String)> = {
            let workers = self.workers.lock();
            let state = self.crashed_drains.inner.lock();
            welcome
                .iter()
                .filter(|snap| snap.has_connected_owner())
                .filter(|snap| !workers.contains_key(&snap.session_id))
                .filter_map(|snap| {
                    state
                        .recently_drained
                        .get(&snap.session_id)
                        .filter(|(epoch, confirmed, _)| *confirmed && epoch == &snap.worker_epoch)
                        .map(|(epoch, _, _)| (snap.session_id.clone(), epoch.clone()))
                })
                .collect()
        };
        if replays.is_empty() {
            return;
        }
        let tx = {
            let guard = self.controller.lock();
            guard.as_ref().and_then(|controller| {
                let capable = controller.lease == lease
                    && controller.protocol >= CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
                    && controller
                        .outer_machine_protocol
                        .is_some_and(|proto| proto >= CRASHED_WORKER_DRAIN_PROTOCOL_VERSION);
                capable.then(|| controller.tx.clone())
            })
        };
        if let Some(tx) = tx {
            for (session_id, worker_epoch) in replays {
                let _ = tx.send(Frame::CrashedWorkerDrained {
                    session_id,
                    worker_epoch,
                });
            }
        }
    }
}

fn direct_process_tree_gone(pid: u32) -> bool {
    let Some(pid_ref) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return false;
    };
    let process_gone = matches!(
        rustix::process::test_kill_process(pid_ref),
        Err(rustix::io::Errno::SRCH)
    );
    process_gone && !direct_worker_group_exists(pid)
}
