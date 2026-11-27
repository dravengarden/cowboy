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
    shared.hub.accepts_runtime_projection(session_id)
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
    generation: String,
    drain_requested: bool,
    busy_turn: bool,
    pending_permissions: std::collections::HashSet<String>,
    background_tasks: bool,
    pending_prompt_count: u64,
    native_cycle_active: bool,
    uncertain_effect: Option<crate::runtime_wire::UncertainEffect>,
}

impl SightingActivity {
    pub(super) fn from_snapshot(worker: &WorkerSnapshot) -> Self {
        Self {
            state: worker.state,
            worker_epoch: worker.worker_epoch.clone(),
            generation: worker.generation.clone(),
            drain_requested: worker.drain_requested,
            busy_turn: worker.current_turn_id.is_some(),
            pending_permissions: worker.pending_permissions.iter().cloned().collect(),
            background_tasks: worker.background_tasks.unwrap_or(0) > 0,
            pending_prompt_count: worker.pending_prompt_count,
            native_cycle_active: worker.native_cycle_active || worker.native_cycle_id.is_some(),
            uncertain_effect: worker.uncertain_effect.clone(),
        }
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

pub(super) fn clear_draining_crashed_epoch_if_matches(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
) {
    let mut draining = shared.draining_crashed_epochs.lock();
    if draining.get(session_id).is_some_and(|e| e == worker_epoch) {
        draining.remove(session_id);
    }
    shared
        .draining_replay_snapshots
        .lock()
        .remove(&(session_id.to_owned(), worker_epoch.to_owned()));
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
/// so replay bypass is granted ONLY after broker confirmation and revoked on
/// refusal (R8-M6).
pub(super) fn handle_crashed_drain_command_ack(
    shared: &Shared,
    session_id: &str,
    command: Option<&super::CoreCommand>,
    accepted: bool,
) {
    let Some(super::CoreCommand::DrainCrashedWorker { worker_epoch, .. }) = command else {
        return;
    };
    if !accepted {
        clear_draining_crashed_epoch_if_matches(shared, session_id, worker_epoch);
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
        let _ = authorize_draining_crashed_epoch_locked(shared, &worker);
    }
}

pub(super) fn is_event_epoch_blocked(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
) -> bool {
    shared
        .blocked_draining_epochs
        .lock()
        .contains(&(session_id.to_owned(), worker_epoch.to_owned()))
}

pub(super) fn settle_draining_event_admission(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
    outcome: super::native_admission::EventAdmission,
) -> anyhow::Result<bool> {
    use super::native_admission::EventAdmission;
    if outcome == EventAdmission::Admitted
        || !is_draining_crashed_epoch(shared, session_id, worker_epoch)
    {
        return Ok(true);
    }
    shared
        .blocked_draining_epochs
        .lock()
        .insert((session_id.to_owned(), worker_epoch.to_owned()));
    match outcome {
        EventAdmission::TransientlyDeferred => {
            anyhow::bail!(
                "draining crashed worker event admission transiently deferred; restarting ordered replay"
            )
        }
        EventAdmission::PermanentlyDenied => {
            clear_draining_crashed_epoch_if_matches(shared, session_id, worker_epoch);
            Ok(false)
        }
        EventAdmission::Admitted => Ok(true),
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
) -> std::collections::HashMap<String, String> {
    workers
        .iter()
        .filter(|worker| {
            worker.has_connected_owner()
                && worker.state == WorkerState::Crashed
                && worker.crashed_drain_active
        })
        .map(|worker| (worker.session_id.clone(), worker.worker_epoch.clone()))
        .collect()
}

pub(super) fn seed_draining_replay_snapshots(
    workers: &[WorkerSnapshot],
) -> std::collections::HashMap<(String, String), WorkerSnapshot> {
    workers
        .iter()
        .filter(|worker| {
            worker.has_connected_owner()
                && worker.state == WorkerState::Crashed
                && worker.crashed_drain_active
        })
        .map(|worker| {
            (
                (worker.session_id.clone(), worker.worker_epoch.clone()),
                worker.clone(),
            )
        })
        .collect()
}

/// Validate Clear fence (including native ID) and launch binding under Hub's
/// projection guard before authorizing a crashed worker epoch for draining
/// replay bypass (R7-H2, R7-H3, R8-M6: bound strictly to broker-confirmed
/// `crashed_drain_active`, never rollout `drain_requested`).
fn authorize_draining_crashed_epoch_locked(shared: &Shared, worker: &WorkerSnapshot) -> bool {
    if !worker.has_connected_owner()
        || worker.state != WorkerState::Crashed
        || !worker.crashed_drain_active
        || is_retired_crashed_epoch(shared, &worker.session_id, &worker.worker_epoch)
    {
        return false;
    }
    let _proj = shared.hub.runtime_projection_guard();
    if shared.hub.classify_runtime_epoch(
        &worker.session_id,
        &worker.worker_epoch,
        worker.agent_session_id.as_deref(),
    ) != crate::core::RuntimeEpoch::Current
        || !shared.hub.matches_runtime_snapshot_binding(worker)
    {
        return false;
    }
    shared
        .draining_crashed_epochs
        .lock()
        .insert(worker.session_id.clone(), worker.worker_epoch.clone());
    shared.draining_replay_snapshots.lock().insert(
        (worker.session_id.clone(), worker.worker_epoch.clone()),
        worker.clone(),
    );
    true
}

/// Capture epoch-bound draining crashed worker evidence from Welcome before
/// `resetting` filters normal snapshot projection during bootstrap reload.
/// Validates Clear fence (including native ID) and launch binding under Hub's
/// projection guard and caches the verified snapshot in
/// `draining_replay_snapshots` so `native_admission::event` admits trailing
/// replay even when `shared.workers` holds an ownerless placeholder (R7-H2, R7-H3).
pub(super) fn record_welcome_draining_crashed_epoch(shared: &Shared, worker: &WorkerSnapshot) {
    let fence = shared.projection_fence.lock();
    if *fence {
        return;
    }
    if !authorize_draining_crashed_epoch_locked(shared, worker) {
        return;
    }
    shared.crashed_worker_sightings.lock().insert(
        worker.session_id.clone(),
        SightingActivity::from_snapshot(worker),
    );
}

/// Observe raw connected-worker state. Rejects late snapshots for already-retired
/// crashed epochs (R3-M7), records replacement epochs during reset (R2-M6), and
/// authorizes draining replay ONLY after verifying Clear fence + launch binding
/// under Hub's projection guard (R7-H2).
pub(super) fn observe_crashed_sighting(shared: &Shared, worker: &WorkerSnapshot) {
    let fence = shared.projection_fence.lock();
    if *fence {
        return;
    }
    if is_retired_crashed_epoch(shared, &worker.session_id, &worker.worker_epoch) {
        return;
    }
    if shared.resetting.lock().contains(&worker.session_id) {
        let is_replaced_epoch = shared
            .reset_replacements
            .lock()
            .get(&worker.session_id)
            .is_none_or(|(replaced, _)| replaced == &worker.worker_epoch);
        if is_replaced_epoch {
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
                .is_some_and(|e| e != &worker.worker_epoch)
            {
                draining.remove(&worker.session_id);
                shared
                    .draining_replay_snapshots
                    .lock()
                    .retain(|(sid, _), _| sid != &worker.session_id);
            }
        }
        if worker.state == WorkerState::Crashed && worker.crashed_drain_active {
            let _ = authorize_draining_crashed_epoch_locked(shared, worker);
        } else {
            clear_draining_crashed_epoch_if_matches(
                shared,
                &worker.session_id,
                &worker.worker_epoch,
            );
        }
        shared.crashed_worker_sightings.lock().insert(
            worker.session_id.clone(),
            SightingActivity::from_snapshot(worker),
        );
    } else {
        shared
            .crashed_worker_sightings
            .lock()
            .remove(&worker.session_id);
        shared
            .draining_crashed_epochs
            .lock()
            .remove(&worker.session_id);
        shared
            .draining_replay_snapshots
            .lock()
            .retain(|(sid, _), _| sid != &worker.session_id);
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
    if is_draining_crashed_epoch(shared, &request.session_id, &request.worker_epoch) {
        if !is_cleanup_only_execution_command(&request.command) {
            return false;
        }
        if let Some(worker) = shared
            .draining_replay_snapshots
            .lock()
            .get(&(request.session_id.clone(), request.worker_epoch.clone()))
            && matches_binding(worker)
        {
            return true;
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
        .is_some_and(|epoch| epoch == worker_epoch)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn project_admitted_worker_event(
    shared: &Shared,
    key: &(String, String),
    session_id: &str,
    worker_epoch: &str,
    runtime_seq: u64,
    event: &RuntimeEvent,
    diagnostics: &[crate::runtime_trace::SpanRecord],
    undelivered_ack: &mut Option<super::notice_recovery::PendingUndeliveredNotice>,
) {
    let resetting = shared.resetting.lock().contains(session_id);
    let stale_idle = matches!(
        event,
        RuntimeEvent::Status {
            state: WorkerState::Running | WorkerState::Draining,
            ..
        }
    ) && shared
        .workers
        .lock()
        .get(session_id)
        .is_some_and(|worker| worker.current_turn_id.is_some());
    if stale_idle {
        tracing::warn!(
            session = %session_id,
            runtime_seq,
            "ignoring stale idle status while a newer turn is active"
        );
        return;
    }
    if !should_project_worker_event(shared, resetting, session_id, worker_epoch, event) {
        return;
    }
    if let Some((machine, telemetry)) = shared.telemetry.lock().as_ref() {
        telemetry.record_runtime_spans(machine, session_id, diagnostics);
    }
    super::resolve_steering_update(shared, session_id, event);
    super::update_snapshot_from_event(shared, session_id, worker_epoch, runtime_seq, event);
    if let RuntimeEvent::Status {
        state: WorkerState::Crashed | WorkerState::Exited,
        detail,
    } = event
    {
        super::fail_config_startup(
            shared,
            session_id,
            detail
                .as_deref()
                .unwrap_or("worker exited before configuration completed"),
        );
    }
    let draining_replay_only = shared
        .workers
        .lock()
        .get(session_id)
        .is_some_and(|w| w.worker_epoch != worker_epoch)
        && shared
            .draining_replay_snapshots
            .lock()
            .contains_key(&(session_id.to_owned(), worker_epoch.to_owned()));
    let worker = if draining_replay_only {
        None
    } else {
        shared.workers.lock().get(session_id).cloned()
    };
    let auto_permission = (!draining_replay_only)
        .then(|| codex_full_access_permission(shared, session_id, event))
        .flatten();
    let idle_guard = worker
        .as_ref()
        .and_then(|worker| super::idle_snapshot_guard(shared, worker));
    if matches!(event, RuntimeEvent::ConfigOptions { .. })
        && let Some(worker) = worker.as_ref()
    {
        super::sync_config_for_worker(shared, worker);
    }
    *undelivered_ack = super::notice_recovery::project_worker_outcome(
        shared,
        key,
        session_id,
        runtime_seq,
        event.clone(),
        idle_guard,
        auto_permission,
    );
}

pub(super) fn should_project_worker_event(
    shared: &Shared,
    resetting: bool,
    session_id: &str,
    worker_epoch: &str,
    event: &RuntimeEvent,
) -> bool {
    if is_retired_crashed_epoch(shared, session_id, worker_epoch) {
        return false;
    }
    let allowed = !resetting
        || (is_draining_crashed_epoch(shared, session_id, worker_epoch)
            && !matches!(
                event,
                RuntimeEvent::Ready { .. }
                    | RuntimeEvent::Status {
                        state: WorkerState::Running | WorkerState::Draining,
                        ..
                    }
            ));
    allowed && admits_event(shared, session_id, worker_epoch, event)
}

pub(super) fn observe_crashed_event(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
    event: &RuntimeEvent,
) {
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
    let Some(entry) = sightings.get_mut(session_id) else {
        return;
    };
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
    shared.blocked_draining_epochs.lock().clear();
    shared
        .crashed_worker_sightings
        .lock()
        .retain(|id, _| ids.contains(id));
    shared
        .draining_crashed_epochs
        .lock()
        .retain(|id, _| ids.contains(id));
    shared
        .draining_replay_snapshots
        .lock()
        .retain(|(id, _), _| ids.contains(id));
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
pub(super) fn retire_crashed_worker_drained(shared: &Shared, session_id: &str, worker_epoch: &str) {
    let fence = shared.projection_fence.lock();
    if *fence {
        return;
    }
    record_retired_crashed_epoch(shared, session_id, worker_epoch);
    clear_draining_crashed_epoch_if_matches(shared, session_id, worker_epoch);
    shared
        .blocked_draining_epochs
        .lock()
        .remove(&(session_id.to_owned(), worker_epoch.to_owned()));
    {
        let mut sightings = shared.crashed_worker_sightings.lock();
        if sightings
            .get(session_id)
            .is_some_and(|entry| entry.worker_epoch == worker_epoch)
        {
            sightings.remove(session_id);
        }
    }
    {
        let mut workers = shared.workers.lock();
        if workers
            .get(session_id)
            .is_some_and(|worker| worker.worker_epoch == worker_epoch)
        {
            workers.remove(session_id);
        }
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
            .draining_replay_snapshots
            .lock()
            .retain(|(sid, _), _| sid != session_id);
        shared
            .blocked_draining_epochs
            .lock()
            .retain(|(sid, _)| sid != session_id);
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
    let raw_wrap = worker.wrap_up_steering;
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
    let applied = apply_snapshot(shared, &worker);
    let repaired = update_declaration(shared, &worker);
    shared.install_worker_with_raw_wrap_up(worker.clone(), raw_wrap);
    if !shared.resetting.lock().contains(&worker.session_id) {
        let mut sightings = shared.crashed_worker_sightings.lock();
        if worker.has_connected_owner() {
            sightings.insert(
                worker.session_id.clone(),
                SightingActivity::from_snapshot(&worker),
            );
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

#[cfg(test)]
mod crashed_drain_tests {
    use super::*;
    use crate::runtime_wire::{CoreCommand, Frame, UncertainEffect};

    fn crashed_snapshot(session_id: &str, epoch: &str) -> WorkerSnapshot {
        let mut snap = super::super::tests::snapshot(session_id);
        snap.worker_epoch = epoch.to_owned();
        snap.state = WorkerState::Crashed;
        snap.current_turn_id = None;
        snap.pending_permissions.clear();
        snap.background_tasks = Some(0);
        snap.pending_prompt_count = 0;
        snap.native_cycle_id = None;
        snap.native_cycle_active = false;
        snap.uncertain_effect = None;
        snap
    }

    #[tokio::test]
    async fn crashed_sighting_gates_on_activity_native_cycle_and_unacknowledged_uncertain_effect() {
        let hub = crate::core::Hub::new();
        hub.create_local_session(
            "s-1".into(),
            "codex".into(),
            "/tmp".into(),
            "Crashed".into(),
            crate::core::SessionOrigin::Api,
            false,
        );
        hub.set_status("s-1", Status::Crashed, Some("boom".into()));
        let mut snap = crashed_snapshot("s-1", "42-1");
        snap.native_cycle_active = true;
        let rt = super::super::RemoteRuntime::for_test(hub.clone(), vec![snap.clone()]);
        rt.connect_for_test();

        let CrashedWorkerObservation::Seen(sighting) = rt.last_crashed_worker("s-1") else {
            panic!("expected crashed sighting");
        };
        assert!(!sighting.idle, "active native cycle must block drain");
        assert!(!rt.drain_crashed_worker("s-1", "42-1"));

        // Ending native cycle while adding an unacknowledged uncertain_effect still blocks drain.
        snap.native_cycle_active = false;
        snap.uncertain_effect = Some(UncertainEffect {
            sequence: 3,
            method: "write".into(),
        });
        observe_crashed_sighting(&rt.shared, &snap);
        let CrashedWorkerObservation::Seen(sighting) = rt.last_crashed_worker("s-1") else {
            panic!("expected crashed sighting");
        };
        assert!(
            !sighting.idle,
            "unacknowledged uncertain_effect must block automatic crashed drain"
        );
        assert!(!rt.drain_crashed_worker("s-1", "42-1"));

        // Owner acknowledges sequence 3 -> sighting becomes idle and drain queues once.
        hub.set_setting(
            format!(
                "{}s-1",
                crate::core::settings_keys::SESSION_UNCERTAIN_EFFECT_ACK_PREFIX
            ),
            serde_json::json!({"acknowledged": 3}),
        );
        let CrashedWorkerObservation::Seen(sighting) = rt.last_crashed_worker("s-1") else {
            panic!("expected crashed sighting");
        };
        assert!(sighting.idle);
        assert!(rt.drain_crashed_worker("s-1", "42-1"));
        assert!(rt.crashed_drain_pending("s-1"));
        assert!(
            !rt.drain_crashed_worker("s-1", "42-1"),
            "duplicate drain while pending must be refused"
        );
    }

    #[tokio::test]
    async fn crashed_worker_drained_retires_matching_epoch_and_keeps_crashed_status_and_uncertain_effect()
     {
        let hub = crate::core::Hub::new();
        hub.create_local_session(
            "s-2".into(),
            "codex".into(),
            "/tmp".into(),
            "Crashed".into(),
            crate::core::SessionOrigin::Api,
            false,
        );
        hub.set_status("s-2", Status::Crashed, Some("vendor refusal".into()));
        let mut snap = crashed_snapshot("s-2", "55-1");
        snap.uncertain_effect = Some(UncertainEffect {
            sequence: 2,
            method: "patch".into(),
        });
        let rt = super::super::RemoteRuntime::for_test(hub.clone(), vec![snap]);
        rt.connect_for_test();

        // Stale epoch retirement does nothing.
        let mut sink = Vec::new();
        super::super::handle_frame(
            &rt.shared,
            Frame::CrashedWorkerDrained {
                session_id: "s-2".into(),
                worker_epoch: "55-0".into(),
            },
            &mut sink,
        )
        .await
        .unwrap();
        assert!(matches!(
            rt.last_crashed_worker("s-2"),
            CrashedWorkerObservation::Seen(_)
        ));

        // Exact epoch retires sighting & worker cache without clearing Crashed status.
        super::super::handle_frame(
            &rt.shared,
            Frame::CrashedWorkerDrained {
                session_id: "s-2".into(),
                worker_epoch: "55-1".into(),
            },
            &mut sink,
        )
        .await
        .unwrap();
        assert_eq!(
            rt.last_crashed_worker("s-2"),
            CrashedWorkerObservation::Absent
        );
        assert_eq!(hub.status("s-2"), Some(Status::Crashed));
        assert_eq!(
            hub.latest_crash_detail("s-2").as_deref(),
            Some("vendor refusal")
        );

        // Verify welcome draining crashed epoch survives active resetting during bootstrap reload (R5-H5).
        let mut reload_snap = crashed_snapshot("s-reload", "66-1");
        reload_snap.crashed_drain_active = true;
        rt.shared.resetting.lock().insert("s-reload".into());
        record_welcome_draining_crashed_epoch(&rt.shared, &reload_snap);
        assert!(is_draining_crashed_epoch(&rt.shared, "s-reload", "66-1"));
        assert!(matches!(
            rt.last_crashed_worker("s-reload"),
            CrashedWorkerObservation::Seen(s) if s.worker_epoch == "66-1"
        ));
    }

    #[tokio::test]
    async fn discard_pending_crashed_drain_drops_unsent_commands_and_respects_protocol_floor() {
        let hub = crate::core::Hub::new();
        let snap = crashed_snapshot("s-3", "77-1");
        let rt = super::super::RemoteRuntime::for_test(hub, vec![snap]);
        rt.connect_for_test();
        assert!(rt.supports_crashed_worker_drain());
        assert!(rt.drain_crashed_worker("s-3", "77-1"));
        assert!(rt.crashed_drain_pending("s-3"));
        discard_pending_crashed_drain(&rt.shared, "s-3", false);
        assert!(!rt.crashed_drain_pending("s-3"));

        // Older runtime protocol refuses drain.
        rt.shared.protocol.store(9, Ordering::Release);
        assert!(!rt.supports_crashed_worker_drain());
        assert!(!rt.drain_crashed_worker("s-3", "77-1"));
        assert!(
            !rt.shared
                .pending
                .lock()
                .values()
                .any(|c| matches!(c, CoreCommand::DrainCrashedWorker { .. }))
        );
    }

    #[tokio::test]
    async fn unprojected_running_to_crashed_event_and_permission_resolved_update_sighting() {
        let hub = crate::core::Hub::new();
        let rt = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());
        rt.connect_for_test();

        // Raw connected Running snapshot (even if never projected into `shared.workers`).
        let mut running = super::super::tests::snapshot("s-named");
        running.worker_epoch = "88-1".into();
        running.state = WorkerState::Running;
        running.current_turn_id = None;
        running.pending_permissions = vec!["perm-1".into()];
        observe_crashed_sighting(&rt.shared, &running);
        assert_eq!(
            rt.last_crashed_worker("s-named"),
            CrashedWorkerObservation::Absent
        );

        // Streamed Crashed status makes it visible, still non-idle due to perm-1.
        observe_crashed_event(
            &rt.shared,
            "s-named",
            "88-1",
            &RuntimeEvent::Status {
                state: WorkerState::Crashed,
                detail: Some("expired".into()),
            },
        );
        let CrashedWorkerObservation::Seen(sighting) = rt.last_crashed_worker("s-named") else {
            panic!("expected crashed sighting after streamed Crashed status");
        };
        assert!(
            !sighting.idle,
            "outstanding permission must keep sighting non-idle"
        );

        // Resolving perm-1 transitions sighting to idle.
        observe_crashed_event(
            &rt.shared,
            "s-named",
            "88-1",
            &RuntimeEvent::PermissionResolved {
                request_id: "perm-1".into(),
                option_id: Some("allow".into()),
            },
        );
        let CrashedWorkerObservation::Seen(sighting) = rt.last_crashed_worker("s-named") else {
            panic!("expected idle crashed sighting");
        };
        assert!(sighting.idle);

        // A newer raw Running replacement must NOT fall back to an older projected Crashed worker.
        let mut stale_projected = crashed_snapshot("s-named", "88-0");
        stale_projected.state = WorkerState::Crashed;
        rt.shared
            .workers
            .lock()
            .insert("s-named".into(), stale_projected);
        let mut fresh_running = super::super::tests::snapshot("s-named");
        fresh_running.worker_epoch = "88-2".into();
        fresh_running.state = WorkerState::Running;
        observe_crashed_sighting(&rt.shared, &fresh_running);
        assert_eq!(
            rt.last_crashed_worker("s-named"),
            CrashedWorkerObservation::Absent
        );
    }

    #[tokio::test]
    async fn retired_crashed_epoch_rejects_late_handover_snapshots_and_preserves_draining_epoch_across_replay()
     {
        let hub = crate::core::Hub::new();
        let snap = crashed_snapshot("s-r3", "101-1");
        let rt = super::super::RemoteRuntime::for_test(hub, vec![snap.clone()]);
        rt.connect_for_test();

        assert!(rt.drain_crashed_worker("s-r3", "101-1"));
        assert!(!is_draining_crashed_epoch(&rt.shared, "s-r3", "101-1"));
        let ack_cmd = super::super::CoreCommand::DrainCrashedWorker {
            session_id: "s-r3".into(),
            command_id: "dc-r3".into(),
            worker_epoch: "101-1".into(),
            acknowledged_uncertain_sequence: 0,
        };
        handle_crashed_drain_command_ack(&rt.shared, "s-r3", Some(&ack_cmd), true);
        assert!(is_draining_crashed_epoch(&rt.shared, "s-r3", "101-1"));

        // Replayed Running/TurnStarted events during reload must NOT clear draining_crashed_epochs (R3-H2).
        observe_crashed_event(
            &rt.shared,
            "s-r3",
            "101-1",
            &RuntimeEvent::Ready {
                agent_session_id: Some("ag-1".into()),
            },
        );
        assert!(is_draining_crashed_epoch(&rt.shared, "s-r3", "101-1"));

        // After retirement, late snapshots for the retired epoch must be rejected (R3-M7).
        retire_crashed_worker_drained(&rt.shared, "s-r3", "101-1");
        assert!(!is_draining_crashed_epoch(&rt.shared, "s-r3", "101-1"));
        assert_eq!(
            rt.last_crashed_worker("s-r3"),
            CrashedWorkerObservation::Absent
        );

        observe_crashed_sighting(&rt.shared, &snap);
        assert!(!project_snapshot(&rt.shared, &snap));
        assert_eq!(
            rt.last_crashed_worker("s-r3"),
            CrashedWorkerObservation::Absent
        );
        assert!(rt.shared.workers.lock().get("s-r3").is_none());

        // A genuinely newer epoch for the same session is admitted normally.
        let newer = crashed_snapshot("s-r3", "101-2");
        observe_crashed_sighting(&rt.shared, &newer);
        assert!(matches!(
            rt.last_crashed_worker("s-r3"),
            CrashedWorkerObservation::Seen(s) if s.worker_epoch == "101-2"
        ));
        // Even AFTER the successor snapshot arrived, a late snapshot for the
        // retired epoch must STILL be rejected (R4-M5).
        observe_crashed_sighting(&rt.shared, &snap);
        assert!(!project_snapshot(&rt.shared, &snap));
        assert!(matches!(
            rt.last_crashed_worker("s-r3"),
            CrashedWorkerObservation::Seen(s) if s.worker_epoch == "101-2"
        ));
    }

    #[tokio::test]
    async fn draining_replay_snapshots_admit_replay_over_ownerless_placeholder_and_reject_raw_sighting()
     {
        let hub = crate::core::Hub::new();
        let rt = super::super::RemoteRuntime::for_test(hub, Vec::new());
        rt.connect_for_test();

        // Raw non-draining Crashed sighting must NOT authorize draining replay bypass (R7-H2).
        let raw_crashed = crashed_snapshot("s-r7", "700-1");
        observe_crashed_sighting(&rt.shared, &raw_crashed);
        assert!(!is_draining_crashed_epoch(&rt.shared, "s-r7", "700-1"));

        // Ownerless placeholder in shared.workers must NOT block replay admission
        // once a verified draining snapshot is captured in draining_replay_snapshots (R7-H3).
        let mut placeholder = crashed_snapshot("s-r7", "broker-s-r7");
        placeholder.state = WorkerState::Crashed;
        rt.shared.workers.lock().insert("s-r7".into(), placeholder);

        let mut rollout_snap = crashed_snapshot("s-r7", "700-1");
        rollout_snap.drain_requested = true;
        record_welcome_draining_crashed_epoch(&rt.shared, &rollout_snap);
        assert!(!is_draining_crashed_epoch(&rt.shared, "s-r7", "700-1"));

        let mut draining_snap = crashed_snapshot("s-r7", "700-1");
        draining_snap.crashed_drain_active = true;
        record_welcome_draining_crashed_epoch(&rt.shared, &draining_snap);
        assert!(is_draining_crashed_epoch(&rt.shared, "s-r7", "700-1"));
        assert!(
            rt.shared
                .draining_replay_snapshots
                .lock()
                .contains_key(&("s-r7".to_owned(), "700-1".to_owned()))
        );

        let mut projected = false;
        let admitted = super::super::native_admission::event(&rt.shared, "s-r7", "700-1", || {
            projected = true;
        })
        .await
        .is_admitted();
        assert!(admitted);
        assert!(projected);
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

        let make_req = |command: crate::execution_protocol::Command| {
            crate::execution_protocol::RuntimeRequest {
                session_id: "s-r10-h5".into(),
                worker_epoch: "100-1".into(),
                request_id: "req-1".into(),
                binding: binding.clone(),
                command,
            }
        };
        let cleanup_cmd = crate::execution_protocol::Command::Observe {
            operation_id: "op-1".into(),
            wait_ms: 0,
        };
        let non_cleanup_cmd = crate::execution_protocol::Command::Describe;

        // Case 1: Unverified cached `crashed_drain_active: true` on a Crashed worker
        // must NOT authorize even cleanup commands when `draining_crashed_epochs` has no entry.
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

        // Case 2: Once verified as draining, even if a replayed Running status
        // overwrote `worker.state`, non-cleanup commands MUST be rejected while
        // cleanup-only commands remain allowed.
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
}
