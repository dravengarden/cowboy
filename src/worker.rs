//! Detached per-session ACP worker.
//!
//! A worker owns exactly one ACP client connection and its adapter subtree. It
//! connects *out* to Machine broker, so Cowboy and Machine broker may restart while the worker
//! keeps an in-flight prompt and pending permission responders alive.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::os::unix::fs::MetadataExt as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

const BROKER_RECONNECT_INITIAL_DELAY: Duration = Duration::from_millis(100);
mod execution_launch;
mod mcp_launch;
mod remote_sink;
mod request_observation;
mod sandbox_extensions;
pub mod startup_gate;
mod workspace;
pub use remote_sink::RemoteSink;
use workspace::{WorkspaceIdentity, workspace_identity};
const BROKER_RECONNECT_MAX_DELAY: Duration = Duration::from_secs(5);
const BROKER_CONNECTION_STABLE_AFTER: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct WorkerArgs {
    pub mcp_plan: Option<Box<crate::mcp_binding::NativeLaunchPlan>>,
    pub socket: PathBuf,
    pub session_id: String,
    pub provider: String,
    pub provider_version: String,
    pub provider_generation_digest: String,
    pub provider_auth_generation: Option<u64>,
    pub provider_account_id: crate::provider_account::ProviderAccountId,
    pub cwd: PathBuf,
    pub resume: Option<String>,
    /// First native thread forks this source instead of `session/new`.
    pub native_fork: Option<crate::runtime_wire::NativeFork>,
    pub system: bool,
    pub generation: String,
    pub worker_epoch: Option<String>,
    pub fallback_for: Option<String>,
    pub execution_binding: Option<crate::execution_environment::ExecutionBinding>,
}

pub struct Shared {
    session_id: String,
    worker_epoch: String,
    generation: String,
    executable: Option<String>,
    fallback_for: Option<String>,
    system: bool,
    next_seq: AtomicU64,
    snapshot: Mutex<WorkerSnapshot>,
    outbox: Mutex<BTreeMap<u64, RuntimeEvent>>,
    pub trace_outbox: Mutex<BTreeMap<u64, Vec<crate::runtime_trace::SpanRecord>>>,
    telemetry: Mutex<crate::worker_telemetry::WorkerTelemetry>,
    request_observer: Option<request_observation::Observer>,
    notify: mpsc::UnboundedSender<()>,
    seen_commands: Mutex<HashSet<String>>,
    /// In-flight `(cmid, command_id)` pairs for prompts handed to the ACP
    /// thread, so `RemoteSink::requeue_prompt` emits the original `command_id`
    /// on `UndeliveredPrompt` rather than a placeholder.
    prompt_commands: Mutex<VecDeque<(Option<String>, String)>>,
    /// Digests of prompt images Cowboy stored before dispatch, newest last.
    /// Their echoes travel by reference instead of carrying the bytes back.
    echo_artifacts: Mutex<VecDeque<String>>,
    workspace_path: PathBuf,
    workspace_identity: Option<WorkspaceIdentity>,
    execution: Option<Arc<crate::worker_execution::Client>>,
    /// A remote managed child's round, read once from its execution target
    /// before the first prompt; each round starts a new worker.
    managed_round: Option<ManagedRound>,
    /// Options the signed package's presets set; see `PreparedLaunch`.
    preset_options: std::collections::BTreeSet<String>,
    /// Passive observation of subagents' own updates.
    observation: Observation,
}

/// Passive observation of subagents' own updates (docs/subagents.md).
#[derive(Default)]
struct Observation {
    /// The latest prompt's Controller keeps observed subagent transcripts.
    enabled: AtomicBool,
    /// Runtime protocol negotiated with the Machine broker; observations are
    /// retired at egress below `SUBAGENT_OBSERVATION_PROTOCOL_VERSION`.
    peer_protocol: AtomicU16,
    /// Per child, observed updates dropped since its last emitted one. Only
    /// the ACP thread touches it.
    dropped: Mutex<HashMap<String, u64>>,
}

/// Unacknowledged runtime events beyond which a subagent's observed update is
/// dropped instead of queued: observation never grows the outbox without
/// bound while the Controller is away, and never waits for it.
const OBSERVE_OUTBOX_LIMIT: usize = 4096;
/// Serialized bytes one observed update may carry, far below a frame.
const OBSERVE_MAX_BYTES: usize = 1024 * 1024;
/// Children whose dropped updates are attributed; more are not counted.
const OBSERVE_DROP_CHILDREN: usize = 1024;

struct ManagedRound {
    ready: tokio::sync::watch::Receiver<Option<Result<(), String>>>,
    marker: Arc<Mutex<Option<crate::managed_calls::round::RoundMarker>>>,
}

fn current_executable() -> Option<String> {
    std::env::current_exe()
        .ok()
        .map(|path| path.display().to_string())
}

impl Shared {
    fn managed_read_only(&self) -> bool {
        self.snapshot
            .lock()
            .launch
            .as_ref()
            .and_then(|launch| launch.execution_binding.as_ref())
            .and_then(crate::execution_environment::ExecutionBinding::managed)
            .is_some()
    }

    fn emit(&self, event: RuntimeEvent) -> u64 {
        // Keep sequence assignment and event/metadata admission ordered even
        // when the ACP thread and broker command loop emit concurrently.
        let mut outbox = self.outbox.lock();
        self.emit_locked(&mut outbox, event)
    }

    fn emit_locked(&self, outbox: &mut BTreeMap<u64, RuntimeEvent>, event: RuntimeEvent) -> u64 {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        self.apply_snapshot_event(seq, &event);
        let diagnostics = self.telemetry.lock().event(&event);
        crate::logs::runtime_spans(&self.session_id, &diagnostics);
        if crate::logs::forward_runtime() && !diagnostics.is_empty() {
            let mut traces = self.trace_outbox.lock();
            if traces.len() < 128 {
                traces.insert(seq, diagnostics);
            }
        }
        outbox.insert(seq, event);
        let _ = self.notify.send(());
        seq
    }

    /// Open or close the execution ledger's readiness window: the
    /// projection's readiness probe runs only while this worker is starting
    /// its session, an authorization from worker state, never from the
    /// caller's process id.
    fn readiness_window(&self, starting: bool) {
        if let Some(ledger) = self.execution.as_ref().and_then(|client| client.ledger()) {
            ledger.set_readiness(starting);
        }
    }

