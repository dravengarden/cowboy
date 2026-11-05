//! Plugin catalog and call handlers, install compatibility, uninstall
//! preview, fences and generation references.

use super::*;

/// Whether a session holds one of its Machine's worker slots. Capacity bounds
/// live Agent processes: a starting session counts before its worker
/// registers, while a session whose worker exited (or was never revived after
/// a restart) holds no memory and must not keep the Machine full.
pub(super) fn holds_worker_slot(
    router: &RuntimeRouter,
    session: &crate::core::SessionMeta,
) -> bool {
    match session.status {
        crate::agent_model::Status::Exited => false,
        crate::agent_model::Status::Starting => {
            !session
                .execution_binding
                .as_ref()
                .is_some_and(|binding| binding.preparation().is_some())
                && !(session.origin == SessionOrigin::Web
                    && session.machine_id != "local"
                    && session.workspace_source_path.as_deref() == Some(session.cwd.as_str()))
        }
        _ => router
            .runtime(&session.machine_id)
            .is_some_and(|runtime| runtime.has_worker(&session.id)),
    }
}

/// The installed Plugin that already serves a legacy provider component slot.
/// A session runs that Plugin's own pinned generation, so the host binary this
/// component reports is not what executes. Offering to update it would ask
/// someone to maintain a runtime Cowboy no longer runs — and CR-2 is explicit
/// that an adapter or CLI is never a separate product component.
pub(super) fn superseding_plugin(
    component: &crate::machine_protocol::ComponentId,
    plugins: &[crate::machine_protocol::PluginInventory],
) -> Option<String> {
    if !matches!(
        component.kind,
        crate::machine_protocol::ComponentKind::ProviderAdapter
            | crate::machine_protocol::ComponentKind::ProviderCli
    ) {
        return None;
    }
    let occupants = crate::plugin_runtime_args::occupancy_provider_ids(&component.slot);
    plugins
        .iter()
        .find(|plugin| {
            plugin.state == crate::machine_protocol::PluginInstallationState::Active
                && occupants.contains(&plugin.plugin_id.as_str())
        })
        .map(|plugin| plugin.plugin_id.clone())
}

/// Generations each `(Machine, Provider)` must keep: every one pinned by a
/// session that is not deleted, including exited and hibernated sessions,
/// because a revive launches exactly its pinned generation.
pub(super) fn referenced_generations(
    sessions: &[crate::core::SessionMeta],
) -> HashMap<(String, String), BTreeSet<String>> {
    let mut referenced: HashMap<(String, String), BTreeSet<String>> = HashMap::new();
    for session in sessions {
        if session.provider_generation_digest.is_empty() {
            continue;
        }
        referenced
            .entry((session.machine_id.clone(), session.provider.clone()))
            .or_default()
            .insert(session.provider_generation_digest.clone());
    }
    referenced
}

/// `(machine, provider)` credential generations live sessions were launched
/// with. Named-account sessions are included: an over-approximation only keeps
/// more on the default account's store.
pub(super) fn referenced_auth_generations(
    sessions: &[crate::core::SessionMeta],
) -> HashMap<(String, String), BTreeSet<u64>> {
    let mut referenced: HashMap<(String, String), BTreeSet<u64>> = HashMap::new();
    for session in sessions {
        if let Some(generation) = session.provider_auth_generation {
            referenced
                .entry((session.machine_id.clone(), session.provider.clone()))
                .or_default()
                .insert(generation);
        }
    }
    referenced
}

pub(super) fn plugin_fence_state_for_session(
    hub: &Hub,
    fences: &PluginLifecycleFences,
    session_id: &str,
) -> Option<PluginFenceState> {
    let key = provider_fence_key_for_session(hub, session_id)?;
    fences.read().get(&key).copied()
}

pub(super) fn provider_fence_key_for_session(
    hub: &Hub,
    session_id: &str,
) -> Option<(String, String)> {
    hub.session_list()
        .into_iter()
        .find(|session| session.id == session_id)
        .map(|session| (session.machine_id, session.provider))
}

