//! Widget decisions (docs/widgets.md §8, as revised: a decision is an ordinary
//! user prompt). The Controller composes the message from what the person
//! saw — the widget handle, title and revision, the button label or the
//! question and the chosen option labels — so the page cannot put words in the
//! person's mouth. Inputs that share a `group` (or a lone input) take one
//! decision; they stay inert until a later revision changes the decided
//! input's labels or options, which asks again.
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

/// Options a multiple choice may submit when it sets no `max_selected`.
const MAX_SELECTED: u64 = 12;

/// A decision ready to be sent and recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub node_id: String,
    pub group_key: String,
    pub signature: String,
    pub labels: Vec<String>,
    pub message: String,
}

/// Why a tap cannot become a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
}

fn refuse(code: &'static str, message: impl Into<String>) -> Refusal {
    Refusal {
        code,
        message: message.into(),
    }
}

/// The interactive node `id` of a document.
fn find<'v>(value: &'v Value, id: &str) -> Option<&'v Map<String, Value>> {
    match value {
        Value::Object(map) => {
            if map.get("id").and_then(Value::as_str) == Some(id)
                && matches!(
                    map.get("type").and_then(Value::as_str),
                    Some("button" | "choice")
                )
            {
                return Some(map);
            }
            map.values().find_map(|child| find(child, id))
        }
        Value::Array(items) => items.iter().find_map(|item| find(item, id)),
        _ => None,
    }
}

fn each_input<'v>(value: &'v Value, out: &mut Vec<&'v Map<String, Value>>) {
    match value {
        Value::Object(map) => {
            if matches!(
                map.get("type").and_then(Value::as_str),
                Some("button" | "choice")
            ) && map.get("id").and_then(Value::as_str).is_some()
            {
                out.push(map);
            }
            for child in map.values() {
                each_input(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                each_input(item, out);
            }
        }
        _ => {}
    }
}

fn text<'v>(node: &'v Map<String, Value>, key: &str) -> &'v str {
    node.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn group_key(node: &Map<String, Value>) -> String {
    node.get("group").and_then(Value::as_str).map_or_else(
        || format!("node:{}", text(node, "id")),
        |group| format!("group:{group}"),
    )
}

/// Digest of what the person reads and chooses from on this input.
fn signature(node: &Map<String, Value>) -> String {
    let options: Vec<Value> = node
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|option| json!([option.get("value"), option.get("label")]))
        .collect();
    let seen = json!([
        node.get("type"),
        node.get("label"),
        node.get("action").and_then(|action| action.get("kind")),
        options,
        node.get("multiple"),
        node.get("max_selected"),
    ]);
    let digest = Sha256::digest(seen.to_string().as_bytes());
    format!("{digest:x}")[..32].to_owned()
}

fn max_selected(node: &Map<String, Value>) -> u64 {
    if node.get("multiple").and_then(Value::as_bool) == Some(true) {
        node.get("max_selected")
            .and_then(Value::as_u64)
            .unwrap_or(MAX_SELECTED)
    } else {
        1
    }
}

/// The decision a tap on `node_id` makes with the option `values` selected.
pub fn decide(
    document: &Value,
    handle: &str,
    revision: i64,
    node_id: &str,
    values: &[String],
) -> Result<Decision, Refusal> {
    let node = find(&document["root"], node_id)
        .ok_or_else(|| refuse("unknown_input", "this widget has no such input"))?;
    let labels = match text(node, "type") {
        "button" => {
            if node
                .get("action")
                .and_then(|action| action.get("kind"))
                .and_then(Value::as_str)
                != Some("prompt")
            {
                return Err(refuse(
                    "not_a_decision",
                    "this button does not send a message",
                ));
            }
            if node.get("disabled").and_then(Value::as_bool) == Some(true) {
                return Err(refuse("disabled", "this button is disabled"));
            }
            if !values.is_empty() {
                return Err(refuse("invalid_selection", "a button takes no options"));
            }
            vec![text(node, "label").to_owned()]
        }
        _ => {
            let options = node
                .get("options")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut seen = std::collections::HashSet::new();
            if values.is_empty()
                || values.len() as u64 > max_selected(node)
                || !values.iter().all(|value| seen.insert(value.as_str()))
            {
                return Err(refuse(
                    "invalid_selection",
                    format!("choose 1 to {} different options", max_selected(node)),
                ));
            }
            // Option order, not tap order: the same selection reads the same.
            let labels: Vec<String> = options
                .iter()
                .filter(|option| {
                    option
                        .get("value")
                        .and_then(Value::as_str)
                        .is_some_and(|value| seen.contains(value))
                })
                .filter_map(|option| option.get("label").and_then(Value::as_str))
                .map(str::to_owned)
                .collect();
            if labels.len() != values.len() {
                return Err(refuse("invalid_selection", "an option is not offered"));
            }
            labels
        }
    };
    let title = document
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let message = if text(node, "type") == "button" {
        format!(
            "Widget #{handle} “{title}” (revision {revision}): {}",
            labels[0]
        )
    } else {
        format!(
            "Widget #{handle} “{title}” (revision {revision}), {}: {}",
            text(node, "label"),
            labels.join("; ")
        )
    };
    Ok(Decision {
        node_id: node_id.to_owned(),
        group_key: group_key(node),
        signature: signature(node),
        labels,
        message,
    })
}

