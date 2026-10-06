//! Stable Machine-local broker for detached ACP workers.
//!
//! The broker deliberately contains no Cowboy business state and no ACP parsing.
//! It grants one controller lease, routes commands, starts session workers, and
//! lets workers replay their unacknowledged outboxes after either side restarts.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::os::fd::{AsFd as _, FromRawFd as _, OwnedFd};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[cfg(test)]
use std::sync::atomic::AtomicBool;

use anyhow::{Context as _, Result, ensure};
use parking_lot::Mutex;
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Command;
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};

use crate::runtime_wire::{
    CoreCommand, Frame, MIN_PROTOCOL_VERSION, PROTOCOL_VERSION, PeerRole, RuntimeEvent,
    StartSession, WorkerCommand, WorkerSnapshot, WorkerState, negotiate, read_frame, write_frame,
};

mod cleanups;
pub(crate) mod deletions;
mod incarnations;
mod namespace;

/// Consecutive failed cleanup attempts (about three minutes of backoff) after
/// which a nominated Session stops holding its handles in this process.
const CLEANUP_ATTEMPT_LIMIT: usize = 8;
const WORKER_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(45);
const WORKER_MONITOR_INTERVAL: Duration = Duration::from_secs(15);
/// Bytes still queued on the broker side prove the worker kept writing and the
/// broker fell behind. Isolating that worker cannot repair the broker, so wait
/// this much longer before treating the connection as unrecoverable.
const WORKER_BACKLOG_ISOLATION_LIMIT: Duration = Duration::from_secs(300);
const CORE_COMMAND_QUEUE_CAPACITY: usize = 64;
const TRANSIENT_UNIT_COLLECT_TIMEOUT: Duration = Duration::from_secs(10);
const DIRECT_WORKER_GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(3);
const DIRECT_WORKER_TERM_TIMEOUT: Duration = Duration::from_secs(2);
const DIRECT_WORKER_KILL_TIMEOUT: Duration = Duration::from_secs(1);
const DIRECT_WORKER_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Longest a worker start waits for a live Provider installation that fences
/// its Plugin; an upgrade normally resolves in about a minute.
const PROVIDER_INSTALL_LAUNCH_WAIT: Duration = Duration::from_secs(300);
const PROVIDER_INSTALL_LAUNCH_POLL: Duration = Duration::from_millis(250);

fn worker_generation_failure_allows_fallback(error: &anyhow::Error) -> bool {
    let detail = format!("{error:#}");
    // Auth, missing native threads, and restore timeouts are session-local.
    // Retrying a previous worker generation hydrates the same thread and hides the real
    // failure behind "fallback after generation launch failed".
    !crate::provider_behavior::is_provider_auth_required_error(&detail)
        && !crate::provider_behavior::is_native_session_restore_timeout(&detail)
        // Older workers do not attach the ACP method to this diagnostic. A
        // missing ACP resource cannot be repaired by selecting an older
        // worker, which may not even understand the exact Provider package.
        && !detail.contains("acp connection: Resource not found:")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnMode {
    /// Child process mode for macOS, development, and hermetic tests. Linux
    /// production uses user-systemd so workers survive broker restarts.
    Direct,
    SystemdUser,
}

fn direct_worker_is_current(
    workers: &Mutex<HashMap<String, u32>>,
    session_id: &str,
    pid: u32,
) -> bool {
    workers.lock().get(session_id).copied() == Some(pid)
}

async fn wait_for_direct_worker_exit(
    workers: &Mutex<HashMap<String, u32>>,
    session_id: &str,
    pid: u32,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if !direct_worker_is_current(workers, session_id, pid) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(DIRECT_WORKER_EXIT_POLL_INTERVAL).await;
    }
}

fn signal_direct_worker(
    workers: &Mutex<HashMap<String, u32>>,
    session_id: &str,
    pid: u32,
    signal: rustix::process::Signal,
) -> Result<()> {
    // Keep the original mapping borrowed through the syscall. No subprocess,
    // PATH lookup or await may separate this owner check from signal delivery.
    let workers = workers.lock();
    if workers.get(session_id).copied() != Some(pid) {
        return Ok(());
    }
    let group = crate::plugin_process::owned_group_id(pid).context("invalid owned worker group")?;
    match rustix::process::kill_process_group(group, signal) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("signalling direct worker {session_id} group {pid}"))
        }
    }
}

fn direct_worker_group_exists(pid: u32) -> bool {
    let Some(group) = crate::plugin_process::owned_group_id(pid) else {
        return true; // Invalid ownership is not evidence of process absence.
    };
    observed_group_exists(rustix::process::test_kill_process_group(group), pid)
}

fn observed_group_exists(result: rustix::io::Result<()>, pid: u32) -> bool {
    match result {
        Ok(()) => true,
        Err(rustix::io::Errno::SRCH) => false,
        Err(error) => {
            // EPERM proves no absence. An external kill program's exit status
            // used to conflate it with ESRCH and could release this fence early.
            tracing::error!(pid, %error, "probing direct worker process group failed");
            true
        }
    }
}

async fn wait_for_direct_worker_group_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if !direct_worker_group_exists(pid) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(DIRECT_WORKER_EXIT_POLL_INTERVAL).await;
    }
}

async fn reap_direct_worker_group(
    workers: &Mutex<HashMap<String, u32>>,
    session_id: &str,
    pid: u32,
) -> bool {
    if !direct_worker_group_exists(pid) {
        return true;
    }
    tracing::warn!(session = %session_id, pid, "direct worker exited with live process-group descendants; sending TERM");
    if let Err(error) =
        signal_direct_worker(workers, session_id, pid, rustix::process::Signal::TERM)
    {
        tracing::warn!(session = %session_id, pid, %error, "direct worker descendant TERM failed");
    }
    if wait_for_direct_worker_group_exit(pid, DIRECT_WORKER_TERM_TIMEOUT).await {
        return true;
    }
    tracing::error!(session = %session_id, pid, "direct worker descendants ignored TERM; sending KILL");
    if let Err(error) =
        signal_direct_worker(workers, session_id, pid, rustix::process::Signal::KILL)
    {
        tracing::error!(session = %session_id, pid, %error, "direct worker descendant KILL failed");
    }
    let reaped = wait_for_direct_worker_group_exit(pid, DIRECT_WORKER_KILL_TIMEOUT).await;
    if !reaped {
        tracing::error!(session = %session_id, pid, "direct worker process group survived KILL timeout");
    }
    reaped
}

async fn enforce_direct_worker_exit(
    workers: Arc<Mutex<HashMap<String, u32>>>,
    session_id: String,
    pid: u32,
    graceful_timeout: Duration,
    term_timeout: Duration,
    kill_timeout: Duration,
) -> bool {
    if wait_for_direct_worker_exit(&workers, &session_id, pid, graceful_timeout).await {
        return true;
    }
    tracing::warn!(session = %session_id, pid, "direct worker ignored graceful Stop; sending TERM");
    if let Err(error) =
        signal_direct_worker(&workers, &session_id, pid, rustix::process::Signal::TERM)
    {
        tracing::warn!(session = %session_id, pid, %error, "direct worker TERM failed");
    }
    if wait_for_direct_worker_exit(&workers, &session_id, pid, term_timeout).await {
        return true;
    }
    tracing::error!(session = %session_id, pid, "direct worker ignored TERM; sending KILL");
    if let Err(error) =
        signal_direct_worker(&workers, &session_id, pid, rustix::process::Signal::KILL)
    {
        tracing::error!(session = %session_id, pid, %error, "direct worker KILL failed");
    }
    wait_for_direct_worker_exit(&workers, &session_id, pid, kill_timeout).await
}

#[derive(Clone)]
pub struct MachineBrokerArgs {
    /// Stable endpoint advertised to workers and the controller.
    pub socket: PathBuf,
    pub worker_command: PathBuf,
    pub desired_generation: String,
    pub spawn_mode: SpawnMode,
    /// Environment owned by the Machine component resolver and injected into
    /// every worker.
    pub worker_environment: BTreeMap<String, String>,
    /// Content-addressed Provider generations and Service-auth projections used
    /// to build each session's process environment.
    pub provider_store: Arc<crate::machine_plugins::MachinePluginStore>,
    /// Root containing Machine-owned session worktrees. Permanent session
    /// deletion may reclaim marked Cargo targets below this exact boundary.
    pub worktree_root: PathBuf,
    pub worker_ready_timeout: Duration,
}

#[derive(Clone)]
struct Controller {
    lease: u64,
    tx: mpsc::UnboundedSender<Frame>,
}

#[derive(Clone)]
struct WorkerPeer {
    connection_id: u64,
    epoch: String,
    tx: mpsc::UnboundedSender<Frame>,
    snapshot: WorkerSnapshot,
    last_seen: Instant,
    /// Duplicate of the broker end of this connection, used only to measure
    /// frames the worker already delivered but the broker has not read.
    receive_probe: Option<Arc<OwnedFd>>,
}

/// Outcome of one heartbeat sweep.
#[derive(Default)]
struct StaleWorkers {
    isolated: Vec<(String, WorkerPeer, String)>,
    lagging: Vec<(String, u64, Duration)>,
}

struct WorkerRegistration {
    session_id: String,
    epoch: String,
    generation: String,
    executable: Option<String>,
    fallback_for: Option<String>,
    connection_id: u64,
    tx: mpsc::UnboundedSender<Frame>,
}

#[derive(Clone)]
struct DeletedWorkspace {
    workspace: crate::session_workspace::CleanupWorkspace,
    command_id: String,
}

struct Broker {
    args: MachineBrokerArgs,
    controller: Mutex<Option<Controller>>,
    workers: Mutex<HashMap<String, WorkerPeer>>,
    pending_commands: Mutex<HashMap<String, VecDeque<WorkerCommand>>>,
    trace_spans: Mutex<crate::runtime_trace::MachineSpans>,
    sessions: Mutex<HashMap<String, StartSession>>,
    session_states: Mutex<HashMap<String, WorkerState>>,
    /// Last startup detail emitted by a worker before readiness. Generation
    /// fallback decisions use this to distinguish session-scoped failures from
    /// a bad Cowboy worker rollout.
    startup_failures: Mutex<HashMap<String, String>>,
    launching: Mutex<HashSet<String>>,
    awaiting_reconnect: Mutex<HashSet<String>>,
    cancelled_sessions: Mutex<HashSet<String>>,
    deletion_journal: Mutex<Option<deletions::Journal>>,
    /// Advisory nominations of deleted Sessions' original worktree roots, so a
    /// resident restart can finish artifact cleanup. Present only on a Machine
    /// admitted to write the deletion journal.
    cleanup_continuations: Mutex<Option<cleanups::Store>>,
    /// Test-only record of cache-protection revocations that were attempted.
    #[cfg(test)]
    revoked_cache_protection: Mutex<Vec<(String, &'static str)>>,
    /// In-process attempts before a Session with a durable nomination stops
    /// retrying, and the first backoff delay. Giving up releases its retained
    /// handles; the nomination retries after the next Machine restart.
    cleanup_retry: Mutex<(usize, Duration)>,
    /// Validated durable Session incarnation namespace. Writes are refused unless
    /// the dedicated writer build was admitted by the component owner.
    incarnations: Mutex<Option<incarnations::Store>>,
    /// Session workspaces awaiting generated-artifact cleanup after their
    /// process owner has been stopped and collected. Source worktrees and
    /// branches are retained.
    deleted_session_workspaces: Mutex<HashMap<String, DeletedWorkspace>>,
    /// Serializes launch declarations, deletion, reset and artifact cleanup per session.
    /// Cleanup may hold a gate across filesystem I/O so a replacement cannot
    /// start midway without blocking unrelated sessions.
    session_lifecycle_gates: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    /// In-flight explicit context resets, keyed by their controller command id.
    /// A permanent stop removes the token so a late reset task cannot revive a
    /// session that was genuinely deleted while the old worker was stopping.
    resetting_sessions: Mutex<HashMap<String, String>>,
    replacing: Mutex<HashMap<String, String>>,
    /// Session-local rollback pins. A healthy fallback must remain available
    /// even though the global desired generation is still marked unhealthy.
    fallback_pins: Mutex<HashMap<String, String>>,
    fallback_targets: Mutex<HashMap<String, String>>,
    /// Sessions whose candidate launch marked a provider generation unhealthy.
    /// Tracking the source lets deletion retract only its own failure.
    unhealthy_generations: Mutex<HashMap<(String, String), HashSet<String>>>,
    healthy_generations: Mutex<HashSet<(String, String)>>,
    /// Exact direct-mode worker owners. A bounded stop watchdog uses this map
    /// on macOS where per-session systemd units are unavailable. Entries are
    /// removed only by the task that reaps the matching child PID.
    direct_worker_pids: Arc<Mutex<HashMap<String, u32>>>,
    desired_generation: Mutex<String>,
    previous_generation: Mutex<Option<String>>,
    generation_commands: Mutex<HashMap<String, PathBuf>>,
    #[cfg(test)]
    deleted_session_owner_collected: AtomicBool,
    next_connection: AtomicU64,
    next_lease: AtomicU64,
}

/// Why a live worker cannot hibernate now, if anything would be lost.
fn hibernation_refusal(worker: &WorkerSnapshot) -> Option<&'static str> {
    if worker.state != WorkerState::Running || worker.current_turn_id.is_some() {
        Some("session is busy")
    } else if !worker.pending_permissions.is_empty() {
        Some("session is waiting for a permission decision")
    } else if worker.background_tasks.unwrap_or(0) > 0 {
        Some("session has background work in progress")
    } else if worker.pending_prompt_count > 0 {
        Some("session has a queued prompt")
    } else {
        None
    }
}

fn should_recycle_for_explicit_revive(worker: &WorkerSnapshot, desired_generation: &str) -> bool {
    matches!(worker.state, WorkerState::Exited | WorkerState::Crashed)
        || (!desired_generation.is_empty()
            && worker.generation != desired_generation
            && worker.drain_requested
            && worker.current_turn_id.is_none()
            && worker.pending_permissions.is_empty()
            && matches!(worker.state, WorkerState::Running | WorkerState::Draining))
}

impl Broker {
    fn revoke_cache_protection(
        &self,
        session: &StartSession,
        session_id: &str,
        reason: &'static str,
    ) {
        let configuration = session.provider_behavior.as_ref().map_or_else(
            || crate::provider_behavior::legacy_behavior(&session.provider).configuration,
            |behavior| behavior.configuration.clone(),
        );
        if !crate::deepseek_cache::supported_behavior(&configuration) {
            return;
        }
        #[cfg(test)]
        self.revoked_cache_protection
            .lock()
            .push((session_id.to_owned(), reason));
        let provider = session.provider.clone();
        let session_id = session_id.to_owned();
        tokio::spawn(async move {
            if let Err(error) =
                crate::deepseek_cache::revoke_local_snapshot(&configuration, &session_id).await
            {
                tracing::warn!(
                    provider,
                    session = %session_id,
                    reason,
                    %error,
                    "failed to revoke DeepSeek cache-protection snapshot"
                );
            } else {
                tracing::debug!(
                    provider,
                    session = %session_id,
                    reason,
                    "revoked DeepSeek cache-protection snapshot"
                );
            }
        });
    }

    fn new(args: MachineBrokerArgs) -> Self {
        let mut generation_commands = HashMap::new();
        if !args.desired_generation.is_empty() {
            generation_commands
                .insert(args.desired_generation.clone(), args.worker_command.clone());
        }
        Self {
            desired_generation: Mutex::new(args.desired_generation.clone()),
            args,
            controller: Mutex::new(None),
            workers: Mutex::new(HashMap::new()),
            pending_commands: Mutex::new(HashMap::new()),
            trace_spans: Mutex::default(),
            sessions: Mutex::new(HashMap::new()),
            session_states: Mutex::new(HashMap::new()),
            startup_failures: Mutex::new(HashMap::new()),
            launching: Mutex::new(HashSet::new()),
            awaiting_reconnect: Mutex::new(HashSet::new()),
            cancelled_sessions: Mutex::new(HashSet::new()),
            deletion_journal: Mutex::new(None),
            cleanup_continuations: Mutex::new(None),
            #[cfg(test)]
            revoked_cache_protection: Mutex::new(Vec::new()),
            cleanup_retry: Mutex::new((CLEANUP_ATTEMPT_LIMIT, Duration::from_secs(1))),
            incarnations: Mutex::new(None),
            deleted_session_workspaces: Mutex::new(HashMap::new()),
            session_lifecycle_gates: Mutex::new(HashMap::new()),
            resetting_sessions: Mutex::new(HashMap::new()),
            replacing: Mutex::new(HashMap::new()),
            fallback_pins: Mutex::new(HashMap::new()),
            fallback_targets: Mutex::new(HashMap::new()),
            unhealthy_generations: Mutex::new(HashMap::new()),
            healthy_generations: Mutex::new(HashSet::new()),
            direct_worker_pids: Arc::new(Mutex::new(HashMap::new())),
            previous_generation: Mutex::new(None),
            generation_commands: Mutex::new(generation_commands),
            #[cfg(test)]
            deleted_session_owner_collected: AtomicBool::new(false),
            next_connection: AtomicU64::new(1),
            next_lease: AtomicU64::new(1),
        }
    }

    fn snapshots(&self) -> Vec<WorkerSnapshot> {
        let pending_prompts: HashMap<String, u64> = self
            .pending_commands
            .lock()
            .iter()
            .map(|(session_id, commands)| {
                let count = commands
                    .iter()
                    .filter(|command| matches!(command, WorkerCommand::Prompt { .. }))
                    .count();
                (session_id.clone(), u64::try_from(count).unwrap_or(u64::MAX))
            })
            .collect();
        let sessions = self.sessions.lock().clone();
        let states = self.session_states.lock().clone();
        let commands = self.generation_commands.lock().clone();
        let mut snapshots: Vec<_> = self
            .workers
            .lock()
            .values()
            .map(|worker| {
                let mut snapshot = worker.snapshot.clone();
                snapshot.pending_prompt_count = pending_prompts
                    .get(&snapshot.session_id)
                    .copied()
                    .unwrap_or(0);
                if snapshot.launch.is_none() {
                    snapshot.launch = sessions.get(&snapshot.session_id).cloned();
                }
                snapshot
            })
            .collect();
        let live: HashSet<String> = snapshots
            .iter()
            .map(|snapshot| snapshot.session_id.clone())
            .collect();
        for (session_id, session) in sessions {
            if live.contains(&session_id) {
                continue;
            }
            snapshots.push(WorkerSnapshot {
                session_id: session_id.clone(),
                worker_epoch: format!("broker-{session_id}"),
                generation: session.generation.clone(),
                executable: commands
                    .get(&session.generation)
                    .map(|path| path.display().to_string()),
                launch: Some(session.clone()),
                state: states
                    .get(&session_id)
                    .copied()
                    .unwrap_or(WorkerState::Starting),
                agent_session_id: session.agent_session_id,
                native_thread_materialized: None,
                current_turn_id: None,
                last_runtime_seq: 0,
                pending_permissions: Vec::new(),
                config_options: None,
                context_used: None,
                context_size: None,
                pending_prompt_count: pending_prompts.get(&session_id).copied().unwrap_or(0),
                drain_requested: false,
                exit_detail: None,
                background_tasks: None,
                incarnation: None,
            });
        }
        for snapshot in &mut snapshots {
            snapshot.incarnation = self.reported_lineage(&snapshot.session_id);
        }
        snapshots.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        snapshots
    }

    fn install_controller(&self, tx: mpsc::UnboundedSender<Frame>) -> u64 {
        let lease = self.next_lease.fetch_add(1, Ordering::Relaxed);
        self.controller.lock().replace(Controller { lease, tx });
        lease
    }

    fn pin_fallback(&self, session_id: &str, worker_generation: &str, failed_generation: &str) {
        self.fallback_pins
            .lock()
            .insert(session_id.to_owned(), worker_generation.to_owned());
        self.fallback_targets
            .lock()
            .insert(session_id.to_owned(), failed_generation.to_owned());
    }

    fn unpin_fallback(&self, session_id: &str) {
        self.fallback_pins.lock().remove(session_id);
        self.fallback_targets.lock().remove(session_id);
    }

    fn clear_generation_failures_for_session(&self, session_id: &str) {
        self.unhealthy_generations.lock().retain(|_, sessions| {
            sessions.remove(session_id);
            !sessions.is_empty()
        });
    }

    fn quarantine_generation_if_unproven(
        &self,
        generation: &str,
        provider: &str,
        session_id: &str,
    ) {
        let generation_key = (generation.to_owned(), provider.to_owned());
        let healthy_generations = self.healthy_generations.lock();
        if healthy_generations.contains(&generation_key) {
            return;
        }
        self.unhealthy_generations
            .lock()
            .entry(generation_key)
            .or_default()
            .insert(session_id.to_owned());
        drop(healthy_generations);
    }

    fn rehabilitate_generation(&self, generation: &str, provider: &str) -> Vec<String> {
        let generation_key = (generation.to_owned(), provider.to_owned());
        let mut healthy_generations = self.healthy_generations.lock();
        let mut unhealthy_generations = self.unhealthy_generations.lock();
        healthy_generations.insert(generation_key.clone());
        unhealthy_generations.remove(&generation_key);
        drop(unhealthy_generations);
        drop(healthy_generations);
        let sessions = self.sessions.lock();
        let rehabilitated: Vec<String> = self
            .fallback_targets
            .lock()
            .iter()
            .filter(|(session_id, failed_generation)| {
                *failed_generation == generation
                    && sessions
                        .get(*session_id)
                        .is_some_and(|session| session.provider == provider)
            })
            .map(|(session_id, _)| session_id.clone())
            .collect();
        drop(sessions);
        for session_id in &rehabilitated {
            self.unpin_fallback(session_id);
        }
        let draining: Vec<String> = {
            let mut workers = self.workers.lock();
            rehabilitated
                .iter()
                .filter_map(|session_id| {
                    let worker = workers.get_mut(session_id)?;
                    if worker.snapshot.generation == generation {
                        return None;
                    }
                    worker.snapshot.drain_requested = true;
                    Some(session_id.clone())
                })
                .collect()
        };
        for session_id in draining {
            self.route_worker(&session_id, WorkerCommand::Drain);
        }
        rehabilitated
    }

    fn controller_for(&self, lease: u64) -> Option<mpsc::UnboundedSender<Frame>> {
        self.controller
            .lock()
            .as_ref()
            .filter(|controller| controller.lease == lease)
            .map(|controller| controller.tx.clone())
    }

    fn current_controller(&self) -> Option<mpsc::UnboundedSender<Frame>> {
        self.controller
            .lock()
            .as_ref()
            .map(|controller| controller.tx.clone())
    }

    fn remove_controller(&self, lease: u64) {
        let mut controller = self.controller.lock();
        if controller
            .as_ref()
            .is_some_and(|current| current.lease == lease)
        {
            controller.take();
        }
    }

    fn route_worker(&self, session_id: &str, command: WorkerCommand) {
        let terminal_stop = matches!(&command, WorkerCommand::Stop { .. });
        match self
            .workers
            .lock()
            .get(session_id)
            .map(|worker| worker.tx.clone())
        {
            Some(tx) => {
                let _ = tx.send(Frame::WorkerCommand {
                    session_id: session_id.to_owned(),
                    command: self.trace_worker_dispatch(session_id, command),
                });
            }
            _ => {
                self.queue_pending(session_id, command);
            }
        }
        if terminal_stop {
            self.arm_direct_worker_stop(session_id);
        }
    }

    fn arm_direct_worker_stop(&self, session_id: &str) {
        if self.args.spawn_mode != SpawnMode::Direct {
            return;
        }
        let workers = Arc::clone(&self.direct_worker_pids);
        let pid = workers.lock().get(session_id).copied();
        let Some(pid) = pid else {
            return;
        };
        let session_id = session_id.to_owned();
        tokio::spawn(async move {
            if !enforce_direct_worker_exit(
                workers,
                session_id.clone(),
                pid,
                DIRECT_WORKER_GRACEFUL_STOP_TIMEOUT,
                DIRECT_WORKER_TERM_TIMEOUT,
                DIRECT_WORKER_KILL_TIMEOUT,
            )
            .await
            {
                tracing::error!(session = %session_id, pid, "direct worker remained owned after KILL timeout");
            }
        });
    }

    /// New prompts wait behind a generation handoff. Cancellation and
    /// permission replies still route to the old worker because they are part
    /// of the in-flight turn that must reach its safe boundary.
    fn route_prompt(&self, session_id: &str, command: WorkerCommand) {
        if let WorkerCommand::Prompt {
            command_id,
            trace: Some(trace),
            ..
        } = &command
        {
            self.trace_spans.lock().start(session_id, command_id, trace);
        }
        let draining = self
            .workers
            .lock()
            .get(session_id)
            .is_some_and(|worker| worker.snapshot.drain_requested);
        if draining {
            self.queue_pending(session_id, command);
        } else {
            self.route_worker(session_id, command);
        }
    }

    fn queue_pending(&self, session_id: &str, command: WorkerCommand) {
        let command_id = worker_command_id(&command);
        let mut pending = self.pending_commands.lock();
        let queue = pending.entry(session_id.to_owned()).or_default();
        if command_id.is_some_and(|command_id| {
            queue
                .iter()
                .any(|queued| worker_command_id(queued) == Some(command_id))
        }) {
            return;
        }
        queue.push_back(command);
    }

    fn trace_worker_dispatch(&self, session_id: &str, mut command: WorkerCommand) -> WorkerCommand {
        if let WorkerCommand::Prompt {
            command_id,
            trace: Some(trace),
            ..
        } = &mut command
        {
            self.trace_spans
                .lock()
                .dispatch(session_id, command_id, trace);
        }
        command
    }

    fn worker_matches(&self, session_id: &str, connection_id: u64, epoch: &str) -> bool {
        self.workers
            .lock()
            .get(session_id)
            .is_some_and(|worker| worker.connection_id == connection_id && worker.epoch == epoch)
    }

    fn remove_worker(&self, session_id: &str, connection_id: u64) -> Option<WorkerPeer> {
        let mut workers = self.workers.lock();
        if workers
            .get(session_id)
            .is_some_and(|worker| worker.connection_id == connection_id)
        {
            return workers.remove(session_id);
        }
        None
    }

    fn attach_worker_receive_probe(
        &self,
        session_id: &str,
        connection_id: u64,
        socket: std::os::fd::BorrowedFd<'_>,
    ) {
        let probe = match socket.try_clone_to_owned() {
            Ok(probe) => Arc::new(probe),
            Err(error) => {
                tracing::warn!(session = session_id, %error, "worker receive-queue probe unavailable");
                return;
            }
        };
        if let Some(worker) = self.workers.lock().get_mut(session_id)
            && worker.connection_id == connection_id
        {
            worker.receive_probe = Some(probe);
        }
    }

    fn take_stale_workers(&self, timeout: Duration, backlog_limit: Duration) -> StaleWorkers {
        let now = Instant::now();
        let mut workers = self.workers.lock();
        let mut sweep = StaleWorkers::default();
        let mut stale = Vec::new();
        for (session_id, worker) in workers.iter() {
            let silent = now.duration_since(worker.last_seen);
            if silent < timeout {
                continue;
            }
            let unread = worker
                .receive_probe
                .as_deref()
                .and_then(|probe| rustix::io::ioctl_fionread(probe).ok())
                .unwrap_or(0);
            let detail = if unread == 0 {
                format!(
                    "worker heartbeat timed out after {}s; Machine broker stopped the worker",
                    silent.as_secs()
                )
            } else if silent < backlog_limit {
                sweep.lagging.push((session_id.clone(), unread, silent));
                continue;
            } else {
                format!(
                    "Machine broker read timed out after {}s with {unread} bytes of worker output queued; stopped the worker",
                    silent.as_secs()
                )
            };
            stale.push((session_id.clone(), detail));
        }
        for (session_id, detail) in stale {
            if let Some(worker) = workers.remove(&session_id) {
                sweep.isolated.push((session_id, worker, detail));
            }
        }
        sweep
    }

