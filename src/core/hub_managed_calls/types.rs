//! Data types and RAII preview guards for durable managed-call completion notices.

use super::*;
use crate::store::NoticeClaimRelease;

pub struct PendingRecoveryPark {
    pub(super) hub: std::sync::Weak<HubInner>,
    pub(super) active: bool,
    pub session_id: String,
    pub(super) added_drafts: Vec<QueuedMessage>,
    pub(super) settled_notices: Vec<(String, String)>,
}

impl Drop for PendingRecoveryPark {
    fn drop(&mut self) {
        if self.active
            && let Some(inner) = self.hub.upgrade()
        {
            (Hub { inner }).abort_park_recovery_notices(self);
        }
    }
}

pub struct PendingNoticeWithdrawal {
    pub(super) hub: std::sync::Weak<HubInner>,
    pub(super) active: bool,
    pub session_id: String,
    pub queue: Vec<QueuedMessage>,
    pub drafts: Vec<QueuedMessage>,
    pub settled_notices: Vec<(String, String)>,
    pub(super) clear_ids: HashSet<String>,
    pub(super) fenced_row_ids: Vec<String>,
}

impl Drop for PendingNoticeWithdrawal {
    fn drop(&mut self) {
        if self.active
            && let Some(inner) = self.hub.upgrade()
        {
            (Hub { inner }).abort_owner_notice_withdrawal(self);
        }
    }
}

/// Parameters for a two-phase conditional non-delivery requeue of a `call-notice:`.
#[derive(Debug, Clone)]
pub struct NoticeRequeueRequest {
    pub session_id: String,
    pub text: String,
    pub content: Vec<serde_json::Value>,
    pub cmid: String,
    pub expected_command_id: Option<String>,
    pub expected_worker_epoch: Option<String>,
    pub require_worker_epoch_match: bool,
    pub receipt_key: String,
    pub require_claim_release: bool,
}

pub struct PendingUndeliveredRequeue {
    pub(super) hub: std::sync::Weak<HubInner>,
    pub(super) active: bool,
    pub session_id: String,
    pub cmid: String,
    pub(super) added_message: Option<QueuedMessage>,
    pub(super) expected_stamp: Option<i64>,
    pub(super) expected_command_id: Option<String>,
    pub(super) expected_in_flight_epoch: Option<u64>,
    pub(super) receipt_key: String,
}

impl Drop for PendingUndeliveredRequeue {
    fn drop(&mut self) {
        if self.active
            && let Some(inner) = self.hub.upgrade()
        {
            (Hub { inner }).abort_undelivered_notice_requeue(self, false);
        }
    }
}

/// Result of atomically checking for queued completion notices and applying or
/// staging an owner queue mutation under a single `sessions` lock.
pub enum OwnerQueueMutation {
    /// No queued completion notice was affected; ordinary mutation applied synchronously.
    AppliedSync(bool),
    /// One or more queued completion notices are affected; await `StoreAckRx` and commit/abort.
    CommitPending(Box<PendingNoticeWithdrawal>, StoreAckRx),
    /// A queued completion notice was affected but durable store admission failed.
    Rejected(&'static str),
}

pub enum OwnerNoticeWithdrawalOp<'a> {
    RemoveQueued {
        id: &'a str,
    },
    EditQueued {
        id: &'a str,
        text: &'a str,
        content: &'a [serde_json::Value],
    },
    ClearQueue,
    QueuedToDraft {
        id: &'a str,
    },
    CancelSubmitted {
        cmid: &'a str,
    },
}

pub(super) fn apply_withdrawal_to_lists(
    queue: &mut Vec<QueuedMessage>,
    drafts: &mut Vec<QueuedMessage>,
    editing: &mut Option<String>,
    op: &OwnerNoticeWithdrawalOp<'_>,
    clear_ids: &HashSet<String>,
) -> (bool, Vec<(String, String)>) {
    match op {
        OwnerNoticeWithdrawalOp::RemoveQueued { id } => {
            let settled = notice_settlements(queue.iter().filter(|m| m.id == *id));
            let before = queue.len();
            queue.retain(|m| m.id != *id);
            if editing.as_deref() == Some(*id) {
                *editing = None;
            }
            (before != queue.len(), settled)
        }
        OwnerNoticeWithdrawalOp::EditQueued { id, text, content } => {
            if text.trim().is_empty() && content.is_empty() {
                let settled = notice_settlements(queue.iter().filter(|m| m.id == *id));
                let before = queue.len();
                queue.retain(|m| m.id != *id);
                if editing.as_deref() == Some(*id) {
                    *editing = None;
                }
                (before != queue.len(), settled)
            } else if let Some(m) = queue.iter_mut().find(|m| m.id == *id) {
                let settled = notice_settlements(std::iter::once(&*m));
                if !settled.is_empty() {
                    m.cmid = None;
                }
                m.text = (*text).to_owned();
                m.content = content.to_vec();
                (true, settled)
            } else {
                (false, Vec::new())
            }
        }
        OwnerNoticeWithdrawalOp::ClearQueue => {
            let settled = notice_settlements(
                queue
                    .iter()
                    .filter(|m| clear_ids.is_empty() || clear_ids.contains(&m.id)),
            );
            let before = queue.len();
            if clear_ids.is_empty() {
                queue.clear();
                *editing = None;
            } else {
                queue.retain(|m| !clear_ids.contains(&m.id));
                if editing.as_deref().is_some_and(|id| clear_ids.contains(id)) {
                    *editing = None;
                }
            }
            (before != queue.len(), settled)
        }
        OwnerNoticeWithdrawalOp::QueuedToDraft { id } => {
            if let Some(pos) = queue.iter().position(|m| m.id == *id) {
                let mut m = queue.remove(pos);
                let settled = notice_settlements(std::iter::once(&m));
                if !settled.is_empty() {
                    m.cmid = None;
                }
                if editing.as_deref() == Some(*id) {
                    *editing = None;
                }
                drafts.push(m);
                (true, settled)
            } else {
                (false, Vec::new())
            }
        }
        OwnerNoticeWithdrawalOp::CancelSubmitted { cmid } => {
            let settled =
                notice_settlements(queue.iter().filter(|m| m.cmid.as_deref() == Some(*cmid)));
            let before = queue.len();
            queue.retain(|m| m.cmid.as_deref() != Some(*cmid));
            (before != queue.len(), settled)
        }
    }
}

