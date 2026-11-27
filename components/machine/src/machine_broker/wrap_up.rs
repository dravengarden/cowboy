//! Broker-side Antigravity wrap-up steering helpers: per-worker reliable-first
//! writer lane, handshake capability sanitization (outer Machine >= 53 AND
//! both runtime legs >= 8), and stateless hint forwarding outside
//! `pending_commands` / `CommandAck`.

use super::account_unit::evicted_command_digest;
use std::os::fd::AsRawFd;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::runtime_wire::{DeliveryTarget, Frame, WorkerCommand, WorkerSnapshot, write_frame};
use crate::wrap_up::{effective_wrap_up_steering, wrap_up_hint_within_size_limit};

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
    diag_reply_rx: mpsc::Receiver<Frame>,
) {
    run_worker_writer_with_headroom_and_diag(
        &mut writer,
        reliable_rx,
        hint_rx,
        Some(diag_reply_rx),
        unix_socket_has_hint_headroom,
    )
    .await;
}

#[cfg(test)]
pub(super) async fn run_worker_writer_with_headroom<W, F>(
    writer: &mut W,
    reliable_rx: mpsc::UnboundedReceiver<Frame>,
    hint_rx: mpsc::Receiver<Frame>,
    has_headroom: F,
) where
    W: tokio::io::AsyncWrite + Unpin,
    F: FnMut(&W, usize) -> bool,
{
    run_worker_writer_with_headroom_and_diag(writer, reliable_rx, hint_rx, None, has_headroom)
        .await;
}

