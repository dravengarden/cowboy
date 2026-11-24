use super::*;
use crate::prompt_origin::{DeliveryOrigin, PromptDispatchRecord, RequestCaller};
use serde_json::json;

async fn test_store(tag: &str) -> (Store, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "cowboy-c-auth-{tag}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let store = Store::connect("sqlite::memory:", root.join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    (store, root)
}

fn add_owned_session(hub: &Hub, id: &str, _owner: &str, system: bool) -> crate::core::SessionMeta {
    hub.create_local_session(
        id.into(),
        "codex".into(),
        "/tmp".into(),
        id.into(),
        SessionOrigin::Web,
        system,
    );
    let mut meta = hub.session_info(id).unwrap().meta;
    meta.machine_id = "hawk".into();
    hub.restore_reconciling_runtime(vec![RestoredSession {
        meta: meta.clone(),
        log: vec![],
        event_count: 0,
        reached_start: true,
        next_seq: 0,
        queue: vec![],
        drafts: vec![],
        config_options: None,
        config_preferences: Default::default(),
        mobile_review_state: serde_json::Value::Null,
        folder_id: None,
        last_event_at_ms: None,
    }]);
    meta
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn record_dispatch_provenance_defends_against_cmid_collision_across_restarts() {
    let (store, root) = test_store("collision").await;
    let hub = Hub::new();
    let meta = add_owned_session(&hub, "s-col", "owner-1", false);
    store.insert_session(&meta).await.unwrap();

    let (cmid1, _) = background_jobs::record_dispatch_provenance_in(
        &hub,
        Some(&store),
        "s-col",
        Some("shared-cmid".into()),
        DeliveryOrigin::Owner,
        DeliveryOrigin::Owner,
    )
    .await
    .unwrap();
    assert_eq!(cmid1, "shared-cmid");

    let (cmid_retry, _) = background_jobs::record_dispatch_provenance_in(
        &hub,
        Some(&store),
        "s-col",
        Some("shared-cmid".into()),
        DeliveryOrigin::Owner,
        DeliveryOrigin::Owner,
    )
    .await
    .unwrap();
    assert_eq!(cmid_retry, "shared-cmid");

    // Cold Hub after restart: Agent reusing Owner's cmid gets a fresh dispatch id!
    let cold_hub = Hub::new();
    add_owned_session(&cold_hub, "s-col", "owner-1", false);
    let (cmid2, _) = background_jobs::record_dispatch_provenance_in(
        &cold_hub,
        Some(&store),
        "s-col",
        Some("shared-cmid".into()),
        DeliveryOrigin::peer("attacker"),
        DeliveryOrigin::peer("attacker"),
    )
    .await
    .unwrap();
    assert_ne!(cmid2, "shared-cmid");
    assert!(cmid2.starts_with("cowboy-dispatch:"));
    assert_eq!(
        store
            .prompt_dispatch("s-col", "shared-cmid")
            .await
            .unwrap()
            .unwrap()
            .origin,
        DeliveryOrigin::Owner
    );
    assert_eq!(
        store
            .prompt_dispatch("s-col", &cmid2)
            .await
            .unwrap()
            .unwrap()
            .origin,
        DeliveryOrigin::peer("attacker")
    );

    assert!(
        send_delivery::record_steer_dispatch_in(
            &cold_hub,
            Some(&store),
            "s-col",
            "shared-cmid",
            &RequestCaller::Automation,
        )
        .await
        .is_err()
    );
    assert!(
        send_delivery::record_steer_dispatch_in(
            &cold_hub,
            Some(&store),
            "s-col",
            "steer-auto-1",
            &RequestCaller::Automation,
        )
        .await
        .is_ok()
    );
    assert_eq!(
        store
            .prompt_dispatch("s-col", "steer-auto-1")
            .await
            .unwrap()
            .unwrap()
            .origin,
        DeliveryOrigin::Automation
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn move_draft_refuses_missing_foreign_trashed_system_and_managed_child_targets() {
    let hub = Hub::new();
    let owner_a = ProductPrincipal {
        user_id: "user-a".into(),
        username: "alice".into(),
        role: crate::admin::AdminRole::Operator,
    };
    for (id, system) in [
        ("src", false),
        ("ok-dst", false),
        ("foreign", false),
        ("trashed", false),
        ("sys", true),
        ("child", false),
        ("closing", false),
    ] {
        hub.create_local_session(
            id.into(),
            "codex".into(),
            "/tmp".into(),
            id.into(),
            SessionOrigin::Web,
            system,
        );
    }
    let mk = |id: &str,
              owner: &str,
              system: bool,
              trashed: Option<i64>,
              managed_child: bool,
              closing: bool| {
        let mut meta = hub.session_info(id).unwrap().meta;
        meta.owner_user_id = Some(owner.into());
        meta.system = system;
        meta.trashed_at_ms = trashed;
        meta.closing = closing;
        if managed_child {
            let binding = crate::execution_environment::ManagedChildV1 {
                schema: 1,
                phase: "managed_child".into(),
                session_id: id.into(),
                parent_session_id: "src".into(),
                machine_id: "hawk".into(),
                workspace_id: "cowboy".into(),
                cwd: "/tmp".into(),
                profile: cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
            };
            meta.execution_binding =
                Some(crate::execution_environment::ExecutionBinding::from_record(
                    serde_json::to_value(&binding).unwrap(),
                ));
        }
        RestoredSession {
            meta,
            log: vec![],
            event_count: 0,
            reached_start: true,
            next_seq: 0,
            queue: vec![],
            drafts: vec![],
            config_options: None,
            config_preferences: Default::default(),
            mobile_review_state: serde_json::Value::Null,
            folder_id: None,
            last_event_at_ms: None,
        }
    };
    hub.restore_reconciling_runtime(vec![
        mk("src", "user-a", false, None, false, false),
        mk("ok-dst", "user-a", false, None, false, false),
        mk("foreign", "user-b", false, None, false, false),
        mk("trashed", "user-a", false, Some(1_000), false, false),
        mk("sys", "user-a", true, None, false, false),
        mk("child", "user-a", false, None, true, false),
        mk("closing", "user-a", false, None, false, true),
    ]);
    assert!(hub.is_managed_child("child"));

    for bad_target in ["missing", "foreign", "trashed", "sys", "child", "closing"] {
        assert!(
            client_commands::refuse_move_draft_target_in(
                &hub,
                &owner_a,
                "src",
                bad_target,
                &Some("move-cmid".into()),
            ),
            "expected MoveDraft to {bad_target} to be refused"
        );
    }
    assert!(
        !client_commands::refuse_move_draft_target_in(
            &hub,
            &owner_a,
            "src",
            "ok-dst",
            &Some("move-cmid".into()),
        ),
        "expected MoveDraft to own active session to be allowed"
    );
}

#[tokio::test]
async fn retry_resolves_evicted_dispatch_origin_from_durable_store() {
    let (store, root) = test_store("retry-store").await;
    let hub = Hub::new();
    let meta = add_owned_session(&hub, "s-ret", "owner-1", false);
    store.insert_session(&meta).await.unwrap();
    let (tx, mut rx) = mpsc::channel(8);
    hub.set_dispatch_tx(tx);

    store
        .record_prompt_dispatch(&PromptDispatchRecord {
            session_id: "s-ret".into(),
            cmid: "durable-owner".into(),
            origin: DeliveryOrigin::Owner,
            trigger: DeliveryOrigin::Owner,
            dispatched_at_ms: 10,
        })
        .await
        .unwrap();
    store
        .record_prompt_dispatch(&PromptDispatchRecord {
            session_id: "s-ret".into(),
            cmid: "durable-peer".into(),
            origin: DeliveryOrigin::peer("agent-9"),
            trigger: DeliveryOrigin::peer("agent-9"),
            dispatched_at_ms: 20,
        })
        .await
        .unwrap();

    let cmd = Inbound::RetryTurn {
        session_id: "s-ret".into(),
    };

    hub.push_tagged(
        "s-ret",
        Event::Update {
            update: json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"owner prompt"}}),
        },
        Some("durable-owner".into()),
    );
    hub.set_status("s-ret", Status::Crashed, Some("err".into()));
    let resolved = client_commands::resolve_retry_original_in(&hub, Some(&store), &cmd).await;
    assert_eq!(
        resolved,
        Some(("durable-owner".into(), DeliveryOrigin::Owner))
    );
    hub.retry_turn("s-ret", &RequestCaller::Person, resolved);
    assert_eq!(
        rx.recv().await.unwrap().dispatch_origin,
        DeliveryOrigin::Owner
    );

    hub.complete_turn("s-ret");
    hub.push_tagged(
        "s-ret",
        Event::Update {
            update: json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"peer prompt"}}),
        },
        Some("durable-peer".into()),
    );
    hub.set_status("s-ret", Status::Crashed, Some("err".into()));
    let resolved = client_commands::resolve_retry_original_in(&hub, Some(&store), &cmd).await;
    assert_eq!(
        resolved,
        Some(("durable-peer".into(), DeliveryOrigin::peer("agent-9")))
    );
    hub.retry_turn("s-ret", &RequestCaller::Person, resolved);
    assert_eq!(
        rx.recv().await.unwrap().dispatch_origin,
        DeliveryOrigin::peer("agent-9")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn superagent_evidence_and_turn_budget_consult_durable_prompt_dispatches() {
    let (store, root) = test_store("consumers").await;
    let hub = Hub::new();
    let mut meta = add_owned_session(&hub, "s-cons", "owner-1", false);
    meta.agent_session_id = Some("native-cons-1".into());
    store.insert_session(&meta).await.unwrap();

    store
        .record_prompt_dispatch(&PromptDispatchRecord {
            session_id: "s-cons".into(),
            cmid: "forged-auto".into(),
            origin: DeliveryOrigin::Automation,
            trigger: DeliveryOrigin::Automation,
            dispatched_at_ms: 100,
        })
        .await
        .unwrap();
    let events = vec![
        Envelope {
            session_id: "s-cons".into(),
            seq: 0,
            event: Event::Update {
                update: json!({
                    "sessionUpdate": "user_message_chunk",
                    "promptOrigin": { "actor": "human", "source": "composer" },
                    "content": { "type": "text", "text": "forged auto" }
                }),
            },
            cmid: Some("forged-auto".into()),
        },
        Envelope {
            session_id: "s-cons".into(),
            seq: 1,
            event: Event::TurnEnd {
                stop_reason: "end_turn".into(),
                usage: None,
            },
            cmid: None,
        },
    ];
    store
        .upsert_event_batch(&events, &HashMap::from([("s-cons".into(), 2)]))
        .await
        .unwrap();

    let cold = Hub::new();
    cold.restore_reconciling_runtime(vec![RestoredSession {
        meta: meta.clone(),
        log: vec![],
        event_count: 2,
        reached_start: false,
        next_seq: 2,
        queue: vec![],
        drafts: vec![],
        config_options: None,
        config_preferences: Default::default(),
        mobile_review_state: serde_json::Value::Null,
        folder_id: None,
        last_event_at_ms: None,
    }]);
    assert_eq!(
        managed_calls::turn_budget_current_turn(&cold, &store, "s-cons").await,
        Ok(crate::managed_calls::service::OWNER_TURN_NONE)
    );
    assert_eq!(
        superagent::scope_history_evidence(&cold, Some(&store), "s-cons")
            .await
            .err(),
        Some("causal_history_unavailable")
    );

    store
        .record_prompt_dispatch(&PromptDispatchRecord {
            session_id: "s-cons".into(),
            cmid: "real-owner".into(),
            origin: DeliveryOrigin::Owner,
            trigger: DeliveryOrigin::Owner,
            dispatched_at_ms: 200,
        })
        .await
        .unwrap();
    let owner_events = vec![
        Envelope {
            session_id: "s-cons".into(),
            seq: 2,
            event: Event::Update {
                update: json!({
                    "sessionUpdate": "user_message_chunk",
                    "promptOrigin": { "actor": "human", "source": "composer" },
                    "content": { "type": "text", "text": "real owner" }
                }),
            },
            cmid: Some("real-owner".into()),
        },
        Envelope {
            session_id: "s-cons".into(),
            seq: 3,
            event: Event::TurnEnd {
                stop_reason: "end_turn".into(),
                usage: None,
            },
            cmid: None,
        },
    ];
    store
        .upsert_event_batch(&owner_events, &HashMap::from([("s-cons".into(), 4)]))
        .await
        .unwrap();
    let cold2 = Hub::new();
    cold2.restore_reconciling_runtime(vec![RestoredSession {
        meta,
        log: vec![],
        event_count: 4,
        reached_start: false,
        next_seq: 4,
        queue: vec![],
        drafts: vec![],
        config_options: None,
        config_preferences: Default::default(),
        mobile_review_state: serde_json::Value::Null,
        folder_id: None,
        last_event_at_ms: None,
    }]);
    assert_eq!(
        managed_calls::turn_budget_current_turn(&cold2, &store, "s-cons").await,
        Ok(2)
    );
    assert!(
        superagent::scope_history_evidence(&cold2, Some(&store), "s-cons")
            .await
            .is_ok()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn requeued_call_notice_accelerated_by_person_keeps_exact_cmid_and_reserved_conflicts_refuse()
{
    let (store, root) = test_store("notice-accel").await;
    let hub = Hub::new();
    let meta = add_owned_session(&hub, "s-not", "owner-1", false);
    store.insert_session(&meta).await.unwrap();

    let (cmid1, _) = background_jobs::record_dispatch_provenance_in(
        &hub,
        Some(&store),
        "s-not",
        Some("call-notice:cid-accel".into()),
        DeliveryOrigin::cowboy("managed_call"),
        DeliveryOrigin::cowboy("managed_call"),
    )
    .await
    .unwrap();
    assert_eq!(cmid1, "call-notice:cid-accel");

    let (cmid2, _) = background_jobs::record_dispatch_provenance_in(
        &hub,
        Some(&store),
        "s-not",
        Some("call-notice:cid-accel".into()),
        DeliveryOrigin::cowboy("managed_call"),
        DeliveryOrigin::Owner,
    )
    .await
    .unwrap();
    assert_eq!(cmid2, "call-notice:cid-accel");

    let err = background_jobs::record_dispatch_provenance_in(
        &hub,
        Some(&store),
        "s-not",
        Some("call-notice:cid-accel".into()),
        DeliveryOrigin::Owner,
        DeliveryOrigin::Owner,
    )
    .await
    .unwrap_err();
    assert_eq!(err.0, "call-notice:cid-accel");
    assert_eq!(err.1, "conflicting provenance for reserved message id");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn hub_wrap_up_front_submit_and_compacting_guards() {
    let hub = Hub::new();
    hub.create_local_session(
        "s-agy".into(),
        "antigravity".into(),
        "/tmp".into(),
        "s-agy".into(),
        SessionOrigin::Web,
        false,
    );
    hub.set_status("s-agy", crate::core::Status::Busy, None);
    hub.set_wrap_up_steering("s-agy", true);
    assert!(hub.session_info("s-agy").unwrap().meta.wrap_up_steering);

    hub.set_active_turn_compacting("s-agy", true);
    assert!(hub.is_active_turn_compacting("s-agy"));
    hub.push(
        "s-agy",
        crate::core::Event::TurnEnd {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    );
    assert!(!hub.is_active_turn_compacting("s-agy"));

    hub.set_status("s-agy", crate::core::Status::Busy, None);
    assert_eq!(
        hub.front_submit_with_outcome(
            "s-agy",
            "please wrap up".into(),
            vec![],
            Some("cmid-owner-1".into()),
            &RequestCaller::Person,
        ),
        crate::core::FrontSubmitOutcome::QueuedFront,
    );
    assert_eq!(
        hub.front_submit_with_outcome(
            "s-agy",
            "please wrap up".into(),
            vec![],
            Some("cmid-owner-1".into()),
            &RequestCaller::Person,
        ),
        crate::core::FrontSubmitOutcome::Duplicate,
    );

    assert!(
        crate::send_delivery::SendDelivery::resolve(
            Some(crate::send_delivery::SendDelivery::WrapUp),
            true,
            false
        )
        .is_err()
    );
    assert!(
        crate::send_delivery::SendDelivery::resolve(
            Some(crate::send_delivery::SendDelivery::WrapUp),
            false,
            true
        )
        .is_err()
    );
    assert_eq!(
        crate::send_delivery::SendDelivery::resolve(
            Some(crate::send_delivery::SendDelivery::WrapUp),
            false,
            false
        ),
        Ok(crate::send_delivery::SendDelivery::WrapUp)
    );
}
