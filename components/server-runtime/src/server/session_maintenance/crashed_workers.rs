//! Detect and cooperatively drain crashed sessions whose worker is still
//! attached, while keeping an operator warning fallback.
//!
//! A non-recoverable turn failure (for example a vendor account refusal, an
//! expired credential or a lost execution environment) makes the worker
//! report its session crashed while its native runtime, and so the process,
//! stays alive. The Controller never reuses it (the next prompt recycles the
//! session), and neither a worker generation rollout nor a credential roll
//! drains a crashed worker on its own.
//!
//! This sweep asks capable Machines (protocol 55+) to cooperatively drain idle
//! crashed workers via epoch-bound `DrainCrashedWorker` after [`DRAIN_AFTER`]
//! (or immediately when a rollout or credential roll has marked
//! `drain_requested`), retrying every [`DRAIN_RETRY_AFTER`] if refused (or
//! expediting once when a rollout/roll arrives during retry backoff). Dispatches
//! are capped at [`MAX_DRAINS_PER_MACHINE_PER_TICK`] per Machine per tick, and
//! backoff is recorded only when a drain command is actually queued. Any worker
//! still attached past [`REPORT_AFTER`] still logs one warning per session and
//! worker epoch and appears in the read-only session-maintenance inventory
//! (`docs/machine-operations.md#crashed-sessions-with-a-live-worker`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::agent_model::Status;
use crate::remote_runtime::{CrashedWorkerObservation, CrashedWorkerSighting};

use super::AppState;

/// Grace period before automatically draining an idle crashed worker so
/// workers whose native runtime died exit on their own first.
pub const DRAIN_AFTER: Duration = Duration::from_secs(60);
/// Backoff before retrying drain for the same crashed worker epoch.
pub const DRAIN_RETRY_AFTER: Duration = Duration::from_mins(10);
/// How long a crashed session's worker stays attached before it is reported
/// for manual operator verification.
pub const REPORT_AFTER: Duration = Duration::from_mins(10);
/// Bound per-Machine crashed-worker drain dispatches in a single sweep tick.
pub const MAX_DRAINS_PER_MACHINE_PER_TICK: usize = 4;
const TICK: Duration = Duration::from_secs(60);

struct Attached {
    worker: CrashedWorkerSighting,
    first_seen: Instant,
    last_drain_attempt: Option<Instant>,
    expedited_for_roll: bool,
    unprojected_named_crash: bool,
    offline: bool,
    warned: bool,
}

/// Actions selected by one [`Observed::observe`] sweep.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ObserveOutcome {
    /// Crashed workers due for epoch-bound cooperative drain now.
    pub drain: Vec<(String, CrashedWorkerSighting)>,
    /// Crashed workers attached past [`REPORT_AFTER`] (each session/epoch once).
    pub warn: Vec<(String, CrashedWorkerSighting, Duration)>,
}

/// One crashed session in one sweep, with its runtime's single observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub session_id: String,
    pub worker: CrashedWorkerObservation,
    /// True when the Machine reports a `Crashed` named-account worker whose
    /// status could not project into Hub (for example, expired credentials).
    pub unprojected_named_crash: bool,
}

/// When this Controller first saw each crashed session's last reported
/// worker epoch. One bounded entry per crashed session: it is kept while
/// the Machine is offline (and hidden), and ends only when the session is
/// gone or no longer crashed, or a connected Machine no longer reports that
/// worker (detached/drained) or reports another epoch (replaced).
#[derive(Default)]
pub struct Observed(parking_lot::Mutex<HashMap<String, Attached>>);

