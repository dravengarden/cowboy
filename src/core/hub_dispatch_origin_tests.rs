use super::*;
use crate::prompt_origin::{DeliveryOrigin, PromptDispatchRecord, RequestCaller};
use serde_json::json;
use tokio::sync::mpsc;

fn hub_session(id: &str) -> (Hub, mpsc::Receiver<DispatchReq>) {
    let hub = Hub::new();
    hub.create_local_session(
        id.into(),
        "codex".into(),
        "/tmp".into(),
        id.into(),
        SessionOrigin::Web,
        false,
    );
    let (tx, rx) = mpsc::channel(16);
    hub.set_dispatch_tx(tx);
    hub.set_status(id, Status::Running, None);
    (hub, rx)
}

fn finish_turn(hub: &Hub, id: &str, req: DispatchReq) {
    drop(req);
    hub.set_status(id, Status::Busy, None);
    hub.set_status(id, Status::Running, None);
    hub.complete_turn(id);
}

fn user_chunks(hub: &Hub, id: &str) -> Vec<(u64, serde_json::Value)> {
    let (log, _) = hub.snapshot(id).unwrap();
    log.into_iter()
        .filter_map(|env| match env.event {
            Event::Update { update }
                if update
                    .get("sessionUpdate")
                    .and_then(serde_json::Value::as_str)
                    == Some("user_message_chunk") =>
            {
                Some((env.seq, update))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn automation_caller_never_becomes_owner_and_does_not_reset_unattended_streak() {
    let (hub, mut rx) = hub_session("auto-s");
    let (sched_tx, mut sched_rx) = mpsc::channel(8);
    hub.set_scheduler_tx(sched_tx);
    hub.submit_automatic("auto-s", "wake".into(), "__wake__1".into(), "wakeup");
    let req0 = rx.try_recv().unwrap();
    finish_turn(&hub, "auto-s", req0);
    assert!(sched_rx.try_recv().is_err());

    hub.submit(
        "auto-s",
        "from automation".into(),
        vec![],
        Some("auto-cmid".into()),
        &RequestCaller::Automation,
    );
    let req = rx.try_recv().unwrap();
    assert_eq!(req.dispatch_origin, DeliveryOrigin::Automation);
    assert_eq!(req.trigger, DeliveryOrigin::Automation);
    assert!(
        sched_rx.try_recv().is_err(),
        "Automation prompt must not notify scheduler of a human turn"
    );
    hub.record_prompt_dispatch(
        "auto-s",
        req.cmid.clone().unwrap(),
        req.dispatch_origin.clone(),
        req.trigger.clone(),
        10,
    );

    hub.push_tagged_from_worker(
        "auto-s",
        Event::Update {
            update: json!({
                "sessionUpdate": "user_message_chunk",
                "promptOrigin": { "actor": "human", "source": "composer" },
                "content": { "type": "text", "text": "from automation" }
            }),
        },
        req.cmid.clone(),
    );
    let chunks = user_chunks(&hub, "auto-s");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].1["promptOrigin"]["actor"], "cowboy");
    assert_eq!(chunks[0].1["promptOrigin"]["source"], "automation");
    assert_eq!(chunks[0].1["autoResumed"], true);
    assert_eq!(
        hub.newest_owner_turn_in_retained_log("auto-s")
            .unwrap()
            .newest_match,
        None
    );

    finish_turn(&hub, "auto-s", req);
    hub.submit(
        "auto-s",
        "from person".into(),
        vec![],
        Some("human-cmid".into()),
        &RequestCaller::Person,
    );
    let req2 = rx.try_recv().unwrap();
    assert_eq!(req2.dispatch_origin, DeliveryOrigin::Owner);
    assert!(matches!(
        sched_rx.try_recv(),
        Ok(crate::scheduler::ScheduleCmd::HumanTurn { session_id }) if session_id == "auto-s"
    ));
    hub.record_prompt_dispatch(
        "auto-s",
        req2.cmid.clone().unwrap(),
        req2.dispatch_origin.clone(),
        req2.trigger.clone(),
        20,
    );
    hub.push_tagged_from_worker(
        "auto-s",
        Event::Update {
            update: json!({
                "sessionUpdate": "user_message_chunk",
                "autoResumed": true,
                "content": { "type": "text", "text": "from person" }
            }),
        },
        req2.cmid.clone(),
    );
    let chunks = user_chunks(&hub, "auto-s");
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[1].1["promptOrigin"]["actor"], "human");
    assert!(chunks[1].1.get("autoResumed").is_none());
    assert_eq!(
        hub.newest_owner_turn_in_retained_log("auto-s")
            .unwrap()
            .newest_match,
        Some(chunks[1].0)
    );
}

#[tokio::test]
async fn draft_activation_by_agent_stays_peer_across_busy_queue_and_restart() {
    let (hub, mut rx) = hub_session("draft-busy");
    let agent = RequestCaller::agent("caller-sess");
    hub.set_status("draft-busy", Status::Busy, None);

    hub.add_draft(
        "draft-busy",
        "owner text".into(),
        vec![],
        Some("d-1".into()),
        &RequestCaller::Person,
    );
    let draft_id = hub.inner.sessions.lock()["draft-busy"].drafts[0].id.clone();
    hub.activate_draft("draft-busy", &draft_id, None, &agent);
    assert!(rx.try_recv().is_err());

    let queued = hub.inner.sessions.lock()["draft-busy"].queue.clone();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].origin, Some(DeliveryOrigin::Owner));
    assert_eq!(
        queued[0].initiator,
        Some(DeliveryOrigin::peer("caller-sess"))
    );

    let meta = hub.session_info("draft-busy").unwrap().meta;
    let restarted = Hub::new();
    let (tx2, mut rx2) = mpsc::channel(16);
    restarted.set_dispatch_tx(tx2);
    restarted.restore_with_workers(
        vec![RestoredSession {
            meta,
            log: vec![],
            event_count: 0,
            reached_start: true,
            next_seq: 0,
            queue: queued,
            drafts: vec![],
            config_options: None,
            config_preferences: Default::default(),
            mobile_review_state: serde_json::Value::Null,
            folder_id: None,
            last_event_at_ms: None,
        }],
        &[],
    );
    restarted.set_status("draft-busy", Status::Running, None);
    restarted.complete_turn("draft-busy");

    let req = rx2.try_recv().expect("drained after restart");
    assert_eq!(req.dispatch_origin, DeliveryOrigin::peer("caller-sess"));
    assert_eq!(req.trigger, DeliveryOrigin::peer("caller-sess"));
}

