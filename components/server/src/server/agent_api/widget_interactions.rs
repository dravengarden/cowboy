//! Widget decisions from the product UI (docs/widgets.md §8, revised): a tap
//! becomes an ordinary user prompt in the session's queue. The page first
//! previews the exact message the Controller composed; the confirmed send
//! carries that message's digest and is refused when the widget changed in
//! between. The prompt's client message id is derived from the decision, so
//! a retried tap is the same prompt and the queue drops the duplicate.
use super::*;
use crate::store::session_widgets::StoredWidget;
use crate::store::session_widgets::interactions::StoredInteraction;
use crate::widgets::interactions::{self, Decided};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use sha2::{Digest as _, Sha256};

/// Interaction records `widgets.read` returns, newest first.
const READ_INTERACTIONS: usize = 20;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct InteractionRequest {
    revision: i64,
    node_id: String,
    #[serde(default)]
    values: Vec<String>,
    /// Present on the confirmed send: the digest of the previewed message.
    #[serde(default)]
    message_sha256: Option<String>,
}

fn refusal(status: StatusCode, code: &str, extra: Value) -> Response {
    let mut body = json!({"code": code});
    if let (Some(map), Value::Object(extra)) = (body.as_object_mut(), extra) {
        map.extend(extra);
    }
    (status, Json(body)).into_response()
}

fn digest(message: &str) -> String {
    format!("{:x}", Sha256::digest(message.as_bytes()))
}

/// The inputs of the widget's current document that a decision settled.
pub(super) async fn decided(
    store: &crate::store::Store,
    session_id: &str,
    widget: &StoredWidget,
) -> Map<String, Value> {
    match store
        .widget_interactions(session_id, &widget.widget_id)
        .await
    {
        Ok(records) => decided_from(widget, &records),
        Err(error) => {
            tracing::warn!(session = %session_id, %error, "widget interactions unavailable");
            Map::new()
        }
    }
}

fn decided_from(widget: &StoredWidget, records: &[StoredInteraction]) -> Map<String, Value> {
    let decided: Vec<Decided<'_>> = records
        .iter()
        .map(|record| Decided {
            node_id: &record.node_id,
            group_key: &record.group_key,
            signature: &record.signature,
            labels: &record.labels,
            created_at_ms: record.created_at_ms,
        })
        .collect();
    interactions::decided_inputs(&widget.document, &decided)
}

/// The newest decisions, for `widgets.read`.
pub(super) async fn recent(
    store: &crate::store::Store,
    session_id: &str,
    widget: &StoredWidget,
) -> Result<Value, String> {
    let records = store
        .widget_interactions(session_id, &widget.widget_id)
        .await
        .map_err(|_| "unavailable")?;
    Ok(Value::Array(
        records
            .iter()
            .take(READ_INTERACTIONS)
            .map(|record| {
                json!({"revision":record.revision,"node_id":record.node_id,
                    "labels":record.labels,"message":record.message,"at_ms":record.created_at_ms})
            })
            .collect(),
    ))
}

/// The session may take a person's prompt right now.
fn admissible(state: &AppState, session_id: &str) -> Result<(), (StatusCode, &'static str)> {
    let Some(info) = state.hub.session_info(session_id) else {
        return Err((StatusCode::NOT_FOUND, "unknown_session"));
    };
    if info.meta.system
        || info.meta.trashed_at_ms.is_some()
        || info
            .meta
            .execution_binding
            .as_ref()
            .is_some_and(|binding| binding.managed().is_some())
    {
        return Err((StatusCode::CONFLICT, "session_unavailable"));
    }
    if crate::session_context::pending(&state.hub, session_id) {
        return Err((StatusCode::CONFLICT, "context_pending"));
    }
    Ok(())
}

/// `POST /api/sessions/{id}/widgets/{widget_id}/interactions`.
pub(in crate::server) async fn api_widget_interaction(
    State(state): State<Arc<AppState>>,
    Path((session_id, widget_id)): Path<(String, String)>,
    Json(request): Json<InteractionRequest>,
) -> Response {
    if let Err((status, code)) = admissible(&state, &session_id) {
        return refusal(status, code, json!({}));
    }
    let Some(store) = state.store.as_ref() else {
        return refusal(StatusCode::SERVICE_UNAVAILABLE, "unavailable", json!({}));
    };
    let _order = super::widgets::WIDGET_WRITES.lock().await;
    let widget = match store.session_widget(&session_id, &widget_id).await {
        Ok(Some(widget)) => widget,
        Ok(None) => return refusal(StatusCode::NOT_FOUND, "unknown_widget", json!({})),
        Err(_) => return refusal(StatusCode::SERVICE_UNAVAILABLE, "unavailable", json!({})),
    };
    if widget.closed {
        return refusal(StatusCode::CONFLICT, "closed", json!({}));
    }
    if widget.revision != request.revision {
        return refusal(
            StatusCode::CONFLICT,
            "stale",
            json!({"current": widget.revision}),
        );
    }
    let decision = match interactions::decide(
        &widget.document,
        &widget.handle,
        widget.revision,
        &request.node_id,
        &request.values,
    ) {
        Ok(decision) => decision,
        Err(refused) => {
            return refusal(
                StatusCode::UNPROCESSABLE_ENTITY,
                refused.code,
                json!({"message": refused.message}),
            );
        }
    };
    let message_sha256 = digest(&decision.message);
    let preview = json!({"message": decision.message, "message_sha256": message_sha256,
        "labels": decision.labels});
    let Some(confirmed) = request.message_sha256 else {
        return (StatusCode::OK, Json(preview)).into_response();
    };
    if confirmed != message_sha256 {
        return refusal(StatusCode::CONFLICT, "changed", preview);
    }
    let Ok(records) = store.widget_interactions(&session_id, &widget_id).await else {
        return refusal(StatusCode::SERVICE_UNAVAILABLE, "unavailable", json!({}));
    };
    let cmid = interactions::decision_cmid(&session_id, &widget_id, widget.revision, &decision);
    let retry = records.iter().any(|record| record.cmid == cmid);
    if !retry && let Some(settled) = decided_from(&widget, &records).get(&decision.node_id) {
        return refusal(StatusCode::CONFLICT, "decided", json!({"decided": settled}));
    }
    // Queue first: the queue is idempotent on `cmid`, so a crash before the
    // record commits is repaired by the retried tap, never sent twice.
    state.hub.submit(
        &session_id,
        decision.message.clone(),
        Vec::new(),
        Some(cmid.clone()),
    );
    let record = StoredInteraction {
        cmid: cmid.clone(),
        revision: widget.revision,
        node_id: decision.node_id.clone(),
        group_key: decision.group_key.clone(),
        signature: decision.signature.clone(),
        labels: decision.labels.clone(),
        message: decision.message.clone(),
        created_at_ms: chrono::Utc::now().timestamp_millis(),
    };
    match store
        .record_widget_interaction(&session_id, &widget_id, &record)
        .await
    {
        Ok(Some(updated)) => {
            let decided = decided(store, &session_id, &updated).await;
            super::widgets::push(&state, &session_id, &updated, "interaction", decided);
        }
        Ok(None) => {}
        Err(error) => {
            // The prompt is queued; the retried tap records it.
            tracing::warn!(session = %session_id, %error, "widget interaction record failed");
            return refusal(
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                json!({"cmid": cmid}),
            );
        }
    }
    (
        StatusCode::OK,
        Json(
            json!({"status":"sent","cmid":cmid,"message":decision.message,
            "labels":decision.labels}),
        ),
    )
        .into_response()
}
