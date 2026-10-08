//! Exact, process-local observations of a Session's code workspace. These are
//! not grants, native-generation leases or serializable restoration records.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::{Hub, Session};

/// A finite read/cache identity, not a filesystem identity proof or writer lease.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CodeReadScope {
    Session(crate::machine_control::SessionReadScope),
    Workspace(crate::machine_control::WorkspaceCodeScope),
}

impl CodeReadScope {
    /// Logical string bytes retained by bounded read caches; not a wire identity.
    pub(crate) fn string_bytes(&self) -> usize {
        match self {
            Self::Session(scope) => scope.string_bytes(),
            Self::Workspace(scope) => scope.string_bytes(),
        }
    }

    pub(crate) fn cwd(&self) -> &str {
        match self {
            Self::Session(scope) => scope.session().cwd(),
            Self::Workspace(scope) => scope.cwd(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct CodeIncarnation(Arc<()>);

impl PartialEq for CodeIncarnation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for CodeIncarnation {}

impl Hash for CodeIncarnation {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SessionCodeScope {
    incarnation: CodeIncarnation,
    session_id: String,
    machine_id: String,
    workspace_id: Option<String>,
    cwd: String,
    owner_user_id: Option<String>,
    execution_binding: Option<Arc<crate::execution_environment::BindingV1>>,
}

impl SessionCodeScope {
    pub(crate) fn string_bytes(&self) -> usize {
        self.session_id.len()
            + self.machine_id.len()
            + self.workspace_id.as_ref().map_or(0, String::len)
            + self.cwd.len()
            + self.owner_user_id.as_ref().map_or(0, String::len)
            + self
                .execution_binding
                .as_deref()
                .map_or(0, crate::execution_environment::BindingV1::string_bytes)
    }

    fn observe(session: &Session) -> Option<Self> {
        let execution_binding = session
            .meta
            .execution_binding
            .as_ref()
            .map(|binding| binding.for_runtime(&session.meta.machine_id, &session.meta.cwd))
            .transpose()
            .ok()?;
        let (machine_id, workspace_id, cwd) = execution_binding.as_ref().map_or_else(
            || {
                (
                    session.meta.machine_id.clone(),
                    session.meta.workspace_id.clone(),
                    session.meta.cwd.clone(),
                )
            },
            |binding| {
                (
                    binding.environment.machine_id.clone(),
                    Some(binding.workspace.id.clone()),
                    binding.workspace.cwd.clone(),
                )
            },
        );
        Some(Self {
            incarnation: session.code_incarnation.clone(),
            session_id: session.meta.id.clone(),
            machine_id,
            workspace_id,
            cwd,
            owner_user_id: session.meta.owner_user_id.clone(),
            execution_binding: execution_binding.map(Arc::new),
        })
    }

    pub(crate) fn machine_id(&self) -> &str {
        &self.machine_id
    }

    pub(crate) fn cwd(&self) -> &str {
        &self.cwd
    }

    pub(crate) fn owner_user_id(&self) -> Option<&str> {
        self.owner_user_id.as_deref()
    }
}

impl Hub {
    pub(crate) fn session_code_scope(&self, session_id: &str) -> Option<SessionCodeScope> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .and_then(SessionCodeScope::observe)
    }

    /// Record the durable lineage the owning Machine reported for a Session.
    /// Returns whether it changed. A change (a reset's new lineage, a Machine
    /// that starts or stops reporting one) gives the Session a new observation
    /// lifetime, so every earlier observation is stale and none is updated in
    /// place; an unchanged value (every reconnect and snapshot of the same
    /// lineage) leaves them current.
    pub(crate) fn set_machine_lineage(&self, session_id: &str, lineage: Option<&str>) -> bool {
        let mut sessions = self.inner.sessions.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return false;
        };
        if session.machine_lineage.as_deref() == lineage {
            return false;
        }
        session.machine_lineage = lineage.map(str::to_owned);
        // A new lifetime as well, so a lineage that is ever reported again (a
        // restored older dataset, say) cannot revive an observation made under
        // it: equality of the value alone would.
        session.code_incarnation = CodeIncarnation::default();
        true
    }

    /// How many Sessions of one Machine this Controller knows, and for how many it
    /// holds a Machine-reported lineage. Counts only: no Session ID or lineage
    /// value, so it can be shown without granting anything.
    pub(crate) fn lineage_counts(&self, machine_id: &str) -> (u64, u64) {
        let sessions = self.inner.sessions.lock();
        sessions
            .values()
            .filter(|session| session.meta.machine_id == machine_id)
            .fold((0, 0), |(total, carried), session| {
                (
                    total + 1,
                    carried + u64::from(session.machine_lineage.is_some()),
                )
            })
    }

    /// Recheck the exact original observation. Removal, replacement or cwd ABA
    /// cannot revive it; unrelated UI/worker status does not invalidate it.
    pub(crate) fn code_scope_is_current(&self, scope: &SessionCodeScope) -> bool {
        self.session_code_scope(&scope.session_id).as_ref() == Some(scope)
    }
}

#[cfg(test)]
mod tests;
