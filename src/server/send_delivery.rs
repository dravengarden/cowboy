//! Native current-turn delivery. Admission survives disconnects and crashes;
//! an unresolved admission is observation-only, never a replay instruction.
use super::*;
use crate::store::agent_api_operations::Admission;
use std::time::Duration;

static IN_FLIGHT: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(16)));

pub(super) fn begin(
    state: &AppState,
    session: &str,
    text: &str,
    content: &[serde_json::Value],
    cmid: String,
) -> Result<crate::remote_runtime::SteeringDelivery, String> {
    let blocks = build_prompt_blocks(text, content).ok_or("empty prompt")?;
    let fences = state.plugin_lifecycle_fences.read();
    if let Some(refusal) = provider_fence_key_for_session(&state.hub, session)
        .as_ref()
        .and_then(|key| fences.get(key))
        .and_then(|fence| provider_fence_refusal(*fence))
    {
        return Err(refusal.into());
    }
    let info = state.hub.session_info(session).ok_or("unknown session")?;
    if !info.meta.supports_current_turn {
        return Err("current-turn delivery is not supported by this running worker".into());
    }
    if info.meta.status != Status::Busy {
        return Err("there is no current turn; choose next_turn".into());
    }
    state.supervisor.start_steer(
        session,
        blocks
            .into_iter()
            .map(|block| serde_json::to_value(block).expect("ACP content serializes"))
            .collect(),
        cmid,
    )
}

pub(super) fn result(outcome: &str) -> serde_json::Value {
    let status = match outcome {
        "injected" => "injected",
        "not_delivered" | "unsupported" => "not_delivered",
        _ => "unknown",
    };
    serde_json::json!({"status":status,"delivery":"current_turn"})
}

fn pending_result() -> serde_json::Value {
    serde_json::json!({"status":"pending","outcome":"unknown","delivery":"current_turn"})
}

fn operation_key(operation: &str) -> Option<String> {
    (!operation.is_empty()
        && operation.len() <= 128
        && operation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte)))
    .then(|| format!("send:{operation}"))
}

pub(super) async fn submit(
    state: Arc<AppState>,
    session: String,
    request: SessionPromptRequest,
) -> Response {
    let Some(operation) = request.operation.as_deref().and_then(operation_key) else {
        return (
            StatusCode::BAD_REQUEST,
            "current_turn requires a stable operation id",
        )
            .into_response();
    };
    let Some(info) = state.hub.session_info(&session) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let owner = info.meta.owner_user_id.unwrap_or_else(|| "system".into());
    let Some(store) = state.store.clone() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Ok(permit) = Arc::clone(&IN_FLIGHT).try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let hash = crate::managed_calls::canonical_digest(&serde_json::json!({
        "delivery":"current_turn", "text":request.text, "content":request.content,
    }));
    match store
        .admit_agent_operation(&owner, &session, &operation, &hash)
        .await
    {
        Ok(Admission::Existing(saved)) => {
            return Json(saved.unwrap_or_else(pending_result)).into_response();
        }
        Ok(Admission::Conflict) => {
            return (StatusCode::CONFLICT, "operation id has different content").into_response();
        }
        Ok(Admission::New) => {}
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
    let pending = serde_json::json!({"status":"pending","delivery":"current_turn","operation":request.operation});
    let task = tokio::spawn(async move {
        let _permit = permit;
        let effect = begin(
            &state,
            &session,
            &request.text,
            &request.content,
            request.operation.clone().expect("validated operation"),
        );
        let mut receipt = match effect {
            Ok(effect) => result(&effect.outcome().await),
            Err(error) => {
                serde_json::json!({"status":"not_delivered","delivery":"current_turn","error":error})
            }
        };
        receipt["operation"] = serde_json::json!(request.operation);
        if store
            .finish_agent_operation(&owner, &session, &operation, &receipt)
            .await
            .is_err()
        {
            return result("unknown");
        }
        receipt
    });
    match tokio::time::timeout(Duration::from_millis(500), task).await {
        Ok(Ok(receipt)) => Json(receipt).into_response(),
        Ok(Err(_)) => Json(result("unknown")).into_response(),
        Err(_) => (StatusCode::ACCEPTED, Json(pending)).into_response(),
    }
}

pub(super) async fn receipt(
    State(state): State<Arc<AppState>>,
    Path((session, operation)): Path<(String, String)>,
) -> Response {
    let Some(operation) = operation_key(&operation) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(info) = state.hub.session_info(&session) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let owner = info.meta.owner_user_id.unwrap_or_else(|| "system".into());
    let Some(store) = state.store.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match store
        .agent_operation_receipt(&owner, &session, &operation)
        .await
    {
        Ok(Some(Some(receipt))) => Json(receipt).into_response(),
        Ok(Some(None)) => Json(pending_result()).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
