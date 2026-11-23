//! Immediate banked reset of a named portable account (Codex): the same
//! admission as the account's usage (placement, the account lock with the
//! exact vault generation and admission revision), on a Machine speaking
//! protocol 51, and only with an exact selected release whose collector
//! declares resets and a refresh-free reset path. Its `consume_reset` runs
//! with `"token_refresh": false`, so an expired access token reports
//! authentication required and never redeems the refresh token.
use super::*;
use crate::machine_protocol::portable_usage::{
    PORTABLE_RESET_PROTOCOL_VERSION, PortableResetRequest,
};

/// The route of a reset of this owner's portable account now.
pub fn route(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
    target: super::super::portable::Target,
) -> Result<Route, &'static str> {
    let placed = super::super::portable::placement(state, owner, id, &target)?;
    route_placed(state, target, placed)
}

/// [`route`] for an already resolved usage placement.
pub fn route_placed(
    state: &AppState,
    target: super::super::portable::Target,
    (machine, installed, sidecars): super::super::portable::Placement,
) -> Result<Route, &'static str> {
    if !state.usage.limiter.holds(1) {
        return Err("account_reset_unsupported");
    }
    // A reset needs the account's verified subscription identity: an adopted
    // account without one could spend another subscription's grant.
    if target.scope.identity_fingerprint.is_none() {
        return Err("portable_reset_identity_required");
    }
    // A reset runs only the collector; never a sidecar Plugin.
    if !sidecars.is_empty() {
        return Err("account_reset_unsupported");
    }
    if !state
        .machine_control
        .machine_supports(&machine, PORTABLE_RESET_PROTOCOL_VERSION)
    {
        return Err("machine_usage_update_required");
    }
    let binding = exact_reset_binding(state, &installed, &target.binding.account)
        .filter(|_| state.usage.refresh_free_reset(&installed))
        .ok_or("account_reset_unsupported")?;
    Ok(Route {
        target: RouteTarget::Portable {
            target: Box::new(target),
            machine,
        },
        binding: Box::new(binding),
        installed: Box::new(installed),
    })
}

/// Enqueue the reset under the owner's account lock, which rechecks the
/// account's availability, admission revision and identity and the vault's
/// exact generation. Once enqueued, only its reply decides the outcome.
pub async fn dispatch(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
    (target, machine): (&super::super::portable::Target, &String),
    route: &Route,
    reset: crate::machine_protocol::NativeResetRequest,
    expires_at_ms: i64,
) -> Result<Value, NativeResetFailure> {
    let request = PortableResetRequest {
        account_id: id.clone(),
        plugin_id: route.installed.plugin_id.clone(),
        plugin_version: route.installed.plugin_version.clone(),
        generation_digest: route.installed.generation_digest.clone(),
        auth_generation: target.auth_generation,
        reset,
        expires_at_ms,
    };
    let (pending, code) =
        super::super::portable::under_account_lock(state, owner, id, target, || {
            state
                .machine_control
                .begin_portable_reset(machine, &route.installed, request)
                .map_err(|error| crate::usage::portable::dispatch_error(&error))
        })
        .await;
    match pending {
        Some(pending) => pending.await,
        None => Err(unsent(code)),
    }
}
