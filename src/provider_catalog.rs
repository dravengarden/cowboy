use std::path::PathBuf;

pub(crate) const CODEX_DEEPSEEK_CATALOG: &str = "/nix/var/nix/profiles/columbus-components/codex-deepseek/share/codex-deepseek/codex-models.json";

/// Return the independently deployed DeepSeek-only model catalog.
#[must_use]
pub(crate) fn available_codex_deepseek_catalog() -> Option<PathBuf> {
    let catalog = PathBuf::from(CODEX_DEEPSEEK_CATALOG);
    catalog.is_file().then_some(catalog)
}

#[cfg(feature = "full")]
mod service_catalog {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use anyhow::{Context as _, Result, bail, ensure};
    use base64::Engine as _;
    use cowboy_provider_sdk::{
        PlatformTarget, ProviderPackage, ProviderUiManifest, StandardProviderSource, build_package,
    };
    use parking_lot::RwLock;
    use serde::Serialize;

    use crate::legacy_provider_release::LegacyProviderRelease;
    use crate::machine_auth::{LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE, verify_namespaced};
    use crate::machine_protocol::DesiredPlugin;

    const EMBEDDED_SOURCES: [(&str, &str); 6] = [
        (
            "claude-code",
            include_str!("../plugins/claude-code/provider.json"),
        ),
        ("codex", include_str!("../plugins/codex/provider.json")),
        ("gemini", include_str!("../plugins/gemini/provider.json")),
        ("grok", include_str!("../plugins/grok/provider.json")),
        (
            "claude-deepseek",
            include_str!("../plugins/claude-deepseek/provider.json"),
        ),
        (
            "codex-deepseek",
            include_str!("../plugins/codex-deepseek/provider.json"),
        ),
    ];

    #[derive(Debug, Clone, Serialize)]
    pub(crate) struct CatalogEntry {
        pub provider_id: String,
        pub provider_version: String,
        pub package_digest: String,
        pub artifact_digest: Option<String>,
        /// Public grouping key for credentials shared by multiple Providers.
        ///
        /// This is derived from the package's portable credential schema. It
        /// never exposes credential paths, environment names, or runtime
        /// transports, but lets Cowboy render and synchronize one Service
        /// credential for every compatible Provider surface.
        pub authentication_scope: String,
        pub release_state: AgentPluginReleaseState,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub release_detail: Option<String>,
        pub publisher: String,
        pub contract_fingerprint: String,
        pub supported_platforms: Vec<PlatformTarget>,
        pub manifest: ProviderUiManifest,
    }

