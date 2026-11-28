//! Copy user configuration, never credentials, native history or runtime state.
use super::*;
use crate::core::SessionMeta;
use crate::core::settings_keys;
use cowboy_provider_sdk::{ConfigurationUiContract, PermissionBehavior};

fn configuration(
    state: &AppState,
    source: &SessionMeta,
    target: &SessionMeta,
    selection: &NextAgent,
) -> Result<BTreeMap<String, serde_json::Value>, String> {
    let package = state
        .provider_catalog
        .package(
            &target.provider,
            &target.provider_version,
            &target.provider_generation_digest,
        )
        .ok_or("Target Provider configuration is unavailable")?;
    let preferences = state.hub.config_preferences(&source.id).unwrap_or_default();
    let observed = state.hub.config_options(&source.id);
    let mut values = compatible_configuration(
        source,
        target,
        &preferences,
        observed.as_ref(),
        &package.manifest.configuration,
    )?;
    let protected_permission = permission_control(target).and_then(|(key, unrestricted)| {
        values
            .get(key)
            .filter(|value| value.as_str() != Some(unrestricted))
            .cloned()
            .map(|value| (key, value))
    });
    // An explicit target preset wins over inherited model/reasoning values.
    values.extend(preset_values(
        &package.manifest.configuration,
        target,
        selection,
    )?);
    // Model/reasoning presets can include a default permission mode. Keeping a
    // restrictive source permission takes precedence over that incidental default.
    if let Some((key, value)) = protected_permission {
        values.insert(key.into(), value);
    }
    Ok(values)
}

/// The signed values of the selection's target preset, if it names one.
fn preset_values(
    contract: &ConfigurationUiContract,
    target: &SessionMeta,
    selection: &NextAgent,
) -> Result<BTreeMap<String, serde_json::Value>, String> {
    let Some(preset) = selection
        .preset
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        return Ok(BTreeMap::new());
    };
    let id = preset
        .strip_prefix(&format!("{}/", target.provider))
        .ok_or("Preset belongs to another Provider")?;
    let preset = contract
        .presets
        .iter()
        .find(|preset| preset.id == id)
        .ok_or("Target preset is unavailable")?;
    Ok(preset
        .values
        .iter()
        .map(|(key, value)| (key.clone(), serde_json::json!(value)))
        .collect())
}

fn effective_runtime_machine(selection: &NextAgent) -> &str {
    selection
        .runtime_machine_id
        .as_deref()
        .unwrap_or(&selection.machine_id)
}

fn target_preview_meta(
    source: &SessionMeta,
    selection: &NextAgent,
    generation: &ResolvedProviderGeneration,
) -> SessionMeta {
    let mut target = source.clone();
    target.machine_id = effective_runtime_machine(selection).to_owned();
    target.provider.clone_from(&selection.provider);
    target.provider_account_id = selection.provider_account_id.clone();
    target.provider_version.clone_from(&generation.version);
    target
        .provider_generation_digest
        .clone_from(&generation.digest);
    target.provider_auth_generation = generation.auth_generation;
    target
}

