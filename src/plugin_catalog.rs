//! Signed Plugin Catalog shared by every installable Cowboy extension.
//!
//! Capability services may project typed payloads from this catalog, but they
//! never select releases or own a second publication directory.

#![cfg(feature = "full")]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use base64::Engine as _;
use cowboy_plugin_sdk::{
    AuthenticationProviderContract, PLUGIN_RELEASE_SIGNATURE_NAMESPACE,
    PluginCompatibilityRequirements, PluginKind, PluginManifest, PluginPackage, PluginRelease,
    TelemetryBackendContract, TelemetryEncoding,
};
use cowboy_provider_sdk::PlatformTarget;
use parking_lot::RwLock;
use serde::Serialize;

use crate::machine_auth::verify_namespaced;
use crate::machine_protocol::DesiredPlugin;
use crate::plugin_activation::{HostActivationPolicy, HostPreflightReport, HostSourcePolicy};
use crate::plugin_host_bundle::{
    HOST_BUNDLE_SCHEMA_VERSION, MAX_HOST_BUNDLE_BYTES, PluginHostBundle,
};

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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_host_bundle: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility_requirements: Option<PluginCompatibilityRequirements>,
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
    host_bundle: Option<PluginHostBundle>,
}

#[derive(Clone)]
struct CatalogSnapshot {
    external: BTreeMap<(String, String, String), CatalogArtifact>,
    runtime: Option<Arc<crate::plugin_runtime::PluginRuntime>>,
}

/// One exact released host candidate. Every release is retained so installed
/// Machines and existing sessions can resolve the declaration matching their
/// immutable Plugin identity; `default_for_id` is only the presentation and
/// Controller-side behavior choice for callers with no exact identity.
#[derive(Debug, Clone)]
pub(crate) struct CatalogHostRelease {
    pub entry: PluginCatalogEntry,
    pub host_bundle: Option<PluginHostBundle>,
    pub default_for_id: bool,
}

pub(crate) struct PluginCatalogInventory {
    pub entries: Vec<PluginCatalogEntry>,
    pub runtime: Option<Arc<crate::plugin_runtime::PluginRuntime>>,
}

/// A verified Catalog snapshot projection, not a serialized grant. Only the
/// signature-checked exact-release lookup below can construct this value.
/// It proves a contract, not enrollment, policy or a live installation lease.
pub(crate) struct VerifiedTelemetryRelease {
    contract: TelemetryBackendContract,
    generation_digest: String,
    contract_fingerprint: String,
}

impl VerifiedTelemetryRelease {
    pub(crate) fn matches_inventory(
        &self,
        plugin: &crate::machine_protocol::PluginInventory,
    ) -> bool {
        plugin.plugin_kind == PluginKind::TelemetryBackend
            && plugin.plugin_id == self.contract.id
            && plugin.plugin_version == self.contract.version
            && plugin.generation_digest == self.generation_digest
            && plugin.contract_fingerprint == self.contract_fingerprint
            && plugin.state == crate::machine_protocol::PluginInstallationState::Active
            && plugin.auth_generation.is_none()
    }

    pub(crate) fn operation_for(
        &self,
        signal: Option<crate::otlp::Signal>,
    ) -> Option<crate::machine_protocol::PluginHostOperation> {
        use crate::machine_protocol::PluginHostOperation;
        let Some(signal) = signal else {
            return (self.contract.schema_version == 1)
                .then_some(PluginHostOperation::ExportTelemetry);
        };
        let route = match signal {
            crate::otlp::Signal::Logs => self.contract.logs.as_ref(),
            crate::otlp::Signal::Metrics => self.contract.metrics.as_ref(),
            crate::otlp::Signal::Traces => self.contract.traces.as_ref(),
        }?;
        (self.contract.schema_version == 2 && route.encoding == TelemetryEncoding::OtlpHttpProtobuf)
            .then_some(PluginHostOperation::ExportOtlp)
    }
}

pub(crate) struct PluginCatalog {
    embedded: BTreeMap<(String, String), PluginCatalogEntry>,
    state: RwLock<Arc<CatalogSnapshot>>,
    refresh_lock: tokio::sync::Mutex<()>,
    roots: Vec<PathBuf>,
    plugin_dir: crate::plugin_dir::PluginDir,
    host_policy: HostActivationPolicy,
}

impl PluginCatalog {
    #[cfg(test)]
    pub(crate) fn open(data_dir: &Path, root: Option<PathBuf>) -> Result<Self> {
        let catalog = Self::inspect(data_dir, root)?;
        catalog.initialize()?;
        Ok(catalog)
    }

    /// Called only after startup preflight, never by inspection or refresh.
    pub(crate) fn initialize(&self) -> Result<()> {
        self.plugin_dir.initialize()?;
        for root in &self.roots {
            fs::create_dir_all(root)
                .with_context(|| format!("creating Plugin Catalog {}", root.display()))?;
        }
        Ok(())
    }