pub(super) async fn run_worker_writer_with_headroom_and_diag<W, F>(
    writer: &mut W,
    mut reliable_rx: mpsc::UnboundedReceiver<Frame>,
    mut hint_rx: mpsc::Receiver<Frame>,
    mut diag_reply_rx: Option<mpsc::Receiver<Frame>>,
    mut has_headroom: F,
) where
    W: tokio::io::AsyncWrite + Unpin,
    F: FnMut(&W, usize) -> bool,
{
    let mut hints_open = true;
    loop {
        let (frame, best_effort) = tokio::select! {
            biased;
            frame = reliable_rx.recv() => match frame {
                Some(frame) => (frame, false),
                None => break,
            },
            hint = hint_rx.recv(), if hints_open => match hint {
                Some(hint) => (hint, true),
                None => {
                    hints_open = false;
                    continue;
                }
            },
            diag = async {
                match diag_reply_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if diag_reply_rx.is_some() => match diag {
                Some(diag) => (diag, true),
                None => {
                    diag_reply_rx = None;
                    continue;
                }
            },
        };
        if best_effort {
            let Some(frame_bytes) = encoded_frame_bytes(&frame) else {
                continue;
            };
            if matches!(frame, Frame::PostEditInspectReply { .. })
                && frame_bytes > crate::post_edit_inspect::MAX_POST_EDIT_REPLY_BYTES + 4
            {
                continue;
            }
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
    mpsc::Sender<(
        u64,
        crate::runtime_wire::CoreCommand,
        PendingCoreCommandGuard,
    )>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, mut rx) = mpsc::channel::<(
        u64,
        crate::runtime_wire::CoreCommand,
        PendingCoreCommandGuard,
    )>(super::CORE_COMMAND_QUEUE_CAPACITY);
    let task = tokio::spawn(async move {
        while let Some((lease, command, guard)) = rx.recv().await {
            if broker.controller_for(lease).is_none() {
                tracing::warn!(lease, "discarding queued command from replaced controller");
                drop(guard);
                continue;
            }
            let broker = Arc::clone(&broker);
            let _ = tokio::spawn(super::COMMAND_LEASE.scope(lease, async move {
                let _guard = guard;
                super::handle_core_command(&broker, command).await
            }))
            .await;
        }
    });
    (tx, task)
}

/// Atomically validate `lease`, normalize v8 prompt ingress, track prompt
/// admission, and enqueue onto `command_tx` under a single controller-lease
/// lock so a replaced Controller can never enqueue a stale command or let a
/// concurrent overflow falsely certify an already-admitted prompt as unhanded.
pub(super) fn enqueue_leased_core_command(
    broker: &Arc<super::Broker>,
    lease: u64,
    command_tx: &mpsc::Sender<(
        u64,
        crate::runtime_wire::CoreCommand,
        PendingCoreCommandGuard,
    )>,
    mut command: crate::runtime_wire::CoreCommand,
) -> anyhow::Result<()> {
    // Finding 10: Classify overflow refusal and bind to this exact lease's
    // sender while holding the controller lock so a replacement lease can never
    // receive an old lease's refusal.
    let overflow = {
        let controller = broker.controller.lock();
        let Some((ctrl_tx, ctrl_proto)) = controller
            .as_ref()
            .filter(|c| c.lease == lease)
            .map(|c| (c.tx.clone(), c.protocol))
        else {
            tracing::warn!(lease, "ignoring command from fenced controller");
            return Ok(());
        };
        normalize_ingress_prompt(ctrl_proto, &mut command);
        let prompt_key = match &command {
            crate::runtime_wire::CoreCommand::Prompt {
                session_id,
                command_id,
                ..
            } => Some((session_id.clone(), command_id.clone())),
            _ => None,
        };
        let mut guard = broker.enter_pending_core_command(command.session_id().map(str::to_owned));
        if let Some((sid, cid)) = prompt_key {
            let mut dispatched = broker.dispatched_prompts.lock();
            dispatched.inc_core_queue(&sid, &cid);
            dispatched.mark_ingressed(&sid, &cid);
            guard.core_prompt = Some((sid, cid));
        }
        match command_tx.try_send((lease, command, guard)) {
            Ok(()) => None,
            Err(mpsc::error::TrySendError::Full((_, command, guard))) => {
                drop(guard);
                let is_prompt = matches!(command, crate::runtime_wire::CoreCommand::Prompt { .. });
                let Some((session_id, command_id)) =
                    super::core_overflow::overflow_refusal(&command)
                else {
                    anyhow::bail!("Machine core command queue saturated; reconnecting controller");
                };
                let reason = if is_prompt
                    && broker
                        .dispatched_prompts
                        .lock()
                        .was_ingressed_or_uncertain(&session_id, &command_id)
                {
                    "delivery_ambiguous: Machine core queue full on retry after prompt ingress"
                        .to_owned()
                } else if is_prompt {
                    "unhanded: Machine is busy; retry shortly".to_owned()
                } else {
                    "Machine is busy; retry shortly".to_owned()
                };
                Some((ctrl_tx, session_id, command_id, reason))
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                anyhow::bail!("Machine core command queue closed");
            }
        }
    };
    if let Some((ctrl_tx, session_id, command_id, reason)) = overflow {
        tracing::warn!(
            session = %session_id,
            %command_id,
            "Machine core command queue full; refusing command"
        );
        let _ = ctrl_tx.send(Frame::CommandAck {
            session_id,
            command_id,
            accepted: false,
            reason: Some(reason),
        });
    }
    Ok(())
}

pub(super) use super::account_unit::PendingCoreCommandGuard;
pub(super) use super::account_unit::sanitize_snapshot_for_controller;

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
            None,
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
        diag_reply_tx: mpsc::Sender<Frame>,
    ) {
        if let Some(worker) = self.workers.lock().get_mut(session_id)
            && worker.connection_id == connection_id
        {
            worker.negotiated_protocol = protocol;
            worker.hint_tx = Some(hint_tx);
            worker.diag_reply_tx = Some(diag_reply_tx);
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
            core_prompt: None,
        }
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

#[derive(Clone)]
struct DispatchedPromptRecord {
    epoch: Option<String>,
    turn_id: Option<String>,
    acked: bool,
    seq: u64,
}

/// Record of `(session_id, command_id)` from ingress admission through worker
/// handoff. Each insertion carries a monotonic `seq` so retired/reinserted keys
/// never match stale `order` entries. Evicted commands stay indexed by exact
/// `(session_id, command_id_digest)` so replaying an evicted command across a
/// worker change is refused without ever blocking brand-new cold-wake or legacy
/// unbound prompts.
#[derive(Default)]
pub(super) struct DispatchedPrompts {
    entries: std::collections::HashMap<(String, String), DispatchedPromptRecord>,
    pub(super) queued_core_prompts: std::collections::HashMap<(String, String), usize>,
    order: std::collections::VecDeque<(String, String, u64)>,
    next_seq: u64,
    /// Evicted command disposition keyed by `(session_id, digest(command_id))`:
    /// `Some(epoch)` when acked with a known handed-off worker epoch, or `None`
    /// when evicted without an epoch (always ambiguous on replay). Entries are
    /// retained intact up to `MAX_EVICTED_COMMANDS` (cleared on session retire)
    /// and new fresh prompt admission fails closed when capacity is reached.
    evicted_commands: std::collections::HashMap<(String, [u8; 16]), Option<String>>,
}

impl DispatchedPrompts {
    const MAX_ENTRIES: usize = 16_384;
    pub(super) fn inc_core_queue(&mut self, sid: &str, cid: &str) {
        *self
            .queued_core_prompts
            .entry((sid.to_owned(), cid.to_owned()))
            .or_insert(0) += 1;
    }
    pub(super) fn dec_core_queue(&mut self, sid: &str, cid: &str) -> usize {
        let k = (sid.to_owned(), cid.to_owned());
        let left = self.queued_core_prompts.get_mut(&k).map_or(0, |c| {
            *c = c.saturating_sub(1);
            *c
        });
        if left == 0 {
            self.queued_core_prompts.remove(&k);
        }
        left
    }
    const MAX_EVICTED_COMMANDS: usize = 262_144;

    pub(super) fn has_unacked_for_epoch_or_ingress(
        &self,
        session_id: &str,
        worker_epoch: &str,
    ) -> bool {
        self.entries.iter().any(|((sid, _), rec)| {
            sid == session_id
                && !rec.acked
                && (rec.epoch.is_none() || rec.epoch.as_deref() == Some(worker_epoch))
        })
    }

    pub(super) fn get(&self, session_id: &str, command_id: &str) -> Option<&str> {
        self.entries
            .get(&(session_id.to_owned(), command_id.to_owned()))
            .and_then(|rec| rec.epoch.as_deref())
    }

    pub(super) fn was_command_evicted(&self, session_id: &str, command_id: &str) -> bool {
        let key = (session_id.to_owned(), evicted_command_digest(command_id));
        self.evicted_commands.contains_key(&key)
    }

    /// True only when THIS specific `(session_id, command_id)` was evicted from
    /// active tracking while unacked or bound to a different worker epoch, so
    /// replaying it unbound is ambiguous while brand-new cold-wake / legacy
    /// commands remain unaffected.
    pub(super) fn evicted_blocks_replay(
        &self,
        session_id: &str,
        command_id: &str,
        current_epoch: &str,
    ) -> bool {
        let key = (session_id.to_owned(), evicted_command_digest(command_id));
        match self.evicted_commands.get(&key) {
            None => false,
            Some(None) => true,
            Some(Some(prior_epoch)) => prior_epoch != current_epoch,
        }
    }

    pub(super) fn was_ingressed_or_uncertain(&self, session_id: &str, command_id: &str) -> bool {
        self.entries
            .contains_key(&(session_id.to_owned(), command_id.to_owned()))
            || self.was_command_evicted(session_id, command_id)
    }

    /// True when active or evicted replay tracking has reached its hard bound
    /// and cannot safely admit a brand-new fresh prompt without forgetting
    /// earlier replayable handoffs. Existing tracked/evicted commands still
    /// resolve deterministically.
    pub(super) fn at_capacity(&mut self) -> bool {
        self.evict_acked_if_needed();
        self.entries.len() >= Self::MAX_ENTRIES * 2
    }

    fn can_record_eviction(&self, session_id: &str, command_id: &str) -> bool {
        if self.evicted_commands.len() < Self::MAX_EVICTED_COMMANDS {
            return true;
        }
        let key = (session_id.to_owned(), evicted_command_digest(command_id));
        self.evicted_commands.contains_key(&key)
    }

    fn record_eviction(
        &mut self,
        session_id: String,
        command_id: &str,
        record: DispatchedPromptRecord,
    ) -> bool {
        let key = (session_id, evicted_command_digest(command_id));
        if self.evicted_commands.len() >= Self::MAX_EVICTED_COMMANDS
            && !self.evicted_commands.contains_key(&key)
        {
            return false;
        }
        let disposition = if record.acked { record.epoch } else { None };
        let existing = self.evicted_commands.get(&key);
        let merged = match (existing, disposition) {
            (Some(None), _) | (_, None) => None,
            (_, Some(epoch)) => Some(epoch),
        };
        self.evicted_commands.insert(key, merged);
        true
    }

    fn alloc_seq(&mut self) -> u64 {
        self.next_seq = self.next_seq.wrapping_add(1);
        self.next_seq
    }

    fn compact_order(&mut self) {
        self.order.retain(|(sid, cid, seq)| {
            self.entries
                .get(&(sid.clone(), cid.clone()))
                .is_some_and(|rec| rec.seq == *seq)
        });
    }

    /// Record a freshly ingressed prompt. Returns `true` only when a brand-new
    /// record was inserted (so dropping an unentered retry can never retire an
    /// earlier live ingress record).
    pub(super) fn mark_ingressed(&mut self, session_id: &str, command_id: &str) -> bool {
        let key = (session_id.to_owned(), command_id.to_owned());
        if !self.entries.contains_key(&key) {
            let seq = self.alloc_seq();
            self.entries.insert(
                key.clone(),
                DispatchedPromptRecord {
                    epoch: None,
                    turn_id: None,
                    acked: false,
                    seq,
                },
            );
            self.order.push_back((key.0, key.1, seq));
            self.evict_acked_if_needed();
            true
        } else {
            false
        }
    }

    /// Retire an entry provably dropped in the broker queue before worker handoff
    /// while keeping `order` generation-accurate and bounded (Findings 9 & 13).
    pub(super) fn retire_if_unhanded(&mut self, session_id: &str, command_id: &str) {
        let key = (session_id.to_owned(), command_id.to_owned());
        if let Some(seq) = self
            .entries
            .get(&key)
            .filter(|rec| rec.epoch.is_none() && !rec.acked)
            .map(|rec| rec.seq)
        {
            self.entries.remove(&key);
            if self
                .order
                .back()
                .is_some_and(|(s, c, g)| s == &key.0 && c == &key.1 && *g == seq)
            {
                self.order.pop_back();
            } else if self.order.len() > self.entries.len().saturating_mul(2).max(Self::MAX_ENTRIES)
            {
                self.compact_order();
            }
        }
    }

    pub(super) fn record(&mut self, session_id: &str, command_id: &str, epoch: &str) {
        let key = (session_id.to_owned(), command_id.to_owned());
        if let Some(rec) = self.entries.get_mut(&key) {
            rec.epoch = Some(epoch.to_owned());
            rec.acked = false;
            return;
        }
        let seq = self.alloc_seq();
        self.entries.insert(
            key.clone(),
            DispatchedPromptRecord {
                epoch: Some(epoch.to_owned()),
                turn_id: None,
                acked: false,
                seq,
            },
        );
        self.order.push_back((key.0, key.1, seq));
        self.evict_acked_if_needed();
    }

    pub(super) fn mark_acked(&mut self, session_id: &str, command_id: &str) {
        if let Some(rec) = self
            .entries
            .get_mut(&(session_id.to_owned(), command_id.to_owned()))
        {
            rec.acked = true;
        }
        self.evict_acked_if_needed();
    }

    pub(super) fn record_with_turn(
        &mut self,
        session_id: &str,
        command_id: &str,
        epoch: &str,
        turn_id: Option<&str>,
    ) {
        self.record(session_id, command_id, epoch);
        if let Some(tid) = turn_id
            && let Some(rec) = self
                .entries
                .get_mut(&(session_id.to_owned(), command_id.to_owned()))
        {
            rec.turn_id = Some(tid.to_owned());
        }
    }
    pub(super) fn mark_acked_by_turn(&mut self, session_id: &str, epoch: &str, turn_id: &str) {
        for ((sid, _), rec) in &mut self.entries {
            if sid == session_id
                && rec.epoch.as_deref() == Some(epoch)
                && rec.turn_id.as_deref() == Some(turn_id)
            {
                rec.acked = true;
            }
        }
        self.evict_acked_if_needed();
    }

    pub(super) fn retire_session(&mut self, session_id: &str) {
        self.entries.retain(|(sid, _), _| sid != session_id);
        self.order.retain(|(sid, _, _)| sid != session_id);
        self.evicted_commands
            .retain(|(sid, _), _| sid != session_id);
    }

    fn evict_acked_if_needed(&mut self) {
        let mut scanned = 0;
        let max_scan = self.order.len();
        while self.entries.len() > Self::MAX_ENTRIES && scanned < max_scan {
            let Some((sid, cid, seq)) = self.order.pop_front() else {
                break;
            };
            scanned += 1;
            let key = (sid, cid);
            match self.entries.get(&key) {
                Some(rec) if rec.seq != seq => {}
                Some(rec) if !rec.acked => {
                    self.order.push_back((key.0, key.1, seq));
                }
                Some(_) if self.can_record_eviction(&key.0, &key.1) => {
                    if let Some(rec) = self.entries.remove(&key) {
                        let _ = self.record_eviction(key.0, &key.1, rec);
                    }
                }
                Some(_) => {
                    // Evicted history is at capacity: keep entry intact in
                    // `entries` so bounded admission fails closed without
                    // dropping earlier replayable handoffs or growing past cap.
                    self.order.push_back((key.0, key.1, seq));
                    break;
                }
                None => {}
            }
        }
        let hard_cap = Self::MAX_ENTRIES * 2;
        if self.order.len() > hard_cap {
            self.compact_order();
        }
    }
}

/// Normalize missing `acknowledged_uncertain_sequence` at Controller ingress so
/// queued prompts retain the admitting Controller's protocol contract even
/// across later Controller lease changes (`Some(0)` for v8+, `None` for legacy).
pub(super) fn normalize_ingress_prompt(
    controller_protocol: u16,
    command: &mut crate::runtime_wire::CoreCommand,
) {
    if controller_protocol >= crate::runtime_wire::WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION
        && let crate::runtime_wire::CoreCommand::Prompt {
            acknowledged_uncertain_sequence,
            ..
        } = command
        && acknowledged_uncertain_sequence.is_none()
    {
        *acknowledged_uncertain_sequence = Some(0);
    }
}

#[allow(clippy::too_many_arguments)]
fn fresh_prompt_refusal(
    dispatched: &DispatchedPrompts,
    session_id: &str,
    command_id: &str,
    expected_epoch: Option<&str>,
    worker_epoch: &str,
    worker_uncertain: Option<&crate::runtime_wire::UncertainEffect>,
    command: &WorkerCommand,
    refusal_reason: &impl Fn(String) -> String,
) -> Option<(String, String)> {
    if expected_epoch.is_none()
        && dispatched.evicted_blocks_replay(session_id, command_id, worker_epoch)
    {
        return Some((
            command_id.to_owned(),
            "delivery_ambiguous: broker evicted prior prompt handoff state across worker change; refusing unbound replay"
                .to_owned(),
        ));
    }
    prompt_admission_refusal(worker_epoch, worker_uncertain, command)
        .map(|(cid, reason)| (cid, refusal_reason(reason)))
}

/// Reject an unhanded `WorkerCommand::Prompt` before dispatching it to `worker`
/// when its bound `expected_worker_epoch` no longer matches or when the worker
/// reports an `uncertain_effect` newer than its ingress-normalized
/// `acknowledged_uncertain_sequence`.
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
            "unhanded: worker epoch changed before prompt admission".to_owned(),
        ));
    }
    if let Some(effect) = worker_uncertain
        && let Some(acknowledged) = acknowledged_uncertain_sequence
    {
        let required = effect.sequence.max(1);
        if required > *acknowledged {
            return Some((
                command_id.clone(),
                format!(
                    "unhanded: Target {} outcome is uncertain (sequence {required}); review and acknowledge before continuing",
                    effect.method
                ),
            ));
        }
    }
    None
}

