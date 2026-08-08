//! Cowboy-side client for the stable Machine runtime broker.

#![warn(clippy::pedantic)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use parking_lot::Mutex;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;

use crate::core::{Event, Hub, Status};
use crate::runtime_wire::{
    CoreCommand, Frame, FrameReader, MIN_PROTOCOL_VERSION, PROTOCOL_VERSION, PeerRole,
    RuntimeEvent, StartSession, WorkerSnapshot, WorkerState, read_frame, write_frame,
};

pub struct RemoteBootstrap {
    socket: PathBuf,
    reader: FrameReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    workers: Vec<WorkerSnapshot>,
    buffered: Vec<Frame>,
}

impl RemoteBootstrap {
    /// Acquire the Machine broker controller lease through an authenticated
    /// transport, such as a Machine WebSocket tunnel.
    pub async fn from_stream(socket: PathBuf, stream: UnixStream) -> Result<Self> {
        let (reader, writer, workers, buffered) = connect_settled_stream(stream).await?;
        Ok(Self {
            socket,
            reader,
            writer,
            workers,
            buffered,
        })
    }
}

struct Shared {
    socket: PathBuf,
    hub: Hub,
    pending: Mutex<HashMap<String, CoreCommand>>,
    sent: Mutex<HashSet<String>>,
    /// Launch metadata re-declared after every broker reconnect. These are
    /// registry claims, not pending commands: absence never authorizes Machine broker
    /// to spawn while surviving workers are still converging.
    declarations: Mutex<HashMap<String, StartSession>>,
    workers: Mutex<HashMap<String, WorkerSnapshot>>,
    /// Worker epochs whose initial config snapshot has already triggered
    /// preference reconciliation. This prevents an agent that rejects a value
    /// from causing a config-option event/retry loop.
    config_sync_epochs: Mutex<HashMap<String, String>>,
    /// Sessions being atomically recycled. Old-worker events and snapshots are
    /// acknowledged but ignored until the reset-flavoured stop is accepted, so
    /// a late `Running` edge cannot drain a force-pushed prompt into the worker
    /// that is being fenced.
    resetting: Mutex<HashSet<String>>,
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
    /// readiness/fallback state. Machine broker updates wait for this to reach zero.
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
                config_sync_epochs: Mutex::new(HashMap::new()),
                resetting: Mutex::new(HashSet::new()),
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

    #[cfg(test)]
    pub(crate) fn for_test(hub: Hub, workers: Vec<WorkerSnapshot>) -> Arc<Self> {
        let (left, _right) = UnixStream::pair().expect("test runtime socket pair");
        let (reader, writer) = left.into_split();
        let bootstrap = RemoteBootstrap {
            socket: PathBuf::from("/tmp/unused-machine-broker.sock"),
            reader: FrameReader::new(reader),
            writer,
            workers,
            buffered: Vec::new(),
        };
        Self::new(
            hub,
            &bootstrap,
            "test-generation".to_owned(),
            Some("/bin/false".to_owned()),
        )
    }

    #[cfg(test)]
    pub(crate) fn pending_for_test(&self) -> Vec<CoreCommand> {
        self.shared.pending.lock().values().cloned().collect()
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
                apply_snapshot(&shared, worker);
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
                worker.has_connected_owner()
                    && !matches!(worker.state, WorkerState::Exited | WorkerState::Crashed)
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
        let session_id = session.session_id.clone();
        let key = format!("ensure:{session_id}");
        self.queue(key, CoreCommand::EnsureSession { session });
        // Queue session-owned preferences before any prompt. The worker may not
        // have advertised its options yet, so this intentionally queues the
        // scalar values without validation; the ACP response remains the
        // authority and will refresh the display snapshot.
        queue_persisted_config_for_session(&self.shared, &session_id, false);
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

    pub fn set_config_option(&self, session_id: &str, config_id: &str, value: serde_json::Value) {
        queue_config_value(&self.shared, session_id, config_id, value);
    }

    pub fn stop(&self, session_id: &str) {
        self.shared.declarations.lock().remove(session_id);
        self.shared.config_sync_epochs.lock().remove(session_id);
        let command_id = self.next_id("stop");
        self.queue(
            command_id.clone(),
            CoreCommand::StopSession {
                session_id: session_id.to_owned(),
                command_id,
            },
        );
    }

    pub fn reset(&self, mut session: StartSession) {
        // The declaration must reach Machine before the reset-flavoured stop,
        // but it must not start a replacement of its own. The reset command is
        // the single owner of fencing the old unit and launching the new one.
        // Without `adopt_only`, EnsureSession can briefly spawn a worker that
        // reset_session immediately stops again.
        session.adopt_only = true;
        if session.generation.is_empty() {
            session
                .generation
                .clone_from(&self.shared.desired_generation);
        }
        let session_id = session.session_id.clone();
        self.shared.resetting.lock().insert(session_id.clone());
        self.shared.config_sync_epochs.lock().remove(&session_id);
        self.shared
            .declarations
            .lock()
            .insert(session_id.clone(), session.clone());
        // Re-declare the replacement metadata before the reset-flavoured stop.
        // `send_pending` orders this adopt-only EnsureSession first, so Machine
        // can atomically fence and relaunch from the fresh specification while
        // preserving the existing v1 wire contract.
        let ensure_key = format!("ensure:{session_id}");
        self.queue(ensure_key, CoreCommand::EnsureSession { session });
        // A reset creates a fresh ACP process, so do not use the old worker's
        // advertised values to decide whether these preferences are needed.
        queue_persisted_config_for_session(&self.shared, &session_id, true);
        let command_id = self.next_id("reset");
        self.queue(
            command_id.clone(),
            CoreCommand::StopSession {
                session_id,
                command_id,
            },
        );
    }

    /// Stop accepting a deployment boundary only after every command already
    /// handed off by the Hub has been acknowledged by its worker. If the
    /// runtime stays unavailable through the deadline, return prompts to the
    /// durable Hub queue before the Postgres writer is closed.
    #[cfg(test)]
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

    /// Fence a disconnected Machine runtime without entering the local socket
    /// reconnect loop. Durable prompts remain owned by the Hub.
    pub fn disconnect(&self) {
        self.shared.shutdown.store(true, Ordering::Release);
        self.shared.connected.store(false, Ordering::Release);
        let _ = self.shared.notify.send(());
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
) -> Result<(
    FrameReader<OwnedReadHalf>,
    OwnedWriteHalf,
    Vec<WorkerSnapshot>,
)> {
    let stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connecting Machine broker socket {}", socket.display()))?;
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
        Some(Frame::Welcome { workers, .. }) => Ok((FrameReader::new(reader), writer, workers)),
        Some(Frame::Reject { reason }) => anyhow::bail!("Machine broker rejected Cowboy: {reason}"),
        Some(other) => anyhow::bail!("unexpected Machine broker handshake frame: {other:?}"),
        None => anyhow::bail!("Machine broker closed during handshake"),
    }
}

async fn connect_once_stream(
    stream: UnixStream,
) -> Result<(
    FrameReader<OwnedReadHalf>,
    OwnedWriteHalf,
    Vec<WorkerSnapshot>,
)> {
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
        Some(Frame::Welcome { workers, .. }) => Ok((FrameReader::new(reader), writer, workers)),
        Some(Frame::Reject { reason }) => anyhow::bail!("Machine broker rejected Cowboy: {reason}"),
        Some(other) => anyhow::bail!("unexpected Machine broker handshake frame: {other:?}"),
        None => anyhow::bail!("Machine broker closed during handshake"),
    }
}

