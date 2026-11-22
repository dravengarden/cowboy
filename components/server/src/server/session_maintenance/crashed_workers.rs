//! Detect crashed sessions whose worker is still attached (no automatic stop).
//!
//! A non-recoverable turn failure (for example a vendor account refusal, an
//! expired credential or a lost execution environment) makes the worker
//! report its session crashed while its native runtime, and so the process,
//! stays alive. The Controller never reuses it (the next prompt recycles the
//! session), and neither a worker generation rollout nor a credential roll
//! drains a crashed worker, so on 2026-11-22 five such workers had run on OVH
//! for up to eleven days, holding memory, slots and retired credentials.
//!
//! This sweep only reports them: one warning per session and worker epoch
//! once one has been attached for [`REPORT_AFTER`], and a read-only list in
//! the session-maintenance inventory. An operator stops the worker
//! (docs/machine-operations.md#crashed-sessions-with-a-live-worker).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::agent_model::Status;
use crate::remote_runtime::{CrashedWorkerObservation, CrashedWorkerSighting};

use super::AppState;

/// How long a crashed session's worker stays attached before it is reported.
/// A worker whose native runtime died exits by itself within seconds, and a
/// resend recycles the session at once.
pub(super) const REPORT_AFTER: Duration = Duration::from_mins(10);
const TICK: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
struct Attached {
    worker: CrashedWorkerSighting,
    first_seen: Instant,
    warned: bool,
}

/// One crashed session in one sweep, with its runtime's single observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Candidate {
    pub session_id: String,
    pub worker: CrashedWorkerObservation,
}

/// Session -> its crashed worker, from when this Controller first saw that
/// worker epoch. One bounded entry per crashed session: it is kept while
/// the Machine is offline (and hidden), and ends only when the session is
/// gone or no longer crashed, or a connected Machine no longer reports that
/// worker (detached) or reports another epoch (replaced).
#[derive(Default)]
pub(in crate::server) struct Observed(parking_lot::Mutex<HashMap<String, Attached>>);

impl Observed {
    /// Record one sweep; returns the workers to warn about now (each session
    /// and epoch once, after [`REPORT_AFTER`]) with their age.
    pub(super) fn observe(
        &self,
        candidates: &[Candidate],
        now: Instant,
    ) -> Vec<(String, CrashedWorkerSighting, Duration)> {
        let mut attached = self.0.lock();
        let by_session: HashMap<&str, &CrashedWorkerObservation> = candidates
            .iter()
            .map(|seen| (seen.session_id.as_str(), &seen.worker))
            .collect();
        attached.retain(|session, _| {
            by_session
                .get(session.as_str())
                .is_some_and(|worker| !matches!(worker, CrashedWorkerObservation::Absent))
        });
        let mut warn = Vec::new();
        for seen in candidates {
            let CrashedWorkerObservation::Seen(worker) = &seen.worker else {
                continue;
            };
            let fresh = || Attached {
                worker: worker.clone(),
                first_seen: now,
                warned: false,
            };
            let entry = attached
                .entry(seen.session_id.clone())
                .or_insert_with(fresh);
            if entry.worker.worker_epoch != worker.worker_epoch {
                *entry = fresh();
            }
            let age = now.duration_since(entry.first_seen);
            if age >= REPORT_AFTER && !entry.warned {
                entry.warned = true;
                warn.push((seen.session_id.clone(), worker.clone(), age));
            }
        }
        warn
    }

    /// The reported sessions among `sessions` of one Machine (attached past
    /// [`REPORT_AFTER`]); none while that Machine is disconnected.
    pub(super) fn report(
        &self,
        sessions: &[&crate::core::SessionMeta],
        connected: bool,
    ) -> Vec<serde_json::Value> {
        if !connected {
            return Vec::new();
        }
        self.report_at(sessions, Instant::now())
    }

    fn report_at(
        &self,
        sessions: &[&crate::core::SessionMeta],
        now: Instant,
    ) -> Vec<serde_json::Value> {
        let attached = self.0.lock();
        sessions
            .iter()
            .filter_map(|session| {
                let entry = attached.get(&session.id)?;
                let age = now.duration_since(entry.first_seen);
                (age >= REPORT_AFTER && session.status == Status::Crashed).then(|| {
                    serde_json::json!({
                        "id": session.id,
                        "title": session.title,
                        "provider": session.provider,
                        "account": account(session),
                        "worker_epoch": entry.worker.worker_epoch,
                        "worker_pid": worker_pid(&entry.worker.worker_epoch),
                        "generation": entry.worker.generation,
                        "attached_for_ms": u64::try_from(age.as_millis()).unwrap_or(u64::MAX),
                        // No detach signal exists: verify the process first.
                        "may_be_stale": true,
                    })
                })
            })
            .collect()
    }
}

/// A worker's generated epoch is `<pid>-<nanoseconds>`; its PID is what an
/// operator verifies before stopping anything.
fn worker_pid(epoch: &str) -> Option<u32> {
    let (pid, nanos) = epoch.split_once('-')?;
    nanos
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then_some(())?;
    pid.parse().ok()
}

fn account(session: &crate::core::SessionMeta) -> &'static str {
    if session.provider_account_id.is_default() {
        "default"
    } else {
        "named"
    }
}

/// What the warning names besides the session.
struct Context {
    machine: String,
    provider: String,
    account: &'static str,
}

