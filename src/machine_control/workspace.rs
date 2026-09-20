//! Continuous, bounded observations of authenticated Machine workspace exports.
//! These are read identities, not filesystem proofs, grants or durable leases.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::{ConnectionToken, LiveState, MachineControl, RequestBinding};
use crate::code_adapter::{CodeAdapterRequest, CodeOperation};
use crate::machine_protocol::{
    CODE_WORKSPACE_ROOT_IDENTITY_PROTOCOL_VERSION, MachineWorkspace, WorkspaceRootIdentity,
};

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
    /// The exact opaque value the Machine minted for the object behind this
    /// root. The Controller stores and echoes it; it never derives, renews or
    /// compares it against a path, revision or any other observation.
    incarnation: Option<String>,
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
            + self.0.incarnation.as_ref().map_or(0, String::len)
    }

    fn incarnation(&self) -> Option<String> {
        self.0.incarnation.clone()
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

/// Structurally invalid or duplicated identities are dropped, not merged: an
/// ambiguous advertisement must end that root, never pick one of two values.
fn unique_incarnations(identities: &[WorkspaceRootIdentity]) -> HashMap<&str, &str> {
    let mut seen: HashMap<&str, Option<&str>> = HashMap::new();
    for identity in identities {
        if !identity.is_well_formed() {
            seen.insert(identity.workspace_id.as_str(), None);
            continue;
        }
        seen.entry(identity.workspace_id.as_str())
            .and_modify(|entry| *entry = None)
            .or_insert(Some(identity.incarnation.as_str()));
    }
    seen.into_iter()
        .filter_map(|(id, value)| Some((id, value?)))
        .collect()
}

impl LiveState {
    pub(super) fn observe_workspaces(
        &mut self,
        connection: &ConnectionToken,
        workspaces: &[MachineWorkspace],
        identities: Option<&[WorkspaceRootIdentity]>,
    ) {
        let machine = &connection.0.machine_id;
        // A Machine that owns root identities must supply one per advertised
        // root. Older Machines supply none and keep the previous behaviour,
        // which this slice deliberately does not widen.
        let owned = self
            .connections
            .get(machine)
            .is_some_and(|live| live.protocol >= CODE_WORKSPACE_ROOT_IDENTITY_PROTOCOL_VERSION);
        let minted = if owned {
            unique_incarnations(identities.unwrap_or_default())
        } else {
            HashMap::new()
        };
        // Count before allocating a second map. A rejected observation ends the
        // old lifetimes, rather than retaining apparently-current stale roots.
        let mut count = workspaces.len();
        let mut bytes = workspaces.iter().fold(0_usize, |total, item| {
            total
                .saturating_add(item.id.len())
                .saturating_add(item.canonical_path.len())
                .saturating_add(connection.0.machine_id.len())
                .saturating_add(connection.0.epoch.len())
                .saturating_add(minted.get(item.id.as_str()).map_or(0, |value| value.len()))
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
            || identities.map_or(0, <[WorkspaceRootIdentity]>::len) > MAX_WORKSPACES_PER_MACHINE
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
                // An owning Machine that cannot observe a root advertises no
                // identity for it. Refuse that root rather than reading it
                // under an identity nobody is enforcing.
                let incarnation = if owned {
                    Some((*minted.get(id)?).to_owned())
                } else {
                    None
                };
                let scope = previous
                    .and_then(|slots| slots.get(id))
                    .filter(|scope| {
                        scope.cwd() == workspace.canonical_path
                            && scope.0.connection.same(connection)
                            && scope.0.incarnation == incarnation
                    })
                    .cloned()
                    .unwrap_or_else(|| {
                        WorkspaceCodeScope(Arc::new(WorkspaceIdentity {
                            connection: connection.clone(),
                            workspace_id: id.into(),
                            cwd: workspace.canonical_path.clone(),
                            incarnation,
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
                // The Machine re-resolves its own root against this exact
                // value before reading. The Controller supplies no default
                // and cannot substitute another observation's identity.
                scope.incarnation(),
                Some(RequestBinding::Workspace(scope)),
            )
            .await;
        // The Machine owns this identity, so its typed refusal is the earliest
        // evidence that the observation ended. Retire the slot now: cached
        // bytes, ETags and continuations keyed by it must not survive either.
        // Retirement records an ended observation; it is not a rollback, an
        // undo, or a claim about work the Code adapter already started.
        if let Err(failure) = &response
            && failure.workspace_root_identity_changed()
        {
            self.retire_workspace_scope(scope);
            return Err("Workspace read scope ended".into());
        }
        // A reply can complete immediately before a remove/re-add or reconnect
        // while its observer remains parked. Never deliver that stale payload.
        if !self.workspace_scope_is_current(scope) {
            return Err("Workspace read scope ended".into());
        }
        response.map_err(String::from)
    }

    /// Remove exactly the ended slot. Another Workspace on the same Machine,
    /// the connection and every other Machine keep their own observations.
    fn retire_workspace_scope(&self, scope: &WorkspaceCodeScope) {
        let mut live = self.live.write();
        let Some(slots) = live.workspace_inventory.get_mut(scope.machine_id()) else {
            return;
        };
        if slots
            .get(&scope.0.workspace_id)
            .is_some_and(|current| current == scope)
        {
            slots.remove(&scope.0.workspace_id);
        }
    }
}

#[cfg(test)]
mod tests;
