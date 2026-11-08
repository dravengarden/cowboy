//! One byte-weighted memory reservation ledger for every cold start on this
//! Device: worker launches and Machine browser instances both reserve their
//! startup peak here, so two admissions can never count the same headroom.
//!
//! The check and the reservation are one compare-and-swap on the reserved
//! total. A reservation is refunded exactly when its guard drops, whether the
//! start succeeded or not, so the ledger only ever describes starts that are
//! still in flight.

use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
use std::time::Duration;

/// Headroom kept free beyond every in-flight reservation.
pub const FLOOR_BYTES: u64 = 512 << 20;

/// The process-wide ledger.
pub static LEDGER: Ledger = Ledger::new();

#[derive(Debug)]
pub struct Ledger {
    reserved: AtomicU64,
}

/// Outcome of one admission attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum Admission {
    /// Reserved; `waited` when headroom was short at first.
    Reserved {
        reservation: Reservation,
        waited: bool,
    },
    /// The caller's stop condition held before a reservation was taken.
    Cancelled,
    /// Headroom stayed short for the whole wait; nothing was reserved.
    Exhausted,
}

/// A reserved startup peak. Dropping it refunds exactly the reserved bytes.
#[derive(Debug)]
pub struct Reservation {
    ledger: &'static Ledger,
    bytes: u64,
}

impl PartialEq for Reservation {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.ledger, other.ledger) && self.bytes == other.bytes
    }
}

impl Eq for Reservation {}

impl Reservation {
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.ledger.reserved.fetch_sub(self.bytes, SeqCst);
    }
}

impl Ledger {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            reserved: AtomicU64::new(0),
        }
    }

    /// Bytes reserved by starts still in flight.
    #[must_use]
    pub fn reserved_bytes(&self) -> u64 {
        self.reserved.load(SeqCst)
    }

    /// Whether `headroom` covers `bytes` beside `reserved` bytes and the floor.
    #[must_use]
    pub fn fits(headroom: u64, reserved: u64, bytes: u64) -> bool {
        reserved
            .checked_add(bytes)
            .and_then(|total| total.checked_add(FLOOR_BYTES))
            .is_some_and(|needed| headroom >= needed)
    }

    /// Wait at most `wait` until `headroom()` fits `bytes` beside every
    /// in-flight reservation, then reserve them. A Device without a measured
    /// headroom (`None`) is admitted at once.
    pub async fn reserve(
        &'static self,
        bytes: u64,
        headroom: impl Fn() -> Option<u64>,
        stop: impl Fn() -> bool,
        wait: Duration,
        poll: Duration,
    ) -> Admission {
        let deadline = tokio::time::Instant::now() + wait;
        let mut waited = false;
        loop {
            if stop() {
                return Admission::Cancelled;
            }
            let current = self.reserved.load(SeqCst);
            if headroom().is_none_or(|available| Self::fits(available, current, bytes)) {
                if self
                    .reserved
                    .compare_exchange(current, current + bytes, SeqCst, SeqCst)
                    .is_ok()
                {
                    return Admission::Reserved {
                        reservation: Reservation {
                            ledger: self,
                            bytes,
                        },
                        waited,
                    };
                }
                continue;
            }
            if tokio::time::Instant::now() >= deadline {
                return Admission::Exhausted;
            }
            waited = true;
            tokio::time::sleep(poll).await;
        }
    }
}

impl Default for Ledger {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    const WAIT: Duration = Duration::from_secs(60);
    const POLL: Duration = Duration::from_secs(1);
    const WORKER: u64 = 512 << 20;
    const BROWSER: u64 = 2048 << 20;

    #[test]
    fn a_start_needs_its_bytes_beside_every_reservation_plus_the_floor() {
        assert!(Ledger::fits(WORKER + FLOOR_BYTES, 0, WORKER));
        assert!(!Ledger::fits(WORKER + FLOOR_BYTES - 1, 0, WORKER));
        assert!(!Ledger::fits(WORKER + FLOOR_BYTES, WORKER, WORKER));
        assert!(Ledger::fits(
            BROWSER + WORKER + FLOOR_BYTES,
            WORKER,
            BROWSER
        ));
        assert!(!Ledger::fits(u64::MAX, u64::MAX, 1));
    }

    #[tokio::test(start_paused = true)]
    async fn a_browser_and_a_worker_never_count_the_same_headroom() {
        static LEDGER: Ledger = Ledger::new();
        let headroom = || Some(BROWSER + FLOOR_BYTES);
        let browser = LEDGER
            .reserve(BROWSER, headroom, || false, WAIT, POLL)
            .await;
        assert!(matches!(browser, Admission::Reserved { waited: false, .. }));
        assert_eq!(LEDGER.reserved_bytes(), BROWSER);
        let worker = LEDGER.reserve(WORKER, headroom, || false, WAIT, POLL).await;
        assert_eq!(worker, Admission::Exhausted);
        drop(browser);
        assert_eq!(LEDGER.reserved_bytes(), 0);
        let worker = LEDGER.reserve(WORKER, headroom, || false, WAIT, POLL).await;
        assert!(matches!(worker, Admission::Reserved { .. }));
        assert_eq!(LEDGER.reserved_bytes(), WORKER);
    }

    #[tokio::test(start_paused = true)]
    async fn a_start_waits_until_reclaim_frees_headroom() {
        static LEDGER: Ledger = Ledger::new();
        let headroom = AtomicU64::new(FLOOR_BYTES);
        let polls = AtomicU64::new(0);
        let outcome = LEDGER
            .reserve(
                WORKER,
                || {
                    if polls.fetch_add(1, SeqCst) == 3 {
                        headroom.store(4 << 30, SeqCst);
                    }
                    Some(headroom.load(SeqCst))
                },
                || false,
                WAIT,
                POLL,
            )
            .await;
        assert!(matches!(outcome, Admission::Reserved { waited: true, .. }));
        assert_eq!(LEDGER.reserved_bytes(), WORKER);
    }

    #[tokio::test(start_paused = true)]
    async fn refusal_after_the_bound_cancellation_and_unmeasured_devices() {
        static LEDGER: Ledger = Ledger::new();
        assert_eq!(
            LEDGER
                .reserve(WORKER, || Some(0), || false, WAIT, POLL)
                .await,
            Admission::Exhausted
        );
        assert_eq!(
            LEDGER
                .reserve(WORKER, || Some(0), || true, WAIT, POLL)
                .await,
            Admission::Cancelled
        );
        assert_eq!(LEDGER.reserved_bytes(), 0);
        let unmeasured = LEDGER.reserve(BROWSER, || None, || false, WAIT, POLL).await;
        assert!(matches!(
            unmeasured,
            Admission::Reserved { waited: false, .. }
        ));
    }
}
