//! Provider handlers: catalog, authentication flows and envelopes, usage
//! sources and generation resolution.

use super::*;

pub async fn api_providers(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let providers = state.provider_catalog.entries();
    let authentication_executors = connected_provider_authentication_executors(
        &state,
        machine_auth_discovery_requested(&headers),
    )
    .await;
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::VARY, "x-cowboy-auth-machine"),
        ],
        Json(serde_json::json!({
            "providers": providers,
            "authentications": state.provider_auth.statuses(),
            "authentication_executors": authentication_executors,
        })),
    )
        .into_response()
}

pub async fn api_provider_catalog_refresh(State(state): State<Arc<AppState>>) -> Response {
    match state
        .plugin_catalog
        .refresh_with_runtime(&state.plugin_storage)
        .await
    {
        Ok(_) => match state.provider_catalog.refresh_external() {
            Ok(count) => {
                adopt_shared_provider_authentication(&state).await;
                Json(serde_json::json!({ "external_releases": count })).into_response()
            }
            Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        },
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

pub async fn api_provider_auth_commit(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Json(request): Json<ProviderAuthCommitRequest>,
) -> Response {
    if crate::store::provider_accounts::legacy_cards::retired_provider(&provider_id) {
        return (
            StatusCode::BAD_REQUEST,
            provider_accounts::retirement::LOGIN_RETIRED,
        )
            .into_response();
    }
    let Some((_provider_version, _generation_digest, package)) =
        state.provider_catalog.latest_package(&provider_id)
    else {
        return (
            StatusCode::BAD_REQUEST,
            "Provider is known, but no signed runtime release is published in the Catalog",
        )
            .into_response();
    };
    let bundle = crate::machine_protocol::PortableCredentialBundle {
        portable_schema: package.manifest.authentication.portable_schema.clone(),
        method_id: request.method,
        values: request.values,
    };
    let packages = state
        .provider_catalog
        .packages_for_authentication_scope(&package.manifest.authentication.portable_schema);
    let statuses = match if packages.len() == 1 {
        state
            .provider_auth
            .commit(
                &packages[0],
                &bundle,
                request.account_label,
                request.expected_generation,
            )
            .map(|status| vec![status])
    } else {
        state.provider_auth.commit_shared(
            &packages,
            &bundle,
            request.account_label,
            &provider_id,
            request.expected_generation,
        )
    } {
        Ok(statuses) => statuses,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let (statuses, replicas) = distribute_and_mark_provider_auth(&state, statuses).await;
    recover_failed_provider_sessions(&state, &statuses, &replicas);
    usage_credentials_changed(&state, &statuses);
    let Some(status) = statuses
        .iter()
        .find(|status| status.provider_id == provider_id)
        .cloned()
    else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "shared Provider authentication commit omitted requested Provider",
        )
            .into_response();
    };
    let replicas_succeeded: usize = replicas.values().map(|value| value.succeeded).sum();
    let replicas_failed: usize = replicas.values().map(|value| value.failed).sum();
    let replicas_pending: usize = replicas.values().map(|value| value.pending).sum();
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "authentication": status,
            "authentications": statuses,
            "replicas": replicas,
            "replicas_succeeded": replicas_succeeded,
            "replicas_failed": replicas_failed,
            "replicas_pending": replicas_pending,
        })),
    )
        .into_response()
}

