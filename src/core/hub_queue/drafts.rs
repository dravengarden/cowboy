//! Session drafts: staging, editing, scheduling, moving, and activation.

use std::sync::Weak;

use super::*;
use crate::core::hub_managed_calls::StoreAckRx;

/// Result of reserving an operation ID + draft slot for a durable recovery-draft activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryReserveErr {
    NotFound,
    InvalidOperationId(&'static str),
    OperationInUse(&'static str),
    AlreadyActivating(&'static str),
}

/// RAII reservation holding `op` in `reserved_recovery_cmids` and `draft_id` in
/// `activating_recovery_drafts` across async SQL history lookup and commit so no
/// competing submit, steer, or draft mutation can claim the ID or delete the draft.
pub struct RecoveryActivationReservation {
    hub: Weak<HubInner>,
    session_id: String,
    draft_id: String,
    op: String,
    active: bool,
}

impl RecoveryActivationReservation {
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[must_use]
    pub fn draft_id(&self) -> &str {
        &self.draft_id
    }

    #[must_use]
    pub fn op(&self) -> &str {
        &self.op
    }

    fn release_in_session(&mut self, s: &mut Session) {
        if self.active {
            self.active = false;
            s.reserved_recovery_cmids.remove(&self.op);
            s.activating_recovery_drafts.remove(&self.draft_id);
        }
    }
}

impl Drop for RecoveryActivationReservation {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        if let Some(inner) = self.hub.upgrade() {
            let mut sessions = inner.sessions.lock();
            if let Some(s) = sessions.get_mut(&self.session_id) {
                s.reserved_recovery_cmids.remove(&self.op);
                s.activating_recovery_drafts.remove(&self.draft_id);
            }
        }
    }
}

/// Non-mutating preview of a durable `recovery-*` draft activation awaiting SQL
/// commit. While in flight, `s.queue` and `s.drafts` retain their committed view
/// and `in_flight_pending_commits` serializes all concurrent pending flushes.
pub struct PendingRecoveryActivationCommit {
    hub: Weak<HubInner>,
    active: bool,
    new_row: QueuedMessage,
    front: bool,
    had_scheduled: bool,
    reservation: RecoveryActivationReservation,
}

impl PendingRecoveryActivationCommit {
    #[must_use]
    pub fn op(&self) -> &str {
        self.reservation.op()
    }
}

impl Drop for PendingRecoveryActivationCommit {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        if let Some(inner) = self.hub.upgrade() {
            let hub = Hub { inner };
            let sid = self.reservation.session_id.clone();
            let mut sessions = hub.inner.sessions.lock();
            let mut claims = hub.inner.notice_claims.lock();
            if let Some(s) = sessions.get_mut(&sid) {
                self.reservation.release_in_session(s);
                hub.finish_in_flight_pending_commit(&sid, &s.queue, &s.drafts, &mut claims);
            } else {
                claims.in_flight_pending_commits.remove(&sid);
            }
        }
    }
}

/// RAII reservation held by competing dispatch/steer paths across their async
/// `prompt_dispatches` SQL write until `Hub::record_prompt_dispatch` completes,
/// closing the symmetric TOCTOU race with recovery-draft activation.
pub struct InFlightDispatchCmidGuard {
    hub: Weak<HubInner>,
    session_id: String,
    cmid: String,
}

impl Drop for InFlightDispatchCmidGuard {
    fn drop(&mut self) {
        if let Some(inner) = self.hub.upgrade() {
            let mut sessions = inner.sessions.lock();
            if let Some(s) = sessions.get_mut(&self.session_id) {
                s.in_flight_dispatch_cmids.remove(&self.cmid);
            }
        }
    }
}

