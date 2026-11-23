//! ACP subagents for bridge clients.
//!
//! The worker declares `clientCapabilities.subagents`, so adapters no longer
//! show a subagent as the parent's Agent/Task tool call: Cowboy folds each
//! child session into one `cowboy_subagent` snapshot of the parent (see
//! `acp_subagents.rs`). The bridge turns those snapshots back into ACP for its
//! own client:
//!
//! - a client that declared `clientCapabilities.subagents` gets the standard
//!   form: `subagent_update` on the parent, the delegation prompt and the
//!   result as linked `session_message`s, and the child's tool calls and
//!   messages as updates of the child session;
//! - any other client gets one ordinary `tool_call` per subagent, updated in
//!   place, so it sees what it saw before Cowboy took the capability.
//!
//! Snapshots are complete, so a projection only needs what it already sent.
//! It is kept per attached session and restarted when the client reloads the
//! thread.

use std::collections::HashMap;

use agent_client_protocol::schema::v1::{SessionNotification, SessionUpdate};
use serde_json::{Value, json};

pub(super) use crate::runtime_wire::SUBAGENT_UPDATE;

/// Activity entries whose kinds ACP's `ToolKind` names.
const TOOL_KINDS: &[&str] = &[
    "read",
    "edit",
    "delete",
    "move",
    "search",
    "execute",
    "think",
    "fetch",
    "switch_mode",
    "other",
];

/// The tool call id a legacy client sees for subagent `child`.
pub(super) fn legacy_tool_call_id(child: &str) -> String {
    format!("cowboy-subagent:{child}")
}

#[derive(Debug, Default)]
struct Projected {
    revision: u64,
    /// Legacy form: the tool call was started; its last presented fields.
    legacy: Option<Value>,
    /// Standard form: the last announced `subagent_update` fields.
    announced: Option<Value>,
    prompt: Option<String>,
    result: Option<String>,
    /// Activity entries already delivered, as an absolute count.
    activity: u64,
    /// Tool status per delivered tool call id.
    tools: HashMap<String, String>,
    /// The last delivered message entry and the text sent for it.
    message: Option<(u64, String)>,
}

/// What one bridge client was told about the subagents of one session.
#[derive(Debug, Default)]
pub(super) struct SubagentProjection {
    children: HashMap<String, Projected>,
}

fn text<'a>(snapshot: &'a Value, key: &str) -> Option<&'a str> {
    snapshot
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn title(snapshot: &Value) -> &str {
    text(snapshot, "title")
        .or_else(|| text(snapshot, "description"))
        .unwrap_or("Subagent")
}

fn reporting(snapshot: &Value) -> bool {
    snapshot.get("reporting").and_then(Value::as_bool) != Some(false)
}

fn working(snapshot: &Value) -> bool {
    matches!(
        snapshot.get("state").and_then(Value::as_str),
        Some("running" | "requires_action")
    )
}

/// The outcome a person reads, as the Web row labels it.
fn status_label(snapshot: &Value) -> &'static str {
    match snapshot.get("state").and_then(Value::as_str) {
        Some("running") if !reporting(snapshot) => "No longer reporting",
        Some("running") => "Working",
        Some("requires_action") => "Needs your answer",
        Some("idle") => match snapshot.get("stopReason").and_then(Value::as_str) {
            Some("cancelled") => "Stopped",
            Some("error") => "Failed",
            Some("refusal") => "Declined",
            Some("max_tokens" | "max_turn_requests") => "Hit its limit",
            _ if text(snapshot, "error").is_some() => "Failed",
            _ => "Done",
        },
        _ => "Ended",
    }
}

/// A legacy tool status: work in progress, done, or ended without its result.
fn legacy_status(snapshot: &Value) -> &'static str {
    match status_label(snapshot) {
        "Working" | "Needs your answer" | "No longer reporting" => "in_progress",
        "Stopped" | "Failed" | "Declined" | "Hit its limit" => "failed",
        _ => "completed",
    }
}

