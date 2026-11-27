//! Controller-held in-flight prompt registry for [`super::RemoteRuntime`].
//!
//! Tracks exact `(command_id, turn_id, session_id, cmid, content, origin,
//! initiator)` handed off by this Controller BEFORE enqueueing into `pending`,
//! so worker `UndeliveredPrompt` receipts can restore Owner provenance ONLY
//! when matching an un-echoed, unfinished command this Controller actually
//! sent, using the Controller's own `content` rather than worker-supplied text.
//!
//! Entries stay live across `TurnStarted` (because the worker emits
//! `TurnStarted` before forwarding the prompt to the ACP thread, where
//! cold-start salvage emits `UndeliveredPrompt`) and retire on the matching
//! `user_message_chunk` echo, the matching `TurnEnded { turn_id }`, a
//! `CommandAck` rejection, `UndeliveredPrompt` consumption, or shutdown.

use std::collections::{HashMap, VecDeque};

use crate::prompt_origin::DeliveryOrigin;
use crate::runtime_wire::RuntimeEvent;

use super::Shared;

const MAX_IN_FLIGHT_PROMPTS: usize = 2048;
/// Owner prompts with attachments kept until their turn ends, so one the
/// agent refused before running it returns to Drafts as sent. Only running
/// turns hold one, within a count and a byte budget; an evicted or retired
/// turn is simply not restored.
const MAX_TURN_PROMPTS: usize = 64;
const MAX_TURN_PROMPT_BYTES: usize = 64 * 1024 * 1024;

/// The Owner prompt one turn dispatched, kept for refused-input recovery.
#[derive(Debug)]
pub(super) struct TurnPrompt {
    pub(super) command_id: String,
    pub(super) cmid: Option<String>,
    pub(super) content: Vec<serde_json::Value>,
    bytes: usize,
}

/// Whether a dispatched prompt may come back as a Draft: the Owner's, with a
/// non-text block (the only refusal so far, `image_answer`, needs an image).
fn recoverable(entry: &InFlightPrompt) -> bool {
    entry.origin.as_ref().is_some_and(DeliveryOrigin::is_owner)
        && entry.content.iter().any(|block| {
            block
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind != "text")
        })
}

fn payload_bytes(content: &[serde_json::Value]) -> usize {
    content
        .iter()
        .flat_map(|block| ["data", "text"].map(|key| block.get(key)))
        .filter_map(|value| value.and_then(serde_json::Value::as_str))
        .map(str::len)
        .sum()
}

#[derive(Clone, Debug)]
pub(super) struct InFlightPrompt {
    pub(super) session_id: String,
    pub(super) turn_id: String,
    pub(super) cmid: Option<String>,
    pub(super) content: Vec<serde_json::Value>,
    pub(super) origin: Option<DeliveryOrigin>,
    pub(super) initiator: Option<DeliveryOrigin>,
}

#[derive(Default)]
pub(super) struct InFlightPrompts {
    entries: HashMap<String, InFlightPrompt>,
    order: VecDeque<String>,
    /// `(session_id, turn_id)` -> the recoverable prompt dispatched for that
    /// turn, kept past its echo and taken once by the turn's `TurnEnded`.
    turns: HashMap<(String, String), TurnPrompt>,
    turn_order: VecDeque<(String, String)>,
    turn_bytes: usize,
}

