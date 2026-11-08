use super::*;

#[test]
fn publishing_preparation_retains_target_verified_source_and_identity_fences() {
    use crate::execution_environment::{ExecutionBinding, PreparationV1};
    let hub = hub_with_session("preparing");
    let ready = crate::execution_environment::fixture();
    let binding = ready.decode().unwrap();
    let intent = PreparationV1 {
        schema: 1,
        phase: "preparing".into(),
        runtime: binding.runtime.clone(),
        machine_id: binding.environment.machine_id.clone(),
        workspace_id: binding.workspace.id.clone(),
        source_path: "/logical/source".into(),
        executor_digest: binding.environment.executor_digest.clone(),
    };
    let expected = ExecutionBinding::from_record(serde_json::to_value(intent).unwrap());
    {
        let mut sessions = hub.inner.sessions.lock();
        let meta = &mut sessions.get_mut("preparing").unwrap().meta;
        meta.machine_id = binding.runtime.machine_id.clone();
        meta.cwd = binding.runtime.cwd.clone();
        meta.execution_binding = Some(expected.clone());
        meta.provider_version = "1.0.0".into();
        meta.provider_generation_digest = "fixture".into();
        meta.provider_behavior = Some(serde_json::from_value(serde_json::json!({
            "schema_version": 1, "permission": "acp_config_full_access_v1",
            "session": "portable_v1", "turn_end": "portable_v1", "configuration": "acp_config_options_v1",
            "execution": {"interface": "jsonrpc_v1", "executor_digests": [binding.environment.executor_digest]}
        })).unwrap());
    }
    assert!(
        hub.accept_execution_binding("preparing", &expected, ready.clone(), None)
            .is_err()
    );
    assert!(
        hub.accept_execution_binding("preparing", &expected, ready.clone(), Some("/unrelated"))
            .is_err()
    );
    let mut wrong = ready.record().clone();
    wrong["environment"]["executor_digest"] =
        serde_json::json!(format!("sha256:{}", "f".repeat(64)));
    assert!(
        hub.accept_execution_binding(
            "preparing",
            &expected,
            ExecutionBinding::from_record(wrong),
            Some(&binding.workspace.source_path)
        )
        .is_err()
    );
    hub.accept_execution_binding(
        "preparing",
        &expected,
        ready.clone(),
        Some(&binding.workspace.source_path),
    )
    .unwrap();
    assert_eq!(
        hub.session_info("preparing")
            .unwrap()
            .meta
            .execution_binding,
        Some(ready)
    );
}

#[test]
fn a_replayed_end_of_the_restored_turn_undoes_the_restart_interruption() {
    let hub = hub_with_session("restored");
    hub.set_status("restored", Status::Busy, None);
    hub.record_restart_interruption("restored");
    assert_eq!(hub.status("restored"), Some(Status::Interrupted));
    // The idle snapshot alone keeps the interruption visible.
    hub.project_runtime_status("restored", Status::Running, None);
    assert_eq!(hub.status("restored"), Some(Status::Interrupted));
    hub.settle_restart_interruption("restored");
    assert_eq!(hub.status("restored"), Some(Status::Running));
    // Only once: a later Interrupted state has its own cause.
    hub.set_status("restored", Status::Interrupted, Some("other".into()));
    hub.settle_restart_interruption("restored");
    assert_eq!(hub.status("restored"), Some(Status::Interrupted));
}

#[test]
fn a_newer_turn_is_not_relabelled_by_a_late_settle() {
    let hub = hub_with_session("newer");
    hub.set_status("newer", Status::Busy, None);
    hub.record_restart_interruption("newer");
    hub.set_status("newer", Status::Busy, None);
    hub.settle_restart_interruption("newer");
    assert_eq!(hub.status("newer"), Some(Status::Busy));
}

#[test]
fn watchdog_revision_does_not_claim_the_replacement_turn() {
    let hub = hub_with_session("status-cas");
    hub.set_status("status-cas", Status::Busy, None);
    let cancelled_turn = hub.status_revision("status-cas").expect("busy revision");

    // The cancelled turn ends and the force-pushed replacement starts. Its
    // status is also Busy, but it is not the turn the watchdog was armed for.
    hub.set_status("status-cas", Status::Running, None);
    hub.set_status("status-cas", Status::Busy, None);

    assert!(!hub.set_status_if_revision(
        "status-cas",
        Some(cancelled_turn),
        Status::Interrupted,
        Some("watchdog".to_owned()),
    ));
    assert_eq!(hub.status("status-cas"), Some(Status::Busy));
}

#[test]
fn a_restated_crash_is_not_appended_as_a_new_one() {
    let hub = hub_with_session("crashed-once");
    let crashes = |hub: &Hub| {
        let (log, _) = hub.snapshot("crashed-once").expect("snapshot");
        log.iter()
            .filter(|envelope| {
                matches!(
                    envelope.event,
                    Event::Lifecycle {
                        status: Status::Crashed,
                        ..
                    }
                )
            })
            .count()
    };
    hub.project_runtime_status("crashed-once", Status::Crashed, Some("first".to_owned()));
    hub.project_runtime_status("crashed-once", Status::Crashed, None);
    hub.project_runtime_status("crashed-once", Status::Crashed, Some("first".to_owned()));
    assert_eq!(crashes(&hub), 1);
    // A different failure, or a crash after a restart, is a new fact.
    hub.project_runtime_status("crashed-once", Status::Crashed, Some("second".to_owned()));
    assert_eq!(crashes(&hub), 2);
    hub.set_status("crashed-once", Status::Starting, None);
    hub.project_runtime_status("crashed-once", Status::Crashed, None);
    assert_eq!(crashes(&hub), 3);
}

#[test]
fn latest_crash_detail_survives_a_detail_less_runtime_projection() {
    let hub = hub_with_session("terminal-crash");
    hub.set_status(
        "terminal-crash",
        Status::Crashed,
        Some("provider retired this login flow".to_owned()),
    );
    hub.set_status("terminal-crash", Status::Crashed, None);
    assert_eq!(
        hub.latest_crash_detail("terminal-crash").as_deref(),
        Some("provider retired this login flow")
    );

    hub.set_status("terminal-crash", Status::Starting, None);
    hub.set_status("terminal-crash", Status::Crashed, None);
    assert_eq!(hub.latest_crash_detail("terminal-crash"), None);
}

#[test]
fn watchdog_revision_ignores_duplicate_busy_snapshots_on_a_stuck_turn() {
    let hub = hub_with_session("status-stuck");
    hub.set_status("status-stuck", Status::Busy, None);
    let stuck_turn = hub.status_revision("status-stuck").expect("busy revision");
    hub.set_status("status-stuck", Status::Busy, None);

    assert!(hub.set_status_if_revision(
        "status-stuck",
        Some(stuck_turn),
        Status::Interrupted,
        None,
    ));
    assert_eq!(hub.status("status-stuck"), Some(Status::Interrupted));
}

fn hub_with_session(id: &str) -> Hub {
    let hub = Hub::new();
    hub.create_local_session(
        id.to_owned(),
        "claude-code".to_owned(),
        "/tmp".to_owned(),
        "t".to_owned(),
        SessionOrigin::Web,
        false,
    );
    hub
}

#[test]
fn mobile_review_sync_is_session_scoped_and_idempotent() {
    let hub = hub_with_session("mobile");
    hub.sync_apply(
        "mobile-review:mobile",
        "m1".to_owned(),
        "open",
        &serde_json::json!({"path": "strategies/README.md"}),
    )
    .unwrap();
    hub.sync_apply(
        "mobile-review:mobile",
        "m1".to_owned(),
        "open",
        &serde_json::json!({"path": "ignored/by-retry.rs"}),
    )
    .unwrap();
    hub.sync_apply(
        "mobile-review:mobile",
        "m2".to_owned(),
        "setPinned",
        &serde_json::json!({"path": "strategies/README.md", "pinned": true}),
    )
    .unwrap();
    hub.sync_apply(
        "mobile-review:mobile",
        "m3".to_owned(),
        "markReviewed",
        &serde_json::json!({"key": "combined:strategies/README.md", "revision": "abc123"}),
    )
    .unwrap();
    hub.sync_apply(
        "mobile-review:mobile",
        "m4".to_owned(),
        "setPosition",
        &serde_json::json!({
            "path": "strategies/README.md",
            "line": 47,
            "revision": "abc123"
        }),
    )
    .unwrap();

    let value = hub.sync_value("mobile-review:mobile");
    assert_eq!(value["mode"], "files");
    assert_eq!(value["active"], "strategies/README.md");
    assert_eq!(value["tabs"].as_array().unwrap().len(), 1);
    assert_eq!(value["tabs"][0]["pinned"], true);
    assert_eq!(value["progress"]["combined:strategies/README.md"], "abc123");
    assert_eq!(value["positions"]["strategies/README.md"]["line"], 47);
    assert_eq!(
        value["positions"]["strategies/README.md"]["revision"],
        "abc123"
    );
}

#[test]
fn review_directory_overrides_persist_null_and_inherit_without_touching_execution() {
    let hub = hub_with_session("zed-defaults");
    let state = "mobile-review:zed-defaults";
    for (index, args) in [
        serde_json::json!({"directory":"workspace::hawk::cowboy"}),
        serde_json::json!({"directory":null}),
        serde_json::json!({"inherit":true}),
    ]
    .into_iter()
    .enumerate()
    {
        hub.sync_apply(state, format!("zed-{index}"), "setDirectory", &args)
            .unwrap();
        let value = hub.sync_value(state);
        let restored = super::MobileReviewState::from_stored(value.clone());
        assert_eq!(serde_json::to_value(restored).unwrap(), value);
        assert_eq!(value.get("directory"), args.get("directory"));
        assert_eq!(hub.session_info("zed-defaults").unwrap().meta.cwd, "/tmp");
    }
    assert!(
        hub.sync_apply(
            state,
            "retry-invalid".into(),
            "setDirectory",
            &serde_json::json!({"directory":"/etc"})
        )
        .is_err()
    );
    // A rejected mutation must not burn the id and acknowledge a later retry.
    assert!(
        hub.sync_apply(
            state,
            "retry-invalid".into(),
            "setDirectory",
            &serde_json::json!({"directory":"/etc"})
        )
        .is_err()
    );
    hub.sync_apply(
        state,
        "retry-invalid".into(),
        "setDirectory",
        &serde_json::json!({"directory":"project"}),
    )
    .unwrap();
    assert_eq!(hub.sync_value(state)["directory"], "project");
}

#[test]
fn review_directory_agent_compare_replay_and_owner_scope() {
    use crate::session_folders::FolderActor;
    let hub = hub_with_session("zed-agent");
    let actor = FolderActor::local();
    let mut input: crate::review_directory::Change = serde_json::from_value(
        serde_json::json!({"scope":"zed","expected":{},"change":{"directory":"project"}}),
    )
    .unwrap();
    hub.review_directory_update(&actor, "zed-agent", "one", &input)
        .unwrap();
    hub.review_directory_update(&actor, "zed-agent", "one", &input)
        .unwrap();
    assert!(hub.review_default_receipt("one"));
    // The canonical spelling replays the same decision, not a conflict.
    let canonical = crate::review_directory::Change {
        scope: "code".into(),
        folder_id: None,
        expected: input.expected.clone(),
        change: input.change.clone(),
    };
    hub.review_directory_update(&actor, "zed-agent", "one", &canonical)
        .unwrap();
    let unknown = crate::review_directory::Change {
        scope: "review".into(),
        ..canonical
    };
    assert_eq!(
        hub.review_directory_update(&actor, "zed-agent", "one", &unknown),
        Err("unknown_scope")
    );
    let remembered = std::mem::take(&mut *hub.inner.review_default_receipts.lock());
    assert_eq!(
        hub.review_directory_update(&actor, "zed-agent", "one", &input),
        Err("receipt_expired")
    );
    *hub.inner.review_default_receipts.lock() = remembered;
    assert_eq!(
        hub.review_directory_update(&actor, "zed-agent", "two", &input),
        Err("revision_conflict")
    );
    input.change = serde_json::json!({"directory":null});
    assert_eq!(
        hub.review_directory_update(&actor, "zed-agent", "one", &input),
        Err("operation_conflict")
    );
    input.expected = serde_json::json!({"directory":"project"});
    assert_eq!(
        hub.review_directory_update(
            &FolderActor {
                user_id: Some("other".into()),
                sees_all: false
            },
            "zed-agent",
            "two",
            &input
        ),
        Err("permission_denied")
    );
    hub.review_directory_update(&actor, "zed-agent", "two", &input)
        .unwrap();
    hub.sync_apply(
        "folders",
        "folder-create".into(),
        "create",
        &serde_json::json!({"id":"zed-folder","name":"Zed"}),
    )
    .unwrap();
    input.folder_id = Some("zed-folder".into());
    input.expected = serde_json::json!({});
    hub.review_directory_update(&actor, "zed-agent", "three", &input)
        .unwrap();
    assert_eq!(
        hub.folder_defaults(&actor, "zed-folder").unwrap().values
            [&crate::session_folders::defaults::DefaultKey::ReviewDirectory],
        None
    );
    assert!(
        hub.review_directory_update(
            &FolderActor {
                user_id: Some("other".into()),
                sees_all: false
            },
            "zed-agent",
            "four",
            &input
        )
        .is_err()
    );
}

#[test]
fn project_language_servers_follow_the_session_directory_default() {
    let hub = hub_with_session("ls");
    assert!(hub.language_servers_allowed("ls"), "the top level means on");
    assert!(!hub.language_servers_allowed("missing"));
    let apply = |op: &str, mutation: &str, args: serde_json::Value| {
        hub.sync_apply("folders", op.into(), mutation, &args)
            .unwrap();
    };
    apply(
        "ls-parent",
        "create",
        serde_json::json!({"id":"ls-parent","name":"P","defaults":{"language_servers":"off"}}),
    );
    apply(
        "ls-child",
        "create",
        serde_json::json!({"id":"ls-child","name":"C","parent":"ls-parent"}),
    );
    apply(
        "ls-place",
        "place",
        serde_json::json!({"session_ids":["ls"],"folder":"ls-child"}),
    );
    assert!(!hub.language_servers_allowed("ls"), "inherited off");
    apply(
        "ls-on",
        "defaults",
        serde_json::json!({"id":"ls-child","values":{"language_servers":"on"}}),
    );
    assert!(hub.language_servers_allowed("ls"), "nearer on wins");
    assert!(
        hub.sync_apply(
            "folders",
            "ls-bad".into(),
            "defaults",
            &serde_json::json!({"id":"ls-child","values":{"language_servers":"maybe"}}),
        )
        .is_err()
    );
}

#[test]
fn mobile_review_bindings_name_contexts_and_stay_bounded() {
    let hub = hub_with_session("bound");
    let apply = |id: String, name: &str, context: &str| {
        hub.sync_apply(
            "mobile-review:bound",
            id,
            name,
            &serde_json::json!({ "context": context }),
        )
    };
    let read = || {
        hub.inner.sessions.lock()["bound"]
            .mobile_review
            .bindings
            .clone()
    };

    apply("b1".into(), "bind", "workspace::hawk::cowboy").unwrap();
    apply("b2".into(), "bind", "workspace::hawk::cowboy").unwrap();
    apply("b3".into(), "bind", "session::other").unwrap();
    assert_eq!(read(), ["workspace::hawk::cowboy", "session::other"]);
    assert!(apply("b4".into(), "bind", "/etc").is_err());
    assert!(apply("b5".into(), "bind", "project").is_err());
    apply("b6".into(), "unbind", "session::other").unwrap();
    assert_eq!(read(), ["workspace::hawk::cowboy"]);
    for index in 0..super::MOBILE_REVIEW_BINDING_CAP - 1 {
        apply(format!("f{index}"), "bind", &format!("session::s{index}")).unwrap();
    }
    assert!(apply("over".into(), "bind", "session::more").is_err());

    let restored = super::MobileReviewState::from_stored(serde_json::json!({
        "bindings": ["session::a", "session::a", "../x", "project"]
    }));
    assert_eq!(restored.bindings, ["session::a"]);
}

#[test]
fn mobile_review_comparison_is_validated_and_clearable() {
    let hub = hub_with_session("compare");
    let reference = "refs/remotes/origin/main";
    let apply = |id: &str, comparison: serde_json::Value| {
        hub.sync_apply(
            "mobile-review:compare",
            id.to_owned(),
            "setComparison",
            &serde_json::json!({ "comparison": comparison }),
        )
    };
    let read =
        || serde_json::to_value(&hub.inner.sessions.lock()["compare"].mobile_review).unwrap();

    apply("c1", serde_json::json!(reference)).unwrap();
    assert_eq!(read()["comparison"], reference);
    apply("c2", serde_json::json!("")).unwrap();
    assert_eq!(read()["comparison"], "");
    assert!(apply("c3", serde_json::json!("--upload-pack=x")).is_err());
    assert_eq!(read()["comparison"], "");
    apply("c4", serde_json::Value::Null).unwrap();
    assert!(read().get("comparison").is_none());

    let restored =
        super::MobileReviewState::from_stored(serde_json::json!({ "comparison": "main; rm -rf" }));
    assert_eq!(restored.comparison, None);
}

#[test]
fn mobile_review_preserves_future_settings_on_older_navigation() {
    let hub = hub_with_session("future-review");
    let saved = serde_json::json!({"future_directory":null,"future":{"nested":true}});
    hub.inner
        .sessions
        .lock()
        .get_mut("future-review")
        .unwrap()
        .mobile_review = super::MobileReviewState::from_stored(saved.clone());
    hub.sync_apply(
        "mobile-review:future-review",
        "open".into(),
        "open",
        &serde_json::json!({"path":"README.md"}),
    )
    .unwrap();
    let value = hub.sync_value("mobile-review:future-review");
    assert_eq!(value.get("future_directory"), saved.get("future_directory"));
    assert_eq!(value["future"], saved["future"]);
    assert_eq!(value["active"], "README.md");
}

