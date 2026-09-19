//! Continuous, bounded observations of authenticated Machine workspace exports.
//! These are read identities, not filesystem proofs, grants or durable leases.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::{ConnectionToken, LiveState, MachineControl, RequestBinding};
use crate::code_adapter::{CodeAdapterRequest, CodeOperation};
use crate::machine_protocol::MachineWorkspace;

const MAX_WORKSPACES_PER_MACHINE: usize = 1024;
const MAX_WORKSPACES: usize = 4096;
const MAX_STRING_BYTES: usize = 8 * 1024 * 1024;

/// Only the authenticated registry can construct or renew this observation.
/// Cloning borrows the SAME incarnation; JSON and equal strings cannot mint it.
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceCodeScope(Arc<WorkspaceIdentity>);

#[derive(Debug)]
struct WorkspaceIdentity {
    connection: ConnectionToken,
    workspace_id: String,
    cwd: String,
}

impl PartialEq for WorkspaceCodeScope {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for WorkspaceCodeScope {}
impl Hash for WorkspaceCodeScope {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

impl WorkspaceCodeScope {
    pub(crate) fn machine_id(&self) -> &str {
        &self.0.connection.0.machine_id
    }

    pub(crate) fn cwd(&self) -> &str {
        &self.0.cwd
    }

    pub(crate) fn string_bytes(&self) -> usize {
        self.machine_id().len()
            + self.0.connection.0.epoch.len()
            + self.0.workspace_id.len()
            + self.cwd().len()
    }

    pub(super) fn matches(&self, live: &LiveState) -> bool {
        live.is_current(&self.0.connection)
            && live
                .workspace_inventory
                .get(self.machine_id())
                .and_then(|slots| slots.get(&self.0.workspace_id))
                == Some(self)
    }
}

impl LiveState {
    pub(super) fn observe_workspaces(
        &mut self,
        connection: &ConnectionToken,
        workspaces: &[MachineWorkspace],
    ) {
        let machine = &connection.0.machine_id;
        // Count before allocating a second map. A rejected observation ends the
        // old lifetimes, rather than retaining apparently-current stale roots.
        let mut count = workspaces.len();
        let mut bytes = workspaces.iter().fold(0_usize, |total, item| {
            total
                .saturating_add(item.id.len())
                .saturating_add(item.canonical_path.len())
                .saturating_add(connection.0.machine_id.len())
                .saturating_add(connection.0.epoch.len())
        });
        for (id, slots) in &self.workspace_inventory {
            if id != machine {
                count = count.saturating_add(slots.len());
                bytes = slots.values().fold(bytes, |total, scope| {
                    total.saturating_add(scope.string_bytes())
                });
            }
        }
        if workspaces.len() > MAX_WORKSPACES_PER_MACHINE
            || count > MAX_WORKSPACES
            || bytes > MAX_STRING_BYTES
        {
            self.workspace_inventory.remove(machine);
            return;
        }
        let mut unique = HashMap::new();
        for workspace in workspaces {
            unique
                .entry(workspace.id.as_str())
                .and_modify(|entry| *entry = None)
                .or_insert(Some(workspace));
        }
        let previous = self.workspace_inventory.get(machine);
        let slots = unique
            .into_iter()
            .filter_map(|(id, workspace)| {
                let workspace = workspace?;
                if id.is_empty()
                    || id.len() > 256
                    || id.contains('\0')
                    || id.contains("::")
                    || !workspace.canonical_path.starts_with('/')
                    || workspace.canonical_path.len() > 4096
                    || workspace.canonical_path.contains('\0')
                {
                    return None;
                }
                let scope = previous
                    .and_then(|slots| slots.get(id))
                    .filter(|scope| {
                        scope.cwd() == workspace.canonical_path
                            && scope.0.connection.same(connection)
                    })
                    .cloned()
                    .unwrap_or_else(|| {
                        WorkspaceCodeScope(Arc::new(WorkspaceIdentity {
                            connection: connection.clone(),
                            workspace_id: id.into(),
                            cwd: workspace.canonical_path.clone(),
                        }))
                    });
                Some((id.into(), scope))
            })
            .collect();
        self.workspace_inventory.insert(machine.clone(), slots);
    }
}

impl MachineControl {
    pub(crate) fn workspace_code_scope(
        &self,
        service: &str,
        machine: &str,
        workspace: &str,
    ) -> Option<WorkspaceCodeScope> {
        if self.service.as_str() != service {
            return None;
        }
        let live = self.live.read();
        let scope = live.workspace_inventory.get(machine)?.get(workspace)?;
        scope.matches(&live).then(|| scope.clone())
    }

    pub(crate) fn workspace_scope_is_current(&self, scope: &WorkspaceCodeScope) -> bool {
        scope.matches(&self.live.read())
    }

    /// No persisted/local-route fallback after the original observation ends.
    pub(crate) fn workspace_scope_is_colocated(
        &self,
        scope: &WorkspaceCodeScope,
    ) -> Result<bool, String> {
        let live = self.live.read();
        if !scope.matches(&live) {
            return Err("Workspace read scope ended".into());
        }
        Ok(live.connections[scope.machine_id()].colocated)
    }

    /// Only a typed Code operation is accepted. The original scope supplies the
    /// root, Machine, adapter and connection; the caller cannot substitute them.
    pub(crate) async fn code_request_in_workspace(
        &self,
        scope: &WorkspaceCodeScope,
        operation: CodeOperation,
    ) -> Result<serde_json::Value, String> {
        let request = serde_json::to_value(CodeAdapterRequest {
            root: scope.cwd().into(),
            operation,
        })
        .map_err(|_| "Workspace Code request encoding failed".to_owned())?;
        let response = self
            .adapter_request_bound(
                scope.machine_id(),
                "code",
                request,
                Some(RequestBinding::Workspace(scope)),
            )
            .await;
        // A reply can complete immediately before a remove/re-add or reconnect
        // while its observer remains parked. Never deliver that stale payload.
        if !self.workspace_scope_is_current(scope) {
            return Err("Workspace read scope ended".into());
        }
        response
    }
}

#[cfg(test)]
mod tests;
