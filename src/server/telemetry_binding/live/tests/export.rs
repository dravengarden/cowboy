//! Both Sites, temporary signed installation, JSON wire, and actual OTLP HTTP.
use super::*;
use crate::machine_protocol::telemetry_export::{ExportAttempt, ExportOutcome};
use crate::server::operator_approval::OperatorApproval;
use crate::server::telemetry_binding::export::ExportScope;
use crate::server::{AuthenticatedProductRequest, ProductRequestAuth};
use axum::http::StatusCode;

struct Approval {
    hub: crate::core::Hub,
    devices: crate::client_auth::DeviceAccessSessions,
    authentication: crate::auth_plugins::ProductAuthentication,
}
impl Approval {
    fn new() -> Self {
        Self {
            hub: crate::core::Hub::new(),
            devices: Default::default(),
            authentication: crate::auth_plugins::ProductAuthentication::test_default(None),
        }
    }
    fn auth(&self) -> ProductRequestAuth<'_> {
        ProductRequestAuth {
            product_auth_enabled: false,
            store: None,
            hub: &self.hub,
            device_access: &self.devices,
            product_authentication: &self.authentication,
        }
    }
    fn capture(&self) -> OperatorApproval {
        OperatorApproval::capture(
            self.auth(),
            "service-test",
            Some(&AuthenticatedProductRequest {
                principal: crate::product_auth::local_product_principal(),
                cookie_session: None,
                device_identity: None,
            }),
            &Default::default(),
        )
        .unwrap()
    }
}

fn attempt(fixture: &Fixture, index: usize) -> ExportAttempt {
    let (signal, bytes) = crate::otlp::client_fixtures().remove(index);
    ExportAttempt {
        schema: 1,
        attempt_id: format!("managed-export-fixture-{index}"),
        service_id: "service-test".into(),
        machine_id: "machine-test".into(),
        binding: fixture.select().machine_step().unwrap().after().unwrap(),
        payload: crate::otlp::Export {
            signal,
            protobuf: base64::engine::general_purpose::STANDARD.encode(bytes),
        },
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 15_000,
    }
}

fn scope(fixture: &Fixture, request: ExportAttempt, approval: &Approval) -> Result<ExportScope> {
    ExportScope::capture(
        request,
        approval.capture(),
        fixture.control.clone(),
        fixture.catalog.clone(),
        fixture.fences.clone(),
    )
}

