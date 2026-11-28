//! Antigravity wrap-up steering Hub helpers: live capability broadcast,
//! active-turn compaction fence, editing query, and explicit 5-outcome
//! front-of-queue admission.

use super::*;

/// Explicit outcome of front-of-queue prompt admission so callers can
/// distinguish immediate dispatch, normal front queueing, fallback queueing
/// after a full/closed dispatch channel, idempotent deduplication, and refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontSubmitOutcome {
    Refused,
    Duplicate,
    DispatchedImmediately,
    QueuedFront,
    QueuedFallback,
}

/// Explicit outcome of promoting an already-queued Owner row for Antigravity
/// wrap-up steering without bypassing Pause or elevating non-Owner content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuedWrapUpOutcome {
    SessionNotFound,
    RowNotFound,
    ForbiddenOrigin,
    Reserved,
    EmptyContent,
    Blocked(&'static str),
    DispatchedImmediately,
    QueuedFront,
    QueuedFallback,
}

/// Explicit outcome of activating an Owner draft for Antigravity wrap-up
/// steering, carrying either the promoted queue row ID or the refusal reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftWrapUpOutcome {
    DispatchedImmediately,
    QueuedFront { row_id: String },
    QueuedFallback,
    Refused(&'static str),
}

/// Identifies the exact promoted Owner follow-up that justifies emitting a
/// `WrapUpHint` so hint admission can verify under `sessions.lock()` that the
/// row is still eligible at queue head right before enqueuing the WS frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapUpFollowUp<'a> {
    RowId(&'a str),
    Cmid(&'a str),
}

impl Inbound {
    /// Construct a plain (non-force, non-front, non-wrap-up) `Inbound::Submit`.
    #[must_use]
    pub fn submit_plain(
        session_id: String,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) -> Self {
        Self::Submit {
            session_id,
            text,
            content,
            cmid,
            force: false,
            front: false,
            wrap_up: false,
            wrap_up_target: None,
        }
    }
}

impl Hub {
    /// Record verified live wrap-up steering support from the owning runtime.
    pub fn set_wrap_up_steering(&self, session_id: &str, supported: bool) {
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            match sessions.get_mut(session_id) {
                Some(s) if s.meta.wrap_up_steering != supported => {
                    s.meta.wrap_up_steering = supported;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.broadcast_session(session_id);
        }
    }

    /// Whether a queued row is currently held open for editing (which pauses
    /// automatic queue draining).
    #[must_use]
    pub fn is_editing(&self, session_id: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|s| s.editing.is_some())
    }

    /// The row id currently held open for editing, if any.
    #[must_use]
    pub fn editing_id(&self, session_id: &str) -> Option<String> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .and_then(|s| s.editing.clone())
    }

    /// The lease token for the active edit hold, if any.
    #[must_use]
    pub fn editing_token(&self, session_id: &str) -> Option<String> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .and_then(|s| s.editing_token.clone())
    }

    fn edit_hold_allows_row(s: &Session, row_id: &str, editing_token: Option<&str>) -> bool {
        match s.editing.as_deref() {
            None => true,
            Some(held_id) => {
                held_id == row_id
                    && s.editing_token
                        .as_deref()
                        .is_some_and(|held_tok| editing_token == Some(held_tok))
            }
        }
    }

    /// Parse staged content through the same ACP `ContentBlock` decoder used
    /// by dispatch so malformed non-ACP JSON blocks are never treated as
    /// substantive follow-up content.
    fn has_substantive_staged_content(text: &str, content: &[serde_json::Value]) -> bool {
        if content.is_empty() {
            if text.is_empty() {
                return false;
            }
            return crate::wrap_up::has_substantive_prompt_blocks(&[
                serde_json::json!({"type": "text", "text": text}),
            ]);
        }
        let normalized: Vec<serde_json::Value> = content
            .iter()
            .filter_map(|v| {
                serde_json::from_value::<agent_client_protocol::schema::v1::ContentBlock>(v.clone())
                    .ok()
                    .and_then(|block| serde_json::to_value(block).ok())
            })
            .collect();
        crate::wrap_up::has_substantive_prompt_blocks(&normalized)
    }

    /// Validate a reserved recovery draft BEFORE durably committing wrap-up
    /// activation so non-Owner, empty/malformed, or edit-held drafts remain
    /// parked untouched.
    pub fn validate_recovery_draft_for_wrap_up(
        &self,
        session_id: &str,
        draft_id: &str,
        editing_token: Option<&str>,
    ) -> Result<(), &'static str> {
        if self.is_managed_child(session_id) {
            return Err("managed child conversations do not accept wrap-up steering");
        }
        let sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get(session_id) else {
            return Err("Session no longer exists; recovery draft was not activated");
        };
        if s.meta.system || s.meta.closing || s.meta.trashed_at_ms.is_some() {
            return Err("session unavailable for wrap-up steering");
        }
        if s.meta.paused {
            return Err("Queue is paused");
        }
        let Some(draft) = s.drafts.iter().find(|m| m.id == draft_id) else {
            return Err("Recovery draft no longer exists");
        };
        if !draft
            .origin
            .as_ref()
            .is_some_and(crate::prompt_origin::DeliveryOrigin::is_owner)
        {
            return Err("Only your own messages can request wrap-up");
        }
        if !Self::has_substantive_staged_content(&draft.text, &draft.content) {
            return Err("empty prompt: no substantive text or content");
        }
        if !Self::edit_hold_allows_row(s, draft_id, editing_token) {
            return Err("Finish editing queued message first");
        }
        Ok(())
    }

    /// Promote an already-queued Owner-authored row to the front of the queue
    /// for Antigravity wrap-up steering without bypassing Pause, stealing
    /// another tab's edit lease, or elevating Peer/Automation/legacy content.
    pub fn wrap_up_queued_row(
        &self,
        session_id: &str,
        id: &str,
        caller: &crate::prompt_origin::RequestCaller,
        editing_token: Option<&str>,
    ) -> QueuedWrapUpOutcome {
        if !caller.is_person() {
            return QueuedWrapUpOutcome::ForbiddenOrigin;
        }
        if self.is_managed_child(session_id) {
            return QueuedWrapUpOutcome::Blocked(
                "managed child conversations do not accept wrap-up steering",
            );
        }
        let wired = self.inner.dispatch_tx.lock().is_some();
        let dispatch = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return QueuedWrapUpOutcome::SessionNotFound;
            };
            if s.meta.system || s.meta.closing || s.meta.trashed_at_ms.is_some() {
                return QueuedWrapUpOutcome::Blocked("session unavailable for wrap-up steering");
            }
            if s.meta.paused {
                return QueuedWrapUpOutcome::Blocked("Queue is paused");
            }
            if s.steer_reservations.values().any(|r| r.row_id == id)
                || self.is_row_withdrawing(session_id, id)
            {
                return QueuedWrapUpOutcome::Reserved;
            }
            let Some(pos) = s.queue.iter().position(|m| m.id == id) else {
                return QueuedWrapUpOutcome::RowNotFound;
            };
            if !s.queue[pos]
                .origin
                .as_ref()
                .is_some_and(crate::prompt_origin::DeliveryOrigin::is_owner)
            {
                return QueuedWrapUpOutcome::ForbiddenOrigin;
            }
            if !Self::has_substantive_staged_content(&s.queue[pos].text, &s.queue[pos].content) {
                return QueuedWrapUpOutcome::EmptyContent;
            }
            if !Self::edit_hold_allows_row(s, id, editing_token) {
                return QueuedWrapUpOutcome::Blocked("Finish editing queued message first");
            }
            if s.editing.as_deref() == Some(id) {
                s.editing = None;
                s.editing_token = None;
            }
            let mut row = s.queue.remove(pos);
            let owner = crate::prompt_origin::DeliveryOrigin::Owner;
            row.initiator = Some(owner.clone());
            let can_dispatch_now = wired
                && Self::ready(s, false)
                && self.context_allows_dispatch(session_id, row.cmid.as_deref());
            if can_dispatch_now {
                let dispatch_origin = crate::prompt_origin::compute_dispatch_origin(
                    row.origin.as_ref(),
                    Some(&owner),
                );
                let dispatch_cmid = hub_dispatch_origin::ensure_dispatch_cmid(row.cmid);
                s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
                s.in_flight = true;
                s.dispatch_pending = Some(s.in_flight_epoch);
                Self::note_dispatched(s, Some(&dispatch_cmid));
                self.settle_explicit_locked(session_id, s);
                Some(DispatchReq {
                    ownership: DispatchOwnership::new(self, session_id, s.in_flight_epoch),
                    session_id: session_id.to_owned(),
                    text: row.text,
                    content: row.content,
                    cmid: Some(dispatch_cmid),
                    explicit: true,
                    origin: row.origin,
                    initiator: Some(owner.clone()),
                    dispatch_origin,
                    trigger: owner,
                })
            } else {
                s.queue.insert(0, row);
                self.settle_explicit_locked(session_id, s);
                None
            }
        };
        self.notify_human_turn(session_id);
        self.emit_pending(session_id);
        match dispatch {
            Some(req) => {
                if self.send_dispatch_checked(req) {
                    QueuedWrapUpOutcome::DispatchedImmediately
                } else {
                    QueuedWrapUpOutcome::QueuedFallback
                }
            }
            None => QueuedWrapUpOutcome::QueuedFront,
        }
    }

    /// Atomically activate an Owner-authored regular draft to the front of the
    /// queue for Antigravity wrap-up steering, emitting an addressed command
    /// rejection for every refusal so clients never retain stale outbox entries.
    pub fn activate_draft_wrap_up(
        &self,
        session_id: &str,
        id: &str,
        cmid: Option<String>,
        caller: &crate::prompt_origin::RequestCaller,
        editing_token: Option<&str>,
    ) -> DraftWrapUpOutcome {
        let reject = |msg: &'static str| -> DraftWrapUpOutcome {
            if let Some(op) = cmid.as_deref().filter(|op| !op.is_empty()) {
                self.command_result(session_id, op, CommandOutcome::Rejected, msg);
            }
            self.broadcast_error(Some(session_id.to_owned()), msg.to_owned());
            DraftWrapUpOutcome::Refused(msg)
        };
        if !caller.is_person() {
            return reject("wrap_up delivery is restricted to the session owner");
        }
        if self.is_managed_child(session_id) {
            return reject("managed child conversations do not accept wrap-up steering");
        }
        if id.starts_with("recovery-") {
            return reject("Recovery draft wrap-up must use the durable recovery path");
        }
        if self.reject_reserved_notice_cmid(session_id, cmid.as_deref()) {
            return reject("reserved completion notice message id is not allowed");
        }
        let trigger = crate::prompt_origin::DeliveryOrigin::Owner;
        let wired = self.inner.dispatch_tx.lock().is_some();
        let mut dispatch = None;
        let mut promoted_row_id: Option<String> = None;
        let mut had_scheduled = false;
        let mut error_msg: Option<&'static str> = None;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                drop(sessions);
                return reject("Session no longer exists");
            };
            if s.meta.system || s.meta.closing || s.meta.trashed_at_ms.is_some() {
                error_msg = Some("session unavailable for wrap-up steering");
            } else if s.meta.paused {
                error_msg = Some("Queue is paused");
            } else if s.activating_recovery_drafts.contains(id)
                || s.steer_reservations.values().any(|r| r.row_id == id)
            {
                error_msg = Some("This draft is already being delivered");
            } else if let Some(pos) = s.drafts.iter().position(|m| m.id == id) {
                if !s.drafts[pos]
                    .origin
                    .as_ref()
                    .is_some_and(crate::prompt_origin::DeliveryOrigin::is_owner)
                {
                    error_msg = Some("Only your own messages can request wrap-up");
                } else if !Self::has_substantive_staged_content(
                    &s.drafts[pos].text,
                    &s.drafts[pos].content,
                ) {
                    error_msg = Some("empty prompt: no substantive text or content");
                } else if !Self::edit_hold_allows_row(s, id, editing_token) {
                    error_msg = Some("Finish editing queued message first");
                } else {
                    let effective_cmid = cmid.clone().or_else(|| {
                        s.drafts[pos]
                            .cmid
                            .clone()
                            .filter(|cid| !crate::agent_model::is_reserved_call_notice_cmid(cid))
                    });
                    if let Some(op) = effective_cmid.as_deref()
                        && Self::prompt_already_accepted_excluding_draft(s, op, Some(pos))
                    {
                        error_msg =
                            Some("Activation operation id is already in use; draft retained");
                    } else {
                        s.deferred_scheduled_drafts.retain(|d| d != id);
                        if s.editing.as_deref() == Some(id) {
                            s.editing = None;
                            s.editing_token = None;
                        }
                        let (scheduled, _owner_turn, req) = self.stage_regular_draft_activation(
                            session_id,
                            s,
                            pos,
                            effective_cmid,
                            true,
                            trigger,
                            wired,
                        );
                        had_scheduled = scheduled;
                        if req.is_none() {
                            promoted_row_id = s.queue.first().map(|m| m.id.clone());
                        }
                        dispatch = req;
                    }
                }
            } else {
                error_msg = Some("Draft no longer exists");
            }
        }
        if let Some(msg) = error_msg {
            return reject(msg);
        }
        self.notify_human_turn(session_id);
        self.emit_pending(session_id);
        if had_scheduled {
            self.cancel_draft_timer(session_id, id);
            self.broadcast_session(session_id);
        }
        if let Some(req) = dispatch {
            if self.send_dispatch_checked(req) {
                DraftWrapUpOutcome::DispatchedImmediately
            } else {
                DraftWrapUpOutcome::QueuedFallback
            }
        } else if let Some(row_id) = promoted_row_id {
            DraftWrapUpOutcome::QueuedFront { row_id }
        } else {
            DraftWrapUpOutcome::QueuedFallback
        }
    }

    /// Latch whether the session's active turn is a native `/compact` turn
    /// computed from the final dispatched prompt blocks.
    pub fn set_active_turn_compacting(&self, session_id: &str, compacting: bool) {
        if let Some(s) = self.inner.sessions.lock().get_mut(session_id) {
            s.active_turn_compacting = compacting;
        }
    }

    /// Whether the currently active turn is a native `/compact` turn.
    #[must_use]
    pub fn is_active_turn_compacting(&self, session_id: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|s| s.active_turn_compacting)
    }

    fn wrap_up_hint_blocked_locked(s: &Session) -> bool {
        s.meta
            .execution_binding
            .as_ref()
            .is_some_and(|binding| binding.managed().is_some())
            || s.clear_pending
            || s.delivery_hold.is_some()
            || s.editing.is_some()
            || s.active_turn_compacting
            || !matches!(s.compaction_usage, context_rollover::CompactionUsage::Idle)
            || s.meta.paused
            || s.meta.system
            || s.meta.closing
            || s.meta.trashed_at_ms.is_some()
            || s.meta.status != Status::Busy
            || !s.meta.wrap_up_steering
            || !crate::wrap_up::is_antigravity_provider(&s.meta.provider)
    }

    /// Atomic Hub-side wrap-up hint admission fence covering clear, delivery
    /// hold, queue editing, sent or observed compaction, pause, and provider.
    #[must_use]
    pub fn wrap_up_hint_blocked(&self, session_id: &str) -> bool {
        let sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get(session_id) else {
            return true;
        };
        Self::wrap_up_hint_blocked_locked(s)
    }

    /// Re-verify all wrap-up fences AND (when `follow_up` names a promoted
    /// queued row or draft activation) verify that the exact Owner row is still
    /// eligible at queue head — holding `sessions.lock()` across the
    /// non-blocking `enqueue` callback so concurrent edits, reorders,
    /// withdrawals, or steer reservations cannot outlive the hint.
    pub fn try_enqueue_wrap_up_hint(
        &self,
        session_id: &str,
        follow_up: Option<WrapUpFollowUp<'_>>,
        enqueue: impl FnOnce() -> bool,
    ) -> bool {
        let sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get(session_id) else {
            return false;
        };
        if Self::wrap_up_hint_blocked_locked(s) {
            return false;
        }
        if let Some(expected) = follow_up {
            let Some(head) = s.queue.first() else {
                return false;
            };
            let id_matches = match expected {
                WrapUpFollowUp::RowId(row_id) => head.id == row_id,
                WrapUpFollowUp::Cmid(cmid) => head.cmid.as_deref() == Some(cmid),
            };
            if !id_matches
                || !head
                    .origin
                    .as_ref()
                    .is_some_and(crate::prompt_origin::DeliveryOrigin::is_owner)
                || s.steer_reservations.values().any(|r| r.row_id == head.id)
                || self.is_row_withdrawing(session_id, &head.id)
                || !Self::has_substantive_staged_content(&head.text, &head.content)
            {
                return false;
            }
        }
        enqueue()
    }

    /// Front-of-queue admission without cancelling the active turn, returning
    /// all 5 distinct outcomes (`Refused`, `Duplicate`, `DispatchedImmediately`,
    /// `QueuedFront`, `QueuedFallback`).
    pub fn front_submit_with_outcome(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        caller: &crate::prompt_origin::RequestCaller,
    ) -> FrontSubmitOutcome {
        if self.is_managed_child(session_id)
            || self.reject_reserved_notice_cmid(session_id, cmid.as_deref())
        {
            return FrontSubmitOutcome::Refused;
        }

        let initiator = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        let content_origin = Some(initiator.clone());
        let dispatch_origin = crate::prompt_origin::compute_dispatch_origin(
            content_origin.as_ref(),
            Some(&initiator),
        );
        if dispatch_origin.is_owner() {
            self.notify_human_turn(session_id);
        }
        let wired = self.inner.dispatch_tx.lock().is_some();
        let mut dispatch = None;
        let duplicate;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return FrontSubmitOutcome::Refused;
            };
            duplicate = cmid
                .as_deref()
                .filter(|c| Self::prompt_already_accepted(s, c))
                .map(str::to_owned);
            if duplicate.is_some() {
                // fall through to addressed confirmation
            } else if wired
                && Self::ready(s, true)
                && s.queue.is_empty()
                && self.context_allows_dispatch(session_id, cmid.as_deref())
            {
                let dispatch_cmid = hub_dispatch_origin::ensure_dispatch_cmid(cmid);
                s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
                s.in_flight = true;
                s.dispatch_pending = Some(s.in_flight_epoch);
                Self::note_dispatched(s, Some(&dispatch_cmid));
                dispatch = Some(DispatchReq {
                    ownership: DispatchOwnership::new(self, session_id, s.in_flight_epoch),
                    session_id: session_id.to_owned(),
                    text,
                    content,
                    cmid: Some(dispatch_cmid),
                    explicit: true,
                    origin: content_origin,
                    initiator: Some(initiator.clone()),
                    dispatch_origin,
                    trigger: initiator,
                });
            } else {
                let id = self.next_qid();
                s.queue.insert(
                    0,
                    QueuedMessage {
                        id,
                        text,
                        content,
                        cmid,
                        schedule: None,
                        origin: content_origin,
                        initiator: Some(initiator),
                        explicit: None,
                    },
                );
                self.settle_explicit_locked(session_id, s);
            }
        }
        if let Some(cmid) = duplicate {
            self.confirm_delivered(session_id, &cmid);
            return FrontSubmitOutcome::Duplicate;
        }
        match dispatch {
            Some(req) => {
                if self.send_dispatch_checked(req) {
                    FrontSubmitOutcome::DispatchedImmediately
                } else {
                    FrontSubmitOutcome::QueuedFallback
                }
            }
            None => {
                self.emit_pending(session_id);
                FrontSubmitOutcome::QueuedFront
            }
        }
    }

    pub(super) fn send_dispatch_checked(&self, req: DispatchReq) -> bool {
        let tx = self.inner.dispatch_tx.lock().clone();
        match tx {
            Some(tx) => match tx.try_send(req) {
                Ok(()) => true,
                Err(error) => {
                    self.retain_rejected_dispatch(error.into_inner());
                    false
                }
            },
            None => {
                self.requeue_prompt(
                    &req.session_id,
                    req.text,
                    req.content,
                    req.cmid,
                    req.origin,
                    req.initiator,
                );
                self.broadcast_error(
                    Some(req.session_id),
                    "dispatch queue is unavailable; prompt remains queued".to_owned(),
                );
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_origin::RequestCaller;

    fn add_session(hub: &Hub, id: &str) {
        hub.create_local_session(
            id.to_owned(),
            "antigravity".to_owned(),
            "/tmp".to_owned(),
            id.to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.set_status(id, Status::Running, None);
    }

    fn idle_hub() -> Hub {
        let hub = Hub::new();
        add_session(&hub, "s1");
        hub
    }

    #[test]
    fn front_submit_covers_all_five_outcomes() {
        let hub = idle_hub();
        let person = RequestCaller::Person;
        assert_eq!(
            hub.front_submit_with_outcome(
                "missing",
                "hi".into(),
                vec![],
                Some("c0".into()),
                &person
            ),
            FrontSubmitOutcome::Refused
        );
        assert_eq!(
            hub.front_submit_with_outcome(
                "s1",
                "hi".into(),
                vec![],
                Some("call-notice:bad".into()),
                &person
            ),
            FrontSubmitOutcome::Refused
        );

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        hub.set_dispatch_tx(tx.clone());

        // Idle + empty queue -> DispatchedImmediately
        assert_eq!(
            hub.front_submit_with_outcome("s1", "first".into(), vec![], Some("c1".into()), &person),
            FrontSubmitOutcome::DispatchedImmediately
        );
        // Same cmid -> Duplicate
        assert_eq!(
            hub.front_submit_with_outcome("s1", "first".into(), vec![], Some("c1".into()), &person),
            FrontSubmitOutcome::Duplicate
        );

        // Busy while first is in flight -> QueuedFront
        hub.set_status("s1", Status::Busy, None);
        assert_eq!(
            hub.front_submit_with_outcome(
                "s1",
                "second".into(),
                vec![],
                Some("c2".into()),
                &person
            ),
            FrontSubmitOutcome::QueuedFront
        );
        assert_eq!(
            hub.front_submit_with_outcome(
                "s1",
                "second".into(),
                vec![],
                Some("c2".into()),
                &person
            ),
            FrontSubmitOutcome::Duplicate
        );

        // Drain first and test TrySendError::Full -> QueuedFallback
        let _first = rx.try_recv().unwrap();
        let hub_full = idle_hub();
        let (full_tx, _full_rx) = tokio::sync::mpsc::channel(1);
        add_session(&hub_full, "s2");
        hub_full.set_dispatch_tx(full_tx);
        assert_eq!(
            hub_full.front_submit_with_outcome(
                "s2",
                "fill".into(),
                vec![],
                Some("f1".into()),
                &person
            ),
            FrontSubmitOutcome::DispatchedImmediately
        );
        assert_eq!(
            hub_full.front_submit_with_outcome(
                "s1",
                "overflow".into(),
                vec![],
                Some("f2".into()),
                &person
            ),
            FrontSubmitOutcome::QueuedFallback
        );
        assert_eq!(
            hub_full.queue_and_drafts("s1", |_| true).unwrap().0.len(),
            1
        );

        // Test TrySendError::Closed -> QueuedFallback
        let hub_closed = idle_hub();
        let (closed_tx, closed_rx) = tokio::sync::mpsc::channel(1);
        drop(closed_rx);
        hub_closed.set_dispatch_tx(closed_tx);
        assert_eq!(
            hub_closed.front_submit_with_outcome(
                "s1",
                "closed".into(),
                vec![],
                Some("cl1".into()),
                &person
            ),
            FrontSubmitOutcome::QueuedFallback
        );
        assert_eq!(
            hub_closed.queue_and_drafts("s1", |_| true).unwrap().0.len(),
            1
        );
    }

    #[test]
    fn wrap_up_hint_blocked_fences_clear_delivery_hold_and_observed_compaction() {
        let hub = idle_hub();
        hub.set_status("s1", Status::Busy, None);
        hub.set_wrap_up_steering("s1", true);
        assert!(!hub.wrap_up_hint_blocked("s1"));

        hub.inner
            .sessions
            .lock()
            .get_mut("s1")
            .unwrap()
            .clear_pending = true;
        assert!(hub.wrap_up_hint_blocked("s1"));
        hub.inner
            .sessions
            .lock()
            .get_mut("s1")
            .unwrap()
            .clear_pending = false;
        assert!(!hub.wrap_up_hint_blocked("s1"));

        hub.publish_delivery_hold("s1", Some("interrupt-op".into()));
        assert!(hub.wrap_up_hint_blocked("s1"));
        hub.publish_delivery_hold("s1", None);
        assert!(!hub.wrap_up_hint_blocked("s1"));

        hub.inner
            .sessions
            .lock()
            .get_mut("s1")
            .unwrap()
            .compaction_usage = context_rollover::CompactionUsage::Running { reported: false };
        assert!(hub.wrap_up_hint_blocked("s1"));
        hub.inner
            .sessions
            .lock()
            .get_mut("s1")
            .unwrap()
            .compaction_usage = context_rollover::CompactionUsage::Unmeasured;
        assert!(hub.wrap_up_hint_blocked("s1"));
        hub.inner
            .sessions
            .lock()
            .get_mut("s1")
            .unwrap()
            .compaction_usage = context_rollover::CompactionUsage::Idle;
        assert!(!hub.wrap_up_hint_blocked("s1"));
    }

    #[test]
    fn wrap_up_queued_row_enforces_owner_origin_editing_lease_pause_content_and_dispatch() {
        let hub = idle_hub();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        hub.set_dispatch_tx(tx.clone());
        hub.set_status("s1", Status::Busy, None);
        hub.set_wrap_up_steering("s1", true);

        let person = crate::prompt_origin::RequestCaller::Person;
        let peer = crate::prompt_origin::RequestCaller::agent("other-sess");
        let automation = crate::prompt_origin::RequestCaller::Automation;

        hub.submit("s1", "from peer".into(), vec![], Some("p1".into()), &peer);
        hub.submit(
            "s1",
            "from owner".into(),
            vec![],
            Some("o1".into()),
            &person,
        );
        {
            let mut sessions = hub.inner.sessions.lock();
            let s = sessions.get_mut("s1").unwrap();
            s.queue.push(QueuedMessage {
                id: "q-legacy".into(),
                text: "legacy row".into(),
                content: vec![],
                cmid: None,
                schedule: None,
                origin: None,
                initiator: None,
                explicit: None,
            });
            s.queue.push(QueuedMessage {
                id: "q-malformed".into(),
                text: "fallback ignored when content non-empty".into(),
                content: vec![serde_json::json!({"type": "bogus_block", "foo": 1})],
                cmid: None,
                schedule: None,
                origin: Some(crate::prompt_origin::DeliveryOrigin::Owner),
                initiator: Some(crate::prompt_origin::DeliveryOrigin::Owner),
                explicit: None,
            });
        }
        let q = hub.queue_and_drafts("s1", |_| true).unwrap().0;
        let peer_id = q[0].id.clone();
        let owner_id = q[1].id.clone();

        assert_eq!(
            hub.wrap_up_queued_row("s1", &owner_id, &automation, None),
            QueuedWrapUpOutcome::ForbiddenOrigin
        );
        assert_eq!(
            hub.wrap_up_queued_row("s1", &owner_id, &peer, None),
            QueuedWrapUpOutcome::ForbiddenOrigin
        );
        assert_eq!(
            hub.wrap_up_queued_row("s1", &peer_id, &person, None),
            QueuedWrapUpOutcome::ForbiddenOrigin
        );
        assert_eq!(
            hub.wrap_up_queued_row("s1", "q-legacy", &person, None),
            QueuedWrapUpOutcome::ForbiddenOrigin
        );
        assert_eq!(
            hub.wrap_up_queued_row("s1", "q-malformed", &person, None),
            QueuedWrapUpOutcome::EmptyContent
        );
        hub.set_queue_editing_with_token("s1", Some(peer_id.clone()), Some("tok-peer".into()));
        assert_eq!(hub.editing_id("s1").as_deref(), Some(peer_id.as_str()));
        assert_eq!(hub.editing_token("s1").as_deref(), Some("tok-peer"));
        assert!(matches!(
            hub.wrap_up_queued_row("s1", &owner_id, &person, Some("tok-peer")),
            QueuedWrapUpOutcome::Blocked(_)
        ));
        hub.set_queue_editing_with_token("s1", Some(owner_id.clone()), Some("tok-tab-b".into()));
        assert!(matches!(
            hub.wrap_up_queued_row("s1", &owner_id, &person, Some("tok-tab-a")),
            QueuedWrapUpOutcome::Blocked(_)
        ));
        assert!(matches!(
            hub.wrap_up_queued_row("s1", &owner_id, &person, None),
            QueuedWrapUpOutcome::Blocked(_)
        ));
        assert_eq!(
            hub.wrap_up_queued_row("s1", &owner_id, &person, Some("tok-tab-b")),
            QueuedWrapUpOutcome::QueuedFront
        );
        assert_eq!(hub.editing_id("s1"), None);
        assert_eq!(hub.editing_token("s1"), None);
        let q_after = hub.queue_and_drafts("s1", |_| true).unwrap().0;
        assert_eq!(q_after[0].id, owner_id);
        assert!(
            hub.try_enqueue_wrap_up_hint("s1", Some(WrapUpFollowUp::RowId(&owner_id)), || true)
        );
        assert!(
            !hub.try_enqueue_wrap_up_hint("s1", Some(WrapUpFollowUp::RowId(&peer_id)), || true)
        );
        hub.set_paused("s1", true);
        assert!(matches!(
            hub.wrap_up_queued_row("s1", &owner_id, &person, None),
            QueuedWrapUpOutcome::Blocked(_)
        ));
        hub.set_paused("s1", false);
        let (closed_tx, closed_rx) = tokio::sync::mpsc::channel(1);
        drop(closed_rx);
        hub.set_queue_editing_with_token("s1", Some(owner_id.clone()), Some("tok-idle".into()));
        hub.set_status("s1", Status::Running, None);
        hub.set_dispatch_tx(closed_tx);
        assert_eq!(
            hub.wrap_up_queued_row("s1", &owner_id, &person, Some("tok-idle")),
            QueuedWrapUpOutcome::QueuedFallback
        );
        let new_head = hub.queue_and_drafts("s1", |_| true).unwrap().0[0]
            .id
            .clone();
        hub.set_queue_editing_with_token("s1", Some(new_head.clone()), Some("tok-go".into()));
        hub.set_dispatch_tx(tx);
        assert!(rx.try_recv().is_err());
        assert_eq!(
            hub.wrap_up_queued_row("s1", &new_head, &person, Some("tok-go")),
            QueuedWrapUpOutcome::DispatchedImmediately
        );
        assert_eq!(hub.editing_id("s1"), None);
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn activate_draft_and_recovery_wrap_up_enforce_owner_origin_lease_and_addressed_rejection() {
        let hub = idle_hub();
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("s1", Status::Busy, None);
        let mut events = hub.subscribe();

        let person = crate::prompt_origin::RequestCaller::Person;
        let peer = crate::prompt_origin::RequestCaller::agent("other-sess");
        hub.add_draft(
            "s1",
            "peer draft".into(),
            vec![],
            Some("d-peer".into()),
            &peer,
        );
        hub.add_draft(
            "s1",
            "owner draft".into(),
            vec![],
            Some("d-owner".into()),
            &person,
        );
        hub.submit(
            "s1",
            "existing queued".into(),
            vec![],
            Some("q-existing".into()),
            &person,
        );
        {
            let mut sessions = hub.inner.sessions.lock();
            let s = sessions.get_mut("s1").unwrap();
            s.drafts.push(QueuedMessage {
                id: "recovery-peer".into(),
                text: "peer recovery".into(),
                content: vec![],
                cmid: Some("rec-old".into()),
                schedule: None,
                origin: Some(crate::prompt_origin::DeliveryOrigin::peer("other")),
                initiator: None,
                explicit: None,
            });
        }
        assert_eq!(
            hub.validate_recovery_draft_for_wrap_up("s1", "recovery-peer", None),
            Err("Only your own messages can request wrap-up")
        );

        let drafts = hub.queue_and_drafts("s1", |_| true).unwrap().1;
        let peer_draft = drafts[0].id.clone();
        let owner_draft = drafts[1].id.clone();

        assert!(matches!(
            hub.activate_draft_wrap_up("s1", &peer_draft, Some("act-1".into()), &person, None),
            DraftWrapUpOutcome::Refused(_)
        ));
        assert!(matches!(
            hub.activate_draft_wrap_up("s1", &owner_draft, Some("act-2".into()), &peer, None),
            DraftWrapUpOutcome::Refused(_)
        ));
        let mut saw_act2_reject = false;
        while let Ok(frame) = events.try_recv() {
            if let Outbound::CommandResult {
                cmid,
                outcome: CommandOutcome::Rejected,
                ..
            } = frame.outbound()
                && cmid == "act-2"
            {
                saw_act2_reject = true;
            }
        }
        assert!(
            saw_act2_reject,
            "non-Person draft wrap-up refusal must emit addressed command_result"
        );

        hub.set_queue_editing_with_token("s1", Some(owner_draft.clone()), Some("tok-draft".into()));
        assert!(matches!(
            hub.activate_draft_wrap_up(
                "s1",
                &owner_draft,
                Some("act-wrong-tok".into()),
                &person,
                Some("other"),
            ),
            DraftWrapUpOutcome::Refused(_)
        ));
        let outcome = hub.activate_draft_wrap_up(
            "s1",
            &owner_draft,
            Some("act-3".into()),
            &person,
            Some("tok-draft"),
        );
        let promoted_id = match outcome {
            DraftWrapUpOutcome::QueuedFront { row_id } => row_id,
            other => panic!("expected QueuedFront, got {other:?}"),
        };
        let (q, d) = hub.queue_and_drafts("s1", |_| true).unwrap();
        assert_eq!(q[0].id, promoted_id);
        assert_eq!(q[0].text, "owner draft");
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].id, peer_draft);
    }
}
