//! Explicit resolution of a proven pre-effect interruption. No Machine port,
//! Provider vault, Catalog lookup, worker command or generic clear-fence API.

use super::*;
use crate::operation_budget::{OperationBudget, TimeSample};
use crate::plugin_operation::Operation;
use crate::plugin_operation::resolution::{
    ResolutionAction, ResolutionIntent, ResolutionPermit, ResolutionReceipt,
    can_abort_before_effects,
};
use std::time::Duration;

struct Preview {
    intent: ResolutionIntent,
    budget: OperationBudget,
}

#[derive(Default)]
pub(in crate::server) struct ResolutionPlans {
    plans: parking_lot::Mutex<HashMap<String, Preview>>,
}

impl ResolutionPlans {
    fn insert(&self, intent: ResolutionIntent) -> Result<()> {
        intent.validate()?;
        let mut plans = self.plans.lock();
        plans.retain(|_, plan| !plan.budget.expired());
        ensure!(plans.len() < 256, "resolution preview capacity exhausted");
        ensure!(
            !plans.contains_key(&intent.resolution_id),
            "resolution preview identity reused"
        );
        let budget = OperationBudget::new(
            intent.expires_at_ms,
            Duration::from_mins(2),
            TimeSample::now(),
        );
        ensure!(!budget.expired(), "resolution preview expired");
        plans.insert(intent.resolution_id.clone(), Preview { intent, budget });
        Ok(())
    }

