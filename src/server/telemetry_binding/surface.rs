//! Ordinary binding confirmation. Core owns target discovery, previews and
//! one-use dispatch; durable receipt reads never reconstruct authority.
use super::http::{ApiError, ApiState};
use super::live::{BINDING_WRITE_ADMISSION, LiveEffects};
use super::resolution::surface::view::OperationView;
use super::*;
use crate::machine_protocol::telemetry_binding::{
    BindingChange, BindingDigest, BindingInstallation, BindingSnapshot, binding_digest,
};
use crate::operation_budget::{OperationBudget, TimeSample};
use crate::plugin_operation::Actor;
use crate::server::{AuthenticatedProductRequest, operator_approval::OperatorApproval};
use crate::telemetry_binding::Ledger;
use anyhow::Context;
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, FromRef, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use std::{collections::HashMap, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    machine_id: String,
    installation: BindingInstallation,
}

#[derive(serde::Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Inspect {
    Select { target: Target },
    Revoke {},
    Restore { operation_id: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Select,
    Revoke,
    Restore,
}

impl From<&BindingChange> for Action {
    fn from(value: &BindingChange) -> Self {
        match value {
            BindingChange::Select { .. } => Self::Select,
            BindingChange::Revoke { .. } => Self::Revoke,
            BindingChange::Restore { .. } => Self::Restore,
        }
    }
}

struct Preview {
    intent: Intent,
    before: BindingDigest,
    budget: OperationBudget,
    effects: LiveEffects,
}

#[derive(Default)]
pub(in crate::server) struct Plans(parking_lot::Mutex<HashMap<String, Preview>>);

impl Plans {
    fn insert(&self, preview: Preview) -> Result<()> {
        let mut plans = self.0.lock();
        plans.retain(|_, p| !p.budget.expired());
        ensure!(plans.len() < 256, "binding preview capacity exhausted");
        ensure!(
            !preview.budget.expired() && !plans.contains_key(&preview.intent.operation_id),
            "binding preview expired or reused"
        );
        plans.insert(preview.intent.operation_id.clone(), preview);
        Ok(())
    }

