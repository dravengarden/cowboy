//! Which execution lifetime each session's target has proved gone.

use super::Hub;

impl Hub {
    pub fn mark_execution_lost(&self, session_id: &str, incarnation: &str) {
        self.inner
            .lost_execution
            .lock()
            .insert(session_id.to_owned(), incarnation.to_owned());
    }

    /// Whether the session is still bound to a lifetime proved gone. A worker
    /// launched on that binding could only crash before readiness.
    pub fn execution_lifetime_lost(&self, session_id: &str) -> bool {
        let Some(lost) = self.inner.lost_execution.lock().get(session_id).cloned() else {
            return false;
        };
        self.session_info(session_id)
            .and_then(|info| info.meta.execution_binding)
            .and_then(|record| record.decode().ok())
            .is_some_and(|binding| binding.environment.incarnation == lost)
    }
}
