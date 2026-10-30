//! Creation is a durable, non-runnable intent followed by one target-owned
//! preparation and a storage compare-and-set. Recovery reuses the same identity.

use super::super::{
    AppState, AuthenticatedProductRequest, product_session_owner, provider_accounts,
    resolve_machine_workspace, resolve_provider_generation,
};
use crate::core::Status;
use crate::execution_environment::{BindingV1, ExecutionBinding, PreparationV1};
use crate::machine_protocol::execution::{Action, ExecutorInventory, Refusal, Request, Response};
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
            if !state
                .supervisor
                .runtime_allowed(&installed.plugin_id, runtime)
            {
                continue;
            }
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
    #[serde(default)]
    context_sources: Vec<String>,
    #[serde(skip)]
    pub(in crate::server) reserved_id: Option<String>,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    provider_account_id: Option<crate::provider_account::ProviderAccountId>,
    #[serde(default)]
    runtime_machine_id: String,
    #[serde(default)]
    machine_id: String,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    folder_id: Option<String>,
    #[serde(default)]
    preset: Option<String>,
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
            "provider_account_id":meta.provider_account_id,
            "machine_id":meta.machine_id,"cwd":meta.cwd,"execution_binding":meta.execution_binding,
        }))).into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

async fn create_checked(
    state: &Arc<AppState>,
    authenticated: &AuthenticatedProductRequest,
    mut request: CreateRequest,
) -> Result<crate::core::SessionMeta, String> {
    use super::super::folder_defaults::{self, value};
    use crate::session_folders::defaults::DefaultKey;
    super::super::session_context::validate_sources(
        state,
        &authenticated.principal,
        &request.context_sources,
    )?;
    let defaults = folder_defaults::resolve(
        state,
        &authenticated.principal,
        request.folder_id.as_deref(),
    )?;
    if request.provider.is_empty() {
        request.provider =
            value(&defaults, DefaultKey::Provider).unwrap_or_else(|| "claude-code".into());
    }
    let provider_account_id = super::super::provider_accounts::resolve_for_create(
        state,
        &authenticated.principal.user_id,
        &request.provider,
        request.provider_account_id.as_ref(),
    )
    .await
    .map_err(str::to_owned)?;
    if request.machine_id.is_empty() {
        request.machine_id = value(&defaults, DefaultKey::Machine).unwrap_or_default();
    }
    if request.runtime_machine_id.is_empty() {
        request.runtime_machine_id = value(&defaults, DefaultKey::RuntimeMachine)
            .or_else(|| {
                state
                    .project_placement
                    .snapshot()
                    .default_runtime_machine_id
                    .clone()
            })
            .unwrap_or_default();
    }
    if request.cwd.is_empty() {
        request.cwd = value(&defaults, DefaultKey::Project).unwrap_or_default();
    }
    if request.folder_id.is_some() && (request.machine_id.is_empty() || request.cwd.is_empty()) {
        if request.cwd.is_empty() && defaults.values.get(&DefaultKey::Project) == Some(&None) {
            return Err(
                "This directory has no project binding; select a project explicitly".into(),
            );
        }
        (request.machine_id, request.cwd) = folder_defaults::default_workspace(
            state,
            (!request.machine_id.is_empty()).then_some(request.machine_id.as_str()),
            (!request.cwd.is_empty()).then_some(request.cwd.as_str()),
        )
        .await?;
    }
    if request.preset.is_none() {
        request.preset = value(&defaults, DefaultKey::Preset);
    }
    if !state
        .supervisor
        .runtime_allowed(&request.provider, &request.runtime_machine_id)
    {
        return Err("Provider policy does not permit this runtime Machine".into());
    }
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
    if capacity.draining {
        return Err("Runtime Machine is draining".into());
    }
    let workspaces: Vec<MachineWorkspace> = serde_json::from_value(
        target
            .inventory
            .get("workspaces")
            .cloned()
            .unwrap_or_default(),
    )
    .unwrap_or_default();
    let workspace = if request.folder_id.is_some() {
        folder_defaults::workspace(&workspaces, Some(&request.cwd))?
    } else {
        resolve_machine_workspace(&workspaces, Some(&request.cwd))?
    };
    let generation = provider_accounts::registration::generation(
        state,
        &providers(runtime),
        &request.provider,
        &authenticated.principal.user_id,
        &provider_account_id,
        &runtime.id,
    )
    .await?;
    let configuration = folder_defaults::preset_values(
        state,
        &request.provider,
        &generation,
        request.preset.as_deref(),
    )?;
    let executor = executor(state, &target.id).await?;
    if !accepts(&generation.behavior, &executor) {
        return Err("This installed Provider cannot run tools in the selected environment".into());
    }
    let session_id = request
        .reserved_id
        .unwrap_or_else(|| state.supervisor.reserve_session_id());
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
    let registration = state.supervisor.prepare_session_on_with_id(
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
            account_id: Some(&provider_account_id),
            version: &generation.version,
            digest: &generation.digest,
            auth_generation: generation.auth_generation,
            behavior: Some(&generation.behavior),
        },
        product_session_owner(state.product_auth_enabled, &authenticated.principal),
    )?;
    super::super::session_context::initialize(
        state,
        &authenticated.principal,
        &session_id,
        request.context_sources,
        request
            .initial_prompt
            .as_ref()
            .filter(|text| !text.trim().is_empty())
            .map(|_| format!("execution-initial:{session_id}")),
    )
    .await?;
    provider_accounts::registration::register(state, registration).await?;
    folder_defaults::apply_created(
        state,
        &authenticated.principal,
        &session_id,
        request.folder_id.as_deref(),
        configuration,
    )?;
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
        crate::server::session_maintenance::wait(
            &state.hub,
            &session_id,
            "preparing the workspace",
        );
    }
    super::super::session_context::initialized(state, &session_id).await?;
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
    if state.execution_closures.lock().contains_key(session_id) {
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
            && !state.execution_closures.lock().contains_key(&session_id)
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
    let response = call(
        state,
        &intent.machine_id,
        Action::Prepare {
            session_id: session_id.into(),
            workspace_id: intent.workspace_id.clone(),
            runtime: intent.runtime.clone(),
            // A fork's new worktree starts from its source's worktree state.
            base_session_id: state
                .hub
                .session_fork(session_id)
                .map(|record| record.source_session_id),
        },
    )
    .await?;
    let binding = prepared_response(session_id, &intent.machine_id, response)?;
    let mut resolved_source = None;
    if !intent.accepts(&binding) {
        let mut alias_intent = intent.clone();
        alias_intent
            .source_path
            .clone_from(&binding.workspace.source_path);
        if intent.validate().is_err() || !alias_intent.accepts(&binding) {
            return Err(
                "Prepared environment no longer matches the selected workspace or executor".into(),
            );
        }
        // Never canonicalize a remote Machine's paths on the Controller or
        // weaken the binding comparison. The Machine's existing discovery
        // operation resolves the selected source; Prepare already verified
        // its retained contract and live keeper. Keep that binding intact.
        let discovered = state
            .machine_control
            .project_request(
                &intent.machine_id,
                crate::machine_protocol::projects::Request::Discover {
                    root: intent.source_path.clone(),
                },
            )
            .await?;
        if discovered.get("root").and_then(serde_json::Value::as_str)
            != Some(binding.workspace.source_path.as_str())
        {
            return Err(
                "Prepared environment no longer matches the selected workspace or executor".into(),
            );
        }
        resolved_source = Some(binding.workspace.source_path.clone());
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
    if closures.contains_key(session_id) {
        return Err("Execution environment is closing".into());
    }
    state
        .supervisor
        .finish_session_preparation(session_id, |hub| {
            hub.accept_execution_binding(
                session_id,
                &expected,
                prepared,
                resolved_source.as_deref(),
            )
        })
}

fn prepared_response(
    session_id: &str,
    machine_id: &str,
    response: Response,
) -> Result<BindingV1, String> {
    match response {
        Response::Prepared { binding } => Ok(binding),
        Response::Refused { reason } => {
            // Retain the closed, content-free Machine reason. Debug on Response
            // intentionally hides its payload, so logging that alone loses it.
            tracing::warn!(
                session = session_id,
                machine = machine_id,
                ?reason,
                "execution preparation refused"
            );
            let detail = match reason {
                Refusal::Unavailable => "The execution Machine is not ready",
                Refusal::InvalidRequest => "The execution Machine rejected the preparation request",
                Refusal::IdentityMismatch => {
                    "The execution environment does not match the selected workspace or runtime"
                }
                Refusal::WorkspaceUnavailable => "The selected workspace could not be prepared",
                Refusal::EnvironmentLost => "The execution environment could not be verified",
                Refusal::PreparationFailed => "The execution Machine could not finish preparation",
                Refusal::Capacity => "The execution Machine has reached its environment capacity",
            };
            Err(format!(
                "{detail} ({machine_id}). This session is retained; Cowboy will retry preparation."
            ))
        }
        other => {
            tracing::warn!(
                session = session_id,
                machine = machine_id,
                response = ?other,
                "unexpected execution preparation response"
            );
            Err(format!(
                "The execution Machine returned an unexpected preparation response ({machine_id}). This session is retained; Cowboy will retry preparation."
            ))
        }
    }
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
    {
        let mut maintenance = state.execution_closures.lock();
        match maintenance.entry(session_id.to_owned()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(super::Maintenance::Close);
            }
            std::collections::hash_map::Entry::Occupied(entry)
                if *entry.get() == super::Maintenance::Close =>
            {
                return Ok(());
            }
            _ => {
                return Err(
                    "Execution recovery is in progress; retain the session until it completes"
                        .into(),
                );
            }
        }
    }
    state.supervisor.delete_session(session_id);
    // The Machine stop can outlive any client acknowledgement deadline, for
    // example while its broker connection is reconnecting.
    state.hub.set_closing(session_id, true);
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
        // Release the closure first so a retry prompted by this edge is accepted.
        hub.set_closing(&session_id, false);
    });
    Ok(())
}

#[cfg(test)]
mod preparation_tests {
    use super::*;

    #[test]
    fn preparation_preserves_the_binding_and_distinguishes_machine_refusals() {
        let binding = crate::execution_environment::fixture().decode().unwrap();
        assert_eq!(
            prepared_response(
                "session",
                "target",
                Response::Prepared {
                    binding: binding.clone()
                }
            ),
            Ok(binding)
        );
        let reasons = [
            Refusal::Unavailable,
            Refusal::InvalidRequest,
            Refusal::IdentityMismatch,
            Refusal::WorkspaceUnavailable,
            Refusal::EnvironmentLost,
            Refusal::PreparationFailed,
            Refusal::Capacity,
        ];
        let messages: std::collections::HashSet<_> = reasons
            .into_iter()
            .map(|reason| {
                let message = prepared_response("session", "target", Response::Refused { reason })
                    .unwrap_err();
                assert!(message.contains("(target)"));
                assert!(message.contains("retry preparation"));
                assert!(!message.contains("original environment"));
                message
            })
            .collect();
        assert_eq!(messages.len(), reasons.len());
        assert!(
            prepared_response("session", "target", Response::Closed)
                .unwrap_err()
                .contains("unexpected")
        );
    }
}
