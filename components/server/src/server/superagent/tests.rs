use super::*;

#[tokio::test]
async fn superagent_long_turn_recovers_human_anchor_from_durable_history() {
    use crate::core::{Envelope, Event};
    let hub = crate::core::Hub::new();
    let owner = "a".repeat(32);
    let meta = add_session(&hub, "worker", &owner, "hawk");
    let dir = tempfile::tempdir().unwrap();
    let store = crate::store::Store::connect("sqlite::memory:", dir.path().into())
        .await
        .unwrap();
    store.migrate().await.unwrap();
    store
        .insert_user(&crate::store::ProductUser {
            id: owner,
            username: "reader".into(),
            password_algo: "argon2id".into(),
            password_hash: "unused-fixture".into(),
            created_at_ms: 1,
            updated_at_ms: 1,
            disabled_at_ms: None,
        })
        .await
        .unwrap();
    store.insert_session(&meta).await.unwrap();
    let mut rows = Vec::new();
    let progress = "progress".repeat(1024);
    for n in 0..450 {
        let event = if n == 449 {
            Event::TurnEnd {
                stop_reason: "end_turn".into(),
                usage: None,
            }
        } else {
            Event::Update {
                update: json!({"sessionUpdate":if n==0 {"user_message_chunk"} else {"agent_message_chunk"},"content":{"type":"text","text":if n==0 {"task"} else {&progress}}}),
            }
        };
        hub.push("worker", event.clone());
        let (tail, _) = hub.snapshot("worker").unwrap();
        rows.push(Envelope {
            session_id: "worker".into(),
            seq: tail.last().unwrap().seq,
            event,
            cmid: None,
        });
    }
    store
        .upsert_event_batch(&rows, &std::collections::HashMap::new())
        .await
        .unwrap();
    assert!(evidence(&hub, "worker").is_err());
    let recovered = scope::history_evidence(&hub, Some(&store), "worker")
        .await
        .unwrap();
    assert_eq!(recovered.seq, rows.last().unwrap().seq);
    assert!(recovered.text.len() < 80 * (progress.len() + 100));
}

