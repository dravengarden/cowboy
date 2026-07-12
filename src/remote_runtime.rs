//! Cowboy-side client for the stable agentd runtime broker.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use parking_lot::Mutex;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use crate::core::{Event, Hub, Status};
use crate::runtime_wire::{
    read_frame, write_frame, CoreCommand, Frame, PeerRole, RuntimeEvent, StartSession,
    WorkerSnapshot, WorkerState, MIN_PROTOCOL_VERSION, PROTOCOL_VERSION,
};

pub struct RemoteBootstrap {
    socket: PathBuf,
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
    workers: Vec<WorkerSnapshot>,
    buffered: Vec<Frame>,
}

impl RemoteBootstrap {
    pub async fn connect(socket: PathBuf) -> Result<Self> {
        let mut backoff = Duration::from_millis(50);
        loop {
            match connect_settled(&socket).await {
                Ok((reader, writer, workers, buffered)) => {
                    return Ok(Self {
                        socket,
                        reader,
                        writer,
                        workers,
                        buffered,
                    });
                }
                Err(error) => {
                    tracing::debug!(%error, socket = %socket.display(), "waiting for agentd bootstrap");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(1));
                }
            }
        }
    }

    #[must_use]
    pub fn workers(&self) -> &[WorkerSnapshot] {
        &self.workers
    }
}

struct Shared {
    socket: PathBuf,
    hub: Hub,
    pending: Mutex<HashMap<String, CoreCommand>>,
    sent: Mutex<HashSet<String>>,
    /// Launch metadata re-declared after every broker reconnect. These are
    /// registry claims, not pending commands: absence never authorizes agentd
    /// to spawn while surviving workers are still converging.
    declarations: Mutex<HashMap<String, StartSession>>,
    workers: Mutex<HashMap<String, WorkerSnapshot>>,
    highwaters: Mutex<HashMap<(String, String), u64>>,
    notify: mpsc::UnboundedSender<()>,
    command_counter: AtomicU64,
    turn_counter: AtomicU64,
    connected: AtomicBool,
    shutdown: AtomicBool,
    desired_generation: String,
    desired_worker_command: Option<String>,
}

pub struct RemoteRuntime {
    shared: Arc<Shared>,
    notify_rx: Mutex<Option<mpsc::UnboundedReceiver<()>>>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RemoteRuntimeStats {
    pub workers: usize,
    pub busy_workers: usize,
    pub draining_workers: usize,
    /// Non-busy generation handoffs that still depend on broker-local
    /// readiness/fallback state. Agentd updates wait for this to reach zero.
    pub handoff_workers: usize,
    pub pending_commands: usize,
}

impl RemoteRuntime {
    #[must_use]
    pub fn new(
        hub: Hub,
        bootstrap: &RemoteBootstrap,
        desired_generation: String,
        desired_worker_command: Option<String>,
    ) -> Arc<Self> {
        let (notify, notify_rx) = mpsc::unbounded_channel();
        let declarations = bootstrap
            .workers
            .iter()
            .filter_map(|worker| worker.launch.clone())
            .map(|mut session| {
                session.adopt_only = true;
                (session.session_id.clone(), session)
            })
            .collect();
        Arc::new(Self {
            shared: Arc::new(Shared {
                socket: bootstrap.socket.clone(),
                hub,
                pending: Mutex::new(HashMap::new()),
                sent: Mutex::new(HashSet::new()),
                declarations: Mutex::new(declarations),
                workers: Mutex::new(
                    bootstrap
                        .workers
                        .iter()
                        .cloned()
                        .map(|worker| (worker.session_id.clone(), worker))
                        .collect(),
                ),
                highwaters: Mutex::new(HashMap::new()),
                notify,
                command_counter: AtomicU64::new(seed_counter()),
                turn_counter: AtomicU64::new(seed_counter()),
                connected: AtomicBool::new(false),
                shutdown: AtomicBool::new(false),
                desired_generation,
                desired_worker_command,
            }),
            notify_rx: Mutex::new(Some(notify_rx)),
        })
    }

