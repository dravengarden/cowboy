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

#[cfg(test)]
mod tests;
