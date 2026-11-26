//! The per-turn call budget (docs/managed-agent-calls.md#budgets).
//!
//! A turn starts at the latest prompt a person sent to the parent session.
//! Cowboy completion notices, call results, peer-session messages, scheduled
//! wakeups and agent injections are user-role prompts too, but they never
//! start a turn: otherwise a notice that wakes the agent into one more call
//! would hand it a fresh budget each time. The turn is named by the event
//! sequence of that prompt; each admitted call is stamped with it
//! (`Choice::owner_turn`) and the budget counts the calls sharing it. A
//! session another session created through the Agent API may also get a turn
//! from its creator, at most once per turn of the creator
//! ([`creator_grants`]).

use std::time::Duration;

use serde_json::{Value, json};

use super::error;
use crate::core::settings_keys::SESSION_OWNER_TURN_LOST_PREFIX;
use crate::core::{Envelope, Hub};
use crate::managed_calls::service::OWNER_TURN_NONE;
use crate::store::Store;

use super::super::super::agent_tools::CallsPolicy;

/// Durable history is read this many events per page, newest first.
const PAGE: usize = 200;
/// Looking for the owner prompt behind a very long turn may not hold an
/// admission longer than this.
const SCAN_DEADLINE: Duration = Duration::from_secs(3);

pub mod creator_grants;
#[cfg(test)]
mod tests;

/// The sequence of the newest owner turn in `log`, as a ledger stamp.
#[cfg(test)]
fn newest_owner_prompt(log: &[Envelope]) -> Option<i64> {
    crate::prompt_origin::newest_owner_turn(log).and_then(|seq| i64::try_from(seq).ok())
}

/// The turn a call admitted now in `session` counts against: the newer of
/// its owner turn and the turn its creator last granted it
/// ([`creator_grants`]). Without a grant this is the owner turn alone.
pub(in crate::server) async fn current_turn(
    hub: &Hub,
    store: &Store,
    session: &str,
) -> Result<i64, &'static str> {
    let granted = creator_grants::granted_turn(hub, session);
    let owner = owner_turn(hub, store, session, granted).await?;
    // A transcript cleared before its owner turn was found: no owner prompt
    // or grant since then means the turn is unknown, never an older one.
    let lost = hub
        .setting(&format!("{SESSION_OWNER_TURN_LOST_PREFIX}{session}"))
        .and_then(|seq| seq.as_i64());
    if owner == OWNER_TURN_NONE && lost.is_some_and(|cleared| granted < cleared) {
        return Err(TURN_CLEARED);
    }
    Ok(owner.max(granted))
}

/// Why a session whose history was cleared before its owner turn was known
/// refuses calls until a person or its creator sends it a new message.
const TURN_CLEARED: &str = "owner_turn_history_cleared";

/// The newest owner prompt of `session`, from the Hub's retained log (which
/// holds events the store has not written yet, whatever a client bootstrap
/// shows) or else from durable history, or [`OWNER_TURN_NONE`] when the whole
/// history has none (a session only automation drives is one turn for its
/// lifetime). Durable history is read only down to `newer` (a granted turn):
/// an owner prompt that old loses to it anyway. `Err` names why the turn could
/// not be established; the caller refuses rather than guess.
async fn owner_turn(
    hub: &Hub,
    store: &Store,
    session: &str,
    newer: i64,
) -> Result<i64, &'static str> {
    let scan = hub
        .newest_owner_turn_in_retained_log(session)
        .ok_or("session_unavailable")?;
    if let Some(turn) = scan.newest_match {
        return Ok(stamp(turn));
    }
    if scan.whole {
        return Ok(OWNER_TURN_NONE);
    }
    // From the retained log's first event on, so a send it cut is found whole.
    let before = scan.first_seq.map_or(i64::MAX as u64, |first| first + 1);
    let floor = u64::try_from(newer).ok();
    let turn = durable_turn(store, session, before, floor).await?;
    // A Clear while the history was read removed what was read: its result,
    // a fragment of a cleared send or an absence, means nothing now.
    let after = hub
        .newest_owner_turn_in_retained_log(session)
        .ok_or("session_unavailable")?;
    if after.whole {
        return Ok(after.newest_match.map_or(OWNER_TURN_NONE, stamp));
    }
    match u64::try_from(turn) {
        Ok(seq) => hub.remember_owner_turn(session, seq),
        // Read to its start: a later Clear loses no owner turn.
        Err(_) if floor.is_none() => hub.confirm_no_owner_turn(session),
        Err(_) => {}
    }
    Ok(turn)
}