fn count(snapshot: &Value, key: &str) -> u64 {
    snapshot.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn plural(value: u64, one: &str, many: &str) -> String {
    format!("{value} {}", if value == 1 { one } else { many })
}

/// The legacy tool call's text: what it was asked, how far it is, and its
/// outcome.
fn legacy_body(snapshot: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(prompt) = text(snapshot, "prompt") {
        parts.push(format!("Asked: {prompt}"));
    }
    let mut progress = vec![status_label(snapshot).to_owned()];
    if let Some(model) = text(snapshot, "model") {
        progress.push(model.to_owned());
    }
    let tools = count(snapshot, "toolCalls");
    if tools > 0 {
        progress.push(plural(tools, "tool call", "tool calls"));
    }
    let messages = count(snapshot, "messages");
    if messages > 0 {
        progress.push(plural(messages, "message", "messages"));
    }
    parts.push(progress.join(" · "));
    if working(snapshot)
        && let Some(last) = snapshot
            .get("activity")
            .and_then(Value::as_array)
            .and_then(|activity| activity.last())
        && let Some(line) = last
            .get("title")
            .or_else(|| last.get("text"))
            .and_then(Value::as_str)
            .and_then(|text| text.lines().find(|line| !line.trim().is_empty()))
    {
        parts.push(format!("Now: {}", line.trim()));
    }
    if !working(snapshot)
        && let Some(result) = text(snapshot, "result")
    {
        parts.push(format!("Result: {result}"));
    }
    if let Some(error) = text(snapshot, "error") {
        parts.push(format!("Error: {error}"));
    }
    parts.join("\n\n")
}

/// The standard `state` of a snapshot. A child that stopped reporting is
/// `unknown`: ACP's word for work that can no longer be observed.
fn standard_state(snapshot: &Value) -> Value {
    match snapshot.get("state").and_then(Value::as_str) {
        Some("running") if reporting(snapshot) => json!({ "state": "running" }),
        Some("requires_action") => json!({ "state": "requires_action" }),
        Some("idle") => {
            let mut state = json!({ "state": "idle" });
            if let Some(reason) = text(snapshot, "stopReason") {
                state["stopReason"] = json!(reason);
            }
            if let Some(error) = text(snapshot, "error") {
                state["stopReason"] = json!("error");
                state["error"] = json!({ "code": -32603, "message": error });
            }
            state
        }
        _ => json!({ "state": "unknown" }),
    }
}

fn notification(session_id: &str, update: Value) -> Option<SessionNotification> {
    match serde_json::from_value::<SessionUpdate>(update) {
        Ok(update) => Some(SessionNotification::new(session_id.to_owned(), update)),
        Err(error) => {
            tracing::debug!(%error, "subagent update has no ACP form");
            None
        }
    }
}

impl SubagentProjection {
    /// The ACP notifications that bring the client up to `snapshot`, a
    /// `cowboy_subagent` update of session `root`. An older revision than the
    /// one already projected yields nothing.
    pub(super) fn project(
        &mut self,
        root: &str,
        snapshot: &Value,
        standard: bool,
    ) -> Vec<SessionNotification> {
        let Some(child) = text(snapshot, "sessionId") else {
            return Vec::new();
        };
        let revision = count(snapshot, "revision");
        let projected = self.children.entry(child.to_owned()).or_default();
        if revision < projected.revision {
            return Vec::new();
        }
        projected.revision = revision;
        if standard {
            Self::standard(projected, root, child, snapshot)
        } else {
            Self::legacy(projected, root, child, snapshot)
        }
        .into_iter()
        .flatten()
        .collect()
    }

    /// Whether `session_id` is a subagent this client was told about.
    pub(super) fn knows(&self, session_id: &str) -> bool {
        self.children.contains_key(session_id)
    }

    fn legacy(
        projected: &mut Projected,
        root: &str,
        child: &str,
        snapshot: &Value,
    ) -> Vec<Option<SessionNotification>> {
        let fields = json!({
            "title": format!("Subagent: {}", title(snapshot)),
            "status": legacy_status(snapshot),
            "content": [{ "type": "content", "content": { "type": "text", "text": legacy_body(snapshot) } }],
        });
        if projected.legacy.as_ref() == Some(&fields) {
            return Vec::new();
        }
        let mut update = fields.clone();
        update["toolCallId"] = json!(legacy_tool_call_id(child));
        if projected.legacy.is_none() {
            update["sessionUpdate"] = json!("tool_call");
            update["kind"] = json!("think");
            update["rawInput"] = json!({
                "subagent": child,
                "description": text(snapshot, "description"),
                "prompt": text(snapshot, "prompt"),
            });
        } else {
            update["sessionUpdate"] = json!("tool_call_update");
        }
        projected.legacy = Some(fields);
        // A nested subagent is drawn in its root session; a legacy client has
        // no child thread to put it in.
        vec![notification(root, update)]
    }

    fn standard(
        projected: &mut Projected,
        root: &str,
        child: &str,
        snapshot: &Value,
    ) -> Vec<Option<SessionNotification>> {
        let parent = text(snapshot, "parentSessionId").unwrap_or(root);
        let mut out = Vec::new();
        let announced = json!({
            "title": text(snapshot, "title"),
            "description": text(snapshot, "description"),
            "capabilities": if snapshot.get("cancellable").and_then(Value::as_bool) == Some(true) {
                json!({ "cancel": {} })
            } else {
                json!({})
            },
            "state": standard_state(snapshot),
        });
        if projected.announced.as_ref() != Some(&announced) {
            let mut update = announced.clone();
            // An absent label is left unchanged; `null` would clear it.
            if let Some(fields) = update.as_object_mut() {
                fields.retain(|_, value| !value.is_null());
            }
            update["sessionUpdate"] = json!("subagent_update");
            update["sessionId"] = json!(child);
            out.push(notification(parent, update));
            projected.announced = Some(announced);
        }
        if let Some(prompt) = text(snapshot, "prompt")
            && projected.prompt.as_deref() != Some(prompt)
        {
            out.push(notification(
                parent,
                json!({
                    "sessionUpdate": "session_message",
                    "messageId": format!("cowboy-subagent:{child}:prompt"),
                    "senderSessionId": parent,
                    "recipientSessionId": child,
                    "content": [{ "type": "text", "text": prompt }],
                }),
            ));
            projected.prompt = Some(prompt.to_owned());
        }
        out.extend(Self::child_activity(projected, child, snapshot));
        if !working(snapshot)
            && let Some(result) = text(snapshot, "result")
            && projected.result.as_deref() != Some(result)
        {
            out.push(notification(
                parent,
                json!({
                    "sessionUpdate": "session_message",
                    "messageId": format!("cowboy-subagent:{child}:result"),
                    "senderSessionId": child,
                    "recipientSessionId": parent,
                    "content": [{ "type": "text", "text": result }],
                }),
            ));
            projected.result = Some(result.to_owned());
        }
        out
    }

    /// The child's own tool calls and messages that the client has not seen.
    /// An entry's absolute position is the number of entries before it,
    /// including those the snapshot no longer carries.
    fn child_activity(
        projected: &mut Projected,
        child: &str,
        snapshot: &Value,
    ) -> Vec<Option<SessionNotification>> {
        let Some(activity) = snapshot.get("activity").and_then(Value::as_array) else {
            return Vec::new();
        };
        let first = count(snapshot, "omittedActivity");
        let mut out = Vec::new();
        for (offset, entry) in (0_u64..).zip(activity) {
            let position = first + offset;
            match entry.get("kind").and_then(Value::as_str) {
                Some("tool") => {
                    let Some(id) = entry.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let status = entry
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("pending");
                    match projected.tools.get(id) {
                        Some(sent) if sent == status => {}
                        Some(_) => out.push(notification(
                            child,
                            json!({ "sessionUpdate": "tool_call_update", "toolCallId": id, "status": status }),
                        )),
                        None if position >= projected.activity => {
                            let mut update = json!({
                                "sessionUpdate": "tool_call",
                                "toolCallId": id,
                                "title": entry.get("title").and_then(Value::as_str).unwrap_or(id),
                                "status": status,
                            });
                            if let Some(kind) = entry
                                .get("toolKind")
                                .and_then(Value::as_str)
                                .filter(|kind| TOOL_KINDS.contains(kind))
                            {
                                update["kind"] = json!(kind);
                            }
                            out.push(notification(child, update));
                        }
                        None => continue,
                    }
                    projected.tools.insert(id.to_owned(), status.to_owned());
                }
                Some("message") => {
                    let body = entry.get("text").and_then(Value::as_str).unwrap_or("");
                    let delta = match &projected.message {
                        Some((sent_at, sent)) if *sent_at == position => body
                            .strip_prefix(sent.as_str())
                            .filter(|delta| !delta.is_empty()),
                        _ if position >= projected.activity => {
                            Some(body).filter(|body| !body.is_empty())
                        }
                        _ => None,
                    };
                    if let Some(delta) = delta {
                        out.push(notification(
                            child,
                            json!({
                                "sessionUpdate": "agent_message_chunk",
                                "content": { "type": "text", "text": delta },
                            }),
                        ));
                        projected.message = Some((position, body.to_owned()));
                    }
                }
                _ => {}
            }
            projected.activity = projected.activity.max(position + 1);
        }
        out
    }
}

/// A child's permission request for the client: the standard form asks in
/// the child session; a legacy client is asked in the parent with the
/// subagent named in the title. `None`: the request is the parent's own.
pub(super) fn subagent_request(tool_call: &mut Value, standard: bool) -> Option<String> {
    let subagent = tool_call.pointer("/_cowboy/subagent")?;
    let child = subagent
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())?
        .to_owned();
    if standard {
        return Some(child);
    }
    let name = subagent
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("Subagent")
        .to_owned();
    let asked = tool_call
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned);
    if let Some(object) = tool_call.as_object_mut() {
        object.insert(
            "title".to_owned(),
            json!(match asked {
                Some(asked) => format!("Subagent {name}: {asked}"),
                None => format!("Subagent {name} asks"),
            }),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(revision: u64, extra: &Value) -> Value {
        let mut value = json!({
            "sessionUpdate": SUBAGENT_UPDATE,
            "sessionId": "child",
            "revision": revision,
            "parentSessionId": null,
            "title": "Explore",
            "description": "Explore",
            "state": "running",
            "cancellable": true,
            "prompt": "Find the bug",
            "toolCalls": 1,
            "messages": 0,
            "activity": [{ "kind": "tool", "id": "t1", "title": "Read a.rs", "toolKind": "read", "status": "in_progress" }],
            "omittedActivity": 0,
            "reporting": true,
        });
        if let (Some(target), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
            target.extend(extra.clone());
        }
        value
    }

    fn wire(notifications: &[SessionNotification]) -> Vec<Value> {
        notifications
            .iter()
            .map(|notification| serde_json::to_value(notification).unwrap())
            .collect()
    }

    #[test]
    fn a_legacy_client_sees_one_tool_call_per_subagent_updated_in_place() {
        let mut projection = SubagentProjection::default();
        let first = wire(&projection.project("root", &snapshot(1, &json!({})), false));
        assert_eq!(first.len(), 1);
        assert_eq!(first[0]["sessionId"], "root");
        let update = &first[0]["update"];
        assert_eq!(update["sessionUpdate"], "tool_call");
        assert_eq!(update["toolCallId"], "cowboy-subagent:child");
        assert_eq!(update["kind"], "think");
        assert_eq!(update["title"], "Subagent: Explore");
        assert_eq!(update["status"], "in_progress");
        let body = update["content"][0]["content"]["text"].as_str().unwrap();
        assert!(body.contains("Asked: Find the bug"));
        assert!(body.contains("Working · 1 tool call"));
        assert!(body.contains("Now: Read a.rs"));

        // The same picture again sends nothing; an older one is ignored.
        assert!(
            projection
                .project("root", &snapshot(1, &json!({})), false)
                .is_empty()
        );
        let done = snapshot(
            3,
            &json!({ "state": "idle", "stopReason": "end_turn", "cancellable": false, "result": "In a.rs" }),
        );
        let done = wire(&projection.project("root", &done, false));
        assert_eq!(done[0]["update"]["sessionUpdate"], "tool_call_update");
        assert_eq!(done[0]["update"]["status"], "completed");
        let body = done[0]["update"]["content"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(body.contains("Result: In a.rs"));
        assert!(
            projection
                .project("root", &snapshot(2, &json!({})), false)
                .is_empty()
        );

        // A stop or failure is not presented as done; silence claims nothing.
        let mut other = SubagentProjection::default();
        let stopped = snapshot(1, &json!({ "state": "idle", "stopReason": "cancelled" }));
        assert_eq!(
            wire(&other.project("root", &stopped, false))[0]["update"]["status"],
            "failed"
        );
        let mut quiet = SubagentProjection::default();
        let silent =
            wire(&quiet.project("root", &snapshot(1, &json!({ "reporting": false })), false));
        assert_eq!(silent[0]["update"]["status"], "in_progress");
        assert!(
            silent[0]["update"]["content"][0]["content"]["text"]
                .as_str()
                .unwrap()
                .contains("No longer reporting")
        );
    }

    #[test]
    fn a_subagent_client_gets_the_standard_form_and_the_child_activity_once() {
        let mut projection = SubagentProjection::default();
        let first = wire(&projection.project("root", &snapshot(1, &json!({})), true));
        let kinds: Vec<_> = first
            .iter()
            .map(|n| {
                (
                    n["sessionId"].as_str().unwrap(),
                    n["update"]["sessionUpdate"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            kinds,
            [
                ("root", "subagent_update"),
                ("root", "session_message"),
                ("child", "tool_call")
            ]
        );
        assert_eq!(first[0]["update"]["sessionId"], "child");
        assert_eq!(first[0]["update"]["capabilities"], json!({ "cancel": {} }));
        assert_eq!(first[0]["update"]["state"], json!({ "state": "running" }));
        assert_eq!(first[1]["update"]["recipientSessionId"], "child");
        assert_eq!(first[2]["update"]["kind"], "read");
        assert!(projection.knows("child"));

        let streaming = |revision, text: &str| {
            snapshot(
                revision,
                &json!({
                    "messages": 1,
                    "activity": [
                        { "kind": "tool", "id": "t1", "title": "Read a.rs", "toolKind": "read", "status": "completed" },
                        { "kind": "message", "text": text },
                    ],
                }),
            )
        };
        let next = wire(&projection.project("root", &streaming(2, "It is"), true));
        assert_eq!(
            next.len(),
            2,
            "unchanged announcement and prompt are not resent"
        );
        assert_eq!(next[0]["update"]["sessionUpdate"], "tool_call_update");
        assert_eq!(next[0]["update"]["status"], "completed");
        assert_eq!(next[1]["update"]["content"]["text"], "It is");
        let more = wire(&projection.project("root", &streaming(3, "It is in a.rs"), true));
        assert_eq!(more.len(), 1);
        assert_eq!(
            more[0]["update"]["content"]["text"], " in a.rs",
            "only the new text"
        );

        let done = snapshot(
            4,
            &json!({
                "state": "idle", "stopReason": "error", "error": "boom", "cancellable": false,
                "result": "partial", "activity": [], "omittedActivity": 2,
            }),
        );
        let done = wire(&projection.project("root", &done, true));
        assert_eq!(done[0]["update"]["capabilities"], json!({}));
        assert_eq!(
            done[0]["update"]["state"],
            json!({ "state": "idle", "stopReason": "error", "error": { "code": -32603, "message": "boom" } })
        );
        assert_eq!(done.last().unwrap()["update"]["senderSessionId"], "child");

        // A quiet child is unknown, never finished.
        let mut quiet = SubagentProjection::default();
        let silent =
            wire(&quiet.project("root", &snapshot(1, &json!({ "reporting": false })), true));
        assert_eq!(silent[0]["update"]["state"], json!({ "state": "unknown" }));
        // A nested child is announced on its parent child.
        let mut nested = SubagentProjection::default();
        let inner = wire(&nested.project(
            "root",
            &snapshot(
                1,
                &json!({ "sessionId": "grandchild", "parentSessionId": "child" }),
            ),
            true,
        ));
        assert_eq!(inner[0]["sessionId"], "child");
    }

    #[test]
    fn a_child_permission_request_names_the_subagent() {
        let request = json!({
            "toolCallId": "t3", "title": "cargo test",
            "_cowboy": { "subagent": { "sessionId": "child", "title": "Explore" } },
        });
        let mut legacy = request.clone();
        assert_eq!(subagent_request(&mut legacy, false), None);
        assert_eq!(legacy["title"], "Subagent Explore: cargo test");
        let mut standard = request.clone();
        assert_eq!(
            subagent_request(&mut standard, true).as_deref(),
            Some("child")
        );
        assert_eq!(standard["title"], "cargo test");
        let mut own = json!({ "toolCallId": "t4", "title": "ls" });
        assert_eq!(subagent_request(&mut own, false), None);
        assert_eq!(own["title"], "ls");
    }
}
