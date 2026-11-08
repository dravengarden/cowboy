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

use super::provider_accounts::browser::coordinator::Observed;
use super::{AppState, holds_worker_slot, session_reclaim};
use crate::config::schema::{
    SESSIONS_MEMORY_RECLAIM_BELOW, SESSIONS_MEMORY_RECLAIM_MIN_IDLE, SESSIONS_MEMORY_RECLAIM_UNTIL,
};
use crate::machine_protocol::machine_browser::State as BrowserState;

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
    /// A browser told to hibernate; settles only on its confirmed exit and
    /// a report taken since. It never times out into "released".
    browser_pending: Option<BrowserPending>,
    stuck_logged: Option<Instant>,
}

struct BrowserPending {
    account: String,
    incarnation: u64,
    /// The connection the targeted instance was observed on.
    connection: crate::machine_control::ConnectionToken,
    at: Instant,
    /// The newest report when the instance was first seen exited; the
    /// memory it freed shows only in a report taken after that.
    exited_at_report: Option<i64>,
    _decision: Option<tokio::sync::OwnedMutexGuard<()>>,
}

/// Whether the targeted browser instance has verifiably exited: hibernated
/// for that incarnation, the Machine's authoritative "nothing runs", or a
/// newer instance that replaced it. `stopping` is not an exit: the instance
/// is still copying its snapshot and holding its memory.
fn browser_exited(
    incarnation: u64,
    connection: &crate::machine_control::ConnectionToken,
    current: Option<&Observed>,
) -> bool {
    let Some(observed) = current else {
        return false;
    };
    let event = &observed.event;
    if event.instance.incarnation > incarnation {
        return true;
    }
    if !observed.connection.same(connection) {
        // A later connection re-announces what survived it.
        return !(observed.live() && event.instance.incarnation == incarnation);
    }
    event.state == BrowserState::Hibernated
        && (event.instance.incarnation == incarnation || event.instance.instance_id.is_empty())
}

/// Whether the browser's exit is reflected in `observed_at_ms`: it exited
/// and a report taken after the exit was seen has arrived.
fn browser_settled(pending: &mut BrowserPending, exited: bool, observed_at_ms: i64) -> bool {
    if !exited {
        return false;
    }
    match pending.exited_at_report {
        None => {
            pending.exited_at_report = Some(observed_at_ms);
            false
        }
        Some(report) => observed_at_ms > report,
    }
}