    /// Start the reconnecting I/O pump after Hub restore and all side-effect
    /// consumers (dispatcher/scheduler) are wired.
    pub fn start(self: &Arc<Self>, bootstrap: RemoteBootstrap) {
        let Some(notify_rx) = self.notify_rx.lock().take() else {
            return;
        };
        let shared = Arc::clone(&self.shared);
        tokio::spawn(async move {
            for worker in &bootstrap.workers {
                apply_snapshot(&shared.hub, worker);
            }
            connection_manager(
                shared,
                notify_rx,
                Some((bootstrap.reader, bootstrap.writer, bootstrap.buffered)),
            )
            .await;
        });
    }

    #[must_use]
    pub fn connected(&self) -> bool {
        self.shared.connected.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn has_worker(&self, session_id: &str) -> bool {
        self.shared
            .workers
            .lock()
            .get(session_id)
            .is_some_and(|worker| {
                !matches!(worker.state, WorkerState::Exited | WorkerState::Crashed)
            })
    }

    #[must_use]
    pub fn stats(&self) -> RemoteRuntimeStats {
        let workers = self.shared.workers.lock();
        RemoteRuntimeStats {
            workers: workers.len(),
            busy_workers: workers
                .values()
                .filter(|worker| worker.state == WorkerState::Busy)
                .count(),
            draining_workers: workers
                .values()
                .filter(|worker| worker.state == WorkerState::Draining || worker.drain_requested)
                .count(),
            handoff_workers: workers
                .values()
                .filter(|worker| {
                    matches!(worker.state, WorkerState::Starting | WorkerState::Draining)
                        || (worker.drain_requested && worker.state == WorkerState::Running)
                })
                .count(),
            pending_commands: self.shared.pending.lock().len(),
        }
    }

    pub fn ensure(&self, mut session: StartSession) {
        session.adopt_only = false;
        if session.generation.is_empty() {
            session
                .generation
                .clone_from(&self.shared.desired_generation);
        }
        let key = format!("ensure:{}", session.session_id);
        self.queue(key, CoreCommand::EnsureSession { session });
    }

    /// Rebuild agentd's session registry without treating a temporarily absent
    /// worker as permission to start another owner. Used at core/broker
    /// deployment boundaries; normal user-driven revival uses [`Self::ensure`].
    pub fn adopt(&self, mut session: StartSession) {
        if let Some(worker) = self.shared.workers.lock().get(&session.session_id) {
            if let Some(launch) = worker.launch.clone() {
                session = launch;
            } else {
                session.generation.clone_from(&worker.generation);
                session
                    .agent_session_id
                    .clone_from(&worker.agent_session_id);
            }
        }
        if session.generation.is_empty() {
            session
                .generation
                .clone_from(&self.shared.desired_generation);
        }
        session.adopt_only = true;
        self.shared
            .declarations
            .lock()
            .insert(session.session_id.clone(), session);
    }

    pub fn prompt(
        &self,
        session_id: &str,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) -> String {
        let command_id = self.next_id("cmd");
        let turn_id = self.next_turn_id();
        self.queue(
            command_id.clone(),
            CoreCommand::Prompt {
                session_id: session_id.to_owned(),
                command_id: command_id.clone(),
                turn_id,
                content,
                cmid,
            },
        );
        command_id
    }

    pub fn cancel(&self, session_id: &str) {
        let command_id = self.next_id("cancel");
        self.queue(
            command_id.clone(),
            CoreCommand::Cancel {
                session_id: session_id.to_owned(),
                command_id,
            },
        );
    }

    pub fn permission(&self, session_id: &str, request_id: String, option_id: Option<String>) {
        let command_id = self.next_id("permission");
        self.queue(
            command_id.clone(),
            CoreCommand::Permission {
                session_id: session_id.to_owned(),
                command_id,
                request_id,
                option_id,
            },
        );
    }

    pub fn set_config_option(&self, session_id: &str, config_id: String, value: serde_json::Value) {
        let command_id = self.next_id("config");
        self.queue(
            command_id.clone(),
            CoreCommand::SetConfigOption {
                session_id: session_id.to_owned(),
                command_id,
                config_id,
                value,
            },
        );
    }

    pub fn stop(&self, session_id: &str) {
        self.shared.declarations.lock().remove(session_id);
        let command_id = self.next_id("stop");
        self.queue(
            command_id.clone(),
            CoreCommand::StopSession {
                session_id: session_id.to_owned(),
                command_id,
            },
        );
    }

    /// Stop accepting a deployment boundary only after every command already
    /// handed off by the Hub has been acknowledged by its worker. If the
    /// runtime stays unavailable through the deadline, return prompts to the
    /// durable Hub queue before the Postgres writer is closed.
    pub async fn graceful_shutdown(&self, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        while !self.shared.pending.lock().is_empty() && tokio::time::Instant::now() < deadline {
            let _ = self.shared.notify.send(());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let broker_owns_sent = self.connected();
        let sent = self.shared.sent.lock().clone();
        self.shared.shutdown.store(true, Ordering::Release);
        let _ = self.shared.notify.send(());

        let pending = std::mem::take(&mut *self.shared.pending.lock());
        for (key, command) in pending {
            let Some(session_id) = command.session_id().map(str::to_owned) else {
                continue;
            };
            if matches!(command, CoreCommand::Prompt { .. })
                && !(broker_owns_sent && sent.contains(&key))
            {
                handle_rejected_command(
                    &self.shared.hub,
                    &session_id,
                    Some(command),
                    Some(
                        "runtime unavailable during shutdown; prompt returned to queue".to_owned(),
                    ),
                );
            }
        }
    }

    fn queue(&self, key: String, command: CoreCommand) {
        self.shared.sent.lock().remove(&key);
        self.shared.pending.lock().insert(key, command);
        let _ = self.shared.notify.send(());
    }

    fn next_id(&self, prefix: &str) -> String {
        let value = self.shared.command_counter.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-{}-{value}", std::process::id())
    }

    fn next_turn_id(&self) -> String {
        let value = self.shared.turn_counter.fetch_add(1, Ordering::Relaxed);
        format!("turn-{}-{value}", std::process::id())
    }
}

fn seed_counter() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(1)
}

async fn connect_once(
    socket: &Path,
) -> Result<(OwnedReadHalf, OwnedWriteHalf, Vec<WorkerSnapshot>)> {
    let stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connecting agentd socket {}", socket.display()))?;
    let (mut reader, mut writer) = stream.into_split();
    write_frame(
        &mut writer,
        &Frame::Hello {
            role: PeerRole::Core,
            min_protocol: MIN_PROTOCOL_VERSION,
            max_protocol: PROTOCOL_VERSION,
            build: env!("CARGO_PKG_VERSION").to_owned(),
            session_id: None,
            worker_epoch: None,
            generation: None,
            executable: None,
            fallback_for: None,
        },
    )
    .await?;
    match read_frame(&mut reader).await? {
        Some(Frame::Welcome { workers, .. }) => Ok((reader, writer, workers)),
        Some(Frame::Reject { reason }) => anyhow::bail!("agentd rejected Cowboy: {reason}"),
        Some(other) => anyhow::bail!("unexpected agentd handshake frame: {other:?}"),
        None => anyhow::bail!("agentd closed during handshake"),
    }
}

/// Let surviving workers converge around every new broker connection, not only
/// process startup. Until this settles, `Shared.workers` intentionally keeps its
/// last snapshot so an agentd restart cannot race an `EnsureSession` into
/// spawning a duplicate owner for a still-running turn.
async fn connect_settled(
    socket: &Path,
) -> Result<(
    OwnedReadHalf,
    OwnedWriteHalf,
    Vec<WorkerSnapshot>,
    Vec<Frame>,
)> {
    let (mut reader, writer, mut workers) = connect_once(socket).await?;
    let mut buffered = Vec::new();
    let now = tokio::time::Instant::now();
    let minimum_settle = now + Duration::from_millis(500);
    let deadline = now + Duration::from_secs(1);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let quiet = remaining.min(Duration::from_millis(250));
        match tokio::time::timeout(quiet, read_frame(&mut reader)).await {
            Ok(Ok(Some(frame))) => {
                if let Frame::Snapshot { worker } = &frame {
                    if let Some(existing) = workers
                        .iter_mut()
                        .find(|existing| existing.session_id == worker.session_id)
                    {
                        existing.clone_from(worker);
                    } else {
                        workers.push((**worker).clone());
                    }
                }
                buffered.push(frame);
            }
            Ok(Ok(None)) => anyhow::bail!("agentd closed during runtime settle"),
            Ok(Err(error)) => return Err(error.into()),
            Err(_) if tokio::time::Instant::now() < minimum_settle => continue,
            Err(_) => break,
        }
    }
    Ok((reader, writer, workers, buffered))
}

async fn connection_manager(
    shared: Arc<Shared>,
    mut notify_rx: mpsc::UnboundedReceiver<()>,
    mut initial: Option<(OwnedReadHalf, OwnedWriteHalf, Vec<Frame>)>,
) {
    let mut backoff = Duration::from_millis(100);
    loop {
        if shared.shutdown.load(Ordering::Acquire) {
            return;
        }
        let connection =
            match initial.take() {
                Some(connection) => Ok(connection),
                None => connect_settled(&shared.socket).await.map(
                    |(reader, writer, workers, buffered)| {
                        merge_worker_snapshots(&shared, workers);
                        (reader, writer, buffered)
                    },
                ),
            };
        let (reader, writer, buffered) = match connection {
            Ok(connection) => connection,
            Err(error) => {
                shared.connected.store(false, Ordering::Release);
                tracing::warn!(error = %error, "agentd unavailable; Cowboy queues runtime commands");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(5));
                continue;
            }
        };
        backoff = Duration::from_millis(100);
        shared.connected.store(true, Ordering::Release);
        shared.sent.lock().clear();
        if let Err(error) = connected(&shared, reader, writer, buffered, &mut notify_rx).await {
            tracing::warn!(error = %error, "agentd connection dropped; reconnecting");
        }
        shared.connected.store(false, Ordering::Release);
        if shared.shutdown.load(Ordering::Acquire) {
            return;
        }
    }
}

