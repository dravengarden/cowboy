//! Provider authentication executors and distribution, account refresh,
//! and provider release upgrades and reloads for sessions.

use super::*;

#[derive(Clone)]
pub(super) struct ProviderAuthExecutor {
    pub(super) native_login: Option<provider_accounts::login::NativeLogin>,
    pub(super) named_login: Option<provider_accounts::login::NamedLogin>,
    pub(super) cancel_requested: bool,
    pub(super) owner_user_id: String,
    pub(super) account_id: crate::provider_account::ProviderAccountId,
    pub(super) machine_id: String,
    pub(super) connection: crate::machine_control::ConnectionToken,
    pub(super) provider_id: String,
    pub(super) provider_version: String,
    pub(super) generation_digest: String,
    pub(super) auth_contract_fingerprint: String,
    pub(super) auth_method: String,
    pub(super) expected_generation: u64,
    pub(super) promotion_started: bool,
    pub(super) expires_at_ms: i64,
}

impl ProviderAuthExecutor {
    pub(super) fn auth_scope<'a>(
        &'a self,
        state: &'a AppState,
    ) -> Option<&'a crate::provider_service::ProviderAuthService> {
        if self.native_login.is_some() {
            return None;
        }
        Some(
            self.named_login
                .as_ref()
                .map_or(&state.provider_auth, |login| &login.scope),
        )
    }
    pub(super) fn owned_by(&self, owner: &str, provider: &str) -> bool {
        self.owner_user_id == owner && self.provider_id == provider
    }

    // Called while holding the executor map lock. A failed validation/commit
    // must remain retryable; only durable credential ownership consumes it.
    pub(super) fn promote_once<T, E>(
        &mut self,
        commit: impl FnOnce(u64) -> Result<T, E>,
    ) -> Result<Option<T>, E> {
        if self.promotion_started {
            return Ok(None);
        }
        let committed = commit(self.expected_generation)?;
        self.promotion_started = true;
        Ok(Some(committed))
    }

    pub(super) fn active(&self, control: &MachineControl, timestamp: i64) -> bool {
        !self.cancel_requested
            && self.expires_at_ms > timestamp
            && control.is_current(&self.connection)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn accepts_candidate(
        &self,
        machine_id: &str,
        account_id: &crate::provider_account::ProviderAccountId,
        provider_id: &str,
        auth_method: &str,
        provider_version: &str,
        generation_digest: &str,
        auth_contract_fingerprint: &str,
    ) -> bool {
        self.machine_id == machine_id
            && self.account_id == *account_id
            && self.provider_id == provider_id
            && self.provider_version == provider_version
            && self.generation_digest == generation_digest
            && self.auth_contract_fingerprint == auth_contract_fingerprint
            && self.auth_method == auth_method
    }
}

pub(super) fn reconcile_provider_auth_executors(
    executors: &mut HashMap<String, ProviderAuthExecutor>,
    provider_id: &str,
    timestamp: i64,
    control: &MachineControl,
) -> ProviderAuthReconciliation {
    let mut expired = Vec::new();
    executors.retain(|request_id, executor| {
        if !executor.account_id.is_default() {
            return true;
        }
        let active = executor.active(control, timestamp);
        if !active {
            expired.push((executor.provider_id.clone(), request_id.clone()));
        }
        active
    });
    let active = executors
        .iter()
        .filter(|(_, executor)| {
            executor.account_id.is_default() && executor.provider_id == provider_id
        })
        .max_by_key(|(_, executor)| executor.expires_at_ms)
        .map(|(request_id, executor)| (request_id.clone(), executor.clone()));
    ProviderAuthReconciliation { expired, active }
}

