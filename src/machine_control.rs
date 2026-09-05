//! Live command channels for authenticated Machine hosts.

#![warn(clippy::pedantic)]

use std::collections::HashMap;

use parking_lot::RwLock;
use tokio::sync::{mpsc, oneshot};

use crate::machine_protocol::{
    MachineCommand, MachineEvent, PLUGIN_HOST_EXECUTION_PROTOCOL_VERSION, PluginHostOperation,
    PluginInventory,
};

const DEFAULT_ADAPTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(40);
// Workspace preparation can run ten sequential Git commands, each bounded to
// 30 seconds on the Machine. Keep the controller alive beyond that complete
// Machine-side envelope so it never abandons a still-running preparation.
const WORKSPACE_ADAPTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(330);
const PROVIDER_STATUS_ADAPTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
const PROVIDER_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
const PLUGIN_HOST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

type PendingResponse = oneshot::Sender<Result<serde_json::Value, String>>;

#[derive(Debug)]
pub(crate) struct PluginHostRequestError {
    pub started: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ConnectedPluginInventory {
    pub machine_id: String,
    pub plugin: PluginInventory,
}

struct PendingRequestGuard<'a> {
    pending: &'a RwLock<HashMap<String, PendingResponse>>,
    request_id: &'a str,
}

impl Drop for PendingRequestGuard<'_> {
    fn drop(&mut self) {
        self.pending.write().remove(self.request_id);
    }
}

fn adapter_timeout(adapter: &str) -> std::time::Duration {
    if adapter == "workspace" {
        WORKSPACE_ADAPTER_TIMEOUT
    } else if adapter == "provider-cache-status" {
        PROVIDER_STATUS_ADAPTER_TIMEOUT
    } else {
        DEFAULT_ADAPTER_TIMEOUT
    }
}

struct Connection {
    epoch: String,
    colocated: bool,
    protocol: u16,
    tx: mpsc::UnboundedSender<MachineCommand>,
    connected_at: std::time::Instant,
}

