//! Original-route observations for core Session filesystem/Git readers.
//! A logical Session can outlive its transport; this read binding cannot.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::local_roots::LocalRoot;
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
    /// What the Controller observed behind this route's cwd, when the
    /// Controller is the party that will read it. A replacement makes this
    /// route unequal to the current one, so cached representations, `ETags` and
    /// page/diff continuations keyed by it stop answering.
    local: LocalRoot,
    remote_root: Option<String>,
}

impl PartialEq for SessionReadScope {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner)
            && self.session == other.session
            && self.local == other.local
            && self.remote_root == other.remote_root
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
        self.local.hash(state);
        self.remote_root.hash(state);
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
            + self.remote_root.as_ref().map_or(0, String::len)
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
        let (connection, colocated) = if session.machine_id() == "local" {
            // The standalone core-owned `local` Session always executes here.
            (None, true)
        } else {
            let live = self.live.read();
            let machine = live.connections.get(session.machine_id())?;
            (Some(machine.token.clone()), machine.colocated)
        };
        // Observe the object only when the Controller is the party that will
        // read it; a remote Machine's cwd is not ours to stat. Resolving a
        // route never refuses: an unobservable root is recorded as such and
        // refused at the single gate that would actually read it.
        let local = if colocated {
            LocalRoot::observe(session.cwd())
        } else {
            LocalRoot::Remote
        };
        Some(SessionReadScope {
            owner: Arc::clone(&self.read_owner),
            session,
            connection,
            local,
            remote_root: None,
        })
    }

    /// Called only after original HTTP credential and Session visibility checks.
    /// Older Machines retain their explicitly legacy read path; negotiation
    /// cannot make them claim the new filesystem observation.
    pub(crate) async fn observe_session_read_root(
        &self,
        scope: &mut SessionReadScope,
    ) -> Result<(), String> {
        if self.session_read_is_colocated(scope)? {
            return Ok(());
        }
        let connection = scope.connection().ok_or("Session route unavailable")?;
        let supported = {
            let live = self.live.read();
            if !live.is_current(connection) {
                return Err("Session route ended".into());
            }
            live.connections[&connection.0.machine_id].protocol
                >= crate::machine_protocol::session_code::PROTOCOL_VERSION
        };
        if !supported {
            return Ok(());
        }
        let value = self
            .session_root_request(
                connection,
                crate::machine_protocol::session_code::Request::Observe {
                    root: scope.session.cwd().into(),
                },
            )
            .await?;
        let incarnation = value
            .get("incarnation")
            .and_then(serde_json::Value::as_str)
            .filter(|value| value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or("invalid Session root observation")?;
        scope.remote_root = Some(incarnation.into());
        Ok(())
    }

    async fn session_root_request(
        &self,
        connection: &ConnectionToken,
        request: crate::machine_protocol::session_code::Request,
    ) -> Result<serde_json::Value, String> {
        let payload = serde_json::to_value(request).map_err(|_| "Session root encoding failed")?;
        // Stat/verification have a short bounded wait, not the Code I/O budget.
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            self.adapter_request_on_connection(
                connection,
                crate::machine_protocol::session_code::ADAPTER,
                payload,
            ),
        )
        .await
        .map_err(|_| "Session root observation timed out".to_owned())?
    }

    pub(crate) async fn session_read_root_is_current(&self, scope: &SessionReadScope) -> bool {
        if !self.session_read_scope_is_current(scope) {
            return false;
        }
        let Some(incarnation) = &scope.remote_root else {
            return true;
        };
        let Some(connection) = scope.connection() else {
            return false;
        };
        self.session_root_request(
            connection,
            crate::machine_protocol::session_code::Request::Verify {
                root: scope.session.cwd().into(),
                incarnation: incarnation.clone(),
            },
        )
        .await
        .is_ok_and(|value| value.get("current") == Some(&serde_json::Value::Bool(true)))
            && self.session_read_scope_is_current(scope)
    }

    pub(crate) fn session_read_scope_is_current(&self, scope: &SessionReadScope) -> bool {
        Arc::ptr_eq(&self.read_owner, &scope.owner)
            && scope
                .connection
                .as_ref()
                .is_none_or(|connection| self.is_current(connection))
            // A replaced root ends this route, including its cached bytes,
            // ETag and `304`, without touching any other Session.
            && scope.local.is_current(scope.session.cwd())
    }

    /// Local execution is chosen only from this original observation. Stored
    /// colocated flags cannot make a disconnected Machine a local executor.
    pub(crate) fn session_read_is_colocated(
        &self,
        scope: &SessionReadScope,
    ) -> Result<bool, String> {
        let colocated = {
            let live = self.live.read();
            if !Arc::ptr_eq(&self.read_owner, &scope.owner) {
                return Err("Session read route ended".into());
            }
            match &scope.connection {
                None => true, // The standalone core-owned `local` Session.
                Some(connection) if live.is_current(connection) => {
                    live.connections[&connection.0.machine_id].colocated
                }
                Some(_) => return Err("Session read route ended".into()),
            }
        };
        if !colocated {
            return Ok(false);
        }
        // Re-observe immediately before the Controller reads the root itself.
        scope.local.readable(scope.session.cwd())?;
        Ok(true)
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
        let (adapter, request) = if let Some(incarnation) = &scope.remote_root {
            (
                crate::machine_protocol::session_code::ADAPTER,
                serde_json::to_value(crate::machine_protocol::session_code::Request::Read {
                    root: scope.session.cwd().into(),
                    incarnation: incarnation.clone(),
                    operation: serde_json::to_value(operation)
                        .map_err(|_| "Session Code encoding failed")?,
                }),
            )
        } else {
            (
                "code",
                serde_json::to_value(CodeAdapterRequest {
                    root: scope.session.cwd().into(),
                    operation,
                }),
            )
        };
        let request = request.map_err(|_| "Session Code request encoding failed".to_owned())?;
        // Atomic original-connection enqueue and post-await validation are
        // shared with the other finite readers. No lookup by Machine name.
        self.adapter_request_on_connection(connection, adapter, request)
            .await
            .map(Some)
    }
}

#[cfg(test)]
mod tests;
