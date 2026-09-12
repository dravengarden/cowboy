//! Real signed install, JSON frames, CLI admission and SQL coordinator; the
//! socket and explicit local Operator are hermetic, not a live deployment test.

use super::*;
use crate::machine_plugins::{MachinePluginStore, PluginExecutionScope};
use crate::machine_protocol::Platform;
use crate::machine_protocol::telemetry_binding::{BindingChange, BindingInstallation};
use crate::machine_protocol::{MachineCommand, MachineEvent, MachineFrame, PluginInventory};
use crate::server::telemetry_binding::tests::FixtureEffects;
use base64::Engine as _;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::sync::atomic::AtomicUsize;
use tokio::sync::mpsc;

struct Fixture {
    root: tempfile::TempDir,
    machine: Arc<MachinePluginStore>,
    store: Store,
    control: Arc<MachineControl>,
    catalog: Arc<PluginCatalog>,
    fences: crate::server::PluginLifecycleFences,
    connection: ConnectionToken,
    installed: PluginInventory,
    sends: Arc<AtomicUsize>,
    queries: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn wire(frame: MachineFrame) -> MachineFrame {
    serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap()
}

impl Fixture {
    async fn new(writer: bool, lose_ack: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let publisher =
            crate::machine_auth::MachineIdentity::load_or_create(&root.path().join("publisher"))
                .unwrap();
        let desired = crate::machine_plugins::telemetry_release_for_test(&publisher, "1.1.0");
        let machine_root = root.path().join("machine");
        let machine = Arc::new(
            MachinePluginStore::new(&machine_root, Platform::Linux, "x86_64".into()).unwrap(),
        );
        machine.enable_installation_tracking().await.unwrap();
        let installed = machine.install(&desired).await.unwrap();
        if writer {
            machine.enable_binding_writer_for_test();
        }
        let policy = machine_root.join("telemetry.json");
        fs::write(&policy, serde_json::to_vec(&serde_json::json!({
            "plugin": {"plugin_id": installed.plugin_id, "plugin_version": installed.plugin_version, "generation_digest": installed.generation_digest},
            "logs": {"base_url": "http://127.0.0.1:1", "bearer_token": "fixture-only"},
            "metrics": null, "traces": null
        })).unwrap()).unwrap();
        fs::set_permissions(&policy, fs::Permissions::from_mode(0o600)).unwrap();
        let catalog_root = root.path().join("catalog");
        fs::create_dir_all(catalog_root.join("trusted-publishers")).unwrap();
        fs::write(
            catalog_root
                .join("trusted-publishers")
                .join(format!("{}.pub", desired.release.publisher)),
            desired.publisher_public_key,
        )
        .unwrap();
        fs::write(
            catalog_root.join("victoria.cowboy-plugin"),
            base64::engine::general_purpose::STANDARD
                .decode(desired.package_base64)
                .unwrap(),
        )
        .unwrap();
        fs::write(
            catalog_root.join("victoria.release.json"),
            serde_json::to_vec(&desired.release).unwrap(),
        )
        .unwrap();
        let mut catalog = PluginCatalog::open(root.path(), Some(catalog_root)).unwrap();
        let mut policy = crate::plugin_activation::HostActivationPolicy::default();
        policy.source = crate::plugin_activation::HostSourcePolicy::CatalogOnly;
        catalog.configure_hosts(policy).unwrap();
        let catalog = Arc::new(catalog);
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let control = Arc::new(MachineControl::default());
        let (tx, mut commands) = mpsc::unbounded_channel();
        let connection =
            control.install("machine-test".into(), "fixture-epoch".into(), false, 16, tx);
        control.record_remote(
            &connection,
            MachineEvent::PluginInventory {
                plugins: vec![installed.clone()],
                observed_at_ms: 1,
            },
        );
        let sends = Arc::new(AtomicUsize::new(0));
        let queries = Arc::new(AtomicUsize::new(0));
        let task = {
            let (control, connection, machine, sends, queries) = (
                control.clone(),
                connection.clone(),
                machine.clone(),
                sends.clone(),
                queries.clone(),
            );
            tokio::spawn(async move {
                let scope = PluginExecutionScope::new(Some("service-test"), "machine-test");
                let (events, mut replies) = mpsc::unbounded_channel();
                while let Some(command) = commands.recv().await {
                    let MachineFrame::Command { command } = wire(MachineFrame::Command { command })
                    else {
                        unreachable!()
                    };
                    let event = match command {
                        MachineCommand::ExportBoundTelemetry {
                            request_id,
                            attempt,
                        } => {
                            crate::machine_cli::telemetry_export::export(
                                request_id,
                                *attempt,
                                machine.clone(),
                                &scope,
                                events.clone(),
                            );
                            replies.recv().await.unwrap()
                        }
                        MachineCommand::CommitTelemetryBinding { request_id, step } => {
                            sends.fetch_add(1, Ordering::Relaxed);
                            crate::machine_cli::telemetry_binding::commit(
                                request_id,
                                *step,
                                machine.clone(),
                                &scope,
                                events.clone(),
                            );
                            let event = replies.recv().await.unwrap();
                            if lose_ack {
                                continue;
                            }
                            event
                        }
                        MachineCommand::QueryTelemetryBinding { request_id, step } => {
                            queries.fetch_add(1, Ordering::Relaxed);
                            let observation = machine
                                .telemetry_binding_observation(
                                    &step,
                                    Some("service-test"),
                                    "machine-test",
                                )
                                .await;
                            MachineEvent::TelemetryBindingObservation {
                                request_id,
                                observation: Box::new(observation),
                            }
                        }
                        other => panic!("unexpected fallback/egress command: {other:?}"),
                    };
                    let MachineFrame::Event { event } = wire(MachineFrame::Event { event }) else {
                        unreachable!()
                    };
                    control.record_remote(&connection, event);
                }
            })
        };
        Self {
            root,
            machine,
            store,
            control,
            catalog,
            fences: Default::default(),
            connection,
            installed,
            sends,
            queries,
            task,
        }
    }

