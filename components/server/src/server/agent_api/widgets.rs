//! `widgets.*` (docs/widgets.md §6): agent-published structured UI. The
//! Controller validates and stores the document, then projects it into the
//! parent transcript as one anchored notice row that later writes replace in
//! place. Interactions are not admitted yet; inputs render inert.
use super::*;
use crate::store::agent_api_operations::Admission;
use crate::store::session_widgets::{PublishOutcome, StoredWidget, UpdateOutcome, WidgetWrite};
use crate::widgets;

/// Orders each widget's store write before its transcript push, so a later
/// snapshot can never be fanned out before an earlier one.
pub(super) static WIDGET_WRITES: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// A write whose commit state is unknown: log the cause, report it opaquely.
fn unknown(session: &str, error: impl std::fmt::Display) -> &'static str {
    tracing::warn!(%session, %error, "widget write outcome unknown");
    "outcome_unknown"
}

fn success(value: Value) -> Value {
    json!({"schema":1,"state":"ok","value":value})
}

fn operation_key(operation: &str) -> String {
    format!("widgets:{operation}")
}

fn meta_view(widget: &StoredWidget) -> Value {
    json!({
        "widget_id": widget.widget_id,
        "handle": widget.handle,
        "title": widget.document["title"],
        "summary": widget.document.get("summary").cloned().unwrap_or(Value::Null),
        "revision": widget.revision,
        "state_version": widget.state_version,
        "closed": widget.closed,
        "updated_at_ms": widget.updated_at_ms,
    })
}

pub(super) fn push(
    state: &AppState,
    session_id: &str,
    widget: &StoredWidget,
    operation: &str,
    decided: Map<String, Value>,
) {
    let title = widget.document["title"]
        .as_str()
        .unwrap_or("Widget")
        .to_owned();
    let update = json!({
        "sessionUpdate": "notice",
        "severity": "info",
        "title": title,
        "description": widgets::description(&widget.document),
        "_cowboy": {"widget": {
            "widgetId": widget.widget_id,
            "handle": widget.handle,
            "revision": widget.revision,
            "stateVersion": widget.state_version,
            "publishedAtMs": widget.created_at_ms,
            "updatedAtMs": widget.updated_at_ms,
            "origin": {"kind": "mcp", "operation": operation},
            "closed": widget.closed,
            "decided": decided,
            "document": widget.document,
        }},
    });
    state.hub.push(session_id, Event::Update { update });
}

// Every result carries `schema`: the call gateway reports a reply without it as
// `outcome_unknown`, hiding a definite refusal the ledger already recorded.
fn invalid(error: &widgets::Invalid) -> Value {
    json!({"schema":1,"state":"error","error":{"code":"invalid_widget","path":error.path,"message":error.message}})
}

fn outcome(code: &str, extra: Value) -> Value {
    let mut value = json!({"schema":1,"state":"error","error":{"code":code}});
    if let (Some(map), Value::Object(extra)) = (value.as_object_mut(), extra) {
        map.extend(extra);
    }
    value
}

fn generated_id(operation: &str) -> String {
    use sha2::{Digest as _, Sha256};
    format!(
        "w-{}",
        &format!("{:x}", Sha256::digest(operation.as_bytes()))[..16]
    )
}

