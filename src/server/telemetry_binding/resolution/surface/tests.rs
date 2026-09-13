use super::*;
use axum::http::Method;
use serde_json::{Value, json};

type BindingFixture = (
    bool,
    Arc<crate::machine_control::MachineControl>,
    Arc<crate::plugin_catalog::PluginCatalog>,
    crate::server::PluginLifecycleFences,
);

#[test]
fn public_projection_matches_the_shared_web_contract() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/telemetry-resolution-surface.json"
    ))
    .unwrap();
    let before: Operation = serde_json::from_value(fixture["source_operation"].clone()).unwrap();
    let intent = ResolutionIntent::new(
        "resolution-contract-fixture".into(),
        Actor::Product {
            user_id: "new-fixture-operator".into(),
        },
        &before,
        ResolutionAction::AbortBeforeDispatch,
        2_300_000_000_000,
    )
    .unwrap();
    let after = intent.conclusion(&before, None).unwrap();
    let record = crate::telemetry_binding::resolution::ResolutionRecord {
        intent: intent.clone(),
        before: before.clone(),
        after: after.clone(),
        resolved_at_ms: 2_200_000_000_000,
    };
    let ledger = Ledger::decode(&json!({"schema":1,"service_id":"service-test","machine_id":"machine-test","current":null,"operations":[before]}).to_string(),"service-test").unwrap();
    let actual = json!({
        "absent":StatusView::new(None,false).unwrap(),
        "retained":StatusView::new(Some(&ledger),false).unwrap(),
        "plan":PlanView::new(&intent,&before,after,false).unwrap(),
        "receipt":ReceiptView::from(&record),
    });
    assert_eq!(fixture["public"], actual);
}

pub(in crate::server::telemetry_binding) struct Fixture {
    _root: tempfile::TempDir,
    pub(in crate::server::telemetry_binding) state: ApiState,
    pub(in crate::server::telemetry_binding) base: String,
    pub(in crate::server::telemetry_binding) client: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    pub(in crate::server::telemetry_binding) async fn new(write_admitted: bool) -> Self {
        Self::with_auth(write_admitted, false).await
    }

    pub(in crate::server::telemetry_binding) async fn with_auth(
        write_admitted: bool,
        product_auth_enabled: bool,
    ) -> Self {
        Self::build(write_admitted, false, product_auth_enabled, None).await
    }

    pub(in crate::server::telemetry_binding) async fn with_recovery(
        write: bool,
        auth: bool,
    ) -> Self {
        Self::build(false, write, auth, None).await
    }

    #[cfg(feature = "machine-host")]
    pub(in crate::server::telemetry_binding) async fn with_binding(
        write: bool,
        auth: bool,
        control: Arc<crate::machine_control::MachineControl>,
        catalog: Arc<crate::plugin_catalog::PluginCatalog>,
        fences: crate::server::PluginLifecycleFences,
    ) -> Self {
        Self::build(false, false, auth, Some((write, control, catalog, fences))).await
    }