pub async fn api_provider_auth_logout(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
) -> Response {
    let scope = match state.provider_auth.status(&provider_id) {
        Some(status) => status.authentication_scope,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "Provider is not authenticated at Cowboy Service scope",
            )
                .into_response();
        }
    };
    let mut provider_ids = state
        .provider_catalog
        .provider_ids_for_authentication_scope(&scope);
    if !provider_ids.contains(&provider_id) {
        provider_ids.push(provider_id.clone());
    }
    let statuses = match state.provider_auth.logout_scope(&provider_ids) {
        Ok(statuses) => statuses,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let (statuses, replicas) = distribute_and_mark_provider_auth(&state, statuses).await;
    let Some(status) = statuses
        .iter()
        .find(|status| status.provider_id == provider_id)
        .cloned()
    else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "shared Provider authentication logout omitted requested Provider",
        )
            .into_response();
    };
    let replicas_succeeded: usize = replicas.values().map(|value| value.succeeded).sum();
    let replicas_failed: usize = replicas.values().map(|value| value.failed).sum();
    let replicas_pending: usize = replicas.values().map(|value| value.pending).sum();
    Json(serde_json::json!({
        "authentication": status,
        "authentications": statuses,
        "replicas": replicas,
        "replicas_succeeded": replicas_succeeded,
        "replicas_failed": replicas_failed,
        "replicas_pending": replicas_pending,
    }))
    .into_response()
}

/// Why the default account cannot sign in with `method`, if it cannot.
fn default_login_refusal(
    package: &cowboy_provider_sdk::ProviderPackage,
    method: &str,
) -> Option<&'static str> {
    if crate::provider_account::named_only(&package.manifest.id) {
        return Some("named_account_required: this Provider signs in only to named accounts");
    }
    if crate::store::provider_accounts::legacy_cards::retired_provider(&package.manifest.id) {
        return Some(provider_accounts::retirement::LOGIN_RETIRED);
    }
    (!crate::provider_account::portable_contract::contract(package)
        .methods
        .iter()
        .any(|candidate| candidate.id == method))
    .then_some("authentication method is not available for the default account")
}

