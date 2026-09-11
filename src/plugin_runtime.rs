//! Activate release-selected host snapshots into the server plugin directory.
//!
//! Exact Catalog releases supply host declarations. The opt-in `catalog_only`
//! policy retires all compile-time bootstrap hosts. During migration, source
//! trees discovered at build time may bootstrap IDs that have never acquired
//! Catalog authority; they cannot replace a missing signed release.

#![warn(clippy::pedantic)]

use std::collections::BTreeMap;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use anyhow::{Context as _, Result, ensure};
use sha2::{Digest as _, Sha256};

use crate::plugin_host::{
    PluginHostSpec, PluginLoginFields, PluginSlotId, PluginUiSpec, PluginUsageSpec,
    PluginVisualSpec, UsageActivityAgent, UsageCacheProtection, UsageErrorKind, UsageLimitLabel,
    UsageLimitParserKind, UsageWidgetKind, UsageWidgetShape,
};
use crate::plugin_storage::{PluginNamespace, PluginStorage};

struct BundledHostFile {
    path: &'static str,
    content: &'static str,
}

struct BundledHost {
    id: &'static str,
    files: &'static [BundledHostFile],
}

const BUNDLED_HOSTS: &[BundledHost] = include!(concat!(env!("OUT_DIR"), "/bundled_hosts.rs"));

#[derive(Debug, Clone)]
pub struct ActivatedHostPlugin {
    pub id: String,
    /// Exact outer Plugin identity for a trusted release. Source-bundled
    /// bootstrap hosts deliberately have neither field.
    pub plugin_version: Option<String>,
    pub artifact_digest: Option<String>,
    /// Content address used by Web to reject mutable host declarations. For a
    /// trusted release this is the outer Plugin artifact digest without its
    /// `sha256:` prefix; bootstrap hosts use their host-tree digest.
    pub generation: String,
    pub slots: Vec<&'static str>,
    pub ui: Option<PluginUiSpec>,
    pub usage: Option<PluginUsageSpec>,
    pub label: Option<String>,
    pub adapter_slot: Option<String>,
    pub login_fields: Option<PluginLoginFields>,
    pub rpc_argv: Vec<String>,
    pub visual: Option<PluginVisualSpec>,
    pub native_capabilities: Vec<String>,
}

/// Least-privilege projection served to Cowboy Web.
///
/// Keep execution policy, storage namespace state, and expanded generation
/// paths inside the Controller runtime; making the activated host type
/// non-serializable prevents a new private field from silently becoming part of
/// the public API.
#[derive(Debug, serde::Serialize)]
pub(crate) struct PublicHostPlugin {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    plugin_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifact_digest: Option<String>,
    generation: String,
    default_for_id: bool,
    slots: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ui: Option<PluginUiSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<PublicPluginUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    adapter_slot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    login_fields: Option<PluginLoginFields>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visual: Option<PluginVisualSpec>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    native_capabilities: Vec<String>,
}

/// Usage fields consumed by the closed Cowboy renderers. Collector commands
/// and Controller-side execution semantics deliberately do not cross the API.
#[derive(Debug, serde::Serialize)]
struct PublicPluginUsage {
    account: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    product: Option<String>,
    #[serde(skip_serializing_if = "is_default_value")]
    parser: UsageLimitParserKind,
    #[serde(skip_serializing_if = "is_default_value")]
    error: UsageErrorKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_auth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_fetch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    order: Option<u16>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    top_bar_windows: Vec<u32>,
    #[serde(skip_serializing_if = "is_default_value")]
    widget: UsageWidgetKind,
    #[serde(skip_serializing_if = "is_default_value")]
    widget_shape: UsageWidgetShape,
    #[serde(skip_serializing_if = "Option::is_none")]
    widget_window: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    empty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    available_status: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    omit_empty_limits: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit_id_prefix: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    limit_labels: Vec<UsageLimitLabel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    widget_balance_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    widget_spend_label: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    activity_agents: Vec<UsageActivityAgent>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    activity_models: Vec<UsageActivityAgent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_protection: Option<UsageCacheProtection>,
}

fn is_default_value<T: Default + PartialEq>(value: &T) -> bool {
    value == &T::default()
}

impl From<&PluginUsageSpec> for PublicPluginUsage {
    fn from(usage: &PluginUsageSpec) -> Self {
        Self {
            account: usage.account.clone(),
            reset: usage.reset.clone(),
            product: usage.product.clone(),
            parser: usage.parser.clone(),
            error: usage.error.clone(),
            error_auth: usage.error_auth.clone(),
            error_config: usage.error_config.clone(),
            error_fetch: usage.error_fetch.clone(),
            order: usage.order,
            top_bar_windows: usage.top_bar_windows.clone(),
            widget: usage.widget.clone(),
            widget_shape: usage.widget_shape,
            widget_window: usage.widget_window,
            empty: usage.empty.clone(),
            available_status: usage.available_status.clone(),
            omit_empty_limits: usage.omit_empty_limits,
            limit_id_prefix: usage.limit_id_prefix.clone(),
            limit_labels: usage.limit_labels.clone(),
            widget_balance_label: usage.widget_balance_label.clone(),
            widget_spend_label: usage.widget_spend_label.clone(),
            activity_agents: usage.activity_agents.clone(),
            activity_models: usage.activity_models.clone(),
            cache_protection: usage.cache_protection.clone(),
        }
    }
}

