//! Durable origin of a user-role timeline prompt.
//!
//! Cowboy stores every prompt as `user_message_chunk`. That role is not the
//! same as "the human typed this". Future senders add a `source` string; they
//! do not invent another actor.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent_model::{
    CALL_NOTICE_PREFIX, LEGACY_AUTO_CONTINUE_PREFIX, PEER_NOTICE_PREFIX, SCHED_PREFIX,
    WAKEUP_PREFIX,
};

pub use crate::delivery_origin::{
    DeliveryOrigin, PromptDispatchRecord, RequestCaller, compute_dispatch_origin,
};

impl DeliveryOrigin {
    /// Map this authoritative dispatch origin onto the timeline `PromptOrigin`
    /// stamped on `user_message_chunk` echoes.
    #[must_use]
    pub fn to_prompt_origin(&self) -> PromptOrigin {
        match self {
            Self::Owner => PromptOrigin::human_composer(),
            Self::Automation => PromptOrigin::cowboy("automation"),
            Self::Peer { .. } => PromptOrigin::cowboy("agent_message"),
            Self::Cowboy { source } => PromptOrigin::cowboy(source),
        }
    }
}

/// Authoritatively stamp `promptOrigin` and `autoResumed` on an echoed
/// `user_message_chunk` from Controller dispatch truth, overriding any
/// worker-guessed cmid label.
pub fn apply_authoritative_prompt_origin(update: &mut Value, origin: &DeliveryOrigin) {
    let prompt_origin = origin.to_prompt_origin();
    if let Ok(value) = serde_json::to_value(&prompt_origin) {
        update["promptOrigin"] = value;
    }
    if origin.is_owner() {
        if let Some(map) = update.as_object_mut() {
            map.remove("autoResumed");
        }
    } else {
        update["autoResumed"] = Value::Bool(true);
    }
}

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
        Some(value) if value.starts_with(LEGACY_AUTO_CONTINUE_PREFIX) => {
            PromptOrigin::cowboy("auto-resume")
        }
        Some(value) if value.starts_with(WAKEUP_PREFIX) || value.starts_with(SCHED_PREFIX) => {
            PromptOrigin::cowboy("schedule")
        }
        Some(value)
            if value.starts_with(CALL_NOTICE_PREFIX) || value.starts_with("managed-call:") =>
        {
            PromptOrigin::cowboy("managed_call")
        }
        Some(value) if value.starts_with(PEER_NOTICE_PREFIX) => {
            PromptOrigin::cowboy("session_notice")
        }
        Some(value) if value.starts_with("cowboy-superagent:") => {
            PromptOrigin::cowboy("superagent")
        }
        Some(value) if value.starts_with("cowboy-code-intelligence:") => {
            PromptOrigin::cowboy("code_intelligence")
        }
        Some(value)
            if value
                .strip_prefix("agent-")
                .is_some_and(|id| id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit())) =>
        {
            PromptOrigin::cowboy("agent_message")
        }
        _ => PromptOrigin::human_composer(),
    }
}

/// Detect the structured review follow-up used by agent-side reviewer/writer
/// loops. Several independent fingerprints are required so a human merely
/// mentioning `review_file` stays a human bubble.
pub fn is_agent_review_task_prompt(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lowered = trimmed.to_ascii_lowercase();
    lowered.contains("the reviewer found issues")
        && lowered.contains("review_file")
        && ((lowered.contains("status: open") && lowered.contains("addressed"))
            || lowered.contains("add a response field")
            || (lowered.contains("wontfix") && lowered.contains("needs-user-input")))
}

