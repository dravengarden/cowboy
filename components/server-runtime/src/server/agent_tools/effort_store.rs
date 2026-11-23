//! Review effort and the calls-per-turn limit live beside, not inside, the
//! schema-one Tools documents.
//!
//! Older Controllers parse those documents with `deny_unknown_fields` and
//! fall back to the built-in Tools policy on any failure, which would re-enable
//! calls and memory a person had turned off. Keeping effort under its own keys
//! means a rollback only forgets the effort choice (back to Max first).
//!
//! The per-turn limit replaced the lifetime `max_per_session`, which a
//! predecessor still requires in the document. A stored document therefore
//! keeps a `max_per_session` (the one it already had, else the retired
//! built-in 64) and the limit itself sits under `agent_turn_budget:<agent>`.
//! A rollback keeps every other setting and goes back to its own lifetime cap;
//! a document no newer Controller has saved reads as a limit migrated from
//! that cap (`legacy_turn_limit`).

use crate::core::settings_keys::{
    AGENT_REVIEW_EFFORT_PREFIX, AGENT_TOOLS_PREFIX, AGENT_TURN_BUDGET_PREFIX,
    SESSION_REVIEW_EFFORT_PREFIX, SESSION_TOOLS_PREFIX,
};
use serde_json::{Map, Value, json};

/// The lifetime cap a predecessor requires, when the document held none: the
/// built-in value of the Controller that read it.
const RETIRED_SESSION_CAP: u64 = 64;

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

/// Remove the calls-per-turn limit from a Tools document; returns the document
/// and the limit object (null when there is none). The document keeps what a
/// predecessor requires: its lifetime `max_per_session`, `previous`'s when it
/// had one. The limit is bound to the digest of the document it was saved
/// with, so a predecessor that replaces the document (a reset, an edit)
/// orphans it instead of resurrecting a stale limit.
fn split_budget(mut document: Value, previous: Option<&Value>) -> (Value, Value) {
    let Some(calls) = document.get_mut("calls").and_then(Value::as_object_mut) else {
        return (document, Value::Null);
    };
    let Some(limit) = calls
        .remove("max_per_turn")
        .filter(|value| !value.is_null())
    else {
        return (document, Value::Null);
    };
    let legacy = previous
        .and_then(|stored| stored.pointer("/calls/max_per_session"))
        .filter(|cap| cap.as_u64().is_some_and(|cap| (1..=1000).contains(&cap)))
        .cloned()
        .unwrap_or_else(|| json!(RETIRED_SESSION_CAP));
    calls.insert("max_per_session".to_owned(), legacy);
    let bound = crate::managed_calls::canonical_digest(&document);
    (
        document,
        json!({ "max_per_turn": limit, "document": bound }),
    )
}

/// Put a stored limit back into a Tools document read from its own key, when
/// the limit was saved with exactly that document (`stored`'s digest). The
/// document's legacy `max_per_session` stays; the typed reader prefers the
/// limit and migrates the cap only when there is none.
fn merge_budget(mut document: Value, budget: Option<Value>, stored: &str) -> Value {
    let limit = budget
        .as_ref()
        .filter(|budget| budget.get("document").and_then(Value::as_str) == Some(stored))
        .and_then(|budget| budget.get("max_per_turn"))
        .filter(|limit| limit.is_u64());
    if let (Some(limit), Some(calls)) = (
        limit,
        document.get_mut("calls").and_then(Value::as_object_mut),
    ) {
        calls.insert("max_per_turn".to_owned(), limit.clone());
    }
    document
}

/// Saves and reads of a Tools document and what sits beside it take turns, so
/// a reader never pairs one save's document with another's limit and the
/// store queue receives writes in the order the Hub committed them.
static SAVES: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// Commit a document and the settings kept beside it under one Hub lock so
/// concurrent saves cannot pair one caller's policy with another's effort or
/// limit; the store queue keeps order.
fn write_group(hub: &crate::core::Hub, key: String, document: Value, beside: Vec<(String, Value)>) {
    let snapshot = hub.with_settings_mut(|settings| {
        for (beside_key, value) in &beside {
            settings.insert(beside_key.clone(), value.clone());
        }
        crate::core::Hub::commit_setting_locked(settings, key.clone(), document.clone())
    });
    for (beside_key, value) in beside {
        hub.publish_setting(beside_key, value, snapshot.clone());
    }
    hub.publish_setting(key, document, snapshot);
}

