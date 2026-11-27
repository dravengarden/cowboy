//! Replay-watermark classification, transient-admission retry fences,
//! projection-guarded turn-end signaling, and launch-scoped credential expiry.

use super::{
    RuntimeEvent, Shared, WorkerSnapshot, WorkerState, admits_event,
    clear_draining_crashed_epoch_if_matches, is_draining_crashed_epoch,
};

pub(in crate::remote_runtime) fn clear_blocked_draining_epochs(shared: &Shared) {
    shared.blocked_draining_epochs.lock().clear();
}

pub(in crate::remote_runtime) fn is_event_epoch_blocked(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
) -> bool {
    shared
        .blocked_draining_epochs
        .lock()
        .contains(&(session_id.to_owned(), worker_epoch.to_owned()))
}

/// Handle non-admitted events for ordered replay:
/// - `TransientlyDeferred`: withhold ACK and restart ordered replay on EVERY
///   transient admission failure, even after a drain timeout abort cleared the
///   draining marker, so a retryable store outage never drops unprojected
///   events (R17-H1). The connection-scoped block clears on reconnect.
/// - `PermanentlyDenied`: clear any active draining replay bypass without
///   blocking later frames so durably-backed `call-notice:` undelivered
///   receipts still settle and ordered replay is never stranded (R16-H3).
pub(in crate::remote_runtime) fn settle_draining_event_admission(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
    outcome: crate::remote_runtime::native_admission::EventAdmission,
) -> anyhow::Result<()> {
    use crate::remote_runtime::native_admission::EventAdmission;
    match outcome {
        EventAdmission::Admitted => Ok(()),
        EventAdmission::TransientlyDeferred => {
            shared
                .blocked_draining_epochs
                .lock()
                .insert((session_id.to_owned(), worker_epoch.to_owned()));
            anyhow::bail!(
                "worker event admission transiently deferred; withholding ACK to restart ordered replay"
            )
        }
        EventAdmission::PermanentlyDenied => {
            if is_draining_crashed_epoch(shared, session_id, worker_epoch) {
                tracing::warn!(
                    session = %session_id,
                    %worker_epoch,
                    "draining crashed worker event permanently denied; aborting drain replay authorization"
                );
                clear_draining_crashed_epoch_if_matches(shared, session_id, worker_epoch);
            }
            Ok(())
        }
    }
}

/// Preserve the highest observed `last_runtime_seq` and its associated snapshot
/// for each draining epoch so a delayed snapshot or drain ACK cloning an older
/// worker cache can never lower the replay watermark (R16-M2).
pub(super) fn upsert_draining_replay_snapshot(shared: &Shared, worker: &WorkerSnapshot) {
    let mut snaps = shared.draining_replay_snapshots.lock();
    let key = (worker.session_id.clone(), worker.worker_epoch.clone());
    match snaps.get_mut(&key) {
        Some(existing) if existing.last_runtime_seq > worker.last_runtime_seq => {
            if let Some(effect) = worker.uncertain_effect.clone() {
                crate::runtime_wire::merge_uncertain_effect(&mut existing.uncertain_effect, effect);
            }
        }
        Some(existing) => {
            let prev_effect = existing.uncertain_effect.clone();
            let mut next = worker.clone();
            if let Some(effect) = prev_effect {
                crate::runtime_wire::merge_uncertain_effect(&mut next.uncertain_effect, effect);
            }
            *existing = next;
        }
        None => {
            snaps.insert(key, worker.clone());
        }
    }
}

/// Unified monotonic `last_runtime_seq` watermark for `(session_id, worker_epoch)`
/// across the owning worker cache, `draining_replay_snapshots`, and
/// `crashed_worker_sightings` so replay classification and maintenance
/// reduction always observe the same watermark (R15-M2, R17-M5).
pub(super) fn epoch_max_watermark(shared: &Shared, session_id: &str, worker_epoch: &str) -> u64 {
    let worker_seq = shared
        .workers
        .lock()
        .get(session_id)
        .filter(|w| w.has_connected_owner() && w.worker_epoch == worker_epoch)
        .map_or(0, |w| w.last_runtime_seq);
    let drain_seq = shared
        .draining_replay_snapshots
        .lock()
        .get(&(session_id.to_owned(), worker_epoch.to_owned()))
        .map_or(0, |w| w.last_runtime_seq);
    let sighting_seq = shared
        .crashed_worker_sightings
        .lock()
        .get(session_id)
        .filter(|entry| entry.worker_epoch == worker_epoch)
        .map_or(0, |entry| entry.last_runtime_seq);
    worker_seq.max(drain_seq).max(sighting_seq)
}