async fn publish(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    args: &Value,
    operation: &str,
    now: i64,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    only_keys(args, &["widget_id", "document"]).map_err(|_| "invalid_argument")?;
    let widget_id = match args.get("widget_id") {
        None => generated_id(operation),
        Some(Value::String(id)) if widgets::valid_id(id) => id.clone(),
        Some(_) => {
            return Ok(outcome(
                "invalid_argument",
                json!({"message":"widget_id must match [A-Za-z0-9_-]{1,64}"}),
            ));
        }
    };
    let mut document = args.get("document").cloned().unwrap_or(Value::Null);
    if let Err(error) = widgets::validate(&document) {
        return Ok(invalid(&error));
    }
    if let Some(refusal) = super::widget_images::resolve(state, meta, &mut document).await? {
        return Ok(refusal);
    }
    let _order = WIDGET_WRITES.lock().await;
    match store
        .publish_session_widget(&meta.id, &widget_id, &document, now)
        .await
        .map_err(|error| unknown(&meta.id, error))?
    {
        PublishOutcome::Committed(widget) => {
            push(state, &meta.id, &widget, operation, Map::new());
            Ok(success(
                json!({"status":"committed","widget_id":widget.widget_id,"handle":widget.handle,
                "revision":widget.revision,"state_version":widget.state_version}),
            ))
        }
        PublishOutcome::Exists(widget) => {
            Ok(outcome("exists", json!({"widget":meta_view(&widget)})))
        }
        PublishOutcome::Limit => Ok(outcome(
            "widget_limit",
            json!({"message":"at most 200 open and 500 total widgets per session"}),
        )),
    }
}

async fn effect(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    action: &str,
    args: &Value,
    operation: &str,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let now = chrono::Utc::now().timestamp_millis();
    match action {
        "widgets.publish" => publish(state, meta, args, operation, now).await,
        "widgets.update" => {
            only_keys(args, &["widget_id", "expected_revision", "change"])
                .map_err(|_| "invalid_argument")?;
            let widget_id = args
                .get("widget_id")
                .and_then(Value::as_str)
                .filter(|id| widgets::valid_id(id))
                .ok_or("invalid_argument")?;
            let expected = args
                .get("expected_revision")
                .and_then(Value::as_i64)
                .ok_or("invalid_argument")?;
            let change = args
                .get("change")
                .and_then(Value::as_object)
                .ok_or("invalid_argument")?;
            let _order = WIDGET_WRITES.lock().await;
            let mut write = match change.get("type").and_then(Value::as_str) {
                Some("replace") => {
                    let document = change.get("document").cloned().unwrap_or(Value::Null);
                    if let Err(error) = widgets::validate(&document) {
                        return Ok(invalid(&error));
                    }
                    WidgetWrite::Document(document)
                }
                Some("patch") => {
                    let current = store
                        .session_widget(&meta.id, widget_id)
                        .await
                        .map_err(|_| "unavailable")?;
                    let Some(current) = current else {
                        return Ok(outcome("unknown_widget", json!({})));
                    };
                    if current.revision != expected {
                        return Ok(outcome(
                            "revision_conflict",
                            json!({"current":current.revision}),
                        ));
                    }
                    let ops = change
                        .get("ops")
                        .and_then(Value::as_array)
                        .ok_or("invalid_argument")?;
                    match widgets::apply_patch(&current.document, ops) {
                        Ok(document) => WidgetWrite::Document(document),
                        Err(error) => return Ok(invalid(&error)),
                    }
                }
                Some("close") => {
                    let summary = match change.get("summary") {
                        None => None,
                        Some(Value::String(summary)) => Some(summary.clone()),
                        Some(_) => return Err("invalid_argument"),
                    };
                    WidgetWrite::Close(summary)
                }
                _ => {
                    return Ok(outcome(
                        "invalid_argument",
                        json!({"message":"change.type is replace, patch or close"}),
                    ));
                }
            };
            if let WidgetWrite::Document(document) = &mut write
                && let Some(refusal) = super::widget_images::resolve(state, meta, document).await?
            {
                return Ok(refusal);
            }
            if let WidgetWrite::Close(Some(summary)) = &write {
                let current = store
                    .session_widget(&meta.id, widget_id)
                    .await
                    .map_err(|_| "unavailable")?;
                if let Some(mut probe) = current.map(|w| w.document) {
                    probe["summary"] = json!(summary);
                    if let Err(error) = widgets::validate(&probe) {
                        return Ok(invalid(&error));
                    }
                }
            }
            match store
                .update_session_widget(&meta.id, widget_id, expected, write, now)
                .await
                .map_err(|error| unknown(&meta.id, error))?
            {
                UpdateOutcome::Committed(widget) => {
                    let decided =
                        super::widget_interactions::decided(store, &meta.id, &widget).await;
                    push(state, &meta.id, &widget, operation, decided);
                    Ok(success(
                        json!({"status":"committed","widget_id":widget.widget_id,"revision":widget.revision,
                        "state_version":widget.state_version,"closed":widget.closed}),
                    ))
                }
                UpdateOutcome::Conflict(current) => {
                    Ok(outcome("revision_conflict", json!({"current":current})))
                }
                UpdateOutcome::Unknown => Ok(outcome("unknown_widget", json!({}))),
                UpdateOutcome::Closed => Ok(outcome("closed", json!({}))),
            }
        }
        _ => Err("unknown_operation"),
    }
}

