//! Free a full Device's session slot on demand (`sessions.reclaim_on_capacity`).
//!
//! The policy only ever hibernates, which sends no model request and keeps the
//! native conversation resumable. It picks the longest-idle session that has
//! been quiet for at least `sessions.reclaim_min_idle`. The next prompt after
//! a wake is not proven to cost the same as after a warm idle: see
//! docs/configuration.md. Busy, permission-waiting, background-working,
//! system and cache-protected sessions are never chosen.

use std::sync::Arc;
use std::time::Duration;

use crate::agent_model::Status;

use super::{AppState, holds_worker_slot};

/// How long a create request waits for the hibernated worker to release its
/// slot before refusing as before.
const SLOT_RELEASE_TIMEOUT: Duration = Duration::from_secs(20);
const SLOT_RELEASE_POLL: Duration = Duration::from_millis(250);
/// A client restoring its tabs opens several dormant sessions within a
/// second. Rebalancing waits this long so the whole burst is known, and never
/// hibernates a session woken within `WAKE_PROTECTION`: on 2026-10-14 three
/// restored tabs each hibernated the next one being opened.
const WAKE_SETTLE: Duration = Duration::from_secs(5);
const WAKE_PROTECTION: Duration = Duration::from_mins(10);

static RECENT_WAKES: std::sync::LazyLock<
    parking_lot::Mutex<std::collections::HashMap<String, std::time::Instant>>,
> = std::sync::LazyLock::new(Default::default);

/// Sessions whose wake already has a rebalance in flight.
static PENDING: std::sync::LazyLock<parking_lot::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);

struct Pending(String);

impl Drop for Pending {
    fn drop(&mut self) {
        PENDING.lock().remove(&self.0);
    }
}

/// Record a wake and return every session woken within `WAKE_PROTECTION`.
pub(super) fn recent_wakes(woken: Option<&str>) -> std::collections::HashSet<String> {
    let now = std::time::Instant::now();
    let mut wakes = RECENT_WAKES.lock();
    if let Some(woken) = woken {
        wakes.insert(woken.to_owned(), now);
    }
    wakes.retain(|_, at| now.duration_since(*at) < WAKE_PROTECTION);
    wakes.keys().cloned().collect()
}

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

/// After an open has revived a session, the session to hibernate so the Device
/// returns to its capacity: only when the Device now holds more workers than
/// `max_sessions`, and never a session in `woken` (the one just opened and any
/// other opened recently).
pub(super) fn choose_after_wake<'a>(
    candidates: &'a [Candidate],
    woken: &std::collections::HashSet<String>,
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
        .filter(|candidate| !woken.contains(&candidate.id))
        .cloned()
        .collect::<Vec<_>>();
    let chosen = choose(&others, min_idle)?.to_owned();
    candidates
        .iter()
        .find(|candidate| candidate.id == chosen)
        .map(|candidate| candidate.id.as_str())
}

/// Rebalance after every wake a prompt causes. The listener holds the state
/// weakly: the Supervisor it is installed on is owned by that state.
pub(super) fn listen_for_wakes(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
    state
        .supervisor
        .set_wake_listener(Arc::new(move |session_id: &str| {
            if let Some(state) = weak.upgrade() {
                tokio::spawn(rebalance_after_wake(state, session_id.to_owned()));
            }
        }));
}

