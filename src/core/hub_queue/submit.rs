//! Ordinary and automated prompt admission, including queue deduplication.
use super::*;

impl Hub {
    pub(in crate::core) fn submit_inner(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        managed: bool,
        front: bool,
    ) {
        // One controller owns a managed child conversation: its parent call.
        if !managed && self.is_managed_child(session_id) {
            self.broadcast_error(
                Some(session_id.to_owned()),
                "This child conversation is controlled by its parent's managed call".to_owned(),
            );
            return;
        }
        // Only a human submit resets the scheduler's runaway guard; automated
        // coordination and completion notices stay in the unattended streak.
        if crate::prompt_origin::origin_from_cmid(cmid.as_deref()).is_human() {
            self.notify_human_turn(session_id);
        }
        let wired = self.inner.dispatch_tx.lock().is_some();
        let mut dispatch = None;
        let duplicate;
        let notice_cmid = cmid
            .as_ref()
            .filter(|id| id.starts_with(crate::agent_model::CALL_NOTICE_PREFIX))
            .cloned();
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            // Idempotent on cmid: a retry whose original landed in the queue, was
            // handed to a worker, or already echoed into the transcript must not
            // run again. The reconnecting client resends every unconfirmed submit.
            duplicate = cmid
                .as_deref()
                .filter(|c| Self::prompt_already_accepted(s, c))
                .map(str::to_owned);
            if duplicate.is_some() {
                // fall through to the addressed confirmation below
            } else if wired
                && !s.meta.paused
                // A trashed session keeps admitted work queued (a scheduled
                // wakeup is preserved, never consumed) until it is restored.
                && s.meta.trashed_at_ms.is_none()
                && Self::ready(s, true)
                && s.queue.is_empty()
                && self.context_allows_dispatch(session_id, cmid.as_deref())
            {
                s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
                s.in_flight = true;
                s.dispatch_pending = Some(s.in_flight_epoch);
                Self::note_dispatched(s, cmid.as_deref());
                dispatch = Some(DispatchReq {
                    ownership: DispatchOwnership::new(self, session_id, s.in_flight_epoch),
                    session_id: session_id.to_owned(),
                    text,
                    content,
                    cmid,
                    explicit: !managed, // never an owner send (account gates)
                });
            } else {
                let id = self.next_qid();
                let message = QueuedMessage {
                    id,
                    text,
                    content,
                    cmid,
                    schedule: None,
                };
                if front {
                    s.queue.insert(0, message);
                } else {
                    s.queue.push(message);
                }
            }
        }
        crate::logs::wait_cache::record_notice_admission(
            session_id,
            notice_cmid.as_deref(),
            if duplicate.is_some() {
                "duplicate"
            } else if dispatch.is_some() {
                "dispatch_pending"
            } else {
                "enqueued"
            },
        );
        if let Some(cmid) = duplicate {
            self.confirm_delivered(session_id, &cmid);
            return;
        }
        match dispatch {
            // Dispatched straight through — never touched a list, so no flicker
            // of the prompt appearing-then-leaving the queue.
            Some(req) => self.send_dispatch(req),
            None => self.emit_pending(session_id),
        }
    }
}