pub(super) async fn api_plugins(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let authentication_executors = connected_provider_authentication_executors(
        &state,
        machine_auth_discovery_requested(&headers),
    )
    .await;
    let inventory = state.plugin_catalog.inventory();
    let hosts = inventory
        .runtime
        .map_or_else(Vec::new, |runtime| runtime.public_descriptors());
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::VARY, "x-cowboy-auth-machine"),
        ],
        Json(serde_json::json!({
            "component_release": crate::plugin::active_component_release(),
            "platform": {
                "schema_version": 1,
                "storage": state.plugin_storage.kind(),
                "slots": crate::plugin_host::PluginSlotId::all()
                    .iter()
                    .map(|slot| slot.as_str())
                    .collect::<Vec<_>>(),
                "host_api": crate::plugin_host::PLUGIN_HOST_API_VERSION,
                "renderer_schema": crate::plugin_host::PLUGIN_RENDERER_SCHEMA_VERSION,
                "native_host_api": crate::plugin_host::PLUGIN_NATIVE_HOST_API_VERSION,
                "surface_transport": "typed-renderer-v1",
                "hosts": hosts,
            },
            "plugins": inventory.entries,
            "providers": state.provider_catalog.entries(),
            "authentications": state.provider_auth.statuses(),
            "authentication_executors": authentication_executors,
        })),
    )
        .into_response()
}

pub(super) async fn api_plugin_call(
    State(state): State<Arc<AppState>>,
    Path(plugin_id): Path<String>,
    Query(identity): Query<PluginCallIdentity>,
    body: axum::body::Bytes,
) -> Response {
    if crate::plugin_host::validate_plugin_id(&plugin_id).is_err() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(runtime) = state.plugin_catalog.runtime() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let host = match (identity.version.as_deref(), identity.digest.as_deref()) {
        (None, None) => runtime.default_host(&plugin_id),
        (Some(version), Some(digest)) => runtime.exact_host(&plugin_id, version, digest),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "plugin version and digest must be supplied together",
            )
                .into_response();
        }
    };
    let Some(host) = host.cloned() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some((program, args)) = host.rpc_argv.split_first() else {
        let Ok(request) = serde_json::from_slice::<serde_json::Value>(&body) else {
            return (StatusCode::BAD_REQUEST, "plugin call body must be JSON").into_response();
        };
        return api_builtin_plugin_call(state, &host, request).await;
    };
    match crate::plugin_process::run_plugin_command(program, args, &body).await {
        Ok(output) => {
            if !output.status.success() {
                return (
                    StatusCode::BAD_GATEWAY,
                    format!("plugin rpc exited {}", output.status),
                )
                    .into_response();
            }
            let json = serde_json::from_slice::<serde_json::Value>(&output.stdout).is_ok();
            let mut response = output.stdout.into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static(if json {
                    "application/json; charset=utf-8"
                } else {
                    "text/plain; charset=utf-8"
                }),
            );
            response
        }
        Err(failure) if failure.timed_out => {
            (StatusCode::GATEWAY_TIMEOUT, "plugin rpc timed out").into_response()
        }
        Err(failure) => (StatusCode::BAD_GATEWAY, failure.error.to_string()).into_response(),
    }
}

/// Host primitives available to a UI-only plugin generation. They are keyed by
/// declared capability and account identity, never by Plugin or Provider id.
pub(super) async fn api_builtin_plugin_call(
    state: Arc<AppState>,
    host: &crate::plugin_runtime::ActivatedHostPlugin,
    request: serde_json::Value,
) -> Response {
    let operation = request.get("operation").and_then(serde_json::Value::as_str);
    if operation != Some("usage_activity") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(provider) = host.usage.as_ref().map(|usage| usage.account.as_str()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if request
        .get("provider")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|candidate| candidate != provider)
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let string_field = |name: &str| {
        request
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    };
    let query = UsageActivityQuery {
        window: string_field("window"),
        model: string_field("model"),
        agent: string_field("agent"),
        from_ms: request.get("from_ms").and_then(serde_json::Value::as_i64),
        to_ms: request.get("to_ms").and_then(serde_json::Value::as_i64),
    };
    api_usage_activity(State(state), Path(provider.to_owned()), Query(query)).await
}

