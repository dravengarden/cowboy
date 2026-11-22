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

use super::AppState;

/// How long a crashed session's worker stays attached before it is reported.
/// A worker whose native runtime died exits by itself within seconds, and a
/// resend recycles the session at once.
pub(super) const REPORT_AFTER: Duration = Duration::from_mins(10);
const TICK: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
struct Attached {
    worker_epoch: String,
    first_seen: Instant,
    warned: bool,
}

/// One crashed session with a live worker, as one sweep observed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Sighting {
    pub session_id: String,
    pub worker_epoch: String,
}

/// Session -> its attached crashed worker, from when this Controller first
/// saw that worker epoch. A new epoch starts over.
#[derive(Default)]
pub(in crate::server) struct Observed(parking_lot::Mutex<HashMap<String, Attached>>);

impl Observed {
    /// Record one sweep's sightings; returns those to warn about now (each
    /// session and epoch once, after [`REPORT_AFTER`]) with their age.
    pub(super) fn observe(
        &self,
        sightings: &[Sighting],
        now: Instant,
    ) -> Vec<(Sighting, Duration)> {
        let mut attached = self.0.lock();
        attached.retain(|session, _| sightings.iter().any(|seen| &seen.session_id == session));
        let mut warn = Vec::new();
        for seen in sightings {
            let entry = attached
                .entry(seen.session_id.clone())
                .or_insert_with(|| Attached {
                    worker_epoch: seen.worker_epoch.clone(),
                    first_seen: now,
                    warned: false,
                });
            if entry.worker_epoch != seen.worker_epoch {
                *entry = Attached {
                    worker_epoch: seen.worker_epoch.clone(),
                    first_seen: now,
                    warned: false,
                };
            }
            let age = now.duration_since(entry.first_seen);
            if age >= REPORT_AFTER && !entry.warned {
                entry.warned = true;
                warn.push((seen.clone(), age));
            }
        }
        warn
    }

    /// The reported sessions among `sessions` (attached past [`REPORT_AFTER`]).
    pub(super) fn report(&self, sessions: &[&crate::core::SessionMeta]) -> Vec<serde_json::Value> {
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
                        "worker_epoch": entry.worker_epoch,
                        "attached_for_ms": u64::try_from(age.as_millis()).unwrap_or(u64::MAX),
                    })
                })
            })
            .collect()
    }
}

fn account(session: &crate::core::SessionMeta) -> &'static str {
    if session.provider_account_id.is_default() {
        "default"
    } else {
        "named"
    }
}

pub(in crate::server) fn start(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        loop {
            let Some(state) = weak.upgrade() else { return };
            if *state.shutdown.borrow() {
                return;
            }
            let sessions = state.hub.session_list();
            let sightings: Vec<_> = sessions
                .iter()
                .filter(|session| session.status == Status::Crashed)
                .filter_map(|session| {
                    let worker_epoch = state
                        .runtime_router
                        .runtime(&session.machine_id)?
                        .attached_crashed_worker(&session.id)?;
                    Some(Sighting {
                        session_id: session.id.clone(),
                        worker_epoch,
                    })
                })
                .collect();
            let warn = state
                .session_maintenance
                .crashed_workers
                .observe(&sightings, Instant::now());
            for (seen, age) in warn {
                let Some(session) = sessions
                    .iter()
                    .find(|session| session.id == seen.session_id)
                else {
                    continue;
                };
                tracing::warn!(session = %session.id, machine = %session.machine_id,
                    provider = %session.provider, account = account(session),
                    worker_epoch = %seen.worker_epoch, attached_for_s = age.as_secs(),
                    "crashed session still holds a live worker; stop it by hand \
                     (docs/machine-operations.md#crashed-sessions-with-a-live-worker)");
            }
            drop(state);
            tokio::time::sleep(TICK).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(session: &str, epoch: &str) -> Sighting {
        Sighting {
            session_id: session.to_owned(),
            worker_epoch: epoch.to_owned(),
        }
    }

    #[test]
    fn each_crashed_worker_is_reported_once_after_its_delay_and_a_new_epoch_starts_over() {
        let observed = Observed::default();
        let start = Instant::now();
        assert!(observed.observe(&[seen("a", "e1")], start).is_empty());
        assert!(
            observed
                .observe(&[seen("a", "e1")], start + REPORT_AFTER / 2)
                .is_empty()
        );
        let warned = observed.observe(&[seen("a", "e1"), seen("b", "f1")], start + REPORT_AFTER);
        assert_eq!(warned.len(), 1);
        assert_eq!(warned[0].0, seen("a", "e1"));
        // Warned once per session and epoch.
        assert!(
            observed
                .observe(
                    &[seen("a", "e1"), seen("b", "f1")],
                    start + REPORT_AFTER * 2
                )
                .iter()
                .all(|(sighting, _)| sighting.session_id == "b")
        );
        // A replacement epoch has its own delay.
        let later = start + REPORT_AFTER * 3;
        assert!(observed.observe(&[seen("a", "e2")], later).is_empty());
        assert_eq!(
            observed
                .observe(&[seen("a", "e2")], later + REPORT_AFTER)
                .len(),
            1
        );
        // A worker no longer attached (stopped, recycled) is forgotten.
        assert!(observed.observe(&[], later + REPORT_AFTER).is_empty());
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
    fn the_inventory_lists_only_crashed_sessions_past_the_delay() {
        let observed = Observed::default();
        let start = Instant::now();
        observed.observe(
            &[
                seen("old", "e1"),
                seen("fresh", "f1"),
                seen("revived", "r1"),
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
        assert_eq!(report[0]["provider"], "claude-code");
        assert_eq!(report[0]["worker_epoch"], "e1");
        assert_eq!(report[1]["id"], "fresh");
        assert_eq!(report[1]["account"], "named");
    }
}
