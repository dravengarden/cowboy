use super::*;

#[test]
fn restore_rehydrates_folder_placements_into_the_live_tree() {
    let hub = Hub::new();
    hub.restore_session_folders(vec![crate::session_folders::SessionFolder {
        defaults: Default::default(),
        id: "f-a".to_owned(),
        owner_user_id: None,
        name: "A".to_owned(),
        parent: None,
        position: 0,
        project: None,
        trashed_at_ms: None,
    }]);
    let mut filed = restored_busy("filed");
    filed.folder_id = Some("f-a".to_owned());
    let mut loose = restored_busy("loose");
    loose.folder_id = Some("f-gone".to_owned());
    hub.restore_reconciling_runtime(vec![filed, loose]);
    let value = hub.sync_value("folders");
    assert_eq!(value["folders"][0]["id"], "f-a");
    assert_eq!(value["placement"]["filed"], "f-a");
    // A placement into a vanished folder degrades to the root.
    assert!(value["placement"].get("loose").is_none());
}

#[test]
fn worker_replays_do_not_reset_the_idle_clock() {
    let hub = Hub::new();
    let mut idle = restored_busy("idle");
    idle.meta.status = Status::Exited;
    idle.last_event_at_ms = Some(now_ms() - 2 * 60 * 60 * 1000);
    hub.restore_with_workers(vec![idle], &[]);
    let long = std::time::Duration::from_secs(7_000);
    // A reconnect re-asserts status and replays worker metadata.
    hub.set_status("idle", Status::Running, None);
    hub.push(
        "idle",
        Event::Update {
            update: serde_json::json!({"sessionUpdate": "available_commands_update", "availableCommands": []}),
        },
    );
    assert!(hub.session_idle_for("idle").unwrap() >= long);
    // Agent output is real use.
    hub.push(
        "idle",
        Event::Update {
            update: serde_json::json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hi"}}),
        },
    );
    assert!(hub.session_idle_for("idle").unwrap() < std::time::Duration::from_secs(5));
    for kind in crate::core::NON_ACTIVITY_SESSION_UPDATES {
        assert!(crate::core::NON_ACTIVITY_SESSION_UPDATES_SQL.contains(&format!("'{kind}'")));
    }
    assert_eq!(
        crate::core::NON_ACTIVITY_SESSION_UPDATES_SQL
            .matches('\'')
            .count(),
        2 * crate::core::NON_ACTIVITY_SESSION_UPDATES.len()
    );
}

#[test]
fn restore_keeps_measuring_idle_time_from_the_newest_durable_event() {
    let hub = Hub::new();
    let mut idle = restored_busy("idle");
    idle.meta.status = Status::Exited;
    idle.last_event_at_ms = Some(now_ms() - 2 * 60 * 60 * 1000);
    let mut unknown = restored_busy("unknown");
    unknown.meta.status = Status::Exited;
    let mut future = restored_busy("future");
    future.meta.status = Status::Exited;
    future.last_event_at_ms = Some(now_ms() + 60_000);
    hub.restore_with_workers(vec![idle, unknown, future], &[]);
    let idle_for = hub.session_idle_for("idle").unwrap();
    assert!(
        idle_for >= std::time::Duration::from_secs(7_190),
        "{idle_for:?}"
    );
    assert!(
        idle_for <= std::time::Duration::from_secs(7_300),
        "{idle_for:?}"
    );
    // No record, or a clock ahead of ours, conservatively means just active.
    assert!(hub.session_idle_for("unknown").unwrap() < std::time::Duration::from_secs(5));
    assert!(hub.session_idle_for("future").unwrap() < std::time::Duration::from_secs(5));
}