    fn update_snapshot(&self, mut snapshot: WorkerSnapshot, connection_id: u64) -> bool {
        let session_id = snapshot.session_id.clone();
        if self.check_deletion_journal().is_err() {
            return false;
        }
        // Authenticate the original peer before a rejected snapshot can send
        // anything to the current worker. A replaced peer owns no such effect.
        if !self.worker_matches(&session_id, connection_id, &snapshot.worker_epoch) {
            return false;
        }
        let state = snapshot.state;
        let launch = snapshot.launch.clone();
        let incoming = launch
            .as_ref()
            .and_then(|launch| launch.execution_binding.as_ref());
        // Keep declaration validation and replacement in one critical section.
        // In particular, an old worker cannot undo a staged workspace reset.
        // Deletion takes the same cancelled -> sessions lock order.
        let cancelled = self.cancelled_sessions.lock();
        let mut sessions = self.sessions.lock();
        if incoming.is_some_and(|binding| binding.decode().is_err())
            || launch
                .as_ref()
                .is_some_and(|launch| launch.session_id != session_id)
            || sessions.get(&session_id).is_some_and(|declared| {
                declared.execution_binding.as_ref() != incoming
                    || launch.as_ref().is_some_and(|launch| {
                        launch.provider != declared.provider
                            || launch.cwd != declared.cwd
                            || launch.system != declared.system
                    })
            })
        {
            if let Some(worker) = self.workers.lock().get(&session_id)
                && worker.connection_id == connection_id
                && worker.epoch == snapshot.worker_epoch
            {
                let _ = worker.tx.send(Frame::Reject {
                    reason: "worker session placement differs from the session declaration".into(),
                });
            }
            return false;
        }
        let mut accepted = false;
        if let Some(worker) = self.workers.lock().get_mut(&session_id)
            && worker.connection_id == connection_id
            && worker.epoch == snapshot.worker_epoch
        {
            snapshot.drain_requested |= worker.snapshot.drain_requested;
            worker.snapshot = snapshot;
            worker.last_seen = Instant::now();
            accepted = true;
        }
        if !accepted {
            return false;
        }
        if cancelled.contains(&session_id) {
            return false;
        }
        if let Some(launch) = launch.as_ref()
            && let Some(failed_generation) = launch.fallback_for.as_ref()
        {
            let desired = self.desired_generation.lock().clone();
            let generation_is_healthy = self
                .healthy_generations
                .lock()
                .contains(&(failed_generation.clone(), launch.provider.clone()));
            if (desired.is_empty() || desired == *failed_generation) && !generation_is_healthy {
                self.pin_fallback(&session_id, &launch.generation, failed_generation);
                *self.previous_generation.lock() = Some(launch.generation.clone());
                self.unhealthy_generations
                    .lock()
                    .entry((failed_generation.clone(), launch.provider.clone()))
                    .or_default()
                    .insert(session_id.clone());
            }
        }
        if let Some(launch) = launch {
            sessions.insert(session_id.clone(), launch);
        }
        drop(sessions);
        self.session_states.lock().insert(session_id, state);
        drop(cancelled);
        true
    }

    fn touch_worker(&self, session_id: &str, connection_id: u64) {
        if let Some(worker) = self.workers.lock().get_mut(session_id)
            && worker.connection_id == connection_id
        {
            worker.last_seen = Instant::now();
        }
    }

    fn acknowledge_worker_event(
        &self,
        session_id: &str,
        connection_id: u64,
        worker_epoch: String,
        runtime_seq: u64,
    ) {
        let tx = self
            .workers
            .lock()
            .get(session_id)
            .filter(|worker| worker.connection_id == connection_id && worker.epoch == worker_epoch)
            .map(|worker| worker.tx.clone());
        if let Some(tx) = tx {
            let _ = tx.send(Frame::Ack {
                session_id: session_id.to_owned(),
                worker_epoch,
                runtime_seq,
            });
        }
    }

    fn register_worker(&self, registration: WorkerRegistration) -> Result<()> {
        self.check_deletion_journal()?;
        let WorkerRegistration {
            session_id,
            epoch,
            generation,
            executable,
            fallback_for,
            connection_id,
            tx,
        } = registration;
        if self.cancelled_sessions.lock().contains(&session_id) {
            // `handle_peer` turns this error into the only legal pre-Welcome
            // response: Reject. Sending Stop first makes the worker treat the
            // handshake as malformed and retry a permanently deleted session.
            anyhow::bail!("session {session_id} was deleted");
        }
        self.awaiting_reconnect.lock().remove(&session_id);
        let desired = self.desired_generation.lock().clone();
        let fallback_provider = self
            .sessions
            .lock()
            .get(&session_id)
            .map(|session| session.provider.clone());
        let fallback_is_healthy = fallback_for.as_ref().is_some_and(|failed| {
            fallback_provider.as_ref().is_some_and(|provider| {
                self.healthy_generations
                    .lock()
                    .contains(&(failed.clone(), provider.clone()))
            })
        });
        if fallback_for
            .as_ref()
            .is_some_and(|failed| desired.is_empty() || failed == &desired)
            && !fallback_is_healthy
        {
            self.pin_fallback(
                &session_id,
                &generation,
                fallback_for.as_deref().expect("checked above"),
            );
            *self.previous_generation.lock() = Some(generation.clone());
        }
        if let Some(executable) = executable.as_ref() {
            let mut commands = self.generation_commands.lock();
            if generation != desired || !commands.contains_key(&generation) {
                commands.insert(generation.clone(), PathBuf::from(executable));
            }
        }
        let pinned = self
            .fallback_pins
            .lock()
            .get(&session_id)
            .is_some_and(|pinned| pinned == &generation);
        if !desired.is_empty() && generation != desired && !pinned {
            self.previous_generation
                .lock()
                .get_or_insert_with(|| generation.clone());
        }
        let mut workers = self.workers.lock();
        if let Some(existing) = workers.get(&session_id)
            && existing.epoch != epoch
        {
            anyhow::bail!(
                "session {session_id} already has worker epoch {}",
                existing.epoch
            );
        }
        workers.insert(
            session_id.clone(),
            WorkerPeer {
                connection_id,
                epoch: epoch.clone(),
                tx,
                snapshot: WorkerSnapshot {
                    session_id: session_id.clone(),
                    worker_epoch: epoch,
                    generation: generation.clone(),
                    executable,
                    launch: None,
                    state: crate::runtime_wire::WorkerState::Starting,
                    agent_session_id: None,
                    native_thread_materialized: None,
                    current_turn_id: None,
                    last_runtime_seq: 0,
                    pending_permissions: Vec::new(),
                    config_options: None,
                    context_used: None,
                    context_size: None,
                    pending_prompt_count: 0,
                    drain_requested: !desired.is_empty() && generation != desired && !pinned,
                    exit_detail: None,
                    background_tasks: None,
                    incarnation: None,
                },
                last_seen: Instant::now(),
                receive_probe: None,
            },
        );
        drop(workers);
        if !self.cancelled_sessions.lock().contains(&session_id) {
            self.session_states
                .lock()
                .insert(session_id, WorkerState::Starting);
        }
        Ok(())
    }

