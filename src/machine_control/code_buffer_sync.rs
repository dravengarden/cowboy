//! Typed original-connection transport, not an authorization constructor.

use super::{ConnectionToken, MachineControl, Reply, ReplyKind, RequestBinding};
use crate::machine_protocol::code_buffer_sync::{Action, Content, Request, Snapshot};
use crate::machine_protocol::{CODE_BUFFER_SYNC_PROTOCOL_VERSION, MachineCommand};

impl MachineControl {
    pub(crate) fn supports_code_buffer_sync(&self, connection: &ConnectionToken) -> bool {
        self.connection_supports(connection, CODE_BUFFER_SYNC_PROTOCOL_VERSION)
    }

    pub(crate) async fn code_buffer_sync(
        &self,
        connection: &ConnectionToken,
        request: Request,
        content: &Content,
    ) -> Result<Snapshot, String> {
        request
            .validate()
            .and_then(|()| content.validate())
            .map_err(|_| "invalid synchronization request".to_owned())?;
        let expected = match &request.action {
            Action::Prepare { .. } => None,
            Action::Apply { operation }
            | Action::Query { operation }
            | Action::Retire { operation } => Some(operation.clone()),
        };
        let request_id = self.request_id("code-sync")?;
        let (receiver, _pending) = self.begin_request(
            &connection.0.machine_id,
            &request_id,
            MachineCommand::CodeBufferSync {
                request_id: request_id.clone(),
                request: Box::new(request),
            },
            ReplyKind::Adapter,
            Some(RequestBinding::Connection(connection)),
        )?;
        let value = match tokio::time::timeout(super::DEFAULT_ADAPTER_TIMEOUT, receiver).await {
            Ok(Ok(Reply::Adapter(Ok(value)))) if self.is_current(connection) => value,
            _ => return Err("synchronization observation unavailable".into()),
        };
        let bytes = serde_json::to_vec(&value)
            .map_err(|_| "invalid synchronization observation".to_owned())?;
        if bytes.len() > 16 * 1024 {
            return Err("synchronization observation too large".into());
        }
        let snapshot: Snapshot = serde_json::from_slice(&bytes)
            .map_err(|_| "invalid synchronization observation".to_owned())?;
        if snapshot.api_version != 1
            || expected.is_some_and(|expected| expected != snapshot.operation)
            || snapshot.operation.validate().is_err()
            || snapshot.state.validate(content).is_err()
        {
            return Err("synchronization observation changed".into());
        }
        Ok(snapshot)
    }
}
