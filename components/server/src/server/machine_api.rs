//! Machine handlers: inventory, enrollment, deployment health, refresh and
//! reconciliation, plugins, and the Machine WebSocket upgrade.

use super::*;

pub(super) fn machine_refresh_path(path: &str) -> bool {
    path.strip_prefix("/api/machines/")
        .and_then(|rest| rest.strip_suffix("/refresh"))
        .is_some_and(|machine| !machine.is_empty() && !machine.contains('/'))
}

pub(super) async fn api_machines(State(state): State<Arc<AppState>>) -> Response {
    match state.machine_snapshots.load().await {
        Ok(machines) => Json(machines).into_response(),
        Err(error) => {
            tracing::error!(%error, "listing Machines");
            (StatusCode::INTERNAL_SERVER_ERROR, "could not list machines").into_response()
        }
    }
}

pub(super) async fn api_machine_events(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    // Native login details are available only through the owner-checked
    // authentication endpoint, never through shared Machine diagnostics.
    Json(
        state
            .machine_control
            .events(&machine_id)
            .into_iter()
            .filter(machine_diagnostic_event_visible)
            .collect::<Vec<_>>(),
    )
    .into_response()
}

pub(super) fn machine_diagnostic_event_visible(
    event: &crate::machine_protocol::MachineEvent,
) -> bool {
    use crate::machine_protocol::MachineEvent;
    match event {
        MachineEvent::LoginChallenge { .. }
        | MachineEvent::LoginState { .. }
        | MachineEvent::ServiceAuthCandidate { .. }
        | MachineEvent::NativeProfileCompleted { .. }
        | MachineEvent::NativeProfileObserved { .. }
        | MachineEvent::NativeLogoutObserved { .. }
        | MachineEvent::LoginStopObserved { .. }
        | MachineEvent::WorkerExitObserved { .. }
        | MachineEvent::NativeBindingPrepared { .. }
        | MachineEvent::ProviderAuthRefreshCandidate { .. } => false,
        MachineEvent::CommandResult { request_id, .. } => {
            !request_id.starts_with("provider-login-")
        }
        _ => true,
    }
}

pub(super) fn machine_workspace_ids_sha256(inventory: &serde_json::Value) -> Option<String> {
    let mut ids = inventory
        .get("workspaces")?
        .as_array()?
        .iter()
        .map(|workspace| workspace.get("id")?.as_str())
        .collect::<Option<Vec<_>>>()?;
    ids.sort_unstable();
    Some(crate::admin::hex_sha256(&serde_json::to_vec(&ids).ok()?))
}

/// Content-free counts and writer flags of one connected Machine's durable state
/// (terminal deletions, Session lineages, pending cleanup). Each dataset is read
/// separately, so it is not an atomic snapshot. The Machine's reply is validated
/// against a closed schema and never proxied as free-form JSON; a Machine that
/// predates the report answers with an error, which is surfaced as such.
pub(super) async fn api_machine_durable_state(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    if !state.runtime_router.connected(&machine_id) {
        return (StatusCode::NOT_FOUND, "Machine is not connected").into_response();
    }
    let reply = match state
        .machine_control
        .adapter_request(&machine_id, "durable-state", serde_json::json!({}))
        .await
    {
        Ok(reply) => reply,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("Machine did not report durable state: {error}"),
            )
                .into_response();
        }
    };
    match crate::durable_state::DurableState::from_reply(reply) {
        Ok(status) => {
            // The Machine's closed report is returned unchanged; what this
            // Controller itself holds is added beside it, as counts only.
            let (sessions, with_lineage) = state.hub.lineage_counts(&machine_id);
            let mut body = serde_json::to_value(status).unwrap_or_default();
            if let Some(object) = body.as_object_mut() {
                object.insert(
                    "controllerObserved".to_owned(),
                    serde_json::json!({"sessions": sessions, "withLineage": with_lineage}),
                );
            }
            Json(body).into_response()
        }
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            format!("Machine reported invalid durable state: {error}"),
        )
            .into_response(),
    }
}

