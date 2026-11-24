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
            self.broadcast_sessions();
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

    /// Atomic Hub-side wrap-up hint admission fence covering clear, delivery
    /// hold, queue editing, sent or observed compaction, pause, and provider.
    #[must_use]
    pub fn wrap_up_hint_blocked(&self, session_id: &str) -> bool {
        if self.is_managed_child(session_id) {
            return true;
        }
        let sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get(session_id) else {
            return true;
        };
        s.clear_pending
            || s.delivery_hold.is_some()
            || s.editing.is_some()
            || s.active_turn_compacting
            || !matches!(s.compaction_usage, context_rollover::CompactionUsage::Idle)
            || s.meta.paused
            || s.meta.system
            || s.meta.closing
            || s.meta.status != Status::Busy
            || !s.meta.wrap_up_steering
            || !crate::wrap_up::is_antigravity_provider(&s.meta.provider)
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
        if self.has_any_retryable_recovery_park(session_id) {
            self.retry_pending_recovery_parks(Some(session_id));
        }
        if let Some(cid) = cmid.as_deref()
            && self.has_pending_recovery_park_cmid(session_id, cid)
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
                    },
                );
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
                    let req = error.into_inner();
                    tracing::error!(session = %req.session_id, "dispatch queue rejected a prompt");
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
                        "dispatch queue is full; prompt remains queued".to_owned(),
                    );
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
        hub.set_dispatch_tx(tx);

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
}
