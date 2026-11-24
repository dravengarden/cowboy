//! Broker-side Antigravity wrap-up steering helpers: per-worker reliable-first
//! writer lane, handshake capability sanitization (outer Machine >= 53 AND
//! both runtime legs >= 8), and stateless hint forwarding outside
//! `pending_commands` / `CommandAck`.

use std::os::fd::AsRawFd;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::runtime_wire::{DeliveryTarget, Frame, WorkerCommand, WorkerSnapshot, write_frame};
use crate::wrap_up::{
    clamp_delivery_capabilities_for_provider, effective_wrap_up_steering,
    wrap_up_hint_within_size_limit,
};

/// Bounded per-worker hint queue capacity from broker to worker UnixStream.
pub(super) const WORKER_HINT_CAPACITY: usize = 8;

/// Run a worker connection's UnixStream writer selecting reliable commands
/// strictly ahead (`biased;`) of bounded drop-on-full hints, always writing
/// complete frames with zero mid-frame cancellation.
pub(crate) const MIN_HINT_HEADROOM_BYTES: usize = 32_768;

#[allow(unsafe_code)]
pub(crate) fn unix_socket_has_hint_headroom(
    writer: &tokio::net::unix::OwnedWriteHalf,
    frame_bytes: usize,
) -> bool {
    let fd = writer.as_ref().as_raw_fd();
    let mut sndbuf: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            std::ptr::from_mut(&mut sndbuf).cast(),
            &mut len,
        )
    };
    if rc != 0 || sndbuf <= 0 {
        return false;
    }
    let outq: usize = {
        #[cfg(target_os = "linux")]
        {
            let mut queued: libc::c_int = 0;
            let ioctl_rc = unsafe { libc::ioctl(fd, libc::TIOCOUTQ, &mut queued) };
            if ioctl_rc != 0 || queued < 0 {
                return false;
            }
            usize::try_from(queued).unwrap_or(usize::MAX)
        }
        #[cfg(target_os = "macos")]
        {
            let mut queued: libc::c_int = 0;
            let mut qlen = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
            let qrc = unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_NWRITE,
                    std::ptr::from_mut(&mut queued).cast(),
                    &mut qlen,
                )
            };
            if qrc != 0 || queued < 0 {
                return false;
            }
            usize::try_from(queued).unwrap_or(usize::MAX)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            0
        }
    };
    let effective_capacity = usize::try_from(sndbuf).unwrap_or(0) / 2;
    effective_capacity
        >= outq
            .saturating_add(frame_bytes)
            .saturating_add(MIN_HINT_HEADROOM_BYTES)
}

pub(crate) fn encoded_frame_bytes(frame: &Frame) -> Option<usize> {
    serde_json::to_vec(frame)
        .ok()
        .map(|bytes| bytes.len().saturating_add(4))
}

pub(super) async fn run_worker_unix_writer(
    mut writer: tokio::net::unix::OwnedWriteHalf,
    reliable_rx: mpsc::UnboundedReceiver<Frame>,
    hint_rx: mpsc::Receiver<Frame>,
) {
    run_worker_writer_with_headroom(
        &mut writer,
        reliable_rx,
        hint_rx,
        unix_socket_has_hint_headroom,
    )
    .await;
}