pub fn is_internal_runtime_prompt(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if is_agent_review_task_prompt(trimmed) {
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

/// The timeline echo of one block steered into a running turn. Every block
/// carries the origin its message id selects, like any other prompt echo, so
/// a peer's or a notice's later blocks never read as the owner's.
pub fn steering_echo(block: &Value, cmid: &str) -> Value {
    let mut update = serde_json::json!({
        "sessionUpdate": "user_message_chunk", "content": block,
        "_meta": {"cowboyDelivery": "current_turn"},
    });
    apply_prompt_origin(&mut update, &origin_from_cmid(Some(cmid)));
    update
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
    if is_agent_review_task_prompt(text) {
        apply_prompt_origin(update, &PromptOrigin::agent("review", provider_id));
        return;
    }
    if is_internal_runtime_prompt(text) {
        apply_prompt_origin(update, &PromptOrigin::agent("runtime", provider_id));
    }
}

fn image_field<'a>(value: &'a Value, source: Option<&'a Value>, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(Value::as_str) {
            return Some(text);
        }
        if let Some(text) = source
            .and_then(|child| child.get(*key))
            .and_then(Value::as_str)
        {
            return Some(text);
        }
    }
    None
}

fn image_payload(value: &Value) -> (Option<&str>, Option<&str>) {
    let source = value.get("source");
    (
        image_field(value, source, &["data"]),
        image_field(value, source, &["url"]),
    )
}