#[tokio::test]
async fn activation_and_move_never_promote_non_owner_or_legacy_drafts() {
    let (hub, mut rx) = hub_session("src");
    hub.create_local_session(
        "dst".into(),
        "codex".into(),
        "/tmp".into(),
        "dst".into(),
        SessionOrigin::Web,
        false,
    );
    hub.set_status("dst", Status::Running, None);
    let agent = RequestCaller::agent("agent-1");

    hub.add_draft(
        "src",
        "from agent".into(),
        vec![],
        Some("a-1".into()),
        &agent,
    );
    let did = hub.inner.sessions.lock()["src"].drafts[0].id.clone();
    hub.activate_draft("src", &did, None, &RequestCaller::Person);
    let req = rx.try_recv().unwrap();
    assert_eq!(req.dispatch_origin, DeliveryOrigin::peer("agent-1"));
    assert_eq!(req.trigger, DeliveryOrigin::Owner);
    finish_turn(&hub, "src", req);

    hub.inner
        .sessions
        .lock()
        .get_mut("src")
        .unwrap()
        .drafts
        .push(QueuedMessage {
            id: "legacy-d".into(),
            text: "legacy draft".into(),
            content: vec![],
            cmid: Some("legacy-cmid".into()),
            schedule: None,
            origin: None,
            initiator: None,
        });
    hub.activate_draft("src", "legacy-d", None, &RequestCaller::Person);
    let req = rx.try_recv().unwrap();
    assert_eq!(req.dispatch_origin, DeliveryOrigin::cowboy("legacy_queue"));
    finish_turn(&hub, "src", req);

    hub.add_draft(
        "src",
        "owner draft".into(),
        vec![],
        Some("own-cmid".into()),
        &RequestCaller::Person,
    );
    let own_did = hub.inner.sessions.lock()["src"].drafts[0].id.clone();
    hub.move_draft("src", &own_did, "dst", &agent);
    {
        let sessions = hub.inner.sessions.lock();
        let moved = &sessions["dst"].drafts[0];
        assert_eq!(moved.origin, Some(DeliveryOrigin::peer("agent-1")));
        assert_eq!(moved.cmid, None);
    }
    hub.move_draft("dst", &own_did, "src", &RequestCaller::Person);
    assert_eq!(
        hub.inner.sessions.lock()["src"].drafts[0].origin,
        Some(DeliveryOrigin::peer("agent-1"))
    );
}