impl InFlightPrompts {
    pub(super) fn record(&mut self, command_id: String, entry: InFlightPrompt) {
        if self.entries.contains_key(&command_id) {
            self.order.retain(|id| id != &command_id);
        }
        while self.order.len() >= MAX_IN_FLIGHT_PROMPTS {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.order.push_back(command_id.clone());
        let bytes = payload_bytes(&entry.content);
        if recoverable(&entry) && bytes <= MAX_TURN_PROMPT_BYTES {
            let turn = (entry.session_id.clone(), entry.turn_id.clone());
            self.take_turn(&turn.0, &turn.1);
            self.turn_bytes += bytes;
            self.turns.insert(
                turn.clone(),
                TurnPrompt {
                    command_id: command_id.clone(),
                    cmid: entry.cmid.clone(),
                    content: entry.content.clone(),
                    bytes,
                },
            );
            self.turn_order.push_back(turn);
            while self.turn_order.len() > MAX_TURN_PROMPTS
                || self.turn_bytes > MAX_TURN_PROMPT_BYTES
            {
                let Some(oldest) = self.turn_order.front().cloned() else {
                    break;
                };
                self.take_turn(&oldest.0, &oldest.1);
            }
        }
        self.entries.insert(command_id, entry);
    }

    /// The recoverable prompt this Controller dispatched for a turn, once.
    pub(super) fn take_turn(&mut self, session_id: &str, turn_id: &str) -> Option<TurnPrompt> {
        let key = (session_id.to_owned(), turn_id.to_owned());
        let entry = self.turns.remove(&key)?;
        self.turn_order.retain(|turn| turn != &key);
        self.turn_bytes -= entry.bytes;
        Some(entry)
    }

    /// A command rejected, parked, requeued or returned undelivered never ran
    /// as its turn: its recovery copy goes with it, so a late ending cannot
    /// park the same prompt a second time.
    pub(super) fn take_command(&mut self, command_id: &str) -> Option<InFlightPrompt> {
        let entry = self.entries.remove(command_id)?;
        self.order.retain(|id| id != command_id);
        if self
            .turns
            .get(&(entry.session_id.clone(), entry.turn_id.clone()))
            .is_some_and(|turn| turn.command_id == command_id)
        {
            self.take_turn(&entry.session_id, &entry.turn_id);
        }
        Some(entry)
    }

    pub(super) fn take_matching(
        &mut self,
        command_id: &str,
        session_id: &str,
        cmid: Option<&str>,
    ) -> Option<InFlightPrompt> {
        let matches = self
            .entries
            .get(command_id)
            .is_some_and(|entry| entry.session_id == session_id && entry.cmid.as_deref() == cmid);
        if !matches {
            return None;
        }
        self.take_command(command_id)
    }

    pub(super) fn retire_command(&mut self, command_id: &str) {
        let _ = self.take_command(command_id);
    }

    pub(super) fn retire_cmid(&mut self, session_id: &str, cmid: &str) {
        let matching_keys: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                entry.session_id == session_id && entry.cmid.as_deref() == Some(cmid)
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in matching_keys {
            self.entries.remove(&key);
            self.order.retain(|id| id != &key);
        }
    }

    pub(super) fn retire_turn(&mut self, session_id: &str, turn_id: &str) {
        let matching_keys: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.session_id == session_id && entry.turn_id == turn_id)
            .map(|(key, _)| key.clone())
            .collect();
        for key in matching_keys {
            self.entries.remove(&key);
            self.order.retain(|id| id != &key);
        }
    }

    #[cfg(test)]
    pub(super) fn turn_of(&self, command_id: &str) -> Option<String> {
        self.entries
            .get(command_id)
            .map(|entry| entry.turn_id.clone())
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.turns.clear();
        self.turn_order.clear();
        self.turn_bytes = 0;
    }
}

/// Non-owner provenance implied by a Controller-reserved `cmid` prefix, used
/// when a Controller-owned command's dispatch row was evicted from the bounded
/// in-memory Hub cache or when handling a post-restart fallback for a reserved
/// non-owner dispatch.
pub(super) fn reserved_origin_for_cmid(cmid: &str) -> Option<DeliveryOrigin> {
    if crate::agent_model::is_reserved_call_notice_cmid(cmid)
        || cmid.starts_with(crate::core::MANAGED_CALL_CMID_PREFIX)
    {
        Some(DeliveryOrigin::cowboy("managed_call"))
    } else if cmid.starts_with(crate::agent_model::PEER_NOTICE_PREFIX) {
        Some(DeliveryOrigin::cowboy("session_notice"))
    } else if cmid.starts_with("cowboy-superagent:") {
        Some(DeliveryOrigin::cowboy("superagent"))
    } else {
        None
    }
}

