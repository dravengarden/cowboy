use super::*;
use crate::admin::{AdminRole, hex_sha256};
use crate::store::{ProductApiToken, ProductUser};

struct AuthFixture {
    fixture: Fixture,
    resource: String,
    authenticated: AuthenticatedProductRequest,
    headers: HeaderMap,
    store: Store,
    token: ProductApiToken,
    _root: tempfile::TempDir,
}

impl AuthFixture {
    async fn new() -> Self {
        let mut fixture = fixture(20);
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let user = ProductUser {
            id: "c".repeat(32),
            username: "fixture".into(),
            password_algo: "argon2id".into(),
            password_hash: "unused-hermetic-fixture".into(),
            created_at_ms: auth_now_ms(),
            updated_at_ms: auth_now_ms(),
            disabled_at_ms: None,
        };
        store.insert_user(&user).await.unwrap();
        fixture.context.hub.delete_session("session");
        crate::server::code_buffers::tests::create_owned(&fixture.context.hub, "machine", &user.id);
        let resource = opened_as(&fixture, 1, &user.id);
        let token = ProductApiToken {
            id: "a".repeat(32),
            user_id: user.id.clone(),
            name: "fixture".into(),
            token_prefix: "cow_test".into(),
            token_hash: hex_sha256(b"cow_test-buffer-sync"),
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
            "Bearer cow_test-buffer-sync".parse().unwrap(),
        );
        let authenticated = resolve_product_api_request_principal(
            fixture.context.auth(),
            &Method::POST,
            &format!("/api/code/buffers/{resource}/synchronizations")
                .parse()
                .unwrap(),
            &headers,
        )
        .await
        .unwrap()
        .unwrap();
        Self {
            fixture,
            resource,
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
            Path(self.resource.clone()),
            Extension(self.authenticated.clone()),
            self.headers.clone(),
            Json(Prepare {
                purpose: Purpose::RefreshFromDisk,
                content: content(),
            }),
        ))
    }

    fn operate(&self, id: &str, action: Action) -> tokio::task::JoinHandle<Response> {
        tokio::spawn(operate(
            self.fixture.context.clone(),
            id.to_owned(),
            self.authenticated.clone(),
            self.headers.clone(),
            action,
        ))
    }

    async fn invalidate(&self, change: &str) {
        if change == "revoked" {
            self.store
                .revoke_user_api_token_for_user(&self.token.user_id, &self.token.id)
                .await
                .unwrap();
        } else {
            self.fixture.context.hub.set_setting(
                crate::admin::PERMISSIONS_SETTING.into(),
                json!({"default_role":AdminRole::Viewer,"grants":[]}),
            );
            if change == "role_aba" {
                self.fixture.context.hub.set_setting(
                    crate::admin::PERMISSIONS_SETTING.into(),
                    json!({"default_role":AdminRole::Operator,"grants":[]}),
                );
            }
        }
    }
}

#[tokio::test]
async fn expired_sync_deadline_never_consumes_an_inert_attempt_or_dispatches() {
    for action in [Action::Apply, Action::Query, Action::Retire] {
        let mut f = fixture(20);
        let resource = opened(&f, 1);
        let prepared = prepared(&mut f, &resource).await;
        let approval = Arc::new(approval(&f.context, &authenticated(), &HeaderMap::new()).unwrap());
        let registry::Admission::Run(job) = f
            .context
            .code_buffers
            .synchronizations
            .admit("local", &prepared.operation_id, action)
            .unwrap()
        else {
            panic!("admission")
        };
        assert!(matches!(
            run_job(f.context.clone(), approval, *job, Instant::now()).await,
            Err(StatusCode::GATEWAY_TIMEOUT)
        ));
        assert!(f.commands.try_recv().is_err());
        assert!(matches!(
            f.context
                .code_buffers
                .synchronizations
                .admit("local", &prepared.operation_id, action)
                .unwrap(),
            registry::Admission::Run(_)
        ));
    }
}

