//! Ordinary and automated prompt admission, including queue deduplication.
use super::*;

impl Hub {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::core) fn submit_inner(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        managed: bool,
        front: bool,
        content_origin: Option<crate::prompt_origin::DeliveryOrigin>,
        initiator: crate::prompt_origin::DeliveryOrigin,
    ) {
        // One controller owns a managed child conversation: its parent call.
        if !managed && self.is_managed_child(session_id) {
            self.broadcast_error(
                Some(session_id.to_owned()),
                "This child conversation is controlled by its parent's managed call".to_owned(),
            );
            return;
        }
        let dispatch_origin = crate::prompt_origin::compute_dispatch_origin(
            content_origin.as_ref(),
            Some(&initiator),
        );
        // Only a verified owner submit resets the scheduler's runaway guard;
        // automation, delegated agents, legacy rows, and internal notices stay
        // in the unattended streak.
        if dispatch_origin.is_owner() {
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
                let dispatch_cmid = hub_dispatch_origin::ensure_dispatch_cmid(cmid);
                Self::note_dispatched(s, Some(&dispatch_cmid));
                hub_dispatch_origin::note_dispatch_origin(
                    s,
                    session_id,
                    &dispatch_cmid,
                    dispatch_origin.clone(),
                    initiator.clone(),
                    now_ms(),
                );
                dispatch = Some(DispatchReq {
                    ownership: DispatchOwnership::new(self, session_id, s.in_flight_epoch),
                    session_id: session_id.to_owned(),
                    text,
                    content,
                    cmid: Some(dispatch_cmid),
                    explicit: !managed, // never an owner send (account gates)
                    origin: content_origin,
                    initiator: Some(initiator.clone()),
                    dispatch_origin,
                    trigger: initiator,
                });
            } else {
                let id = self.next_qid();
                let message = QueuedMessage {
                    id,
                    text,
                    content,
                    cmid,
                    schedule: None,
                    origin: content_origin,
                    initiator: Some(initiator),
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

    /// Wake a cleanly dormant (`Exited`) parent whose queue head is a
    /// recovered completion notice. Checks `Exited` status and the exact
    /// completion ID while popping the queue head under the same lock so an
    /// interleaved removal or reorder can never wake the worker for another
    /// prompt.
    pub fn wake_queued_call_notice(&self, session_id: &str, completion_id: &str) -> bool {
        if self.inner.dispatch_tx.lock().is_none() {
            return false;
        }
        let req = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if s.meta.status != Status::Exited
                || s.meta.paused
                || s.meta.closing
                || s.meta.trashed_at_ms.is_some()
                || s.in_flight
                || s.dispatch_pending.is_some()
                || s.delivery_hold.is_some()
                || s.editing.is_some()
                || s.queue.first().and_then(|m| m.cmid.as_deref()) != Some(completion_id)
                || s.queue
                    .first()
                    .is_some_and(|m| self.is_row_withdrawing(session_id, &m.id))
                || self.is_notice_withdrawing(session_id, completion_id)
                || !self.context_allows_dispatch(session_id, Some(completion_id))
            {
                return false;
            }
            let head = s.queue.remove(0);
            s.explicit_wait = false;
            s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
            s.in_flight = true;
            s.dispatch_pending = Some(s.in_flight_epoch);
            let trigger = crate::prompt_origin::DeliveryOrigin::cowboy("managed_call");
            let dispatch_origin =
                crate::prompt_origin::compute_dispatch_origin(head.origin.as_ref(), Some(&trigger));
            let dispatch_cmid = hub_dispatch_origin::ensure_dispatch_cmid(head.cmid);
            Self::note_dispatched(s, Some(&dispatch_cmid));
            hub_dispatch_origin::note_dispatch_origin(
                s,
                session_id,
                &dispatch_cmid,
                dispatch_origin.clone(),
                trigger.clone(),
                now_ms(),
            );
            DispatchReq {
                ownership: DispatchOwnership::new(self, session_id, s.in_flight_epoch),
                session_id: session_id.to_owned(),
                text: head.text,
                content: head.content,
                cmid: Some(dispatch_cmid),
                explicit: false,
                origin: head.origin,
                initiator: head.initiator.or_else(|| Some(trigger.clone())),
                dispatch_origin,
                trigger,
            }
        };
        self.emit_pending(session_id);
        self.send_dispatch(req);
        true
    }

    /// Owner cancellation of a still-queued prompt by `cmid`. If the removed
    /// row was a completion notice, records `owner_withdrawn` atomically with
    /// the queue update so startup recovery does not replay it.
    pub fn withdraw_queued_by_cmid(&self, session_id: &str, cmid: &str) -> bool {
        let (removed, settled) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            let before = s.queue.len();
            let mut settled = Vec::new();
            s.queue.retain(|m| {
                if m.cmid.as_deref() == Some(cmid) {
                    if cmid.starts_with(crate::agent_model::CALL_NOTICE_PREFIX) {
                        settled.push((cmid.to_owned(), "owner_withdrawn".to_owned()));
                    }
                    false
                } else {
                    true
                }
            });
            (before != s.queue.len(), settled)
        };
        if removed {
            self.emit_pending_with_notice_ops(session_id, settled, Vec::new());
            self.try_drain(session_id);
        }
        removed
    }

    pub(super) fn reject_reserved_notice_cmid(&self, session_id: &str, cmid: Option<&str>) -> bool {
        if cmid.is_some_and(crate::agent_model::is_reserved_call_notice_cmid) {
            self.broadcast_error(
                Some(session_id.to_owned()),
                "reserved completion notice message id is not allowed for client submissions"
                    .to_owned(),
            );
            return true;
        }
        false
    }
}

pub(super) fn extract_notice_settlements<'a>(
    items: impl IntoIterator<Item = &'a QueuedMessage>,
) -> Vec<(String, String)> {
    items
        .into_iter()
        .filter_map(|m| m.cmid.as_deref())
        .filter(|c| c.starts_with(crate::agent_model::CALL_NOTICE_PREFIX))
        .map(|c| (c.to_owned(), "owner_withdrawn".to_owned()))
        .collect()
}