    /// Read the same trusted Catalog as startup without creating any state.
    /// Missing roots are empty inventories, not an instruction to initialize.
    pub(crate) fn inspect(data_dir: &Path, root: Option<PathBuf>) -> Result<Self> {
        let plugin_dir = crate::plugin_dir::PluginDir::inspect(data_dir);
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
                    component_release: manifest.component_release.clone(),
                    supported_platforms: Vec::new(),
                    manifest: manifest.clone(),
                    has_host_bundle: false,
                    compatibility_requirements: None,
                };
                ((manifest.id.clone(), manifest.version.clone()), entry)
            })
            .collect();
        let roots = if let Some(root) = root {
            vec![root]
        } else {
            let mut roots = vec![plugin_dir.catalog_dir()];
            let legacy = crate::plugin_dir::PluginDir::legacy_catalog_dir(data_dir);
            if legacy.exists() && legacy != plugin_dir.catalog_dir() {
                roots.push(legacy);
            }
            roots
        };
        let mut external = BTreeMap::new();
        for (index, root) in roots.iter().enumerate() {
            load_catalog_root(root, &mut external, index > 0)?;
        }
        Ok(Self {
            embedded,
            state: RwLock::new(Arc::new(CatalogSnapshot {
                external,
                runtime: None,
            })),
            refresh_lock: tokio::sync::Mutex::new(()),
            roots,
            plugin_dir,
            host_policy: HostActivationPolicy::default(),
        })
    }

    fn load_external(&self) -> Result<BTreeMap<(String, String, String), CatalogArtifact>> {
        let mut next = BTreeMap::new();
        for (index, root) in self.roots.iter().enumerate() {
            load_catalog_root(root, &mut next, index > 0)?;
        }
        Ok(next)
    }

    /// Set the immutable startup policy before the Catalog is shared. This
    /// preflight performs no generation staging, marker writes, or migrations.
    pub(crate) fn configure_hosts(&mut self, policy: HostActivationPolicy) -> Result<()> {
        let current = self.state.read();
        ensure!(
            current.runtime.is_none(),
            "host activation policy is immutable after startup"
        );
        self.select_host_releases(&current.external, &policy)?;
        drop(current);
        self.host_policy = policy;
        Ok(())
    }

    pub(crate) fn strict_host_activation(&self) -> bool {
        self.host_policy.strict_activation()
    }

    pub(crate) fn host_preflight_report(&self) -> Result<HostPreflightReport> {
        let current = self.state.read();
        let (releases, catalog_only_recorded) =
            self.select_host_releases(&current.external, &self.host_policy)?;
        self.host_policy.report(&releases, catalog_only_recorded)
    }

    fn select_host_releases(
        &self,
        external: &BTreeMap<(String, String, String), CatalogArtifact>,
        policy: &HostActivationPolicy,
    ) -> Result<(Vec<CatalogHostRelease>, bool)> {
        let catalog_only_recorded = self.plugin_dir.catalog_only_required()?;
        ensure!(
            !catalog_only_recorded || policy.source == HostSourcePolicy::CatalogOnly,
            "Controller previously activated catalog_only; configure its exact host policy before restarting"
        );
        let mut releases = catalog_host_releases(external);
        policy.select(&mut releases)?;
        if policy.source == HostSourcePolicy::Bootstrap && policy.require_webauthn_storage {
            self.check_bootstrap_webauthn_storage(&releases)?;
        }
        Ok((releases, catalog_only_recorded))
    }

    /// Source storage is only the pre-Catalog compatibility path. Once its ID
    /// has a release authority, missing selection must fail preflight, not get
    /// discovered after the core database has already been migrated.
    fn check_bootstrap_webauthn_storage(&self, releases: &[CatalogHostRelease]) -> Result<()> {
        let capability = crate::core_passkeys::STORAGE_CAPABILITY;
        let mut claims = Vec::new();
        for release in releases.iter().filter(|release| release.default_for_id) {
            let Some(bundle) = &release.host_bundle else {
                continue;
            };
            let host = crate::plugin_host::PluginHostSpec::from_json(
                bundle.files["host.json"].as_bytes(),
            )?;
            if host
                .native_capabilities
                .iter()
                .any(|name| name == capability)
            {
                ensure!(
                    release.entry.plugin_kind == PluginKind::AuthenticationProvider
                        && host.storage.is_some(),
                    "selected WebAuthn host does not implement authentication storage"
                );
                claims.push(release.entry.plugin_id.as_str());
            }
        }
        for (id, source) in crate::first_party_sources::HOST_SOURCES {
            if releases
                .iter()
                .any(|release| release.entry.plugin_id == *id)
                || self.plugin_dir.has_catalog_authority(id)?
            {
                continue;
            }
            let host = crate::plugin_host::PluginHostSpec::from_json(source.as_bytes())?;
            crate::core_passkeys::validate_legacy_host(&host)?;
            if host
                .native_capabilities
                .iter()
                .any(|name| name == capability)
            {
                ensure!(
                    host.storage.is_some(),
                    "bootstrap WebAuthn host has no storage"
                );
                claims.push(id);
            }
        }
        ensure!(
            claims.len() == 1,
            "bootstrap requires one WebAuthn storage host; configure an exact host selection for a published storage Plugin"
        );
        Ok(())
    }

    fn record_host_authority(&self) -> Result<()> {
        if self.host_policy.source == HostSourcePolicy::CatalogOnly {
            self.plugin_dir.record_catalog_only()?;
        }
        Ok(())
    }

    /// Stage the current Catalog host inventory and attach it to this Catalog
    /// snapshot. Startup uses this after the storage backend is available.
    ///
    /// # Errors
    /// Returns without changing the visible snapshot when staging, semantic
    /// validation, or a required Plugin migration fails.
    pub(crate) async fn activate_runtime(
        &self,
        storage: &crate::plugin_storage::PluginStorage,
    ) -> Result<Arc<crate::plugin_runtime::PluginRuntime>> {
        let _refresh = self.refresh_lock.lock().await;
        let current = self.state.read().clone();
        let (releases, _) = self.select_host_releases(&current.external, &self.host_policy)?;
        let candidate_ids = releases
            .iter()
            .map(|release| release.entry.plugin_id.clone())
            .collect();
        let mut runtime = crate::plugin_runtime::PluginRuntime::activate_releases(
            storage,
            &releases,
            self.host_policy.source == HostSourcePolicy::Bootstrap,
        )
        .await?;
        if let Some(previous) = current.runtime.as_deref() {
            runtime.retain_previous_generations(previous, &candidate_ids);
        }
        let runtime = Arc::new(runtime);
        self.record_host_authority()?;
        *self.state.write() = Arc::new(CatalogSnapshot {
            external: current.external.clone(),
            runtime: Some(Arc::clone(&runtime)),
        });
        Ok(runtime)
    }

    /// Reload, verify, stage, and migrate a complete Catalog candidate before
    /// atomically making either its releases or host runtime visible.
    ///
    /// # Errors
    /// Leaves the previous Catalog/runtime snapshot untouched on any failure.
    pub(crate) async fn refresh_with_runtime(
        &self,
        storage: &crate::plugin_storage::PluginStorage,
    ) -> Result<usize> {
        let _refresh = self.refresh_lock.lock().await;
        let current = self.state.read().clone();
        let external = self.load_external()?;
        let (releases, _) = self.select_host_releases(&external, &self.host_policy)?;
        let candidate_ids = releases
            .iter()
            .map(|release| release.entry.plugin_id.clone())
            .collect();
        let mut runtime = crate::plugin_runtime::PluginRuntime::activate_releases(
            storage,
            &releases,
            self.host_policy.source == HostSourcePolicy::Bootstrap,
        )
        .await?;
        if let Some(previous) = current.runtime.as_deref() {
            runtime.retain_previous_generations(previous, &candidate_ids);
        }
        let runtime = Arc::new(runtime);
        let count = external.len();
        self.record_host_authority()?;
        *self.state.write() = Arc::new(CatalogSnapshot {
            external,
            runtime: Some(runtime),
        });
        Ok(count)
    }

    /// Install a fail-closed empty runtime after a startup activation failure.
    pub(crate) fn install_empty_runtime(&self) -> Arc<crate::plugin_runtime::PluginRuntime> {
        let runtime = Arc::new(crate::plugin_runtime::PluginRuntime::empty());
        let current = self.state.read().clone();
        *self.state.write() = Arc::new(CatalogSnapshot {
            external: current.external.clone(),
            runtime: Some(Arc::clone(&runtime)),
        });
        runtime
    }

    #[must_use]
    pub(crate) fn runtime(&self) -> Option<Arc<crate::plugin_runtime::PluginRuntime>> {
        self.state.read().runtime.clone()
    }

    pub(crate) fn entries(&self) -> Vec<PluginCatalogEntry> {
        let snapshot = self.state.read().clone();
        self.entries_for(&snapshot.external)
    }

    #[must_use]
    pub(crate) fn inventory(&self) -> PluginCatalogInventory {
        let snapshot = self.state.read().clone();
        PluginCatalogInventory {
            entries: self.entries_for(&snapshot.external),
            runtime: snapshot.runtime.clone(),
        }
    }

    fn entries_for(
        &self,
        external: &BTreeMap<(String, String, String), CatalogArtifact>,
    ) -> Vec<PluginCatalogEntry> {
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
        self.state
            .read()
            .external
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
        let snapshot = self.state.read().clone();
        let external = &snapshot.external;
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

    /// No latest/version fallback and no embedded/source-only contract. Trust
    /// is the currently accepted Catalog snapshot; refresh failure retains the
    /// previous snapshot under the existing Catalog policy. An in-flight call
    /// may finish against its resolved snapshot, not an arbitrary future one.
    pub(crate) fn resolve_telemetry_backend(
        &self,
        plugin_id: &str,
        version: &str,
        digest: &str,
    ) -> Result<VerifiedTelemetryRelease> {
        let snapshot = self.state.read();
        let artifact = snapshot
            .external
            .get(&(plugin_id.to_owned(), version.to_owned(), digest.to_owned()))
            .context("telemetry Plugin release is not in the Catalog")?;
        let cowboy_plugin_sdk::PluginPayload::TelemetryBackend(contract) =
            &artifact.package.payload
        else {
            anyhow::bail!("configured Plugin is not a Telemetry Backend");
        };
        Ok(VerifiedTelemetryRelease {
            contract: contract.clone(),
            generation_digest: digest.to_owned(),
            contract_fingerprint: artifact.package.contract_fingerprint.clone(),
        })
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
        let relative = Path::new("artifacts")
            .join(digest.to_ascii_lowercase())
            .join(name);
        // Download from the same ordered roots used to read the Catalog. A
        // default-layout migration must not strand immutable published URLs in
        // the still-readable legacy root. An explicit root remains exclusive.
        // Only absence permits fallback: a present but unreadable/non-file
        // primary candidate must reach the normal HTTP error path instead.
        self.roots
            .iter()
            .map(|root| root.join(&relative))
            .find(|path| {
                !matches!(fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
            })
            .or_else(|| Some(self.catalog_root().join(relative)))
    }

    pub(crate) fn catalog_root(&self) -> PathBuf {
        self.roots
            .first()
            .cloned()
            .expect("plugin catalog has a primary root")
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn host_releases(&self) -> Vec<CatalogHostRelease> {
        catalog_host_releases(&self.state.read().external)
    }

    pub(crate) fn resolve(
        &self,
        plugin_id: &str,
        version: Option<&str>,
        digest: Option<&str>,
    ) -> Result<DesiredPlugin> {
        let snapshot = self.state.read().clone();
        let external = &snapshot.external;
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
        if digest.is_none() {
            ensure!(
                !candidates.iter().any(|candidate| {
                    candidate.entry.plugin_version == selected.entry.plugin_version
                        && candidate.entry.artifact_digest != selected.entry.artifact_digest
                }),
                "latest Plugin version is ambiguous; select its exact version and digest"
            );
        }
        Ok(selected.desired.clone())
    }
}

fn load_catalog_root(
    root: &Path,
    next: &mut BTreeMap<(String, String, String), CatalogArtifact>,
    skip_duplicates: bool,
) -> Result<()> {
    let trust_root = root.join("trusted-publishers");
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading Plugin Catalog {}", root.display()));
        }
    };
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("cowboy-plugin") {
            continue;
        }
        let release_path = path.with_extension("release.json");
        // Publication installs this commit marker last. Inspect its format
        // before the package: newer packages may be opaque to this reader.
        // Unsupported releases grant no identity, host or install authority.
        let Some(release) = read_supported_release(&release_path)? else {
            continue;
        };
        let bytes = fs::read(&path)
            .with_context(|| format!("reading Plugin artifact {}", path.display()))?;
        let package = PluginPackage::from_bytes(&bytes)
            .with_context(|| format!("validating Plugin artifact {}", path.display()))?;
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
        let host_bundle = PluginHostBundle::load_bytes_for_package(
            &path.with_extension("hostbundle.json"),
            &package.manifest.id,
            &package.manifest.version,
            &PluginPackage::artifact_digest(&bytes),
            release.host_bundle_digest.as_deref(),
        )?;
        let artifact = catalog_artifact(package, bytes, release, &public_key, host_bundle)?;
        let key = (
            artifact.entry.plugin_id.clone(),
            artifact.entry.plugin_version.clone(),
            artifact
                .entry
                .artifact_digest
                .clone()
                .context("released Plugin has no artifact digest")?,
        );
        if next.contains_key(&key) {
            ensure!(
                skip_duplicates,
                "duplicate Plugin release in {}",
                path.display()
            );
            continue;
        }
        next.insert(key, artifact);
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
            "unsupported Plugin release skipped by Catalog reader"
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
    host_bundle: Option<(PluginHostBundle, Vec<u8>)>,
) -> Result<CatalogArtifact> {
    let (host_bundle, host_bundle_bytes) = host_bundle
        .map(|(bundle, bytes)| (Some(bundle), Some(bytes)))
        .unwrap_or((None, None));
    let compatibility_requirements =
        plugin_compatibility_requirements(&package, &release, host_bundle.as_ref())?;
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
        has_host_bundle: host_bundle.is_some(),
        compatibility_requirements: Some(compatibility_requirements),
    };
    Ok(CatalogArtifact {
        entry,
        desired: DesiredPlugin {
            release,
            package_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            publisher_public_key: crate::machine_auth::validate_public_key(public_key)?,
            host_bundle_base64: host_bundle_bytes
                .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
        },
        package,
        host_bundle,
    })
}

pub(crate) fn desired_plugin_compatibility(
    desired: &DesiredPlugin,
) -> Result<(PluginPackage, PluginCompatibilityRequirements)> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&desired.package_base64)
        .context("decoding Catalog Plugin package")?;
    let package = desired
        .release
        .validate_bytes(&bytes)
        .context("validating Catalog Plugin release")?;
    let host_bytes = desired
        .host_bundle_base64
        .as_deref()
        .map(|encoded| {
            ensure!(
                encoded.len() <= MAX_HOST_BUNDLE_BYTES.saturating_mul(4).div_ceil(3) + 4,
                "encoded Catalog Plugin host bundle is too large"
            );
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .context("decoding Catalog Plugin host bundle")
        })
        .transpose()?;
    let host = PluginHostBundle::from_bytes_for_package(
        host_bytes.as_deref(),
        &package.manifest.id,
        &package.manifest.version,
        &desired.release.package_digest,
        desired.release.host_bundle_digest.as_deref(),
    )?;
    let requirements =
        plugin_compatibility_requirements(&package, &desired.release, host.as_ref())?;
    Ok((package, requirements))
}

fn plugin_compatibility_requirements(
    package: &PluginPackage,
    release: &PluginRelease,
    host: Option<&PluginHostBundle>,
) -> Result<PluginCompatibilityRequirements> {
    package.validate_host_contract(host.map(|host| &host.files))?;
    let host_schema = host
        .map(|host| {
            crate::plugin_host::PluginHostSpec::from_json(host.files["host.json"].as_bytes())
                .map(|spec| spec.schema_version)
        })
        .transpose()?;
    PluginCompatibilityRequirements::for_release(
        package,
        release,
        host.map(|_| HOST_BUNDLE_SCHEMA_VERSION),
        host_schema,
    )
}

