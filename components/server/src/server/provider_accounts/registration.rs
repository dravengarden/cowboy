//! Shared account-aware registration for local-tool and split-execution sessions.
use super::*;

pub(in crate::server) async fn generation(
    state: &AppState,
    inventory: &[crate::machine_protocol::PluginInventory],
    provider: &str,
    owner: &str,
    account: &crate::provider_account::ProviderAccountId,
    machine: &str,
) -> Result<ResolvedProviderGeneration, String> {
    if account.is_default() {
        return resolve_provider_generation(
            &state.provider_catalog,
            &state.plugin_catalog,
            inventory,
            provider,
            state.provider_auth.status(provider).as_ref(),
        );
    }
    let store = state.store.as_ref().ok_or("account_store_unavailable")?;
    let Some((operation, completion)) = store
        .native_login_for_session(owner, account, machine)
        .await
        .map_err(|error| error.to_string())?
    else {
        let pending = store
            .native_login_pending(owner, account, machine)
            .await
            .unwrap_or(false);
        return Err(login_message(if pending {
            "native_login_pending"
        } else {
            "native_login_required"
        }));
    };
    store
        .with_native_login_binding(owner, &operation, &completion, || {
            resolve_provider_generation_with_auth(
                &state.provider_catalog,
                &state.plugin_catalog,
                inventory,
                provider,
                SchedulingAuthentication::Native(&completion.profile),
            )
            .map_err(|error| anyhow::Error::msg(login_message(&error)))
        })
        .await
        .map_err(|error| error.to_string())
}

/// Creation errors are shown to people; keep other codes for diagnostics.
fn login_message(code: &str) -> String {
    match code {
        "native_login_required" => {
            "This subscription is not signed in on the selected Machine. Sign in from Settings → Subscriptions.".into()
        }
        "native_login_pending" => {
            "A newer sign-in for this subscription has not finished on the selected Machine. Finish or cancel it in Settings → Subscriptions.".into()
        }
        "native_provider_generation_mismatch" => {
            "The Provider was updated on the selected Machine after this subscription signed in. Sign in again from Settings → Subscriptions.".into()
        }
        other => other.to_owned(),
    }
}

/// Shown when a fork cannot reuse its source's login: a named subscription's
/// generation advances only on a real sign-in, whose old tokens may be gone.
pub(in crate::server) const SIGNED_IN_AGAIN: &str =
    "This subscription was signed in again since the session started; use Continue in new agent";

/// Keep a creation on an older `retained` auth generation (a native fork runs
/// in its source's generation home). A Service-managed (default) account
/// advances its generation on every token refresh, and each retained home is
/// re-projected with the newest credentials, so the older generation is as
/// current as resuming the source. Named subscriptions advance only on a new
/// sign-in and are refused.
pub(in crate::server) fn retain_auth_generation(
    generation: &mut ResolvedProviderGeneration,
    account: &crate::provider_account::ProviderAccountId,
    retained: u64,
) -> Result<(), String> {
    if !retainable(generation.auth_generation, account.is_default(), retained) {
        return Err(SIGNED_IN_AGAIN.into());
    }
    generation.auth_generation = Some(retained);
    Ok(())
}

fn retainable(current: Option<u64>, service_account: bool, retained: u64) -> bool {
    current.is_some_and(|current| current == retained || (service_account && retained < current))
}

/// Register while `scheduling` — the Service generation creation resolved —
/// stays immutable; the session may record an older retained generation.
pub(in crate::server) async fn register(
    state: &AppState,
    registration: crate::core::SessionRegistration,
    scheduling: Option<u64>,
) -> Result<(), String> {
    let key = (
        registration.machine_id.clone(),
        registration.provider.clone(),
    );
    if registration.provider_account_id.is_default() {
        let fence = state.plugin_lifecycle_fences.read();
        if fence.contains_key(&key) {
            return Err("Provider is changing on the selected Machine".into());
        }
        let provider = registration.provider.clone();
        if let (Some(current), Some(retained)) = (scheduling, registration.provider_auth_generation)
            && current != retained
        {
            return state
                .provider_auth
                .with_retained_scheduling_generation(&provider, current, retained, || {
                    state.hub.create_session(registration)
                })
                .map_err(|error| error.to_string());
        }
        return state
            .provider_auth
            .with_scheduling_generation(
                &provider,
                registration.provider_auth_generation.is_some(),
                registration.provider_auth_generation,
                || state.hub.create_session(registration),
            )
            .map_err(|error| error.to_string());
    }
    let owner = registration
        .owner_user_id
        .clone()
        .ok_or("account_owner_required")?;
    let store = state.store.as_ref().ok_or("account_store_unavailable")?;
    let (connection, protocol) = state
        .machine_control
        .connection_protocol(&registration.machine_id)
        .ok_or("machine_unavailable")?;
    if protocol < crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION {
        return Err("named_account_protocol_unavailable".into());
    }
    let (operation, completion) = store
        .native_login_for_session(
            &owner,
            &registration.provider_account_id,
            &registration.machine_id,
        )
        .await
        .map_err(|error| error.to_string())?
        .ok_or("native_login_required")?;
    store
        .register_native_session_with_projection(
            &owner,
            &operation,
            &completion,
            crate::core::Hub::prepare_session(registration),
            |meta| {
                let fence = state.plugin_lifecycle_fences.read();
                anyhow::ensure!(
                    !fence.contains_key(&key),
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
                    state.hub.project_persisted_session(meta),
                    "session_projection_conflict"
                );
                Ok(())
            },
        )
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_fork_keeps_only_a_service_refresh_generation() {
        // Service (default) account: a token refresh advanced 76 -> 180.
        assert!(super::retainable(Some(180), true, 76));
        assert!(super::retainable(Some(76), true, 76));
        // Never ahead of the current generation, never without one.
        assert!(!super::retainable(Some(76), true, 180));
        assert!(!super::retainable(None, true, 76));
        // A named subscription moves only on a real sign-in.
        assert!(super::retainable(Some(6), false, 6));
        assert!(!super::retainable(Some(7), false, 6));
    }

    #[test]
    fn login_refusals_tell_people_how_to_recover() {
        for code in [
            "native_login_required",
            "native_login_pending",
            "native_provider_generation_mismatch",
        ] {
            let message = super::login_message(code);
            assert!(message.contains("Settings → Subscriptions"), "{message}");
        }
        assert_eq!(
            super::login_message("machine_unavailable"),
            "machine_unavailable"
        );
    }
}