#[tokio::test]
async fn queued_row_edits_and_triggers_track_authority_and_clear_cmid_on_transition() {
    let (hub, mut rx) = hub_session("q-auth");
    let agent = RequestCaller::agent("peer-a");
    hub.set_status("q-auth", Status::Busy, None);

    hub.submit(
        "q-auth",
        "owner queued".into(),
        vec![],
        Some("owner-cmid-1".into()),
        &RequestCaller::Person,
    );
    let qid = hub.inner.sessions.lock()["q-auth"].queue[0].id.clone();
    hub.request_send_queued("q-auth", &qid, &agent);
    {
        let row = &hub.inner.sessions.lock()["q-auth"].queue[0];
        assert_eq!(row.origin, Some(DeliveryOrigin::Owner));
        assert_eq!(row.initiator, Some(DeliveryOrigin::peer("peer-a")));
        assert_eq!(row.cmid.as_deref(), Some("owner-cmid-1"));
    }
    hub.set_status("q-auth", Status::Running, None);
    hub.complete_turn("q-auth");
    let req = rx.try_recv().unwrap();
    assert_eq!(req.dispatch_origin, DeliveryOrigin::peer("peer-a"));
    drop(req);

    hub.set_status("q-auth", Status::Busy, None);
    hub.submit(
        "q-auth",
        "owner original".into(),
        vec![],
        Some("owner-cmid-2".into()),
        &RequestCaller::Person,
    );
    let qid2 = hub.inner.sessions.lock()["q-auth"].queue[0].id.clone();
    assert!(hub.client_edit("q-auth", &qid2, "agent rewrite".into(), vec![], &agent));
    {
        let row = &hub.inner.sessions.lock()["q-auth"].queue[0];
        assert_eq!(row.origin, Some(DeliveryOrigin::peer("peer-a")));
        assert_eq!(row.initiator, Some(DeliveryOrigin::peer("peer-a")));
        assert_eq!(row.cmid, None);
    }
    assert!(hub.client_edit(
        "q-auth",
        &qid2,
        "owner rewrite".into(),
        vec![],
        &RequestCaller::Person
    ));
    {
        let row = &hub.inner.sessions.lock()["q-auth"].queue[0];
        assert_eq!(row.origin, Some(DeliveryOrigin::Owner));
        assert_eq!(row.initiator, Some(DeliveryOrigin::Owner));
    }
    hub.inner.sessions.lock().get_mut("q-auth").unwrap().queue[0].cmid =
        Some("owner-cmid-3".into());
    hub.queued_to_draft("q-auth", &qid2, &agent);
    {
        let draft = &hub.inner.sessions.lock()["q-auth"].drafts[0];
        assert_eq!(draft.origin, Some(DeliveryOrigin::peer("peer-a")));
        assert_eq!(draft.initiator, None);
        assert_eq!(draft.cmid, None);
    }
}

#[tokio::test]
async fn legacy_queued_row_stays_non_owner_until_edited_by_owner() {
    let (hub, mut rx) = hub_session("legacy-q");
    hub.set_status("legacy-q", Status::Busy, None);
    hub.inner
        .sessions
        .lock()
        .get_mut("legacy-q")
        .unwrap()
        .queue
        .push(QueuedMessage {
            id: "q-old".into(),
            text: "from before c-auth".into(),
            content: vec![],
            cmid: Some("old-cmid".into()),
            schedule: None,
            origin: None,
            initiator: None,
        });
    hub.set_status("legacy-q", Status::Running, None);
    hub.request_send_queued("legacy-q", "q-old", &RequestCaller::Person);
    let req = rx.try_recv().unwrap();
    assert_eq!(req.dispatch_origin, DeliveryOrigin::cowboy("legacy_queue"));
}

