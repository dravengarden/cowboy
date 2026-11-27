//! Cooperative epoch-bound retirement of idle workers in `WorkerState::Crashed`.
//!
//! All state lives under a single leaf mutex (`State`) acquired only AFTER any
//! broker locks (`controller` -> `cancelled_sessions` -> `sessions` -> `workers`
//! -> `pending_commands` -> `dispatched_prompts` -> `pending_core_commands` ->
//! `crashed_drains`) and never held across `.await` or `send_controller`.
//! Lifecycle mutations serialize on the broker's `session_lifecycle_gate`.

use super::*;
use crate::machine_protocol::CRASHED_WORKER_DRAIN_PROTOCOL_VERSION;
use crate::runtime_wire::CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION;

const RECENTLY_DRAINED_TTL: Duration = Duration::from_secs(120);
const MAX_RECENTLY_DRAINED: usize = 256;

#[derive(Default)]
pub(super) struct State {
    inner: Mutex<Inner>,
    pub(super) settled: Arc<tokio::sync::Notify>,
}

#[derive(Default)]
struct Inner {
    pending: HashMap<String, PendingDrain>,
    active: HashMap<String, ActiveDrain>,
    unconfirmed_sandbox: HashMap<String, String>,
    recently_drained: HashMap<String, (String, Instant)>,
}

struct PendingDrain {
    session_id: String,
    epoch: String,
    /// `0` while the worker is disconnected with an ambiguous ACK; rebound and
    /// replayed on same-epoch reconnect.
    connection_id: u64,
    acknowledged_uncertain_sequence: u64,
    peer_pid: Option<u32>,
}

struct ActiveDrain {
    epoch: String,
    connection_id: u64,
    confirmed: bool,
    peer_pid: Option<u32>,
}

enum DrainAdmission {
    Forwarded,
    RetryUnconfirmedSandbox,
    Rejected(String),
}

impl Broker {
    pub(super) async fn handle_drain_crashed_command(self: &Arc<Self>, command: CoreCommand) {
        if let CoreCommand::DrainCrashedWorker {
            session_id,
            command_id,
            worker_epoch,
            acknowledged_uncertain_sequence,
        } = command
        {
            self.drain_crashed_worker(
                session_id,
                command_id,
                worker_epoch,
                acknowledged_uncertain_sequence,
            )
            .await;
        }
    }