#[derive(Default)]
pub struct MachineControl {
    connections: RwLock<HashMap<String, Connection>>,
    events: RwLock<HashMap<String, Vec<MachineEvent>>>,
    plugin_inventory: RwLock<HashMap<String, Vec<PluginInventory>>>,
    pending: RwLock<HashMap<String, PendingResponse>>,
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
        self.plugin_inventory.write().remove(&machine_id);
        self.connections.write().insert(
            machine_id,
            Connection {
                epoch,
                colocated,
                protocol,
                tx,
                connected_at: std::time::Instant::now(),
            },
        );
    }

    /// Age of the current authenticated Machine transport. `None` means the
    /// Controller has no live command channel for this Machine.
    #[must_use]
    pub fn connection_age(&self, machine_id: &str) -> Option<std::time::Duration> {
        self.connections
            .read()
            .get(machine_id)
            .map(|connection| connection.connected_at.elapsed())
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
            self.plugin_inventory.write().remove(machine_id);
        }
    }

    /// Drop the active command channel for a revoked Machine regardless of
    /// connection epoch. The WebSocket task observes the closed receiver and
    /// exits; subsequent reconnects fail durable identity validation.
    pub fn disconnect(&self, machine_id: &str) {
        self.connections.write().remove(machine_id);
        self.plugin_inventory.write().remove(machine_id);
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
        // Credential refresh candidates are consumed directly by the
        // authenticated Machine WebSocket handler. They must never enter the
        // ordinary in-memory history exposed by the Machine events API, even
        // if a future caller accidentally routes one through this method.
        if matches!(&event, MachineEvent::ProviderAuthRefreshCandidate { .. }) {
            return;
        }
        if let MachineEvent::PluginHostResponse {
            request_id,
            accepted,
            started,
            payload,
            detail,
        } = &event
        {
            if let Some(sender) = self.pending.write().remove(request_id) {
                let _ = sender.send(Ok(serde_json::json!({
                    "accepted": accepted,
                    "started": started,
                    "payload": payload,
                    "detail": detail,
                })));
            }
            return;
        }
        if let MachineEvent::PluginInventory { plugins, .. } = &event {
            self.plugin_inventory
                .write()
                .insert(machine_id.to_owned(), plugins.clone());
        }
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
        let _pending = PendingRequestGuard {
            pending: &self.pending,
            request_id: &request_id,
        };
        self.send(
            machine_id,
            MachineCommand::AdapterRequest {
                request_id: request_id.clone(),
                adapter: adapter.to_owned(),
                payload,
            },
        )?;
        match tokio::time::timeout(adapter_timeout(adapter), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Machine adapter response channel closed".to_owned()),
            Err(_) => Err("Machine adapter request timed out".to_owned()),
        }
    }

    pub async fn command_request(
        &self,
        machine_id: &str,
        request_id: String,
        command: MachineCommand,
    ) -> Result<(), String> {
        self.command_request_with_timeout(machine_id, request_id, command, PROVIDER_COMMAND_TIMEOUT)
            .await
    }

    pub async fn command_request_with_timeout(
        &self,
        machine_id: &str,
        request_id: String,
        command: MachineCommand,
        timeout: std::time::Duration,
    ) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.pending.write().insert(request_id.clone(), tx);
        let _pending = PendingRequestGuard {
            pending: &self.pending,
            request_id: &request_id,
        };
        self.send(machine_id, command)?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(_))) => Ok(()),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(_)) => Err("Machine command response channel closed".to_owned()),
            Err(_) => Err("Machine command timed out".to_owned()),
        }
    }

    pub async fn plugin_host_request(
        &self,
        machine_id: &str,
        plugin: &PluginInventory,
        operation: PluginHostOperation,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, PluginHostRequestError> {
        let request_id = format!(
            "plugin-host-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |value| value.as_nanos())
        );
        let (tx, rx) = oneshot::channel();
        self.pending.write().insert(request_id.clone(), tx);
        let _pending = PendingRequestGuard {
            pending: &self.pending,
            request_id: &request_id,
        };
        self.send(
            machine_id,
            MachineCommand::InvokePluginHost {
                request_id: request_id.clone(),
                plugin_id: plugin.plugin_id.clone(),
                plugin_version: plugin.plugin_version.clone(),
                generation_digest: plugin.generation_digest.clone(),
                auth_generation: plugin.auth_generation,
                operation,
                payload,
            },
        )
        .map_err(|detail| PluginHostRequestError {
            started: false,
            detail,
        })?;
        let response = match tokio::time::timeout(PLUGIN_HOST_TIMEOUT, rx).await {
            Ok(Ok(Ok(response))) => response,
            Ok(Ok(Err(detail))) => {
                return Err(PluginHostRequestError {
                    started: true,
                    detail,
                });
            }
            Ok(Err(_)) => {
                return Err(PluginHostRequestError {
                    started: true,
                    detail: "Machine Plugin host response channel closed".to_owned(),
                });
            }
            Err(_) => {
                return Err(PluginHostRequestError {
                    started: true,
                    detail: "Machine Plugin host request timed out".to_owned(),
                });
            }
        };
        let accepted = response
            .get("accepted")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let started = response
            .get("started")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        if !accepted {
            return Err(PluginHostRequestError {
                started,
                detail: response
                    .get("detail")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Machine rejected Plugin host request")
                    .to_owned(),
            });
        }
        response
            .get("payload")
            .cloned()
            .filter(|payload| !payload.is_null())
            .ok_or_else(|| PluginHostRequestError {
                started: true,
                detail: "Machine Plugin host response has no payload".to_owned(),
            })
    }

    #[must_use]
    pub fn connected_machine_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self.connections.read().keys().cloned().collect();
        ids.sort();
        ids
    }

    #[must_use]
    pub(crate) fn connected_plugin_inventory(&self) -> Vec<ConnectedPluginInventory> {
        let connections = self.connections.read();
        let inventories = self.plugin_inventory.read();
        let mut inventory = connections
            .iter()
            .filter(|(_, connection)| connection.protocol >= PLUGIN_HOST_EXECUTION_PROTOCOL_VERSION)
            .filter_map(|(machine_id, _)| {
                inventories
                    .get(machine_id)
                    .cloned()
                    .map(|plugins| (machine_id, plugins))
            })
            .flat_map(|(machine_id, plugins)| {
                plugins.into_iter().map(|plugin| ConnectedPluginInventory {
                    machine_id: machine_id.clone(),
                    plugin,
                })
            })
            .collect::<Vec<_>>();
        inventory.sort_by(|left, right| {
            left.machine_id
                .cmp(&right.machine_id)
                .then(left.plugin.plugin_id.cmp(&right.plugin.plugin_id))
        });
        inventory
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
    use crate::machine_protocol::{
        PluginInstallationState, ProviderMaterializationState, ProviderReplicaState,
    };

    fn plugin_inventory(id: &str) -> PluginInventory {
        PluginInventory {
            plugin_id: id.to_owned(),
            plugin_version: "1.2.3".to_owned(),
            plugin_kind: cowboy_plugin_sdk::PluginKind::AgentProvider,
            generation_digest: format!("sha256:{}", "ab".repeat(32)),
            contract_fingerprint: format!("sha256:{}", "cd".repeat(32)),
            state: PluginInstallationState::Active,
            rollback_generation_digest: None,
            active_session_leases: 0,
            auth_generation: Some(4),
            replica_state: ProviderReplicaState::Current,
            materialization_state: ProviderMaterializationState::Current,
            detail: None,
        }
    }

    #[test]
    fn workspace_requests_cover_the_complete_machine_preparation_envelope() {
        assert_eq!(
            adapter_timeout("workspace"),
            std::time::Duration::from_secs(330)
        );
        assert_eq!(adapter_timeout("zed"), std::time::Duration::from_secs(40));
        assert_eq!(
            adapter_timeout("provider-cache-status"),
            std::time::Duration::from_secs(3)
        );
    }

    #[test]
    fn provider_refresh_credentials_never_enter_machine_event_history() {
        let control = MachineControl::default();
        control.record(
            "hawk",
            MachineEvent::ProviderAuthRefreshCandidate {
                request_id: "refresh-1".to_owned(),
                provider_id: "grok".to_owned(),
                expected_generation: 3,
                provider_version: "1.1.8".to_owned(),
                generation_digest: format!("sha256:{}", "ab".repeat(32)),
                auth_contract_fingerprint: format!("sha256:{}", "cd".repeat(32)),
                portable_schema: "grok-auth-v1".to_owned(),
                auth_method: "xai-account".to_owned(),
                bundle: std::collections::BTreeMap::from([(
                    "auth_json".to_owned(),
                    "c2VjcmV0".to_owned(),
                )]),
            },
        );

        assert!(control.events("hawk").is_empty());
    }

    #[tokio::test]
    async fn plugin_host_payload_is_correlated_without_entering_event_history() {
        let control = Arc::new(MachineControl::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        control.install("hawk".to_owned(), "epoch".to_owned(), false, 7, tx);
        let plugin = plugin_inventory("codex");
        let requester = Arc::clone(&control);
        let request = tokio::spawn(async move {
            requester
                .plugin_host_request(
                    "hawk",
                    &plugin,
                    PluginHostOperation::CollectUsage,
                    serde_json::json!({ "provider": "openai" }),
                )
                .await
        });
        let MachineCommand::InvokePluginHost { request_id, .. } =
            rx.recv().await.expect("Plugin host command")
        else {
            panic!("wrong command");
        };
        control.record(
            "hawk",
            MachineEvent::PluginHostResponse {
                request_id,
                accepted: true,
                started: true,
                payload: Some(serde_json::json!({ "account": { "email": "private" } })),
                detail: None,
            },
        );
        assert_eq!(
            request.await.expect("request task").expect("host response")["account"]["email"],
            "private"
        );
        assert!(control.events("hawk").is_empty());
    }

    #[tokio::test]
    async fn plugin_host_preflight_rejection_preserves_not_started() {
        let control = Arc::new(MachineControl::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        control.install("hawk".to_owned(), "epoch".to_owned(), false, 7, tx);
        let plugin = plugin_inventory("codex");
        let requester = Arc::clone(&control);
        let request = tokio::spawn(async move {
            requester
                .plugin_host_request(
                    "hawk",
                    &plugin,
                    PluginHostOperation::ResetUsage,
                    serde_json::json!({}),
                )
                .await
        });
        let MachineCommand::InvokePluginHost { request_id, .. } =
            rx.recv().await.expect("Plugin host command")
        else {
            panic!("wrong command");
        };
        control.record(
            "hawk",
            MachineEvent::PluginHostResponse {
                request_id,
                accepted: false,
                started: false,
                payload: None,
                detail: Some("exact generation changed".to_owned()),
            },
        );
        let error = request
            .await
            .expect("request task")
            .expect_err("preflight rejection");
        assert!(!error.started);
        assert_eq!(error.detail, "exact generation changed");
        assert!(control.events("hawk").is_empty());
    }

    #[test]
    fn exact_host_inventory_is_independent_of_bounded_history_and_old_protocols() {
        let control = MachineControl::default();
        let (old_tx, _) = mpsc::unbounded_channel();
        control.install("old".to_owned(), "old-epoch".to_owned(), false, 6, old_tx);
        control.record(
            "old",
            MachineEvent::PluginInventory {
                plugins: vec![plugin_inventory("codex")],
                observed_at_ms: 1,
            },
        );
        let (current_tx, _) = mpsc::unbounded_channel();
        control.install(
            "current".to_owned(),
            "current-epoch".to_owned(),
            false,
            7,
            current_tx,
        );
        control.record(
            "current",
            MachineEvent::PluginInventory {
                plugins: vec![plugin_inventory("codex")],
                observed_at_ms: 1,
            },
        );
        for index in 0..70 {
            control.record(
                "current",
                MachineEvent::CommandResult {
                    request_id: format!("unmatched-{index}"),
                    accepted: true,
                    detail: None,
                },
            );
        }
        let inventory = control.connected_plugin_inventory();
        assert_eq!(inventory.len(), 1);
        assert_eq!(inventory[0].machine_id, "current");
        assert_eq!(inventory[0].plugin.plugin_id, "codex");
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

    #[tokio::test]
    async fn command_response_is_correlated_without_polling_event_history() {
        let control = Arc::new(MachineControl::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        control.install("mac".to_owned(), "epoch".to_owned(), false, 3, tx);
        let requester = Arc::clone(&control);
        let request = tokio::spawn(async move {
            requester
                .command_request_with_timeout(
                    "mac",
                    "refresh-1".to_owned(),
                    MachineCommand::RefreshInventory {
                        request_id: "refresh-1".to_owned(),
                    },
                    std::time::Duration::from_secs(1),
                )
                .await
        });

        assert_eq!(
            rx.recv().await.expect("command"),
            MachineCommand::RefreshInventory {
                request_id: "refresh-1".to_owned(),
            }
        );
        control.record(
            "mac",
            MachineEvent::CommandResult {
                request_id: "refresh-1".to_owned(),
                accepted: true,
                detail: None,
            },
        );

        request.await.expect("task").expect("response");
        assert!(control.pending.read().is_empty());
    }

    #[tokio::test]
    async fn command_timeout_removes_the_pending_correlation() {
        let control = MachineControl::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        control.install("mac".to_owned(), "epoch".to_owned(), false, 3, tx);

        let error = control
            .command_request_with_timeout(
                "mac",
                "refresh-timeout".to_owned(),
                MachineCommand::RefreshInventory {
                    request_id: "refresh-timeout".to_owned(),
                },
                std::time::Duration::from_millis(1),
            )
            .await
            .expect_err("timeout");

        assert_eq!(error, "Machine command timed out");
        assert!(control.pending.read().is_empty());
    }

    #[tokio::test]
    async fn cancelled_command_wait_removes_the_pending_correlation() {
        let control = Arc::new(MachineControl::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        control.install("mac".to_owned(), "epoch".to_owned(), false, 3, tx);
        let requester = Arc::clone(&control);
        let request = tokio::spawn(async move {
            requester
                .command_request_with_timeout(
                    "mac",
                    "refresh-cancelled".to_owned(),
                    MachineCommand::RefreshInventory {
                        request_id: "refresh-cancelled".to_owned(),
                    },
                    std::time::Duration::from_mins(1),
                )
                .await
        });
        let _ = rx.recv().await.expect("command");

        request.abort();
        let _ = request.await;

        assert!(control.pending.read().is_empty());
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
    fn plugin_uninstall_and_compensation_require_machine_protocol_five() {
        let control = MachineControl::default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        control.install("old".to_owned(), "epoch".to_owned(), false, 3, tx);
        let digest = format!("sha256:{}", "ab".repeat(32));
        for command in [
            MachineCommand::UninstallPlugin {
                request_id: "uninstall".to_owned(),
                plugin_id: "gemini".to_owned(),
                generation_digest: digest.clone(),
            },
            MachineCommand::ReactivatePlugin {
                request_id: "reactivate".to_owned(),
                plugin_id: "gemini".to_owned(),
                generation_digest: digest.clone(),
            },
        ] {
            let error = control.send("old", command).unwrap_err();
            assert!(error.contains("requires 5"));
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn plugin_host_execution_requires_machine_protocol_seven() {
        let control = MachineControl::default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        control.install("old".to_owned(), "epoch".to_owned(), false, 6, tx);
        let plugin = plugin_inventory("codex");
        let error = control
            .send(
                "old",
                MachineCommand::InvokePluginHost {
                    request_id: "usage".to_owned(),
                    plugin_id: plugin.plugin_id,
                    plugin_version: plugin.plugin_version,
                    generation_digest: plugin.generation_digest,
                    auth_generation: plugin.auth_generation,
                    operation: PluginHostOperation::CollectUsage,
                    payload: serde_json::json!({}),
                },
            )
            .unwrap_err();
        assert!(error.contains("requires 7"));
        assert!(rx.try_recv().is_err());
    }
}