fn configure(fixture: &Fixture, endpoint: &str) {
    let lane = serde_json::json!({"base_url": endpoint, "bearer_token": "fixture-only"});
    let path = fixture.root.path().join("machine/telemetry.json");
    fs::write(&path, serde_json::to_vec(&serde_json::json!({
        "plugin": {"plugin_id": fixture.installed.plugin_id, "plugin_version": fixture.installed.plugin_version, "generation_digest": fixture.installed.generation_digest},
        "logs": lane, "metrics": lane, "traces": lane
    })).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

type CapturedRequests = Vec<(String, Vec<u8>)>;

struct Destination {
    endpoint: String,
    requests: Arc<parking_lot::Mutex<CapturedRequests>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Destination {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Destination {
    async fn new(status: StatusCode, partial_metrics: bool) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let requests = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let received = requests.clone();
        let app = axum::Router::new().fallback(
            move |uri: axum::extract::OriginalUri,
                  headers: axum::http::HeaderMap,
                  body: axum::body::Bytes| {
                let received = received.clone();
                async move {
                    assert_eq!(headers["content-type"], "application/x-protobuf");
                    assert_eq!(headers["authorization"], "Bearer fixture-only");
                    received.lock().push((uri.path().into(), body.to_vec()));
                    let body = if partial_metrics && uri.path().ends_with("/metrics") {
                        vec![10, 2, 8, 1]
                    } else {
                        vec![]
                    };
                    (
                        status,
                        [
                            ("content-type", "application/x-protobuf"),
                            ("location", "/must-not-follow"),
                        ],
                        body,
                    )
                }
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            endpoint,
            requests,
            task,
        }
    }
}

#[tokio::test]
async fn managed_export_json_wire_delivers_all_standard_signals_without_mutating_evidence() {
    let fixture = Fixture::new(true, false).await;
    let destination = Destination::new(StatusCode::OK, true).await;
    configure(&fixture, &destination.endpoint);
    fixture.coordinate(&fixture.select()).await;
    let approval = Approval::new();
    let path = fixture
        .root
        .path()
        .join("machine/plugin-operations/telemetry-bindings-v1.json");
    let machine_before = fs::read(&path).unwrap();
    let service_before = fixture
        .store
        .telemetry_binding_ledger("service-test")
        .await
        .unwrap();
    for index in 0..3 {
        let request = attempt(&fixture, index);
        let receipt = scope(&fixture, request.clone(), &approval)
            .unwrap()
            .execute(&fixture.store, approval.auth())
            .await
            .unwrap();
        assert!(receipt.matches(&request));
        assert_eq!(
            receipt.outcome,
            if request.payload.signal == crate::otlp::Signal::Metrics {
                ExportOutcome::Partial { rejected_items: 1 }
            } else {
                ExportOutcome::Delivered {}
            }
        );
        let requests = destination.requests.lock();
        assert_eq!(requests.len(), index + 1);
        assert_eq!(requests[index].1, request.payload.decode().unwrap().0);
        let encoded = serde_json::to_string(&receipt).unwrap();
        assert!(!encoded.contains("fixture-only") && !encoded.contains(&destination.endpoint));
    }
    assert_eq!(fs::read(path).unwrap(), machine_before);
    assert_eq!(
        fixture
            .store
            .telemetry_binding_ledger("service-test")
            .await
            .unwrap(),
        service_before
    );
    assert_eq!(fixture.sends.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.queries.load(Ordering::Relaxed), 1);
    assert!(
        !fixture
            .control
            .events("machine-test")
            .iter()
            .any(|e| matches!(e, MachineEvent::TelemetryExported { .. }))
    );
}

#[tokio::test]
async fn managed_export_http_failures_do_not_retry_or_follow_redirects() {
    for status in [
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::TEMPORARY_REDIRECT,
    ] {
        let fixture = Fixture::new(true, false).await;
        let destination = Destination::new(status, false).await;
        configure(&fixture, &destination.endpoint);
        fixture.coordinate(&fixture.select()).await;
        let approval = Approval::new();
        let receipt = scope(&fixture, attempt(&fixture, 0), &approval)
            .unwrap()
            .execute(&fixture.store, approval.auth())
            .await
            .unwrap();
        assert_eq!(receipt.outcome, ExportOutcome::Unknown {});
        assert_eq!(
            destination.requests.lock().len(),
            1,
            "one independently admitted attempt"
        );
    }
}

#[tokio::test]
async fn managed_export_service_authority_and_live_selection_are_independently_required() {
    for boundary in [
        "absent",
        "unresolved",
        "operator",
        "fence",
        "connection",
        "installation",
        "protocol",
    ] {
        let fixture = Fixture::new(true, false).await;
        let destination = Destination::new(StatusCode::OK, false).await;
        configure(&fixture, &destination.endpoint);
        if boundary != "absent" {
            fixture.coordinate(&fixture.select()).await;
        }
        let approval = Approval::new();
        let scope = scope(&fixture, attempt(&fixture, 0), &approval).unwrap();
        let mut auth = approval.auth();
        match boundary {
            "unresolved" => {
                let mut next = fixture.select();
                next.operation_id = "managed-export-pending-revoke".into();
                next.expected = Some(next.machine_step().unwrap().after().unwrap());
                next.change = BindingChange::Revoke {
                    policy_epoch: "2".to_owned().try_into().unwrap(),
                };
                fixture
                    .store
                    .change_telemetry_binding(&Change::Begin(&next), &|| true)
                    .await
                    .unwrap();
            }
            "operator" => auth.product_auth_enabled = true,
            "fence" => {
                fixture.fences.write().insert(
                    ("machine-test".into(), fixture.installed.plugin_id.clone()),
                    crate::server::PluginFenceState::Installing,
                );
            }
            "connection" | "protocol" => {
                let (tx, _rx) = mpsc::unbounded_channel();
                fixture.control.install(
                    "machine-test".into(),
                    "fixture-epoch".into(),
                    false,
                    if boundary == "protocol" { 15 } else { 16 },
                    tx,
                );
                fixture.publish(vec![fixture.installed.clone()]);
            }
            "installation" => fixture.publish(vec![]),
            _ => {}
        }
        assert!(
            scope.execute(&fixture.store, auth).await.is_err(),
            "{boundary}"
        );
        assert!(destination.requests.lock().is_empty(), "{boundary}");
    }
}

#[tokio::test]
async fn managed_export_machine_rechecks_owned_binding_policy_and_original_queued_lease() {
    for boundary in [
        "absent",
        "binding",
        "file",
        "policy",
        "owner",
        "expired",
        "disconnect",
        "installation",
    ] {
        let fixture = Fixture::new(true, false).await;
        let destination = Destination::new(StatusCode::OK, false).await;
        configure(&fixture, &destination.endpoint);
        if boundary != "absent" {
            fixture.coordinate(&fixture.select()).await;
        }
        let mut request = attempt(&fixture, 0);
        if boundary == "binding" {
            request.binding.policy_epoch = "2".to_owned().try_into().unwrap();
        }
        if boundary == "expired" {
            request.expires_at_ms = 1;
        }
        if boundary == "owner" {
            request.service_id = "foreign-service".into();
        }
        if boundary == "file" {
            fs::remove_file(
                fixture
                    .root
                    .path()
                    .join("machine/plugin-operations/telemetry-bindings-v1.json"),
            )
            .unwrap();
        }
        if boundary == "policy" {
            fs::set_permissions(
                fixture.root.path().join("machine/telemetry.json"),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap();
        }
        if boundary == "installation" {
            request
                .binding
                .selection
                .as_mut()
                .unwrap()
                .installation_revision = format!("installation-{}", "9".repeat(64))
                .try_into()
                .unwrap();
        }
        let connection = PluginExecutionScope::new(Some("service-test"), "machine-test");
        let (tx, mut rx) = mpsc::unbounded_channel();
        crate::machine_cli::telemetry_export::export(
            "managed-export-rpc".into(),
            request.clone(),
            fixture.machine.clone(),
            &connection,
            tx,
        );
        if boundary == "disconnect" {
            drop(connection);
        }
        let MachineEvent::TelemetryExported {
            receipt: Some(receipt),
            ..
        } = rx.recv().await.unwrap()
        else {
            panic!()
        };
        assert!(receipt.matches(&request));
        assert_eq!(receipt.outcome, ExportOutcome::NotAdmitted {}, "{boundary}");
        assert!(destination.requests.lock().is_empty(), "{boundary}");
    }
}