#[tokio::test]
async fn retry_uses_proven_original_origin_and_guards_against_toctou() {
    let (hub, mut rx) = hub_session("retry-auth");
    let agent = RequestCaller::agent("agent-retry");

    hub.record_prompt_dispatch(
        "retry-auth",
        "cmid-o1".into(),
        DeliveryOrigin::Owner,
        DeliveryOrigin::Owner,
        10,
    );
    hub.push_tagged_from_worker(
        "retry-auth",
        Event::Update {
            update: json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "hello"}}),
        },
        Some("cmid-o1".into()),
    );
    hub.set_status("retry-auth", Status::Crashed, Some("err".into()));
    hub.retry_turn("retry-auth", &RequestCaller::Person, None);
    let req = rx.try_recv().unwrap();
    assert_eq!(req.dispatch_origin, DeliveryOrigin::Owner);
    let owner_retry_cmid = req.cmid.clone().unwrap();
    hub.record_prompt_dispatch(
        "retry-auth",
        owner_retry_cmid.clone(),
        req.dispatch_origin.clone(),
        req.trigger.clone(),
        15,
    );
    finish_turn(&hub, "retry-auth", req);

    hub.set_status("retry-auth", Status::Crashed, Some("err".into()));
    hub.retry_turn("retry-auth", &agent, None);
    let req2 = rx.try_recv().unwrap();
    assert_eq!(req2.dispatch_origin, DeliveryOrigin::peer("agent-retry"));
    assert_ne!(req2.cmid.as_deref(), Some(owner_retry_cmid.as_str()));
    finish_turn(&hub, "retry-auth", req2);

    hub.record_prompt_dispatch(
        "retry-auth",
        "cmid-p1".into(),
        DeliveryOrigin::peer("other"),
        DeliveryOrigin::peer("other"),
        20,
    );
    hub.push_tagged_from_worker(
        "retry-auth",
        Event::Update {
            update: json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "peer msg"}}),
        },
        Some("cmid-p1".into()),
    );
    hub.set_status("retry-auth", Status::Crashed, Some("err".into()));
    hub.retry_turn(
        "retry-auth",
        &RequestCaller::Person,
        Some(("cmid-o1".into(), DeliveryOrigin::Owner)),
    );
    let req3 = rx.try_recv().unwrap();
    assert_eq!(req3.dispatch_origin, DeliveryOrigin::peer("other"));
    finish_turn(&hub, "retry-auth", req3);

    hub.push_tagged(
        "retry-auth",
        Event::Update {
            update: json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "unrecorded"}}),
        },
        Some("cmid-unknown".into()),
    );
    hub.set_status("retry-auth", Status::Crashed, Some("err".into()));
    hub.retry_turn("retry-auth", &RequestCaller::Person, None);
    assert_eq!(
        rx.try_recv().unwrap().dispatch_origin,
        DeliveryOrigin::cowboy("legacy_queue")
    );
}

