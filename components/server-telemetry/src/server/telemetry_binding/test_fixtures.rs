//! Shared test fixtures for telemetry binding unit and cross-crate integration tests.

use super::http::ApiState;
use super::{Confirmation, Effects};
use crate::machine_protocol::telemetry_binding::{
    BindingObservation, BindingObservationSnapshot, BindingStep,
};
use crate::server::operator_approval::{OperatorApproval, TelemetryResolutionAuthority};
use crate::server::{AuthenticatedProductRequest, ProductRequestAuth};
use crate::store::Store;
use crate::telemetry_binding::resolution::ResolutionIntent;
use crate::telemetry_binding::tests::applied;
use crate::telemetry_binding::{Intent, Operation, Progress, writer::Change};
use anyhow::Result;
use axum::extract::State;
use axum::http::{Method, StatusCode, header};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

pub struct FixtureEffects {
    pub intent: Intent,
    pub result: BindingObservation,
    pub lose_ack: bool,
    pub lose_query: bool,
    pub revoke_on_dispatch: bool,
    pub alive: AtomicBool,
    pub sends: AtomicUsize,
    pub queries: AtomicUsize,
    pub checks: AtomicUsize,
    pub revoke_on_check: Option<usize>,
    pub authority: crate::server::operator_approval::TelemetryBindingAuthority,
    pub hub: crate::core::Hub,
    pub devices: crate::client_auth::DeviceAccessSessions,
    pub authentication: crate::auth_plugins::ProductAuthentication,
}

impl FixtureEffects {
    pub fn new(intent: &Intent) -> Self {
        let hub = crate::core::Hub::new();
        let devices = crate::client_auth::DeviceAccessSessions::default();
        let authentication = crate::auth_plugins::ProductAuthentication::test_default(None);
        let auth = ProductRequestAuth {
            product_auth_enabled: false,
            store: None,
            hub: &hub,
            device_access: &devices,
            product_authentication: &authentication,
        };
        let verified = AuthenticatedProductRequest {
            permissions: None,
            principal: crate::product_auth::local_product_principal(),
            caller: crate::prompt_origin::RequestCaller::Person,
            cookie_session: None,
            device_identity: None,
        };
        let approval = OperatorApproval::capture(
            auth,
            &intent.service_id,
            Some(&verified),
            &axum::http::HeaderMap::new(),
        )
        .unwrap();
        let authority = approval.bind_telemetry(intent).unwrap();
        Self {
            intent: intent.clone(),
            result: applied(intent),
            lose_ack: false,
            lose_query: false,
            revoke_on_dispatch: false,
            alive: AtomicBool::new(true),
            sends: AtomicUsize::new(0),
            queries: AtomicUsize::new(0),
            checks: AtomicUsize::new(0),
            revoke_on_check: None,
            authority,
            hub,
            devices,
            authentication,
        }
    }

    pub fn confirmation(&self) -> Confirmation<'_> {
        Confirmation {
            authority: &self.authority,
            auth: ProductRequestAuth {
                product_auth_enabled: false,
                store: None,
                hub: &self.hub,
                device_access: &self.devices,
                product_authentication: &self.authentication,
            },
        }
    }
}

impl Effects for FixtureEffects {
    async fn authorized(&self, intent: &Intent) -> bool {
        let check = self.checks.fetch_add(1, Ordering::Relaxed) + 1;
        if self.revoke_on_check == Some(check) {
            self.alive.store(false, Ordering::Release);
        }
        intent == &self.intent && self.within_budget()
    }
    fn within_budget(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }
    async fn dispatch(&self, step: &BindingStep) -> Result<BindingObservation> {
        assert_eq!(step, &self.intent.machine_step().unwrap());
        assert_eq!(
            self.sends.fetch_add(1, Ordering::Relaxed),
            0,
            "never retry a finite mutation"
        );
        if self.revoke_on_dispatch {
            self.alive.store(false, Ordering::Release);
        }
        if self.lose_ack {
            anyhow::bail!("hermetic lost ACK");
        }
        Ok(self.result.clone())
    }
    async fn observe(&self, step: &BindingStep) -> Result<BindingObservation> {
        assert_eq!(step, &self.intent.machine_step().unwrap());
        let query = self.queries.fetch_add(1, Ordering::Relaxed);
        if query == 0 {
            return Ok(BindingObservation::Observed {
                snapshot: Box::new(BindingObservationSnapshot {
                    request_digest: step.request_digest()?,
                    receipt: None,
                    current: self.intent.expected.clone(),
                    unresolved: false,
                }),
            });
        }
        if self.lose_query {
            anyhow::bail!("hermetic observation unavailable");
        }
        Ok(self.result.clone())
    }
}

