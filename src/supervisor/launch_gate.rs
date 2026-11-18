//! The Controller's launch gate: every worker launch, revive, reset, reload
//! and replacement builds its declaration through `Supervisor::start_session`,
//! which asks this gate first. The Controller binds it to the account
//! controls of named-only Providers, so no entry point (Clear conversation,
//! Reload, a crashed-worker recycle) can revive a parked account's worker.
use super::Supervisor;
use std::sync::Arc;

/// Refuses a session's launch with the visible reason.
pub type LaunchGate = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

impl Supervisor {
    pub fn set_launch_gate(&self, gate: LaunchGate) {
        *self.launch_gate.lock() = Some(gate);
    }

    pub(super) fn check_launch_gate(&self, session_id: &str) -> Result<(), String> {
        let gate = self.launch_gate.lock().clone();
        gate.map_or(Ok(()), |gate| gate(session_id))
    }
}
