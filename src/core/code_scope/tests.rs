use super::*;
use crate::core::{SessionOrigin, Status};

fn create(hub: &Hub, id: &str) {
    hub.create_local_session(
        id.into(),
        "codex".into(),
        "/work/a".into(),
        "title".into(),
        SessionOrigin::default(),
        false,
    );
}

#[test]
fn observation_tracks_workspace_lifetime_not_status_or_title() {
    let hub = Hub::new();
    create(&hub, "session");
    let scope = hub.session_code_scope("session").unwrap();
    hub.rename_session("session", "renamed".into());
    hub.set_status("session", Status::Running, None);
    hub.update_session_cwd("session", "/work/a".into()).unwrap();
    assert!(hub.code_scope_is_current(&scope));
    hub.update_session_cwd("session", "/work/b".into()).unwrap();
    assert!(!hub.code_scope_is_current(&scope));
    hub.update_session_cwd("session", "/work/a".into()).unwrap();
    assert!(!hub.code_scope_is_current(&scope));
    assert!(hub.code_scope_is_current(&hub.session_code_scope("session").unwrap()));
}

#[test]
fn removal_recreation_and_other_hubs_never_adopt_an_observation() {
    let hub = Hub::new();
    create(&hub, "session");
    let scope = hub.session_code_scope("session").unwrap();
    let other = Hub::new();
    create(&other, "session");
    assert!(!other.code_scope_is_current(&scope));
    assert!(hub.delete_session("session"));
    assert!(!hub.code_scope_is_current(&scope));
    create(&hub, "session");
    assert!(!hub.code_scope_is_current(&scope));
}

#[test]
fn independent_sessions_on_the_same_path_keep_independent_lifetimes() {
    let hub = Hub::new();
    create(&hub, "a");
    create(&hub, "b");
    let a = hub.session_code_scope("a").unwrap();
    let b = hub.session_code_scope("b").unwrap();
    assert_ne!(a, b);
    hub.update_session_cwd("a", "/work/b".into()).unwrap();
    assert!(!hub.code_scope_is_current(&a));
    assert!(hub.code_scope_is_current(&b));
}

#[test]
fn machine_workspace_and_principal_are_checked_even_with_same_incarnation() {
    let hub = Hub::new();
    create(&hub, "session");
    let scope = hub.session_code_scope("session").unwrap();
    // Exercise accidental future metadata mutation without the normal lifetime
    // replacement. Exact tuple checks must still reject the original scope.
    for changed in ["machine", "workspace", "principal"] {
        {
            let mut sessions = hub.inner.sessions.lock();
            let meta = &mut sessions.get_mut("session").unwrap().meta;
            match changed {
                "machine" => meta.machine_id = "other-machine".into(),
                "workspace" => meta.workspace_id = Some("other-workspace".into()),
                "principal" => meta.owner_user_id = Some("other-user".into()),
                _ => unreachable!(),
            }
        }
        assert!(!hub.code_scope_is_current(&scope));
        let mut sessions = hub.inner.sessions.lock();
        let meta = &mut sessions.get_mut("session").unwrap().meta;
        meta.machine_id = "local".into();
        meta.workspace_id = None;
        meta.owner_user_id = None;
    }
}

fn bind(hub: &Hub) {
    create(hub, "remote-session");
    let mut sessions = hub.inner.sessions.lock();
    let meta = &mut sessions.get_mut("remote-session").unwrap().meta;
    meta.machine_id = "ovh".into();
    meta.cwd = "/runtime/session".into();
    meta.execution_binding = Some(crate::execution_environment::fixture());
}

