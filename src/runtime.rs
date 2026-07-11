//! Health state for daemon-owned background tasks.

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub struct RuntimeHealth {
    dispatcher: AtomicBool,
    scheduler: AtomicBool,
    purge_sweeper: AtomicBool,
    store_writer: AtomicBool,
}

impl RuntimeHealth {
    pub fn set_dispatcher(&self, alive: bool) {
        self.dispatcher.store(alive, Ordering::Relaxed);
    }

    pub fn set_scheduler(&self, alive: bool) {
        self.scheduler.store(alive, Ordering::Relaxed);
    }

    pub fn set_purge_sweeper(&self, alive: bool) {
        self.purge_sweeper.store(alive, Ordering::Relaxed);
    }

    pub fn set_store_writer(&self, alive: bool) {
        self.store_writer.store(alive, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_healthy(&self, persistence_enabled: bool) -> bool {
        self.dispatcher.load(Ordering::Relaxed)
            && self.scheduler.load(Ordering::Relaxed)
            && (!persistence_enabled
                || (self.purge_sweeper.load(Ordering::Relaxed)
                    && self.store_writer.load(Ordering::Relaxed)))
    }
}
