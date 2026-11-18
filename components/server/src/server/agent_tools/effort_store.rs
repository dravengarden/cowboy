//! Review effort lives beside, not inside, the schema-one Tools documents.
//!
//! Older Controllers parse those documents with `deny_unknown_fields` and
//! fall back to the built-in Tools policy on any failure, which would re-enable
//! calls and memory a person had turned off. Keeping effort under its own keys
//! means a rollback only forgets the effort choice (back to Max first).

use crate::core::settings_keys::{
    AGENT_REVIEW_EFFORT_PREFIX, AGENT_TOOLS_PREFIX, SESSION_REVIEW_EFFORT_PREFIX,
    SESSION_TOOLS_PREFIX,
};
use serde_json::{Map, Value};

const EFFORT_KEYS: [&str; 2] = ["claude_effort", "gpt_effort"];

/// Remove effort fields from a Tools document; returns the document and the
/// effort object (null when there is none).
fn split(mut document: Value) -> (Value, Value) {
    let mut effort = Map::new();
    let mut emptied = false;
    if let Some(review) = document
        .pointer_mut("/calls/review")
        .and_then(Value::as_object_mut)
    {
        for key in EFFORT_KEYS {
            if let Some(value) = review.remove(key).filter(|value| !value.is_null()) {
                effort.insert(key.to_owned(), value);
            }
        }
        emptied = review.is_empty();
    }
    // A session override that only changed effort must not leave `review:{}`.
    if emptied
        && let Some(calls) = document
            .pointer_mut("/calls")
            .and_then(Value::as_object_mut)
    {
        calls.remove("review");
    }
    let effort = if effort.is_empty() {
        Value::Null
    } else {
        Value::Object(effort)
    };
    (document, effort)
}

/// Put stored effort back into a Tools document read from its own key.
fn merge(mut document: Value, effort: Option<Value>) -> Value {
    let Some(Value::Object(effort)) = effort else {
        return document;
    };
    let Some(calls) = document.get_mut("calls").and_then(Value::as_object_mut) else {
        return document;
    };
    let review = calls
        .entry("review")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(review) = review.as_object_mut() {
        for key in EFFORT_KEYS {
            if let Some(value) = effort.get(key) {
                review.insert(key.to_owned(), value.clone());
            }
        }
    }
    document
}

/// Commit both settings under one Hub lock so concurrent saves cannot pair
/// one caller's policy with another's effort; the store queue keeps order.
fn write_pair(
    hub: &crate::core::Hub,
    key: String,
    document: Value,
    effort_key: String,
    effort: Value,
) {
    let snapshot = hub.with_settings_mut(|settings| {
        settings.insert(effort_key.clone(), effort.clone());
        crate::core::Hub::commit_setting_locked(settings, key.clone(), document.clone())
    });
    hub.publish_setting(effort_key, effort, snapshot.clone());
    hub.publish_setting(key, document, snapshot);
}

fn store(hub: &crate::core::Hub, key: String, effort_key: String, document: Option<Value>) {
    let (document, effort) = document.map_or((Value::Null, Value::Null), split);
    write_pair(hub, key, document, effort_key, effort);
}

pub(in crate::server) fn agent_document(hub: &crate::core::Hub, agent: &str) -> Option<Value> {
    hub.setting(&format!("{AGENT_TOOLS_PREFIX}{agent}"))
        .map(|document| {
            merge(
                document,
                hub.setting(&format!("{AGENT_REVIEW_EFFORT_PREFIX}{agent}")),
            )
        })
}

pub(in crate::server) fn session_document(hub: &crate::core::Hub, session: &str) -> Option<Value> {
    let effort = hub.setting(&format!("{SESSION_REVIEW_EFFORT_PREFIX}{session}"));
    // An effort-only override has no Tools document of its own.
    match hub.setting(&format!("{SESSION_TOOLS_PREFIX}{session}")) {
        Some(document) => Some(merge(document, effort)),
        None => {
            effort.map(|effort| merge(serde_json::json!({"schema":1,"calls":{}}), Some(effort)))
        }
    }
}

pub(in crate::server) fn store_agent(hub: &crate::core::Hub, agent: &str, document: Option<Value>) {
    store(
        hub,
        format!("{AGENT_TOOLS_PREFIX}{agent}"),
        format!("{AGENT_REVIEW_EFFORT_PREFIX}{agent}"),
        document,
    );
}

pub(in crate::server) fn store_session(
    hub: &crate::core::Hub,
    session: &str,
    document: Option<Value>,
) {
    let (document, effort) = document.map_or((Value::Null, Value::Null), split);
    // Keep the schema-one document absent when only effort was overridden.
    let document = if serde_json::from_value::<super::SessionTools>(document.clone())
        .is_ok_and(|tools| tools.is_empty())
    {
        Value::Null
    } else {
        document
    };
    write_pair(
        hub,
        format!("{SESSION_TOOLS_PREFIX}{session}"),
        document,
        format!("{SESSION_REVIEW_EFFORT_PREFIX}{session}"),
        effort,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn effort_never_reaches_the_schema_one_document_and_round_trips() {
        let document = json!({"schema":1,"calls":{"enabled":false,"review":{
            "claude":["opus"],"gpt":["astra","sol"],"claude_effort":"max","gpt_effort":"standard"}}});
        let (stored, effort) = split(document.clone());
        assert_eq!(
            stored["calls"]["review"],
            json!({"claude":["opus"],"gpt":["astra","sol"]})
        );
        assert_eq!(stored["calls"]["enabled"], false);
        assert_eq!(merge(stored, Some(effort)), document);
    }

    #[test]
    fn an_effort_only_session_override_leaves_no_empty_review_object() {
        let (stored, effort) =
            split(json!({"schema":1,"calls":{"review":{"gpt_effort":"standard"}}}));
        assert_eq!(stored, json!({"schema":1,"calls":{}}));
        assert_eq!(effort, json!({"gpt_effort":"standard"}));
        assert_eq!(
            merge(json!({"schema":1,"calls":{}}), Some(effort)),
            json!({"schema":1,"calls":{"review":{"gpt_effort":"standard"}}})
        );
        assert_eq!(split(json!({"schema":1,"calls":{}})).1, Value::Null);
    }
}
