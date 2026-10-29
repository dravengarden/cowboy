//! Canonicalization for the durable event stream.

use std::collections::HashMap;

use crate::core::{Envelope, Event};

/// Durable key carrying the originating client's message id beside the event.
/// `Event` is internally tagged and ignores unknown keys, so a Controller that
/// predates this field still reads every row.
const PERSISTED_CMID_KEY: &str = "cmid";
const PERSISTED_CMID_MAX_BYTES: usize = 128;

/// Serialize an envelope's event for durable storage, keeping the `cmid` of a
/// client-submitted prompt. A reconnecting client resends every unconfirmed
/// submit with the same id; only a persisted echo lets the restored Hub (and a
/// reloaded client reading its snapshot) recognise that the prompt already ran.
pub fn persisted_event_payload(envelope: &Envelope) -> serde_json::Result<serde_json::Value> {
    let mut payload = serde_json::to_value(&envelope.event)?;
    if let (Some(cmid), Some(object)) = (envelope.cmid.as_deref(), payload.as_object_mut())
        && !cmid.is_empty()
        && cmid.len() <= PERSISTED_CMID_MAX_BYTES
    {
        object.insert(
            PERSISTED_CMID_KEY.to_owned(),
            serde_json::Value::String(cmid.to_owned()),
        );
    }
    Ok(payload)
}

/// Remove and return the persisted `cmid`, leaving the plain event payload.
pub fn take_persisted_cmid(payload: &mut serde_json::Value) -> Option<String> {
    match payload.as_object_mut()?.remove(PERSISTED_CMID_KEY)? {
        serde_json::Value::String(cmid)
            if !cmid.is_empty() && cmid.len() <= PERSISTED_CMID_MAX_BYTES =>
        {
            Some(cmid)
        }
        _ => None,
    }
}

#[derive(Clone)]
struct TextSlot {
    kind: String,
    message_id: Option<String>,
    envelope: Envelope,
}

#[derive(Default)]
pub struct EventReducer {
    text: HashMap<String, TextSlot>,
    tools: HashMap<(String, String), Envelope>,
}

impl EventReducer {
    pub fn clear_session(&mut self, session_id: &str) {
        self.text.remove(session_id);
        self.tools.retain(|(session, _), _| session != session_id);
        self.shrink_tools_if_sparse();
    }