#[test]
fn remote_review_binding_survives_local_navigation_and_is_validated() {
    let hub = hub_with_session("remote-review");
    let state = "mobile-review:remote-review";
    let binding = serde_json::json!({"pluginId":"fixture", "view":"pull-requests", "host":"github.com",
        "owner":"owner", "repository":"repo", "repositoryId":"123", "number":"12"});
    hub.sync_apply(
        state,
        "bind".into(),
        "setRemoteReview",
        &serde_json::json!({"binding":binding}),
    )
    .unwrap();
    hub.sync_apply(
        state,
        "select".into(),
        "selectRemoteReview",
        &serde_json::json!({"selected":true}),
    )
    .unwrap();
    hub.sync_apply(
        state,
        "open".into(),
        "open",
        &serde_json::json!({"path":"README.md"}),
    )
    .unwrap();
    hub.sync_apply(
        state,
        "close".into(),
        "close",
        &serde_json::json!({"path":"README.md"}),
    )
    .unwrap();
    let value = hub.sync_value(state);
    assert_eq!(value["remote_review"], binding);
    assert_eq!(value["remote_selected"], true);
    let restored = super::MobileReviewState::from_stored(value);
    assert_eq!(restored.remote_review.unwrap().number, "12");
    let mut bad = binding.clone();
    bad["host"] = serde_json::json!("token@github.com");
    assert!(
        hub.sync_apply(
            state,
            "invalid".into(),
            "setRemoteReview",
            &serde_json::json!({"binding":bad})
        )
        .is_err()
    );
    assert_eq!(hub.sync_value(state)["remote_review"], binding);
    hub.sync_apply(
        state,
        "unbind".into(),
        "setRemoteReview",
        &serde_json::json!({"binding":null}),
    )
    .unwrap();
    assert!(hub.sync_value(state).get("remote_review").is_none());
}

#[test]
fn folders_sync_is_arbitrated_idempotent_and_seeded_on_resync() {
    let hub = hub_with_session("filed");
    hub.sync_apply(
        "folders",
        "f1".to_owned(),
        "create",
        &serde_json::json!({"id": "f-a", "name": "Cowboy", "project": "cowboy"}),
    )
    .unwrap();
    // A retried delivery is a no-op, not a second folder.
    hub.sync_apply(
        "folders",
        "f1".to_owned(),
        "create",
        &serde_json::json!({"id": "f-a", "name": "Retry"}),
    )
    .unwrap();
    hub.sync_apply(
        "folders",
        "f2".to_owned(),
        "place",
        &serde_json::json!({"session_ids": ["filed"], "folder": "f-a"}),
    )
    .unwrap();
    let value = hub.sync_value("folders");
    assert_eq!(value["folders"].as_array().unwrap().len(), 1);
    assert_eq!(value["folders"][0]["name"], "Cowboy");
    assert_eq!(value["folders"][0]["project"], "cowboy");
    assert_eq!(value["placement"]["filed"], "f-a");

    // A rejected mutation leaves its id unconsumed, so the corrected retry
    // under the same id still applies.
    assert_eq!(
        hub.sync_apply(
            "folders",
            "f3".to_owned(),
            "create",
            &serde_json::json!({"id": "f-b", "name": "   "}),
        )
        .unwrap_err(),
        "folder name cannot be empty"
    );
    hub.sync_apply(
        "folders",
        "f3".to_owned(),
        "create",
        &serde_json::json!({"id": "f-b", "name": "B", "parent": "f-a"}),
    )
    .unwrap();
    assert_eq!(hub.sync_value("folders")["folders"][1]["parent"], "f-a");
    assert_eq!(
        hub.sync_apply(
            "folders",
            "f4".to_owned(),
            "explode",
            &serde_json::json!({})
        )
        .unwrap_err(),
        "unknown folders mutation explode"
    );

    assert!(hub.sync_resync().iter().any(|message| matches!(
        message,
        super::Outbound::SyncPatch { state, resync: true, .. } if state == "folders"
    )));
    assert!(hub.delete_session("filed"));
    assert!(
        hub.sync_value("folders")["placement"]
            .get("filed")
            .is_none()
    );
}

#[test]
fn mobile_review_sync_rejects_escaping_paths() {
    let hub = hub_with_session("mobile-invalid");
    let error = hub
        .sync_apply(
            "mobile-review:mobile-invalid",
            "m1".to_owned(),
            "open",
            &serde_json::json!({"path": "../secret"}),
        )
        .unwrap_err();
    assert_eq!(error, "invalid mobile review path");
}

#[test]
fn queued_prompt_can_be_cancelled_by_exact_cmid() {
    let hub = hub_with_session("s");
    hub.submit(
        "s",
        "from ACP".to_owned(),
        Vec::new(),
        Some("acp-1".to_owned()),
    );
    assert_eq!(hub.session_info("s").unwrap().queue_count, 1);
    assert!(!hub.remove_queued_by_cmid("s", "another-client"));
    assert!(hub.remove_queued_by_cmid("s", "acp-1"));
    assert_eq!(hub.session_info("s").unwrap().queue_count, 0);
    assert!(!hub.remove_queued_by_cmid("s", "acp-1"));
}

#[test]
fn a_repeated_call_notice_queues_once_and_is_not_an_authored_prompt() {
    let hub = hub_with_session("parent");
    hub.submit_call_notice("parent", "call-1", "done".to_owned());
    hub.submit_call_notice("parent", "call-1", "done".to_owned());
    assert_eq!(hub.session_info("parent").unwrap().queue_count, 1);
    hub.submit_call_notice("parent", "call-2", "done".to_owned());
    assert_eq!(hub.session_info("parent").unwrap().queue_count, 2);
    assert!(!Hub::is_authored_cmid("call-notice:call-1"));
    hub.submit_call_notice("missing", "call-1", "done".to_owned());
}

#[test]
fn renewed_queue_edit_hold_survives_replaced_socket_cleanup() {
    let hub = hub_with_session("edit-reload");
    let replaced_epoch = hub.set_queue_editing("edit-reload", Some("queued-1".to_owned()));
    let replacement_epoch = hub.set_queue_editing("edit-reload", Some("queued-1".to_owned()));

    assert_ne!(replaced_epoch, replacement_epoch);
    assert!(!hub.release_queue_editing_if_epoch("edit-reload", "queued-1", replaced_epoch,));
    assert_eq!(
        hub.inner
            .sessions
            .lock()
            .get("edit-reload")
            .and_then(|session| session.editing.as_deref()),
        Some("queued-1"),
    );
    assert!(hub.release_queue_editing_if_epoch("edit-reload", "queued-1", replacement_epoch,));
}

// The queue texts as clients would see them (via the resync patch).
fn queue_texts(hub: &Hub, id: &str) -> Vec<String> {
    match hub.queue_resync(id) {
        Some(Outbound::SyncPatch { value, .. }) => value["queue"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|m| m["text"].as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default(),
        _ => vec![],
    }
}

// A prompt salvaged from a crashed cold-start lands back on the queue (so it's
// never lost) and is idempotent on cmid (a racing re-revive can't double it).
#[test]
fn requeue_prompt_restores_and_dedupes() {
    let hub = hub_with_session("r1");
    assert!(queue_texts(&hub, "r1").is_empty());
    hub.requeue_prompt(
        "r1",
        "hello agent".to_owned(),
        vec![],
        Some("c1".to_owned()),
    );
    assert_eq!(queue_texts(&hub, "r1"), vec!["hello agent".to_owned()]);
    // Same cmid (the delivery raced a re-revive) → not double-queued.
    hub.requeue_prompt(
        "r1",
        "hello agent".to_owned(),
        vec![],
        Some("c1".to_owned()),
    );
    assert_eq!(
        queue_texts(&hub, "r1").len(),
        1,
        "same cmid must not double-queue"
    );
    // A different message DOES stack (front-inserted).
    hub.requeue_prompt("r1", "second".to_owned(), vec![], Some("c2".to_owned()));
    assert_eq!(
        queue_texts(&hub, "r1"),
        vec!["second".to_owned(), "hello agent".to_owned()]
    );
}

#[tokio::test]
async fn pending_context_holds_schedules_drafts_and_manual_queue_until_cleared() {
    let hub = hub_with_session("context-hold");
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    hub.set_dispatch_tx(tx);
    hub.set_status("context-hold", Status::Running, None);
    let guard = hub.begin_context_edit("context-hold", false).unwrap();
    // Simulate a scheduler racing the durable preference write.
    hub.submit(
        "context-hold",
        "wake".into(),
        vec![],
        Some("__wakeup-test".into()),
    );
    assert!(rx.try_recv().is_err());
    hub.load_settings(vec![(
        crate::session_context::key("context-hold"),
        serde_json::json!({"revision":1,"applied_revision":0}),
    )]);
    drop(guard);
    hub.add_draft(
        "context-hold",
        "draft".into(),
        vec![],
        Some("draft-id".into()),
    );
    hub.activate_all_drafts("context-hold");
    hub.add_draft(
        "context-hold",
        "scheduled".into(),
        vec![],
        Some("scheduled-id".into()),
    );
    let id = hub.queue_and_drafts("context-hold", |_| true).unwrap().1[0]
        .id
        .clone();
    hub.fire_scheduled_draft("context-hold", &id);
    assert!(!hub.force_submit(
        "context-hold",
        "front".into(),
        vec![],
        Some("front-id".into()),
        true
    ));
    for _ in 0..3 {
        hub.drain_now("context-hold");
        hub.try_drain("context-hold");
    }
    assert!(rx.try_recv().is_err());
    assert_eq!(
        queue_texts(&hub, "context-hold"),
        vec!["front", "wake", "draft", "scheduled"]
    );
    // Clearing remains possible with a held queue and releases it once.
    let guard = hub.begin_context_edit("context-hold", true).unwrap();
    hub.load_settings(vec![(
        crate::session_context::key("context-hold"),
        serde_json::json!({"revision":2,"applied_revision":2}),
    )]);
    drop(guard);
    assert_eq!(rx.try_recv().unwrap().text, "front");
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn context_bootstrap_precedes_racing_queue_without_unpausing_it() {
    let hub = hub_with_session("context-first");
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    hub.set_dispatch_tx(tx);
    hub.set_status("context-first", Status::Running, None);
    hub.set_paused("context-first", true);
    hub.load_settings(vec![(crate::session_context::key("context-first"), serde_json::json!({
        "revision":2,"applied_revision":2,"delivery":{"operation":"first","fingerprint":"hash","session_id":"context-first","state":"preparing"},
        "injection":{"cmid":"context-first-message","sources":[]}
    }))]);
    hub.submit(
        "context-first",
        "racing message".into(),
        vec![],
        Some("racing".into()),
    );
    hub.submit_context(
        "context-first",
        "bootstrap".into(),
        vec![],
        "context-first-message".into(),
    );
    assert_eq!(
        queue_texts(&hub, "context-first"),
        vec!["bootstrap", "racing message"]
    );
    assert!(rx.try_recv().is_err());
    assert!(hub.session_info("context-first").unwrap().meta.paused);
    hub.drain_now("context-first");
    assert_eq!(rx.try_recv().unwrap().text, "bootstrap");
    assert!(rx.try_recv().is_err());
}

#[test]
fn continuation_bootstrap_waits_for_its_agent_and_can_be_taken_back() {
    let hub = hub_with_session("continued");
    hub.load_settings(vec![(
        crate::session_context::key("continued"),
        serde_json::json!({"revision":2,"applied_revision":2,
            "injection":{"cmid":"context-handoff","sources":[]}}),
    )]);
    let waits = |cmid| crate::session_context::waits_for_agent(&hub, "continued", Some(cmid));
    // Only a continuation's own bootstrap waits; links in place do not.
    assert!(!waits("context-handoff"));
    hub.set_continued_from("continued", Some("source".into()));
    assert!(waits("context-handoff"));
    assert!(!waits("typed"));
    hub.requeue_prompt(
        "continued",
        "first".into(),
        vec![],
        Some("context-handoff".into()),
    );
    hub.requeue_prompt("continued", "later".into(), vec![], Some("typed".into()));
    // Other queued work stays with the destination.
    assert!(
        hub.take_sole_queued("continued", "context-handoff")
            .is_none()
    );
    assert!(hub.remove_queued_by_cmid("continued", "typed"));
    assert!(hub.take_sole_queued("continued", "typed").is_none());
    let (text, _) = hub
        .take_sole_queued("continued", "context-handoff")
        .unwrap();
    assert_eq!(text, "first");
    assert!(queue_texts(&hub, "continued").is_empty());
    // A cancelled receipt stays readable, lets prompts through, and keeps
    // startup from recording the relationship again from the source's receipt.
    let receipt = |state: &str| serde_json::json!({"operation":"handoff","fingerprint":"f","session_id":"continued","state":state});
    hub.create_local_session(
        "source".to_owned(),
        "claude-code".to_owned(),
        "/tmp".to_owned(),
        "t".to_owned(),
        SessionOrigin::Web,
        false,
    );
    hub.load_settings(vec![
        (
            crate::session_context::key("continued"),
            serde_json::json!({"revision":3,"applied_revision":3,"delivery":receipt("cancelled")}),
        ),
        (
            crate::session_context::key("source"),
            serde_json::json!({"revision":2,"applied_revision":2,"delivery":receipt("accepted")}),
        ),
    ]);
    assert!(crate::session_context::read(&hub, "continued").is_ok());
    assert!(!crate::session_context::pending(&hub, "continued"));
    hub.set_continued_from("continued", None);
    hub.restore_continuations();
    assert_eq!(
        hub.session_info("continued").unwrap().meta.continued_from,
        None
    );
}

#[tokio::test]
async fn resource_retry_respects_pause_edit_cancel_and_dispatches_once() {
    let hub = hub_with_session("wait");
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    hub.set_dispatch_tx(tx);
    hub.set_status("wait", Status::Exited, None);
    hub.requeue_prompt("wait", "keep me".into(), vec![], Some("once".into()));
    hub.set_paused("wait", true);
    hub.retry_resource_wait("wait");
    assert!(rx.try_recv().is_err());
    hub.set_paused("wait", false);
    let id = hub.queue_and_drafts("wait", |_| true).unwrap().0[0]
        .id
        .clone();
    hub.set_queue_editing("wait", Some(id));
    hub.retry_resource_wait("wait");
    assert!(rx.try_recv().is_err());
    hub.set_queue_editing("wait", None);
    hub.remove_queued_by_cmid("wait", "once");
    hub.retry_resource_wait("wait");
    assert!(rx.try_recv().is_err());
    hub.requeue_prompt("wait", "send once".into(), vec![], Some("next".into()));
    hub.retry_resource_wait("wait");
    hub.retry_resource_wait("wait");
    assert_eq!(rx.try_recv().unwrap().cmid.as_deref(), Some("next"));
    assert!(rx.try_recv().is_err());
}

#[test]
fn queue_pause_survives_restore_and_is_removed_with_the_session() {
    let hub = hub_with_session("paused");
    hub.set_paused("paused", true);
    let restored = Hub::new();
    restored.load_settings(hub.settings_snapshot().into_iter().collect());
    let mut session = super::runtime_reconciliation_tests::restored_busy("paused");
    session.meta.status = Status::Exited;
    restored.restore_with_workers(vec![session], &[]);
    assert!(restored.session_info("paused").unwrap().meta.paused);
    restored.set_paused("paused", false);
    assert_eq!(
        restored.setting("session_queue_paused:paused"),
        Some(serde_json::Value::Bool(false))
    );
    restored.delete_session("paused");
    assert_eq!(restored.setting("session_queue_paused:paused"), None);
}

#[tokio::test]
async fn paused_empty_queue_holds_submit_but_explicit_send_once_does_not_resume() {
    let hub = hub_with_session("paused");
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    hub.set_dispatch_tx(tx);
    hub.set_status("paused", Status::Running, None);
    hub.set_paused("paused", true);
    hub.submit("paused", "first".into(), vec![], Some("one".into()));
    hub.submit("paused", "second".into(), vec![], Some("two".into()));
    assert!(rx.try_recv().is_err());
    assert_eq!(queue_texts(&hub, "paused"), vec!["first", "second"]);
    hub.drain_now("paused");
    assert_eq!(rx.try_recv().unwrap().cmid.as_deref(), Some("one"));
    assert!(hub.session_info("paused").unwrap().meta.paused);
    assert_eq!(queue_texts(&hub, "paused"), vec!["second"]);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn dispatch_marks_explicit_sends_and_consumes_a_waiting_intent() {
    let hub = hub_with_session("explicit");
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    hub.set_dispatch_tx(tx);
    hub.set_status("explicit", Status::Running, None);
    hub.set_paused("explicit", true);
    hub.submit("explicit", "held".into(), vec![], Some("one".into()));
    // Maintenance's ordinary retry respects Pause and is not explicit.
    hub.retry_resource_wait("explicit");
    assert!(rx.try_recv().is_err());
    hub.set_explicit_wait("explicit", true);
    hub.drain_now("explicit");
    let request = rx.try_recv().unwrap();
    assert!(request.explicit);
    assert_eq!(request.cmid.as_deref(), Some("one"));
    assert!(
        !hub.explicit_wait("explicit"),
        "dispatch consumes the intent"
    );
}

#[test]
fn copy_undo_requires_the_original_unsent_row() {
    let hub = hub_with_session("undo-copy");
    let content = vec![serde_json::json!({"type":"text", "text":"original"})];
    hub.add_draft(
        "undo-copy",
        "original".into(),
        content.clone(),
        Some("copy-1".into()),
    );
    let id = hub.inner.sessions.lock()["undo-copy"].drafts[0].id.clone();
    assert!(!hub.remove_draft_if_unchanged("missing", "copy-1", "original", &content));
    assert!(!hub.remove_draft_if_unchanged("undo-copy", "copy-1", "original", &[]));
    hub.edit_draft("undo-copy", &id, "edited".into(), content.clone());
    assert!(!hub.remove_draft_if_unchanged("undo-copy", "copy-1", "original", &content));
    assert_eq!(hub.inner.sessions.lock()["undo-copy"].drafts.len(), 1);
    assert!(hub.remove_draft_if_unchanged("undo-copy", "copy-1", "edited", &content));
    assert!(!hub.remove_draft_if_unchanged("undo-copy", "copy-1", "edited", &content));
    hub.add_draft(
        "undo-copy",
        "original".into(),
        content.clone(),
        Some("copy-2".into()),
    );
    hub.inner
        .sessions
        .lock()
        .get_mut("undo-copy")
        .unwrap()
        .drafts[0]
        .schedule = Some(DraftSchedule {
        fire_at_ms: i64::MAX,
        delivery: Delivery::default(),
    });
    assert!(!hub.remove_draft_if_unchanged("undo-copy", "copy-2", "original", &content));
}

#[tokio::test]
async fn interrupt_hold_blocks_drain_and_only_its_owner_can_replace_once() {
    let hub = hub_with_session("held");
    let (tx, mut rx) = mpsc::channel(8);
    hub.set_dispatch_tx(tx);
    hub.set_status("held", Status::Running, None);
    hub.publish_delivery_hold("held", Some("interrupt-a".into()));
    hub.submit("held", "queued".into(), vec![], Some("queued-a".into()));
    hub.drain_now("held");
    assert!(rx.try_recv().is_err());
    assert!(!hub.dispatch_interrupt_once("held", "wrong-owner", "wrong".into(), vec![]));
    hub.set_paused("held", true);
    assert!(hub.dispatch_interrupt_once("held", "interrupt-a", "replacement".into(), vec![]));
    let request = rx.try_recv().unwrap();
    assert_eq!(request.text, "replacement");
    assert_eq!(request.cmid.as_deref(), Some("interrupt-a"));
    assert!(!hub.dispatch_interrupt_once("held", "interrupt-a", "duplicate".into(), vec![]));
    hub.publish_delivery_hold("held", None);
    assert!(hub.session_info("held").unwrap().meta.paused);
    assert_eq!(queue_texts(&hub, "held"), vec!["queued"]);
    assert!(rx.try_recv().is_err());
}

#[test]
fn interrupt_hold_survives_restore_and_session_deletion_removes_it() {
    let hub = hub_with_session("held");
    hub.publish_delivery_hold("held", Some("unresolved".into()));
    let restored = Hub::new();
    restored.load_settings(hub.settings_snapshot().into_iter().collect());
    let mut session = super::runtime_reconciliation_tests::restored_busy("held");
    session.meta.status = Status::Exited;
    restored.restore_with_workers(vec![session], &[]);
    assert_eq!(
        restored.delivery_hold("held").as_deref(),
        Some("unresolved")
    );
    restored.delete_session("held");
    assert!(restored.setting("session_delivery_hold:held").is_none());
}

#[tokio::test]
async fn activated_image_draft_keeps_its_echo_identity_and_replay_is_deduped() {
    let hub = hub_with_session("draft-send");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.set_status("draft-send", Status::Running, None);
    let content = vec![
        serde_json::json!({"type": "image", "data": "c2hvdA==", "mimeType": "image/png"}),
        serde_json::json!({"type": "text", "text": "caption"}),
    ];
    hub.add_draft(
        "draft-send",
        "caption".to_owned(),
        content.clone(),
        Some("draft-cmid".to_owned()),
    );
    let id = hub.inner.sessions.lock()["draft-send"].drafts[0].id.clone();
    hub.activate_draft("draft-send", &id, Some("send-cmid".to_owned()));
    let dispatched = rx.recv().await.expect("draft dispatch");
    assert_eq!(dispatched.cmid.as_deref(), Some("send-cmid"));
    assert_eq!(dispatched.content, content);
    hub.activate_draft("draft-send", &id, Some("send-cmid".to_owned()));
    assert!(
        rx.try_recv().is_err(),
        "replayed activation must not dispatch twice"
    );
    for (index, block) in content.into_iter().enumerate() {
        hub.push_tagged(
            "draft-send",
            Event::Update {
                update: serde_json::json!({"sessionUpdate": "user_message_chunk", "content": block}),
            },
            (index == 0).then(|| dispatched.cmid.clone()).flatten(),
        );
    }
    let (events, _) = hub.snapshot("draft-send").expect("transcript");
    let echo = events
        .iter()
        .find(|env| is_user_message_chunk(env))
        .expect("image echo");
    assert_eq!(echo.cmid.as_deref(), Some("send-cmid"));
    assert_eq!(
        crate::persistence::persisted_event_payload(echo).expect("persisted echo")["cmid"],
        "send-cmid"
    );
    let mut live = hub.subscribe();
    hub.submit(
        "draft-send",
        "caption".to_owned(),
        vec![],
        Some("send-cmid".to_owned()),
    );
    assert!(
        rx.try_recv().is_err(),
        "replayed submit must not dispatch twice"
    );
    let frame = live.try_recv().expect("delivery confirmation");
    assert!(
        matches!(frame.outbound(), Outbound::SyncPatch { confirmed, .. }
        if confirmed == &vec!["send-cmid".to_owned()])
    );
}

#[tokio::test]
async fn bulk_draft_activation_preserves_ids_in_dispatch_and_queue() {
    let hub = hub_with_session("bulk-drafts");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.set_status("bulk-drafts", Status::Running, None);
    for cmid in [Some("first"), Some("second"), None] {
        hub.add_draft(
            "bulk-drafts",
            "prompt".to_owned(),
            vec![],
            cmid.map(str::to_owned),
        );
    }
    hub.activate_all_drafts("bulk-drafts");
    assert_eq!(
        rx.recv().await.expect("first dispatch").cmid.as_deref(),
        Some("first")
    );
    let sessions = hub.inner.sessions.lock();
    let session = &sessions["bulk-drafts"];
    assert!(session.drafts.is_empty());
    assert_eq!(session.queue.len(), 2);
    assert_eq!(session.queue[0].cmid.as_deref(), Some("second"));
    assert_eq!(session.queue[1].cmid, None);
}

#[tokio::test]
async fn legacy_draft_activation_keeps_the_source_identity_when_available() {
    for cmid in [Some("legacy-draft"), None] {
        let hub = hub_with_session("legacy-draft-send");
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("legacy-draft-send", Status::Running, None);
        hub.add_draft(
            "legacy-draft-send",
            "prompt".to_owned(),
            vec![],
            cmid.map(str::to_owned),
        );
        let id = hub.inner.sessions.lock()["legacy-draft-send"].drafts[0]
            .id
            .clone();
        let request: Inbound = serde_json::from_value(serde_json::json!({
            "type": "activate_draft", "session_id": "legacy-draft-send", "id": id,
        }))
        .expect("legacy request");
        let Inbound::ActivateDraft {
            session_id,
            id,
            cmid: send_cmid,
        } = request
        else {
            panic!("expected draft activation");
        };
        assert!(send_cmid.is_none());
        hub.activate_draft(&session_id, &id, send_cmid);
        assert_eq!(
            rx.recv().await.expect("draft dispatch").cmid.as_deref(),
            cmid
        );
    }
}

// Dispatch admission prevents duplicate execution, but it is not proof
// that a queued row or transcript exists to replace the browser's outbox.
#[tokio::test]
async fn replayed_submit_before_echo_stays_unconfirmed_and_is_not_rerun() {
    let hub = hub_with_session("replay");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.set_status("replay", Status::Running, None);
    hub.submit("replay", "hello".to_owned(), vec![], Some("c-1".to_owned()));
    assert_eq!(rx.recv().await.expect("first dispatch").text, "hello");

    let mut live = hub.subscribe();
    hub.submit("replay", "hello".to_owned(), vec![], Some("c-1".to_owned()));
    assert!(!hub.force_submit(
        "replay",
        "hello".to_owned(),
        vec![],
        Some("c-1".to_owned()),
        true,
    ));
    assert!(
        rx.try_recv().is_err(),
        "a replayed submit must not dispatch again"
    );
    assert!(queue_texts(&hub, "replay").is_empty());
    assert!(
        live.try_recv().is_err(),
        "dispatch admission must not erase a prompt before its user echo"
    );

    hub.push_tagged(
        "replay",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": {"type": "text", "text": "hello"}
            }),
        },
        Some("c-1".to_owned()),
    );
    // A fresh reconnect can now retire the outbox: transcript replay owns
    // the prompt. The original dispatch remains the only execution.
    let mut live = hub.subscribe();
    hub.submit("replay", "hello".to_owned(), vec![], Some("c-1".to_owned()));
    assert!(rx.try_recv().is_err());
    let frame = live.try_recv().expect("echo-backed confirmation");
    let Outbound::SyncPatch {
        state, confirmed, ..
    } = frame.outbound()
    else {
        panic!("expected a queue confirmation");
    };
    assert_eq!(state, "queue:replay");
    assert_eq!(confirmed, &vec!["c-1".to_owned()]);

    hub.push(
        "replay",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "subsequent output"}
            }),
        },
    );
    {
        let mut sessions = hub.inner.sessions.lock();
        let s = sessions.get_mut("replay").unwrap();
        assert!(trim_hot_log(&mut s.log, &mut s.log_bytes, false, 1));
        assert!(!s.log.iter().any(is_user_message_chunk));
    }
    let mut live = hub.subscribe();
    hub.submit("replay", "hello".to_owned(), vec![], Some("c-1".to_owned()));
    assert!(rx.try_recv().is_err(), "evicted echoes must not run again");
    let frame = live.try_recv().expect("retained delivery evidence");
    assert!(
        matches!(frame.outbound(), Outbound::SyncPatch { confirmed, .. }
        if confirmed == &vec!["c-1".to_owned()])
    );
}

