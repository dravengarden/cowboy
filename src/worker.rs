//! Detached per-session ACP worker.
//!
//! A worker owns exactly one ACP client connection and its adapter subtree. It
//! connects *out* to Machine broker, so Cowboy and Machine broker may restart while the worker
//! keeps an in-flight prompt and pending permission responders alive.

use std::collections::{BTreeMap, HashSet};
use std::os::unix::fs::MetadataExt as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::ContentBlock;
use anyhow::{Context as _, Result};
use parking_lot::Mutex;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use crate::acp::{self, AgentCommand};
use crate::agent_model::{Event, Status};
use crate::agent_sink::AgentSink;
use crate::provider;
use crate::runtime_wire::{
    Frame, FrameReader, MIN_PROTOCOL_VERSION, PROTOCOL_VERSION, PeerRole, RuntimeEvent,
    WorkerCommand, WorkerSnapshot, WorkerState, read_frame, write_frame,
};

#[derive(Debug, Clone)]
pub struct WorkerArgs {
    pub socket: PathBuf,
    pub session_id: String,
    pub provider: String,
    pub cwd: PathBuf,
    pub resume: Option<String>,
    pub system: bool,
    pub generation: String,
    pub worker_epoch: Option<String>,
    pub fallback_for: Option<String>,
}

