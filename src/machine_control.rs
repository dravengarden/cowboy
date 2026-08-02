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

fn adapter_timeout(adapter: &str) -> std::time::Duration {
    if adapter == "workspace" {
        WORKSPACE_ADAPTER_TIMEOUT
    } else {
        DEFAULT_ADAPTER_TIMEOUT
    }
}

struct Connection {
    epoch: String,
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
        tx: mpsc::UnboundedSender<MachineCommand>,
    ) {
        self.connections
            .write()
            .insert(machine_id, Connection { epoch, tx });
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

    pub fn send(&self, machine_id: &str, command: MachineCommand) -> Result<(), String> {
        self.connections
            .read()
            .get(machine_id)
            .ok_or_else(|| format!("machine {machine_id:?} is not connected"))?
            .tx
            .send(command)
            .map_err(|_| format!("machine {machine_id:?} disconnected"))
    }

    pub fn record(&self, machine_id: &str, event: MachineEvent) {
        if let MachineEvent::AdapterResponse {
            request_id,
            accepted,
            payload,
            detail,
        } = &event
            && let Some(sender) = self.pending.write().remove(request_id)
        {
            let result = if *accepted {
                payload
                    .clone()
                    .ok_or_else(|| "adapter response has no payload".to_owned())
            } else {
                Err(detail
                    .clone()
                    .unwrap_or_else(|| "adapter request rejected".to_owned()))
            };
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
    }

    #[tokio::test]
    async fn adapter_response_is_correlated_without_entering_another_machine() {
        let control = Arc::new(MachineControl::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        control.install("mac".to_owned(), "epoch".to_owned(), tx);
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
}