#[tokio::test]
async fn revoked_failures_do_not_disclose_sync_outcomes_or_rearm_apply() {
    for action in [Action::Apply, Action::Query, Action::Retire] {
        for change in ["revoked", "role", "role_aba"] {
            let mut fixture = AuthFixture::new().await;
            let task = fixture.prepare();
            let sent = command(&mut fixture.fixture).await;
            reply(
                &fixture.fixture,
                sent,
                observation(NativeState::Prepared {}),
            );
            let id = json_response(task.await.unwrap()).await["operationId"]
                .as_str()
                .unwrap()
                .to_owned();
            let task = fixture.operate(&id, action);
            let sent = command(&mut fixture.fixture).await;
            fixture.invalidate(change).await;
            reply(&fixture.fixture, sent, Value::Null);
            let response = task.await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{action:?}/{change}"
            );
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert!(!response.headers().contains_key(header::ETAG));
            if action != Action::Query {
                let registry::Admission::Saved(snapshot) = fixture
                    .fixture
                    .context
                    .code_buffers
                    .synchronizations
                    .admit(&fixture.token.user_id, &id, action)
                    .unwrap()
                else {
                    panic!("an uncertain effect must never be rearmed");
                };
                assert!(!snapshot.pending);
                if action == Action::Apply {
                    assert!(matches!(snapshot.state, State::Unknown {}));
                }
            }
            assert!(fixture.fixture.commands.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn credentials_and_role_are_rechecked_on_preparation_confirmation_and_response() {
    for stage in ["prepare", "before_apply", "apply_reply"] {
        for change in ["revoked", "role", "role_aba"] {
            let mut fixture = AuthFixture::new().await;
            let task = fixture.prepare();
            let sent = command(&mut fixture.fixture).await;
            if stage == "prepare" {
                fixture.invalidate(change).await;
            }
            reply(
                &fixture.fixture,
                sent,
                observation(NativeState::Prepared {}),
            );
            let response = task.await.unwrap();
            if stage == "prepare" {
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
                drop(
                    fixture
                        .fixture
                        .context
                        .code_buffers
                        .admit_read(&fixture.token.user_id, &fixture.resource)
                        .unwrap(),
                );
                assert!(fixture.fixture.commands.try_recv().is_err());
                continue;
            }
            let id = json_response(response).await["operationId"]
                .as_str()
                .unwrap()
                .to_owned();
            if stage == "before_apply" {
                fixture.invalidate(change).await;
            }
            let task = fixture.operate(&id, Action::Apply);
            if stage == "apply_reply" {
                let sent = command(&mut fixture.fixture).await;
                fixture.invalidate(change).await;
                reply(&fixture.fixture, sent, observation(applied()));
            }
            assert_eq!(task.await.unwrap().status(), StatusCode::UNAUTHORIZED);
            assert!(fixture.fixture.commands.try_recv().is_err());
            if stage == "apply_reply" {
                // Effect evidence is retained even when response authorization
                // ends. Neither the old token nor role repair can replay Apply.
                assert_eq!(
                    fixture.operate(&id, Action::Query).await.unwrap().status(),
                    StatusCode::UNAUTHORIZED
                );
                let registry::Admission::Saved(snapshot) = fixture
                    .fixture
                    .context
                    .code_buffers
                    .synchronizations
                    .admit(&fixture.token.user_id, &id, Action::Apply)
                    .unwrap()
                else {
                    panic!();
                };
                assert!(matches!(snapshot.state, State::Applied { .. }));
            }
        }
    }
}

#[test]
fn all_synchronization_methods_are_product_operator_only_and_admin_is_not_inherited() {
    for method in [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::HEAD,
    ] {
        for path in [
            "/api/code/buffers/resource/synchronizations",
            "/api/code/buffer-synchronizations/operation",
        ] {
            assert_eq!(classify_route(&method, path), RouteAuth::ProductOperator);
        }
    }
    let fixture = fixture(20);
    let mut principal = authenticated();
    principal.principal.role = AdminRole::Viewer;
    assert!(matches!(
        approval(&fixture.context, &principal, &HeaderMap::new()),
        Err(StatusCode::FORBIDDEN)
    ));
}

#[tokio::test]
async fn pending_apply_keeps_its_fence_when_the_original_connection_ends() {
    let mut fixture = fixture(20);
    let resource = opened(&fixture, 1);
    let prepared = prepared(&mut fixture, &resource).await;
    let task = start(&fixture, &prepared.operation_id, Action::Apply);
    let sent = command(&mut fixture).await;
    fixture
        .context
        .machine_control
        .remove_if_current(&fixture.connection);
    reply(&fixture, sent, observation(applied()));
    assert_eq!(task.await.unwrap().status(), StatusCode::BAD_GATEWAY);
    assert_eq!(
        json_response(
            start(&fixture, &prepared.operation_id, Action::Apply)
                .await
                .unwrap()
        )
        .await["state"]["kind"],
        "unknown"
    );
    assert!(matches!(
        fixture.context.code_buffers.admit_read("local", &resource),
        Err(StatusCode::CONFLICT)
    ));
}
