//! Receipt completion only: never sends a Machine mutation or stops a worker.
use super::*;
use crate::plugin_operation::resolution::{ResolutionIntent, can_complete_removal};

pub(in crate::server) async fn confirmed_reconcile_uninstall(
    state: Arc<AppState>,
    machine: String,
    plugin: String,
    operation: String,
    approval: OperatorApproval,
) -> Response {
    let result = reconcile(&state, &machine, &plugin, &operation, approval).await;
    match result {
        Ok(receipt) => no_store_json(StatusCode::OK, serde_json::json!({
            "operation_id": operation, "phase": "completed", "resolution": receipt,
            "machine_effect_replayed": false,
        })),
        Err(_) => (StatusCode::CONFLICT,
            "Uninstall receipt completion refused; inspect the original operation and Machine evidence").into_response(),
    }
}

async fn reconcile(
    state: &Arc<AppState>,
    machine: &str,
    plugin: &str,
    operation: &str,
    approval: OperatorApproval,
) -> Result<crate::plugin_operation::resolution::ResolutionReceipt> {
    let store = state
        .store
        .as_ref()
        .context("Service storage unavailable")?;
    ensure!(
        approval
            .current_operator(ProductRequestAuth::from(state.as_ref()))
            .await
            .as_ref()
            == Some(approval.actor()),
        "Operator authority ended"
    );
    let before = store
        .plugin_uninstall_operation(operation)
        .await?
        .context("operation missing")?;
    ensure!(
        before.intent.service_id == state.service_id
            && before.intent.machine_id == machine
            && before.intent.plugin_id == plugin,
        "operation identity changed"
    );
    if let Some(receipt) = store.plugin_uninstall_resolution(operation).await? {
        ensure!(receipt.matches_completed(&before)?
            && receipt.intent.action == crate::plugin_operation::resolution::ResolutionAction::CompleteVerifiedRemoval,
            "resolution evidence changed");
        return Ok(receipt);
    }
    ensure!(
        can_complete_removal(&before),
        "operation is not a completion candidate"
    );
    let fence = OperationFence::acquire_resolution(
        &state.plugin_lifecycle_fences,
        (machine.to_owned(), plugin.to_owned()),
    )?;
    let connection = state
        .machine_control
        .operation_connection(machine)
        .map_err(anyhow::Error::msg)?;
    let step = before.intent.machine_step()?;
    let evidence = state
        .machine_control
        .plugin_uninstall_recovery(&connection, &step)
        .await
        .map_err(|_| anyhow::anyhow!("Machine evidence unavailable"))?;
    let intent = ResolutionIntent::complete_removal(
        format!("complete-{:032x}", rand::random::<u128>()),
        approval.actor().clone(),
        &before,
        now_ms() + 60_000,
        evidence,
    )?;
    let permit = approval
        .authorize_resolution(
            ProductRequestAuth::from(state.as_ref()),
            &state.service_id,
            intent,
        )
        .await?;
    // The local transaction repeats the complete saved-operation CAS. No
    // session deletion is necessary or permitted for this zero-session action.
    // The admitted transaction survives HTTP observer cancellation. The shared
    // resolver also checks our durable result after an ambiguous COMMIT ACK.
    tokio::spawn(resolution::resolve_no_effect(store.clone(), permit, fence)).await?
}
