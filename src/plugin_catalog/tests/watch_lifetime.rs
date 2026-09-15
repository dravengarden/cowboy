//! Actual signed Catalog reader/runtime fixtures behind the owned observer.
use super::*;
use std::time::Duration;

async fn observed(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("observer must reconcile a real published candidate");
}

#[tokio::test]
async fn signed_publication_is_adopted_and_a_rejected_candidate_keeps_original_leases() {
    let root = ReaderFixture::new();
    let first = publish_auth_fixture(&root.0, "google", "1.0.0", |_| {});
    let catalog = Arc::new(PluginCatalog::open(&root.0, Some(root.0.join("external"))).unwrap());
    let storage = crate::plugin_storage::PluginStorage::sqlite_files(catalog.plugin_dir.clone());
    let providers = Arc::new(
        crate::provider_catalog::ProviderCatalog::open(&root.0, Arc::clone(&catalog)).unwrap(),
    );
    let lease = catalog
        .resolve_verified_exact("google", "1.0.0", &first.artifact_digest)
        .unwrap();
    let (_service, shutdown) = tokio::sync::watch::channel(false);
    let observer = spawn_catalog_watcher(Arc::clone(&catalog), storage, providers, shutdown);
    observed(|| catalog.runtime().is_some()).await;

    let second = publish_auth_fixture(&root.0, "google", "2.0.0", |_| {});
    observed(|| {
        catalog
            .resolve_verified_exact("google", "2.0.0", &second.artifact_digest)
            .is_ok()
    })
    .await;
    assert!(lease.current(&catalog));
    let accepted = catalog.state.read().clone();
    let marker = root.0.join("external/google-2.0.0.release.json");
    let bytes = fs::read(&marker).unwrap();
    let mut forged = second.clone();
    forged.signature = "synthetic invalid signature".to_owned();
    fs::write(&marker, serde_json::to_vec(&forged).unwrap()).unwrap();
    assert!(
        catalog.load_external().is_err(),
        "real signature reader rejects candidate"
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(Arc::ptr_eq(&accepted, &catalog.state.read()));
    assert!(lease.current(&catalog));

    fs::write(&marker, bytes).unwrap();
    let third = publish_auth_fixture(&root.0, "google", "3.0.0", |_| {});
    observed(|| {
        catalog
            .resolve_verified_exact("google", "3.0.0", &third.artifact_digest)
            .is_ok()
    })
    .await;
    assert!(lease.current(&catalog));
    observer.shutdown().await.unwrap();
}

#[tokio::test]
async fn provider_failure_after_catalog_commit_recovers_from_selected_legacy_root() {
    let root = ReaderFixture::new();
    publish_auth_fixture(&root.0, "google", "1.0.0", |_| {});
    fs::rename(root.0.join("external"), root.0.join("plugin-catalog")).unwrap();
    let catalog = Arc::new(PluginCatalog::open(&root.0, None).unwrap());
    let providers = Arc::new(
        crate::provider_catalog::ProviderCatalog::open(&root.0, Arc::clone(&catalog)).unwrap(),
    );
    let legacy = providers.legacy_catalog_root().unwrap();
    assert_eq!(legacy, root.0.join("provider-catalog"));
    fs::create_dir_all(legacy).unwrap();
    let bad = legacy.join("invalid.cowboy-provider");
    let marker = legacy.join("invalid.release.json");
    fs::write(&bad, b"synthetic invalid package").unwrap();
    fs::write(&marker, b"{}").unwrap();
    let storage = crate::plugin_storage::PluginStorage::sqlite_files(catalog.plugin_dir.clone());
    let before = catalog.state.read().clone();
    let projection = serde_json::to_value(providers.entries()).unwrap();
    let (_service, shutdown) = tokio::sync::watch::channel(false);
    let observer = spawn_catalog_watcher(
        Arc::clone(&catalog),
        storage,
        Arc::clone(&providers),
        shutdown,
    );
    observed(|| !Arc::ptr_eq(&before, &catalog.state.read())).await;
    assert!(providers.refresh_external().is_err());
    assert_eq!(
        serde_json::to_value(providers.entries()).unwrap(),
        projection
    );
    let committed = catalog.state.read().clone();
    // The first Catalog commit was not rolled back with the Provider error.
    // Clearing only this already-selected legacy input must permit a retry.
    fs::remove_file(bad).unwrap();
    fs::remove_file(marker).unwrap();
    observed(|| !Arc::ptr_eq(&committed, &catalog.state.read())).await;
    assert!(providers.refresh_external().is_ok());
    observer.shutdown().await.unwrap();
}
