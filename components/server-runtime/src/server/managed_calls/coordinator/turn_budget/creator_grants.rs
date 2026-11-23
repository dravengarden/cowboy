//! Call-budget turns a creator grants the sessions it created
//! (docs/managed-agent-calls.md#call-budgets).
//!
//! A session that another session created through the Agent API
//! (`sessions.create`) and then drives only with `queue.submit` never gets a
//! prompt from a person, so its whole life would be one turn. Its creator's
//! message may start a turn instead, but only the first such message in each
//! turn of the creator: the authority stays the owner's prompt at the root of
//! the chain, and a creator and child messaging each other cannot mint
//! budgets. A creator that is itself a created session grants under its own
//! current turn, so the chain recurses to an owner prompt or grants nothing.
//!
//! The sender is the Agent API caller the Controller authenticated, never the
//! message text. A grant is taken only once the message was accepted, and
//! is named by the child's newest event sequence then, so it never depends
//! on finding the message's echo again: a later owner prompt has a larger
//! sequence and wins, and a grant that shares a sequence with an earlier turn
//! joins it, which only ever spends budget. Anything that cannot be established grants
//! nothing: the budget then counts as it did without grants.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::Hub;
use crate::core::settings_keys::{
    SESSION_CREATED_BY_PREFIX, SESSION_SPAWNED_FROM_PREFIX, SESSION_TURN_GRANTS_PREFIX,
};
use crate::managed_calls::service::OWNER_TURN_NONE;
use crate::store::Store;

/// Serializes grant writes, so each one reads the last one written and the
/// store receives them in order.
static UPDATES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The latest turn a session's creator granted it
/// (`session_turn_grants:<id>`).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Grant {
    /// The creator's own turn it was granted in. A creator turn grants at
    /// most once, and only a newer one grants again.
    creator_turn: i64,
    /// The granted turn: the session's newest event sequence when the grant
    /// was taken, as a ledger stamp.
    turn: i64,
}

/// What the stored setting says.
enum Stored {
    None,
    Grant(Grant),
    /// Present but not readable by this Controller: no turn, no new grant.
    Unreadable,
}

fn grants_key(session: &str) -> String {
    format!("{SESSION_TURN_GRANTS_PREFIX}{session}")
}

fn stored(value: Option<&Value>) -> Stored {
    match value.filter(|value| !value.is_null()) {
        None => Stored::None,
        Some(value) => {
            serde_json::from_value(value.clone()).map_or(Stored::Unreadable, Stored::Grant)
        }
    }
}

/// The turn the creator last granted `session`, or [`OWNER_TURN_NONE`].
pub(super) fn granted_turn(hub: &Hub, session: &str) -> i64 {
    match stored(hub.setting(&grants_key(session)).as_ref()) {
        Stored::Grant(grant) => grant.turn,
        Stored::None | Stored::Unreadable => OWNER_TURN_NONE,
    }
}

/// Record that `creator`'s agent created `child`, wherever it was filed.
pub fn record_creator(hub: &Hub, creator: &str, child: &str) {
    hub.set_setting(
        format!("{SESSION_CREATED_BY_PREFIX}{child}"),
        creator.into(),
    );
}