    fn apply_snapshot_event(&self, seq: u64, event: &RuntimeEvent) {
        let mut snapshot = self.snapshot.lock();
        snapshot.last_runtime_seq = seq;
        snapshot.observe_native_thread(event);
        match event {
            RuntimeEvent::Ready { .. } => {
                snapshot.state = WorkerState::Running;
                self.readiness_window(false);
            }
            RuntimeEvent::Status { state, .. } => {
                snapshot.state = if *state == WorkerState::Running && snapshot.drain_requested {
                    WorkerState::Draining
                } else {
                    *state
                };
                // Only a starting session may run the readiness probe.
                self.readiness_window(*state == WorkerState::Starting);
                // A late startup/idle Running edge can arrive after a prompt
                // was accepted. Only TurnEnded owns clean turn completion;
                // process death may clear it without that edge.
                if matches!(state, WorkerState::Exited | WorkerState::Crashed) {
                    snapshot.current_turn_id = None;
                    snapshot.native_cycle_id = None;
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
                if let Some(ledger) = self.execution.as_ref().and_then(|client| client.ledger()) {
                    ledger.begin_turn(turn_id);
                }
                snapshot.current_turn_id = Some(turn_id.clone());
                // A Cowboy turn owns delivery from here; the adapter ends the
                // self-started cycle when it receives the prompt.
                snapshot.native_cycle_id = None;
                snapshot.state = WorkerState::Busy;
            }
            RuntimeEvent::TurnEnded { turn_id, .. } => {
                if snapshot.current_turn_id.as_deref() == Some(turn_id) {
                    snapshot.current_turn_id = None;
                }
            }
            RuntimeEvent::ConfigOptions { options } => {
                snapshot.config_options = Some(options.clone());
            }
            RuntimeEvent::ContextUsage { used, size, .. } => {
                snapshot.context_used = Some(*used);
                snapshot.context_size = Some(*size);
            }
            RuntimeEvent::Update { update, .. }
                if crate::runtime_wire::current_turn_support(update).is_some() =>
            {
                snapshot.supports_current_turn =
                    crate::runtime_wire::current_turn_support(update).unwrap_or(false);
                snapshot.delivery_controls.next_checkpoint = update
                    .get("nextCheckpoint")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
            }
            RuntimeEvent::Update { update, .. }
                if crate::runtime_wire::background_tasks_count(update).is_some() =>
            {
                snapshot.background_tasks = crate::runtime_wire::background_tasks_count(update);
            }
            RuntimeEvent::Update { update, .. }
                if crate::runtime_wire::uncertain_effect(update).is_some() =>
            {
                if let Some(effect) = crate::runtime_wire::uncertain_effect(update) {
                    crate::runtime_wire::merge_uncertain_effect(
                        &mut snapshot.uncertain_effect,
                        effect,
                    );
                }
            }
            RuntimeEvent::Update { update, .. }
                if crate::runtime_wire::native_cycle(update).is_some() =>
            {
                snapshot.native_cycle_id = crate::runtime_wire::native_cycle(update).flatten();
            }
            RuntimeEvent::AgentSessionId { .. }
            | RuntimeEvent::Update { .. }
            | RuntimeEvent::ScheduleWakeup { .. }
            | RuntimeEvent::UndeliveredPrompt { .. }
            | RuntimeEvent::CommandRejected { .. }
            | RuntimeEvent::Error { .. } => {}
        }
    }

    pub fn snapshot(&self) -> WorkerSnapshot {
        self.snapshot.lock().clone()
    }

    /// Hand `command` to the ACP thread. A cancellation fences the current
    /// turn's target process starts as the agent is asked to stop.
    fn forward(&self, tx: &mpsc::UnboundedSender<AgentCommand>, command: AgentCommand) -> bool {
        if matches!(
            command,
            AgentCommand::Cancel | AgentCommand::CancelTurn { .. }
        ) && let Some(execution) = &self.execution
        {
            execution.cancel_turn();
        }
        tx.send(command).is_ok()
    }

    /// Surface a lost mutation reply: the session holds an effect whose
    /// outcome on the target is unknown until the owner reviews it.
    fn mark_uncertain_effect(&self, method: &str, sequence: u64) {
        let effect = crate::runtime_wire::UncertainEffect {
            method: method.to_owned(),
            sequence,
        };
        crate::runtime_wire::merge_uncertain_effect(
            &mut self.snapshot.lock().uncertain_effect,
            effect.clone(),
        );
        // Published to the broker and Controller at once, ahead of the turn's
        // completion, not only in the next handshake's snapshot.
        self.emit(RuntimeEvent::Update {
            update: crate::runtime_wire::uncertain_effect_update(&effect),
            cmid: None,
        });
        self.emit(RuntimeEvent::Error {
            message: format!("The outcome of a target {method} is unknown (its reply was lost); review the workspace before continuing"),
        });
    }

    pub fn ack(&self, seq: u64) {
        self.outbox.lock().retain(|candidate, _| *candidate > seq);
        self.trace_outbox
            .lock()
            .retain(|candidate, _| *candidate > seq);
    }

    fn mark_command(&self, command_id: &str) -> bool {
        self.seen_commands.lock().insert(command_id.to_owned())
    }

    fn unmark_command(&self, command_id: &str) {
        self.seen_commands.lock().remove(command_id);
    }

    fn record_prompt_command(&self, cmid: Option<String>, command_id: String) {
        const RETAINED: usize = 64;
        let mut prompts = self.prompt_commands.lock();
        prompts.push_back((cmid, command_id));
        while prompts.len() > RETAINED {
            prompts.pop_front();
        }
    }

    fn take_prompt_command(&self, cmid: Option<&str>) -> Option<String> {
        let mut prompts = self.prompt_commands.lock();
        let pos = prompts
            .iter()
            .position(|(stored_cmid, _)| stored_cmid.as_deref() == cmid)?;
        prompts.remove(pos).map(|(_, command_id)| command_id)
    }

    fn remove_prompt_command_id(&self, command_id: &str) {
        let mut prompts = self.prompt_commands.lock();
        if let Some(pos) = prompts
            .iter()
            .rposition(|(_, stored_id)| stored_id == command_id)
        {
            prompts.remove(pos);
        }
    }

    fn remember_echo_artifacts(&self, content: &[serde_json::Value]) {
        const RETAINED: usize = 32;
        let digests = content.iter().filter_map(|block| {
            (block.get("type").and_then(serde_json::Value::as_str) == Some("image"))
                .then(|| block.get("data").and_then(serde_json::Value::as_str))
                .flatten()
                .and_then(crate::artifacts::large_image_digest)
        });
        let mut stored = self.echo_artifacts.lock();
        for digest in digests {
            if !stored.contains(&digest) {
                stored.push_back(digest);
            }
        }
        while stored.len() > RETAINED {
            stored.pop_front();
        }
    }
}

fn generated_epoch() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{}-{now}", std::process::id())
}

/// Runs `worker` until it ends or the process receives SIGTERM. A Machine
/// hibernates an idle systemd worker by stopping its unit; returning here
/// lets the caller's log guard flush queued evidence and record a clean stop
/// within the unit's stop timeout, where the default action lost both. The
/// unit's control group ends the Provider processes.
/// The handler is installed before this returns, so a signal that arrives
/// before the first poll is not lost.
pub fn until_terminated(
    session: String,
    worker: impl std::future::Future<Output = Result<()>>,
) -> Result<impl std::future::Future<Output = Result<()>>> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    Ok(async move {
        tokio::select! {
            result = worker => result,
            _ = terminate.recv() => {
                tracing::info!(event_name = "cowboy.worker.terminated", %session, "worker stopped by SIGTERM");
                Ok(())
            }
        }
    })
}

