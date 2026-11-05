
use super::*;
use crate::core::Event;

fn update(seq: u64, value: serde_json::Value) -> Envelope {
    Envelope {
        session_id: "sess-test".to_owned(),
        seq,
        event: Event::Update { update: value },
        cmid: None,
    }
}

#[test]
fn reducer_coalesces_text_at_the_first_seq() {
    let mut reducer = EventReducer::default();
    let first = reducer
        .reduce(update(
            10,
            serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "messageId": "m1",
                "content": {"type": "text", "text": "hello "}
            }),
        ))
        .expect("first chunk persists");
    assert_eq!(first.seq, 10);
    let joined = reducer
        .reduce(update(
            11,
            serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "messageId": "m1",
                "content": {"type": "text", "text": "world"}
            }),
        ))
        .expect("second chunk updates canonical row");
    assert_eq!(joined.seq, 10);
    let Event::Update { update } = joined.event else {
        panic!("update")
    };
    assert_eq!(update["content"]["text"], "hello world");
}

#[test]
fn reducer_folds_tool_updates_into_the_original_call() {
    let mut reducer = EventReducer::default();
    reducer.reduce(update(
        20,
        serde_json::json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "tool-1",
            "title": "run",
            "status": "pending"
        }),
    ));
    let folded = reducer
        .reduce(update(
            21,
            serde_json::json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "tool-1",
                "status": "completed",
                "content": [{"type": "text", "text": "ok"}]
            }),
        ))
        .expect("tool update folds");
    assert_eq!(folded.seq, 20);
    let Event::Update { update } = folded.event else {
        panic!("update")
    };
    assert_eq!(update["sessionUpdate"], "tool_call");
    assert_eq!(update["status"], "completed");
    assert_eq!(update["title"], "run");
}

#[test]
fn reducer_drops_transient_frames() {
    let mut reducer = EventReducer::default();
    assert!(
        reducer
            .reduce(update(
                30,
                serde_json::json!({"sessionUpdate": "usage_update", "used": 1, "size": 2}),
            ))
            .is_none()
    );
}