pub(super) fn restored_busy(id: &str) -> RestoredSession {
    RestoredSession {
        meta: SessionMeta {
            continued_from: None,
            forked_from: None,
            id: id.to_owned(),
            provider: "codex".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "hawk".to_owned(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: "/tmp".to_owned(),
            title: "test".to_owned(),
            status: Status::Busy,
            origin: SessionOrigin::Web,
            agent_session_id: Some("agent-1".to_owned()),
            paused: false,
            closing: false,
            system: false,
            context_used: 0,
            context_size: 0,
            usage: None,
            background_tasks: 0,
            supports_current_turn: false,
            provider_update: None,
            provider_update_available: None,
            resource_wait: None,
            next_schedule_ms: None,
            owner_user_id: None,
            owner_username: None,
            trashed_at_ms: None,
        },
        log: Vec::new(),
        event_count: 0,
        reached_start: true,
        next_seq: 0,
        queue: Vec::new(),
        drafts: Vec::new(),
        config_options: None,
        config_preferences: serde_json::json!({}),
        mobile_review_state: serde_json::Value::Null,
        folder_id: None,
        last_event_at_ms: None,
    }
}

pub(super) fn worker_snapshot(session_id: &str, worker_epoch: &str) -> WorkerSnapshot {
    WorkerSnapshot {
        session_id: session_id.to_owned(),
        worker_epoch: worker_epoch.to_owned(),
        generation: "gen-1".to_owned(),
        executable: None,
        launch: None,
        state: WorkerState::Busy,
        agent_session_id: Some("agent-1".to_owned()),
        native_thread_materialized: None,
        current_turn_id: Some("turn-1".to_owned()),
        supports_current_turn: false,
        delivery_controls: crate::runtime_wire::DeliveryControls::default(),
        supports_subagent_cancel: false,
        last_runtime_seq: 7,
        pending_permissions: Vec::new(),
        config_options: None,
        context_used: None,
        context_size: None,
        pending_prompt_count: 0,
        drain_requested: false,
        exit_detail: None,
        background_tasks: None,
        native_cycle_id: None,
        incarnation: None,
    }
}

fn pending(id: &str, text: &str, cmid: &str) -> QueuedMessage {
    QueuedMessage {
        id: id.to_owned(),
        text: text.to_owned(),
        content: Vec::new(),
        cmid: Some(cmid.to_owned()),
        schedule: None,
    }
}

#[test]
fn placeholder_cannot_settle_restored_busy_turn() {
    let hub = Hub::new();
    hub.restore_reconciling_runtime(vec![restored_busy("session-1")]);

    assert_eq!(hub.status("session-1"), Some(Status::Busy));
    assert!(!hub.accept_runtime_snapshot(&worker_snapshot("session-1", "broker-session-1")));
    assert_eq!(hub.status("session-1"), Some(Status::Busy));

    assert_eq!(
        hub.finalize_runtime_reconciliation(),
        vec!["session-1".to_owned()]
    );
    assert_eq!(hub.status("session-1"), Some(Status::Interrupted));
    assert!(hub.finalize_runtime_reconciliation().is_empty());
}

#[test]
fn connected_worker_adopts_restored_busy_turn() {
    let hub = Hub::new();
    hub.restore_reconciling_runtime(vec![restored_busy("session-2")]);

    assert!(hub.accept_runtime_snapshot(&worker_snapshot("session-2", "worker-epoch-2")));
    assert!(hub.finalize_runtime_reconciliation().is_empty());
    assert_eq!(hub.status("session-2"), Some(Status::Busy));
}

#[test]
fn idle_worker_cannot_silently_settle_a_restored_busy_turn() {
    let hub = Hub::new();
    hub.restore_reconciling_runtime(vec![restored_busy("session-idle")]);
    let mut idle = worker_snapshot("session-idle", "worker-epoch-idle");
    idle.state = WorkerState::Running;
    idle.current_turn_id = None;

    assert!(hub.accept_runtime_snapshot(&idle));
    assert_eq!(hub.status("session-idle"), Some(Status::Interrupted));
    assert!(!hub.session_has_in_flight_prompt("session-idle"));
    let (log, _) = hub.snapshot("session-idle").expect("snapshot");
    assert!(log.iter().any(|envelope| matches!(
        envelope.event,
        Event::Lifecycle {
            status: Status::Interrupted,
            ..
        }
    )));
    hub.project_runtime_status("session-idle", Status::Running, None);
    assert_eq!(hub.status("session-idle"), Some(Status::Interrupted));
    assert!(hub.finalize_runtime_reconciliation().is_empty());
}

#[test]
fn restore_heals_a_preference_retired_by_the_persisted_option_snapshot() {
    let health = Arc::new(PersistenceHealth::default());
    let (sink, mut rx) = StoreSink::channel(4, health);
    let hub = Hub::with_store(Some(sink));
    let mut restored = restored_busy("session-stale-config");
    restored.config_preferences = serde_json::json!({
        "model": "gpt-5.3-codex-spark",
        "reasoning_effort": "max",
    });
    restored.config_options = Some(serde_json::json!([
        {
            "id": "model",
            "currentValue": "gpt-5.3-codex-spark",
            "options": [{"value": "gpt-5.3-codex-spark"}],
        },
        {
            "id": "reasoning_effort",
            "currentValue": "low",
            "options": [
                {"value": "low"},
                {"value": "medium"},
                {"value": "high"},
                {"value": "xhigh"},
            ],
        },
    ]));

    hub.restore_reconciling_runtime(vec![restored]);

    assert_eq!(
        hub.config_preferences("session-stale-config")
            .and_then(|value| value.get("reasoning_effort").cloned()),
        Some(serde_json::json!("low"))
    );
    assert!(matches!(
        rx.try_recv(),
        Ok(StoreWrite::UpdateConfigPreferences {
            session_id,
            preferences,
        }) if session_id == "session-stale-config"
            && preferences["reasoning_effort"] == serde_json::json!("low")
    ));
}

#[test]
fn immediate_restore_does_not_treat_broker_placeholder_as_live_owner() {
    let hub = Hub::new();
    hub.restore_with_workers(
        vec![restored_busy("session-3")],
        &[worker_snapshot("session-3", "broker-session-3")],
    );

    assert_eq!(hub.status("session-3"), Some(Status::Interrupted));
}

#[test]
fn restore_discards_retired_continuations_but_keeps_user_work() {
    let hub = Hub::new();
    let mut restored = restored_busy("session-4");
    restored.queue = vec![
        pending("q1", "legacy queue continuation", "__cont__old"),
        pending("q2", "user queued message", "user-cmid"),
    ];
    restored.drafts = vec![
        pending("q3", "legacy draft continuation", "__cont__draft"),
        pending("q4", "user draft", "draft-cmid"),
    ];

    hub.restore_with_workers(vec![restored], &[]);

    assert_eq!(hub.status("session-4"), Some(Status::Interrupted));
    let Some(Outbound::SyncPatch { value, .. }) = hub.queue_resync("session-4") else {
        panic!("queue resync missing");
    };
    assert_eq!(value["queue"].as_array().unwrap().len(), 1);
    assert_eq!(value["queue"][0]["text"], "user queued message");
    assert_eq!(value["drafts"].as_array().unwrap().len(), 1);
    assert_eq!(value["drafts"][0]["text"], "user draft");
}
