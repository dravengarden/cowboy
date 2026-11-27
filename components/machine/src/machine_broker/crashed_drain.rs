//! Cooperative epoch-bound retirement of idle workers in `WorkerState::Crashed`.
//!
//! All state lives under a single leaf mutex (`State`) acquired only AFTER any
//! broker locks (`cancelled_sessions` -> `sessions` -> `workers` ->
//! `pending_commands` -> `dispatched_prompts` -> `crashed_drains`) and never
//! held across `.await` or `send_controller`. Lifecycle mutations serialize on
//! the broker's existing `session_lifecycle_gate(session_id)`.

use super::*;
use crate::machine_protocol::CRASHED_WORKER_DRAIN_PROTOCOL_VERSION;
use crate::runtime_wire::CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION;

#[derive(Default)]
pub(super) struct State(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    pending: HashMap<String, PendingDrain>,
    active: HashMap<String, ActiveDrain>,
    unconfirmed_sandbox: HashMap<String, String>,
}

struct PendingDrain {
    session_id: String,
    epoch: String,
    connection_id: u64,
}

struct ActiveDrain {
    epoch: String,
    connection_id: u64,
    confirmed: bool,
}

enum DrainAdmission {
    Forwarded,
    RetryUnconfirmedSandbox,
    Rejected(String),
}

impl Broker {
    pub(super) async fn drain_crashed_worker(
        self: &Arc<Self>,
        session_id: String,
        command_id: String,
        worker_epoch: String,
    ) {
        if self
            .permanently_deleted_sessions
            .lock()
            .contains(&session_id)
            || self.cancelled_sessions.lock().contains(&session_id)
        {
            self.command_rejected(&session_id, command_id, "session was deleted".into());
            return;
        }
        let gate = self.session_lifecycle_gate(&session_id);
        let Ok(lifecycle_guard) = gate.try_lock_owned() else {
            self.command_rejected(
                &session_id,
                command_id,
                "session lifecycle operation is in progress".into(),
            );
            return;
        };
        if self.launching.lock().contains(&session_id)
            || self.resetting_sessions.lock().contains_key(&session_id)
        {
            self.command_rejected(
                &session_id,
                command_id,
                "session launch or reset is in progress".into(),
            );
            return;
        }
        match self.admit_crashed_drain(&session_id, &command_id, &worker_epoch) {
            DrainAdmission::Forwarded => {}
            DrainAdmission::Rejected(reason) => {
                self.command_rejected(&session_id, command_id, reason);
            }
            DrainAdmission::RetryUnconfirmedSandbox => {
                self.spawn_unconfirmed_sandbox_drain_retry(
                    lifecycle_guard,
                    session_id,
                    command_id,
                    worker_epoch,
                );
            }
        }
    }

    fn admit_crashed_drain(
        &self,
        session_id: &str,
        command_id: &str,
        worker_epoch: &str,
    ) -> DrainAdmission {
        let mut workers = self.workers.lock();
        let pending_cmds = self.pending_commands.lock();
        let mut state = self.crashed_drains.0.lock();
        match workers.get_mut(session_id) {
            None => {
                if state
                    .unconfirmed_sandbox
                    .get(session_id)
                    .is_some_and(|epoch| epoch == worker_epoch)
                {
                    DrainAdmission::RetryUnconfirmedSandbox
                } else {
                    DrainAdmission::Rejected("no live worker attached for session".into())
                }
            }
            Some(worker) if worker.epoch != worker_epoch => {
                DrainAdmission::Rejected("worker epoch changed before crashed drain".into())
            }
            Some(worker) if !worker.welcomed => {
                DrainAdmission::Rejected("worker handshake is not complete".into())
            }
            Some(worker)
                if worker.negotiated_protocol < CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION =>
            {
                DrainAdmission::Rejected("worker protocol does not support crashed drain".into())
            }
            Some(worker) if worker.crashed_drain_fenced => {
                DrainAdmission::Rejected("crashed worker drain already in progress".into())
            }
            Some(worker) if worker.snapshot.state != WorkerState::Crashed => {
                DrainAdmission::Rejected("worker is not in Crashed state".into())
            }
            Some(worker) if !worker_snapshot_idle_for_crashed_drain(&worker.snapshot) => {
                DrainAdmission::Rejected("crashed worker still has active work".into())
            }
            Some(_)
                if pending_cmds
                    .get(session_id)
                    .is_some_and(|queue| !queue.is_empty()) =>
            {
                DrainAdmission::Rejected("session has queued broker commands".into())
            }
            Some(worker) => {
                let frame = Frame::WorkerCommand {
                    session_id: session_id.to_owned(),
                    command: WorkerCommand::DrainCrashed {
                        command_id: command_id.to_owned(),
                        worker_epoch: worker_epoch.to_owned(),
                    },
                };
                if worker.tx.send(frame).is_ok() {
                    worker.crashed_drain_fenced = true;
                    state.pending.insert(
                        command_id.to_owned(),
                        PendingDrain {
                            session_id: session_id.to_owned(),
                            epoch: worker_epoch.to_owned(),
                            connection_id: worker.connection_id,
                        },
                    );
                    DrainAdmission::Forwarded
                } else {
                    DrainAdmission::Rejected(
                        "worker disconnected before crashed drain handoff".into(),
                    )
                }
            }
        }
    }

