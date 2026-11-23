//! Shared Provider generation resolution, executor, and DTO helpers for provider_accounts.

use super::*;

#[derive(Clone)]
pub struct ResolvedProviderGeneration {
    pub version: String,
    pub digest: String,
    pub auth_generation: Option<u64>,
    pub behavior: cowboy_provider_sdk::ProviderBehaviorContract,
    /// Set only by [`named_upgrade_target`]: authorizes moving one named
    /// session to this release on its existing login.
    pub named_carry: Option<NamedReleaseMove>,
    /// The Service vault owns this credential lineage (no native sign-in).
    pub service_lineage: bool,
}

/// A verified move of one named session to another release: the Hub proof
/// and the native login (operation, completion) whose runtime admission must
/// hold around the change; `None` for a portable account's vault key.
#[derive(Clone)]
pub struct NamedReleaseMove {
    pub carry: crate::core::NamedReleaseCarry,
    pub native: Option<(
        String,
        crate::store::provider_accounts::logins::NativeCompletion,
    )>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderAuthStartRequest {
    #[serde(default)]
    pub machine_id: Option<String>,
    pub method: String,
    pub provider_version: String,
    pub generation_digest: String,
}

#[derive(Debug, Default, Serialize)]
pub struct ProviderDistributionOutcome {
    pub succeeded: usize,
    pub failed: usize,
    pub pending: usize,
    #[serde(skip)]
    pub synchronized_generations: BTreeMap<String, u64>,
}

impl ProviderDistributionOutcome {
    pub const fn state(&self) -> crate::provider_service::ServiceDistributionState {
        use crate::provider_service::ServiceDistributionState;
        if self.failed == 0 && self.pending == 0 {
            ServiceDistributionState::Current
        } else if self.succeeded == 0 && self.failed == 0 {
            ServiceDistributionState::Pending
        } else if self.succeeded == 0 && self.pending == 0 {
            ServiceDistributionState::Failed
        } else {
            ServiceDistributionState::Partial
        }
    }
}

#[derive(Deserialize)]
pub struct ResetRequest {
    pub confirm: String,
    pub expected_credit_id: String,
}

#[derive(Clone)]
pub struct ProviderAuthExecutor {
    pub native_login: Option<provider_accounts::login::NativeLogin>,
    pub named_login: Option<provider_accounts::login::NamedLogin>,
    pub cancel_requested: bool,
    pub owner_user_id: String,
    pub account_id: crate::provider_account::ProviderAccountId,
    pub machine_id: String,
    pub connection: crate::machine_control::ConnectionToken,
    pub provider_id: String,
    pub provider_version: String,
    pub generation_digest: String,
    pub auth_contract_fingerprint: String,
    pub auth_method: String,
    pub expected_generation: u64,
    pub promotion_started: bool,
    pub expires_at_ms: i64,
}

impl ProviderAuthExecutor {
    pub fn auth_scope<'a>(
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
    pub fn owned_by(&self, owner: &str, provider: &str) -> bool {
        self.owner_user_id == owner && self.provider_id == provider
    }

