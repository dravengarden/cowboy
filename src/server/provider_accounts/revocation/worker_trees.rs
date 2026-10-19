//! Read-only Machine observations, admitted under the original stop scope.
use super::{Arc, Result, batch};
use anyhow::Context as _;
use serde_json::{Value, json};

pub(super) async fn run(
    state: &Arc<crate::server::AppState>,
    owner: &str,
    operation: &str,
) -> Result<Value> {
    let store = state.store.as_ref().context("store unavailable")?;
    let receipt = store
        .provider_account_receipt(owner, operation)
        .await?
        .context("revocation missing")?;
    let plan = receipt.revocation.context("revocation plan missing")?;
    Ok(json!(batch::bounded_pass(&plan.sessions, |target| async {
        let result = observe(state, owner, operation, &target.session_id).await;
        json!({"session": target.session_id, "state": if result.is_ok() { "tree_exit_recorded" } else { "unconfirmed" }, "evidence": result.ok()})
    }).await))
}

async fn observe(
    state: &Arc<crate::server::AppState>,
    owner: &str,
    operation: &str,
    session: &str,
) -> Result<crate::provider_account::WorkerExitObservation> {
    let store = state.store.as_ref().context("store unavailable")?;
    let mut evidence = store
        .provider_account_stop(owner, operation, session)
        .await?
        .context("session stop unconfirmed")?;
    if let Some(tree) = evidence.tree_exit {
        return Ok(tree);
    }
    let key = store
        .machine_encryption_public_key(&evidence.machine_id)
        .await?
        .context("machine unavailable")?;
    anyhow::ensure!(
        crate::admin::hex_sha256(key.as_bytes()) == evidence.recipient_fingerprint,
        "machine enrollment changed"
    );
    let (connection, protocol) = state
        .machine_control
        .connection_protocol(&evidence.machine_id)
        .context("machine offline")?;
    anyhow::ensure!(
        protocol >= crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION,
        "account protocol unavailable"
    );
    let tree = state
        .machine_control
        .query_worker_exit(
            &connection,
            &evidence.machine_id,
            &crate::provider_account::WorkerExitRequest {
                stop_command_id: None,
                account_id: evidence.account_id.clone(),
                session_id: session.into(),
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;
    evidence.tree_exit = Some(tree.clone());
    store
        .record_provider_account_stop(owner, operation, &evidence, || {
            state.machine_control.is_current(&connection)
        })
        .await?;
    Ok(tree)
}
