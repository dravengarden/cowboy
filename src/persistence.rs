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
    }

    /// Convert the high-frequency ACP event stream into stable history rows.
    /// Live WS clients still receive every raw event; durable history and the
    /// Hub's hot replay tail both use this canonical form. A returned envelope
    /// may reuse an earlier seq, causing an UPSERT or in-memory replacement.
    pub(crate) fn reduce(&mut self, env: Envelope) -> Option<Envelope> {
        let sid = env.session_id.clone();
        let Event::Update { update } = &env.event else {
            self.text.remove(&sid);
            if matches!(env.event, Event::TurnEnd { .. } | Event::Lifecycle { .. }) {
                self.tools.retain(|(session, _), _| session != &sid);
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
                            for (name, value) in delta {
                                if name != "sessionUpdate" {
                                    base_object.insert(name.clone(), value.clone());
                                }
                            }
                            base_object.insert(
                                "sessionUpdate".to_owned(),
                                serde_json::Value::String("tool_call".to_owned()),
                            );
                            merged = true;
                        }
                        if merged {
                            return Some(tool_env.clone());
                        }
                    }
                } else {
                    self.tools.insert(key, env.clone());
                }
            }
        }
        Some(env)
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
        assert!(reducer.reduce(completed).is_some());
    }
}
