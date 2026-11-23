//! Plugin catalog and call handlers, install compatibility, uninstall
//! preview, fences and generation references.

use super::*;

/// Whether a session holds one of its Machine's worker slots. Capacity bounds
/// live Agent processes: a starting session counts before its worker
/// registers, while a session whose worker exited (or was never revived after
/// a restart) holds no memory and must not keep the Machine full.
pub fn holds_worker_slot(router: &RuntimeRouter, session: &crate::core::SessionMeta) -> bool {
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
pub fn superseding_plugin(
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
pub fn referenced_generations(
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
pub fn referenced_auth_generations(
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

pub fn plugin_fence_state_for_session(
    hub: &Hub,
    fences: &PluginLifecycleFences,
    session_id: &str,
) -> Option<PluginFenceState> {
    let key = provider_fence_key_for_session(hub, session_id)?;
    fences.read().get(&key).copied()
}

pub fn provider_fence_key_for_session(hub: &Hub, session_id: &str) -> Option<(String, String)> {
    hub.session_list()
        .into_iter()
        .find(|session| session.id == session_id)
        .map(|session| (session.machine_id, session.provider))
}

pub async fn api_plugins(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
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

pub async fn api_plugin_call(
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
pub async fn api_builtin_plugin_call(
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

pub async fn api_plugin_catalog_refresh(State(state): State<Arc<AppState>>) -> Response {
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

pub async fn current_machine_plugin(
    state: &Arc<AppState>,
    machine_id: &str,
    provider_id: &str,
) -> Result<crate::machine_protocol::PluginInventory, String> {
    cowboy_server_auth::server::current_machine_plugin_from_store(
        state.store.as_ref(),
        machine_id,
        provider_id,
    )
    .await
}