pub(super) fn resumed_provider_authentication_response(
    request_id: String,
    executor: &ProviderAuthExecutor,
    request: &ProviderAuthStartRequest,
    owner_user_id: &str,
) -> Response {
    // Default credentials remain Service-wide, but the temporary native login
    // and its challenges belong only to the initiating product user.
    if executor.owner_user_id != owner_user_id {
        return (
            StatusCode::CONFLICT,
            "Provider authentication is already in progress",
        )
            .into_response();
    }
    if executor.auth_method != request.method
        || request
            .machine_id
            .as_ref()
            .is_some_and(|id| id != &executor.machine_id)
        || executor.provider_version != request.provider_version
        || executor.generation_digest != request.generation_digest
    {
        return (
            StatusCode::CONFLICT,
            "A different authentication flow is active; cancel it before starting another",
        )
            .into_response();
    }
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "request_id": request_id,
            "expires_at_ms": executor.expires_at_ms,
            "method": executor.auth_method,
            "machine_id": executor.machine_id,
            "resumed": true,
        })),
    )
        .into_response()
}

pub(super) async fn connected_provider_authentication_executors(
    state: &AppState,
    include_machine: bool,
) -> Vec<ProviderAuthenticationExecutorIdentity> {
    let executors: BTreeSet<_> = provider_installations(state, true)
        .await
        .into_iter()
        .map(
            |(machine_id, installed)| ProviderAuthenticationExecutorIdentity {
                machine_id: include_machine.then_some(machine_id),
                provider_id: installed.plugin_id,
                provider_version: installed.plugin_version,
                generation_digest: installed.generation_digest,
            },
        )
        .collect();
    executors.into_iter().collect()
}

