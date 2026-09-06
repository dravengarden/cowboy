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
        let catalog = Self::inspect(data_dir, root)?;
        fs::create_dir_all(&catalog.root)
            .with_context(|| format!("creating Plugin Catalog {}", catalog.root.display()))?;
        Ok(catalog)
    }

    pub(crate) fn inspect(data_dir: &Path, root: Option<PathBuf>) -> Result<Self> {
        ensure_pre_host_cutover(data_dir)?;
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
        let trust_root = self.root.join("trusted-publishers");
        let mut next = BTreeMap::new();
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => Some(entries),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("reading Plugin Catalog"),
        };
        for entry in entries.into_iter().flatten() {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("cowboy-plugin") {
                continue;
            }
            // Publication installs this commit marker last. Inspect its format
            // before the package: newer packages may be opaque to this reader.
            // Unsupported releases grant no identity, host or install authority.
            let Some(release) = read_supported_release(&path.with_extension("release.json"))?
            else {
                continue;
            };
            let bytes = fs::read(&path)
                .with_context(|| format!("reading Plugin artifact {}", path.display()))?;
            let package = PluginPackage::from_bytes(&bytes)
                .with_context(|| format!("validating Plugin artifact {}", path.display()))?;
            release
                .validate_bytes(&bytes)
                .with_context(|| format!("validating Plugin release for {}", path.display()))?;
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

/// This bridge is a pre-cutover reader, not a downgrade of activated Plugin
/// hosts/storage. Reject one-way authority before startup writes any state.
pub(crate) fn ensure_pre_host_cutover(data_dir: &Path) -> Result<()> {
    fn absent(path: &Path) -> Result<()> {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("inspecting Plugin cutover authority"),
            Ok(_) => anyhow::bail!(
                "Catalog reader bridge cannot run after Plugin host authority activation; retain a host-capable Controller"
            ),
        }
    }
    absent(&data_dir.join("plugins/.catalog-only-v1"))?;
    let live = data_dir.join("plugins/live");
    let entries = match fs::read_dir(&live) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("inspecting Plugin host authority"),
    };
    for entry in entries {
        absent(&entry?.path().join(".catalog-authority-v1"))?;
    }
    Ok(())
}

