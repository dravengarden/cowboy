//! Recover sessions whose startup the Machine never completed.
//!
//! Once a reset is acknowledged the controller has nothing in flight for the
//! session, so a launch the Machine lost (an aborted command, a dropped queue)
//! would leave it "starting" forever. The Machine bounds every launch at its
//! worker ready timeout (255 s) and reports the outcome, so a session still
//! starting with nothing in flight well past that is stranded. Re-issue its
//! launch once; if that stalls too, report it crashed so the next message
//! restarts it.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use tokio::time::Instant;

pub(super) const STALL: Duration = Duration::from_mins(6);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Action {
    Relaunch(String),
    Fail(String),
}

pub(super) struct Observation<'a> {
    pub(super) session_id: &'a str,
    pub(super) starting: bool,
    /// A command for this session still awaits the Machine's answer.
    pub(super) in_flight: bool,
}

#[derive(Default)]
pub(super) struct StartupWatch {
    /// Start of the current idle stall, and whether its launch was re-issued.
    stalled: HashMap<String, (Instant, bool)>,
}

impl StartupWatch {
    pub(super) fn observe<'a>(
        &mut self,
        now: Instant,
        sessions: impl IntoIterator<Item = Observation<'a>>,
    ) -> Vec<Action> {
        let mut starting = HashSet::new();
        let mut actions = Vec::new();
        for session in sessions {
            if !session.starting {
                continue;
            }
            starting.insert(session.session_id.to_owned());
            let (since, relaunched) = self
                .stalled
                .entry(session.session_id.to_owned())
                .or_insert((now, false));
            if session.in_flight {
                // Keep `relaunched`: a re-issued launch is itself in flight.
                *since = now;
                continue;
            }
            if now.duration_since(*since) < STALL {
                continue;
            }
            if *relaunched {
                actions.push(Action::Fail(session.session_id.to_owned()));
            } else {
                *since = now;
                *relaunched = true;
                actions.push(Action::Relaunch(session.session_id.to_owned()));
            }
        }
        for action in &actions {
            if let Action::Fail(id) = action {
                starting.remove(id);
            }
        }
        self.stalled.retain(|id, _| starting.contains(id));
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starting(session_id: &str, in_flight: bool) -> Observation<'_> {
        Observation {
            session_id,
            starting: true,
            in_flight,
        }
    }

    #[test]
    fn a_stranded_start_is_relaunched_once_then_reported() {
        let mut watch = StartupWatch::default();
        let start = Instant::now();
        assert!(watch.observe(start, [starting("s", false)]).is_empty());
        assert!(
            watch
                .observe(
                    start + STALL - Duration::from_secs(1),
                    [starting("s", false)]
                )
                .is_empty()
        );
        assert_eq!(
            watch.observe(start + STALL, [starting("s", false)]),
            vec![Action::Relaunch("s".into())]
        );
        // The re-issued launch is in flight, then acknowledged and stalls again.
        let relaunched = start + STALL + Duration::from_secs(30);
        assert!(watch.observe(relaunched, [starting("s", true)]).is_empty());
        assert!(
            watch
                .observe(
                    relaunched + STALL - Duration::from_secs(1),
                    [starting("s", false)]
                )
                .is_empty()
        );
        assert_eq!(
            watch.observe(relaunched + STALL, [starting("s", false)]),
            vec![Action::Fail("s".into())]
        );
    }

    #[test]
    fn progress_or_a_settled_status_clears_the_stall() {
        let mut watch = StartupWatch::default();
        let start = Instant::now();
        watch.observe(start, [starting("busy", true), starting("done", false)]);
        // A command in flight keeps resetting the clock.
        assert!(
            watch
                .observe(start + STALL, [starting("busy", true)])
                .is_empty()
        );
        // "done" left starting, so a later start begins a fresh stall.
        let later = start + 2 * STALL;
        assert!(watch.observe(later, [starting("done", false)]).is_empty());
        assert_eq!(
            watch.observe(later + STALL, [starting("done", false)]),
            vec![Action::Relaunch("done".into())]
        );
        let settled = Observation {
            session_id: "done",
            starting: false,
            in_flight: false,
        };
        assert!(watch.observe(later + 3 * STALL, [settled]).is_empty());
        assert!(watch.stalled.is_empty());
    }
}
