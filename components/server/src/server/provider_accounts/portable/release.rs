//! Portable session registration and release moves, each under the owner's
//! account lock with the account vault key held Ready.
use super::*;

/// The registry accounts that serve `provider`: its own, or one sharing its
/// portable credential scope.
fn serves(
    state: &AppState,
    provider: &str,
) -> impl Fn(&crate::store::provider_accounts::Account) -> bool {
    let provider = provider.to_owned();
    let members = state.provider_catalog.portable_account_scope(&provider);
    move |account| account.provider_id == provider || members.contains(&account.provider_id)
}

/// Run `action` while the vault holds `current` (or an older `retained`
/// session generation of the same lineage) Ready and immutable.
fn hold<T>(
    scope: &ProviderAuthService,
    provider: &str,
    current: Option<u64>,
    retained: Option<u64>,
    action: impl FnOnce() -> T,
) -> anyhow::Result<T> {
    match (current, retained) {
        (Some(current), Some(retained)) if current != retained => {
            scope.with_retained_scheduling_generation(provider, current, retained, action)
        }
        _ => scope.with_scheduling_generation(provider, true, retained, action),
    }
}

/// Persist the session row under the owner's account lock first, then
/// project it under fresh admission, like native registration: a concurrent
/// revocation either sees the row or refuses this registration, and no
/// worker can be admitted before the row exists.
pub(in crate::server) async fn register(
    state: &AppState,
    registration: crate::core::SessionRegistration,
    scheduling: Option<u64>,
) -> Result<(), String> {
    let owner = registration
        .owner_user_id
        .clone()
        .ok_or("account_owner_required")?;
    let store = state.store.as_ref().ok_or("account_store_unavailable")?;
    let account = registration.provider_account_id.clone();
    let provider = registration.provider.clone();
    let key = (registration.machine_id.clone(), provider.clone());
    let scope = scope(state, &account, &provider).ok_or("account_credentials_unavailable")?;
    if !super::machine_ready(state, &registration.machine_id, &provider) {
        return Err(super::super::registration::MACHINE_UPDATE_REQUIRED.into());
    }
    let (connection, _) = state
        .machine_control
        .connection_protocol(&registration.machine_id)
        .ok_or("machine_unavailable")?;
    let retained = registration.provider_auth_generation;
    let current = scheduling.or(retained);
    let meta = crate::core::Hub::prepare_session(registration);
    store
        .with_portable_account(
            &owner,
            &account,
            Use::Insert(&meta),
            serves(state, &provider),
            || std::future::ready(hold(&scope, &provider, current, retained, || ())),
        )
        .await
        .map_err(|error| error.to_string())?;
    store
        .with_portable_account(
            &owner,
            &account,
            Use::Existing(&meta),
            serves(state, &provider),
            || {
                std::future::ready(
                    hold(&scope, &provider, current, retained, || {
                        anyhow::ensure!(
                            !state.plugin_lifecycle_fences.read().contains_key(&key),
                            "Provider is changing on the selected Machine"
                        );
                        anyhow::ensure!(
                            state.machine_control.is_current(&connection),
                            "machine_connection_changed"
                        );
                        anyhow::ensure!(
                            state
                                .supervisor
                                .runtime_allowed(&meta.provider, &meta.machine_id),
                            "runtime_not_allowed"
                        );
                        anyhow::ensure!(
                            state.hub.project_persisted_session(meta.clone()),
                            "session_projection_conflict"
                        );
                        Ok(())
                    })
                    .and_then(|result| result),
                )
            },
        )
        .await
        .map_err(|error| error.to_string())
}

