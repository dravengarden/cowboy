//! Machine-proposed refreshes of an account-vault credential: the same
//! Service compare-and-swap lineage as the default account, scoped to the
//! account. One Machine's refresh wins and is delivered to every Machine; a
//! concurrent loser receives the current generation instead.
use super::*;

/// Whether a Machine can hold this account credential. A subscription's
/// account-scope replica needs a Machine that runs vault-bound sessions.
pub fn machine_ready(state: &AppState, machine_id: &str, provider: &str) -> bool {
    !subscription(state, provider)
        || state.machine_control.machine_supports(
            machine_id,
            crate::machine_protocol::PORTABLE_SUBSCRIPTION_PROTOCOL_VERSION,
        )
}

pub fn subscription(state: &AppState, provider: &str) -> bool {
    state
        .provider_catalog
        .latest_package(provider)
        .is_some_and(|(_, _, package)| {
            crate::provider_catalog::subscription_account_contract(&package.manifest.authentication)
        })
}

/// A subscription refresh (own or a redirected legacy rotation) must carry
/// identity evidence; an entered key has none to carry.
fn require_evidence(
    subscription: bool,
    identity: Option<&crate::provider_account::NativeIdentity>,
) -> Result<(), &'static str> {
    if subscription && identity.is_none() {
        Err("native_identity_evidence_required")
    } else {
        Ok(())
    }
}

/// Identity evidence of a refreshed subscription credential (read from its
/// credential file with the signed recipe) must fit that recipe; binding
/// then checks it is the account's identity.
fn refresh_identity(
    package: &cowboy_provider_sdk::ProviderPackage,
    account: &ProviderAccountId,
    bundle: &crate::machine_protocol::PortableCredentialBundle,
    identity: Option<&crate::provider_account::NativeIdentity>,
) -> Result<(), &'static str> {
    let Some(identity) = identity else {
        return Ok(());
    };
    let auth = &package.manifest.authentication;
    let recipe =
        crate::provider_account::portable_contract::native_method_for(auth, &bundle.method_id)
            .and_then(|method| method.native_identity.as_ref())
            .ok_or("native_identity_evidence_unexpected")?;
    validate_native_evidence(&package.manifest.id, account, Some(recipe), Some(identity))
}

/// Route a Machine-proposed refresh that belongs to an account vault: a
/// named account's own, or a rotation of a legacy root generation that
/// moved into an account (it lands in that account). `None` leaves it to
/// the Service account or a Machine grant.
pub async fn route_refresh(
    state: &Arc<AppState>,
    machine_id: &str,
    account: &ProviderAccountId,
    package: &cowboy_provider_sdk::ProviderPackage,
    bundle: &crate::machine_protocol::PortableCredentialBundle,
    expected_generation: u64,
    identity: Option<&crate::provider_account::NativeIdentity>,
) -> Option<Result<(), String>> {
    let provider = package.manifest.id.as_str();
    if account.is_default() {
        let moved = state
            .provider_auth
            .legacy_move(provider)
            .filter(|moved| moved.root_generation == expected_generation)?;
        return Some(
            refresh(
                state,
                machine_id,
                &moved.account_id,
                package,
                bundle,
                moved.account_generation,
                identity,
            )
            .await,
        );
    }
    scope(state, account, provider)?;
    Some(
        refresh(
            state,
            machine_id,
            account,
            package,
            bundle,
            expected_generation,
            identity,
        )
        .await,
    )
}

/// A stale Service-account refresh: the root generation may have moved
/// into an account meanwhile, so forward the rotation there; otherwise
/// restore the Service generation on the Machine.
pub async fn forward_or_restore(
    (request_id, current_generation): (&str, u64),
    state: &Arc<AppState>,
    machine_id: &str,
    package: &cowboy_provider_sdk::ProviderPackage,
    bundle: &crate::machine_protocol::PortableCredentialBundle,
    expected_generation: u64,
    identity: Option<&crate::provider_account::NativeIdentity>,
) -> Result<(), String> {
    tracing::info!(%request_id, %machine_id, provider = %package.manifest.id, expected_generation,
        current_generation, "stale Provider credential refresh: forwarding a moved rotation or restoring the Service generation");
    let default = ProviderAccountId::default();
    if let Some(routed) = route_refresh(
        state,
        machine_id,
        &default,
        package,
        bundle,
        expected_generation,
        identity,
    )
    .await
    {
        return routed;
    }
    sync_provider_auth_to_machine(state, machine_id, &package.manifest.id)
        .await
        .map(drop)
}

