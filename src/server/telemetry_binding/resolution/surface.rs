//! Finite core confirmation surface. Preview is read-only, commit is one-use
//! and purpose-bound, receipt reads never repeat a Machine command or DB write.
use super::*;
use crate::machine_protocol::telemetry_binding::{BindingDigest, binding_digest};
use crate::operation_budget::{OperationBudget, TimeSample};
use crate::plugin_operation::Actor;
use crate::server::{AppState, AuthenticatedProductRequest, operator_approval::OperatorApproval};
use anyhow::Context;
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, FromRef, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use std::{collections::HashMap, time::Duration};

mod view;
use view::{Action, PlanView, ReceiptView, StatusView};

struct Preview {
    intent: ResolutionIntent,
    budget: OperationBudget,
}

#[derive(Default)]
pub(in crate::server) struct Plans(parking_lot::Mutex<HashMap<String, Preview>>);

impl Plans {
    fn insert(&self, preview: Preview) -> Result<()> {
        preview.intent.digest()?;
        let mut plans = self.0.lock();
        plans.retain(|_, saved| !saved.budget.expired());
        ensure!(plans.len() < 256, "preview capacity exhausted");
        ensure!(
            !plans.contains_key(&preview.intent.resolution_id),
            "preview identity reused"
        );
        ensure!(!preview.budget.expired(), "preview expired");
        plans.insert(preview.intent.resolution_id.clone(), preview);
        Ok(())
    }

    fn consume(
        &self,
        actor: &Actor,
        service: &str,
        operation: &str,
        request: &Confirmation,
    ) -> Result<Preview> {
        let mut plans = self.0.lock();
        let preview = plans.get(&request.plan_id).context("preview unavailable")?;
        ensure!(
            &preview.intent.actor == actor
                && preview.intent.service_id == service
                && preview.intent.operation_id == operation
                && Action::from(&preview.intent.action) == request.action
                && !preview.budget.expired(),
            "preview owner, action, operation or deadline changed"
        );
        Ok(plans
            .remove(&request.plan_id)
            .expect("checked under same lock"))
    }
}

// Only the dependencies needed by this core surface, not Plugin-controlled
// state or an alternative authentication registry. Clone holds no authority.
#[derive(Clone)]
pub(in crate::server) struct ApiState {
    service: String,
    store: Option<Store>,
    control: Arc<MachineControl>,
    plans: Arc<Plans>,
    hub: crate::core::Hub,
    product_auth_enabled: bool,
    devices: Arc<crate::client_auth::DeviceAccessSessions>,
    authentication: Arc<crate::auth_plugins::ProductAuthentication>,
    #[cfg(test)]
    fixture_write_admission: bool,
}

impl FromRef<Arc<AppState>> for ApiState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        Self {
            service: state.service_id.clone(),
            store: state.store.clone(),
            control: state.machine_control.clone(),
            plans: state.telemetry_resolution_plans.clone(),
            hub: state.hub.clone(),
            product_auth_enabled: state.product_auth_enabled,
            devices: state.device_access.clone(),
            authentication: state.product_authentication.clone(),
            #[cfg(test)]
            fixture_write_admission: false,
        }
    }
}

