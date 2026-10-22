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

mod lease;
mod retention;
mod watch;

use lease::ReleaseObservation;
pub use lease::VerifiedPluginRelease;
pub use retention::CatalogKeepSet;
pub use watch::spawn as spawn_catalog_watcher;

#[derive(Debug, Clone, Serialize)]
pub struct PluginCatalogEntry {
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
pub enum PluginReleaseState {
    Unbound,
    Ready,
}

#[derive(Clone)]
pub struct CatalogArtifact {
    entry: PluginCatalogEntry,
    desired: DesiredPlugin,
    package: PluginPackage,
    host_bundle: Option<PluginHostBundle>,
    incarnation: Arc<()>,
}

#[derive(Clone)]
pub struct CatalogSnapshot {
    pub external: BTreeMap<(String, String, String), CatalogArtifact>,
    pub runtime: Option<Arc<crate::plugin_runtime::PluginRuntime>>,
}

/// One exact released host candidate. Every release is retained so installed
/// Machines and existing sessions can resolve the declaration matching their
/// immutable Plugin identity; `default_for_id` is only the presentation and
/// Controller-side behavior choice for callers with no exact identity.
#[derive(Debug, Clone)]
pub struct CatalogHostRelease {
    pub entry: PluginCatalogEntry,
    pub host_bundle: Option<PluginHostBundle>,
    pub default_for_id: bool,
}

pub struct PluginCatalogInventory {
    pub entries: Vec<PluginCatalogEntry>,
    pub runtime: Option<Arc<crate::plugin_runtime::PluginRuntime>>,
}

/// A verified Catalog snapshot projection, not a serialized grant. Only the
/// signature-checked exact-release lookup below can construct this value.
/// It proves a contract, not enrollment, policy or a live installation lease.
pub struct VerifiedTelemetryRelease {
    contract: TelemetryBackendContract,
    generation_digest: String,
    contract_fingerprint: String,
    observation: ReleaseObservation,
}

impl VerifiedTelemetryRelease {
    pub fn current(&self, catalog: &PluginCatalog) -> bool {
        self.observation.current(catalog)
    }

    pub fn matches_installation(
        &self,
        installation: &crate::machine_protocol::telemetry_binding::BindingInstallation,
    ) -> bool {
        installation.plugin_id == self.contract.id
            && installation.plugin_version == self.contract.version
            && String::from(installation.generation_digest.clone()) == self.generation_digest
            && String::from(installation.contract_fingerprint.clone()) == self.contract_fingerprint
    }

    pub fn matches_inventory(&self, plugin: &crate::machine_protocol::PluginInventory) -> bool {
        plugin.plugin_kind == PluginKind::TelemetryBackend
            && plugin.plugin_id == self.contract.id
            && plugin.plugin_version == self.contract.version
            && plugin.generation_digest == self.generation_digest
            && plugin.contract_fingerprint == self.contract_fingerprint
            && plugin.state == crate::machine_protocol::PluginInstallationState::Active
            && plugin.auth_generation.is_none()
    }

    pub fn operation_for(
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

pub struct PluginCatalog {
    pub embedded: BTreeMap<(String, String), PluginCatalogEntry>,
    pub state: RwLock<Arc<CatalogSnapshot>>,
    pub refresh_lock: tokio::sync::Mutex<()>,
    pub roots: Vec<PathBuf>,
    pub plugin_dir: crate::plugin_dir::PluginDir,
    pub host_policy: HostActivationPolicy,
    pub disabled: std::collections::BTreeSet<String>,
}

impl PluginCatalog {
    #[cfg(any(test, feature = "test-support"))]
    pub fn open(data_dir: &Path, root: Option<PathBuf>) -> Result<Self> {
        let catalog = Self::inspect(data_dir, root)?;
        catalog.initialize()?;
        Ok(catalog)
    }

    /// Called only after startup preflight, never by inspection or refresh.
    pub fn initialize(&self) -> Result<()> {
        self.plugin_dir.initialize()?;
        for root in &self.roots {
            fs::create_dir_all(root)
                .with_context(|| format!("creating Plugin Catalog {}", root.display()))?;
        }
        Ok(())
    }

    /// Read the same trusted Catalog as startup without creating any state.
    /// Missing roots are empty inventories, not an instruction to initialize.
    pub fn inspect(data_dir: &Path, root: Option<PathBuf>) -> Result<Self> {
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
            disabled: std::collections::BTreeSet::new(),
        })
    }

