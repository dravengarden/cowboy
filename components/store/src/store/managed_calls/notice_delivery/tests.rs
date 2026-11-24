use super::*;
use crate::agent_model::{Event, Status};
use crate::managed_calls::Request;
use crate::managed_calls::lifecycle::{Placement, Record, State};
use crate::model::{Envelope, QueuedMessage};
use serde_json::json;

#[tokio::test]
async fn sqlite_notice_delivery_contract_is_atomic_and_consistent() {
    Box::pin(conformance("sqlite::memory:")).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_notice_delivery_contract_matches_sqlite() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
    Box::pin(conformance(&url)).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_begin_dispatch_waits_for_concurrent_settlement_row_lock() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
    let root = std::env::temp_dir().join(format!("cowboy-pg-notice-lock-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = Store::connect(&url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();
    let parent = format!("pg-lock-parent-{}", uuid::Uuid::new_v4());
    let call_id = format!("pg-lock-call-{}", uuid::Uuid::new_v4());
    let completion_id = format!("{CALL_NOTICE_PREFIX}{call_id}");
    let meta = serde_json::from_value(json!({
        "id": parent,
        "provider": "codex",
        "machine_id": "hawk",
        "cwd": "/workspace",
        "title": "pg-lock",
        "origin": "api",
        "status": "running"
    }))
    .unwrap();
    store.insert_session(&meta).await.unwrap();
    let (rec, req) = make_notify_call(&parent, &call_id, &format!("req-{call_id}"), true);
    store.admit_managed_call(&rec, &req).await.unwrap();
    assert!(
        store
            .advance_managed_call_through(&rec, &complete_steps(&rec, 200))
            .await
            .unwrap()
    );
    let DispatchClaim::Claimed(_) = store
        .begin_call_notice_dispatch(&parent, &completion_id, "cmd-lock-1", 300)
        .await
        .unwrap()
    else {
        panic!("expected initial Claimed");
    };

    // Hold a concurrent PostgreSQL settlement transaction locking the exact row
    // and updating it from `enqueued` -> `accepted` without committing yet.
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let updated = sqlx::query(
        "UPDATE managed_call_notices \
         SET state = 'accepted', echo_seq = 1, updated_at_ms = updated_at_ms + 1 \
         WHERE parent_session_id = $1 AND completion_id = $2",
    )
    .bind(&parent)
    .bind(&completion_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    assert_eq!(updated.rows_affected(), 1);

    // Retry `begin_call_notice_dispatch` with the SAME `command_id`: because it
    // locks the row `FOR UPDATE`, it must block behind the uncommitted settlement
    // transaction rather than returning a stale `Claimed` from the old tuple.
    let store_retry = store.clone();
    let parent_retry = parent.clone();
    let completion_retry = completion_id.clone();
    let mut retry_handle = tokio::spawn(async move {
        store_retry
            .begin_call_notice_dispatch(&parent_retry, &completion_retry, "cmd-lock-1", 305)
            .await
            .unwrap()
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(80), &mut retry_handle)
            .await
            .is_err(),
        "begin_call_notice_dispatch retry must block on PostgreSQL FOR UPDATE row lock"
    );
    tx.commit().await.unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), retry_handle)
        .await
        .expect("retry unblocks after settlement commits")
        .unwrap();
    assert_eq!(
        outcome,
        DispatchClaim::AlreadyHandled(NoticeDeliveryState::Accepted),
        "retry must see the committed Accepted state instead of returning a stale Claimed stamp"
    );

    pool.close().await;
    let _ = std::fs::remove_dir_all(&root);
}

fn make_notify_call(
    parent: &str,
    call_id: &str,
    request_id: &str,
    notify: bool,
) -> (Record, Request) {
    let raw = if notify {
        format!(
            r#"{{"schema":1,"request_id":"{request_id}","purpose":"review","instruction":"check","context":{{"scope":"current-worktree"}},"access":"read-only","conversation":{{"mode":"fresh"}},"labels":{{"notify":"parent"}}}}"#
        )
    } else {
        format!(
            r#"{{"schema":1,"request_id":"{request_id}","purpose":"review","instruction":"check","context":{{"scope":"current-worktree"}},"access":"read-only","conversation":{{"mode":"fresh"}}}}"#
        )
    };
    let request = Request::parse(raw.as_bytes()).unwrap();
    let record = Record {
        schema: 1,
        call_id: call_id.into(),
        request_id: request_id.into(),
        request_digest: request.digest(),
        provider: "codex".into(),
        purpose: request.purpose,
        access: request.access,
        labels: request.labels.clone(),
        placement: Placement {
            service_id: "svc-1".into(),
            parent_session_id: parent.into(),
            parent_revision: "r1".into(),
            machine_id: "hawk".into(),
            workspace_id: "project".into(),
            cwd: "/workspace".into(),
        },
        child_session_id: format!("child-{call_id}"),
        state: State::Queued,
        revision: 1,
        created_at_ms: 100,
        updated_at_ms: 100,
        started_at_ms: None,
        finished_at_ms: None,
        preset: None,
        selection: None,
        configuration: None,
        provider_account_id: None,
        input_revision: None,
        result: None,
        runtime_machine_id: None,
        provider_version: None,
        provider_generation_digest: None,
        child_cursor: None,
        input_files: Vec::new(),
        cancel_requested_at_ms: None,
        error: None,
    };
    (record, request)
}

fn complete_steps(base: &Record, finished_at_ms: i64) -> Vec<Record> {
    let mut starting = base.clone();
    starting.state = State::Starting;
    starting.revision = base.revision + 1;
    starting.updated_at_ms = finished_at_ms - 2;

    let mut running = starting.clone();
    running.state = State::Running;
    running.revision = starting.revision + 1;
    running.started_at_ms = Some(finished_at_ms - 1);
    running.updated_at_ms = finished_at_ms - 1;
    running.input_revision = Some(base.request_digest.clone());

    let mut done = running.clone();
    done.state = State::Completed;
    done.revision = running.revision + 1;
    done.updated_at_ms = finished_at_ms;
    done.finished_at_ms = Some(finished_at_ms);
    done.result = Some(json!({"summary": "done"}));

    vec![starting, running, done]
}

fn user_chunk_envelope(session_id: &str, seq: u64, cmid: Option<&str>) -> Envelope {
    Envelope {
        session_id: session_id.into(),
        seq,
        cmid: cmid.map(str::to_owned),
        event: Event::Update {
            update: json!({
                "sessionUpdate": "user_message_chunk",
                "content": {"type": "text", "text": "hello"}
            }),
        },
    }
}

fn agent_chunk_envelope(session_id: &str, seq: u64) -> Envelope {
    Envelope {
        session_id: session_id.into(),
        seq,
        cmid: None,
        event: Event::Update {
            update: json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "thinking"}
            }),
        },
    }
}

