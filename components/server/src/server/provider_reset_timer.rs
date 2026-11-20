//! The scheduled provider reset timer.

use super::*;
use crate::usage::{ResetError, ResetReservation, ResetResult};

/// One persisted reset lane: a Service collector account (its reset ID, such
/// as `anthropic`) or a named native account (`account-<id>`). Both share the
/// one-shot claim, retry and action-log contract of [`fire_due_reset`].
pub(super) trait ResetLane {
    /// The `scheduled_provider_actions` key and action-log provider.
    fn lane(&self) -> &str;
    /// Reserve a paced start and say whether the Plugin claims the one-shot
    /// timer before any Provider call (it cannot accept an idempotency key).
    async fn reserve(&self) -> Result<(ResetReservation, bool), ResetError>;
    async fn consume(
        &self,
        reservation: ResetReservation,
        idempotency_key: &str,
    ) -> Result<ResetResult, ResetError>;
    /// The persisted schedule was claimed or cleared.
    async fn cleared(&self);
    /// Whether the authority that created this schedule still holds. A
    /// withdrawn schedule is dropped unspent, never deferred like pacing.
    async fn admitted(&self, _flight: &crate::store::ResetFlight) -> bool {
        true
    }
}

struct ServiceLane<'a> {
    usage: &'a UsageService,
    provider: &'a str,
}

impl ResetLane for ServiceLane<'_> {
    fn lane(&self) -> &str {
        self.provider
    }
    async fn reserve(&self) -> Result<(ResetReservation, bool), ResetError> {
        let reservation = self.usage.reserve_reset(self.provider).await?;
        Ok((
            reservation,
            self.usage.reset_claims_before_attempt(self.provider),
        ))
    }
    async fn consume(
        &self,
        reservation: ResetReservation,
        idempotency_key: &str,
    ) -> Result<ResetResult, ResetError> {
        self.usage
            .consume_reserved_reset(reservation, self.provider, idempotency_key)
            .await
    }
    async fn cleared(&self) {
        self.usage.set_reset_schedule(self.provider, None).await;
    }
}

/// Fire scheduled provider resets when they come due, paced by usage limits,
/// until shutdown. Reset credits belong to provider accounts, not sessions.
pub(super) fn spawn_provider_reset_timer(
    usage: UsageService,
    store: Option<Store>,
    mut shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            // A schedule exists only in the persistent store.
            if let Some(store) = store.as_ref() {
                for provider in &usage.reset_provider_ids() {
                    let lane = ServiceLane {
                        usage: &usage,
                        provider,
                    };
                    fire_if_due(&lane, store).await;
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(15)) => {}
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { break; }
                }
            }
        }
    })
}

/// Fire `lane`'s persisted schedule if it is due. Each lane fires serially,
/// so a schedule found in flight was interrupted (a crash or a lost clear
/// after its attempt): its outcome is unknown and it is never run again.
pub(super) async fn fire_if_due(lane: &impl ResetLane, store: &Store) {
    let (Ok(Some(action)), Ok(Some(flight))) = (
        store.load_provider_reset(lane.lane()).await,
        store.provider_reset_flight(lane.lane()).await,
    ) else {
        return;
    };
    if flight.idempotency_key != action.idempotency_key {
        return;
    }
    let key = action.idempotency_key.clone();
    let drop_with = |status, phase, message: &'static str| {
        let key = key.clone();
        async move {
            log_reset(
                store,
                lane.lane(),
                "scheduled",
                status,
                phase,
                message,
                None,
                Some(&key),
            )
            .await;
            clear(lane, store, &key).await;
        }
    };
    if flight.in_flight {
        tracing::error!(
            lane = lane.lane(),
            "interrupted scheduled provider reset; outcome unknown"
        );
        drop_with(
            "unknown",
            "consume",
            "The scheduled reset was interrupted; its outcome is unknown and it is not retried.",
        )
        .await;
        return;
    }
    if !lane.admitted(&flight).await {
        drop_with(
            "cancelled",
            "timer",
            "Scheduled reset withdrawn: the account's sign-in or availability changed.",
        )
        .await;
        return;
    }
    if action.next_attempt_at_ms <= now_ms() {
        fire_due_reset(lane, store, action).await;
    }
}

/// Append one reset action-log row; logging never blocks the reset.
#[allow(clippy::too_many_arguments)]
pub(super) async fn log_reset(
    store: &Store,
    lane: &str,
    trigger: &str,
    status: &str,
    phase: &str,
    message: &str,
    credit_id: Option<&str>,
    idempotency_key: Option<&str>,
) {
    let _ = store
        .append_provider_action_log(
            lane,
            trigger,
            status,
            phase,
            message,
            credit_id,
            idempotency_key,
            now_ms(),
        )
        .await;
}

