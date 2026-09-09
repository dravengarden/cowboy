//! Hermetic signed Catalog -> Service policy -> live Machine port tests.
//! No production policy, publisher key, endpoint, or worker is consulted.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use cowboy_plugin_sdk::{
    PluginManifest, PluginPackage, PluginPayload, PluginRelease, TelemetryBackendContract,
};
use tokio::sync::mpsc;

use super::controller_exporter;
use crate::machine_control::{ConnectionToken, MachineControl};
use crate::machine_protocol::{
    MachineCommand, MachineEvent, PluginHostOperation, PluginInstallationState, PluginInventory,
    ProviderMaterializationState, ProviderReplicaState,
};
use crate::observability::{ExportBatch, ExportReceipt, TelemetryExporter};
use crate::otlp::Signal;
use crate::plugin_catalog::PluginCatalog;

struct Fixture {
    root: tempfile::TempDir,
    release: PluginRelease,
}

impl Fixture {
    fn new(schema: u16) -> Self {
        let root = tempfile::Builder::new()
            .prefix("cowboy-telemetry-binding-")
            .tempdir()
            .unwrap();
        let manifest: PluginManifest = serde_json::from_str(include_str!(
            "../../examples/telemetry/victoria/plugin.json"
        ))
        .unwrap();
        let mut contract: TelemetryBackendContract = serde_json::from_str(include_str!(
            "../../examples/telemetry/victoria/telemetry.json"
        ))
        .unwrap();
        if schema == 1 {
            contract.schema_version = 1;
            let logs = contract.logs.as_mut().unwrap();
            logs.encoding = cowboy_plugin_sdk::TelemetryEncoding::JsonLines;
            logs.path = "/insert/jsonline".to_owned();
            let metrics = contract.metrics.as_mut().unwrap();
            metrics.encoding = cowboy_plugin_sdk::TelemetryEncoding::PrometheusText;
            metrics.path = "/api/v1/import/prometheus".to_owned();
            contract.traces = None;
        }
        let platforms = contract.supported_platforms.clone();
        let package = PluginPackage::new(
            manifest.clone(),
            manifest.component_release.clone(),
            PluginPayload::TelemetryBackend(contract),
        )
        .unwrap();
        let bytes = package.canonical_bytes().unwrap();
        let mut release = PluginRelease {
            // Data-only packages have no host bundle. Payload schema and
            // outer release schema are independent version domains.
            release_schema: 1,
            plugin_id: manifest.id,
            plugin_version: manifest.version,
            plugin_kind: manifest.kind,
            package_digest: PluginPackage::artifact_digest(&bytes),
            artifact_digest: String::new(),
            artifact_url: "https://plugins.example.test/victoria.cowboy-plugin".to_owned(),
            publisher: manifest.publisher,
            contract_fingerprint: package.contract_fingerprint,
            component_release: manifest.component_release,
            host_bundle_digest: None,
            signature: String::new(),
            runtime_artifacts: platforms
                .iter()
                .map(|platform| cowboy_plugin_sdk::PluginRuntimeArtifacts {
                    os: platform.os.clone(),
                    architecture: platform.architecture.clone(),
                    components: Vec::new(),
                })
                .collect(),
            supported_platforms: platforms,
        };
        let identity = crate::machine_auth::MachineIdentity::load_or_create(
            &root.path().join("fixture-publisher"),
        )
        .unwrap();
        release.artifact_digest = release.computed_artifact_digest().unwrap();
        release.signature = identity
            .sign_namespaced(
                cowboy_plugin_sdk::PLUGIN_RELEASE_SIGNATURE_NAMESPACE,
                &release.proof(),
            )
            .unwrap();
        release.validate_bytes(&bytes).unwrap();
        let catalog = root.path().join("catalog");
        fs::create_dir_all(catalog.join("trusted-publishers")).unwrap();
        fs::write(
            catalog
                .join("trusted-publishers")
                .join(format!("{}.pub", release.publisher)),
            identity.public_key(),
        )
        .unwrap();
        fs::write(catalog.join("victoria.cowboy-plugin"), bytes).unwrap();
        fs::write(
            catalog.join("victoria.release.json"),
            serde_json::to_vec(&release).unwrap(),
        )
        .unwrap();
        Self { root, release }
    }

