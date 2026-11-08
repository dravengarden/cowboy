//! Connection-local extension dispatch. Slow source reads never block the link.
use super::*;
use crate::mcp_binding::TransportRequest;

pub(super) struct McpLink {
    #[cfg(target_os = "linux")]
    sessions: Arc<crate::machine_plugins::McpSessions>,
    #[cfg(target_os = "linux")]
    scope: std::sync::Weak<PluginExecutionScope>,
    #[cfg(target_os = "linux")]
    capacity: Arc<tokio::sync::Semaphore>,
}

impl McpLink {
    pub(super) fn new(scope: &Arc<PluginExecutionScope>) -> Self {
        let _ = scope;
        Self {
            #[cfg(target_os = "linux")]
            sessions: Arc::new(crate::machine_plugins::McpSessions::new(scope)),
            #[cfg(target_os = "linux")]
            scope: Arc::downgrade(scope),
            #[cfg(target_os = "linux")]
            capacity: Arc::new(tokio::sync::Semaphore::new(8)),
        }
    }

    #[cfg(target_os = "linux")]
    fn dispatch(
        &self,
        request_id: String,
        request: TransportRequest,
        store: Arc<MachinePluginStore>,
        events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    ) {
        if !crate::managed_calls::valid_id(&request_id) {
            return;
        }
        let reply = move |response| {
            let _ = events.send(MachineEvent::McpTransportResponse {
                request_id,
                response,
            });
        };
        if request.validate().is_err() {
            reply(serde_json::json!({"error":"invalid_mcp_request"}));
            return;
        }
        if let TransportRequest::ConfirmNative { handle } = &request {
            let confirmed = self.sessions.confirm_native(handle);
            reply(serde_json::json!({"confirmed":confirmed}));
            return;
        }
        if let TransportRequest::ReleaseNative { handle } = &request {
            let released = self.sessions.release_native(handle);
            reply(serde_json::json!({"released":released}));
            return;
        }
        if let TransportRequest::Close { handle } = &request {
            self.sessions.close(handle);
            reply(serde_json::json!({"closed":true}));
            return;
        }
        if let TransportRequest::Renew { handle, profile } = &request {
            let response = match self.sessions.renew(handle, profile) {
                Ok(()) => serde_json::json!({"renewed":true}),
                Err(_) => serde_json::json!({"error":"mcp_unavailable"}),
            };
            reply(response);
            return;
        }
        let Ok(permit) = Arc::clone(&self.capacity).try_acquire_owned() else {
            reply(serde_json::json!({"error":"mcp_capacity"}));
            return;
        };
        let sessions = Arc::clone(&self.sessions);
        let scope = self.scope.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let result = match request {
                TransportRequest::PrepareNative { profile } => {
                    sessions.prepare_native(&store, *profile).await
                }
                TransportRequest::Open { profile } => sessions
                    .open(&store, *profile)
                    .await
                    .map(|handle| serde_json::json!({"handle":handle})),
                TransportRequest::Request { handle, message } => sessions
                    .request(&handle, message)
                    .await
                    .map(|response| serde_json::json!({"response":response})),
                TransportRequest::Close { .. }
                | TransportRequest::Renew { .. }
                | TransportRequest::ReleaseNative { .. }
                | TransportRequest::ConfirmNative { .. } => unreachable!(),
            };
            if scope.upgrade().is_some() {
                reply(result.unwrap_or_else(|_| serde_json::json!({"error":"mcp_unavailable"})));
            }
        });
    }
    #[cfg(not(target_os = "linux"))]
    fn dispatch(
        &self,
        request_id: String,
        _request: TransportRequest,
        _store: Arc<MachinePluginStore>,
        events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    ) {
        let _ = events.send(MachineEvent::McpTransportResponse {
            request_id,
            response: serde_json::json!({"error":"mcp_platform_unsupported"}),
        });
    }
}

pub(super) fn consume(
    command: MachineCommand,
    config: &ControllerConfig,
    events: &tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    mcp: &McpLink,
) -> Option<MachineCommand> {
    match command {
        MachineCommand::ManagedCallReply {
            request_id,
            response,
        } => {
            if let Some(calls) = config.execution.calls() {
                calls.reply(&request_id, response);
            }
        }
        MachineCommand::MemoryRelayReply {
            request_id,
            response,
        } => {
            if let Some(relay) = config.memory_relay.as_ref() {
                relay.reply(&request_id, response);
            }
        }
        MachineCommand::MemoryRelay {
            request_id,
            identity,
            request,
        } => {
            crate::memory_relay::serve_home(request_id, identity, request, events.clone());
        }
        MachineCommand::McpTransport {
            request_id,
            request,
        } => {
            mcp.dispatch(
                request_id,
                request,
                Arc::clone(&config.providers),
                events.clone(),
            );
        }
        command => return Some(command),
    }
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
