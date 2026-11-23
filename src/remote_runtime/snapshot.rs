use super::{
    RuntimeEvent, Shared, Status, WorkerSnapshot, WorkerState, fail_config_startup,
    idle_snapshot_guard, reconcile_idle_guard, recoverable_live_worker_error,
    sync_config_for_worker, worker_status,
};

/// The session accepts runtime projection and `worker` belongs to its current
/// context (`Hub::admit_runtime_epoch`): Clear fences the epochs it discarded.
pub(super) fn admits_worker(shared: &Shared, worker: &WorkerSnapshot) -> bool {
    shared.hub.accepts_runtime_projection(&worker.session_id) && current_context(shared, worker)
}

/// `worker` belongs to the session's current context; one of a cleared context
/// owns nothing, not even the worker cache entry.
pub(super) fn current_context(shared: &Shared, worker: &WorkerSnapshot) -> bool {
    shared.hub.admit_runtime_epoch(
        &worker.session_id,
        &worker.worker_epoch,
        worker.agent_session_id.as_deref(),
    )
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
        && shared.hub.admit_runtime_epoch(session_id, epoch, native)
}

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
        (worker.state == WorkerState::Crashed)
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
}

impl super::RemoteRuntime {
    /// The worker this Controller last saw for `session_id` in the Crashed
    /// state, as the Machine reported it on the current connection. The
    /// Controller treats it as absent ([`Self::has_worker`]) and never reuses
    /// it, yet nothing stops a live one: the next prompt recycles the
    /// session, a rollout or roll drains only running workers.
    ///
    /// This is no proof that the process is still alive. The runtime wire
    /// has no attach or detach signal: a worker that disconnects restates
    /// itself as Crashed with the same epoch (or, for an undeclared session,
    /// says nothing), and only the inventory of the next connection drops it.
    /// Read-only evidence for an operator, who verifies the process first.
    pub fn last_crashed_worker(&self, session_id: &str) -> CrashedWorkerObservation {
        // One read: the worker map is held while the connection state is
        // checked, so "absent" always comes from a connected runtime.
        let workers = self.shared.workers.lock();
        if !self.connected() {
            return CrashedWorkerObservation::Unavailable;
        }
        workers
            .get(session_id)
            .filter(|worker| worker.has_connected_owner() && worker.state == WorkerState::Crashed)
            .map_or(CrashedWorkerObservation::Absent, |worker| {
                CrashedWorkerObservation::Seen(CrashedWorkerSighting {
                    worker_epoch: worker.worker_epoch.clone(),
                    generation: worker.generation.clone(),
                })
            })
    }
}
