//! Canonicalization for the durable event stream.

use std::collections::HashMap;

use crate::core::{Envelope, Event};

#[derive(Clone)]
struct TextSlot {
    kind: String,
    message_id: Option<String>,
    envelope: Envelope,
}

#[derive(Default)]
pub(crate) struct EventReducer {
    text: HashMap<String, TextSlot>,
    tools: HashMap<(String, String), Envelope>,
}

impl EventReducer {
    pub(crate) fn clear_session(&mut self, session_id: &str) {
        self.text.remove(session_id);
        self.tools.retain(|(session, _), _| session != session_id);
        self.shrink_tools_if_sparse();
    }

    /// Convert the high-frequency ACP event stream into stable history rows.
    /// Live WS clients receive compact deltas (duplicate `rawOutput` already
    /// stripped at ingest). Durable history and the Hub hot tail use this
    /// canonical form. A returned envelope may reuse an earlier seq, causing an
    /// UPSERT or in-memory replacement.
    pub(crate) fn reduce(&mut self, env: Envelope) -> Option<Envelope> {
        let sid = env.session_id.clone();
        let Event::Update { update } = &env.event else {
            self.text.remove(&sid);
            if matches!(env.event, Event::TurnEnd { .. } | Event::Lifecycle { .. }) {
                self.tools.retain(|(session, _), _| session != &sid);
                self.shrink_tools_if_sparse();
            }
            return Some(env);
        };
        let kind = update
            .get("sessionUpdate")
            .and_then(serde_json::Value::as_str);
        if matches!(kind, Some("usage_update" | "session_info_update")) {
            return None;
        }
        // Codex exposes terminal stdout/stderr as an extension-only delta. It
        // is useful to connected clients while a command is running, but it is
        // not a self-contained transcript row: without the original tool_call
        // it renders nothing. In particular, a background command can keep
        // emitting after TurnEnd has finalized and evicted its tool slot. If we
        // persist those orphan deltas one row at a time, an unbounded `tail -f`
        // eventually pushes every renderable row out of the bounded bootstrap
        // window. Keep the raw broadcast live, advance the durable sequence
        // watermark in the store writer, and omit pure terminal deltas from the
        // canonical transcript. A semantic tool update carrying status/content
        // alongside `_meta` still follows the normal merge path below.
        if kind == Some("tool_call_update") && is_pure_terminal_output_delta(update) {
            return None;
        }
        if matches!(kind, Some("agent_message_chunk" | "agent_thought_chunk")) {
            let text = update
                .get("content")
                .and_then(|content| content.get("text"))
                .and_then(serde_json::Value::as_str);
            let message_id = update
                .get("messageId")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            if let (Some(kind), Some(chunk)) = (kind, text) {
                if let Some(slot) = self.text.get_mut(&sid)
                    && slot.kind == kind
                    && slot.message_id == message_id
                    && let Event::Update { update } = &mut slot.envelope.event
                    && let Some(value) = update
                        .get_mut("content")
                        .and_then(|content| content.get_mut("text"))
                {
                    let mut joined = value.as_str().unwrap_or_default().to_owned();
                    joined.push_str(chunk);
                    *value = serde_json::Value::String(joined);
                    return Some(slot.envelope.clone());
                }
                self.text.insert(
                    sid,
                    TextSlot {
                        kind: kind.to_owned(),
                        message_id,
                        envelope: env.clone(),
                    },
                );
                return Some(env);
            }
        }
        self.text.remove(&sid);
        if matches!(kind, Some("tool_call" | "tool_call_update")) {
            let tool_id = update
                .get("toolCallId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if !tool_id.is_empty() {
                let key = (sid, tool_id);
                if kind == Some("tool_call_update") {
                    let mut terminal = false;
                    let mut canonical = None;
                    if let Some(tool_env) = self.tools.get_mut(&key) {
                        let mut merged = false;
                        if let (
                            Event::Update {
                                update: base_update,
                            },
                            Event::Update { update: delta },
                        ) = (&mut tool_env.event, &env.event)
                            && let (Some(base_object), Some(delta)) =
                                (base_update.as_object_mut(), delta.as_object())
                        {
                            let formatted_fallback = if !base_object.contains_key("content")
                                && !delta.contains_key("content")
                            {
                                delta
                                    .get("rawOutput")
                                    .and_then(serde_json::Value::as_object)
                                    .and_then(|raw| raw.get("formatted_output"))
                                    .and_then(serde_json::Value::as_str)
                                    .map(str::to_owned)
                            } else {
                                None
                            };
                            for (name, value) in delta {
                                if name != "sessionUpdate" && name != "rawOutput" {
                                    base_object.insert(name.clone(), value.clone());
                                }
                            }
                            if let Some(formatted) = formatted_fallback {
                                base_object.insert(
                                    "content".to_owned(),
                                    serde_json::json!([{
                                        "type": "raw_output",
                                        "text": formatted,
                                    }]),
                                );
                            }
                            base_object.insert(
                                "sessionUpdate".to_owned(),
                                serde_json::Value::String("tool_call".to_owned()),
                            );
                            merged = true;
                        }
                        if merged {
                            compact_canonical_tool_output(tool_env);
                            terminal = envelope_tool_is_terminal(tool_env);
                            canonical = Some(tool_env.clone());
                        }
                    }
                    if terminal {
                        self.tools.remove(&key);
                    }
                    if canonical.is_some() {
                        return canonical;
                    }
                } else {
                    let mut canonical = env;
                    compact_canonical_tool_output(&mut canonical);
                    if !envelope_tool_is_terminal(&canonical) {
                        self.tools.insert(key, canonical.clone());
                    }
                    return Some(canonical);
                }
            }
            let mut canonical = env;
            compact_canonical_tool_output(&mut canonical);
            return Some(canonical);
        }
        Some(env)
    }

