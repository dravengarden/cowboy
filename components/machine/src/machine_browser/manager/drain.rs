//! Plugin-uninstall drain: before the browser Plugin's active generation is
//! removed, every running instance is hibernated (gracefully, with its
//! stop-time snapshot where possible) and its exit confirmed, and new opens
//! are refused until the uninstall finishes. Starts and drains share one
//! gate: a drain refuses new starts at once and waits for every start already
//! admitted to settle, so no open can launch a unit behind a completed drain.

use std::any::Any;

use super::*;
use crate::machine_plugins::{BrowserDrain, DrainFuture};

/// The uninstall's authority, re-checked before every stop.
type DrainAuthority<'a> = dyn Fn() -> Result<()> + Send + Sync + 'a;

/// Admission of unit starts against uninstall drains.
#[derive(Default)]
pub(super) struct StartGate {
    state: parking_lot::Mutex<GateState>,
    settled: tokio::sync::Notify,
}

#[derive(Default)]
struct GateState {
    draining: usize,
    starting: usize,
}

/// One admitted start; dropping it lets a waiting drain proceed.
pub(super) struct StartGuard(Arc<StartGate>);

impl Drop for StartGuard {
    fn drop(&mut self) {
        let mut state = self.0.state.lock();
        state.starting -= 1;
        if state.starting == 0 {
            self.0.settled.notify_waiters();
        }
    }
}

/// Refuses new starts while held.
struct DrainHold(Arc<StartGate>);

impl Drop for DrainHold {
    fn drop(&mut self) {
        self.0.state.lock().draining -= 1;
    }
}

impl StartGate {
    /// Admit one start unless a drain holds the gate.
    pub(super) fn enter(self: &Arc<Self>) -> Option<StartGuard> {
        let mut state = self.state.lock();
        if state.draining > 0 {
            return None;
        }
        state.starting += 1;
        Some(StartGuard(Arc::clone(self)))
    }

    /// Whether a drain holds the gate; an admitted start waiting for memory
    /// gives up then.
    pub(super) fn draining(&self) -> bool {
        self.state.lock().draining > 0
    }

    /// Refuse new starts, then wait until every admitted start has settled.
    async fn drain(self: &Arc<Self>) -> DrainHold {
        self.state.lock().draining += 1;
        let hold = DrainHold(Arc::clone(self));
        loop {
            let settled = self.settled.notified();
            tokio::pin!(settled);
            // Registered before the check, so a release in between is seen.
            settled.as_mut().enable();
            if self.state.lock().starting == 0 {
                return hold;
            }
            settled.await;
        }
    }

    #[cfg(test)]
    pub(super) fn counts(&self) -> (usize, usize) {
        let state = self.state.lock();
        (state.draining, state.starting)
    }
}

impl BrowserManager {
    /// Make this manager the Plugin store's drain for browser uninstalls.
    pub(super) fn register_drain(&self) {
        let drain: Weak<dyn BrowserDrain> = self.this.clone();
        self.providers.register_browser_drain(drain);
    }

    /// Hibernate every instance once in-flight starts settled. `authority`
    /// is re-checked before every stop: an uninstall whose execution lease
    /// ended stops nothing more. Repeating a drain is harmless; it stops
    /// only what still runs.
    pub(super) async fn drain_instances(
        &self,
        authority: &DrainAuthority<'_>,
    ) -> Result<Box<dyn Any + Send>> {
        authority()?;
        let hold = self.starts.drain().await;
        for instance in self.running() {
            authority().context("uninstall authority ended during the browser drain")?;
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
    fn drain<'a>(&'a self, authority: &'a DrainAuthority<'a>) -> DrainFuture<'a> {
        Box::pin(self.drain_instances(authority))
    }
}
