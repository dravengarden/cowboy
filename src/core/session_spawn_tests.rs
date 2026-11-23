//! Sessions an agent creates are filed beside their creator.
use super::*;
use crate::core::{Outbound, SessionOrigin, SessionRegistration};

fn session(hub: &Hub, id: &str, owner: &str, project: &str) {
    hub.create_session(SessionRegistration {
        id: id.into(),
        provider: "codex".into(),
        provider_account_id: Default::default(),
        provider_version: "fixture".into(),
        provider_generation_digest: "fixture".into(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: "hawk".into(),
        workspace_id: Some(project.into()),
        workspace_name: Some(project.into()),
        workspace_source_path: None,
        execution_binding: None,
        cwd: format!("/worktrees/{id}"),
        title: format!("Task {id}"),
        origin: SessionOrigin::Web,
        system: false,
        owner_user_id: Some(owner.into()),
        owner_username: None,
    });
}

fn me() -> FolderActor {
    FolderActor {
        user_id: Some("me".into()),
        sees_all: false,
    }
}

fn apply(hub: &Hub, state: &str, name: &str, args: serde_json::Value) {
    hub.sync_apply_as(&me(), state, format!("{state}:{name}:{args}"), name, &args)
        .unwrap();
}

/// The Sessions list top to bottom, as both Web sidebars show it.
fn displayed(hub: &Hub) -> Vec<String> {
    let order = hub.sync_value("order");
    order
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .map(|id| id.as_str().unwrap().to_owned())
        .collect()
}

fn workspace(hub: &Hub) -> Vec<String> {
    hub.sync_value("workspace-order")["me"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| row.as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn placement(hub: &Hub, id: &str) -> Option<String> {
    hub.sync_value("folders")["placement"][id]
        .as_str()
        .map(str::to_owned)
}

/// A filed creator, two neighbours around it and a foreign session.
fn filed_creator(hub: &Hub, folder: &str) {
    for id in ["above", "creator", "below"] {
        session(hub, id, "me", "cowboy");
    }
    session(hub, "foreign", "them", "cowboy");
    apply(hub, "folders", "create", json!({"id":folder,"name":"Work"}));
    apply(
        hub,
        "folders",
        "place",
        json!({"session_ids":["creator"],"folder":folder}),
    );
    apply(
        hub,
        "order",
        "reorder",
        json!({"order":["below","foreign","creator","above"]}),
    );
}

#[test]
fn spawns_follow_the_creator_in_its_folder_and_keep_creation_order() {
    let hub = Hub::new();
    filed_creator(&hub, "work");
    for id in ["first", "second", "third"] {
        session(&hub, id, "me", "cowboy");
    }
    for id in ["first", "second", "third"] {
        assert!(hub.place_spawned("creator", id, &me()).unwrap());
    }
    for id in ["first", "second", "third"] {
        assert_eq!(placement(&hub, id).as_deref(), Some("work"), "{id}");
    }
    assert_eq!(
        displayed(&hub),
        [
            "above", "creator", "first", "second", "third", "foreign", "below"
        ]
    );
    let rows = workspace(&hub);
    let at = rows.iter().position(|id| id == "session:creator").unwrap();
    assert_eq!(
        rows[at..at + 4],
        [
            "session:creator",
            "session:first",
            "session:second",
            "session:third"
        ]
    );
    // The creator and every row away from it stay where they were.
    assert_eq!(placement(&hub, "creator").as_deref(), Some("work"));
    assert_eq!(placement(&hub, "above"), None);
}

#[test]
fn a_later_spawn_lands_after_the_earlier_ones_even_after_a_restart() {
    let hub = Hub::new();
    filed_creator(&hub, "work");
    for id in ["first", "second", "third"] {
        session(&hub, id, "me", "cowboy");
    }
    for id in ["first", "second"] {
        hub.place_spawned("creator", id, &me()).unwrap();
    }
    let mark = |id: &str| {
        format!(
            "{}{id}",
            super::super::settings_keys::SESSION_SPAWNED_FROM_PREFIX
        )
    };
    assert_eq!(hub.setting(&mark("second")), Some(json!("creator")));
    assert_eq!(hub.setting(&mark("creator")), None);

    let restored = Hub::new();
    for id in ["above", "creator", "below", "first", "second", "third"] {
        session(&restored, id, "me", "cowboy");
    }
    session(&restored, "foreign", "them", "cowboy");
    restored.load_settings(
        ["first", "second"]
            .map(|id| (mark(id), hub.setting(&mark(id)).unwrap()))
            .into(),
    );
    apply(
        &restored,
        "order",
        "reorder",
        json!({"order": hub.sync_value("order")}),
    );
    restored.place_spawned("creator", "third", &me()).unwrap();
    let list = displayed(&restored);
    let creator = list.iter().position(|id| id == "creator").unwrap();
    assert_eq!(
        list[creator..creator + 4],
        ["creator", "first", "second", "third"]
    );
}

#[test]
fn a_purged_earlier_spawn_does_not_reverse_the_visible_order() {
    let hub = Hub::new();
    filed_creator(&hub, "work");
    for id in ["first", "second", "third"] {
        session(&hub, id, "me", "cowboy");
    }
    for id in ["first", "second"] {
        hub.place_spawned("creator", id, &me()).unwrap();
    }
    assert!(hub.delete_session("first"));
    hub.place_spawned("creator", "third", &me()).unwrap();
    let live = |rows: Vec<String>| -> Vec<String> {
        rows.into_iter()
            .filter(|row| {
                let id = row.strip_prefix("session:").unwrap_or(row);
                hub.session_info(id).is_some()
            })
            .collect()
    };
    assert_eq!(
        displayed(&hub),
        ["above", "creator", "second", "third", "foreign", "below"]
    );
    let rows = live(workspace(&hub));
    let at = rows
        .iter()
        .position(|row| row == "session:creator")
        .unwrap();
    assert_eq!(
        rows[at..at + 3],
        ["session:creator", "session:second", "session:third"]
    );
}

#[test]
fn spawns_filed_out_of_order_still_read_in_creation_order() {
    let hub = Hub::new();
    filed_creator(&hub, "work");
    // Ids come from one counter; overlapping creations can be filed in any order.
    for id in ["sess-11", "sess-12", "sess-13"] {
        session(&hub, id, "me", "cowboy");
    }
    for id in ["sess-12", "sess-11", "sess-13"] {
        assert!(hub.place_spawned("creator", id, &me()).unwrap());
    }
    assert_eq!(
        displayed(&hub),
        [
            "above", "creator", "sess-11", "sess-12", "sess-13", "foreign", "below"
        ]
    );
    let rows = workspace(&hub);
    let at = rows
        .iter()
        .position(|row| row == "session:creator")
        .unwrap();
    assert_eq!(
        rows[at..at + 4],
        [
            "session:creator",
            "session:sess-11",
            "session:sess-12",
            "session:sess-13"
        ]
    );
}

#[test]
fn a_session_in_trash_between_the_spawns_does_not_break_their_order() {
    let hub = Hub::new();
    filed_creator(&hub, "work");
    for id in ["first", "stray", "second"] {
        session(&hub, id, "me", "cowboy");
    }
    hub.place_spawned("creator", "first", &me()).unwrap();
    // An unrelated session moved in among them, then trashed.
    apply(
        &hub,
        "order",
        "reorder",
        json!({"order": ["below", "foreign", "first", "stray", "creator", "above"]}),
    );
    hub.trash_items(&me(), &[], &["stray".into()], 1).unwrap();
    hub.place_spawned("creator", "second", &me()).unwrap();
    let list: Vec<_> = displayed(&hub)
        .into_iter()
        .filter(|id| id != "stray")
        .collect();
    assert_eq!(
        list,
        ["above", "creator", "first", "second", "foreign", "below"]
    );
}

#[test]
fn a_creator_at_the_top_level_keeps_its_spawns_there() {
    let hub = Hub::new();
    session(&hub, "creator", "me", "cowboy");
    session(&hub, "spawn", "me", "cowboy");
    // No folder claims either session: nothing is filed.
    assert!(hub.place_spawned("creator", "spawn", &me()).unwrap());
    assert_eq!(placement(&hub, "spawn"), None);
    assert_eq!(displayed(&hub)[..2], ["creator", "spawn"]);

    // A folder bound to the spawn's project would file it away from a creator
    // that was moved out to the top level, so the spawn is pinned there too.
    session(&hub, "other-project", "me", "garden");
    session(&hub, "later", "me", "garden");
    apply(
        &hub,
        "folders",
        "create",
        json!({"id":"garden","name":"Garden","project":"garden"}),
    );
    assert!(hub.place_spawned("creator", "later", &me()).unwrap());
    assert_eq!(placement(&hub, "later").as_deref(), Some(""));
    assert_eq!(
        displayed(&hub),
        ["other-project", "creator", "spawn", "later"]
    );

    apply(
        &hub,
        "folders",
        "place",
        json!({"session_ids":["creator"],"folder":null}),
    );
    assert!(
        hub.place_spawned("creator", "other-project", &me())
            .unwrap()
    );
    assert_eq!(placement(&hub, "other-project").as_deref(), Some(""));
}

#[test]
fn a_creator_in_a_nested_folder_keeps_its_spawn_in_that_folder() {
    let hub = Hub::new();
    session(&hub, "creator", "me", "cowboy");
    session(&hub, "spawn", "me", "cowboy");
    apply(
        &hub,
        "folders",
        "create",
        json!({"id":"outer","name":"Outer"}),
    );
    apply(
        &hub,
        "folders",
        "create",
        json!({"id":"inner","name":"Inner","parent":"outer"}),
    );
    apply(
        &hub,
        "folders",
        "place",
        json!({"session_ids":["creator"],"folder":"inner"}),
    );
    assert!(hub.place_spawned("creator", "spawn", &me()).unwrap());
    assert_eq!(placement(&hub, "spawn").as_deref(), Some("inner"));
    assert_eq!(displayed(&hub)[..2], ["creator", "spawn"]);
}

#[test]
fn a_project_bound_folder_that_files_the_creator_files_its_spawn() {
    let hub = Hub::new();
    session(&hub, "creator", "me", "cowboy");
    session(&hub, "spawn", "me", "garden");
    apply(
        &hub,
        "folders",
        "create",
        json!({"id":"cowboy","name":"Cowboy","project":"cowboy"}),
    );
    apply(
        &hub,
        "folders",
        "create",
        json!({"id":"garden","name":"Garden","project":"garden"}),
    );
    // The spawn belongs to another project, yet sits beside its creator.
    assert!(hub.place_spawned("creator", "spawn", &me()).unwrap());
    assert_eq!(placement(&hub, "spawn").as_deref(), Some("cowboy"));
}

#[test]
fn a_trashed_or_deleted_creator_leaves_the_spawn_where_it_was_created() {
    let hub = Hub::new();
    filed_creator(&hub, "work");
    session(&hub, "spawn", "me", "cowboy");
    let before = (
        hub.sync_value("folders"),
        hub.sync_value("order"),
        hub.sync_value("workspace-order"),
    );
    hub.trash_items(&me(), &[], &["creator".into()], 1).unwrap();
    assert!(!hub.place_spawned("creator", "spawn", &me()).unwrap());
    assert!(!hub.place_spawned("gone", "spawn", &me()).unwrap());
    assert_eq!(placement(&hub, "spawn"), None);
    assert_eq!(hub.sync_value("order"), before.1);
    assert_eq!(hub.sync_value("workspace-order"), before.2);
    assert_eq!(hub.setting("session_spawned_from:spawn"), None);

    // Restored, the creator takes spawns again.
    hub.restore_items(&me(), &[], &["creator".into()]).unwrap();
    assert!(hub.place_spawned("creator", "spawn", &me()).unwrap());
    assert_eq!(placement(&hub, "spawn").as_deref(), Some("work"));
}

#[test]
fn filing_never_lifts_a_language_server_ban_but_may_add_one() {
    let hub = Hub::new();
    session(&hub, "creator", "me", "cowboy");
    session(&hub, "locked-spawn", "me", "garden");
    session(&hub, "open-spawn", "me", "cowboy");
    apply(
        &hub,
        "folders",
        "create",
        json!({"id":"open","name":"Open"}),
    );
    apply(
        &hub,
        "folders",
        "create",
        json!({"id":"garden","name":"Garden","project":"garden",
            "defaults":{"language_servers":"off"}}),
    );
    apply(
        &hub,
        "folders",
        "place",
        json!({"session_ids":["creator"],"folder":"open"}),
    );
    // The garden folder claims this spawn and bans language servers: an agent's
    // implicit placement beside a creator in an open folder must not lift that.
    let before = (hub.sync_value("folders"), hub.sync_value("order"));
    assert!(!hub.place_spawned("creator", "locked-spawn", &me()).unwrap());
    assert_eq!((hub.sync_value("folders"), hub.sync_value("order")), before);
    assert_eq!(hub.setting("session_spawned_from:locked-spawn"), None);
    assert_eq!(placement(&hub, "locked-spawn"), None);

    // The same folder as the creator's is no change; a ban the creator's folder
    // carries is simply inherited by the spawn.
    apply(
        &hub,
        "folders",
        "place",
        json!({"session_ids":["creator"],"folder":"garden"}),
    );
    assert!(hub.place_spawned("creator", "locked-spawn", &me()).unwrap());
    assert!(hub.place_spawned("creator", "open-spawn", &me()).unwrap());
    assert_eq!(placement(&hub, "open-spawn").as_deref(), Some("garden"));
}

#[test]
fn another_owners_creator_changes_nothing() {
    let hub = Hub::new();
    filed_creator(&hub, "work");
    session(&hub, "spawn", "me", "cowboy");
    let before = (
        hub.sync_value("folders"),
        hub.sync_value("order"),
        hub.sync_value("workspace-order"),
    );
    assert!(hub.place_spawned("foreign", "spawn", &me()).is_err());
    assert_eq!(
        (
            hub.sync_value("folders"),
            hub.sync_value("order"),
            hub.sync_value("workspace-order")
        ),
        before
    );
    assert_eq!(hub.setting("session_spawned_from:spawn"), None);
}

#[test]
fn placement_reaches_connected_clients_live() {
    let hub = Hub::new();
    filed_creator(&hub, "work");
    session(&hub, "spawn", "me", "cowboy");
    let mut live = hub.subscribe();
    hub.place_spawned("creator", "spawn", &me()).unwrap();
    let mut patched = std::collections::BTreeSet::new();
    while let Ok(frame) = live.try_recv() {
        if let Outbound::SyncPatch { state, value, .. } = frame.outbound() {
            if state == "folders" {
                assert_eq!(value["placement"]["spawn"], "work");
            }
            patched.insert(state.clone());
        }
    }
    for state in ["folders", "order", "workspace-order"] {
        assert!(patched.contains(state), "no live patch for {state}");
    }
}
