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

    /// Activate one draft: validate operation ownership and move from drafts to
    /// queue/dispatch atomically under one `sessions` lock so a reused operation
    /// ID can never discard a draft (Finding 8).
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
        let requested_cmid = cmid.clone();
        let trigger = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        let wired = self.inner.dispatch_tx.lock().is_some();
        let mut dispatch = None;
        let mut durable_recovery: Option<(
            String,
            String,
            QueuedMessage,
            crate::core::hub_managed_calls::StoreAckRx,
        )> = None;
        let mut sync_recovery_drain = false;
        let mut had_scheduled = false;
        let mut is_owner_turn = false;
        let mut error_msg: Option<&'static str> = None;
        let activated = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let Some(pos) = s.drafts.iter().position(|m| m.id == id) else {
                return;
            };
            let target = &s.drafts[pos];
            let is_recovery = target.id.starts_with("recovery-");
            let effective_cmid = cmid.or_else(|| {
                target
                    .cmid
                    .clone()
                    .filter(|cid| !crate::agent_model::is_reserved_call_notice_cmid(cid))
            });
            if is_recovery
                && !effective_cmid.as_deref().is_some_and(|op| {
                    Self::is_authored_cmid(op) && Some(op) != target.cmid.as_deref()
                })
            {
                error_msg =
                    Some("Resending a parked recovery draft requires a new client operation id");
                false
            } else if let Some(op) = effective_cmid.as_deref()
                && Self::prompt_already_accepted_excluding_draft(s, op, Some(pos))
            {
                error_msg = Some("Activation operation id is already in use; draft retained");
                false
            } else if is_recovery {
                match self.stage_recovery_draft_activation(
                    session_id,
                    s,
                    pos,
                    effective_cmid.clone().unwrap_or_default(),
                    trigger,
                ) {
                    Ok((scheduled, pending)) => {
                        had_scheduled = scheduled;
                        if let Some(entry) = pending {
                            durable_recovery = Some(entry);
                        } else {
                            sync_recovery_drain = true;
                        }
                        true
                    }
                    Err(msg) => {
                        error_msg = Some(msg);
                        false
                    }
                }
            } else {
                let (scheduled, owner_turn, req) = self.stage_regular_draft_activation(
                    session_id,
                    s,
                    pos,
                    effective_cmid,
                    trigger,
                    wired,
                );
                had_scheduled = scheduled;
                is_owner_turn = owner_turn;
                dispatch = req;
                true
            }
        };
        if let Some(msg) = error_msg {
            if let Some(op) = requested_cmid.as_deref().filter(|op| !op.is_empty()) {
                self.command_result(session_id, op, CommandOutcome::Rejected, msg);
            }
            self.broadcast_error(Some(session_id.to_owned()), msg.to_owned());
            return;
        }
        if !activated {
            return;
        }
        if let Some((new_qid, op, original, rx)) = durable_recovery {
            self.spawn_recovery_activation_commit(
                session_id.to_owned(),
                id.to_owned(),
                new_qid,
                op,
                original,
                had_scheduled,
                caller.clone(),
                rx,
            );
            return;
        }
        if is_owner_turn {
            self.notify_human_turn(session_id);
        }
        self.emit_pending(session_id);
        if had_scheduled {
            self.cancel_draft_timer(session_id, id);
            self.broadcast_sessions();
        }
        if sync_recovery_drain {
            self.drain_head(session_id, true, true, Some(caller));
        } else if let Some(req) = dispatch {
            self.send_dispatch(req);
        }
    }

    fn stage_regular_draft_activation(
        &self,
        session_id: &str,
        s: &mut Session,
        pos: usize,
        effective_cmid: Option<String>,
        trigger: crate::prompt_origin::DeliveryOrigin,
        wired: bool,
    ) -> (bool, bool, Option<DispatchReq>) {
        let m = s.drafts.remove(pos);
        let had_scheduled = m.schedule.is_some();
        let dispatch_origin =
            crate::prompt_origin::compute_dispatch_origin(m.origin.as_ref(), Some(&trigger));
        let is_owner_turn = dispatch_origin.is_owner();
        let dispatch = if wired
            && !s.meta.paused
            && s.meta.trashed_at_ms.is_none()
            && Self::ready(s, true)
            && s.queue.is_empty()
            && self.context_allows_dispatch(session_id, effective_cmid.as_deref())
        {
            s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
            s.in_flight = true;
            s.dispatch_pending = Some(s.in_flight_epoch);
            let dispatch_cmid = hub_dispatch_origin::ensure_dispatch_cmid(effective_cmid);
            Self::note_dispatched(s, Some(&dispatch_cmid));
            Some(DispatchReq {
                ownership: DispatchOwnership::new(self, session_id, s.in_flight_epoch),
                session_id: session_id.to_owned(),
                text: m.text,
                content: m.content,
                cmid: Some(dispatch_cmid),
                explicit: true,
                origin: m.origin,
                initiator: Some(trigger.clone()),
                dispatch_origin,
                trigger,
            })
        } else {
            s.queue.push(QueuedMessage {
                id: self.next_qid(),
                text: m.text,
                content: m.content,
                cmid: effective_cmid,
                schedule: None,
                origin: m.origin,
                initiator: Some(trigger),
            });
            None
        };
        (had_scheduled, is_owner_turn, dispatch)
    }

    #[allow(clippy::type_complexity)]
    fn stage_recovery_draft_activation(
        &self,
        session_id: &str,
        s: &mut Session,
        pos: usize,
        op: String,
        trigger: crate::prompt_origin::DeliveryOrigin,
    ) -> Result<
        (
            bool,
            Option<(
                String,
                String,
                QueuedMessage,
                crate::core::hub_managed_calls::StoreAckRx,
            )>,
        ),
        &'static str,
    > {
        let original = s.drafts.remove(pos);
        let had_scheduled = original.schedule.is_some();
        let new_qid = self.next_qid();
        s.queue.push(QueuedMessage {
            id: new_qid.clone(),
            text: original.text.clone(),
            content: original.content.clone(),
            cmid: Some(op.clone()),
            schedule: None,
            origin: original.origin.clone(),
            initiator: Some(trigger),
        });
        if self.has_store_writer() && tokio::runtime::Handle::try_current().is_ok() {
            if let Some(rx) = self.send_acked_update_pending_with_retired(
                session_id,
                s.queue.clone(),
                s.drafts.clone(),
                Vec::new(),
                Vec::new(),
                s.pending_retired_dispatches.clone(),
                false,
            ) {
                s.pending_durable_queue_ids.insert(new_qid.clone());
                Ok((had_scheduled, Some((new_qid, op, original, rx))))
            } else {
                s.queue.retain(|m| m.id != new_qid);
                s.drafts.insert(pos.min(s.drafts.len()), original);
                Err("Failed to persist recovery draft activation; draft retained")
            }
        } else {
            Ok((had_scheduled, None))
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_recovery_activation_commit(
        &self,
        sid: String,
        draft_id: String,
        new_qid: String,
        op: String,
        original: QueuedMessage,
        had_scheduled: bool,
        caller: crate::prompt_origin::RequestCaller,
        rx: crate::core::hub_managed_calls::StoreAckRx,
    ) {
        let hub = self.clone();
        self.spawn_teardown_requeue(async move {
            match rx.await {
                Ok(Ok(true)) => {
                    let snapshot = {
                        let mut sessions = hub.inner.sessions.lock();
                        sessions.get_mut(&sid).map(|s| {
                            s.pending_durable_queue_ids.remove(&new_qid);
                            let visible_queue: Vec<QueuedMessage> = s
                                .queue
                                .iter()
                                .filter(|m| !s.pending_durable_queue_ids.contains(&m.id))
                                .cloned()
                                .collect();
                            (visible_queue, s.drafts.clone())
                        })
                    };
                    if let Some((q, d)) = snapshot {
                        hub.broadcast_pending_snapshot(&sid, &q, &d);
                        if had_scheduled {
                            hub.cancel_draft_timer(&sid, &draft_id);
                            hub.broadcast_sessions();
                        }
                        hub.drain_head(&sid, true, true, Some(&caller));
                    }
                }
                _ => {
                    let restored = {
                        let mut sessions = hub.inner.sessions.lock();
                        sessions.get_mut(&sid).is_some_and(|s| {
                            s.pending_durable_queue_ids.remove(&new_qid);
                            let had_row = s.queue.iter().any(|m| m.id == new_qid);
                            s.queue.retain(|m| m.id != new_qid);
                            if had_row && !s.drafts.iter().any(|m| m.id == original.id) {
                                s.drafts.push(original);
                                true
                            } else {
                                false
                            }
                        })
                    };
                    if restored {
                        hub.emit_pending(&sid);
                    }
                    let msg = "Failed to persist recovery draft activation; draft retained";
                    hub.command_result(&sid, &op, CommandOutcome::Rejected, msg);
                    hub.broadcast_error(Some(sid), msg.to_owned());
                }
            }
        });
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
            let mut activatable = Vec::new();
            let mut retained = Vec::new();
            for m in std::mem::take(&mut s.drafts) {
                if m.id.starts_with("recovery-") {
                    retained.push(m);
                } else {
                    activatable.push(m);
                }
            }
            s.drafts = retained;
            activatable
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
