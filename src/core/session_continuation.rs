//! A continuation remains beside its source in both sidebar orderings.
use super::Hub;
use crate::session_folders::FolderActor;
use serde_json::json;

impl Hub {
    pub fn continuation_folder(
        &self,
        source: &str,
        actor: &FolderActor,
    ) -> Result<Option<Option<String>>, String> {
        let source = self.session_info(source).ok_or("Source unavailable")?.meta;
        if !actor.sees_all && source.owner_user_id != actor.user_id {
            return Err("Invalid continuation ownership".into());
        }
        self.inner
            .folders
            .lock()
            .effective_folder(&source.id, actor, sidebar_project(&source))
    }

    /// The caller has admitted a new, fenced destination. All sidebar changes
    /// share the normal sync gate so a concurrent move cannot split the snapshot.
    pub fn place_continuation(
        &self,
        source: &str,
        destination: &str,
        inherit_settings: bool,
        actor: &FolderActor,
    ) -> Result<(), String> {
        let title = self.place_beside(source, destination, inherit_settings, actor)?;
        self.set_setting(
            format!(
                "{}{destination}",
                super::settings_keys::SESSION_CONTINUED_FROM_PREFIX
            ),
            json!(source),
        );
        self.set_continued_from(destination, Some(source.to_owned()));
        self.rename_session(destination, title);
        Ok(())
    }

    /// Place an admitted fork beside its source, like a continuation, under
    /// the source title marked as a fork.
    pub fn place_fork(
        &self,
        source: &str,
        destination: &str,
        actor: &FolderActor,
    ) -> Result<(), String> {
        let title = self.place_beside(source, destination, true, actor)?;
        self.rename_session(destination, format!("{title} (fork)"));
        Ok(())
    }

    /// Place an admitted copy (a new agent that reads its source as linked
    /// context) beside its source, under the source title marked as a copy.
    /// Unlike a continuation the source stays the active conversation.
    pub fn place_copy(
        &self,
        source: &str,
        destination: &str,
        actor: &FolderActor,
    ) -> Result<(), String> {
        let title = self.place_beside(source, destination, true, actor)?;
        let title = if title.ends_with(" (copy)") {
            title
        } else {
            format!("{title} (copy)")
        };
        self.rename_session(destination, title);
        Ok(())
    }