fn read_supported_release(path: &Path) -> Result<Option<PluginRelease>> {
    use std::io::Read as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    const MAX_ENVELOPE_BYTES: u64 = 1024 * 1024;
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("opening Plugin release marker"),
    };
    ensure!(
        file.metadata()?.is_file(),
        "Plugin release marker must be a regular file"
    );
    let mut bytes = Vec::new();
    file.take(MAX_ENVELOPE_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_ENVELOPE_BYTES,
        "Plugin release marker is too large"
    );
    // Deliberately inspect only the format discriminator. Serde still rejects
    // absent, duplicate, non-integer and negative schema fields. No unsigned
    // Plugin ID, version, URL or other future field influences the inventory.
    #[derive(serde::Deserialize)]
    struct Header {
        release_schema: u32,
    }
    let header: Header =
        serde_json::from_slice(&bytes).context("decoding Plugin release header")?;
    ensure!(header.release_schema > 0, "invalid Plugin release schema");
    if header.release_schema > u32::from(cowboy_plugin_sdk::RELEASE_SCHEMA_VERSION) {
        tracing::warn!(
            schema = header.release_schema,
            "unsupported Plugin release skipped by pre-cutover Catalog reader"
        );
        return Ok(None);
    }
    Ok(Some(
        serde_json::from_slice(&bytes).context("decoding supported Plugin release")?,
    ))
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

    struct ReaderFixture(PathBuf);

    impl ReaderFixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "cowboy-catalog-reader-test-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for ReaderFixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn reader_inspection_and_refresh_never_create_missing_catalog_or_service_state() {
        let fixture = ReaderFixture::new();
        let data = fixture.0.join("missing-service");
        let catalog = PluginCatalog::inspect(&data, None).unwrap();
        assert!(!data.exists());
        assert_eq!(catalog.refresh_external().unwrap(), 0);
        assert!(!data.exists());
    }

    #[test]
    fn reader_ignores_uncommitted_and_future_packages_without_decoding_them() {
        let fixture = ReaderFixture::new();
        let catalog_root = fixture.0.join("catalog");
        fs::create_dir(&catalog_root).unwrap();
        fs::write(
            catalog_root.join("future.cowboy-plugin"),
            b"not a supported package",
        )
        .unwrap();
        let catalog = PluginCatalog::open(&fixture.0, Some(catalog_root.clone())).unwrap();
        assert_eq!(catalog.refresh_external().unwrap(), 0);
        fs::write(catalog_root.join("future.release.json"),
            br#"{"release_schema":2,"plugin_id":"codex","plugin_version":"999.0.0","future_field":{"opaque":true}}"#).unwrap();
        assert_eq!(catalog.refresh_external().unwrap(), 0);
        assert!(catalog.resolve("codex", Some("999.0.0"), None).is_err());
        assert!(
            catalog
                .entries()
                .iter()
                .all(|entry| matches!(entry.release_state, PluginReleaseState::Unbound))
        );
        let restarted = PluginCatalog::open(&fixture.0, Some(catalog_root)).unwrap();
        assert!(restarted.released_plugins().is_empty());
    }

    #[test]
    fn reader_rejects_malformed_or_ambiguous_headers_and_bad_supported_releases() {
        let fixture = ReaderFixture::new();
        let path = fixture.0.join("release.json");
        for bytes in [
            "not-json",
            "{}",
            "{\"release_schema\":0}",
            "{\"release_schema\":-1}",
            "{\"release_schema\":1.5}",
            "{\"release_schema\":\"2\"}",
            "{\"release_schema\":1,\"release_schema\":2}",
            "{\"release_schema\":2,\"release_schema\":1}",
            "{\"release_schema\":1,\"future_field\":true}",
        ] {
            fs::write(&path, bytes).unwrap();
            assert!(
                read_supported_release(&path).is_err(),
                "accepted invalid envelope: {bytes}"
            );
        }
        fs::write(&path, vec![b' '; 1024 * 1024 + 1]).unwrap();
        assert!(
            read_supported_release(&path)
                .unwrap_err()
                .to_string()
                .contains("too large")
        );
    }

    #[test]
    fn reader_rejects_linked_or_non_regular_release_markers() {
        let fixture = ReaderFixture::new();
        let marker = fixture.0.join("release.json");
        let target = fixture.0.join("target");
        fs::write(&target, br#"{"release_schema":2}"#).unwrap();
        std::os::unix::fs::symlink(&target, &marker).unwrap();
        assert!(read_supported_release(&marker).is_err());
        fs::remove_file(&marker).unwrap();
        fs::create_dir(&marker).unwrap();
        assert!(read_supported_release(&marker).is_err());
    }

    #[test]
    fn reader_never_downgrades_catalog_only_or_per_plugin_authority() {
        for marker in [
            "plugins/.catalog-only-v1",
            "plugins/live/future/.catalog-authority-v1",
        ] {
            let fixture = ReaderFixture::new();
            let marker = fixture.0.join(marker);
            fs::create_dir_all(marker.parent().unwrap()).unwrap();
            assert!(ensure_pre_host_cutover(&fixture.0).is_ok());
            fs::write(&marker, b"authority").unwrap();
            assert!(ensure_pre_host_cutover(&fixture.0).is_err());
            assert!(PluginCatalog::open(&fixture.0, None).is_err());
            assert!(!fixture.0.join("plugin-catalog").exists());
            assert_eq!(fs::read(&marker).unwrap(), b"authority");
            fs::remove_file(&marker).unwrap();
            std::os::unix::fs::symlink("absent-target", &marker).unwrap();
            assert!(ensure_pre_host_cutover(&fixture.0).is_err());
        }
    }

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

        let catalog = PluginCatalog::open(&root, Some(catalog_root.clone())).unwrap();
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
        // Append-only publication cannot let an opaque newer-format entry
        // shadow, replace or lend authority to this exact signed legacy login.
        let future = catalog_root.join("future.cowboy-plugin");
        fs::write(&future, b"future package bytes").unwrap();
        assert_eq!(catalog.refresh_external().unwrap(), 1);
        fs::write(
            future.with_extension("release.json"),
            br#"{"release_schema":2,"plugin_id":"google","plugin_version":"1.0.0"}"#,
        )
        .unwrap();
        assert_eq!(catalog.refresh_external().unwrap(), 1);
        let restarted = PluginCatalog::open(&root, Some(catalog_root.clone())).unwrap();
        assert_eq!(
            restarted
                .resolve_authentication_provider("google", "1.0.0", &release.artifact_digest)
                .unwrap(),
            contract
        );
        drop(restarted);
        // Corrupt supported releases remain fatal, never silently ignored. A
        // failed refresh retains the old complete snapshot; cold start fails.
        let mut bad_release = release.clone();
        bad_release.signature.clear();
        fs::write(
            future.with_extension("release.json"),
            serde_json::to_vec(&bad_release).unwrap(),
        )
        .unwrap();
        fs::write(&future, &bytes).unwrap();
        assert!(catalog.refresh_external().is_err());
        assert_eq!(
            catalog
                .resolve_authentication_provider("google", "1.0.0", &release.artifact_digest)
                .unwrap(),
            contract
        );
        assert!(PluginCatalog::open(&root, Some(catalog_root)).is_err());
        drop(catalog);
        drop(identity);
        fs::remove_dir_all(root).unwrap();
    }
}