fn catalog_host_releases(
    external: &BTreeMap<(String, String, String), CatalogArtifact>,
) -> Vec<CatalogHostRelease> {
    let mut defaults = BTreeMap::<&str, Option<(&str, &str)>>::new();
    let mut grouped = BTreeMap::<&str, Vec<&CatalogArtifact>>::new();
    for artifact in external.values() {
        grouped
            .entry(artifact.entry.plugin_id.as_str())
            .or_default()
            .push(artifact);
    }
    for (plugin_id, artifacts) in grouped {
        let identity = latest_unique_release_identity(artifacts.iter().filter_map(|artifact| {
            Some((
                artifact.entry.plugin_version.as_str(),
                artifact.entry.artifact_digest.as_deref()?,
            ))
        }));
        if identity.is_none() {
            tracing::error!(
                plugin_id,
                "latest Plugin version is ambiguous; an exact host selection is required for an ID default"
            );
        }
        defaults.insert(plugin_id, identity);
    }
    external
        .values()
        .map(|artifact| {
            let identity = artifact
                .entry
                .artifact_digest
                .as_deref()
                .map(|digest| (artifact.entry.plugin_version.as_str(), digest));
            CatalogHostRelease {
                entry: artifact.entry.clone(),
                host_bundle: artifact.host_bundle.clone(),
                default_for_id: defaults
                    .get(artifact.entry.plugin_id.as_str())
                    .copied()
                    .flatten()
                    == identity,
            }
        })
        .collect()
}

fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    semver::Version::parse(left)
        .expect("validated Plugin semantic version")
        .cmp(&semver::Version::parse(right).expect("validated Plugin semantic version"))
}