pub async fn api_provider_auth_start(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(provider_id): Path<String>,
    Json(request): Json<ProviderAuthStartRequest>,
) -> Response {
    let Some(package) = state.provider_catalog.package(
        &provider_id,
        &request.provider_version,
        &request.generation_digest,
    ) else {
        return (
            StatusCode::BAD_REQUEST,
            "Provider release is not in the Catalog",
        )
            .into_response();
    };
    if let Some(refusal) = default_login_refusal(&package, &request.method) {
        return (StatusCode::BAD_REQUEST, refusal).into_response();
    }
    let timestamp = now_ms();
    let reconciliation = {
        let mut executors = state.provider_auth_executors.lock();
        reconcile_provider_auth_executors(
            &mut executors,
            &provider_id,
            timestamp,
            &state.machine_control,
        )
    };
    let active_request_id = state
        .provider_auth
        .active_authentication_request(&provider_id);
    let resumable = reconciliation.resumable(active_request_id.as_deref());
    for (expired_provider, expired_request) in reconciliation.expired {
        state
            .provider_auth
            .cancel_authentication(&expired_provider, &expired_request);
    }
    if let Some((request_id, executor)) = resumable {
        return resumed_provider_authentication_response(
            request_id,
            &executor,
            &request,
            &auth.principal.user_id,
        );
    }
    let candidates = state.machine_control.connected_machine_ids();
    let mut executor = None;
    for machine_id in candidates {
        if request
            .machine_id
            .as_ref()
            .is_some_and(|id| id != &machine_id)
        {
            continue;
        }
        if !state.supervisor.runtime_allowed(&provider_id, &machine_id) {
            continue;
        }
        if current_machine_plugin(&state, &machine_id, &provider_id)
            .await
            .is_ok_and(|installed| {
                installed.plugin_version == request.provider_version
                    && installed.generation_digest == request.generation_digest
            })
        {
            executor = Some(machine_id);
            break;
        }
    }
    let Some(machine_id) = executor else {
        return (
            StatusCode::CONFLICT,
            "No permitted connected Machine has this exact Agent Plugin release installed for temporary authentication",
        )
            .into_response();
    };
    let connection = match state.machine_control.operation_connection(&machine_id) {
        Ok(connection) => connection,
        Err(error) => return (StatusCode::CONFLICT, error).into_response(),
    };
    let request_id = machine_request_id("provider-login");
    let expected_generation = state
        .provider_auth
        .status(&provider_id)
        .map_or(0, |status| status.auth_generation);
    let timestamp = now_ms();
    let expires_at_ms = timestamp.saturating_add(15 * 60 * 1_000);
    let mut executors = state.provider_auth_executors.lock();
    let reconciliation = reconcile_provider_auth_executors(
        &mut executors,
        &provider_id,
        timestamp,
        &state.machine_control,
    );
    let active_request_id = state
        .provider_auth
        .active_authentication_request(&provider_id);
    if let Some((active_request_id, active_executor)) =
        reconciliation.resumable(active_request_id.as_deref())
    {
        drop(executors);
        for (expired_provider, expired_request) in reconciliation.expired {
            state
                .provider_auth
                .cancel_authentication(&expired_provider, &expired_request);
        }
        return resumed_provider_authentication_response(
            active_request_id,
            &active_executor,
            &request,
            &auth.principal.user_id,
        );
    }
    executors.insert(
        request_id.clone(),
        ProviderAuthExecutor {
            native_login: None,
            named_login: None,
            cancel_requested: false,
            owner_user_id: auth.principal.user_id.clone(),
            machine_id: machine_id.clone(),
            account_id: Default::default(),
            connection: connection.clone(),
            provider_id: provider_id.clone(),
            provider_version: request.provider_version.clone(),
            generation_digest: request.generation_digest.clone(),
            auth_contract_fingerprint: package
                .manifest
                .compatibility
                .auth_contract_fingerprint
                .clone(),
            auth_method: request.method.clone(),
            expected_generation,
            promotion_started: false,
            expires_at_ms,
        },
    );
    for (expired_provider, expired_request) in reconciliation.expired {
        state
            .provider_auth
            .cancel_authentication(&expired_provider, &expired_request);
    }
    if let Err(error) = state
        .provider_auth
        .begin_authentication(&package, &request_id)
    {
        executors.remove(&request_id);
        return (StatusCode::CONFLICT, error.to_string()).into_response();
    }
    if let Err(error) = state.machine_control.send_on_connection(
        &connection,
        crate::machine_protocol::MachineCommand::BeginLogin {
            request_id: request_id.clone(),
            account_id: Default::default(),
            provider: provider_id.clone(),
            auth_method: Some(request.method.clone()),
        },
    ) {
        executors.remove(&request_id);
        state
            .provider_auth
            .cancel_authentication(&provider_id, &request_id);
        return (StatusCode::CONFLICT, error).into_response();
    }
    // Publish the executor, transient state and BeginLogin under one lock.
    // Cancellation must never overtake the initial command enqueue.
    drop(executors);
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "request_id": request_id,
            "expires_at_ms": expires_at_ms,
            "method": request.method,
            "machine_id": machine_id,
            "resumed": false,
        })),
    )
        .into_response()
}