    fn spawn_unconfirmed_sandbox_drain_retry(
        self: &Arc<Self>,
        lifecycle_guard: tokio::sync::OwnedMutexGuard<()>,
        session_id: String,
        command_id: String,
        worker_epoch: String,
    ) {
        let broker = Arc::clone(self);
        tokio::spawn(async move {
            let _guard = lifecycle_guard;
            if broker
                .permanently_deleted_sessions
                .lock()
                .contains(&session_id)
                || broker.cancelled_sessions.lock().contains(&session_id)
                || broker.launching.lock().contains(&session_id)
                || broker.resetting_sessions.lock().contains_key(&session_id)
            {
                broker.command_rejected(
                    &session_id,
                    command_id,
                    "session state changed during sandbox teardown retry".into(),
                );
                return;
            }
            let gone = broker
                .confirm_durable_session_account_units_gone(&session_id)
                .await;
            if !gone {
                broker.command_rejected(
                    &session_id,
                    command_id,
                    "crashed worker sandbox unit teardown is still unconfirmed".into(),
                );
                return;
            }
            let cleared = {
                let workers = broker.workers.lock();
                let mut state = broker.crashed_drains.0.lock();
                if workers.contains_key(&session_id) {
                    false
                } else if state
                    .unconfirmed_sandbox
                    .get(&session_id)
                    .is_some_and(|epoch| epoch == &worker_epoch)
                {
                    state.unconfirmed_sandbox.remove(&session_id);
                    true
                } else {
                    false
                }
            };
            if cleared {
                broker.send_controller(Frame::CommandAck {
                    session_id: session_id.clone(),
                    command_id,
                    accepted: true,
                    reason: None,
                });
                broker.send_crashed_worker_drained(&session_id, &worker_epoch);
            } else {
                broker.command_rejected(
                    &session_id,
                    command_id,
                    "crashed worker sandbox retry was superseded".into(),
                );
            }
        });
    }

    pub(super) fn record_crashed_drain_ack(
        &self,
        session_id: &str,
        epoch: &str,
        connection_id: u64,
        command_id: &str,
        accepted: bool,
    ) {
        let mut workers = self.workers.lock();
        let mut state = self.crashed_drains.0.lock();
        let Some(entry) = state.pending.get(command_id) else {
            return;
        };
        if entry.session_id != session_id || entry.epoch != epoch {
            return;
        }
        state.pending.remove(command_id);
        let worker_matches = workers
            .get(session_id)
            .is_some_and(|worker| worker.epoch == epoch && worker.connection_id == connection_id);
        if accepted {
            let already_confirmed = state.active.get(session_id).is_some_and(|active| {
                active.epoch == epoch && active.connection_id == connection_id && active.confirmed
            });
            state.active.insert(
                session_id.to_owned(),
                ActiveDrain {
                    epoch: epoch.to_owned(),
                    connection_id,
                    confirmed: already_confirmed,
                },
            );
            if worker_matches && let Some(worker) = workers.get_mut(session_id) {
                worker.crashed_drain_fenced = true;
            }
        } else {
            let still_active = state.active.get(session_id).is_some_and(|active| {
                active.epoch == epoch && active.connection_id == connection_id
            });
            let other_pending = state
                .pending
                .values()
                .any(|pending| pending.session_id == session_id && pending.epoch == epoch);
            if !still_active
                && !other_pending
                && worker_matches
                && let Some(worker) = workers.get_mut(session_id)
            {
                worker.crashed_drain_fenced = false;
            }
        }
    }

