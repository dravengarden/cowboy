use std::collections::HashMap;

use super::*;
use crate::agent_model::Event;
use crate::model::Envelope;
use crate::persistence::EventReducer;
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
        hop_order: 1,
        parent_session_id: parent.to_owned(),
        parent_is_root: false,
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
#[allow(clippy::too_many_lines)]
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

    let mut e_ba = make_edge("sess-b", "sess-a", "native_fork", 2);
    e_ba.parent_is_root = true;
    store.upsert_lineage_edge(&e_ba).await.unwrap();
    store
        .upsert_lineage_edge(&make_edge("sess-c", "sess-b", "replace", 1))
        .await
        .unwrap();

    // Soft-delete A and B immediately seals segments (no 3-day gap!).
    store.delete_session("sess-a").await.unwrap();
    store.delete_session("sess-b").await.unwrap();
    let pre_purge_display = store.child_display_lineage("sess-c").await.unwrap();
    assert_eq!(pre_purge_display.hops.len(), 2);
    assert!(pre_purge_display.hops[0].segment_id.is_some());
    assert!(pre_purge_display.hops[1].segment_id.is_some());

    assert_eq!(store.purge_deleted(-1).await.unwrap(), 2);
    assert!(
        store.session_auth_generations().await.unwrap().is_empty(),
        "archived lineage must not pin retired auth generations"
    );

    // C has both hops (B < 1, A < 2) sealed and readable through checked snapshot API.
    let display = store.child_display_lineage("sess-c").await.unwrap();
    assert_eq!(display.hops.len(), 2);
    let hop_b = &display.hops[0];
    let hop_a = &display.hops[1];
    assert_eq!(hop_b.parent_session_id, "sess-b");
    assert_eq!(hop_a.parent_session_id, "sess-a");
    let seg_b = hop_b.segment_id.clone().expect("seg B");
    let seg_a = hop_a.segment_id.clone().expect("seg A");

    let LineageHistoryOutcome::Page(page_b) = store
        .lineage_history_page_checked("sess-c", "sess-b", None, Some(hop_b.edge_version), 100, 50)
        .await
        .unwrap()
    else {
        panic!("expected page B");
    };
    assert!(page_b.is_sealed);
    assert_eq!(page_b.events.len(), 1);
    assert_eq!(page_b.events[0].seq, 0);

    // Stale edge_version returns VersionConflict atomically.
    assert!(matches!(
        store
            .lineage_history_page_checked(
                "sess-c",
                "sess-b",
                None,
                Some(hop_b.edge_version + 9),
                100,
                50
            )
            .await
            .unwrap(),
        LineageHistoryOutcome::VersionConflict { .. }
    ));

    let LineageHistoryOutcome::Page(page_a) = store
        .lineage_history_page_checked("sess-c", "sess-a", None, Some(hop_a.edge_version), 100, 50)
        .await
        .unwrap()
    else {
        panic!("expected page A");
    };
    assert!(page_a.is_sealed);
    assert_eq!(page_a.events.len(), 2);

    // Clearing C drops display references AND immediately GCs unreachable segments
    // even while B->A auth edge remains retained for Tools ancestry (H10).
    store
        .clear_conversation("sess-c", 0, None, None)
        .await
        .unwrap();
    let _ = store.purge_deleted(-1).await.unwrap();
    let after_clear = store.child_display_lineage("sess-c").await.unwrap();
    assert!(after_clear.has_durable_edge);
    assert!(after_clear.hops.is_empty());
    let auth_ba = store
        .lineage_edge("sess-b")
        .await
        .unwrap()
        .expect("auth edge B->A retained");
    assert!(auth_ba.parent_is_root);
    assert_eq!(
        auth_ba.segment_id, None,
        "auth-only edge must not hold segment"
    );
    assert!(
        store.lineage_segment(&seg_b).await.unwrap().is_none(),
        "seg B GC'd"
    );
    assert!(
        store.lineage_segment(&seg_a).await.unwrap().is_none(),
        "seg A GC'd"
    );

    // Once C is purged too, auth-only edges are GC'd.
    store.delete_session("sess-c").await.unwrap();
    assert_eq!(store.purge_deleted(-1).await.unwrap(), 1);
    assert!(store.lineage_edge("sess-c").await.unwrap().is_none());
    assert!(store.lineage_edge("sess-b").await.unwrap().is_none());
}