/// Run one portable session's release change under the owner's account lock
/// on its exact persisted binding, with the vault key held Ready.
pub(in crate::server) async fn with_release_admission<T>(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let owner = meta
        .owner_user_id
        .as_deref()
        .ok_or("named session has no owner")?;
    let store = state.store.as_ref().ok_or("account_store_unavailable")?;
    let account = &meta.provider_account_id;
    let scope = scope(state, account, &meta.provider).ok_or("account_credentials_unavailable")?;
    let current = ready(scope.status(&meta.provider))?.auth_generation;
    let mut outcome = None;
    store
        .with_portable_account(
            owner,
            account,
            Use::Existing(meta),
            serves(state, &meta.provider),
            || {
                std::future::ready(hold(
                    &scope,
                    &meta.provider,
                    Some(current),
                    meta.provider_auth_generation,
                    || {
                        outcome = Some(action());
                    },
                ))
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    outcome.unwrap_or_else(|| Err("release admission did not run".to_owned()))
}

/// The installed release a portable session can move to: the account key
/// stays usable when the target's portable credential contract is the one
/// the vault key was committed under, and the session contract is unchanged.
pub(in crate::server) async fn upgrade_target(
    state: &Arc<AppState>,
    meta: &crate::core::SessionMeta,
) -> Result<ResolvedProviderGeneration, String> {
    let owner = meta
        .owner_user_id
        .as_deref()
        .ok_or("named session has no owner")?;
    let status = admitted_status(state, owner, &meta.provider_account_id, &meta.provider).await?;
    let auth_generation = meta
        .provider_auth_generation
        .filter(|generation| *generation > 0 && *generation <= status.auth_generation)
        .ok_or("this subscription's credential changed; reopen the session and try again")?;
    let installed = current_machine_plugin(state, &meta.machine_id, &meta.provider).await?;
    let mut target = resolve_provider_generation_with_auth(
        &state.provider_catalog,
        &state.plugin_catalog,
        &[installed],
        &meta.provider,
        SchedulingAuthentication::Portable(&status),
    )?;
    let catalog = &state.provider_catalog;
    let (Some(previous), Some(next)) = (
        catalog.package(
            &meta.provider,
            &meta.provider_version,
            &meta.provider_generation_digest,
        ),
        catalog.package(&meta.provider, &target.version, &target.digest),
    ) else {
        return Err(
            "a Provider package is unavailable; cannot verify this session's key".to_owned(),
        );
    };
    if !release_keeps_key(&previous, &next, &status.auth_contract_fingerprint) {
        return Err("Provider credential or session contract changed; sign in or enter the API key again and start a new session".to_owned());
    }
    target.named_carry = Some(NamedReleaseMove {
        carry: crate::core::NamedReleaseCarry {
            session_id: meta.id.clone(),
            auth_generation,
            version: target.version.clone(),
            digest: target.digest.clone(),
        },
        native: None,
    });
    Ok(target)
}

/// The Machine refuses a launch whose release does not match the replica's
/// portable contract fingerprint, so a move must keep that exact contract.
fn release_keeps_key(
    previous: &cowboy_provider_sdk::ProviderPackage,
    next: &cowboy_provider_sdk::ProviderPackage,
    key_contract: &str,
) -> bool {
    provider_reload_contract_compatible(&previous.manifest, &next.manifest)
        && crate::provider_catalog::account_vault_contract(&next.manifest.authentication)
        && crate::provider_account::portable_contract::fingerprint(next)
            .is_ok_and(|fingerprint| fingerprint == key_contract)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_move_keeps_only_the_committed_key_contract() {
        let source: cowboy_provider_sdk::StandardProviderSource = serde_json::from_str(
            include_str!("../../../../../../plugins/codex-deepseek/provider.json"),
        )
        .unwrap();
        let previous =
            cowboy_provider_sdk::build_package(source.clone().compile().unwrap()).unwrap();
        let key = crate::provider_account::portable_contract::fingerprint(&previous).unwrap();
        let mut next_source = source.clone();
        next_source.version = "999.0.0".into();
        let next = cowboy_provider_sdk::build_package(next_source.compile().unwrap()).unwrap();
        assert!(release_keeps_key(&previous, &next, &key));
        assert!(!release_keeps_key(&previous, &next, "sha256:other"));
        let mut changed = source;
        changed.version = "999.0.1".into();
        changed.authentication.projection_schema.push_str("-next");
        let changed = cowboy_provider_sdk::build_package(changed.compile().unwrap()).unwrap();
        assert!(!release_keeps_key(&previous, &changed, &key));
    }
}
