use super::*;
use crate::plugin_operation::fixture;
use crate::store::{ProductApiToken, ProductUser, ProductUserSession};

struct Harness {
    _root: tempfile::TempDir,
    store: Store,
    hub: Hub,
    devices: crate::client_auth::DeviceAccessSessions,
    authentication: crate::auth_plugins::ProductAuthentication,
    user: ProductUser,
}

impl Harness {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let user = ProductUser {
            id: "c".repeat(32),
            username: "operator".into(),
            password_algo: "argon2id".into(),
            password_hash: "unused-hermetic-fixture".into(),
            created_at_ms: auth_now_ms(),
            updated_at_ms: auth_now_ms(),
            disabled_at_ms: None,
        };
        store.insert_user(&user).await.unwrap();
        let h = Self {
            _root: root,
            store,
            hub: Hub::new(),
            user,
            devices: crate::client_auth::DeviceAccessSessions::default(),
            authentication: crate::auth_plugins::ProductAuthentication::test_default(None),
        };
        h.role(AdminRole::Operator);
        h
    }

    fn role(&self, role: AdminRole) {
        self.hub.set_setting(
            crate::admin::PERMISSIONS_SETTING.into(),
            serde_json::json!({
                "default_role": role, "grants": [],
            }),
        );
    }

    fn context(&self) -> ProductRequestAuth<'_> {
        ProductRequestAuth {
            product_auth_enabled: true,
            store: Some(&self.store),
            hub: &self.hub,
            device_access: &self.devices,
            product_authentication: &self.authentication,
        }
    }

    async fn cookie(&self) -> (HeaderMap, AuthenticatedProductRequest) {
        let now = auth_now_ms();
        let session = ProductUserSession {
            token_hash: hex_sha256(b"hermetic-cookie-secret"),
            session_id: "session-approval".into(),
            user_id: self.user.id.clone(),
            client_kind: "browser".into(),
            principal_class: "human".into(),
            created_at_ms: now,
            expires_at_ms: now + 300_000,
            last_seen_at_ms: now,
            user_agent: None,
            passkey_verified_at_ms: None,
            primary_authenticated_at_ms: now,
            primary_auth_method: Some("password".into()),
            revoked_at_ms: None,
            revoke_reason: None,
            auth_provider_id: None,
            auth_issuer: None,
            auth_subject: None,
            auth_sid: None,
            id_token_ciphertext: None,
        };
        self.store.insert_user_session(&session).await.unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "cowboy_user=hermetic-cookie-secret".parse().unwrap(),
        );
        (
            headers,
            AuthenticatedProductRequest {
                principal: product_principal(&self.hub, &self.user),
                cookie_session: Some(session),
                device_identity: None,
            },
        )
    }

    fn bind(
        &self,
        headers: &HeaderMap,
        verified: &AuthenticatedProductRequest,
    ) -> (UninstallAuthority, UninstallIntent) {
        let approval =
            OperatorApproval::capture(self.context(), "service-test", Some(verified), headers)
                .unwrap();
        let mut intent = fixture("authority");
        intent.actor = approval.actor().clone();
        (approval.bind(&intent).unwrap(), intent)
    }
}

#[tokio::test]
async fn cookie_revocation_role_changes_and_user_disable_are_rechecked_and_sticky() {
    for change in ["logout", "role", "disable"] {
        let h = Harness::new().await;
        let (headers, verified) = h.cookie().await;
        let (authority, intent) = h.bind(&headers, &verified);
        assert!(authority.check(h.context(), "service-test", &intent).await);
        match change {
            "logout" => {
                h.store
                    .revoke_user_session_for_user(
                        &h.user.id,
                        "session-approval",
                        "logout",
                        auth_now_ms(),
                    )
                    .await
                    .unwrap();
            }
            "role" => h.role(AdminRole::Viewer),
            _ => h
                .store
                .set_user_disabled_at(&h.user.id, Some(auth_now_ms()))
                .await
                .unwrap(),
        }
        assert!(
            !authority.check(h.context(), "service-test", &intent).await,
            "{change}"
        );
        h.role(AdminRole::Operator);
        h.store
            .set_user_disabled_at(&h.user.id, None)
            .await
            .unwrap();
        assert!(
            !authority.check(h.context(), "service-test", &intent).await,
            "a repaired account cannot renew approval"
        );
    }
}