impl ApiState {
    fn auth(&self) -> ProductRequestAuth<'_> {
        ProductRequestAuth {
            product_auth_enabled: self.product_auth_enabled,
            store: self.store.as_ref(),
            hub: &self.hub,
            device_access: &self.devices,
            product_authentication: &self.authentication,
        }
    }

    fn write_admitted(&self) -> bool {
        #[cfg(test)]
        if self.fixture_write_admission {
            return true;
        }
        RESOLUTION_WRITE_ADMISSION
    }

    fn store(&self) -> Result<&Store, ApiError> {
        self.store.as_ref().ok_or(ApiError::EvidenceUnavailable)
    }

    async fn ledger(&self) -> Result<Option<Ledger>, ApiError> {
        self.store()?
            .telemetry_binding_ledger(&self.service)
            .await
            .map_err(|_| ApiError::EvidenceUnavailable)
    }

    async fn approve(
        &self,
        verified: Option<&AuthenticatedProductRequest>,
        headers: &HeaderMap,
    ) -> Result<OperatorApproval, ApiError> {
        let approval = OperatorApproval::capture(self.auth(), &self.service, verified, headers)
            .map_err(ApiError::Authorization)?;
        if approval.current_operator(self.auth()).await.as_ref() != Some(approval.actor()) {
            return Err(ApiError::Authorization(StatusCode::UNAUTHORIZED));
        }
        Ok(approval)
    }

    async fn complete(self, preview: Preview, approval: OperatorApproval) -> Result<ReceiptView> {
        ensure!(self.write_admitted(), "resolution admission closed");
        let authority = approval
            .bind_telemetry_resolution(&preview.intent)?
            .constrain_to_preview(preview.budget);
        let observer = if matches!(preview.intent.action, ResolutionAction::AbortBeforeDispatch) {
            None
        } else {
            Some(LiveObservation::capture(
                self.control.clone(),
                &preview.intent.machine_id,
            )?)
        };
        let store = self.store.as_ref().context("missing journal")?;
        // Hermetic tests exercise the same coordinator with only their own
        // temporary DB admitted. No feature flag, env or HTTP body can enable it.
        #[cfg(test)]
        if self.fixture_write_admission {
            coordinate(
                store,
                &preview.intent,
                authority,
                self.auth(),
                observer.as_ref(),
            )
            .await?;
            return self.exact_receipt(&preview.intent).await;
        }
        resolve(
            store,
            &preview.intent,
            authority,
            self.auth(),
            observer.as_ref(),
        )
        .await?;
        self.exact_receipt(&preview.intent).await
    }

    async fn exact_receipt(&self, intent: &ResolutionIntent) -> Result<ReceiptView> {
        let ledger = self
            .ledger()
            .await
            .map_err(|_| anyhow::anyhow!("receipt unavailable"))?
            .context("missing journal")?;
        let record = ledger
            .resolutions
            .iter()
            .find(|record| &record.intent == intent)
            .context("exact resolution not recorded")?;
        Ok(ReceiptView::from(record))
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Inspect {}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirmation {
    plan_id: String,
    action: Action,
}

#[derive(Debug)]
enum ApiError {
    Authorization(StatusCode),
    EvidenceUnavailable,
    NotFound,
    Changed,
    AdmissionClosed,
    OutcomeUnverified,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Authorization(status) => (status, "authorization_required"),
            Self::EvidenceUnavailable => (StatusCode::SERVICE_UNAVAILABLE, "evidence_unavailable"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Changed => (StatusCode::CONFLICT, "preview_or_evidence_changed"),
            Self::AdmissionClosed => (StatusCode::CONFLICT, "resolution_admission_closed"),
            Self::OutcomeUnverified => (StatusCode::CONFLICT, "outcome_unverified"),
        };
        (
            status,
            Json(serde_json::json!({"schema": 1, "error": code})),
        )
            .into_response()
    }
}

