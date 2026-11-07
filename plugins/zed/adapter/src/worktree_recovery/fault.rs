//! Per-transport test-only fault: discard an actual original native response.
use super::Kind;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

#[derive(Default)]
pub(crate) struct Fault {
    kind: AtomicI32,
    message: AtomicU32,
    dropped: AtomicBool,
}
impl Fault {
    pub(crate) fn arm(&self, kind: Kind) {
        self.message.store(0, Ordering::SeqCst);
        self.dropped.store(false, Ordering::SeqCst);
        self.kind.store(kind as i32, Ordering::SeqCst);
    }
    pub(crate) fn dispatched(&self, kind: Kind, message: u32) {
        if self
            .kind
            .compare_exchange(kind as i32, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.message.store(message, Ordering::SeqCst);
        }
    }
    pub(crate) fn discard(&self, message: u32) -> bool {
        if self
            .message
            .compare_exchange(message, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.dropped.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }
    pub(crate) fn dropped(&self) -> bool {
        self.dropped.load(Ordering::SeqCst)
    }
}
