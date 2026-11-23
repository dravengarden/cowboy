//! Controller-authoritative prompt/steer dispatch origin tracking on the Hub.
//!
//! Keeps an in-memory bounded cache of `PromptDispatchRecord` per session
//! (hydrated from `prompt_dispatches` on startup and updated before every
//! handoff). Echoes from workers are authoritatively stamped from these
//! records so legacy or untrusted workers can never mint Owner turns.

use super::*;
use crate::prompt_origin::{
    DeliveryOrigin, PromptDispatchRecord, apply_authoritative_prompt_origin,
    is_owner_chunk_with_dispatches, newest_owner_turn_with_dispatches,
};

const MAX_SESSION_DISPATCH_ORIGINS: usize = 512;
pub(super) const DISPATCH_CMID_PREFIX: &str = "cowboy-dispatch:";

pub(super) fn ensure_dispatch_cmid(cmid: Option<String>) -> String {
    match cmid {
        Some(value) if !value.is_empty() => value,
        _ => format!("{DISPATCH_CMID_PREFIX}{}", uuid::Uuid::new_v4().simple()),
    }
}

pub(super) fn note_dispatch_origin(
    s: &mut Session,
    session_id: &str,
    cmid: &str,
    origin: DeliveryOrigin,
    trigger: DeliveryOrigin,
    dispatched_at_ms: i64,
) {
    if s.dispatched_origins.contains_key(cmid) {
        return;
    }
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
    /// Record an authoritative dispatch origin in the Hub's per-session cache.
    /// Immutable once recorded for a given `(session_id, cmid)`.
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
        note_dispatch_origin(s, session_id, &cmid, origin, trigger, dispatched_at_ms);
    }

    /// Seed the Hub's per-session dispatch caches from durable `prompt_dispatches`
    /// rows loaded during Controller startup.
    pub fn hydrate_prompt_dispatches(&self, records: Vec<PromptDispatchRecord>) {
        let mut sessions = self.inner.sessions.lock();
        for record in records {
            if let Some(s) = sessions.get_mut(&record.session_id) {
                note_dispatch_origin(
                    s,
                    &record.session_id,
                    &record.cmid,
                    record.origin,
                    record.trigger,
                    record.dispatched_at_ms,
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

    /// Authoritatively stamp `promptOrigin` on a `user_message_chunk` from
    /// Controller dispatch truth, overriding any worker-guessed cmid label.
    /// Multi-block prompts carry `cmid` only on block 0; trailing blocks inherit
    /// `s.active_echo_origin` across contiguous `user_message_chunk` events.
    pub(super) fn annotate_dispatched_prompt(
        s: &mut Session,
        event: &mut Event,
        cmid: Option<&str>,
        from_worker: bool,
    ) {
        let Event::Update { update } = event else {
            s.active_echo_origin = None;
            return;
        };
        if update
            .get("sessionUpdate")
            .and_then(serde_json::Value::as_str)
            != Some("user_message_chunk")
        {
            s.active_echo_origin = None;
            return;
        }
        // Preserve runtime-injected `<system-reminder>` / review prompts.
        if update
            .pointer("/promptOrigin/actor")
            .and_then(serde_json::Value::as_str)
            == Some("agent")
        {
            s.active_echo_origin = None;
            return;
        }
        if let Some(cmid) = cmid {
            if let Some(record) = s.dispatched_origins.get(cmid) {
                let origin = record.origin.clone();
                apply_authoritative_prompt_origin(update, &origin);
                s.active_echo_origin = Some(origin);
                return;
            }
            s.active_echo_origin = None;
            if from_worker {
                let unverified = DeliveryOrigin::cowboy("unverified_worker");
                apply_authoritative_prompt_origin(update, &unverified);
            }
            return;
        }
        // Continuation block (`cmid == None`) of a multi-block dispatched prompt.
        if let Some(origin) = s.active_echo_origin.clone()
            && s.log.last().is_some_and(is_user_message_chunk)
        {
            apply_authoritative_prompt_origin(update, &origin);
            return;
        }
        s.active_echo_origin = None;
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
