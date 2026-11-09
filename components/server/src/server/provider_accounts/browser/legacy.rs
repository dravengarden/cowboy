//! Machine browsers of the legacy default usage cards: the Service-collected
//! default account of each account-usage provider. A card is one browser
//! identity: the usage page shows one card per account-usage provider, so
//! the DeepSeek card ("Mole"), collected for `claude-deepseek`,
//! `codex-deepseek` and `deepseek-harness` alike, has exactly one browser,
//! distinct from every other card and every named account. Its key is
//! `store::provider_accounts::legacy_browsers::account(owner, card)`.
//! See `docs/machine-browser.md`, "Legacy default cards".

use super::super::super::*;
use crate::provider_account::ProviderAccountId;
use crate::provider_service::ServiceAuthenticationState;
use crate::store::provider_accounts::legacy_browsers;
use serde_json::{Value, json};

/// A legacy default card as the browser routes see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::server) struct LegacyCard {
    /// Its account-usage provider id; `None` for a key known only from a
    /// retained placement (its Provider was uninstalled), which can still be
    /// stopped and reset but never opened.
    pub card: Option<String>,
    /// The Service collects this card now, so the usage page shows it.
    pub bound: bool,
    /// The owner enabled this card's browser.
    pub enabled: bool,
}

/// The cards the Service collects now, each with the Provider ids behind it.
pub(in crate::server) fn bound_cards(state: &AppState) -> BTreeMap<String, Vec<String>> {
    let mut cards: BTreeMap<String, Vec<String>> = state
        .usage
        .plugin_bindings()
        .into_iter()
        .map(|binding| (binding.account, Vec::new()))
        .collect();
    for entry in state.provider_catalog.entries() {
        if let Some(usage) = &entry.manifest.host.account_usage
            && let Some(providers) = cards.get_mut(&usage.provider)
            && !providers.contains(&entry.provider_id)
        {
            providers.push(entry.provider_id.clone());
        }
    }
    for providers in cards.values_mut() {
        providers.sort();
    }
    cards
}

/// The legacy card `id` names for `owner`, if it is one: a card the Service
/// collects or the owner enabled, else a key with a retained placement.
pub(in crate::server) async fn resolve(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<Option<LegacyCard>, &'static str> {
    if !legacy_browsers::is_account(id) {
        return Ok(None);
    }
    let store = state.store.as_ref().ok_or("unavailable")?;
    let enabled = store
        .legacy_browser_cards(owner)
        .await
        .map_err(|_| "unavailable")?;
    let bound = bound_cards(state);
    if let Some(card) = bound
        .keys()
        .chain(&enabled)
        .find(|card| legacy_browsers::account(owner, card) == *id)
    {
        return Ok(Some(LegacyCard {
            bound: bound.contains_key(card),
            enabled: enabled.contains(card),
            card: Some(card.clone()),
        }));
    }
    let placed = !store
        .browser_placements(owner, id)
        .await
        .map_err(|_| "unavailable")?
        .is_empty();
    Ok(placed.then_some(LegacyCard {
        card: None,
        bound: false,
        enabled: false,
    }))
}

/// Machines holding the card's default credentials: enrolled Machines with
/// an active installation of one of its Providers whose Service credentials
/// are ready (the Service distributes them to every enrolled Machine). This
/// plays the role a named account's native generations play. The Provider
/// execution policy does not apply: the browser runs no Provider CLI. An
/// unreadable inventory is `unavailable`, never "no credentials anywhere"
/// (which would admit the first-Machine chooser).
pub(in crate::server) async fn credential_machines(
    state: &AppState,
    card: &str,
) -> Result<Vec<String>, &'static str> {
    let installed: Vec<(String, String)> =
        super::super::super::provider_auth_flow::catalogued_installations(state, false, false)
            .await
            .map_err(|()| "unavailable")?
            .into_iter()
            .filter(|(_, installed)| {
                state
                    .provider_catalog
                    .account_usage_provider(
                        &installed.plugin_id,
                        &installed.plugin_version,
                        &installed.generation_digest,
                    )
                    .is_some_and(|usage| usage == card)
            })
            .map(|(machine, installed)| (machine, installed.plugin_id))
            .collect();
    Ok(machines_with_ready_credentials(&installed, |provider| {
        state
            .provider_auth
            .status(provider)
            .is_some_and(|status| status.authentication_state == ServiceAuthenticationState::Ready)
    }))
}

/// The sorted, distinct Machines among `installed` (Machine, Provider) pairs
/// whose Provider has ready Service credentials.
fn machines_with_ready_credentials(
    installed: &[(String, String)],
    ready: impl Fn(&str) -> bool,
) -> Vec<String> {
    let machines: std::collections::BTreeSet<&String> = installed
        .iter()
        .filter(|(_, provider)| ready(provider))
        .map(|(machine, _)| machine)
        .collect();
    machines.into_iter().cloned().collect()
}

/// The owner's legacy cards for the account list: every card the Service
/// collects and every card whose browser the owner enabled, with its
/// browser key, Providers and enablement.
pub(in crate::server) async fn list_value(
    state: &AppState,
    owner: &str,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let enabled = store
        .legacy_browser_cards(owner)
        .await
        .map_err(|_| "unavailable")?;
    let mut cards = bound_cards(state);
    let bound: Vec<String> = cards.keys().cloned().collect();
    for card in &enabled {
        cards.entry(card.clone()).or_default();
    }
    Ok(Value::Array(
        cards
            .into_iter()
            .map(|(card, providers)| {
                json!({
                    "account_id": legacy_browsers::account(owner, &card),
                    "providers": providers,
                    "browser_enabled": enabled.contains(&card),
                    "available": bound.contains(&card),
                    "card": card,
                })
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_place_a_card_only_where_its_providers_are_ready() {
        let installed = [
            ("ovh".to_owned(), "claude-deepseek".to_owned()),
            ("hawk".to_owned(), "codex-deepseek".to_owned()),
            ("hawk".to_owned(), "deepseek-harness".to_owned()),
            ("falcon".to_owned(), "deepseek-harness".to_owned()),
        ];
        assert_eq!(
            machines_with_ready_credentials(&installed, |provider| provider != "claude-deepseek"),
            vec!["falcon".to_owned(), "hawk".to_owned()]
        );
        // Signed out everywhere: no credential Machine, so the owner chooses
        // the card's first browser Machine.
        assert!(machines_with_ready_credentials(&installed, |_| false).is_empty());
        assert!(machines_with_ready_credentials(&[], |_| true).is_empty());
    }
}
