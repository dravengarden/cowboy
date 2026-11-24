//! Managed call children: Controller-owned prompts.

use super::*;
use crate::store::NoticeClaimRelease;

mod types;
mod withdrawals;
use types::DeferredPendingFlush;
pub(in crate::core) use types::NoticeClaims;
pub use types::{
    NoticeClaimGuard, NoticeRequeueRequest, OwnerNoticeWithdrawalOp, OwnerQueueMutation,
    PendingNoticeWithdrawal, PendingRecoveryPark, PendingUndeliveredRequeue, RecoveryParkPreview,
    StoreAckSink, UndeliveredRequeue,
};

/// Acknowledged FIFO `StoreWrite::UpdatePending` result (`Ok(true)` = applied,
/// `Ok(false)` = conditional claim-release skipped as duplicate/settled).
pub type StoreAckRx = tokio::sync::oneshot::Receiver<Result<bool, String>>;

/// Acknowledged durable owner-withdrawal preview + FIFO commit handle.
pub type PendingNoticeWithdrawalCommit = (PendingNoticeWithdrawal, StoreAckRx);

/// What [`Hub::newest_in_retained_log`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetainedLogScan {
    pub newest_match: Option<u64>,
    pub first_seq: Option<u64>,
    /// Nothing older exists: the retained log is the session's whole history.
    pub whole: bool,
}

impl Hub {
    /// Track a runtime-teardown notice requeue task so graceful shutdown drains
    /// it before closing persistence (Finding 6).
    pub fn spawn_teardown_requeue(
        &self,
        fut: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let join = handle.spawn(fut);
        let mut claims = self.inner.notice_claims.lock();
        claims.teardown_tasks.retain(|h| !h.is_finished());
        claims.teardown_tasks.push(join);
    }

    /// Await all in-flight runtime-teardown notice requeue tasks before closing
    /// persistence. Logs a warning after `warn_after` and keeps persistence
    /// alive until the bounded retries in each admitted task finish (matching
    /// `dispatcher_task` shutdown behavior).
    pub async fn drain_teardown_requeues(&self, warn_after: std::time::Duration) {
        self.retry_pending_recovery_parks(None);
        let mut warned = false;
        let warn_at = tokio::time::Instant::now() + warn_after;
        loop {
            let handles = {
                let mut claims = self.inner.notice_claims.lock();
                std::mem::take(&mut claims.teardown_tasks)
            };
            if handles.is_empty() {
                return;
            }
            for mut handle in handles {
                if !warned {
                    let remaining = warn_at.saturating_duration_since(tokio::time::Instant::now());
                    match tokio::time::timeout(remaining, &mut handle).await {
                        Ok(Ok(())) => continue,
                        Ok(Err(error)) => {
                            tracing::error!(
                                %error,
                                "runtime-teardown notice requeue task failed during shutdown"
                            );
                            continue;
                        }
                        Err(_) => {
                            warned = true;
                            tracing::warn!(
                                "runtime-teardown notice requeues still committing; keeping persistence alive"
                            );
                        }
                    }
                }
                if let Err(error) = handle.await {
                    tracing::error!(
                        %error,
                        "runtime-teardown notice requeue task failed while finishing shutdown"
                    );
                }
            }
        }
    }

    /// Acquire the per-session async mutex that serializes acknowledged durable
    /// pending commits (owner withdrawals, recovery parks, undelivered notice
    /// requeues) and startup reconciliation queue removals.
    pub async fn pending_commit_lock(&self, session_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut claims = self.inner.notice_claims.lock();
            Arc::clone(
                claims
                    .commit_locks
                    .entry(session_id.to_owned())
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        lock.lock_owned().await
    }

    pub(super) fn emit_pending_with_notice_ops(
        &self,
        session_id: &str,
        settled_notices: Vec<(String, String)>,
        released_claims: Vec<NoticeClaimRelease>,
    ) {
        let (queue, drafts) = {
            let sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get(session_id) else {
                return;
            };
            let queue = s.queue.clone();
            let drafts = s.drafts.clone();
            let mut claims = self.inner.notice_claims.lock();
            if let Some(deferred) = claims.in_flight_pending_commits.get_mut(session_id) {
                // An acknowledged pending commit is in flight for this session.
                // Defer the snapshot until that commit finishes so a pre-commit
                // Hub view can NEVER overwrite a committed edit, parked draft,
                // or requeued notice in the FIFO store channel.
                deferred.dirty = true;
                deferred.settled_notices.extend(settled_notices);
                deferred.released_claims.extend(released_claims);
            } else if let Some(tx) = self.inner.store_tx.as_ref() {
                let _ = tx.send(StoreWrite::UpdatePending {
                    session_id: session_id.to_owned(),
                    queue: queue.clone(),
                    drafts: drafts.clone(),
                    settled_notices,
                    released_claims,
                    require_claim_release: false,
                    ack_id: None,
                });
            }
            (queue, drafts)
        };
        self.broadcast_pending_snapshot(session_id, &queue, &drafts);
    }

    fn finish_in_flight_pending_commit(
        &self,
        session_id: &str,
        queue: &[QueuedMessage],
        drafts: &[QueuedMessage],
        claims: &mut NoticeClaims,
    ) {
        let Some(deferred) = claims.in_flight_pending_commits.remove(session_id) else {
            return;
        };
        if (deferred.dirty
            || !deferred.settled_notices.is_empty()
            || !deferred.released_claims.is_empty())
            && let Some(tx) = self.inner.store_tx.as_ref()
        {
            let _ = tx.send(StoreWrite::UpdatePending {
                session_id: session_id.to_owned(),
                queue: queue.to_vec(),
                drafts: drafts.to_vec(),
                settled_notices: deferred.settled_notices,
                released_claims: deferred.released_claims,
                require_claim_release: false,
                ack_id: None,
            });
        }
    }

    pub(super) fn broadcast_pending_snapshot(
        &self,
        session_id: &str,
        queue: &[QueuedMessage],
        drafts: &[QueuedMessage],
    ) {
        // Broadcast on the generic optimistic-sync channel as state "queue:<sid>".
        // Confirmed = the cmids in the value, so a client drops its optimistic add
        // the moment its cmid lands here (the cross-terminal reconcile).
        let confirmed = Self::cmids_of(queue, drafts);
        let value = serde_json::json!({ "queue": queue, "drafts": drafts });
        self.sync_emit(&format!("queue:{session_id}"), value, confirmed);
    }

    /// Submit a managed call's prompt to its child. Client entry points refuse
    /// managed children; only the Controller-owned call runner reaches here.
    pub fn submit_managed_prompt(&self, session_id: &str, call_id: &str, text: String) {
        let cmid = format!("{MANAGED_CALL_CMID_PREFIX}{call_id}");
        let origin = crate::prompt_origin::DeliveryOrigin::cowboy("managed_call");
        self.submit_inner(
            session_id,
            text,
            Vec::new(),
            Some(cmid),
            true,
            false,
            Some(origin.clone()),
            origin,
        );
    }

    /// What `find` makes of a session's retained log (an event's sequence), with the
    /// log's first sequence and whether it is the whole history. The retained
    /// log holds every event not yet trimmed, written to the store or not,
    /// whatever a client bootstrap shows of it. A turn the Hub saw begin
    /// (`owner_turn_seq`) counts even after its event left the log.
    pub fn newest_in_retained_log(
        &self,
        session_id: &str,
        find: impl Fn(&[Envelope]) -> Option<u64>,
    ) -> Option<RetainedLogScan> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        let in_log = find(&s.log);
        let first_seq = s.log.first().map(|e| e.seq);
        // A match at the log's first event may be a send whose first chunks
        // were trimmed: the turn the Hub saw begin is that send's start, and
        // one it never saw begin (restored from a cut tail) is not known here.
        let cut = !s.reached_start && in_log.is_some() && in_log == first_seq;
        Some(RetainedLogScan {
            newest_match: if cut {
                s.owner_turn_seq.filter(|seq| Some(*seq) <= in_log)
            } else {
                in_log.max(s.owner_turn_seq)
            },
            first_seq,
            whole: s.reached_start,
        })
    }