pub enum RecoveryActivationCommitStep {
    CommittedSync,
    Pending(Box<PendingRecoveryActivationCommit>, StoreAckRx),
}

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
                    explicit: None,
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
        if id.as_deref().is_some_and(|i| i.starts_with("recovery-")) {
            let msg = "Parked recovery drafts cannot be scheduled; resend explicitly";
            if let Some(op) = cmid.as_deref().filter(|c| !c.is_empty()) {
                self.command_result(session_id, op, CommandOutcome::Rejected, msg);
            }
            self.broadcast_error(Some(session_id.to_owned()), msg.to_owned());
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
            if let Some(p) = pos
                && (s.drafts[p].id.starts_with("recovery-")
                    || s.activating_recovery_drafts.contains(&s.drafts[p].id))
            {
                return;
            }
            let schedule = Some(DraftSchedule {
                fire_at_ms,
                delivery,
            });
            match pos {
                Some(p) => {
                    let did = s.drafts[p].id.clone();
                    if s.steer_reservations.values().any(|r| r.row_id == did) {
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
                    s.deferred_scheduled_drafts.retain(|d| d != &did);
                    did
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
                        explicit: None,
                    });
                    did
                }
            }
        };
        self.emit_pending(session_id);
        self.broadcast_session(session_id);
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
            if s.activating_recovery_drafts.contains(id)
                || s.steer_reservations.values().any(|r| r.row_id == id)
            {
                return;
            }
            match s.drafts.iter_mut().find(|m| m.id == id) {
                Some(m) if m.schedule.is_some() => {
                    m.schedule = None;
                    s.deferred_scheduled_drafts.retain(|d| d != id);
                    true
                }
                _ => false,
            }
        };
        if cleared {
            self.emit_pending(session_id);
            self.broadcast_session(session_id);
            self.cancel_draft_timer(session_id, id);
        }
    }

    /// Fire a scheduled draft if its current schedule is still due.
    pub fn fire_scheduled_draft(&self, session_id: &str, draft_id: &str) {
        self.fire_scheduled_draft_at(session_id, draft_id, None);
    }

    /// Fire a scheduled draft (called by the scheduler at its fire time, or on
    /// edit-hold release when a timer elapsed during an edit): remove it from
    /// drafts and submit it per its `delivery`. Revalidates the draft's live
    /// schedule and due/expected fire timestamp under the same lock that removes
    /// it, so a cancelled or rescheduled draft can never be fired by a stale
    /// timer callback or an out-of-date deferred release list.
    pub fn fire_scheduled_draft_at(
        &self,
        session_id: &str,
        draft_id: &str,
        expected_fire_at_ms: Option<i64>,
    ) {
        if self.is_managed_child(session_id) || draft_id.starts_with("recovery-") {
            return;
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
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
            let Some(pos) = s.drafts.iter().position(|m| m.id == draft_id) else {
                s.deferred_scheduled_drafts.retain(|id| id != draft_id);
                return;
            };
            let still_due = s.drafts[pos].schedule.as_ref().is_some_and(|sc| {
                expected_fire_at_ms.map_or(sc.fire_at_ms <= now_ms, |expected| {
                    sc.fire_at_ms == expected && sc.fire_at_ms <= now_ms
                })
            });
            if !still_due {
                s.deferred_scheduled_drafts.retain(|id| id != draft_id);
                return;
            }
            // Do not fire stale text out from under an active editor or an
            // in-flight steer reservation. Record the elapsed timer so releasing
            // the hold or reservation fires the draft if still due.
            if s.editing.as_deref() == Some(draft_id)
                || s.activating_recovery_drafts.contains(draft_id)
                || s.steer_reservations.values().any(|r| r.row_id == draft_id)
            {
                if !s.deferred_scheduled_drafts.iter().any(|id| id == draft_id) {
                    s.deferred_scheduled_drafts.push(draft_id.to_owned());
                }
                return;
            }
            s.deferred_scheduled_drafts.retain(|id| id != draft_id);
            Some(s.drafts.remove(pos))
        };
        let Some(m) = fired else {
            return;
        };
        // The draft left the drafts list → refresh the pending panel and the
        // session list (its next_schedule_ms just changed).
        self.emit_pending(session_id);
        self.broadcast_session(session_id);

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
                    explicit: None,
                };
                if front {
                    s.queue.insert(0, msg);
                    self.settle_explicit_locked(session_id, s);
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
            if s.activating_recovery_drafts.contains(id)
                || s.steer_reservations.values().any(|r| r.row_id == id)
            {
                return false;
            }
            if text.trim().is_empty() && content.is_empty() {
                let had = s.drafts.iter().any(|m| m.id == id && m.schedule.is_some());
                let before = s.drafts.len();
                s.drafts.retain(|m| m.id != id);
                s.deferred_scheduled_drafts.retain(|d| d != id);
                if s.editing.as_deref() == Some(id) {
                    s.editing = None;
                    s.editing_token = None;
                }
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
            self.broadcast_session(session_id);
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
            if session.activating_recovery_drafts.contains(&row.id)
                || session
                    .steer_reservations
                    .values()
                    .any(|r| r.row_id == row.id)
                || row.schedule.is_some()
                || row.text != text
                || row.content != content
            {
                return false;
            }
            let removed_id = session.drafts.remove(index).id;
            if session.editing.as_deref() == Some(removed_id.as_str()) {
                session.editing = None;
                session.editing_token = None;
            }
        }
        self.emit_pending(session_id);
        true
    }

    /// Drop one draft. Returns whether the draft still existed.
    pub fn remove_draft(&self, session_id: &str, id: &str) -> bool {
        let (removed, unscheduled, dropped_reservation) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if s.activating_recovery_drafts.contains(id) {
                return false;
            }
            let had = s.drafts.iter().any(|m| m.id == id && m.schedule.is_some());
            let before = s.drafts.len();
            s.drafts.retain(|m| m.id != id);
            s.deferred_scheduled_drafts.retain(|d| d != id);
            let res_before = s.steer_reservations.len();
            s.steer_reservations.retain(|_, r| r.row_id != id);
            if s.editing.as_deref() == Some(id) {
                s.editing = None;
                s.editing_token = None;
            }
            (
                before != s.drafts.len(),
                had,
                res_before != s.steer_reservations.len(),
            )
        };
        if dropped_reservation {
            self.persist_steer_reservations(session_id);
        }
        self.emit_pending(session_id);
        if unscheduled {
            self.cancel_draft_timer(session_id, id);
            self.broadcast_session(session_id);
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
                    if s.activating_recovery_drafts.contains(id)
                        || s.steer_reservations.values().any(|r| r.row_id == id)
                    {
                        return None;
                    }
                    s.deferred_scheduled_drafts.retain(|d| d != id);
                    if s.editing.as_deref() == Some(id) {
                        s.editing = None;
                        s.editing_token = None;
                    }
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
            self.broadcast_session(from);
            self.broadcast_session(to);
        }
    }

    /// Drop a session's whole draft list (preserving drafts held by active steer reservations).
    pub fn clear_drafts(&self, session_id: &str) {
        let scheduled_ids: Vec<String> = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let is_reserved = |id: &str, res: &HashMap<String, SteerReservation>| {
                res.values().any(|r| r.row_id == id)
            };
            let ids = s
                .drafts
                .iter()
                .filter(|m| {
                    m.schedule.is_some()
                        && !s.activating_recovery_drafts.contains(&m.id)
                        && !is_reserved(&m.id, &s.steer_reservations)
                })
                .map(|m| m.id.clone())
                .collect();
            let keep_draft =
                |id: &str,
                 activating: &HashSet<String>,
                 res: &HashMap<String, SteerReservation>| {
                    activating.contains(id) || is_reserved(id, res)
                };
            let editing_cleared = s.editing.as_deref().is_some_and(|id| {
                s.drafts.iter().any(|m| m.id == id)
                    && !keep_draft(id, &s.activating_recovery_drafts, &s.steer_reservations)
            });
            if editing_cleared {
                s.editing = None;
                s.editing_token = None;
            }
            let activating = &s.activating_recovery_drafts;
            let res = &s.steer_reservations;
            s.deferred_scheduled_drafts
                .retain(|d| keep_draft(d, activating, res));
            s.drafts.retain(|m| keep_draft(&m.id, activating, res));
            ids
        };
        self.emit_pending(session_id);
        for did in &scheduled_ids {
            self.cancel_draft_timer(session_id, did);
        }
        if !scheduled_ids.is_empty() {
            self.broadcast_session(session_id);
        }
    }

    /// Reserve `cmid` for an in-flight async `prompt_dispatches` SQL write.
    /// Returns `None` if an active recovery-draft activation has already
    /// reserved `cmid` (or if the same `cmid` is already mid-write).
    pub fn reserve_in_flight_dispatch_cmid(
        &self,
        session_id: &str,
        cmid: &str,
    ) -> Option<InFlightDispatchCmidGuard> {
        let mut sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get_mut(session_id) else {
            // Unknown/unregistered session: return inert guard so callers keep
            // their existing unknown-session behavior.
            return Some(InFlightDispatchCmidGuard {
                hub: Weak::new(),
                session_id: session_id.to_owned(),
                cmid: cmid.to_owned(),
            });
        };
        if s.reserved_recovery_cmids.contains(cmid) || s.in_flight_dispatch_cmids.contains(cmid) {
            return None;
        }
        s.in_flight_dispatch_cmids.insert(cmid.to_owned());
        Some(InFlightDispatchCmidGuard {
            hub: Arc::downgrade(&self.inner),
            session_id: session_id.to_owned(),
            cmid: cmid.to_owned(),
        })
    }

    /// Atomically reserve a fresh authored operation ID and draft slot for a
    /// `recovery-*` draft activation BEFORE running the async SQL history check.
    pub fn reserve_recovery_draft_activation(
        &self,
        session_id: &str,
        draft_id: &str,
        op: &str,
    ) -> Result<RecoveryActivationReservation, RecoveryReserveErr> {
        if !draft_id.starts_with("recovery-") {
            return Err(RecoveryReserveErr::NotFound);
        }
        if !Self::is_authored_cmid(op) {
            return Err(RecoveryReserveErr::InvalidOperationId(
                "Resending a parked recovery draft requires a new client operation id",
            ));
        }
        let mut sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get_mut(session_id) else {
            return Err(RecoveryReserveErr::NotFound);
        };
        let Some(pos) = s.drafts.iter().position(|m| m.id == draft_id) else {
            return Err(RecoveryReserveErr::NotFound);
        };
        if s.activating_recovery_drafts.contains(draft_id)
            || s.steer_reservations.values().any(|r| r.row_id == draft_id)
        {
            return Err(RecoveryReserveErr::AlreadyActivating(
                "Recovery draft activation is already in progress; draft retained",
            ));
        }
        if Some(op) == s.drafts[pos].cmid.as_deref() {
            return Err(RecoveryReserveErr::InvalidOperationId(
                "Resending a parked recovery draft requires a new client operation id",
            ));
        }
        if Self::prompt_already_accepted_excluding_draft(s, op, Some(pos)) {
            return Err(RecoveryReserveErr::OperationInUse(
                "Activation operation id is already in use; draft retained",
            ));
        }
        s.reserved_recovery_cmids.insert(op.to_owned());
        s.activating_recovery_drafts.insert(draft_id.to_owned());
        Ok(RecoveryActivationReservation {
            hub: Arc::downgrade(&self.inner),
            session_id: session_id.to_owned(),
            draft_id: draft_id.to_owned(),
            op: op.to_owned(),
            active: true,
        })
    }

    /// Preview a reserved recovery-draft activation without mutating `s.queue`
    /// or `s.drafts`, install the per-session `in_flight_pending_commits` fence,
    /// and enqueue an acknowledged `StoreWrite::UpdatePending`.
    pub fn begin_durable_recovery_draft_activation(
        &self,
        mut reservation: RecoveryActivationReservation,
        front: bool,
        caller: &crate::prompt_origin::RequestCaller,
    ) -> Result<RecoveryActivationCommitStep, &'static str> {
        let sid = reservation.session_id.clone();
        let draft_id = reservation.draft_id.clone();
        let op = reservation.op.clone();
        let trigger = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        let (sync_row, sync_had_scheduled, step) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(&sid) else {
                return Err("Session no longer exists; recovery draft was not activated");
            };
            let Some(pos) = s.drafts.iter().position(|m| m.id == draft_id) else {
                return Err("Recovery draft no longer exists");
            };
            let original = &s.drafts[pos];
            let had_scheduled = original.schedule.is_some();
            let new_row = QueuedMessage {
                id: self.next_qid(),
                text: original.text.clone(),
                content: original.content.clone(),
                cmid: Some(op),
                schedule: None,
                origin: original.origin.clone(),
                initiator: Some(trigger),
                explicit: None,
            };
            if !self.has_store_writer() {
                s.drafts.remove(pos);
                s.deferred_scheduled_drafts.retain(|d| d != &draft_id);
                if s.editing.as_deref() == Some(draft_id.as_str()) {
                    s.editing = None;
                    s.editing_token = None;
                }
                let new_qid = new_row.id.clone();
                if front {
                    s.queue.insert(0, new_row);
                    self.settle_explicit_locked(&sid, s);
                } else {
                    s.queue.push(new_row);
                }
                reservation.release_in_session(s);
                (Some(new_qid), had_scheduled, None)
            } else {
                let mut claims = self.inner.notice_claims.lock();
                if claims.in_flight_pending_commits.contains_key(&sid) {
                    return Err("Another pending queue update is in progress; draft retained");
                }
                let mut preview_queue = s.queue.clone();
                if front {
                    preview_queue.insert(0, new_row.clone());
                    // The displaced command ends now, its receipt ahead of the
                    // preview write: a crash before the commit cannot leave it
                    // replayable against a row persisted without its intent.
                    super::explicit_intent::strip_displaced_intents(&mut preview_queue);
                    s.settle_head_command();
                    self.write_receipts_locked(&sid, s);
                } else {
                    preview_queue.push(new_row.clone());
                }
                let mut preview_drafts = s.drafts.clone();
                preview_drafts.remove(pos);
                let Some(rx) = self.send_acked_update_pending_with_retired(
                    &sid,
                    preview_queue,
                    preview_drafts,
                    Vec::new(),
                    Vec::new(),
                    s.pending_retired_dispatches.clone(),
                    false,
                ) else {
                    if front {
                        // The head stays first: its command did not end.
                        s.unsettle_head_command();
                        self.write_receipts_locked(&sid, s);
                    }
                    return Err("Failed to persist recovery draft activation; draft retained");
                };
                claims
                    .in_flight_pending_commits
                    .insert(sid.clone(), Default::default());
                let commit = Box::new(PendingRecoveryActivationCommit {
                    hub: Arc::downgrade(&self.inner),
                    active: true,
                    new_row,
                    front,
                    had_scheduled,
                    reservation,
                });
                (None, false, Some((commit, rx)))
            }
        };
        if let Some((commit, rx)) = step {
            return Ok(RecoveryActivationCommitStep::Pending(commit, rx));
        }
        self.emit_pending(&sid);
        if sync_had_scheduled {
            self.cancel_draft_timer(&sid, &draft_id);
            self.broadcast_session(&sid);
        }
        if let Some(new_qid) = sync_row {
            self.drain_specific_head(&sid, &new_qid, true, Some(caller));
        }
        Ok(RecoveryActivationCommitStep::CommittedSync)
    }

    /// Apply a durably committed recovery-draft activation to Hub, flush any
    /// deferred concurrent pending writes, broadcast, and dispatch ONLY if the
    /// newly committed row is at the queue head under normal drain rules.
    pub fn commit_recovery_draft_activation(
        &self,
        commit: &mut PendingRecoveryActivationCommit,
        caller: &crate::prompt_origin::RequestCaller,
    ) {
        if !commit.active {
            return;
        }
        commit.active = false;
        let sid = commit.reservation.session_id.clone();
        let draft_id = commit.reservation.draft_id.clone();
        let new_qid = commit.new_row.id.clone();
        let snapshot = {
            let mut sessions = self.inner.sessions.lock();
            let mut claims = self.inner.notice_claims.lock();
            if let Some(s) = sessions.get_mut(&sid) {
                s.drafts.retain(|m| m.id != draft_id);
                s.deferred_scheduled_drafts.retain(|d| d != &draft_id);
                if s.editing.as_deref() == Some(draft_id.as_str()) {
                    s.editing = None;
                    s.editing_token = None;
                }
                if commit.front {
                    s.queue.insert(0, commit.new_row.clone());
                    self.settle_explicit_locked(&sid, s);
                } else {
                    s.queue.push(commit.new_row.clone());
                }
                commit.reservation.release_in_session(s);
                let snapshot = (s.queue.clone(), s.drafts.clone());
                self.finish_in_flight_pending_commit(&sid, &snapshot.0, &snapshot.1, &mut claims);
                Some(snapshot)
            } else {
                claims.in_flight_pending_commits.remove(&sid);
                None
            }
        };
        if let Some((q, d)) = snapshot {
            self.broadcast_pending_snapshot(&sid, &q, &d);
            if commit.had_scheduled {
                self.cancel_draft_timer(&sid, &draft_id);
                self.broadcast_session(&sid);
            }
            self.drain_specific_head(&sid, &new_qid, true, Some(caller));
        }
    }

    /// Abort an uncommitted recovery-draft activation, releasing its fences
    /// while leaving the original draft untouched in `s.drafts`.
    pub fn abort_recovery_draft_activation(&self, commit: &mut PendingRecoveryActivationCommit) {
        if !commit.active {
            return;
        }
        commit.active = false;
        let sid = commit.reservation.session_id.clone();
        let mut sessions = self.inner.sessions.lock();
        let mut claims = self.inner.notice_claims.lock();
        if let Some(s) = sessions.get_mut(&sid) {
            commit.reservation.release_in_session(s);
            if commit.front {
                // The held head stays first: its command did not end. Written
                // ahead of the snapshot that restores the queue.
                s.unsettle_head_command();
                self.write_receipts_locked(&sid, s);
            }
            self.finish_in_flight_pending_commit(&sid, &s.queue, &s.drafts, &mut claims);
        } else {
            claims.in_flight_pending_commits.remove(&sid);
        }
    }

    /// Activate one draft: validate operation ownership and move from drafts to
    /// queue/dispatch atomically under one `sessions` lock so a reused operation
    /// ID can never discard a draft.
    pub fn activate_draft(
        &self,
        session_id: &str,
        id: &str,
        cmid: Option<String>,
        caller: &crate::prompt_origin::RequestCaller,
    ) {
        self.activate_draft_with_front(session_id, id, cmid, false, caller);
    }

    /// Activate one draft: remove it from drafts (cancelling any schedule timer)
    /// and submit it to the back (`front = false`) or front (`front = true`) of
    /// the queue in one server-owned transition.
    pub fn activate_draft_with_front(
        &self,
        session_id: &str,
        id: &str,
        cmid: Option<String>,
        front: bool,
        caller: &crate::prompt_origin::RequestCaller,
    ) {
        if self.is_managed_child(session_id)
            || self.reject_reserved_notice_cmid(session_id, cmid.as_deref())
        {
            return;
        }
        if id.starts_with("recovery-") {
            self.activate_recovery_draft_direct(session_id, id, cmid, front, caller);
            return;
        }
        let requested_cmid = cmid.clone();
        let trigger = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        let wired = self.inner.dispatch_tx.lock().is_some();
        let mut dispatch = None;
        let mut had_scheduled = false;
        let mut is_owner_turn = false;
        let mut error_msg: Option<&'static str> = None;
        let activated = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if s.activating_recovery_drafts.contains(id)
                || s.steer_reservations.values().any(|r| r.row_id == id)
            {
                return;
            }
            let Some(pos) = s.drafts.iter().position(|m| m.id == id) else {
                return;
            };
            let effective_cmid = cmid.or_else(|| {
                s.drafts[pos]
                    .cmid
                    .clone()
                    .filter(|cid| !crate::agent_model::is_reserved_call_notice_cmid(cid))
            });
            if let Some(op) = effective_cmid.as_deref()
                && Self::prompt_already_accepted_excluding_draft(s, op, Some(pos))
            {
                error_msg = Some("Activation operation id is already in use; draft retained");
                false
            } else {
                s.deferred_scheduled_drafts.retain(|d| d != id);
                if s.editing.as_deref() == Some(id) {
                    s.editing = None;
                    s.editing_token = None;
                }
                let (scheduled, owner_turn, req) = self.stage_regular_draft_activation(
                    session_id,
                    s,
                    pos,
                    effective_cmid,
                    front,
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
        if is_owner_turn {
            self.notify_human_turn(session_id);
        }
        self.emit_pending(session_id);
        if had_scheduled {
            self.cancel_draft_timer(session_id, id);
            self.broadcast_session(session_id);
        }
        if let Some(req) = dispatch {
            self.send_dispatch(req);
        }
    }

    fn activate_recovery_draft_direct(
        &self,
        session_id: &str,
        id: &str,
        cmid: Option<String>,
        front: bool,
        caller: &crate::prompt_origin::RequestCaller,
    ) {
        let Some(op) = cmid.as_deref().filter(|op| !op.is_empty()) else {
            let msg = "Resending a parked recovery draft requires a new client operation id";
            self.broadcast_error(Some(session_id.to_owned()), msg.to_owned());
            return;
        };
        let reservation = match self.reserve_recovery_draft_activation(session_id, id, op) {
            Ok(r) => r,
            Err(RecoveryReserveErr::NotFound) => return,
            Err(
                RecoveryReserveErr::InvalidOperationId(msg)
                | RecoveryReserveErr::OperationInUse(msg)
                | RecoveryReserveErr::AlreadyActivating(msg),
            ) => {
                self.command_result(session_id, op, CommandOutcome::Rejected, msg);
                self.broadcast_error(Some(session_id.to_owned()), msg.to_owned());
                return;
            }
        };
        if self.has_store_writer() && tokio::runtime::Handle::try_current().is_err() {
            let msg = "Failed to persist recovery draft activation; draft retained";
            self.command_result(session_id, op, CommandOutcome::Rejected, msg);
            self.broadcast_error(Some(session_id.to_owned()), msg.to_owned());
            return;
        }
        match self.begin_durable_recovery_draft_activation(reservation, front, caller) {
            Ok(RecoveryActivationCommitStep::CommittedSync) => {}
            Ok(RecoveryActivationCommitStep::Pending(mut commit, rx)) => {
                let hub = self.clone();
                let sid = session_id.to_owned();
                let caller = caller.clone();
                self.spawn_teardown_requeue(async move {
                    match rx.await {
                        Ok(Ok(true)) => hub.commit_recovery_draft_activation(&mut commit, &caller),
                        _ => {
                            let op_id = commit.op().to_owned();
                            hub.abort_recovery_draft_activation(&mut commit);
                            let msg = "Failed to persist recovery draft activation; draft retained";
                            hub.command_result(&sid, &op_id, CommandOutcome::Rejected, msg);
                            hub.broadcast_error(Some(sid), msg.to_owned());
                        }
                    }
                });
            }
            Err(msg) => {
                self.command_result(session_id, op, CommandOutcome::Rejected, msg);
                self.broadcast_error(Some(session_id.to_owned()), msg.to_owned());
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stage_regular_draft_activation(
        &self,
        session_id: &str,
        s: &mut Session,
        pos: usize,
        effective_cmid: Option<String>,
        front: bool,
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
            let row = QueuedMessage {
                id: self.next_qid(),
                text: m.text,
                content: m.content,
                cmid: effective_cmid,
                schedule: None,
                origin: m.origin,
                initiator: Some(trigger),
                explicit: None,
            };
            if front {
                s.queue.insert(0, row);
                self.settle_explicit_locked(session_id, s);
            } else {
                s.queue.push(row);
            }
            None
        };
        (had_scheduled, is_owner_turn, dispatch)
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
            let mut kept = Vec::new();
            let mut activated = Vec::new();
            for m in std::mem::take(&mut s.drafts) {
                if m.id.starts_with("recovery-")
                    || s.activating_recovery_drafts.contains(&m.id)
                    || s.steer_reservations.values().any(|r| r.row_id == m.id)
                {
                    kept.push(m);
                } else {
                    activated.push(m);
                }
            }
            if s.editing
                .as_deref()
                .is_some_and(|id| activated.iter().any(|m| m.id == id))
            {
                s.editing = None;
                s.editing_token = None;
            }
            let res = &s.steer_reservations;
            let reserved = &s.activating_recovery_drafts;
            s.deferred_scheduled_drafts.retain(|d| {
                d.starts_with("recovery-")
                    || reserved.contains(d)
                    || res.values().any(|r| r.row_id == *d)
            });
            s.drafts = kept;
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
            self.broadcast_session(session_id);
        }
    }
}
