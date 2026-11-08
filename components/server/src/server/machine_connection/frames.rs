//! Per-frame handling for one authenticated Machine connection: inventory
//! tracking, event dispatch, and off-loop managed calls and memory relays.

use std::ops::ControlFlow;

use super::*;
use crate::machine_control::ConnectionToken;
use crate::machine_protocol::{
    ComponentInventory, MachineEvent, MachineFrame, MachineWorkspace, PluginInventory,
    WorkspaceRootIdentity,
};

/// The latest component, workspace and plugin inventory this connection has
/// reported, seeded from its hello.
pub(super) struct MachineInventory {
    components: Vec<ComponentInventory>,
    workspaces: Vec<MachineWorkspace>,
    workspace_identities: Vec<WorkspaceRootIdentity>,
    workspace_revision: Option<String>,
    workspace_owner: Option<crate::machine_protocol::projects::Owner>,
    providers: Vec<PluginInventory>,
}

impl MachineInventory {
    pub(super) fn from_hello(hello: &crate::machine_protocol::MachineHello) -> Self {
        Self {
            components: hello.components.clone(),
            workspaces: hello.workspaces.clone(),
            workspace_identities: hello.workspace_identities.clone(),
            workspace_revision: hello.workspace_revision.clone(),
            workspace_owner: hello.workspace_owner,
            providers: hello.plugins.clone(),
        }
    }
}

/// What one connection's frame handlers share with its read loop.
pub(super) struct MachineLink<'a> {
    pub(super) state: &'a Arc<AppState>,
    pub(super) store: &'a Store,
    pub(super) hello: &'a crate::machine_protocol::MachineHello,
    pub(super) challenge_id: &'a str,
    pub(super) connection: &'a ConnectionToken,
    pub(super) machine_write_tx: &'a mpsc::UnboundedSender<Message>,
    pub(super) execution_calls: &'a Arc<tokio::sync::Semaphore>,
    pub(super) lanes: &'a Arc<crate::machine_lanes::LaneSet<Message>>,
    pub(super) runtime_write_tx: &'a mpsc::UnboundedSender<crate::runtime_wire::Frame>,
}