fn arm_worker_replay(worker: &mut super::WorkerPeer) {
    let floor = worker
        .earliest_unacked_seq
        .unwrap_or(worker.acked_runtime_seq.saturating_add(1))
        .max(worker.acked_runtime_seq.saturating_add(1));
    if worker.highest_runtime_seq >= floor {
        worker.replay_gate = Some((floor, worker.highest_runtime_seq));
    } else {
        worker.replay_gate = None;
    }
    let after_runtime_seq = floor.saturating_sub(1).max(worker.acked_runtime_seq);
    let _ = worker.tx.send(Frame::Replay {
        session_id: worker.snapshot.session_id.clone(),
        worker_epoch: worker.epoch.clone(),
        after_runtime_seq,
    });
}

impl super::Broker {
    pub(super) fn mark_worker_welcomed(&self, session_id: &str, connection_id: u64) -> bool {
        // Finding 12: Acquire controller->workers so any deferred Replay is
        // queued strictly AFTER Frame::Welcome.
        let ctrl_guard = self.controller.lock();
        let mut workers = self.workers.lock();
        if let Some(worker) = workers
            .get_mut(session_id)
            .filter(|w| w.connection_id == connection_id)
        {
            worker.welcomed = true;
            if std::mem::take(&mut worker.replay_deferred) && ctrl_guard.is_some() {
                arm_worker_replay(worker);
            }
            return worker.snapshot.drain_requested;
        }
        false
    }