    pub fn load_external(&self) -> Result<BTreeMap<(String, String, String), CatalogArtifact>> {
        let mut next = BTreeMap::new();
        for (index, root) in self.roots.iter().enumerate() {
            load_catalog_root(root, &mut next, index > 0)?;
        }
        next.retain(|(id, _, _), _| self.plugin_enabled(id));
        Ok(next)
    }

    pub fn plugin_enabled(&self, id: &str) -> bool {
        !self.disabled.contains(id)
    }

    pub fn configure_disabled(&mut self, ids: Vec<String>) -> Result<()> {
        ensure!(
            self.state.read().runtime.is_none(),
            "Plugin disable policy is immutable after startup"
        );
        self.disabled = ids.into_iter().collect();
        self.embedded
            .retain(|(id, _), _| !self.disabled.contains(id));
        let external = self.load_external()?;
        *self.state.write() = Arc::new(CatalogSnapshot {
            external,
            runtime: None,
        });
        Ok(())
    }

    /// Set the immutable startup policy before the Catalog is shared. This
    /// preflight performs no generation staging, marker writes, or migrations.
    pub fn configure_hosts(&mut self, policy: HostActivationPolicy) -> Result<()> {
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

    pub fn strict_host_activation(&self) -> bool {
        self.host_policy.strict_activation()
    }

    pub fn host_preflight_report(&self) -> Result<HostPreflightReport> {
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
    pub async fn activate_runtime(
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
        let mut runtime = crate::plugin_runtime::PluginRuntime::activate_selected(
            storage,
            &releases,
            self.host_policy.source == HostSourcePolicy::Bootstrap,
            &self.disabled,
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
    pub async fn refresh_with_runtime(
        &self,
        storage: &crate::plugin_storage::PluginStorage,
    ) -> Result<usize> {
        let _refresh = self.refresh_lock.lock().await;
        let current = self.state.read().clone();
        let mut external = self.load_external()?;
        let (releases, _) = self.select_host_releases(&external, &self.host_policy)?;
        let candidate_ids = releases
            .iter()
            .map(|release| release.entry.plugin_id.clone())
            .collect();
        let mut runtime = crate::plugin_runtime::PluginRuntime::activate_selected(
            storage,
            &releases,
            self.host_policy.source == HostSourcePolicy::Bootstrap,
            &self.disabled,
        )
        .await?;
        if let Some(previous) = current.runtime.as_deref() {
            runtime.retain_previous_generations(previous, &candidate_ids);
        }
        let runtime = Arc::new(runtime);
        let count = external.len();
        self.record_host_authority()?;
        let mut accepted = self.state.write();
        // Only accepted, continuous *complete envelopes* retain a lease. All
        // fallible validation/staging above must finish before this publication.
        // Never revive a removed release from retained runtime generations.
        for (key, next) in &mut external {
            if let Some(previous) = accepted.external.get(key)
                && previous.desired == next.desired
            {
                next.incarnation = Arc::clone(&previous.incarnation);
            }
        }
        *accepted = Arc::new(CatalogSnapshot {
            external,
            runtime: Some(runtime),
        });
        Ok(count)
    }

    /// Install a fail-closed empty runtime after a startup activation failure.
    pub fn install_empty_runtime(&self) -> Arc<crate::plugin_runtime::PluginRuntime> {
        let runtime = Arc::new(crate::plugin_runtime::PluginRuntime::empty());
        let mut current = self.state.write();
        *current = Arc::new(CatalogSnapshot {
            external: current.external.clone(),
            runtime: Some(Arc::clone(&runtime)),
        });
        runtime
    }

    #[must_use]
    pub fn runtime(&self) -> Option<Arc<crate::plugin_runtime::PluginRuntime>> {
        self.state.read().runtime.clone()
    }

    pub fn entries(&self) -> Vec<PluginCatalogEntry> {
        let snapshot = self.state.read().clone();
        self.entries_for(&snapshot.external)
    }

    #[must_use]
    pub fn inventory(&self) -> PluginCatalogInventory {
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

    pub fn released_plugins(&self) -> Vec<DesiredPlugin> {
        self.state
            .read()
            .external
            .values()
            .filter(|artifact| artifact.entry.plugin_kind != PluginKind::AuthenticationProvider)
            .map(|artifact| artifact.desired.clone())
            .collect()
    }

    pub fn resolve_authentication_provider(
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
    /// previous snapshot under the existing Catalog policy. The result retains
    /// its original observation; callers recheck it at later effect boundaries.
    pub fn resolve_telemetry_backend(
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
            observation: ReleaseObservation::capture(artifact),
        })
    }

    pub fn published_artifact_path(&self, digest: &str, name: &str) -> Option<PathBuf> {
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

    pub fn catalog_root(&self) -> PathBuf {
        self.roots
            .first()
            .cloned()
            .expect("plugin catalog has a primary root")
    }

    #[must_use]
    #[cfg(any(test, feature = "test-support"))]
    pub fn host_releases(&self) -> Vec<CatalogHostRelease> {
        catalog_host_releases(&self.state.read().external)
    }

    pub fn resolve(
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
        if requires_newer_provider_sdk(&bytes)? {
            // The package is opaque to this reader. It contributes neither a
            // Catalog identity nor host/install authority. An exact active
            // host pin still fails closed if only an unsupported release exists.
            ensure!(
                PluginPackage::artifact_digest(&bytes) == release.package_digest,
                "unsupported Plugin package digest mismatch"
            );
            tracing::warn!("newer Provider SDK package skipped by Catalog reader");
            continue;
        }
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

pub fn requires_newer_provider_sdk(bytes: &[u8]) -> Result<bool> {
    // Inspect only the existing typed format discriminator, before decoding a
    // future Provider vocabulary. Derived structs reject duplicate identity
    // fields; unknown future fields confer no authority and remain opaque.
    #[derive(serde::Deserialize)]
    struct PackageHeader {
        payload: PayloadHeader,
    }
    #[derive(serde::Deserialize)]
    struct PayloadHeader {
        kind: PluginKind,
        contract: ContractHeader,
    }
    #[derive(serde::Deserialize)]
    struct ContractHeader {
        manifest: Option<ManifestHeader>,
    }
    #[derive(serde::Deserialize)]
    struct ManifestHeader {
        sdk_version: String,
    }
    let header: PackageHeader =
        serde_json::from_slice(bytes).context("decoding Plugin payload SDK header")?;
    if header.payload.kind != PluginKind::AgentProvider {
        return Ok(false);
    }
    let version = header
        .payload
        .contract
        .manifest
        .context("Agent Plugin is missing its Provider manifest")?
        .sdk_version;
    ensure!(version.len() <= 128, "Provider SDK version is too long");
    let requested = semver::Version::parse(&version).context("invalid Provider SDK version")?;
    let supported = semver::Version::parse(cowboy_provider_sdk::PROVIDER_SDK_VERSION)?;
    Ok(requested > supported)
}

pub fn read_supported_release(path: &Path) -> Result<Option<PluginRelease>> {
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
        incarnation: Arc::new(()),
    })
}

pub fn desired_plugin_compatibility(
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

pub fn latest_unique_release_identity<'a>(
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