/// Let surviving workers converge around every new broker connection, not only
/// process startup. Until this settles, `Shared.workers` intentionally keeps its
/// last snapshot so a Machine broker restart cannot race an `EnsureSession` into
/// spawning a duplicate owner for a still-running turn.
async fn connect_settled(
    socket: &Path,
) -> Result<(
    FrameReader<OwnedReadHalf>,
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
        match tokio::time::timeout(quiet, reader.next()).await {
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
            Ok(Ok(None)) => anyhow::bail!("Machine broker closed during runtime settle"),
            Ok(Err(error)) => return Err(error.into()),
            Err(_) if tokio::time::Instant::now() < minimum_settle => {}
            Err(_) => break,
        }
    }
    Ok((reader, writer, workers, buffered))
}

async fn connect_settled_stream(
    stream: UnixStream,
) -> Result<(
    FrameReader<OwnedReadHalf>,
    OwnedWriteHalf,
    Vec<WorkerSnapshot>,
    Vec<Frame>,
)> {
    let (mut reader, writer, mut workers) = connect_once_stream(stream).await?;
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
        match tokio::time::timeout(quiet, reader.next()).await {
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
            Ok(Ok(None)) => anyhow::bail!("Machine broker closed during runtime settle"),
            Ok(Err(error)) => return Err(error.into()),
            Err(_) if tokio::time::Instant::now() < minimum_settle => {}
            Err(_) => break,
        }
    }
    Ok((reader, writer, workers, buffered))
}

async fn connection_manager(
    shared: Arc<Shared>,
    mut notify_rx: mpsc::UnboundedReceiver<()>,
    mut initial: Option<(FrameReader<OwnedReadHalf>, OwnedWriteHalf, Vec<Frame>)>,
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
                tracing::warn!(error = %error, "Machine broker unavailable; Cowboy queues runtime commands");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(5));
                continue;
            }
        };
        backoff = Duration::from_millis(100);
        shared.connected.store(true, Ordering::Release);
        shared.sent.lock().clear();
        if let Err(error) = connected(&shared, reader, writer, buffered, &mut notify_rx).await {
            tracing::warn!(error = %error, "Machine broker connection dropped; reconnecting");
        }
        shared.connected.store(false, Ordering::Release);
        if shared.shutdown.load(Ordering::Acquire) {
            return;
        }
    }
}