    pub(super) fn route_prompt_for_optional_lease(
        &self,
        lease: Option<u64>,
        session_id: &str,
        command: WorkerCommand,
    ) {
        self.route_prompt_inner(lease, session_id, command);
    }

    #[cfg(test)]
    pub(super) fn route_prompt_guarded(&self, session_id: &str, command: WorkerCommand) {
        self.route_prompt_inner(None, session_id, command);
    }

    fn dispatch_prior_epoch_prompt(
        &self,
        workers: &std::collections::HashMap<String, super::WorkerPeer>,
        session_id: &str,
        prev_epoch: &str,
        command_id: String,
        command: WorkerCommand,
    ) -> Option<(String, String)> {
        if let Some(worker) = workers.get(session_id)
            && worker.welcomed
            && worker.epoch == prev_epoch
            && worker
                .tx
                .send(Frame::WorkerCommand {
                    session_id: session_id.to_owned(),
                    command: self.trace_worker_dispatch(session_id, command),
                })
                .is_ok()
        {
            None
        } else {
            Some((
                command_id,
                "delivery_ambiguous: original worker changed after prompt handoff".to_owned(),
            ))
        }
    }

    fn route_prompt_inner(
        &self,
        expected_lease: Option<u64>,
        session_id: &str,
        command: WorkerCommand,
    ) {
        let Some((command_id, expected_epoch, prompt_turn_id)) =
            self.extract_prompt_header(session_id, &command)
        else {
            self.route_worker(session_id, command);
            return;
        };
        let reject: Option<(Option<mpsc::UnboundedSender<Frame>>, String, String)> = {
            // Single consistent lock order (`controller` -> `workers` -> `pending_commands` -> `dispatched_prompts`)
            // validates the lease at effect admission (Finding 7) and serializes routing & flushes.
            let ctrl_guard = self.controller.lock();
            let bound_tx = match expected_lease {
                Some(lease) => match ctrl_guard.as_ref().filter(|c| c.lease == lease) {
                    Some(c) => Some(c.tx.clone()),
                    None => {
                        tracing::warn!(lease, session = %session_id, "discarding stale-lease prompt at admission");
                        return;
                    }
                },
                None => ctrl_guard.as_ref().map(|c| c.tx.clone()),
            };
            let workers = self.workers.lock();
            let mut pending = self.pending_commands.lock();
            let mut dispatched = self.dispatched_prompts.lock();
            let prior_epoch = dispatched.get(session_id, &command_id).map(str::to_owned);
            let command_evicted = dispatched.was_command_evicted(session_id, &command_id);
            let refusal_reason = |reason: String| -> String {
                if command_evicted {
                    reason.replacen("unhanded:", "delivery_ambiguous:", 1)
                } else {
                    reason
                }
            };
            if !dispatched.was_ingressed_or_uncertain(session_id, &command_id)
                && dispatched.at_capacity()
            {
                Some((
                    command_id,
                    "unhanded: broker prompt replay history at capacity; retry".to_owned(),
                ))
            } else if let Some(prev_epoch) = prior_epoch {
                self.dispatch_prior_epoch_prompt(
                    &workers,
                    session_id,
                    &prev_epoch,
                    command_id,
                    command,
                )
            } else {
                match workers.get(session_id) {
                    Some(worker) if worker.crashed_drain_fenced => {
                        dispatched.mark_acked(session_id, &command_id);
                        Some((
                            command_id,
                            refusal_reason(
                                "unhanded: crashed worker is draining; retry on replacement".into(),
                            ),
                        ))
                    }
                    Some(worker)
                        if worker.welcomed
                            && (!worker.snapshot.drain_requested
                                || worker.snapshot.state
                                    == crate::runtime_wire::WorkerState::Crashed) =>
                    {
                        if let Some((cid, reason)) = fresh_prompt_refusal(
                            &dispatched,
                            session_id,
                            &command_id,
                            expected_epoch.as_deref(),
                            &worker.epoch,
                            worker.snapshot.uncertain_effect.as_ref(),
                            &command,
                            &refusal_reason,
                        ) {
                            dispatched.mark_acked(session_id, &cid);
                            Some((cid, reason))
                        } else {
                            let traced = self.trace_worker_dispatch(session_id, command);
                            if worker
                                .tx
                                .send(Frame::WorkerCommand {
                                    session_id: session_id.to_owned(),
                                    command: traced,
                                })
                                .is_ok()
                            {
                                dispatched.record_with_turn(
                                    session_id,
                                    &command_id,
                                    &worker.epoch,
                                    prompt_turn_id.as_deref(),
                                );
                                None
                            } else {
                                dispatched.mark_acked(session_id, &command_id);
                                Some((
                                    command_id,
                                    refusal_reason(
                                        "unhanded: worker disconnected before prompt handoff"
                                            .to_owned(),
                                    ),
                                ))
                            }
                        }
                    }
                    Some(worker)
                        if worker.snapshot.drain_requested
                            && expected_epoch
                                .as_deref()
                                .is_some_and(|exp| exp != worker.epoch) =>
                    {
                        dispatched.mark_acked(session_id, &command_id);
                        Some((
                            command_id,
                            refusal_reason(
                                "unhanded: worker epoch changed before prompt admission".to_owned(),
                            ),
                        ))
                    }
                    _ => super::account_unit::enqueue_pending_prompt(
                        &mut pending,
                        session_id,
                        &command_id,
                        command,
                    ),
                }
            }
            .map(|(cid, reason)| (bound_tx, cid, reason))
        };
        if let Some((bound_tx, command_id, reason)) = reject {
            let frame = Frame::CommandAck {
                session_id: session_id.to_owned(),
                command_id,
                accepted: false,
                reason: Some(reason),
            };
            if let Some(tx) = bound_tx {
                let _ = tx.send(frame);
            } else {
                self.send_controller(frame);
            }
        }
    }

    /// Atomically install `controller`, arm contiguous replay gating on every
    /// welcomed worker, and queue `Frame::Replay` on each unbounded worker channel
    /// under `controller -> workers` so an old lease's `Ack` can never overtake
    /// the replacement's `Replay` and no unwelcomed worker receives `Replay`
    /// ahead of `Welcome` (Findings 2, 3, 12).
    pub(super) fn arm_all_worker_replays_locked(
        workers: &mut std::collections::HashMap<String, super::WorkerPeer>,
    ) {
        for worker in workers.values_mut() {
            if worker.welcomed {
                worker.replay_deferred = false;
                arm_worker_replay(worker);
            } else {
                worker.replay_deferred = true;
            }
        }
    }

