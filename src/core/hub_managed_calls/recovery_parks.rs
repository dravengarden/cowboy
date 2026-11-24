//! Unified Controller-owned durable recovery-draft parking and activation.
//!
//! Serializes late `CommandAck` / `TurnEnded` / verified `UndeliveredPrompt`
//! settlements atomically with 2-phase provisional->final SQL parking on `Hub`
//! (Findings 2, 3, 4, 5, 6, 7).

use super::types::{
    DeferredPendingFlush, PendingRecoveryDraftRecord, RecoveryParkSettlement,
    VerifiedUndeliveredPayload,
};
use super::*;

type ParkRetiredHook = Arc<dyn Fn() + Send + Sync>;

pub(in crate::core) const PROVISIONAL_RECOVERY_PREFIX: &str = "provisional-recovery-";

enum FinalizeStep {
    NeedsCancel(Vec<QueuedMessage>, Vec<QueuedMessage>),
    PromoteToFinal(Vec<QueuedMessage>, Vec<QueuedMessage>, String),
    AlreadyGone,
}

impl Hub {
    /// Register a Controller-owned durable recovery park record on `Hub` so it
    /// survives runtime disconnect/replacement, serializes late ACK/`TurnEnded`
    /// and verified `UndeliveredPrompt` evidence, and retries across storage outages.
    #[allow(clippy::too_many_arguments)]
    pub fn register_pending_recovery_park(
        &self,
        session_id: &str,
        command_id: &str,
        cmid: Option<String>,
        text: String,
        content: Vec<serde_json::Value>,
        reason: String,
        cancellable: bool,
        on_retired: Option<Arc<dyn Fn() + Send + Sync>>,
        on_requeue: Option<Arc<dyn Fn(Option<String>) + Send + Sync>>,
    ) -> bool {
        let key = (session_id.to_owned(), command_id.to_owned());
        let mut claims = self.inner.notice_claims.lock();
        if claims.recovery_parks.contains_key(&key) {
            return false;
        }
        claims.recovery_parks.insert(
            key,
            PendingRecoveryDraftRecord {
                cmid,
                text,
                content,
                reason,
                cancellable,
                finalizing: !cancellable,
                cancelling: false,
                provisional_draft_id: None,
                in_progress: true,
                settlement: None,
                verified_undelivered: None,
                on_retired,
                on_requeue,
            },
        );
        true
    }

    /// Record a late positive `CommandAck`, `TurnEnded`, or non-ambiguous
    /// `unhanded:` rejection against an active `Hub` recovery park record.
    /// Returns `true` if a live park owns `(session_id, command_id)` (even if
    /// already finalizing, so the caller lets the park task finish cleanup).
    pub fn record_recovery_park_settlement(
        &self,
        session_id: &str,
        command_id: &str,
        settlement: RecoveryParkSettlement,
    ) -> bool {
        let mut claims = self.inner.notice_claims.lock();
        let Some(rec) = claims
            .recovery_parks
            .get_mut(&(session_id.to_owned(), command_id.to_owned()))
        else {
            return false;
        };
        if !rec.finalizing && rec.settlement.is_none() {
            rec.settlement = Some(settlement);
        }
        true
    }

    /// Retain verified `UndeliveredPrompt` evidence on an active `Hub` recovery
    /// park so a subsequent positive `CommandAck` cancelling the park requeues
    /// exactly one copy instead of dropping the prompt (Finding 3).
    pub fn attach_verified_undelivered_to_park(
        &self,
        session_id: &str,
        command_id: &str,
        cmid: Option<&str>,
        payload: VerifiedUndeliveredPayload,
    ) -> bool {
        let mut claims = self.inner.notice_claims.lock();
        if let Some(rec) = claims
            .recovery_parks
            .get_mut(&(session_id.to_owned(), command_id.to_owned()))
        {
            if rec.verified_undelivered.is_none() {
                rec.verified_undelivered = Some(payload);
            }
            return true;
        }
        if let Some(cid) = cmid
            && let Some(rec) = claims
                .recovery_parks
                .iter_mut()
                .find(|((sid, _), r)| sid == session_id && r.cmid.as_deref() == Some(cid))
                .map(|(_, r)| r)
        {
            if rec.verified_undelivered.is_none() {
                rec.verified_undelivered = Some(payload);
            }
            return true;
        }
        false
    }