    // Called while holding the executor map lock. A failed validation/commit
    // must remain retryable; only durable credential ownership consumes it.
    pub fn promote_once<T, E>(
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

    pub fn active(&self, control: &MachineControl, timestamp: i64) -> bool {
        !self.cancel_requested
            && self.expires_at_ms > timestamp
            && control.is_current(&self.connection)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn accepts_candidate(
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

/// Active installations of a catalogued release on enrolled, unrevoked
/// Machines. `runtime_policy` additionally applies the Provider execution
/// policy (where a Provider may run); credential discovery does not, since
/// Service credentials reach every enrolled Machine. An unreadable Machine
/// list is an error, never an empty result; an undecodable inventory is
/// skipped for executor selection (`runtime_policy`) and an error otherwise.
#[allow(clippy::result_unit_err)]
pub async fn catalogued_installations(
    state: &AppState,
    connected_only: bool,
    runtime_policy: bool,
) -> Result<Vec<(String, crate::machine_protocol::PluginInventory)>, ()> {
    let store = state.store.as_ref().ok_or(())?;
    let machines = store.list_machines().await.map_err(|_| ())?;
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
            // Executor selection is best-effort; credential discovery must
            // not read an undecodable inventory as "no installation here".
            if runtime_policy {
                continue;
            }
            return Err(());
        };
        for installed in providers.into_iter().filter(|provider| {
            provider.state == crate::machine_protocol::PluginInstallationState::Active
        }) {
            if runtime_policy
                && !state
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
    Ok(installations)
}

pub fn provider_inventory_contract_matches(
    installed: &crate::machine_protocol::PluginInventory,
    provider_fingerprint: &str,
    plugin_fingerprint: Option<&str>,
) -> bool {
    plugin_fingerprint == Some(installed.contract_fingerprint.as_str())
        || ((installed.installation_revision.is_none() || plugin_fingerprint.is_none())
            && provider_fingerprint == installed.contract_fingerprint)
}

pub fn resolve_provider_generation(
    catalog: &crate::provider_catalog::ProviderCatalog,
    plugins: &crate::plugin_catalog::PluginCatalog,
    inventory: &[crate::machine_protocol::PluginInventory],
    provider_id: &str,
    service_auth: Option<&crate::provider_service::ProviderAuthenticationStatus>,
) -> Result<ResolvedProviderGeneration, String> {
    resolve_provider_generation_with_auth(
        catalog,
        plugins,
        inventory,
        provider_id,
        SchedulingAuthentication::Service(service_auth),
    )
}

pub enum SchedulingAuthentication<'a> {
    /// Installation discovery only; never use this result to launch a session.
    Discovery,
    Service(Option<&'a crate::provider_service::ProviderAuthenticationStatus>),
    Native(&'a crate::provider_account::NativeProfileReceipt),
    /// A named portable account's own Service vault entry.
    Portable(&'a crate::provider_service::ProviderAuthenticationStatus),
}

pub fn resolve_provider_generation_with_auth(
    catalog: &crate::provider_catalog::ProviderCatalog,
    plugins: &crate::plugin_catalog::PluginCatalog,
    inventory: &[crate::machine_protocol::PluginInventory],
    provider_id: &str,
    authentication: SchedulingAuthentication<'_>,
) -> Result<ResolvedProviderGeneration, String> {
    let installed = inventory
        .iter()
        .find(|provider| {
            provider.plugin_id == provider_id
                && provider.state == crate::machine_protocol::PluginInstallationState::Active
        })
        .ok_or_else(|| format!("Provider {provider_id:?} is not installed on this Machine"))?;
    let package = catalog
        .package(
            provider_id,
            &installed.plugin_version,
            &installed.generation_digest,
        )
        .ok_or_else(|| {
            format!(
                "installed Provider generation {} is not trusted by this Catalog",
                installed.generation_digest
            )
        })?;
    // Generic Plugin inventory uses the outer, signed Plugin fingerprint. Old
    // Machines still project the inner Provider fingerprint; accept that only
    // for untracked inventories while the reader bridge is rolling out. An
    // exact signed legacy-only Provider Catalog generation has no outer Plugin
    // contract at all; adopting its slot must not break its existing scheduler.
    let exact_plugin = plugins
        .resolve(
            provider_id,
            Some(&installed.plugin_version),
            Some(&installed.generation_digest),
        )
        .ok();
    if !provider_inventory_contract_matches(
        installed,
        &package.contract_fingerprint,
        exact_plugin
            .as_ref()
            .map(|trusted| trusted.release.contract_fingerprint.as_str()),
    ) {
        return Err(
            "installed Provider contract fingerprint does not match the Catalog".to_owned(),
        );
    }
    let service_lineage = matches!(
        authentication,
        SchedulingAuthentication::Service(_) | SchedulingAuthentication::Portable(_)
    );
    let auth_generation = match authentication {
        SchedulingAuthentication::Discovery => None,
        SchedulingAuthentication::Portable(status) => {
            provider_accounts::portable::scheduling_generation(
                package.manifest.authentication.required,
                status,
            )?
        }
        SchedulingAuthentication::Service(service_auth) => resolve_scheduling_auth_generation(
            installed,
            package.manifest.authentication.required,
            service_auth,
        )?,
        SchedulingAuthentication::Native(profile) => {
            let signed_in = catalog.package(
                provider_id,
                &profile.provider_version,
                &profile.generation_digest,
            );
            Some(provider_accounts::native_generation::resolve(
                profile,
                installed,
                &package.manifest,
                signed_in.as_ref().map(|signed_in| &signed_in.manifest),
            )?)
        }
    };
    Ok(ResolvedProviderGeneration {
        version: installed.plugin_version.clone(),
        digest: installed.generation_digest.clone(),
        auth_generation,
        behavior: package.manifest.runtime.behavior,
        named_carry: None,
        service_lineage,
    })
}

pub struct ProviderAuthReconciliation {
    pub expired: Vec<(String, String)>,
    pub active: Option<(String, ProviderAuthExecutor)>,
}

impl ProviderAuthReconciliation {
    pub fn resumable(
        &self,
        active_request_id: Option<&str>,
    ) -> Option<(String, ProviderAuthExecutor)> {
        self.active
            .as_ref()
            .filter(|(request_id, _)| active_request_id == Some(request_id.as_str()))
            .cloned()
    }
}
pub fn reconcile_provider_auth_executors(
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

pub fn resumed_provider_authentication_response(
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

pub async fn distribute_and_mark_scope(
    state: &Arc<AppState>,
    scope: &Arc<crate::provider_service::ProviderAuthService>,
    account: &crate::provider_account::ProviderAccountId,
    statuses: Vec<crate::provider_service::ProviderAuthenticationStatus>,
) -> (
    Vec<crate::provider_service::ProviderAuthenticationStatus>,
    BTreeMap<String, ProviderDistributionOutcome>,
) {
    let mut next = Vec::with_capacity(statuses.len());
    let mut replicas = BTreeMap::new();
    for status in statuses {
        let provider_id = status.provider_id.clone();
        let outcome = distribute_account_auth(state, &provider_id, account).await;
        match scope.mark_distribution(&provider_id, status.auth_generation, outcome.state()) {
            Ok(marked) => {
                replicas.insert(provider_id, outcome);
                next.push(marked);
            }
            Err(error) => {
                tracing::warn!(%error, %provider_id, "ignoring stale Provider auth distribution result");
                next.push(scope.status(&provider_id).unwrap_or(status));
            }
        }
    }
    (next, replicas)
}

async fn distribute_account_auth(
    state: &Arc<AppState>,
    provider_id: &str,
    account: &crate::provider_account::ProviderAccountId,
) -> ProviderDistributionOutcome {
    let connected = state.machine_control.connected_machine_ids();
    let Some(store) = state.store.as_ref() else {
        return ProviderDistributionOutcome {
            failed: 1,
            ..ProviderDistributionOutcome::default()
        };
    };
    // The same recipients the reconciler and the retirement gate use.
    let enrolled = match store.credential_recipients().await {
        Ok(recipients) => recipients,
        Err(error) => {
            tracing::warn!(%error, %provider_id, "listing enrolled Machines for Provider auth sync");
            return ProviderDistributionOutcome {
                failed: 1,
                ..ProviderDistributionOutcome::default()
            };
        }
    };
    let mut outcome = ProviderDistributionOutcome::default();
    for (machine_id, _) in enrolled {
        if !connected.contains(&machine_id) {
            outcome.pending += 1;
            continue;
        }
        match sync_provider_account_auth_to_machine(state, &machine_id, provider_id, account).await
        {
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

pub fn authentication_only_adds_methods(
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

pub fn resolve_scheduling_auth_generation(
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

pub fn provider_reload_contract_compatible(
    previous: &cowboy_provider_sdk::ProviderManifest,
    next: &cowboy_provider_sdk::ProviderManifest,
) -> bool {
    previous.id == next.id
        && (previous.compatibility.auth_contract_fingerprint
            == next.compatibility.auth_contract_fingerprint
            || authentication_only_adds_methods(&previous.authentication, &next.authentication))
        && previous.authentication.projection_schema == next.authentication.projection_schema
        && previous.authentication.portable_schema == next.authentication.portable_schema
        && previous.runtime.behavior.session == next.runtime.behavior.session
        && previous.runtime.behavior.configuration == next.runtime.behavior.configuration
}

pub async fn sealed_auth_envelope_for_machine(
    state: &Arc<AppState>,
    machine_id: &str,
    provider_id: &str,
    account_id: &crate::provider_account::ProviderAccountId,
) -> Result<(crate::machine_protocol::SealedProviderAuth, String), String> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(|| "Provider authentication sync requires persistence".to_owned())?;
    let public_key = store
        .machine_encryption_public_key(machine_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Machine {machine_id:?} has no enrolled encryption key"))?;
    // A named portable account's key lives at account scope and is sealed to
    // any enrolled Machine; native accounts keep their per-Machine grants.
    if let Some(scope) = provider_accounts::portable::scope(state, account_id, provider_id) {
        if !provider_accounts::portable::machine_ready(state, machine_id, provider_id) {
            return Err("machine_update_required".into());
        }
        return scope
            .seal_for_machine(provider_id, &public_key)
            .map(|envelope| (envelope, public_key))
            .map_err(|error| error.to_string());
    }
    let scope = provider_auth_scope_for_machine(state, machine_id, account_id).await?;
    if account_id.is_default()
        || scope.status(provider_id).is_some_and(|status| {
            status.authentication_state
                == crate::provider_service::ServiceAuthenticationState::SignedOut
        })
    {
        let envelope = scope
            .seal_for_machine(provider_id, &public_key)
            .map_err(|error| error.to_string())?;
        if !account_id.is_default()
            && envelope.action != crate::machine_protocol::ProviderAuthAction::Wipe
        {
            return Err("Provider grant changed during cleanup".into());
        }
        Ok((envelope, public_key))
    } else {
        scope
            .seal_current_grant(store, machine_id, provider_id)
            .await
            .map(|envelope| (envelope, public_key))
            .map_err(|error| error.to_string())
    }
}

pub async fn provider_auth_scope_for_machine(
    state: &Arc<AppState>,
    machine_id: &str,
    account_id: &crate::provider_account::ProviderAccountId,
) -> Result<Arc<crate::provider_service::ProviderAuthService>, String> {
    if account_id.is_default() {
        return Ok(Arc::clone(&state.provider_auth));
    }
    if state
        .machine_control
        .connection_protocol(machine_id)
        .is_none()
    {
        return Err("Machine does not support named Provider accounts".to_owned());
    }
    let store = state
        .store
        .as_ref()
        .ok_or("Provider authentication requires persistence")?;
    let key = store
        .machine_encryption_public_key(machine_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("Machine has no enrolled encryption key")?;
    state
        .provider_auth
        .existing_machine_grant(account_id, &key)
        .map_err(|error| error.to_string())
}

pub async fn sync_provider_auth_to_machine(
    state: &Arc<AppState>,
    machine_id: &str,
    provider_id: &str,
) -> Result<u64, String> {
    sync_provider_account_auth_to_machine(
        state,
        machine_id,
        provider_id,
        &crate::provider_account::ProviderAccountId::default(),
    )
    .await
}

pub async fn sync_provider_account_auth_to_machine(
    state: &Arc<AppState>,
    machine_id: &str,
    provider_id: &str,
    account_id: &crate::provider_account::ProviderAccountId,
) -> Result<u64, String> {
    let connection = state.machine_control.operation_connection(machine_id)?;
    let (envelope, sealed_to) =
        sealed_auth_envelope_for_machine(state, machine_id, provider_id, account_id).await?;
    let generation = state
        .provider_auth_sync
        .apply(&state.machine_control, &connection, envelope)
        .await?;
    // The durable per-Machine acknowledgement the reconciler and the
    // retirement's startup gate read.
    let scope = if account_id.is_default() {
        Some(Arc::clone(&state.provider_auth))
    } else {
        state.provider_auth.existing_account(account_id)
    };
    // Exactly the enrollment the envelope was sealed to, never a key read
    // after the receipt (a re-enrollment may have landed meanwhile).
    let enrollment = crate::admin::hex_sha256(sealed_to.as_bytes());
    if let Some(scope) = scope
        && let Err(error) =
            scope.record_delivery(provider_id, (machine_id, &enrollment), generation)
    {
        tracing::warn!(%error, %machine_id, %provider_id, "recording a credential delivery acknowledgement");
    }
    Ok(generation)
}