/// Build an [`InFlightPrompt`] from Hub provenance outside runtime locks so
/// callers can insert it into `shared.in_flight_prompts` (a leaf mutex) right
/// before inserting into `shared.pending`.
pub(super) fn prepare_sent_prompt(
    hub: &crate::core::Hub,
    session_id: &str,
    turn_id: &str,
    cmid: Option<&str>,
    content: &[serde_json::Value],
) -> InFlightPrompt {
    let (origin, initiator) = provenance_from_hub_or_reserved(hub, session_id, cmid);
    InFlightPrompt {
        session_id: session_id.to_owned(),
        turn_id: turn_id.to_owned(),
        cmid: cmid.map(str::to_owned),
        content: content.to_vec(),
        origin,
        initiator,
    }
}

/// Retire tracked in-flight prompts once the worker confirms delivery via a
/// matching `user_message_chunk` echo or finishes the turn via `TurnEnded`.
///
/// Intentionally does NOT retire on `TurnStarted`: the worker emits
/// `TurnStarted` before forwarding the prompt to the ACP thread, and cold-start
/// salvage emits `UndeliveredPrompt` after `TurnStarted`.
pub(super) fn observe_worker_event(shared: &Shared, session_id: &str, event: &RuntimeEvent) {
    match event {
        RuntimeEvent::TurnEnded {
            turn_id,
            stop_reason,
            ..
        } => {
            let dispatched = {
                let mut prompts = shared.in_flight_prompts.lock();
                prompts.retire_turn(session_id, turn_id);
                prompts.take_turn(session_id, turn_id)
            };
            // The Owner's prompt the agent refused before running it (the
            // proxy's proven rejection) returns to Drafts as the Owner wrote
            // it: without the context Cowboy generated at dispatch.
            if let Some(prompt) = dispatched
                && crate::account_controls::is_prompt_rejection(stop_reason)
            {
                let content: Vec<serde_json::Value> = crate::session_context::without_preamble(
                    &shared.hub,
                    session_id,
                    prompt.cmid.as_deref(),
                    prompt.content,
                )
                .into_iter()
                .filter(|block| !crate::context_rollover::is_renewal_block(block))
                .collect();
                shared.hub.restore_rejected_prompt(session_id, content);
            }
        }
        RuntimeEvent::Update {
            update,
            cmid: Some(id),
        } if update
            .get("sessionUpdate")
            .and_then(serde_json::Value::as_str)
            == Some("user_message_chunk") =>
        {
            shared.in_flight_prompts.lock().retire_cmid(session_id, id);
        }
        _ => {}
    }
}

/// Look up provenance for a Controller-owned `CoreCommand::Prompt` (from
/// `shared.pending`, never worker-authored) when rejected or parked on recovery.
pub(super) fn provenance_for_controller_command(
    shared: &Shared,
    session_id: &str,
    command_id: &str,
    cmid: Option<&str>,
) -> (Option<DeliveryOrigin>, Option<DeliveryOrigin>) {
    if let Some(tracked) = shared.in_flight_prompts.lock().take_command(command_id)
        && tracked.session_id == session_id
    {
        return (tracked.origin, tracked.initiator);
    }
    provenance_from_hub_or_reserved(&shared.hub, session_id, cmid)
}

pub(super) fn provenance_from_hub_or_reserved(
    hub: &crate::core::Hub,
    session_id: &str,
    cmid: Option<&str>,
) -> (Option<DeliveryOrigin>, Option<DeliveryOrigin>) {
    cmid.and_then(|id| hub.prompt_dispatch_record(session_id, id))
        .map(|record| (Some(record.origin), Some(record.trigger)))
        .or_else(|| {
            cmid.and_then(reserved_origin_for_cmid)
                .map(|origin| (Some(origin.clone()), Some(origin)))
        })
        .unwrap_or((None, None))
}