    fn select(&self) -> Intent {
        let mut intent = crate::telemetry_binding::fixture("wire-select");
        intent.schema = 2;
        intent.actor = crate::plugin_operation::Actor::Product {
            user_id: crate::product_auth::local_product_principal().user_id,
        };
        intent.change = BindingChange::Select {
            installation: BindingInstallation {
                plugin_id: self.installed.plugin_id.clone(),
                plugin_version: self.installed.plugin_version.clone(),
                generation_digest: self.installed.generation_digest.clone().try_into().unwrap(),
                installation_revision: self.installed.installation_revision.clone().unwrap(),
                contract_fingerprint: self
                    .installed
                    .contract_fingerprint
                    .clone()
                    .try_into()
                    .unwrap(),
            },
            policy_epoch: "1".to_owned().try_into().unwrap(),
        };
        intent
    }

    fn bind(&self, intent: &Intent) -> Result<LiveEffects> {
        LiveEffects::bind(
            self.control.clone(),
            self.catalog.clone(),
            self.fences.clone(),
            intent,
        )
    }

    async fn coordinate(&self, intent: &Intent) -> Operation {
        let fence = LegacyFence::recover(Some(&self.store), &intent.service_id)
            .await
            .unwrap();
        let authority = FixtureEffects::new(intent);
        let effects = self.bind(intent).unwrap();
        let op = super::super::coordinate(
            &self.store,
            &fence,
            intent,
            authority.confirmation(),
            &effects,
        )
        .await
        .unwrap();
        assert!(!fence.allows_legacy());
        op
    }

