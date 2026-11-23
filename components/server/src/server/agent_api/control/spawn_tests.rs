use super::*;
use crate::core::{Hub, SessionOrigin, SessionRegistration};

fn prepared(body: Value) -> Result<(Spawn, Value), &'static str> {
    let mut input = json!({"body": body});
    Spawn::prepare(&mut input).map(|spawn| (spawn, input))
}

fn actor() -> FolderActor {
    FolderActor {
        user_id: Some("owner".into()),
        sees_all: false,
    }
}

fn session(hub: &Hub, id: &str) {
    hub.create_session(SessionRegistration {
        id: id.into(),
        provider: "codex".into(),
        provider_account_id: Default::default(),
        provider_version: "1".into(),
        provider_generation_digest: "generation".into(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: "hawk".into(),
        workspace_id: Some("cowboy".into()),
        workspace_name: Some("cowboy".into()),
        workspace_source_path: None,
        execution_binding: None,
        cwd: format!("/worktrees/{id}"),
        title: format!("codex · /worktrees/{id}"),
        origin: SessionOrigin::Web,
        system: false,
        owner_user_id: Some("owner".into()),
        owner_username: None,
    });
}

/// The receipt a write returns for a creation the product handler answered.
fn receipt(status: u16, session: &str) -> Value {
    json!({"status":"accepted","http_status":status,"result":{"session_id":session}})
}

fn placement(hub: &Hub, id: &str) -> Option<String> {
    hub.sync_value("folders")["placement"][id]
        .as_str()
        .map(str::to_owned)
}

fn folder(hub: &Hub, id: &str) {
    hub.sync_apply_as(
        &actor(),
        "folders",
        format!("create-{id}"),
        "create",
        &json!({"id":id,"name":id}),
    )
    .unwrap();
}

#[test]
fn the_title_is_validated_like_a_rename_and_stays_out_of_the_product_body() {
    let (spawn, input) = prepared(json!({"machine_id":"hawk","title":"  Review the diff  "}))
        .expect("a padded title is trimmed");
    assert_eq!(spawn.title.as_deref(), Some("Review the diff"));
    assert!(input["body"].get("title").is_none());
    assert_eq!(input["body"]["origin"], "web");
    for ignored in [json!({"machine_id":"hawk"}), json!({"title":null})] {
        assert_eq!(prepared(ignored).unwrap().0.title, None);
    }
    // `sessions.rename` refuses these; so does creation, before anything exists.
    for title in [json!(""), json!(" \t\n"), json!(7), json!(["x"]), json!({})] {
        assert_eq!(
            prepared(json!({"title":title})).err(),
            Some("invalid_argument"),
            "{title}"
        );
    }
}

#[test]
fn an_agent_cannot_name_the_owner_origin_or_system_flag() {
    for key in ["system", "owner_user_id", "origin"] {
        assert_eq!(
            prepared(json!({ key: "x" })).err(),
            Some("invalid_argument")
        );
    }
    assert!(Spawn::check(&json!({})).is_err());
}

#[test]
fn only_a_request_naming_no_folder_follows_its_creator() {
    for (body, beside) in [
        (json!({}), true),
        (json!({"folder_id":null}), true),
        (json!({"folder_id":""}), false),
        (json!({"folder_id":"f-work"}), false),
    ] {
        assert_eq!(
            prepared(body.clone()).unwrap().0.beside_creator,
            beside,
            "{body}"
        );
    }
}

#[test]
fn a_created_session_is_titled_and_filed_beside_its_creator() {
    let hub = Hub::new();
    for id in ["creator", "spawn"] {
        session(&hub, id);
    }
    folder(&hub, "work");
    hub.sync_apply_as(
        &actor(),
        "folders",
        "place".into(),
        "place",
        &json!({"session_ids":["creator"],"folder":"work"}),
    )
    .unwrap();
    let (spawn, _) = prepared(json!({"title":"Port the parser"})).unwrap();
    spawn.finish(&hub, "creator", &actor(), &receipt(201, "spawn"));
    assert_eq!(
        hub.session_info("spawn").unwrap().meta.title,
        "Port the parser"
    );
    assert_eq!(placement(&hub, "spawn").as_deref(), Some("work"));
    // Without a title the creation-time default stays for the prompt to replace.
    session(&hub, "plain");
    let (spawn, _) = prepared(json!({})).unwrap();
    spawn.finish(&hub, "creator", &actor(), &receipt(201, "plain"));
    assert_eq!(
        hub.session_info("plain").unwrap().meta.title,
        "codex · /worktrees/plain"
    );
    assert_eq!(placement(&hub, "plain").as_deref(), Some("work"));
}

#[test]
fn an_explicit_folder_is_never_overridden() {
    let hub = Hub::new();
    for id in ["creator", "spawn"] {
        session(&hub, id);
    }
    folder(&hub, "work");
    hub.sync_apply_as(
        &actor(),
        "folders",
        "place".into(),
        "place",
        &json!({"session_ids":["creator"],"folder":"work"}),
    )
    .unwrap();
    // The product handler filed the session in the folder the caller named.
    hub.sync_apply_as(
        &actor(),
        "folders",
        "place-spawn".into(),
        "place",
        &json!({"session_ids":["spawn"],"folder":null}),
    )
    .unwrap();
    let (spawn, _) = prepared(json!({"folder_id":"","title":"Elsewhere"})).unwrap();
    spawn.finish(&hub, "creator", &actor(), &receipt(201, "spawn"));
    assert_eq!(placement(&hub, "spawn").as_deref(), Some(""));
    assert_eq!(hub.session_info("spawn").unwrap().meta.title, "Elsewhere");
    assert_eq!(hub.setting("session_spawned_from:spawn"), None);
}

#[test]
fn a_refused_creation_or_a_vanished_creator_never_fails_the_session() {
    let hub = Hub::new();
    for id in ["creator", "spawn"] {
        session(&hub, id);
    }
    folder(&hub, "work");
    hub.sync_apply_as(
        &actor(),
        "folders",
        "place".into(),
        "place",
        &json!({"session_ids":["creator"],"folder":"work"}),
    )
    .unwrap();
    // A rejected receipt names no session: nothing is renamed or moved.
    let (spawn, _) = prepared(json!({"title":"Never applied"})).unwrap();
    spawn.finish(&hub, "creator", &actor(), &receipt(409, "spawn"));
    assert_eq!(placement(&hub, "spawn"), None);
    assert_eq!(
        hub.session_info("spawn").unwrap().meta.title,
        "codex · /worktrees/spawn"
    );
    // A creator deleted in the meantime: the session keeps today's placement
    // and still gets its title.
    let (spawn, _) = prepared(json!({"title":"Orphan"})).unwrap();
    spawn.finish(&hub, "deleted", &actor(), &receipt(201, "spawn"));
    assert_eq!(placement(&hub, "spawn"), None);
    assert_eq!(hub.session_info("spawn").unwrap().meta.title, "Orphan");
    // So does one in Trash.
    hub.trash_items(&actor(), &[], &["creator".into()], 1)
        .unwrap();
    let (spawn, _) = prepared(json!({})).unwrap();
    spawn.finish(&hub, "creator", &actor(), &receipt(201, "spawn"));
    assert_eq!(placement(&hub, "spawn"), None);
}

#[test]
fn a_malformed_body_is_refused_before_admission() {
    assert_eq!(
        Spawn::check(&json!({"body":{"title":"  "}})),
        Err("invalid_argument")
    );
    assert_eq!(
        Spawn::check(&json!({"body":{"origin":"api"}})),
        Err("invalid_argument")
    );
    assert_eq!(Spawn::check(&json!({"body":{"title":"Fine"}})), Ok(()));
    // The product handler's own request type decides the rest, so nothing it
    // would reject is admitted first.
    for body in [
        json!({"folder_id":true}),
        json!({"context_sources":"one"}),
        json!({"machine_id":7}),
    ] {
        assert_eq!(
            Spawn::check(&json!({ "body": body })),
            Err("invalid_argument"),
            "{body}"
        );
    }
    assert_eq!(
        Spawn::check(
            &json!({"body":{"folder_id":"","preset":"codex/fast","context_sources":["sess-1"]}})
        ),
        Ok(())
    );
    // The check never changes the caller's input.
    let input = json!({"body":{"title":" Fine "}});
    Spawn::check(&input).unwrap();
    assert_eq!(input["body"]["title"], " Fine ");
}