fn join_content_text(content: &[serde_json::Value]) -> String {
    content
        .iter()
        .filter_map(|value| value.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join(
            "
",
        )
}

/// Handle a worker-reported `RuntimeEvent::UndeliveredPrompt` (non-`call-notice:`).
///
/// - If `(command_id, session_id, cmid)` matches an un-echoed entry this
///   Controller actually sent, consume that entry and requeue the Controller's
///   OWN `content` and recorded `(origin, initiator)` — ignoring worker text.
/// - Otherwise, if `cmid` is an un-echoed hydrated pending echo (e.g. across a
///   Controller restart), consume it once and:
///   - for `cowboy-superagent:`, abandon without requeue (matching
///     `Hub::requeue_prompt`'s no-replay supervision contract);
///   - for `call-notice:`, drop (call notices use durable `notice_recovery`);
///   - for other reserved prefixes (`managed-call:`, `session-notice:`), keep
///     the reserved `cmid` with its non-owner `reserved_origin_for_cmid`;
///   - for unreserved `cmid`s, downgrade to `Cowboy("worker_requeue")` and
///     strip `cmid = None` so worker-supplied text can never mint Owner
///     provenance or collide with immutable records.
/// - Otherwise (already echoed, forged, or replayed), drop without requeuing.
pub(super) fn handle_undelivered_prompt(
    shared: &Shared,
    session_id: &str,
    command_id: &str,
    cmid: Option<&str>,
    content: Vec<serde_json::Value>,
    error: String,
) {
    if !shared.hub.accepts_runtime_projection(session_id) {
        return;
    }
    if let Some(tracked) = shared
        .in_flight_prompts
        .lock()
        .take_matching(command_id, session_id, cmid)
    {
        if tracked.cmid.as_deref().is_some_and(|id| {
            shared.hub.is_cmid_already_echoed(session_id, id)
                || shared.hub.is_cmid_parked_or_fenced(session_id, id)
        }) {
            tracing::warn!(
                session = %session_id,
                %command_id,
                "dropping UndeliveredPrompt for already-echoed or parked-draft dispatch"
            );
            return;
        }
        let text = join_content_text(&tracked.content);
        shared.hub.requeue_prompt(
            session_id,
            text,
            tracked.content,
            tracked.cmid,
            tracked.origin,
            tracked.initiator,
        );
        shared
            .hub
            .broadcast_error(Some(session_id.to_owned()), error);
        return;
    }
    handle_unverified_undelivered_prompt(&shared.hub, session_id, cmid, content, error);
}

/// Fail-closed fallback for an `UndeliveredPrompt` without a matching
/// Controller-held in-flight command (post-restart or direct `apply_event`).
pub(super) fn handle_unverified_undelivered_prompt(
    hub: &crate::core::Hub,
    session_id: &str,
    cmid: Option<&str>,
    content: Vec<serde_json::Value>,
    error: String,
) {
    let Some(id) = cmid else {
        tracing::warn!(
            session = %session_id,
            "dropping unverified UndeliveredPrompt without cmid"
        );
        return;
    };
    if hub
        .take_pending_echo_for_undelivered(session_id, id)
        .is_none()
    {
        tracing::warn!(
            session = %session_id,
            cmid = %id,
            "dropping unverified or already-echoed UndeliveredPrompt"
        );
        return;
    }
    if id.starts_with("cowboy-superagent:") {
        hub.record_session_error(
            session_id,
            &format!("Super-agent delivery returned without replay: {id}"),
        );
        hub.abandon_superagent_prompt(session_id, id);
        hub.broadcast_error(Some(session_id.to_owned()), error);
        return;
    }
    if crate::agent_model::is_reserved_call_notice_cmid(id) {
        tracing::warn!(
            session = %session_id,
            cmid = %id,
            "dropping unverified call-notice UndeliveredPrompt outside notice_recovery"
        );
        return;
    }
    let text = join_content_text(&content);
    if let Some(reserved_origin) = reserved_origin_for_cmid(id) {
        hub.requeue_prompt(
            session_id,
            text,
            content,
            Some(id.to_owned()),
            Some(reserved_origin.clone()),
            Some(reserved_origin),
        );
    } else {
        let downgraded = DeliveryOrigin::cowboy("worker_requeue");
        hub.requeue_prompt(
            session_id,
            text,
            content,
            None,
            Some(downgraded.clone()),
            Some(downgraded),
        );
    }
    hub.broadcast_error(Some(session_id.to_owned()), error);
}