async fn connected(
    shared: &Arc<Shared>,
    mut reader: FrameReader<OwnedReadHalf>,
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
            incoming = reader.next() => {
                let Some(frame) = incoming? else { anyhow::bail!("Machine broker closed") };
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
                    anyhow::bail!("Machine broker heartbeat timed out");
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
    commands.sort_by_key(|(_, command)| command_priority(command));
    for (key, command) in commands {
        write_frame(writer, &Frame::CoreCommand { command }).await?;
        shared.sent.lock().insert(key);
    }
    Ok(())
}

fn command_priority(command: &CoreCommand) -> (u8, u8, String) {
    match command {
        CoreCommand::EnsureSession { .. } => (0, 0, String::new()),
        CoreCommand::StopSession { command_id, .. } if command_id.starts_with("reset-") => {
            (1, 0, String::new())
        }
        CoreCommand::SetConfigOption { config_id, .. } => {
            let config_rank = match config_id.as_str() {
                // Changing the model can reset the provider's reasoning
                // choice, so replay it before reasoning_effort.
                "model" => 0,
                "reasoning_effort" => 1,
                _ => 2,
            };
            (2, config_rank, config_id.clone())
        }
        // Machine broker handles a reset stop synchronously. Prompts sent after it are
        // queued for the replacement worker instead of reaching the old worker
        // and being cleared by reset_session.
        _ => (3, 0, String::new()),
    }
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

#[allow(clippy::too_many_lines)] // One exhaustive wire-frame dispatch keeps broker ordering visible.
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
            if let Some(previous) = previous
                && runtime_seq > previous.saturating_add(1)
            {
                anyhow::bail!(
                    "runtime event gap for {session_id}/{worker_epoch}: have {previous}, got {runtime_seq}"
                );
            }
            if previous.is_none_or(|previous| runtime_seq > previous) {
                let resetting = shared.resetting.lock().contains(&session_id);
                let stale_idle = matches!(
                    event,
                    RuntimeEvent::Status {
                        state: WorkerState::Running | WorkerState::Draining,
                        ..
                    }
                ) && shared
                    .workers
                    .lock()
                    .get(&session_id)
                    .is_some_and(|worker| worker.current_turn_id.is_some());
                if stale_idle {
                    tracing::warn!(
                        session = %session_id,
                        runtime_seq,
                        "ignoring stale idle status while a newer turn is active"
                    );
                } else if !resetting {
                    let auto_permission = codex_full_access_permission(shared, &session_id, &event);
                    let is_config_options = matches!(&event, RuntimeEvent::ConfigOptions { .. });
                    update_snapshot_from_event(shared, &session_id, runtime_seq, &event);
                    if is_config_options
                        && let Some(worker) = shared.workers.lock().get(&session_id).cloned()
                    {
                        sync_config_for_worker(shared, &worker);
                    }
                    if let Some((request_id, option_id)) = auto_permission {
                        tracing::info!(
                            session = %session_id,
                            %request_id,
                            %option_id,
                            "auto-approving Codex full-access permission at runtime boundary"
                        );
                        queue_permission(shared, &session_id, request_id, option_id);
                    } else {
                        apply_event(&shared.hub, &session_id, event);
                    }
                }
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
            let reset_stop = matches!(
                &command,
                Some(CoreCommand::StopSession { command_id, .. })
                    if command_id.starts_with("reset-")
            );
            if let Some(CoreCommand::StopSession { command_id, .. }) = &command {
                if !command_id.starts_with("reset-") {
                    shared.declarations.lock().remove(&session_id);
                }
                shared.workers.lock().remove(&session_id);
            }
            if reset_stop {
                shared.resetting.lock().remove(&session_id);
                shared.hub.set_status(
                    &session_id,
                    if accepted {
                        Status::Starting
                    } else {
                        Status::Crashed
                    },
                    None,
                );
            }
            if !accepted {
                if reason.as_deref().is_some_and(|message| {
                    message.starts_with("session workspace was replaced or removed:")
                }) {
                    reset_after_workspace_replacement(shared, &session_id);
                }
                handle_rejected_command(&shared.hub, &session_id, command, reason);
            }
        }
        Frame::Snapshot { worker } => {
            let session_id = worker.session_id.clone();
            if shared.resetting.lock().contains(&session_id) {
                return Ok(());
            }
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
            if apply_snapshot(shared, &worker) {
                reconcile_idle_snapshot(shared, &worker);
            }
        }
        Frame::Welcome { workers, .. } => update_worker_snapshots(shared, workers),
        Frame::Heartbeat => {}
        Frame::Reject { reason } => anyhow::bail!("Machine broker rejected controller: {reason}"),
        other => tracing::debug!(?other, "ignoring unrelated Machine broker frame"),
    }
    Ok(())
}

fn codex_full_access_permission(
    shared: &Shared,
    session_id: &str,
    event: &RuntimeEvent,
) -> Option<(String, String)> {
    let RuntimeEvent::PermissionRequest {
        request_id,
        options,
        ..
    } = event
    else {
        return None;
    };
    let workers = shared.workers.lock();
    let worker = workers.get(session_id)?;
    if !crate::provider::is_codex(&worker.launch.as_ref()?.provider)
        || !worker
            .config_options
            .as_ref()
            .is_some_and(codex_config_is_full_access)
    {
        return None;
    }
    let option_id = options.as_array()?.iter().find_map(|option| {
        (option.get("kind").and_then(serde_json::Value::as_str) == Some("allow_always"))
            .then(|| option.get("optionId").and_then(serde_json::Value::as_str))
            .flatten()
            .map(str::to_owned)
    })?;
    Some((request_id.clone(), option_id))
}

fn codex_config_is_full_access(options: &serde_json::Value) -> bool {
    options.as_array().is_some_and(|options| {
        options.iter().any(|option| {
            option.get("id").and_then(serde_json::Value::as_str) == Some("mode")
                && option
                    .get("currentValue")
                    .or_else(|| option.get("current_value"))
                    .and_then(serde_json::Value::as_str)
                    == Some("agent-full-access")
        })
    })
}

fn queue_permission(shared: &Shared, session_id: &str, request_id: String, option_id: String) {
    let value = shared.command_counter.fetch_add(1, Ordering::Relaxed);
    let command_id = format!("permission-{}-{value}", std::process::id());
    shared.sent.lock().remove(&command_id);
    shared.pending.lock().insert(
        command_id.clone(),
        CoreCommand::Permission {
            session_id: session_id.to_owned(),
            command_id,
            request_id,
            option_id: Some(option_id),
        },
    );
    let _ = shared.notify.send(());
}

