//! Exact, one-use Machine recovery confirmation. A saved request is query data,
//! never a renewed permit. No Service resolution is chained to this action.
use super::super::http::{ApiError, ApiState};
use super::*;
use crate::machine_protocol::telemetry_binding::{BindingDigest, binding_digest};
use crate::machine_protocol::telemetry_recovery::{RecoveryAction, RecoveryActor, prepared};
use crate::operation_budget::{OperationBudget, TimeSample};
use crate::server::telemetry_binding::resolution::surface::view::OperationView;
use crate::server::{AuthenticatedProductRequest, operator_approval::OperatorApproval};
use anyhow::Context;
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, FromRef, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

struct Preview {
    request: RecoveryRequest,
    budget: Option<OperationBudget>,
    // Bounded process-local query handle. Restart discards it, not Machine audit.
    retain_until: Instant,
}

#[derive(Default)]
pub(in crate::server) struct Plans(parking_lot::Mutex<HashMap<String, Preview>>);

impl Plans {
    fn insert(&self, request: RecoveryRequest, budget: OperationBudget) -> Result<()> {
        request.validate()?;
        let mut plans = self.0.lock();
        plans.retain(|_, p| {
            Instant::now() < p.retain_until && p.budget.as_ref().is_none_or(|b| !b.expired())
        });
        ensure!(
            plans.len() < 256 && !plans.contains_key(&request.resolution_id) && !budget.expired(),
            "preview unavailable"
        );
        plans.insert(
            request.resolution_id.clone(),
            Preview {
                request,
                budget: Some(budget),
                retain_until: Instant::now() + Duration::from_hours(1),
            },
        );
        Ok(())
    }

    fn owned<'a>(
        plans: &'a mut HashMap<String, Preview>,
        actor: &RecoveryActor,
        service: &str,
        operation: &str,
        id: &str,
    ) -> Result<&'a mut Preview> {
        let p = plans.get_mut(id).context("preview unavailable")?;
        ensure!(
            &p.request.actor == actor
                && p.request.step.service_id == service
                && p.request.step.operation_id == operation
                && Instant::now() < p.retain_until,
            "preview owner changed"
        );
        Ok(p)
    }

    fn consume(
        &self,
        actor: &RecoveryActor,
        service: &str,
        operation: &str,
        confirmation: &Confirmation,
    ) -> Result<(RecoveryRequest, OperationBudget)> {
        let mut plans = self.0.lock();
        let p = Self::owned(&mut plans, actor, service, operation, &confirmation.plan_id)?;
        ensure!(
            p.request.action == confirmation.action
                && p.budget.as_ref().is_some_and(|b| !b.expired()),
            "preview ended"
        );
        Ok((
            p.request.clone(),
            p.budget.take().expect("checked under same lock"),
        ))
    }

    fn submitted(
        &self,
        actor: &RecoveryActor,
        service: &str,
        operation: &str,
        id: &str,
    ) -> Result<RecoveryRequest> {
        let mut plans = self.0.lock();
        let p = Self::owned(&mut plans, actor, service, operation, id)?;
        ensure!(p.budget.is_none(), "request not submitted");
        Ok(p.request.clone())
    }
}

impl ApiState {
    fn recovery_admitted(&self) -> bool {
        #[cfg(test)]
        if self.fixture_recovery_admission {
            return true;
        }
        RECOVERY_WRITE_ADMISSION
    }

