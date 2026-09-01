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
    AuthenticationProviderContract, PLUGIN_RELEASE_SIGNATURE_NAMESPACE, PluginKind, PluginManifest,
    PluginPackage, PluginRelease,
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
    package: PluginPackage,
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
            let package = PluginPackage::from_bytes(&bytes)
                .with_context(|| format!("validating Plugin artifact {}", path.display()))?;
            let release_path = path.with_extension("release.json");
            let release: PluginRelease = serde_json::from_slice(
                &fs::read(&release_path)
                    .with_context(|| format!("reading {}", release_path.display()))?,
            )?;
            release
                .validate_bytes(&bytes)
                .with_context(|| format!("validating Plugin release {}", release_path.display()))?;
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
            .filter(|artifact| artifact.entry.plugin_kind != PluginKind::AuthenticationProvider)
            .map(|artifact| artifact.desired.clone())
            .collect()
    }

    pub(crate) fn resolve_authentication_provider(
        &self,
        plugin_id: &str,
        version: &str,
        digest: &str,
    ) -> Result<AuthenticationProviderContract> {
        let external = self.external.read();
        let artifact = external
            .get(&(plugin_id.to_owned(), version.to_owned(), digest.to_owned()))
            .context("authentication Plugin release is not in the Catalog")?;
        ensure!(
            artifact.entry.plugin_kind == PluginKind::AuthenticationProvider,
            "configured Plugin is not an Authentication Provider"
        );
        artifact
            .package
            .authentication_provider()
            .cloned()
            .context("authentication Plugin payload is unavailable")
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
        package,
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

    #[test]
    fn signed_authentication_plugin_is_resolved_but_never_sent_to_machine() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-auth-plugin-catalog-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let catalog_root = root.join("catalog");
        let trust_root = catalog_root.join("trusted-publishers");
        fs::create_dir_all(&trust_root).unwrap();
        let identity =
            crate::machine_auth::MachineIdentity::load_or_create(&root.join("identity")).unwrap();
        fs::write(
            trust_root.join("example-publisher.pub"),
            identity.public_key(),
        )
        .unwrap();

        let manifest = cowboy_plugin_sdk::PluginManifest {
            schema_version: 1,
            id: "google".to_owned(),
            version: "1.0.0".to_owned(),
            component_release: "2.0.3".to_owned(),
            publisher: "example-publisher".to_owned(),
            kind: PluginKind::AuthenticationProvider,
            entrypoint: "authentication.json".to_owned(),
            components: vec![cowboy_plugin_sdk::ComponentDependency {
                id: "cowboy.plugin-contract".to_owned(),
                version: "1.2.0".to_owned(),
            }],
        };
        let contract = cowboy_plugin_sdk::AuthenticationProviderContract {
            schema_version: 1,
            id: "google".to_owned(),
            version: "1.0.0".to_owned(),
            display_name: "Google".to_owned(),
            button_label: "Continue with Google".to_owned(),
            protocol: cowboy_plugin_sdk::AuthenticationProtocol::OpenIdConnect(
                cowboy_plugin_sdk::OpenIdConnectContract {
                    issuer: "https://accounts.google.com".to_owned(),
                    authorization_endpoint: "https://accounts.google.com/o/oauth2/v2/auth"
                        .to_owned(),
                    pushed_authorization_request_endpoint: None,
                    token_endpoint: "https://oauth2.googleapis.com/token".to_owned(),
                    jwks_uri: "https://www.googleapis.com/oauth2/v3/certs".to_owned(),
                    end_session_endpoint: None,
                    scopes: vec!["openid".to_owned()],
                    client_authentication_methods: vec![
                        cowboy_plugin_sdk::OidcClientAuthenticationMethod::ClientSecretPost,
                    ],
                    id_token_signing_algorithms: vec![
                        cowboy_plugin_sdk::OidcIdTokenAlgorithm::RS256,
                    ],
                    authorization_parameters: BTreeMap::new(),
                },
            ),
        };
        let package = cowboy_plugin_sdk::PluginPackage::new(
            manifest,
            "2.0.3".to_owned(),
            cowboy_plugin_sdk::PluginPayload::AuthenticationProvider(contract.clone()),
        )
        .unwrap();
        let bytes = package.canonical_bytes().unwrap();
        let mut release = cowboy_plugin_sdk::PluginRelease {
            release_schema: cowboy_plugin_sdk::RELEASE_SCHEMA_VERSION,
            plugin_id: "google".to_owned(),
            plugin_version: "1.0.0".to_owned(),
            plugin_kind: PluginKind::AuthenticationProvider,
            package_digest: cowboy_plugin_sdk::PluginPackage::artifact_digest(&bytes),
            artifact_digest: String::new(),
            artifact_url: "https://plugins.example/google.cowboy-plugin".to_owned(),
            publisher: "example-publisher".to_owned(),
            contract_fingerprint: package.contract_fingerprint.clone(),
            component_release: "2.0.3".to_owned(),
            signature: String::new(),
            supported_platforms: Vec::new(),
            runtime_artifacts: Vec::new(),
        };
        release.artifact_digest = release.computed_artifact_digest().unwrap();
        release.signature = identity
            .sign_namespaced(PLUGIN_RELEASE_SIGNATURE_NAMESPACE, &release.proof())
            .unwrap();
        release.validate_bytes(&bytes).unwrap();
        fs::write(catalog_root.join("google.cowboy-plugin"), &bytes).unwrap();
        fs::write(
            catalog_root.join("google.release.json"),
            serde_json::to_vec(&release).unwrap(),
        )
        .unwrap();

        let catalog = PluginCatalog::open(&root, Some(catalog_root)).unwrap();
        assert!(catalog.released_plugins().is_empty());
        assert_eq!(
            catalog
                .resolve_authentication_provider("google", "1.0.0", &release.artifact_digest,)
                .unwrap(),
            contract
        );
        assert!(
            catalog
                .resolve_authentication_provider(
                    "google",
                    "1.0.0",
                    &format!("sha256:{}", "0".repeat(64)),
                )
                .is_err()
        );
        drop(catalog);
        drop(identity);
        fs::remove_dir_all(root).unwrap();
    }
}
