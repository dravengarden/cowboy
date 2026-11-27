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
        uncertain_effect: crate::runtime_wire::UncertainEffect,
    ) {
        let (updated_snapshot, cleared_fence) = {
            let mut workers = self.workers.lock();
            let Some(worker) = workers
                .get_mut(session_id)
                .filter(|w| w.connection_id == connection_id && w.epoch == epoch)
            else {
                return;
            };
            let mut state = self.crashed_drains.inner.lock();
            let matched_active = state.active.get(session_id).is_some_and(|a| {
                !a.confirmed && a.epoch == epoch && a.connection_id == connection_id
            });
            let matched_pending = command_id.is_some_and(|cid| {
                state.pending.get(cid).is_some_and(|p| {
                    p.session_id == session_id
                        && p.epoch == epoch
                        && p.connection_id == connection_id
                })
            });
            // Idempotent for restated aborts after reconnect/broker-restart
            // (R7-M1): still merge a newer uncertain_effect onto the matching
            // live worker snapshot even when active/pending drain was already
            // cleared on an earlier abort.
            let prev_effect = worker.snapshot.uncertain_effect.clone();
            crate::runtime_wire::merge_uncertain_effect(
                &mut worker.snapshot.uncertain_effect,
                uncertain_effect,
            );
            if !matched_active && !matched_pending {
                if worker.snapshot.uncertain_effect == prev_effect {
                    return;
                }
                (worker.snapshot.clone(), false)
            } else {
                if matched_active {
                    state.active.remove(session_id);
                }
                if let Some(cid) = command_id {
                    state.pending.remove(cid);
                }
                let still_pending = state
                    .pending
                    .values()
                    .any(|p| p.session_id == session_id && p.epoch == epoch);
                let cleared = if !still_pending && !state.active.contains_key(session_id) {
                    worker.crashed_drain_fenced = false;
                    true
                } else {
                    false
                };
                (worker.snapshot.clone(), cleared)
            }
        };
        self.send_controller(Frame::Snapshot {
            worker: Box::new(updated_snapshot),
        });
        if cleared_fence {
            self.flush_pending(session_id);
        }
        self.crashed_drains.settled.notify_waiters();
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

    pub(in crate::machine_broker) fn retained_receiptless_uncertain(
        &self,
        session_id: &str,
    ) -> Option<crate::runtime_wire::UncertainEffect> {
        self.crashed_drains
            .inner
            .lock()
            .receiptless_uncertain_effects
            .get(session_id)
            .map(|(_, eff)| eff.clone())
    }

    /// Enqueue changed snapshots after `Welcome` under the exact
    /// `controller -> sessions -> workers -> crashed_drains` ordering boundary
    /// so a retired worker's ownerless fallback can never be overwritten by a
    /// stale connected catch-up snapshot on a downgraded Controller (R6-M4).
    pub(in crate::machine_broker) fn send_handover_catchup_snapshots(
        &self,
        welcome: &[WorkerSnapshot],
        lease: u64,
    ) {
        let controller = self.controller.lock();
        let Some(ctrl) = controller.as_ref().filter(|c| c.lease == lease) else {
            return;
        };
        let tx = ctrl.tx.clone();
        let outer = ctrl.outer_machine_protocol;
        let protocol = ctrl.protocol;
        let sessions = self.sessions.lock();
        let workers = self.workers.lock();
        let pending_prompts: HashMap<String, u64> = self
            .pending_commands
            .lock()
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
            if has_drain {
                snap.drain_requested = true;
            }
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
        // Emit ownerless `broker-<session_id>` corrections for live Welcome
        // workers that retired before this Controller lease installed, when
        // the Controller will NOT receive a matching `CrashedWorkerDrained`
        // replay (for example a downgraded Controller or receiptless exit,
        // R7-M3).
        let controller_capable = protocol >= CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
            && outer.is_some_and(|p| p >= CRASHED_WORKER_DRAIN_PROTOCOL_VERSION);
        for welcomed in welcome {
            if !welcomed.has_connected_owner() || workers.contains_key(&welcomed.session_id) {
                continue;
            }
            let replayed_drained = controller_capable
                && state
                    .recently_drained
                    .get(&welcomed.session_id)
                    .is_some_and(|(ep, _)| ep == &welcomed.worker_epoch);
            if replayed_drained {
                continue;
            }
            let session = sessions.get(&welcomed.session_id);
            let mut ownerless = WorkerSnapshot {
                session_id: welcomed.session_id.clone(),
                worker_epoch: format!("broker-{}", welcomed.session_id),
                generation: session
                    .map(|s| s.generation.clone())
                    .unwrap_or_else(|| welcomed.generation.clone()),
                executable: welcomed.executable.clone(),
                launch: session.cloned().or_else(|| welcomed.launch.clone()),
                state: WorkerState::Crashed,
                agent_session_id: session
                    .and_then(|s| s.agent_session_id.clone())
                    .or_else(|| welcomed.agent_session_id.clone()),
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
                pending_prompt_count: pending_prompts
                    .get(&welcomed.session_id)
                    .copied()
                    .unwrap_or(0),
                drain_requested: false,
                exit_detail: welcomed.exit_detail.clone(),
                background_tasks: None,
                native_cycle_id: None,
                native_cycle_active: false,
                incarnation: self.reported_lineage(&welcomed.session_id),
                account_unit: None,
                uncertain_effect: state
                    .receiptless_uncertain_effects
                    .get(&welcomed.session_id)
                    .map(|(_, eff)| eff.clone())
                    .or_else(|| welcomed.uncertain_effect.clone()),
            };
            let declared_provider = session.map(|s| s.provider.as_str());
            wrap_up::sanitize_snapshot_for_controller(
                &mut ownerless,
                false,
                protocol,
                outer,
                protocol,
                declared_provider,
            );
            let _ = tx.send(Frame::Snapshot {
                worker: Box::new(ownerless),
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
            if !broker
                .confirm_retired_teardown_complete(
                    &session_id,
                    entry.peer_pid,
                    entry.retired_launch,
                )
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
                if !broker
                    .confirm_retired_teardown_complete(
                        &session_id,
                        entry.peer_pid,
                        entry.retired_launch,
                    )
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
        if self.args.spawn_mode == SpawnMode::Direct {
            let deadline = tokio::time::Instant::now() + DIRECT_EXIT_GRACE;
            while !self
                .confirm_direct_owner_gone(session_id, recorded_pid)
                .await
            {
                if tokio::time::Instant::now() >= deadline {
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
        self.confirm_retired_sandbox_units_gone(session_id, retired_launch)
            .await
    }

    pub(super) fn spawn_receiptless_drain_reconciler(
        self: &Arc<Self>,
        session_id: String,
        worker_epoch: String,
        recorded_pid: Option<u32>,
        retired_launch: RetiredLaunch,
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
        if self.args.spawn_mode == SpawnMode::Direct
            && !self
                .confirm_direct_owner_gone(session_id, recorded_pid)
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