    /// Revalidate `lease`, advance `acked_runtime_seq` + `earliest_unacked_seq`,
    /// and enqueue `Frame::Ack` under the exact same `controller -> workers` lock
    /// order as `install_controller_and_arm_replays` (Findings 2 & 3).
    pub(super) fn acknowledge_from_controller(
        &self,
        lease: u64,
        session_id: String,
        worker_epoch: String,
        runtime_seq: u64,
    ) {
        let ctrl_guard = self.controller.lock();
        if !ctrl_guard.as_ref().is_some_and(|c| c.lease == lease) {
            tracing::warn!(lease, %session_id, "ignoring Ack from replaced controller");
            return;
        }
        let mut workers = self.workers.lock();
        if let Some(worker) = workers.get_mut(&session_id)
            && worker.epoch == worker_epoch
        {
            worker.acked_runtime_seq = worker.acked_runtime_seq.max(runtime_seq);
            if worker.acked_runtime_seq >= worker.highest_runtime_seq {
                worker.earliest_unacked_seq = None;
            } else if worker
                .earliest_unacked_seq
                .is_some_and(|first| first <= worker.acked_runtime_seq)
            {
                worker.earliest_unacked_seq = Some(worker.acked_runtime_seq.saturating_add(1));
            }
            if worker.welcomed {
                let _ = worker.tx.send(Frame::Ack {
                    session_id,
                    worker_epoch,
                    runtime_seq,
                });
            }
        }
    }

