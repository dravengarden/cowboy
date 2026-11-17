//! Observed subagent transcripts (docs/subagents.md#observed-transcripts):
//! the writer's replay fence and coalescing against a real store, paging,
//! and Clear.

use super::*;
use crate::subagent_transcripts::{Observed, Writer};

fn observed(runtime_seq: u64, update: serde_json::Value) -> Observed {
    Observed {
        session_id: "sess-observed".to_owned(),
        worker_epoch: "epoch-1".to_owned(),
        runtime_seq,
        child_id: "child-a".to_owned(),
        update,
        dropped: 0,
    }
}

fn say(text: &str) -> serde_json::Value {
    serde_json::json!({"sessionUpdate": "agent_message_chunk", "messageId": "m",
                       "content": {"type": "text", "text": text}})
}

fn tool(id: &str) -> serde_json::Value {
    serde_json::json!({"sessionUpdate": "tool_call", "toolCallId": id, "title": id})
}

#[tokio::test]
async fn sqlite_observed_transcript_coalesces_fences_replays_pages_and_clears() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    store
        .insert_session(&session("sess-observed"))
        .await
        .unwrap();

    let mut writer = Writer::default();
    writer
        .apply(
            &store,
            vec![
                observed(1, say("Hel")),
                observed(2, say("lo")),
                observed(3, tool("t1")),
            ],
        )
        .await;
    // A restarted Controller replays what it already stored: nothing changes.
    let mut restarted = Writer::default();
    restarted
        .apply(
            &store,
            vec![
                observed(2, say("lo")),
                observed(3, tool("t1")),
                observed(4, tool("t2")),
            ],
        )
        .await;

    let page = store
        .subagent_events_page("sess-observed", "child-a", None, None, 10)
        .await
        .unwrap();
    let kinds: Vec<_> = page
        .rows
        .iter()
        .map(|(seq, _, payload)| (*seq, payload["sessionUpdate"].as_str().unwrap().to_owned()))
        .collect();
    assert_eq!(
        kinds,
        [
            (1, "agent_message_chunk".to_owned()),
            (2, "tool_call".to_owned()),
            (3, "tool_call".to_owned())
        ]
    );
    assert_eq!(page.rows[0].2["content"]["text"], "Hello");
    assert!(!page.more_before);

    let newest = store
        .subagent_events_page("sess-observed", "child-a", None, None, 2)
        .await
        .unwrap();
    assert_eq!(
        newest.rows.iter().map(|row| row.0).collect::<Vec<_>>(),
        [2, 3]
    );
    assert!(newest.more_before);
    let older = store
        .subagent_events_page("sess-observed", "child-a", None, Some(2), 2)
        .await
        .unwrap();
    assert_eq!(older.rows.iter().map(|row| row.0).collect::<Vec<_>>(), [1]);
    let after = store
        .subagent_events_page("sess-observed", "child-a", Some(1), None, 10)
        .await
        .unwrap();
    assert_eq!(
        after.rows.iter().map(|row| row.0).collect::<Vec<_>>(),
        [2, 3]
    );

    // Rows of an unknown session are skipped, not an error.
    let mut stray = observed(9, tool("x"));
    stray.session_id = "sess-missing".to_owned();
    writer.apply(&store, vec![stray]).await;
    assert!(
        store
            .subagent_events_page("sess-missing", "child-a", None, None, 10)
            .await
            .unwrap()
            .rows
            .is_empty()
    );

    store.clear_events("sess-observed", 0).await.unwrap();
    assert!(
        store
            .subagent_events_page("sess-observed", "child-a", None, None, 10)
            .await
            .unwrap()
            .rows
            .is_empty()
    );
}
