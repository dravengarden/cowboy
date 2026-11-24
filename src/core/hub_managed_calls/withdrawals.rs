//! Owner queue withdrawals, recovery parking, and queue inspection for
//! durable managed-call completion notices.

use super::types::{
    DeferredDrainActions, DeferredPendingFlush, affected_row_ids, apply_withdrawal_to_lists,
};
use super::*;

const SHUTDOWN_AWAIT_GRACE: std::time::Duration = std::time::Duration::from_secs(1);

type StagedRecoveryActivation = (StoreAckRx, QueuedMessage, Option<(i64, u64)>, bool);

impl Hub {
    pub(in crate::core) fn has_in_flight_pending_commit(&self, session_id: &str) -> bool {
        self.inner
            .notice_claims
            .lock()
            .in_flight_pending_commits
            .contains_key(session_id)
    }

    /// Await `fut` normally until shutdown begins; once graceful shutdown is
    /// signaled, bound the remaining wait to `SHUTDOWN_AWAIT_GRACE` so a
    /// stalled commit lock or hung store writer can never deadlock shutdown.
    pub async fn await_with_shutdown<F>(&self, fut: F) -> Option<F::Output>
    where
        F: std::future::Future,
    {
        tokio::pin!(fut);
        if !self.inner.teardown_shutdown.load(Ordering::Acquire) {
            let notified = self.inner.teardown_shutdown_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.inner.teardown_shutdown.load(Ordering::Acquire) {
                tokio::select! {
                    out = &mut fut => return Some(out),
                    () = &mut notified => {}
                }
            }
        }
        tokio::time::timeout(SHUTDOWN_AWAIT_GRACE, fut).await.ok()
    }

    pub(in crate::core) fn sorted_recovery_cmids(s: &Session) -> Option<Vec<String>> {
        if s.recovery_cmids.is_empty() {
            return None;
        }
        let mut list: Vec<String> = s.recovery_cmids.iter().cloned().collect();
        list.sort();
        Some(list)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::core) fn send_acked_update_pending_with_recovery(
        &self,
        session_id: &str,
        queue: Vec<QueuedMessage>,
        drafts: Vec<QueuedMessage>,
        settled_notices: Vec<(String, String)>,
        released_claims: Vec<NoticeClaimRelease>,
        recovery_cmids: Option<Vec<String>>,
        require_claim_release: bool,
    ) -> Option<StoreAckRx> {
        if self.is_pending_writes_poisoned(session_id) {
            return None;
        }
        let tx = self.inner.store_tx.as_ref()?;
        let (ack_id, rx) = self.inner.store_acks.allocate();
        let sent = tx.send(StoreWrite::UpdatePending {
            session_id: session_id.to_owned(),
            queue,
            drafts,
            settled_notices,
            released_claims,
            recovery_cmids,
            require_claim_release,
            ack_id: Some(ack_id),
        });
        if !sent {
            self.inner.store_acks.cancel(ack_id);
            return None;
        }
        Some(rx)
    }

    /// Preview parking one or more unacknowledged `call-notice:` prompts as
    /// plain drafts (`cmid: None`) with `execution_recovery_parked`, fence the
    /// session's pending snapshots, and enqueue the atomic FIFO
    /// `StoreWrite::UpdatePending` UNDER `sessions.lock()`.
    pub fn preview_park_recovery_notices(
        &self,
        session_id: &str,
        prompts: &[(String, Vec<serde_json::Value>, String)],
    ) -> Option<(PendingRecoveryPark, StoreAckRx)> {
        self.preview_park_recovery_batch(session_id, prompts, &[])
    }