async fn connected(
    shared: &Arc<Shared>,
    mut reader: OwnedReadHalf,
    mut writer: OwnedWriteHalf,
    buffered: Vec<Frame>,
    notify_rx: &mut mpsc::UnboundedReceiver<()>,
) -> Result<()> {
    write_frame(
        &mut writer,
        &Frame::CoreCommand {
            command: CoreCommand::SetDesiredGeneration {
                generation: shared.desired_generation.clone(),
                worker_command: shared.desired_worker_command.clone(),
            },
        },
    )
    .await?;
    send_declarations(shared, &mut writer).await?;
    let mut buffered = VecDeque::from(buffered);
    while let Some(frame) = buffered.pop_front() {
        handle_frame(shared, frame, &mut writer).await?;
    }
    send_pending(shared, &mut writer).await?;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    let mut last_broker_frame = tokio::time::Instant::now();
    loop {
        tokio::select! {
            incoming = read_frame(&mut reader) => {
                let Some(frame) = incoming? else { anyhow::bail!("agentd closed") };
                last_broker_frame = tokio::time::Instant::now();
                handle_frame(shared, frame, &mut writer).await?;
            }
            notified = notify_rx.recv() => {
                if shared.shutdown.load(Ordering::Acquire) {
                    return Ok(());
                }
                if notified.is_none() {
                    anyhow::bail!("runtime command channel closed");
                }
                send_pending(shared, &mut writer).await?;
            }
            _ = heartbeat.tick() => {
                if last_broker_frame.elapsed() > Duration::from_secs(35) {
                    anyhow::bail!("agentd heartbeat timed out");
                }
                write_frame(&mut writer, &Frame::Heartbeat).await?;
            }
        }
    }
}

