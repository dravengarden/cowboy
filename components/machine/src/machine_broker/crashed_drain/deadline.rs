//! Broker-side deadline watchdog for cooperative crashed-worker drains.
//!
//! Ensures an accepted or pending crashed drain cannot wedge revive/reset
//! indefinitely if the worker hangs during settlement or before closing its
//! socket.

use super::*;

impl Broker {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn publish_drain_fence_snapshot(
        &self,
        session_id: &str,
        sessions: &HashMap<String, StartSession>,
        pending_cmds: &HashMap<String, VecDeque<WorkerCommand>>,
        worker: &WorkerPeer,
        ctrl_tx: &mpsc::UnboundedSender<Frame>,
        ctrl_outer: Option<u16>,
        ctrl_protocol: u16,
        crashed_drain_active: bool,
    ) {
        let mut snap = worker.snapshot.clone();
        if snap.launch.is_none() {
            snap.launch = sessions.get(session_id).cloned();
        }
        let broker_queued = pending_cmds.get(session_id).map_or(0, |queue| {
            u64::try_from(
                queue
                    .iter()
                    .filter(|cmd| matches!(cmd, WorkerCommand::Prompt { .. }))
                    .count(),
            )
            .unwrap_or(u64::MAX)
        });
        snap.pending_prompt_count = snap.pending_prompt_count.max(broker_queued);
        snap.crashed_drain_active = crashed_drain_active;
        snap.incarnation = self.reported_lineage(session_id);
        let declared = sessions.get(session_id).map(|s| s.provider.as_str());
        wrap_up::sanitize_snapshot_for_controller(
            &mut snap,
            worker.raw_wrap_up_steering,
            worker.negotiated_protocol,
            ctrl_outer,
            ctrl_protocol,
            declared,
        );
        let _ = ctrl_tx.send(Frame::Snapshot {
            worker: Box::new(snap),
        });
    }

    pub(in crate::machine_broker) fn arm_crashed_drain_deadline_if_active(
        self: &Arc<Self>,
        session_id: &str,
        epoch: &str,
        connection_id: u64,
    ) {
        let deadline = {
            let state = self.crashed_drains.inner.lock();
            state
                .active
                .get(session_id)
                .filter(|a| a.epoch == epoch && a.connection_id == connection_id)
                .map(|a| a.deadline)
                .or_else(|| {
                    state
                        .pending
                        .values()
                        .filter(|p| {
                            p.session_id == session_id
                                && p.epoch == epoch
                                && p.connection_id == connection_id
                        })
                        .map(|p| p.deadline)
                        .min()
                })
        };
        if let Some(deadline) = deadline {
            self.spawn_crashed_drain_deadline_watchdog(
                session_id.to_owned(),
                epoch.to_owned(),
                connection_id,
                deadline,
            );
        }
    }

    pub(super) fn spawn_crashed_drain_deadline_watchdog(
        self: &Arc<Self>,
        session_id: String,
        epoch: String,
        connection_id: u64,
        deadline: Instant,
    ) {
        let broker = Arc::clone(self);
        tokio::spawn(async move {
            let wait = deadline.saturating_duration_since(Instant::now());
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
            broker.expire_crashed_drain_if_timed_out(&session_id, &epoch, connection_id);
        });
    }

    fn expire_crashed_drain_if_timed_out(
        self: &Arc<Self>,
        session_id: &str,
        epoch: &str,
        connection_id: u64,
    ) {
        let now = Instant::now();
        let (orphaned, stop_pid, stop_unit, released_fence) = {
            let cancelled = self.cancelled_sessions.lock();
            let deleted = self.permanently_deleted_sessions.lock();
            if cancelled.contains(session_id) || deleted.contains(session_id) {
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
            let pending_cmds = self.pending_commands.lock();
            let mut state = self.crashed_drains.inner.lock();
            let active_expired = state.active.get(session_id).is_some_and(|a| {
                a.epoch == epoch && a.connection_id == connection_id && now >= a.deadline
            });
            let pending_expired = state.pending.values().any(|p| {
                p.session_id == session_id
                    && p.epoch == epoch
                    && p.connection_id == connection_id
                    && now >= p.deadline
            });
            if !active_expired && !pending_expired {
                return;
            }
            let confirmed = state.active.get(session_id).is_some_and(|a| {
                a.epoch == epoch && a.connection_id == connection_id && a.confirmed
            });
            let _ = worker.tx.send(Frame::WorkerCommand {
                session_id: session_id.to_owned(),
                command: WorkerCommand::Stop {
                    command_id: format!("crashed-drain-timeout-{session_id}"),
                },
            });
            let stop_pid = (self.args.spawn_mode == SpawnMode::Direct)
                .then(|| {
                    worker
                        .peer_pid
                        .or_else(|| self.direct_worker_pids.lock().get(session_id).copied())
                })
                .flatten();
            let stop_unit = (self.args.spawn_mode == SpawnMode::SystemdUser)
                .then(|| worker_unit_name(&self.args.socket, session_id));
            if confirmed {
                (Vec::new(), stop_pid, stop_unit, false)
            } else {
                worker.crashed_drain_fenced = false;
                if state
                    .active
                    .get(session_id)
                    .is_some_and(|a| a.epoch == epoch && a.connection_id == connection_id)
                {
                    state.active.remove(session_id);
                }
                let orphaned = drain_pending_for_connection(
                    &mut state.pending,
                    session_id,
                    epoch,
                    connection_id,
                );
                if let Some(ctrl) = ctrl_guard.as_ref() {
                    self.publish_drain_fence_snapshot(
                        session_id,
                        &sessions,
                        &pending_cmds,
                        worker,
                        &ctrl.tx,
                        ctrl.outer_machine_protocol,
                        ctrl.protocol,
                        false,
                    );
                }
                (orphaned, stop_pid, stop_unit, true)
            }
        };
        for (sid, cid) in orphaned {
            self.command_rejected(&sid, cid, "crashed worker drain timed out".into());
        }
        if let Some(pid) = stop_pid {
            self.arm_direct_worker_stop_pid(session_id, pid);
        }
        if let Some(unit) = stop_unit {
            let broker = Arc::clone(self);
            let sid = session_id.to_owned();
            let ep = epoch.to_owned();
            tokio::spawn(async move {
                tokio::time::sleep(DIRECT_EXIT_GRACE).await;
                if !broker
                    .workers
                    .lock()
                    .get(&sid)
                    .is_some_and(|w| w.connection_id == connection_id && w.epoch == ep)
                {
                    return;
                }
                let stop = Command::new("systemctl")
                    .args(["--user", "stop", &unit])
                    .status();
                let _ = tokio::time::timeout(Duration::from_secs(10), stop).await;
            });
        }
        if released_fence {
            self.crashed_drains.settled.notify_waiters();
        }
    }
}
