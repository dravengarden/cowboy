//! Creation is a durable, non-runnable intent followed by one target-owned
//! preparation and a storage compare-and-set. Recovery reuses the same identity.

use super::super::{
    AppState, AuthenticatedProductRequest, product_session_owner, resolve_machine_workspace,
    resolve_provider_generation,
};
use crate::core::Status;
use crate::execution_environment::{ExecutionBinding, PreparationV1};
use crate::machine_protocol::execution::{Action, ExecutorInventory, Request, Response};
use crate::machine_protocol::{MachineCapacity, MachineWorkspace, PluginInventory};
use axum::{
    Json,
    extract::{Extension, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

async fn call(state: &AppState, machine_id: &str, action: Action) -> Result<Response, String> {
    state
        .machine_control
        .execution_request(
            machine_id,
            Request {
                service_id: state.service_id.clone(),
                machine_id: machine_id.to_owned(),
                action,
            },
        )
        .await
}

pub(in crate::server) async fn executor(
    state: &AppState,
    machine: &str,
) -> Result<ExecutorInventory, String> {
    match call(state, machine, Action::Inventory).await? {
        Response::Inventory {
            executor: Some(executor),
        } => Ok(executor),
        _ => Err("The selected environment has no compatible executor".into()),
    }
}

pub(in crate::server) fn accepts(
    behavior: &cowboy_provider_sdk::ProviderBehaviorContract,
    executor: &ExecutorInventory,
) -> bool {
    behavior
        .execution
        .as_ref()
        .is_some_and(|contract| contract.accepts(executor.protocol, &executor.digest))
}

pub(in crate::server) fn providers(machine: &crate::store::MachineRecord) -> Vec<PluginInventory> {
    machine
        .inventory
        .get("plugins")
        .or_else(|| machine.inventory.get("providers"))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

#[derive(Default, Deserialize)]
pub(in crate::server) struct AvailabilityQuery {
    runtime_machine_id: Option<String>,
    machine_id: Option<String>,
}

pub(in crate::server) async fn availability(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AvailabilityQuery>,
) -> axum::response::Response {
    let policy = state.project_placement.snapshot();
    let default = policy.default_runtime_machine_id.as_deref();
    let runtime = query
        .runtime_machine_id
        .as_deref()
        .or(default)
        .unwrap_or("");
    let Some(target) = query.machine_id.as_deref() else {
        return Json(json!({"enabled":true,"default_runtime_machine_id":default})).into_response();
    };
    let machines = match &state.store {
        Some(store) => store.list_machines().await.unwrap_or_default(),
        None => Vec::new(),
    };
    let machine = machines
        .iter()
        .find(|machine| machine.id == runtime && !machine.revoked);
    let target_registered = machines
        .iter()
        .any(|machine| machine.id == target && !machine.revoked);
    let mut compatible = Vec::new();
    if policy.allows(runtime, target)
        && target_registered
        && state.runtime_router.connected(runtime)
        && let Some(machine) = machine
        && let Ok(executor) = executor(&state, target).await
    {
        let inventory = providers(machine);
        for installed in &inventory {
            let authentication = state.provider_auth.status(&installed.plugin_id);
            if let Ok(generation) = resolve_provider_generation(
                &state.provider_catalog,
                &state.plugin_catalog,
                &inventory,
                &installed.plugin_id,
                authentication.as_ref(),
            ) && accepts(&generation.behavior, &executor)
            {
                compatible.push(installed.plugin_id.clone());
            }
        }
    }
    Json(json!({"enabled":true, "default_runtime_machine_id":default,
        "runtime_machine_id":runtime, "machine_id":target, "providers":compatible}))
    .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct CreateRequest {
    provider: String,
    runtime_machine_id: String,
    machine_id: String,
    cwd: String,
    #[serde(default)]
    origin: crate::core::SessionOrigin,
    #[serde(default)]
    initial_prompt: Option<String>,
}

pub(in crate::server) async fn create(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Json(request): Json<CreateRequest>,
) -> axum::response::Response {
    if !authenticated
        .principal
        .role
        .at_least(crate::admin::AdminRole::Operator)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    match create_checked(&state, &authenticated, request).await {
        Ok(meta) => (StatusCode::CREATED, Json(json!({
            "session_id":meta.id, "provider_version":meta.provider_version,
            "provider_generation_digest":meta.provider_generation_digest,
            "provider_auth_generation":meta.provider_auth_generation,
            "machine_id":meta.machine_id,"cwd":meta.cwd,"execution_binding":meta.execution_binding,
        }))).into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

async fn create_checked(
    state: &Arc<AppState>,
    authenticated: &AuthenticatedProductRequest,
    request: CreateRequest,
) -> Result<crate::core::SessionMeta, String> {
    if !state
        .project_placement
        .snapshot()
        .allows(&request.runtime_machine_id, &request.machine_id)
    {
        return Err("Machine policy does not permit this runtime and project placement".into());
    }
    if request.runtime_machine_id == request.machine_id
        || request.runtime_machine_id == "local"
        || request.machine_id == "local"
    {
        return Err("Select distinct enrolled runtime and execution Machines".into());
    }
    let store = state.store.as_ref().ok_or("Machine registry unavailable")?;
    let machines = store
        .list_machines()
        .await
        .map_err(|_| "Machine registry unavailable")?;
    let runtime = machines
        .iter()
        .find(|machine| machine.id == request.runtime_machine_id && !machine.revoked)
        .ok_or("Unknown runtime Machine")?;
    let target = machines
        .iter()
        .find(|machine| machine.id == request.machine_id && !machine.revoked)
        .ok_or("Unknown execution Machine")?;
    if !state.runtime_router.connected(&runtime.id) || !state.runtime_router.connected(&target.id) {
        return Err("Both Machines must be connected before creating this session".into());
    }
    let capacity: MachineCapacity = serde_json::from_value(
        runtime
            .inventory
            .get("capacity")
            .cloned()
            .unwrap_or_default(),
    )
    .unwrap_or_default();
    let active = state
        .hub
        .session_list()
        .iter()
        .filter(|meta| meta.machine_id == runtime.id && meta.status != Status::Exited)
        .count();
    if capacity.draining || active >= capacity.max_sessions as usize {
        return Err("Runtime Machine is draining or at capacity".into());
    }
    let workspaces: Vec<MachineWorkspace> = serde_json::from_value(
        target
            .inventory
            .get("workspaces")
            .cloned()
            .unwrap_or_default(),
    )
    .unwrap_or_default();
    let workspace = resolve_machine_workspace(&workspaces, Some(&request.cwd))?;
    let authentication = state.provider_auth.status(&request.provider);
    let generation = resolve_provider_generation(
        &state.provider_catalog,
        &state.plugin_catalog,
        &providers(runtime),
        &request.provider,
        authentication.as_ref(),
    )?;
    let executor = executor(state, &target.id).await?;
    if !accepts(&generation.behavior, &executor) {
        return Err("This installed Provider cannot run tools in the selected environment".into());
    }
    let session_id = state.supervisor.reserve_session_id();
    let Response::RuntimePrepared { runtime: location } = call(
        state,
        &runtime.id,
        Action::PrepareRuntime {
            session_id: session_id.clone(),
        },
    )
    .await?
    else {
        return Err("Runtime Machine could not prepare a private session entry".into());
    };
    if location.machine_id != runtime.id {
        return Err("Runtime placement changed during preparation".into());
    }
    let intent = PreparationV1 {
        schema: 1,
        phase: "preparing".into(),
        runtime: location,
        machine_id: target.id.clone(),
        workspace_id: workspace.id.clone(),
        source_path: workspace.canonical_path.clone(),
        executor_digest: executor.digest,
    };
    intent.validate()?;
    let binding = ExecutionBinding::from_record(
        serde_json::to_value(&intent).map_err(|_| "Invalid execution preparation")?,
    );
    {
        let fence = state.plugin_lifecycle_fences.read();
        if fence.contains_key(&(runtime.id.clone(), request.provider.clone())) {
            return Err("Provider is changing on the runtime Machine".into());
        }
        state
            .provider_auth
            .with_scheduling_generation(
                &request.provider,
                generation.auth_generation.is_some(),
                generation.auth_generation,
                || {
                    state.supervisor.register_session_on_with_id(
                        &session_id,
                        &request.provider,
                        Some(intent.runtime.cwd.clone()),
                        request.origin,
                        false,
                        crate::supervisor::SessionPlacement {
                            machine_id: &runtime.id,
                            workspace: Some(workspace),
                            execution_binding: Some(&binding),
                        },
                        crate::supervisor::ProviderGeneration {
                            version: &generation.version,
                            digest: &generation.digest,
                            auth_generation: generation.auth_generation,
                            behavior: Some(&generation.behavior),
                        },
                        product_session_owner(state.product_auth_enabled, &authenticated.principal),
                    )
                },
            )
            .map_err(|error| error.to_string())??;
    }
    if let Some(prompt) = request
        .initial_prompt
        .filter(|text| !text.trim().is_empty())
    {
        state.hub.requeue_prompt(
            &session_id,
            prompt,
            vec![],
            Some(format!("execution-initial:{session_id}")),
        );
    }
    schedule(state, &session_id);
    state
        .hub
        .session_info(&session_id)
        .map(|info| info.meta)
        .ok_or_else(|| "Session was removed during preparation".into())
}

pub(in crate::server) fn start_recovery(state: &Arc<AppState>) {
    let state = Arc::downgrade(state);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            let Some(state) = state.upgrade() else {
                return;
            };
            if *state.shutdown.borrow() {
                return;
            }
            for meta in state.hub.session_list() {
                if meta
                    .execution_binding
                    .as_ref()
                    .and_then(ExecutionBinding::preparation)
                    .is_some()
                {
                    schedule(&state, &meta.id);
                }
            }
        }
    });
}

fn schedule(state: &Arc<AppState>, session_id: &str) {
    if state.execution_closures.lock().contains(session_id) {
        return;
    }
    if !state
        .execution_preparations
        .lock()
        .insert(session_id.to_owned())
    {
        return;
    }
    let state = Arc::clone(state);
    let session_id = session_id.to_owned();
    tokio::spawn(async move {
        if let Err(error) = prepare(&state, &session_id).await
            && !state.execution_closures.lock().contains(&session_id)
            && state
                .hub
                .session_info(&session_id)
                .is_some_and(|info| info.meta.status != Status::Crashed)
        {
            state
                .hub
                .set_status(&session_id, Status::Crashed, Some(error));
        }
        state.execution_preparations.lock().remove(&session_id);
    });
}

async fn prepare(state: &AppState, session_id: &str) -> Result<(), String> {
    let info = state
        .hub
        .session_info(session_id)
        .ok_or("Session removed")?;
    let expected = info
        .meta
        .execution_binding
        .ok_or("No execution preparation")?;
    let intent = expected.preparation().ok_or("No execution preparation")?;
    let store = state
        .store
        .as_ref()
        .ok_or("Execution preparation requires durable storage")?;
    // The ordinary Hub writer owns insertion. Wait for that exact intent to be
    // durable before allocating any target worktree or process.
    let mut durable = false;
    for _ in 0..50 {
        if store
            .execution_binding_matches(session_id, expected.record())
            .await
            .map_err(|_| "Cannot verify durable execution preparation")?
        {
            durable = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    if !durable {
        return Err("Execution preparation has not reached durable storage".into());
    }
    let Response::Prepared { binding } = call(
        state,
        &intent.machine_id,
        Action::Prepare {
            session_id: session_id.into(),
            workspace_id: intent.workspace_id.clone(),
            runtime: intent.runtime.clone(),
        },
    )
    .await?
    else {
        return Err(
            "Execution environment is unavailable; the original environment has been retained"
                .into(),
        );
    };
    if !intent.accepts(&binding) {
        return Err(
            "Prepared environment no longer matches the selected workspace or executor".into(),
        );
    }
    let prepared = ExecutionBinding::from_record(
        serde_json::to_value(binding).map_err(|_| "Invalid execution binding")?,
    );
    let current = state
        .hub
        .session_info(session_id)
        .ok_or("Session removed")?;
    if current.meta.execution_binding.as_ref() != Some(&expected) {
        return Err("Session preparation changed".into());
    }
    let mut candidate = current.meta;
    candidate.execution_binding = Some(prepared.clone());
    candidate.require_runtime_launch()?;
    if !store
        .commit_execution_binding(session_id, expected.record(), prepared.record())
        .await
        .map_err(|_| "Cannot commit execution environment")?
    {
        return Err("Durable session preparation changed".into());
    }
    let closures = state.execution_closures.lock();
    if closures.contains(session_id) {
        return Err("Execution environment is closing".into());
    }
    state
        .hub
        .accept_execution_binding(session_id, &expected, prepared)?;
    state.supervisor.start_registered_session(session_id)
}

/// Removing the runtime alone cannot prove remote jobs stopped. Retain the
/// visible session on missing receipts, so the user can repeat the same close.
pub(in crate::server) fn delete(state: &AppState, session_id: &str) -> Result<(), String> {
    let info = state
        .hub
        .session_info(session_id)
        .ok_or("Session no longer exists")?;
    let record = info
        .meta
        .execution_binding
        .ok_or("No execution environment")?;
    let (machine, action) = if let Some(preparation) = record.preparation() {
        (
            preparation.machine_id.clone(),
            Action::AbandonPreparation {
                session_id: session_id.to_owned(),
                preparation: Box::new(preparation),
            },
        )
    } else {
        let binding = record.decode().map_err(|_| "This execution binding is not understood; retain the session until its original environment can be stopped")?;
        (
            binding.environment.machine_id.clone(),
            Action::Close {
                session_id: session_id.to_owned(),
                binding: Box::new(binding),
            },
        )
    };
    if !state
        .execution_closures
        .lock()
        .insert(session_id.to_owned())
    {
        return Ok(());
    }
    state.supervisor.delete_session(session_id);
    let request = Request {
        service_id: state.service_id.clone(),
        machine_id: machine.clone(),
        action,
    };
    let control = Arc::clone(&state.machine_control);
    let supervisor = Arc::clone(&state.supervisor);
    let hub = state.hub.clone();
    let closures = Arc::clone(&state.execution_closures);
    let session_id = session_id.to_owned();
    tokio::spawn(async move {
        if matches!(
            control.execution_request(&machine, request).await,
            Ok(Response::Closed)
        ) {
            supervisor.delete_session(&session_id);
            hub.delete_session(&session_id);
        } else {
            hub.set_status(&session_id, Status::Crashed, Some("Remote environment stop is unconfirmed. The session and worktree are retained; retry deletion when the environment is reachable.".into()));
        }
        closures.lock().remove(&session_id);
    });
    Ok(())
}
