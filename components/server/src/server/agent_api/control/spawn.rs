//! `sessions.create`: the Agent API's additions to the product's creation
//! handler. The session is created exactly as the GUI would create it; its
//! title and its place in the Sessions list are applied afterwards, the way a
//! fork or copy is filed beside its source.
use super::*;
use crate::session_folders::FolderActor;

/// What a `sessions.create` call asked for beyond the product handler's body.
pub(super) struct Spawn {
    title: Option<String>,
    /// The caller named no folder, so the session is filed beside its creator.
    beside_creator: bool,
}

impl Spawn {
    /// Refuse a malformed body before the call is admitted, so the caller gets a
    /// definite rejection instead of a receipt of unknown effect. The body must
    /// also be one the product handler accepts.
    pub(super) fn check(input: &Value) -> Result<(), &'static str> {
        let mut probe = input.clone();
        Self::prepare(&mut probe)?;
        serde_json::from_value::<web::NewSessionRequest>(probe["body"].clone())
            .map(drop)
            .map_err(|_| "invalid_argument")
    }

    /// Agent creation follows the managed-worktree path of the GUI, so it can
    /// neither mark a session as a system session nor name its owner or origin.
    pub(super) fn prepare(input: &mut Value) -> Result<Self, &'static str> {
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
        if let Some(title) = self.title {
            // As `sessions.rename`, fork and copy set a title.
            hub.rename_session(session, title);
        }
        if !self.beside_creator {
            return;
        }
        if let Err(error) = hub.place_spawned(creator, session, actor) {
            tracing::warn!(%session, %creator, %error, "spawned session stays where it was created");
        }
    }
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;
