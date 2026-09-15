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
