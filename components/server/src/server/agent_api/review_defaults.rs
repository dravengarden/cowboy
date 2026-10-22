use super::*;
use crate::session_folders::FolderActor;

pub(super) fn actor(owner: &str) -> FolderActor {
    FolderActor {
        user_id: Some(owner.into()),
        sees_all: false,
    }
}

fn owned_tree(tree: Value, session: &str, owner: &str) -> Value {
    crate::session_folders::project_folders_value(
        tree,
        &std::collections::HashSet::from([session.to_owned()]),
        |folder_owner| folder_owner == Some(owner),
    )
}

pub(super) async fn read(
    state: &AppState,
    meta: &SessionMeta,
    owner: &str,
) -> Result<Value, &'static str> {
    let review = state.hub.sync_value(&format!("mobile-review:{}", meta.id));
    let own = review
        .get("directory")
        .map_or_else(|| json!({}), |v| json!({"directory":v}));
    let tree = owned_tree(state.hub.sync_value("folders"), &meta.id, owner);
    let folders = tree["folders"].as_array().ok_or("unavailable")?;
    let directories = folders.iter().map(|f| json!({"id":f["id"],"name":f["name"],"parent":f["parent"],"project":f["project"],
        "override":f["defaults"].get("review_directory").map_or_else(|| json!({}), |v| json!({"directory":v})),
    })).collect::<Vec<_>>();
    let machines = state
        .machine_snapshots
        .load()
        .await
        .map_err(|_| "unavailable")?;
    let projects = machines.iter().flat_map(|m| m.workspaces.iter().map(|w| json!({"directory":format!("workspace::{}::{}",m.id,w.id),"machine":m.display_name,"project":w.display_name,"path":w.canonical_path}))).collect::<Vec<_>>();
    Ok(
        json!({"scope":"zed","session_id":meta.id,"override":own,"placement":tree["placement"].get(&meta.id),
        "project":meta.workspace_name.as_ref().or(meta.workspace_id.as_ref()),"directories":directories,"projects":projects,
        "semantics":"Missing override inherits; null is Automatic; project follows the associated checkout; session::<id> follows that visible session worktree. Defaults never change Agent execution."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zed_read_projects_real_folder_wire_identity_without_leaking_other_owners() {
        let mut tree = crate::session_folders::SessionFolders::default();
        for owner in ["me", "other"] {
            tree.apply(
                &actor(owner),
                "create",
                &json!({"id":owner,"name":owner,"defaults":{"review_directory":"project"}}),
            )
            .unwrap();
        }
        tree.apply(&actor("me"), "create", &json!({"id":"child","name":"Child","parent":"me","defaults":{"review_directory":null}})).unwrap();
        tree.apply(
            &actor("me"),
            "place",
            &json!({"session_ids":["current","hidden"],"folder":"child"}),
        )
        .unwrap();
        let value = owned_tree(tree.value(), "current", "me");
        let folders = value["folders"].as_array().unwrap();
        assert_eq!(folders.len(), 2);
        assert!(
            folders
                .iter()
                .all(|f| f.get("owner").is_none() && f["id"] != "other")
        );
        assert_eq!(folders[0]["defaults"]["review_directory"], "project");
        assert_eq!(folders[1]["parent"], "me");
        assert_eq!(
            folders[1]["defaults"].get("review_directory"),
            Some(&Value::Null)
        );
        assert_eq!(value["placement"], json!({"current":"child"}));
    }
}
