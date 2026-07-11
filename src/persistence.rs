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
    /// Convert the high-frequency ACP event stream into stable history rows.
    /// Live WS clients still receive every raw event; only durable history is
    /// reduced. A returned envelope may reuse an earlier seq, causing an UPSERT.
    pub(crate) fn reduce(&mut self, env: Envelope) -> Option<Envelope> {
        let sid = env.session_id.clone();
        let Event::Update { update } = &env.event else {
            self.text.remove(&sid);
            if matches!(env.event, Event::TurnEnd { .. }) {
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
                if let Some(slot) = self.text.get_mut(&sid) {
                    if slot.kind == kind && slot.message_id == message_id {
                        if let Event::Update { update } = &mut slot.envelope.event {
                            if let Some(value) = update
                                .get_mut("content")
                                .and_then(|content| content.get_mut("text"))
                            {
                                let mut joined = value.as_str().unwrap_or_default().to_owned();
                                joined.push_str(chunk);
                                *value = serde_json::Value::String(joined);
                                return Some(slot.envelope.clone());
                            }
                        }
                    }
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
                        {
                            if let (Some(base_object), Some(delta)) =
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