pub(super) async fn api_plugin_catalog_refresh(State(state): State<Arc<AppState>>) -> Response {
    match state
        .plugin_catalog
        .refresh_with_runtime(&state.plugin_storage)
        .await
    {
        Ok(count) => {
            if let Err(error) = state.provider_catalog.refresh_external() {
                return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
            }
            adopt_shared_provider_authentication(&state).await;
            Json(serde_json::json!({ "external_releases": count })).into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

pub(super) async fn plugin_install_compatibility(
    state: &AppState,
    machine_id: &str,
    desired: &crate::machine_protocol::DesiredPlugin,
) -> Result<Option<cowboy_plugin_sdk::PluginCompatibilityProblem>, String> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(|| "Plugin lifecycle requires persistence".to_owned())?;
    let machine = store
        .list_machines()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|machine| machine.id == machine_id && !machine.revoked)
        .ok_or_else(|| format!("unknown or revoked Machine {machine_id:?}"))?;
    let Some(raw_contracts) = machine.inventory.get("plugin_contracts").cloned() else {
        return Ok(Some(
            cowboy_plugin_sdk::PluginCompatibilityProblem::capability_inventory_unavailable(),
        ));
    };
    let contracts = match serde_json::from_value::<cowboy_plugin_sdk::PluginContractInventory>(
        raw_contracts,
    ) {
        Ok(contracts) => contracts,
        Err(_) => {
            return Ok(Some(cowboy_plugin_sdk::PluginCompatibilityProblem::new(
                cowboy_plugin_sdk::PluginCompatibilityCode::CapabilityInventoryInvalid,
                "Cowboy Machine reported an invalid Plugin capability inventory. Update Cowboy Machine before installing or upgrading Plugins.",
            )));
        }
    };
    let target = serde_json::from_value::<cowboy_provider_sdk::PlatformTarget>(serde_json::json!({
        "os": machine.platform,
        "architecture": machine.architecture,
    }))
    .map_err(|error| format!("Machine Plugin platform is invalid: {error}"))?;
    let (_, requirements) = crate::plugin_catalog::desired_plugin_compatibility(desired)
        .map_err(|error| format!("Catalog Plugin release is invalid: {error}"))?;
    Ok(contracts.compatibility_problem(
        &requirements,
        &desired.release.plugin_id,
        &desired.release.plugin_version,
        &desired.release.supported_platforms,
        &target,
    ))
}

pub(super) async fn agent_plugin_install_compatibility(
    state: &AppState,
    machine_id: &str,
    desired: &crate::machine_protocol::DesiredPlugin,
) -> Result<Option<cowboy_provider_sdk::ProviderCompatibilityProblem>, String> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(|| "Provider lifecycle requires persistence".to_owned())?;
    let machine = store
        .list_machines()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|machine| machine.id == machine_id && !machine.revoked)
        .ok_or_else(|| format!("unknown or revoked Machine {machine_id:?}"))?;
    let Some(raw_contracts) = machine.inventory.get("provider_contracts").cloned() else {
        return Ok(Some(
            cowboy_provider_sdk::ProviderCompatibilityProblem::capability_inventory_unavailable(),
        ));
    };
    let contracts = match serde_json::from_value::<cowboy_provider_sdk::ProviderContractInventory>(
        raw_contracts,
    ) {
        Ok(contracts) => contracts,
        Err(_) => {
            return Ok(Some(
                cowboy_provider_sdk::ProviderCompatibilityProblem::new(
                    cowboy_provider_sdk::ProviderCompatibilityCode::CapabilityInventoryInvalid,
                    "Cowboy Machine reported an invalid Provider capability inventory. Update Cowboy Machine before installing or upgrading Providers.",
                ),
            ));
        }
    };
    let target = serde_json::from_value::<cowboy_provider_sdk::PlatformTarget>(serde_json::json!({
        "os": machine.platform,
        "architecture": machine.architecture,
    }))
    .map_err(|error| format!("Machine Provider platform is invalid: {error}"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&desired.package_base64)
        .map_err(|error| format!("Catalog Provider package is invalid: {error}"))?;
    let plugin_package = desired
        .release
        .validate_bytes(&bytes)
        .map_err(|error| format!("Catalog Plugin release is invalid: {error}"))?;
    let package = plugin_package
        .agent_provider()
        .ok_or_else(|| "Plugin is not an Agent Provider".to_owned())?;
    let binding = desired
        .release
        .agent_provider_binding(&plugin_package)
        .map_err(|error| format!("Agent Plugin binding is invalid: {error}"))?;
    Ok(contracts.compatibility_problem(package, &binding, &target))
}

pub(super) fn plugin_compatibility_response(
    problem: cowboy_plugin_sdk::PluginCompatibilityProblem,
) -> Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "error": "plugin_incompatible",
            "code": problem.code,
            "detail": problem.detail,
        })),
    )
        .into_response()
}

pub(super) async fn current_machine_plugin(
    state: &Arc<AppState>,
    machine_id: &str,
    provider_id: &str,
) -> Result<crate::machine_protocol::PluginInventory, String> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(|| "Provider lifecycle requires persistence".to_owned())?;
    let machines = store
        .list_machines()
        .await
        .map_err(|error| error.to_string())?;
    active_machine_plugin(&machines, machine_id, provider_id)
}

