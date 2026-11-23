//! Managed call children: Controller-owned prompts.

use super::*;

/// What [`Hub::newest_in_retained_log`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetainedLogScan {
    pub newest_match: Option<u64>,
    pub first_seq: Option<u64>,
    /// Nothing older exists: the retained log is the session's whole history.
    pub whole: bool,
}

impl Hub {
    /// Submit a managed call's prompt to its child. Client entry points refuse
    /// managed children; only the Controller-owned call runner reaches here.
    pub fn submit_managed_prompt(&self, session_id: &str, call_id: &str, text: String) {
        let cmid = format!("{MANAGED_CALL_CMID_PREFIX}{call_id}");
        self.submit_inner(session_id, text, Vec::new(), Some(cmid), true, false);
    }

    /// What `find` makes of a session's retained log (an event's sequence), with the
    /// log's first sequence and whether it is the whole history. The retained
    /// log holds every event not yet trimmed, written to the store or not,
    /// whatever a client bootstrap shows of it. A turn the Hub saw begin
    /// (`owner_turn_seq`) counts even after its event left the log.
    pub fn newest_in_retained_log(
        &self,
        session_id: &str,
        find: impl Fn(&[Envelope]) -> Option<u64>,
    ) -> Option<RetainedLogScan> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        let in_log = find(&s.log);
        Some(RetainedLogScan {
            newest_match: in_log.max(s.owner_turn_seq),
            first_seq: s.log.first().map(|e| e.seq),
            whole: s.reached_start,
        })
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

    /// A client's queue edit. A managed call's queued prompt is its durable
    /// request: an edit would run other instructions under its (reusable)
    /// receipt, so a managed child's queue is never edited by a client.
    pub fn client_edit(
        &self,
        session_id: &str,
        id: &str,
        text: String,
        content: Vec<serde_json::Value>,
    ) -> bool {
        !self.is_managed_child(session_id) && self.edit_queued(session_id, id, text, content)
    }

    /// Queued managed prompts as `(child, parent, call)`. Withdrawal is
    /// persisted best-effort, so a restart can restore a row whose call
    /// already ended; recovery removes those before they can be dispatched.
    pub fn queued_managed_calls(&self) -> Vec<(String, String, String)> {
        let sessions = self.inner.sessions.lock();
        let mut queued = Vec::new();
        for (id, s) in sessions.iter() {
            let Some(parent) = s
                .meta
                .execution_binding
                .as_ref()
                .and_then(|binding| binding.managed())
                .map(|managed| managed.parent_session_id)
            else {
                continue;
            };
            for row in &s.queue {
                if let Some(call) = row
                    .cmid
                    .as_deref()
                    .and_then(|c| c.strip_prefix(MANAGED_CALL_CMID_PREFIX))
                {
                    queued.push((id.clone(), parent.clone(), call.to_owned()));
                }
            }
        }
        queued
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