/// Run one detached worker until its ACP session and acknowledged outbox have
/// both drained.
pub async fn run(args: WorkerArgs) -> Result<()> {
    // The detached worker has its own process and never enters Cli::run.
    // Reqwest's provider-neutral TLS feature also needs this before a plain
    // loopback sidecar Client is constructed during Provider preparation.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let managed_child = args
        .execution_binding
        .as_ref()
        .and_then(|binding| binding.managed_child());
    if let Some(child) = &managed_child {
        anyhow::ensure!(
            child.session_id == args.session_id && std::path::Path::new(&child.cwd) == args.cwd,
            "managed child worker identity mismatch"
        );
    }
    // A remote managed child executes in a snapshot named after itself.
    let remote_managed = args
        .execution_binding
        .as_ref()
        .and_then(|binding| binding.decode().ok())
        .filter(|binding| binding.managed.is_some());
    if let Some(binding) = &remote_managed {
        anyhow::ensure!(
            binding.workspace.worktree_id == args.session_id,
            "managed child worker identity mismatch"
        );
    }
    let profile = args
        .execution_binding
        .as_ref()
        .and_then(crate::execution_environment::ExecutionBinding::managed)
        .map(|managed| managed.profile);
    let prepared = provider::prepare(&args.provider, profile).await?;
    let mut spec = prepared.spec.clone();
    let _mcp_descriptor = mcp_launch::configure(&args, &prepared, &mut spec)?;
    let provider_behavior = (!args.provider_generation_digest.is_empty())
        .then(|| crate::provider::behavior(&args.provider));
    let configuration = provider_behavior.as_ref().map_or_else(
        || crate::provider::legacy_behavior(&args.provider).configuration,
        |behavior| behavior.configuration.clone(),
    );
    let context_budget = std::env::var(crate::deepseek_context::SESSION_CONTEXT_WINDOW_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .zip(
            std::env::var(crate::deepseek_context::SESSION_AUTO_COMPACT_TOKEN_LIMIT_ENV)
                .ok()
                .and_then(|value| value.parse::<u64>().ok()),
        )
        .and_then(|(window, compact)| {
            crate::deepseek_context::from_launch_values(&configuration, window, compact)
        });
    let cache_protection = std::env::var(crate::deepseek_cache::SESSION_POLICY_ENV)
        .ok()
        .and_then(|value| match value.as_str() {
            "auto" => Some(true),
            "off" => Some(false),
            _ => None,
        })
        .filter(|_| crate::deepseek_cache::supported_behavior(&configuration));
    let epoch = args.worker_epoch.clone().unwrap_or_else(generated_epoch);
    let executable = current_executable();
    let launch = crate::runtime_wire::StartSession {
        session_id: args.session_id.clone(),
        provider: args.provider.clone(),
        provider_version: args.provider_version.clone(),
        provider_generation_digest: args.provider_generation_digest.clone(),
        provider_account_id: args.provider_account_id.clone(),
        provider_auth_generation: args.provider_auth_generation,
        provider_behavior,
        cwd: args.cwd.display().to_string(),
        agent_session_id: args.resume.clone(),
        system: args.system,
        context_window: context_budget.map(|value| value.context_window),
        auto_compact_token_limit: context_budget.map(|value| value.auto_compact_token_limit),
        cache_protection,
        generation: args.generation.clone(),
        fallback_for: args.fallback_for.clone(),
        adopt_only: false,
        execution_binding: args.execution_binding.clone(),
        extensions: None,
        mcp_plan: args.mcp_plan.clone(),
        native_fork: args.native_fork.clone().map(Box::new),
    };
    let (notify_tx, notify_rx) = mpsc::unbounded_channel();
    let (endpoint, managed_round) = execution_launch::prepare(
        &args,
        &launch,
        &prepared,
        managed_child.is_some(),
        &epoch,
        &notify_tx,
        &mut spec,
    )
    .await?;
    let expected_workspace_identity = workspace_identity(&args.cwd);
    let collector =
        request_observation::Collector::start(&mut spec, &args, &prepared, &epoch).await;
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
            // A resumed thread already exists on disk; a fresh one does not
            // until its first prompt.
            native_thread_materialized: Some(args.resume.is_some()),
            current_turn_id: None,
            supports_current_turn: false,
            delivery_controls: crate::runtime_wire::DeliveryControls {
                version: 1,
                next_checkpoint: false,
            },
            // This worker decodes and executes the command itself; whether a
            // given subagent may be stopped is decided per child in ACP.
            supports_subagent_cancel: true,
            last_runtime_seq: 0,
            pending_permissions: Vec::new(),
            config_options: None,
            context_used: Some(0),
            context_size: Some(0),
            pending_prompt_count: 0,
            drain_requested: false,
            exit_detail: None,
            background_tasks: Some(0),
            native_cycle_id: None,
            incarnation: None,
            account_unit: None,
            uncertain_effect: None,
        }),
        outbox: Mutex::new(BTreeMap::new()),
        trace_outbox: Mutex::default(),
        telemetry: Mutex::default(),
        request_observer: collector
            .as_ref()
            .map(|collector| collector.observer.clone()),
        notify: notify_tx,
        seen_commands: Mutex::new(HashSet::new()),
        prompt_commands: Mutex::new(VecDeque::new()),
        echo_artifacts: Mutex::new(VecDeque::new()),
        workspace_path: args.cwd.clone(),
        workspace_identity: expected_workspace_identity,
        execution: endpoint
            .as_ref()
            .map(|endpoint| Arc::clone(&endpoint.client)),
        managed_round,
        preset_options: prepared.preset_options.clone(),
        observation: Observation::default(),
    });
    execution_launch::observe(&shared);
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (done_tx, mut done_rx) = mpsc::channel(1);
    let thread_shared = Arc::clone(&shared);
    let session_id = args.session_id.clone();
    let cwd = args.cwd;
    let start = match (args.resume, args.native_fork) {
        (Some(resume), _) => acp::NativeStart::Resume(resume),
        (None, Some(fork)) => acp::NativeStart::Fork(fork),
        (None, None) => acp::NativeStart::New,
    };
    let service_auth_projected = args.provider_auth_generation.is_some();
    let runtime = tokio::runtime::Handle::current();
    std::thread::Builder::new()
        .name(format!("acp-worker-{session_id}"))
        .spawn(move || {
            let sink: Arc<dyn AgentSink> = Arc::new(RemoteSink {
                shared: Arc::clone(&thread_shared),
            });
            acp::run_agent_with_sink(
                &spec,
                &session_id,
                cwd,
                start,
                service_auth_projected,
                cmd_rx,
                &sink,
            );
            // Unit loss and teardown end every recorded target process
            // while the Machine connection still carries the requests.
            if let Some(execution) = thread_shared.execution.clone() {
                runtime.block_on(execution_launch::settle(execution));
            }
            let _ = done_tx.blocking_send(());
        })
        .context("spawning ACP worker thread")?;

    let result = connection_loop(&args.socket, shared, cmd_tx, notify_rx, &mut done_rx).await;
    // Keep exact session sidecars alive through ACP shutdown and outbox drain.
    // Dropping their kill-on-drop handles then tears down the full Provider
    // process tree before the worker exits.
    drop(prepared.sidecars);
    drop(endpoint);
    result
}

async fn connection_loop(
    socket: &PathBuf,
    shared: Arc<Shared>,
    cmd_tx: mpsc::UnboundedSender<AgentCommand>,
    mut notify_rx: mpsc::UnboundedReceiver<()>,
    done_rx: &mut mpsc::Receiver<()>,
) -> Result<()> {
    let mut backoff = BROKER_RECONNECT_INITIAL_DELAY;
    let mut cmd_tx = Some(cmd_tx);
    loop {
        let stream = match UnixStream::connect(socket).await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(
                    reason = crate::logs::error_reason(&error),
                    error = %error,
                    socket = %socket.display(),
                    retry_after_ms = backoff.as_millis(),
                    "Machine broker unavailable; worker keeps running"
                );
                wait_for_broker_retry(&mut backoff).await;
                continue;
            }
        };
        let connected_at = Instant::now();
        let outcome = connected(
            stream,
            Arc::clone(&shared),
            &mut cmd_tx,
            &mut notify_rx,
            done_rx,
        )
        .await;
        if connected_at.elapsed() >= BROKER_CONNECTION_STABLE_AFTER {
            backoff = BROKER_RECONNECT_INITIAL_DELAY;
        }
        match outcome {
            Ok(ConnectedExit::WorkerDrained) => return Ok(()),
            Ok(ConnectedExit::Rejected { reason }) => {
                anyhow::bail!("Machine broker rejected worker: {reason}")
            }
            Ok(ConnectedExit::Disconnected) => {
                tracing::warn!(
                    reason = "broker_closed",
                    retry_after_ms = backoff.as_millis(),
                    "Machine broker connection closed; reconnecting"
                );
            }
            Err(error) => {
                tracing::warn!(
                    reason = crate::logs::failure_reason(&error),
                    error = %error,
                    retry_after_ms = backoff.as_millis(),
                    "Machine broker connection lost; reconnecting"
                );
            }
        }
        wait_for_broker_retry(&mut backoff).await;
    }
}