struct Pending {
    session: String,
    at: Instant,
    /// The report the decision was taken on.
    observed_at_ms: i64,
    /// The newest report when the worker was seen released; the memory it
    /// freed shows only in a report taken after that.
    released_at_report: Option<i64>,
    /// Keep wakes and upgrades out until the release is observed.
    _decision: Option<tokio::sync::OwnedMutexGuard<()>>,
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
    /// Running browsers hibernate before any session is selected.
    HibernateBrowser(Box<Observed>),
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
        if super::session_maintenance::policy(&state.hub, &state.service_config, &machine_id)
            .memory_reclaim
            != Some(true)
        {
            devices.remove(&machine_id);
            continue;
        }
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
            Step::HibernateBrowser(observed) => {
                let Ok(decision) = session_reclaim::decisions(&machine_id).try_lock_owned() else {
                    continue;
                };
                let account = observed.event.instance.account_id.clone();
                tracing::info!(machine = %machine_id, headroom_bytes = headroom,
                    "hibernating a Machine browser to relieve Device memory");
                device.browser_pending = Some(BrowserPending {
                    account: account.as_str().to_owned(),
                    incarnation: observed.event.instance.incarnation,
                    connection: observed.connection.clone(),
                    at: Instant::now(),
                    exited_at_report: None,
                    _decision: Some(decision),
                });
                let state = Arc::clone(state);
                tokio::spawn(async move {
                    if let Err(error) =
                        super::provider_accounts::browser::hibernate_running(&state, &account).await
                    {
                        tracing::warn!(%error, "memory reclaim browser hibernate failed");
                    }
                });
            }
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
            Step::Hibernate(session) => {
                let Ok(decision) = session_reclaim::decisions(&machine_id).try_lock_owned() else {
                    continue;
                };
                match state.supervisor.hibernate_session(&session) {
                    Ok(()) => {
                        tracing::info!(machine = %machine_id, session = %session, headroom_bytes = headroom,
                        "hibernating the longest-idle session to relieve Device memory");
                        device.pending = Some(Pending {
                            session,
                            at: Instant::now(),
                            observed_at_ms: observed.observed_at_ms,
                            released_at_report: None,
                            _decision: Some(decision),
                        });
                    }
                    Err(error) => {
                        tracing::warn!(machine = %machine_id, session = %session, %error,
                        "could not hibernate a session to relieve Device memory");
                    }
                }
            }
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
    if let Some(pending) = device.browser_pending.as_mut() {
        let exited = browser_exited(
            pending.incarnation,
            &pending.connection,
            state.browsers.state(&pending.account).as_ref(),
        );
        // A stalled browser stop stays pending and blocks session selection;
        // only its verified exit and a sample taken after it settle it.
        if !browser_settled(pending, exited, observed_at_ms) {
            if !exited && pending.at.elapsed() >= RELEASE_TIMEOUT {
                return Step::Stuck;
            }
            return Step::Wait;
        }
        device.browser_pending = None;
    }
    device.reclaiming = policy.reclaiming(device.reclaiming, headroom);
    if !device.reclaiming {
        device.stuck_logged = None;
        return Step::Idle;
    }
    if let Some(browser) = state
        .browsers
        .running_on(machine_id)
        .into_iter()
        .find(|observed| state.machine_control.is_current(&observed.connection))
    {
        return Step::HibernateBrowser(Box::new(browser));
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
    use super::{
        BrowserPending, Observed, Pending, Policy, browser_exited, browser_settled, pending_settled,
    };
    use crate::machine_control::{ConnectionToken, MachineControl};
    use crate::machine_protocol::machine_browser::{InstanceRef, State, StateEvent};
    use std::time::Instant;

    fn connection(control: &MachineControl, epoch: &str) -> ConnectionToken {
        control.install(
            "ovh".into(),
            epoch.into(),
            false,
            crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
            tokio::sync::mpsc::unbounded_channel().0,
        )
    }

    fn observed(connection: &ConnectionToken, incarnation: u64, state: State) -> Observed {
        Observed {
            machine_id: "ovh".into(),
            connection: connection.clone(),
            event: StateEvent {
                request_id: None,
                instance: InstanceRef {
                    account_id: "11111111-2222-4333-8444-555555555555"
                        .to_owned()
                        .try_into()
                        .unwrap(),
                    instance_id: if incarnation == 0 {
                        String::new()
                    } else {
                        "b-1".into()
                    },
                    incarnation,
                },
                state,
                reason: None,
                geometry: None,
                scale: None,
                deadline_ms: None,
                attachment_id: None,
                snapshot_generation: None,
                snapshot_at_ms: None,
                profile_revision: Some(1),
                unsaved_state_lost: false,
                observed_at_ms: 1,
            },
            observed_at: Instant::now(),
        }
    }

    #[test]
    fn a_browser_counts_as_exited_only_once_hibernated() {
        let control = MachineControl::default();
        let first = connection(&control, "one");
        let exited = |current: Option<Observed>| browser_exited(7, &first, current.as_ref());
        assert!(!exited(None));
        assert!(!exited(Some(observed(&first, 7, State::Detached))));
        // Stopping still copies the snapshot and holds the memory.
        assert!(!exited(Some(observed(&first, 7, State::Stopping))));
        assert!(exited(Some(observed(&first, 7, State::Hibernated))));
        // The Machine's authoritative "nothing runs", or a replacement.
        assert!(exited(Some(observed(&first, 0, State::Hibernated))));
        assert!(exited(Some(observed(&first, 8, State::Starting))));
        // A reconnected Machine re-announces a survivor as running.
        let second = connection(&control, "two");
        assert!(!exited(Some(observed(&second, 7, State::Detached))));
        assert!(exited(Some(observed(&second, 0, State::Hibernated))));
    }

    #[test]
    fn browser_reclaim_settles_on_a_report_taken_after_the_exit() {
        let control = MachineControl::default();
        let mut pending = BrowserPending {
            account: "a".into(),
            incarnation: 7,
            connection: connection(&control, "one"),
            at: Instant::now(),
            exited_at_report: None,
            _decision: None,
        };
        // Not exited yet: even a much newer report settles nothing.
        assert!(!browser_settled(&mut pending, false, 500));
        // Exited, but the newest report may predate the exit.
        assert!(!browser_settled(&mut pending, true, 500));
        assert!(!browser_settled(&mut pending, true, 500));
        assert!(browser_settled(&mut pending, true, 505));
    }

    #[test]
    fn the_next_decision_waits_for_a_report_taken_after_the_release() {
        let mut pending = Pending {
            session: "s".to_owned(),
            at: Instant::now(),
            observed_at_ms: 100,
            released_at_report: None,
            _decision: None,
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
