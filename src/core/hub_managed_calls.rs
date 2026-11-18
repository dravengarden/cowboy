//! Managed call children: Controller-owned prompts and their dispatch release.

use super::*;

impl Hub {
    /// Submit a managed call's prompt to its child. Client entry points refuse
    /// managed children; only the Controller-owned call runner reaches here.
    /// The caller has just confirmed the child's configuration, so the prompt
    /// is released for dispatch on this Controller.
    pub fn submit_managed_prompt(&self, session_id: &str, call_id: &str, text: String) {
        let cmid = format!("{MANAGED_CALL_CMID_PREFIX}{call_id}");
        self.inner.managed_releases.lock().insert(cmid.clone());
        self.submit_inner(session_id, text, Vec::new(), Some(cmid), true, false);
    }

    /// Release a held managed prompt (see [`ManagedTurn::held`]) after the
    /// runner confirmed the child's configuration again.
    pub fn release_managed_prompt(&self, session_id: &str, call_id: &str) {
        self.inner
            .managed_releases
            .lock()
            .insert(format!("{MANAGED_CALL_CMID_PREFIX}{call_id}"));
        self.try_drain(session_id);
    }

    /// Drop a held managed prompt whose configuration could not be confirmed.
    pub fn discard_managed_prompt(&self, session_id: &str, call_id: &str) {
        let cmid = format!("{MANAGED_CALL_CMID_PREFIX}{call_id}");
        let removed = {
            let mut sessions = self.inner.sessions.lock();
            sessions.get_mut(session_id).is_some_and(|s| {
                let before = s.queue.len();
                s.queue.retain(|m| m.cmid.as_deref() != Some(cmid.as_str()));
                before != s.queue.len()
            })
        };
        if removed {
            self.emit_pending(session_id);
        }
    }

    /// Forget a finished call's release.
    pub fn forget_managed_release(&self, call_id: &str) {
        self.inner
            .managed_releases
            .lock()
            .remove(&format!("{MANAGED_CALL_CMID_PREFIX}{call_id}"));
    }

    pub fn is_managed_child(&self, session_id: &str) -> bool {
        self.inner.sessions.lock().get(session_id).is_some_and(|s| {
            s.meta
                .execution_binding
                .as_ref()
                .is_some_and(|binding| binding.managed().is_some())
        })
    }
}