    fn flush_pending(&self, session_id: &str) {
        let worker = self
            .workers
            .lock()
            .get(session_id)
            .map(|worker| (worker.tx.clone(), worker.snapshot.drain_requested));
        let Some((tx, draining)) = worker else { return };
        let commands = self.pending_commands.lock().remove(session_id);
        if let Some(mut commands) = commands {
            let mut held_prompts = VecDeque::new();
            while let Some(command) = commands.pop_front() {
                if draining && matches!(command, WorkerCommand::Prompt { .. }) {
                    held_prompts.push_back(command);
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

    /// The Session's durable lineage, reported only while this build's writer
    /// is admitted. A reader-only build could miss a rotation made by a writer
    /// elsewhere, so it reports none rather than a value nobody maintains.
    fn reported_lineage(&self, session_id: &str) -> Option<String> {
        let store = self.incarnations.lock();
        let store = store.as_ref().filter(|store| store.writer_enabled())?;
        store.get(session_id).map(|entry| entry.incarnation.clone())
    }

    fn send_controller(&self, mut frame: Frame) {
        if let Frame::Snapshot { worker } = &mut frame {
            worker.incarnation = self.reported_lineage(&worker.session_id);
        }
        if let Some(tx) = self.current_controller() {
            let _ = tx.send(frame);
        }
    }

    fn publish_session_state(&self, session_id: &str, state: WorkerState) {
        self.session_states
            .lock()
            .insert(session_id.to_owned(), state);
        if let Some(worker) = self
            .snapshots()
            .into_iter()
            .find(|worker| worker.session_id == session_id)
        {
            self.send_controller(Frame::Snapshot {
                worker: Box::new(worker),
            });
        }
    }

    fn command_rejected(&self, session_id: &str, command_id: String, reason: String) {
        self.send_controller(Frame::CommandAck {
            session_id: session_id.to_owned(),
            command_id,
            accepted: false,
            reason: Some(reason),
        });
    }

    fn session_lifecycle_gate(&self, session_id: &str) -> Arc<AsyncMutex<()>> {
        self.session_lifecycle_gates
            .lock()
            .entry(session_id.to_owned())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    fn attach_deletion_journal(&self, journal: deletions::Journal) {
        self.cancelled_sessions
            .lock()
            .extend(journal.deleted().iter().cloned());
        *self.deletion_journal.lock() = Some(journal);
    }

    fn check_deletion_journal(&self) -> Result<()> {
        if let Some(journal) = self.deletion_journal.lock().as_ref() {
            journal.check()?;
        }
        Ok(())
    }

    async fn record_deletion(self: &Arc<Self>, session_id: &str) -> Result<()> {
        let broker = Arc::clone(self);
        let session_id = session_id.to_owned();
        tokio::task::spawn_blocking(move || {
            if let Some(journal) = broker.deletion_journal.lock().as_mut() {
                journal.check()?;
                if journal.writer_enabled() {
                    journal.mark_deleted(&session_id)?;
                }
            }
            Ok(())
        })
        .await
        .context("joining Session deletion journal write")?
    }

    fn attach_cleanup_continuations(&self, store: cleanups::Store) {
        *self.cleanup_continuations.lock() = Some(store);
    }

    /// Best effort: the committed deletion journal, not this advisory record,
    /// decides the deletion. A failed write only means cleanup cannot resume
    /// after a restart, which is the behaviour before the record existed.
    async fn record_cleanup_continuation(
        self: &Arc<Self>,
        session_id: &str,
        root: crate::session_workspace::RootIdentity,
    ) {
        let broker = Arc::clone(self);
        let owned = session_id.to_owned();
        let outcome = tokio::task::spawn_blocking(move || {
            match broker.cleanup_continuations.lock().as_mut() {
                Some(store) => store.record(&owned, root),
                None => Ok(()),
            }
        })
        .await;
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(session = %session_id, %error,
                "deleted session cleanup will not resume after a Machine restart"),
            Err(error) => tracing::warn!(session = %session_id, %error,
                "joining cleanup continuation write failed"),
        }
    }

    async fn retire_cleanup_continuation(self: &Arc<Self>, session_id: &str) {
        let broker = Arc::clone(self);
        let owned = session_id.to_owned();
        let outcome = tokio::task::spawn_blocking(move || {
            match broker.cleanup_continuations.lock().as_mut() {
                Some(store) => store.retire(&owned),
                None => Ok(()),
            }
        })
        .await;
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(session = %session_id, %error,
                "retaining cleanup continuation; a later restart will observe it again"),
            Err(error) => tracing::warn!(session = %session_id, %error,
                "joining cleanup continuation retirement failed"),
        }
    }

    /// Finish cleanup that a previous resident process accepted but did not
    /// complete. Each nomination needs the committed terminal deletion, then
    /// the exact original root object; Cargo targets are always rescanned.
    fn resume_cleanup_continuations(self: &Arc<Self>) {
        let broker = Arc::clone(self);
        tokio::spawn(async move {
            let pending = broker
                .cleanup_continuations
                .lock()
                .as_ref()
                .map(cleanups::Store::pending)
                .unwrap_or_default();
            if pending.is_empty() {
                return;
            }
            tracing::info!(sessions = pending.len(), "resuming deleted session cleanup");
            for (session_id, root) in pending {
                if !broker.cancelled_sessions.lock().contains(&session_id) {
                    tracing::warn!(session = %session_id,
                        "cleanup continuation has no committed terminal deletion; leaving it untouched");
                    continue;
                }
                let worktree_root = broker.args.worktree_root.clone();
                let observed = {
                    let session_id = session_id.clone();
                    tokio::task::spawn_blocking(move || {
                        crate::session_workspace::resume_cleanup_workspace(
                            &worktree_root,
                            &session_id,
                            &root,
                        )
                    })
                    .await
                };
                match observed {
                    Ok(Ok(workspace)) => {
                        let command_id = format!("resume-{session_id}");
                        broker.deleted_session_workspaces.lock().insert(
                            session_id.clone(),
                            DeletedWorkspace {
                                workspace,
                                command_id: command_id.clone(),
                            },
                        );
                        let attempt = broker.cleanup_deleted_session(&session_id, &command_id);
                        // Start sessions one at a time without letting a
                        // persistently failing one starve the rest.
                        let _ = tokio::time::timeout(Duration::from_secs(120), attempt).await;
                    }
                    Ok(Err(error))
                        if error
                            .downcast_ref::<crate::session_workspace::CleanupRootChanged>()
                            .is_some() =>
                    {
                        tracing::warn!(session = %session_id,
                            "preserving artifacts: the original worktree root is gone or was replaced; retiring cleanup continuation");
                        broker.retire_cleanup_continuation(&session_id).await;
                    }
                    Ok(Err(error)) => tracing::warn!(session = %session_id, %error,
                        "deleted session cleanup could not be resumed; keeping its continuation"),
                    Err(error) => tracing::warn!(session = %session_id, %error,
                        "joining cleanup resume observation failed"),
                }
            }
        });
    }

    /// Make sure the Session has a durable incarnation before any worker is
    /// declared or adopted. An existing lineage is kept (replayed declarations,
    /// reconnects and wake never rotate it). A build whose writer was not
    /// admitted neither writes nor refuses anything.
    async fn ensure_incarnation(
        self: &Arc<Self>,
        session_id: &str,
        origin: incarnations::Origin,
    ) -> Result<()> {
        match self.incarnations.lock().as_ref() {
            None => return Ok(()),
            Some(store) if !store.writer_enabled() || store.get(session_id).is_some() => {
                return Ok(());
            }
            Some(_) => {}
        }
        let broker = Arc::clone(self);
        let session_id = session_id.to_owned();
        tokio::task::spawn_blocking(move || {
            if let Some(store) = broker.incarnations.lock().as_mut() {
                store.mint(&session_id, origin)?;
            }
            Ok(())
        })
        .await
        .context("joining Session incarnation write")?
    }

    /// Start a new lineage for a reset, committed before its first effect.
    async fn rotate_incarnation(self: &Arc<Self>, session_id: &str) -> Result<()> {
        match self.incarnations.lock().as_ref() {
            Some(store) if store.writer_enabled() => {}
            _ => return Ok(()),
        }
        let broker = Arc::clone(self);
        let session_id = session_id.to_owned();
        tokio::task::spawn_blocking(move || {
            if let Some(store) = broker.incarnations.lock().as_mut() {
                store.rotate(&session_id)?;
            }
            Ok(())
        })
        .await
        .context("joining Session incarnation rotation")?
    }

    /// Drop the record of a Session whose terminal deletion is already committed.
    async fn end_incarnation(self: &Arc<Self>, session_id: &str) -> Result<()> {
        match self.incarnations.lock().as_ref() {
            Some(store) if store.writer_enabled() && store.get(session_id).is_some() => {}
            _ => return Ok(()),
        }
        let broker = Arc::clone(self);
        let session_id = session_id.to_owned();
        tokio::task::spawn_blocking(move || {
            if let Some(store) = broker.incarnations.lock().as_mut() {
                store.end(&session_id)?;
            }
            Ok(())
        })
        .await
        .context("joining Session incarnation retirement")?
    }

    fn has_deleted_session_owner_exit_proof(&self) -> bool {
        #[cfg(test)]
        if self.deleted_session_owner_collected.load(Ordering::Acquire) {
            return true;
        }
        false
    }

    async fn confirm_deleted_session_owner_exit(&self, session_id: &str) -> Result<()> {
        if self.has_deleted_session_owner_exit_proof() {
            return Ok(());
        }
        if self.args.spawn_mode != SpawnMode::SystemdUser {
            anyhow::bail!("direct mode has no process-exit proof");
        }
        prepare_transient_unit(&worker_unit_name(&self.args.socket, session_id)).await
    }

    fn cleanup_deleted_session(
        self: &Arc<Self>,
        session_id: &str,
        command_id: &str,
    ) -> tokio::task::JoinHandle<()> {
        let broker = Arc::clone(self);
        let session_id = session_id.to_owned();
        let command_id = command_id.to_owned();
        tokio::spawn(async move {
            let (attempt_limit, first_delay) = *broker.cleanup_retry.lock();
            let mut retry_delay = first_delay;
            let mut failures = 0_usize;
            loop {
                let gate = broker.session_lifecycle_gate(&session_id);
                let _guard = gate.lock().await;
                let outcome: Result<Option<Vec<PathBuf>>> = async {
                    let Some(workspace) = broker
                        .deleted_session_workspaces
                        .lock()
                        .get(&session_id)
                        .cloned()
                    else {
                        return Ok(None);
                    };
                    if workspace.command_id != command_id {
                        return Ok(None);
                    }
                    if broker.args.spawn_mode != SpawnMode::SystemdUser
                        && !broker.has_deleted_session_owner_exit_proof()
                    {
                        broker.deleted_session_workspaces.lock().remove(&session_id);
                        tracing::warn!(
                            session = %session_id,
                            "preserving deleted session build artifacts because direct mode has no process-exit proof"
                        );
                        return Ok(None);
                    }
                    if !broker.cancelled_sessions.lock().contains(&session_id)
                        || broker.sessions.lock().contains_key(&session_id)
                    {
                        broker.deleted_session_workspaces.lock().remove(&session_id);
                        tracing::debug!(
                            session = %session_id,
                            "cancelled session became live; preserving build artifacts"
                        );
                        return Ok(None);
                    }

                    let deadline =
                        tokio::time::Instant::now() + broker.args.worker_ready_timeout;
                    while broker.launching.lock().contains(&session_id) {
                        if tokio::time::Instant::now() >= deadline {
                            anyhow::bail!("worker launch did not settle before cleanup");
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }

                    // Production workers are transient user-systemd units. The
                    // unit disappearing is the owner-exit proof; a broker socket
                    // disconnect alone is deliberately insufficient.
                    broker
                        .confirm_deleted_session_owner_exit(&session_id)
                        .await?;
                    broker.workers.lock().remove(&session_id);
                    let removed = crate::session_workspace::cleanup_build_artifacts(&workspace.workspace)
                    .await?;
                    if broker
                        .deleted_session_workspaces
                        .lock()
                        .get(&session_id)
                        .is_some_and(|current| current.command_id == command_id)
                    {
                        broker.deleted_session_workspaces.lock().remove(&session_id);
                    }
                    Ok(Some(removed))
                }
                .await;

                match outcome {
                    Ok(None) => return,
                    Ok(Some(removed)) if removed.is_empty() => {
                        tracing::debug!(session = %session_id, "deleted session had no marked Cargo targets");
                        broker.retire_cleanup_continuation(&session_id).await;
                        return;
                    }
                    Ok(Some(removed)) => {
                        tracing::info!(
                            session = %session_id,
                            targets = removed.len(),
                            "reclaimed deleted session Cargo targets"
                        );
                        broker.retire_cleanup_continuation(&session_id).await;
                        return;
                    }
                    Err(error)
                        if error
                            .downcast_ref::<crate::session_workspace::CleanupRootChanged>()
                            .is_some()
                            || error
                                .downcast_ref::<crate::session_workspace::CleanupTargetChanged>()
                                .is_some() =>
                    {
                        broker.deleted_session_workspaces.lock().remove(&session_id);
                        tracing::warn!(session = %session_id, %error,
                            "preserving artifacts after observed cleanup directory or marker change; retiring cleanup");
                        broker.retire_cleanup_continuation(&session_id).await;
                        return;
                    }
                    Err(error) => {
                        failures += 1;
                        let nominated = broker
                            .cleanup_continuations
                            .lock()
                            .as_ref()
                            .is_some_and(|store| store.contains(&session_id));
                        if failures >= attempt_limit && nominated {
                            // Release the root, target and marker handles. The
                            // durable nomination observes the root again and
                            // rescans after the next Machine restart.
                            let mut workspaces = broker.deleted_session_workspaces.lock();
                            if workspaces
                                .get(&session_id)
                                .is_some_and(|current| current.command_id == command_id)
                            {
                                workspaces.remove(&session_id);
                            }
                            drop(workspaces);
                            tracing::warn!(
                                session = %session_id,
                                %error,
                                attempts = failures,
                                "deleted session cleanup keeps failing; releasing its handles and leaving the durable continuation for the next Machine restart"
                            );
                            return;
                        }
                        tracing::warn!(
                            session = %session_id,
                            %error,
                            delay_seconds = retry_delay.as_secs(),
                            "deleted session build-artifact cleanup failed closed; retrying"
                        );
                    }
                }
                drop(_guard);
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(Duration::from_secs(60));
            }
        })
    }

    async fn ensure_session(self: &Arc<Self>, session: StartSession) {
        let gate = self.session_lifecycle_gate(&session.session_id);
        let guard = gate.lock().await;
        self.ensure_session_in_lifecycle(session, &guard).await;
    }

    async fn ensure_session_in_lifecycle(
        self: &Arc<Self>,
        session: StartSession,
        _guard: &tokio::sync::MutexGuard<'_, ()>,
    ) {
        if let Err(error) = self.check_deletion_journal() {
            self.command_rejected(
                &session.session_id,
                format!("ensure:{}", session.session_id),
                format!("Session deletion reader unavailable: {error}"),
            );
            return;
        }
        // A delayed controller declaration is not permission to undo deletion.
        // Reset owns its separate fence and clears the tombstone deliberately.
        if self.cancelled_sessions.lock().contains(&session.session_id) {
            self.command_rejected(
                &session.session_id,
                format!("ensure:{}", session.session_id),
                "session was deleted; launch declaration was not adopted".into(),
            );
            return;
        }
        if session
            .execution_binding
            .as_ref()
            .is_some_and(|binding| binding.decode().is_err())
            || self
                .sessions
                .lock()
                .get(&session.session_id)
                .is_some_and(|previous| previous.execution_binding != session.execution_binding)
            || self
                .workers
                .lock()
                .get(&session.session_id)
                .and_then(|worker| worker.snapshot.launch.as_ref())
                .is_some_and(|launch| launch.execution_binding != session.execution_binding)
        {
            self.command_rejected(
                &session.session_id,
                format!("ensure:{}", session.session_id),
                "execution placement cannot change through worker adoption".into(),
            );
            return;
        }
        // Durable lineage first: no declaration, worker launch or adoption proceeds
        // without one when the writer is admitted.
        let origin = if self.workers.lock().contains_key(&session.session_id) {
            incarnations::Origin::Adopted
        } else {
            incarnations::Origin::Minted
        };
        if let Err(error) = self.ensure_incarnation(&session.session_id, origin).await {
            self.command_rejected(
                &session.session_id,
                format!("ensure:{}", session.session_id),
                format!("durable Session incarnation was not confirmed: {error}"),
            );
            return;
        }
        let adopt_only = session.adopt_only;
        let mut session = session;
        // `adopt_only` describes this controller message, not how a future
        // replacement worker should be launched or report its own snapshot.
        session.adopt_only = false;
        self.sessions
            .lock()
            .insert(session.session_id.clone(), session.clone());
        let desired_generation = self.desired_generation.lock().clone();
        let existing = {
            let mut workers = self.workers.lock();
            match workers.get(&session.session_id) {
                Some(worker)
                    if !adopt_only
                        && should_recycle_for_explicit_revive(
                            &worker.snapshot,
                            &desired_generation,
                        ) =>
                {
                    workers.remove(&session.session_id).map(Ok)
                }
                Some(worker) => Some(Err(worker.snapshot.clone())),
                None => None,
            }
        };
        match existing {
            Some(Err(worker)) => {
                self.send_controller(Frame::Snapshot {
                    worker: Box::new(worker),
                });
                return;
            }
            Some(Ok(worker)) => {
                // A legacy worker can publish its terminal event and then
                // remain connected forever. Take ownership from a rollout
                // that was waiting for that impossible disconnect.
                self.replacing.lock().remove(&session.session_id);
                self.startup_failures.lock().remove(&session.session_id);
                tracing::warn!(
                    session = %session.session_id,
                    state = ?worker.snapshot.state,
                    generation = %worker.snapshot.generation,
                    drain_requested = worker.snapshot.drain_requested,
                    desired_generation = %desired_generation,
                    "recycling stale worker before session revive"
                );
                let _ = worker.tx.send(Frame::WorkerCommand {
                    session_id: session.session_id.clone(),
                    command: WorkerCommand::Stop {
                        command_id: format!("revive-stop-{}", session.session_id),
                    },
                });
                if self.args.spawn_mode == SpawnMode::Direct {
                    let workers = Arc::clone(&self.direct_worker_pids);
                    let pid = workers.lock().get(&session.session_id).copied();
                    if let Some(pid) = pid
                        && !enforce_direct_worker_exit(
                            workers,
                            session.session_id.clone(),
                            pid,
                            DIRECT_WORKER_GRACEFUL_STOP_TIMEOUT,
                            DIRECT_WORKER_TERM_TIMEOUT,
                            DIRECT_WORKER_KILL_TIMEOUT,
                        )
                        .await
                    {
                        tracing::error!(
                            session = %session.session_id,
                            pid,
                            "stale direct worker remained owned after KILL timeout"
                        );
                    }
                }
            }
            None => {}
        }
        if adopt_only {
            tracing::debug!(session = %session.session_id, "adopted launch registry; waiting for worker reconnect");
            self.arm_reconnect_timeout(session.session_id.clone());
            return;
        }
        self.awaiting_reconnect.lock().remove(&session.session_id);
        self.session_states
            .lock()
            .insert(session.session_id.clone(), WorkerState::Starting);
        if !self.launching.lock().insert(session.session_id.clone()) {
            return;
        }
        let broker = Arc::clone(self);
        tokio::spawn(async move {
            let result = broker.spawn_with_fallback(session.clone(), None).await;
            broker.launching.lock().remove(&session.session_id);
            if let Err(error) = result {
                let cancelled = broker.cancelled_sessions.lock();
                if cancelled.contains(&session.session_id) {
                    tracing::debug!(
                        session = %session.session_id,
                        %error,
                        "deleted session launch stopped"
                    );
                    return;
                }
                if broker
                    .resetting_sessions
                    .lock()
                    .contains_key(&session.session_id)
                {
                    tracing::debug!(
                        session = %session.session_id,
                        %error,
                        "superseded worker launch stopped for context reset"
                    );
                    return;
                }
                tracing::error!(session = %session.session_id, error = %error, "worker launch failed");
                broker.publish_session_state(&session.session_id, WorkerState::Crashed);
                broker.command_rejected(
                    &session.session_id,
                    format!("ensure:{}", session.session_id),
                    error.to_string(),
                );
                drop(cancelled);
            }
        });
    }

    async fn reset_session(self: &Arc<Self>, mut session: StartSession, command_id: String) {
        let session_id = session.session_id.clone();
        session.adopt_only = false;
        let cleanup_gate = self.session_lifecycle_gate(&session_id);
        let _cleanup_guard = cleanup_gate.lock().await;
        let journal_admission = (|| -> Result<()> {
            if let Some(journal) = self.deletion_journal.lock().as_ref() {
                journal.check()?;
                ensure!(
                    !journal.deleted().contains(&session_id),
                    "Session ID is durably deleted"
                );
            }
            Ok(())
        })();
        if let Err(error) = journal_admission {
            self.command_rejected(&session_id, command_id, error.to_string());
            return;
        }
        // The new lineage is durable before the reset's first effect; if it
        // cannot be confirmed the old lineage stays current and nothing changes.
        if let Err(error) = self.rotate_incarnation(&session_id).await {
            self.command_rejected(
                &session_id,
                command_id,
                format!("durable Session incarnation was not confirmed: {error}"),
            );
            return;
        }
        self.revoke_cache_protection(&session, &session_id, "session_reset");
        self.deleted_session_workspaces.lock().remove(&session_id);
        self.resetting_sessions
            .lock()
            .insert(session_id.clone(), command_id.clone());

        // Fence both an attached worker and a launch that has not connected yet.
        // The tombstone remains set until every old launch task has observed it.
        self.cancelled_sessions.lock().insert(session_id.clone());
        self.sessions.lock().remove(&session_id);
        self.awaiting_reconnect.lock().remove(&session_id);
        self.session_states.lock().remove(&session_id);
        self.pending_commands.lock().remove(&session_id);
        self.startup_failures.lock().remove(&session_id);
        self.unpin_fallback(&session_id);
        self.force_recycle_failed_start(&session_id).await;

        let deadline = tokio::time::Instant::now() + self.args.worker_ready_timeout;
        while self.launching.lock().contains(&session_id) && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        let still_current = self
            .resetting_sessions
            .lock()
            .get(&session_id)
            .is_some_and(|current| current == &command_id);
        if !still_current {
            return;
        }
        if self.launching.lock().contains(&session_id) {
            self.resetting_sessions.lock().remove(&session_id);
            self.command_rejected(
                &session_id,
                command_id,
                "previous worker launch did not stop before reset".to_owned(),
            );
            return;
        }

        // `force_recycle_failed_start` queues Stop when no worker was attached;
        // do not let that stale command reach the fresh replacement.
        self.pending_commands.lock().remove(&session_id);
        self.cancelled_sessions.lock().remove(&session_id);
        self.resetting_sessions.lock().remove(&session_id);
        self.ensure_session_in_lifecycle(session, &_cleanup_guard)
            .await;
        self.send_controller(Frame::CommandAck {
            session_id,
            command_id,
            accepted: true,
            reason: None,
        });
    }

    /// Release an idle session's worker without deleting the session. Unlike
    /// [`Self::reset_session`] nothing relaunches: the declaration is dropped
    /// and the Controller's next `EnsureSession` resumes the retained native
    /// thread, exactly as after a Machine restart. A turn, a permission prompt
    /// or native background work refuses, because stopping would lose it.
    async fn hibernate_session(self: &Arc<Self>, session_id: String, command_id: String) {
        let cleanup_gate = self.session_lifecycle_gate(&session_id);
        let _cleanup_guard = cleanup_gate.lock().await;
        let snapshot = self
            .workers
            .lock()
            .get(&session_id)
            .map(|worker| worker.snapshot.clone());
        let Some(mut snapshot) = snapshot else {
            self.command_rejected(
                &session_id,
                command_id,
                "session has no live worker".to_owned(),
            );
            return;
        };
        if let Some(reason) = hibernation_refusal(&snapshot).or_else(|| {
            self.launching
                .lock()
                .contains(&session_id)
                .then_some("session is starting")
        }) {
            self.command_rejected(&session_id, command_id, reason.to_owned());
            return;
        }
        // Fence any launch while the worker stops, as a reset does, then
        // forget the declaration so nothing replaces it.
        self.cancelled_sessions.lock().insert(session_id.clone());
        // A sleeping session must not keep paying for its prompt cache: the
        // provider gateway replays an unrevoked snapshot with real model
        // requests (`cache_keepalive`). Delete, reset and provider roll already
        // revoke it; hibernation releases the same resource.
        let released = self.sessions.lock().remove(&session_id);
        if let Some(session) = released {
            self.revoke_cache_protection(&session, &session_id, "session_hibernated");
        }
        self.awaiting_reconnect.lock().remove(&session_id);
        self.startup_failures.lock().remove(&session_id);
        self.unpin_fallback(&session_id);
        self.force_recycle_failed_start(&session_id).await;
        self.pending_commands.lock().remove(&session_id);
        self.cancelled_sessions.lock().remove(&session_id);
        self.session_states.lock().remove(&session_id);
        snapshot.state = WorkerState::Exited;
        snapshot.exit_detail = Some("hibernated".to_owned());
        snapshot.background_tasks = Some(0);
        tracing::info!(session = %session_id, "session hibernated");
        self.send_controller(Frame::Snapshot {
            worker: Box::new(snapshot),
        });
        self.send_controller(Frame::CommandAck {
            session_id,
            command_id,
            accepted: true,
            reason: None,
        });
    }

    fn arm_reconnect_timeout(self: &Arc<Self>, session_id: String) {
        if self.args.spawn_mode != SpawnMode::SystemdUser
            || !self.awaiting_reconnect.lock().insert(session_id.clone())
        {
            return;
        }
        let broker = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(WORKER_HEARTBEAT_TIMEOUT).await;
            if !broker.awaiting_reconnect.lock().remove(&session_id)
                || broker.workers.lock().contains_key(&session_id)
                || !broker.sessions.lock().contains_key(&session_id)
            {
                return;
            }
            tracing::error!(
                session = %session_id,
                "declared worker never reconnected; applying session-level extreme recovery"
            );
            let stop = Command::new("systemctl")
                .args([
                    "--user",
                    "stop",
                    &worker_unit_name(&broker.args.socket, &session_id),
                ])
                .status();
            match tokio::time::timeout(Duration::from_secs(10), stop).await {
                Ok(Ok(status)) if status.success() => {}
                Ok(Ok(status)) => {
                    tracing::warn!(session = %session_id, %status, "stopping missing worker unit failed")
                }
                Ok(Err(error)) => {
                    tracing::warn!(session = %session_id, %error, "stopping missing worker unit failed")
                }
                Err(_) => {
                    tracing::warn!(session = %session_id, "stopping missing worker unit timed out")
                }
            }
            broker.publish_session_state(&session_id, WorkerState::Crashed);
        });
    }

    async fn spawn_with_fallback(
        &self,
        session: StartSession,
        session_fallback: Option<String>,
    ) -> Result<()> {
        // Previous generations may predate this mandatory launch field. They
        // must never be tried as an implicit recovery path for a bound worker.
        if session.execution_binding.is_some() {
            return self.spawn_and_wait_ready(&session).await;
        }
        let generation_key = (session.generation.clone(), session.provider.clone());
        let desired_is_unhealthy = self
            .unhealthy_generations
            .lock()
            .contains_key(&generation_key);
        let mut selected = session.clone();
        if desired_is_unhealthy
            && let Some(previous) = session_fallback
                .clone()
                .or_else(|| self.previous_generation.lock().clone())
        {
            selected.generation = previous;
            selected.fallback_for = Some(session.generation.clone());
            self.pin_fallback(
                &selected.session_id,
                &selected.generation,
                &session.generation,
            );
        }
        match self.spawn_and_wait_ready(&selected).await {
            Ok(()) => {
                if selected.generation == session.generation {
                    self.unpin_fallback(&session.session_id);
                }
                Ok(())
            }
            Err(error) => {
                if !worker_generation_failure_allows_fallback(&error) {
                    self.clear_generation_failures_for_session(&selected.session_id);
                    self.unpin_fallback(&selected.session_id);
                    return Err(error);
                }
                {
                    let cancelled = self.cancelled_sessions.lock();
                    if cancelled.contains(&selected.session_id) {
                        return Err(error);
                    }
                    // A session-local resume failure must not quarantine a
                    // generation that another worker has already proved
                    // healthy. The failing session can still use its pinned
                    // fallback below; only an unproven rollout is held back
                    // globally while a canary establishes readiness.
                    self.quarantine_generation_if_unproven(
                        &selected.generation,
                        &selected.provider,
                        &selected.session_id,
                    );
                }
                let fallback = session_fallback
                    .clone()
                    .or_else(|| self.previous_generation.lock().clone());
                if let Some(previous) = fallback.filter(|previous| *previous != selected.generation)
                {
                    tracing::warn!(
                        session = %selected.session_id,
                        failed_generation = %selected.generation,
                        fallback_generation = %previous,
                        error = %error,
                        "worker generation failed; falling back"
                    );
                    selected.generation = previous;
                    selected.fallback_for = Some(session.generation.clone());
                    self.pin_fallback(
                        &selected.session_id,
                        &selected.generation,
                        &session.generation,
                    );
                    self.spawn_and_wait_ready(&selected).await.with_context(|| {
                        format!("fallback after generation launch failed: {error}")
                    })
                } else {
                    Err(error)
                }
            }
        }
    }

    async fn spawn_and_wait_ready(&self, session: &StartSession) -> Result<()> {
        if self.cancelled_sessions.lock().contains(&session.session_id) {
            anyhow::bail!("session {} was deleted during launch", session.session_id);
        }
        self.startup_failures.lock().remove(&session.session_id);
        self.session_states
            .lock()
            .insert(session.session_id.clone(), WorkerState::Starting);
        let mut worker_exit = self.spawn_worker(session).await?;
        let deadline = tokio::time::Instant::now() + self.args.worker_ready_timeout;
        loop {
            if self.cancelled_sessions.lock().contains(&session.session_id) {
                self.force_recycle_failed_start(&session.session_id).await;
                anyhow::bail!("session {} was deleted during launch", session.session_id);
            }
            let state = self
                .workers
                .lock()
                .get(&session.session_id)
                .filter(|worker| worker.snapshot.generation == session.generation)
                .map(|worker| worker.snapshot.state)
                .or_else(|| self.session_states.lock().get(&session.session_id).copied());
            match state {
                Some(WorkerState::Running | WorkerState::Busy | WorkerState::Draining) => {
                    return Ok(());
                }
                Some(WorkerState::Exited | WorkerState::Crashed) => {
                    let detail = self.startup_failures.lock().remove(&session.session_id);
                    self.force_recycle_failed_start(&session.session_id).await;
                    if let Some(detail) = detail {
                        anyhow::bail!(
                            "worker {} entered {state:?} before readiness: {detail}",
                            session.session_id
                        );
                    }
                    anyhow::bail!(
                        "worker {} entered {state:?} before readiness",
                        session.session_id
                    );
                }
                Some(WorkerState::Starting) | None => {}
            }
            if tokio::time::Instant::now() >= deadline {
                self.force_recycle_failed_start(&session.session_id).await;
                anyhow::bail!(
                    "worker {} did not become ready within {:?}",
                    session.session_id,
                    self.args.worker_ready_timeout
                );
            }
            tokio::select! {
                exit = &mut worker_exit => {
                    let detail = self.startup_failures.lock().remove(&session.session_id);
                    self.force_recycle_failed_start(&session.session_id).await;
                    if let Some(detail) = detail {
                        anyhow::bail!(
                            "worker {} exited before readiness: {detail}",
                            session.session_id
                        );
                    }
                    match exit {
                        Ok(Ok(status)) => anyhow::bail!(
                            "worker {} exited before readiness with {status}",
                            session.session_id
                        ),
                        Ok(Err(error)) => anyhow::bail!(
                            "waiting for worker {} failed before readiness: {error}",
                            session.session_id
                        ),
                        Err(_) => anyhow::bail!(
                            "worker {} exit monitor stopped before readiness",
                            session.session_id
                        ),
                    }
                }
                () = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        }
    }

    async fn force_recycle_failed_start(&self, session_id: &str) {
        self.route_worker(
            session_id,
            WorkerCommand::Stop {
                command_id: format!("failed-start-stop-{session_id}"),
            },
        );
        if self.args.spawn_mode == SpawnMode::SystemdUser {
            let _ = Command::new("systemctl")
                .args([
                    "--user",
                    "stop",
                    &worker_unit_name(&self.args.socket, session_id),
                ])
                .status()
                .await;
        } else {
            let workers = Arc::clone(&self.direct_worker_pids);
            let pid = workers.lock().get(session_id).copied();
            if let Some(pid) = pid
                && !enforce_direct_worker_exit(
                    workers,
                    session_id.to_owned(),
                    pid,
                    DIRECT_WORKER_GRACEFUL_STOP_TIMEOUT,
                    DIRECT_WORKER_TERM_TIMEOUT,
                    DIRECT_WORKER_KILL_TIMEOUT,
                )
                .await
            {
                tracing::error!(session = %session_id, pid, "failed-start direct worker remained owned after KILL timeout");
            }
        }
        // A worker whose IPC loop is wedged may never consume Stop. Fence it so
        // a fallback generation can register; its stale connection is ignored.
        self.workers.lock().remove(session_id);
    }

    fn set_desired_generation(&self, generation: String, worker_command: Option<String>) {
        if let Some(worker_command) = worker_command {
            self.generation_commands
                .lock()
                .insert(generation.clone(), PathBuf::from(worker_command));
        }
        let live_fallback = self
            .workers
            .lock()
            .values()
            .find(|worker| {
                worker.snapshot.generation != generation
                    && matches!(
                        worker.snapshot.state,
                        WorkerState::Running | WorkerState::Busy | WorkerState::Draining
                    )
            })
            .map(|worker| worker.snapshot.generation.clone());
        let mut generation_changed = false;
        let previous = {
            let mut desired = self.desired_generation.lock();
            if *desired == generation {
                None
            } else if desired.is_empty() {
                *desired = generation.clone();
                generation_changed = true;
                None
            } else {
                let old_desired = std::mem::replace(&mut *desired, generation.clone());
                let previous = live_fallback.clone().unwrap_or(old_desired);
                generation_changed = true;
                *self.previous_generation.lock() = Some(previous.clone());
                Some(previous)
            }
        };
        if generation_changed {
            self.healthy_generations.lock().clear();
            let retained: HashSet<String> = {
                let mut targets = self.fallback_targets.lock();
                targets.retain(|_, failed| failed == &generation);
                targets.keys().cloned().collect()
            };
            self.fallback_pins
                .lock()
                .retain(|session_id, _| retained.contains(session_id));
        }
        if let Some(previous) = previous {
            tracing::info!(%previous, desired = %generation, "worker generation rollout started");
        }
        if self.previous_generation.lock().is_none() {
            let live_previous = self
                .workers
                .lock()
                .values()
                .find(|worker| worker.snapshot.generation != generation)
                .map(|worker| worker.snapshot.generation.clone());
            if let Some(previous) = live_previous {
                *self.previous_generation.lock() = Some(previous.clone());
                tracing::info!(%previous, desired = %generation, "adopted live fallback generation");
            }
        }
        let sessions: Vec<String> = self
            .workers
            .lock()
            .values()
            .filter(|worker| {
                worker.snapshot.generation != generation
                    && self
                        .fallback_pins
                        .lock()
                        .get(&worker.snapshot.session_id)
                        .is_none_or(|pinned| pinned != &worker.snapshot.generation)
            })
            .map(|worker| worker.snapshot.session_id.clone())
            .collect();
        for session_id in sessions {
            let snapshot = if let Some(worker) = self.workers.lock().get_mut(&session_id) {
                worker.snapshot.drain_requested = true;
                Some(worker.snapshot.clone())
            } else {
                None
            };
            if let Some(snapshot) = snapshot {
                self.send_controller(Frame::Snapshot {
                    worker: Box::new(snapshot),
                });
            }
            self.route_worker(&session_id, WorkerCommand::Drain);
        }
    }

    fn roll_provider(&self, provider: &str) {
        let sessions: Vec<(String, StartSession)> = self
            .sessions
            .lock()
            .iter()
            .filter(|(_, session)| session.provider == provider)
            .map(|(session_id, session)| (session_id.clone(), session.clone()))
            .collect();
        tracing::info!(
            provider,
            sessions = sessions.len(),
            "rolling Provider workers"
        );
        for (session_id, session) in sessions {
            self.revoke_cache_protection(&session, &session_id, "provider_roll");
            let snapshot = if let Some(worker) = self.workers.lock().get_mut(&session_id) {
                worker.snapshot.drain_requested = true;
                Some(worker.snapshot.clone())
            } else {
                None
            };
            if let Some(snapshot) = snapshot {
                self.send_controller(Frame::Snapshot {
                    worker: Box::new(snapshot),
                });
                self.route_worker(&session_id, WorkerCommand::Drain);
                self.maybe_cutover(&session_id);
            }
        }
    }

    fn update_from_event(
        &self,
        session_id: &str,
        connection_id: u64,
        runtime_seq: u64,
        event: &RuntimeEvent,
    ) -> Vec<String> {
        let mut workers = self.workers.lock();
        let Some(worker) = workers.get_mut(session_id) else {
            return Vec::new();
        };
        if worker.connection_id != connection_id {
            return Vec::new();
        }
        worker.snapshot.last_runtime_seq = runtime_seq;
        worker.snapshot.observe_native_thread(event);
        match event {
            RuntimeEvent::Ready { .. } => {
                worker.snapshot.state = WorkerState::Running;
                self.startup_failures.lock().remove(session_id);
            }
            RuntimeEvent::Status { state, detail } => {
                worker.snapshot.state = *state;
                if matches!(state, WorkerState::Exited | WorkerState::Crashed) {
                    if let Some(detail) = detail {
                        self.startup_failures
                            .lock()
                            .insert(session_id.to_owned(), detail.clone());
                    }
                } else {
                    self.startup_failures.lock().remove(session_id);
                }
            }
            RuntimeEvent::TurnStarted { turn_id, .. } => {
                worker.snapshot.state = WorkerState::Busy;
                worker.snapshot.current_turn_id = Some(turn_id.clone());
            }
            RuntimeEvent::TurnEnded { turn_id, .. } => {
                if worker.snapshot.current_turn_id.as_deref() == Some(turn_id) {
                    worker.snapshot.current_turn_id = None;
                }
            }
            RuntimeEvent::PermissionRequest { request_id, .. } => {
                if !worker.snapshot.pending_permissions.contains(request_id) {
                    worker.snapshot.pending_permissions.push(request_id.clone());
                }
            }
            RuntimeEvent::PermissionResolved { request_id, .. } => {
                worker
                    .snapshot
                    .pending_permissions
                    .retain(|pending| pending != request_id);
            }
            RuntimeEvent::ConfigOptions { options } => {
                worker.snapshot.config_options = Some(options.clone());
            }
            RuntimeEvent::ContextUsage { used, size, .. } => {
                worker.snapshot.context_used = Some(*used);
                worker.snapshot.context_size = Some(*size);
            }
            RuntimeEvent::Update { update, .. }
                if crate::runtime_wire::background_tasks_count(update).is_some() =>
            {
                worker.snapshot.background_tasks =
                    crate::runtime_wire::background_tasks_count(update);
            }
            RuntimeEvent::AgentSessionId { .. }
            | RuntimeEvent::Update { .. }
            | RuntimeEvent::ScheduleWakeup { .. }
            | RuntimeEvent::UndeliveredPrompt { .. }
            | RuntimeEvent::CommandRejected { .. }
            | RuntimeEvent::Error { .. } => {}
        }
        let state = worker.snapshot.state;
        let generation = worker.snapshot.generation.clone();
        drop(workers);
        self.session_states
            .lock()
            .insert(session_id.to_owned(), state);
        // RemoteSink publishes startup readiness as Status::Running. Ready is
        // retained for older peers, but is not emitted by current ACP workers.
        // Ignoring Running leaves a working generation "unproven", so one
        // missing native thread can quarantine it for every new session.
        if !matches!(
            event,
            RuntimeEvent::Ready { .. }
                | RuntimeEvent::Status {
                    state: WorkerState::Running,
                    ..
                }
        ) || generation != *self.desired_generation.lock()
        {
            return Vec::new();
        }
        let provider = self
            .sessions
            .lock()
            .get(session_id)
            .map_or_else(String::new, |session| session.provider.clone());
        if provider.is_empty() {
            return Vec::new();
        }
        self.rehabilitate_generation(&generation, &provider)
    }

    fn maybe_cutover(&self, session_id: &str) {
        let desired = self.desired_generation.lock().clone();
        if desired.is_empty() {
            return;
        }
        let old_generation = {
            let workers = self.workers.lock();
            let Some(worker) = workers.get(session_id) else {
                return;
            };
            let snapshot = &worker.snapshot;
            if !snapshot.drain_requested
                || snapshot.current_turn_id.is_some()
                || !snapshot.pending_permissions.is_empty()
                || !matches!(snapshot.state, WorkerState::Running | WorkerState::Draining)
            {
                return;
            }
            snapshot.generation.clone()
        };
        if self
            .replacing
            .lock()
            .insert(session_id.to_owned(), old_generation)
            .is_some()
        {
            return;
        }
        self.route_worker(
            session_id,
            WorkerCommand::Stop {
                command_id: format!("rollout-stop-{session_id}"),
            },
        );
    }

    async fn worker_disconnected(
        self: &Arc<Self>,
        session_id: String,
        peer: WorkerPeer,
        exit_detail: Option<String>,
    ) {
        if !self.sessions.lock().contains_key(&session_id) {
            self.session_states.lock().remove(&session_id);
            self.pending_commands.lock().remove(&session_id);
            self.startup_failures.lock().remove(&session_id);
            self.unpin_fallback(&session_id);
            return;
        }
        let Some(old_generation) = self.replacing.lock().remove(&session_id) else {
            let mut snapshot = peer.snapshot;
            snapshot.state = WorkerState::Crashed;
            snapshot.current_turn_id = None;
            snapshot.pending_permissions.clear();
            snapshot.exit_detail = exit_detail;
            self.session_states
                .lock()
                .insert(session_id.clone(), WorkerState::Crashed);
            self.send_controller(Frame::Snapshot {
                worker: Box::new(snapshot),
            });
            return;
        };
        let Some(mut session) = self.sessions.lock().get(&session_id).cloned() else {
            return;
        };
        // A thread that never received a prompt has no Provider transcript;
        // the replacement opens a fresh one instead of a strict resume.
        session.agent_session_id = peer.snapshot.resumable_agent_session_id();
        session.generation = self.desired_generation.lock().clone();
        session.fallback_for = None;
        self.session_states
            .lock()
            .insert(session_id.clone(), WorkerState::Starting);
        self.sessions
            .lock()
            .insert(session_id.clone(), session.clone());
        if !self.launching.lock().insert(session_id.clone()) {
            return;
        }
        let broker = Arc::clone(self);
        tokio::spawn(async move {
            let result = broker
                .spawn_with_fallback(session, Some(old_generation.clone()))
                .await;
            broker.launching.lock().remove(&session_id);
            match result {
                Ok(()) => {
                    tracing::info!(session = %session_id, %old_generation, "worker generation cutover launched")
                }
                Err(error) => {
                    let cancelled = broker.cancelled_sessions.lock();
                    if cancelled.contains(&session_id) {
                        tracing::debug!(session = %session_id, %error, "deleted session rollout stopped");
                        return;
                    }
                    tracing::error!(session = %session_id, %old_generation, %error, "worker cutover and fallback failed");
                    broker.publish_session_state(&session_id, WorkerState::Crashed);
                    broker.command_rejected(
                        &session_id,
                        format!("rollout:{session_id}"),
                        error.to_string(),
                    );
                    drop(cancelled);
                }
            }
        });
    }

    async fn spawn_worker(
        &self,
        session: &StartSession,
    ) -> Result<oneshot::Receiver<std::result::Result<std::process::ExitStatus, String>>> {
        // Verification reads and hashes the retained runtime; keep that file
        // work off the broker's async workers so other sessions keep flowing.
        let verification_started = Instant::now();
        let provider = if session.provider_generation_digest.is_empty() {
            None
        } else {
            let store = Arc::clone(&self.args.provider_store);
            let (provider_id, digest, auth_generation) = (
                session.provider.clone(),
                session.provider_generation_digest.clone(),
                session.provider_auth_generation,
            );
            // An Operator install fences the Plugin until it resolves (about a
            // minute for a Provider upgrade). A start in that window waits for
            // the install rather than reporting a crashed Agent; a fence with
            // no live installer still fails at once for reconciliation.
            let install_deadline = Instant::now() + PROVIDER_INSTALL_LAUNCH_WAIT;
            loop {
                let launch = {
                    let (store, provider_id, digest) =
                        (Arc::clone(&store), provider_id.clone(), digest.clone());
                    tokio::task::spawn_blocking(move || {
                        store.launch_context(&provider_id, &digest, auth_generation)
                    })
                    .await
                    .context("Provider launch verification task failed")?
                };
                match launch {
                    Ok(launch) => break Some(launch),
                    Err(error)
                        if store.install_in_progress(&provider_id)
                            && Instant::now() < install_deadline
                            && !self.cancelled_sessions.lock().contains(&session.session_id) =>
                    {
                        tracing::info!(
                            session = %session.session_id,
                            provider = %provider_id,
                            %error,
                            "worker launch waiting for Provider installation"
                        );
                        tokio::time::sleep(PROVIDER_INSTALL_LAUNCH_POLL).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        };
        tracing::info!(
            session = %session.session_id,
            provider = %session.provider,
            verification_ms = verification_started.elapsed().as_millis(),
            "worker launch context prepared"
        );
        if let (Some(declared), Some(installed)) =
            (session.provider_behavior.as_ref(), provider.as_ref())
        {
            ensure!(
                declared == &installed.behavior,
                "session Provider behavior does not match the installed generation"
            );
        }
        if let Some(installed) = &provider {
            ensure!(
                session.provider_version == installed.version,
                "session Provider version does not match the installed generation"
            );
        }
        let behavior = provider.as_ref().map_or_else(
            || {
                session
                    .provider_behavior
                    .clone()
                    .unwrap_or_else(|| crate::provider_behavior::legacy_behavior(&session.provider))
            },
            |provider| provider.behavior.clone(),
        );
        if let Some(binding) = &session.execution_binding {
            let binding = binding.decode().map_err(anyhow::Error::msg)?;
            ensure!(
                behavior
                    .execution
                    .as_ref()
                    .is_some_and(|execution| execution.accepts(
                        binding.environment.protocol,
                        &binding.environment.executor_digest
                    )),
                "Provider has not accepted this exact execution component"
            );
            ensure!(
                binding.runtime.cwd == session.cwd
                    && provider
                        .as_ref()
                        .is_some_and(|provider| provider.execution_jsonrpc),
                "Provider generation cannot launch this execution binding"
            );
        }
        let mut session_environment =
            session_context_environment(session, &behavior.configuration)?;
        if let Some(provider) = &provider {
            session_environment.push((
                "COWBOY_PROVIDER_PACKAGE_PATH",
                provider.package_path.display().to_string(),
            ));
            session_environment.push(("COWBOY_PROVIDER_ENTRYPOINT", provider.command.clone()));
            if let Some(home) = &provider.home {
                session_environment.push(("HOME", home.display().to_string()));
            }
        }
        let worker_command = self
            .generation_commands
            .lock()
            .get(&session.generation)
            .cloned()
            .with_context(|| {
                format!(
                    "no worker executable registered for generation {}",
                    session.generation
                )
            })?;
        if self.args.spawn_mode == SpawnMode::Direct {
            let workers = Arc::clone(&self.direct_worker_pids);
            let existing_pid = workers.lock().get(&session.session_id).copied();
            if let Some(existing_pid) = existing_pid {
                tracing::warn!(
                    session = %session.session_id,
                    pid = existing_pid,
                    "waiting for previous direct worker owner before launch"
                );
                ensure!(
                    enforce_direct_worker_exit(
                        Arc::clone(&workers),
                        session.session_id.clone(),
                        existing_pid,
                        DIRECT_WORKER_GRACEFUL_STOP_TIMEOUT,
                        DIRECT_WORKER_TERM_TIMEOUT,
                        DIRECT_WORKER_KILL_TIMEOUT,
                    )
                    .await,
                    "previous direct worker {} pid {} did not exit before replacement",
                    session.session_id,
                    existing_pid
                );
            }
            ensure!(
                !workers.lock().contains_key(&session.session_id),
                "direct worker {} still has a live process owner",
                session.session_id
            );
        }
        let mut command = match self.args.spawn_mode {
            SpawnMode::Direct => {
                let mut command = Command::new(&worker_command);
                command.envs(&self.args.worker_environment);
                if let Some(provider) = &provider {
                    for name in &provider.remove_environment {
                        command.env_remove(name);
                    }
                    for (name, _) in std::env::vars().filter(|(name, _)| {
                        provider
                            .remove_environment_prefixes
                            .iter()
                            .any(|prefix| name.starts_with(prefix))
                    }) {
                        command.env_remove(name);
                    }
                    // Apply signed Provider and credential projections after
                    // inherited-variable scrubbing. Otherwise an auth value
                    // with a scrubbed prefix can disappear only when the
                    // broker happens to inherit a variable with the same name.
                    command.envs(&provider.environment);
                }
                for (name, value) in &session_environment {
                    command.env(name, value);
                }
                command
            }
            SpawnMode::SystemdUser => {
                let mut command = Command::new("systemd-run");
                let unit = worker_unit_name(&self.args.socket, &session.session_id);
                prepare_transient_unit(&unit).await?;
                command.args([
                    "--user",
                    "--quiet",
                    "--wait",
                    "--collect",
                    "--service-type=exec",
                    "--property=KillMode=control-group",
                    "--property=Restart=no",
                    "--property=Delegate=yes",
                    "--property=TimeoutStopSec=15s",
                    "--property=Slice=cowboy-agents.slice",
                    &format!("--unit={unit}"),
                ]);
                for (name, _) in std::env::vars().filter(|(name, _)| {
                    let inherited = matches!(
                        name.as_str(),
                        "HOME" | "USER" | "LOGNAME" | "PATH" | "NPM_CONFIG_PREFIX" | "RUST_LOG"
                    ) || name.starts_with("COWBOY_ACP_");
                    inherited
                        && !provider.as_ref().is_some_and(|provider| {
                            provider.remove_environment.contains(name)
                                || provider
                                    .remove_environment_prefixes
                                    .iter()
                                    .any(|prefix| name.starts_with(prefix))
                        })
                }) {
                    command.arg(format!("--setenv={name}"));
                }
                for (name, value) in &self.args.worker_environment {
                    if !provider.as_ref().is_some_and(|provider| {
                        provider.remove_environment.contains(name)
                            || provider
                                .remove_environment_prefixes
                                .iter()
                                .any(|prefix| name.starts_with(prefix))
                    }) {
                        command.arg(format!("--setenv={name}={value}"));
                    }
                }
                if let Some(provider) = &provider {
                    for (name, value) in &provider.environment {
                        command.env(name, value);
                        command.arg(format!("--setenv={name}"));
                    }
                }
                for (name, value) in &session_environment {
                    command.env(name, value);
                    command.arg(format!("--setenv={name}"));
                }
                if let Some(fallback_for) = &session.fallback_for {
                    command.arg(format!("--setenv=COWBOY_FALLBACK_FOR={fallback_for}"));
                }
                command.arg(&worker_command);
                command
            }
        };
        command
            .arg("--socket")
            .arg(&self.args.socket)
            .arg("--session-id")
            .arg(&session.session_id)
            .arg("--provider")
            .arg(&session.provider)
            .arg("--provider-version")
            .arg(&session.provider_version)
            .arg("--provider-generation-digest")
            .arg(&session.provider_generation_digest)
            .arg("--cwd")
            .arg(&session.cwd)
            .arg("--generation")
            .arg(&session.generation);
        if let Some(auth_generation) = session.provider_auth_generation {
            command
                .arg("--provider-auth-generation")
                .arg(auth_generation.to_string());
        }
        if let Some(binding) = &session.execution_binding {
            command
                .arg("--execution-binding")
                .arg(serde_json::to_string(binding)?);
        }
        if self.args.spawn_mode == SpawnMode::Direct
            && let Some(fallback_for) = &session.fallback_for
        {
            command.env("COWBOY_FALLBACK_FOR", fallback_for);
        }
        if let Some(resume) = &session.agent_session_id {
            command.arg("--resume").arg(resume);
        }
        if session.system {
            command.arg("--system");
        }
        if self.args.spawn_mode == SpawnMode::Direct {
            // If the broker runtime itself disappears, Tokio dropping the
            // reaper must not orphan a macOS worker outside our ownership.
            command.kill_on_drop(true);
            // Make the worker the leader of an isolated process group. The
            // bounded watchdog can then terminate its whole provider subtree,
            // not just the worker parent.
            command.as_std_mut().process_group(0);
        }
        // Close the final race between the async owner check above and spawn:
        // two concurrent launch tasks must never overwrite each other's PID.
        // `Command::spawn` is synchronous, so this lock is not held across an
        // await point.
        let mut direct_workers = if self.args.spawn_mode == SpawnMode::Direct {
            let workers = self.direct_worker_pids.lock();
            ensure!(
                !workers.contains_key(&session.session_id),
                "direct worker {} acquired a process owner during launch",
                session.session_id
            );
            Some(workers)
        } else {
            None
        };
        let mut child = command
            .spawn()
            .with_context(|| match self.args.spawn_mode {
                SpawnMode::Direct => "spawning worker process",
                SpawnMode::SystemdUser => "starting transient worker unit",
            })?;
        let session_id = session.session_id.clone();
        let direct_pid = if self.args.spawn_mode == SpawnMode::Direct {
            let pid = child
                .id()
                .context("direct worker did not expose its process id")?;
            direct_workers
                .as_mut()
                .expect("direct worker registry lock")
                .insert(session_id.clone(), pid);
            Some(pid)
        } else {
            None
        };
        drop(direct_workers);
        let direct_worker_pids = Arc::clone(&self.direct_worker_pids);
        let (exit_tx, exit_rx) = oneshot::channel();
        tokio::spawn(async move {
            let result = child.wait().await.map_err(|error| error.to_string());
            if let Some(pid) = direct_pid {
                if reap_direct_worker_group(&direct_worker_pids, &session_id, pid).await {
                    let mut workers = direct_worker_pids.lock();
                    if workers.get(&session_id).copied() == Some(pid) {
                        workers.remove(&session_id);
                    }
                } else {
                    tracing::error!(
                        session = %session_id,
                        pid,
                        "retaining direct worker owner fence after process-group cleanup failure"
                    );
                }
            }
            match &result {
                Ok(status) => {
                    tracing::info!(session = %session_id, %status, "worker process exited")
                }
                Err(error) => {
                    tracing::warn!(session = %session_id, %error, "waiting for worker failed")
                }
            }
            let _ = exit_tx.send(result);
        });
        Ok(exit_rx)
    }
}

fn session_context_environment(
    session: &StartSession,
    configuration: &cowboy_provider_sdk::ConfigurationBehavior,
) -> Result<Vec<(&'static str, String)>> {
    let mut environment = match (session.context_window, session.auto_compact_token_limit) {
        (None, None) => Vec::new(),
        (Some(window), Some(compact))
            if crate::deepseek_context::from_launch_values(configuration, window, compact)
                .is_some() =>
        {
            vec![
                (
                    crate::deepseek_context::SESSION_CONTEXT_WINDOW_ENV,
                    window.to_string(),
                ),
                (
                    crate::deepseek_context::SESSION_AUTO_COMPACT_TOKEN_LIMIT_ENV,
                    compact.to_string(),
                ),
            ]
        }
        _ => {
            anyhow::bail!(
                "invalid context budget for provider {}: window={:?}, compact={:?}",
                session.provider,
                session.context_window,
                session.auto_compact_token_limit
            )
        }
    };
    let cache_provider = crate::deepseek_cache::supported_behavior(configuration);
    if cache_provider {
        // Legacy launch snapshots predate the additive policy field. Preserve
        // the product default during a rolling Machine cutover instead of
        // silently enrolling only sessions created after the upgrade.
        let enabled = session.cache_protection.unwrap_or(true);
        environment.push((
            crate::deepseek_cache::SESSION_POLICY_ENV,
            if enabled { "auto" } else { "off" }.to_owned(),
        ));
    } else if session.cache_protection.is_some() {
        anyhow::bail!(
            "cache protection is unavailable for provider {}",
            session.provider
        );
    }
    Ok(environment)
}

/// `systemd-run --collect` removes a transient unit asynchronously after its
/// process exits. A rolling cutover learns about the disconnect before that
/// collection finishes, so immediately reusing the stable per-session unit name
/// races systemd with "already loaded or has a fragment file". Wait for the
/// unit to disappear before either the desired generation or its fallback is
/// launched. A brand-new session returns `not-found` immediately.
async fn prepare_transient_unit(unit: &str) -> Result<()> {
    let deadline = tokio::time::Instant::now() + TRANSIENT_UNIT_COLLECT_TIMEOUT;
    let mut stopped_orphan = false;
    loop {
        let output = Command::new("systemctl")
            .args([
                "--user",
                "show",
                unit,
                "--property=LoadState",
                "--property=ActiveState",
            ])
            .output()
            .await
            .with_context(|| format!("checking transient worker unit {unit}"))?;
        let fields = String::from_utf8_lossy(&output.stdout);
        let load_state = systemd_show_value(&fields, "LoadState").unwrap_or_default();
        let active_state = systemd_show_value(&fields, "ActiveState").unwrap_or_default();
        if transient_unit_collected(&load_state) {
            return Ok(());
        }
        // Reaching spawn_worker means this broker has no attached peer and no
        // launch owner for the session. An active same-name transient unit is
        // therefore an orphan from the retired broker socket, not a worker we
        // can safely adopt. Stop that exact per-session unit once, then wait for
        // --collect to remove it before reusing the stable name.
        if !stopped_orphan && matches!(active_state.as_str(), "active" | "activating") {
            tracing::warn!(%unit, %active_state, "recycling orphaned transient worker unit");
            let status = Command::new("systemctl")
                .args(["--user", "stop", unit])
                .status()
                .await
                .with_context(|| format!("stopping orphaned transient worker unit {unit}"))?;
            if !status.success() {
                // `show` and `stop` are necessarily separate systemd calls. A
                // transient unit can finish and be collected between them, in
                // which case `stop` returns "unit not loaded" even though the
                // state we need has already been reached. Re-check before
                // treating the stop status as a launch failure.
                let output = Command::new("systemctl")
                    .args(["--user", "show", unit, "--property=LoadState"])
                    .output()
                    .await
                    .with_context(|| {
                        format!("rechecking transient worker unit {unit} after stop")
                    })?;
                let fields = String::from_utf8_lossy(&output.stdout);
                let load_state = systemd_show_value(&fields, "LoadState").unwrap_or_default();
                if transient_unit_collected(&load_state) {
                    return Ok(());
                }
                anyhow::bail!("systemctl stop {unit} exited {status} (LoadState={load_state})");
            }
            stopped_orphan = true;
            continue;
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "transient worker unit {unit} was not collected (LoadState={load_state}, ActiveState={active_state})"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn transient_unit_collected(load_state: &str) -> bool {
    // A failed `systemctl show` may mean the user bus itself is unavailable,
    // not that this unit has been collected. Deletion callers need an
    // affirmative systemd state instead of inferring owner exit from an error.
    load_state == "not-found"
}

fn systemd_show_value(output: &str, key: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.split_once('=').filter(|(name, _)| *name == key))
        .map(|(_, value)| value.trim().to_owned())
}

fn worker_unit_name(socket: &Path, session_id: &str) -> String {
    use sha2::Digest as _;

    let namespace = format!(
        "{:x}",
        sha2::Sha256::digest(socket.as_os_str().as_encoded_bytes())
    );
    let safe: String = session_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    format!("cowboy-worker-{}-{safe}", &namespace[..12])
}

fn worker_command_id(command: &WorkerCommand) -> Option<&str> {
    match command {
        WorkerCommand::Prompt { command_id, .. }
        | WorkerCommand::Cancel { command_id }
        | WorkerCommand::Permission { command_id, .. }
        | WorkerCommand::SetConfigOption { command_id, .. }
        | WorkerCommand::Stop { command_id } => Some(command_id),
        WorkerCommand::Drain => None,
    }
}

#[cfg(test)]
async fn run(args: MachineBrokerArgs) -> Result<()> {
    run_broker(args, None, None, None).await
}

/// Default production builds stay read-only. The dedicated writer build must
/// pass its component owner's exact selection and reader-floor admission.
pub(crate) async fn run_with_deletion_reader(
    args: MachineBrokerArgs,
    path: PathBuf,
    cleanup_path: PathBuf,
    incarnation_path: PathBuf,
    owner: deletions::Owner,
) -> Result<()> {
    let writer_enabled =
        crate::session_deletion_admission::owner_writer::admitted(&path, &owner.machine_id)
            .context("admitting component Session deletion writer")?;
    let cleanup_owner = owner.clone();
    let incarnation_owner = owner.clone();
    // Both writers are admitted before any durable namespace is opened, so a
    // refusal leaves every namespace exactly as it was.
    let incarnation_writer_enabled = writer_enabled
        && crate::session_deletion_admission::owner_writer::admitted_dataset(
            crate::session_deletion_admission::owner_writer::Dataset::Incarnation,
            &incarnation_path,
            &incarnation_owner.machine_id,
        )
        .context("admitting component Session incarnation writer")?;
    let journal =
        tokio::task::spawn_blocking(move || deletions::Journal::open(&path, owner, writer_enabled))
            .await
            .context("joining Session deletion reader open")??;
    tracing::info!(
        deleted_sessions = journal.deleted().len(),
        writer_enabled,
        "Session deletion journal reader ready"
    );
    // Every build reads and validates the incarnation namespace and refuses to
    // start on invalid state, like the deletion journal. Only the dedicated
    // writer build behind the incarnation floor, and only together with an
    // admitted deletion writer, may write it (admitted above, before any
    // namespace was opened).
    let incarnations = tokio::task::spawn_blocking(move || {
        incarnations::Store::open(
            &incarnation_path,
            &incarnation_owner,
            incarnation_writer_enabled,
        )
    })
    .await
    .context("joining Session incarnation open")?
    .context("opening Session incarnation namespace")?;
    tracing::info!(
        incarnations = incarnations.len(),
        writer_enabled = incarnation_writer_enabled,
        "Session incarnation namespace ready"
    );
    // Cleanup effects resume from durable state only on a Machine already
    // admitted to write terminal deletions. The advisory namespace can never
    // keep the resident Machine from starting.
    let continuations = if writer_enabled {
        match tokio::task::spawn_blocking(move || {
            cleanups::Store::open(&cleanup_path, cleanup_owner)
        })
        .await
        .context("joining cleanup continuation open")
        {
            Ok(Ok(store)) => {
                tracing::info!(
                    pending = store.pending().len(),
                    "durable Session cleanup continuations ready"
                );
                Some(store)
            }
            Ok(Err(error)) | Err(error) => {
                tracing::warn!(%error, "durable Session cleanup continuations unavailable");
                None
            }
        }
    } else {
        None
    };
    run_broker(args, Some(journal), continuations, Some(incarnations)).await
}

async fn run_broker(
    args: MachineBrokerArgs,
    journal: Option<deletions::Journal>,
    continuations: Option<cleanups::Store>,
    incarnations: Option<incarnations::Store>,
) -> Result<()> {
    let broker = Arc::new(Broker::new(args));
    if let Some(store) = incarnations {
        *broker.incarnations.lock() = Some(store);
    }
    if let Some(journal) = journal {
        broker.attach_deletion_journal(journal);
    }
    if let Some(store) = continuations {
        broker.attach_cleanup_continuations(store);
        broker.resume_cleanup_continuations();
    }
    let listener = match inherited_systemd_listener()? {
        Some(listener) => {
            tracing::info!(socket = %broker.args.socket.display(), "cowboy Machine broker using systemd socket");
            listener
        }
        None => bind_runtime_listener(&broker.args.socket).await?,
    };
    // A cancelled server must not retain the durable namespace's owner lock
    // through an orphaned monitor task.
    let _monitor = BrokerMonitor(tokio::spawn(monitor_workers(Arc::clone(&broker))));
    accept_runtime_peers(broker.args.socket.clone(), listener, broker).await
}

struct BrokerMonitor(tokio::task::JoinHandle<()>);

impl Drop for BrokerMonitor {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn bind_runtime_listener(socket: &Path) -> Result<UnixListener> {
    if let Some(parent) = socket.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating runtime socket dir {}", parent.display()))?;
    }
    remove_stale_socket(socket).await?;
    let listener = UnixListener::bind(socket)
        .with_context(|| format!("binding Machine broker socket {}", socket.display()))?;
    tracing::info!(socket = %socket.display(), "cowboy Machine broker listening");
    Ok(listener)
}

async fn accept_runtime_peers(
    socket: PathBuf,
    listener: UnixListener,
    broker: Arc<Broker>,
) -> Result<()> {
    loop {
        let (stream, _) = listener.accept().await.context("accepting runtime peer")?;
        let broker = Arc::clone(&broker);
        tokio::spawn(async move {
            if let Err(error) = handle_peer(broker, stream).await {
                tracing::warn!(%error, "Machine runtime peer disconnected with error");
            }
        });
        tracing::trace!(socket = %socket.display(), "accepted Machine runtime peer");
    }
}

/// Adopt fd 3 when launched by a systemd `.socket` unit. Socket ownership then
/// stays outside Machine broker, so connections queue while the broker binary rolls.
#[allow(
    unsafe_code,
    reason = "systemd transfers ownership of the inherited socket descriptor"
)]
fn inherited_systemd_listener() -> Result<Option<UnixListener>> {
    let listen_pid = std::env::var("LISTEN_PID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok());
    let listen_fds = std::env::var("LISTEN_FDS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    if listen_pid != Some(std::process::id()) || listen_fds == 0 {
        return Ok(None);
    }
    if listen_fds != 1 {
        anyhow::bail!("expected exactly one systemd socket, received {listen_fds}");
    }
    // SAFETY: systemd's socket-activation contract assigns the first inherited
    // descriptor to fd 3 and transfers ownership to this process.
    let listener = unsafe { std::os::unix::net::UnixListener::from_raw_fd(3) };
    listener
        .set_nonblocking(true)
        .context("setting inherited Machine broker socket nonblocking")?;
    UnixListener::from_std(listener)
        .map(Some)
        .context("adopting inherited Machine broker socket")
}

async fn monitor_workers(broker: Arc<Broker>) {
    let mut interval = tokio::time::interval(WORKER_MONITOR_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let sweep =
            broker.take_stale_workers(WORKER_HEARTBEAT_TIMEOUT, WORKER_BACKLOG_ISOLATION_LIMIT);
        for (session_id, unread_bytes, silent) in sweep.lagging {
            tracing::warn!(
                session = %session_id,
                unread_bytes,
                silent_seconds = silent.as_secs(),
                "Machine broker has not consumed queued worker output; deferring heartbeat isolation"
            );
        }
        for (session_id, peer, detail) in sweep.isolated {
            tracing::error!(
                session = %session_id,
                generation = %peer.snapshot.generation,
                %detail,
                "worker heartbeat timed out; isolating affected session"
            );
            let _ = peer.tx.send(Frame::WorkerCommand {
                session_id: session_id.clone(),
                command: WorkerCommand::Stop {
                    command_id: format!("heartbeat-timeout-{session_id}"),
                },
            });
            if broker.args.spawn_mode == SpawnMode::SystemdUser {
                let stop = Command::new("systemctl")
                    .args([
                        "--user",
                        "stop",
                        &worker_unit_name(&broker.args.socket, &session_id),
                    ])
                    .status();
                match tokio::time::timeout(Duration::from_secs(10), stop).await {
                    Ok(Ok(status)) if status.success() => {}
                    Ok(Ok(status)) => {
                        tracing::warn!(session = %session_id, %status, "stopping stale worker failed")
                    }
                    Ok(Err(error)) => {
                        tracing::warn!(session = %session_id, %error, "stopping stale worker failed")
                    }
                    Err(_) => {
                        tracing::warn!(session = %session_id, "stopping stale worker timed out")
                    }
                }
            } else {
                // A stale direct worker may no longer service IPC even though
                // its process (or Provider descendants) is still live.
                broker.arm_direct_worker_stop(&session_id);
            }
            broker
                .worker_disconnected(session_id, peer, Some(detail))
                .await;
        }
    }
}

async fn remove_stale_socket(path: &Path) -> Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("removing stale socket {}", path.display()))
        }
    }
}

async fn handle_peer(broker: Arc<Broker>, stream: UnixStream) -> Result<()> {
    let connection_id = broker.next_connection.fetch_add(1, Ordering::Relaxed);
    let (mut reader, mut writer) = stream.into_split();
    let hello = read_frame(&mut reader)
        .await?
        .ok_or_else(|| anyhow::anyhow!("peer closed before hello"))?;
    let Frame::Hello {
        role,
        min_protocol,
        max_protocol,
        session_id,
        worker_epoch,
        generation,
        executable,
        fallback_for,
        ..
    } = hello
    else {
        anyhow::bail!("first runtime frame was not hello");
    };
    let Some(protocol) = negotiate(
        MIN_PROTOCOL_VERSION,
        PROTOCOL_VERSION,
        min_protocol,
        max_protocol,
    ) else {
        write_frame(
            &mut writer,
            &Frame::Reject {
                reason: format!(
                    "no protocol overlap: Machine broker {MIN_PROTOCOL_VERSION}..={PROTOCOL_VERSION}, peer {min_protocol}..={max_protocol}"
                ),
            },
        )
        .await?;
        return Ok(());
    };
    let (tx, mut rx) = mpsc::unbounded_channel::<Frame>();
    let writer_task = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if write_frame(&mut writer, &frame).await.is_err() {
                break;
            }
        }
    });
    match role {
        PeerRole::Core => {
            let lease = broker.install_controller(tx.clone());
            let _ = tx.send(Frame::Welcome {
                protocol,
                controller_epoch: lease,
                workers: broker.snapshots(),
            });
            for worker in broker.workers.lock().values() {
                let _ = worker.tx.send(Frame::Replay {
                    session_id: worker.snapshot.session_id.clone(),
                    worker_epoch: worker.epoch.clone(),
                    after_runtime_seq: worker.snapshot.last_runtime_seq,
                });
            }
            handle_core(Arc::clone(&broker), lease, &mut reader).await?;
            broker.remove_controller(lease);
        }
        PeerRole::Worker => {
            let session_id =
                session_id.ok_or_else(|| anyhow::anyhow!("worker hello missing session"))?;
            let epoch =
                worker_epoch.ok_or_else(|| anyhow::anyhow!("worker hello missing epoch"))?;
            let generation = generation.unwrap_or_else(|| "unknown".to_owned());
            if let Err(error) = broker.register_worker(WorkerRegistration {
                session_id: session_id.clone(),
                epoch: epoch.clone(),
                generation: generation.clone(),
                executable,
                fallback_for,
                connection_id,
                tx: tx.clone(),
            }) {
                let _ = tx.send(Frame::Reject {
                    reason: error.to_string(),
                });
                drop(tx);
                let _ = writer_task.await;
                return Ok(());
            }
            broker.attach_worker_receive_probe(&session_id, connection_id, reader.as_ref().as_fd());
            let lease = broker
                .controller
                .lock()
                .as_ref()
                .map_or(0, |controller| controller.lease);
            let _ = tx.send(Frame::Welcome {
                protocol,
                controller_epoch: lease,
                workers: Vec::new(),
            });
            let draining = broker
                .workers
                .lock()
                .get(&session_id)
                .is_some_and(|worker| worker.snapshot.drain_requested);
            if draining {
                broker.route_worker(&session_id, WorkerCommand::Drain);
            }
            broker.flush_pending(&session_id);
            let worker_result = handle_worker(
                Arc::clone(&broker),
                connection_id,
                &session_id,
                &epoch,
                &mut reader,
            )
            .await;
            if let Some(peer) = broker.remove_worker(&session_id, connection_id) {
                broker.worker_disconnected(session_id, peer, None).await;
            }
            worker_result?;
        }
    }
    drop(tx);
    writer_task.abort();
    Ok(())
}