struct Shared {
    session_id: String,
    worker_epoch: String,
    generation: String,
    executable: Option<String>,
    fallback_for: Option<String>,
    system: bool,
    next_seq: AtomicU64,
    snapshot: Mutex<WorkerSnapshot>,
    outbox: Mutex<BTreeMap<u64, RuntimeEvent>>,
    notify: mpsc::UnboundedSender<()>,
    seen_commands: Mutex<HashSet<String>>,
    done: AtomicBool,
    workspace_path: PathBuf,
    workspace_identity: Option<WorkspaceIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorkspaceIdentity {
    device: u64,
    inode: u64,
}

fn workspace_identity(path: &std::path::Path) -> Option<WorkspaceIdentity> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(WorkspaceIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

impl Shared {
    fn workspace_is_current(&self) -> bool {
        self.workspace_identity.is_some()
            && workspace_identity(&self.workspace_path) == self.workspace_identity
    }

    fn emit(&self, event: RuntimeEvent) -> u64 {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        self.apply_snapshot_event(seq, &event);
        self.outbox.lock().insert(seq, event);
        let _ = self.notify.send(());
        seq
    }

    fn apply_snapshot_event(&self, seq: u64, event: &RuntimeEvent) {
        let mut snapshot = self.snapshot.lock();
        snapshot.last_runtime_seq = seq;
        match event {
            RuntimeEvent::Ready { agent_session_id } => {
                snapshot.state = WorkerState::Running;
                if agent_session_id.is_some() {
                    snapshot.agent_session_id.clone_from(agent_session_id);
                }
            }
            RuntimeEvent::Status { state, .. } => {
                snapshot.state = if *state == WorkerState::Running && snapshot.drain_requested {
                    WorkerState::Draining
                } else {
                    *state
                };
                // A late startup/idle Running edge can arrive after a prompt
                // was accepted. Only TurnEnded owns clean turn completion;
                // process death may clear it without that edge.
                if matches!(state, WorkerState::Exited | WorkerState::Crashed) {
                    snapshot.current_turn_id = None;
                }
            }
            RuntimeEvent::PermissionRequest { request_id, .. } => {
                if !snapshot.pending_permissions.contains(request_id) {
                    snapshot.pending_permissions.push(request_id.clone());
                }
            }
            RuntimeEvent::PermissionResolved { request_id, .. } => {
                snapshot.pending_permissions.retain(|id| id != request_id);
            }
            RuntimeEvent::TurnStarted { turn_id, .. } => {
                snapshot.current_turn_id = Some(turn_id.clone());
                snapshot.state = WorkerState::Busy;
            }
            RuntimeEvent::TurnEnded { turn_id, .. } => {
                if snapshot.current_turn_id.as_deref() == Some(turn_id) {
                    snapshot.current_turn_id = None;
                }
            }
            RuntimeEvent::AgentSessionId { agent_session_id } => {
                snapshot.agent_session_id = Some(agent_session_id.clone());
            }
            RuntimeEvent::ConfigOptions { options } => {
                snapshot.config_options = Some(options.clone());
            }
            RuntimeEvent::ContextUsage { used, size, .. } => {
                snapshot.context_used = Some(*used);
                snapshot.context_size = Some(*size);
            }
            RuntimeEvent::Update { .. }
            | RuntimeEvent::ScheduleWakeup { .. }
            | RuntimeEvent::UndeliveredPrompt { .. }
            | RuntimeEvent::CommandRejected { .. }
            | RuntimeEvent::Error { .. } => {}
        }
    }

    fn snapshot(&self) -> WorkerSnapshot {
        self.snapshot.lock().clone()
    }

    fn ack(&self, seq: u64) {
        self.outbox.lock().retain(|candidate, _| *candidate > seq);
    }

    fn mark_command(&self, command_id: &str) -> bool {
        self.seen_commands.lock().insert(command_id.to_owned())
    }

    fn unmark_command(&self, command_id: &str) {
        self.seen_commands.lock().remove(command_id);
    }
}

#[derive(Clone)]
struct RemoteSink {
    shared: Arc<Shared>,
}

impl RemoteSink {
    fn status_state(status: Status) -> WorkerState {
        match status {
            Status::Starting => WorkerState::Starting,
            Status::Running => WorkerState::Running,
            Status::Busy => WorkerState::Busy,
            Status::Exited | Status::Interrupted => WorkerState::Exited,
            Status::Crashed => WorkerState::Crashed,
        }
    }

    fn emit_event(&self, event: Event, cmid: Option<String>) {
        let runtime = match event {
            Event::Update { update } => RuntimeEvent::Update { update, cmid },
            Event::PermissionRequest {
                request_id,
                tool_call,
                options,
            } => RuntimeEvent::PermissionRequest {
                request_id,
                tool_call,
                options,
            },
            Event::PermissionResolved {
                request_id,
                option_id,
            } => RuntimeEvent::PermissionResolved {
                request_id,
                option_id,
            },
            Event::Lifecycle { status, detail } => RuntimeEvent::Status {
                state: Self::status_state(status),
                detail,
            },
            Event::TurnEnd { stop_reason } => {
                let turn_id = self
                    .shared
                    .snapshot
                    .lock()
                    .current_turn_id
                    .clone()
                    .unwrap_or_else(|| "unknown-turn".to_owned());
                RuntimeEvent::TurnEnded {
                    turn_id,
                    stop_reason,
                }
            }
        };
        self.shared.emit(runtime);
    }
}

impl AgentSink for RemoteSink {
    fn set_status(&self, _session_id: &str, status: Status, detail: Option<String>) {
        let mut state = Self::status_state(status);
        if state == WorkerState::Running && self.shared.snapshot.lock().drain_requested {
            state = WorkerState::Draining;
        }
        self.shared.emit(RuntimeEvent::Status { state, detail });
    }

    fn push(&self, _session_id: &str, event: Event) {
        self.emit_event(event, None);
    }

    fn push_tagged(&self, _session_id: &str, event: Event, cmid: Option<String>) {
        self.emit_event(event, cmid);
    }

    fn set_config_options(&self, _session_id: &str, options: serde_json::Value) {
        self.shared.emit(RuntimeEvent::ConfigOptions { options });
    }

    fn set_agent_session_id(&self, _session_id: &str, agent_session_id: String) {
        self.shared
            .emit(RuntimeEvent::AgentSessionId { agent_session_id });
    }

    fn set_session_usage(&self, _session_id: &str, usage: crate::agent_model::SessionUsage) {
        self.shared.emit(RuntimeEvent::ContextUsage {
            used: usage.used,
            size: usage.size,
            raw: usage.raw,
            observed_at_ms: usage.observed_at_ms,
        });
    }

    fn schedule_wakeup(&self, _session_id: &str, delay_seconds: i64, prompt: String) {
        self.shared.emit(RuntimeEvent::ScheduleWakeup {
            delay_seconds,
            prompt,
        });
    }

    fn session_is_system(&self, _session_id: &str) -> bool {
        self.shared.system
    }

    fn broadcast_error(&self, _session_id: Option<String>, message: String) {
        self.shared.emit(RuntimeEvent::Error { message });
    }

    fn requeue_prompt(
        &self,
        _session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) {
        let content = if content.is_empty() && !text.is_empty() {
            vec![serde_json::json!({"type": "text", "text": text})]
        } else {
            content
        };
        self.shared.emit(RuntimeEvent::UndeliveredPrompt {
            command_id: "unknown-command".to_owned(),
            cmid,
            content,
            error: "agent failed before accepting prompt".to_owned(),
        });
    }
}

fn generated_epoch() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{}-{now}", std::process::id())
}

/// Run one detached worker until its ACP session and acknowledged outbox have
/// both drained.
pub async fn run(args: WorkerArgs) -> Result<()> {
    let spec = provider::lookup(&args.provider)
        .ok_or_else(|| anyhow::anyhow!("unknown provider {:?}", args.provider))?;
    let epoch = args.worker_epoch.unwrap_or_else(generated_epoch);
    let executable = std::env::current_exe()
        .ok()
        .map(|path| path.display().to_string());
    let launch = crate::runtime_wire::StartSession {
        session_id: args.session_id.clone(),
        provider: args.provider.clone(),
        cwd: args.cwd.display().to_string(),
        agent_session_id: args.resume.clone(),
        system: args.system,
        generation: args.generation.clone(),
        fallback_for: args.fallback_for.clone(),
        adopt_only: false,
    };
    let (notify_tx, notify_rx) = mpsc::unbounded_channel();
    let expected_workspace_identity = workspace_identity(&args.cwd);
    let shared = Arc::new(Shared {
        session_id: args.session_id.clone(),
        worker_epoch: epoch.clone(),
        generation: args.generation.clone(),
        executable: executable.clone(),
        fallback_for: args.fallback_for,
        system: args.system,
        next_seq: AtomicU64::new(1),
        snapshot: Mutex::new(WorkerSnapshot {
            session_id: args.session_id.clone(),
            worker_epoch: epoch,
            generation: args.generation,
            executable,
            launch: Some(launch),
            state: WorkerState::Starting,
            agent_session_id: args.resume.clone(),
            current_turn_id: None,
            last_runtime_seq: 0,
            pending_permissions: Vec::new(),
            config_options: None,
            context_used: Some(0),
            context_size: Some(0),
            pending_prompt_count: 0,
            drain_requested: false,
        }),
        outbox: Mutex::new(BTreeMap::new()),
        notify: notify_tx,
        seen_commands: Mutex::new(HashSet::new()),
        done: AtomicBool::new(false),
        workspace_path: args.cwd.clone(),
        workspace_identity: expected_workspace_identity,
    });
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (done_tx, mut done_rx) = mpsc::channel(1);
    let thread_shared = Arc::clone(&shared);
    let session_id = args.session_id.clone();
    let cwd = args.cwd;
    let resume = args.resume;
    std::thread::Builder::new()
        .name(format!("acp-worker-{session_id}"))
        .spawn(move || {
            let sink: Arc<dyn AgentSink> = Arc::new(RemoteSink {
                shared: Arc::clone(&thread_shared),
            });
            acp::run_agent_with_sink(&spec, &session_id, cwd, resume, cmd_rx, &sink);
            thread_shared.done.store(true, Ordering::Release);
            let _ = done_tx.blocking_send(());
        })
        .context("spawning ACP worker thread")?;

    connection_loop(&args.socket, shared, cmd_tx, notify_rx, &mut done_rx).await
}

async fn connection_loop(
    socket: &PathBuf,
    shared: Arc<Shared>,
    cmd_tx: mpsc::UnboundedSender<AgentCommand>,
    mut notify_rx: mpsc::UnboundedReceiver<()>,
    done_rx: &mut mpsc::Receiver<()>,
) -> Result<()> {
    let mut backoff = Duration::from_millis(100);
    let mut cmd_tx = Some(cmd_tx);
    loop {
        let stream = match UnixStream::connect(socket).await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(error = %error, socket = %socket.display(), "Machine broker unavailable; worker keeps running");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(5));
                continue;
            }
        };
        backoff = Duration::from_millis(100);
        match connected(
            stream,
            Arc::clone(&shared),
            &mut cmd_tx,
            &mut notify_rx,
            done_rx,
        )
        .await
        {
            Ok(ConnectedExit::WorkerDrained) => return Ok(()),
            Ok(ConnectedExit::Disconnected) => {}
            Err(error) => {
                tracing::warn!(error = %error, "Machine broker connection lost; reconnecting")
            }
        }
    }
}