#[tokio::test]
async fn worker_echo_overrides_multiblock_and_fails_closed_when_unverified() {
    let (hub, _rx) = hub_session("echo-auth");
    hub.hydrate_prompt_dispatches(vec![PromptDispatchRecord {
        session_id: "echo-auth".into(),
        cmid: "peer-mb".into(),
        origin: DeliveryOrigin::peer("sender-1"),
        trigger: DeliveryOrigin::peer("sender-1"),
        dispatched_at_ms: 100,
        undelivered_retired: false,
        dispatch_seq: 0,
    }]);

    hub.push_tagged_from_worker(
        "echo-auth",
        Event::Update {
            update: json!({
                "sessionUpdate": "user_message_chunk",
                "promptOrigin": { "actor": "human", "source": "composer" },
                "content": { "type": "image", "mimeType": "image/png", "url": "/api/artifacts/a.png" }
            }),
        },
        Some("peer-mb".into()),
    );
    hub.push_tagged_from_worker(
        "echo-auth",
        Event::Update {
            update: json!({
                "sessionUpdate": "user_message_chunk",
                "promptOrigin": { "actor": "human", "source": "composer" },
                "content": { "type": "text", "text": "caption" }
            }),
        },
        None,
    );
    hub.push_tagged_from_worker(
        "echo-auth",
        Event::Update {
            update: json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "ack"}}),
        },
        None,
    );
    hub.push_tagged_from_worker(
        "echo-auth",
        Event::Update {
            update: json!({
                "sessionUpdate": "user_message_chunk",
                "promptOrigin": { "actor": "human", "source": "composer" },
                "content": { "type": "text", "text": "spoofed human" }
            }),
        },
        Some("unknown-worker-cmid".into()),
    );
    hub.push_tagged_from_worker(
        "echo-auth",
        Event::Update {
            update: json!({
                "sessionUpdate": "user_message_chunk",
                "promptOrigin": { "actor": "agent", "source": "runtime", "provider": "codex" },
                "autoResumed": true,
                "content": { "type": "text", "text": "<system-reminder>hi</system-reminder>" }
            }),
        },
        None,
    );

    let chunks = user_chunks(&hub, "echo-auth");
    assert_eq!(chunks.len(), 4);
    for (idx, expected_actor, expected_source) in [
        (0, "cowboy", "agent_message"),
        (1, "cowboy", "agent_message"),
        (2, "cowboy", "unverified_worker"),
        (3, "agent", "runtime"),
    ] {
        assert_eq!(
            chunks[idx].1["promptOrigin"]["actor"], expected_actor,
            "idx={idx}"
        );
        assert_eq!(
            chunks[idx].1["promptOrigin"]["source"], expected_source,
            "idx={idx}"
        );
        assert_eq!(chunks[idx].1["autoResumed"], true, "idx={idx}");
    }
    assert_eq!(
        hub.newest_owner_turn_in_retained_log("echo-auth")
            .unwrap()
            .newest_match,
        None
    );
}

#[tokio::test]
async fn send_now_by_owner_preserves_call_notice_and_peer_cmids_without_promoting() {
    let (hub, mut rx) = hub_session("send-now-preserve");
    hub.set_paused("send-now-preserve", true);

    hub.submit_call_notice("send-now-preserve", "cid-1", "review done".into());
    assert!(rx.try_recv().is_err(), "paused session must not auto-drain");
    let notice_qid = hub.inner.sessions.lock()["send-now-preserve"].queue[0]
        .id
        .clone();
    hub.request_send_queued("send-now-preserve", &notice_qid, &RequestCaller::Person);
    let req = rx.try_recv().unwrap();
    assert_eq!(req.cmid.as_deref(), Some("call-notice:cid-1"));
    assert_eq!(req.dispatch_origin, DeliveryOrigin::cowboy("managed_call"));
    assert_eq!(req.trigger, DeliveryOrigin::Owner);
    finish_turn(&hub, "send-now-preserve", req);

    hub.submit(
        "send-now-preserve",
        "from agent".into(),
        vec![],
        Some("op-peer-1".into()),
        &RequestCaller::agent("parent-7"),
    );
    assert!(rx.try_recv().is_err(), "paused session must not auto-drain");
    let peer_qid = hub.inner.sessions.lock()["send-now-preserve"].queue[0]
        .id
        .clone();
    hub.request_send_queued("send-now-preserve", &peer_qid, &RequestCaller::Person);
    let req2 = rx.try_recv().unwrap();
    assert_eq!(req2.cmid.as_deref(), Some("op-peer-1"));
    assert_eq!(req2.dispatch_origin, DeliveryOrigin::peer("parent-7"));
    assert_eq!(req2.trigger, DeliveryOrigin::Owner);
}

