//! Controller-authoritative prompt/steer dispatch origin tracking on the Hub.
//!
//! Keeps an in-memory bounded cache of `PromptDispatchRecord` per session
//! (hydrated from `prompt_dispatches` on startup and updated before every
//! handoff), plus a single-use `pending_echoes` set with exact dispatched block
//! counts so untrusted or legacy workers can never replay historical Owner
//! cmids or append forged untagged Owner chunks.

use super::*;
use crate::prompt_origin::{
    DeliveryOrigin, PromptDispatchRecord, apply_authoritative_prompt_origin,
    is_owner_chunk_with_dispatches, newest_owner_turn_with_dispatches,
};

const MAX_SESSION_DISPATCH_ORIGINS: usize = 512;
const MAX_PENDING_ECHOES: usize = 64;
/// Upper bound on continuation blocks for a single prompt that was already in
/// flight across a Controller restart (whose exact block count was not saved).
const HYDRATED_PENDING_ECHO_MAX_BLOCKS: usize = 16;
pub(super) const DISPATCH_CMID_PREFIX: &str = "cowboy-dispatch:";

pub(super) fn ensure_dispatch_cmid(cmid: Option<String>) -> String {
    match cmid {
        Some(value) if !value.is_empty() => value,
        _ => format!("{DISPATCH_CMID_PREFIX}{}", uuid::Uuid::new_v4().simple()),
    }
}

fn note_pending_echo(s: &mut Session, cmid: &str, origin: DeliveryOrigin, expected_blocks: usize) {
    let blocks = expected_blocks.max(1);
    if let Some((existing_origin, existing_blocks)) = s.pending_echoes.get_mut(cmid) {
        if *existing_origin != origin {
            return;
        }
        *existing_blocks = (*existing_blocks).max(blocks);
        return;
    }
    while s.pending_echo_order.len() >= MAX_PENDING_ECHOES {
        if let Some(oldest) = s.pending_echo_order.pop_front() {
            s.pending_echoes.remove(&oldest);
        }
    }
    s.pending_echo_order.push_back(cmid.to_owned());
    s.pending_echoes.insert(cmid.to_owned(), (origin, blocks));
}

pub(super) fn remove_pending_echo(s: &mut Session, cmid: &str) {
    if s.pending_echoes.remove(cmid).is_some() {
        s.pending_echo_order.retain(|id| id != cmid);
    }
}

fn note_dispatch_origin(
    s: &mut Session,
    session_id: &str,
    cmid: &str,
    origin: DeliveryOrigin,
    trigger: DeliveryOrigin,
    dispatched_at_ms: i64,
    expected_blocks: usize,
) {
    if let Some(existing) = s.dispatched_origins.get_mut(cmid) {
        if existing.origin != origin {
            // Fail closed: never arm or mutate a conflicting provenance entry.
            return;
        }
        existing.dispatched_at_ms = existing.dispatched_at_ms.max(dispatched_at_ms);
        let kept_origin = existing.origin.clone();
        note_pending_echo(s, cmid, kept_origin, expected_blocks);
        return;
    }
    note_pending_echo(s, cmid, origin.clone(), expected_blocks);
    while s.dispatched_origin_order.len() >= MAX_SESSION_DISPATCH_ORIGINS {
        if let Some(oldest) = s.dispatched_origin_order.pop_front() {
            s.dispatched_origins.remove(&oldest);
        }
    }
    s.dispatched_origin_order.push_back(cmid.to_owned());
    s.dispatched_origins.insert(
        cmid.to_owned(),
        PromptDispatchRecord {
            session_id: session_id.to_owned(),
            cmid: cmid.to_owned(),
            origin,
            trigger,
            dispatched_at_ms,
        },
    );
}

impl Hub {
    /// Record an authoritative dispatch origin in the Hub's per-session cache
    /// and arm its single-use pending worker echo. Called only AFTER durable
    /// provenance validation succeeds.
    pub fn record_prompt_dispatch(
        &self,
        session_id: &str,
        cmid: String,
        origin: DeliveryOrigin,
        trigger: DeliveryOrigin,
        dispatched_at_ms: i64,
    ) {
        let mut sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get_mut(session_id) else {
            return;
        };
        note_dispatch_origin(s, session_id, &cmid, origin, trigger, dispatched_at_ms, 1);
    }

    /// Disarm a pending worker echo when a dispatch or steer fails before
    /// handoff so refused attempts never leave stale echo authorizations.
    pub fn disarm_pending_echo(&self, session_id: &str, cmid: &str) {
        let mut sessions = self.inner.sessions.lock();
        if let Some(s) = sessions.get_mut(session_id) {
            remove_pending_echo(s, cmid);
        }
    }