pub async fn api_provider_auth_events(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((provider_id, request_id)): Path<(String, String)>,
) -> Response {
    let executor = state
        .provider_auth_executors
        .lock()
        .get(&request_id)
        .cloned();
    let Some(executor) =
        executor.filter(|value| value.owned_by(&auth.principal.user_id, &provider_id))
    else {
        return provider_accounts::login::native_events(
            &state.accounts,
            &auth.principal.user_id,
            &provider_id,
            &request_id,
            None,
        )
        .await;
    };
    if executor.native_login.is_some() {
        return provider_accounts::login::native_events(
            &state.accounts,
            &auth.principal.user_id,
            &provider_id,
            &request_id,
            Some(&executor),
        )
        .await;
    }
    if !executor.active(&state.machine_control, now_ms()) {
        if let Some(active) = state.provider_auth_executors.lock().get_mut(&request_id) {
            active.cancel_requested = true;
        }
        let cleanup =
            provider_accounts::login::cancel(&state.accounts, &executor, &request_id).await;
        if let Err(error) = cleanup {
            return (StatusCode::SERVICE_UNAVAILABLE, error).into_response();
        }
        if let Err(error) = provider_accounts::login::dispatch_cancel(
            &state.machine_control,
            &state.provider_auth_executors,
            &executor,
            &request_id,
        )
        .await
        {
            return (StatusCode::SERVICE_UNAVAILABLE, error).into_response();
        }
        return (
            StatusCode::NOT_FOUND,
            "authentication request expired or its Machine disconnected; start a new authorization",
        )
            .into_response();
    }
    if executor.named_login.is_some()
        && executor.promotion_started
        && let Err(error) =
            provider_accounts::login::recover_pending(&state.accounts, &executor, &request_id).await
    {
        return (StatusCode::SERVICE_UNAVAILABLE, error).into_response();
    }
    let events: Vec<_> = state
        .machine_control
        .events(&executor.machine_id)
        .into_iter()
        .filter(|event| match event {
            crate::machine_protocol::MachineEvent::LoginChallenge { request_id: id, .. }
            | crate::machine_protocol::MachineEvent::LoginState { request_id: id, .. }
            | crate::machine_protocol::MachineEvent::CommandResult { request_id: id, .. } => {
                id == &request_id
            }
            _ => false,
        })
        .collect();
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({ "request_id": request_id, "events": events })),
    )
        .into_response()
}

pub async fn api_provider_auth_submit(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((provider_id, request_id)): Path<(String, String)>,
    Json(request): Json<ProviderAuthSubmitRequest>,
) -> Response {
    submit_provider_auth_code(
        &state.provider_auth_executors,
        &state.machine_control,
        &auth.principal.user_id,
        &provider_id,
        request_id,
        request.code,
    )
}

pub async fn api_provider_auth_cancel(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((provider_id, request_id)): Path<(String, String)>,
) -> Response {
    cancel_provider_auth_request(&state, &auth.principal.user_id, &provider_id, request_id).await
}

pub fn provider_auth_recovery_generations(
    session: &crate::core::SessionMeta,
    status: &crate::provider_service::ProviderAuthenticationStatus,
    synchronized_generations: &BTreeMap<String, u64>,
    pinned_auth_contract_fingerprint: &str,
) -> Option<(u64, u64)> {
    let expected_generation = session.provider_auth_generation?;
    (status.provider_id == session.provider
        && status.account_id == session.provider_account_id
        && status.authentication_state
            == crate::provider_service::ServiceAuthenticationState::Ready
        && status.auth_generation > expected_generation
        && synchronized_generations.get(&session.machine_id) == Some(&status.auth_generation)
        && status.auth_contract_fingerprint == pinned_auth_contract_fingerprint)
        .then_some((expected_generation, status.auth_generation))
}