    /// Remember an owner turn found in durable history (older than the
    /// retained log), unless the Hub already knows one at least as new, so a
    /// cleared transcript keeps it.
    pub fn remember_owner_turn(&self, session_id: &str, seq: u64) {
        if let Some(s) = self.inner.sessions.lock().get_mut(session_id)
            && s.owner_turn_seq.is_none_or(|known| known < seq)
        {
            s.owner_turn_seq = Some(seq);
        }
    }

    /// Withdraw a call's still-queued prompt (cancelled, refused or finished).
    /// Returns whether a queued prompt was withdrawn (so it never started).
    pub fn discard_managed_prompt(&self, session_id: &str, call_id: &str) -> bool {
        let cmid = format!("{MANAGED_CALL_CMID_PREFIX}{call_id}");
        let removed = {
            let mut sessions = self.inner.sessions.lock();
            sessions.get_mut(session_id).is_some_and(|s| {
                let before = s.queue.len();
                s.queue.retain(|m| m.cmid.as_deref() != Some(cmid.as_str()));
                before != s.queue.len()
            })
        };
        if removed {
            self.emit_pending(session_id);
        }
        removed
    }

    /// A client's queue edit. A managed call's queued prompt is its durable
    /// request: an edit would run other instructions under its (reusable)
    /// receipt, so a managed child's queue is never edited by a client.
    pub fn client_edit(
        &self,
        session_id: &str,
        id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        caller: &crate::prompt_origin::RequestCaller,
    ) -> bool {
        !self.is_managed_child(session_id)
            && self.edit_queued(session_id, id, text, content, caller)
    }

    /// Queued managed prompts as `(child, parent, call)`. Withdrawal is
    /// persisted best-effort, so a restart can restore a row whose call
    /// already ended; recovery removes those before they can be dispatched.
    pub fn queued_managed_calls(&self) -> Vec<(String, String, String)> {
        let sessions = self.inner.sessions.lock();
        let mut queued = Vec::new();
        for (id, s) in sessions.iter() {
            let Some(parent) = s
                .meta
                .execution_binding
                .as_ref()
                .and_then(|binding| binding.managed())
                .map(|managed| managed.parent_session_id)
            else {
                continue;
            };
            for row in &s.queue {
                if let Some(call) = row
                    .cmid
                    .as_deref()
                    .and_then(|c| c.strip_prefix(MANAGED_CALL_CMID_PREFIX))
                {
                    queued.push((id.clone(), parent.clone(), call.to_owned()));
                }
            }
        }
        queued
    }

    pub fn is_managed_child(&self, session_id: &str) -> bool {
        self.inner.sessions.lock().get(session_id).is_some_and(|s| {
            s.meta
                .execution_binding
                .as_ref()
                .is_some_and(|binding| binding.managed().is_some())
        })
    }

    /// Record the active durable dispatch `(stamp, command_id)` for a `call-notice:` handoff.
    pub fn record_notice_claim(
        &self,
        session_id: &str,
        completion_id: &str,
        stamp: i64,
        command_id: String,
    ) {
        let mut claims = self.inner.notice_claims.lock();
        let key = (session_id.to_owned(), completion_id.to_owned());
        claims.ambiguous_claims.remove(&key);
        claims.active.insert(key, (stamp, command_id));
        claims.this_lifetime_stamps.insert((
            session_id.to_owned(),
            completion_id.to_owned(),
            stamp,
        ));
    }

    /// Atomically record the active durable dispatch `(stamp, command_id)` only
    /// if no `calls.result` relay reservation, pending result consumption, or
    /// owner withdrawal took ownership under `notice_claims`.
    #[must_use]
    pub fn try_record_notice_claim(
        &self,
        session_id: &str,
        completion_id: &str,
        stamp: i64,
        command_id: String,
    ) -> bool {
        let mut claims = self.inner.notice_claims.lock();
        let key = (session_id.to_owned(), completion_id.to_owned());
        if claims
            .result_relay_reservations
            .get(&key)
            .is_some_and(|count| *count > 0)
            || claims.pending_result_consumptions.contains_key(&key)
            || claims.withdrawing.contains(&key)
        {
            return false;
        }
        claims.ambiguous_claims.remove(&key);
        claims.active.insert(key, (stamp, command_id));
        claims.this_lifetime_stamps.insert((
            session_id.to_owned(),
            completion_id.to_owned(),
            stamp,
        ));
        true
    }

