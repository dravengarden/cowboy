//! Owner queue withdrawals, recovery parking, and queue inspection for
//! durable managed-call completion notices.

use super::types::{
    DeferredPendingFlush, PendingRecoveryPromptItem, affected_row_ids, apply_withdrawal_to_lists,
};
use super::*;

impl Hub {
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
        let queue = s.queue.clone();
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
            };
            drafts.push(draft.clone());
            added_drafts.push(draft);
            if let Some(c) = cmid.as_ref().filter(|c| !c.is_empty()) {
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
        let mut drafts = s.drafts.clone();
        let replaced_draft_id = cmid.as_deref().and_then(|c| {
            drafts
                .iter()
                .position(|m| m.cmid.as_deref() == Some(c))
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
        let queue = s.queue.clone();
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
        self.drain_activated_recovery_row(session_id, &moved.0);
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
        let staged = {
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
            let m = target.clone();
            let new_row = QueuedMessage {
                id: self.next_qid(),
                text: m.text,
                content: m.content,
                cmid: Some(op.clone()),
                schedule: None,
                origin: m.origin.clone(),
                initiator: Some(trigger.clone()),
            };
            let mut next_queue = s.queue.clone();
            next_queue.push(new_row.clone());
            let mut next_drafts = s.drafts.clone();
            next_drafts.remove(pos);
            let Some(rx) = self.send_acked_update_pending_with_recovery(
                session_id,
                next_queue,
                next_drafts,
                Vec::new(),
                Vec::new(),
                Self::sorted_recovery_cmids(s),
                false,
            ) else {
                drop(sessions);
                self.broadcast_error(
                    Some(session_id.to_owned()),
                    "Could not persist recovery draft activation; draft retained".to_owned(),
                );
                return;
            };
            s.pending_recovery_cmids.insert(op.clone());
            let mut claims = self.inner.notice_claims.lock();
            claims
                .withdrawing_row_ids
                .insert((session_id.to_owned(), id.to_owned()));
            claims
                .in_flight_pending_commits
                .insert(session_id.to_owned(), DeferredPendingFlush::default());
            let dispatch_origin =
                crate::prompt_origin::compute_dispatch_origin(m.origin.as_ref(), Some(&trigger));
            (
                rx,
                op,
                new_row,
                m.schedule.is_some(),
                dispatch_origin.is_owner(),
            )
        };
        let (rx, op, new_row, had_schedule, is_owner) = staged;
        let hub = self.clone();
        let sid = session_id.to_owned();
        let draft_id = id.to_owned();
        self.spawn_teardown_requeue(async move {
            let committed = matches!(rx.await, Ok(Ok(true)));
            let broadcast = {
                let mut sessions = hub.inner.sessions.lock();
                let mut claims = hub.inner.notice_claims.lock();
                claims
                    .withdrawing_row_ids
                    .remove(&(sid.clone(), draft_id.clone()));
                if let Some(s) = sessions.get_mut(&sid) {
                    s.pending_recovery_cmids.remove(&op);
                    if committed {
                        s.drafts.retain(|m| m.id != draft_id);
                        s.queue.push(new_row.clone());
                    }
                    let queue = s.queue.clone();
                    let drafts = s.drafts.clone();
                    hub.finish_in_flight_pending_commit(
                        &sid,
                        &queue,
                        &drafts,
                        Self::sorted_recovery_cmids(s),
                        &mut claims,
                    );
                    Some((queue, drafts))
                } else {
                    claims.in_flight_pending_commits.remove(&sid);
                    None
                }
            };
            if committed {
                if let Some((queue, drafts)) = broadcast {
                    hub.broadcast_pending_snapshot(&sid, &queue, &drafts);
                }
                if had_schedule {
                    hub.cancel_draft_timer(&sid, &draft_id);
                    hub.broadcast_sessions();
                }
                if is_owner {
                    hub.notify_human_turn(&sid);
                }
                hub.drain_activated_recovery_row(&sid, &new_row.id);
            } else {
                hub.broadcast_error(
                    Some(sid),
                    "Could not persist recovery draft activation; draft retained".to_owned(),
                );
            }
        });
    }

    /// Commit an acknowledged ordinary recovery draft park + atomic tombstone
    /// setting into Hub memory without issuing a second store write.
    pub fn commit_park_recovery_prompt(&self, preview: &mut PendingRecoveryPark) {
        if !preview.active {
            return;
        }
        preview.active = false;
        let (queue, drafts, setting_update) = {
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
            s.drafts.extend(preview.added_drafts.iter().cloned());
            for cmid in &preview.committed_recovery_cmids {
                s.recovery_cmids.insert(cmid.clone());
                s.committed_recovery_cmids.insert(cmid.clone());
                s.pending_recovery_cmids.remove(cmid);
            }
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
            self.finish_in_flight_pending_commit(
                &preview.session_id,
                &queue,
                &drafts,
                recovery_cmids.clone(),
                &mut claims,
            );
            (queue, drafts, recovery_cmids)
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
    }

    /// Controller-owned durable recovery-park retry queue (Finding 1): survives
    /// `RemoteRuntime` disconnection/replacement and retries the exact same
    /// idempotent durable park until acknowledged by the store or shutdown.
    pub fn enqueue_durable_recovery_park(
        &self,
        session_id: String,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        reason: String,
        on_committed: Option<Arc<dyn Fn() + Send + Sync>>,
    ) {
        if !self.has_store_writer() {
            self.add_recovery_draft(&session_id, text, content, cmid);
            if let Some(cb) = on_committed {
                cb();
            }
            self.broadcast_error(Some(session_id), reason);
            return;
        }
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(&session_id) else {
                return;
            };
            if let Some(ref c) = cmid
                && !c.is_empty()
            {
                s.pending_recovery_cmids.insert(c.clone());
            }
        }
        let start_worker = {
            let mut claims = self.inner.notice_claims.lock();
            let queue = claims
                .pending_recovery_parks
                .entry(session_id.clone())
                .or_default();
            let already_queued = cmid
                .as_deref()
                .is_some_and(|c| queue.iter().any(|item| item.cmid.as_deref() == Some(c)));
            if !already_queued {
                queue.push(PendingRecoveryPromptItem {
                    text,
                    content,
                    cmid,
                    reason,
                    on_committed,
                });
            }
            claims
                .active_recovery_park_workers
                .insert(session_id.clone())
        };
        if !start_worker {
            return;
        }
        let hub = self.clone();
        self.spawn_teardown_requeue(async move {
            hub.run_durable_recovery_park_loop(session_id).await;
        });
    }

    async fn run_durable_recovery_park_loop(&self, session_id: String) {
        let mut backoff_ms = 50_u64;
        loop {
            let next_item = {
                let mut claims = self.inner.notice_claims.lock();
                if let Some(item) = claims
                    .pending_recovery_parks
                    .get(&session_id)
                    .and_then(|q| q.first())
                    .cloned()
                {
                    Some(item)
                } else {
                    claims.pending_recovery_parks.remove(&session_id);
                    claims.active_recovery_park_workers.remove(&session_id);
                    None
                }
            };
            let Some(item) = next_item else {
                break;
            };
            if !self.has_store_writer() {
                self.inner
                    .notice_claims
                    .lock()
                    .active_recovery_park_workers
                    .remove(&session_id);
                break;
            }
            let _commit_guard = self.pending_commit_lock(&session_id).await;
            let outcome = match self.preview_park_recovery_prompt(
                &session_id,
                item.text.clone(),
                item.content.clone(),
                item.cmid.clone(),
            ) {
                RecoveryDraftPark::NoStore | RecoveryDraftPark::Duplicate => Ok(true),
                RecoveryDraftPark::Pending(mut preview, rx) => match rx.await {
                    Ok(Ok(true)) => {
                        self.commit_park_recovery_prompt(&mut preview);
                        Ok(true)
                    }
                    Ok(Ok(false) | Err(_)) => {
                        self.abort_park_recovery_notices(&mut preview);
                        Ok(false)
                    }
                    Err(_) => {
                        self.abort_park_recovery_notices(&mut preview);
                        Err(false)
                    }
                },
                RecoveryDraftPark::Rejected("session unavailable") => Err(true),
                RecoveryDraftPark::Rejected(_) => Ok(false),
            };
            match outcome {
                Ok(true) => {
                    backoff_ms = 50;
                    if let Some(ref c) = item.cmid
                        && let Some(s) = self.inner.sessions.lock().get_mut(&session_id)
                    {
                        s.pending_recovery_cmids.remove(c);
                    }
                    {
                        let mut claims = self.inner.notice_claims.lock();
                        if let Some(q) = claims.pending_recovery_parks.get_mut(&session_id)
                            && !q.is_empty()
                        {
                            q.remove(0);
                        }
                    }
                    if let Some(cb) = item.on_committed {
                        cb();
                    }
                    self.broadcast_error(Some(session_id.clone()), item.reason);
                }
                Ok(false) => {
                    // Finding 5: Bound retries during graceful Controller shutdown
                    // so a persistent store outage can never hang shutdown.
                    if self.inner.teardown_shutdown.load(Ordering::Acquire) {
                        tracing::error!(
                            session = %session_id,
                            "durable recovery park retry aborted during Controller shutdown"
                        );
                        self.inner
                            .notice_claims
                            .lock()
                            .active_recovery_park_workers
                            .remove(&session_id);
                        break;
                    }
                    tokio::select! {
                        () = tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)) => {}
                        () = self.inner.teardown_shutdown_notify.notified() => {}
                    }
                    backoff_ms = (backoff_ms * 2).min(2_000);
                }
                Err(drop_queue) => {
                    let mut claims = self.inner.notice_claims.lock();
                    if drop_queue {
                        claims.pending_recovery_parks.remove(&session_id);
                    }
                    claims.active_recovery_park_workers.remove(&session_id);
                    break;
                }
            }
        }
    }

    /// Apply an already-committed durable recovery park to `Hub`.
    pub fn commit_park_recovery_notices(&self, preview: &mut PendingRecoveryPark) {
        if !preview.active {
            return;
        }
        preview.active = false;
        let (queue, drafts) = {
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
            self.finish_in_flight_pending_commit(
                &preview.session_id,
                &queue,
                &drafts,
                Self::sorted_recovery_cmids(s),
                &mut claims,
            );
            (queue, drafts)
        };
        self.broadcast_pending_snapshot(&preview.session_id, &queue, &drafts);
    }

    /// Abort a durable recovery park without mutating `Hub.drafts`, lifting its
    /// fences and flushing any deferred pending snapshot.
    pub fn abort_park_recovery_notices(&self, preview: &mut PendingRecoveryPark) {
        if !preview.active {
            return;
        }
        preview.active = false;
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
            );
        } else {
            claims.in_flight_pending_commits.remove(&preview.session_id);
        }
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
                if let Some(deferred) = claims.in_flight_pending_commits.get_mut(session_id) {
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
        {
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
                );
            } else {
                claims.in_flight_pending_commits.remove(&preview.session_id);
            }
        }
        self.try_drain(&preview.session_id);
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
        let (changed, queue, drafts) = {
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
            self.finish_in_flight_pending_commit(
                &preview.session_id,
                &queue,
                &drafts,
                Self::sorted_recovery_cmids(s),
                &mut claims,
            );
            (changed, queue, drafts)
        };
        self.broadcast_pending_snapshot(&preview.session_id, &queue, &drafts);
        self.try_drain(&preview.session_id);
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
}
