use super::{
    Ordering, RuntimeEvent, Shared, Status, WorkerSnapshot, WorkerState,
    acknowledge_pending_ensure_from_snapshot, fail_config_startup, idle_snapshot_guard,
    notice_recovery, reconcile_idle_guard, recoverable_live_worker_error, resend_declaration,
    sync_config_for_worker, update_declaration, worker_status,
};

/// The session accepts runtime projection and `worker` belongs to its current
/// context (`Hub::admit_runtime_epoch`): Clear fences the epochs it discarded.
pub(super) fn admits_worker(shared: &Shared, worker: &WorkerSnapshot) -> bool {
    shared.hub.accepts_runtime_projection(&worker.session_id) && current_context(shared, worker)
}

/// `worker` belongs to the session's current context; one of a cleared context
/// owns nothing, not even the worker cache entry.
pub(super) fn current_context(shared: &Shared, worker: &WorkerSnapshot) -> bool {
    admit_epoch(
        shared,
        &worker.session_id,
        &worker.worker_epoch,
        worker.agent_session_id.as_deref(),
    )
}

/// An epoch reclassified as cleared also loses the worker cache entry it took
/// while unidentified; a newer owner's entry stays. A merely fenced epoch keeps
/// its entry: a live Clear still observes the worker it is replacing.
fn admit_epoch(shared: &Shared, session_id: &str, epoch: &str, native: Option<&str>) -> bool {
    let class = shared.hub.classify_runtime_epoch(session_id, epoch, native);
    if class == crate::core::RuntimeEpoch::Reclassified {
        let mut workers = shared.workers.lock();
        if workers
            .get(session_id)
            .is_some_and(|cached| cached.worker_epoch == epoch)
        {
            workers.remove(session_id);
        }
    }
    if class != crate::core::RuntimeEpoch::Current {
        clear_draining_crashed_epoch_if_matches(shared, session_id, epoch);
    }
    class == crate::core::RuntimeEpoch::Current
}

/// [`admits_worker`] for one streamed event of `epoch`.
pub(super) fn admits_event(
    shared: &Shared,
    session_id: &str,
    epoch: &str,
    event: &RuntimeEvent,
) -> bool {
    let native = match event {
        RuntimeEvent::AgentSessionId {
            agent_session_id, ..
        }
        | RuntimeEvent::Ready {
            agent_session_id: Some(agent_session_id),
        } => Some(agent_session_id.as_str()),
        _ => None,
    };
    !is_retired_crashed_epoch(shared, session_id, epoch)
        && shared.hub.accepts_runtime_projection(session_id)
        && admit_epoch(shared, session_id, epoch, native)
}

#[allow(clippy::too_many_lines)]
pub(super) fn apply_snapshot(shared: &Shared, worker: &WorkerSnapshot) -> bool {
    if !admits_worker(shared, worker) {
        return false;
    }
    // Settle the release first: the acceptance fence below refuses a bound
    // session's owner whose release differs from the recorded one, which is
    // exactly the divergence this reconciliation repairs.
    shared.hub.reconcile_provider_release(worker);
    if !shared.hub.accept_runtime_snapshot(worker) {
        return false;
    }
    shared.optimistic_revivals.lock().remove(&worker.session_id);
    shared
        .hub
        .set_current_turn_support(&worker.session_id, worker.supports_current_turn);
    shared
        .hub
        .set_wrap_up_steering(&worker.session_id, worker.wrap_up_steering);
    shared
        .hub
        .set_machine_lineage(&worker.session_id, worker.incarnation.as_deref());
    if matches!(worker.state, WorkerState::Crashed | WorkerState::Exited) {
        fail_config_startup(
            shared,
            &worker.session_id,
            "worker exited before configuration completed",
        );
    }
    let idle_guard = idle_snapshot_guard(shared, worker);
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
                compacted_at_ms: None,
            },
        );
    }
    let busy = worker.current_turn_id.is_some()
        && matches!(worker.state, WorkerState::Running | WorkerState::Draining);
    let recoverable_detail = (!busy)
        .then(|| shared.hub.latest_crash_detail(&worker.session_id))
        .flatten()
        .filter(|detail| {
            // TurnEnded clears current_turn_id before the trailing idle edge.
            // A Busy snapshot in that gap must not release a refusal hold.
            (detail == crate::core::MODEL_REFUSAL_DETAIL
                && worker.state == WorkerState::Busy
                && worker.current_turn_id.is_none())
                || recoverable_live_worker_error(
                    &shared.hub,
                    &worker.session_id,
                    worker.state,
                    Some(detail.as_str()),
                )
        });
    let preserves_refusal =
        recoverable_detail.as_deref() == Some(crate::core::MODEL_REFUSAL_DETAIL);
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
    let detail = recoverable_detail.or_else(|| {
        (worker.state == WorkerState::Crashed
            || (worker.state == WorkerState::Exited
                && worker.exit_detail.as_deref() == Some("hibernated")))
        .then(|| worker.exit_detail.clone())
        .flatten()
    });
    // The hold is already projected. Re-emitting a terminal status would clear
    // the dispatch guard of an explicit prompt sent before this idle snapshot.
    if !preserves_refusal {
        shared
            .hub
            .project_runtime_status(&worker.session_id, status, detail);
    }
    // A reconnect restores the live level; an exited worker has none left.
    let background_tasks = if matches!(worker.state, WorkerState::Exited | WorkerState::Crashed) {
        Some(0)
    } else {
        worker.background_tasks
    };
    if let Some(count) = background_tasks {
        shared.hub.set_background_tasks(&worker.session_id, count);
    }
    reconcile_idle_guard(shared, &worker.session_id, idle_guard);
    shared.hub.reconcile_session_attention(
        &worker.session_id,
        if matches!(worker.state, WorkerState::Exited | WorkerState::Crashed) {
            &[]
        } else {
            &worker.pending_permissions
        },
    );
    true
}

/// What one runtime says about a crashed session's worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrashedWorkerObservation {
    /// The Machine is not connected: nothing is known.
    Unavailable,
    /// The connected Machine reports no crashed worker for the session.
    Absent,
    /// The connected Machine last reported this crashed worker.
    Seen(CrashedWorkerSighting),
}

/// The last worker this Controller saw for a session it recorded crashed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashedWorkerSighting {
    pub worker_epoch: String,
    pub generation: String,
    /// Whether a Machine rollout or credential roll has asked for drain.
    pub drain_requested: bool,
    /// True when the latest snapshot/event evidence shows no turn, native
    /// cycle, permission request, background task, queued prompt, or
    /// unacknowledged uncertain effect.
    pub idle: bool,
}

/// Authoritative per-session raw connected-worker activity observed directly
/// from Machine frames (even when named-account admission denies Hub
/// projection). Tracking every connected state lets a streamed `Crashed`
/// transition from an unprojected `Running` worker enter the sweep, and stops
/// a newer non-crashed worker from falling back to an older projected crash.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub(super) struct SightingActivity {
    state: WorkerState,
    worker_epoch: String,
    last_runtime_seq: u64,
    generation: String,
    drain_requested: bool,
    busy_turn: bool,
    pending_permissions: std::collections::HashSet<String>,
    background_tasks: bool,
    pending_prompt_count: u64,
    native_cycle_active: bool,
    uncertain_effect: Option<crate::runtime_wire::UncertainEffect>,
    agent_session_id: Option<String>,
    launch: Option<crate::runtime_wire::StartSession>,
}

