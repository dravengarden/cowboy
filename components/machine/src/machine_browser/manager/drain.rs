//! Plugin-uninstall drain: before the browser Plugin's active generation is
//! removed, every running instance is hibernated (gracefully, with its
//! stop-time snapshot where possible) and its exit confirmed, and new opens
//! are refused until the uninstall finishes.

use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::machine_plugins::{BrowserDrain, DrainFuture};

/// Refuses new opens while held.
struct DrainHold(Arc<AtomicUsize>);

impl DrainHold {
    fn new(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(counter))
    }
}

impl Drop for DrainHold {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl BrowserManager {
    /// Make this manager the Plugin store's drain for browser uninstalls.
    pub(super) fn register_drain(&self) {
        let drain: Weak<dyn BrowserDrain> = self.this.clone();
        self.providers.register_browser_drain(drain);
    }

    pub(super) async fn drain_instances(&self) -> Result<Box<dyn Any + Send>> {
        let hold = DrainHold::new(&self.draining);
        for instance in self.running() {
            let event = self
                .stop_instance(&instance, Reason::Reclaim, true, None)
                .await;
            self.send_state(event);
        }
        let running = self.running().len();
        let unconfirmed = self.unconfirmed.lock().len();
        ensure!(
            running == 0 && unconfirmed == 0,
            "{} browser instance(s) could not be confirmed stopped",
            running + unconfirmed
        );
        Ok(Box::new(hold))
    }
}

impl BrowserDrain for BrowserManager {
    fn drain(&self) -> DrainFuture<'_> {
        Box::pin(self.drain_instances())
    }
}
