//! Clear conversation fences the cleared context's workers: a late frame from
//! a worker epoch admitted before Clear cannot repopulate the fresh context's
//! usage, native id, configuration, transcript or pending actions.

use super::tests::snapshot;
use super::*;

fn session(hub: &Hub) {
    hub.create_local_session(
        "s".to_owned(),
        "codex".to_owned(),
        "/tmp".to_owned(),
        "test".to_owned(),
        crate::model::SessionOrigin::Web,
        false,
    );
}

fn worker(epoch: &str, state: WorkerState, agent: Option<&str>) -> WorkerSnapshot {
    let mut worker = snapshot("s");
    worker.worker_epoch = epoch.to_owned();
    worker.state = state;
    worker.current_turn_id = None;
    worker.agent_session_id = agent.map(str::to_owned);
    if let Some(launch) = worker.launch.as_mut() {
        launch.agent_session_id = agent.map(str::to_owned);
    }
    worker
}

fn clear(hub: &Hub) {
    hub.prepare_context_reset("s");
    hub.clear_transcript("s");
    hub.mark_context_cleared("s");
}

fn late_frames() -> Vec<RuntimeEvent> {
    vec![
        RuntimeEvent::ContextUsage {
            used: 90_000,
            size: 200_000,
            raw: serde_json::Value::Null,
            observed_at_ms: 1,
        },
        RuntimeEvent::AgentSessionId {
            agent_session_id: "agent-1".to_owned(),
            materialized: false,
        },
        RuntimeEvent::ConfigOptions {
            options: serde_json::json!([{"id": "late", "currentValue": "x", "options": []}]),
        },
        RuntimeEvent::Update {
            update: serde_json::json!({"sessionUpdate": "plan", "entries": []}),
            cmid: None,
        },
        RuntimeEvent::PermissionRequest {
            request_id: "late-permission".to_owned(),
            tool_call: serde_json::json!({"kind": "execute"}),
            options: serde_json::json!([]),
        },
    ]
}

async fn deliver(runtime: &RemoteRuntime, epoch: &str, first_seq: u64, events: Vec<RuntimeEvent>) {
    for (offset, event) in (0u64..).zip(events) {
        handle_frame(
            &runtime.shared,
            Frame::WorkerEvent {
                session_id: "s".to_owned(),
                worker_epoch: epoch.to_owned(),
                runtime_seq: first_seq + offset,
                event,
                diagnostics: Vec::new(),
            },
            &mut tokio::io::sink(),
        )
        .await
        .expect("worker event");
    }
}

fn assert_fresh(hub: &Hub, events_after_clear: u64, options: Option<&serde_json::Value>) {
    let info = hub.session_info("s").expect("session");
    assert_eq!(
        info.meta.context_used, 0,
        "late usage repopulated the context"
    );
    assert_eq!(info.meta.context_size, 0);
    assert!(info.meta.usage.is_none());
    assert_eq!(
        info.meta.agent_session_id, None,
        "late frame restored the cleared native id"
    );
    assert_eq!(
        info.event_count, events_after_clear,
        "late frame wrote into the cleared transcript"
    );
    assert!(
        info.meta.attention.is_empty(),
        "late permission became pending"
    );
    assert_eq!(hub.reported_config_options("s").as_ref(), options);
}

fn usage(used: u64) -> RuntimeEvent {
    RuntimeEvent::ContextUsage {
        used,
        size: 200_000,
        raw: serde_json::Value::Null,
        observed_at_ms: 2,
    }
}

fn agent(id: &str) -> RuntimeEvent {
    RuntimeEvent::AgentSessionId {
        agent_session_id: id.to_owned(),
        materialized: false,
    }
}

#[tokio::test]
async fn a_dormant_clear_ignores_late_frames_from_the_old_worker() {
    let hub = Hub::new();
    session(&hub);
    let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
    merge_worker_snapshots(
        &runtime.shared,
        vec![worker("old", WorkerState::Exited, Some("agent-1"))],
    )
    .await;
    deliver(&runtime, "old", 1, vec![usage(50_000)]).await;
    let before = hub.session_info("s").unwrap().meta;
    assert_eq!(before.agent_session_id.as_deref(), Some("agent-1"));
    assert_eq!(before.context_used, 50_000);

    // A dormant session has no worker to recycle, so Clear sends nothing.
    clear(&hub);
    let after_clear = hub.session_info("s").unwrap().event_count;
    let options = hub.reported_config_options("s");
    deliver(&runtime, "old", 2, late_frames()).await;
    assert_fresh(&hub, after_clear, options.as_ref());

    // The Machine re-reports the old worker on reconnect: still the old context.
    let mut reported = worker("old", WorkerState::Exited, Some("agent-1"));
    reported.context_used = Some(80_000);
    reported.context_size = Some(200_000);
    merge_worker_snapshots(&runtime.shared, vec![reported]).await;
    assert_fresh(&hub, after_clear, options.as_ref());
}

