//! The network policy an account's Machine browser unit enforces (R3-15,
//! `docs/antigravity-real-accounts-design.md`, "Sign-in").
use super::{BrowserAccount, Resolved};
use crate::machine_protocol::machine_browser::BrowserNetwork;
use crate::server::AppState;
use cowboy_provider_sdk::RuntimeNetworkPolicy;

/// `ipv4_only_v1` when the account's Provider, as installed on `machine_id`,
/// declares it. A named-only Provider whose installed manifest cannot be
/// resolved is restricted too: its browser never runs unrestricted before
/// the first sign-in.
pub(super) async fn policy(
    state: &AppState,
    resolved: &Resolved,
    machine_id: &str,
) -> Option<BrowserNetwork> {
    let BrowserAccount::Named(account) = &resolved.account else {
        return None;
    };
    let declared = match state.store.as_ref() {
        Some(store) => store.list_machines().await.ok().and_then(|machines| {
            let installed = crate::server::plugin_api::active_machine_plugin(
                &machines,
                machine_id,
                &account.provider_id,
            )
            .ok()?;
            let package = state.provider_catalog.package(
                &account.provider_id,
                &installed.plugin_version,
                &installed.generation_digest,
            )?;
            Some(package.manifest.runtime.network == Some(RuntimeNetworkPolicy::Ipv4OnlyV1))
        }),
        None => None,
    };
    declared
        .unwrap_or_else(|| crate::provider_account::named_only(&account.provider_id))
        .then_some(BrowserNetwork::Ipv4OnlyV1)
}
