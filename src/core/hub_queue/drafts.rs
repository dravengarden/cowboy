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

    pub(in crate::core) fn add_recovery_draft(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) {
        let (duplicate, updated_setting) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let duplicate = cmid
                .as_deref()
                .filter(|c| {
                    s.committed_recovery_cmids.contains(*c)
                        || s.drafts.iter().any(|m| m.cmid.as_deref() == Some(*c))
                })
                .map(str::to_owned);
            if duplicate.is_none() {
                if let Some(c) = cmid.as_deref() {
                    let alias = format!("recovery-{c}");
                    s.drafts
                        .retain(|m| m.cmid.as_deref() != Some(alias.as_str()));
                    let before_q = s.queue.len();
                    s.queue.retain(|m| m.cmid.as_deref() != Some(c));
                    if before_q != s.queue.len()
                        && s.editing
                            .as_deref()
                            .is_some_and(|ed| !s.queue.iter().any(|m| m.id == ed))
                    {
                        s.editing = None;
                    }
                    hub_dispatch_origin::remove_pending_echo(s, c);
                }
                let id = format!("recovery-{}", self.next_qid());
                s.drafts.push(QueuedMessage {
                    id,
                    text,
                    content,
                    cmid: cmid.clone(),
                    schedule: None,
                    origin: None,
                    initiator: None,
                    explicit: false,
                    wake_dormant: false,
                });
            }
            // Finding 17: update draft and both recovery cmid sets atomically
            // in the same critical section so Clear Drafts can never interleave.
            let updated_setting = cmid.as_deref().filter(|c| !c.is_empty()).and_then(|c| {
                s.committed_recovery_cmids.insert(c.to_owned());
                s.pending_recovery_cmids.remove(c);
                hub_dispatch_origin::remove_pending_echo(s, c);
                if s.recovery_cmids.insert(c.to_owned()) {
                    let mut list: Vec<String> = s.recovery_cmids.iter().cloned().collect();
                    list.sort();
                    Some(list)
                } else {
                    None
                }
            });
            (duplicate, updated_setting)
        };
        if let Some(list) = updated_setting {
            let key = format!(
                "{}{session_id}",
                settings_keys::SESSION_RECOVERY_CMIDS_PREFIX
            );
            if self.has_store_writer() {
                self.load_settings(vec![(key, serde_json::json!(list))]);
            } else {
                self.set_setting(key, serde_json::json!(list));
            }
        }
        if let Some(cmid) = duplicate {
            self.confirm_delivered(session_id, &cmid);
        } else {
            self.emit_pending(session_id);
        }
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
            let browser_recovery_orig = cmid
                .as_deref()
                .and_then(|c| c.strip_prefix("recovery-"))
                .filter(|orig| !orig.is_empty());
            duplicate = cmid
                .as_deref()
                .filter(|c| {
                    s.drafts.iter().any(|m| m.cmid.as_deref() == Some(*c))
                        || browser_recovery_orig.is_some_and(|orig| {
                            s.committed_recovery_cmids.contains(orig)
                                || s.pending_recovery_cmids.contains(orig)
                                || s.drafts.iter().any(|m| m.cmid.as_deref() == Some(orig))
                                || Self::prompt_already_accepted(s, orig)
                        })
                })
                .map(str::to_owned);
            if duplicate.is_none() {
                let id = if browser_recovery_orig.is_some() {
                    format!("recovery-{}", self.next_qid())
                } else {
                    self.next_qid()
                };
                s.drafts.push(QueuedMessage {
                    id,
                    text,
                    content,
                    cmid,
                    schedule: None,
                    origin,
                    initiator: None,
                    explicit: false,
                    wake_dormant: false,
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
        let generation = self.inner.next_qid.fetch_add(1, Ordering::Relaxed);
        let (draft_id, duplicate_recovery) = {
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
                generation,
            });
            match pos {
                Some(p) => {
                    if self.is_row_withdrawing(session_id, &s.drafts[p].id) {
                        return;
                    }
                    let m = &mut s.drafts[p];
                    if !text.trim().is_empty() || !content.is_empty() {
                        m.text = text;
                        m.content = content;
                        m.origin = Some(author);
                    }
                    m.initiator = None;
                    m.schedule = schedule;
                    (m.id.clone(), None)
                }
                None => {
                    let browser_recovery_orig = cmid
                        .as_deref()
                        .and_then(|c| c.strip_prefix("recovery-"))
                        .filter(|orig| !orig.is_empty());
                    if let Some(orig) = browser_recovery_orig
                        && (s.committed_recovery_cmids.contains(orig)
                            || s.pending_recovery_cmids.contains(orig)
                            || s.drafts.iter().any(|m| m.cmid.as_deref() == Some(orig))
                            || Self::prompt_already_accepted(s, orig))
                    {
                        (String::new(), cmid.clone())
                    } else {
                        let did = if browser_recovery_orig.is_some() {
                            format!("recovery-{}", self.next_qid())
                        } else {
                            self.next_qid()
                        };
                        s.drafts.push(QueuedMessage {
                            id: did.clone(),
                            text,
                            content,
                            cmid,
                            schedule,
                            origin: Some(author),
                            initiator: None,
                            explicit: false,
                            wake_dormant: false,
                        });
                        (did, None)
                    }
                }
            }
        };
        if let Some(dup) = duplicate_recovery {
            self.confirm_delivered(session_id, &dup);
            return;
        }
        self.emit_pending(session_id);
        self.broadcast_sessions();
        self.arm_draft_timer(session_id, &draft_id, fire_at_ms, generation);
    }

    /// Strip the schedule off a draft, leaving it a plain parked draft, and cancel
    /// its timer. No-op if the draft is gone or wasn't scheduled.
    pub fn unschedule_draft(&self, session_id: &str, id: &str) {
        let cleared = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if self.is_row_withdrawing(session_id, id) {
                return;
            }
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
    fn finish_scheduled_recovery_activation(
        &self,
        session_id: &str,
        draft_id: &str,
        trigger: crate::prompt_origin::DeliveryOrigin,
        front: bool,
        reserved: Option<(String, Option<(bool, bool)>)>,
    ) {
        let Some((reserved_op, in_memory_post)) = reserved else {
            return;
        };
        if let Some((had_schedule, is_owner)) = in_memory_post {
            self.emit_pending(session_id);
            if had_schedule {
                self.cancel_draft_timer(session_id, draft_id);
                self.broadcast_sessions();
            }
            if is_owner {
                self.notify_human_turn(session_id);
            }
            self.try_drain(session_id);
        } else {
            self.spawn_commit_activated_recovery_draft(
                session_id.to_owned(),
                draft_id.to_owned(),
                reserved_op,
                trigger,
                front,
                false,
                true,
            );
        }
    }

    /// Authoritative current `(fire_at_ms, generation)` for a live, non-trashed
    /// scheduled draft; used by the scheduler to reconcile delayed timer commands
    /// against Hub state (Finding 17).
    pub(crate) fn current_draft_schedule(
        &self,
        session_id: &str,
        draft_id: &str,
    ) -> Option<(i64, u64)> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        if s.meta.trashed_at_ms.is_some() {
            return None;
        }
        s.drafts
            .iter()
            .find(|m| m.id == draft_id)
            .and_then(|m| m.schedule.as_ref().map(|sc| (sc.fire_at_ms, sc.generation)))
    }

    pub fn fire_scheduled_draft(&self, session_id: &str, draft_id: &str) {
        self.fire_scheduled_draft_checked(session_id, draft_id, None);
    }

    /// Fire a scheduled draft after validating trash state, withdrawal fence,
    /// current schedule presence, and (when invoked from the timer loop) the
    /// exact armed `(fire_at_ms, generation)` and wall-clock due time under ONE
    /// `sessions` lock (Finding 11).
    pub(crate) fn fire_scheduled_draft_checked(
        &self,
        session_id: &str,
        draft_id: &str,
        expected: Option<(i64, u64)>,
    ) {
        if self.is_managed_child(session_id) {
            return;
        }
        let now = chrono::Utc::now().timestamp_millis();
        let fired = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if s.meta.trashed_at_ms.is_some() || self.is_row_withdrawing(session_id, draft_id) {
                return;
            }
            let Some(pos) = s.drafts.iter().position(|m| m.id == draft_id) else {
                return;
            };
            let Some(sc) = s.drafts[pos].schedule.as_ref() else {
                return;
            };
            if let Some((expected_fire_at_ms, expected_gen)) = expected
                && (sc.fire_at_ms != expected_fire_at_ms
                    || sc.generation != expected_gen
                    || now < sc.fire_at_ms)
            {
                return;
            }
            if draft_id.starts_with("recovery-") {
                let front = sc.delivery == Delivery::Front;
                let op = format!("{SCHED_PREFIX}{session_id}-{draft_id}");
                let trigger = crate::prompt_origin::DeliveryOrigin::cowboy("schedule");
                let reserved = self.reserve_scheduled_recovery_activation_locked(
                    s,
                    session_id,
                    draft_id,
                    pos,
                    op,
                    trigger.clone(),
                    front,
                );
                drop(sessions);
                self.finish_scheduled_recovery_activation(
                    session_id, draft_id, trigger, front, reserved,
                );
                return;
            }
            Some(s.drafts.remove(pos))
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
                && Self::ready_for(s, false, true)
                && s.queue.is_empty()
                && !self.has_in_flight_pending_commit(session_id)
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
                    wake_dormant: true,
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
                    explicit: false,
                    wake_dormant: true,
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
            if self.is_row_withdrawing(session_id, id) {
                return false;
            }
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
            if self.is_row_withdrawing(session_id, &row.id)
                || row.schedule.is_some()
                || row.text != text
                || row.content != content
            {
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
            if self.is_row_withdrawing(session_id, id) {
                return false;
            }
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
            if self.is_row_withdrawing(from, id) {
                return;
            }
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
            let scheduled = msg
                .schedule
                .as_ref()
                .map(|sc| (sc.fire_at_ms, sc.generation));
            if let Some(dst) = sessions.get_mut(to) {
                dst.drafts.push(msg);
            }
            scheduled
        };
        self.emit_pending(from);
        self.emit_pending(to);
        // A scheduled draft keeps its fire time across the move — retarget the
        // timer from the source session to the destination (same draft id).
        if let Some((fire_at_ms, generation)) = scheduled {
            self.cancel_draft_timer(from, id);
            self.arm_draft_timer(to, id, fire_at_ms, generation);
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
            let mut ids = Vec::new();
            s.drafts.retain(|m| {
                if self.is_row_withdrawing(session_id, &m.id) {
                    return true;
                }
                if m.schedule.is_some() {
                    ids.push(m.id.clone());
                }
                false
            });
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
    /// Recovery drafts atomically validate `cmid` and move into `queue` under
    /// ONE `sessions` lock before emitting one atomic `UpdatePending` (Finding 6).
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
        let trigger = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        if id.starts_with("recovery-") {
            self.activate_recovery_draft_durable(session_id, id, cmid, trigger);
            return;
        }
        let msg = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if self.is_row_withdrawing(session_id, id) {
                return;
            }
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
            let mut retained = Vec::new();
            let mut activated = Vec::new();
            for m in std::mem::take(&mut s.drafts) {
                if m.id.starts_with("recovery-") || self.is_row_withdrawing(session_id, &m.id) {
                    retained.push(m);
                } else {
                    activated.push(m);
                }
            }
            s.drafts = retained;
            activated
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

#[cfg(test)]
mod r10_recovery_tests {
    use super::*;
    use crate::core::{PersistenceHealth, StoreSink, StoreWrite};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn owner_caller() -> crate::prompt_origin::RequestCaller {
        crate::prompt_origin::RequestCaller::Person
    }

    fn pending_of(hub: &Hub, id: &str) -> (Vec<QueuedMessage>, Vec<QueuedMessage>) {
        hub.queue_and_drafts(id, |_| true).unwrap()
    }

    fn seed_session(hub: &Hub, id: &str, status: Status) {
        hub.create_local_session(
            id.to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "r10".to_owned(),
            crate::model::SessionOrigin::Web,
            false,
        );
        hub.set_status(id, status, None);
    }

    #[tokio::test]
    async fn scheduled_recovery_draft_blocked_during_activation_and_rearms_on_sql_abort() {
        let health = Arc::new(PersistenceHealth::default());
        let (sink, mut store_rx) = StoreSink::channel(16, health);
        let hub = Hub::with_store(Some(sink));
        let acks = hub.store_ack_sink();
        let (sched_tx, mut sched_rx) = mpsc::channel(16);
        let (disp_tx, mut disp_rx) = mpsc::channel(16);
        hub.set_scheduler_tx(sched_tx);
        hub.set_dispatch_tx(disp_tx);
        seed_session(&hub, "s-f1", Status::Running);

        // Seed recovery draft + schedule it.
        hub.add_recovery_draft("s-f1", "recovered".into(), vec![], Some("orig-1".into()));
        let draft_id = pending_of(&hub, "s-f1").1[0].id.clone();
        hub.schedule_draft(
            "s-f1",
            Some(draft_id.clone()),
            None,
            String::new(),
            vec![],
            1_900_000_000_000,
            Delivery::Back,
            &owner_caller(),
        );
        while sched_rx.try_recv().is_ok() {}
        while let Ok(w) = store_rx.try_recv() {
            if let StoreWrite::UpdatePending {
                ack_id: Some(id), ..
            } = w
            {
                acks.complete(id, Ok(true));
            }
        }

        // Start durable activation (holds withdrawing_row_ids + in_flight_pending_commits).
        hub.activate_draft("s-f1", &draft_id, Some("resend-1".into()), &owner_caller());
        let ack_id = loop {
            if let StoreWrite::UpdatePending {
                ack_id: Some(id), ..
            } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                break id;
            }
        };

        // While activation commit is in flight, scheduled fire + all draft mutators MUST be fenced.
        hub.fire_scheduled_draft("s-f1", &draft_id);
        assert!(!hub.edit_draft("s-f1", &draft_id, "hack".into(), vec![], &owner_caller()));
        assert!(!hub.remove_draft("s-f1", &draft_id));
        hub.unschedule_draft("s-f1", &draft_id);
        assert!(
            disp_rx.try_recv().is_err(),
            "must not dispatch while activation is uncommitted"
        );

        // Abort the SQL write -> draft stays, schedule timer is re-armed.
        acks.complete(ack_id, Err("db down".into()));
        let mut rearmed = false;
        for _ in 0..10 {
            match tokio::time::timeout(std::time::Duration::from_millis(200), sched_rx.recv()).await
            {
                Ok(Some(crate::scheduler::ScheduleCmd::ArmDraft {
                    session_id,
                    draft_id: did,
                    fire_at_ms,
                    ..
                })) if session_id == "s-f1"
                    && did == draft_id
                    && fire_at_ms == 1_900_000_000_000 =>
                {
                    rearmed = true;
                    break;
                }
                Ok(Some(_)) => {}
                _ => break,
            }
        }
        assert!(
            rearmed,
            "aborted scheduled recovery activation must re-arm draft timer"
        );
        assert_eq!(pending_of(&hub, "s-f1").1.len(), 1);
        assert!(pending_of(&hub, "s-f1").0.is_empty());
    }

    #[tokio::test]
    async fn in_flight_commit_blocks_drain_and_explicit_recovery_row_survives_pause() {
        let health = Arc::new(PersistenceHealth::default());
        let (sink, mut store_rx) = StoreSink::channel(32, health);
        let hub = Hub::with_store(Some(sink));
        let acks = hub.store_ack_sink();
        let (disp_tx, mut disp_rx) = mpsc::channel(16);
        hub.set_dispatch_tx(disp_tx);
        seed_session(&hub, "s-f3", Status::Exited);

        // Park a recovery draft and pause the session before activating.
        hub.add_recovery_draft(
            "s-f3",
            "explicit-revive".into(),
            vec![],
            Some("orig-3".into()),
        );
        let draft_id = pending_of(&hub, "s-f3").1[0].id.clone();
        hub.set_paused("s-f3", true);
        while store_rx.try_recv().is_ok() {}

        // Start durable activation while paused -> holds in_flight_pending_commits.
        hub.activate_draft("s-f3", &draft_id, Some("resend-3".into()), &owner_caller());
        let ack_id = loop {
            if let StoreWrite::UpdatePending {
                ack_id: Some(id), ..
            } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                break id;
            }
        };

        // Even if drain is attempted during in_flight_pending_commits, nothing dispatches yet.
        hub.try_drain("s-f3");
        assert!(disp_rx.try_recv().is_err());

        // Commit activation -> row lands in queue with `explicit: true`, still held by pause.
        acks.complete(ack_id, Ok(true));
        for _ in 0..40 {
            if !pending_of(&hub, "s-f3").0.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let (queue, drafts) = pending_of(&hub, "s-f3");
        assert!(drafts.is_empty());
        assert_eq!(queue.len(), 1);
        assert!(
            queue[0].explicit,
            "activated recovery row must persist explicit intent"
        );
        let roundtrip: QueuedMessage =
            serde_json::from_value(serde_json::to_value(&queue[0]).unwrap()).unwrap();
        assert!(roundtrip.explicit);
        assert!(disp_rx.try_recv().is_err(), "paused queue must hold row");

        // Unpause -> try_drain revives the Exited session with `DispatchReq.explicit == true`.
        hub.set_paused("s-f3", false);
        let req = tokio::time::timeout(std::time::Duration::from_secs(1), disp_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            req.explicit,
            "deferred recovery row must keep explicit=true after unpause"
        );
        assert_eq!(req.cmid.as_deref(), Some("resend-3"));
    }

    #[tokio::test]
    async fn r12_shutdown_timeout_poisons_session_blocking_all_subsequent_sql_writes() {
        let health = Arc::new(PersistenceHealth::default());
        let (sink, mut store_rx) = StoreSink::channel(32, health);
        let hub = Hub::with_store(Some(sink));
        seed_session(&hub, "s-r12-1", Status::Running);

        let retired = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let r1 = Arc::clone(&retired);
        let r2 = Arc::clone(&retired);
        // With a store writer attached, enqueue returns false until authoritative SQL ACK.
        assert!(!hub.enqueue_durable_recovery_park(
            "s-r12-1".into(),
            "unresolved payload".into(),
            vec![],
            Some("cmid-r12-1".into()),
            "disconnect during outage".into(),
            Some(Arc::new(move || {
                r1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })),
        ));
        // Duplicate enqueue before SQL commits must also return false (never early-retire).
        assert!(!hub.enqueue_durable_recovery_park(
            "s-r12-1".into(),
            "unresolved payload".into(),
            vec![],
            Some("cmid-r12-1".into()),
            "disconnect during outage".into(),
            Some(Arc::new(move || {
                r2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })),
        ));
        let mut acked_seen = 0usize;
        while let Ok(Some(w)) =
            tokio::time::timeout(std::time::Duration::from_millis(300), store_rx.recv()).await
        {
            if let StoreWrite::UpdatePending {
                ack_id: Some(_), ..
            } = w
            {
                acked_seen += 1;
                break;
            }
        }
        assert_eq!(acked_seen, 1);
        assert_eq!(retired.load(std::sync::atomic::Ordering::SeqCst), 0);

        // Unrelated mutation while commit is in flight is deferred.
        hub.add_draft("s-r12-1", "unrelated".into(), vec![], None, &owner_caller());

        // Graceful shutdown bounds wait on hung SQL and permanently poisons the session.
        let started = std::time::Instant::now();
        hub.drain_teardown_requeues(std::time::Duration::from_millis(50))
            .await;
        assert!(
            started.elapsed() < std::time::Duration::from_secs(4),
            "shutdown must not hang on stalled recovery park ack"
        );
        assert_eq!(
            retired.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "timed-out park must never run retirement callbacks"
        );

        // After poisoning, neither fire-and-forget nor acknowledged UpdatePending may be admitted.
        hub.add_draft(
            "s-r12-1",
            "after-timeout".into(),
            vec![],
            None,
            &owner_caller(),
        );
        assert!(matches!(
            hub.preview_park_recovery_prompt(
                "s-r12-1",
                "second".into(),
                vec![],
                Some("cmid-r12-2".into())
            ),
            crate::core::RecoveryDraftPark::Rejected(_)
        ));
        while let Ok(w) = store_rx.try_recv() {
            assert!(
                !matches!(w, StoreWrite::UpdatePending { .. }),
                "poisoned session must NEVER emit any further UpdatePending behind timed-out commit"
            );
        }
    }

    #[test]
    fn settled_recovery_cmid_not_recreated_in_memory_and_confirmed_in_resync_and_outbox() {
        let hub = Hub::new();
        seed_session(&hub, "s-f10", Status::Running);
        hub.add_recovery_draft("s-f10", "once".into(), vec![], Some("cmid-settled".into()));
        let draft_id = pending_of(&hub, "s-f10").1[0].id.clone();
        assert!(hub.remove_draft("s-f10", &draft_id));
        assert!(pending_of(&hub, "s-f10").1.is_empty());

        // Finding 10: calling add_recovery_draft again for a settled cmid must NOT recreate it.
        let mut rx = hub.subscribe();
        hub.add_recovery_draft("s-f10", "once".into(), vec![], Some("cmid-settled".into()));
        assert!(pending_of(&hub, "s-f10").1.is_empty());

        // Finding 11: confirm_delivered broadcast AND queue_resync must both confirm "cmid-settled".
        let mut saw_confirm = false;
        while let Ok(frame) = rx.try_recv() {
            if let Outbound::SyncPatch { confirmed, .. } = &**frame
                && confirmed.iter().any(|c| c == "cmid-settled")
            {
                saw_confirm = true;
            }
        }
        assert!(
            saw_confirm,
            "confirm_delivered must confirm settled recovery cmid"
        );

        let Some(Outbound::SyncPatch { confirmed, .. }) = hub.queue_resync("s-f10") else {
            panic!("expected SyncPatch from queue_resync");
        };
        assert!(
            confirmed.iter().any(|c| c == "cmid-settled"),
            "queue_resync must confirm settled recovery cmid"
        );
    }

    #[test]
    fn r12_browser_recovery_draft_blocks_undelivered_and_stale_queue_row_advances_fifo() {
        let hub = Hub::new();
        seed_session(&hub, "s-r12-11", Status::Running);
        let (disp_tx, mut disp_rx) = mpsc::channel(8);
        hub.set_dispatch_tx(disp_tx);

        // Finding 11: browser-parked `recovery-orig-11` draft blocks UndeliveredPrompt hydration for `orig-11`.
        hub.add_draft(
            "s-r12-11",
            "parked in browser".into(),
            vec![],
            Some("recovery-orig-11".into()),
            &owner_caller(),
        );
        hub.hydrate_prompt_dispatches(vec![crate::prompt_origin::PromptDispatchRecord {
            session_id: "s-r12-11".into(),
            cmid: "orig-11".into(),
            origin: crate::prompt_origin::DeliveryOrigin::from_caller(&owner_caller()),
            trigger: crate::prompt_origin::DeliveryOrigin::from_caller(&owner_caller()),
            dispatched_at_ms: 1_000,
        }]);
        assert!(
            hub.take_pending_echo_for_undelivered("s-r12-11", "orig-11")
                .is_none(),
            "browser recovery-X draft must suppress UndeliveredPrompt echo for X"
        );

        // Finding 14: stale settled queue row at head is stripped so valid row behind it dispatches.
        hub.set_paused("s-r12-11", true);
        hub.submit(
            "s-r12-11",
            "stale".into(),
            vec![],
            Some("settled-14".into()),
            &owner_caller(),
        );
        hub.submit(
            "s-r12-11",
            "valid".into(),
            vec![],
            Some("valid-14".into()),
            &owner_caller(),
        );
        {
            let mut sessions = hub.inner.sessions.lock();
            sessions
                .get_mut("s-r12-11")
                .unwrap()
                .committed_recovery_cmids
                .insert("settled-14".into());
        }
        hub.set_paused("s-r12-11", false);
        let req = disp_rx
            .try_recv()
            .expect("valid row behind stale settled row must dispatch");
        assert_eq!(req.cmid.as_deref(), Some("valid-14"));

        // Finding 18: scheduled row wakes Exited cleanly but NEVER revives Crashed.
        hub.clear_in_flight("s-r12-11");
        hub.set_status("s-r12-11", Status::Crashed, Some("boom".into()));
        let due_ms = chrono::Utc::now().timestamp_millis() - 1_000;
        hub.schedule_draft(
            "s-r12-11",
            None,
            Some("sched-crashed".into()),
            "wait for human".into(),
            vec![],
            due_ms,
            Delivery::Back,
            &owner_caller(),
        );
        let sched_id = pending_of(&hub, "s-r12-11")
            .1
            .iter()
            .find(|d| d.cmid.as_deref() == Some("sched-crashed"))
            .unwrap()
            .id
            .clone();
        let sched_gen = pending_of(&hub, "s-r12-11")
            .1
            .iter()
            .find(|d| d.id == sched_id)
            .unwrap()
            .schedule
            .as_ref()
            .unwrap()
            .generation;
        assert_eq!(
            hub.current_draft_schedule("s-r12-11", &sched_id),
            Some((due_ms, sched_gen))
        );
        hub.fire_scheduled_draft_checked("s-r12-11", &sched_id, Some((due_ms, sched_gen)));
        assert!(
            disp_rx.try_recv().is_err(),
            "scheduled draft must NOT revive a Crashed session"
        );
        assert_eq!(pending_of(&hub, "s-r12-11").0.len(), 1);
        hub.set_status("s-r12-11", Status::Exited, None);
        hub.try_drain("s-r12-11");
        let woken = disp_rx
            .try_recv()
            .expect("scheduled row must wake cleanly dormant Exited session");
        assert!(woken.wake_dormant);
        assert!(!woken.explicit);
    }

    #[test]
    fn r11_stale_queue_row_fenced_and_browser_recovery_alias_and_undelivered_echo_deduped() {
        let hub = Hub::new();
        seed_session(&hub, "s-r11-5", Status::Running);
        let (disp_tx, mut disp_rx) = mpsc::channel(8);
        hub.set_dispatch_tx(disp_tx);

        hub.set_paused("s-r11-5", true);
        hub.submit(
            "s-r11-5",
            "original".into(),
            vec![],
            Some("cmid-r11-5".into()),
            &owner_caller(),
        );
        hub.record_prompt_dispatch(
            "s-r11-5",
            "cmid-r11-5".into(),
            crate::prompt_origin::DeliveryOrigin::from_caller(&owner_caller()),
            crate::prompt_origin::DeliveryOrigin::from_caller(&owner_caller()),
            1_000,
        );
        hub.add_recovery_draft(
            "s-r11-5",
            "original".into(),
            vec![],
            Some("cmid-r11-5".into()),
        );
        assert!(
            pending_of(&hub, "s-r11-5").0.is_empty(),
            "stale queue row must be stripped"
        );
        assert_eq!(pending_of(&hub, "s-r11-5").1.len(), 1);
        let did = pending_of(&hub, "s-r11-5").1[0].id.clone();
        assert!(hub.remove_draft("s-r11-5", &did));
        assert!(
            hub.take_pending_echo_for_undelivered("s-r11-5", "cmid-r11-5")
                .is_none(),
            "late UndeliveredPrompt must never consume echo of a settled recovery prompt"
        );
        hub.add_draft(
            "s-r11-5",
            "browser replay".into(),
            vec![],
            Some("recovery-cmid-r11-5".into()),
            &owner_caller(),
        );
        assert!(
            pending_of(&hub, "s-r11-5").1.is_empty(),
            "browser recovery-<orig> alias must not resurrect a settled server recovery draft"
        );
        hub.add_draft(
            "s-r11-5",
            "browser only".into(),
            vec![],
            Some("recovery-fresh-op".into()),
            &owner_caller(),
        );
        assert!(pending_of(&hub, "s-r11-5").1[0].id.starts_with("recovery-"));
        hub.set_paused("s-r11-5", false);
        hub.activate_all_drafts("s-r11-5", &owner_caller());
        assert_eq!(
            pending_of(&hub, "s-r11-5").1.len(),
            1,
            "browser recovery draft must be excluded from bulk activate_all_drafts"
        );
        assert!(disp_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn r11_scheduled_recovery_stale_timer_rejected_and_due_fire_wakes_exited_and_deferred_drain_replays()
     {
        let health = Arc::new(PersistenceHealth::default());
        let (sink, mut store_rx) = StoreSink::channel(32, health);
        let hub = Hub::with_store(Some(sink));
        let acks = hub.store_ack_sink();
        let (disp_tx, mut disp_rx) = mpsc::channel(8);
        hub.set_dispatch_tx(disp_tx);
        seed_session(&hub, "s-r11-11", Status::Exited);

        hub.add_recovery_draft("s-r11-11", "wake me".into(), vec![], Some("orig-11".into()));
        let did = pending_of(&hub, "s-r11-11").1[0].id.clone();
        let due_ms = chrono::Utc::now().timestamp_millis() - 1_000;
        hub.schedule_draft(
            "s-r11-11",
            Some(did.clone()),
            None,
            String::new(),
            vec![],
            due_ms,
            Delivery::Back,
            &owner_caller(),
        );
        let gen1 = pending_of(&hub, "s-r11-11").1[0]
            .schedule
            .as_ref()
            .unwrap()
            .generation;
        hub.fire_scheduled_draft_checked("s-r11-11", &did, Some((due_ms, gen1 + 999)));
        assert_eq!(pending_of(&hub, "s-r11-11").1.len(), 1);

        while store_rx.try_recv().is_ok() {}
        hub.fire_scheduled_draft_checked("s-r11-11", &did, Some((due_ms, gen1)));
        let ack_id = loop {
            if let StoreWrite::UpdatePending {
                ack_id: Some(id), ..
            } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                break id;
            }
        };
        acks.complete(ack_id, Ok(true));
        let req = tokio::time::timeout(std::time::Duration::from_secs(1), disp_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            !req.explicit,
            "scheduled recovery fire keeps automatic provenance"
        );
        assert!(
            req.wake_dormant,
            "scheduled recovery fire preserves revive permission"
        );

        hub.clear_in_flight("s-r11-11");
        hub.set_paused("s-r11-11", true);
        hub.submit(
            "s-r11-11",
            "manual-head".into(),
            vec![],
            Some("m-head".into()),
            &owner_caller(),
        );
        hub.add_recovery_draft("s-r11-11", "rec-2".into(), vec![], Some("orig-13".into()));
        let did2 = pending_of(&hub, "s-r11-11").1[0].id.clone();
        while store_rx.try_recv().is_ok() {}
        hub.activate_draft("s-r11-11", &did2, Some("resend-13".into()), &owner_caller());
        let ack2 = loop {
            if let StoreWrite::UpdatePending {
                ack_id: Some(id), ..
            } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                break id;
            }
        };
        hub.drain_now("s-r11-11", &owner_caller());
        assert!(disp_rx.try_recv().is_err());
        acks.complete(ack2, Ok(true));
        let req2 = tokio::time::timeout(std::time::Duration::from_secs(1), disp_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            req2.explicit,
            "deferred manual drain_now must replay bypassing pause"
        );
        assert_eq!(req2.cmid.as_deref(), Some("m-head"));

        // Finding 16: deferred manual drain_now must NOT bypass pause for a different prepended row.
        hub.clear_in_flight("s-r11-11");
        hub.submit(
            "s-r11-11",
            "row-a".into(),
            vec![],
            Some("cmid-a".into()),
            &owner_caller(),
        );
        hub.add_recovery_draft("s-r11-11", "rec-3".into(), vec![], Some("orig-16".into()));
        let did3 = pending_of(&hub, "s-r11-11").1[0].id.clone();
        while store_rx.try_recv().is_ok() {}
        hub.activate_draft("s-r11-11", &did3, Some("resend-16".into()), &owner_caller());
        let ack3 = loop {
            if let StoreWrite::UpdatePending {
                ack_id: Some(id), ..
            } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                break id;
            }
        };
        hub.drain_now("s-r11-11", &owner_caller());
        // Prepend a different row before the fence lifts: deferred manual intent must NOT send it.
        let _ = hub.force_submit(
            "s-r11-11",
            "row-b".into(),
            vec![],
            Some("cmid-b".into()),
            false,
            &owner_caller(),
        );
        acks.complete(ack3, Ok(true));
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(
            disp_rx.try_recv().is_err(),
            "deferred manual drain_now must never dispatch a different prepended row through pause"
        );
    }
}
