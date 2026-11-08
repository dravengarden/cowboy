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
use crate::provider_account::ProviderAccountId;

const TICK: Duration = Duration::from_secs(5);
/// A report older than this no longer describes the Device; act on none.
const SAMPLE_MAX_AGE: Duration = Duration::from_secs(60);
/// How long a hibernation may take to release its worker before the next
/// decision proceeds anyway on a fresh sample.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(60);
/// A Device with pressure and nothing eligible is reported at most this often.
const STUCK_LOG_INTERVAL: Duration = Duration::from_mins(10);
/// While a reclaim step waits on a browser it cannot resolve from events
/// alone (no terminal event follows an incarnation-less `stopping`), the
/// Machine is asked directly at most this often per account. Longer than the
/// query's own timeout, so at most one query per account is in flight.
const QUERY_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Default)]
struct DeviceState {
    reclaiming: bool,
    pending: Option<Pending>,
    /// A browser told to hibernate; settles only on its confirmed exit and
    /// a report taken since. It never times out into "released".
    browser_pending: Option<BrowserPending>,
    /// Browsers already stopping on this Machine (an owner request, recovery,
    /// an earlier Controller) still hold memory: no session is selected until
    /// they are gone and a report taken since has arrived.
    stopping_wait: Option<StoppingWait>,
    /// When each account's browser on this Machine was last queried.
    queried: HashMap<String, Instant>,
    /// Accounts whose browser this tick wants queried.
    queries_due: Vec<ProviderAccountId>,
    stuck_logged: Option<Instant>,
}

impl DeviceState {
    /// Ask for an authoritative answer about the account's browser, at most
    /// once per `QUERY_INTERVAL`.
    fn want_query(&mut self, account: &ProviderAccountId, now: Instant) {
        if query_due(&mut self.queried, account.as_str(), now) {
            self.queries_due.push(account.clone());
        }
    }
}

/// Whether the account's browser may be queried again at `now`; records the
/// query when it may.
fn query_due(queried: &mut HashMap<String, Instant>, account: &str, now: Instant) -> bool {
    queried.retain(|_, at| now.saturating_duration_since(*at) < QUERY_INTERVAL);
    if queried.contains_key(account) {
        return false;
    }
    queried.insert(account.to_owned(), now);
    true
}

struct StoppingWait {
    since: Instant,
    /// The newest report when no stopping browser remained.
    cleared_at_report: Option<i64>,
}

/// What already-stopping browsers mean for session selection: `Some(step)`
/// keeps it waiting, `None` lets it proceed. While any browser stops the
/// step waits (reported stuck past `RELEASE_TIMEOUT`, never released); once
/// none does, a report taken after that is required.
fn stopping_step(
    wait: &mut Option<StoppingWait>,
    stopping: bool,
    observed_at_ms: i64,
) -> Option<Step> {
    if stopping {
        let wait = wait.get_or_insert_with(|| StoppingWait {
            since: Instant::now(),
            cleared_at_report: None,
        });
        wait.cleared_at_report = None;
        return Some(if wait.since.elapsed() >= RELEASE_TIMEOUT {
            Step::Stuck
        } else {
            Step::Wait
        });
    }
    let waiting = wait.as_mut()?;
    match waiting.cleared_at_report {
        None => {
            waiting.cleared_at_report = Some(observed_at_ms);
            Some(Step::Wait)
        }
        Some(report) if observed_at_ms <= report => Some(Step::Wait),
        Some(_) => {
            *wait = None;
            None
        }
    }
}

/// The targeted instance, captured on the Machine whose memory is low. Only
/// that Machine's own report about that account settles it: the same
/// account's browser on another Machine is a different instance.
struct BrowserPending {
    machine_id: String,
    account: ProviderAccountId,
    incarnation: u64,
    /// The connection the targeted instance was observed on.
    connection: crate::machine_control::ConnectionToken,
    at: Instant,
    /// The newest report when the instance was first seen exited; the
    /// memory it freed shows only in a report taken after that.
    exited_at_report: Option<i64>,
    _decision: Option<tokio::sync::OwnedMutexGuard<()>>,
}

