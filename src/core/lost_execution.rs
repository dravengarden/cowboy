//! Which execution lifetime each session's target has proved gone.

use super::Hub;
use std::time::{Duration, Instant};

impl Hub {
    pub fn mark_execution_lost(&self, session_id: &str, incarnation: &str) {
        let mut lost = self.inner.lost_execution.lock();
        if lost
            .get(session_id)
            .is_none_or(|(known, _)| known != incarnation)
        {
            lost.insert(
                session_id.to_owned(),
                (incarnation.to_owned(), Instant::now()),
            );
        }
    }

    /// How long ago the target first proved this session's lifetime lost.
    pub fn execution_lost_for(&self, session_id: &str) -> Option<Duration> {
        self.inner
            .lost_execution
            .lock()
            .get(session_id)
            .map(|(_, since)| since.elapsed())
    }

    /// Whether the session is still bound to a lifetime proved gone. A worker
    /// launched on that binding could only crash before readiness.
    pub fn execution_lifetime_lost(&self, session_id: &str) -> bool {
        let Some((lost, _)) = self.inner.lost_execution.lock().get(session_id).cloned() else {
            return false;
        };
        self.session_info(session_id)
            .and_then(|info| info.meta.execution_binding)
            .and_then(|record| record.decode().ok())
            .is_some_and(|binding| binding.environment.incarnation == lost)
    }
}