    pub(in crate::core) fn has_pending_recovery_park_cmid(
        &self,
        session_id: &str,
        cmid: &str,
    ) -> bool {
        self.inner
            .notice_claims
            .lock()
            .recovery_parks
            .iter()
            .any(|((sid, _), rec)| sid == session_id && rec.cmid.as_deref() == Some(cmid))
    }

    pub(in crate::core) fn has_any_retryable_recovery_park(&self, session_id: &str) -> bool {
        self.inner
            .notice_claims
            .lock()
            .recovery_parks
            .iter()
            .any(|((sid, _), rec)| sid == session_id && !rec.in_progress)
    }

    pub fn spawn_recovery_park_driver(&self, session_id: String, command_id: String) {
        let hub = self.clone();
        self.spawn_teardown_requeue(async move {
            hub.drive_recovery_park(session_id, command_id, true).await;
        });
    }

    /// Retry any paused/deferred Controller-owned recovery draft parks for
    /// `session_id` (or all sessions if `None`) on reconnect, snapshot, submit,
    /// storage recovery, or graceful shutdown (Findings 3 & 5).
    pub fn retry_pending_recovery_parks(&self, session_id: Option<&str>) {
        if !self.has_store_writer() || tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let keys: Vec<(String, String)> = {
            let mut claims = self.inner.notice_claims.lock();
            claims
                .recovery_parks
                .iter_mut()
                .filter_map(|((sid, cid), rec)| {
                    if rec.in_progress || session_id.is_some_and(|want| want != sid) {
                        return None;
                    }
                    rec.in_progress = true;
                    Some((sid.clone(), cid.clone()))
                })
                .collect()
        };
        for (sid, cid) in keys {
            let hub = self.clone();
            self.spawn_teardown_requeue(async move {
                hub.drive_recovery_park(sid, cid, false).await;
            });
        }
    }

    async fn drive_recovery_park(
        &self,
        session_id: String,
        command_id: String,
        with_backoff: bool,
    ) {
        let _commit_guard = self.pending_commit_lock(&session_id).await;
        let delays: &[u64] = if with_backoff {
            &[0, 25, 75, 150]
        } else {
            &[0]
        };
        for (attempt, delay_ms) in delays.iter().copied().enumerate() {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            if self.try_settle_before_sql(&session_id, &command_id).await {
                return;
            }
            let next_action = {
                let claims = self.inner.notice_claims.lock();
                claims
                    .recovery_parks
                    .get(&(session_id.clone(), command_id.clone()))
                    .map(|rec| {
                        (
                            rec.cancelling,
                            rec.cancellable && !rec.finalizing,
                            rec.text.clone(),
                            rec.content.clone(),
                            rec.cmid.clone(),
                        )
                    })
            };
            let Some((cancelling, is_provisional_phase, text, content, cmid)) = next_action else {
                return;
            };
            if cancelling {
                if self.try_acknowledged_cancel(&session_id, &command_id).await {
                    return;
                }
                continue;
            }
            if is_provisional_phase {
                if self
                    .run_provisional_then_finalize(&session_id, &command_id, text, content, cmid)
                    .await
                {
                    return;
                }
            } else if self
                .run_final_park_write(&session_id, &command_id, text, content, cmid)
                .await
            {
                return;
            }
        }
        if self.try_settle_before_sql(&session_id, &command_id).await {
            return;
        }
        let reason = {
            let mut claims = self.inner.notice_claims.lock();
            if let Some(rec) = claims
                .recovery_parks
                .get_mut(&(session_id.clone(), command_id.clone()))
            {
                rec.in_progress = false;
                Some(rec.reason.clone())
            } else {
                None
            }
        };
        if with_backoff && let Some(reason) = reason {
            self.broadcast_error(
                Some(session_id),
                format!("{reason} (durable draft save deferred; will retry when storage recovers)"),
            );
        }
    }

