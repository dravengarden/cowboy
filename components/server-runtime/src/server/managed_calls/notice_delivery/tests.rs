use super::*;
use crate::agent_model::Event;
use crate::core::{DispatchReq, Hub, PersistenceHealth, RestoredSession, StoreSink};
use crate::managed_calls::Request;
use crate::managed_calls::lifecycle::{Placement, Record, State};
use crate::server::background_jobs::run_store_writer_with_acks;
use crate::server::managed_calls::runner::outcome::notify_parent_in;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

const PARENT: &str = "parent-notice-test";

struct TestController {
    hub: Hub,
    dispatch_rx: mpsc::Receiver<DispatchReq>,
    writer: tokio::task::JoinHandle<()>,
    _shutdown_tx: watch::Sender<bool>,
}

impl TestController {
    async fn start(store: &Store) -> Self {
        let health = Arc::new(PersistenceHealth::default());
        let (sink, rx) = StoreSink::channel(4096, health.clone());
        let hub = Hub::with_store(Some(sink));
        let (dispatch_tx, dispatch_rx) = mpsc::channel(64);
        hub.set_dispatch_tx(dispatch_tx);
        let mut restored: Vec<RestoredSession> = store
            .load_all()
            .await
            .unwrap()
            .into_iter()
            .map(|ls| RestoredSession {
                meta: ls.meta,
                log: ls.events,
                event_count: ls.event_count,
                reached_start: ls.reached_start,
                next_seq: ls.next_seq,
                queue: ls.queue,
                drafts: ls.drafts,
                config_options: ls.config_options,
                config_preferences: ls.config_preferences,
                mobile_review_state: ls.mobile_review_state,
                folder_id: ls.folder_id,
                last_event_at_ms: ls.last_event_at_ms,
            })
            .collect();
        prune_settled_restored_notices(store, &mut restored).await;
        hub.restore_reconciling_runtime(restored);
        let (shutdown_tx, shutdown) = watch::channel(false);
        let writer_store = store.clone();
        let writer_acks = hub.store_ack_sink();
        let writer = tokio::spawn(async move {
            run_store_writer_with_acks(writer_store, rx, health, shutdown, Some(writer_acks)).await;
        });
        Self {
            hub,
            dispatch_rx,
            writer,
            _shutdown_tx: shutdown_tx,
        }
    }

    async fn stop(self) {
        drop(self.hub);
        tokio::time::timeout(Duration::from_secs(30), self.writer)
            .await
            .expect("store writer drains cleanly on shutdown")
            .unwrap();
    }
}

async fn setup_store(tag: &str) -> (Store, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "cowboy-notice-server-{tag}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let store = Store::connect("sqlite::memory:", root.join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    (store, root)
}

async fn insert_parent(store: &Store, id: &str, status: &str) {
    let meta = serde_json::from_value(json!({
        "id": id,
        "provider": "codex",
        "machine_id": "hawk",
        "workspace_id": "project",
        "cwd": "/workspace",
        "title": "Parent",
        "origin": "api",
        "status": status,
        "agent_session_id": "native-thread-1"
    }))
    .unwrap();
    store.insert_session(&meta).await.unwrap();
}

