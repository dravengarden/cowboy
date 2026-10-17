use super::*;
use crate::session_folders::FolderActor;

pub(super) fn actor(owner: &str) -> FolderActor {
    FolderActor {
        user_id: Some(owner.into()),
        sees_all: false,
    }
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
    let tree = state.hub.sync_value("folders");
    let folders = tree["folders"].as_array().ok_or("unavailable")?;
    let folders = folders
        .iter()
        .filter(|f| f["owner_user_id"].as_str() == Some(owner))
        .collect::<Vec<_>>();
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