    pub(super) fn confirm_crashed_drain(
        &self,
        session_id: &str,
        epoch: &str,
        connection_id: u64,
        command_id: Option<&str>,
    ) {
        let mut workers = self.workers.lock();
        let mut state = self.crashed_drains.0.lock();
        let Some(worker) = workers.get_mut(session_id) else {
            return;
        };
        if worker.epoch != epoch || worker.connection_id != connection_id {
            return;
        }
        let authorized = worker.crashed_drain_fenced
            || state.active.get(session_id).is_some_and(|active| {
                active.epoch == epoch && active.connection_id == connection_id
            })
            || command_id.is_some_and(|id| {
                state.pending.get(id).is_some_and(|pending| {
                    pending.session_id == session_id
                        && pending.epoch == epoch
                        && pending.connection_id == connection_id
                })
            });
        if !authorized {
            tracing::warn!(
                session = session_id,
                %epoch,
                "ignoring unsolicited CrashedDrainCompleted frame"
            );
            return;
        }
        worker.crashed_drain_fenced = true;
        state.active.insert(
            session_id.to_owned(),
            ActiveDrain {
                epoch: epoch.to_owned(),
                connection_id,
                confirmed: true,
            },
        );
    }

    /// Rebind same-epoch drain state when a worker reconnects, or clear stale
    /// drain state when a new epoch connects. Must be called while holding
    /// `self.workers.lock()`; caller rejects returned `(session_id, command_id)`
    /// entries after releasing locks.
    pub(super) fn rebind_crashed_drain_on_reconnect(
        &self,
        session_id: &str,
        epoch: &str,
        connection_id: u64,
        worker: &mut WorkerPeer,
    ) -> Vec<(String, String)> {
        let mut state = self.crashed_drains.0.lock();
        let mut same_epoch = false;
        if let Some(active) = state.active.get_mut(session_id)
            && active.epoch == epoch
        {
            active.connection_id = connection_id;
            same_epoch = true;
        }
        for pending in state.pending.values_mut() {
            if pending.session_id == session_id && pending.epoch == epoch {
                pending.connection_id = connection_id;
                same_epoch = true;
            }
        }
        if same_epoch {
            worker.crashed_drain_fenced = true;
            return Vec::new();
        }
        state.active.remove(session_id);
        state.unconfirmed_sandbox.remove(session_id);
        drain_pending_for_session(&mut state.pending, session_id)
    }