// After a Controller restart the dispatched-id window is empty, but the user
// echo restored from storage still carries the cmid of the prompt it ran.
#[tokio::test]
async fn replayed_submit_is_recognised_from_the_persisted_echo() {
    let hub = hub_with_session("restored");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.set_status("restored", Status::Running, None);
    let echo = || Event::Update {
        update: serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": {"type": "text", "text": "hello"},
        }),
    };
    hub.push_tagged("restored", echo(), Some("c-2".to_owned()));

    hub.submit(
        "restored",
        "hello".to_owned(),
        vec![],
        Some("c-2".to_owned()),
    );
    assert!(
        rx.try_recv().is_err(),
        "an echoed prompt must not run again"
    );
    assert!(queue_texts(&hub, "restored").is_empty());

    // Hub-synthesized ids are reusable on purpose and never deduped by the log.
    let synthesized = "cowboy-retry:restored:1".to_owned();
    hub.push_tagged("restored", echo(), Some(synthesized.clone()));
    hub.submit("restored", "again".to_owned(), vec![], Some(synthesized));
    assert_eq!(
        rx.recv().await.expect("synthesized id dispatches").text,
        "again"
    );
}

#[tokio::test]
async fn replayed_image_submit_retains_payload_until_requeued() {
    let hub = hub_with_session("image-retry");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.set_status("image-retry", Status::Running, None);
    let content = vec![serde_json::json!({
        "type": "image", "mimeType": "image/png", "data": "synthetic-image"
    })];
    hub.submit(
        "image-retry",
        "explain".into(),
        content.clone(),
        Some("image-1".into()),
    );
    let dispatched = rx.recv().await.unwrap();
    assert_eq!(dispatched.content, content);
    // A tagged diagnostic is not a user echo and must not acknowledge it.
    hub.push_tagged(
        "image-retry",
        Event::Update {
            update: serde_json::json!({"sessionUpdate": "config_option_update"}),
        },
        Some("image-1".into()),
    );
    let mut live = hub.subscribe();
    hub.submit(
        "image-retry",
        "explain".into(),
        content.clone(),
        Some("image-1".into()),
    );
    assert!(rx.try_recv().is_err());
    assert!(live.try_recv().is_err());

    // A failed transport returns the exact prompt to an authoritative row.
    hub.requeue_prompt(
        "image-retry",
        dispatched.text,
        dispatched.content,
        dispatched.cmid,
    );
    let mut live = hub.subscribe();
    hub.submit(
        "image-retry",
        "explain".into(),
        content.clone(),
        Some("image-1".into()),
    );
    let frame = live.try_recv().expect("queue-backed acknowledgement");
    let Outbound::SyncPatch {
        value, confirmed, ..
    } = frame.outbound()
    else {
        panic!("expected queue patch");
    };
    assert_eq!(confirmed, &vec!["image-1".to_owned()]);
    assert_eq!(value["queue"][0]["content"], serde_json::json!(content));
    assert!(rx.try_recv().is_err());
}

// A replayed draft is confirmed again so a client that missed the first
// patch retires its outbox entry instead of waiting out a timeout.
#[test]
fn replayed_add_draft_is_confirmed_not_duplicated() {
    let hub = hub_with_session("draft-replay");
    hub.add_draft(
        "draft-replay",
        "later".to_owned(),
        vec![],
        Some("d-1".to_owned()),
    );
    let mut live = hub.subscribe();
    hub.add_draft(
        "draft-replay",
        "later".to_owned(),
        vec![],
        Some("d-1".to_owned()),
    );
    let frame = live.try_recv().expect("replayed draft confirmation");
    let Outbound::SyncPatch {
        state,
        value,
        confirmed,
        ..
    } = frame.outbound()
    else {
        panic!("expected a queue patch");
    };
    assert_eq!(state, "queue:draft-replay");
    assert_eq!(confirmed, &vec!["d-1".to_owned()]);
    assert_eq!(value["drafts"].as_array().unwrap().len(), 1);
}

// A duplicate sync delivery is confirmed again, so a client that missed the
// original patch can retire its outbox entry.
#[test]
fn duplicate_sync_delivery_is_confirmed_again() {
    let hub = hub_with_session("dup");
    hub.sync_apply(
        "title",
        "m-1".to_owned(),
        "rename",
        &serde_json::json!({"session_id": "dup", "title": "First"}),
    )
    .unwrap();
    let mut live = hub.subscribe();
    hub.sync_apply(
        "title",
        "m-1".to_owned(),
        "rename",
        &serde_json::json!({"session_id": "dup", "title": "Ignored"}),
    )
    .unwrap();
    let frame = live.try_recv().expect("replayed delivery confirmation");
    let Outbound::SyncPatch {
        state,
        value,
        confirmed,
        ..
    } = frame.outbound()
    else {
        panic!("expected a sync patch");
    };
    assert_eq!(state, "title");
    assert_eq!(confirmed, &vec!["m-1".to_owned()]);
    assert_eq!(value["dup"], "First");
}

// The arbiter's dedupe set does not survive a restart; a create that already
// produced exactly this folder is still the same retried delivery.
#[test]
fn folder_create_replayed_across_restart_is_confirmed() {
    let hub = hub_with_session("filed-replay");
    let args = serde_json::json!({"id": "f-r", "name": "Cowboy", "project": "cowboy"});
    hub.sync_apply("folders", "m-1".to_owned(), "create", &args)
        .unwrap();
    let folders = hub.inner.folders.lock().folders().to_vec();

    let restarted = hub_with_session("filed-replay");
    restarted.restore_session_folders(folders);
    let mut live = restarted.subscribe();
    restarted
        .sync_apply("folders", "m-1".to_owned(), "create", &args)
        .unwrap();
    let frame = live.try_recv().expect("replayed create confirmation");
    let Outbound::SyncPatch {
        state,
        value,
        confirmed,
        ..
    } = frame.outbound()
    else {
        panic!("expected a folders patch");
    };
    assert_eq!(state, "folders");
    assert_eq!(confirmed, &vec!["m-1".to_owned()]);
    assert_eq!(value["folders"].as_array().unwrap().len(), 1);
    // A different folder under the same id is still a conflict.
    assert_eq!(
        restarted
            .sync_apply(
                "folders",
                "m-2".to_owned(),
                "create",
                &serde_json::json!({"id": "f-r", "name": "Other"}),
            )
            .unwrap_err(),
        "folder id already exists"
    );
}

// A replayed edit or removal must be able to tell the client when its row
// already left the queue or drafts, so the outbox entry can be retired.
#[test]
fn queue_and_draft_edits_report_whether_the_row_still_exists() {
    let hub = hub_with_session("rows");
    hub.submit("rows", "queued".to_owned(), vec![], Some("q-1".to_owned()));
    hub.add_draft("rows", "draft".to_owned(), vec![], Some("d-1".to_owned()));
    let Some(Outbound::SyncPatch { value, .. }) = hub.queue_resync("rows") else {
        panic!("expected a queue resync");
    };
    let queued = value["queue"][0]["id"].as_str().unwrap().to_owned();
    let draft = value["drafts"][0]["id"].as_str().unwrap().to_owned();

    assert!(hub.edit_queued("rows", &queued, "edited".to_owned(), vec![]));
    assert!(!hub.edit_queued("rows", "missing", "edited".to_owned(), vec![]));
    assert!(hub.remove_queued("rows", &queued));
    assert!(!hub.remove_queued("rows", &queued));
    assert!(!hub.edit_queued("rows", &queued, String::new(), vec![]));

    assert!(hub.edit_draft("rows", &draft, "edited".to_owned(), vec![]));
    assert!(!hub.edit_draft("rows", "missing", "edited".to_owned(), vec![]));
    assert!(hub.remove_draft("rows", &draft));
    assert!(!hub.remove_draft("rows", &draft));
    assert!(!hub.edit_draft("rows", &draft, String::new(), vec![]));

    assert!(!hub.remove_queued("gone", "x"));
    assert!(!hub.remove_draft("gone", "x"));
}

