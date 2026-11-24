//! The Hub's per-session send queue and drafts: dispatch, draining, delivery holds, submission, queue and draft edits, and scheduled drafts.

use super::*;

mod drafts;
mod retry;
mod submit;

/// Why a session's queue head stays queued, or that it may be dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DrainDecision {
    Dispatch,
    Trashed,
    NotReady,
    Paused,
    Empty,
    ContextHold,
    Editing,
}

impl Hub {
    pub(super) fn seed_restored_queue_ids(&self, sessions: &[RestoredSession]) {
        // Seed the qid counter PAST every restored id. The counter (`next_qid`)
        // is in-memory and resets to 1 on each daemon restart, so without this a
        // draft/queued message created after a restart reuses q1, q2, … and
        // collides with a pre-restart one — duplicate ids, which the client keys
        // rows by, so a "3 Drafts" header renders only 2 distinct rows. Done
        // before the loop so the dedup below can mint fresh ids past the max.
        let mut max_qid = 0u64;
        for r in sessions {
            for m in r.queue.iter().chain(r.drafts.iter()) {
                if let Some(n) = m.id.strip_prefix('q').and_then(|s| s.parse::<u64>().ok()) {
                    max_qid = max_qid.max(n);
                }
            }
        }
        self.inner.next_qid.store(max_qid + 1, Ordering::Relaxed);
    }

    pub(super) fn next_qid(&self) -> String {
        format!("q{}", self.inner.next_qid.fetch_add(1, Ordering::Relaxed))
    }

    /// Re-broadcast (and persist) a session's queue + drafts after any change.
    /// Re-locks `sessions`, so callers MUST NOT hold the lock when calling.
    pub(super) fn emit_pending(&self, session_id: &str) {
        self.emit_pending_with_notice_ops(session_id, Vec::new(), Vec::new());
    }

    pub(super) fn send_dispatch(&self, req: DispatchReq) {
        if let Some(tx) = self.inner.dispatch_tx.lock().as_ref()
            && let Err(error) = tx.try_send(req)
        {
            let req = error.into_inner();
            tracing::error!(session = %req.session_id, "dispatch queue rejected a prompt");
            self.requeue_prompt(
                &req.session_id,
                req.text,
                req.content,
                req.cmid,
                req.origin,
                req.initiator,
            );
            self.broadcast_error(
                Some(req.session_id),
                "dispatch queue is full; prompt remains queued".to_owned(),
            );
        }
    }

