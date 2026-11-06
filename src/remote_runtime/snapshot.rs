use super::{
    Shared, Status, WorkerSnapshot, WorkerState, fail_config_startup, idle_snapshot_guard,
    reconcile_idle_guard, recoverable_live_worker_error, sync_config_for_worker, worker_status,
};

pub(super) fn apply_snapshot(shared: &Shared, worker: &WorkerSnapshot) -> bool {
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
