use super::*;
use crate::core::{Hub, SessionOrigin, SessionRegistration};

fn session(hub: &Hub, id: &str, owner: Option<&str>, system: bool) {
    hub.create_session(SessionRegistration {
        id: id.into(),
        provider: "codex".into(),
        provider_account_id: Default::default(),
        provider_version: String::new(),
        provider_generation_digest: String::new(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: "hawk".into(),
        workspace_id: None,
        workspace_name: None,
        workspace_source_path: None,
        execution_binding: None,
        cwd: "/fixture".into(),
        title: String::new(),
        origin: SessionOrigin::Web,
        system,
        owner_user_id: owner.map(str::to_owned),
        owner_username: None,
    });
    hub.add_draft(id, "other session draft".into(), vec![], None);
    hub.push(id, Event::Update { update: json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"other session reply"}}) });
}

#[test]
fn session_filters_combine_exact_identity_with_unicode_metadata_search() {
    let hub = Hub::new();
    session(&hub, "s", Some("owner"), false);
    let mut meta = hub.session_info("s").unwrap().meta;
    meta.title = "检查 Cowboy MCP".into();
    meta.workspace_id = Some("project-id".into());
    assert!(
        ListFilter::parse(&json!({"query":"cowboy","provider":"codex","workspace":"project-id"}))
            .unwrap()
            .matches(&meta)
    );
    assert!(
        ListFilter::parse(&json!({"query":"检查"}))
            .unwrap()
            .matches(&meta)
    );
    assert!(
        !ListFilter::parse(&json!({"query":"cowboy","machine":"other"}))
            .unwrap()
            .matches(&meta)
    );
    for args in [
        json!({"query":""}),
        json!({"query":"x".repeat(257)}),
        json!({"provider":1}),
    ] {
        assert!(ListFilter::parse(&args).is_err());
    }
}

#[test]
fn content_reads_share_owner_visibility_and_never_wake_a_target() {
    let root = tempfile::tempdir().unwrap();
    let plugins = Arc::new(crate::plugin_catalog::PluginCatalog::open(root.path(), None).unwrap());
    let catalog = crate::provider_catalog::ProviderCatalog::open(root.path(), plugins).unwrap();
    let hub = Hub::new();
    for (id, owner, system) in [
        ("parent", Some("me"), false),
        ("mine", Some("me"), false),
        ("foreign", Some("other"), false),
        ("anonymous", None, false),
        ("system", Some("me"), true),
        ("closing", Some("me"), false),
    ] {
        session(&hub, id, owner, system);
    }
    hub.set_closing("closing", true);
    let parent = hub.session_info("parent").unwrap().meta;
    let before = hub.session_info("mine").unwrap();
    let drafts = read(
        &hub,
        &catalog,
        &parent,
        "sessions.drafts",
        json!({"session_id":"mine"}),
    )
    .unwrap();
    assert_eq!(drafts[0]["text"], "other session draft");
    let message = read(
        &hub,
        &catalog,
        &parent,
        "sessions.message",
        json!({"session_id":"mine","id":drafts[0]["id"],"chars":5}),
    )
    .unwrap();
    assert_eq!(message["text"], "other");
    assert_eq!(message["range"]["next"], 5);
    let events = read(
        &hub,
        &catalog,
        &parent,
        "sessions.events",
        json!({"session_id":"mine","kinds":["agent"]}),
    )
    .unwrap();
    assert_eq!(events["items"][0]["text"], "other session reply");
    let usage = read(
        &hub,
        &catalog,
        &parent,
        "sessions.usage",
        json!({"id":"mine"}),
    )
    .unwrap();
    assert_eq!(usage["account"], Value::Null); // unknown generation never guesses an account
    assert_eq!(usage["usage"], Value::Null);
    assert_eq!(
        hub.session_info("mine").unwrap().meta.status,
        before.meta.status
    );
    assert_eq!(
        hub.session_info("mine").unwrap().drafts_count,
        before.drafts_count
    );
    for id in ["foreign", "anonymous", "system", "closing", "missing"] {
        for (op, args) in [
            ("sessions.info", json!({"id":id})),
            ("sessions.usage", json!({"id":id})),
            ("sessions.queue", json!({"session_id":id})),
            ("sessions.drafts", json!({"session_id":id})),
            ("sessions.message", json!({"session_id":id,"id":"message"})),
            ("sessions.events", json!({"session_id":id})),
        ] {
            assert_eq!(
                read(&hub, &catalog, &parent, op, args),
                Err("not_found".into()),
                "{id} {op}"
            );
        }
    }
    assert_eq!(
        read(
            &hub,
            &catalog,
            &parent,
            "session.events",
            json!({"session_id":"mine"})
        ),
        Err(
            "invalid_argument: session_id is not accepted; options are after, before, limit, chars, kinds"
                .into()
        )
    );
    hub.set_closing("mine", true);
    assert_eq!(
        read(
            &hub,
            &catalog,
            &parent,
            "sessions.events",
            json!({"session_id":"mine"})
        ),
        Err("not_found".into())
    );
}