    /// Folder, both sidebar orders and the review directory; returns the
    /// source title.
    fn place_beside(
        &self,
        source: &str,
        destination: &str,
        inherit_settings: bool,
        actor: &FolderActor,
    ) -> Result<String, String> {
        let _guard = self.inner.sync_apply_gate.lock();
        let source_meta = self.session_info(source).ok_or("Source unavailable")?.meta;
        let target = self
            .session_info(destination)
            .ok_or("Destination unavailable")?
            .meta;
        if source == destination
            || source_meta.owner_user_id != target.owner_user_id
            || source_meta.closing
            || target.closing
            || source_meta.system
            || target.system
        {
            return Err("Invalid continuation ownership".into());
        }
        let folder = self.continuation_folder(source, actor)?;
        if let Some(folder) = folder {
            self.sync_apply_as_locked(
                actor,
                "folders",
                format!("continuation-folder:{destination}"),
                "place",
                &json!({"session_ids":[destination], "folder":folder}),
            )?;
        }

        // The session wire order is oldest-first; both Web sidebars reverse it.
        // Mixed workspace order already uses display order. Work in display order
        // throughout so filling in unranked rows never reverses the user's list.
        let mut display_order: Vec<_> = self.inner.order.lock().iter().rev().cloned().collect();
        insert_after(&mut display_order, source, destination);
        let order: Vec<_> = display_order.iter().rev().cloned().collect();
        self.sync_apply_as_locked(
            actor,
            "order",
            format!("continuation-order:{destination}"),
            "reorder",
            &json!({"order":order}),
        )?;

        let owner = actor.user_id.as_deref().unwrap_or_default();
        let previous = self
            .inner
            .workspace_orders
            .lock()
            .get(owner)
            .cloned()
            .unwrap_or_default();
        // Unranked rows lead in the sidebar. Materialize unranked sessions in
        // their existing order before the explicit order, retaining every draft.
        // Unranked drafts remain unranked and therefore still lead this list.
        let ranked: std::collections::HashSet<_> = previous.iter().map(String::as_str).collect();
        let owned: std::collections::HashSet<_> = self
            .inner
            .sessions
            .lock()
            .iter()
            .filter(|(_, session)| session.meta.owner_user_id == actor.user_id)
            .map(|(id, _)| id.clone())
            .collect();
        let mut mixed: Vec<_> = display_order
            .iter()
            .filter(|id| owned.contains(*id))
            .map(|id| format!("session:{id}"))
            .filter(|key| !ranked.contains(key.as_str()))
            .collect();
        mixed.extend(previous);
        insert_after(
            &mut mixed,
            &format!("session:{source}"),
            &format!("session:{destination}"),
        );
        self.sync_apply_as_locked(
            actor,
            "workspace-order",
            format!("continuation-workspace:{destination}"),
            "reorder",
            &json!({"order":mixed}),
        )?;
        if inherit_settings {
            let review = self.sync_value(&format!("mobile-review:{source}"));
            // "This session's worktree" means the destination's own worktree;
            // copying the source reference would browse the old files.
            let own = serde_json::json!(format!("session::{source}"));
            if let Some(directory) = review.get("directory").map(|directory| {
                if *directory == own {
                    serde_json::json!(format!("session::{destination}"))
                } else {
                    directory.clone()
                }
            }) {
                self.sync_apply_as_locked(
                    actor,
                    &format!("mobile-review:{destination}"),
                    format!("continuation-review:{destination}"),
                    "setDirectory",
                    &json!({"directory":directory}),
                )?;
            }
        }
        Ok(source_meta.title)
    }
}

/// Match sessionProjectLabel in the Web sidebar, including legacy checkouts.
pub(super) fn sidebar_project(meta: &super::SessionMeta) -> Option<&str> {
    meta.workspace_name
        .as_deref()
        .into_iter()
        .chain(meta.workspace_id.as_deref())
        .map(str::trim)
        .find(|value| !value.is_empty())
        .or_else(|| {
            meta.workspace_source_path
                .as_deref()
                .into_iter()
                .chain(Some(meta.cwd.as_str()))
                .filter_map(|path| path.split_once("/columbus/projects/"))
                .filter_map(|(_, path)| path.split('/').next())
                .find(|value| !value.is_empty())
        })
}

