//! Live command channels for authenticated Machine hosts.
//!
//! Connection identity, inventory, request correlation and enqueue share one
//! lock. No lock survives an await. Transport scope is NOT session ownership.

#![warn(clippy::pedantic)]

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use parking_lot::RwLock;
use tokio::sync::{mpsc, oneshot};

mod telemetry_export;
mod telemetry_recovery;

use crate::machine_protocol::plugin_recovery::RecoveryObservation;
use crate::machine_protocol::plugin_step::{StepLookup, StepObservation, UninstallStep};
use crate::machine_protocol::telemetry_binding::{
    BindingCommitResult, BindingObservation, BindingStep,
};
use crate::machine_protocol::{
    MachineCommand, MachineEvent, PLUGIN_HOST_EXECUTION_PROTOCOL_VERSION, PluginHostOperation,
    PluginInstallationState, PluginInventory,
};

const DEFAULT_ADAPTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(40);
// Ten sequential Git commands may each take 30 seconds on the Machine.
const WORKSPACE_ADAPTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(330);
const PROVIDER_STATUS_ADAPTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
const PROVIDER_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
const PLUGIN_HOST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
const MAX_PENDING: usize = 4096;
const MAX_PENDING_PER_MACHINE: usize = 256;

/// Created only by installing an authenticated channel. Not deserializable;
/// even accidentally reusing an epoch string creates a distinct incarnation.
#[derive(Clone, Debug)]
pub(crate) struct ConnectionToken(Arc<ConnectionIdentity>);

#[derive(Debug)]
struct ConnectionIdentity {
    machine_id: String,
    epoch: String,
}

impl ConnectionToken {
    fn same(&self, other: &Self) -> bool {
        self.0.epoch == other.0.epoch && Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Core-originated login projection, never a remote RPC response or inventory.
pub(crate) struct ServiceLoginNotice {
    pub request_id: String,
    pub provider: String,
    pub state: crate::machine_protocol::AuthState,
    pub account_label: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug)]
pub(crate) struct PluginHostRequestError {
    /// Conservative: true after enqueue means execution may have started.
    /// Losing a connection/receipt is not evidence that an effect was undone.
    pub started: bool,
    pub detail: String,
}

/// Receipt certainty, not an assumption that a rejected command had no effects.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CommandFailure {
    NotSent,
    Rejected,
    Unknown,
}

#[derive(Debug)]
pub(crate) struct CommandRequestError {
    pub certainty: CommandFailure,
    pub detail: String,
}

#[derive(Clone, Copy)]
enum RequestBinding<'a> {
    Plugin(&'a PluginHostBinding),
    Connection(&'a ConnectionToken),
    Reactivate(&'a ConnectionToken, RetainedPluginTarget<'a>),
    Telemetry(&'a ConnectionToken, &'a BindingStep),
    TelemetryExport(
        &'a ConnectionToken,
        &'a crate::machine_protocol::telemetry_export::ExportAttempt,
    ),
}

#[derive(Clone, Copy)]
pub(crate) struct RetainedPluginTarget<'a> {
    pub plugin_id: &'a str,
    pub version: &'a str,
    pub digest: &'a str,
    pub fingerprint: &'a str,
}

#[derive(Debug, Clone)]
pub(crate) struct ConnectedPluginInventory {
    pub machine_id: String,
    pub plugin: PluginInventory,
}

/// A single local remote-port binding; contains no secret or serialized grant.
/// The Machine still independently authorizes and validates the exact release.
#[derive(Debug)]
pub(crate) struct PluginHostBinding {
    connection: ConnectionToken,
    inventory_revision: Arc<()>,
    plugin: PluginInventory,
    operation: PluginHostOperation,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReplyKind {
    Adapter,
    Command,
    PluginHost,
    PluginStep,
    PluginRecovery,
    TelemetryBinding,
    TelemetryBindingCommit,
    TelemetryExport,
    TelemetryRecovery,
    TelemetryRecoveryCommit,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PluginUninstallTransport {
    Legacy,
    Leased,
}

enum Reply {
    PluginStep(Box<StepObservation>),
    PluginRecovery(Box<RecoveryObservation>),
    TelemetryBinding(Box<BindingObservation>),
    TelemetryBindingCommit(Box<BindingCommitResult>),
    TelemetryExport(Option<Box<crate::machine_protocol::telemetry_export::ExportReceipt>>),
    TelemetryRecovery(Box<crate::machine_protocol::telemetry_recovery::RecoveryObservation>),
    TelemetryRecoveryCommit(Box<crate::machine_protocol::telemetry_recovery::RecoveryResult>),
    Adapter(Result<serde_json::Value, String>),
    Command(Result<(), String>),
    PluginHost {
        accepted: bool,
        started: bool,
        payload: Option<serde_json::Value>,
        detail: Option<String>,
    },
}

impl Reply {
    const fn kind(&self) -> ReplyKind {
        match self {
            Self::PluginStep(_) => ReplyKind::PluginStep,
            Self::PluginRecovery(_) => ReplyKind::PluginRecovery,
            Self::TelemetryBinding(_) => ReplyKind::TelemetryBinding,
            Self::TelemetryBindingCommit(_) => ReplyKind::TelemetryBindingCommit,
            Self::TelemetryExport(_) => ReplyKind::TelemetryExport,
            Self::TelemetryRecovery(_) => ReplyKind::TelemetryRecovery,
            Self::TelemetryRecoveryCommit(_) => ReplyKind::TelemetryRecoveryCommit,
            Self::Adapter(_) => ReplyKind::Adapter,
            Self::Command(_) => ReplyKind::Command,
            Self::PluginHost { .. } => ReplyKind::PluginHost,
        }
    }
}

struct PendingResponse {
    connection: ConnectionToken,
    kind: ReplyKind,
    ticket: Arc<()>,
    sender: oneshot::Sender<Reply>,
}

struct PendingRequestGuard<'a> {
    live: &'a RwLock<LiveState>,
    request_id: &'a str,
    ticket: Arc<()>,
}

impl Drop for PendingRequestGuard<'_> {
    fn drop(&mut self) {
        let mut live = self.live.write();
        // A late finalizer cannot remove a newer waiter that reused the key.
        if live
            .pending
            .get(self.request_id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.ticket, &self.ticket))
        {
            live.pending.remove(self.request_id);
        }
    }
}

fn adapter_timeout(adapter: &str) -> std::time::Duration {
    match adapter {
        "workspace" => WORKSPACE_ADAPTER_TIMEOUT,
        "provider-cache-status" => PROVIDER_STATUS_ADAPTER_TIMEOUT,
        _ => DEFAULT_ADAPTER_TIMEOUT,
    }
}

struct Connection {
    token: ConnectionToken,
    colocated: bool,
    protocol: u16,
    tx: mpsc::UnboundedSender<MachineCommand>,
    connected_at: std::time::Instant,
}

struct PluginInventorySnapshot {
    plugins: Vec<PluginInventory>,
    // Local observation fence, NOT a durable Machine installation generation.
    revision: Arc<()>,
}

fn same_installation(left: &PluginInventory, right: &PluginInventory) -> bool {
    left.plugin_id == right.plugin_id
        && left.plugin_kind == right.plugin_kind
        && left.plugin_version == right.plugin_version
        && left.generation_digest == right.generation_digest
        && left.installation_revision == right.installation_revision
        && left.contract_fingerprint == right.contract_fingerprint
        && left.state == right.state
        && left.auth_generation == right.auth_generation
}

#[derive(Default)]
struct LiveState {
    connections: HashMap<String, Connection>,
    events: HashMap<String, Vec<MachineEvent>>,
    plugin_inventory: HashMap<String, PluginInventorySnapshot>,
    pending: HashMap<String, PendingResponse>,
}

impl LiveState {
    fn telemetry_target_matches(&self, machine: &str, step: &BindingStep) -> bool {
        let Ok(after) = step.after() else {
            return false;
        };
        let Some(target) = after.selection else {
            return true;
        };
        self.telemetry_installation_matches(machine, &target)
    }

    fn telemetry_installation_matches(
        &self,
        machine: &str,
        target: &crate::machine_protocol::telemetry_binding::BindingInstallation,
    ) -> bool {
        self.plugin_inventory.get(machine).is_some_and(|inventory| {
            let mut slot = inventory
                .plugins
                .iter()
                .filter(|p| p.plugin_id == target.plugin_id);
            slot.next().is_some_and(|plugin| {
                plugin.plugin_kind == cowboy_plugin_sdk::PluginKind::TelemetryBackend
                    && plugin.state == PluginInstallationState::Active
                    && plugin.auth_generation.is_none()
                    && plugin.plugin_version == target.plugin_version
                    && plugin.generation_digest == String::from(target.generation_digest.clone())
                    && plugin.contract_fingerprint
                        == String::from(target.contract_fingerprint.clone())
                    && plugin.installation_revision.as_ref() == Some(&target.installation_revision)
            }) && slot.next().is_none()
        })
    }