pub(super) async fn api_machine_deployment_health(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "machine registry unavailable",
        )
            .into_response();
    };
    let machines = match store.list_machines().await {
        Ok(machines) => machines,
        Err(error) => {
            tracing::error!(%error, "reading Machine deployment health");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not read Machine deployment health",
            )
                .into_response();
        }
    };
    let Some(machine) = machines
        .into_iter()
        .find(|machine| machine.id == machine_id && !machine.revoked)
    else {
        return (StatusCode::NOT_FOUND, "unknown or revoked Machine").into_response();
    };
    let active_acp_generation = machine
        .inventory
        .get("components")
        .cloned()
        .and_then(|value| {
            serde_json::from_value::<Vec<crate::machine_protocol::ComponentInventory>>(value).ok()
        })
        .and_then(|components| {
            components.into_iter().find_map(|component| {
                (component.id.kind == crate::machine_protocol::ComponentKind::AcpRuntime
                    && component.state == crate::machine_protocol::ComponentState::Active)
                    .then_some(component.generation)
            })
        });
    Json(MachineDeploymentHealthResponse {
        connected: state.runtime_router.connected(&machine.id),
        status: machine.status,
        active_acp_generation,
        workspace_revision: machine
            .inventory
            .get("workspace_revision")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        // Host activation needs proof that retained Machines accepted the
        // candidate workspace manifest, without exposing private paths or
        // project names on the public deployment-health surface.
        workspace_ids_sha256: machine_workspace_ids_sha256(&machine.inventory),
        workspace_owner: machine
            .inventory
            .get("workspace_owner")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok()),
    })
    .into_response()
}

pub(super) async fn api_machine_create_enrollment(
    State(state): State<Arc<AppState>>,
    Json(request): Json<MachineEnrollmentRequest>,
) -> Response {
    const TTL_SECONDS: i64 = 900;
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Machine enrollment requires persistence",
        )
            .into_response();
    };
    let display_name = request
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Computer")
        .to_owned();
    let requested_id = request
        .machine_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned);
    let mut last_error = None;
    for _ in 0..8 {
        let machine_id = if let Some(id) = requested_id.as_deref() {
            id.to_owned()
        } else {
            match crate::store::generate_machine_id() {
                Ok(id) => id,
                Err(error) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
                }
            }
        };
        match store
            .create_machine_enrollment(&machine_id, &display_name, TTL_SECONDS)
            .await
        {
            Ok(token) => {
                return Json(MachineEnrollmentResponse {
                    machine_id,
                    display_name,
                    token,
                    expires_in_seconds: TTL_SECONDS,
                })
                .into_response();
            }
            Err(error) if requested_id.is_some() => {
                return (StatusCode::CONFLICT, error.to_string()).into_response();
            }
            Err(error) => last_error = Some(error),
        }
    }
    (
        StatusCode::CONFLICT,
        last_error.map_or_else(
            || "could not allocate a Machine id".to_owned(),
            |error| error.to_string(),
        ),
    )
        .into_response()
}

pub(super) async fn api_machine_cancel_enrollment(
    State(state): State<Arc<AppState>>,
    Json(request): Json<MachineEnrollmentCancelRequest>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Machine enrollment requires persistence",
        )
            .into_response();
    };
    if request.token.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "enrollment token is required").into_response();
    }
    match store.cancel_machine_enrollment(&request.token).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) fn machine_command_error_status(error: &str) -> StatusCode {
    if error == "Machine command timed out" {
        StatusCode::GATEWAY_TIMEOUT
    } else if error == "Machine command response channel closed" {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::CONFLICT
    }
}

pub(super) fn machine_request_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        random_machine_token().unwrap_or_else(|_| now_ms().to_string())
    )
}

pub(super) async fn api_machine_refresh(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    let request_id = machine_request_id("refresh");
    await_machine_command(
        state.as_ref(),
        &machine_id,
        request_id.clone(),
        crate::machine_protocol::MachineCommand::RefreshInventory {
            request_id: request_id.clone(),
        },
        "refresh_inventory",
        MACHINE_INVENTORY_COMMAND_TIMEOUT,
    )
    .await
}

