//! Process-local write-behind admission. Every accepted intent has one FIFO
//! owner; closing admission drains that owner instead of cancelling send tasks.
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use parking_lot::Mutex;
use tokio::sync::{Notify, mpsc::error::TryRecvError};

use super::{Event, StoreWrite, estimated_store_write_bytes};

pub(crate) const NORMAL_BYTES: usize = 8 * 1024 * 1024;
const RESERVED_BYTES: usize = 256 * 1024;
const RESERVED_SLOTS: usize = 64;
// The local runtime's 32 MiB frame needs room for decoded-envelope overhead.
// This is one slot, not one allowance per producer, session or queue item.
const MAX_SINGLE_BYTES: usize = 2 * crate::runtime_wire::MAX_FRAME_BYTES;

#[derive(Debug, Default)]
pub struct PersistenceHealth {
    pending: AtomicUsize,
    pending_bytes: AtomicUsize,
    dropped: AtomicU64,
    failed_batches: AtomicU64,
    degraded: AtomicBool,
    last_error: Mutex<Option<String>>,
}

impl PersistenceHealth {
    #[must_use]
    pub fn pending(&self) -> usize {
        self.pending.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn pending_bytes(&self) -> usize {
        self.pending_bytes.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn failed_batches(&self) -> u64 {
        self.failed_batches.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn is_healthy(&self) -> bool {
        !self.degraded.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }

    pub(crate) fn mark_failed_batch(&self) {
        self.failed_batches.fetch_add(1, Ordering::Relaxed);
        self.degraded.store(true, Ordering::Relaxed);
        *self.last_error.lock() = Some("database write retries exhausted".to_owned());
    }

    fn rejected(&self, reason: &'static str, count: usize) {
        self.dropped.fetch_add(count as u64, Ordering::Relaxed);
        self.degraded.store(true, Ordering::Relaxed);
        *self.last_error.lock() = Some(reason.to_owned());
        tracing::error!(reason, count, "persistence admission rejected intents");
    }

    fn queued(&self, bytes: usize) {
        self.pending.fetch_add(1, Ordering::Relaxed);
        self.pending_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    fn consumed(&self, bytes: usize) {
        self.pending.fetch_sub(1, Ordering::Relaxed);
        self.pending_bytes.fetch_sub(bytes, Ordering::Relaxed);
    }
}

enum Charge {
    Normal,
    Reserved,
    Oversized,
}

struct Entry {
    write: StoreWrite,
    bytes: usize,
    charge: Charge,
}

#[derive(Default)]
struct State {
    entries: VecDeque<Entry>,
    normal_bytes: usize,
    reserved_bytes: usize,
    oversized: bool,
    closed: bool,
}

impl State {
    fn charge(
        &self,
        bytes: usize,
        critical: bool,
        capacity: usize,
    ) -> Result<Charge, &'static str> {
        if self.closed {
            return Err("persistence admission closed");
        }
        let slots = capacity.saturating_add(if critical { RESERVED_SLOTS } else { 0 });
        if self.entries.len() >= slots {
            return Err("persistence queue count budget exhausted");
        }
        if bytes > MAX_SINGLE_BYTES {
            return Err("persistence intent exceeds single-item budget");
        }
        if bytes > NORMAL_BYTES {
            return if self.oversized {
                Err("persistence oversized slot occupied")
            } else {
                Ok(Charge::Oversized)
            };
        }
        if self.normal_bytes.saturating_add(bytes) <= NORMAL_BYTES {
            return Ok(Charge::Normal);
        }
        if critical && self.reserved_bytes.saturating_add(bytes) <= RESERVED_BYTES {
            return Ok(Charge::Reserved);
        }
        Err("persistence queue byte budget exhausted")
    }

    fn push(&mut self, entry: Entry) {
        match entry.charge {
            Charge::Normal => self.normal_bytes += entry.bytes,
            Charge::Reserved => self.reserved_bytes += entry.bytes,
            Charge::Oversized => self.oversized = true,
        }
        self.entries.push_back(entry);
    }

    fn pop(&mut self, health: &PersistenceHealth) -> Option<StoreWrite> {
        let entry = self.entries.pop_front()?;
        match entry.charge {
            Charge::Normal => self.normal_bytes -= entry.bytes,
            Charge::Reserved => self.reserved_bytes -= entry.bytes,
            Charge::Oversized => self.oversized = false,
        }
        health.consumed(entry.bytes);
        Some(entry.write)
    }
}

struct Shared {
    state: Mutex<State>,
    ready: Notify,
    health: Arc<PersistenceHealth>,
    senders: AtomicUsize,
    capacity: usize,
}

pub struct StoreSink {
    shared: Arc<Shared>,
}

pub struct StoreReceiver {
    shared: Arc<Shared>,
}

impl StoreSink {
    #[must_use]
    pub fn channel(capacity: usize, health: Arc<PersistenceHealth>) -> (Self, StoreReceiver) {
        assert!(capacity > 0, "persistence queue capacity must be nonzero");
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            ready: Notify::new(),
            health,
            senders: AtomicUsize::new(1),
            capacity,
        });
        (
            Self {
                shared: shared.clone(),
            },
            StoreReceiver { shared },
        )
    }

    pub fn send(&self, write: StoreWrite) -> bool {
        let bytes = estimated_store_write_bytes(&write);
        let mut state = self.shared.state.lock();
        let charge = match state.charge(bytes, is_critical(&write), self.shared.capacity) {
            Ok(charge) => charge,
            Err(reason) => {
                // Coordinates aid incident investigation without logging any
                // event, prompt, setting value, attachment or credential.
                if let StoreWrite::AppendEvent(envelope) = &write {
                    tracing::error!(session_id = %envelope.session_id, seq = envelope.seq,
                        bytes, reason, "persistence event was not admitted");
                }
                self.shared.health.rejected(reason, 1);
                return false;
            }
        };
        self.shared.health.queued(bytes);
        state.push(Entry {
            write,
            bytes,
            charge,
        });
        drop(state);
        self.shared.ready.notify_one();
        true
    }
}

impl Clone for StoreSink {
    fn clone(&self) -> Self {
        self.shared.senders.fetch_add(1, Ordering::Relaxed);
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl Drop for StoreSink {
    fn drop(&mut self) {
        if self.shared.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.shared.state.lock().closed = true;
            self.shared.ready.notify_one();
        }
    }
}

impl StoreReceiver {
    pub fn close(&mut self) {
        self.shared.state.lock().closed = true;
        self.shared.ready.notify_one();
    }

