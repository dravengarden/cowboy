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

    /// A registered session's start: runtime placement restrictions, then
    /// the launch gate like every other launch (a retired or blocked account).
    pub(super) fn check_registered_launch(
        &self,
        meta: &crate::core::SessionMeta,
    ) -> Result<(), String> {
        self.runtime_restrictions
            .check(&meta.provider, &meta.machine_id)?;
        self.check_launch_gate(&meta.id)
    }

    pub(super) fn check_launch_gate(&self, session_id: &str) -> Result<(), String> {
        let gate = self.launch_gate.lock().clone();
        gate.map_or(Ok(()), |gate| gate(session_id))
    }
}

/// A trashed session never launches, resumes or replaces a worker: every
/// wake path (open, send, queue drain, schedule, wakeup, reload) passes here.
pub(super) fn refuse_trashed(hub: &crate::core::Hub, session_id: &str) -> Result<(), String> {
    if hub.session_is_trashed(session_id) {
        return Err(crate::core::TRASHED_SESSION_REFUSAL.to_owned());
    }
    Ok(())
}