/// Scope historical-only gating strictly to non-owning draining epochs OR
/// below-watermark replay of an already-Crashed/draining epoch (using the max
/// same-epoch watermark across `shared.workers`, `draining_replay_snapshots`,
/// and `crashed_worker_sightings`), leaving live Running/Busy replay untouched
/// (R15-H1, R15-H2, R15-M2, R17-M5).
pub(in crate::remote_runtime) fn is_historical_replay_event(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
    runtime_seq: u64,
) -> bool {
    let owning_worker = shared
        .workers
        .lock()
        .get(session_id)
        .filter(|w| w.has_connected_owner() && w.worker_epoch == worker_epoch)
        .cloned();
    let drain_replay_snap = shared
        .draining_replay_snapshots
        .lock()
        .get(&(session_id.to_owned(), worker_epoch.to_owned()))
        .cloned();
    let non_owning_epoch = owning_worker.is_none() && drain_replay_snap.is_some();
    let max_watermark = epoch_max_watermark(shared, session_id, worker_epoch);
    let crashed_or_draining = non_owning_epoch
        || is_draining_crashed_epoch(shared, session_id, worker_epoch)
        || owning_worker
            .as_ref()
            .is_some_and(|w| w.state == WorkerState::Crashed);
    non_owning_epoch || (crashed_or_draining && runtime_seq > 0 && runtime_seq < max_watermark)
}

/// Evaluate under Hub's projection guard whether an admitted `TurnEnded`
/// belongs to the positively identified current owner and matches the session's
/// credential pin, then signal account-wide controls outside the guard (R16-H1,
/// R17-H2).
pub(in crate::remote_runtime) fn signal_admitted_turn_end(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
    runtime_seq: u64,
    event: &RuntimeEvent,
) {
    if !matches!(event, RuntimeEvent::TurnEnded { .. }) {
        return;
    }
    let should_signal = {
        let _projection = shared.hub.runtime_projection_guard();
        let historical_only =
            is_historical_replay_event(shared, session_id, worker_epoch, runtime_seq);
        should_signal_turn_end(shared, session_id, worker_epoch, event, historical_only)
    };
    if should_signal {
        crate::remote_runtime::turn_end::signal(shared, session_id, event);
    }
}

fn should_signal_turn_end(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
    event: &RuntimeEvent,
    historical_only: bool,
) -> bool {
    if historical_only || is_draining_crashed_epoch(shared, session_id, worker_epoch) {
        return false;
    }
    if !admits_event(shared, session_id, worker_epoch, event) {
        return false;
    }
    let Some(worker) = shared.workers.lock().get(session_id).cloned() else {
        return false;
    };
    if !worker.has_connected_owner() || worker.worker_epoch != worker_epoch {
        return false;
    }
    if let Some(launch) = worker.launch.as_ref()
        && let Some(info) = shared.hub.session_info(session_id)
        && (info.meta.provider != launch.provider
            || info.meta.provider_account_id != launch.provider_account_id
            || info.meta.provider_auth_generation != launch.provider_auth_generation)
    {
        return false;
    }
    true
}

/// Resolve the credential target for `mark_projected_provider_auth_expired`
/// strictly from an exact connected owner's admitted launch identity and
/// populated `provider_auth_generation` — never falls back to mutable Hub
/// metadata (R16-M3, R17-M1).
pub(in crate::remote_runtime) fn provider_auth_expiry_target(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
    event: &RuntimeEvent,
) -> Option<(String, crate::provider_account::ProviderAccountId, u64)> {
    let RuntimeEvent::Status {
        detail: Some(detail),
        ..
    } = event
    else {
        return None;
    };
    if !crate::provider_behavior::is_provider_auth_required_error(detail) {
        return None;
    }
    let worker = shared.workers.lock().get(session_id).cloned()?;
    if !worker.has_connected_owner() || worker.worker_epoch != worker_epoch {
        return None;
    }
    let launch = worker.launch?;
    let generation = launch.provider_auth_generation?;
    Some((launch.provider, launch.provider_account_id, generation))
}
