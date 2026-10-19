//! Read-only local login cleanup observations. Never whole-account completion.
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
    Ok(batch::bounded_pass(&plan.logins, |target| async {
        if store.provider_login_receipt(owner, &target.operation_id).await.ok().flatten().is_some_and(|login|
            login.input.account_id == receipt.account_id && login.input.machine_id == target.machine_id
                && login.cancel_requested && login.never_dispatched()) {
            return json!({"operation": target.operation_id, "machine": target.machine_id, "state": "not_dispatched", "evidence": null});
        }
        let observed = match receipt.login_stops.get(&target.operation_id) {
            Some(saved) => Ok(saved.observation.clone()),
            None => observe(state, owner, operation, &receipt.account_id, target).await,
        };
        json!({"operation": target.operation_id, "machine": target.machine_id,
            "state": if observed.is_ok() { "cleanup_recorded" } else { "unconfirmed" },
            "evidence": observed.ok()})
    })
    .await)
}

async fn observe(
    state: &Arc<super::super::super::AppState>,
    owner: &str,
    operation: &str,
    account: &crate::provider_account::ProviderAccountId,
    target: &crate::store::provider_accounts::revocation::OperationTarget,
) -> Result<crate::provider_account::LoginStopObservation> {
    let store = state.store.as_ref().context("store unavailable")?;
    let login = store
        .provider_login_receipt(owner, &target.operation_id)
        .await?
        .context("login missing")?;
    anyhow::ensure!(
        login.input.account_id == *account
            && login.input.machine_id == target.machine_id
            && login.cancel_requested
            && login.native_generation.is_none(),
        "login scope mismatch"
    );
    let key = store
        .machine_encryption_public_key(&target.machine_id)
        .await?
        .context("machine unavailable")?;
    let fingerprint = crate::admin::hex_sha256(key.as_bytes());
    anyhow::ensure!(
        login.dispatch_recipient_fingerprint.as_ref() == Some(&fingerprint),
        "machine enrollment changed"
    );
    let (connection, protocol) = state
        .machine_control
        .connection_protocol(&target.machine_id)
        .context("machine offline")?;
    anyhow::ensure!(
        protocol >= crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION,
        "account protocol unavailable"
    );
    let observation = state
        .machine_control
        .query_login_stop(
            &connection,
            &target.machine_id,
            &crate::provider_account::LoginStopRequest {
                account_id: account.clone(),
                provider_id: login.provider_id,
                login_request_id: login.request_id,
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;
    let saved = store
        .record_provider_login_stop(
            owner,
            operation,
            &crate::store::provider_accounts::revocation::login_stops::Evidence {
                login_operation: target.operation_id.clone(),
                machine_id: target.machine_id.clone(),
                recipient_fingerprint: fingerprint,
                observation,
            },
            || state.machine_control.is_current(&connection),
        )
        .await?;
    Ok(saved.observation)
}