#[tokio::test]
async fn login_freshness_and_original_budget_bound_the_continuation() {
    let mut h = Harness::new().await;
    let (headers, verified) = h.cookie().await;
    let (authority, intent) = h.bind(&headers, &verified);
    assert!(authority.check(h.context(), "service-test", &intent).await);
    h.authentication.session.primary_max_age_ms = 0;
    assert!(!authority.check(h.context(), "service-test", &intent).await);
    h.authentication = crate::auth_plugins::ProductAuthentication::test_default(None);
    let (authority, intent) = h.bind(&headers, &verified);
    authority.budget.expire_for_test();
    assert!(!authority.check(h.context(), "service-test", &intent).await);
}

#[tokio::test]
async fn personal_token_revocation_is_not_masked_by_a_still_valid_cookie() {
    let h = Harness::new().await;
    let (mut headers, _) = h.cookie().await;
    let token = ProductApiToken {
        id: "a".repeat(32),
        user_id: h.user.id.clone(),
        name: "fixture".into(),
        token_prefix: "cow_test".into(),
        token_hash: hex_sha256(b"cow_test-secret"),
        created_at_ms: auth_now_ms(),
        expires_at_ms: Some(auth_now_ms() + 300_000),
        last_used_at_ms: None,
        revoked_at_ms: None,
    };
    h.store.insert_user_api_token(&token).await.unwrap();
    headers.insert(
        header::AUTHORIZATION,
        "Bearer cow_test-secret".parse().unwrap(),
    );
    let verified = resolve_product_api_request_principal(
        h.context(),
        &Method::POST,
        &"/api/machines/hawk/plugins/victoria/uninstall"
            .parse()
            .unwrap(),
        &headers,
    )
    .await
    .unwrap()
    .unwrap();
    let (authority, intent) = h.bind(&headers, &verified);
    assert!(authority.check(h.context(), "service-test", &intent).await);
    h.store
        .revoke_user_api_token_for_user(&h.user.id, &token.id)
        .await
        .unwrap();
    assert!(!authority.check(h.context(), "service-test", &intent).await);
}