#[tokio::test]
async fn requeue_without_explicit_origin_never_silently_promotes_to_owner() {
    let (hub, mut rx) = hub_session("requeue-rec");

    hub.record_prompt_dispatch(
        "requeue-rec",
        "owner-orig".into(),
        DeliveryOrigin::Owner,
        DeliveryOrigin::Owner,
        100,
    );
    hub.requeue_prompt(
        "requeue-rec",
        "unverified requeue".into(),
        vec![],
        Some("owner-orig".into()),
        None,
        None,
    );
    hub.set_status("requeue-rec", Status::Running, None);
    hub.drain_head("requeue-rec", false, false, None);
    let resent = rx.try_recv().unwrap();
    assert_eq!(
        resent.dispatch_origin,
        DeliveryOrigin::cowboy("legacy_queue"),
        "blanket requeue without verified origin must fail closed as non-owner"
    );
}

#[tokio::test]
async fn requeued_prompt_resent_after_newer_echo_arms_on_restart_and_trimmed_echo_does_not() {
    let (hub, _rx) = hub_session("restart-resent");
    {
        let mut sessions = hub.inner.sessions.lock();
        let s = sessions.get_mut("restart-resent").unwrap();
        s.reached_start = true;
        s.next_seq = 2;
        s.restored_last_event_at_ms = Some(200);
        s.log.push(Envelope {
            session_id: "restart-resent".into(),
            seq: 1,
            event: Event::Update {
                update: json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": "notice B"}
                }),
            },
            cmid: Some("call-notice:b".into()),
        });
    }
    hub.hydrate_prompt_dispatches(vec![
        PromptDispatchRecord {
            session_id: "restart-resent".into(),
            cmid: "call-notice:b".into(),
            origin: DeliveryOrigin::cowboy("managed_call"),
            trigger: DeliveryOrigin::cowboy("managed_call"),
            dispatched_at_ms: 200,
            undelivered_retired: false,
            dispatch_seq: 0,
        },
        PromptDispatchRecord {
            session_id: "restart-resent".into(),
            cmid: "owner-a".into(),
            origin: DeliveryOrigin::Owner,
            trigger: DeliveryOrigin::Owner,
            dispatched_at_ms: 300,
            undelivered_retired: false,
            dispatch_seq: 0,
        },
    ]);
    hub.push_tagged_from_worker(
        "restart-resent",
        Event::Update {
            update: json!({
                "sessionUpdate": "user_message_chunk",
                "content": {"type": "text", "text": "owner A"}
            }),
        },
        Some("owner-a".into()),
    );
    let chunks = user_chunks(&hub, "restart-resent");
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[1].1["promptOrigin"]["actor"], "human");

    let (hub2, _rx2) = hub_session("restart-trimmed");
    {
        let mut sessions = hub2.inner.sessions.lock();
        let s = sessions.get_mut("restart-trimmed").unwrap();
        s.reached_start = false;
        s.next_seq = 51;
        s.restored_last_event_at_ms = Some(1_000);
        s.log.push(Envelope {
            session_id: "restart-trimmed".into(),
            seq: 50,
            event: Event::Update {
                update: json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "later reply"}
                }),
            },
            cmid: None,
        });
    }
    hub2.hydrate_prompt_dispatches(vec![PromptDispatchRecord {
        session_id: "restart-trimmed".into(),
        cmid: "owner-trimmed".into(),
        origin: DeliveryOrigin::Owner,
        trigger: DeliveryOrigin::Owner,
        dispatched_at_ms: 500,
        undelivered_retired: false,
        dispatch_seq: 0,
    }]);
    hub2.push_tagged_from_worker(
        "restart-trimmed",
        Event::Update {
            update: json!({
                "sessionUpdate": "user_message_chunk",
                "promptOrigin": {"actor": "human", "source": "composer"},
                "content": {"type": "text", "text": "replayed trimmed owner"}
            }),
        },
        Some("owner-trimmed".into()),
    );
    let chunks2 = user_chunks(&hub2, "restart-trimmed");
    assert_eq!(chunks2.len(), 1);
    assert_eq!(chunks2[0].1["promptOrigin"]["source"], "unverified_worker");
}

