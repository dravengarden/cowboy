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
    /// `(worker_epoch, confirmed_receipt, retired_at)` for exact epoch-bound
    /// handover replay (`confirmed_receipt == true` -> `CrashedWorkerDrained` on
    /// capable Controller, or ownerless correction on downgraded Controller;
    /// `confirmed_receipt == false` -> ownerless correction only, R8-M5).
    recently_drained: HashMap<String, (String, bool, Instant)>,
    /// Last trusted Direct PID observed for `(session_id -> (epoch, pid))`,
    /// preserved across same-epoch reconnects and cleared on epoch change or
    /// confirmed process exit (R5-M5).
    trusted_epoch_pids: HashMap<String, (String, u32)>,
    /// Session-scoped highest unpersisted/receiptless `UncertainEffect`
    /// retained across worker replacement and reset until durably acknowledged
    /// by the owner or the session is permanently deleted (R9-H3).
    receiptless_uncertain_effects: HashMap<String, crate::runtime_wire::UncertainEffect>,
}

/// Durable launch ownership proof for a retiring crashed worker (R7-H6).
/// Distinguishes a confirmed absent durable record (`VerifiedNone`) from a
/// transient store-read error (`Unresolved`, which fails closed until resolved).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::machine_broker) enum RetiredLaunch {
    Verified(uuid::Uuid),
    VerifiedNone,
    Unresolved,
}

impl RetiredLaunch {
    fn or(self, other: Self) -> Self {
        if self == Self::Unresolved {
            other
        } else {
            self
        }
    }
}

struct PendingDrain {
    session_id: String,
    epoch: String,
    /// `0` while the worker is disconnected with an ambiguous ACK; rebound and
    /// replayed on same-epoch reconnect.
    connection_id: u64,
    acknowledged_uncertain_sequence: u64,
    peer_pid: Option<u32>,
    retired_launch: RetiredLaunch,
    /// True ONLY after a same-epoch disconnect/reconnect when the command must
    /// be replayed; false while the live worker's ACK is already in flight (R4-M1).
    needs_replay: bool,
}

struct ActiveDrain {
    epoch: String,
    connection_id: u64,
    command_id: Option<String>,
    confirmed: bool,
    peer_pid: Option<u32>,
    retired_launch: RetiredLaunch,
}

