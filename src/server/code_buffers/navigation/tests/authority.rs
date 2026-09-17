use super::*;
use crate::admin::{AdminRole, hex_sha256};
use crate::store::{ProductApiToken, ProductUser};

#[test]
fn all_navigation_routes_are_product_operator_only() {
    for method in [Method::GET, Method::POST, Method::PUT, Method::DELETE] {
        for path in [
            "/api/code/buffers/source/navigations",
            "/api/code/navigations/group",
            "/api/code/navigations/group/destinations",
        ] {
            assert_eq!(classify_route(&method, path), RouteAuth::ProductOperator);
        }
    }
}

struct AuthFixture {
    fixture: Fixture,
    source: String,
    authenticated: AuthenticatedProductRequest,
    headers: HeaderMap,
    store: Store,
    token: ProductApiToken,
    _root: tempfile::TempDir,
}

impl AuthFixture {
    async fn new() -> Self {
        let mut fixture = fixture(21);
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let user = ProductUser {
            id: "c".repeat(32),
            username: "fixture".into(),
            password_algo: "argon2id".into(),
            password_hash: "unused-test-password".into(),
            created_at_ms: auth_now_ms(),
            updated_at_ms: auth_now_ms(),
            disabled_at_ms: None,
        };
        store.insert_user(&user).await.unwrap();
        fixture.context.hub.delete_session("session");
        crate::server::code_buffers::tests::create_owned(&fixture.context.hub, "machine", &user.id);
        let source = opened_as(&fixture, &user.id);
        let token = ProductApiToken {
            id: "a".repeat(32),
            user_id: user.id.clone(),
            name: "fixture".into(),
            token_prefix: "cow_test".into(),
            token_hash: hex_sha256(b"cow_test-navigation"),
            created_at_ms: auth_now_ms(),
            expires_at_ms: Some(auth_now_ms() + 300_000),
            last_used_at_ms: None,
            revoked_at_ms: None,
        };
        store.insert_user_api_token(&token).await.unwrap();
        fixture.context.product_auth_enabled = true;
        fixture.context.store = Some(store.clone());
        fixture.context.hub.set_setting(
            crate::admin::PERMISSIONS_SETTING.into(),
            json!({"default_role":AdminRole::Operator,"grants":[]}),
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer cow_test-navigation".parse().unwrap(),
        );
        let authenticated = resolve_product_api_request_principal(
            fixture.context.auth(),
            &Method::POST,
            &format!("/api/code/buffers/{source}/navigations")
                .parse()
                .unwrap(),
            &headers,
        )
        .await
        .unwrap()
        .unwrap();
        Self {
            fixture,
            source,
            authenticated,
            headers,
            store,
            token,
            _root: root,
        }
    }

    fn prepare(&self) -> tokio::task::JoinHandle<Response> {
        tokio::spawn(prepare(
            AxumState(self.fixture.context.clone()),
            Path(self.source.clone()),
            Extension(self.authenticated.clone()),
            self.headers.clone(),
            Json(request()),
        ))
    }

    fn operate(&self, id: &str, action: Action) -> tokio::task::JoinHandle<Response> {
        tokio::spawn(operate(
            self.fixture.context.clone(),
            id.into(),
            self.authenticated.clone(),
            self.headers.clone(),
            action,
        ))
    }

    async fn invalidate(&self, revoke: bool) {
        if revoke {
            self.store
                .revoke_user_api_token_for_user(&self.token.user_id, &self.token.id)
                .await
                .unwrap();
        } else {
            self.fixture.context.hub.set_setting(
                crate::admin::PERMISSIONS_SETTING.into(),
                json!({"default_role":AdminRole::Viewer,"grants":[]}),
            );
        }
    }
}

#[tokio::test]
async fn navigation_rechecks_original_credential_and_operator_role_at_each_boundary() {
    for stage in [
        "prepare",
        "before_execute",
        "execute_reply",
        "destination_reply",
        "query_reply",
        "release_reply",
    ] {
        for revoke in [false, true] {
            let mut fixture = AuthFixture::new().await;
            let task = fixture.prepare();
            let sent = command(&mut fixture.fixture).await;
            if stage == "prepare" {
                fixture.invalidate(revoke).await;
            }
            reply(
                &fixture.fixture,
                sent,
                serde_json::to_value(observed(Phase::Prepared)).unwrap(),
            );
            let response = task.await.unwrap();
            if stage == "prepare" {
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
                continue;
            }
            let id = json_response(response).await["navigationId"]
                .as_str()
                .unwrap()
                .to_owned();
            if stage == "before_execute" {
                fixture.invalidate(revoke).await;
                assert_eq!(
                    fixture
                        .operate(&id, Action::Execute)
                        .await
                        .unwrap()
                        .status(),
                    StatusCode::UNAUTHORIZED
                );
                assert!(fixture.fixture.commands.try_recv().is_err());
                continue;
            }
            let task = fixture.operate(&id, Action::Execute);
            let sent = command(&mut fixture.fixture).await;
            if stage == "execute_reply" {
                fixture.invalidate(revoke).await;
            }
            reply(
                &fixture.fixture,
                sent,
                serde_json::to_value(observed(Phase::Retained)).unwrap(),
            );
            let response = task.await.unwrap();
            if stage == "execute_reply" {
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
                continue;
            }
            assert_eq!(response.status(), StatusCode::OK);
            let action = match stage {
                "destination_reply" => Action::Destination {
                    destination: 0,
                    content: content(),
                },
                "query_reply" => Action::Query,
                "release_reply" => Action::Release,
                _ => unreachable!(),
            };
            let task = fixture.operate(&id, action);
            let sent = command(&mut fixture.fixture).await;
            fixture.invalidate(revoke).await;
            let mut value = observed(if stage == "release_reply" {
                Phase::Released
            } else {
                Phase::Retained
            });
            if stage == "destination_reply" {
                value.destinations.push(
                    crate::machine_protocol::code_buffer_navigation::Destination {
                        destination: 0,
                        lease: serde_json::from_value(native(2)).unwrap(),
                    },
                );
            }
            reply(&fixture.fixture, sent, serde_json::to_value(value).unwrap());
            assert_eq!(task.await.unwrap().status(), StatusCode::UNAUTHORIZED);
        }
    }
}