pub async fn refresh(
    state: &Arc<AppState>,
    machine_id: &str,
    account: &ProviderAccountId,
    package: &cowboy_provider_sdk::ProviderPackage,
    bundle: &crate::machine_protocol::PortableCredentialBundle,
    expected_generation: u64,
    identity: Option<&crate::provider_account::NativeIdentity>,
) -> Result<(), String> {
    let provider = package.manifest.id.as_str();
    let vault = scope(state, account, provider).ok_or("account_credentials_unavailable")?;
    // A subscription refresh must prove it is still the account's identity:
    // without evidence it could carry another user's sign-in into the vault.
    require_evidence(subscription(state, provider), identity)?;
    refresh_identity(package, account, bundle, identity)?;
    let owner = state
        .store
        .as_ref()
        .ok_or("persistence_required")?
        .provider_account_owner(account)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("account_unavailable")?;
    let packages = super::login::scope_packages(state, package);
    // Under the owner's account lock: a disabled or revoking account takes
    // no refresh, and identity evidence binds or must match the account's.
    let usage = identity.map_or(Use::Commit, Use::SignIn);
    let mut result = None;
    state
        .store
        .as_ref()
        .ok_or("persistence_required")?
        .with_portable_account(
            &owner,
            account,
            usage,
            |current| {
                current.provider_id == provider
                    || packages
                        .iter()
                        .any(|p| p.manifest.id == current.provider_id)
            },
            || {
                result = Some(vault.compare_and_swap_refresh(
                    &packages,
                    bundle,
                    provider,
                    expected_generation,
                ));
                std::future::ready(Ok(()))
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    match result
        .ok_or("refresh_not_admitted")?
        .map_err(|error| error.to_string())?
    {
        crate::provider_service::ProviderAuthRefreshResult::Stale { current_generation } => {
            tracing::info!(%machine_id, %provider, account = account.as_str(), expected_generation,
                current_generation, "discarding stale account credential refresh and restoring the vault generation");
            sync_provider_account_auth_to_machine(state, machine_id, provider, account).await?;
        }
        crate::provider_service::ProviderAuthRefreshResult::Unchanged(_) => {}
        crate::provider_service::ProviderAuthRefreshResult::Updated(statuses) => {
            tracing::info!(%machine_id, %provider, account = account.as_str(), expected_generation,
                generations = ?statuses.iter().map(|status| status.auth_generation).collect::<Vec<_>>(),
                "promoted a Machine-refreshed account credential to every Machine");
            distribute_and_mark_scope(state, &vault, account, statuses).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refreshed_subscription_proves_only_its_own_recipe_identity() {
        let source: cowboy_provider_sdk::StandardProviderSource = serde_json::from_str(
            include_str!("../../../../../../plugins/codex/provider.json"),
        )
        .unwrap();
        let package = cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap();
        let account: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let bundle = crate::machine_protocol::PortableCredentialBundle {
            portable_schema: "codex-auth-v1".into(),
            method_id: "chatgpt".into(),
            values: BTreeMap::new(),
        };
        let identity = crate::provider_account::NativeIdentity {
            authority: "chatgpt".into(),
            subject: "user".into(),
            tenant: Some("org".into()),
        };
        assert_eq!(refresh_identity(&package, &account, &bundle, None), Ok(()));
        // Without evidence a subscription refresh never reaches the vault.
        assert_eq!(
            require_evidence(true, None),
            Err("native_identity_evidence_required")
        );
        assert_eq!(require_evidence(true, Some(&identity)), Ok(()));
        assert_eq!(require_evidence(false, None), Ok(()));
        assert_eq!(
            refresh_identity(&package, &account, &bundle, Some(&identity)),
            Ok(())
        );
        let mut other = identity.clone();
        other.authority = "x-ai".into();
        assert!(refresh_identity(&package, &account, &bundle, Some(&other)).is_err());
        let mut key = bundle;
        key.method_id = "api-key".into();
        assert!(refresh_identity(&package, &account, &key, Some(&identity)).is_err());
    }
}
