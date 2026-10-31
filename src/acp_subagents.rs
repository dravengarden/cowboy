//! ACP subagents (`unstable_subagents`) inside one Cowboy session timeline.
//!
//! An adapter that sees `clientCapabilities.subagents` runs each subagent as
//! its own child ACP session. The child's messages and tool calls arrive as
//! `session/update` notifications whose `sessionId` is the child's; the root
//! session receives `subagent_update` (announcement, labels, cancel capability
//! and work state) and linked `session_message`s (the delegation prompt and
//! the reply).
//!
//! Cowboy owns one transcript per session, so the worker never forwards a
//! child's own updates. It folds them into a bounded snapshot of that child
//! and publishes the snapshot as a [`SUBAGENT_UPDATE`] in the root timeline.
//! Every snapshot is complete and carries a monotonic `revision`; a later one
//! replaces an earlier one for the same child.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

pub use crate::runtime_wire::SUBAGENT_UPDATE;

/// Children kept in memory per root session. An evicted child keeps its last
/// durable row; its late updates are still kept out of the root timeline.
const MAX_CHILDREN: usize = 128;
/// Ids of evicted children whose late updates are still dropped.
const MAX_RETIRED: usize = 1024;
/// Most recent activity entries (messages and tool calls) per child.
const MAX_ACTIVITY: usize = 24;
/// Text kept for one activity entry.
const ACTIVITY_TEXT_BYTES: usize = 600;
/// Text kept for the delegation prompt, the result and the last message.
const MESSAGE_BYTES: usize = 8 * 1024;
/// Text kept for a title, description or tool title.
const LABEL_BYTES: usize = 240;
/// Streaming text alone republishes a child at most this often. Structural
/// changes (state, tool calls, a new message) publish at once.
const TEXT_PUBLISH_INTERVAL: Duration = Duration::from_secs(1);

/// The outcome of routing one notification.
#[derive(Debug, Default)]
pub struct Routed {
    /// The update belongs to the root session's own timeline.
    pub root: bool,
    /// Subagent snapshots to publish in the root timeline, in order.
    pub snapshots: Vec<Value>,
}

#[derive(Debug, Default, Clone)]
struct Text {
    value: String,
    truncated: bool,
}

impl Text {
    fn set(&mut self, text: &str, limit: usize) -> bool {
        let before = std::mem::take(&mut self.value);
        let was_truncated = self.truncated;
        self.truncated = false;
        self.append(text, limit);
        before != self.value || was_truncated != self.truncated
    }

    fn append(&mut self, text: &str, limit: usize) -> bool {
        if self.truncated || text.is_empty() {
            return false;
        }
        let room = limit.saturating_sub(self.value.len());
        if text.len() <= room {
            self.value.push_str(text);
            return true;
        }
        let mut cut = room;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        self.value.push_str(&text[..cut]);
        self.value.push('…');
        self.truncated = true;
        true
    }

    fn get(&self) -> Option<&str> {
        (!self.value.is_empty()).then_some(self.value.as_str())
    }
}

#[derive(Debug, Clone)]
enum Activity {
    Message {
        message_id: Option<String>,
        text: Text,
    },
    Tool {
        id: String,
        title: Text,
        kind: Option<String>,
        status: String,
    },
}

