//! Two-phase acknowledged session-draft Trash archive & restore transfers.

use std::sync::Weak;

use super::*;
use crate::core::hub_managed_calls::StoreAckRx;

/// RAII reservation holding `draft_id` in `activating_recovery_drafts` across
/// async Trash archive + acknowledged queue removal.
pub struct DraftTransferReservation {
    hub: Weak<HubInner>,
    session_id: String,
    draft_id: String,
}

impl Drop for DraftTransferReservation {
    fn drop(&mut self) {
        if let Some(inner) = self.hub.upgrade() {
            let mut sessions = inner.sessions.lock();
            if let Some(s) = sessions.get_mut(&self.session_id) {
                s.activating_recovery_drafts.remove(&self.draft_id);
            }
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

/// Non-mutating two-phase commit guard for durable draft Trash removal or
/// restore under `in_flight_pending_commits`.
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

pub enum DraftRestoreStage {
    AlreadyPresent(String),
    CommittedSync(String),
    Pending(String, PendingDraftTransferCommit, StoreAckRx),
}

impl Hub {
    /// Reserve one authoritative session draft in `activating_recovery_drafts`
    /// after verifying full ordinary session mutability (live, non-trashed,
    /// non-closing, non-system, non-managed-child) so no concurrent edit,
    /// schedule, move, or activation can mutate it during archive transfer.
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

    /// Stage a two-phase acknowledged removal of a reserved session draft under
    /// the per-session `in_flight_pending_commits` fence. Returns `Ok(None)`
    /// only when running without a store writer (memory-only tests).
    pub fn begin_durable_draft_removal(
        &self,
        session_id: &str,
        draft_id: &str,
    ) -> Result<Option<(PendingDraftTransferCommit, StoreAckRx)>, &'static str> {
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
        if !self.has_store_writer() {
            s.drafts.remove(pos);
            s.deferred_scheduled_drafts.retain(|d| d != draft_id);
            if s.editing.as_deref() == Some(draft_id) {
                s.editing = None;
                s.editing_token = None;
            }
            let (q, d) = (s.queue.clone(), s.drafts.clone());
            drop(sessions);
            self.broadcast_pending_snapshot(session_id, &q, &d);
            if had_schedule {
                self.cancel_draft_timer(session_id, draft_id);
                self.broadcast_session(session_id);
            }
            return Ok(None);
        }
        let mut claims = self.inner.notice_claims.lock();
        if claims.in_flight_pending_commits.contains_key(session_id) {
            return Err("Another pending queue update is in progress; try again");
        }
        let mut preview_drafts = s.drafts.clone();
        preview_drafts.remove(pos);
        let Some(rx) = self.send_acked_update_pending(
            session_id,
            s.queue.clone(),
            preview_drafts,
            Vec::new(),
            Vec::new(),
            false,
        ) else {
            return Err("Failed to persist draft removal; try again");
        };
        claims
            .in_flight_pending_commits
            .insert(session_id.to_owned(), Default::default());
        Ok(Some((
            PendingDraftTransferCommit {
                hub: Arc::downgrade(&self.inner),
                active: true,
                session_id: session_id.to_owned(),
                kind: DraftTransferKind::Remove {
                    draft_id: draft_id.to_owned(),
                    had_schedule,
                },
            },
            rx,
        )))
    }

    /// Stage a two-phase acknowledged insert of a restored draft into a live,
    /// mutable session under `in_flight_pending_commits`, preserving original
    /// `DeliveryOrigin` and `recovery-*` classification.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_durable_draft_restore(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: String,
        origin: Option<crate::prompt_origin::DeliveryOrigin>,
        is_recovery: bool,
    ) -> Result<DraftRestoreStage, String> {
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
        if let Some(existing) = s.drafts.iter().find(|m| m.cmid.as_deref() == Some(&cmid)) {
            return Ok(DraftRestoreStage::AlreadyPresent(existing.id.clone()));
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
        if !self.has_store_writer() {
            s.drafts.push(row);
            let (q, d) = (s.queue.clone(), s.drafts.clone());
            drop(sessions);
            self.broadcast_pending_snapshot(session_id, &q, &d);
            return Ok(DraftRestoreStage::CommittedSync(id));
        }
        let mut claims = self.inner.notice_claims.lock();
        if claims.in_flight_pending_commits.contains_key(session_id) {
            return Err("Another pending queue update is in progress; try again".into());
        }
        let mut preview_drafts = s.drafts.clone();
        preview_drafts.push(row.clone());
        let Some(rx) = self.send_acked_update_pending(
            session_id,
            s.queue.clone(),
            preview_drafts,
            Vec::new(),
            Vec::new(),
            false,
        ) else {
            return Err("Failed to persist restored draft; try again".into());
        };
        claims
            .in_flight_pending_commits
            .insert(session_id.to_owned(), Default::default());
        Ok(DraftRestoreStage::Pending(
            id,
            PendingDraftTransferCommit {
                hub: Arc::downgrade(&self.inner),
                active: true,
                session_id: session_id.to_owned(),
                kind: DraftTransferKind::Insert(Box::new(row)),
            },
            rx,
        ))
    }

    pub fn commit_durable_draft_transfer(&self, commit: &mut PendingDraftTransferCommit) {
        if !commit.active {
            return;
        }
        commit.active = false;
        let sid = commit.session_id.clone();
        let (snapshot, cancel_timer) =
            {
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
                            if !s.drafts.iter().any(|m| {
                                m.id == row.id || (row.cmid.is_some() && m.cmid == row.cmid)
                            }) {
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