/// The options a copy for `selection` would offer before it is created:
/// the target generation's signed defaults, then what creation saves
/// (inherited settings and the preset, or the preset alone).
pub async fn target_options(
    state: &AppState,
    auth: &AuthenticatedProductRequest,
    source: &SessionMeta,
    selection: &NextAgent,
) -> Result<serde_json::Value, String> {
    let generation = target_generation(state, auth, selection).await?;
    let mut target = target_preview_meta(source, selection, &generation);
    let mut default_val = serde_json::to_value(&generation.behavior.default_preferences)
        .unwrap_or_else(|_| serde_json::json!({}));
    crate::managed_config::strip_opt_in_defaults(&mut default_val);
    let mut preferences = match default_val {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    if selection.inherit_settings
        && crate::deepseek_cache::supported_behavior(&generation.behavior.configuration)
        && let Some(source_prefs) = state.hub.config_preferences(&source.id)
        && let Some(consent) =
            crate::managed_config::effective_cache_protection_consent(&source_prefs)
    {
        preferences.insert(
            crate::managed_config::CACHE_PROTECTION_ID.to_owned(),
            serde_json::Value::Bool(consent),
        );
        preferences.insert(
            crate::managed_config::CACHE_PROTECTION_OWNER_ID.to_owned(),
            serde_json::Value::Bool(true),
        );
    }
    target.provider_behavior = Some(generation.behavior);
    let chosen = if selection.inherit_settings {
        configuration(state, source, &target, selection)?
    } else {
        let package = state
            .provider_catalog
            .package(
                &target.provider,
                &target.provider_version,
                &target.provider_generation_digest,
            )
            .ok_or("Target Provider configuration is unavailable")?;
        preset_values(&package.manifest.configuration, &target, selection)?
    };
    preferences.extend(chosen);
    super::super::session_configuration::target_preview(
        state,
        &target,
        &serde_json::Value::Object(preferences),
    )
    .ok_or_else(|| "Target Provider configuration is unavailable".into())
}

/// Read-only admission before reserving a session or preparing its worktree.
/// Creation still resolves the generation again; apply rechecks it after admission.
pub async fn preflight(
    state: &AppState,
    auth: &AuthenticatedProductRequest,
    source: &SessionMeta,
    selection: &NextAgent,
) -> Result<(), String> {
    if !selection.inherit_settings {
        return Ok(());
    }
    let generation = target_generation(state, auth, selection).await?;
    let mut target = source.clone();
    target.provider.clone_from(&selection.provider);
    target.provider_version = generation.version;
    target.provider_generation_digest = generation.digest;
    target.provider_behavior = Some(generation.behavior);
    configuration(state, source, &target, selection).map(|_| ())
}

/// The release and login generation that creation would admit for `selection`.
pub async fn target_generation(
    state: &AppState,
    auth: &AuthenticatedProductRequest,
    selection: &NextAgent,
) -> Result<ResolvedProviderGeneration, String> {
    let machine_id = effective_runtime_machine(selection);
    let store = state.store.as_ref().ok_or("Machine registry unavailable")?;
    let machine = store
        .list_machines()
        .await
        .map_err(|_| "Machine registry unavailable")?
        .into_iter()
        .find(|machine| machine.id == machine_id && !machine.revoked)
        .ok_or("Target Machine is unavailable")?;
    let inventory: Vec<crate::machine_protocol::PluginInventory> = serde_json::from_value(
        machine
            .inventory
            .get("plugins")
            .or_else(|| machine.inventory.get("providers"))
            .cloned()
            .unwrap_or_default(),
    )
    .map_err(|_| "Target Provider inventory is unavailable")?;
    let account = provider_accounts::resolve_for_create(
        &state.accounts,
        &auth.principal.user_id,
        &selection.provider,
        Some(&selection.provider_account_id),
    )
    .await
    .map_err(str::to_owned)?;
    provider_accounts::registration::generation(
        &state.accounts,
        &inventory,
        &selection.provider,
        &auth.principal.user_id,
        &account,
        machine_id,
    )
    .await
}

pub fn apply(
    state: &AppState,
    source: &SessionMeta,
    target: &SessionMeta,
    selection: &NextAgent,
) -> Result<(), String> {
    let values = configuration(state, source, target, selection)?;
    for (key, value) in values {
        state.hub.set_config_preference(&target.id, key, value)?;
    }
    state
        .hub
        .inherit_cache_protection_consent(&source.id, &target.id);
    let tools = super::super::agent_tools::inherited_settings(state, source, &target.provider);
    super::super::agent_tools::store_session(
        &state.hub,
        &target.id,
        Some(serde_json::json!(tools)),
    );
    state.hub.set_setting(
        format!(
            "{}{}",
            settings_keys::SESSION_PROVIDER_AUTO_UPDATE_PREFIX,
            target.id
        ),
        serde_json::json!(super::super::session_provider_updates::enabled(
            &state.hub, &source.id
        )),
    );
    super::rollover::inherit(state, &source.id, &target.id);
    let policy = super::super::send_delivery::policy(&state.hub, &source.id);
    state.hub.set_setting(
        format!(
            "{}{}",
            settings_keys::SESSION_DELIVERY_POLICY_PREFIX,
            target.id
        ),
        serde_json::json!(crate::send_delivery::DeliveryPolicy {
            mode: policy.mode,
            revision: 0
        }),
    );
    // Queue contents, Pause, pending operations and one-shot approvals are not settings.
    super::super::session_configuration::prepare(state, &target.id);
    Ok(())
}

fn permission_control(meta: &SessionMeta) -> Option<(&'static str, &'static str)> {
    match meta.provider_behavior.as_ref()?.permission {
        PermissionBehavior::AcpConfigFullAccessV1 => Some(("mode", "agent-full-access")),
        PermissionBehavior::AcpSessionModeBypassPermissionsV1 => {
            Some(("mode", "bypassPermissions"))
        }
        PermissionBehavior::AcpSessionModeYoloV1 => Some(("mode", "yolo")),
        PermissionBehavior::XaiSessionV1 => Some(("permission_mode", "always-approve")),
        PermissionBehavior::PortableV1 => None,
    }
}

fn compatible_configuration(
    source: &SessionMeta,
    target: &SessionMeta,
    preferences: &serde_json::Value,
    observed: Option<&serde_json::Value>,
    contract: &ConfigurationUiContract,
) -> Result<BTreeMap<String, serde_json::Value>, String> {
    let same_release = source.provider_version == target.provider_version
        && source.provider_generation_digest == target.provider_generation_digest;
    let mut candidates = BTreeMap::new();
    // Include active CLI selections even if Cowboy has no explicit override yet.
    for option in observed
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        if let (Some(id), Some(value)) = (option["id"].as_str(), option.get("currentValue")) {
            candidates.insert(id.to_owned(), value.clone());
        }
    }
    if let Some(preferences) = preferences.as_object() {
        candidates.extend(
            preferences
                .iter()
                .map(|(id, value)| (id.clone(), value.clone())),
        );
    }
    // Paid keepalive consent is inherited separately via verified provenance so
    // legacy defaults never enroll continuations and internal keys never leak.
    candidates.remove(crate::managed_config::CACHE_PROTECTION_ID);
    candidates.remove(crate::managed_config::CACHE_PROTECTION_OWNER_ID);
    let permission = permission_control(source);
    let same_permission_contract = permission.is_some() && permission == permission_control(target);
    // Never silently replace a restrictive mode with the destination's permissive
    // default. A different native release/provider is not proof it accepts this
    // runtime-only value; the user can explicitly opt into target defaults instead.
    if let Some((key, unrestricted)) = permission
        && let Some(value) = candidates.get(key)
        && value.as_str() != Some(unrestricted)
        && (source.provider != target.provider || !same_release || !same_permission_contract)
    {
        return Err("The current permission mode cannot be inherited by this Provider version. Keep the same Provider version, or turn off Inherit compatible settings to explicitly use the new agent's defaults.".into());
    }
    if source.provider != target.provider {
        return Ok(BTreeMap::new());
    }
    let Some(behavior) = target.provider_behavior.as_ref() else {
        return Ok(BTreeMap::new());
    };
    candidates.retain(|id, value| {
        if !value.is_string() && !value.is_boolean() {
            return false;
        }
        let default = behavior
            .default_preferences
            .get(id)
            .and_then(|value| serde_json::to_value(value).ok());
        let preset_value = contract.presets.iter().any(|preset| {
            preset
                .values
                .get(id)
                .is_some_and(|candidate| value.as_str() == Some(candidate))
        });
        let declared = default.is_some()
            || (same_permission_contract && permission.is_some_and(|(key, _)| key == id))
            || contract.options.iter().any(|option| option.id == *id)
            || contract
                .presets
                .iter()
                .any(|preset| preset.values.contains_key(id))
            || crate::managed_config::is_managed(&behavior.configuration, id);
        // Never forward opaque project/auth controls. Across releases only signed
        // target values are evidence of compatibility; no guessed permission mapping.
        declared && (same_release || default.as_ref() == Some(value) || preset_value)
    });
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_cross_accounts_but_opaque_controls_and_foreign_provider_values_do_not() {
        let hub = Hub::new();
        hub.create_local_session(
            "source".into(),
            "codex".into(),
            "/worktree".into(),
            "Task".into(),
            SessionOrigin::Web,
            false,
        );
        let mut source = hub.session_info("source").unwrap().meta;
        source.provider_version = "1".into();
        source.provider_generation_digest = "signed-1".into();
        source.provider_behavior = Some(crate::provider::legacy_behavior("codex"));
        let contract: ConfigurationUiContract = serde_json::from_value(json!({
            "schema_version":1,"presets":[{"id":"deep","name":"Deep","detail":"","values":{"model":"astra","reasoning_effort":"max"}}],
            "options":[]
        })).unwrap();
        let mut target = source.clone();
        target.id = "target".into();
        target.provider_account_id =
            serde_json::from_value(json!("11111111-2222-3333-4444-555555555555")).unwrap();
        let prefs = json!({"reasoning_effort":"high", "mode":"read-only", "credentials":"secret", "workspace_agent":"private"});
        let options = json!([{"id":"model","currentValue":"astra"}]);
        let inherited =
            compatible_configuration(&source, &target, &prefs, Some(&options), &contract).unwrap();
        assert_eq!(inherited.get("model"), Some(&json!("astra")));
        assert_eq!(inherited.get("reasoning_effort"), Some(&json!("high")));
        assert_eq!(inherited.get("mode"), Some(&json!("read-only")));
        assert!(!inherited.contains_key("credentials"));
        assert!(!inherited.contains_key("workspace_agent"));
        target.provider_generation_digest = "signed-2".into();
        assert!(
            compatible_configuration(&source, &target, &prefs, Some(&options), &contract).is_err()
        );
        let unrestricted = json!({"model":"astra", "mode":"agent-full-access"});
        let changed =
            compatible_configuration(&source, &target, &unrestricted, Some(&options), &contract)
                .unwrap();
        assert_eq!(changed.get("model"), Some(&json!("astra")));
        assert!(!changed.contains_key("mode"));
        target.provider = "claude-code".into();
        assert!(
            compatible_configuration(&source, &target, &prefs, Some(&options), &contract).is_err()
        );
        assert!(
            compatible_configuration(&source, &target, &unrestricted, Some(&options), &contract)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn observed_restrictive_permission_survives_account_change_without_static_options() {
        let hub = Hub::new();
        let contract: ConfigurationUiContract = serde_json::from_value(json!({
            "schema_version":1, "presets":[], "options":[]
        }))
        .unwrap();
        for (provider, key, value) in [
            ("codex", "mode", "read-only"),
            ("claude-code", "mode", "plan"),
            ("grok", "permission_mode", "default"),
        ] {
            hub.create_local_session(
                provider.into(),
                provider.into(),
                "/worktree".into(),
                "Task".into(),
                SessionOrigin::Web,
                false,
            );
            let mut source = hub.session_info(provider).unwrap().meta;
            source.provider_behavior = Some(crate::provider::legacy_behavior(provider));
            source.provider_version = "1".into();
            source.provider_generation_digest = "signed-1".into();
            let mut target = source.clone();
            target.provider_account_id =
                serde_json::from_value(json!("11111111-2222-3333-4444-555555555555")).unwrap();
            let options = json!([{"id":key,"currentValue":value}]);
            let values =
                compatible_configuration(&source, &target, &json!({}), Some(&options), &contract)
                    .unwrap();
            assert_eq!(values.get(key), Some(&json!(value)), "{provider}");
            target.provider_behavior = None;
            assert!(
                compatible_configuration(&source, &target, &json!({}), Some(&options), &contract)
                    .is_err()
            );
        }
    }

    #[test]
    fn target_preview_meta_uses_effective_target_runtime_machine() {
        let hub = Hub::new();
        hub.create_local_session(
            "source".into(),
            "codex".into(),
            "/worktree".into(),
            "Task".into(),
            SessionOrigin::Web,
            false,
        );
        let mut source = hub.session_info("source").unwrap().meta;
        source.machine_id = "hawk".into();
        let generation = ResolvedProviderGeneration {
            version: "0.2.42".into(),
            digest: "signed-ovh".into(),
            auth_generation: Some(1),
            named_carry: None,
            service_lineage: false,
            behavior: crate::provider::legacy_behavior("antigravity"),
        };
        let split_selection = NextAgent {
            provider: "antigravity".into(),
            provider_account_id: serde_json::from_value(json!(
                "5f5f8e08-c104-4982-be58-717f0be79f01"
            ))
            .unwrap(),
            machine_id: "hawk".into(),
            cwd: "columbus".into(),
            runtime_machine_id: Some("ovh".into()),
            preset: None,
            inherit_settings: true,
            config: serde_json::Map::new(),
        };
        let target = target_preview_meta(&source, &split_selection, &generation);
        assert_eq!(target.machine_id, "ovh");
        assert_eq!(target.provider, "antigravity");

        let local_selection = NextAgent {
            runtime_machine_id: None,
            machine_id: "falcon".into(),
            ..split_selection
        };
        assert_eq!(
            target_preview_meta(&source, &local_selection, &generation).machine_id,
            "falcon"
        );
    }
}
