//! The per-turn call budget (docs/managed-agent-calls.md#budgets).
//!
//! A turn starts at the latest prompt a person sent to the parent session.
//! Cowboy completion notices, call results, peer-session messages, scheduled
//! wakeups and agent injections are user-role prompts too, but they never
//! start a turn: otherwise a notice that wakes the agent into one more call
//! would hand it a fresh budget each time. The turn is named by the event
//! sequence of that prompt; each admitted call is stamped with it
//! (`Choice::owner_turn`) and the budget counts the calls sharing it.

use std::time::Duration;

use serde_json::{Value, json};

use super::error;
use crate::core::{Envelope, Hub};
use crate::managed_calls::service::OWNER_TURN_NONE;
use crate::store::Store;

use super::super::super::agent_tools::CallsPolicy;

/// Durable history is read this many events per page, newest first.
const PAGE: usize = 200;
/// Looking for the owner prompt behind a very long turn may not hold an
/// admission longer than this.
const SCAN_DEADLINE: Duration = Duration::from_secs(3);

#[cfg(test)]
mod tests;

/// The sequence of the newest owner turn in `log`, as a ledger stamp.
fn newest_owner_prompt(log: &[Envelope]) -> Option<i64> {
    crate::prompt_origin::newest_owner_turn(log).and_then(|seq| i64::try_from(seq).ok())
}

/// The owner turn a call admitted now counts against: the newest owner
/// prompt of `parent`, from the Hub's retained log (which holds events the
/// store has not written yet, whatever a client bootstrap shows) or else from
/// durable history, or [`OWNER_TURN_NONE`] when the whole history has none (a
/// session only automation drives is one turn for its lifetime). `Err` names
/// why the turn could not be established; the caller refuses rather than
/// guess.
async fn current_turn(hub: &Hub, store: &Store, parent: &str) -> Result<i64, &'static str> {
    let scan = hub
        .newest_in_retained_log(parent, crate::prompt_origin::newest_owner_turn)
        .ok_or("session_unavailable")?;
    if let Some(turn) = scan.newest_match.and_then(|seq| i64::try_from(seq).ok()) {
        return Ok(turn);
    }
    if scan.whole {
        return Ok(OWNER_TURN_NONE);
    }
    durable_turn(store, parent, scan.first_seq.unwrap_or(i64::MAX as u64)).await
}

/// The newest owner prompt among the events before `before` in durable
/// history, read newest first.
async fn durable_turn(store: &Store, parent: &str, mut before: u64) -> Result<i64, &'static str> {
    let deadline = tokio::time::Instant::now() + SCAN_DEADLINE;
    loop {
        let (page, next, done) =
            tokio::time::timeout_at(deadline, store.history_page(parent, before, PAGE))
                .await
                .map_err(|_| "owner_turn_scan_timeout")?
                .map_err(|_| "owner_turn_history_unavailable")?;
        if let Some(turn) = newest_owner_prompt(&page) {
            return Ok(turn);
        }
        if done {
            return Ok(OWNER_TURN_NONE);
        }
        before = next
            .filter(|next| *next < before)
            .ok_or("owner_turn_history_unavailable")?;
    }
}

/// Refuse a call past the parent's limits, or name the owner turn it counts
/// against. Both limits are soft: concurrent admissions may pass them together.
/// Nothing is recorded by a refusal (`admission: not_submitted`).
pub(super) async fn admit(
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
        refusal["error"]["retryable"] = json!(true);
        refusal
    })?;
    let used = store
        .managed_call_turn_count(parent, turn)
        .await
        .map_err(|_| unavailable())?;
    if used >= u64::from(policy.max_per_turn) {
        return Err(error("call_limit", "not_submitted"));
    }
    Ok(turn)
}
