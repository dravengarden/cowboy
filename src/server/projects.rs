//! Native project and AI-placement surface. Transport uses enrolled Machines;
//! Stormbird and directory naming are not project identities or routing rules.
use super::*;
use crate::machine_protocol::projects::Request;
use crate::project_placement::{Mode, Policy};

pub(super) async fn policies(State(state): State<Arc<AppState>>) -> Response {
    no_store_json(
        StatusCode::OK,
        serde_json::to_value(state.project_placement.snapshot()).expect("placement JSON"),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PolicyUpdate {
    expected_revision: String,
    policy: Policy,
    preferred: bool,
}

pub(super) async fn update_policy(
    State(state): State<Arc<AppState>>,
    Path(machine): Path<String>,
    Json(request): Json<PolicyUpdate>,
) -> Response {
    let Some(store) = &state.store else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let machines = match store.list_machines().await {
        Ok(machines) => machines,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let enrolled = |id: &str| machines.iter().any(|m| m.id == id && !m.revoked);
    if !enrolled(&machine)
        || request
            .policy
            .remote_targets
            .as_ref()
            .is_some_and(|targets| {
                targets.len() > 256 || targets.iter().any(|id| id == &machine || !enrolled(id))
            })
    {
        return (
            StatusCode::BAD_REQUEST,
            "Choose enrolled target Machines distinct from this runtime",
        )
            .into_response();
    }
    if request.preferred && request.policy.agent_mode == Mode::Disabled {
        return (
            StatusCode::BAD_REQUEST,
            "A disabled AI runtime cannot be preferred",
        )
            .into_response();
    }
    match state.project_placement.update(
        &request.expected_revision,
        machine,
        request.policy,
        request.preferred,
    ) {
        Ok(value) => no_store_json(
            StatusCode::OK,
            serde_json::to_value(value).expect("placement JSON"),
        ),
        Err(error) => (StatusCode::CONFLICT, error.to_string()).into_response(),
    }
}

pub(super) async fn list(
    State(state): State<Arc<AppState>>,
    Path(machine): Path<String>,
) -> Response {
    request(State(state), Path(machine), Json(Request::List)).await
}

pub(super) async fn request(
    State(state): State<Arc<AppState>>,
    Path(machine): Path<String>,
    Json(request): Json<Request>,
) -> Response {
    match state
        .machine_control
        .project_request(&machine, request)
        .await
    {
        Ok(value) => no_store_json(StatusCode::OK, value),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct PlacementQuery {
    machine_id: String,
}

/// One readiness lookup per target, regardless of how many AI installations
/// are available. The create endpoints recheck policy and exact generations.
pub(super) async fn placements(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PlacementQuery>,
) -> Response {
    use super::execution::sessions::{accepts, executor, providers};
    let Some(store) = &state.store else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let machines = match store.list_machines().await {
        Ok(machines) => machines,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let policy = state.project_placement.snapshot();
    if !machines
        .iter()
        .any(|m| m.id == query.machine_id && !m.revoked)
    {
        return (StatusCode::NOT_FOUND, "Unknown project Machine").into_response();
    }
    let remote_executor = if machines.iter().any(|m| {
        m.id != query.machine_id
            && policy.allows(&m.id, &query.machine_id)
            && state.runtime_router.connected(&m.id)
    }) {
        executor(&state, &query.machine_id).await.ok()
    } else {
        None
    };
    let mut placements = Vec::new();
    for machine in &machines {
        if machine.revoked
            || !state.runtime_router.connected(&machine.id)
            || !state.runtime_router.connected(&query.machine_id)
            || !policy.allows(&machine.id, &query.machine_id)
        {
            continue;
        }
        let inventory = providers(machine);
        let remote = machine.id != query.machine_id;
        for installed in &inventory {
            let auth = state.provider_auth.status(&installed.plugin_id);
            if let Ok(generation) = resolve_provider_generation(
                &state.provider_catalog,
                &state.plugin_catalog,
                &inventory,
                &installed.plugin_id,
                auth.as_ref(),
            ) && (!remote
                || remote_executor
                    .as_ref()
                    .is_some_and(|executor| accepts(&generation.behavior, executor)))
            {
                placements.push(serde_json::json!({"runtime_machine_id":machine.id,"provider":installed.plugin_id,"mode":if remote {"remote"} else {"local"}}));
            }
        }
    }
    no_store_json(
        StatusCode::OK,
        serde_json::json!({"machine_id":query.machine_id,"default_runtime_machine_id":policy.default_runtime_machine_id,"placements":placements}),
    )
}