/// Waking a hibernated session revives it even on a full Device, because the
/// Machine treats capacity as advisory. With `sessions.reclaim_on_capacity`,
/// hibernate one other eligible idle session afterwards so the slot count
/// returns to the limit. Best effort: it never delays or refuses the open.
pub(super) async fn rebalance_after_wake(state: Arc<AppState>, opened: String) {
    // Two clients, or a prompt racing an open, can wake the same session
    // together; one rebalance covers it (production logged every
    // hibernation twice).
    if !PENDING.lock().insert(opened.clone()) {
        return;
    }
    let _pending = Pending(opened.clone());
    recent_wakes(Some(&opened));
    tokio::time::sleep(WAKE_SETTLE).await;
    let Some(machine_id) = state
        .hub
        .session_list()
        .into_iter()
        .find(|session| session.id == opened)
        .map(|session| session.machine_id)
    else {
        return;
    };
    let decisions = decisions(&machine_id);
    let (chosen, _decision) = loop {
        // Policy and capacity are read on every pass: a retry may follow a
        // ten-minute wait during which an operator changed them.
        if !reclaim_enabled(&state, &machine_id) {
            return;
        }
        let Some(max_sessions) = device_capacity(&state, &machine_id).await else {
            return;
        };
        let decision = decisions.lock().await;
        // Creates reserved but not registered yet hold slots too.
        let max_sessions = max_sessions.saturating_sub(reserved(&machine_id));
        let min_idle = state
            .service_config
            .get(&crate::config::schema::SESSIONS_RECLAIM_MIN_IDLE);
        let candidates = candidates(&state, &machine_id);
        // Taken last, so a session woken while the store was read is known.
        let woken = recent_wakes(None);
        if let Some(chosen) = choose_after_wake(&candidates, &woken, max_sessions, min_idle) {
            break (chosen.to_owned(), decision);
        }
        // Over capacity with only recently woken sessions eligible: try again
        // once their protection has expired, rather than staying over it.
        let unprotected = std::collections::HashSet::new();
        if choose_after_wake(&candidates, &unprotected, max_sessions, min_idle).is_none() {
            return;
        }
        drop(decision);
        tokio::time::sleep(WAKE_PROTECTION).await;
    };
    match state.supervisor.hibernate_session(&chosen) {
        Ok(()) => tracing::info!(machine = %machine_id, opened = %opened, session = %chosen,
            "hibernating the longest-idle session to return a woken Device to capacity"),
        Err(error) => tracing::warn!(machine = %machine_id, session = %chosen, %error,
            "could not hibernate a session after a wake exceeded capacity"),
    }
}

fn reclaim_enabled(state: &AppState, machine_id: &str) -> bool {
    super::session_maintenance::policy(&state.hub, &state.service_config, machine_id)
        .reclaim_on_capacity
        == Some(true)
}

/// The enrolled Device's current `max_sessions`, if it is still enrolled.
pub(super) async fn device_capacity(state: &AppState, machine_id: &str) -> Option<usize> {
    let machines = state.store.as_ref()?.list_machines().await.ok()?;
    let machine = machines
        .into_iter()
        .find(|machine| machine.id == machine_id && !machine.revoked)?;
    let capacity: crate::machine_protocol::MachineCapacity = machine
        .inventory
        .get("capacity")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    Some(capacity.max_sessions as usize)
}

pub(super) fn candidates(state: &AppState, machine_id: &str) -> Vec<Candidate> {
    candidates_in(&state.hub, &state.runtime_router, machine_id)
}