    /// Record a dispatch attempt whose SQL `begin_call_notice_dispatch` response
    /// was lost/ambiguous so the next dispatch attempt can read back ownership
    /// instead of dropping the queued copy as `AlreadyHandled` (Finding 5).
    pub fn record_ambiguous_notice_claim(
        &self,
        session_id: &str,
        completion_id: &str,
        command_id: String,
        requested_stamp: i64,
    ) {
        self.inner.notice_claims.lock().ambiguous_claims.insert(
            (session_id.to_owned(), completion_id.to_owned()),
            (command_id, requested_stamp),
        );
    }

    /// Inspect an ambiguous prior dispatch attempt for `(session_id, completion_id)`.
    #[must_use]
    pub fn peek_ambiguous_notice_claim(
        &self,
        session_id: &str,
        completion_id: &str,
    ) -> Option<(String, i64)> {
        self.inner
            .notice_claims
            .lock()
            .ambiguous_claims
            .get(&(session_id.to_owned(), completion_id.to_owned()))
            .cloned()
    }

    /// Clear an ambiguous prior dispatch attempt once resolved against durable storage.
    pub fn clear_ambiguous_notice_claim(&self, session_id: &str, completion_id: &str) {
        self.inner
            .notice_claims
            .lock()
            .ambiguous_claims
            .remove(&(session_id.to_owned(), completion_id.to_owned()));
    }

    /// Whether this notice has an unresolved ambiguous dispatch claim from this lifetime.
    #[must_use]
    pub fn has_ambiguous_notice_claim(&self, session_id: &str, completion_id: &str) -> bool {
        self.inner
            .notice_claims
            .lock()
            .ambiguous_claims
            .contains_key(&(session_id.to_owned(), completion_id.to_owned()))
    }

    /// Whether `stamp` or `command_id` was issued by this Controller process lifetime.
    /// Startup recovery skips live dispatches belonging to this lifetime so it
    /// never marks an in-flight or ambiguous handoff `dispatch_unconfirmed`.
    #[must_use]
    pub fn owns_notice_claim(
        &self,
        session_id: &str,
        completion_id: &str,
        stamp: i64,
        command_id: Option<&str>,
    ) -> bool {
        let claims = self.inner.notice_claims.lock();
        if claims.this_lifetime_stamps.contains(&(
            session_id.to_owned(),
            completion_id.to_owned(),
            stamp,
        )) {
            return true;
        }
        command_id.is_some_and(|cmd| {
            claims
                .ambiguous_claims
                .get(&(session_id.to_owned(), completion_id.to_owned()))
                .is_some_and(|(ambiguous_cmd, _)| ambiguous_cmd == cmd)
        })
    }

    /// Drop an active dispatch claim once the notice echoed, settled, or was released synchronously.
    pub fn clear_notice_claim(&self, session_id: &str, completion_id: &str) {
        let mut claims = self.inner.notice_claims.lock();
        let key = (session_id.to_owned(), completion_id.to_owned());
        claims.active.remove(&key);
        claims.ambiguous_claims.remove(&key);
    }

    /// Clear all in-memory claim/release state once a notice is durably consumed.
    pub fn clear_all_notice_claims(&self, session_id: &str, completion_id: &str) {
        let mut claims = self.inner.notice_claims.lock();
        let key = (session_id.to_owned(), completion_id.to_owned());
        claims.active.remove(&key);
        claims.ambiguous_claims.remove(&key);
        claims.pending_releases.remove(&key);
    }

    /// Take any pending authoritative non-delivery `(stamp, command_id)` that must
    /// be cleared in the durable ledger before claiming a fresh dispatch attempt.
    #[must_use]
    pub fn take_pending_notice_release(
        &self,
        session_id: &str,
        completion_id: &str,
    ) -> Option<(i64, String)> {
        self.inner
            .notice_claims
            .lock()
            .pending_releases
            .remove(&(session_id.to_owned(), completion_id.to_owned()))
    }

    /// Restore a pending non-delivery release `(stamp, command_id)` if clearing it transiently failed.
    pub fn restore_pending_notice_release(
        &self,
        session_id: &str,
        completion_id: &str,
        stamp: i64,
        command_id: String,
    ) {
        self.inner.notice_claims.lock().pending_releases.insert(
            (session_id.to_owned(), completion_id.to_owned()),
            (stamp, command_id),
        );
    }

    /// Whether this notice has an authoritative non-delivery release awaiting DB commit.
    #[must_use]
    pub fn has_pending_notice_release(&self, session_id: &str, completion_id: &str) -> bool {
        self.inner
            .notice_claims
            .lock()
            .pending_releases
            .contains_key(&(session_id.to_owned(), completion_id.to_owned()))
    }

    /// Whether an owner withdrawal DB transaction is currently fencing this notice.
    #[must_use]
    pub fn is_notice_withdrawing(&self, session_id: &str, completion_id: &str) -> bool {
        self.inner
            .notice_claims
            .lock()
            .withdrawing
            .contains(&(session_id.to_owned(), completion_id.to_owned()))
    }

    /// Whether an owner queue withdrawal DB transaction is currently fencing
    /// this specific queued row ID (including plain prompts in a mixed `ClearQueue`).
    #[must_use]
    pub fn is_row_withdrawing(&self, session_id: &str, row_id: &str) -> bool {
        self.inner
            .notice_claims
            .lock()
            .withdrawing_row_ids
            .contains(&(session_id.to_owned(), row_id.to_owned()))
    }

