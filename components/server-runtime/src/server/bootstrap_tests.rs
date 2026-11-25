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
        classify_route(&Method::POST, "/api/provider-accounts/account/credentials"),
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

#[test]
fn fanout_delta_projection_enforces_seq_visibility_and_legacy_fallback() {
    use super::client_ws::{FanoutDelivery, LegacySessionTracker, resolve_fanout_delivery};
    use super::connect_bootstrap_with_seq;
    use crate::core::SessionRegistration;

    let hub = Hub::new();
    let mut rx = hub.subscribe();
    let register = |id: &str, owner: Option<&str>| {
        hub.create_session(SessionRegistration {
            id: id.to_owned(),
            provider: "codex".to_owned(),
            provider_version: "1.0.0".to_owned(),
            provider_generation_digest: "sha256:abc".to_owned(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: Some(crate::provider::legacy_behavior("codex")),
            machine_id: "hawk".to_owned(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: "/tmp".to_owned(),
            title: id.to_owned(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: owner.map(str::to_owned),
            owner_username: owner.map(str::to_owned),
        });
    };
    let owner = test_owner_principal();
    let member = ProductPrincipal {
        user_id: "member-a".to_owned(),
        username: "member-a".to_owned(),
        role: crate::admin::AdminRole::Operator,
    };
    let other_member = ProductPrincipal {
        user_id: "member-b".to_owned(),
        username: "member-b".to_owned(),
        role: crate::admin::AdminRole::Operator,
    };

    register("s-a", Some("member-a"));
    let pre_bootstrap = rx.try_recv().expect("pre-bootstrap frame");
    let (bootstrap, bootstrap_seq) = connect_bootstrap_with_seq(&hub, true, &member, None);
    let Some(Outbound::Sessions { sessions }) = bootstrap.first() else {
        panic!("expected bootstrap Sessions frame");
    };
    assert_eq!(sessions.len(), 1);
    assert!(
        sessions[0].provider_behavior.is_none(),
        "bootstrap must strip provider_behavior"
    );

    // Pre-bootstrap frame is skipped regardless of client delta support.
    let mut no_legacy = None;
    let mut member_legacy = LegacySessionTracker::from_bootstrap(&bootstrap, false);
    let mut other_legacy = Some(LegacySessionTracker::default());
    assert!(matches!(
        resolve_fanout_delivery(
            &hub,
            &member,
            true,
            bootstrap_seq,
            &mut no_legacy,
            &pre_bootstrap
        ),
        FanoutDelivery::Skip
    ));
    assert!(matches!(
        resolve_fanout_delivery(
            &hub,
            &member,
            false,
            bootstrap_seq,
            &mut member_legacy,
            &pre_bootstrap
        ),
        FanoutDelivery::Skip
    ));

    // Post-bootstrap mutations to member-a's session: queue closing=true, then
    // closing=false BEFORE draining on the legacy socket to verify legacy
    // clients replay each delta in order instead of reading newer Hub state.
    hub.set_background_tasks("s-a", 3);
    let updated = rx.try_recv().expect("update frame");
    hub.set_closing("s-a", true);
    let closing_true = rx.try_recv().expect("closing true frame");
    hub.set_closing("s-a", false);
    let closing_false = rx.try_recv().expect("closing false frame");

    assert!(matches!(
        resolve_fanout_delivery(&hub, &member, true, bootstrap_seq, &mut no_legacy, &updated),
        FanoutDelivery::Shared(frame) if matches!(frame.outbound(), Outbound::SessionUpdated { .. })
    ));
    assert!(matches!(
        resolve_fanout_delivery(&hub, &owner, true, bootstrap_seq, &mut no_legacy, &updated),
        FanoutDelivery::Shared(frame) if matches!(frame.outbound(), Outbound::SessionUpdated { .. })
    ));
    assert!(matches!(
        resolve_fanout_delivery(
            &hub,
            &other_member,
            true,
            bootstrap_seq,
            &mut no_legacy,
            &updated
        ),
        FanoutDelivery::Skip
    ));
    assert!(matches!(
        resolve_fanout_delivery(
            &hub,
            &other_member,
            false,
            bootstrap_seq,
            &mut other_legacy,
            &updated
        ),
        FanoutDelivery::Skip
    ));
    match resolve_fanout_delivery(
        &hub,
        &member,
        false,
        bootstrap_seq,
        &mut member_legacy,
        &updated,
    ) {
        FanoutDelivery::Owned(outbound) => match *outbound {
            Outbound::Sessions { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].background_tasks, 3);
                assert!(!sessions[0].closing);
                assert!(sessions[0].provider_behavior.is_none());
            }
            _ => panic!("legacy client must receive translated full Sessions list"),
        },
        _ => panic!("legacy client must receive translated full Sessions list"),
    }
    match resolve_fanout_delivery(
        &hub,
        &member,
        false,
        bootstrap_seq,
        &mut member_legacy,
        &closing_true,
    ) {
        FanoutDelivery::Owned(outbound) => match *outbound {
            Outbound::Sessions { sessions } => assert!(
                sessions[0].closing,
                "legacy client must observe transient closing=true in sequence"
            ),
            _ => panic!("expected translated Sessions list"),
        },
        _ => panic!("expected translated Sessions list"),
    }
    match resolve_fanout_delivery(
        &hub,
        &member,
        false,
        bootstrap_seq,
        &mut member_legacy,
        &closing_false,
    ) {
        FanoutDelivery::Owned(outbound) => match *outbound {
            Outbound::Sessions { sessions } => assert!(!sessions[0].closing),
            _ => panic!("expected translated Sessions list"),
        },
        _ => panic!("expected translated Sessions list"),
    }

    // Deletion of member-a's session reaches member-a and owner, never member-b.
    assert!(hub.delete_session("s-a"));
    let removed = rx.try_recv().expect("remove frame");
    assert!(matches!(
        resolve_fanout_delivery(&hub, &member, true, bootstrap_seq, &mut no_legacy, &removed),
        FanoutDelivery::Shared(frame) if matches!(frame.outbound(), Outbound::SessionRemoved { session_id } if session_id == "s-a")
    ));
    assert!(matches!(
        resolve_fanout_delivery(&hub, &owner, true, bootstrap_seq, &mut no_legacy, &removed),
        FanoutDelivery::Shared(frame) if matches!(frame.outbound(), Outbound::SessionRemoved { session_id } if session_id == "s-a")
    ));
    assert!(matches!(
        resolve_fanout_delivery(
            &hub,
            &other_member,
            true,
            bootstrap_seq,
            &mut no_legacy,
            &removed
        ),
        FanoutDelivery::Skip
    ));
    assert!(matches!(
        resolve_fanout_delivery(
            &hub,
            &other_member,
            false,
            bootstrap_seq,
            &mut other_legacy,
            &removed
        ),
        FanoutDelivery::Skip
    ));
    match resolve_fanout_delivery(
        &hub,
        &member,
        false,
        bootstrap_seq,
        &mut member_legacy,
        &removed,
    ) {
        FanoutDelivery::Owned(outbound) => match *outbound {
            Outbound::Sessions { sessions } => assert!(sessions.is_empty()),
            _ => panic!("legacy client must receive translated empty Sessions list after delete"),
        },
        _ => panic!("legacy client must receive translated empty Sessions list after delete"),
    }

    // Web renames and reorders travel as SyncPatch frames; legacy session lists
    // fold those patches in version order so a subsequent delta for session B
    // never synthesizes session A with its stale title or order.
    register("s-1", Some("member-a"));
    let _ = rx.try_recv().expect("create s-1");
    register("s-2", Some("member-a"));
    let _ = rx.try_recv().expect("create s-2");
    hub.sync_apply(
        "title",
        "m-rename-pre".to_owned(),
        "rename",
        &serde_json::json!({ "session_id": "s-1", "title": "Renamed Before Connect" }),
    )
    .expect("pre-bootstrap rename");
    let stale_title_patch = rx.try_recv().expect("pre-bootstrap title patch");
    hub.sync_apply(
        "title",
        "m-rename-latest".to_owned(),
        "rename",
        &serde_json::json!({ "session_id": "s-1", "title": "Latest Before Connect" }),
    )
    .expect("latest pre-bootstrap rename");
    let _ = rx.try_recv().expect("latest title patch");

    let (sync_bootstrap, sync_bootstrap_seq) =
        connect_bootstrap_with_seq(&hub, true, &member, None);
    let mut synced_legacy = LegacySessionTracker::from_bootstrap(&sync_bootstrap, false);

    // A stale pre-bootstrap title SyncPatch must not overwrite the bootstrap title.
    let _ = resolve_fanout_delivery(
        &hub,
        &member,
        false,
        sync_bootstrap_seq,
        &mut synced_legacy,
        &stale_title_patch,
    );

    hub.sync_apply(
        "title",
        "m-rename-live".to_owned(),
        "rename",
        &serde_json::json!({ "session_id": "s-1", "title": "Renamed Live" }),
    )
    .expect("live rename");
    let live_title_patch = rx.try_recv().expect("live title patch");
    let _ = resolve_fanout_delivery(
        &hub,
        &member,
        false,
        sync_bootstrap_seq,
        &mut synced_legacy,
        &live_title_patch,
    );

    hub.sync_apply(
        "order",
        "m-reorder-live".to_owned(),
        "reorder",
        &serde_json::json!({ "order": ["s-2", "s-1"] }),
    )
    .expect("live reorder");
    let live_order_patch = rx.try_recv().expect("live order patch");
    let _ = resolve_fanout_delivery(
        &hub,
        &member,
        false,
        sync_bootstrap_seq,
        &mut synced_legacy,
        &live_order_patch,
    );

    // Mutate only s-2; the synthesized legacy Sessions list must preserve s-1's
    // live title and the reordered [s-2, s-1] sequence.
    hub.set_background_tasks("s-2", 1);
    let s2_delta = rx.try_recv().expect("s-2 delta");
    match resolve_fanout_delivery(
        &hub,
        &member,
        false,
        sync_bootstrap_seq,
        &mut synced_legacy,
        &s2_delta,
    ) {
        FanoutDelivery::Owned(outbound) => match *outbound {
            Outbound::Sessions { sessions } => {
                assert_eq!(
                    sessions
                        .iter()
                        .map(|s| (s.id.as_str(), s.title.as_str()))
                        .collect::<Vec<_>>(),
                    vec![("s-2", "s-2"), ("s-1", "Renamed Live")]
                );
            }
            _ => panic!("expected translated Sessions list"),
        },
        _ => panic!("expected translated Sessions list"),
    }
}
