//! One user-preference service shared by HTTP and the agent dispatcher.
use super::*;
use crate::store::preferences::{PreferenceChange, PreferenceResult};
use serde_json::{Value, json};

pub(super) async fn read_value(
    state: &AppState,
    owner: &str,
    scope: &str,
) -> Result<Value, &'static str> {
    let prefs = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .preferences(owner, scope)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("unknown_scope")?;
    if scope != "about" {
        return Ok(json!({"preferences":prefs}));
    }
    let machines = state
        .machine_snapshots
        .load()
        .await
        .map_err(|_| "unavailable")?;
    // Discover names from Plugin metadata; configuring layout must not warm
    // collectors or copy activity/usage payloads into the agent response.
    let mut usage = state.usage.plugin_bindings();
    usage.sort_by_key(|binding| binding.order.unwrap_or(u16::MAX));
    Ok(json!({"preferences":prefs,"choices":{
        "usage_order":usage.iter().map(|p| json!({"id":p.account,"name":p.product.as_deref().unwrap_or(&p.account)})).collect::<Vec<_>>(),
        "machine_order":machines.iter().map(|m| json!({"id":m.id,"name":m.display_name})).collect::<Vec<_>>()
    }}))
}

pub(super) async fn update_value(
    state: &AppState,
    owner: &str,
    operation: &str,
    change: &PreferenceChange,
) -> Result<Value, &'static str> {
    match state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .update_preferences(owner, operation, change)
        .await
        .map_err(|_| "outcome_unknown")?
    {
        PreferenceResult::Applied(prefs) => {
            state
                .hub
                .announce_preferences(owner, &prefs.scope, prefs.revision);
            Ok(json!({"status":"committed","preferences":prefs}))
        }
        PreferenceResult::Conflict(current) => {
            Ok(json!({"error":{"code":"revision_conflict"},"current":current}))
        }
        PreferenceResult::Invalid(code) => Err(code),
    }
}

pub(super) async fn read(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(scope): Path<String>,
) -> Response {
    match read_value(&state, &auth.principal.user_id, &scope).await {
        Ok(mut value) => {
            value["dataset"] =
                serde_json::to_value(sync_dataset::descriptor(&state.service_id, &auth.principal))
                    .unwrap();
            ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response()
        }
        Err(code) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":{"code":code}})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Update {
    operation: String,
    #[serde(default)]
    dataset: Option<String>,
    #[serde(flatten)]
    change: PreferenceChange,
}

pub(super) async fn update(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    headers: HeaderMap,
    Json(input): Json<Update>,
) -> Response {
    // Bind the write to the identity that read the editor, even after a cookie switch.
    if !headers
        .get("x-cowboy-dataset")
        .and_then(|v| v.to_str().ok())
        .or(input.dataset.as_deref())
        .is_some_and(|v| sync_dataset::matches(&state.service_id, &auth.principal, v))
    {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error":{"code":"dataset_changed"}})),
        )
            .into_response();
    }
    match update_value(
        &state,
        &auth.principal.user_id,
        &input.operation,
        &input.change,
    )
    .await
    {
        Ok(value) => (
            if value.get("error").is_some() {
                StatusCode::CONFLICT
            } else {
                StatusCode::OK
            },
            Json(value),
        )
            .into_response(),
        Err(code) => (
            if code == "outcome_unknown" || code == "unavailable" {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            },
            Json(json!({"error":{"code":code}})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_http_input_rejects_authority_and_unknown_fields() {
        let mut input = json!({"operation":"one","scope":"about","expected_revision":0,"changes":{"usage_order":[]}});
        assert!(serde_json::from_value::<Update>(input.clone()).is_ok());
        input["dataset"] = json!("observed-account-dataset");
        assert!(serde_json::from_value::<Update>(input.clone()).is_ok());
        input["owner"] = json!("victim");
        assert!(serde_json::from_value::<Update>(input).is_err());
    }
}
