//! Provider authentication executors and distribution, account refresh,
//! and provider release upgrades and reloads for sessions.

use super::*;
use std::time::Duration;

pub mod reconciler;

pub async fn connected_provider_authentication_executors(
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
pub async fn provider_installations(
    state: &AppState,
    connected_only: bool,
) -> Vec<(String, crate::machine_protocol::PluginInventory)> {
    catalogued_installations(&state.accounts, connected_only, true)
        .await
        .unwrap_or_default()
}

pub fn submit_provider_auth_code(
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

pub async fn cancel_provider_auth_request(
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
    let cleanup = provider_accounts::login::cancel(&state.accounts, &executor, &request_id).await;
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
pub async fn adopt_shared_provider_authentication(state: &Arc<AppState>) {
    provider_accounts::portable::adopt(&state.accounts).await;
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

pub async fn distribute_and_mark_provider_auth(
    state: &Arc<AppState>,
    statuses: Vec<crate::provider_service::ProviderAuthenticationStatus>,
) -> (
    Vec<crate::provider_service::ProviderAuthenticationStatus>,
    BTreeMap<String, ProviderDistributionOutcome>,
) {
    distribute_and_mark_scope(
        &state.accounts,
        &state.provider_auth,
        &Default::default(),
        statuses,
    )
    .await
}

pub async fn reconcile_account_refresh(
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

pub async fn sync_and_mark_account_grant(
    state: &Arc<AppState>,
    machine_id: &str,
    provider_id: &str,
    account_id: &crate::provider_account::ProviderAccountId,
) -> Result<(), String> {
    let generation =
        sync_provider_account_auth_to_machine(&state.accounts, machine_id, provider_id, account_id)
            .await?;
    provider_auth_scope_for_machine(&state.accounts, machine_id, account_id)
        .await?
        .mark_distribution(
            provider_id,
            generation,
            crate::provider_service::ServiceDistributionState::Current,
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn failed_provider_auth_projection_ids(
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
pub fn refresh_identity_evidence(
    provider_id: &str,
    account_id: &crate::provider_account::ProviderAccountId,
    recipe: Option<&cowboy_provider_sdk::NativeIdentityRecipe>,
    identity: Option<&crate::provider_account::NativeIdentity>,
) -> Result<(), String> {
    if account_id.is_default() && identity.is_none() {
        return Ok(());
    }
    provider_accounts::validate_native_evidence(provider_id, account_id, recipe, identity)
        .map_err(str::to_owned)
}

/// The installed release a named session can move to on its existing login:
/// resolved exactly as new-session admission resolves that login (identical
/// signed authentication to the release it signed in under), with the native
/// session contract unchanged from the session's current release. A changed
/// authentication still requires a new login and an explicit transition.
pub async fn named_upgrade_target(
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
    // A session still bound to a native login moves on it; every other
    // named session of an account-vault Provider moves on the vault.
    let native = store
        .native_login_for_registered_session(owner, &meta.id)
        .await
        .map_err(|error| error.to_string())?;
    if native.is_none() && provider_accounts::portable::provider(&state.accounts, &meta.provider) {
        return provider_accounts::portable::upgrade_target(&state.accounts, meta).await;
    }
    let (operation, completion) =
        native.ok_or("this session's subscription login is no longer admitted")?;
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
        native: Some((operation, completion)),
    });
    Ok(target)
}

/// The Device's installed release for a session pinned to `version`/`digest`,
/// when it keeps authentication and the native session contract unchanged.
/// Depends only on its arguments, so a pass may share it between sessions.
pub async fn installed_upgrade_target(
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
pub fn installed_upgrade_target_from(
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

pub async fn apply_session_provider_reload(
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
pub async fn with_release_admission<T>(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    target: &ResolvedProviderGeneration,
    action: impl FnOnce(Option<&crate::core::NamedReleaseCarry>) -> Result<T, String>,
) -> Result<T, String> {
    let Some(named) = target.named_carry.as_ref() else {
        return action(None);
    };
    let Some((operation, completion)) = &named.native else {
        return provider_accounts::portable::with_release_admission(&state.accounts, meta, || {
            action(Some(&named.carry))
        })
        .await;
    };
    let store = state.store.as_ref().ok_or("account_store_unavailable")?;
    let owner = meta
        .owner_user_id
        .as_deref()
        .ok_or("named session has no owner")?;
    let mut outcome = None;
    store
        .with_native_session_release_binding(owner, operation, completion, meta, || {
            outcome = Some(action(Some(&named.carry)));
            std::future::ready(Ok(()))
        })
        .await
        .map_err(|error| error.to_string())?;
    outcome.unwrap_or_else(|| Err("release admission did not run".to_owned()))
}

/// Re-pin a dormant session through [`with_release_admission`].
pub async fn repin_dormant_session(
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