    fn consume(
        &self,
        actor: &Actor,
        service: &str,
        target: &(String, String, String),
        request: &Confirmation,
    ) -> Result<ResolutionIntent> {
        let mut plans = self.plans.lock();
        let preview = plans
            .get(&request.plan_id)
            .context("resolution preview missing or consumed")?;
        let intent = &preview.intent;
        ensure!(
            &intent.actor == actor
                && intent.service_id == service
                && intent.machine_id == target.0
                && intent.plugin_id == target.1
                && intent.operation_id == target.2
                && intent.action == request.action
                && !preview.budget.expired(),
            "resolution preview owner, action, target or expiry changed"
        );
        Ok(plans
            .remove(&request.plan_id)
            .expect("validated under the same lock")
            .intent)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct Confirmation {
    plan_id: String,
    action: ResolutionAction,
}

pub(super) fn candidates(operation: &Operation) -> Vec<ResolutionAction> {
    if can_abort_before_effects(operation) {
        vec![ResolutionAction::AbortBeforeEffects]
    } else {
        vec![]
    }
}

async fn owned_operation(
    state: &AppState,
    target: &(String, String, String),
) -> Result<Operation, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .plugin_uninstall_operation(&target.2)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .filter(|op| {
            op.intent.service_id == state.service_id
                && op.intent.machine_id == target.0
                && op.intent.plugin_id == target.1
        })
        .ok_or(StatusCode::NOT_FOUND)
}

pub(in crate::server) async fn api_resolution_plan(
    State(state): State<Arc<AppState>>,
    Path(target): Path<(String, String, String)>,
    authenticated: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
) -> Response {
    let actor = match request_actor(&state, authenticated, &headers) {
        Ok(actor) => actor,
        Err(status) => return status.into_response(),
    };
    let operation = match owned_operation(&state, &target).await {
        Ok(operation) => operation,
        Err(status) => return status.into_response(),
    };
    if state
        .plugin_lifecycle_fences
        .read()
        .get(&(target.0, target.1))
        != Some(&PluginFenceState::NeedsReconcile)
    {
        return (
            StatusCode::CONFLICT,
            "Operation is not available for resolution",
        )
            .into_response();
    }
    let result =
        async {
            let intent = ResolutionIntent::new(
                random_machine_token()?,
                actor,
                &operation,
                now_ms().saturating_add(120_000),
            )?;
            // Minting an expiring preview is not authority and performs no durable
            // write. The response never includes actors, session IDs or credentials.
            state.plugin_resolution_plans.insert(intent.clone())?;
            Ok::<_, anyhow::Error>(Json(serde_json::json!({
            "schema": 1, "plan_id": intent.resolution_id, "operation_id": intent.operation_id,
            "machine_id": intent.machine_id, "plugin_id": intent.plugin_id,
            "plugin_version": operation.intent.plugin_version,
            "action": intent.action, "expires_at_ms": intent.expires_at_ms,
            "service_phase": operation.phase, "attention_from": operation.attention_from,
            "affected_session_count": operation.intent.session_ids.len(),
            "requires_confirmation": true,
            "plugin_mutation_performed": false, "session_mutation_performed": false,
            "worker_restoration_performed": false,
        })).into_response())
        }
        .await;
    result.unwrap_or_else(|_| {
        (
            StatusCode::CONFLICT,
            "Only interrupted operations proven to have stopped before effects can be resolved",
        )
            .into_response()
    })
}

fn receipt_response(receipt: &ResolutionReceipt) -> Response {
    Json(serde_json::json!({
        "schema": 1, "operation_id": receipt.intent.operation_id,
        "resolution_id": receipt.intent.resolution_id, "action": receipt.intent.action,
        "phase": Phase::Aborted, "resolved_at_ms": receipt.resolved_at_ms,
        "plugin_mutation_performed": false, "session_mutation_performed": false,
        "worker_restoration_performed": false,
    }))
    .into_response()
}

async fn resolve_no_effect(
    store: Store,
    permit: ResolutionPermit,
    mut fence: OperationFence,
) -> Result<ResolutionReceipt> {
    let result = store.resolve_plugin_uninstall(&permit).await;
    let receipt = match result {
        Ok(receipt) => receipt,
        Err(error) => {
            // A COMMIT response may be lost. Query OUR exact durable resolution,
            // never retry the mutation or infer it from an Aborted phase alone.
            let Some(saved) = store
                .plugin_uninstall_resolution(&permit.intent().operation_id)
                .await?
            else {
                return Err(error);
            };
            ensure!(
                &saved.intent == permit.intent(),
                "another resolution owns this result"
            );
            let operation = store
                .plugin_uninstall_operation(&saved.intent.operation_id)
                .await?
                .context("resolution operation unavailable")?;
            ensure!(
                saved.matches_completed(&operation)?,
                "resolution result is inconsistent"
            );
            saved
        }
    };
    // Completion of this admitted local effect; not a new restoration grant.
    // If result observation fails or the task is interrupted, Drop retains the
    // memory fence. Startup reconstructs it from the actual durable phase.
    fence.finish(Phase::Aborted);
    Ok(receipt)
}

pub(in crate::server) async fn api_resolve(
    State(state): State<Arc<AppState>>,
    Path(target): Path<(String, String, String)>,
    authenticated: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
    Json(request): Json<Confirmation>,
) -> Response {
    let approval = match OperatorApproval::capture(
        ProductRequestAuth::from(state.as_ref()),
        &state.service_id,
        authenticated.as_ref().map(|Extension(auth)| auth),
        &headers,
    ) {
        Ok(approval) => approval,
        Err(status) => return status.into_response(),
    };
    let Some(store) = state.store.clone() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let result = async {
        let intent = state.plugin_resolution_plans.consume(
            approval.actor(),
            &state.service_id,
            &target,
            &request,
        )?;
        let fence = OperationFence::acquire_resolution(
            &state.plugin_lifecycle_fences,
            (target.0, target.1),
        )?;
        let permit = approval
            .authorize_resolution(
                ProductRequestAuth::from(state.as_ref()),
                &state.service_id,
                intent,
            )
            .await?;
        // HTTP cancellation ends only observation; shutdown leaves either the
        // atomic receipt+Aborted pair or the original NeedsAttention record.
        tokio::spawn(resolve_no_effect(store, permit, fence)).await?
    }
    .await;
    match result {
        Ok(receipt) => receipt_response(&receipt),
        Err(_) => (StatusCode::CONFLICT,
            "Resolution did not return a verified result; inspect the resolution receipt and operation before retrying").into_response(),
    }
}

pub(in crate::server) async fn api_resolution_receipt(
    State(state): State<Arc<AppState>>,
    Path(target): Path<(String, String, String)>,
) -> Response {
    let operation = match owned_operation(&state, &target).await {
        Ok(operation) => operation,
        Err(status) => return status.into_response(),
    };
    let result = state
        .store
        .as_ref()
        .expect("owned operation requires storage")
        .plugin_uninstall_resolution(&operation.intent.operation_id)
        .await;
    match result {
        Ok(Some(receipt)) if receipt.matches_completed(&operation).unwrap_or(false) => {
            receipt_response(&receipt)
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Resolution evidence is unavailable",
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests;
