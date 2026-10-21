//! A continuation remains beside its source in both sidebar orderings.
use super::Hub;
use crate::session_folders::FolderActor;
use serde_json::json;

impl Hub {
    pub(crate) fn continuation_folder(
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
    pub(crate) fn place_continuation(
        &self,
        source: &str,
        destination: &str,
        inherit_settings: bool,
        actor: &FolderActor,
    ) -> Result<(), String> {
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
            if let Some(directory) = review.get("directory") {
                self.sync_apply_as_locked(
                    actor,
                    &format!("mobile-review:{destination}"),
                    format!("continuation-review:{destination}"),
                    "setDirectory",
                    &json!({"directory":directory}),
                )?;
            }
        }
        self.rename_session(destination, source_meta.title);
        Ok(())
    }
}

/// Match sessionProjectLabel in the Web sidebar, including legacy checkouts.
fn sidebar_project(meta: &super::SessionMeta) -> Option<&str> {
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
