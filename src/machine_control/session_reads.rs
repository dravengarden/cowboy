//! Original-route observations for core Session filesystem/Git readers.
//! A logical Session can outlive its transport; this read binding cannot.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::{ConnectionToken, MachineControl};
use crate::code_adapter::{CodeAdapterRequest, CodeOperation};
use crate::core::SessionCodeScope;

/// Private construction, no serde: equal names, paths or epochs cannot mint a
/// route. Cloning borrows the original observation, never a replacement route.
#[derive(Clone, Debug)]
pub(crate) struct SessionReadScope {
    owner: Arc<()>,
    session: SessionCodeScope,
    connection: Option<ConnectionToken>,
}

impl PartialEq for SessionReadScope {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner)
            && self.session == other.session
            && match (&self.connection, &other.connection) {
                (None, None) => true,
                (Some(left), Some(right)) => left.same(right),
                _ => false,
            }
    }
}
impl Eq for SessionReadScope {}
impl Hash for SessionReadScope {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.owner).hash(state);
        self.session.hash(state);
        self.connection
            .as_ref()
            .map(|connection| Arc::as_ptr(&connection.0))
            .hash(state);
    }
}

impl SessionReadScope {
    pub(crate) fn session(&self) -> &SessionCodeScope {
        &self.session
    }

    pub(crate) fn connection(&self) -> Option<&ConnectionToken> {
        self.connection.as_ref()
    }

    pub(crate) fn string_bytes(&self) -> usize {
        self.session.string_bytes()
            + self.connection.as_ref().map_or(0, |connection| {
                connection.0.machine_id.len() + connection.0.epoch.len()
            })
    }
}

impl MachineControl {
    pub(crate) fn session_read_scope(
        &self,
        service: &str,
        session: SessionCodeScope,
    ) -> Option<SessionReadScope> {
        if self.service.as_str() != service {
            return None;
        }
        let connection = if session.machine_id() == "local" {
            None
        } else {
            Some(
                self.live
                    .read()
                    .connections
                    .get(session.machine_id())?
                    .token
                    .clone(),
            )
        };
        Some(SessionReadScope {
            owner: Arc::clone(&self.read_owner),
            session,
            connection,
        })
    }

    pub(crate) fn session_read_scope_is_current(&self, scope: &SessionReadScope) -> bool {
        Arc::ptr_eq(&self.read_owner, &scope.owner)
            && scope
                .connection
                .as_ref()
                .is_none_or(|connection| self.is_current(connection))
    }

    /// Local execution is chosen only from this original observation. Stored
    /// colocated flags cannot make a disconnected Machine a local executor.
    pub(crate) fn session_read_is_colocated(
        &self,
        scope: &SessionReadScope,
    ) -> Result<bool, String> {
        let live = self.live.read();
        if !Arc::ptr_eq(&self.read_owner, &scope.owner) {
            return Err("Session read route ended".into());
        }
        match &scope.connection {
            None => Ok(true), // The standalone core-owned `local` Session.
            Some(connection) if live.is_current(connection) => {
                Ok(live.connections[&connection.0.machine_id].colocated)
            }
            Some(_) => Err("Session read route ended".into()),
        }
    }

    pub(crate) async fn code_request_in_session(
        &self,
        scope: &SessionReadScope,
        operation: CodeOperation,
    ) -> Result<Option<serde_json::Value>, String> {
        if self.session_read_is_colocated(scope)? {
            return Ok(None);
        }
        let connection = scope
            .connection()
            .ok_or_else(|| "Session read route unavailable".to_owned())?;
        let request = serde_json::to_value(CodeAdapterRequest {
            root: scope.session.cwd().into(),
            operation,
        })
        .map_err(|_| "Session Code request encoding failed".to_owned())?;
        // Atomic original-connection enqueue and post-await validation are
        // shared with the other finite readers. No lookup by Machine name.
        self.adapter_request_on_connection(connection, "code", request)
            .await
            .map(Some)
    }
}

#[cfg(test)]
mod tests;
