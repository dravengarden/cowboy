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

    use anyhow::{Context as _, Result, bail, ensure};
    use base64::Engine as _;
    use cowboy_provider_sdk::{
        PlatformTarget, ProviderPackage, ProviderRelease, ProviderUiManifest,
        StandardProviderSource, build_package,
    };
    use parking_lot::RwLock;
    use serde::Serialize;

    use crate::machine_auth::{PROVIDER_RELEASE_SIGNATURE_NAMESPACE, verify_namespaced};
    use crate::machine_protocol::DesiredProvider;

    const EMBEDDED_SOURCES: [(&str, &str); 6] = [
        (
            "claude-code",
            include_str!("../providers/claude-code/provider.json"),
        ),
        ("codex", include_str!("../providers/codex/provider.json")),
        ("gemini", include_str!("../providers/gemini/provider.json")),
        ("grok", include_str!("../providers/grok/provider.json")),
        (
            "claude-deepseek",
            include_str!("../providers/claude-deepseek/provider.json"),
        ),
        (
            "codex-deepseek",
            include_str!("../providers/codex-deepseek/provider.json"),
        ),
    ];

    #[derive(Debug, Clone, Serialize)]
    pub(crate) struct CatalogEntry {
        pub provider_id: String,
        pub provider_version: String,
        pub package_digest: String,
        pub artifact_digest: Option<String>,
        pub release_state: ProviderReleaseState,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub release_detail: Option<String>,
        pub publisher: String,
        pub contract_fingerprint: String,
        pub supported_platforms: Vec<PlatformTarget>,
        pub manifest: ProviderUiManifest,
    }

    #[derive(Debug, Clone, Copy, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum ProviderReleaseState {
        Unbound,
        Ready,
    }

    #[derive(Clone)]
    struct CatalogArtifact {
        entry: CatalogEntry,
        desired: DesiredProvider,
    }

    pub(crate) struct ProviderCatalog {
        embedded: BTreeMap<(String, String), CatalogEntry>,
        external: RwLock<BTreeMap<(String, String, String), CatalogArtifact>>,
        external_root: Option<PathBuf>,
    }

    impl ProviderCatalog {
        pub(crate) fn open(data_dir: &Path, external_root: Option<PathBuf>) -> Result<Self> {
            let external_root =
                Some(external_root.unwrap_or_else(|| data_dir.join("provider-catalog")));
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
                    release_state: ProviderReleaseState::Unbound,
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
            let catalog = Self {
                embedded,
                external: RwLock::new(BTreeMap::new()),
                external_root,
            };
            catalog.refresh_external()?;
            Ok(catalog)
        }

        pub(crate) fn published_artifact_path(&self, digest: &str, name: &str) -> Option<PathBuf> {
            let digest = digest.strip_prefix("sha256:").unwrap_or(digest);
            if digest.len() != 64
                || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                || name.is_empty()
                || name.len() > 255
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            {
                return None;
            }
            self.external_root.as_ref().map(|root| {
                root.join("artifacts")
                    .join(digest.to_ascii_lowercase())
                    .join(name)
            })
        }

        pub(crate) fn refresh_external(&self) -> Result<usize> {
            let Some(root) = &self.external_root else {
                self.external.write().clear();
                return Ok(0);
            };
            fs::create_dir_all(root)
                .with_context(|| format!("creating Provider Catalog {}", root.display()))?;
            let trust_root = root.join("trusted-publishers");
            let mut next = BTreeMap::new();
            for entry in fs::read_dir(root)
                .with_context(|| format!("reading Provider Catalog {}", root.display()))?
            {
                let path = entry?.path();
                if path.extension().and_then(|value| value.to_str()) != Some("cowboy-provider") {
                    continue;
                }
                let bytes = fs::read(&path)
                    .with_context(|| format!("reading Provider artifact {}", path.display()))?;
                let package = ProviderPackage::from_bytes(&bytes)?;
                let release_path = path.with_extension("release.json");
                let release: ProviderRelease = serde_json::from_slice(
                    &fs::read(&release_path)
                        .with_context(|| format!("reading {}", release_path.display()))?,
                )?;
                release.validate_bytes(&bytes)?;
                let publisher_key_path =
                    trust_root.join(format!("{}.pub", package.manifest.publisher));
                let publisher_key = fs::read_to_string(&publisher_key_path).with_context(|| {
                    format!("reading trusted publisher {}", publisher_key_path.display())
                })?;
                ensure!(
                    verify_namespaced(
                        &publisher_key,
                        PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
                        &release.proof(),
                        &release.signature,
                    )?,
                    "external Provider signature is invalid"
                );
                let artifact = catalog_artifact(package, bytes, release, &publisher_key)?;
                insert_unique(&mut next, artifact)?;
            }
            let count = next.len();
            *self.external.write() = next;
            Ok(count)
        }

        pub(crate) fn entries(&self) -> Vec<CatalogEntry> {
            let external = self.external.read();
            let released_ids: std::collections::BTreeSet<_> = external
                .values()
                .map(|artifact| artifact.entry.provider_id.as_str())
                .collect();
            let mut combined: Vec<_> = external
                .values()
                .map(|artifact| artifact.entry.clone())
                .collect();
            combined.extend(
                self.embedded
                    .values()
                    .filter(|entry| !released_ids.contains(entry.provider_id.as_str()))
                    .cloned(),
            );
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

        pub(crate) fn resolve(
            &self,
            provider_id: &str,
            version: Option<&str>,
            digest: Option<&str>,
        ) -> Result<DesiredProvider> {
            let external = self.external.read();
            let mut candidates: Vec<_> = external
                .values()
                .filter(|artifact| artifact.entry.provider_id == provider_id)
                .filter(|artifact| {
                    version.is_none_or(|value| artifact.entry.provider_version == value)
                })
                .filter(|artifact| {
                    digest.is_none_or(|value| {
                        artifact.entry.artifact_digest.as_deref() == Some(value)
                    })
                })
                .collect();
            candidates.sort_by(|left, right| {
                compare_versions(&left.entry.provider_version, &right.entry.provider_version)
                    .then(left.entry.artifact_digest.cmp(&right.entry.artifact_digest))
            });
            let selected = match candidates.pop() {
                Some(selected) => selected,
                None if self
                    .embedded
                    .values()
                    .any(|entry| entry.provider_id == provider_id) =>
                {
                    bail!(
                        "Provider is known, but no signed runtime release is published in the Catalog"
                    )
                }
                None => bail!("Provider release is not in the Catalog"),
            };
            if version.is_some() && digest.is_none() {
                let same_version = candidates.iter().any(|candidate| {
                    candidate.entry.provider_version == selected.entry.provider_version
                        && candidate.entry.artifact_digest != selected.entry.artifact_digest
                });
                ensure!(
                    !same_version,
                    "Provider version is ambiguous; select its exact digest"
                );
            }
            Ok(selected.desired.clone())
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
                .and_then(|artifact| {
                    base64::engine::general_purpose::STANDARD
                        .decode(&artifact.desired.package_base64)
                        .ok()
                })
                .and_then(|bytes| ProviderPackage::from_bytes(&bytes).ok())
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

    fn catalog_artifact(
        package: ProviderPackage,
        bytes: Vec<u8>,
        release: ProviderRelease,
        public_key: &str,
    ) -> Result<CatalogArtifact> {
        release.validate_bytes(&bytes)?;
        ensure!(
            verify_namespaced(
                public_key,
                PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
                &release.proof(),
                &release.signature,
            )?,
            "Provider release signature is invalid"
        );
        let entry = CatalogEntry {
            provider_id: release.provider_id.clone(),
            provider_version: release.provider_version.clone(),
            package_digest: release.package_digest.clone(),
            artifact_digest: Some(release.artifact_digest.clone()),
            release_state: ProviderReleaseState::Ready,
            release_detail: None,
            publisher: release.publisher.clone(),
            contract_fingerprint: release.contract_fingerprint.clone(),
            supported_platforms: release.supported_platforms.clone(),
            manifest: package.manifest.ui_projection(),
        };
        Ok(CatalogArtifact {
            entry,
            desired: DesiredProvider {
                release,
                package_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                publisher_public_key: crate::machine_auth::validate_public_key(public_key)?,
            },
        })
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
            bail!("duplicate Provider release in Catalog")
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

        #[test]
        fn embedded_catalog_contains_all_six_first_party_providers() {
            let root = std::env::temp_dir().join(format!(
                "cowboy-provider-catalog-test-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let catalog = ProviderCatalog::open(&root, None).unwrap();
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
                matches!(entry.release_state, ProviderReleaseState::Unbound)
                    && entry.artifact_digest.is_none()
            }));
            let _ = fs::remove_dir_all(root);
        }

        #[test]
        fn catalog_entries_expose_only_the_typed_ui_projection() {
            let root = std::env::temp_dir().join(format!(
                "cowboy-provider-catalog-projection-test-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let catalog = ProviderCatalog::open(&root, None).unwrap();
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
            let catalog = ProviderCatalog::open(&root, None).unwrap();
            let digest = "a".repeat(64);
            assert_eq!(
                catalog
                    .published_artifact_path(&format!("sha256:{digest}"), "codex.tar.gz")
                    .unwrap(),
                root.join("provider-catalog/artifacts")
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