    /// Convert the high-frequency ACP event stream into stable history rows.
    /// Live WS clients receive compact deltas (duplicate `rawOutput` already
    /// stripped at ingest). Durable history and the Hub hot tail use this
    /// canonical form. A returned envelope may reuse an earlier seq, causing an
    /// UPSERT or in-memory replacement.
    pub fn reduce(&mut self, mut env: Envelope) -> Option<Envelope> {
        // Also normalize replayed/raw completions before merging them. Otherwise
        // the merge would discard a raw-only result before it can be projected.
        compact_inbound_event(&mut env.event);
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
        // A compaction summary streams like a message; its compaction identity
        // takes the place of the message identity when coalescing.
        if matches!(
            kind,
            Some("agent_message_chunk" | "agent_thought_chunk" | "compaction_summary_chunk")
        ) {
            let text = update
                .get("content")
                .and_then(|content| content.get("text"))
                .and_then(serde_json::Value::as_str);
            let message_id = update
                .get(if kind == Some("compaction_summary_chunk") {
                    "compactionId"
                } else {
                    "messageId"
                })
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
                            for (name, value) in delta {
                                if name != "sessionUpdate" && name != "rawOutput" {
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
pub fn compact_inbound_event(event: &mut Event) {
    match event {
        Event::Update { update } => compact_tool_update(update),
        Event::PermissionRequest { tool_call, .. } => compact_tool_update(tool_call),
        _ => {}
    }
}

/// Durable history and the Hub hot tail only need the fields the Cowboy client
/// can render. ACP adapters commonly duplicate multi-megabyte command/image
/// results in `rawOutput` even when `content` already carries the presentation.
/// A raw-only result is not a duplicate. Preserve it once in `content` for both
/// live subscribers and history. In particular Codex MCP reports the complete
/// `{result, error}` envelope only in `rawOutput`, without any ACP content.
pub fn compact_canonical_tool_output(envelope: &mut Envelope) {
    compact_inbound_event(&mut envelope.event);
}

pub fn compact_tool_update(update: &mut serde_json::Value) {
    crate::managed_calls::media::compact_metadata(update);
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
    let Some(raw_output) = update.remove("rawOutput") else {
        return;
    };
    let has_content = update.get("content").is_some_and(|content| match content {
        serde_json::Value::Null => false,
        serde_json::Value::Array(blocks) => !blocks.is_empty(),
        _ => true,
    });
    if has_content {
        return;
    }
    let content = match raw_output {
        serde_json::Value::Object(mut raw)
            if raw
                .get("formatted_output")
                .is_some_and(serde_json::Value::is_string) =>
        {
            serde_json::json!([{
                "type": "raw_output", "text": raw.remove("formatted_output"),
            }])
        }
        raw => serde_json::json!([{ "type": "tool_result", "value": raw }]),
    };
    update.insert("content".to_owned(), content);
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
    fn persisted_payload_keeps_a_client_cmid_and_reads_legacy_rows() {
        let mut envelope = update(1, "user_message_chunk", "");
        envelope.cmid = Some("c-1".to_owned());
        let mut payload = persisted_event_payload(&envelope).expect("serialize");
        assert_eq!(payload["cmid"], "c-1");
        assert_eq!(take_persisted_cmid(&mut payload).as_deref(), Some("c-1"));
        assert!(payload.get("cmid").is_none());
        let event: Event = serde_json::from_value(payload).expect("plain event");
        assert!(matches!(event, Event::Update { .. }));

        for cmid in [None, Some(String::new()), Some("x".repeat(129))] {
            envelope.cmid = cmid;
            let payload = persisted_event_payload(&envelope).expect("serialize");
            assert!(
                payload.get("cmid").is_none(),
                "only a bounded client id is persisted"
            );
        }
        let mut legacy = serde_json::to_value(&envelope.event).expect("legacy row");
        assert!(take_persisted_cmid(&mut legacy).is_none());
        assert!(take_persisted_cmid(&mut serde_json::Value::Null).is_none());
    }

    #[test]
    fn compaction_summary_chunks_coalesce_per_compaction() {
        let summary = |seq, id: &str, text: &str| Envelope {
            session_id: "session".to_owned(),
            seq,
            event: Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "compaction_summary_chunk",
                    "compactionId": id,
                    "content": { "type": "text", "text": text },
                }),
            },
            cmid: None,
        };
        let text = |envelope: &Envelope| match &envelope.event {
            Event::Update { update } => update
                .pointer("/content/text")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            _ => None,
        };
        let mut reducer = EventReducer::default();
        let first = reducer.reduce(summary(1, "c-1", "Kept ")).expect("first");
        assert_eq!(first.seq, 1);
        let joined = reducer
            .reduce(summary(2, "c-1", "context."))
            .expect("joined");
        assert_eq!(joined.seq, 1);
        assert_eq!(text(&joined).as_deref(), Some("Kept context."));
        let next = reducer.reduce(summary(3, "c-2", "Later.")).expect("next");
        assert_eq!(next.seq, 3);
        assert_eq!(text(&next).as_deref(), Some("Later."));
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
                        usage: None,
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

    #[test]
    fn mcp_results_survive_ingest_reduction_and_replay_once() {
        // The pinned Codex ACP reports MCP completions only as rawOutput.
        // Other adapters use plain text, arrays, structured values or errors.
        for raw in [
            serde_json::json!({"result":{"content":[{"type":"text","text":"help calls"}],
                "structuredContent":{"api":"cowboy-ts-v1"}},"error":null}),
            serde_json::json!({"result":null,"error":{"message":"unavailable","code":42}}),
            serde_json::json!({"content":[],"structuredContent":{"hits":[]}}),
            serde_json::json!("plain result"),
            serde_json::json!([]),
            serde_json::json!(false),
            serde_json::json!(0),
            serde_json::Value::Null,
        ] {
            for already_compacted in [false, true] {
                let mut reducer = EventReducer::default();
                reducer.reduce(update(1, "tool_call", "mcp"));
                let mut completion = update(2, "tool_call_update", "mcp");
                let Event::Update { update } = &mut completion.event else {
                    unreachable!();
                };
                update["status"] = serde_json::json!("completed");
                update["content"] = serde_json::json!([]);
                update["rawOutput"] = raw.clone();
                if already_compacted {
                    compact_inbound_event(&mut completion.event);
                }
                let canonical = reducer.reduce(completion).expect("completion");
                assert_eq!(canonical.seq, 1, "keep one canonical tool row");
                let Event::Update { update } = &canonical.event else {
                    unreachable!();
                };
                assert!(update.get("rawOutput").is_none());
                assert_eq!(update["content"][0]["value"], raw);
                assert!(reducer.tools.is_empty());
                let encoded = serde_json::to_vec(&canonical).expect("serialize");
                let replay: Envelope = serde_json::from_slice(&encoded).expect("replay");
                let replay = EventReducer::default()
                    .reduce(replay)
                    .expect("reduce replay");
                assert_eq!(serde_json::to_vec(&replay).unwrap(), encoded);
            }
        }
    }

    #[test]
    fn mcp_image_results_remain_available_for_artifact_externalization() {
        let mut event = Event::Update {
            update: serde_json::json!({"sessionUpdate":"tool_call_update","toolCallId":"image",
                "rawOutput":{"result":{"content":[{"type":"image","mimeType":"image/png",
                    "data":"image-bytes"}]},"error":null}}),
        };
        compact_inbound_event(&mut event);
        let Event::Update { update } = event else {
            unreachable!()
        };
        assert_eq!(
            update["content"][0]["value"]["result"]["content"][0]["data"],
            "image-bytes"
        );
    }
}