async fn handle_core(
    broker: Arc<Broker>,
    lease: u64,
    reader: &mut tokio::net::unix::OwnedReadHalf,
) -> Result<()> {
    // Core commands such as an explicit reset can wait for a previous worker
    // launch to stop. Keep that lifecycle work serialized, but move it off the
    // frame reader so a long command cannot starve controller heartbeats.
    let (command_tx, mut command_rx) = mpsc::channel(CORE_COMMAND_QUEUE_CAPACITY);
    let command_broker = Arc::clone(&broker);
    let command_task = tokio::spawn(async move {
        while let Some(command) = command_rx.recv().await {
            handle_core_command(&command_broker, command).await;
        }
    });

    let result = async {
        while let Some(frame) = read_frame(reader).await? {
            if broker.controller_for(lease).is_none() {
                tracing::warn!(lease, "ignoring command from fenced controller");
                continue;
            }
            match frame {
                Frame::ExecutionReply { reply } => {
                    if let Some(worker) = broker.workers.lock().get(&reply.session_id)
                        && worker.epoch == reply.worker_epoch
                        && worker
                            .snapshot
                            .launch
                            .as_ref()
                            .and_then(|launch| launch.execution_binding.as_ref())
                            .and_then(|binding| binding.decode().ok())
                            .is_some_and(|binding| {
                                crate::execution_protocol::Scope::from_binding(&binding)
                                    == reply.scope
                            })
                    {
                        let _ = worker.tx.send(Frame::ExecutionReply { reply });
                    }
                }
                Frame::CoreCommand { command } => {
                    command_tx.try_send(command).map_err(|error| {
                        anyhow::anyhow!(
                            "Machine core command queue saturated; reconnecting controller: {error}"
                        )
                    })?
                }
                Frame::Ack {
                    session_id,
                    worker_epoch,
                    runtime_seq,
                } => {
                    if let Some(worker) = broker.workers.lock().get(&session_id)
                        && worker.epoch == worker_epoch
                    {
                        let _ = worker.tx.send(Frame::Ack {
                            session_id,
                            worker_epoch,
                            runtime_seq,
                        });
                    }
                }
                Frame::Heartbeat => {
                    if let Some(tx) = broker.controller_for(lease) {
                        let _ = tx.send(Frame::Heartbeat);
                    }
                }
                other => tracing::debug!(?other, "ignoring non-core runtime frame"),
            }
        }
        Ok(())
    }
    .await;

    command_task.abort();
    let _ = command_task.await;
    result
}

