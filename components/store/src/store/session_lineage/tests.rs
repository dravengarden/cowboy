use std::collections::HashMap;

use super::*;
use crate::agent_model::Event;
use crate::model::Envelope;
use crate::store::fixtures::session;

async fn test_store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::connect("sqlite::memory:", dir.path().join("artifacts"))
        .await
        .expect("connect sqlite");
    store.migrate().await.expect("migrate sqlite");
    (store, dir)
}

fn msg(session_id: &str, seq: u64, text: &str) -> Envelope {
    Envelope {
        session_id: session_id.to_owned(),
        seq,
        event: Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": text},
            }),
        },
        cmid: None,
    }
}

fn make_edge(child: &str, parent: &str, kind: &str, before_seq: u64) -> LineageEdge {
    LineageEdge {
        child_session_id: child.to_owned(),
        parent_session_id: parent.to_owned(),
        owner_user_id: String::new(),
        kind: kind.to_owned(),
        before_seq,
        source_title: format!("Title {parent}"),
        source_provider: "claude".to_owned(),
        segment_id: None,
        display_detached: false,
        edge_version: 1,
        created_at_ms: 1_700_000_000_000,
    }
}

#[tokio::test]
async fn transitive_purge_archives_ancestors_without_pinning_auth_generation_and_gcs_on_leaf_purge()
{
    let (store, _dir) = test_store().await;

    let mut a = session("sess-a");
    a.provider_auth_generation = Some(42);
    let mut b = session("sess-b");
    b.provider_auth_generation = Some(43);
    let c = session("sess-c");
    store.insert_session(&a).await.unwrap();
    store.insert_session(&b).await.unwrap();
    store.insert_session(&c).await.unwrap();

    // A has seq 0, 1, 2 (B forks before 2). B has seq 0, 1 (C forks before 1).
    store
        .upsert_event_batch(
            &[
                msg("sess-a", 0, "a0"),
                msg("sess-a", 1, "a1"),
                msg("sess-a", 2, "a2-after-fork"),
                msg("sess-b", 0, "b0"),
                msg("sess-b", 1, "b1-after-fork"),
            ],
            &HashMap::from([("sess-a".to_owned(), 3), ("sess-b".to_owned(), 2)]),
        )
        .await
        .unwrap();

    store
        .upsert_lineage_edge(&make_edge("sess-b", "sess-a", "native_fork", 2))
        .await
        .unwrap();
    store
        .upsert_lineage_edge(&make_edge("sess-c", "sess-b", "replace", 1))
        .await
        .unwrap();

    // Soft-delete A and B so both expire on purge_deleted(-1), while C stays alive.
    store.delete_session("sess-a").await.unwrap();
    store.delete_session("sess-b").await.unwrap();
    assert_eq!(store.purge_deleted(-1).await.unwrap(), 2);

    // Neither A nor B pins provider_auth_generation anymore.
    assert!(
        store.session_auth_generations().await.unwrap().is_empty(),
        "archived lineage must not pin retired auth generations"
    );

    // Both edges survived and now reference immutable archived segments.
    let edge_cb = store
        .lineage_edge("sess-c")
        .await
        .unwrap()
        .expect("edge C->B");
    let edge_ba = store
        .lineage_edge("sess-b")
        .await
        .unwrap()
        .expect("edge B->A");
    assert!(edge_cb.segment_id.is_some());
    assert!(edge_ba.segment_id.is_some());

    // C can read B's prefix (< 1) and A's prefix (< 2), strictly excluding post-fork rows.
    let page_b = store.lineage_history_page(&edge_cb, 100, 50).await.unwrap();
    assert!(page_b.is_sealed);
    assert_eq!(page_b.events.len(), 1);
    assert_eq!(page_b.events[0].seq, 0);

    let page_a = store.lineage_history_page(&edge_ba, 100, 50).await.unwrap();
    assert!(page_a.is_sealed);
    assert_eq!(page_a.events.len(), 2);
    assert_eq!(page_a.events[0].seq, 0);
    assert_eq!(page_a.events[1].seq, 1);

    // Once C is purged too, GC drops both edges and both archived segments.
    store.delete_session("sess-c").await.unwrap();
    assert_eq!(store.purge_deleted(-1).await.unwrap(), 1);
    assert!(store.lineage_edge("sess-c").await.unwrap().is_none());
    assert!(store.lineage_edge("sess-b").await.unwrap().is_none());
    assert!(
        store
            .lineage_segment(edge_cb.segment_id.as_deref().unwrap())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .lineage_segment(edge_ba.segment_id.as_deref().unwrap())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn clear_seals_parent_prefix_and_detaches_child_display_without_dropping_auth_edge() {
    let (store, _dir) = test_store().await;
    store.insert_session(&session("parent")).await.unwrap();
    store.insert_session(&session("child")).await.unwrap();
    store
        .upsert_event_batch(
            &[msg("parent", 0, "p0"), msg("parent", 1, "p1")],
            &HashMap::from([("parent".to_owned(), 2)]),
        )
        .await
        .unwrap();
    store
        .upsert_lineage_edge(&make_edge("child", "parent", "linked_fork", 2))
        .await
        .unwrap();

    // Clearing parent seals `< 2` before deleting parent's events.
    store
        .clear_conversation("parent", 0, None, None)
        .await
        .unwrap();
    let edge = store.lineage_edge("child").await.unwrap().unwrap();
    assert!(edge.segment_id.is_some());
    assert!(!edge.display_detached);
    let page = store.lineage_history_page(&edge, 2, 50).await.unwrap();
    assert!(page.is_sealed);
    assert_eq!(page.events.len(), 2);

    // Clearing child detaches its visible lineage while keeping parent_session_id for auth.
    store
        .clear_conversation("child", 0, None, None)
        .await
        .unwrap();
    let detached = store.lineage_edge("child").await.unwrap().unwrap();
    assert!(detached.display_detached);
    assert_eq!(detached.parent_session_id, "parent");
    assert!(detached.segment_id.is_none());
    assert!(detached.edge_version > edge.edge_version);
}

#[tokio::test]
async fn live_seal_state_respects_write_behind_and_zero_cutoff() {
    let (store, _dir) = test_store().await;
    store.insert_session(&session("src")).await.unwrap();
    store.insert_session(&session("dst")).await.unwrap();

    // Unflushed prefix: store next_seq is 1 while edge.before_seq is 3.
    store
        .upsert_event_batch(
            &[msg("src", 0, "s0")],
            &HashMap::from([("src".to_owned(), 1)]),
        )
        .await
        .unwrap();
    let edge = make_edge("dst", "src", "replace", 3);
    store.upsert_lineage_edge(&edge).await.unwrap();
    let unsealed = store.lineage_history_page(&edge, 3, 50).await.unwrap();
    assert!(
        !unsealed.is_sealed,
        "unflushed write-behind must not mark sealed"
    );

    // Once flushed past before_seq, page becomes sealed.
    store
        .upsert_event_batch(
            &[msg("src", 1, "s1"), msg("src", 2, "s2")],
            &HashMap::from([("src".to_owned(), 3)]),
        )
        .await
        .unwrap();
    let sealed = store.lineage_history_page(&edge, 3, 50).await.unwrap();
    assert!(sealed.is_sealed);
    assert_eq!(sealed.events.len(), 3);

    // Zero cutoff admits 0 events and is sealed.
    let zero_edge = make_edge("dst-zero", "src", "replace", 0);
    let empty = store
        .lineage_history_page(&zero_edge, 10, 50)
        .await
        .unwrap();
    assert!(empty.is_sealed);
    assert!(empty.events.is_empty());
    assert!(empty.reached_start);
}

#[tokio::test]
async fn turn_completions_and_duplicate_receipts_persist_and_clear_cleanly() {
    let (store, _dir) = test_store().await;
    store.insert_session(&session("src")).await.unwrap();

    store
        .record_turn_completion(&TurnCompletion {
            session_id: "src".to_owned(),
            context_generation: 0,
            message_id: "m1".to_owned(),
            before_seq: 4,
            completed_at_ms: 100,
        })
        .await
        .unwrap();
    store
        .record_turn_completion(&TurnCompletion {
            session_id: "src".to_owned(),
            context_generation: 0,
            message_id: "m2".to_owned(),
            before_seq: 9,
            completed_at_ms: 200,
        })
        .await
        .unwrap();
    assert_eq!(
        store.latest_turn_completion("src", 0).await.unwrap(),
        Some(("m2".to_owned(), 9))
    );
    assert_eq!(
        store
            .turn_completion_before_seq("src", 0, "m1")
            .await
            .unwrap(),
        Some(4)
    );

    // Clear wipes turn completions so stale message IDs cannot be forked after Clear.
    store
        .clear_conversation("src", 0, None, None)
        .await
        .unwrap();
    assert_eq!(store.latest_turn_completion("src", 0).await.unwrap(), None);

    // Duplicate receipt is idempotent and rejects conflicting second insert.
    let receipt = DuplicateReceipt {
        owner_user_id: "u1".to_owned(),
        source_session_id: "src".to_owned(),
        operation_id: "op-1".to_owned(),
        fingerprint: "fp-a".to_owned(),
        destination_session_id: "dst-1".to_owned(),
        created_at_ms: 300,
    };
    assert!(store.put_duplicate_receipt(&receipt).await.unwrap());
    assert!(!store.put_duplicate_receipt(&receipt).await.unwrap());
    assert_eq!(
        store.duplicate_receipt("u1", "src", "op-1").await.unwrap(),
        Some(receipt)
    );
    store
        .delete_duplicate_receipt("u1", "src", "op-1")
        .await
        .unwrap();
    assert_eq!(
        store.duplicate_receipt("u1", "src", "op-1").await.unwrap(),
        None
    );
}

#[tokio::test]
async fn lineage_question_pages_clamp_strictly_and_survive_parent_purge() {
    let (store, _dir) = test_store().await;
    store.insert_session(&session("parent-q")).await.unwrap();
    store.insert_session(&session("child-q")).await.unwrap();
    let user_msg = |seq: u64, text: &str| Envelope {
        session_id: "parent-q".to_owned(),
        seq,
        event: Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": {"type": "text", "text": text},
            }),
        },
        cmid: None,
    };
    store
        .upsert_event_batch(
            &[
                user_msg(0, "First question"),
                msg("parent-q", 1, "First answer"),
                user_msg(2, "Second question after fork"),
                msg("parent-q", 3, "Second answer"),
            ],
            &HashMap::from([("parent-q".to_owned(), 4)]),
        )
        .await
        .unwrap();
    let edge = make_edge("child-q", "parent-q", "linked_fork", 2);
    store.upsert_lineage_edge(&edge).await.unwrap();

    // Purge parent -> archives `< 2`.
    store.delete_session("parent-q").await.unwrap();
    assert_eq!(store.purge_deleted(-1).await.unwrap(), 1);
    let archived_edge = store.lineage_edge("child-q").await.unwrap().unwrap();
    assert!(archived_edge.segment_id.is_some());

    let (pages, next_before, total) = store
        .lineage_question_pages(&archived_edge, None, 20)
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].id, 0);
    assert_eq!(next_before, None);

    // Reading page 0 succeeds and includes seq 0..2; reading post-cutoff page 2 returns None.
    let detail = store
        .lineage_question_page_at(&archived_edge, 0)
        .await
        .unwrap()
        .expect("page 0");
    assert_eq!(detail.len(), 2);
    assert!(
        store
            .lineage_question_page_at(&archived_edge, 2)
            .await
            .unwrap()
            .is_none(),
        "post-cutoff question page must be refused"
    );
}
