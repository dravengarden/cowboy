//! Messages an app asks to send (docs/widget-apps.md §7). The app's text is
//! shown to the person, labelled as written by the app; the confirmed send
//! carries its digest and is queued as the person's ordinary prompt. Like a
//! decision, it is recorded before it is queued, retried only while no stored
//! echo proves delivery, and re-queued after a Controller restart. One
//! confirmation (`occurrence`) is one message: two confirmed sends of the same
//! text are two prompts, a retried one is one.
use super::widget_interactions::{admissible, digest, refusal, unavailable};
use super::*;
use crate::store::session_widgets::interactions::StoredInteraction;
use crate::widgets::interactions;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct AppMessageRequest {
    code_sha256: String,
    /// Host-owned id of one confirmation, kept across retries of that send.
    occurrence: String,
    text: String,
    /// Present on the confirmed send: the digest of the previewed text.
    #[serde(default)]
    message_sha256: Option<String>,
}

fn valid_occurrence(occurrence: &str) -> bool {
    (8..=64).contains(&occurrence.len())
        && occurrence
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// `POST /api/sessions/{id}/widgets/{widget_id}/apps/{node_id}/messages`.
pub(in crate::server) async fn api_widget_app_message(
    State(state): State<Arc<AppState>>,
    Path((session_id, widget_id, node_id)): Path<(String, String, String)>,
    Json(request): Json<AppMessageRequest>,
) -> Response {
    if let Err((status, code)) = admissible(&state, &session_id) {
        return refusal(status, code, json!({}));
    }
    if !valid_occurrence(&request.occurrence) {
        return refusal(StatusCode::BAD_REQUEST, "invalid_occurrence", json!({}));
    }
    let Some(store) = state.store.as_ref() else {
        return unavailable();
    };
    let _order = super::widgets::WIDGET_WRITES.lock().await;
    let widget = match store.session_widget(&session_id, &widget_id).await {
        Ok(Some(widget)) => widget,
        Ok(None) => return refusal(StatusCode::NOT_FOUND, "unknown_widget", json!({})),
        Err(_) => return unavailable(),
    };
    if widget.closed {
        return refusal(StatusCode::CONFLICT, "closed", json!({}));
    }
    let message = match interactions::app_message(
        &widget.document,
        &node_id,
        &request.code_sha256,
        &request.text,
    ) {
        Ok(message) => message,
        Err(refused) => {
            let status = if refused.code == "app_updated" {
                StatusCode::CONFLICT
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            return refusal(status, refused.code, json!({"message": refused.message}));
        }
    };
    let message_sha256 = digest(&request.text);
    let Some(confirmed) = request.message_sha256 else {
        return (
            StatusCode::OK,
            Json(json!({"message": request.text, "message_sha256": message_sha256})),
        )
            .into_response();
    };
    if confirmed != message_sha256 {
        return refusal(StatusCode::CONFLICT, "changed", json!({}));
    }
    let cmid = interactions::app_message_cmid(
        &session_id,
        &widget_id,
        &node_id,
        &message.signature,
        &request.occurrence,
    );
    let queue = || {
        state.hub.submit(
            &session_id,
            request.text.clone(),
            Vec::new(),
            Some(cmid.clone()),
        );
    };
    let sent = || {
        (
            StatusCode::OK,
            Json(json!({"status": "sent", "cmid": cmid, "message": request.text})),
        )
            .into_response()
    };
    match store.widget_interaction(&session_id, &cmid).await {
        Ok(Some(existing)) if existing.message != request.text => {
            return refusal(StatusCode::CONFLICT, "occurrence_reused", json!({}));
        }
        Ok(Some(_)) => {
            match store.prompt_echoed(&session_id, &cmid).await {
                Ok(true) => {}
                Ok(false) => queue(),
                Err(_) => return unavailable(),
            }
            return sent();
        }
        Ok(None) => {}
        Err(_) => return unavailable(),
    }
    let record = StoredInteraction {
        cmid: cmid.clone(),
        revision: widget.revision,
        node_id: node_id.clone(),
        group_key: message.group_key,
        signature: message.signature,
        labels: Vec::new(),
        message: request.text.clone(),
        created_at_ms: chrono::Utc::now().timestamp_millis(),
    };
    let updated = match store
        .record_widget_interaction(&session_id, &widget_id, &record)
        .await
    {
        Ok(Some(updated)) => updated,
        Ok(None) => return refusal(StatusCode::CONFLICT, "cleared", json!({})),
        Err(error) => {
            tracing::warn!(session = %session_id, %error, "widget app message record failed");
            return unavailable();
        }
    };
    queue();
    let decided = super::widget_interactions::decided(store, &session_id, &updated).await;
    super::widgets::push(&state, &session_id, &updated, "app_message", decided);
    sent()
}