impl SightingActivity {
    pub(super) fn from_snapshot(worker: &WorkerSnapshot) -> Self {
        Self {
            state: worker.state,
            worker_epoch: worker.worker_epoch.clone(),
            last_runtime_seq: worker.last_runtime_seq,
            generation: worker.generation.clone(),
            drain_requested: worker.drain_requested,
            busy_turn: worker.current_turn_id.is_some(),
            pending_permissions: worker.pending_permissions.iter().cloned().collect(),
            background_tasks: worker.background_tasks.unwrap_or(0) > 0,
            pending_prompt_count: worker.pending_prompt_count,
            native_cycle_active: worker.native_cycle_active || worker.native_cycle_id.is_some(),
            uncertain_effect: worker.uncertain_effect.clone(),
            agent_session_id: worker.agent_session_id.clone(),
            launch: worker.launch.clone(),
        }
    }

    /// Merge a same-epoch delayed snapshot (`worker.last_runtime_seq <
    /// self.last_runtime_seq`) without regressing newer lifecycle state while
    /// still applying broker-owned drain/queue/uncertainty metadata (R12-M4).
    fn merge_snapshot(&mut self, worker: &WorkerSnapshot) {
        if self.worker_epoch == worker.worker_epoch
            && worker.last_runtime_seq < self.last_runtime_seq
        {
            self.drain_requested |= worker.drain_requested;
            self.pending_prompt_count = worker.pending_prompt_count;
            if let Some(effect) = worker.uncertain_effect.clone() {
                crate::runtime_wire::merge_uncertain_effect(&mut self.uncertain_effect, effect);
            }
            return;
        }
        *self = Self::from_snapshot(worker);
    }

    fn to_sighting(&self, hub: &crate::core::Hub, session_id: &str) -> CrashedWorkerSighting {
        let required_uncertain = self
            .uncertain_effect
            .as_ref()
            .map_or(0, |effect| effect.sequence.max(1));
        let unacked_uncertain = required_uncertain > 0
            && super::config_sync::acknowledged_uncertain_sequence_for(hub, session_id)
                < required_uncertain;
        let idle = !self.busy_turn
            && self.pending_permissions.is_empty()
            && !self.background_tasks
            && self.pending_prompt_count == 0
            && !self.native_cycle_active
            && !unacked_uncertain;
        CrashedWorkerSighting {
            worker_epoch: self.worker_epoch.clone(),
            generation: self.generation.clone(),
            drain_requested: self.drain_requested,
            idle,
        }
    }
}

const RETIRED_EPOCH_TTL: std::time::Duration = std::time::Duration::from_secs(600);
const MAX_RETIRED_EPOCHS: usize = 4096;

fn is_retired_crashed_epoch(shared: &Shared, session_id: &str, worker_epoch: &str) -> bool {
    let mut retired = shared.retired_crashed_epochs.lock();
    let now = std::time::Instant::now();
    retired.retain(|_, at| now.saturating_duration_since(*at) < RETIRED_EPOCH_TTL);
    retired.contains_key(&(session_id.to_owned(), worker_epoch.to_owned()))
}

fn record_retired_crashed_epoch(shared: &Shared, session_id: &str, worker_epoch: &str) {
    let mut retired = shared.retired_crashed_epochs.lock();
    let now = std::time::Instant::now();
    retired.retain(|_, at| now.saturating_duration_since(*at) < RETIRED_EPOCH_TTL);
    if retired.len() >= MAX_RETIRED_EPOCHS
        && let Some(oldest) = retired
            .iter()
            .min_by_key(|(_, at)| *at)
            .map(|(k, _)| k.clone())
    {
        retired.remove(&oldest);
    }
    retired.insert((session_id.to_owned(), worker_epoch.to_owned()), now);
}

/// Verified draining crashed-worker epoch, optionally correlated with the
/// accepted `DrainCrashedWorker` command ID (`None` when learned from broker
/// `Welcome`/`Snapshot` handover, R11-H5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DrainingCrashedEpoch {
    pub(super) worker_epoch: String,
    pub(super) command_id: Option<String>,
    pub(super) execution_binding: Option<crate::execution_environment::ExecutionBinding>,
}

pub(super) fn clear_draining_crashed_epoch_if_matches(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
) {
    let mut draining = shared.draining_crashed_epochs.lock();
    if draining
        .get(session_id)
        .is_some_and(|entry| entry.worker_epoch == worker_epoch)
    {
        draining.remove(session_id);
    }
    if let Some(worker) = shared
        .workers
        .lock()
        .get_mut(session_id)
        .filter(|w| w.worker_epoch == worker_epoch)
    {
        worker.crashed_drain_active = false;
    }
}

/// Handle broker confirmation/refusal of `CoreCommand::DrainCrashedWorker`
/// so replay bypass is granted ONLY after broker confirmation, and refusing a
/// duplicate/secondary drain command never revokes an already-active drain
/// (R8-M6, R11-H5).
pub(super) fn handle_crashed_drain_command_ack(
    shared: &Shared,
    session_id: &str,
    command: Option<&super::CoreCommand>,
    accepted: bool,
    reason: Option<&str>,
) {
    let Some(super::CoreCommand::DrainCrashedWorker {
        command_id,
        worker_epoch,
        ..
    }) = command
    else {
        return;
    };
    if !accepted {
        if reason.is_some_and(|r| r.contains("already_in_progress")) {
            return;
        }
        let matches_active_command = shared
            .draining_crashed_epochs
            .lock()
            .get(session_id)
            .is_some_and(|entry| {
                entry.worker_epoch == *worker_epoch
                    && entry.command_id.as_deref() == Some(command_id.as_str())
            });
        if matches_active_command {
            clear_draining_crashed_epoch_if_matches(shared, session_id, worker_epoch);
        }
        return;
    }
    let fence = shared.projection_fence.lock();
    if *fence {
        return;
    }
    if let Some(mut worker) = shared
        .workers
        .lock()
        .get(session_id)
        .filter(|w| w.worker_epoch == *worker_epoch)
        .cloned()
    {
        worker.crashed_drain_active = true;
        let _ = authorize_draining_crashed_epoch_locked(shared, &worker, Some(command_id));
    }
}

pub(super) fn seed_crashed_sightings(
    workers: &[WorkerSnapshot],
) -> std::collections::HashMap<String, SightingActivity> {
    workers
        .iter()
        .filter(|worker| worker.has_connected_owner())
        .map(|worker| {
            (
                worker.session_id.clone(),
                SightingActivity::from_snapshot(worker),
            )
        })
        .collect()
}

pub(super) fn seed_draining_crashed_epochs(
    workers: &[WorkerSnapshot],
) -> std::collections::HashMap<String, DrainingCrashedEpoch> {
    workers
        .iter()
        .filter(|worker| worker.has_connected_owner() && worker.crashed_drain_active)
        .map(|worker| {
            (
                worker.session_id.clone(),
                DrainingCrashedEpoch {
                    worker_epoch: worker.worker_epoch.clone(),
                    command_id: None,
                    execution_binding: worker
                        .launch
                        .as_ref()
                        .and_then(|launch| launch.execution_binding.clone()),
                },
            )
        })
        .collect()
}