    /// True if `Hub`'s in-memory transcript already contains a non-steering
    /// user echo for `cmid` (even before the write-behind store commits `events`).
    #[must_use]
    pub fn has_echoed_cmid(&self, session_id: &str, cmid: &str) -> bool {
        let sessions = self.inner.sessions.lock();
        sessions.get(session_id).is_some_and(|s| {
            s.log.iter().rev().any(|entry| {
                entry.cmid.as_deref() == Some(cmid)
                    && matches!(
                        &entry.event,
                        Event::Update { update }
                            if update
                                .get("sessionUpdate")
                                .and_then(serde_json::Value::as_str)
                                == Some("user_message_chunk")
                                && update
                                    .pointer("/_meta/cowboyDelivery")
                                    .and_then(serde_json::Value::as_str)
                                    != Some("current_turn")
                    )
            })
        })
    }

    /// Record a post-relay `result_read` consumption whose durable SQL commit
    /// failed after the result payload was already relayed to the caller, so
    /// notice dispatch/wake fences and flushes it instead of turning it into an
    /// extra parent turn.
    pub fn record_pending_result_consumption(
        &self,
        parent: &str,
        call_id: &str,
        completion_id: &str,
    ) {
        self.inner
            .notice_claims
            .lock()
            .pending_result_consumptions
            .insert(
                (parent.to_owned(), completion_id.to_owned()),
                call_id.to_owned(),
            );
    }

    /// Ref-counted pre-relay fence installed right before writing a terminal
    /// `calls.result` reply to the CodeAct sandbox. Decided atomically under
    /// `notice_claims`: returns `false` when a notice dispatch claim is already
    /// in progress (`claiming`) or active (`active`), preventing both the
    /// notice turn and the `calls.result` reply from being delivered together.
    #[must_use]
    pub fn try_reserve_result_relay(&self, parent: &str, completion_id: &str) -> bool {
        let mut claims = self.inner.notice_claims.lock();
        let key = (parent.to_owned(), completion_id.to_owned());
        if claims.claiming.contains(&key) || claims.active.contains_key(&key) {
            return false;
        }
        *claims.result_relay_reservations.entry(key).or_insert(0) += 1;
        true
    }

    pub fn release_result_relay(&self, parent: &str, completion_id: &str) {
        let mut claims = self.inner.notice_claims.lock();
        let key = (parent.to_owned(), completion_id.to_owned());
        if let Some(count) = claims.result_relay_reservations.get_mut(&key) {
            if *count <= 1 {
                claims.result_relay_reservations.remove(&key);
            } else {
                *count -= 1;
            }
        }
    }

    /// Atomically release one pre-relay reservation AND promote it to a durable
    /// `pending_result_consumptions` fence synchronously on sandbox flush so
    /// there is zero unfenced window during settlement retries (Round 3 #5).
    pub fn promote_result_relay_to_pending_consumption(
        &self,
        parent: &str,
        call_id: &str,
        completion_id: &str,
    ) {
        let mut claims = self.inner.notice_claims.lock();
        let key = (parent.to_owned(), completion_id.to_owned());
        if let Some(count) = claims.result_relay_reservations.get_mut(&key) {
            if *count <= 1 {
                claims.result_relay_reservations.remove(&key);
            } else {
                *count -= 1;
            }
        }
        claims
            .pending_result_consumptions
            .insert(key, call_id.to_owned());
    }

    #[must_use]
    pub fn is_result_relay_reserved(&self, parent: &str, completion_id: &str) -> bool {
        self.inner
            .notice_claims
            .lock()
            .result_relay_reservations
            .get(&(parent.to_owned(), completion_id.to_owned()))
            .is_some_and(|count| *count > 0)
    }

    #[must_use]
    pub fn pending_result_consumption(&self, parent: &str, completion_id: &str) -> Option<String> {
        self.inner
            .notice_claims
            .lock()
            .pending_result_consumptions
            .get(&(parent.to_owned(), completion_id.to_owned()))
            .cloned()
    }

    pub fn clear_pending_result_consumption(&self, parent: &str, completion_id: &str) {
        self.inner
            .notice_claims
            .lock()
            .pending_result_consumptions
            .remove(&(parent.to_owned(), completion_id.to_owned()));
    }

    #[must_use]
    pub fn pending_result_consumptions_for_parent(&self, parent: &str) -> Vec<(String, String)> {
        self.inner
            .notice_claims
            .lock()
            .pending_result_consumptions
            .iter()
            .filter(|((p, _), _)| p == parent)
            .map(|((_, completion_id), call_id)| (call_id.clone(), completion_id.clone()))
            .collect()
    }

    /// Fence a notice while a current-lifetime SQL dispatch claim is in flight
    /// so startup recovery cannot mark it `dispatch_unconfirmed` in the interval
    /// between the SQL claim commit and `record_notice_claim`.
    #[must_use]
    pub fn begin_notice_claim_guard(
        &self,
        session_id: &str,
        completion_id: &str,
    ) -> NoticeClaimGuard {
        let key = (session_id.to_owned(), completion_id.to_owned());
        self.inner.notice_claims.lock().claiming.insert(key.clone());
        NoticeClaimGuard {
            hub: self.clone(),
            key,
        }
    }

    /// Atomically install a `claiming` guard only when no `calls.result` relay
    /// reservation, pending result consumption, or owner withdrawal already
    /// owns this notice under `notice_claims`.
    #[must_use]
    pub fn try_begin_notice_claim_guard(
        &self,
        session_id: &str,
        completion_id: &str,
    ) -> Option<NoticeClaimGuard> {
        let key = (session_id.to_owned(), completion_id.to_owned());
        let mut claims = self.inner.notice_claims.lock();
        if claims
            .result_relay_reservations
            .get(&key)
            .is_some_and(|count| *count > 0)
            || claims.pending_result_consumptions.contains_key(&key)
            || claims.withdrawing.contains(&key)
        {
            return None;
        }
        claims.claiming.insert(key.clone());
        Some(NoticeClaimGuard {
            hub: self.clone(),
            key,
        })
    }

    /// Whether a current-lifetime dispatch claim is currently being acquired.
    #[must_use]
    pub fn is_notice_claiming(&self, session_id: &str, completion_id: &str) -> bool {
        self.inner
            .notice_claims
            .lock()
            .claiming
            .contains(&(session_id.to_owned(), completion_id.to_owned()))
    }

