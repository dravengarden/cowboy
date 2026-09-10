//! Core, process-local admission time. Not a credential, durable grant, hard
//! syscall timeout, or portable suspend-inclusive/offline clock.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(crate) struct TimeSample {
    monotonic: Instant,
    wall_ms: i64,
}

impl TimeSample {
    pub(crate) fn now() -> Self {
        Self {
            monotonic: Instant::now(),
            wall_ms: chrono::Utc::now().timestamp_millis(),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(monotonic: Instant, wall_ms: i64) -> Self {
        Self { monotonic, wall_ms }
    }
}

/// One original deadline. Neither checking nor repairing a clock renews it.
/// Capture the sample before scheduling, validation, or lock acquisition.
pub(crate) struct OperationBudget {
    received: Instant,
    deadline: Instant,
    expires_at_ms: i64,
    wall_high_water: AtomicI64,
    expired: AtomicBool,
}

impl OperationBudget {
    pub(crate) fn new(expires_at_ms: i64, cap: Duration, received: TimeSample) -> Self {
        let remaining = expires_at_ms.saturating_sub(received.wall_ms).max(0);
        let budget = Duration::from_millis(remaining.unsigned_abs()).min(cap);
        Self {
            received: received.monotonic,
            deadline: received.monotonic + budget,
            expires_at_ms,
            wall_high_water: AtomicI64::new(received.wall_ms),
            expired: AtomicBool::new(received.wall_ms <= 0 || remaining == 0),
        }
    }

    pub(crate) fn expired(&self) -> bool {
        self.check_at(TimeSample::now())
    }

    fn check_at(&self, now: TimeSample) -> bool {
        let previous_wall = self
            .wall_high_water
            .fetch_max(now.wall_ms, Ordering::AcqRel);
        if now.monotonic < self.received
            || now.monotonic >= self.deadline
            || now.wall_ms <= 0
            || now.wall_ms < previous_wall
            || now.wall_ms >= self.expires_at_ms
        {
            self.expired.store(true, Ordering::Release);
        }
        self.expired.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn expired_at(&self, now: TimeSample) -> bool {
        self.check_at(now)
    }

    #[cfg(test)]
    pub(crate) fn expire_for_test(&self) {
        self.expired.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_monotonic_budget_survives_a_slow_or_repaired_wall_clock() {
        let start = Instant::now();
        let issued = TimeSample::for_test(start, 1_000_000);
        let budget = OperationBudget::new(2_000_000, Duration::from_mins(5), issued);
        assert!(!budget.expired_at(TimeSample::for_test(
            start + Duration::from_secs(299),
            1_000_001
        )));
        assert!(budget.expired_at(TimeSample::for_test(
            start + Duration::from_mins(5),
            1_000_002
        )));
        for wall in [0, i64::MIN, 2_000_000, i64::MAX] {
            let invalid = OperationBudget::new(
                2_000_000,
                Duration::from_mins(5),
                TimeSample::for_test(start, wall),
            );
            assert!(invalid.expired_at(TimeSample::for_test(
                start + Duration::from_millis(1),
                1_000_001
            )));
        }
    }

    #[test]
    fn rollback_and_deadline_rejection_are_sticky() {
        let start = Instant::now();
        let issued = TimeSample::for_test(start, 1_000_000);
        for next in [999_999, 1_000_200] {
            let budget = OperationBudget::new(1_000_200, Duration::from_mins(1), issued);
            assert!(
                budget.expired_at(TimeSample::for_test(start + Duration::from_millis(1), next))
            );
            assert!(budget.expired_at(TimeSample::for_test(
                start + Duration::from_millis(2),
                1_000_001
            )));
        }
        let budget = OperationBudget::new(1_000_200, Duration::from_mins(1), issued);
        assert!(budget.expired_at(TimeSample::for_test(
            start + Duration::from_millis(200),
            1_000_001
        )));
        let budget = OperationBudget::new(1_000_200, Duration::from_mins(1), issued);
        assert!(budget.expired_at(TimeSample::for_test(
            start - Duration::from_millis(1),
            1_000_001
        )));
    }
}