/// Active, runtime-allowed Provider installations of a catalogued release on
/// enrolled, unrevoked Machines, as last reported in their inventory; only
/// currently connected Machines when `connected_only`.
pub(super) async fn provider_installations(
    state: &AppState,
    connected_only: bool,
) -> Vec<(String, crate::machine_protocol::PluginInventory)> {
    let Some(store) = state.store.as_ref() else {
        return Vec::new();
    };
    let Ok(machines) = store.list_machines().await else {
        return Vec::new();
    };
    let connected = state.machine_control.connected_machine_ids();
    let mut installations = Vec::new();
    for machine in machines
        .into_iter()
        .filter(|machine| !machine.revoked && (!connected_only || connected.contains(&machine.id)))
    {
        let Ok(providers) = serde_json::from_value::<Vec<crate::machine_protocol::PluginInventory>>(
            machine
                .inventory
                .get("plugins")
                .or_else(|| machine.inventory.get("providers"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        ) else {
            continue;
        };
        for installed in providers.into_iter().filter(|provider| {
            provider.state == crate::machine_protocol::PluginInstallationState::Active
        }) {
            if !state
                .supervisor
                .runtime_allowed(&installed.plugin_id, &machine.id)
            {
                continue;
            }
            if state
                .provider_catalog
                .package(
                    &installed.plugin_id,
                    &installed.plugin_version,
                    &installed.generation_digest,
                )
                .is_none()
            {
                continue;
            }
            installations.push((machine.id.clone(), installed));
        }
    }
    installations
}

pub(super) fn submit_provider_auth_code(
    executors: &parking_lot::Mutex<HashMap<String, ProviderAuthExecutor>>,
    control: &MachineControl,
    owner: &str,
    provider: &str,
    request_id: String,
    code: String,
) -> Response {
    let code = match crate::provider_account::login_code(&code) {
        Ok(code) => code.to_owned(),
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    // Keep validation and enqueue serialized with cancellation/promotion.
    let executors = executors.lock();
    let executor = executors.get(&request_id);
    let Some(executor) = executor.filter(|value| {
        value.owned_by(owner, provider)
            && !value.promotion_started
            && value.active(control, now_ms())
    }) else {
        return (
            StatusCode::NOT_FOUND,
            "authentication request is not active",
        )
            .into_response();
    };
    match control.send_on_connection(
        &executor.connection,
        crate::machine_protocol::MachineCommand::SubmitLoginCode { request_id, code },
    ) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

pub(super) async fn cancel_provider_auth_request(
    state: &Arc<AppState>,
    owner: &str,
    provider_id: &str,
    request_id: String,
) -> Response {
    let executor = {
        let mut executors = state.provider_auth_executors.lock();
        executors
            .get_mut(&request_id)
            .filter(|value| value.owned_by(owner, provider_id))
            .map(|executor| {
                executor.cancel_requested = true;
                executor.clone()
            })
    };
    let Some(executor) = executor else {
        return (
            StatusCode::NOT_FOUND,
            "authentication request is not active",
        )
            .into_response();
    };
    let cleanup = provider_accounts::login::cancel(state, &executor, &request_id).await;
    if let Err(error) = cleanup {
        return (StatusCode::SERVICE_UNAVAILABLE, error).into_response();
    }
    match provider_accounts::login::dispatch_cancel(
        &state.machine_control,
        &state.provider_auth_executors,
        &executor,
        &request_id,
    )
    .await
    {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

/// Give Providers that joined a shared credential scope after its secret was
/// entered the scope's current Service credential, then distribute it. Runs
/// after every Provider Catalog refresh and periodically; it never overwrites
/// a Provider that already has durable authentication state.
pub(super) async fn adopt_shared_provider_authentication(state: &Arc<AppState>) {
    let mut adopted = Vec::new();
    for scope in state.provider_catalog.authentication_scopes() {
        let packages = state
            .provider_catalog
            .packages_for_authentication_scope(&scope);
        match state.provider_auth.adopt_shared_scope(&packages) {
            Ok(statuses) => adopted.extend(statuses),
            Err(error) => {
                tracing::warn!(%error, %scope, "cannot extend shared Provider authentication");
            }
        }
    }
    if adopted.is_empty() {
        return;
    }
    let (statuses, replicas) = distribute_and_mark_provider_auth(state, adopted).await;
    recover_failed_provider_sessions(state, &statuses, &replicas);
    usage_credentials_changed(state, &statuses);
}

pub(super) async fn distribute_and_mark_provider_auth(
    state: &Arc<AppState>,
    statuses: Vec<crate::provider_service::ProviderAuthenticationStatus>,
) -> (
    Vec<crate::provider_service::ProviderAuthenticationStatus>,
    BTreeMap<String, ProviderDistributionOutcome>,
) {
    let mut next = Vec::with_capacity(statuses.len());
    let mut replicas = BTreeMap::new();
    for status in statuses {
        let provider_id = status.provider_id.clone();
        let outcome = distribute_provider_auth(state, &provider_id).await;
        match state.provider_auth.mark_distribution(
            &provider_id,
            status.auth_generation,
            outcome.state(),
        ) {
            Ok(marked) => {
                replicas.insert(provider_id, outcome);
                next.push(marked);
            }
            Err(error) => {
                tracing::warn!(%error, %provider_id, "ignoring stale Provider auth distribution result");
                next.push(state.provider_auth.status(&provider_id).unwrap_or(status));
            }
        }
    }
    (next, replicas)
}

pub(super) async fn distribute_provider_auth(
    state: &Arc<AppState>,
    provider_id: &str,
) -> ProviderDistributionOutcome {
    let connected = state.machine_control.connected_machine_ids();
    let Some(store) = state.store.as_ref() else {
        return ProviderDistributionOutcome {
            failed: 1,
            ..ProviderDistributionOutcome::default()
        };
    };
    let enrolled = match store.list_machines().await {
        Ok(machines) => machines,
        Err(error) => {
            tracing::warn!(%error, %provider_id, "listing enrolled Machines for Provider auth sync");
            return ProviderDistributionOutcome {
                failed: 1,
                ..ProviderDistributionOutcome::default()
            };
        }
    };
    let mut outcome = ProviderDistributionOutcome::default();
    for machine in enrolled.into_iter().filter(|machine| !machine.revoked) {
        let machine_id = machine.id;
        if !connected.contains(&machine_id) {
            outcome.pending += 1;
            continue;
        }
        match sync_provider_auth_to_machine(state, &machine_id, provider_id).await {
            Ok(auth_generation) => {
                outcome.succeeded += 1;
                outcome
                    .synchronized_generations
                    .insert(machine_id, auth_generation);
            }
            Err(error) => {
                outcome.failed += 1;
                tracing::warn!(%error, %machine_id, %provider_id, "Provider auth replica sync failed");
            }
        }
    }
    outcome
}

pub(super) async fn reconcile_account_refresh(
    state: &Arc<AppState>,
    machine_id: &str,
    provider_id: &str,
    account_id: &crate::provider_account::ProviderAccountId,
    result: crate::provider_service::ProviderAuthRefreshResult,
) -> Result<(), String> {
    // Log both outcomes like the Service account does, so a named account's
    // refresh lineage is auditable from the Controller journal.
    let providers = match result {
        crate::provider_service::ProviderAuthRefreshResult::Stale { current_generation } => {
            tracing::info!(
                %machine_id,
                %provider_id,
                account_id = account_id.as_str(),
                current_generation,
                "discarding stale account credential refresh and restoring its grant"
            );
            vec![provider_id.to_owned()]
        }
        crate::provider_service::ProviderAuthRefreshResult::Unchanged(_) => return Ok(()),
        crate::provider_service::ProviderAuthRefreshResult::Updated(statuses) => {
            tracing::info!(
                %machine_id,
                %provider_id,
                account_id = account_id.as_str(),
                generations = ?statuses.iter().map(|status| status.auth_generation).collect::<Vec<_>>(),
                "promoted Machine-refreshed account credentials"
            );
            statuses
                .into_iter()
                .map(|status| status.provider_id)
                .collect()
        }
    };
    // A native refresh belongs to this Machine grant. Never broadcast it to
    // another account or overwrite another Machine's independent refresh lineage.
    for provider in providers {
        sync_and_mark_account_grant(state, machine_id, &provider, account_id).await?;
    }
    Ok(())
}

pub(super) async fn sync_and_mark_account_grant(
    state: &Arc<AppState>,
    machine_id: &str,
    provider_id: &str,
    account_id: &crate::provider_account::ProviderAccountId,
) -> Result<(), String> {
    let generation =
        sync_provider_account_auth_to_machine(state, machine_id, provider_id, account_id).await?;
    provider_auth_scope_for_machine(state, machine_id, account_id)
        .await?
        .mark_distribution(
            provider_id,
            generation,
            crate::provider_service::ServiceDistributionState::Current,
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn failed_provider_auth_projection_ids(
    plugins: &[crate::machine_protocol::PluginInventory],
) -> Vec<String> {
    plugins
        .iter()
        .filter(|plugin| {
            plugin.state == crate::machine_protocol::PluginInstallationState::Active
                && plugin.replica_state == crate::machine_protocol::ProviderReplicaState::Current
                && plugin.materialization_state
                    == crate::machine_protocol::ProviderMaterializationState::Failed
        })
        .map(|plugin| plugin.plugin_id.clone())
        .collect()
}

/// Named accounts always carry evidence. The default account has no recorded
/// identity to compare, and current Machines send none for it, so here this
/// only refuses evidence that breaks the signed method's contract.
pub(super) fn refresh_identity_evidence(
    provider_id: &str,
    account_id: &crate::provider_account::ProviderAccountId,
    recipe: Option<&cowboy_provider_sdk::NativeIdentityRecipe>,
    identity: Option<&crate::provider_account::NativeIdentity>,
) -> Result<(), String> {
    if account_id.is_default() && identity.is_none() {
        return Ok(());
    }
    provider_accounts::validate_native_evidence(provider_id, recipe, identity)
        .map_err(str::to_owned)
}

/// The installed release a named session can move to on its existing login:
/// resolved exactly as new-session admission resolves that login (identical
/// signed authentication to the release it signed in under), with the native
/// session contract unchanged from the session's current release. A changed
/// authentication still requires a new login and an explicit transition.
pub(super) async fn named_upgrade_target(
    state: &Arc<AppState>,
    meta: &crate::core::SessionMeta,
) -> Result<ResolvedProviderGeneration, String> {
    if !state.machine_control.machine_supports(
        &meta.machine_id,
        crate::machine_protocol::NAMED_RELEASE_CARRY_PROTOCOL_VERSION,
    ) {
        return Err(
            "this session's Machine needs a Cowboy update to move it to another release".to_owned(),
        );
    }
    let store = state.store.as_ref().ok_or("account_store_unavailable")?;
    let owner = meta
        .owner_user_id
        .as_deref()
        .ok_or("named session has no owner")?;
    let (operation, completion) = store
        .native_login_for_registered_session(owner, &meta.id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("this session's subscription login is no longer admitted")?;
    let auth_generation = meta
        .provider_auth_generation
        .filter(|generation| *generation == completion.profile.generation)
        .ok_or("this session's login generation changed; reopen it and try again")?;
    let installed = current_machine_plugin(state, &meta.machine_id, &meta.provider).await?;
    let mut target = resolve_provider_generation_with_auth(
        &state.provider_catalog,
        &state.plugin_catalog,
        &[installed],
        &meta.provider,
        SchedulingAuthentication::Native(&completion.profile),
    )?;
    let catalog = &state.provider_catalog;
    let package = |version: &str, digest: &str| {
        catalog
            .package(&meta.provider, version, digest)
            .map(|package| package.manifest)
    };
    let (Some(previous), Some(signed_in), Some(next)) = (
        package(&meta.provider_version, &meta.provider_generation_digest),
        package(
            &completion.profile.provider_version,
            &completion.profile.generation_digest,
        ),
        package(&target.version, &target.digest),
    ) else {
        return Err(
            "a Provider package is unavailable; cannot verify this session's login".to_owned(),
        );
    };
    if !provider_reload_contract_compatible(&previous, &next)
        || !crate::provider_account::native_release_compatible(&signed_in, &next)
    {
        return Err("Provider authentication or native session contract changed; sign in again and move the session explicitly".to_owned());
    }
    target.named_carry = Some(NamedReleaseMove {
        carry: crate::core::NamedReleaseCarry {
            session_id: meta.id.clone(),
            auth_generation,
            version: target.version.clone(),
            digest: target.digest.clone(),
        },
        operation,
        completion,
    });
    Ok(target)
}

/// The Device's installed release for a session pinned to `version`/`digest`,
/// when it keeps authentication and the native session contract unchanged.
/// Depends only on its arguments, so a pass may share it between sessions.
pub(super) async fn installed_upgrade_target(
    state: &Arc<AppState>,
    machine_id: &str,
    provider: &str,
    version: &str,
    digest: &str,
) -> Result<ResolvedProviderGeneration, String> {
    let installed = current_machine_plugin(state, machine_id, provider).await?;
    installed_upgrade_target_from(state, installed, provider, version, digest)
}

/// [`installed_upgrade_target`] for an installation the caller already read.
pub(super) fn installed_upgrade_target_from(
    state: &AppState,
    installed: crate::machine_protocol::PluginInventory,
    provider: &str,
    version: &str,
    digest: &str,
) -> Result<ResolvedProviderGeneration, String> {
    let auth = state.provider_auth.status(provider);
    let generation = resolve_provider_generation(
        &state.provider_catalog,
        &state.plugin_catalog,
        &[installed],
        provider,
        auth.as_ref(),
    )?;
    let previous = state.provider_catalog.package(provider, version, digest).ok_or_else(|| {
        "the previous Provider package is unavailable; cannot verify native session compatibility"
            .to_owned()
    })?;
    let next = state
        .provider_catalog
        .package(provider, &generation.version, &generation.digest)
        .ok_or_else(|| "the target Provider package is unavailable".to_owned())?;
    if !provider_reload_contract_compatible(&previous.manifest, &next.manifest) {
        return Err("Provider authentication or native session contract changed; keeping the existing runtime".to_owned());
    }
    Ok(generation)
}

/// Whether `next` is `previous` plus further sign-in methods. A session can
/// only be bound to a method its release offered, and each of those is
/// unchanged, so it keeps an identical authentication contract. Claude Code
/// 3.20.1 added the named "claude-subscription" method this way; comparing
/// whole-section fingerprints pinned every 3.20.0 session to 3.20.0 forever.
pub(super) fn authentication_only_adds_methods(
    previous: &cowboy_provider_sdk::AuthenticationContract,
    next: &cowboy_provider_sdk::AuthenticationContract,
) -> bool {
    let mut widened = previous.clone();
    widened.methods.clone_from(&next.methods);
    next.methods.len() > previous.methods.len()
        && widened == *next
        && previous
            .methods
            .iter()
            .all(|method| next.methods.contains(method))
}

pub(super) async fn apply_session_provider_reload(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    target: &ResolvedProviderGeneration,
    automatic: bool,
) -> Result<(), String> {
    if execution::recovery::target_proved_lost(state, meta).await {
        return Err(
            "The execution environment was lost; Reload the session or send a message to recover it, then update the Provider"
                .to_owned(),
        );
    }
    let reload = |carry: Option<&crate::core::NamedReleaseCarry>| {
        state.supervisor.reload_session_provider(
            meta,
            crate::supervisor::ProviderGeneration {
                account_id: Some(&meta.provider_account_id),
                version: &target.version,
                digest: &target.digest,
                auth_generation: meta.provider_auth_generation,
                behavior: Some(&target.behavior),
            },
            automatic,
            carry,
        )
    };
    // A named login is admitted by the Store, not by Service authentication.
    if target.named_carry.is_some() {
        return with_release_admission(state, meta, target, reload).await;
    }
    state
        .provider_auth
        .with_scheduling_generation(
            &meta.provider,
            target.auth_generation.is_some(),
            target.auth_generation,
            || reload(None),
        )
        .map_err(|error| error.to_string())?
}

/// Run one session's release change. A named session's change runs inside its
/// login's runtime admission: the session row must still hold this exact
/// release and login generation, and the login must still be admitted (not
/// cancelled, revoked, disabled or re-enrolled). Without a verified move, a
/// named session never changes release here.
pub(super) async fn with_release_admission<T>(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    target: &ResolvedProviderGeneration,
    action: impl FnOnce(Option<&crate::core::NamedReleaseCarry>) -> Result<T, String>,
) -> Result<T, String> {
    let Some(named) = target.named_carry.as_ref() else {
        return action(None);
    };
    let store = state.store.as_ref().ok_or("account_store_unavailable")?;
    let owner = meta
        .owner_user_id
        .as_deref()
        .ok_or("named session has no owner")?;
    let mut outcome = None;
    store
        .with_native_session_release_binding(
            owner,
            &named.operation,
            &named.completion,
            meta,
            || {
                outcome = Some(action(Some(&named.carry)));
                std::future::ready(Ok(()))
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    outcome.unwrap_or_else(|| Err("release admission did not run".to_owned()))
}

/// Re-pin a dormant session through [`with_release_admission`].
pub(super) async fn repin_dormant_session(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    target: &ResolvedProviderGeneration,
) -> Result<(), String> {
    with_release_admission(state, meta, target, |carry| {
        state.hub.repin_dormant_provider(
            meta,
            &target.version,
            &target.digest,
            &target.behavior,
            carry,
        )
    })
    .await
}

pub(super) fn resolve_scheduling_auth_generation(
    installed: &crate::machine_protocol::PluginInventory,
    authentication_required: bool,
    service_auth: Option<&crate::provider_service::ProviderAuthenticationStatus>,
) -> Result<Option<u64>, String> {
    if !authentication_required {
        return Ok(None);
    }
    let provider_id = &installed.plugin_id;
    let service_auth = service_auth.ok_or_else(|| {
        format!("Provider {provider_id:?} is not authenticated at Cowboy Service scope")
    })?;
    if service_auth.authentication_state
        != crate::provider_service::ServiceAuthenticationState::Ready
    {
        return Err(format!(
            "Provider {provider_id:?} authentication is not ready at Cowboy Service scope"
        ));
    }
    if installed.replica_state != crate::machine_protocol::ProviderReplicaState::Current
        || installed.materialization_state
            != crate::machine_protocol::ProviderMaterializationState::Current
    {
        return Err(format!(
            "Provider {provider_id:?} Service authentication is not synchronized to this Machine"
        ));
    }
    let installed_generation = installed
        .auth_generation
        .ok_or_else(|| "Provider auth generation is missing".to_owned())?;
    if installed_generation != service_auth.auth_generation {
        return Err(format!(
            "Provider {provider_id:?} Service authentication generation is not synchronized to this Machine"
        ));
    }
    Ok(Some(installed_generation))
}