/// Write the creator link to the store and wait for it, so a restart keeps
/// it even if the write-behind queue had not reached it yet. A failed write is
/// retried a few times; after that the write-behind queue and the next grant
/// (which writes the link again first) remain.
pub async fn persist_creator(store: Option<&Store>, creator: &str, child: &str) {
    let Some(store) = store else {
        return;
    };
    let key = format!("{SESSION_CREATED_BY_PREFIX}{child}");
    for delay_ms in [100, 400, 1_600, 0] {
        match store.put_setting(&key, &creator.into()).await {
            Ok(()) => return,
            Err(error) if delay_ms == 0 => {
                tracing::warn!(%child, %error, "creator link left to the write-behind queue");
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await,
        }
    }
}

/// The session whose agent created `child`, with the setting that says so.
/// Sessions created before the creator was recorded have it only when they
/// were filed beside it.
fn creator_of(hub: &Hub, child: &str) -> Option<(String, String)> {
    [SESSION_CREATED_BY_PREFIX, SESSION_SPAWNED_FROM_PREFIX]
        .into_iter()
        .find_map(|prefix| {
            let key = format!("{prefix}{child}");
            let creator = hub.setting(&key)?.as_str()?.to_owned();
            Some((key, creator))
        })
}

/// A message `sender` is about to send a session it created, with the turn
/// of its own it would grant under.
pub struct Pending {
    child: String,
    creator_turn: i64,
    /// The creator link the grant relies on, written before the grant so a
    /// restart never keeps a grant without it.
    link: (String, String),
}

/// Whether the message `sender` is about to send `child` may grant it a turn:
/// `sender` created `child` and has a current turn of its own.
pub async fn prepare(
    hub: &Hub,
    store: Option<&Store>,
    sender: &str,
    child: &str,
) -> Option<Pending> {
    let link = creator_of(hub, child)?;
    if sender == child || link.1 != sender {
        return None;
    }
    let creator_turn = super::current_turn(hub, store?, sender).await.ok()?;
    (creator_turn != OWNER_TURN_NONE).then(|| Pending {
        child: child.to_owned(),
        creator_turn,
        link,
    })
}

impl Pending {
    /// Grant the turn once the message was taken (`receipt`), unless the
    /// record already holds this creator turn or a newer one. The grant is
    /// written to the store before it counts, so a restart never forgets a
    /// grant a call was admitted under; a failed write grants nothing.
    pub async fn commit(self, hub: &Hub, store: Option<&Store>, receipt: &Value) {
        let Some(store) = store.filter(|_| taken(receipt)) else {
            return;
        };
        let _ordered = UPDATES.lock().await;
        if hub.session_info(&self.child).is_none() {
            return;
        }
        let key = grants_key(&self.child);
        match stored(hub.setting(&key).as_ref()) {
            Stored::Unreadable => return,
            Stored::Grant(grant) if grant.creator_turn >= self.creator_turn => return,
            Stored::None | Stored::Grant(_) => {}
        }
        // The session's sequence only grows, so the granted turn is at least
        // as new as any turn it already had.
        let Some(turn) = hub
            .last_event_seq(&self.child)
            .and_then(|seq| i64::try_from(seq).ok())
        else {
            return;
        };
        let grant = Grant {
            creator_turn: self.creator_turn,
            turn,
        };
        let Ok(value) = serde_json::to_value(grant) else {
            return;
        };
        let (link_key, creator) = &self.link;
        if store
            .put_setting(link_key, &creator.as_str().into())
            .await
            .is_err()
            || store.put_setting(&key, &value).await.is_err()
        {
            return;
        }
        hub.with_settings_mut(|settings| Hub::commit_setting_locked(settings, key.clone(), value));
        // A target deleted meanwhile tombstoned its settings before this
        // grant was committed: remove what this grant wrote. Deleted after
        // this check, its own tombstones remove it.
        if hub.session_info(&self.child).is_none() {
            for key in [key, link_key.clone()] {
                hub.set_setting(key, Value::Null);
            }
        }
    }
}

/// Whether a dispatched `queue.submit` receipt says its message was taken:
/// queued or injected, or for an interrupt (an HTTP receipt) submitted.
/// Anything else (refused, pending, unknown) grants nothing; the creator's
/// next message in the same turn may.
fn taken(receipt: &Value) -> bool {
    if receipt.get("http_status").is_some() {
        return receipt["result"]["status"] == "submitted";
    }
    matches!(receipt["status"].as_str(), Some("accepted" | "injected"))
}

#[cfg(test)]
#[path = "creator_grants_tests.rs"]
mod tests;