enum ConnectedExit {
    Disconnected,
    WorkerDrained,
}

async fn connected(
    stream: UnixStream,
    shared: Arc<Shared>,
    cmd_tx: &mut Option<mpsc::UnboundedSender<AgentCommand>>,
    notify_rx: &mut mpsc::UnboundedReceiver<()>,
    done_rx: &mut mpsc::Receiver<()>,
) -> Result<ConnectedExit> {
    let (mut reader, mut writer) = stream.into_split();
    write_frame(
        &mut writer,
        &Frame::Hello {
            role: PeerRole::Worker,
            min_protocol: MIN_PROTOCOL_VERSION,
            max_protocol: PROTOCOL_VERSION,
            build: env!("CARGO_PKG_VERSION").to_owned(),
            session_id: Some(shared.session_id.clone()),
            worker_epoch: Some(shared.worker_epoch.clone()),
            generation: Some(shared.generation.clone()),
            executable: shared.executable.clone(),
            fallback_for: shared.fallback_for.clone(),
        },
    )
    .await?;
    match read_frame(&mut reader).await? {
        Some(Frame::Welcome { .. }) => {}
        Some(Frame::Reject { reason }) => anyhow::bail!("Machine broker rejected worker: {reason}"),
        Some(other) => anyhow::bail!("unexpected Machine broker handshake frame: {other:?}"),
        None => return Ok(ConnectedExit::Disconnected),
    }
    write_frame(
        &mut writer,
        &Frame::Snapshot {
            worker: Box::new(shared.snapshot()),
        },
    )
    .await?;
    let mut reader = FrameReader::new(reader);
    let mut last_sent = 0;
    send_outbox(&shared, &mut writer, &mut last_sent).await?;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            incoming = reader.next() => {
                let Some(frame) = incoming? else {
                    return Ok(ConnectedExit::Disconnected);
                };
                match frame {
                    Frame::WorkerCommand { session_id, command }
                        if session_id == shared.session_id => {
                        let ack = handle_command(&shared, cmd_tx, command);
                        // Publish state transitions (especially TurnStarted)
                        // before ACKing acceptance. Once Cowboy drops a pending
                        // command, Machine broker's snapshot is therefore already
                        // authoritative across an immediate core restart.
                        send_outbox(&shared, &mut writer, &mut last_sent).await?;
                        write_frame(&mut writer, &ack).await?;
                    }
                    Frame::Ack { session_id, worker_epoch, runtime_seq }
                        if session_id == shared.session_id && worker_epoch == shared.worker_epoch => {
                        shared.ack(runtime_seq);
                        if shared.done.load(Ordering::Acquire) && shared.outbox.lock().is_empty() {
                            return Ok(ConnectedExit::WorkerDrained);
                        }
                    }
                    Frame::Replay { session_id, worker_epoch, after_runtime_seq }
                        if session_id == shared.session_id && worker_epoch == shared.worker_epoch => {
                        last_sent = after_runtime_seq;
                        send_outbox(&shared, &mut writer, &mut last_sent).await?;
                    }
                    Frame::Heartbeat => {}
                    Frame::Reject { reason } => anyhow::bail!("Machine broker rejected live worker: {reason}"),
                    other => tracing::debug!(?other, "ignoring unrelated runtime frame"),
                }
            }
            notified = notify_rx.recv() => {
                if notified.is_none()
                    && shared.done.load(Ordering::Acquire)
                    && shared.outbox.lock().is_empty()
                {
                    return Ok(ConnectedExit::WorkerDrained);
                }
                send_outbox(&shared, &mut writer, &mut last_sent).await?;
            }
            done = done_rx.recv() => {
                if done.is_some() && shared.outbox.lock().is_empty() {
                    return Ok(ConnectedExit::WorkerDrained);
                }
                send_outbox(&shared, &mut writer, &mut last_sent).await?;
            }
            _ = heartbeat.tick() => {
                write_frame(&mut writer, &Frame::Heartbeat).await?;
            }
        }
    }
}

