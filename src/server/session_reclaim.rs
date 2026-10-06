//! Free a full Device's session slot on demand (`sessions.reclaim_on_capacity`).
//!
//! The policy only ever hibernates, which sends no model request and keeps the
//! native conversation resumable. It picks the longest-idle session that has
//! been quiet for at least `sessions.reclaim_min_idle`, so a reclaimed
//! session's Provider prompt cache has already expired and resuming it costs
//! no more than leaving it running would have. Busy, permission-waiting,
//! background-working, system and cache-protected sessions are never chosen.

use std::sync::Arc;
use std::time::Duration;

use crate::agent_model::Status;

use super::{AppState, holds_worker_slot};

/// How long a create request waits for the hibernated worker to release its
/// slot before refusing as before.
const SLOT_RELEASE_TIMEOUT: Duration = Duration::from_secs(20);
const SLOT_RELEASE_POLL: Duration = Duration::from_millis(250);

/// What the selection needs to know about one session on the full Device.
#[derive(Debug, Clone)]
pub(super) struct Candidate {
    pub id: String,
    pub idle_for: Duration,
    pub status: Status,
    pub holds_slot: bool,
    pub in_flight: bool,
    pub background_tasks: u32,
    pub system: bool,
    pub cache_protection: bool,
}

/// The longest-idle session that may be hibernated, if any.
pub(super) fn choose(candidates: &[Candidate], min_idle: Duration) -> Option<&str> {
    candidates
        .iter()
        .filter(|candidate| {
            candidate.holds_slot
                && candidate.status == Status::Running
                && !candidate.in_flight
                && candidate.background_tasks == 0
                && !candidate.system
                && !candidate.cache_protection
                && candidate.idle_for >= min_idle
        })
        .max_by_key(|candidate| candidate.idle_for)
        .map(|candidate| candidate.id.as_str())
}

/// After an open has revived `opened`, the session to hibernate so the Device
/// returns to its capacity: only when the Device now holds more workers than
/// `max_sessions`, and never the session that was just opened.
pub(super) fn choose_after_wake<'a>(
    candidates: &'a [Candidate],
    opened: &str,
    max_sessions: usize,
    min_idle: Duration,
) -> Option<&'a str> {
    let active = candidates
        .iter()
        .filter(|candidate| candidate.holds_slot)
        .count();
    if active <= max_sessions {
        return None;
    }
    let others = candidates
        .iter()
        .filter(|candidate| candidate.id != opened)
        .cloned()
        .collect::<Vec<_>>();
    let chosen = choose(&others, min_idle)?.to_owned();
    candidates
        .iter()
        .find(|candidate| candidate.id == chosen)
        .map(|candidate| candidate.id.as_str())
}

/// Opening a hibernated session revives it even on a full Device, because the
/// Machine treats capacity as advisory. With `sessions.reclaim_on_capacity`,
/// hibernate one other eligible idle session afterwards so the slot count
/// returns to the limit. Best effort: it never delays or refuses the open.
pub(super) async fn rebalance_after_wake(state: Arc<AppState>, opened: String) {
    if !state
        .service_config
        .get(&crate::config::schema::SESSIONS_RECLAIM_ON_CAPACITY)
    {
        return;
    }
    let Some(machine_id) = state
        .hub
        .session_list()
        .into_iter()
        .find(|session| session.id == opened)
        .map(|session| session.machine_id)
    else {
        return;
    };
    let Some(store) = state.store.as_ref() else {
        return;
    };
    let Ok(machines) = store.list_machines().await else {
        return;
    };
    let Some(machine) = machines
        .into_iter()
        .find(|machine| machine.id == machine_id && !machine.revoked)
    else {
        return;
    };
    let capacity: crate::machine_protocol::MachineCapacity = machine
        .inventory
        .get("capacity")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let min_idle = state
        .service_config
        .get(&crate::config::schema::SESSIONS_RECLAIM_MIN_IDLE);
    let candidates = candidates(&state, &machine_id);
    let Some(chosen) = choose_after_wake(
        &candidates,
        &opened,
        capacity.max_sessions as usize,
        min_idle,
    )
    .map(str::to_owned) else {
        return;
    };
    match state.supervisor.hibernate_session(&chosen) {
        Ok(()) => tracing::info!(machine = %machine_id, opened = %opened, session = %chosen,
            "hibernating the longest-idle session to return a woken Device to capacity"),
        Err(error) => tracing::warn!(machine = %machine_id, session = %chosen, %error,
            "could not hibernate a session after a wake exceeded capacity"),
    }
}

