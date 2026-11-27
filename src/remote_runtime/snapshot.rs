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
        let mut required_uncertain = self
            .uncertain_effect
            .as_ref()
            .map_or(0, |effect| effect.sequence.max(1));
        if let Some(persisted) = super::config_sync::persisted_uncertain_effect_for(hub, session_id)
        {
            required_uncertain = required_uncertain.max(persisted.sequence.max(1));
        }
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

pub(super) fn observe_crashed_sighting(shared: &Shared, worker: &WorkerSnapshot) {
    let fence = shared.projection_fence.lock();
    if *fence || shared.resetting.lock().contains(&worker.session_id) {
        return;
    }
    let mut sightings = shared.crashed_worker_sightings.lock();
    if worker.has_connected_owner() {
        sightings.insert(
            worker.session_id.clone(),
            SightingActivity::from_snapshot(worker),
        );
    } else {
        sightings.remove(&worker.session_id);
    }
}

pub(super) fn observe_crashed_event(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
    event: &RuntimeEvent,
) {
    let fence = shared.projection_fence.lock();
    if *fence || shared.resetting.lock().contains(session_id) {
        return;
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

/// Durably record an observed uncertain effect in `session_uncertain_effect_ack:<session>`
/// (CAS against concurrent owner acknowledgements or reconciler writes) and
/// publish it into `Hub` settings so it survives crashed-worker retirement,
/// named-account projection denial, runtime replacement, and Controller restart.
pub(super) async fn persist_observed_uncertain_effect(
    shared: &Shared,
    session_id: &str,
    effect: crate::runtime_wire::UncertainEffect,
) {
    let sequence = effect.sequence.max(1);
    let key = format!(
        "{}{}",
        crate::core::settings_keys::SESSION_UNCERTAIN_EFFECT_ACK_PREFIX,
        session_id
    );
    if shared
        .hub
        .setting(&key)
        .and_then(|value| value.get("observed").and_then(serde_json::Value::as_u64))
        .is_some_and(|observed| observed >= sequence)
    {
        return;
    }
    {
        let mut retained = shared.retained_uncertain_effects.lock();
        let merged = retained.entry(session_id.to_owned()).or_insert_with(|| {
            crate::runtime_wire::UncertainEffect {
                method: effect.method.clone(),
                sequence,
            }
        });
        let mut slot = Some(merged.clone());
        crate::runtime_wire::merge_uncertain_effect(&mut slot, effect.clone());
        if let Some(updated) = slot {
            *merged = updated;
        }
    }
    let store = shared
        .native_store
        .lock()
        .as_ref()
        .map(|(_, store)| store.clone());
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_u64, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        });
    let Some(store) = store else {
        let mut record = shared
            .hub
            .setting(&key)
            .unwrap_or_else(|| serde_json::json!({}));
        if record["observed"].as_u64().unwrap_or(0) < sequence {
            record["observed"] = serde_json::json!(sequence);
            record["method"] = serde_json::json!(effect.method);
            record["observed_at_ms"] = serde_json::json!(now_ms);
            shared.hub.set_setting(key, record);
        }
        return;
    };
    for _ in 0..4 {
        let prev = match store.load_settings().await {
            Ok(settings) => settings
                .into_iter()
                .find(|(existing_key, _)| existing_key == &key)
                .map(|(_, value)| value),
            Err(error) => {
                tracing::warn!(session = session_id, %error, "loading uncertain effect setting failed");
                return;
            }
        };
        let mut record = prev.clone().unwrap_or_else(|| serde_json::json!({}));
        if record["observed"].as_u64().unwrap_or(0) >= sequence {
            shared.hub.set_setting(key, record);
            return;
        }
        record["observed"] = serde_json::json!(sequence);
        record["method"] = serde_json::json!(effect.method);
        record["observed_at_ms"] = serde_json::json!(now_ms);
        match store
            .compare_and_put_setting(&key, prev.as_ref(), &record)
            .await
        {
            Ok(true) => {
                shared.hub.set_setting(key, record);
                return;
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(session = session_id, %error, "persisting uncertain effect setting failed");
                return;
            }
        }
    }
}

pub(super) async fn persist_event_uncertain_effect(
    shared: &Shared,
    session_id: &str,
    event: &RuntimeEvent,
) {
    if let RuntimeEvent::Update { update, .. } = event
        && let Some(effect) = crate::runtime_wire::uncertain_effect(update)
    {
        persist_observed_uncertain_effect(shared, session_id, effect).await;
    }
}