impl MachineLink<'_> {
    /// Handles one decoded frame. `Break` ends the connection.
    pub(super) async fn handle_frame(
        &self,
        inventory: &mut MachineInventory,
        declined_usage_batch: &mut Option<(String, u64)>,
        heartbeat_watchdog: &mut MachineHeartbeatWatchdog,
        frame: MachineFrame,
    ) -> ControlFlow<()> {
        let Self {
            state,
            store,
            hello,
            challenge_id,
            connection,
            machine_write_tx,
            ..
        } = self;
        let result = match frame {
            MachineFrame::Heartbeat { .. } => {
                heartbeat_watchdog.observe_at(std::time::Instant::now());
                store
                    .machine_seen(&hello.machine_id, challenge_id, None)
                    .await
            }
            MachineFrame::Event {
                event:
                    MachineEvent::Inventory {
                        components,
                        workspaces,
                        workspace_identities,
                        workspace_revision,
                        workspace_owner,
                        ..
                    },
            } => {
                self.record_inventory(
                    inventory,
                    components,
                    workspaces,
                    workspace_identities,
                    workspace_revision,
                    workspace_owner,
                )
                .await
            }
            MachineFrame::Event {
                event:
                    MachineEvent::PluginInventory {
                        plugins,
                        observed_at_ms,
                    },
            } => {
                self.record_plugin_inventory(inventory, plugins, observed_at_ms)
                    .await
            }
            MachineFrame::Event {
                event:
                    MachineEvent::ProviderUsageBatch {
                        producer_id,
                        first_sequence,
                        last_sequence,
                        events,
                    },
            } => {
                ingest_machine_usage_batch(
                    state,
                    store,
                    hello,
                    challenge_id,
                    machine_write_tx,
                    &inventory.providers,
                    declined_usage_batch,
                    producer_id,
                    first_sequence,
                    last_sequence,
                    events,
                )
                .await
            }
            MachineFrame::Event {
                event: MachineEvent::NativeProfileCompleted { profile },
            } => {
                let completion_state = Arc::clone(state);
                let completion_connection = (*connection).clone();
                tokio::spawn(async move {
                    if let Err(code) = provider_accounts::login::accept_native_completion(
                        &completion_state,
                        &completion_connection,
                        &profile,
                    )
                    .await
                    {
                        // Do not log account identity or let rejected events
                        // overwrite another request's legitimate status.
                        tracing::warn!(%code, "native Provider completion was not recorded");
                    }
                });
                store
                    .machine_seen(&hello.machine_id, challenge_id, None)
                    .await
            }
            MachineFrame::Event {
                event:
                    MachineEvent::ServiceAuthCandidate {
                        request_id,
                        provider_id,
                        account_id,
                        native_identity,
                        auth_method,
                        provider_version,
                        generation_digest,
                        auth_contract_fingerprint,
                        portable_schema,
                        bundle,
                        account_label,
                    },
            } => {
                accept_machine_service_auth_candidate(
                    state,
                    store,
                    hello,
                    challenge_id,
                    request_id,
                    provider_id,
                    account_id,
                    native_identity,
                    auth_method,
                    provider_version,
                    generation_digest,
                    auth_contract_fingerprint,
                    portable_schema,
                    bundle,
                    account_label,
                )
                .await
            }
            MachineFrame::Event {
                event:
                    MachineEvent::ProviderAuthRefreshCandidate {
                        native_identity,
                        request_id,
                        provider_id,
                        account_id,
                        expected_generation,
                        provider_version,
                        generation_digest,
                        auth_contract_fingerprint,
                        portable_schema,
                        auth_method,
                        bundle,
                    },
            } => {
                accept_machine_auth_refresh_candidate(
                    state,
                    store,
                    hello,
                    challenge_id,
                    &inventory.providers,
                    native_identity,
                    request_id,
                    provider_id,
                    account_id,
                    expected_generation,
                    provider_version,
                    generation_digest,
                    auth_contract_fingerprint,
                    portable_schema,
                    auth_method,
                    bundle,
                )
                .await
            }
            MachineFrame::Event {
                event:
                    MachineEvent::ManagedCall {
                        request_id,
                        grant,
                        action,
                    },
            } => {
                self.spawn_managed_call(request_id, grant, *action);
                return ControlFlow::Continue(());
            }
            MachineFrame::Event {
                event:
                    MachineEvent::MemoryRelayRequest {
                        request_id,
                        session_id,
                        request,
                    },
            } => {
                self.spawn_memory_relay(request_id, session_id, request);
                return ControlFlow::Continue(());
            }
            MachineFrame::Event {
                event:
                    MachineEvent::MemoryRelayResponse {
                        request_id,
                        response,
                    },
            } => {
                state
                    .memory_relay()
                    .complete(&hello.machine_id, &request_id, response);
                return ControlFlow::Continue(());
            }
            MachineFrame::Event { event } => {
                self.fail_signed_out_login(&event);
                state.machine_control.record_remote(connection, event);
                store
                    .machine_seen(&hello.machine_id, challenge_id, None)
                    .await
            }
            MachineFrame::Runtime { frame } => return self.route_runtime_frame(frame),
            _ => return ControlFlow::Continue(()),
        };
        if let Err(error) = result {
            tracing::warn!(%error, machine = %hello.machine_id, "updating Machine state");
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }

    async fn record_inventory(
        &self,
        inventory: &mut MachineInventory,
        components: Vec<ComponentInventory>,
        workspaces: Option<Vec<MachineWorkspace>>,
        workspace_identities: Option<Vec<WorkspaceRootIdentity>>,
        workspace_revision: Option<String>,
        workspace_owner: Option<crate::machine_protocol::projects::Owner>,
    ) -> anyhow::Result<()> {
        let Self {
            state,
            store,
            hello,
            challenge_id,
            connection,
            ..
        } = self;
        inventory.components = components;
        if workspaces.is_some() {
            inventory.workspace_owner = workspace_owner;
        }
        apply_workspace_inventory(
            &mut inventory.workspaces,
            &mut inventory.workspace_identities,
            &mut inventory.workspace_revision,
            workspaces,
            workspace_identities,
            workspace_revision,
        );
        state.machine_control.record_remote(
            connection,
            MachineEvent::Inventory {
                components: inventory.components.clone(),
                workspaces: Some(inventory.workspaces.clone()),
                workspace_identities: Some(inventory.workspace_identities.clone()),
                workspace_revision: inventory.workspace_revision.clone(),
                workspace_owner: inventory.workspace_owner,
                observed_at_ms: now_ms(),
            },
        );
        let seen = serde_json::json!({
            "components": &inventory.components,
            "providers": &inventory.providers,
            "provider_contracts": &hello.provider_contracts,
            "plugin_contracts": &hello.plugin_contracts,
            "workspaces": &inventory.workspaces,
            "workspace_revision": &inventory.workspace_revision,
            "workspace_owner": &inventory.workspace_owner,
            "capacity": &hello.capacity,
        });
        let result = store
            .machine_seen(&hello.machine_id, challenge_id, Some(&seen))
            .await;
        if result.is_ok() {
            state.machine_snapshots.publish().await;
        }
        result
    }

    async fn record_plugin_inventory(
        &self,
        inventory: &mut MachineInventory,
        plugins: Vec<PluginInventory>,
        observed_at_ms: i64,
    ) -> anyhow::Result<()> {
        let Self {
            state,
            store,
            hello,
            challenge_id,
            connection,
            ..
        } = self;
        inventory.providers = plugins;
        let failed_provider_auth = failed_provider_auth_projection_ids(&inventory.providers);
        state.machine_control.record_remote(
            connection,
            MachineEvent::PluginInventory {
                plugins: inventory.providers.clone(),
                observed_at_ms,
            },
        );
        let seen = serde_json::json!({
            "components": &inventory.components,
            "plugins": &inventory.providers,
            "provider_contracts": &hello.provider_contracts,
            "plugin_contracts": &hello.plugin_contracts,
            "workspaces": &inventory.workspaces,
            "workspace_revision": &inventory.workspace_revision,
            "workspace_owner": &inventory.workspace_owner,
            "capacity": &hello.capacity,
        });
        let result = store
            .machine_seen(&hello.machine_id, challenge_id, Some(&seen))
            .await;
        if result.is_ok() {
            state.machine_snapshots.publish().await;
            for provider_id in failed_provider_auth {
                let repair_state = Arc::clone(state);
                let repair_machine_id = hello.machine_id.clone();
                tokio::spawn(async move {
                    if let Err(error) = sync_provider_auth_to_machine(
                        &repair_state,
                        &repair_machine_id,
                        &provider_id,
                    )
                    .await
                    {
                        tracing::warn!(
                            %error,
                            machine = %repair_machine_id,
                            %provider_id,
                            "repairing failed Provider credential projection"
                        );
                    }
                });
            }
        }
        result
    }

    /// Handled off the read loop: the reply travels on this same connection
    /// and the action may wait for a bounded interval.
    fn spawn_managed_call(
        &self,
        request_id: String,
        grant: crate::managed_calls::protocol::Grant,
        action: crate::managed_calls::protocol::Action,
    ) {
        if let crate::managed_calls::protocol::Action::Mcp { request } = &action
            && matches!(
                request.action,
                crate::mcp_binding::RelayAction::Close { .. }
            )
        {
            let response = super::super::mcp_bindings::relay::close(
                self.state,
                self.connection,
                &grant,
                request,
            );
            self.reply_managed_call(request_id, response);
            return;
        }
        let call_state = Arc::clone(self.state);
        let call_connection = self.connection.clone();
        let outgoing = self.machine_write_tx.clone();
        let permit = Arc::clone(self.execution_calls).try_acquire_owned();
        tokio::spawn(async move {
            let response = if let Ok(_permit) = permit {
                managed_calls::handle(&call_state, &call_connection, grant, action).await
            } else {
                serde_json::json!({"schema":1,"state":"error",
                    "error":{"code":"capacity","admission":"not_submitted"}})
            };
            if call_state.machine_control.is_current(&call_connection) {
                let _ = queue_machine_json(
                    &outgoing,
                    &MachineFrame::Command {
                        command: crate::machine_protocol::MachineCommand::ManagedCallReply {
                            request_id,
                            response,
                        },
                    },
                );
            }
        });
    }

    fn reply_managed_call(&self, request_id: String, response: serde_json::Value) {
        if self.state.machine_control.is_current(self.connection) {
            let _ = queue_machine_json(
                self.machine_write_tx,
                &MachineFrame::Command {
                    command: crate::machine_protocol::MachineCommand::ManagedCallReply {
                        request_id,
                        response,
                    },
                },
            );
        }
    }

    /// Off the read loop, like a managed call: a slow home Machine must not
    /// hold this connection. An exhausted budget is an ordinary unavailable
    /// answer.
    fn spawn_memory_relay(
        &self,
        request_id: String,
        session_id: String,
        request: crate::machine_protocol::memory_relay::Request,
    ) {
        let relay_state = Arc::clone(self.state);
        let relay_connection = self.connection.clone();
        let relay_machine = self.hello.machine_id.clone();
        let outgoing = self.machine_write_tx.clone();
        let permit = self.state.extensions.memory.try_admit();
        tokio::spawn(async move {
            let response = if let Some(_permit) = permit {
                memory_relay::handle(&relay_state, &relay_machine, &session_id, request).await
            } else {
                crate::machine_protocol::memory_relay::Response::unavailable()
            };
            if relay_state.machine_control.is_current(&relay_connection) {
                let _ = queue_machine_json(
                    &outgoing,
                    &MachineFrame::Command {
                        command: crate::machine_protocol::MachineCommand::MemoryRelayReply {
                            request_id,
                            response,
                        },
                    },
                );
            }
        });
    }

    /// A terminal login state from this Machine fails the matching pending
    /// Provider authentication it was executing.
    fn fail_signed_out_login(&self, event: &MachineEvent) {
        let state = self.state;
        if let MachineEvent::LoginState {
            request_id,
            provider,
            state: auth_state,
            ..
        } = event
            && matches!(
                auth_state,
                crate::machine_protocol::AuthState::SignedOut
                    | crate::machine_protocol::AuthState::Unsupported
                    | crate::machine_protocol::AuthState::Error
            )
        {
            let executors = state.provider_auth_executors.lock();
            if let Some(executor) = executors.get(request_id).filter(|executor| {
                executor.machine_id == self.hello.machine_id
                    && executor.provider_id == *provider
                    && !executor.promotion_started
                    && executor.active(&state.machine_control, now_ms())
            }) && let Some(scope) = executor.auth_scope(state)
            {
                scope.fail_authentication(provider, request_id);
            }
        }
    }

    fn route_runtime_frame(&self, frame: crate::runtime_wire::Frame) -> ControlFlow<()> {
        if let crate::runtime_wire::Frame::ExecutionRequest { request } = frame {
            machine_lanes::spawn_execution_request(
                self.state,
                self.connection,
                &self.hello.machine_id,
                self.execution_calls,
                request,
                machine_lanes::ReplyRoute {
                    primary: self.machine_write_tx.clone(),
                    lanes: Arc::clone(self.lanes),
                },
            );
            return ControlFlow::Continue(());
        }
        if let Err(error) = self.runtime_write_tx.send(frame) {
            tracing::warn!(%error, machine = %self.hello.machine_id, "Machine runtime writer closed");
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }
}