fn update_worker_snapshots(shared: &Shared, workers: Vec<WorkerSnapshot>) {
    let mut current = HashMap::new();
    for worker in workers {
        if shared.resetting.lock().contains(&worker.session_id) {
            continue;
        }
        update_declaration(shared, &worker);
        if apply_snapshot(shared, &worker) {
            reconcile_idle_snapshot(shared, &worker);
        }
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
        if shared.resetting.lock().contains(&worker.session_id) {
            continue;
        }
        update_declaration(shared, &worker);
        if apply_snapshot(shared, &worker) {
            reconcile_idle_snapshot(shared, &worker);
        }
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
        WorkerState::Running | WorkerState::Draining => Status::Running,
        WorkerState::Busy => Status::Busy,
        WorkerState::Exited => Status::Exited,
        WorkerState::Crashed => Status::Crashed,
    }
}

fn queue_persisted_config_for_session(shared: &Shared, session_id: &str, force: bool) {
    let options = if force {
        None
    } else {
        shared
            .workers
            .lock()
            .get(session_id)
            .and_then(|worker| worker.config_options.clone())
    };
    queue_persisted_config(shared, session_id, options.as_ref());
}

fn queue_persisted_config(shared: &Shared, session_id: &str, options: Option<&serde_json::Value>) {
    let Some(preferences) = shared.hub.config_preferences(session_id) else {
        return;
    };
    let Some(preferences) = preferences.as_object() else {
        return;
    };
    let Some(options) = options else {
        for (config_id, value) in preferences {
            if config_id == crate::deepseek_context::CONFIG_ID
                || config_id == crate::deepseek_cache::CONFIG_ID
            {
                continue;
            }
            queue_config_value(shared, session_id, config_id, value.clone());
        }
        return;
    };
    let Some(options) = options.as_array() else {
        return;
    };
    for (config_id, value) in preferences {
        if config_id == crate::deepseek_context::CONFIG_ID
            || config_id == crate::deepseek_cache::CONFIG_ID
        {
            continue;
        }
        let Some(option) = options
            .iter()
            .find(|option| option.get("id").and_then(serde_json::Value::as_str) == Some(config_id))
        else {
            continue;
        };
        if config_current_value(option) == Some(value) || !config_option_accepts(option, value) {
            continue;
        }
        queue_config_value(shared, session_id, config_id, value.clone());
    }
}

fn sync_config_for_worker(shared: &Shared, worker: &WorkerSnapshot) {
    if !worker.has_connected_owner() || worker.config_options.is_none() {
        return;
    }
    let already_synced = {
        let mut epochs = shared.config_sync_epochs.lock();
        if epochs
            .get(&worker.session_id)
            .is_some_and(|epoch| epoch == &worker.worker_epoch)
        {
            true
        } else {
            epochs.insert(worker.session_id.clone(), worker.worker_epoch.clone());
            false
        }
    };
    if already_synced {
        return;
    }
    queue_persisted_config(shared, &worker.session_id, worker.config_options.as_ref());
}

fn queue_config_value(
    shared: &Shared,
    session_id: &str,
    config_id: &str,
    value: serde_json::Value,
) {
    let same_pending = shared.pending.lock().iter().any(|(_, command)| {
        matches!(
            command,
            CoreCommand::SetConfigOption {
                session_id: pending_session,
                config_id: pending_id,
                value: pending_value,
                ..
            } if pending_session == session_id
                && pending_id == config_id
                && pending_value == &value
        )
    });
    if same_pending {
        return;
    }
    let stale_keys: Vec<String> = shared
        .pending
        .lock()
        .iter()
        .filter_map(|(key, command)| {
            matches!(
                command,
                CoreCommand::SetConfigOption {
                    session_id: pending_session,
                    config_id: pending_id,
                    ..
                } if pending_session == session_id && pending_id == config_id
            )
            .then_some(key.clone())
        })
        .collect();
    for key in stale_keys {
        shared.pending.lock().remove(&key);
        shared.sent.lock().remove(&key);
    }
    let counter = shared.command_counter.fetch_add(1, Ordering::Relaxed);
    let command_id = format!("config-{}-{counter}", std::process::id());
    shared.pending.lock().insert(
        command_id.clone(),
        CoreCommand::SetConfigOption {
            session_id: session_id.to_owned(),
            command_id,
            config_id: config_id.to_owned(),
            value,
        },
    );
    let _ = shared.notify.send(());
}

fn config_current_value(option: &serde_json::Value) -> Option<&serde_json::Value> {
    option
        .get("currentValue")
        .or_else(|| option.get("current_value"))
}

fn config_option_accepts(option: &serde_json::Value, value: &serde_json::Value) -> bool {
    option
        .get("options")
        .is_none_or(|choices| config_value_list_contains(choices, value))
}

fn config_value_list_contains(options: &serde_json::Value, value: &serde_json::Value) -> bool {
    match options {
        serde_json::Value::Array(options) => options
            .iter()
            .any(|option| config_value_list_contains(option, value)),
        serde_json::Value::Object(option) => {
            option.get("value") == Some(value)
                || option
                    .get("options")
                    .is_some_and(|nested| config_value_list_contains(nested, value))
        }
        _ => options == value,
    }
}

