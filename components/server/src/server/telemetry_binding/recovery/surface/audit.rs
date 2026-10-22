//! No process-local handle or replicated recovery journal. Machine owns audit;
//! Service retains the original operation and its resolution's exact before.
use super::*;
use crate::machine_protocol::telemetry_recovery_audit::{
    RecoveryAuditObservation, RecoveryAuditQuery,
};
use crate::telemetry_binding::Ledger;

#[derive(serde::Serialize)]
pub(super) struct AuditView {
    schema: u16,
    operation: OperationView,
    recovery: Option<RecordedRecovery>,
}

#[derive(serde::Serialize)]
struct RecordedRecovery {
    before: OperationView,
    receipt: ReceiptView,
}

impl AuditView {
    pub(super) fn new(
        ledger: &Ledger,
        operation: &Operation,
        query: &RecoveryAuditQuery,
        observation: &RecoveryAuditObservation,
    ) -> Result<Self> {
        ensure!(
            operation.intent.machine_step()? == query.step && observation.matches(query),
            "audit query changed"
        );
        let RecoveryAuditObservation::Observed { snapshot } = observation else {
            anyhow::bail!("Machine audit unavailable");
        };
        let recovery = snapshot
            .receipt
            .as_ref()
            .map(|receipt| {
                // A Service resolution changes the operation digest. Its retained
                // before, not the new terminal phase, must match Machine's audit.
                let before = ledger
                    .resolutions
                    .iter()
                    .find(|record| record.intent.operation_id == operation.intent.operation_id)
                    .map_or(operation, |record| &record.before);
                ensure!(
                    matches!(before.progress, Progress::NeedsAttention { .. })
                        && before.intent == operation.intent
                        && binding_digest(&serde_json::to_vec(before)?)
                            == receipt.request.service_operation_digest,
                    "Machine audit does not belong to the retained Service operation"
                );
                match &operation.progress {
                    Progress::NeedsAttention { .. } => {}
                    Progress::Rejected {
                        observation: BindingObservation::Observed { snapshot },
                    } if snapshot.receipt.as_deref() == Some(&receipt.binding) => {}
                    _ => anyhow::bail!("Service conclusion conflicts with Machine recovery audit"),
                }
                Ok(RecordedRecovery {
                    before: OperationView::new(before)?,
                    receipt: ReceiptView::from_audit(receipt)?,
                })
            })
            .transpose()?;
        Ok(Self {
            schema: 1,
            operation: OperationView::new(operation)?,
            recovery,
        })
    }
}

async fn read(
    state: &ApiState,
    operation: &str,
    approval: &OperatorApproval,
    budget: &OperationBudget,
) -> Result<AuditView, ApiError> {
    let ledger = state.ledger().await?.ok_or(ApiError::NotFound)?;
    let original = ledger
        .operations
        .iter()
        .find(|op| op.intent.operation_id == operation)
        .ok_or(ApiError::NotFound)?;
    let query = RecoveryAuditQuery {
        schema: 1,
        step: original
            .intent
            .machine_step()
            .map_err(|_| ApiError::EvidenceUnavailable)?,
    };
    if budget.expired() {
        return Err(ApiError::EvidenceUnavailable);
    }
    let connection = state
        .control
        .operation_connection(&query.step.machine_id)
        .map_err(|_| ApiError::EvidenceUnavailable)?;
    let observation = tokio::time::timeout(
        budget.remaining(),
        state.control.telemetry_recovery_audit(&connection, &query),
    )
    .await
    .map_err(|_| ApiError::EvidenceUnavailable)?
    .map_err(|_| ApiError::EvidenceUnavailable)?;
    let retained = state.ledger().await?.ok_or(ApiError::EvidenceUnavailable)?;
    let current = retained
        .operations
        .iter()
        .find(|op| op.intent.operation_id == operation)
        .ok_or(ApiError::EvidenceUnavailable)?;
    if approval.current_operator(state.auth()).await.as_ref() != Some(approval.actor()) {
        return Err(ApiError::Authorization(StatusCode::UNAUTHORIZED));
    }
    if budget.expired()
        || !state
            .control
            .telemetry_recovery_audit_target_current(&connection, &query)
    {
        return Err(ApiError::EvidenceUnavailable);
    }
    AuditView::new(&retained, current, &query, &observation)
        .map_err(|_| ApiError::EvidenceUnavailable)
}

pub(super) async fn inspect(
    State(state): State<ApiState>,
    Path(operation): Path<String>,
    verified: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
) -> Result<Json<AuditView>, ApiError> {
    let received = TimeSample::now();
    let budget = OperationBudget::new(
        chrono::Utc::now().timestamp_millis().saturating_add(10_000),
        Duration::from_secs(10),
        received,
    );
    tokio::time::timeout(budget.remaining(), async {
        let approval = state
            .approve(verified.as_ref().map(|Extension(v)| v), &headers)
            .await?;
        read(&state, &operation, &approval, &budget).await.map(Json)
    })
    .await
    .map_err(|_| ApiError::EvidenceUnavailable)?
}

pub(super) async fn find_receipt(
    state: &ApiState,
    operation: &str,
    resolution: &str,
    approval: &OperatorApproval,
    budget: &OperationBudget,
) -> Result<ReceiptView, ApiError> {
    read(state, operation, approval, budget)
        .await?
        .recovery
        .map(|recovery| recovery.receipt)
        .filter(|receipt| receipt.resolution_id == resolution)
        .ok_or(ApiError::OutcomeUnverified)
}

#[cfg(test)]
mod tests;