async fn send_pending<W: tokio::io::AsyncWrite + Unpin>(
    shared: &Shared,
    writer: &mut W,
) -> Result<()> {
    let mut commands: Vec<_> = shared
        .pending
        .lock()
        .iter()
        .map(|(key, command)| (key.clone(), command.clone()))
        .collect();
    commands.sort_by_key(|(_, command)| {
        if matches!(command, CoreCommand::EnsureSession { .. }) {
            0
        } else {
            1
        }
    });
    for (key, command) in commands {
        write_frame(writer, &Frame::CoreCommand { command }).await?;
        shared.sent.lock().insert(key);
    }
    Ok(())
}

async fn send_declarations<W: tokio::io::AsyncWrite + Unpin>(
    shared: &Shared,
    writer: &mut W,
) -> Result<()> {
    let mut declarations: Vec<_> = shared.declarations.lock().values().cloned().collect();
    declarations.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    for mut session in declarations {
        session.adopt_only = true;
        write_frame(
            writer,
            &Frame::CoreCommand {
                command: CoreCommand::EnsureSession { session },
            },
        )
        .await?;
    }
    Ok(())
}

async fn handle_frame<W: tokio::io::AsyncWrite + Unpin>(
    shared: &Shared,
    frame: Frame,
    writer: &mut W,
) -> Result<()> {
    match frame {
        Frame::WorkerEvent {
            session_id,
            worker_epoch,
            runtime_seq,
            event,
        } => {
            let key = (session_id.clone(), worker_epoch.clone());
            let previous = shared.highwaters.lock().get(&key).copied();
            // After a Cowboy restart the worker has already discarded every
            // ACKed prefix, so the first replayed sequence may be greater than
            // one. Once this controller observes a sequence, gaps are errors.
            if let Some(previous) = previous {
                if runtime_seq > previous.saturating_add(1) {
                    anyhow::bail!(
                        "runtime event gap for {session_id}/{worker_epoch}: have {previous}, got {runtime_seq}"
                    );
                }
            }
            if previous.is_none_or(|previous| runtime_seq > previous) {
                update_snapshot_from_event(shared, &session_id, runtime_seq, &event);
                apply_event(&shared.hub, &session_id, event);
                shared.highwaters.lock().insert(key, runtime_seq);
            }
            write_frame(
                writer,
                &Frame::Ack {
                    session_id,
                    worker_epoch,
                    runtime_seq,
                },
            )
            .await?;
        }
        Frame::CommandAck {
            session_id,
            command_id,
            accepted,
            reason,
        } => {
            let command = shared.pending.lock().remove(&command_id);
            shared.sent.lock().remove(&command_id);
            if matches!(&command, Some(CoreCommand::StopSession { .. })) {
                shared.declarations.lock().remove(&session_id);
                shared.workers.lock().remove(&session_id);
            }
            if !accepted {
                handle_rejected_command(&shared.hub, &session_id, command, reason);
            }
        }
        Frame::Snapshot { worker } => {
            let session_id = worker.session_id.clone();
            update_declaration(shared, &worker);
            shared
                .workers
                .lock()
                .insert(session_id.clone(), (*worker).clone());
            shared
                .pending
                .lock()
                .remove(&format!("ensure:{session_id}"));
            shared.sent.lock().remove(&format!("ensure:{session_id}"));
            apply_snapshot(&shared.hub, &worker);
        }
        Frame::Welcome { workers, .. } => update_worker_snapshots(shared, workers),
        Frame::Heartbeat => {}
        Frame::Reject { reason } => anyhow::bail!("agentd rejected controller: {reason}"),
        other => tracing::debug!(?other, "ignoring unrelated agentd frame"),
    }
    Ok(())
}

