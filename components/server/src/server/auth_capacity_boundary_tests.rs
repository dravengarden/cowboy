use super::*;

fn principal(user_id: &str) -> ProductPrincipal {
    ProductPrincipal {
        user_id: user_id.to_owned(),
        username: user_id.to_owned(),
        role: crate::admin::AdminRole::Operator,
    }
}

fn cookie_request(user_id: &str) -> AuthenticatedProductRequest {
    AuthenticatedProductRequest {
        permissions: None,
        principal: principal(user_id),
        cookie_session: Some(crate::store::ProductUserSession {
            token_hash: "aa".repeat(32),
            session_id: "browser-session-0000000000000001".to_owned(),
            user_id: user_id.to_owned(),
            client_kind: "browser".to_owned(),
            principal_class: "human".to_owned(),
            created_at_ms: 1,
            expires_at_ms: 2,
            last_seen_at_ms: 1,
            user_agent: None,
            passkey_verified_at_ms: None,
            primary_authenticated_at_ms: 1,
            primary_auth_method: Some(crate::auth_plugins::PASSWORD_LOGIN_METHOD.to_owned()),
            revoked_at_ms: None,
            revoke_reason: None,
            auth_provider_id: None,
            auth_issuer: None,
            auth_subject: None,
            auth_sid: None,
            id_token_ciphertext: None,
        }),
        device_identity: None,
    }
}

fn device_request(
    user_id: &str,
    principal_class: &str,
    client_kind: &str,
    scopes: &[&str],
) -> AuthenticatedProductRequest {
    AuthenticatedProductRequest {
        permissions: None,
        principal: principal(user_id),
        cookie_session: None,
        device_identity: Some(crate::client_auth::DeviceAccessIdentity {
            device_id: "device-000000000000000000000001".to_owned(),
            user_id: user_id.to_owned(),
            principal_class: principal_class.to_owned(),
            client_kind: client_kind.to_owned(),
            scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        }),
    }
}

fn websocket_query(kind: &str) -> WebSocketQuery {
    WebSocketQuery {
        bootstrap: None,
        client_id: Some("stable-client-0000000000000001".to_owned()),
        client_kind: Some(kind.to_owned()),
        dataset: None,
    }
}

#[test]
fn websocket_client_kind_is_bounded_by_authenticated_credential_class() {
    let cookie = cookie_request("user-a");
    for kind in ["browser", "native_shell"] {
        assert!(websocket_capacity_identity(true, &cookie, &websocket_query(kind)).is_ok());
    }
    for kind in ["cli", "acp", "automation"] {
        assert_eq!(
            websocket_capacity_identity(true, &cookie, &websocket_query(kind)).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }

    let device = device_request("user-a", "human", "cli", &[]);
    for kind in ["cli", "acp"] {
        assert!(websocket_capacity_identity(true, &device, &websocket_query(kind)).is_ok());
    }
    for kind in ["browser", "native_shell", "automation"] {
        assert_eq!(
            websocket_capacity_identity(true, &device, &websocket_query(kind)).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }

    let automation = device_request("user-a", "automation", "automation", &["websocket"]);
    assert!(websocket_capacity_identity(true, &automation, &websocket_query("automation")).is_ok());
    for kind in ["browser", "native_shell", "cli", "acp"] {
        assert_eq!(
            websocket_capacity_identity(true, &automation, &websocket_query(kind)).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[test]
fn websocket_client_labels_are_namespaced_by_authenticated_user() {
    let first =
        websocket_capacity_identity(true, &cookie_request("user-a"), &websocket_query("browser"))
            .unwrap();
    let second =
        websocket_capacity_identity(true, &cookie_request("user-b"), &websocket_query("browser"))
            .unwrap();
    assert_ne!(first.client_id, second.client_id);
    assert_eq!(first.principal_class, "human");
    assert_eq!(first.user_id.as_deref(), Some("user-a"));
}

#[test]
fn automation_credentials_are_confined_to_declared_route_scopes() {
    let websocket = device_request("user-a", "automation", "automation", &["websocket"]);
    let websocket = websocket.device_identity.as_ref().unwrap();
    assert!(automation_route_allowed(websocket, &Method::GET, "/ws"));
    assert!(!automation_route_allowed(
        websocket,
        &Method::GET,
        "/api/auth/me"
    ));

    let reader = device_request("user-a", "automation", "automation", &["api:read"]);
    let reader = reader.device_identity.as_ref().unwrap();
    assert!(automation_route_allowed(
        reader,
        &Method::GET,
        "/api/auth/status"
    ));
    assert!(automation_route_allowed(
        reader,
        &Method::GET,
        "/api/sessions/example/events"
    ));
    assert!(!automation_route_allowed(
        reader,
        &Method::GET,
        "/api/admin/accounts"
    ));
    assert!(!automation_route_allowed(
        reader,
        &Method::POST,
        "/api/auth/logout"
    ));

    let writer = device_request("user-a", "automation", "automation", &["sessions:write"]);
    let writer = writer.device_identity.as_ref().unwrap();
    assert!(automation_route_allowed(
        writer,
        &Method::POST,
        "/api/sessions"
    ));
    assert!(automation_route_allowed(
        writer,
        &Method::DELETE,
        "/api/sessions/example"
    ));
    assert!(!automation_route_allowed(
        writer,
        &Method::POST,
        "/api/auth/automation/credentials"
    ));
    for human_only in [
        "/api/sessions/example/widgets/w/interactions",
        "/api/sessions/example/widgets/w/apps/dash/messages",
    ] {
        assert!(
            !automation_route_allowed(writer, &Method::POST, human_only),
            "{human_only} speaks for the person"
        );
    }

    let human = device_request("user-a", "human", "cli", &[]);
    assert!(automation_route_allowed(
        human.device_identity.as_ref().unwrap(),
        &Method::POST,
        "/api/auth/logout"
    ));
}
