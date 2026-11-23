//! Machine browsers the retired legacy default cards left behind
//! (`docs/legacy-card-retirement.md`). A card key
//! (`store::provider_accounts::legacy_cards::browser_key`) that is no
//! registry account (an adopted card's key is one) but still has a placement
//! can be read, stopped and reset, never opened, so its owner can wipe the
//! profile. See `docs/machine-browser.md`, "Legacy default cards".

use super::super::super::*;
use crate::provider_account::ProviderAccountId;
use crate::provider_service::ServiceAuthenticationState;
use crate::store::provider_accounts::legacy_cards;

/// Whether `id` is a retired card key of `owner` with a retained placement.
/// The caller has already ruled out registry accounts and share keys.
pub async fn retained(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<bool, &'static str> {
    if !legacy_cards::is_browser_key(id) {
        return Ok(false);
    }
    Ok(!state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .browser_placements(owner, id)
        .await
        .map_err(|_| "unavailable")?
        .is_empty())
}

/// Machines holding a named account's credentials: its retained native
/// login generations, and, once its account vault holds a Ready credential,
/// every Machine with its Provider installed (the vault serves them all).
pub async fn named_credential_machines(
    state: &AppState,
    owner: &str,
    account: &ProviderAccountId,
    provider: &str,
) -> Result<Vec<String>, &'static str> {
    let mut machines = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .native_generation_machines(owner, account)
        .await
        .map_err(|_| "unavailable")?;
    let vault = super::super::portable::scope(state, account, provider)
        .and_then(|vault| vault.status(provider))
        .is_some_and(|status| status.authentication_state == ServiceAuthenticationState::Ready);
    if vault {
        let installed = crate::server::catalogued_installations(state, false, false)
            .await
            .map_err(|()| "unavailable")?;
        machines.extend(
            installed
                .into_iter()
                .filter(|(_, installed)| installed.plugin_id == provider)
                .map(|(machine, _)| machine),
        );
        machines.sort();
        machines.dedup();
    }
    Ok(machines)
}