pub(in crate::server) fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    ApiState: FromRef<S>,
{
    Router::new()
        .route("/api/telemetry/binding", get(status))
        .route(
            "/api/telemetry/binding/operations/{operation}/resolution-plan",
            post(plan),
        )
        .route(
            "/api/telemetry/binding/operations/{operation}/resolve",
            post(confirm),
        )
        .route(
            "/api/telemetry/binding/operations/{operation}/resolution",
            get(receipt),
        )
        .layer(DefaultBodyLimit::max(1024))
        .layer(axum::middleware::map_response(
            |mut response: Response| async move {
                // Do not echo JSON/path extractor diagnostics or supplied fields.
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

async fn status(
    State(state): State<ApiState>,
    verified: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
) -> Result<Json<StatusView>, ApiError> {
    state
        .approve(verified.as_ref().map(|Extension(v)| v), &headers)
        .await?;
    StatusView::new(state.ledger().await?.as_ref(), state.write_admitted())
        .map(Json)
        .map_err(|_| ApiError::EvidenceUnavailable)
}

async fn plan(
    State(state): State<ApiState>,
    Path(operation): Path<String>,
    verified: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
    Json(_request): Json<Inspect>,
) -> Result<Json<PlanView>, ApiError> {
    let received = TimeSample::now();
    let expires = chrono::Utc::now()
        .timestamp_millis()
        .saturating_add(120_000);
    let budget = OperationBudget::new(expires, Duration::from_mins(2), received);
    let approval = state
        .approve(verified.as_ref().map(|Extension(v)| v), &headers)
        .await?;
    let ledger = state.ledger().await?.ok_or(ApiError::NotFound)?;
    let before = ledger
        .operations
        .last()
        .filter(|op| op.intent.operation_id == operation)
        .ok_or(ApiError::NotFound)?;
    let observer = match before.progress {
        Progress::Prepared => None,
        Progress::Dispatching | Progress::NeedsAttention { .. } => Some(
            LiveObservation::capture(state.control.clone(), &before.intent.machine_id)
                .map_err(|_| ApiError::Changed)?,
        ),
        _ => return Err(ApiError::Changed),
    };
    let observation = match &observer {
        None => None,
        Some(observer) => Some(
            tokio::time::timeout(
                budget.remaining().min(Duration::from_secs(10)),
                observer.observe(
                    &before
                        .intent
                        .machine_step()
                        .map_err(|_| ApiError::Changed)?,
                ),
            )
            .await
            .map_err(|_| ApiError::Changed)?
            .map_err(|_| ApiError::Changed)?,
        ),
    };
    let action = match (&before.progress, &observation) {
        (Progress::Prepared, None) => ResolutionAction::AbortBeforeDispatch,
        (_, Some(value)) => match super::super::conclusion(&before.intent, value.clone()) {
            Progress::Completed { .. } => ResolutionAction::AcceptApplied {
                observation_digest: binding_digest(
                    &serde_json::to_vec(value).map_err(|_| ApiError::Changed)?,
                ),
            },
            Progress::Rejected { .. } => ResolutionAction::RecordRejected {
                observation_digest: binding_digest(
                    &serde_json::to_vec(value).map_err(|_| ApiError::Changed)?,
                ),
            },
            _ => return Err(ApiError::Changed),
        },
        _ => return Err(ApiError::Changed),
    };
    let intent = ResolutionIntent::new(
        crate::server::random_machine_token().map_err(|_| ApiError::Changed)?,
        approval.actor().clone(),
        before,
        action,
        expires,
    )
    .map_err(|_| ApiError::Changed)?;
    let after = intent
        .conclusion(before, observation)
        .map_err(|_| ApiError::Changed)?;
    // Preview is not a permit. Still reject changes during read/query before
    // showing this exact snapshot; confirmation checks them independently again.
    if budget.expired()
        || observer.as_ref().is_some_and(|o| !o.current())
        || approval.current_operator(state.auth()).await.as_ref() != Some(approval.actor())
        || state
            .ledger()
            .await?
            .as_ref()
            .and_then(|l| l.operations.last())
            != Some(before)
    {
        return Err(ApiError::Changed);
    }
    let view = PlanView::new(&intent, before, after, state.write_admitted())
        .map_err(|_| ApiError::Changed)?;
    state
        .plans
        .insert(Preview { intent, budget })
        .map_err(|_| ApiError::Changed)?;
    Ok(Json(view))
}

async fn confirm(
    State(state): State<ApiState>,
    Path(operation): Path<String>,
    verified: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
    Json(request): Json<Confirmation>,
) -> Result<Json<ReceiptView>, ApiError> {
    let approval = state
        .approve(verified.as_ref().map(|Extension(v)| v), &headers)
        .await?;
    if !state.write_admitted() {
        return Err(ApiError::AdmissionClosed);
    }
    let preview = state
        .plans
        .consume(approval.actor(), &state.service, &operation, &request)
        .map_err(|_| ApiError::Changed)?;
    // Observer cancellation detaches the admitted task. The one-use plan is
    // never put back. Lost HTTP/COMMIT responses are inspected via GET only.
    tokio::spawn(state.complete(preview, approval))
        .await
        .map_err(|_| ApiError::OutcomeUnverified)?
        .map(Json)
        .map_err(|_| ApiError::OutcomeUnverified)
}

async fn receipt(
    State(state): State<ApiState>,
    Path(operation): Path<String>,
    verified: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
) -> Result<Json<ReceiptView>, ApiError> {
    state
        .approve(verified.as_ref().map(|Extension(v)| v), &headers)
        .await?;
    let ledger = state.ledger().await?.ok_or(ApiError::NotFound)?;
    ledger
        .resolutions
        .iter()
        .find(|record| record.intent.operation_id == operation)
        .map(|record| Json(ReceiptView::from(record)))
        .ok_or(ApiError::NotFound)
}

#[cfg(test)]
mod tests;