#[allow(clippy::too_many_arguments)]
pub async fn accept_provider_auth_refresh_candidate(
    state: &Arc<AppState>,
    machine_id: &str,
    installed: Option<crate::machine_protocol::PluginInventory>,
    request_id: &str,
    provider_id: &str,
    account_id: &crate::provider_account::ProviderAccountId,
    native_identity: Option<&crate::provider_account::NativeIdentity>,
    expected_generation: u64,
    provider_version: &str,
    generation_digest: &str,
    auth_contract_fingerprint: &str,
    portable_schema: &str,
    auth_method: &str,
    values: BTreeMap<String, String>,
) -> Result<(), String> {
    let installed = installed
        .filter(|installed| {
            installed.plugin_id == provider_id
                && installed.plugin_version == provider_version
                && installed.generation_digest == generation_digest
                && installed.state == crate::machine_protocol::PluginInstallationState::Active
                && (!account_id.is_default()
                    || (installed.auth_generation == Some(expected_generation)
                        && installed.replica_state
                            == crate::machine_protocol::ProviderReplicaState::Current))
        })
        .ok_or_else(|| {
            "credential refresh does not match the Machine's active Provider generation".to_owned()
        })?;
    let package = state
        .provider_catalog
        .package(provider_id, provider_version, generation_digest)
        .ok_or_else(|| {
            "credential refresh references an untrusted Agent Plugin release".to_owned()
        })?;
    if package.manifest.authentication.refresh
        != cowboy_provider_sdk::RefreshOwnership::CompareAndSwap
    {
        return Err("Provider does not allow Machine credential refresh".to_owned());
    }
    if package.manifest.compatibility.auth_contract_fingerprint != auth_contract_fingerprint
        || package.manifest.authentication.portable_schema != portable_schema
        || (account_id.is_default() && installed.auth_generation != Some(expected_generation))
    {
        return Err("credential refresh contract does not match the active Provider".to_owned());
    }
    let bundle = crate::machine_protocol::PortableCredentialBundle {
        portable_schema: portable_schema.to_owned(),
        method_id: auth_method.to_owned(),
        values,
    };
    let packages = state
        .provider_catalog
        .packages_for_authentication_scope(portable_schema);
    if let Some(routed) = provider_accounts::portable::route_refresh(
        &state.accounts,
        machine_id,
        account_id,
        &package,
        &bundle,
        expected_generation,
        native_identity,
    )
    .await
    {
        return routed;
    }
    let method = package
        .manifest
        .authentication
        .methods
        .iter()
        .find(|method| method.id == auth_method)
        .ok_or("unknown refresh method")?;
    // Default-account evidence only serves a redirected legacy rotation.
    refresh_identity_evidence(
        provider_id,
        account_id,
        method.native_identity.as_ref(),
        native_identity.filter(|_| !account_id.is_default()),
    )?;
    let scope = provider_auth_scope_for_machine(&state.accounts, machine_id, account_id).await?;
    let result = if account_id.is_default() {
        scope
            .compare_and_swap_refresh(&packages, &bundle, provider_id, expected_generation)
            .map_err(|error| error.to_string())?
    } else {
        scope
            .refresh_current_grant(
                state.store.as_ref().ok_or("refresh requires persistence")?,
                machine_id,
                &package,
                &bundle,
                expected_generation,
                native_identity.ok_or("named refresh requires native identity")?,
            )
            .await
            .map_err(|error| error.to_string())?
    };
    if !account_id.is_default() {
        return reconcile_account_refresh(state, machine_id, provider_id, account_id, result).await;
    }
    match result {
        crate::provider_service::ProviderAuthRefreshResult::Stale { current_generation } => {
            return provider_accounts::portable::forward_or_restore(
                (request_id, current_generation),
                &state.accounts,
                machine_id,
                &package,
                &bundle,
                expected_generation,
                native_identity,
            )
            .await;
        }
        crate::provider_service::ProviderAuthRefreshResult::Unchanged(_) => {
            tracing::debug!(
                %request_id,
                %machine_id,
                %provider_id,
                expected_generation,
                "ignoring unchanged Provider credential refresh"
            );
        }
        crate::provider_service::ProviderAuthRefreshResult::Updated(statuses) => {
            let next_generation = statuses
                .iter()
                .map(|status| status.auth_generation)
                .max()
                .unwrap_or(expected_generation);
            tracing::info!(
                %request_id,
                %machine_id,
                %provider_id,
                expected_generation,
                next_generation,
                "promoted Machine-refreshed Provider credentials"
            );
            let (statuses, replicas) = distribute_and_mark_provider_auth(state, statuses).await;
            recover_failed_provider_sessions(state, &statuses, &replicas);
            usage_credentials_changed(state, &statuses);
        }
    }
    Ok(())
}

