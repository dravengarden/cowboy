//! Owner queue withdrawals, recovery parking, and queue inspection for
//! durable managed-call completion notices.

use super::types::{DeferredPendingFlush, affected_row_ids, apply_withdrawal_to_lists};
use super::*;

impl Hub {
    /// Preview parking one or more unacknowledged `call-notice:` prompts as
    /// plain drafts (`cmid: None`) with `execution_recovery_parked`, fence the
    /// session's pending snapshots, and enqueue the atomic FIFO
    /// `StoreWrite::UpdatePending` UNDER `sessions.lock()`.
    pub fn preview_park_recovery_notices(
        &self,
        session_id: &str,
        prompts: &[(String, Vec<serde_json::Value>, String)],
    ) -> Option<(PendingRecoveryPark, StoreAckRx)> {
        if prompts.is_empty() {
            return None;
        }
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        let queue = s.queue.clone();
        let mut drafts = s.drafts.clone();
        let mut added_drafts = Vec::with_capacity(prompts.len());
        let mut settled_notices = Vec::with_capacity(prompts.len());
        for (text, content, cid) in prompts {
            let draft = QueuedMessage {
                id: self.next_qid(),
                text: text.clone(),
                content: content.clone(),
                cmid: None,
                schedule: None,
            };
            drafts.push(draft.clone());
            added_drafts.push(draft);
            settled_notices.push((cid.clone(), "execution_recovery_parked".to_owned()));
        }
        let rx = self.send_acked_update_pending(
            session_id,
            queue,
            drafts,
            settled_notices.clone(),
            Vec::new(),
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
                settled_notices,
            },
            rx,
        ))
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
            self.finish_in_flight_pending_commit(&preview.session_id, &queue, &drafts, &mut claims);
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
        if let Some(s) = sessions.get(&preview.session_id) {
            self.finish_in_flight_pending_commit(
                &preview.session_id,
                &s.queue,
                &s.drafts,
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
                        require_claim_release: false,
                        ack_id: None,
                    });
                }
                (queue, drafts)
            } else {
                let Some(rx) = self.send_acked_update_pending(
                    session_id,
                    queue.clone(),
                    drafts.clone(),
                    settled_notices.clone(),
                    Vec::new(),
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
        let Some(rx) = self.send_acked_update_pending(
            session_id,
            queue.clone(),
            drafts.clone(),
            settled_notices.clone(),
            Vec::new(),
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
            self.finish_in_flight_pending_commit(&preview.session_id, &queue, &drafts, &mut claims);
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
