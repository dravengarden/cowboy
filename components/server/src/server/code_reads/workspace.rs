//! Resolve a continuous authenticated Workspace observation against persisted
//! enrollment. A saved inventory alone never supplies a live read identity.

use crate::core::CodeReadScope;
use crate::machine_control::MachineControl;
use crate::machine_protocol::MachineWorkspace;
use crate::store::Store;

pub(in crate::server) async fn resolve(
    store: &Store,
    control: &MachineControl,
    service_id: &str,
    machine_id: &str,
    workspace_id: &str,
) -> Option<CodeReadScope> {
    let machines = store.list_machines().await.ok()?;
    let machine = machines
        .into_iter()
        .find(|machine| machine.id == machine_id && !machine.revoked)?;
    let workspaces: Vec<MachineWorkspace> = machine
        .inventory
        .get("workspaces")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())?;
    let mut matching = workspaces
        .iter()
        .filter(|workspace| workspace.id == workspace_id);
    let workspace = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    // Capture only AFTER the durable read, and require agreement with the live
    // original connection. DB/cache snapshots cannot recreate an ended scope.
    let scope = control.workspace_code_scope(service_id, machine_id, workspace_id)?;
    (scope.cwd() == workspace.canonical_path).then_some(CodeReadScope::Workspace(scope))
}

/// A worktree of a registered workspace, read through that workspace's live
/// observation. Only a colocated Controller admits one, since it is the party
/// that can check the path; a Machine adapter refuses roots it did not export.
pub(in crate::server) async fn resolve_worktree(
    store: &Store,
    control: &MachineControl,
    service_id: &str,
    machine_id: &str,
    workspace_id: &str,
    path: &str,
) -> Option<CodeReadScope> {
    let CodeReadScope::Workspace(parent) =
        resolve(store, control, service_id, machine_id, workspace_id).await?
    else {
        return None;
    };
    if let Some(scope) = control.known_worktree_scope(&parent, path) {
        return Some(CodeReadScope::Workspace(scope));
    }
    if !control.workspace_scope_is_colocated(&parent).ok()? {
        return None;
    }
    let root = std::path::PathBuf::from(parent.cwd());
    let candidate = std::path::PathBuf::from(path);
    let member = tokio::task::spawn_blocking(move || {
        crate::code_review::is_repository_worktree(&root, &candidate)
    })
    .await
    .ok()?;
    member
        .then(|| control.admit_worktree_scope(&parent, path))
        .flatten()
        .map(CodeReadScope::Workspace)
}

#[cfg(test)]
mod tests;