/// `widgets.publish` / `widgets.update`: admitted once per operation; a replay
/// returns the recorded result and never writes twice.
pub(super) async fn execute(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    action: &str,
    args: Value,
    operation: &str,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let owner = meta.owner_user_id.clone().unwrap_or_default();
    let key = operation_key(operation);
    let hash = crate::managed_calls::canonical_digest(&json!({"action":action,"arguments":args}));
    match store
        .admit_agent_operation(&owner, &meta.id, &key, &hash)
        .await
        .map_err(|_| "unavailable")?
    {
        Admission::Existing(Some(result)) => return Ok(result),
        Admission::Existing(None) => {
            return Ok(outcome(
                "outcome_unknown",
                json!({"message":"read the widget to learn whether this write applied"}),
            ));
        }
        Admission::Conflict => return Err("operation_conflict"),
        Admission::New => {}
    }
    let result = effect(state, meta, action, &args, operation).await?;
    store
        .finish_agent_operation(&owner, &meta.id, &key, &result)
        .await
        .map_err(|error| unknown(&meta.id, error))?;
    Ok(result)
}

pub(super) async fn receipt(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    operation: &str,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let owner = meta.owner_user_id.clone().unwrap_or_default();
    Ok(
        match store
            .agent_operation_receipt(&owner, &meta.id, &operation_key(operation))
            .await
            .map_err(|_| "unavailable")?
        {
            Some(Some(result)) => success(json!({"status":"finished","result":result})),
            Some(None) => success(json!({"status":"pending","outcome":"unknown"})),
            None => success(json!({"status":"unknown"})),
        },
    )
}

/// `widgets.list` / `widgets.read` for the calling session.
pub(super) async fn read(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    op: &str,
    args: &Value,
) -> Result<Value, String> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    if op == "widgets.list" {
        only_keys(args, &["after", "limit", "include_closed"])?;
        let limit = page_limit(args, 50, 200)?;
        let include_closed = optional_bool(args, "include_closed")?;
        let after = after_id(args)?;
        let widgets = store
            .session_widgets(
                &meta.id,
                after,
                i64::try_from(limit + 1).unwrap_or(i64::MAX),
            )
            .await
            .map_err(|_| "unavailable")?;
        let more = widgets.len() > limit;
        let next = more.then(|| widgets[limit - 1].widget_id.clone());
        let items: Vec<Value> = widgets
            .iter()
            .take(limit)
            .filter(|w| include_closed || !w.closed)
            .map(meta_view)
            .collect();
        return Ok(json!({"widgets":items,"next":next}));
    }
    only_keys(args, &["widget_id", "offset", "chars"])?;
    let id = arg_str(args, "widget_id")?;
    let widget = store
        .session_widget(&meta.id, id)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("unknown_widget")?;
    let mut value = meta_view(&widget);
    value["interactions"] = super::widget_interactions::recent(store, &meta.id, &widget).await?;
    value["document"] = json!(serde_json::to_string(&widget.document).map_err(|_| "unavailable")?);
    text_window(&mut value, "document", args)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_keep_the_gateway_schema() {
        let refusal = outcome("revision_conflict", json!({"current": 2}));
        assert_eq!(refusal["schema"], 1);
        assert_eq!(refusal["current"], 2);
        let error = widgets::validate(&json!({})).unwrap_err();
        assert_eq!(invalid(&error)["schema"], 1);
    }
}