async fn handle_core_command(broker: &Arc<Broker>, command: CoreCommand) {
    match command {
        CoreCommand::EnsureSession { mut session } => {
            if session.generation.is_empty() {
                session.generation = broker.desired_generation.lock().clone();
            }
            broker.ensure_session(session).await;
        }
        CoreCommand::Prompt {
            session_id,
            command_id,
            turn_id,
            content,
            cmid,
            trace,
            echo_artifacts,
        } => broker.route_prompt(
            &session_id,
            WorkerCommand::Prompt {
                command_id,
                turn_id,
                content,
                cmid,
                trace,
                echo_artifacts,
            },
        ),
        CoreCommand::Cancel {
            session_id,
            command_id,
        } => broker.route_worker(&session_id, WorkerCommand::Cancel { command_id }),
        CoreCommand::Permission {
            session_id,
            command_id,
            request_id,
            option_id,
        } => broker.route_worker(
            &session_id,
            WorkerCommand::Permission {
                command_id,
                request_id,
                option_id,
            },
        ),
        CoreCommand::SetConfigOption {
            session_id,
            command_id,
            config_id,
            value,
        } => {
            if let Some(session) = broker.sessions.lock().get(&session_id).cloned() {
                broker.revoke_cache_protection(&session, &session_id, "agent_config_changed");
            }
            broker.route_worker(
                &session_id,
                WorkerCommand::SetConfigOption {
                    command_id,
                    config_id,
                    value,
                },
            );
        }
        CoreCommand::DrainSession { session_id, .. } => {
            let snapshot = if let Some(worker) = broker.workers.lock().get_mut(&session_id) {
                worker.snapshot.drain_requested = true;
                Some(worker.snapshot.clone())
            } else {
                None
            };
            if let Some(snapshot) = snapshot {
                broker.send_controller(Frame::Snapshot {
                    worker: Box::new(snapshot),
                });
            }
            broker.route_worker(&session_id, WorkerCommand::Drain);
        }
        CoreCommand::HibernateSession {
            session_id,
            command_id,
        } => broker.hibernate_session(session_id, command_id).await,
        CoreCommand::StopSession {
            session_id,
            command_id,
        } if command_id.starts_with("reset-") => {
            let Some(session) = broker.sessions.lock().get(&session_id).cloned() else {
                broker.command_rejected(
                    &session_id,
                    command_id,
                    "reset session metadata was not declared".to_owned(),
                );
                return;
            };
            broker.reset_session(session, command_id).await;
        }
        CoreCommand::StopSession {
            session_id,
            command_id,
        } => {
            // Serialize permanent deletion with reset and asynchronous target
            // cleanup. Whichever controller command acquires this fence last
            // owns the session lifecycle decision.
            let cleanup_gate = broker.session_lifecycle_gate(&session_id);
            let _cleanup_guard = cleanup_gate.lock().await;
            if let Err(error) = broker.record_deletion(&session_id).await {
                broker.command_rejected(
                    &session_id,
                    command_id,
                    format!("durable Session deletion was not confirmed: {error}"),
                );
                return;
            }
            // The journal commit above is the deletion decision; a record that
            // cannot be dropped is harmless because every reader treats the ID
            // as terminal.
            if let Err(error) = broker.end_incarnation(&session_id).await {
                tracing::warn!(session = %session_id, %error,
                    "deleted Session's incarnation record was retained");
            }
            let cleanup_command_id = command_id.clone();
            let cleanup_session = broker.sessions.lock().get(&session_id).cloned();
            let cleanup_cwd = cleanup_session.as_ref().map(|session| session.cwd.clone());
            if let Some(session) = cleanup_session {
                broker.revoke_cache_protection(&session, &session_id, "session_deleted");
            }
            broker.resetting_sessions.lock().remove(&session_id);
            {
                // Scoped, not dropped: the guard must not be live across the
                // continuation write below.
                let mut cancelled = broker.cancelled_sessions.lock();
                cancelled.insert(session_id.clone());
                broker.clear_generation_failures_for_session(&session_id);
                broker.sessions.lock().remove(&session_id);
                broker.awaiting_reconnect.lock().remove(&session_id);
                broker.session_states.lock().remove(&session_id);
                broker.pending_commands.lock().remove(&session_id);
                broker.startup_failures.lock().remove(&session_id);
                broker.unpin_fallback(&session_id);
            }
            if let Some(cwd) = cleanup_cwd {
                match crate::session_workspace::capture_cleanup_workspace(
                    &broker.args.worktree_root,
                    &session_id,
                    Path::new(&cwd),
                ) {
                    Ok(workspace) => {
                        match workspace.root_identity() {
                            Ok(root) => {
                                broker.record_cleanup_continuation(&session_id, root).await;
                            }
                            Err(error) => tracing::warn!(session = %session_id, %error,
                                "deleted session cleanup will not resume after a Machine restart"),
                        }
                        broker.deleted_session_workspaces.lock().insert(
                            session_id.clone(),
                            DeletedWorkspace {
                                workspace,
                                command_id: cleanup_command_id.clone(),
                            },
                        );
                    }
                    Err(error) => tracing::warn!(session = %session_id, %error,
                        "preserving deleted session artifacts because the worktree could not be captured"),
                }
            }
            if broker.workers.lock().contains_key(&session_id) {
                broker.route_worker(&session_id, WorkerCommand::Stop { command_id });
            } else {
                broker.send_controller(Frame::CommandAck {
                    session_id: session_id.clone(),
                    command_id,
                    accepted: true,
                    reason: None,
                });
                // The IPC peer can disappear before the owned direct process.
                // Deletion must still arm the process-group watchdog.
                broker.arm_direct_worker_stop(&session_id);
            }
            broker.cleanup_deleted_session(&session_id, &cleanup_command_id);
        }
        CoreCommand::SetDesiredGeneration {
            generation,
            worker_command,
        } => {
            broker.set_desired_generation(generation, worker_command);
        }
        CoreCommand::RollProvider { provider } => broker.roll_provider(&provider),
    }
}

async fn handle_worker(
    broker: Arc<Broker>,
    connection_id: u64,
    session_id: &str,
    epoch: &str,
    reader: &mut tokio::net::unix::OwnedReadHalf,
) -> Result<()> {
    while let Some(frame) = read_frame(reader).await? {
        broker.touch_worker(session_id, connection_id);
        if !broker.worker_matches(session_id, connection_id, epoch) {
            tracing::warn!(session = session_id, %epoch, "ignoring frame from fenced worker");
            continue;
        }
        match frame {
            Frame::ExecutionRequest { request } => {
                let allowed = request.session_id == session_id
                    && request.worker_epoch == epoch
                    && !broker.cancelled_sessions.lock().contains(session_id)
                    && broker
                        .sessions
                        .lock()
                        .get(session_id)
                        .and_then(|launch| launch.execution_binding.as_ref())
                        .and_then(|binding| binding.decode().ok())
                        .is_some_and(|binding| binding == request.binding);
                if allowed {
                    broker.send_controller(Frame::ExecutionRequest { request });
                } else if let Some(worker) = broker.workers.lock().get(session_id) {
                    let _ = worker.tx.send(Frame::ExecutionReply {
                        reply: Box::new(crate::execution_protocol::RuntimeReply {
                            session_id: session_id.to_owned(),
                            worker_epoch: epoch.to_owned(),
                            request_id: request.request_id,
                            scope: crate::execution_protocol::Scope::from_binding(&request.binding),
                            response: crate::machine_protocol::execution::Response::Refused {
                                reason:
                                    crate::machine_protocol::execution::Refusal::IdentityMismatch,
                            },
                        }),
                    });
                }
            }
            Frame::Snapshot { worker } if worker.session_id == session_id => {
                if !broker.update_snapshot((*worker).clone(), connection_id) {
                    continue;
                }
                let cancelled = broker.cancelled_sessions.lock();
                if cancelled.contains(session_id) {
                    continue;
                }
                let provider = broker
                    .sessions
                    .lock()
                    .get(session_id)
                    .map_or_else(String::new, |session| session.provider.clone());
                let desired = broker.desired_generation.lock().clone();
                let rehabilitated = if worker.generation == desired
                    && matches!(
                        worker.state,
                        WorkerState::Running | WorkerState::Busy | WorkerState::Draining
                    ) {
                    broker.rehabilitate_generation(&worker.generation, &provider)
                } else {
                    Vec::new()
                };
                let worker = broker
                    .workers
                    .lock()
                    .get(session_id)
                    .map_or_else(|| *worker, |peer| peer.snapshot.clone());
                broker.send_controller(Frame::Snapshot {
                    worker: Box::new(worker),
                });
                drop(cancelled);
                broker.maybe_cutover(session_id);
                for fallback_session in rehabilitated {
                    if fallback_session != session_id {
                        broker.maybe_cutover(&fallback_session);
                    }
                }
            }
            Frame::WorkerEvent {
                session_id: event_session,
                worker_epoch,
                runtime_seq,
                event,
                diagnostics,
            } if event_session == session_id && worker_epoch == epoch => {
                let cancelled = broker.cancelled_sessions.lock();
                if cancelled.contains(session_id) {
                    // The event is intentionally not forwarded after permanent
                    // deletion, but it was consumed by this broker. ACK it so
                    // the stopping worker can drain its final Exited event and
                    // terminate instead of retaining an unbounded outbox.
                    drop(cancelled);
                    broker.acknowledge_worker_event(
                        session_id,
                        connection_id,
                        worker_epoch,
                        runtime_seq,
                    );
                    continue;
                }
                let rehabilitated =
                    broker.update_from_event(session_id, connection_id, runtime_seq, &event);
                broker.send_controller(Frame::WorkerEvent {
                    session_id: event_session,
                    worker_epoch,
                    runtime_seq,
                    event,
                    diagnostics,
                });
                broker.maybe_cutover(session_id);
                drop(cancelled);
                for fallback_session in rehabilitated {
                    if fallback_session != session_id {
                        broker.maybe_cutover(&fallback_session);
                    }
                }
            }
            Frame::CommandAck { .. } => broker.send_controller(frame),
            Frame::Heartbeat => {}
            other => tracing::debug!(?other, "ignoring non-worker runtime frame"),
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "full"))]
pub(crate) use tests::dispatch_trace_fixture;

#[cfg(test)]
mod tests {
    use super::*;

    mod deletion_process;
    mod incarnation_process;

    #[test]
    fn process_probe_refuses_to_treat_permission_failure_as_exit() {
        assert!(observed_group_exists(Ok(()), 2));
        assert!(!observed_group_exists(Err(rustix::io::Errno::SRCH), 2));
        for error in [
            rustix::io::Errno::PERM,
            rustix::io::Errno::INTR,
            rustix::io::Errno::INVAL,
        ] {
            assert!(observed_group_exists(Err(error), 2));
        }
        for pid in [0, 1, u32::MAX] {
            assert!(direct_worker_group_exists(pid));
        }
    }

    #[test]
    fn signal_requires_original_mapping_and_a_bounded_group() {
        let workers = Mutex::new(HashMap::new());
        for pid in [0, 1, u32::MAX] {
            assert!(
                signal_direct_worker(&workers, "stale", pid, rustix::process::Signal::KILL).is_ok()
            );
            workers.lock().insert("owned".into(), pid);
            assert!(
                signal_direct_worker(&workers, "owned", pid, rustix::process::Signal::KILL)
                    .is_err()
            );
        }
    }