/// `(gateway slot, agent)` pairs for Providers active on one Machine that run
/// a shared provider gateway as a session sidecar. Such a gateway reports its
/// own component slot as producer while labelling events with the consuming
/// Provider's agent, e.g. DeepSeek Harness over the claude-deepseek gateway.
pub fn provider_usage_shared_gateways(
    current_plugins: &[crate::machine_protocol::PluginInventory],
    runtime: &crate::plugin_runtime::PluginRuntime,
    catalog: &crate::provider_catalog::ProviderCatalog,
) -> BTreeSet<(String, String)> {
    let mut pairs = BTreeSet::new();
    for plugin in current_plugins
        .iter()
        .filter(|plugin| plugin.state == crate::machine_protocol::PluginInstallationState::Active)
    {
        let Some(agent) = runtime
            .exact_host(
                &plugin.plugin_id,
                &plugin.plugin_version,
                &plugin.generation_digest,
            )
            .and_then(|host| host.adapter_slot.clone())
        else {
            continue;
        };
        let Some(package) = catalog.package(
            &plugin.plugin_id,
            &plugin.plugin_version,
            &plugin.generation_digest,
        ) else {
            continue;
        };
        for sidecar in &package.manifest.runtime.sidecars {
            if sidecar.component.kind == cowboy_provider_sdk::PrivateComponentKind::ProviderGateway
            {
                pairs.insert((sidecar.component.slot.clone(), agent.clone()));
            }
        }
    }
    pairs
}

pub fn provider_usage_source_is_authorized(
    current_plugins: &[crate::machine_protocol::PluginInventory],
    runtime: &crate::plugin_runtime::PluginRuntime,
    shared_gateways: &BTreeSet<(String, String)>,
    producer_id: &str,
    provider: &str,
    agent: &str,
) -> bool {
    let producer_agent = current_plugins
        .iter()
        .find(|plugin| {
            plugin.plugin_id == producer_id
                && plugin.state == crate::machine_protocol::PluginInstallationState::Active
        })
        .and_then(|producer| {
            runtime.exact_host(
                producer_id,
                &producer.plugin_version,
                &producer.generation_digest,
            )
        })
        .and_then(|host| host.adapter_slot.clone());
    (producer_agent.as_deref() == Some(agent)
        || shared_gateways.contains(&(producer_id.to_owned(), agent.to_owned())))
        && runtime.default_hosts().into_iter().any(|host| {
            host.usage.as_ref().is_some_and(|usage| {
                usage.activity
                    && usage.account == provider
                    && (usage.activity_agents.is_empty()
                        || usage.activity_agents.iter().any(|entry| entry.id == agent))
            })
        })
}

#[cfg(test)]
mod default_login_tests {
    use super::default_login_refusal;

    fn package(source: &str) -> cowboy_provider_sdk::ProviderPackage {
        let source: cowboy_provider_sdk::StandardProviderSource =
            serde_json::from_str(source).unwrap();
        cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap()
    }

    #[test]
    fn antigravity_default_account_never_signs_in() {
        let antigravity = package(include_str!(
            "../../../../plugins/antigravity/provider.json"
        ));
        for method in ["google-account", "google-subscription"] {
            assert!(
                default_login_refusal(&antigravity, method)
                    .is_some_and(|refusal| refusal.starts_with("named_account_required"))
            );
        }
    }

    #[test]
    fn a_retired_legacy_card_never_signs_in_at_service_scope() {
        for (source, method) in [
            (
                include_str!("../../../../plugins/grok/provider.json"),
                "xai-account",
            ),
            (
                include_str!("../../../../plugins/codex/provider.json"),
                "chatgpt",
            ),
            (
                include_str!("../../../../plugins/claude-code/provider.json"),
                "claude-account",
            ),
            (
                include_str!("../../../../plugins/claude-deepseek/provider.json"),
                "api-key",
            ),
        ] {
            assert_eq!(
                default_login_refusal(&package(source), method),
                Some(super::provider_accounts::retirement::LOGIN_RETIRED)
            );
        }
    }
}
