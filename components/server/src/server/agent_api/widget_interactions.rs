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
const READ_INTERACTIONS: i64 = 20;
/// How long a recorded decision may wait for its prompt to be re-queued
/// after a Controller restart.
const RECOVERY_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

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

pub(super) fn refusal(status: StatusCode, code: &str, extra: Value) -> Response {
    let mut body = json!({"code": code});
    if let (Some(map), Value::Object(extra)) = (body.as_object_mut(), extra) {
        map.extend(extra);
    }
    (status, Json(body)).into_response()
}

pub(super) fn unavailable() -> Response {
    refusal(StatusCode::SERVICE_UNAVAILABLE, "unavailable", json!({}))
}

pub(super) fn digest(message: &str) -> String {
    format!("{:x}", Sha256::digest(message.as_bytes()))
}

fn decided_ref(record: &StoredInteraction) -> Decided<'_> {
    Decided {
        node_id: &record.node_id,
        group_key: &record.group_key,
        signature: &record.signature,
        labels: &record.labels,
        created_at_ms: record.created_at_ms,
    }
}

fn decided_from(widget: &StoredWidget, records: &[StoredInteraction]) -> Map<String, Value> {
    let decided: Vec<Decided<'_>> = records.iter().map(decided_ref).collect();
    interactions::decided_inputs(&widget.document, &decided)
}

/// The widget's active decisions: never truncated, because they are the
/// authoritative decided state (at most one per input group stays active).
async fn active(
    store: &crate::store::Store,
    session_id: &str,
    widget_id: &str,
) -> anyhow::Result<Vec<StoredInteraction>> {
    store
        .widget_interactions(session_id, widget_id, true, i64::MAX)
        .await
}

/// The inputs of the widget's current document that a decision settled.
pub(super) async fn decided(
    store: &crate::store::Store,
    session_id: &str,
    widget: &StoredWidget,
) -> Map<String, Value> {
    match active(store, session_id, &widget.widget_id).await {
        Ok(records) => decided_from(widget, &records),
        Err(error) => {
            tracing::warn!(session = %session_id, %error, "widget interactions unavailable");
            Map::new()
        }
    }
}

/// After a widget update: retire the decisions whose input changed (so a
/// later revision restoring it asks again), then the inputs still decided.
pub(super) async fn refresh(
    store: &crate::store::Store,
    session_id: &str,
    widget: &StoredWidget,
) -> Map<String, Value> {
    let Ok(records) = active(store, session_id, &widget.widget_id).await else {
        return Map::new();
    };
    let obsolete: Vec<String> = records
        .iter()
        .filter(|record| !interactions::still_live(&widget.document, &decided_ref(record)))
        .map(|record| record.cmid.clone())
        .collect();
    if !obsolete.is_empty()
        && let Err(error) = store
            .retire_widget_interactions(session_id, &widget.widget_id, &obsolete)
            .await
    {
        tracing::warn!(session = %session_id, %error, "widget decisions not retired");
    }
    decided_from(widget, &records)
}

/// The newest decisions, for `widgets.read`.
pub(super) async fn recent(
    store: &crate::store::Store,
    session_id: &str,
    widget: &StoredWidget,
) -> Result<Value, String> {
    let records = store
        .widget_interactions(session_id, &widget.widget_id, false, READ_INTERACTIONS)
        .await
        .map_err(|_| "unavailable")?;
    Ok(Value::Array(
        records
            .iter()
            .map(|record| {
                json!({"revision":record.revision,"node_id":record.node_id,
                    "labels":record.labels,"message":record.message,"at_ms":record.created_at_ms})
            })
            .collect(),
    ))
}

/// Decisions recorded before a Controller restart whose prompt never
/// reached the transcript are queued again under their own `cmid`, the
/// guarantee the browser outbox gives a typed message.
pub(in crate::server) async fn recover_decisions(state: &AppState) {
    let Some(store) = state.store.as_ref() else {
        return;
    };
    let since = chrono::Utc::now().timestamp_millis() - RECOVERY_WINDOW_MS;
    let pending = match store.undelivered_widget_decisions(since).await {
        Ok(pending) => pending,
        Err(error) => {
            tracing::warn!(%error, "widget decision recovery unavailable");
            return;
        }
    };
    for (session_id, cmid, message) in pending {
        if admissible(state, &session_id).is_ok() {
            tracing::info!(session = %session_id, %cmid, "re-queued an undelivered widget decision");
            state
                .hub
                .submit(&session_id, message, Vec::new(), Some(cmid));
        }
    }
}

/// The session may take a person's prompt right now.
pub(super) fn admissible(
    state: &AppState,
    session_id: &str,
) -> Result<(), (StatusCode, &'static str)> {
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
    send(&state, store, &session_id, &widget, &decision).await
}

/// Record, queue and republish one confirmed decision.
async fn send(
    state: &AppState,
    store: &crate::store::Store,
    session_id: &str,
    widget: &StoredWidget,
    decision: &interactions::Decision,
) -> Response {
    let cmid =
        interactions::decision_cmid(session_id, &widget.widget_id, widget.revision, decision);
    let sent = || {
        (
            StatusCode::OK,
            Json(
                json!({"status":"sent","cmid":cmid,"message":decision.message,
                "labels":decision.labels}),
            ),
        )
            .into_response()
    };
    let queue = || {
        state.hub.submit(
            session_id,
            decision.message.clone(),
            Vec::new(),
            Some(cmid.clone()),
        );
    };
    match store.widget_interaction(session_id, &cmid).await {
        Ok(Some(_)) => {
            // The same decision again: answer from its record. Queue it again
            // only while no stored echo proves delivery; the queue drops a
            // duplicate it still holds.
            match store.prompt_echoed(session_id, &cmid).await {
                Ok(true) => {}
                Ok(false) => queue(),
                Err(_) => return unavailable(),
            }
            return sent();
        }
        Ok(None) => {}
        Err(_) => return unavailable(),
    }
    let Ok(records) = active(store, session_id, &widget.widget_id).await else {
        return unavailable();
    };
    if let Some(settled) = decided_from(widget, &records).get(&decision.node_id) {
        return refusal(StatusCode::CONFLICT, "decided", json!({"decided": settled}));
    }
    // Record first: the durable intent survives a crash before the queue or
    // the transcript holds the prompt, and recovery or a retried tap queues
    // it again under the same `cmid`. A widget cleared meanwhile records
    // nothing and sends nothing.
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
    let updated = match store
        .record_widget_interaction(session_id, &widget.widget_id, &record)
        .await
    {
        Ok(Some(updated)) => updated,
        Ok(None) => return refusal(StatusCode::CONFLICT, "cleared", json!({})),
        Err(error) => {
            tracing::warn!(session = %session_id, %error, "widget interaction record failed");
            return unavailable();
        }
    };
    queue();
    let decided = decided(store, session_id, &updated).await;
    super::widgets::push(state, session_id, &updated, "interaction", decided);
    sent()
}