    /// Atomically gate `runtime_seq` against any active replay prefix, track
    /// `earliest_unacked_seq` across BOTH connected and detached delivery, and
    /// capture `(tx, protocol, outer_machine_protocol)` for protocol-sanitized
    /// event forwarding (Findings 2 & 11).
    pub(super) fn admit_and_bind_controller_for_event(
        &self,
        session_id: &str,
        connection_id: u64,
        runtime_seq: u64,
    ) -> Option<(mpsc::UnboundedSender<Frame>, u16, Option<u16>)> {
        let ctrl_guard = self.controller.lock();
        let mut workers = self.workers.lock();
        // Finding 3: Reject frames from a replaced/absent worker connection
        // under the joint controller->workers locks before binding the sender.
        let worker = workers
            .get_mut(session_id)
            .filter(|w| w.connection_id == connection_id)?;
        worker.highest_runtime_seq = worker.highest_runtime_seq.max(runtime_seq);
        if runtime_seq > worker.acked_runtime_seq {
            worker.earliest_unacked_seq = Some(
                worker
                    .earliest_unacked_seq
                    .map_or(runtime_seq, |first| first.min(runtime_seq)),
            );
        }
        let ctrl = ctrl_guard.as_ref()?;
        if let Some((next_expected, until_seq)) = worker.replay_gate {
            if runtime_seq != next_expected {
                return None;
            }
            if runtime_seq >= until_seq {
                worker.replay_gate = None;
            } else {
                worker.replay_gate = Some((next_expected.saturating_add(1), until_seq));
            }
        }
        Some((ctrl.tx.clone(), ctrl.protocol, ctrl.outer_machine_protocol))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_worker_event(
        self: &Arc<Self>,
        session_id: &str,
        connection_id: u64,
        event_session: String,
        worker_epoch: String,
        runtime_seq: u64,
        event: crate::runtime_wire::RuntimeEvent,
        diagnostics: Vec<crate::runtime_trace::SpanRecord>,
    ) {
        let cancelled = self.cancelled_sessions.lock();
        if cancelled.contains(session_id) {
            let permanently_deleted = self
                .permanently_deleted_sessions
                .lock()
                .contains(session_id);
            drop(cancelled);
            if permanently_deleted {
                self.acknowledge_worker_event(session_id, connection_id, worker_epoch, runtime_seq);
            } else if let Some((ctrl_tx, protocol, outer)) =
                self.admit_and_bind_controller_for_event(session_id, connection_id, runtime_seq)
            {
                self.send_sanitized_to_controller(
                    &ctrl_tx,
                    protocol,
                    outer,
                    Frame::WorkerEvent {
                        session_id: event_session,
                        worker_epoch,
                        runtime_seq,
                        event,
                        diagnostics,
                    },
                );
            }
            return;
        }
        let bound_ctrl =
            self.admit_and_bind_controller_for_event(session_id, connection_id, runtime_seq);
        let rehabilitated = self.update_from_event(session_id, connection_id, runtime_seq, &event);
        if let Some((ctrl_tx, protocol, outer)) = bound_ctrl {
            self.send_sanitized_to_controller(
                &ctrl_tx,
                protocol,
                outer,
                Frame::WorkerEvent {
                    session_id: event_session,
                    worker_epoch,
                    runtime_seq,
                    event,
                    diagnostics,
                },
            );
        }
        self.maybe_cutover(session_id);
        drop(cancelled);
        for fallback_session in rehabilitated {
            if fallback_session != session_id {
                self.maybe_cutover(&fallback_session);
            }
        }
    }

    pub(super) fn flush_pending_guarded(&self, session_id: &str) {
        let mut rejects = Vec::new();
        {
            let workers = self.workers.lock();
            let Some(worker) = workers.get(session_id).filter(|w| w.welcomed) else {
                return;
            };
            let mut pending = self.pending_commands.lock();
            let Some(queue) = pending.get_mut(session_id) else {
                return;
            };
            let mut dispatched = self.dispatched_prompts.lock();
            let mut remaining = std::collections::VecDeque::new();
            while let Some(command) = queue.pop_front() {
                if worker.crashed_drain_fenced
                    || (worker.snapshot.drain_requested
                        && worker.snapshot.state != crate::runtime_wire::WorkerState::Crashed
                        && matches!(command, WorkerCommand::Prompt { .. }))
                {
                    remaining.push_back(command);
                    continue;
                }
                let expected_epoch = match &command {
                    WorkerCommand::Prompt {
                        expected_worker_epoch,
                        ..
                    } => expected_worker_epoch.as_deref(),
                    _ => None,
                };
                let flush_turn_id = match &command {
                    WorkerCommand::Prompt { turn_id, .. } => Some(turn_id.clone()),
                    _ => None,
                };
                if let WorkerCommand::Prompt { ref command_id, .. } = command {
                    let command_evicted = dispatched.was_command_evicted(session_id, command_id);
                    let refusal_reason = |reason: String| -> String {
                        if command_evicted {
                            reason.replacen("unhanded:", "delivery_ambiguous:", 1)
                        } else {
                            reason
                        }
                    };
                    if let Some((cid, reason)) = fresh_prompt_refusal(
                        &dispatched,
                        session_id,
                        command_id,
                        expected_epoch,
                        &worker.epoch,
                        worker.snapshot.uncertain_effect.as_ref(),
                        &command,
                        &refusal_reason,
                    ) {
                        dispatched.mark_acked(session_id, &cid);
                        rejects.push((cid, reason));
                        continue;
                    }
                }
                let prompt_cid = match &command {
                    WorkerCommand::Prompt { command_id, .. } => Some(command_id.clone()),
                    _ => None,
                };
                let traced = self.trace_worker_dispatch(session_id, command);
                if worker
                    .tx
                    .send(Frame::WorkerCommand {
                        session_id: session_id.to_owned(),
                        command: traced,
                    })
                    .is_ok()
                {
                    if let Some(cid) = prompt_cid {
                        dispatched.record_with_turn(
                            session_id,
                            &cid,
                            &worker.epoch,
                            flush_turn_id.as_deref(),
                        );
                    }
                } else if let Some(cid) = prompt_cid {
                    let reason = if dispatched.was_command_evicted(session_id, &cid) {
                        "delivery_ambiguous: worker disconnected before queued prompt handoff"
                    } else {
                        "unhanded: worker disconnected before queued prompt handoff"
                    };
                    dispatched.mark_acked(session_id, &cid);
                    rejects.push((cid, reason.to_owned()));
                }
            }
            if remaining.is_empty() {
                pending.remove(session_id);
            } else {
                *queue = remaining;
            }
        }
        for (command_id, reason) in rejects {
            self.send_controller(Frame::CommandAck {
                session_id: session_id.to_owned(),
                command_id,
                accepted: false,
                reason: Some(reason),
            });
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
            native_cycle_active: false,
            incarnation: None,
            account_unit: None,
            uncertain_effect: None,
            crashed_drain_active: false,
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
        let lease = broker.attach_controller_with_outer(8, Some(53), &ctrl_tx, None);
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
                earliest_unacked_seq: None,
                highest_runtime_seq: 0,
                replay_gate: None,
                welcomed: true,
                replay_deferred: false,
                last_seen: std::time::Instant::now(),
                receive_probe: None,
                liveness: None,
                negotiated_protocol: 8,
                raw_wrap_up_steering: true,
                hint_tx: Some(hint_tx),
                diag_reply_tx: None,
                last_hinted_target: None,
                crashed_drain_fenced: false,
                peer_pid: None,
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
        let p8 = crate::runtime_wire::WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION;
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
        let mut v8_omitted = crate::runtime_wire::CoreCommand::Prompt {
            session_id: "s1".into(),
            command_id: "cmd-v8".into(),
            turn_id: "t-v8".into(),
            content: vec![],
            cmid: None,
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: None,
            acknowledged_uncertain_sequence: None,
        };
        let mut legacy_omitted = v8_omitted.clone();
        normalize_ingress_prompt(p8, &mut v8_omitted);
        normalize_ingress_prompt(p8 - 1, &mut legacy_omitted);
        let (_, v8_worker_cmd) = v8_omitted.into_worker_prompt().unwrap();
        let (_, legacy_worker_cmd) = legacy_omitted.into_worker_prompt().unwrap();
        assert!(prompt_admission_refusal("e-old", Some(&effect), &v8_worker_cmd).is_some());
        assert!(prompt_admission_refusal("e-old", Some(&effect), &legacy_worker_cmd).is_none());
    }

    #[test]
    fn route_prompt_guarded_distinguishes_unhanded_same_epoch_retry_and_ambiguous_handoff() {
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
        let (ctrl_tx, mut ctrl_rx) = mpsc::unbounded_channel();
        broker.attach_controller(
            crate::runtime_wire::WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION,
            &ctrl_tx,
        );
        let _ = ctrl_rx.try_recv(); // Welcome

        // Attach worker epoch e1.
        let (w1_tx, mut w1_rx) = mpsc::unbounded_channel();
        let mut s1 = sample_snapshot(Some("antigravity"));
        s1.worker_epoch = "e1".into();
        broker.workers.lock().insert(
            "s1".into(),
            super::super::WorkerPeer {
                connection_id: 1,
                epoch: "e1".into(),
                tx: w1_tx,
                snapshot: s1,
                acked_runtime_seq: 0,
                earliest_unacked_seq: None,
                highest_runtime_seq: 0,
                replay_gate: None,
                welcomed: true,
                replay_deferred: false,
                last_seen: std::time::Instant::now(),
                receive_probe: None,
                liveness: None,
                negotiated_protocol: crate::runtime_wire::WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION,
                raw_wrap_up_steering: true,
                hint_tx: None,
                diag_reply_tx: None,
                last_hinted_target: None,
                crashed_drain_fenced: false,
                peer_pid: None,
            },
        );

        // 1. Unhanded stale-epoch prompt is rejected with `unhanded:` prefix.
        let stale = WorkerCommand::Prompt {
            command_id: "cmd-stale".into(),
            turn_id: "t-stale".into(),
            content: vec![],
            cmid: None,
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: Some("e0".into()),
            acknowledged_uncertain_sequence: Some(0),
        };
        broker.route_prompt_guarded("s1", stale);
        let ack = ctrl_rx.try_recv().unwrap();
        assert!(matches!(
            ack,
            Frame::CommandAck {
                command_id,
                accepted: false,
                reason: Some(reason),
                ..
            } if command_id == "cmd-stale" && reason.starts_with("unhanded:")
        ));

        // 2. First dispatch to e1 succeeds and records handoff; same-epoch retry
        // still forwards to e1 even if e1 later acquires an uncertain_effect
        // so e1 can return its authoritative duplicate ack.
        let first = WorkerCommand::Prompt {
            command_id: "cmd-once".into(),
            turn_id: "t-once".into(),
            content: vec![],
            cmid: None,
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: Some("e1".into()),
            acknowledged_uncertain_sequence: Some(0),
        };
        broker.route_prompt_guarded("s1", first.clone());
        assert!(w1_rx.try_recv().is_ok());

        broker
            .workers
            .lock()
            .get_mut("s1")
            .unwrap()
            .snapshot
            .uncertain_effect = Some(crate::runtime_wire::UncertainEffect {
            method: "fs/writeFile".into(),
            sequence: 7,
        });
        broker.route_prompt_guarded("s1", first.clone());
        assert!(w1_rx.try_recv().is_ok());

        // 3. After replacement worker e2 takes over, retrying cmd-once emits
        // `delivery_ambiguous:` instead of `unhanded:`.
        let (w2_tx, mut w2_rx) = mpsc::unbounded_channel();
        let mut s2 = sample_snapshot(Some("antigravity"));
        s2.worker_epoch = "e2".into();
        broker.workers.lock().insert(
            "s1".into(),
            super::super::WorkerPeer {
                connection_id: 2,
                epoch: "e2".into(),
                tx: w2_tx,
                snapshot: s2,
                acked_runtime_seq: 0,
                earliest_unacked_seq: None,
                highest_runtime_seq: 0,
                replay_gate: None,
                welcomed: true,
                replay_deferred: false,
                last_seen: std::time::Instant::now(),
                receive_probe: None,
                liveness: None,
                negotiated_protocol: crate::runtime_wire::WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION,
                raw_wrap_up_steering: true,
                hint_tx: None,
                diag_reply_tx: None,
                last_hinted_target: None,
                crashed_drain_fenced: false,
                peer_pid: None,
            },
        );
        broker.route_prompt_guarded("s1", first);
        assert!(w2_rx.try_recv().is_err());
        let amb = ctrl_rx.try_recv().unwrap();
        assert!(matches!(
            amb,
            Frame::CommandAck {
                command_id,
                accepted: false,
                reason: Some(reason),
                ..
            } if command_id == "cmd-once" && reason.starts_with("delivery_ambiguous:")
        ));
    }

    #[test]
    fn replay_gate_blocks_out_of_order_live_event_until_contiguous_replay_prefix() {
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
        let (w_tx, _w_rx) = mpsc::unbounded_channel();
        broker.workers.lock().insert(
            "s1".into(),
            super::super::WorkerPeer {
                connection_id: 1,
                epoch: "e1".into(),
                tx: w_tx,
                snapshot: sample_snapshot(Some("antigravity")),
                acked_runtime_seq: 0,
                earliest_unacked_seq: None,
                highest_runtime_seq: 0,
                replay_gate: None,
                welcomed: true,
                replay_deferred: false,
                last_seen: std::time::Instant::now(),
                receive_probe: None,
                liveness: None,
                negotiated_protocol: 8,
                raw_wrap_up_steering: true,
                hint_tx: None,
                diag_reply_tx: None,
                last_hinted_target: None,
                crashed_drain_fenced: false,
                peer_pid: None,
            },
        );
        assert!(
            broker
                .admit_and_bind_controller_for_event("s1", 1, 9)
                .is_none()
        );

        let (ctrl_tx, _ctrl_rx) = mpsc::unbounded_channel();
        super::super::post_edit::install_controller_and_arm_replays(
            &broker,
            super::super::Controller {
                lease: 1,
                tx: ctrl_tx,
                protocol: 8,
                outer_machine_protocol: Some(53),
            },
            None,
        );

        assert!(
            broker
                .admit_and_bind_controller_for_event("s1", 1, 10)
                .is_none()
        );
        assert!(
            broker
                .admit_and_bind_controller_for_event("s1", 1, 9)
                .is_some()
        );
        assert!(
            broker
                .admit_and_bind_controller_for_event("s1", 1, 10)
                .is_some()
        );

        // Finding 2: A second reconnect BEFORE seq 9..10 are ACKed must STILL
        // arm replay_gate at floor 9 (never fall back to pruned seq 1).
        let (ctrl_tx2, _ctrl_rx2) = mpsc::unbounded_channel();
        super::super::post_edit::install_controller_and_arm_replays(
            &broker,
            super::super::Controller {
                lease: 2,
                tx: ctrl_tx2,
                protocol: 8,
                outer_machine_protocol: Some(53),
            },
            None,
        );
        assert!(
            broker
                .admit_and_bind_controller_for_event("s1", 1, 10)
                .is_none(),
            "second reconnect before ACK must still wait for unacked seq 9"
        );
        assert!(
            broker
                .admit_and_bind_controller_for_event("s1", 1, 9)
                .is_some()
        );
    }

    #[test]
    fn dropped_unhanded_ingress_retires_record_without_evicting_prior_ingress() {
        let mut tracked = DispatchedPrompts::default();
        assert!(tracked.mark_ingressed("s1", "cmd-drop"));
        tracked.retire_if_unhanded("s1", "cmd-drop");
        assert!(!tracked.was_ingressed_or_uncertain("s1", "cmd-drop"));

        assert!(tracked.mark_ingressed("s1", "cmd-live"));
        assert!(!tracked.mark_ingressed("s1", "cmd-live"));
        tracked.record("s1", "cmd-live", "e1");
        tracked.retire_if_unhanded("s1", "cmd-live");
        assert_eq!(tracked.get("s1", "cmd-live"), Some("e1"));
    }

    #[test]
    fn sustained_unhanded_overflows_keep_eviction_order_bounded() {
        let mut tracked = DispatchedPrompts::default();
        // Non-tail retirement + repeated reinsertion of the SAME key exercises generation compaction (F13).
        assert!(tracked.mark_ingressed("s1", "reused-key"));
        assert!(tracked.mark_ingressed("s1", "tail-blocker"));
        for _ in 0..(DispatchedPrompts::MAX_ENTRIES + 512) {
            tracked.retire_if_unhanded("s1", "reused-key");
            assert!(tracked.mark_ingressed("s1", "reused-key"));
            tracked.retire_if_unhanded("s1", "tail-blocker");
            assert!(tracked.mark_ingressed("s1", "tail-blocker"));
        }
        assert_eq!(tracked.entries.len(), 2);
        assert!(
            tracked.order.len() <= DispatchedPrompts::MAX_ENTRIES,
            "order index must drop stale generations on compaction, got {}",
            tracked.order.len()
        );
    }

    #[test]
    fn evicted_prior_epoch_blocks_unbound_replay_on_replacement_worker() {
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
        let (ctrl_tx, mut ctrl_rx) = mpsc::unbounded_channel();
        broker.attach_controller(
            crate::runtime_wire::WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION,
            &ctrl_tx,
        );
        let _ = ctrl_rx.try_recv();
        let (w_tx, mut w_rx) = mpsc::unbounded_channel();
        let mut snap = sample_snapshot(Some("antigravity"));
        snap.session_id = "s-evict".into();
        snap.worker_epoch = "epoch-new".into();
        broker.workers.lock().insert(
            "s-evict".into(),
            super::super::WorkerPeer {
                connection_id: 1,
                epoch: "epoch-new".into(),
                tx: w_tx,
                snapshot: snap,
                acked_runtime_seq: 0,
                earliest_unacked_seq: None,
                highest_runtime_seq: 0,
                replay_gate: None,
                welcomed: true,
                replay_deferred: false,
                last_seen: std::time::Instant::now(),
                receive_probe: None,
                liveness: None,
                negotiated_protocol: crate::runtime_wire::WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION,
                raw_wrap_up_steering: true,
                hint_tx: None,
                diag_reply_tx: None,
                last_hinted_target: None,
                crashed_drain_fenced: false,
                peer_pid: None,
            },
        );

        // Simulate prior eviction of `cmd-unbound` handed off on epoch-old.
        broker.dispatched_prompts.lock().record_eviction(
            "s-evict".into(),
            "cmd-unbound",
            DispatchedPromptRecord {
                epoch: Some("epoch-old".into()),
                turn_id: None,
                acked: true,
                seq: 0,
            },
        );
        // Replaying the evicted command unbound on a replacement worker must be refused.
        broker.route_prompt_guarded(
            "s-evict",
            WorkerCommand::Prompt {
                command_id: "cmd-unbound".into(),
                turn_id: "t1".into(),
                content: vec![],
                cmid: Some("c1".into()),
                trace: None,
                echo_artifacts: false,
                observe_subagents: false,
                expected_worker_epoch: None,
                acknowledged_uncertain_sequence: None,
            },
        );
        assert!(
            w_rx.try_recv().is_err(),
            "evicted unbound replay must not reach worker"
        );
        match ctrl_rx.try_recv().expect("refusal ack") {
            Frame::CommandAck {
                command_id,
                accepted,
                reason,
                ..
            } => {
                assert_eq!(command_id, "cmd-unbound");
                assert!(!accepted);
                assert!(
                    reason
                        .as_deref()
                        .is_some_and(|r| r.starts_with("delivery_ambiguous:")),
                    "expected delivery_ambiguous refusal, got {reason:?}"
                );
            }
            other => panic!("expected CommandAck, got {other:?}"),
        }
        // A brand-new unbound cold-wake / legacy prompt must still succeed on the replacement worker.
        broker.route_prompt_guarded(
            "s-evict",
            WorkerCommand::Prompt {
                command_id: "cmd-fresh-cold-wake".into(),
                turn_id: "t2".into(),
                content: vec![],
                cmid: Some("c2".into()),
                trace: None,
                echo_artifacts: false,
                observe_subagents: false,
                expected_worker_epoch: None,
                acknowledged_uncertain_sequence: None,
            },
        );
        assert!(
            matches!(
                w_rx.try_recv().expect("fresh cold-wake reaches worker"),
                Frame::WorkerCommand {
                    command: WorkerCommand::Prompt { command_id, .. },
                    ..
                } if command_id == "cmd-fresh-cold-wake"
            ),
            "fresh unbound prompt must not be blocked by an earlier evicted command"
        );
        // Bound v8 prompt targeting epoch-new still succeeds.
        broker.route_prompt_guarded(
            "s-evict",
            WorkerCommand::Prompt {
                command_id: "cmd-bound".into(),
                turn_id: "t2".into(),
                content: vec![],
                cmid: Some("c2".into()),
                trace: None,
                echo_artifacts: false,
                observe_subagents: false,
                expected_worker_epoch: Some("epoch-new".into()),
                acknowledged_uncertain_sequence: Some(0),
            },
        );
        assert!(
            w_rx.try_recv().is_ok(),
            "bound prompt for epoch-new must dispatch"
        );

        // An evicted unbound prompt queued BEFORE replacement worker connects must
        // be refused on flush_pending_guarded, while a brand-new cold-wake prompt
        // queued beside it still flushes cleanly to epoch-new.
        broker.workers.lock().remove("s-evict");
        broker.route_prompt_guarded(
            "s-evict",
            WorkerCommand::Prompt {
                command_id: "cmd-unbound".into(),
                turn_id: "t3".into(),
                content: vec![],
                cmid: Some("c3".into()),
                trace: None,
                echo_artifacts: false,
                observe_subagents: false,
                expected_worker_epoch: None,
                acknowledged_uncertain_sequence: None,
            },
        );
        broker.route_prompt_guarded(
            "s-evict",
            WorkerCommand::Prompt {
                command_id: "cmd-queued-fresh".into(),
                turn_id: "t4".into(),
                content: vec![],
                cmid: Some("c4".into()),
                trace: None,
                echo_artifacts: false,
                observe_subagents: false,
                expected_worker_epoch: None,
                acknowledged_uncertain_sequence: None,
            },
        );
        let (w2_tx, mut w2_rx) = mpsc::unbounded_channel();
        let mut snap2 = sample_snapshot(Some("antigravity"));
        snap2.session_id = "s-evict".into();
        snap2.worker_epoch = "epoch-new".into();
        broker.workers.lock().insert(
            "s-evict".into(),
            super::super::WorkerPeer {
                connection_id: 2,
                epoch: "epoch-new".into(),
                tx: w2_tx,
                snapshot: snap2,
                acked_runtime_seq: 0,
                earliest_unacked_seq: None,
                highest_runtime_seq: 0,
                replay_gate: None,
                welcomed: true,
                replay_deferred: false,
                last_seen: std::time::Instant::now(),
                receive_probe: None,
                liveness: None,
                negotiated_protocol: crate::runtime_wire::WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION,
                raw_wrap_up_steering: true,
                hint_tx: None,
                diag_reply_tx: None,
                last_hinted_target: None,
                crashed_drain_fenced: false,
                peer_pid: None,
            },
        );
        broker.flush_pending_guarded("s-evict");
        assert!(
            matches!(
                w2_rx.try_recv().expect("fresh queued prompt flushes"),
                Frame::WorkerCommand {
                    command: WorkerCommand::Prompt { command_id, .. },
                    ..
                } if command_id == "cmd-queued-fresh"
            ),
            "only the fresh queued prompt should flush to replacement worker"
        );
        assert!(
            w2_rx.try_recv().is_err(),
            "evicted queued unbound prompt must not flush to replacement worker"
        );
        match ctrl_rx.try_recv().expect("flush refusal ack") {
            Frame::CommandAck {
                command_id,
                accepted,
                reason,
                ..
            } => {
                assert_eq!(command_id, "cmd-unbound");
                assert!(!accepted);
                assert!(
                    reason
                        .as_deref()
                        .is_some_and(|r| r.starts_with("delivery_ambiguous:"))
                );
            }
            other => panic!("expected CommandAck, got {other:?}"),
        }
    }

    #[test]
    fn broker_dispatched_prompts_bounds_admission_without_forgetting_evicted_handoffs() {
        let mut tracker = DispatchedPrompts::default();
        // Saturate evicted_commands to its exact cap with worker-acked handoffs.
        for i in 0..DispatchedPrompts::MAX_EVICTED_COMMANDS {
            let cid = format!("evicted-{i}");
            assert!(tracker.record_eviction(
                "s-cap".into(),
                &cid,
                DispatchedPromptRecord {
                    epoch: Some("epoch-old".into()),
                    turn_id: None,
                    acked: true,
                    seq: 0,
                },
            ));
        }
        assert_eq!(
            tracker.evicted_commands.len(),
            DispatchedPrompts::MAX_EVICTED_COMMANDS
        );
        // Refuses growing past cap and retains the very first evicted handoff.
        assert!(!tracker.record_eviction(
            "s-cap".into(),
            "evicted-overflow",
            DispatchedPromptRecord {
                epoch: Some("epoch-old".into()),
                turn_id: None,
                acked: true,
                seq: 0,
            },
        ));
        assert_eq!(
            tracker.evicted_commands.len(),
            DispatchedPrompts::MAX_EVICTED_COMMANDS
        );
        assert!(
            tracker.evicted_blocks_replay("s-cap", "evicted-0", "epoch-new"),
            "earliest worker-acked handoff must never be forgotten"
        );
        // Saturate active entries to hard bound; at_capacity() must fail closed
        // without dropping any evicted record.
        for i in 0..(DispatchedPrompts::MAX_ENTRIES * 2) {
            let cid = format!("live-{i}");
            tracker.entries.insert(
                ("s-cap".into(), cid),
                DispatchedPromptRecord {
                    epoch: Some("epoch-new".into()),
                    turn_id: None,
                    acked: true,
                    seq: i as u64,
                },
            );
        }
        assert!(tracker.at_capacity());
        assert_eq!(
            tracker.evicted_commands.len(),
            DispatchedPrompts::MAX_EVICTED_COMMANDS
        );
        assert!(tracker.evicted_blocks_replay("s-cap", "evicted-0", "epoch-new"));
    }
    #[test]
    fn dead_epoch_unacked_prompt_does_not_block_newer_epoch_drain_check() {
        let mut dispatched = DispatchedPrompts::default();
        dispatched.record("s-m5", "cmd-old", "1-1");
        assert!(dispatched.has_unacked_for_epoch_or_ingress("s-m5", "1-1"));
        assert!(
            !dispatched.has_unacked_for_epoch_or_ingress("s-m5", "2-1"),
            "dead-epoch unacked prompt must not veto a newer replacement epoch"
        );
        // An ingress-stage prompt (epoch=None) still blocks any epoch.
        assert!(dispatched.mark_ingressed("s-m5", "cmd-ingress"));
        assert!(dispatched.has_unacked_for_epoch_or_ingress("s-m5", "2-1"));
    }
}
