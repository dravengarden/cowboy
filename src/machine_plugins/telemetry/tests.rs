use super::*;
use crate::machine_plugins::tests::telemetry_release;
use axum::http::StatusCode;
use std::sync::atomic::AtomicUsize;

struct Fixture {
    _root: tempfile::TempDir,
    store: Arc<MachinePluginStore>,
    installed: PluginInventory,
    desired: DesiredPlugin,
    policy: PathBuf,
}

impl Fixture {
    async fn new(legacy: bool) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let root = tempfile::tempdir().unwrap();
        let publisher =
            crate::machine_auth::MachineIdentity::load_or_create(&root.path().join("publisher"))
                .unwrap();
        let desired = telemetry_release(&publisher, if legacy { "1.0.0" } else { "1.1.0" });
        let machine = root.path().join("machine");
        let store =
            Arc::new(MachinePluginStore::new(&machine, Platform::Linux, "x86_64".into()).unwrap());
        let installed = store.install(&desired).await.unwrap();
        Self {
            _root: root,
            store,
            installed,
            desired,
            policy: machine.join("telemetry.json"),
        }
    }

    fn configure(&self, endpoint: &str) {
        atomic_write(
            &self.policy,
            &serde_json::to_vec(&serde_json::json!({
                "plugin": { "plugin_id": self.installed.plugin_id,
                    "plugin_version": self.installed.plugin_version,
                    "generation_digest": self.installed.generation_digest },
                "logs": { "base_url": endpoint }, "metrics": { "base_url": endpoint },
                "traces": { "base_url": endpoint }
            }))
            .unwrap(),
            0o600,
        )
        .unwrap();
    }

    fn request(&self) -> PluginHostRequest {
        let legacy = self.installed.plugin_version == "1.0.0";
        PluginHostRequest {
            plugin_id: self.installed.plugin_id.clone(),
            plugin_version: self.installed.plugin_version.clone(),
            generation_digest: self.installed.generation_digest.clone(),
            auth_generation: None,
            operation: if legacy {
                PluginHostOperation::ExportTelemetry
            } else {
                PluginHostOperation::ExportOtlp
            },
            payload: if legacy {
                serde_json::json!({"logs":"{}\n", "metrics":"fixture 1\n"})
            } else {
                let (signal, bytes) = crate::otlp::client_fixtures().remove(0);
                serde_json::json!({"signal":signal, "protobuf":base64::engine::general_purpose::STANDARD.encode(bytes)})
            },
        }
    }

    fn scope() -> PluginExecutionScope {
        PluginExecutionScope::new(Some("service-test"), "machine-test")
    }
}

// Real HTTP gated by explicit fixture signals, never guessed retry sleeps.
struct Destination {
    endpoint: String,
    requests: Arc<AtomicUsize>,
    entered: tokio::sync::mpsc::UnboundedReceiver<()>,
    responses: Arc<tokio::sync::Semaphore>,
    task: tokio::task::JoinHandle<()>,
}