fn update_worker_snapshots(shared: &Shared, workers: Vec<WorkerSnapshot>) {
    let mut current = HashMap::new();
    for worker in workers {
        update_declaration(shared, &worker);
        apply_snapshot(&shared.hub, &worker);
        shared
            .pending
            .lock()
            .remove(&format!("ensure:{}", worker.session_id));
        shared
            .sent
            .lock()
            .remove(&format!("ensure:{}", worker.session_id));
        current.insert(worker.session_id.clone(), worker);
    }
    *shared.workers.lock() = current;
}

fn merge_worker_snapshots(shared: &Shared, workers: Vec<WorkerSnapshot>) {
    let mut current = shared.workers.lock();
    for worker in workers {
        update_declaration(shared, &worker);
        apply_snapshot(&shared.hub, &worker);
        shared
            .pending
            .lock()
            .remove(&format!("ensure:{}", worker.session_id));
        shared
            .sent
            .lock()
            .remove(&format!("ensure:{}", worker.session_id));
        current.insert(worker.session_id.clone(), worker);
    }
}

fn update_declaration(shared: &Shared, worker: &WorkerSnapshot) {
    let Some(mut session) = worker.launch.clone() else {
        return;
    };
    session.adopt_only = true;
    shared
        .declarations
        .lock()
        .insert(session.session_id.clone(), session);
}

