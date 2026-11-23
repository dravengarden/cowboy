//! `sessions.create`: the Agent API's additions to the product's creation
//! handler. The session is created exactly as the GUI would create it; its
//! title and its place in the Sessions list are applied afterwards, the way a
//! fork or copy is filed beside its source.
use super::*;
use crate::server::managed_calls::creator_grants;
use crate::session_folders::FolderActor;

/// What a `sessions.create` call asked for beyond the product handler's body.
pub(super) struct Spawn {
    title: Option<String>,
    /// The caller named no folder, so the session is filed beside its creator.
    beside_creator: bool,
}

impl Spawn {
    /// [`Self::prepare`] for a `sessions.create`; nothing for another call.
    pub(super) fn prepare_for(name: &str, input: &mut Value) -> Result<Option<Self>, &'static str> {
        (name == "sessions.create")
            .then(|| Self::prepare(input))
            .transpose()
    }

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

    /// [`Self::place`], with the creator link written to the store before
    /// the creation's receipt is, so no grant ever waits for it.
    pub(super) async fn finish(
        self,
        state: &AppState,
        creator: &str,
        actor: &FolderActor,
        result: &Value,
    ) {
        if let Some(session) = self.place(&state.hub, creator, actor, result) {
            creator_grants::persist_creator(state.store.as_ref(), creator, &session).await;
        }
    }

    /// Apply the title and the placement to the session the handler created,
    /// and record its creator; returns the created session.
    ///
    /// `result` is the write receipt of the creation. Neither step may turn a
    /// created session into an error, which would invite a retry that creates
    /// a duplicate: a creator in Trash or gone, or any refusal, leaves the
    /// session where the handler put it.
    pub(super) fn place(
        self,
        hub: &crate::core::Hub,
        creator: &str,
        actor: &FolderActor,
        result: &Value,
    ) -> Option<String> {
        let created = result["http_status"]
            .as_u64()
            .is_some_and(|status| (200..300).contains(&status))
            .then(|| result["result"]["session_id"].as_str())
            .flatten();
        let session = created?;
        creator_grants::record_creator(hub, creator, session);
        if let Some(title) = self.title {
            // As `sessions.rename`, fork and copy set a title.
            hub.rename_session(session, title);
        }
        if self.beside_creator
            && let Err(error) = hub.place_spawned(creator, session, actor)
        {
            tracing::warn!(%session, %creator, %error, "spawned session stays where it was created");
        }
        Some(session.to_owned())
    }
}

/// A `queue.submit` from the session that created its target may start a
/// call-budget turn there (docs/managed-agent-calls.md#call-budgets): read
/// before the message is sent, granted once it was taken.
pub(super) async fn turn_grant(
    state: &AppState,
    sender: &str,
    name: &str,
    input: &Value,
) -> Option<creator_grants::Pending> {
    if name != "queue.submit" || !deliverable(input) {
        return None;
    }
    let child = input["session_id"].as_str()?;
    creator_grants::prepare(&state.hub, state.store.as_ref(), sender, child).await
}

/// Whether a `queue.submit` has something dispatch would prompt: text that
/// is not blank, or content blocks of which at least one decodes and is not
/// blank text, read with
/// dispatch's own precedence (content first). Anything else is dropped
/// before it is prompted and grants nothing.
fn deliverable(input: &Value) -> bool {
    let text = input["text"].as_str().unwrap_or_default();
    let content = input["content"].as_array().map_or(&[][..], Vec::as_slice);
    if content.is_empty() && text.trim().is_empty() {
        return false;
    }
    // Blank text blocks are not a message either.
    crate::server::client_commands::build_prompt_blocks(text, content).is_some_and(|blocks| {
        blocks.iter().any(|block| {
            serde_json::to_value(block).is_ok_and(|block| {
                block["type"] != "text"
                    || block["text"]
                        .as_str()
                        .is_some_and(|text| !text.trim().is_empty())
            })
        })
    })
}

/// Grant a [`turn_grant`] once its message's receipt says it was taken.
pub(super) async fn commit_turn_grant(
    state: &AppState,
    grant: Option<creator_grants::Pending>,
    receipt: &Value,
) {
    if let Some(grant) = grant {
        grant
            .commit(&state.hub, state.store.as_ref(), receipt)
            .await;
    }
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;