/// `candidates` for callers that hold the Hub and router but no `AppState`.
pub(super) fn candidates_in(
    hub: &crate::core::Hub,
    router: &crate::runtime_router::RuntimeRouter,
    machine_id: &str,
) -> Vec<Candidate> {
    hub.session_list()
        .into_iter()
        .filter(|session| session.machine_id == machine_id)
        .filter_map(|session| {
            let idle_for = hub.session_idle_for(&session.id)?;
            let configuration = session.provider_behavior.as_ref().map_or_else(
                || crate::provider::legacy_behavior(&session.provider).configuration,
                |behavior| behavior.configuration.clone(),
            );
            let preferences = hub
                .config_preferences(&session.id)
                .unwrap_or_else(|| serde_json::json!({}));
            Some(Candidate {
                holds_slot: holds_worker_slot(router, &session),
                in_flight: hub.session_has_in_flight_prompt(&session.id),
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

/// How many hibernations one create may trigger before it is refused: each
/// may lose its slot to a concurrent wake.
const MAX_RECLAIMS_PER_CREATE: usize = 2;
/// Idle sessions a capacity refusal names.
const REFUSAL_IDLE_LISTED: usize = 5;

/// Creates admitted on each Machine whose session is not registered yet.
static RESERVED: std::sync::LazyLock<parking_lot::Mutex<std::collections::HashMap<String, usize>>> =
    std::sync::LazyLock::new(Default::default);
/// Serialises each Machine's slot decisions: a create's count, its reclaim
/// and its reservation, and a wake's rebalance. Without it two creates could
/// both see the last free slot, or both count one hibernated victim.
static DECISIONS: std::sync::LazyLock<
    parking_lot::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(Default::default);

pub(super) fn decisions(machine_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    Arc::clone(DECISIONS.lock().entry(machine_id.to_owned()).or_default())
}

fn reserved(machine_id: &str) -> usize {
    RESERVED.lock().get(machine_id).copied().unwrap_or(0)
}

/// A worker slot held for a create until its session is registered (as
/// `Starting`, which then holds the slot itself). Dropping it releases the
/// reservation, whether the create succeeded or failed.
pub(super) struct SlotReservation {
    machine_id: String,
}

impl Drop for SlotReservation {
    fn drop(&mut self) {
        let mut reserved = RESERVED.lock();
        if let Some(count) = reserved.get_mut(&self.machine_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                reserved.remove(&self.machine_id);
            }
        }
    }
}

/// Live worker slots on `machine_id`: sessions holding a worker plus creates
/// reserved but not yet registered. The reservations stay locked while the
/// sessions are read: a create registers as `Starting` before it drops its
/// reservation, so it is counted at least once (at worst twice, which only
/// refuses or reclaims conservatively for that instant), never zero times.
pub(super) fn occupied(state: &AppState, machine_id: &str) -> usize {
    let reserved = RESERVED.lock();
    let live = state
        .hub
        .session_list()
        .iter()
        .filter(|session| {
            session.machine_id == machine_id && holds_worker_slot(&state.runtime_router, session)
        })
        .count();
    live + reserved.get(machine_id).copied().unwrap_or(0)
}

/// Whether a full Machine could still take a session: capacity reclaim is on
/// and some session is idle enough to hibernate.
pub(super) fn reclaimable(
    reclaim_on_capacity: bool,
    min_idle: Duration,
    candidates: &[Candidate],
) -> bool {
    reclaim_on_capacity && choose(candidates, min_idle).is_some()
}

/// Reserve one live worker slot on `machine_id` for a create. Counts only
/// sessions holding a worker (and other pending creates), never dormant
/// history. On a full Machine with `sessions.reclaim_on_capacity`, hibernates
/// the longest-idle eligible session and counts again. A refusal names the
/// idle sessions the user could hibernate.
pub(super) async fn reserve_slot(
    state: &Arc<AppState>,
    machine_id: &str,
    capacity: &crate::machine_protocol::MachineCapacity,
) -> Result<SlotReservation, String> {
    if capacity.draining {
        return Err(format!("machine {machine_id:?} is draining or at capacity"));
    }
    let max_sessions = capacity.max_sessions as usize;
    let decisions = decisions(machine_id);
    let _decision = decisions.lock().await;
    let mut reclaims = 0;
    loop {
        if !super::session_maintenance::has_waiters(state, machine_id) {
            return Err("No messages are waiting for resources".into());
        }
        if occupied(state, machine_id) < max_sessions {
            *RESERVED.lock().entry(machine_id.to_owned()).or_default() += 1;
            return Ok(SlotReservation {
                machine_id: machine_id.to_owned(),
            });
        }
        if reclaims == MAX_RECLAIMS_PER_CREATE
            || !reclaim_enabled(state, machine_id)
            || !reclaim_slot(state, machine_id).await
        {
            return Err(capacity_refusal(state, machine_id, max_sessions));
        }
        reclaims += 1;
    }
}

fn capacity_refusal(state: &AppState, machine_id: &str, max_sessions: usize) -> String {
    let mut idle = candidates(state, machine_id)
        .into_iter()
        .filter(|candidate| {
            candidate.holds_slot
                && candidate.status == Status::Running
                && !candidate.in_flight
                && !candidate.system
        })
        .collect::<Vec<_>>();
    idle.sort_by_key(|candidate| std::cmp::Reverse(candidate.idle_for));
    let titles = state
        .hub
        .session_list()
        .into_iter()
        .map(|session| (session.id, session.title))
        .collect::<std::collections::HashMap<_, _>>();
    let listed = idle
        .iter()
        .take(REFUSAL_IDLE_LISTED)
        .map(|candidate| {
            let title = titles
                .get(&candidate.id)
                .map_or(candidate.id.as_str(), String::as_str);
            format!("{title:?} (idle {}m)", candidate.idle_for.as_secs() / 60)
        })
        .collect::<Vec<_>>();
    let mut message =
        format!("machine {machine_id:?} is draining or at capacity: {max_sessions} live sessions");
    if listed.is_empty() {
        message.push_str("; none is idle, so wait for one to finish");
    } else {
        message.push_str("; hibernate an idle one to free a slot: ");
        message.push_str(&listed.join(", "));
    }
    message
}

/// Try to free one slot on `machine_id`. Returns whether a slot was released
/// within the timeout; on `false` the caller refuses exactly as before.
pub(super) async fn reclaim_slot(state: &Arc<AppState>, machine_id: &str) -> bool {
    let min_idle = state
        .service_config
        .get(&crate::config::schema::SESSIONS_RECLAIM_MIN_IDLE);
    let woken = recent_wakes(None);
    let candidates = candidates(state, machine_id)
        .into_iter()
        .filter(|candidate| !woken.contains(&candidate.id))
        .collect::<Vec<_>>();
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

    #[test]
    fn a_reservation_counts_until_dropped() {
        let machine = "reservation-test-machine";
        let take = || {
            *RESERVED.lock().entry(machine.to_owned()).or_default() += 1;
            SlotReservation {
                machine_id: machine.to_owned(),
            }
        };
        let first = take();
        let second = take();
        assert_eq!(reserved(machine), 2);
        drop(first);
        assert_eq!(reserved(machine), 1);
        drop(second);
        assert_eq!(reserved(machine), 0);
        assert!(!RESERVED.lock().contains_key(machine));
    }

    #[test]
    fn a_full_machine_is_schedulable_only_with_something_to_reclaim() {
        let hour = Duration::from_hours(1);
        let sessions = [idle("fresh", 10), idle("stale", 90)];
        assert!(reclaimable(true, hour, &sessions));
        assert!(!reclaimable(false, hour, &sessions));
        assert!(!reclaimable(true, hour, &sessions[..1]));
    }

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
    fn a_wake_reclaims_only_when_over_capacity_and_never_a_recently_woken_session() {
        let hour = Duration::from_hours(1);
        let woken = |ids: &[&str]| {
            ids.iter()
                .map(|id| (*id).to_owned())
                .collect::<std::collections::HashSet<_>>()
        };
        let opened = Candidate {
            status: Status::Starting,
            ..idle("opened", 900)
        };
        let sessions = [opened, idle("old", 300), idle("older", 600)];
        // Three workers against a limit of three: nothing to do.
        assert_eq!(
            choose_after_wake(&sessions, &woken(&["opened"]), 3, hour),
            None
        );
        // Over the limit: the longest-idle other session, never the one opened.
        assert_eq!(
            choose_after_wake(&sessions, &woken(&["opened"]), 2, hour),
            Some("older")
        );
        // A restored tab opened just before is protected too.
        assert_eq!(
            choose_after_wake(&sessions, &woken(&["opened", "older"]), 2, hour),
            Some("old")
        );
        let only_opened = [Candidate {
            status: Status::Running,
            ..idle("opened", 900)
        }];
        assert_eq!(
            choose_after_wake(&only_opened, &woken(&["opened"]), 0, hour),
            None
        );
    }

    #[test]
    fn recent_wakes_are_remembered_for_the_protection_window() {
        let id = "sess-recent-wake-test";
        assert!(recent_wakes(Some(id)).contains(id));
        assert!(recent_wakes(None).contains(id));
        RECENT_WAKES
            .lock()
            .insert(id.to_owned(), std::time::Instant::now() - WAKE_PROTECTION);
        assert!(!recent_wakes(None).contains(id));
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
