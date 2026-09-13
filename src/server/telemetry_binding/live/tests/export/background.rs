//! Standing host authority with real signed Victoria, both ledgers, protocol
//! dispatch and loopback HTTP. No Operator cookie or production policy is used.
use super::*;
use crate::observability::{ExportBatch, TelemetryExporter};
use crate::telemetry_plugin::background_policy::Activation;
use crate::telemetry_plugin::background_policy::BackgroundPolicy;

fn policy_path(f: &Fixture) -> std::path::PathBuf {
    f.root.path().join("background-policy.json")
}

fn write_policy(f: &Fixture) {
    fs::write(
        policy_path(f),
        serde_json::to_vec(&serde_json::json!({
            "schema": 1, "service_id": "service-test", "machine_id": "machine-test",
            "binding": f.select().machine_step().unwrap().after().unwrap(),
            "signals": {"logs": true, "metrics": true, "traces": true},
            "startup": "activate_exact_binding"
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(policy_path(f), fs::Permissions::from_mode(0o600)).unwrap();
}

fn exporter(f: &Fixture, policy: Arc<BackgroundPolicy>) -> TelemetryExporter {
    crate::server::telemetry_binding::background::exporter(
        policy,
        f.store.clone(),
        f.control.clone(),
        f.catalog.clone(),
        f.fences.clone(),
    )
}

fn batch(f: &Fixture, index: usize) -> ExportBatch {
    ExportBatch {
        logs: String::new(),
        metrics: String::new(),
        otlp: Some(attempt(f, index).payload),
    }
}

#[tokio::test]
async fn background_export_delivers_all_otlp_lanes_under_independent_exact_policy() {
    let f = Fixture::new(true, false).await;
    let destination = Destination::new(StatusCode::OK, true).await;
    configure(&f, &destination.endpoint);
    f.coordinate(&f.select()).await;
    write_policy(&f);
    let policy = BackgroundPolicy::load(&policy_path(&f), "service-test").unwrap();
    assert!(policy.check_binding(&f.store).await);
    let export = exporter(&f, policy);
    let machine_path = f
        .root
        .path()
        .join("machine/plugin-operations/telemetry-bindings-v1.json");
    let machine_before = fs::read(&machine_path).unwrap();
    let before = f
        .store
        .telemetry_binding_ledger("service-test")
        .await
        .unwrap();
    let fixtures = crate::otlp::client_fixtures();
    for (index, (signal, _)) in fixtures.iter().enumerate() {
        let request = attempt(&f, index);
        let receipt = export(batch(&f, index)).await;
        assert_eq!(receipt.logs_delivered, *signal == crate::otlp::Signal::Logs);
        assert_eq!(
            receipt.metrics_delivered,
            *signal == crate::otlp::Signal::Metrics
        );
        assert_eq!(
            receipt.traces_delivered,
            *signal == crate::otlp::Signal::Traces
        );
        assert_eq!(
            receipt.rejected_items,
            u64::from(*signal == crate::otlp::Signal::Metrics)
        );
        let requests = destination.requests.lock();
        assert_eq!(requests.len(), index + 1);
        assert_eq!(requests[index].1, request.payload.decode().unwrap().0);
    }
    let legacy = export(ExportBatch {
        logs: "legacy".into(),
        metrics: "legacy".into(),
        otlp: None,
    })
    .await;
    assert!(!legacy.logs_delivered && !legacy.metrics_delivered);
    assert_eq!(
        destination.requests.lock().len(),
        fixtures.len(),
        "no encoding/legacy fallback"
    );
    assert_eq!(fs::read(machine_path).unwrap(), machine_before);
    assert_eq!(
        f.store
            .telemetry_binding_ledger("service-test")
            .await
            .unwrap(),
        before
    );
    assert_eq!(f.sends.load(Ordering::Relaxed), 1);
    assert_eq!(f.queries.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn background_activation_requires_current_complete_binding_and_does_not_adopt_later_history()
{
    let f = Fixture::new(true, false).await;
    let destination = Destination::new(StatusCode::OK, false).await;
    configure(&f, &destination.endpoint);
    write_policy(&f);
    let absent = BackgroundPolicy::load(&policy_path(&f), "service-test").unwrap();
    assert!(matches!(
        absent.clone().activate(&f.store).await,
        Activation::Stopped
    ));
    f.coordinate(&f.select()).await;
    assert!(
        matches!(absent.activate(&f.store).await, Activation::Stopped),
        "stopped startup cannot adopt a later binding"
    );
    let policy = BackgroundPolicy::load(&policy_path(&f), "service-test").unwrap();
    let Activation::Active(policy) = policy.activate(&f.store).await else {
        panic!("a fresh explicit activation must accept the exact binding");
    };
    let export = exporter(&f, policy);
    assert!(export(batch(&f, 1)).await.logs_delivered);

    let mut revoke = f.select();
    revoke.operation_id = "background-policy-revoke".into();
    revoke.expected = Some(revoke.machine_step().unwrap().after().unwrap());
    revoke.change = BindingChange::Revoke {
        policy_epoch: "2".to_owned().try_into().unwrap(),
    };
    f.coordinate(&revoke).await;
    assert!(!export(batch(&f, 1)).await.logs_delivered);
    assert!(
        !BackgroundPolicy::load(&policy_path(&f), "service-test")
            .unwrap()
            .check_binding(&f.store)
            .await
    );
    let mut restore = revoke.clone();
    restore.operation_id = "background-policy-restore".into();
    restore.expected = Some(revoke.machine_step().unwrap().after().unwrap());
    restore.change = BindingChange::Restore {
        forward_request_digest: revoke.machine_step().unwrap().request_digest().unwrap(),
        selection: revoke.expected.as_ref().unwrap().selection.clone(),
        policy_epoch: "3".to_owned().try_into().unwrap(),
    };
    f.coordinate(&restore).await;
    assert!(
        !export(batch(&f, 1)).await.logs_delivered,
        "restoration is a new epoch, not old policy authority"
    );
    assert!(
        !BackgroundPolicy::load(&policy_path(&f), "service-test")
            .unwrap()
            .check_binding(&f.store)
            .await
    );
    assert_eq!(destination.requests.lock().len(), 1);
}

#[tokio::test]
async fn background_pending_service_operation_stops_until_an_explicit_new_activation() {
    let f = Fixture::new(true, false).await;
    let destination = Destination::new(StatusCode::OK, false).await;
    configure(&f, &destination.endpoint);
    f.coordinate(&f.select()).await;
    write_policy(&f);
    let policy = BackgroundPolicy::load(&policy_path(&f), "service-test").unwrap();
    let export = exporter(&f, policy.clone());
    let mut next = f.select();
    next.operation_id = "background-prepared-revoke".into();
    next.expected = Some(next.machine_step().unwrap().after().unwrap());
    next.change = BindingChange::Revoke {
        policy_epoch: "2".to_owned().try_into().unwrap(),
    };
    let pending = f
        .store
        .change_telemetry_binding(&Change::Begin(&next), &|| true)
        .await
        .unwrap()
        .operation;
    assert!(!export(batch(&f, 1)).await.logs_delivered);
    f.store
        .change_telemetry_binding(
            &Change::Advance {
                expected: &pending,
                progress: Progress::Aborted,
            },
            &|| true,
        )
        .await
        .unwrap();
    assert!(!policy.check_binding(&f.store).await);
    assert!(!export(batch(&f, 1)).await.logs_delivered);
    assert!(destination.requests.lock().is_empty());
    let fresh = BackgroundPolicy::load(&policy_path(&f), "service-test").unwrap();
    assert!(fresh.check_binding(&f.store).await);
    assert!(exporter(&f, fresh)(batch(&f, 1)).await.logs_delivered);
}

#[tokio::test]
async fn background_scopes_recheck_original_policy_connection_inventory_and_fences() {
    for boundary in [
        "policy",
        "connection",
        "protocol",
        "inventory",
        "fence",
        "catalog",
    ] {
        let f = Fixture::new(true, false).await;
        let destination = Destination::new(StatusCode::OK, false).await;
        configure(&f, &destination.endpoint);
        f.coordinate(&f.select()).await;
        write_policy(&f);
        let policy = BackgroundPolicy::load(&policy_path(&f), "service-test").unwrap();
        let (request, permit) = policy.admit(attempt(&f, 0).payload).unwrap();
        let scope = ExportScope::background(
            request,
            permit,
            f.control.clone(),
            f.catalog.clone(),
            f.fences.clone(),
        )
        .unwrap();
        match boundary {
            "policy" => {
                // Byte-identical atomic replacement is still a new policy owner.
                let next = f.root.path().join("next-policy.json");
                fs::copy(policy_path(&f), &next).unwrap();
                fs::rename(next, policy_path(&f)).unwrap();
            }
            "connection" | "protocol" => {
                let (tx, _rx) = mpsc::unbounded_channel();
                let connection = f.control.install(
                    "machine-test".into(),
                    "fixture-epoch".into(),
                    false,
                    if boundary == "protocol" { 15 } else { 18 },
                    tx,
                );
                f.control.record_remote(
                    &connection,
                    MachineEvent::PluginInventory {
                        plugins: vec![f.installed.clone()],
                        observed_at_ms: 2,
                    },
                );
            }
            "inventory" => f.publish(vec![]),
            "fence" => {
                f.fences.write().insert(
                    ("machine-test".into(), f.installed.plugin_id.clone()),
                    crate::server::PluginFenceState::Installing,
                );
            }
            "catalog" => {
                fs::remove_file(f.root.path().join("catalog/victoria.release.json")).unwrap();
                let storage = crate::plugin_storage::PluginStorage::sqlite_files(
                    crate::plugin_dir::PluginDir::open(f.root.path()).unwrap(),
                );
                f.catalog.refresh_with_runtime(&storage).await.unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            scope.execute_background(&f.store).await.is_err(),
            "{boundary}"
        );
        assert!(destination.requests.lock().is_empty(), "{boundary}");
    }
}

#[tokio::test]
async fn background_and_operator_scopes_keep_distinct_typed_authorities() {
    let f = Fixture::new(true, false).await;
    let destination = Destination::new(StatusCode::OK, false).await;
    configure(&f, &destination.endpoint);
    f.coordinate(&f.select()).await;
    write_policy(&f);
    let policy = BackgroundPolicy::load(&policy_path(&f), "service-test").unwrap();
    let approval = Approval::new();
    let (request, permit) = policy.admit(attempt(&f, 0).payload).unwrap();
    let background: ExportScope<crate::telemetry_plugin::background_policy::BackgroundPermit> =
        ExportScope::background(
            request,
            permit,
            f.control.clone(),
            f.catalog.clone(),
            f.fences.clone(),
        )
        .unwrap();
    let confirmed: ExportScope<crate::server::operator_approval::TelemetryExportAuthority> =
        scope(&f, attempt(&f, 0), &approval).unwrap();
    assert_eq!(
        background
            .execute_background(&f.store)
            .await
            .unwrap()
            .outcome,
        ExportOutcome::Delivered {}
    );
    assert_eq!(
        confirmed
            .execute(&f.store, approval.auth())
            .await
            .unwrap()
            .outcome,
        ExportOutcome::Delivered {}
    );
    assert_eq!(destination.requests.lock().len(), 2);
}

#[tokio::test]
async fn background_failure_is_one_attempt_and_local_file_pipeline_is_independent() {
    let f = Fixture::new(true, false).await;
    let destination = Destination::new(StatusCode::SERVICE_UNAVAILABLE, false).await;
    configure(&f, &destination.endpoint);
    f.coordinate(&f.select()).await;
    write_policy(&f);
    let policy = BackgroundPolicy::load(&policy_path(&f), "service-test").unwrap();
    let file_root = f.root.path().join("local-telemetry");
    let file = crate::telemetry_file::TelemetryFile::open(
        file_root.clone(),
        65_536,
        2,
        chrono::Utc::now().timestamp_millis(),
    )
    .unwrap();
    let writer = crate::observability::Observability::start(
        Some(f.store.clone()),
        file,
        Some(exporter(&f, policy)),
    );
    for (index, (signal, bytes)) in crate::otlp::client_fixtures().into_iter().enumerate() {
        writer
            .submit_otlp(
                "fixture-owner",
                signal,
                &format!("background-{index}"),
                &bytes,
            )
            .unwrap();
    }
    writer.drain().await;
    assert_eq!(
        destination.requests.lock().len(),
        4,
        "one attempt per SDK fixture, no retry on 503"
    );
    assert_eq!(writer.health().failed_log_batches(), 1);
    assert_eq!(writer.health().failed_metric_batches(), 2);
    assert_eq!(writer.health().failed_trace_batches(), 1);
    assert_eq!(writer.health().failed_file_batches(), 0);
    assert!(
        !fs::read(file_root.join("telemetry.jsonl"))
            .unwrap()
            .is_empty()
    );
}
