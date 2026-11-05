use super::*;

fn registration(id: &str) -> SessionRegistration {
    SessionRegistration {
        id: id.to_owned(),
        provider: "codex".to_owned(),
        provider_version: String::new(),
        provider_generation_digest: String::new(),
        provider_account_id: crate::provider_account::ProviderAccountId::default(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: "local".to_owned(),
        workspace_id: None,
        workspace_name: None,
        workspace_source_path: None,
        execution_binding: None,
        cwd: "/tmp".to_owned(),
        title: "owner stamp".to_owned(),
        origin: SessionOrigin::Web,
        system: false,
        owner_user_id: None,
        owner_username: None,
    }
}

#[test]
fn new_session_stamps_optional_owner_and_broadcasts_it() {
    let hub = Hub::new();
    let mut registration = registration("sess-owned");
    registration.owner_user_id = Some("0123456789abcdef0123456789abcdef".to_owned());
    registration.owner_username = Some("draven".to_owned());
    hub.create_session(registration);

    let meta = hub
        .session_list()
        .into_iter()
        .find(|session| session.id == "sess-owned")
        .expect("created session");
    assert_eq!(
        meta.owner_user_id.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
    assert_eq!(meta.owner_username.as_deref(), Some("draven"));
}

#[test]
fn unauthenticated_create_leaves_owner_null() {
    let hub = Hub::new();
    hub.create_session(registration("sess-shared"));

    let meta = hub
        .session_list()
        .into_iter()
        .find(|session| session.id == "sess-shared")
        .expect("created session");
    assert!(meta.owner_user_id.is_none());
    assert!(meta.owner_username.is_none());
    assert!(!hub.owned_by_product_user("sess-shared", "anyone"));
    assert_eq!(hub.session_owner_user_id("sess-shared"), None);
}
