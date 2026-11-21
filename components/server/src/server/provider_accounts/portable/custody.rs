//! Refresh custody of named portable subscription accounts (Grok): the
//! refresh token rests only in the account vault. Every Machine receives an
//! access-only copy; one recorded renewal at a time restores the full
//! credential on one Machine (`usage::portable::renewal`). See
//! `docs/portable-refresh-lease.md` ("Grok implementation design").
//!
//! A Provider without the signed declaration (Codex, DeepSeek, Claude,
//! Antigravity) is never managed and keeps every existing path.
use super::*;
use crate::machine_protocol::refresh_custody::RefreshCustody;

/// Closed reason: the Machine is too old to run a custody-managed account.
pub(in crate::server) const CUSTODY_MACHINE_UPDATE_REQUIRED: &str =
    "portable_refresh_custody_machine_update_required";

/// How a Machine must receive `account`'s `provider` credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::server) enum SealMode {
    /// Today's full copy (not managed).
    Full,
    /// Access-only copy under this declaration.
    AccessOnly(RefreshCustody),
}

/// The seal mode, failing closed: an account recorded as managed whose
/// Provider no longer declares custody is never sealed with its refresh
/// token again.
pub(in crate::server) fn seal_mode(
    state: &AppState,
    account: &ProviderAccountId,
    provider: &str,
) -> Result<SealMode, &'static str> {
    if account.is_default() {
        return Ok(SealMode::Full);
    }
    let custody = &state.usage.custody;
    mode(
        state.usage.refresh_custody(provider),
        custody.managed(account.as_str()),
        custody.recorded_managed(account.as_str()),
    )
}

fn mode(
    declaration: Option<RefreshCustody>,
    managed: bool,
    recorded: bool,
) -> Result<SealMode, &'static str> {
    match declaration {
        Some(declaration) if managed => Ok(SealMode::AccessOnly(declaration)),
        None if recorded => Err("refresh_custody_declaration_missing"),
        _ => Ok(SealMode::Full),
    }
}

/// Whether `account` is custody-managed for `provider` (or must be treated so).
pub(in crate::server) fn managed(
    state: &AppState,
    account: &ProviderAccountId,
    provider: &str,
) -> bool {
    !matches!(seal_mode(state, account, provider), Ok(SealMode::Full))
}

/// A managed account never runs on a Machine below protocol 51.
pub(in crate::server) fn machine_admits(
    state: &AppState,
    account: &ProviderAccountId,
    machine_id: &str,
    provider: &str,
) -> Result<(), &'static str> {
    if managed(state, account, provider)
        && !state.machine_control.machine_supports(
            machine_id,
            crate::machine_protocol::REFRESH_CUSTODY_PROTOCOL_VERSION,
        )
    {
        return Err(CUSTODY_MACHINE_UPDATE_REQUIRED);
    }
    Ok(())
}

/// Seal `account`'s current `provider` generation for one Machine, honouring
/// its seal mode.
pub(in crate::server) fn seal_for_machine(
    state: &AppState,
    scope: &ProviderAuthService,
    account: &ProviderAccountId,
    provider: &str,
    machine_public_key: &str,
) -> Result<crate::machine_protocol::SealedProviderAuth, String> {
    match seal_mode(state, account, provider).map_err(str::to_owned)? {
        SealMode::Full => scope.seal_for_machine(provider, machine_public_key),
        SealMode::AccessOnly(custody) => {
            scope.seal_access_only_for_machine(provider, machine_public_key, &custody)
        }
    }
    .map_err(|error| error.to_string())
}

/// Whether the account is quiescent: no worker of it runs anywhere, and none
/// of its live sessions sits on a Machine whose state this Controller cannot
/// see. Only then can no process hold a refresh token it was sealed before.
fn quiescent(state: &AppState, account: &ProviderAccountId) -> bool {
    let connected = state.machine_control.connected_machine_ids();
    state.hub.session_list().iter().all(|meta| {
        meta.provider_account_id != *account
            || meta.trashed_at_ms.is_some()
            || (!state.supervisor.has_live_worker(&meta.id) && connected.contains(&meta.machine_id))
    })
}