/// Claim (delete) exactly this schedule so it fires once.
async fn clear(lane: &impl ResetLane, store: &Store, key: &str) {
    match store.claim_provider_reset(lane.lane(), key).await {
        Ok(true) => lane.cleared().await,
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(%error, lane = lane.lane(), "clearing scheduled provider reset");
        }
    }
}

/// Run one due schedule. Pacing may delay it; reserve before an irreversible
/// claim so pacing never discards it. A Plugin that cannot accept an
/// idempotency key claims the one-shot timer before any Provider call, so a
/// crash or ambiguous response cannot consume a later reset on a retry. An
/// unknown outcome is never retried; only a refused preflight is, twice.
pub(super) async fn fire_due_reset(
    lane: &impl ResetLane,
    store: &Store,
    action: crate::store::ScheduledProviderAction,
) {
    let id = lane.lane();
    let key = action.idempotency_key;
    let (reservation, claim_before) = match lane.reserve().await {
        Ok(reserved) => reserved,
        Err(error) => {
            tracing::debug!(%error, lane = id, "scheduled provider reset waits for usage pacing");
            return;
        }
    };
    // Atomically take exactly this pending schedule before any Provider call:
    // a cancel or replacement while waiting for the reservation wins.
    let taken = if claim_before {
        store.claim_provider_reset(id, &key).await
    } else {
        store.begin_provider_reset(id, &key, now_ms()).await
    };
    match taken {
        Ok(true) if claim_before => lane.cleared().await,
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            tracing::error!(%error, lane = id, "taking scheduled provider reset");
            return;
        }
    }
    let log = |status, phase, message: String, credit: Option<String>| {
        let key = key.clone();
        async move {
            log_reset(
                store,
                id,
                "scheduled",
                status,
                phase,
                &message,
                credit.as_deref(),
                Some(&key),
            )
            .await;
        }
    };
    log(
        "started",
        "preflight",
        "Scheduled reset attempt started".into(),
        None,
    )
    .await;
    match lane.consume(reservation, &key).await {
        Ok(result) => {
            tracing::info!(lane = id, outcome = %result.outcome, "scheduled provider reset finished");
            log(
                "succeeded",
                "provider_response",
                result.outcome,
                result.credit_id,
            )
            .await;
        }
        Err(error) => {
            let reached = error.call_may_have_reached_provider;
            let policy = match scheduled_reset_failure_policy(reached, action.attempt_count) {
                ScheduledResetFailurePolicy::RetryPreflight if claim_before => {
                    ScheduledResetFailurePolicy::StopFailed
                }
                policy => policy,
            };
            let (status, phase) = match policy {
                ScheduledResetFailurePolicy::StopUnknown => {
                    tracing::error!(%error, lane = id, "scheduled provider reset outcome unknown; automatic retry disabled");
                    ("unknown", "consume")
                }
                ScheduledResetFailurePolicy::RetryPreflight => {
                    tracing::warn!(%error, lane = id, "scheduled provider reset preflight failed; retrying safely");
                    ("retrying", "preflight")
                }
                ScheduledResetFailurePolicy::StopFailed => {
                    tracing::error!(%error, lane = id, "scheduled provider reset stopped without retry");
                    ("failed", "preflight")
                }
            };
            log(status, phase, error.to_string(), error.credit_id).await;
            if policy == ScheduledResetFailurePolicy::RetryPreflight {
                // Proven unsent: only this exact schedule returns to pending.
                let _ = store
                    .rearm_provider_reset(id, &key, now_ms().saturating_add(60_000))
                    .await;
                return;
            }
        }
    }
    if !claim_before {
        clear(lane, store, &key).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fake {
        claim_before: bool,
        reached: Option<bool>,
        limiter: crate::usage::limiter::Limiter,
        starts: AtomicUsize,
        consumed: AtomicUsize,
        cleared: AtomicUsize,
        admitted: bool,
    }

    impl Fake {
        fn new(claim_before: bool, reached: Option<bool>) -> Self {
            Self {
                claim_before,
                reached,
                limiter: crate::usage::limiter::Limiter::new(None),
                starts: AtomicUsize::new(0),
                consumed: AtomicUsize::new(0),
                cleared: AtomicUsize::new(0),
                admitted: true,
            }
        }
    }

    impl ResetLane for Fake {
        fn lane(&self) -> &str {
            "account-11111111-1111-4111-8111-111111111111"
        }
        async fn reserve(&self) -> Result<(ResetReservation, bool), ResetError> {
            // A distinct pacing lane per attempt keeps this test off the clock.
            let start = self.starts.fetch_add(1, Ordering::SeqCst);
            let permit = self
                .limiter
                .start_action(&format!("p{start}"), "k", now_ms())
                .map_err(|_| ResetError::new(false, None, "paced"))?;
            let serial = Arc::new(tokio::sync::Mutex::new(())).lock_owned().await;
            Ok((
                ResetReservation::new("k".into(), permit, serial),
                self.claim_before,
            ))
        }
        async fn consume(
            &self,
            mut reservation: ResetReservation,
            _key: &str,
        ) -> Result<ResetResult, ResetError> {
            self.consumed.fetch_add(1, Ordering::SeqCst);
            reservation
                .take("k")?
                .finish(crate::usage::limiter::Outcome::Success, now_ms());
            match self.reached {
                None => Ok(ResetResult {
                    outcome: "reset".into(),
                    credit_id: Some("grant:1".into()),
                }),
                Some(reached) => Err(ResetError::new(reached, None, "refused")),
            }
        }
        async fn cleared(&self) {
            self.cleared.fetch_add(1, Ordering::SeqCst);
        }
        async fn admitted(&self, _flight: &crate::store::ResetFlight) -> bool {
            self.admitted
        }
    }

    async fn store() -> (Store, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        (store, root)
    }

    async fn fire(lane: &Fake) -> (Option<crate::store::ScheduledProviderAction>, Vec<String>) {
        let (store, _root) = store().await;
        store
            .upsert_provider_reset(lane.lane(), now_ms() - 1, "key")
            .await
            .unwrap();
        fire_if_due(lane, &store).await;
        let statuses = store
            .provider_action_logs(10)
            .await
            .unwrap()
            .into_iter()
            .rev()
            .map(|log| log.status)
            .collect();
        (
            store.load_provider_reset(lane.lane()).await.unwrap(),
            statuses,
        )
    }

    #[tokio::test]
    async fn interrupted_cancelled_or_withdrawn_schedules_never_run() {
        // Interrupted: still in flight when the timer next looks.
        let lane = Fake::new(false, None);
        let (db, _root) = store().await;
        db.upsert_provider_reset(lane.lane(), now_ms() - 1, "key")
            .await
            .unwrap();
        assert!(
            db.begin_provider_reset(lane.lane(), "key", now_ms())
                .await
                .unwrap()
        );
        fire_if_due(&lane, &db).await;
        assert_eq!(lane.consumed.load(Ordering::SeqCst), 0);
        assert!(db.load_provider_reset(lane.lane()).await.unwrap().is_none());
        let logs = db.provider_action_logs(10).await.unwrap();
        assert_eq!(logs[0].status, "unknown");

        // Replaced while waiting for the reservation: the old key never runs.
        let (replaced, _second) = store().await;
        replaced
            .upsert_provider_reset(lane.lane(), now_ms() - 1, "old")
            .await
            .unwrap();
        let action = replaced
            .load_provider_reset(lane.lane())
            .await
            .unwrap()
            .unwrap();
        replaced
            .upsert_provider_reset(lane.lane(), now_ms() + 600_000, "new")
            .await
            .unwrap();
        fire_due_reset(&lane, &replaced, action).await;
        assert_eq!(lane.consumed.load(Ordering::SeqCst), 0);
        let left = replaced
            .provider_reset_flight(lane.lane())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (left.idempotency_key.as_str(), left.in_flight),
            ("new", false)
        );

        // Withdrawn authority: dropped unspent, never deferred like pacing.
        let mut withdrawn = Fake::new(false, None);
        withdrawn.admitted = false;
        let (left, logs) = fire(&withdrawn).await;
        assert!(left.is_none());
        assert_eq!(logs, ["cancelled"]);
        assert_eq!(withdrawn.consumed.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_named_lane_fires_once_and_never_retries_an_unknown_outcome() {
        let success = Fake::new(false, None);
        let (left, logs) = fire(&success).await;
        assert!(left.is_none());
        assert_eq!(logs, ["started", "succeeded"]);
        assert_eq!(success.cleared.load(Ordering::SeqCst), 1);

        let unknown = Fake::new(false, Some(true));
        let (left, logs) = fire(&unknown).await;
        assert!(left.is_none(), "an unknown outcome is never retried");
        assert_eq!(logs, ["started", "unknown"]);

        // A refused preflight is deferred, not cleared, and not due at once.
        let refused = Fake::new(false, Some(false));
        let (left, logs) = fire(&refused).await;
        let left = left.expect("retried later");
        assert!(left.next_attempt_at_ms > now_ms());
        assert_eq!(logs, ["started", "retrying"]);

        // A claim-before Plugin loses the schedule before any Provider call.
        let claimed = Fake::new(true, Some(false));
        let (left, logs) = fire(&claimed).await;
        assert!(left.is_none());
        assert_eq!(logs, ["started", "failed"]);
        assert_eq!(claimed.cleared.load(Ordering::SeqCst), 1);
        assert_eq!(claimed.consumed.load(Ordering::SeqCst), 1);
    }
}
