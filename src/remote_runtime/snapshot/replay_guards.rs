//! Replay-scope classification, transient-admission retry fences,
//! Hub-persisted event highwaters, projection-guarded turn-end signaling, and
//! launch-scoped credential expiry.

use super::{
    RuntimeEvent, Shared, WorkerSnapshot, WorkerState, admits_event,
    clear_draining_crashed_epoch_if_matches, is_draining_crashed_epoch,
};

/// Classification of an admitted streamed worker event:
/// - `Live`: normal live event from the current owning worker.
/// - `OwningBelowCrashWatermark`: first-delivery replay from the CURRENT
///   owning worker that is already `Crashed` (in `shared.workers`,
///   `draining_replay_snapshots`, or raw `crashed_worker_sightings`) or
///   actively draining, with `runtime_seq < epoch_max_watermark`: preserves
///   first-delivery outcomes (`TurnEnded`, auth expiry, content updates, and
///   non-displacing `SUBAGENT_UPDATE`) without regressing worker/Hub status
///   back to Running/Busy or surfacing stale permissions/wakeups/config
///   (R18-H4, R18-M2, R18-M3).
/// - `NonOwningDraining`: draining replay from an epoch that is resetting or
///   no longer owns `shared.workers`: strict transcript-only projection via
///   `push_historical_from_worker`, never mutating live status, turn-end
///   boundaries, account signals, worker incarnation, attention, compaction,
///   permissions, or native thread ID (R16-H1, R16-H2, R16-M1, R17-M2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::remote_runtime) enum ReplayScope {
    Live,
    OwningBelowCrashWatermark,
    NonOwningDraining,
}

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

/// Validate contiguous sequence ordering on this runtime and deduplicate
/// sequences already processed into the still-running `Hub` before a tunnel
/// replacement (R18-H3). Returns `Ok(true)` when already processed (caller
/// should skip projection and send `Frame::Ack`).
pub(in crate::remote_runtime) fn check_and_dedup_event_sequence(
    shared: &Shared,
    key: &(String, String),
    runtime_seq: u64,
) -> anyhow::Result<bool> {
    let local_previous = shared.highwaters.lock().get(key).copied();
    if let Some(previous) = local_previous
        && runtime_seq > previous.saturating_add(1)
    {
        anyhow::bail!(
            "runtime event gap for {}/{}: have {previous}, got {runtime_seq}",
            key.0,
            key.1
        );
    }
    if local_previous.is_some_and(|previous| runtime_seq <= previous) {
        return Ok(true);
    }
    if shared
        .hub
        .runtime_event_highwater(&key.0, &key.1)
        .is_some_and(|hub_prev| runtime_seq <= hub_prev)
    {
        shared.highwaters.lock().insert(key.clone(), runtime_seq);
        return Ok(true);
    }
    Ok(false)
}

pub(in crate::remote_runtime) fn record_processed_event_sequence(
    shared: &Shared,
    key: (String, String),
    runtime_seq: u64,
) {
    shared
        .hub
        .record_runtime_event_highwater(&key.0, &key.1, runtime_seq);
    shared.highwaters.lock().insert(key, runtime_seq);
}

/// Handle non-admitted events for ordered replay:
/// - `TransientlyDeferred`: withhold ACK and restart ordered replay on EVERY
///   transient admission failure, even after a drain timeout abort cleared the
///   draining marker, so a retryable store outage never drops unprojected
///   events (R17-H1). Hub-persisted highwaters ensure tunnel replacement never
///   re-applies already-projected events from other sessions on the Machine
///   (R18-H3).
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
/// reduction always observe the same watermark (R15-M2, R17-M5, R18-M2).
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

/// Classify an admitted event's replay scope using the exact same lifecycle
/// evidence (`shared.workers`, `draining_replay_snapshots`, and raw
/// `crashed_worker_sightings`) and unified watermark as maintenance reduction
/// (R18-H4, R18-M2).
pub(in crate::remote_runtime) fn classify_replay_scope(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
    runtime_seq: u64,
) -> ReplayScope {
    let resetting = shared.resetting.lock().contains(session_id);
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
    if resetting || (owning_worker.is_none() && drain_replay_snap.is_some()) {
        return ReplayScope::NonOwningDraining;
    }
    let sighting_crashed = shared
        .crashed_worker_sightings
        .lock()
        .get(session_id)
        .is_some_and(|entry| {
            entry.worker_epoch == worker_epoch && entry.state == WorkerState::Crashed
        });
    let crashed_or_draining = is_draining_crashed_epoch(shared, session_id, worker_epoch)
        || owning_worker
            .as_ref()
            .is_some_and(|w| w.state == WorkerState::Crashed)
        || sighting_crashed;
    let max_watermark = epoch_max_watermark(shared, session_id, worker_epoch);
    if crashed_or_draining && runtime_seq > 0 && runtime_seq < max_watermark {
        ReplayScope::OwningBelowCrashWatermark
    } else {
        ReplayScope::Live
    }
}

/// MUST be called while holding `shared.hub.runtime_projection_guard()` so
/// Clear cannot advance the context fence between checking the epoch and
/// registering the account-wide turn-end signal (R17-H2, R18-H2).
/// Allows first-delivery `TurnEnded` from a positively identified current owner
/// (including `OwningBelowCrashWatermark`, R18-H4) whose launch and credential
/// pin match the session, while refusing `NonOwningDraining` or cleared epochs.
pub(in crate::remote_runtime) fn signal_admitted_turn_end_locked(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
    runtime_seq: u64,
    event: &RuntimeEvent,
) {
    if !matches!(event, RuntimeEvent::TurnEnded { .. }) {
        return;
    }
    let scope = classify_replay_scope(shared, session_id, worker_epoch, runtime_seq);
    if scope == ReplayScope::NonOwningDraining {
        return;
    }
    if !admits_event(shared, session_id, worker_epoch, event) {
        return;
    }
    let Some(worker) = shared.workers.lock().get(session_id).cloned() else {
        return;
    };
    if !worker.has_connected_owner() || worker.worker_epoch != worker_epoch {
        return;
    }
    let Some(launch) = worker.launch.as_ref() else {
        return;
    };
    let Some(info) = shared.hub.session_info(session_id) else {
        return;
    };
    if info.meta.provider != launch.provider
        || info.meta.provider_account_id != launch.provider_account_id
        || info.meta.provider_auth_generation != launch.provider_auth_generation
    {
        return;
    }
    crate::remote_runtime::turn_end::signal(shared, session_id, event);
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
