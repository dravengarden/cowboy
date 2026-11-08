//! Runtime generation pins reported to Plugin generation retirement.

use super::*;

/// A plan whose launcher exists under `root/<digest>`.
fn plan_at(root: &Path, digest: &str) -> MachineBrowserRuntimePlan {
    let launcher = root.join(digest).join("cowboy-browser-launcher");
    std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    std::fs::write(&launcher, b"launcher").unwrap();
    MachineBrowserRuntimePlan {
        generation_digest: format!("sha256:{digest}"),
        launcher,
        ..plan()
    }
}

fn pinned(manager: &BrowserManager) -> Result<BTreeSet<String>> {
    let mut seen = BTreeSet::new();
    manager.retire_unpinned(&mut |pins| {
        seen.clone_from(pins);
        Ok(())
    })?;
    Ok(seen)
}

#[tokio::test]
async fn running_starting_and_retained_instances_pin_their_generations() {
    let fixture = fixture().await;
    // Before a restart inventory completed nothing is known.
    assert!(pinned(&fixture.manager).is_err());
    fixture.manager.pins.inventoried();
    // The registered instance pins the generation it launched from.
    assert_eq!(
        pinned(&fixture.manager).unwrap(),
        BTreeSet::from(["sha256:aa".to_owned()])
    );
    // An open that resolved a runtime pins it until it returns.
    let root = tempfile::tempdir().unwrap();
    let starting = plan_at(root.path(), "bb");
    let pin = fixture.manager.pins.pin(&starting).unwrap();
    assert_eq!(
        fixture.manager.pins.starting(),
        vec!["sha256:bb".to_owned()]
    );
    // A unit retained until its exit is confirmed pins its generation.
    let mut retained = Unconfirmed::unit("cowboy-browser-x", Some(&fixture.account));
    retained.generation = Some("sha256:cc".into());
    fixture.manager.unconfirmed.lock().insert(1, retained);
    assert_eq!(
        pinned(&fixture.manager).unwrap(),
        ["sha256:aa", "sha256:bb", "sha256:cc"]
            .map(ToOwned::to_owned)
            .into()
    );
    drop(pin);
    assert!(fixture.manager.pins.starting().is_empty());
    // A unit of unknown generation may run from any of them.
    fixture
        .manager
        .unconfirmed
        .lock()
        .insert(2, Unconfirmed::unit("cowboy-browser-y", None));
    assert!(pinned(&fixture.manager).is_err());
    fixture.manager.unconfirmed.lock().clear();
    // An incomplete inventory pass hides surviving units' pins again.
    fixture
        .manager
        .inventory_pending
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(pinned(&fixture.manager).is_err());
}

#[tokio::test]
async fn a_start_cannot_pin_a_generation_retired_after_it_was_resolved() {
    let fixture = fixture().await;
    let root = tempfile::tempdir().unwrap();
    let plan = plan_at(root.path(), "bb");
    std::fs::remove_dir_all(root.path().join("bb")).unwrap();
    assert!(fixture.manager.pins.pin(&plan).is_none());
    assert!(fixture.manager.pins.starting().is_empty());
}
