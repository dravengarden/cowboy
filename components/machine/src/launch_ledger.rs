//! One byte-weighted memory reservation ledger for every start on this
//! Device: worker launches reserve their startup peak until they are ready,
//! and Machine browser instances reserve their admitted memory budget (the
//! unit's own `MemoryMax`) for their whole life, so two admissions can never
//! count the same headroom.
//!
//! The check and the reservation are one step under the ledger lock. A
//! reservation is refunded exactly when its guard drops. A reservation
//! charged against a cgroup counts only the part of it that cgroup has not
//! used yet: its usage (net of inactive file cache, like the headroom
//! sample) is already deducted from every later headroom sample, and
//! counting it twice would refuse starts the Device can hold.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Headroom kept free beyond every outstanding reservation.
pub const FLOOR_BYTES: u64 = 512 << 20;
/// A partial grant is rounded down to whole MiB.
const GRANULE: u64 = 1 << 20;

/// The process-wide ledger.
pub static LEDGER: Ledger = Ledger::new();

#[derive(Debug)]
pub struct Ledger {
    held: parking_lot::Mutex<Held>,
}

#[derive(Debug)]
struct Held {
    next: u64,
    entries: BTreeMap<u64, Entry>,
}

#[derive(Debug)]
struct Entry {
    bytes: u64,
    /// The cgroup whose usage already counts against the headroom.
    cgroup: Option<PathBuf>,
}

impl Entry {
    /// The reserved bytes no headroom sample has deducted yet. An unreadable
    /// cgroup (gone, or not yet populated) credits nothing.
    fn outstanding(&self) -> u64 {
        self.cgroup
            .as_deref()
            .and_then(charged_bytes)
            .map_or(self.bytes, |charged| self.bytes.saturating_sub(charged))
    }
}