impl Activity {
    fn to_json(&self) -> Value {
        match self {
            Self::Message { text, .. } => json!({ "kind": "message", "text": text.value }),
            Self::Tool {
                id,
                title,
                kind,
                status,
            } => json!({
                "kind": "tool",
                "id": id,
                "title": title.value,
                "toolKind": kind,
                "status": status,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Link {
    Prompt,
    Result,
}

#[derive(Debug, Default)]
struct Child {
    /// The child session that announced this one, or `None` for a direct
    /// child of the root session.
    parent: Option<String>,
    title: Option<Text>,
    description: Option<Text>,
    cancel: bool,
    state: Option<String>,
    stop_reason: Option<String>,
    error: Option<Text>,
    prompt: Text,
    reply: Text,
    last_message: Text,
    tool_calls: u32,
    messages: u32,
    activity: VecDeque<Activity>,
    omitted: u32,
    last_publish: Option<Instant>,
    dirty: bool,
}

impl Child {
    fn working(&self) -> bool {
        matches!(self.state.as_deref(), Some("running" | "requires_action"))
    }

    fn push_activity(&mut self, entry: Activity) {
        if self.activity.len() == MAX_ACTIVITY {
            self.activity.pop_front();
            self.omitted = self.omitted.saturating_add(1);
        }
        self.activity.push_back(entry);
    }

    /// What the parent sees as the child's answer: the reply the adapter
    /// linked to the root session, else, once its work ended, its last
    /// message.
    fn result(&self) -> Option<&str> {
        self.reply.get().or_else(|| {
            matches!(self.state.as_deref(), Some("idle"))
                .then(|| self.last_message.get())
                .flatten()
        })
    }
}

/// The subagents of one root session, as announced on its ACP connection.
#[derive(Debug, Default)]
pub struct Subagents {
    children: HashMap<String, Child>,
    order: VecDeque<String>,
    retired: HashSet<String>,
    retired_order: VecDeque<String>,
    /// `(transcript session, messageId)` of a `session_message` linked to a
    /// child, so its later upserts and chunks reach the same field.
    links: HashMap<(String, String), (String, Link)>,
    revision: u64,
}

/// Apply a nullable label patch: absent leaves `target`, a string sets it,
/// anything else (`null`) clears it.
fn patch_label(target: &mut Option<Text>, update: &Value, key: &str) {
    if let Some(value) = update.get(key) {
        *target = value.as_str().map(label);
    }
}

fn label(value: &str) -> Text {
    let mut text = Text::default();
    text.set(value.trim(), LABEL_BYTES);
    text
}

/// Plain text of a `session_message` content array.
fn content_text(content: &Value) -> String {
    let Some(blocks) = content.as_array() else {
        return String::new();
    };
    let texts: Vec<&str> = blocks
        .iter()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect();
    texts.join("\n\n")
}

fn chunk_text(update: &Value) -> Option<&str> {
    (update.pointer("/content/type")? == "text")
        .then(|| update.pointer("/content/text")?.as_str())
        .flatten()
}

impl Subagents {
    /// Whether `session_id` is a child (or an evicted child) of this session.
    #[must_use]
    pub fn owns(&self, session_id: &str) -> bool {
        self.children.contains_key(session_id) || self.retired.contains(session_id)
    }

    /// Children whose foreground work is in progress.
    #[must_use]
    pub fn working(&self) -> u32 {
        let count = self
            .children
            .values()
            .filter(|child| child.working())
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// The child may be stopped alone with `session/cancel`.
    #[must_use]
    pub fn can_cancel(&self, session_id: &str) -> bool {
        self.children
            .get(session_id)
            .is_some_and(|child| child.cancel && child.working())
    }

    /// The identity a permission or elicitation request raised by a child
    /// carries, so the person answering knows which subagent asks.
    #[must_use]
    pub fn request_tag(&self, session_id: &str) -> Option<Value> {
        let child = self.children.get(session_id)?;
        Some(json!({
            "sessionId": session_id,
            "title": child.title.as_ref().map(|title| title.value.as_str()),
        }))
    }

    /// Route one `session/update` of `session_id`. `update` is the
    /// serialized `SessionUpdate`.
    pub fn route(&mut self, session_id: &str, update: &Value, now: Instant, now_ms: u64) -> Routed {
        let kind = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if self.children.contains_key(session_id) {
            let mut routed = Routed::default();
            if kind == "subagent_update" {
                if let Some(snapshot) = self.announce(Some(session_id), update, now, now_ms) {
                    routed.snapshots.push(snapshot);
                }
            } else if self.absorb(session_id, kind, update, now) {
                routed.snapshots.push(self.publish(session_id, now, now_ms));
            }
            return routed;
        }
        if self.retired.contains(session_id) {
            return Routed::default();
        }
        let mut routed = Routed {
            root: false,
            snapshots: Vec::new(),
        };
        match kind {
            "subagent_update" => {
                if let Some(snapshot) = self.announce(None, update, now, now_ms) {
                    routed.snapshots.push(snapshot);
                }
            }
            "session_message" | "session_message_chunk" => {
                match self.link_message(session_id, kind, update) {
                    Some(child) => routed.snapshots.push(self.publish(&child, now, now_ms)),
                    None => routed.root = true,
                }
            }
            _ => routed.root = true,
        }
        if routed.root {
            routed.snapshots.extend(self.due(now, now_ms));
        }
        routed
    }

    /// Snapshots of children whose streamed text has waited long enough.
    pub fn due(&mut self, now: Instant, now_ms: u64) -> Vec<Value> {
        let ready: Vec<String> = self
            .order
            .iter()
            .filter(|id| {
                self.children.get(id.as_str()).is_some_and(|child| {
                    child.dirty
                        && child
                            .last_publish
                            .is_none_or(|at| now.duration_since(at) >= TEXT_PUBLISH_INTERVAL)
                })
            })
            .cloned()
            .collect();
        ready
            .into_iter()
            .map(|id| self.publish(&id, now, now_ms))
            .collect()
    }

    /// Apply a `subagent_update` whose transcript is `parent` (`None`: the
    /// root session). Every field is a patch: absent leaves it, `null`
    /// clears it.
    fn announce(
        &mut self,
        parent: Option<&str>,
        update: &Value,
        now: Instant,
        now_ms: u64,
    ) -> Option<Value> {
        let id = update
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && Some(*id) != parent)?
            .to_owned();
        if !self.children.contains_key(&id) {
            self.insert(&id, parent);
        }
        let child = self.children.get_mut(&id)?;
        patch_label(&mut child.title, update, "title");
        patch_label(&mut child.description, update, "description");
        if let Some(capabilities) = update.get("capabilities") {
            child.cancel = capabilities
                .get("cancel")
                .is_some_and(|cancel| !cancel.is_null());
        }
        if let Some(state) = update.get("state") {
            let next = state
                .get("state")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if next.as_deref() == Some("running") && child.state.as_deref() == Some("idle") {
                // Resumed: the earlier run's outcome no longer describes it.
                child.reply = Text::default();
                child.last_message = Text::default();
            }
            child.state = next;
            child.stop_reason = state
                .get("stopReason")
                .and_then(Value::as_str)
                .map(str::to_owned);
            child.error = state
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(label);
        }
        Some(self.publish(&id, now, now_ms))
    }

    fn insert(&mut self, id: &str, parent: Option<&str>) {
        if self.retired.remove(id) {
            self.retired_order.retain(|retired| retired != id);
        }
        if self.children.len() >= MAX_CHILDREN {
            let victim = self
                .order
                .iter()
                .position(|old| self.children.get(old).is_none_or(|child| !child.working()))
                .unwrap_or(0);
            if let Some(old) = self.order.remove(victim) {
                self.children.remove(&old);
                self.links.retain(|_, (child, _)| child != &old);
                self.retired.insert(old.clone());
                self.retired_order.push_back(old);
                while self.retired_order.len() > MAX_RETIRED {
                    if let Some(expired) = self.retired_order.pop_front() {
                        self.retired.remove(&expired);
                    }
                }
            }
        }
        self.children.insert(
            id.to_owned(),
            Child {
                parent: parent.map(str::to_owned),
                ..Child::default()
            },
        );
        self.order.push_back(id.to_owned());
    }

    /// Link a root-transcript `session_message` (or a chunk of one) to the
    /// child it was sent to or received from. `None`: no known child.
    fn link_message(&mut self, transcript: &str, kind: &str, update: &Value) -> Option<String> {
        let message_id = update.get("messageId").and_then(Value::as_str)?.to_owned();
        let key = (transcript.to_owned(), message_id);
        let known = self.links.get(&key).cloned();
        let (child_id, link) = match known {
            Some(link) => link,
            None if kind == "session_message" => {
                let sender = update.get("senderSessionId").and_then(Value::as_str);
                let recipient = update.get("recipientSessionId").and_then(Value::as_str);
                let link =
                    if let Some(child) = recipient.filter(|id| self.children.contains_key(*id)) {
                        (child.to_owned(), Link::Prompt)
                    } else {
                        let child = sender.filter(|id| self.children.contains_key(*id))?;
                        (child.to_owned(), Link::Result)
                    };
                self.links.insert(key, link.clone());
                link
            }
            None => return None,
        };
        let child = self.children.get_mut(&child_id)?;
        let field = match link {
            Link::Prompt => &mut child.prompt,
            Link::Result => &mut child.reply,
        };
        if kind == "session_message_chunk" {
            if let Some(text) = chunk_text(update) {
                field.append(text, MESSAGE_BYTES);
            }
        } else if let Some(content) = update.get("content") {
            field.set(&content_text(content), MESSAGE_BYTES);
        }
        Some(child_id)
    }

    /// Fold one update from inside child `id`. Returns whether the child
    /// must be republished now.
    fn absorb(&mut self, id: &str, kind: &str, update: &Value, now: Instant) -> bool {
        if matches!(kind, "session_message" | "session_message_chunk") {
            // The child's own view of a delegation or of its reply. A root
            // view, when the adapter sends one, carries the same text.
            return self.absorb_child_message(id, kind, update);
        }
        let Some(child) = self.children.get_mut(id) else {
            return false;
        };
        match kind {
            "agent_message_chunk" => {
                let Some(text) = chunk_text(update) else {
                    return false;
                };
                let message_id = update
                    .get("messageId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if let Some(Activity::Message {
                    message_id: current,
                    text: entry,
                }) = child.activity.back_mut()
                    && *current == message_id
                {
                    entry.append(text, ACTIVITY_TEXT_BYTES);
                    child.last_message.append(text, MESSAGE_BYTES);
                    child.dirty = true;
                    return child
                        .last_publish
                        .is_none_or(|at| now.duration_since(at) >= TEXT_PUBLISH_INTERVAL);
                }
                let mut entry = Text::default();
                entry.append(text, ACTIVITY_TEXT_BYTES);
                child.last_message.set(text, MESSAGE_BYTES);
                child.messages = child.messages.saturating_add(1);
                child.push_activity(Activity::Message {
                    message_id,
                    text: entry,
                });
                true
            }
            "tool_call" => {
                let Some(tool) = update.get("toolCallId").and_then(Value::as_str) else {
                    return false;
                };
                let title = update.get("title").and_then(Value::as_str).unwrap_or(tool);
                child.tool_calls = child.tool_calls.saturating_add(1);
                child.push_activity(Activity::Tool {
                    id: tool.to_owned(),
                    title: label(title),
                    kind: update
                        .get("kind")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    status: update
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("pending")
                        .to_owned(),
                });
                true
            }
            "tool_call_update" => {
                let Some(tool) = update.get("toolCallId").and_then(Value::as_str) else {
                    return false;
                };
                let Some(Activity::Tool { title, status, .. }) = child
                    .activity
                    .iter_mut()
                    .rev()
                    .find(|entry| matches!(entry, Activity::Tool { id, .. } if id == tool))
                else {
                    return false;
                };
                let mut changed = false;
                if let Some(next) = update.get("status").and_then(Value::as_str)
                    && next != status
                {
                    next.clone_into(status);
                    changed = true;
                }
                if let Some(next) = update.get("title").and_then(Value::as_str) {
                    changed |= title.set(next.trim(), LABEL_BYTES);
                }
                changed
            }
            // Thoughts, plans, usage, configuration and session metadata of
            // a child are the child's own; the root shows only its outline.
            _ => false,
        }
    }

    fn absorb_child_message(&mut self, id: &str, kind: &str, update: &Value) -> bool {
        let Some(message_id) = update.get("messageId").and_then(Value::as_str) else {
            return false;
        };
        let key = (id.to_owned(), message_id.to_owned());
        let link = match self.links.get(&key) {
            Some((_, link)) => *link,
            None if kind == "session_message" => {
                let sender = update.get("senderSessionId").and_then(Value::as_str);
                let recipient = update.get("recipientSessionId").and_then(Value::as_str);
                let link = if sender == Some(id) {
                    Link::Result
                } else if recipient == Some(id) || recipient.is_none() {
                    Link::Prompt
                } else {
                    return false;
                };
                self.links.insert(key, (id.to_owned(), link));
                link
            }
            None => return false,
        };
        let Some(child) = self.children.get_mut(id) else {
            return false;
        };
        let field = match link {
            Link::Prompt => &mut child.prompt,
            Link::Result => &mut child.reply,
        };
        if kind == "session_message_chunk" {
            chunk_text(update).is_some_and(|text| field.append(text, MESSAGE_BYTES))
        } else {
            update
                .get("content")
                .is_some_and(|content| field.set(&content_text(content), MESSAGE_BYTES))
        }
    }

    fn publish(&mut self, id: &str, now: Instant, now_ms: u64) -> Value {
        self.revision = (self.revision + 1).max(now_ms);
        let revision = self.revision;
        let Some(child) = self.children.get_mut(id) else {
            return Value::Null;
        };
        child.dirty = false;
        child.last_publish = Some(now);
        json!({
            "sessionUpdate": SUBAGENT_UPDATE,
            "sessionId": id,
            "revision": revision,
            "parentSessionId": child.parent,
            "title": child.title.as_ref().map(|text| text.value.as_str()),
            "description": child.description.as_ref().map(|text| text.value.as_str()),
            "state": child.state,
            "stopReason": child.stop_reason,
            "error": child.error.as_ref().map(|text| text.value.as_str()),
            "cancellable": child.cancel && child.working(),
            "prompt": child.prompt.get(),
            "result": child.result(),
            "toolCalls": child.tool_calls,
            "messages": child.messages,
            "activity": child.activity.iter().map(Activity::to_json).collect::<Vec<_>>(),
            "omittedActivity": child.omitted,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "root";

    fn announce(id: &str, extra: &Value) -> Value {
        let mut update = json!({ "sessionUpdate": "subagent_update", "sessionId": id });
        if let (Some(target), Some(extra)) = (update.as_object_mut(), extra.as_object()) {
            target.extend(extra.clone());
        }
        update
    }

    fn chunk(text: &str) -> Value {
        json!({
            "sessionUpdate": "agent_message_chunk",
            "messageId": "m1",
            "content": { "type": "text", "text": text },
        })
    }

    #[test]
    fn a_child_is_one_snapshot_and_its_own_updates_never_reach_the_root() {
        let mut subagents = Subagents::default();
        let now = Instant::now();
        let routed = subagents.route(
            ROOT,
            &announce(
                "child",
                &json!({
                    "title": "Explore",
                    "description": "Explore",
                    "capabilities": { "cancel": {} },
                    "state": { "state": "running" },
                }),
            ),
            now,
            10,
        );
        assert!(!routed.root);
        let snapshot = &routed.snapshots[0];
        assert_eq!(snapshot["sessionUpdate"], SUBAGENT_UPDATE);
        assert_eq!(snapshot["sessionId"], "child");
        assert_eq!(snapshot["title"], "Explore");
        assert_eq!(snapshot["cancellable"], true);
        assert!(subagents.can_cancel("child"));
        assert_eq!(subagents.working(), 1);

        for update in [
            json!({ "sessionUpdate": "config_option_update", "configOptions": [] }),
            json!({ "sessionUpdate": "usage_update", "used": 1, "size": 2 }),
            json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "hm" } }),
        ] {
            let routed = subagents.route("child", &update, now, 11);
            assert!(!routed.root);
            assert!(routed.snapshots.is_empty());
        }
        let routed = subagents.route(
            "child",
            &json!({ "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "Read a.rs", "kind": "read", "status": "in_progress" }),
            now,
            12,
        );
        assert!(!routed.root);
        assert_eq!(routed.snapshots[0]["toolCalls"], 1);
        assert_eq!(routed.snapshots[0]["activity"][0]["title"], "Read a.rs");
        let routed = subagents.route(
            "child",
            &json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed", "content": [] }),
            now,
            13,
        );
        assert_eq!(routed.snapshots[0]["activity"][0]["status"], "completed");
        // A content-only update republishes nothing.
        let routed = subagents.route(
            "child",
            &json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t1", "content": [] }),
            now,
            14,
        );
        assert!(routed.snapshots.is_empty());

        let root = subagents.route(ROOT, &chunk("parent text"), now, 15);
        assert!(root.root);
    }

    #[test]
    fn streamed_child_text_is_throttled_and_flushed_by_later_root_activity() {
        let mut subagents = Subagents::default();
        let start = Instant::now();
        subagents.route(
            ROOT,
            &announce("child", &json!({ "state": { "state": "running" } })),
            start,
            1,
        );
        let first = subagents.route("child", &chunk("Hello"), start, 2);
        assert_eq!(first.snapshots[0]["activity"][0]["text"], "Hello");
        let second = subagents.route("child", &chunk(" world"), start, 3);
        assert!(
            second.snapshots.is_empty(),
            "text inside the interval waits"
        );
        let later = start + TEXT_PUBLISH_INTERVAL;
        let flushed = subagents.route(
            ROOT,
            &json!({ "sessionUpdate": "plan", "entries": [] }),
            later,
            4,
        );
        assert!(flushed.root);
        assert_eq!(flushed.snapshots[0]["activity"][0]["text"], "Hello world");
        let idle = subagents.route(
            ROOT,
            &announce(
                "child",
                &json!({ "state": { "state": "idle", "stopReason": "end_turn" } }),
            ),
            later,
            5,
        );
        let snapshot = &idle.snapshots[0];
        assert_eq!(snapshot["state"], "idle");
        assert_eq!(
            snapshot["result"], "Hello world",
            "the last message is the outcome"
        );
        assert_eq!(snapshot["cancellable"], false);
        assert!(snapshot["revision"].as_u64() > flushed.snapshots[0]["revision"].as_u64());
        assert_eq!(subagents.working(), 0);
    }

    #[test]
    fn linked_session_messages_become_the_prompt_and_the_reply() {
        let mut subagents = Subagents::default();
        let now = Instant::now();
        subagents.route(
            ROOT,
            &announce("child", &json!({ "state": { "state": "running" } })),
            now,
            1,
        );
        let prompt = json!({
            "sessionUpdate": "session_message", "messageId": "toolu_1",
            "senderSessionId": ROOT, "recipientSessionId": "child", "recipientMessageId": "toolu_1",
            "content": [{ "type": "text", "text": "Find the bug" }],
        });
        let routed = subagents.route(ROOT, &prompt, now, 2);
        assert!(!routed.root);
        assert_eq!(routed.snapshots[0]["prompt"], "Find the bug");
        // The child's incoming view of the same delegation changes nothing.
        let mut incoming = prompt.clone();
        incoming["senderMessageId"] = json!("toolu_1");
        assert!(
            subagents
                .route("child", &incoming, now, 3)
                .snapshots
                .is_empty()
        );
        let reply = json!({
            "sessionUpdate": "session_message", "messageId": "toolu_1:reply",
            "senderSessionId": "child", "recipientSessionId": ROOT,
            "content": [{ "type": "text", "text": "It is in a.rs" }],
        });
        let routed = subagents.route(ROOT, &reply, now, 4);
        assert_eq!(routed.snapshots[0]["result"], "It is in a.rs");
        let more = json!({
            "sessionUpdate": "session_message_chunk", "messageId": "toolu_1:reply",
            "content": { "type": "text", "text": ", line 3" },
        });
        assert_eq!(
            subagents.route(ROOT, &more, now, 5).snapshots[0]["result"],
            "It is in a.rs, line 3"
        );
        // A message between unknown sessions stays an ordinary root update.
        let unrelated = json!({
            "sessionUpdate": "session_message", "messageId": "x",
            "senderSessionId": "someone", "recipientSessionId": ROOT,
            "content": [{ "type": "text", "text": "hi" }],
        });
        assert!(subagents.route(ROOT, &unrelated, now, 6).root);
    }

    #[test]
    fn nested_children_name_their_parent_and_cancel_follows_the_latest_capability() {
        let mut subagents = Subagents::default();
        let now = Instant::now();
        subagents.route(
            ROOT,
            &announce(
                "child",
                &json!({ "capabilities": {}, "state": { "state": "running" } }),
            ),
            now,
            1,
        );
        assert!(!subagents.can_cancel("child"));
        subagents.route(
            ROOT,
            &announce("child", &json!({ "capabilities": { "cancel": {} } })),
            now,
            2,
        );
        assert!(subagents.can_cancel("child"));
        subagents.route(
            ROOT,
            &announce("child", &json!({ "capabilities": { "cancel": null } })),
            now,
            3,
        );
        assert!(!subagents.can_cancel("child"));
        let nested = subagents.route(
            "child",
            &announce(
                "grandchild",
                &json!({ "title": "Inner", "state": { "state": "running" } }),
            ),
            now,
            4,
        );
        assert!(!nested.root);
        assert_eq!(nested.snapshots[0]["sessionId"], "grandchild");
        assert_eq!(nested.snapshots[0]["parentSessionId"], "child");
        assert!(subagents.owns("grandchild"));
        assert_eq!(
            subagents.request_tag("grandchild").unwrap()["title"],
            "Inner"
        );
        assert!(subagents.request_tag(ROOT).is_none());
    }

    #[test]
    fn a_failed_child_reports_its_error_and_a_resume_clears_the_old_outcome() {
        let mut subagents = Subagents::default();
        let now = Instant::now();
        subagents.route(
            ROOT,
            &announce("child", &json!({ "state": { "state": "running" } })),
            now,
            1,
        );
        subagents.route("child", &chunk("partial"), now, 2);
        let failed = subagents.route(
            ROOT,
            &announce(
                "child",
                &json!({ "state": { "state": "idle", "stopReason": "error", "error": { "code": -32603, "message": "boom" } } }),
            ),
            now,
            3,
        );
        assert_eq!(failed.snapshots[0]["stopReason"], "error");
        assert_eq!(failed.snapshots[0]["error"], "boom");
        let resumed = subagents.route(
            ROOT,
            &announce("child", &json!({ "state": { "state": "running" } })),
            now,
            4,
        );
        assert_eq!(resumed.snapshots[0]["result"], Value::Null);
        assert_eq!(resumed.snapshots[0]["error"], Value::Null);
    }

    #[test]
    fn memory_and_snapshots_stay_bounded() {
        let mut subagents = Subagents::default();
        let now = Instant::now();
        subagents.route(
            ROOT,
            &announce("child", &json!({ "state": { "state": "running" } })),
            now,
            1,
        );
        let long = "x".repeat(MESSAGE_BYTES * 2);
        for index in 0..(MAX_ACTIVITY * 3) {
            subagents.route(
                "child",
                &json!({ "sessionUpdate": "tool_call", "toolCallId": format!("t{index}"), "title": long }),
                now,
                2,
            );
        }
        let snapshot = subagents.route(
            "child",
            &json!({ "sessionUpdate": "agent_message_chunk", "messageId": "m", "content": { "type": "text", "text": long } }),
            now,
            3,
        );
        let snapshot = &snapshot.snapshots[0];
        assert_eq!(snapshot["activity"].as_array().unwrap().len(), MAX_ACTIVITY);
        assert_eq!(snapshot["toolCalls"], MAX_ACTIVITY * 3);
        assert_eq!(snapshot["omittedActivity"], MAX_ACTIVITY * 2 + 1);
        assert!(serde_json::to_string(snapshot).unwrap().len() < 40 * 1024);

        for index in 0..(MAX_CHILDREN + 8) {
            subagents.route(
                ROOT,
                &announce(
                    &format!("c{index}"),
                    &json!({ "state": { "state": "idle" } }),
                ),
                now,
                4,
            );
        }
        assert!(subagents.children.len() <= MAX_CHILDREN);
        assert!(
            subagents.owns("c0"),
            "an evicted child stays out of the root timeline"
        );
        assert!(
            subagents
                .route("c0", &chunk("late"), now, 5)
                .snapshots
                .is_empty()
        );
        assert!(
            subagents.children.contains_key("child"),
            "working children are evicted last"
        );
    }
}