/// A recorded decision, as far as deciding inertness needs it.
pub struct Decided<'a> {
    pub node_id: &'a str,
    pub group_key: &'a str,
    pub signature: &'a str,
    pub labels: &'a [String],
    pub created_at_ms: i64,
}

/// The inputs of `document` that are already decided, with the decision that
/// settled their group: `{node_id: {labels, chosen, atMs}}`. A decision
/// counts while its input still exists unchanged; `decided` is newest first.
#[must_use]
pub fn decided_inputs(document: &Value, decided: &[Decided<'_>]) -> Map<String, Value> {
    let mut inputs = Vec::new();
    each_input(&document["root"], &mut inputs);
    let live = |record: &&Decided<'_>| {
        inputs.iter().any(|node| {
            text(node, "id") == record.node_id
                && group_key(node) == record.group_key
                && signature(node) == record.signature
        })
    };
    let mut out = Map::new();
    for node in &inputs {
        let key = group_key(node);
        if let Some(record) = decided
            .iter()
            .filter(live)
            .find(|record| record.group_key == key)
        {
            out.insert(
                text(node, "id").to_owned(),
                json!({
                    "labels": record.labels,
                    "chosen": record.node_id == text(node, "id"),
                    "atMs": record.created_at_ms,
                }),
            );
        }
    }
    out
}

/// The prompt id of a decision: the same tap retried is the same prompt.
#[must_use]
pub fn decision_cmid(
    session_id: &str,
    widget_id: &str,
    revision: i64,
    decision: &Decision,
) -> String {
    let seen = json!([
        session_id,
        widget_id,
        revision,
        decision.node_id,
        decision.labels
    ]);
    let digest = Sha256::digest(seen.to_string().as_bytes());
    format!("widget-{}", &format!("{digest:x}")[..32])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> Value {
        json!({"schema":"cowboy.widget/1","title":"Cleanup","root":{"type":"stack","children":[
            {"type":"choice","id":"cache","label":"Which cache may be removed?","multiple":true,
             "options":[{"value":"cargo","label":"Cargo target"},{"value":"logs","label":"Old logs"}]},
            {"type":"button","id":"yes","label":"Approve","action":{"kind":"prompt"},"group":"go"},
            {"type":"button","id":"no","label":"Reject","action":{"kind":"prompt"},"group":"go"},
            {"type":"button","id":"copy","label":"Copy","action":{"kind":"copy","text":"x"}}]}})
    }

    #[test]
    fn decisions_compose_from_visible_labels_in_option_order() {
        let doc = document();
        let choice = decide(&doc, "w4", 2, "cache", &["logs".into(), "cargo".into()]).unwrap();
        assert_eq!(
            choice.message,
            "Widget #w4 “Cleanup” (revision 2), Which cache may be removed?: Cargo target; Old logs"
        );
        assert_eq!(choice.group_key, "node:cache");
        let yes = decide(&doc, "w4", 2, "yes", &[]).unwrap();
        assert_eq!(yes.message, "Widget #w4 “Cleanup” (revision 2): Approve");
        assert_eq!(yes.group_key, "group:go");
        for (node, values, code) in [
            ("copy", vec![], "not_a_decision"),
            ("missing", vec![], "unknown_input"),
            ("cache", vec![], "invalid_selection"),
            ("cache", vec!["rm -rf".to_owned()], "invalid_selection"),
            (
                "cache",
                vec!["logs".to_owned(), "logs".to_owned()],
                "invalid_selection",
            ),
            ("yes", vec!["cargo".to_owned()], "invalid_selection"),
        ] {
            assert_eq!(
                decide(&doc, "w4", 2, node, &values).unwrap_err().code,
                code,
                "{node}"
            );
        }
        assert_eq!(
            decision_cmid("s", "w", 2, &choice),
            decision_cmid(
                "s",
                "w",
                2,
                &decide(&doc, "w4", 2, "cache", &["cargo".into(), "logs".into()]).unwrap()
            ),
            "the same selection is the same prompt"
        );
    }

    #[test]
    fn a_group_stays_decided_until_the_decided_input_changes() {
        let mut doc = document();
        let yes = decide(&doc, "w4", 2, "yes", &[]).unwrap();
        let record = Decided {
            node_id: &yes.node_id,
            group_key: &yes.group_key,
            signature: &yes.signature,
            labels: &yes.labels,
            created_at_ms: 9,
        };
        let inert = decided_inputs(&doc, std::slice::from_ref(&record));
        assert_eq!(inert["yes"]["chosen"], true);
        assert_eq!(inert["no"]["chosen"], false);
        assert!(!inert.contains_key("cache"));
        // Moving the decided button out of its group asks again (F5).
        doc["root"]["children"][1]["group"] = json!("other");
        assert!(decided_inputs(&doc, std::slice::from_ref(&record)).is_empty());
        doc["root"]["children"][1]["group"] = json!("go");
        doc["root"]["children"][1]["label"] = json!("Approve again");
        assert!(decided_inputs(&doc, &[record]).is_empty());
    }
}
