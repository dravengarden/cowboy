//! Observed subagent transcripts (docs/subagents.md#observed-transcripts):
//! the writer's replay fence, coalescing and Clear barrier against a real
//! store of each backend, paging, and NUL-safe JSON.

use super::*;
use crate::subagent_transcripts::{Msg, Observed, Writer};

fn observed(session: &str, runtime_seq: u64, update: serde_json::Value) -> Msg {
    Msg::Observed(Observed {
        session_id: session.to_owned(),
        worker_epoch: "epoch-1".to_owned(),
        runtime_seq,
        child_id: "child-a".to_owned(),
        update,
        dropped: 0,
    })
}

fn say(text: &str) -> serde_json::Value {
    serde_json::json!({"sessionUpdate": "agent_message_chunk", "messageId": "m",
                       "content": {"type": "text", "text": text}})
}

fn tool(id: &str) -> serde_json::Value {
    serde_json::json!({"sessionUpdate": "tool_call", "toolCallId": id, "title": id})
}

async fn seqs(
    store: &Store,
    id: &str,
    after: Option<i64>,
    before: Option<i64>,
    limit: usize,
) -> Vec<i64> {
    store
        .subagent_events_page(id, "child-a", after, before, limit)
        .await
        .unwrap()
        .rows
        .iter()
        .map(|row| row.0)
        .collect()
}

async fn transcript_contract(store: &Store, id: &str) {
    store.insert_session(&session(id)).await.unwrap();
    let mut writer = Writer::default();
    writer
        .apply(
            store,
            vec![
                observed(id, 1, say("Hel")),
                observed(id, 2, say("lo")),
                observed(id, 3, tool("t1")),
            ],
        )
        .await;
    // A restarted Controller replays what it already stored: nothing changes.
    Writer::default()
        .apply(
            store,
            vec![
                observed(id, 2, say("lo")),
                observed(id, 3, tool("t1")),
                observed(id, 4, tool("t2")),
            ],
        )
        .await;
    let page = store
        .subagent_events_page(id, "child-a", None, None, 10)
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
        .subagent_events_page(id, "child-a", None, None, 2)
        .await
        .unwrap();
    assert_eq!(
        newest.rows.iter().map(|row| row.0).collect::<Vec<_>>(),
        [2, 3]
    );
    assert!(newest.more_before);
    assert_eq!(seqs(store, id, None, Some(2), 2).await, [1]);
    let forward = store
        .subagent_events_page(id, "child-a", Some(0), None, 2)
        .await
        .unwrap();
    assert_eq!(
        forward.rows.iter().map(|row| row.0).collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(
        forward.more_after,
        "a forward page at its limit says more follow"
    );
    assert_eq!(seqs(store, id, Some(1), None, 10).await, [2, 3]);

    // Rows of an unknown session are skipped, not an error.
    writer
        .apply(store, vec![observed("sess-missing", 9, tool("x"))])
        .await;
    assert!(seqs(store, "sess-missing", None, None, 10).await.is_empty());

    store.clear_events(id, 0).await.unwrap();
    assert!(seqs(store, id, None, None, 10).await.is_empty());
}

async fn clear_and_nul_contract(store: &Store, id: &str) {
    store.insert_session(&session(id)).await.unwrap();
    let mut writer = Writer::default();
    // Queued before the Clear barrier, written in the same batch: gone after.
    writer
        .apply(
            store,
            vec![
                observed(id, 1, say("before")),
                Msg::Clear(id.to_owned()),
                observed(id, 2, tool("after\u{0}clear")),
            ],
        )
        .await;
    let page = store
        .subagent_events_page(id, "child-a", None, None, 10)
        .await
        .unwrap();
    assert_eq!(page.rows.len(), 1);
    // The cached sequence was forgotten with the Clear: numbering restarts.
    assert_eq!(page.rows[0].0, 1);
    assert_eq!(page.rows[0].2["toolCallId"], "afterclear");
}

#[tokio::test]
async fn sqlite_observed_transcripts_follow_their_storage_contract() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    transcript_contract(&store, "sess-observed").await;
    clear_and_nul_contract(&store, "sess-observed-clear").await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_observed_transcripts_follow_their_storage_contract() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
        .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    transcript_contract(&store, "sess-observed-pg").await;
    clear_and_nul_contract(&store, "sess-observed-clear-pg").await;
}