    fn is_current(&self, token: &ConnectionToken) -> bool {
        self.connections
            .get(&token.0.machine_id)
            .is_some_and(|connection| connection.token.same(token))
    }

    fn disconnect(&mut self, machine_id: &str) {
        self.connections.remove(machine_id);
        self.plugin_inventory.remove(machine_id);
        // Drop only this channel's RPC observation resources. Do not cancel
        // Machine operations, remove workers, or delete a session/worktree.
        self.pending
            .retain(|_, request| request.connection.0.machine_id != machine_id);
    }

    fn remember(&mut self, machine_id: &str, event: MachineEvent) {
        let history = self.events.entry(machine_id.to_owned()).or_default();
        history.push(event);
        if history.len() > 64 {
            history.drain(..history.len() - 64);
        }
    }

    fn complete(&mut self, token: &ConnectionToken, request_id: &str, reply: Reply) {
        if self
            .pending
            .get(request_id)
            .is_some_and(|request| request.connection.same(token) && request.kind == reply.kind())
            && let Some(request) = self.pending.remove(request_id)
        {
            let _ = request.sender.send(reply);
        }
    }

    fn plugin_matches(&self, machine_id: &str, expected: &PluginInventory) -> bool {
        self.plugin_inventory
            .get(machine_id)
            .is_some_and(|inventory| {
                let mut matches = inventory
                    .plugins
                    .iter()
                    .filter(|plugin| plugin.plugin_id == expected.plugin_id);
                matches.next().is_some_and(|plugin| {
                    plugin.state == PluginInstallationState::Active
                        && same_installation(plugin, expected)
                }) && matches.next().is_none()
            })
    }

    fn may_reactivate(&self, machine_id: &str, target: RetainedPluginTarget<'_>) -> bool {
        self.plugin_inventory
            .get(machine_id)
            .is_some_and(|inventory| {
                let mut slot = inventory
                    .plugins
                    .iter()
                    .filter(|p| p.plugin_id == target.plugin_id);
                let matches = slot.next().is_none_or(|p| {
                    p.state == PluginInstallationState::Active
                        && p.plugin_version == target.version
                        && p.generation_digest == target.digest
                        && p.contract_fingerprint == target.fingerprint
                });
                matches && slot.next().is_none()
            })
    }

    fn observe_plugins(&mut self, machine_id: &str, plugins: &[PluginInventory]) {
        let unchanged = self.plugin_inventory.get(machine_id).filter(|previous| {
            previous.plugins.len() == plugins.len()
                && previous
                    .plugins
                    .iter()
                    .zip(plugins)
                    .all(|(a, b)| same_installation(a, b))
        });
        let revision = unchanged.map_or_else(|| Arc::new(()), |old| Arc::clone(&old.revision));
        self.plugin_inventory.insert(
            machine_id.to_owned(),
            PluginInventorySnapshot {
                plugins: plugins.to_vec(),
                revision,
            },
        );
    }
}

#[derive(Default)]
pub struct MachineControl {
    live: RwLock<LiveState>,
    next_request: AtomicU64,
}

impl MachineControl {
    pub(crate) fn install(
        &self,
        machine_id: String,
        epoch: String,
        colocated: bool,
        protocol: u16,
        tx: mpsc::UnboundedSender<MachineCommand>,
    ) -> ConnectionToken {
        let token = ConnectionToken(Arc::new(ConnectionIdentity {
            machine_id: machine_id.clone(),
            epoch,
        }));
        let mut live = self.live.write();
        live.disconnect(&machine_id);
        live.connections.insert(
            machine_id,
            Connection {
                token: token.clone(),
                colocated,
                protocol,
                tx,
                connected_at: std::time::Instant::now(),
            },
        );
        token
    }

    #[must_use]
    pub(crate) fn is_current(&self, token: &ConnectionToken) -> bool {
        self.live.read().is_current(token)
    }

    /// Age of the current authenticated transport, not of detached sessions.
    #[must_use]
    pub fn connection_age(&self, machine_id: &str) -> Option<std::time::Duration> {
        self.live
            .read()
            .connections
            .get(machine_id)
            .map(|connection| connection.connected_at.elapsed())
    }

    #[must_use]
    pub fn is_colocated(&self, machine_id: &str) -> Option<bool> {
        self.live
            .read()
            .connections
            .get(machine_id)
            .map(|connection| connection.colocated)
    }

    pub(crate) fn remove_if_current(&self, token: &ConnectionToken) {
        let mut live = self.live.write();
        if live.is_current(token) {
            live.disconnect(&token.0.machine_id);
        }
    }

    /// Revocation removes the channel and its waiters, not detached sessions.
    pub fn disconnect(&self, machine_id: &str) {
        self.live.write().disconnect(machine_id);
    }

    pub fn send(&self, machine_id: &str, command: MachineCommand) -> Result<(), String> {
        let live = self.live.read();
        let connection = live
            .connections
            .get(machine_id)
            .ok_or_else(|| "Machine is not connected".to_owned())?;
        Self::check_protocol(connection, &command)?;
        connection
            .tx
            .send(command)
            .map_err(|_| "Machine disconnected".to_owned())
    }

    fn check_protocol(connection: &Connection, command: &MachineCommand) -> Result<(), String> {
        let required = command.minimum_protocol();
        if connection.protocol < required {
            return Err(format!(
                "Machine negotiated protocol {}, but this command requires {required}",
                connection.protocol
            ));
        }
        Ok(())
    }

    pub(crate) fn record_service_login(&self, machine_id: &str, notice: ServiceLoginNotice) {
        self.live.write().remember(
            machine_id,
            MachineEvent::LoginState {
                request_id: notice.request_id,
                provider: notice.provider,
                state: notice.state,
                account_label: notice.account_label,
                detail: notice.detail,
            },
        );
    }

    /// Remote callers cannot invent a source Machine/epoch. Late or mismatched
    /// replies are dropped before touching a waiter or retaining their payload.
    #[allow(clippy::too_many_lines)] // Keep the closed reply dispatch under one correlation lock.
    pub(crate) fn record_remote(&self, token: &ConnectionToken, event: MachineEvent) {
        let mut live = self.live.write();
        if !live.is_current(token) {
            return;
        }
        let machine_id = &token.0.machine_id;
        match event {
            MachineEvent::TelemetryBindingRecovered { request_id, result } => {
                live.complete(token, &request_id, Reply::TelemetryRecoveryCommit(result));
            }
            MachineEvent::TelemetryRecoveryObservation {
                request_id,
                observation,
            } => {
                live.complete(token, &request_id, Reply::TelemetryRecovery(observation));
            }
            MachineEvent::TelemetryExported {
                request_id,
                receipt,
            } => {
                live.complete(token, &request_id, Reply::TelemetryExport(receipt));
            }
            MachineEvent::TelemetryBindingCommitted { request_id, result } => {
                live.complete(token, &request_id, Reply::TelemetryBindingCommit(result));
            }
            MachineEvent::TelemetryBindingObservation {
                request_id,
                observation,
            } => {
                live.complete(token, &request_id, Reply::TelemetryBinding(observation));
            }
            MachineEvent::PluginUninstallRecovery {
                request_id,
                observation,
            } => {
                live.complete(token, &request_id, Reply::PluginRecovery(observation));
            }
            MachineEvent::PluginUninstallStep {
                request_id,
                observation,
            } => {
                live.complete(token, &request_id, Reply::PluginStep(observation));
            }
            MachineEvent::ProviderAuthRefreshCandidate { .. }
            | MachineEvent::ServiceAuthCandidate { .. } => {}
            MachineEvent::PluginHostResponse {
                request_id,
                accepted,
                started,
                payload,
                detail,
            } => {
                live.complete(
                    token,
                    &request_id,
                    Reply::PluginHost {
                        accepted,
                        started,
                        payload,
                        detail,
                    },
                );
            }
            MachineEvent::AdapterResponse {
                request_id,
                accepted,
                payload,
                detail,
            } => {
                let result = if accepted {
                    payload.ok_or_else(|| "adapter response has no payload".to_owned())
                } else {
                    Err(detail.unwrap_or_else(|| "adapter request rejected".to_owned()))
                };
                live.complete(token, &request_id, Reply::Adapter(result));
            }
            MachineEvent::CommandResult {
                request_id,
                accepted,
                detail,
            } => {
                let result = if accepted {
                    Ok(())
                } else {
                    Err(detail
                        .clone()
                        .unwrap_or_else(|| "Machine command rejected".to_owned()))
                };
                live.complete(token, &request_id, Reply::Command(result));
                // Login commands also have an asynchronous UI observer. Keep
                // their bounded status projection, never host/adapter payloads.
                live.remember(
                    machine_id,
                    MachineEvent::CommandResult {
                        request_id,
                        accepted,
                        detail,
                    },
                );
            }
            event => {
                if let MachineEvent::PluginInventory { plugins, .. } = &event {
                    live.observe_plugins(machine_id, plugins);
                }
                live.remember(machine_id, event);
            }
        }
    }