pub fn agent_document(hub: &crate::core::Hub, agent: &str) -> Option<Value> {
    let _turn = SAVES.lock();
    let stored = hub.setting(&format!("{AGENT_TOOLS_PREFIX}{agent}"))?;
    let digest = crate::managed_calls::canonical_digest(&stored);
    let document = merge(
        stored,
        hub.setting(&format!("{AGENT_REVIEW_EFFORT_PREFIX}{agent}")),
    );
    Some(merge_budget(
        document,
        hub.setting(&format!("{AGENT_TURN_BUDGET_PREFIX}{agent}")),
        &digest,
    ))
}

pub fn session_document(hub: &crate::core::Hub, session: &str) -> Option<Value> {
    let _turn = SAVES.lock();
    let effort = hub.setting(&format!("{SESSION_REVIEW_EFFORT_PREFIX}{session}"));
    // An effort-only override has no Tools document of its own.
    match hub.setting(&format!("{SESSION_TOOLS_PREFIX}{session}")) {
        Some(document) => Some(merge(document, effort)),
        None => {
            effort.map(|effort| merge(serde_json::json!({"schema":1,"calls":{}}), Some(effort)))
        }
    }
}

pub fn store_agent(hub: &crate::core::Hub, agent: &str, document: Option<Value>) {
    let _turn = SAVES.lock();
    let key = format!("{AGENT_TOOLS_PREFIX}{agent}");
    let previous = hub.setting(&key);
    let (document, effort) = document.map_or((Value::Null, Value::Null), split);
    let (document, budget) = split_budget(document, previous.as_ref());
    write_group(
        hub,
        key,
        document,
        vec![
            (format!("{AGENT_REVIEW_EFFORT_PREFIX}{agent}"), effort),
            (format!("{AGENT_TURN_BUDGET_PREFIX}{agent}"), budget),
        ],
    );
}