/// Make a declaring Provider's account managed once it is quiescent, then
/// re-seal its access-only copy to every Machine. Returns whether the
/// account is managed now.
pub(in crate::server) async fn ensure_managed(
    state: &Arc<AppState>,
    account: &ProviderAccountId,
    provider: &str,
) -> bool {
    if managed(state, account, provider) {
        return true;
    }
    if account.is_default()
        || state.usage.refresh_custody(provider).is_none()
        || scope(state, account, provider).is_none()
        || !quiescent(state, account)
        || !state.usage.custody.enable(account.as_str())
    {
        return false;
    }
    tracing::info!(account = account.as_str(), %provider,
        "refresh custody enabled: every Machine now receives an access-only copy");
    if let Some(vault) = scope(state, account, provider) {
        let statuses = vault.status(provider).into_iter().collect();
        distribute_and_mark_scope(state, &vault, account, statuses).await;
    }
    true
}

/// Apply a recorded renewal's resulting credential: the unchanged account
/// lock and generation compare-and-swap, then access-only distribution.
/// Returns whether the vault's token at `expected_generation` was redeemed
/// (a rotation, applied or stale), so no renewal reuses it.
pub(in crate::server) async fn apply_renewal(
    state: &Arc<AppState>,
    account: &ProviderAccountId,
    provider: &str,
    bundle: &crate::machine_protocol::PortableCredentialBundle,
    expected_generation: u64,
) -> Result<bool, String> {
    let vault = scope(state, account, provider).ok_or("account_credentials_unavailable")?;
    let (_, _, package) = state
        .provider_catalog
        .latest_package(provider)
        .ok_or("provider_unavailable")?;
    let packages = super::login::scope_packages(state, &package);
    let store = state.store.as_ref().ok_or("persistence_required")?;
    let owner = store
        .provider_account_owner(account)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("account_unavailable")?;
    let mut result = None;
    store
        .with_portable_account(
            &owner,
            account,
            Use::Commit,
            |current| current.provider_id == provider,
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
        crate::provider_service::ProviderAuthRefreshResult::Updated(statuses) => {
            tracing::info!(account = account.as_str(), %provider, expected_generation,
                "promoted a renewed account credential; distributing access-only copies");
            distribute_and_mark_scope(state, &vault, account, statuses).await;
            Ok(true)
        }
        crate::provider_service::ProviderAuthRefreshResult::Unchanged(_) => Ok(false),
        // The vault moved on meanwhile (a new sign-in); this run still
        // redeemed its own generation's token.
        crate::provider_service::ProviderAuthRefreshResult::Stale { .. } => Ok(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Provider without the declaration (Codex, DeepSeek, Claude,
    /// Antigravity) keeps the full seal in every state, including a poisoned
    /// custody store; a declaring one is access-only once managed, and an
    /// account recorded as managed never falls back to a full seal.
    #[test]
    fn only_a_declaring_provider_is_sealed_access_only() {
        let grok = RefreshCustody::parse(include_str!(
            "../../../../../../plugins/grok/collector/refresh-custody.json"
        ))
        .unwrap();
        for (managed, recorded) in [(false, false), (true, false), (true, true)] {
            assert_eq!(
                mode(None, managed, false),
                Ok(SealMode::Full),
                "{managed} {recorded}"
            );
        }
        assert_eq!(mode(Some(grok.clone()), false, false), Ok(SealMode::Full));
        assert_eq!(
            mode(Some(grok.clone()), true, false),
            Ok(SealMode::AccessOnly(grok.clone()))
        );
        assert_eq!(
            mode(Some(grok.clone()), true, true),
            Ok(SealMode::AccessOnly(grok))
        );
        assert_eq!(
            mode(None, true, true),
            Err("refresh_custody_declaration_missing")
        );
    }

    #[test]
    fn the_closed_reason_is_stable() {
        assert_eq!(
            CUSTODY_MACHINE_UPDATE_REQUIRED,
            "portable_refresh_custody_machine_update_required"
        );
    }
}