/// The active installation of `provider_id` among already loaded Machine
/// records, so one pass over many sessions reads the Machines once.
pub(super) fn active_machine_plugin(
    machines: &[crate::store::MachineRecord],
    machine_id: &str,
    provider_id: &str,
) -> Result<crate::machine_protocol::PluginInventory, String> {
    let machine = machines
        .iter()
        .find(|machine| machine.id == machine_id && !machine.revoked)
        .ok_or_else(|| format!("unknown or revoked Machine {machine_id:?}"))?;
    let providers: Vec<crate::machine_protocol::PluginInventory> = machine
        .inventory
        .get("plugins")
        .or_else(|| machine.inventory.get("providers"))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    providers
        .into_iter()
        .find(|provider| {
            provider.plugin_id == provider_id
                && provider.state == crate::machine_protocol::PluginInstallationState::Active
        })
        .ok_or_else(|| {
            format!("Provider {provider_id:?} is not installed on Machine {machine_id:?}")
        })
}

/// Refuse a client-initiated lifecycle change on a Machine whose Plugins are
/// declared Service-side. The UI already hides these actions there; this is the
/// authority behind that, so an old client or a direct call cannot fork the
/// declared membership. Convergence itself does not pass through here: it runs
/// under the host grant on the private Operator endpoint.
pub(super) fn service_managed_refusal(state: &AppState, machine_id: &str) -> Option<Response> {
    state
        .desired_machine_plugins
        .current()
        .value
        .manages(machine_id)
        .then(|| {
            (
                StatusCode::CONFLICT,
                "This Machine's Plugins are declared Service-side; change the declaration instead of installing or removing from a client.",
            )
                .into_response()
        })
}

pub(super) async fn preview_machine_plugin_uninstall(
    state: Arc<AppState>,
    machine_id: String,
    provider_id: String,
    actor: crate::plugin_operation::Actor,
) -> Response {
    if state
        .plugin_lifecycle_fences
        .read()
        .contains_key(&(machine_id.clone(), provider_id.clone()))
    {
        return (
            StatusCode::CONFLICT,
            "Plugin lifecycle is changing or requires reconciliation",
        )
            .into_response();
    }
    let installed = match current_machine_plugin(&state, &machine_id, &provider_id).await {
        Ok(installed) => installed,
        Err(error) => return (StatusCode::CONFLICT, error).into_response(),
    };
    let mut affected: Vec<_> = state
        .hub
        .session_list()
        .into_iter()
        .filter(|session| {
            installed.plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider
                && session.machine_id == machine_id
                && session.provider == provider_id
        })
        .collect();
    affected.sort_by(|left, right| left.id.cmp(&right.id));
    let session_ids: Vec<_> = affected.iter().map(|session| session.id.clone()).collect();
    let active_session_ids: Vec<_> = affected
        .iter()
        .filter(|session| provider_session_has_active_turn(session.status))
        .map(|session| session.id.clone())
        .collect();
    let timestamp = now_ms();
    let purge_after_ms = timestamp.saturating_add(PROVIDER_UNINSTALL_RETENTION_MS);
    let expires_at_ms = timestamp.saturating_add(PROVIDER_UNINSTALL_PLAN_TTL_MS);
    let plan_id = match random_machine_token() {
        Ok(value) => value,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    let mut plans = state.plugin_uninstall_plans.lock();
    plans.retain(|_, plan| plan.expires_at_ms >= timestamp);
    if plans.len() >= 256 {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "uninstall preview budget exceeded",
        )
            .into_response();
    }
    plans.insert(
        plan_id.clone(),
        PluginUninstallPlan {
            actor,
            machine_id: machine_id.clone(),
            plugin_id: provider_id.clone(),
            plugin_version: installed.plugin_version.clone(),
            generation_digest: installed.generation_digest.clone(),
            installation_revision: installed.installation_revision.clone(),
            contract_fingerprint: installed.contract_fingerprint.clone(),
            session_ids,
            active_session_ids: active_session_ids.clone(),
            purge_after_ms,
            expires_at_ms,
        },
    );
    Json(serde_json::json!({
        "plan_id": plan_id,
        "machine_id": machine_id,
        "plugin_id": provider_id,
        "plugin_version": installed.plugin_version,
        "generation_digest": installed.generation_digest,
        "affected_sessions": affected,
        "active_session_ids": active_session_ids,
        "purge_after_ms": purge_after_ms,
        "expires_at_ms": expires_at_ms,
        "warning": if installed.plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider { "Uninstalling this Provider removes it from this Machine and soft-deletes every affected session. Session data remains recoverable only until the stated purge deadline." } else { "Uninstalling this Plugin disables its capability on this Machine. Retained generations and private activation policy are not deleted; in-flight bounded operations may finish." },
    }))
    .into_response()
}
