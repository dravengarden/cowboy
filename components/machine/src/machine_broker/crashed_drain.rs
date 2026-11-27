//! Cooperative epoch-bound retirement of idle workers in `WorkerState::Crashed`.
//!
//! All state lives under a single leaf mutex (`State`) acquired only AFTER any
//! broker locks (`controller` -> `cancelled_sessions` -> `sessions` -> `workers`
//! -> `pending_commands` -> `dispatched_prompts` -> `pending_core_commands` ->
//! `crashed_drains`) and never held across `.await` or `send_controller`.
//! Lifecycle mutations serialize on the broker's `session_lifecycle_gate`.

mod teardown;

use super::*;
use crate::machine_protocol::CRASHED_WORKER_DRAIN_PROTOCOL_VERSION;
use crate::runtime_wire::CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION;

const RECENTLY_DRAINED_TTL: Duration = Duration::from_secs(120);
const MAX_RECENTLY_DRAINED: usize = 256;
const DIRECT_EXIT_GRACE: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(super) struct State {
    inner: Mutex<Inner>,
    pub(super) settled: Arc<tokio::sync::Notify>,
}

#[derive(Default)]
struct Inner {
    pending: HashMap<String, PendingDrain>,
    active: HashMap<String, ActiveDrain>,
    unconfirmed_teardown: HashMap<String, UnconfirmedTeardown>,
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
    retired_launch: Option<uuid::Uuid>,
}

struct ActiveDrain {
    epoch: String,
    connection_id: u64,
    confirmed: bool,
    peer_pid: Option<u32>,
    retired_launch: Option<uuid::Uuid>,
}

#[derive(Clone)]
struct UnconfirmedTeardown {
    epoch: String,
    peer_pid: Option<u32>,
    retired_launch: Option<uuid::Uuid>,
}