    /// Clone handle to the FIFO store-write acknowledgement registry.
    /// Does not retain `store_tx`, so holding it inside `run_store_writer` never
    /// creates a shutdown reference cycle.
    #[must_use]
    pub fn store_ack_sink(&self) -> StoreAckSink {
        self.inner.store_acks.clone()
    }

    /// Whether a durable store writer is wired to this `Hub`.
    #[must_use]
    pub fn has_store_writer(&self) -> bool {
        self.inner.store_tx.is_some()
    }

    /// Whether the queue dispatcher is wired to this `Hub`.
    #[must_use]
    pub fn has_dispatch_tx(&self) -> bool {
        self.inner.dispatch_tx.lock().is_some()
    }

    /// Whether a notice currently holds an active in-lifetime dispatch claim.
    #[must_use]
    pub fn has_active_notice_claim(&self, session_id: &str, completion_id: &str) -> bool {
        self.inner
            .notice_claims
            .lock()
            .active
            .contains_key(&(session_id.to_owned(), completion_id.to_owned()))
    }

    /// Whether a worker `UndeliveredPrompt` event (`"{worker_epoch}:{runtime_seq}"`)
    /// has already been durably processed for this notice in this Controller lifetime.
    #[must_use]
    pub fn is_undelivered_key_seen(
        &self,
        session_id: &str,
        completion_id: &str,
        undelivered_key: &str,
    ) -> bool {
        self.inner
            .notice_claims
            .lock()
            .seen_undelivered_keys
            .contains(&(
                session_id.to_owned(),
                completion_id.to_owned(),
                undelivered_key.to_owned(),
            ))
    }

    /// Remember an already-committed worker `UndeliveredPrompt` receipt so a
    /// redelivery after a lost ACK is a strict no-op.
    pub fn mark_undelivered_key_seen(
        &self,
        session_id: &str,
        completion_id: &str,
        undelivered_key: &str,
    ) {
        self.inner
            .notice_claims
            .lock()
            .seen_undelivered_keys
            .insert((
                session_id.to_owned(),
                completion_id.to_owned(),
                undelivered_key.to_owned(),
            ));
    }