fn latest_unique_release_identity<'a>(
    candidates: impl Iterator<Item = (&'a str, &'a str)>,
) -> Option<(&'a str, &'a str)> {
    let mut latest = None;
    let mut ambiguous = false;
    for candidate in candidates {
        let Some(current) = latest else {
            latest = Some(candidate);
            continue;
        };
        match compare_versions(candidate.0, current.0) {
            std::cmp::Ordering::Greater => {
                latest = Some(candidate);
                ambiguous = false;
            }
            std::cmp::Ordering::Equal if candidate.1 != current.1 => {
                ambiguous = true;
            }
            std::cmp::Ordering::Equal | std::cmp::Ordering::Less => {}
        }
    }
    (!ambiguous).then_some(latest).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest as _;
    use std::os::unix::fs::PermissionsExt as _;

    fn unlock_tree(path: &Path) {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return;
        };
        if metadata.file_type().is_symlink() {
            return;
        }
        if metadata.is_dir() {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
            for entry in fs::read_dir(path).unwrap() {
                unlock_tree(&entry.unwrap().path());
            }
        } else {
            fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        }
    }

    #[test]
    fn host_bundle_identity_uses_semver_and_rejects_ambiguous_latest_bytes() {
        assert_eq!(
            latest_unique_release_identity([("2.0.0", "two"), ("10.0.0", "ten")].into_iter()),
            Some(("10.0.0", "ten"))
        );
        assert_eq!(
            latest_unique_release_identity(
                [("10.0.0", "first"), ("9.0.0", "old"), ("10.0.0", "second")].into_iter()
            ),
            None
        );
        assert_eq!(
            latest_unique_release_identity(
                [("10.0.0", "first"), ("10.0.0", "second"), ("11.0.0", "new")].into_iter()
            ),
            Some(("11.0.0", "new"))
        );
    }

    #[test]
    fn default_catalog_root_is_the_server_plugin_dir() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-plugin-catalog-layout-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        let catalog = PluginCatalog::open(&root, None).unwrap();
        assert_eq!(
            catalog.catalog_root(),
            crate::plugin_dir::PluginDir::open(&root)
                .unwrap()
                .catalog_dir()
        );
        let _ = fs::remove_dir_all(root);
    }

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
            unlock_tree(&self.0);
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn published_artifacts_follow_read_roots_without_migrating_bytes() {
        let fixture = ReaderFixture::new();
        let digest = "ab".repeat(32);
        let legacy = fixture.0.join("plugin-catalog/artifacts").join(&digest);
        fs::create_dir_all(&legacy).unwrap();
        for name in ["codex.cowboy-plugin", "codex.tar.gz"] {
            fs::write(legacy.join(name), b"published bytes").unwrap();
        }
        let catalog = PluginCatalog::inspect(&fixture.0, None).unwrap();
        let canonical = fixture.0.join("plugins/catalog/artifacts").join(&digest);
        for name in ["codex.cowboy-plugin", "codex.tar.gz"] {
            assert_eq!(
                catalog.published_artifact_path(
                    &format!("sha256:{}", digest.to_ascii_uppercase()),
                    name
                ),
                Some(legacy.join(name))
            );
        }
        assert!(!fixture.0.join("plugins").exists());
        catalog.initialize().unwrap();
        assert_eq!(
            catalog.published_artifact_path(&digest, "codex.tar.gz"),
            Some(legacy.join("codex.tar.gz"))
        );
        assert!(!canonical.exists());
        assert_eq!(
            fs::read(legacy.join("codex.tar.gz")).unwrap(),
            b"published bytes"
        );

        fs::create_dir_all(&canonical).unwrap();
        fs::write(canonical.join("codex.tar.gz"), b"published bytes").unwrap();
        assert_eq!(
            catalog.published_artifact_path(&digest, "codex.tar.gz"),
            Some(canonical.join("codex.tar.gz"))
        );
        assert_eq!(
            catalog.published_artifact_path(&digest, "missing.tar.gz"),
            Some(canonical.join("missing.tar.gz"))
        );
        for (digest, name) in [
            ("short", "codex"),
            (&digest, "../secret"),
            (&digest, "a/b"),
            (&digest, ""),
        ] {
            assert!(catalog.published_artifact_path(digest, name).is_none());
        }
    }

    #[test]
    fn published_artifact_explicit_root_does_not_search_default_or_legacy() {
        let fixture = ReaderFixture::new();
        let digest = "a".repeat(64);
        for root in ["plugin-catalog", "plugins/catalog"] {
            let directory = fixture.0.join(root).join("artifacts").join(&digest);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("codex.tar.gz"), b"not in selected root").unwrap();
        }
        let selected = fixture.0.join("selected");
        let catalog = PluginCatalog::inspect(&fixture.0, Some(selected.clone())).unwrap();
        let expected = selected
            .join("artifacts")
            .join(&digest)
            .join("codex.tar.gz");
        assert_eq!(
            catalog.published_artifact_path(&digest, "codex.tar.gz"),
            Some(expected.clone())
        );
        assert!(!selected.exists());
        fs::create_dir_all(expected.parent().unwrap()).unwrap();
        fs::write(&expected, b"selected bytes").unwrap();
        assert_eq!(
            catalog.published_artifact_path(&digest, "codex.tar.gz"),
            Some(expected)
        );
    }

    #[test]
    fn published_artifact_present_primary_never_falls_through_to_legacy() {
        let fixture = ReaderFixture::new();
        let digest = "a".repeat(64);
        let legacy = fixture.0.join("plugin-catalog/artifacts").join(&digest);
        let primary = fixture.0.join("plugins/catalog/artifacts").join(&digest);
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&primary).unwrap();
        for name in ["directory", "dangling"] {
            fs::write(legacy.join(name), b"must not mask primary failure").unwrap();
        }
        fs::create_dir(primary.join("directory")).unwrap();
        std::os::unix::fs::symlink("missing", primary.join("dangling")).unwrap();
        let catalog = PluginCatalog::inspect(&fixture.0, None).unwrap();
        for name in ["directory", "dangling"] {
            assert_eq!(
                catalog.published_artifact_path(&digest, name),
                Some(primary.join(name))
            );
        }
    }

    #[test]
    fn reader_inspection_and_reload_never_create_missing_catalog_or_service_state() {
        let fixture = ReaderFixture::new();
        let data = fixture.0.join("missing-service");
        let catalog = PluginCatalog::inspect(&data, None).unwrap();
        assert!(!data.exists());
        assert!(catalog.load_external().unwrap().is_empty());
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
        assert!(catalog.load_external().unwrap().is_empty());
        fs::write(catalog_root.join("future.release.json"),
            br#"{"release_schema":3,"plugin_id":"codex","plugin_version":"999.0.0","future_field":{"opaque":true}}"#).unwrap();
        assert!(catalog.load_external().unwrap().is_empty());
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
            "{\"release_schema\":2,\"release_schema\":3}",
            "{\"release_schema\":3,\"release_schema\":2}",
            "{\"release_schema\":1,\"future_field\":true}",
            "{\"release_schema\":2,\"future_field\":true}",
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
        fs::write(&target, br#"{"release_schema":3}"#).unwrap();
        std::os::unix::fs::symlink(&target, &marker).unwrap();
        assert!(read_supported_release(&marker).is_err());
        fs::remove_file(&marker).unwrap();
        fs::create_dir(&marker).unwrap();
        assert!(read_supported_release(&marker).is_err());
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
        for entry in catalog.entries() {
            assert_eq!(entry.component_release, entry.manifest.component_release);
        }
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn failed_refresh_keeps_the_previous_catalog_and_runtime_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-plugin-catalog-transaction-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let catalog_root = root.join("external");
        let catalog = PluginCatalog::open(&root, Some(catalog_root.clone())).unwrap();
        let storage = crate::plugin_storage::PluginStorage::sqlite_files(
            crate::plugin_dir::PluginDir::open(&root).unwrap(),
        );
        let before = catalog.activate_runtime(&storage).await.unwrap();
        let before_entries = catalog.entries();

        fs::write(catalog_root.join("broken.cowboy-plugin"), b"not a package").unwrap();
        fs::write(catalog_root.join("broken.release.json"), b"{}").unwrap();
        assert!(catalog.refresh_with_runtime(&storage).await.is_err());

        let after = catalog.runtime().expect("previous runtime");
        assert!(Arc::ptr_eq(&before, &after));
        assert_eq!(catalog.entries().len(), before_entries.len());
        assert_eq!(
            catalog
                .entries()
                .iter()
                .map(|entry| (&entry.plugin_id, &entry.plugin_version))
                .collect::<Vec<_>>(),
            before_entries
                .iter()
                .map(|entry| (&entry.plugin_id, &entry.plugin_version))
                .collect::<Vec<_>>()
        );

        unlock_tree(&root);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn signed_authentication_plugin_is_resolved_but_never_sent_to_machine() {
        for release_schema in [1, 2] {
            assert_signed_authentication_reader(release_schema).await;
        }
    }

    async fn assert_signed_authentication_reader(release_schema: u16) {
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
            components: vec![
                cowboy_plugin_sdk::ComponentDependency {
                    id: "cowboy.plugin-contract".to_owned(),
                    version: "1.2.0".to_owned(),
                },
                cowboy_plugin_sdk::ComponentDependency {
                    id: "cowboy.plugin-sdk".to_owned(),
                    version: cowboy_plugin_sdk::PLUGIN_SDK_VERSION.to_owned(),
                },
            ],
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
        let host_bundle = crate::plugin_host_bundle::PluginHostBundle::from_plugin_dir(
            std::path::Path::new("examples/authentication/google"),
            "google",
            "1.0.0",
            &cowboy_plugin_sdk::PluginPackage::artifact_digest(&bytes),
        )
        .unwrap()
        .unwrap();
        let host_bytes = serde_json::to_vec(&host_bundle).unwrap();
        let host_digest = format!("sha256:{:x}", sha2::Sha256::digest(&host_bytes));
        let mut release = cowboy_plugin_sdk::PluginRelease {
            release_schema,
            plugin_id: "google".to_owned(),
            plugin_version: "1.0.0".to_owned(),
            plugin_kind: PluginKind::AuthenticationProvider,
            package_digest: cowboy_plugin_sdk::PluginPackage::artifact_digest(&bytes),
            artifact_digest: String::new(),
            artifact_url: "https://plugins.example/google.cowboy-plugin".to_owned(),
            publisher: "example-publisher".to_owned(),
            contract_fingerprint: package.contract_fingerprint.clone(),
            component_release: "2.0.3".to_owned(),
            host_bundle_digest: (release_schema == 2).then_some(host_digest),
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
        fs::write(catalog_root.join("orphan.cowboy-plugin"), &bytes).unwrap();
        if release_schema == 2 {
            fs::write(catalog_root.join("google.hostbundle.json"), host_bytes).unwrap();
        }
        fs::write(
            catalog_root.join("google.release.json"),
            serde_json::to_vec(&release).unwrap(),
        )
        .unwrap();

        let catalog = PluginCatalog::open(&root, Some(catalog_root.clone())).unwrap();
        assert!(catalog.released_plugins().is_empty());
        let requirements = catalog
            .entries()
            .into_iter()
            .find(|entry| entry.plugin_id == "google")
            .and_then(|entry| entry.compatibility_requirements)
            .expect("released Plugin compatibility requirements");
        assert_eq!(requirements.release_schema, release_schema);
        assert_eq!(
            requirements.host_bundle_schema,
            (release_schema == 2).then_some(1)
        );
        assert_eq!(requirements.host_schema, (release_schema == 2).then_some(1));
        assert_eq!(
            catalog
                .host_releases()
                .into_iter()
                .filter(|release| release.default_for_id && release.host_bundle.is_some())
                .count(),
            usize::from(release_schema == 2)
        );
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
        // shadow, replace or lend authority to this exact signed login.
        let storage = crate::plugin_storage::PluginStorage::sqlite_files(
            crate::plugin_dir::PluginDir::open(&root).unwrap(),
        );
        catalog.activate_runtime(&storage).await.unwrap();
        let future = catalog_root.join("future.cowboy-plugin");
        fs::write(&future, b"future package bytes").unwrap();
        assert_eq!(catalog.refresh_with_runtime(&storage).await.unwrap(), 1);
        fs::write(
            future.with_extension("release.json"),
            br#"{"release_schema":3,"plugin_id":"google","plugin_version":"1.0.0"}"#,
        )
        .unwrap();
        assert_eq!(catalog.refresh_with_runtime(&storage).await.unwrap(), 1);
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
        let before = catalog.state.read().clone();
        let mut bad_release = release.clone();
        bad_release.signature = base64::engine::general_purpose::STANDARD.encode([0_u8; 64]);
        fs::write(
            future.with_extension("release.json"),
            serde_json::to_vec(&bad_release).unwrap(),
        )
        .unwrap();
        fs::write(&future, &bytes).unwrap();
        assert!(
            catalog
                .refresh_with_runtime(&storage)
                .await
                .unwrap_err()
                .to_string()
                .contains("signature")
        );
        assert!(Arc::ptr_eq(&before, &catalog.state.read()));
        assert_eq!(
            catalog
                .resolve_authentication_provider("google", "1.0.0", &release.artifact_digest)
                .unwrap(),
            contract
        );
        assert!(PluginCatalog::open(&root, Some(catalog_root.clone())).is_err());
        drop(catalog);
        fs::remove_file(future.with_extension("release.json")).unwrap();
        fs::write(
            catalog_root.join("google.hostbundle.json"),
            br#"{"tampered":true}"#,
        )
        .unwrap();
        assert!(
            PluginCatalog::open(&root, Some(catalog_root.clone())).is_err(),
            "a host bundle outside the signed release identity was accepted"
        );
        drop(identity);
        drop(storage);
        drop(before);
        unlock_tree(&root);
        fs::remove_dir_all(root).unwrap();
    }

    fn publish_local_auth_fixture(root: &Path, id: &str, wrong_renderer: bool) -> PluginRelease {
        publish_auth_fixture(root, id, "1.0.0", |spec| {
            if wrong_renderer {
                spec["ui"]["renderers"]["login.method"] = serde_json::json!("login-oidc-v1");
            }
        })
    }

    #[tokio::test]
    async fn core_security_excludes_local_auth_defaults_and_rejects_their_pins() {
        let root = auth_test_root("core-security-pins");
        let password = publish_local_auth_fixture(&root, "password", false);
        let passkey = publish_local_auth_fixture(&root, "passkey", false);
        let mut catalog = PluginCatalog::open(&root, Some(root.join("external"))).unwrap();
        let mut policy = HostActivationPolicy::default();
        policy.source = HostSourcePolicy::CatalogOnly;
        policy.core_security = Some(
            serde_json::from_value(serde_json::json!({
                "namespace_id":"passkey", "source":"adopt_legacy"
            }))
            .unwrap(),
        );
        let authentication = crate::auth_plugins::ProductAuthentication::test_default(None);
        authentication.configure_host_policy(&mut policy).unwrap();
        assert!(authentication.password_enabled);
        assert!(!policy.require_webauthn_storage);
        assert!(policy.authentication_methods.is_empty());
        for release in [&password, &passkey] {
            let mut pinned = policy.clone();
            pinned.pin(release_pin(release), true).unwrap();
            assert!(catalog.configure_hosts(pinned).is_err());
            assert!(
                fs::read_dir(catalog.plugin_dir.live_root())
                    .unwrap()
                    .next()
                    .is_none()
            );
        }
        catalog.configure_hosts(policy).unwrap();
        let storage =
            crate::plugin_storage::PluginStorage::sqlite_files(catalog.plugin_dir.clone());
        let runtime = catalog.activate_runtime(&storage).await.unwrap();
        assert!(runtime.default_hosts().is_empty());
        assert!(runtime.namespace_for_capability("webauthn").is_none());
        assert!(authentication.public_host_plugins(&runtime).is_empty());
        assert!(
            !catalog
                .plugin_dir
                .plugin_live_dir("passkey")
                .unwrap()
                .join("state")
                .exists()
        );
        drop(runtime);
        drop(catalog);
        unlock_tree(&root);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_signed_non_telemetry_release_cannot_construct_a_telemetry_projection() {
        let fixture = tempfile::Builder::new()
            .prefix("cowboy-catalog-kind-")
            .tempdir()
            .unwrap();
        let release = publish_local_auth_fixture(fixture.path(), "password", false);
        let catalog =
            PluginCatalog::inspect(fixture.path(), Some(fixture.path().join("external"))).unwrap();
        assert!(
            catalog
                .resolve_telemetry_backend(
                    &release.plugin_id,
                    &release.plugin_version,
                    &release.artifact_digest
                )
                .is_err()
        );
    }

    fn publish_auth_fixture(
        root: &Path,
        id: &str,
        version: &str,
        edit_host: impl FnOnce(&mut serde_json::Value),
    ) -> PluginRelease {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/authentication")
            .join(id);
        let mut manifest: PluginManifest =
            serde_json::from_slice(&fs::read(source.join("plugin.json")).unwrap()).unwrap();
        manifest.version = version.to_owned();
        let mut contract: AuthenticationProviderContract =
            serde_json::from_slice(&fs::read(source.join(&manifest.entrypoint)).unwrap()).unwrap();
        contract.version = version.to_owned();
        let package = PluginPackage::new(
            manifest.clone(),
            manifest.component_release.clone(),
            cowboy_plugin_sdk::PluginPayload::AuthenticationProvider(contract),
        )
        .unwrap();
        let bytes = package.canonical_bytes().unwrap();
        let package_digest = PluginPackage::artifact_digest(&bytes);
        let mut host =
            PluginHostBundle::from_plugin_dir(&source, id, &manifest.version, &package_digest)
                .unwrap()
                .unwrap();
        let mut spec: serde_json::Value = serde_json::from_str(&host.files["host.json"]).unwrap();
        edit_host(&mut spec);
        host.files.insert("host.json".to_owned(), spec.to_string());
        publish_host_fixture(root, package, host, Vec::new())
    }

    fn publish_host_fixture(
        root: &Path,
        package: PluginPackage,
        host: PluginHostBundle,
        runtime_artifacts: Vec<cowboy_plugin_sdk::PluginRuntimeArtifacts>,
    ) -> PluginRelease {
        let manifest = &package.manifest;
        let id = &manifest.id;
        let version = &manifest.version;
        let bytes = package.canonical_bytes().unwrap();
        let host_bytes = serde_json::to_vec(&host).unwrap();
        let mut release = PluginRelease {
            release_schema: 2,
            plugin_id: id.clone(),
            plugin_version: version.clone(),
            plugin_kind: manifest.kind,
            package_digest: PluginPackage::artifact_digest(&bytes),
            artifact_digest: String::new(),
            artifact_url: format!("https://plugins.example/{id}.cowboy-plugin"),
            publisher: manifest.publisher.clone(),
            contract_fingerprint: package.contract_fingerprint.clone(),
            component_release: manifest.component_release.clone(),
            host_bundle_digest: Some(format!("sha256:{:x}", sha2::Sha256::digest(&host_bytes))),
            signature: String::new(),
            supported_platforms: runtime_artifacts
                .iter()
                .map(|target| PlatformTarget {
                    os: target.os.clone(),
                    architecture: target.architecture.clone(),
                })
                .collect(),
            runtime_artifacts,
        };
        let identity =
            crate::machine_auth::MachineIdentity::load_or_create(&root.join("identity")).unwrap();
        release.artifact_digest = release.computed_artifact_digest().unwrap();
        release.signature = identity
            .sign_namespaced(PLUGIN_RELEASE_SIGNATURE_NAMESPACE, &release.proof())
            .unwrap();
        let catalog = root.join("external");
        fs::create_dir_all(catalog.join("trusted-publishers")).unwrap();
        fs::write(
            catalog
                .join("trusted-publishers")
                .join(format!("{}.pub", release.publisher)),
            identity.public_key(),
        )
        .unwrap();
        let stem = if version == "1.0.0" {
            id.to_owned()
        } else {
            format!("{id}-{version}")
        };
        fs::write(catalog.join(format!("{stem}.cowboy-plugin")), bytes).unwrap();
        fs::write(catalog.join(format!("{stem}.hostbundle.json")), host_bytes).unwrap();
        fs::write(
            catalog.join(format!("{stem}.release.json")),
            serde_json::to_vec(&release).unwrap(),
        )
        .unwrap();
        release
    }

    fn storage_host_fixture(migrations: u8) -> serde_json::Value {
        let mut host = serde_json::json!({
            "schema_version": 1,
            "label": "Fixture storage",
            "native_capabilities": ["fixture-storage"],
        });
        if migrations > 0 {
            let mut sql = vec![serde_json::json!({
                "version": "0001", "sql": "CREATE TABLE notes (id TEXT PRIMARY KEY);"
            })];
            if migrations > 1 {
                sql.push(serde_json::json!({
                    "version": "0002", "sql": "CREATE TABLE upgrade_only (id TEXT PRIMARY KEY);"
                }));
            }
            host["storage"] = serde_json::json!({
                "postgres": {"migrations": sql}, "sqlite": {"migrations": sql}
            });
        }
        host
    }

    // The Controller must not fetch or execute the fixture's declared Machine
    // adapter. Its full signed envelope still crosses real Catalog validation.
    fn publish_storage_fixture(
        root: &Path,
        id: &str,
        version: &str,
        migrations: u8,
    ) -> PluginRelease {
        let mut manifest: PluginManifest =
            serde_json::from_str(include_str!("../plugins/zed/plugin.json")).unwrap();
        manifest.id = id.to_owned();
        manifest.version = version.to_owned();
        let mut contract: cowboy_plugin_sdk::CodeIntelligenceContract =
            serde_json::from_str(include_str!("../plugins/zed/contract.json")).unwrap();
        contract.id = id.to_owned();
        contract.version = version.to_owned();
        contract.schema_version = 1;
        contract.runtime = None;
        let runtime_artifacts = contract
            .supported_platforms
            .iter()
            .map(|target| {
                serde_json::from_value(serde_json::json!({
                    "os": target.os, "architecture": target.architecture,
                    "components": [{
                        "kind": "code_intelligence_adapter", "slot": id,
                        "dependency": "fixture-adapter", "version": "1.0.0",
                        "command": "fixture-adapter", "artifact_format": "raw",
                        "artifact_url": "https://must-not-be-contacted.invalid/adapter",
                        "artifact_digest": format!("sha256:{:x}", sha2::Sha256::digest(b"fixture")),
                        "probe": {"args": ["--help"], "timeout_ms": 1000}
                    }]
                }))
                .unwrap()
            })
            .collect();
        let package = PluginPackage::new(
            manifest.clone(),
            manifest.component_release.clone(),
            cowboy_plugin_sdk::PluginPayload::CodeIntelligence(contract),
        )
        .unwrap();
        let host = PluginHostBundle {
            schema: crate::plugin_host_bundle::HOST_BUNDLE_SCHEMA.to_owned(),
            plugin_id: id.to_owned(),
            plugin_version: version.to_owned(),
            package_digest: PluginPackage::artifact_digest(&package.canonical_bytes().unwrap()),
            files: BTreeMap::from([(
                "host.json".to_owned(),
                storage_host_fixture(migrations).to_string(),
            )]),
        };
        package.validate_host_contract(Some(&host.files)).unwrap();
        publish_host_fixture(root, package, host, runtime_artifacts)
    }

    #[test]
    fn every_released_storage_kind_requires_an_exact_pin_in_both_source_modes() {
        let root = auth_test_root("storage-policy");
        let old = publish_storage_fixture(&root, "future-storage", "1.0.0", 1);
        publish_storage_fixture(&root, "future-storage", "2.0.0", 2);
        let catalog = PluginCatalog::inspect(&root, Some(root.join("external"))).unwrap();
        let before = state_fingerprint(&root);
        for source in [HostSourcePolicy::Bootstrap, HostSourcePolicy::CatalogOnly] {
            for kind in [
                PluginKind::AgentProvider,
                PluginKind::CodeIntelligence,
                PluginKind::AuthenticationProvider,
            ] {
                // Policy unit fixture: payload verification already precedes
                // selection; kind must not change storage authorization.
                let mut releases = catalog.host_releases();
                for release in &mut releases {
                    release.entry.plugin_kind = kind;
                }
                let mut policy = HostActivationPolicy::default();
                policy.source = source;
                policy.select(&mut releases).unwrap();
                assert!(releases.iter().all(|release| !release.default_for_id));
                policy.pin(release_pin(&old), true).unwrap();
                policy.select(&mut releases).unwrap();
                let defaults = releases
                    .iter()
                    .filter(|release| release.default_for_id)
                    .collect::<Vec<_>>();
                assert_eq!(defaults.len(), 1);
                assert_eq!(
                    defaults[0].entry.artifact_digest.as_deref(),
                    Some(old.artifact_digest.as_str())
                );
            }
        }
        assert_eq!(before, state_fingerprint(&root));
        unlock_tree(&root);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn sqlite_storage_publication_requires_explicit_activation() {
        assert_storage_publication_requires_explicit_activation(None).await;
    }

    #[tokio::test]
    async fn stateless_defaults_cannot_acquire_storage_on_catalog_refresh() {
        for source in [HostSourcePolicy::Bootstrap, HostSourcePolicy::CatalogOnly] {
            let root = auth_test_root("stateless-to-storage");
            let id = "future-stateless";
            publish_storage_fixture(&root, id, "1.0.0", 0);
            let mut catalog = PluginCatalog::open(&root, Some(root.join("external"))).unwrap();
            let mut policy = HostActivationPolicy::default();
            policy.source = source;
            catalog.configure_hosts(policy).unwrap();
            let storage =
                crate::plugin_storage::PluginStorage::sqlite_files(catalog.plugin_dir.clone());
            let first = catalog.activate_runtime(&storage).await.unwrap();
            assert_eq!(
                first.default_host(id).unwrap().plugin_version.as_deref(),
                Some("1.0.0")
            );
            let second = publish_storage_fixture(&root, id, "2.0.0", 0);
            catalog.refresh_with_runtime(&storage).await.unwrap();
            assert_eq!(
                catalog
                    .runtime()
                    .unwrap()
                    .default_host(id)
                    .unwrap()
                    .artifact_digest
                    .as_deref(),
                Some(second.artifact_digest.as_str())
            );

            let stateful = publish_storage_fixture(&root, id, "3.0.0", 1);
            catalog.refresh_with_runtime(&storage).await.unwrap();
            let runtime = catalog.runtime().unwrap();
            assert!(
                runtime.default_host(id).is_none(),
                "unselected storage must not fall back to an older stateless default"
            );
            assert!(
                runtime
                    .exact_host(id, "3.0.0", &stateful.artifact_digest)
                    .is_some()
            );
            assert!(
                runtime
                    .namespace_for_capability("fixture-storage")
                    .is_none()
            );
            assert!(
                !catalog
                    .plugin_dir
                    .live_root()
                    .join(id)
                    .join("state")
                    .exists()
            );
            let report = serde_json::to_value(catalog.host_preflight_report().unwrap()).unwrap();
            assert!(
                report["catalog_defaults"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|host| host["release"]["plugin_id"] != id)
            );
            unlock_tree(&root);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn bootstrap_storage_selection_is_checked_before_database_or_state_creation() {
        let root = auth_test_root("bootstrap-storage-preflight");
        let passkey = publish_local_auth_fixture(&root, "passkey", false);
        let data_dir = root.join("controller");
        let args = || {
            let mut args = crate::cli::ServeArgs::test_plugin_check(&data_dir);
            args.plugin_catalog_dir = Some(root.join("external"));
            args.database_url = Some("postgresql://fixture@127.0.0.1:1/never-connect".to_owned());
            args
        };
        let before = state_fingerprint(&root);
        let mut failures = Vec::new();
        for check in [true, false] {
            let mut args = args();
            args.check_plugin_hosts = check;
            failures.push(format!(
                "{:#}",
                crate::server::serve(args).await.unwrap_err()
            ));
            assert!(!data_dir.exists());
            assert_eq!(before, state_fingerprint(&root));
        }
        assert_eq!(failures[0], failures[1]);
        assert!(failures[0].contains("exact host selection"));
        let policy_path = write_host_policy(&root, &[&passkey]);
        let mut policy: serde_json::Value =
            serde_json::from_slice(&fs::read(&policy_path).unwrap()).unwrap();
        policy["source_policy"] = serde_json::json!("bootstrap");
        fs::write(&policy_path, serde_json::to_vec(&policy).unwrap()).unwrap();
        let before = state_fingerprint(&root);
        let mut args = args();
        args.plugin_host_config = Some(policy_path);
        crate::server::serve(args).await.unwrap();
        assert!(!data_dir.exists());
        assert_eq!(before, state_fingerprint(&root));

        // A restart after Catalog files disappear must respect the existing
        // authority marker instead of resurrecting bundled credential storage.
        let dir = crate::plugin_dir::PluginDir::open(&data_dir).unwrap();
        dir.record_catalog_authority("passkey").unwrap();
        for extension in ["cowboy-plugin", "release.json", "hostbundle.json"] {
            fs::remove_file(root.join("external").join(format!("passkey.{extension}"))).unwrap();
        }
        let before = state_fingerprint(&root);
        let mut args = crate::cli::ServeArgs::test_plugin_check(&data_dir);
        args.plugin_catalog_dir = Some(root.join("external"));
        args.database_url = Some("postgresql://fixture@127.0.0.1:1/never-connect".to_owned());
        let error = crate::server::serve(args).await.unwrap_err();
        assert!(format!("{error:#}").contains("exact host selection"));
        assert_eq!(before, state_fingerprint(&root));
        unlock_tree(&root);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    #[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
    async fn postgres_storage_publication_requires_explicit_activation() {
        let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
            .expect("run nix develop -c just test-postgres");
        assert_storage_publication_requires_explicit_activation(Some(&url)).await;
    }

    async fn assert_storage_publication_requires_explicit_activation(postgres_url: Option<&str>) {
        for source in [HostSourcePolicy::Bootstrap, HostSourcePolicy::CatalogOnly] {
            let id = if source == HostSourcePolicy::Bootstrap {
                "future-storage-bootstrap"
            } else {
                "future-storage-catalog"
            };
            let root = auth_test_root(id);
            let store = crate::store::Store::connect(
                postgres_url.unwrap_or("sqlite::memory:"),
                root.join("artifacts"),
            )
            .await
            .unwrap();
            store.migrate().await.unwrap();
            let dir = crate::plugin_dir::PluginDir::open(&root).unwrap();
            let storage = store.plugin_storage(dir);
            assert_storage_activation_sequence(&root, id, source, &store, &storage).await;
            drop(store);
            unlock_tree(&root);
            fs::remove_dir_all(root).unwrap();
        }
    }

    async fn assert_storage_activation_sequence(
        root: &Path,
        id: &str,
        source: HostSourcePolicy,
        store: &crate::store::Store,
        storage: &crate::plugin_storage::PluginStorage,
    ) {
        let catalog_root = root.join("external");
        let first = publish_storage_fixture(root, id, "1.0.0", 1);
        let mut catalog = PluginCatalog::open(root, Some(catalog_root.clone())).unwrap();
        let mut policy = HostActivationPolicy::default();
        policy.source = source;
        catalog.configure_hosts(policy.clone()).unwrap();
        let inactive = catalog.activate_runtime(storage).await.unwrap();
        assert!(inactive.default_host(id).is_none());
        assert!(
            inactive
                .exact_host(id, "1.0.0", &first.artifact_digest)
                .is_some()
        );
        assert!(
            inactive
                .namespace_for_capability("fixture-storage")
                .is_none()
        );
        assert!(
            !storage
                .plugin_dir()
                .live_root()
                .join(id)
                .join("state")
                .exists()
        );
        if let Some(pool) = store.postgres_pool() {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM information_schema.schemata WHERE schema_name = $1",
            )
            .bind(crate::plugin_host::postgres_schema_name(id).unwrap())
            .fetch_one(pool)
            .await
            .unwrap();
            assert_eq!(count, 0, "publication created a PostgreSQL namespace");
        }

        policy.pin(release_pin(&first), true).unwrap();
        let mut selected = PluginCatalog::open(root, Some(catalog_root.clone())).unwrap();
        selected.configure_hosts(policy.clone()).unwrap();
        let active = selected.activate_runtime(storage).await.unwrap();
        let namespace = active.namespace_for_capability("fixture-storage").unwrap();
        namespace
            .execute("INSERT INTO notes (id) VALUES ('keep-me')")
            .await
            .unwrap();
        let second = publish_storage_fixture(root, id, "2.0.0", 2);
        selected.refresh_with_runtime(storage).await.unwrap();
        let refreshed = selected.runtime().unwrap();
        assert_eq!(
            refreshed
                .default_host(id)
                .unwrap()
                .artifact_digest
                .as_deref(),
            Some(first.artifact_digest.as_str())
        );
        assert!(
            refreshed
                .exact_host(id, "2.0.0", &second.artifact_digest)
                .is_some()
        );
        assert!(
            namespace
                .fetch_i64("SELECT COUNT(*) FROM upgrade_only")
                .await
                .is_err()
        );
        assert_eq!(
            namespace
                .fetch_i64("SELECT COUNT(*) FROM _cowboy_plugin_migrations")
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            namespace
                .fetch_i64("SELECT COUNT(*) FROM notes")
                .await
                .unwrap(),
            1
        );

        let mut unselected = PluginCatalog::open(root, Some(catalog_root.clone())).unwrap();
        let mut unselected_policy = HostActivationPolicy::default();
        unselected_policy.source = source;
        unselected.configure_hosts(unselected_policy).unwrap();
        let inactive_restart = unselected.activate_runtime(storage).await.unwrap();
        assert!(inactive_restart.default_host(id).is_none());
        assert!(
            inactive_restart
                .namespace_for_capability("fixture-storage")
                .is_none()
        );
        assert_eq!(
            namespace
                .fetch_i64("SELECT COUNT(*) FROM _cowboy_plugin_migrations")
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            namespace
                .fetch_i64("SELECT COUNT(*) FROM notes")
                .await
                .unwrap(),
            1
        );

        let mut upgrade_policy = HostActivationPolicy::default();
        upgrade_policy.source = source;
        upgrade_policy.pin(release_pin(&second), true).unwrap();
        let mut upgraded = PluginCatalog::open(root, Some(catalog_root.clone())).unwrap();
        upgraded.configure_hosts(upgrade_policy).unwrap();
        let upgraded_runtime = upgraded.activate_runtime(storage).await.unwrap();
        let upgraded_namespace = upgraded_runtime
            .namespace_for_capability("fixture-storage")
            .unwrap();
        assert_eq!(
            upgraded_namespace
                .fetch_i64("SELECT COUNT(*) FROM upgrade_only")
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            upgraded_namespace
                .fetch_i64("SELECT COUNT(*) FROM _cowboy_plugin_migrations")
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            upgraded_namespace
                .fetch_i64("SELECT COUNT(*) FROM notes")
                .await
                .unwrap(),
            1
        );

        // Removing the exact selection cannot silently choose a newer release.
        for extension in ["cowboy-plugin", "release.json", "hostbundle.json"] {
            fs::remove_file(catalog_root.join(format!("{id}.{extension}"))).unwrap();
        }
        let before = state_fingerprint(root);
        assert!(selected.refresh_with_runtime(storage).await.is_err());
        assert!(Arc::ptr_eq(&refreshed, &selected.runtime().unwrap()));
        assert_eq!(before, state_fingerprint(root));
    }

    fn release_pin(release: &PluginRelease) -> crate::plugin_activation::HostReleasePin {
        crate::plugin_activation::HostReleasePin {
            plugin_id: release.plugin_id.clone(),
            plugin_version: release.plugin_version.clone(),
            artifact_digest: release.artifact_digest.clone(),
        }
    }

    fn auth_test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "cowboy-auth-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_host_policy(root: &Path, releases: &[&PluginRelease]) -> PathBuf {
        let path = root.join("host-policy.json");
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "schema":"dravengarden.cowboy.plugin-host-activation/v1",
                "source_policy":"catalog_only",
                "hosts": releases.iter().map(|release| release_pin(release)).collect::<Vec<_>>()
            }))
            .unwrap(),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    fn state_fingerprint(root: &Path) -> BTreeMap<PathBuf, (u32, Vec<u8>)> {
        fn visit(root: &Path, path: &Path, state: &mut BTreeMap<PathBuf, (u32, Vec<u8>)>) {
            let metadata = fs::symlink_metadata(path).unwrap();
            assert!(
                !metadata.file_type().is_symlink(),
                "unexpected fixture symlink"
            );
            let digest = if metadata.is_file() {
                sha2::Sha256::digest(fs::read(path).unwrap()).to_vec()
            } else {
                Vec::new()
            };
            state.insert(
                path.strip_prefix(root).unwrap().to_owned(),
                (metadata.permissions().mode(), digest),
            );
            if metadata.is_dir() {
                for entry in fs::read_dir(path).unwrap() {
                    visit(root, &entry.unwrap().path(), state);
                }
            }
        }
        let mut state = BTreeMap::new();
        visit(root, root, &mut state);
        state
    }

    #[tokio::test]
    async fn plugin_preflight_verifies_exact_releases_without_mutating_or_contacting_services() {
        let root = auth_test_root("read-only-check");
        let password = publish_local_auth_fixture(&root, "password", false);
        let passkey = publish_local_auth_fixture(&root, "passkey", false);
        publish_auth_fixture(&root, "password", "2.0.0", |_| {});
        let path = write_host_policy(&root, &[&password, &passkey]);
        let data_dir = root.join("controller");
        let mut args = crate::cli::ServeArgs::test_plugin_check(&data_dir);
        args.plugin_catalog_dir = Some(root.join("external"));
        args.plugin_host_config = Some(path.clone());
        args.product_auth_enabled = true;
        // The check may infer a durable-store requirement, but cannot connect,
        // print this URL, or claim to have checked storage/credentials.
        args.database_url = Some("postgres://fixture-secret@127.0.0.1:1/unreachable".to_owned());
        args.machine_components_manifest = Some(root.join("missing-components.json"));
        let before = state_fingerprint(&root);
        let (catalog, _, _) = crate::plugin_activation::prepare_controller_hosts(&args).unwrap();
        let report = serde_json::to_value(catalog.host_preflight_report().unwrap()).unwrap();
        assert_eq!(
            report["schema"],
            "dravengarden.cowboy.plugin-host-preflight/v1"
        );
        assert_eq!(report["status"], "configuration_valid");
        assert_eq!(report["source_policy"], "catalog_only");
        assert_eq!(report["catalog_only_recorded"], false);
        assert_eq!(report["webauthn_storage_required"], true);
        assert_eq!(report["exact_selections"].as_array().unwrap().len(), 2);
        assert_eq!(report["catalog_defaults"].as_array().unwrap().len(), 2);
        assert!(
            report["catalog_defaults"]
                .as_array()
                .unwrap()
                .iter()
                .all(|host| host["release"]["plugin_version"] == "1.0.0")
        );
        assert_eq!(
            report["not_checked"],
            serde_json::json!([
                "runtime_artifact_bytes",
                "generation_staging",
                "database_migrations",
                "credential_import",
                "live_authentication"
            ])
        );
        let output = report.to_string();
        assert!(
            !output.contains("fixture-secret")
                && !output.contains("127.0.0.1")
                && !output.contains("CREATE TABLE")
        );
        assert!(catalog.runtime().is_none());
        crate::server::serve(args).await.unwrap();
        assert!(!data_dir.exists());
        assert_eq!(before, state_fingerprint(&root));

        // A bad signature is rejected through the same read-only verifier.
        let release_path = root.join("external/password.release.json");
        let mut invalid = password;
        invalid.signature = "invalid".to_owned();
        fs::write(&release_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let before = state_fingerprint(&root);
        let mut args = crate::cli::ServeArgs::test_plugin_check(&data_dir);
        args.plugin_catalog_dir = Some(root.join("external"));
        args.plugin_host_config = Some(path);
        assert!(crate::plugin_activation::prepare_controller_hosts(&args).is_err());
        assert_eq!(before, state_fingerprint(&root));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn plugin_preflight_and_startup_reject_unready_hosts_before_creating_state() {
        let root = auth_test_root("read-only-failure");
        fs::create_dir(&root).unwrap();
        let path = write_host_policy(&root, &[]);
        let data_dir = root.join("controller");
        let before = state_fingerprint(&root);
        let mut failures = Vec::new();
        for check in [true, false] {
            let mut args = crate::cli::ServeArgs::test_plugin_check(&data_dir);
            args.product_auth_enabled = true;
            args.plugin_host_config = Some(path.clone());
            args.check_plugin_hosts = check;
            failures.push(format!(
                "{:#}",
                crate::server::serve(args).await.unwrap_err()
            ));
            assert!(!data_dir.exists());
        }
        assert_eq!(
            failures[0], failures[1],
            "check and startup used different readiness rules"
        );
        assert!(failures[0].contains("login method password"));
        assert_eq!(before, state_fingerprint(&root));

        // A completed cutover still forbids a missing policy, including in
        // inspection mode. Checking must never rewrite the receipt.
        let dir = crate::plugin_dir::PluginDir::open(&data_dir).unwrap();
        dir.record_catalog_only().unwrap();
        let before = state_fingerprint(&root);
        let args = crate::cli::ServeArgs::test_plugin_check(&data_dir);
        let error = crate::server::serve(args).await.unwrap_err();
        assert!(format!("{error:#}").contains("previously activated catalog_only"));
        assert_eq!(before, state_fingerprint(&root));
        let mut args = crate::cli::ServeArgs::test_plugin_check(&data_dir);
        args.plugin_host_config = Some(path);
        let (catalog, _, _) = crate::plugin_activation::prepare_controller_hosts(&args).unwrap();
        let report = serde_json::to_value(catalog.host_preflight_report().unwrap()).unwrap();
        assert_eq!(report["catalog_only_recorded"], true);
        assert_eq!(before, state_fingerprint(&root));
        unlock_tree(&root);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn catalog_only_cutover_pins_storage_and_does_not_activate_new_publications() {
        let root = auth_test_root("pinned-cutover");
        let dir = crate::plugin_dir::PluginDir::open(&root).unwrap();
        let storage = crate::plugin_storage::PluginStorage::sqlite_files(dir);
        assert_catalog_only_cutover(root, storage).await;
    }

    #[tokio::test]
    #[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
    async fn postgres_catalog_only_cutover_pins_storage_and_does_not_activate_new_publications() {
        let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
            .expect("run nix develop -c just test-postgres");
        let root = auth_test_root("postgres-pinned-cutover");
        let store = crate::store::Store::connect(&url, root.join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let storage = store.plugin_storage(crate::plugin_dir::PluginDir::open(&root).unwrap());
        assert_catalog_only_cutover(root, storage).await;
    }

    async fn assert_catalog_only_cutover(
        root: PathBuf,
        storage: crate::plugin_storage::PluginStorage,
    ) {
        let catalog_root = root.join("external");
        let dir = storage.plugin_dir();
        let bootstrap = PluginCatalog::open(&root, Some(catalog_root.clone())).unwrap();
        let old = bootstrap.activate_runtime(&storage).await.unwrap();
        old.namespace_for_capability("webauthn").unwrap().execute(
            "INSERT INTO user_passkeys (id, user_id, credential_id, nickname, passkey_json, created_at_ms) \
             VALUES ('keep-this-passkey', 'fixture-user', 'fixture-credential', 'Fixture', '{}', 1)"
        )
        .await
        .unwrap();

        let password = publish_local_auth_fixture(&root, "password", false);
        let passkey = publish_local_auth_fixture(&root, "passkey", false);
        let mut policy = HostActivationPolicy::default();
        policy.source = HostSourcePolicy::CatalogOnly;
        policy.pin(release_pin(&password), true).unwrap();
        policy.pin(release_pin(&passkey), true).unwrap();
        let authentication = crate::auth_plugins::ProductAuthentication::test_default(None);
        authentication.configure_host_policy(&mut policy).unwrap();
        let mut catalog = PluginCatalog::open(&root, Some(catalog_root.clone())).unwrap();
        catalog.configure_hosts(policy.clone()).unwrap();
        assert!(
            !dir.catalog_only_required().unwrap(),
            "preflight cannot commit the cutover"
        );
        let active = catalog.activate_runtime(&storage).await.unwrap();
        assert!(dir.catalog_only_required().unwrap());
        assert_eq!(
            active.default_hosts().len(),
            2,
            "bootstrap hosts leaked into catalog_only"
        );
        assert_eq!(
            active
                .default_host("passkey")
                .unwrap()
                .artifact_digest
                .as_deref(),
            Some(passkey.artifact_digest.as_str())
        );
        assert_eq!(
            active
                .namespace_for_capability("webauthn")
                .unwrap()
                .fetch_i64("SELECT COUNT(*) FROM user_passkeys WHERE user_id = 'fixture-user'")
                .await
                .unwrap(),
            1
        );
        assert!(
            catalog.configure_hosts(policy.clone()).is_err(),
            "running host policy must be immutable"
        );

        let future_passkey = publish_auth_fixture(&root, "passkey", "2.0.0", |host| {
            for dialect in ["postgres", "sqlite"] {
                host["storage"][dialect]["migrations"].as_array_mut().unwrap().push(serde_json::json!({
                    "version":"0002", "sql":"CREATE TABLE future_release_only (id TEXT PRIMARY KEY);"
                }));
            }
        });
        publish_auth_fixture(&root, "password", "2.0.0", |_| {});
        assert_eq!(catalog.refresh_with_runtime(&storage).await.unwrap(), 4);
        let refreshed = catalog.runtime().unwrap();
        assert_eq!(
            refreshed
                .default_host("passkey")
                .unwrap()
                .artifact_digest
                .as_deref(),
            Some(passkey.artifact_digest.as_str())
        );
        assert_eq!(
            refreshed
                .default_host("password")
                .unwrap()
                .artifact_digest
                .as_deref(),
            Some(password.artifact_digest.as_str())
        );
        assert_eq!(refreshed.public_descriptors().len(), 4);
        let namespace = refreshed.namespace_for_capability("webauthn").unwrap();
        let count: i64 = match &namespace.backend {
            crate::plugin_storage::NamespaceBackend::Sqlite(pool) => sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'future_release_only'",
            )
            .fetch_one(pool)
            .await
            .unwrap(),
            crate::plugin_storage::NamespaceBackend::Postgres { pool, schema } => {
                sqlx::query_scalar(
                    "SELECT COUNT(*) FROM information_schema.tables \
                     WHERE table_schema = $1 AND table_name = 'future_release_only'",
                )
                .bind(schema)
                .fetch_one(pool)
                .await
                .unwrap()
            }
        };
        assert_eq!(
            count, 0,
            "publishing an unselected release ran its migration"
        );
        assert_eq!(
            namespace
                .fetch_i64("SELECT COUNT(*) FROM user_passkeys WHERE user_id = 'fixture-user'")
                .await
                .unwrap(),
            1
        );

        // Even a separately selected, validly signed release cannot run new
        // Passkey SQL. Reject before any namespace/authority filesystem writes.
        let before_incompatible = state_fingerprint(&root);
        for source in [HostSourcePolicy::Bootstrap, HostSourcePolicy::CatalogOnly] {
            let mut incompatible = HostActivationPolicy::default();
            incompatible.source = source;
            incompatible.pin(release_pin(&password), true).unwrap();
            incompatible
                .pin(release_pin(&future_passkey), true)
                .unwrap();
            authentication
                .configure_host_policy(&mut incompatible)
                .unwrap();
            // Pure policy validation also covers Bootstrap even though this
            // fixture has already recorded its Catalog-only authority.
            let error = incompatible
                .select(&mut catalog.host_releases())
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("CoreSecurity Passkey schema is frozen")
            );
        }
        assert_eq!(before_incompatible, state_fingerprint(&root));
        assert!(Arc::ptr_eq(&refreshed, &catalog.runtime().unwrap()));

        let mut restarted = PluginCatalog::open(&root, Some(catalog_root.clone())).unwrap();
        let error = restarted
            .configure_hosts(HostActivationPolicy::default())
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("previously activated catalog_only")
        );
        assert!(restarted.activate_runtime(&storage).await.is_err());
        restarted.configure_hosts(policy.clone()).unwrap();
        assert_eq!(
            restarted
                .activate_runtime(&storage)
                .await
                .unwrap()
                .default_hosts()
                .len(),
            2
        );

        for extension in ["cowboy-plugin", "release.json", "hostbundle.json"] {
            fs::remove_file(catalog_root.join(format!("passkey.{extension}"))).unwrap();
        }
        // An unsupported publication claiming the missing pin is not trusted
        // identity and cannot authorize a host/storage fallback on refresh or
        // restart, even after catalog_only authority has been recorded.
        fs::write(
            catalog_root.join("future.cowboy-plugin"),
            b"opaque future package",
        )
        .unwrap();
        fs::write(
            catalog_root.join("future.release.json"),
            serde_json::to_vec(&serde_json::json!({
                "release_schema": 3,
                "plugin_id": passkey.plugin_id,
                "plugin_version": passkey.plugin_version,
                "artifact_digest": passkey.artifact_digest,
            }))
            .unwrap(),
        )
        .unwrap();
        let before = state_fingerprint(&root);
        let error = catalog.refresh_with_runtime(&storage).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("missing from the trusted Catalog")
        );
        assert!(Arc::ptr_eq(&refreshed, &catalog.runtime().unwrap()));
        assert!(
            catalog
                .entries()
                .iter()
                .any(|entry| entry.artifact_digest.as_deref()
                    == Some(passkey.artifact_digest.as_str())),
            "failed refresh changed the public Catalog snapshot"
        );
        let mut missing_pin = PluginCatalog::open(&root, Some(catalog_root)).unwrap();
        assert!(missing_pin.configure_hosts(policy).is_err());
        assert!(missing_pin.runtime().is_none());
        assert_eq!(before, state_fingerprint(&root));
        unlock_tree(&root);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn catalog_only_preflight_requires_enabled_methods_and_durable_admin_storage() {
        let root = auth_test_root("readiness");
        let password = publish_local_auth_fixture(&root, "password", false);
        let passkey = publish_local_auth_fixture(&root, "passkey", false);
        let mut catalog = PluginCatalog::open(&root, Some(root.join("external"))).unwrap();
        let mut policy = HostActivationPolicy::default();
        policy.source = HostSourcePolicy::CatalogOnly;
        crate::auth_plugins::ProductAuthentication::test_default(None)
            .configure_host_policy(&mut policy)
            .unwrap();
        assert!(
            catalog
                .configure_hosts(policy.clone())
                .unwrap_err()
                .to_string()
                .contains("login method password")
        );
        policy.pin(release_pin(&password), true).unwrap();
        assert!(
            catalog
                .configure_hosts(policy.clone())
                .unwrap_err()
                .to_string()
                .contains("WebAuthn storage host")
        );
        policy.pin(release_pin(&passkey), true).unwrap();
        catalog.configure_hosts(policy.clone()).unwrap();
        let mut hostless = catalog.host_releases();
        hostless
            .iter_mut()
            .find(|release| release.entry.plugin_id == "password")
            .unwrap()
            .host_bundle = None;
        assert!(
            policy
                .select(&mut hostless)
                .unwrap_err()
                .to_string()
                .contains("release-bound host bundle")
        );
        let mut wrong_method = policy.clone();
        wrong_method.authentication_methods.insert(
            "password".to_owned(),
            cowboy_plugin_sdk::PluginRendererId::LoginOidcV1,
        );
        assert!(
            wrong_method
                .select(&mut catalog.host_releases())
                .unwrap_err()
                .to_string()
                .contains("configured login method")
        );
        let mut ambiguous_storage = catalog.host_releases();
        let mut duplicate = ambiguous_storage
            .iter()
            .find(|release| release.entry.plugin_id == "passkey")
            .unwrap()
            .clone();
        duplicate.entry.plugin_id = "another-passkey".to_owned();
        let mut duplicate_pin = release_pin(&passkey);
        duplicate_pin.plugin_id = duplicate.entry.plugin_id.clone();
        ambiguous_storage.push(duplicate);
        let mut ambiguous_policy = policy;
        ambiguous_policy.pin(duplicate_pin, true).unwrap();
        assert!(
            ambiguous_policy
                .select(&mut ambiguous_storage)
                .unwrap_err()
                .to_string()
                .contains("exactly one")
        );

        let mut admin_only = HostActivationPolicy::default();
        admin_only.source = HostSourcePolicy::CatalogOnly;
        crate::auth_plugins::ProductAuthentication::disabled()
            .configure_host_policy(&mut admin_only)
            .unwrap();
        admin_only.require_webauthn_storage = true;
        assert!(
            catalog.configure_hosts(admin_only.clone()).is_err(),
            "admin storage must not depend on Product login enablement"
        );
        admin_only.pin(release_pin(&passkey), true).unwrap();
        catalog.configure_hosts(admin_only).unwrap();
        let dir = &catalog.plugin_dir;
        assert!(!dir.catalog_only_required().unwrap());
        assert!(
            fs::read_dir(dir.live_root()).unwrap().next().is_none(),
            "preflight staged generations or storage"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn oidc_driver_and_public_host_share_the_configured_exact_release() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let root = auth_test_root("oidc-pin");
        let google = publish_auth_fixture(&root, "google", "1.0.0", |_| {});
        let newer = publish_auth_fixture(&root, "google", "2.0.0", |_| {});
        publish_auth_fixture(&root, "apple", "1.0.0", |_| {});
        let mut catalog = PluginCatalog::open(&root, Some(root.join("external"))).unwrap();
        let secret_path = root.join("oidc-secret");
        fs::write(&secret_path, "hermetic-fixture-not-a-credential").unwrap();
        fs::set_permissions(&secret_path, fs::Permissions::from_mode(0o600)).unwrap();
        let path = root.join("authentication.json");
        fs::write(&path, serde_json::to_vec(&serde_json::json!({
            "schema": "dravengarden.cowboy.authentication/v2",
            "password": {"enabled":false},
            "passkeys": {"enabled":false,"prompt_after_login":false,"session_refresh_enabled":false},
            "providers":[{
                "plugin_id": google.plugin_id, "plugin_version":google.plugin_version,
                "artifact_digest":google.artifact_digest,
                "oidc": {
                    "client_id":"fixture-client", "redirect_uri":"https://cowboy.example/api/auth/providers/google/callback",
                    "subject":"fixture-subject", "account":"fixture-user",
                    "client_authentication":{"method":"client_secret_post", "client_secret_file":secret_path}
                }
            }]
        })).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let authentication =
            crate::auth_plugins::ProductAuthentication::load(Some(&path), &catalog, None).unwrap();
        let mut policy = HostActivationPolicy::default();
        policy.source = HostSourcePolicy::CatalogOnly;
        authentication.configure_host_policy(&mut policy).unwrap();
        catalog.configure_hosts(policy.clone()).unwrap();
        // Core local security leaves the external-only login policy intact:
        // exact signed OIDC remains required, and no password fallback appears.
        let mut core_policy = policy.clone();
        core_policy.core_security = Some(
            serde_json::from_value(serde_json::json!({
                "namespace_id":"passkey", "source":"fresh"
            }))
            .unwrap(),
        );
        authentication
            .configure_host_policy(&mut core_policy)
            .unwrap();
        assert!(!authentication.password_enabled);
        assert_eq!(core_policy.authentication_methods.len(), 1);
        assert!(core_policy.authentication_methods.contains_key("google"));
        catalog.configure_hosts(core_policy).unwrap();
        let storage =
            crate::plugin_storage::PluginStorage::sqlite_files(catalog.plugin_dir.clone());
        let runtime = catalog.activate_runtime(&storage).await.unwrap();
        let public = serde_json::to_value(authentication.public_host_plugins(&runtime)).unwrap();
        assert_eq!(public.as_array().unwrap().len(), 1);
        assert_eq!(public[0]["id"], "google");
        assert_eq!(public[0]["plugin_version"], google.plugin_version);
        assert_eq!(public[0]["artifact_digest"], google.artifact_digest);
        assert_eq!(
            runtime.default_hosts().len(),
            1,
            "an unconfigured Authentication host was enabled"
        );
        assert!(
            runtime
                .exact_host("google", "2.0.0", &newer.artifact_digest)
                .is_some()
        );
        assert!(
            crate::auth_plugins::ProductAuthentication::disabled()
                .public_host_plugins(&runtime)
                .is_empty()
        );

        let mut conflicting = HostActivationPolicy::default();
        conflicting.pin(release_pin(&newer), true).unwrap();
        assert!(
            authentication
                .configure_host_policy(&mut conflicting)
                .unwrap_err()
                .to_string()
                .contains("conflicting")
        );
        let legacy = crate::auth_plugins::ProductAuthentication::load(
            None,
            &catalog,
            Some(Arc::clone(authentication.provider("google").unwrap())),
        )
        .unwrap();
        assert!(
            legacy
                .configure_host_policy(&mut policy)
                .unwrap_err()
                .to_string()
                .contains("migrate legacy OIDC")
        );

        // Legacy signed OIDC contracts may be hostless in bootstrap, but cannot
        // use a newer host or pass the Catalog-only readiness gate.
        let mut hostless = catalog.host_releases();
        hostless
            .iter_mut()
            .find(|release| {
                release.entry.artifact_digest.as_deref() == Some(google.artifact_digest.as_str())
            })
            .unwrap()
            .host_bundle = None;
        let mut bootstrap_policy = HostActivationPolicy::default();
        authentication
            .configure_host_policy(&mut bootstrap_policy)
            .unwrap();
        bootstrap_policy.select(&mut hostless).unwrap();
        assert_eq!(
            hostless
                .iter()
                .filter(|release| release.entry.plugin_id == "google" && release.default_for_id)
                .count(),
            1
        );
        assert!(policy.select(&mut hostless).is_err());
        unlock_tree(&root);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn failed_catalog_only_migration_does_not_commit_cutover() {
        let root = auth_test_root("failed-cutover");
        // Passkey SQL is now rejected in preflight. Keep the actual runtime
        // migration-failure gate on an ordinary (non-security) storage host.
        let release = publish_storage_fixture(&root, "migration-failure", "2.0.0", 2);
        let mut catalog = PluginCatalog::open(&root, Some(root.join("external"))).unwrap();
        let mut policy = HostActivationPolicy::default();
        policy.source = HostSourcePolicy::CatalogOnly;
        policy.pin(release_pin(&release), true).unwrap();
        catalog.configure_hosts(policy).unwrap();
        assert!(catalog.strict_host_activation());
        let storage =
            crate::plugin_storage::PluginStorage::sqlite_files(catalog.plugin_dir.clone());
        let initial = crate::plugin_host::PluginHostSpec::from_json(
            storage_host_fixture(1).to_string().as_bytes(),
        )
        .unwrap();
        let namespace = storage
            .migrate_plugin("migration-failure", initial.storage.as_ref().unwrap())
            .await
            .unwrap();
        // A fixture-only conflicting table makes the valid 0002 fail in SQL.
        namespace
            .execute("CREATE TABLE upgrade_only (id TEXT PRIMARY KEY)")
            .await
            .unwrap();
        let error = catalog.activate_runtime(&storage).await.err().unwrap();
        assert!(format!("{error:#}").contains("already exists"));
        assert!(catalog.runtime().is_none());
        assert!(!catalog.plugin_dir.catalog_only_required().unwrap());
        unlock_tree(&root);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn local_auth_releases_replace_bootstrap_without_replacing_credentials() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-local-auth-catalog-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let catalog_root = root.join("external");
        let mut catalog = PluginCatalog::open(&root, Some(catalog_root.clone())).unwrap();
        let authentication = crate::auth_plugins::ProductAuthentication::test_default(None);
        let mut bootstrap_policy = HostActivationPolicy::default();
        authentication
            .configure_host_policy(&mut bootstrap_policy)
            .unwrap();
        catalog.configure_hosts(bootstrap_policy.clone()).unwrap();
        let storage = crate::plugin_storage::PluginStorage::sqlite_files(
            crate::plugin_dir::PluginDir::open(&root).unwrap(),
        );
        let before = catalog.activate_runtime(&storage).await.unwrap();
        let public = serde_json::to_value(authentication.public_host_plugins(&before)).unwrap();
        let mut ids = public
            .as_array()
            .unwrap()
            .iter()
            .map(|host| host["id"].as_str().unwrap())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(
            ids,
            ["passkey", "password"],
            "unconfigured bootstrap login hosts leaked into auth status"
        );
        assert!(
            before
                .default_host("password")
                .unwrap()
                .artifact_digest
                .is_none()
        );
        let namespace = before.namespace_for_capability("webauthn").unwrap();
        namespace.execute(
            "INSERT INTO user_passkeys (id, user_id, credential_id, nickname, passkey_json, created_at_ms) \
             VALUES ('fixture-passkey', 'fixture-user', 'fixture-credential', 'Fixture', '{}', 1)"
        )
        .await
        .unwrap();

        let password = publish_local_auth_fixture(&root, "password", false);
        let passkey = publish_local_auth_fixture(&root, "passkey", false);
        let error = catalog.refresh_with_runtime(&storage).await.unwrap_err();
        assert!(error.to_string().contains("exact host selection"));
        assert!(Arc::ptr_eq(&before, &catalog.runtime().unwrap()));
        bootstrap_policy.pin(release_pin(&password), true).unwrap();
        bootstrap_policy.pin(release_pin(&passkey), true).unwrap();
        let mut catalog = PluginCatalog::open(&root, Some(catalog_root.clone())).unwrap();
        catalog.configure_hosts(bootstrap_policy).unwrap();
        let activated = catalog.activate_runtime(&storage).await.unwrap();
        for release in [&password, &passkey] {
            let host = activated
                .exact_host(
                    &release.plugin_id,
                    &release.plugin_version,
                    &release.artifact_digest,
                )
                .unwrap();
            assert_eq!(
                host.artifact_digest.as_deref(),
                Some(release.artifact_digest.as_str())
            );
            assert_eq!(
                activated
                    .default_host(&release.plugin_id)
                    .unwrap()
                    .generation,
                host.generation
            );
            let contract = catalog
                .resolve_authentication_provider(
                    &release.plugin_id,
                    &release.plugin_version,
                    &release.artifact_digest,
                )
                .unwrap();
            assert_eq!(contract.schema_version, 2);
        }
        assert!(
            catalog.released_plugins().is_empty(),
            "Controller protocols cannot install on a Machine"
        );
        assert_eq!(
            activated
                .namespace_for_capability("webauthn")
                .unwrap()
                .fetch_i64("SELECT COUNT(*) FROM user_passkeys WHERE user_id = 'fixture-user'")
                .await
                .unwrap(),
            1
        );

        // A trusted signature cannot authorize another protocol's renderer.
        publish_local_auth_fixture(&root, "password", true);
        let error = catalog.refresh_with_runtime(&storage).await.unwrap_err();
        assert!(format!("{error:#}").contains("renderer does not match its protocol"));
        assert!(Arc::ptr_eq(&activated, &catalog.runtime().unwrap()));

        for id in ["password", "passkey"] {
            for extension in ["cowboy-plugin", "release.json", "hostbundle.json"] {
                fs::remove_file(catalog_root.join(format!("{id}.{extension}"))).unwrap();
            }
        }
        let restarted = PluginCatalog::open(&root, Some(catalog_root)).unwrap();
        let empty = restarted.activate_runtime(&storage).await.unwrap();
        assert!(empty.default_host("password").is_none());
        assert!(empty.default_host("passkey").is_none());
        unlock_tree(&root);
        fs::remove_dir_all(root).unwrap();
    }
}