    async fn try_settle_before_sql(&self, session_id: &str, command_id: &str) -> bool {
        let key = (session_id.to_owned(), command_id.to_owned());
        let settled = {
            let mut claims = self.inner.notice_claims.lock();
            let Some(rec) = claims.recovery_parks.get(&key) else {
                return true;
            };
            if rec.finalizing || rec.cancelling || rec.provisional_draft_id.is_some() {
                return false;
            }
            if rec.settlement.is_some() {
                claims.recovery_parks.remove(&key)
            } else {
                None
            }
        };
        if let Some(rec) = settled {
            self.finish_cancelled_park(session_id, rec);
            return true;
        }
        false
    }

    async fn run_provisional_then_finalize(
        &self,
        session_id: &str,
        command_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) -> bool {
        let draft_id = format!("{PROVISIONAL_RECOVERY_PREFIX}{}", self.next_qid());
        let Some((mut preview, rx)) = self.stage_park_write(
            session_id,
            draft_id.clone(),
            text.clone(),
            content.clone(),
            cmid.clone(),
        ) else {
            return false;
        };
        if !matches!(rx.await, Ok(Ok(true))) {
            self.abort_park_recovery_notices(&mut preview);
            return false;
        }
        let step = self.decide_after_provisional(session_id, command_id, &mut preview, &draft_id);
        match step {
            FinalizeStep::AlreadyGone => true,
            FinalizeStep::NeedsCancel(queue, drafts) => {
                self.complete_acknowledged_cancel(session_id, command_id, queue, drafts)
                    .await
            }
            FinalizeStep::PromoteToFinal(queue, mut drafts, final_id) => {
                let final_draft = QueuedMessage {
                    id: final_id,
                    text,
                    content,
                    cmid,
                    schedule: None,
                    origin: None,
                    initiator: None,
                };
                drafts.push(final_draft.clone());
                let Some(rx2) = self.send_acked_update_pending(
                    session_id,
                    queue,
                    drafts,
                    Vec::new(),
                    Vec::new(),
                    false,
                ) else {
                    return false;
                };
                if !matches!(rx2.await, Ok(Ok(true))) {
                    self.abort_in_flight_pending_commit(session_id);
                    return false;
                }
                let (cb, reason) = self.commit_finalized_park(session_id, command_id, final_draft);
                if let Some(cb) = cb {
                    cb();
                }
                if let Some(reason) = reason {
                    self.broadcast_error(Some(session_id.to_owned()), reason);
                }
                true
            }
        }
    }

    fn decide_after_provisional(
        &self,
        session_id: &str,
        command_id: &str,
        preview: &mut PendingRecoveryPark,
        provisional_id: &str,
    ) -> FinalizeStep {
        preview.active = false;
        let key = (session_id.to_owned(), command_id.to_owned());
        let sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get(session_id) else {
            let mut claims = self.inner.notice_claims.lock();
            claims.in_flight_pending_commits.remove(session_id);
            claims.recovery_parks.remove(&key);
            return FinalizeStep::AlreadyGone;
        };
        let mut claims = self.inner.notice_claims.lock();
        let Some(rec) = claims.recovery_parks.get_mut(&key) else {
            self.finish_in_flight_pending_commit(session_id, &s.queue, &s.drafts, &mut claims);
            return FinalizeStep::AlreadyGone;
        };
        if rec.settlement.is_some() {
            rec.cancelling = true;
            rec.provisional_draft_id = Some(provisional_id.to_owned());
            return FinalizeStep::NeedsCancel(s.queue.clone(), s.drafts.clone());
        }
        // Atomically lock settlement closed under the same sessions+claims lock (Finding 2).
        rec.finalizing = true;
        rec.provisional_draft_id = Some(provisional_id.to_owned());
        let final_id = format!("recovery-{}", self.next_qid());
        FinalizeStep::PromoteToFinal(s.queue.clone(), s.drafts.clone(), final_id)
    }