async fn wait_for_broker_retry(backoff: &mut Duration) {
    tokio::time::sleep(*backoff).await;
    *backoff = (*backoff * 2).min(BROKER_RECONNECT_MAX_DELAY);
}

enum ConnectedExit {
    Disconnected,
    WorkerDrained,
    Rejected { reason: String },
}

async fn connected(
    stream: UnixStream,
    shared: Arc<Shared>,
    cmd_tx: &mut Option<mpsc::UnboundedSender<AgentCommand>>,
    notify_rx: &mut mpsc::UnboundedReceiver<()>,
    done_rx: &mut mpsc::Receiver<()>,
) -> Result<ConnectedExit> {
    let (mut reader, mut writer) = stream.into_split();
    let minimum_protocol = shared.snapshot.lock().launch.as_ref().map_or(
        if shared.execution.is_some() {
            2
        } else {
            MIN_PROTOCOL_VERSION
        },
        crate::runtime_wire::StartSession::minimum_protocol,
    );
    write_frame(
        &mut writer,
        &Frame::Hello {
            role: PeerRole::Worker,
            min_protocol: minimum_protocol,
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
        Some(Frame::Welcome { protocol, .. }) => {
            anyhow::ensure!(
                (minimum_protocol..=PROTOCOL_VERSION).contains(&protocol),
                "Machine broker selected an unsupported runtime protocol"
            );
            shared
                .observation
                .peer_protocol
                .store(protocol, Ordering::Relaxed);
        }
        Some(Frame::Reject { reason }) => return Ok(ConnectedExit::Rejected { reason }),
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
    // `done_rx.recv()` becomes immediately ready forever after its sender is
    // dropped. Poll it exactly once; otherwise a completed ACP thread with an
    // unacknowledged final event turns this select loop into a CPU spin and can
    // starve the broker ACK that would let the worker drain.
    let mut agent_done = false;
    if let Some(execution) = &shared.execution {
        for frame in execution.frames(true) {
            write_frame(&mut writer, &frame).await?;
        }
    }
    send_outbox(&shared, &mut writer, &mut last_sent).await?;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            incoming = reader.next() => {
                let Some(frame) = incoming? else {
                    return Ok(ConnectedExit::Disconnected);
                };
                match frame {
                    Frame::ExecutionReply { reply } => {
                        if let Some(execution) = &shared.execution { execution.complete(*reply); }
                    }
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
                        if agent_done && shared.outbox.lock().is_empty() {
                            return Ok(ConnectedExit::WorkerDrained);
                        }
                    }
                    Frame::Replay { session_id, worker_epoch, after_runtime_seq }
                        if session_id == shared.session_id && worker_epoch == shared.worker_epoch => {
                        last_sent = after_runtime_seq;
                        // The broker survives a Controller reconnect, so our
                        // local socket never reconnects. Replay pending target
                        // calls too instead of waiting for their 30s retry.
                        // frames(true) retains request and effect identities.
                        if let Some(execution) = &shared.execution {
                            for frame in execution.frames(true) {
                                write_frame(&mut writer, &frame).await?;
                            }
                        }
                        send_outbox(&shared, &mut writer, &mut last_sent).await?;
                    }
                    Frame::Heartbeat => {}
                    Frame::Reject { reason } => {
                        return Ok(ConnectedExit::Rejected { reason });
                    }
                    other => tracing::debug!(?other, "ignoring unrelated runtime frame"),
                }
            }
            notified = notify_rx.recv() => {
                if notified.is_none()
                    && agent_done
                    && shared.outbox.lock().is_empty()
                {
                    return Ok(ConnectedExit::WorkerDrained);
                }
                send_outbox(&shared, &mut writer, &mut last_sent).await?;
            }
            _ = done_rx.recv(), if !agent_done => {
                // `None` is completion too: it means the ACP thread dropped
                // its final sender (including a panic path). Disable this
                // branch before looping so a closed receiver cannot stay hot.
                agent_done = true;
                if shared.outbox.lock().is_empty() {
                    return Ok(ConnectedExit::WorkerDrained);
                }
                send_outbox(&shared, &mut writer, &mut last_sent).await?;
            }
            _ = heartbeat.tick() => {
                write_frame(&mut writer, &Frame::Heartbeat).await?;
                send_outbox(&shared, &mut writer, &mut last_sent).await?;
            }
        }
    }
}

pub async fn send_outbox<W: tokio::io::AsyncWrite + Unpin>(
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
    let legacy = shared.observation.peer_protocol.load(Ordering::Relaxed)
        < crate::runtime_wire::SUBAGENT_OBSERVATION_PROTOCOL_VERSION;
    for (seq, mut event) in pending {
        if legacy {
            // An older broker would hand it to a Controller that stores it.
            crate::runtime_wire::retire_observation(&mut event);
        }
        let diagnostics = shared
            .trace_outbox
            .lock()
            .get(&seq)
            .cloned()
            .unwrap_or_default();
        write_frame(
            writer,
            &Frame::WorkerEvent {
                session_id: shared.session_id.clone(),
                worker_epoch: shared.worker_epoch.clone(),
                runtime_seq: seq,
                event,
                diagnostics,
            },
        )
        .await?;
        *last_sent = seq;
    }
    if let Some(execution) = &shared.execution {
        for frame in execution.frames(false) {
            write_frame(writer, &frame).await?;
        }
    }
    Ok(())
}