/// Usage of `cgroup` net of its inactive file cache: what every headroom
/// sample of an ancestor already deducts for it.
#[must_use]
pub fn charged_bytes(cgroup: &Path) -> Option<u64> {
    let current = std::fs::read_to_string(cgroup.join("memory.current"))
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    let inactive_file = std::fs::read_to_string(cgroup.join("memory.stat"))
        .unwrap_or_default()
        .lines()
        .find_map(|line| line.strip_prefix("inactive_file "))
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    Some(current.saturating_sub(inactive_file))
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

/// Reserved bytes. Dropping it refunds exactly the reserved bytes.
#[derive(Debug)]
pub struct Reservation {
    ledger: &'static Ledger,
    id: u64,
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

    /// From now on, credit `cgroup`'s usage against this reservation (see
    /// [`charged_bytes`]): the reservation then holds only what that
    /// cgroup may still grow into.
    pub fn charge_to(&self, cgroup: &Path) {
        if let Some(entry) = self.ledger.held.lock().entries.get_mut(&self.id) {
            entry.cgroup = Some(cgroup.to_path_buf());
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.ledger.held.lock().entries.remove(&self.id);
    }
}

impl Held {
    fn outstanding(&self) -> u64 {
        self.entries.values().fold(0u64, |total, entry| {
            total.saturating_add(entry.outstanding())
        })
    }

    fn insert(&mut self, bytes: u64) -> u64 {
        let id = self.next;
        self.next += 1;
        self.entries.insert(
            id,
            Entry {
                bytes,
                cgroup: None,
            },
        );
        id
    }
}

impl Ledger {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            held: parking_lot::const_mutex(Held {
                next: 0,
                entries: BTreeMap::new(),
            }),
        }
    }

    /// Reserved bytes no headroom sample has deducted yet.
    #[must_use]
    pub fn reserved_bytes(&self) -> u64 {
        self.held.lock().outstanding()
    }

    /// Whether `headroom` covers `bytes` beside `reserved` bytes and the floor.
    #[must_use]
    pub fn fits(headroom: u64, reserved: u64, bytes: u64) -> bool {
        Self::grant(headroom, reserved, bytes, bytes).is_some()
    }

    /// The largest grant from `min` up to `max` that `headroom` covers beside
    /// `reserved` bytes and the floor; a grant below `max` is rounded down
    /// to whole MiB. `None` when not even `min` fits.
    #[must_use]
    pub fn grant(headroom: u64, reserved: u64, min: u64, max: u64) -> Option<u64> {
        let available = headroom.checked_sub(reserved)?.checked_sub(FLOOR_BYTES)?;
        let bytes = if available >= max {
            max
        } else {
            available - available % GRANULE
        };
        (bytes >= min).then_some(bytes)
    }

    /// Wait at most `wait` until `headroom()` fits `bytes` beside every
    /// outstanding reservation, then reserve them. A Device without a
    /// measured headroom (`None`) is admitted at once.
    pub async fn reserve(
        &'static self,
        bytes: u64,
        headroom: impl Fn() -> Option<u64>,
        stop: impl Fn() -> bool,
        wait: Duration,
        poll: Duration,
    ) -> Admission {
        self.reserve_within(bytes, bytes, headroom, stop, wait, poll)
            .await
    }

    /// Wait at most `wait` until `headroom()` fits at least `min` bytes
    /// beside every outstanding reservation, then reserve the largest
    /// [grant](Self::grant) up to `max` in the same step. A Device without a
    /// measured headroom (`None`) is granted `max` at once.
    pub async fn reserve_within(
        &'static self,
        min: u64,
        max: u64,
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
            {
                let mut held = self.held.lock();
                let granted = match headroom() {
                    None => Some(max),
                    Some(available) => Self::grant(available, held.outstanding(), min, max),
                };
                if let Some(bytes) = granted {
                    let id = held.insert(bytes);
                    return Admission::Reserved {
                        reservation: Reservation {
                            ledger: self,
                            id,
                            bytes,
                        },
                        waited,
                    };
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Admission::Exhausted;
            }
            waited = true;
            tokio::time::sleep(poll).await;
        }
    }

    /// Reserve `bytes` whatever the headroom: memory a running unit (one that
    /// survived a restart) may already use or grow into.
    pub fn force(&'static self, bytes: u64) -> Reservation {
        let id = self.held.lock().insert(bytes);
        Reservation {
            ledger: self,
            id,
            bytes,
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
    use std::sync::atomic::{AtomicU64, Ordering::SeqCst};

    const WAIT: Duration = Duration::from_secs(60);
    const POLL: Duration = Duration::from_secs(1);
    const MIB: u64 = 1 << 20;
    const WORKER: u64 = 512 << 20;
    const BROWSER: u64 = 2048 << 20;
    /// The default `browser.memory_reserve`.
    const BROWSER_RESERVE: u64 = 768 << 20;
    /// Observed on OVH: memory.high 10.2 GB less (memory.current 9.59 GB
    /// minus inactive_file 2.37 GB), about 2.78 GiB.
    const OBSERVED_HEADROOM: u64 = 10_200_000_000 - (9_590_000_000 - 2_370_000_000);

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

    #[test]
    fn a_browser_reserve_fits_the_observed_headroom_and_keeps_the_floor() {
        assert!(Ledger::fits(OBSERVED_HEADROOM, 0, BROWSER_RESERVE));
        // Beside two in-flight cold workers, too.
        assert!(Ledger::fits(OBSERVED_HEADROOM, 2 * WORKER, BROWSER_RESERVE));
        assert!(Ledger::fits(
            BROWSER_RESERVE + FLOOR_BYTES,
            0,
            BROWSER_RESERVE
        ));
        assert!(!Ledger::fits(
            BROWSER_RESERVE + FLOOR_BYTES - 1,
            0,
            BROWSER_RESERVE
        ));
        assert!(!Ledger::fits(
            BROWSER_RESERVE + FLOOR_BYTES + WORKER - 1,
            WORKER,
            BROWSER_RESERVE
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn a_reserve_is_admitted_with_observed_headroom_and_refused_below_it() {
        static LEDGER: Ledger = Ledger::new();
        let admitted = LEDGER
            .reserve(
                BROWSER_RESERVE,
                || Some(OBSERVED_HEADROOM),
                || false,
                WAIT,
                POLL,
            )
            .await;
        assert!(matches!(
            admitted,
            Admission::Reserved { waited: false, .. }
        ));
        assert_eq!(LEDGER.reserved_bytes(), BROWSER_RESERVE);
        drop(admitted);
        let short = BROWSER_RESERVE + FLOOR_BYTES - 1;
        assert_eq!(
            LEDGER
                .reserve(BROWSER_RESERVE, || Some(short), || false, WAIT, POLL)
                .await,
            Admission::Exhausted
        );
        assert_eq!(LEDGER.reserved_bytes(), 0);
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

    #[test]
    fn a_budget_is_max_with_ample_headroom_smaller_when_tight_and_refused_below_min() {
        let max = BROWSER;
        let min = BROWSER_RESERVE;
        assert_eq!(Ledger::grant(8 << 30, 0, min, max), Some(max));
        assert_eq!(Ledger::grant(max + FLOOR_BYTES, 0, min, max), Some(max));
        // Tight: what is left beside the outstanding reservations and the
        // floor, rounded down to whole MiB, still at least the reserve.
        let tight = 1300 * MIB + 12_345 + WORKER + FLOOR_BYTES;
        assert_eq!(Ledger::grant(tight, WORKER, min, max), Some(1300 * MIB));
        // The observed OVH headroom grants the full MemoryMax.
        assert_eq!(Ledger::grant(OBSERVED_HEADROOM, 0, min, max), Some(max));
        assert_eq!(
            Ledger::grant(OBSERVED_HEADROOM, WORKER, min, max),
            Some((OBSERVED_HEADROOM - WORKER - FLOOR_BYTES) / MIB * MIB)
        );
        assert_eq!(Ledger::grant(min + FLOOR_BYTES, 0, min, max), Some(min));
        assert_eq!(Ledger::grant(min + FLOOR_BYTES - 1, 0, min, max), None);
        assert_eq!(Ledger::grant(0, 0, min, max), None);
        assert_eq!(Ledger::grant(u64::MAX, u64::MAX, 1, 1), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_budget_is_reserved_in_one_step_and_held_against_every_later_start() {
        static LEDGER: Ledger = Ledger::new();
        let headroom = || Some(BROWSER + WORKER + FLOOR_BYTES);
        let Admission::Reserved {
            reservation: browser,
            ..
        } = LEDGER
            .reserve_within(BROWSER_RESERVE, BROWSER, headroom, || false, WAIT, POLL)
            .await
        else {
            panic!("ample headroom refused");
        };
        assert_eq!(browser.bytes(), BROWSER);
        assert_eq!(LEDGER.reserved_bytes(), BROWSER);
        // A worker fits beside the held budget; a second browser does not.
        let Admission::Reserved {
            reservation: worker,
            ..
        } = LEDGER.reserve(WORKER, headroom, || false, WAIT, POLL).await
        else {
            panic!("worker refused beside the budget");
        };
        assert_eq!(
            LEDGER
                .reserve_within(BROWSER_RESERVE, BROWSER, headroom, || false, WAIT, POLL)
                .await,
            Admission::Exhausted
        );
        drop(worker);
        // With less headroom left a second open gets a smaller budget.
        let second = LEDGER
            .reserve_within(
                BROWSER_RESERVE,
                BROWSER,
                || Some(BROWSER + BROWSER_RESERVE + 100 * MIB + FLOOR_BYTES),
                || false,
                WAIT,
                POLL,
            )
            .await;
        let Admission::Reserved {
            reservation: second,
            ..
        } = second
        else {
            panic!("a reduced budget was refused");
        };
        assert_eq!(second.bytes(), BROWSER_RESERVE + 100 * MIB);
        drop((browser, second));
        assert_eq!(LEDGER.reserved_bytes(), 0);
    }

    #[test]
    fn a_charged_budget_counts_only_what_its_cgroup_has_not_used() {
        static LEDGER: Ledger = Ledger::new();
        let cgroup = tempfile::tempdir().unwrap();
        let budget = LEDGER.force(BROWSER);
        // Not yet populated: the whole budget is outstanding.
        budget.charge_to(cgroup.path());
        assert_eq!(LEDGER.reserved_bytes(), BROWSER);
        std::fs::write(
            cgroup.path().join("memory.current"),
            format!("{}\n", 700 * MIB),
        )
        .unwrap();
        std::fs::write(
            cgroup.path().join("memory.stat"),
            format!("anon 1\ninactive_file {}\n", 60 * MIB),
        )
        .unwrap();
        // The headroom sample already deducts 640 MiB of it.
        assert_eq!(LEDGER.reserved_bytes(), BROWSER - 640 * MIB);
        std::fs::write(
            cgroup.path().join("memory.current"),
            (3u64 << 30).to_string(),
        )
        .unwrap();
        assert_eq!(LEDGER.reserved_bytes(), 0);
        let worker = LEDGER.force(WORKER);
        assert_eq!(LEDGER.reserved_bytes(), WORKER);
        drop(budget);
        drop(worker);
        assert_eq!(LEDGER.reserved_bytes(), 0);
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