    fn request_id(&self, prefix: &str) -> Result<String, String> {
        let sequence = self
            .next_request
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| "Machine request identity exhausted".to_owned())?;
        Ok(format!("{prefix}-{}-{sequence}", std::process::id()))
    }

    fn begin_request<'a>(
        &'a self,
        machine_id: &str,
        request_id: &'a str,
        command: MachineCommand,
        kind: ReplyKind,
        binding: Option<RequestBinding<'_>>,
    ) -> Result<(oneshot::Receiver<Reply>, PendingRequestGuard<'a>), String> {
        let mut live = self.live.write();
        let connection = live
            .connections
            .get(machine_id)
            .ok_or_else(|| "Machine is not connected".to_owned())?;
        Self::check_protocol(connection, &command)?;
        if let Some(
            RequestBinding::Connection(token)
            | RequestBinding::Reactivate(token, _)
            | RequestBinding::Telemetry(token, _)
            | RequestBinding::TelemetryExport(token, _),
        ) = binding
            && !connection.token.same(token)
        {
            return Err("Machine operation connection is no longer current".to_owned());
        }
        if let Some(RequestBinding::Telemetry(_, step)) = binding
            && !live.telemetry_target_matches(machine_id, step)
        {
            return Err("Telemetry binding installation changed before dispatch".to_owned());
        }
        if let Some(RequestBinding::TelemetryExport(_, attempt)) = binding
            && !attempt
                .binding
                .selection
                .as_ref()
                .is_some_and(|target| live.telemetry_installation_matches(machine_id, target))
        {
            return Err("Managed telemetry installation changed before dispatch".to_owned());
        }
        if let Some(RequestBinding::Reactivate(_, target)) = binding
            && !live.may_reactivate(machine_id, target)
        {
            return Err(
                "Plugin recovery inventory is missing or the installation changed".to_owned(),
            );
        }
        if let Some(RequestBinding::Plugin(binding)) = binding
            && (!connection.token.same(&binding.connection)
                || !live
                    .plugin_inventory
                    .get(machine_id)
                    .is_some_and(|inventory| {
                        Arc::ptr_eq(&inventory.revision, &binding.inventory_revision)
                    })
                || !live.plugin_matches(machine_id, &binding.plugin))
        {
            return Err("Plugin binding is no longer current".to_owned());
        }
        if live.pending.contains_key(request_id) {
            return Err("Machine request id is already pending".to_owned());
        }
        if live.pending.len() >= MAX_PENDING
            || live
                .pending
                .values()
                .filter(|p| p.connection.same(&connection.token))
                .count()
                >= MAX_PENDING_PER_MACHINE
        {
            return Err("Machine pending request budget exceeded".to_owned());
        }
        let token = connection.token.clone();
        let outgoing = connection.tx.clone();
        let ticket = Arc::new(());
        let (sender, receiver) = oneshot::channel();
        live.pending.insert(
            request_id.to_owned(),
            PendingResponse {
                connection: token,
                kind,
                ticket: Arc::clone(&ticket),
                sender,
            },
        );
        // Registration, binding validation and enqueue are atomic with replace/revoke.
        if outgoing.send(command).is_err() {
            live.pending.remove(request_id);
            return Err("Machine disconnected".to_owned());
        }
        drop(live);
        Ok((
            receiver,
            PendingRequestGuard {
                live: &self.live,
                request_id,
                ticket,
            },
        ))
    }

    pub async fn adapter_request(
        &self,
        machine_id: &str,
        adapter: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let request_id = self.request_id("adapter")?;
        let (rx, _pending) = self.begin_request(
            machine_id,
            &request_id,
            MachineCommand::AdapterRequest {
                request_id: request_id.clone(),
                adapter: adapter.to_owned(),
                payload,
            },
            ReplyKind::Adapter,
            None,
        )?;
        match tokio::time::timeout(adapter_timeout(adapter), rx).await {
            Ok(Ok(Reply::Adapter(result))) => result,
            Ok(Ok(_)) => Err("Machine adapter reply kind mismatch".to_owned()),
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

    /// Caller-generated command correlation IDs must be fresh per invocation;
    /// these are not persistent operation/idempotency IDs.
    pub async fn command_request_with_timeout(
        &self,
        machine_id: &str,
        request_id: String,
        command: MachineCommand,
        timeout: std::time::Duration,
    ) -> Result<(), String> {
        self.command_receipt(machine_id, request_id, command, timeout, None)
            .await
            .map_err(|error| error.detail)
    }

    /// Short-lived connection evidence. Never saved in the operation journal or
    /// reacquired after restart to replay an old destructive command.
    pub(crate) fn operation_connection(&self, machine_id: &str) -> Result<ConnectionToken, String> {
        self.live
            .read()
            .connections
            .get(machine_id)
            .map(|connection| connection.token.clone())
            .ok_or_else(|| "Machine is not connected".to_owned())
    }

    pub(crate) fn plugin_uninstall_transport(
        &self,
        token: &ConnectionToken,
        step: &UninstallStep,
    ) -> Result<PluginUninstallTransport, String> {
        step.validate()
            .map_err(|_| "Invalid Plugin uninstall intent".to_owned())?;
        if step.machine_id != token.0.machine_id {
            return Err("Plugin uninstall target mismatch".to_owned());
        }
        let live = self.live.read();
        let protocol = live
            .connections
            .get(&token.0.machine_id)
            .filter(|c| c.token.same(token))
            .map(|c| c.protocol)
            .ok_or_else(|| "Machine connection changed before Plugin preflight".to_owned())?;
        if protocol >= crate::machine_protocol::PLUGIN_EXECUTION_LEASE_PROTOCOL_VERSION {
            Ok(PluginUninstallTransport::Leased)
        } else if step.schema == 1
            && (5..crate::machine_protocol::PLUGIN_STEP_PROTOCOL_VERSION).contains(&protocol)
        {
            Ok(PluginUninstallTransport::Legacy)
        } else {
            Err(
                "Update the Machine for bounded Plugin execution; no workers were stopped"
                    .to_owned(),
            )
        }
    }

    /// Historical receipt queries retain their original protocol floor.
    pub(crate) fn durable_plugin_steps(&self, token: &ConnectionToken) -> bool {
        self.connection_supports(token, crate::machine_protocol::PLUGIN_STEP_PROTOCOL_VERSION)
    }

    pub(crate) fn leased_plugin_steps(&self, token: &ConnectionToken) -> bool {
        self.connection_supports(
            token,
            crate::machine_protocol::PLUGIN_EXECUTION_LEASE_PROTOCOL_VERSION,
        )
    }

    fn connection_supports(&self, token: &ConnectionToken, minimum: u16) -> bool {
        self.live
            .read()
            .connections
            .get(&token.0.machine_id)
            .is_some_and(|c| c.token.same(token) && c.protocol >= minimum)
    }

    pub(crate) async fn plugin_uninstall_step(
        &self,
        token: &ConnectionToken,
        step: &UninstallStep,
        query_only: bool,
    ) -> Result<StepObservation, CommandRequestError> {
        let fail = |certainty, detail: &str| CommandRequestError {
            certainty,
            detail: detail.to_owned(),
        };
        step.validate()
            .map_err(|_| fail(CommandFailure::NotSent, "invalid Machine step"))?;
        if step.machine_id != token.0.machine_id {
            return Err(fail(
                CommandFailure::NotSent,
                "Machine step target mismatch",
            ));
        }
        if !query_only && !self.leased_plugin_steps(token) {
            return Err(fail(
                CommandFailure::NotSent,
                "Machine update required for bounded Plugin execution",
            ));
        }
        let request_id = self.request_id("plugin-step").map_err(|_| {
            fail(
                CommandFailure::NotSent,
                "Machine request identity unavailable",
            )
        })?;
        let command = if query_only {
            MachineCommand::QueryPluginUninstallStep {
                request_id: request_id.clone(),
                step: Box::new(step.clone()),
            }
        } else {
            MachineCommand::UninstallPluginStep {
                request_id: request_id.clone(),
                step: Box::new(step.clone()),
            }
        };
        let (rx, _pending) = self
            .begin_request(
                &token.0.machine_id,
                &request_id,
                command,
                ReplyKind::PluginStep,
                Some(RequestBinding::Connection(token)),
            )
            .map_err(|_| fail(CommandFailure::NotSent, "Machine step channel unavailable"))?;
        match tokio::time::timeout(PROVIDER_COMMAND_TIMEOUT, rx).await {
            Ok(Ok(Reply::PluginStep(observation))) => {
                if let StepLookup::Found { receipt } = &observation.result
                    && !receipt.matches(step)
                {
                    return Err(fail(
                        CommandFailure::Unknown,
                        "Machine step receipt identity mismatch",
                    ));
                }
                Ok(*observation)
            }
            _ => Err(fail(
                CommandFailure::Unknown,
                "Machine step receipt unavailable",
            )),
        }
    }

    pub(crate) async fn plugin_uninstall_recovery(
        &self,
        token: &ConnectionToken,
        step: &UninstallStep,
    ) -> Result<RecoveryObservation, CommandRequestError> {
        let fail = |certainty, detail: &str| CommandRequestError {
            certainty,
            detail: detail.to_owned(),
        };
        step.validate()
            .map_err(|_| fail(CommandFailure::NotSent, "invalid recovery query"))?;
        if step.machine_id != token.0.machine_id {
            return Err(fail(
                CommandFailure::NotSent,
                "recovery query target mismatch",
            ));
        }
        let request_id = self.request_id("plugin-recovery").map_err(|_| {
            fail(
                CommandFailure::NotSent,
                "recovery query identity unavailable",
            )
        })?;
        let (rx, _pending) = self
            .begin_request(
                &token.0.machine_id,
                &request_id,
                MachineCommand::QueryPluginUninstallRecovery {
                    request_id: request_id.clone(),
                    step: Box::new(step.clone()),
                },
                ReplyKind::PluginRecovery,
                Some(RequestBinding::Connection(token)),
            )
            .map_err(|_| {
                fail(
                    CommandFailure::NotSent,
                    "recovery query channel unavailable",
                )
            })?;
        match tokio::time::timeout(PROVIDER_COMMAND_TIMEOUT, rx).await {
            Ok(Ok(Reply::PluginRecovery(observation))) if observation.matches(step) => {
                Ok(*observation)
            }
            _ => Err(fail(
                CommandFailure::Unknown,
                "recovery query evidence unavailable",
            )),
        }
    }

    // Reader bridge for the forthcoming durable Service coordinator. No live
    // mutation endpoint can manufacture an operation or invoke this as a grant.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn telemetry_binding_observation(
        &self,
        token: &ConnectionToken,
        step: &BindingStep,
    ) -> Result<BindingObservation, CommandRequestError> {
        let fail = |certainty, detail: &str| CommandRequestError {
            certainty,
            detail: detail.to_owned(),
        };
        step.validate()
            .map_err(|_| fail(CommandFailure::NotSent, "invalid telemetry binding query"))?;
        if step.machine_id != token.0.machine_id {
            return Err(fail(
                CommandFailure::NotSent,
                "telemetry binding query target mismatch",
            ));
        }
        let request_id = self.request_id("telemetry-binding").map_err(|_| {
            fail(
                CommandFailure::NotSent,
                "telemetry binding query identity unavailable",
            )
        })?;
        let (rx, _pending) = self
            .begin_request(
                &token.0.machine_id,
                &request_id,
                MachineCommand::QueryTelemetryBinding {
                    request_id: request_id.clone(),
                    step: Box::new(step.clone()),
                },
                ReplyKind::TelemetryBinding,
                Some(RequestBinding::Connection(token)),
            )
            .map_err(|_| {
                fail(
                    CommandFailure::NotSent,
                    "telemetry binding query channel unavailable",
                )
            })?;
        match tokio::time::timeout(std::time::Duration::from_secs(15), rx).await {
            Ok(Ok(Reply::TelemetryBinding(observation))) if observation.matches(step) => {
                Ok(*observation)
            }
            _ => Err(fail(
                CommandFailure::Unknown,
                "telemetry binding query evidence unavailable",
            )),
        }
    }

    pub(crate) fn telemetry_binding_target_current(
        &self,
        token: &ConnectionToken,
        step: &BindingStep,
    ) -> bool {
        let live = self.live.read();
        step.validate_commit().is_ok()
            && step.machine_id == token.0.machine_id
            && live.connections.get(&token.0.machine_id).is_some_and(|c| {
                c.token.same(token)
                    && c.protocol
                        >= crate::machine_protocol::TELEMETRY_BINDING_COMMIT_PROTOCOL_VERSION
            })
            && live.telemetry_target_matches(&token.0.machine_id, step)
    }

    pub(crate) async fn commit_telemetry_binding(
        &self,
        token: &ConnectionToken,
        step: &BindingStep,
    ) -> Result<BindingObservation, CommandRequestError> {
        let failure = |certainty, detail: &str| CommandRequestError {
            certainty,
            detail: detail.into(),
        };
        if step.validate_commit().is_err() || step.machine_id != token.0.machine_id {
            return Err(failure(
                CommandFailure::NotSent,
                "invalid telemetry binding mutation",
            ));
        }
        let request_id = self.request_id("telemetry-binding-commit").map_err(|_| {
            failure(
                CommandFailure::NotSent,
                "binding request identity unavailable",
            )
        })?;
        let (rx, _pending) = self
            .begin_request(
                &token.0.machine_id,
                &request_id,
                MachineCommand::CommitTelemetryBinding {
                    request_id: request_id.clone(),
                    step: Box::new(step.clone()),
                },
                ReplyKind::TelemetryBindingCommit,
                Some(RequestBinding::Telemetry(token, step)),
            )
            .map_err(|_| {
                failure(
                    CommandFailure::NotSent,
                    "binding mutation channel or target unavailable",
                )
            })?;
        match tokio::time::timeout(std::time::Duration::from_secs(45), rx).await {
            Ok(Ok(Reply::TelemetryBindingCommit(result))) => match *result {
                BindingCommitResult::Observed { observation } if observation.matches(step) => {
                    Ok(observation)
                }
                BindingCommitResult::Unavailable {
                    failure:
                        crate::machine_protocol::telemetry_binding::BindingCommitFailure::Unavailable(
                            crate::machine_protocol::telemetry_binding::BindingUnavailable::Storage,
                        ),
                } => Err(failure(
                    CommandFailure::Unknown,
                    "binding persistence outcome is uncertain",
                )),
                BindingCommitResult::Unavailable { .. } => Err(failure(
                    CommandFailure::Rejected,
                    "binding mutation returned a rejection",
                )),
                BindingCommitResult::Observed { .. } => Err(failure(
                    CommandFailure::Unknown,
                    "binding mutation evidence mismatch",
                )),
            },
            _ => Err(failure(
                CommandFailure::Unknown,
                "binding mutation receipt unavailable",
            )),
        }
    }

    pub(crate) async fn command_on_connection(
        &self,
        connection: &ConnectionToken,
        request_id: String,
        command: MachineCommand,
    ) -> Result<(), CommandRequestError> {
        self.command_receipt(
            &connection.0.machine_id,
            request_id,
            command,
            PROVIDER_COMMAND_TIMEOUT,
            Some(RequestBinding::Connection(connection)),
        )
        .await
    }

    /// Reject an observed replacement before enqueue. The older Machine
    /// protocol still has no durable installation-CAS; this is NOT that proof.
    pub(crate) async fn reactivate_plugin_on_connection(
        &self,
        connection: &ConnectionToken,
        request_id: String,
        target: RetainedPluginTarget<'_>,
    ) -> Result<(), CommandRequestError> {
        let command = MachineCommand::ReactivatePlugin {
            request_id: request_id.clone(),
            plugin_id: target.plugin_id.to_owned(),
            generation_digest: target.digest.to_owned(),
        };
        self.command_receipt(
            &connection.0.machine_id,
            request_id,
            command,
            PROVIDER_COMMAND_TIMEOUT,
            Some(RequestBinding::Reactivate(connection, target)),
        )
        .await
    }

    async fn command_receipt(
        &self,
        machine_id: &str,
        request_id: String,
        command: MachineCommand,
        timeout: std::time::Duration,
        binding: Option<RequestBinding<'_>>,
    ) -> Result<(), CommandRequestError> {
        let (rx, _pending) = self
            .begin_request(
                machine_id,
                &request_id,
                command,
                ReplyKind::Command,
                binding,
            )
            .map_err(|detail| CommandRequestError {
                certainty: CommandFailure::NotSent,
                detail,
            })?;
        let unknown = |detail: &str| CommandRequestError {
            certainty: CommandFailure::Unknown,
            detail: detail.to_owned(),
        };
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Reply::Command(result))) => result.map_err(|detail| CommandRequestError {
                certainty: CommandFailure::Rejected,
                detail,
            }),
            Ok(Ok(_)) => Err(unknown("Machine command reply kind mismatch")),
            Ok(Err(_)) => Err(unknown("Machine command response channel closed")),
            Err(_) => Err(unknown("Machine command timed out")),
        }
    }

    pub(crate) fn bind_plugin_host(
        &self,
        machine_id: &str,
        plugin: &PluginInventory,
        operation: PluginHostOperation,
    ) -> Result<PluginHostBinding, PluginHostRequestError> {
        let live = self.live.read();
        let connection = live
            .connections
            .get(machine_id)
            .filter(|_| live.plugin_matches(machine_id, plugin))
            .ok_or_else(|| PluginHostRequestError {
                started: false,
                detail: "Exact active Plugin binding is unavailable".to_owned(),
            })?;
        Ok(PluginHostBinding {
            connection: connection.token.clone(),
            inventory_revision: Arc::clone(&live.plugin_inventory[machine_id].revision),
            plugin: plugin.clone(),
            operation,
        })
    }

    pub async fn plugin_host_request(
        &self,
        machine_id: &str,
        plugin: &PluginInventory,
        operation: PluginHostOperation,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, PluginHostRequestError> {
        let binding = self.bind_plugin_host(machine_id, plugin, operation)?;
        self.invoke_plugin_host(binding, payload).await
    }

    pub(crate) async fn invoke_plugin_host(
        &self,
        binding: PluginHostBinding,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, PluginHostRequestError> {
        let preflight = |detail| PluginHostRequestError {
            started: false,
            detail,
        };
        let request_id = self.request_id("plugin-host").map_err(preflight)?;
        let plugin = &binding.plugin;
        let (rx, _pending) = self
            .begin_request(
                &binding.connection.0.machine_id,
                &request_id,
                MachineCommand::InvokePluginHost {
                    request_id: request_id.clone(),
                    plugin_id: plugin.plugin_id.clone(),
                    plugin_version: plugin.plugin_version.clone(),
                    generation_digest: plugin.generation_digest.clone(),
                    auth_generation: plugin.auth_generation,
                    operation: binding.operation,
                    payload,
                },
                ReplyKind::PluginHost,
                Some(RequestBinding::Plugin(&binding)),
            )
            .map_err(preflight)?;
        match tokio::time::timeout(PLUGIN_HOST_TIMEOUT, rx).await {
            Ok(Ok(Reply::PluginHost {
                accepted,
                started,
                payload,
                detail,
            })) => {
                if !accepted {
                    return Err(PluginHostRequestError {
                        started,
                        detail: detail
                            .unwrap_or_else(|| "Machine rejected Plugin host request".to_owned()),
                    });
                }
                payload
                    .filter(|value| !value.is_null())
                    .ok_or_else(|| PluginHostRequestError {
                        started: true,
                        detail: "Machine Plugin host response has no payload".to_owned(),
                    })
            }
            Ok(Ok(_)) => Err(PluginHostRequestError {
                started: true,
                detail: "Machine Plugin host reply kind mismatch".to_owned(),
            }),
            Ok(Err(_)) => Err(PluginHostRequestError {
                started: true,
                detail: "Machine Plugin host response channel closed".to_owned(),
            }),
            Err(_) => Err(PluginHostRequestError {
                started: true,
                detail: "Machine Plugin host request timed out".to_owned(),
            }),
        }
    }

    #[must_use]
    pub fn connected_machine_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self.live.read().connections.keys().cloned().collect();
        ids.sort();
        ids
    }

    #[must_use]
    pub(crate) fn connected_plugin_inventory(&self) -> Vec<ConnectedPluginInventory> {
        let live = self.live.read();
        let mut inventory = live
            .connections
            .iter()
            .filter(|(_, connection)| connection.protocol >= PLUGIN_HOST_EXECUTION_PROTOCOL_VERSION)
            .filter_map(|(machine_id, _)| {
                live.plugin_inventory
                    .get(machine_id)
                    .map(|inventory| (machine_id, &inventory.plugins))
            })
            .flat_map(|(machine_id, plugins)| {
                plugins.iter().map(|plugin| ConnectedPluginInventory {
                    machine_id: machine_id.clone(),
                    plugin: plugin.clone(),
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
        self.live
            .read()
            .events
            .get(machine_id)
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
#[path = "machine_control/telemetry_binding_tests.rs"]
mod telemetry_binding_tests;

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
            installation_revision: None,
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

    fn connect(
        control: &MachineControl,
        id: &str,
    ) -> (ConnectionToken, mpsc::UnboundedReceiver<MachineCommand>) {
        let (tx, rx) = mpsc::unbounded_channel();
        // Deliberately reuse the external epoch: local identity must still fence.
        (
            control.install(id.to_owned(), "same-epoch".to_owned(), false, 9, tx),
            rx,
        )
    }

    #[tokio::test]
    async fn installation_cas_rejects_protocol_ten_without_enqueuing() {
        let control = MachineControl::default();
        let (tx, mut commands) = mpsc::unbounded_channel();
        let connection = control.install("machine-test".into(), "epoch".into(), false, 10, tx);
        let mut step = crate::machine_protocol::plugin_step::fixture();
        step.schema = 2;
        step.installation_revision = Some(
            format!("installation-{}", "a".repeat(64))
                .try_into()
                .unwrap(),
        );
        let error = control
            .plugin_uninstall_step(&connection, &step, true)
            .await
            .unwrap_err();
        assert_eq!(error.certainty, CommandFailure::NotSent);
        assert!(commands.try_recv().is_err());
    }

    #[tokio::test]
    async fn recovery_observation_rejects_older_peer_and_wrong_machine_without_enqueue() {
        for (protocol, machine) in [(11, "machine-test"), (12, "other-machine")] {
            let control = MachineControl::default();
            let (tx, mut commands) = mpsc::unbounded_channel();
            let connection = control.install(machine.into(), "epoch".into(), false, protocol, tx);
            let step = crate::machine_protocol::plugin_step::fixture();
            assert_eq!(
                control
                    .plugin_uninstall_recovery(&connection, &step)
                    .await
                    .unwrap_err()
                    .certainty,
                CommandFailure::NotSent
            );
            assert!(commands.try_recv().is_err());
            assert!(control.live.read().pending.is_empty());
        }
    }

    #[tokio::test]
    async fn recovery_observation_validates_missing_receipt_identity_and_reply_kind() {
        use crate::machine_protocol::plugin_recovery::{InstallationEvidence, RecoverySnapshot};
        let control = MachineControl::default();
        let (tx, mut commands) = mpsc::unbounded_channel();
        let connection = control.install("machine-test".into(), "epoch".into(), false, 12, tx);
        let step = crate::machine_protocol::plugin_step::fixture();
        for changed in [false, true] {
            let request = control.plugin_uninstall_recovery(&connection, &step);
            let respond = async {
                let MachineCommand::QueryPluginUninstallRecovery {
                    request_id,
                    step: received,
                } = commands.recv().await.unwrap()
                else {
                    panic!("read-only command required")
                };
                assert_eq!(*received, step);
                control.record_remote(
                    &connection,
                    MachineEvent::PluginUninstallStep {
                        request_id: request_id.clone(),
                        observation: Box::new(StepObservation {
                            admission_enabled: true,
                            result: StepLookup::NotFound {},
                        }),
                    },
                );
                assert!(control.live.read().pending.contains_key(&request_id));
                control.record_remote(
                    &connection,
                    MachineEvent::PluginUninstallRecovery {
                        request_id,
                        observation: Box::new(RecoveryObservation::Observed {
                            snapshot: Box::new(RecoverySnapshot {
                                request_digest: if changed {
                                    crate::machine_protocol::plugin_step::digest(b"other request")
                                } else {
                                    step.request_digest().unwrap()
                                },
                                receipt: None,
                                installation: InstallationEvidence::Untracked {},
                                slot_fenced: false,
                            }),
                        }),
                    },
                );
            };
            let (result, ()) = tokio::join!(request, respond);
            if changed {
                assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown);
            } else {
                assert!(result.is_ok());
            }
            assert!(control.live.read().pending.is_empty());
        }
        assert!(
            control.events("machine-test").is_empty(),
            "recovery replies are never event history"
        );
    }

    #[tokio::test]
    async fn recovery_observation_drops_old_connection_and_cancelled_observers() {
        use crate::machine_protocol::plugin_step::StepUnavailable;
        let control = MachineControl::default();
        let (tx, mut commands) = mpsc::unbounded_channel();
        let old = control.install("machine-test".into(), "epoch".into(), false, 12, tx);
        let step = crate::machine_protocol::plugin_step::fixture();
        let request = control.plugin_uninstall_recovery(&old, &step);
        let replace = async {
            let MachineCommand::QueryPluginUninstallRecovery { request_id, .. } =
                commands.recv().await.unwrap()
            else {
                panic!()
            };
            let (tx, _rx) = mpsc::unbounded_channel();
            control.install("machine-test".into(), "epoch".into(), false, 12, tx);
            control.record_remote(
                &old,
                MachineEvent::PluginUninstallRecovery {
                    request_id,
                    observation: Box::new(RecoveryObservation::Unavailable {
                        reason: StepUnavailable::Storage,
                    }),
                },
            );
        };
        let (result, ()) = tokio::join!(request, replace);
        assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown);
        let (tx, mut commands) = mpsc::unbounded_channel();
        let current = control.install("machine-test".into(), "epoch".into(), false, 12, tx);
        let mut pending = Box::pin(control.plugin_uninstall_recovery(&current, &step));
        let request_id = tokio::select! {
            _ = &mut pending => panic!("no response has arrived"),
            command = commands.recv() => {
                let Some(MachineCommand::QueryPluginUninstallRecovery { request_id, .. }) = command else { panic!() };
                request_id
            }
        };
        assert!(control.live.read().pending.contains_key(&request_id));
        drop(pending);
        assert!(control.live.read().pending.is_empty());
        control.record_remote(
            &current,
            MachineEvent::PluginUninstallRecovery {
                request_id,
                observation: Box::new(RecoveryObservation::Unavailable {
                    reason: StepUnavailable::Storage,
                }),
            },
        );
        assert!(control.events("machine-test").is_empty());
    }

    #[tokio::test]
    async fn durable_step_rejects_old_protocol_before_enqueue() {
        let control = MachineControl::default();
        let (connection, mut commands) = connect(&control, "machine-test");
        let error = control
            .plugin_uninstall_step(
                &connection,
                &crate::machine_protocol::plugin_step::fixture(),
                true,
            )
            .await
            .unwrap_err();
        assert_eq!(error.certainty, CommandFailure::NotSent);
        assert!(commands.try_recv().is_err());
        assert!(control.live.read().pending.is_empty());
    }

    #[tokio::test]
    async fn leased_uninstall_requires_current_capable_connection_without_legacy_downgrade() {
        let control = MachineControl::default();
        let step = crate::machine_protocol::plugin_step::fixture();
        let mut tracked = step.clone();
        tracked.schema = 2;
        tracked.installation_revision = Some(
            format!("installation-{}", "a".repeat(64))
                .try_into()
                .unwrap(),
        );
        for protocol in 1..=13 {
            let (sender, mut commands) = mpsc::unbounded_channel();
            let connection = control.install(
                "machine-test".into(),
                "epoch".into(),
                false,
                protocol,
                sender,
            );
            let selected = control.plugin_uninstall_transport(&connection, &step);
            match protocol {
                5..=9 => assert_eq!(selected.unwrap(), PluginUninstallTransport::Legacy),
                13 => assert_eq!(selected.unwrap(), PluginUninstallTransport::Leased),
                _ => assert!(selected.is_err()),
            }
            assert_eq!(
                control
                    .plugin_uninstall_transport(&connection, &tracked)
                    .is_ok(),
                protocol == 13
            );
            let mut wrong = step.clone();
            wrong.machine_id = "other-machine".into();
            assert!(
                control
                    .plugin_uninstall_transport(&connection, &wrong)
                    .is_err()
            );
            let mut invalid = step.clone();
            invalid.schema = 3;
            assert!(
                control
                    .plugin_uninstall_transport(&connection, &invalid)
                    .is_err()
            );
            if protocol < 13 {
                let result = control
                    .plugin_uninstall_step(
                        &connection,
                        &crate::machine_protocol::plugin_step::fixture(),
                        false,
                    )
                    .await;
                assert_eq!(result.unwrap_err().certainty, CommandFailure::NotSent);
                assert!(commands.try_recv().is_err());
                assert!(control.live.read().pending.is_empty());
            }
            control.disconnect("machine-test");
            assert!(
                control
                    .plugin_uninstall_transport(&connection, &step)
                    .is_err()
            );
            assert!(!control.leased_plugin_steps(&connection));
            let (sender, _rx) = mpsc::unbounded_channel();
            let replacement =
                control.install("machine-test".into(), "epoch".into(), false, 13, sender);
            assert!(
                control
                    .plugin_uninstall_transport(&connection, &step)
                    .is_err()
            );
            assert_eq!(
                control
                    .plugin_uninstall_transport(&replacement, &step)
                    .unwrap(),
                PluginUninstallTransport::Leased
            );
        }
    }

    #[tokio::test]
    async fn durable_step_query_correlates_exact_evidence_and_drops_late_replies() {
        use crate::machine_protocol::plugin_step::{StepOutcome, StepReceipt, fixture};
        let control = MachineControl::default();
        let (sender, mut commands) = mpsc::unbounded_channel();
        let connection = control.install("machine-test".into(), "epoch".into(), false, 10, sender);
        let step = fixture();
        for changed in [false, true] {
            let request = control.plugin_uninstall_step(&connection, &step, true);
            let respond = async {
                let MachineCommand::QueryPluginUninstallStep {
                    request_id,
                    step: received,
                } = commands.recv().await.unwrap()
                else {
                    panic!("query must not mutate");
                };
                assert_eq!(*received, step);
                // Wrong reply union cannot complete the typed waiter.
                control.record_remote(
                    &connection,
                    MachineEvent::CommandResult {
                        request_id: request_id.clone(),
                        accepted: true,
                        detail: None,
                    },
                );
                assert!(control.live.read().pending.contains_key(&request_id));
                let mut receipt = StepReceipt {
                    step: step.clone(),
                    request_digest: step.request_digest().unwrap(),
                    outcome: StepOutcome::Applied {},
                };
                if changed {
                    receipt.step.plan_digest =
                        crate::machine_protocol::plugin_step::digest(b"other actor");
                }
                control.record_remote(
                    &connection,
                    MachineEvent::PluginUninstallStep {
                        request_id,
                        observation: Box::new(StepObservation {
                            admission_enabled: false,
                            result: StepLookup::Found {
                                receipt: Box::new(receipt),
                            },
                        }),
                    },
                );
            };
            let (result, ()) = tokio::join!(request, respond);
            if changed {
                assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown);
            } else {
                assert!(matches!(result.unwrap().result, StepLookup::Found { .. }));
            }
        }
        let (sender, mut commands) = mpsc::unbounded_channel();
        let connection = control.install("machine-test".into(), "epoch".into(), false, 13, sender);
        let request = control.plugin_uninstall_step(&connection, &step, false);
        let replace = async {
            let MachineCommand::UninstallPluginStep { request_id, .. } =
                commands.recv().await.unwrap()
            else {
                panic!();
            };
            let (sender, _commands) = mpsc::unbounded_channel();
            control.install("machine-test".into(), "epoch".into(), false, 13, sender);
            control.record_remote(
                &connection,
                MachineEvent::PluginUninstallStep {
                    request_id,
                    observation: Box::new(StepObservation {
                        admission_enabled: true,
                        result: StepLookup::NotFound {},
                    }),
                },
            );
        };
        let (result, ()) = tokio::join!(request, replace);
        assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown);
        assert!(control.live.read().pending.is_empty());
        assert!(
            !control
                .events("machine-test")
                .iter()
                .any(|e| matches!(e, MachineEvent::PluginUninstallStep { .. }))
        );
    }

    fn observe(
        control: &MachineControl,
        connection: &ConnectionToken,
        plugins: Vec<PluginInventory>,
    ) {
        control.record_remote(
            connection,
            MachineEvent::PluginInventory {
                plugins,
                observed_at_ms: 1,
            },
        );
    }

    fn refresh(id: &str) -> MachineCommand {
        MachineCommand::RefreshInventory {
            request_id: id.to_owned(),
        }
    }

    fn command_reply(id: &str) -> MachineEvent {
        MachineEvent::CommandResult {
            request_id: id.to_owned(),
            accepted: true,
            detail: None,
        }
    }

    #[tokio::test]
    async fn lifecycle_receipt_distinguishes_not_sent_rejected_and_unknown() {
        let control = MachineControl::default();
        let (old, mut rx) = connect(&control, "hawk");
        let request = control.command_on_connection(&old, "reject".to_owned(), refresh("reject"));
        let (result, ()) = tokio::join!(request, async {
            rx.recv().await.unwrap();
            control.record_remote(
                &old,
                MachineEvent::CommandResult {
                    request_id: "reject".to_owned(),
                    accepted: false,
                    detail: Some("known failure".to_owned()),
                },
            );
        });
        assert_eq!(result.unwrap_err().certainty, CommandFailure::Rejected);
        let request = control.command_on_connection(&old, "lost".to_owned(), refresh("lost"));
        let (result, ()) = tokio::join!(request, async {
            rx.recv().await.unwrap();
            control.disconnect("hawk");
        });
        assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown);
        let (_replacement, mut next_rx) = connect(&control, "hawk");
        let error = control
            .command_on_connection(&old, "restore".to_owned(), refresh("restore"))
            .await
            .unwrap_err();
        assert_eq!(error.certainty, CommandFailure::NotSent);
        assert!(
            next_rx.try_recv().is_err(),
            "a recovery command cannot migrate to the new connection"
        );
    }

    #[tokio::test]
    async fn lifecycle_timeout_is_unknown_and_does_not_leave_a_waiter() {
        let control = MachineControl::default();
        let (connection, _rx) = connect(&control, "hawk");
        let result = control
            .command_receipt(
                "hawk",
                "timeout".to_owned(),
                refresh("timeout"),
                std::time::Duration::from_millis(1),
                Some(RequestBinding::Connection(&connection)),
            )
            .await;
        assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown);
        assert!(control.live.read().pending.is_empty());
    }

    #[tokio::test]
    async fn compensation_rejects_a_replaced_or_ambiguous_installation_slot() {
        let control = MachineControl::default();
        let (connection, mut rx) = connect(&control, "hawk");
        let expected = plugin_inventory("codex");
        let target = RetainedPluginTarget {
            plugin_id: &expected.plugin_id,
            version: &expected.plugin_version,
            digest: &expected.generation_digest,
            fingerprint: &expected.contract_fingerprint,
        };
        assert!(
            control
                .reactivate_plugin_on_connection(&connection, "missing".to_owned(), target)
                .await
                .is_err()
        );
        let mut replaced = expected.clone();
        replaced.generation_digest = format!("sha256:{}", "f".repeat(64));
        for inventory in [vec![replaced], vec![expected.clone(), expected.clone()]] {
            observe(&control, &connection, inventory);
            let error = control
                .reactivate_plugin_on_connection(&connection, "conflict".to_owned(), target)
                .await
                .unwrap_err();
            assert_eq!(error.certainty, CommandFailure::NotSent);
            assert!(rx.try_recv().is_err());
        }
        for inventory in [vec![expected.clone()], Vec::new()] {
            observe(&control, &connection, inventory);
            let request =
                control.reactivate_plugin_on_connection(&connection, "allowed".to_owned(), target);
            let (result, ()) = tokio::join!(request, async {
                assert!(matches!(
                    rx.recv().await,
                    Some(MachineCommand::ReactivatePlugin { .. })
                ));
                control.record_remote(&connection, command_reply("allowed"));
            });
            result.unwrap();
        }
    }

    #[test]
    fn replacement_fences_late_replies_inventory_and_cleanup_even_with_reused_epoch() {
        let control = MachineControl::default();
        let (old, _old_rx) = connect(&control, "hawk");
        let (mut abandoned, old_guard) = control
            .begin_request(
                "hawk",
                "same-id",
                refresh("same-id"),
                ReplyKind::Command,
                None,
            )
            .unwrap();
        observe(&control, &old, vec![plugin_inventory("old")]);
        let (current, _rx) = connect(&control, "hawk");
        assert!(matches!(
            abandoned.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
        assert!(control.connected_plugin_inventory().is_empty());
        let (mut reply, _guard) = control
            .begin_request(
                "hawk",
                "same-id",
                refresh("same-id"),
                ReplyKind::Command,
                None,
            )
            .unwrap();
        drop(old_guard);
        control.remove_if_current(&old);
        observe(&control, &old, vec![plugin_inventory("old")]);
        control.record_remote(&old, command_reply("same-id"));
        assert!(control.is_current(&current));
        assert!(!control.is_current(&old));
        assert!(control.connected_plugin_inventory().is_empty());
        assert!(matches!(
            reply.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        control.record_remote(&current, command_reply("same-id"));
        assert!(matches!(reply.try_recv(), Ok(Reply::Command(Ok(())))));
        assert!(control.live.read().pending.is_empty());
    }

    #[test]
    fn duplicate_ids_and_wrong_reply_kinds_preserve_the_original_waiter() {
        let control = MachineControl::default();
        let (connection, mut outgoing) = connect(&control, "hawk");
        let (mut reply, _guard) = control
            .begin_request("hawk", "one", refresh("one"), ReplyKind::Command, None)
            .unwrap();
        outgoing.try_recv().unwrap();
        assert!(
            control
                .begin_request("hawk", "one", refresh("one"), ReplyKind::Command, None)
                .is_err()
        );
        assert!(outgoing.try_recv().is_err());
        for event in [
            MachineEvent::AdapterResponse {
                request_id: "one".to_owned(),
                accepted: true,
                payload: Some(serde_json::json!({})),
                detail: None,
            },
            MachineEvent::PluginHostResponse {
                request_id: "one".to_owned(),
                accepted: true,
                started: true,
                payload: Some(serde_json::json!({})),
                detail: None,
            },
        ] {
            control.record_remote(&connection, event);
            assert!(matches!(
                reply.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ));
        }
        control.record_remote(&connection, command_reply("one"));
        assert!(matches!(reply.try_recv(), Ok(Reply::Command(Ok(())))));
        assert!(
            control
                .events("hawk")
                .iter()
                .all(|event| matches!(event, MachineEvent::CommandResult { .. }))
        );
    }

    #[test]
    fn disconnect_releases_only_its_machine_waiters() {
        let control = MachineControl::default();
        let (_hawk, _hawk_rx) = connect(&control, "hawk");
        let (falcon, _falcon_rx) = connect(&control, "falcon");
        let (mut hawk_reply, _hawk_guard) = control
            .begin_request("hawk", "one", refresh("one"), ReplyKind::Command, None)
            .unwrap();
        let (mut falcon_reply, _falcon_guard) = control
            .begin_request("falcon", "two", refresh("two"), ReplyKind::Command, None)
            .unwrap();
        control.disconnect("hawk");
        assert!(matches!(
            hawk_reply.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
        assert!(matches!(
            falcon_reply.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        control.record_remote(&falcon, command_reply("two"));
        assert!(matches!(
            falcon_reply.try_recv(),
            Ok(Reply::Command(Ok(())))
        ));
        assert!(control.live.read().pending.is_empty());
    }

    #[test]
    fn send_failure_and_request_id_exhaustion_leave_no_waiter() {
        let control = MachineControl::default();
        let (_connection, rx) = connect(&control, "hawk");
        drop(rx);
        assert!(
            control
                .begin_request("hawk", "one", refresh("one"), ReplyKind::Command, None)
                .is_err()
        );
        assert!(control.live.read().pending.is_empty());
        assert_ne!(
            control.request_id("adapter").unwrap(),
            control.request_id("adapter").unwrap()
        );
        control.next_request.store(u64::MAX, Ordering::Relaxed);
        assert!(control.request_id("adapter").is_err());
        assert_eq!(control.next_request.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn pending_budgets_fail_before_enqueue_and_guards_release_every_entry() {
        let control = MachineControl::default();
        let ids = (0..MAX_PENDING)
            .map(|i| format!("request-{i}"))
            .collect::<Vec<_>>();
        let machines = (0..=MAX_PENDING / MAX_PENDING_PER_MACHINE)
            .map(|i| format!("machine-{i}"))
            .collect::<Vec<_>>();
        let mut outgoing = machines
            .iter()
            .map(|id| connect(&control, id).1)
            .collect::<Vec<_>>();
        let mut pending = Vec::new();
        for (index, id) in ids.iter().enumerate() {
            let machine = index / MAX_PENDING_PER_MACHINE;
            pending.push(
                control
                    .begin_request(
                        &machines[machine],
                        id,
                        refresh(id),
                        ReplyKind::Command,
                        None,
                    )
                    .unwrap(),
            );
            outgoing[machine].try_recv().unwrap();
            if index == MAX_PENDING_PER_MACHINE - 1 {
                assert!(
                    control
                        .begin_request(
                            &machines[0],
                            "excess",
                            refresh("excess"),
                            ReplyKind::Command,
                            None
                        )
                        .is_err()
                );
                assert!(outgoing[0].try_recv().is_err());
            }
        }
        assert!(
            control
                .begin_request(
                    machines.last().unwrap(),
                    "excess",
                    refresh("excess"),
                    ReplyKind::Command,
                    None
                )
                .is_err()
        );
        assert!(outgoing.last_mut().unwrap().try_recv().is_err());
        drop(pending);
        assert!(control.live.read().pending.is_empty());
    }

    #[test]
    fn plugin_binding_requires_one_exact_active_inventory_entry() {
        let control = MachineControl::default();
        let (connection, _rx) = connect(&control, "hawk");
        let expected = plugin_inventory("codex");
        let bind =
            || control.bind_plugin_host("hawk", &expected, PluginHostOperation::CollectUsage);
        assert!(!bind().unwrap_err().started);
        for field in 0..8 {
            let mut changed = expected.clone();
            match field {
                0 => changed.plugin_id = "other".to_owned(),
                1 => changed.plugin_version = "1.2.4".to_owned(),
                2 => changed.generation_digest = format!("sha256:{}", "ef".repeat(32)),
                3 => changed.contract_fingerprint = format!("sha256:{}", "ef".repeat(32)),
                4 => changed.auth_generation = Some(5),
                5 => changed.plugin_kind = cowboy_plugin_sdk::PluginKind::TelemetryBackend,
                6 => {
                    changed.installation_revision = Some(
                        format!("installation-{}", "a".repeat(64))
                            .try_into()
                            .unwrap(),
                    );
                }
                _ => changed.state = PluginInstallationState::Uninstalling,
            }
            observe(&control, &connection, vec![changed]);
            assert!(!bind().unwrap_err().started);
        }
        observe(
            &control,
            &connection,
            vec![expected.clone(), expected.clone()],
        );
        assert!(!bind().unwrap_err().started);
        observe(&control, &connection, vec![expected]);
        assert!(
            control
                .bind_plugin_host(
                    "hawk",
                    &plugin_inventory("codex"),
                    PluginHostOperation::CollectUsage
                )
                .is_ok()
        );
    }

    #[tokio::test]
    async fn binding_cannot_retarget_after_reconnect_or_observed_inventory_aba() {
        for reconnect in [false, true] {
            let control = MachineControl::default();
            let (connection, mut rx) = connect(&control, "hawk");
            let plugin = plugin_inventory("codex");
            observe(&control, &connection, vec![plugin.clone()]);
            let binding = control
                .bind_plugin_host("hawk", &plugin, PluginHostOperation::CollectUsage)
                .unwrap();
            if reconnect {
                let (replacement, replacement_rx) = connect(&control, "hawk");
                observe(&control, &replacement, vec![plugin]);
                rx = replacement_rx;
            } else {
                observe(&control, &connection, vec![]);
                observe(&control, &connection, vec![plugin]);
            }
            let error = control
                .invoke_plugin_host(binding, serde_json::json!({}))
                .await
                .unwrap_err();
            assert!(!error.started);
            assert!(rx.try_recv().is_err());
            assert!(control.live.read().pending.is_empty());
        }
    }

    #[tokio::test]
    async fn lease_observations_preserve_binding_but_a_lost_or_null_receipt_is_unknown() {
        for outcome in [0, 1, 2] {
            let control = MachineControl::default();
            let (connection, mut rx) = connect(&control, "hawk");
            let mut plugin = plugin_inventory("codex");
            observe(&control, &connection, vec![plugin.clone()]);
            let binding = control
                .bind_plugin_host("hawk", &plugin, PluginHostOperation::CollectUsage)
                .unwrap();
            plugin.active_session_leases = 10;
            observe(&control, &connection, vec![plugin]);
            let (result, ()) = tokio::join!(
                control.invoke_plugin_host(binding, serde_json::json!({})),
                async {
                    let MachineCommand::InvokePluginHost { request_id, .. } =
                        rx.recv().await.unwrap()
                    else {
                        panic!("wrong command");
                    };
                    if outcome == 0 {
                        control.disconnect("hawk");
                    } else {
                        control.record_remote(
                            &connection,
                            MachineEvent::PluginHostResponse {
                                request_id,
                                accepted: true,
                                started: true,
                                detail: None,
                                payload: (outcome == 1).then_some(serde_json::Value::Null),
                            },
                        );
                    }
                }
            );
            assert!(result.unwrap_err().started);
            assert!(control.live.read().pending.is_empty());
        }
    }

    #[tokio::test]
    async fn a_different_machine_cannot_complete_an_adapter_request() {
        let control = Arc::new(MachineControl::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let hawk = control.install("hawk".to_owned(), "hawk-epoch".to_owned(), false, 9, tx);
        let (other, _other_rx) = mpsc::unbounded_channel();
        let falcon = control.install(
            "falcon".to_owned(),
            "falcon-epoch".to_owned(),
            false,
            9,
            other,
        );
        let requester = Arc::clone(&control);
        let mut request = tokio::spawn(async move {
            requester
                .adapter_request("hawk", "zed", serde_json::json!({}))
                .await
        });
        let MachineCommand::AdapterRequest { request_id, .. } = rx.recv().await.unwrap() else {
            panic!("command");
        };
        let response = MachineEvent::AdapterResponse {
            request_id,
            accepted: true,
            payload: Some(serde_json::json!({"source":"fixture"})),
            detail: None,
        };
        control.record_remote(&falcon, response.clone());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut request)
                .await
                .is_err()
        );
        control.record_remote(&hawk, response);
        request.await.unwrap().unwrap();
    }

    #[test]
    fn provider_refresh_credentials_never_enter_machine_event_history() {
        let control = MachineControl::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let connection = control.install("hawk".to_owned(), "epoch".to_owned(), false, 9, tx);
        control.record_remote(
            &connection,
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

        assert!(
            control
                .events("hawk")
                .iter()
                .all(|event| matches!(event, MachineEvent::PluginInventory { .. }))
        );
    }

    #[tokio::test]
    async fn plugin_host_payload_is_correlated_without_entering_event_history() {
        let control = Arc::new(MachineControl::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let connection = control.install("hawk".to_owned(), "epoch".to_owned(), false, 7, tx);
        let plugin = plugin_inventory("codex");
        control.record_remote(
            &connection,
            MachineEvent::PluginInventory {
                plugins: vec![plugin.clone()],
                observed_at_ms: 1,
            },
        );
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
        control.record_remote(
            &connection,
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
        assert!(
            control
                .events("hawk")
                .iter()
                .all(|event| matches!(event, MachineEvent::PluginInventory { .. }))
        );
    }

    #[tokio::test]
    async fn plugin_host_preflight_rejection_preserves_not_started() {
        let control = Arc::new(MachineControl::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let connection = control.install("hawk".to_owned(), "epoch".to_owned(), false, 7, tx);
        let plugin = plugin_inventory("codex");
        control.record_remote(
            &connection,
            MachineEvent::PluginInventory {
                plugins: vec![plugin.clone()],
                observed_at_ms: 1,
            },
        );
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
        control.record_remote(
            &connection,
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
        assert!(
            control
                .events("hawk")
                .iter()
                .all(|event| matches!(event, MachineEvent::PluginInventory { .. }))
        );
    }

    #[test]
    fn exact_host_inventory_is_independent_of_bounded_history_and_old_protocols() {
        let control = MachineControl::default();
        let (old_tx, _) = mpsc::unbounded_channel();
        let old_connection =
            control.install("old".to_owned(), "old-epoch".to_owned(), false, 6, old_tx);
        control.record_remote(
            &old_connection,
            MachineEvent::PluginInventory {
                plugins: vec![plugin_inventory("codex")],
                observed_at_ms: 1,
            },
        );
        let (current_tx, _) = mpsc::unbounded_channel();
        let connection = control.install(
            "current".to_owned(),
            "current-epoch".to_owned(),
            false,
            7,
            current_tx,
        );
        control.record_remote(
            &connection,
            MachineEvent::PluginInventory {
                plugins: vec![plugin_inventory("codex")],
                observed_at_ms: 1,
            },
        );
        for index in 0..70 {
            control.record_remote(
                &connection,
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
        let connection = control.install("mac".to_owned(), "epoch".to_owned(), false, 3, tx);
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
        control.record_remote(
            &connection,
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
        let connection = control.install("mac".to_owned(), "epoch".to_owned(), false, 3, tx);
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
        control.record_remote(
            &connection,
            MachineEvent::CommandResult {
                request_id: "refresh-1".to_owned(),
                accepted: true,
                detail: None,
            },
        );

        request.await.expect("task").expect("response");
        assert!(control.live.read().pending.is_empty());
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
        assert!(control.live.read().pending.is_empty());
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

        assert!(control.live.read().pending.is_empty());
    }

    #[test]
    fn colocated_state_belongs_to_the_current_machine_connection() {
        let control = MachineControl::default();
        let (first, _) = mpsc::unbounded_channel();
        let first = control.install("hawk".to_owned(), "old".to_owned(), true, 3, first);
        assert_eq!(control.is_colocated("hawk"), Some(true));

        let (current, _) = mpsc::unbounded_channel();
        let current = control.install("hawk".to_owned(), "current".to_owned(), false, 3, current);
        control.remove_if_current(&first);
        assert_eq!(control.is_colocated("hawk"), Some(false));

        control.remove_if_current(&current);
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