#[tokio::test]
async fn archived_history_is_paged_without_hydrating_or_waking_the_session() {
    const OWNER: &str = "0123456789abcdef0123456789abcdef";
    let root = tempfile::tempdir().unwrap();
    let store = crate::store::Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    store
        .insert_user(&crate::store::ProductUser {
            id: OWNER.into(),
            username: "reader".into(),
            password_algo: "fixture".into(),
            password_hash: "unused".into(),
            created_at_ms: 1,
            updated_at_ms: 1,
            disabled_at_ms: None,
        })
        .await
        .unwrap();
    let hub = Hub::new();
    session(&hub, "parent", Some(OWNER), false);
    session(&hub, "archived", Some(OWNER), false);
    let parent = hub.session_info("parent").unwrap().meta;
    let before = hub.session_info("archived").unwrap();
    store.insert_session(&before.meta).await.unwrap();
    let events: Vec<_> = (0..130)
        .map(|seq| Envelope {
            session_id: "archived".into(),
            seq,
            cmid: None,
            event: Event::TurnEnd {
                usage: None,
                stop_reason: format!("saved-{seq}"),
            },
        })
        .collect();
    store
        .upsert_event_batch(&events, &HashMap::from([("archived".into(), 130)]))
        .await
        .unwrap();
    let mut args = json!({"session_id":"archived","limit":100});
    let mut all = Vec::new();
    for _ in 0..4 {
        let page = history(&hub, Some(&store), &parent, true, args.clone())
            .await
            .unwrap();
        assert_eq!(page["source"], "durable");
        all.extend(
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["seq"].as_u64().unwrap()),
        );
        if page["next"].is_null() {
            assert_eq!(page["reached_start"], true);
            break;
        }
        args["before"] = page["next"]["before"].clone();
    }
    all.sort_unstable();
    assert_eq!(all, (0..130).collect::<Vec<_>>());
    let after = hub.session_info("archived").unwrap();
    assert_eq!(after.event_count, before.event_count);
    assert_eq!(after.meta.status, before.meta.status);
    let detail_args = json!({"session_id":"archived","seq":80,"path":"/stop_reason","chars":4});
    let detail = super::super::event_reads::read(&hub, Some(&store), &parent, true, &detail_args)
        .await
        .unwrap();
    assert_eq!(detail["source"], "durable");
    assert_eq!(detail["page"]["data"], "save");
    let next = super::super::event_reads::read(&hub,Some(&store),&parent,true,
        &json!({"session_id":"archived","seq":80,"path":"/stop_reason","offset":4,"expected":detail["revision"]})).await.unwrap();
    assert_eq!(next["page"]["data"], "d-80");
    assert_eq!(
        super::super::event_reads::read(
            &hub,
            Some(&store),
            &parent,
            true,
            &json!({"session_id":"archived","seq":80,"expected":"old"})
        )
        .await,
        Err("revision_conflict".into())
    );
    let found = super::super::event_reads::search(
        &hub,
        Some(&store),
        &parent,
        true,
        &json!({"session_id":"archived","query":"saved-8","limit":2}),
    )
    .await
    .unwrap();
    assert_eq!(found["matches"][0]["seq"], 89);
    assert_eq!(found["matches"][1]["seq"], 88);
    assert_eq!(found["next_before"], 88);
    let empty = super::super::event_reads::search(
        &hub,
        Some(&store),
        &parent,
        true,
        &json!({"session_id":"archived","query":"absent"}),
    )
    .await
    .unwrap();
    assert_eq!(empty["matches"], json!([]));
    assert!(!empty["next_before"].is_null());
    let mut foreign = parent.clone();
    foreign.owner_user_id = Some("other".into());
    assert_eq!(
        super::super::event_reads::read(&hub, Some(&store), &foreign, true, &detail_args).await,
        Err("not_found".into())
    );
    assert_eq!(
        super::super::event_reads::search(
            &hub,
            Some(&store),
            &foreign,
            true,
            &json!({"session_id":"archived","query":"saved"})
        )
        .await,
        Err("not_found".into())
    );
    assert_eq!(
        history(&hub, Some(&store), &foreign, true, args.clone()).await,
        Err("not_found".into())
    );
    hub.set_closing("archived", true);
    assert_eq!(
        history(&hub, Some(&store), &parent, true, args).await,
        Err("not_found".into())
    );
    assert_eq!(
        history(&hub, Some(&store), &parent, false, json!({"after":0})).await,
        Err(
            "invalid_argument: after is not accepted; options are before, limit, chars, kinds"
                .into()
        )
    );
}