#[tokio::test]
async fn device_continuation_rechecks_the_same_token_without_consuming_proof_twice() {
    let h = Harness::new().await;
    let key = crate::client_auth::new_signing_key().unwrap();
    let public = crate::client_auth::public_key_to_base64(&key);
    let device = "d".repeat(32);
    let (token, _) = h
        .devices
        .issue(&device, &h.user.id, &public, auth_now_ms())
        .unwrap();
    let path = "/api/machines/hawk/plugins/victoria/uninstall";
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    for (name, value) in
        crate::client_auth::signed_proof_headers(&key, &device, &token, "POST", path, auth_now_ms())
            .unwrap()
    {
        headers.insert(
            header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    let verified = resolve_product_api_request_principal(
        h.context(),
        &Method::POST,
        &path.parse().unwrap(),
        &headers,
    )
    .await
    .unwrap()
    .unwrap();
    let (authority, intent) = h.bind(&headers, &verified);
    for _ in 0..2 {
        assert!(authority.check(h.context(), "service-test", &intent).await);
    }
    assert!(
        h.devices
            .authenticate(&headers, &Method::POST, path, auth_now_ms())
            .is_err(),
        "proof still rejects HTTP replay"
    );
    h.devices.revoke_access_token(&token);
    assert!(!authority.check(h.context(), "service-test", &intent).await);
}

#[tokio::test]
async fn authority_binds_the_complete_intent_and_never_falls_back_to_auth_off() {
    let h = Harness::new().await;
    let (headers, verified) = h.cookie().await;
    for field in [
        "operation",
        "machine",
        "digest",
        "sessions",
        "service",
        "auth-off",
    ] {
        let (authority, mut intent) = h.bind(&headers, &verified);
        let mut context = h.context();
        let mut service = "service-test";
        match field {
            "operation" => intent.operation_id.push('x'),
            "machine" => intent.machine_id = "falcon".into(),
            "digest" => intent.generation_digest = format!("sha256:{}", "f".repeat(64)),
            "sessions" => intent.session_ids.push("another-session".into()),
            "service" => service = "other-service",
            _ => context.product_auth_enabled = false,
        }
        assert!(!authority.check(context, service, &intent).await, "{field}");
    }
    assert!(OperatorApproval::capture(h.context(), "service-test", None, &headers).is_err());
    let mut viewer = verified.clone();
    viewer.principal.role = AdminRole::Viewer;
    assert!(
        OperatorApproval::capture(h.context(), "service-test", Some(&viewer), &headers).is_err()
    );
}

#[tokio::test]
async fn admin_continuation_keeps_credential_precedence_and_current_role() {
    for change in ["logout", "role", "expiry"] {
        let h = Harness::new().await;
        let (mut headers, verified) = h.cookie().await;
        headers.insert(
            header::COOKIE,
            "cowboy_admin=hermetic-admin-secret; cowboy_user=hermetic-cookie-secret"
                .parse()
                .unwrap(),
        );
        let mut identities = crate::admin::AdminIdentities {
            accounts: vec![crate::admin::AdminAccount {
                account: "admin".into(),
                role: AdminRole::Operator,
                password_salt: String::new(),
                password_hash: "unused-fixture".into(),
                created_at_ms: auth_now_ms(),
                passkey_reauth_enabled: false,
                last_step_up_at_ms: None,
            }],
            sessions: vec![crate::admin::AdminSessionRecord {
                token_hash: hex_sha256(b"hermetic-admin-secret"),
                account: "admin".into(),
                expires_at_ms: auth_now_ms() + 300_000,
            }],
            setup_token_hash: None,
        };
        h.hub.set_setting(
            crate::admin::ADMIN_IDENTITIES_SETTING.into(),
            serde_json::to_value(&identities).unwrap(),
        );
        let (authority, intent) = h.bind(&headers, &verified);
        assert!(matches!(intent.actor, Actor::Admin { .. }));
        assert!(authority.check(h.context(), "service-test", &intent).await);
        match change {
            "logout" => identities.sessions.clear(),
            "role" => identities.accounts[0].role = AdminRole::Viewer,
            _ => identities.sessions[0].expires_at_ms = auth_now_ms(),
        }
        h.hub.set_setting(
            crate::admin::ADMIN_IDENTITIES_SETTING.into(),
            serde_json::to_value(&identities).unwrap(),
        );
        assert!(
            !authority.check(h.context(), "service-test", &intent).await,
            "{change}"
        );
    }
}

#[tokio::test]
async fn explicit_local_mode_is_not_a_reusable_authentication_bypass() {
    let h = Harness::new().await;
    let verified = AuthenticatedProductRequest {
        principal: crate::product_auth::local_product_principal(),
        cookie_session: None,
        device_identity: None,
    };
    let local_context = || ProductRequestAuth {
        product_auth_enabled: false,
        ..h.context()
    };
    let approval = OperatorApproval::capture(
        local_context(),
        "service-test",
        Some(&verified),
        &HeaderMap::new(),
    )
    .unwrap();
    let mut intent = fixture("local");
    intent.actor = approval.actor().clone();
    let authority = approval.bind(&intent).unwrap();
    assert!(
        authority
            .check(local_context(), "service-test", &intent)
            .await
    );
    assert!(!authority.check(h.context(), "service-test", &intent).await);
    assert!(
        !authority
            .check(local_context(), "service-test", &intent)
            .await
    );
}

#[tokio::test]
async fn scheduling_and_validation_cannot_renew_the_confirmation_budget() {
    let h = Harness::new().await;
    let (headers, verified) = h.cookie().await;
    let mut approval =
        OperatorApproval::capture(h.context(), "service-test", Some(&verified), &headers).unwrap();
    approval.received = TimeSample::for_test(
        std::time::Instant::now() - Duration::from_mins(6),
        auth_now_ms(),
    );
    let mut intent = fixture("queued-approval");
    intent.actor = approval.actor().clone();
    intent.expires_at_ms += 300_000;
    let authority = approval.bind(&intent).unwrap();
    assert!(
        !authority.check(h.context(), "service-test", &intent).await,
        "even a future wall deadline cannot extend the captured monotonic budget"
    );
}