    async fn complete_recovery(
        self,
        request: RecoveryRequest,
        budget: OperationBudget,
        approval: OperatorApproval,
    ) -> Result<ReceiptView> {
        ensure!(
            self.recovery_admitted(),
            "Machine recovery admission closed"
        );
        let ledger = self
            .ledger()
            .await
            .map_err(|_| anyhow::anyhow!("journal unavailable"))?
            .context("missing journal")?;
        let before = ledger.operations.last().context("missing operation")?;
        let authority = approval
            .bind_telemetry_recovery(&request, before)?
            .constrain_to_preview(budget);
        let store = self.store.as_ref().context("missing store")?;
        #[cfg(test)]
        if self.fixture_recovery_admission {
            let live = Live::bind(self.control.clone(), &request)?;
            let observed = coordinate(store, &request, authority, self.auth(), &live).await?;
            return ReceiptView::new(&request, &observed);
        }
        let observed = recover_machine(
            store,
            &request,
            authority,
            self.auth(),
            self.control.clone(),
        )
        .await?;
        ReceiptView::new(&request, &observed)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Inspect {}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirmation {
    plan_id: String,
    action: RecoveryAction,
}

#[derive(serde::Serialize)]
struct PlanView {
    schema: u16,
    plan_id: String,
    action: RecoveryAction,
    request_digest: BindingDigest,
    expires_at_ms: i64,
    confirmation_available: bool,
    operation: OperationView,
    machine_head: crate::machine_protocol::telemetry_binding::BindingSnapshot,
}

impl PlanView {
    fn new(request: &RecoveryRequest, before: &Operation, enabled: bool) -> Result<Self> {
        ensure!(
            matches!(before.progress, Progress::NeedsAttention { .. })
                && before.intent.machine_step()? == request.step
                && binding_digest(&serde_json::to_vec(before)?) == request.service_operation_digest,
            "operation changed"
        );
        Ok(Self {
            schema: 1,
            plan_id: request.resolution_id.clone(),
            action: request.action,
            request_digest: request.digest()?,
            expires_at_ms: request.expires_at_ms,
            confirmation_available: enabled,
            operation: OperationView::new(before)?,
            machine_head: request.step.expected.clone(),
        })
    }
}

#[derive(serde::Serialize)]
struct ReceiptView {
    schema: u16,
    resolution_id: String,
    operation_id: String,
    machine_id: String,
    action: RecoveryAction,
    operation_digest: BindingDigest,
    request_digest: BindingDigest,
    resolved_at_ms: i64,
}

impl ReceiptView {
    fn new(request: &RecoveryRequest, observed: &RecoveryObservation) -> Result<Self> {
        ensure!(observed.matches(request), "recovery evidence mismatch");
        let RecoveryObservation::Observed { snapshot } = observed else {
            anyhow::bail!("recovery evidence unavailable");
        };
        let receipt = snapshot
            .receipt
            .as_ref()
            .context("missing recovery audit")?;
        ensure!(receipt.matches(request), "recovery audit mismatch");
        Ok(Self {
            schema: 1,
            resolution_id: request.resolution_id.clone(),
            operation_id: request.step.operation_id.clone(),
            machine_id: request.step.machine_id.clone(),
            action: request.action,
            operation_digest: request.service_operation_digest.clone(),
            request_digest: request.digest()?,
            resolved_at_ms: receipt.resolved_at_ms,
        })
    }
}

pub(in crate::server) fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    ApiState: FromRef<S>,
{
    Router::new()
        .route(
            "/api/telemetry/binding/operations/{operation}/machine-recovery-plan",
            post(plan),
        )
        .route(
            "/api/telemetry/binding/operations/{operation}/recover-machine",
            post(confirm),
        )
        .route(
            "/api/telemetry/binding/operations/{operation}/machine-recoveries/{resolution}",
            get(receipt),
        )
        .layer(DefaultBodyLimit::max(1024))
        .layer(axum::middleware::map_response(
            |mut response: Response| async move {
                if matches!(
                    response.status(),
                    StatusCode::BAD_REQUEST
                        | StatusCode::UNPROCESSABLE_ENTITY
                        | StatusCode::PAYLOAD_TOO_LARGE
                        | StatusCode::UNSUPPORTED_MEDIA_TYPE
                ) {
                    response = (
                        response.status(),
                        Json(serde_json::json!({"schema":1,"error":"invalid_request"})),
                    )
                        .into_response();
                }
                response.headers_mut().insert(
                    header::CACHE_CONTROL,
                    header::HeaderValue::from_static("no-store"),
                );
                response
            },
        ))
}

async fn plan(
    State(state): State<ApiState>,
    Path(operation): Path<String>,
    verified: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
    Json(_body): Json<Inspect>,
) -> Result<Json<PlanView>, ApiError> {
    let received = TimeSample::now();
    // One minute INCLUDING preview work. This exact deadline crosses the wire;
    // confirmation cannot give Machine queueing a later wall deadline.
    let expires = chrono::Utc::now().timestamp_millis().saturating_add(60_000);
    let budget = OperationBudget::new(expires, Duration::from_mins(1), received);
    let approval = state
        .approve(verified.as_ref().map(|Extension(v)| v), &headers)
        .await?;
    let ledger = state.ledger().await?.ok_or(ApiError::NotFound)?;
    let before = ledger
        .operations
        .last()
        .filter(|op| op.intent.operation_id == operation)
        .ok_or(ApiError::NotFound)?;
    if !matches!(before.progress, Progress::NeedsAttention { .. }) {
        return Err(ApiError::Changed);
    }
    let step = before
        .intent
        .machine_step()
        .map_err(|_| ApiError::Changed)?;
    let request = RecoveryRequest {
        schema: 1,
        resolution_id: crate::server::random_machine_token().map_err(|_| ApiError::Changed)?,
        actor: approval.actor().into(),
        action: RecoveryAction::RejectInterruptedPrepared,
        service_operation_digest: binding_digest(
            &serde_json::to_vec(before).map_err(|_| ApiError::Changed)?,
        ),
        expected_observation_digest: binding_digest(
            &serde_json::to_vec(&prepared(&step).map_err(|_| ApiError::Changed)?)
                .map_err(|_| ApiError::Changed)?,
        ),
        step,
        expires_at_ms: expires,
    };
    if budget.expired() {
        return Err(ApiError::Changed);
    }
    let live = Live::bind(state.control.clone(), &request).map_err(|_| ApiError::Changed)?;
    let observed = tokio::time::timeout(
        budget.remaining().min(Duration::from_secs(10)),
        live.observe(&request),
    )
    .await
    .map_err(|_| ApiError::Changed)?
    .map_err(|_| ApiError::Changed)?;
    if !observed.matches(&request)
        || !matches!(&observed, RecoveryObservation::Observed { snapshot } if snapshot.receipt.is_none() && request.expects(&snapshot.binding))
        || approval.current_operator(state.auth()).await.as_ref() != Some(approval.actor())
        || state
            .ledger()
            .await?
            .as_ref()
            .and_then(|l| l.operations.last())
            != Some(before)
        || !live.current()
        || budget.expired()
    {
        return Err(ApiError::Changed);
    }
    // A Prepared query does NOT prove a validated Machine reopen. Only the
    // Machine can check that process-local proof during its own admission.
    let view = PlanView::new(&request, before, state.recovery_admitted())
        .map_err(|_| ApiError::Changed)?;
    state
        .recovery_plans
        .insert(request, budget)
        .map_err(|_| ApiError::Changed)?;
    Ok(Json(view))
}

async fn confirm(
    State(state): State<ApiState>,
    Path(operation): Path<String>,
    verified: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
    Json(body): Json<Confirmation>,
) -> Result<Json<ReceiptView>, ApiError> {
    let approval = state
        .approve(verified.as_ref().map(|Extension(v)| v), &headers)
        .await?;
    if !state.recovery_admitted() {
        return Err(ApiError::RecoveryAdmissionClosed);
    }
    let (request, budget) = state
        .recovery_plans
        .consume(&approval.actor().into(), &state.service, &operation, &body)
        .map_err(|_| ApiError::Changed)?;
    // HTTP cancellation drops an observer, never the admitted task. The spent
    // preview stays spent even if the task is rejected or its result is lost.
    tokio::spawn(state.complete_recovery(request, budget, approval))
        .await
        .map_err(|_| ApiError::OutcomeUnverified)?
        .map(Json)
        .map_err(|_| ApiError::OutcomeUnverified)
}

async fn receipt(
    State(state): State<ApiState>,
    Path((operation, resolution)): Path<(String, String)>,
    verified: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
) -> Result<Json<ReceiptView>, ApiError> {
    let received = TimeSample::now();
    let budget = OperationBudget::new(
        chrono::Utc::now().timestamp_millis().saturating_add(10_000),
        Duration::from_secs(10),
        received,
    );
    let approval = state
        .approve(verified.as_ref().map(|Extension(v)| v), &headers)
        .await?;
    let request = state
        .recovery_plans
        .submitted(
            &approval.actor().into(),
            &state.service,
            &operation,
            &resolution,
        )
        .map_err(|_| ApiError::NotFound)?;
    if budget.expired() {
        return Err(ApiError::EvidenceUnavailable);
    }
    let live =
        Live::bind(state.control.clone(), &request).map_err(|_| ApiError::EvidenceUnavailable)?;
    let observed = tokio::time::timeout(budget.remaining(), live.observe(&request))
        .await
        .map_err(|_| ApiError::EvidenceUnavailable)?
        .map_err(|_| ApiError::EvidenceUnavailable)?;
    if approval.current_operator(state.auth()).await.as_ref() != Some(approval.actor())
        || !live.current()
        || budget.expired()
    {
        return Err(ApiError::Authorization(StatusCode::UNAUTHORIZED));
    }
    ReceiptView::new(&request, &observed)
        .map(Json)
        .map_err(|_| ApiError::OutcomeUnverified)
}

#[cfg(test)]
mod tests;