#[tokio::test]
async fn pre_provenance_submit_and_conflicting_record_never_arm_stale_pending_echo() {
    let (hub, mut rx) = hub_session("no-prearm");
    hub.submit(
        "no-prearm",
        "owner text".into(),
        vec![],
        Some("shared-cmid".into()),
        &RequestCaller::Person,
    );
    let _req = rx.try_recv().unwrap();
    assert!(
        !hub.inner.sessions.lock()["no-prearm"]
            .pending_echoes
            .contains_key("shared-cmid")
    );
    hub.record_prompt_dispatch(
        "no-prearm",
        "shared-cmid".into(),
        DeliveryOrigin::Owner,
        DeliveryOrigin::Owner,
        100,
    );
    assert!(
        hub.inner.sessions.lock()["no-prearm"]
            .pending_echoes
            .contains_key("shared-cmid")
    );
    hub.disarm_pending_echo("no-prearm", "shared-cmid");
    assert!(
        !hub.inner.sessions.lock()["no-prearm"]
            .pending_echoes
            .contains_key("shared-cmid")
    );
    hub.record_prompt_dispatch(
        "no-prearm",
        "shared-cmid".into(),
        DeliveryOrigin::peer("attacker"),
        DeliveryOrigin::peer("attacker"),
        200,
    );
    assert!(
        !hub.inner.sessions.lock()["no-prearm"]
            .pending_echoes
            .contains_key("shared-cmid")
    );
}

#[tokio::test]
async fn worker_cannot_replay_echoed_owner_cmid_or_append_excess_untagged_blocks() {
    let (hub, _rx) = hub_session("replay-guard");
    hub.record_prompt_dispatch(
        "replay-guard",
        "owner-2blocks".into(),
        DeliveryOrigin::Owner,
        DeliveryOrigin::Owner,
        100,
    );
    hub.set_pending_echo_blocks("replay-guard", "owner-2blocks", 2);

    hub.push_tagged_from_worker(
        "replay-guard",
        Event::Update {
            update: json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "b0"}}),
        },
        Some("owner-2blocks".into()),
    );
    hub.push_tagged_from_worker(
        "replay-guard",
        Event::Update {
            update: json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "b1"}}),
        },
        None,
    );
    hub.push_tagged_from_worker(
        "replay-guard",
        Event::Update {
            update: json!({
                "sessionUpdate": "user_message_chunk",
                "promptOrigin": { "actor": "human", "source": "composer" },
                "content": { "type": "text", "text": "forged b2" }
            }),
        },
        None,
    );
    hub.push_tagged_from_worker(
        "replay-guard",
        Event::Update {
            update: json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "reply"}}),
        },
        None,
    );
    let owner_turn_before_replay = hub
        .newest_owner_turn_in_retained_log("replay-guard")
        .unwrap()
        .newest_match
        .unwrap();
    hub.push_tagged_from_worker(
        "replay-guard",
        Event::Update {
            update: json!({
                "sessionUpdate": "user_message_chunk",
                "promptOrigin": { "actor": "human", "source": "composer" },
                "content": { "type": "text", "text": "replayed owner" }
            }),
        },
        Some("owner-2blocks".into()),
    );

    let (log, _) = hub.snapshot("replay-guard").unwrap();
    let user_envs: Vec<&Envelope> = log.iter().filter(|e| is_user_message_chunk(e)).collect();
    assert_eq!(user_envs.len(), 4);
    let Event::Update { update: u0 } = &user_envs[0].event else {
        panic!()
    };
    let Event::Update { update: u1 } = &user_envs[1].event else {
        panic!()
    };
    let Event::Update { update: u2 } = &user_envs[2].event else {
        panic!()
    };
    let Event::Update { update: u3 } = &user_envs[3].event else {
        panic!()
    };
    assert_eq!(u0["promptOrigin"]["actor"], "human");
    assert_eq!(u1["promptOrigin"]["actor"], "human");
    assert_eq!(u2["promptOrigin"]["source"], "unverified_worker");
    assert_eq!(u2["autoResumed"], true);
    assert_eq!(u3["promptOrigin"]["source"], "unverified_worker");
    assert_eq!(u3["autoResumed"], true);
    assert_eq!(user_envs[3].cmid, None);
    assert_eq!(
        hub.newest_owner_turn_in_retained_log("replay-guard")
            .unwrap()
            .newest_match,
        Some(owner_turn_before_replay)
    );
}