// A permission answered on two devices resolves once in the transcript;
// the first answer wins and the second appends no row.
#[test]
fn duplicate_permission_resolution_appends_no_second_row() {
    let hub = hub_with_session("perm");
    hub.push(
        "perm",
        Event::PermissionRequest {
            request_id: "r-1".to_owned(),
            tool_call: serde_json::json!({}),
            options: serde_json::json!([]),
        },
    );
    hub.push(
        "perm",
        Event::PermissionResolved {
            request_id: "r-1".to_owned(),
            option_id: Some("allow".to_owned()),
        },
    );
    hub.push(
        "perm",
        Event::PermissionResolved {
            request_id: "r-1".to_owned(),
            option_id: Some("deny".to_owned()),
        },
    );
    let resolutions = |hub: &Hub| -> Vec<Option<String>> {
        hub.snapshot("perm")
            .expect("snapshot")
            .0
            .into_iter()
            .filter_map(|entry| match entry.event {
                Event::PermissionResolved { option_id, .. } => Some(option_id),
                _ => None,
            })
            .collect()
    };
    assert_eq!(resolutions(&hub), vec![Some("allow".to_owned())]);
    hub.push(
        "perm",
        Event::PermissionResolved {
            request_id: "r-2".to_owned(),
            option_id: None,
        },
    );
    assert_eq!(resolutions(&hub), vec![Some("allow".to_owned()), None]);
}

// The request id is the agent's JSON-RPC id and starts over with every
// agent process, so an old resolution must not swallow the answer to a
// later request that reuses the number.
#[test]
fn a_reused_permission_request_id_is_resolved_again() {
    let hub = hub_with_session("perm");
    let request = |hub: &Hub| {
        hub.push(
            "perm",
            Event::PermissionRequest {
                request_id: "1".to_owned(),
                tool_call: serde_json::json!({}),
                options: serde_json::json!([]),
            },
        );
    };
    let resolve = |hub: &Hub, option: &str| {
        hub.push(
            "perm",
            Event::PermissionResolved {
                request_id: "1".to_owned(),
                option_id: Some(option.to_owned()),
            },
        );
    };
    let resolutions = |hub: &Hub| -> Vec<Option<String>> {
        hub.snapshot("perm")
            .expect("snapshot")
            .0
            .into_iter()
            .filter_map(|entry| match entry.event {
                Event::PermissionResolved { option_id, .. } => Some(option_id),
                _ => None,
            })
            .collect()
    };
    request(&hub);
    resolve(&hub, "first");
    // A new agent process asks again under the same id.
    request(&hub);
    resolve(&hub, "second");
    // A second device answering that same request is still a duplicate.
    resolve(&hub, "late");
    assert_eq!(
        resolutions(&hub),
        vec![Some("first".to_owned()), Some("second".to_owned())]
    );
}

// Per-session sync dedupe sets are dropped with the session.
#[test]
fn deleting_a_session_drops_its_sync_states() {
    let hub = hub_with_session("gone");
    hub.sync_apply(
        "mobile-review:gone",
        "m-1".to_owned(),
        "open",
        &serde_json::json!({"path": "README.md"}),
    )
    .unwrap();
    hub.add_draft("gone", "draft".to_owned(), vec![], Some("d-1".to_owned()));
    assert!(hub.inner.sync.lock().contains_key("mobile-review:gone"));
    assert!(hub.inner.sync.lock().contains_key("queue:gone"));
    assert!(hub.delete_session("gone"));
    assert!(!hub.inner.sync.lock().contains_key("mobile-review:gone"));
    assert!(!hub.inner.sync.lock().contains_key("queue:gone"));
}

// A pending environment stop is visible on the wire only while it lasts.
#[test]
fn closing_flag_is_listed_only_while_set() {
    let hub = hub_with_session("closing");
    let listed =
        |hub: &Hub| serde_json::to_value(&hub.session_list()[0]).expect("serialize session");
    assert!(listed(&hub).get("closing").is_none());
    hub.set_closing("closing", true);
    assert_eq!(listed(&hub)["closing"], true);
    hub.set_closing("closing", false);
    assert!(listed(&hub).get("closing").is_none());
}

#[test]
fn session_cwd_retarget_preserves_native_thread_and_transcript() {
    let hub = hub_with_session("migrated");
    hub.set_agent_session_id("migrated", "codex-thread-1".to_owned());
    hub.push(
        "migrated",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {"text": "preserved"}
            }),
        },
    );
    let before = hub.snapshot("migrated").expect("snapshot").0;

    hub.update_session_cwd("migrated", "/new/checkout".to_owned())
        .expect("retarget");

    let meta = hub
        .session_list()
        .into_iter()
        .find(|meta| meta.id == "migrated")
        .expect("session");
    assert_eq!(meta.cwd, "/new/checkout");
    assert_eq!(meta.agent_session_id.as_deref(), Some("codex-thread-1"));
    let after = hub.snapshot("migrated").expect("snapshot").0;
    assert_eq!(after.len(), before.len());
    assert_eq!(after[0].seq, before[0].seq);
    assert_eq!(
        serde_json::to_value(&after[0].event).expect("serialize after"),
        serde_json::to_value(&before[0].event).expect("serialize before")
    );
}

#[tokio::test]
async fn retry_after_startup_failure_dispatches_the_original_queued_prompt_once() {
    let hub = hub_with_session("startup-retry");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.set_status(
        "startup-retry",
        Status::Crashed,
        Some("execution binding failed".into()),
    );
    let content =
        vec![serde_json::json!({"type":"image", "data":"fixture", "mimeType":"image/png"})];
    hub.requeue_prompt(
        "startup-retry",
        "first prompt".into(),
        content.clone(),
        Some("original-cmid".into()),
    );

    hub.retry_turn("startup-retry");
    hub.retry_turn("startup-retry");

    let dispatched = rx.try_recv().expect("startup retry reaches dispatch");
    assert_eq!(dispatched.text, "first prompt");
    assert_eq!(dispatched.content, content);
    assert_eq!(dispatched.cmid.as_deref(), Some("original-cmid"));
    assert!(rx.try_recv().is_err(), "startup retry duplicated delivery");
    assert_eq!(hub.session_info("startup-retry").unwrap().queue_count, 0);
}

#[tokio::test]
async fn repeated_retry_dispatches_original_prompt_once() {
    let hub = hub_with_session("retry-once");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.push(
        "retry-once",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": {"text": "do the thing"}
            }),
        },
    );
    hub.set_status("retry-once", Status::Crashed, Some("boom".to_owned()));

    hub.retry_turn("retry-once");
    hub.retry_turn("retry-once");

    let first = rx.recv().await.expect("first retry");
    assert_eq!(first.text, "do the thing");
    assert!(
        first
            .cmid
            .as_deref()
            .is_some_and(|id| id.starts_with("cowboy-retry:"))
    );
    assert!(rx.try_recv().is_err(), "duplicate retry was dispatched");
}

#[tokio::test]
async fn retry_resubmits_every_block_of_the_last_prompt() {
    let hub = hub_with_session("retry-blocks");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    let image = serde_json::json!({"type": "image", "data": "fixture", "mimeType": "image/png"});
    let text = serde_json::json!({"type": "text", "text": "describe this"});
    for update in [
        serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": {"type": "text", "text": "an earlier prompt"}
        }),
        serde_json::json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": "an earlier reply"}
        }),
        serde_json::json!({"sessionUpdate": "user_message_chunk", "content": image}),
        serde_json::json!({"sessionUpdate": "user_message_chunk", "content": text}),
    ] {
        hub.push("retry-blocks", Event::Update { update });
    }
    hub.set_status("retry-blocks", Status::Crashed, Some("boom".to_owned()));

    hub.retry_turn("retry-blocks");

    let dispatched = rx.try_recv().expect("retry dispatches the last prompt");
    assert_eq!(dispatched.text, "describe this");
    assert_eq!(dispatched.content, vec![image, text]);
    assert!(
        rx.try_recv().is_err(),
        "retry dispatched more than one prompt"
    );
}

#[tokio::test]
async fn retry_of_a_prompt_ending_with_an_image_is_not_a_no_op() {
    let hub = hub_with_session("retry-trailing-image");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    for update in [
        serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": {"type": "text", "text": "look"}
        }),
        serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": {"type": "image", "data": "fixture", "mimeType": "image/png"}
        }),
    ] {
        hub.push("retry-trailing-image", Event::Update { update });
    }
    hub.set_status(
        "retry-trailing-image",
        Status::Crashed,
        Some("boom".to_owned()),
    );

    hub.retry_turn("retry-trailing-image");

    let dispatched = rx
        .try_recv()
        .expect("a prompt that ends with an image is still retried");
    assert_eq!(dispatched.text, "look");
    assert_eq!(dispatched.content.len(), 2);
}

#[tokio::test]
async fn retry_restores_image_bytes_externalized_into_artifacts() {
    let root = std::env::temp_dir().join(format!(
        "cowboy-retry-artifacts-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let hub = hub_with_session("retry-artifact");
    hub.set_artifacts(crate::artifacts::ArtifactStore::new(root.clone()).unwrap());
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    let data = base64::engine::general_purpose::STANDARD.encode(vec![7_u8; 40_000]);
    hub.push(
        "retry-artifact",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": {"type": "image", "mimeType": "image/png", "data": data}
            }),
        },
    );
    hub.push(
        "retry-artifact",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": {"type": "text", "text": "what is this"}
            }),
        },
    );
    let externalized = hub
        .snapshot("retry-artifact")
        .expect("snapshot")
        .0
        .iter()
        .any(|envelope| match &envelope.event {
            Event::Update { update } => update.pointer("/content/url").is_some(),
            _ => false,
        });
    assert!(
        externalized,
        "history should hold the image as an artifact URL"
    );
    hub.set_status("retry-artifact", Status::Crashed, Some("boom".to_owned()));

    hub.retry_turn("retry-artifact");

    let dispatched = rx.try_recv().expect("retry dispatches the prompt");
    assert_eq!(dispatched.text, "what is this");
    assert_eq!(dispatched.content.len(), 2);
    assert_eq!(
        dispatched.content[0]
            .get("data")
            .and_then(serde_json::Value::as_str),
        Some(data.as_str()),
        "the image bytes come back inline"
    );
    assert!(dispatched.content[0].get("url").is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn retry_refuses_visibly_when_the_stored_attachment_is_gone() {
    let hub = hub_with_session("retry-lost-artifact");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.push(
        "retry-lost-artifact",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": {
                    "type": "image",
                    "mimeType": "image/png",
                    "url": "/api/artifacts/0000000000000000000000000000000000000000000000000000000000000000.png"
                }
            }),
        },
    );
    hub.set_status(
        "retry-lost-artifact",
        Status::Crashed,
        Some("boom".to_owned()),
    );

    hub.retry_turn("retry-lost-artifact");

    assert!(
        rx.try_recv().is_err(),
        "retry dispatched a prompt without its image"
    );
    let detail = hub
        .snapshot("retry-lost-artifact")
        .expect("snapshot")
        .0
        .iter()
        .rev()
        .find_map(|envelope| match &envelope.event {
            Event::Lifecycle {
                status: Status::Crashed,
                detail: Some(detail),
            } => Some(detail.clone()),
            _ => None,
        })
        .expect("failure card detail");
    assert!(detail.contains("attachment"), "{detail}");
}

#[tokio::test]
async fn retry_of_an_attachment_only_prompt_keeps_a_different_queued_attachment() {
    let hub = hub_with_session("retry-attachment-only");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    let failed = serde_json::json!({"type": "image", "mimeType": "image/png", "data": "ZmFpbGVk"});
    hub.push(
        "retry-attachment-only",
        Event::Update {
            update: serde_json::json!({"sessionUpdate": "user_message_chunk", "content": failed}),
        },
    );
    hub.set_status(
        "retry-attachment-only",
        Status::Crashed,
        Some("boom".to_owned()),
    );
    hub.requeue_prompt(
        "retry-attachment-only",
        String::new(),
        vec![serde_json::json!({
            "type": "resource_link",
            "mimeType": "text/plain",
            "uri": "file:///notes.txt",
            "name": "notes.txt"
        })],
        Some("other-cmid".to_owned()),
    );

    hub.retry_turn("retry-attachment-only");

    let dispatched = rx.try_recv().expect("the failed image prompt is retried");
    assert_eq!(dispatched.content, vec![failed]);
    assert_eq!(
        hub.session_info("retry-attachment-only")
            .expect("session")
            .queue_count,
        1,
        "the unrelated queued attachment was consumed"
    );
}

#[tokio::test]
async fn retry_drains_rejected_prompt_without_enqueuing_a_copy() {
    let hub = hub_with_session("retry-requeued");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.push(
        "retry-requeued",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": {"text": "preserve this prompt"}
            }),
        },
    );
    hub.set_status(
        "retry-requeued",
        Status::Crashed,
        Some("stale cwd".to_owned()),
    );
    hub.requeue_prompt(
        "retry-requeued",
        "preserve this prompt".to_owned(),
        vec![serde_json::json!({"type": "text", "text": "preserve this prompt"})],
        Some("original-cmid".to_owned()),
    );

    hub.retry_turn("retry-requeued");

    let dispatched = rx.recv().await.expect("requeued prompt");
    assert_eq!(dispatched.text, "preserve this prompt");
    assert_eq!(dispatched.cmid.as_deref(), Some("original-cmid"));
    assert!(rx.try_recv().is_err(), "retry inserted a duplicate prompt");
    assert_eq!(
        hub.session_info("retry-requeued")
            .expect("session")
            .queue_count,
        0
    );
}

#[test]
fn persisted_hub_bounds_hot_history_but_keeps_total_count() {
    let health = std::sync::Arc::new(PersistenceHealth::default());
    let (sink, _rx) = StoreSink::channel(HOT_TAIL + HOT_TAIL_TRIM_BATCH + 2, health);
    let hub = Hub::with_store(Some(sink));
    hub.create_local_session(
        "bounded".to_owned(),
        "codex".to_owned(),
        "/tmp".to_owned(),
        "bounded".to_owned(),
        SessionOrigin::Api,
        false,
    );
    for n in 0..=HOT_TAIL + HOT_TAIL_TRIM_BATCH {
        hub.push(
            "bounded",
            Event::Update {
                update: serde_json::json!({"sessionUpdate": "plan", "n": n}),
            },
        );
    }
    assert_eq!(
        hub.event_total(),
        u64::try_from(HOT_TAIL + HOT_TAIL_TRIM_BATCH + 1).unwrap()
    );
    let (snapshot, reached_start) = hub.snapshot("bounded").expect("session snapshot");
    assert_eq!(snapshot.len(), SNAPSHOT_TAIL);
    assert!(!reached_start);
}

#[test]
fn hub_hot_history_reduces_stream_chunks_but_broadcasts_raw_frames() {
    let hub = hub_with_session("canonical-hot-tail");
    let mut live = hub.subscribe();
    for (seq, text) in ["hello ", "world"].into_iter().enumerate() {
        hub.push(
            "canonical-hot-tail",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "messageId": "answer",
                    "content": {"type": "text", "text": text},
                }),
            },
        );
        let frame = live.try_recv().expect("raw live frame");
        let Outbound::Event { envelope } = &**frame else {
            panic!("expected event");
        };
        assert_eq!(envelope.seq, u64::try_from(seq).unwrap());
    }

    let (snapshot, reached_start) = hub.snapshot("canonical-hot-tail").expect("snapshot");
    assert!(reached_start);
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].seq, 0);
    let Event::Update { update } = &snapshot[0].event else {
        panic!("expected canonical update");
    };
    assert_eq!(
        update
            .pointer("/content/text")
            .and_then(serde_json::Value::as_str),
        Some("hello world")
    );
    assert_eq!(hub.event_total(), 1);
}

#[test]
fn fanout_shares_compact_live_frame_while_hot_history_is_compact() {
    let hub = hub_with_session("shared-fanout");
    let mut first = hub.subscribe();
    let mut second = hub.subscribe();
    hub.push(
        "shared-fanout",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "image",
                "status": "completed",
                "content": [{"type": "content", "content": {"type": "text", "text": "saved"}}],
                "rawOutput": {"result": "x".repeat(128 * 1024)},
            }),
        },
    );

    let first = first.try_recv().expect("first compact frame");
    let second = second.try_recv().expect("second compact frame");
    assert!(Arc::ptr_eq(&first, &second));
    let Outbound::Event { envelope } = &**first else {
        panic!("expected event");
    };
    let Event::Update { update } = &envelope.event else {
        panic!("expected compact update");
    };
    assert!(update.get("rawOutput").is_none());
    assert_eq!(
        first.json().expect("shared json").len(),
        second.json().expect("shared json").len()
    );

    let (snapshot, _) = hub.snapshot("shared-fanout").expect("snapshot");
    let Event::Update { update } = &snapshot[0].event else {
        panic!("expected canonical update");
    };
    assert!(update.get("rawOutput").is_none());
}

#[test]
fn persist_queue_receives_compact_tool_events() {
    let health = Arc::new(PersistenceHealth::default());
    let (sink, mut rx) = StoreSink::channel(4, Arc::clone(&health));
    let hub = Hub::with_store(Some(sink));
    hub.create_local_session(
        "queued-compact".to_owned(),
        "codex".to_owned(),
        "/tmp".to_owned(),
        "queued-compact".to_owned(),
        SessionOrigin::Api,
        false,
    );
    while rx.try_recv().is_ok() {}
    hub.push(
        "queued-compact",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "image",
                "status": "completed",
                "content": [{"type": "content", "content": {"type": "text", "text": "saved"}}],
                "rawOutput": {"result": "x".repeat(128 * 1024)},
            }),
        },
    );
    assert!(health.pending_bytes() > 0);
    let write = rx.try_recv().expect("compact persist intent");
    let StoreWrite::AppendEvent(envelope) = &write else {
        panic!("expected compact persist intent");
    };
    let Event::Update { update } = &envelope.event else {
        panic!("expected update");
    };
    assert!(update.get("rawOutput").is_none());
    assert_eq!(health.pending_bytes(), 0);
}