    pub(super) async fn drain_crashed_worker(
        self: &Arc<Self>,
        session_id: String,
        command_id: String,
        worker_epoch: String,
        acknowledged_uncertain_sequence: u64,
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
        match self.admit_crashed_drain(
            &session_id,
            &command_id,
            &worker_epoch,
            acknowledged_uncertain_sequence,
        ) {
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
        acknowledged_uncertain_sequence: u64,
    ) -> DrainAdmission {
        let controller = self.controller.lock();
        let controller_capable = controller.as_ref().is_some_and(|ctrl| {
            ctrl.protocol >= CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
                && ctrl
                    .outer_machine_protocol
                    .is_some_and(|proto| proto >= CRASHED_WORKER_DRAIN_PROTOCOL_VERSION)
        });
        if !controller_capable {
            return DrainAdmission::Rejected(
                "controller protocol does not support crashed worker drain".into(),
            );
        }
        let mut workers = self.workers.lock();
        let pending_cmds = self.pending_commands.lock();
        let dispatched = self.dispatched_prompts.lock();
        let pending_core = self.pending_core_commands.lock();
        let mut state = self.crashed_drains.inner.lock();
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
            Some(worker)
                if !worker_snapshot_idle_for_crashed_drain(
                    &worker.snapshot,
                    acknowledged_uncertain_sequence,
                ) =>
            {
                DrainAdmission::Rejected("crashed worker still has active or uncertain work".into())
            }
            Some(_)
                if pending_cmds
                    .get(session_id)
                    .is_some_and(|queue| !queue.is_empty())
                    || dispatched.has_unacked_for_session(session_id)
                    || pending_core.get(session_id).copied().unwrap_or(0) > 1 =>
            {
                DrainAdmission::Rejected("session has queued or in-flight prompt handoffs".into())
            }
            Some(worker) => {
                let frame = Frame::WorkerCommand {
                    session_id: session_id.to_owned(),
                    command: WorkerCommand::DrainCrashed {
                        command_id: command_id.to_owned(),
                        worker_epoch: worker_epoch.to_owned(),
                        acknowledged_uncertain_sequence,
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
                            acknowledged_uncertain_sequence,
                            peer_pid: worker.peer_pid,
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
            let (cleared, rejections) = {
                let workers = broker.workers.lock();
                let mut pending_cmds = broker.pending_commands.lock();
                let mut dispatched = broker.dispatched_prompts.lock();
                let mut state = broker.crashed_drains.inner.lock();
                if workers.contains_key(&session_id) {
                    (false, Vec::new())
                } else if state
                    .unconfirmed_sandbox
                    .get(&session_id)
                    .is_some_and(|epoch| epoch == &worker_epoch)
                {
                    state.unconfirmed_sandbox.remove(&session_id);
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
                broker.send_crashed_worker_drained(&session_id, &worker_epoch);
                broker.crashed_drains.settled.notify_waiters();
            } else {
                broker.command_rejected(
                    &session_id,
                    command_id,
                    "crashed worker sandbox retry was superseded".into(),
                );
            }
        });
    }

    fn spawn_background_sandbox_cleanup_retry(
        self: &Arc<Self>,
        session_id: String,
        worker_epoch: String,
    ) {
        let broker = Arc::clone(self);
        tokio::spawn(async move {
            for attempt in 0..8_u32 {
                let delay = Duration::from_secs((2_u64 << attempt.min(4)).min(30));
                tokio::time::sleep(delay).await;
                let still_unconfirmed = broker
                    .crashed_drains
                    .inner
                    .lock()
                    .unconfirmed_sandbox
                    .get(&session_id)
                    .is_some_and(|epoch| epoch == &worker_epoch);
                if !still_unconfirmed {
                    return;
                }
                let gate = broker.session_lifecycle_gate(&session_id);
                let _guard = gate.lock().await;
                if broker
                    .permanently_deleted_sessions
                    .lock()
                    .contains(&session_id)
                    || broker.cancelled_sessions.lock().contains(&session_id)
                    || broker.workers.lock().contains_key(&session_id)
                {
                    return;
                }
                if !broker
                    .confirm_durable_session_account_units_gone(&session_id)
                    .await
                {
                    continue;
                }
                let (cleared, rejections) = {
                    let workers = broker.workers.lock();
                    let mut pending_cmds = broker.pending_commands.lock();
                    let mut dispatched = broker.dispatched_prompts.lock();
                    let mut state = broker.crashed_drains.inner.lock();
                    if !workers.contains_key(&session_id)
                        && state
                            .unconfirmed_sandbox
                            .get(&session_id)
                            .is_some_and(|epoch| epoch == &worker_epoch)
                    {
                        state.unconfirmed_sandbox.remove(&session_id);
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
                    broker.send_crashed_worker_drained(&session_id, &worker_epoch);
                    broker.crashed_drains.settled.notify_waiters();
                }
                return;
            }
        });
    }

    /// Process a worker `CommandAck`. Returns `false` when the ACK came from a
    /// superseded connection for a tracked crashed-drain command (caller must
    /// NOT forward it to Controller).
    pub(super) fn record_crashed_drain_ack(
        &self,
        session_id: &str,
        epoch: &str,
        connection_id: u64,
        command_id: &str,
        accepted: bool,
    ) -> bool {
        let mut workers = self.workers.lock();
        let mut state = self.crashed_drains.inner.lock();
        let Some(entry) = state.pending.get(command_id) else {
            return true;
        };
        let worker_matches = workers
            .get(session_id)
            .is_some_and(|worker| worker.epoch == epoch && worker.connection_id == connection_id);
        if !worker_matches
            || entry.session_id != session_id
            || entry.epoch != epoch
            || entry.connection_id != connection_id
        {
            return false;
        }
        let peer_pid = entry.peer_pid;
        state.pending.remove(command_id);
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
                    peer_pid,
                },
            );
            if let Some(worker) = workers.get_mut(session_id) {
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
            if !still_active && !other_pending {
                if let Some(worker) = workers.get_mut(session_id) {
                    worker.crashed_drain_fenced = false;
                }
                self.crashed_drains.settled.notify_waiters();
            }
        }
        true
    }

    pub(super) fn confirm_crashed_drain(
        &self,
        session_id: &str,
        epoch: &str,
        connection_id: u64,
        command_id: Option<&str>,
    ) {
        let mut workers = self.workers.lock();
        let mut state = self.crashed_drains.inner.lock();
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
                peer_pid: worker.peer_pid,
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
        let mut state = self.crashed_drains.inner.lock();
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

    /// After sending `Welcome` to a reconnected worker, replay any ambiguous
    /// pending `DrainCrashed` command for that exact `(session_id, epoch,
    /// connection_id)` so the worker returns its remembered decision (or runs
    /// the drain if it never received the first frame).
    pub(super) fn replay_pending_crashed_drain(&self, session_id: &str, connection_id: u64) {
        let to_send = {
            let workers = self.workers.lock();
            let Some(worker) = workers
                .get(session_id)
                .filter(|w| w.connection_id == connection_id && w.welcomed)
            else {
                return;
            };
            let state = self.crashed_drains.inner.lock();
            let commands: Vec<WorkerCommand> = state
                .pending
                .iter()
                .filter(|(_, entry)| {
                    entry.session_id == session_id
                        && entry.epoch == worker.epoch
                        && entry.connection_id == connection_id
                })
                .map(|(cid, entry)| WorkerCommand::DrainCrashed {
                    command_id: cid.clone(),
                    worker_epoch: entry.epoch.clone(),
                    acknowledged_uncertain_sequence: entry.acknowledged_uncertain_sequence,
                })
                .collect();
            (worker.tx.clone(), commands)
        };
        let (tx, commands) = to_send;
        for command in commands {
            let _ = tx.send(Frame::WorkerCommand {
                session_id: session_id.to_owned(),
                command,
            });
        }
    }

    pub(super) fn attach_worker_peer_pid(
        &self,
        session_id: &str,
        connection_id: u64,
        pid: Option<u32>,
    ) {
        let mut workers = self.workers.lock();
        if let Some(worker) = workers
            .get_mut(session_id)
            .filter(|w| w.connection_id == connection_id)
        {
            worker.peer_pid = pid;
            let mut state = self.crashed_drains.inner.lock();
            if let Some(active) = state.active.get_mut(session_id)
                && active.connection_id == connection_id
            {
                active.peer_pid = pid;
            }
            for pending in state.pending.values_mut() {
                if pending.session_id == session_id && pending.connection_id == connection_id {
                    pending.peer_pid = pid;
                }
            }
        }
    }

    /// Handle disconnect of a crashed worker that already sent
    /// `CrashedDrainCompleted`. Returns `true` when this path owned the
    /// disconnect and suppressed normal `Crashed` snapshot re-emission.
    pub(super) async fn try_finish_crashed_drain_disconnect(
        self: &Arc<Self>,
        session_id: &str,
        peer: &WorkerPeer,
    ) -> bool {
        let confirmed = {
            let mut state = self.crashed_drains.inner.lock();
            let confirmed = state.active.get(session_id).is_some_and(|active| {
                active.epoch == peer.epoch
                    && active.connection_id == peer.connection_id
                    && active.confirmed
            });
            if !confirmed {
                // Keep ambiguous pending drain alive across transient socket
                // loss so same-epoch reconnect rebinds and replays the exact
                // command ID, while mid-drain death is reconciled by Ensure.
                for pending in state.pending.values_mut() {
                    if pending.session_id == session_id
                        && pending.epoch == peer.epoch
                        && pending.connection_id == peer.connection_id
                    {
                        pending.connection_id = 0;
                    }
                }
            }
            confirmed
        };
        if !confirmed {
            self.crashed_drains.settled.notify_waiters();
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
                .inner
                .lock()
                .active
                .get(session_id)
                .is_some_and(|active| {
                    active.epoch == peer.epoch && active.connection_id == peer.connection_id
                });
        if !still_owned {
            let orphaned = {
                let mut state = self.crashed_drains.inner.lock();
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
            self.crashed_drains.settled.notify_waiters();
            return true;
        }

        let unit_gone = self
            .confirm_durable_session_account_units_gone(session_id)
            .await;
        self.commit_crashed_drain_disconnect(session_id, peer, unit_gone);
        self.crashed_drains.settled.notify_waiters();
        true
    }

    fn commit_crashed_drain_disconnect(
        self: &Arc<Self>,
        session_id: &str,
        peer: &WorkerPeer,
        unit_gone: bool,
    ) {
        let (commit, session_for_cache, orphaned_pending, queued_rejections) = {
            let cancelled = self.cancelled_sessions.lock();
            let deleted = self.permanently_deleted_sessions.lock();
            let sessions = self.sessions.lock();
            let workers = self.workers.lock();
            let mut pending_cmds = self.pending_commands.lock();
            let mut dispatched = self.dispatched_prompts.lock();
            let mut state = self.crashed_drains.inner.lock();

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
                let rejections = classify_and_drain_queued_commands(
                    &mut pending_cmds,
                    &mut dispatched,
                    session_id,
                );
                if unit_gone {
                    state.unconfirmed_sandbox.remove(session_id);
                    (
                        true,
                        sessions.get(session_id).cloned(),
                        orphaned,
                        rejections,
                    )
                } else {
                    state
                        .unconfirmed_sandbox
                        .insert(session_id.to_owned(), peer.epoch.clone());
                    (true, None, orphaned, rejections)
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
        for (cid, reason) in queued_rejections {
            self.command_rejected(session_id, cid, reason);
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
        } else {
            self.spawn_background_sandbox_cleanup_retry(session_id.to_owned(), peer.epoch.clone());
        }
    }

    /// Defer `reset_session` before rotating incarnation or fencing projection
    /// while an accepted crashed-worker drain is actively flushing outbox
    /// events. Releases `lifecycle_guard` across bounded waits so disconnect
    /// teardown can finish, then re-acquires and returns the guard.
    pub(super) async fn wait_for_crashed_drain_before_reset<'a>(
        &self,
        session_id: &str,
        gate: &'a tokio::sync::Mutex<()>,
        mut guard: tokio::sync::MutexGuard<'a, ()>,
    ) -> Result<tokio::sync::MutexGuard<'a, ()>, String> {
        let deadline = tokio::time::Instant::now() + self.args.worker_ready_timeout;
        loop {
            let live_draining = self
                .workers
                .lock()
                .get(session_id)
                .is_some_and(|worker| worker.crashed_drain_fenced);
            let confirmed_settling = self
                .crashed_drains
                .inner
                .lock()
                .active
                .get(session_id)
                .is_some_and(|active| active.confirmed);
            if !live_draining && !confirmed_settling {
                return Ok(guard);
            }
            if !lease_active(self) {
                return Err("controller lease changed while awaiting crashed drain".into());
            }
            let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now())
            else {
                return Err("crashed worker drain is still flushing; retry".into());
            };
            let notified = self.crashed_drains.settled.notified();
            drop(guard);
            let _ = tokio::time::timeout(remaining.min(Duration::from_millis(200)), notified).await;
            guard = gate.lock().await;
        }
    }

    /// Gate explicit non-`adopt_only` `EnsureSession` under
    /// `session_lifecycle_gate` so a live draining crashed worker is never
    /// ripped out mid-flush and any unconfirmed exit or durable account sandbox
    /// teardown is confirmed before spawning a replacement.
    pub(super) async fn gate_ensure_against_crashed_drain(
        &self,
        session_id: &str,
    ) -> Result<(), String> {
        let (fenced_live, has_live, unsettled_pid, has_unsettled, unconfirmed_epoch) = {
            let workers = self.workers.lock();
            let state = self.crashed_drains.inner.lock();
            let live = workers.get(session_id);
            let active_entry = state.active.get(session_id);
            let pending_entry = state
                .pending
                .values()
                .find(|pending| pending.session_id == session_id);
            let pid = active_entry
                .and_then(|entry| entry.peer_pid)
                .or_else(|| pending_entry.and_then(|entry| entry.peer_pid));
            (
                live.is_some_and(|worker| worker.crashed_drain_fenced),
                live.is_some(),
                pid,
                active_entry.is_some() || pending_entry.is_some(),
                state.unconfirmed_sandbox.get(session_id).cloned(),
            )
        };
        if fenced_live {
            return Err("crashed worker drain is in progress; retry".into());
        }
        if has_live {
            return Ok(());
        }
        if has_unsettled
            && !self
                .confirm_mid_drain_worker_exited(session_id, unsettled_pid)
                .await
        {
            return Err("crashed worker drain is still settling; retry".into());
        }
        // Always confirm durable account sandbox units are gone before clearing
        // an accepted/pending drain OR admitting a replacement (covers both
        // mid-drain exit and broker-restart durability).
        if !self
            .confirm_durable_session_account_units_gone(session_id)
            .await
        {
            return Err(
                "previous crashed worker sandbox unit teardown is unconfirmed; retry".into(),
            );
        }
        if has_unsettled || unconfirmed_epoch.is_some() {
            let (orphaned, emit_epoch, queued_rejections) = {
                let workers = self.workers.lock();
                let mut pending_cmds = self.pending_commands.lock();
                let mut dispatched = self.dispatched_prompts.lock();
                let mut state = self.crashed_drains.inner.lock();
                if workers.contains_key(session_id) {
                    return Err("crashed worker reconnected during drain; retry".into());
                }
                state.active.remove(session_id);
                let orphaned = drain_pending_for_session(&mut state.pending, session_id);
                let emit = unconfirmed_epoch.filter(|epoch| {
                    state
                        .unconfirmed_sandbox
                        .get(session_id)
                        .is_some_and(|recorded| recorded == epoch)
                });
                let rejections = if emit.is_some() {
                    state.unconfirmed_sandbox.remove(session_id);
                    classify_and_drain_queued_commands(
                        &mut pending_cmds,
                        &mut dispatched,
                        session_id,
                    )
                } else {
                    Vec::new()
                };
                (orphaned, emit, rejections)
            };
            for (sid, cid) in orphaned {
                self.command_rejected(&sid, cid, "crashed worker exited during drain".into());
            }
            for (cid, reason) in queued_rejections {
                self.command_rejected(session_id, cid, reason);
            }
            if let Some(epoch) = emit_epoch {
                self.send_crashed_worker_drained(session_id, &epoch);
            }
            self.crashed_drains.settled.notify_waiters();
        }
        Ok(())
    }

    async fn confirm_mid_drain_worker_exited(
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
                if self.direct_worker_pids.lock().contains_key(session_id) {
                    return false;
                }
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
                // Adopted Direct workers after a broker restart have no entry
                // in `direct_worker_pids`; verify the recorded PID and process
                // group directly (fail closed when PID identity is unknown).
                recorded_pid.is_some_and(direct_process_tree_gone)
            }
        }
    }

    pub(super) fn clear_crashed_drain(&self, session_id: &str, reason: &str) {
        let orphaned = {
            let mut state = self.crashed_drains.inner.lock();
            state.active.remove(session_id);
            state.unconfirmed_sandbox.remove(session_id);
            drain_pending_for_session(&mut state.pending, session_id)
        };
        for (sid, cid) in orphaned {
            self.command_rejected(&sid, cid, reason.to_owned());
        }
        self.crashed_drains.settled.notify_waiters();
    }

    pub(super) fn unconfirmed_sandbox_epoch(&self, session_id: &str) -> Option<String> {
        self.crashed_drains
            .inner
            .lock()
            .unconfirmed_sandbox
            .get(session_id)
            .cloned()
    }

    pub(super) fn send_crashed_worker_drained(&self, session_id: &str, worker_epoch: &str) {
        {
            let mut state = self.crashed_drains.inner.lock();
            let now = Instant::now();
            state
                .recently_drained
                .retain(|_, (_, at)| now.saturating_duration_since(*at) < RECENTLY_DRAINED_TTL);
            if state.recently_drained.len() >= MAX_RECENTLY_DRAINED
                && let Some(oldest) = state
                    .recently_drained
                    .iter()
                    .min_by_key(|(_, (_, at))| *at)
                    .map(|(k, _)| k.clone())
            {
                state.recently_drained.remove(&oldest);
            }
            state
                .recently_drained
                .insert(session_id.to_owned(), (worker_epoch.to_owned(), now));
        }
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

    /// Close the Controller handover gap: if a worker appeared in `welcome`
    /// and completed crashed-drain retirement before the new Controller lease
    /// was installed, replay `CrashedWorkerDrained` to the new lease.
    pub(super) fn replay_recently_drained_after_welcome(
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
                        .filter(|(epoch, _)| epoch == &snap.worker_epoch)
                        .map(|(epoch, _)| (snap.session_id.clone(), epoch.clone()))
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

fn classify_and_drain_queued_commands(
    pending_cmds: &mut HashMap<String, VecDeque<WorkerCommand>>,
    dispatched: &mut wrap_up::DispatchedPrompts,
    session_id: &str,
) -> Vec<(String, String)> {
    let Some(queue) = pending_cmds.remove(session_id) else {
        return Vec::new();
    };
    let mut rejections = Vec::new();
    for cmd in queue {
        let Some(cid) = worker_command_id(&cmd).map(str::to_owned) else {
            continue;
        };
        let reason = if matches!(cmd, WorkerCommand::Prompt { .. }) {
            let ambiguous = dispatched.was_command_evicted(session_id, &cid)
                || dispatched.get(session_id, &cid).is_some();
            dispatched.mark_acked(session_id, &cid);
            if ambiguous {
                "delivery_ambiguous: prompt handoff across crashed worker drain is unresolved; retained as draft"
            } else {
                "unhanded: crashed worker drained; retry on replacement"
            }
        } else {
            dispatched.mark_acked(session_id, &cid);
            "unhanded: crashed worker drained; retry on replacement"
        };
        rejections.push((cid, reason.to_owned()));
    }
    rejections
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
        peer_pid: None,
    }
}

fn worker_snapshot_idle_for_crashed_drain(
    snapshot: &WorkerSnapshot,
    acknowledged_uncertain_sequence: u64,
) -> bool {
    snapshot.current_turn_id.is_none()
        && snapshot.pending_permissions.is_empty()
        && snapshot.background_tasks.unwrap_or(0) == 0
        && !snapshot.native_cycle_active
        && snapshot.native_cycle_id.is_none()
        && snapshot.pending_prompt_count == 0
        && snapshot
            .uncertain_effect
            .as_ref()
            .is_none_or(|effect| effect.sequence.max(1) <= acknowledged_uncertain_sequence)
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

pub(super) fn evicted_command_digest(command_id: &str) -> [u8; 16] {
    use sha2::Digest as _;
    let hash = sha2::Sha256::digest(command_id.as_bytes());
    let mut out = [0_u8; 16];
    out.copy_from_slice(&hash[..16]);
    out
}

/// Sanitize a worker snapshot for the currently attached Controller using exact
/// `(controller_outer_protocol, controller_runtime_protocol, worker_protocol)`
/// and broker-verified provider identity (fails closed if provider is missing).
pub(super) fn sanitize_snapshot_for_controller(
    snapshot: &mut WorkerSnapshot,
    raw_wrap_up: bool,
    worker_protocol: u16,
    controller_outer_protocol: Option<u16>,
    controller_runtime_protocol: u16,
    declared_provider: Option<&str>,
) {
    let provider = snapshot
        .launch
        .as_ref()
        .map(|launch| launch.provider.as_str())
        .or(declared_provider);
    crate::wrap_up::clamp_delivery_capabilities_for_provider(
        provider,
        &mut snapshot.supports_current_turn,
        &mut snapshot.delivery_controls.next_checkpoint,
    );
    let min_runtime_protocol = worker_protocol.min(controller_runtime_protocol);
    snapshot.wrap_up_steering = crate::wrap_up::effective_wrap_up_steering(
        provider,
        raw_wrap_up,
        controller_outer_protocol,
        min_runtime_protocol,
        true,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_broker(dir: &Path) -> Arc<Broker> {
        Arc::new(Broker::new(MachineBrokerArgs {
            socket: dir.join("broker.sock"),
            worker_command: PathBuf::from("/bin/true"),
            desired_generation: "gen-1".into(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: crate::machine_broker::tests::test_provider_store(),
            worktree_root: crate::session_workspace::WorktreeRoots::single(dir.join("worktrees")),
            worker_ready_timeout: Duration::from_secs(2),
            call_context_root: None,
            memory_relay: None,
        }))
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
        // Dead test PID (never matches a live process) unless overridden.
        peer.peer_pid = Some(999_999_999);
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
            .drain_crashed_worker("s-1".into(), "dc-1".into(), "100-1".into(), 0)
            .await;

        assert!(matches!(
            worker_rx.try_recv(),
            Ok(Frame::WorkerCommand {
                command: WorkerCommand::DrainCrashed { ref command_id, ref worker_epoch, acknowledged_uncertain_sequence: 0 },
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

        assert!(broker.record_crashed_drain_ack("s-1", "100-1", 1, "dc-1", true));
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
    async fn mid_drain_unconfirmed_disconnect_verifies_direct_pid_before_ensure() {
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
        // Live PID (our own process) simulates an adopted Direct worker still alive after broker restart.
        broker.attach_worker_peer_pid("s-2", 1, Some(std::process::id()));

        broker
            .drain_crashed_worker("s-2".into(), "dc-2".into(), "200-1".into(), 0)
            .await;
        assert!(broker.record_crashed_drain_ack("s-2", "200-1", 1, "dc-2", true));
        let peer = broker.remove_worker("s-2", 1).unwrap();
        broker.worker_disconnected("s-2".into(), peer, None).await;
        assert!(matches!(ctrl_rx.try_recv(), Ok(Frame::Snapshot { .. })));
        assert!(
            broker
                .gate_ensure_against_crashed_drain("s-2")
                .await
                .is_err(),
            "live adopted direct pid must block replacement ensure"
        );

        // Once the PID is dead, ensure reconciles and clears the stale active drain.
        broker
            .crashed_drains
            .inner
            .lock()
            .active
            .get_mut("s-2")
            .unwrap()
            .peer_pid = Some(999_999_999);
        assert!(
            broker
                .gate_ensure_against_crashed_drain("s-2")
                .await
                .is_ok()
        );
        assert!(
            !broker
                .crashed_drains
                .inner
                .lock()
                .active
                .contains_key("s-2")
        );
    }

    #[tokio::test]
    async fn lost_ack_replays_pending_drain_on_same_epoch_reconnect_and_rejects_stale_connection_ack()
     {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        let _ctrl_rx = attach_controller(
            &broker,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
            Some(CRASHED_WORKER_DRAIN_PROTOCOL_VERSION),
        );
        let _old_rx = attach_crashed_worker(
            &broker,
            "s-rebind",
            "400-1",
            1,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        broker
            .drain_crashed_worker("s-rebind".into(), "dc-rebind".into(), "400-1".into(), 2)
            .await;
        // Disconnect before ACK arrives: pending entry stays alive with connection_id = 0.
        let peer = broker.remove_worker("s-rebind", 1).unwrap();
        broker
            .worker_disconnected("s-rebind".into(), peer, None)
            .await;
        assert!(
            broker
                .crashed_drains
                .inner
                .lock()
                .pending
                .contains_key("dc-rebind")
        );

        // Same-epoch reconnect rebinds and replays the exact original command ID.
        let mut new_rx = attach_crashed_worker(
            &broker,
            "s-rebind",
            "400-1",
            2,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        assert!(broker.workers.lock()["s-rebind"].crashed_drain_fenced);
        broker.replay_pending_crashed_drain("s-rebind", 2);
        assert!(matches!(
            new_rx.try_recv(),
            Ok(Frame::WorkerCommand {
                command: WorkerCommand::DrainCrashed {
                    ref command_id,
                    ref worker_epoch,
                    acknowledged_uncertain_sequence: 2,
                },
                ..
            }) if command_id == "dc-rebind" && worker_epoch == "400-1"
        ));

        // Stale ACK from superseded connection 1 must NOT consume connection 2's pending command.
        assert!(!broker.record_crashed_drain_ack("s-rebind", "400-1", 1, "dc-rebind", false));
        assert!(
            broker
                .crashed_drains
                .inner
                .lock()
                .pending
                .contains_key("dc-rebind")
        );
        assert!(broker.workers.lock()["s-rebind"].crashed_drain_fenced);
    }

    #[tokio::test]
    async fn drain_refuses_legacy_worker_legacy_controller_and_unacked_uncertain_effect() {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        let mut ctrl_rx = attach_controller(&broker, 9, Some(54));
        let _rx = attach_crashed_worker(
            &broker,
            "s-legacy-ctrl",
            "300-1",
            1,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        broker
            .drain_crashed_worker("s-legacy-ctrl".into(), "dc-lc".into(), "300-1".into(), 0)
            .await;
        assert!(matches!(
            ctrl_rx.try_recv(),
            Ok(Frame::CommandAck {
                ref command_id,
                accepted: false,
                ..
            }) if command_id == "dc-lc"
        ));

        let mut ctrl_rx = attach_controller(
            &broker,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
            Some(CRASHED_WORKER_DRAIN_PROTOCOL_VERSION),
        );
        let _rx2 = attach_crashed_worker(
            &broker,
            "s-uncertain",
            "301-1",
            2,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        broker
            .workers
            .lock()
            .get_mut("s-uncertain")
            .unwrap()
            .snapshot
            .uncertain_effect = Some(crate::runtime_wire::UncertainEffect {
            method: "write".into(),
            sequence: 4,
        });
        broker
            .drain_crashed_worker("s-uncertain".into(), "dc-u".into(), "301-1".into(), 3)
            .await;
        assert!(matches!(
            ctrl_rx.try_recv(),
            Ok(Frame::CommandAck {
                ref command_id,
                accepted: false,
                ..
            }) if command_id == "dc-u"
        ));
    }

    #[tokio::test]
    async fn disconnect_cleanup_marks_handed_or_evicted_queued_prompts_delivery_ambiguous_and_replays_handover_retirement()
     {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        let mut ctrl_rx = attach_controller(
            &broker,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
            Some(CRASHED_WORKER_DRAIN_PROTOCOL_VERSION),
        );
        let _worker_rx = attach_crashed_worker(
            &broker,
            "s-ambig",
            "500-1",
            1,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        broker
            .drain_crashed_worker("s-ambig".into(), "dc-ambig".into(), "500-1".into(), 0)
            .await;
        assert!(broker.record_crashed_drain_ack("s-ambig", "500-1", 1, "dc-ambig", true));
        broker.confirm_crashed_drain("s-ambig", "500-1", 1, Some("dc-ambig"));

        // Capture a Welcome inventory BEFORE disconnect (simulating Controller handover).
        let welcome = broker.snapshots();
        // Simulate a retransmitted prompt queued during disconnect whose handoff was recorded.
        broker
            .dispatched_prompts
            .lock()
            .record("s-ambig", "p-handed", "500-1");
        broker
            .pending_commands
            .lock()
            .entry("s-ambig".into())
            .or_default()
            .push_back(WorkerCommand::Prompt {
                command_id: "p-handed".into(),
                turn_id: "t-handed".into(),
                content: vec![serde_json::json!({"type": "text", "text": "retry"})],
                echo_artifacts: false,
                observe_subagents: false,
                cmid: None,
                trace: None,
                expected_worker_epoch: None,
                acknowledged_uncertain_sequence: None,
            });

        let peer = broker.remove_worker("s-ambig", 1).unwrap();
        broker
            .worker_disconnected("s-ambig".into(), peer, None)
            .await;

        assert!(matches!(
            ctrl_rx.try_recv(),
            Ok(Frame::CommandAck {
                ref command_id,
                accepted: false,
                reason: Some(ref reason),
                ..
            }) if command_id == "p-handed" && reason.starts_with("delivery_ambiguous:")
        ));
        assert!(matches!(
            ctrl_rx.try_recv(),
            Ok(Frame::CrashedWorkerDrained {
                ref session_id,
                ref worker_epoch,
            }) if session_id == "s-ambig" && worker_epoch == "500-1"
        ));

        // New Controller lease installed after capturing `welcome` receives the replayed retirement.
        let mut new_ctrl_rx = attach_controller(
            &broker,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
            Some(CRASHED_WORKER_DRAIN_PROTOCOL_VERSION),
        );
        broker.replay_recently_drained_after_welcome(&welcome, 1);
        assert!(matches!(
            new_ctrl_rx.try_recv(),
            Ok(Frame::CrashedWorkerDrained {
                ref session_id,
                ref worker_epoch,
            }) if session_id == "s-ambig" && worker_epoch == "500-1"
        ));
    }
}
