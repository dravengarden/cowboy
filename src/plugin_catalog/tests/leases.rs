use super::*;

struct Fixture {
    root: ReaderFixture,
    catalog: PluginCatalog,
    storage: crate::plugin_storage::PluginStorage,
    release: PluginRelease,
}

impl Fixture {
    fn new() -> Self {
        let root = ReaderFixture::new();
        let release = publish_auth_fixture(&root.0, "google", "1.0.0", |_| {});
        let catalog = PluginCatalog::open(&root.0, Some(root.0.join("external"))).unwrap();
        let storage =
            crate::plugin_storage::PluginStorage::sqlite_files(catalog.plugin_dir.clone());
        Self {
            root,
            catalog,
            storage,
            release,
        }
    }

    fn resolve(&self) -> VerifiedPluginRelease {
        self.catalog
            .resolve_verified_exact(
                &self.release.plugin_id,
                &self.release.plugin_version,
                &self.release.artifact_digest,
            )
            .unwrap()
    }

    fn marker(&self) -> PathBuf {
        self.root.0.join("external/google.release.json")
    }

    async fn refresh(&self) {
        self.catalog
            .refresh_with_runtime(&self.storage)
            .await
            .unwrap();
    }
}

#[test]
fn release_lease_requires_an_exact_signed_entry_and_owns_immutable_bytes() {
    let f = Fixture::new();
    for (id, version, digest) in [
        ("unknown", "1.0.0", f.release.artifact_digest.as_str()),
        ("google", "99.0.0", f.release.artifact_digest.as_str()),
        ("google", "1.0.0", "sha256:untrusted"),
        ("google", "", ""),
        ("codex", "1.0.0", ""),
    ] {
        assert!(
            f.catalog
                .resolve_verified_exact(id, version, digest)
                .is_err()
        );
    }
    let lease = f.resolve();
    let mut serialized = lease.desired().clone();
    serialized.package_base64.clear();
    assert_ne!(&serialized, lease.desired());
    assert!(lease.current(&f.catalog));
}

#[tokio::test]
async fn unchanged_release_lease_survives_refresh_runtime_and_unrelated_publication() {
    let f = Fixture::new();
    let lease = f.resolve();
    f.refresh().await;
    f.catalog.activate_runtime(&f.storage).await.unwrap();
    f.catalog.install_empty_runtime();
    assert!(lease.current(&f.catalog));
    publish_auth_fixture(&f.root.0, "google", "2.0.0", |_| {});
    f.refresh().await;
    assert!(
        lease.current(&f.catalog),
        "a new default cannot invalidate an exact older release"
    );
    fs::remove_file(f.root.0.join("external/google-2.0.0.release.json")).unwrap();
    f.refresh().await;
    assert!(lease.current(&f.catalog));
}

#[tokio::test]
async fn accepted_release_aba_ends_lease_even_with_retained_snapshot_and_no_intermediate_check() {
    let f = Fixture::new();
    f.refresh().await;
    let retained = f.catalog.state.read().clone();
    let lease = f.resolve();
    let bytes = fs::read(f.marker()).unwrap();
    fs::remove_file(f.marker()).unwrap();
    f.refresh().await;
    fs::write(f.marker(), bytes).unwrap();
    f.refresh().await;
    assert!(!retained.external.is_empty());
    assert!(!lease.current(&f.catalog));
    assert!(f.resolve().current(&f.catalog));
}

#[tokio::test]
async fn rejected_candidate_never_ends_an_accepted_release_lease() {
    let f = Fixture::new();
    let lease = f.resolve();
    let bytes = fs::read(f.marker()).unwrap();
    fs::write(f.marker(), b"invalid candidate").unwrap();
    assert!(f.catalog.refresh_with_runtime(&f.storage).await.is_err());
    assert!(lease.current(&f.catalog));
    fs::write(f.marker(), bytes).unwrap();
    f.refresh().await;
    assert!(lease.current(&f.catalog));
}

#[tokio::test]
async fn publisher_key_rotation_with_identical_release_tuple_cannot_revive_old_lease() {
    let f = Fixture::new();
    let lease = f.resolve();
    let original = fs::read(f.marker()).unwrap();
    let trust = f
        .root
        .0
        .join("external/trusted-publishers")
        .join(format!("{}.pub", f.release.publisher));
    let key = fs::read(&trust).unwrap();
    let identity =
        crate::machine_auth::MachineIdentity::load_or_create(&f.root.0.join("rotated")).unwrap();
    let mut rotated = f.release.clone();
    rotated.signature = identity
        .sign_namespaced(PLUGIN_RELEASE_SIGNATURE_NAMESPACE, &rotated.proof())
        .unwrap();
    fs::write(&trust, identity.public_key()).unwrap();
    fs::write(f.marker(), serde_json::to_vec(&rotated).unwrap()).unwrap();
    f.refresh().await;
    assert_eq!(rotated.artifact_digest, f.release.artifact_digest);
    let replacement = f.resolve();
    fs::write(&trust, key).unwrap();
    fs::write(f.marker(), original).unwrap();
    f.refresh().await;
    assert!(!lease.current(&f.catalog));
    assert!(!replacement.current(&f.catalog));
    assert!(f.resolve().current(&f.catalog));
}

#[test]
fn identical_bytes_in_a_different_catalog_owner_never_share_a_release_lease() {
    let f = Fixture::new();
    let lease = f.resolve();
    let replacement = PluginCatalog::inspect(&f.root.0, Some(f.root.0.join("external"))).unwrap();
    assert!(!lease.current(&replacement));
    assert!(lease.current(&f.catalog));
    drop(f.catalog);
    assert!(!lease.current(&replacement));
}
