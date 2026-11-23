use super::*;
use crate::admin::AdminRole;
use crate::store::{ProductApiToken, ProductUser, ProductUserSession};

pub struct Harness {
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
