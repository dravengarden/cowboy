//! Single-transaction atomic session-draft Trash archive & restore staging.
//!
//! Both removal and restore hold `in_flight_pending_commits` (preventing any
//! concurrent `StoreWrite::UpdatePending` for the session) and
//! `activating_recovery_drafts` (preventing concurrent edits, schedules,
//! sends, or steers) while a request-independent task flushes the writer
//! barrier, executes ONE atomic SQL transaction across `sessions` and
//! `draft_documents`, and then commits or aborts Hub memory deterministically.

use std::sync::Weak;

use super::*;

/// RAII reservation holding `draft_id` in `activating_recovery_drafts` across
/// an atomic Trash archive transfer. If a scheduled draft's fire timer elapsed
/// while reserved and the transfer aborted (leaving the draft in place), Drop
/// re-triggers `fire_scheduled_draft` so the overdue draft is not stranded.
pub struct DraftTransferReservation {
    hub: Weak<HubInner>,
    session_id: String,
    draft_id: String,
}

impl Drop for DraftTransferReservation {
    fn drop(&mut self) {
        let Some(inner) = self.hub.upgrade() else {
            return;
        };
        let should_refire = {
            let mut sessions = inner.sessions.lock();
            if let Some(s) = sessions.get_mut(&self.session_id) {
                s.activating_recovery_drafts.remove(&self.draft_id);
                s.deferred_scheduled_drafts
                    .iter()
                    .any(|id| id == &self.draft_id)
                    && s.drafts.iter().any(|m| m.id == self.draft_id)
            } else {
                false
            }
        };
        if should_refire {
            let hub = Hub { inner };
            hub.fire_scheduled_draft(&self.session_id, &self.draft_id);
        }
    }
}

enum DraftTransferKind {
    Remove {
        draft_id: String,
        had_schedule: bool,
    },
    Insert(Box<QueuedMessage>),
}

/// Non-mutating commit guard holding `in_flight_pending_commits` for one
/// session across a single-transaction atomic archive or restore.
pub struct PendingDraftTransferCommit {
    hub: Weak<HubInner>,
    active: bool,
    session_id: String,
    kind: DraftTransferKind,
}

impl Drop for PendingDraftTransferCommit {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        if let Some(inner) = self.hub.upgrade() {
            let hub = Hub { inner };
            let sid = self.session_id.clone();
            let mut sessions = hub.inner.sessions.lock();
            let mut claims = hub.inner.notice_claims.lock();
            if let Some(s) = sessions.get_mut(&sid) {
                hub.finish_in_flight_pending_commit(&sid, &s.queue, &s.drafts, &mut claims);
            } else {
                claims.in_flight_pending_commits.remove(&sid);
            }
        }
    }
}

