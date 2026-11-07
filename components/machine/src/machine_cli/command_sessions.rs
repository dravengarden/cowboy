//! `MachineCommand` family routed by `handle_machine_command`: Session execution, code buffers, project listing, worker exits, adapter and Plugin host requests, and reconciliation.

use super::*;

pub(super) fn handle(
    command: MachineCommand,
    context: MachineCommandContext,
    execution: &PluginExecutionScope,
) {
    let MachineCommandContext {
        protocol,
        execution_manager,
        service_id,
        machine_id,
        events,
        components,
        providers,
        zed_adapter_socket,
        code_adapter_socket,
        worktree_roots,
        workspaces,
        durable_state,
        ..
    } = context;
    match command {
        MachineCommand::Execution {
            request_id,
            request,
        } => {
            let workspaces = workspaces.snapshot().workspaces;
            tokio::spawn(async move {
                let response = execution_manager.request(*request, &workspaces).await;
                let _ = events.send(MachineEvent::ExecutionResponse {
                    request_id,
                    response: Box::new(response),
                });
            });
        }
        MachineCommand::CodeBufferNavigation {
            request_id,
            request,
        } => {
            let invocation = execution.code_navigation(*request);
            tokio::spawn(async move {
                let result = async {
                    Ok::<_, anyhow::Error>(serde_json::to_value(
                        providers.navigate_code_buffer(invocation?).await?,
                    )?)
                }
                .await;
                let _ = events.send(MachineEvent::AdapterResponse {
                    request_id,
                    accepted: result.is_ok(),
                    payload: result.ok(),
                    detail: None,
                    refusal: None,
                });
            });
        }
        MachineCommand::CodeBufferSync {
            request_id,
            request,
        } => {
            // Capture the actual connection before spawning or waiting. Wire
            // data and a read lease alone cannot construct an invocation.
            let invocation = execution.code_buffer_sync(*request);
            tokio::spawn(async move {
                let result = async {
                    Ok::<_, anyhow::Error>(serde_json::to_value(
                        providers.synchronize_code_buffer(invocation?).await?,
                    )?)
                }
                .await;
                let _ = events.send(MachineEvent::AdapterResponse {
                    request_id,
                    accepted: result.is_ok(),
                    payload: result.ok(),
                    detail: None,
                    refusal: None,
                });
            });
        }
        MachineCommand::Projects {
            request_id,
            service_id: requested_service,
            machine_id: requested_machine,
            request,
        } => {
            if service_id.as_deref() != Some(requested_service.as_str())
                || machine_id != requested_machine
            {
                let _ = events.send(MachineEvent::AdapterResponse {
                    request_id,
                    accepted: false,
                    payload: None,
                    detail: Some("Project request site mismatch".into()),
                    refusal: None,
                });
                return;
            }
            tokio::spawn(async move {
                let mutation = matches!(
                    request,
                    crate::machine_protocol::projects::Request::Upsert { .. }
                        | crate::machine_protocol::projects::Request::Remove { .. }
                        | crate::machine_protocol::projects::Request::Adopt { .. }
                );
                let registry = Arc::clone(&workspaces);
                let result = tokio::task::spawn_blocking(move || registry.project_request(request))
                    .await
                    .map_err(anyhow::Error::from)
                    .and_then(std::convert::identity);
                if mutation && result.is_ok() {
                    let components =
                        collect_inventory(&components, zed_adapter_socket.as_deref()).await;
                    let (snapshot, identities) = workspaces.advertisement();
                    let _ = events.send(MachineEvent::Inventory {
                        components,
                        workspace_identities: Some(identities),
                        workspaces: Some(snapshot.workspaces),
                        workspace_revision: snapshot.revision,
                        workspace_owner: Some(snapshot.owner),
                        observed_at_ms: unix_ms(),
                    });
                }
                let _ = events.send(MachineEvent::AdapterResponse {
                    request_id,
                    accepted: result.is_ok(),
                    payload: result.as_ref().ok().cloned(),
                    detail: result.err().map(|error| format!("{error:#}")),
                    refusal: None,
                });
            });
        }
        MachineCommand::QueryWorkerExit { request_id, worker } => {
            query_worker_exit(protocol, &providers, &events, request_id, &worker);
        }
        MachineCommand::AdapterRequest {
            request_id,
            adapter,
            payload,
            workspace_incarnation,
        } => {
            #[cfg(target_os = "linux")]
            let observer_grant = (adapter == crate::machine_protocol::code_observer::ADAPTER)
                .then(|| observation::capture(&payload, execution_manager.calls()))
                .flatten();
            #[cfg(not(target_os = "linux"))]
            let observer_grant = None;
            tokio::spawn(run_adapter_request(
                request_id,
                adapter,
                payload,
                AdapterRequestContext {
                    observer_grant,
                    providers,
                    zed_adapter_socket,
                    code_adapter_socket,
                    worktree_roots,
                    workspaces: workspaces.snapshot().workspaces,
                    // The live registry, not the advertised snapshot: the root
                    // may have been replaced since the last advertisement.
                    root_identities: workspaces.identities(),
                    session_identities: Arc::clone(&workspaces.session_identities),
                    session_workspaces: Arc::clone(&workspaces),
                    workspace_incarnation,
                    events,
                    durable_state,
                },
            ));
        }
        MachineCommand::InvokePluginHost {
            request_id,
            plugin_id,
            plugin_version,
            generation_digest,
            auth_generation,
            operation,
            payload,
        } => {
            let invocation = execution.host(crate::machine_plugins::PluginHostRequest {
                plugin_id,
                plugin_version,
                generation_digest,
                auth_generation,
                operation,
                payload,
            });
            tokio::spawn(async move {
                let result = providers.invoke_host(invocation).await;
                let event = match result {
                    Ok(payload) => MachineEvent::PluginHostResponse {
                        request_id,
                        accepted: true,
                        started: true,
                        payload: Some(payload),
                        detail: None,
                    },
                    Err(failure) => MachineEvent::PluginHostResponse {
                        request_id,
                        accepted: false,
                        started: failure.started,
                        payload: None,
                        detail: Some(format!("{:#}", failure.error)),
                    },
                };
                let _ = events.send(event);
            });
        }
        MachineCommand::Reconcile {
            request_id,
            components: desired,
        } => {
            tokio::spawn(async move {
                let active = components.active().unwrap_or_default();
                let restart_host = desired.iter().any(|component| {
                    component_requires_host_restart(&component.id.kind)
                        && !active.iter().any(|(current, _)| {
                            current.id == component.id
                                && current.digest.eq_ignore_ascii_case(&component.digest)
                        })
                });
                let result = reconcile_components(request_id, desired, components).await;
                let restart_host = accepted_component_restart(restart_host, &result);
                for event in result {
                    let _ = events.send(event);
                }
                if restart_host {
                    // The service manager owns host replacement. Payload
                    // processes and state survive; exit only after the result
                    // has had a chance to reach the controller.
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    std::process::exit(75);
                }
            });
        }
        other => tracing::error!(
            command = ?std::mem::discriminant(&other),
            "machine command routed to the wrong family (command_sessions)"
        ),
    }
}

fn query_worker_exit(
    protocol: u16,
    providers: &MachinePluginStore,
    events: &tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    request_id: String,
    worker: &crate::provider_account::WorkerExitRequest,
) {
    let observation = (protocol >= crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION)
        .then(|| providers.observe_account_worker_exit(worker).ok().flatten())
        .flatten();
    let _ = events.send(MachineEvent::WorkerExitObserved {
        request_id,
        observation,
    });
}