    /// Enqueue an acknowledged `StoreWrite::UpdatePending` through the same FIFO
    /// store writer as all other pending snapshots so earlier buffered writes
    /// can never overwrite a committed withdrawal, edit, requeue, or recovery park.
    /// Returns `None` if no store writer is attached or the write channel rejected it.
    pub fn send_acked_update_pending(
        &self,
        session_id: &str,
        queue: Vec<QueuedMessage>,
        drafts: Vec<QueuedMessage>,
        settled_notices: Vec<(String, String)>,
        released_claims: Vec<NoticeClaimRelease>,
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
            require_claim_release,
            ack_id: Some(ack_id),
        });
        if !sent {
            self.inner.store_acks.cancel(ack_id);
            return None;
        }
        Some(rx)
    }

    /// Two-phase authoritative non-delivery requeue for a `call-notice:` prompt.
    /// Admits the conditional FIFO `StoreWrite::UpdatePending` and fences the
    /// notice + session pending snapshots under `sessions.lock()` WITHOUT
    /// mutating `Hub`'s queue, `in_flight`, or active claim until the SQL
    /// transaction confirms `Ok(true)`.
    ///
    /// Callers MUST hold [`Self::pending_commit_lock`] across preview, `ack_rx`,
    /// and [`Self::commit_undelivered_notice_requeue`] / [`Self::abort_undelivered_notice_requeue`].
    pub fn preview_and_send_undelivered_notice_requeue(
        &self,
        req: NoticeRequeueRequest,
    ) -> UndeliveredRequeue {
        let NoticeRequeueRequest {
            session_id,
            text,
            content,
            cmid,
            expected_command_id,
            expected_worker_epoch,
            require_worker_epoch_match,
            receipt_key,
            require_claim_release,
        } = req;
        let original = content.len();
        let content =
            crate::session_context::without_preamble(self, &session_id, Some(&cmid), content);
        let text = if content.len() < original {
            content
                .iter()
                .filter_map(|value| value.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            text
        };
        let mut sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get_mut(&session_id) else {
            return UndeliveredRequeue::NoStore;
        };
        let mut next_queue = s.queue.clone();
        let added_message = if next_queue
            .iter()
            .any(|m| m.cmid.as_deref() == Some(cmid.as_str()))
        {
            None
        } else {
            let msg = QueuedMessage {
                id: self.next_qid(),
                text,
                content,
                cmid: Some(cmid.clone()),
                schedule: None,
                origin: Some(crate::prompt_origin::DeliveryOrigin::cowboy("managed_call")),
                initiator: Some(crate::prompt_origin::DeliveryOrigin::cowboy("managed_call")),
            };
            next_queue.insert(0, msg.clone());
            Some(msg)
        };
        let drafts = s.drafts.clone();
        let active_claim = self
            .inner
            .notice_claims
            .lock()
            .active
            .get(&(session_id.clone(), cmid.clone()))
            .cloned();
        let (expected_stamp, resolved_cmd) = match (active_claim, expected_command_id) {
            (Some((stamp, active_cmd)), Some(req_cmd)) if active_cmd == req_cmd => {
                (Some(stamp), Some(req_cmd))
            }
            (Some(_), Some(req_cmd)) => {
                // Active claim belongs to a different dispatch attempt; keep
                // `expected_stamp: None` and pass `req_cmd` so SQL enforces the
                // exact command ID without conflating attempts.
                (None, Some(req_cmd))
            }
            (Some((stamp, active_cmd)), None) => (Some(stamp), Some(active_cmd)),
            (None, req_cmd) => (None, req_cmd),
        };
        let expected_in_flight_epoch = s
            .dispatched_cmids
            .iter()
            .any(|p| p.cmid == cmid && p.epoch == s.in_flight_epoch)
            .then_some(s.in_flight_epoch);
        if self.inner.store_tx.is_none() {
            // In-memory test mode without durable store.
            if let Some(msg) = added_message {
                s.queue.insert(0, msg);
            }
            if expected_in_flight_epoch == Some(s.in_flight_epoch)
                && s.dispatched_cmids
                    .iter()
                    .any(|p| p.cmid == cmid && p.epoch == s.in_flight_epoch)
            {
                s.in_flight = false;
                s.dispatch_pending = None;
            }
            s.dispatched_cmids.retain(|seen| seen.cmid != cmid);
            let mut claims = self.inner.notice_claims.lock();
            claims.active.remove(&(session_id.clone(), cmid.clone()));
            claims
                .pending_releases
                .remove(&(session_id.clone(), cmid.clone()));
            claims
                .seen_undelivered_keys
                .insert((session_id.clone(), cmid, receipt_key));
            let queue = s.queue.clone();
            let drafts = s.drafts.clone();
            drop(claims);
            drop(sessions);
            self.broadcast_pending_snapshot(&session_id, &queue, &drafts);
            return UndeliveredRequeue::NoStore;
        }
        let released_claims = vec![NoticeClaimRelease {
            completion_id: cmid.clone(),
            expected_stamp,
            expected_command_id: resolved_cmd.clone(),
            expected_worker_epoch,
            require_worker_epoch_match,
            receipt_key: receipt_key.clone(),
            reconcile_only: false,
        }];
        let Some(rx) = self.send_acked_update_pending(
            &session_id,
            next_queue,
            drafts,
            Vec::new(),
            released_claims,
            require_claim_release,
        ) else {
            return UndeliveredRequeue::Rejected(
                "persistence queue full or closed for undelivered notice requeue",
            );
        };
        let mut claims = self.inner.notice_claims.lock();
        claims
            .withdrawing
            .insert((session_id.clone(), cmid.clone()));
        claims
            .in_flight_pending_commits
            .insert(session_id.clone(), DeferredPendingFlush::default());
        UndeliveredRequeue::Pending(
            Box::new(PendingUndeliveredRequeue {
                hub: Arc::downgrade(&self.inner),
                active: true,
                session_id,
                cmid,
                added_message,
                expected_stamp,
                expected_command_id: resolved_cmd,
                expected_in_flight_epoch,
                receipt_key,
            }),
            rx,
        )
    }

    /// Commit an already-applied (`Ok(true)`) conditional undelivered notice
    /// requeue to `Hub`, lifting fences, flushing any deferred snapshot,
    /// broadcasting, and waking drain. Only clears `in_flight` and `claims.active`
    /// when they still match the exact epoch, stamp, and command ID captured at preview time.
    pub fn commit_undelivered_notice_requeue(&self, preview: &mut PendingUndeliveredRequeue) {
        if !preview.active {
            return;
        }
        preview.active = false;
        let (queue, drafts) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(&preview.session_id) else {
                let mut claims = self.inner.notice_claims.lock();
                claims
                    .withdrawing
                    .remove(&(preview.session_id.clone(), preview.cmid.clone()));
                claims.in_flight_pending_commits.remove(&preview.session_id);
                return;
            };
            if let Some(msg) = preview.added_message.as_ref()
                && !s
                    .queue
                    .iter()
                    .any(|m| m.cmid.as_deref() == Some(preview.cmid.as_str()))
            {
                s.queue.insert(0, msg.clone());
            }
            if preview.expected_in_flight_epoch == Some(s.in_flight_epoch)
                && s.dispatched_cmids
                    .iter()
                    .any(|p| p.cmid == preview.cmid && p.epoch == s.in_flight_epoch)
            {
                s.in_flight = false;
                s.dispatch_pending = None;
            }
            if preview.expected_in_flight_epoch.is_some() {
                s.dispatched_cmids.retain(|seen| {
                    !(seen.cmid == preview.cmid
                        && Some(seen.epoch) == preview.expected_in_flight_epoch)
                });
            }
            let queue = s.queue.clone();
            let drafts = s.drafts.clone();
            let mut claims = self.inner.notice_claims.lock();
            let key = (preview.session_id.clone(), preview.cmid.clone());
            claims.withdrawing.remove(&key);
            let active_matches = claims.active.get(&key).is_some_and(|(stamp, cmd)| {
                preview.expected_stamp.is_none_or(|s| s == *stamp)
                    && preview
                        .expected_command_id
                        .as_deref()
                        .is_none_or(|c| c == cmd)
            });
            if active_matches {
                claims.active.remove(&key);
            }
            claims.pending_releases.remove(&key);
            claims.seen_undelivered_keys.insert((
                preview.session_id.clone(),
                preview.cmid.clone(),
                preview.receipt_key.clone(),
            ));
            self.finish_in_flight_pending_commit(&preview.session_id, &queue, &drafts, &mut claims);
            (queue, drafts)
        };
        self.broadcast_pending_snapshot(&preview.session_id, &queue, &drafts);
        self.try_drain(&preview.session_id);
    }

    /// Release the exact Hub dispatch guard for a pre-delivery rejected notice
    /// when SQL confirmed the notice was ALREADY settled (e.g. consumed by a
    /// `calls.result` read before a broker capacity NACK arrived), without
    /// reinserting the notice into `queue` or touching any newer turn (Round 3 #14).
    pub fn release_rejected_settled_notice_dispatch(
        &self,
        preview: &mut PendingUndeliveredRequeue,
    ) {
        if !preview.active {
            return;
        }
        preview.active = false;
        {
            let mut sessions = self.inner.sessions.lock();
            let mut claims = self.inner.notice_claims.lock();
            let key = (preview.session_id.clone(), preview.cmid.clone());
            claims.withdrawing.remove(&key);
            if let Some(s) = sessions.get_mut(&preview.session_id) {
                if preview.expected_in_flight_epoch == Some(s.in_flight_epoch)
                    && s.dispatched_cmids
                        .iter()
                        .any(|p| p.cmid == preview.cmid && p.epoch == s.in_flight_epoch)
                {
                    s.in_flight = false;
                    s.dispatch_pending = None;
                }
                if preview.expected_in_flight_epoch.is_some() {
                    s.dispatched_cmids.retain(|seen| {
                        !(seen.cmid == preview.cmid
                            && Some(seen.epoch) == preview.expected_in_flight_epoch)
                    });
                }
                let active_matches = claims.active.get(&key).is_some_and(|(stamp, cmd)| {
                    preview.expected_stamp.is_none_or(|st| st == *stamp)
                        && preview
                            .expected_command_id
                            .as_deref()
                            .is_none_or(|c| c == cmd)
                });
                if active_matches {
                    claims.active.remove(&key);
                }
                claims.pending_releases.remove(&key);
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

    /// Abort an undelivered notice requeue (on SQL error or duplicate/settled
    /// `Ok(false)`), leaving `Hub` queue, `in_flight`, and active claims untouched.
    pub fn abort_undelivered_notice_requeue(
        &self,
        preview: &mut PendingUndeliveredRequeue,
        mark_seen: bool,
    ) {
        if !preview.active {
            return;
        }
        preview.active = false;
        {
            let sessions = self.inner.sessions.lock();
            let mut claims = self.inner.notice_claims.lock();
            claims
                .withdrawing
                .remove(&(preview.session_id.clone(), preview.cmid.clone()));
            if mark_seen {
                claims.seen_undelivered_keys.insert((
                    preview.session_id.clone(),
                    preview.cmid.clone(),
                    preview.receipt_key.clone(),
                ));
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

    /// Identity-checked teardown/recovery-abort fallback when durable conditional
    /// requeue fails: retains the unsent notice in Hub and moves its exact active
    /// claim into `pending_releases`, while ONLY clearing `in_flight` if the
    /// captured `expected_in_flight_epoch` still owns the session (Finding 10).
    pub fn fallback_undelivered_notice_requeue(&self, preview: &mut PendingUndeliveredRequeue) {
        if !preview.active {
            return;
        }
        preview.active = false;
        let (queue, drafts) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(&preview.session_id) else {
                let mut claims = self.inner.notice_claims.lock();
                claims
                    .withdrawing
                    .remove(&(preview.session_id.clone(), preview.cmid.clone()));
                claims.in_flight_pending_commits.remove(&preview.session_id);
                return;
            };
            if let Some(msg) = preview.added_message.as_ref()
                && !s
                    .queue
                    .iter()
                    .any(|m| m.cmid.as_deref() == Some(preview.cmid.as_str()))
            {
                s.queue.insert(0, msg.clone());
            }
            if preview.expected_in_flight_epoch == Some(s.in_flight_epoch)
                && s.dispatched_cmids
                    .iter()
                    .any(|p| p.cmid == preview.cmid && p.epoch == s.in_flight_epoch)
            {
                s.in_flight = false;
                s.dispatch_pending = None;
            }
            if preview.expected_in_flight_epoch.is_some() {
                s.dispatched_cmids.retain(|seen| {
                    !(seen.cmid == preview.cmid
                        && Some(seen.epoch) == preview.expected_in_flight_epoch)
                });
            }
            let queue = s.queue.clone();
            let drafts = s.drafts.clone();
            let mut claims = self.inner.notice_claims.lock();
            let key = (preview.session_id.clone(), preview.cmid.clone());
            claims.withdrawing.remove(&key);
            if let Some((active_stamp, active_cmd)) = claims.active.get(&key).cloned()
                && preview.expected_stamp.is_none_or(|st| st == active_stamp)
                && preview
                    .expected_command_id
                    .as_deref()
                    .is_none_or(|c| c == active_cmd)
            {
                claims.active.remove(&key);
                claims
                    .pending_releases
                    .insert(key, (active_stamp, active_cmd));
            }
            self.finish_in_flight_pending_commit(&preview.session_id, &queue, &drafts, &mut claims);
            (queue, drafts)
        };
        self.broadcast_pending_snapshot(&preview.session_id, &queue, &drafts);
        self.try_drain(&preview.session_id);
    }

    /// Identity-checked fallback when `preview_and_send_undelivered_notice_requeue`
    /// returned `Rejected` (store writer full/closed): preserves any newer prompt's
    /// `in_flight` guard and only moves the matching claim into `pending_releases`.
    pub fn fallback_undelivered_notice_requeue_rejected(&self, req: &NoticeRequeueRequest) {
        let original = req.content.len();
        let content = crate::session_context::without_preamble(
            self,
            &req.session_id,
            Some(&req.cmid),
            req.content.clone(),
        );
        let text = if content.len() < original {
            content
                .iter()
                .filter_map(|value| value.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            req.text.clone()
        };
        let (queue, drafts) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(&req.session_id) else {
                return;
            };
            let mut claims = self.inner.notice_claims.lock();
            let key = (req.session_id.clone(), req.cmid.clone());
            let claim_matches = claims.active.get(&key).is_some_and(|(_, active_cmd)| {
                req.expected_command_id
                    .as_deref()
                    .is_none_or(|c| c == active_cmd)
            });
            if !s
                .queue
                .iter()
                .any(|m| m.cmid.as_deref() == Some(req.cmid.as_str()))
            {
                let id = self.next_qid();
                s.queue.insert(
                    0,
                    QueuedMessage {
                        id,
                        text,
                        content,
                        cmid: Some(req.cmid.clone()),
                        schedule: None,
                        origin: Some(crate::prompt_origin::DeliveryOrigin::cowboy("managed_call")),
                        initiator: Some(crate::prompt_origin::DeliveryOrigin::cowboy(
                            "managed_call",
                        )),
                    },
                );
            }
            if claim_matches
                && s.dispatched_cmids
                    .iter()
                    .any(|p| p.cmid == req.cmid && p.epoch == s.in_flight_epoch)
            {
                s.in_flight = false;
                s.dispatch_pending = None;
                let epoch = s.in_flight_epoch;
                s.dispatched_cmids
                    .retain(|seen| !(seen.cmid == req.cmid && seen.epoch == epoch));
            }
            if claim_matches && let Some(entry) = claims.active.remove(&key) {
                claims.pending_releases.insert(key, entry);
            }
            (s.queue.clone(), s.drafts.clone())
        };
        self.broadcast_pending_snapshot(&req.session_id, &queue, &drafts);
        self.try_drain(&req.session_id);
    }

    /// Two-phase durable reconciliation for a notice requeue that ALREADY
    /// committed in `managed_call_notices` (observed via matching
    /// `last_undelivered_key` on a worker replay after a lost SQL commit
    /// response). Repairs durable `sessions.queue` through the FIFO store
    /// writer BEFORE mutating `Hub` and BEFORE the worker receives `Frame::Ack`,
    /// so a stale pre-requeue snapshot can never leave the notice missing from
    /// `sessions.queue` across a restart (Round 2 #15).
    pub fn preview_and_send_reconcile_notice_requeue(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: &str,
        expected_command_id: Option<&str>,
        receipt_key: &str,
    ) -> UndeliveredRequeue {
        let original = content.len();
        let content =
            crate::session_context::without_preamble(self, session_id, Some(cmid), content);
        let text = if content.len() < original {
            content
                .iter()
                .filter_map(|value| value.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            text
        };
        let mut sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get_mut(session_id) else {
            self.mark_undelivered_key_seen(session_id, cmid, receipt_key);
            return UndeliveredRequeue::NoStore;
        };
        let mut next_queue = s.queue.clone();
        let added_message = if next_queue.iter().any(|m| m.cmid.as_deref() == Some(cmid)) {
            None
        } else {
            let msg = QueuedMessage {
                id: self.next_qid(),
                text,
                content,
                cmid: Some(cmid.to_owned()),
                schedule: None,
                origin: Some(crate::prompt_origin::DeliveryOrigin::cowboy("managed_call")),
                initiator: Some(crate::prompt_origin::DeliveryOrigin::cowboy("managed_call")),
            };
            next_queue.insert(0, msg.clone());
            Some(msg)
        };
        let drafts = s.drafts.clone();
        let active_claim = self
            .inner
            .notice_claims
            .lock()
            .active
            .get(&(session_id.to_owned(), cmid.to_owned()))
            .cloned();
        let (expected_stamp, resolved_cmd) = match (active_claim, expected_command_id) {
            (Some((stamp, active_cmd)), Some(req_cmd)) if active_cmd == req_cmd => {
                (Some(stamp), Some(req_cmd.to_owned()))
            }
            (Some(_), Some(req_cmd)) => (None, Some(req_cmd.to_owned())),
            (Some((stamp, active_cmd)), None) => (Some(stamp), Some(active_cmd)),
            (None, req_cmd) => (None, req_cmd.map(str::to_owned)),
        };
        let expected_in_flight_epoch = s
            .dispatched_cmids
            .iter()
            .any(|p| p.cmid == cmid && p.epoch == s.in_flight_epoch)
            .then_some(s.in_flight_epoch);
        if self.inner.store_tx.is_none() {
            if let Some(msg) = added_message {
                s.queue.insert(0, msg);
            }
            let mut claims = self.inner.notice_claims.lock();
            let key = (session_id.to_owned(), cmid.to_owned());
            let active_matches = claims.active.get(&key).is_none_or(|(_, active_cmd)| {
                resolved_cmd
                    .as_deref()
                    .is_none_or(|expected| expected == active_cmd)
            });
            if active_matches {
                if expected_in_flight_epoch == Some(s.in_flight_epoch)
                    && s.dispatched_cmids
                        .iter()
                        .any(|p| p.cmid == cmid && p.epoch == s.in_flight_epoch)
                {
                    s.in_flight = false;
                    s.dispatch_pending = None;
                    let epoch = s.in_flight_epoch;
                    s.dispatched_cmids
                        .retain(|seen| !(seen.cmid == cmid && seen.epoch == epoch));
                }
                claims.active.remove(&key);
                claims.pending_releases.remove(&key);
            }
            claims.seen_undelivered_keys.insert((
                session_id.to_owned(),
                cmid.to_owned(),
                receipt_key.to_owned(),
            ));
            let queue = s.queue.clone();
            let drafts = s.drafts.clone();
            drop(claims);
            drop(sessions);
            self.broadcast_pending_snapshot(session_id, &queue, &drafts);
            self.try_drain(session_id);
            return UndeliveredRequeue::NoStore;
        }
        let reconcile_guard = vec![NoticeClaimRelease {
            completion_id: cmid.to_owned(),
            expected_stamp: None,
            expected_command_id: None,
            expected_worker_epoch: None,
            require_worker_epoch_match: false,
            receipt_key: receipt_key.to_owned(),
            reconcile_only: true,
        }];
        let Some(rx) = self.send_acked_update_pending(
            session_id,
            next_queue,
            drafts,
            Vec::new(),
            reconcile_guard,
            true,
        ) else {
            return UndeliveredRequeue::Rejected(
                "persistence queue full or closed for committed notice requeue reconciliation",
            );
        };
        let mut claims = self.inner.notice_claims.lock();
        claims
            .withdrawing
            .insert((session_id.to_owned(), cmid.to_owned()));
        claims
            .in_flight_pending_commits
            .insert(session_id.to_owned(), DeferredPendingFlush::default());
        UndeliveredRequeue::Pending(
            Box::new(PendingUndeliveredRequeue {
                hub: Arc::downgrade(&self.inner),
                active: true,
                session_id: session_id.to_owned(),
                cmid: cmid.to_owned(),
                added_message,
                expected_stamp,
                expected_command_id: resolved_cmd,
                expected_in_flight_epoch,
                receipt_key: receipt_key.to_owned(),
            }),
            rx,
        )
    }

    /// Move an active dispatch claim into `pending_releases` and return `(completion_id, stamp, command_id)`
    /// so `requeue_prompt` commits both the requeued queue and the claim release in one SQL tx.
    pub(super) fn take_requeue_claim_release(
        &self,
        session_id: &str,
        completion_id: &str,
    ) -> Option<(String, i64, String)> {
        let key = (session_id.to_owned(), completion_id.to_owned());
        let mut claims = self.inner.notice_claims.lock();
        let (stamp, command_id) = claims.active.remove(&key)?;
        claims
            .pending_releases
            .insert(key, (stamp, command_id.clone()));
        Some((completion_id.to_owned(), stamp, command_id))
    }
}
