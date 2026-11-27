//! `MachineCommand` family routed by `handle_machine_command`: Plugin installation, uninstallation and inventory, and npm component updates.

use super::*;

pub(super) fn handle(
    command: MachineCommand,
    context: MachineCommandContext,
    execution: &PluginExecutionScope,
) {
    let MachineCommandContext {
        service_id,
        machine_id,
        plugin_operation_admission,
        events,
        components,
        providers,
        zed_adapter_socket,
        workspaces,
        runtime_commands,
        ..
    } = context;
    let query_only = matches!(&command, MachineCommand::QueryPluginUninstallStep { .. });
    match command {
        MachineCommand::QueryPluginUninstallRecovery { request_id, step } => {
            tokio::spawn(async move {
                let observation = providers
                    .uninstall_recovery_observation(&step, service_id.as_deref(), &machine_id)
                    .await;
                let _ = events.send(MachineEvent::PluginUninstallRecovery {
                    request_id,
                    observation: Box::new(observation),
                });
            });
        }
        MachineCommand::UninstallPluginStep { request_id, step }
        | MachineCommand::QueryPluginUninstallStep { request_id, step } => {
            // Capture the deadline on receipt, not after detached scheduling or
            // lifecycle-lock acquisition. Queries never mint execution leases.
            let lease = if query_only {
                None
            } else {
                match execution.uninstall(&step) {
                    Ok(lease) => Some(lease),
                    Err(reason) => {
                        let _ = events.send(MachineEvent::PluginUninstallStep {
                            request_id,
                            observation: Box::new(StepObservation {
                                admission_enabled: false,
                                result: StepLookup::Unavailable { reason },
                            }),
                        });
                        return;
                    }
                }
            };
            tokio::spawn(async move {
                let observation = providers
                    .uninstall_step(
                        &step,
                        service_id.as_deref(),
                        &machine_id,
                        plugin_operation_admission,
                        lease
                            .as_ref()
                            .map_or(UninstallAccess::Observe, UninstallAccess::Execute),
                    )
                    .await;
                let _ = events.send(MachineEvent::PluginUninstallStep {
                    request_id,
                    observation: Box::new(observation),
                });
                if !query_only {
                    // An inventory read failure must not advertise an empty
                    // installation as proof of a completed uninstall.
                    tokio::spawn(async move {
                        let _ = providers
                            .publish_inventory_with(|plugins| {
                                let _ = events.send(MachineEvent::PluginInventory {
                                    plugins,
                                    observed_at_ms: unix_ms(),
                                });
                            })
                            .await;
                    });
                }
            });
        }
        MachineCommand::RefreshInventory { request_id } => {
            tokio::spawn(refresh_inventory_task(
                request_id,
                components,
                zed_adapter_socket,
                workspaces,
                providers,
                events,
            ));
        }
        MachineCommand::ObservePluginInstallation { request_id, query } => {
            tokio::spawn(async move {
                let observation = providers
                    .installation_target(
                        &query,
                        service_id.as_deref(),
                        &machine_id,
                        installation::WRITER_ENABLED && plugin_operation_admission,
                    )
                    .await;
                let _ = events.send(MachineEvent::PluginInstallationTarget {
                    request_id,
                    observation: Box::new(observation),
                });
            });
        }
        MachineCommand::QueryPluginInstallStep { request_id, step } => {
            tokio::spawn(async move {
                let observation = providers
                    .query_installation_step(
                        &step,
                        service_id.as_deref(),
                        &machine_id,
                        installation::WRITER_ENABLED && plugin_operation_admission,
                    )
                    .await;
                let _ = events.send(MachineEvent::PluginInstallationStep {
                    request_id,
                    observation: Box::new(observation),
                });
            });
        }
        MachineCommand::InstallPluginStep {
            request_id,
            step,
            plugin,
        } => {
            installation::install(
                request_id,
                *step,
                *plugin,
                providers,
                execution,
                installation::WRITER_ENABLED && plugin_operation_admission,
                events,
            );
        }
        MachineCommand::InstallPlugin { request_id, plugin } => {
            tokio::spawn(async move {
                let result = providers.install(&plugin).await;
                let accepted = result.is_ok();
                let detail = result.as_ref().err().map(|error| format!("{error:#}"));
                let _ = providers
                    .publish_inventory_with(|plugins| {
                        let _ = events.send(MachineEvent::PluginInventory {
                            plugins,
                            observed_at_ms: unix_ms(),
                        });
                    })
                    .await;
                let _ = events.send(MachineEvent::CommandResult {
                    request_id,
                    accepted,
                    detail,
                });
            });
        }
        MachineCommand::UninstallPlugin {
            request_id,
            plugin_id,
            generation_digest,
        } => {
            tokio::spawn(async move {
                let result = providers.uninstall(&plugin_id, &generation_digest).await;
                let accepted = result.is_ok();
                let detail = result.err().map(|error| format!("{error:#}"));
                publish_plugin_inventory_if_ok(&providers, &events, "after uninstall").await;
                let _ = events.send(MachineEvent::CommandResult {
                    request_id,
                    accepted,
                    detail,
                });
            });
        }
        MachineCommand::ReactivatePlugin {
            request_id,
            plugin_id,
            generation_digest,
        } => {
            tokio::spawn(async move {
                let result = providers.reactivate(&plugin_id, &generation_digest).await;
                let accepted = result.is_ok();
                let detail = result.err().map(|error| format!("{error:#}"));
                publish_plugin_inventory_if_ok(&providers, &events, "after reactivation").await;
                let _ = events.send(MachineEvent::CommandResult {
                    request_id,
                    accepted,
                    detail,
                });
            });
        }
        MachineCommand::UpdateNpmComponent {
            request_id,
            component,
        } => {
            tokio::spawn(async move {
                let result = update_npm_component(&component).await;
                let inventory = collect_inventory(&components, zed_adapter_socket.as_deref()).await;
                let recovered =
                    result.is_err() && npm_update_is_confirmed_by_inventory(&component, &inventory);
                let accepted = result.is_ok() || recovered;
                if recovered && let Err(error) = &result {
                    tracing::warn!(
                        component = ?component,
                        error = %error,
                        "npm update reported an error but inventory confirms the latest release"
                    );
                }
                if accepted && let Some(provider) = provider_for_component(&component) {
                    let _ = runtime_commands.send(crate::runtime_wire::Frame::CoreCommand {
                        command: crate::runtime_wire::CoreCommand::RollProvider {
                            provider: provider.to_owned(),
                            account_id: None,
                        },
                    });
                }
                let _ = events.send(MachineEvent::Inventory {
                    components: inventory,
                    workspaces: None,
                    workspace_identities: None,
                    workspace_revision: None,
                    workspace_owner: None,
                    observed_at_ms: unix_ms(),
                });
                let _ = events.send(MachineEvent::CommandResult {
                    request_id,
                    accepted,
                    detail: match result {
                        Ok(()) => None,
                        Err(_) if recovered => Some(
                            "npm reported an error, but the installed component is up to date"
                                .to_owned(),
                        ),
                        Err(error) => Some(format!("{error:#}")),
                    },
                });
            });
        }
        other => tracing::error!(
            command = ?std::mem::discriminant(&other),
            "machine command routed to the wrong family (command_plugins)"
        ),
    }
}