fn apply_snapshot(shared: &Shared, worker: &WorkerSnapshot) -> bool {
    if !shared.hub.accept_runtime_snapshot(worker) {
        return false;
    }
    if let Some(agent_session_id) = &worker.agent_session_id {
        shared
            .hub
            .set_agent_session_id(&worker.session_id, agent_session_id.clone());
    }
    if let Some(options) = &worker.config_options {
        shared
            .hub
            .set_config_options(&worker.session_id, options.clone());
        sync_config_for_worker(shared, worker);
    }
    if let (Some(used), Some(size)) = (worker.context_used, worker.context_size) {
        shared.hub.set_session_usage(
            &worker.session_id,
            crate::agent_model::SessionUsage {
                used,
                size,
                raw: serde_json::Value::Null,
                observed_at_ms: 0,
            },
        );
    }
    let busy = worker.current_turn_id.is_some()
        && matches!(worker.state, WorkerState::Running | WorkerState::Draining);
    let recoverable_detail = (!busy)
        .then(|| shared.hub.latest_crash_detail(&worker.session_id))
        .flatten()
        .filter(|detail| {
            recoverable_live_worker_error(
                &shared.hub,
                &worker.session_id,
                worker.state,
                Some(detail.as_str()),
            )
        });
    let status = if busy {
        Status::Busy
    } else if recoverable_detail.is_some() {
        // A status-only worker snapshot cannot replay an already-acked detail.
        // Preserve the controller's durable recoverable-turn hold while the
        // same live worker correctly reports itself idle and reusable.
        Status::Crashed
    } else {
        worker_status(worker.state)
    };
    shared
        .hub
        .set_status(&worker.session_id, status, recoverable_detail);
    true
}

fn recoverable_live_worker_error(
    hub: &Hub,
    session_id: &str,
    state: WorkerState,
    detail: Option<&str>,
) -> bool {
    matches!(state, WorkerState::Running | WorkerState::Draining)
        && hub.session_info(session_id).is_some_and(|session| {
            detail.is_some_and(|detail| {
                crate::provider::claude_code::keeps_worker_alive(&session.meta.provider, detail)
            })
        })
}

fn pending_prompt_for(shared: &Shared, session_id: &str) -> bool {
    shared.pending.lock().values().any(|command| {
        matches!(
            command,
            CoreCommand::Prompt {
                session_id: pending_session,
                ..
            } if pending_session == session_id
        )
    })
}