/// The crashed sessions of one sweep, without keeping the session list.
fn candidates(state: &AppState) -> (Vec<Candidate>, HashMap<String, Context>) {
    let mut context = HashMap::new();
    let candidates = state
        .hub
        .session_list()
        .into_iter()
        .filter(|session| session.status == Status::Crashed)
        .map(|session| {
            let worker = state
                .runtime_router
                .runtime(&session.machine_id)
                .map_or(CrashedWorkerObservation::Unavailable, |runtime| {
                    runtime.last_crashed_worker(&session.id)
                });
            context.insert(
                session.id.clone(),
                Context {
                    account: account(&session),
                    machine: session.machine_id,
                    provider: session.provider,
                },
            );
            Candidate {
                session_id: session.id,
                worker,
            }
        })
        .collect();
    (candidates, context)
}

pub(in crate::server) fn start(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        loop {
            {
                let Some(state) = weak.upgrade() else { return };
                if *state.shutdown.borrow() {
                    return;
                }
                let (candidates, context) = candidates(&state);
                let warn = state
                    .session_maintenance
                    .crashed_workers
                    .observe(&candidates, Instant::now());
                for (session_id, worker, age) in warn {
                    let Some(Context {
                        machine,
                        provider,
                        account,
                    }) = context.get(&session_id)
                    else {
                        continue;
                    };
                    tracing::warn!(session = %session_id, %machine, %provider, account,
                        worker_epoch = %worker.worker_epoch, generation = %worker.generation,
                        attached_for_s = age.as_secs(),
                        "crashed session may still hold a live worker; verify and stop it by hand \
                         (docs/machine-operations.md#crashed-sessions-with-a-live-worker)");
                }
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(session: &str, epoch: &str) -> Candidate {
        Candidate {
            session_id: session.to_owned(),
            worker: CrashedWorkerObservation::Seen(CrashedWorkerSighting {
                worker_epoch: epoch.to_owned(),
                generation: "gen-1".to_owned(),
            }),
        }
    }

    fn gone(session: &str, connected: bool) -> Candidate {
        Candidate {
            session_id: session.to_owned(),
            worker: if connected {
                CrashedWorkerObservation::Absent
            } else {
                CrashedWorkerObservation::Unavailable
            },
        }
    }

    fn warned(found: &[(String, CrashedWorkerSighting, Duration)]) -> Vec<&str> {
        found.iter().map(|(session, ..)| session.as_str()).collect()
    }

    #[test]
    fn each_crashed_worker_is_reported_once_after_its_delay_and_a_new_epoch_starts_over() {
        let observed = Observed::default();
        let start = Instant::now();
        assert!(observed.observe(&[seen("a", "7-1")], start).is_empty());
        assert!(
            observed
                .observe(&[seen("a", "7-1")], start + REPORT_AFTER / 2)
                .is_empty()
        );
        let found = observed.observe(&[seen("a", "7-1"), seen("b", "8-1")], start + REPORT_AFTER);
        assert_eq!(warned(&found), ["a"]);
        // Warned once per session and epoch.
        assert_eq!(
            warned(&observed.observe(
                &[seen("a", "7-1"), seen("b", "8-1")],
                start + REPORT_AFTER * 2
            )),
            ["b"]
        );
        // An offline Machine keeps the entry (no second warning on return).
        let later = start + REPORT_AFTER * 3;
        assert!(observed.observe(&[gone("a", false)], later).is_empty());
        assert!(observed.observe(&[seen("a", "7-1")], later).is_empty());
        // A replacement epoch has its own delay.
        assert!(observed.observe(&[seen("a", "9-1")], later).is_empty());
        assert_eq!(
            warned(&observed.observe(&[seen("a", "9-1")], later + REPORT_AFTER)),
            ["a"]
        );
        // A connected Machine no longer reporting it, or a session no longer
        // crashed, ends the entry.
        assert!(
            observed
                .observe(&[gone("a", true)], later + REPORT_AFTER)
                .is_empty()
        );
        assert!(observed.0.lock().is_empty());
    }

    fn meta(id: &str, status: Status, named: bool) -> crate::core::SessionMeta {
        let hub = crate::core::Hub::new();
        hub.create_local_session(
            id.into(),
            "claude-code".into(),
            "/tmp".into(),
            "Crashed fixture".into(),
            crate::core::SessionOrigin::Api,
            false,
        );
        let mut meta = hub.session_info(id).unwrap().meta;
        meta.status = status;
        if named {
            meta.provider_account_id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        }
        meta
    }

    #[test]
    fn the_inventory_lists_crashed_sessions_past_the_delay_as_possibly_stale() {
        let observed = Observed::default();
        let start = Instant::now();
        observed.observe(
            &[
                seen("old", "4242-17"),
                seen("fresh", "broker-x"),
                seen("revived", "1-1"),
            ],
            start,
        );
        let old = meta("old", Status::Crashed, false);
        let revived = meta("revived", Status::Running, true);
        let fresh = meta("fresh", Status::Crashed, true);
        let rows = [&old, &revived, &fresh];
        assert!(observed.report_at(&rows, start).is_empty());
        let report = observed.report_at(&rows, start + REPORT_AFTER);
        assert_eq!(report.len(), 2, "{report:?}");
        assert_eq!(report[0]["id"], "old");
        assert_eq!(report[0]["account"], "default");
        assert_eq!(report[0]["worker_epoch"], "4242-17");
        assert_eq!(report[0]["worker_pid"], 4242);
        assert_eq!(report[0]["generation"], "gen-1");
        assert_eq!(report[0]["may_be_stale"], true);
        assert_eq!(report[1]["account"], "named");
        assert!(report[1]["worker_pid"].is_null());
        // Hidden while the Machine is disconnected.
        assert!(observed.report(&rows, false).is_empty());
    }
}