impl Observed {
    /// Record one sweep; returns crashed workers eligible for cooperative
    /// drain now (caller must invoke [`Self::record_drain_attempt`] only when
    /// a drain command is actually queued) and workers to warn about now (each
    /// session and epoch once, after [`REPORT_AFTER`]).
    pub fn observe(&self, candidates: &[Candidate], now: Instant) -> ObserveOutcome {
        let mut map = self.0.lock();
        let crashed: std::collections::HashSet<&str> = candidates
            .iter()
            .map(|candidate| candidate.session_id.as_str())
            .collect();
        map.retain(|session, _| crashed.contains(session.as_str()));

        let mut outcome = ObserveOutcome::default();
        let mut drain_due: Vec<(Instant, Instant, String, CrashedWorkerSighting)> = Vec::new();
        for candidate in candidates {
            match &candidate.worker {
                CrashedWorkerObservation::Unavailable => {
                    if let Some(existing) = map.get_mut(&candidate.session_id) {
                        existing.offline = true;
                    }
                }
                CrashedWorkerObservation::Absent => {
                    map.remove(&candidate.session_id);
                }
                CrashedWorkerObservation::Seen(worker) => {
                    let entry = map
                        .entry(candidate.session_id.clone())
                        .and_modify(|existing| {
                            if existing.worker.worker_epoch != worker.worker_epoch {
                                *existing = Attached {
                                    worker: worker.clone(),
                                    first_seen: now,
                                    last_drain_attempt: None,
                                    expedited_for_roll: false,
                                    unprojected_named_crash: candidate.unprojected_named_crash,
                                    offline: false,
                                    warned: false,
                                };
                            } else {
                                existing.worker.generation.clone_from(&worker.generation);
                                existing.worker.drain_requested = worker.drain_requested;
                                existing.worker.idle = worker.idle;
                                existing.unprojected_named_crash =
                                    candidate.unprojected_named_crash;
                                existing.offline = false;
                            }
                        })
                        .or_insert_with(|| Attached {
                            worker: worker.clone(),
                            first_seen: now,
                            last_drain_attempt: None,
                            expedited_for_roll: false,
                            unprojected_named_crash: candidate.unprojected_named_crash,
                            offline: false,
                            warned: false,
                        });
                    let age = now.saturating_duration_since(entry.first_seen);
                    let eligible_since = match entry.last_drain_attempt {
                        None => {
                            if worker.drain_requested {
                                entry.first_seen
                            } else {
                                entry.first_seen + DRAIN_AFTER
                            }
                        }
                        Some(last) => {
                            if worker.drain_requested && !entry.expedited_for_roll {
                                last
                            } else {
                                last + DRAIN_RETRY_AFTER
                            }
                        }
                    };
                    if now >= eligible_since {
                        drain_due.push((
                            eligible_since,
                            entry.first_seen,
                            candidate.session_id.clone(),
                            entry.worker.clone(),
                        ));
                    }
                    if !entry.warned && age >= REPORT_AFTER {
                        entry.warned = true;
                        outcome.warn.push((
                            candidate.session_id.clone(),
                            entry.worker.clone(),
                            age,
                        ));
                    }
                }
            }
        }
        // Starvation-free ordering by `eligible_since` ascending: the worker
        // waiting longest since becoming due goes first, so neither repeated
        // refusals nor continuous fresh crashes can starve older due retries.
        drain_due.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.cmp(&right.2))
        });
        outcome.drain = drain_due
            .into_iter()
            .map(|(_, _, session_id, worker)| (session_id, worker))
            .collect();
        outcome
    }

    /// Record that a `DrainCrashedWorker` command was actually queued for
    /// `(session_id, worker_epoch)` at `now`, starting retry backoff without
    /// penalizing workers skipped by Supervisor guards, legacy Machines, or
    /// per-Machine tick caps.
    pub fn record_drain_attempt(
        &self,
        session_id: &str,
        worker_epoch: &str,
        drain_requested: bool,
        now: Instant,
    ) {
        let mut map = self.0.lock();
        if let Some(entry) = map.get_mut(session_id)
            && entry.worker.worker_epoch == worker_epoch
        {
            entry.last_drain_attempt = Some(now);
            if drain_requested {
                entry.expedited_for_roll = true;
            }
        }
    }

    /// Read-only inventory rows for one Machine in `GET /api/session-maintenance`.
    /// Empty while the Machine is disconnected; every reported row requires a
    /// current observation for the reported epoch so a revived session never
    /// lingers between sweep ticks.
    pub fn report(
        &self,
        sessions: &[&crate::core::SessionMeta],
        connected: bool,
        observe: impl Fn(&crate::core::SessionMeta) -> CrashedWorkerObservation,
    ) -> Vec<serde_json::Value> {
        if !connected {
            return Vec::new();
        }
        self.report_at(sessions, Instant::now(), observe)
    }

    fn report_at(
        &self,
        sessions: &[&crate::core::SessionMeta],
        now: Instant,
        observe: impl Fn(&crate::core::SessionMeta) -> CrashedWorkerObservation,
    ) -> Vec<serde_json::Value> {
        let map = self.0.lock();
        let mut rows: Vec<(u64, serde_json::Value)> = sessions
            .iter()
            .filter_map(|meta| {
                let entry = map.get(&meta.id)?;
                if entry.offline {
                    return None;
                }
                let CrashedWorkerObservation::Seen(live) = observe(meta) else {
                    return None;
                };
                if live.worker_epoch != entry.worker.worker_epoch {
                    return None;
                }
                if meta.status != Status::Crashed && meta.provider_account_id.is_default() {
                    return None;
                }
                let age = now.saturating_duration_since(entry.first_seen);
                if age < REPORT_AFTER {
                    return None;
                }
                let attached_for_ms = u64::try_from(age.as_millis()).unwrap_or(u64::MAX);
                Some((
                    attached_for_ms,
                    serde_json::json!({
                        "id": meta.id,
                        "title": meta.title,
                        "provider": meta.provider,
                        "account": if meta.provider_account_id.is_default() {
                            "default"
                        } else {
                            "named"
                        },
                        "worker_epoch": live.worker_epoch,
                        "worker_pid": worker_pid(&live.worker_epoch),
                        "generation": live.generation,
                        "attached_for_ms": attached_for_ms,
                        "may_be_stale": true,
                    }),
                ))
            })
            .collect();
        rows.sort_by_key(|left| std::cmp::Reverse(left.0));
        rows.into_iter().map(|(_, row)| row).collect()
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

pub fn start(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        loop {
            {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                if *state.shutdown.borrow() {
                    return;
                }
                let all_metas = state.hub.session_list();
                let mut metas = Vec::new();
                let mut candidates = Vec::new();
                for meta in all_metas {
                    let worker = state
                        .runtime_router
                        .runtime(&meta.machine_id)
                        .map_or(CrashedWorkerObservation::Unavailable, |runtime| {
                            runtime.last_crashed_worker(&meta.id)
                        });
                    let unprojected_named_crash = meta.status != Status::Crashed
                        && !meta.provider_account_id.is_default()
                        && matches!(worker, CrashedWorkerObservation::Seen(_));
                    let offline_named = meta.status != Status::Crashed
                        && !meta.provider_account_id.is_default()
                        && matches!(worker, CrashedWorkerObservation::Unavailable);
                    if meta.status == Status::Crashed || unprojected_named_crash || offline_named {
                        candidates.push(Candidate {
                            session_id: meta.id.clone(),
                            worker,
                            unprojected_named_crash,
                        });
                        metas.push(meta);
                    }
                }
                let now = Instant::now();
                let outcome = state
                    .session_maintenance
                    .crashed_workers
                    .observe(&candidates, now);
                let mut per_machine_dispatched: HashMap<String, usize> = HashMap::new();
                for (session_id, worker) in outcome.drain {
                    let machine_id = metas
                        .iter()
                        .find(|meta| meta.id == session_id)
                        .map(|meta| meta.machine_id.clone())
                        .unwrap_or_default();
                    let count = per_machine_dispatched
                        .entry(machine_id.clone())
                        .or_insert(0);
                    if *count >= MAX_DRAINS_PER_MACHINE_PER_TICK {
                        continue;
                    }
                    match state
                        .supervisor
                        .drain_crashed_worker(&session_id, &worker.worker_epoch)
                    {
                        Ok(true) => {
                            *count += 1;
                            state
                                .session_maintenance
                                .crashed_workers
                                .record_drain_attempt(
                                    &session_id,
                                    &worker.worker_epoch,
                                    worker.drain_requested,
                                    now,
                                );
                            tracing::info!(
                                session = %session_id,
                                machine = %machine_id,
                                worker_epoch = %worker.worker_epoch,
                                generation = %worker.generation,
                                drain_requested = worker.drain_requested,
                                "requested cooperative drain for idle crashed worker"
                            );
                        }
                        Ok(false) => {}
                        Err(error) => {
                            tracing::debug!(
                                session = %session_id,
                                worker_epoch = %worker.worker_epoch,
                                %error,
                                "skipped automatic crashed-worker drain"
                            );
                        }
                    }
                }
                for (session_id, worker, age) in outcome.warn {
                    if let Some(meta) = metas.iter().find(|meta| meta.id == session_id) {
                        let machine = &meta.machine_id;
                        let provider = &meta.provider;
                        let account = if meta.provider_account_id.is_default() {
                            "default"
                        } else {
                            "named"
                        };
                        tracing::warn!(session = %session_id, %machine, %provider, account,
                        worker_epoch = %worker.worker_epoch, generation = %worker.generation,
                        attached_for_s = age.as_secs(),
                        "crashed session may still hold a live worker; verify and stop it by hand                          (docs/machine-operations.md#crashed-sessions-with-a-live-worker)");
                    }
                }
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen_with_drain(session: &str, epoch: &str, drain_requested: bool) -> Candidate {
        Candidate {
            session_id: session.to_owned(),
            worker: CrashedWorkerObservation::Seen(CrashedWorkerSighting {
                worker_epoch: epoch.to_owned(),
                generation: "gen-1".to_owned(),
                drain_requested,
                idle: true,
            }),
            unprojected_named_crash: false,
        }
    }

    fn seen(session: &str, epoch: &str) -> Candidate {
        seen_with_drain(session, epoch, false)
    }

    fn gone(session: &str, connected: bool) -> Candidate {
        Candidate {
            session_id: session.to_owned(),
            worker: if connected {
                CrashedWorkerObservation::Absent
            } else {
                CrashedWorkerObservation::Unavailable
            },
            unprojected_named_crash: false,
        }
    }

    fn warned(found: &[(String, CrashedWorkerSighting, Duration)]) -> Vec<&str> {
        found.iter().map(|(session, ..)| session.as_str()).collect()
    }

    fn drained(found: &[(String, CrashedWorkerSighting)]) -> Vec<&str> {
        found.iter().map(|(session, _)| session.as_str()).collect()
    }

    #[test]
    fn crashed_workers_drain_after_grace_and_expedite_on_rollout_even_in_backoff() {
        let observed = Observed::default();
        let start = Instant::now();

        let first = observed.observe(
            &[seen("a", "7-1"), seen_with_drain("roll", "8-1", true)],
            start,
        );
        assert_eq!(drained(&first.drain), ["roll"]);
        assert!(first.warn.is_empty());
        observed.record_drain_attempt("roll", "8-1", true, start);

        let at_grace = observed.observe(&[seen("a", "7-1")], start + DRAIN_AFTER);
        assert_eq!(drained(&at_grace.drain), ["a"]);
        let still_due = observed.observe(
            &[seen("a", "7-1")],
            start + DRAIN_AFTER + Duration::from_secs(1),
        );
        assert_eq!(drained(&still_due.drain), ["a"]);

        let attempt_t = start + DRAIN_AFTER + Duration::from_secs(1);
        observed.record_drain_attempt("a", "7-1", false, attempt_t);
        let in_backoff = observed.observe(&[seen("a", "7-1")], attempt_t + Duration::from_secs(30));
        assert!(in_backoff.drain.is_empty());

        let roll_t = attempt_t + Duration::from_secs(45);
        let expedited = observed.observe(&[seen_with_drain("a", "7-1", true)], roll_t);
        assert_eq!(drained(&expedited.drain), ["a"]);
        observed.record_drain_attempt("a", "7-1", true, roll_t);
        let after_expedite = observed.observe(
            &[seen_with_drain("a", "7-1", true)],
            roll_t + Duration::from_secs(30),
        );
        assert!(after_expedite.drain.is_empty());
        let retry = observed.observe(
            &[seen_with_drain("a", "7-1", true)],
            roll_t + DRAIN_RETRY_AFTER,
        );
        assert_eq!(drained(&retry.drain), ["a"]);
    }

    #[test]
    fn repeated_refusals_rotate_and_never_starve_later_or_rollout_workers() {
        let observed = Observed::default();
        let start = Instant::now();
        let all: Vec<Candidate> = (0..8)
            .map(|idx| seen(&format!("s{idx}"), &format!("{idx}-1")))
            .collect();
        assert!(observed.observe(&all, start).drain.is_empty());

        // Tick 1 at DRAIN_AFTER: first 4 get dispatched and recorded.
        let t1 = start + DRAIN_AFTER;
        let out1 = observed.observe(&all, t1);
        assert_eq!(out1.drain.len(), 8);
        for (session_id, worker) in out1.drain.iter().take(MAX_DRAINS_PER_MACHINE_PER_TICK) {
            observed.record_drain_attempt(session_id, &worker.worker_epoch, false, t1);
        }

        // Tick 2: the 4 never-attempted workers come first, and a rollout-marked
        // late worker preempts even ahead of ordinary retries.
        let t2 = t1 + Duration::from_secs(60);
        let mut with_roll = all.clone();
        with_roll[7] = seen_with_drain("s7", "7-1", true);
        let out2 = observed.observe(&with_roll, t2);
        let names2 = drained(&out2.drain);
        assert_eq!(names2[0], "s7");
        for (session_id, worker) in out2.drain.iter().take(MAX_DRAINS_PER_MACHINE_PER_TICK) {
            observed.record_drain_attempt(
                session_id,
                &worker.worker_epoch,
                worker.drain_requested,
                t2,
            );
        }

        // After retry window, older due retries (eligible at t1+10m) MUST sort
        // ahead of fresh crashes becoming eligible at t3, preventing starvation.
        let t3 = t2 + DRAIN_RETRY_AFTER;
        let mut with_fresh = with_roll;
        for idx in 8..12 {
            with_fresh.push(seen(&format!("s{idx}"), &format!("{idx}-1")));
        }
        let _ = observed.observe(&with_fresh, t3 - DRAIN_AFTER);
        let out3 = observed.observe(&with_fresh, t3);
        let first_four = &drained(&out3.drain)[..4];
        assert!(
            first_four
                .iter()
                .all(|id| ["s0", "s1", "s2", "s3"].contains(id)),
            "older due retries must not be starved by fresh crashes: {first_four:?}"
        );
    }

    #[test]
    fn each_crashed_worker_is_reported_once_after_its_delay_and_a_new_epoch_starts_over() {
        let observed = Observed::default();
        let start = Instant::now();
        assert!(observed.observe(&[seen("a", "7-1")], start).warn.is_empty());
        assert!(
            observed
                .observe(&[seen("a", "7-1")], start + REPORT_AFTER / 2)
                .warn
                .is_empty()
        );
        let found = observed.observe(&[seen("a", "7-1"), seen("b", "8-1")], start + REPORT_AFTER);
        assert_eq!(warned(&found.warn), ["a"]);
        assert_eq!(
            warned(
                &observed
                    .observe(
                        &[seen("a", "7-1"), seen("b", "8-1")],
                        start + REPORT_AFTER * 2
                    )
                    .warn
            ),
            ["b"]
        );
        let later = start + REPORT_AFTER * 3;
        assert!(observed.observe(&[gone("a", false)], later).warn.is_empty());
        assert!(observed.observe(&[seen("a", "7-1")], later).warn.is_empty());
        assert!(observed.observe(&[seen("a", "9-1")], later).warn.is_empty());
        assert_eq!(
            warned(
                &observed
                    .observe(&[seen("a", "9-1")], later + REPORT_AFTER)
                    .warn
            ),
            ["a"]
        );
        assert!(
            observed
                .observe(&[gone("a", true)], later + REPORT_AFTER)
                .warn
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
        let live = |m: &crate::core::SessionMeta| match m.id.as_str() {
            "old" => CrashedWorkerObservation::Seen(CrashedWorkerSighting {
                worker_epoch: "4242-17".into(),
                generation: "gen-1".into(),
                drain_requested: false,
                idle: true,
            }),
            "fresh" => CrashedWorkerObservation::Seen(CrashedWorkerSighting {
                worker_epoch: "broker-x".into(),
                generation: "gen-1".into(),
                drain_requested: false,
                idle: true,
            }),
            // Revived named session no longer reports a crashed worker.
            _ => CrashedWorkerObservation::Absent,
        };
        assert!(observed.report_at(&rows, start, live).is_empty());
        let report = observed.report_at(&rows, start + REPORT_AFTER, live);
        assert_eq!(report.len(), 2, "{report:?}");
        assert_eq!(report[0]["id"], "old");
        assert_eq!(report[0]["account"], "default");
        assert_eq!(report[0]["worker_epoch"], "4242-17");
        assert_eq!(report[0]["worker_pid"], 4242);
        assert_eq!(report[0]["generation"], "gen-1");
        assert_eq!(
            report[0]["attached_for_ms"],
            u64::try_from(REPORT_AFTER.as_millis()).unwrap()
        );
        assert_eq!(report[0]["may_be_stale"], true);
        assert_eq!(report[1]["account"], "named");
        assert!(report[1]["worker_pid"].is_null());
        assert!(observed.report(&rows, false, live).is_empty());
    }

    #[test]
    fn unprojected_named_crash_preserves_age_across_offline_flaps_and_hides_when_revived() {
        let observed = Observed::default();
        let start = Instant::now();
        let mut c = seen("named-1", "99-1");
        c.unprojected_named_crash = true;
        assert!(observed.observe(&[c.clone()], start).warn.is_empty());

        // Machine disconnects mid-window: entry stays tracked with original
        // `first_seen` but is hidden while offline.
        let offline = Candidate {
            session_id: "named-1".into(),
            worker: CrashedWorkerObservation::Unavailable,
            unprojected_named_crash: false,
        };
        assert!(
            observed
                .observe(&[offline], start + REPORT_AFTER / 2)
                .warn
                .is_empty()
        );
        let named_meta = meta("named-1", Status::Running, true);
        let still_crashed = |_: &crate::core::SessionMeta| c.worker.clone();
        assert!(
            observed
                .report_at(&[&named_meta], start + REPORT_AFTER, still_crashed)
                .is_empty(),
            "offline entry must stay hidden"
        );

        // Machine reconnects with the same crashed epoch: original age fires
        // the warning immediately instead of resetting.
        let back = observed.observe(&[c.clone()], start + REPORT_AFTER);
        assert_eq!(warned(&back.warn), ["named-1"]);
        assert_eq!(
            observed
                .report_at(&[&named_meta], start + REPORT_AFTER, still_crashed)
                .len(),
            1
        );

        // Before the next sweep tick, a replacement becomes active so live
        // observation returns Absent: report_at must drop the row immediately.
        assert!(
            observed
                .report_at(&[&named_meta], start + REPORT_AFTER, |_| {
                    CrashedWorkerObservation::Absent
                })
                .is_empty()
        );
    }
}