#[test]
fn execution_environment_routes_code_to_target_without_moving_runtime() {
    let hub = Hub::new();
    bind(&hub);
    let scope = hub.session_code_scope("remote-session").unwrap();
    assert_eq!(scope.machine_id(), "hawk");
    assert_eq!(scope.cwd(), "/tasks/cowboy");
    assert_eq!(scope.workspace_id.as_deref(), Some("cowboy"));
    let meta = hub.session_info("remote-session").unwrap().meta;
    assert_eq!(meta.machine_id, "ovh");
    assert_eq!(meta.cwd, "/runtime/session");
    assert!(meta.require_runtime_launch().is_err());
    assert!(
        hub.update_session_cwd("remote-session", "/different-runtime".into())
            .is_err()
    );
    assert!(hub.code_scope_is_current(&scope));

    for pointer in [
        "/revision",
        "/environment/incarnation",
        "/environment/executor_digest",
        "/workspace/worktree_id",
    ] {
        let mut changed = crate::execution_environment::fixture().record().clone();
        *changed.pointer_mut(pointer).unwrap() = match pointer {
            "/revision" => serde_json::json!(2),
            "/environment/executor_digest" => {
                serde_json::json!(format!("sha256:{}", "cd".repeat(32)))
            }
            _ => serde_json::json!("another-identity"),
        };
        hub.inner
            .sessions
            .lock()
            .get_mut("remote-session")
            .unwrap()
            .meta
            .execution_binding = Some(crate::execution_environment::ExecutionBinding::from_record(
            changed,
        ));
        assert!(!hub.code_scope_is_current(&scope), "{pointer}");
        assert!(hub.session_code_scope("remote-session").is_some());
    }
}

#[test]
fn execution_environment_unknown_or_wrong_runtime_never_resolves_as_local() {
    let hub = Hub::new();
    bind(&hub);
    for value in [
        serde_json::Value::Null,
        serde_json::json!({"schema": 2}),
        serde_json::json!(false),
    ] {
        hub.inner
            .sessions
            .lock()
            .get_mut("remote-session")
            .unwrap()
            .meta
            .execution_binding = Some(crate::execution_environment::ExecutionBinding::from_record(
            value,
        ));
        let meta = hub.session_info("remote-session").unwrap().meta;
        let restored: crate::core::SessionMeta =
            serde_json::from_value(serde_json::to_value(&meta).unwrap()).unwrap();
        assert_eq!(restored.execution_binding, meta.execution_binding);
        assert!(restored.require_runtime_launch().is_err());
        assert!(hub.session_code_scope("remote-session").is_none());
    }
    let mut sessions = hub.inner.sessions.lock();
    let meta = &mut sessions.get_mut("remote-session").unwrap().meta;
    meta.execution_binding = Some(crate::execution_environment::fixture());
    meta.machine_id = "falcon".into();
    drop(sessions);
    assert!(hub.session_code_scope("remote-session").is_none());
}

#[test]
fn a_changed_machine_lineage_retires_observations_but_repeats_do_not() {
    let hub = Hub::new();
    create(&hub, "session");
    let process_local = hub.session_code_scope("session").unwrap();
    // The first lineage a Machine reports changes the scope once.
    assert!(hub.set_machine_lineage("session", Some("a".repeat(32).as_str())));
    assert!(!hub.code_scope_is_current(&process_local));
    let first = hub.session_code_scope("session").unwrap();
    assert!(first.string_bytes() > process_local.string_bytes());
    // Every later snapshot of the same lineage keeps it current.
    assert!(!hub.set_machine_lineage("session", Some("a".repeat(32).as_str())));
    assert!(hub.code_scope_is_current(&first));
    // A reset (new lineage) retires it; the old value can never come back.
    assert!(hub.set_machine_lineage("session", Some("b".repeat(32).as_str())));
    assert!(!hub.code_scope_is_current(&first));
    let second = hub.session_code_scope("session").unwrap();
    assert!(hub.set_machine_lineage("session", Some("a".repeat(32).as_str())));
    assert!(!hub.code_scope_is_current(&second));
    assert!(!hub.code_scope_is_current(&first));
    // A Machine that stops reporting one is a change too, never silently kept.
    let third = hub.session_code_scope("session").unwrap();
    assert!(hub.set_machine_lineage("session", None));
    assert!(!hub.code_scope_is_current(&third));
    // An unknown Session records nothing.
    assert!(!hub.set_machine_lineage("unknown", Some("c".repeat(32).as_str())));
    // Lineage never substitutes for the other identity parts.
    let other = Hub::new();
    create(&other, "session");
    other.set_machine_lineage("session", Some("a".repeat(32).as_str()));
    assert!(!other.code_scope_is_current(&first));
}