    /// Whether `cmid` is currently parked as a draft in this session.
    #[must_use]
    pub fn is_cmid_parked_as_draft(&self, session_id: &str, cmid: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|s| s.drafts.iter().any(|m| m.cmid.as_deref() == Some(cmid)))
    }

    /// Whether `cmid` has already echoed into this session's transcript.
    #[must_use]
    pub fn is_cmid_already_echoed(&self, session_id: &str, cmid: &str) -> bool {
        let sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get(session_id) else {
            return false;
        };
        s.dispatched_cmids
            .iter()
            .any(|seen| seen.cmid == cmid && seen.echoed)
            || s.log.iter().any(|env| env.cmid.as_deref() == Some(cmid))
    }

    /// Consume a single-use pending echo authorization when an unverified
    /// (e.g. post-restart) `UndeliveredPrompt` arrives for an un-echoed
    /// dispatch. Returns `None` if already echoed or not armed.
    pub fn take_pending_echo_for_undelivered(
        &self,
        session_id: &str,
        cmid: &str,
    ) -> Option<DeliveryOrigin> {
        let mut sessions = self.inner.sessions.lock();
        let s = sessions.get_mut(session_id)?;
        let echoed = s
            .dispatched_cmids
            .iter()
            .any(|seen| seen.cmid == cmid && seen.echoed)
            || s.log.iter().any(|env| env.cmid.as_deref() == Some(cmid));
        let parked_as_draft = s.drafts.iter().any(|m| m.cmid.as_deref() == Some(cmid));
        if echoed || parked_as_draft {
            remove_pending_echo(s, cmid);
            return None;
        }
        let (origin, _) = s.pending_echoes.remove(cmid)?;
        s.pending_echo_order.retain(|id| id != cmid);
        Some(origin)
    }

    /// Record the exact number of content blocks (including any prepended
    /// session-context preamble) handed to a worker for `(session_id, cmid)` so
    /// continuation-chunk stamping is strictly bounded to that count.
    pub fn set_pending_echo_blocks(&self, session_id: &str, cmid: &str, blocks: usize) {
        let mut sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get_mut(session_id) else {
            return;
        };
        let blocks = blocks.max(1);
        if let Some((_, existing_blocks)) = s.pending_echoes.get_mut(cmid) {
            *existing_blocks = blocks;
        } else if let Some(record) = s.dispatched_origins.get(cmid) {
            let origin = record.origin.clone();
            note_pending_echo(s, cmid, origin, blocks);
        }
    }

    /// Seed the Hub's per-session dispatch caches from durable
    /// `prompt_dispatches` rows loaded during Controller startup.
    ///
    /// Historical dispatches are placed in `dispatched_origins` for history
    /// scans and Retry lookups, NOT in `pending_echoes` (so a compromised
    /// worker cannot replay historical Owner cmids). Only the single newest
    /// un-echoed hydrated record at the session's latest `dispatched_at_ms` is
    /// armed in `pending_echoes`, provided its timestamp neither predates a
    /// trimmed hot log nor predates a newer echoed user turn.
    pub fn hydrate_prompt_dispatches(&self, records: Vec<PromptDispatchRecord>) {
        let mut sessions = self.inner.sessions.lock();
        let mut per_session: HashMap<String, Vec<PromptDispatchRecord>> = HashMap::new();
        for record in records {
            if let Some(s) = sessions.get_mut(&record.session_id) {
                if !s.dispatched_origins.contains_key(&record.cmid) {
                    while s.dispatched_origin_order.len() >= MAX_SESSION_DISPATCH_ORIGINS {
                        if let Some(oldest) = s.dispatched_origin_order.pop_front() {
                            s.dispatched_origins.remove(&oldest);
                        }
                    }
                    s.dispatched_origin_order.push_back(record.cmid.clone());
                    s.dispatched_origins
                        .insert(record.cmid.clone(), record.clone());
                }
                per_session
                    .entry(record.session_id.clone())
                    .or_default()
                    .push(record);
            }
        }
        for (session_id, rows) in per_session {
            let Some(s) = sessions.get_mut(&session_id) else {
                continue;
            };
            let Some(max_ts) = rows.iter().map(|r| r.dispatched_at_ms).max() else {
                continue;
            };
            let trimmed_floor = (!s.reached_start)
                .then_some(s.restored_last_event_at_ms)
                .flatten();
            let candidate = rows
                .into_iter()
                .rev()
                .filter(|r| r.dispatched_at_ms == max_ts)
                .find(|r| {
                    !s.log
                        .iter()
                        .any(|env| env.cmid.as_deref() == Some(r.cmid.as_str()))
                });
            if let Some(record) = candidate
                && trimmed_floor.is_none_or(|floor| record.dispatched_at_ms >= floor)
            {
                note_pending_echo(
                    s,
                    &record.cmid,
                    record.origin,
                    HYDRATED_PENDING_ECHO_MAX_BLOCKS,
                );
            }
        }
    }

    /// Look up the Controller-recorded dispatch origin for `(session_id, cmid)`.
    #[must_use]
    pub fn prompt_dispatch_origin(&self, session_id: &str, cmid: &str) -> Option<DeliveryOrigin> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .and_then(|s| s.dispatched_origins.get(cmid))
            .map(|record| record.origin.clone())
    }

    /// Look up the full recorded dispatch record for `(session_id, cmid)`.
    #[must_use]
    pub fn prompt_dispatch_record(
        &self,
        session_id: &str,
        cmid: &str,
    ) -> Option<PromptDispatchRecord> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .and_then(|s| s.dispatched_origins.get(cmid))
            .cloned()
    }

    /// The `cmid` of the session's newest `user_message_chunk` in the retained
    /// log, used by Retry to look up durable dispatch provenance before retrying.
    #[must_use]
    pub fn last_user_prompt_cmid(&self, session_id: &str) -> Option<String> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        last_user_prompt(&s.log).cmid
    }

    /// Like [`Self::newest_in_retained_log`], specialized for owner turns using
    /// Controller-recorded `prompt_dispatches` instead of cmid prefix guessing.
    #[must_use]
    pub fn newest_owner_turn_in_retained_log(&self, session_id: &str) -> Option<RetainedLogScan> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        let in_log = newest_owner_turn_with_dispatches(&s.log, |c| {
            s.dispatched_origins.get(c).map(|r| &r.origin)
        });
        let first_seq = s.log.first().map(|e| e.seq);
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

    /// Authoritatively stamp `promptOrigin` on a `user_message_chunk` from
    /// Controller dispatch truth, overriding any worker-guessed cmid label.
    ///
    /// For live worker frames (`from_worker == true`):
    /// - A `cmid`-bearing chunk is accepted ONLY if `cmid` is currently armed
    ///   in `s.pending_echoes` (consumed on first echo so historical Owner
    ///   cmids can never be replayed).
    /// - Trailing chunks are accepted ONLY while contiguous with the active
    ///   echo AND within the exact dispatched block count.
    /// - Any replayed, unknown, or excess chunk fails closed as
    ///   `Cowboy("unverified_worker")` and has its `cmid` stripped so durable
    ///   scans can never match a spoofed echo to a prior Owner dispatch.
    pub(super) fn annotate_dispatched_prompt(
        s: &mut Session,
        event: &mut Event,
        cmid: &mut Option<String>,
        from_worker: bool,
    ) {
        let Event::Update { update } = event else {
            s.active_echo = None;
            return;
        };
        if update
            .get("sessionUpdate")
            .and_then(serde_json::Value::as_str)
            != Some("user_message_chunk")
        {
            s.active_echo = None;
            return;
        }
        // Preserve runtime-injected `<system-reminder>` / review prompts.
        if update
            .pointer("/promptOrigin/actor")
            .and_then(serde_json::Value::as_str)
            == Some("agent")
        {
            s.active_echo = None;
            return;
        }
        if let Some(id) = cmid.as_deref() {
            if let Some((origin, expected_blocks)) = s.pending_echoes.remove(id) {
                s.pending_echo_order.retain(|seen| seen != id);
                apply_authoritative_prompt_origin(update, &origin);
                s.active_echo = Some((
                    Some(id.to_owned()),
                    origin,
                    expected_blocks.saturating_sub(1),
                ));
                return;
            }
            if s.log.last().is_some_and(is_user_message_chunk)
                && let Some((active_cmid, origin, remaining)) = s.active_echo.clone()
                && remaining > 0
                && active_cmid.as_deref() == Some(id)
            {
                apply_authoritative_prompt_origin(update, &origin);
                s.active_echo = Some((active_cmid, origin, remaining - 1));
                return;
            }
            if !from_worker && let Some(record) = s.dispatched_origins.get(id) {
                let origin = record.origin.clone();
                apply_authoritative_prompt_origin(update, &origin);
                s.active_echo = Some((
                    Some(id.to_owned()),
                    origin,
                    HYDRATED_PENDING_ECHO_MAX_BLOCKS,
                ));
                return;
            }
            s.active_echo = None;
            if from_worker {
                let unverified = DeliveryOrigin::cowboy("unverified_worker");
                apply_authoritative_prompt_origin(update, &unverified);
                *cmid = None;
            }
            return;
        }
        // Continuation block (`cmid == None`) of a multi-block dispatched prompt.
        if s.log.last().is_some_and(is_user_message_chunk)
            && let Some((active_cmid, origin, remaining)) = s.active_echo.clone()
            && remaining > 0
        {
            apply_authoritative_prompt_origin(update, &origin);
            s.active_echo = Some((active_cmid, origin, remaining - 1));
            return;
        }
        s.active_echo = None;
        if from_worker {
            let unverified = DeliveryOrigin::cowboy("unverified_worker");
            apply_authoritative_prompt_origin(update, &unverified);
        }
    }

    pub(super) fn is_owner_envelope_for_session(s: &Session, envelope: &Envelope) -> bool {
        is_owner_chunk_with_dispatches(envelope, |c| s.dispatched_origins.get(c).map(|r| &r.origin))
    }
}

#[cfg(test)]
#[path = "hub_dispatch_origin_tests.rs"]
mod tests;