    /// Stage BOTH pending completion notices AND ordinary prompts for explicit
    /// execution recovery in ONE atomic acknowledged `UpdatePending` transaction
    /// alongside durable `session_recovery_cmids` tombstones (Findings 2 & 9).
    pub fn preview_park_recovery_batch(
        &self,
        session_id: &str,
        notice_prompts: &[(String, Vec<serde_json::Value>, String)],
        other_prompts: &[(String, Vec<serde_json::Value>, Option<String>)],
    ) -> Option<(PendingRecoveryPark, StoreAckRx)> {
        if notice_prompts.is_empty() && other_prompts.is_empty() {
            return None;
        }
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        let mut queue = s.queue.clone();
        let mut drafts = s.drafts.clone();
        let mut added_drafts = Vec::with_capacity(notice_prompts.len() + other_prompts.len());
        let mut settled_notices = Vec::with_capacity(notice_prompts.len());
        let mut staged_cmids = s.recovery_cmids.clone();
        let mut committed_recovery_cmids = Vec::new();
        for (text, content, cid) in notice_prompts {
            let draft = QueuedMessage {
                id: self.next_qid(),
                text: text.clone(),
                content: content.clone(),
                cmid: None,
                schedule: None,
                origin: Some(crate::prompt_origin::DeliveryOrigin::cowboy("managed_call")),
                initiator: None,
                explicit: false,
                wake_dormant: false,
            };
            drafts.push(draft.clone());
            added_drafts.push(draft);
            settled_notices.push((cid.clone(), "execution_recovery_parked".to_owned()));
        }
        for (text, content, cmid) in other_prompts {
            if let Some(c) = cmid.as_deref()
                && s.committed_recovery_cmids.contains(c)
            {
                continue;
            }
            let draft = QueuedMessage {
                id: format!("recovery-{}", self.next_qid()),
                text: text.clone(),
                content: content.clone(),
                cmid: cmid.clone(),
                schedule: None,
                origin: None,
                initiator: None,
                explicit: false,
                wake_dormant: false,
            };
            drafts.push(draft.clone());
            added_drafts.push(draft);
            if let Some(c) = cmid.as_ref().filter(|c| !c.is_empty()) {
                queue.retain(|m| m.cmid.as_deref() != Some(c.as_str()));
                let alias = format!("recovery-{c}");
                drafts.retain(|m| m.cmid.as_deref() != Some(alias.as_str()));
                staged_cmids.insert(c.clone());
                committed_recovery_cmids.push(c.clone());
            }
        }
        let recovery_cmids = if staged_cmids.is_empty() {
            None
        } else {
            let mut list: Vec<String> = staged_cmids.into_iter().collect();
            list.sort();
            Some(list)
        };
        let rx = self.send_acked_update_pending_with_recovery(
            session_id,
            queue,
            drafts,
            settled_notices.clone(),
            Vec::new(),
            recovery_cmids,
            false,
        )?;
        let mut claims = self.inner.notice_claims.lock();
        for (cid, _) in &settled_notices {
            claims
                .withdrawing
                .insert((session_id.to_owned(), cid.clone()));
        }
        claims
            .in_flight_pending_commits
            .insert(session_id.to_owned(), DeferredPendingFlush::default());
        Some((
            PendingRecoveryPark {
                hub: Arc::downgrade(&self.inner),
                active: true,
                session_id: session_id.to_owned(),
                added_drafts,
                replaced_draft_id: None,
                committed_recovery_cmids,
                settled_notices,
            },
            rx,
        ))
    }

    /// Stage parking a single ambiguous ordinary prompt as a `recovery-*` draft
    /// AND its durable `session_recovery_cmids:<session_id>` tombstone inside
    /// ONE atomic acknowledged `StoreWrite::UpdatePending` transaction (Findings 4 & 5).
    pub fn preview_park_recovery_prompt(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) -> RecoveryDraftPark {
        if !self.has_store_writer() {
            self.add_recovery_draft(session_id, text, content, cmid);
            return RecoveryDraftPark::NoStore;
        }
        let sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get(session_id) else {
            return RecoveryDraftPark::Rejected("session unavailable");
        };
        // Finding 7: Once a recovery operation ID is durably committed, settle
        // retries immediately even if the owner already edited/deleted/activated the draft.
        if let Some(c) = cmid.as_deref()
            && s.committed_recovery_cmids.contains(c)
        {
            let dup = c.to_owned();
            drop(sessions);
            self.confirm_delivered(session_id, &dup);
            return RecoveryDraftPark::Duplicate;
        }
        let mut queue = s.queue.clone();
        let mut drafts = s.drafts.clone();
        let replaced_draft_id = cmid.as_deref().and_then(|c| {
            queue.retain(|m| m.cmid.as_deref() != Some(c));
            let alias = format!("recovery-{c}");
            drafts
                .iter()
                .position(|m| {
                    m.cmid.as_deref() == Some(c) || m.cmid.as_deref() == Some(alias.as_str())
                })
                .map(|pos| drafts.remove(pos).id)
        });
        let draft = QueuedMessage {
            id: format!("recovery-{}", self.next_qid()),
            text,
            content,
            cmid: cmid.clone(),
            schedule: None,
            origin: None,
            initiator: None,
            explicit: false,
            wake_dormant: false,
        };
        drafts.push(draft.clone());
        let mut staged_cmids = s.recovery_cmids.clone();
        let mut committed_recovery_cmids = Vec::new();
        if let Some(c) = cmid.filter(|c| !c.is_empty()) {
            staged_cmids.insert(c.clone());
            committed_recovery_cmids.push(c);
        }
        let recovery_cmids_setting = if staged_cmids.is_empty() {
            None
        } else {
            let mut list: Vec<String> = staged_cmids.into_iter().collect();
            list.sort();
            Some(list)
        };
        let Some(rx) = self.send_acked_update_pending_with_recovery(
            session_id,
            queue,
            drafts,
            Vec::new(),
            Vec::new(),
            recovery_cmids_setting,
            false,
        ) else {
            return RecoveryDraftPark::Rejected("store writer unavailable");
        };
        let mut claims = self.inner.notice_claims.lock();
        if let Some(ref old_id) = replaced_draft_id {
            // Finding 8: Fence the converted draft row while the commit is in flight.
            claims
                .withdrawing_row_ids
                .insert((session_id.to_owned(), old_id.clone()));
        }
        claims
            .in_flight_pending_commits
            .insert(session_id.to_owned(), DeferredPendingFlush::default());
        RecoveryDraftPark::Pending(
            Box::new(PendingRecoveryPark {
                hub: Arc::downgrade(&self.inner),
                active: true,
                session_id: session_id.to_owned(),
                added_drafts: vec![draft],
                replaced_draft_id,
                committed_recovery_cmids,
                settled_notices: Vec::new(),
            }),
            rx,
        )
    }