    /// A long autonomous turn can finish thousands of tools. Their payloads are
    /// removed at the terminal update above; release an unusually large hash
    /// table at the next lifecycle boundary as well instead of pinning its peak
    /// bucket allocation for the rest of the daemon lifetime.
    fn shrink_tools_if_sparse(&mut self) {
        const LARGE_TOOL_TABLE: usize = 256;
        if self.tools.capacity() > LARGE_TOOL_TABLE
            && self.tools.len().saturating_mul(4) < self.tools.capacity()
        {
            self.tools.shrink_to(LARGE_TOOL_TABLE);
        }
    }
}

fn envelope_tool_is_terminal(envelope: &Envelope) -> bool {
    let Event::Update { update } = &envelope.event else {
        return false;
    };
    matches!(
        update.get("status").and_then(serde_json::Value::as_str),
        Some("completed" | "failed")
    )
}

/// Compact a raw ACP event before the Hub clones it into history, the
/// persistence queue, and live fan-out. Token chunks stay deltas; only the
/// duplicated multi-megabyte `rawOutput` object is removed.
pub(crate) fn compact_inbound_event(event: &mut Event) {
    match event {
        Event::Update { update } => compact_tool_update(update),
        Event::PermissionRequest { tool_call, .. } => compact_tool_update(tool_call),
        _ => {}
    }
}

/// Durable history and the Hub hot tail only need the fields the Cowboy client
/// can render. ACP adapters commonly duplicate multi-megabyte command/image
/// results in `rawOutput` even when `content` already carries the presentation.
/// Codex MCP reads are the one supported fallback: when they only expose
/// `rawOutput.formatted_output`, project it into the same `content` shape the
/// frontend already derives for a live frame before dropping it. Live
/// subscribers now receive this compact form too — `derive.ts` already prefers
/// `content` and only reads `formatted_output` as a fallback.
pub(crate) fn compact_canonical_tool_output(envelope: &mut Envelope) {
    compact_inbound_event(&mut envelope.event);
}

pub(crate) fn compact_tool_update(update: &mut serde_json::Value) {
    let Some(update) = update.as_object_mut() else {
        return;
    };
    if !matches!(
        update
            .get("sessionUpdate")
            .and_then(serde_json::Value::as_str),
        Some("tool_call" | "tool_call_update")
    ) {
        return;
    }
    let raw_output = update.remove("rawOutput");
    if !update.contains_key("content")
        && let Some(serde_json::Value::String(formatted)) = raw_output.and_then(|raw| match raw {
            serde_json::Value::Object(mut raw) => raw.remove("formatted_output"),
            _ => None,
        })
    {
        update.insert(
            "content".to_owned(),
            serde_json::json!([{ "type": "raw_output", "text": formatted }]),
        );
    }
}