/// Validate Clear fence (including native ID) and launch binding under Hub's
/// projection guard before authorizing a crashed worker epoch for draining
/// replay bypass (R7-H2, R7-H3, R8-M6, R11-H2: bound strictly to broker-stamped
/// `crashed_drain_active`, independent of replayed `Running`/`TurnStarted` state).
fn authorize_draining_crashed_epoch_locked(
    shared: &Shared,
    worker: &WorkerSnapshot,
    command_id: Option<&str>,
) -> bool {
    if !worker.has_connected_owner()
        || !worker.crashed_drain_active
        || is_retired_crashed_epoch(shared, &worker.session_id, &worker.worker_epoch)
    {
        return false;
    }
    let _proj = shared.hub.runtime_projection_guard();
    if !admit_epoch(
        shared,
        &worker.session_id,
        &worker.worker_epoch,
        worker.agent_session_id.as_deref(),
    ) || !shared.hub.matches_runtime_snapshot_binding(worker)
    {
        clear_draining_crashed_epoch_if_matches(shared, &worker.session_id, &worker.worker_epoch);
        return false;
    }
    {
        let mut draining = shared.draining_crashed_epochs.lock();
        let preserved_cmd = draining
            .get(&worker.session_id)
            .filter(|existing| existing.worker_epoch == worker.worker_epoch)
            .and_then(|existing| existing.command_id.clone());
        draining.insert(
            worker.session_id.clone(),
            DrainingCrashedEpoch {
                worker_epoch: worker.worker_epoch.clone(),
                command_id: command_id.map(str::to_owned).or(preserved_cmd),
                execution_binding: worker
                    .launch
                    .as_ref()
                    .and_then(|launch| launch.execution_binding.clone()),
            },
        );
    }
    true
}

/// Capture epoch-bound draining crashed worker evidence from Welcome before
/// `resetting` filters normal snapshot projection during bootstrap reload.
/// Validates Clear fence (including native ID) and launch binding under Hub's
/// projection guard and caches the verified snapshot in
/// `draining_replay_snapshots` so `native_admission::event` admits trailing
/// replay even when `shared.workers` holds an ownerless placeholder (R7-H2, R7-H3).
fn revoke_ownerless_draining_state(shared: &Shared, session_id: &str) {
    shared.crashed_worker_sightings.lock().remove(session_id);
    shared.draining_crashed_epochs.lock().remove(session_id);
    if let Some(worker) = shared.workers.lock().get_mut(session_id) {
        worker.crashed_drain_active = false;
    }
}

pub(super) fn record_welcome_draining_crashed_epoch(shared: &Shared, worker: &WorkerSnapshot) {
    let fence = shared.projection_fence.lock();
    if *fence {
        return;
    }
    if !worker.has_connected_owner() {
        revoke_ownerless_draining_state(shared, &worker.session_id);
        return;
    }
    if !worker.crashed_drain_active {
        clear_draining_crashed_epoch_if_matches(shared, &worker.session_id, &worker.worker_epoch);
        return;
    }
    if !authorize_draining_crashed_epoch_locked(shared, worker, None) {
        return;
    }
    let mut sightings = shared.crashed_worker_sightings.lock();
    if let Some(existing) = sightings.get_mut(&worker.session_id) {
        existing.merge_snapshot(worker);
    } else {
        sightings.insert(
            worker.session_id.clone(),
            SightingActivity::from_snapshot(worker),
        );
    }
}

/// Observe raw connected-worker state. Rejects late snapshots for already-retired
/// crashed epochs (R3-M7), records replacement epochs during reset (R2-M6), and
/// authorizes draining replay ONLY after verifying Clear fence + launch binding
/// under Hub's projection guard (R7-H2).
pub(super) fn observe_frame_snapshot(shared: &Shared, worker: &WorkerSnapshot) {
    if worker.crashed_drain_active && shared.resetting.lock().contains(&worker.session_id) {
        record_welcome_draining_crashed_epoch(shared, worker);
    }
    observe_crashed_sighting(shared, worker);
}

pub(super) fn observe_crashed_sighting(shared: &Shared, worker: &WorkerSnapshot) {
    let fence = shared.projection_fence.lock();
    if *fence {
        return;
    }
    if is_retired_crashed_epoch(shared, &worker.session_id, &worker.worker_epoch) {
        return;
    }
    if !worker.has_connected_owner() {
        // Authoritative ownerless inventory clears any departing real epoch's
        // drain authorization and sighting even if reset started from a
        // `broker-` placeholder (R14-M7).
        revoke_ownerless_draining_state(shared, &worker.session_id);
        return;
    }
    if shared.resetting.lock().contains(&worker.session_id) {
        let is_replaced_epoch = shared
            .reset_replacements
            .lock()
            .get(&worker.session_id)
            .is_none_or(|(replaced, _)| replaced == &worker.worker_epoch);
        if is_replaced_epoch {
            // Apply authoritative broker drain revocation for this exact epoch
            // before filtering reset lifecycle projection so a failed/withdrawn
            // reset never retains a stale drain marker (R13-M4).
            if !worker.crashed_drain_active {
                clear_draining_crashed_epoch_if_matches(
                    shared,
                    &worker.session_id,
                    &worker.worker_epoch,
                );
            }
            return;
        }
    }
    if worker.has_connected_owner() {
        // Never erase retired epoch tombstones on successor snapshots (R4-M5);
        // clear stale draining markers for replaced epochs and verify binding
        // + Clear fence before authorizing draining replay (R7-H2).
        {
            let mut draining = shared.draining_crashed_epochs.lock();
            if draining
                .get(&worker.session_id)
                .is_some_and(|e| e.worker_epoch != worker.worker_epoch)
            {
                draining.remove(&worker.session_id);
            }
        }
        if worker.crashed_drain_active {
            let _ = authorize_draining_crashed_epoch_locked(shared, worker, None);
        } else {
            clear_draining_crashed_epoch_if_matches(
                shared,
                &worker.session_id,
                &worker.worker_epoch,
            );
        }
        {
            let mut sightings = shared.crashed_worker_sightings.lock();
            if let Some(existing) = sightings.get_mut(&worker.session_id) {
                existing.merge_snapshot(worker);
            } else {
                sightings.insert(
                    worker.session_id.clone(),
                    SightingActivity::from_snapshot(worker),
                );
            }
        }
    } else {
        shared
            .crashed_worker_sightings
            .lock()
            .remove(&worker.session_id);
        shared
            .draining_crashed_epochs
            .lock()
            .remove(&worker.session_id);
    }
}

/// True ONLY when `worker_epoch` is a verified draining crashed epoch
/// (Clear-fence and launch-binding checked, R7-H2). Raw maintenance sightings
/// never authorize replay bypass on their own.
fn is_cleanup_only_execution_command(command: &crate::execution_protocol::Command) -> bool {
    use crate::execution_protocol::Command;
    match command {
        Command::Invoke { invocation, .. } => matches!(
            invocation.method.as_str(),
            "process/terminate" | "process/read" | crate::worker_execution::USER_MCP_STOP_METHOD
        ),
        Command::Observe { .. } | Command::Events { .. } => true,
        Command::Describe | Command::Shutdown => false,
    }
}

/// Verify execution-request ownership. Normally requires a live non-terminal
/// worker; additionally permits a strict cleanup-only allowlist (`process/terminate`,
/// `process/read`, `cowboy/stopUserMcp`, `Observe`, `Events`) for an exact
/// broker-fenced draining crashed epoch and execution binding so cooperative
/// drain settlement completes cleanly (R9-M1).
pub(super) fn execution_owner_matches(
    shared: &Shared,
    request: &crate::execution_protocol::RuntimeRequest,
) -> bool {
    let matches_binding = |worker: &WorkerSnapshot| {
        worker.worker_epoch == request.worker_epoch
            && worker
                .launch
                .as_ref()
                .and_then(|launch| launch.execution_binding.as_ref())
                .and_then(|binding| binding.decode().ok())
                .is_some_and(|binding| binding == request.binding)
    };
    // Evaluate verified draining crashed epoch FIRST so replayed Running/TurnStarted
    // events can never escalate a draining epoch into the unrestricted live-worker
    // path, and never trust stale cached `crashed_drain_active` after revocation (R10-H5).
    if let Some(entry) = shared
        .draining_crashed_epochs
        .lock()
        .get(&request.session_id)
        .filter(|entry| entry.worker_epoch == request.worker_epoch)
        .cloned()
    {
        if !is_cleanup_only_execution_command(&request.command) {
            return false;
        }
        if let Some(binding) = entry
            .execution_binding
            .as_ref()
            .and_then(|binding| binding.decode().ok())
        {
            return binding == request.binding;
        }
        return shared
            .workers
            .lock()
            .get(&request.session_id)
            .is_some_and(matches_binding);
    }
    shared
        .workers
        .lock()
        .get(&request.session_id)
        .is_some_and(|worker| {
            matches_binding(worker)
                && !matches!(worker.state, WorkerState::Exited | WorkerState::Crashed)
        })
}