impl Hub {
    /// Reserve one authoritative session draft in `activating_recovery_drafts`
    /// after verifying full ordinary session mutability (live, non-trashed,
    /// non-closing, non-system, non-managed-child) so no concurrent edit,
    /// schedule, move, send, or steer can mutate or dispatch it during transfer.
    pub fn reserve_draft_for_transfer(
        &self,
        session_id: &str,
        identity: &str,
    ) -> Result<Option<(QueuedMessage, DraftTransferReservation)>, &'static str> {
        if self.is_managed_child(session_id) {
            return Err("Managed child sessions cannot modify drafts");
        }
        let mut sessions = self.inner.sessions.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return Ok(None);
        };
        if session.meta.closing {
            return Err("Session is being deleted");
        }
        if session.meta.trashed_at_ms.is_some() {
            return Err("This session is in Trash. Restore it to continue.");
        }
        if session.meta.system {
            return Err("System sessions cannot modify drafts");
        }
        let Some(row) = session
            .drafts
            .iter()
            .find(|row| row.id == identity || row.cmid.as_deref() == Some(identity))
            .cloned()
        else {
            return Ok(None);
        };
        if session.activating_recovery_drafts.contains(&row.id)
            || session
                .steer_reservations
                .values()
                .any(|r| r.row_id == row.id)
        {
            return Err("This draft is currently sending or moving; try again");
        }
        session.activating_recovery_drafts.insert(row.id.clone());
        Ok(Some((
            row.clone(),
            DraftTransferReservation {
                hub: Arc::downgrade(&self.inner),
                session_id: session_id.to_owned(),
                draft_id: row.id,
            },
        )))
    }

    /// Install `in_flight_pending_commits` and build the post-removal
    /// `(queue, drafts)` preview for an atomic SQL transaction without mutating
    /// Hub memory or enqueueing a separate write-behind `UpdatePending`.
    pub fn stage_atomic_draft_removal(
        &self,
        session_id: &str,
        draft_id: &str,
    ) -> Result<
        (
            Vec<QueuedMessage>,
            Vec<QueuedMessage>,
            PendingDraftTransferCommit,
        ),
        &'static str,
    > {
        let mut sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get_mut(session_id) else {
            return Err("Session no longer exists");
        };
        if s.meta.closing || s.meta.trashed_at_ms.is_some() || s.meta.system {
            return Err("Session is not mutable");
        }
        let Some(pos) = s.drafts.iter().position(|m| m.id == draft_id) else {
            return Err("Draft no longer exists");
        };
        let had_schedule = s.drafts[pos].schedule.is_some();
        let mut claims = self.inner.notice_claims.lock();
        if claims.in_flight_pending_commits.contains_key(session_id) {
            return Err("Another pending queue update is in progress; try again");
        }
        let preview_queue = s.queue.clone();
        let mut preview_drafts = s.drafts.clone();
        preview_drafts.remove(pos);
        claims
            .in_flight_pending_commits
            .insert(session_id.to_owned(), Default::default());
        Ok((
            preview_queue,
            preview_drafts,
            PendingDraftTransferCommit {
                hub: Arc::downgrade(&self.inner),
                active: true,
                session_id: session_id.to_owned(),
                kind: DraftTransferKind::Remove {
                    draft_id: draft_id.to_owned(),
                    had_schedule,
                },
            },
        ))
    }

    /// Install `in_flight_pending_commits` and build the post-restore
    /// `(row_id, queue, drafts)` preview for an atomic SQL transaction without
    /// mutating Hub memory. Preserves `origin` verbatim (including `None`) and
    /// `recovery-*` classification (R3-4, R3-14, R3-17).
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub fn stage_atomic_draft_restore(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: String,
        origin: Option<crate::prompt_origin::DeliveryOrigin>,
        is_recovery: bool,
    ) -> Result<
        (
            String,
            Vec<QueuedMessage>,
            Vec<QueuedMessage>,
            PendingDraftTransferCommit,
        ),
        String,
    > {
        if self.reject_reserved_notice_cmid(session_id, Some(&cmid)) {
            return Err("Invalid restore operation id".into());
        }
        if self.is_managed_child(session_id) {
            return Err("Managed child sessions cannot receive restored drafts".into());
        }
        let mut sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get_mut(session_id) else {
            return Err("Source session no longer exists".into());
        };
        if s.meta.closing {
            return Err("Source session is being deleted".into());
        }
        if s.meta.trashed_at_ms.is_some() {
            return Err("Source session is in Trash. Restore it first.".into());
        }
        if s.meta.system {
            return Err("System sessions cannot receive restored drafts".into());
        }
        let mut claims = self.inner.notice_claims.lock();
        if claims.in_flight_pending_commits.contains_key(session_id) {
            return Err("Another pending queue update is in progress; try again".into());
        }
        let id = if is_recovery {
            format!("recovery-{}", uuid::Uuid::new_v4().simple())
        } else {
            format!("draft-{}", uuid::Uuid::new_v4().simple())
        };
        let row = QueuedMessage {
            id: id.clone(),
            text,
            content,
            cmid: Some(cmid),
            schedule: None,
            origin,
            initiator: None,
            explicit: None,
        };
        let preview_queue = s.queue.clone();
        let mut preview_drafts = s.drafts.clone();
        preview_drafts.push(row.clone());
        claims
            .in_flight_pending_commits
            .insert(session_id.to_owned(), Default::default());
        Ok((
            id,
            preview_queue,
            preview_drafts,
            PendingDraftTransferCommit {
                hub: Arc::downgrade(&self.inner),
                active: true,
                session_id: session_id.to_owned(),
                kind: DraftTransferKind::Insert(Box::new(row)),
            },
        ))
    }

    /// Snapshot the current post-barrier `(queue, drafts)` under the held
    /// `in_flight_pending_commits` fence right before entering the atomic SQL tx.
    pub fn snapshot_for_staged_transfer(
        &self,
        commit: &PendingDraftTransferCommit,
    ) -> Result<(Vec<QueuedMessage>, Vec<QueuedMessage>), &'static str> {
        let sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get(&commit.session_id) else {
            return Err("Session no longer exists");
        };
        if s.meta.closing || s.meta.trashed_at_ms.is_some() || s.meta.system {
            return Err("Session is no longer mutable");
        }
        let queue = s.queue.clone();
        let mut drafts = s.drafts.clone();
        match &commit.kind {
            DraftTransferKind::Remove { draft_id, .. } => {
                let Some(pos) = drafts.iter().position(|m| m.id == *draft_id) else {
                    return Err("Draft no longer exists");
                };
                drafts.remove(pos);
            }
            DraftTransferKind::Insert(row) => {
                if !drafts.iter().any(|m| m.id == row.id) {
                    drafts.push((**row).clone());
                }
            }
        }
        Ok((queue, drafts))
    }

    pub fn commit_durable_draft_transfer(&self, commit: &mut PendingDraftTransferCommit) {
        if !commit.active {
            return;
        }
        commit.active = false;
        let sid = commit.session_id.clone();
        let (snapshot, cancel_timer) = {
            let mut sessions = self.inner.sessions.lock();
            let mut claims = self.inner.notice_claims.lock();
            if let Some(s) = sessions.get_mut(&sid) {
                let mut cancel_timer = None;
                match &commit.kind {
                    DraftTransferKind::Remove {
                        draft_id,
                        had_schedule,
                    } => {
                        s.drafts.retain(|m| m.id != *draft_id);
                        s.deferred_scheduled_drafts.retain(|d| d != draft_id);
                        if s.editing.as_deref() == Some(draft_id.as_str()) {
                            s.editing = None;
                            s.editing_token = None;
                        }
                        if *had_schedule {
                            cancel_timer = Some(draft_id.clone());
                        }
                    }
                    DraftTransferKind::Insert(row) => {
                        if !s.drafts.iter().any(|m| m.id == row.id) {
                            s.drafts.push((**row).clone());
                        }
                    }
                }
                let snap = (s.queue.clone(), s.drafts.clone());
                self.finish_in_flight_pending_commit(&sid, &snap.0, &snap.1, &mut claims);
                (Some(snap), cancel_timer)
            } else {
                claims.in_flight_pending_commits.remove(&sid);
                (None, None)
            }
        };
        if let Some((q, d)) = snapshot {
            self.broadcast_pending_snapshot(&sid, &q, &d);
            if let Some(draft_id) = cancel_timer {
                self.cancel_draft_timer(&sid, &draft_id);
                self.broadcast_session(&sid);
            }
        }
    }

    pub fn abort_durable_draft_transfer(&self, commit: &mut PendingDraftTransferCommit) {
        if !commit.active {
            return;
        }
        commit.active = false;
        let sid = commit.session_id.clone();
        let mut sessions = self.inner.sessions.lock();
        let mut claims = self.inner.notice_claims.lock();
        if let Some(s) = sessions.get_mut(&sid) {
            self.finish_in_flight_pending_commit(&sid, &s.queue, &s.drafts, &mut claims);
        } else {
            claims.in_flight_pending_commits.remove(&sid);
        }
    }
}