#[test]
fn ingest_externalizes_live_images_before_fanout() {
    let root = std::env::temp_dir().join(format!(
        "cowboy-live-artifacts-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let hub = hub_with_session("live-image");
    hub.set_artifacts(crate::artifacts::ArtifactStore::new(root.clone()).unwrap());
    let mut live = hub.subscribe();
    let data = base64::engine::general_purpose::STANDARD.encode(vec![7_u8; 40_000]);
    hub.push(
        "live-image",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": {
                    "type": "image",
                    "mimeType": "image/png",
                    "data": data,
                }
            }),
        },
    );
    let frame = live.try_recv().expect("live image");
    let Outbound::Event { envelope } = &**frame else {
        panic!("expected event");
    };
    let Event::Update { update } = &envelope.event else {
        panic!("expected update");
    };
    assert!(update.pointer("/content/data").is_none());
    let url = update
        .pointer("/content/url")
        .and_then(serde_json::Value::as_str)
        .expect("artifact url");
    assert!(url.starts_with("/api/artifacts/"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn managed_images_survive_live_compaction_and_tool_update_merge() {
    use crate::managed_calls::media;
    let root = tempfile::tempdir().unwrap();
    let store = crate::artifacts::ArtifactStore::new(root.path().to_owned()).unwrap();
    let png = media::tests::png();
    let hub = hub_with_session("generated-image");
    hub.set_artifacts(store.clone());
    let mut success = serde_json::json!({"sessionUpdate":"tool_call_update","toolCallId":"image",
        "status":"completed","rawOutput":{"cowboyGeneratedImage":true,"result":"duplicate"},
        "content":[{"type":"content","content":{"type":"image","mimeType":"image/png",
        "data":base64::engine::general_purpose::STANDARD.encode(&png)}}]});
    for rejected in [false, true] {
        let cursor = hub.last_event_seq("generated-image").unwrap();
        hub.push(
            "generated-image",
            Event::Update {
                update: serde_json::json!({
            "sessionUpdate":"user_message_chunk","content":{"type":"text","text":"image"}}),
            },
        );
        hub.push(
            "generated-image",
            Event::Update {
                update: serde_json::json!({
            "sessionUpdate":"tool_call","toolCallId":"image","status":"in_progress"}),
            },
        );
        if rejected {
            success["content"][0]["content"]["data"] = serde_json::json!("invalid base64!");
        }
        hub.push(
            "generated-image",
            Event::Update {
                update: success.clone(),
            },
        );
        let turn = hub.managed_turn("generated-image", cursor).unwrap();
        assert_eq!(turn.generated_images.len(), 1);
        let update = &turn.generated_images[0];
        assert!(update.get("rawOutput").is_none());
        assert!(update["content"][0]["content"].get("data").is_none());
        let result = media::collect(update, &store);
        if rejected {
            assert_eq!(result.unwrap_err(), "artifact_invalid");
        } else {
            assert_eq!(result.unwrap(), [media::inspect(&png).unwrap()]);
        }
    }
    let cursor = hub.last_event_seq("generated-image").unwrap();
    hub.push(
        "generated-image",
        Event::Update {
            update: serde_json::json!({
        "sessionUpdate":"tool_call_update","toolCallId":"quota","status":"failed",
        "rawOutput":{"cowboyGeneratedImage":true,"failure":{"type":"usageLimitExceeded",
            "limitId":"image_generation","resetsAt":1234,"unused":"discard"}}}),
        },
    );
    let turn = hub.managed_turn("generated-image", cursor).unwrap();
    assert_eq!(turn.generated_images.len(), 1);
    assert_eq!(
        media::quota_failure(&turn.generated_images[0]),
        Some(serde_json::json!({
        "type":"usageLimitExceeded","limitId":"image_generation","resetsAt":1234}))
    );
    assert!(turn.generated_images[0].get("rawOutput").is_none());
}

#[test]
fn idle_hot_tail_is_tighter_than_a_busy_turn() {
    let health = Arc::new(PersistenceHealth::default());
    let (sink, _rx) = StoreSink::channel(32, health);
    let hub = Hub::with_store(Some(sink));
    hub.create_local_session(
        "idle-tail".to_owned(),
        "codex".to_owned(),
        "/tmp".to_owned(),
        "idle-tail".to_owned(),
        SessionOrigin::Api,
        false,
    );
    let payload = "x".repeat(200 * 1024);
    for n in 0..8 {
        hub.push(
            "idle-tail",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "plan",
                    "n": n,
                    "payload": payload,
                }),
            },
        );
    }
    let sessions = hub.inner.sessions.lock();
    let session = sessions.get("idle-tail").expect("session");
    assert!(session.log_bytes <= HOT_TAIL_IDLE_MAX_BYTES);
    assert!(session.log.len() < 8);
}

#[test]
fn do_shaped_working_set_keeps_shared_compact_frames() {
    let hub = Hub::new();
    for index in 0..17 {
        hub.create_local_session(
            format!("session-{index}"),
            "codex".to_owned(),
            "/tmp".to_owned(),
            format!("session-{index}"),
            SessionOrigin::Api,
            false,
        );
    }
    hub.set_status("session-0", Status::Busy, None);
    let mut terminals: Vec<_> = (0..3).map(|_| hub.subscribe()).collect();
    hub.push(
        "session-0",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "read",
                "status": "completed",
                "content": [{"type": "raw_output", "text": "ok"}],
                "rawOutput": {"result": "x".repeat(2_580_000)},
            }),
        },
    );
    let first = terminals[0].try_recv().expect("terminal 0");
    let second = terminals[1].try_recv().expect("terminal 1");
    let third = terminals[2].try_recv().expect("terminal 2");
    assert!(Arc::ptr_eq(&first, &second) && Arc::ptr_eq(&second, &third));
    let json = first.json().expect("shared json");
    assert!(json.len() < 64 * 1024, "compact live JSON {}", json.len());
    assert!(!json.contains("rawOutput"));
    let stats = hub.memory_stats();
    assert_eq!(stats.session_count, 17);
    assert!(stats.hot_log_bytes < HOT_TAIL_MAX_BYTES);
    assert!(stats.broadcast_last_bytes < 64 * 1024);
}

fn production_shaped_do_fixture() -> serde_json::Value {
    let artifact_root = std::env::temp_dir().join(format!(
        "cowboy-do-fixture-artifacts-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let _ = std::fs::remove_dir_all(&artifact_root);
    let hub = Hub::new();
    hub.set_artifacts(crate::artifacts::ArtifactStore::new(artifact_root.clone()).unwrap());
    for index in 0..17 {
        hub.create_local_session(
            format!("session-{index}"),
            "codex".to_owned(),
            "/tmp".to_owned(),
            format!("session-{index}"),
            SessionOrigin::Api,
            false,
        );
    }
    for index in 1..17 {
        for n in 0..4 {
            hub.push(
                &format!("session-{index}"),
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "plan",
                        "n": n,
                        "title": format!("idle-{index}-{n}"),
                    }),
                },
            );
        }
    }
    hub.set_status("session-0", Status::Busy, None);
    for text in ["The ", "quick ", "brown ", "fox "] {
        for _ in 0..20 {
            hub.push(
                "session-0",
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "agent_message_chunk",
                        "messageId": "turn",
                        "content": {"type": "text", "text": text},
                    }),
                },
            );
        }
    }
    let mut terminals: Vec<_> = (0..3).map(|_| hub.subscribe()).collect();
    hub.push(
        "session-0",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "read",
                "status": "completed",
                "content": [{"type": "raw_output", "text": "ok"}],
                "rawOutput": {"result": "x".repeat(2_580_000)},
            }),
        },
    );
    let image = base64::engine::general_purpose::STANDARD.encode(vec![7_u8; 40_000]);
    hub.push(
        "session-0",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": {
                    "type": "image",
                    "mimeType": "image/png",
                    "data": image,
                }
            }),
        },
    );
    let live_frames: Vec<String> = terminals
        .iter_mut()
        .map(|rx| {
            let mut last = String::new();
            while let Ok(frame) = rx.try_recv() {
                last = frame.json().expect("live json").to_owned();
            }
            last
        })
        .collect();
    let sessions = (0..17)
        .map(|index| {
            let id = format!("session-{index}");
            let (hot_tail, _) = hub.snapshot(&id).expect("snapshot");
            serde_json::json!({
                "id": id,
                "status": format!("{:?}", hub.status(&id).expect("status")).to_lowercase(),
                "hotTail": hot_tail,
                "hotTailBytes": hot_tail.iter().fold(0usize, |size, envelope| {
                    size.saturating_add(estimated_envelope_bytes(envelope))
                }),
            })
        })
        .collect::<Vec<_>>();
    let stats = hub.memory_stats();
    let _ = std::fs::remove_dir_all(artifact_root);
    serde_json::json!({
        "generatedBy": "cowboy Hub compact ingest",
        "terminals": 3,
        "rawFanoutWouldHaveBeen": 2_580_000 * 3
            + base64::engine::general_purpose::STANDARD.encode(vec![7_u8; 40_000]).len() * 3,
        "hubHotLogBytes": stats.hot_log_bytes,
        "hubBroadcastLastBytes": stats.broadcast_last_bytes,
        "liveFrames": live_frames,
        "sessions": sessions,
    })
}

#[test]
fn exports_production_shaped_do_fixture() {
    let fixture = production_shaped_do_fixture();
    let live = fixture["liveFrames"].as_array().expect("live frames");
    assert_eq!(live.len(), 3);
    assert!(live.iter().all(|frame| frame == &live[0]));
    let live_json = live[0].as_str().expect("live json");
    assert!(!live_json.contains("rawOutput"));
    assert!(
        live_json.contains("/api/artifacts/") || live_json.contains("\"type\":\"image\""),
        "live image should be externalized or still an image block: {live_json}"
    );
    let sessions = fixture["sessions"].as_array().expect("sessions");
    assert_eq!(sessions.len(), 17);
    let hot_log_bytes = sessions.iter().fold(0usize, |size, session| {
        size.saturating_add(session["hotTailBytes"].as_u64().unwrap_or(0) as usize)
    });
    assert!(hot_log_bytes < HOT_TAIL_MAX_BYTES + HOT_TAIL_IDLE_MAX_BYTES * 16);
    assert!(
        fixture["hubHotLogBytes"].as_u64().unwrap() < 64 * 1024,
        "compact hub tails should stay tiny, got {}",
        fixture["hubHotLogBytes"]
    );
    if let Ok(path) = std::env::var("COWBOY_DO_FIXTURE_OUT") {
        std::fs::write(&path, serde_json::to_vec_pretty(&fixture).unwrap())
            .unwrap_or_else(|error| panic!("write {path}: {error}"));
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// Extreme DO fixture: dozens of sessions, filled compact tails, many
/// terminals, fat raw payloads stripped at ingest, plus a count of
/// SQLite-only archive rows for the worker to synthesize. The Hub still
/// trims idle/busy tails so the exported `hotTail` is what a focused
/// isolate should load — not the full durable log.
fn extreme_do_fixture() -> serde_json::Value {
    let sessions = env_usize("COWBOY_DO_SESSIONS", 50);
    let focused = env_usize("COWBOY_DO_FOCUSED", 4).min(sessions);
    let terminals = env_usize("COWBOY_DO_TERMINALS", 8);
    let idle_chunk_bytes = env_usize("COWBOY_DO_IDLE_CHUNK_BYTES", 8 * 1024);
    let idle_chunks = env_usize("COWBOY_DO_IDLE_CHUNKS", 8);
    let busy_chunks = env_usize("COWBOY_DO_BUSY_CHUNKS", 24);
    let tools = env_usize("COWBOY_DO_TOOLS", 200);
    let fat_events = env_usize("COWBOY_DO_FAT_EVENTS", 1);
    let archive_rows = env_usize("COWBOY_DO_ARCHIVE_ROWS", 8_000);
    let archive_bytes = env_usize("COWBOY_DO_ARCHIVE_BYTES", 512);

    let artifact_root = std::env::temp_dir().join(format!(
        "cowboy-do-extreme-artifacts-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let _ = std::fs::remove_dir_all(&artifact_root);
    let health = Arc::new(PersistenceHealth::default());
    let (sink, _rx) = StoreSink::channel(64_000, health);
    let hub = Hub::with_store(Some(sink));
    hub.set_artifacts(crate::artifacts::ArtifactStore::new(artifact_root.clone()).unwrap());

    for index in 0..sessions {
        hub.create_local_session(
            format!("session-{index}"),
            "codex".to_owned(),
            "/tmp".to_owned(),
            format!("session-{index}"),
            SessionOrigin::Api,
            false,
        );
    }
    let pad = "x".repeat(idle_chunk_bytes);
    for index in focused..sessions {
        let id = format!("session-{index}");
        for n in 0..idle_chunks {
            hub.push(
                &id,
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "plan",
                        "n": n,
                        "payload": pad,
                    }),
                },
            );
        }
    }
    for index in 0..focused {
        let id = format!("session-{index}");
        hub.set_status(&id, Status::Busy, None);
        for n in 0..busy_chunks {
            hub.push(
                &id,
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "plan",
                        "n": n,
                        "payload": pad,
                    }),
                },
            );
        }
    }
    for n in 0..tools {
        hub.push(
            "session-0",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": format!("tool-{n}"),
                    "status": "completed",
                    "content": [{"type": "raw_output", "text": format!("done-{n}")}],
                }),
            },
        );
    }
    let mut live_rx: Vec<_> = (0..terminals).map(|_| hub.subscribe()).collect();
    let mut raw_fanout = 0usize;
    for n in 0..fat_events {
        raw_fanout = raw_fanout.saturating_add(2_580_000 * terminals);
        hub.push(
            "session-0",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": format!("fat-{n}"),
                    "status": "completed",
                    "content": [{"type": "raw_output", "text": "ok"}],
                    "rawOutput": {"result": "x".repeat(2_580_000)},
                }),
            },
        );
    }
    let image = base64::engine::general_purpose::STANDARD.encode(vec![7_u8; 40_000]);
    raw_fanout = raw_fanout.saturating_add(image.len() * terminals);
    hub.push(
        "session-0",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": {
                    "type": "image",
                    "mimeType": "image/png",
                    "data": image,
                }
            }),
        },
    );
    let live_frames: Vec<String> = live_rx
        .iter_mut()
        .map(|rx| {
            let mut last = String::new();
            while let Ok(frame) = rx.try_recv() {
                last = frame.json().expect("live json").to_owned();
            }
            last
        })
        .collect();

    let session_rows = {
        let locked = hub.inner.sessions.lock();
        (0..sessions)
            .map(|index| {
                let id = format!("session-{index}");
                let session = locked.get(&id).expect("session");
                serde_json::json!({
                    "id": id,
                    "status": format!("{:?}", session.meta.status).to_lowercase(),
                    "focused": index < focused,
                    "hotTail": session.log,
                    "hotTailBytes": session.log_bytes,
                })
            })
            .collect::<Vec<_>>()
    };
    let stats = hub.memory_stats();
    let _ = std::fs::remove_dir_all(artifact_root);
    serde_json::json!({
        "generatedBy": "cowboy Hub extreme compact ingest",
        "profile": "extreme",
        "terminals": terminals,
        "focusedSessionIds": (0..focused).map(|index| format!("session-{index}")).collect::<Vec<_>>(),
        "rawFanoutWouldHaveBeen": raw_fanout,
        "hubHotLogBytes": stats.hot_log_bytes,
        "hubBroadcastLastBytes": stats.broadcast_last_bytes,
        "archiveRows": archive_rows,
        "archivePayloadBytes": archive_bytes,
        "liveFrames": live_frames,
        "sessions": session_rows,
    })
}

#[test]
fn exports_extreme_do_fixture() {
    let fixture = extreme_do_fixture();
    let sessions = fixture["sessions"].as_array().expect("sessions");
    assert!(
        sessions.len() >= 20,
        "extreme mock needs dozens of sessions"
    );
    let live = fixture["liveFrames"].as_array().expect("live frames");
    assert!(live.len() >= 5);
    assert!(live.iter().all(|frame| frame == &live[0]));
    let live_json = live[0].as_str().expect("live json");
    assert!(!live_json.contains("rawOutput"));
    assert!(live_json.contains("/api/artifacts/"));
    let focused = sessions
        .iter()
        .filter(|session| session["focused"].as_bool() == Some(true))
        .count();
    assert!(focused >= 2);
    let hub_bytes = fixture["hubHotLogBytes"].as_u64().unwrap();
    assert!(
        hub_bytes < 80 * 1024 * 1024,
        "even extreme compact tails must stay under the DO isolate, got {hub_bytes}"
    );
    if let Ok(path) = std::env::var("COWBOY_DO_FIXTURE_OUT") {
        std::fs::write(&path, serde_json::to_vec(&fixture).unwrap())
            .unwrap_or_else(|error| panic!("write {path}: {error}"));
    }
}

