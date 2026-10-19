//! Persist signed remote wipe observations, never upstream logout.
use super::*;
use serde_json::{Value, json};

pub(super) async fn run(
    state: &Arc<super::super::super::AppState>,
    owner: &str,
    operation: &str,
) -> Result<Vec<Value>> {
    let store = state.store.as_ref().context("store unavailable")?;
    let receipt = store
        .provider_account_receipt(owner, operation)
        .await?
        .context("revocation operation missing")?;
    let plan = receipt.revocation.context("revocation plan missing")?;
    Ok(batch::bounded_pass(&plan.grants, |target| async {
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            deliver(state, owner, operation, target),
        )
        .await;
        let status = match result {
            Ok(Ok(_)) => "wipe_recorded",
            Ok(Err(_)) => "unconfirmed",
            Err(_) => "pending",
        };
        json!({"operation":target.operation_id,"machine":target.machine_id,"state":status})
    })
    .await)
}

async fn deliver(
    state: &Arc<super::super::super::AppState>,
    owner: &str,
    operation: &str,
    target: &crate::store::provider_accounts::revocation::OperationTarget,
) -> Result<u64> {
    let store = state.store.as_ref().context("store unavailable")?;
    let receipt = store
        .provider_account_receipt(owner, operation)
        .await?
        .context("revocation operation missing")?;
    if let Some(recorded) = receipt.remote_wipes.get(&target.operation_id) {
        anyhow::ensure!(
            recorded.machine_id == target.machine_id,
            "remote wipe scope mismatch"
        );
        return Ok(recorded.generation);
    }
    let (connection, protocol) = state
        .machine_control
        .connection_protocol(&target.machine_id)
        .context("machine offline")?;
    anyhow::ensure!(
        protocol >= crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION,
        "named account protocol unavailable"
    );
    let key = store
        .machine_encryption_public_key(&target.machine_id)
        .await?
        .context("machine unavailable")?;
    let envelope = store
        .with_provider_grant_revocation(owner, operation, &target.operation_id, |grant| {
            state
                .provider_auth
                .existing_revocation_grant(grant)?
                .seal_revoked_grant(grant, &key)
        })
        .await?;
    let generation = state
        .provider_auth_sync
        .apply(&state.machine_control, &connection, envelope)
        .await
        .map_err(anyhow::Error::msg)?;
    store
        .record_provider_remote_wipe(
            owner,
            operation,
            &crate::store::provider_accounts::revocation::remote_wipes::Evidence {
                grant_operation: target.operation_id.clone(),
                machine_id: target.machine_id.clone(),
                recipient_fingerprint: crate::admin::hex_sha256(key.as_bytes()),
                generation,
            },
            || state.machine_control.is_current(&connection),
        )
        .await?;
    Ok(generation)
}
