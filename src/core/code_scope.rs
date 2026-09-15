//! Exact, process-local observations of a Session's code workspace. These are
//! not grants, native-generation leases or serializable restoration records.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::{Hub, Session};

/// A finite read/cache identity. Workspace fields are the Service's current
/// advertised snapshot, not a filesystem identity proof or writer lease.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CodeReadScope {
    Session(SessionCodeScope),
    Workspace {
        service_id: String,
        machine_id: String,
        workspace_id: String,
        cwd: String,
    },
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
}

impl SessionCodeScope {
    fn observe(session: &Session) -> Self {
        Self {
            incarnation: session.code_incarnation.clone(),
            session_id: session.meta.id.clone(),
            machine_id: session.meta.machine_id.clone(),
            workspace_id: session.meta.workspace_id.clone(),
            cwd: session.meta.cwd.clone(),
            owner_user_id: session.meta.owner_user_id.clone(),
        }
    }

    pub(crate) fn machine_id(&self) -> &str {
        &self.machine_id
    }

    pub(crate) fn cwd(&self) -> &str {
        &self.cwd
    }
}

impl Hub {
    pub(crate) fn session_code_scope(&self, session_id: &str) -> Option<SessionCodeScope> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .map(SessionCodeScope::observe)
    }

    /// Recheck the exact original observation. Removal, replacement or cwd ABA
    /// cannot revive it; unrelated UI/worker status does not invalidate it.
    pub(crate) fn code_scope_is_current(&self, scope: &SessionCodeScope) -> bool {
        self.session_code_scope(&scope.session_id).as_ref() == Some(scope)
    }
}

#[cfg(test)]
mod tests;
