use super::*;

fn child(id: &str) -> SessionRegistration {
    let binding = crate::execution_environment::ManagedChildV1 {
        schema: 1,
        phase: "managed_child".into(),
        session_id: id.into(),
        parent_session_id: "parent".into(),
        machine_id: "hawk".into(),
        workspace_id: "project".into(),
        cwd: "/owned/managed/child/workspace".into(),
        profile: cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
    };
    SessionRegistration {
        id: id.to_owned(),
        provider: "codex".to_owned(),
        provider_version: "3.4.0".to_owned(),
        provider_generation_digest: "generation".to_owned(),
        provider_account_id: crate::provider_account::ProviderAccountId::default(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: "hawk".to_owned(),
        workspace_id: None,
        workspace_name: None,
        workspace_source_path: None,
        execution_binding: Some(crate::execution_environment::ExecutionBinding::from_record(
            serde_json::to_value(binding).unwrap(),
        )),
        cwd: "/owned/managed/child/workspace".to_owned(),
        title: "child".to_owned(),
        origin: SessionOrigin::Api,
        system: false,
        owner_user_id: None,
        owner_username: None,
    }
}

fn update(kind: &str, text: &str) -> Event {
    Event::Update {
        update: serde_json::json!({"sessionUpdate": kind, "content": {"type":"text","text": text}}),
    }
}

#[test]
fn a_bound_session_settles_on_its_surviving_workers_release_before_acceptance() {
    let hub = Hub::new();
    let registration = child("bound");
    let binding = registration.execution_binding.clone();
    hub.create_session(registration);
    hub.set_agent_session_id("bound", "native-thread".to_owned());
    let meta = hub.session_info("bound").unwrap().meta;
    let mut worker = super::runtime_reconciliation_tests::worker_snapshot("bound", "epoch-1");
    worker.agent_session_id.clone_from(&meta.agent_session_id);
    worker.launch = Some(crate::runtime_wire::StartSession {
        session_id: meta.id.clone(),
        provider: meta.provider.clone(),
        provider_version: "3.3.0".to_owned(),
        provider_generation_digest: "older-generation".to_owned(),
        provider_account_id: meta.provider_account_id.clone(),
        provider_auth_generation: meta.provider_auth_generation,
        provider_behavior: None,
        cwd: meta.cwd.clone(),
        agent_session_id: meta.agent_session_id.clone(),
        system: false,
        context_window: None,
        auto_compact_token_limit: None,
        cache_protection: None,
        generation: "worker-generation".to_owned(),
        fallback_for: None,
        adopt_only: false,
        execution_binding: None,
        extensions: None,
        native_fork: None,
    });
    // An owner on another placement is neither adopted nor reconciled.
    hub.reconcile_provider_release(&worker);
    assert_eq!(
        hub.session_info("bound").unwrap().meta.provider_version,
        "3.4.0"
    );
    assert!(!hub.accept_runtime_snapshot(&worker));

    // The record ran ahead of the live worker; its owner defines the
    // release, and only then does the acceptance fence admit it.
    worker.launch.as_mut().unwrap().execution_binding = binding;
    assert!(!hub.accept_runtime_snapshot(&worker));
    hub.reconcile_provider_release(&worker);
    let settled = hub.session_info("bound").unwrap().meta;
    assert_eq!(settled.provider_version, "3.3.0");
    assert_eq!(settled.provider_generation_digest, "older-generation");
    assert!(hub.accept_runtime_snapshot(&worker));
}

#[test]
fn a_crashed_managed_child_is_never_reloaded_onto_another_provider_release() {
    // 2026-10-29: Provider updates restarted failed review children whose
    // Hawk execution lifetimes were lost; each replacement only crashed.
    let hub = Hub::new();
    hub.create_session(child("crashed-child"));
    hub.set_status("crashed-child", Status::Crashed, Some("lost".to_owned()));
    let before = hub.session_info("crashed-child").unwrap().meta;
    let behavior = crate::provider::legacy_behavior("codex");

    let error = hub
        .begin_provider_reload(&before, "3.5.0", "newer", &behavior, false, None)
        .expect_err("a managed child keeps its release");

    assert!(error.contains("managed child"), "{error}");
    let after = hub.session_info("crashed-child").unwrap().meta;
    assert_eq!(after.status, Status::Crashed);
    assert_eq!(after.provider_version, "3.4.0");
    assert_eq!(after.provider_generation_digest, "generation");
}

#[test]
fn an_exited_worker_never_undoes_a_dormant_re_pin() {
    let hub = Hub::new();
    let registration = child("dormant");
    let binding = registration.execution_binding.clone();
    hub.create_session(registration);
    hub.set_agent_session_id("dormant", "native-thread".to_owned());
    let meta = hub.session_info("dormant").unwrap().meta;
    let mut worker = super::runtime_reconciliation_tests::worker_snapshot("dormant", "epoch-1");
    worker.state = WorkerState::Exited;
    worker.agent_session_id.clone_from(&meta.agent_session_id);
    worker.launch = Some(crate::runtime_wire::StartSession {
        session_id: meta.id.clone(),
        provider: meta.provider.clone(),
        provider_version: "3.3.0".to_owned(),
        provider_generation_digest: "older-generation".to_owned(),
        provider_account_id: meta.provider_account_id.clone(),
        provider_auth_generation: meta.provider_auth_generation,
        provider_behavior: None,
        cwd: meta.cwd.clone(),
        agent_session_id: meta.agent_session_id.clone(),
        system: false,
        context_window: None,
        auto_compact_token_limit: None,
        cache_protection: None,
        generation: "worker-generation".to_owned(),
        fallback_for: None,
        adopt_only: false,
        execution_binding: binding,
        extensions: None,
        native_fork: None,
    });

    hub.reconcile_provider_release(&worker);
    assert_eq!(
        hub.session_info("dormant").unwrap().meta.provider_version,
        "3.4.0"
    );
    worker.state = WorkerState::Crashed;
    hub.reconcile_provider_release(&worker);
    assert_eq!(
        hub.session_info("dormant").unwrap().meta.provider_version,
        "3.4.0"
    );
}

#[test]
fn client_entry_points_cannot_prompt_a_managed_child() {
    let hub = Hub::new();
    hub.create_session(child("child"));
    hub.submit(
        "child",
        "injected".into(),
        Vec::new(),
        Some("client-1".into()),
    );
    assert!(!hub.force_submit("child", "forced".into(), Vec::new(), None, true));
    hub.add_draft("child", "draft".into(), Vec::new(), None);
    hub.activate_all_drafts("child");
    let turn = hub.managed_turn("child", 0).unwrap();
    assert!(!turn.prompted);
    hub.submit_managed_prompt("child", "call-1", "review".into());
    assert!(hub.managed_turn("child", 0).unwrap().prompted);
}

#[test]
fn managed_turn_returns_the_final_message_after_its_last_tool_update() {
    let hub = Hub::new();
    hub.create_session(child("child"));
    hub.push("child", update("agent_message_chunk", "previous call"));
    hub.push(
        "child",
        Event::TurnEnd {
            usage: None,
            stop_reason: "end_turn".into(),
        },
    );
    let cursor = hub.last_event_seq("child").unwrap();
    hub.push("child", update("user_message_chunk", "review"));
    hub.push("child", update("agent_message_chunk", "I will inspect"));
    hub.push(
        "child",
        Event::Update {
            update: serde_json::json!({"sessionUpdate":"tool_call","toolCallId":"t"}),
        },
    );
    hub.push("child", update("agent_message_chunk", "{\"verdict\":"));
    hub.push("child", update("agent_message_chunk", "\"approve\"}"));
    let running = hub.managed_turn("child", cursor).unwrap();
    assert!(running.prompted && running.stop_reason.is_none());
    hub.push(
        "child",
        Event::TurnEnd {
            usage: None,
            stop_reason: "end_turn".into(),
        },
    );
    let ended = hub.managed_turn("child", cursor).unwrap();
    assert_eq!(ended.stop_reason.as_deref(), Some("end_turn"));
    assert_eq!(ended.final_text, "{\"verdict\":\"approve\"}");
    assert!(hub.managed_turn("missing", 0).is_none());
}

#[test]
fn managed_turn_does_not_prepend_commentary_to_a_distinct_final_message() {
    let hub = Hub::new();
    hub.create_session(child("child"));
    for (message_id, text) in [
        (Some("progress"), "The probes confirm the defect.\n"),
        (Some("answer"), "{\"verdict\":"),
        (Some("answer"), "\"needs-attention\""),
        // Some adapters omit an ID on subsequent chunks of the same message.
        (None, "}"),
    ] {
        hub.push(
            "child",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "messageId": message_id,
                    "content": {"type": "text", "text": text},
                }),
            },
        );
        hub.push("child", update("agent_thought_chunk", "internal"));
    }
    hub.push(
        "child",
        Event::TurnEnd {
            usage: None,
            stop_reason: "end_turn".into(),
        },
    );
    // A later turn must never replace this call's result.
    hub.push("child", update("agent_message_chunk", "next turn"));
    let turn = hub.managed_turn("child", 0).unwrap();
    assert_eq!(turn.final_text, "{\"verdict\":\"needs-attention\"}");
    assert!(serde_json::from_str::<serde_json::Value>(&turn.final_text).is_ok());
}

