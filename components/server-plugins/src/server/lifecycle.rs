//! Plugin install compatibility, uninstall preview, and Service-managed refusal.

use super::*;

pub async fn plugin_install_compatibility(
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

pub async fn agent_plugin_install_compatibility(
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

pub fn plugin_compatibility_response(
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

pub async fn current_machine_plugin(
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
pub fn active_machine_plugin(
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
pub fn service_managed_refusal(state: &AppState, machine_id: &str) -> Option<Response> {
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

pub async fn preview_machine_plugin_uninstall(
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