async fn send_outbox<W: tokio::io::AsyncWrite + Unpin>(
    shared: &Shared,
    writer: &mut W,
    last_sent: &mut u64,
) -> Result<()> {
    let pending: Vec<_> = shared
        .outbox
        .lock()
        .range((last_sent.saturating_add(1))..)
        .map(|(seq, event)| (*seq, event.clone()))
        .collect();
    for (seq, event) in pending {
        write_frame(
            writer,
            &Frame::WorkerEvent {
                session_id: shared.session_id.clone(),
                worker_epoch: shared.worker_epoch.clone(),
                runtime_seq: seq,
                event,
            },
        )
        .await?;
        *last_sent = seq;
    }
    Ok(())
}

fn handle_command(
    shared: &Shared,
    cmd_tx: &mut Option<mpsc::UnboundedSender<AgentCommand>>,
    command: WorkerCommand,
) -> Frame {
    let (command_id, agent_command, turn): (String, Option<AgentCommand>, Option<String>) =
        match command {
            WorkerCommand::Prompt {
                command_id,
                turn_id,
                content,
                cmid,
            } => {
                if !shared.workspace_is_current() {
                    let message = format!(
                        "session workspace was replaced or removed: {}; retry after Cowboy recycles this worker",
                        shared.workspace_path.display()
                    );
                    shared.emit(RuntimeEvent::Status {
                        state: WorkerState::Crashed,
                        detail: Some(message.clone()),
                    });
                    return command_ack(shared, command_id, false, Some(message));
                }
                if !shared.mark_command(&command_id) {
                    return command_ack(shared, command_id, true, Some("duplicate".to_owned()));
                }
                let blocks: Vec<ContentBlock> = content
                    .into_iter()
                    .filter_map(|value| serde_json::from_value(value).ok())
                    .collect();
                if blocks.is_empty() {
                    return command_ack(
                        shared,
                        command_id,
                        false,
                        Some("prompt has no valid content blocks".to_owned()),
                    );
                }
                (
                    command_id,
                    Some(AgentCommand::Prompt(blocks, cmid, None)),
                    Some(turn_id),
                )
            }
            WorkerCommand::Cancel { command_id } => {
                if !shared.mark_command(&command_id) {
                    return command_ack(shared, command_id, true, Some("duplicate".to_owned()));
                }
                (command_id, Some(AgentCommand::Cancel), None)
            }
            WorkerCommand::Permission {
                command_id,
                request_id,
                option_id,
            } => {
                if !shared.mark_command(&command_id) {
                    return command_ack(shared, command_id, true, Some("duplicate".to_owned()));
                }
                (
                    command_id,
                    Some(AgentCommand::Permission {
                        request_id,
                        option_id,
                    }),
                    None,
                )
            }
            WorkerCommand::SetConfigOption {
                command_id,
                config_id,
                value,
            } => {
                if !shared.mark_command(&command_id) {
                    return command_ack(shared, command_id, true, Some("duplicate".to_owned()));
                }
                (
                    command_id,
                    Some(AgentCommand::SetConfigOption { config_id, value }),
                    None,
                )
            }
            WorkerCommand::Drain => {
                let idle = {
                    let mut snapshot = shared.snapshot.lock();
                    snapshot.drain_requested = true;
                    snapshot.state == WorkerState::Running
                };
                if idle {
                    shared.emit(RuntimeEvent::Status {
                        state: WorkerState::Draining,
                        detail: Some("waiting for generation handoff".to_owned()),
                    });
                }
                return command_ack(shared, "drain".to_owned(), true, None);
            }
            WorkerCommand::Stop { command_id } => {
                if !shared.mark_command(&command_id) {
                    return command_ack(shared, command_id, true, Some("duplicate".to_owned()));
                }
                if let Some(tx) = cmd_tx.take() {
                    let _ = tx.send(AgentCommand::Cancel);
                }
                return command_ack(shared, command_id, true, None);
            }
        };
    // Publish the turn edge before handing the prompt to the ACP thread. Some
    // agents can complete a trivial prompt synchronously; sending first lets
    // their TurnEnded race ahead of TurnStarted and leaves the controller busy.
    if let Some(turn_id) = turn.as_ref() {
        shared.emit(RuntimeEvent::TurnStarted {
            turn_id: turn_id.clone(),
            command_id: command_id.clone(),
        });
    }
    let sent = match (agent_command, cmd_tx.as_ref()) {
        (Some(command), Some(tx)) => tx.send(command).is_ok(),
        (Some(_), None) => false,
        (None, _) => true,
    };
    if sent {
        command_ack(shared, command_id, true, None)
    } else {
        // A reconnect may safely retry a command that never reached ACP.
        shared.unmark_command(&command_id);
        if turn.is_some() {
            shared.emit(RuntimeEvent::Status {
                state: WorkerState::Crashed,
                detail: Some("ACP command loop is closed".to_owned()),
            });
        }
        command_ack(
            shared,
            command_id,
            false,
            Some("ACP command loop is closed".to_owned()),
        )
    }
}