    /// Atomically activate a `recovery-*` draft: when a durable store writer is
    /// attached, fences the draft row + reserves the new operation ID and awaits
    /// acknowledged atomic `UpdatePending` BEFORE mutating Hub or allowing
    /// dispatch (Findings 4 & 10).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::core) fn reserve_scheduled_recovery_activation_locked(
        &self,
        s: &mut Session,
        session_id: &str,
        draft_id: &str,
        pos: usize,
        op: String,
        trigger: crate::prompt_origin::DeliveryOrigin,
        front: bool,
    ) -> Option<(String, Option<(bool, bool)>)> {
        let target = &s.drafts[pos];
        if op.is_empty()
            || Some(op.as_str()) == target.cmid.as_deref()
            || Self::prompt_already_accepted(s, &op)
        {
            return None;
        }
        if !self.has_store_writer() {
            let m = s.drafts.remove(pos);
            let had_schedule = m.schedule.is_some();
            let dispatch_origin =
                crate::prompt_origin::compute_dispatch_origin(m.origin.as_ref(), Some(&trigger));
            let row = QueuedMessage {
                id: self.next_qid(),
                text: m.text,
                content: m.content,
                cmid: Some(op.clone()),
                schedule: None,
                origin: m.origin,
                initiator: Some(trigger),
                explicit: false,
                wake_dormant: true,
            };
            if front {
                s.queue.insert(0, row);
            } else {
                s.queue.push(row);
            }
            return Some((op, Some((had_schedule, dispatch_origin.is_owner()))));
        }
        let mut claims = self.inner.notice_claims.lock();
        if !claims
            .withdrawing_row_ids
            .insert((session_id.to_owned(), draft_id.to_owned()))
        {
            return None;
        }
        s.pending_recovery_cmids.insert(op.clone());
        Some((op, None))
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::core) fn spawn_commit_activated_recovery_draft(
        &self,
        sid: String,
        draft_id: String,
        op: String,
        trigger: crate::prompt_origin::DeliveryOrigin,
        front: bool,
        explicit: bool,
        wake_dormant: bool,
    ) {
        let hub = self.clone();
        self.spawn_teardown_requeue(async move {
            hub.commit_activated_recovery_draft(
                sid,
                draft_id,
                op,
                trigger,
                front,
                explicit,
                wake_dormant,
            )
            .await;
        });
    }

    fn activate_recovery_draft_in_memory(
        &self,
        session_id: &str,
        id: &str,
        cmid: Option<String>,
        trigger: crate::prompt_origin::DeliveryOrigin,
    ) {
        let moved = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if self.is_row_withdrawing(session_id, id) {
                return;
            }
            let Some(pos) = s.drafts.iter().position(|m| m.id == id) else {
                return;
            };
            let target = &s.drafts[pos];
            let Some(op) = cmid
                .as_deref()
                .filter(|op| {
                    !op.is_empty()
                        && Some(*op) != target.cmid.as_deref()
                        && !Self::prompt_already_accepted(s, op)
                })
                .map(str::to_owned)
            else {
                drop(sessions);
                self.broadcast_error(
                    Some(session_id.to_owned()),
                    "Resending a parked recovery draft requires a new operation id".to_owned(),
                );
                return;
            };
            let m = s.drafts.remove(pos);
            let had_schedule = m.schedule.is_some();
            let dispatch_origin =
                crate::prompt_origin::compute_dispatch_origin(m.origin.as_ref(), Some(&trigger));
            let new_id = self.next_qid();
            s.queue.push(QueuedMessage {
                id: new_id.clone(),
                text: m.text,
                content: m.content,
                cmid: Some(op),
                schedule: None,
                origin: m.origin,
                initiator: Some(trigger),
                explicit: true,
                wake_dormant: true,
            });
            (new_id, had_schedule, dispatch_origin.is_owner())
        };
        self.emit_pending(session_id);
        if moved.1 {
            self.cancel_draft_timer(session_id, id);
            self.broadcast_sessions();
        }
        if moved.2 {
            self.notify_human_turn(session_id);
        }
        self.try_drain(session_id);
    }

    pub(in crate::core) fn activate_recovery_draft_durable(
        &self,
        session_id: &str,
        id: &str,
        cmid: Option<String>,
        trigger: crate::prompt_origin::DeliveryOrigin,
    ) {
        if !self.has_store_writer() {
            self.activate_recovery_draft_in_memory(session_id, id, cmid, trigger);
            return;
        }
        let reserved = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let Some(pos) = s.drafts.iter().position(|m| m.id == id) else {
                return;
            };
            let target = &s.drafts[pos];
            let Some(op) = cmid
                .as_deref()
                .filter(|op| {
                    !op.is_empty()
                        && Some(*op) != target.cmid.as_deref()
                        && !Self::prompt_already_accepted(s, op)
                })
                .map(str::to_owned)
            else {
                drop(sessions);
                self.broadcast_error(
                    Some(session_id.to_owned()),
                    "Resending a parked recovery draft requires a new operation id".to_owned(),
                );
                return;
            };
            let mut claims = self.inner.notice_claims.lock();
            if !claims
                .withdrawing_row_ids
                .insert((session_id.to_owned(), id.to_owned()))
            {
                return;
            }
            s.pending_recovery_cmids.insert(op.clone());
            op
        };
        self.spawn_commit_activated_recovery_draft(
            session_id.to_owned(),
            id.to_owned(),
            reserved,
            trigger,
            false,
            true,
            true,
        );
    }

    fn abort_pre_stage_recovery_activation(&self, sid: &str, draft_id: &str, op: &str) {
        let rearm = {
            let mut sessions = self.inner.sessions.lock();
            let mut claims = self.inner.notice_claims.lock();
            claims
                .withdrawing_row_ids
                .remove(&(sid.to_owned(), draft_id.to_owned()));
            sessions.get_mut(sid).and_then(|s| {
                s.pending_recovery_cmids.remove(op);
                s.drafts
                    .iter()
                    .find(|m| m.id == draft_id)
                    .and_then(|m| m.schedule.as_ref().map(|sc| (sc.fire_at_ms, sc.generation)))
            })
        };
        if let Some((fire_at_ms, generation)) = rearm {
            self.arm_draft_timer(sid, draft_id, fire_at_ms, generation);
        }
        self.broadcast_error(
            Some(sid.to_owned()),
            "Could not persist recovery draft activation; draft retained".to_owned(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn stage_recovery_draft_activation(
        &self,
        sid: &str,
        draft_id: &str,
        op: &str,
        trigger: &crate::prompt_origin::DeliveryOrigin,
        front: bool,
        explicit: bool,
        wake_dormant: bool,
    ) -> Option<StagedRecoveryActivation> {
        let mut sessions = self.inner.sessions.lock();
        let mut claims = self.inner.notice_claims.lock();
        let Some(s) = sessions.get_mut(sid) else {
            claims
                .withdrawing_row_ids
                .remove(&(sid.to_owned(), draft_id.to_owned()));
            return None;
        };
        let Some(pos) = s.drafts.iter().position(|m| m.id == draft_id) else {
            s.pending_recovery_cmids.remove(op);
            claims
                .withdrawing_row_ids
                .remove(&(sid.to_owned(), draft_id.to_owned()));
            return None;
        };
        let m = s.drafts[pos].clone();
        let schedule_info = m.schedule.as_ref().map(|sc| (sc.fire_at_ms, sc.generation));
        let new_row = QueuedMessage {
            id: self.next_qid(),
            text: m.text,
            content: m.content,
            cmid: Some(op.to_owned()),
            schedule: None,
            origin: m.origin.clone(),
            initiator: Some(trigger.clone()),
            explicit,
            wake_dormant,
        };
        let mut next_queue = s.queue.clone();
        if front {
            next_queue.insert(0, new_row.clone());
        } else {
            next_queue.push(new_row.clone());
        }
        let mut next_drafts = s.drafts.clone();
        next_drafts.remove(pos);
        let Some(rx) = self.send_acked_update_pending_with_recovery(
            sid,
            next_queue,
            next_drafts,
            Vec::new(),
            Vec::new(),
            Self::sorted_recovery_cmids(s),
            false,
        ) else {
            drop(claims);
            drop(sessions);
            self.abort_pre_stage_recovery_activation(sid, draft_id, op);
            return None;
        };
        claims
            .in_flight_pending_commits
            .insert(sid.to_owned(), DeferredPendingFlush::default());
        let dispatch_origin =
            crate::prompt_origin::compute_dispatch_origin(m.origin.as_ref(), Some(trigger));
        Some((rx, new_row, schedule_info, dispatch_origin.is_owner()))
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit_activated_recovery_draft(
        &self,
        sid: String,
        draft_id: String,
        op: String,
        trigger: crate::prompt_origin::DeliveryOrigin,
        front: bool,
        explicit: bool,
        wake_dormant: bool,
    ) {
        let Some(_commit_guard) = self
            .await_with_shutdown(self.pending_commit_lock(&sid))
            .await
        else {
            self.abort_pre_stage_recovery_activation(&sid, &draft_id, &op);
            return;
        };
        let Some((rx, new_row, schedule_info, is_owner)) = self.stage_recovery_draft_activation(
            &sid,
            &draft_id,
            &op,
            &trigger,
            front,
            explicit,
            wake_dormant,
        ) else {
            return;
        };
        let outcome = self.await_with_shutdown(rx).await;
        if outcome.is_none() {
            self.poison_session_pending_writes(&sid);
            return;
        }
        let committed = matches!(outcome, Some(Ok(Ok(true))));
        let (broadcast, rearm, drains) = {
            let mut sessions = self.inner.sessions.lock();
            let mut claims = self.inner.notice_claims.lock();
            claims
                .withdrawing_row_ids
                .remove(&(sid.clone(), draft_id.clone()));
            if let Some(s) = sessions.get_mut(&sid) {
                s.pending_recovery_cmids.remove(&op);
                if committed {
                    s.drafts.retain(|m| m.id != draft_id);
                    if front {
                        s.queue.insert(0, new_row.clone());
                    } else {
                        s.queue.push(new_row.clone());
                    }
                }
                let rearm = (!committed)
                    .then(|| {
                        schedule_info.and_then(|_| {
                            s.drafts.iter().find(|m| m.id == draft_id).and_then(|m| {
                                m.schedule.as_ref().map(|sc| (sc.fire_at_ms, sc.generation))
                            })
                        })
                    })
                    .flatten();
                let queue = s.queue.clone();
                let drafts = s.drafts.clone();
                let drains = self.finish_in_flight_pending_commit(
                    &sid,
                    &queue,
                    &drafts,
                    Self::sorted_recovery_cmids(s),
                    &mut claims,
                );
                (Some((queue, drafts)), rearm, drains)
            } else {
                claims.in_flight_pending_commits.remove(&sid);
                (None, None, DeferredDrainActions::default())
            }
        };
        if committed {
            if let Some((queue, drafts)) = broadcast {
                self.broadcast_pending_snapshot(&sid, &queue, &drafts);
            }
            if schedule_info.is_some() {
                self.cancel_draft_timer(&sid, &draft_id);
                self.broadcast_sessions();
            }
            if is_owner {
                self.notify_human_turn(&sid);
            }
        } else {
            if let Some((fire_at_ms, generation)) = rearm {
                self.arm_draft_timer(&sid, &draft_id, fire_at_ms, generation);
            }
            self.broadcast_error(
                Some(sid.clone()),
                "Could not persist recovery draft activation; draft retained".to_owned(),
            );
        }
        self.replay_deferred_drains(&sid, drains);
    }

    /// Commit an acknowledged ordinary recovery draft park + atomic tombstone
    /// setting into Hub memory without issuing a second store write.
    pub fn commit_park_recovery_prompt(&self, preview: &mut PendingRecoveryPark) {
        if !preview.active {
            return;
        }
        preview.active = false;
        let (queue, drafts, setting_update, drains) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(&preview.session_id) else {
                let mut claims = self.inner.notice_claims.lock();
                for (cid, _) in &preview.settled_notices {
                    claims
                        .withdrawing
                        .remove(&(preview.session_id.clone(), cid.clone()));
                }
                if let Some(ref old_id) = preview.replaced_draft_id {
                    claims
                        .withdrawing_row_ids
                        .remove(&(preview.session_id.clone(), old_id.clone()));
                }
                claims.in_flight_pending_commits.remove(&preview.session_id);
                return;
            };
            if let Some(ref old_id) = preview.replaced_draft_id {
                s.drafts.retain(|m| m.id != *old_id);
            }
            for cmid in &preview.committed_recovery_cmids {
                let alias = format!("recovery-{cmid}");
                s.drafts
                    .retain(|m| m.cmid.as_deref() != Some(alias.as_str()));
                let before_q = s.queue.len();
                s.queue.retain(|m| m.cmid.as_deref() != Some(cmid.as_str()));
                if before_q != s.queue.len()
                    && s.editing
                        .as_deref()
                        .is_some_and(|ed| !s.queue.iter().any(|m| m.id == ed))
                {
                    s.editing = None;
                }
                hub_dispatch_origin::remove_pending_echo(s, cmid);
                s.recovery_cmids.insert(cmid.clone());
                s.committed_recovery_cmids.insert(cmid.clone());
                s.pending_recovery_cmids.remove(cmid);
            }
            s.drafts.extend(preview.added_drafts.iter().cloned());
            let recovery_cmids = Self::sorted_recovery_cmids(s);
            let queue = s.queue.clone();
            let drafts = s.drafts.clone();
            let mut claims = self.inner.notice_claims.lock();
            for (cid, _) in &preview.settled_notices {
                let key = (preview.session_id.clone(), cid.clone());
                claims.withdrawing.remove(&key);
                claims.active.remove(&key);
                claims.pending_releases.remove(&key);
            }
            if let Some(ref old_id) = preview.replaced_draft_id {
                claims
                    .withdrawing_row_ids
                    .remove(&(preview.session_id.clone(), old_id.clone()));
            }
            let drains = self.finish_in_flight_pending_commit(
                &preview.session_id,
                &queue,
                &drafts,
                recovery_cmids.clone(),
                &mut claims,
            );
            (queue, drafts, recovery_cmids, drains)
        };
        if let Some(list) = setting_update {
            let key = format!(
                "{}{}",
                settings_keys::SESSION_RECOVERY_CMIDS_PREFIX,
                preview.session_id
            );
            self.load_settings(vec![(key, serde_json::json!(list))]);
        }
        self.broadcast_pending_snapshot(&preview.session_id, &queue, &drafts);
        self.replay_deferred_drains(&preview.session_id, drains);
    }

    /// Apply an already-committed durable recovery park to `Hub`.
    pub fn commit_park_recovery_notices(&self, preview: &mut PendingRecoveryPark) {
        if !preview.active {
            return;
        }
        preview.active = false;
        let (queue, drafts, drains) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(&preview.session_id) else {
                let mut claims = self.inner.notice_claims.lock();
                for (cid, _) in &preview.settled_notices {
                    claims
                        .withdrawing
                        .remove(&(preview.session_id.clone(), cid.clone()));
                }
                claims.in_flight_pending_commits.remove(&preview.session_id);
                return;
            };
            s.drafts.extend(preview.added_drafts.iter().cloned());
            let queue = s.queue.clone();
            let drafts = s.drafts.clone();
            let mut claims = self.inner.notice_claims.lock();
            for (cid, _) in &preview.settled_notices {
                let key = (preview.session_id.clone(), cid.clone());
                claims.withdrawing.remove(&key);
                claims.active.remove(&key);
                claims.pending_releases.remove(&key);
            }
            let drains = self.finish_in_flight_pending_commit(
                &preview.session_id,
                &queue,
                &drafts,
                Self::sorted_recovery_cmids(s),
                &mut claims,
            );
            (queue, drafts, drains)
        };
        self.broadcast_pending_snapshot(&preview.session_id, &queue, &drafts);
        self.replay_deferred_drains(&preview.session_id, drains);
    }

    pub fn abandon_unknown_outcome_recovery_park(&self, preview: &mut PendingRecoveryPark) {
        if !preview.active {
            return;
        }
        preview.active = false;
        self.poison_session_pending_writes(&preview.session_id);
    }

    /// Abort a durable recovery park without mutating `Hub.drafts`, lifting its
    /// fences and flushing any deferred pending snapshot.
    pub fn abort_park_recovery_notices(&self, preview: &mut PendingRecoveryPark) {
        if !preview.active {
            return;
        }
        preview.active = false;
        let drains = {
            let sessions = self.inner.sessions.lock();
            let mut claims = self.inner.notice_claims.lock();
            for (cid, _) in &preview.settled_notices {
                claims
                    .withdrawing
                    .remove(&(preview.session_id.clone(), cid.clone()));
            }
            if let Some(ref old_id) = preview.replaced_draft_id {
                claims
                    .withdrawing_row_ids
                    .remove(&(preview.session_id.clone(), old_id.clone()));
            }
            if let Some(s) = sessions.get(&preview.session_id) {
                self.finish_in_flight_pending_commit(
                    &preview.session_id,
                    &s.queue,
                    &s.drafts,
                    Self::sorted_recovery_cmids(s),
                    &mut claims,
                )
            } else {
                claims.in_flight_pending_commits.remove(&preview.session_id);
                DeferredDrainActions::default()
            }
        };
        self.replay_deferred_drains(&preview.session_id, drains);
    }

    /// Atomically inspect and apply an owner queue mutation under ONE `sessions`
    /// lock:
    /// - If no queued `call-notice:` row is affected, applies the mutation right
    ///   there under the same lock so a concurrently arriving notice can never
    ///   be wiped by an unlocked second pass (`OwnerQueueMutation::AppliedSync`).
    /// - If one or more queued `call-notice:` rows are affected, installs the
    ///   withdrawal (for BOTH affected notice `cmid`s AND every affected queue
    ///   `row_id`, so a mixed `ClearQueue` cannot drain a plain prompt ahead of
    ///   the notice while the durable commit is in flight) + pending-snapshot
    ///   fences and enqueues the acknowledged FIFO `StoreWrite::UpdatePending`
    ///   (`OwnerQueueMutation::CommitPending`).
    pub fn apply_or_preview_owner_notice_withdrawal(
        &self,
        session_id: &str,
        op: &OwnerNoticeWithdrawalOp<'_>,
    ) -> OwnerQueueMutation {
        let sync_broadcast = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return OwnerQueueMutation::AppliedSync(false);
            };
            if matches!(op, OwnerNoticeWithdrawalOp::EditQueued { .. })
                && s.meta
                    .execution_binding
                    .as_ref()
                    .is_some_and(|binding| binding.managed().is_some())
            {
                return OwnerQueueMutation::AppliedSync(false);
            }
            let clear_ids = if matches!(op, OwnerNoticeWithdrawalOp::ClearQueue) {
                s.queue.iter().map(|m| m.id.clone()).collect()
            } else {
                HashSet::new()
            };
            let fenced_row_ids = affected_row_ids(&s.queue, op, &clear_ids);
            let mut queue = s.queue.clone();
            let mut drafts = s.drafts.clone();
            let mut editing = s.editing.clone();
            let (changed, settled_notices) =
                apply_withdrawal_to_lists(&mut queue, &mut drafts, &mut editing, op, &clear_ids);
            if settled_notices.is_empty() {
                if !changed {
                    return OwnerQueueMutation::AppliedSync(false);
                }
                s.queue = queue.clone();
                s.drafts = drafts.clone();
                s.editing = editing;
                let mut claims = self.inner.notice_claims.lock();
                if self.is_pending_writes_poisoned(session_id) {
                    // Never send UpdatePending behind a timed-out unknown-outcome commit.
                } else if let Some(deferred) = claims.in_flight_pending_commits.get_mut(session_id)
                {
                    deferred.dirty = true;
                } else if let Some(tx) = self.inner.store_tx.as_ref() {
                    let _ = tx.send(StoreWrite::UpdatePending {
                        session_id: session_id.to_owned(),
                        queue: queue.clone(),
                        drafts: drafts.clone(),
                        settled_notices: Vec::new(),
                        released_claims: Vec::new(),
                        recovery_cmids: Self::sorted_recovery_cmids(s),
                        require_claim_release: false,
                        ack_id: None,
                    });
                }
                (queue, drafts)
            } else {
                let Some(rx) = self.send_acked_update_pending_with_recovery(
                    session_id,
                    queue.clone(),
                    drafts.clone(),
                    settled_notices.clone(),
                    Vec::new(),
                    Self::sorted_recovery_cmids(s),
                    false,
                ) else {
                    return OwnerQueueMutation::Rejected(
                        "durable store writer unavailable; completion notice withdrawal was not committed",
                    );
                };
                let mut claims = self.inner.notice_claims.lock();
                for (cid, _) in &settled_notices {
                    claims
                        .withdrawing
                        .insert((session_id.to_owned(), cid.clone()));
                }
                for row_id in &fenced_row_ids {
                    claims
                        .withdrawing_row_ids
                        .insert((session_id.to_owned(), row_id.clone()));
                }
                claims
                    .in_flight_pending_commits
                    .insert(session_id.to_owned(), DeferredPendingFlush::default());
                return OwnerQueueMutation::CommitPending(
                    Box::new(PendingNoticeWithdrawal {
                        hub: Arc::downgrade(&self.inner),
                        active: true,
                        session_id: session_id.to_owned(),
                        queue,
                        drafts,
                        settled_notices,
                        clear_ids,
                        fenced_row_ids,
                    }),
                    rx,
                );
            }
        };
        self.broadcast_pending_snapshot(session_id, &sync_broadcast.0, &sync_broadcast.1);
        self.try_drain(session_id);
        OwnerQueueMutation::AppliedSync(true)
    }

    /// Atomically preview an owner queue mutation that affects one or more
    /// queued `call-notice:` rows, install its withdrawal + pending-snapshot
    /// fence, AND admit its acknowledged `StoreWrite::UpdatePending` to the
    /// FIFO store writer under the SAME `sessions` lock.
    pub fn preview_and_send_owner_notice_withdrawal(
        &self,
        session_id: &str,
        op: &OwnerNoticeWithdrawalOp<'_>,
    ) -> Option<Result<PendingNoticeWithdrawalCommit, &'static str>> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        if matches!(op, OwnerNoticeWithdrawalOp::EditQueued { .. })
            && s.meta
                .execution_binding
                .as_ref()
                .is_some_and(|binding| binding.managed().is_some())
        {
            return None;
        }
        let clear_ids = if matches!(op, OwnerNoticeWithdrawalOp::ClearQueue) {
            s.queue.iter().map(|m| m.id.clone()).collect()
        } else {
            HashSet::new()
        };
        let fenced_row_ids = affected_row_ids(&s.queue, op, &clear_ids);
        let mut queue = s.queue.clone();
        let mut drafts = s.drafts.clone();
        let mut editing = s.editing.clone();
        let (_, settled_notices) =
            apply_withdrawal_to_lists(&mut queue, &mut drafts, &mut editing, op, &clear_ids);
        if settled_notices.is_empty() {
            return None;
        }
        let Some(rx) = self.send_acked_update_pending_with_recovery(
            session_id,
            queue.clone(),
            drafts.clone(),
            settled_notices.clone(),
            Vec::new(),
            Self::sorted_recovery_cmids(s),
            false,
        ) else {
            return Some(Err(
                "durable store writer unavailable; completion notice withdrawal was not committed",
            ));
        };
        let mut claims = self.inner.notice_claims.lock();
        for (cid, _) in &settled_notices {
            claims
                .withdrawing
                .insert((session_id.to_owned(), cid.clone()));
        }
        for row_id in &fenced_row_ids {
            claims
                .withdrawing_row_ids
                .insert((session_id.to_owned(), row_id.clone()));
        }
        claims
            .in_flight_pending_commits
            .insert(session_id.to_owned(), DeferredPendingFlush::default());
        Some(Ok((
            PendingNoticeWithdrawal {
                hub: Arc::downgrade(&self.inner),
                active: true,
                session_id: session_id.to_owned(),
                queue,
                drafts,
                settled_notices,
                clear_ids,
                fenced_row_ids,
            },
            rx,
        )))
    }

    /// Release a withdrawal fence without mutating `Hub` when the durable store
    /// commit failed or was unavailable.
    pub fn abort_owner_notice_withdrawal(&self, preview: &mut PendingNoticeWithdrawal) {
        if !preview.active {
            return;
        }
        preview.active = false;
        let drains = {
            let sessions = self.inner.sessions.lock();
            let mut claims = self.inner.notice_claims.lock();
            for (cid, _) in &preview.settled_notices {
                claims
                    .withdrawing
                    .remove(&(preview.session_id.clone(), cid.clone()));
            }
            for row_id in &preview.fenced_row_ids {
                claims
                    .withdrawing_row_ids
                    .remove(&(preview.session_id.clone(), row_id.clone()));
            }
            if let Some(s) = sessions.get(&preview.session_id) {
                self.finish_in_flight_pending_commit(
                    &preview.session_id,
                    &s.queue,
                    &s.drafts,
                    Self::sorted_recovery_cmids(s),
                    &mut claims,
                )
            } else {
                claims.in_flight_pending_commits.remove(&preview.session_id);
                DeferredDrainActions::default()
            }
        };
        self.replay_deferred_drains(&preview.session_id, drains);
    }

    /// Apply an already-committed owner notice withdrawal to `Hub`, lifting the
    /// withdrawal + pending-snapshot fences, flushing any merged deferred
    /// snapshot under `sessions` lock, broadcasting, and draining any surviving head.
    pub fn commit_owner_notice_withdrawal(
        &self,
        preview: &mut PendingNoticeWithdrawal,
        op: &OwnerNoticeWithdrawalOp<'_>,
    ) -> bool {
        if !preview.active {
            return false;
        }
        preview.active = false;
        let (changed, queue, drafts, drains) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(&preview.session_id) else {
                let mut claims = self.inner.notice_claims.lock();
                for (cid, _) in &preview.settled_notices {
                    claims
                        .withdrawing
                        .remove(&(preview.session_id.clone(), cid.clone()));
                }
                for row_id in &preview.fenced_row_ids {
                    claims
                        .withdrawing_row_ids
                        .remove(&(preview.session_id.clone(), row_id.clone()));
                }
                claims.in_flight_pending_commits.remove(&preview.session_id);
                return false;
            };
            let (changed, _) = apply_withdrawal_to_lists(
                &mut s.queue,
                &mut s.drafts,
                &mut s.editing,
                op,
                &preview.clear_ids,
            );
            let queue = s.queue.clone();
            let drafts = s.drafts.clone();
            let mut claims = self.inner.notice_claims.lock();
            for (cid, _) in &preview.settled_notices {
                let key = (preview.session_id.clone(), cid.clone());
                claims.withdrawing.remove(&key);
                claims.active.remove(&key);
                claims.pending_releases.remove(&key);
            }
            for row_id in &preview.fenced_row_ids {
                claims
                    .withdrawing_row_ids
                    .remove(&(preview.session_id.clone(), row_id.clone()));
            }
            let drains = self.finish_in_flight_pending_commit(
                &preview.session_id,
                &queue,
                &drafts,
                Self::sorted_recovery_cmids(s),
                &mut claims,
            );
            (changed, queue, drafts, drains)
        };
        self.broadcast_pending_snapshot(&preview.session_id, &queue, &drafts);
        self.replay_deferred_drains(&preview.session_id, drains);
        changed
    }

    /// Return the exact queued row ID currently carrying `cmid`, if any.
    #[must_use]
    pub fn queued_row_id_by_cmid(&self, session_id: &str, cmid: &str) -> Option<String> {
        let sessions = self.inner.sessions.lock();
        sessions
            .get(session_id)?
            .queue
            .iter()
            .find(|m| m.cmid.as_deref() == Some(cmid))
            .map(|m| m.id.clone())
    }

    /// Remove a queued row ONLY IF its exact `row_id` AND `cmid` still match,
    /// preventing startup reconciliation from deleting a freshly reopened row.
    pub fn remove_queued_by_id_if_cmid(&self, session_id: &str, row_id: &str, cmid: &str) -> bool {
        let removed = {
            let mut sessions = self.inner.sessions.lock();
            sessions.get_mut(session_id).is_some_and(|s| {
                let before = s.queue.len();
                s.queue
                    .retain(|m| !(m.id == row_id && m.cmid.as_deref() == Some(cmid)));
                if s.editing.as_deref() == Some(row_id) && before != s.queue.len() {
                    s.editing = None;
                }
                before != s.queue.len()
            })
        };
        if removed {
            self.emit_pending(session_id);
        }
        removed
    }

    /// All currently queued completion notices across sessions as `(parent_session_id, completion_id)`.
    #[must_use]
    pub fn queued_call_notices(&self) -> Vec<(String, String)> {
        let sessions = self.inner.sessions.lock();
        let mut notices = Vec::new();
        for (id, s) in sessions.iter() {
            for row in &s.queue {
                if let Some(cmid) = row
                    .cmid
                    .as_deref()
                    .filter(|c| c.starts_with(crate::agent_model::CALL_NOTICE_PREFIX))
                {
                    notices.push((id.clone(), cmid.to_owned()));
                }
            }
        }
        notices
    }

    /// The `call-notice:` completion ID at the head of a session's queue, if any.
    #[must_use]
    pub fn queue_head_call_notice(&self, session_id: &str) -> Option<String> {
        let sessions = self.inner.sessions.lock();
        sessions
            .get(session_id)?
            .queue
            .first()
            .and_then(|row| row.cmid.as_deref())
            .filter(|cmid| cmid.starts_with(crate::agent_model::CALL_NOTICE_PREFIX))
            .map(str::to_owned)
    }
    /// Park an unacknowledged prompt as a draft during execution recovery.
    /// For a `call-notice:`, strips the reserved `cmid` so the parked draft is
    /// never rejected or re-bound to the notice lifecycle, clears any active
    /// claim, and records `execution_recovery_parked` atomically with the draft.
    pub fn park_recovery_draft(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) {
        let Some(notice_cid) = cmid
            .as_deref()
            .filter(|id| id.starts_with(crate::agent_model::CALL_NOTICE_PREFIX))
            .map(str::to_owned)
        else {
            self.add_recovery_draft(session_id, text, content, cmid);
            return;
        };
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let id = self.next_qid();
            s.drafts.push(QueuedMessage {
                id,
                text,
                content,
                cmid: None,
                schedule: None,
                origin: Some(crate::prompt_origin::DeliveryOrigin::cowboy("managed_call")),
                initiator: None,
                explicit: false,
                wake_dormant: false,
            });
            let mut claims = self.inner.notice_claims.lock();
            let key = (session_id.to_owned(), notice_cid.clone());
            claims.active.remove(&key);
            claims.pending_releases.remove(&key);
        }
        self.emit_pending_with_notice_ops(
            session_id,
            vec![(notice_cid, "execution_recovery_parked".to_owned())],
            Vec::new(),
        );
    }
    pub(super) fn make_notice_requeue_row(
        &self,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: &str,
    ) -> QueuedMessage {
        let origin = Some(crate::prompt_origin::DeliveryOrigin::cowboy("managed_call"));
        QueuedMessage {
            id: self.next_qid(),
            text,
            content,
            cmid: Some(cmid.to_owned()),
            schedule: None,
            origin: origin.clone(),
            initiator: origin,
            explicit: false,
            wake_dormant: false,
        }
    }
}
