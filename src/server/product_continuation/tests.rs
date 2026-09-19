use super::*;
use crate::admin::AdminRole;
use crate::store::{ProductApiToken, ProductUser, ProductUserSession};

pub(in crate::server) struct Harness {
    _root: tempfile::TempDir,
    pub store: Store,
    pub hub: Hub,
    pub user: ProductUser,
    pub devices: crate::client_auth::DeviceAccessSessions,
    pub authentication: crate::auth_plugins::ProductAuthentication,
}

impl Harness {
    pub async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let user = ProductUser {
            id: "c".repeat(32),
            username: "reader".into(),
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
            user,
            hub: Hub::new(),
            devices: crate::client_auth::DeviceAccessSessions::default(),
            authentication: crate::auth_plugins::ProductAuthentication::test_default(None),
        };
        h.role(AdminRole::Viewer);
        h
    }

    pub fn role(&self, role: AdminRole) {
        self.hub.set_setting(
            crate::admin::PERMISSIONS_SETTING.into(),
            serde_json::json!({"default_role":role,"grants":[]}),
        );
    }

    pub fn auth(&self) -> ProductRequestAuth<'_> {
        ProductRequestAuth {
            product_auth_enabled: true,
            store: Some(&self.store),
            hub: &self.hub,
            device_access: &self.devices,
            product_authentication: &self.authentication,
        }
    }

    pub async fn cookie(&self) -> HeaderMap {
        let now = auth_now_ms();
        self.store
            .insert_user_session(&ProductUserSession {
                token_hash: hex_sha256(b"hermetic-read-cookie"),
                session_id: "reader-session-credential".into(),
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
            })
            .await
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "cowboy_user=hermetic-read-cookie".parse().unwrap(),
        );
        headers
    }

    pub async fn token(&self) -> HeaderMap {
        self.store
            .insert_user_api_token(&ProductApiToken {
                id: "a".repeat(32),
                user_id: self.user.id.clone(),
                name: "reader".into(),
                token_prefix: "cow_test".into(),
                token_hash: hex_sha256(b"cow_test-read-continuation"),
                created_at_ms: auth_now_ms(),
                expires_at_ms: Some(auth_now_ms() + 300_000),
                last_used_at_ms: None,
                revoked_at_ms: None,
            })
            .await
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer cow_test-read-continuation".parse().unwrap(),
        );
        headers
    }

    pub async fn capture(&self, headers: &HeaderMap) -> ProductContinuation {
        let verified = resolve_product_api_request_principal(
            self.auth(),
            &Method::GET,
            &"/api/code/sessions/fixture/file?path=read.txt"
                .parse()
                .unwrap(),
            headers,
        )
        .await
        .unwrap()
        .unwrap();
        ProductContinuation::capture(self.auth(), Some(&verified), headers).unwrap()
    }

    pub async fn revoke(&self, kind: &str) {
        match kind {
            "cookie" => {
                self.store
                    .revoke_user_session_for_user(
                        &self.user.id,
                        "reader-session-credential",
                        "logout",
                        auth_now_ms(),
                    )
                    .await
                    .unwrap();
            }
            "token" => {
                self.store
                    .revoke_user_api_token_for_user(&self.user.id, &"a".repeat(32))
                    .await
                    .unwrap();
            }
            "disabled" => self
                .store
                .set_user_disabled_at(&self.user.id, Some(auth_now_ms()))
                .await
                .unwrap(),
            "visibility" => self.role(AdminRole::Viewer),
            "permission_aba" => {
                let role = permission_policy(&self.hub).role_for(&self.user.username);
                self.role(if role == AdminRole::Viewer {
                    AdminRole::Owner
                } else {
                    AdminRole::Viewer
                });
                self.role(role);
            }
            _ => panic!("unknown fixture change"),
        }
    }
}

#[tokio::test]
async fn permission_aba_cannot_revive_an_original_product_continuation() {
    let h = Harness::new().await;
    h.role(AdminRole::Owner);
    let headers = h.cookie().await;
    let captured = h.capture(&headers).await;
    h.role(AdminRole::Viewer);
    h.role(AdminRole::Owner);
    // No intermediate poll: the core's actual permission mutation must end
    // the original lifetime, not just compare equal before/after role values.
    assert!(captured.current(h.auth()).await.is_none());
    assert!(h.capture(&headers).await.current(h.auth()).await.is_some());
}

#[tokio::test]
async fn queued_authentication_cannot_capture_a_replacement_permission_lifetime() {
    for token in [false, true] {
        let h = Harness::new().await;
        h.role(AdminRole::Operator);
        let headers = if token {
            h.token().await
        } else {
            h.cookie().await
        };
        let verified = resolve_product_api_request_principal(
            h.auth(),
            &Method::GET,
            &"/api/plugins".parse().unwrap(),
            &headers,
        )
        .await
        .unwrap()
        .unwrap();
        h.revoke("permission_aba").await;
        assert!(ProductContinuation::capture(h.auth(), Some(&verified), &headers).is_err());
        assert!(h.capture(&headers).await.current(h.auth()).await.is_some());
    }
}