    fn publish(&self, plugins: Vec<PluginInventory>) {
        self.control.record_remote(
            &self.connection,
            MachineEvent::PluginInventory {
                plugins,
                observed_at_ms: 2,
            },
        );
    }
}

mod export;
mod resolution;

#[tokio::test]
async fn finite_wire_select_revoke_restore_recovers_evidence_without_replay() {
    let fixture = Fixture::new(true, false).await;
    let select = fixture.select();
    let mut revoke = select.clone();
    revoke.operation_id = "wire-revoke-operation".into();
    revoke.expected = Some(select.machine_step().unwrap().after().unwrap());
    revoke.change = BindingChange::Revoke {
        policy_epoch: "2".to_owned().try_into().unwrap(),
    };
    let mut restore = revoke.clone();
    restore.operation_id = "wire-restore-operation".into();
    restore.expected = Some(revoke.machine_step().unwrap().after().unwrap());
    restore.change = BindingChange::Restore {
        forward_request_digest: revoke.machine_step().unwrap().request_digest().unwrap(),
        selection: revoke.expected.as_ref().unwrap().selection.clone(),
        policy_epoch: "3".to_owned().try_into().unwrap(),
    };
    for intent in [&select, &revoke, &restore] {
        let result = fixture.coordinate(intent).await;
        assert!(
            matches!(result.progress, Progress::Completed { .. }),
            "{result:?}"
        );
        let ledger = fixture
            .store
            .telemetry_binding_ledger(&intent.service_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            ledger.current,
            Some(intent.machine_step().unwrap().after().unwrap())
        );
        let bytes = serde_json::to_vec(&ledger).unwrap();
        let reopened = crate::telemetry_binding::Ledger::decode(
            std::str::from_utf8(&bytes).unwrap(),
            &intent.service_id,
        )
        .unwrap();
        assert_eq!(reopened.current, ledger.current);
        assert!(!String::from_utf8(bytes).unwrap().contains("fixture-only"));
    }
    let path = fixture
        .root
        .path()
        .join("machine/plugin-operations/telemetry-bindings-v1.json");
    let bytes = fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("fixture-only"));
    // Historical duplicate need not match the current head and is never sent.
    assert!(matches!(
        fixture.coordinate(&select).await.progress,
        Progress::Completed { .. }
    ));
    assert_eq!(fixture.sends.load(Ordering::Relaxed), 3);
    assert_eq!(fixture.queries.load(Ordering::Relaxed), 3);
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[tokio::test]
async fn lost_wire_ack_queries_original_request_without_resending() {
    let fixture = Fixture::new(true, true).await;
    // Exercise the real 45-second timeout and original Operator budget. Do not
    // change Cargo features (and hence detached worker generation) for a clock.
    let select = fixture.select();
    let result = fixture.coordinate(&select).await;
    assert!(
        matches!(result.progress, Progress::Completed { .. }),
        "{result:?}"
    );
    assert_eq!(fixture.sends.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.queries.load(Ordering::Relaxed), 2);
    assert_eq!(fixture.machine.inventory().unwrap().len(), 1);
}

#[tokio::test]
async fn production_capture_and_machine_writer_remain_closed() {
    let fixture = Fixture::new(false, false).await;
    let select = fixture.select();
    assert!(
        LiveEffects::capture(
            fixture.control.clone(),
            fixture.catalog.clone(),
            fixture.fences.clone(),
            &select
        )
        .is_err()
    );
    assert!(
        fixture
            .store
            .telemetry_binding_ledger(&select.service_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(fixture.queries.load(Ordering::Relaxed), 0);
    let effects = fixture.bind(&select).unwrap();
    assert!(
        effects
            .dispatch(&select.machine_step().unwrap())
            .await
            .is_err()
    );
    assert!(
        !fixture
            .root
            .path()
            .join("machine/plugin-operations/telemetry-bindings-v1.json")
            .exists()
    );
    assert!(
        fixture
            .store
            .telemetry_binding_ledger(&select.service_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn exact_target_fences_and_original_connection_are_sticky() {
    let fixture = Fixture::new(true, false).await;
    let select = fixture.select();
    for boundary in ["installation", "contract", "fence", "absent"] {
        let effects = fixture.bind(&select).unwrap();
        let mut changed = fixture.installed.clone();
        match boundary {
            "installation" => changed.installation_revision = None,
            "contract" => changed.contract_fingerprint = format!("sha256:{}", "a".repeat(64)),
            "fence" => {
                fixture.fences.write().insert(
                    (
                        select.machine_id.clone(),
                        fixture.installed.plugin_id.clone(),
                    ),
                    crate::server::PluginFenceState::Installing,
                );
            }
            "absent" => {}
            _ => unreachable!(),
        }
        fixture.publish(if boundary == "absent" {
            vec![]
        } else {
            vec![changed]
        });
        assert!(!effects.authorized(&select).await, "{boundary}");
        fixture.fences.write().clear();
        fixture.publish(vec![fixture.installed.clone()]);
        assert!(
            !effects.authorized(&select).await,
            "repair cannot revive {boundary}"
        );
    }
    let effects = fixture.bind(&select).unwrap();
    fixture.control.disconnect("machine-test");
    let (sender, _receiver) = mpsc::unbounded_channel();
    fixture.control.install(
        "machine-test".into(),
        "replacement".into(),
        false,
        15,
        sender,
    );
    assert!(!effects.authorized(&select).await);
    assert!(
        effects
            .observe(&select.machine_step().unwrap())
            .await
            .is_err()
    );
    assert_eq!(fixture.sends.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn revoke_does_not_require_a_removed_installation() {
    let fixture = Fixture::new(true, false).await;
    let select = fixture.select();
    fixture.coordinate(&select).await;
    let mut revoke = select.clone();
    revoke.operation_id = "wire-removed-revoke".into();
    revoke.expected = Some(select.machine_step().unwrap().after().unwrap());
    revoke.change = BindingChange::Revoke {
        policy_epoch: "2".to_owned().try_into().unwrap(),
    };
    fixture.publish(vec![]);
    fixture.fences.write().insert(
        (
            select.machine_id.clone(),
            fixture.installed.plugin_id.clone(),
        ),
        crate::server::PluginFenceState::Uninstalled,
    );
    assert!(matches!(
        fixture.coordinate(&revoke).await.progress,
        Progress::Completed { .. }
    ));
}