#[tokio::test]
async fn clearing_intermediate_branch_b_preserves_inherited_a_and_b_history_for_c() {
    let (store, _dir) = test_store().await;
    store.insert_session(&session("a")).await.unwrap();
    store.insert_session(&session("b")).await.unwrap();
    store.insert_session(&session("c")).await.unwrap();
    store
        .upsert_event_batch(
            &[msg("a", 0, "a0"), msg("b", 0, "b0")],
            &HashMap::from([("a".to_owned(), 1), ("b".to_owned(), 1)]),
        )
        .await
        .unwrap();
    store
        .upsert_lineage_edge(&make_edge("b", "a", "native_fork", 1))
        .await
        .unwrap();
    store
        .upsert_lineage_edge(&make_edge("c", "b", "linked_fork", 1))
        .await
        .unwrap();

    // Clear intermediate B: B detaches from A, B's prefix archives for C,
    // AND C retains its materialized hop to A!
    store.clear_conversation("b", 0, None, None).await.unwrap();

    let b_display = store.child_display_lineage("b").await.unwrap();
    assert!(b_display.has_durable_edge);
    assert!(b_display.hops.is_empty(), "B's own view is detached");

    let c_display = store.child_display_lineage("c").await.unwrap();
    assert_eq!(c_display.hops.len(), 2, "C keeps both B and A hops");
    assert_eq!(c_display.hops[0].parent_session_id, "b");
    assert!(c_display.hops[0].segment_id.is_some());
    assert_eq!(c_display.hops[1].parent_session_id, "a");

    // Even if A is now cleared too, C's materialized hop to A gets archived!
    store.clear_conversation("a", 0, None, None).await.unwrap();
    let c_after_a_clear = store.child_display_lineage("c").await.unwrap();
    assert_eq!(c_after_a_clear.hops.len(), 2);
    assert!(c_after_a_clear.hops[1].segment_id.is_some());
    let page_a = store
        .lineage_history_page(&c_after_a_clear.hops[1], 10, 50)
        .await
        .unwrap();
    assert_eq!(page_a.events.len(), 1);
    assert_eq!(page_a.events[0].seq, 0);
}

#[tokio::test]
async fn live_lineage_page_reports_unsealed_until_parent_next_seq_reaches_cutoff() {
    let (store, _dir) = test_store().await;
    store.insert_session(&session("src")).await.unwrap();
    store.insert_session(&session("dst")).await.unwrap();
    store
        .upsert_event_batch(
            &[msg("src", 0, "s0")],
            &HashMap::from([("src".to_owned(), 1)]),
        )
        .await
        .unwrap();
    let edge = make_edge("dst", "src", "native_fork", 3);
    store.upsert_lineage_edge(&edge).await.unwrap();

    let unsealed = store.lineage_history_page(&edge, 3, 50).await.unwrap();
    assert!(!unsealed.is_sealed);
    assert_eq!(unsealed.events.len(), 1);

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
#[allow(clippy::too_many_lines)]
async fn turn_completions_cold_scan_and_duplicate_receipt_lifecycle() {
    let (store, _dir) = test_store().await;
    store.insert_session(&session("src")).await.unwrap();
    store.insert_session(&session("dst-1")).await.unwrap();

    // Cold-history SQL scan finds pre-migration turn boundary without turn_completions row.
    store
        .upsert_event_batch(
            &[
                Envelope {
                    session_id: "src".to_owned(),
                    seq: 0,
                    event: Event::Update {
                        update: serde_json::json!({
                            "sessionUpdate": "agent_message_chunk",
                            "messageId": "legacy-m1",
                            "content": {"type": "text", "text": "ans1"},
                        }),
                    },
                    cmid: None,
                },
                Envelope {
                    session_id: "src".to_owned(),
                    seq: 1,
                    event: Event::TurnEnd {
                        stop_reason: "end_turn".to_owned(),
                        usage: None,
                    },
                    cmid: None,
                },
            ],
            &HashMap::from([("src".to_owned(), 2)]),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .find_message_turn_before_seq("src", "legacy-m1")
            .await
            .unwrap(),
        Some(2)
    );
    assert_eq!(
        store
            .find_message_turn_before_seq("src", "missing")
            .await
            .unwrap(),
        None
    );

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
    assert_eq!(
        store.latest_turn_completion("src", 0).await.unwrap(),
        Some(("m1".to_owned(), 4))
    );
    assert_eq!(
        store.load_max_turn_completion_floors().await.unwrap(),
        vec![("src".to_owned(), 4)]
    );

    // Durable duplicate receipt reservation -> ready -> deleted on destination delete.
    let receipt = DuplicateReceipt {
        owner_user_id: "u1".to_owned(),
        source_session_id: "src".to_owned(),
        operation_id: "op-1".to_owned(),
        fingerprint: "fp-a".to_owned(),
        destination_session_id: "dst-1".to_owned(),
        state: "reserved".to_owned(),
        created_at_ms: 300,
        updated_at_ms: 300,
    };
    assert_eq!(
        store.reserve_duplicate_receipt(&receipt).await.unwrap(),
        ReserveOutcome::Reserved
    );
    assert!(matches!(
        store.reserve_duplicate_receipt(&receipt).await.unwrap(),
        ReserveOutcome::Existing(ref r) if r.state == "reserved"
    ));
    store
        .complete_duplicate_receipt("u1", "src", "op-1", 400)
        .await
        .unwrap();
    assert_eq!(
        store
            .duplicate_receipt("u1", "src", "op-1")
            .await
            .unwrap()
            .unwrap()
            .state,
        "ready"
    );
    // Deleting the destination automatically transitions its receipt to `deleted`.
    store.delete_session("dst-1").await.unwrap();
    assert_eq!(
        store
            .duplicate_receipt("u1", "src", "op-1")
            .await
            .unwrap()
            .unwrap()
            .state,
        "deleted"
    );
}

#[tokio::test]
async fn lineage_freeze_floor_rebases_subagent_anchor_in_reducer_and_durable_table() {
    let (store, _dir) = test_store().await;
    store.insert_session(&session("s")).await.unwrap();
    let mut reducer = EventReducer::default();
    let sub_running = Envelope {
        session_id: "s".to_owned(),
        seq: 1,
        event: Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "cowboy_subagent",
                "sessionId": "sub-1",
                "state": "running",
            }),
        },
        cmid: None,
    };
    let r1 = reducer.reduce(sub_running).unwrap();
    assert_eq!(r1.seq, 1);
    let end = Envelope {
        session_id: "s".to_owned(),
        seq: 2,
        event: Event::TurnEnd {
            stop_reason: "end_turn".to_owned(),
            usage: None,
        },
        cmid: None,
    };
    let _ = reducer.reduce(end).unwrap();
    // Unforked session preserves single-row subagent semantics; once a fork
    // cutoff pins the prefix at seq 3, subsequent updates rebase past the floor.
    reducer.set_freeze_floor("s", 3);
    assert_eq!(reducer.freeze_floor("s"), 3);

    // Post-TurnEnd subagent completion allocates fresh seq 5 rather than overwriting seq 1.
    let sub_done = Envelope {
        session_id: "s".to_owned(),
        seq: 5,
        event: Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "cowboy_subagent",
                "sessionId": "sub-1",
                "state": "idle",
            }),
        },
        cmid: None,
    };
    let r2 = reducer.reduce(sub_done).unwrap();
    assert_eq!(r2.seq, 5, "frozen turn-1 subagent row preserved intact");

    // Durable anchor rebase moves anchor from 1 -> 5 in storage.
    let key = crate::transcript_anchors::anchor_key(match &r1.event {
        Event::Update { update } => update,
        _ => unreachable!(),
    })
    .unwrap();
    store
        .upsert_event_batch_with_anchors(
            &[r1],
            &HashMap::from([("s".to_owned(), 2)]),
            &[crate::transcript_anchors::AnchorWrite {
                session_id: "s".to_owned(),
                key: key.clone(),
                seq: 1,
                rebase: false,
            }],
            &[],
        )
        .await
        .unwrap();
    assert_eq!(store.transcript_anchor("s", &key).await.unwrap(), Some(1));
    store
        .upsert_event_batch_with_anchors(
            &[r2],
            &HashMap::from([("s".to_owned(), 6)]),
            &[crate::transcript_anchors::AnchorWrite {
                session_id: "s".to_owned(),
                key: key.clone(),
                seq: 5,
                rebase: true,
            }],
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        store.transcript_anchor("s", &key).await.unwrap(),
        Some(5),
        "rebased anchor points at post-floor row"
    );
}

