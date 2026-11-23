//! `sessions.create`: the Agent API's additions to the product's creation
//! handler. The session is created exactly as the GUI would create it; its
//! title and its place in the Sessions list are applied afterwards, the way a
//! fork or copy is filed beside its source.
use super::*;
use crate::session_folders::FolderActor;

use std::collections::HashMap;
use std::sync::Weak;
use tokio::sync::{Mutex, OwnedMutexGuard};

/// How long a creation waits for its creator's earlier one before going ahead
/// unordered: a stuck creation must not hold up every later spawn.
const TURN_WAIT: Duration = Duration::from_secs(30);

/// One lock per creator with a creation in flight; dead entries are dropped on
/// the next admission.
static TURNS: LazyLock<parking_lot::Mutex<HashMap<String, Weak<Mutex<()>>>>> =
    LazyLock::new(Default::default);

/// What a `sessions.create` call asked for beyond the product handler's body.
pub(super) struct Spawn {
    title: Option<String>,
    /// The caller named no folder, so the session is filed beside its creator.
    beside_creator: bool,
    /// Held until the session is filed, so one creator's spawns are filed in
    /// the order they were admitted however long each creation takes.
    _turn: Option<OwnedMutexGuard<()>>,
}

impl Spawn {
    /// Refuse a malformed body before the call is admitted, so the caller gets a
    /// definite rejection instead of a receipt of unknown effect.
    pub(super) fn check(input: &Value) -> Result<(), &'static str> {
        Self::parse(&mut input.clone()).map(drop)
    }

    /// Validate the body, strip the fields the product handler must not see and
    /// wait for this creator's turn.
    pub(super) async fn admit(input: &mut Value, creator: &str) -> Result<Self, &'static str> {
        let mut spawn = Self::parse(input)?;
        spawn._turn = turn(creator).await;
        Ok(spawn)
    }

    /// Agent creation follows the managed-worktree path of the GUI, so it can
    /// neither mark a session as a system session nor name its owner or origin.
    fn parse(input: &mut Value) -> Result<Self, &'static str> {
        let body = input
            .get_mut("body")
            .and_then(Value::as_object_mut)
            .ok_or("invalid_argument")?;
        if body.contains_key("system")
            || body.contains_key("owner_user_id")
            || body.contains_key("origin")
        {
            return Err("invalid_argument");
        }
        // The title follows `sessions.rename`: text, trimmed, never empty.
        let title = match body.remove("title") {
            None | Some(Value::Null) => None,
            Some(Value::String(title)) => {
                Some(crate::core::Hub::clean_title(&title).map_err(|_| "invalid_argument")?)
            }
            Some(_) => return Err("invalid_argument"),
        };
        // Any named folder, including "" for the top level, is the caller's
        // choice. Only a request that names none follows its creator.
        let beside_creator = body.get("folder_id").is_none_or(Value::is_null);
        body.insert("origin".into(), json!("web"));
        Ok(Self {
            title,
            beside_creator,
            _turn: None,
        })
    }

    /// Apply the title and the placement to the session the handler created.
    ///
    /// `result` is the write receipt of the creation. Neither step may turn a
    /// created session into an error, which would invite a retry that creates
    /// a duplicate: a creator in Trash or gone, or any refusal, leaves the
    /// session where the handler put it.
    pub(super) fn finish(
        self,
        hub: &crate::core::Hub,
        creator: &str,
        actor: &FolderActor,
        result: &Value,
    ) {
        let created = result["http_status"]
            .as_u64()
            .is_some_and(|status| (200..300).contains(&status))
            .then(|| result["result"]["session_id"].as_str())
            .flatten();
        let Some(session) = created else {
            return;
        };
        if let Some(title) = &self.title {
            // The synchronized title writer, as the Web's own rename uses, so a
            // client that already holds this session's title adopts the new one.
            let renamed = hub.sync_apply_as(
                actor,
                "title",
                format!("spawn-title:{session}"),
                "rename",
                &json!({"session_id": session, "title": title}),
            );
            if let Err(error) = renamed {
                tracing::warn!(%session, %error, "spawned session keeps its generated title");
            }
        }
        if !self.beside_creator {
            return;
        }
        if let Err(error) = hub.place_spawned(creator, session, actor) {
            tracing::warn!(%session, %creator, %error, "spawned session stays where it was created");
        }
    }
}

/// The creator's turn to create, or `None` once an earlier creation outlasts
/// [`TURN_WAIT`].
async fn turn(creator: &str) -> Option<OwnedMutexGuard<()>> {
    let lock = {
        let mut turns = TURNS.lock();
        turns.retain(|_, lock| lock.strong_count() > 0);
        let live = turns.get(creator).and_then(Weak::upgrade);
        live.unwrap_or_else(|| {
            let lock = Arc::new(Mutex::new(()));
            turns.insert(creator.to_owned(), Arc::downgrade(&lock));
            lock
        })
    };
    tokio::time::timeout(TURN_WAIT, lock.lock_owned())
        .await
        .ok()
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;