fn reconcile_idle_snapshot(shared: &Shared, worker: &WorkerSnapshot) {
    let idle = worker.current_turn_id.is_none()
        && worker.pending_prompt_count == 0
        && matches!(worker.state, WorkerState::Running | WorkerState::Draining);
    if idle && !pending_prompt_for(shared, &worker.session_id) {
        shared.hub.reconcile_runtime_idle(&worker.session_id);
    }
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
            // claude-agent-acp survives turn-scoped provider failures and
            // reports Running so Machine retains the live worker. Keep the
            // controller session errored to hold queued prompts and expose Retry;
            // Supervisor recognizes the same detail and reuses this worker for
            // /compact or the next explicit prompt instead of recycling it.
            let status = if recoverable_live_worker_error(hub, session_id, state, detail.as_deref())
            {
                Status::Crashed
            } else {
                worker_status(state)
            };
            hub.set_status(session_id, status, detail);
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

fn reset_after_workspace_replacement(shared: &Shared, session_id: &str) {
    let Some(mut session) = shared
        .declarations
        .lock()
        .get(session_id)
        .cloned()
        .or_else(|| {
            shared
                .workers
                .lock()
                .get(session_id)
                .and_then(|worker| worker.launch.clone())
        })
    else {
        shared.hub.set_status(
            session_id,
            Status::Crashed,
            Some("workspace changed but no replacement launch metadata is available".to_owned()),
        );
        return;
    };
    session.adopt_only = false;
    if session.generation.is_empty() {
        session.generation.clone_from(&shared.desired_generation);
    }
    shared.resetting.lock().insert(session_id.to_owned());
    shared.config_sync_epochs.lock().remove(session_id);
    shared
        .declarations
        .lock()
        .insert(session_id.to_owned(), session.clone());
    let ensure_key = format!("ensure:{session_id}");
    shared.sent.lock().remove(&ensure_key);
    shared
        .pending
        .lock()
        .insert(ensure_key, CoreCommand::EnsureSession { session });
    queue_persisted_config_for_session(shared, session_id, true);
    let value = shared.command_counter.fetch_add(1, Ordering::Relaxed);
    let command_id = format!("reset-{}-{value}", std::process::id());
    shared.pending.lock().insert(
        command_id.clone(),
        CoreCommand::StopSession {
            session_id: session_id.to_owned(),
            command_id,
        },
    );
    shared.hub.set_status(session_id, Status::Starting, None);
    let _ = shared.notify.send(());
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
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
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

    #[test]
    fn codex_full_access_config_is_detected_without_weakening_other_modes() {
        assert!(codex_config_is_full_access(&serde_json::json!([{
            "id": "mode",
            "currentValue": "agent-full-access"
        }])));
        assert!(!codex_config_is_full_access(&serde_json::json!([{
            "id": "mode",
            "currentValue": "agent"
        }])));
        assert!(!codex_config_is_full_access(&serde_json::json!([])));
    }

    #[tokio::test]
    async fn context_rejection_keeps_all_claude_workers_live_but_sessions_errored() {
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "claude-deepseek".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            crate::core::SessionOrigin::Web,
            false,
        );
        let detail = "API Error: 400 This model's maximum context length is 1048576 tokens. However, you requested 1048875 tokens";

        apply_event(
            &hub,
            "s",
            RuntimeEvent::Status {
                state: WorkerState::Running,
                detail: Some(detail.to_owned()),
            },
        );

        assert_eq!(hub.status("s"), Some(Status::Crashed));
        assert_eq!(hub.latest_crash_detail("s").as_deref(), Some(detail));

        hub.create_local_session(
            "ordinary".to_owned(),
            "claude-code".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            crate::core::SessionOrigin::Web,
            false,
        );
        apply_event(
            &hub,
            "ordinary",
            RuntimeEvent::Status {
                state: WorkerState::Running,
                detail: Some(detail.to_owned()),
            },
        );
        assert_eq!(hub.status("ordinary"), Some(Status::Crashed));
        assert_eq!(hub.latest_crash_detail("ordinary").as_deref(), Some(detail));

        let empty_stream =
            "API Error: Stream ended without receiving any events {\"errorKind\":\"unknown\"}";
        apply_event(
            &hub,
            "ordinary",
            RuntimeEvent::Status {
                state: WorkerState::Running,
                detail: Some(empty_stream.to_owned()),
            },
        );
        assert_eq!(hub.status("ordinary"), Some(Status::Crashed));
        assert_eq!(
            hub.latest_crash_detail("ordinary").as_deref(),
            Some(empty_stream)
        );

        let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
        let mut reconnected = snapshot("ordinary");
        reconnected.state = WorkerState::Running;
        reconnected.current_turn_id = None;
        reconnected.launch.as_mut().expect("launch").provider = "claude-code".to_owned();
        assert!(apply_snapshot(&runtime.shared, &reconnected));
        reconcile_idle_snapshot(&runtime.shared, &reconnected);
        assert_eq!(hub.status("ordinary"), Some(Status::Crashed));
        assert_eq!(
            hub.latest_crash_detail("ordinary").as_deref(),
            Some(empty_stream)
        );
    }

    #[test]
    fn reset_stop_is_always_ordered_before_prompt() {
        let stop = CoreCommand::StopSession {
            session_id: "s".to_owned(),
            command_id: "reset-1".to_owned(),
        };
        let prompt = CoreCommand::Prompt {
            session_id: "s".to_owned(),
            command_id: "prompt-1".to_owned(),
            turn_id: "turn-1".to_owned(),
            content: vec![serde_json::json!({"type": "text", "text": "continue"})],
            cmid: Some("retry-1".to_owned()),
        };
        for mut commands in [
            vec![prompt.clone(), stop.clone()],
            vec![stop.clone(), prompt.clone()],
        ] {
            commands.sort_by_key(command_priority);
            assert!(matches!(
                commands.as_slice(),
                [CoreCommand::StopSession { command_id, .. }, CoreCommand::Prompt { .. }]
                    if command_id.starts_with("reset-")
            ));
        }
    }

    #[tokio::test]
    async fn session_preferences_are_queued_before_the_first_prompt() {
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            crate::core::SessionOrigin::Web,
            false,
        );
        let runtime = RemoteRuntime::for_test(hub, Vec::new());
        runtime.ensure(snapshot("s").launch.expect("launch metadata"));
        runtime.prompt(
            "s",
            vec![serde_json::json!({"type": "text", "text": "hello"})],
            None,
        );

        let mut commands = runtime.pending_for_test();
        commands.sort_by_key(command_priority);
        let ensure_index = commands
            .iter()
            .position(|command| matches!(command, CoreCommand::EnsureSession { .. }))
            .expect("ensure command");
        let prompt_index = commands
            .iter()
            .position(|command| matches!(command, CoreCommand::Prompt { .. }))
            .expect("prompt command");
        let config_commands: Vec<_> = commands
            .iter()
            .filter_map(|command| match command {
                CoreCommand::SetConfigOption {
                    config_id, value, ..
                } => Some((config_id.as_str(), value)),
                _ => None,
            })
            .collect();

        assert!(ensure_index < prompt_index);
        assert_eq!(config_commands.len(), 2);
        assert_eq!(config_commands[0].0, "model");
        assert_eq!(config_commands[1].0, "reasoning_effort");
        assert!(
            config_commands
                .iter()
                .any(|(id, value)| *id == "model" && **value == serde_json::json!("gpt-5.6-luna"))
        );
        assert!(
            config_commands
                .iter()
                .any(|(id, value)| *id == "reasoning_effort" && **value == serde_json::json!("max"))
        );
        assert!(
            commands[..prompt_index]
                .iter()
                .any(|command| matches!(command, CoreCommand::SetConfigOption { .. }))
        );
    }

    #[tokio::test]
    async fn host_owned_deepseek_settings_are_never_forwarded_as_acp_options() {
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex-deepseek".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            crate::core::SessionOrigin::Web,
            false,
        );
        let runtime = RemoteRuntime::for_test(hub, Vec::new());
        let mut launch = snapshot("s").launch.expect("launch metadata");
        launch.provider = "codex-deepseek".to_owned();
        launch.context_window = Some(680_000);
        launch.auto_compact_token_limit = Some(646_000);
        runtime.ensure(launch);

        let config_ids = runtime
            .pending_for_test()
            .into_iter()
            .filter_map(|command| match command {
                CoreCommand::SetConfigOption { config_id, .. } => Some(config_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(config_ids.len(), 3);
        assert!(!config_ids.iter().any(|id| id == "deepseek_context"));
        assert!(
            !config_ids
                .iter()
                .any(|id| id == "deepseek_cache_protection")
        );
    }

    #[tokio::test]
    async fn empty_broker_reconnect_does_not_forget_a_surviving_worker_claim() {
        let hub = Hub::new();
        let (left, _right) = UnixStream::pair().expect("socket pair");
        let (reader, writer) = left.into_split();
        let bootstrap = RemoteBootstrap {
            socket: PathBuf::from("/tmp/unused-machine-broker.sock"),
            reader: FrameReader::new(reader),
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
        hub.create_local_session(
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
    async fn reset_uses_existing_wire_and_preserves_replacement_declaration() {
        let hub = Hub::new();
        hub.create_local_session(
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
            socket: PathBuf::from("/tmp/unused-machine-broker.sock"),
            reader: FrameReader::new(reader),
            writer,
            workers: vec![snapshot("s")],
            buffered: Vec::new(),
        };
        let runtime = RemoteRuntime::new(
            hub,
            &bootstrap,
            "gen-1".to_owned(),
            Some("/bin/worker".to_owned()),
        );
        let mut replacement = snapshot("s").launch.expect("launch metadata");
        replacement.agent_session_id = None;

        runtime.reset(replacement);
        runtime.shared.hub.set_status("s", Status::Starting, None);
        assert!(runtime.shared.resetting.lock().contains("s"));

        handle_frame(
            &runtime.shared,
            Frame::WorkerEvent {
                session_id: "s".to_owned(),
                worker_epoch: "old-worker".to_owned(),
                runtime_seq: 1,
                event: RuntimeEvent::Status {
                    state: WorkerState::Busy,
                    detail: None,
                },
            },
            &mut tokio::io::sink(),
        )
        .await
        .expect("late old-worker event");
        let mut late_snapshot = snapshot("s");
        late_snapshot.state = WorkerState::Busy;
        handle_frame(
            &runtime.shared,
            Frame::Snapshot {
                worker: Box::new(late_snapshot),
            },
            &mut tokio::io::sink(),
        )
        .await
        .expect("late old-worker snapshot");
        assert_eq!(runtime.shared.hub.status("s"), Some(Status::Starting));

        let reset_command_id = {
            let pending = runtime.shared.pending.lock();
            assert!(pending
                .values()
                .any(|command| matches!(command, CoreCommand::EnsureSession { session } if session.agent_session_id.is_none() && session.adopt_only)));
            assert!(pending.values().any(
                |command| matches!(command, CoreCommand::StopSession { command_id, .. } if command_id.starts_with("reset-"))
            ));
            pending
                .values()
                .find_map(|command| match command {
                    CoreCommand::StopSession { command_id, .. }
                        if command_id.starts_with("reset-") =>
                    {
                        Some(command_id.clone())
                    }
                    _ => None,
                })
                .expect("reset command")
        };
        assert!(
            runtime
                .shared
                .declarations
                .lock()
                .get("s")
                .is_some_and(|session| session.agent_session_id.is_none())
        );
        handle_frame(
            &runtime.shared,
            Frame::CommandAck {
                session_id: "s".to_owned(),
                command_id: reset_command_id,
                accepted: true,
                reason: None,
            },
            &mut tokio::io::sink(),
        )
        .await
        .expect("reset acknowledgement");
        assert!(!runtime.shared.resetting.lock().contains("s"));
        assert_eq!(runtime.shared.hub.status("s"), Some(Status::Starting));
    }

    #[tokio::test]
    async fn idle_snapshot_waits_for_controller_pending_prompt_before_reconciliation() {
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            crate::core::SessionOrigin::Web,
            false,
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("s", Status::Running, None);
        hub.submit("s", "first".to_owned(), vec![], None);
        assert_eq!(rx.recv().await.expect("first dispatch").text, "first");
        hub.submit("s", "second".to_owned(), vec![], None);

        let runtime = RemoteRuntime::for_test(hub, vec![snapshot("s")]);
        runtime.shared.pending.lock().insert(
            "prompt-in-transit".to_owned(),
            CoreCommand::Prompt {
                session_id: "s".to_owned(),
                command_id: "prompt-in-transit".to_owned(),
                turn_id: "turn-in-transit".to_owned(),
                content: vec![serde_json::json!({"type": "text", "text": "first"})],
                cmid: None,
            },
        );
        let mut idle = snapshot("s");
        idle.state = WorkerState::Running;
        idle.current_turn_id = None;

        reconcile_idle_snapshot(&runtime.shared, &idle);
        assert!(
            rx.try_recv().is_err(),
            "pending prompt must retain the guard"
        );

        runtime.shared.pending.lock().clear();
        reconcile_idle_snapshot(&runtime.shared, &idle);
        assert_eq!(
            rx.recv().await.expect("dispatch after reconciliation").text,
            "second"
        );
    }

    #[tokio::test]
    async fn idle_snapshot_waits_for_broker_pending_prompt_before_reconciliation() {
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            crate::core::SessionOrigin::Web,
            false,
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("s", Status::Running, None);
        hub.submit("s", "first".to_owned(), vec![], None);
        assert_eq!(rx.recv().await.expect("first dispatch").text, "first");
        hub.submit("s", "second".to_owned(), vec![], None);

        let runtime = RemoteRuntime::for_test(hub, vec![snapshot("s")]);
        let mut snapshot_with_pending = snapshot("s");
        snapshot_with_pending.state = WorkerState::Running;
        snapshot_with_pending.current_turn_id = None;
        snapshot_with_pending.pending_prompt_count = 1;

        reconcile_idle_snapshot(&runtime.shared, &snapshot_with_pending);
        assert!(
            rx.try_recv().is_err(),
            "broker-owned prompt must retain the guard"
        );

        snapshot_with_pending.pending_prompt_count = 0;
        reconcile_idle_snapshot(&runtime.shared, &snapshot_with_pending);
        assert_eq!(
            rx.recv().await.expect("dispatch after broker drained").text,
            "second"
        );
    }

    #[tokio::test]
    async fn workspace_reset_keeps_native_thread_and_uses_replacement_cwd() {
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex".to_owned(),
            "/old/checkout".to_owned(),
            "test".to_owned(),
            crate::core::SessionOrigin::Web,
            false,
        );
        hub.set_agent_session_id("s", "codex-thread-1".to_owned());
        let (left, _right) = UnixStream::pair().expect("socket pair");
        let (reader, writer) = left.into_split();
        let bootstrap = RemoteBootstrap {
            socket: PathBuf::from("/tmp/unused-machine-broker.sock"),
            reader: FrameReader::new(reader),
            writer,
            workers: vec![snapshot("s")],
            buffered: Vec::new(),
        };
        let runtime = RemoteRuntime::new(
            hub.clone(),
            &bootstrap,
            "gen-1".to_owned(),
            Some("/bin/worker".to_owned()),
        );
        let mut replacement = snapshot("s").launch.expect("launch metadata");
        replacement.cwd = "/new/checkout".to_owned();
        replacement.agent_session_id = Some("codex-thread-1".to_owned());

        runtime.reset(replacement);

        let pending = runtime.shared.pending.lock();
        assert!(pending.values().any(|command| {
            matches!(
                command,
                CoreCommand::EnsureSession { session }
                    if session.cwd == "/new/checkout"
                        && session.agent_session_id.as_deref() == Some("codex-thread-1")
            )
        }));
        assert!(pending.values().any(|command| {
            matches!(
                command,
                CoreCommand::StopSession { command_id, .. }
                    if command_id.starts_with("reset-")
            )
        }));
        assert_eq!(
            hub.session_info("s")
                .expect("session")
                .meta
                .agent_session_id
                .as_deref(),
            Some("codex-thread-1")
        );
    }

    #[tokio::test]
    async fn workspace_rejection_automatically_queues_worker_reset() {
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            crate::core::SessionOrigin::Web,
            false,
        );
        hub.set_agent_session_id("s", "agent-1".to_owned());
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![snapshot("s")]);

        reset_after_workspace_replacement(&runtime.shared, "s");

        let pending = runtime.pending_for_test();
        assert!(pending.iter().any(|command| matches!(
            command,
            CoreCommand::EnsureSession { session }
                if session.agent_session_id.as_deref() == Some("agent-1")
        )));
        assert!(pending.iter().any(|command| matches!(
            command,
            CoreCommand::StopSession { command_id, .. }
                if command_id.starts_with("reset-")
        )));
        assert_eq!(hub.status("s"), Some(Status::Starting));
    }

    #[tokio::test]
    async fn stale_idle_status_cannot_end_a_newer_remote_turn() {
        let hub = Hub::new();
        hub.create_local_session(
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
            socket: PathBuf::from("/tmp/unused-machine-broker.sock"),
            reader: FrameReader::new(reader),
            writer,
            workers: vec![snapshot("s")],
            buffered: Vec::new(),
        };
        let runtime = RemoteRuntime::new(
            hub,
            &bootstrap,
            "gen-1".to_owned(),
            Some("/bin/worker".to_owned()),
        );
        let epoch = "worker-1".to_owned();

        for (runtime_seq, event) in [
            (
                1,
                RuntimeEvent::TurnStarted {
                    turn_id: "new-turn".to_owned(),
                    command_id: "new-prompt".to_owned(),
                },
            ),
            (
                2,
                RuntimeEvent::Status {
                    state: WorkerState::Running,
                    detail: None,
                },
            ),
        ] {
            handle_frame(
                &runtime.shared,
                Frame::WorkerEvent {
                    session_id: "s".to_owned(),
                    worker_epoch: epoch.clone(),
                    runtime_seq,
                    event,
                },
                &mut tokio::io::sink(),
            )
            .await
            .expect("worker event");
        }
        assert_eq!(runtime.shared.hub.status("s"), Some(Status::Busy));
        assert_eq!(
            runtime
                .shared
                .workers
                .lock()
                .get("s")
                .and_then(|worker| worker.current_turn_id.as_deref()),
            Some("new-turn")
        );

        for (runtime_seq, event) in [
            (
                3,
                RuntimeEvent::TurnEnded {
                    turn_id: "new-turn".to_owned(),
                    stop_reason: "EndTurn".to_owned(),
                },
            ),
            (
                4,
                RuntimeEvent::Status {
                    state: WorkerState::Running,
                    detail: None,
                },
            ),
        ] {
            handle_frame(
                &runtime.shared,
                Frame::WorkerEvent {
                    session_id: "s".to_owned(),
                    worker_epoch: epoch.clone(),
                    runtime_seq,
                    event,
                },
                &mut tokio::io::sink(),
            )
            .await
            .expect("worker event");
        }
        assert_eq!(runtime.shared.hub.status("s"), Some(Status::Running));
    }

    #[tokio::test]
    async fn shutdown_returns_unacknowledged_prompt_to_queue() {
        let hub = Hub::new();
        hub.create_local_session(
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
            socket: PathBuf::from("/tmp/unused-machine-broker.sock"),
            reader: FrameReader::new(reader),
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
        hub.create_local_session(
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
            socket: PathBuf::from("/tmp/unused-machine-broker.sock"),
            reader: FrameReader::new(reader),
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