pub(super) async fn api_machine_reconcile(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    let desired = state.desired_machine_components.current();
    if desired.is_empty() {
        return (
            StatusCode::PRECONDITION_FAILED,
            "no signed Machine component manifest is configured",
        )
            .into_response();
    }
    let request_id = machine_request_id("reconcile");
    await_machine_command(
        state.as_ref(),
        &machine_id,
        request_id.clone(),
        crate::machine_protocol::MachineCommand::Reconcile {
            request_id: request_id.clone(),
            components: desired.components.clone(),
        },
        "reconcile_components",
        MACHINE_COMPONENT_COMMAND_TIMEOUT,
    )
    .await
}

pub(super) async fn api_machine_reconcile_one(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
    Json(component_id): Json<crate::machine_protocol::ComponentId>,
) -> Response {
    let Some(component) = state
        .desired_machine_components
        .current()
        .find(&component_id)
        .cloned()
    else {
        return (StatusCode::NOT_FOUND, "no signed update for this component").into_response();
    };
    let request_id = machine_request_id("reconcile-one");
    await_machine_command(
        state.as_ref(),
        &machine_id,
        request_id.clone(),
        crate::machine_protocol::MachineCommand::Reconcile {
            request_id: request_id.clone(),
            components: vec![component],
        },
        "reconcile_component",
        MACHINE_COMPONENT_COMMAND_TIMEOUT,
    )
    .await
}

pub(super) async fn api_machine_update_npm(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
    Json(component): Json<crate::machine_protocol::ComponentId>,
) -> Response {
    let request_id = machine_request_id("update-npm");
    await_machine_command(
        state.as_ref(),
        &machine_id,
        request_id.clone(),
        crate::machine_protocol::MachineCommand::UpdateNpmComponent {
            request_id: request_id.clone(),
            component,
        },
        "update_npm_component",
        MACHINE_COMPONENT_COMMAND_TIMEOUT,
    )
    .await
}

pub(super) async fn api_machine_revoke(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "persistence unavailable").into_response();
    };
    match store.revoke_machine(&machine_id).await {
        Ok(()) => {
            state.machine_control.disconnect(&machine_id);
            state.runtime_router.remove(&machine_id);
            state.component_convergence.forget(&machine_id);
            state.machine_snapshots.publish().await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

pub(super) fn machine_auth_discovery_requested(headers: &HeaderMap) -> bool {
    headers
        .get("x-cowboy-auth-machine")
        .is_some_and(|value| value == "1")
}

pub(super) async fn api_machine_plugins(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "machine registry unavailable",
        )
            .into_response();
    };
    match store.list_machines().await {
        Ok(machines) => machines
            .into_iter()
            .find(|machine| machine.id == machine_id && !machine.revoked)
            .map_or_else(
                || (StatusCode::NOT_FOUND, "unknown or revoked Machine").into_response(),
                |machine| {
                    Json(
                        machine
                            .inventory
                            .get("plugins")
                            .or_else(|| machine.inventory.get("providers"))
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!([])),
                    )
                    .into_response()
                },
            ),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) async fn api_machine_provider_inventory_compat(
    State(state): State<Arc<AppState>>,
    Path(machine_id): Path<String>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "machine registry unavailable",
        )
            .into_response();
    };
    match store.list_machines().await {
        Ok(machines) => {
            let Some(machine) = machines
                .into_iter()
                .find(|machine| machine.id == machine_id && !machine.revoked)
            else {
                return (StatusCode::NOT_FOUND, "unknown or revoked Machine").into_response();
            };
            let plugins = machine
                .inventory
                .get("plugins")
                .or_else(|| machine.inventory.get("providers"))
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            let providers = plugins
                .into_iter()
                .filter_map(|mut plugin| {
                    let object = plugin.as_object_mut()?;
                    if object
                        .get("plugin_kind")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|kind| kind != "agent_provider")
                    {
                        return None;
                    }
                    if let Some(id) = object.remove("plugin_id") {
                        object.insert("provider_id".to_owned(), id);
                    }
                    if let Some(version) = object.remove("plugin_version") {
                        object.insert("provider_version".to_owned(), version);
                    }
                    object.remove("plugin_kind");
                    Some(plugin)
                })
                .collect::<Vec<_>>();
            Json(providers).into_response()
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) async fn api_machine_plugin_uninstall_plan(
    State(state): State<Arc<AppState>>,
    Path((machine_id, provider_id)): Path<(String, String)>,
    authenticated: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
) -> Response {
    let actor = match plugin_uninstall::request_actor(&state, authenticated, &headers) {
        Ok(actor) => actor,
        Err(status) => return status.into_response(),
    };
    if let Some(refusal) = service_managed_refusal(&state, &machine_id) {
        return refusal;
    }
    preview_machine_plugin_uninstall(state, machine_id, provider_id, actor).await
}

