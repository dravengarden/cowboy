//! Native current-turn delivery. Admission survives disconnects and crashes;
//! an unresolved admission is observation-only, never a replay instruction.
use super::*;
use crate::store::agent_api_operations::Admission;
use std::time::Duration;

static IN_FLIGHT: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(16)));

static POLICY_WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn policy_key(session: &str) -> String {
    format!(
        "{}{session}",
        crate::core::settings_keys::SESSION_DELIVERY_POLICY_PREFIX
    )
}

pub(super) fn policy(hub: &Hub, session: &str) -> crate::send_delivery::DeliveryPolicy {
    hub.setting(&policy_key(session))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

pub(super) async fn set_policy(
    State(state): State<Arc<AppState>>,
    Path(session): Path<String>,
    Json(request): Json<crate::send_delivery::DeliveryPolicy>,
) -> Response {
    // A detached transaction finishes even when the initiating client closes
    // the picker. CAS makes a lost reply observable without overwriting a newer
    // preference. Settings never acquire or wake a worker.
    match tokio::spawn(async move {
        let _guard = POLICY_WRITES.lock().await;
        if state.hub.session_info(&session).is_none() {
            return StatusCode::NOT_FOUND.into_response();
        }
        let Some(store) = state.store.as_ref() else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        let key = policy_key(&session);
        let expected = state.hub.setting(&key);
        let current = policy(&state.hub, &session);
        if current.revision != request.revision {
            return (StatusCode::CONFLICT, Json(current)).into_response();
        }
        let Some(revision) = current.revision.checked_add(1) else {
            return StatusCode::CONFLICT.into_response();
        };
        let saved = crate::send_delivery::DeliveryPolicy {
            mode: request.mode,
            revision,
        };
        let value = serde_json::to_value(&saved).expect("delivery policy serializes");
        match store
            .compare_and_put_setting(&key, expected.as_ref(), &value)
            .await
        {
            Ok(true) => {
                // Already durable: do not enqueue a second write behind other
                // mutations. The persisted snapshot is loaded on restart.
                state.hub.load_settings(vec![(key, value)]);
                Json(saved).into_response()
            }
            Ok(false) => StatusCode::CONFLICT.into_response(),
            Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        }
    })
    .await
    {
        Ok(response) => response,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

pub(super) fn begin(
    state: &AppState,
    session: &str,
    text: &str,
    content: &[serde_json::Value],
    cmid: String,
    target: Option<&crate::send_delivery::DeliveryTarget>,
    next_checkpoint: bool,
) -> Result<crate::remote_runtime::SteeringDelivery, String> {
    if crate::session_context::pending(&state.hub, session) {
        return Err("Context changes are pending. Wait for this turn to finish, then send normally, or cancel those changes.".into());
    }
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
    let delivery = state.supervisor.start_steer(
        session,
        blocks
            .into_iter()
            .map(|block| serde_json::to_value(block).expect("ACP content serializes"))
            .collect(),
        cmid,
        target,
        next_checkpoint,
    )?;
    // A steered message is sent too: it ends the Back window.
    super::config_revert::clear(&state.hub, session);
    Ok(delivery)
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
    serde_json::json!({"status":"pending","outcome":"unknown"})
}

async fn release_hold(state: &AppState, session: &str, operation: &str) -> bool {
    let Some(store) = state.store.as_ref() else {
        return false;
    };
    let key = format!(
        "{}{session}",
        crate::core::settings_keys::SESSION_DELIVERY_HOLD_PREFIX
    );
    if !matches!(
        store
            .compare_and_put_setting(
                &key,
                Some(&serde_json::json!(operation)),
                &serde_json::Value::Null
            )
            .await,
        Ok(true)
    ) {
        return false;
    }
    state.hub.publish_delivery_hold(session, None);
    true
}

#[derive(Deserialize)]
pub(super) struct ReleaseHold {
    operation: String,
}

pub(super) async fn release(
    State(state): State<Arc<AppState>>,
    Path(session): Path<String>,
    Json(request): Json<ReleaseHold>,
) -> Response {
    let _guard = POLICY_WRITES.lock().await;
    if state.hub.delivery_hold(&session).as_deref() != Some(&request.operation)
        || state.hub.status(&session) != Some(Status::Running)
        || state.supervisor.delivery_controls(&session).version == 0
        || state.supervisor.delivery_target(&session).is_some()
    {
        return (
            StatusCode::CONFLICT,
            "Hold release requires the matching operation and an idle worker",
        )
            .into_response();
    }
    if release_hold(&state, &session, &request.operation).await {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::SERVICE_UNAVAILABLE.into_response()
    }
}

async fn interrupt_once(
    state: &AppState,
    session: &str,
    request: &SessionPromptRequest,
) -> serde_json::Value {
    let refuse = |error: &str| serde_json::json!({"status":"not_delivered","error":error});
    let Some(target) = request.target.as_ref() else {
        return refuse("interrupt requires an exact turn target");
    };
    let operation = request.operation.as_deref().expect("validated operation");
    if build_prompt_blocks(&request.text, &request.content).is_none() {
        return refuse("empty prompt");
    }
    if state.supervisor.delivery_target(session).as_ref() != Some(target) {
        return refuse("the captured turn has changed");
    }
    let store = state.store.as_ref().expect("validated store");
    let key = format!(
        "{}{session}",
        crate::core::settings_keys::SESSION_DELIVERY_HOLD_PREFIX
    );
    // Serialize hold publication with settings writes. The durable hold blocks
    // drain after restart too; an unresolved cancellation never becomes replay.
    {
        let _guard = POLICY_WRITES.lock().await;
        if state.hub.delivery_hold(session).is_some() {
            return refuse("another interrupt is unresolved; inspect its receipt");
        }
        let expected = state.hub.setting(&key);
        if !matches!(
            store
                .compare_and_put_setting(&key, expected.as_ref(), &serde_json::json!(operation))
                .await,
            Ok(true)
        ) {
            return refuse("could not reserve interrupt delivery");
        }
        state
            .hub
            .publish_delivery_hold(session, Some(operation.to_owned()));
    }
    let effect = {
        let fences = state.plugin_lifecycle_fences.read();
        if let Some(refusal) = provider_fence_key_for_session(&state.hub, session)
            .as_ref()
            .and_then(|key| fences.get(key))
            .and_then(|fence| provider_fence_refusal(*fence))
        {
            Err(refusal.to_owned())
        } else {
            state
                .supervisor
                .start_cancel_turn(session, target, operation.to_owned())
        }
    };
    let effect = match effect {
        Ok(effect) => effect,
        Err(error) => {
            release_hold(state, session, operation).await;
            return refuse(&error);
        }
    };
    match effect.outcome().await.as_str() {
        "not_delivered" | "unsupported" => {
            release_hold(state, session, operation).await;
            return refuse("the captured turn could not be cancelled");
        }
        "cancel_requested" => {}
        _ => {
            return serde_json::json!({"status":"unknown","error":"Cancellation acknowledgement is unknown; queue held."});
        }
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        match state.supervisor.delivery_turn_finished(session, target) {
            Ok(true) => break,
            Ok(false) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await
            }
            _ => {
                return serde_json::json!({"status":"unknown","error":"Turn settlement is unknown; queue held."});
            }
        }
    }
    let _guard = POLICY_WRITES.lock().await;
    let fences = state.plugin_lifecycle_fences.read();
    if provider_fence_key_for_session(&state.hub, session)
        .as_ref()
        .and_then(|key| fences.get(key))
        .and_then(|fence| provider_fence_refusal(*fence))
        .is_some()
    {
        return serde_json::json!({"status":"unknown","error":"Provider maintenance began after cancellation; queue held."});
    }
    if !state.hub.dispatch_interrupt_once(
        session,
        operation,
        request.text.clone(),
        request.content.clone(),
    ) {
        return serde_json::json!({"status":"unknown","error":"Replacement admission is unresolved; queue held."});
    }
    serde_json::json!({"status":"submitted"})
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
        "delivery":request.delivery, "text":request.text, "content":request.content,
        "target":request.target,
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
    let pending = serde_json::json!({"status":"pending","delivery":request.delivery,"operation":request.operation});
    let task = tokio::spawn(async move {
        let _permit = permit;
        let mut receipt = if request.delivery == Some(crate::send_delivery::SendDelivery::Interrupt)
        {
            interrupt_once(&state, &session, &request).await
        } else {
            let effect = begin(
                &state,
                &session,
                &request.text,
                &request.content,
                request.operation.clone().expect("validated operation"),
                request.target.as_ref(),
                request.delivery == Some(crate::send_delivery::SendDelivery::NextCheckpoint),
            );
            match effect {
                Ok(effect) => result(&effect.outcome().await),
                Err(error) => {
                    serde_json::json!({"status":"not_delivered","delivery":"current_turn","error":error})
                }
            }
        };
        receipt["operation"] = serde_json::json!(request.operation);
        receipt["delivery"] = serde_json::json!(request.delivery);
        if store
            .finish_agent_operation(&owner, &session, &operation, &receipt)
            .await
            .is_err()
        {
            return result("unknown");
        }
        if receipt["status"] == "submitted" {
            release_hold(
                &state,
                &session,
                request.operation.as_deref().expect("validated operation"),
            )
            .await;
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

pub(super) async fn options(
    State(state): State<Arc<AppState>>,
    Path(session): Path<String>,
) -> Response {
    let Some(info) = state.hub.session_info(&session) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    Json(serde_json::json!({
        "target": state.supervisor.delivery_target(&session),
        "controls": state.supervisor.delivery_controls(&session),
        "steer": info.meta.supports_current_turn,
        "status": info.meta.status,
        "paused": info.meta.paused,
        "hold": state.hub.delivery_hold(&session),
        "policy": policy(&state.hub, &session),
    }))
    .into_response()
}