pub(super) fn affected_row_ids(
    queue: &[QueuedMessage],
    op: &OwnerNoticeWithdrawalOp<'_>,
    clear_ids: &HashSet<String>,
) -> Vec<String> {
    match op {
        OwnerNoticeWithdrawalOp::RemoveQueued { id }
        | OwnerNoticeWithdrawalOp::EditQueued { id, .. }
        | OwnerNoticeWithdrawalOp::QueuedToDraft { id } => vec![(*id).to_owned()],
        OwnerNoticeWithdrawalOp::ClearQueue => clear_ids.iter().cloned().collect(),
        OwnerNoticeWithdrawalOp::CancelSubmitted { cmid } => queue
            .iter()
            .filter(|m| m.cmid.as_deref() == Some(*cmid))
            .map(|m| m.id.clone())
            .collect(),
    }
}

pub(super) fn notice_settlements<'a>(
    items: impl IntoIterator<Item = &'a QueuedMessage>,
) -> Vec<(String, String)> {
    items
        .into_iter()
        .filter_map(|m| m.cmid.as_deref())
        .filter(|c| c.starts_with(crate::agent_model::CALL_NOTICE_PREFIX))
        .map(|c| (c.to_owned(), "owner_withdrawn".to_owned()))
        .collect()
}

/// RAII fence held while a current-lifetime SQL dispatch claim is being acquired.
pub struct NoticeClaimGuard {
    pub(super) hub: Hub,
    pub(super) key: (String, String),
}

impl Drop for NoticeClaimGuard {
    fn drop(&mut self) {
        self.hub
            .inner
            .notice_claims
            .lock()
            .claiming
            .remove(&self.key);
    }
}

/// Registry of in-flight FIFO `StoreWrite::UpdatePending` acknowledgements.
#[derive(Clone, Default)]
pub struct StoreAckSink {
    inner: Arc<StoreAckInner>,
}

#[derive(Default)]
struct StoreAckInner {
    next_id: AtomicU64,
    waiters: Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Result<bool, String>>>>,
}

impl StoreAckSink {
    pub fn allocate(&self) -> (u64, StoreAckRx) {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.inner.waiters.lock().insert(id, tx);
        (id, rx)
    }

    pub fn complete(&self, id: u64, result: Result<bool, String>) {
        if let Some(tx) = self.inner.waiters.lock().remove(&id) {
            let _ = tx.send(result);
        }
    }

    pub fn cancel(&self, id: u64) {
        self.inner.waiters.lock().remove(&id);
    }

    pub fn fail_all(&self, reason: &str) {
        let waiters = std::mem::take(&mut *self.inner.waiters.lock());
        for (_, tx) in waiters {
            let _ = tx.send(Err(reason.to_owned()));
        }
    }
}

/// Result of attempting to durably requeue an authoritative worker
/// `UndeliveredPrompt` or provably unsent teardown notice through the FIFO store writer.
pub enum UndeliveredRequeue {
    /// No store writer is attached (in-memory test mode); Hub already updated.
    NoStore,
    /// Enqueued in the FIFO store writer; await `StoreAckRx` before committing
    /// or aborting Hub state and before ACKing the worker.
    Pending(Box<PendingUndeliveredRequeue>, StoreAckRx),
    /// Store queue was full or closed; Hub memory was not mutated and the worker
    /// must NOT receive `Frame::Ack`.
    Rejected(&'static str),
}

#[derive(Default)]
pub(super) struct DeferredPendingFlush {
    pub(super) dirty: bool,
    pub(super) settled_notices: Vec<(String, String)>,
    pub(super) released_claims: Vec<NoticeClaimRelease>,
}

#[derive(Default)]
pub(in crate::core) struct NoticeClaims {
    pub(super) active: HashMap<(String, String), (i64, String)>,
    pub(super) this_lifetime_stamps: HashSet<(String, String, i64)>,
    pub(super) ambiguous_claims: HashMap<(String, String), (String, i64)>,
    pub(super) pending_releases: HashMap<(String, String), (i64, String)>,
    pub(super) withdrawing: HashSet<(String, String)>,
    pub(super) withdrawing_row_ids: HashSet<(String, String)>,
    pub(super) claiming: HashSet<(String, String)>,
    pub(super) seen_undelivered_keys: HashSet<(String, String, String)>,
    pub(super) result_relay_reservations: HashMap<(String, String), usize>,
    pub(super) pending_result_consumptions: HashMap<(String, String), String>,
    pub(super) in_flight_pending_commits: HashMap<String, DeferredPendingFlush>,
    pub(super) commit_locks: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
    pub(super) teardown_tasks: Vec<tokio::task::JoinHandle<()>>,
}