pub(super) async fn api_machine_service(
    State(state): State<Arc<AppState>>,
) -> Json<MachineServiceResponse> {
    Json(MachineServiceResponse {
        service_id: state.service_id.clone(),
    })
}

pub(super) async fn api_machine_enroll(
    State(state): State<Arc<AppState>>,
    Json(request): Json<MachineEnrollRequest>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Machine enrollment requires persistence",
        )
            .into_response();
    };
    let public_key = match crate::machine_auth::validate_public_key(&request.public_key) {
        Ok(public_key) => public_key,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    match store
        .consume_machine_enrollment(&request.token, &public_key, &request.encryption_public_key)
        .await
    {
        Ok(machine) => {
            let response = MachineEnrollResponse {
                service_id: state.service_id.clone(),
                machine_id: machine.id,
                display_name: machine.display_name,
                fingerprint: machine.fingerprint,
            };
            state.machine_snapshots.publish().await;
            (StatusCode::CREATED, Json(response)).into_response()
        }
        Err(error) => {
            tracing::warn!(%error, "Machine enrollment rejected");
            (
                StatusCode::UNAUTHORIZED,
                "invalid or expired enrollment token",
            )
                .into_response()
        }
    }
}

pub(super) async fn machine_ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let chunked = headers
        .get(crate::machine_transport::HEADER)
        .is_some_and(|value| value == crate::machine_transport::CHUNKED);
    let compressed = chunked
        && headers
            .get(crate::machine_transport::COMPRESSION_HEADER)
            .is_some_and(|value| value == crate::machine_transport::DEFLATE);
    let paced = chunked
        && headers
            .get(crate::machine_transport::PACING_HEADER)
            .is_some_and(|value| value == crate::machine_transport::CREDIT);
    let stream = chunked
        && headers
            .get(crate::machine_transport::STREAM_HEADER)
            .is_some_and(|value| value == crate::machine_transport::STREAM);
    // Bulk lanes need the chunked transport; the token binds them to this
    // connection once its Machine has proven its identity.
    let lane_token = (chunked
        && headers
            .get(crate::machine_lanes::LANES_HEADER)
            .is_some_and(|value| value == crate::machine_lanes::LANES))
    .then(random_machine_token)
    .and_then(Result::ok);
    let upgrade_token = lane_token.clone();
    let mut response = ws.on_upgrade(move |socket| {
        handle_machine_ws(
            socket,
            state,
            (chunked, compressed, paced, stream),
            upgrade_token,
        )
    });
    if let Some(token) = lane_token.as_deref()
        && let Ok(value) = axum::http::HeaderValue::from_str(token)
    {
        response.headers_mut().insert(
            crate::machine_lanes::LANES_HEADER,
            axum::http::HeaderValue::from_static(crate::machine_lanes::LANES),
        );
        response
            .headers_mut()
            .insert(crate::machine_lanes::LANE_TOKEN_HEADER, value);
    }
    if chunked {
        response.headers_mut().insert(
            crate::machine_transport::HEADER,
            axum::http::HeaderValue::from_static(crate::machine_transport::CHUNKED),
        );
    }
    if compressed {
        response.headers_mut().insert(
            crate::machine_transport::COMPRESSION_HEADER,
            axum::http::HeaderValue::from_static(crate::machine_transport::DEFLATE),
        );
    }
    if paced {
        response.headers_mut().insert(
            crate::machine_transport::PACING_HEADER,
            axum::http::HeaderValue::from_static(crate::machine_transport::CREDIT),
        );
    }
    if stream {
        response.headers_mut().insert(
            crate::machine_transport::STREAM_HEADER,
            axum::http::HeaderValue::from_static(crate::machine_transport::STREAM),
        );
    }
    response
}
