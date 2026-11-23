//! Session drafts: staging, editing, scheduling, moving, and activation.

use super::*;

impl Hub {
    /// Park a new draft.
    pub fn add_draft(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        caller: &crate::prompt_origin::RequestCaller,
    ) {
        self.add_draft_with_origin(
            session_id,
            text,
            content,
            cmid,
            Some(crate::prompt_origin::DeliveryOrigin::from_caller(caller)),
        );
    }

    pub(in crate::core) fn add_draft_with_origin(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        origin: Option<crate::prompt_origin::DeliveryOrigin>,
    ) {
        if self.reject_reserved_notice_cmid(session_id, cmid.as_deref()) {
            return;
        }
        let duplicate;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            duplicate = cmid
                .as_deref()
                .filter(|c| s.drafts.iter().any(|m| m.cmid.as_deref() == Some(*c)))
                .map(str::to_owned);
            if duplicate.is_none() {
                let id = self.next_qid();
                s.drafts.push(QueuedMessage {
                    id,
                    text,
                    content,
                    cmid,
                    schedule: None,
                    origin,
                    initiator: None,
                });
            }
        }
        if let Some(cmid) = duplicate {
            self.confirm_delivered(session_id, &cmid);
            return;
        }
        self.emit_pending(session_id);
    }

    /// Attach (or update) a future fire time on a draft — the user-driven analog
    /// of the agent's `ScheduleWakeup`. Targets an existing draft by `id`, else
    /// by `cmid`, else creates a fresh draft carrying the schedule. `text`/
    /// `content` overwrite the target only when non-empty (a reschedule-in-place
    /// from the chip passes the current text; a bare time-change can pass empty).
    /// Persists via the drafts jsonb and arms the server-side timer. Broadcasts
    /// the session list too so the row clock badge updates promptly.
    #[allow(clippy::too_many_arguments)]
    pub fn schedule_draft(
        &self,
        session_id: &str,
        id: Option<String>,
        cmid: Option<String>,
        text: String,
        content: Vec<serde_json::Value>,
        fire_at_ms: i64,
        delivery: Delivery,
        caller: &crate::prompt_origin::RequestCaller,
    ) {
        if self.reject_reserved_notice_cmid(session_id, cmid.as_deref()) {
            return;
        }
        let author = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        let draft_id = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let pos = id
                .as_deref()
                .and_then(|i| s.drafts.iter().position(|m| m.id == i))
                .or_else(|| {
                    cmid.as_deref()
                        .and_then(|c| s.drafts.iter().position(|m| m.cmid.as_deref() == Some(c)))
                });
            let schedule = Some(DraftSchedule {
                fire_at_ms,
                delivery,
            });
            match pos {
                Some(p) => {
                    let m = &mut s.drafts[p];
                    if !text.trim().is_empty() || !content.is_empty() {
                        m.text = text;
                        m.content = content;
                        m.origin = Some(author);
                    }
                    m.initiator = None;
                    m.schedule = schedule;
                    m.id.clone()
                }
                None => {
                    let did = self.next_qid();
                    s.drafts.push(QueuedMessage {
                        id: did.clone(),
                        text,
                        content,
                        cmid,
                        schedule,
                        origin: Some(author),
                        initiator: None,
                    });
                    did
                }
            }
        };
        self.emit_pending(session_id);
        self.broadcast_sessions();
        self.arm_draft_timer(session_id, &draft_id, fire_at_ms);
    }

    /// Strip the schedule off a draft, leaving it a plain parked draft, and cancel
    /// its timer. No-op if the draft is gone or wasn't scheduled.
    pub fn unschedule_draft(&self, session_id: &str, id: &str) {
        let cleared = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            match s.drafts.iter_mut().find(|m| m.id == id) {
                Some(m) if m.schedule.is_some() => {
                    m.schedule = None;
                    true
                }
                _ => false,
            }
        };
        if cleared {
            self.emit_pending(session_id);
            self.broadcast_sessions();
            self.cancel_draft_timer(session_id, id);
        }
    }

    /// Fire a scheduled draft (called by the scheduler at its fire time): remove
    /// it from drafts and submit it per its `delivery`. Tagged with `SCHED_PREFIX`
    /// so the echo renders as a "↻ scheduled" note. No-op if the draft is gone
    /// (the user removed/activated it before it fired — the timer was cancelled,
    /// but a fire already in-flight is harmless here).
    pub fn fire_scheduled_draft(&self, session_id: &str, draft_id: &str) {
        if self.is_managed_child(session_id) {
            return;
        }
        let fired = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            // A trashed session keeps its scheduled Drafts unsent; they stay
            // visible (overdue) if the session is restored. Checked under the
            // lock that removes the Draft, so a concurrent Trash cannot slip in.
            if s.meta.trashed_at_ms.is_some() {
                return;
            }
            s.drafts
                .iter()
                .position(|m| m.id == draft_id)
                .map(|pos| s.drafts.remove(pos))
        };
        let Some(m) = fired else {
            return;
        };
        // The draft left the drafts list → refresh the pending panel and the
        // session list (its next_schedule_ms just changed).
        self.emit_pending(session_id);
        self.broadcast_sessions();

        let front = m
            .schedule
            .as_ref()
            .is_some_and(|sc| sc.delivery == Delivery::Front);
        let cmid = Some(format!("{SCHED_PREFIX}{session_id}-{draft_id}"));

        // Land it in the queue. A scheduled fire ALWAYS respects a paused queue —
        // it never bypasses the ⏸ hold — and never interrupts a live turn. It
        // dispatches straight through ONLY when the session is idle, unpaused, and
        // the queue is empty; otherwise it enqueues (head for Front, tail for Back)
        // and the normal drain runs it when the queue resumes / the turn ends.
        let wired = self.inner.dispatch_tx.lock().is_some();
        let mut dispatch = None;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if let Some(c) = cmid.as_deref()
                && s.queue.iter().any(|q| q.cmid.as_deref() == Some(c))
            {
                return;
            }
            let trigger = crate::prompt_origin::DeliveryOrigin::cowboy("schedule");
            let dispatch_origin =
                crate::prompt_origin::compute_dispatch_origin(m.origin.as_ref(), Some(&trigger));
            if wired
                && !s.meta.paused
                && Self::ready(s, true)
                && s.queue.is_empty()
                && self.context_allows_dispatch(session_id, cmid.as_deref())
            {
                let dispatch_cmid = hub_dispatch_origin::ensure_dispatch_cmid(cmid);
                s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
                s.in_flight = true;
                s.dispatch_pending = Some(s.in_flight_epoch);
                hub_dispatch_origin::note_dispatch_origin(
                    s,
                    session_id,
                    &dispatch_cmid,
                    dispatch_origin.clone(),
                    trigger.clone(),
                    now_ms(),
                    m.content.len().max(1),
                );
                dispatch = Some(DispatchReq {
                    ownership: DispatchOwnership::new(self, session_id, s.in_flight_epoch),
                    session_id: session_id.to_owned(),
                    text: m.text,
                    content: m.content,
                    cmid: Some(dispatch_cmid),
                    explicit: false,
                    origin: m.origin,
                    initiator: Some(trigger.clone()),
                    dispatch_origin,
                    trigger,
                });
            } else {
                let id = self.next_qid();
                let msg = QueuedMessage {
                    id,
                    text: m.text,
                    content: m.content,
                    cmid,
                    schedule: None,
                    origin: m.origin,
                    initiator: Some(trigger),
                };
                if front {
                    s.queue.insert(0, msg);
                } else {
                    s.queue.push(msg);
                }
            }
        }
        match dispatch {
            Some(req) => self.send_dispatch(req),
            None => self.emit_pending(session_id),
        }
    }

    /// Edit a draft in place. Empty text + content removes it.
    /// Edit a draft in place. Empty text + content removes it. Returns whether
    /// the draft still existed.
    pub fn edit_draft(
        &self,
        session_id: &str,
        id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        caller: &crate::prompt_origin::RequestCaller,
    ) -> bool {
        let editor = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        let (found, unscheduled) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if text.trim().is_empty() && content.is_empty() {
                let had = s.drafts.iter().any(|m| m.id == id && m.schedule.is_some());
                let before = s.drafts.len();
                s.drafts.retain(|m| m.id != id);
                (before != s.drafts.len(), had)
            } else if let Some(m) = s.drafts.iter_mut().find(|m| m.id == id) {
                if m.origin.as_ref() != Some(&editor) {
                    m.cmid = None;
                }
                m.text = text;
                m.content = content;
                m.origin = Some(editor);
                m.initiator = None;
                (true, false)
            } else {
                (false, false)
            }
        };
        self.emit_pending(session_id);
        if unscheduled {
            self.cancel_draft_timer(session_id, id);
            self.broadcast_sessions();
        }
        found
    }

    /// Compare and remove under one lock. A delayed Undo may never remove a
    /// row another device has edited, scheduled or already sent.
    pub fn remove_draft_if_unchanged(
        &self,
        session_id: &str,
        identity: &str,
        text: &str,
        content: &[serde_json::Value],
    ) -> bool {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(session) = sessions.get_mut(session_id) else {
                return false;
            };
            let Some(index) = session
                .drafts
                .iter()
                .position(|row| row.cmid.as_deref() == Some(identity) || row.id == identity)
            else {
                return false;
            };
            let row = &session.drafts[index];
            if row.schedule.is_some() || row.text != text || row.content != content {
                return false;
            }
            session.drafts.remove(index);
        }
        self.emit_pending(session_id);
        true
    }

    /// Drop one draft. Returns whether the draft still existed.
    pub fn remove_draft(&self, session_id: &str, id: &str) -> bool {
        let (removed, unscheduled) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            let had = s.drafts.iter().any(|m| m.id == id && m.schedule.is_some());
            let before = s.drafts.len();
            s.drafts.retain(|m| m.id != id);
            (before != s.drafts.len(), had)
        };
        self.emit_pending(session_id);
        if unscheduled {
            self.cancel_draft_timer(session_id, id);
            self.broadcast_sessions();
        }
        removed
    }

    /// Move a draft out of `from`'s draft list and onto the END of `to`'s. The
    /// "parked in the wrong session" fix. No-op if `from == to`, either session
    /// is unknown, or the id isn't a draft of `from`. Crucially, the draft is
    /// pulled out ONLY when `to` exists, so a bad destination can never drop the
    /// message. Both sessions are persisted + broadcast.
    pub fn move_draft(
        &self,
        from: &str,
        id: &str,
        to: &str,
        caller: &crate::prompt_origin::RequestCaller,
    ) {
        if from == to || self.is_managed_child(to) {
            return;
        }
        let mover = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        let scheduled = {
            let mut sessions = self.inner.sessions.lock();
            let Some(mut msg) = (if sessions.contains_key(to) {
                sessions.get_mut(from).and_then(|s| {
                    s.drafts
                        .iter()
                        .position(|m| m.id == id)
                        .map(|pos| s.drafts.remove(pos))
                })
            } else {
                None
            }) else {
                return;
            };
            if !mover.is_owner() {
                if msg.origin.as_ref() != Some(&mover) {
                    msg.cmid = None;
                }
                msg.origin = Some(mover);
            }
            msg.initiator = None;
            let scheduled = msg.schedule.as_ref().map(|sc| sc.fire_at_ms);
            if let Some(dst) = sessions.get_mut(to) {
                dst.drafts.push(msg);
            }
            scheduled
        };
        self.emit_pending(from);
        self.emit_pending(to);
        // A scheduled draft keeps its fire time across the move — retarget the
        // timer from the source session to the destination (same draft id).
        if let Some(fire_at_ms) = scheduled {
            self.cancel_draft_timer(from, id);
            self.arm_draft_timer(to, id, fire_at_ms);
            self.broadcast_sessions();
        }
    }

    /// Drop a session's whole draft list.
    pub fn clear_drafts(&self, session_id: &str) {
        let scheduled_ids: Vec<String> = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let ids = s
                .drafts
                .iter()
                .filter(|m| m.schedule.is_some())
                .map(|m| m.id.clone())
                .collect();
            s.drafts.clear();
            ids
        };
        self.emit_pending(session_id);
        for did in &scheduled_ids {
            self.cancel_draft_timer(session_id, did);
        }
        if !scheduled_ids.is_empty() {
            self.broadcast_sessions();
        }
    }

    /// Activate one draft: remove it from drafts and submit it (send-or-queue).
    pub fn activate_draft(
        &self,
        session_id: &str,
        id: &str,
        cmid: Option<String>,
        caller: &crate::prompt_origin::RequestCaller,
    ) {
        if self.is_managed_child(session_id)
            || self.reject_reserved_notice_cmid(session_id, cmid.as_deref())
        {
            return;
        }
        let msg = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.drafts
                .iter()
                .position(|m| m.id == id)
                .map(|pos| s.drafts.remove(pos))
        };
        if let Some(m) = msg {
            self.emit_pending(session_id);
            if m.schedule.is_some() {
                self.cancel_draft_timer(session_id, id);
                self.broadcast_sessions();
            }
            let fallback_cmid = m
                .cmid
                .filter(|id| !crate::agent_model::is_reserved_call_notice_cmid(id));
            let trigger = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
            self.submit_inner(
                session_id,
                m.text,
                m.content,
                cmid.or(fallback_cmid),
                false,
                false,
                m.origin,
                trigger,
            );
        }
    }

    pub fn activate_all_drafts(
        &self,
        session_id: &str,
        caller: &crate::prompt_origin::RequestCaller,
    ) {
        if self.is_managed_child(session_id) {
            return;
        }
        let msgs = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            std::mem::take(&mut s.drafts)
        };
        if msgs.is_empty() {
            return;
        }
        self.emit_pending(session_id);
        let trigger = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        let mut had_scheduled = false;
        for m in msgs {
            if m.schedule.is_some() {
                self.cancel_draft_timer(session_id, &m.id);
                had_scheduled = true;
            }
            let cmid = m
                .cmid
                .filter(|id| !crate::agent_model::is_reserved_call_notice_cmid(id));
            self.submit_inner(
                session_id,
                m.text,
                m.content,
                cmid,
                false,
                false,
                m.origin,
                trigger.clone(),
            );
        }
        if had_scheduled {
            self.broadcast_sessions();
        }
    }
}
