//! Teardown confirmation, launch-UUID-scoped sandbox cleanup, and
//! capped-backoff background reconciliation for cooperative crashed-worker drain.

use super::*;

impl Broker {
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
        retired_launch: Option<uuid::Uuid>,
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
        retired_launch: Option<uuid::Uuid>,
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
                    drain_pending_for_epoch(&mut state.pending, &session_id, &worker_epoch)
                };
                for (sid, cid) in orphaned {
                    broker.command_rejected(
                        &sid,
                        cid,
                        "worker exited during crashed drain without completion receipt".into(),
                    );
                }
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
        retired_launch: Option<uuid::Uuid>,
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
        expected_launch: Option<uuid::Uuid>,
    ) -> bool {
        let store = Arc::clone(&self.args.provider_store);
        let session = session_id.to_owned();
        let confirmed = tokio::task::spawn_blocking(move || match expected_launch {
            Some(expected) => match store.account_worker_exit(&session)? {
                Some(receipt) if receipt.launch == expected => {
                    store.confirm_session_account_units_gone(&session)
                }
                _ => Ok(()),
            },
            None => match store.account_worker_exit(&session)? {
                None => Ok(()),
                Some(_) => {
                    // A named launch now owns this session; do not stop its sandbox.
                    Ok(())
                }
            },
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
