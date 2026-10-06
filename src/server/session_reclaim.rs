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