pub fn resolution_authority(
    auth: ProductRequestAuth<'_>,
    intent: &ResolutionIntent,
) -> TelemetryResolutionAuthority {
    let verified = AuthenticatedProductRequest {
        permissions: None,
        principal: crate::product_auth::local_product_principal(),
        caller: crate::prompt_origin::RequestCaller::Person,
        cookie_session: None,
        device_identity: None,
    };
    OperatorApproval::capture(
        auth,
        &intent.service_id,
        Some(&verified),
        &axum::http::HeaderMap::new(),
    )
    .unwrap()
    .bind_telemetry_resolution(intent)
    .unwrap()
}

type BindingFixture = (
    bool,
    Arc<crate::machine_control::MachineControl>,
    Arc<crate::plugin_catalog::PluginCatalog>,
    crate::server::PluginLifecycleFences,
);

pub struct HttpFixture {
    _root: tempfile::TempDir,
    pub state: ApiState,
    pub base: String,
    pub client: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
}

impl HttpFixture {
    pub async fn with_admission(
        self,
        policy: Arc<crate::telemetry_plugin::writer_admission::WriterAdmission>,
    ) -> Self {
        let Self {
            _root,
            mut state,
            task,
            ..
        } = self;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        state.admission = Some(policy);
        state.fixture_binding_admission = false;
        state.fixture_recovery_admission = false;
        state.fixture_write_admission = false;
        state.plans = Arc::default();
        state.recovery_plans = Arc::default();
        state.binding_plans = Arc::default();
        Self::serve(_root, state).await
    }

    pub async fn new(write_admitted: bool) -> Self {
        Self::with_auth(write_admitted, false).await
    }

    pub async fn with_auth(write_admitted: bool, product_auth_enabled: bool) -> Self {
        Self::build(write_admitted, false, product_auth_enabled, None).await
    }

    pub async fn with_recovery(write: bool, auth: bool) -> Self {
        Self::build(false, write, auth, None).await
    }

    pub async fn with_binding(
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
            admission: None,
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
        Self::serve(root, state).await
    }

    pub async fn restart(self) -> Self {
        let Self {
            _root,
            mut state,
            task,
            ..
        } = self;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        state.plans = Arc::default();
        state.recovery_plans = Arc::default();
        state.binding_plans = Arc::default();
        state.control = Arc::default();
        Self::serve(_root, state).await
    }

    async fn serve(root: tempfile::TempDir, state: ApiState) -> Self {
        let router = crate::server::telemetry_binding::resolution::surface::routes()
            .merge(crate::server::telemetry_binding::recovery::surface::routes())
            .merge(crate::server::telemetry_binding::surface::routes())
            .with_state(state.clone())
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                |State(state): State<ApiState>,
                 mut request: axum::extract::Request,
                 next: axum::middleware::Next| async move {
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

    pub async fn request(
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

    pub async fn pending(&self, dispatch: bool) -> Operation {
        let mut intent = crate::telemetry_binding::fixture("surface-original");
        intent.schema = 2;
        intent.expires_at_ms = 1;
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

    pub async fn inspect(&self, before: &Operation) -> (StatusCode, Value) {
        self.request(
            Method::POST,
            &format!(
                "/api/telemetry/binding/operations/{}/resolution-plan",
                before.intent.operation_id
            ),
            Some(json!({})),
        )
        .await
    }

    pub async fn stop(self) {
        self.task.abort();
        assert!(self.task.await.unwrap_err().is_cancelled());
    }
}
