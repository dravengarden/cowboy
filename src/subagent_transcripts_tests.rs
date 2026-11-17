use super::*;

fn observed(runtime_seq: u64, update: Value) -> Observed {
    Observed {
        session_id: "sess-1".to_owned(),
        worker_epoch: "epoch-1".to_owned(),
        runtime_seq,
        child_id: "child-a".to_owned(),
        update,
        dropped: 0,
    }
}

fn chunk(message: &str, text: &str) -> Value {
    json!({"sessionUpdate": "agent_message_chunk", "messageId": message,
           "content": {"type": "text", "text": text}})
}

fn fresh() -> ChildState {
    ChildState {
        next_seq: 1,
        ..ChildState::default()
    }
}

#[test]
fn intercept_diverts_only_observed_child_updates() {
    let ordinary = RuntimeEvent::Update {
        update: json!({"sessionUpdate": "agent_message_chunk"}),
        cmid: None,
    };
    assert_eq!(intercept("s", "e", 1, ordinary.clone()), Some(ordinary));
    let child = RuntimeEvent::Update {
        update: json!({"sessionUpdate": SUBAGENT_EVENT_UPDATE, "childSessionId": "child-a",
                       "update": chunk("m", "hi"), "dropped": 2}),
        cmid: None,
    };
    assert_eq!(intercept("s", "e", 1, child), None);
    let parsed = parse(
        "s",
        "e",
        7,
        &json!({"sessionUpdate": SUBAGENT_EVENT_UPDATE, "childSessionId": "child-a",
                "update": chunk("m", "hi"), "dropped": 2}),
    )
    .unwrap();
    assert_eq!((parsed.runtime_seq, parsed.dropped), (7, 2));
    // A malformed one is still kept out of the parent timeline, and dropped.
    assert!(
        parse(
            "s",
            "e",
            1,
            &json!({"sessionUpdate": SUBAGENT_EVENT_UPDATE})
        )
        .is_none()
    );
}

#[test]
fn streamed_chunks_of_one_message_share_a_row() {
    let mut state = fresh();
    let first = fold(&mut state, observed(1, chunk("m1", "Hel")), 10);
    let second = fold(&mut state, observed(2, chunk("m1", "lo")), 11);
    assert_eq!((first[0].seq, second[0].seq), (1, 1));
    assert_eq!(second[0].payload["content"]["text"], "Hello");
    assert_eq!(second[0].runtime_seq, 2);
    let tool = fold(
        &mut state,
        observed(3, json!({"sessionUpdate": "tool_call", "toolCallId": "t"})),
        12,
    );
    assert_eq!(tool[0].seq, 2);
    // A new message after a tool starts its own row.
    let next = fold(&mut state, observed(4, chunk("m1", "again")), 13);
    assert_eq!(next[0].seq, 3);
    let other = fold(&mut state, observed(5, chunk("m2", "x")), 14);
    assert_eq!(other[0].seq, 4);
    assert_eq!(state.rows, 4);
}

#[test]
fn dropped_updates_become_a_visible_gap_row() {
    let mut state = fresh();
    let mut update = observed(1, chunk("m", "after"));
    update.dropped = 3;
    let rows = fold(&mut state, update, 10);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].payload["sessionUpdate"], GAP_UPDATE);
    assert_eq!(rows[0].payload["dropped"], 3);
    assert_eq!(rows[1].seq, 2);
}

#[test]
fn a_child_stops_growing_at_its_row_limit_with_one_final_marker() {
    let mut state = ChildState {
        next_seq: MAX_CHILD_ROWS + 1,
        rows: MAX_CHILD_ROWS,
        ..ChildState::default()
    };
    let rows = fold(&mut state, observed(1, chunk("m", "x")), 10);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].payload["reason"], "limit");
    assert!(state.capped);
    assert!(fold(&mut state, observed(2, chunk("m", "y")), 11).is_empty());
}

#[test]
fn oversized_updates_are_trimmed_but_keep_their_identity() {
    let big = "x".repeat(MAX_ROW_BYTES * 2);
    let tool = bounded(
        json!({"sessionUpdate": "tool_call_update", "toolCallId": "t",
                              "title": "Read", "status": "completed",
                              "content": [{"type": "content", "content": {"type": "text", "text": big}}]}),
    );
    assert!(serde_json::to_vec(&tool).unwrap().len() <= MAX_ROW_BYTES);
    assert_eq!(tool["toolCallId"], "t");
    assert_eq!(tool["status"], "completed");
    assert_eq!(tool["_cowboy"]["truncated"], true);
    let text = bounded(chunk("m", &"é".repeat(MAX_ROW_BYTES)));
    assert!(serde_json::to_vec(&text).unwrap().len() <= MAX_ROW_BYTES);
    assert_eq!(text["messageId"], "m");
    let small = chunk("m", "fine");
    assert_eq!(bounded(small.clone()), small);
}

#[test]
fn a_legacy_controller_gets_an_inert_event_and_huge_labels_are_bounded() {
    let mut event = RuntimeEvent::Update {
        update: json!({"sessionUpdate": SUBAGENT_EVENT_UPDATE, "childSessionId": "c",
                       "update": chunk("m", "x")}),
        cmid: None,
    };
    crate::runtime_wire::retire_observation(&mut event);
    let RuntimeEvent::Update { update, .. } = &event else {
        unreachable!()
    };
    assert_eq!(
        update,
        &json!({"sessionUpdate": crate::runtime_wire::STEERING_RESULT_UPDATE})
    );
    let title = "t".repeat(MAX_ROW_BYTES * 3);
    let bounded = bounded(json!({"sessionUpdate": "tool_call", "toolCallId": "t", "title": title}));
    assert!(serde_json::to_vec(&bounded).unwrap().len() <= MAX_ROW_BYTES);
    assert_eq!(bounded["toolCallId"], "t");
    // A coalesced message never grows past the row budget.
    let open = chunk("m", &"a".repeat(MAX_ROW_BYTES - 200));
    assert!(coalesce(&open, &chunk("m", &"\"".repeat(100))).is_none());
}