fn worker_status(state: WorkerState) -> Status {
    match state {
        WorkerState::Starting => Status::Starting,
        WorkerState::Running => Status::Running,
        WorkerState::Busy => Status::Busy,
        WorkerState::Draining => Status::Running,
        WorkerState::Exited => Status::Exited,
        WorkerState::Crashed => Status::Crashed,
    }
}

fn apply_snapshot(hub: &Hub, worker: &WorkerSnapshot) {
    if let Some(agent_session_id) = &worker.agent_session_id {
        hub.set_agent_session_id(&worker.session_id, agent_session_id.clone());
    }
    if let Some(options) = &worker.config_options {
        hub.set_config_options(&worker.session_id, options.clone());
    }
    if let (Some(used), Some(size)) = (worker.context_used, worker.context_size) {
        hub.set_session_usage(
            &worker.session_id,
            crate::agent_model::SessionUsage {
                used,
                size,
                raw: serde_json::Value::Null,
                observed_at_ms: 0,
            },
        );
    }
    hub.set_status(&worker.session_id, worker_status(worker.state), None);
}

fn update_snapshot_from_event(
    shared: &Shared,
    session_id: &str,
    runtime_seq: u64,
    event: &RuntimeEvent,
) {
    let mut workers = shared.workers.lock();
    let Some(worker) = workers.get_mut(session_id) else {
        return;
    };
    worker.last_runtime_seq = runtime_seq;
    match event {
        RuntimeEvent::Ready { agent_session_id } => {
            worker.state = WorkerState::Running;
            if agent_session_id.is_some() {
                worker.agent_session_id.clone_from(agent_session_id);
            }
        }
        RuntimeEvent::Status { state, .. } => worker.state = *state,
        RuntimeEvent::TurnStarted { turn_id, .. } => {
            worker.state = WorkerState::Busy;
            worker.current_turn_id = Some(turn_id.clone());
        }
        RuntimeEvent::TurnEnded { turn_id, .. } => {
            if worker.current_turn_id.as_deref() == Some(turn_id) {
                worker.current_turn_id = None;
            }
        }
        RuntimeEvent::AgentSessionId { agent_session_id } => {
            worker.agent_session_id = Some(agent_session_id.clone());
        }
        RuntimeEvent::PermissionRequest { request_id, .. } => {
            if !worker.pending_permissions.contains(request_id) {
                worker.pending_permissions.push(request_id.clone());
            }
        }
        RuntimeEvent::PermissionResolved { request_id, .. } => {
            worker.pending_permissions.retain(|id| id != request_id);
        }
        RuntimeEvent::ConfigOptions { options } => {
            worker.config_options = Some(options.clone());
        }
        RuntimeEvent::ContextUsage { used, size, .. } => {
            worker.context_used = Some(*used);
            worker.context_size = Some(*size);
        }
        RuntimeEvent::Update { .. }
        | RuntimeEvent::ScheduleWakeup { .. }
        | RuntimeEvent::UndeliveredPrompt { .. }
        | RuntimeEvent::CommandRejected { .. }
        | RuntimeEvent::Error { .. } => {}
    }
}