    /// Whether a queued prompt can be dispatched now. `allow_revive` is the
    /// crucial distinction between the two callers:
    ///
    /// - **AUTO-drain** (turn-end / a queue mutation while idle) passes `false`,
    ///   so it ONLY fires into an alive, idle agent (`Running`). It must never
    ///   revive a dead one — otherwise an agent that exits or crashes MID-TASK
    ///   clears the in-flight guard on its death edge and the auto-drain would
    ///   immediately send the next queued prompt into a freshly-revived agent
    ///   that has lost the unfinished task. That is the "a task wasn't done but
    ///   the queue auto-sent" bug.
    /// - **EXPLICIT** sends (submit a new message, "send now") pass `true`,
    ///   deliberately reviving an exited/crashed session via `session/load`.
    ///
    /// Both require that nothing of ours is already in flight.
    pub fn delivery_hold(&self, session_id: &str) -> Option<String> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .and_then(|s| s.delivery_hold.clone())
    }

    /// Caller must commit the corresponding setting first. This method only
    /// publishes already-durable state, never sends a second write-behind.
    pub fn publish_delivery_hold(&self, session_id: &str, operation: Option<String>) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.delivery_hold.clone_from(&operation);
        }
        self.load_settings(vec![(
            format!(
                "{}{session_id}",
                settings_keys::SESSION_DELIVERY_HOLD_PREFIX
            ),
            serde_json::json!(operation),
        )]);
        self.broadcast_sessions();
        if operation.is_none() {
            self.try_drain(session_id);
        }
    }

    /// Run exactly the replacement message after its original turn has ended.
    /// Other queued items and the user's Pause remain unchanged.
    pub fn dispatch_interrupt_once(
        &self,
        session_id: &str,
        operation: &str,
        text: String,
        content: Vec<serde_json::Value>,
        caller: &crate::prompt_origin::RequestCaller,
    ) -> bool {
        if self.reject_reserved_notice_cmid(session_id, Some(operation))
            || self.inner.dispatch_tx.lock().is_none()
        {
            return false;
        }
        let origin = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        let dispatch_origin =
            crate::prompt_origin::compute_dispatch_origin(Some(&origin), Some(&origin));
        let req = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if s.delivery_hold.as_deref() != Some(operation)
                || s.clear_pending
                || !self.context_allows_dispatch(session_id, Some(operation))
                || s.meta.status != Status::Running
                || s.in_flight
                || s.dispatch_pending.is_some()
            {
                return false;
            }
            if Self::prompt_already_accepted(s, operation) {
                return true;
            }
            s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
            s.in_flight = true;
            s.dispatch_pending = Some(s.in_flight_epoch);
            Self::note_dispatched(s, Some(operation));
            DispatchReq {
                ownership: DispatchOwnership::new(self, session_id, s.in_flight_epoch),
                session_id: session_id.to_owned(),
                text,
                content,
                cmid: Some(operation.to_owned()),
                explicit: true,
                origin: Some(origin.clone()),
                initiator: Some(origin.clone()),
                dispatch_origin: dispatch_origin.clone(),
                trigger: origin,
            }
        };
        if dispatch_origin.is_owner() {
            self.notify_human_turn(session_id);
        }
        self.send_dispatch(req);
        true
    }

    /// The drain policy: whether the queue head may be dispatched now, or why
    /// it stays queued. Pure over the session; the context check runs only
    /// when every earlier condition passed, in the same order as before.
    pub(super) fn drain_decision(
        s: &Session,
        allow_revive: bool,
        manual: bool,
        context_allows: impl FnOnce(Option<&str>) -> bool,
    ) -> DrainDecision {
        // A trashed session holds its queue until it is restored.
        if s.meta.trashed_at_ms.is_some() {
            return DrainDecision::Trashed;
        }
        if !Self::ready(s, allow_revive) {
            return DrainDecision::NotReady;
        }
        // The user MANUALLY paused the drain (the ⏸ toggle) → hold the queue
        // after the running turn finishes. A manual send still
        // overrides (the user can force a specific message through).
        if !manual && s.meta.paused {
            return DrainDecision::Paused;
        }
        let Some(head) = s.queue.first() else {
            return DrainDecision::Empty;
        };
        if !context_allows(head.cmid.as_deref()) {
            return DrainDecision::ContextHold;
        }
        // Editing a queued message pauses the whole queue auto-drain; editing a
        // scheduled draft protects only that draft's fire timer without freezing
        // the unrelated queue. Manual sends always override.
        let editing_draft = s
            .editing
            .as_deref()
            .is_some_and(|id| s.drafts.iter().any(|m| m.id == id));
        if !manual && s.editing.is_some() && !editing_draft {
            return DrainDecision::Editing;
        }
        DrainDecision::Dispatch
    }

    /// The drain policy for one session as the drain would evaluate it now.
    #[cfg(test)]
    pub(super) fn drain_decision_for(
        &self,
        session_id: &str,
        allow_revive: bool,
        manual: bool,
    ) -> Option<DrainDecision> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        Some(Self::drain_decision(s, allow_revive, manual, |cmid| {
            self.context_allows_dispatch(session_id, cmid)
        }))
    }

    pub(super) fn ready(s: &Session, allow_revive: bool) -> bool {
        let can = if allow_revive {
            matches!(
                s.meta.status,
                Status::Running | Status::Exited | Status::Crashed | Status::Interrupted
            )
        } else {
            s.meta.status == Status::Running
        };
        can && !s.in_flight
            && s.dispatch_pending.is_none()
            && s.delivery_hold.is_none()
            && !s.clear_pending
    }

    /// Dispatch the head of a session's queue if it can take a turn and the head
    /// isn't held for editing. Pops the head, marks in-flight, hands the prompt
    /// to the dispatcher task, and re-broadcasts the shrunken queue. No-op
    /// otherwise. `allow_revive` is forwarded to [`Self::ready`].
    pub(super) fn drain_head(
        &self,
        session_id: &str,
        allow_revive: bool,
        manual: bool,
        trigger: Option<&crate::prompt_origin::RequestCaller>,
    ) {
        if self.inner.dispatch_tx.lock().is_none() {
            return;
        }
        let req = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let decision = Self::drain_decision(s, allow_revive, manual, |cmid| {
                self.context_allows_dispatch(session_id, cmid)
            });
            if decision != DrainDecision::Dispatch {
                return;
            }
            // A retried explicit send carries no caller of its own: it must
            // not take a row out from under the terminal editing it. A refused
            // dispatch requeues the row under a new id, which that editor reads
            // as a sent message and parks its buffer as a duplicate draft. The
            // retry resumes once the edit is saved or abandoned.
            // A caller-less manual drain is a retried explicit send: it needs
            // live intent. Another terminal may have sent the held row since
            // maintenance read the flag; the next row then gets no bypass.
            if manual && trigger.is_none() && !s.explicit_wait {
                return;
            }
            if manual
                && trigger.is_none()
                && s.queue
                    .first()
                    .is_some_and(|head| s.editing.as_deref() == Some(head.id.as_str()))
            {
                return;
            }
            // Its intent belongs to the row it was recorded for: once the
            // user removed or reordered that row, the new head is not sent
            // through Pause on its behalf, and the intent ends. Checked under
            // the lock that takes the row out.
            if manual
                && trigger.is_none()
                && s.explicit_row
                    .as_ref()
                    .is_some_and(|row| s.queue.first().is_none_or(|head| &head.id != row))
            {
                s.explicit_wait = false;
                s.explicit_row = None;
                drop(sessions);
                self.persist_explicit_wait(session_id);
                return;
            }
            if s.queue.first().is_some_and(|head| {
                self.is_row_withdrawing(session_id, &head.id)
                    || head
                        .cmid
                        .as_deref()
                        .is_some_and(|cid| self.is_notice_withdrawing(session_id, cid))
            }) {
                return;
            }
            let head = s.queue.remove(0);
            let effective_trigger = trigger
                .map(crate::prompt_origin::DeliveryOrigin::from_caller)
                .or_else(|| head.initiator.clone());
            let dispatch_origin = crate::prompt_origin::compute_dispatch_origin(
                head.origin.as_ref(),
                effective_trigger.as_ref(),
            );
            let record_trigger = effective_trigger
                .clone()
                .unwrap_or_else(|| crate::prompt_origin::DeliveryOrigin::cowboy("legacy_queue"));
            let dispatch_cmid = hub_dispatch_origin::ensure_dispatch_cmid(head.cmid);
            s.explicit_wait = false;
            s.explicit_row = None;
            s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
            s.in_flight = true;
            s.dispatch_pending = Some(s.in_flight_epoch);
            Self::note_dispatched(s, Some(&dispatch_cmid));
            DispatchReq {
                ownership: DispatchOwnership::new(self, session_id, s.in_flight_epoch),
                session_id: session_id.to_owned(),
                text: head.text,
                content: head.content,
                cmid: Some(dispatch_cmid),
                explicit: manual,
                origin: head.origin,
                initiator: effective_trigger,
                dispatch_origin,
                trigger: record_trigger,
            }
        };
        if req.dispatch_origin.is_owner() {
            self.notify_human_turn(session_id);
        }
        self.emit_pending(session_id);
        self.send_dispatch(req);
    }

    pub(super) fn try_drain(&self, session_id: &str) {
        self.drain_head(session_id, false, false, None);
    }

    pub fn drain_now(&self, session_id: &str, caller: &crate::prompt_origin::RequestCaller) {
        self.drain_head(session_id, true, true, Some(caller));
    }

    pub fn retry_resource_wait(&self, session_id: &str) {
        self.drain_head(session_id, true, false, None);
    }

    /// Retry a previously explicit send parked on a resource wait, preserving
    /// its stored `(origin, initiator)` without fabricating a new caller.
    /// Only the row that intent was recorded for is sent this way.
    pub fn retry_explicit_wait(&self, session_id: &str) {
        self.drain_head(session_id, true, true, None);
    }

    /// Whether the head row is held by a terminal editing it, which a retried
    /// explicit send waits for.
    pub fn head_held_for_editing(&self, session_id: &str) -> bool {
        self.inner.sessions.lock().get(session_id).is_some_and(|s| {
            s.queue
                .first()
                .is_some_and(|head| s.editing.as_deref() == Some(head.id.as_str()))
        })
    }

    /// Record (or end) explicit send intent for the current head row.
    pub fn set_explicit_wait(&self, session_id: &str, explicit: bool) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.explicit_wait = explicit;
            s.explicit_row = s
                .queue
                .first()
                .filter(|_| explicit)
                .map(|head| head.id.clone());
        }
        self.persist_explicit_wait(session_id);
    }

    /// Write the session's explicit intent into its persisted wait, which
    /// carries it across a Controller restart.
    pub fn persist_explicit_wait(&self, session_id: &str) {
        // Read and write as one step: two concurrent holds must not leave the
        // older row's intent persisted over the newer one.
        static PERSIST: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
        let _serial = PERSIST.lock();
        let Some((explicit, row)) = self
            .inner
            .sessions
            .lock()
            .get(session_id)
            .map(|s| (s.explicit_wait, s.explicit_row.clone()))
        else {
            return;
        };
        let key = format!(
            "{}{session_id}",
            settings_keys::SESSION_RESOURCE_WAIT_PREFIX
        );
        if let Some(serde_json::Value::Object(mut wait)) = self.setting(&key) {
            let before = wait.clone();
            wait.insert("explicit".to_owned(), serde_json::Value::Bool(explicit));
            match row {
                Some(row) => {
                    wait.insert("row".to_owned(), serde_json::Value::String(row));
                }
                None => {
                    wait.remove("row");
                }
            }
            if wait != before {
                self.set_setting(key, serde_json::Value::Object(wait));
            }
        }
    }

    pub fn explicit_wait(&self, session_id: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|s| s.explicit_wait)
    }

    /// Clear the in-flight guard (used by the dispatcher when a send fails) and
    /// try the next queued prompt.
    pub fn clear_in_flight(&self, session_id: &str) {
        {
            let mut sessions = self.inner.sessions.lock();
            if let Some(s) = sessions.get_mut(session_id) {
                s.in_flight = false;
                s.dispatch_pending = None;
            }
        }
        self.try_drain(session_id);
    }

    /// Complete one agent turn and release at most one dispatch guard.
    ///
    /// Runtime transports publish both `TurnEnded` and a trailing idle status.
    /// The first completion drains the next prompt; the second must be a no-op
    /// until that prompt reports its own `Busy` start, otherwise two queued
    /// prompts can be handed to the same worker at once.
    pub fn complete_turn(&self, session_id: &str) {
        let first_completion = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if s.turn_completion_latched || s.dispatch_pending.is_some() {
                false
            } else {
                s.in_flight = false;
                s.dispatch_pending = None;
                s.turn_completion_latched = true;
                true
            }
        };
        if first_completion {
            self.try_drain(session_id);
        }
    }

    /// Reconcile an authoritative runtime snapshot that says the worker is idle.
    ///
    /// Remote worker lifecycle events can straddle a Machine broker reconnect.
    /// If Cowboy missed the Busy -> Running edge, the Hub may still retain the
    /// dispatch guard even though the worker snapshot proves that no turn is
    /// active. Callers must first prove that no prompt command is still pending
    /// in the controller; otherwise a Running snapshot can merely predate a
    /// prompt that is still travelling to the worker.
    pub fn reconcile_runtime_idle(&self, session_id: &str, expected_epoch: u64) {
        let released = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if s.meta.status != Status::Running
                || !s.in_flight
                || s.in_flight_epoch != expected_epoch
                || s.dispatch_pending.is_some()
            {
                false
            } else {
                s.in_flight = false;
                s.dispatch_pending = None;
                s.turn_completion_latched = true;
                true
            }
        };
        if released {
            tracing::warn!(
                session = %session_id,
                "authoritative idle runtime snapshot released stale in-flight guard"
            );
            self.try_drain(session_id);
        }
    }

    /// Put a dispatched-but-never-run prompt BACK on the queue front.
    ///
    /// A prompt sent to a session that had to REVIVE rides `cmd_rx` into the
    /// freshly-spawned agent thread. If that agent dies during cold-start (e.g.
    /// the `npx` adapter fails to install) it returns BEFORE the command loop
    /// consumes `cmd_rx`, so the prompt is never logged and — without this —
    /// evaporates with the dead thread: gone from the composer (cleared on send),
    /// the queue (it was dispatched straight through), the transcript (never
    /// echoed), and even Retry (which reads the log). `run_agent` salvages the
    /// un-consumed prompt here so it lands back in the durable queue — visible
    /// again, and re-drained the moment the session next reaches `Running`.
    /// Idempotent on `cmid` (a racing re-revive must not double-queue it).
    pub fn requeue_prompt(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        origin: Option<crate::prompt_origin::DeliveryOrigin>,
        initiator: Option<crate::prompt_origin::DeliveryOrigin>,
    ) {
        if let Some(id) = cmid
            .as_deref()
            .filter(|id| id.starts_with("cowboy-superagent:"))
        {
            self.record_session_error(
                session_id,
                &format!("Super-agent delivery returned without replay: {id}"),
            );
            self.abandon_superagent_prompt(session_id, id);
            return;
        }
        let released_claims: Vec<crate::store::NoticeClaimRelease> = cmid
            .as_deref()
            .filter(|id| id.starts_with(crate::agent_model::CALL_NOTICE_PREFIX))
            .and_then(|id| self.take_requeue_claim_release(session_id, id))
            .map(|(cid, stamp, cmd_id)| crate::store::NoticeClaimRelease {
                completion_id: cid,
                expected_stamp: Some(stamp),
                expected_command_id: Some(cmd_id.clone()),
                expected_worker_epoch: None,
                require_worker_epoch_match: false,
                receipt_key: format!("requeue:{cmd_id}"),
                reconcile_only: false,
            })
            .into_iter()
            .collect();
        let original = content.len();
        let content =
            crate::session_context::without_preamble(self, session_id, cmid.as_deref(), content);
        // Runtime returns join every text block; drop the preamble's line too.
        let text = if content.len() < original {
            content
                .iter()
                .filter_map(|value| value.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            text
        };
        let inserted = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            // The in-flight turn is over (it crashed); free the guard so the queue
            // can drain again once an agent is alive.
            s.in_flight = false;
            s.dispatch_pending = None;
            // The prompt never ran: it is no longer "delivered", so the drain (or
            // a client resend) may hand it to a worker again.
            if let Some(c) = cmid.as_deref() {
                s.dispatched_cmids.retain(|seen| seen.cmid != c);
                hub_dispatch_origin::remove_pending_echo(s, c);
            }
            if let Some(c) = cmid.as_deref()
                && s.queue.iter().any(|m| m.cmid.as_deref() == Some(c))
            {
                false
            } else {
                let id = self.next_qid();
                s.queue.insert(
                    0,
                    QueuedMessage {
                        id,
                        text,
                        content,
                        cmid,
                        schedule: None,
                        origin,
                        initiator,
                    },
                );
                true
            }
        };
        if inserted || !released_claims.is_empty() {
            self.emit_pending_with_notice_ops(session_id, Vec::new(), released_claims);
        }
    }

    /// Queue-aware send: dispatch immediately when the session is idle and
    /// nothing is queued/in-flight; otherwise append to the queue. The single
    /// entry point the Web composer uses (the bridge/API still use `Prompt`).
    pub fn submit(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        caller: &crate::prompt_origin::RequestCaller,
    ) {
        if self.reject_reserved_notice_cmid(session_id, cmid.as_deref()) {
            return;
        }
        let origin = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        self.submit_inner(
            session_id,
            text,
            content,
            cmid,
            false,
            false,
            Some(origin.clone()),
            origin,
        );
    }

    pub fn submit_context(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: String,
        caller: &crate::prompt_origin::RequestCaller,
    ) {
        let origin = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        self.submit_inner(
            session_id,
            text,
            content,
            Some(cmid),
            false,
            true,
            Some(origin.clone()),
            origin,
        );
        self.drain_head(session_id, true, false, Some(caller));
    }

    #[must_use]
    pub fn force_submit(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        interrupt_on_busy: bool,
        caller: &crate::prompt_origin::RequestCaller,
    ) -> bool {
        self.force_submit_once(session_id, text, content, cmid, interrupt_on_busy, caller)
            .unwrap_or(false)
    }

    /// `None` means no new admission: callers must not cancel or drain on replay.
    pub fn force_submit_once(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        interrupt_on_busy: bool,
        caller: &crate::prompt_origin::RequestCaller,
    ) -> Option<bool> {
        let origin = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        self.force_submit_with_provenance(
            session_id,
            text,
            content,
            cmid,
            interrupt_on_busy,
            Some(origin.clone()),
            origin,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn force_submit_with_provenance(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        interrupt_on_busy: bool,
        content_origin: Option<crate::prompt_origin::DeliveryOrigin>,
        initiator: crate::prompt_origin::DeliveryOrigin,
    ) -> Option<bool> {
        if self.is_managed_child(session_id)
            || self.reject_reserved_notice_cmid(session_id, cmid.as_deref())
        {
            return None;
        }
        let dispatch_origin = crate::prompt_origin::compute_dispatch_origin(
            content_origin.as_ref(),
            Some(&initiator),
        );
        if dispatch_origin.is_owner() {
            self.notify_human_turn(session_id);
        }
        let wired = self.inner.dispatch_tx.lock().is_some();
        let mut dispatch = None;
        let mut interrupt = false;
        let duplicate;
        {
            let mut sessions = self.inner.sessions.lock();
            let s = sessions.get_mut(session_id)?;
            let context_allows = self.context_allows_dispatch(session_id, cmid.as_deref());
            duplicate = cmid
                .as_deref()
                .filter(|c| Self::prompt_already_accepted(s, c))
                .map(str::to_owned);
            if duplicate.is_some() {
                // fall through to the addressed confirmation below
            } else if wired
                && Self::ready(s, true)
                && s.queue.is_empty()
                && self.context_allows_dispatch(session_id, cmid.as_deref())
            {
                let dispatch_cmid = hub_dispatch_origin::ensure_dispatch_cmid(cmid);
                s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
                s.in_flight = true;
                s.dispatch_pending = Some(s.in_flight_epoch);
                Self::note_dispatched(s, Some(&dispatch_cmid));
                dispatch = Some(DispatchReq {
                    ownership: DispatchOwnership::new(self, session_id, s.in_flight_epoch),
                    session_id: session_id.to_owned(),
                    text,
                    content,
                    cmid: Some(dispatch_cmid),
                    explicit: true,
                    origin: content_origin,
                    initiator: Some(initiator.clone()),
                    dispatch_origin,
                    trigger: initiator,
                });
            } else {
                let id = self.next_qid();
                s.queue.insert(
                    0,
                    QueuedMessage {
                        id,
                        text,
                        content,
                        cmid,
                        schedule: None,
                        origin: content_origin,
                        initiator: Some(initiator),
                    },
                );
                interrupt = interrupt_on_busy && context_allows;
            }
        }
        if let Some(cmid) = duplicate {
            self.confirm_delivered(session_id, &cmid);
            return None;
        }
        match dispatch {
            Some(req) => self.send_dispatch(req),
            None => self.emit_pending(session_id),
        }
        Some(interrupt)
    }

    /// Whether `cmid` names a prompt a browser client minted. Hub-synthesized
    /// ids (scheduler wakeups, scheduled drafts, retry, legacy continuation)
    /// are deliberately reusable and never participate in delivery dedupe.
    /// A human/API prompt, not a scheduler, retry or managed-call prompt.
    pub fn is_authored_cmid(cmid: &str) -> bool {
        Self::is_client_cmid(cmid)
            && !cmid.starts_with(MANAGED_CALL_CMID_PREFIX)
            && !cmid.starts_with(crate::agent_model::CALL_NOTICE_PREFIX)
            && !cmid.starts_with(crate::agent_model::PEER_NOTICE_PREFIX)
    }

    pub(super) fn is_client_cmid(cmid: &str) -> bool {
        !cmid.is_empty() && !cmid.starts_with("__") && !cmid.starts_with("cowboy-")
    }

    pub(super) fn tracks_delivery(cmid: &str) -> bool {
        Self::is_client_cmid(cmid) || cmid.starts_with("cowboy-superagent:")
    }

    /// True once this Hub has accepted the prompt: still queued, handed to a
    /// worker, or echoed into the transcript (which survives
    /// a Controller restart because the echo persists its `cmid`).
    pub(super) fn prompt_already_accepted(s: &Session, cmid: &str) -> bool {
        if s.queue.iter().any(|m| m.cmid.as_deref() == Some(cmid)) {
            return true;
        }
        Self::tracks_delivery(cmid)
            && (s.dispatched_cmids.iter().any(|seen| seen.cmid == cmid)
                || s.log
                    .iter()
                    .rev()
                    .any(|entry| entry.cmid.as_deref() == Some(cmid)))
    }

    /// Tell every client that `cmid` is already folded into this session, without
    /// changing or re-persisting the queue. Confirmations are monotonic facts the
    /// sync client accepts from any patch, so the resending device retires its
    /// outbox entry instead of waiting out an acknowledgement timeout.
    /// Capture `(queue, drafts, editing)` and assign the monotonic `queue:<sid>`
    /// sync version under one `sync -> sessions` lock acquisition (matching
    /// `sync_resync`'s lock order), so concurrent queue mutations and hold
    /// changes can never publish older rows under a newer version.
    pub(super) fn publish_pending_snapshot(&self, session_id: &str, extra_confirmed: &[String]) {
        let state = format!("queue:{session_id}");
        let (version, value, confirmed) = {
            let mut reg = self.inner.sync.lock();
            let sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get(session_id) else {
                return;
            };
            let entry = reg.entry(state.clone()).or_default();
            entry.version += 1;
            let version = entry.version;
            let mut confirmed = Self::cmids_of(&s.queue, &s.drafts);
            for cmid in extra_confirmed {
                if !confirmed.iter().any(|existing| existing == cmid) {
                    confirmed.push(cmid.clone());
                }
            }
            let value = serde_json::json!({
                "queue": &s.queue,
                "drafts": &s.drafts,
                "editing": &s.editing,
            });
            (version, value, confirmed)
        };
        tracing::info!(target: "cowboy::oplog", op = "change", %state, version, confirmed = ?confirmed);
        self.fanout(Outbound::SyncPatch {
            state,
            version,
            value,
            confirmed,
            resync: false,
        });
    }

    pub(super) fn confirm_delivered(&self, session_id: &str, cmid: &str) {
        let should_confirm = {
            let sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get(session_id) else {
                return;
            };
            // Dispatch admission deduplicates execution but does not prove
            // delivery. Until a queue row or user echo owns the prompt, the
            // browser must retain its durable outbox (including attachments).
            s.queue
                .iter()
                .chain(&s.drafts)
                .any(|m| m.cmid.as_deref() == Some(cmid))
                || s.dispatched_cmids
                    .iter()
                    .any(|seen| seen.cmid == cmid && seen.echoed)
                || s.log.iter().any(|entry| {
                    entry.cmid.as_deref() == Some(cmid) && is_user_message_chunk(entry)
                })
        };
        if !should_confirm {
            return;
        }
        self.publish_pending_snapshot(session_id, &[cmid.to_owned()]);
    }

    /// Drop one queued prompt. Returns whether the row was still queued.
    pub fn remove_queued(&self, session_id: &str, id: &str) -> bool {
        let (removed, settled) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            let settled = submit::extract_notice_settlements(s.queue.iter().filter(|m| m.id == id));
            let before = s.queue.len();
            s.queue.retain(|m| m.id != id);
            if s.editing.as_deref() == Some(id) {
                s.editing = None;
            }
            (before != s.queue.len(), settled)
        };
        self.emit_pending_with_notice_ops(session_id, settled, Vec::new());
        self.try_drain(session_id);
        removed
    }

    /// Remove exactly one still-queued prompt by its client correlation id.
    ///
    /// ACP `session/cancel` is session-scoped, but a bridge prompt can be
    /// waiting behind a turn started by another surface. Cancelling that
    /// request must not interrupt the unrelated active turn. Returns whether a
    /// queued prompt was removed; an already-dispatched prompt is absent and
    /// must instead be cancelled through the provider.
    pub fn remove_queued_by_cmid(&self, session_id: &str, cmid: &str) -> bool {
        let removed = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            let before = s.queue.len();
            s.queue.retain(|m| m.cmid.as_deref() != Some(cmid));
            before != s.queue.len()
        };
        if removed {
            self.emit_pending(session_id);
            self.try_drain(session_id);
        }
        removed
    }

    /// Edit a queued prompt in place. Empty text + content removes it. Returns
    /// whether the row was still queued.
    pub fn edit_queued(
        &self,
        session_id: &str,
        id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        caller: &crate::prompt_origin::RequestCaller,
    ) -> bool {
        let editor = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        let (found, settled) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if text.trim().is_empty() && content.is_empty() {
                let settled =
                    submit::extract_notice_settlements(s.queue.iter().filter(|m| m.id == id));
                let before = s.queue.len();
                s.queue.retain(|m| m.id != id);
                if s.editing.as_deref() == Some(id) {
                    s.editing = None;
                }
                (before != s.queue.len(), settled)
            } else if let Some(m) = s.queue.iter_mut().find(|m| m.id == id) {
                let settled = submit::extract_notice_settlements(std::iter::once(&*m));
                let authority_changed =
                    m.origin.as_ref() != Some(&editor) || m.initiator.as_ref() != Some(&editor);
                if !settled.is_empty() || authority_changed {
                    m.cmid = None;
                }
                m.text = text;
                m.content = content;
                m.origin = Some(editor.clone());
                m.initiator = Some(editor);
                (true, settled)
            } else {
                (false, Vec::new())
            }
        };
        self.emit_pending_with_notice_ops(session_id, settled, Vec::new());
        self.try_drain(session_id);
        found
    }

    /// Drop a session's whole queue.
    pub fn clear_queue(&self, session_id: &str) {
        let settled = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let settled = submit::extract_notice_settlements(s.queue.iter());
            s.queue.clear();
            // Only drop queue edit holds; a scheduled-draft hold belongs to
            // `s.drafts` and must stay intact so its deferred timer still fires.
            let editing_draft = s
                .editing
                .as_deref()
                .is_some_and(|id| s.drafts.iter().any(|m| m.id == id));
            if !editing_draft {
                s.editing = None;
            }
            settled
        };
        self.emit_pending_with_notice_ops(session_id, settled, Vec::new());
    }

    /// "Send now": move a queued prompt to the front, then dispatch it. This is
    /// an EXPLICIT user action, so it may revive an exited/crashed session
    /// (`allow_revive` = true) — unlike the auto-drain. If the agent is mid-turn it
    /// just becomes next in line.
    pub fn request_send_queued(
        &self,
        session_id: &str,
        id: &str,
        caller: &crate::prompt_origin::RequestCaller,
    ) {
        if self.is_managed_child(session_id) {
            return;
        }
        if self.promote_queued(session_id, id, caller) {
            self.drain_head(session_id, true, true, Some(caller));
        }
    }

    /// Move a queued prompt to the front under `caller`'s send intent without
    /// dispatching it, and record that explicit intent for exactly this row in
    /// the same critical section: a session that cannot dispatch now keeps the
    /// row in place instead of taking it out only to requeue it under a new
    /// id. The caller persists the intent with its wait. Whether the row exists.
    pub fn hold_queued(
        &self,
        session_id: &str,
        id: &str,
        caller: &crate::prompt_origin::RequestCaller,
    ) -> bool {
        self.promote_with(session_id, id, caller, true)
    }

    /// Move a queued prompt to the front under `caller`'s send intent.
    pub fn promote_queued(
        &self,
        session_id: &str,
        id: &str,
        caller: &crate::prompt_origin::RequestCaller,
    ) -> bool {
        self.promote_with(session_id, id, caller, false)
    }

    fn promote_with(
        &self,
        session_id: &str,
        id: &str,
        caller: &crate::prompt_origin::RequestCaller,
        hold: bool,
    ) -> bool {
        if self.is_managed_child(session_id) {
            return false;
        }
        let trigger = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            let Some(pos) = s.queue.iter().position(|m| m.id == id) else {
                return false;
            };
            let mut m = s.queue.remove(pos);
            m.initiator = Some(trigger);
            s.queue.insert(0, m);
            if hold {
                s.explicit_wait = true;
                s.explicit_row = Some(id.to_owned());
            }
        }
        self.emit_pending(session_id);
        true
    }

    /// Move a queued prompt back to drafts.
    pub fn queued_to_draft(
        &self,
        session_id: &str,
        id: &str,
        caller: &crate::prompt_origin::RequestCaller,
    ) {
        let actor = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        let settled = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if let Some(pos) = s.queue.iter().position(|m| m.id == id) {
                let mut m = s.queue.remove(pos);
                let settled = submit::extract_notice_settlements(std::iter::once(&m));
                if !settled.is_empty() {
                    m.cmid = None;
                }
                if !actor.is_owner() {
                    if m.origin.as_ref() != Some(&actor) {
                        m.cmid = None;
                    }
                    m.origin = Some(actor);
                }
                m.initiator = None;
                if s.editing.as_deref() == Some(id) {
                    s.editing = None;
                }
                s.drafts.push(m);
                settled
            } else {
                return;
            }
        };
        self.emit_pending_with_notice_ops(session_id, settled, Vec::new());
        self.try_drain(session_id);
    }

    fn take_due_deferred_drafts(s: &mut Session, keep_editing_id: Option<&str>) -> Vec<String> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let mut kept = Vec::new();
        let mut due = Vec::new();
        for draft_id in std::mem::take(&mut s.deferred_scheduled_drafts) {
            if keep_editing_id == Some(draft_id.as_str()) {
                kept.push(draft_id);
                continue;
            }
            if s.drafts.iter().any(|m| {
                m.id == draft_id
                    && m.schedule
                        .as_ref()
                        .is_some_and(|sc| sc.fire_at_ms <= now_ms)
            }) {
                due.push(draft_id);
            }
        }
        s.deferred_scheduled_drafts = kept;
        due
    }

    /// Hold (`Some`) or release (`None`) a queued message or scheduled draft for
    /// editing. Broadcasts `editing` to every client; releasing or displacing a
    /// hold retries auto-drain when the queue is unblocked and fires any
    /// scheduled draft whose fire time elapsed while it was held.
    pub fn set_queue_editing(&self, session_id: &str, id: Option<String>) -> u64 {
        let (epoch, unblock_queue, deferred_due) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return 0;
            };
            s.editing_epoch = s.editing_epoch.wrapping_add(1);
            s.editing = id;
            let editing_draft = s
                .editing
                .as_deref()
                .is_some_and(|cur| s.drafts.iter().any(|m| m.id == cur));
            let unblock_queue = s.editing.is_none() || editing_draft;
            let keep = s.editing.clone();
            let deferred = Self::take_due_deferred_drafts(s, keep.as_deref());
            (s.editing_epoch, unblock_queue, deferred)
        };
        self.publish_pending_snapshot(session_id, &[]);
        if unblock_queue {
            self.try_drain(session_id);
        }
        for draft_id in deferred_due {
            self.fire_scheduled_draft_at(session_id, &draft_id, None);
        }
        epoch
    }

    /// Release a disconnected editor only if no replacement connection has
    /// renewed the same transaction during the reload grace period.
    pub fn release_queue_editing_if_epoch(&self, session_id: &str, id: &str, epoch: u64) -> bool {
        let released = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if s.editing_epoch != epoch || s.editing.as_deref() != Some(id) {
                return false;
            }
            s.editing = None;
            s.editing_epoch = s.editing_epoch.wrapping_add(1);
            Some(Self::take_due_deferred_drafts(s, None))
        };
        if let Some(deferred_due) = released {
            self.publish_pending_snapshot(session_id, &[]);
            self.try_drain(session_id);
            for draft_id in deferred_due {
                self.fire_scheduled_draft_at(session_id, &draft_id, None);
            }
            return true;
        }
        false
    }

    // --- Reorder --------------------------------------------------------------

    /// Reorder one session's queue to the given id order, then re-broadcast +
    /// persist. Ids not in `order` keep their relative order at the end (a
    /// stable sort), so a stale/partial order can't drop messages. Also re-tries
    /// the drain in case the new head is now dispatchable.
    pub fn reorder_queue(&self, session_id: &str, order: &[String]) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            sort_by_id_order(&mut s.queue, order, |m| &m.id);
        }
        self.emit_pending(session_id);
        self.try_drain(session_id);
    }

    /// Reorder one session's drafts to the given id order (see `reorder_queue`).
    pub fn reorder_drafts(&self, session_id: &str, order: &[String]) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            sort_by_id_order(&mut s.drafts, order, |m| &m.id);
        }
        self.emit_pending(session_id);
    }

    /// Reorder the session list to the given id order, then persist + broadcast.
    /// Submitted ids only permute names they include; existing ids are never dropped.
    pub fn reorder_sessions(&self, order: &[String]) {
        {
            let mut list = self.inner.order.lock();
            *list = merge_session_order(&list, order);
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateSessionOrder {
                order: self.inner.order.lock().clone(),
            });
        }
        self.broadcast_sessions();
    }
}

#[cfg(test)]
mod drain_policy_tests;
