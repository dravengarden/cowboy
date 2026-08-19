//! Durable origin of a user-role timeline prompt.
//!
//! Cowboy stores every prompt as `user_message_chunk`. That role is not the
//! same as "the human typed this". Future senders add a `source` string; they
//! do not invent another actor.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent_model::{AUTO_CONTINUE_PREFIX, SCHED_PREFIX, WAKEUP_PREFIX};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptActor {
    Human,
    Cowboy,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptOrigin {
    pub actor: PromptActor,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

impl PromptOrigin {
    pub fn human_composer() -> Self {
        Self {
            actor: PromptActor::Human,
            source: "composer".to_owned(),
            provider: None,
        }
    }

    pub fn cowboy(source: &str) -> Self {
        Self {
            actor: PromptActor::Cowboy,
            source: source.to_owned(),
            provider: None,
        }
    }

    pub fn agent(source: &str, provider: &str) -> Self {
        Self {
            actor: PromptActor::Agent,
            source: source.to_owned(),
            provider: Some(provider.to_owned()),
        }
    }

    pub fn is_human(&self) -> bool {
        self.actor == PromptActor::Human
    }
}

pub fn origin_from_cmid(cmid: Option<&str>) -> PromptOrigin {
    match cmid {
        Some(value) if value.starts_with(AUTO_CONTINUE_PREFIX) => {
            PromptOrigin::cowboy("auto-resume")
        }
        Some(value) if value.starts_with(WAKEUP_PREFIX) || value.starts_with(SCHED_PREFIX) => {
            PromptOrigin::cowboy("schedule")
        }
        _ => PromptOrigin::human_composer(),
    }
}

/// Grok Build's design-review / writer loop injects a user-role prompt
/// without `<system-reminder>` wrappers. Several fingerprints are required
/// so a human mentioning "review_file" stays a human bubble.
pub fn is_grok_runtime_task_prompt(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lowered = trimmed.to_ascii_lowercase();
    let mut hits = 0u8;
    if lowered.contains("/tmp/grok-") {
        hits += 2;
    }
    if lowered.contains("grok-design-review-") || lowered.contains("grok-design-doc-") {
        hits += 2;
    }
    if lowered.contains("the reviewer found issues") {
        hits += 2;
    }
    if lowered.contains("review_file") {
        hits += 1;
    }
    if lowered.contains("status: open") && lowered.contains("addressed") {
        hits += 1;
    }
    if lowered.contains("add a response field") {
        hits += 1;
    }
    if lowered.contains("wontfix") && lowered.contains("needs-user-input") {
        hits += 1;
    }
    hits >= 3
}

pub fn is_internal_runtime_prompt(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if is_grok_runtime_task_prompt(trimmed) {
        return true;
    }
    let lowered = trimmed.to_ascii_lowercase();
    if lowered.starts_with("<system-reminder") {
        return true;
    }
    let stripped = strip_system_reminder_blocks(trimmed);
    stripped.trim().is_empty() && lowered.contains("<system-reminder")
}

fn strip_system_reminder_blocks(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut rest_lower = lower.as_str();
    while let Some(start) = rest_lower.find("<system-reminder") {
        out.push_str(&rest[..start]);
        let after_open = start + "<system-reminder".len();
        let close_rel = rest_lower[after_open..]
            .find("</system-reminder>")
            .map(|index| after_open + index + "</system-reminder>".len());
        match close_rel {
            Some(end) => {
                rest = &rest[end..];
                rest_lower = &rest_lower[end..];
            }
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

pub fn apply_prompt_origin(update: &mut Value, origin: &PromptOrigin) {
    if let Ok(value) = serde_json::to_value(origin) {
        update["promptOrigin"] = value;
    }
    if !origin.is_human() {
        update["autoResumed"] = Value::Bool(true);
    }
}

pub fn annotate_inbound_user_prompt(update: &mut Value, provider_id: &str) {
    if update.get("sessionUpdate").and_then(Value::as_str) != Some("user_message_chunk") {
        return;
    }
    if update.get("promptOrigin").is_some() {
        return;
    }
    let text = update
        .pointer("/content/text")
        .and_then(Value::as_str)
        .unwrap_or("");
    if is_grok_runtime_task_prompt(text) {
        apply_prompt_origin(update, &PromptOrigin::agent("review", "grok"));
        return;
    }
    if is_internal_runtime_prompt(text) {
        apply_prompt_origin(update, &PromptOrigin::agent("runtime", provider_id));
    }
}

/// Whether two ACP user-content blocks are the same prompt piece.
///
/// Grok (and any agent that "accepts" a prompt by echoing it) reserializes the
/// same type/text/url/data. Extra keys must not keep a replay visible.
pub fn user_content_equiv(left: &Value, right: &Value) -> bool {
    let left_type = left.get("type").and_then(Value::as_str);
    if left_type != right.get("type").and_then(Value::as_str) {
        return false;
    }
    match left_type {
        Some("text") => left.get("text") == right.get("text"),
        Some("image") => {
            left.get("url") == right.get("url")
                && left.get("data") == right.get("data")
                && left.get("mimeType") == right.get("mimeType")
        }
        _ => left == right,
    }
}

/// Drop an inbound `user_message_chunk` that replays a prompt Cowboy already
/// echoed. Runtime injections are stamped with `promptOrigin` first and stay.
/// Returns true when `remaining` consumed the matching echoed block.
pub fn take_matching_inbound_prompt_echo(update: &Value, remaining: &mut Vec<Value>) -> bool {
    if update.get("sessionUpdate").and_then(Value::as_str) != Some("user_message_chunk") {
        return false;
    }
    if update.get("promptOrigin").is_some() {
        return false;
    }
    let Some(content) = update.get("content") else {
        return false;
    };
    if remaining
        .first()
        .is_some_and(|expected| user_content_equiv(expected, content))
    {
        remaining.remove(0);
        return true;
    }
    false
}

pub fn is_human_prompt_update(update: &Value) -> bool {
    if update.get("autoResumed").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    if let Some(actor) = update
        .pointer("/promptOrigin/actor")
        .and_then(Value::as_str)
        && actor != "human"
    {
        return false;
    }
    let text = update
        .pointer("/content/text")
        .and_then(Value::as_str)
        .unwrap_or("");
    !is_internal_runtime_prompt(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmid_prefixes_select_cowboy_sources() {
        assert_eq!(
            origin_from_cmid(Some("__cont__abc")),
            PromptOrigin::cowboy("auto-resume")
        );
        assert_eq!(
            origin_from_cmid(Some("__wake__abc")),
            PromptOrigin::cowboy("schedule")
        );
        assert_eq!(
            origin_from_cmid(Some("cmid-1")),
            PromptOrigin::human_composer()
        );
    }

    #[test]
    fn runtime_markup_is_agent_owned() {
        assert!(is_internal_runtime_prompt(
            "<system-reminder>Background task completed.</system-reminder>"
        ));
        assert!(is_internal_runtime_prompt(
            "<system-reminder>Background task still running"
        ));
        assert!(!is_internal_runtime_prompt(
            "Please do not leak <system-reminder> tags"
        ));
    }

    #[test]
    fn grok_review_follow_up_is_agent_owned() {
        let prompt = "The reviewer found issues. The review_file is at: /tmp/grok-1000/grok-design-review-d58766af.md\n\nRead the review_file. Address ALL issues with Status: open -- including nits.\nThen update the review_file:\n- Status: open -> addressed\n- Add a Response field\nYou may set wontfix or needs-user-input.";
        assert!(is_grok_runtime_task_prompt(prompt));
        assert!(is_internal_runtime_prompt(prompt));
        assert!(!is_grok_runtime_task_prompt("Read the review_file please"));
        assert!(!is_grok_runtime_task_prompt(
            "The reviewer found issues in my PR"
        ));
        let mut update = serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": { "type": "text", "text": prompt }
        });
        annotate_inbound_user_prompt(&mut update, "grok");
        assert_eq!(update["promptOrigin"]["actor"], "agent");
        assert_eq!(update["promptOrigin"]["source"], "review");
        assert_eq!(update["promptOrigin"]["provider"], "grok");
        assert!(!is_human_prompt_update(&update));
    }

    #[test]
    fn inbound_annotation_stamps_grok_runtime() {
        let mut update = serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": { "type": "text", "text": "<system-reminder>done</system-reminder>" }
        });
        annotate_inbound_user_prompt(&mut update, "grok");
        assert_eq!(update["promptOrigin"]["actor"], "agent");
        assert_eq!(update["promptOrigin"]["source"], "runtime");
        assert_eq!(update["promptOrigin"]["provider"], "grok");
        assert_eq!(update["autoResumed"], true);
        assert!(!is_human_prompt_update(&update));
    }

    #[test]
    fn inbound_grok_prompt_echo_consumes_the_cowboy_echo() {
        let mut remaining = vec![
            serde_json::json!({"type": "image", "mimeType": "image/jpeg", "url": "blob:1"}),
            serde_json::json!({"type": "text", "text": "why two messages?"}),
        ];
        let image = serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": {"type": "image", "mimeType": "image/jpeg", "url": "blob:1"}
        });
        let text = serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": {"type": "text", "text": "why two messages?"}
        });
        assert!(take_matching_inbound_prompt_echo(&image, &mut remaining));
        assert!(take_matching_inbound_prompt_echo(&text, &mut remaining));
        assert!(remaining.is_empty());
    }

    #[test]
    fn inbound_runtime_injection_is_not_a_prompt_echo() {
        let mut remaining = vec![serde_json::json!({"type": "text", "text": "hello"})];
        let mut update = serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": {"type": "text", "text": "<system-reminder>done</system-reminder>"}
        });
        annotate_inbound_user_prompt(&mut update, "grok");
        assert!(!take_matching_inbound_prompt_echo(&update, &mut remaining));
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn inbound_unrelated_user_chunk_is_kept() {
        let mut remaining = vec![serde_json::json!({"type": "text", "text": "hello"})];
        let update = serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": {"type": "text", "text": "something else"}
        });
        assert!(!take_matching_inbound_prompt_echo(&update, &mut remaining));
        assert_eq!(remaining.len(), 1);
    }
}
