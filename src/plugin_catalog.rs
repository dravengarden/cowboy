//! Signed Plugin Catalog shared by every installable Cowboy extension.
//!
//! Capability services may project typed payloads from this catalog, but they
//! never select releases or own a second publication directory.

#![cfg(feature = "full")]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use base64::Engine as _;
use cowboy_plugin_sdk::{
    PLUGIN_RELEASE_SIGNATURE_NAMESPACE, PluginKind, PluginManifest, PluginPackage, PluginRelease,
};
use cowboy_provider_sdk::PlatformTarget;
use parking_lot::RwLock;
use serde::Serialize;

use crate::machine_auth::verify_namespaced;
use crate::machine_protocol::DesiredPlugin;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PluginCatalogEntry {
    pub plugin_id: String,
    pub plugin_version: String,
    pub plugin_kind: PluginKind,
    pub package_digest: Option<String>,
    pub artifact_digest: Option<String>,
    pub release_state: PluginReleaseState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_detail: Option<String>,
    pub publisher: String,
    pub contract_fingerprint: Option<String>,
    pub component_release: String,
    pub supported_platforms: Vec<PlatformTarget>,
    pub manifest: PluginManifest,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PluginReleaseState {
    Unbound,
    Ready,
}

#[derive(Clone)]
struct CatalogArtifact {
    entry: PluginCatalogEntry,
    desired: DesiredPlugin,
}

pub(crate) struct PluginCatalog {
    embedded: BTreeMap<(String, String), PluginCatalogEntry>,
    external: RwLock<BTreeMap<(String, String, String), CatalogArtifact>>,
    root: PathBuf,
}

impl PluginCatalog {
    pub(crate) fn open(data_dir: &Path, root: Option<PathBuf>) -> Result<Self> {
        let embedded = crate::plugin::first_party_plugins()
            .iter()
            .map(|manifest| {
                let entry = PluginCatalogEntry {
                    plugin_id: manifest.id.clone(),
                    plugin_version: manifest.version.clone(),
                    plugin_kind: manifest.kind,
                    package_digest: None,
                    artifact_digest: None,
                    release_state: PluginReleaseState::Unbound,
                    release_detail: Some(
                        "No signed runtime release is published for this Plugin version."
                            .to_owned(),
                    ),
                    publisher: manifest.publisher.clone(),
                    contract_fingerprint: None,
                    component_release: crate::plugin::active_component_release().to_owned(),
                    supported_platforms: Vec::new(),
                    manifest: manifest.clone(),
                };
                ((manifest.id.clone(), manifest.version.clone()), entry)
            })
            .collect();
        let catalog = Self {
            embedded,
            external: RwLock::new(BTreeMap::new()),
            root: root.unwrap_or_else(|| data_dir.join("plugin-catalog")),
        };
        catalog.refresh_external()?;
        Ok(catalog)
    }

    pub(crate) fn refresh_external(&self) -> Result<usize> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("creating Plugin Catalog {}", self.root.display()))?;
        let trust_root = self.root.join("trusted-publishers");
        let mut next = BTreeMap::new();
        for entry in fs::read_dir(&self.root)
            .with_context(|| format!("reading Plugin Catalog {}", self.root.display()))?
        {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("cowboy-plugin") {
                continue;
            }
            let bytes = fs::read(&path)
                .with_context(|| format!("reading Plugin artifact {}", path.display()))?;
            let package = PluginPackage::from_bytes(&bytes)?;
            let release_path = path.with_extension("release.json");
            let release: PluginRelease = serde_json::from_slice(
                &fs::read(&release_path)
                    .with_context(|| format!("reading {}", release_path.display()))?,
            )?;
            release.validate_bytes(&bytes)?;
            let key_path = trust_root.join(format!("{}.pub", package.manifest.publisher));
            let public_key = fs::read_to_string(&key_path)
                .with_context(|| format!("reading trusted publisher {}", key_path.display()))?;
            ensure!(
                verify_namespaced(
                    &public_key,
                    PLUGIN_RELEASE_SIGNATURE_NAMESPACE,
                    &release.proof(),
                    &release.signature,
                )?,
                "Plugin release signature is invalid"
            );
            let artifact = catalog_artifact(package, bytes, release, &public_key)?;
            let key = (
                artifact.entry.plugin_id.clone(),
                artifact.entry.plugin_version.clone(),
                artifact
                    .entry
                    .artifact_digest
                    .clone()
                    .context("released Plugin has no artifact digest")?,
            );
            ensure!(
                next.insert(key, artifact).is_none(),
                "duplicate Plugin release"
            );
        }
        let count = next.len();
        *self.external.write() = next;
        Ok(count)
    }

    pub(crate) fn entries(&self) -> Vec<PluginCatalogEntry> {
        let external = self.external.read();
        let released_ids = external
            .values()
            .map(|artifact| artifact.entry.plugin_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut entries = external
            .values()
            .map(|artifact| artifact.entry.clone())
            .collect::<Vec<_>>();
        entries.extend(
            self.embedded
                .values()
                .filter(|entry| !released_ids.contains(entry.plugin_id.as_str()))
                .cloned(),
        );
        entries.sort_by(|left, right| {
            left.plugin_id
                .cmp(&right.plugin_id)
                .then(compare_versions(
                    &left.plugin_version,
                    &right.plugin_version,
                ))
                .then(left.artifact_digest.cmp(&right.artifact_digest))
        });
        entries
    }

    pub(crate) fn released_plugins(&self) -> Vec<DesiredPlugin> {
        self.external
            .read()
            .values()
            .map(|artifact| artifact.desired.clone())
            .collect()
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
        Some(
            self.root
                .join("artifacts")
                .join(digest.to_ascii_lowercase())
                .join(name),
        )
    }

    pub(crate) fn catalog_root(&self) -> PathBuf {
        self.root.clone()
    }

    pub(crate) fn resolve(
        &self,
        plugin_id: &str,
        version: Option<&str>,
        digest: Option<&str>,
    ) -> Result<DesiredPlugin> {
        let external = self.external.read();
        let mut candidates = external
            .values()
            .filter(|artifact| artifact.entry.plugin_id == plugin_id)
            .filter(|artifact| version.is_none_or(|value| artifact.entry.plugin_version == value))
            .filter(|artifact| {
                digest.is_none_or(|value| artifact.entry.artifact_digest.as_deref() == Some(value))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            compare_versions(&left.entry.plugin_version, &right.entry.plugin_version)
                .then(left.entry.artifact_digest.cmp(&right.entry.artifact_digest))
        });
        let selected = candidates.pop().ok_or_else(|| {
            anyhow::anyhow!(if self.embedded.keys().any(|(id, _)| id == plugin_id) {
                "Plugin is known, but no signed runtime release is published"
            } else {
                "Plugin release is not in the Catalog"
            })
        })?;
        if version.is_some() && digest.is_none() {
            ensure!(
                !candidates.iter().any(|candidate| {
                    candidate.entry.plugin_version == selected.entry.plugin_version
                        && candidate.entry.artifact_digest != selected.entry.artifact_digest
                }),
                "Plugin version is ambiguous; select its exact digest"
            );
        }
        Ok(selected.desired.clone())
    }
}