fn insert_after(order: &mut Vec<String>, source: &str, destination: &str) {
    order.retain(|id| id != destination);
    let at = order
        .iter()
        .position(|id| id == source)
        .map_or(order.len(), |index| index + 1);
    order.insert(at, destination.to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{SessionOrigin, SessionRegistration};

    fn session(hub: &Hub, id: &str, owner: &str) {
        hub.create_session(SessionRegistration {
            id: id.into(),
            provider: "claude-code".into(),
            provider_account_id: Default::default(),
            provider_version: "fixture".into(),
            provider_generation_digest: "fixture".into(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "hawk".into(),
            workspace_id: Some("suger".into()),
            workspace_name: Some("suger".into()),
            workspace_source_path: None,
            execution_binding: None,
            cwd: format!("/worktrees/{id}"),
            title: format!("Task {id}"),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: Some(owner.into()),
            owner_username: None,
        });
    }

    #[test]
    fn continuation_stays_next_to_its_source_and_preserves_other_rows_and_runtime() {
        let hub = Hub::new();
        for id in ["source", "other", "target", "unranked", "unranked-new"] {
            session(&hub, id, "me");
        }
        session(&hub, "foreign", "them");
        let actor = FolderActor {
            user_id: Some("me".into()),
            sees_all: false,
        };
        hub.sync_apply_as(
            &actor,
            "folders",
            "create".into(),
            "create",
            &json!({"id":"brazil","name":"Brazil 2P"}),
        )
        .unwrap();
        hub.sync_apply_as(
            &actor,
            "folders",
            "place".into(),
            "place",
            &json!({"session_ids":["source"],"folder":"brazil"}),
        )
        .unwrap();
        hub.sync_apply_as(
            &actor,
            "order",
            "order".into(),
            "reorder",
            &json!({"order":["unranked","source","foreign","other","unranked-new","target"]}),
        )
        .unwrap();
        hub.sync_apply_as(
            &actor,
            "workspace-order",
            "mixed".into(),
            "reorder",
            &json!({"order":["session:source","draft:notes","session:other"]}),
        )
        .unwrap();
        hub.sync_apply_as(
            &actor,
            "mobile-review:source",
            "directory".into(),
            "setDirectory",
            &json!({"directory":"project"}),
        )
        .unwrap();
        hub.set_paused("source", true);
        let original = serde_json::to_value(hub.session_info("source").unwrap()).unwrap();
        let target_cwd = hub.session_info("target").unwrap().meta.cwd;
        hub.place_continuation("source", "target", true, &actor)
            .unwrap();
        assert_eq!(hub.sync_value("folders")["placement"]["target"], "brazil");
        assert_eq!(
            hub.sync_value("order"),
            json!([
                "unranked",
                "target",
                "source",
                "foreign",
                "other",
                "unranked-new"
            ])
        );
        assert_eq!(
            hub.sync_value("workspace-order")["me"],
            json!([
                "session:unranked-new",
                "session:unranked",
                "session:source",
                "session:target",
                "draft:notes",
                "session:other"
            ])
        );
        assert_eq!(
            hub.sync_value("mobile-review:target")["directory"],
            "project"
        );
        let target = hub.session_info("target").unwrap();
        assert_eq!(target.meta.title, "Task source");
        assert_eq!(target.meta.cwd, target_cwd);
        assert!(!target.meta.paused);
        assert!(target.meta.agent_session_id.is_none());
        assert_eq!(
            serde_json::to_value(hub.session_info("source").unwrap()).unwrap(),
            original
        );
        let before = hub.sync_value("folders");
        assert!(
            hub.place_continuation("foreign", "target", true, &actor)
                .is_err()
        );
        assert_eq!(hub.sync_value("folders"), before);
    }

    #[test]
    fn a_self_referencing_browse_directory_follows_the_new_worktree() {
        let hub = Hub::new();
        for id in ["source", "target", "pinned", "third", "next"] {
            session(&hub, id, "me");
        }
        let actor = FolderActor {
            user_id: Some("me".into()),
            sees_all: false,
        };
        for (source, directory) in [("source", "session::source"), ("pinned", "session::third")] {
            hub.sync_apply_as(
                &actor,
                &format!("mobile-review:{source}"),
                "directory".into(),
                "setDirectory",
                &json!({ "directory": directory }),
            )
            .unwrap();
        }
        hub.place_continuation("source", "target", true, &actor)
            .unwrap();
        assert_eq!(
            hub.sync_value("mobile-review:target")["directory"],
            "session::target"
        );
        assert_eq!(
            hub.session_info("target")
                .unwrap()
                .meta
                .continued_from
                .as_deref(),
            Some("source")
        );
        // The mark is a persisted setting, projected again after a restart.
        let key = format!(
            "{}target",
            super::super::settings_keys::SESSION_CONTINUED_FROM_PREFIX
        );
        let restored = Hub::new();
        session(&restored, "target", "me");
        restored.load_settings(vec![(key.clone(), hub.setting(&key).unwrap())]);
        restored.restore_continuations();
        assert_eq!(
            restored
                .session_info("target")
                .unwrap()
                .meta
                .continued_from
                .as_deref(),
            Some("source")
        );
        // An explicit choice of another session remains that choice.
        hub.place_continuation("pinned", "next", true, &actor)
            .unwrap();
        assert_eq!(
            hub.sync_value("mobile-review:next")["directory"],
            "session::third"
        );
    }

    #[test]
    fn earlier_continuations_are_marked_from_their_source_receipt() {
        let hub = Hub::new();
        for id in ["source", "target", "linked"] {
            session(&hub, id, "me");
        }
        let receipt = |session: &str| {
            json!({"revision": 3, "applied_revision": 3, "delivery": {
                "operation": "op", "fingerprint": "", "session_id": session,
                "state": "accepted"}})
        };
        hub.load_settings(vec![
            (crate::session_context::key("source"), receipt("target")),
            // Linking context to the same agent is not a continuation.
            (crate::session_context::key("linked"), receipt("linked")),
        ]);
        hub.restore_continuations();
        let mark = |id: &str| hub.session_info(id).unwrap().meta.continued_from;
        assert_eq!(mark("target").as_deref(), Some("source"));
        assert_eq!(mark("linked"), None);
        assert_eq!(mark("source"), None);
        // The recorded mark survives a later send from the same source.
        hub.load_settings(vec![(
            crate::session_context::key("source"),
            receipt("source"),
        )]);
        hub.restore_continuations();
        assert_eq!(mark("target").as_deref(), Some("source"));
    }

    #[test]
    fn project_bound_folder_and_explicit_global_survive_new_agent_creation() {
        let hub = Hub::new();
        for id in ["source", "target", "second"] {
            session(&hub, id, "me");
        }
        // Old sessions can still auto-file by their stable checkout path.
        {
            let mut sessions = hub.inner.sessions.lock();
            let meta = &mut sessions.get_mut("source").unwrap().meta;
            meta.workspace_name = Some(" ".into());
            meta.workspace_id = None;
            meta.cwd = "/home/owner/columbus/projects/suger".into();
        }
        let actor = FolderActor {
            user_id: Some("me".into()),
            sees_all: false,
        };
        hub.sync_apply_as(
            &actor,
            "folders",
            "create".into(),
            "create",
            &json!({"id":"suger-folder","name":"Suger","project":"suger"}),
        )
        .unwrap();
        hub.place_continuation("source", "target", false, &actor)
            .unwrap();
        assert_eq!(
            hub.sync_value("workspace-order")["me"],
            json!(["session:second", "session:source", "session:target"])
        );
        assert_eq!(
            hub.sync_value("folders")["placement"]["target"],
            "suger-folder"
        );
        hub.sync_apply_as(
            &actor,
            "folders",
            "global".into(),
            "place",
            &json!({"session_ids":["source"],"folder":null}),
        )
        .unwrap();
        hub.place_continuation("source", "second", false, &actor)
            .unwrap();
        assert_eq!(hub.sync_value("folders")["placement"]["second"], "");
        assert!(
            hub.sync_value("mobile-review:second")
                .get("directory")
                .is_none()
        );
    }

    #[test]
    fn legacy_visible_folder_uses_real_actor_and_unplaced_stays_unplaced() {
        let hub = Hub::new();
        for id in ["source", "target", "unplaced", "empty-target"] {
            session(&hub, id, "me");
        }
        let legacy = FolderActor {
            user_id: None,
            sees_all: true,
        };
        hub.sync_apply_as(
            &legacy,
            "folders",
            "legacy-create".into(),
            "create",
            &json!({"id":"legacy", "name":"Old folder"}),
        )
        .unwrap();
        let owner = FolderActor {
            user_id: Some("me".into()),
            sees_all: true,
        };
        hub.sync_apply_as(
            &owner,
            "folders",
            "legacy-place".into(),
            "place",
            &json!({"session_ids":["source"],"folder":"legacy"}),
        )
        .unwrap();
        let restricted = FolderActor {
            user_id: Some("me".into()),
            sees_all: false,
        };
        assert!(hub.continuation_folder("source", &restricted).is_err());
        hub.place_continuation("source", "target", true, &owner)
            .unwrap();
        assert_eq!(hub.sync_value("folders")["placement"]["target"], "legacy");
        assert_eq!(hub.continuation_folder("unplaced", &owner).unwrap(), None);
        hub.place_continuation("unplaced", "empty-target", true, &owner)
            .unwrap();
        assert!(
            hub.sync_value("folders")["placement"]
                .get("empty-target")
                .is_none()
        );
    }
}