fn command_ack(
    shared: &Shared,
    command_id: String,
    accepted: bool,
    reason: Option<String>,
) -> Frame {
    Frame::CommandAck {
        session_id: shared.session_id.clone(),
        command_id,
        accepted,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> (Arc<Shared>, mpsc::UnboundedReceiver<()>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let workspace_path = std::env::current_dir().expect("current dir");
        let expected_workspace_identity = workspace_identity(&workspace_path);
        (
            Arc::new(Shared {
                session_id: "sess-1".to_owned(),
                worker_epoch: "epoch-1".to_owned(),
                generation: "gen-1".to_owned(),
                executable: Some("/test/cowboy-acp-worker".to_owned()),
                fallback_for: None,
                system: false,
                next_seq: AtomicU64::new(1),
                snapshot: Mutex::new(WorkerSnapshot {
                    session_id: "sess-1".to_owned(),
                    worker_epoch: "epoch-1".to_owned(),
                    generation: "gen-1".to_owned(),
                    executable: Some("/test/cowboy-acp-worker".to_owned()),
                    launch: None,
                    state: WorkerState::Running,
                    agent_session_id: None,
                    current_turn_id: None,
                    last_runtime_seq: 0,
                    pending_permissions: vec![],
                    config_options: None,
                    context_used: Some(0),
                    context_size: Some(0),
                    pending_prompt_count: 0,
                    drain_requested: false,
                }),
                outbox: Mutex::new(BTreeMap::new()),
                notify: tx,
                seen_commands: Mutex::new(HashSet::new()),
                done: AtomicBool::new(false),
                workspace_path,
                workspace_identity: expected_workspace_identity,
            }),
            rx,
        )
    }

    #[test]
    fn event_ack_prunes_only_acknowledged_prefix() {
        let (shared, _rx) = shared();
        shared.emit(RuntimeEvent::Error {
            message: "a".to_owned(),
        });
        shared.emit(RuntimeEvent::Error {
            message: "b".to_owned(),
        });
        shared.emit(RuntimeEvent::Error {
            message: "c".to_owned(),
        });
        shared.ack(2);
        assert_eq!(
            shared.outbox.lock().keys().copied().collect::<Vec<_>>(),
            [3]
        );
    }

    #[test]
    fn duplicate_command_id_never_reaches_acp_twice() {
        let (shared, _rx) = shared();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut tx = Some(tx);
        let command = || WorkerCommand::Cancel {
            command_id: "cmd-1".to_owned(),
        };
        let first = handle_command(&shared, &mut tx, command());
        let duplicate = handle_command(&shared, &mut tx, command());
        assert!(matches!(first, Frame::CommandAck { accepted: true, .. }));
        assert!(
            matches!(duplicate, Frame::CommandAck { accepted: true, reason: Some(reason), .. } if reason == "duplicate")
        );
        assert!(matches!(rx.try_recv(), Ok(AgentCommand::Cancel)));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn prompt_turn_starts_before_acp_can_receive_it() {
        let (shared, _rx) = shared();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut tx = Some(tx);
        let ack = handle_command(
            &shared,
            &mut tx,
            WorkerCommand::Prompt {
                command_id: "cmd-1".to_owned(),
                turn_id: "turn-1".to_owned(),
                content: vec![serde_json::json!({"type": "text", "text": "hello"})],
                cmid: None,
            },
        );

        assert!(matches!(ack, Frame::CommandAck { accepted: true, .. }));
        let snapshot = shared.snapshot();
        assert_eq!(snapshot.current_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(snapshot.state, WorkerState::Busy);
        assert!(matches!(rx.try_recv(), Ok(AgentCommand::Prompt(..))));
        assert!(matches!(
            shared.outbox.lock().get(&1),
            Some(RuntimeEvent::TurnStarted { turn_id, .. }) if turn_id == "turn-1"
        ));
    }

    #[test]
    fn stale_running_status_does_not_clear_active_turn() {
        let (shared, _rx) = shared();
        shared.emit(RuntimeEvent::TurnStarted {
            turn_id: "turn-1".to_owned(),
            command_id: "cmd-1".to_owned(),
        });
        shared.emit(RuntimeEvent::Status {
            state: WorkerState::Running,
            detail: None,
        });

        let snapshot = shared.snapshot();
        assert_eq!(snapshot.current_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(snapshot.state, WorkerState::Running);
    }

    #[test]
    fn workspace_identity_detects_same_path_replacement() {
        let root =
            std::env::temp_dir().join(format!("cowboy-worker-workspace-{}", generated_epoch()));
        let stable = root.join("main");
        let backup = root.join("backup");
        std::fs::create_dir_all(&stable).expect("stable");
        let original = workspace_identity(&stable).expect("original identity");

        std::fs::rename(&stable, &backup).expect("move original");
        std::fs::create_dir_all(&stable).expect("replacement");

        assert_ne!(workspace_identity(&stable), Some(original));
        let _ = std::fs::remove_dir_all(root);
    }
}