    async fn build(
        write_admitted: bool,
        recovery: bool,
        product_auth_enabled: bool,
        binding: Option<BindingFixture>,
    ) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let legacy_fence =
            crate::telemetry_binding::LegacyFence::recover(Some(&store), "service-test")
                .await
                .unwrap();
        let (binding_write, control, catalog, fences) = binding.unwrap_or_else(|| {
            (
                false,
                Arc::default(),
                Arc::new(crate::plugin_catalog::PluginCatalog::open(root.path(), None).unwrap()),
                Default::default(),
            )
        });
        let state = ApiState {
            service: "service-test".into(),
            store: Some(store),
            control,
            catalog,
            fences,
            legacy_fence,
            binding_plans: Arc::default(),
            plans: Arc::default(),
            recovery_plans: Arc::default(),
            hub: crate::core::Hub::new(),
            product_auth_enabled,
            devices: Arc::default(),
            authentication: Arc::new(crate::auth_plugins::ProductAuthentication::test_default(
                None,
            )),
            fixture_write_admission: write_admitted,
            fixture_recovery_admission: recovery,
            fixture_binding_admission: binding_write,
        };
        let router = routes()
            .merge(crate::server::telemetry_binding::recovery::surface::routes())
            .merge(crate::server::telemetry_binding::surface::routes())
            .with_state(state.clone())
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                |State(state): State<ApiState>,
                 mut request: axum::extract::Request,
                 next: axum::middleware::Next| async move {
                    // Use the real credential resolver. Production additionally
                    // supplies its standard origin/freshness/routing middleware.
                    let verified = crate::server::resolve_product_api_request_principal(
                        state.auth(),
                        request.method(),
                        request.uri(),
                        request.headers(),
                    )
                    .await;
                    if let Ok(Some(verified)) = verified {
                        request.extensions_mut().insert(verified);
                    }
                    next.run(request).await
                },
            ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            _root: root,
            state,
            base,
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap(),
            task,
        }
    }

    pub(in crate::server::telemetry_binding) async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = self.client.request(method, format!("{}{path}", self.base));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let status = response.status();
        let bytes = response.text().await.unwrap();
        for secret in [
            "actor",
            "user_id",
            "token",
            "endpoint",
            "observation_digest",
        ] {
            assert!(
                !bytes.contains(secret),
                "private data in response: {secret}"
            );
        }
        (
            status,
            serde_json::from_str(&bytes).unwrap_or(Value::String(bytes)),
        )
    }

    pub(in crate::server::telemetry_binding) async fn pending(&self, dispatch: bool) -> Operation {
        let mut intent = crate::telemetry_binding::fixture("surface-original");
        intent.schema = 2;
        intent.expires_at_ms = 1; // Historical intent is never renewed.
        let store = self.state.store.as_ref().unwrap();
        let before = store
            .change_telemetry_binding(&Change::Begin(&intent), &|| true)
            .await
            .unwrap()
            .operation;
        if dispatch {
            crate::server::telemetry_binding::advance(store, &before, Progress::Dispatching)
                .await
                .unwrap()
        } else {
            before
        }
    }

    async fn inspect(&self, before: &Operation) -> (StatusCode, Value) {
        self.request(
            Method::POST,
            &path(before, "resolution-plan"),
            Some(json!({})),
        )
        .await
    }

    pub(in crate::server::telemetry_binding) async fn stop(self) {
        self.task.abort();
        assert!(self.task.await.unwrap_err().is_cancelled());
    }
}

fn path(before: &Operation, suffix: &str) -> String {
    format!(
        "/api/telemetry/binding/operations/{}/{suffix}",
        before.intent.operation_id
    )
}

fn confirmation(plan: &Value) -> Value {
    json!({"plan_id": plan["plan_id"], "action": plan["action"]})
}