async fn admit_and_complete_call(store: &Store, parent: &str, call_id: &str, ts: i64) -> Record {
    let raw = format!(
        r#"{{"schema":1,"request_id":"req-{call_id}","purpose":"review","instruction":"check","context":{{"scope":"current-worktree"}},"access":"read-only","conversation":{{"mode":"fresh"}},"labels":{{"notify":"parent"}}}}"#
    );
    let request = Request::parse(raw.as_bytes()).unwrap();
    let queued = Record {
        schema: 1,
        call_id: call_id.into(),
        request_id: format!("req-{call_id}"),
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
        created_at_ms: ts,
        updated_at_ms: ts,
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
    store.admit_managed_call(&queued, &request).await.unwrap();
    let mut starting = queued.clone();
    starting.state = State::Starting;
    starting.revision = 2;
    starting.updated_at_ms = ts + 1;
    let mut running = starting.clone();
    running.state = State::Running;
    running.revision = 3;
    running.started_at_ms = Some(ts + 2);
    running.updated_at_ms = ts + 2;
    running.input_revision = Some(queued.request_digest.clone());
    let mut done = running.clone();
    done.state = State::Completed;
    done.revision = 4;
    done.updated_at_ms = ts + 10;
    done.finished_at_ms = Some(ts + 10);
    done.result = Some(json!({"summary": "ok"}));
    assert!(
        store
            .advance_managed_call_through(&queued, &[starting, running, done.clone()])
            .await
            .unwrap()
    );
    done
}

#[tokio::test]
async fn live_event_writer_advances_and_async_requeue_releases_exact_claim() {
    let (store, root) = setup_store("live").await;
    insert_parent(&store, PARENT, "running").await;
    insert_parent(&store, "other-parent", "running").await;
    let record = admit_and_complete_call(&store, PARENT, "call-live-1", 1000).await;

    let mut ctrl = TestController::start(&store).await;
    ctrl.hub.set_status(PARENT, Status::Running, None);
    ctrl.hub.set_status("other-parent", Status::Running, None);

    // Client submission with reserved `call-notice:` prefix is rejected (Finding 1).
    ctrl.hub.submit(
        "other-parent",
        "spoof".into(),
        Vec::new(),
        Some("call-notice:call-live-1".into()),
    );
    assert!(ctrl.dispatch_rx.try_recv().is_err());

    notify_parent_in(&ctrl.hub, Some(&store), &record).await;
    let req = ctrl
        .dispatch_rx
        .try_recv()
        .expect("notice dispatched immediately");
    assert_eq!(req.cmid.as_deref(), Some("call-notice:call-live-1"));

    // Claiming succeeds once, rejects duplicate, and synchronous error releases exact token.
    let DispatchGate::Proceed(Some(token1)) =
        before_notice_dispatch_in(&ctrl.hub, Some(&store), PARENT, req.cmid.as_deref()).await
    else {
        panic!("expected Proceed(Some(token))");
    };
    assert_eq!(
        before_notice_dispatch_in(&ctrl.hub, Some(&store), PARENT, req.cmid.as_deref()).await,
        DispatchGate::DropDuplicate
    );
    after_notice_dispatch_error_in(
        &ctrl.hub,
        Some(&store),
        PARENT,
        req.cmid.as_deref(),
        Some(&token1),
    )
    .await;
    let DispatchGate::Proceed(Some(token2)) =
        before_notice_dispatch_in(&ctrl.hub, Some(&store), PARENT, req.cmid.as_deref()).await
    else {
        panic!("expected Proceed(Some(token2)) after sync release");
    };

    // Simulate asynchronous runtime non-delivery (UndeliveredPrompt -> hub.requeue_prompt) (Finding 2).
    ctrl.hub
        .requeue_prompt(PARENT, req.text, req.content, req.cmid.clone());
    let DispatchGate::Proceed(Some(token3)) =
        before_notice_dispatch_in(&ctrl.hub, Some(&store), PARENT, req.cmid.as_deref()).await
    else {
        panic!("expected Proceed(Some(token3)) after async requeue release");
    };
    assert!(token3.stamp > token2.stamp);

    // Multi-block rollover echo + TurnEnd through the real store writer commits atomically (Findings 4 & 5).
    ctrl.hub.push_tagged(
        PARENT,
        Event::Update {
            update: json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "preamble"}}),
        },
        Some("call-notice:call-live-1".into()),
    );
    ctrl.hub.push(
        PARENT,
        Event::Update {
            update: json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "[Cowboy]"}}),
        },
    );
    ctrl.hub.push(
        PARENT,
        Event::TurnEnd {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    );
    ctrl.stop().await;

    let delivery = store
        .call_notice_delivery(PARENT, "call-live-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivery.state, NoticeDeliveryState::Consumed);
    assert_eq!(delivery.reason.as_deref(), Some("turn_completed"));
    assert!(delivery.echo_seq.is_some());

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn restart_recovery_replays_pre_send_fences_post_send_and_respects_owner_withdrawal() {
    let (store, root) = setup_store("restart").await;
    insert_parent(&store, "parent-pre-queued", "exited").await;
    insert_parent(&store, "parent-pre-lost", "running").await;
    insert_parent(&store, "parent-post-send", "running").await;
    insert_parent(&store, "parent-paused", "exited").await;
    insert_parent(&store, "parent-withdrawn", "exited").await;
    insert_parent(&store, "parent-stale-queue", "running").await;

    let _c_pre_q = admit_and_complete_call(&store, "parent-pre-queued", "call-pre-q", 1000).await;
    let _c_pre_l = admit_and_complete_call(&store, "parent-pre-lost", "call-pre-l", 1010).await;
    let c_post = admit_and_complete_call(&store, "parent-post-send", "call-post", 1020).await;
    let _c_paused = admit_and_complete_call(&store, "parent-paused", "call-paused", 1030).await;
    let _c_withdrawn =
        admit_and_complete_call(&store, "parent-withdrawn", "call-withdrawn", 1040).await;
    let _c_stale = admit_and_complete_call(&store, "parent-stale-queue", "call-stale", 1050).await;

    {
        let mut ctrl = TestController::start(&store).await;
        ctrl.hub
            .submit_call_notice("parent-pre-queued", "call-pre-q", "notice pre-q".into());
        let _ = ctrl.dispatch_rx.try_recv();
        ctrl.hub.requeue_prompt(
            "parent-pre-queued",
            "notice pre-q".into(),
            Vec::new(),
            Some("call-notice:call-pre-q".into()),
        );
        ctrl.hub
            .set_status("parent-pre-queued", Status::Exited, None);

        ctrl.hub.set_paused("parent-paused", true);
        ctrl.hub
            .submit_call_notice("parent-paused", "call-paused", "notice paused".into());
        ctrl.hub.set_status("parent-paused", Status::Exited, None);

        // Owner moves a queued notice to drafts -> durably withdrawn & draft loses reserved cmid (Finding 12).
        ctrl.hub.set_paused("parent-withdrawn", true);
        ctrl.hub.submit_call_notice(
            "parent-withdrawn",
            "call-withdrawn",
            "notice withdrawn".into(),
        );
        let qid = ctrl
            .hub
            .queue_and_drafts("parent-withdrawn", |_| true)
            .unwrap()
            .0[0]
            .id
            .clone();
        ctrl.hub.queued_to_draft("parent-withdrawn", &qid);
        assert!(
            ctrl.hub
                .queue_and_drafts("parent-withdrawn", |_| true)
                .unwrap()
                .1[0]
                .cmid
                .is_none()
        );

        // Consumed notice left as stale queue row in DB -> pruned at startup recovery (Finding 7).
        ctrl.hub.set_paused("parent-stale-queue", true);
        ctrl.hub
            .submit_call_notice("parent-stale-queue", "call-stale", "stale".into());
        store
            .mark_call_notice_consumed(
                "parent-stale-queue",
                "call-notice:call-stale",
                "result_read",
                None,
                1060,
            )
            .await
            .unwrap();

        notify_parent_in(&ctrl.hub, Some(&store), &c_post).await;
        let _ = ctrl.dispatch_rx.try_recv();
        assert!(matches!(
            before_notice_dispatch_in(
                &ctrl.hub,
                Some(&store),
                "parent-post-send",
                Some("call-notice:call-post"),
            )
            .await,
            DispatchGate::Proceed(Some(_))
        ));
        ctrl.stop().await;
    }

    let mut restarted = TestController::start(&store).await;
    restarted
        .hub
        .set_status("parent-pre-queued", Status::Exited, None);
    restarted
        .hub
        .set_status("parent-pre-lost", Status::Running, None);
    restarted
        .hub
        .set_status("parent-post-send", Status::Running, None);
    restarted.hub.set_paused("parent-paused", true);
    restarted
        .hub
        .set_status("parent-paused", Status::Exited, None);
    restarted
        .hub
        .set_status("parent-withdrawn", Status::Running, None);
    restarted
        .hub
        .set_status("parent-stale-queue", Status::Running, None);

    recover_in(&restarted.hub, &store).await;

    let mut dispatched = Vec::new();
    while let Ok(req) = restarted.dispatch_rx.try_recv() {
        dispatched.push((req.session_id, req.cmid));
    }
    assert!(dispatched.contains(&(
        "parent-pre-queued".into(),
        Some("call-notice:call-pre-q".into())
    )));
    assert!(dispatched.contains(&(
        "parent-pre-lost".into(),
        Some("call-notice:call-pre-l".into())
    )));
    assert!(!dispatched.iter().any(|(sid, _)| matches!(
        sid.as_str(),
        "parent-post-send" | "parent-paused" | "parent-withdrawn" | "parent-stale-queue"
    )));
    assert!(
        !restarted
            .hub
            .has_queued_cmid("parent-stale-queue", "call-notice:call-stale"),
        "settled notice must be pruned from restored queue before dispatch"
    );

    let withdrawn = store
        .call_notice_delivery("parent-withdrawn", "call-withdrawn")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(withdrawn.state, NoticeDeliveryState::Unknown);
    assert_eq!(withdrawn.reason.as_deref(), Some("owner_withdrawn"));

    // Late surviving worker echo + TurnEnd still promotes dispatch_unconfirmed -> accepted -> consumed.
    restarted.hub.push_tagged(
        "parent-post-send",
        Event::Update {
            update: json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "[Cowboy]"}}),
        },
        Some("call-notice:call-post".into()),
    );
    restarted.hub.push(
        "parent-post-send",
        Event::TurnEnd {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    );
    restarted.stop().await;

    let promoted = store
        .call_notice_delivery("parent-post-send", "call-post")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(promoted.state, NoticeDeliveryState::Consumed);
    assert_eq!(promoted.reason.as_deref(), Some("turn_completed"));

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn native_session_rollover_and_late_machine_reconnect_deliver_notice_once() {
    let (store, root) = setup_store("rollover").await;
    insert_parent(&store, PARENT, "exited").await;
    let record = admit_and_complete_call(&store, PARENT, "call-rollover-1", 2000).await;

    {
        let mut ctrl = TestController::start(&store).await;
        ctrl.hub.set_status(PARENT, Status::Exited, None);
        ctrl.hub
            .submit_call_notice(PARENT, "call-rollover-1", "rollover notice".into());
        let first = ctrl.dispatch_rx.try_recv().expect("initial wake request");
        assert_eq!(first.cmid.as_deref(), Some("call-notice:call-rollover-1"));
        assert!(
            ctrl.hub
                .mark_context_rolled_over(PARENT, 1, 3_600_000, 120_000)
                .is_some()
        );
        ctrl.hub
            .requeue_prompt(PARENT, first.text, first.content, first.cmid);
        ctrl.hub.set_status(PARENT, Status::Exited, None);
        ctrl.stop().await;
    }

    // Simulate late Machine reconnect waking dormant Exited rolled-over parent (Finding 9)
    // and verify atomic head check refuses waking the wrong prompt (Finding 8).
    let mut ctrl2 = TestController::start(&store).await;
    assert!(
        ctrl2
            .hub
            .session_info(PARENT)
            .unwrap()
            .meta
            .agent_session_id
            .is_none()
    );
    ctrl2.hub.set_status(PARENT, Status::Exited, None);
    assert!(
        !ctrl2
            .hub
            .wake_queued_call_notice(PARENT, "call-notice:different-call"),
        "wake_queued_call_notice must never pop a mismatched queue head"
    );
    assert!(ctrl2.dispatch_rx.try_recv().is_err());

    wake_notices_for_machine(&ctrl2.hub, Some(&store), "hawk").await;
    let drained = ctrl2
        .dispatch_rx
        .try_recv()
        .expect("Machine reconnect wakes dormant parent for queued notice");
    assert_eq!(drained.cmid.as_deref(), Some("call-notice:call-rollover-1"));
    assert!(matches!(
        before_notice_dispatch_in(&ctrl2.hub, Some(&store), PARENT, drained.cmid.as_deref()).await,
        DispatchGate::Proceed(Some(_))
    ));

    ctrl2.hub.push_tagged(
        PARENT,
        Event::Update {
            update: json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "rollover preamble"}}),
        },
        Some("call-notice:call-rollover-1".into()),
    );
    ctrl2.hub.push(
        PARENT,
        Event::Update {
            update: json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "rollover notice"}}),
        },
    );
    ctrl2.hub.push(
        PARENT,
        Event::TurnEnd {
            stop_reason: "EndTurn".into(),
            usage: None,
        },
    );
    ctrl2.stop().await;

    let delivery = store
        .call_notice_delivery(PARENT, "call-rollover-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivery.state, NoticeDeliveryState::Consumed);

    let mut ctrl3 = TestController::start(&store).await;
    ctrl3.hub.set_status(PARENT, Status::Running, None);
    recover_in(&ctrl3.hub, &store).await;
    notify_parent_in(&ctrl3.hub, Some(&store), &record).await;
    assert!(ctrl3.dispatch_rx.try_recv().is_err());
    ctrl3.stop().await;

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn reading_terminal_result_marks_consumed_and_withdraws_queued_notice() {
    let (store, root) = setup_store("result-read").await;
    insert_parent(&store, PARENT, "busy").await;
    let record = admit_and_complete_call(&store, PARENT, "call-read-1", 3000).await;

    let ctrl = TestController::start(&store).await;
    ctrl.hub.set_status(PARENT, Status::Busy, None);
    notify_parent_in(&ctrl.hub, Some(&store), &record).await;
    assert!(ctrl.hub.has_queued_cmid(PARENT, "call-notice:call-read-1"));

    // A refused windowed read (e.g. text_window_required) must NOT mark consumed
    // or withdraw the queued notice (Finding 6).
    let parent_meta = ctrl.hub.session_info(PARENT).unwrap().meta;
    let ledger =
        crate::managed_calls::service::Ledger::for_parent(store.clone(), "svc-1", &parent_meta)
            .unwrap();
    let refused = read_windowed_result_in(
        &ctrl.hub,
        Some(&store),
        &ledger,
        "call-read-1",
        None,
        |_| Err("text_window_required".to_owned()),
    )
    .await;
    assert_eq!(refused, Err("text_window_required".to_owned()));
    assert!(ctrl.hub.has_queued_cmid(PARENT, "call-notice:call-read-1"));
    assert_eq!(
        store
            .call_notice_delivery(PARENT, "call-read-1")
            .await
            .unwrap()
            .unwrap()
            .state,
        NoticeDeliveryState::Enqueued
    );

    // Succeeding windowed read commits consumption durably and withdraws the queued notice.
    let value = read_windowed_result_in(
        &ctrl.hub,
        Some(&store),
        &ledger,
        "call-read-1",
        None,
        |_| Ok(()),
    )
    .await
    .unwrap();
    assert_eq!(value["notice_delivery"]["state"], "consumed");
    assert_eq!(value["notice_delivery"]["reason"], "result_read");

    assert!(!ctrl.hub.has_queued_cmid(PARENT, "call-notice:call-read-1"));
    let delivery = store
        .call_notice_delivery(PARENT, "call-read-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivery.state, NoticeDeliveryState::Consumed);
    assert_eq!(delivery.reason.as_deref(), Some("result_read"));
    assert_eq!(
        before_notice_dispatch_in(
            &ctrl.hub,
            Some(&store),
            PARENT,
            Some("call-notice:call-read-1")
        )
        .await,
        DispatchGate::DropDuplicate
    );
    ctrl.stop().await;

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn unavailable_parent_marks_notice_unknown_and_keeps_call_visible() {
    let (store, root) = setup_store("unavailable").await;
    insert_parent(&store, PARENT, "running").await;
    let record = admit_and_complete_call(&store, PARENT, "call-gone-1", 4000).await;
    store.delete_session(PARENT).await.unwrap();

    let ctrl = TestController::start(&store).await;
    notify_parent_in(&ctrl.hub, Some(&store), &record).await;
    ctrl.stop().await;

    let delivery = store
        .call_notice_delivery(PARENT, "call-gone-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivery.state, NoticeDeliveryState::Unknown);
    assert_eq!(delivery.reason.as_deref(), Some("parent_unavailable"));
    let visible = store.managed_calls(PARENT, None).await.unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].call_id, "call-gone-1");

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn round2_owner_withdrawal_edit_fence_and_recovery_park() {
    let (store, root) = setup_store("r2-withdraw").await;
    insert_parent(&store, PARENT, "busy").await;
    let rec_edit = admit_and_complete_call(&store, PARENT, "call-r2-edit", 5000).await;
    let rec_park = admit_and_complete_call(&store, PARENT, "call-r2-park", 5010).await;

    let ctrl = TestController::start(&store).await;
    ctrl.hub.set_status(PARENT, Status::Busy, None);
    notify_parent_in(&ctrl.hub, Some(&store), &rec_edit).await;
    let edit_row_id = ctrl
        .hub
        .queue_and_drafts(PARENT, |_| true)
        .unwrap()
        .0
        .into_iter()
        .find(|m| m.cmid.as_deref() == Some("call-notice:call-r2-edit"))
        .unwrap()
        .id;

    // Previewing a non-empty edit installs the withdrawal fence: dispatch
    // defers, and committing durably settles the notice to owner_withdrawn
    // while stripping cmid from the edited queue row. An overlapping draft
    // added while the commit is in flight is deferred and merged AFTER commit
    // so it NEVER erases the committed edit in SQL!
    let op = crate::core::OwnerNoticeWithdrawalOp::EditQueued {
        id: &edit_row_id,
        text: "edited user text",
        content: &[],
    };
    let (mut preview, ack_rx) = ctrl
        .hub
        .preview_and_send_owner_notice_withdrawal(PARENT, &op)
        .expect("queued notice edit must produce withdrawal preview")
        .expect("store writer available");
    assert!(matches!(
        before_notice_dispatch_in(
            &ctrl.hub,
            Some(&store),
            PARENT,
            Some("call-notice:call-r2-edit")
        )
        .await,
        DispatchGate::Requeue(_)
    ));
    // Overlapping ordinary draft mutation while commit is in flight!
    ctrl.hub
        .add_draft(PARENT, "concurrent draft".into(), Vec::new(), None);
    ack_rx.await.unwrap().unwrap();
    assert!(ctrl.hub.commit_owner_notice_withdrawal(&mut preview, &op));
    let (queue, drafts) = ctrl.hub.queue_and_drafts(PARENT, |_| true).unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].text, "edited user text");
    assert_eq!(queue[0].cmid, None);
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].text, "concurrent draft");
    let delivery = store
        .call_notice_delivery(PARENT, "call-r2-edit")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivery.state, NoticeDeliveryState::Unknown);
    assert_eq!(delivery.reason.as_deref(), Some("owner_withdrawn"));

    // Execution recovery parking retains the notice as a plain draft and settles
    // its durable notice row to execution_recovery_parked (R2 #14).
    ctrl.hub.park_recovery_draft(
        PARENT,
        "recovered notice".into(),
        Vec::new(),
        Some(crate::store::completion_id_for_call(&rec_park.call_id)),
    );
    let (_, drafts) = ctrl.hub.queue_and_drafts(PARENT, |_| true).unwrap();
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts[1].text, "recovered notice");
    assert_eq!(drafts[1].cmid, None);
    ctrl.stop().await;

    // Verify SQL retained BOTH the committed edit AND the concurrent draft
    // (the concurrent draft's emit_pending did NOT overwrite the edit!).
    let persisted = store
        .load_all()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.meta.id == PARENT)
        .unwrap();
    assert_eq!(persisted.queue.len(), 1);
    assert_eq!(persisted.queue[0].text, "edited user text");
    assert_eq!(persisted.queue[0].cmid, None);
    assert_eq!(persisted.drafts.len(), 2);

    let parked = store
        .call_notice_delivery(PARENT, "call-r2-park")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parked.state, NoticeDeliveryState::Unknown);
    assert_eq!(parked.reason.as_deref(), Some("execution_recovery_parked"));

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn round2_reserved_cmid_nul_and_interrupt_rejected() {
    let (store, root) = setup_store("r2-reserved").await;
    insert_parent(&store, PARENT, "running").await;
    let ctrl = TestController::start(&store).await;
    ctrl.hub.set_status(PARENT, Status::Running, None);

    // Embedded NUL or direct call-notice: prefix must be rejected on client submit
    // and dispatch_interrupt_once (R2 #2, #3).
    assert!(crate::agent_model::is_reserved_call_notice_cmid(
        "ca\0ll-notice:call-1"
    ));
    ctrl.hub.submit(
        PARENT,
        "forged nul".into(),
        Vec::new(),
        Some("ca\0ll-notice:call-1".into()),
    );
    assert!(
        ctrl.hub
            .queue_and_drafts(PARENT, |_| true)
            .unwrap()
            .0
            .is_empty()
    );

    ctrl.hub
        .publish_delivery_hold(PARENT, Some("call-notice:forged".into()));
    assert!(!ctrl.hub.dispatch_interrupt_once(
        PARENT,
        "call-notice:forged",
        "forged interrupt".into(),
        Vec::new(),
    ));
    ctrl.stop().await;

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn round2_live_claim_survives_recovery_and_reconnect_wakes_pending_release() {
    let (store, root) = setup_store("r2-live-claim").await;
    insert_parent(&store, PARENT, "exited").await;
    let record = admit_and_complete_call(&store, PARENT, "call-r2-live", 6000).await;

    let mut ctrl = TestController::start(&store).await;
    ctrl.hub.set_status(PARENT, Status::Exited, None);
    notify_parent_in(&ctrl.hub, Some(&store), &record).await;
    let req = ctrl.dispatch_rx.try_recv().expect("notice dispatched");

    // Claim in THIS Controller lifetime.
    let DispatchGate::Proceed(Some(token)) =
        before_notice_dispatch_in(&ctrl.hub, Some(&store), PARENT, req.cmid.as_deref()).await
    else {
        panic!("expected Proceed(Some(token))");
    };
    assert!(ctrl.hub.owns_notice_claim(
        PARENT,
        "call-notice:call-r2-live",
        token.stamp,
        Some(&token.command_id),
    ));

    // Concurrent startup recovery must NOT mark THIS lifetime's live claim dispatch_unconfirmed (R2 #5).
    recover_in(&ctrl.hub, &store).await;
    let during = store
        .call_notice_delivery(PARENT, "call-r2-live")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(during.state, NoticeDeliveryState::Enqueued);
    assert_eq!(during.dispatch_started_at_ms, Some(token.stamp));

    // Authoritative non-delivery requeue records pending_releases + atomic UpdatePending (R2 #7).
    drop(req.ownership);
    ctrl.hub
        .requeue_prompt(&req.session_id, req.text, req.content, req.cmid);
    assert!(
        ctrl.hub
            .has_pending_notice_release(PARENT, "call-notice:call-r2-live")
    );
    // Machine reconnect wakes Exited parent even if the background writer hasn't
    // cleared the DB stamp yet (R2 #10).
    wake_notices_for_machine(&ctrl.hub, Some(&store), "hawk").await;
    let req2 = ctrl
        .dispatch_rx
        .try_recv()
        .expect("reconnect must wake parent with pending release");
    let DispatchGate::Proceed(Some(token2)) =
        before_notice_dispatch_in(&ctrl.hub, Some(&store), PARENT, req2.cmid.as_deref()).await
    else {
        panic!("expected second Proceed(Some(token2))");
    };
    assert!(token2.stamp > token.stamp);
    drop(req2.ownership);
    ctrl.stop().await;

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn round2_oversized_result_rejected_before_consume_and_legacy_read_upserts() {
    let (store, root) = setup_store("r2-result-budget").await;
    insert_parent(&store, PARENT, "busy").await;
    let record = admit_and_complete_call(&store, PARENT, "call-r2-big", 7000).await;

    let ctrl = TestController::start(&store).await;
    ctrl.hub.set_status(PARENT, Status::Busy, None);
    notify_parent_in(&ctrl.hub, Some(&store), &record).await;

    let parent_meta = ctrl.hub.session_info(PARENT).unwrap().meta;
    let ledger =
        crate::managed_calls::service::Ledger::for_parent(store.clone(), "svc-1", &parent_meta)
            .unwrap();

    // Oversized structured payload fails with reply_too_large BEFORE consuming notice (R2 #11).
    let oversized =
        read_windowed_result_in(&ctrl.hub, Some(&store), &ledger, "call-r2-big", None, |r| {
            *r = json!({ "huge_scalar": "x".repeat(crate::agent_api::MAX_REPLY_BYTES) });
            Ok(())
        })
        .await;
    assert_eq!(oversized, Err("reply_too_large".to_owned()));
    assert!(ctrl.hub.has_queued_cmid(PARENT, "call-notice:call-r2-big"));

    // Remaining aggregate broker budget exhaustion fails with budget_exceeded BEFORE
    // consuming the notice (R3 #10).
    let tight_budget = cowboy_codeact_runtime::ReplyBudget::new(64);
    let call_budget = cowboy_codeact_runtime::CallBudget::new(&tight_budget);
    let exhausted = read_windowed_result_in(
        &ctrl.hub,
        Some(&store),
        &ledger,
        "call-r2-big",
        Some(&call_budget),
        |_| Ok(()),
    )
    .await;
    assert_eq!(exhausted, Err("budget_exceeded".to_owned()));
    assert!(ctrl.hub.has_queued_cmid(PARENT, "call-notice:call-r2-big"));
    assert_eq!(
        store
            .call_notice_delivery(PARENT, "call-r2-big")
            .await
            .unwrap()
            .unwrap()
            .state,
        NoticeDeliveryState::Enqueued
    );

    // Legacy pre-2A opted-in Call (verified with absent row in store conformance)
    // still yields a durable Consumed("result_read") row on read_windowed_result_in (R2 #12).
    let _ = admit_and_complete_call(&store, PARENT, "call-r2-legacy", 7100).await;
    let val = read_windowed_result_in(
        &ctrl.hub,
        Some(&store),
        &ledger,
        "call-r2-legacy",
        None,
        |_| Ok(()),
    )
    .await
    .unwrap();
    assert_eq!(val["notice_delivery"]["state"], "consumed");
    assert_eq!(val["notice_delivery"]["reason"], "result_read");
    ctrl.stop().await;

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn round3_fifo_withdrawal_clear_queue_claim_guard_and_legacy_draft_activation() {
    let (store, root) = setup_store("r3-hardening").await;
    insert_parent(&store, PARENT, "busy").await;
    let rec1 = admit_and_complete_call(&store, PARENT, "call-r3-1", 8000).await;
    let rec2 = admit_and_complete_call(&store, PARENT, "call-r3-2", 8010).await;
    let _rec3 = admit_and_complete_call(&store, PARENT, "call-r3-3", 8020).await;

    let ctrl = TestController::start(&store).await;
    ctrl.hub.set_status(PARENT, Status::Busy, None);
    notify_parent_in(&ctrl.hub, Some(&store), &rec1).await;

    // 1. ClearQueue commits through the FIFO store writer and only removes the exact
    //    previewed rows, preserving a second notice that arrives concurrently (R3 #2, #5).
    let clear_op = crate::core::OwnerNoticeWithdrawalOp::ClearQueue;
    let (mut preview, ack_rx) = ctrl
        .hub
        .preview_and_send_owner_notice_withdrawal(PARENT, &clear_op)
        .unwrap()
        .unwrap();
    // Second Call finishes while ClearQueue is committing!
    notify_parent_in(&ctrl.hub, Some(&store), &rec2).await;
    ack_rx.await.unwrap().unwrap();
    assert!(
        ctrl.hub
            .commit_owner_notice_withdrawal(&mut preview, &clear_op)
    );
    assert!(!ctrl.hub.has_queued_cmid(PARENT, "call-notice:call-r3-1"));
    assert!(
        ctrl.hub.has_queued_cmid(PARENT, "call-notice:call-r3-2"),
        "newly arrived notice must NOT be dropped by an earlier ClearQueue preview"
    );
    assert_eq!(
        store
            .call_notice_delivery(PARENT, "call-r3-1")
            .await
            .unwrap()
            .unwrap()
            .reason
            .as_deref(),
        Some("owner_withdrawn")
    );
    assert_eq!(
        store
            .call_notice_delivery(PARENT, "call-r3-2")
            .await
            .unwrap()
            .unwrap()
            .state,
        NoticeDeliveryState::Enqueued
    );

    // 2. NoticeClaimGuard fences startup recovery between SQL claim and Hub registration (R3 #6).
    let claim_guard = ctrl
        .hub
        .begin_notice_claim_guard(PARENT, "call-notice:call-r3-3");
    let DispatchClaim::Claimed(stamp) = store
        .begin_call_notice_dispatch(PARENT, "call-notice:call-r3-3", "cmd-r3-3", 8100)
        .await
        .unwrap()
    else {
        panic!("expected Claimed");
    };
    recover_in(&ctrl.hub, &store).await;
    assert_eq!(
        store
            .call_notice_delivery(PARENT, "call-r3-3")
            .await
            .unwrap()
            .unwrap()
            .state,
        NoticeDeliveryState::Enqueued,
        "recovery must skip a notice whose current-lifetime claim is still being acquired"
    );
    ctrl.hub.record_notice_claim(
        PARENT,
        "call-notice:call-r3-3",
        stamp,
        "cmd-r3-3".to_owned(),
    );
    drop(claim_guard);

    // 3. Authoritative post-crash UndeliveredPrompt replay uses two-phase conditional
    //    commit: Hub stays fenced/untouched until Ok(true), and duplicate replay returns
    //    Ok(false) without mutating Hub or releasing a fresher claim (R3 #7, R4 #4, #5).
    ctrl.hub.clear_notice_claim(PARENT, "call-notice:call-r3-3");
    let crate::core::UndeliveredRequeue::Pending(mut undelivered_preview, ack) = ctrl
        .hub
        .preview_and_send_undelivered_notice_requeue(crate::core::NoticeRequeueRequest {
            session_id: PARENT.to_owned(),
            text: "notice 3".into(),
            content: Vec::new(),
            cmid: "call-notice:call-r3-3".into(),
            expected_command_id: Some("cmd-r3-3".into()),
            expected_worker_epoch: Some("epoch-1".into()),
            require_worker_epoch_match: false,
            receipt_key: "worker:epoch-1:42".into(),
            require_claim_release: true,
        })
    else {
        panic!("expected UndeliveredRequeue::Pending");
    };
    assert!(
        !ctrl.hub.has_queued_cmid(PARENT, "call-notice:call-r3-3"),
        "Hub queue must NOT expose retry before SQL commits"
    );
    assert!(
        ack.await.unwrap().unwrap(),
        "first undelivered receipt must apply"
    );
    ctrl.hub
        .commit_undelivered_notice_requeue(&mut undelivered_preview);
    assert!(ctrl.hub.has_queued_cmid(PARENT, "call-notice:call-r3-3"));
    assert_eq!(
        store
            .call_notice_delivery(PARENT, "call-r3-3")
            .await
            .unwrap()
            .unwrap()
            .dispatch_started_at_ms,
        None
    );

    // Replaying the SAME undelivered_key returns Ok(false) and leaves Hub untouched.
    let crate::core::UndeliveredRequeue::Pending(mut dup_preview, dup_ack) = ctrl
        .hub
        .preview_and_send_undelivered_notice_requeue(crate::core::NoticeRequeueRequest {
            session_id: PARENT.to_owned(),
            text: "notice 3 dup".into(),
            content: Vec::new(),
            cmid: "call-notice:call-r3-3".into(),
            expected_command_id: Some("cmd-r3-3".into()),
            expected_worker_epoch: Some("epoch-1".into()),
            require_worker_epoch_match: false,
            receipt_key: "worker:epoch-1:42".into(),
            require_claim_release: true,
        })
    else {
        panic!("expected Pending for duplicate check");
    };
    assert!(
        !dup_ack.await.unwrap().unwrap(),
        "duplicate undelivered_key must return Ok(false)"
    );
    ctrl.hub
        .abort_undelivered_notice_requeue(&mut dup_preview, true);

    // Exact row_id removal refuses to delete a notice whose row_id changed (reopened).
    let reopened_row_id = ctrl
        .hub
        .queued_row_id_by_cmid(PARENT, "call-notice:call-r3-3")
        .unwrap();
    assert!(
        !ctrl
            .hub
            .remove_queued_by_id_if_cmid(PARENT, "q-stale-row-id", "call-notice:call-r3-3"),
        "stale row_id must never remove a freshly reopened notice"
    );
    assert!(ctrl.hub.has_queued_cmid(PARENT, "call-notice:call-r3-3"));
    assert!(ctrl.hub.remove_queued_by_id_if_cmid(
        PARENT,
        &reopened_row_id,
        "call-notice:call-r3-3"
    ));

    // 4. Legacy parked notice drafts keep their text on restore and bulk activation (R3 #11).
    ctrl.stop().await;
    let legacy_draft = crate::core::QueuedMessage {
        id: "q-legacy".into(),
        text: "legacy parked notice".into(),
        content: Vec::new(),
        cmid: Some("call-notice:old-call".into()),
        schedule: None,
    };
    store
        .update_pending(PARENT, &[], &[legacy_draft])
        .await
        .unwrap();

    let ctrl2 = TestController::start(&store).await;
    ctrl2.hub.set_status(PARENT, Status::Busy, None);
    let (_, drafts) = ctrl2.hub.queue_and_drafts(PARENT, |_| true).unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].cmid, None);
    ctrl2.hub.activate_all_drafts(PARENT);
    let (queue, drafts) = ctrl2.hub.queue_and_drafts(PARENT, |_| true).unwrap();
    assert!(drafts.is_empty());
    assert!(queue.iter().any(|m| m.text == "legacy parked notice"));
    ctrl2.stop().await;

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn round4_atomic_withdrawal_drain_undelivered_reject_and_budget_headroom() {
    let (store, root) = setup_store("r4-hardening").await;
    insert_parent(&store, PARENT, "busy").await;
    let rec1 = admit_and_complete_call(&store, PARENT, "call-r4-1", 9000).await;
    let rec2 = admit_and_complete_call(&store, PARENT, "call-r4-2", 9010).await;

    let mut ctrl = TestController::start(&store).await;
    ctrl.hub.set_status(PARENT, Status::Busy, None);
    notify_parent_in(&ctrl.hub, Some(&store), &rec1).await;

    // 1. Atomic preview_and_send_owner_notice_withdrawal + ClearQueue commit on an
    //    idle (Running) parent must drain a concurrently arrived surviving notice.
    let clear_op = crate::core::OwnerNoticeWithdrawalOp::ClearQueue;
    let (mut preview, ack_rx) = ctrl
        .hub
        .preview_and_send_owner_notice_withdrawal(PARENT, &clear_op)
        .unwrap()
        .unwrap();
    notify_parent_in(&ctrl.hub, Some(&store), &rec2).await;
    ctrl.hub.set_status(PARENT, Status::Running, None);
    ack_rx.await.unwrap().unwrap();
    assert!(
        ctrl.hub
            .commit_owner_notice_withdrawal(&mut preview, &clear_op)
    );
    let req = tokio::time::timeout(Duration::from_secs(2), ctrl.dispatch_rx.recv())
        .await
        .expect("ClearQueue commit must drain surviving queued notice on idle parent")
        .expect("dispatch channel open");
    assert_eq!(req.cmid.as_deref(), Some("call-notice:call-r4-2"));
    drop(req.ownership);

    // 2. Closed ReplyBudget refuses read_windowed_result_in BEFORE consuming
    //    the notice even when remaining bytes are large (R4 #10), while an open
    //    budget with upper-bound headroom succeeds.
    let _ = store
        .begin_call_notice_dispatch(
            PARENT,
            "call-notice:call-r4-2",
            "cmd-r4-2",
            1_765_432_100_000,
        )
        .await
        .unwrap();
    let _ = store
        .mark_call_notice_accepted(PARENT, "call-notice:call-r4-2", 987_654, 1_765_432_100_500)
        .await
        .unwrap();
    let parent_meta = ctrl.hub.session_info(PARENT).unwrap().meta;
    let ledger =
        crate::managed_calls::service::Ledger::for_parent(store.clone(), "svc-1", &parent_meta)
            .unwrap();
    let closed_budget = cowboy_codeact_runtime::ReplyBudget::new(1024 * 1024);
    closed_budget.close();
    let closed_call_budget = cowboy_codeact_runtime::CallBudget::new(&closed_budget);
    assert_eq!(
        read_windowed_result_in(
            &ctrl.hub,
            Some(&store),
            &ledger,
            "call-r4-2",
            Some(&closed_call_budget),
            |_| Ok(()),
        )
        .await,
        Err("budget_exceeded".to_owned()),
        "closed ReplyBudget must refuse result read before consuming notice"
    );
    assert_eq!(
        store
            .call_notice_delivery(PARENT, "call-r4-2")
            .await
            .unwrap()
            .unwrap()
            .state,
        NoticeDeliveryState::Accepted
    );
    let probe =
        read_windowed_result_in(&ctrl.hub, Some(&store), &ledger, "call-r4-2", None, |_| {
            Ok(())
        })
        .await
        .unwrap();
    let exact_len = serde_json::to_vec(&probe).unwrap().len();
    // A CallBudget matching the post-consume rendered size succeeds without exceeding budget.
    let fit_budget = cowboy_codeact_runtime::ReplyBudget::new(exact_len + 128);
    let call_budget = cowboy_codeact_runtime::CallBudget::new(&fit_budget);
    let val = read_windowed_result_in(
        &ctrl.hub,
        Some(&store),
        &ledger,
        "call-r4-2",
        Some(&call_budget),
        |_| Ok(()),
    )
    .await
    .unwrap();
    assert!(serde_json::to_vec(&val).unwrap().len() <= exact_len + 128);
    ctrl.stop().await;

    // 3. UndeliveredRequeue::Rejected leaves Hub queue & in_flight untouched when
    //    the persistence worker channel is closed.
    let health = Arc::new(PersistenceHealth::default());
    let (sink, rx) = StoreSink::channel(8, health);
    drop(rx);
    let closed_hub = Hub::with_store(Some(sink));
    let meta = serde_json::from_value(json!({
        "id": PARENT,
        "provider": "codex",
        "machine_id": "hawk",
        "workspace_id": "project",
        "cwd": "/workspace",
        "title": "Parent",
        "origin": "api",
        "status": "busy",
        "agent_session_id": "native-thread-1"
    }))
    .unwrap();
    closed_hub.restore_reconciling_runtime(vec![RestoredSession {
        meta,
        log: Vec::new(),
        event_count: 0,
        reached_start: true,
        next_seq: 1,
        queue: Vec::new(),
        drafts: Vec::new(),
        config_options: None,
        config_preferences: Default::default(),
        mobile_review_state: serde_json::Value::Null,
        folder_id: None,
        last_event_at_ms: None,
    }]);
    let rejected =
        closed_hub.preview_and_send_undelivered_notice_requeue(crate::core::NoticeRequeueRequest {
            session_id: PARENT.to_owned(),
            text: "should not mutate".into(),
            content: Vec::new(),
            cmid: "call-notice:call-r4-2".into(),
            expected_command_id: Some("cmd-r4-2".into()),
            expected_worker_epoch: Some("epoch-1".into()),
            require_worker_epoch_match: false,
            receipt_key: "worker:epoch-1:7".into(),
            require_claim_release: true,
        });
    assert!(matches!(
        rejected,
        crate::core::UndeliveredRequeue::Rejected(_)
    ));
    assert!(
        !closed_hub.has_queued_cmid(PARENT, "call-notice:call-r4-2"),
        "rejected undelivered requeue must not mutate Hub queue"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn round5_redesign_invariants_atomic_clear_staged_consume_legacy_adopt_and_disconnect_refusal()
 {
    let (store, root) = setup_store("r5-redesign").await;
    insert_parent(&store, PARENT, "busy").await;
    let rec = admit_and_complete_call(&store, PARENT, "call-r5-stage", 9500).await;

    let ctrl = TestController::start(&store).await;
    ctrl.hub.set_status(PARENT, Status::Busy, None);

    // 1. Atomic apply_or_preview_owner_notice_withdrawal applies plain ClearQueue
    //    synchronously under ONE sessions lock with no second-pass race window.
    ctrl.hub
        .submit(PARENT, "plain user msg".into(), Vec::new(), None);
    assert!(matches!(
        ctrl.hub.apply_or_preview_owner_notice_withdrawal(
            PARENT,
            &crate::core::OwnerNoticeWithdrawalOp::ClearQueue,
        ),
        crate::core::OwnerQueueMutation::AppliedSync(true)
    ));
    assert!(
        ctrl.hub
            .queue_and_drafts(PARENT, |_| true)
            .unwrap()
            .0
            .is_empty()
    );

    // 2. prepare_windowed_result_in stages reply & reserves budget WITHOUT marking
    //    the notice Consumed until commit_windowed_result_in runs AFTER relay.
    notify_parent_in(&ctrl.hub, Some(&store), &rec).await;
    let parent_meta = ctrl.hub.session_info(PARENT).unwrap().meta;
    let ledger =
        crate::managed_calls::service::Ledger::for_parent(store.clone(), "svc-1", &parent_meta)
            .unwrap();
    let budget = cowboy_codeact_runtime::ReplyBudget::new(1024 * 1024);
    let call_budget = cowboy_codeact_runtime::CallBudget::new(&budget);
    let (staged, reservation) = prepare_windowed_result_in(
        Some(&store),
        &ledger,
        "call-r5-stage",
        Some(&call_budget),
        |_| Ok(()),
    )
    .await
    .unwrap();
    assert!(reservation.is_some());
    assert_eq!(staged["notice_delivery"]["state"], "enqueued");
    assert_eq!(
        store
            .call_notice_delivery(PARENT, "call-r5-stage")
            .await
            .unwrap()
            .unwrap()
            .state,
        NoticeDeliveryState::Enqueued,
        "staging a result must NOT consume the notice before authorized relay"
    );
    // Simulating broker commit_after_relay consumes durably and removes queued notice.
    commit_windowed_result_in(&ctrl.hub, Some(&store), &rec)
        .await
        .unwrap();
    assert_eq!(
        store
            .call_notice_delivery(PARENT, "call-r5-stage")
            .await
            .unwrap()
            .unwrap()
            .state,
        NoticeDeliveryState::Consumed
    );
    assert!(
        !ctrl
            .hub
            .has_queued_cmid(PARENT, "call-notice:call-r5-stage"),
        "commit_after_relay must withdraw the queued notice"
    );

    // 3. Legacy queued notice (pre-2A Call with no pre-existing managed_call_notices
    //    row) is adopted atomically in the state implied by transcript evidence (Finding 9).
    let raw_legacy = r#"{"schema":1,"request_id":"req-r5-legacy","purpose":"review","instruction":"check","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}"#;
    let req_legacy = Request::parse(raw_legacy.as_bytes()).unwrap();
    let mut rec_legacy = rec.clone();
    rec_legacy.call_id = "call-r5-legacy".into();
    rec_legacy.request_id = "req-r5-legacy".into();
    rec_legacy.request_digest = req_legacy.digest();
    rec_legacy.labels = req_legacy.labels.clone();
    rec_legacy.child_session_id = "child-call-r5-legacy".into();
    rec_legacy.state = State::Queued;
    rec_legacy.revision = 1;
    rec_legacy.finished_at_ms = None;
    rec_legacy.started_at_ms = None;
    rec_legacy.input_revision = None;
    rec_legacy.result = None;
    store
        .admit_managed_call(&rec_legacy, &req_legacy)
        .await
        .unwrap();
    let mut starting = rec_legacy.clone();
    starting.state = State::Starting;
    starting.revision = 2;
    starting.updated_at_ms = 9598;
    let mut running = starting.clone();
    running.state = State::Running;
    running.revision = 3;
    running.started_at_ms = Some(9599);
    running.updated_at_ms = 9599;
    running.input_revision = Some(rec_legacy.request_digest.clone());
    let mut done = running.clone();
    done.state = State::Completed;
    done.revision = 4;
    done.updated_at_ms = 9600;
    done.finished_at_ms = Some(9600);
    done.result = Some(json!({"summary": "done"}));
    assert!(
        store
            .advance_managed_call_through(&rec_legacy, &[starting, running, done])
            .await
            .unwrap()
    );
    assert!(
        store
            .call_notice_delivery(PARENT, "call-r5-legacy")
            .await
            .unwrap()
            .is_none()
    );
    store
        .upsert_event_batch_with_anchors(
            &[
                crate::core::Envelope {
                    session_id: PARENT.to_owned(),
                    seq: 12,
                    event: Event::Update {
                        update: json!({
                            "sessionUpdate": "user_message_chunk",
                            "content": {"type": "text", "text": "legacy notice"},
                        }),
                    },
                    cmid: Some("call-notice:call-r5-legacy".to_owned()),
                },
                crate::core::Envelope {
                    session_id: PARENT.to_owned(),
                    seq: 13,
                    event: Event::TurnEnd {
                        stop_reason: "end_turn".to_owned(),
                        usage: None,
                    },
                    cmid: None,
                },
            ],
            &std::collections::HashMap::new(),
            &[],
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .adopt_legacy_call_notice_atomic(
                PARENT,
                "call-r5-legacy",
                "call-notice:call-r5-legacy",
                "cmd-r5-legacy",
                9601,
            )
            .await
            .unwrap(),
        LegacyAdoptionOutcome::SettledFromEvidence
    );
    let adopted = store
        .call_notice_delivery(PARENT, "call-r5-legacy")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(adopted.state, NoticeDeliveryState::Consumed);
    assert_eq!(adopted.reason.as_deref(), Some("turn_completed"));
    assert_eq!(
        before_notice_dispatch_in(
            &ctrl.hub,
            Some(&store),
            PARENT,
            Some("call-notice:call-r5-legacy")
        )
        .await,
        DispatchGate::DropDuplicate
    );

    // 4. RemoteRuntime::try_prompt_traced refuses enqueue synchronously after disconnect().
    let runtime = crate::remote_runtime::RemoteRuntime::for_test(ctrl.hub.clone(), Vec::new());
    runtime.disconnect();
    assert!(
        runtime
            .try_prompt_traced(
                PARENT,
                vec![json!({"type": "text", "text": "late notice"})],
                Some("call-notice:call-r5-late".into()),
                None,
            )
            .is_err(),
        "try_prompt_traced must synchronously refuse handoff after disconnect"
    );

    drop(runtime);
    ctrl.stop().await;
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn v2_review_findings_invariants() {
    let (store, root) = setup_store("v2-review").await;
    insert_parent(&store, PARENT, "busy").await;
    let rec_amb = admit_and_complete_call(&store, PARENT, "call-v2-amb", 9700).await;
    let rec_fb = admit_and_complete_call(&store, PARENT, "call-v2-fb", 9710).await;
    let rec_replay = admit_and_complete_call(&store, PARENT, "call-v2-replay", 9720).await;

    let ctrl = TestController::start(&store).await;
    ctrl.hub.set_status(PARENT, Status::Busy, None);

    // Finding 5: Lost `begin_call_notice_dispatch` response (SQL committed, response lost)
    // is recovered via stable `dispatch_command_id` on next dispatch instead of dropping!
    let DispatchClaim::Claimed(committed_stamp) = store
        .begin_call_notice_dispatch(PARENT, "call-notice:call-v2-amb", "cmd-v2-amb", 9701)
        .await
        .unwrap()
    else {
        panic!("expected Claimed");
    };
    ctrl.hub.record_ambiguous_notice_claim(
        PARENT,
        "call-notice:call-v2-amb",
        "cmd-v2-amb".to_owned(),
        9701,
    );
    let gate = before_notice_dispatch_in(
        &ctrl.hub,
        Some(&store),
        PARENT,
        Some("call-notice:call-v2-amb"),
    )
    .await;
    assert_eq!(
        gate,
        DispatchGate::Proceed(Some(NoticeDispatchToken {
            stamp: committed_stamp,
            command_id: "cmd-v2-amb".to_owned(),
        })),
        "ambiguous committed claim must be recovered instead of dropped as AlreadyHandled"
    );
    let _ = rec_amb;

    // Finding 10: Identity-checked fallback requeue must NOT clear a newer prompt's in_flight guard.
    ctrl.hub.set_status(PARENT, Status::Running, None);
    notify_parent_in(&ctrl.hub, Some(&store), &rec_fb).await;
    // Consume the notice dispatch from queue so it is in_flight at epoch N.
    let _ = store
        .begin_call_notice_dispatch(PARENT, "call-notice:call-v2-fb", "cmd-v2-fb", 9711)
        .await
        .unwrap();
    ctrl.hub
        .record_notice_claim(PARENT, "call-notice:call-v2-fb", 9711, "cmd-v2-fb".into());
    // Preview requeue at epoch N, then simulate a replacement runtime starting a newer prompt (epoch N+1).
    let crate::core::UndeliveredRequeue::Pending(mut fb_preview, _rx) = ctrl
        .hub
        .preview_and_send_undelivered_notice_requeue(crate::core::NoticeRequeueRequest {
            session_id: PARENT.to_owned(),
            text: "fb notice".into(),
            content: Vec::new(),
            cmid: "call-notice:call-v2-fb".into(),
            expected_command_id: Some("cmd-v2-fb".into()),
            expected_worker_epoch: None,
            require_worker_epoch_match: false,
            receipt_key: "teardown:cmd-v2-fb".into(),
            require_claim_release: true,
        })
    else {
        panic!("expected Pending");
    };
    ctrl.hub.clear_in_flight(PARENT);
    ctrl.hub.submit(
        PARENT,
        "newer user prompt".into(),
        Vec::new(),
        Some("user-newer-1".into()),
    );
    assert!(ctrl.hub.session_has_in_flight_prompt(PARENT));
    ctrl.hub
        .fallback_undelivered_notice_requeue(&mut fb_preview);
    assert!(
        ctrl.hub.session_has_in_flight_prompt(PARENT),
        "teardown fallback must NEVER clear a newer prompt's in_flight guard"
    );
    assert!(ctrl.hub.has_queued_cmid(PARENT, "call-notice:call-v2-fb"));

    // Finding 7 / Round 2 #15 / Round 3 #13: Worker replay of an already-committed
    // receipt_key repairs durable `sessions.queue` via the two-phase FIFO writer
    // BEFORE mutating Hub and BEFORE sending Frame::Ack, guarded transactionally
    // by `reconcile_only` so a settled notice is never written back.
    ctrl.hub.clear_in_flight(PARENT);
    let _ = store
        .begin_call_notice_dispatch(PARENT, "call-notice:call-v2-replay", "cmd-v2-replay", 9720)
        .await
        .unwrap();
    assert!(
        store
            .update_pending_requiring_claim_release(
                PARENT,
                &[],
                &[],
                &[crate::store::NoticeClaimRelease {
                    completion_id: "call-notice:call-v2-replay".into(),
                    expected_stamp: None,
                    expected_command_id: Some("cmd-v2-replay".into()),
                    expected_worker_epoch: None,
                    require_worker_epoch_match: false,
                    receipt_key: "worker:epoch-1:88".into(),
                    reconcile_only: false,
                }],
            )
            .await
            .unwrap()
    );
    let crate::core::UndeliveredRequeue::Pending(mut replay_preview, replay_rx) =
        ctrl.hub.preview_and_send_reconcile_notice_requeue(
            PARENT,
            crate::managed_calls::notice::call_notice(&rec_replay),
            Vec::new(),
            "call-notice:call-v2-replay",
            Some("cmd-v2-replay"),
            "worker:epoch-1:88",
        )
    else {
        panic!("expected Pending durable reconciliation");
    };
    assert!(replay_rx.await.unwrap().unwrap());
    ctrl.hub
        .commit_undelivered_notice_requeue(&mut replay_preview);
    assert!(ctrl.hub.is_undelivered_key_seen(
        PARENT,
        "call-notice:call-v2-replay",
        "worker:epoch-1:88"
    ));
    let persisted_replay_queue = store
        .load_all()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.meta.id == PARENT)
        .unwrap()
        .queue;
    assert!(
        persisted_replay_queue
            .iter()
            .any(|m| m.cmid.as_deref() == Some("call-notice:call-v2-replay")),
        "two-phase replay reconciliation must durably repair sessions.queue before ACK"
    );

    // Finding 6: Hub tracks teardown requeues and drains them cleanly before persistence closes.
    let drained = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let drained_flag = Arc::clone(&drained);
    ctrl.hub.spawn_teardown_requeue(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        drained_flag.store(true, std::sync::atomic::Ordering::Release);
    });
    ctrl.hub
        .drain_teardown_requeues(Duration::from_secs(2))
        .await;
    assert!(
        drained.load(std::sync::atomic::Ordering::Acquire),
        "drain_teardown_requeues must wait for tracked teardown tasks"
    );

    // Finding 13: Disconnected runtime's projection_fence blocks snapshot projection into Hub.
    let stale_runtime =
        crate::remote_runtime::RemoteRuntime::for_test(ctrl.hub.clone(), Vec::new());
    stale_runtime.disconnect();
    let stale_snapshot: crate::runtime_wire::WorkerSnapshot = serde_json::from_value(json!({
        "session_id": PARENT,
        "worker_epoch": "stale-epoch",
        "generation": "gen-1",
        "state": "crashed",
        "pending_prompt_count": 0,
        "pending_permissions": [],
        "drain_requested": false,
        "last_runtime_seq": 0
    }))
    .unwrap();
    assert!(
        !stale_runtime.snapshot_native_test(stale_snapshot).await,
        "disconnected runtime must refuse projecting snapshots into Hub"
    );
    drop(stale_runtime);

    // Finding 1: Concurrent send_pending + cancel does not deadlock under unified lock order.
    let rt = crate::remote_runtime::RemoteRuntime::for_test(ctrl.hub.clone(), Vec::new());
    for i in 0..20 {
        let _ = rt.try_prompt_traced(
            PARENT,
            vec![json!({"type": "text", "text": format!("p-{i}")})],
            None,
            None,
        );
        rt.cancel(PARENT);
    }
    drop(rt);

    ctrl.stop().await;
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn v2_round2_findings_invariants() {
    let (store, root) = setup_store("v2-round2").await;
    insert_parent(&store, PARENT, "busy").await;
    let rec_mix = admit_and_complete_call(&store, PARENT, "call-r2-mix", 9800).await;
    let rec_idem = admit_and_complete_call(&store, PARENT, "call-r2-idem", 9801).await;
    let rec_pend = admit_and_complete_call(&store, PARENT, "call-r2-pend", 9802).await;

    let ctrl = TestController::start(&store).await;
    ctrl.hub.set_status(PARENT, Status::Busy, None);

    // High #1: Mixed-queue ClearQueue fences plain prompts ahead of the notice
    // so `drain_now` cannot pop/execute the removed plain prompt before commit.
    ctrl.hub
        .submit(PARENT, "plain before notice".into(), Vec::new(), None);
    notify_parent_in(&ctrl.hub, Some(&store), &rec_mix).await;
    let crate::core::OwnerQueueMutation::CommitPending(mut withdrawal, clear_rx) =
        ctrl.hub.apply_or_preview_owner_notice_withdrawal(
            PARENT,
            &crate::core::OwnerNoticeWithdrawalOp::ClearQueue,
        )
    else {
        panic!("expected CommitPending for mixed queue with call-notice");
    };
    let head_row_id = ctrl
        .hub
        .queue_and_drafts(PARENT, |_| true)
        .unwrap()
        .0
        .first()
        .unwrap()
        .id
        .clone();
    assert!(
        ctrl.hub.is_row_withdrawing(PARENT, &head_row_id),
        "plain head row in mixed ClearQueue must be fenced before commit"
    );
    // Even if session becomes Running while commit is in flight, drain must NOT pop it.
    ctrl.hub.set_status(PARENT, Status::Running, None);
    ctrl.hub.drain_now(PARENT);
    assert!(
        !ctrl.hub.session_has_in_flight_prompt(PARENT),
        "drain_now must not dispatch a plain prompt fenced by in-flight ClearQueue"
    );
    assert!(clear_rx.await.unwrap().unwrap());
    assert!(ctrl.hub.commit_owner_notice_withdrawal(
        &mut withdrawal,
        &crate::core::OwnerNoticeWithdrawalOp::ClearQueue,
    ));
    assert!(!ctrl.hub.is_row_withdrawing(PARENT, &head_row_id));
    assert!(
        ctrl.hub
            .queue_and_drafts(PARENT, |_| true)
            .unwrap()
            .0
            .is_empty()
    );

    // Medium #10: `begin_call_notice_dispatch` is idempotent on the same
    // `command_id` inside one SQL tx, and rejects a different `command_id`.
    let DispatchClaim::Claimed(stamp_1) = store
        .begin_call_notice_dispatch(PARENT, "call-notice:call-r2-idem", "cmd-same", 9810)
        .await
        .unwrap()
    else {
        panic!("expected initial Claimed");
    };
    assert_eq!(
        store
            .begin_call_notice_dispatch(PARENT, "call-notice:call-r2-idem", "cmd-same", 9811)
            .await
            .unwrap(),
        DispatchClaim::Claimed(stamp_1),
        "retrying begin_call_notice_dispatch with the same command_id must reclaim the exact stamp"
    );
    assert_eq!(
        store
            .begin_call_notice_dispatch(PARENT, "call-notice:call-r2-idem", "cmd-other", 9812)
            .await
            .unwrap(),
        DispatchClaim::AlreadyHandled(NoticeDeliveryState::Enqueued),
        "different command_id must not hijack an active claim"
    );
    let _ = rec_idem;

    // Medium #11: Hub in-memory transcript echo blocks legacy adoption dispatch
    // even before the write-behind store flushes `events` to SQL.
    ctrl.hub.push_tagged(
        PARENT,
        Event::Update {
            update: json!({
                "sessionUpdate": "user_message_chunk",
                "content": {"type": "text", "text": "in-memory echo"},
            }),
        },
        Some("call-notice:call-r2-mem-echo".to_owned()),
    );
    assert!(
        ctrl.hub
            .has_echoed_cmid(PARENT, "call-notice:call-r2-mem-echo")
    );
    assert_eq!(
        before_notice_dispatch_in(
            &ctrl.hub,
            Some(&store),
            PARENT,
            Some("call-notice:call-r2-mem-echo"),
        )
        .await,
        DispatchGate::DropDuplicate,
        "in-memory transcript echo must prevent legacy adoption from dispatching twice"
    );

    // Round 3 #5: try_reserve_result_relay fences dispatch before flush, and
    // promote_result_relay_to_pending_consumption atomically promotes it.
    ctrl.hub.set_status(PARENT, Status::Busy, None);
    notify_parent_in(&ctrl.hub, Some(&store), &rec_pend).await;
    assert!(
        ctrl.hub
            .try_reserve_result_relay(PARENT, "call-notice:call-r2-pend")
    );
    assert!(matches!(
        before_notice_dispatch_in(
            &ctrl.hub,
            Some(&store),
            PARENT,
            Some("call-notice:call-r2-pend"),
        )
        .await,
        DispatchGate::Requeue(_)
    ));
    ctrl.hub.promote_result_relay_to_pending_consumption(
        PARENT,
        "call-r2-pend",
        "call-notice:call-r2-pend",
    );
    assert!(
        !ctrl
            .hub
            .is_result_relay_reserved(PARENT, "call-notice:call-r2-pend")
    );
    assert_eq!(
        before_notice_dispatch_in(
            &ctrl.hub,
            Some(&store),
            PARENT,
            Some("call-notice:call-r2-pend"),
        )
        .await,
        DispatchGate::DropDuplicate,
        "pending_result_consumption must flush result_read and drop dispatch"
    );
    assert!(
        ctrl.hub
            .pending_result_consumption(PARENT, "call-notice:call-r2-pend")
            .is_none()
    );
    let settled = store
        .call_notice_delivery(PARENT, "call-r2-pend")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(settled.state, NoticeDeliveryState::Consumed);
    assert_eq!(settled.reason.as_deref(), Some("result_read"));

    // Low #17: gateway ledger_unavailable keeps retryable schema-1 capacity error.
    let err_val = super::super::coordinator::ledger_unavailable();
    assert_eq!(err_val["error"]["code"], "capacity");
    assert_eq!(err_val["error"]["reason"], "ledger_unavailable");
    assert_eq!(err_val["error"]["retryable"], true);

    ctrl.stop().await;
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn result_relay_reservation_and_notice_claim_are_mutually_exclusive() {
    let (store, root) = setup_store("v3-round2-race").await;
    insert_parent(&store, PARENT, "busy").await;
    let rec_claim = admit_and_complete_call(&store, PARENT, "call-r4-claim", 9910).await;
    let rec_race = admit_and_complete_call(&store, PARENT, "call-r4-race", 9920).await;
    let ctrl = TestController::start(&store).await;
    ctrl.hub.set_status(PARENT, Status::Busy, None);
    notify_parent_in(&ctrl.hub, Some(&store), &rec_claim).await;
    notify_parent_in(&ctrl.hub, Some(&store), &rec_race).await;

    let cid_claim = "call-notice:call-r4-claim";
    // 1. While a notice dispatch claim is in progress (`claiming`), `calls.result`
    //    must fail to reserve rather than delivering a duplicate result reply.
    let guard = ctrl
        .hub
        .try_begin_notice_claim_guard(PARENT, cid_claim)
        .expect("claim guard must succeed when no reservation exists");
    assert!(
        !ctrl.hub.try_reserve_result_relay(PARENT, cid_claim),
        "calls.result reservation must be refused while notice claim is in progress"
    );
    drop(guard);

    // 2. While a notice claim is already active, `calls.result` must also be refused;
    //    once the notice echoes and clears the active claim, reservation succeeds.
    assert!(
        ctrl.hub
            .try_record_notice_claim(PARENT, cid_claim, 9915, "cmd-r4-claim".into())
    );
    assert!(
        !ctrl.hub.try_reserve_result_relay(PARENT, cid_claim),
        "calls.result reservation must be refused while notice claim is active"
    );
    ctrl.hub.clear_notice_claim(PARENT, cid_claim);
    assert!(
        ctrl.hub.try_reserve_result_relay(PARENT, cid_claim),
        "calls.result reservation must succeed after active notice claim clears"
    );
    // While reserved, `try_begin_notice_claim_guard` must also refuse to start claiming.
    assert!(
        ctrl.hub
            .try_begin_notice_claim_guard(PARENT, cid_claim)
            .is_none(),
        "notice claim guard must refuse while result relay reservation is held"
    );
    ctrl.hub.release_result_relay(PARENT, cid_claim);

    // 3. Interleaving a reservation between SQL `begin_call_notice_dispatch`
    //    returning `Claimed` and recording the claim must release the SQL
    //    dispatch stamp and return `DispatchGate::Requeue`.
    let cid_race = "call-notice:call-r4-race";
    let DispatchClaim::Claimed(claimed_stamp) = store
        .begin_call_notice_dispatch(PARENT, cid_race, "cmd-r4-race", 9925)
        .await
        .unwrap()
    else {
        panic!("expected initial Claimed");
    };
    assert!(
        ctrl.hub.try_reserve_result_relay(PARENT, cid_race),
        "interleaved result relay reservation succeeds before record_notice_claim"
    );
    let gate = finish_notice_claim_or_release(
        &ctrl.hub,
        &store,
        PARENT,
        cid_race,
        claimed_stamp,
        "cmd-r4-race".into(),
    )
    .await;
    assert!(
        matches!(gate, DispatchGate::Requeue(_)),
        "raced claim must requeue instead of proceeding, got {gate:?}"
    );
    assert!(
        !ctrl.hub.has_active_notice_claim(PARENT, cid_race),
        "raced claim must not record an active notice claim"
    );
    let row = store
        .call_notice_delivery(PARENT, "call-r4-race")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, NoticeDeliveryState::Enqueued);
    assert!(
        row.dispatch_started_at_ms.is_none(),
        "raced SQL dispatch_started_at_ms must be cleared back to enqueued"
    );
    assert!(
        row.dispatch_command_id.is_none(),
        "raced SQL dispatch_command_id must be cleared"
    );
    ctrl.hub.release_result_relay(PARENT, cid_race);

    ctrl.stop().await;
    let _ = std::fs::remove_dir_all(&root);
}