    /// Handle disconnect of a crashed worker that already sent
    /// `CrashedDrainCompleted`. Returns `true` when this path owned the
    /// disconnect and suppressed normal `Crashed` snapshot re-emission.
    pub(super) async fn try_finish_crashed_drain_disconnect(
        self: &Arc<Self>,
        session_id: &str,
        peer: &WorkerPeer,
    ) -> bool {
        let (confirmed, orphaned_pending) = {
            let mut state = self.crashed_drains.0.lock();
            let confirmed = state.active.get(session_id).is_some_and(|active| {
                active.epoch == peer.epoch
                    && active.connection_id == peer.connection_id
                    && active.confirmed
            });
            let orphaned = if confirmed {
                Vec::new()
            } else {
                drain_pending_for_connection(
                    &mut state.pending,
                    session_id,
                    &peer.epoch,
                    peer.connection_id,
                )
            };
            (confirmed, orphaned)
        };
        for (sid, cid) in orphaned_pending {
            self.command_rejected(
                &sid,
                cid,
                "worker disconnected before crashed drain completed".into(),
            );
        }
        if !confirmed {
            return false;
        }

        let gate = self.session_lifecycle_gate(session_id);
        let _guard = gate.lock().await;

        let still_owned = !self
            .permanently_deleted_sessions
            .lock()
            .contains(session_id)
            && !self.cancelled_sessions.lock().contains(session_id)
            && !self.launching.lock().contains(session_id)
            && !self.resetting_sessions.lock().contains_key(session_id)
            && !self.workers.lock().contains_key(session_id)
            && self
                .crashed_drains
                .0
                .lock()
                .active
                .get(session_id)
                .is_some_and(|active| {
                    active.epoch == peer.epoch && active.connection_id == peer.connection_id
                });
        if !still_owned {
            let orphaned = {
                let mut state = self.crashed_drains.0.lock();
                if state.active.get(session_id).is_some_and(|active| {
                    active.epoch == peer.epoch && active.connection_id == peer.connection_id
                }) {
                    state.active.remove(session_id);
                }
                drain_pending_for_connection(
                    &mut state.pending,
                    session_id,
                    &peer.epoch,
                    peer.connection_id,
                )
            };
            for (sid, cid) in orphaned {
                self.command_rejected(&sid, cid, "crashed worker drain superseded".into());
            }
            return true;
        }

        let unit_gone = self
            .confirm_durable_session_account_units_gone(session_id)
            .await;
        self.commit_crashed_drain_disconnect(session_id, peer, unit_gone);
        true
    }

    fn commit_crashed_drain_disconnect(
        &self,
        session_id: &str,
        peer: &WorkerPeer,
        unit_gone: bool,
    ) {
        let (commit, session_for_cache, orphaned_pending, drained_commands) = {
            let cancelled = self.cancelled_sessions.lock();
            let deleted = self.permanently_deleted_sessions.lock();
            let sessions = self.sessions.lock();
            let workers = self.workers.lock();
            let mut pending_cmds = self.pending_commands.lock();
            let mut dispatched = self.dispatched_prompts.lock();
            let mut state = self.crashed_drains.0.lock();

            let valid = !cancelled.contains(session_id)
                && !deleted.contains(session_id)
                && !workers.contains_key(session_id)
                && state.active.get(session_id).is_some_and(|active| {
                    active.epoch == peer.epoch && active.connection_id == peer.connection_id
                });
            if !valid {
                if state.active.get(session_id).is_some_and(|active| {
                    active.epoch == peer.epoch && active.connection_id == peer.connection_id
                }) {
                    state.active.remove(session_id);
                }
                let orphaned = drain_pending_for_connection(
                    &mut state.pending,
                    session_id,
                    &peer.epoch,
                    peer.connection_id,
                );
                (false, None, orphaned, Vec::new())
            } else {
                state.active.remove(session_id);
                let orphaned = drain_pending_for_connection(
                    &mut state.pending,
                    session_id,
                    &peer.epoch,
                    peer.connection_id,
                );
                let drained_cmds = pending_cmds
                    .remove(session_id)
                    .map(|queue| queue.into_iter().collect::<Vec<_>>())
                    .unwrap_or_default();
                for cmd in &drained_cmds {
                    if let Some(cid) = worker_command_id(cmd) {
                        dispatched.mark_acked(session_id, cid);
                    }
                }
                if unit_gone {
                    state.unconfirmed_sandbox.remove(session_id);
                    (
                        true,
                        sessions.get(session_id).cloned(),
                        orphaned,
                        drained_cmds,
                    )
                } else {
                    state
                        .unconfirmed_sandbox
                        .insert(session_id.to_owned(), peer.epoch.clone());
                    (true, None, orphaned, drained_cmds)
                }
            }
        };

        for (sid, cid) in orphaned_pending {
            self.command_rejected(
                &sid,
                cid,
                "crashed worker exited before acknowledging drain".into(),
            );
        }
        for cmd in drained_commands {
            if let Some(cid) = worker_command_id(&cmd) {
                self.command_rejected(
                    session_id,
                    cid.to_owned(),
                    "unhanded: crashed worker drained; retry on replacement".into(),
                );
            }
        }
        if !commit {
            return;
        }
        self.session_states
            .lock()
            .insert(session_id.to_owned(), WorkerState::Crashed);
        self.awaiting_reconnect.lock().remove(session_id);
        self.replacing.lock().remove(session_id);
        if unit_gone {
            if let Some(session) = session_for_cache {
                self.revoke_cache_protection(&session, session_id, "crashed_worker_drained");
            }
            self.send_crashed_worker_drained(session_id, &peer.epoch);
        }
    }

