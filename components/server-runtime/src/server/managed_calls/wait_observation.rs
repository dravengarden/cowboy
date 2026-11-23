//! Independent observation of a Call's execution and its parent's notice.
//! The Call runner and transcript remain the owners of their own effects.
use super::super::AppState;
use crate::logs::wait_cache::{self, Fields};
use crate::managed_calls::{lifecycle::Record, review::NOTIFY};

pub fn wants_notice(record: &Record) -> bool {
    record
        .labels
        .get(NOTIFY)
        .is_some_and(|value| value == "parent")
}

pub fn note_admission(
    state: &AppState,
    parent: &str,
    record: &Record,
    request: &str,
    origin: &super::super::background_tasks::CallOrigin,
) {
    let approval = state.managed_calls.approvals.admitted(parent, request);
    super::super::background_tasks::push_call_anchor(
        &state.hub,
        parent,
        &record.call_id,
        &record.request_id,
        origin,
        approval.as_deref(),
    );
    admitted(state, record);
}

pub fn admitted(state: &AppState, record: &Record) {
    let parent = &record.placement.parent_session_id;
    let fields = observation(state, record).string("outcome", "admitted");
    wait_cache::record(
        wait_cache::DISPATCH,
        &[parent, &record.call_id],
        Some(record.created_at_ms),
        fields,
    );
}

pub fn settled(state: &AppState, record: &Record) {
    let parent = &record.placement.parent_session_id;
    let ended = state
        .hub
        .parent_ended_during_wait(parent, &format!("call:{}", record.call_id));
    let wake_needed = state.hub.session_info(parent).and_then(|info| {
        if !wants_notice(record)
            || info.meta.closing
            || info.meta.trashed_at_ms.is_some()
            || state.supervisor.has_live_worker(parent)
        {
            Some(false)
        } else {
            // The native wake is a separate fact from whether the parent
            // ended its turn. A paused parent can still need a later wake.
            (info.meta.status == crate::core::Status::Exited).then_some(true)
        }
    });
    let duration = record
        .finished_at_ms
        .and_then(|finished| u64::try_from(finished.saturating_sub(record.created_at_ms)).ok());
    wait_cache::record(
        wait_cache::SETTLED,
        &[parent, &record.call_id],
        record.finished_at_ms,
        observation(state, record)
            .string("outcome", record.state.code())
            .boolean("parent_ended_turn", ended)
            .boolean("wake_needed", wake_needed)
            .integer("wait_duration_ms", duration)
            .integer("cowboy.duration_ms", duration),
    );
}

fn observation(state: &AppState, record: &Record) -> Fields {
    let parent = &record.placement.parent_session_id;
    let wait_path = if wants_notice(record) {
        "managed_call_notify"
    } else {
        "managed_call_observe"
    };
    let mut fields = Fields::default()
        .string("cowboy.session.id", parent)
        .string("call_id", &record.call_id)
        .string("wait_path", wait_path)
        .string("child_provider", wait_cache::provider(&record.provider));
    if let Some(info) = state.hub.session_info(parent) {
        fields = fields
            .string("provider", wait_cache::provider(&info.meta.provider))
            .boolean("parent_paused", info.meta.paused)
            .integer(
                "verified_context_used",
                state.hub.verified_context_usage(&info.meta),
            );
    }
    fields
}

/// Recheck recent UTC cohorts so a late echo can turn up after Controller
/// restart. A snapshot replaces the previous evidence for the same day.
pub fn spawn_reconciliation(state: &std::sync::Arc<AppState>) {
    use std::time::Duration;
    let Some(store) = state.store.clone() else {
        return;
    };
    let mut shutdown = state.shutdown.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_mins(30));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = shutdown.wait_for(|stopping| *stopping) => return,
                _ = tick.tick() => {}
            }
            let now = chrono::Utc::now().timestamp_millis();
            let day = wait_cache::utc_day(now);
            for offset in 0..7 {
                let from = day.saturating_sub(offset * wait_cache::DAY_MS);
                let to = from + wait_cache::DAY_MS;
                let result = tokio::time::timeout(
                    Duration::from_secs(3),
                    store.reconcile_call_notice_echoes(from, to.min(now)),
                )
                .await;
                let mut fields =
                    Fields::default().integer("snapshot_at_ms", u64::try_from(now).ok());
                if let Ok(Ok(counts)) = result {
                    fields = fields
                        .string(
                            "outcome",
                            if counts.truncated {
                                "truncated"
                            } else {
                                "observed"
                            },
                        )
                        .integer("terminal_calls", counts.terminal_calls)
                        .integer("durable_echoes", counts.durable_echoes)
                        .integer(
                            "echo_unobserved",
                            counts.terminal_calls.saturating_sub(counts.durable_echoes),
                        );
                } else {
                    fields = fields.string("outcome", "unavailable");
                }
                wait_cache::record(
                    wait_cache::RECONCILIATION,
                    &[&from.to_string(), &now.to_string()],
                    Some(from),
                    fields,
                );
            }
        }
    });
}
