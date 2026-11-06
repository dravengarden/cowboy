//! The Hub's per-session send queue and drafts: dispatch, draining, delivery holds, submission, queue and draft edits, and scheduled drafts.

use super::*;

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
    pub(super) fn next_qid(&self) -> String {
        format!("q{}", self.inner.next_qid.fetch_add(1, Ordering::Relaxed))
    }

    /// Re-broadcast (and persist) a session's queue + drafts after any change.
    /// Re-locks `sessions`, so callers MUST NOT hold the lock when calling.
    pub(super) fn emit_pending(&self, session_id: &str) {
        let (queue, drafts) = {
            let sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get(session_id) else {
                return;
            };
            (s.queue.clone(), s.drafts.clone())
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdatePending {
                session_id: session_id.to_owned(),
                queue: queue.clone(),
                drafts: drafts.clone(),
            });
        }
        // Broadcast on the generic optimistic-sync channel as state "queue:<sid>".
        // Confirmed = the cmids in the value, so a client drops its optimistic add
        // the moment its cmid lands here (the cross-terminal reconcile).
        let confirmed = Self::cmids_of(&queue, &drafts);
        let value = serde_json::json!({ "queue": queue, "drafts": drafts });
        self.sync_emit(&format!("queue:{session_id}"), value, confirmed);
    }

    pub(super) fn send_dispatch(&self, req: DispatchReq) {
        if let Some(tx) = self.inner.dispatch_tx.lock().as_ref()
            && let Err(error) = tx.try_send(req)
        {
            let req = error.into_inner();
            tracing::error!(session = %req.session_id, "dispatch queue rejected a prompt");
            self.requeue_prompt(&req.session_id, req.text, req.content, req.cmid);
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
    ) -> bool {
        if self.inner.dispatch_tx.lock().is_none() {
            return false;
        }
        let req = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if s.delivery_hold.as_deref() != Some(operation)
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
            }
        };
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
        // ANY message being edited pauses the WHOLE auto-drain — not just when
        // the head is the one open (the old `== head` check let the head fire
        // out from under you while you edited message #2). The hold lifts on
        // Save/Cancel (set_queue_editing(None)). A MANUAL "send now" still
        // overrides (the user explicitly chose to send), so the queue is never
        // permanently trapped.
        if !manual && s.editing.is_some() {
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
        can && !s.in_flight && s.dispatch_pending.is_none() && s.delivery_hold.is_none()
    }

    /// Dispatch the head of a session's queue if it can take a turn and the head
    /// isn't held for editing. Pops the head, marks in-flight, hands the prompt
    /// to the dispatcher task, and re-broadcasts the shrunken queue. No-op
    /// otherwise. `allow_revive` is forwarded to [`Self::ready`].
    pub(super) fn drain_head(&self, session_id: &str, allow_revive: bool, manual: bool) {
        // Without a dispatcher wired we must not pop (the prompt would be lost).
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
            let head = s.queue.remove(0);
            s.explicit_wait = false;
            s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
            s.in_flight = true;
            s.dispatch_pending = Some(s.in_flight_epoch);
            Self::note_dispatched(s, head.cmid.as_deref());
            DispatchReq {
                ownership: DispatchOwnership::new(self, session_id, s.in_flight_epoch),
                session_id: session_id.to_owned(),
                text: head.text,
                content: head.content,
                cmid: head.cmid,
                explicit: manual,
            }
        };
        self.emit_pending(session_id);
        self.send_dispatch(req);
    }

    /// The AUTO-drain: only fires into an alive idle agent (never revives a dead
    /// one). Called after every queue mutation and on every status change.
    pub(super) fn try_drain(&self, session_id: &str) {
        self.drain_head(session_id, false, false);
    }

    /// MANUAL drain of the queue head: bypasses the paused hold
    /// (the user explicitly chose "send now") and revives a dormant session. Used
    /// by force-push so a ⚡ on a PAUSED queue runs the front message immediately
    /// WITHOUT resuming the rest of the held queue.
    pub fn drain_now(&self, session_id: &str) {
        self.drain_head(session_id, true, true);
    }

    /// Retry a never-dispatched resource wait, respecting pause and editing.
    pub fn retry_resource_wait(&self, session_id: &str) {
        self.drain_head(session_id, true, false);
    }

    /// Remember that the requeued head is an explicit send waiting for a
    /// transient resource, or forget that intent.
    pub fn set_explicit_wait(&self, session_id: &str, explicit: bool) {
        if let Some(s) = self.inner.sessions.lock().get_mut(session_id) {
            s.explicit_wait = explicit;
        }
        // The persisted wait carries the intent across a Controller restart.
        let key = format!(
            "{}{session_id}",
            settings_keys::SESSION_RESOURCE_WAIT_PREFIX
        );
        if let Some(serde_json::Value::Object(mut wait)) = self.setting(&key)
            && wait
                .get("explicit")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
                != explicit
        {
            wait.insert("explicit".to_owned(), serde_json::Value::Bool(explicit));
            self.set_setting(key, serde_json::Value::Object(wait));
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
    ) {
        if let Some(id) = cmid
            .as_deref()
            .filter(|id| id.starts_with("cowboy-superagent:"))
        {
            self.abandon_superagent_prompt(session_id, id);
            return;
        }
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
        {
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
            }
            if let Some(c) = cmid.as_deref()
                && s.queue.iter().any(|m| m.cmid.as_deref() == Some(c))
            {
                return;
            }
            let id = self.next_qid();
            s.queue.insert(
                0,
                QueuedMessage {
                    id,
                    text,
                    content,
                    cmid,
                    schedule: None,
                },
            );
        }
        self.emit_pending(session_id);
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
    ) {
        self.submit_inner(session_id, text, content, cmid, false, false);
    }

    /// The admitted bootstrap precedes messages queued while its intent was
    /// being persisted. It still respects Pause and never interrupts a turn.
    pub fn submit_context(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: String,
    ) {
        self.submit_inner(session_id, text, content, Some(cmid), false, true);
        self.drain_head(session_id, true, false);
    }

    pub(super) fn submit_inner(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        managed: bool,
        front: bool,
    ) {
        // One controller owns a managed child conversation: its parent call.
        if !managed && self.is_managed_child(session_id) {
            self.broadcast_error(
                Some(session_id.to_owned()),
                "This child conversation is controlled by its parent's managed call".to_owned(),
            );
            return;
        }
        // Only a human submit resets the scheduler's runaway guard; automated
        // coordination and completion notices stay in the unattended streak.
        if crate::prompt_origin::origin_from_cmid(cmid.as_deref()).is_human() {
            self.notify_human_turn(session_id);
        }
        let wired = self.inner.dispatch_tx.lock().is_some();
        let mut dispatch = None;
        let duplicate;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            // Idempotent on cmid: a retry whose original landed in the queue, was
            // handed to a worker, or already echoed into the transcript must not
            // run again. The reconnecting client resends every unconfirmed submit.
            duplicate = cmid
                .as_deref()
                .filter(|c| Self::prompt_already_accepted(s, c))
                .map(str::to_owned);
            if duplicate.is_some() {
                // fall through to the addressed confirmation below
            } else if wired
                && !s.meta.paused
                // A trashed session keeps admitted work queued (a scheduled
                // wakeup is preserved, never consumed) until it is restored.
                && s.meta.trashed_at_ms.is_none()
                && Self::ready(s, true)
                && s.queue.is_empty()
                && self.context_allows_dispatch(session_id, cmid.as_deref())
            {
                s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
                s.in_flight = true;
                s.dispatch_pending = Some(s.in_flight_epoch);
                Self::note_dispatched(s, cmid.as_deref());
                dispatch = Some(DispatchReq {
                    ownership: DispatchOwnership::new(self, session_id, s.in_flight_epoch),
                    session_id: session_id.to_owned(),
                    text,
                    content,
                    cmid,
                    explicit: true,
                });
            } else {
                let id = self.next_qid();
                let message = QueuedMessage {
                    id,
                    text,
                    content,
                    cmid,
                    schedule: None,
                };
                if front {
                    s.queue.insert(0, message);
                } else {
                    s.queue.push(message);
                }
            }
        }
        if let Some(cmid) = duplicate {
            self.confirm_delivered(session_id, &cmid);
            return;
        }
        match dispatch {
            // Dispatched straight through — never touched a list, so no flicker
            // of the prompt appearing-then-leaving the queue.
            Some(req) => self.send_dispatch(req),
            None => self.emit_pending(session_id),
        }
    }

    /// Force-push a fresh prompt (the long-press-send affordance). When a turn is
    /// in flight, the prompt jumps to the FRONT of the queue and this returns
    /// `true` so the caller interrupts the running turn — the cancelled turn ends,
    /// the drain then runs this prompt next. On an idle session there's nothing to
    /// jump ahead of, so it behaves exactly like `submit` (dispatches straight
    /// through) and returns `false`. Same cmid-idempotency as `submit`.
    #[must_use]
    pub fn force_submit(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        // `true` = also interrupt the running turn (force push). `false` = just
        // jump to the front of the queue and let the current turn finish first
        // ("jump to front" / `submit { front: true }`).
        interrupt_on_busy: bool,
    ) -> bool {
        self.force_submit_once(session_id, text, content, cmid, interrupt_on_busy)
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
    ) -> Option<bool> {
        if self.is_managed_child(session_id) {
            return None;
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
                // Idle + nothing queued → straight dispatch, identical to submit.
                s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
                s.in_flight = true;
                s.dispatch_pending = Some(s.in_flight_epoch);
                Self::note_dispatched(s, cmid.as_deref());
                dispatch = Some(DispatchReq {
                    ownership: DispatchOwnership::new(self, session_id, s.in_flight_epoch),
                    session_id: session_id.to_owned(),
                    text,
                    content,
                    cmid,
                    explicit: true,
                });
            } else {
                // Busy / draining / queued ahead → jump to the FRONT so it runs
                // next; ask the caller to interrupt the in-flight turn only when
                // this is a force push (not a no-interrupt "jump to front").
                let id = self.next_qid();
                s.queue.insert(
                    0,
                    QueuedMessage {
                        id,
                        text,
                        content,
                        cmid,
                        schedule: None,
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
    pub(super) fn confirm_delivered(&self, session_id: &str, cmid: &str) {
        let (queue, drafts) = {
            let sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get(session_id) else {
                return;
            };
            // Dispatch admission deduplicates execution but does not prove
            // delivery. Until a queue row or user echo owns the prompt, the
            // browser must retain its durable outbox (including attachments).
            if !s
                .queue
                .iter()
                .chain(&s.drafts)
                .any(|m| m.cmid.as_deref() == Some(cmid))
                && !s
                    .dispatched_cmids
                    .iter()
                    .any(|seen| seen.cmid == cmid && seen.echoed)
                && !s.log.iter().any(|entry| {
                    entry.cmid.as_deref() == Some(cmid) && is_user_message_chunk(entry)
                })
            {
                return;
            }
            (s.queue.clone(), s.drafts.clone())
        };
        let mut confirmed = Self::cmids_of(&queue, &drafts);
        if !confirmed.iter().any(|seen| seen == cmid) {
            confirmed.push(cmid.to_owned());
        }
        let value = serde_json::json!({ "queue": queue, "drafts": drafts });
        self.sync_emit(&format!("queue:{session_id}"), value, confirmed);
    }

    /// Drop one queued prompt. Returns whether the row was still queued.
    pub fn remove_queued(&self, session_id: &str, id: &str) -> bool {
        let removed = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            let before = s.queue.len();
            s.queue.retain(|m| m.id != id);
            if s.editing.as_deref() == Some(id) {
                s.editing = None;
            }
            before != s.queue.len()
        };
        self.emit_pending(session_id);
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
    ) -> bool {
        let found = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if text.trim().is_empty() && content.is_empty() {
                let before = s.queue.len();
                s.queue.retain(|m| m.id != id);
                if s.editing.as_deref() == Some(id) {
                    s.editing = None;
                }
                before != s.queue.len()
            } else if let Some(m) = s.queue.iter_mut().find(|m| m.id == id) {
                m.text = text;
                m.content = content;
                true
            } else {
                false
            }
        };
        self.emit_pending(session_id);
        self.try_drain(session_id);
        found
    }

    /// Drop a session's whole queue.
    pub fn clear_queue(&self, session_id: &str) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.queue.clear();
            s.editing = None;
        }
        self.emit_pending(session_id);
    }

    /// "Send now": move a queued prompt to the front, then dispatch it. This is
    /// an EXPLICIT user action, so it may revive an exited/crashed session
    /// (`allow_revive` = true) — unlike the auto-drain. If the agent is mid-turn it
    /// just becomes next in line.
    pub fn request_send_queued(&self, session_id: &str, id: &str) {
        if self.is_managed_child(session_id) {
            return;
        }
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if let Some(pos) = s.queue.iter().position(|m| m.id == id) {
                let m = s.queue.remove(pos);
                s.queue.insert(0, m);
            } else {
                return;
            }
        }
        self.emit_pending(session_id);
        // Explicit user "send now" → manual drain, bypassing the awaiting hold.
        self.drain_head(session_id, true, true);
    }

    /// Overlay "Retry" for an errored/crashed turn: re-run the last user prompt
    /// (reviving the session). No-op if there's no prior prompt.
    pub fn retry_turn(&self, session_id: &str) {
        if self.is_managed_child(session_id) {
            return;
        }
        let (prompt, mut content, status, retry_cmid, already_queued) = {
            let sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get(session_id) else {
                tracing::warn!(session = %session_id, "retry_turn: unknown session — no-op");
                return;
            };
            let Some(last_user_seq) = s.log.iter().rev().find_map(|envelope| {
                let Event::Update { update } = &envelope.event else {
                    return None;
                };
                (update
                    .get("sessionUpdate")
                    .and_then(serde_json::Value::as_str)
                    == Some("user_message_chunk"))
                .then_some(envelope.seq)
            }) else {
                // Startup can fail before the first prompt reaches the native
                // agent. That prompt is still queued, with its original cmid
                // and attachments. Retry its delivery through preflight rather
                // than requiring a user event that cannot yet exist.
                drop(sessions);
                self.drain_head(session_id, true, true);
                return;
            };
            let retry_cmid = format!("cowboy-retry:{session_id}:{last_user_seq}");
            let LastUserPrompt {
                text: prompt,
                content,
                cmid: echoed_cmid,
            } = last_user_prompt(&s.log);
            if s.in_flight
                || s.queue
                    .iter()
                    .any(|message| message.cmid.as_deref() == Some(retry_cmid.as_str()))
            {
                tracing::info!(session = %session_id, "retry_turn: identical retry already pending — no-op");
                return;
            }
            // A rejected prompt may already sit in the queue (requeued after a
            // failed dispatch). Identify it by its cmid when the echo still
            // carries one; otherwise by text, or — for an attachment-only
            // prompt whose text is empty — by the attachment shape, so one
            // image-only prompt never passes for another.
            let already_queued = s
                .queue
                .iter()
                .find(|message| {
                    if echoed_cmid.is_some() && message.cmid == echoed_cmid {
                        return true;
                    }
                    if prompt.trim().is_empty() {
                        attachment_signature(&message.content) == attachment_signature(&content)
                    } else {
                        message.text == prompt
                    }
                })
                .map(|message| message.id.clone());
            (prompt, content, s.meta.status, retry_cmid, already_queued)
        };
        if prompt.trim().is_empty() && content.is_empty() {
            // The "no response" report points here first: a crashed turn whose
            // user prompt never made it into the log leaves nothing to re-run.
            tracing::warn!(session = %session_id, ?status, "retry_turn: no prior prompt to retry — no-op");
            return;
        }
        if let Some(id) = already_queued {
            tracing::info!(session = %session_id, "retry_turn: rejected prompt already queued — sending it without duplication");
            self.request_send_queued(session_id, &id);
            return;
        }
        if let Err(reference) = self.restore_externalized_images(&mut content) {
            // History only keeps a URL for a large image; without the stored
            // bytes the retry would silently drop the attachment (an ACP image
            // block needs inline data). Refuse, and say so on the failure card.
            tracing::warn!(session = %session_id, ?status, %reference, "retry_turn: attachment no longer stored — not retrying");
            if matches!(
                status,
                Status::Crashed | Status::Exited | Status::Interrupted
            ) {
                self.set_status(
                    session_id,
                    status,
                    Some(
                        "Retry needs an attachment that is no longer stored; send the prompt again with the file."
                            .to_owned(),
                    ),
                );
            }
            return;
        }
        tracing::info!(session = %session_id, ?status, prompt_len = prompt.len(), blocks = content.len(), "retry_turn: re-submitting last prompt");
        let _ = self.force_submit(session_id, prompt, content, Some(retry_cmid), true);
        // force_submit DISPATCHES only when the queue is empty AND the session is
        // ready; with messages already queued — or a crashed/exited session — it
        // just parks the prompt at the queue FRONT and emits pending. That left
        // Retry looking like "added to the top of the queue, now send it yourself".
        // Drain the head WITH revive so Retry runs
        // the prompt immediately, reviving a dead session — no manual send. Safe
        // after a direct dispatch too: force_submit set `in_flight`, so `ready`
        // returns false and this drain no-ops (no double send).
        self.drain_head(session_id, true, true);
    }

    /// Move a queued prompt back to drafts.
    pub fn queued_to_draft(&self, session_id: &str, id: &str) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if let Some(pos) = s.queue.iter().position(|m| m.id == id) {
                let m = s.queue.remove(pos);
                if s.editing.as_deref() == Some(id) {
                    s.editing = None;
                }
                s.drafts.push(m);
            } else {
                return;
            }
        }
        self.emit_pending(session_id);
        self.try_drain(session_id);
    }

    /// Hold (`Some`) or release (`None`) the queue head for editing. A held head
    /// pauses the drain on every terminal; releasing tries the drain again.
    pub fn set_queue_editing(&self, session_id: &str, id: Option<String>) -> u64 {
        let released = id.is_none();
        let epoch = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return 0;
            };
            s.editing_epoch = s.editing_epoch.wrapping_add(1);
            s.editing = id;
            s.editing_epoch
        };
        if released {
            self.try_drain(session_id);
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
            true
        };
        if released {
            self.try_drain(session_id);
        }
        released
    }

    /// Park a new draft.
    pub fn add_draft(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) {
        let duplicate;
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            // Idempotent on cmid: when a send looked failed the client resends
            // with the SAME cmid, but the original may actually have landed — so
            // a matching cmid means "already staged", don't double-add. Confirm
            // it again instead: the resending client missed the first patch.
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
    ) {
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
                    let m = &mut s.drafts[p];
                    if !text.trim().is_empty() || !content.is_empty() {
                        m.text = text;
                        m.content = content;
                    }
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
            // lock that removes the Draft, so a concurrent Trash cannot slip in.
            if s.meta.trashed_at_ms.is_some() {
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
            if wired
                && !s.meta.paused
                && Self::ready(s, true)
                && s.queue.is_empty()
                && self.context_allows_dispatch(session_id, cmid.as_deref())
            {
                s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
                s.in_flight = true;
                s.dispatch_pending = Some(s.in_flight_epoch);
                dispatch = Some(DispatchReq {
                    ownership: DispatchOwnership::new(self, session_id, s.in_flight_epoch),
                    session_id: session_id.to_owned(),
                    text: m.text,
                    content: m.content,
                    cmid,
                    // A scheduled fire never bypasses the user's Pause.
                    explicit: false,
                });
            } else {
                let id = self.next_qid();
                let msg = QueuedMessage {
                    id,
                    text: m.text,
                    content: m.content,
                    cmid,
                    schedule: None,
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
    ) -> bool {
        let (found, unscheduled) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if text.trim().is_empty() && content.is_empty() {
                let had = s.drafts.iter().any(|m| m.id == id && m.schedule.is_some());
                let before = s.drafts.len();
                s.drafts.retain(|m| m.id != id);
                (before != s.drafts.len(), had)
            } else if let Some(m) = s.drafts.iter_mut().find(|m| m.id == id) {
                m.text = text;
                m.content = content;
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
            if row.schedule.is_some() || row.text != text || row.content != content {
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
    pub fn move_draft(&self, from: &str, id: &str, to: &str) {
        if from == to {
            return;
        }
        let scheduled = {
            let mut sessions = self.inner.sessions.lock();
            // Take the draft out of `from`, but only if `to` exists to receive it.
            let Some(msg) = (if sessions.contains_key(to) {
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
            let scheduled = msg.schedule.as_ref().map(|sc| sc.fire_at_ms);
            // `to` existed at the top of this lock and we still hold it, so this
            // can't miss; the `if let` just avoids an unwrap.
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
            let ids = s
                .drafts
                .iter()
                .filter(|m| m.schedule.is_some())
                .map(|m| m.id.clone())
                .collect();
            s.drafts.clear();
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
    pub fn activate_draft(&self, session_id: &str, id: &str, cmid: Option<String>) {
        if self.is_managed_child(session_id) {
            return;
        }
        let msg = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.drafts
                .iter()
                .position(|m| m.id == id)
                .map(|pos| s.drafts.remove(pos))
        };
        if let Some(m) = msg {
            self.emit_pending(session_id);
            // Manually sending a scheduled draft now → cancel its pending fire.
            if m.schedule.is_some() {
                self.cancel_draft_timer(session_id, id);
                self.broadcast_sessions();
            }
            // Echo the explicit send identity so the sender can retire its
            // optimistic bubble. Older clients retain the draft's identity.
            self.submit(session_id, m.text, m.content, cmid.or(m.cmid));
        }
    }

    /// Activate every draft, front-to-back, then clear them.
    pub fn activate_all_drafts(&self, session_id: &str) {
        if self.is_managed_child(session_id) {
            return;
        }
        let msgs = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            std::mem::take(&mut s.drafts)
        };
        if msgs.is_empty() {
            return;
        }
        self.emit_pending(session_id);
        let mut had_scheduled = false;
        for m in msgs {
            // Manually sending a scheduled draft now → cancel its pending fire.
            if m.schedule.is_some() {
                self.cancel_draft_timer(session_id, &m.id);
                had_scheduled = true;
            }
            // Bulk activation preserves the same identity as a single send.
            self.submit(session_id, m.text, m.content, m.cmid);
        }
        if had_scheduled {
            self.broadcast_sessions();
        }
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
