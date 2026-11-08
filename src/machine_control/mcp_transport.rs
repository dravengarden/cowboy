//! Transport correlation only. Callers must separately admit the parent and
//! session policy, and cancel the handle when either authority ends.
use super::{ConnectionToken, MachineCommand, MachineControl, Reply, ReplyKind, RequestBinding};
use crate::mcp_binding::TransportRequest;

impl MachineControl {
    pub async fn mcp_request_on_connection(
        &self,
        connection: &ConnectionToken,
        request: TransportRequest,
    ) -> Result<serde_json::Value, String> {
        request
            .validate()
            .map_err(|_| "invalid MCP request".to_owned())?;
        let request_id = self.request_id("mcp")?;
        let (receiver, _pending) = self.begin_request(
            connection.machine_id(),
            &request_id,
            MachineCommand::McpTransport {
                request_id: request_id.clone(),
                request,
            },
            ReplyKind::Mcp,
            Some(RequestBinding::Connection(connection)),
        )?;
        let response =
            match tokio::time::timeout(std::time::Duration::from_secs(30), receiver).await {
                Ok(Ok(Reply::Mcp(response))) if self.is_current(connection) => response,
                _ => return Err("MCP connection or request ended".into()),
            };
        if serde_json::to_vec(&response).map_or(true, |bytes| bytes.len() > 1024 * 1024 + 4096) {
            return Err("MCP response too large".into());
        }
        Ok(response)
    }
}

impl super::LiveState {
    pub(super) fn complete_transport_reply(
        &mut self,
        token: &ConnectionToken,
        event: super::MachineEvent,
    ) -> Option<super::MachineEvent> {
        match event {
            super::MachineEvent::McpTransportResponse {
                request_id,
                response,
            } => self.complete(token, &request_id, Reply::Mcp(response)),
            super::MachineEvent::ExecutionResponse {
                request_id,
                response,
            } => self.complete(token, &request_id, Reply::Execution(response)),
            event => return Some(event),
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::MachineEvent;
    use serde_json::json;

    fn connect(
        control: &MachineControl,
        protocol: u16,
    ) -> (
        ConnectionToken,
        tokio::sync::mpsc::UnboundedReceiver<MachineCommand>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (
            control.install("hawk".into(), "same-epoch".into(), false, protocol, tx),
            rx,
        )
    }

    fn request() -> TransportRequest {
        TransportRequest::Close {
            handle: format!("mcp-{}", "a".repeat(32)),
        }
    }

    #[tokio::test]
    async fn mcp_transport_requires_version_and_current_connection_at_delivery() {
        let control = MachineControl::default();
        let (legacy, mut legacy_rx) = connect(&control, 41);
        assert!(
            control
                .mcp_request_on_connection(&legacy, request())
                .await
                .is_err()
        );
        assert!(legacy_rx.try_recv().is_err());
        let (current, mut rx) = connect(&control, 42);
        let (result, ()) = tokio::join!(
            control.mcp_request_on_connection(&current, request()),
            async {
                let MachineCommand::McpTransport { request_id, .. } = rx.recv().await.unwrap()
                else {
                    panic!()
                };
                control.record_remote(
                    &legacy,
                    MachineEvent::McpTransportResponse {
                        request_id: request_id.clone(),
                        response: json!({"foreign":true}),
                    },
                );
                control.record_remote(
                    &current,
                    MachineEvent::McpTransportResponse {
                        request_id,
                        response: json!({"closed":true}),
                    },
                );
            }
        );
        assert_eq!(result.unwrap(), json!({"closed":true}));
        let (result, ()) = tokio::join!(
            control.mcp_request_on_connection(&current, request()),
            async {
                let MachineCommand::McpTransport { request_id, .. } = rx.recv().await.unwrap()
                else {
                    panic!()
                };
                control.record_remote(
                    &current,
                    MachineEvent::McpTransportResponse {
                        request_id,
                        response: json!({"closed":true}),
                    },
                );
                let _replacement = connect(&control, 42);
            }
        );
        assert!(
            result.is_err(),
            "reply survived connection replacement before delivery"
        );
    }

    fn take_request(commands: &mut tokio::sync::mpsc::UnboundedReceiver<MachineCommand>) -> String {
        let MachineCommand::McpTransport { request_id, .. } = commands.try_recv().unwrap() else {
            panic!("MCP transport request expected")
        };
        request_id
    }

    #[tokio::test]
    async fn cancelling_mcp_wait_preserves_other_requests_and_discards_late_reply() {
        let control = MachineControl::default();
        let (connection, mut commands) = connect(&control, 42);
        let mut retained = Box::pin(control.mcp_request_on_connection(&connection, request()));
        assert!(futures::poll!(&mut retained).is_pending());
        let retained_id = take_request(&mut commands);
        let mut cancelled = Box::pin(control.mcp_request_on_connection(&connection, request()));
        assert!(futures::poll!(&mut cancelled).is_pending());
        let cancelled_id = take_request(&mut commands);
        assert_eq!(control.live.read().pending.len(), 2);
        drop(cancelled);
        assert_eq!(control.live.read().pending.len(), 1);
        assert!(control.live.read().pending.contains_key(&retained_id));
        control.record_remote(
            &connection,
            MachineEvent::McpTransportResponse {
                request_id: cancelled_id,
                response: json!({"late":true}),
            },
        );
        assert!(futures::poll!(&mut retained).is_pending());
        control.record_remote(
            &connection,
            MachineEvent::McpTransportResponse {
                request_id: retained_id,
                response: json!({"retained":true}),
            },
        );
        assert_eq!(retained.await.unwrap(), json!({"retained":true}));
        assert!(control.live.read().pending.is_empty());
        assert!(
            commands.try_recv().is_err(),
            "transport must not replay requests"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn mcp_timeout_releases_waiter_and_discards_late_reply() {
        let control = MachineControl::default();
        let (connection, mut commands) = connect(&control, 42);
        let mut pending = Box::pin(control.mcp_request_on_connection(&connection, request()));
        assert!(futures::poll!(&mut pending).is_pending());
        let request_id = take_request(&mut commands);
        tokio::time::advance(std::time::Duration::from_secs(31)).await;
        assert!(pending.await.is_err());
        assert!(control.live.read().pending.is_empty());
        control.record_remote(
            &connection,
            MachineEvent::McpTransportResponse {
                request_id,
                response: json!({"late":true}),
            },
        );
        assert!(control.live.read().pending.is_empty());
        assert!(commands.try_recv().is_err());
    }

    #[tokio::test]
    async fn mcp_disconnect_releases_waiter_before_deadline() {
        let control = MachineControl::default();
        let (connection, mut commands) = connect(&control, 42);
        let mut pending = Box::pin(control.mcp_request_on_connection(&connection, request()));
        assert!(futures::poll!(&mut pending).is_pending());
        take_request(&mut commands);
        control.remove_if_current(&connection);
        assert!(matches!(
            futures::poll!(&mut pending),
            std::task::Poll::Ready(Err(_))
        ));
        assert!(control.live.read().pending.is_empty());
    }
}