fn is_pure_terminal_output_delta(update: &serde_json::Value) -> bool {
    let Some(object) = update.as_object() else {
        return false;
    };
    object
        .keys()
        .all(|key| matches!(key.as_str(), "sessionUpdate" | "toolCallId" | "_meta"))
        && update
            .pointer("/_meta/terminal_output_delta/data")
            .is_some_and(serde_json::Value::is_string)
        && update
            .get("_meta")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|meta| meta.keys().all(|key| key == "terminal_output_delta"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(seq: u64, kind: &str, tool_id: &str) -> Envelope {
        Envelope {
            session_id: "session".to_owned(),
            seq,
            event: Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": kind,
                    "toolCallId": tool_id,
                    "title": format!("event-{seq}"),
                }),
            },
            cmid: None,
        }
    }

    #[test]
    fn clear_session_drops_reducer_state_from_the_old_transcript() {
        let mut reducer = EventReducer::default();
        assert!(reducer.reduce(update(1, "tool_call", "tool")).is_some());
        reducer.clear_session("session");

        let fresh = reducer
            .reduce(update(2, "tool_call_update", "tool"))
            .expect("fresh update");
        assert_eq!(fresh.seq, 2);
    }

    #[test]
    fn pure_terminal_output_is_live_only_even_after_turn_end() {
        let mut reducer = EventReducer::default();
        assert!(reducer.reduce(update(1, "tool_call", "tool")).is_some());
        assert!(
            reducer
                .reduce(Envelope {
                    session_id: "session".to_owned(),
                    seq: 2,
                    event: Event::TurnEnd {
                        stop_reason: "end_turn".to_owned(),
                    },
                    cmid: None,
                })
                .is_some()
        );
        let delta = Envelope {
            session_id: "session".to_owned(),
            seq: 3,
            event: Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "tool",
                    "_meta": {
                        "terminal_output_delta": {
                            "terminal_id": "tool",
                            "data": "one more log line\n"
                        }
                    }
                }),
            },
            cmid: None,
        };
        assert!(reducer.reduce(delta).is_none());
    }

    #[test]
    fn semantic_tool_update_with_terminal_metadata_still_persists() {
        let mut reducer = EventReducer::default();
        assert!(reducer.reduce(update(1, "tool_call", "tool")).is_some());
        let completed = Envelope {
            session_id: "session".to_owned(),
            seq: 2,
            event: Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "tool",
                    "status": "completed",
                    "_meta": {
                        "terminal_output_delta": {
                            "terminal_id": "tool",
                            "data": "done\n"
                        }
                    }
                }),
            },
            cmid: None,
        };
        let canonical = reducer.reduce(completed).expect("canonical completion");
        assert!(
            reducer.tools.is_empty(),
            "completed payload must not stay live"
        );
        let Event::Update { update } = canonical.event else {
            panic!("expected update");
        };
        assert_eq!(update["status"], "completed");
    }

    #[test]
    fn canonical_tool_output_drops_duplicate_raw_output() {
        let mut reducer = EventReducer::default();
        let canonical = reducer
            .reduce(Envelope {
                session_id: "session".to_owned(),
                seq: 1,
                event: Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": "image",
                        "status": "completed",
                        "content": [{"type": "content", "content": {"type": "text", "text": "saved"}}],
                        "rawOutput": {"result": "x".repeat(2 * 1024 * 1024)},
                    }),
                },
                cmid: None,
            })
            .expect("canonical tool");
        let Event::Update { update } = canonical.event else {
            panic!("expected update");
        };
        assert!(update.get("rawOutput").is_none());
        assert_eq!(
            update
                .pointer("/content/0/content/text")
                .and_then(serde_json::Value::as_str),
            Some("saved")
        );
    }

    #[test]
    fn canonical_tool_output_preserves_formatted_output_fallback() {
        let mut reducer = EventReducer::default();
        let canonical = reducer
            .reduce(Envelope {
                session_id: "session".to_owned(),
                seq: 1,
                event: Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": "read",
                        "status": "completed",
                        "rawOutput": {"exit_code": 0, "formatted_output": "file bytes"},
                    }),
                },
                cmid: None,
            })
            .expect("canonical tool");
        let Event::Update { update } = canonical.event else {
            panic!("expected update");
        };
        assert!(update.get("rawOutput").is_none());
        assert_eq!(
            update
                .pointer("/content/0/text")
                .and_then(serde_json::Value::as_str),
            Some("file bytes")
        );
    }
}