fn add_session(hub: &crate::core::Hub, id: &str, owner: &str, machine: &str) -> SessionMeta {
    hub.create_session(crate::core::SessionRegistration {
        id: id.into(),
        provider: "codex".into(),
        provider_version: String::new(),
        provider_generation_digest: String::new(),
        provider_account_id: crate::provider_account::ProviderAccountId::default(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: machine.into(),
        workspace_id: None,
        workspace_name: None,
        workspace_source_path: None,
        execution_binding: None,
        cwd: "/tmp".into(),
        title: id.into(),
        origin: crate::core::SessionOrigin::Web,
        system: false,
        owner_user_id: Some(owner.into()),
        owner_username: None,
    });
    hub.set_status(id, crate::core::Status::Running, None);
    hub.set_agent_session_id(id, format!("native-{id}"));
    hub.session_info(id).unwrap().meta
}

#[test]
fn superagent_scope_denies_off_paused_foreign_and_cross_machine_targets() {
    let hub = crate::core::Hub::new();
    let parent = add_session(&hub, "coordinator", "owner", "hawk");
    add_session(&hub, "worker", "owner", "hawk");
    add_session(&hub, "foreign", "other", "hawk");
    add_session(&hub, "remote", "owner", "falcon");
    let mut config = Settings::default();
    config.projects.insert(
        "cowboy".into(),
        crate::superagent::Project {
            profile: "project".into(),
            coordinator: parent.id.clone(),
            mode: Mode::Observe,
            preset: crate::superagent::Preset::AstraMax,
            max_followups: 2,
            learning: false,
        },
    );
    for id in ["worker", "foreign", "remote"] {
        config.sessions.insert(
            id.into(),
            crate::superagent::Participation {
                project: "cowboy".into(),
                mode: None,
                mandate: String::new(),
            },
        );
    }
    assert!(target(&hub, "owner", &parent, &config, "worker").is_ok());
    for id in ["foreign", "remote", "missing"] {
        assert!(target(&hub, "owner", &parent, &config, id).is_err());
    }
    config.sessions.get_mut("worker").unwrap().mode = Some(Mode::Off);
    assert!(target(&hub, "owner", &parent, &config, "worker").is_err());
    config.sessions.get_mut("worker").unwrap().mode = None;
    hub.set_paused("worker", true);
    assert!(target(&hub, "owner", &parent, &config, "worker").is_err());
    hub.set_paused("worker", false);
    hub.set_paused("coordinator", true);
    assert!(target(&hub, "owner", &parent, &config, "worker").is_err());
}

#[test]
fn superagent_remote_editor_cannot_steer_another_machine_through_preferences() {
    let hub = crate::core::Hub::new();
    let actor = add_session(&hub, "editor", "owner", "falcon");
    add_session(&hub, "local-coordinator", "owner", "falcon");
    add_session(&hub, "remote-coordinator", "owner", "hawk");
    let mut before = Settings::default();
    for (id, coordinator) in [
        ("local", "local-coordinator"),
        ("remote", "remote-coordinator"),
    ] {
        before.projects.insert(
            id.into(),
            crate::superagent::Project {
                profile: "global".into(),
                coordinator: coordinator.into(),
                mode: Mode::Off,
                preset: crate::superagent::Preset::AstraMax,
                max_followups: 2,
                learning: false,
            },
        );
    }
    let colocated = std::collections::BTreeSet::from(["hawk".into()]);
    let mut after = before.clone();
    after.projects.get_mut("local").unwrap().learning = true;
    assert!(scope::editable(&hub, &colocated, &actor, &before, &after).is_ok());
    after.projects.get_mut("remote").unwrap().mode = Mode::Coordinate;
    assert_eq!(
        scope::editable(&hub, &colocated, &actor, &before, &after),
        Err("machine_scope_denied")
    );
    after = before.clone();
    after.sources.push(crate::superagent::patterns::Source {
        revision: "new".into(),
        profile: "profile.md".into(),
        files: Default::default(),
    });
    assert!(scope::editable(&hub, &colocated, &actor, &before, &after).is_err());
}

#[test]
fn superagent_causal_budget_only_resets_on_a_human_and_cancel_never_resumes() {
    use crate::core::Event;
    let hub = crate::core::Hub::new();
    add_session(&hub, "worker", "owner", "hawk");
    assert!(evidence(&hub, "worker").is_err());
    let prompt = |actor| Event::Update {
        update: json!({"sessionUpdate":"user_message_chunk",
        "content":{"type":"text","text":"task"},"promptOrigin":{"actor":actor,"source":"fixture"}}),
    };
    let end = |reason: &str| Event::TurnEnd {
        stop_reason: reason.into(),
        usage: None,
    };
    hub.push("worker", prompt("human"));
    hub.push("worker", end("end_turn"));
    let first = evidence(&hub, "worker").unwrap();
    hub.push("worker", prompt("cowboy"));
    hub.push("worker", end("end_turn"));
    let next = evidence(&hub, "worker").unwrap();
    assert_eq!(first.chain, next.chain);
    assert_ne!(first.marker, next.marker);
    // Historical retained workers may have labelled a known agent cmid human.
    hub.push_tagged(
        "worker",
        prompt("human"),
        Some(format!("agent-{}", "a".repeat(64))),
    );
    hub.push("worker", end("end_turn"));
    assert_eq!(first.chain, evidence(&hub, "worker").unwrap().chain);
    hub.push("worker", prompt("human"));
    hub.push("worker", end("end_turn"));
    assert_ne!(first.chain, evidence(&hub, "worker").unwrap().chain);
    hub.push("worker", end("cancelled"));
    assert!(evidence(&hub, "worker").is_err());
}

#[test]
fn superagent_native_and_wire_completion_spellings_are_eligible() {
    use crate::core::Event;
    for reason in [
        "EndTurn",
        "end_turn",
        "Cancelled",
        "cancelled",
        "MaxTokens",
        "unknown",
    ] {
        let hub = crate::core::Hub::new();
        add_session(&hub, "worker", "owner", "hawk");
        hub.push(
            "worker",
            Event::Update {
                update: json!({"sessionUpdate":"user_message_chunk",
                "content":{"type":"text","text":"task"},
                "promptOrigin":{"actor":"human","source":"composer"}}),
            },
        );
        hub.push(
            "worker",
            Event::TurnEnd {
                stop_reason: reason.into(),
                usage: None,
            },
        );
        let result = evidence(&hub, "worker");
        if matches!(reason, "EndTurn" | "end_turn") {
            assert!(result.is_ok(), "{reason}");
        } else {
            assert_eq!(result.err(), Some("turn_not_completed"), "{reason}");
        }
    }
}

#[test]
fn superagent_context_pages_preserve_unicode_and_bound_reads() {
    assert_eq!(
        page("中文🚀abc", &json!({"offset":1,"chars":2})).unwrap()["text"],
        "文🚀"
    );
    assert!(page("hello", &json!({"chars":4001})).is_err());
    assert!(page("hello", &json!({"offset":-1})).is_err());
}

#[tokio::test]
async fn superagent_budget_is_causal_concurrent_and_blocks_unknown_outcomes() {
    let dir = tempfile::tempdir().unwrap();
    let store = crate::store::Store::connect("sqlite::memory:", dir.path().to_owned())
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let mut observation = Evidence {
        marker: "m1".into(),
        chain: "human-task".into(),
        seq: 10,
        text: String::new(),
    };
    let (a, b) = tokio::join!(
        actions::reserve_chain(&store, "owner", "worker", &observation, "follow", 2),
        actions::reserve_chain(&store, "owner", "worker", &observation, "follow", 2)
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_ne!(a.is_some(), b.is_some());
    let first = a.or(b).unwrap();
    observation.seq = 20;
    observation.marker = "m2".into();
    assert!(
        actions::reserve_chain(&store, "owner", "worker", &observation, "follow", 2)
            .await
            .unwrap()
            .is_none()
    );
    store
        .finish_agent_operation(
            "owner",
            "worker",
            &first,
            &json!({"turn_seq":10,"status":"submitted"}),
        )
        .await
        .unwrap();
    let second = actions::reserve_chain(&store, "owner", "worker", &observation, "follow", 2)
        .await
        .unwrap()
        .unwrap();
    store
        .finish_agent_operation(
            "owner",
            "worker",
            &second,
            &json!({"turn_seq":20,"status":"submitted"}),
        )
        .await
        .unwrap();
    observation.seq = 30;
    observation.marker = "m3".into();
    assert!(
        actions::reserve_chain(&store, "owner", "worker", &observation, "follow", 2)
            .await
            .unwrap()
            .is_none()
    );
    observation.chain = "new-human-task".into();
    assert!(
        actions::reserve_chain(&store, "owner", "worker", &observation, "follow", 2)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn superagent_config_and_unknown_reservation_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("state.db").display()
    );
    let store = crate::store::Store::connect(&url, dir.path().to_owned())
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let change = crate::store::preferences::PreferenceChange {
        scope: "superagent".into(),
        expected_revision: 0,
        changes: json!({"configuration":Settings::default()}),
    };
    assert!(matches!(
        store
            .update_preferences("owner", "configure", &change)
            .await
            .unwrap(),
        crate::store::preferences::PreferenceResult::Applied(_)
    ));
    use crate::store::agent_api_operations::Admission;
    assert_eq!(
        store
            .admit_agent_operation("owner", "worker", "sa-follow-cause-0", "evidence")
            .await
            .unwrap(),
        Admission::New
    );
    drop(store);
    let store = crate::store::Store::connect(&url, dir.path().to_owned())
        .await
        .unwrap();
    assert_eq!(
        store
            .preferences("owner", "superagent")
            .await
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    assert_eq!(
        store
            .admit_agent_operation("owner", "worker", "sa-follow-cause-0", "evidence")
            .await
            .unwrap(),
        Admission::Existing(None)
    );
    assert_eq!(
        store
            .admit_agent_operation("owner", "worker", "sa-follow-cause-0", "other")
            .await
            .unwrap(),
        Admission::Conflict
    );
    assert!(
        store
            .superagent_activity("other-owner", &(i64::MAX, String::new(), String::new()))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .superagent_activity("owner", &(i64::MAX, String::new(), String::new()))
            .await
            .unwrap()
            .len(),
        1
    );
}