    #[derive(Debug, Clone, Copy, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum AgentPluginReleaseState {
        Unbound,
        Ready,
    }

    #[derive(Clone)]
    struct CatalogArtifact {
        entry: CatalogEntry,
        package: ProviderPackage,
    }

    pub(crate) struct ProviderCatalog {
        embedded: BTreeMap<(String, String), CatalogEntry>,
        external: RwLock<BTreeMap<(String, String, String), CatalogArtifact>>,
        plugin_catalog: Arc<crate::plugin_catalog::PluginCatalog>,
        legacy_root: Option<PathBuf>,
    }

    impl ProviderCatalog {
        pub(crate) fn open(
            data_dir: &Path,
            plugin_catalog: Arc<crate::plugin_catalog::PluginCatalog>,
        ) -> Result<Self> {
            let mut embedded = BTreeMap::new();
            for (expected_id, source) in EMBEDDED_SOURCES {
                let source: StandardProviderSource = serde_json::from_str(source)
                    .with_context(|| format!("decoding embedded Provider {expected_id}"))?;
                let package = build_package(source.compile()?)?;
                ensure!(
                    package.manifest.id == expected_id,
                    "embedded Provider id mismatch"
                );
                let bytes = package.canonical_bytes()?;
                let digest = ProviderPackage::artifact_digest(&bytes);
                let key = (
                    package.manifest.id.clone(),
                    package.manifest.version.clone(),
                );
                let entry = CatalogEntry {
                    provider_id: package.manifest.id.clone(),
                    provider_version: package.manifest.version.clone(),
                    package_digest: digest,
                    artifact_digest: None,
                    authentication_scope: package.manifest.authentication.portable_schema.clone(),
                    release_state: AgentPluginReleaseState::Unbound,
                    release_detail: Some(
                        "No signed runtime release is published for this Provider version."
                            .to_owned(),
                    ),
                    publisher: package.manifest.publisher.clone(),
                    contract_fingerprint: package.contract_fingerprint.clone(),
                    supported_platforms: package
                        .manifest
                        .runtime
                        .platforms
                        .iter()
                        .map(|payload| PlatformTarget {
                            os: payload.os.clone(),
                            architecture: payload.architecture.clone(),
                        })
                        .collect(),
                    manifest: package.manifest.ui_projection(),
                };
                ensure!(
                    embedded.insert(key, entry).is_none(),
                    "duplicate embedded Provider"
                );
            }
            // The generic Plugin Catalog is the only installable source. Read
            // the former Provider Catalog only when the default directory was
            // selected, so generations installed before the Plugin cutover
            // remain schedulable until Machines upgrade them.
            let legacy_root = (plugin_catalog.catalog_root() == data_dir.join("plugin-catalog"))
                .then(|| data_dir.join("provider-catalog"));
            let catalog = Self {
                embedded,
                external: RwLock::new(BTreeMap::new()),
                plugin_catalog,
                legacy_root,
            };
            catalog.refresh_external()?;
            Ok(catalog)
        }

        pub(crate) fn published_artifact_path(&self, digest: &str, name: &str) -> Option<PathBuf> {
            self.plugin_catalog.published_artifact_path(digest, name)
        }

        pub(crate) fn catalog_root(&self) -> Option<PathBuf> {
            Some(self.plugin_catalog.catalog_root())
        }

        pub(crate) fn refresh_external(&self) -> Result<usize> {
            self.plugin_catalog.refresh_external()?;
            let mut next = BTreeMap::new();
            for desired in self.plugin_catalog.released_plugins() {
                if desired.release.plugin_kind != cowboy_plugin_sdk::PluginKind::AgentProvider {
                    continue;
                }
                let artifact = catalog_artifact(desired)?;
                insert_unique(&mut next, artifact)?;
            }
            if let Some(root) = self.legacy_root.as_deref() {
                load_legacy_catalog(root, &mut next)?;
            }
            let count = next.len();
            *self.external.write() = next;
            Ok(count)
        }

        pub(crate) fn entries(&self) -> Vec<CatalogEntry> {
            let external = self.external.read();
            let mut combined: Vec<_> = external
                .values()
                .map(|artifact| artifact.entry.clone())
                .collect();
            // Keep newer embedded manifests visible as presentation-only
            // entries while an older signed generation remains installed.
            // Catalog consumers prefer a ready release at equal SemVer, while
            // session chrome may safely adopt a newer compatible presentation.
            combined.extend(self.embedded.values().cloned());
            combined.sort_by(|left, right| {
                left.provider_id
                    .cmp(&right.provider_id)
                    .then(compare_versions(
                        &left.provider_version,
                        &right.provider_version,
                    ))
                    .then(left.artifact_digest.cmp(&right.artifact_digest))
            });
            combined
        }

        pub(crate) fn package(
            &self,
            provider_id: &str,
            version: &str,
            digest: &str,
        ) -> Option<ProviderPackage> {
            let key = (
                provider_id.to_owned(),
                version.to_owned(),
                digest.to_owned(),
            );
            self.external
                .read()
                .get(&key)
                .map(|artifact| artifact.package.clone())
        }

        pub(crate) fn latest_package(
            &self,
            provider_id: &str,
        ) -> Option<(String, String, ProviderPackage)> {
            let external = self.external.read();
            external
                .values()
                .filter(|artifact| artifact.entry.provider_id == provider_id)
                .max_by(|left, right| {
                    compare_versions(&left.entry.provider_version, &right.entry.provider_version)
                        .then(left.entry.artifact_digest.cmp(&right.entry.artifact_digest))
                })
                .and_then(|artifact| {
                    Some((
                        artifact.entry.provider_version.clone(),
                        artifact.entry.artifact_digest.clone()?,
                        artifact.package.clone(),
                    ))
                })
        }

        /// Return the newest signed package for each Provider in one public
        /// credential scope. Provider packages keep their own projection
        /// contract; only the portable bundle is shared.
        pub(crate) fn packages_for_authentication_scope(
            &self,
            scope: &str,
        ) -> Vec<ProviderPackage> {
            let external = self.external.read();
            let mut latest: BTreeMap<String, &CatalogArtifact> = BTreeMap::new();
            for artifact in external
                .values()
                .filter(|artifact| artifact.entry.authentication_scope == scope)
            {
                let replace = latest
                    .get(&artifact.entry.provider_id)
                    .is_none_or(|current| {
                        compare_versions(
                            &artifact.entry.provider_version,
                            &current.entry.provider_version,
                        ) == std::cmp::Ordering::Greater
                            || (artifact.entry.provider_version == current.entry.provider_version
                                && artifact.entry.artifact_digest > current.entry.artifact_digest)
                    });
                if replace {
                    latest.insert(artifact.entry.provider_id.clone(), artifact);
                }
            }
            latest
                .into_values()
                .map(|artifact| artifact.package.clone())
                .collect()
        }

        pub(crate) fn provider_ids_for_authentication_scope(&self, scope: &str) -> Vec<String> {
            self.packages_for_authentication_scope(scope)
                .into_iter()
                .map(|package| package.manifest.id)
                .collect()
        }

        pub(crate) fn account_usage_provider(
            &self,
            provider_id: &str,
            version: &str,
            digest: &str,
        ) -> Option<&'static str> {
            self.package(provider_id, version, digest)
                .and_then(|package| package.manifest.host.account_usage)
                .map(|usage| usage.provider.as_str())
        }
    }

    fn catalog_artifact(desired: DesiredPlugin) -> Result<CatalogArtifact> {
        let bytes = base64::engine::general_purpose::STANDARD.decode(&desired.package_base64)?;
        let plugin_package = desired.release.validate_bytes(&bytes)?;
        let package = plugin_package
            .agent_provider()
            .context("Agent Plugin has no Provider payload")?
            .clone();
        let release = &desired.release;
        let entry = CatalogEntry {
            provider_id: release.plugin_id.clone(),
            provider_version: release.plugin_version.clone(),
            package_digest: ProviderPackage::artifact_digest(&package.canonical_bytes()?),
            artifact_digest: Some(release.artifact_digest.clone()),
            authentication_scope: package.manifest.authentication.portable_schema.clone(),
            release_state: AgentPluginReleaseState::Ready,
            release_detail: None,
            publisher: release.publisher.clone(),
            contract_fingerprint: package.contract_fingerprint.clone(),
            supported_platforms: release.supported_platforms.clone(),
            manifest: package.manifest.ui_projection(),
        };
        Ok(CatalogArtifact { entry, package })
    }

    fn load_legacy_catalog(
        root: &Path,
        target: &mut BTreeMap<(String, String, String), CatalogArtifact>,
    ) -> Result<()> {
        if !root.is_dir() {
            return Ok(());
        }
        let trust_root = root.join("trusted-publishers");
        for entry in fs::read_dir(root)
            .with_context(|| format!("reading legacy Provider Catalog {}", root.display()))?
        {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("cowboy-provider") {
                continue;
            }
            let bytes = fs::read(&path)
                .with_context(|| format!("reading legacy Provider artifact {}", path.display()))?;
            let release_path = path.with_extension("release.json");
            let release: LegacyProviderRelease = serde_json::from_slice(
                &fs::read(&release_path)
                    .with_context(|| format!("reading {}", release_path.display()))?,
            )?;
            let (package, _binding) = release.validate_and_project(&bytes)?;
            let publisher_key_path = trust_root.join(format!("{}.pub", package.manifest.publisher));
            let publisher_key = fs::read_to_string(&publisher_key_path).with_context(|| {
                format!(
                    "reading legacy trusted publisher {}",
                    publisher_key_path.display()
                )
            })?;
            ensure!(
                verify_namespaced(
                    &publisher_key,
                    LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
                    &release.proof()?,
                    &release.signature,
                )?,
                "legacy Provider release signature is invalid"
            );
            let entry = CatalogEntry {
                provider_id: release.provider_id,
                provider_version: release.provider_version,
                package_digest: release.package_digest,
                artifact_digest: Some(release.artifact_digest.clone()),
                authentication_scope: package.manifest.authentication.portable_schema.clone(),
                release_state: AgentPluginReleaseState::Ready,
                release_detail: None,
                publisher: release.publisher,
                contract_fingerprint: package.contract_fingerprint.clone(),
                supported_platforms: release.supported_platforms,
                manifest: package.manifest.ui_projection(),
            };
            insert_unique(target, CatalogArtifact { entry, package })?;
        }
        Ok(())
    }

    fn insert_unique(
        target: &mut BTreeMap<(String, String, String), CatalogArtifact>,
        artifact: CatalogArtifact,
    ) -> Result<()> {
        let key = (
            artifact.entry.provider_id.clone(),
            artifact.entry.provider_version.clone(),
            artifact
                .entry
                .artifact_digest
                .clone()
                .context("released Catalog artifact has no composite digest")?,
        );
        if target.insert(key, artifact).is_some() {
            bail!("duplicate Agent Plugin release in Plugin Catalog")
        }
        Ok(())
    }

    fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
        let left = semver::Version::parse(left).expect("validated Provider semantic version");
        let right = semver::Version::parse(right).expect("validated Provider semantic version");
        left.cmp(&right)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn write_legacy_provider_release(root: &Path) -> (String, String, String) {
            use cowboy_provider_sdk::{
                PlatformRuntimeArtifacts, PlatformTarget, ProviderArtifactFormat,
                ProviderArtifactProbe, ReleasedPrivateComponent, StandardProviderSource,
                build_package,
            };

            let source: StandardProviderSource =
                serde_json::from_str(include_str!("../plugins/gemini/provider.json")).unwrap();
            let package = build_package(source.compile().unwrap()).unwrap();
            let bytes = package.canonical_bytes().unwrap();
            let runtime_artifacts: Vec<PlatformRuntimeArtifacts> = package
                .manifest
                .runtime
                .platforms
                .iter()
                .map(|payload| PlatformRuntimeArtifacts {
                    os: payload.os.clone(),
                    architecture: payload.architecture.clone(),
                    components: payload
                        .private_components
                        .iter()
                        .map(|requirement| ReleasedPrivateComponent {
                            kind: requirement.kind.clone(),
                            slot: requirement.slot.clone(),
                            dependency: requirement.dependency.clone(),
                            version: package.manifest.runtime.dependencies[0].version.clone(),
                            command: requirement.command.clone(),
                            artifact_url: "https://example.invalid/runtime".to_owned(),
                            artifact_digest: format!("sha256:{}", "ab".repeat(32)),
                            artifact_format: ProviderArtifactFormat::Raw,
                            entrypoint: None,
                            probe: ProviderArtifactProbe {
                                args: vec!["--version".to_owned()],
                                timeout_ms: 1_000,
                            },
                        })
                        .collect(),
                })
                .collect();
            let supported_platforms = package
                .manifest
                .runtime
                .platforms
                .iter()
                .map(|payload| PlatformTarget {
                    os: payload.os.clone(),
                    architecture: payload.architecture.clone(),
                })
                .collect::<Vec<_>>();
            let mut release = LegacyProviderRelease {
                release_schema: 2,
                provider_id: package.manifest.id.clone(),
                provider_version: package.manifest.version.clone(),
                package_digest: ProviderPackage::artifact_digest(&bytes),
                artifact_digest: String::new(),
                artifact_url: "https://example.invalid/gemini.cowboy-provider".to_owned(),
                publisher: package.manifest.publisher.clone(),
                contract_fingerprint: package.contract_fingerprint.clone(),
                signature: String::new(),
                supported_platforms,
                runtime_artifacts,
            };
            release.artifact_digest = cowboy_provider_sdk::fingerprint_json(&serde_json::json!({
                "release_schema": release.release_schema,
                "provider_id": release.provider_id,
                "provider_version": release.provider_version,
                "package_digest": release.package_digest,
                "publisher": release.publisher,
                "contract_fingerprint": release.contract_fingerprint,
                "supported_platforms": release.supported_platforms,
                "runtime_artifacts": release.runtime_artifacts,
            }))
            .unwrap();
            let publisher = crate::machine_auth::MachineIdentity::load_or_create(
                &root.join("legacy-publisher"),
            )
            .unwrap();
            release.signature = publisher
                .sign_namespaced(
                    LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
                    &release.proof().unwrap(),
                )
                .unwrap();
            let catalog = root.join("provider-catalog");
            fs::create_dir_all(catalog.join("trusted-publishers")).unwrap();
            fs::write(
                catalog
                    .join("trusted-publishers")
                    .join(format!("{}.pub", release.publisher)),
                publisher.public_key(),
            )
            .unwrap();
            let basename = format!(
                "{}-{}-{}",
                release.provider_id,
                release.provider_version,
                release.artifact_digest.trim_start_matches("sha256:")
            );
            fs::write(catalog.join(format!("{basename}.cowboy-provider")), bytes).unwrap();
            fs::write(
                catalog.join(format!("{basename}.release.json")),
                serde_json::to_vec(&release).unwrap(),
            )
            .unwrap();
            (
                release.provider_id,
                release.provider_version,
                release.artifact_digest,
            )
        }

        #[test]
        fn provider_fingerprints_ignore_serde_json_map_order() {
            let mut first = serde_json::Map::new();
            first.insert("z".to_owned(), serde_json::json!({ "y": 2, "b": 3 }));
            first.insert("a".to_owned(), serde_json::json!(1));
            let mut second = serde_json::Map::new();
            second.insert("a".to_owned(), serde_json::json!(1));
            second.insert("z".to_owned(), serde_json::json!({ "b": 3, "y": 2 }));

            assert_eq!(
                cowboy_provider_sdk::fingerprint_json(&first).unwrap(),
                cowboy_provider_sdk::fingerprint_json(&second).unwrap(),
            );
        }

        #[test]
        fn embedded_catalog_contains_all_six_first_party_providers() {
            let root = std::env::temp_dir().join(format!(
                "cowboy-provider-catalog-test-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let plugins =
                Arc::new(crate::plugin_catalog::PluginCatalog::open(&root, None).unwrap());
            let catalog = ProviderCatalog::open(&root, plugins).unwrap();
            let ids: std::collections::BTreeSet<_> = catalog
                .entries()
                .into_iter()
                .map(|entry| entry.provider_id)
                .collect();
            assert_eq!(
                ids,
                [
                    "claude-code",
                    "claude-deepseek",
                    "codex",
                    "codex-deepseek",
                    "gemini",
                    "grok",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect()
            );
            assert!(catalog.entries().iter().all(|entry| {
                matches!(entry.release_state, AgentPluginReleaseState::Unbound)
                    && entry.artifact_digest.is_none()
            }));
            let _ = fs::remove_dir_all(root);
        }

        #[test]
        fn default_catalog_retains_signed_legacy_generations_for_execution() {
            let root = std::env::temp_dir().join(format!(
                "cowboy-legacy-provider-catalog-test-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let (provider_id, provider_version, generation_digest) =
                write_legacy_provider_release(&root);
            let plugins =
                Arc::new(crate::plugin_catalog::PluginCatalog::open(&root, None).unwrap());
            let catalog = ProviderCatalog::open(&root, Arc::clone(&plugins)).unwrap();

            let entry = catalog
                .entries()
                .into_iter()
                .find(|entry| {
                    entry.provider_id == provider_id
                        && entry.provider_version == provider_version
                        && entry.artifact_digest.as_deref() == Some(&generation_digest)
                })
                .unwrap();
            assert!(matches!(
                entry.release_state,
                AgentPluginReleaseState::Ready
            ));
            assert!(catalog.entries().iter().any(|entry| {
                entry.provider_id == provider_id
                    && matches!(entry.release_state, AgentPluginReleaseState::Unbound)
            }));
            assert!(
                catalog
                    .package(&provider_id, &provider_version, &generation_digest)
                    .is_some()
            );
            assert!(plugins.released_plugins().is_empty());
            let _ = fs::remove_dir_all(root);
        }

        #[test]
        fn catalog_entries_expose_only_the_typed_ui_projection() {
            let root = std::env::temp_dir().join(format!(
                "cowboy-provider-catalog-projection-test-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let plugins =
                Arc::new(crate::plugin_catalog::PluginCatalog::open(&root, None).unwrap());
            let catalog = ProviderCatalog::open(&root, plugins).unwrap();
            let entry = serde_json::to_value(&catalog.entries()[0]).unwrap();
            let manifest = entry.get("manifest").unwrap();
            let authentication = manifest.get("authentication").unwrap();

            assert!(manifest.get("runtime").is_none());
            assert!(authentication.get("portable_schema").is_none());
            assert!(authentication.get("projection_schema").is_none());
            assert!(authentication.get("credential_files").is_none());
            assert!(authentication.get("environment_projection").is_none());
            assert!(
                authentication["methods"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|method| method.get("executor").is_none()
                        && method.get("required_bundle_keys").is_none())
            );
            let _ = fs::remove_dir_all(root);
        }

        #[test]
        fn provider_versions_use_semver_precedence() {
            assert_eq!(
                compare_versions("1.0.0-rc.10", "1.0.0-rc.2"),
                std::cmp::Ordering::Greater
            );
            assert_eq!(
                compare_versions("1.0.0", "1.0.0-rc.10"),
                std::cmp::Ordering::Greater
            );
        }

        #[test]
        fn published_artifact_paths_are_content_addressed_and_confined() {
            let root = std::env::temp_dir().join(format!(
                "cowboy-provider-artifact-path-test-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let plugins =
                Arc::new(crate::plugin_catalog::PluginCatalog::open(&root, None).unwrap());
            let catalog = ProviderCatalog::open(&root, plugins).unwrap();
            let digest = "a".repeat(64);
            assert_eq!(
                catalog
                    .published_artifact_path(&format!("sha256:{digest}"), "codex.tar.gz")
                    .unwrap(),
                root.join("plugin-catalog/artifacts")
                    .join(&digest)
                    .join("codex.tar.gz")
            );
            assert!(catalog.published_artifact_path("short", "codex").is_none());
            assert!(
                catalog
                    .published_artifact_path(&digest, "../secret")
                    .is_none()
            );
            let _ = fs::remove_dir_all(root);
        }
    }
}

#[cfg(feature = "full")]
pub(crate) use service_catalog::ProviderCatalog;

#[cfg(test)]
mod tests {
    #[test]
    fn catalog_is_owned_by_the_component_profile() {
        assert_eq!(
            super::CODEX_DEEPSEEK_CATALOG,
            "/nix/var/nix/profiles/columbus-components/codex-deepseek/share/codex-deepseek/codex-models.json"
        );
        assert!(!super::CODEX_DEEPSEEK_CATALOG.starts_with("/etc/"));
    }
}