#[tokio::test]
async fn a_live_clear_ignores_the_replaced_worker_and_admits_its_replacement() {
    let hub = Hub::new();
    session(&hub);
    let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
    merge_worker_snapshots(
        &runtime.shared,
        vec![worker("old", WorkerState::Running, Some("agent-1"))],
    )
    .await;
    clear(&hub);
    // The replacement reports after the reset acknowledgement.
    merge_worker_snapshots(
        &runtime.shared,
        vec![worker("new", WorkerState::Running, None)],
    )
    .await;
    let after_clear = hub.session_info("s").unwrap().event_count;
    let options = hub.reported_config_options("s");
    deliver(&runtime, "old", 9, late_frames()).await;
    assert_fresh(&hub, after_clear, options.as_ref());

    deliver(&runtime, "new", 1, vec![agent("agent-2"), usage(1_000)]).await;
    let meta = hub.session_info("s").unwrap().meta;
    assert_eq!(meta.agent_session_id.as_deref(), Some("agent-2"));
    assert_eq!(meta.context_used, 1_000);
}

#[tokio::test]
async fn an_unobserved_worker_on_the_cleared_native_context_is_fenced() {
    // The Machine was disconnected when the user cleared: this Controller
    // never saw the old worker's epoch, only the native id it ran.
    let hub = Hub::new();
    session(&hub);
    hub.set_agent_session_id("s", "agent-1".to_owned());
    let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
    clear(&hub);
    let after_clear = hub.session_info("s").unwrap().event_count;
    let options = hub.reported_config_options("s");
    merge_worker_snapshots(
        &runtime.shared,
        vec![worker("old", WorkerState::Running, Some("agent-1"))],
    )
    .await;
    deliver(&runtime, "old", 3, late_frames()).await;
    assert_fresh(&hub, after_clear, options.as_ref());
}

#[tokio::test]
async fn a_restarted_controller_accepts_the_current_context_replay() {
    // A Controller restart forgets the fence; the current worker's replay,
    // which starts past the acknowledged prefix, must project as before.
    let hub = Hub::new();
    session(&hub);
    let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
    merge_worker_snapshots(
        &runtime.shared,
        vec![worker("new", WorkerState::Running, Some("agent-2"))],
    )
    .await;
    deliver(&runtime, "new", 7, vec![usage(3_000), agent("agent-2")]).await;
    let meta = hub.session_info("s").unwrap().meta;
    assert_eq!(meta.agent_session_id.as_deref(), Some("agent-2"));
    assert_eq!(meta.context_used, 3_000);
}

#[tokio::test]
async fn a_fenced_worker_owns_nothing_but_still_retires_its_turn() {
    let hub = Hub::new();
    session(&hub);
    let runtime = RemoteRuntime::for_test(hub.clone(), Vec::new());
    merge_worker_snapshots(
        &runtime.shared,
        vec![worker("old", WorkerState::Running, Some("agent-1"))],
    )
    .await;
    runtime.prompt_traced("s", Vec::new(), Some("c1".into()), None);
    let turn = runtime
        .pending_for_test()
        .into_iter()
        .find_map(|command| match command {
            CoreCommand::Prompt { turn_id, .. } => Some(turn_id),
            _ => None,
        })
        .expect("pending prompt");
    clear(&hub);
    merge_worker_snapshots(
        &runtime.shared,
        vec![worker("new", WorkerState::Running, None)],
    )
    .await;
    // A late re-report of the old worker, and its placeholder restating the
    // discarded launch, neither take the cache nor restore the native id.
    let mut placeholder = worker("broker-s", WorkerState::Exited, Some("agent-1"));
    placeholder.current_turn_id = None;
    handle_frame(
        &runtime.shared,
        Frame::Snapshot {
            worker: Box::new(worker("old", WorkerState::Running, Some("agent-1"))),
        },
        &mut tokio::io::sink(),
    )
    .await
    .expect("snapshot");
    merge_worker_snapshots(&runtime.shared, vec![placeholder]).await;
    assert_eq!(
        runtime
            .shared
            .workers
            .lock()
            .get("s")
            .map(|worker| worker.worker_epoch.clone())
            .as_deref(),
        Some("new")
    );
    assert_eq!(hub.session_info("s").unwrap().meta.agent_session_id, None);
    assert_eq!(hub.status("s"), Some(Status::Running));

    deliver(
        &runtime,
        "old",
        9,
        vec![RuntimeEvent::TurnEnded {
            turn_id: turn.clone(),
            stop_reason: "EndTurn".to_owned(),
            usage: None,
        }],
    )
    .await;
    assert!(
        !runtime.pending_for_test().iter().any(|command| matches!(
            command,
            CoreCommand::Prompt { turn_id, .. } if *turn_id == turn
        )),
        "a fenced turn ending left its prompt pending"
    );
}