#[tokio::test]
async fn lineage_question_pages_paginate_in_sql_and_survive_parent_purge() {
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
                user_msg(2, "Second question"),
                msg("parent-q", 3, "Second answer"),
                user_msg(4, "Third question after fork"),
                msg("parent-q", 5, "Third answer"),
            ],
            &HashMap::from([("parent-q".to_owned(), 6)]),
        )
        .await
        .unwrap();
    let edge = make_edge("child-q", "parent-q", "linked_fork", 4);
    store.upsert_lineage_edge(&edge).await.unwrap();

    store.delete_session("parent-q").await.unwrap();
    assert_eq!(store.purge_deleted(-1).await.unwrap(), 1);
    let archived_edge = store.lineage_edge("child-q").await.unwrap().unwrap();
    assert!(archived_edge.segment_id.is_some());

    // Paginate with limit=1 to verify exact ordinals, total=2, and cursor paging.
    let (page1, next1, total1) = store
        .lineage_question_pages(&archived_edge, None, 1)
        .await
        .unwrap();
    assert_eq!(total1, 2);
    assert_eq!(page1.len(), 1);
    assert_eq!(page1[0].id, 2);
    assert_eq!(page1[0].ordinal, 2);
    assert_eq!(next1, Some(2));

    let (page2, next2, total2) = store
        .lineage_question_pages(&archived_edge, next1, 1)
        .await
        .unwrap();
    assert_eq!(total2, 2);
    assert_eq!(page2.len(), 1);
    assert_eq!(page2[0].id, 0);
    assert_eq!(page2[0].ordinal, 1);
    assert_eq!(next2, None);

    let detail = store
        .lineage_question_page_at(&archived_edge, 0)
        .await
        .unwrap()
        .expect("page 0");
    assert_eq!(detail.len(), 2);
    assert!(
        store
            .lineage_question_page_at(&archived_edge, 4)
            .await
            .unwrap()
            .is_none(),
        "post-cutoff question page must be refused"
    );
}
