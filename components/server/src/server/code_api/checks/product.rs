//! Explicit product request to run a declared check, under a current Operator.
use super::*;
use crate::server::operator_approval::OperatorApproval;

pub(super) static EPOCH: LazyLock<String> = LazyLock::new(|| uuid::Uuid::new_v4().to_string());
type OperationKey = (String, String);
type Operations = HashMap<OperationKey, Option<(String, String)>>;
// No eviction: a forgotten request must not execute again on retry.
static OPERATIONS: LazyLock<Mutex<Operations>> = LazyLock::new(Mutex::default);

fn reserve(
    operations: &mut Operations,
    key: OperationKey,
    check: &str,
) -> Result<Option<String>, StatusCode> {
    if let Some(result) = operations.get(&key) {
        return match result {
            Some((original, run)) if original == check => Ok(Some(run.clone())),
            _ => Err(StatusCode::CONFLICT),
        };
    }
    if operations.len() >= 4096 {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    if operations
        .keys()
        .filter(|existing| existing.0 == key.0)
        .count()
        >= 64
    {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    operations.insert(key, None);
    Ok(None)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartRequest {
    check: String,
    operation: String,
    epoch: String,
}

pub(super) async fn start_product(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Path(session): Path<String>,
    headers: HeaderMap,
    Json(request): Json<StartRequest>,
) -> Response {
    if request.epoch != *EPOCH {
        return (StatusCode::CONFLICT, Json(json!({"status":"epoch_changed", "reason":"The service restarted. Earlier results are unavailable; inspect the outcome before starting a new check."})))
            .into_response();
    }
    if request.operation.is_empty()
        || request.operation.len() > 128
        || !request
            .operation
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let approval = match OperatorApproval::capture_product(
        ProductRequestAuth::from(state.as_ref()),
        &state.service_id,
        Some(&authenticated),
        &headers,
    ) {
        Ok(approval) => approval,
        Err(status) => return status.into_response(),
    };
    if let Err(error) = authorize(&state, &session, &approval).await {
        return (StatusCode::FORBIDDEN, error).into_response();
    }
    let key = (session.clone(), request.operation.clone());
    {
        let mut operations = OPERATIONS.lock();
        match reserve(&mut operations, key.clone(), &request.check) {
            Ok(Some(run_id)) => {
                return Json(json!({"status":"existing", "runId":run_id})).into_response();
            }
            Err(status) => return (
                status,
                "Check request unresolved, changed, or receipt capacity reached; inspect results",
            )
                .into_response(),
            Ok(None) => {}
        }
    }
    let outcome = start_authorized(
        &state,
        &session,
        &request.check,
        &request.operation,
        Some(&approval),
    )
    .await;
    finish(&mut OPERATIONS.lock(), key, outcome)
}

fn finish(
    operations: &mut Operations,
    key: OperationKey,
    outcome: Result<Started, String>,
) -> Response {
    match outcome {
        Ok(started) => {
            let (status, run) = match started {
                Started::New(run) => ("accepted", run),
                Started::Existing(run) => ("existing", run),
            };
            operations.insert(key, Some((run.check.clone(), run.run_id.clone())));
            Json(json!({"status":status, "run":run})).into_response()
        }
        Err(error) => {
            // Every Err precedes recording/spawning a run: this is a definite
            // refusal, not an unknown effect. Releasing it cannot replay work.
            operations.remove(&key);
            let status = if error.starts_with("restricted") {
                StatusCode::FORBIDDEN
            } else if error.starts_with("no declared check") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            (status, Json(json!({"status":"refused", "reason":error}))).into_response()
        }
    }
}

pub(super) async fn authorize(
    state: &AppState,
    session: &str,
    approval: &OperatorApproval,
) -> Result<(), String> {
    let principal = approval
        .current_product_operator(ProductRequestAuth::from(state))
        .await
        .ok_or("restricted: operator authorization expired")?;
    let info = state.hub.session_info(session).ok_or("unavailable")?;
    if !principal.can_mutate(info.meta.owner_user_id.as_deref()) {
        return Err("restricted: session ownership changed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn definite_refusal_releases_reservation_and_returns_a_typed_response() {
        for (reason, expected) in [
            ("restricted: changed", StatusCode::FORBIDDEN),
            ("no declared check named lint", StatusCode::NOT_FOUND),
            ("unavailable", StatusCode::SERVICE_UNAVAILABLE),
        ] {
            let mut operations = Operations::new();
            let key = ("s".into(), "op".into());
            assert_eq!(reserve(&mut operations, key.clone(), "lint"), Ok(None));
            let response = finish(&mut operations, key.clone(), Err(reason.into()));
            assert_eq!(response.status(), expected);
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["status"], "refused");
            assert_eq!(body["reason"], reason);
            assert!(operations.is_empty());
            assert_eq!(reserve(&mut operations, key, "lint"), Ok(None));
        }
    }

    #[test]
    fn uncertain_requests_never_reserve_twice_and_identity_is_bound() {
        let mut operations = Operations::new();
        let key = ("session".into(), "operation".into());
        assert_eq!(reserve(&mut operations, key.clone(), "lint"), Ok(None));
        assert_eq!(
            reserve(&mut operations, key.clone(), "lint"),
            Err(StatusCode::CONFLICT)
        );
        operations.insert(key.clone(), Some(("lint".into(), "run-1".into())));
        assert_eq!(
            reserve(&mut operations, key.clone(), "lint"),
            Ok(Some("run-1".into()))
        );
        assert_eq!(
            reserve(&mut operations, key, "test"),
            Err(StatusCode::CONFLICT)
        );
    }

    #[test]
    fn capacity_preserves_old_receipts_and_refuses_new_effects() {
        let mut operations = Operations::new();
        for n in 0..4096 {
            operations.insert(
                ("s".into(), n.to_string()),
                Some(("lint".into(), n.to_string())),
            );
        }
        assert_eq!(
            reserve(&mut operations, ("s".into(), "new".into()), "lint"),
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
        assert_eq!(
            reserve(&mut operations, ("s".into(), "0".into()), "lint"),
            Ok(Some("0".into()))
        );
        assert_eq!(operations.len(), 4096);
    }

    #[test]
    fn one_session_cannot_consume_another_sessions_share() {
        let mut operations = Operations::new();
        for n in 0..64 {
            assert_eq!(
                reserve(&mut operations, ("a".into(), n.to_string()), "lint"),
                Ok(None)
            );
        }
        assert_eq!(
            reserve(&mut operations, ("a".into(), "next".into()), "lint"),
            Err(StatusCode::TOO_MANY_REQUESTS)
        );
        assert_eq!(
            reserve(&mut operations, ("b".into(), "next".into()), "lint"),
            Ok(None)
        );
    }
}