pub(super) fn is_draining_crashed_epoch(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
) -> bool {
    shared
        .draining_crashed_epochs
        .lock()
        .get(session_id)
        .is_some_and(|entry| entry.worker_epoch == worker_epoch)
}

pub(super) fn observe_crashed_event(
    shared: &Shared,
    key: &(String, String),
    runtime_seq: u64,
    event: &RuntimeEvent,
) {
    let (session_id, worker_epoch) = (key.0.as_str(), key.1.as_str());
    let fence = shared.projection_fence.lock();
    if *fence || is_retired_crashed_epoch(shared, session_id, worker_epoch) {
        return;
    }
    if shared.resetting.lock().contains(session_id) {
        let is_replaced_epoch = shared
            .reset_replacements
            .lock()
            .get(session_id)
            .is_none_or(|(replaced, _)| replaced == worker_epoch);
        if is_replaced_epoch && !is_draining_crashed_epoch(shared, session_id, worker_epoch) {
            return;
        }
    }
    let mut sightings = shared.crashed_worker_sightings.lock();
    if sightings
        .get(session_id)
        .is_none_or(|entry| entry.worker_epoch != worker_epoch)
    {
        if let Some(worker) = shared.workers.lock().get(session_id).cloned()
            && worker.worker_epoch == worker_epoch
            && worker.has_connected_owner()
        {
            sightings.insert(
                session_id.to_owned(),
                SightingActivity::from_snapshot(&worker),
            );
        } else {
            return;
        }
    }
    let worker_watermark = shared
        .workers
        .lock()
        .get(session_id)
        .filter(|w| w.has_connected_owner() && w.worker_epoch == worker_epoch)
        .map_or(0, |w| w.last_runtime_seq);
    let Some(entry) = sightings.get_mut(session_id) else {
        return;
    };
    let effective_watermark = entry.last_runtime_seq.max(worker_watermark);
    entry.last_runtime_seq = effective_watermark;
    if (entry.state == WorkerState::Crashed
        || is_draining_crashed_epoch(shared, session_id, worker_epoch))
        && runtime_seq > 0
        && runtime_seq < effective_watermark
    {
        // Never roll back sighting lifecycle state on historical replay below
        // the unified monotonic epoch watermark; keep monotonic uncertainty
        // (R13-M2, R17-M5).
        if let RuntimeEvent::Update { update, .. } = event
            && let Some(effect) = crate::runtime_wire::uncertain_effect(update)
        {
            crate::runtime_wire::merge_uncertain_effect(&mut entry.uncertain_effect, effect);
        }
        return;
    }
    entry.last_runtime_seq = effective_watermark.max(runtime_seq);
    match event {
        RuntimeEvent::Ready { .. } => entry.state = WorkerState::Running,
        RuntimeEvent::TurnStarted { .. } => {
            entry.state = WorkerState::Busy;
            entry.busy_turn = true;
        }
        RuntimeEvent::TurnEnded { .. } => entry.busy_turn = false,
        RuntimeEvent::PermissionRequest { request_id, .. } => {
            entry.pending_permissions.insert(request_id.clone());
        }
        RuntimeEvent::PermissionResolved { request_id, .. } => {
            entry.pending_permissions.remove(request_id);
        }
        RuntimeEvent::Status { state, .. } => {
            entry.state = *state;
            if *state == WorkerState::Exited {
                entry.native_cycle_active = false;
            }
        }
        RuntimeEvent::Update { update, .. } => {
            if let Some(count) = crate::runtime_wire::background_tasks_count(update) {
                entry.background_tasks = count > 0;
            }
            if let Some(cycle) = crate::runtime_wire::native_cycle(update) {
                entry.native_cycle_active = cycle.is_some();
            }
            if let Some(effect) = crate::runtime_wire::uncertain_effect(update) {
                crate::runtime_wire::merge_uncertain_effect(&mut entry.uncertain_effect, effect);
            }
        }
        _ => {}
    }
}

pub(super) fn retain_crashed_sightings_after_welcome(
    shared: &Shared,
    ids: &std::collections::HashSet<String>,
) {
    shared
        .crashed_worker_sightings
        .lock()
        .retain(|id, _| ids.contains(id));
    shared
        .draining_crashed_epochs
        .lock()
        .retain(|id, _| ids.contains(id));
}

pub(super) fn handle_crashed_worker_drained_frame(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
) {
    if super::foreign_session(shared, session_id) {
        super::reject_foreign_frame(shared, session_id, "CrashedWorkerDrained");
        return;
    }
    retire_crashed_worker_drained(shared, session_id, worker_epoch);
}

/// Synchronously and idempotently retire a crashed worker after the Machine
/// confirms cooperative exit + sandbox teardown. A draining worker never emits
/// `CrashedDrainCompleted` while holding an unacknowledged `uncertain_effect`,
/// so retirement needs no store persistence or async retry.
/// Stop-only epoch check for `CoreCommand::DrainCrashedWorker` in
/// `native_admission::send`: validates the exact epoch is not retired or
/// superseded by a replacement worker, independent of launch credentials (R20-M2).
pub(super) fn drain_crashed_epoch_admitted(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
) -> bool {
    if is_retired_crashed_epoch(shared, session_id, worker_epoch)
        || shared.resetting.lock().contains(session_id)
    {
        return false;
    }
    if shared
        .workers
        .lock()
        .get(session_id)
        .is_some_and(|worker| worker.has_connected_owner() && worker.worker_epoch != worker_epoch)
    {
        return false;
    }
    if shared
        .crashed_worker_sightings
        .lock()
        .get(session_id)
        .is_some_and(|entry| entry.worker_epoch != worker_epoch)
    {
        return false;
    }
    true
}

fn has_pending_replacement_command(shared: &Shared, session_id: &str) -> bool {
    shared.pending.lock().values().any(|command| match command {
        super::CoreCommand::EnsureSession { session } => session.session_id == session_id,
        super::CoreCommand::RecoverExecutionSession { session, .. }
        | super::CoreCommand::ReadmitAccountSession { session, .. } => {
            session.session_id == session_id
        }
        super::CoreCommand::StopSession {
            session_id: owner,
            command_id,
        } => owner == session_id && command_id.starts_with("reset-"),
        _ => false,
    })
}

fn retired_owner_matches_meta(
    shared: &Shared,
    meta: &crate::model::SessionMeta,
    cached_worker: Option<&WorkerSnapshot>,
    matched_sighting: Option<&SightingActivity>,
    matched_draining: Option<&DrainingCrashedEpoch>,
) -> bool {
    if let Some(worker) = cached_worker {
        return shared.hub.matches_runtime_snapshot_binding(worker);
    }
    if let Some(launch) = matched_sighting.and_then(|sighting| sighting.launch.as_ref()) {
        if launch.provider != meta.provider
            || launch.provider_account_id != meta.provider_account_id
            || launch.provider_auth_generation != meta.provider_auth_generation
        {
            return false;
        }
        if let Some(expected) = meta.execution_binding.as_ref()
            && launch.execution_binding.as_ref() != Some(expected)
        {
            return false;
        }
        return true;
    }
    if let Some(expected) = meta.execution_binding.as_ref() {
        return matched_draining.and_then(|entry| entry.execution_binding.as_ref())
            == Some(expected);
    }
    true
}