    pub fn try_recv(&mut self) -> Result<StoreWrite, TryRecvError> {
        let mut state = self.shared.state.lock();
        state.pop(&self.shared.health).ok_or(if state.closed {
            TryRecvError::Disconnected
        } else {
            TryRecvError::Empty
        })
    }

    pub async fn recv(&mut self) -> Option<StoreWrite> {
        let shared = Arc::clone(&self.shared);
        loop {
            // One receiver only. notify_one retains a permit when send/close
            // races this empty check, so cancellation cannot lose a wakeup.
            let ready = shared.ready.notified();
            match self.try_recv() {
                Ok(write) => return Some(write),
                Err(TryRecvError::Disconnected) => return None,
                Err(TryRecvError::Empty) => {}
            }
            ready.await;
        }
    }
}

impl Drop for StoreReceiver {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock();
        state.closed = true;
        let count = state.entries.len();
        while state.pop(&self.shared.health).is_some() {}
        if count > 0 {
            self.shared
                .health
                .rejected("persistence receiver dropped before drain", count);
        }
    }
}

fn is_critical(write: &StoreWrite) -> bool {
    !matches!(
        write,
        StoreWrite::AppendEvent(super::Envelope {
            event: Event::Update { .. },
            ..
        })
    )
}

#[cfg(test)]
mod tests;
