//! Keep each Device's Agent workers below the cgroup limit that contains them
//! (`sessions.memory_reclaim`).
//!
//! A Device reports `memory_headroom_bytes` with every heartbeat. When it falls
//! below `memory_reclaim_below`, the Service hibernates the longest-idle
//! eligible session on that Device, waits for a report taken after the worker
//! released its memory, and repeats until the headroom reaches
//! `memory_reclaim_until`. One session at a time: each decision uses a sample
//! that already reflects the previous one. Hibernation sends no model request,
//! and an opened dormant session shows its history without waking.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{AppState, holds_worker_slot, session_reclaim};
use crate::config::schema::{
    SESSIONS_MEMORY_RECLAIM, SESSIONS_MEMORY_RECLAIM_BELOW, SESSIONS_MEMORY_RECLAIM_MIN_IDLE,
    SESSIONS_MEMORY_RECLAIM_UNTIL,
};

const TICK: Duration = Duration::from_secs(5);
/// A report older than this no longer describes the Device; act on none.
const SAMPLE_MAX_AGE: Duration = Duration::from_secs(60);
/// How long a hibernation may take to release its worker before the next
/// decision proceeds anyway on a fresh sample.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(60);
/// A Device with pressure and nothing eligible is reported at most this often.
const STUCK_LOG_INTERVAL: Duration = Duration::from_mins(10);

#[derive(Default)]
struct DeviceState {
    reclaiming: bool,
    pending: Option<Pending>,
    stuck_logged: Option<Instant>,
}

struct Pending {
    session: String,
    at: Instant,
    /// The report the decision was taken on.
    observed_at_ms: i64,
    /// The newest report when the worker was seen released; the memory it
    /// freed shows only in a report taken after that.
    released_at_report: Option<i64>,
}

/// Whether the previous hibernation is reflected in `observed_at_ms`: its
/// worker has released (or the release timed out) and a report taken since
/// has arrived.
fn pending_settled(pending: &mut Pending, released: bool, observed_at_ms: i64) -> bool {
    if !released {
        return pending.at.elapsed() >= RELEASE_TIMEOUT && observed_at_ms > pending.observed_at_ms;
    }
    match pending.released_at_report {
        None => {
            pending.released_at_report = Some(observed_at_ms);
            false
        }
        Some(report) => observed_at_ms > report,
    }
}

/// What one tick decides for one Device.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Idle,
    Wait,
    Hibernate(String),
    Stuck,
}

struct Policy {
    below: u64,
    until: u64,
}

impl Policy {
    /// Hysteresis: start below `below`, keep going until `until`.
    fn reclaiming(&self, was: bool, headroom: u64) -> bool {
        if was {
            headroom < self.until
        } else {
            headroom < self.below
        }
    }
}

pub(super) fn start(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        let mut devices: HashMap<String, DeviceState> = HashMap::new();
        loop {
            tokio::time::sleep(TICK).await;
            let Some(state) = weak.upgrade() else { return };
            if !state.service_config.get(&SESSIONS_MEMORY_RECLAIM) {
                devices.clear();
                continue;
            }
            tick(&state, &mut devices);
        }
    });
}

fn tick(state: &Arc<AppState>, devices: &mut HashMap<String, DeviceState>) {
    let below = state.service_config.get(&SESSIONS_MEMORY_RECLAIM_BELOW);
    let policy = Policy {
        below,
        until: state
            .service_config
            .get(&SESSIONS_MEMORY_RECLAIM_UNTIL)
            .max(below),
    };
    let min_idle = state.service_config.get(&SESSIONS_MEMORY_RECLAIM_MIN_IDLE);
    let now_ms = super::now_ms();
    for (machine_id, observed) in state.machine_control.all_host_resources() {
        let Some(headroom) = observed.resources.memory_headroom_bytes else {
            continue;
        };
        let age =
            Duration::from_millis(u64::try_from(now_ms - observed.observed_at_ms).unwrap_or(0));
        if age > SAMPLE_MAX_AGE {
            continue;
        }
        let device = devices.entry(machine_id.clone()).or_default();
        let step = decide(
            state,
            &machine_id,
            device,
            &policy,
            headroom,
            observed.observed_at_ms,
            min_idle,
        );
        match step {
            Step::Idle | Step::Wait => {}
            Step::Stuck => {
                if device
                    .stuck_logged
                    .is_none_or(|at| at.elapsed() >= STUCK_LOG_INTERVAL)
                {
                    device.stuck_logged = Some(Instant::now());
                    tracing::warn!(machine = %machine_id, headroom_bytes = headroom,
                        "Device memory is low and no session is idle enough to hibernate");
                }
            }
            Step::Hibernate(session) => match state.supervisor.hibernate_session(&session) {
                Ok(()) => {
                    tracing::info!(machine = %machine_id, session = %session, headroom_bytes = headroom,
                        "hibernating the longest-idle session to relieve Device memory");
                    device.pending = Some(Pending {
                        session,
                        at: Instant::now(),
                        observed_at_ms: observed.observed_at_ms,
                        released_at_report: None,
                    });
                }
                Err(error) => {
                    tracing::warn!(machine = %machine_id, session = %session, %error,
                        "could not hibernate a session to relieve Device memory");
                }
            },
        }
    }
}

fn decide(
    state: &AppState,
    machine_id: &str,
    device: &mut DeviceState,
    policy: &Policy,
    headroom: u64,
    observed_at_ms: i64,
    min_idle: Duration,
) -> Step {
    if let Some(pending) = device.pending.as_mut() {
        let released = state
            .hub
            .session_list()
            .into_iter()
            .find(|session| session.id == pending.session)
            .is_none_or(|session| !holds_worker_slot(&state.runtime_router, &session));
        if !pending_settled(pending, released, observed_at_ms) {
            return Step::Wait;
        }
        device.pending = None;
    }
    device.reclaiming = policy.reclaiming(device.reclaiming, headroom);
    if !device.reclaiming {
        device.stuck_logged = None;
        return Step::Idle;
    }
    let woken = session_reclaim::recent_wakes(None);
    let candidates = session_reclaim::candidates(state, machine_id)
        .into_iter()
        .filter(|candidate| !woken.contains(&candidate.id))
        .collect::<Vec<_>>();
    match session_reclaim::choose(&candidates, min_idle) {
        Some(session) => Step::Hibernate(session.to_owned()),
        None => Step::Stuck,
    }
}

#[cfg(test)]
mod tests {
    use super::{Pending, Policy, pending_settled};
    use std::time::Instant;

    #[test]
    fn the_next_decision_waits_for_a_report_taken_after_the_release() {
        let mut pending = Pending {
            session: "s".to_owned(),
            at: Instant::now(),
            observed_at_ms: 100,
            released_at_report: None,
        };
        // Still holding its worker: wait.
        assert!(!pending_settled(&mut pending, false, 200));
        // Released, but the latest report may predate the release.
        assert!(!pending_settled(&mut pending, true, 200));
        assert!(!pending_settled(&mut pending, true, 200));
        // A report taken afterwards reflects the freed memory.
        assert!(pending_settled(&mut pending, true, 215));
    }

    #[test]
    fn reclaim_starts_below_and_continues_until_the_upper_bound() {
        let policy = Policy {
            below: 1_000,
            until: 2_000,
        };
        assert!(!policy.reclaiming(false, 1_500));
        assert!(policy.reclaiming(false, 999));
        assert!(policy.reclaiming(true, 1_500));
        assert!(!policy.reclaiming(true, 2_000));
    }
}