impl ActivatedHostPlugin {
    #[must_use]
    pub fn public_auth_surface(&self) -> bool {
        self.slots.iter().any(|slot| {
            *slot == PluginSlotId::LoginMethod.as_str()
                || *slot == PluginSlotId::AccountPanel.as_str()
        })
    }

    #[must_use]
    pub(crate) fn public_descriptor(&self, default_for_id: bool) -> PublicHostPlugin {
        PublicHostPlugin {
            id: self.id.clone(),
            plugin_version: self.plugin_version.clone(),
            artifact_digest: self.artifact_digest.clone(),
            generation: self.generation.clone(),
            default_for_id,
            slots: self.slots.clone(),
            ui: self.ui.clone(),
            usage: self.usage.as_ref().map(PublicPluginUsage::from),
            label: self.label.clone(),
            adapter_slot: self.adapter_slot.clone(),
            login_fields: self.login_fields.clone(),
            visual: self.visual.clone(),
            native_capabilities: self.native_capabilities.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PluginHostIdentity {
    pub plugin_id: String,
    pub plugin_version: String,
    pub artifact_digest: String,
}

pub struct PluginRuntime {
    exact_hosts: BTreeMap<PluginHostIdentity, ActivatedHostPlugin>,
    default_hosts: BTreeMap<String, ActivatedHostPlugin>,
    namespaces: BTreeMap<String, PluginNamespace>,
}

impl PluginRuntime {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            exact_hosts: BTreeMap::new(),
            default_hosts: BTreeMap::new(),
            namespaces: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn usage_bindings(&self) -> Vec<PluginUsageSpec> {
        self.default_hosts
            .values()
            .filter_map(|host| host.usage.clone())
            .collect()
    }

    /// Every released exact host capable of serving one usage account. This
    /// is intentionally separate from presentation defaults: a connected
    /// Machine may still be draining an older immutable release.
    #[must_use]
    pub(crate) fn exact_usage_hosts(&self, account: &str) -> Vec<ActivatedHostPlugin> {
        self.exact_hosts
            .values()
            .filter(|host| {
                host.plugin_version.is_some()
                    && host.artifact_digest.is_some()
                    && host
                        .usage
                        .as_ref()
                        .is_some_and(|usage| usage.account == account)
            })
            .cloned()
            .collect()
    }

    #[must_use]
    pub(crate) fn default_hosts(&self) -> Vec<&ActivatedHostPlugin> {
        self.default_hosts.values().collect()
    }

    #[must_use]
    pub(crate) fn default_host(&self, plugin_id: &str) -> Option<&ActivatedHostPlugin> {
        self.default_hosts.get(plugin_id)
    }

    /// Resolve only an exact trusted release. Supplying half an identity or an
    /// unknown generation never falls through to the mutable default.
    #[must_use]
    pub(crate) fn exact_host(
        &self,
        plugin_id: &str,
        plugin_version: &str,
        artifact_digest: &str,
    ) -> Option<&ActivatedHostPlugin> {
        self.exact_hosts.get(&PluginHostIdentity {
            plugin_id: plugin_id.to_owned(),
            plugin_version: plugin_version.to_owned(),
            artifact_digest: artifact_digest.to_owned(),
        })
    }

    #[must_use]
    pub(crate) fn public_descriptors(&self) -> Vec<PublicHostPlugin> {
        let mut descriptors = self
            .exact_hosts
            .iter()
            .map(|(identity, host)| {
                host.public_descriptor(self.default_hosts.get(&identity.plugin_id).is_some_and(
                    |default| {
                        default.plugin_version.as_deref() == Some(identity.plugin_version.as_str())
                            && default.artifact_digest.as_deref()
                                == Some(identity.artifact_digest.as_str())
                    },
                ))
            })
            .collect::<Vec<_>>();
        descriptors.extend(
            self.default_hosts
                .values()
                .filter(|host| host.artifact_digest.is_none())
                .map(|host| host.public_descriptor(true)),
        );
        descriptors
    }

    #[cfg(test)]
    pub(crate) fn from_default_hosts(hosts: Vec<ActivatedHostPlugin>) -> Self {
        let mut runtime = Self::empty();
        for host in hosts {
            if let (Some(plugin_version), Some(artifact_digest)) =
                (&host.plugin_version, &host.artifact_digest)
            {
                runtime.exact_hosts.insert(
                    PluginHostIdentity {
                        plugin_id: host.id.clone(),
                        plugin_version: plugin_version.clone(),
                        artifact_digest: artifact_digest.clone(),
                    },
                    host.clone(),
                );
            }
            runtime.default_hosts.insert(host.id.clone(), host);
        }
        runtime
    }

    /// Retain exact immutable generations from the prior snapshot. When an ID
    /// disappears entirely from a Catalog scan, also keep its prior default
    /// and namespace so transient publication gaps cannot break active leases.
    /// A present candidate ID always owns the new default decision, including
    /// an intentional fail-closed result for an ambiguous or hostless latest
    /// release.
    pub(crate) fn retain_previous_generations(
        &mut self,
        previous: &Self,
        candidate_plugin_ids: &std::collections::BTreeSet<String>,
    ) {
        for (identity, host) in &previous.exact_hosts {
            self.exact_hosts
                .entry(identity.clone())
                .or_insert_with(|| host.clone());
        }
        for (plugin_id, host) in &previous.default_hosts {
            if !candidate_plugin_ids.contains(plugin_id) {
                self.default_hosts
                    .entry(plugin_id.clone())
                    .or_insert_with(|| host.clone());
                if let Some(namespace) = previous.namespaces.get(plugin_id) {
                    self.namespaces
                        .entry(plugin_id.clone())
                        .or_insert_with(|| namespace.clone());
                }
            }
        }
    }

    /// Resolve one plugin storage namespace by a declared host capability.
    /// Ambiguous capability claims fail closed.
    #[must_use]
    pub fn namespace_for_capability(&self, capability: &str) -> Option<&PluginNamespace> {
        let mut matches = self.default_hosts.values().filter(|host| {
            host.native_capabilities
                .iter()
                .any(|candidate| candidate == capability)
                && self.namespaces.contains_key(&host.id)
        });
        let host = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        self.namespaces.get(&host.id)
    }

    /// Stage bundled host plugins plus every exact released generation and
    /// migrate storage required by the selected default hosts.
    ///
    /// # Errors
    /// Returns when any generation, host spec, runtime reference, or required
    /// storage migration is invalid. Callers publish the resulting immutable
    /// snapshot only after this method succeeds completely.
    #[cfg(test)]
    pub async fn activate(
        storage: &PluginStorage,
        catalog: Option<&crate::plugin_catalog::PluginCatalog>,
    ) -> Result<Self> {
        let releases = catalog
            .map(crate::plugin_catalog::PluginCatalog::host_releases)
            .unwrap_or_default();
        Self::activate_releases(storage, &releases, true).await
    }

    #[allow(clippy::too_many_lines)] // One transaction stages every authority and generation before migration.
    pub(crate) async fn activate_releases(
        storage: &PluginStorage,
        releases: &[crate::plugin_catalog::CatalogHostRelease],
        allow_bootstrap: bool,
    ) -> Result<Self> {
        let mut runtime = Self::empty();
        let released_plugin_ids = releases
            .iter()
            .map(|release| release.entry.plugin_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        for plugin_id in &released_plugin_ids {
            storage.plugin_dir().record_catalog_authority(plugin_id)?;
        }
        let mut release_authority_ids = released_plugin_ids;
        for host in BUNDLED_HOSTS {
            if storage.plugin_dir().has_catalog_authority(host.id)? {
                release_authority_ids.insert(host.id.to_owned());
            }
        }
        let mut default_specs = BTreeMap::<String, PluginHostSpec>::new();
        for host in BUNDLED_HOSTS {
            if !allow_bootstrap || !bundled_fallback_allowed(&release_authority_ids, host.id) {
                continue;
            }
            let files = bundled_files(host);
            let source = bundled_file(host, "host.json")
                .with_context(|| format!("bundled {} host.json", host.id))?;
            let spec = PluginHostSpec::from_json(source.as_bytes())
                .with_context(|| format!("bundled {} host spec", host.id))?;
            let revision = bundled_host_revision(host);
            let generation = storage
                .plugin_dir()
                .install_host_files(host.id, "0.0.0", &revision, &files, false)?;
            let staged = PluginHostSpec::load_optional(&generation)?
                .with_context(|| format!("bundled {} host spec missing after stage", host.id))?;
            ensure!(
                staged == spec,
                "bundled {} host spec does not match staged files",
                host.id
            );
            let activated =
                build_activated_host(host.id, &spec, &generation, &revision, None, None, &files)?;
            ensure!(
                runtime
                    .default_hosts
                    .insert(host.id.to_owned(), activated)
                    .is_none(),
                "duplicate bundled default host {}",
                host.id
            );
            default_specs.insert(host.id.to_owned(), spec);
        }
        for release in releases {
            let Some(bundle) = release.host_bundle.as_ref() else {
                continue;
            };
            let artifact_digest = release
                .entry
                .artifact_digest
                .as_deref()
                .context("released host has no outer artifact digest")?;
            let generation_id = artifact_digest
                .strip_prefix("sha256:")
                .unwrap_or(artifact_digest);
            let generation = storage.plugin_dir().install_host_files(
                &release.entry.plugin_id,
                &release.entry.plugin_version,
                artifact_digest,
                &bundle.files,
                false,
            )?;
            let host_json = bundle
                .files
                .get("host.json")
                .context("released host bundle has no host.json")?;
            let spec = PluginHostSpec::from_json(host_json.as_bytes()).with_context(|| {
                format!(
                    "released {} {} host spec",
                    release.entry.plugin_id, release.entry.plugin_version
                )
            })?;
            let staged = PluginHostSpec::load_optional(&generation)?.with_context(|| {
                format!(
                    "released {} {} host spec missing after stage",
                    release.entry.plugin_id, release.entry.plugin_version
                )
            })?;
            ensure!(
                staged == spec,
                "released {} {} host spec does not match staged files",
                release.entry.plugin_id,
                release.entry.plugin_version
            );
            let activated = build_activated_host(
                &release.entry.plugin_id,
                &spec,
                &generation,
                generation_id,
                Some(&release.entry.plugin_version),
                Some(artifact_digest),
                &bundle.files,
            )?;
            let identity = PluginHostIdentity {
                plugin_id: release.entry.plugin_id.clone(),
                plugin_version: release.entry.plugin_version.clone(),
                artifact_digest: artifact_digest.to_owned(),
            };
            ensure!(
                runtime
                    .exact_hosts
                    .insert(identity, activated.clone())
                    .is_none(),
                "duplicate exact Plugin host generation"
            );
            if release.default_for_id {
                ensure!(
                    runtime
                        .default_hosts
                        .insert(release.entry.plugin_id.clone(), activated)
                        .is_none(),
                    "duplicate default Plugin host generation"
                );
                default_specs.insert(release.entry.plugin_id.clone(), spec);
            }
        }
        validate_default_host_claims(&runtime.default_hosts)?;
        // Check all selected schemas before any one host can migrate storage.
        for spec in default_specs.values() {
            crate::core_passkeys::validate_legacy_host(spec)?;
        }
        for (plugin_id, spec) in default_specs {
            if let Some(storage_spec) = &spec.storage {
                let namespace = storage
                    .migrate_plugin(&plugin_id, storage_spec)
                    .await
                    .with_context(|| format!("plugin {plugin_id} storage migration failed"))?;
                tracing::debug!(
                    plugin_id = namespace.plugin_id(),
                    schema = namespace.schema_name(),
                    "plugin storage ready"
                );
                runtime.namespaces.insert(plugin_id, namespace);
            }
        }
        Ok(runtime)
    }
}

fn validate_default_host_claims(hosts: &BTreeMap<String, ActivatedHostPlugin>) -> Result<()> {
    let mut accounts = BTreeMap::<&str, &str>::new();
    let mut resets = BTreeMap::<&str, &str>::new();
    for host in hosts.values() {
        let Some(usage) = host.usage.as_ref() else {
            continue;
        };
        ensure!(
            accounts
                .insert(usage.account.as_str(), host.id.as_str())
                .is_none(),
            "multiple default Plugin hosts claim usage account {}",
            usage.account
        );
        if let Some(reset) = usage.reset.as_deref() {
            ensure!(
                resets.insert(reset, host.id.as_str()).is_none(),
                "multiple default Plugin hosts claim usage reset {reset}"
            );
        }
    }
    Ok(())
}

fn bundled_fallback_allowed(
    released_plugin_ids: &std::collections::BTreeSet<String>,
    plugin_id: &str,
) -> bool {
    !released_plugin_ids.contains(plugin_id)
}

fn build_activated_host(
    plugin_id: &str,
    spec: &PluginHostSpec,
    generation: &Path,
    revision: &str,
    plugin_version: Option<&str>,
    artifact_digest: Option<&str>,
    files: &BTreeMap<String, String>,
) -> Result<ActivatedHostPlugin> {
    validate_host_files(spec, files)?;
    Ok(ActivatedHostPlugin {
        id: plugin_id.to_owned(),
        plugin_version: plugin_version.map(ToOwned::to_owned),
        artifact_digest: artifact_digest.map(ToOwned::to_owned),
        generation: revision.to_owned(),
        slots: spec
            .slots
            .iter()
            .map(|slot| PluginSlotId::as_str(*slot))
            .collect(),
        ui: spec.ui.clone(),
        usage: activated_usage(spec.usage.as_ref(), generation),
        label: spec.label.clone(),
        adapter_slot: spec.adapter_slot.clone(),
        login_fields: spec.login_fields.clone(),
        rpc_argv: expand_plugin_argv(&spec.rpc_argv, generation),
        visual: spec.visual.clone(),
        native_capabilities: spec.native_capabilities.clone(),
    })
}

fn validate_host_files(spec: &PluginHostSpec, files: &BTreeMap<String, String>) -> Result<()> {
    spec.validate_runtime_files(files)
}

fn activated_usage(usage: Option<&PluginUsageSpec>, generation: &Path) -> Option<PluginUsageSpec> {
    let mut usage = usage?.clone();
    usage.collector_argv = expand_plugin_argv(&usage.collector_argv, generation);
    usage.reset_argv = expand_plugin_argv(&usage.reset_argv, generation);
    Some(usage)
}

fn expand_plugin_argv(argv: &[String], generation: &Path) -> Vec<String> {
    let generation = generation.to_string_lossy();
    argv.iter()
        .map(|argument| argument.replace("${PLUGIN_DIR}", &generation))
        .collect()
}

fn bundled_file<'a>(host: &'a BundledHost, path: &str) -> Result<&'a str> {
    host.files
        .iter()
        .find(|file| file.path == path)
        .map(|file| file.content)
        .with_context(|| format!("bundled {} is missing {path}", host.id))
}

fn bundled_files(host: &BundledHost) -> BTreeMap<String, String> {
    host.files
        .iter()
        .map(|file| (file.path.to_owned(), file.content.to_owned()))
        .collect()
}

#[cfg(test)]
fn stage_bundled_generation(
    dir: &crate::plugin_dir::PluginDir,
    host: &BundledHost,
    revision: &str,
) -> Result<PathBuf> {
    dir.install_host_files(host.id, "0.0.0", revision, &bundled_files(host), false)
}

fn bundled_host_revision(host: &BundledHost) -> String {
    let mut files = host.files.iter().collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(right.path));
    let mut digest = Sha256::new();
    digest.update(host.id.len().to_string());
    digest.update(b":");
    digest.update(host.id);
    digest.update(b"\n");
    for file in files {
        digest.update(file.path.len().to_string());
        digest.update(b":");
        digest.update(file.path);
        digest.update(b"\n");
        digest.update(format!("{:x}", Sha256::digest(file.content.as_bytes())));
        digest.update(b"\n");
    }
    format!("{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;
    use crate::plugin_dir::PluginDir;

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

    fn bundled(id: &str) -> &'static BundledHost {
        BUNDLED_HOSTS
            .iter()
            .find(|host| host.id == id)
            .unwrap_or_else(|| panic!("bundled host {id} is missing"))
    }

    fn released_host(
        version: &str,
        digest_byte: char,
        label: &str,
        default_for_id: bool,
    ) -> crate::plugin_catalog::CatalogHostRelease {
        let mut manifest = crate::plugin::first_party_plugins()[0].clone();
        manifest.id = "future-plugin".to_owned();
        manifest.version = version.to_owned();
        let package_digest = format!("sha256:{}", "1".repeat(64));
        let artifact_digest = format!("sha256:{}", digest_byte.to_string().repeat(64));
        crate::plugin_catalog::CatalogHostRelease {
            entry: crate::plugin_catalog::PluginCatalogEntry {
                plugin_id: manifest.id.clone(),
                plugin_version: manifest.version.clone(),
                plugin_kind: manifest.kind,
                package_digest: Some(package_digest.clone()),
                artifact_digest: Some(artifact_digest),
                release_state: crate::plugin_catalog::PluginReleaseState::Ready,
                release_detail: None,
                publisher: manifest.publisher.clone(),
                contract_fingerprint: Some(format!("sha256:{}", "2".repeat(64))),
                component_release: manifest.component_release.clone(),
                supported_platforms: Vec::new(),
                manifest,
                has_host_bundle: true,
                compatibility_requirements: None,
            },
            host_bundle: Some(crate::plugin_host_bundle::PluginHostBundle {
                schema: crate::plugin_host_bundle::HOST_BUNDLE_SCHEMA.to_owned(),
                plugin_id: "future-plugin".to_owned(),
                plugin_version: version.to_owned(),
                package_digest,
                files: BTreeMap::from([(
                    "host.json".to_owned(),
                    format!(r#"{{"schema_version":1,"label":"{label}"}}"#),
                )]),
            }),
            default_for_id,
        }
    }

    #[tokio::test]
    async fn exact_released_host_generations_coexist_with_one_explicit_default() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-plugin-runtime-exact-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let storage = PluginStorage::sqlite_files(PluginDir::open(&root).unwrap());
        let old = released_host("1.0.0", 'a', "Old", false);
        let current = released_host("2.0.0", 'b', "Current", true);
        let runtime = PluginRuntime::activate_releases(&storage, &[old, current], true)
            .await
            .unwrap();

        assert_eq!(
            runtime
                .exact_host(
                    "future-plugin",
                    "1.0.0",
                    &format!("sha256:{}", "a".repeat(64)),
                )
                .and_then(|host| host.label.as_deref()),
            Some("Old")
        );
        assert_eq!(
            runtime
                .default_host("future-plugin")
                .and_then(|host| host.label.as_deref()),
            Some("Current")
        );
        let public = runtime
            .public_descriptors()
            .into_iter()
            .map(|descriptor| serde_json::to_value(descriptor).unwrap())
            .filter(|descriptor| descriptor["id"] == "future-plugin")
            .collect::<Vec<_>>();
        assert_eq!(public.len(), 2);
        assert_eq!(
            public
                .iter()
                .filter(|descriptor| descriptor["default_for_id"] == true)
                .count(),
            1
        );
        assert!(public.iter().all(|descriptor| {
            descriptor["generation"]
                .as_str()
                .is_some_and(|generation| generation.len() == 64)
        }));

        let mut transient_gap = PluginRuntime::activate_releases(&storage, &[], true)
            .await
            .unwrap();
        transient_gap.retain_previous_generations(&runtime, &std::collections::BTreeSet::default());
        assert_eq!(
            transient_gap
                .default_host("future-plugin")
                .and_then(|host| host.label.as_deref()),
            Some("Current")
        );
        assert!(
            transient_gap
                .exact_host(
                    "future-plugin",
                    "1.0.0",
                    &format!("sha256:{}", "a".repeat(64)),
                )
                .is_some()
        );

        let mut hostless = released_host("3.0.0", 'c', "Unused", true);
        hostless.host_bundle = None;
        let mut explicit_candidate = PluginRuntime::activate_releases(&storage, &[hostless], true)
            .await
            .unwrap();
        explicit_candidate.retain_previous_generations(
            &runtime,
            &std::collections::BTreeSet::from(["future-plugin".to_owned()]),
        );
        assert!(explicit_candidate.default_host("future-plugin").is_none());
        assert!(
            explicit_candidate
                .exact_host(
                    "future-plugin",
                    "1.0.0",
                    &format!("sha256:{}", "a".repeat(64)),
                )
                .is_some()
        );

        unlock_tree(&root);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_auth_hosts_exclude_usage_plugins() {
        let login = ActivatedHostPlugin {
            id: "password".to_owned(),
            plugin_version: None,
            artifact_digest: None,
            generation: "a".repeat(64),
            slots: vec!["login.method"],
            ui: None,
            usage: None,
            label: Some("Password".to_owned()),
            adapter_slot: None,
            login_fields: None,
            rpc_argv: Vec::new(),
            visual: None,
            native_capabilities: Vec::new(),
        };
        let grok = ActivatedHostPlugin {
            id: "grok".to_owned(),
            plugin_version: None,
            artifact_digest: None,
            generation: "b".repeat(64),
            slots: vec!["provider.usage"],
            ui: None,
            usage: None,
            label: None,
            adapter_slot: None,
            login_fields: None,
            rpc_argv: Vec::new(),
            visual: None,
            native_capabilities: Vec::new(),
        };
        assert!(login.public_auth_surface());
        assert!(!grok.public_auth_surface());
    }

    #[test]
    #[allow(clippy::too_many_lines)] // one exhaustive private/public field fixture
    fn public_descriptor_cannot_serialize_private_host_execution_policy() {
        let spec = PluginHostSpec::from_json(
            br##"{
                "schema_version": 1,
                "slots": ["provider.usage"],
                "ui": {
                    "schema_version": 1,
                    "renderers": {"provider.usage": "provider-usage-activity-v1"}
                },
                "usage": {
                    "account": "future-account",
                    "collector": "future-collector",
                    "collector_argv": ["@plugin-js", "run", "${PLUGIN_DIR}/collector/index.js"],
                    "reset_argv": ["@plugin-js", "run", "${PLUGIN_DIR}/collector/reset.js"],
                    "reset": "future-reset",
                    "product": "Future Account",
                    "parser": "future-parser",
                    "error": "future-error",
                    "error_auth": "Sign in again.",
                    "error_config": "Configure the account.",
                    "error_fetch": "Could not refresh usage.",
                    "order": 7,
                    "top_bar_windows": [60],
                    "widget": "future-widget",
                    "widget_shape": "percent",
                    "widget_window": 60,
                    "reset_claim": "before-attempt",
                    "session_overlay": "future-overlay",
                    "session_rate_limits": {
                        "pointer": "/rate_limits",
                        "target": "limits",
                        "required_number_fields": ["used"]
                    },
                    "empty": "No limits yet.",
                    "available_status": "READY",
                    "omit_empty_limits": true,
                    "limit_id_prefix": "future",
                    "limit_labels": [{"id": "hour", "label": "1h", "window_minutes": 60}],
                    "widget_balance_label": "Balance",
                    "widget_spend_label": "Spend",
                    "activity_agents": [{"id": "future-agent", "label": "Future Agent"}],
                    "activity_models": [{"id": "future-model", "label": "Future Model"}],
                    "cache_protection": {
                        "min_hit_tokens": 1000,
                        "min_hit_label": "1K",
                        "interval_ms": 60000,
                        "interval_label": "1m",
                        "option_name": "Protection",
                        "option_description": "Protect the cache.",
                        "option_on": "On",
                        "option_off": "Off"
                    },
                    "activity": true
                },
                "storage": {
                    "postgres": {"migrations": [{"version": "0001", "sql": "CREATE TABLE notes (id TEXT PRIMARY KEY);"}]},
                    "sqlite": {"migrations": [{"version": "0001", "sql": "CREATE TABLE notes (id TEXT PRIMARY KEY);"}]}
                },
                "label": "Future",
                "adapter_slot": "future-agent",
                "login_fields": {"account": "Account"},
                "rpc_argv": ["@plugin-js", "run", "${PLUGIN_DIR}/collector/rpc.js"],
                "visual": {
                    "light": {"primary": "#112233", "secondary": "#445566"},
                    "dark": {"primary": "#AABBCC", "secondary": "#DDEEFF"}
                },
                "native_capabilities": ["future-native"]
            }"##,
        )
        .expect("future host spec");
        let private_generation = Path::new("/srv/cowboy/private/generations/future");
        let host = ActivatedHostPlugin {
            id: "future-plugin".to_owned(),
            plugin_version: Some("1.2.3".to_owned()),
            artifact_digest: Some(format!("sha256:{}", "c".repeat(64))),
            generation: "c".repeat(64),
            slots: spec
                .slots
                .iter()
                .map(|slot| PluginSlotId::as_str(*slot))
                .collect(),
            ui: spec.ui,
            usage: activated_usage(spec.usage.as_ref(), private_generation),
            label: spec.label,
            adapter_slot: spec.adapter_slot,
            login_fields: spec.login_fields,
            rpc_argv: expand_plugin_argv(&spec.rpc_argv, private_generation),
            visual: spec.visual,
            native_capabilities: spec.native_capabilities,
        };

        let public = serde_json::to_value(host.public_descriptor(true)).expect("public descriptor");
        let encoded = serde_json::to_string(&public).expect("public descriptor JSON");
        for private in [
            "storage",
            "usage_account",
            "rpc_argv",
            "collector",
            "collector_argv",
            "reset_argv",
            "reset_claim",
            "session_overlay",
            "session_rate_limits",
            "activity",
        ] {
            assert!(
                !encoded.contains(&format!("\"{private}\"")),
                "leaked {private}"
            );
        }
        assert!(!encoded.contains("/srv/cowboy/private"));
        assert_eq!(public["id"], "future-plugin");
        assert_eq!(public["plugin_version"], "1.2.3");
        assert_eq!(
            public["artifact_digest"],
            format!("sha256:{}", "c".repeat(64))
        );
        assert_eq!(public["generation"], "c".repeat(64));
        assert_eq!(public["default_for_id"], true);
        assert_eq!(public["usage"]["account"], "future-account");
        assert_eq!(public["usage"]["widget"], "future-widget");
        assert_eq!(public["adapter_slot"], "future-agent");
        assert_eq!(public["native_capabilities"][0], "future-native");
    }

    #[test]
    fn bundled_host_inventory_is_discovered_not_hardcoded() {
        let ids: Vec<&str> = BUNDLED_HOSTS.iter().map(|host| host.id).collect();
        for id in [
            "password",
            "passkey",
            "google",
            "apple",
            "cloudflare-email",
            "grok",
            "codex",
            "claude-code",
            "claude-deepseek",
            "codex-deepseek",
            "gemini",
        ] {
            assert!(ids.contains(&id), "expected bundled host {id}, got {ids:?}");
        }
        assert!(
            !ids.contains(&"oidc-login"),
            "shared renderer helpers are not plugins"
        );
    }

    #[test]
    fn trusted_release_presence_disables_a_bundled_host_fallback() {
        let released = std::collections::BTreeSet::from(["future-plugin".to_owned()]);
        assert!(!bundled_fallback_allowed(&released, "future-plugin"));
        assert!(bundled_fallback_allowed(&released, "another-plugin"));
    }

    #[test]
    fn host_files_need_only_include_signed_process_entrypoints() {
        let ui = PluginHostSpec::from_json(
            br#"{"schema_version":1,"slots":["login.method"],"ui":{"schema_version":1,"renderers":{"login.method":"login-password-v1"}}}"#,
        )
        .unwrap();
        let files =
            BTreeMap::from([("host.json".to_owned(), r#"{"schema_version":1}"#.to_owned())]);
        validate_host_files(&ui, &files).unwrap();

        let collector = PluginHostSpec::from_json(
            br#"{"schema_version":1,"rpc_argv":["@plugin-js","run","${PLUGIN_DIR}/collector/rpc.js"]}"#,
        )
        .unwrap();
        assert!(validate_host_files(&collector, &files).is_err());
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One integration fixture validates every bundled host contract.
    fn bundled_host_specs_validate() {
        for host in BUNDLED_HOSTS {
            let spec =
                PluginHostSpec::from_json(bundled_file(host, "host.json").unwrap().as_bytes())
                    .unwrap_or_else(|error| panic!("{}: {error}", host.id));
            if let Some(ui) = &spec.ui {
                assert_eq!(ui.schema_version, 1, "{}", host.id);
                assert_eq!(ui.renderers.len(), spec.slots.len(), "{}", host.id);
                assert!(
                    host.files.iter().all(|file| !file.path.starts_with("ui/")),
                    "{} contains executable UI",
                    host.id
                );
            } else {
                assert!(
                    spec.loopback_origin.is_some(),
                    "{} metadata-only host needs loopback_origin",
                    host.id
                );
            }
            if let Some(usage) = &spec.usage {
                for argv in [&usage.collector_argv, &usage.reset_argv] {
                    for argument in argv
                        .iter()
                        .filter_map(|argument| argument.strip_prefix("${PLUGIN_DIR}/"))
                    {
                        assert!(
                            bundled_file(host, argument).is_ok(),
                            "{} command references an unstaged sidecar {argument}",
                            host.id
                        );
                    }
                }
            }
        }
        let passkey = PluginHostSpec::from_json(
            bundled_file(bundled("passkey"), "host.json")
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        assert!(passkey.storage.is_some());
        assert!(passkey.slots.contains(&PluginSlotId::AccountPanel));
        for host in BUNDLED_HOSTS {
            let spec =
                PluginHostSpec::from_json(bundled_file(host, "host.json").unwrap().as_bytes())
                    .unwrap();
            if spec.slots.contains(&PluginSlotId::ProviderUsage) {
                assert!(spec.usage.is_some(), "{}", host.id);
                assert!(spec.adapter_slot.is_some(), "{}", host.id);
            }
            if spec.slots.contains(&PluginSlotId::LoginMethod) {
                assert!(spec.label.is_some(), "{}", host.id);
            }
            if let Some(env) = &spec.isolated_home_env {
                assert!(
                    env.bytes()
                        .next()
                        .is_some_and(|byte| { byte.is_ascii_uppercase() || byte == b'_' })
                        && env.bytes().all(|byte| {
                            byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                        }),
                    "{}",
                    host.id
                );
            }
            if spec.isolated_shell {
                assert!(spec.isolated_home_env.is_some(), "{}", host.id);
            }
        }
        let password = PluginHostSpec::from_json(
            bundled_file(bundled("password"), "host.json")
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        assert!(
            password
                .login_fields
                .as_ref()
                .and_then(|fields| fields.secret.as_deref())
                .is_some()
        );
        assert_eq!(
            PluginHostSpec::from_json(
                bundled_file(bundled("google"), "host.json")
                    .unwrap()
                    .as_bytes(),
            )
            .unwrap()
            .ui
            .unwrap()
            .renderers
            .get(&PluginSlotId::LoginMethod),
            Some(&crate::plugin_host::PluginRendererId::LoginOidcV1)
        );

        let codex = PluginHostSpec::from_json(
            bundled_file(bundled("codex"), "host.json")
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        let usage = activated_usage(codex.usage.as_ref(), Path::new("/tmp/plugin-generation"))
            .expect("Codex usage binding");
        assert!(
            usage
                .collector_argv
                .iter()
                .any(|argument| { argument == "/tmp/plugin-generation/collector/index.js" })
        );
        assert!(
            usage
                .collector_argv
                .iter()
                .all(|argument| { !argument.contains("${PLUGIN_DIR}") })
        );
    }

    #[test]
    fn bundled_generation_is_idempotent() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-plugin-runtime-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        let dir = PluginDir::open(&root).unwrap();
        let password = bundled("password");
        let password_revision = bundled_host_revision(password);
        let password_generation =
            stage_bundled_generation(&dir, password, &password_revision).unwrap();
        stage_bundled_generation(&dir, password, &password_revision).unwrap();
        dir.activate_host_generation("password", &password_generation)
            .unwrap();
        let current = dir.current_generation("password").unwrap().unwrap();
        assert!(
            current
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(&password_revision)
        );
        assert!(current.join("host.json").is_file());
        assert!(!current.join("ui").exists());
        let google = bundled("google");
        let google_revision = bundled_host_revision(google);
        let google_generation = stage_bundled_generation(&dir, google, &google_revision).unwrap();
        dir.activate_host_generation("google", &google_generation)
            .unwrap();
        assert!(
            dir.current_generation("google")
                .unwrap()
                .unwrap()
                .join("host.json")
                .is_file()
        );
        unlock_tree(&root);
        let _ = fs::remove_dir_all(root);
    }
}