pub(super) async fn run_worker_writer_with_headroom<W, F>(
    writer: &mut W,
    mut reliable_rx: mpsc::UnboundedReceiver<Frame>,
    mut hint_rx: mpsc::Receiver<Frame>,
    mut has_headroom: F,
) where
    W: tokio::io::AsyncWrite + Unpin,
    F: FnMut(&W, usize) -> bool,
{
    let mut hints_open = true;
    loop {
        let (frame, is_hint) = if hints_open {
            tokio::select! {
                biased;
                frame = reliable_rx.recv() => match frame {
                    Some(frame) => (frame, false),
                    None => break,
                },
                hint = hint_rx.recv() => match hint {
                    Some(hint) => (hint, true),
                    None => {
                        hints_open = false;
                        continue;
                    }
                },
            }
        } else {
            match reliable_rx.recv().await {
                Some(frame) => (frame, false),
                None => break,
            }
        };
        if is_hint {
            let Some(frame_bytes) = encoded_frame_bytes(&frame) else {
                continue;
            };
            if !has_headroom(writer, frame_bytes) {
                continue;
            }
        }
        if write_frame(writer, &frame).await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
pub(super) async fn run_worker_writer<W>(
    mut writer: W,
    reliable_rx: mpsc::UnboundedReceiver<Frame>,
    hint_rx: mpsc::Receiver<Frame>,
) where
    W: tokio::io::AsyncWrite + Unpin,
{
    run_worker_writer_with_headroom(&mut writer, reliable_rx, hint_rx, |_, _| true).await;
}

pub(super) fn spawn_core_command_loop(
    broker: Arc<super::Broker>,
) -> (
    mpsc::Sender<(crate::runtime_wire::CoreCommand, PendingCoreCommandGuard)>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, mut rx) = mpsc::channel(super::CORE_COMMAND_QUEUE_CAPACITY);
    let task = tokio::spawn(async move {
        while let Some((command, guard)) = rx.recv().await {
            let broker = Arc::clone(&broker);
            let _ = tokio::spawn(async move {
                let _guard = guard;
                super::handle_core_command(&broker, command).await
            })
            .await;
        }
    });
    (tx, task)
}

pub(super) fn enqueue_core_command(
    broker: &Arc<super::Broker>,
    command_tx: &mpsc::Sender<(crate::runtime_wire::CoreCommand, PendingCoreCommandGuard)>,
    command: crate::runtime_wire::CoreCommand,
) -> anyhow::Result<()> {
    let guard = broker.enter_pending_core_command(command.session_id().map(str::to_owned));
    match command_tx.try_send((command, guard)) {
        Ok(()) => Ok(()),
        Err(mpsc::error::TrySendError::Full((command, _guard))) => {
            let Some((session_id, command_id)) = super::core_overflow::overflow_refusal(&command)
            else {
                anyhow::bail!("Machine core command queue saturated; reconnecting controller");
            };
            tracing::warn!(
                session = %session_id,
                %command_id,
                "Machine core command queue full; refusing command"
            );
            broker.command_rejected(
                &session_id,
                command_id,
                "Machine is busy; retry shortly".to_owned(),
            );
            Ok(())
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            anyhow::bail!("Machine core command queue closed");
        }
    }
}

pub(super) struct PendingCoreCommandGuard {
    broker: Arc<super::Broker>,
    session_id: Option<String>,
}

impl Drop for PendingCoreCommandGuard {
    fn drop(&mut self) {
        if let Some(session_id) = self.session_id.take() {
            let mut pending = self.broker.pending_core_commands.lock();
            if let Some(count) = pending.get_mut(&session_id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    pending.remove(&session_id);
                }
            }
        }
    }
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
    clamp_delivery_capabilities_for_provider(
        provider,
        &mut snapshot.supports_current_turn,
        &mut snapshot.delivery_controls.next_checkpoint,
    );
    let min_runtime_protocol = worker_protocol.min(controller_runtime_protocol);
    snapshot.wrap_up_steering = effective_wrap_up_steering(
        provider,
        raw_wrap_up,
        controller_outer_protocol,
        min_runtime_protocol,
        true,
    );
}

/// Sanitize a live `CURRENT_TURN_CAPABILITIES_UPDATE` event before forwarding
/// to the currently attached Controller.
pub(super) fn sanitize_capability_event(
    update: &mut serde_json::Value,
    raw_wrap_up: bool,
    worker_protocol: u16,
    controller_outer_protocol: Option<u16>,
    controller_runtime_protocol: u16,
    declared_provider: Option<&str>,
) {
    let eff = effective_wrap_up_steering(
        declared_provider,
        raw_wrap_up,
        controller_outer_protocol,
        worker_protocol.min(controller_runtime_protocol),
        true,
    );
    if let Some(obj) = update.as_object_mut() {
        if declared_provider.is_none_or(crate::wrap_up::is_antigravity_provider) {
            obj.insert("currentTurn".into(), serde_json::Value::Bool(false));
            obj.insert("nextCheckpoint".into(), serde_json::Value::Bool(false));
        }
        obj.insert("wrapUpSteering".into(), serde_json::Value::Bool(eff));
    }
}

/// Build and enqueue a bounded `WorkerCommand::WrapUpHint` onto a worker's
/// dedicated hint channel when all protocol, provider, target, and dedup
/// invariants hold. Returns `true` iff a new frame was enqueued.
#[allow(clippy::too_many_arguments)]
pub(super) fn try_forward_wrap_up_hint(
    session_id: &str,
    target: &DeliveryTarget,
    worker_epoch: &str,
    worker_snapshot: &WorkerSnapshot,
    raw_wrap_up: bool,
    worker_protocol: u16,
    controller_outer_protocol: Option<u16>,
    controller_runtime_protocol: u16,
    declared_provider: Option<&str>,
    last_hinted: &mut Option<DeliveryTarget>,
    hint_tx: &mpsc::Sender<Frame>,
) -> bool {
    let provider = worker_snapshot
        .launch
        .as_ref()
        .map(|launch| launch.provider.as_str())
        .or(declared_provider);
    if !effective_wrap_up_steering(
        provider,
        raw_wrap_up,
        controller_outer_protocol,
        worker_protocol.min(controller_runtime_protocol),
        true,
    ) {
        return false;
    }
    if worker_epoch != target.worker_epoch
        || worker_snapshot.current_turn_id.as_deref() != Some(target.turn_id.as_str())
        || worker_snapshot.drain_requested
    {
        return false;
    }
    if last_hinted.as_ref() == Some(target) {
        return false;
    }
    let frame = Frame::WorkerCommand {
        session_id: session_id.to_owned(),
        command: WorkerCommand::WrapUpHint {
            session_id: session_id.to_owned(),
            target: target.clone(),
        },
    };
    if !wrap_up_hint_within_size_limit(&frame) {
        return false;
    }
    if hint_tx.try_send(frame).is_ok() {
        *last_hinted = Some(target.clone());
        true
    } else {
        false
    }
}