fn maybe_mark_retired_unprojected_crash(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
    cached_worker: Option<&WorkerSnapshot>,
    matched_sighting: Option<&SightingActivity>,
    matched_draining: Option<&DrainingCrashedEpoch>,
) {
    if shared.resetting.lock().contains(session_id)
        || shared.optimistic_revivals.lock().contains(session_id)
        || has_pending_replacement_command(shared, session_id)
    {
        return;
    }
    let owner_native_id = cached_worker
        .and_then(|worker| worker.agent_session_id.as_deref())
        .or_else(|| matched_sighting.and_then(|sighting| sighting.agent_session_id.as_deref()));
    let cached_was_starting =
        cached_worker.is_some_and(|worker| worker.state == WorkerState::Starting);
    let _proj = shared.hub.runtime_projection_guard();
    if shared.hub.accepts_runtime_projection(session_id)
        && admit_epoch(shared, session_id, worker_epoch, owner_native_id)
        && shared.hub.session_info(session_id).is_some_and(|info| {
            shared
                .machine_id
                .as_deref()
                .is_none_or(|machine| info.meta.machine_id == machine)
                && retired_owner_matches_meta(
                    shared,
                    &info.meta,
                    cached_worker,
                    matched_sighting,
                    matched_draining,
                )
                && (matches!(info.meta.status, Status::Busy | Status::Running)
                    || (cached_was_starting && info.meta.status == Status::Starting)
                    || shared.hub.session_has_in_flight_prompt(session_id))
        })
    {
        shared.hub.set_status(
            session_id,
            Status::Crashed,
            Some("crashed worker exited during drain".to_owned()),
        );
    }
}

pub(super) fn retire_crashed_worker_drained(shared: &Shared, session_id: &str, worker_epoch: &str) {
    let fence = shared.projection_fence.lock();
    if *fence {
        return;
    }
    let already_retired = is_retired_crashed_epoch(shared, session_id, worker_epoch);
    record_retired_crashed_epoch(shared, session_id, worker_epoch);
    let (matched_draining, draining_replaced) = {
        let mut draining = shared.draining_crashed_epochs.lock();
        match draining.get(session_id) {
            Some(entry) if entry.worker_epoch == worker_epoch => {
                (draining.remove(session_id), false)
            }
            Some(_) => (None, true),
            None => (None, false),
        }
    };
    let (matched_sighting, sighting_replaced) = {
        let mut sightings = shared.crashed_worker_sightings.lock();
        match sightings.get(session_id) {
            Some(entry) if entry.worker_epoch == worker_epoch => {
                (sightings.remove(session_id), false)
            }
            Some(_) => (None, true),
            None => (None, false),
        }
    };
    let (cached_worker, worker_replaced) = {
        let mut workers = shared.workers.lock();
        let replaced = workers.get(session_id).is_some_and(|worker| {
            worker.has_connected_owner() && worker.worker_epoch != worker_epoch
        });
        let removed = if workers
            .get(session_id)
            .is_some_and(|worker| worker.worker_epoch == worker_epoch)
        {
            workers.remove(session_id)
        } else {
            None
        };
        (removed, replaced)
    };
    let had_matching_owner =
        cached_worker.is_some() || matched_sighting.is_some() || matched_draining.is_some();
    let has_replacement_owner = worker_replaced || sighting_replaced || draining_replaced;
    // Retiring an unprojected crash (e.g. a named worker whose crash snapshot
    // failed account admission while Hub was still Busy) may clear the stuck
    // live status ONLY when this exact epoch is still the tracked owner,
    // matches the session's current Machine and binding, has no successor
    // sighting or replacement launch in flight, and has not been fenced by
    // Clear (R17-M4, R20-M1).
    if !already_retired && had_matching_owner && !has_replacement_owner {
        maybe_mark_retired_unprojected_crash(
            shared,
            session_id,
            worker_epoch,
            cached_worker.as_ref(),
            matched_sighting.as_ref(),
            matched_draining.as_ref(),
        );
    }
    let mut pending = shared.pending.lock();
    let sent = shared.sent.lock();
    let attempted = shared.attempted_sends.lock();
    pending.retain(|id, command| {
        !(matches!(
            command,
            super::CoreCommand::DrainCrashedWorker {
                session_id: owner,
                worker_epoch: target,
                ..
            } if owner == session_id && target == worker_epoch
        ) && !sent.contains(id)
            && !attempted.contains(id))
    });
}

pub(super) fn discard_pending_crashed_drain(
    shared: &Shared,
    session_id: &str,
    forget_session: bool,
) {
    if forget_session {
        shared.crashed_worker_sightings.lock().remove(session_id);
        shared.draining_crashed_epochs.lock().remove(session_id);
        shared
            .retired_crashed_epochs
            .lock()
            .retain(|(sid, _), _| sid != session_id);
    }
    let mut pending = shared.pending.lock();
    let sent = shared.sent.lock();
    let attempted = shared.attempted_sends.lock();
    pending.retain(|id, command| {
        !(matches!(
            command,
            super::CoreCommand::DrainCrashedWorker {
                session_id: owner,
                ..
            } if owner == session_id
        ) && !sent.contains(id)
            && !attempted.contains(id))
    });
}

impl super::RemoteRuntime {
    /// Whether this Machine and runtime protocol pair supports cooperative
    /// epoch-bound crashed-worker drain (`CoreCommand::DrainCrashedWorker`).
    #[must_use]
    pub fn supports_crashed_worker_drain(&self) -> bool {
        self.connected()
            && self.shared.machine_protocol
                >= crate::machine_protocol::CRASHED_WORKER_DRAIN_PROTOCOL_VERSION
            && self.shared.protocol.load(Ordering::Acquire)
                >= crate::runtime_wire::CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
    }

    /// Whether a prompt command is currently queued for `session_id`.
    #[must_use]
    pub fn has_pending_prompt(&self, session_id: &str) -> bool {
        self.shared.pending.lock().values().any(|command| {
            matches!(
                command,
                super::CoreCommand::Prompt {
                    session_id: pending,
                    ..
                } if pending == session_id
            )
        })
    }

    /// Whether a crashed-worker drain command is already queued for `session_id`.
    #[must_use]
    pub fn crashed_drain_pending(&self, session_id: &str) -> bool {
        self.shared.pending.lock().values().any(|command| {
            matches!(
                command,
                super::CoreCommand::DrainCrashedWorker {
                    session_id: owner,
                    ..
                } if owner == session_id
            )
        })
    }

    /// Queue an epoch-bound cooperative drain for an idle crashed worker when
    /// the Machine supports protocol 55+ / Runtime 10+. Returns `true` only
    /// when a new command was queued.
    pub fn drain_crashed_worker(&self, session_id: &str, worker_epoch: &str) -> bool {
        if !self.supports_crashed_worker_drain()
            || worker_epoch.is_empty()
            || worker_epoch.starts_with("broker-")
            || self.shared.resetting.lock().contains(session_id)
            || self.crashed_drain_pending(session_id)
            || is_draining_crashed_epoch(&self.shared, session_id, worker_epoch)
        {
            return false;
        }
        let CrashedWorkerObservation::Seen(sighting) = self.last_crashed_worker(session_id) else {
            return false;
        };
        if sighting.worker_epoch != worker_epoch || !sighting.idle {
            return false;
        }
        let acknowledged_uncertain_sequence =
            super::config_sync::acknowledged_uncertain_sequence_for(&self.shared.hub, session_id);
        let command_id = self.next_id("drain-crashed");
        self.queue(
            command_id.clone(),
            super::CoreCommand::DrainCrashedWorker {
                session_id: session_id.to_owned(),
                command_id,
                worker_epoch: worker_epoch.to_owned(),
                acknowledged_uncertain_sequence,
            },
        );
        true
    }