fn apply_event(hub: &Hub, session_id: &str, event: RuntimeEvent) {
    match event {
        RuntimeEvent::Ready { agent_session_id } => {
            if let Some(agent_session_id) = agent_session_id {
                hub.set_agent_session_id(session_id, agent_session_id);
            }
            hub.set_status(session_id, Status::Running, None);
        }
        RuntimeEvent::Status { state, detail } => {
            hub.set_status(session_id, worker_status(state), detail);
        }
        RuntimeEvent::Update { update, cmid } => {
            hub.push_tagged(session_id, Event::Update { update }, cmid);
        }
        RuntimeEvent::ConfigOptions { options } => hub.set_config_options(session_id, options),
        RuntimeEvent::ContextUsage {
            used,
            size,
            raw,
            observed_at_ms,
        } => hub.set_session_usage(
            session_id,
            crate::agent_model::SessionUsage {
                used,
                size,
                raw,
                observed_at_ms,
            },
        ),
        RuntimeEvent::PermissionRequest {
            request_id,
            tool_call,
            options,
        } => hub.push(
            session_id,
            Event::PermissionRequest {
                request_id,
                tool_call,
                options,
            },
        ),
        RuntimeEvent::PermissionResolved {
            request_id,
            option_id,
        } => hub.push(
            session_id,
            Event::PermissionResolved {
                request_id,
                option_id,
            },
        ),
        RuntimeEvent::TurnStarted { .. } => hub.set_status(session_id, Status::Busy, None),
        RuntimeEvent::TurnEnded { stop_reason, .. } => {
            hub.push(session_id, Event::TurnEnd { stop_reason });
        }
        RuntimeEvent::AgentSessionId { agent_session_id } => {
            hub.set_agent_session_id(session_id, agent_session_id);
        }
        RuntimeEvent::ScheduleWakeup {
            delay_seconds,
            prompt,
        } => hub.schedule_wakeup(session_id, delay_seconds, prompt),
        RuntimeEvent::UndeliveredPrompt {
            cmid,
            content,
            error,
            ..
        } => {
            let text = content
                .iter()
                .filter_map(|value| value.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            hub.requeue_prompt(session_id, text, content, cmid);
            hub.broadcast_error(Some(session_id.to_owned()), error);
        }
        RuntimeEvent::CommandRejected { message, .. } | RuntimeEvent::Error { message } => {
            hub.broadcast_error(Some(session_id.to_owned()), message);
        }
    }
}

fn handle_rejected_command(
    hub: &Hub,
    session_id: &str,
    command: Option<CoreCommand>,
    reason: Option<String>,
) {
    let reason = reason.unwrap_or_else(|| "runtime rejected command".to_owned());
    if let Some(CoreCommand::Prompt { content, cmid, .. }) = command {
        let text = content
            .iter()
            .filter_map(|value| value.get("text").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        hub.requeue_prompt(session_id, text, content, cmid);
    }
    hub.broadcast_error(Some(session_id.to_owned()), reason);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(session_id: &str) -> WorkerSnapshot {
        WorkerSnapshot {
            session_id: session_id.to_owned(),
            worker_epoch: "epoch-1".to_owned(),
            generation: "gen-1".to_owned(),
            executable: Some("/bin/false".to_owned()),
            launch: Some(StartSession {
                session_id: session_id.to_owned(),
                provider: "codex".to_owned(),
                cwd: "/tmp".to_owned(),
                agent_session_id: Some("agent-1".to_owned()),
                system: false,
                generation: "gen-1".to_owned(),
                fallback_for: None,
                adopt_only: false,
            }),
            state: WorkerState::Busy,
            agent_session_id: Some("agent-1".to_owned()),
            current_turn_id: Some("turn-1".to_owned()),
            last_runtime_seq: 4,
            pending_permissions: Vec::new(),
            config_options: None,
            context_used: None,
            context_size: None,
            pending_prompt_count: 0,
            drain_requested: false,
        }
    }

    #[tokio::test]
    async fn empty_broker_reconnect_does_not_forget_a_surviving_worker_claim() {
        let hub = Hub::new();
        let (left, _right) = UnixStream::pair().expect("socket pair");
        let (reader, writer) = left.into_split();
        let bootstrap = RemoteBootstrap {
            socket: PathBuf::from("/tmp/unused-agentd.sock"),
            reader,
            writer,
            workers: vec![snapshot("s")],
            buffered: Vec::new(),
        };
        let runtime = RemoteRuntime::new(
            hub,
            &bootstrap,
            "gen-2".to_owned(),
            Some("/bin/new-worker".to_owned()),
        );

        merge_worker_snapshots(&runtime.shared, Vec::new());

        assert!(runtime.has_worker("s"));
        assert!(runtime.shared.declarations.lock().contains_key("s"));
    }

    #[test]
    fn rejected_prompt_returns_to_durable_queue() {
        let hub = Hub::new();
        hub.create_session(
            "s".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            crate::core::SessionOrigin::Web,
            false,
        );
        handle_rejected_command(
            &hub,
            "s",
            Some(CoreCommand::Prompt {
                session_id: "s".to_owned(),
                command_id: "c".to_owned(),
                turn_id: "t".to_owned(),
                content: vec![serde_json::json!({"type": "text", "text": "keep me"})],
                cmid: Some("m".to_owned()),
            }),
            Some("failed".to_owned()),
        );
        assert_eq!(hub.session_info("s").expect("session").queue_count, 1);
    }

    #[tokio::test]
    async fn shutdown_returns_unacknowledged_prompt_to_queue() {
        let hub = Hub::new();
        hub.create_session(
            "s".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            crate::core::SessionOrigin::Web,
            false,
        );
        let (left, _right) = UnixStream::pair().expect("socket pair");
        let (reader, writer) = left.into_split();
        let bootstrap = RemoteBootstrap {
            socket: PathBuf::from("/tmp/unused-agentd.sock"),
            reader,
            writer,
            workers: Vec::new(),
            buffered: Vec::new(),
        };
        let runtime = RemoteRuntime::new(
            hub.clone(),
            &bootstrap,
            "gen-1".to_owned(),
            Some("/bin/false".to_owned()),
        );
        runtime.prompt(
            "s",
            vec![serde_json::json!({"type": "text", "text": "keep me"})],
            Some("cmid-1".to_owned()),
        );
        runtime.graceful_shutdown(Duration::ZERO).await;
        assert_eq!(hub.session_info("s").expect("session").queue_count, 1);
    }

    #[tokio::test]
    async fn shutdown_does_not_duplicate_prompt_already_owned_by_broker() {
        let hub = Hub::new();
        hub.create_session(
            "s".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            crate::core::SessionOrigin::Web,
            false,
        );
        let (left, _right) = UnixStream::pair().expect("socket pair");
        let (reader, writer) = left.into_split();
        let bootstrap = RemoteBootstrap {
            socket: PathBuf::from("/tmp/unused-agentd.sock"),
            reader,
            writer,
            workers: Vec::new(),
            buffered: Vec::new(),
        };
        let runtime = RemoteRuntime::new(
            hub.clone(),
            &bootstrap,
            "gen-1".to_owned(),
            Some("/bin/false".to_owned()),
        );
        let command_id = runtime.prompt(
            "s",
            vec![serde_json::json!({"type": "text", "text": "owned"})],
            Some("cmid-2".to_owned()),
        );
        runtime.shared.sent.lock().insert(command_id);
        runtime.shared.connected.store(true, Ordering::Release);
        runtime.graceful_shutdown(Duration::ZERO).await;
        assert_eq!(hub.session_info("s").expect("session").queue_count, 0);
    }
}