pub fn store_session(hub: &crate::core::Hub, session: &str, document: Option<Value>) {
    let _turn = SAVES.lock();
    let (document, effort) = document.map_or((Value::Null, Value::Null), split);
    // Keep the schema-one document absent when only effort was overridden.
    let document = if serde_json::from_value::<super::SessionTools>(document.clone())
        .is_ok_and(|tools| tools.is_empty())
    {
        Value::Null
    } else {
        document
    };
    write_group(
        hub,
        format!("{SESSION_TOOLS_PREFIX}{session}"),
        document,
        vec![(format!("{SESSION_REVIEW_EFFORT_PREFIX}{session}"), effort)],
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

    /// The document shape a predecessor Controller parses, field for field.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)] // Only parsing matters.
    struct PredecessorTools {
        schema: u16,
        matrix: Value,
        calls: PredecessorCalls,
    }

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct PredecessorCalls {
        #[serde(default)]
        review: Value,
        enabled: bool,
        targets: Value,
        default: String,
        max_concurrent: u16,
        max_per_session: u16,
        #[serde(default)]
        images: bool,
        #[serde(default)]
        self_images: bool,
    }

    fn saved(hub: &crate::core::Hub, limit: u16) {
        let mut tools = super::super::AgentTools::builtin("codex");
        tools.calls.max_per_turn = limit;
        store_agent(hub, "codex", Some(json!(tools)));
    }

    #[test]
    fn the_turn_limit_sits_beside_a_document_a_predecessor_still_reads() {
        let hub = crate::core::Hub::new();
        saved(&hub, 8);
        let stored = hub.setting("agent_tools:codex").unwrap();
        // A rollback parses the stored document and keeps every setting.
        let predecessor: PredecessorTools = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(predecessor.calls.max_per_session, 64);
        assert!(stored["calls"].get("max_per_turn").is_none());
        let beside = hub.setting("agent_turn_budget:codex").unwrap();
        assert_eq!(beside["max_per_turn"], 8);
        // The limit is bound to the exact document it was saved with.
        assert_eq!(
            beside["document"],
            crate::managed_calls::canonical_digest(&stored)
        );
        let (tools, customized) = super::super::hub_agent_defaults(&hub, "codex");
        assert!(customized);
        assert_eq!(tools.calls.max_per_turn, 8);
        // A cap the stored document already had stays what a rollback sees.
        let mut earlier = stored;
        earlier["calls"]["max_per_session"] = json!(128);
        hub.with_settings_mut(|settings| {
            settings.insert("agent_tools:codex".into(), earlier);
        });
        saved(&hub, 32);
        let stored = hub.setting("agent_tools:codex").unwrap();
        let predecessor: PredecessorTools = serde_json::from_value(stored).unwrap();
        assert_eq!(predecessor.calls.max_per_session, 128);
        assert_eq!(
            super::super::hub_agent_defaults(&hub, "codex")
                .0
                .calls
                .max_per_turn,
            32
        );
    }

    #[test]
    fn a_predecessor_reset_does_not_resurrect_an_old_limit() {
        let hub = crate::core::Hub::new();
        saved(&hub, 64);
        // Rolled back: the predecessor resets to built-in defaults (it cannot
        // clear the limit kept beside the document), then saves a lifetime cap.
        let mut legacy = hub.setting("agent_tools:codex").unwrap();
        legacy["calls"]["max_per_session"] = json!(8);
        hub.with_settings_mut(|settings| {
            settings.insert("agent_tools:codex".into(), legacy);
        });
        let (tools, customized) = super::super::hub_agent_defaults(&hub, "codex");
        assert!(customized);
        assert_eq!(
            tools.calls.max_per_turn, 8,
            "the cap migrates, not the orphan"
        );
    }

    #[test]
    fn a_predecessor_edit_drops_the_limit_saved_with_the_document_it_replaced() {
        let hub = crate::core::Hub::new();
        saved(&hub, 4);
        let mut edited = hub.setting("agent_tools:codex").unwrap();
        edited["calls"]["enabled"] = json!(false);
        hub.with_settings_mut(|settings| {
            settings.insert("agent_tools:codex".into(), edited);
        });
        let (tools, _) = super::super::hub_agent_defaults(&hub, "codex");
        assert!(!tools.calls.enabled, "the predecessor's edit is kept");
        assert_eq!(tools.calls.max_per_turn, 16, "from its lifetime cap of 64");
    }

    #[test]
    fn saves_and_reads_never_pair_one_save_with_anothers_limit() {
        let hub = std::sync::Arc::new(crate::core::Hub::new());
        let save = |hub: &crate::core::Hub, enabled: bool, limit: u16| {
            let mut tools = super::super::AgentTools::builtin("codex");
            tools.calls.enabled = enabled;
            tools.calls.max_per_turn = limit;
            store_agent(hub, "codex", Some(json!(tools)));
        };
        save(&hub, false, 4);
        let write = |enabled: bool, limit: u16| {
            let hub = std::sync::Arc::clone(&hub);
            std::thread::spawn(move || {
                for _ in 0..300 {
                    save(&hub, enabled, limit);
                }
            })
        };
        let writers = [write(false, 4), write(true, 64)];
        for _ in 0..600 {
            let (tools, _) = super::super::hub_agent_defaults(&hub, "codex");
            // A document paired with another save's limit would fail the
            // digest and read as the lifetime cap's 16.
            assert!(
                matches!(
                    (tools.calls.enabled, tools.calls.max_per_turn),
                    (false, 4) | (true, 64)
                ),
                "{:?}",
                (tools.calls.enabled, tools.calls.max_per_turn)
            );
        }
        for writer in writers {
            writer.join().unwrap();
        }
    }

    #[test]
    fn restoring_the_built_in_defaults_clears_the_limit_too() {
        let hub = crate::core::Hub::new();
        saved(&hub, 4);
        store_agent(&hub, "codex", None);
        assert!(hub.setting("agent_tools:codex").is_none());
        assert!(hub.setting("agent_turn_budget:codex").is_none());
        let (tools, customized) = super::super::hub_agent_defaults(&hub, "codex");
        assert!(!customized);
        assert_eq!(tools.calls.max_per_turn, 16);
        // Without a stored limit, the document's own cap is what migrates.
        saved(&hub, 4);
        hub.with_settings_mut(|settings| {
            settings.insert("agent_turn_budget:codex".into(), Value::Null);
        });
        let (tools, customized) = super::super::hub_agent_defaults(&hub, "codex");
        assert!(customized);
        assert_eq!(tools.calls.max_per_turn, 16);
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