impl super::Broker {
    #[cfg(test)]
    pub(super) fn attach_controller(
        &self,
        protocol: u16,
        tx: &mpsc::UnboundedSender<Frame>,
    ) -> u64 {
        self.attach_controller_with_outer(
            protocol,
            Some(crate::machine_protocol::MACHINE_PROTOCOL_VERSION),
            tx,
        )
    }

    pub(super) fn sanitized_snapshots(
        &self,
        outer: Option<u16>,
        ctrl_proto: u16,
    ) -> Vec<WorkerSnapshot> {
        let mut welcome = self.snapshots();
        for snapshot in &mut welcome {
            self.sanitize_snapshot_for_controller_protocols(snapshot, outer, ctrl_proto);
        }
        welcome
    }

    pub(super) fn attach_worker_wrap_up(
        &self,
        session_id: &str,
        connection_id: u64,
        protocol: u16,
        hint_tx: mpsc::Sender<Frame>,
    ) {
        if let Some(worker) = self.workers.lock().get_mut(session_id)
            && worker.connection_id == connection_id
        {
            worker.negotiated_protocol = protocol;
            worker.hint_tx = Some(hint_tx);
        }
    }

    pub(super) fn sanitize_snapshot_for_controller_protocols(
        &self,
        worker: &mut WorkerSnapshot,
        outer: Option<u16>,
        ctrl_proto: u16,
    ) {
        let (raw, worker_proto) = self
            .workers
            .lock()
            .get(&worker.session_id)
            .map_or((false, 0), |p| {
                (p.raw_wrap_up_steering, p.negotiated_protocol)
            });
        let declared = self
            .sessions
            .lock()
            .get(&worker.session_id)
            .map(|s| s.provider.clone());
        sanitize_snapshot_for_controller(
            worker,
            raw,
            worker_proto,
            outer,
            ctrl_proto,
            declared.as_deref(),
        );
    }

    pub(super) fn enter_pending_core_command(
        self: &Arc<Self>,
        session_id: Option<String>,
    ) -> PendingCoreCommandGuard {
        if let Some(ref sid) = session_id {
            *self
                .pending_core_commands
                .lock()
                .entry(sid.clone())
                .or_insert(0) += 1;
        }
        PendingCoreCommandGuard {
            broker: Arc::clone(self),
            session_id,
        }
    }

    pub(super) fn has_pending_core_command(&self, session_id: &str) -> bool {
        self.pending_core_commands
            .lock()
            .get(session_id)
            .copied()
            .unwrap_or(0)
            > 0
    }

    pub(super) fn handle_wrap_up_hint(
        &self,
        lease: u64,
        session_id: &str,
        target: &DeliveryTarget,
    ) {
        if self.has_pending_core_command(session_id)
            || self.cancelled_sessions.lock().contains(session_id)
            || self.resetting_sessions.lock().contains_key(session_id)
            || self.account_quiesced_sessions.lock().contains(session_id)
        {
            return;
        }
        let Some((outer, ctrl_proto)) = self
            .controller
            .lock()
            .as_ref()
            .filter(|c| c.lease == lease)
            .map(|c| (c.outer_machine_protocol, c.protocol))
        else {
            return;
        };
        let declared = self
            .sessions
            .lock()
            .get(session_id)
            .map(|s| s.provider.clone());
        if let Some(worker) = self.workers.lock().get_mut(session_id)
            && let Some(hint_tx) = worker.hint_tx.as_ref()
        {
            try_forward_wrap_up_hint(
                session_id,
                target,
                &worker.epoch,
                &worker.snapshot,
                worker.raw_wrap_up_steering,
                worker.negotiated_protocol,
                outer,
                ctrl_proto,
                declared.as_deref(),
                &mut worker.last_hinted_target,
                hint_tx,
            );
        }
    }
}

/// Narrowly scoped target-cleanup authority retained for a departing worker
/// epoch during `reset_session` so `process/terminate`, `process/read`,
/// `Observe`, `Events`, and `UNCERTAIN_EFFECT_UPDATE` can settle before stop.
#[derive(Clone)]
pub(super) struct DepartingCleanup {
    pub(super) epoch: String,
    pub(super) binding: crate::execution_environment::BindingV1,
}