pub fn handle_command(
    shared: &Shared,
    cmd_tx: &mut Option<mpsc::UnboundedSender<AgentCommand>>,
    command: WorkerCommand,
) -> Frame {
    let (command_id, agent_command, turn): (String, Option<AgentCommand>, Option<String>) =
        match command {
            WorkerCommand::CancelTurn {
                command_id,
                target,
                cmid,
            } => {
                let snapshot = shared.snapshot();
                if snapshot.worker_epoch != target.worker_epoch
                    || snapshot.current_turn_id.as_deref() != Some(&target.turn_id)
                    || snapshot.drain_requested
                    || shared.managed_read_only()
                    || !shared.workspace_is_current()
                {
                    return command_ack(
                        shared,
                        command_id,
                        false,
                        Some("cancellation target expired".into()),
                    );
                }
                if !shared.mark_command(&command_id) {
                    return command_ack(shared, command_id, true, Some("duplicate".into()));
                }
                (
                    command_id,
                    Some(AgentCommand::CancelTurn { target, cmid }),
                    None,
                )
            }
            WorkerCommand::Steer {
                command_id,
                worker_epoch,
                turn_id,
                content,
                cmid,
                next_checkpoint,
            } => {
                // This command must never wait for a future worker or turn.
                let snapshot = shared.snapshot();
                if !snapshot.supports_current_turn
                    || (next_checkpoint && !snapshot.delivery_controls.next_checkpoint)
                    || !shared.workspace_is_current()
                    || shared.managed_read_only()
                    || snapshot.worker_epoch != worker_epoch
                    || snapshot.delivery_turn_id() != Some(turn_id.as_str())
                    || snapshot.drain_requested
                {
                    return command_ack(
                        shared,
                        command_id,
                        false,
                        Some("current turn is unavailable".into()),
                    );
                }
                let Ok(blocks) = content
                    .into_iter()
                    .map(serde_json::from_value)
                    .collect::<Result<Vec<ContentBlock>, _>>()
                else {
                    return command_ack(
                        shared,
                        command_id,
                        false,
                        Some("invalid steering content".into()),
                    );
                };
                if blocks.is_empty() {
                    return command_ack(
                        shared,
                        command_id,
                        false,
                        Some("empty steering content".into()),
                    );
                }
                if !shared.mark_command(&command_id) {
                    return command_ack(shared, command_id, true, Some("duplicate".into()));
                }
                (
                    command_id,
                    Some(AgentCommand::Steer {
                        blocks,
                        cmid,
                        next_checkpoint,
                        target: Some(crate::send_delivery::DeliveryTarget {
                            worker_epoch,
                            turn_id,
                        }),
                    }),
                    None,
                )
            }
            WorkerCommand::Prompt {
                command_id,
                turn_id,
                content,
                cmid,
                trace,
                echo_artifacts,
                observe_subagents,
            } => {
                // Each turn's Controller states whether it keeps observed
                // transcripts, so a rolled-back Controller stops receiving them.
                shared
                    .observation
                    .enabled
                    .store(observe_subagents, Ordering::Relaxed);
                if let Some(message) = shared.workspace_replaced() {
                    return command_ack(shared, command_id, false, Some(message));
                }
                if !shared.mark_command(&command_id) {
                    return command_ack(shared, command_id, true, Some("duplicate".to_owned()));
                }
                let blocks = decoded_prompt(shared, content, echo_artifacts);
                if blocks.is_empty() {
                    return command_ack(
                        shared,
                        command_id,
                        false,
                        Some("prompt has no valid content blocks".to_owned()),
                    );
                }
                shared.telemetry.lock().admit(cmid.as_deref(), trace);
                shared.record_prompt_command(cmid.clone(), command_id.clone());
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
            WorkerCommand::CancelSubagent {
                command_id,
                subagent_session_id,
            } => {
                if !shared.mark_command(&command_id) {
                    return command_ack(shared, command_id, true, Some("duplicate".to_owned()));
                }
                (
                    command_id,
                    Some(AgentCommand::CancelSubagent {
                        subagent_session_id,
                    }),
                    None,
                )
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
                // The parent's preset is the one configuration a managed child
                // takes; the ACP command loop applies the same allowlist.
                if shared.managed_read_only() && !shared.preset_options.contains(&config_id) {
                    return command_ack(
                        shared,
                        command_id,
                        false,
                        Some("managed review configuration is immutable".into()),
                    );
                }
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
                    shared.forward(&tx, AgentCommand::Cancel);
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
        (Some(command), Some(tx)) => shared.forward(tx, command),
        (Some(_), None) => false,
        (None, _) => true,
    };
    if sent {
        command_ack(shared, command_id, true, None)
    } else {
        // A reconnect may safely retry a command that never reached ACP.
        shared.unmark_command(&command_id);
        shared.remove_prompt_command_id(&command_id);
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

fn decoded_prompt(
    shared: &Shared,
    content: Vec<serde_json::Value>,
    echo_artifacts: bool,
) -> Vec<ContentBlock> {
    if echo_artifacts {
        shared.remember_echo_artifacts(&content);
    }
    if let Some(observer) = &shared.request_observer {
        observer.prompt(&content);
    }
    content
        .into_iter()
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect()
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

#[cfg(any(test, feature = "test-support"))]
pub fn shared() -> (Arc<Shared>, mpsc::UnboundedReceiver<()>) {
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
                native_thread_materialized: None,
                current_turn_id: None,
                supports_current_turn: false,
                delivery_controls: crate::runtime_wire::DeliveryControls::default(),
                supports_subagent_cancel: false,
                last_runtime_seq: 0,
                pending_permissions: vec![],
                config_options: None,
                context_used: Some(0),
                context_size: Some(0),
                pending_prompt_count: 0,
                drain_requested: false,
                exit_detail: None,
                background_tasks: None,
                native_cycle_id: None,
                incarnation: None,
                account_unit: None,
                uncertain_effect: None,
            }),
            outbox: Mutex::new(BTreeMap::new()),
            trace_outbox: Mutex::default(),
            telemetry: Mutex::default(),
            request_observer: None,
            notify: tx,
            seen_commands: Mutex::new(HashSet::new()),
            prompt_commands: Mutex::new(VecDeque::new()),
            echo_artifacts: Mutex::new(VecDeque::new()),
            workspace_path,
            workspace_identity: expected_workspace_identity,
            execution: None,
            managed_round: None,
            preset_options: std::collections::BTreeSet::new(),
            observation: Observation::default(),
        }),
        rx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn sigterm_ends_a_running_worker_cleanly() {
        let stopped = until_terminated("sess-test".into(), std::future::pending()).unwrap();
        rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::TERM)
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), stopped)
            .await
            .expect("SIGTERM did not end the worker")
            .unwrap();
    }

    #[tokio::test]
    async fn an_older_broker_never_receives_an_observed_update() {
        let (shared, _rx) = shared();
        shared.observation.enabled.store(true, Ordering::Relaxed);
        let sink = RemoteSink {
            shared: Arc::clone(&shared),
        };
        sink.observe_subagent(
            "sess-1",
            "child",
            serde_json::json!({"sessionUpdate": "tool_call"}),
        );
        for (protocol, expected) in [
            (6, crate::runtime_wire::STEERING_RESULT_UPDATE),
            (7, crate::runtime_wire::SUBAGENT_EVENT_UPDATE),
        ] {
            shared
                .observation
                .peer_protocol
                .store(protocol, Ordering::Relaxed);
            let mut bytes = Vec::new();
            let mut last_sent = 0;
            send_outbox(&shared, &mut bytes, &mut last_sent)
                .await
                .unwrap();
            let frame = crate::runtime_wire::read_frame(&mut bytes.as_slice())
                .await
                .unwrap();
            let Some(Frame::WorkerEvent {
                event: RuntimeEvent::Update { update, .. },
                runtime_seq: 1,
                ..
            }) = frame
            else {
                panic!("the observed update keeps its runtime sequence");
            };
            assert_eq!(update["sessionUpdate"], expected);
        }
    }

    #[test]
    fn observed_child_updates_follow_the_prompt_flag_and_never_grow_a_full_outbox() {
        let (shared, _rx) = shared();
        let sink = RemoteSink {
            shared: Arc::clone(&shared),
        };
        let update = || serde_json::json!({"sessionUpdate": "tool_call", "toolCallId": "t"});
        sink.observe_subagent("sess-1", "child", update());
        assert!(shared.outbox.lock().is_empty(), "no flag, no observation");
        shared.observation.enabled.store(true, Ordering::Relaxed);
        for _ in 0..OBSERVE_OUTBOX_LIMIT {
            shared.emit(RuntimeEvent::Error {
                message: "unacknowledged".to_owned(),
            });
        }
        sink.observe_subagent("sess-1", "child", update());
        sink.observe_subagent("sess-1", "child", update());
        assert_eq!(shared.outbox.lock().len(), OBSERVE_OUTBOX_LIMIT);
        shared.ack(u64::MAX);
        sink.observe_subagent("sess-1", "child", update());
        let outbox = shared.outbox.lock();
        let Some(RuntimeEvent::Update { update, .. }) = outbox.values().next() else {
            panic!("observed update was not emitted");
        };
        assert_eq!(
            update["sessionUpdate"],
            crate::runtime_wire::SUBAGENT_EVENT_UPDATE
        );
        assert_eq!(update["childSessionId"], "child");
        assert_eq!(update["dropped"], 2);
        assert_eq!(update["update"]["toolCallId"], "t");
        drop(outbox);
        assert!(shared.observation.dropped.lock().is_empty());
    }

    #[test]
    fn managed_read_only_blocks_configuration_at_worker_admission_and_projection() {
        let (shared, _notify) = shared();
        let sink = RemoteSink {
            shared: Arc::clone(&shared),
        };
        let options = serde_json::json!([{"id":"approval_policy","currentValue":"full-access"}]);
        sink.set_config_options("sess-1", options.clone());
        assert_eq!(shared.snapshot().config_options, Some(options.clone()));
        shared.snapshot.lock().launch = Some(
            serde_json::from_value(serde_json::json!({
                "session_id":"sess-1", "provider":"codex", "cwd":"/snapshot", "generation":"gen-1",
                "execution_binding":{
                    "schema":1,"phase":"managed_child","session_id":"sess-1",
                    "parent_session_id":"parent-1","machine_id":"hawk","workspace_id":"cowboy",
                    "cwd":"/snapshot","profile":"read_only_v1"
                }
            }))
            .unwrap(),
        );
        assert!(sink.session_is_managed_read_only("sess-1"));
        assert!(!sink.session_is_managed_read_only("other-session"));
        sink.set_config_options("sess-1", options);
        assert_eq!(
            shared.snapshot().config_options,
            Some(serde_json::json!([]))
        );
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut tx = Some(tx);
        for _ in 0..2 {
            let ack = handle_command(
                &shared,
                &mut tx,
                WorkerCommand::SetConfigOption {
                    command_id: "widen-permissions".into(),
                    config_id: "approval_policy".into(),
                    value: serde_json::json!("full-access"),
                },
            );
            assert!(matches!(
                ack,
                Frame::CommandAck {
                    accepted: false,
                    ..
                }
            ));
            assert!(rx.try_recv().is_err());
        }
        let ack = handle_command(
            &shared,
            &mut tx,
            WorkerCommand::Cancel {
                command_id: "stop".into(),
            },
        );
        assert!(matches!(ack, Frame::CommandAck { accepted: true, .. }));
        assert!(matches!(rx.try_recv(), Ok(AgentCommand::Cancel)));
    }

    #[test]
    fn a_named_native_cycle_is_a_steering_target_only_without_a_cowboy_turn() {
        let (shared, _notify) = shared();
        shared.snapshot.lock().supports_current_turn = true;
        let sink = RemoteSink {
            shared: Arc::clone(&shared),
        };
        let target = |turn: &str| crate::send_delivery::DeliveryTarget {
            worker_epoch: "epoch-1".into(),
            turn_id: turn.into(),
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut tx = Some(tx);
        let mut steer = |id: &str, turn: &str| {
            handle_command(
                &shared,
                &mut tx,
                WorkerCommand::Steer {
                    command_id: id.into(),
                    worker_epoch: "epoch-1".into(),
                    turn_id: turn.into(),
                    content: vec![serde_json::json!({"type":"text","text":"correction"})],
                    cmid: id.into(),
                    next_checkpoint: false,
                },
            )
        };
        let accepted = |frame: &Frame| matches!(frame, Frame::CommandAck { accepted: true, .. });
        assert!(!accepted(&steer("none", "native-a")));

        sink.set_native_cycle("sess-1", Some("native-a"));
        assert_eq!(
            shared.snapshot().native_cycle_id.as_deref(),
            Some("native-a")
        );
        assert!(sink.delivery_target_is_current("sess-1", &target("native-a")));
        assert!(!sink.delivery_target_is_current("sess-1", &target("native-b")));
        assert!(!accepted(&steer("other", "native-b")));
        assert!(accepted(&steer("join", "native-a")));
        assert!(matches!(
            rx.try_recv(),
            Ok(AgentCommand::Steer { target: Some(target), .. }) if target.turn_id == "native-a"
        ));

        // A Cowboy turn owns delivery: the cycle's name no longer addresses it.
        shared.emit(RuntimeEvent::TurnStarted {
            turn_id: "turn-1".into(),
            command_id: "command-1".into(),
        });
        assert_eq!(shared.snapshot().native_cycle_id, None);
        assert!(!accepted(&steer("late", "native-a")));

        sink.set_native_cycle("sess-1", Some("native-c"));
        shared.emit(RuntimeEvent::Status {
            state: WorkerState::Crashed,
            detail: None,
        });
        assert_eq!(shared.snapshot().native_cycle_id, None);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn managed_read_only_admits_only_the_preset_options() {
        let (mut shared, _notify) = shared();
        Arc::get_mut(&mut shared)
            .unwrap()
            .preset_options
            .insert("model".to_owned());
        shared.snapshot.lock().launch = Some(
            serde_json::from_value(serde_json::json!({
                "session_id":"sess-1", "provider":"codex", "cwd":"/snapshot", "generation":"gen-1",
                "execution_binding":{
                    "schema":1,"phase":"managed_child","session_id":"sess-1",
                    "parent_session_id":"parent-1","machine_id":"hawk","workspace_id":"cowboy",
                    "cwd":"/snapshot","profile":"read_only_v1"
                }
            }))
            .unwrap(),
        );
        let sink = RemoteSink {
            shared: Arc::clone(&shared),
        };
        sink.set_config_options(
            "sess-1",
            serde_json::json!([
                {"id":"model","currentValue":"chosen"},
                {"id":"approval_policy","currentValue":"full-access"}
            ]),
        );
        assert_eq!(
            shared.snapshot().config_options,
            Some(serde_json::json!([
                {"id":"model","currentValue":"chosen"}
            ]))
        );
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut tx = Some(tx);
        let mut set = |command_id: &str, config_id: &str| {
            handle_command(
                &shared,
                &mut tx,
                WorkerCommand::SetConfigOption {
                    command_id: command_id.into(),
                    config_id: config_id.into(),
                    value: serde_json::json!("value"),
                },
            )
        };
        assert!(matches!(
            set("preset-model", "model"),
            Frame::CommandAck { accepted: true, .. }
        ));
        assert!(matches!(
            set("widen-permissions", "approval_policy"),
            Frame::CommandAck {
                accepted: false,
                ..
            }
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(AgentCommand::SetConfigOption { config_id, .. }) if config_id == "model"
        ));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn trace_outbox_is_bounded_without_blocking_events_and_ack_releases_capacity() {
        use crate::runtime_trace::{TraceCarrier, TraceContext};

        let (shared, _notify) = shared();
        let turn = |id: &str| {
            let trace = TraceCarrier::new(
                TraceContext::from_browser(
                    "00-11111111111111111111111111111111-2222222222222222-01",
                )
                .unwrap(),
            );
            {
                let mut telemetry = shared.telemetry.lock();
                telemetry.admit(Some(id), Some(trace));
                telemetry.started(Some(id));
                telemetry.completed(Some(id), "EndTurn");
            }
            shared.emit(RuntimeEvent::TurnEnded {
                usage: None,
                turn_id: id.into(),
                stop_reason: "EndTurn".into(),
            })
        };
        for i in 0..256 {
            turn(&i.to_string());
        }
        assert_eq!(shared.trace_outbox.lock().len(), 128);
        assert!(
            shared
                .trace_outbox
                .lock()
                .values()
                .all(|spans| spans.len() <= 4)
        );
        assert_eq!(
            shared.outbox.lock().len(),
            256,
            "business events survive diagnostic saturation"
        );
        let last_traced = *shared.trace_outbox.lock().last_key_value().unwrap().0;
        shared.ack(last_traced);
        assert!(shared.trace_outbox.lock().is_empty());
        assert_eq!(shared.outbox.lock().len(), 128);
        let seq = turn("fresh");
        assert_eq!(shared.trace_outbox.lock().len(), 1);
        shared.ack(seq);
        assert!(shared.outbox.lock().is_empty());
        assert!(shared.trace_outbox.lock().is_empty());
    }

    use super::shared;

    #[tokio::test]
    async fn controller_replay_resends_pending_execution_without_worker_reconnect() {
        use futures::SinkExt as _;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        let root = tempfile::Builder::new()
            .prefix("cw-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = root.path().join("broker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let (mut shared, notify_rx) = shared();
        let binding: crate::execution_environment::BindingV1 = serde_json::from_value(
            serde_json::json!({
                "schema": 1, "id": "binding", "revision": 1,
                "runtime": {"machine_id": "runtime", "cwd": "/runtime"},
                "environment": {"machine_id": "target", "id": "environment",
                    "incarnation": "incarnation", "executor_digest": format!("sha256:{}", "a".repeat(64)), "protocol": 1},
                "workspace": {"id": "project", "worktree_id": "session", "source_path": "/source", "cwd": "/target"},
                "access": "project"
            }),
        ).unwrap();
        let execution = crate::worker_execution::Client::new(
            "sess-1".into(),
            "epoch-1".into(),
            binding,
            shared.notify.clone(),
        );
        let endpoint =
            crate::worker_execution::Endpoint::start(Arc::clone(&execution), root.path())
                .await
                .unwrap();
        Arc::get_mut(&mut shared).unwrap().execution = Some(execution);
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_done_tx, mut done_rx) = mpsc::channel(1);
        let worker = tokio::spawn(async move {
            connection_loop(&socket, shared, cmd_tx, notify_rx, &mut done_rx).await
        });
        let (mut broker, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut broker).await.unwrap(),
            Some(Frame::Hello { .. })
        ));
        write_frame(
            &mut broker,
            &Frame::Welcome {
                protocol: PROTOCOL_VERSION,
                controller_epoch: 1,
                workers: vec![],
            },
        )
        .await
        .unwrap();
        let descriptor: serde_json::Value =
            serde_json::from_slice(&std::fs::read(endpoint.descriptor().unwrap()).unwrap())
                .unwrap();
        let mut request = descriptor["endpoint"]
            .as_str()
            .unwrap()
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {}", descriptor["bearer_token"].as_str().unwrap())
                .parse()
                .unwrap(),
        );
        let (mut native, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        native
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({"id": 1, "method": "initialize", "params": {}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let initial = loop {
            let frame = read_frame(&mut broker).await.unwrap().unwrap();
            if matches!(frame, Frame::ExecutionRequest { .. }) {
                break frame;
            }
        };
        // Drop the reply upstream while keeping the worker's socket alive.
        // This is the Replay the broker sends when a Controller reattaches.
        write_frame(
            &mut broker,
            &Frame::Replay {
                session_id: "sess-1".into(),
                worker_epoch: "epoch-1".into(),
                after_runtime_seq: 0,
            },
        )
        .await
        .unwrap();
        let replay = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let frame = read_frame(&mut broker).await.unwrap().unwrap();
                if matches!(frame, Frame::ExecutionRequest { .. }) {
                    break frame;
                }
            }
        })
        .await
        .expect("Controller reconnect must not wait for the 30s retry");
        assert_eq!(replay, initial, "preserve the original request identity");
        worker.abort();
        let _ = worker.await;
    }

    #[tokio::test]
    async fn broker_rejection_is_terminal_and_is_not_retried() {
        // macOS limits Unix-domain socket paths to 103 bytes. TMPDIR commonly
        // points into a long /var/folders path there, so keep this hermetic
        // broker fixture under the short, standard Unix temporary directory.
        let root = PathBuf::from("/tmp").join(format!("cw-{}", generated_epoch()));
        std::fs::create_dir_all(&root).expect("create socket directory");
        let socket = root.join("broker.sock");
        let listener = UnixListener::bind(&socket).expect("bind broker socket");
        let broker = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept worker");
            assert!(matches!(
                read_frame(&mut stream).await.expect("read worker hello"),
                Some(Frame::Hello {
                    role: PeerRole::Worker,
                    ..
                })
            ));
            write_frame(
                &mut stream,
                &Frame::Reject {
                    reason: "duplicate worker epoch".to_owned(),
                },
            )
            .await
            .expect("reject worker");
            assert!(
                tokio::time::timeout(Duration::from_millis(250), listener.accept())
                    .await
                    .is_err(),
                "a rejected worker must not reconnect"
            );
        });

        let (shared, notify_rx) = shared();
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_done_tx, mut done_rx) = mpsc::channel(1);
        let error = tokio::time::timeout(
            Duration::from_secs(1),
            connection_loop(&socket, shared, cmd_tx, notify_rx, &mut done_rx),
        )
        .await
        .expect("rejected worker exits promptly")
        .expect_err("broker rejection is terminal");
        assert!(
            error
                .to_string()
                .contains("Machine broker rejected worker: duplicate worker epoch")
        );
        broker.await.expect("broker task");
        std::fs::remove_file(&socket).expect("remove broker socket");
        std::fs::remove_dir(&root).expect("remove socket directory");
    }

    #[tokio::test]
    async fn stop_with_a_final_event_drains_once_without_reconnecting() {
        let root = PathBuf::from("/tmp").join(format!("cw-{}", generated_epoch()));
        std::fs::create_dir_all(&root).expect("create socket directory");
        let socket = root.join("broker.sock");
        let listener = UnixListener::bind(&socket).expect("bind broker socket");
        let broker = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept worker");
            assert!(matches!(
                read_frame(&mut stream).await.expect("read worker hello"),
                Some(Frame::Hello {
                    role: PeerRole::Worker,
                    ..
                })
            ));
            write_frame(
                &mut stream,
                &Frame::Welcome {
                    protocol: PROTOCOL_VERSION,
                    controller_epoch: 1,
                    workers: Vec::new(),
                },
            )
            .await
            .expect("welcome worker");
            assert!(matches!(
                read_frame(&mut stream).await.expect("read worker snapshot"),
                Some(Frame::Snapshot { .. })
            ));
            write_frame(
                &mut stream,
                &Frame::WorkerCommand {
                    session_id: "sess-1".to_owned(),
                    command: WorkerCommand::Stop {
                        command_id: "delete-sess-1".to_owned(),
                    },
                },
            )
            .await
            .expect("stop worker");

            let mut saw_final_event = false;
            let mut saw_stop_ack = false;
            loop {
                let frame = tokio::time::timeout(Duration::from_secs(1), read_frame(&mut stream))
                    .await
                    .expect("stopped worker frame timeout")
                    .expect("read stopped worker frame");
                let Some(frame) = frame else { break };
                match frame {
                    Frame::WorkerEvent {
                        session_id,
                        worker_epoch,
                        runtime_seq,
                        event:
                            RuntimeEvent::Status {
                                state: WorkerState::Exited,
                                ..
                            },
                        ..
                    } => {
                        saw_final_event = true;
                        write_frame(
                            &mut stream,
                            &Frame::Ack {
                                session_id,
                                worker_epoch,
                                runtime_seq,
                            },
                        )
                        .await
                        .expect("ack final worker event");
                    }
                    Frame::CommandAck {
                        command_id,
                        accepted: true,
                        ..
                    } if command_id == "delete-sess-1" => saw_stop_ack = true,
                    _ => {}
                }
            }
            assert!(saw_final_event, "stopped worker must publish Exited");
            assert!(saw_stop_ack, "stopped worker must ACK Stop");
            assert!(
                tokio::time::timeout(Duration::from_millis(250), listener.accept())
                    .await
                    .is_err(),
                "a drained worker must not reconnect"
            );
        });

        let (shared, notify_rx) = shared();
        let acp_shared = Arc::clone(&shared);
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
        let (done_tx, mut done_rx) = mpsc::channel(1);
        let acp = tokio::spawn(async move {
            assert!(matches!(cmd_rx.recv().await, Some(AgentCommand::Cancel)));
            acp_shared.emit(RuntimeEvent::Status {
                state: WorkerState::Exited,
                detail: None,
            });
            done_tx.send(()).await.expect("signal ACP completion");
        });
        tokio::time::timeout(
            Duration::from_secs(2),
            connection_loop(&socket, shared, cmd_tx, notify_rx, &mut done_rx),
        )
        .await
        .expect("stopped worker exits promptly")
        .expect("stopped worker drains cleanly");
        acp.await.expect("ACP simulation");
        broker.await.expect("broker task");
        std::fs::remove_file(&socket).expect("remove broker socket");
        std::fs::remove_dir(&root).expect("remove socket directory");
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
                trace: None,
                turn_id: "turn-1".to_owned(),
                content: vec![serde_json::json!({"type": "text", "text": "hello"})],
                cmid: None,
                echo_artifacts: false,
                observe_subagents: false,
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
    fn prompt_echo_references_images_the_controller_already_stored() {
        use base64::Engine as _;

        let (shared, _notify) = shared();
        let (tx, _rx) = mpsc::unbounded_channel();
        let image = |byte: u8| {
            serde_json::json!({
                "type": "image",
                "mimeType": "image/jpeg",
                "data": base64::engine::general_purpose::STANDARD.encode(vec![byte; 40_000]),
            })
        };
        handle_command(
            &shared,
            &mut Some(tx),
            WorkerCommand::Prompt {
                command_id: "cmd-1".to_owned(),
                trace: None,
                turn_id: "turn-1".to_owned(),
                content: vec![image(1)],
                cmid: Some("c-1".to_owned()),
                echo_artifacts: true,
                observe_subagents: false,
            },
        );
        let sink = RemoteSink {
            shared: Arc::clone(&shared),
        };
        let echo = |content: serde_json::Value| Event::Update {
            update: serde_json::json!({"sessionUpdate": "user_message_chunk", "content": content}),
        };
        sink.push_tagged("sess-1", echo(image(1)), Some("c-1".to_owned()));
        sink.push_tagged("sess-1", echo(image(2)), None);
        sink.push(
            "sess-1",
            Event::Update {
                update: serde_json::json!({"sessionUpdate": "tool_call_update", "content": image(1)}),
            },
        );

        let contents: Vec<serde_json::Value> = shared
            .outbox
            .lock()
            .values()
            .filter_map(|event| match event {
                RuntimeEvent::Update { update, .. } => Some(update["content"].clone()),
                _ => None,
            })
            .collect();
        assert_eq!(contents.len(), 3);
        assert!(contents[0].get("data").is_none());
        assert_eq!(
            contents[0][crate::artifacts::ECHO_ARTIFACT_FIELD],
            crate::artifacts::large_image_digest(image(1)["data"].as_str().unwrap()).unwrap()
        );
        assert_eq!(contents[1], image(2), "an unstored image stays inline");
        assert_eq!(contents[2], image(1), "only prompt echoes are referenced");
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
    fn reported_context_size_is_bounded_by_the_selected_session_budget() {
        let (shared, _rx) = shared();
        shared.snapshot.lock().launch = Some(crate::runtime_wire::StartSession {
            session_id: "sess-1".to_owned(),
            provider: "codex-deepseek".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: None,
            cwd: "/work".to_owned(),
            agent_session_id: None,
            system: false,
            context_window: Some(680_000),
            auto_compact_token_limit: Some(646_000),
            cache_protection: Some(true),
            generation: "gen-1".to_owned(),
            fallback_for: None,
            adopt_only: false,
            execution_binding: None,
            extensions: None,
            mcp_plan: None,
            native_fork: None,
        });
        let sink = RemoteSink {
            shared: Arc::clone(&shared),
        };
        sink.set_session_usage(
            "sess-1",
            crate::agent_model::SessionUsage {
                used: 320_000,
                size: 1_048_576,
                raw: serde_json::json!({"providerSize": 1_048_576}),
                observed_at_ms: 42,
            },
        );

        assert!(matches!(
            shared.outbox.lock().get(&1),
            Some(RuntimeEvent::ContextUsage {
                used: 320_000,
                size: 680_000,
                observed_at_ms: 42,
                ..
            })
        ));
        assert_eq!(shared.snapshot().context_size, Some(680_000));
    }

    #[test]
    fn requeue_prompt_emits_original_prompt_command_id() {
        let (shared, _rx) = shared();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut tx = Some(tx);
        let ack = handle_command(
            &shared,
            &mut tx,
            WorkerCommand::Prompt {
                command_id: "cmd-notice-42".to_owned(),
                trace: None,
                turn_id: "turn-42".to_owned(),
                content: vec![serde_json::json!({"type": "text", "text": "notice body"})],
                cmid: Some("call-notice:call-42".to_owned()),
                echo_artifacts: false,
                observe_subagents: false,
            },
        );
        assert!(matches!(ack, Frame::CommandAck { accepted: true, .. }));
        assert!(matches!(rx.try_recv(), Ok(AgentCommand::Prompt(..))));

        let sink = RemoteSink {
            shared: Arc::clone(&shared),
        };
        sink.requeue_prompt(
            "sess-1",
            "notice body".to_owned(),
            vec![],
            Some("call-notice:call-42".to_owned()),
        );
        let undelivered = shared.outbox.lock().values().find_map(|event| match event {
            RuntimeEvent::UndeliveredPrompt {
                command_id, cmid, ..
            } => Some((command_id.clone(), cmid.clone())),
            _ => None,
        });
        assert_eq!(
            undelivered,
            Some((
                "cmd-notice-42".to_owned(),
                Some("call-notice:call-42".to_owned()),
            )),
            "worker-produced UndeliveredPrompt must carry the original WorkerCommand::Prompt command_id"
        );
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