fn catalog_artifact(
    package: PluginPackage,
    bytes: Vec<u8>,
    release: PluginRelease,
    public_key: &str,
) -> Result<CatalogArtifact> {
    let entry = PluginCatalogEntry {
        plugin_id: release.plugin_id.clone(),
        plugin_version: release.plugin_version.clone(),
        plugin_kind: release.plugin_kind,
        package_digest: Some(release.package_digest.clone()),
        artifact_digest: Some(release.artifact_digest.clone()),
        release_state: PluginReleaseState::Ready,
        release_detail: None,
        publisher: release.publisher.clone(),
        contract_fingerprint: Some(release.contract_fingerprint.clone()),
        component_release: release.component_release.clone(),
        supported_platforms: release.supported_platforms.clone(),
        manifest: package.manifest.clone(),
    };
    Ok(CatalogArtifact {
        entry,
        desired: DesiredPlugin {
            release,
            package_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            publisher_public_key: crate::machine_auth::validate_public_key(public_key)?,
        },
    })
}

fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    semver::Version::parse(left)
        .expect("validated Plugin semantic version")
        .cmp(&semver::Version::parse(right).expect("validated Plugin semantic version"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_contains_agent_and_code_plugins() {
        let root =
            std::env::temp_dir().join(format!("cowboy-plugin-catalog-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let catalog = PluginCatalog::open(&root, None).unwrap();
        assert_eq!(catalog.entries().len(), 7);
        assert!(catalog.entries().iter().any(|entry| {
            entry.plugin_id == "zed" && entry.plugin_kind == PluginKind::CodeIntelligence
        }));
        let _ = fs::remove_dir_all(root);
    }
}
