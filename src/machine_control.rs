//! Live command channels for authenticated Machine hosts.

#![warn(clippy::pedantic)]

use std::collections::HashMap;

use parking_lot::RwLock;
use tokio::sync::{mpsc, oneshot};

use crate::machine_protocol::{MachineCommand, MachineEvent};

const DEFAULT_ADAPTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(40);
// Workspace preparation can run ten sequential Git commands, each bounded to
// 30 seconds on the Machine. Keep the controller alive beyond that complete
// Machine-side envelope so it never abandons a still-running preparation.
const WORKSPACE_ADAPTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(330);
const CACHE_STATUS_ADAPTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
const PROVIDER_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

fn adapter_timeout(adapter: &str) -> std::time::Duration {
    if adapter == "workspace" {
        WORKSPACE_ADAPTER_TIMEOUT
    } else if adapter == "deepseek-cache-status" {
        CACHE_STATUS_ADAPTER_TIMEOUT
    } else {
        DEFAULT_ADAPTER_TIMEOUT
    }
}

struct Connection {
    epoch: String,
    colocated: bool,
    protocol: u16,
    tx: mpsc::UnboundedSender<MachineCommand>,
}

#[derive(Default)]
pub struct MachineControl {
    connections: RwLock<HashMap<String, Connection>>,
    events: RwLock<HashMap<String, Vec<MachineEvent>>>,
    pending: RwLock<HashMap<String, oneshot::Sender<Result<serde_json::Value, String>>>>,
}

impl MachineControl {
    pub fn install(
        &self,
        machine_id: String,
        epoch: String,
        colocated: bool,
        protocol: u16,
        tx: mpsc::UnboundedSender<MachineCommand>,
    ) {
        self.connections.write().insert(
            machine_id,
            Connection {
                epoch,
                colocated,
                protocol,
                tx,
            },
        );
    }

    /// Return whether the current authenticated connection shares the
    /// controller filesystem. `None` means the Machine is not connected, so
    /// callers may consult persisted connection metadata instead.
    #[must_use]
    pub fn is_colocated(&self, machine_id: &str) -> Option<bool> {
        self.connections
            .read()
            .get(machine_id)
            .map(|connection| connection.colocated)
    }

    pub fn remove_if_current(&self, machine_id: &str, epoch: &str) {
        let mut connections = self.connections.write();
        if connections
            .get(machine_id)
            .is_some_and(|connection| connection.epoch == epoch)
        {
            connections.remove(machine_id);
        }
    }

    /// Drop the active command channel for a revoked Machine regardless of
    /// connection epoch. The WebSocket task observes the closed receiver and
    /// exits; subsequent reconnects fail durable identity validation.
    pub fn disconnect(&self, machine_id: &str) {
        self.connections.write().remove(machine_id);
    }

    pub fn send(&self, machine_id: &str, command: MachineCommand) -> Result<(), String> {
        let connections = self.connections.read();
        let connection = connections
            .get(machine_id)
            .ok_or_else(|| format!("machine {machine_id:?} is not connected"))?;
        let required = command.minimum_protocol();
        if connection.protocol < required {
            return Err(format!(
                "machine {machine_id:?} negotiated protocol {}, but this command requires {required}",
                connection.protocol
            ));
        }
        connection
            .tx
            .send(command)
            .map_err(|_| format!("machine {machine_id:?} disconnected"))
    }

    pub fn record(&self, machine_id: &str, event: MachineEvent) {
        let correlated = match &event {
            MachineEvent::AdapterResponse {
                request_id,
                accepted,
                payload,
                detail,
            } => Some((
                request_id,
                if *accepted {
                    payload
                        .clone()
                        .ok_or_else(|| "adapter response has no payload".to_owned())
                } else {
                    Err(detail
                        .clone()
                        .unwrap_or_else(|| "adapter request rejected".to_owned()))
                },
            )),
            MachineEvent::CommandResult {
                request_id,
                accepted,
                detail,
            } => Some((
                request_id,
                if *accepted {
                    Ok(serde_json::Value::Null)
                } else {
                    Err(detail
                        .clone()
                        .unwrap_or_else(|| "Machine command rejected".to_owned()))
                },
            )),
            _ => None,
        };
        if let Some((request_id, result)) = correlated
            && let Some(sender) = self.pending.write().remove(request_id)
        {
            let _ = sender.send(result);
        }
        let mut events = self.events.write();
        let history = events.entry(machine_id.to_owned()).or_default();
        history.push(event);
        if history.len() > 64 {
            history.drain(..history.len() - 64);
        }
    }

    pub async fn adapter_request(
        &self,
        machine_id: &str,
        adapter: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let request_id = format!(
            "adapter-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |value| value.as_nanos())
        );
        let (tx, rx) = oneshot::channel();
        self.pending.write().insert(request_id.clone(), tx);
        if let Err(error) = self.send(
            machine_id,
            MachineCommand::AdapterRequest {
                request_id: request_id.clone(),
                adapter: adapter.to_owned(),
                payload,
            },
        ) {
            self.pending.write().remove(&request_id);
            return Err(error);
        }
        match tokio::time::timeout(adapter_timeout(adapter), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Machine adapter response channel closed".to_owned()),
            Err(_) => {
                self.pending.write().remove(&request_id);
                Err("Machine adapter request timed out".to_owned())
            }
        }
    }

