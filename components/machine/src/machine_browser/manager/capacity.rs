//! Machine-wide instance capacity: `browser.max_instances` is reserved
//! atomically before any asynchronous start and released only once the
//! instance's unit is confirmed gone, so concurrent opens for different
//! accounts can never exceed it.

use std::sync::Arc;

#[derive(Debug, Default)]
pub(super) struct CapacityLedger {
    used: parking_lot::Mutex<usize>,
}

/// One reserved instance slot; dropping it releases the slot.
#[derive(Debug)]
pub(super) struct CapacityGuard {
    ledger: Arc<CapacityLedger>,
}

impl Drop for CapacityGuard {
    fn drop(&mut self) {
        let mut used = self.ledger.used.lock();
        *used = used.saturating_sub(1);
    }
}

impl CapacityLedger {
    /// Reserve a slot when fewer than `max` are held.
    pub(super) fn try_reserve(self: &Arc<Self>, max: u32) -> Option<CapacityGuard> {
        let mut used = self.used.lock();
        if *used >= usize::try_from(max).unwrap_or(usize::MAX) {
            return None;
        }
        *used += 1;
        Some(CapacityGuard {
            ledger: Arc::clone(self),
        })
    }

    /// Count a unit that is already running (adopted at restart, or one whose
    /// exit is unconfirmed) whatever the current limit.
    pub(super) fn force_reserve(self: &Arc<Self>) -> CapacityGuard {
        *self.used.lock() += 1;
        CapacityGuard {
            ledger: Arc::clone(self),
        }
    }

    #[cfg(test)]
    pub(super) fn used(&self) -> usize {
        *self.used.lock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_reserved_atomically_and_released_on_drop() {
        let ledger = Arc::new(CapacityLedger::default());
        assert!(ledger.try_reserve(0).is_none());
        let first = ledger.try_reserve(2).unwrap();
        let second = ledger.try_reserve(2).unwrap();
        assert!(ledger.try_reserve(2).is_none());
        drop(first);
        let third = ledger.try_reserve(2).unwrap();
        let forced = ledger.force_reserve();
        assert_eq!(ledger.used(), 3);
        assert!(ledger.try_reserve(3).is_none());
        drop((second, third, forced));
        assert_eq!(ledger.used(), 0);
    }

    #[test]
    fn concurrent_reservations_never_exceed_the_limit() {
        let ledger = Arc::new(CapacityLedger::default());
        let barrier = Arc::new(std::sync::Barrier::new(16));
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let ledger = Arc::clone(&ledger);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    ledger.try_reserve(3)
                })
            })
            .collect();
        let held: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(held.iter().filter(|slot| slot.is_some()).count(), 3);
        assert_eq!(ledger.used(), 3);
        drop(held);
        assert_eq!(ledger.used(), 0);
    }
}