#[test]
fn persisted_hub_bounds_canonical_hot_history_by_payload_bytes() {
    let health = std::sync::Arc::new(PersistenceHealth::default());
    let (sink, _rx) = StoreSink::channel(32, health);
    let hub = Hub::with_store(Some(sink));
    hub.create_local_session(
        "byte-hot-tail".to_owned(),
        "codex".to_owned(),
        "/tmp".to_owned(),
        "byte-hot-tail".to_owned(),
        SessionOrigin::Api,
        false,
    );
    let payload = "x".repeat(384 * 1024);
    for n in 0..12 {
        hub.push(
            "byte-hot-tail",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "plan",
                    "n": n,
                    "payload": payload,
                }),
            },
        );
    }

    let sessions = hub.inner.sessions.lock();
    let session = sessions.get("byte-hot-tail").expect("session");
    assert!(session.log.len() < 12);
    assert!(session.log_bytes <= HOT_TAIL_MAX_BYTES);
    assert!(!session.reached_start);
    assert_eq!(session.event_count, 12);
}

#[test]
fn persisted_hub_keeps_one_oversized_newest_event() {
    let health = std::sync::Arc::new(PersistenceHealth::default());
    let (sink, _rx) = StoreSink::channel(4, health);
    let hub = Hub::with_store(Some(sink));
    hub.create_local_session(
        "oversized-hot-tail".to_owned(),
        "codex".to_owned(),
        "/tmp".to_owned(),
        "oversized-hot-tail".to_owned(),
        SessionOrigin::Api,
        false,
    );
    hub.push(
        "oversized-hot-tail",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "tool_call_update",
                "payload": "x".repeat(HOT_TAIL_MAX_BYTES + 1),
            }),
        },
    );

    let sessions = hub.inner.sessions.lock();
    let session = sessions.get("oversized-hot-tail").expect("session");
    assert_eq!(session.log.len(), 1);
    assert!(session.log_bytes > HOT_TAIL_MAX_BYTES);
    assert_eq!(session.event_count, 1);
}

#[test]
fn session_snapshot_is_byte_bounded_and_keeps_the_newest_event() {
    let hub = hub_with_session("byte-bounded");
    let payload = "x".repeat(96 * 1024);
    for n in 0..4 {
        hub.push(
            "byte-bounded",
            Event::Update {
                update: serde_json::json!({"sessionUpdate": "tool_call_update", "n": n, "payload": payload}),
            },
        );
    }

    let (snapshot, reached_start) = hub.snapshot("byte-bounded").expect("session snapshot");
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].seq, 3);
    assert!(!reached_start);
    assert!(serde_json::to_vec(&snapshot[0]).unwrap().len() < SNAPSHOT_MAX_BYTES);
}

#[test]
fn session_snapshot_does_not_split_a_rich_user_prompt() {
    let hub = hub_with_session("rich-prompt-boundary");
    let payload = "x".repeat(96 * 1024);
    hub.push(
        "rich-prompt-boundary",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": { "type": "image", "data": payload, "mimeType": "image/jpeg" }
            }),
        },
    );
    hub.push(
        "rich-prompt-boundary",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": { "type": "text", "text": "adjust this image" }
            }),
        },
    );
    hub.push(
        "rich-prompt-boundary",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "tool_call_update",
                "payload": payload
            }),
        },
    );

    let (snapshot, reached_start) = hub
        .snapshot("rich-prompt-boundary")
        .expect("session snapshot");
    assert_eq!(snapshot.len(), 3);
    assert_eq!(snapshot[0].seq, 0);
    assert!(is_user_message_chunk(&snapshot[0]));
    assert!(is_user_message_chunk(&snapshot[1]));
    assert!(reached_start);
    assert!(
        serde_json::to_vec(&snapshot).unwrap().len() > SNAPSHOT_MAX_BYTES,
        "message atomicity may exceed the soft byte budget"
    );
}

#[test]
fn cursor_history_ignores_sequence_gaps() {
    let hub = hub_with_session("cursor");
    {
        let mut sessions = hub.inner.sessions.lock();
        let session = sessions.get_mut("cursor").expect("session");
        session.log = (0..450)
            .map(|index| Envelope {
                session_id: "cursor".to_owned(),
                seq: u64::try_from(index * 3).unwrap(),
                event: Event::TurnEnd {
                    usage: None,
                    stop_reason: "done".to_owned(),
                },
                cmid: None,
            })
            .collect();
        session.reached_start = true;
    }
    let mut cursor = Some(u64::MAX);
    let mut seen = Vec::new();
    while let Some(before_seq) = cursor {
        let (page, next, reached_start) = hub.history_page("cursor", before_seq).unwrap();
        assert!(!page.is_empty());
        assert!(page.len() <= HISTORY_PAGE);
        seen.splice(0..0, page.iter().map(|event| event.seq));
        cursor = next;
        if reached_start {
            assert_eq!(cursor, None);
            break;
        }
    }
    assert_eq!(seen.len(), 450);
    assert!(seen.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn cursor_history_is_byte_bounded_and_always_advances() {
    let hub = hub_with_session("history-bytes");
    {
        let mut sessions = hub.inner.sessions.lock();
        let session = sessions.get_mut("history-bytes").expect("session");
        session.log = (0..3)
            .map(|seq| Envelope {
                session_id: "history-bytes".to_owned(),
                seq,
                event: Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": format!("tool-{seq}"),
                        "content": "x".repeat(300 * 1024),
                    }),
                },
                cmid: None,
            })
            .collect();
        session.reached_start = true;
    }

    let (newest, cursor, reached_start) = hub.history_page("history-bytes", u64::MAX).unwrap();
    assert_eq!(newest.len(), 1);
    assert_eq!(newest[0].seq, 2);
    assert!(!reached_start);

    let (middle, next, reached_start) = hub.history_page("history-bytes", cursor.unwrap()).unwrap();
    assert_eq!(middle.len(), 1);
    assert_eq!(middle[0].seq, 1);
    assert!(!reached_start);
    assert!(next.unwrap() < 2);
}

#[test]
fn question_history_returns_one_complete_prompt_rooted_page() {
    let hub = hub_with_session("question-page");
    for (kind, text) in [
        ("user_message_chunk", "first question"),
        ("agent_message_chunk", "first answer"),
        ("tool_call_update", "first tool"),
        ("user_message_chunk", "second question"),
        ("agent_message_chunk", "second answer"),
    ] {
        hub.push(
            "question-page",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": kind,
                    "content": {"text": text},
                }),
            },
        );
    }

    let (page, cursor, reached_start) = hub.question_page_before("question-page", 3).unwrap();
    assert_eq!(
        page.iter().map(|event| event.seq).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(cursor, None);
    assert!(reached_start);

    let (latest, before, total, exact) = hub
        .question_page_summaries("question-page", None, 1)
        .expect("latest question summary");
    assert_eq!(latest[0].title, "second question");
    assert_eq!(latest[0].ordinal, 2);
    assert_eq!(before, Some(3));
    assert_eq!(total, 2);
    assert!(exact);

    let (earlier, before, _, _) = hub
        .question_page_summaries("question-page", before, 1)
        .expect("earlier question summary");
    assert_eq!(earlier[0].title, "first question");
    assert_eq!(earlier[0].ordinal, 1);
    assert_eq!(before, None);

    let lazy_page = hub
        .question_page_at("question-page", 0)
        .expect("lazy question page");
    assert_eq!(
        lazy_page.iter().map(|event| event.seq).collect::<Vec<_>>(),
        [0, 1, 2]
    );
}

#[test]
fn question_history_stops_before_background_output_after_turn_end() {
    let hub = hub_with_session("question-tail");
    hub.push(
        "question-tail",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": {"text": "show me the logs"},
            }),
        },
    );
    hub.push(
        "question-tail",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {"text": "the watcher is running"},
            }),
        },
    );
    hub.push(
        "question-tail",
        Event::TurnEnd {
            usage: None,
            stop_reason: "end_turn".to_owned(),
        },
    );
    for index in 0..100 {
        hub.push(
            "question-tail",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "watcher",
                    "line": index,
                }),
            },
        );
    }

    let (page, _, _) = hub
        .question_page_before("question-tail", 103)
        .expect("question page");
    assert_eq!(
        page.iter().map(|event| event.seq).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    let lazy = hub
        .question_page_at("question-tail", 0)
        .expect("lazy question page");
    assert_eq!(
        lazy.iter().map(|event| event.seq).collect::<Vec<_>>(),
        [0, 1, 2]
    );
}

#[test]
fn context_management_commands_are_not_question_pages() {
    let hub = hub_with_session("question-commands");
    for text in ["first question", "/compact", "second question"] {
        hub.push(
            "question-commands",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"text": text},
                }),
            },
        );
        hub.push(
            "question-commands",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"text": "response"},
                }),
            },
        );
    }

    let (pages, next, total, exact) = hub
        .question_page_summaries("question-commands", None, 64)
        .expect("question summaries");
    assert_eq!(pages.len(), 2);
    assert_eq!(next, None);
    assert_eq!(total, 2);
    assert!(exact);
}

#[test]
fn native_thread_becomes_resumable_only_after_a_user_turn_in_current_context() {
    let hub = hub_with_session("native-durability");
    hub.set_agent_session_id("native-durability", "thread-empty".to_owned());
    assert_eq!(hub.agent_session_id_for_resume("native-durability"), None);

    hub.push(
        "native-durability",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": {"type": "text", "text": "first prompt"},
            }),
        },
    );
    assert_eq!(
        hub.agent_session_id_for_resume("native-durability")
            .as_deref(),
        Some("thread-empty")
    );

    hub.mark_context_cleared("native-durability");
    hub.set_agent_session_id("native-durability", "thread-after-clear".to_owned());
    assert_eq!(hub.agent_session_id_for_resume("native-durability"), None);

    hub.push(
        "native-durability",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": {"type": "text", "text": "new context prompt"},
            }),
        },
    );
    assert_eq!(
        hub.agent_session_id_for_resume("native-durability")
            .as_deref(),
        Some("thread-after-clear")
    );
}

#[test]
fn context_reset_forgets_the_discarded_context_usage() {
    let hub = hub_with_session("clear-usage");
    hub.set_session_usage(
        "clear-usage",
        crate::agent_model::SessionUsage {
            used: 870,
            size: 1000,
            raw: serde_json::Value::Null,
            observed_at_ms: 1,
        },
    );
    hub.set_background_tasks("clear-usage", 2);

    hub.prepare_context_reset("clear-usage");

    let meta = hub.session_info("clear-usage").expect("session").meta;
    assert_eq!((meta.context_used, meta.context_size), (0, 0));
    assert!(meta.usage.is_none());
    assert_eq!(meta.background_tasks, 0);
    assert!(meta.agent_session_id.is_none());
}

#[test]
fn clear_transcript_discards_history_before_the_new_boundary() {
    let hub = hub_with_session("clear-history");
    hub.push(
        "clear-history",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "old"},
            }),
        },
    );

    hub.clear_transcript("clear-history");
    hub.mark_context_cleared("clear-history");

    let (events, reached_start) = hub.snapshot("clear-history").expect("snapshot");
    assert!(reached_start);
    assert_eq!(events.len(), 1);
    assert!(is_context_cleared(&events[0]));
    assert_eq!(
        hub.session_info("clear-history")
            .expect("session")
            .event_count,
        1
    );
}

fn provider_reload_fixture() -> (Hub, SessionMeta) {
    let hub = hub_with_session("provider-reload");
    hub.push("provider-reload", Event::Update {
        update: serde_json::json!({"sessionUpdate": "user_message_chunk", "content": {"text": "keep history"}}),
    });
    hub.set_agent_session_id("provider-reload", "native-thread".to_owned());
    hub.set_status("provider-reload", Status::Running, None);
    hub.set_config_preference(
        "provider-reload",
        "model".to_owned(),
        serde_json::json!("saved-model"),
    )
    .expect("preference");
    let meta = hub.session_info("provider-reload").expect("session").meta;
    (hub, meta)
}

#[test]
fn native_binding_projection_is_scoped_idempotent_and_preserves_user_state() {
    let (hub, before) = provider_reload_fixture();
    let account = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    {
        let mut sessions = hub.inner.sessions.lock();
        let session = sessions.get_mut(&before.id).unwrap();
        session.meta.provider = "claude-code".into();
        session.meta.provider_account_id = account;
        session.meta.provider_auth_generation = Some(1);
        session.meta.status = Status::Exited;
    }
    let expected = hub.session_info(&before.id).unwrap().meta;
    let target = crate::provider_account::NativeProfileReceipt {
        account_id: expected.provider_account_id.clone(),
        provider_id: expected.provider.clone(),
        request_id: "login".into(),
        generation: 2,
        method: "native".into(),
        provider_version: "new".into(),
        generation_digest: "new-digest".into(),
        auth_contract_fingerprint: "fixture".into(),
        identity: crate::provider_account::NativeIdentity {
            authority: "fixture".into(),
            subject: "private".into(),
            tenant: None,
        },
    };
    let behavior = crate::provider::legacy_behavior("claude-code");
    hub.add_draft(&before.id, "keep draft".into(), vec![], None);
    let history = hub.snapshot(&before.id).unwrap().0;
    let mut wrong = expected.clone();
    wrong.machine_id = "different".into();
    assert!(
        hub.project_native_binding(&wrong, &target, &behavior)
            .is_err()
    );
    hub.set_status(&before.id, Status::Running, None);
    assert!(
        hub.project_native_binding(&expected, &target, &behavior)
            .is_err()
    );
    hub.set_status(&before.id, Status::Exited, None);
    hub.project_native_binding(&expected, &target, &behavior)
        .unwrap();
    let epoch = hub.inner.sessions.lock()[&before.id].lifecycle_epoch;
    hub.project_native_binding(&expected, &target, &behavior)
        .unwrap();
    assert_eq!(hub.inner.sessions.lock()[&before.id].lifecycle_epoch, epoch);
    let after = hub.session_info(&before.id).unwrap();
    assert_eq!(after.meta.provider_auth_generation, Some(2));
    assert!(after.meta.agent_session_id.is_none());
    assert_eq!(after.meta.status, Status::Exited);
    native_readmission_declaration_contract(after.meta.clone(), &target);
    assert_eq!(after.drafts_count, 1);
    assert_eq!(after.meta.cwd, expected.cwd);
    assert_eq!(after.meta.title, expected.title);
    assert_eq!(
        hub.config_preferences(&before.id).unwrap()["model"],
        "saved-model"
    );
    assert!(hub.snapshot(&before.id).unwrap().0.len() >= history.len());
    let mut stale = target.clone();
    stale.generation = 3;
    assert!(
        hub.project_native_binding(&expected, &stale, &behavior)
            .is_err()
    );
    assert_eq!(
        hub.session_info(&before.id)
            .unwrap()
            .meta
            .provider_auth_generation,
        Some(2)
    );
}

fn native_readmission_declaration_contract(
    mut meta: SessionMeta,
    target: &crate::provider_account::NativeProfileReceipt,
) {
    meta.owner_user_id = Some("owner".into());
    assert!(meta.require_native_readmission(target).is_ok());
    meta.machine_id = "hawk".into();
    assert!(meta.require_runtime_launch().is_ok());
    for field in 0..9 {
        let mut wrong = meta.clone();
        match field {
            0 => wrong.owner_user_id = None,
            1 => wrong.owner_user_id = Some(String::new()),
            2 => wrong.closing = true,
            3 => wrong.machine_id = "local".into(),
            4 => wrong.provider = "codex".into(),
            5 => wrong.provider_version.clear(),
            6 => wrong.provider_generation_digest.clear(),
            7 => wrong.provider_auth_generation = Some(0),
            _ => wrong.provider_behavior = None,
        }
        assert!(wrong.require_runtime_launch().is_err());
    }
    for field in 0..9 {
        let mut wrong = meta.clone();
        match field {
            0 => wrong.owner_user_id = None,
            1 => wrong.status = Status::Running,
            2 => wrong.closing = true,
            3 => wrong.agent_session_id = Some("old-thread".into()),
            4 => wrong.provider_account_id = crate::provider_account::ProviderAccountId::default(),
            5 => wrong.provider = "other".into(),
            6 => wrong.provider_version = "old".into(),
            7 => wrong.provider_generation_digest = "old".into(),
            _ => wrong.provider_auth_generation = None,
        }
        assert!(
            wrong.require_native_readmission(target).is_err(),
            "field {field}"
        );
    }
    let mut wrong_target = target.clone();
    wrong_target.generation = 0;
    meta.provider_auth_generation = Some(0);
    assert!(meta.require_native_readmission(&wrong_target).is_err());
}

#[tokio::test]
async fn provider_reload_preserves_identity_and_fences_new_prompts() {
    let (hub, before) = provider_reload_fixture();
    let behavior = crate::provider::legacy_behavior("codex");
    hub.add_draft(&before.id, "saved draft".to_owned(), vec![], None);
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    let (history, _) = hub.snapshot(&before.id).expect("history");
    hub.begin_provider_reload(&before, "new-version", "new-digest", &behavior, false, None)
        .expect("reload");
    hub.submit(&before.id, "racing prompt".to_owned(), vec![], None);
    assert!(rx.try_recv().is_err(), "prompt must wait for replacement");
    let after = hub.session_info(&before.id).expect("session");
    assert_eq!(after.meta.provider_version, "new-version");
    assert_eq!(after.meta.provider_generation_digest, "new-digest");
    assert_eq!(after.meta.provider_behavior, Some(behavior));
    assert_eq!(after.meta.agent_session_id, before.agent_session_id);
    assert_eq!(
        after.meta.provider_auth_generation,
        before.provider_auth_generation
    );
    assert_eq!(after.meta.cwd, before.cwd);
    assert_eq!(after.meta.title, before.title);
    assert_eq!(after.meta.machine_id, before.machine_id);
    assert_eq!(after.meta.status, Status::Starting);
    assert_eq!(after.drafts_count, 1);
    assert_eq!(after.queue_count, 1);
    assert_eq!(
        hub.config_preferences(&before.id).unwrap()["model"],
        "saved-model"
    );
    assert_eq!(
        serde_json::to_value(&hub.snapshot(&before.id).unwrap().0[..history.len()]).unwrap(),
        serde_json::to_value(history).unwrap()
    );
}