#[test]
fn managed_turn_keeps_its_final_message_across_a_trailing_notice() {
    let hub = Hub::new();
    hub.create_session(child("child"));
    hub.push("child", update("user_message_chunk", "review"));
    hub.push(
        "child",
        update("agent_message_chunk", "{\"verdict\":\"approve\"}"),
    );
    hub.push(
        "child",
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "notice",
                "severity": "warning",
                "title": "Model fallback",
            }),
        },
    );
    hub.push(
        "child",
        Event::TurnEnd {
            usage: None,
            stop_reason: "end_turn".into(),
        },
    );
    let turn = hub.managed_turn("child", 0).unwrap();
    assert_eq!(turn.final_text, "{\"verdict\":\"approve\"}");
}

#[test]
fn managed_turn_usage_is_scoped_to_the_first_end_after_its_cursor() {
    let hub = Hub::new();
    hub.create_session(child("child"));
    let usage = crate::agent_model::TurnUsage::from_acp(
        &agent_client_protocol::schema::v1::Usage::new(100, 90, 10),
    )
    .unwrap();
    let end = |usage| Event::TurnEnd {
        stop_reason: "end_turn".into(),
        usage,
    };
    hub.push("child", end(Some(usage)));
    let cursor = hub.last_event_seq("child").unwrap();
    // A missing usage report stays unknown; never borrow an adjacent turn.
    hub.push("child", end(None));
    let next_cursor = hub.last_event_seq("child").unwrap();
    hub.push("child", end(Some(usage)));
    assert_eq!(hub.managed_turn("child", cursor).unwrap().usage, None);
    assert_eq!(
        hub.managed_turn("child", next_cursor).unwrap().usage,
        Some(usage)
    );
}