/// Whether the targeted browser instance has verifiably exited, judged
/// only from its own Machine's observation of the account (`current`):
/// hibernated for that incarnation (including the terminal `hibernated` the
/// Machine reports once a background termination retry confirms the exit),
/// the Machine's authoritative "nothing runs", or a newer instance that
/// replaced it there. `stopping` is not an exit: the instance is still
/// copying its snapshot and holding its memory.
fn browser_exited(
    (machine_id, incarnation): (&str, u64),
    connection: &crate::machine_control::ConnectionToken,
    current: Option<&Observed>,
) -> bool {
    let Some(observed) = current else {
        return false;
    };
    if observed.machine_id != machine_id {
        return false;
    }
    let event = &observed.event;
    if event.instance.incarnation > incarnation {
        return true;
    }
    if !observed.connection.same(connection) {
        // A later connection re-announces what survived it; a `stopping`
        // naming no instance (an unconfirmed unit of unknown owner, or the
        // Machine's inventory still pending) may be it.
        return !(observed.live()
            && (event.instance.incarnation == incarnation
                || event.instance.instance_id.is_empty()));
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
        for account in std::mem::take(&mut device.queries_due) {
            let state = Arc::clone(state);
            let machine_id = machine_id.clone();
            // Bounded by the query's own timeout; the answer replaces the
            // cached observation, which the next tick settles on.
            tokio::spawn(async move {
                super::provider_accounts::browser::query_machine(&state, &machine_id, &account)
                    .await;
            });
        }
        match step {
            Step::Idle | Step::Wait => {}
            Step::HibernateBrowser(observed) => {
                let Ok(decision) = session_reclaim::decisions(&machine_id).try_lock_owned() else {
                    continue;
                };
                // Dispatched to the captured instance on its own Machine.
                tracing::info!(machine = %machine_id, headroom_bytes = headroom,
                    "hibernating a Machine browser to relieve Device memory");
                device.browser_pending = Some(BrowserPending {
                    machine_id: machine_id.clone(),
                    account: observed.event.instance.account_id.clone(),
                    incarnation: observed.event.instance.incarnation,
                    connection: observed.connection.clone(),
                    at: Instant::now(),
                    exited_at_report: None,
                    _decision: Some(decision),
                });
                let state = Arc::clone(state);
                tokio::spawn(async move {
                    if let Err(error) =
                        super::provider_accounts::browser::hibernate_instance(&state, &observed)
                            .await
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
            (&pending.machine_id, pending.incarnation),
            &pending.connection,
            state
                .browsers
                .state_on(&pending.machine_id, pending.account.as_str())
                .as_ref(),
        );
        // A stalled browser stop stays pending and blocks session selection;
        // only its verified exit and a sample taken after it settle it. An
        // unresolved observation (for example a reconnected Machine's
        // incarnation-less `stopping`, which no terminal event follows) is
        // refreshed by asking the Machine directly.
        if !browser_settled(pending, exited, observed_at_ms) {
            let (account, at) = (pending.account.clone(), pending.at);
            if !exited {
                device.want_query(&account, Instant::now());
            }
            if !exited && at.elapsed() >= RELEASE_TIMEOUT {
                return Step::Stuck;
            }
            return Step::Wait;
        }
        device.browser_pending = None;
    }
    device.reclaiming = policy.reclaiming(device.reclaiming, headroom);
    if !device.reclaiming {
        device.stuck_logged = None;
        device.stopping_wait = None;
        return Step::Idle;
    }
    if let Some(browser) = state
        .browsers
        .running_on(machine_id)
        .into_iter()
        .find(|observed| state.machine_control.is_current(&observed.connection))
    {
        device.stopping_wait = None;
        return Step::HibernateBrowser(Box::new(browser));
    }
    // A browser already stopping still holds its memory: wait for its
    // confirmed exit (asking the Machine, since an incarnation-less
    // `stopping` gets no terminal event) and a report taken after it.
    let stopping = state.browsers.stopping_on(machine_id);
    let now = Instant::now();
    for observed in &stopping {
        device.want_query(&observed.event.instance.account_id, now);
    }
    if let Some(step) = stopping_step(
        &mut device.stopping_wait,
        !stopping.is_empty(),
        observed_at_ms,
    ) {
        return step;
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
        BrowserPending, Observed, Pending, Policy, QUERY_INTERVAL, RELEASE_TIMEOUT, Step,
        StoppingWait, browser_exited, browser_settled, pending_settled, query_due, stopping_step,
    };
    use crate::machine_control::{ConnectionToken, MachineControl};
    use crate::machine_protocol::machine_browser::{InstanceRef, State, StateEvent};
    use std::collections::HashMap;
    use std::time::Instant;

    fn connection(control: &MachineControl, epoch: &str) -> ConnectionToken {
        machine_connection(control, "ovh", epoch)
    }

    fn machine_connection(control: &MachineControl, machine: &str, epoch: &str) -> ConnectionToken {
        control.install(
            machine.into(),
            epoch.into(),
            false,
            crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
            tokio::sync::mpsc::unbounded_channel().0,
        )
    }

    fn observed(connection: &ConnectionToken, incarnation: u64, state: State) -> Observed {
        Observed {
            machine_id: connection.machine_id().into(),
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
        let exited =
            |current: Option<Observed>| browser_exited(("ovh", 7), &first, current.as_ref());
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
        // `stopping` naming no instance (an unconfirmed unit of unknown
        // owner, or inventory pending) is never an exit, on either connection.
        assert!(!exited(Some(observed(&first, 0, State::Stopping))));
        assert!(!exited(Some(observed(&second, 0, State::Stopping))));
        // Another Machine's browser of the same account is never evidence.
        let other = machine_connection(&control, "hawk", "one");
        assert!(!exited(Some(observed(&other, 8, State::Starting))));
        assert!(!exited(Some(observed(&other, 7, State::Hibernated))));
        assert!(!exited(Some(observed(&other, 0, State::Hibernated))));
    }

    #[test]
    fn a_stopping_browser_settles_on_its_terminal_hibernation() {
        use crate::machine_protocol::machine_browser::Event;
        use crate::server::provider_accounts::browser::coordinator::Coordinator;
        let control = MachineControl::default();
        let coordinator = Coordinator::default();
        let ovh = machine_connection(&control, "ovh", "one");
        let hawk = machine_connection(&control, "hawk", "one");
        coordinator.connected(&ovh, None);
        coordinator.connected(&hawk, None);
        let state = |connection: &ConnectionToken, incarnation, state| {
            Event::State(observed(connection, incarnation, state).event)
        };
        let exited = |coordinator: &Coordinator| {
            browser_exited(
                ("ovh", 7),
                &ovh,
                coordinator
                    .state_on("ovh", "11111111-2222-4333-8444-555555555555")
                    .as_ref(),
            )
        };
        coordinator.observe(&ovh, &state(&ovh, 7, State::Stopping));
        // The other Machine starts and stops its own instance meanwhile.
        coordinator.observe(&hawk, &state(&hawk, 9, State::Detached));
        coordinator.observe(&hawk, &state(&hawk, 9, State::Hibernated));
        assert!(!exited(&coordinator));
        // A query answers `stopping` for a restart-retained unit whose exit
        // is unconfirmed: still not an exit, and not a browser to reclaim.
        let mut queried = observed(&ovh, 7, State::Stopping).event;
        queried.observed_at_ms = 50;
        coordinator.record_authoritative(&ovh, &queried);
        assert!(!exited(&coordinator));
        assert!(coordinator.running_on("ovh").is_empty());
        // The targeted Machine confirms the exit after a termination retry.
        coordinator.observe(&ovh, &state(&ovh, 7, State::Hibernated));
        assert!(exited(&coordinator));
    }

    #[test]
    fn a_reconnected_machine_settles_the_reclaim_through_its_query_answer() {
        use crate::machine_protocol::machine_browser::Reason;
        use crate::server::provider_accounts::browser::coordinator::Coordinator;
        let control = MachineControl::default();
        let coordinator = Coordinator::default();
        let first = connection(&control, "one");
        coordinator.connected(&first, None);
        let account = "11111111-2222-4333-8444-555555555555";
        let exited = |coordinator: &Coordinator| {
            browser_exited(
                ("ovh", 7),
                &first,
                coordinator.state_on("ovh", account).as_ref(),
            )
        };
        // The Machine reconnects while the targeted instance is stopping; its
        // inventory is pending, so the query names no instance.
        let second = connection(&control, "two");
        coordinator.connected(&second, None);
        let mut busy = observed(&second, 0, State::Stopping).event;
        busy.reason = Some(Reason::Busy);
        busy.observed_at_ms = 10;
        coordinator.record_authoritative(&second, &busy);
        assert!(!exited(&coordinator));
        // Session selection also waits for it: it may still hold memory.
        assert_eq!(coordinator.stopping_on("ovh").len(), 1);
        assert!(coordinator.running_on("ovh").is_empty());
        // No terminal event follows; the next query answers "nothing runs".
        let mut gone = observed(&second, 0, State::Hibernated).event;
        gone.observed_at_ms = 20;
        coordinator.record_authoritative(&second, &gone);
        assert!(exited(&coordinator));
        assert!(coordinator.stopping_on("ovh").is_empty());
    }

    #[test]
    fn unresolved_browsers_are_queried_at_a_bounded_rate() {
        let mut queried = HashMap::new();
        let start = Instant::now();
        assert!(query_due(&mut queried, "a", start));
        assert!(!query_due(&mut queried, "a", start));
        // Another account on the same Machine is queried independently.
        assert!(query_due(&mut queried, "b", start));
        assert!(!query_due(&mut queried, "a", start + QUERY_INTERVAL / 2));
        assert!(query_due(&mut queried, "a", start + QUERY_INTERVAL));
    }

    #[test]
    fn sessions_wait_for_already_stopping_browsers_and_a_fresh_report() {
        let mut wait = None;
        // Nothing stopping and nothing waited for: proceed.
        assert_eq!(stopping_step(&mut wait, false, 100), None);
        assert_eq!(stopping_step(&mut wait, true, 100), Some(Step::Wait));
        assert_eq!(stopping_step(&mut wait, true, 200), Some(Step::Wait));
        // Gone, but the newest report may predate the exit.
        assert_eq!(stopping_step(&mut wait, false, 200), Some(Step::Wait));
        assert_eq!(stopping_step(&mut wait, false, 200), Some(Step::Wait));
        // Stopping again restarts the wait for a later report.
        assert_eq!(stopping_step(&mut wait, true, 205), Some(Step::Wait));
        assert_eq!(stopping_step(&mut wait, false, 205), Some(Step::Wait));
        assert_eq!(stopping_step(&mut wait, false, 210), None);
        assert!(wait.is_none());
        // A stop that stalls past the release timeout is stuck, never released.
        let mut stalled = Some(StoppingWait {
            since: Instant::now().checked_sub(RELEASE_TIMEOUT).unwrap(),
            cleared_at_report: None,
        });
        assert_eq!(stopping_step(&mut stalled, true, 500), Some(Step::Stuck));
        assert_eq!(stopping_step(&mut stalled, true, 900), Some(Step::Stuck));
    }

    #[test]
    fn browser_reclaim_settles_on_a_report_taken_after_the_exit() {
        let control = MachineControl::default();
        let mut pending = BrowserPending {
            machine_id: "ovh".into(),
            account: "11111111-2222-4333-8444-555555555555"
                .to_owned()
                .try_into()
                .unwrap(),
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