/// Whether two ACP user-content blocks are the same prompt piece.
///
/// An agent that "accepts" a prompt by echoing it may reserialize the same
/// type/text/url/data. Extra keys must not keep a replay visible. Cowboy's live
/// echo externalizes large images to `/api/artifacts/…` while `last_echoed`
/// still holds the original `data` block; those are one prompt piece.
pub fn user_content_equiv(left: &Value, right: &Value) -> bool {
    let left_type = left.get("type").and_then(Value::as_str);
    if left_type != right.get("type").and_then(Value::as_str) {
        return false;
    }
    match left_type {
        Some("text") => left.get("text") == right.get("text"),
        Some("image") => {
            let (left_data, left_url) = image_payload(left);
            let (right_data, right_url) = image_payload(right);
            if left_data.is_some() && right_data.is_some() {
                return left_data == right_data;
            }
            if left_url.is_some() && right_url.is_some() {
                return left_url == right_url;
            }
            (left_data.is_some() || left_url.is_some())
                && (right_data.is_some() || right_url.is_some())
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

/// Whether a recorded timeline event is a prompt a person sent: a user-role
/// chunk whose client message id and stamped origin are both the owner's.
/// Cowboy completion notices, peer-session messages, scheduled wakeups and
/// agent-side injections are user-role chunks too, but never an owner prompt.
pub fn is_owner_prompt_event(cmid: Option<&str>, update: &Value) -> bool {
    update.get("sessionUpdate").and_then(Value::as_str) == Some("user_message_chunk")
        && origin_from_cmid(cmid).is_human()
        && is_human_prompt_update(update)
}

/// Whether a recorded user-role chunk is affirmatively an owner's: the Hub
/// stamped it with a human origin when it echoed the prompt, and its message
/// id, if it kept one, is not an automation's. A chunk nobody stamped (an ACP
/// client's unmatched text, a steered block of an older worker) and older rows
/// from before the stamp are not, whatever their text; a stamped chunk is the
/// owner's even when it is a Cowboy preamble carried ahead of the owner's text.
pub fn is_owner_chunk(envelope: &crate::model::Envelope) -> bool {
    let crate::agent_model::Event::Update { update } = &envelope.event else {
        return false;
    };
    update.get("sessionUpdate").and_then(Value::as_str) == Some("user_message_chunk")
        && update.get("autoResumed").and_then(Value::as_bool) != Some(true)
        && update
            .pointer("/promptOrigin/actor")
            .and_then(Value::as_str)
            == Some("human")
        && origin_from_cmid(envelope.cmid.as_deref()).is_human()
}

/// The sequence that begins the newest owner turn in `log` (any slice, oldest
/// first): the first chunk of the newest run of consecutive owner chunks, so a
/// send has one anchor however many blocks (a context preamble, images) it
/// has. A run cut by the slice's start is anchored at its first chunk there.
pub fn newest_owner_turn(log: &[crate::model::Envelope]) -> Option<u64> {
    let newest = log.iter().rposition(is_owner_chunk)?;
    let root = log[..newest]
        .iter()
        .rposition(|event| !is_owner_chunk(event))
        .map_or(0, |gap| gap + 1);
    Some(log[root].seq)
}

/// Whether a recorded timeline event is an owner prompt, consulting
/// Controller-recorded `prompt_dispatches` first and falling back to persisted
/// echo labels only for historical pre-C-auth rows without a dispatch record.
pub fn is_owner_prompt_event_with_dispatches<'a, F>(
    cmid: Option<&str>,
    update: &Value,
    lookup: F,
) -> bool
where
    F: Fn(&str) -> Option<&'a DeliveryOrigin>,
{
    if update.get("sessionUpdate").and_then(Value::as_str) != Some("user_message_chunk") {
        return false;
    }
    if update
        .pointer("/promptOrigin/actor")
        .and_then(Value::as_str)
        == Some("agent")
    {
        return false;
    }
    if let Some(origin) = cmid.and_then(lookup) {
        return origin.is_owner()
            && !is_internal_runtime_prompt(
                update
                    .pointer("/content/text")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            );
    }
    is_owner_prompt_event(cmid, update)
}

/// Whether `envelope` is a chunk of an owner prompt, consulting durable
/// `prompt_dispatches` before falling back to persisted pre-C-auth echo labels.
pub fn is_owner_chunk_with_dispatches<'a, F>(envelope: &crate::model::Envelope, lookup: F) -> bool
where
    F: Fn(&str) -> Option<&'a DeliveryOrigin>,
{
    let crate::agent_model::Event::Update { update } = &envelope.event else {
        return false;
    };
    if update.get("sessionUpdate").and_then(Value::as_str) != Some("user_message_chunk") {
        return false;
    }
    if update
        .pointer("/promptOrigin/actor")
        .and_then(Value::as_str)
        == Some("agent")
    {
        return false;
    }
    if let Some(origin) = envelope.cmid.as_deref().and_then(lookup) {
        return origin.is_owner();
    }
    is_owner_chunk(envelope)
}

/// The sequence that begins the newest owner turn in `log`, using durable
/// `prompt_dispatches` when available.
pub fn newest_owner_turn_with_dispatches<'a, F>(
    log: &[crate::model::Envelope],
    lookup: F,
) -> Option<u64>
where
    F: Fn(&str) -> Option<&'a DeliveryOrigin>,
{
    let newest = log
        .iter()
        .rposition(|event| is_owner_chunk_with_dispatches(event, &lookup))?;
    let root = log[..newest]
        .iter()
        .rposition(|event| !is_owner_chunk_with_dispatches(event, &lookup))
        .map_or(0, |gap| gap + 1);
    Some(log[root].seq)
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
    #[test]
    fn managed_call_prompts_do_not_start_a_new_human_budget() {
        let origin = super::origin_from_cmid(Some("managed-call:call-fixture:round-1"));
        assert!(!origin.is_human());
        assert_eq!(origin.source, "managed_call");
    }
    use super::*;

    #[test]
    fn only_a_person_sends_an_owner_prompt() {
        let chunk = |cmid: Option<&str>, text: &str| {
            let mut update = serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": { "type": "text", "text": text }
            });
            apply_prompt_origin(&mut update, &origin_from_cmid(cmid));
            (cmid.map(str::to_owned), update)
        };
        let peer = format!("agent-{}", "a".repeat(64));
        for cmid in [
            Some("call-notice:call-1"),
            Some("managed-call:call-1:round-1"),
            Some("session-notice:peer-1"),
            Some("__wake__s-1"),
            Some("__sched__s-draft"),
            Some("__cont__abc"),
            Some(peer.as_str()),
            Some("cowboy-superagent:op"),
        ] {
            let (cmid, update) = chunk(cmid, "done");
            assert!(
                !is_owner_prompt_event(cmid.as_deref(), &update),
                "{cmid:?} must not be an owner prompt"
            );
        }
        for cmid in [Some("cmid-1"), Some("acp-1-2"), None] {
            let (cmid, update) = chunk(cmid, "please review");
            assert!(is_owner_prompt_event(cmid.as_deref(), &update), "{cmid:?}");
        }
        // The second block of a prompt carries no cmid; its stamped origin
        // keeps a notice's later blocks from reading as the owner's.
        let (_, notice_block) = chunk(Some("call-notice:call-1"), "second block");
        assert!(!is_owner_prompt_event(None, &notice_block));
        // Runtime markup and non-prompt updates are never owner prompts.
        let (cmid, update) = chunk(Some("cmid-1"), "<system-reminder>x</system-reminder>");
        assert!(!is_owner_prompt_event(cmid.as_deref(), &update));
        assert!(!is_owner_prompt_event(
            Some("cmid-1"),
            &serde_json::json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hi"}})
        ));
    }

    fn envelope(seq: u64, cmid: Option<&str>, stamp: bool, text: &str) -> crate::model::Envelope {
        let mut update = serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": { "type": "text", "text": text }
        });
        if stamp {
            apply_prompt_origin(&mut update, &origin_from_cmid(cmid));
        }
        crate::model::Envelope {
            session_id: "s".into(),
            seq,
            event: crate::agent_model::Event::Update { update },
            cmid: cmid.map(str::to_owned),
        }
    }

    fn agent_chunk(seq: u64) -> crate::model::Envelope {
        crate::model::Envelope {
            session_id: "s".into(),
            seq,
            event: crate::agent_model::Event::Update {
                update: serde_json::json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"x"}}),
            },
            cmid: None,
        }
    }

    #[test]
    fn an_owner_turn_is_the_first_chunk_of_the_newest_run_of_owner_chunks() {
        // A send with a Cowboy preamble ahead of the owner's text: the id sits
        // on the preamble block, the text has none, both are stamped.
        let send = [
            envelope(
                10,
                Some("cmid-1"),
                true,
                "<system-reminder>linked context</system-reminder>",
            ),
            envelope(11, None, true, "Continue"),
        ];
        assert_eq!(newest_owner_turn(&send), Some(10));
        // An id longer than the durable bound is dropped on restore; the stamp
        // keeps the send recognizable, and a direct API prompt never had one.
        assert_eq!(
            newest_owner_turn(&[envelope(4, None, true, "direct")]),
            Some(4)
        );
        // Separate sends are separate turns, and a notice after one is not.
        let log = [
            envelope(1, Some("cmid-1"), true, "first"),
            agent_chunk(2),
            envelope(3, Some("cmid-2"), true, "second"),
            envelope(4, Some("call-notice:call-1"), true, "done"),
            agent_chunk(5),
        ];
        assert_eq!(newest_owner_turn(&log), Some(3));
        assert_eq!(newest_owner_turn(&log[..2]), Some(1));
        assert_eq!(newest_owner_turn(&log[3..]), None);
        // Nothing unstamped begins a turn: an ACP client's unmatched text, an
        // older worker's steered block, or a row from before the stamp.
        for unstamped in [
            envelope(7, None, false, "unmatched acp text"),
            envelope(8, Some("cmid-1"), false, "steered by an older worker"),
        ] {
            assert_eq!(newest_owner_turn(&[unstamped]), None);
        }
        // An automation's id never does, stamped or not, and neither does a
        // chunk the Hub flagged as automatic.
        let peer = format!("agent-{}", "f".repeat(64));
        for cmid in [
            "call-notice:call-1",
            "session-notice:p",
            "__wake__s",
            peer.as_str(),
        ] {
            assert_eq!(
                newest_owner_turn(&[envelope(9, Some(cmid), true, "x")]),
                None,
                "{cmid}"
            );
        }
        // Only user-role chunks count.
        assert_eq!(newest_owner_turn(&[agent_chunk(1)]), None);
    }

    #[test]
    fn every_steered_block_carries_the_origin_of_its_message_id() {
        let peer = format!("agent-{}", "d".repeat(64));
        for (cmid, owner) in [
            (peer.as_str(), false),
            ("call-notice:call-1", false),
            ("cmid-1", true),
        ] {
            for text in ["first block", "second block"] {
                let block = serde_json::json!({"type": "text", "text": text});
                let update = steering_echo(&block, cmid);
                assert_eq!(update["_meta"]["cowboyDelivery"], "current_turn");
                // Only the first block carries the id; the echo must not need it.
                assert_eq!(is_owner_prompt_event(None, &update), owner, "{cmid} {text}");
            }
        }
    }

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
            origin_from_cmid(Some("call-notice:call-1")),
            PromptOrigin::cowboy("managed_call")
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
    fn structured_review_follow_up_is_owned_by_the_current_agent() {
        let prompt = "The reviewer found issues. The review_file is at: /tmp/grok-1000/grok-design-review-d58766af.md\n\nRead the review_file. Address ALL issues with Status: open -- including nits.\nThen update the review_file:\n- Status: open -> addressed\n- Add a Response field\nYou may set wontfix or needs-user-input.";
        assert!(is_agent_review_task_prompt(prompt));
        assert!(is_internal_runtime_prompt(prompt));
        assert!(!is_agent_review_task_prompt("Read the review_file please"));
        assert!(!is_agent_review_task_prompt(
            "The reviewer found issues in my PR"
        ));
        let mut update = serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": { "type": "text", "text": prompt }
        });
        annotate_inbound_user_prompt(&mut update, "future-agent");
        assert_eq!(update["promptOrigin"]["actor"], "agent");
        assert_eq!(update["promptOrigin"]["source"], "review");
        assert_eq!(update["promptOrigin"]["provider"], "future-agent");
        assert!(!is_human_prompt_update(&update));
    }

    #[test]
    fn inbound_annotation_stamps_the_current_runtime() {
        let mut update = serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": { "type": "text", "text": "<system-reminder>done</system-reminder>" }
        });
        annotate_inbound_user_prompt(&mut update, "future-agent");
        assert_eq!(update["promptOrigin"]["actor"], "agent");
        assert_eq!(update["promptOrigin"]["source"], "runtime");
        assert_eq!(update["promptOrigin"]["provider"], "future-agent");
        assert_eq!(update["autoResumed"], true);
        assert!(!is_human_prompt_update(&update));
    }

    #[test]
    fn inbound_prompt_echo_consumes_the_cowboy_echo() {
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
    fn inbound_prompt_echo_matches_externalized_image_bytes() {
        let mut remaining = vec![serde_json::json!({
            "type": "image",
            "mimeType": "image/jpeg",
            "data": "c2hvdA=="
        })];
        let artifact = serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": {
                "type": "image",
                "mimeType": "image/jpeg",
                "url": "/api/artifacts/shot.jpg"
            }
        });
        assert!(take_matching_inbound_prompt_echo(&artifact, &mut remaining));
        assert!(remaining.is_empty());
    }

    #[test]
    fn inbound_prompt_echo_matches_nested_image_source() {
        let mut remaining = vec![serde_json::json!({
            "type": "image",
            "source": { "data": "c2hvdA==", "media_type": "image/jpeg" }
        })];
        let flat = serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": { "type": "image", "data": "c2hvdA==", "mimeType": "image/jpeg" }
        });
        assert!(take_matching_inbound_prompt_echo(&flat, &mut remaining));
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

    #[test]
    fn compute_dispatch_origin_truth_table_never_promotes_and_fails_closed() {
        let owner = DeliveryOrigin::Owner;
        let auto = DeliveryOrigin::Automation;
        let peer = DeliveryOrigin::peer("parent-1");
        let sched = DeliveryOrigin::cowboy("schedule");

        assert_eq!(compute_dispatch_origin(Some(&owner), Some(&owner)), owner);
        assert_eq!(compute_dispatch_origin(Some(&owner), Some(&peer)), peer);
        assert_eq!(compute_dispatch_origin(Some(&owner), Some(&auto)), auto);
        assert_eq!(compute_dispatch_origin(Some(&owner), Some(&sched)), sched);

        assert_eq!(compute_dispatch_origin(Some(&peer), Some(&owner)), peer);
        assert_eq!(compute_dispatch_origin(Some(&auto), Some(&owner)), auto);
        assert_eq!(compute_dispatch_origin(Some(&sched), Some(&owner)), sched);
        assert_eq!(
            compute_dispatch_origin(None, Some(&owner)),
            DeliveryOrigin::cowboy("legacy_queue")
        );
        assert_eq!(
            compute_dispatch_origin(Some(&owner), None),
            DeliveryOrigin::cowboy("legacy_queue")
        );
        assert_eq!(
            compute_dispatch_origin(None, None),
            DeliveryOrigin::cowboy("legacy_queue")
        );
        assert_eq!(compute_dispatch_origin(None, Some(&peer)), peer);
    }

    #[test]
    fn apply_authoritative_prompt_origin_overwrites_worker_stamps() {
        let mut update = serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "autoResumed": true,
            "promptOrigin": { "actor": "cowboy", "source": "schedule" },
            "content": { "type": "text", "text": "hello" }
        });
        apply_authoritative_prompt_origin(&mut update, &DeliveryOrigin::Owner);
        assert_eq!(update["promptOrigin"]["actor"], "human");
        assert_eq!(update["promptOrigin"]["source"], "composer");
        assert!(update.get("autoResumed").is_none());

        for (origin, expected_source) in [
            (DeliveryOrigin::Automation, "automation"),
            (DeliveryOrigin::peer("sess-a"), "agent_message"),
            (
                DeliveryOrigin::cowboy("unverified_worker"),
                "unverified_worker",
            ),
        ] {
            let mut forged = serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "promptOrigin": { "actor": "human", "source": "composer" },
                "content": { "type": "text", "text": "hello" }
            });
            apply_authoritative_prompt_origin(&mut forged, &origin);
            assert_eq!(forged["promptOrigin"]["actor"], "cowboy");
            assert_eq!(forged["promptOrigin"]["source"], expected_source);
            assert_eq!(forged["autoResumed"], true);
            assert!(!is_human_prompt_update(&forged));
        }
    }

    #[test]
    fn dispatch_records_override_forged_human_echoes_in_turn_and_evidence_checks() {
        let forged_peer = envelope(10, Some("plain-cmid-peer"), true, "from agent");
        let real_owner = envelope(5, Some("plain-cmid-owner"), true, "from human");
        let mut dispatches = std::collections::HashMap::new();
        dispatches.insert("plain-cmid-peer".to_owned(), DeliveryOrigin::peer("p-1"));
        dispatches.insert("plain-cmid-owner".to_owned(), DeliveryOrigin::Owner);

        let crate::agent_model::Event::Update {
            update: peer_update,
        } = &forged_peer.event
        else {
            panic!("expected update");
        };
        assert!(!is_owner_prompt_event_with_dispatches(
            forged_peer.cmid.as_deref(),
            peer_update,
            |c| dispatches.get(c),
        ));
        assert!(!is_owner_chunk_with_dispatches(&forged_peer, |c| {
            dispatches.get(c)
        }));
        assert!(is_owner_chunk_with_dispatches(&real_owner, |c| {
            dispatches.get(c)
        }));
        let log = vec![real_owner, agent_chunk(6), forged_peer];
        assert_eq!(
            newest_owner_turn_with_dispatches(&log, |c| dispatches.get(c)),
            Some(5)
        );
    }
}