    /// Authoritative crashed-worker observation for `session_id` on the
    /// current connection.
    pub fn last_crashed_worker(&self, session_id: &str) -> CrashedWorkerObservation {
        let sightings = self.shared.crashed_worker_sightings.lock();
        if !self.connected() {
            return CrashedWorkerObservation::Unavailable;
        }
        sightings
            .get(session_id)
            .filter(|entry| entry.state == WorkerState::Crashed)
            .map_or(CrashedWorkerObservation::Absent, |entry| {
                CrashedWorkerObservation::Seen(entry.to_sighting(&self.shared.hub, session_id))
            })
    }
}

/// Merge a worker's published uncertain effect into its snapshot, by
/// sequence: an older observation never replaces a newer one.
pub(super) fn record_uncertain_effect(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
    event: &RuntimeEvent,
) {
    let RuntimeEvent::Update { update, .. } = event else {
        return;
    };
    let Some(effect) = crate::runtime_wire::uncertain_effect(update) else {
        return;
    };
    if is_retired_crashed_epoch(shared, session_id, worker_epoch) {
        return;
    }
    if let Some(worker) = shared.workers.lock().get_mut(session_id)
        && worker.worker_epoch == worker_epoch
    {
        crate::runtime_wire::merge_uncertain_effect(&mut worker.uncertain_effect, effect);
    }
}

pub(super) fn project_snapshot(shared: &Shared, worker: &WorkerSnapshot) -> bool {
    project_snapshot_with_protocol(shared, worker, shared.protocol.load(Ordering::Acquire))
}

pub(super) fn project_snapshot_with_protocol(
    shared: &Shared,
    worker: &WorkerSnapshot,
    runtime_protocol: u16,
) -> bool {
    let mut worker = worker.clone();
    let mut raw_wrap = worker.wrap_up_steering;
    let _projection = shared.hub.runtime_projection_guard();
    shared.wrap_up.sanitize_snapshot(
        &mut worker,
        raw_wrap,
        shared.machine_protocol,
        runtime_protocol,
        &shared.hub,
    );
    if is_retired_crashed_epoch(shared, &worker.session_id, &worker.worker_epoch) {
        return false;
    }
    if !current_context(shared, &worker) {
        return false;
    }
    // Reconcile compatible provider release transitions before enforcing the
    // bound launch tuple, matching `apply_snapshot`.
    if admits_worker(shared, &worker) {
        shared.hub.reconcile_provider_release(&worker);
    }
    if !shared.hub.matches_runtime_snapshot_binding(&worker)
        || shared.hub.is_reconciling_runtime_placeholder(&worker)
    {
        // A surviving pre-recovery worker (stale execution binding after a
        // committed recovery + Controller restart) must still be visible in
        // `shared.workers` for operator inspection (`execution-sessions` /
        // `delivery_diagnostics`) and idle-worker replacement
        // (`replaceable_worker`), unless a binding-matched worker is already
        // cached for this session (which a mismatched snapshot must never
        // overwrite). Never project it into Hub, cache `raw_wrap_up`, advertise
        // `wrap_up_steering`, acknowledge `EnsureSession`, or retire pending
        // prompts.
        if !shared.hub.is_reconciling_runtime_placeholder(&worker) {
            worker.wrap_up_steering = false;
            let mut workers = shared.workers.lock();
            let preserve_matched_worker = workers
                .get(&worker.session_id)
                .is_some_and(|existing| shared.hub.matches_runtime_snapshot_binding(existing));
            if !preserve_matched_worker {
                workers.insert(worker.session_id.clone(), worker);
            }
        }
        return false;
    }
    if worker.has_connected_owner()
        && let Some(existing) = shared.workers.lock().get(&worker.session_id).cloned()
        && existing.has_connected_owner()
        && existing.worker_epoch == worker.worker_epoch
        && worker.last_runtime_seq < existing.last_runtime_seq
    {
        // Reject older same-epoch lifecycle fields while keeping broker-owned
        // drain, queue, incarnation, and uncertainty metadata (R12-M4).
        let mut merged = existing;
        merged.drain_requested |= worker.drain_requested;
        merged.crashed_drain_active = worker.crashed_drain_active;
        merged.pending_prompt_count = worker.pending_prompt_count;
        if worker.incarnation.is_some() {
            merged.incarnation.clone_from(&worker.incarnation);
        }
        if let Some(effect) = worker.uncertain_effect.clone() {
            crate::runtime_wire::merge_uncertain_effect(&mut merged.uncertain_effect, effect);
        }
        // Preserve newer delivery capabilities and cached raw wrap-up state
        // from `existing`, then recompute protocol/provider policy (R13-L1).
        let preserved_raw = shared
            .wrap_up
            .cached_raw(&merged.session_id, &merged.worker_epoch)
            .unwrap_or(raw_wrap);
        shared.wrap_up.sanitize_snapshot(
            &mut merged,
            preserved_raw,
            shared.machine_protocol,
            runtime_protocol,
            &shared.hub,
        );
        raw_wrap = preserved_raw;
        worker = merged;
    }
    let applied = apply_snapshot(shared, &worker);
    let repaired = update_declaration(shared, &worker);
    shared.install_worker_with_raw_wrap_up(worker.clone(), raw_wrap);
    if !shared.resetting.lock().contains(&worker.session_id) {
        let mut sightings = shared.crashed_worker_sightings.lock();
        if worker.has_connected_owner() {
            if let Some(existing) = sightings.get_mut(&worker.session_id) {
                existing.merge_snapshot(&worker);
            } else {
                sightings.insert(
                    worker.session_id.clone(),
                    SightingActivity::from_snapshot(&worker),
                );
            }
        } else {
            sightings.remove(&worker.session_id);
        }
    }
    if applied {
        acknowledge_pending_ensure_from_snapshot(shared, &worker);
        notice_recovery::spawn_record_snapshot_worker_epoch_if_notice(shared, &worker);
    }
    if let Some(session) = repaired {
        resend_declaration(shared, session, &worker.worker_epoch);
    }
    applied
}