#[test]
fn named_release_changes_cannot_repin_or_reload_an_old_login() {
    let (hub, meta) = provider_reload_fixture();
    {
        let mut sessions = hub.inner.sessions.lock();
        let session = sessions.get_mut(&meta.id).unwrap();
        session.meta.provider_account_id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        session.meta.provider_auth_generation = Some(7);
    }
    let behavior = crate::provider::legacy_behavior("codex");
    for automatic in [false, true] {
        let before = hub.session_info(&meta.id).unwrap().meta;
        assert!(
            hub.begin_provider_reload(&before, "new", "new-digest", &behavior, automatic, None)
                .unwrap_err()
                .contains("verified login transition")
        );
        assert_eq!(
            serde_json::to_value(hub.session_info(&meta.id).unwrap().meta).unwrap(),
            serde_json::to_value(before).unwrap()
        );
    }
    hub.set_status(&meta.id, Status::Exited, None);
    let before = hub.session_info(&meta.id).unwrap().meta;
    assert!(
        hub.repin_dormant_provider(&before, "new", "new-digest", &behavior, None)
            .unwrap_err()
            .contains("verified login transition")
    );
    assert_eq!(
        serde_json::to_value(hub.session_info(&meta.id).unwrap().meta).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}

#[test]
fn a_verified_carry_moves_one_named_session_on_its_existing_login() {
    let (hub, meta) = provider_reload_fixture();
    let account: crate::provider_account::ProviderAccountId =
        uuid::Uuid::new_v4().to_string().try_into().unwrap();
    {
        let mut sessions = hub.inner.sessions.lock();
        let session = sessions.get_mut(&meta.id).unwrap();
        session.meta.provider_account_id = account.clone();
        session.meta.provider_auth_generation = Some(7);
    }
    let behavior = crate::provider::legacy_behavior("codex");
    let carry = NamedReleaseCarry {
        session_id: meta.id.clone(),
        auth_generation: 7,
        version: "new".into(),
        digest: "new-digest".into(),
    };
    // A proof for another session, login generation or release is refused.
    for wrong in [
        NamedReleaseCarry {
            session_id: "other".into(),
            ..carry.clone()
        },
        NamedReleaseCarry {
            auth_generation: 8,
            ..carry.clone()
        },
        NamedReleaseCarry {
            digest: "other-digest".into(),
            ..carry.clone()
        },
    ] {
        let before = hub.session_info(&meta.id).unwrap().meta;
        assert!(
            hub.begin_provider_reload(&before, "new", "new-digest", &behavior, false, Some(&wrong))
                .unwrap_err()
                .contains("verified login transition")
        );
    }
    let before = hub.session_info(&meta.id).unwrap().meta;
    hub.begin_provider_reload(&before, "new", "new-digest", &behavior, false, Some(&carry))
        .expect("verified carry reloads");
    let moved = hub.session_info(&meta.id).unwrap().meta;
    assert_eq!(
        (
            moved.provider_version.as_str(),
            moved.provider_generation_digest.as_str()
        ),
        ("new", "new-digest")
    );
    // The login binding itself never changes.
    assert_eq!(moved.provider_account_id, account);
    assert_eq!(moved.provider_auth_generation, Some(7));

    let (hub, meta) = provider_reload_fixture();
    {
        let mut sessions = hub.inner.sessions.lock();
        let session = sessions.get_mut(&meta.id).unwrap();
        session.meta.provider_account_id = account;
        session.meta.provider_auth_generation = Some(7);
    }
    hub.set_status(&meta.id, Status::Exited, None);
    let before = hub.session_info(&meta.id).unwrap().meta;
    hub.repin_dormant_provider(&before, "new", "new-digest", &behavior, Some(&carry))
        .expect("verified carry re-pins a dormant session");
    assert_eq!(
        hub.session_info(&meta.id).unwrap().meta.provider_version,
        "new"
    );
}

#[test]
fn provider_reload_publishes_its_update_until_the_worker_settles() {
    let (hub, before) = provider_reload_fixture();
    let behavior = crate::provider::legacy_behavior("codex");
    hub.begin_provider_reload(&before, "new-version", "new-digest", &behavior, true, None)
        .expect("reload");
    let starting = hub.session_info(&before.id).expect("session").meta;
    let update = starting.provider_update.clone().expect("published update");
    assert_eq!(update.from, before.provider_version);
    assert_eq!(update.to, "new-version");
    assert!(update.automatic);
    assert!(update.started_at_ms > 0);
    let wire = serde_json::to_value(&starting).expect("serialize");
    assert_eq!(wire["provider_update"]["automatic"], true);
    hub.set_status(&before.id, Status::Starting, None);
    assert!(
        hub.session_info(&before.id)
            .unwrap()
            .meta
            .provider_update
            .is_some()
    );
    hub.set_status(&before.id, Status::Running, None);
    let settled = hub.session_info(&before.id).expect("session").meta;
    assert_eq!(settled.provider_update, None);
    assert!(
        serde_json::to_value(&settled)
            .unwrap()
            .get("provider_update")
            .is_none()
    );
}

#[test]
fn unattended_provider_reload_waits_for_background_work() {
    let (hub, _) = provider_reload_fixture();
    let behavior = crate::provider::legacy_behavior("codex");
    hub.set_background_tasks("provider-reload", 1);
    let working = hub.session_info("provider-reload").expect("session").meta;
    assert_eq!(
        hub.begin_provider_reload(&working, "new-version", "new-digest", &behavior, true, None),
        Err("session has background work in progress".to_owned())
    );
    assert_eq!(
        hub.session_info(&working.id).unwrap().meta.status,
        Status::Running
    );
    // An explicit Reload stays the user's decision.
    hub.begin_provider_reload(
        &working,
        "new-version",
        "new-digest",
        &behavior,
        false,
        None,
    )
    .expect("explicit reload");
}

#[test]
fn only_a_crashed_unattended_update_holds_its_release_back() {
    let hour = std::time::Duration::from_hours(1);
    let behavior = crate::provider::legacy_behavior("codex");
    let failed = |hub: &Hub, meta: &SessionMeta| {
        hub.automatic_update_failed_within(&meta.machine_id, &meta.provider, "new-digest", hour)
    };
    for (automatic, outcome, held) in [
        (true, Status::Running, false),
        (false, Status::Crashed, false),
        (true, Status::Crashed, true),
    ] {
        let (hub, before) = provider_reload_fixture();
        hub.begin_provider_reload(
            &before,
            "new-version",
            "new-digest",
            &behavior,
            automatic,
            None,
        )
        .expect("reload");
        assert!(!failed(&hub, &before));
        hub.set_status(&before.id, outcome, None);
        assert_eq!(failed(&hub, &before), held, "{automatic} {outcome:?}");
        assert!(!hub.automatic_update_failed_within(
            &before.machine_id,
            &before.provider,
            "new-digest",
            std::time::Duration::ZERO
        ));
        // A later crash of the settled session is not the update's.
        hub.set_status(&before.id, Status::Running, None);
        hub.set_status(&before.id, Status::Crashed, None);
        assert_eq!(failed(&hub, &before), held);
    }
}

#[test]
fn provider_reload_rejects_a_session_hibernated_after_selection() {
    let behavior = crate::provider::legacy_behavior("codex");
    let (hub, before) = provider_reload_fixture();
    hub.set_status(&before.id, Status::Exited, None);
    assert!(
        hub.begin_provider_reload(&before, "new-version", "new-digest", &behavior, true, None)
            .is_err()
    );
    assert_eq!(hub.status(&before.id), Some(Status::Exited));
}

#[test]
fn a_crashed_unattended_update_holds_its_session_until_it_is_used() {
    let behavior = crate::provider::legacy_behavior("codex");
    let (hub, before) = provider_reload_fixture();
    hub.begin_provider_reload(&before, "new-version", "new-digest", &behavior, true, None)
        .expect("reload");
    hub.set_status(&before.id, Status::Crashed, None);
    assert!(hub.automatic_update_failed_since_activity(&before.id));
    // Anything the user does in the session lifts the hold.
    hub.push(
        &before.id,
        Event::Update {
            update: serde_json::json!({"sessionUpdate": "user_message_chunk", "content": {"text": "back"}}),
        },
    );
    assert!(!hub.automatic_update_failed_since_activity(&before.id));
    // So does an update that finishes, however it was started.
    let (hub, before) = provider_reload_fixture();
    hub.begin_provider_reload(&before, "new-version", "new-digest", &behavior, true, None)
        .expect("reload");
    hub.set_status(&before.id, Status::Crashed, None);
    hub.set_status(&before.id, Status::Running, None);
    let crashed = hub.session_info(&before.id).expect("session").meta;
    hub.begin_provider_reload(&crashed, "newer", "newer-digest", &behavior, false, None)
        .expect("explicit reload");
    hub.set_status(&before.id, Status::Running, None);
    assert!(!hub.automatic_update_failed_since_activity(&before.id));
    assert!(!hub.automatic_update_failed_since_activity("unknown"));
}

#[tokio::test]
async fn provider_reload_rejects_a_prompt_that_won_the_race() {
    let (hub, before) = provider_reload_fixture();
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.submit(&before.id, "active prompt".to_owned(), vec![], None);
    assert!(rx.recv().await.is_some());
    let behavior = crate::provider::legacy_behavior("codex");
    assert!(
        hub.begin_provider_reload(&before, "new", "new", &behavior, false, None)
            .unwrap_err()
            .contains("current turn")
    );
    assert_eq!(
        hub.session_info(&before.id)
            .unwrap()
            .meta
            .provider_generation_digest,
        before.provider_generation_digest
    );
    assert!(hub.session_has_in_flight_prompt(&before.id));
}

#[test]
fn provider_reload_reconciles_interrupted_reset_without_migrating_native_identity() {
    let (hub, before) = provider_reload_fixture();
    let behavior = crate::provider::legacy_behavior("codex");
    hub.begin_provider_reload(
        &before,
        "intended",
        "intended-digest",
        &behavior,
        false,
        None,
    )
    .unwrap();
    let mut worker =
        super::runtime_reconciliation_tests::worker_snapshot(&before.id, "surviving-worker");
    worker.agent_session_id.clone_from(&before.agent_session_id);
    worker.launch = Some(crate::runtime_wire::StartSession {
        session_id: before.id.clone(),
        provider: before.provider.clone(),
        provider_version: "actual".to_owned(),
        provider_generation_digest: "actual-digest".to_owned(),
        provider_account_id: before.provider_account_id.clone(),
        provider_auth_generation: before.provider_auth_generation,
        provider_behavior: Some(behavior),
        cwd: before.cwd.clone(),
        agent_session_id: before.agent_session_id.clone(),
        system: false,
        context_window: None,
        auto_compact_token_limit: None,
        cache_protection: None,
        generation: "worker-generation".to_owned(),
        fallback_for: None,
        adopt_only: false,
        execution_binding: None,
        extensions: None,
        mcp_plan: None,
        native_fork: None,
    });
    let history = serde_json::to_value(hub.snapshot(&before.id).unwrap().0).unwrap();
    for mismatch in ["native", "home", "cwd", "placeholder"] {
        let mut invalid = worker.clone();
        match mismatch {
            "native" => invalid.agent_session_id = Some("other-thread".to_owned()),
            "home" => invalid.launch.as_mut().unwrap().provider_auth_generation = Some(999),
            "cwd" => invalid.launch.as_mut().unwrap().cwd = "/other".to_owned(),
            _ => invalid.worker_epoch = format!("broker-{}", before.id),
        }
        hub.reconcile_provider_release(&invalid);
        assert_eq!(
            hub.session_info(&before.id).unwrap().meta.provider_version,
            "intended"
        );
    }
    hub.reconcile_provider_release(&worker);
    let actual = hub.session_info(&before.id).unwrap().meta;
    assert_eq!(actual.provider_version, "actual");
    assert_eq!(actual.provider_generation_digest, "actual-digest");
    assert_eq!(actual.agent_session_id, before.agent_session_id);
    assert_eq!(
        actual.provider_auth_generation,
        before.provider_auth_generation
    );
    assert_eq!(
        serde_json::to_value(hub.snapshot(&before.id).unwrap().0).unwrap(),
        history
    );
    // The opposite crash ordering (new worker, old persisted binding) uses
    // the same actual-owner reconciliation without an automatic restart.
    worker.launch.as_mut().unwrap().provider_version = "replacement".to_owned();
    worker.launch.as_mut().unwrap().provider_generation_digest = "replacement-digest".to_owned();
    hub.reconcile_provider_release(&worker);
    assert_eq!(
        hub.session_info(&before.id).unwrap().meta.provider_version,
        "replacement"
    );
}

#[test]
fn dormant_repin_changes_only_the_binding_and_only_while_exited() {
    let (hub, running) = provider_reload_fixture();
    let behavior = crate::provider::legacy_behavior("codex");
    // A live session is not dormant.
    assert!(
        hub.repin_dormant_provider(&running, "new", "new-digest", &behavior, None)
            .unwrap_err()
            .contains("dormant")
    );
    hub.set_status(&running.id, Status::Exited, None);
    let before = hub.session_info(&running.id).unwrap().meta;
    let mut stale = before.clone();
    stale.provider_generation_digest = "other".to_owned();
    assert!(
        hub.repin_dormant_provider(&stale, "new", "new-digest", &behavior, None)
            .unwrap_err()
            .contains("changed")
    );
    hub.repin_dormant_provider(&before, "new", "new-digest", &behavior, None)
        .expect("re-pin");
    let after = hub.session_info(&before.id).unwrap().meta;
    assert_eq!(after.provider_version, "new");
    assert_eq!(after.provider_generation_digest, "new-digest");
    assert_eq!(after.provider_behavior, Some(behavior));
    assert_eq!(after.status, Status::Exited, "nothing is started");
    assert_eq!(after.agent_session_id, before.agent_session_id);
    assert_eq!(
        after.provider_auth_generation,
        before.provider_auth_generation
    );
    assert_eq!(after.cwd, before.cwd);
    assert_eq!(after.machine_id, before.machine_id);
}

#[test]
fn a_missing_generation_repin_moves_a_crashed_session_only_once_noted() {
    let (hub, running) = provider_reload_fixture();
    let behavior = crate::provider::legacy_behavior("codex");
    hub.set_status(&running.id, Status::Crashed, None);
    let before = hub.session_info(&running.id).unwrap().meta;
    // Without the Machine's refusal a crashed session keeps its pin.
    assert!(
        hub.repin_missing_provider(&before, "new", "new-digest", &behavior)
            .unwrap_err()
            .contains("dormant")
    );
    hub.note_provider_generation_missing(&before.id);
    assert_eq!(
        hub.provider_generation_missing_sessions(),
        vec![before.id.clone()]
    );
    hub.repin_missing_provider(&before, "new", "new-digest", &behavior)
        .expect("re-pin");
    let after = hub.session_info(&before.id).unwrap().meta;
    assert_eq!(after.provider_version, "new");
    assert_eq!(after.provider_generation_digest, "new-digest");
    assert_eq!(after.status, Status::Exited, "the next open launches it");
    assert_eq!(after.agent_session_id, before.agent_session_id);
    assert!(hub.provider_generation_missing_sessions().is_empty());
}

#[test]
fn provider_update_offer_follows_binding_and_one_shot_request() {
    let (hub, meta) = provider_reload_fixture();
    let offer = |when_idle| ProviderUpdateOffer {
        version: "new".to_owned(),
        digest: "new-digest".to_owned(),
        when_idle,
        automatic_after: Some(std::time::Duration::from_hours(1)),
        on_open: false,
    };
    let listed = |hub: &Hub| {
        hub.session_list()
            .into_iter()
            .find(|listed| listed.id == meta.id)
            .unwrap()
            .provider_update_available
    };
    assert_eq!(listed(&hub), None);
    hub.publish_provider_update_offers(HashMap::from([(meta.id.clone(), offer(false))]));
    let available = listed(&hub).expect("offer listed");
    assert_eq!(available.version, "new");
    assert!(!available.when_idle);
    let at = available.automatic_at_ms.expect("idle policy time");
    assert!(at > now_ms() + 3_500_000 && at <= now_ms() + 3_600_000);

    hub.set_provider_update_when_idle(&meta.id, true);
    assert!(hub.provider_update_when_idle(&meta.id));
    assert!(listed(&hub).unwrap().when_idle);

    // Once the session is bound to the offered release it is not offered.
    let behavior = crate::provider::legacy_behavior("codex");
    hub.set_status(&meta.id, Status::Exited, None);
    let exited = hub.session_info(&meta.id).unwrap().meta;
    hub.repin_dormant_provider(&exited, "new", "new-digest", &behavior, None)
        .expect("re-pin");
    assert_eq!(listed(&hub), None);

    hub.settle_provider_update_offer(&meta.id);
    assert!(!hub.provider_update_when_idle(&meta.id));
}

#[test]
fn provider_reload_rejects_stale_identity_and_unsaved_context() {
    let (hub, before) = provider_reload_fixture();
    let behavior = crate::provider::legacy_behavior("codex");
    let mut stale = before.clone();
    stale.provider_generation_digest = "other".to_owned();
    assert!(
        hub.begin_provider_reload(&stale, "new", "new", &behavior, false, None)
            .unwrap_err()
            .contains("changed")
    );
    // A prompted context whose native session was never saved has
    // nothing to resume on another release.
    let unsaved = hub_with_session("provider-reload-unsaved");
    unsaved.push("provider-reload-unsaved", Event::Update {
        update: serde_json::json!({"sessionUpdate": "user_message_chunk", "content": {"text": "lost"}}),
    });
    unsaved.set_status("provider-reload-unsaved", Status::Running, None);
    let unsaved_meta = unsaved
        .session_info("provider-reload-unsaved")
        .unwrap()
        .meta;
    assert!(!unsaved.provider_version_changeable(&unsaved_meta.id));
    assert!(
        unsaved
            .begin_provider_reload(&unsaved_meta, "new", "new", &behavior, false, None)
            .unwrap_err()
            .contains("saved native")
    );
    assert_eq!(unsaved.status(&unsaved_meta.id), Some(Status::Running));
}

#[test]
fn a_cleared_or_unprompted_context_changes_provider_version() {
    let behavior = crate::provider::legacy_behavior("codex");
    // Cleared: the old native thread is not resumed on either release.
    let (hub, before) = provider_reload_fixture();
    hub.prepare_context_reset(&before.id);
    hub.push(
        &before.id,
        Event::Update {
            update: serde_json::json!({"sessionUpdate": "context_cleared"}),
        },
    );
    assert!(hub.provider_version_changeable(&before.id));
    let cleared = hub.session_info(&before.id).unwrap().meta;
    hub.begin_provider_reload(&cleared, "new", "new-digest", &behavior, false, None)
        .expect("reload a cleared context");
    let reloaded = hub.session_info(&before.id).unwrap().meta;
    assert_eq!(reloaded.provider_version, "new");
    assert_eq!(reloaded.status, Status::Starting);

    // Never prompted and dormant: re-pinned without starting.
    let fresh = hub_with_session("provider-reload-fresh");
    fresh.set_status("provider-reload-fresh", Status::Exited, None);
    let exited = fresh.session_info("provider-reload-fresh").unwrap().meta;
    assert!(fresh.provider_version_changeable(&exited.id));
    fresh
        .repin_dormant_provider(&exited, "new", "new-digest", &behavior, None)
        .expect("re-pin an unprompted session");
    let after = fresh.session_info(&exited.id).unwrap().meta;
    assert_eq!(after.provider_version, "new");
    assert_eq!(after.status, Status::Exited);
}

#[tokio::test]
async fn context_reset_releases_stale_in_flight_guard_for_queued_send() {
    let hub = hub_with_session("reset-queue");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.set_status("reset-queue", Status::Running, None);

    hub.submit("reset-queue", "old turn".to_owned(), vec![], None);
    let old_turn = rx.recv().await.expect("old turn dispatch");
    assert_eq!(old_turn.text, "old turn");
    // This test resets an already handed-off runtime turn.
    drop(old_turn);
    hub.submit("reset-queue", "after reset".to_owned(), vec![], None);
    assert_eq!(queue_texts(&hub, "reset-queue"), vec!["after reset"]);

    hub.prepare_context_reset("reset-queue");
    hub.set_status("reset-queue", Status::Starting, None);
    hub.set_status("reset-queue", Status::Running, None);

    let dispatched = rx.recv().await.expect("queued dispatch after reset");
    assert_eq!(dispatched.text, "after reset");
    assert!(rx.try_recv().is_err());
    assert!(queue_texts(&hub, "reset-queue").is_empty());
}

#[tokio::test]
async fn context_reset_preserves_pending_dispatch_order_and_failed_prompt() {
    for failed in [false, true] {
        let hub = hub_with_session("reset-preflight");
        let (tx, mut rx) = mpsc::channel(4);
        hub.set_dispatch_tx(tx);
        hub.set_status("reset-preflight", Status::Running, None);
        hub.submit(
            "reset-preflight",
            "first".into(),
            vec![],
            Some("first".into()),
        );
        let first = rx.recv().await.unwrap();

        hub.prepare_context_reset("reset-preflight");
        hub.set_status("reset-preflight", Status::Starting, None);
        hub.set_status("reset-preflight", Status::Running, None);
        hub.submit(
            "reset-preflight",
            "second".into(),
            vec![],
            Some("second".into()),
        );
        hub.drain_now("reset-preflight");
        assert!(
            rx.try_recv().is_err(),
            "reset cannot supersede preflight ownership"
        );
        assert_eq!(queue_texts(&hub, "reset-preflight"), vec!["second"]);

        if failed {
            hub.requeue_prompt(&first.session_id, first.text, first.content, first.cmid);
            drop(first.ownership);
            hub.try_drain("reset-preflight");
            let retry = rx.recv().await.unwrap();
            assert_eq!(retry.text, "first");
            assert_eq!(retry.cmid.as_deref(), Some("first"));
            hub.try_drain("reset-preflight");
            assert!(rx.try_recv().is_err());
            drop(retry);
        } else {
            // Successful preflight hands off the first prompt before completion.
            drop(first);
        }
        hub.set_status("reset-preflight", Status::Busy, None);
        hub.set_status("reset-preflight", Status::Running, None);
        let second = rx.recv().await.unwrap();
        assert_eq!(second.text, "second");
        assert!(rx.try_recv().is_err());
    }
}

#[tokio::test]
async fn clean_turn_end_drains_next_message_without_classifier_hold() {
    let hub = hub_with_session("queue-after-turn");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.set_status("queue-after-turn", Status::Running, None);

    hub.submit("queue-after-turn", "first".to_owned(), vec![], None);
    assert_eq!(rx.recv().await.expect("first dispatch").text, "first");
    hub.submit("queue-after-turn", "second".to_owned(), vec![], None);
    assert_eq!(queue_texts(&hub, "queue-after-turn"), vec!["second"]);

    hub.set_status("queue-after-turn", Status::Busy, None);
    hub.set_status("queue-after-turn", Status::Running, None);

    assert_eq!(rx.recv().await.expect("turn-end dispatch").text, "second");
    assert!(queue_texts(&hub, "queue-after-turn").is_empty());
}

#[tokio::test]
async fn repeated_interrupt_admission_cannot_cancel_or_drain_again() {
    let hub = hub_with_session("force-replay");
    hub.set_status("force-replay", Status::Busy, None);
    assert_eq!(
        hub.force_submit_once(
            "force-replay",
            "correction".into(),
            vec![],
            Some("same-send".into()),
            true
        ),
        Some(true)
    );
    assert_eq!(
        hub.force_submit_once(
            "force-replay",
            "correction".into(),
            vec![],
            Some("same-send".into()),
            true
        ),
        None
    );
    assert_eq!(queue_texts(&hub, "force-replay"), vec!["correction"]);
}

#[tokio::test]
async fn turn_end_then_idle_status_dispatches_only_the_force_pushed_prompt() {
    let hub = hub_with_session("force-turn-end-first");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.set_status("force-turn-end-first", Status::Running, None);

    hub.submit(
        "force-turn-end-first",
        "active turn".to_owned(),
        vec![],
        None,
    );
    assert_eq!(
        rx.recv().await.expect("active dispatch").text,
        "active turn"
    );
    hub.set_status("force-turn-end-first", Status::Busy, None);
    hub.submit(
        "force-turn-end-first",
        "old queue head".to_owned(),
        vec![],
        None,
    );
    assert!(hub.force_submit(
        "force-turn-end-first",
        "forced prompt".to_owned(),
        vec![],
        None,
        true,
    ));
    assert_eq!(
        queue_texts(&hub, "force-turn-end-first"),
        vec!["forced prompt", "old queue head"]
    );

    hub.complete_turn("force-turn-end-first");
    hub.set_status("force-turn-end-first", Status::Running, None);
    assert_eq!(
        rx.recv().await.expect("forced dispatch").text,
        "forced prompt"
    );

    assert!(rx.try_recv().is_err(), "old queue head must remain parked");
    assert_eq!(
        queue_texts(&hub, "force-turn-end-first"),
        vec!["old queue head"]
    );
    assert!(hub.session_has_in_flight_prompt("force-turn-end-first"));

    hub.set_status("force-turn-end-first", Status::Busy, None);
    hub.complete_turn("force-turn-end-first");
    hub.set_status("force-turn-end-first", Status::Running, None);
    assert_eq!(
        rx.recv().await.expect("old head dispatch").text,
        "old queue head"
    );
    assert!(queue_texts(&hub, "force-turn-end-first").is_empty());
}

#[tokio::test]
async fn idle_status_then_turn_end_dispatches_only_the_force_pushed_prompt() {
    let hub = hub_with_session("force-idle-first");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.set_status("force-idle-first", Status::Running, None);

    hub.submit("force-idle-first", "active turn".to_owned(), vec![], None);
    assert_eq!(
        rx.recv().await.expect("active dispatch").text,
        "active turn"
    );
    hub.set_status("force-idle-first", Status::Busy, None);
    hub.submit(
        "force-idle-first",
        "old queue head".to_owned(),
        vec![],
        None,
    );
    assert!(hub.force_submit(
        "force-idle-first",
        "forced prompt".to_owned(),
        vec![],
        None,
        true,
    ));

    hub.set_status("force-idle-first", Status::Running, None);
    assert_eq!(
        rx.recv().await.expect("forced dispatch").text,
        "forced prompt"
    );
    hub.complete_turn("force-idle-first");

    assert!(rx.try_recv().is_err(), "old queue head must remain parked");
    assert_eq!(
        queue_texts(&hub, "force-idle-first"),
        vec!["old queue head"]
    );
    assert!(hub.session_has_in_flight_prompt("force-idle-first"));

    hub.set_status("force-idle-first", Status::Busy, None);
    hub.set_status("force-idle-first", Status::Running, None);
    assert_eq!(
        rx.recv().await.expect("old head dispatch").text,
        "old queue head"
    );
    hub.complete_turn("force-idle-first");
    assert!(
        rx.try_recv().is_err(),
        "duplicate turn end must stay a no-op"
    );
    assert!(queue_texts(&hub, "force-idle-first").is_empty());
}

#[tokio::test]
async fn authoritative_runtime_idle_releases_missed_turn_lifecycle_guard() {
    let hub = hub_with_session("runtime-reconnect");
    let (tx, mut rx) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.set_status("runtime-reconnect", Status::Running, None);

    hub.submit(
        "runtime-reconnect",
        "lost lifecycle turn".to_owned(),
        vec![],
        None,
    );
    let first = rx.recv().await.expect("first dispatch");
    assert_eq!(first.text, "lost lifecycle turn");
    // The dispatcher has handed this prompt to the runtime. Keeping its
    // request alive would instead model a still-pending async preflight.
    drop(first);
    hub.submit("runtime-reconnect", "queued turn".to_owned(), vec![], None);
    assert_eq!(queue_texts(&hub, "runtime-reconnect"), vec!["queued turn"]);

    let guard_epoch = hub
        .in_flight_prompt_epoch("runtime-reconnect")
        .expect("stale guard epoch");
    hub.reconcile_runtime_idle("runtime-reconnect", guard_epoch);

    let dispatched = rx
        .recv()
        .await
        .expect("queued dispatch after runtime reconciliation");
    assert_eq!(dispatched.text, "queued turn");
    assert!(queue_texts(&hub, "runtime-reconnect").is_empty());
}

#[test]
fn truncated_hot_tail_conservatively_preserves_native_thread() {
    let hub = hub_with_session("truncated-native-history");
    hub.set_agent_session_id("truncated-native-history", "thread-existing".to_owned());
    {
        let mut sessions = hub.inner.sessions.lock();
        let session = sessions
            .get_mut("truncated-native-history")
            .expect("session");
        session.reached_start = false;
    }

    assert_eq!(
        hub.agent_session_id_for_resume("truncated-native-history")
            .as_deref(),
        Some("thread-existing")
    );
}

#[test]
fn idle_failed_sessions_recover_without_switching_runtime_generation() {
    let hub = Hub::new();
    hub.create_session(SessionRegistration {
        id: "auth-refresh".to_owned(),
        provider: "grok".to_owned(),
        provider_version: "1.1.8".to_owned(),
        provider_generation_digest: "sha256:provider".to_owned(),
        provider_account_id: crate::provider_account::ProviderAccountId::default(),
        provider_auth_generation: Some(2),
        provider_behavior: None,
        machine_id: "hawk".to_owned(),
        workspace_id: None,
        workspace_name: None,
        workspace_source_path: None,
        execution_binding: None,
        cwd: "/tmp".to_owned(),
        title: "test".to_owned(),
        origin: SessionOrigin::Web,
        system: false,
        owner_user_id: None,
        owner_username: None,
    });
    hub.set_status(
        "auth-refresh",
        Status::Crashed,
        Some("login required".to_owned()),
    );
    let crashed_revision = hub.status_revision("auth-refresh").unwrap();
    hub.set_status(
        "auth-refresh",
        Status::Crashed,
        Some("new crash edge".to_owned()),
    );
    assert!(
        !hub.begin_provider_auth_recovery(
            "auth-refresh",
            crashed_revision,
            "login required",
            2,
            3,
        )
        .expect("stale lifecycle edge")
    );
    let crashed_revision = hub.status_revision("auth-refresh").unwrap();

    assert!(
        hub.begin_provider_auth_recovery("auth-refresh", crashed_revision, "new crash edge", 2, 3,)
            .expect("safe recovery")
    );
    assert_eq!(
        hub.session_info("auth-refresh")
            .unwrap()
            .meta
            .provider_auth_generation,
        Some(2),
        "the original runtime generation owns the native rollout"
    );
    assert_eq!(hub.status("auth-refresh"), Some(Status::Starting));

    hub.set_agent_session_id("auth-refresh", "native-thread".to_owned());
    hub.set_status(
        "auth-refresh",
        Status::Crashed,
        Some("second login failure".to_owned()),
    );
    hub.set_status("auth-refresh", Status::Running, None);
    let idle_revision = hub.status_revision("auth-refresh").unwrap();
    assert!(
        hub.begin_provider_auth_recovery(
            "auth-refresh",
            idle_revision,
            "second login failure",
            2,
            4,
        )
        .expect("idle native thread follows explicit reauthorization")
    );
    assert_eq!(
        hub.session_info("auth-refresh")
            .unwrap()
            .meta
            .provider_auth_generation,
        Some(2)
    );
    assert_eq!(
        hub.session_info("auth-refresh")
            .unwrap()
            .meta
            .agent_session_id
            .as_deref(),
        Some("native-thread")
    );

    hub.set_status("auth-refresh", Status::Exited, None);
    let exited_revision = hub.status_revision("auth-refresh").unwrap();
    assert!(
        !hub.begin_provider_auth_recovery(
            "auth-refresh",
            exited_revision,
            "second login failure",
            2,
            5,
        )
        .expect("an explicitly exited session remains stopped")
    );
    hub.set_status(
        "auth-refresh",
        Status::Crashed,
        Some("third login failure".to_owned()),
    );
    hub.set_status("auth-refresh", Status::Running, None);

    hub.push(
        "auth-refresh",
        Event::TurnEnd {
            usage: None,
            stop_reason: "end_turn".to_owned(),
        },
    );
    let completed_revision = hub.status_revision("auth-refresh").unwrap();
    assert!(
        !hub.begin_provider_auth_recovery(
            "auth-refresh",
            completed_revision,
            "third login failure",
            2,
            5,
        )
        .expect("a completed turn clears the unresolved failure")
    );
}

#[test]
fn newer_crash_without_detail_preserves_an_older_auth_failure() {
    let hub = hub_with_session("newer-detail-less-crash");
    hub.set_status(
        "newer-detail-less-crash",
        Status::Crashed,
        Some("login required".to_owned()),
    );
    hub.set_status("newer-detail-less-crash", Status::Crashed, None);

    assert_eq!(
        hub.unresolved_crash_detail("newer-detail-less-crash")
            .as_deref(),
        Some("login required")
    );
}

#[tokio::test]
async fn critical_persistence_waits_behind_a_full_event_queue() {
    let health = Arc::new(PersistenceHealth::default());
    let (sink, mut rx) = StoreSink::channel(1, Arc::clone(&health));
    assert!(sink.send(StoreWrite::AppendEvent(Envelope {
        session_id: "s".to_owned(),
        seq: 1,
        event: Event::TurnEnd {
            usage: None,
            stop_reason: "done".to_owned(),
        },
        cmid: None,
    })));
    assert!(sink.send(StoreWrite::UpdateTitle {
        session_id: "s".to_owned(),
        title: "durable".to_owned(),
    }));
    assert!(matches!(rx.recv().await, Some(StoreWrite::AppendEvent(_))));
    assert!(matches!(
        rx.recv().await,
        Some(StoreWrite::UpdateTitle { .. })
    ));
    assert_eq!(health.dropped(), 0);
    assert_eq!(health.pending(), 0);
}

#[tokio::test]
async fn session_broadcast_errors_are_persisted_outside_the_transcript() {
    let health = Arc::new(PersistenceHealth::default());
    let (sink, mut rx) = StoreSink::channel(4, health);
    let hub = Hub::with_store(Some(sink));
    let mut live = hub.subscribe();

    hub.broadcast_error(
        Some("session-1".to_owned()),
        "runtime rejected command".to_owned(),
    );

    let Some(StoreWrite::RecordSessionError {
        id,
        session_id,
        message,
        ..
    }) = rx.recv().await
    else {
        panic!("expected a durable session error");
    };
    assert!(id.starts_with("session-error:session-1:"));
    assert_eq!(session_id, "session-1");
    assert_eq!(message, "runtime rejected command");
    let frame = live.recv().await.expect("error frame");
    assert!(matches!(
        &**frame,
        Outbound::Error {
            session_id: Some(session_id),
            message,
        } if session_id == "session-1" && message == "runtime rejected command"
    ));
    assert!(hub.snapshot("session-1").is_none());

    hub.broadcast_error(Some("session-1".to_owned()), "你".repeat(2_000));
    let Some(StoreWrite::RecordSessionError { message, .. }) = rx.recv().await else {
        panic!("expected a bounded unicode error");
    };
    assert!(message.len() <= 4 * 1024);
    assert_eq!(message, "你".repeat(message.chars().count()));
}
