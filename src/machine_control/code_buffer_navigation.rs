//! Closed transport bound to the original authenticated connection. These
//! declarations do not construct principal, Session or acquisition authority.
use super::{ConnectionToken, MachineControl, Reply, ReplyKind, RequestBinding};
use crate::machine_protocol::code_buffer_navigation::{Action, Request, Snapshot};
use crate::machine_protocol::{CODE_BUFFER_NAVIGATION_PROTOCOL_VERSION, MachineCommand};

impl MachineControl {
    pub(crate) fn supports_code_buffer_navigation(&self, connection: &ConnectionToken) -> bool {
        self.connection_supports(connection, CODE_BUFFER_NAVIGATION_PROTOCOL_VERSION)
    }

    pub(crate) async fn code_buffer_navigation(
        &self,
        connection: &ConnectionToken,
        request: Request,
    ) -> Result<Snapshot, String> {
        request
            .validate()
            .map_err(|_| "invalid navigation request")?;
        let expected = match &request.action {
            Action::Prepare { .. } => None,
            Action::Execute { navigation }
            | Action::Query { navigation }
            | Action::Release { navigation }
            | Action::PrepareDestination { navigation, .. } => Some(navigation.clone()),
        };
        let request_id = self.request_id("code-navigation")?;
        let (receiver, _pending) = self.begin_request(
            &connection.0.machine_id,
            &request_id,
            MachineCommand::CodeBufferNavigation {
                request_id: request_id.clone(),
                request: Box::new(request),
            },
            ReplyKind::Adapter,
            Some(RequestBinding::Connection(connection)),
        )?;
        let value = match tokio::time::timeout(super::DEFAULT_ADAPTER_TIMEOUT, receiver).await {
            Ok(Ok(Reply::Adapter(Ok(value)))) if self.is_current(connection) => value,
            _ => return Err("navigation observation unavailable".into()),
        };
        let bytes = serde_json::to_vec(&value).map_err(|_| "invalid navigation observation")?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("navigation observation too large".into());
        }
        let snapshot: Snapshot =
            serde_json::from_slice(&bytes).map_err(|_| "invalid navigation observation")?;
        if expected.is_some_and(|expected| expected != snapshot.navigation)
            || snapshot.validate().is_err()
        {
            return Err("navigation observation changed".into());
        }
        Ok(snapshot)
    }
}
