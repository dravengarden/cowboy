//! Transcript rows for signed views (docs/widget-apps-mcp.md §8). When the
//! relay delivers an agent-originated `tools/call` result for a tool whose
//! signed view the session grants, the Controller publishes a widget whose
//! root is an `mcp_app` node. Only the Controller writes such a node: agents
//! can neither publish nor patch one. View-originated calls never publish.
use std::sync::Arc;

use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use super::super::AppState;
use super::apps::current_grant;
use crate::store::session_widgets::PublishOutcome;
use crate::widgets::{self, MCP_APP_INPUT_BYTES, MCP_APP_RESULT_BYTES};

const FALLBACK_CHARS: usize = 2000;

/// Called after the relay delivered `response` to the agent; publishing is
/// detached so it never delays the agent's call.
pub(super) fn after_agent_call(
    state: &Arc<AppState>,
    session: &str,
    binding: &str,
    message: &Value,
    response: &Value,
) {
    if message["method"] != "tools/call" {
        return;
    }
    let Some(tool) = message["params"]["name"].as_str() else {
        return;
    };
    let Some(result) = response["response"]["result"].as_object() else {
        return;
    };
    if result.get("isError") == Some(&Value::Bool(true)) {
        return;
    }
    let Some(row) = row(
        &state.hub,
        &state.plugin_catalog,
        session,
        binding,
        tool,
        &message["params"],
        result,
    ) else {
        return;
    };
    let state = Arc::clone(state);
    let session = session.to_owned();
    tokio::spawn(async move { publish(&state, &session, row).await });
}

/// The row's document, if `tool` has a signed view this session grants.
pub(super) fn row(
    hub: &crate::core::Hub,
    catalog: &crate::plugin_catalog::PluginCatalog,
    session: &str,
    binding: &str,
    tool: &str,
    params: &Value,
    result: &Map<String, Value>,
) -> Option<Value> {
    let stored = super::Stored::load(hub, session)?;
    let selected = stored.bindings.iter().find(|b| b.name == binding)?;
    let contract = catalog
        .resolve_mcp_server(
            &selected.server.plugin_id,
            &selected.server.generation_digest,
        )
        .ok()?;
    let uri = contract.ui_resource_for(tool)?.uri.clone();
    let granted = current_grant(hub, catalog, session, binding, &uri).ok()?;
    let input = params.get("arguments").cloned().unwrap_or(json!({}));
    Some(document(
        binding,
        tool,
        &uri,
        &granted.epoch,
        &format!("{:x}", Sha256::digest(granted.html.as_bytes())),
        &input,
        result,
    ))
}

fn bounded(value: &Value, max: usize) -> Option<String> {
    serde_json::to_string(value)
        .ok()
        .filter(|text| text.len() <= max)
}

fn fallback(tool: &str, result: &Map<String, Value>) -> String {
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let text = if text.trim().is_empty() {
        format!("{tool} returned no text")
    } else {
        text
    };
    text.chars().take(FALLBACK_CHARS).collect()
}

fn document(
    binding: &str,
    tool: &str,
    uri: &str,
    epoch: &str,
    sha256: &str,
    input: &Value,
    result: &Map<String, Value>,
) -> Value {
    let truncated = json!({"_cowboy":{"truncated":true}}).to_string();
    let mut fallback = fallback(tool, result);
    let mut input = bounded(input, MCP_APP_INPUT_BYTES).unwrap_or_else(|| truncated.clone());
    let mut result = bounded(&Value::Object(result.clone()), MCP_APP_RESULT_BYTES);
    let title: String = format!("{binding} · {tool}").chars().take(120).collect();
    // Embedded JSON text is escaped again in the document, so the stored
    // size is checked whole; the result goes first, then the input, then
    // most of the fallback, until the row fits.
    for step in 0..4 {
        let document = json!({
            "schema": widgets::SCHEMA,
            "title": title,
            "root": {
                "type": "mcp_app",
                "id": "view",
                "title": title,
                "fallback": fallback,
                "binding": binding,
                "tool": tool,
                "uri": uri,
                "sha256": sha256,
                "epoch": epoch,
                "input": input,
                "result": result,
            },
        });
        if serde_json::to_vec(&document).map_or(usize::MAX, |bytes| bytes.len())
            <= widgets::DOCUMENT_BYTES
            || step == 3
        {
            return document;
        }
        match step {
            0 => result = None,
            1 => input.clone_from(&truncated),
            _ => fallback = fallback.chars().take(400).collect(),
        }
    }
    unreachable!()
}

async fn publish(state: &Arc<AppState>, session: &str, document: Value) {
    if let Err(error) = widgets::validate_cowboy(&document) {
        tracing::warn!(%session, path = %error.path, message = %error.message, "mcp_app row refused");
        return;
    }
    let Some(store) = state.store.as_ref() else {
        return;
    };
    let widget_id = format!("mcp-{:016x}", rand::random::<u64>());
    let now = chrono::Utc::now().timestamp_millis();
    let _order = super::super::agent_api::WIDGET_WRITES.lock().await;
    match store
        .publish_session_widget(session, &widget_id, &document, now)
        .await
    {
        Ok(PublishOutcome::Committed(widget)) => {
            super::super::agent_api::push_widget(state, session, &widget, "mcp_app", Map::new());
        }
        Ok(PublishOutcome::Exists(_) | PublishOutcome::Limit) => {}
        Err(error) => tracing::warn!(%session, %error, "mcp_app row not published"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(text: &str) -> Map<String, Value> {
        json!({"content":[{"type":"text","text":text}]})
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn rows_validate_only_as_cowboy_documents_and_bound_payloads() {
        let doc = document(
            "suger",
            "suger_status",
            "ui://suger/status",
            &"e".repeat(32),
            &"a".repeat(64),
            &json!({"q":1}),
            &result("ok"),
        );
        widgets::validate_cowboy(&doc).unwrap();
        let refused = widgets::validate(&doc).unwrap_err();
        assert!(refused.message.contains("only by Cowboy"), "{refused:?}");
        assert_eq!(doc["root"]["fallback"], "ok");
        assert_eq!(doc["root"]["input"], r#"{"q":1}"#);

        let big = "x".repeat(MCP_APP_RESULT_BYTES);
        let doc = document(
            "suger",
            "suger_status",
            "ui://suger/status",
            &"e".repeat(32),
            &"a".repeat(64),
            &json!({"q": "y".repeat(MCP_APP_INPUT_BYTES)}),
            &result(&big),
        );
        widgets::validate_cowboy(&doc).unwrap();
        assert!(
            doc["root"]["result"].is_null(),
            "an oversized result is dropped"
        );
        assert!(doc["root"]["input"].as_str().unwrap().contains("truncated"));
        assert_eq!(
            doc["root"]["fallback"].as_str().unwrap().chars().count(),
            FALLBACK_CHARS
        );
    }
}