    /// Gate explicit non-`adopt_only` `EnsureSession` under
    /// `session_lifecycle_gate` so a live draining crashed worker is never
    /// ripped out mid-flush and an unconfirmed exit or sandbox teardown is
    /// reconciled before spawning a replacement.
    pub(super) async fn gate_ensure_against_crashed_drain(
        &self,
        session_id: &str,
    ) -> Result<(), String> {
        let (fenced_live, has_live, active_unconfirmed, unconfirmed_epoch) = {
            let workers = self.workers.lock();
            let state = self.crashed_drains.0.lock();
            let live = workers.get(session_id);
            (
                live.is_some_and(|worker| worker.crashed_drain_fenced),
                live.is_some(),
                state.active.contains_key(session_id),
                state.unconfirmed_sandbox.get(session_id).cloned(),
            )
        };
        if fenced_live {
            return Err("crashed worker drain is in progress; retry".into());
        }
        if has_live {
            return Ok(());
        }
        if active_unconfirmed {
            if !self.confirm_mid_drain_worker_exited(session_id).await {
                return Err("crashed worker drain is still settling; retry".into());
            }
            let orphaned = {
                let workers = self.workers.lock();
                let mut state = self.crashed_drains.0.lock();
                if workers.contains_key(session_id) {
                    return Err("crashed worker reconnected during drain; retry".into());
                }
                state.active.remove(session_id);
                drain_pending_for_session(&mut state.pending, session_id)
            };
            for (sid, cid) in orphaned {
                self.command_rejected(&sid, cid, "crashed worker exited during drain".into());
            }
        }
        if let Some(epoch) = unconfirmed_epoch {
            if !self
                .confirm_durable_session_account_units_gone(session_id)
                .await
            {
                return Err(
                    "previous crashed worker sandbox unit teardown is unconfirmed; retry".into(),
                );
            }
            let emit = {
                let workers = self.workers.lock();
                let mut state = self.crashed_drains.0.lock();
                if !workers.contains_key(session_id)
                    && state
                        .unconfirmed_sandbox
                        .get(session_id)
                        .is_some_and(|recorded| recorded == &epoch)
                {
                    state.unconfirmed_sandbox.remove(session_id);
                    true
                } else {
                    false
                }
            };
            if emit {
                self.send_crashed_worker_drained(session_id, &epoch);
            }
        }
        Ok(())
    }

    async fn confirm_mid_drain_worker_exited(&self, session_id: &str) -> bool {
        let store = Arc::clone(&self.args.provider_store);
        let session = session_id.to_owned();
        let account_exited = tokio::task::spawn_blocking(move || {
            let _ = store.reconcile_account_worker_boot(&session);
            match store.account_worker_exit(&session) {
                Ok(Some(receipt)) => receipt.tree_exited,
                Ok(None) => true,
                Err(_) => false,
            }
        })
        .await
        .unwrap_or(false);
        if !account_exited {
            return false;
        }
        match self.args.spawn_mode {
            SpawnMode::Direct => !self.direct_worker_pids.lock().contains_key(session_id),
            SpawnMode::SystemdUser => {
                let unit = worker_unit_name(&self.args.socket, session_id);
                systemd_exit::unit_inactive(&unit).await.unwrap_or(false)
            }
        }
    }