    pub async fn command_request(
        &self,
        machine_id: &str,
        request_id: String,
        command: MachineCommand,
    ) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.pending.write().insert(request_id.clone(), tx);
        if let Err(error) = self.send(machine_id, command) {
            self.pending.write().remove(&request_id);
            return Err(error);
        }
        match tokio::time::timeout(PROVIDER_COMMAND_TIMEOUT, rx).await {
            Ok(Ok(Ok(_))) => Ok(()),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(_)) => Err("Machine command response channel closed".to_owned()),
            Err(_) => {
                self.pending.write().remove(&request_id);
                Err("Machine command timed out".to_owned())
            }
        }
    }

    #[must_use]
    pub fn connected_machine_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self.connections.read().keys().cloned().collect();
        ids.sort();
        ids
    }

    #[must_use]
    pub fn events(&self, machine_id: &str) -> Vec<MachineEvent> {
        self.events
            .read()
            .get(machine_id)
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn workspace_requests_cover_the_complete_machine_preparation_envelope() {
        assert_eq!(
            adapter_timeout("workspace"),
            std::time::Duration::from_secs(330)
        );
        assert_eq!(adapter_timeout("zed"), std::time::Duration::from_secs(40));
        assert_eq!(
            adapter_timeout("deepseek-cache-status"),
            std::time::Duration::from_secs(3)
        );
    }

    #[test]
    fn explicit_disconnect_removes_the_active_command_channel() {
        let control = MachineControl::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        control.install("macbook-air".to_owned(), "epoch".to_owned(), false, 3, tx);

        control.disconnect("macbook-air");

        assert!(
            control
                .send(
                    "macbook-air",
                    MachineCommand::RefreshInventory {
                        request_id: "refresh".to_owned(),
                    },
                )
                .is_err()
        );
    }

    #[tokio::test]
    async fn adapter_response_is_correlated_without_entering_another_machine() {
        let control = Arc::new(MachineControl::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        control.install("mac".to_owned(), "epoch".to_owned(), false, 3, tx);
        let requester = Arc::clone(&control);
        let request = tokio::spawn(async move {
            requester
                .adapter_request("mac", "zed", serde_json::json!({ "type": "health" }))
                .await
        });
        let command = rx.recv().await.expect("command");
        let MachineCommand::AdapterRequest { request_id, .. } = command else {
            panic!("wrong command");
        };
        control.record(
            "mac",
            MachineEvent::AdapterResponse {
                request_id,
                accepted: true,
                payload: Some(serde_json::json!({ "type": "health", "apiVersion": 1 })),
                detail: None,
            },
        );
        assert_eq!(
            request.await.expect("task").expect("response")["type"],
            "health"
        );
    }

    #[test]
    fn colocated_state_belongs_to_the_current_machine_connection() {
        let control = MachineControl::default();
        let (first, _) = mpsc::unbounded_channel();
        control.install("hawk".to_owned(), "old".to_owned(), true, 3, first);
        assert_eq!(control.is_colocated("hawk"), Some(true));

        let (current, _) = mpsc::unbounded_channel();
        control.install("hawk".to_owned(), "current".to_owned(), false, 3, current);
        control.remove_if_current("hawk", "old");
        assert_eq!(control.is_colocated("hawk"), Some(false));

        control.remove_if_current("hawk", "current");
        assert_eq!(control.is_colocated("hawk"), None);
    }

    #[test]
    fn provider_commands_fail_before_crossing_an_old_machine_protocol() {
        let control = MachineControl::default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        control.install("old".to_owned(), "epoch".to_owned(), false, 2, tx);
        let error = control
            .send(
                "old",
                MachineCommand::BeginLogin {
                    request_id: "request".to_owned(),
                    provider: "gemini".to_owned(),
                    auth_method: Some("code-assist".to_owned()),
                },
            )
            .unwrap_err();
        assert!(error.contains("requires 3"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn provider_uninstall_and_compensation_require_machine_protocol_four() {
        let control = MachineControl::default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        control.install("old".to_owned(), "epoch".to_owned(), false, 3, tx);
        let digest = format!("sha256:{}", "ab".repeat(32));
        for command in [
            MachineCommand::UninstallProvider {
                request_id: "uninstall".to_owned(),
                provider_id: "gemini".to_owned(),
                generation_digest: digest.clone(),
            },
            MachineCommand::ReactivateProvider {
                request_id: "reactivate".to_owned(),
                provider_id: "gemini".to_owned(),
                generation_digest: digest.clone(),
            },
        ] {
            let error = control.send("old", command).unwrap_err();
            assert!(error.contains("requires 4"));
        }
        assert!(rx.try_recv().is_err());
    }
}
