//! Explicit owner-authorized revocation. Metadata edits cannot invoke it.
use super::*;
use crate::provider_account::ProviderAccountId;
use crate::store::provider_accounts::Change;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct Revoke {
    pub account_id: ProviderAccountId,
    pub expected_revision: i64,
}

pub(in crate::server) async fn start(
    state: &AppState,
    owner: &str,
    operation: &str,
    input: Revoke,
) -> Result<Value, &'static str> {
    update_value(
        state,
        owner,
        operation,
        &Mutation {
            expected_revision: input.expected_revision,
            change: Change::BeginRevocation {
                id: input.account_id,
            },
        },
    )
    .await
}

pub(in crate::server) async fn status(
    state: &AppState,
    owner: &str,
    account: &ProviderAccountId,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let Some((operation, receipt)) = store
        .provider_account_revocation(owner, account)
        .await
        .map_err(|_| "unavailable")?
    else {
        return Ok(json!({"status":"not_started", "account_id":account}));
    };
    let mut value = receipt_value(&receipt);
    value["completion"] = serde_json::to_value(
        store
            .provider_revocation_readiness(owner, &operation)
            .await
            .map_err(|_| "unavailable")?,
    )
    .map_err(|_| "unavailable")?;
    Ok(value)
}

pub(in crate::server) async fn resume(
    state: &Arc<AppState>,
    owner: &str,
    account: &ProviderAccountId,
) -> Result<Value, &'static str> {
    let (operation, _) = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .provider_account_revocation(owner, account)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("revocation_operation_missing")?;
    // This locator conveys no dispatch authority. Every stage rechecks the
    // immutable plan, owner, enrollment and current connection itself.
    super::reconcile(state, owner, &operation).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct Start {
    operation: String,
    dataset: String,
    expected_revision: i64,
}

fn operation(id: &str) -> Result<String, &'static str> {
    if !crate::managed_calls::valid_id(id) || matches!(id, "." | "..") {
        return Err("invalid_operation");
    }
    Ok(format!(
        "http-revoke-{}",
        crate::admin::hex_sha256(id.as_bytes())
    ))
}

fn response(result: Result<Value, &'static str>) -> Response {
    let (status, value) = match result {
        Ok(value) => (
            if value.get("error").is_some() {
                StatusCode::CONFLICT
            } else {
                StatusCode::OK
            },
            value,
        ),
        Err(code) => (
            if matches!(code, "unavailable" | "outcome_unknown") {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::CONFLICT
            },
            json!({"error":{"code":code}}),
        ),
    };
    (status, [(header::CACHE_CONTROL, "no-store")], Json(value)).into_response()
}

pub(in crate::server) async fn http_start(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account_id): Path<ProviderAccountId>,
    Json(input): Json<Start>,
) -> Response {
    if !sync_dataset::matches(&state.service_id, &auth.principal, &input.dataset) {
        return response(Err("dataset_changed"));
    }
    let operation = match operation(&input.operation) {
        Ok(operation) => operation,
        Err(code) => return response(Err(code)),
    };
    response(
        start(
            &state,
            &auth.principal.user_id,
            &operation,
            Revoke {
                account_id,
                expected_revision: input.expected_revision,
            },
        )
        .await,
    )
}

pub(in crate::server) async fn http_status(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account): Path<ProviderAccountId>,
) -> Response {
    response(status(&state, &auth.principal.user_id, &account).await)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct Resume {
    dataset: String,
}

pub(in crate::server) async fn http_resume(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account): Path<ProviderAccountId>,
    Json(input): Json<Resume>,
) -> Response {
    if !sync_dataset::matches(&state.service_id, &auth.principal, &input.dataset) {
        return response(Err("dataset_changed"));
    }
    response(resume(&state, &auth.principal.user_id, &account).await)
}
