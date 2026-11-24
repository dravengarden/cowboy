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

    pub(in crate::core) fn persist_recovery_cmid_if_needed(
        &self,
        session_id: &str,
        cmid: Option<&str>,
    ) {
        let Some(cmid) = cmid.filter(|c| !c.is_empty()) else {
            return;
        };
        let updated = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.committed_recovery_cmids.insert(cmid.to_owned());
            if s.recovery_cmids.insert(cmid.to_owned()) {
                let mut list: Vec<String> = s.recovery_cmids.iter().cloned().collect();
                list.sort();
                Some(list)
            } else {
                None
            }
        };
        if let Some(list) = updated {
            let key = format!(
                "{}{session_id}",
                settings_keys::SESSION_RECOVERY_CMIDS_PREFIX
            );
            if self.has_store_writer() {
                // Finding 2: Never issue a separate best-effort PutSetting when a
                // store writer is attached; `UpdatePending` commits `recovery_cmids`
                // atomically in the same SQL transaction as `queue`/`drafts`.
                self.load_settings(vec![(key, serde_json::json!(list))]);
            } else {
                self.set_setting(key, serde_json::json!(list));
            }
        }
    }

    pub(in crate::core) fn add_recovery_draft(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) {
        let saved_cmid = cmid.clone();
        let duplicate;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            duplicate = cmid
                .as_deref()
                .filter(|c| {
                    s.committed_recovery_cmids.contains(*c)
                        || s.drafts.iter().any(|m| m.cmid.as_deref() == Some(*c))
                })
                .map(str::to_owned);
            if duplicate.is_none() {
                let id = format!("recovery-{}", self.next_qid());
                s.drafts.push(QueuedMessage {
                    id,
                    text,
                    content,
                    cmid,
                    schedule: None,
                    origin: None,
                    initiator: None,
                    explicit: false,
                });
            }
        }
        self.persist_recovery_cmid_if_needed(session_id, saved_cmid.as_deref());
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
                    explicit: false,
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
                        explicit: false,
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
            // lock that removes the Draft, so a concurrent Trash or conversion
            // fence cannot slip in.
            if s.meta.trashed_at_ms.is_some() || self.is_row_withdrawing(session_id, draft_id) {
                return;
            }
            if draft_id.starts_with("recovery-") {
                let Some(pos) = s.drafts.iter().position(|m| m.id == draft_id) else {
                    return;
                };
                let front = s.drafts[pos]
                    .schedule
                    .as_ref()
                    .is_some_and(|sc| sc.delivery == Delivery::Front);
                drop(sessions);
                let cmid = Some(format!("{SCHED_PREFIX}{session_id}-{draft_id}"));
                self.activate_recovery_draft_durable_opts(
                    session_id,
                    draft_id,
                    cmid,
                    crate::prompt_origin::DeliveryOrigin::cowboy("schedule"),
                    front,
                    false,
                );
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
    async fn recovery_journal_survives_stalled_shutdown_and_reconciles_on_restart() {
        let dir = tempfile::tempdir().unwrap();
        let journal_path = dir.path().join("recovery-parks.journal.json");

        // Controller #1: store writer NEVER acks (simulated Postgres hang).
        let health1 = Arc::new(PersistenceHealth::default());
        let (sink1, _hung_rx1) = StoreSink::channel(16, health1);
        let hub1 = Hub::with_store(Some(sink1));
        seed_session(&hub1, "s-f4", Status::Running);
        hub1.attach_and_reconcile_recovery_journal(journal_path.clone());

        hub1.enqueue_durable_recovery_park(
            "s-f4".into(),
            "unresolved payload".into(),
            vec![serde_json::json!({"type": "text", "text": "unresolved payload"})],
            Some("cmid-f4".into()),
            "disconnect during outage".into(),
            None,
        );
        assert!(journal_path.exists());

        // Graceful shutdown must complete within bounded grace despite hung store ack.
        let started = std::time::Instant::now();
        hub1.drain_teardown_requeues(std::time::Duration::from_millis(50))
            .await;
        assert!(
            started.elapsed() < std::time::Duration::from_secs(4),
            "shutdown must not hang on stalled recovery park ack"
        );
        assert!(journal_path.exists());

        // Controller #2: healthy store writer reconciles journal on startup and prunes file.
        let health2 = Arc::new(PersistenceHealth::default());
        let (sink2, mut store_rx2) = StoreSink::channel(16, health2);
        let hub2 = Hub::with_store(Some(sink2));
        let acks2 = hub2.store_ack_sink();
        seed_session(&hub2, "s-f4", Status::Interrupted);
        let ack_task = tokio::spawn(async move {
            while let Some(write) = store_rx2.recv().await {
                if let StoreWrite::UpdatePending {
                    ack_id: Some(id), ..
                } = write
                {
                    acks2.complete(id, Ok(true));
                }
            }
        });
        hub2.attach_and_reconcile_recovery_journal(journal_path.clone());
        for _ in 0..50 {
            if !pending_of(&hub2, "s-f4").1.is_empty() && !journal_path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let (_, drafts) = pending_of(&hub2, "s-f4");
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].cmid.as_deref(), Some("cmid-f4"));
        assert!(!journal_path.exists(), "journal should prune after commit");
        ack_task.abort();
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
}