    fn consume(&self, actor: &Actor, service: &str, request: &Confirm) -> Result<Preview> {
        let mut plans = self.0.lock();
        let p = plans
            .get(&request.plan_id)
            .context("binding preview unavailable")?;
        ensure!(
            &p.intent.actor == actor
                && p.intent.service_id == service
                && Action::from(&p.intent.change) == request.action
                && !p.budget.expired(),
            "binding preview owner, purpose or deadline changed"
        );
        Ok(plans
            .remove(&request.plan_id)
            .expect("checked under same lock"))
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirm {
    plan_id: String,
    action: Action,
}

#[derive(serde::Serialize)]
struct ChoicesView {
    schema: u16,
    confirmation_available: bool,
    owner_machine_id: Option<String>,
    targets: Vec<Target>,
    revoke_available: bool,
    restore_operation_id: Option<String>,
}

#[derive(serde::Serialize)]
struct PlanView {
    schema: u16,
    plan_id: String,
    action: Action,
    request_digest: BindingDigest,
    expires_at_ms: i64,
    confirmation_available: bool,
    operation: OperationView,
    result_head: BindingSnapshot,
    restores_operation_id: Option<String>,
}

impl PlanView {
    fn new(intent: &Intent, enabled: bool, restores_operation_id: Option<String>) -> Result<Self> {
        let step = intent.machine_step()?;
        Ok(Self {
            schema: 1,
            plan_id: intent.operation_id.clone(),
            action: Action::from(&intent.change),
            request_digest: step.request_digest()?,
            expires_at_ms: intent.expires_at_ms,
            confirmation_available: enabled,
            operation: OperationView::new(&Operation {
                intent: intent.clone(),
                progress: Progress::Prepared,
            })?,
            result_head: step.after()?,
            restores_operation_id,
        })
    }
}

#[derive(serde::Serialize)]
struct ReceiptView {
    schema: u16,
    request_digest: BindingDigest,
    operation: OperationView,
}

impl ReceiptView {
    fn new(operation: &Operation) -> Result<Self> {
        Ok(Self {
            schema: 1,
            request_digest: operation.intent.machine_step()?.request_digest()?,
            operation: OperationView::new(operation)?,
        })
    }
}

fn fingerprint(ledger: &Option<Ledger>) -> Result<BindingDigest> {
    Ok(binding_digest(&serde_json::to_vec(ledger)?))
}

fn forward(ledger: &Ledger) -> Option<&Operation> {
    ledger.operations.iter().rev().find(|op| {
        matches!(op.progress, Progress::Completed { .. })
            && op
                .intent
                .machine_step()
                .and_then(|s| s.after())
                .is_ok_and(|after| ledger.current.as_ref() == Some(&after))
    })
}

impl ApiState {
    fn binding_admitted(&self) -> bool {
        #[cfg(test)]
        if self.fixture_binding_admission {
            return true;
        }
        BINDING_WRITE_ADMISSION
    }

    fn draft(
        &self,
        ledger: Option<&Ledger>,
        actor: &Actor,
        request: Inspect,
        expires: i64,
    ) -> Result<Intent> {
        let expected = ledger.and_then(|l| l.current.clone());
        let epoch = expected
            .as_ref()
            .unwrap_or(&BindingSnapshot::initial())
            .policy_epoch
            .next()?;
        let owner = || {
            ledger
                .and_then(|l| l.operations.last())
                .map(|op| op.intent.machine_id.clone())
                .context("missing binding owner")
        };
        let (machine_id, change) = match request {
            Inspect::Select { target } => (
                target.machine_id,
                BindingChange::Select {
                    installation: target.installation,
                    policy_epoch: epoch,
                },
            ),
            Inspect::Revoke {} => {
                ensure!(
                    expected
                        .as_ref()
                        .is_some_and(|head| head.selection.is_some()),
                    "no managed selection to revoke"
                );
                (
                    owner()?,
                    BindingChange::Revoke {
                        policy_epoch: epoch,
                    },
                )
            }
            Inspect::Restore { operation_id } => {
                let op = ledger
                    .and_then(forward)
                    .filter(|op| op.intent.operation_id == operation_id)
                    .context("no exact completed forward operation")?;
                let step = op.intent.machine_step()?;
                (
                    owner()?,
                    BindingChange::Restore {
                        forward_request_digest: step.request_digest()?,
                        selection: step.expected.selection,
                        policy_epoch: epoch,
                    },
                )
            }
        };
        let intent = Intent {
            schema: 2,
            operation_id: crate::server::random_machine_token()?,
            service_id: self.service.clone(),
            actor: actor.clone(),
            machine_id,
            expected,
            change,
            expires_at_ms: expires,
        };
        intent.machine_step()?.validate_commit()?;
        Ok(intent)
    }

    fn binding_effects(&self, intent: &Intent) -> Result<LiveEffects> {
        LiveEffects::bind(
            self.control.clone(),
            self.catalog.clone(),
            self.fences.clone(),
            intent,
        )
    }

    async fn complete_binding(
        self,
        preview: Preview,
        approval: OperatorApproval,
    ) -> Result<ReceiptView> {
        ensure!(self.binding_admitted(), "binding admission closed");
        let authority = approval
            .bind_telemetry(&preview.intent)?
            .constrain_to_preview(preview.budget);
        ensure!(
            fingerprint(
                &self
                    .ledger()
                    .await
                    .map_err(|_| anyhow::anyhow!("journal unavailable"))?
            )? == preview.before,
            "Service evidence changed"
        );
        let operation = coordinate(
            self.store.as_ref().context("missing journal")?,
            &self.legacy_fence,
            &preview.intent,
            super::Confirmation {
                authority: &authority,
                auth: self.auth(),
            },
            &preview.effects,
        )
        .await?;
        ReceiptView::new(&operation)
    }
}

pub(in crate::server) fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    ApiState: FromRef<S>,
{
    Router::new()
        .route("/api/telemetry/binding/choices", get(choices))
        .route("/api/telemetry/binding/plan", post(plan))
        .route("/api/telemetry/binding/confirm", post(confirm))
        .route(
            "/api/telemetry/binding/operations/{operation}/receipt",
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

async fn choices(
    State(state): State<ApiState>,
    verified: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
) -> Result<Json<ChoicesView>, ApiError> {
    let approval = state
        .approve(verified.as_ref().map(|Extension(v)| v), &headers)
        .await?;
    let ledger = state.ledger().await?;
    let owner = ledger
        .as_ref()
        .and_then(|l| l.operations.last())
        .map(|op| op.intent.machine_id.clone());
    let idle = ledger.as_ref().is_none_or(|l| {
        l.operations.last().is_some_and(|op| {
            matches!(
                op.progress,
                Progress::Completed { .. } | Progress::Rejected { .. } | Progress::Aborted
            )
        })
    });
    let mut targets = Vec::new();
    if idle {
        for entry in state.control.connected_plugin_inventory() {
            if owner
                .as_ref()
                .is_some_and(|owner| owner != &entry.machine_id)
            {
                continue;
            }
            let Some(revision) = entry.plugin.installation_revision else {
                continue;
            };
            let (Ok(generation), Ok(contract)) = (
                BindingDigest::try_from(entry.plugin.generation_digest),
                BindingDigest::try_from(entry.plugin.contract_fingerprint),
            ) else {
                continue;
            };
            let target = Target {
                machine_id: entry.machine_id,
                installation: BindingInstallation {
                    plugin_id: entry.plugin.plugin_id,
                    plugin_version: entry.plugin.plugin_version,
                    generation_digest: generation,
                    installation_revision: revision,
                    contract_fingerprint: contract,
                },
            };
            let intent = state.draft(
                ledger.as_ref(),
                approval.actor(),
                Inspect::Select {
                    target: target.clone(),
                },
                chrono::Utc::now().timestamp_millis().saturating_add(60_000),
            );
            if intent.is_ok_and(|intent| state.binding_effects(&intent).is_ok())
                && !targets.contains(&target)
            {
                if targets.len() == 64 {
                    return Err(ApiError::EvidenceUnavailable);
                }
                targets.push(target);
            }
        }
    }
    targets.sort_by(|a, b| {
        (&a.machine_id, &a.installation.plugin_id).cmp(&(&b.machine_id, &b.installation.plugin_id))
    });
    if approval.current_operator(state.auth()).await.as_ref() != Some(approval.actor()) {
        return Err(ApiError::Authorization(StatusCode::UNAUTHORIZED));
    }
    Ok(Json(ChoicesView {
        schema: 1,
        confirmation_available: state.binding_admitted(),
        owner_machine_id: owner,
        targets,
        revoke_available: idle
            && ledger
                .as_ref()
                .and_then(|l| l.current.as_ref())
                .is_some_and(|head| head.selection.is_some()),
        restore_operation_id: if idle {
            ledger
                .as_ref()
                .and_then(forward)
                .map(|op| op.intent.operation_id.clone())
        } else {
            None
        },
    }))
}

async fn plan(
    State(state): State<ApiState>,
    verified: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
    Json(request): Json<Inspect>,
) -> Result<Json<PlanView>, ApiError> {
    let received = TimeSample::now();
    let expires = chrono::Utc::now().timestamp_millis().saturating_add(60_000);
    let budget = OperationBudget::new(expires, Duration::from_mins(1), received);
    let approval = state
        .approve(verified.as_ref().map(|Extension(v)| v), &headers)
        .await?;
    let mut ledger = state.ledger().await?;
    let before = fingerprint(&ledger).map_err(|_| ApiError::EvidenceUnavailable)?;
    let restores_operation_id = match &request {
        Inspect::Restore { operation_id } => Some(operation_id.clone()),
        _ => None,
    };
    let intent = state
        .draft(ledger.as_ref(), approval.actor(), request, expires)
        .map_err(|_| ApiError::Changed)?;
    crate::telemetry_binding::writer::apply(&mut ledger, &Change::Begin(&intent))
        .map_err(|_| ApiError::Changed)?;
    let effects = state
        .binding_effects(&intent)
        .map_err(|_| ApiError::Changed)?;
    let step = intent.machine_step().map_err(|_| ApiError::Changed)?;
    let observation = tokio::time::timeout(
        budget.remaining().min(Duration::from_secs(10)),
        effects.observe(&step),
    )
    .await
    .map_err(|_| ApiError::Changed)?
    .map_err(|_| ApiError::Changed)?;
    if !observation.matches(&step)
        || !matches!(observation, BindingObservation::Observed { snapshot } if snapshot.current == intent.expected && !snapshot.unresolved && snapshot.receipt.is_none())
        || !effects.current()
        || budget.expired()
        || approval.current_operator(state.auth()).await.as_ref() != Some(approval.actor())
        || fingerprint(&state.ledger().await?).map_err(|_| ApiError::EvidenceUnavailable)? != before
        || budget.expired()
    {
        return Err(ApiError::Changed);
    }
    let view = PlanView::new(&intent, state.binding_admitted(), restores_operation_id)
        .map_err(|_| ApiError::Changed)?;
    state
        .binding_plans
        .insert(Preview {
            intent,
            before,
            budget,
            effects,
        })
        .map_err(|_| ApiError::Changed)?;
    Ok(Json(view))
}

async fn confirm(
    State(state): State<ApiState>,
    verified: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
    Json(request): Json<Confirm>,
) -> Result<Json<ReceiptView>, ApiError> {
    let approval = state
        .approve(verified.as_ref().map(|Extension(v)| v), &headers)
        .await?;
    if !state.binding_admitted() {
        return Err(ApiError::BindingAdmissionClosed);
    }
    let preview = state
        .binding_plans
        .consume(approval.actor(), &state.service, &request)
        .map_err(|_| ApiError::Changed)?;
    // HTTP cancellation cannot put the consumed plan back or cancel bookkeeping.
    tokio::spawn(state.complete_binding(preview, approval))
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
    let approval = state
        .approve(verified.as_ref().map(|Extension(v)| v), &headers)
        .await?;
    let ledger = state.ledger().await?.ok_or(ApiError::NotFound)?;
    let op = ledger
        .operations
        .iter()
        .find(|op| op.intent.operation_id == operation)
        .ok_or(ApiError::NotFound)?;
    if approval.current_operator(state.auth()).await.as_ref() != Some(approval.actor()) {
        return Err(ApiError::Authorization(StatusCode::UNAUTHORIZED));
    }
    ReceiptView::new(op)
        .map(Json)
        .map_err(|_| ApiError::EvidenceUnavailable)
}

#[cfg(test)]
mod tests;