pub(super) struct DepartingCleanupGuard {
    map: Arc<parking_lot::Mutex<std::collections::HashMap<String, DepartingCleanup>>>,
    session_id: String,
    epoch: String,
}

impl Drop for DepartingCleanupGuard {
    fn drop(&mut self) {
        let mut map = self.map.lock();
        if map
            .get(&self.session_id)
            .is_some_and(|entry| entry.epoch == self.epoch)
        {
            map.remove(&self.session_id);
        }
    }
}

pub(super) fn capture_departing_cleanup(
    broker: &super::Broker,
    session_id: &str,
) -> Option<DepartingCleanupGuard> {
    let (epoch, worker_binding) = {
        let workers = broker.workers.lock();
        let worker = workers.get(session_id)?;
        let binding = worker
            .snapshot
            .launch
            .as_ref()
            .and_then(|launch| launch.execution_binding.as_ref())
            .and_then(|binding| binding.decode().ok());
        (worker.epoch.clone(), binding)
    };
    let binding = worker_binding.or_else(|| {
        broker
            .sessions
            .lock()
            .get(session_id)
            .and_then(|launch| launch.execution_binding.as_ref())
            .and_then(|binding| binding.decode().ok())
    })?;
    broker.departing_cleanup.lock().insert(
        session_id.to_owned(),
        DepartingCleanup {
            epoch: epoch.clone(),
            binding,
        },
    );
    Some(DepartingCleanupGuard {
        map: Arc::clone(&broker.departing_cleanup),
        session_id: session_id.to_owned(),
        epoch,
    })
}

pub(super) fn is_departing_cleanup_command(command: &crate::execution_protocol::Command) -> bool {
    match command {
        crate::execution_protocol::Command::Invoke { invocation, .. } => {
            matches!(
                invocation.method.as_str(),
                "process/terminate" | "process/read"
            )
        }
        crate::execution_protocol::Command::Observe { .. }
        | crate::execution_protocol::Command::Events { .. } => true,
        crate::execution_protocol::Command::Describe
        | crate::execution_protocol::Command::Shutdown => false,
    }
}

pub(super) fn departing_cleanup_allows(
    broker: &super::Broker,
    session_id: &str,
    epoch: &str,
    request: &crate::execution_protocol::RuntimeRequest,
) -> bool {
    is_departing_cleanup_command(&request.command)
        && broker
            .departing_cleanup
            .lock()
            .get(session_id)
            .is_some_and(|entry| entry.epoch == epoch && entry.binding == request.binding)
}

pub(super) fn departing_uncertainty_event(
    broker: &super::Broker,
    session_id: &str,
    worker_epoch: &str,
    event: &crate::runtime_wire::RuntimeEvent,
) -> bool {
    matches!(
        event,
        crate::runtime_wire::RuntimeEvent::Update { update, .. }
            if crate::runtime_wire::uncertain_effect(update).is_some()
    ) && broker
        .departing_cleanup
        .lock()
        .get(session_id)
        .is_some_and(|entry| entry.epoch == worker_epoch)
}

/// Reject a `WorkerCommand::Prompt` before sending it to `worker` when its
/// bound `expected_worker_epoch` no longer matches or when the worker reports
/// an `uncertain_effect` newer than the prompt's `acknowledged_uncertain_sequence`.
pub(super) fn prompt_admission_refusal(
    worker_epoch: &str,
    worker_uncertain: Option<&crate::runtime_wire::UncertainEffect>,
    command: &WorkerCommand,
) -> Option<(String, String)> {
    let WorkerCommand::Prompt {
        command_id,
        expected_worker_epoch,
        acknowledged_uncertain_sequence,
        ..
    } = command
    else {
        return None;
    };
    if let Some(expected) = expected_worker_epoch
        && expected != worker_epoch
    {
        return Some((
            command_id.clone(),
            "worker epoch changed before prompt admission".to_owned(),
        ));
    }
    if let Some(effect) = worker_uncertain {
        let required = effect.sequence.max(1);
        let acknowledged = acknowledged_uncertain_sequence.unwrap_or(0);
        if required > acknowledged {
            return Some((
                command_id.clone(),
                format!(
                    "Target {} outcome is uncertain (sequence {required}); review and acknowledge before continuing",
                    effect.method
                ),
            ));
        }
    }
    None
}

