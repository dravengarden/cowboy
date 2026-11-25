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
                || s.head_awaits_retry()
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
            s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
            s.in_flight = true;
            s.dispatch_pending = Some(s.in_flight_epoch);
            let trigger = crate::prompt_origin::DeliveryOrigin::cowboy("managed_call");
            let dispatch_origin =
                crate::prompt_origin::compute_dispatch_origin(head.origin.as_ref(), Some(&trigger));
            let dispatch_cmid = hub_dispatch_origin::ensure_dispatch_cmid(head.cmid);
            s.dispatch_explicit_intent(&head.id, &dispatch_cmid);
            Self::note_dispatched(s, Some(&dispatch_cmid));
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

    pub(in crate::core) fn reject_reserved_notice_cmid(
        &self,
        session_id: &str,
        cmid: Option<&str>,
    ) -> bool {
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

    /// Whether `cmid` names a prompt a browser client minted. Hub-synthesized
    /// ids (scheduler wakeups, scheduled drafts, retry, legacy continuation)
    /// are deliberately reusable and never participate in delivery dedupe.
    /// A human/API prompt, not a scheduler, retry or managed-call prompt.
    pub fn is_authored_cmid(cmid: &str) -> bool {
        Self::is_client_cmid(cmid)
            && !cmid.starts_with(MANAGED_CALL_CMID_PREFIX)
            && !cmid.starts_with(crate::agent_model::CALL_NOTICE_PREFIX)
            && !cmid.starts_with(crate::agent_model::PEER_NOTICE_PREFIX)
    }

    pub(in crate::core) fn is_client_cmid(cmid: &str) -> bool {
        !cmid.is_empty() && !cmid.starts_with("__") && !cmid.starts_with("cowboy-")
    }

    pub(in crate::core) fn tracks_delivery(cmid: &str) -> bool {
        Self::is_client_cmid(cmid) || cmid.starts_with("cowboy-superagent:")
    }

    /// True once this Hub has accepted the prompt: still queued, handed to a
    /// worker, or echoed into the transcript (which survives
    /// a Controller restart because the echo persists its `cmid`).
    pub(in crate::core) fn prompt_already_accepted(s: &Session, cmid: &str) -> bool {
        Self::prompt_already_accepted_excluding_draft(s, cmid, None)
    }

    pub(in crate::core) fn prompt_already_accepted_excluding_draft(
        s: &Session,
        cmid: &str,
        skip_draft_pos: Option<usize>,
    ) -> bool {
        if s.queue.iter().any(|m| m.cmid.as_deref() == Some(cmid))
            || s.drafts
                .iter()
                .enumerate()
                .any(|(i, m)| Some(i) != skip_draft_pos && m.cmid.as_deref() == Some(cmid))
        {
            return true;
        }
        s.reserved_recovery_cmids.contains(cmid)
            || s.in_flight_dispatch_cmids.contains(cmid)
            || (Self::tracks_delivery(cmid)
                && (s.dispatched_cmids.iter().any(|seen| seen.cmid == cmid)
                    || s.dispatched_origins.contains_key(cmid)
                    || s.committed_parked_cmids.iter().any(|c| c == cmid)
                    || s.log
                        .iter()
                        .rev()
                        .any(|entry| entry.cmid.as_deref() == Some(cmid))))
    }

    /// True when `cmid` is currently reserved by an in-flight durable recovery
    /// draft activation so competing steer/submit/dispatch paths cannot claim
    /// the same operation ID in SQL before the activation commits.
    pub fn is_cmid_reserved_for_recovery(&self, session_id: &str, cmid: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|s| s.reserved_recovery_cmids.contains(cmid))
    }

    /// True when `cmid` is already known in Hub's in-memory session state
    /// (queued, drafted, reserved, dispatched, parked, or in hot log).
    pub fn is_cmid_known_in_hub(&self, session_id: &str, cmid: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|s| Self::prompt_already_accepted(s, cmid))
    }

    /// Fold a durably retired `cmid` discovered on a Hub cache miss back into
    /// memory and send an addressed confirmation so the replaying browser drops
    /// its outbox entry without re-executing the retired prompt.
    pub fn confirm_durably_retired_cmid(&self, session_id: &str, cmid: &str) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            Self::note_dispatched(s, Some(cmid));
            if !s.committed_parked_cmids.iter().any(|c| c == cmid) {
                while s.committed_parked_cmids.len() >= 512 {
                    s.committed_parked_cmids.pop_front();
                }
                s.committed_parked_cmids.push_back(cmid.to_owned());
            }
            if let Some(rec) = s.dispatched_origins.get_mut(cmid) {
                rec.undelivered_retired = true;
            }
        }
        self.confirm_delivered(session_id, cmid);
    }

    /// Tell every client that `cmid` is already folded into this session, without
    /// changing or re-persisting the queue. Confirmations are monotonic facts the
    /// sync client accepts from any patch, so the resending device retires its
    /// outbox entry instead of waiting out an acknowledgement timeout.
    /// Capture `(queue, drafts, editing)` and assign the monotonic `queue:<sid>`
    /// sync version under one `sync -> sessions` lock acquisition (matching
    /// `sync_resync`'s lock order), so concurrent queue mutations and hold
    /// changes can never publish older rows under a newer version.
    pub(in crate::core) fn publish_pending_snapshot(
        &self,
        session_id: &str,
        extra_confirmed: &[String],
    ) {
        let state = format!("queue:{session_id}");
        let (version, value, confirmed) = {
            let mut reg = self.inner.sync.lock();
            let sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get(session_id) else {
                return;
            };
            let entry = reg.entry(state.clone()).or_default();
            entry.version += 1;
            let version = entry.version;
            let mut confirmed = Self::cmids_of(&s.queue, &s.drafts);
            for cmid in extra_confirmed {
                if !confirmed.iter().any(|existing| existing == cmid) {
                    confirmed.push(cmid.clone());
                }
            }
            let value = serde_json::json!({
                "queue": &s.queue,
                "drafts": &s.drafts,
                "editing": &s.editing,
                "editing_token": &s.editing_token,
            });
            (version, value, confirmed)
        };
        tracing::info!(target: "cowboy::oplog", op = "change", %state, version, confirmed = ?confirmed);
        self.fanout(Outbound::SyncPatch {
            state,
            version,
            value,
            confirmed,
            resync: false,
        });
    }

    pub(in crate::core) fn confirm_delivered(&self, session_id: &str, cmid: &str) {
        let should_confirm = {
            let sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get(session_id) else {
                return;
            };
            // Dispatch admission deduplicates execution but does not prove
            // delivery. Until a queue row or user echo owns the prompt, the
            // browser must retain its durable outbox (including attachments).
            s.committed_parked_cmids.iter().any(|c| c == cmid)
                || s.dispatched_origins
                    .get(cmid)
                    .is_some_and(|rec| rec.undelivered_retired)
                || s.queue
                    .iter()
                    .chain(&s.drafts)
                    .any(|m| m.cmid.as_deref() == Some(cmid))
                || s.dispatched_cmids
                    .iter()
                    .any(|seen| seen.cmid == cmid && seen.echoed)
                || s.log.iter().any(|entry| {
                    entry.cmid.as_deref() == Some(cmid) && is_user_message_chunk(entry)
                })
        };
        if !should_confirm {
            return;
        }
        self.publish_pending_snapshot(session_id, &[cmid.to_owned()]);
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
