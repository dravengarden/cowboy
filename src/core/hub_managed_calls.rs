//! Managed call children: Controller-owned prompts and generation evidence.

use super::*;

impl Hub {
    /// Submit a managed call's prompt to its child. Client entry points refuse
    /// managed children; only the Controller-owned call runner reaches here.
    pub fn submit_managed_prompt(&self, session_id: &str, call_id: &str, text: String) {
        let cmid = format!("{MANAGED_CALL_CMID_PREFIX}{call_id}");
        self.submit_inner(session_id, text, Vec::new(), Some(cmid), true, false);
    }

    /// Withdraw a call's still-queued prompt (cancelled, refused or finished).
    /// Returns whether a queued prompt was withdrawn (so it never started).
    pub fn discard_managed_prompt(&self, session_id: &str, call_id: &str) -> bool {
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
        removed
    }

    /// Whether this session's last launch was refused because its pinned
    /// Provider generation is no longer installed on its Machine.
    pub fn provider_generation_missing(&self, session_id: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|s| s.provider_generation_missing)
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
