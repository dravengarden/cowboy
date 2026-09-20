//! The session bootstrap is validated by a digest of its own body, so a reader
//! that reopens an unchanged session revalidates instead of downloading the
//! tail again (docs/offline-first-sync.md §Reopening a session).
//!
//! These tests exist to defend the choice of a digest over a `since_seq`
//! cursor: transcript rows are coalesced in place under an existing seq, so a
//! cursor would report "nothing new" for changes the reader must see.
use super::*;

fn hub_with_session(id: &str) -> Hub {
    let hub = Hub::new();
    hub.create_local_session(
        id.to_owned(),
        "claude-code".to_owned(),
        "/tmp".to_owned(),
        "validator fixture".to_owned(),
        SessionOrigin::Web,
        false,
    );
    hub
}

fn update(hub: &Hub, session: &str, update: serde_json::Value) {
    hub.push(session, Event::Update { update });
}

fn etag_of(hub: &Hub, session: &str) -> String {
    let messages = focused_session_bootstrap(hub, session).expect("session exists");
    bootstrap_etag(&serde_json::to_vec(&SessionBootstrapResponse { messages }).expect("encodable"))
}

fn last_seq(hub: &Hub, session: &str) -> u64 {
    let (events, _) = hub.snapshot(session).expect("session exists");
    events.last().expect("at least one event").seq
}

#[test]
fn an_unchanged_tail_keeps_its_validator_and_a_new_event_changes_it() {
    let hub = hub_with_session("steady");
    update(
        &hub,
        "steady",
        serde_json::json!({"sessionUpdate": "user_message_chunk",
            "content": {"type": "text", "text": "hello"}}),
    );
    let first = etag_of(&hub, "steady");
    // Reading twice cannot change what the reader would receive.
    assert_eq!(first, etag_of(&hub, "steady"));

    update(
        &hub,
        "steady",
        serde_json::json!({"sessionUpdate": "agent_message_chunk",
            "messageId": "m1", "content": {"type": "text", "text": "hi"}}),
    );
    assert_ne!(first, etag_of(&hub, "steady"), "a new row must invalidate");
}

#[test]
fn a_row_coalesced_in_place_invalidates_even_though_the_last_seq_does_not_move() {
    let hub = hub_with_session("coalesced");
    update(
        &hub,
        "coalesced",
        serde_json::json!({"sessionUpdate": "tool_call", "toolCallId": "t1",
            "title": "read file", "status": "pending"}),
    );
    let pending = etag_of(&hub, "coalesced");
    let pending_seq = last_seq(&hub, "coalesced");

    // The reducer merges this into the stored row under its ORIGINAL seq.
    update(
        &hub,
        "coalesced",
        serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1",
            "status": "completed"}),
    );

    assert_eq!(
        pending_seq,
        last_seq(&hub, "coalesced"),
        "the tool call was rewritten in place, so a cursor would see nothing new"
    );
    assert_ne!(
        pending,
        etag_of(&hub, "coalesced"),
        "a digest over the tail still reports the completed tool call"
    );
}

#[test]
fn a_streamed_message_growing_in_place_invalidates() {
    let hub = hub_with_session("streaming");
    update(
        &hub,
        "streaming",
        serde_json::json!({"sessionUpdate": "agent_message_chunk", "messageId": "m1",
            "content": {"type": "text", "text": "The ans"}}),
    );
    let partial = etag_of(&hub, "streaming");
    let partial_seq = last_seq(&hub, "streaming");

    update(
        &hub,
        "streaming",
        serde_json::json!({"sessionUpdate": "agent_message_chunk", "messageId": "m1",
            "content": {"type": "text", "text": "wer is 42."}}),
    );

    assert_eq!(partial_seq, last_seq(&hub, "streaming"));
    assert_ne!(partial, etag_of(&hub, "streaming"));
}

#[test]
fn only_an_exact_token_revalidates() {
    let etag = "\"bootstrap-v1-abc\"";
    assert!(if_none_match_has(Some(etag), etag));
    assert!(if_none_match_has(
        Some("\"other\", \"bootstrap-v1-abc\""),
        etag
    ));
    assert!(if_none_match_has(Some("W/\"bootstrap-v1-abc\""), etag));
    assert!(!if_none_match_has(None, etag));
    assert!(!if_none_match_has(Some("\"bootstrap-v1-abcd\""), etag));
    // A prefix must not revalidate: the previous implementation of this shape
    // elsewhere in the file uses `contains`, which would accept it.
    assert!(!if_none_match_has(Some("\"bootstrap-v1-ab\""), etag));
    // This representation is private to one reader, so a wildcard would answer
    // 304 for a body the caller has never seen.
    assert!(!if_none_match_has(Some("*"), etag));
}
