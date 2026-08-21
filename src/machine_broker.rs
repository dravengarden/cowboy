//! Stable Machine-local broker for detached ACP workers.
//!
//! The broker deliberately contains no Cowboy business state and no ACP parsing.
//! It grants one controller lease, routes commands, starts session workers, and
//! lets workers replay their unacknowledged outboxes after either side restarts.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::os::fd::FromRawFd as _;
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

const WORKER_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(45);
const WORKER_MONITOR_INTERVAL: Duration = Duration::from_secs(15);
const TRANSIENT_UNIT_COLLECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnMode {
    /// Child process mode for development and hermetic tests. Production uses
    /// user-systemd so worker units are siblings and survive Machine broker restarts.
    Direct,
    SystemdUser,
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
    pub provider_store: Arc<crate::machine_providers::MachineProviderStore>,
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
    cwd: String,
    command_id: String,
}

struct Broker {
    args: MachineBrokerArgs,
    controller: Mutex<Option<Controller>>,
    workers: Mutex<HashMap<String, WorkerPeer>>,
    pending_commands: Mutex<HashMap<String, VecDeque<WorkerCommand>>>,
    sessions: Mutex<HashMap<String, StartSession>>,
    session_states: Mutex<HashMap<String, WorkerState>>,
    launching: Mutex<HashSet<String>>,
    awaiting_reconnect: Mutex<HashSet<String>>,
    cancelled_sessions: Mutex<HashSet<String>>,
    /// Session workspaces awaiting generated-artifact cleanup after their
    /// process owner has been stopped and collected. Source worktrees and
    /// branches are retained.
    deleted_session_workspaces: Mutex<HashMap<String, DeletedWorkspace>>,
    /// Serializes reset/relaunch with generated-artifact deletion per session.
    /// Cleanup may hold a gate across filesystem I/O so a replacement cannot
    /// start midway without blocking unrelated sessions.
    deleted_session_cleanup_gates: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
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
    desired_generation: Mutex<String>,
    previous_generation: Mutex<Option<String>>,
    generation_commands: Mutex<HashMap<String, PathBuf>>,
    #[cfg(test)]
    deleted_session_owner_collected: AtomicBool,
    next_connection: AtomicU64,
    next_lease: AtomicU64,
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
            sessions: Mutex::new(HashMap::new()),
            session_states: Mutex::new(HashMap::new()),
            launching: Mutex::new(HashSet::new()),
            awaiting_reconnect: Mutex::new(HashSet::new()),
            cancelled_sessions: Mutex::new(HashSet::new()),
            deleted_session_workspaces: Mutex::new(HashMap::new()),
            deleted_session_cleanup_gates: Mutex::new(HashMap::new()),
            resetting_sessions: Mutex::new(HashMap::new()),
            replacing: Mutex::new(HashMap::new()),
            fallback_pins: Mutex::new(HashMap::new()),
            fallback_targets: Mutex::new(HashMap::new()),
            unhealthy_generations: Mutex::new(HashMap::new()),
            healthy_generations: Mutex::new(HashSet::new()),
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
                current_turn_id: None,
                last_runtime_seq: 0,
                pending_permissions: Vec::new(),
                config_options: None,
                context_used: None,
                context_size: None,
                pending_prompt_count: pending_prompts.get(&session_id).copied().unwrap_or(0),
                drain_requested: false,
            });
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
        match self
            .workers
            .lock()
            .get(session_id)
            .map(|worker| worker.tx.clone())
        {
            Some(tx) => {
                let _ = tx.send(Frame::WorkerCommand {
                    session_id: session_id.to_owned(),
                    command,
                });
            }
            _ => {
                self.queue_pending(session_id, command);
            }
        }
    }

    /// New prompts wait behind a generation handoff. Cancellation and
    /// permission replies still route to the old worker because they are part
    /// of the in-flight turn that must reach its safe boundary.
    fn route_prompt(&self, session_id: &str, command: WorkerCommand) {
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

    fn take_stale_workers(&self, timeout: Duration) -> Vec<(String, WorkerPeer)> {
        let now = Instant::now();
        let mut workers = self.workers.lock();
        let stale: Vec<String> = workers
            .iter()
            .filter(|(_, worker)| now.duration_since(worker.last_seen) >= timeout)
            .map(|(session_id, _)| session_id.clone())
            .collect();
        stale
            .into_iter()
            .filter_map(|session_id| {
                workers
                    .remove(&session_id)
                    .map(|worker| (session_id, worker))
            })
            .collect()
    }

    fn update_snapshot(&self, mut snapshot: WorkerSnapshot, connection_id: u64) {
        let session_id = snapshot.session_id.clone();
        let state = snapshot.state;
        let launch = snapshot.launch.clone();
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
            return;
        }
        let cancelled = self.cancelled_sessions.lock();
        if cancelled.contains(&session_id) {
            return;
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
            self.sessions.lock().insert(session_id.clone(), launch);
        }
        self.session_states.lock().insert(session_id, state);
        drop(cancelled);
    }

    fn touch_worker(&self, session_id: &str, connection_id: u64) {
        if let Some(worker) = self.workers.lock().get_mut(session_id)
            && worker.connection_id == connection_id
        {
            worker.last_seen = Instant::now();
        }
    }

    fn register_worker(&self, registration: WorkerRegistration) -> Result<()> {
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
            let _ = tx.send(Frame::WorkerCommand {
                session_id: session_id.clone(),
                command: WorkerCommand::Stop {
                    command_id: format!("deleted-session-stop-{session_id}"),
                },
            });
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
                    current_turn_id: None,
                    last_runtime_seq: 0,
                    pending_permissions: Vec::new(),
                    config_options: None,
                    context_used: None,
                    context_size: None,
                    pending_prompt_count: 0,
                    drain_requested: !desired.is_empty() && generation != desired && !pinned,
                },
                last_seen: Instant::now(),
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
                    command,
                });
            }
            if !held_prompts.is_empty() {
                self.pending_commands
                    .lock()
                    .insert(session_id.to_owned(), held_prompts);
            }
        }
    }

    fn send_controller(&self, frame: Frame) {
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

    fn deleted_session_cleanup_gate(&self, session_id: &str) -> Arc<AsyncMutex<()>> {
        self.deleted_session_cleanup_gates
            .lock()
            .entry(session_id.to_owned())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
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

    fn cleanup_deleted_session(self: &Arc<Self>, session_id: &str, command_id: &str) {
        let broker = Arc::clone(self);
        let session_id = session_id.to_owned();
        let command_id = command_id.to_owned();
        tokio::spawn(async move {
            let mut retry_delay = Duration::from_secs(1);
            loop {
                let gate = broker.deleted_session_cleanup_gate(&session_id);
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
                    let removed = crate::session_workspace::cleanup_build_artifacts(
                        &broker.args.worktree_root,
                        &session_id,
                        Path::new(&workspace.cwd),
                    )
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
                        return;
                    }
                    Ok(Some(removed)) => {
                        tracing::info!(
                            session = %session_id,
                            targets = removed.len(),
                            "reclaimed deleted session Cargo targets"
                        );
                        return;
                    }
                    Err(error) => {
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
        });
    }

    async fn ensure_session(self: &Arc<Self>, session: StartSession) {
        let adopt_only = session.adopt_only;
        let mut session = session;
        // `adopt_only` describes this controller message, not how a future
        // replacement worker should be launched or report its own snapshot.
        session.adopt_only = false;
        self.sessions
            .lock()
            .insert(session.session_id.clone(), session.clone());
        if self.workers.lock().contains_key(&session.session_id) {
            if let Some(worker) = self
                .snapshots()
                .into_iter()
                .find(|worker| worker.session_id == session.session_id)
            {
                self.send_controller(Frame::Snapshot {
                    worker: Box::new(worker),
                });
            }
            return;
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
        self.revoke_cache_protection(&session, &session_id, "session_reset");
        session.adopt_only = false;
        let cleanup_gate = self.deleted_session_cleanup_gate(&session_id);
        let _cleanup_guard = cleanup_gate.lock().await;
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
        self.ensure_session(session).await;
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
                    self.force_recycle_failed_start(&session.session_id).await;
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
                    self.force_recycle_failed_start(&session.session_id).await;
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
        match event {
            RuntimeEvent::Ready { agent_session_id } => {
                worker.snapshot.state = WorkerState::Running;
                if agent_session_id.is_some() {
                    worker
                        .snapshot
                        .agent_session_id
                        .clone_from(agent_session_id);
                }
            }
            RuntimeEvent::Status { state, .. } => worker.snapshot.state = *state,
            RuntimeEvent::TurnStarted { turn_id, .. } => {
                worker.snapshot.state = WorkerState::Busy;
                worker.snapshot.current_turn_id = Some(turn_id.clone());
            }
            RuntimeEvent::TurnEnded { turn_id, .. } => {
                if worker.snapshot.current_turn_id.as_deref() == Some(turn_id) {
                    worker.snapshot.current_turn_id = None;
                }
            }
            RuntimeEvent::AgentSessionId { agent_session_id } => {
                worker.snapshot.agent_session_id = Some(agent_session_id.clone());
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
            RuntimeEvent::Update { .. }
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
        if !matches!(event, RuntimeEvent::Ready { .. })
            || generation != *self.desired_generation.lock()
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

    async fn worker_disconnected(self: &Arc<Self>, session_id: String, peer: WorkerPeer) {
        if !self.sessions.lock().contains_key(&session_id) {
            self.session_states.lock().remove(&session_id);
            self.pending_commands.lock().remove(&session_id);
            self.unpin_fallback(&session_id);
            return;
        }
        let Some(old_generation) = self.replacing.lock().remove(&session_id) else {
            let mut snapshot = peer.snapshot;
            snapshot.state = WorkerState::Crashed;
            snapshot.current_turn_id = None;
            snapshot.pending_permissions.clear();
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
        session.agent_session_id = peer.snapshot.agent_session_id;
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
        let provider = (!session.provider_generation_digest.is_empty())
            .then(|| {
                self.args.provider_store.launch_context(
                    &session.provider,
                    &session.provider_generation_digest,
                    session.provider_auth_generation,
                )
            })
            .transpose()?;
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
        let mut child = command
            .spawn()
            .with_context(|| match self.args.spawn_mode {
                SpawnMode::Direct => "spawning worker process",
                SpawnMode::SystemdUser => "starting transient worker unit",
            })?;
        let session_id = session.session_id.clone();
        let (exit_tx, exit_rx) = oneshot::channel();
        tokio::spawn(async move {
            let result = child.wait().await.map_err(|error| error.to_string());
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

pub async fn run(args: MachineBrokerArgs) -> Result<()> {
    let listener = match inherited_systemd_listener()? {
        Some(listener) => {
            tracing::info!(socket = %args.socket.display(), "cowboy Machine broker using systemd socket");
            listener
        }
        None => bind_runtime_listener(&args.socket).await?,
    };
    let broker = Arc::new(Broker::new(args));
    tokio::spawn(monitor_workers(Arc::clone(&broker)));
    accept_runtime_peers(broker.args.socket.clone(), listener, broker).await
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
        for (session_id, peer) in broker.take_stale_workers(WORKER_HEARTBEAT_TIMEOUT) {
            tracing::error!(
                session = %session_id,
                generation = %peer.snapshot.generation,
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
            }
            broker.worker_disconnected(session_id, peer).await;
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
                    after_runtime_seq: 0,
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
                broker.worker_disconnected(session_id, peer).await;
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
    while let Some(frame) = read_frame(reader).await? {
        if broker.controller_for(lease).is_none() {
            tracing::warn!(lease, "ignoring command from fenced controller");
            continue;
        }
        match frame {
            Frame::CoreCommand { command } => handle_core_command(&broker, command).await,
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
        } => broker.route_prompt(
            &session_id,
            WorkerCommand::Prompt {
                command_id,
                turn_id,
                content,
                cmid,
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
            let cleanup_gate = broker.deleted_session_cleanup_gate(&session_id);
            let _cleanup_guard = cleanup_gate.lock().await;
            let cleanup_command_id = command_id.clone();
            let cleanup_session = broker.sessions.lock().get(&session_id).cloned();
            let cleanup_cwd = cleanup_session.as_ref().map(|session| session.cwd.clone());
            if let Some(session) = cleanup_session {
                broker.revoke_cache_protection(&session, &session_id, "session_deleted");
            }
            broker.resetting_sessions.lock().remove(&session_id);
            let mut cancelled = broker.cancelled_sessions.lock();
            cancelled.insert(session_id.clone());
            broker.clear_generation_failures_for_session(&session_id);
            broker.sessions.lock().remove(&session_id);
            broker.awaiting_reconnect.lock().remove(&session_id);
            broker.session_states.lock().remove(&session_id);
            broker.pending_commands.lock().remove(&session_id);
            broker.unpin_fallback(&session_id);
            drop(cancelled);
            if let Some(cwd) = cleanup_cwd {
                broker.deleted_session_workspaces.lock().insert(
                    session_id.clone(),
                    DeletedWorkspace {
                        cwd,
                        command_id: cleanup_command_id.clone(),
                    },
                );
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
            Frame::Snapshot { worker } if worker.session_id == session_id => {
                broker.update_snapshot((*worker).clone(), connection_id);
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
            } if event_session == session_id && worker_epoch == epoch => {
                let cancelled = broker.cancelled_sessions.lock();
                if cancelled.contains(session_id) {
                    continue;
                }
                let rehabilitated =
                    broker.update_from_event(session_id, connection_id, runtime_seq, &event);
                broker.send_controller(Frame::WorkerEvent {
                    session_id: event_session,
                    worker_epoch,
                    runtime_seq,
                    event,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_provider_store() -> Arc<crate::machine_providers::MachineProviderStore> {
        static STORE: std::sync::OnceLock<Arc<crate::machine_providers::MachineProviderStore>> =
            std::sync::OnceLock::new();
        Arc::clone(STORE.get_or_init(|| {
            Arc::new(
                crate::machine_providers::MachineProviderStore::new(
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
            command_id: "prompt-during-drain".to_owned(),
            turn_id: "turn-2".to_owned(),
            content: vec![serde_json::json!({"type": "text", "text": "next"})],
            cmid: None,
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

    #[test]
    fn reconnecting_worker_rebuilds_broker_launch_state() {
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
                current_turn_id: Some("turn-1".to_owned()),
                last_runtime_seq: 10,
                pending_permissions: Vec::new(),
                config_options: None,
                context_used: None,
                context_size: None,
                pending_prompt_count: 0,
                drain_requested: false,
            },
            1,
        );
        assert_eq!(broker.sessions.lock().get("sess-1"), Some(&launch));
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

        assert_eq!(
            broker.update_from_event(
                "sess-ready",
                1,
                1,
                &RuntimeEvent::Ready {
                    agent_session_id: Some("agent-ready".to_owned()),
                },
            ),
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
                turn_id: "turn-1".to_owned(),
                content: vec![serde_json::json!({"type": "text", "text": "next"})],
                cmid: None,
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
            })
            .await;

        let sessions = broker.sessions.lock();
        let adopted = sessions.get("sess-adopt").expect("registry entry");
        assert!(!adopted.adopt_only, "launch specs must be normalized");
        assert!(broker.workers.lock().is_empty());
        assert!(broker.launching.lock().is_empty());
    }

    #[tokio::test]
    async fn explicit_reset_revokes_delete_tombstone_before_relaunch() {
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
                cwd: "/work".to_owned(),
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
                cwd: workspace.display().to_string(),
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
            .worker_disconnected("sess-reset-race".to_owned(), old_peer)
            .await;

        assert!(broker.deleted_session_workspaces.lock().is_empty());
        assert!(target.is_dir());
        assert!(broker.sessions.lock().contains_key("sess-reset-race"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cancelled_worker_reconnect_is_rejected_and_stopped() {
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
        assert!(matches!(
            worker_rx.try_recv(),
            Ok(Frame::WorkerCommand {
                command: WorkerCommand::Stop { .. },
                ..
            })
        ));
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
            if !target.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert!(broker.deleted_session_workspaces.lock().is_empty());
        assert!(!target.exists());
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
            .worker_disconnected("sess-connected-delete".to_owned(), peer)
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
                    current_turn_id: Some("turn-1".to_owned()),
                    last_runtime_seq: 1,
                    pending_permissions: Vec::new(),
                    config_options: None,
                    context_used: None,
                    context_size: None,
                    pending_prompt_count: 0,
                    drain_requested: false,
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
                after_runtime_seq: 0,
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