pub(super) async fn observe_and_persist_snapshot(shared: &Shared, worker: &WorkerSnapshot) {
    observe_crashed_sighting(shared, worker);
    if let Some(effect) = worker.uncertain_effect.clone() {
        persist_observed_uncertain_effect(shared, &worker.session_id, effect).await;
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
}

pub(super) async fn handle_crashed_worker_drained_frame(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
) {
    if super::foreign_session(shared, session_id) {
        super::reject_foreign_frame(shared, session_id, "CrashedWorkerDrained");
        return;
    }
    retire_crashed_worker_drained(shared, session_id, worker_epoch).await;
}

pub(super) async fn retire_crashed_worker_drained(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
) {
    // Capture any uncertain effect from both raw sighting (covers unprojected
    // named workers) and cached worker snapshot BEFORE retiring them.
    let effect_to_persist = {
        let fence = shared.projection_fence.lock();
        if *fence {
            return;
        }
        let mut merged: Option<crate::runtime_wire::UncertainEffect> = None;
        if let Some(entry) = shared.crashed_worker_sightings.lock().get(session_id)
            && entry.worker_epoch == worker_epoch
            && let Some(effect) = entry.uncertain_effect.clone()
        {
            crate::runtime_wire::merge_uncertain_effect(&mut merged, effect);
        }
        if let Some(worker) = shared.workers.lock().get(session_id)
            && worker.worker_epoch == worker_epoch
            && let Some(effect) = worker.uncertain_effect.clone()
        {
            crate::runtime_wire::merge_uncertain_effect(&mut merged, effect);
        }
        merged
    };
    if let Some(effect) = effect_to_persist {
        persist_observed_uncertain_effect(shared, session_id, effect).await;
    }
    let fence = shared.projection_fence.lock();
    if *fence {
        return;
    }
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
        shared.retained_uncertain_effects.lock().remove(session_id);
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

pub(super) fn worker_snapshot_with_retained_uncertain(
    shared: &Shared,
    session_id: &str,
) -> Option<WorkerSnapshot> {
    let mut worker = shared.workers.lock().get(session_id).cloned()?;
    let retained = shared
        .retained_uncertain_effects
        .lock()
        .get(session_id)
        .cloned();
    if let Some(effect) = retained {
        if super::config_sync::acknowledged_uncertain_sequence_for(&shared.hub, session_id)
            >= effect.sequence.max(1)
        {
            shared.retained_uncertain_effects.lock().remove(session_id);
        } else {
            crate::runtime_wire::merge_uncertain_effect(&mut worker.uncertain_effect, effect);
        }
    }
    if let Some(persisted) =
        super::config_sync::persisted_uncertain_effect_for(&shared.hub, session_id)
    {
        crate::runtime_wire::merge_uncertain_effect(&mut worker.uncertain_effect, persisted);
    }
    Some(worker)
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
    /// current connection. Uses the raw connected-worker sighting table so an
    /// unprojected named worker that transitions `Running` -> `Crashed` enters
    /// the sweep while a non-crashed replacement never falls back to a stale
    /// projected crash.
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

        // Replacement snapshot merges retained uncertain_effect until acknowledged.
        let mut replacement = super::super::tests::snapshot("s-2");
        replacement.worker_epoch = "55-2".into();
        replacement.state = WorkerState::Running;
        rt.shared.workers.lock().insert("s-2".into(), replacement);
        let merged = rt.worker_snapshot("s-2").unwrap();
        assert_eq!(
            merged.uncertain_effect.as_ref().map(|e| e.sequence),
            Some(2)
        );
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
    async fn persisted_uncertain_effect_blocks_successor_even_after_worker_retirement() {
        let hub = crate::core::Hub::new();
        let mut snap = crashed_snapshot("s-persisted", "91-1");
        snap.uncertain_effect = Some(UncertainEffect {
            method: "write".into(),
            sequence: 5,
        });
        let rt = super::super::RemoteRuntime::for_test(hub.clone(), vec![snap]);
        rt.connect_for_test();

        retire_crashed_worker_drained(&rt.shared, "s-persisted", "91-1").await;
        assert!(rt.shared.workers.lock().get("s-persisted").is_none());

        // Fresh successor worker has no local uncertain_effect, yet persisted Hub setting still blocks it.
        let mut successor = super::super::tests::snapshot("s-persisted");
        successor.worker_epoch = "91-2".into();
        successor.uncertain_effect = None;
        assert!(
            super::super::config_sync::unacknowledged_uncertain_effect(&hub, &successor).is_some(),
            "persisted uncertain_effect must survive retirement and gate successor"
        );
    }
}
