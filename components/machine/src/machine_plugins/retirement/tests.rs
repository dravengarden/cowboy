//! Machine browser runtime generation retirement against a registered
//! browser manager's pins.

use super::*;
use cowboy_plugin_sdk::{PluginArtifactProbe, ReleasedPluginComponent};

/// Browser pins as a manager reports them; `None` stands for unknown.
struct Pins(parking_lot::Mutex<Option<BTreeSet<String>>>);

impl BrowserDrain for Pins {
    fn drain<'a>(&'a self, _authority: &'a machine_browser::DrainAuthority<'a>) -> DrainFuture<'a> {
        Box::pin(async { anyhow::bail!("not drained in this test") })
    }

    fn with_generation_pins(&self, retire: &mut GenerationRetirement<'_>) -> Result<()> {
        let pins = self.0.lock().clone().context("pins unknown")?;
        retire(&pins)
    }
}

#[allow(clippy::too_many_lines)]
fn write_signed_active_generation(
    store: &MachinePluginStore,
    plugin_id: &str,
    kind: cowboy_plugin_sdk::PluginKind,
) -> String {
    let (manifest_str, contract_str) = match kind {
        cowboy_plugin_sdk::PluginKind::MachineBrowser => (
            include_str!("../../../../../plugins/machine-browser/plugin.json"),
            include_str!("../../../../../plugins/machine-browser/contract.json"),
        ),
        cowboy_plugin_sdk::PluginKind::CodeIntelligence => (
            include_str!("../../../../../plugins/zed/plugin.json"),
            include_str!("../../../../../plugins/zed/contract.json"),
        ),
        cowboy_plugin_sdk::PluginKind::MemoryBackend => (
            include_str!("../../../../../plugins/matrix/plugin.json"),
            include_str!("../../../../../plugins/matrix/contract.json"),
        ),
        cowboy_plugin_sdk::PluginKind::CallProfile => (
            include_str!("../../../../../plugins/review/plugin.json"),
            include_str!("../../../../../plugins/review/contract.json"),
        ),
        other => panic!("unexpected fixture kind: {other:?}"),
    };
    let mut manifest: cowboy_plugin_sdk::PluginManifest =
        serde_json::from_str(manifest_str).unwrap();
    manifest.id = plugin_id.to_owned();
    let payload = match kind {
        cowboy_plugin_sdk::PluginKind::MachineBrowser => {
            PluginPayload::MachineBrowser(serde_json::from_str(contract_str).unwrap())
        }
        cowboy_plugin_sdk::PluginKind::CodeIntelligence => {
            PluginPayload::CodeIntelligence(serde_json::from_str(contract_str).unwrap())
        }
        cowboy_plugin_sdk::PluginKind::MemoryBackend => {
            PluginPayload::MemoryBackend(serde_json::from_str(contract_str).unwrap())
        }
        cowboy_plugin_sdk::PluginKind::CallProfile => {
            PluginPayload::CallProfile(serde_json::from_str(contract_str).unwrap())
        }
        other => panic!("unexpected fixture kind: {other:?}"),
    };
    let package = PluginPackage::new(
        manifest.clone(),
        manifest.component_release.clone(),
        payload,
    )
    .unwrap();
    let bytes = package.canonical_bytes().unwrap();
    let key_dir = tempfile::tempdir().unwrap();
    let publisher = crate::machine_auth::MachineIdentity::load_or_create(key_dir.path()).unwrap();
    let platforms: Vec<cowboy_provider_sdk::PlatformTarget> = match &package.payload {
        PluginPayload::MachineBrowser(contract) => contract.supported_platforms.clone(),
        PluginPayload::CodeIntelligence(contract) => contract.supported_platforms.clone(),
        PluginPayload::MemoryBackend(contract) => contract.supported_platforms.clone(),
        PluginPayload::CallProfile(contract) => contract.supported_platforms.clone(),
        _ => Vec::new(),
    };
    let mut release = cowboy_plugin_sdk::PluginRelease {
        release_schema: package.minimum_release_schema(),
        plugin_id: manifest.id.clone(),
        plugin_version: manifest.version.clone(),
        plugin_kind: manifest.kind,
        package_digest: PluginPackage::artifact_digest(&bytes),
        artifact_digest: String::new(),
        artifact_url: format!("https://plugins.example.test/{}.cowboy-plugin", manifest.id),
        publisher: manifest.publisher.clone(),
        contract_fingerprint: package.contract_fingerprint.clone(),
        component_release: manifest.component_release,
        host_bundle_digest: None,
        signature: String::new(),
        runtime_artifacts: platforms
            .iter()
            .map(|platform| {
                let components = match &package.payload {
                    PluginPayload::MachineBrowser(contract) => vec![ReleasedPluginComponent {
                        kind: PluginComponentKind::MachineBrowserRuntime,
                        slot: contract.runtime.artifact.clone(),
                        dependency: "cowboy-browser-runtime".to_owned(),
                        version: contract.version.clone(),
                        command: "cowboy-browser-launcher".to_owned(),
                        artifact_url: "https://plugins.example.test/machine-browser-runtime.tar.gz"
                            .to_owned(),
                        artifact_digest: format!("sha256:{}", "1".repeat(64)),
                        artifact_format: PluginArtifactFormat::TarGz,
                        entrypoint: Some(contract.runtime.launcher.clone()),
                        probe: PluginArtifactProbe {
                            args: vec!["--version".to_owned()],
                            timeout_ms: 1_000,
                        },
                    }],
                    PluginPayload::CodeIntelligence(contract) => contract
                        .runtime
                        .as_ref()
                        .map(|runtime| {
                            runtime
                                .components
                                .iter()
                                .map(|component| ReleasedPluginComponent {
                                    kind: component.kind,
                                    slot: component.slot.clone(),
                                    dependency: component.dependency.clone(),
                                    version: component.version.clone(),
                                    command: component.command.clone(),
                                    artifact_url: format!(
                                        "https://plugins.example.test/{}",
                                        component.dependency
                                    ),
                                    artifact_digest: format!("sha256:{}", "2".repeat(64)),
                                    artifact_format: PluginArtifactFormat::Raw,
                                    entrypoint: None,
                                    probe: PluginArtifactProbe {
                                        args: vec!["--help".to_owned()],
                                        timeout_ms: 1_000,
                                    },
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                PluginRuntimeArtifacts {
                    os: platform.os.clone(),
                    architecture: platform.architecture.clone(),
                    components,
                }
            })
            .collect(),
        supported_platforms: platforms,
    };
    release.artifact_digest = release.computed_artifact_digest().unwrap();
    release.signature = publisher
        .sign_namespaced(PLUGIN_RELEASE_SIGNATURE_NAMESPACE, &release.proof())
        .unwrap();
    let gen_name = digest_generation_name(&release.artifact_digest).unwrap();
    let content = store
        .plugin_root(plugin_id)
        .join("generations")
        .join(&gen_name)
        .join("content");
    fs::create_dir_all(&content).unwrap();
    fs::write(content.join("package.cowboy-plugin"), &bytes).unwrap();
    fs::write(
        content.join("plugin-release.json"),
        serde_json::to_vec(&release).unwrap(),
    )
    .unwrap();
    fs::write(content.join("publisher.pub"), publisher.public_key()).unwrap();
    let inventory = PluginInventory {
        plugin_id: manifest.id,
        plugin_version: manifest.version,
        plugin_kind: manifest.kind,
        generation_digest: release.artifact_digest,
        installation_revision: None,
        contract_fingerprint: package.contract_fingerprint,
        state: PluginInstallationState::Active,
        rollback_generation_digest: None,
        active_session_leases: 0,
        auth_generation: None,
        replica_state: ProviderReplicaState::Absent,
        materialization_state: ProviderMaterializationState::NotInstalled,
        detail: None,
    };
    fs::write(
        content.join("plugin-inventory.json"),
        serde_json::to_vec(&inventory).unwrap(),
    )
    .unwrap();
    gen_name
}

fn name(letter: &str) -> String {
    letter.repeat(64)
}

fn digest(letter: &str) -> String {
    format!("sha256:{}", name(letter))
}

/// A browser Plugin with signed active generation plus rollback `b` and `c..e`.
fn browser_store() -> (
    tempfile::TempDir,
    MachinePluginStore,
    std::sync::Arc<Pins>,
    String,
) {
    let root = tempfile::tempdir().unwrap();
    let store = MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".to_owned()).unwrap();
    let active_gen = write_signed_active_generation(
        &store,
        "machine-browser",
        cowboy_plugin_sdk::PluginKind::MachineBrowser,
    );
    let generations = store.plugin_root("machine-browser").join("generations");
    for letter in ["b", "c", "d", "e"] {
        let content = generations.join(name(letter)).join("content");
        fs::create_dir_all(&content).unwrap();
        let inventory = serde_json::json!({
            "plugin_id": "machine-browser",
            "plugin_version": "1.0.0",
            "plugin_kind": cowboy_plugin_sdk::PluginKind::MachineBrowser,
            "generation_digest": digest(letter),
            "contract_fingerprint": "sha256:fixture",
            "state": PluginInstallationState::Active,
        });
        fs::write(
            content.join("plugin-inventory.json"),
            serde_json::to_vec(&inventory).unwrap(),
        )
        .unwrap();
    }
    let plugin = store.plugin_root("machine-browser");
    restore_generation_link(&plugin.join("active"), Some(&active_gen)).unwrap();
    restore_generation_link(&plugin.join("rollback"), Some(&name("b"))).unwrap();
    let pins = std::sync::Arc::new(Pins(parking_lot::Mutex::new(None)));
    // The weak reference stays live as long as the returned `pins`.
    let drain: std::sync::Arc<dyn BrowserDrain> = pins.clone();
    store.register_browser_drain(std::sync::Arc::downgrade(&drain));
    (root, store, pins, active_gen)
}

fn present(store: &MachinePluginStore) -> Vec<String> {
    let mut names: Vec<String> =
        fs::read_dir(store.plugin_root("machine-browser").join("generations"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
    names.sort();
    names
}

#[tokio::test]
async fn installed_browser_generations_keep_active_rollback_and_pinned_ones() {
    let (_root, store, pins, active_gen) = browser_store();
    // Unknown pins retire nothing, directly or from the periodic sweep.
    assert!(
        store
            .retire_generations("machine-browser", &BTreeSet::new())
            .await
            .is_err()
    );
    assert!(
        store
            .retire_uninstalled_generations(&BTreeMap::new(), &BTreeSet::new())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(present(&store).len(), 5);
    // A running instance pins `c`; `d` and `e` retire.
    *pins.0.lock() = Some(BTreeSet::from([digest("c")]));
    let outcome = store
        .retire_generations("machine-browser", &BTreeSet::new())
        .await
        .unwrap();
    assert_eq!(outcome.retired, vec![digest("d"), digest("e")]);
    let mut expected_retained = vec![format!("sha256:{active_gen}"), digest("b"), digest("c")];
    expected_retained.sort();
    assert_eq!(outcome.retained, expected_retained);
    let mut expected_present = vec![active_gen.clone(), name("b"), name("c")];
    expected_present.sort();
    assert_eq!(present(&store), expected_present);
    // Once that instance exits, the periodic sweep retires `c` too.
    *pins.0.lock() = Some(BTreeSet::new());
    let outcomes = store
        .retire_uninstalled_generations(&BTreeMap::new(), &BTreeSet::new())
        .await
        .unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].retired, vec![digest("c")]);
    let mut expected_ab = vec![active_gen, name("b")];
    expected_ab.sort();
    assert_eq!(present(&store), expected_ab);
}

#[tokio::test]
async fn uninstalled_browser_generations_retire_once_no_instance_pins_them() {
    let (_root, store, pins, active_gen) = browser_store();
    let plugin = store.plugin_root("machine-browser");
    fs::remove_file(plugin.join("active")).unwrap();
    // An instance still runs from the removed active generation.
    *pins.0.lock() = Some(BTreeSet::from([format!("sha256:{active_gen}")]));
    // The uninstall saga may still compensate: untouched.
    let skip = BTreeSet::from(["machine-browser".to_owned()]);
    assert!(
        store
            .retire_uninstalled_generations(&BTreeMap::new(), &skip)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(present(&store).len(), 5);
    let outcomes = store
        .retire_uninstalled_generations(&BTreeMap::new(), &BTreeSet::new())
        .await
        .unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        outcomes[0].retired,
        ["b", "c", "d", "e"].map(digest).to_vec()
    );
    assert_eq!(present(&store), vec![active_gen]);
    // The rollback link pointed at a retired generation and is removed.
    assert!(plugin.join("rollback").symlink_metadata().is_err());
    // The instance exited: nothing is left.
    *pins.0.lock() = Some(BTreeSet::new());
    store
        .retire_uninstalled_generations(&BTreeMap::new(), &BTreeSet::new())
        .await
        .unwrap();
    assert!(present(&store).is_empty());
}

#[tokio::test]
async fn code_and_session_extension_generations_keep_active_rollback_and_retire_old() {
    let root = tempfile::tempdir().unwrap();
    let store = MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".to_owned()).unwrap();
    for (plugin_id, kind) in [
        ("zed", cowboy_plugin_sdk::PluginKind::CodeIntelligence),
        ("matrix", cowboy_plugin_sdk::PluginKind::MemoryBackend),
        ("review", cowboy_plugin_sdk::PluginKind::CallProfile),
    ] {
        let active_gen = write_signed_active_generation(&store, plugin_id, kind);
        let active_digest = format!("sha256:{active_gen}");
        let generations = store.plugin_root(plugin_id).join("generations");
        for letter in ["b", "c", "d"] {
            let content = generations.join(name(letter)).join("content");
            fs::create_dir_all(&content).unwrap();
            let inventory = serde_json::json!({
                "plugin_id": plugin_id,
                "plugin_version": "1.0.0",
                "plugin_kind": kind,
                "generation_digest": digest(letter),
                "contract_fingerprint": "sha256:fixture",
                "state": PluginInstallationState::Active,
            });
            fs::write(
                content.join("plugin-inventory.json"),
                serde_json::to_vec(&inventory).unwrap(),
            )
            .unwrap();
        }
        let plugin = store.plugin_root(plugin_id);
        restore_generation_link(&plugin.join("active"), Some(&active_gen)).unwrap();
        restore_generation_link(&plugin.join("rollback"), Some(&name("b"))).unwrap();

        let outcome = store
            .retire_generations(plugin_id, &BTreeSet::new())
            .await
            .unwrap();
        assert_eq!(outcome.retired, vec![digest("c"), digest("d")]);
        let mut expected_kept = vec![active_digest.clone(), digest("b")];
        expected_kept.sort();
        assert_eq!(outcome.retained, expected_kept);

        fs::remove_file(plugin.join("active")).unwrap();
        let skip = BTreeSet::from([plugin_id.to_owned()]);
        assert!(
            store
                .retire_uninstalled_generations(&BTreeMap::new(), &skip)
                .await
                .unwrap()
                .is_empty()
        );
        let outcomes = store
            .retire_uninstalled_generations(&BTreeMap::new(), &BTreeSet::new())
            .await
            .unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].plugin_id, plugin_id);
        assert_eq!(outcomes[0].retired, expected_kept);
        assert_eq!(fs::read_dir(&generations).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn browser_retirement_preserves_historical_code_generations_and_finishes_tombstones() {
    let (_root, store, pins, _active_gen) = browser_store();
    let generations = store.plugin_root("machine-browser").join("generations");
    // Place a historical CodeIntelligence generation inside the same slot and a
    // stranded tombstone colliding with generation `c`.
    let code_gen = name("f");
    let code_content = generations.join(&code_gen).join("content");
    fs::create_dir_all(&code_content).unwrap();
    fs::write(
        code_content.join("plugin-inventory.json"),
        serde_json::to_vec(&serde_json::json!({
            "plugin_id": "machine-browser",
            "plugin_version": "1.0.0",
            "plugin_kind": cowboy_plugin_sdk::PluginKind::CodeIntelligence,
            "generation_digest": digest("f"),
            "contract_fingerprint": "sha256:fixture",
            "state": PluginInstallationState::Active,
        }))
        .unwrap(),
    )
    .unwrap();
    let colliding = generations.join(format!("{RETIRED_GENERATION_PREFIX}{}", name("c")));
    fs::create_dir_all(&colliding).unwrap();

    *pins.0.lock() = Some(BTreeSet::new());
    let outcome = store
        .retire_generations("machine-browser", &BTreeSet::new())
        .await
        .unwrap();
    assert!(outcome.retired.contains(&digest("c")));
    assert!(outcome.retained.contains(&digest("f")));
    assert!(generations.join(&code_gen).is_dir());
    assert!(!colliding.exists());

    // An uninstalled plugin with only a stranded tombstone still cleans it up.
    let stranded_dir = store
        .plugin_root("stranded")
        .join("generations")
        .join(format!("{RETIRED_GENERATION_PREFIX}{}", name("9")));
    fs::create_dir_all(&stranded_dir).unwrap();
    store
        .retire_uninstalled_generations(&BTreeMap::new(), &BTreeSet::new())
        .await
        .unwrap();
    assert!(!stranded_dir.exists());

    // Missing `plugin_kind` in plugin-inventory.json must NOT default to
    // AgentProvider, and an unsigned package.cowboy-provider without verified
    // release/signature is also preserved.
    let mixed = store.plugin_root("legacy-mixed").join("generations");
    let omitted_kind = mixed.join(name("1")).join("content");
    let unsigned_legacy = mixed.join(name("2")).join("content");
    fs::create_dir_all(&omitted_kind).unwrap();
    fs::create_dir_all(&unsigned_legacy).unwrap();
    fs::write(
        omitted_kind.join("plugin-inventory.json"),
        serde_json::to_vec(&serde_json::json!({
            "plugin_id": "legacy-mixed",
            "plugin_version": "1.0.0",
            "generation_digest": digest("1"),
            "contract_fingerprint": "sha256:fixture",
            "state": PluginInstallationState::Active,
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(unsigned_legacy.join("package.cowboy-provider"), b"{}").unwrap();
    let outcomes = store
        .retire_uninstalled_generations(&BTreeMap::new(), &BTreeSet::new())
        .await
        .unwrap();
    assert!(outcomes.is_empty());
    assert!(omitted_kind.exists());
    assert!(unsigned_legacy.exists());

    // Even when a slot is currently active as an excluded kind (Telemetry),
    // stranded validated tombstones are still swept while live gens stay untouched.
    let telemetry_root = store.plugin_root("telem-slot");
    let telem_active = telemetry_root
        .join("generations")
        .join(name("7"))
        .join("content");
    fs::create_dir_all(&telem_active).unwrap();
    fs::write(
        telem_active.join("plugin-inventory.json"),
        serde_json::to_vec(&serde_json::json!({
            "plugin_id": "telem-slot",
            "plugin_version": "1.0.0",
            "plugin_kind": cowboy_plugin_sdk::PluginKind::TelemetryBackend,
            "generation_digest": digest("7"),
            "contract_fingerprint": "sha256:fixture",
            "state": PluginInstallationState::Active,
        }))
        .unwrap(),
    )
    .unwrap();
    restore_generation_link(&telemetry_root.join("active"), Some(&name("7"))).unwrap();
    let telem_tombstone = telemetry_root
        .join("generations")
        .join(format!("{RETIRED_GENERATION_PREFIX}{}-123-4", name("8")));
    fs::create_dir_all(&telem_tombstone).unwrap();
    store
        .retire_uninstalled_generations(&BTreeMap::new(), &BTreeSet::new())
        .await
        .unwrap();
    assert!(!telem_tombstone.exists());
    assert!(telem_active.exists());
}