async fn refresh_inventory_task(
    request_id: String,
    components: Arc<ComponentStore>,
    zed_adapter_socket: Option<PathBuf>,
    workspaces: Arc<WorkspaceConfig>,
    providers: Arc<MachinePluginStore>,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
) {
    let components = collect_inventory(&components, zed_adapter_socket.as_deref()).await;
    let workspace_result = workspaces.reload();
    let mut inventory_error = None;
    if workspace_result.is_ok() {
        let (snapshot, workspace_identities) = workspaces.advertisement();
        let _ = events.send(MachineEvent::Inventory {
            components,
            workspace_identities: Some(workspace_identities),
            workspaces: Some(snapshot.workspaces.clone()),
            workspace_revision: snapshot.revision.clone(),
            workspace_owner: Some(snapshot.owner),
            observed_at_ms: unix_ms(),
        });
        if let Err(error) = providers
            .publish_inventory_with(|plugins| {
                let _ = events.send(MachineEvent::PluginInventory {
                    plugins,
                    observed_at_ms: unix_ms(),
                });
            })
            .await
        {
            tracing::warn!(
                error = %format!("{error:#}"),
                "Plugin inventory refresh failed; preserving previous observed Plugins"
            );
            inventory_error = Some(error);
        }
    }
    let (accepted, detail) = match (workspace_result, inventory_error) {
        (Err(error), _) => (
            false,
            Some(format!("workspace configuration reload failed: {error:#}")),
        ),
        (Ok(_), Some(error)) => (
            false,
            Some(format!("Plugin inventory refresh failed: {error:#}")),
        ),
        (Ok(_), None) => (true, None),
    };
    let _ = events.send(MachineEvent::CommandResult {
        request_id,
        accepted,
        detail,
    });
}

async fn publish_plugin_inventory_if_ok(
    providers: &MachinePluginStore,
    events: &tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    context: &str,
) {
    if let Err(error) = providers
        .publish_inventory_with(|plugins| {
            let _ = events.send(MachineEvent::PluginInventory {
                plugins,
                observed_at_ms: unix_ms(),
            });
        })
        .await
    {
        tracing::warn!(
            error = %format!("{error:#}"),
            context,
            "Plugin inventory refresh failed; preserving previous observed Plugins"
        );
    }
}

pub(super) fn spawn_post_welcome_inventory_publish(
    providers: &Arc<MachinePluginStore>,
    events: &tokio::sync::mpsc::UnboundedSender<MachineEvent>,
) {
    let providers = Arc::clone(providers);
    let events = events.clone();
    tokio::spawn(async move {
        let _ = providers
            .publish_inventory_with(|plugins| {
                let _ = events.send(MachineEvent::PluginInventory {
                    plugins,
                    observed_at_ms: unix_ms(),
                });
            })
            .await;
    });
}