enum DrainAdmission {
    Forwarded,
    RetryUnconfirmedTeardown(UnconfirmedTeardown),
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
            || self.replacing.lock().contains_key(&session_id)
        {
            self.command_rejected(
                &session_id,
                command_id,
                "session launch or reset is in progress".into(),
            );
            return;
        }
        let current_launch = match self.current_durable_launch_id(&session_id).await {
            Ok(launch) => launch,
            Err(error) => {
                self.command_rejected(
                    &session_id,
                    command_id,
                    format!("unable to verify durable launch identity: {error}"),
                );
                return;
            }
        };
        match self.admit_crashed_drain(
            &session_id,
            &command_id,
            &worker_epoch,
            acknowledged_uncertain_sequence,
            current_launch,
        ) {
            DrainAdmission::Forwarded => {}
            DrainAdmission::Rejected(reason) => {
                self.command_rejected(&session_id, command_id, reason);
            }
            DrainAdmission::RetryUnconfirmedTeardown(entry) => {
                self.spawn_unconfirmed_sandbox_drain_retry(
                    lifecycle_guard,
                    session_id,
                    command_id,
                    entry,
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
        retired_launch: Option<uuid::Uuid>,
    ) -> DrainAdmission {
        let controller = self.controller.lock();
        let expected_lease = active_command_lease();
        let controller_capable = controller.as_ref().is_some_and(|ctrl| {
            expected_lease.is_none_or(|lease| ctrl.lease == lease)
                && ctrl.protocol >= CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
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
                if let Some(entry) = state
                    .unconfirmed_teardown
                    .get(session_id)
                    .filter(|entry| entry.epoch == worker_epoch)
                    .cloned()
                {
                    DrainAdmission::RetryUnconfirmedTeardown(entry)
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
                    || dispatched.has_unacked_for_epoch_or_ingress(session_id, worker_epoch)
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
                            retired_launch,
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

    pub(super) fn extract_prompt_header(
        &self,
        session_id: &str,
        command: &WorkerCommand,
    ) -> Option<(String, Option<String>, Option<String>)> {
        let WorkerCommand::Prompt {
            command_id,
            turn_id,
            trace,
            expected_worker_epoch,
            ..
        } = command
        else {
            return None;
        };
        if let Some(trace) = trace {
            self.trace_spans.lock().start(session_id, command_id, trace);
        }
        Some((
            command_id.clone(),
            expected_worker_epoch.clone(),
            Some(turn_id.clone()),
        ))
    }

    pub(super) fn handle_worker_command_ack(&self, epoch: &str, connection_id: u64, frame: Frame) {
        let Frame::CommandAck {
            ref session_id,
            ref command_id,
            accepted,
            ..
        } = frame
        else {
            return;
        };
        let (should_forward, cleared_fence) =
            self.record_crashed_drain_ack(session_id, epoch, connection_id, command_id, accepted);
        if !should_forward {
            return;
        }
        let sid = session_id.clone();
        self.dispatched_prompts
            .lock()
            .mark_acked(session_id, command_id);
        self.send_controller(frame);
        if cleared_fence {
            self.flush_pending(&sid);
        }
    }

    pub(super) fn record_crashed_drain_ack(
        &self,
        session_id: &str,
        epoch: &str,
        connection_id: u64,
        command_id: &str,
        accepted: bool,
    ) -> (bool, bool) {
        let mut workers = self.workers.lock();
        let mut state = self.crashed_drains.inner.lock();
        let Some(entry) = state.pending.get(command_id) else {
            return (true, false);
        };
        let worker_matches = workers
            .get(session_id)
            .is_some_and(|worker| worker.epoch == epoch && worker.connection_id == connection_id);
        if !worker_matches
            || entry.session_id != session_id
            || entry.epoch != epoch
            || entry.connection_id != connection_id
        {
            return (false, false);
        }
        let peer_pid = entry.peer_pid;
        let retired_launch = entry.retired_launch;
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
                    retired_launch,
                },
            );
            if let Some(worker) = workers.get_mut(session_id) {
                worker.crashed_drain_fenced = true;
            }
            (true, false)
        } else {
            let still_active = state.active.get(session_id).is_some_and(|active| {
                active.epoch == epoch && active.connection_id == connection_id
            });
            let other_pending = state
                .pending
                .values()
                .any(|pending| pending.session_id == session_id && pending.epoch == epoch);
            let mut cleared_fence = false;
            if !still_active && !other_pending {
                if let Some(worker) = workers.get_mut(session_id) {
                    worker.crashed_drain_fenced = false;
                    cleared_fence = true;
                }
                self.crashed_drains.settled.notify_waiters();
            }
            (true, cleared_fence)
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
        let mut state = self.crashed_drains.inner.lock();
        let Some(worker) = workers.get_mut(session_id) else {
            return;
        };
        if worker.epoch != epoch || worker.connection_id != connection_id {
            return;
        }
        let matching_pending_launch = command_id.and_then(|id| {
            state
                .pending
                .get(id)
                .filter(|pending| {
                    pending.session_id == session_id
                        && pending.epoch == epoch
                        && pending.connection_id == connection_id
                })
                .and_then(|pending| pending.retired_launch)
        });
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
        let retired_launch = state
            .active
            .get(session_id)
            .and_then(|active| active.retired_launch)
            .or(matching_pending_launch);
        worker.crashed_drain_fenced = true;
        state.active.insert(
            session_id.to_owned(),
            ActiveDrain {
                epoch: epoch.to_owned(),
                connection_id,
                confirmed: true,
                peer_pid: worker.peer_pid,
                retired_launch,
            },
        );
    }

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
        state.unconfirmed_teardown.remove(session_id);
        drain_pending_for_session(&mut state.pending, session_id)
    }

    pub(super) fn has_crashed_drain_for_epoch(&self, session_id: &str, epoch: &str) -> bool {
        let state = self.crashed_drains.inner.lock();
        state
            .active
            .get(session_id)
            .is_some_and(|a| a.epoch == epoch)
            || state
                .pending
                .values()
                .any(|p| p.session_id == session_id && p.epoch == epoch)
    }

    pub(super) fn crashed_drain_fenced_for_epoch(&self, session_id: &str, epoch: &str) -> bool {
        let workers = self.workers.lock();
        workers
            .get(session_id)
            .is_some_and(|w| w.epoch == epoch && w.crashed_drain_fenced)
            || self.has_crashed_drain_for_epoch(session_id, epoch)
    }

    /// After sending `Welcome` to a reconnected worker, replay any ambiguous
    /// pending `DrainCrashed` command under canonical `controller -> workers ->
    /// crashed_drains` lock order. When no Controller lease is currently
    /// attached (handover gap), preserve pending state and fence intact until a
    /// lease attaches (R3-M1).
    pub(super) fn replay_pending_crashed_drain(&self, session_id: &str, connection_id: u64) {
        let (incompatible_rejects, cleared_fence) = {
            let controller = self.controller.lock();
            let Some(ctrl) = controller.as_ref() else {
                // Handover gap: keep ambiguous ownership fenced until next lease installs.
                return;
            };
            let controller_capable = ctrl.protocol >= CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
                && ctrl
                    .outer_machine_protocol
                    .is_some_and(|proto| proto >= CRASHED_WORKER_DRAIN_PROTOCOL_VERSION);
            let mut workers = self.workers.lock();
            let Some(worker) = workers
                .get_mut(session_id)
                .filter(|w| w.connection_id == connection_id && w.welcomed)
            else {
                return;
            };
            let mut state = self.crashed_drains.inner.lock();
            let matching_ids: Vec<String> = state
                .pending
                .iter()
                .filter(|(_, entry)| {
                    entry.session_id == session_id
                        && entry.epoch == worker.epoch
                        && entry.connection_id == connection_id
                })
                .map(|(cid, _)| cid.clone())
                .collect();
            if matching_ids.is_empty() {
                return;
            }
            if !controller_capable
                || worker.negotiated_protocol < CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
            {
                let mut rejects = Vec::new();
                for cid in matching_ids {
                    if let Some(entry) = state.pending.remove(&cid) {
                        rejects.push((entry.session_id, cid));
                    }
                }
                let mut cleared = false;
                if !state.active.contains_key(session_id) {
                    worker.crashed_drain_fenced = false;
                    cleared = true;
                }
                (rejects, cleared)
            } else {
                for cid in matching_ids {
                    if let Some(entry) = state.pending.get(&cid) {
                        let _ = worker.tx.send(Frame::WorkerCommand {
                            session_id: session_id.to_owned(),
                            command: WorkerCommand::DrainCrashed {
                                command_id: cid,
                                worker_epoch: entry.epoch.clone(),
                                acknowledged_uncertain_sequence: entry
                                    .acknowledged_uncertain_sequence,
                            },
                        });
                    }
                }
                (Vec::new(), false)
            }
        };
        for (sid, cid) in incompatible_rejects {
            self.command_rejected(
                &sid,
                cid,
                "protocol downgraded before crashed drain replay".into(),
            );
        }
        if cleared_fence {
            self.flush_pending(session_id);
        }
    }

    /// Resume or cleanly reject any pending crashed-drain commands whose worker
    /// reconnected while no Controller lease was installed (R3-M1).
    pub(super) fn replay_pending_crashed_drains_for_new_controller(&self, lease: u64) {
        let targets: Vec<(String, u64)> = {
            let controller = self.controller.lock();
            if !controller.as_ref().is_some_and(|c| c.lease == lease) {
                return;
            }
            let workers = self.workers.lock();
            let state = self.crashed_drains.inner.lock();
            state
                .pending
                .values()
                .filter_map(|entry| {
                    workers
                        .get(&entry.session_id)
                        .filter(|w| {
                            w.welcomed
                                && w.epoch == entry.epoch
                                && w.connection_id == entry.connection_id
                        })
                        .map(|w| (entry.session_id.clone(), w.connection_id))
                })
                .collect()
        };
        let mut seen = HashSet::new();
        for (sid, conn_id) in targets {
            if seen.insert((sid.clone(), conn_id)) {
                self.replay_pending_crashed_drain(&sid, conn_id);
            }
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
        let (confirmed, retired_launch, recorded_pid) = {
            let mut state = self.crashed_drains.inner.lock();
            let active = state.active.get(session_id).filter(|active| {
                active.epoch == peer.epoch
                    && active.connection_id == peer.connection_id
                    && active.confirmed
            });
            if let Some(active) = active {
                (
                    true,
                    active.retired_launch,
                    active.peer_pid.or(peer.peer_pid),
                )
            } else {
                for pending in state.pending.values_mut() {
                    if pending.session_id == session_id
                        && pending.epoch == peer.epoch
                        && pending.connection_id == peer.connection_id
                    {
                        pending.connection_id = 0;
                    }
                }
                (false, None, None)
            }
        };
        if !confirmed {
            let receiptless = {
                let state = self.crashed_drains.inner.lock();
                let active = state
                    .active
                    .get(session_id)
                    .filter(|a| a.epoch == peer.epoch && a.connection_id == peer.connection_id)
                    .map(|a| (a.peer_pid.or(peer.peer_pid), a.retired_launch));
                let pending = state
                    .pending
                    .values()
                    .find(|p| p.session_id == session_id && p.epoch == peer.epoch)
                    .map(|p| (p.peer_pid.or(peer.peer_pid), p.retired_launch));
                active.or(pending)
            };
            if let Some((pid, launch)) = receiptless {
                self.spawn_receiptless_drain_reconciler(
                    session_id.to_owned(),
                    peer.epoch.clone(),
                    pid,
                    launch,
                );
            }
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

        let teardown_ok = self
            .wait_for_direct_exit_then_confirm_sandbox(session_id, recorded_pid, retired_launch)
            .await;
        self.commit_crashed_drain_disconnect(
            session_id,
            peer,
            recorded_pid,
            retired_launch,
            teardown_ok,
        );
        self.crashed_drains.settled.notify_waiters();
        true
    }

    fn commit_crashed_drain_disconnect(
        self: &Arc<Self>,
        session_id: &str,
        peer: &WorkerPeer,
        recorded_pid: Option<u32>,
        retired_launch: Option<uuid::Uuid>,
        teardown_ok: bool,
    ) {
        let (commit, session_for_cache, orphaned_pending, queued_rejections, retry_entry) = {
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
                (false, None, orphaned, Vec::new(), None)
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
                if teardown_ok {
                    state.unconfirmed_teardown.remove(session_id);
                    (
                        true,
                        sessions.get(session_id).cloned(),
                        orphaned,
                        rejections,
                        None,
                    )
                } else {
                    let entry = UnconfirmedTeardown {
                        epoch: peer.epoch.clone(),
                        peer_pid: recorded_pid,
                        retired_launch,
                    };
                    state
                        .unconfirmed_teardown
                        .insert(session_id.to_owned(), entry.clone());
                    (true, None, orphaned, rejections, Some(entry))
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
        if let Some(entry) = retry_entry {
            let mut snapshot = peer.snapshot.clone();
            snapshot.state = WorkerState::Crashed;
            snapshot.current_turn_id = None;
            snapshot.pending_permissions.clear();
            self.send_controller(Frame::Snapshot {
                worker: Box::new(snapshot),
            });
            self.spawn_background_teardown_retry(session_id.to_owned(), entry);
        } else {
            if let Some(session) = session_for_cache {
                self.revoke_cache_protection(&session, session_id, "crashed_worker_drained");
            }
            self.send_crashed_worker_drained(session_id, &peer.epoch);
        }
    }

    pub(super) async fn wait_for_crashed_drain_before_reset<'a>(
        &self,
        session_id: &str,
        gate: &'a tokio::sync::Mutex<()>,
        mut guard: tokio::sync::MutexGuard<'a, ()>,
    ) -> Result<tokio::sync::MutexGuard<'a, ()>, String> {
        let deadline = tokio::time::Instant::now() + self.args.worker_ready_timeout;
        loop {
            let (live_draining, disconnected_unsettled) = {
                let workers = self.workers.lock();
                let state = self.crashed_drains.inner.lock();
                let live = workers.get(session_id);
                let live_draining = live.is_some_and(|w| w.crashed_drain_fenced);
                let disconnected = if live.is_none() {
                    let active = state.active.get(session_id);
                    let pending = state.pending.values().find(|p| p.session_id == session_id);
                    let unconfirmed = state.unconfirmed_teardown.get(session_id);
                    if active.is_some() || pending.is_some() || unconfirmed.is_some() {
                        let pid = active
                            .and_then(|a| a.peer_pid)
                            .or_else(|| pending.and_then(|p| p.peer_pid))
                            .or_else(|| unconfirmed.and_then(|u| u.peer_pid));
                        let launch = active
                            .and_then(|a| a.retired_launch)
                            .or_else(|| pending.and_then(|p| p.retired_launch))
                            .or_else(|| unconfirmed.and_then(|u| u.retired_launch));
                        Some((pid, launch))
                    } else {
                        None
                    }
                } else {
                    None
                };
                (live_draining, disconnected)
            };
            if !live_draining && disconnected_unsettled.is_none() {
                return Ok(guard);
            }
            if !live_draining
                && let Some((pid, launch)) = disconnected_unsettled
                && self.confirm_mid_drain_worker_exited(session_id, pid).await
                && self
                    .confirm_retired_sandbox_units_gone(session_id, launch)
                    .await
            {
                return Ok(guard);
            }
            if !lease_active(self) {
                return Err("controller lease changed while awaiting crashed drain".into());
            }
            let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now())
            else {
                return Err("crashed worker drain or process exit is still settling; retry".into());
            };
            let notified = self.crashed_drains.settled.notified();
            drop(guard);
            let _ = tokio::time::timeout(remaining.min(Duration::from_millis(100)), notified).await;
            guard = gate.lock().await;
        }
    }

    pub(super) async fn gate_ensure_against_crashed_drain(
        &self,
        session_id: &str,
    ) -> Result<(), String> {
        if self.launching.lock().contains(session_id) {
            return Ok(());
        }
        let (fenced_live, has_live, unsettled, unconfirmed) = {
            let workers = self.workers.lock();
            let state = self.crashed_drains.inner.lock();
            let live = workers.get(session_id);
            let active_entry = state.active.get(session_id);
            let pending_entry = state
                .pending
                .values()
                .find(|pending| pending.session_id == session_id);
            let unsettled = (active_entry.is_some() || pending_entry.is_some()).then(|| {
                (
                    active_entry
                        .and_then(|e| e.peer_pid)
                        .or_else(|| pending_entry.and_then(|e| e.peer_pid)),
                    active_entry
                        .and_then(|e| e.retired_launch)
                        .or_else(|| pending_entry.and_then(|e| e.retired_launch)),
                )
            });
            (
                live.is_some_and(|worker| worker.crashed_drain_fenced),
                live.is_some(),
                unsettled,
                state.unconfirmed_teardown.get(session_id).cloned(),
            )
        };
        if fenced_live {
            return Err("crashed worker drain is in progress; retry".into());
        }
        if has_live || (unsettled.is_none() && unconfirmed.is_none()) {
            return Ok(());
        }
        if let Some((pid, launch)) = unsettled {
            if !self.confirm_mid_drain_worker_exited(session_id, pid).await {
                return Err("crashed worker drain is still settling; retry".into());
            }
            if !self
                .confirm_retired_sandbox_units_gone(session_id, launch)
                .await
            {
                return Err(
                    "previous crashed worker sandbox unit teardown is unconfirmed; retry".into(),
                );
            }
        }
        if let Some(entry) = &unconfirmed
            && !self
                .confirm_retired_teardown_complete(session_id, entry.peer_pid, entry.retired_launch)
                .await
        {
            return Err("previous crashed worker teardown is still unconfirmed; retry".into());
        }
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
            let emit = unconfirmed
                .as_ref()
                .filter(|entry| {
                    state
                        .unconfirmed_teardown
                        .get(session_id)
                        .is_some_and(|recorded| recorded.epoch == entry.epoch)
                })
                .map(|entry| entry.epoch.clone());
            let rejections = if emit.is_some() {
                state.unconfirmed_teardown.remove(session_id);
                classify_and_drain_queued_commands(&mut pending_cmds, &mut dispatched, session_id)
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
        Ok(())
    }

    pub(super) fn clear_crashed_drain(&self, session_id: &str, reason: &str) {
        let orphaned = {
            let mut state = self.crashed_drains.inner.lock();
            state.active.remove(session_id);
            state.unconfirmed_teardown.remove(session_id);
            drain_pending_for_session(&mut state.pending, session_id)
        };
        for (sid, cid) in orphaned {
            self.command_rejected(&sid, cid, reason.to_owned());
        }
        self.crashed_drains.settled.notify_waiters();
    }

    pub(super) fn unconfirmed_sandbox_epoch(&self, session_id: &str) -> Option<String> {
        let state = self.crashed_drains.inner.lock();
        state
            .unconfirmed_teardown
            .get(session_id)
            .map(|entry| entry.epoch.clone())
            .or_else(|| {
                state
                    .active
                    .get(session_id)
                    .map(|entry| entry.epoch.clone())
            })
            .or_else(|| {
                state
                    .pending
                    .values()
                    .find(|entry| entry.session_id == session_id)
                    .map(|entry| entry.epoch.clone())
            })
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

pub(super) fn classify_and_drain_queued_commands(
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

fn drain_pending_for_epoch(
    pending: &mut HashMap<String, PendingDrain>,
    session_id: &str,
    epoch: &str,
) -> Vec<(String, String)> {
    let ids: Vec<String> = pending
        .iter()
        .filter(|(_, entry)| entry.session_id == session_id && entry.epoch == epoch)
        .map(|(id, _)| id.clone())
        .collect();
    ids.into_iter()
        .filter_map(|id| pending.remove(&id).map(|entry| (entry.session_id, id)))
        .collect()
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

        assert_eq!(
            broker.record_crashed_drain_ack("s-1", "100-1", 1, "dc-1", true),
            (true, false)
        );
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
        assert_eq!(
            broker.record_crashed_drain_ack("s-2", "200-1", 1, "dc-2", true),
            (true, false)
        );
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
        assert_eq!(
            broker.record_crashed_drain_ack("s-rebind", "400-1", 1, "dc-rebind", false),
            (false, false)
        );
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
        assert_eq!(
            broker.record_crashed_drain_ack("s-ambig", "500-1", 1, "dc-ambig", true),
            (true, false)
        );
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