#[derive(Clone)]
struct UnconfirmedTeardown {
    epoch: String,
    peer_pid: Option<u32>,
    retired_launch: RetiredLaunch,
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
            Ok(Some(launch)) => RetiredLaunch::Verified(launch),
            Ok(None) => RetiredLaunch::VerifiedNone,
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
        retired_launch: RetiredLaunch,
    ) -> DrainAdmission {
        let controller = self.controller.lock();
        let expected_lease = active_command_lease();
        let controller_meta = controller
            .as_ref()
            .filter(|ctrl| {
                expected_lease.is_none_or(|lease| ctrl.lease == lease)
                    && ctrl.protocol >= CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
                    && ctrl
                        .outer_machine_protocol
                        .is_some_and(|proto| proto >= CRASHED_WORKER_DRAIN_PROTOCOL_VERSION)
            })
            .map(|ctrl| (ctrl.tx.clone(), ctrl.protocol, ctrl.outer_machine_protocol));
        let Some((ctrl_tx, ctrl_protocol, ctrl_outer)) = controller_meta else {
            return DrainAdmission::Rejected(
                "controller protocol does not support crashed worker drain".into(),
            );
        };
        let sessions = self.sessions.lock();
        let mut workers = self.workers.lock();
        let pending_cmds = self.pending_commands.lock();
        let dispatched = self.dispatched_prompts.lock();
        let pending_core = self.pending_core_commands.lock();
        let mut state = self.crashed_drains.inner.lock();
        let replacing = self.replacing.lock();
        if replacing.contains_key(session_id) {
            return DrainAdmission::Rejected("session rollout replacement is in progress".into());
        }
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
                let trusted_pid = worker
                    .peer_pid
                    .or_else(|| self.direct_worker_pids.lock().get(session_id).copied());
                if self.args.spawn_mode == SpawnMode::Direct && trusted_pid.is_none() {
                    return DrainAdmission::Rejected(
                        "direct worker lacks verifiable process identity for drain teardown".into(),
                    );
                }
                if trusted_pid.is_some() {
                    worker.peer_pid = trusted_pid;
                }
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
                            peer_pid: trusted_pid,
                            retired_launch,
                            needs_replay: false,
                        },
                    );
                    // Publish broker-verified `crashed_drain_active: true`
                    // atomically under `controller -> sessions -> workers`
                    // BEFORE any trailing worker event can be forwarded to the
                    // Controller (R9-H2).
                    let mut snap = worker.snapshot.clone();
                    if snap.launch.is_none() {
                        snap.launch = sessions.get(session_id).cloned();
                    }
                    snap.crashed_drain_active = true;
                    snap.incarnation = self.reported_lineage(session_id);
                    let declared_provider = sessions.get(session_id).map(|s| s.provider.as_str());
                    wrap_up::sanitize_snapshot_for_controller(
                        &mut snap,
                        worker.raw_wrap_up_steering,
                        worker.negotiated_protocol,
                        ctrl_outer,
                        ctrl_protocol,
                        declared_provider,
                    );
                    let _ = ctrl_tx.send(Frame::Snapshot {
                        worker: Box::new(snap),
                    });
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
            // Publish an ordered snapshot clearing `crashed_drain_active` even
            // when the prompt queue was empty so a post-handover Controller
            // without the original pending command clears its drain marker (R12-M1).
            self.publish_snapshot_after_crashed_flush(&sid, true);
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
                    command_id: Some(command_id.to_owned()),
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
                .map(|pending| pending.retired_launch)
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
        let retired_launch = match state
            .active
            .get(session_id)
            .map(|active| active.retired_launch)
        {
            Some(launch) if launch != RetiredLaunch::Unresolved => launch,
            _ => matching_pending_launch.unwrap_or(RetiredLaunch::Unresolved),
        };
        worker.crashed_drain_fenced = true;
        state.active.insert(
            session_id.to_owned(),
            ActiveDrain {
                epoch: epoch.to_owned(),
                connection_id,
                command_id: command_id.map(str::to_owned),
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
        replaced_same_epoch_pid: Option<u32>,
        resumed_drain: Option<(String, RetiredLaunch)>,
        worker: &mut WorkerPeer,
    ) -> Vec<(String, String)> {
        let mut state = self.crashed_drains.inner.lock();
        let mut same_epoch = false;
        let remembered_same_epoch_pid = state
            .trusted_epoch_pids
            .get(session_id)
            .filter(|(recorded_epoch, _)| recorded_epoch == epoch)
            .map(|(_, pid)| *pid);
        if state
            .trusted_epoch_pids
            .get(session_id)
            .is_some_and(|(recorded_epoch, _)| recorded_epoch != epoch)
        {
            state.trusted_epoch_pids.remove(session_id);
        }
        // Invalidate any prior epoch's confirmed drain receipt once a successor
        // takes ownership so the old retirement can never suppress the
        // successor's dormant handover snapshot (R19-M2).
        if state
            .recently_drained
            .get(session_id)
            .is_some_and(|(drained_epoch, _, _)| drained_epoch != epoch)
        {
            state.recently_drained.remove(session_id);
        }
        // Retain the session-scoped highest unpersisted `uncertain_effect`
        // across worker replacement and reset until the owner explicitly
        // acknowledges it or deletes the session (R9-H3).
        if let Some(effect) = state.receiptless_uncertain_effects.get(session_id).cloned() {
            crate::runtime_wire::merge_uncertain_effect(
                &mut worker.snapshot.uncertain_effect,
                effect,
            );
        }
        let mut preserved_pid = worker
            .peer_pid
            .or(replaced_same_epoch_pid)
            .or(remembered_same_epoch_pid)
            .or_else(|| self.direct_worker_pids.lock().get(session_id).copied());
        if let Some(active) = state.active.get_mut(session_id)
            && active.epoch == epoch
        {
            active.connection_id = connection_id;
            preserved_pid = preserved_pid.or(active.peer_pid);
            if preserved_pid.is_some() {
                active.peer_pid = preserved_pid;
            }
            if let Some((ref cmd_id, launch)) = resumed_drain {
                if active.command_id.is_none() {
                    active.command_id = Some(cmd_id.clone());
                }
                if active.retired_launch == RetiredLaunch::Unresolved {
                    active.retired_launch = launch;
                }
            }
            same_epoch = true;
        }
        for pending in state.pending.values_mut() {
            if pending.session_id == session_id && pending.epoch == epoch {
                pending.connection_id = connection_id;
                pending.needs_replay = true;
                preserved_pid = preserved_pid.or(pending.peer_pid);
                if preserved_pid.is_some() {
                    pending.peer_pid = preserved_pid;
                }
                same_epoch = true;
            }
        }
        if let Some(pid) = preserved_pid {
            worker.peer_pid = Some(pid);
            state
                .trusted_epoch_pids
                .insert(session_id.to_owned(), (epoch.to_owned(), pid));
        }
        if same_epoch {
            worker.crashed_drain_fenced = true;
            return Vec::new();
        }
        // Surviving worker reconnecting after broker restart with an accepted
        // drain carried in `Frame::Hello`: install fence + `ActiveDrain`
        // atomically during registration before `Welcome` and `flush_pending`
        // (R7-H4).
        if let Some((cmd_id, retired_launch)) = resumed_drain {
            state.unconfirmed_teardown.remove(session_id);
            let orphaned = drain_pending_for_session(&mut state.pending, session_id);
            state.active.insert(
                session_id.to_owned(),
                ActiveDrain {
                    epoch: epoch.to_owned(),
                    connection_id,
                    command_id: Some(cmd_id),
                    confirmed: false,
                    peer_pid: preserved_pid,
                    retired_launch,
                },
            );
            worker.crashed_drain_fenced = true;
            return orphaned;
        }
        state.active.remove(session_id);
        state.unconfirmed_teardown.remove(session_id);
        drain_pending_for_session(&mut state.pending, session_id)
    }

    fn has_crashed_drain_for_epoch_locked(state: &Inner, session_id: &str, epoch: &str) -> bool {
        state
            .active
            .get(session_id)
            .is_some_and(|a| a.epoch == epoch)
            || state
                .pending
                .values()
                .any(|p| p.session_id == session_id && p.epoch == epoch)
    }

    pub(super) fn has_crashed_drain_for_epoch(&self, session_id: &str, epoch: &str) -> bool {
        let state = self.crashed_drains.inner.lock();
        Self::has_crashed_drain_for_epoch_locked(&state, session_id, epoch)
    }

    /// Stamp live `pending_prompt_count` and `crashed_drain_active` and enqueue
    /// `Frame::Snapshot` atomically under `pending_commands -> crashed_drains`
    /// so concurrent queue mutations and drain transitions can never publish
    /// snapshots out of order (R11-H3, R13-M5).
    pub(super) fn send_snapshot_ordered_with_crashed_drain(
        &self,
        tx: &mpsc::UnboundedSender<Frame>,
        mut worker: Box<WorkerSnapshot>,
    ) {
        let pending = self.pending_commands.lock();
        let state = self.crashed_drains.inner.lock();
        worker.pending_prompt_count = pending.get(&worker.session_id).map_or(0, |commands| {
            u64::try_from(
                commands
                    .iter()
                    .filter(|cmd| matches!(cmd, WorkerCommand::Prompt { .. }))
                    .count(),
            )
            .unwrap_or(u64::MAX)
        });
        worker.crashed_drain_active = worker.has_connected_owner()
            && Self::has_crashed_drain_for_epoch_locked(
                &state,
                &worker.session_id,
                &worker.worker_epoch,
            );
        let _ = tx.send(Frame::Snapshot { worker });
    }

    /// Atomically serialize rollout cutover admission against crashed-worker
    /// drain under the shared `workers -> crashed_drains -> replacing` lock
    /// boundary and enqueue `Stop` onto the exact validated peer (R4-H1).
    /// Returns the validated Direct PID when a Direct watchdog should be armed.
    pub(super) fn try_admit_rollout_cutover(&self, session_id: &str) -> Option<u32> {
        let workers = self.workers.lock();
        let state = self.crashed_drains.inner.lock();
        let mut replacing = self.replacing.lock();
        let worker = workers.get(session_id)?;
        let snapshot = &worker.snapshot;
        let has_drain = state
            .active
            .get(session_id)
            .is_some_and(|a| a.epoch == worker.epoch)
            || state
                .pending
                .values()
                .any(|p| p.session_id == session_id && p.epoch == worker.epoch);
        if worker.crashed_drain_fenced
            || has_drain
            || !snapshot.drain_requested
            || snapshot.current_turn_id.is_some()
            || !snapshot.pending_permissions.is_empty()
            || !matches!(snapshot.state, WorkerState::Running | WorkerState::Draining)
            || replacing.contains_key(session_id)
        {
            return None;
        }
        let command = self.trace_worker_dispatch(
            session_id,
            WorkerCommand::Stop {
                command_id: format!("rollout-stop-{session_id}"),
            },
        );
        if worker
            .tx
            .send(Frame::WorkerCommand {
                session_id: session_id.to_owned(),
                command,
            })
            .is_err()
        {
            return None;
        }
        replacing.insert(session_id.to_owned(), snapshot.generation.clone());
        if self.args.spawn_mode == SpawnMode::Direct {
            worker
                .peer_pid
                .or_else(|| self.direct_worker_pids.lock().get(session_id).copied())
        } else {
            None
        }
    }

    /// After sending `Welcome` to a reconnected worker, replay any ambiguous
    /// pending `DrainCrashed` command under canonical `controller -> workers ->
    /// crashed_drains` lock order. When no Controller lease is currently
    /// attached (handover gap), preserve pending state and fence intact until a
    /// lease attaches (R3-M1).
    pub(super) fn replay_pending_crashed_drain(&self, session_id: &str, connection_id: u64) {
        let (incompatible_rejects, cleared_fence) = {
            let controller = self.controller.lock();
            if controller.is_none() {
                // Handover gap: keep ambiguous ownership fenced until next lease installs.
                return;
            }
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
                    entry.needs_replay
                        && entry.session_id == session_id
                        && entry.epoch == worker.epoch
                        && entry.connection_id == connection_id
                })
                .map(|(cid, _)| cid.clone())
                .collect();
            if matching_ids.is_empty() {
                return;
            }
            // Only clear if the *worker* itself downgraded below drain support;
            // a downgraded Controller must NEVER release an ambiguous drain (R4-M1).
            if worker.negotiated_protocol < CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION {
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
                    if let Some(entry) = state.pending.get_mut(&cid) {
                        let _ = worker.tx.send(Frame::WorkerCommand {
                            session_id: session_id.to_owned(),
                            command: WorkerCommand::DrainCrashed {
                                command_id: cid,
                                worker_epoch: entry.epoch.clone(),
                                acknowledged_uncertain_sequence: entry
                                    .acknowledged_uncertain_sequence,
                            },
                        });
                        entry.needs_replay = false;
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
                    if !entry.needs_replay {
                        return None;
                    }
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
            let mut state = self.crashed_drains.inner.lock();
            let remembered_pid = state
                .trusted_epoch_pids
                .get(session_id)
                .filter(|(recorded_epoch, _)| recorded_epoch == &worker.epoch)
                .map(|(_, recorded_pid)| *recorded_pid);
            let resolved = pid
                .or(worker.peer_pid)
                .or(remembered_pid)
                .or_else(|| self.direct_worker_pids.lock().get(session_id).copied());
            if let Some(pid_val) = resolved {
                worker.peer_pid = Some(pid_val);
                state
                    .trusted_epoch_pids
                    .insert(session_id.to_owned(), (worker.epoch.clone(), pid_val));
            }
            if let Some(active) = state.active.get_mut(session_id)
                && active.connection_id == connection_id
                && resolved.is_some()
            {
                active.peer_pid = resolved;
            }
            for pending in state.pending.values_mut() {
                if pending.session_id == session_id
                    && pending.connection_id == connection_id
                    && resolved.is_some()
                {
                    pending.peer_pid = resolved;
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
                        pending.needs_replay = true;
                    }
                }
                (false, RetiredLaunch::Unresolved, None)
            }
        };
        if !confirmed {
            let receiptless = {
                let mut state = self.crashed_drains.inner.lock();
                if let Some(effect) = peer.snapshot.uncertain_effect.clone() {
                    let mut merged = state.receiptless_uncertain_effects.get(session_id).cloned();
                    crate::runtime_wire::merge_uncertain_effect(&mut merged, effect);
                    if let Some(merged) = merged {
                        state
                            .receiptless_uncertain_effects
                            .insert(session_id.to_owned(), merged);
                    }
                }
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
        retired_launch: RetiredLaunch,
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
                    state.trusted_epoch_pids.remove(session_id);
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
                        let epoch_for_launch = active
                            .map(|a| a.epoch.clone())
                            .or_else(|| pending.map(|p| p.epoch.clone()))
                            .or_else(|| unconfirmed.map(|u| u.epoch.clone()))
                            .unwrap_or_default();
                        let launch =
                            active
                                .map_or(RetiredLaunch::Unresolved, |a| a.retired_launch)
                                .or(pending.map_or(RetiredLaunch::Unresolved, |p| p.retired_launch))
                                .or(unconfirmed
                                    .map_or(RetiredLaunch::Unresolved, |u| u.retired_launch));
                        Some((pid, epoch_for_launch, launch))
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
            if !live_draining && let Some((pid, epoch_for_launch, launch)) = disconnected_unsettled
            {
                let launch = self
                    .resolve_or_refresh_retired_launch(session_id, &epoch_for_launch, launch)
                    .await;
                if self.confirm_mid_drain_worker_exited(session_id, pid).await
                    && self
                        .confirm_retired_sandbox_units_gone(session_id, launch)
                        .await
                {
                    return Ok(guard);
                }
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
                        .map(|e| e.epoch.clone())
                        .or_else(|| pending_entry.map(|e| e.epoch.clone()))
                        .unwrap_or_default(),
                    active_entry
                        .map_or(RetiredLaunch::Unresolved, |e| e.retired_launch)
                        .or(pending_entry.map_or(RetiredLaunch::Unresolved, |e| e.retired_launch)),
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
        if let Some((pid, epoch_for_launch, launch)) = unsettled {
            let launch = self
                .resolve_or_refresh_retired_launch(session_id, &epoch_for_launch, launch)
                .await;
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
        if let Some(entry) = &unconfirmed {
            let launch = self
                .resolve_or_refresh_retired_launch(session_id, &entry.epoch, entry.retired_launch)
                .await;
            if !self
                .confirm_retired_teardown_complete(session_id, entry.peer_pid, launch)
                .await
            {
                return Err("previous crashed worker teardown is still unconfirmed; retry".into());
            }
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

    pub(super) fn clear_crashed_drain(
        &self,
        session_id: &str,
        reason: &str,
        permanent_delete: bool,
    ) {
        let orphaned = {
            let mut state = self.crashed_drains.inner.lock();
            state.active.remove(session_id);
            state.unconfirmed_teardown.remove(session_id);
            state.trusted_epoch_pids.remove(session_id);
            if permanent_delete {
                state.receiptless_uncertain_effects.remove(session_id);
            }
            drain_pending_for_session(&mut state.pending, session_id)
        };
        for (sid, cid) in orphaned {
            self.command_rejected(&sid, cid, reason.to_owned());
        }
        self.crashed_drains.settled.notify_waiters();
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
                None,
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
        assert!(matches!(
            ctrl_rx.try_recv(),
            Ok(Frame::Snapshot { ref worker })
                if worker.session_id == "s-1"
                    && worker.worker_epoch == "100-1"
                    && worker.crashed_drain_active
        ));

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
        assert!(matches!(
            ctrl_rx.try_recv(),
            Ok(Frame::Snapshot { ref worker })
                if worker.session_id == "s-ambig" && worker.crashed_drain_active
        ));
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

    #[tokio::test]
    async fn unresolved_launch_fails_closed_and_same_epoch_reconnect_preserves_receiptless_uncertain()
     {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        // R7-H6: Unresolved durable launch identity must fail closed.
        assert!(
            !broker
                .confirm_retired_sandbox_units_gone("s-unresolved", RetiredLaunch::Unresolved)
                .await
        );
        assert!(
            broker
                .confirm_retired_sandbox_units_gone("s-unresolved", RetiredLaunch::VerifiedNone)
                .await
        );

        // R7-H5: Receiptless uncertain effect is preserved across same-epoch
        // reconnect (even when no active/pending drain exists, e.g. after abort)
        // and cleared only on epoch replacement.
        broker
            .crashed_drains
            .inner
            .lock()
            .receiptless_uncertain_effects
            .insert(
                "s-r7-h5".into(),
                crate::runtime_wire::UncertainEffect {
                    method: "write".into(),
                    sequence: 9,
                },
            );
        let _rx1 = attach_crashed_worker(
            &broker,
            "s-r7-h5",
            "705-1",
            1,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        assert_eq!(
            broker
                .workers
                .lock()
                .get("s-r7-h5")
                .and_then(|w| w.snapshot.uncertain_effect.as_ref())
                .map(|e| e.sequence),
            Some(9)
        );
        assert!(
            broker
                .crashed_drains
                .inner
                .lock()
                .receiptless_uncertain_effects
                .contains_key("s-r7-h5")
        );
        // R8-M4: A subsequent same-epoch Snapshot with no uncertain_effect must
        // preserve the broker's retained sequence via merge.
        let mut weaker_snap = broker
            .workers
            .lock()
            .get("s-r7-h5")
            .unwrap()
            .snapshot
            .clone();
        weaker_snap.uncertain_effect = None;
        assert!(broker.update_snapshot(weaker_snap, 1));
        assert_eq!(
            broker
                .workers
                .lock()
                .get("s-r7-h5")
                .and_then(|w| w.snapshot.uncertain_effect.as_ref())
                .map(|e| e.sequence),
            Some(9)
        );

        // R9-H3: Replacement epoch must STILL preserve the session-scoped
        // unresolved uncertain_effect until durably acknowledged or deleted!
        broker.remove_worker("s-r7-h5", 1);
        let _rx2 = attach_crashed_worker(
            &broker,
            "s-r7-h5",
            "705-2",
            2,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        assert_eq!(
            broker
                .workers
                .lock()
                .get("s-r7-h5")
                .and_then(|w| w.snapshot.uncertain_effect.as_ref())
                .map(|e| e.sequence),
            Some(9)
        );
        // Permanent session deletion clears the retained uncertain effect.
        // Reset (`permanent_delete = false`) must preserve the session-scoped effect.
        broker.clear_crashed_drain("s-r7-h5", "reset", false);
        assert!(
            broker
                .crashed_drains
                .inner
                .lock()
                .receiptless_uncertain_effects
                .contains_key("s-r7-h5")
        );
        // Permanent session deletion clears the retained uncertain effect.
        broker.clear_crashed_drain("s-r7-h5", "deleted", true);
        assert!(
            !broker
                .crashed_drains
                .inner
                .lock()
                .receiptless_uncertain_effects
                .contains_key("s-r7-h5")
        );
    }

    #[tokio::test]
    async fn handover_reconciliation_stamps_crashed_drain_active_and_defaults_dormant_to_starting()
    {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        let dormant: StartSession = serde_json::from_value(serde_json::json!({
            "session_id": "s-dormant",
            "provider": "codex",
            "cwd": "/tmp",
            "generation": "gen-1"
        }))
        .unwrap();
        broker.sessions.lock().insert("s-dormant".into(), dormant);
        // Intentionally leave `session_states` empty for `s-dormant` (adopt-only).

        let _w_rx = attach_crashed_worker(
            &broker,
            "s-draining",
            "50-1",
            1,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        broker
            .workers
            .lock()
            .get_mut("s-draining")
            .unwrap()
            .crashed_drain_fenced = true;
        broker.crashed_drains.inner.lock().active.insert(
            "s-draining".into(),
            ActiveDrain {
                epoch: "50-1".into(),
                connection_id: 1,
                command_id: None,
                confirmed: false,
                peer_pid: None,
                retired_launch: RetiredLaunch::VerifiedNone,
            },
        );

        let (ctrl_tx, mut ctrl_rx) = mpsc::unbounded_channel();
        post_edit::install_controller_and_arm_replays(
            &broker,
            Controller {
                lease: 1,
                tx: ctrl_tx,
                protocol: CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
                outer_machine_protocol: Some(CRASHED_WORKER_DRAIN_PROTOCOL_VERSION),
            },
            None,
            &[],
        );

        let mut saw_draining = false;
        let mut saw_dormant_starting = false;
        while let Ok(frame) = ctrl_rx.try_recv() {
            if let Frame::Snapshot { worker } = frame {
                if worker.session_id == "s-draining" {
                    assert!(
                        worker.crashed_drain_active,
                        "handover reconciliation must stamp crashed_drain_active=true"
                    );
                    saw_draining = true;
                } else if worker.session_id == "s-dormant" {
                    assert_eq!(
                        worker.state,
                        WorkerState::Starting,
                        "missing session_states entry must default to Starting, not Crashed"
                    );
                    saw_dormant_starting = true;
                }
            }
        }
        assert!(
            saw_draining,
            "expected draining worker snapshot on handover"
        );
        assert!(
            saw_dormant_starting,
            "expected dormant Starting snapshot on handover"
        );
    }

    #[tokio::test]
    async fn reset_records_departing_worker_uncertain_effect_even_without_controller() {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        let _w_rx = attach_crashed_worker(
            &broker,
            "s-r11-h1",
            "60-1",
            1,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        // Simulate non-permanent reset: cancelled_sessions set, no Controller connected.
        broker.cancelled_sessions.lock().insert("s-r11-h1".into());
        let event = RuntimeEvent::Update {
            cmid: None,
            update: serde_json::json!({
                "sessionUpdate": "cowboy_uncertain_effect",
                "method": "tool/write",
                "sequence": 7
            }),
        };
        broker.handle_worker_event(
            "s-r11-h1",
            1,
            "s-r11-h1".into(),
            "60-1".into(),
            1,
            event.clone(),
            Vec::new(),
        );
        assert_eq!(
            broker
                .retained_receiptless_uncertain("s-r11-h1")
                .map(|e| e.sequence),
            Some(7),
            "departing worker uncertain_effect during reset must persist even without Controller"
        );
        // Permanent deletion must NOT record new uncertain effects.
        broker.clear_crashed_drain("s-r11-h1", "deleted", true);
        broker
            .permanently_deleted_sessions
            .lock()
            .insert("s-r11-h1".into());
        broker.handle_worker_event(
            "s-r11-h1",
            1,
            "s-r11-h1".into(),
            "60-1".into(),
            2,
            event,
            Vec::new(),
        );
        assert!(broker.retained_receiptless_uncertain("s-r11-h1").is_none());
    }

    #[tokio::test]
    async fn broker_execution_boundary_restricts_draining_crashed_epoch_to_cleanup_only() {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        let mut w_rx = attach_crashed_worker(
            &broker,
            "s-r11-h2",
            "61-1",
            1,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        let binding = crate::execution_environment::BindingV1 {
            schema: 1,
            id: "binding-r11".into(),
            revision: 1,
            runtime: crate::execution_environment::RuntimeLocation {
                machine_id: "runtime".into(),
                cwd: "/runtime".into(),
            },
            environment: crate::execution_environment::EnvironmentLocation {
                machine_id: "target".into(),
                id: "environment".into(),
                incarnation: "incarnation".into(),
                executor_digest: format!("sha256:{}", "a".repeat(64)),
                protocol: 1,
            },
            workspace: crate::execution_environment::WorkspaceLocation {
                id: "project".into(),
                worktree_id: "session".into(),
                source_path: "/source".into(),
                cwd: "/target".into(),
            },
            access: crate::execution_environment::ExecutionAccess::Project,
            managed: None,
        };
        let encoded = crate::execution_environment::ExecutionBinding::from_record(
            serde_json::to_value(&binding).unwrap(),
        );
        broker
            .sessions
            .lock()
            .get_mut("s-r11-h2")
            .unwrap()
            .execution_binding = Some(encoded);
        broker.crashed_drains.inner.lock().active.insert(
            "s-r11-h2".into(),
            ActiveDrain {
                epoch: "61-1".into(),
                connection_id: 1,
                command_id: None,
                confirmed: false,
                peer_pid: None,
                retired_launch: RetiredLaunch::VerifiedNone,
            },
        );
        let make_req = |command: crate::execution_protocol::Command| {
            crate::execution_protocol::RuntimeRequest {
                session_id: "s-r11-h2".into(),
                worker_epoch: "61-1".into(),
                request_id: "req-1".into(),
                binding: binding.clone(),
                command,
            }
        };
        let terminate = make_req(crate::execution_protocol::Command::Invoke {
            invocation: crate::execution_protocol::Invocation {
                operation_id: "op-1".into(),
                method: "process/terminate".into(),
                params: serde_json::json!({}),
            },
            wait_ms: 1000,
        });
        let stop_mcp = make_req(crate::execution_protocol::Command::Invoke {
            invocation: crate::execution_protocol::Invocation {
                operation_id: "op-2".into(),
                method: crate::execution_host::user_mcp::USER_MCP_STOP_METHOD.into(),
                params: serde_json::json!({}),
            },
            wait_ms: 1000,
        });
        let start_proc = make_req(crate::execution_protocol::Command::Invoke {
            invocation: crate::execution_protocol::Invocation {
                operation_id: "op-3".into(),
                method: "process/start".into(),
                params: serde_json::json!({}),
            },
            wait_ms: 1000,
        });
        let mut ctrl_rx = attach_controller(
            &broker,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
            Some(CRASHED_WORKER_DRAIN_PROTOCOL_VERSION),
        );
        broker.forward_or_refuse_execution_request("s-r11-h2", "61-1", 1, Box::new(terminate));
        broker.forward_or_refuse_execution_request("s-r11-h2", "61-1", 1, Box::new(stop_mcp));
        broker.forward_or_refuse_execution_request("s-r11-h2", "61-1", 1, Box::new(start_proc));
        broker.forward_or_refuse_execution_request(
            "s-r11-h2",
            "61-1",
            1,
            Box::new(make_req(crate::execution_protocol::Command::Describe)),
        );
        assert!(matches!(
            ctrl_rx.try_recv().expect("terminate forwarded"),
            Frame::ExecutionRequest { .. }
        ));
        assert!(matches!(
            ctrl_rx.try_recv().expect("stop_mcp forwarded"),
            Frame::ExecutionRequest { .. }
        ));
        assert!(
            ctrl_rx.try_recv().is_err(),
            "non-cleanup execution requests must never be forwarded during crashed drain"
        );
        // Both non-cleanup commands must receive IdentityMismatch refusals on worker channel.
        for _ in 0..2 {
            assert!(matches!(
                w_rx.try_recv().expect("worker refusal"),
                Frame::ExecutionReply { reply }
                    if matches!(
                        reply.response,
                        crate::machine_protocol::execution::Response::Refused {
                            reason: crate::machine_protocol::execution::Refusal::IdentityMismatch
                        }
                    )
            ));
        }
    }

    #[tokio::test]
    async fn flushing_rollout_prompts_on_crashed_worker_publishes_refreshed_queue_count() {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        let mut ctrl_rx = attach_controller(
            &broker,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
            Some(CRASHED_WORKER_DRAIN_PROTOCOL_VERSION),
        );
        let _w_rx = attach_crashed_worker(
            &broker,
            "s-r11-m1",
            "62-1",
            1,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        broker
            .workers
            .lock()
            .get_mut("s-r11-m1")
            .unwrap()
            .snapshot
            .drain_requested = true;
        broker
            .pending_commands
            .lock()
            .entry("s-r11-m1".into())
            .or_default()
            .push_back(WorkerCommand::Prompt {
                command_id: "cmd-r11-m1".into(),
                content: Vec::new(),
                turn_id: "t-1".into(),
                cmid: None,
                expected_worker_epoch: None,
                trace: None,
                echo_artifacts: false,
                observe_subagents: false,
                acknowledged_uncertain_sequence: None,
            });
        broker.flush_pending_guarded("s-r11-m1");
        let first = ctrl_rx.try_recv().expect("rejection ack");
        assert!(matches!(
            first,
            Frame::CommandAck {
                ref command_id,
                accepted: false,
                ..
            } if command_id == "cmd-r11-m1"
        ));
        let second = ctrl_rx.try_recv().expect("refreshed snapshot");
        match second {
            Frame::Snapshot { worker } => {
                assert_eq!(worker.session_id, "s-r11-m1");
                assert_eq!(worker.pending_prompt_count, 0);
            }
            other => panic!("expected refreshed Frame::Snapshot, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn genuine_drain_refusal_publishes_cleared_snapshot_and_late_abort_after_delete_is_ignored()
     {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        let mut ctrl_rx = attach_controller(
            &broker,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
            Some(CRASHED_WORKER_DRAIN_PROTOCOL_VERSION),
        );
        let _w_rx = attach_crashed_worker(
            &broker,
            "s-r12-m1",
            "70-1",
            1,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        // Simulate pending drain with empty prompt queue, then worker refuses (R12-M1).
        broker
            .workers
            .lock()
            .get_mut("s-r12-m1")
            .unwrap()
            .crashed_drain_fenced = true;
        broker.crashed_drains.inner.lock().pending.insert(
            "dc-refuse".into(),
            PendingDrain {
                session_id: "s-r12-m1".into(),
                epoch: "70-1".into(),
                connection_id: 1,
                acknowledged_uncertain_sequence: 0,
                peer_pid: None,
                retired_launch: RetiredLaunch::VerifiedNone,
                needs_replay: false,
            },
        );
        broker.handle_worker_command_ack(
            "70-1",
            1,
            Frame::CommandAck {
                session_id: "s-r12-m1".into(),
                command_id: "dc-refuse".into(),
                accepted: false,
                reason: Some("busy".into()),
            },
        );
        assert!(matches!(
            ctrl_rx.try_recv().expect("refusal ack"),
            Frame::CommandAck {
                accepted: false,
                ..
            }
        ));
        match ctrl_rx.try_recv().expect("cleared drain snapshot") {
            Frame::Snapshot { worker } => {
                assert_eq!(worker.session_id, "s-r12-m1");
                assert!(!worker.crashed_drain_active);
            }
            other => panic!("expected Frame::Snapshot clearing drain marker, got {other:?}"),
        }

        // Permanent deletion followed by late drain abort must neither retain
        // uncertain_effect nor publish a snapshot (R12-M2).
        broker.clear_crashed_drain("s-r12-m1", "deleted", true);
        broker.cancelled_sessions.lock().insert("s-r12-m1".into());
        broker
            .permanently_deleted_sessions
            .lock()
            .insert("s-r12-m1".into());
        broker.handle_crashed_drain_aborted(
            "s-r12-m1",
            "70-1",
            1,
            Some("dc-late"),
            Some(crate::runtime_wire::UncertainEffect {
                method: "tool/late".into(),
                sequence: 99,
            }),
        );
        assert!(broker.retained_receiptless_uncertain("s-r12-m1").is_none());
        assert!(ctrl_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn stale_peer_uncertain_event_ignored_and_direct_refusal_refreshes_crashed_queue_count() {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        let mut ctrl_rx = attach_controller(
            &broker,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
            Some(CRASHED_WORKER_DRAIN_PROTOCOL_VERSION),
        );
        let _w_rx = attach_crashed_worker(
            &broker,
            "s-r12-m3",
            "71-2",
            2,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        // Stale connection 1 / old epoch 71-1 must NOT pollute retained uncertainty (R12-M3).
        let stale_event = RuntimeEvent::Update {
            cmid: None,
            update: serde_json::json!({
                "sessionUpdate": "cowboy_uncertain_effect",
                "method": "tool/stale",
                "sequence": 42
            }),
        };
        broker.handle_worker_event(
            "s-r12-m3",
            1,
            "s-r12-m3".into(),
            "71-1".into(),
            1,
            stale_event,
            Vec::new(),
        );
        assert!(
            broker.retained_receiptless_uncertain("s-r12-m3").is_none(),
            "stale/replaced peer must not record uncertain_effect"
        );

        // Direct prompt refusal removing queued copy on Crashed worker must
        // publish refreshed snapshot with pending_prompt_count == 0 (R12-M5).
        broker
            .workers
            .lock()
            .get_mut("s-r12-m3")
            .unwrap()
            .crashed_drain_fenced = true;
        let prompt = WorkerCommand::Prompt {
            command_id: "cmd-r12-m5".into(),
            content: Vec::new(),
            turn_id: "t-1".into(),
            cmid: None,
            expected_worker_epoch: None,
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            acknowledged_uncertain_sequence: None,
        };
        broker
            .pending_commands
            .lock()
            .entry("s-r12-m3".into())
            .or_default()
            .push_back(prompt.clone());
        broker.route_prompt_for_optional_lease(Some(1), "s-r12-m3", prompt);
        assert!(matches!(
            ctrl_rx.try_recv().expect("direct refusal ack"),
            Frame::CommandAck {
                ref command_id,
                accepted: false,
                ..
            } if command_id == "cmd-r12-m5"
        ));
        match ctrl_rx
            .try_recv()
            .expect("refreshed snapshot after direct refusal")
        {
            Frame::Snapshot { worker } => {
                assert_eq!(worker.session_id, "s-r12-m3");
                assert_eq!(worker.pending_prompt_count, 0);
            }
            other => panic!("expected refreshed Frame::Snapshot, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn historical_running_replay_cannot_roll_back_crashed_snapshot_or_trigger_rollout_stop() {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        let mut w_rx = attach_crashed_worker(
            &broker,
            "s-r14-h1",
            "80-1",
            1,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        {
            let mut workers = broker.workers.lock();
            let peer = workers.get_mut("s-r14-h1").unwrap();
            peer.snapshot.last_runtime_seq = 10;
            peer.snapshot.drain_requested = true;
            peer.snapshot.state = WorkerState::Crashed;
        }
        // Replayed Running event below watermark (seq 4 < 10) must NOT lower
        // last_runtime_seq, flip state to Running, or send rollout Stop (R14-H1).
        broker.handle_worker_event(
            "s-r14-h1",
            1,
            "s-r14-h1".into(),
            "80-1".into(),
            4,
            RuntimeEvent::Status {
                state: WorkerState::Running,
                detail: None,
            },
            Vec::new(),
        );
        {
            let workers = broker.workers.lock();
            let peer = workers.get("s-r14-h1").unwrap();
            assert_eq!(peer.snapshot.state, WorkerState::Crashed);
            assert_eq!(peer.snapshot.last_runtime_seq, 10);
        }
        assert!(
            w_rx.try_recv().is_err(),
            "historical Running replay on Crashed worker must never trigger rollout Stop"
        );
        // Historical UndeliveredPrompt below watermark must still retire dispatched handoff (R15-L1).
        broker
            .dispatched_prompts
            .lock()
            .record("s-r14-h1", "cmd-undeliv", "80-1");
        assert!(
            broker
                .dispatched_prompts
                .lock()
                .has_unacked_for_epoch_or_ingress("s-r14-h1", "80-1")
        );
        broker.handle_worker_event(
            "s-r14-h1",
            1,
            "s-r14-h1".into(),
            "80-1".into(),
            5,
            RuntimeEvent::UndeliveredPrompt {
                command_id: "cmd-undeliv".into(),
                cmid: None,
                content: Vec::new(),
                error: "crashed".into(),
            },
            Vec::new(),
        );
        assert!(
            !broker
                .dispatched_prompts
                .lock()
                .has_unacked_for_epoch_or_ingress("s-r14-h1", "80-1")
        );
    }

    #[tokio::test]
    async fn abort_with_foreign_command_id_cannot_remove_another_sessions_pending_drain() {
        let dir = tempfile::tempdir().unwrap();
        let broker = test_broker(dir.path());
        let _w_a = attach_crashed_worker(
            &broker,
            "s-a",
            "81-1",
            1,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        let _w_b = attach_crashed_worker(
            &broker,
            "s-b",
            "82-1",
            2,
            CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
        );
        broker.crashed_drains.inner.lock().active.insert(
            "s-a".into(),
            ActiveDrain {
                epoch: "81-1".into(),
                connection_id: 1,
                command_id: Some("dc-a".into()),
                confirmed: false,
                peer_pid: None,
                retired_launch: RetiredLaunch::VerifiedNone,
            },
        );
        broker.crashed_drains.inner.lock().pending.insert(
            "dc-b".into(),
            PendingDrain {
                session_id: "s-b".into(),
                epoch: "82-1".into(),
                connection_id: 2,
                acknowledged_uncertain_sequence: 0,
                peer_pid: None,
                retired_launch: RetiredLaunch::VerifiedNone,
                needs_replay: false,
            },
        );
        // Worker A sends an abort naming session B's pending command ID (R14-M2).
        broker.handle_crashed_drain_aborted("s-a", "81-1", 1, Some("dc-b"), None);
        let state = broker.crashed_drains.inner.lock();
        assert!(
            state.pending.contains_key("dc-b"),
            "session A's abort must never remove session B's pending drain"
        );
        assert!(
            state.active.contains_key("s-a"),
            "mismatched command_id must not clear session A's active drain"
        );
        drop(state);
        // Absent command_id (None) must ALSO fail to clear an active drain with a known ID (R15-M4).
        broker.handle_crashed_drain_aborted("s-a", "81-1", 1, None, None);
        assert!(
            broker
                .crashed_drains
                .inner
                .lock()
                .active
                .contains_key("s-a")
        );
        // Exact matching command_id clears session A's active drain cleanly.
        broker.handle_crashed_drain_aborted("s-a", "81-1", 1, Some("dc-a"), None);
        assert!(
            !broker
                .crashed_drains
                .inner
                .lock()
                .active
                .contains_key("s-a")
        );

        // R16-M4: when an abort consumes a pending drain command, the broker
        // MUST emit a correlated rejection CommandAck before Snapshot so the
        // Controller does not orphan the pending command if the worker drops.
        let (ctrl_tx, mut ctrl_rx) = mpsc::unbounded_channel();
        *broker.controller.lock() = Some(Controller {
            lease: 1,
            tx: ctrl_tx,
            protocol: CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
            outer_machine_protocol: Some(CRASHED_WORKER_DRAIN_PROTOCOL_VERSION),
        });
        broker.handle_crashed_drain_aborted(
            "s-b",
            "82-1",
            2,
            Some("dc-b"),
            Some(crate::runtime_wire::UncertainEffect {
                method: "user_mcp".into(),
                sequence: 1,
            }),
        );
        assert!(
            !broker
                .crashed_drains
                .inner
                .lock()
                .pending
                .contains_key("dc-b")
        );
        match ctrl_rx.try_recv().expect("expected correlated CommandAck") {
            Frame::CommandAck {
                session_id,
                command_id,
                accepted,
                reason,
            } => {
                assert_eq!(session_id, "s-b");
                assert_eq!(command_id, "dc-b");
                assert!(!accepted);
                assert!(
                    reason.as_deref().is_some_and(
                        |r| r.contains("crashed_worker_drain_aborted_uncertain_effect")
                    )
                );
            }
            other => panic!("expected CommandAck before Snapshot, got {other:?}"),
        }
        match ctrl_rx
            .try_recv()
            .expect("expected Snapshot after CommandAck")
        {
            Frame::Snapshot { worker } => {
                assert_eq!(worker.session_id, "s-b");
                assert!(!worker.crashed_drain_active);
                assert_eq!(
                    worker.uncertain_effect.as_ref().map(|e| e.sequence),
                    Some(1)
                );
            }
            other => panic!("expected Snapshot after CommandAck, got {other:?}"),
        }

        // R18-M1: when a confirmed crashed-drain retirement finishes and the
        // worker exits BEFORE the next Welcome is collected (so Welcome carries
        // no connected owner), handover MUST still replay `CrashedWorkerDrained`
        // so the Controller clears stuck Busy.
        let (handover_tx, mut handover_rx) = mpsc::unbounded_channel();
        let handover_ctrl = Controller {
            lease: 2,
            tx: handover_tx,
            protocol: CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION,
            outer_machine_protocol: Some(CRASHED_WORKER_DRAIN_PROTOCOL_VERSION),
        };
        broker
            .crashed_drains
            .inner
            .lock()
            .recently_drained
            .insert("s-gone".into(), ("99-1".into(), true, Instant::now()));
        let sessions = HashMap::new();
        let session_states = HashMap::new();
        let startup_failures = HashMap::new();
        let generation_commands = HashMap::new();
        let empty_workers = HashMap::new();
        broker.enqueue_handover_reconciliation_locked(
            &handover_ctrl,
            &sessions,
            &session_states,
            &startup_failures,
            &generation_commands,
            &empty_workers,
            &[],
        );
        let mut saw_drained = false;
        while let Ok(frame) = handover_rx.try_recv() {
            if matches!(
                frame,
                Frame::CrashedWorkerDrained {
                    ref session_id,
                    ref worker_epoch,
                } if session_id == "s-gone" && worker_epoch == "99-1"
            ) {
                saw_drained = true;
            }
        }
        assert!(
            saw_drained,
            "confirmed retirement must replay on handover even after worker exited before Welcome (R18-M1)"
        );

        // R19-M2: once a successor worker registers for `s-gone`, the prior
        // epoch's confirmed receipt MUST be invalidated so it can never
        // suppress the successor's dormant handover snapshot.
        let (succ_tx, _succ_rx) = mpsc::unbounded_channel();
        let mut successor = account_unit::new_registered_peer(
            "s-gone",
            "100-successor",
            "gen-1",
            None,
            99,
            succ_tx,
            true,
            false,
        );
        let _ = broker.rebind_crashed_drain_on_reconnect(
            "s-gone",
            "100-successor",
            99,
            None,
            None,
            &mut successor,
        );
        assert!(
            !broker
                .crashed_drains
                .inner
                .lock()
                .recently_drained
                .contains_key("s-gone"),
            "successor registration must invalidate prior epoch's recently_drained receipt (R19-M2)"
        );
    }
}
