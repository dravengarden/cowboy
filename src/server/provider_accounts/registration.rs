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
    let (operation, completion) = store
        .native_login_for_session(owner, account, machine)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("native_login_required")?;
    store
        .with_native_login_binding(owner, &operation, &completion, || {
            resolve_provider_generation_with_auth(
                &state.provider_catalog,
                &state.plugin_catalog,
                inventory,
                provider,
                SchedulingAuthentication::Native(&completion.profile),
            )
            .map_err(anyhow::Error::msg)
        })
        .await
        .map_err(|error| error.to_string())
}

pub(in crate::server) async fn register(
    state: &AppState,
    registration: crate::core::SessionRegistration,
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