    #[cfg(feature = "full")]
    pub(crate) async fn dispatch_trace_fixture(command: CoreCommand) -> WorkerCommand {
        let root = tempfile::tempdir().unwrap();
        let session = command.session_id().unwrap().to_owned();
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: root.path().join("broker.sock"),
            worker_command: PathBuf::from("false"),
            desired_generation: String::new(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: root.path().join("worktrees"),
            worker_ready_timeout: Duration::from_secs(1),
        }));
        // Replay while no worker exists must retain one original context.
        handle_core_command(&broker, command.clone()).await;
        handle_core_command(&broker, command).await;
        assert_eq!(broker.pending_commands.lock()[&session].len(), 1);
        let (tx, mut rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: session.clone(),
                epoch: "fixture-epoch".into(),
                generation: "fixture-generation".into(),
                executable: None,
                fallback_for: None,
                connection_id: 1,
                tx,
            })
            .unwrap();
        broker.flush_pending(&session);
        let frame = rx.try_recv().unwrap();
        assert!(rx.try_recv().is_err());
        let frame: Frame = serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
        let Frame::WorkerCommand { command, .. } = frame else {
            panic!("worker command");
        };
        command
    }

    fn test_provider_store() -> Arc<crate::machine_plugins::MachinePluginStore> {
        static STORE: std::sync::OnceLock<Arc<crate::machine_plugins::MachinePluginStore>> =
            std::sync::OnceLock::new();
        Arc::clone(STORE.get_or_init(|| {
            Arc::new(
                crate::machine_plugins::MachinePluginStore::new(
                    &std::env::temp_dir().join(format!(
                        "cowboy-machine-broker-provider-test-{}",
                        std::process::id()
                    )),
                    crate::machine_protocol::Platform::Linux,
                    "x86_64".to_owned(),
                )
                .expect("test Provider store"),
            )
        }))
    }

    #[test]
    fn systemd_show_parser_keeps_load_and_active_state_distinct() {
        let output = "ActiveState=active\nLoadState=loaded\n";
        assert_eq!(
            systemd_show_value(output, "LoadState").as_deref(),
            Some("loaded")
        );
        assert_eq!(
            systemd_show_value(output, "ActiveState").as_deref(),
            Some("active")
        );
        assert_eq!(systemd_show_value(output, "SubState"), None);
    }

    #[test]
    fn collection_requires_authoritative_not_found_state() {
        assert!(!transient_unit_collected(""));
        assert!(transient_unit_collected("not-found"));
        assert!(!transient_unit_collected("loaded"));
    }

    #[test]
    fn projected_auth_rejection_never_triggers_worker_generation_fallback() {
        let auth = anyhow::anyhow!(format!(
            "{}Grok credentials expired",
            crate::provider_behavior::PROVIDER_AUTH_REQUIRED_PREFIX
        ));
        let binary = anyhow::anyhow!("worker exited before readiness");
        let resume = anyhow::anyhow!(
            "worker sess-1 exited before readiness: agent did not complete ACP session/resume within 240s"
        );
        let missing = anyhow::anyhow!(
            "worker sess-1 exited before readiness: acp connection: Resource not found: native-thread: {{\"uri\":\"native-thread\"}}"
        );

        assert!(!worker_generation_failure_allows_fallback(&auth));
        assert!(worker_generation_failure_allows_fallback(&binary));
        assert!(!worker_generation_failure_allows_fallback(&resume));
        assert!(!worker_generation_failure_allows_fallback(&missing));
    }

    #[test]
    fn session_context_environment_accepts_only_known_provider_budgets() {
        let mut session = StartSession {
            session_id: "s".to_owned(),
            provider: "claude-deepseek".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_auth_generation: None,
            provider_behavior: None,
            cwd: "/work".to_owned(),
            agent_session_id: None,
            system: false,
            context_window: Some(830_000),
            auto_compact_token_limit: Some(819_200),
            cache_protection: Some(true),
            generation: "gen-1".to_owned(),
            fallback_for: None,
            adopt_only: false,
            execution_binding: None,
        };
        let mut configuration = cowboy_provider_sdk::ConfigurationBehavior::AnthropicGatewayV1;
        assert_eq!(
            session_context_environment(&session, &configuration).expect("known Claude budget"),
            vec![
                (
                    crate::deepseek_context::SESSION_CONTEXT_WINDOW_ENV,
                    "830000".to_owned(),
                ),
                (
                    crate::deepseek_context::SESSION_AUTO_COMPACT_TOKEN_LIMIT_ENV,
                    "819200".to_owned(),
                ),
                (crate::deepseek_cache::SESSION_POLICY_ENV, "auto".to_owned(),),
            ]
        );

        session.auto_compact_token_limit = Some(830_000);
        assert!(session_context_environment(&session, &configuration).is_err());
        session.provider = "claude-code".to_owned();
        configuration = cowboy_provider_sdk::ConfigurationBehavior::PortableV1;
        assert!(session_context_environment(&session, &configuration).is_err());

        session.provider = "codex-deepseek".to_owned();
        configuration = cowboy_provider_sdk::ConfigurationBehavior::OpenaiGatewayV1;
        session.context_window = None;
        session.auto_compact_token_limit = None;
        session.cache_protection = Some(false);
        assert_eq!(
            session_context_environment(&session, &configuration).expect("known cache policy"),
            vec![(crate::deepseek_cache::SESSION_POLICY_ENV, "off".to_owned(),)]
        );

        session.cache_protection = None;
        assert_eq!(
            session_context_environment(&session, &configuration).expect("legacy cache policy"),
            vec![(crate::deepseek_cache::SESSION_POLICY_ENV, "auto".to_owned(),)]
        );

        session.provider = "codex".to_owned();
        configuration = cowboy_provider_sdk::ConfigurationBehavior::PortableV1;
        assert!(
            session_context_environment(&session, &configuration)
                .expect("unrelated legacy session")
                .is_empty()
        );

        session.provider = "grok".to_owned();
        assert!(
            session_context_environment(&session, &configuration)
                .expect("Provider runtime owns workspace-trust projection")
                .is_empty()
        );
    }
    use crate::runtime_wire::{RuntimeEvent, WorkerState};

    fn test_socket() -> PathBuf {
        std::env::temp_dir().join(format!(
            "cowboy-machine-broker-test-{}-{}.sock",
            std::process::id(),
            broker_nonce()
        ))
    }

    fn broker_nonce() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos())
    }

    async fn connect_peer(
        socket: &Path,
        role: PeerRole,
        session_id: Option<&str>,
        epoch: Option<&str>,
    ) -> (
        tokio::net::unix::OwnedReadHalf,
        tokio::net::unix::OwnedWriteHalf,
        Frame,
    ) {
        let stream = UnixStream::connect(socket).await.expect("connect peer");
        let (mut reader, mut writer) = stream.into_split();
        write_frame(
            &mut writer,
            &Frame::Hello {
                role,
                min_protocol: PROTOCOL_VERSION,
                max_protocol: PROTOCOL_VERSION,
                build: "test".to_owned(),
                session_id: session_id.map(str::to_owned),
                worker_epoch: epoch.map(str::to_owned),
                generation: Some("gen-1".to_owned()),
                executable: Some("/bin/false".to_owned()),
                fallback_for: None,
            },
        )
        .await
        .expect("write hello");
        let welcome = read_frame(&mut reader)
            .await
            .expect("read welcome")
            .expect("welcome frame");
        (reader, writer, welcome)
    }

    #[tokio::test]
    async fn core_heartbeat_is_not_blocked_by_a_long_reset_command() {
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_secs(1),
        }));
        broker.sessions.lock().insert(
            "sess-heartbeat-reset".to_owned(),
            StartSession {
                session_id: "sess-heartbeat-reset".to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: "/tmp".to_owned(),
                agent_session_id: None,
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-1".to_owned(),
                fallback_for: None,
                adopt_only: false,
                execution_binding: None,
            },
        );
        broker
            .launching
            .lock()
            .insert("sess-heartbeat-reset".to_owned());

        let (controller_tx, mut controller_rx) = mpsc::unbounded_channel();
        let lease = broker.install_controller(controller_tx);
        let (mut controller, broker_stream) = UnixStream::pair().expect("core socket pair");
        let (mut broker_reader, _broker_writer) = broker_stream.into_split();
        let core_task = tokio::spawn(async move {
            handle_core(Arc::clone(&broker), lease, &mut broker_reader).await
        });

        write_frame(
            &mut controller,
            &Frame::CoreCommand {
                command: CoreCommand::StopSession {
                    session_id: "sess-heartbeat-reset".to_owned(),
                    command_id: "reset-heartbeat".to_owned(),
                },
            },
        )
        .await
        .expect("send long reset command");
        write_frame(&mut controller, &Frame::Heartbeat)
            .await
            .expect("send heartbeat");

        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(100), controller_rx.recv())
                .await
                .expect("heartbeat response timeout")
                .expect("controller response"),
            Frame::Heartbeat
        ));

        drop(controller);
        tokio::time::timeout(Duration::from_secs(1), core_task)
            .await
            .expect("core reader shutdown timeout")
            .expect("core reader task")
            .expect("core reader result");
    }

    #[test]
    fn unit_names_are_safe_and_stable() {
        let first = worker_unit_name(Path::new("/service-a/run.sock"), "sess-123");
        let second = worker_unit_name(Path::new("/service-b/run.sock"), "sess-123");
        assert!(first.starts_with("cowboy-worker-"));
        assert!(first.ends_with("-sess-123"));
        assert_ne!(first, second);
        assert!(
            worker_unit_name(Path::new("/service-a/run.sock"), "weird/id").ends_with("-weird_id")
        );
    }

    #[tokio::test]
    async fn worker_exit_fails_launch_without_waiting_for_readiness_timeout() {
        let broker = Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_secs(10),
        });
        let session = StartSession {
            session_id: "sess-fast-failure".to_owned(),
            provider: "codex".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_auth_generation: None,
            provider_behavior: None,
            cwd: "/tmp".to_owned(),
            agent_session_id: None,
            system: false,
            context_window: None,
            auto_compact_token_limit: None,
            cache_protection: None,
            generation: "gen-1".to_owned(),
            fallback_for: None,
            adopt_only: false,
            execution_binding: None,
        };

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            broker.spawn_and_wait_ready(&session),
        )
        .await
        .expect("worker exit must beat the readiness timeout")
        .expect_err("exited worker must fail launch");

        assert!(
            error.to_string().contains("exited before readiness"),
            "unexpected launch error: {error:#}"
        );
        assert!(!error.to_string().contains("did not become ready"));
    }

    #[test]
    fn generation_rollout_drains_then_stops_only_an_idle_worker() {
        let broker = Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        });
        let (tx, mut rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: "sess-1".to_owned(),
                epoch: "epoch-1".to_owned(),
                generation: "gen-1".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: None,
                connection_id: 1,
                tx,
            })
            .expect("register worker");
        {
            let mut workers = broker.workers.lock();
            let worker = workers.get_mut("sess-1").expect("worker");
            worker.snapshot.state = WorkerState::Busy;
            worker.snapshot.current_turn_id = Some("turn-1".to_owned());
        }
        broker.set_desired_generation("gen-2".to_owned(), Some("/bin/false".to_owned()));
        assert!(matches!(
            rx.try_recv(),
            Ok(Frame::WorkerCommand {
                command: WorkerCommand::Drain,
                ..
            })
        ));
        let prompt = WorkerCommand::Prompt {
            trace: None,
            command_id: "prompt-during-drain".to_owned(),
            turn_id: "turn-2".to_owned(),
            content: vec![serde_json::json!({"type": "text", "text": "next"})],
            cmid: None,
            echo_artifacts: false,
        };
        broker.route_prompt("sess-1", prompt.clone());
        broker.route_prompt("sess-1", prompt);
        assert!(
            rx.try_recv().is_err(),
            "draining worker must not receive a new prompt"
        );
        assert_eq!(
            broker
                .pending_commands
                .lock()
                .get("sess-1")
                .map(VecDeque::len),
            Some(1),
            "controller retries must not duplicate a held prompt"
        );
        broker.maybe_cutover("sess-1");
        assert!(rx.try_recv().is_err(), "busy worker must not be stopped");

        broker.update_from_event(
            "sess-1",
            1,
            2,
            &RuntimeEvent::TurnEnded {
                turn_id: "turn-1".to_owned(),
                stop_reason: "end_turn".to_owned(),
            },
        );
        broker.update_from_event(
            "sess-1",
            1,
            3,
            &RuntimeEvent::Status {
                state: WorkerState::Draining,
                detail: None,
            },
        );
        broker.maybe_cutover("sess-1");
        assert!(matches!(
            rx.try_recv(),
            Ok(Frame::WorkerCommand {
                command: WorkerCommand::Stop { .. },
                ..
            })
        ));
        assert_eq!(
            broker.replacing.lock().get("sess-1").map(String::as_str),
            Some("gen-1")
        );
    }

    #[test]
    fn provider_rollout_drains_and_replaces_matching_idle_workers() {
        let broker = Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        });
        broker.sessions.lock().insert(
            "sess-1".to_owned(),
            StartSession {
                session_id: "sess-1".to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: "/work".to_owned(),
                agent_session_id: Some("agent-1".to_owned()),
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-1".to_owned(),
                fallback_for: None,
                adopt_only: false,
                execution_binding: None,
            },
        );
        let (tx, mut rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: "sess-1".to_owned(),
                epoch: "epoch-1".to_owned(),
                generation: "gen-1".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: None,
                connection_id: 1,
                tx,
            })
            .expect("register worker");
        broker
            .workers
            .lock()
            .get_mut("sess-1")
            .unwrap()
            .snapshot
            .state = WorkerState::Running;

        broker.roll_provider("codex");

        assert!(matches!(
            rx.try_recv(),
            Ok(Frame::WorkerCommand {
                command: WorkerCommand::Drain,
                ..
            })
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(Frame::WorkerCommand {
                command: WorkerCommand::Stop { .. },
                ..
            })
        ));
        assert_eq!(
            broker.replacing.lock().get("sess-1").map(String::as_str),
            Some("gen-1")
        );
    }

    /// Roll an idle worker whose runtime events are `events` and return the
    /// native thread its replacement launch would resume.
    async fn provider_roll_resume_after(events: &[RuntimeEvent]) -> Option<String> {
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        }));
        broker.sessions.lock().insert(
            "sess-1".to_owned(),
            StartSession {
                session_id: "sess-1".to_owned(),
                provider: "claude-code".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: "/work".to_owned(),
                agent_session_id: None,
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-1".to_owned(),
                fallback_for: None,
                adopt_only: false,
                execution_binding: None,
            },
        );
        let (tx, _rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: "sess-1".to_owned(),
                epoch: "epoch-1".to_owned(),
                generation: "gen-1".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: None,
                connection_id: 1,
                tx,
            })
            .expect("register worker");
        for (seq, event) in (1..).zip(events) {
            broker.update_from_event("sess-1", 1, seq, event);
        }
        broker
            .workers
            .lock()
            .get_mut("sess-1")
            .unwrap()
            .snapshot
            .state = WorkerState::Running;

        broker.roll_provider("claude-code");
        let peer = broker.workers.lock().remove("sess-1").expect("worker");
        broker
            .worker_disconnected("sess-1".to_owned(), peer, None)
            .await;
        broker
            .sessions
            .lock()
            .get("sess-1")
            .and_then(|session| session.agent_session_id.clone())
    }

    #[tokio::test]
    async fn provider_roll_opens_a_fresh_thread_for_a_session_without_a_prompt() {
        // Claude and Codex persist a native thread only at its first prompt;
        // a strict resume of a cleared, still-empty session would crash it.
        let resume = provider_roll_resume_after(&[RuntimeEvent::AgentSessionId {
            agent_session_id: "thread-empty".to_owned(),
        }])
        .await;
        assert_eq!(resume, None);
    }

    #[tokio::test]
    async fn provider_roll_resumes_a_thread_that_received_a_prompt() {
        let resume = provider_roll_resume_after(&[
            RuntimeEvent::AgentSessionId {
                agent_session_id: "thread-used".to_owned(),
            },
            RuntimeEvent::TurnStarted {
                turn_id: "turn-1".to_owned(),
                command_id: "command-1".to_owned(),
            },
            RuntimeEvent::TurnEnded {
                turn_id: "turn-1".to_owned(),
                stop_reason: "end_turn".to_owned(),
            },
        ])
        .await;
        assert_eq!(resume.as_deref(), Some("thread-used"));
    }

    #[tokio::test]
    async fn heartbeat_sweep_separates_broker_read_backlog_from_silent_workers() {
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: String::new(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        }));
        let mut sockets = Vec::new();
        for (connection_id, session_id) in [(1, "sess-silent"), (2, "sess-lagging")] {
            let (tx, _rx) = mpsc::unbounded_channel();
            broker
                .register_worker(WorkerRegistration {
                    session_id: session_id.to_owned(),
                    epoch: format!("epoch-{connection_id}"),
                    generation: "gen-1".to_owned(),
                    executable: None,
                    fallback_for: None,
                    connection_id,
                    tx,
                })
                .expect("register worker");
            broker.sessions.lock().insert(
                session_id.to_owned(),
                StartSession {
                    session_id: session_id.to_owned(),
                    provider: "codex".to_owned(),
                    provider_version: String::new(),
                    provider_generation_digest: String::new(),
                    provider_auth_generation: None,
                    provider_behavior: None,
                    cwd: "/tmp".to_owned(),
                    agent_session_id: None,
                    system: false,
                    context_window: None,
                    auto_compact_token_limit: None,
                    cache_protection: None,
                    generation: "gen-1".to_owned(),
                    fallback_for: None,
                    adopt_only: false,
                    execution_binding: None,
                },
            );
            let (broker_end, worker_end) = UnixStream::pair().expect("worker socket pair");
            broker.attach_worker_receive_probe(session_id, connection_id, broker_end.as_fd());
            broker
                .workers
                .lock()
                .get_mut(session_id)
                .expect("registered worker")
                .last_seen = Instant::now()
                .checked_sub(Duration::from_secs(60))
                .expect("monotonic clock origin");
            sockets.push((broker_end, worker_end));
        }
        // The lagging worker already delivered a frame the broker never read.
        write_frame(&mut sockets[1].1, &Frame::Heartbeat)
            .await
            .expect("queue worker heartbeat");

        let sweep = broker.take_stale_workers(Duration::from_secs(45), Duration::from_secs(300));
        assert_eq!(sweep.lagging.len(), 1);
        assert_eq!(sweep.lagging[0].0, "sess-lagging");
        assert!(sweep.lagging[0].1 > 0);
        assert!(broker.workers.lock().contains_key("sess-lagging"));
        let [(session_id, peer, detail)] = <[_; 1]>::try_from(sweep.isolated).ok().unwrap();
        assert_eq!(session_id, "sess-silent");
        assert!(detail.starts_with("worker heartbeat timed out after 60s"));

        let (controller_tx, mut controller_rx) = mpsc::unbounded_channel();
        broker.install_controller(controller_tx);
        broker
            .worker_disconnected(session_id, peer, Some(detail.clone()))
            .await;
        let Some(Frame::Snapshot { worker }) = controller_rx.recv().await else {
            panic!("crashed worker snapshot");
        };
        assert_eq!(worker.state, WorkerState::Crashed);
        assert_eq!(worker.exit_detail, Some(detail));

        // A backlog that never drains is still bounded.
        let sweep = broker.take_stale_workers(Duration::from_secs(45), Duration::from_secs(30));
        assert!(sweep.lagging.is_empty());
        let [(session_id, _, detail)] = <[_; 1]>::try_from(sweep.isolated).ok().unwrap();
        assert_eq!(session_id, "sess-lagging");
        assert!(detail.contains("bytes of worker output queued"));
        assert!(broker.workers.lock().is_empty());
    }

    fn reconnecting_worker_fixture() -> (Broker, StartSession, mpsc::UnboundedReceiver<Frame>) {
        let broker = Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: String::new(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        });
        let (tx, rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: "sess-1".to_owned(),
                epoch: "epoch-1".to_owned(),
                generation: "gen-1".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: None,
                connection_id: 1,
                tx,
            })
            .expect("register worker");
        let launch = StartSession {
            session_id: "sess-1".to_owned(),
            provider: "codex".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_auth_generation: None,
            provider_behavior: None,
            cwd: "/work".to_owned(),
            agent_session_id: Some("agent-1".to_owned()),
            system: false,
            context_window: None,
            auto_compact_token_limit: None,
            cache_protection: None,
            generation: "gen-1".to_owned(),
            fallback_for: None,
            adopt_only: false,
            execution_binding: None,
        };
        broker.update_snapshot(
            WorkerSnapshot {
                session_id: "sess-1".to_owned(),
                worker_epoch: "epoch-1".to_owned(),
                generation: "gen-1".to_owned(),
                executable: Some("/bin/false".to_owned()),
                launch: Some(launch.clone()),
                state: WorkerState::Busy,
                agent_session_id: Some("agent-1".to_owned()),
                native_thread_materialized: None,
                current_turn_id: Some("turn-1".to_owned()),
                last_runtime_seq: 10,
                pending_permissions: Vec::new(),
                config_options: None,
                context_used: None,
                context_size: None,
                pending_prompt_count: 0,
                drain_requested: false,
                exit_detail: None,
                background_tasks: None,
                incarnation: None,
            },
            1,
        );
        (broker, launch, rx)
    }

    #[tokio::test]
    async fn hibernation_refuses_a_busy_worker_and_releases_an_idle_one() {
        let (broker, launch, mut worker_rx) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        let (controller_tx, mut controller_rx) = mpsc::unbounded_channel();
        broker.install_controller(controller_tx);

        broker
            .hibernate_session("sess-1".to_owned(), "hibernate-1".to_owned())
            .await;
        let Some(Frame::CommandAck {
            accepted, reason, ..
        }) = controller_rx.recv().await
        else {
            panic!("busy hibernation acknowledgement");
        };
        assert!(!accepted);
        assert_eq!(reason.as_deref(), Some("session is busy"));
        assert!(broker.workers.lock().contains_key("sess-1"));
        assert_eq!(broker.sessions.lock().get("sess-1"), Some(&launch));

        let mut idle = broker.snapshots().into_iter().next().expect("worker");
        idle.state = WorkerState::Running;
        idle.current_turn_id = None;
        broker.update_snapshot(idle, 1);
        while controller_rx.try_recv().is_ok() {}
        while worker_rx.try_recv().is_ok() {}

        broker
            .hibernate_session("sess-1".to_owned(), "hibernate-2".to_owned())
            .await;
        let Some(Frame::Snapshot { worker }) = controller_rx.recv().await else {
            panic!("hibernated worker snapshot");
        };
        assert_eq!(worker.state, WorkerState::Exited);
        assert_eq!(worker.exit_detail.as_deref(), Some("hibernated"));
        // The resumable native id survives for the next ensure.
        assert_eq!(worker.agent_session_id.as_deref(), Some("agent-1"));
        let Some(Frame::CommandAck { accepted, .. }) = controller_rx.recv().await else {
            panic!("hibernation acknowledgement");
        };
        assert!(accepted);
        assert!(matches!(
            worker_rx.try_recv(),
            Ok(Frame::WorkerCommand {
                command: WorkerCommand::Stop { .. },
                ..
            })
        ));
        // Nothing relaunches it: the worker and its declaration are released.
        assert!(!broker.workers.lock().contains_key("sess-1"));
        assert!(!broker.sessions.lock().contains_key("sess-1"));
        assert!(!broker.cancelled_sessions.lock().contains("sess-1"));
    }

    #[tokio::test]
    async fn hibernation_revokes_gateway_cache_protection_only_for_gateway_providers() {
        for gateway in [true, false] {
            let (broker, launch, _worker_rx) = reconnecting_worker_fixture();
            let broker = Arc::new(broker);
            let (controller_tx, _controller_rx) = mpsc::unbounded_channel();
            broker.install_controller(controller_tx);
            let mut idle = broker.snapshots().into_iter().next().expect("worker");
            idle.state = WorkerState::Running;
            idle.current_turn_id = None;
            broker.update_snapshot(idle, 1);
            let mut session = launch.clone();
            if gateway {
                session.provider = "claude-deepseek".to_owned();
                let behavior = crate::provider_behavior::legacy_behavior("claude-deepseek");
                assert!(
                    crate::deepseek_cache::supported_behavior(&behavior.configuration),
                    "fixture must use a cache-protected gateway behavior"
                );
                session.provider_behavior = Some(behavior);
            }
            broker.sessions.lock().insert("sess-1".to_owned(), session);
            broker
                .hibernate_session("sess-1".to_owned(), "hibernate-cache".to_owned())
                .await;
            assert!(!broker.workers.lock().contains_key("sess-1"));
            let revoked = broker.revoked_cache_protection.lock().clone();
            let expected: Vec<(String, &'static str)> = if gateway {
                vec![("sess-1".to_owned(), "session_hibernated")]
            } else {
                Vec::new()
            };
            assert_eq!(revoked, expected);
        }
    }

    fn attach_incarnations(broker: &Broker, root: &Path, writer: bool) {
        *broker.incarnations.lock() = Some(incarnation_process::open(root, writer));
    }

    fn lineage(broker: &Broker) -> Option<(String, u64, incarnations::Origin)> {
        broker
            .incarnations
            .lock()
            .as_ref()
            .and_then(|store| store.get("sess-1"))
            .map(|entry| (entry.incarnation.clone(), entry.epoch, entry.origin))
    }

    #[tokio::test]
    async fn only_an_admitted_writer_stamps_its_lineage_on_every_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let (broker, _launch, _worker_rx) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        let (controller_tx, mut controller_rx) = mpsc::unbounded_channel();
        broker.install_controller(controller_tx);
        // No store: nothing is reported.
        assert!(
            broker
                .snapshots()
                .iter()
                .all(|worker| worker.incarnation.is_none())
        );

        let namespace = root.path().join("incarnations");
        attach_incarnations(&broker, &namespace, true);
        let value = broker
            .incarnations
            .lock()
            .as_mut()
            .unwrap()
            .mint("sess-1", incarnations::Origin::Adopted)
            .unwrap();
        // Bulk snapshots (Welcome, resync) and individually sent ones agree.
        assert_eq!(
            broker.snapshots()[0].incarnation.as_deref(),
            Some(value.as_str())
        );
        broker.publish_session_state("sess-1", WorkerState::Running);
        let Some(Frame::Snapshot { worker }) = controller_rx.recv().await else {
            panic!("published snapshot");
        };
        assert_eq!(worker.incarnation.as_deref(), Some(value.as_str()));
        // A worker-supplied value is never trusted: the Machine overwrites it.
        let mut forged = broker.snapshots().into_iter().next().unwrap();
        forged.incarnation = Some("f".repeat(32));
        broker.send_controller(Frame::Snapshot {
            worker: Box::new(forged),
        });
        let Some(Frame::Snapshot { worker }) = controller_rx.recv().await else {
            panic!("forged snapshot");
        };
        assert_eq!(worker.incarnation.as_deref(), Some(value.as_str()));
        assert!(
            broker
                .incarnations
                .lock()
                .as_mut()
                .unwrap()
                .end("sess-1")
                .is_ok()
        );
        assert!(broker.snapshots()[0].incarnation.is_none());

        // A reader-only build holds the same record but reports nothing, since
        // it could miss a rotation made by a writer elsewhere.
        broker
            .incarnations
            .lock()
            .as_mut()
            .unwrap()
            .mint("sess-1", incarnations::Origin::Adopted)
            .unwrap();
        *broker.incarnations.lock() = None;
        attach_incarnations(&broker, &namespace, false);
        assert!(lineage(&broker).is_some());
        assert!(
            broker
                .snapshots()
                .iter()
                .all(|worker| worker.incarnation.is_none())
        );
        broker.publish_session_state("sess-1", WorkerState::Running);
        let Some(Frame::Snapshot { worker }) = controller_rx.recv().await else {
            panic!("reader-only snapshot");
        };
        assert!(worker.incarnation.is_none());
    }

    #[tokio::test]
    async fn a_declaration_mints_one_lineage_and_replays_keep_it() {
        let root = tempfile::tempdir().unwrap();
        let (broker, launch, _worker_rx) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        let (controller_tx, mut controller_rx) = mpsc::unbounded_channel();
        broker.install_controller(controller_tx);
        attach_incarnations(&broker, &root.path().join("incarnations"), true);
        let mut adopt = launch.clone();
        adopt.adopt_only = true;
        broker.ensure_session(adopt.clone()).await;
        // A worker already runs, so the first record is an adoption.
        let (first, epoch, origin) = lineage(&broker).expect("lineage minted before adoption");
        assert_eq!((epoch, origin), (1, incarnations::Origin::Adopted));
        broker.ensure_session(adopt).await;
        assert_eq!(lineage(&broker).unwrap().0, first);
        while let Ok(frame) = controller_rx.try_recv() {
            assert!(!matches!(
                frame,
                Frame::CommandAck {
                    accepted: false,
                    ..
                }
            ));
        }

        // A durably deleted ID is refused before any lineage can be created.
        broker
            .cancelled_sessions
            .lock()
            .insert("sess-gone".to_owned());
        let mut gone = launch;
        gone.session_id = "sess-gone".to_owned();
        broker.ensure_session(gone).await;
        assert!(
            broker
                .incarnations
                .lock()
                .as_ref()
                .unwrap()
                .get("sess-gone")
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_reader_only_build_neither_writes_nor_refuses_launches() {
        let root = tempfile::tempdir().unwrap();
        let (broker, launch, _worker_rx) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        let (controller_tx, mut controller_rx) = mpsc::unbounded_channel();
        broker.install_controller(controller_tx);
        attach_incarnations(&broker, &root.path().join("incarnations"), false);
        let mut adopt = launch;
        adopt.adopt_only = true;
        broker.ensure_session(adopt).await;
        assert!(lineage(&broker).is_none());
        assert!(!root.path().join("incarnations/incarnations.json").exists());
        while let Ok(frame) = controller_rx.try_recv() {
            assert!(!matches!(
                frame,
                Frame::CommandAck {
                    accepted: false,
                    ..
                }
            ));
        }
        assert!(broker.workers.lock().contains_key("sess-1"));
    }

    #[tokio::test]
    async fn an_unconfirmed_lineage_refuses_the_launch_without_declaring_it() {
        let root = tempfile::tempdir().unwrap();
        let (broker, launch, _worker_rx) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        let (controller_tx, mut controller_rx) = mpsc::unbounded_channel();
        broker.install_controller(controller_tx);
        let namespace = root.path().join("incarnations");
        attach_incarnations(&broker, &namespace, true);
        std::fs::create_dir(namespace.join("incarnations.json")).unwrap();
        broker.sessions.lock().remove("sess-1");
        let mut adopt = launch;
        adopt.adopt_only = true;
        broker.ensure_session(adopt).await;
        let Some(Frame::CommandAck {
            accepted, reason, ..
        }) = controller_rx.recv().await
        else {
            panic!("launch refusal acknowledgement");
        };
        assert!(!accepted);
        assert!(
            reason
                .unwrap()
                .contains("durable Session incarnation was not confirmed")
        );
        assert!(!broker.sessions.lock().contains_key("sess-1"));
    }

    #[tokio::test]
    async fn reset_rotates_before_any_effect_and_refuses_when_unconfirmed() {
        let root = tempfile::tempdir().unwrap();
        let (broker, launch, _worker_rx) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        let (controller_tx, mut controller_rx) = mpsc::unbounded_channel();
        broker.install_controller(controller_tx);
        let namespace = root.path().join("incarnations");
        attach_incarnations(&broker, &namespace, true);
        let before = broker
            .incarnations
            .lock()
            .as_mut()
            .unwrap()
            .mint("sess-1", incarnations::Origin::Adopted)
            .unwrap();

        // Unconfirmed rotation: old lineage current, nothing stopped or fenced.
        std::fs::remove_file(namespace.join("incarnations.json")).unwrap();
        std::fs::create_dir(namespace.join("incarnations.json")).unwrap();
        handle_core_command(
            &broker,
            CoreCommand::StopSession {
                session_id: "sess-1".into(),
                command_id: "reset-refused".into(),
            },
        )
        .await;
        let Some(Frame::CommandAck {
            accepted, reason, ..
        }) = controller_rx.recv().await
        else {
            panic!("reset refusal acknowledgement");
        };
        assert!(!accepted);
        assert!(
            reason
                .unwrap()
                .contains("durable Session incarnation was not confirmed")
        );
        assert_eq!(broker.sessions.lock().get("sess-1"), Some(&launch));
        assert!(broker.workers.lock().contains_key("sess-1"));
        assert!(broker.resetting_sessions.lock().is_empty());
        assert!(!broker.cancelled_sessions.lock().contains("sess-1"));
        assert_eq!(lineage(&broker).unwrap().0, before);
    }

    #[tokio::test]
    async fn a_confirmed_rotation_starts_a_new_lineage_and_hibernation_keeps_it() {
        let root = tempfile::tempdir().unwrap();
        let (broker, _launch, _worker_rx) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        let (controller_tx, _controller_rx) = mpsc::unbounded_channel();
        broker.install_controller(controller_tx);
        attach_incarnations(&broker, &root.path().join("incarnations"), true);
        let before = broker
            .incarnations
            .lock()
            .as_mut()
            .unwrap()
            .mint("sess-1", incarnations::Origin::Adopted)
            .unwrap();
        broker.rotate_incarnation("sess-1").await.unwrap();
        let (rotated, epoch, origin) = lineage(&broker).unwrap();
        assert_ne!(rotated, before);
        assert_eq!((epoch, origin), (2, incarnations::Origin::Reset));

        let mut idle = broker.snapshots().into_iter().next().expect("worker");
        idle.state = WorkerState::Running;
        idle.current_turn_id = None;
        broker.update_snapshot(idle, 1);
        broker
            .hibernate_session("sess-1".to_owned(), "hibernate-lineage".to_owned())
            .await;
        assert!(!broker.workers.lock().contains_key("sess-1"));
        assert_eq!(lineage(&broker).unwrap().0, rotated);
    }

    #[tokio::test]
    async fn deletion_ends_the_lineage_only_after_the_journal_commit() {
        let root = tempfile::tempdir().unwrap();
        let (broker, _launch, _worker_rx) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        let (controller_tx, mut controller_rx) = mpsc::unbounded_channel();
        broker.install_controller(controller_tx);
        broker.attach_deletion_journal(
            deletions::Journal::open(
                &root.path().join("deletions"),
                deletion_fixture_owner(),
                true,
            )
            .unwrap(),
        );
        attach_incarnations(&broker, &root.path().join("incarnations"), true);
        broker
            .incarnations
            .lock()
            .as_mut()
            .unwrap()
            .mint("sess-1", incarnations::Origin::Adopted)
            .unwrap();
        handle_core_command(
            &broker,
            CoreCommand::StopSession {
                session_id: "sess-1".into(),
                command_id: "delete-lineage".into(),
            },
        )
        .await;
        assert!(broker.cancelled_sessions.lock().contains("sess-1"));
        assert!(lineage(&broker).is_none());
        while let Ok(frame) = controller_rx.try_recv() {
            assert!(!matches!(
                frame,
                Frame::CommandAck {
                    accepted: false,
                    ..
                }
            ));
        }

        // A rejected journal write leaves the lineage in place.
        let (broker, _launch, _worker_rx) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        let (controller_tx, _controller_rx) = mpsc::unbounded_channel();
        broker.install_controller(controller_tx);
        let journal_root = root.path().join("deletions-failing");
        broker.attach_deletion_journal(
            deletions::Journal::open(&journal_root, deletion_fixture_owner(), true).unwrap(),
        );
        std::fs::create_dir(journal_root.join("deletions.json")).unwrap();
        attach_incarnations(&broker, &root.path().join("incarnations-kept"), true);
        broker
            .incarnations
            .lock()
            .as_mut()
            .unwrap()
            .mint("sess-1", incarnations::Origin::Adopted)
            .unwrap();
        handle_core_command(
            &broker,
            CoreCommand::StopSession {
                session_id: "sess-1".into(),
                command_id: "delete-refused".into(),
            },
        )
        .await;
        assert!(!broker.cancelled_sessions.lock().contains("sess-1"));
        assert!(lineage(&broker).is_some());
    }

    #[test]
    fn hibernation_keeps_every_kind_of_unfinished_work() {
        let (broker, _, _) = reconnecting_worker_fixture();
        let mut worker = broker.snapshots().into_iter().next().expect("worker");
        worker.state = WorkerState::Running;
        worker.current_turn_id = None;
        assert_eq!(hibernation_refusal(&worker), None);
        worker.pending_permissions = vec!["permission-1".to_owned()];
        assert!(hibernation_refusal(&worker).is_some());
        worker.pending_permissions.clear();
        worker.background_tasks = Some(1);
        assert!(hibernation_refusal(&worker).is_some());
        worker.background_tasks = Some(0);
        worker.pending_prompt_count = 1;
        assert!(hibernation_refusal(&worker).is_some());
    }

    #[test]
    fn reconnecting_worker_rebuilds_broker_launch_state() {
        let (broker, launch, _) = reconnecting_worker_fixture();
        assert_eq!(broker.sessions.lock().get("sess-1"), Some(&launch));
    }

    #[test]
    fn worker_snapshot_cannot_rewrite_declared_session_placement() {
        for field in ["session_id", "provider", "cwd", "system"] {
            let (broker, launch, mut rx) = reconnecting_worker_fixture();
            let original = broker.workers.lock()["sess-1"].snapshot.clone();
            let mut changed = original.clone();
            changed.last_runtime_seq += 1;
            let metadata = changed.launch.as_mut().expect("launch");
            match field {
                "session_id" => metadata.session_id = "other-session".into(),
                "provider" => metadata.provider = "claude-code".into(),
                "cwd" => metadata.cwd = "/other-worktree".into(),
                "system" => metadata.system = true,
                _ => unreachable!(),
            }
            broker.update_snapshot(changed, 1);
            assert_eq!(
                broker.sessions.lock().get("sess-1"),
                Some(&launch),
                "{field}"
            );
            assert_eq!(
                broker.workers.lock()["sess-1"].snapshot,
                original,
                "{field}"
            );
            assert!(matches!(rx.try_recv(), Ok(Frame::Reject { .. })), "{field}");
        }
    }

    #[test]
    fn stale_snapshot_cannot_reject_the_current_worker() {
        for stale_epoch in [false, true] {
            let (broker, launch, mut rx) = reconnecting_worker_fixture();
            let original = broker.workers.lock()["sess-1"].snapshot.clone();
            let mut stale = original.clone();
            stale.launch.as_mut().expect("launch").cwd = "/stale".into();
            if stale_epoch {
                stale.worker_epoch = "retired-epoch".into();
            }
            broker.update_snapshot(stale, if stale_epoch { 1 } else { 2 });
            assert!(rx.try_recv().is_err());
            assert_eq!(broker.sessions.lock().get("sess-1"), Some(&launch));
            assert_eq!(broker.workers.lock()["sess-1"].snapshot, original);
        }
    }

    #[test]
    fn first_worker_snapshot_cannot_seed_another_session_id() {
        let (broker, _, mut rx) = reconnecting_worker_fixture();
        broker.sessions.lock().clear();
        let mut snapshot = broker.workers.lock()["sess-1"].snapshot.clone();
        snapshot.launch.as_mut().expect("launch").session_id = "other-session".into();
        broker.update_snapshot(snapshot, 1);
        assert!(broker.sessions.lock().is_empty());
        assert!(matches!(rx.try_recv(), Ok(Frame::Reject { .. })));
    }

    #[tokio::test]
    async fn old_worker_snapshot_cannot_undo_staged_workspace_reset() {
        let (broker, mut replacement, mut rx) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        let old = broker.workers.lock()["sess-1"].snapshot.clone();
        replacement.cwd = "/replacement-worktree".into();
        replacement.adopt_only = true;
        broker.ensure_session(replacement.clone()).await;
        replacement.adopt_only = false;
        broker.update_snapshot(old, 1);
        assert_eq!(broker.sessions.lock().get("sess-1"), Some(&replacement));
        assert!(matches!(rx.try_recv(), Ok(Frame::Reject { .. })));
        assert!(broker.launching.lock().is_empty());
    }

    #[tokio::test]
    async fn deleted_session_refuses_late_adoption_and_launch() {
        let (broker, mut launch, _) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        let (tx, mut rx) = mpsc::unbounded_channel();
        broker.install_controller(tx);
        handle_core_command(
            &broker,
            CoreCommand::StopSession {
                session_id: "sess-1".into(),
                command_id: "delete-1".into(),
            },
        )
        .await;
        for adopt_only in [true, false] {
            launch.adopt_only = adopt_only;
            broker.ensure_session(launch.clone()).await;
            assert!(broker.cancelled_sessions.lock().contains("sess-1"));
            assert!(!broker.sessions.lock().contains_key("sess-1"));
            assert!(!broker.session_states.lock().contains_key("sess-1"));
            assert!(!broker.launching.lock().contains("sess-1"));
            assert!(!broker.awaiting_reconnect.lock().contains("sess-1"));
        }
        let mut refused = 0;
        while let Ok(frame) = rx.try_recv() {
            if let Frame::CommandAck {
                command_id,
                accepted,
                reason,
                ..
            } = frame
                && command_id == "ensure:sess-1"
            {
                assert!(!accepted);
                assert!(reason.expect("refusal reason").contains("deleted"));
                refused += 1;
            }
        }
        assert_eq!(refused, 2);
    }

    #[tokio::test]
    async fn lifecycle_gate_blocks_same_session_adoption_but_not_other_sessions() {
        let (broker, mut launch, _) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        broker.sessions.lock().clear();
        launch.adopt_only = true;
        let gate = broker.session_lifecycle_gate("sess-1");
        let guard = gate.lock().await;
        let declaration = broker.ensure_session(launch.clone());
        tokio::pin!(declaration);
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(declaration.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        assert!(!broker.sessions.lock().contains_key("sess-1"));
        let mut other = launch.clone();
        other.session_id = "sess-2".into();
        tokio::time::timeout(Duration::from_secs(2), broker.ensure_session(other))
            .await
            .expect("unrelated session proceeds");
        assert!(broker.sessions.lock().contains_key("sess-2"));
        drop(guard);
        tokio::time::timeout(Duration::from_secs(2), declaration)
            .await
            .expect("original declaration proceeds after cleanup fence");
        launch.adopt_only = false;
        assert_eq!(broker.sessions.lock().get("sess-1"), Some(&launch));
    }

    #[tokio::test]
    async fn queued_delete_wins_over_a_later_launch_declaration() {
        let (broker, mut launch, _) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        launch.adopt_only = true;
        let gate = broker.session_lifecycle_gate("sess-1");
        let guard = gate.lock().await;
        let deletion = handle_core_command(
            &broker,
            CoreCommand::StopSession {
                session_id: "sess-1".into(),
                command_id: "delete-queued".into(),
            },
        );
        let declaration = broker.ensure_session(launch);
        tokio::pin!(deletion, declaration);
        // Poll in order to establish real FIFO lock admission, not a sleep.
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(deletion.as_mut(), cx).is_pending());
            assert!(std::future::Future::poll(declaration.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(guard);
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(deletion, declaration);
        })
        .await
        .expect("queued lifecycle operations finish");
        assert!(broker.cancelled_sessions.lock().contains("sess-1"));
        assert!(!broker.sessions.lock().contains_key("sess-1"));
        assert!(broker.launching.lock().is_empty());
    }

    #[tokio::test]
    async fn core_ipc_refuses_late_declaration_after_acknowledged_delete() {
        let (broker, mut launch, _) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        broker.workers.lock().clear();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let lease = broker.install_controller(tx);
        let (mut client, peer) = UnixStream::pair().expect("core IPC pair");
        let (mut reader, _writer) = peer.into_split();
        let handler_broker = Arc::clone(&broker);
        let handler =
            tokio::spawn(async move { handle_core(handler_broker, lease, &mut reader).await });
        write_frame(
            &mut client,
            &Frame::CoreCommand {
                command: CoreCommand::StopSession {
                    session_id: "sess-1".into(),
                    command_id: "delete-ipc".into(),
                },
            },
        )
        .await
        .expect("delete frame");
        assert!(
            matches!(tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await.expect("delete progresses"),
            Some(Frame::CommandAck { command_id, accepted: true, .. })
                if command_id == "delete-ipc")
        );
        launch.adopt_only = true;
        write_frame(
            &mut client,
            &Frame::CoreCommand {
                command: CoreCommand::EnsureSession { session: launch },
            },
        )
        .await
        .expect("late declaration frame");
        assert!(
            matches!(tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await.expect("late declaration answered"),
            Some(Frame::CommandAck { command_id, accepted: false, .. })
                if command_id == "ensure:sess-1")
        );
        assert!(broker.cancelled_sessions.lock().contains("sess-1"));
        assert!(!broker.sessions.lock().contains_key("sess-1"));
        assert!(broker.launching.lock().is_empty());
        drop(client);
        handler.await.expect("core task").expect("core EOF");
    }

    fn deletion_fixture_owner() -> deletions::Owner {
        deletions::Owner {
            machine_id: "fixture-machine".into(),
            service_id: Some("fixture-service".into()),
        }
    }

    #[tokio::test]
    async fn deletion_storage_failure_has_no_stop_or_registry_effect() {
        let root = tempfile::tempdir().expect("journal root");
        let (broker, launch, mut worker_rx) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        broker.attach_deletion_journal(
            deletions::Journal::open(root.path(), deletion_fixture_owner(), true).unwrap(),
        );
        std::fs::create_dir(root.path().join("deletions.json")).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        broker.install_controller(tx);
        handle_core_command(
            &broker,
            CoreCommand::StopSession {
                session_id: "sess-1".into(),
                command_id: "delete-storage-failed".into(),
            },
        )
        .await;
        assert!(matches!(rx.try_recv(), Ok(Frame::CommandAck {
            accepted: false, command_id, ..
        }) if command_id == "delete-storage-failed"));
        assert_eq!(broker.sessions.lock().get("sess-1"), Some(&launch));
        assert!(!broker.cancelled_sessions.lock().contains("sess-1"));
        assert!(broker.deleted_session_workspaces.lock().is_empty());
        assert!(worker_rx.try_recv().is_err());
        assert!(broker.check_deletion_journal().is_err());
    }

    #[tokio::test]
    async fn read_only_deletion_reader_preserves_existing_volatile_delete() {
        let root = tempfile::tempdir().expect("journal root");
        let (broker, _, _) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        broker.attach_deletion_journal(
            deletions::Journal::open(root.path(), deletion_fixture_owner(), false).unwrap(),
        );
        handle_core_command(
            &broker,
            CoreCommand::StopSession {
                session_id: "sess-1".into(),
                command_id: "delete-volatile".into(),
            },
        )
        .await;
        assert!(broker.cancelled_sessions.lock().contains("sess-1"));
        assert!(!broker.sessions.lock().contains_key("sess-1"));
        assert!(!root.path().join("deletions.json").exists());
    }

    #[tokio::test]
    async fn durable_delete_survives_broker_restart_and_fences_real_ipc() {
        let root = tempfile::tempdir().expect("restart fixture");
        let socket = root.path().join("runtime.sock");
        let journal_root = root.path().join("deletions");
        let (fixture, mut launch, _) = reconnecting_worker_fixture();
        let mut args = fixture.args.clone();
        args.socket = socket.clone();
        args.worktree_root = root.path().join("worktrees");
        let journal =
            deletions::Journal::open(&journal_root, deletion_fixture_owner(), true).unwrap();
        let server = tokio::spawn(run_broker(args.clone(), Some(journal), None, None));
        tokio::time::timeout(Duration::from_secs(2), async {
            while !socket.exists() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("writer socket ready");
        let (mut reader, mut writer, _) = connect_peer(&socket, PeerRole::Core, None, None).await;
        write_frame(
            &mut writer,
            &Frame::CoreCommand {
                command: CoreCommand::StopSession {
                    session_id: "sess-1".into(),
                    command_id: "durable-delete".into(),
                },
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), read_frame(&mut reader))
                .await
                .unwrap()
                .unwrap(),
            Some(Frame::CommandAck { accepted: true, .. })
        ));
        assert!(journal_root.join("deletions.json").is_file());
        drop(reader);
        drop(writer);
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        // Reopen succeeds only after the actual old owner has released its
        // lock. This also tests cancellation of the broker's monitor task.
        let journal = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match deletions::Journal::open(&journal_root, deletion_fixture_owner(), false) {
                    Ok(journal) => break journal,
                    Err(error) if error.to_string().contains("already owned") => {
                        tokio::task::yield_now().await
                    }
                    Err(error) => panic!("cold reader refused: {error}"),
                }
            }
        })
        .await
        .expect("old namespace owner exits");
        assert!(journal.deleted().contains("sess-1"));
        let server = tokio::spawn(run_broker(args, Some(journal), None, None));
        let (deleted_reader, deleted_writer, reply) =
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    match UnixStream::connect(&socket).await {
                        Ok(stream) => {
                            drop(stream);
                            break;
                        }
                        Err(_) => tokio::task::yield_now().await,
                    }
                }
                connect_peer(&socket, PeerRole::Worker, Some("sess-1"), Some("old-epoch")).await
            })
            .await
            .expect("cold worker handshake");
        assert!(matches!(reply, Frame::Reject { reason } if reason.contains("deleted")));
        drop(deleted_reader);
        drop(deleted_writer);
        let (mut reader, mut writer, _) = connect_peer(&socket, PeerRole::Core, None, None).await;
        launch.adopt_only = true;
        write_frame(
            &mut writer,
            &Frame::CoreCommand {
                command: CoreCommand::EnsureSession { session: launch },
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), read_frame(&mut reader))
                .await
                .unwrap()
                .unwrap(),
            Some(Frame::CommandAck {
                accepted: false,
                ..
            })
        ));
        drop(reader);
        drop(writer);
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn durable_terminal_identity_cannot_be_cleared_by_reset() {
        let root = tempfile::tempdir().expect("journal root");
        let mut journal =
            deletions::Journal::open(root.path(), deletion_fixture_owner(), true).unwrap();
        journal.mark_deleted("sess-1").unwrap();
        let (broker, launch, mut worker_rx) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        broker.attach_deletion_journal(journal);
        let (tx, mut rx) = mpsc::unbounded_channel();
        broker.install_controller(tx);
        broker
            .reset_session(launch.clone(), "reset-deleted".into())
            .await;
        assert!(matches!(
            rx.try_recv(),
            Ok(Frame::CommandAck {
                accepted: false,
                ..
            })
        ));
        assert!(broker.cancelled_sessions.lock().contains("sess-1"));
        assert_eq!(broker.sessions.lock().get("sess-1"), Some(&launch));
        assert!(worker_rx.try_recv().is_err());
    }

    #[test]
    fn worker_snapshot_preserves_release_and_native_thread_updates() {
        let (broker, _, mut rx) = reconnecting_worker_fixture();
        let mut updated = broker.workers.lock()["sess-1"].snapshot.clone();
        let launch = updated.launch.as_mut().expect("launch");
        launch.provider_version = "next".into();
        launch.provider_generation_digest = "next-digest".into();
        launch.agent_session_id = Some("materialized-thread".into());
        launch.context_window = Some(128_000);
        let expected = launch.clone();
        updated.last_runtime_seq += 1;
        broker.update_snapshot(updated.clone(), 1);
        assert_eq!(broker.sessions.lock().get("sess-1"), Some(&expected));
        assert_eq!(broker.workers.lock()["sess-1"].snapshot, updated);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn rejected_ipc_snapshot_has_no_controller_or_rollout_effect() {
        let (broker, launch, mut worker_rx) = reconnecting_worker_fixture();
        let broker = Arc::new(broker);
        *broker.desired_generation.lock() = "gen-1".into();
        let (controller_tx, mut controller_rx) = mpsc::unbounded_channel();
        broker.install_controller(controller_tx);
        let original = broker.workers.lock()["sess-1"].snapshot.clone();
        let mut invalid = original.clone();
        invalid.launch.as_mut().expect("launch").cwd = "/other-worktree".into();
        let (peer, mut client) = UnixStream::pair().expect("IPC pair");
        let (mut reader, _writer) = peer.into_split();
        let handler_broker = Arc::clone(&broker);
        let handler = tokio::spawn(async move {
            handle_worker(handler_broker, 1, "sess-1", "epoch-1", &mut reader).await
        });
        write_frame(
            &mut client,
            &Frame::Snapshot {
                worker: Box::new(invalid),
            },
        )
        .await
        .expect("send invalid snapshot");
        // A subsequent frame proves the reader consumed the invalid snapshot;
        // no elapsed-time assumption is needed to assert absence of projection.
        let marker = Frame::CommandAck {
            session_id: "sess-1".into(),
            command_id: "marker".into(),
            accepted: true,
            reason: None,
        };
        write_frame(&mut client, &marker)
            .await
            .expect("send marker");
        let projected = tokio::time::timeout(Duration::from_secs(2), controller_rx.recv())
            .await
            .expect("IPC progress")
            .expect("controller frame");
        assert_eq!(projected, marker);
        assert!(controller_rx.try_recv().is_err());
        assert!(matches!(worker_rx.try_recv(), Ok(Frame::Reject { .. })));
        assert!(broker.healthy_generations.lock().is_empty());
        assert_eq!(broker.sessions.lock().get("sess-1"), Some(&launch));
        assert_eq!(broker.workers.lock()["sess-1"].snapshot, original);
        drop(client);
        handler.await.expect("handler task").expect("handler EOF");
    }

    #[test]
    fn healthy_fallback_is_not_redrained_until_next_rollout() {
        let broker = Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: String::new(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        });
        let (tx, mut rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: "sess-1".to_owned(),
                epoch: "epoch-1".to_owned(),
                generation: "gen-1".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: Some("gen-2".to_owned()),
                connection_id: 1,
                tx,
            })
            .expect("register fallback");
        broker.set_desired_generation("gen-2".to_owned(), Some("/bin/false".to_owned()));
        assert!(
            rx.try_recv().is_err(),
            "same rollout must retain its fallback"
        );

        broker.set_desired_generation("gen-3".to_owned(), Some("/bin/false".to_owned()));
        assert!(matches!(
            rx.try_recv(),
            Ok(Frame::WorkerCommand {
                command: WorkerCommand::Drain,
                ..
            })
        ));
    }

    #[test]
    fn healthy_generation_releases_matching_provider_fallbacks() {
        let broker = Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-2".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        });
        for (session_id, provider) in [("sess-codex", "codex"), ("sess-claude", "claude-code")] {
            broker.sessions.lock().insert(
                session_id.to_owned(),
                StartSession {
                    session_id: session_id.to_owned(),
                    provider: provider.to_owned(),
                    provider_version: String::new(),
                    provider_generation_digest: String::new(),
                    provider_auth_generation: None,
                    provider_behavior: None,
                    cwd: "/work".to_owned(),
                    agent_session_id: None,
                    system: false,
                    context_window: None,
                    auto_compact_token_limit: None,
                    cache_protection: None,
                    generation: "gen-1".to_owned(),
                    fallback_for: Some("gen-2".to_owned()),
                    adopt_only: false,
                    execution_binding: None,
                },
            );
            broker.pin_fallback(session_id, "gen-1", "gen-2");
        }
        broker.unhealthy_generations.lock().insert(
            ("gen-2".to_owned(), "codex".to_owned()),
            HashSet::from(["sess-failed".to_owned()]),
        );
        let (codex_tx, mut codex_rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: "sess-codex".to_owned(),
                epoch: "epoch-codex".to_owned(),
                generation: "gen-1".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: Some("gen-2".to_owned()),
                connection_id: 1,
                tx: codex_tx,
            })
            .expect("register codex fallback");

        assert_eq!(
            broker.rehabilitate_generation("gen-2", "codex"),
            vec!["sess-codex".to_owned()]
        );
        assert!(matches!(
            codex_rx.try_recv(),
            Ok(Frame::WorkerCommand {
                command: WorkerCommand::Drain,
                ..
            })
        ));
        assert!(
            broker
                .workers
                .lock()
                .get("sess-codex")
                .is_some_and(|worker| worker.snapshot.drain_requested)
        );
        assert!(!broker.fallback_pins.lock().contains_key("sess-codex"));
        assert!(!broker.fallback_targets.lock().contains_key("sess-codex"));
        assert!(broker.fallback_pins.lock().contains_key("sess-claude"));
        assert!(broker.fallback_targets.lock().contains_key("sess-claude"));
        assert!(broker.unhealthy_generations.lock().is_empty());

        broker.sessions.lock().insert(
            "sess-late".to_owned(),
            StartSession {
                session_id: "sess-late".to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: "/work".to_owned(),
                agent_session_id: None,
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-1".to_owned(),
                fallback_for: Some("gen-2".to_owned()),
                adopt_only: false,
                execution_binding: None,
            },
        );
        let (late_tx, _late_rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: "sess-late".to_owned(),
                epoch: "epoch-late".to_owned(),
                generation: "gen-1".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: Some("gen-2".to_owned()),
                connection_id: 2,
                tx: late_tx,
            })
            .expect("register late fallback");
        assert!(!broker.fallback_pins.lock().contains_key("sess-late"));
        let mut late_snapshot = broker
            .workers
            .lock()
            .get("sess-late")
            .expect("late worker")
            .snapshot
            .clone();
        late_snapshot.drain_requested = false;
        late_snapshot.launch = broker.sessions.lock().get("sess-late").cloned();
        broker.update_snapshot(late_snapshot, 2);
        assert!(!broker.fallback_pins.lock().contains_key("sess-late"));
        assert!(broker.unhealthy_generations.lock().is_empty());
        assert!(
            broker
                .workers
                .lock()
                .get("sess-late")
                .is_some_and(|worker| worker.snapshot.drain_requested)
        );
    }

    #[test]
    fn ready_event_rehabilitates_generation_fallbacks() {
        assert_readiness_rehabilitates_generation_fallbacks(RuntimeEvent::Ready {
            agent_session_id: Some("agent-ready".to_owned()),
        });
    }

    #[test]
    fn running_status_rehabilitates_generation_fallbacks() {
        assert_readiness_rehabilitates_generation_fallbacks(RuntimeEvent::Status {
            state: WorkerState::Running,
            detail: None,
        });
    }

    fn assert_readiness_rehabilitates_generation_fallbacks(ready: RuntimeEvent) {
        let broker = Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-2".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        });
        broker.sessions.lock().insert(
            "sess-ready".to_owned(),
            StartSession {
                session_id: "sess-ready".to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: "/work".to_owned(),
                agent_session_id: None,
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-2".to_owned(),
                fallback_for: None,
                adopt_only: false,
                execution_binding: None,
            },
        );
        broker.sessions.lock().insert(
            "sess-fallback".to_owned(),
            StartSession {
                session_id: "sess-fallback".to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: "/work".to_owned(),
                agent_session_id: None,
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-1".to_owned(),
                fallback_for: Some("gen-2".to_owned()),
                adopt_only: false,
                execution_binding: None,
            },
        );
        broker.pin_fallback("sess-fallback", "gen-1", "gen-2");
        broker.unhealthy_generations.lock().insert(
            ("gen-2".to_owned(), "codex".to_owned()),
            HashSet::from(["sess-failed".to_owned()]),
        );
        let (tx, _rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: "sess-ready".to_owned(),
                epoch: "epoch-ready".to_owned(),
                generation: "gen-2".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: None,
                connection_id: 1,
                tx,
            })
            .expect("register desired worker");

        assert!(
            broker
                .update_from_event("sess-ready", 99, 1, &ready)
                .is_empty()
        );
        assert!(!broker.unhealthy_generations.lock().is_empty());
        assert_eq!(
            broker.update_from_event("sess-ready", 1, 1, &ready),
            vec!["sess-fallback".to_owned()]
        );
        assert!(broker.unhealthy_generations.lock().is_empty());
        assert!(!broker.fallback_pins.lock().contains_key("sess-fallback"));
        assert!(
            broker
                .healthy_generations
                .lock()
                .contains(&("gen-2".to_owned(), "codex".to_owned()))
        );
        broker.quarantine_generation_if_unproven("gen-2", "codex", "missing-native-thread");
        assert!(broker.unhealthy_generations.lock().is_empty());
    }

    #[test]
    fn known_healthy_generation_is_not_requarantined() {
        let broker = Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-2".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        });
        broker
            .healthy_generations
            .lock()
            .insert(("gen-2".to_owned(), "codex".to_owned()));

        broker.quarantine_generation_if_unproven("gen-2", "codex", "sess-late-failure");
        assert!(broker.unhealthy_generations.lock().is_empty());

        broker.quarantine_generation_if_unproven("gen-3", "codex", "sess-canary-failure");
        assert_eq!(
            broker
                .unhealthy_generations
                .lock()
                .get(&("gen-3".to_owned(), "codex".to_owned()))
                .cloned(),
            Some(HashSet::from(["sess-canary-failure".to_owned()]))
        );
    }

    #[test]
    fn broker_snapshot_preserves_launch_and_held_prompt_without_worker() {
        let broker = Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        });
        let launch = StartSession {
            session_id: "sess-1".to_owned(),
            provider: "codex".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_auth_generation: None,
            provider_behavior: None,
            cwd: "/work".to_owned(),
            agent_session_id: None,
            system: false,
            context_window: None,
            auto_compact_token_limit: None,
            cache_protection: None,
            generation: "gen-1".to_owned(),
            fallback_for: None,
            adopt_only: false,
            execution_binding: None,
        };
        broker
            .sessions
            .lock()
            .insert("sess-1".to_owned(), launch.clone());
        broker
            .session_states
            .lock()
            .insert("sess-1".to_owned(), WorkerState::Starting);
        broker.queue_pending(
            "sess-1",
            WorkerCommand::Prompt {
                command_id: "cmd-1".to_owned(),
                trace: None,
                turn_id: "turn-1".to_owned(),
                content: vec![serde_json::json!({"type": "text", "text": "next"})],
                cmid: None,
                echo_artifacts: false,
            },
        );
        let snapshots = broker.snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].launch.as_ref(), Some(&launch));
        assert_eq!(snapshots[0].state, WorkerState::Starting);
        assert_eq!(snapshots[0].pending_prompt_count, 1);
    }

    #[tokio::test]
    async fn adopt_only_rebuilds_registry_without_spawning_an_owner() {
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        }));
        broker
            .ensure_session(StartSession {
                session_id: "sess-adopt".to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: "/work".to_owned(),
                agent_session_id: Some("agent-1".to_owned()),
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-1".to_owned(),
                fallback_for: None,
                adopt_only: true,
                execution_binding: None,
            })
            .await;

        let sessions = broker.sessions.lock();
        let adopted = sessions.get("sess-adopt").expect("registry entry");
        assert!(!adopted.adopt_only, "launch specs must be normalized");
        assert!(broker.workers.lock().is_empty());
        assert!(broker.launching.lock().is_empty());
    }

    #[tokio::test]
    async fn explicit_revive_recycles_an_attached_terminal_worker() {
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-current".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        }));
        let (worker_tx, mut worker_rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: "sess-terminal".to_owned(),
                epoch: "epoch-terminal".to_owned(),
                generation: "gen-retired".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: None,
                connection_id: 41,
                tx: worker_tx,
            })
            .expect("register terminal worker");
        broker
            .workers
            .lock()
            .get_mut("sess-terminal")
            .expect("terminal worker")
            .snapshot
            .state = WorkerState::Exited;
        broker.startup_failures.lock().insert(
            "sess-terminal".to_owned(),
            "stale terminal failure".to_owned(),
        );
        broker
            .replacing
            .lock()
            .insert("sess-terminal".to_owned(), "gen-retired".to_owned());

        broker
            .ensure_session(StartSession {
                session_id: "sess-terminal".to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: "/work".to_owned(),
                agent_session_id: Some("agent-terminal".to_owned()),
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-current".to_owned(),
                fallback_for: None,
                adopt_only: false,
                execution_binding: None,
            })
            .await;

        assert!(matches!(
            worker_rx.try_recv(),
            Ok(Frame::WorkerCommand {
                session_id,
                command: WorkerCommand::Stop { command_id },
            }) if session_id == "sess-terminal"
                && command_id == "revive-stop-sess-terminal"
        ));
        assert!(
            !broker.workers.lock().contains_key("sess-terminal"),
            "the terminal peer must be fenced before replacement startup"
        );
        assert!(
            broker
                .pending_commands
                .lock()
                .get("sess-terminal")
                .is_none(),
            "the old Stop command must not leak to the replacement worker"
        );
        assert!(
            !broker.startup_failures.lock().contains_key("sess-terminal"),
            "the replacement must not inherit stale startup failure detail"
        );
        assert!(
            !broker.replacing.lock().contains_key("sess-terminal"),
            "explicit revive must take ownership from the stuck rollout"
        );
    }

    #[tokio::test]
    async fn explicit_revive_recycles_a_drain_ready_retired_worker() {
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-current".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        }));
        let (worker_tx, mut worker_rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: "sess-retired".to_owned(),
                epoch: "epoch-retired".to_owned(),
                generation: "gen-retired".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: None,
                connection_id: 43,
                tx: worker_tx,
            })
            .expect("register retired worker");
        {
            let mut workers = broker.workers.lock();
            let snapshot = &mut workers
                .get_mut("sess-retired")
                .expect("retired worker")
                .snapshot;
            snapshot.state = WorkerState::Draining;
            snapshot.drain_requested = true;
        }

        broker
            .ensure_session(StartSession {
                session_id: "sess-retired".to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: "/work".to_owned(),
                agent_session_id: Some("agent-retired".to_owned()),
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-current".to_owned(),
                fallback_for: None,
                adopt_only: false,
                execution_binding: None,
            })
            .await;

        assert!(matches!(
            worker_rx.try_recv(),
            Ok(Frame::WorkerCommand {
                session_id,
                command: WorkerCommand::Stop { command_id },
            }) if session_id == "sess-retired"
                && command_id == "revive-stop-sess-retired"
        ));
        assert!(
            !broker.workers.lock().contains_key("sess-retired"),
            "the drain-ready retired peer must be fenced before replacement startup"
        );
    }

    #[tokio::test]
    async fn adopt_only_keeps_an_attached_terminal_worker_dormant() {
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-current".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        }));
        let (worker_tx, mut worker_rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: "sess-dormant".to_owned(),
                epoch: "epoch-dormant".to_owned(),
                generation: "gen-retired".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: None,
                connection_id: 42,
                tx: worker_tx,
            })
            .expect("register dormant worker");
        broker
            .workers
            .lock()
            .get_mut("sess-dormant")
            .expect("dormant worker")
            .snapshot
            .state = WorkerState::Exited;

        broker
            .ensure_session(StartSession {
                session_id: "sess-dormant".to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: "/work".to_owned(),
                agent_session_id: Some("agent-dormant".to_owned()),
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-current".to_owned(),
                fallback_for: None,
                adopt_only: true,
                execution_binding: None,
            })
            .await;

        assert!(worker_rx.try_recv().is_err());
        assert!(broker.workers.lock().contains_key("sess-dormant"));
        assert!(broker.launching.lock().is_empty());
    }

    #[tokio::test]
    async fn explicit_reset_revokes_delete_tombstone_before_relaunch() {
        let temp = tempfile::tempdir().expect("cleanup root");
        let workspace = temp.path().join("sess-reset");
        std::fs::create_dir(&workspace).expect("cleanup workspace");
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(100),
        }));
        broker
            .cancelled_sessions
            .lock()
            .insert("sess-reset".to_owned());
        broker.deleted_session_workspaces.lock().insert(
            "sess-reset".to_owned(),
            DeletedWorkspace {
                workspace: crate::session_workspace::capture_cleanup_workspace(
                    temp.path(),
                    "sess-reset",
                    &workspace,
                )
                .expect("original cleanup workspace"),
                command_id: "delete-before-reset".to_owned(),
            },
        );
        broker.sessions.lock().insert(
            "sess-reset".to_owned(),
            StartSession {
                session_id: "sess-reset".to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: "/work".to_owned(),
                agent_session_id: None,
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-1".to_owned(),
                fallback_for: None,
                adopt_only: false,
                execution_binding: None,
            },
        );

        handle_core_command(
            &broker,
            CoreCommand::StopSession {
                session_id: "sess-reset".to_owned(),
                command_id: "reset-1".to_owned(),
            },
        )
        .await;

        assert!(!broker.cancelled_sessions.lock().contains("sess-reset"));
        assert!(broker.sessions.lock().contains_key("sess-reset"));
        assert!(!broker.resetting_sessions.lock().contains_key("sess-reset"));
        assert!(broker.deleted_session_workspaces.lock().is_empty());
    }

    #[tokio::test]
    async fn reset_cancels_cleanup_before_the_old_worker_disconnects() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-reset-cleanup-race-{}-{}",
            std::process::id(),
            broker_nonce()
        ));
        let worktree_root = root.join("worktrees");
        let workspace = worktree_root.join("sess-reset-race");
        let target = workspace.join("target");
        std::fs::create_dir_all(target.join("debug/deps")).expect("target");
        std::fs::write(target.join(".rustc_info.json"), "{}\n").expect("rustc marker");
        std::fs::write(
            target.join("CACHEDIR.TAG"),
            "Signature: 8a477f597d28d172789f06886806bc55\n",
        )
        .expect("cache tag");
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: root.join("unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: worktree_root.clone(),
            worker_ready_timeout: Duration::from_millis(100),
        }));
        broker.sessions.lock().insert(
            "sess-reset-race".to_owned(),
            StartSession {
                session_id: "sess-reset-race".to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: workspace.display().to_string(),
                agent_session_id: None,
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-1".to_owned(),
                fallback_for: None,
                adopt_only: false,
                execution_binding: None,
            },
        );
        let (worker_tx, _worker_rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: "sess-reset-race".to_owned(),
                epoch: "epoch-reset-race".to_owned(),
                generation: "gen-1".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: None,
                connection_id: 11,
                tx: worker_tx,
            })
            .expect("register old worker");
        let old_peer = broker
            .workers
            .lock()
            .get("sess-reset-race")
            .expect("old worker")
            .clone();
        broker
            .cancelled_sessions
            .lock()
            .insert("sess-reset-race".to_owned());
        broker.deleted_session_workspaces.lock().insert(
            "sess-reset-race".to_owned(),
            DeletedWorkspace {
                workspace: crate::session_workspace::capture_cleanup_workspace(
                    &worktree_root,
                    "sess-reset-race",
                    &workspace,
                )
                .expect("original cleanup workspace"),
                command_id: "delete-before-reset".to_owned(),
            },
        );

        handle_core_command(
            &broker,
            CoreCommand::StopSession {
                session_id: "sess-reset-race".to_owned(),
                command_id: "reset-race".to_owned(),
            },
        )
        .await;
        broker
            .worker_disconnected("sess-reset-race".to_owned(), old_peer, None)
            .await;

        assert!(broker.deleted_session_workspaces.lock().is_empty());
        assert!(target.is_dir());
        assert!(broker.sessions.lock().contains_key("sess-reset-race"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cancelled_worker_reconnect_is_rejected_without_a_pre_handshake_command() {
        let broker = Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(100),
        });
        broker
            .cancelled_sessions
            .lock()
            .insert("sess-deleted-reconnect".to_owned());
        let (worker_tx, mut worker_rx) = mpsc::unbounded_channel();

        let error = broker
            .register_worker(WorkerRegistration {
                session_id: "sess-deleted-reconnect".to_owned(),
                epoch: "epoch-deleted-reconnect".to_owned(),
                generation: "gen-1".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: None,
                connection_id: 9,
                tx: worker_tx,
            })
            .expect_err("deleted worker must not reconnect");

        assert!(error.to_string().contains("was deleted"));
        assert!(broker.workers.lock().is_empty());
        assert!(
            worker_rx.try_recv().is_err(),
            "handle_peer must send Reject as the first and only handshake response"
        );
    }

    #[tokio::test]
    async fn cancelled_worker_handshake_sends_reject_as_its_first_frame() {
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(100),
        }));
        broker
            .cancelled_sessions
            .lock()
            .insert("sess-deleted-handshake".to_owned());
        let (mut worker_stream, broker_stream) = UnixStream::pair().expect("worker socket pair");
        let peer_task = tokio::spawn(handle_peer(Arc::clone(&broker), broker_stream));
        write_frame(
            &mut worker_stream,
            &Frame::Hello {
                role: PeerRole::Worker,
                min_protocol: MIN_PROTOCOL_VERSION,
                max_protocol: PROTOCOL_VERSION,
                build: "test".to_owned(),
                session_id: Some("sess-deleted-handshake".to_owned()),
                worker_epoch: Some("epoch-deleted-handshake".to_owned()),
                generation: Some("gen-1".to_owned()),
                executable: Some("/bin/false".to_owned()),
                fallback_for: None,
            },
        )
        .await
        .expect("send worker hello");

        let first = tokio::time::timeout(Duration::from_secs(1), read_frame(&mut worker_stream))
            .await
            .expect("worker handshake timeout")
            .expect("read worker handshake")
            .expect("worker handshake response");
        assert!(matches!(
            first,
            Frame::Reject { reason } if reason.contains("was deleted")
        ));
        assert!(
            tokio::time::timeout(Duration::from_secs(1), read_frame(&mut worker_stream))
                .await
                .expect("worker handshake close timeout")
                .expect("read worker handshake close")
                .is_none(),
            "Reject must be the only handshake response"
        );
        peer_task
            .await
            .expect("broker peer task")
            .expect("broker peer result");
    }

    #[tokio::test]
    async fn cancelled_worker_final_event_is_acknowledged_locally() {
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(100),
        }));
        let (peer_tx, mut peer_rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: "sess-deleted-final-event".to_owned(),
                epoch: "epoch-deleted-final-event".to_owned(),
                generation: "gen-1".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: None,
                connection_id: 11,
                tx: peer_tx,
            })
            .expect("register worker before deletion");
        broker
            .cancelled_sessions
            .lock()
            .insert("sess-deleted-final-event".to_owned());

        let (mut worker_stream, broker_stream) = UnixStream::pair().expect("worker socket pair");
        let (mut broker_reader, _broker_writer) = broker_stream.into_split();
        let task_broker = Arc::clone(&broker);
        let worker_task = tokio::spawn(async move {
            handle_worker(
                task_broker,
                11,
                "sess-deleted-final-event",
                "epoch-deleted-final-event",
                &mut broker_reader,
            )
            .await
        });
        write_frame(
            &mut worker_stream,
            &Frame::WorkerEvent {
                session_id: "sess-deleted-final-event".to_owned(),
                diagnostics: Vec::new(),
                worker_epoch: "epoch-deleted-final-event".to_owned(),
                runtime_seq: 7,
                event: RuntimeEvent::Status {
                    state: WorkerState::Exited,
                    detail: None,
                },
            },
        )
        .await
        .expect("send final worker event");

        let ack = tokio::time::timeout(Duration::from_secs(1), peer_rx.recv())
            .await
            .expect("deleted worker event ACK timeout")
            .expect("deleted worker event ACK channel");
        assert!(matches!(
            ack,
            Frame::Ack {
                session_id,
                worker_epoch,
                runtime_seq: 7,
            } if session_id == "sess-deleted-final-event"
                && worker_epoch == "epoch-deleted-final-event"
        ));
        assert!(broker.current_controller().is_none());

        drop(worker_stream);
        worker_task
            .await
            .expect("worker reader task")
            .expect("worker reader result");
    }

    #[tokio::test]
    async fn direct_worker_watchdog_reaps_an_unresponsive_process() {
        let workers = Arc::new(Mutex::new(HashMap::new()));
        let mut command = Command::new("sh");
        command.arg("-c").arg("trap '' TERM; while :; do :; done");
        command.kill_on_drop(true);
        command.as_std_mut().process_group(0);
        let mut child = command.spawn().expect("spawn unresponsive worker fixture");
        let pid = child.id().expect("worker fixture pid");
        workers.lock().insert("sess-unresponsive".to_owned(), pid);
        let reaper_workers = Arc::clone(&workers);
        let reaper = tokio::spawn(async move {
            let status = child.wait().await.expect("wait for worker fixture");
            let mut workers = reaper_workers.lock();
            if workers.get("sess-unresponsive").copied() == Some(pid) {
                workers.remove("sess-unresponsive");
            }
            status
        });

        assert!(
            enforce_direct_worker_exit(
                Arc::clone(&workers),
                "sess-unresponsive".to_owned(),
                pid,
                Duration::from_millis(100),
                Duration::from_millis(100),
                Duration::from_secs(1),
            )
            .await,
            "watchdog must observe the exact worker owner exit"
        );
        let status = tokio::time::timeout(Duration::from_secs(1), reaper)
            .await
            .expect("worker reaper timeout")
            .expect("worker reaper task");
        assert!(!status.success());
        assert!(workers.lock().is_empty());
    }

    #[tokio::test]
    async fn deletion_reaps_a_direct_owner_after_its_ipc_peer_disappears() {
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_secs(1),
        }));
        let mut command = Command::new("sh");
        command.arg("-c").arg("sleep 60");
        command.kill_on_drop(true);
        command.as_std_mut().process_group(0);
        let mut child = command.spawn().expect("spawn disconnected worker fixture");
        let pid = child.id().expect("disconnected worker fixture pid");
        broker
            .direct_worker_pids
            .lock()
            .insert("sess-disconnected-delete".to_owned(), pid);
        let workers = Arc::clone(&broker.direct_worker_pids);
        let reaper = tokio::spawn(async move {
            let status = child.wait().await.expect("wait for disconnected worker");
            if reap_direct_worker_group(&workers, "sess-disconnected-delete", pid).await {
                let mut workers = workers.lock();
                if workers.get("sess-disconnected-delete").copied() == Some(pid) {
                    workers.remove("sess-disconnected-delete");
                }
            }
            status
        });

        handle_core_command(
            &broker,
            CoreCommand::StopSession {
                session_id: "sess-disconnected-delete".to_owned(),
                command_id: "delete-disconnected-owner".to_owned(),
            },
        )
        .await;

        let status = tokio::time::timeout(Duration::from_secs(5), reaper)
            .await
            .expect("direct deletion watchdog timeout")
            .expect("direct deletion reaper");
        assert!(!status.success());
        assert!(broker.direct_worker_pids.lock().is_empty());
    }

    #[tokio::test]
    async fn direct_worker_reaper_terminates_remaining_process_group_descendants() {
        let workers = Arc::new(Mutex::new(HashMap::new()));
        let mut command = Command::new("sh");
        command.arg("-c").arg("trap '' HUP; sleep 60 &");
        command.kill_on_drop(true);
        command.as_std_mut().process_group(0);
        let mut child = command.spawn().expect("spawn worker descendant fixture");
        let pid = child.id().expect("worker descendant fixture pid");
        workers.lock().insert("sess-descendant".to_owned(), pid);
        let status = match tokio::time::timeout(Duration::from_secs(1), child.wait()).await {
            Ok(result) => result.expect("wait for worker descendant fixture"),
            Err(_) => {
                let _ = signal_direct_worker(
                    &workers,
                    "sess-descendant",
                    pid,
                    rustix::process::Signal::KILL,
                );
                panic!("worker fixture leader did not exit");
            }
        };
        assert!(status.success());
        assert!(
            direct_worker_group_exists(pid),
            "fixture must leave a process-group descendant"
        );

        assert!(reap_direct_worker_group(&workers, "sess-descendant", pid).await);
        assert!(!direct_worker_group_exists(pid));
        workers.lock().remove("sess-descendant");
    }

    #[tokio::test]
    async fn direct_worker_replacement_waits_for_the_previous_process_owner() {
        let broker = Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_secs(1),
        });
        let mut old_command = Command::new("sh");
        old_command.arg("-c").arg("sleep 0.2");
        old_command.kill_on_drop(true);
        old_command.as_std_mut().process_group(0);
        let mut old_child = old_command.spawn().expect("spawn previous worker fixture");
        let old_pid = old_child.id().expect("previous worker fixture pid");
        broker
            .direct_worker_pids
            .lock()
            .insert("sess-replacement".to_owned(), old_pid);
        let old_workers = Arc::clone(&broker.direct_worker_pids);
        let old_reaper = tokio::spawn(async move {
            old_child.wait().await.expect("wait for previous worker");
            if reap_direct_worker_group(&old_workers, "sess-replacement", old_pid).await {
                let mut workers = old_workers.lock();
                if workers.get("sess-replacement").copied() == Some(old_pid) {
                    workers.remove("sess-replacement");
                }
            }
        });
        let session = StartSession {
            session_id: "sess-replacement".to_owned(),
            provider: "codex".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_auth_generation: None,
            provider_behavior: None,
            cwd: "/tmp".to_owned(),
            agent_session_id: None,
            system: true,
            context_window: None,
            auto_compact_token_limit: None,
            cache_protection: None,
            generation: "gen-1".to_owned(),
            fallback_for: None,
            adopt_only: false,
            execution_binding: None,
        };

        let replacement_exit = broker
            .spawn_worker(&session)
            .await
            .expect("spawn replacement after previous owner exits");
        old_reaper.await.expect("previous worker reaper");
        let replacement_status = tokio::time::timeout(Duration::from_secs(1), replacement_exit)
            .await
            .expect("replacement worker exit timeout")
            .expect("replacement worker exit channel")
            .expect("replacement worker wait");
        assert!(!replacement_status.success());
        assert!(broker.direct_worker_pids.lock().is_empty());
    }

    #[tokio::test]
    async fn replacement_before_cleanup_retires_the_original_observation_without_deleting_targets()
    {
        let temp = tempfile::tempdir().expect("cleanup fixture");
        let managed = temp.path().join("worktrees");
        let workspace = managed.join("sess-replaced");
        let original = managed.join("original");
        let write_target = |root: &Path| {
            let target = root.join("target");
            std::fs::create_dir_all(&target).unwrap();
            std::fs::write(target.join(".rustc_info.json"), "{}").unwrap();
            std::fs::write(
                target.join("CACHEDIR.TAG"),
                "Signature: 8a477f597d28d172789f06886806bc55\n",
            )
            .unwrap();
            std::fs::write(target.join("artifact"), "preserve").unwrap();
        };
        write_target(&workspace);
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: temp.path().join("unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: managed.clone(),
            worker_ready_timeout: Duration::from_secs(1),
        }));
        broker
            .deleted_session_owner_collected
            .store(true, Ordering::Release);
        broker
            .cancelled_sessions
            .lock()
            .insert("sess-replaced".into());
        broker.deleted_session_workspaces.lock().insert(
            "sess-replaced".into(),
            DeletedWorkspace {
                workspace: crate::session_workspace::capture_cleanup_workspace(
                    &managed,
                    "sess-replaced",
                    &workspace,
                )
                .unwrap(),
                command_id: "delete".into(),
            },
        );
        let gate = broker.session_lifecycle_gate("sess-replaced");
        let guard = gate.lock().await;
        broker.cleanup_deleted_session("sess-replaced", "delete");
        std::fs::rename(&workspace, &original).unwrap();
        write_target(&workspace);
        drop(guard);
        tokio::time::timeout(Duration::from_secs(2), async {
            while !broker.deleted_session_workspaces.lock().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("replacement ends cleanup without retry");
        assert!(workspace.join("target/artifact").is_file());
        assert!(original.join("target/artifact").is_file());
        assert!(broker.cancelled_sessions.lock().contains("sess-replaced"));
    }

    #[tokio::test]
    async fn confirmed_process_exit_reclaims_targets_and_preserves_the_worktree() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-deleted-session-cleanup-{}-{}",
            std::process::id(),
            broker_nonce()
        ));
        let worktree_root = root.join("worktrees");
        let workspace = worktree_root.join("sess-delete");
        let target = workspace.join("target");
        std::fs::create_dir_all(target.join("debug/deps")).expect("target");
        std::fs::write(workspace.join("source.rs"), "fn main() {}\n").expect("source");
        std::fs::write(target.join(".rustc_info.json"), "{}\n").expect("rustc marker");
        std::fs::write(
            target.join("CACHEDIR.TAG"),
            "Signature: 8a477f597d28d172789f06886806bc55\n",
        )
        .expect("cache tag");
        std::fs::write(target.join("debug/deps/libtest.rlib"), "generated\n").expect("artifact");
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: root.join("unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: worktree_root.clone(),
            worker_ready_timeout: Duration::from_secs(1),
        }));
        broker
            .deleted_session_owner_collected
            .store(true, Ordering::Release);
        broker.sessions.lock().insert(
            "sess-delete".to_owned(),
            StartSession {
                session_id: "sess-delete".to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: workspace.display().to_string(),
                agent_session_id: None,
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-1".to_owned(),
                fallback_for: None,
                adopt_only: false,
                execution_binding: None,
            },
        );

        handle_core_command(
            &broker,
            CoreCommand::StopSession {
                session_id: "sess-delete".to_owned(),
                command_id: "stop-delete".to_owned(),
            },
        )
        .await;
        for _ in 0..100 {
            // Clearing target contents precedes the asynchronous bookkeeping update.
            // Wait for the cleanup task's completion, not that intermediate step.
            if broker.deleted_session_workspaces.lock().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert!(broker.deleted_session_workspaces.lock().is_empty());
        assert!(target.is_dir());
        assert!(!target.join(".rustc_info.json").exists());
        assert!(!target.join("CACHEDIR.TAG").exists());
        assert!(!target.join("debug/deps/libtest.rlib").exists());
        assert!(workspace.join("source.rs").is_file());
        assert!(workspace.is_dir());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn broker_disconnect_without_process_exit_proof_preserves_targets() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-connected-session-cleanup-{}-{}",
            std::process::id(),
            broker_nonce()
        ));
        let worktree_root = root.join("worktrees");
        let workspace = worktree_root.join("sess-connected-delete");
        let target = workspace.join("target");
        std::fs::create_dir_all(target.join("debug/deps")).expect("target");
        std::fs::write(workspace.join("source.rs"), "fn main() {}\n").expect("source");
        std::fs::write(target.join(".rustc_info.json"), "{}\n").expect("rustc marker");
        std::fs::write(
            target.join("CACHEDIR.TAG"),
            "Signature: 8a477f597d28d172789f06886806bc55\n",
        )
        .expect("cache tag");
        std::fs::write(target.join("debug/deps/libtest.rlib"), "generated\n").expect("artifact");
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: root.join("unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: worktree_root.clone(),
            worker_ready_timeout: Duration::from_secs(1),
        }));
        broker.sessions.lock().insert(
            "sess-connected-delete".to_owned(),
            StartSession {
                session_id: "sess-connected-delete".to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: workspace.display().to_string(),
                agent_session_id: None,
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-1".to_owned(),
                fallback_for: None,
                adopt_only: false,
                execution_binding: None,
            },
        );
        let (worker_tx, _worker_rx) = mpsc::unbounded_channel();
        broker
            .register_worker(WorkerRegistration {
                session_id: "sess-connected-delete".to_owned(),
                epoch: "epoch-connected-delete".to_owned(),
                generation: "gen-1".to_owned(),
                executable: Some("/bin/false".to_owned()),
                fallback_for: None,
                connection_id: 7,
                tx: worker_tx,
            })
            .expect("register worker");

        handle_core_command(
            &broker,
            CoreCommand::StopSession {
                session_id: "sess-connected-delete".to_owned(),
                command_id: "stop-connected-delete".to_owned(),
            },
        )
        .await;

        assert!(target.is_dir(), "connected worker still owns its target");
        let peer = broker
            .remove_worker("sess-connected-delete", 7)
            .expect("registered worker");
        broker
            .worker_disconnected("sess-connected-delete".to_owned(), peer, None)
            .await;
        for _ in 0..100 {
            if broker.deleted_session_workspaces.lock().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert!(broker.deleted_session_workspaces.lock().is_empty());
        assert!(target.is_dir());
        assert!(workspace.join("source.rs").is_file());
        assert!(workspace.is_dir());
        let _ = std::fs::remove_dir_all(root);
    }

    fn continuation_broker(root: &Path, collected: bool) -> Arc<Broker> {
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: root.join("unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: root.join("worktrees"),
            worker_ready_timeout: Duration::from_secs(1),
        }));
        broker
            .deleted_session_owner_collected
            .store(collected, Ordering::Release);
        // A sibling test may be between fork and exec with an inherited copy of
        // a just-closed namespace lock; only that window is retried.
        broker.attach_deletion_journal(retry_while_owned(|| {
            deletions::Journal::open(&root.join("deletions"), deletion_fixture_owner(), true)
        }));
        broker.attach_cleanup_continuations(retry_while_owned(|| {
            cleanups::Store::open(&root.join("cleanups"), deletion_fixture_owner())
        }));
        broker
    }

    fn retry_while_owned<T>(mut open: impl FnMut() -> Result<T>) -> T {
        for _ in 0..300 {
            match open() {
                Ok(opened) => return opened,
                Err(error) if format!("{error:#}").contains("already owned") => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("opening durable namespace: {error:#}"),
            }
        }
        panic!("durable namespace stayed owned");
    }

    /// Release the durable namespaces as process exit would. Detached tasks may
    /// still hold the broker `Arc`, so waiting for a drop is not deterministic.
    fn end_resident(broker: &Broker) {
        drop(broker.deletion_journal.lock().take());
        drop(broker.cleanup_continuations.lock().take());
    }

    fn write_continuation_target(root: &Path) {
        let target = root.join("target");
        std::fs::create_dir_all(target.join("debug")).unwrap();
        std::fs::write(target.join(".rustc_info.json"), "{}").unwrap();
        std::fs::write(
            target.join("CACHEDIR.TAG"),
            "Signature: 8a477f597d28d172789f06886806bc55\n",
        )
        .unwrap();
        std::fs::write(target.join("debug/artifact"), "generated").unwrap();
    }

    fn pending_continuations(broker: &Broker) -> Vec<String> {
        broker
            .cleanup_continuations
            .lock()
            .as_ref()
            .unwrap()
            .pending()
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    async fn wait_for_no_continuations(broker: &Broker) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !pending_continuations(broker).is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("continuation retired");
    }

    fn stop_fixture_session(broker: &Broker, id: &str, cwd: &Path) {
        broker.sessions.lock().insert(
            id.to_owned(),
            StartSession {
                session_id: id.to_owned(),
                provider: "codex".to_owned(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: cwd.display().to_string(),
                agent_session_id: None,
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                generation: "gen-1".to_owned(),
                fallback_for: None,
                adopt_only: false,
                execution_binding: None,
            },
        );
    }

    #[tokio::test]
    async fn accepted_deletion_resumes_cleanup_after_a_resident_restart() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("worktrees/sess-restart");
        write_continuation_target(&workspace);
        std::fs::write(workspace.join("source.rs"), "fn main() {}\n").unwrap();

        // The first resident accepts deletion but has no process-exit proof, so
        // it preserves the artifacts and leaves its continuation pending.
        let first = continuation_broker(temp.path(), false);
        stop_fixture_session(&first, "sess-restart", &workspace);
        let (tx, mut rx) = mpsc::unbounded_channel();
        first.install_controller(tx);
        handle_core_command(
            &first,
            CoreCommand::StopSession {
                session_id: "sess-restart".into(),
                command_id: "delete-restart".into(),
            },
        )
        .await;
        assert!(matches!(
            rx.try_recv(),
            Ok(Frame::CommandAck { accepted: true, .. })
        ));
        assert_eq!(pending_continuations(&first), ["sess-restart"]);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(workspace.join("target/debug/artifact").is_file());
        drop(rx);
        end_resident(&first);

        // The next resident reads the committed deletion and continuation.
        let second = continuation_broker(temp.path(), true);
        assert!(second.cancelled_sessions.lock().contains("sess-restart"));
        assert_eq!(pending_continuations(&second), ["sess-restart"]);
        second.resume_cleanup_continuations();
        wait_for_no_continuations(&second).await;
        assert!(workspace.join("target").is_dir());
        assert!(!workspace.join("target/debug/artifact").exists());
        assert!(!workspace.join("target/CACHEDIR.TAG").exists());
        assert!(workspace.join("source.rs").is_file());
        assert!(second.deleted_session_workspaces.lock().is_empty());
        end_resident(&second);

        // Completion is durable: a later resident has nothing to resume.
        let third = continuation_broker(temp.path(), true);
        assert!(pending_continuations(&third).is_empty());
    }

    #[tokio::test]
    async fn a_failing_resumed_cleanup_releases_its_handles_and_keeps_the_nomination() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("worktrees/sess-stuck");
        write_continuation_target(&workspace);
        let first = continuation_broker(temp.path(), false);
        stop_fixture_session(&first, "sess-stuck", &workspace);
        handle_core_command(
            &first,
            CoreCommand::StopSession {
                session_id: "sess-stuck".into(),
                command_id: "delete-stuck".into(),
            },
        )
        .await;
        assert_eq!(pending_continuations(&first), ["sess-stuck"]);
        end_resident(&first);

        // A launch that never settles makes every attempt fail closed.
        let second = continuation_broker(temp.path(), true);
        *second.cleanup_retry.lock() = (3, Duration::from_millis(10));
        second.launching.lock().insert("sess-stuck".to_owned());
        second.resume_cleanup_continuations();
        // First the resumed Session must actually hold its workspace (each
        // attempt waits out the launch timeout), then release it by giving up.
        tokio::time::timeout(Duration::from_secs(10), async {
            while second.deleted_session_workspaces.lock().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("resume holds the workspace while attempting");
        tokio::time::timeout(Duration::from_secs(30), async {
            while !second.deleted_session_workspaces.lock().is_empty() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("bounded retries release the retained workspace");
        // Nothing was removed, and the durable record still names the root.
        assert!(workspace.join("target/debug/artifact").is_file());
        assert_eq!(pending_continuations(&second), ["sess-stuck"]);
        end_resident(&second);

        // The next resident, with the obstruction gone, completes it.
        let third = continuation_broker(temp.path(), true);
        third.resume_cleanup_continuations();
        wait_for_no_continuations(&third).await;
        assert!(!workspace.join("target/debug/artifact").exists());
    }

    #[tokio::test]
    async fn replaced_root_retires_the_continuation_without_touching_either_tree() {
        let temp = tempfile::tempdir().unwrap();
        let managed = temp.path().join("worktrees");
        let workspace = managed.join("sess-swapped");
        write_continuation_target(&workspace);
        let first = continuation_broker(temp.path(), false);
        stop_fixture_session(&first, "sess-swapped", &workspace);
        handle_core_command(
            &first,
            CoreCommand::StopSession {
                session_id: "sess-swapped".into(),
                command_id: "delete-swapped".into(),
            },
        )
        .await;
        assert_eq!(pending_continuations(&first), ["sess-swapped"]);
        end_resident(&first);

        // While no resident is running the root is replaced by a new object.
        std::fs::rename(&workspace, managed.join("original")).unwrap();
        write_continuation_target(&workspace);

        let second = continuation_broker(temp.path(), true);
        second.resume_cleanup_continuations();
        wait_for_no_continuations(&second).await;
        assert!(workspace.join("target/debug/artifact").is_file());
        assert!(managed.join("original/target/debug/artifact").is_file());
    }

    #[tokio::test]
    async fn missing_or_linked_root_retires_and_foreign_nomination_is_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        let managed = temp.path().join("worktrees");
        let kept = temp.path().join("kept");
        write_continuation_target(&kept);
        std::fs::create_dir_all(&managed).unwrap();
        let identity = |dir: &Path| {
            crate::session_workspace::capture_cleanup_workspace(
                dir.parent().unwrap(),
                dir.file_name().unwrap().to_str().unwrap(),
                dir,
            )
            .unwrap()
            .root_identity()
            .unwrap()
        };
        let gone = managed.join("sess-gone");
        std::fs::create_dir_all(&gone).unwrap();
        let gone_identity = identity(&gone);
        std::fs::remove_dir_all(&gone).unwrap();
        let linked = managed.join("sess-linked");
        std::fs::create_dir_all(&linked).unwrap();
        let linked_identity = identity(&linked);
        std::fs::remove_dir_all(&linked).unwrap();
        std::os::unix::fs::symlink(&kept, &linked).unwrap();
        let foreign = managed.join("sess-foreign");
        write_continuation_target(&foreign);
        let foreign_identity = identity(&foreign);

        let broker = continuation_broker(temp.path(), true);
        {
            let mut journal = broker.deletion_journal.lock();
            let journal = journal.as_mut().unwrap();
            journal.mark_deleted("sess-gone").unwrap();
            journal.mark_deleted("sess-linked").unwrap();
        }
        {
            let mut store = broker.cleanup_continuations.lock();
            let store = store.as_mut().unwrap();
            store.record("sess-gone", gone_identity).unwrap();
            store.record("sess-linked", linked_identity).unwrap();
            // No committed terminal deletion names this Session.
            store.record("sess-foreign", foreign_identity).unwrap();
        }
        broker
            .cancelled_sessions
            .lock()
            .extend(["sess-gone".to_owned(), "sess-linked".to_owned()]);
        broker.resume_cleanup_continuations();
        tokio::time::timeout(Duration::from_secs(10), async {
            while pending_continuations(&broker) != ["sess-foreign"] {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("both observed-gone roots retire");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(pending_continuations(&broker), ["sess-foreign"]);
        assert!(foreign.join("target/debug/artifact").is_file());
        assert!(kept.join("target/debug/artifact").is_file());
    }

    #[tokio::test]
    async fn reader_only_or_unavailable_namespace_keeps_deletion_working() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("worktrees/sess-no-store");
        write_continuation_target(&workspace);
        let broker = continuation_broker(temp.path(), false);
        *broker.cleanup_continuations.lock() = None;
        stop_fixture_session(&broker, "sess-no-store", &workspace);
        let (tx, mut rx) = mpsc::unbounded_channel();
        broker.install_controller(tx);
        handle_core_command(
            &broker,
            CoreCommand::StopSession {
                session_id: "sess-no-store".into(),
                command_id: "delete-no-store".into(),
            },
        )
        .await;
        assert!(matches!(
            rx.try_recv(),
            Ok(Frame::CommandAck { accepted: true, .. })
        ));
        assert!(broker.cancelled_sessions.lock().contains("sess-no-store"));
        broker.resume_cleanup_continuations();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(workspace.join("target/debug/artifact").is_file());
        assert!(!temp.path().join("cleanups/cleanups.json").exists());
    }

    #[tokio::test]
    async fn deleted_launch_does_not_quarantine_the_worker_generation() {
        let broker = Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-2".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        });
        broker
            .cancelled_sessions
            .lock()
            .insert("sess-deleted".to_owned());

        let error = broker
            .spawn_with_fallback(
                StartSession {
                    session_id: "sess-deleted".to_owned(),
                    provider: "codex".to_owned(),
                    provider_version: String::new(),
                    provider_generation_digest: String::new(),
                    provider_auth_generation: None,
                    provider_behavior: None,
                    cwd: "/work".to_owned(),
                    agent_session_id: None,
                    system: false,
                    context_window: None,
                    auto_compact_token_limit: None,
                    cache_protection: None,
                    generation: "gen-2".to_owned(),
                    fallback_for: None,
                    adopt_only: false,
                    execution_binding: None,
                },
                Some("gen-1".to_owned()),
            )
            .await
            .expect_err("deleted launch must stop");

        assert!(error.to_string().contains("deleted during launch"));
        assert!(broker.unhealthy_generations.lock().is_empty());
        assert!(broker.fallback_pins.lock().is_empty());
        assert!(broker.fallback_targets.lock().is_empty());
    }

    #[tokio::test]
    async fn deletion_retracts_only_its_generation_failure() {
        let broker = Arc::new(Broker::new(MachineBrokerArgs {
            socket: PathBuf::from("/tmp/unused.sock"),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-2".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(10),
        }));
        broker.unhealthy_generations.lock().insert(
            ("gen-2".to_owned(), "codex".to_owned()),
            HashSet::from(["sess-deleted".to_owned(), "sess-live".to_owned()]),
        );

        handle_core_command(
            &broker,
            CoreCommand::StopSession {
                session_id: "sess-deleted".to_owned(),
                command_id: "delete-1".to_owned(),
            },
        )
        .await;

        assert_eq!(
            broker
                .unhealthy_generations
                .lock()
                .get(&("gen-2".to_owned(), "codex".to_owned()))
                .cloned(),
            Some(HashSet::from(["sess-live".to_owned()]))
        );
    }

    #[tokio::test]
    async fn worker_replay_survives_controller_reconnect() {
        let socket = test_socket();
        let task = tokio::spawn(run(MachineBrokerArgs {
            socket: socket.clone(),
            worker_command: PathBuf::from("/bin/false"),
            desired_generation: "gen-1".to_owned(),
            spawn_mode: SpawnMode::Direct,
            worker_environment: BTreeMap::new(),
            provider_store: test_provider_store(),
            worktree_root: PathBuf::from("/tmp/unused-worktrees"),
            worker_ready_timeout: Duration::from_millis(100),
        }));
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        let (mut core_reader, core_writer, welcome) =
            connect_peer(&socket, PeerRole::Core, None, None).await;
        assert!(matches!(welcome, Frame::Welcome { .. }));
        let (mut worker_reader, mut worker_writer, welcome) =
            connect_peer(&socket, PeerRole::Worker, Some("sess-1"), Some("epoch-1")).await;
        assert!(matches!(welcome, Frame::Welcome { .. }));
        write_frame(
            &mut worker_writer,
            &Frame::Snapshot {
                worker: Box::new(WorkerSnapshot {
                    session_id: "sess-1".to_owned(),
                    worker_epoch: "epoch-1".to_owned(),
                    generation: "gen-1".to_owned(),
                    executable: Some("/bin/false".to_owned()),
                    launch: None,
                    state: WorkerState::Busy,
                    agent_session_id: Some("agent-1".to_owned()),
                    native_thread_materialized: None,
                    current_turn_id: Some("turn-1".to_owned()),
                    last_runtime_seq: 1,
                    pending_permissions: Vec::new(),
                    config_options: None,
                    context_used: None,
                    context_size: None,
                    pending_prompt_count: 0,
                    drain_requested: false,
                    exit_detail: None,
                    background_tasks: None,
                    incarnation: None,
                }),
            },
        )
        .await
        .expect("send snapshot");
        assert!(matches!(
            read_frame(&mut core_reader).await.expect("snapshot read"),
            Some(Frame::Snapshot { .. })
        ));
        let event = Frame::WorkerEvent {
            diagnostics: Vec::new(),
            session_id: "sess-1".to_owned(),
            worker_epoch: "epoch-1".to_owned(),
            runtime_seq: 1,
            event: RuntimeEvent::Update {
                update: serde_json::json!({"sessionUpdate": "agent_message_chunk"}),
                cmid: None,
            },
        };
        write_frame(&mut worker_writer, &event)
            .await
            .expect("send event");
        assert_eq!(
            read_frame(&mut core_reader).await.expect("event read"),
            Some(event.clone())
        );

        drop(core_reader);
        drop(core_writer);
        let (mut next_core_reader, mut next_core_writer, welcome) =
            connect_peer(&socket, PeerRole::Core, None, None).await;
        assert!(matches!(welcome, Frame::Welcome { workers, .. } if workers.len() == 1));
        let replay = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            read_frame(&mut worker_reader),
        )
        .await
        .expect("replay timeout")
        .expect("replay read")
        .expect("replay frame");
        assert!(matches!(
            replay,
            Frame::Replay {
                session_id,
                worker_epoch,
                after_runtime_seq: 1,
            } if session_id == "sess-1" && worker_epoch == "epoch-1"
        ));
        write_frame(&mut worker_writer, &event)
            .await
            .expect("replay event");
        assert_eq!(
            read_frame(&mut next_core_reader)
                .await
                .expect("replayed event read"),
            Some(event)
        );
        write_frame(
            &mut next_core_writer,
            &Frame::Ack {
                session_id: "sess-1".to_owned(),
                worker_epoch: "epoch-1".to_owned(),
                runtime_seq: 1,
            },
        )
        .await
        .expect("core ack");
        assert!(matches!(
            read_frame(&mut worker_reader)
                .await
                .expect("worker ack read"),
            Some(Frame::Ack { runtime_seq: 1, .. })
        ));

        task.abort();
        let _ = tokio::fs::remove_file(socket).await;
    }
}