impl Destination {
    async fn new(status: StatusCode) -> Self {
        let requests = Arc::new(AtomicUsize::new(0));
        let (tx, entered) = tokio::sync::mpsc::unbounded_channel();
        let responses = Arc::new(tokio::sync::Semaphore::new(0));
        let count = Arc::clone(&requests);
        let gate = Arc::clone(&responses);
        let router = axum::Router::new().fallback(move || {
            let (count, tx, gate) = (Arc::clone(&count), tx.clone(), Arc::clone(&gate));
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                tx.send(()).unwrap();
                gate.acquire().await.unwrap().forget();
                status
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            endpoint,
            requests,
            entered,
            responses,
            task,
        }
    }
    async fn started(&mut self) {
        tokio::time::timeout(Duration::from_secs(2), self.entered.recv())
            .await
            .unwrap()
            .unwrap();
    }
    fn release(&self) {
        self.responses.add_permits(8);
    }
    fn count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Drop for Destination {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn finish(
    task: tokio::task::JoinHandle<
        std::result::Result<serde_json::Value, PluginHostInvocationFailure>,
    >,
) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn disconnected_or_expired_invocations_cannot_be_revived_by_a_new_connection() {
    let fixture = Fixture::new(false).await;
    let destination = Destination::new(StatusCode::OK).await;
    fixture.configure(&destination.endpoint);
    for disconnected in [true, false] {
        let scope = Fixture::scope();
        let invocation = scope.host(fixture.request());
        if disconnected {
            drop(scope);
        } else {
            invocation.telemetry.as_ref().unwrap().retire();
        }
        let _replacement = Fixture::scope();
        let failure = fixture.store.invoke_host(invocation).await.unwrap_err();
        assert!(!failure.started);
    }
    assert_eq!(destination.count(), 0);
}

#[tokio::test]
async fn queued_command_rechecks_connection_after_the_lifecycle_lock() {
    let fixture = Fixture::new(false).await;
    let destination = Destination::new(StatusCode::OK).await;
    fixture.configure(&destination.endpoint);
    let scope = Fixture::scope();
    let invocation = scope.host(fixture.request());
    let lifecycle = fixture.store.lifecycle.lock().await;
    let store = Arc::clone(&fixture.store);
    let task = tokio::spawn(async move { store.invoke_host(invocation).await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while fixture.store.telemetry_export.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(scope);
    let _replacement = Fixture::scope();
    drop(lifecycle);
    let failure = task.await.unwrap().unwrap_err();
    assert!(!failure.started);
    assert_eq!(destination.count(), 0);
}

#[tokio::test]
async fn a_queued_call_ends_within_its_original_budget_without_acquiring_the_lock() {
    let fixture = Fixture::new(false).await;
    let scope = Fixture::scope();
    let mut invocation = scope.host(fixture.request());
    invocation.telemetry.as_mut().unwrap().deadline = Instant::now() + Duration::from_millis(30);
    let _lifecycle = fixture.store.lifecycle.lock().await;
    let failure = tokio::time::timeout(
        Duration::from_secs(1),
        fixture.store.invoke_host(invocation),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(!failure.started);
    assert!(fixture.store.telemetry_export.try_lock().is_ok());
}

#[tokio::test]
async fn disconnect_stops_retry_but_does_not_undo_an_admitted_http_receipt() {
    for status in [StatusCode::SERVICE_UNAVAILABLE, StatusCode::OK] {
        let fixture = Fixture::new(false).await;
        let mut destination = Destination::new(status).await;
        fixture.configure(&destination.endpoint);
        let scope = Fixture::scope();
        let invocation = scope.host(fixture.request());
        let store = Arc::clone(&fixture.store);
        let task = tokio::spawn(async move { store.invoke_host(invocation).await });
        destination.started().await;
        drop(scope);
        let _replacement = Fixture::scope();
        destination.release();
        let result = finish(task).await;
        assert_eq!(result["otlp"]["delivered"], status == StatusCode::OK);
        assert_eq!(result["otlp"]["enabled"], true);
        assert_eq!(destination.count(), 1);
        assert!(fixture.store.telemetry_export.try_lock().is_ok());
    }
}

#[tokio::test]
async fn policy_revocation_replacement_and_invalidity_never_retarget_a_retry() {
    for mutation in [
        "delete",
        "replace_same",
        "replace_new",
        "in_place_aba",
        "public",
        "invalid",
        "symlink",
    ] {
        let fixture = Fixture::new(false).await;
        let mut original = Destination::new(StatusCode::SERVICE_UNAVAILABLE).await;
        let replacement = Destination::new(StatusCode::OK).await;
        fixture.configure(&original.endpoint);
        let bytes = fs::read(&fixture.policy).unwrap();
        let scope = Fixture::scope();
        let invocation = scope.host(fixture.request());
        let store = Arc::clone(&fixture.store);
        let task = tokio::spawn(async move { store.invoke_host(invocation).await });
        original.started().await;
        match mutation {
            "delete" => fs::remove_file(&fixture.policy).unwrap(),
            "replace_same" => fixture.configure(&original.endpoint),
            "replace_new" => fixture.configure(&replacement.endpoint),
            "in_place_aba" => {
                fs::write(&fixture.policy, b"{}").unwrap();
                fs::write(&fixture.policy, &bytes).unwrap();
            }
            "public" => {
                fs::set_permissions(&fixture.policy, fs::Permissions::from_mode(0o644)).unwrap();
            }
            "invalid" => fs::write(&fixture.policy, b"not JSON").unwrap(),
            "symlink" => {
                let target = fixture.policy.with_extension("fixture");
                fs::rename(&fixture.policy, &target).unwrap();
                symlink(&target, &fixture.policy).unwrap();
            }
            _ => unreachable!(),
        }
        original.release();
        assert_eq!(finish(task).await["otlp"]["delivered"], false, "{mutation}");
        assert_eq!(original.count(), 1, "{mutation}");
        assert_eq!(replacement.count(), 0, "{mutation}");
    }
}

#[tokio::test]
async fn uninstall_reinstall_and_tampering_end_old_attempts_without_holding_lifecycle_during_http()
{
    for mutation in ["uninstall", "reinstall", "tamper"] {
        let fixture = Fixture::new(false).await;
        let mut destination = Destination::new(StatusCode::SERVICE_UNAVAILABLE).await;
        fixture.configure(&destination.endpoint);
        let scope = Fixture::scope();
        let invocation = scope.host(fixture.request());
        let store = Arc::clone(&fixture.store);
        let task = tokio::spawn(async move { store.invoke_host(invocation).await });
        destination.started().await;
        tokio::time::timeout(Duration::from_secs(1), async {
            if mutation == "tamper" {
                let (_, _, content) = fixture
                    .store
                    .verified_plugin_generation("victoria", &fixture.installed.generation_digest)
                    .unwrap();
                atomic_write(&content.join("package.cowboy-plugin"), b"{}", 0o600).unwrap();
            } else {
                fixture
                    .store
                    .uninstall("victoria", &fixture.installed.generation_digest)
                    .await
                    .unwrap();
                if mutation == "reinstall" {
                    fixture.store.install(&fixture.desired).await.unwrap();
                }
            }
        })
        .await
        .unwrap();
        destination.release();
        assert_eq!(finish(task).await["otlp"]["delivered"], false, "{mutation}");
        assert_eq!(destination.count(), 1, "{mutation}");
    }
}

#[tokio::test]
async fn concurrent_export_is_rejected_and_legacy_lanes_each_recheck_before_retry() {
    let fixture = Fixture::new(true).await;
    let mut destination = Destination::new(StatusCode::SERVICE_UNAVAILABLE).await;
    fixture.configure(&destination.endpoint);
    let scope = Fixture::scope();
    let invocation = scope.host(fixture.request());
    let store = Arc::clone(&fixture.store);
    let task = tokio::spawn(async move { store.invoke_host(invocation).await });
    destination.started().await;
    destination.started().await;
    let failure = fixture
        .store
        .invoke_host(scope.host(fixture.request()))
        .await
        .unwrap_err();
    assert!(!failure.started);
    drop(scope);
    destination.release();
    assert_eq!(
        finish(task).await,
        serde_json::json!({"logs_delivered": false, "metrics_delivered": false})
    );
    assert_eq!(destination.count(), 2);
}

#[tokio::test]
async fn deadline_stops_retry_for_every_otlp_signal_without_cancelling_the_first_attempt() {
    for (signal, bytes) in crate::otlp::client_fixtures() {
        let fixture = Fixture::new(false).await;
        let mut destination = Destination::new(StatusCode::SERVICE_UNAVAILABLE).await;
        fixture.configure(&destination.endpoint);
        let scope = Fixture::scope();
        let mut request = fixture.request();
        request.payload = serde_json::json!({"signal":signal,
            "protobuf":base64::engine::general_purpose::STANDARD.encode(bytes)});
        let mut invocation = scope.host(request);
        // One HTTP attempt is acknowledged only after this deadline. The
        // subsequent retry must use the original budget, not renew 15 seconds.
        invocation.telemetry.as_mut().unwrap().deadline =
            Instant::now() + Duration::from_millis(100);
        let deadline = invocation.telemetry.as_ref().unwrap().deadline;
        let store = Arc::clone(&fixture.store);
        let task = tokio::spawn(async move { store.invoke_host(invocation).await });
        destination.started().await;
        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
        destination.release();
        assert_eq!(finish(task).await["otlp"]["delivered"], false);
        assert_eq!(destination.count(), 1);
    }
}

#[tokio::test]
async fn fresh_call_can_use_a_new_policy_but_never_revives_an_old_invocation() {
    let fixture = Fixture::new(false).await;
    let mut original = Destination::new(StatusCode::SERVICE_UNAVAILABLE).await;
    let replacement = Destination::new(StatusCode::OK).await;
    fixture.configure(&original.endpoint);
    let scope = Fixture::scope();
    let invocation = scope.host(fixture.request());
    let store = Arc::clone(&fixture.store);
    let task = tokio::spawn(async move { store.invoke_host(invocation).await });
    original.started().await;
    fixture.configure(&replacement.endpoint);
    original.release();
    assert_eq!(finish(task).await["otlp"]["delivered"], false);
    assert_eq!(replacement.count(), 0);
    replacement.release();
    let next = fixture
        .store
        .invoke_host(scope.host(fixture.request()))
        .await
        .unwrap();
    assert_eq!(next["otlp"]["delivered"], true);
    assert_eq!(original.count(), 1);
    assert_eq!(replacement.count(), 1);
}

#[test]
fn retired_lease_remains_retired_even_if_a_clock_or_policy_is_repaired() {
    let connected = Arc::new(AtomicBool::new(true));
    let mut lease = TelemetryExecutionLease {
        connected: Arc::clone(&connected),
        deadline: Instant::now(),
        retired: AtomicBool::new(false),
    };
    assert!(lease.remaining().is_err());
    lease.deadline = Instant::now() + ADMISSION_BUDGET;
    assert!(lease.remaining().is_err());
    let lease = TelemetryExecutionLease {
        connected: Arc::clone(&connected),
        deadline: Instant::now() + ADMISSION_BUDGET,
        retired: AtomicBool::new(false),
    };
    connected.store(false, Ordering::Release);
    assert!(lease.remaining().is_err());
    connected.store(true, Ordering::Release);
    assert!(lease.remaining().is_err());
}

#[tokio::test]
async fn inventory_cannot_substitute_a_version_or_contract_for_the_signed_release() {
    for field in ["plugin_version", "contract_fingerprint"] {
        let mut fixture = Fixture::new(false).await;
        let destination = Destination::new(StatusCode::OK).await;
        let (_, _, content) = fixture
            .store
            .verified_plugin_generation("victoria", &fixture.installed.generation_digest)
            .unwrap();
        let path = content.join("plugin-inventory.json");
        let mut inventory: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        if field == "plugin_version" {
            fixture.installed.plugin_version = "9.0.0".into();
            inventory[field] = serde_json::json!("9.0.0");
        } else {
            inventory[field] = serde_json::json!(format!("sha256:{}", "0".repeat(64)));
        }
        atomic_write(&path, &serde_json::to_vec(&inventory).unwrap(), 0o600).unwrap();
        fixture.configure(&destination.endpoint);
        let scope = Fixture::scope();
        let failure = fixture
            .store
            .invoke_host(scope.host(fixture.request()))
            .await
            .unwrap_err();
        assert!(!failure.started, "{field}");
        assert_eq!(destination.count(), 0, "{field}");
    }
}

#[tokio::test]
async fn revocation_after_prepare_but_before_first_attempt_is_a_preflight_rejection() {
    for revoke_connection in [true, false] {
        let fixture = Fixture::new(false).await;
        let destination = Destination::new(StatusCode::OK).await;
        fixture.configure(&destination.endpoint);
        let scope = Fixture::scope();
        let invocation = scope.host(fixture.request());
        let lifecycle = fixture.store.lifecycle.lock().await;
        let store = Arc::clone(&fixture.store);
        let task = tokio::spawn(async move { store.invoke_host(invocation).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while fixture.store.telemetry_export.try_lock().is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // Tokio's FIFO mutex puts this mutation after initial validation, but
        // before the attempt's separate checkpoint. Both paths are real code.
        let store = Arc::clone(&fixture.store);
        let policy = fixture.policy.clone();
        let (queued, waiting) = tokio::sync::oneshot::channel();
        let mutation = tokio::spawn(async move {
            queued.send(()).unwrap();
            let _lifecycle = store.lifecycle.lock().await;
            if revoke_connection {
                drop(scope);
                None
            } else {
                fs::remove_file(policy).unwrap();
                Some(scope)
            }
        });
        waiting.await.unwrap();
        drop(lifecycle);
        let _remaining_scope = mutation.await.unwrap();
        let failure = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(!failure.started);
        assert_eq!(destination.count(), 0);
    }
}