    async fn run_final_park_write(
        &self,
        session_id: &str,
        command_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) -> bool {
        let final_id = format!("recovery-{}", self.next_qid());
        match self.preview_park_recovery_prompt_with_id(session_id, final_id, text, content, cmid) {
            RecoveryParkPreview::Duplicate => {
                let (cb, reason) = self.retire_park_as_duplicate(session_id, command_id);
                if let Some(cb) = cb {
                    cb();
                }
                if let Some(reason) = reason {
                    self.broadcast_error(Some(session_id.to_owned()), reason);
                }
                true
            }
            RecoveryParkPreview::Pending(mut preview, rx) => {
                if !matches!(rx.await, Ok(Ok(true))) {
                    self.abort_park_recovery_notices(&mut preview);
                    return false;
                }
                preview.active = false;
                let draft = preview.added_drafts.remove(0);
                let (cb, reason) = self.commit_finalized_park(session_id, command_id, draft);
                if let Some(cb) = cb {
                    cb();
                }
                if let Some(reason) = reason {
                    self.broadcast_error(Some(session_id.to_owned()), reason);
                }
                true
            }
            RecoveryParkPreview::Rejected => false,
        }
    }

    async fn try_acknowledged_cancel(&self, session_id: &str, command_id: &str) -> bool {
        let snapshot = {
            let sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get(session_id) else {
                let key = (session_id.to_owned(), command_id.to_owned());
                self.inner.notice_claims.lock().recovery_parks.remove(&key);
                return true;
            };
            self.inner
                .notice_claims
                .lock()
                .in_flight_pending_commits
                .entry(session_id.to_owned())
                .or_default();
            (s.queue.clone(), s.drafts.clone())
        };
        self.complete_acknowledged_cancel(session_id, command_id, snapshot.0, snapshot.1)
            .await
    }

    async fn complete_acknowledged_cancel(
        &self,
        session_id: &str,
        command_id: &str,
        queue: Vec<QueuedMessage>,
        drafts: Vec<QueuedMessage>,
    ) -> bool {
        let Some(rx) = self.send_acked_update_pending(
            session_id,
            queue,
            drafts,
            Vec::new(),
            Vec::new(),
            false,
        ) else {
            self.abort_in_flight_pending_commit(session_id);
            return false;
        };
        if !matches!(rx.await, Ok(Ok(true))) {
            self.abort_in_flight_pending_commit(session_id);
            return false;
        }
        let rec = {
            let sessions = self.inner.sessions.lock();
            let mut claims = self.inner.notice_claims.lock();
            if let Some(s) = sessions.get(session_id) {
                self.finish_in_flight_pending_commit(session_id, &s.queue, &s.drafts, &mut claims);
            } else {
                claims.in_flight_pending_commits.remove(session_id);
            }
            claims
                .recovery_parks
                .remove(&(session_id.to_owned(), command_id.to_owned()))
        };
        if let Some(rec) = rec {
            self.finish_cancelled_park(session_id, rec);
        }
        true
    }

    fn finish_cancelled_park(&self, session_id: &str, rec: PendingRecoveryDraftRecord) {
        if let Some(cb) = rec.on_retired {
            cb();
        }
        match rec.settlement {
            Some(RecoveryParkSettlement::Unhanded(reason)) => {
                if let Some(requeue) = rec.on_requeue {
                    requeue(reason);
                }
            }
            Some(RecoveryParkSettlement::Admitted) => {
                // Finding 3: if worker admitted then reported verified UndeliveredPrompt
                // while parked, requeue that exact verified copy now instead of losing it.
                if let Some(u) = rec.verified_undelivered {
                    self.requeue_prompt(
                        session_id,
                        u.text,
                        u.content,
                        u.cmid,
                        u.origin,
                        u.initiator,
                    );
                    self.broadcast_error(Some(session_id.to_owned()), u.error);
                }
            }
            None => {}
        }
    }