fn candidates(state: &AppState, machine_id: &str) -> Vec<Candidate> {
    state
        .hub
        .session_list()
        .into_iter()
        .filter(|session| session.machine_id == machine_id)
        .filter_map(|session| {
            let idle_for = state.hub.session_idle_for(&session.id)?;
            let configuration = session.provider_behavior.as_ref().map_or_else(
                || crate::provider::legacy_behavior(&session.provider).configuration,
                |behavior| behavior.configuration.clone(),
            );
            let preferences = state
                .hub
                .config_preferences(&session.id)
                .unwrap_or_else(|| serde_json::json!({}));
            Some(Candidate {
                holds_slot: holds_worker_slot(&state.runtime_router, &session),
                in_flight: state.hub.session_has_in_flight_prompt(&session.id),
                cache_protection: crate::managed_config::cache_protection(
                    &configuration,
                    &preferences,
                )
                .unwrap_or(false),
                id: session.id,
                idle_for,
                status: session.status,
                background_tasks: session.background_tasks,
                system: session.system,
            })
        })
        .collect()
}

/// Try to free one slot on `machine_id`. Returns whether a slot was released
/// within the timeout; on `false` the caller refuses exactly as before.
pub(super) async fn reclaim_slot(state: &Arc<AppState>, machine_id: &str) -> bool {
    let min_idle = state
        .service_config
        .get(&crate::config::schema::SESSIONS_RECLAIM_MIN_IDLE);
    let candidates = candidates(state, machine_id);
    let Some(chosen) = choose(&candidates, min_idle).map(str::to_owned) else {
        tracing::info!(machine = %machine_id, "Device is full and no session is idle enough to reclaim");
        return false;
    };
    if let Err(error) = state.supervisor.hibernate_session(&chosen) {
        tracing::warn!(machine = %machine_id, session = %chosen, %error, "could not hibernate a session to free a slot");
        return false;
    }
    tracing::info!(machine = %machine_id, session = %chosen, "hibernating the longest-idle session to free a slot");
    let deadline = tokio::time::Instant::now() + SLOT_RELEASE_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        let released = state
            .hub
            .session_list()
            .into_iter()
            .find(|session| session.id == chosen)
            .is_none_or(|session| !holds_worker_slot(&state.runtime_router, &session));
        if released {
            return true;
        }
        tokio::time::sleep(SLOT_RELEASE_POLL).await;
    }
    tracing::warn!(machine = %machine_id, session = %chosen, "hibernated session did not release its slot in time");
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle(id: &str, minutes: u64) -> Candidate {
        Candidate {
            id: id.to_owned(),
            idle_for: Duration::from_mins(minutes),
            status: Status::Running,
            holds_slot: true,
            in_flight: false,
            background_tasks: 0,
            system: false,
            cache_protection: false,
        }
    }

    #[test]
    fn picks_the_longest_idle_session_past_the_threshold() {
        let hour = Duration::from_hours(1);
        let sessions = [idle("recent", 10), idle("old", 300), idle("older", 90)];
        assert_eq!(choose(&sessions, hour), Some("old"));
        assert_eq!(choose(&[idle("recent", 59)], hour), None);
        assert_eq!(choose(&[], hour), None);
    }

    #[test]
    fn a_wake_reclaims_only_when_over_capacity_and_never_the_opened_session() {
        let hour = Duration::from_hours(1);
        let opened = Candidate {
            status: Status::Starting,
            ..idle("opened", 900)
        };
        let sessions = [opened, idle("old", 300), idle("older", 600)];
        // Three workers against a limit of three: nothing to do.
        assert_eq!(choose_after_wake(&sessions, "opened", 3, hour), None);
        // Over the limit: the longest-idle other session, never the one opened.
        assert_eq!(
            choose_after_wake(&sessions, "opened", 2, hour),
            Some("older")
        );
        let only_opened = [Candidate {
            status: Status::Running,
            ..idle("opened", 900)
        }];
        assert_eq!(choose_after_wake(&only_opened, "opened", 0, hour), None);
    }

    #[test]
    fn never_picks_a_session_that_would_lose_work_or_cost_tokens() {
        let hour = Duration::from_hours(1);
        let excluded = [
            Candidate {
                status: Status::Busy,
                ..idle("busy", 600)
            },
            Candidate {
                status: Status::Starting,
                ..idle("starting", 600)
            },
            Candidate {
                holds_slot: false,
                ..idle("hibernated", 600)
            },
            Candidate {
                in_flight: true,
                ..idle("queued", 600)
            },
            Candidate {
                background_tasks: 1,
                ..idle("background", 600)
            },
            Candidate {
                system: true,
                ..idle("system", 600)
            },
            Candidate {
                cache_protection: true,
                ..idle("protected", 600)
            },
        ];
        assert_eq!(choose(&excluded, hour), None);
        let mut with_eligible = excluded.to_vec();
        with_eligible.push(idle("eligible", 61));
        assert_eq!(choose(&with_eligible, hour), Some("eligible"));
    }
}