    pub(super) fn clear_crashed_drain(&self, session_id: &str, reason: &str) {
        let orphaned = {
            let mut state = self.crashed_drains.0.lock();
            state.active.remove(session_id);
            state.unconfirmed_sandbox.remove(session_id);
            drain_pending_for_session(&mut state.pending, session_id)
        };
        for (sid, cid) in orphaned {
            self.command_rejected(&sid, cid, reason.to_owned());
        }
    }

    pub(super) fn send_crashed_worker_drained(&self, session_id: &str, worker_epoch: &str) {
        let tx = {
            let guard = self.controller.lock();
            guard.as_ref().and_then(|controller| {
                let capable = controller.protocol >= CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
                    && controller
                        .outer_machine_protocol
                        .is_some_and(|proto| proto >= CRASHED_WORKER_DRAIN_PROTOCOL_VERSION);
                capable.then(|| controller.tx.clone())
            })
        };
        if let Some(tx) = tx {
            let _ = tx.send(Frame::CrashedWorkerDrained {
                session_id: session_id.to_owned(),
                worker_epoch: worker_epoch.to_owned(),
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn new_registered_peer(
    session_id: &str,
    epoch: &str,
    generation: &str,
    executable: Option<String>,
    connection_id: u64,
    tx: mpsc::UnboundedSender<Frame>,
    welcomed: bool,
    drain_requested: bool,
) -> WorkerPeer {
    WorkerPeer {
        connection_id,
        epoch: epoch.to_owned(),
        tx,
        snapshot: WorkerSnapshot {
            session_id: session_id.to_owned(),
            worker_epoch: epoch.to_owned(),
            generation: generation.to_owned(),
            executable,
            launch: None,
            state: crate::runtime_wire::WorkerState::Starting,
            agent_session_id: None,
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
            pending_prompt_count: 0,
            drain_requested,
            exit_detail: None,
            background_tasks: None,
            native_cycle_id: None,
            native_cycle_active: false,
            incarnation: None,
            account_unit: None,
            uncertain_effect: None,
        },
        acked_runtime_seq: 0,
        earliest_unacked_seq: None,
        highest_runtime_seq: 0,
        replay_gate: None,
        welcomed,
        replay_deferred: false,
        last_seen: Instant::now(),
        receive_probe: None,
        liveness: None,
        negotiated_protocol: 0,
        raw_wrap_up_steering: false,
        hint_tx: None,
        diag_reply_tx: None,
        last_hinted_target: None,
        crashed_drain_fenced: false,
    }
}

fn worker_snapshot_idle_for_crashed_drain(snapshot: &WorkerSnapshot) -> bool {
    snapshot.current_turn_id.is_none()
        && snapshot.pending_permissions.is_empty()
        && snapshot.background_tasks.unwrap_or(0) == 0
        && !snapshot.native_cycle_active
        && snapshot.native_cycle_id.is_none()
        && snapshot.pending_prompt_count == 0
}

fn drain_pending_for_session(
    pending: &mut HashMap<String, PendingDrain>,
    session_id: &str,
) -> Vec<(String, String)> {
    let ids: Vec<String> = pending
        .iter()
        .filter(|(_, entry)| entry.session_id == session_id)
        .map(|(id, _)| id.clone())
        .collect();
    ids.into_iter()
        .filter_map(|id| pending.remove(&id).map(|entry| (entry.session_id, id)))
        .collect()
}

fn drain_pending_for_connection(
    pending: &mut HashMap<String, PendingDrain>,
    session_id: &str,
    epoch: &str,
    connection_id: u64,
) -> Vec<(String, String)> {
    let ids: Vec<String> = pending
        .iter()
        .filter(|(_, entry)| {
            entry.session_id == session_id
                && entry.epoch == epoch
                && entry.connection_id == connection_id
        })
        .map(|(id, _)| id.clone())
        .collect();
    ids.into_iter()
        .filter_map(|id| pending.remove(&id).map(|entry| (entry.session_id, id)))
        .collect()
}

pub(super) fn enqueue_pending_prompt(
    pending: &mut HashMap<String, VecDeque<WorkerCommand>>,
    session_id: &str,
    command_id: &str,
    command: WorkerCommand,
) -> Option<(String, String)> {
    let queue = pending.entry(session_id.to_owned()).or_default();
    if !queue
        .iter()
        .any(|queued| super::worker_command_id(queued) == Some(command_id))
    {
        queue.push_back(command);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_broker(dir: &Path) -> Arc<Broker> {
        let mut args = tests_support::broker_args(dir);
        args.spawn_mode = SpawnMode::Direct;
        Arc::new(Broker::new(args))
    }

    mod tests_support {
        use super::*;

        pub(super) fn broker_args(dir: &Path) -> MachineBrokerArgs {
            MachineBrokerArgs {
                socket: dir.join("broker.sock"),
                worker_command: PathBuf::from("/bin/true"),
                desired_generation: "gen-1".into(),
                spawn_mode: SpawnMode::Direct,
                worker_environment: BTreeMap::new(),
                provider_store: crate::machine_broker::tests::test_provider_store(),
                worktree_root: crate::session_workspace::WorktreeRoots::single(
                    dir.join("worktrees"),
                ),
                worker_ready_timeout: Duration::from_secs(2),
                call_context_root: None,
                memory_relay: None,
            }
        }
    }

    fn attach_controller(
        broker: &Broker,
        protocol: u16,
        outer_machine_protocol: Option<u16>,
    ) -> mpsc::UnboundedReceiver<Frame> {
        let (tx, rx) = mpsc::unbounded_channel();
        *broker.controller.lock() = Some(Controller {
            lease: 1,
            tx,
            protocol,
            outer_machine_protocol,
        });
        rx
    }

    fn attach_crashed_worker(
        broker: &Broker,
        session_id: &str,
        epoch: &str,
        connection_id: u64,
        protocol: u16,
    ) -> mpsc::UnboundedReceiver<Frame> {
        let (tx, rx) = mpsc::unbounded_channel();
        broker
            .register_worker_inner(
                WorkerRegistration {
                    session_id: session_id.into(),
                    epoch: epoch.into(),
                    generation: "gen-1".into(),
                    executable: None,
                    fallback_for: None,
                    connection_id,
                    tx,
                },
                true,
            )
            .unwrap();
        let mut workers = broker.workers.lock();
        let peer = workers.get_mut(session_id).unwrap();
        peer.negotiated_protocol = protocol;
        peer.snapshot.state = WorkerState::Crashed;
        peer.snapshot.background_tasks = Some(0);
        peer.snapshot.pending_prompt_count = 0;
        drop(workers);
        broker.sessions.lock().insert(
            session_id.into(),
            StartSession {
                session_id: session_id.into(),
                provider: "codex".into(),
                cwd: "/tmp".into(),
                generation: "gen-1".into(),
                ..serde_json::from_value(serde_json::json!({
                    "session_id": session_id,
                    "provider": "codex",
                    "cwd": "/tmp",
                    "generation": "gen-1"
                }))
                .unwrap()
            },
        );
        rx
    }

    #[tokio::test]
    async fn full_crashed_drain_fences_prompts_and_emits_retired_notice_on_disconnect() {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        let mut ctrl_rx = attach_controller(
            &broker,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
            Some(CRASHED_WORKER_DRAIN_PROTOCOL_VERSION),
        );
        let mut worker_rx = attach_crashed_worker(
            &broker,
            "s-1",
            "100-1",
            1,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );

        broker
            .drain_crashed_worker("s-1".into(), "dc-1".into(), "100-1".into())
            .await;

        assert!(matches!(
            worker_rx.try_recv(),
            Ok(Frame::WorkerCommand {
                command: WorkerCommand::DrainCrashed { ref command_id, ref worker_epoch },
                ..
            }) if command_id == "dc-1" && worker_epoch == "100-1"
        ));
        assert!(broker.workers.lock()["s-1"].crashed_drain_fenced);

        broker.route_prompt(
            "s-1",
            WorkerCommand::Prompt {
                command_id: "p-1".into(),
                turn_id: "t-1".into(),
                content: vec![serde_json::json!({"type": "text", "text": "hello"})],
                echo_artifacts: false,
                observe_subagents: false,
                cmid: None,
                trace: None,
                expected_worker_epoch: None,
                acknowledged_uncertain_sequence: None,
            },
        );
        assert!(matches!(
            ctrl_rx.try_recv(),
            Ok(Frame::CommandAck {
                ref command_id,
                accepted: false,
                reason: Some(ref reason),
                ..
            }) if command_id == "p-1" && reason.starts_with("unhanded:")
        ));
        assert!(broker.pending_commands.lock().get("s-1").is_none());

        assert!(
            broker
                .gate_ensure_against_crashed_drain("s-1")
                .await
                .is_err()
        );

        broker.record_crashed_drain_ack("s-1", "100-1", 1, "dc-1", true);
        broker.confirm_crashed_drain("s-1", "100-1", 1, Some("dc-1"));
        let peer = broker.remove_worker("s-1", 1).unwrap();
        broker.worker_disconnected("s-1".into(), peer, None).await;

        assert!(matches!(
            ctrl_rx.try_recv(),
            Ok(Frame::CrashedWorkerDrained {
                ref session_id,
                ref worker_epoch,
            }) if session_id == "s-1" && worker_epoch == "100-1"
        ));
        assert_eq!(
            broker.session_states.lock().get("s-1"),
            Some(&WorkerState::Crashed)
        );
        assert!(broker.sessions.lock().contains_key("s-1"));
        assert!(
            broker
                .gate_ensure_against_crashed_drain("s-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn mid_drain_unconfirmed_disconnect_reconciles_exit_before_ensure() {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        let mut ctrl_rx = attach_controller(
            &broker,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
            Some(CRASHED_WORKER_DRAIN_PROTOCOL_VERSION),
        );
        let _worker_rx = attach_crashed_worker(
            &broker,
            "s-2",
            "200-1",
            1,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );

        broker
            .drain_crashed_worker("s-2".into(), "dc-2".into(), "200-1".into())
            .await;
        broker.record_crashed_drain_ack("s-2", "200-1", 1, "dc-2", true);
        broker.direct_worker_pids.lock().insert("s-2".into(), 99999);
        let peer = broker.remove_worker("s-2", 1).unwrap();
        broker.worker_disconnected("s-2".into(), peer, None).await;
        assert!(matches!(ctrl_rx.try_recv(), Ok(Frame::Snapshot { .. })));
        assert!(
            broker
                .gate_ensure_against_crashed_drain("s-2")
                .await
                .is_err(),
            "live direct pid must block replacement ensure"
        );

        broker.direct_worker_pids.lock().remove("s-2");
        assert!(
            broker
                .gate_ensure_against_crashed_drain("s-2")
                .await
                .is_ok()
        );
        assert!(!broker.crashed_drains.0.lock().active.contains_key("s-2"));
    }

    #[tokio::test]
    async fn drain_refuses_legacy_worker_and_active_native_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        let mut ctrl_rx = attach_controller(
            &broker,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
            Some(CRASHED_WORKER_DRAIN_PROTOCOL_VERSION),
        );
        let _old_rx = attach_crashed_worker(&broker, "s-old", "300-1", 1, 9);
        broker
            .drain_crashed_worker("s-old".into(), "dc-old".into(), "300-1".into())
            .await;
        assert!(matches!(
            ctrl_rx.try_recv(),
            Ok(Frame::CommandAck {
                ref command_id,
                accepted: false,
                ..
            }) if command_id == "dc-old"
        ));

        let _busy_rx = attach_crashed_worker(
            &broker,
            "s-cycle",
            "301-1",
            2,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        broker
            .workers
            .lock()
            .get_mut("s-cycle")
            .unwrap()
            .snapshot
            .native_cycle_active = true;
        broker
            .drain_crashed_worker("s-cycle".into(), "dc-cycle".into(), "301-1".into())
            .await;
        assert!(matches!(
            ctrl_rx.try_recv(),
            Ok(Frame::CommandAck {
                ref command_id,
                accepted: false,
                ..
            }) if command_id == "dc-cycle"
        ));
    }
}