fn turn_end_envelope(session_id: &str, seq: u64, stop_reason: &str) -> Envelope {
    Envelope {
        session_id: session_id.into(),
        seq,
        cmid: None,
        event: Event::TurnEnd {
            stop_reason: stop_reason.into(),
            usage: None,
        },
    }
}

fn lifecycle_envelope(session_id: &str, seq: u64, status: Status) -> Envelope {
    Envelope {
        session_id: session_id.into(),
        seq,
        cmid: None,
        event: Event::Lifecycle {
            status,
            detail: None,
        },
    }
}

#[allow(clippy::too_many_lines)]
async fn conformance(url: &str) {
    let root = std::env::temp_dir().join(format!(
        "cowboy-notice-store-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let store = Store::connect(url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();

    for id in ["parent-a", "parent-b"] {
        let meta = serde_json::from_value(json!({
            "id": id,
            "provider": "codex",
            "machine_id": "hawk",
            "cwd": "/workspace",
            "title": "Parent",
            "origin": "api",
            "status": "running"
        }))
        .unwrap();
        store.insert_session(&meta).await.unwrap();
    }

    // 1. Atomic creation on terminal transition + rollback on CAS conflict.
    let (c1, req1) = make_notify_call("parent-a", "call-1", "req-1", true);
    store.admit_managed_call(&c1, &req1).await.unwrap();
    assert!(
        store
            .call_notice_delivery("parent-a", "call-1")
            .await
            .unwrap()
            .is_none()
    );

    let mut stale = c1.clone();
    stale.revision = 99;
    assert!(
        !store
            .advance_managed_call_through(&stale, &complete_steps(&stale, 200))
            .await
            .unwrap()
    );
    assert!(
        store
            .call_notice_delivery("parent-a", "call-1")
            .await
            .unwrap()
            .is_none(),
        "rolled-back CAS must not create a notice row"
    );
    assert!(
        store
            .advance_managed_call_through(&c1, &complete_steps(&c1, 200))
            .await
            .unwrap()
    );
    let delivery = store
        .call_notice_delivery("parent-a", "call-1")
        .await
        .unwrap()
        .expect("enqueued notice created atomically");
    assert_eq!(delivery.completion_id, "call-notice:call-1");
    assert_eq!(delivery.state, NoticeDeliveryState::Enqueued);

    // Non-notify Call gets no notice row.
    let (c_quiet, req_quiet) = make_notify_call("parent-a", "call-quiet", "req-quiet", false);
    store
        .admit_managed_call(&c_quiet, &req_quiet)
        .await
        .unwrap();
    assert!(
        store
            .advance_managed_call_through(&c_quiet, &complete_steps(&c_quiet, 210))
            .await
            .unwrap()
    );
    assert!(
        store
            .call_notice_delivery("parent-a", "call-quiet")
            .await
            .unwrap()
            .is_none()
    );

    // 2. Parent-scoped dispatch CAS fencing and exact-stamp + command_id release (Findings 1 & 2).
    assert_eq!(
        store
            .begin_call_notice_dispatch("parent-b", "call-notice:call-1", "cmd-b", 299)
            .await
            .unwrap(),
        DispatchClaim::Untracked,
        "another parent must never claim parent-a's notice"
    );
    assert_eq!(
        store
            .begin_call_notice_dispatch("parent-a", "call-notice:call-1", "cmd-1", 300)
            .await
            .unwrap(),
        DispatchClaim::Claimed(300)
    );
    assert_eq!(
        store
            .begin_call_notice_dispatch("parent-a", "call-notice:call-1", "cmd-dup", 301)
            .await
            .unwrap(),
        DispatchClaim::AlreadyHandled(NoticeDeliveryState::Enqueued)
    );
    assert!(
        !store
            .clear_call_notice_dispatching(
                "parent-a",
                "call-notice:call-1",
                Some(999),
                Some("cmd-1"),
                302,
            )
            .await
            .unwrap(),
        "wrong claim stamp must not release active dispatch claim"
    );
    assert!(
        !store
            .clear_call_notice_dispatching(
                "parent-a",
                "call-notice:call-1",
                Some(300),
                Some("wrong-cmd"),
                302,
            )
            .await
            .unwrap(),
        "wrong command_id must not release active dispatch claim"
    );
    assert!(
        store
            .clear_call_notice_dispatching(
                "parent-a",
                "call-notice:call-1",
                Some(300),
                Some("cmd-1"),
                302,
            )
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .begin_call_notice_dispatch("parent-a", "call-notice:call-1", "cmd-2", 303)
            .await
            .unwrap(),
        DispatchClaim::Claimed(303)
    );

    // Wrong parent echo cannot accept parent-a's notice (Finding 1).
    store
        .upsert_event_batch(
            &[user_chunk_envelope(
                "parent-b",
                1,
                Some("call-notice:call-1"),
            )],
            &HashMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .call_notice_delivery("parent-a", "call-1")
            .await
            .unwrap()
            .unwrap()
            .state,
        NoticeDeliveryState::Enqueued
    );

    // 3. Multi-block rollover preamble + untagged notice body + TurnEnd committed atomically (Findings 4 & 5).
    store
        .upsert_event_batch(
            &[
                user_chunk_envelope("parent-a", 1, Some("call-notice:call-1")),
                user_chunk_envelope("parent-a", 2, None),
                agent_chunk_envelope("parent-a", 3),
            ],
            &HashMap::new(),
        )
        .await
        .unwrap();
    let accepted = store
        .call_notice_delivery("parent-a", "call-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(accepted.state, NoticeDeliveryState::Accepted);
    assert_eq!(accepted.echo_seq, Some(1));
    assert_eq!(
        store
            .call_notice_echo_outcome("parent-a", "call-notice:call-1")
            .await
            .unwrap(),
        NoticeEchoEvidence::Echoed {
            echo_seq: 1,
            turn_completed: None,
            superseded_by_user_turn: false,
        },
        "contiguous untagged second block must not supersede its own notice"
    );

    // Conditional unknown CAS refuses stale pre-acceptance snapshot (Finding 3).
    assert!(
        !store
            .mark_call_notice_unknown_if(
                "parent-a",
                "call-notice:call-1",
                NoticeDeliveryState::Enqueued,
                Some(303),
                None,
                "dispatch_unconfirmed",
                None,
                315,
            )
            .await
            .unwrap(),
        "stale Enqueued CAS must never overwrite concurrently Accepted notice"
    );

    store
        .upsert_event_batch(
            &[turn_end_envelope("parent-a", 4, "end_turn")],
            &HashMap::new(),
        )
        .await
        .unwrap();
    let settled = store
        .call_notice_delivery("parent-a", "call-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(settled.state, NoticeDeliveryState::Consumed);
    assert_eq!(settled.reason.as_deref(), Some("turn_completed"));

    // 4. Restart-interrupted worker_stopped promoted by atomic TurnEnd vs blocked by untagged user turn after agent chunk.
    let (c2, req2) = make_notify_call("parent-a", "call-2", "req-2", true);
    store.admit_managed_call(&c2, &req2).await.unwrap();
    assert!(
        store
            .advance_managed_call_through(&c2, &complete_steps(&c2, 330))
            .await
            .unwrap()
    );
    store
        .upsert_event_batch(
            &[
                user_chunk_envelope("parent-a", 5, Some("call-notice:call-2")),
                user_chunk_envelope("parent-a", 6, None),
                lifecycle_envelope("parent-a", 7, Status::Crashed),
            ],
            &HashMap::new(),
        )
        .await
        .unwrap();
    let crashed = store
        .call_notice_delivery("parent-a", "call-2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(crashed.state, NoticeDeliveryState::Unknown);
    assert_eq!(crashed.reason.as_deref(), Some("worker_stopped"));
    // Promotable unknown rows appear in unsettled_call_notices_page (Finding 5).
    let page = store.unsettled_call_notices_page(None, 100).await.unwrap();
    assert!(page.iter().any(|n| n.call_id == "call-2"));

    store
        .upsert_event_batch(
            &[turn_end_envelope("parent-a", 8, "EndTurn")],
            &HashMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .call_notice_delivery("parent-a", "call-2")
            .await
            .unwrap()
            .unwrap()
            .state,
        NoticeDeliveryState::Consumed
    );

    // Untagged user turn AFTER an agent chunk DOES supersede.
    let (c3, req3) = make_notify_call("parent-a", "call-3", "req-3", true);
    store.admit_managed_call(&c3, &req3).await.unwrap();
    assert!(
        store
            .advance_managed_call_through(&c3, &complete_steps(&c3, 340))
            .await
            .unwrap()
    );
    store
        .upsert_event_batch(
            &[
                user_chunk_envelope("parent-a", 9, Some("call-notice:call-3")),
                agent_chunk_envelope("parent-a", 10),
                user_chunk_envelope("parent-a", 11, None),
                turn_end_envelope("parent-a", 12, "end_turn"),
            ],
            &HashMap::new(),
        )
        .await
        .unwrap();
    let superseded = store
        .call_notice_delivery("parent-a", "call-3")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(superseded.state, NoticeDeliveryState::Unknown);
    assert_eq!(
        superseded.reason.as_deref(),
        Some("superseded_by_user_turn")
    );
    assert_eq!(
        store
            .call_notice_echo_outcome("parent-a", "call-notice:call-3")
            .await
            .unwrap(),
        NoticeEchoEvidence::Echoed {
            echo_seq: 9,
            turn_completed: None,
            superseded_by_user_turn: true,
        }
    );

    // 5. Transcript clear preserves active opted-in Calls and unconfirmed terminal Calls (Finding 11),
    //    and succeeds even if a row's request_document contains `\u0000` (R2 Finding 13).
    let (c_active, req_active) = make_notify_call("parent-a", "call-active", "req-active", true);
    store
        .admit_managed_call(&c_active, &req_active)
        .await
        .unwrap();
    let (c_nul, req_nul) = make_notify_call("parent-a", "call-nul", "req-nul", false);
    store.admit_managed_call(&c_nul, &req_nul).await.unwrap();
    assert!(
        store
            .advance_managed_call_through(&c_nul, &complete_steps(&c_nul, 349))
            .await
            .unwrap()
    );
    let nul_json = r#"{"labels":{"notify":"parent"},"raw":"payload with \u0000 nul escape"}"#;
    match &store.backend {
        crate::store::StorageBackend::Postgres(pg) => {
            sqlx::query(
                "UPDATE managed_agent_calls SET request_document = $1 WHERE call_id = 'call-nul'",
            )
            .bind(nul_json)
            .execute(&pg.pool)
            .await
            .unwrap();
            let backfilled: bool = sqlx::query_scalar(
                "SELECT replace(request_document, $nul$\\u0000$nul$, $replacement$\\uFFFD$replacement$)::json #>> '{labels,notify}' = 'parent' \
                 FROM managed_agent_calls WHERE call_id = 'call-nul'",
            )
            .fetch_one(&pg.pool)
            .await
            .unwrap();
            assert!(backfilled);
        }
        crate::store::StorageBackend::Sqlite(sq) => {
            sqlx::query(
                "UPDATE managed_agent_calls SET request_document = ?1 WHERE call_id = 'call-nul'",
            )
            .bind(nul_json)
            .execute(&sq.pool)
            .await
            .unwrap();
        }
    }

    let (c4, req4) = make_notify_call("parent-a", "call-4", "req-4", true);
    store.admit_managed_call(&c4, &req4).await.unwrap();
    assert!(
        store
            .advance_managed_call_through(&c4, &complete_steps(&c4, 350))
            .await
            .unwrap()
    );
    assert!(
        store
            .mark_call_notice_accepted("parent-a", "call-notice:call-4", 13, 351)
            .await
            .unwrap()
    );

    let (c5, req5) = make_notify_call("parent-a", "call-5", "req-5", true);
    store.admit_managed_call(&c5, &req5).await.unwrap();
    assert!(
        store
            .advance_managed_call_through(&c5, &complete_steps(&c5, 352))
            .await
            .unwrap()
    );

    store.clear_events("parent-a", 10_000).await.unwrap();
    let visible: Vec<String> = store
        .managed_calls("parent-a", None)
        .await
        .unwrap()
        .into_iter()
        .map(|record| record.call_id)
        .collect();
    assert!(!visible.contains(&"call-1".to_string()));
    assert!(!visible.contains(&"call-2".to_string()));
    assert!(!visible.contains(&"call-quiet".to_string()));
    assert!(visible.contains(&"call-active".to_string()));
    assert!(visible.contains(&"call-3".to_string()));
    assert!(visible.contains(&"call-4".to_string()));
    assert!(visible.contains(&"call-5".to_string()));
    assert_eq!(
        store
            .call_notice_delivery("parent-a", "call-4")
            .await
            .unwrap()
            .unwrap()
            .reason
            .as_deref(),
        Some("transcript_cleared")
    );

    // Completing call-active after Clear keeps it visible.
    assert!(
        store
            .advance_managed_call_through(&c_active, &complete_steps(&c_active, 360))
            .await
            .unwrap()
    );
    assert!(
        store
            .managed_calls("parent-a", None)
            .await
            .unwrap()
            .iter()
            .any(|record| record.call_id == "call-active")
    );

    // 6. Atomic owner withdrawal & claim release with update_pending_with_notice_ops (R2 #7, #8, #14).
    let first_claim = store
        .begin_call_notice_dispatch("parent-a", "call-notice:call-5", "cmd-5a", 400)
        .await
        .unwrap();
    let DispatchClaim::Claimed(stamp1) = first_claim else {
        panic!("expected first Claimed, got {first_claim:?}");
    };
    // Release claim atomically with pending queue write; re-claim with same/lower wall-clock
    // timestamp must still advance monotonically above stamp1 (R2 #8).
    store
        .update_pending_with_notice_ops(
            "parent-a",
            &[] as &[QueuedMessage],
            &[] as &[QueuedMessage],
            &[],
            &[NoticeClaimRelease {
                completion_id: "call-notice:call-5".to_owned(),
                expected_stamp: Some(stamp1),
                expected_command_id: Some("cmd-5a".to_owned()),
                expected_worker_epoch: None,
                require_worker_epoch_match: false,
                receipt_key: "teardown:cmd-5a".to_owned(),
                reconcile_only: false,
            }],
        )
        .await
        .unwrap();
    let second_claim = store
        .begin_call_notice_dispatch("parent-a", "call-notice:call-5", "cmd-5b", 400)
        .await
        .unwrap();
    let DispatchClaim::Claimed(stamp2) = second_claim else {
        panic!("expected second Claimed, got {second_claim:?}");
    };
    assert!(
        stamp2 > stamp1,
        "re-claimed stamp ({stamp2}) must strictly advance past previous stamp ({stamp1})"
    );
    // Stale delayed release for stamp1 must NOT clear stamp2.
    assert!(
        !store
            .clear_call_notice_dispatching(
                "parent-a",
                "call-notice:call-5",
                Some(stamp1),
                Some("cmd-5a"),
                401,
            )
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .call_notice_delivery("parent-a", "call-5")
            .await
            .unwrap()
            .unwrap()
            .dispatch_started_at_ms,
        Some(stamp2)
    );
    // Owner withdrawal settles notice atomically with pending queue update.
    store
        .update_pending_with_notice_ops(
            "parent-a",
            &[] as &[QueuedMessage],
            &[] as &[QueuedMessage],
            &[(
                "call-notice:call-5".to_owned(),
                "owner_withdrawn".to_owned(),
            )],
            &[],
        )
        .await
        .unwrap();
    let withdrawn = store
        .call_notice_delivery("parent-a", "call-5")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(withdrawn.state, NoticeDeliveryState::Unknown);
    assert_eq!(withdrawn.reason.as_deref(), Some("owner_withdrawn"));
    // mark_call_notice_accepted must NEVER resurrect an owner-withdrawn notice (R2 #6).
    assert!(
        !store
            .mark_call_notice_accepted("parent-a", "call-notice:call-5", 99, 410)
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .call_notice_delivery("parent-a", "call-5")
            .await
            .unwrap()
            .unwrap()
            .state,
        NoticeDeliveryState::Unknown
    );

    // 6b. Non-superseding user chunks (current_turn steering, repeated same-notice echo,
    //     untagged continuation) keep notice Accepted in both live projection and echo lookup (R2 #9).
    let (c_steer, req_steer) = make_notify_call("parent-a", "call-steer", "req-steer", true);
    store
        .admit_managed_call(&c_steer, &req_steer)
        .await
        .unwrap();
    assert!(
        store
            .advance_managed_call_through(&c_steer, &complete_steps(&c_steer, 420))
            .await
            .unwrap()
    );
    let empty_hw = std::collections::HashMap::new();
    store
        .upsert_event_batch_with_anchors(
            &[
                user_chunk_envelope("parent-a", 30, Some("call-notice:call-steer")),
                user_chunk_envelope("parent-a", 31, Some("call-notice:call-steer")),
                user_chunk_envelope("parent-a", 32, None),
                Envelope {
                    session_id: "parent-a".to_owned(),
                    seq: 33,
                    cmid: Some("steer-1".to_owned()),
                    event: Event::Update {
                        update: serde_json::json!({
                            "sessionUpdate": "user_message_chunk",
                            "content": { "type": "text", "text": "steer" },
                            "_meta": { "cowboyDelivery": "current_turn" },
                        }),
                    },
                },
            ],
            &empty_hw,
            &[],
            &[],
        )
        .await
        .unwrap();
    let steer_delivery = store
        .call_notice_delivery("parent-a", "call-steer")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(steer_delivery.state, NoticeDeliveryState::Accepted);
    assert_eq!(steer_delivery.echo_seq, Some(30));
    assert_eq!(
        store
            .call_notice_echo_outcome("parent-a", "call-notice:call-steer")
            .await
            .unwrap(),
        NoticeEchoEvidence::Echoed {
            echo_seq: 30,
            turn_completed: None,
            superseded_by_user_turn: false,
        }
    );
    // Completing the turn after steering marks it Consumed.
    store
        .upsert_event_batch_with_anchors(
            &[turn_end_envelope("parent-a", 34, "EndTurn")],
            &empty_hw,
            &[],
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .call_notice_delivery("parent-a", "call-steer")
            .await
            .unwrap()
            .unwrap()
            .state,
        NoticeDeliveryState::Consumed
    );

    // 6c. Legacy pre-2A opted-in Call (no pre-existing managed_call_notices row) gets a
    //     durable Consumed("result_read") row via UPSERT (R2 #12).
    let (c_legacy, req_legacy) = make_notify_call("parent-a", "call-legacy", "req-legacy", false);
    store
        .admit_managed_call(&c_legacy, &req_legacy)
        .await
        .unwrap();
    assert!(
        store
            .advance_managed_call_through(&c_legacy, &complete_steps(&c_legacy, 430))
            .await
            .unwrap()
    );
    assert!(
        store
            .call_notice_delivery("parent-a", "call-legacy")
            .await
            .unwrap()
            .is_none()
    );
    store
        .mark_call_notice_consumed_for_result_read(
            "parent-a",
            "call-legacy",
            "call-notice:call-legacy",
            431,
        )
        .await
        .unwrap();
    let legacy_delivery = store
        .call_notice_delivery("parent-a", "call-legacy")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(legacy_delivery.state, NoticeDeliveryState::Consumed);
    assert_eq!(legacy_delivery.reason.as_deref(), Some("result_read"));

    // 6d. Authoritative UndeliveredPrompt reopens unknown(dispatch_unconfirmed),
    //     requires matching dispatch_command_id (never unstamped rows), and
    //     deduplicates replays by last_undelivered_key without reopening owner_withdrawn.
    let (c_reopen, req_reopen) = make_notify_call("parent-a", "call-reopen", "req-reopen", true);
    store
        .admit_managed_call(&c_reopen, &req_reopen)
        .await
        .unwrap();
    assert!(
        store
            .advance_managed_call_through(&c_reopen, &complete_steps(&c_reopen, 440))
            .await
            .unwrap()
    );
    // Unstamped enqueued row must reject any conditional claim release (Finding 2).
    assert!(
        !store
            .update_pending_requiring_claim_release(
                "parent-a",
                &[] as &[QueuedMessage],
                &[] as &[QueuedMessage],
                &[NoticeClaimRelease {
                    completion_id: "call-notice:call-reopen".to_owned(),
                    expected_stamp: None,
                    expected_command_id: Some("forged-cmd".to_owned()),
                    expected_worker_epoch: Some("epoch-bad".to_owned()),
                    require_worker_epoch_match: false,
                    receipt_key: "worker:epoch-bad:1".to_owned(),
                    reconcile_only: false,
                }],
            )
            .await
            .unwrap(),
        "unstamped enqueued notice must never accept UndeliveredPrompt"
    );
    let DispatchClaim::Claimed(reopen_stamp) = store
        .begin_call_notice_dispatch("parent-a", "call-notice:call-reopen", "cmd-reopen-1", 441)
        .await
        .unwrap()
    else {
        panic!("expected Claimed for call-reopen");
    };
    assert!(
        store
            .record_call_notice_dispatch_worker_epoch(
                "parent-a",
                "call-notice:call-reopen",
                "cmd-reopen-1",
                "epoch-1",
            )
            .await
            .unwrap()
    );
    assert!(
        store
            .mark_call_notice_unknown_if(
                "parent-a",
                "call-notice:call-reopen",
                NoticeDeliveryState::Enqueued,
                Some(reopen_stamp),
                None,
                "dispatch_unconfirmed",
                None,
                442,
            )
            .await
            .unwrap()
    );
    // Wrong command_id or wrong worker_epoch (when required) must not release.
    assert!(
        !store
            .update_pending_requiring_claim_release(
                "parent-a",
                &[] as &[QueuedMessage],
                &[] as &[QueuedMessage],
                &[NoticeClaimRelease {
                    completion_id: "call-notice:call-reopen".to_owned(),
                    expected_stamp: None,
                    expected_command_id: Some("cmd-reopen-1".to_owned()),
                    expected_worker_epoch: Some("epoch-wrong".to_owned()),
                    require_worker_epoch_match: true,
                    receipt_key: "worker:epoch-wrong:9".to_owned(),
                    reconcile_only: false,
                }],
            )
            .await
            .unwrap()
    );
    assert!(
        store
            .update_pending_requiring_claim_release(
                "parent-a",
                &[] as &[QueuedMessage],
                &[] as &[QueuedMessage],
                &[NoticeClaimRelease {
                    completion_id: "call-notice:call-reopen".to_owned(),
                    expected_stamp: None,
                    expected_command_id: Some("cmd-reopen-1".to_owned()),
                    expected_worker_epoch: Some("epoch-1".to_owned()),
                    require_worker_epoch_match: true,
                    receipt_key: "worker:epoch-1:9".to_owned(),
                    reconcile_only: false,
                }],
            )
            .await
            .unwrap()
    );
    // Round 3 #13: reconcile_only succeeds while notice is still unclaimed
    // enqueued with matching last_undelivered_key, and refuses once re-claimed
    // or settled (leaving sessions.queue untouched).
    let repaired = QueuedMessage {
        id: "q-repaired".into(),
        text: "repaired".into(),
        content: Vec::new(),
        cmid: Some("call-notice:call-reopen".into()),
        schedule: None,
        origin: None,
        initiator: None,
        explicit: false,
        wake_dormant: false,
    };
    assert!(
        store
            .update_pending_requiring_claim_release(
                "parent-a",
                std::slice::from_ref(&repaired),
                &[] as &[QueuedMessage],
                &[NoticeClaimRelease {
                    completion_id: "call-notice:call-reopen".to_owned(),
                    expected_stamp: None,
                    expected_command_id: None,
                    expected_worker_epoch: None,
                    require_worker_epoch_match: false,
                    receipt_key: "worker:epoch-1:9".to_owned(),
                    reconcile_only: true,
                }],
            )
            .await
            .unwrap(),
        "reconcile_only must repair sessions.queue when notice is still unclaimed enqueued"
    );
    let reopened = store
        .call_notice_delivery("parent-a", "call-reopen")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reopened.state, NoticeDeliveryState::Enqueued);
    assert_eq!(reopened.reason, None);
    assert_eq!(reopened.dispatch_started_at_ms, None);
    assert_eq!(reopened.dispatch_command_id, None);
    assert_eq!(reopened.dispatch_worker_epoch, None);
    assert_eq!(
        reopened.last_undelivered_key.as_deref(),
        Some("worker:epoch-1:9")
    );

    // Re-claim and replay the SAME undelivered_key via conditional claim-release:
    // must return Ok(false), leave sessions.queue UNTOUCHED, and keep the fresh claim.
    let sentinel = QueuedMessage {
        id: "q-sentinel".into(),
        text: "must stay intact".into(),
        content: Vec::new(),
        cmid: None,
        schedule: None,
        origin: None,
        initiator: None,
        explicit: false,
        wake_dormant: false,
    };
    store
        .update_pending("parent-a", std::slice::from_ref(&sentinel), &[])
        .await
        .unwrap();
    let DispatchClaim::Claimed(fresh_stamp) = store
        .begin_call_notice_dispatch("parent-a", "call-notice:call-reopen", "cmd-reopen-2", 445)
        .await
        .unwrap()
    else {
        panic!("expected Claimed after reopen");
    };
    let applied = store
        .update_pending_requiring_claim_release(
            "parent-a",
            &[] as &[QueuedMessage],
            &[] as &[QueuedMessage],
            &[NoticeClaimRelease {
                completion_id: "call-notice:call-reopen".to_owned(),
                expected_stamp: None,
                expected_command_id: Some("cmd-reopen-1".to_owned()),
                expected_worker_epoch: Some("epoch-1".to_owned()),
                require_worker_epoch_match: false,
                receipt_key: "worker:epoch-1:9".to_owned(),
                reconcile_only: false,
            }],
        )
        .await
        .unwrap();
    assert!(
        !applied,
        "replaying the same undelivered_key must return Ok(false)"
    );
    // Round 3 #13: reconcile_only must also return Ok(false) once the notice
    // has been reclaimed/settled, leaving q-sentinel untouched.
    assert!(
        !store
            .update_pending_requiring_claim_release(
                "parent-a",
                &[] as &[QueuedMessage],
                &[] as &[QueuedMessage],
                &[NoticeClaimRelease {
                    completion_id: "call-notice:call-reopen".to_owned(),
                    expected_stamp: None,
                    expected_command_id: None,
                    expected_worker_epoch: None,
                    require_worker_epoch_match: false,
                    receipt_key: "worker:epoch-1:9".to_owned(),
                    reconcile_only: true,
                }],
            )
            .await
            .unwrap(),
        "reconcile_only must refuse overwriting sessions.queue once notice is claimed or settled"
    );
    assert_eq!(
        store
            .call_notice_delivery("parent-a", "call-reopen")
            .await
            .unwrap()
            .unwrap()
            .dispatch_started_at_ms,
        Some(fresh_stamp),
        "replaying the same undelivered_key must not clear a fresher dispatch claim"
    );
    let restored_after_dup = store
        .load_all()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.meta.id == "parent-a")
        .unwrap();
    assert_eq!(
        restored_after_dup.queue.len(),
        1,
        "duplicate undelivered_key must not overwrite sessions.queue"
    );
    assert_eq!(restored_after_dup.queue[0].id, "q-sentinel");

    // 6e. Owner withdrawal settles a still-queued unknown(dispatch_unconfirmed)
    //     notice to owner_withdrawn so a later UndeliveredPrompt cannot reopen it.
    assert!(
        store
            .mark_call_notice_unknown_if(
                "parent-a",
                "call-notice:call-reopen",
                NoticeDeliveryState::Enqueued,
                Some(fresh_stamp),
                None,
                "dispatch_unconfirmed",
                None,
                448,
            )
            .await
            .unwrap()
    );
    store
        .update_pending_with_notice_ops(
            "parent-a",
            &[] as &[QueuedMessage],
            &[] as &[QueuedMessage],
            &[(
                "call-notice:call-reopen".to_owned(),
                "owner_withdrawn".to_owned(),
            )],
            &[],
        )
        .await
        .unwrap();
    let withdrawn_unconfirmed = store
        .call_notice_delivery("parent-a", "call-reopen")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(withdrawn_unconfirmed.state, NoticeDeliveryState::Unknown);
    assert_eq!(
        withdrawn_unconfirmed.reason.as_deref(),
        Some("owner_withdrawn")
    );
    let reopened_after_withdraw = store
        .update_pending_requiring_claim_release(
            "parent-a",
            &[] as &[QueuedMessage],
            &[] as &[QueuedMessage],
            &[NoticeClaimRelease {
                completion_id: "call-notice:call-reopen".to_owned(),
                expected_stamp: None,
                expected_command_id: Some("cmd-reopen-2".to_owned()),
                expected_worker_epoch: None,
                require_worker_epoch_match: false,
                receipt_key: "worker:epoch-2:99".to_owned(),
                reconcile_only: false,
            }],
        )
        .await
        .unwrap();
    assert!(
        !reopened_after_withdraw,
        "owner_withdrawn notice must never be reopened by a late UndeliveredPrompt"
    );

    // Owner-withdrawn notices must NEVER be reopened by a late UndeliveredPrompt.
    store
        .update_pending_with_notice_ops(
            "parent-a",
            &[] as &[QueuedMessage],
            &[] as &[QueuedMessage],
            &[],
            &[NoticeClaimRelease {
                completion_id: "call-notice:call-5".to_owned(),
                expected_stamp: None,
                expected_command_id: Some("cmd-5b".to_owned()),
                expected_worker_epoch: None,
                require_worker_epoch_match: false,
                receipt_key: "worker:epoch-9:1".to_owned(),
                reconcile_only: false,
            }],
        )
        .await
        .unwrap();
    let still_withdrawn = store
        .call_notice_delivery("parent-a", "call-5")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still_withdrawn.state, NoticeDeliveryState::Unknown);
    assert_eq!(still_withdrawn.reason.as_deref(), Some("owner_withdrawn"));

    // 7. Keyset pagination walks all unsettled rows cleanly (Finding 10).
    let (c6, req6) = make_notify_call("parent-b", "call-6", "req-6", true);
    store.admit_managed_call(&c6, &req6).await.unwrap();
    assert!(
        store
            .advance_managed_call_through(&c6, &complete_steps(&c6, 370))
            .await
            .unwrap()
    );
    let p1 = store.unsettled_call_notices_page(None, 1).await.unwrap();
    assert_eq!(p1.len(), 1);
    let p2 = store
        .unsettled_call_notices_page(Some((p1[0].created_at_ms, &p1[0].completion_id)), 10)
        .await
        .unwrap();
    assert!(!p2.is_empty());
    assert!(
        p2.iter()
            .all(|row| row.completion_id != p1[0].completion_id)
    );

    // Exact-ID batch lookup returns all requested rows.
    let batch = store
        .call_notice_deliveries_for_parent(
            "parent-a",
            &["call-1".into(), "call-4".into(), "call-5".into()],
        )
        .await
        .unwrap();
    assert_eq!(batch.len(), 3);

    // 8. Foreign-key cascade deletes notices when the parent session is purged.
    store.delete_session("parent-a").await.unwrap();
    assert_eq!(store.purge_deleted(-1).await.unwrap(), 1);
    assert!(
        store
            .call_notice_by_parent_completion_id("parent-a", "call-notice:call-5")
            .await
            .unwrap()
            .is_none(),
        "purging parent session must cascade-delete managed_call_notices"
    );

    let _ = std::fs::remove_dir_all(&root);
}