    fn stage_park_write(
        &self,
        session_id: &str,
        draft_id: String,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) -> Option<(PendingRecoveryPark, StoreAckRx)> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        let queue = s.queue.clone();
        let mut drafts = s.drafts.clone();
        let draft = QueuedMessage {
            id: draft_id,
            text,
            content,
            cmid,
            schedule: None,
            origin: None,
            initiator: None,
        };
        drafts.push(draft.clone());
        let rx = self.send_acked_update_pending(
            session_id,
            queue,
            drafts,
            Vec::new(),
            Vec::new(),
            false,
        )?;
        self.inner
            .notice_claims
            .lock()
            .in_flight_pending_commits
            .insert(session_id.to_owned(), DeferredPendingFlush::default());
        Some((
            PendingRecoveryPark {
                hub: Arc::downgrade(&self.inner),
                active: true,
                session_id: session_id.to_owned(),
                added_drafts: vec![draft],
                settled_notices: Vec::new(),
            },
            rx,
        ))
    }

    fn abort_in_flight_pending_commit(&self, session_id: &str) {
        let sessions = self.inner.sessions.lock();
        let mut claims = self.inner.notice_claims.lock();
        if let Some(s) = sessions.get(session_id) {
            self.finish_in_flight_pending_commit(session_id, &s.queue, &s.drafts, &mut claims);
        } else {
            claims.in_flight_pending_commits.remove(session_id);
        }
    }

    fn retire_park_as_duplicate(
        &self,
        session_id: &str,
        command_id: &str,
    ) -> (Option<ParkRetiredHook>, Option<String>) {
        self.inner
            .notice_claims
            .lock()
            .recovery_parks
            .remove(&(session_id.to_owned(), command_id.to_owned()))
            .map_or((None, None), |rec| (rec.on_retired, Some(rec.reason)))
    }

    fn commit_finalized_park(
        &self,
        session_id: &str,
        command_id: &str,
        draft: QueuedMessage,
    ) -> (Option<ParkRetiredHook>, Option<String>) {
        let (queue, drafts, cb, reason) = {
            let mut sessions = self.inner.sessions.lock();
            let mut claims = self.inner.notice_claims.lock();
            let removed = claims
                .recovery_parks
                .remove(&(session_id.to_owned(), command_id.to_owned()));
            let (cb, reason) = removed.map_or((None, None), |r| (r.on_retired, Some(r.reason)));
            let Some(s) = sessions.get_mut(session_id) else {
                claims.in_flight_pending_commits.remove(session_id);
                return (cb, reason);
            };
            if let Some(cid) = draft.cmid.as_deref() {
                hub_dispatch_origin::remove_pending_echo(s, cid);
                Self::record_retired_undelivered_cmid(&mut claims, session_id, cid);
            }
            if !draft
                .cmid
                .as_deref()
                .is_some_and(|c| s.drafts.iter().any(|m| m.cmid.as_deref() == Some(c)))
            {
                s.drafts.push(draft);
            }
            let queue = s.queue.clone();
            let drafts = s.drafts.clone();
            self.finish_in_flight_pending_commit(session_id, &queue, &drafts, &mut claims);
            (queue, drafts, cb, reason)
        };
        self.broadcast_pending_snapshot(session_id, &queue, &drafts);
        (cb, reason)
    }

    /// Durable recovery-draft activation: commits removing the `recovery-*` draft
    /// AND adding the queued prompt under the fresh operation ID in ONE
    /// acknowledged SQL `UpdatePending` transaction BEFORE allowing `drain_head`
    /// to take it (Finding 7).
    pub(in crate::core) fn activate_recovery_draft_durable(
        &self,
        session_id: &str,
        draft_id: &str,
        fresh_cmid: String,
        caller: crate::prompt_origin::RequestCaller,
    ) {
        let fence_key = (session_id.to_owned(), draft_id.to_owned());
        {
            let mut claims = self.inner.notice_claims.lock();
            if !claims.activating_drafts.insert(fence_key.clone()) {
                return;
            }
        }
        let hub = self.clone();
        let sid = session_id.to_owned();
        let did = draft_id.to_owned();
        self.spawn_teardown_requeue(async move {
            let _commit_guard = hub.pending_commit_lock(&sid).await;
            let trigger = crate::prompt_origin::DeliveryOrigin::from_caller(&caller);
            let staged = {
                let sessions = hub.inner.sessions.lock();
                let Some(s) = sessions.get(&sid) else {
                    hub.inner
                        .notice_claims
                        .lock()
                        .activating_drafts
                        .remove(&fence_key);
                    return;
                };
                let Some(pos) = s.drafts.iter().position(|m| m.id == did) else {
                    hub.inner
                        .notice_claims
                        .lock()
                        .activating_drafts
                        .remove(&fence_key);
                    return;
                };
                let target = &s.drafts[pos];
                if fresh_cmid.is_empty() || Some(fresh_cmid.as_str()) == target.cmid.as_deref() {
                    drop(sessions);
                    hub.inner
                        .notice_claims
                        .lock()
                        .activating_drafts
                        .remove(&fence_key);
                    let msg = "Resending a parked recovery draft requires a new operation id";
                    hub.command_result(&sid, &fresh_cmid, CommandOutcome::Rejected, msg);
                    hub.broadcast_error(Some(sid), msg.to_owned());
                    return;
                }
                if Self::prompt_already_accepted_excluding_draft(s, &fresh_cmid, Some(pos))
                    || hub.has_pending_recovery_park_cmid(&sid, &fresh_cmid)
                {
                    drop(sessions);
                    hub.inner
                        .notice_claims
                        .lock()
                        .activating_drafts
                        .remove(&fence_key);
                    let msg = "Activation operation id is already in use; draft retained";
                    hub.command_result(&sid, &fresh_cmid, CommandOutcome::Rejected, msg);
                    hub.broadcast_error(Some(sid), msg.to_owned());
                    return;
                }
                let mut drafts = s.drafts.clone();
                let m = drafts.remove(pos);
                let mut queue = s.queue.clone();
                let queued = QueuedMessage {
                    id: hub.next_qid(),
                    text: m.text,
                    content: m.content,
                    cmid: Some(fresh_cmid.clone()),
                    schedule: None,
                    origin: m.origin.clone(),
                    initiator: Some(trigger.clone()),
                };
                let is_owner = crate::prompt_origin::compute_dispatch_origin(
                    m.origin.as_ref(),
                    Some(&trigger),
                )
                .is_owner();
                queue.push(queued.clone());
                let Some(rx) = hub.send_acked_update_pending(
                    &sid,
                    queue,
                    drafts,
                    Vec::new(),
                    Vec::new(),
                    false,
                ) else {
                    drop(sessions);
                    hub.inner
                        .notice_claims
                        .lock()
                        .activating_drafts
                        .remove(&fence_key);
                    let msg = "Durable storage unavailable; recovery draft retained";
                    hub.command_result(&sid, &fresh_cmid, CommandOutcome::Rejected, msg);
                    hub.broadcast_error(Some(sid), msg.to_owned());
                    return;
                };
                hub.inner
                    .notice_claims
                    .lock()
                    .in_flight_pending_commits
                    .insert(sid.clone(), DeferredPendingFlush::default());
                (rx, queued, m.schedule.is_some(), is_owner)
            };
            let (rx, queued, had_scheduled, is_owner) = staged;
            if !matches!(rx.await, Ok(Ok(true))) {
                hub.abort_in_flight_pending_commit(&sid);
                hub.inner
                    .notice_claims
                    .lock()
                    .activating_drafts
                    .remove(&fence_key);
                let msg = "Failed to durably queue recovery draft; draft retained";
                hub.command_result(&sid, &fresh_cmid, CommandOutcome::Rejected, msg);
                hub.broadcast_error(Some(sid), msg.to_owned());
                return;
            }
            let snapshot = {
                let mut sessions = hub.inner.sessions.lock();
                let mut claims = hub.inner.notice_claims.lock();
                claims.activating_drafts.remove(&fence_key);
                let Some(s) = sessions.get_mut(&sid) else {
                    claims.in_flight_pending_commits.remove(&sid);
                    return;
                };
                if let Some(pos) = s.drafts.iter().position(|m| m.id == did) {
                    s.drafts.remove(pos);
                    s.queue.push(queued);
                }
                let queue = s.queue.clone();
                let drafts = s.drafts.clone();
                hub.finish_in_flight_pending_commit(&sid, &queue, &drafts, &mut claims);
                (queue, drafts)
            };
            if is_owner {
                hub.notify_human_turn(&sid);
            }
            hub.broadcast_pending_snapshot(&sid, &snapshot.0, &snapshot.1);
            if had_scheduled {
                hub.cancel_draft_timer(&sid, &did);
                hub.broadcast_sessions();
            }
            hub.drain_head(&sid, true, false, Some(&caller));
        });
    }
}