#[tokio::test]
async fn permission_observations_cannot_be_substituted_from_another_core() {
    let h = Harness::new().await;
    let headers = h.cookie().await;
    let captured = h.capture(&headers).await;
    let other = Hub::new();
    assert!(
        captured
            .current(ProductRequestAuth {
                hub: &other,
                ..h.auth()
            })
            .await
            .is_none()
    );
    assert!(captured.current(h.auth()).await.is_some());
}

#[tokio::test]
async fn original_cookie_and_token_never_fall_back_or_change_user() {
    for kind in ["cookie", "token", "disabled"] {
        let h = Harness::new().await;
        let mut headers = h.cookie().await;
        if kind == "token" {
            headers.extend(h.token().await);
        }
        let captured = h.capture(&headers).await;
        assert_eq!(captured.current(h.auth()).await.unwrap().user_id, h.user.id);
        h.revoke(kind).await;
        assert!(captured.current(h.auth()).await.is_none(), "{kind}");
        assert!(
            captured
                .current(ProductRequestAuth {
                    product_auth_enabled: false,
                    ..h.auth()
                })
                .await
                .is_none()
        );
        assert!(ProductContinuation::capture(h.auth(), None, &headers).is_err());
    }
}

#[tokio::test]
async fn current_roles_preserve_viewer_reads_without_granting_mutations() {
    let h = Harness::new().await;
    let headers = h.cookie().await;
    let captured = h.capture(&headers).await;
    let viewer = captured.current(h.auth()).await.unwrap();
    assert!(viewer.can_see(Some(&h.user.id)) && viewer.can_see(None));
    assert!(!viewer.can_see(Some("other-user")) && !viewer.can_mutate(Some(&h.user.id)));
    h.role(AdminRole::Owner);
    assert!(captured.current(h.auth()).await.is_none());
    let owner = h.capture(&headers).await;
    assert!(
        owner
            .current(h.auth())
            .await
            .unwrap()
            .can_see(Some("other-user"))
    );
    h.role(AdminRole::Viewer);
    assert!(owner.current(h.auth()).await.is_none());
    assert!(captured.current(h.auth()).await.is_none());
    let fresh = h.capture(&headers).await;
    assert!(
        !fresh
            .current(h.auth())
            .await
            .unwrap()
            .can_see(Some("other-user"))
    );
}

#[tokio::test]
async fn device_and_automation_reads_do_not_replay_the_original_proof() {
    for automation in [false, true] {
        let mut h = Harness::new().await;
        h.authentication.automation.enabled = automation;
        if automation {
            h.role(AdminRole::Operator);
        }
        let key = crate::client_auth::new_signing_key().unwrap();
        let public = crate::client_auth::public_key_to_base64(&key);
        let device = "d".repeat(32);
        let (token, _) = if automation {
            h.devices.issue_automation(
                &device,
                &h.user.id,
                &public,
                vec!["api:read".into()],
                auth_now_ms(),
                60_000,
            )
        } else {
            h.devices.issue(&device, &h.user.id, &public, auth_now_ms())
        }
        .unwrap();
        let path = "/api/code/sessions/fixture/file?path=read.txt";
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        for (name, value) in crate::client_auth::signed_proof_headers(
            &key,
            &device,
            &token,
            "GET",
            path,
            auth_now_ms(),
        )
        .unwrap()
        {
            headers.insert(
                header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        let verified = resolve_product_api_request_principal(
            h.auth(),
            &Method::GET,
            &path.parse().unwrap(),
            &headers,
        )
        .await
        .unwrap()
        .unwrap();
        let captured = ProductContinuation::capture(h.auth(), Some(&verified), &headers).unwrap();
        if automation {
            assert!(
                crate::server::operator_approval::OperatorApproval::capture_product(
                    h.auth(),
                    "service",
                    Some(&verified),
                    &headers
                )
                .is_err()
            );
        }
        for _ in 0..2 {
            assert!(captured.current(h.auth()).await.is_some());
        }
        assert!(
            h.devices
                .authenticate(&headers, &Method::GET, path, auth_now_ms())
                .is_err()
        );
        if automation {
            h.authentication.automation.enabled = false;
            assert!(captured.current(h.auth()).await.is_none());
            h.authentication.automation.enabled = true;
        }
        h.devices.revoke_access_token(&token);
        assert!(captured.current(h.auth()).await.is_none());
    }
}

#[tokio::test]
async fn only_verified_explicit_local_mode_can_capture_local_continuation() {
    let h = Harness::new().await;
    let local = ProductRequestAuth {
        product_auth_enabled: false,
        ..h.auth()
    };
    let verified = resolve_product_api_request_principal(
        local,
        &Method::GET,
        &"/api/code/sessions/s/file".parse().unwrap(),
        &HeaderMap::new(),
    )
    .await
    .unwrap()
    .unwrap();
    let captured = ProductContinuation::capture(local, Some(&verified), &HeaderMap::new()).unwrap();
    assert_eq!(
        captured.current(local).await,
        Some(crate::product_auth::local_product_principal())
    );
    assert!(captured.current(h.auth()).await.is_none());
    assert!(ProductContinuation::capture(local, None, &HeaderMap::new()).is_err());
}