#[tokio::test]
async fn real_cookie_logout_and_role_loss_cannot_confirm_or_read_saved_evidence() {
    use crate::store::{ProductUser, ProductUserSession};
    for failure in ["logout", "role", "disabled"] {
        let f = Fixture::with_auth(true, true).await;
        let now = chrono::Utc::now().timestamp_millis();
        let user = ProductUser {
            id: "c".repeat(32),
            username: "operator".into(),
            password_algo: "argon2id".into(),
            password_hash: "unused-fixture".into(),
            created_at_ms: now,
            updated_at_ms: now,
            disabled_at_ms: None,
        };
        let store = f.state.store.as_ref().unwrap();
        store.insert_user(&user).await.unwrap();
        f.state.hub.set_setting(
            crate::admin::PERMISSIONS_SETTING.into(),
            json!({"default_role":"operator","grants":[]}),
        );
        let session = ProductUserSession {
            token_hash: crate::admin::hex_sha256(b"surface-cookie"),
            session_id: "surface-session-fixture".into(),
            user_id: user.id.clone(),
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
        store.insert_user_session(&session).await.unwrap();
        let before = f.pending(false).await;
        assert_eq!(
            f.request(Method::GET, "/api/telemetry/binding", None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        let response = f
            .client
            .post(format!("{}{}", f.base, path(&before, "resolution-plan")))
            .header(header::COOKIE, "cowboy_user=surface-cookie")
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let plan: Value = response.json().await.unwrap();
        match failure {
            "logout" => {
                store
                    .revoke_user_session_for_user(
                        &user.id,
                        "surface-session-fixture",
                        "logout",
                        now,
                    )
                    .await
                    .unwrap();
            }
            "role" => f.state.hub.set_setting(
                crate::admin::PERMISSIONS_SETTING.into(),
                json!({"default_role":"viewer","grants":[]}),
            ),
            _ => {
                store
                    .set_user_disabled_at(&user.id, Some(now))
                    .await
                    .unwrap();
            }
        }
        for (method, route, body) in [
            (
                Method::POST,
                path(&before, "resolve"),
                Some(confirmation(&plan)),
            ),
            (Method::GET, path(&before, "resolution"), None),
            (Method::GET, "/api/telemetry/binding".into(), None),
        ] {
            let mut request = f
                .client
                .request(method, format!("{}{route}", f.base))
                .header(header::COOKIE, "cowboy_user=surface-cookie");
            if let Some(body) = body {
                request = request.json(&body);
            }
            let response = request.send().await.unwrap();
            assert!(
                matches!(
                    response.status(),
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
                ),
                "{failure}"
            );
        }
        assert_eq!(
            f.state.ledger().await.unwrap().unwrap().operations,
            vec![before]
        );
        f.stop().await;
    }
}

#[tokio::test]
async fn actual_http_production_gate_allows_preview_but_never_writes() {
    let f = Fixture::new(false).await;
    let (status, view) = f.request(Method::GET, "/api/telemetry/binding", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        view,
        json!({"schema":1,"resolution_admission":"closed","journal":{"state":"absent"}})
    );
    let before = f.pending(false).await;
    let (status, plan) = f.inspect(&before).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(plan["action"], "abort_before_dispatch");
    assert_eq!(plan["confirmation_available"], false);
    let (status, error) = f
        .request(
            Method::POST,
            &path(&before, "resolve"),
            Some(confirmation(&plan)),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["error"], "resolution_admission_closed");
    assert_eq!(
        f.state.ledger().await.unwrap().unwrap().operations,
        vec![before]
    );
    f.stop().await;
}

#[tokio::test]
async fn actual_http_offline_abort_is_one_use_and_receipts_are_read_only() {
    let f = Fixture::new(true).await;
    let before = f.pending(false).await;
    let (status, plan) = f.inspect(&before).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(plan["result_head"], Value::Null);
    let (status, receipt) = f
        .request(
            Method::POST,
            &path(&before, "resolve"),
            Some(confirmation(&plan)),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(receipt["phase"], "aborted");
    assert_eq!(receipt["resolution_id"], plan["plan_id"]);
    assert_eq!(
        f.request(
            Method::POST,
            &path(&before, "resolve"),
            Some(confirmation(&plan))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let saved = f.state.ledger().await.unwrap().unwrap();
    for _ in 0..2 {
        assert_eq!(
            f.request(Method::GET, &path(&before, "resolution"), None)
                .await,
            (StatusCode::OK, receipt.clone())
        );
    }
    assert_eq!(f.state.ledger().await.unwrap().unwrap(), saved);
    assert_eq!(saved.operations[0].intent, before.intent);
    assert_eq!(saved.resolutions.len(), 1);
    assert!(
        !LegacyFence::recover(f.state.store.as_ref(), "service-test")
            .await
            .unwrap()
            .allows_legacy()
    );
    assert_eq!(f.inspect(&before).await.0, StatusCode::CONFLICT);
    f.stop().await;
}

#[tokio::test]
async fn actual_http_preview_rechecks_full_operation_and_closed_request_shape() {
    let f = Fixture::new(true).await;
    let before = f.pending(false).await;
    let (_, plan) = f.inspect(&before).await;
    for body in [
        json!({"plan_id":plan["plan_id"],"action":"clear_fence"}),
        json!({"plan_id":plan["plan_id"],"action":"abort_before_dispatch","force":true}),
        json!({"plan_id":plan["plan_id"],"action":"abort_before_dispatch","actor":"forged"}),
    ] {
        assert_eq!(
            f.request(Method::POST, &path(&before, "resolve"), Some(body))
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        f.request(
            Method::POST,
            &path(&before, "resolution-plan"),
            Some(json!({"action":"abort_before_dispatch"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let changed = crate::server::telemetry_binding::advance(
        f.state.store.as_ref().unwrap(),
        &before,
        Progress::Dispatching,
    )
    .await
    .unwrap();
    assert_eq!(
        f.request(
            Method::POST,
            &path(&before, "resolve"),
            Some(confirmation(&plan))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(
        f.state.plans.0.lock().is_empty(),
        "an admitted failed confirmation cannot retry"
    );
    assert_eq!(
        f.state.ledger().await.unwrap().unwrap().operations,
        vec![changed]
    );
    f.stop().await;
}

#[tokio::test]
async fn http_remote_resolution_queries_freshly_and_refuses_changed_or_unknown_evidence() {
    use crate::machine_protocol::{MachineCommand, MachineEvent};
    for changed in [false, true] {
        let f = Fixture::new(true).await;
        let before = f.pending(true).await;
        let value = crate::telemetry_binding::tests::applied(&before.intent);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        f.state
            .control
            .install("machine-test".into(), "preview-wire".into(), false, 17, tx);
        let connection = f
            .state
            .control
            .operation_connection("machine-test")
            .unwrap();
        let control = f.state.control.clone();
        let task = tokio::spawn(async move {
            let mut queries = 0;
            while let Some(command) = rx.recv().await {
                let MachineCommand::QueryTelemetryBinding { request_id, .. } = command else {
                    panic!("resolution must never dispatch a mutation");
                };
                queries += 1;
                let observation = if changed && queries == 2 {
                    BindingObservation::Unavailable {
                        reason:
                            crate::machine_protocol::telemetry_binding::BindingUnavailable::Storage,
                    }
                } else {
                    value.clone()
                };
                control.record_remote(
                    &connection,
                    MachineEvent::TelemetryBindingObservation {
                        request_id,
                        observation: Box::new(observation),
                    },
                );
                if queries == 2 {
                    return queries;
                }
            }
            queries
        });
        let (status, plan) = f.inspect(&before).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(plan["action"], "accept_applied");
        assert_eq!(plan["result_phase"], "completed");
        assert_eq!(
            f.request(
                Method::POST,
                &path(&before, "resolve"),
                Some(confirmation(&plan))
            )
            .await
            .0,
            if changed {
                StatusCode::CONFLICT
            } else {
                StatusCode::OK
            }
        );
        assert_eq!(task.await.unwrap(), 2);
        let ledger = f.state.ledger().await.unwrap().unwrap();
        assert_eq!(ledger.resolutions.len(), usize::from(!changed));
        if changed {
            assert_eq!(ledger.operations, vec![before]);
        }
        f.stop().await;
    }
}

#[test]
fn bounded_previews_bind_actor_service_action_and_original_monotonic_deadline() {
    let plans = Plans::default();
    let before = Operation {
        intent: crate::telemetry_binding::fixture("preview-owner"),
        progress: Progress::Prepared,
    };
    let intent = ResolutionIntent::new(
        "resolution-preview-owner".into(),
        before.intent.actor.clone(),
        &before,
        ResolutionAction::AbortBeforeDispatch,
        chrono::Utc::now().timestamp_millis() + 120_000,
    )
    .unwrap();
    let insert = |intent: ResolutionIntent| {
        plans.insert(Preview {
            budget: OperationBudget::new(
                intent.expires_at_ms,
                Duration::from_mins(2),
                TimeSample::now(),
            ),
            intent,
        })
    };
    insert(intent.clone()).unwrap();
    assert!(insert(intent.clone()).is_err());
    let request = Confirmation {
        plan_id: intent.resolution_id.clone(),
        action: Action::AbortBeforeDispatch,
    };
    assert!(
        plans
            .consume(
                &Actor::Admin {
                    account: "foreign".into()
                },
                &intent.service_id,
                &intent.operation_id,
                &request
            )
            .is_err()
    );
    assert!(
        plans
            .consume(&intent.actor, "foreign", &intent.operation_id, &request)
            .is_err()
    );
    assert!(
        plans
            .consume(&intent.actor, &intent.service_id, "foreign", &request)
            .is_err()
    );
    let wrong = Confirmation {
        plan_id: request.plan_id.clone(),
        action: Action::AcceptApplied,
    };
    assert!(
        plans
            .consume(
                &intent.actor,
                &intent.service_id,
                &intent.operation_id,
                &wrong
            )
            .is_err()
    );
    assert_eq!(
        plans
            .consume(
                &intent.actor,
                &intent.service_id,
                &intent.operation_id,
                &request
            )
            .unwrap()
            .intent,
        intent
    );
    assert!(
        plans
            .consume(
                &intent.actor,
                &intent.service_id,
                &intent.operation_id,
                &request
            )
            .is_err()
    );
    insert(intent.clone()).unwrap();
    plans
        .0
        .lock()
        .get(&intent.resolution_id)
        .unwrap()
        .budget
        .expire_for_test();
    assert!(
        plans
            .consume(
                &intent.actor,
                &intent.service_id,
                &intent.operation_id,
                &request
            )
            .is_err()
    );
    for index in 0..256 {
        let mut next = intent.clone();
        next.resolution_id = format!("resolution-{index:016}");
        insert(next).unwrap();
    }
    assert!(insert(intent).is_err());
    for route in [
        "/api/telemetry/binding".to_owned(),
        path(&before, "resolution-plan"),
        path(&before, "resolve"),
        path(&before, "resolution"),
    ] {
        for method in [Method::GET, Method::POST] {
            assert_eq!(
                crate::server::classify_route(&method, &route),
                crate::server::RouteAuth::ProductOrAdminOperator
            );
        }
    }
}
