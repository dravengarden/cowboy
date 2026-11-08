use super::{
    connect_bootstrap, fanout_sends_shared_frame, focused_session_bootstrap, holds_worker_slot,
    project_outbound,
};
use crate::agent_model::Status;
use crate::core::{Event, Hub, Outbound, SessionOrigin};
use crate::product_auth::ProductPrincipal;
use crate::runtime_router::RuntimeRouter;
use std::collections::HashSet;

fn test_owner_principal() -> ProductPrincipal {
    ProductPrincipal {
        user_id: "owner".to_owned(),
        username: "owner".to_owned(),
        role: crate::admin::AdminRole::Owner,
    }
}

fn hub_with_sessions() -> Hub {
    let hub = Hub::new();
    for id in ["focused", "inactive"] {
        hub.create_local_session(
            id.to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            id.to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.push(
                id,
                Event::Update {
                    update: serde_json::json!({"sessionUpdate": "agent_message_chunk", "messageId": id, "content": {"type": "text", "text": id}}),
                },
            );
    }
    hub
}

#[test]
fn every_open_session_pins_its_generation_per_machine_and_provider() {
    let hub = hub_with_sessions();
    let mut sessions = hub.session_list();
    let digest = |name: &str| format!("sha256:{}", name.repeat(64));
    sessions[0].machine_id = "ovh".to_owned();
    sessions[0].provider_generation_digest = digest("a");
    sessions[1].machine_id = "ovh".to_owned();
    sessions[1].provider_generation_digest = digest("b");
    sessions[1].status = Status::Exited;
    let mut legacy = sessions[0].clone();
    legacy.provider_generation_digest.clear();
    sessions.push(legacy);
    let referenced = super::referenced_generations(&sessions);
    assert_eq!(
        referenced.get(&("ovh".to_owned(), "codex".to_owned())),
        Some(&[digest("a"), digest("b")].into_iter().collect())
    );
    assert_eq!(referenced.len(), 1);
}

#[test]
fn only_starting_or_live_worker_sessions_hold_capacity() {
    let hub = hub_with_sessions();
    let router = RuntimeRouter::new();
    let slot = |status: Status| {
        hub.set_status("focused", status, None);
        let session = hub
            .session_list()
            .into_iter()
            .find(|session| session.id == "focused")
            .expect("session");
        holds_worker_slot(&router, &session)
    };
    // A starting session reserves its slot before the worker registers.
    assert!(slot(Status::Starting));
    // A recorded Running/Busy status without a live worker is stale and
    // holds no Agent process, so it must not keep the Machine full.
    assert!(!slot(Status::Running));
    assert!(!slot(Status::Busy));
    assert!(!slot(Status::Exited));
    let mut preparing = hub.session_list()[0].clone();
    preparing.status = Status::Starting;
    preparing.origin = SessionOrigin::Web;
    preparing.machine_id = "ovh".into();
    preparing.workspace_source_path = Some(preparing.cwd.clone());
    assert!(
        !holds_worker_slot(&router, &preparing),
        "workspace preparation is not an AI worker"
    );
}

#[test]
fn owner_fanout_projects_its_own_workspace_order() {
    let hub = Hub::new();
    let owner = test_owner_principal();
    let patch = Outbound::SyncPatch {
        state: "workspace-order".to_owned(),
        version: 3,
        value: serde_json::json!({
            "owner": ["session:a", "draft:b"],
            "other": ["session:c"],
        }),
        confirmed: vec!["c-1".to_owned()],
        resync: false,
    };
    assert!(!fanout_sends_shared_frame(&owner, &patch));
    let Some(Outbound::SyncPatch { value, .. }) =
        project_outbound(&hub, &owner, &HashSet::new(), patch)
    else {
        panic!("workspace order patch must reach its owner");
    };
    assert_eq!(value, serde_json::json!(["session:a", "draft:b"]));

    let title = Outbound::SyncPatch {
        state: "title".to_owned(),
        version: 1,
        value: serde_json::json!({}),
        confirmed: Vec::new(),
        resync: false,
    };
    assert!(fanout_sends_shared_frame(&owner, &title));
}

#[test]
fn draft_announcements_reach_only_their_owner() {
    let hub = Hub::new();
    let owner = test_owner_principal();
    let mut events = hub.subscribe();
    hub.announce_draft("other", serde_json::json!({"id": "d1", "revision": 2}));
    let frame = events.try_recv().expect("announcement must be broadcast");
    let patch = frame.outbound().clone();
    assert!(matches!(&patch, Outbound::SyncPatch { state, .. } if state == "drafts"));
    assert!(!fanout_sends_shared_frame(&owner, &patch));
    assert!(project_outbound(&hub, &owner, &HashSet::new(), patch).is_none());

    hub.announce_draft("owner", serde_json::json!({"id": "d2", "revision": 5}));
    // An older or repeated revision is not announced again.
    hub.announce_draft("owner", serde_json::json!({"id": "d2", "revision": 4}));
    let Some(Outbound::SyncPatch { value, .. }) = hub
        .sync_resync()
        .into_iter()
        .find(|message| matches!(message, Outbound::SyncPatch { state, .. } if state == "drafts"))
        .and_then(|message| project_outbound(&hub, &owner, &HashSet::new(), message))
    else {
        panic!("resync must carry the owner's announcements");
    };
    assert_eq!(
        value,
        serde_json::json!({"d2": {"id": "d2", "revision": 5}})
    );
}

#[test]
fn preferences_routes_and_announcements_are_account_scoped() {
    use super::{RouteAuth, classify_route};
    use axum::http::Method;
    for path in [
        "/api/provider-accounts/account/revocation",
        "/api/provider-accounts/account/revocation/reconcile",
    ] {
        for method in [Method::GET, Method::POST] {
            assert_eq!(classify_route(&method, path), RouteAuth::ProductOperator);
        }
    }
    assert_eq!(
        classify_route(&Method::POST, "/api/provider-accounts/account/login"),
        RouteAuth::ProductOperator
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/provider-accounts/account/login"),
        RouteAuth::ProductOperator
    );
    for method in [Method::GET, Method::PUT] {
        assert_eq!(
            classify_route(&method, "/api/provider-accounts"),
            RouteAuth::ProductOperator
        );
    }
    for method in [Method::GET, Method::DELETE] {
        assert_eq!(
            classify_route(&method, "/api/provider-accounts/account/logins/request"),
            RouteAuth::ProductOperator
        );
    }
    // Machine browser routes: the viewer routes are owner product routes,
    // the Machine's stream half is a token-authenticated Machine route.
    for (method, path) in [
        (Method::GET, "/api/provider-accounts/account/browser"),
        (Method::POST, "/api/provider-accounts/account/browser/open"),
        (
            Method::POST,
            "/api/provider-accounts/account/browser/navigate",
        ),
        (
            Method::POST,
            "/api/provider-accounts/account/browser/resize",
        ),
        (
            Method::POST,
            "/api/provider-accounts/account/browser/snapshot",
        ),
        (
            Method::POST,
            "/api/provider-accounts/account/browser/hibernate",
        ),
        (Method::POST, "/api/provider-accounts/account/browser/reset"),
        (Method::GET, "/api/provider-accounts/account/browser/stream"),
        (
            Method::POST,
            "/api/provider-accounts/account/logins/request/browser",
        ),
    ] {
        assert_eq!(classify_route(&method, path), RouteAuth::ProductOperator);
    }
    assert_eq!(
        classify_route(
            &Method::GET,
            crate::machine_protocol::machine_browser::STREAM_PATH
        ),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/preferences/about"),
        RouteAuth::Product
    );
    assert_eq!(
        classify_route(&Method::PUT, "/api/preferences"),
        RouteAuth::Product
    );
    let hub = Hub::new();
    let owner = test_owner_principal();
    let mut events = hub.subscribe();
    hub.announce_preferences("other", "about", 1);
    let patch = events.try_recv().unwrap().outbound().clone();
    assert!(!fanout_sends_shared_frame(&owner, &patch));
    assert!(project_outbound(&hub, &owner, &HashSet::new(), patch).is_none());
    hub.announce_preferences("owner", "about", 5);
    hub.announce_preferences("owner", "about", 2);
    let patch = events.try_recv().unwrap().outbound().clone();
    let Some(Outbound::SyncPatch { value, .. }) =
        project_outbound(&hub, &owner, &HashSet::new(), patch)
    else {
        panic!("missing preference announcement")
    };
    assert_eq!(value, serde_json::json!({"about":5}));
    assert!(events.try_recv().is_err());
}

#[test]
fn native_usage_invalidations_are_private_even_for_global_observers() {
    let hub = Hub::new();
    let owner = test_owner_principal();
    for id in ["owner", "other"] {
        let event = Outbound::AccountUsageChanged {
            owner_user_id: id.into(),
        };
        assert!(!fanout_sends_shared_frame(&owner, &event));
        assert_eq!(
            project_outbound(&hub, &owner, &HashSet::new(), event).is_some(),
            id == "owner"
        );
    }
}

#[test]
fn websocket_bootstrap_contains_only_global_state() {
    let messages = connect_bootstrap(
        &hub_with_sessions(),
        true,
        &test_owner_principal(),
        Some(Outbound::Machines {
            revision: 7,
            machines: Vec::new(),
            resync: true,
        }),
    );
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Outbound::Sessions { .. }))
    );
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Outbound::BootstrapComplete))
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        Outbound::Machines {
            revision: 7,
            resync: true,
            ..
        }
    )));
    let machines_index = messages
        .iter()
        .position(|message| matches!(message, Outbound::Machines { .. }))
        .expect("Machine snapshot");
    let complete_index = messages
        .iter()
        .position(|message| matches!(message, Outbound::BootstrapComplete))
        .expect("bootstrap completion");
    assert!(machines_index < complete_index);
    assert!(!messages.iter().any(|message| matches!(
        message,
        Outbound::Snapshot { .. } | Outbound::ConfigOptions { .. }
    )));
    assert!(!messages.iter().any(|message| matches!(
        message,
        Outbound::SyncPatch { state, .. } if state.starts_with("queue:")
    )));
}

#[test]
fn legacy_websocket_bootstrap_remains_complete() {
    let messages = connect_bootstrap(&hub_with_sessions(), false, &test_owner_principal(), None);
    let snapshots = messages
        .iter()
        .filter(|message| matches!(message, Outbound::Snapshot { .. }))
        .count();
    assert_eq!(snapshots, 2);
    assert!(matches!(messages.last(), Some(Outbound::BootstrapComplete)));
}

#[test]
fn focused_bootstrap_does_not_replay_another_session() {
    let messages = focused_session_bootstrap(&hub_with_sessions(), "focused")
        .expect("focused session bootstrap");
    let snapshots: Vec<_> = messages
        .iter()
        .filter_map(|message| match message {
            Outbound::Snapshot {
                session_id, events, ..
            } => Some((session_id, events)),
            _ => None,
        })
        .collect();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].0, "focused");
    assert_eq!(snapshots[0].1.len(), 1);
    assert!(messages.iter().all(|message| match message {
        Outbound::Snapshot { session_id, .. } | Outbound::ConfigOptions { session_id, .. } =>
            session_id == "focused",
        Outbound::SyncPatch { state, .. } => state == "queue:focused",
        _ => true,
    }));
}