impl super::Broker {
    pub(super) fn route_prompt_guarded(&self, session_id: &str, command: WorkerCommand) {
        if let WorkerCommand::Prompt {
            command_id,
            trace: Some(trace),
            ..
        } = &command
        {
            self.trace_spans.lock().start(session_id, command_id, trace);
        }
        let worker_gate = self.workers.lock().get(session_id).map(|worker| {
            (
                worker.snapshot.drain_requested,
                prompt_admission_refusal(
                    &worker.epoch,
                    worker.snapshot.uncertain_effect.as_ref(),
                    &command,
                ),
            )
        });
        if let Some((false, Some((command_id, reason)))) = worker_gate {
            self.send_controller(Frame::CommandAck {
                session_id: session_id.to_owned(),
                command_id,
                accepted: false,
                reason: Some(reason),
            });
            return;
        }
        if worker_gate.is_some_and(|(draining, _)| draining) {
            self.queue_pending(session_id, command);
        } else {
            self.route_worker(session_id, command);
        }
    }

    pub(super) fn flush_pending_guarded(&self, session_id: &str) {
        let worker = self.workers.lock().get(session_id).map(|worker| {
            (
                worker.tx.clone(),
                worker.epoch.clone(),
                worker.snapshot.drain_requested,
                worker.snapshot.uncertain_effect.clone(),
            )
        });
        let Some((tx, epoch, draining, uncertain)) = worker else {
            return;
        };
        let commands = self.pending_commands.lock().remove(session_id);
        if let Some(mut commands) = commands {
            let mut held_prompts = std::collections::VecDeque::new();
            while let Some(command) = commands.pop_front() {
                if draining && matches!(command, WorkerCommand::Prompt { .. }) {
                    held_prompts.push_back(command);
                    continue;
                }
                if let Some((command_id, reason)) =
                    prompt_admission_refusal(&epoch, uncertain.as_ref(), &command)
                {
                    self.send_controller(Frame::CommandAck {
                        session_id: session_id.to_owned(),
                        command_id,
                        accepted: false,
                        reason: Some(reason),
                    });
                    continue;
                }
                let _ = tx.send(Frame::WorkerCommand {
                    session_id: session_id.to_owned(),
                    command: self.trace_worker_dispatch(session_id, command),
                });
            }
            if !held_prompts.is_empty() {
                self.pending_commands
                    .lock()
                    .insert(session_id.to_owned(), held_prompts);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_wire::{DeliveryControls, WorkerState, read_frame};

    fn sample_snapshot(provider: Option<&str>) -> WorkerSnapshot {
        WorkerSnapshot {
            session_id: "s1".into(),
            worker_epoch: "e1".into(),
            generation: "g1".into(),
            executable: None,
            launch: provider.map(|p| crate::runtime_wire::StartSession {
                session_id: "s1".into(),
                provider: p.into(),
                provider_version: "0.2.35".into(),
                provider_generation_digest: "sha256:abc".into(),
                provider_account_id: Default::default(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: "/tmp".into(),
                generation: "g1".into(),
                agent_session_id: None,
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                fallback_for: None,
                adopt_only: false,
                execution_binding: None,
                extensions: None,
                mcp_plan: None,
                native_fork: None,
            }),
            state: WorkerState::Busy,
            agent_session_id: None,
            native_thread_materialized: None,
            current_turn_id: Some("t1".into()),
            supports_current_turn: true,
            delivery_controls: DeliveryControls {
                version: 1,
                next_checkpoint: true,
            },
            wrap_up_steering: true,
            supports_subagent_cancel: false,
            last_runtime_seq: 1,
            pending_permissions: vec![],
            config_options: None,
            context_used: None,
            context_size: None,
            pending_prompt_count: 0,
            drain_requested: false,
            exit_detail: None,
            background_tasks: None,
            native_cycle_id: None,
            incarnation: None,
            account_unit: None,
            uncertain_effect: None,
        }
    }

    #[test]
    fn sanitize_fails_closed_on_missing_provider_or_legacy_protocol() {
        let mut s = sample_snapshot(Some("antigravity"));
        sanitize_snapshot_for_controller(&mut s, true, 8, Some(53), 8, None);
        assert!(s.wrap_up_steering);
        assert!(!s.supports_current_turn);
        assert!(!s.delivery_controls.next_checkpoint);

        // Machine 52 + Runtime 8 -> false
        let mut s52 = sample_snapshot(Some("antigravity"));
        sanitize_snapshot_for_controller(&mut s52, true, 8, Some(52), 8, None);
        assert!(!s52.wrap_up_steering);

        // Machine 53 + Runtime 7 -> false
        let mut s7 = sample_snapshot(Some("antigravity"));
        sanitize_snapshot_for_controller(&mut s7, true, 7, Some(53), 8, None);
        assert!(!s7.wrap_up_steering);

        // Missing provider -> fails closed across ALL THREE delivery flags.
        let mut missing = sample_snapshot(None);
        sanitize_snapshot_for_controller(&mut missing, true, 8, Some(53), 8, None);
        assert!(!missing.wrap_up_steering);
        assert!(!missing.supports_current_turn);
        assert!(!missing.delivery_controls.next_checkpoint);
        let mut ev = serde_json::json!({
            "currentTurn": true,
            "nextCheckpoint": true,
            "wrapUpSteering": true,
        });
        sanitize_capability_event(&mut ev, true, 8, Some(53), 8, None);
        assert_eq!(ev["currentTurn"], serde_json::Value::Bool(false));
        assert_eq!(ev["nextCheckpoint"], serde_json::Value::Bool(false));
        assert_eq!(ev["wrapUpSteering"], serde_json::Value::Bool(false));
    }

    #[tokio::test]
    async fn worker_writer_delivers_stop_intact_after_hint_with_zero_stream_corruption() {
        let (reliable_tx, reliable_rx) = mpsc::unbounded_channel();
        let (hint_tx, hint_rx) = mpsc::channel(WORKER_HINT_CAPACITY);
        let target = DeliveryTarget {
            worker_epoch: "e1".into(),
            turn_id: "t1".into(),
        };
        let mut last = None;
        let snap = sample_snapshot(Some("antigravity"));
        assert!(try_forward_wrap_up_hint(
            "s1",
            &target,
            "e1",
            &snap,
            true,
            8,
            Some(53),
            8,
            None,
            &mut last,
            &hint_tx
        ));
        // Duplicate hint for same turn is deduplicated.
        assert!(!try_forward_wrap_up_hint(
            "s1",
            &target,
            "e1",
            &snap,
            true,
            8,
            Some(53),
            8,
            None,
            &mut last,
            &hint_tx
        ));
        reliable_tx
            .send(Frame::WorkerCommand {
                session_id: "s1".into(),
                command: WorkerCommand::Cancel {
                    command_id: "stop-1".into(),
                },
            })
            .unwrap();
        // Small duplex buffer forces partial progress / backpressure while reader pauses.
        let (client, mut server) = tokio::io::duplex(64);
        let writer = tokio::spawn(run_worker_writer(client, reliable_rx, hint_rx));
        let first = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            first,
            Frame::WorkerCommand {
                command: WorkerCommand::Cancel { .. },
                ..
            }
        ));
        let second = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            second,
            Frame::WorkerCommand {
                command: WorkerCommand::WrapUpHint { .. },
                ..
            }
        ));
        drop(reliable_tx);
        drop(hint_tx);
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn surviving_busy_r1_worker_reconnects_against_disabled_broker_and_settles_reliably() {
        let mut snap = sample_snapshot(Some("antigravity"));
        sanitize_snapshot_for_controller(&mut snap, true, 8, Some(52), 8, None);
        assert!(!snap.wrap_up_steering);
        assert!(!snap.supports_current_turn);

        let (reliable_tx, reliable_rx) = mpsc::unbounded_channel();
        let (hint_tx, hint_rx) = mpsc::channel(WORKER_HINT_CAPACITY);
        let target = DeliveryTarget {
            worker_epoch: "e1".into(),
            turn_id: "t1".into(),
        };
        let mut last = None;
        assert!(!try_forward_wrap_up_hint(
            "s1",
            &target,
            "e1",
            &snap,
            true,
            8,
            Some(52),
            8,
            None,
            &mut last,
            &hint_tx
        ));
        reliable_tx
            .send(Frame::WorkerCommand {
                session_id: "s1".into(),
                command: WorkerCommand::Permission {
                    command_id: "perm-1".into(),
                    request_id: "req-1".into(),
                    option_id: Some("allow_once".into()),
                },
            })
            .unwrap();
        reliable_tx
            .send(Frame::WorkerCommand {
                session_id: "s1".into(),
                command: WorkerCommand::Cancel {
                    command_id: "cancel-1".into(),
                },
            })
            .unwrap();
        let (client, mut server) = tokio::io::duplex(256);
        let writer = tokio::spawn(run_worker_writer(client, reliable_rx, hint_rx));
        let first = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            first,
            Frame::WorkerCommand {
                command: WorkerCommand::Permission { .. },
                ..
            }
        ));
        let second = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            second,
            Frame::WorkerCommand {
                command: WorkerCommand::Cancel { .. },
                ..
            }
        ));
        drop(reliable_tx);
        drop(hint_tx);
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn worker_writer_drops_hint_before_byte_zero_when_headroom_insufficient() {
        let (reliable_tx, reliable_rx) = mpsc::unbounded_channel();
        let (hint_tx, hint_rx) = mpsc::channel(WORKER_HINT_CAPACITY);
        hint_tx
            .try_send(Frame::WorkerCommand {
                session_id: "s1".into(),
                command: WorkerCommand::WrapUpHint {
                    session_id: "s1".into(),
                    target: DeliveryTarget {
                        worker_epoch: "e1".into(),
                        turn_id: "t1".into(),
                    },
                },
            })
            .unwrap();
        let (mut client, mut server) = tokio::io::duplex(256);
        let writer = tokio::spawn(async move {
            run_worker_writer_with_headroom(&mut client, reliable_rx, hint_rx, |_, _| false).await;
        });
        tokio::task::yield_now().await;
        reliable_tx
            .send(Frame::WorkerCommand {
                session_id: "s1".into(),
                command: WorkerCommand::Cancel {
                    command_id: "stop-after-dropped-hint".into(),
                },
            })
            .unwrap();
        let first = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            first,
            Frame::WorkerCommand {
                command: WorkerCommand::Cancel { command_id },
                ..
            } if command_id == "stop-after-dropped-hint"
        ));
        drop(reliable_tx);
        drop(hint_tx);
        writer.await.unwrap();
    }

    struct PartialHintWriter<W> {
        inner: W,
        split_once_at: Option<usize>,
        on_split: Option<tokio::sync::oneshot::Sender<()>>,
        resume_rx: Option<tokio::sync::oneshot::Receiver<()>>,
    }

    impl<W: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for PartialHintWriter<W> {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            if let Some(limit) = self.split_once_at.take() {
                let n = limit.min(buf.len());
                let res = std::pin::Pin::new(&mut self.inner).poll_write(cx, &buf[..n]);
                if let std::task::Poll::Ready(Ok(written)) = res {
                    if let Some(tx) = self.on_split.take() {
                        let _ = tx.send(());
                    }
                    return std::task::Poll::Ready(Ok(written));
                }
                self.split_once_at = Some(limit);
                return res;
            }
            if let Some(rx) = self.resume_rx.as_mut() {
                match std::pin::Pin::new(rx).poll(cx) {
                    std::task::Poll::Pending => return std::task::Poll::Pending,
                    std::task::Poll::Ready(_) => {
                        self.resume_rx = None;
                    }
                }
            }
            std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
        }

        fn poll_flush(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.inner).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
        }
    }

    #[tokio::test]
    async fn worker_writer_finishes_partial_hint_then_delivers_cancel_intact() {
        let (reliable_tx, reliable_rx) = mpsc::unbounded_channel();
        let (hint_tx, hint_rx) = mpsc::channel(WORKER_HINT_CAPACITY);
        let (split_tx, split_rx) = tokio::sync::oneshot::channel();
        let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
        let (client, mut server) = tokio::io::duplex(512);
        let mut partial = PartialHintWriter {
            inner: client,
            split_once_at: Some(7),
            on_split: Some(split_tx),
            resume_rx: Some(resume_rx),
        };
        let writer = tokio::spawn(async move {
            run_worker_writer_with_headroom(&mut partial, reliable_rx, hint_rx, |_, _| true).await;
        });
        hint_tx
            .try_send(Frame::WorkerCommand {
                session_id: "s1".into(),
                command: WorkerCommand::WrapUpHint {
                    session_id: "s1".into(),
                    target: DeliveryTarget {
                        worker_epoch: "e1".into(),
                        turn_id: "t1".into(),
                    },
                },
            })
            .unwrap();
        split_rx.await.unwrap();
        reliable_tx
            .send(Frame::WorkerCommand {
                session_id: "s1".into(),
                command: WorkerCommand::Cancel {
                    command_id: "stop-after-partial-hint".into(),
                },
            })
            .unwrap();
        let _ = resume_tx.send(());

        let first = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            first,
            Frame::WorkerCommand {
                command: WorkerCommand::WrapUpHint { .. },
                ..
            }
        ));
        let second = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            second,
            Frame::WorkerCommand {
                command: WorkerCommand::Cancel { command_id },
                ..
            } if command_id == "stop-after-partial-hint"
        ));
        drop(reliable_tx);
        drop(hint_tx);
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn pending_core_command_fences_inline_wrap_up_hint() {
        let dir = tempfile::tempdir().unwrap();
        let broker = Arc::new(super::super::Broker::new(super::super::MachineBrokerArgs {
            socket: dir.path().join("broker.sock"),
            worker_command: std::path::PathBuf::from("/bin/false"),
            desired_generation: "g1".into(),
            spawn_mode: super::super::SpawnMode::Direct,
            worker_environment: std::collections::BTreeMap::new(),
            provider_store: Arc::new(
                crate::machine_plugins::MachinePluginStore::new(
                    &dir.path().join("plugins"),
                    crate::machine_protocol::Platform::Linux,
                    "x86_64".into(),
                )
                .unwrap(),
            ),
            worktree_root: dir.path().join("worktrees").into(),
            worker_ready_timeout: std::time::Duration::from_secs(1),
            call_context_root: None,
            memory_relay: None,
        }));
        let (ctrl_tx, _ctrl_rx) = mpsc::unbounded_channel();
        let lease = broker.attach_controller_with_outer(8, Some(53), &ctrl_tx);
        let (worker_tx, _worker_rx) = mpsc::unbounded_channel();
        let (hint_tx, mut hint_rx) = mpsc::channel(WORKER_HINT_CAPACITY);
        let snap = sample_snapshot(Some("antigravity"));
        broker.workers.lock().insert(
            "s1".into(),
            super::super::WorkerPeer {
                connection_id: 1,
                epoch: "e1".into(),
                tx: worker_tx,
                snapshot: snap,
                acked_runtime_seq: 0,
                last_seen: std::time::Instant::now(),
                receive_probe: None,
                liveness: None,
                negotiated_protocol: 8,
                raw_wrap_up_steering: true,
                hint_tx: Some(hint_tx),
                last_hinted_target: None,
            },
        );
        let target = DeliveryTarget {
            worker_epoch: "e1".into(),
            turn_id: "t1".into(),
        };
        let guard = broker.enter_pending_core_command(Some("s1".into()));
        broker.handle_wrap_up_hint(lease, "s1", &target);
        assert!(hint_rx.try_recv().is_err());
        drop(guard);

        broker.handle_wrap_up_hint(lease, "s1", &target);
        assert!(matches!(
            hint_rx.try_recv().ok(),
            Some(Frame::WorkerCommand {
                command: WorkerCommand::WrapUpHint { .. },
                ..
            })
        ));
    }

    #[test]
    fn non_antigravity_provider_preserves_current_turn_and_never_advertises_wrap_up() {
        let mut codex = sample_snapshot(Some("codex"));
        sanitize_snapshot_for_controller(&mut codex, true, 8, Some(53), 8, None);
        assert!(!codex.wrap_up_steering);
        assert!(codex.supports_current_turn);
        assert!(codex.delivery_controls.next_checkpoint);
    }

    #[test]
    fn departing_cleanup_allows_only_terminate_read_observe_events_and_rejects_mutations() {
        let terminate = crate::execution_protocol::Command::Invoke {
            invocation: crate::execution_protocol::Invocation {
                operation_id: "op-1".into(),
                method: "process/terminate".into(),
                params: serde_json::json!({"processId": "p1"}),
            },
            wait_ms: 1000,
        };
        let read = crate::execution_protocol::Command::Invoke {
            invocation: crate::execution_protocol::Invocation {
                operation_id: "op-2".into(),
                method: "process/read".into(),
                params: serde_json::json!({"processId": "p1"}),
            },
            wait_ms: 1000,
        };
        let start = crate::execution_protocol::Command::Invoke {
            invocation: crate::execution_protocol::Invocation {
                operation_id: "op-3".into(),
                method: "process/start".into(),
                params: serde_json::json!({"processId": "p2"}),
            },
            wait_ms: 1000,
        };
        let write = crate::execution_protocol::Command::Invoke {
            invocation: crate::execution_protocol::Invocation {
                operation_id: "op-4".into(),
                method: "fs/writeFile".into(),
                params: serde_json::json!({"path": "a"}),
            },
            wait_ms: 1000,
        };
        assert!(is_departing_cleanup_command(&terminate));
        assert!(is_departing_cleanup_command(&read));
        assert!(is_departing_cleanup_command(
            &crate::execution_protocol::Command::Observe {
                operation_id: "op-1".into(),
                wait_ms: 1000,
            }
        ));
        assert!(is_departing_cleanup_command(
            &crate::execution_protocol::Command::Events {
                after: 0,
                wait_ms: 1000,
            }
        ));
        assert!(!is_departing_cleanup_command(&start));
        assert!(!is_departing_cleanup_command(&write));
    }

    #[test]
    fn broker_prompt_admission_refusal_blocks_stale_epoch_and_unacknowledged_uncertainty() {
        let prompt = WorkerCommand::Prompt {
            command_id: "cmd-1".into(),
            turn_id: "t-1".into(),
            content: vec![],
            cmid: None,
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: Some("e-old".into()),
            acknowledged_uncertain_sequence: Some(5),
        };
        assert!(prompt_admission_refusal("e-new", None, &prompt).is_some());
        let effect = crate::runtime_wire::UncertainEffect {
            method: "process/terminate".into(),
            sequence: 6,
        };
        assert!(prompt_admission_refusal("e-old", Some(&effect), &prompt).is_some());
        let acked = crate::runtime_wire::UncertainEffect {
            method: "process/terminate".into(),
            sequence: 5,
        };
        assert!(prompt_admission_refusal("e-old", Some(&acked), &prompt).is_none());
    }
}