pub(super) fn codex_full_access_permission(
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
    let launch = worker.launch.as_ref()?;
    let permission = launch.provider_behavior.as_ref().map_or_else(
        || crate::provider::legacy_behavior(&launch.provider).permission,
        |behavior| behavior.permission.clone(),
    );
    if permission != cowboy_provider_sdk::PermissionBehavior::AcpConfigFullAccessV1
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

pub(super) fn codex_config_is_full_access(options: &serde_json::Value) -> bool {
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

pub(super) fn queue_permission(
    shared: &Shared,
    session_id: &str,
    request_id: String,
    option_id: String,
) {
    let value = shared.command_counter.fetch_add(1, Ordering::Relaxed);
    let command_id = format!("permission-{}-{value}", std::process::id());
    shared.sent.lock().remove(&command_id);
    shared.pending.lock().insert(
        command_id.clone(),
        super::CoreCommand::Permission {
            session_id: session_id.to_owned(),
            command_id,
            request_id,
            option_id: Some(option_id),
        },
    );
    let _ = shared.notify.send(());
}

pub(super) fn begin_reset(shared: &Shared, session_id: &str) {
    let replaced = shared
        .workers
        .lock()
        .get(session_id)
        .map(|worker| worker.worker_epoch.clone())
        .unwrap_or_default();
    let placeholder = format!(
        "pending-reset:{}",
        shared.command_counter.fetch_add(1, Ordering::Relaxed) + 1
    );
    shared.resetting.lock().insert(session_id.to_owned());
    shared
        .active_reset_commands
        .lock()
        .insert(session_id.to_owned(), placeholder);
    shared
        .reset_replacements
        .lock()
        .insert(session_id.to_owned(), (replaced, None));
}

#[cfg(test)]
mod crashed_drain_tests {
    use super::*;
    use crate::runtime_wire::{Frame, StartSession};

    fn crashed_snapshot(session_id: &str, epoch: &str) -> WorkerSnapshot {
        WorkerSnapshot {
            session_id: session_id.to_owned(),
            worker_epoch: epoch.to_owned(),
            generation: "gen-1".to_owned(),
            executable: Some("/bin/false".to_owned()),
            launch: Some(StartSession {
                session_id: session_id.to_owned(),
                provider: "codex".to_owned(),
                provider_version: "1.0.0".to_owned(),
                provider_generation_digest: "sha256:test".to_owned(),
                provider_account_id: crate::provider_account::ProviderAccountId::default(),
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
                extensions: None,
                mcp_plan: None,
                native_fork: None,
            }),
            state: WorkerState::Crashed,
            agent_session_id: None,
            native_thread_materialized: None,
            current_turn_id: None,
            supports_current_turn: false,
            delivery_controls: crate::runtime_wire::DeliveryControls::default(),
            wrap_up_steering: false,
            supports_subagent_cancel: false,
            last_runtime_seq: 10,
            pending_permissions: Vec::new(),
            config_options: None,
            context_used: None,
            context_size: None,
            pending_prompt_count: 0,
            drain_requested: false,
            exit_detail: Some("acp crashed".to_owned()),
            background_tasks: None,
            native_cycle_id: None,
            native_cycle_active: false,
            incarnation: None,
            account_unit: None,
            uncertain_effect: None,
            crashed_drain_active: false,
        }
    }

    #[tokio::test]
    async fn crashed_sighting_gates_on_activity_native_cycle_and_unacknowledged_uncertain_effect() {
        let hub = crate::core::Hub::new();
        hub.create_local_session(
            "s-1".into(),
            "codex".into(),
            "/tmp".into(),
            "test".into(),
            crate::model::SessionOrigin::Web,
            false,
        );
        let rt = super::super::RemoteRuntime::for_test(hub, Vec::new());
        rt.connect_for_test();

        let mut snap = crashed_snapshot("s-1", "100-1");
        snap.native_cycle_active = true;
        observe_crashed_sighting(&rt.shared, &snap);
        assert!(matches!(
            rt.last_crashed_worker("s-1"),
            CrashedWorkerObservation::Seen(s) if !s.idle
        ));

        snap.native_cycle_active = false;
        snap.uncertain_effect = Some(crate::runtime_wire::UncertainEffect {
            method: "user_mcp".into(),
            sequence: 2,
        });
        observe_crashed_sighting(&rt.shared, &snap);
        assert!(matches!(
            rt.last_crashed_worker("s-1"),
            CrashedWorkerObservation::Seen(s) if !s.idle
        ));

        snap.uncertain_effect = None;
        observe_crashed_sighting(&rt.shared, &snap);
        assert!(matches!(
            rt.last_crashed_worker("s-1"),
            CrashedWorkerObservation::Seen(s) if s.idle && s.worker_epoch == "100-1"
        ));
    }

    #[tokio::test]
    async fn retired_crashed_epoch_ignores_late_snapshots_even_after_successor_arrives() {
        let hub = crate::core::Hub::new();
        let snap = crashed_snapshot("s-r3", "101-1");
        let rt = super::super::RemoteRuntime::for_test(hub, vec![snap.clone()]);
        rt.connect_for_test();

        retire_crashed_worker_drained(&rt.shared, "s-r3", "101-1");
        assert!(matches!(
            rt.last_crashed_worker("s-r3"),
            CrashedWorkerObservation::Absent
        ));
        observe_crashed_sighting(&rt.shared, &snap);
        assert!(matches!(
            rt.last_crashed_worker("s-r3"),
            CrashedWorkerObservation::Absent
        ));
        assert!(!project_snapshot(&rt.shared, &snap));

        let newer = crashed_snapshot("s-r3", "101-2");
        observe_crashed_sighting(&rt.shared, &newer);
        assert!(matches!(
            rt.last_crashed_worker("s-r3"),
            CrashedWorkerObservation::Seen(s) if s.worker_epoch == "101-2"
        ));
        observe_crashed_sighting(&rt.shared, &snap);
        assert!(!project_snapshot(&rt.shared, &snap));
        assert!(matches!(
            rt.last_crashed_worker("s-r3"),
            CrashedWorkerObservation::Seen(s) if s.worker_epoch == "101-2"
        ));
    }

    #[tokio::test]
    async fn execution_owner_matches_enforces_cleanup_only_on_draining_epoch_and_ignores_unverified_flag()
     {
        let hub = crate::core::Hub::new();
        let rt = super::super::RemoteRuntime::for_test(hub, Vec::new());
        let binding = crate::execution_environment::BindingV1 {
            schema: 1,
            id: "binding-r10".into(),
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
        let make_req = |command| crate::execution_protocol::RuntimeRequest {
            request_id: "req-1".into(),
            session_id: "s-r10-h5".into(),
            worker_epoch: "100-1".into(),
            binding: binding.clone(),
            command,
        };
        let cleanup_cmd = crate::execution_protocol::Command::Observe {
            operation_id: "op-1".into(),
            wait_ms: 0,
        };
        let non_cleanup_cmd = crate::execution_protocol::Command::Describe;

        let mut stale_crashed = crashed_snapshot("s-r10-h5", "100-1");
        stale_crashed.crashed_drain_active = true;
        stale_crashed.launch.as_mut().unwrap().execution_binding = Some(encoded.clone());
        rt.shared
            .workers
            .lock()
            .insert("s-r10-h5".into(), stale_crashed);
        assert!(!execution_owner_matches(
            &rt.shared,
            &make_req(cleanup_cmd.clone())
        ));

        let mut draining_running = crashed_snapshot("s-r10-h5", "100-1");
        draining_running.crashed_drain_active = true;
        draining_running.launch.as_mut().unwrap().execution_binding = Some(encoded);
        record_welcome_draining_crashed_epoch(&rt.shared, &draining_running);
        draining_running.state = WorkerState::Running;
        rt.shared
            .workers
            .lock()
            .insert("s-r10-h5".into(), draining_running);
        assert!(execution_owner_matches(&rt.shared, &make_req(cleanup_cmd)));
        assert!(!execution_owner_matches(
            &rt.shared,
            &make_req(non_cleanup_cmd)
        ));
    }

    #[tokio::test]
    async fn republished_running_snapshot_with_active_drain_and_duplicate_refusal_preserve_authority()
     {
        let hub = crate::core::Hub::new();
        let snap = crashed_snapshot("s-r11-h5", "105-1");
        let rt = super::super::RemoteRuntime::for_test(hub, vec![snap.clone()]);
        rt.connect_for_test();

        let first_cmd = super::super::CoreCommand::DrainCrashedWorker {
            session_id: "s-r11-h5".into(),
            command_id: "dc-first".into(),
            worker_epoch: "105-1".into(),
            acknowledged_uncertain_sequence: 0,
        };
        handle_crashed_drain_command_ack(&rt.shared, "s-r11-h5", Some(&first_cmd), true, None);
        assert!(is_draining_crashed_epoch(&rt.shared, "s-r11-h5", "105-1"));
        assert!(!rt.drain_crashed_worker("s-r11-h5", "105-1"));

        let second_cmd = super::super::CoreCommand::DrainCrashedWorker {
            session_id: "s-r11-h5".into(),
            command_id: "dc-second".into(),
            worker_epoch: "105-1".into(),
            acknowledged_uncertain_sequence: 0,
        };
        handle_crashed_drain_command_ack(
            &rt.shared,
            "s-r11-h5",
            Some(&second_cmd),
            false,
            Some("crashed_worker_drain_already_in_progress"),
        );
        assert!(is_draining_crashed_epoch(&rt.shared, "s-r11-h5", "105-1"));

        let mut republished_running = snap;
        republished_running.state = WorkerState::Running;
        republished_running.crashed_drain_active = true;
        observe_crashed_sighting(&rt.shared, &republished_running);
        assert!(is_draining_crashed_epoch(&rt.shared, "s-r11-h5", "105-1"));

        republished_running.crashed_drain_active = false;
        observe_crashed_sighting(&rt.shared, &republished_running);
        assert!(!is_draining_crashed_epoch(&rt.shared, "s-r11-h5", "105-1"));
    }

    #[tokio::test]
    async fn retiring_unprojected_crash_clears_stuck_busy_unless_replaced_or_cleared() {
        let hub = crate::core::Hub::new();
        hub.create_local_session(
            "s-retire".into(),
            "codex".into(),
            "/tmp".into(),
            "test".into(),
            crate::model::SessionOrigin::Web,
            false,
        );
        let rt = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());
        rt.connect_for_test();

        hub.set_status("s-retire", Status::Busy, None);
        observe_crashed_sighting(&rt.shared, &crashed_snapshot("s-retire", "200-1"));
        retire_crashed_worker_drained(&rt.shared, "s-retire", "200-1");
        assert_eq!(
            hub.session_info("s-retire").unwrap().meta.status,
            Status::Crashed
        );

        let mut repl = crashed_snapshot("s-retire", "200-2");
        repl.state = WorkerState::Busy;
        repl.current_turn_id = Some("live-turn".into());
        rt.shared.workers.lock().insert("s-retire".into(), repl);
        hub.set_status("s-retire", Status::Busy, None);
        retire_crashed_worker_drained(&rt.shared, "s-retire", "200-old");
        assert_eq!(
            hub.session_info("s-retire").unwrap().meta.status,
            Status::Busy
        );

        let mut high_drain = crashed_snapshot("s-retire", "200-3");
        high_drain.crashed_drain_active = true;
        high_drain.last_runtime_seq = 15;
        record_welcome_draining_crashed_epoch(&rt.shared, &high_drain);
        let mut low_drain = crashed_snapshot("s-retire", "200-3");
        low_drain.state = WorkerState::Busy;
        low_drain.current_turn_id = Some("old".into());
        low_drain.crashed_drain_active = true;
        low_drain.last_runtime_seq = 5;
        record_welcome_draining_crashed_epoch(&rt.shared, &low_drain);
        let sighting = rt
            .shared
            .crashed_worker_sightings
            .lock()
            .get("s-retire")
            .cloned()
            .unwrap();
        assert_eq!(sighting.last_runtime_seq, 15);
        assert_eq!(sighting.state, WorkerState::Crashed);

        rt.shared.resetting.lock().insert("s-retire".into());
        rt.shared
            .reset_replacements
            .lock()
            .insert("s-retire".into(), ("broker-s-retire".into(), None));
        let mut ownerless = crashed_snapshot("s-retire", "broker-s-retire");
        ownerless.crashed_drain_active = false;
        observe_crashed_sighting(&rt.shared, &ownerless);
        assert!(!is_draining_crashed_epoch(&rt.shared, "s-retire", "200-3"));
        let _ = Frame::Heartbeat;
    }

    #[tokio::test]
    async fn clear_reclassification_on_draining_worker_evicts_cache_and_revokes_cleanup_grant() {
        let hub = crate::core::Hub::new();
        hub.create_local_session(
            "s-h3".into(),
            "codex".into(),
            "/tmp".into(),
            "test".into(),
            crate::model::SessionOrigin::Web,
            false,
        );
        hub.set_setting(
            format!(
                "{}s-h3",
                crate::core::settings_keys::SESSION_CLEARED_NATIVE_PREFIX
            ),
            serde_json::json!(["native-cleared"]),
        );

        let rt = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());
        rt.connect_for_test();

        // First sighting has no native ID yet, so it enters workers and draining_crashed_epochs.
        let mut unidentified = crashed_snapshot("s-h3", "ep-h3");
        unidentified.agent_session_id = None;
        unidentified.crashed_drain_active = true;
        observe_frame_snapshot(&rt.shared, &unidentified);
        assert!(project_snapshot(&rt.shared, &unidentified));
        assert!(rt.shared.workers.lock().contains_key("s-h3"));
        assert!(is_draining_crashed_epoch(&rt.shared, "s-h3", "ep-h3"));

        // Later snapshot reveals the discarded native ID from before Clear.
        let mut identified = unidentified.clone();
        identified.agent_session_id = Some("native-cleared".into());
        observe_frame_snapshot(&rt.shared, &identified);
        assert!(
            !rt.shared.workers.lock().contains_key("s-h3"),
            "Clear reclassification via observe_frame_snapshot must evict cached worker (R20-H3)"
        );
        assert!(
            !is_draining_crashed_epoch(&rt.shared, "s-h3", "ep-h3"),
            "Clear reclassification must revoke draining cleanup authority (R20-H3)"
        );
        assert!(!project_snapshot(&rt.shared, &identified));
    }

    #[tokio::test]
    async fn delayed_retirement_never_overwrites_unprojected_successor_or_starting_replacement() {
        let hub = crate::core::Hub::new();
        hub.create_local_session(
            "s-m1".into(),
            "codex".into(),
            "/tmp".into(),
            "test".into(),
            crate::model::SessionOrigin::Web,
            false,
        );
        let rt = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());
        rt.connect_for_test();

        // (a) Unprojected successor in crashed_worker_sightings blocks delayed old receipt.
        let mut succ = crashed_snapshot("s-m1", "ep-succ");
        succ.state = WorkerState::Busy;
        succ.current_turn_id = Some("turn-succ".into());
        observe_crashed_sighting(&rt.shared, &succ);
        hub.set_status("s-m1", Status::Busy, None);
        retire_crashed_worker_drained(&rt.shared, "s-m1", "ep-old-1");
        assert_eq!(
            hub.session_info("s-m1").unwrap().meta.status,
            Status::Busy,
            "unprojected successor sighting must prevent old receipt from setting Crashed (R20-M1)"
        );

        // (b) Delayed receipt for an untracked epoch during Starting replacement is ignored.
        rt.shared.crashed_worker_sightings.lock().clear();
        hub.set_status("s-m1", Status::Starting, None);
        retire_crashed_worker_drained(&rt.shared, "s-m1", "ep-old-2");
        assert_eq!(
            hub.session_info("s-m1").unwrap().meta.status,
            Status::Starting,
            "untracked delayed receipt must never overwrite a Starting replacement (R20-M1)"
        );

        // (c) Pending EnsureSession blocks even a tracked epoch's receipt from overwriting Hub.
        observe_crashed_sighting(&rt.shared, &crashed_snapshot("s-m1", "ep-old-3"));
        rt.shared.pending.lock().insert(
            "ensure:s-m1".into(),
            super::super::CoreCommand::EnsureSession {
                session: crashed_snapshot("s-m1", "ep-old-3").launch.unwrap(),
            },
        );
        hub.set_status("s-m1", Status::Busy, None);
        retire_crashed_worker_drained(&rt.shared, "s-m1", "ep-old-3");
        assert_eq!(
            hub.session_info("s-m1").unwrap().meta.status,
            Status::Busy,
            "pending replacement launch must prevent delayed receipt from overwriting Hub (R20-M1)"
        );
    }
}