    fn catalog(&self) -> Arc<PluginCatalog> {
        let mut catalog =
            PluginCatalog::open(self.root.path(), Some(self.root.path().join("catalog"))).unwrap();
        let mut policy = crate::plugin_activation::HostActivationPolicy::default();
        policy.source = crate::plugin_activation::HostSourcePolicy::CatalogOnly;
        catalog.configure_hosts(policy).unwrap();
        Arc::new(catalog)
    }

    fn policy(&self, edit: impl FnOnce(&mut serde_json::Value)) -> PathBuf {
        let mut value = serde_json::json!({
            "machine_id": "hawk", "plugin": {
                "plugin_id": self.release.plugin_id, "plugin_version": self.release.plugin_version,
                "generation_digest": self.release.artifact_digest,
            },
        });
        edit(&mut value);
        let path = self.root.path().join("service-telemetry.json");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    fn inventory(&self) -> PluginInventory {
        PluginInventory {
            plugin_id: self.release.plugin_id.clone(),
            plugin_version: self.release.plugin_version.clone(),
            plugin_kind: self.release.plugin_kind,
            generation_digest: self.release.artifact_digest.clone(),
            contract_fingerprint: self.release.contract_fingerprint.clone(),
            state: PluginInstallationState::Active,
            rollback_generation_digest: None,
            active_session_leases: 0,
            auth_generation: None,
            replica_state: ProviderReplicaState::Absent,
            materialization_state: ProviderMaterializationState::NotInstalled,
            detail: None,
        }
    }

    fn exporter(
        &self,
        control: &Arc<MachineControl>,
        catalog: &Arc<PluginCatalog>,
    ) -> TelemetryExporter {
        controller_exporter(
            Some(&self.policy(|_| {})),
            Arc::clone(control),
            Arc::clone(catalog),
        )
        .unwrap()
        .unwrap()
    }
}

fn connect(
    control: &MachineControl,
    id: &str,
    protocol: u16,
    plugins: Vec<PluginInventory>,
) -> (ConnectionToken, mpsc::UnboundedReceiver<MachineCommand>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let token = control.install(id.to_owned(), "test-epoch".to_owned(), false, protocol, tx);
    control.record_remote(
        &token,
        MachineEvent::PluginInventory {
            plugins,
            observed_at_ms: 1,
        },
    );
    (token, rx)
}

fn batch(signal: Option<Signal>) -> ExportBatch {
    match signal {
        Some(signal) => {
            let (_, bytes) = crate::otlp::client_fixtures()
                .into_iter()
                .find(|(lane, _)| *lane == signal)
                .unwrap();
            ExportBatch {
                logs: String::new(),
                metrics: String::new(),
                otlp: Some(crate::otlp::Export {
                    signal,
                    protobuf: base64::engine::general_purpose::STANDARD.encode(bytes),
                }),
            }
        }
        None => ExportBatch {
            logs: "{\"message\":\"fixture\"}\n".to_owned(),
            metrics: "fixture 1\n".to_owned(),
            otlp: None,
        },
    }
}

async fn denied(
    exporter: &TelemetryExporter,
    signal: Option<Signal>,
    rx: &mut mpsc::UnboundedReceiver<MachineCommand>,
) {
    let receipt = tokio::time::timeout(std::time::Duration::from_secs(1), exporter(batch(signal)))
        .await
        .expect("preflight must not await a Machine reply");
    assert!(!receipt.logs_delivered && !receipt.metrics_delivered && !receipt.traces_delivered);
    assert_eq!(receipt.rejected_items, 0);
    assert!(
        rx.try_recv().is_err(),
        "rejected binding enqueued a command"
    );
}

async fn deliver(
    fixture: &Fixture,
    control: &MachineControl,
    connection: &ConnectionToken,
    rx: &mut mpsc::UnboundedReceiver<MachineCommand>,
    exporter: &TelemetryExporter,
    signal: Option<Signal>,
) -> ExportReceipt {
    let expected = batch(signal);
    let (receipt, ()) = tokio::join!(exporter(batch(signal)), async {
        let command = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let MachineCommand::InvokePluginHost {
            request_id,
            plugin_id,
            plugin_version,
            generation_digest,
            auth_generation,
            operation,
            payload,
        } = command
        else {
            panic!("wrong command");
        };
        assert_eq!(plugin_id, fixture.release.plugin_id);
        assert_eq!(plugin_version, fixture.release.plugin_version);
        assert_eq!(generation_digest, fixture.release.artifact_digest);
        assert_eq!(auth_generation, None);
        let response = if let Some(otlp) = expected.otlp {
            assert_eq!(operation, PluginHostOperation::ExportOtlp);
            assert_eq!(payload, serde_json::to_value(otlp).unwrap());
            serde_json::json!({ "logs_delivered": false, "metrics_delivered": false,
                "otlp": { "enabled": true, "delivered": true, "rejected_items": 0 } })
        } else {
            assert_eq!(operation, PluginHostOperation::ExportTelemetry);
            assert_eq!(
                payload,
                serde_json::json!({"logs": expected.logs, "metrics": expected.metrics})
            );
            serde_json::json!({"logs_delivered": true, "metrics_delivered": true})
        };
        control.record_remote(
            connection,
            MachineEvent::PluginHostResponse {
                request_id,
                accepted: true,
                started: true,
                payload: Some(response),
                detail: None,
            },
        );
    });
    receipt
}

#[tokio::test]
async fn signed_policy_binding_delivers_all_otlp_lanes_without_payload_history() {
    let fixture = Fixture::new(2);
    let catalog = fixture.catalog();
    let control = Arc::new(MachineControl::default());
    let (connection, mut rx) = connect(&control, "hawk", 9, vec![fixture.inventory()]);
    let exporter = fixture.exporter(&control, &catalog);
    for signal in [Signal::Logs, Signal::Metrics, Signal::Traces] {
        let receipt = deliver(
            &fixture,
            &control,
            &connection,
            &mut rx,
            &exporter,
            Some(signal),
        )
        .await;
        assert_eq!(receipt.logs_delivered, signal == Signal::Logs);
        assert_eq!(receipt.metrics_delivered, signal == Signal::Metrics);
        assert_eq!(receipt.traces_delivered, signal == Signal::Traces);
    }
    assert!(
        control
            .events("hawk")
            .iter()
            .all(|event| matches!(event, MachineEvent::PluginInventory { .. }))
    );
    denied(&exporter, None, &mut rx).await;
}

#[tokio::test]
async fn legacy_contract_does_not_accept_otlp_or_lower_protocols() {
    let fixture = Fixture::new(1);
    let catalog = fixture.catalog();
    let control = Arc::new(MachineControl::default());
    let (connection, mut rx) = connect(&control, "hawk", 8, vec![fixture.inventory()]);
    let exporter = fixture.exporter(&control, &catalog);
    let receipt = deliver(&fixture, &control, &connection, &mut rx, &exporter, None).await;
    assert!(receipt.logs_delivered && receipt.metrics_delivered);
    for signal in [Signal::Logs, Signal::Metrics, Signal::Traces] {
        denied(&exporter, Some(signal), &mut rx).await;
    }
    let (_, mut old_rx) = connect(&control, "hawk", 7, vec![fixture.inventory()]);
    denied(&exporter, None, &mut old_rx).await;
}

#[tokio::test]
async fn service_policy_and_inventory_never_fall_back_to_a_different_identity() {
    let fixture = Fixture::new(2);
    let catalog = fixture.catalog();
    let control = Arc::new(MachineControl::default());
    let (connection, mut rx) = connect(&control, "hawk", 9, vec![fixture.inventory()]);
    for (field, value) in [
        ("plugin_id", "other".to_owned()),
        ("plugin_version", "999.0.0".to_owned()),
        ("generation_digest", format!("sha256:{}", "a".repeat(64))),
    ] {
        let path = fixture.policy(|config| config["plugin"][field] = value.into());
        let exporter = controller_exporter(Some(&path), Arc::clone(&control), Arc::clone(&catalog))
            .unwrap()
            .unwrap();
        denied(&exporter, Some(Signal::Logs), &mut rx).await;
    }
    let path = fixture.policy(|config| config["machine_id"] = "falcon".into());
    let exporter = controller_exporter(Some(&path), Arc::clone(&control), Arc::clone(&catalog))
        .unwrap()
        .unwrap();
    denied(&exporter, Some(Signal::Logs), &mut rx).await;
    let exporter = fixture.exporter(&control, &catalog);
    for mismatch in 0..7 {
        let mut entry = fixture.inventory();
        match mismatch {
            0 => entry.contract_fingerprint = format!("sha256:{}", "a".repeat(64)),
            1 => entry.state = PluginInstallationState::Uninstalling,
            2 => entry.plugin_kind = cowboy_plugin_sdk::PluginKind::AgentProvider,
            3 => entry.generation_digest = format!("sha256:{}", "a".repeat(64)),
            4 => entry.plugin_version = "999.0.0".to_owned(),
            5 => entry.plugin_id = "other".to_owned(),
            _ => entry.auth_generation = Some(1),
        }
        control.record_remote(
            &connection,
            MachineEvent::PluginInventory {
                plugins: vec![entry],
                observed_at_ms: 1,
            },
        );
        denied(&exporter, Some(Signal::Logs), &mut rx).await;
    }
    control.record_remote(
        &connection,
        MachineEvent::PluginInventory {
            plugins: vec![fixture.inventory(), fixture.inventory()],
            observed_at_ms: 1,
        },
    );
    denied(&exporter, Some(Signal::Logs), &mut rx).await;
    let (_, mut old_rx) = connect(&control, "hawk", 8, vec![fixture.inventory()]);
    denied(&exporter, Some(Signal::Logs), &mut old_rx).await;
}

#[test]
fn signed_projection_rejects_unpublished_and_untrusted_catalog_artifacts() {
    for corruption in 0..4 {
        let fixture = Fixture::new(2);
        let root = fixture.root.path().join("catalog");
        match corruption {
            0 => {
                fs::remove_file(root.join("victoria.release.json")).unwrap();
                assert!(
                    fixture
                        .catalog()
                        .resolve_telemetry_backend(
                            &fixture.release.plugin_id,
                            &fixture.release.plugin_version,
                            &fixture.release.artifact_digest
                        )
                        .is_err()
                );
                continue;
            }
            1 => fs::remove_file(
                root.join("trusted-publishers")
                    .join(format!("{}.pub", fixture.release.publisher)),
            )
            .unwrap(),
            2 => fs::write(root.join("victoria.cowboy-plugin"), b"{}").unwrap(),
            _ => {
                let mut release = fixture.release.clone();
                release.signature = "not a signature".to_owned();
                fs::write(
                    root.join("victoria.release.json"),
                    serde_json::to_vec(&release).unwrap(),
                )
                .unwrap();
            }
        }
        assert!(PluginCatalog::inspect(fixture.root.path(), Some(root)).is_err());
    }
}

#[tokio::test]
async fn accepted_catalog_refresh_removes_future_authority_but_failed_refresh_is_atomic() {
    let fixture = Fixture::new(2);
    let catalog = fixture.catalog();
    let control = Arc::new(MachineControl::default());
    let (connection, mut rx) = connect(&control, "hawk", 9, vec![fixture.inventory()]);
    let exporter = fixture.exporter(&control, &catalog);
    let storage = crate::plugin_storage::PluginStorage::sqlite_files(
        crate::plugin_dir::PluginDir::open(fixture.root.path()).unwrap(),
    );
    let marker = fixture.root.path().join("catalog/victoria.release.json");
    fs::write(&marker, b"invalid marker").unwrap();
    assert!(catalog.refresh_with_runtime(&storage).await.is_err());
    assert!(
        deliver(
            &fixture,
            &control,
            &connection,
            &mut rx,
            &exporter,
            Some(Signal::Logs)
        )
        .await
        .logs_delivered
    );
    fs::remove_file(&marker).unwrap();
    assert_eq!(catalog.refresh_with_runtime(&storage).await.unwrap(), 0);
    denied(&exporter, Some(Signal::Logs), &mut rx).await;
}