fn stamp(seq: u64) -> i64 {
    i64::try_from(seq).unwrap_or(i64::MAX)
}

/// The newest owner prompt among the events before `before` and after
/// `floor` in durable history, read newest first. A send whose first chunks
/// are on an older page is anchored at its real first chunk, however the
/// pages fall; one that began at or before `floor` counts as none.
async fn durable_turn(
    store: &Store,
    parent: &str,
    mut before: u64,
    floor: Option<u64>,
) -> Result<i64, &'static str> {
    let deadline = tokio::time::Instant::now() + SCAN_DEADLINE;
    let mut run: Vec<Envelope> = Vec::new();
    loop {
        // A send still open from a newer page needs its older page read.
        if run.is_empty() && floor.is_some_and(|floor| before <= floor.saturating_add(1)) {
            return Ok(OWNER_TURN_NONE);
        }
        let (mut page, next, done) =
            tokio::time::timeout_at(deadline, store.history_page(parent, before, PAGE))
                .await
                .map_err(|_| "owner_turn_scan_timeout")?
                .map_err(|_| "owner_turn_history_unavailable")?;
        page.append(&mut run);
        let cmids: Vec<String> = page.iter().filter_map(|e| e.cmid.clone()).collect();
        let dispatches =
            tokio::time::timeout_at(deadline, store.prompt_dispatches_by_cmids(parent, &cmids))
                .await
                .map_err(|_| "owner_turn_scan_timeout")?
                .map_err(|_| "owner_turn_history_unavailable")?;
        let found =
            crate::prompt_origin::newest_owner_turn_with_dispatches(&page, |c| dispatches.get(c))
                .filter(|seq| floor.is_none_or(|floor| *seq > floor));
        // A run that begins the page may have begun on the page before it.
        let open = found.is_some_and(|seq| page.first().is_some_and(|first| first.seq == seq));
        match found {
            Some(turn) if !open || done => return Ok(stamp(turn)),
            Some(_) => run = page,
            None if done => return Ok(OWNER_TURN_NONE),
            None => {}
        }
        before = next
            .filter(|next| *next < before)
            .ok_or("owner_turn_history_unavailable")?;
    }
}

/// Refuse a call past the parent's limits, or name the owner turn it counts
/// against. Both limits are soft: concurrent admissions may pass them together.
/// Nothing is recorded by a refusal (`admission: not_submitted`).
pub async fn admit(
    hub: &Hub,
    store: Option<&Store>,
    parent: &str,
    policy: &CallsPolicy,
) -> Result<i64, Value> {
    let unavailable = || error("capacity", "not_submitted");
    let store = store.ok_or_else(unavailable)?;
    let (active, _) = store
        .managed_call_counts(parent)
        .await
        .map_err(|_| unavailable())?;
    if active >= u64::from(policy.max_concurrent) {
        return Err(error("concurrency_limit", "not_submitted"));
    }
    let turn = current_turn(hub, store, parent).await.map_err(|reason| {
        let mut refusal = unavailable();
        refusal["error"]["reason"] = json!(reason);
        refusal["error"]["retryable"] = json!(reason != TURN_CLEARED);
        refusal
    })?;
    let used = store
        .managed_call_turn_count(parent, turn)
        .await
        .map_err(|_| unavailable())?;
    if used >= u64::from(policy.max_per_turn) {
        return Err(call_limit(policy.max_per_turn, used));
    }
    Ok(turn)
}

/// The refusal of a call past the turn's budget, with what an agent needs to
/// report it accurately: the limit, the calls this turn already admitted and
/// how the budget comes back.
fn call_limit(limit: u16, used: u64) -> Value {
    let mut refusal = error("call_limit", "not_submitted");
    refusal["error"]["limit"] = json!(limit);
    refusal["error"]["used"] = json!(used);
    refusal["error"]["retryable"] = json!(false);
    refusal["error"]["resets_on"] = json!("owner_prompt");
    refusal["error"]["message"] = json!(format!(
        "this turn already admitted {used} of its {limit} managed calls; nothing was submitted"
    ));
    refusal["error"]["hint"] = json!(
        "the budget resets when the session's owner sends a new message (for a session another \
         session created, also its creator's first message in a new turn); notices and \
         results do not reset it. Only the owner can raise it: Settings > Tools > Calls per \
         turn for the agent kind, or this session's Tools (1-256). Agents cannot raise their \
         own limit (tools_widening_denied). Do not retry in this turn; report the skipped call."
    );
    refusal
}
