//! Revoking a portable account key: wipe the account vault, then collect an
//! authenticated wipe from every Machine enrolled when the vault was wiped.
//! Completion (in the Store) requires both; nothing here is completion.
use super::*;
use crate::store::provider_accounts::portable::{LocalWipe, RemoteWipe};

/// Wipe the account vault, returning its wipe generation (0: never keyed).
/// Rerunning after a crash reuses an existing wipe instead of advancing it.
fn wipe_vault(scope: Option<&ProviderAuthService>) -> anyhow::Result<u64> {
    let Some(scope) = scope else {
        return Ok(0);
    };
    let statuses = scope.replica_statuses();
    let keyed: Vec<String> = statuses
        .iter()
        .filter(|status| {
            status.authentication_state
                != crate::provider_service::ServiceAuthenticationState::SignedOut
        })
        .map(|status| status.provider_id.clone())
        .collect();
    if keyed.is_empty() {
        return Ok(statuses
            .iter()
            .map(|status| status.auth_generation)
            .max()
            .unwrap_or(0));
    }
    let providers: Vec<String> = statuses
        .into_iter()
        .map(|status| status.provider_id)
        .collect();
    Ok(scope
        .logout_scope(&providers)?
        .iter()
        .map(|status| status.auth_generation)
        .max()
        .unwrap_or(0))
}

/// One bounded pass. Offline Machines stay pending for the recovery loop.
pub(in crate::server) async fn run(
    state: &Arc<AppState>,
    owner: &str,
    operation: &str,
) -> anyhow::Result<Value> {
    let store = state.store.as_ref().context("store unavailable")?;
    let receipt = store
        .provider_account_receipt(owner, operation)
        .await?
        .context("revocation operation missing")?;
    if !receipt
        .revocation
        .as_ref()
        .is_some_and(|plan| plan.portable_credentials)
    {
        return Ok(json!({"state":"none"}));
    }
    let local = match receipt.portable_wipe.clone() {
        Some(local) => local,
        None => {
            let scope = state.provider_auth.existing_account(&receipt.account_id);
            let generation = wipe_vault(scope.as_deref())?;
            store
                .record_portable_local_wipe(owner, operation, generation)
                .await?
        }
    };
    let mut machines = Vec::new();
    for target in &local.machines {
        let state_name = if receipt
            .portable_remote_wipes
            .contains_key(&target.machine_id)
        {
            "wipe_recorded"
        } else {
            match deliver(state, owner, operation, &receipt.account_id, &local, target).await {
                Ok(_) => "wipe_recorded",
                Err(_) => "pending",
            }
        };
        machines.push(json!({"machine":target.machine_id,"state":state_name}));
    }
    Ok(json!({"vault_wipe_generation":local.generation,"machines":machines}))
}

async fn deliver(
    state: &Arc<AppState>,
    owner: &str,
    operation: &str,
    account: &ProviderAccountId,
    local: &LocalWipe,
    target: &crate::store::provider_accounts::portable::Target,
) -> anyhow::Result<RemoteWipe> {
    let store = state.store.as_ref().context("store unavailable")?;
    let (connection, _) = state
        .machine_control
        .connection_protocol(&target.machine_id)
        .context("machine offline")?;
    let key = store
        .machine_encryption_public_key(&target.machine_id)
        .await?
        .context("machine unavailable")?;
    anyhow::ensure!(
        crate::admin::hex_sha256(key.as_bytes()) == target.recipient_fingerprint,
        "machine_enrollment_changed"
    );
    let scope = state
        .provider_auth
        .existing_account(account)
        .context("portable vault missing")?;
    let mut acknowledged = u64::MAX;
    for status in scope.replica_statuses() {
        let envelope = scope.seal_for_machine(&status.provider_id, &key)?;
        anyhow::ensure!(
            envelope.action == crate::machine_protocol::ProviderAuthAction::Wipe
                && envelope.auth_generation >= local.generation,
            "portable vault is not wiped"
        );
        let generation = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            state
                .provider_auth_sync
                .apply(&state.machine_control, &connection, envelope),
        )
        .await
        .context("wipe acknowledgement pending")?
        .map_err(anyhow::Error::msg)?;
        acknowledged = acknowledged.min(generation);
    }
    anyhow::ensure!(acknowledged < u64::MAX, "portable vault has no entries");
    store
        .record_portable_remote_wipe(
            owner,
            operation,
            &RemoteWipe {
                machine_id: target.machine_id.clone(),
                recipient_fingerprint: target.recipient_fingerprint.clone(),
                generation: acknowledged,
            },
            || state.machine_control.is_current(&connection),
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vault_wipe_is_one_generation_and_replays_without_advancing() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderAuthService::open(root.path()).unwrap();
        assert_eq!(wipe_vault(None).unwrap(), 0);
        let id: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let scope = service.account(&id).unwrap();
        assert_eq!(wipe_vault(Some(&scope)).unwrap(), 0);
        let packages: Vec<_> = [
            include_str!("../../../../../../plugins/claude-deepseek/provider.json"),
            include_str!("../../../../../../plugins/codex-deepseek/provider.json"),
        ]
        .into_iter()
        .map(|source| {
            let source: cowboy_provider_sdk::StandardProviderSource =
                serde_json::from_str(source).unwrap();
            cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap()
        })
        .collect();
        let bundle = secret_bundle(&packages[0], "api-key", "sk-revoked").unwrap();
        scope
            .commit_shared(&packages, &bundle, None, "claude-deepseek", Some(0))
            .unwrap();
        assert_eq!(wipe_vault(Some(&scope)).unwrap(), 2);
        assert_eq!(wipe_vault(Some(&scope)).unwrap(), 2);
        for status in scope.replica_statuses() {
            assert_eq!(
                status.authentication_state,
                crate::provider_service::ServiceAuthenticationState::SignedOut
            );
            let machine = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([4; 32]));
            let key = base64::engine::general_purpose::STANDARD.encode(machine.as_bytes());
            let envelope = scope.seal_for_machine(&status.provider_id, &key).unwrap();
            assert_eq!(
                envelope.action,
                crate::machine_protocol::ProviderAuthAction::Wipe
            );
        }
    }
}
