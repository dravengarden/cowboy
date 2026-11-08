//! Machine browser runtime generation retirement against a registered
//! browser manager's pins.

use super::*;

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

fn name(letter: &str) -> String {
    letter.repeat(64)
}

fn digest(letter: &str) -> String {
    format!("sha256:{}", name(letter))
}

/// A browser Plugin with generations a..e, `a` active and `b` rollback.
fn browser_store() -> (tempfile::TempDir, MachinePluginStore, std::sync::Arc<Pins>) {
    let root = tempfile::tempdir().unwrap();
    let store = MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".to_owned()).unwrap();
    let generations = store.plugin_root("machine-browser").join("generations");
    for letter in ["a", "b", "c", "d", "e"] {
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
    restore_generation_link(&plugin.join("active"), Some(&name("a"))).unwrap();
    restore_generation_link(&plugin.join("rollback"), Some(&name("b"))).unwrap();
    let pins = std::sync::Arc::new(Pins(parking_lot::Mutex::new(None)));
    // The weak reference stays live as long as the returned `pins`.
    let drain: std::sync::Arc<dyn BrowserDrain> = pins.clone();
    store.register_browser_drain(std::sync::Arc::downgrade(&drain));
    (root, store, pins)
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
    let (_root, store, pins) = browser_store();
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
    assert_eq!(
        outcome.retained,
        vec![digest("a"), digest("b"), digest("c")]
    );
    assert_eq!(present(&store), vec![name("a"), name("b"), name("c")]);
    // Once that instance exits, the periodic sweep retires `c` too.
    *pins.0.lock() = Some(BTreeSet::new());
    let outcomes = store
        .retire_uninstalled_generations(&BTreeMap::new(), &BTreeSet::new())
        .await
        .unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].retired, vec![digest("c")]);
    assert_eq!(present(&store), vec![name("a"), name("b")]);
}

#[tokio::test]
async fn uninstalled_browser_generations_retire_once_no_instance_pins_them() {
    let (_root, store, pins) = browser_store();
    let plugin = store.plugin_root("machine-browser");
    fs::remove_file(plugin.join("active")).unwrap();
    // An instance still runs from the removed active generation.
    *pins.0.lock() = Some(BTreeSet::from([digest("a")]));
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
    assert_eq!(present(&store), vec![name("a")]);
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
