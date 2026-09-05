//! Controller host activation policy, separate from Catalog publication.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use cowboy_plugin_sdk::{PluginHostSpec, PluginKind, PluginRendererId, PluginSlotId};
use serde::{Deserialize, Serialize};

use crate::plugin_catalog::{CatalogHostRelease, PluginCatalog};

const CONFIG_SCHEMA: &str = "dravengarden.cowboy.plugin-host-activation/v1";
const MAX_CONFIG_BYTES: u64 = 128 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HostSourcePolicy {
    #[default]
    Bootstrap,
    CatalogOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostReleasePin {
    pub plugin_id: String,
    pub plugin_version: String,
    pub artifact_digest: String,
}

impl HostReleasePin {
    fn validate(&self) -> Result<()> {
        crate::plugin_host::validate_plugin_id(&self.plugin_id)?;
        let version = semver::Version::parse(&self.plugin_version)?;
        ensure!(
            version.pre.is_empty() && version.build.is_empty(),
            "host pin requires exact stable SemVer"
        );
        ensure!(
            self.artifact_digest.len() == 71
                && self.artifact_digest.starts_with("sha256:")
                && self.artifact_digest[7..]
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
            "host pin requires a lowercase SHA-256 artifact digest"
        );
        Ok(())
    }

    fn matches(&self, release: &CatalogHostRelease) -> bool {
        release.entry.plugin_id == self.plugin_id
            && release.entry.plugin_version == self.plugin_version
            && release.entry.artifact_digest.as_deref() == Some(self.artifact_digest.as_str())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostActivationDocument {
    schema: String,
    source_policy: HostSourcePolicy,
    #[serde(default)]
    hosts: Vec<HostReleasePin>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct HostActivationPolicy {
    pub source: HostSourcePolicy,
    pins: BTreeMap<String, HostReleasePin>,
    required_bundles: BTreeSet<String>,
    pub authentication_methods: BTreeMap<String, PluginRendererId>,
    pub require_webauthn_storage: bool,
}

/// A read-only check receipt, never an activation or database readiness claim.
#[derive(Debug, Serialize)]
pub(crate) struct HostPreflightReport {
    schema: &'static str,
    status: &'static str,
    source_policy: HostSourcePolicy,
    catalog_only_recorded: bool,
    exact_selections: Vec<HostReleasePin>,
    login_methods: BTreeMap<String, PluginRendererId>,
    webauthn_storage_required: bool,
    catalog_defaults: Vec<HostPreflightDefault>,
    not_checked: [&'static str; 5],
}

#[derive(Debug, Serialize)]
struct HostPreflightDefault {
    release: HostReleasePin,
    has_host_bundle: bool,
}

/// Shared by the read-only CLI check and actual startup, before any Service
/// identity, cache, database, host generation or authority marker is created.
pub(crate) fn prepare_controller_hosts(
    args: &crate::cli::ServeArgs,
) -> Result<(PluginCatalog, crate::auth_plugins::ProductAuthentication)> {
    let mut catalog = PluginCatalog::inspect(&args.data_dir, args.plugin_catalog_dir.clone())?;
    let legacy = load_legacy_oidc_provider(
        args.product_auth_enabled,
        args.cardea_oidc_config.as_deref(),
    )?;
    let authentication = if args.product_auth_enabled {
        crate::auth_plugins::ProductAuthentication::load(
            args.auth_config.as_deref(),
            &catalog,
            legacy,
        )
        .context("loading product authentication methods")?
    } else {
        crate::auth_plugins::ProductAuthentication::disabled()
    };
    let mut policy = HostActivationPolicy::load(args.plugin_host_config.as_deref())
        .context("loading Plugin host activation policy")?;
    authentication.configure_host_policy(&mut policy)?;
    // Durable admin authentication also requires WebAuthn storage even when
    // Product authentication is disabled. Never connect merely to check this.
    policy.require_webauthn_storage |= args.database_url().is_some();
    catalog
        .configure_hosts(policy)
        .context("checking Plugin host activation readiness")?;
    Ok((catalog, authentication))
}

pub(crate) fn load_legacy_oidc_provider(
    enabled: bool,
    path: Option<&Path>,
) -> Result<Option<std::sync::Arc<crate::oidc::OidcProvider>>> {
    if !enabled {
        return Ok(None);
    }
    path.map(crate::oidc::OidcProvider::load)
        .transpose()
        .context("loading Cardea OIDC consumer profile")
        .map(|provider| provider.map(std::sync::Arc::new))
}

impl HostActivationPolicy {
    pub(crate) fn load(path: Option<&Path>) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        ensure!(
            path.is_absolute(),
            "Plugin host configuration path must be absolute"
        );
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .context("opening Plugin host activation configuration")?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.mode() & 0o077 == 0,
            "Plugin host configuration must be a private regular file (0600)"
        );
        ensure!(
            metadata.len() <= MAX_CONFIG_BYTES,
            "Plugin host configuration is too large"
        );
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_CONFIG_BYTES,
            "Plugin host configuration is too large"
        );
        Self::from_bytes(&bytes)
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let document: HostActivationDocument = crate::auth_plugins::decode_private_json(
            bytes,
            "Plugin host activation configuration",
        )?;
        ensure!(
            document.schema == CONFIG_SCHEMA,
            "unsupported Plugin host activation configuration"
        );
        ensure!(
            document.hosts.len() <= 128,
            "too many Plugin host selections"
        );
        let mut policy = Self {
            source: document.source_policy,
            ..Self::default()
        };
        for pin in document.hosts {
            ensure!(
                !policy.pins.contains_key(&pin.plugin_id),
                "duplicate Plugin host selection"
            );
            policy.pin(pin, true)?;
        }
        Ok(policy)
    }

    /// Merge the exact OIDC driver selection, rejecting a conflicting host pin.
    pub(crate) fn pin(&mut self, pin: HostReleasePin, require_bundle: bool) -> Result<()> {
        pin.validate()?;
        if let Some(existing) = self.pins.get(&pin.plugin_id) {
            ensure!(
                existing == &pin,
                "conflicting Plugin host selection for {}",
                pin.plugin_id
            );
        }
        if require_bundle {
            self.required_bundles.insert(pin.plugin_id.clone());
        }
        self.pins.insert(pin.plugin_id.clone(), pin);
        Ok(())
    }

    pub(crate) fn strict_activation(&self) -> bool {
        self.source == HostSourcePolicy::CatalogOnly || !self.pins.is_empty()
    }

    pub(crate) fn report(
        &self,
        releases: &[CatalogHostRelease],
        catalog_only_recorded: bool,
    ) -> Result<HostPreflightReport> {
        let catalog_defaults = releases
            .iter()
            .filter(|release| release.default_for_id)
            .map(|release| {
                Ok(HostPreflightDefault {
                    release: HostReleasePin {
                        plugin_id: release.entry.plugin_id.clone(),
                        plugin_version: release.entry.plugin_version.clone(),
                        artifact_digest: release
                            .entry
                            .artifact_digest
                            .clone()
                            .context("Catalog default has no exact release digest")?,
                    },
                    has_host_bundle: release.host_bundle.is_some(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(HostPreflightReport {
            schema: "dravengarden.cowboy.plugin-host-preflight/v1",
            status: "configuration_valid",
            source_policy: self.source,
            catalog_only_recorded,
            exact_selections: self.pins.values().cloned().collect(),
            login_methods: self.authentication_methods.clone(),
            webauthn_storage_required: self.require_webauthn_storage,
            catalog_defaults,
            not_checked: [
                "runtime_artifact_bytes",
                "generation_staging",
                "database_migrations",
                "credential_import",
                "live_authentication",
            ],
        })
    }

    /// Pure preflight: selects defaults and rejects incomplete candidates before
    /// staging, authority-marker writes, or any storage migration can run.
    pub(crate) fn select(&self, releases: &mut [CatalogHostRelease]) -> Result<()> {
        for pin in self.pins.values() {
            let release = releases
                .iter()
                .find(|release| pin.matches(release))
                .with_context(|| {
                    format!(
                        "selected Plugin host {} {} {} is missing from the trusted Catalog",
                        pin.plugin_id, pin.plugin_version, pin.artifact_digest
                    )
                })?;
            ensure!(
                !self.required_bundles.contains(&pin.plugin_id) || release.host_bundle.is_some(),
                "selected Plugin {} requires a release-bound host bundle",
                pin.plugin_id
            );
        }
        for release in releases.iter_mut() {
            if let Some(pin) = self.pins.get(&release.entry.plugin_id) {
                release.default_for_id = pin.matches(release);
            } else {
                let owns_storage =
                    release.host_bundle.is_some() && host_spec(release)?.storage.is_some();
                if owns_storage
                    || (self.source == HostSourcePolicy::CatalogOnly
                        && release.entry.plugin_kind == PluginKind::AuthenticationProvider)
                {
                    // A Catalog default is presentation, not migration authority.
                    // Every released storage host needs an exact policy pin,
                    // regardless of payload kind or bootstrap/cutover mode.
                    release.default_for_id = false;
                }
            }
        }
        for (id, renderer) in &self.authentication_methods {
            if self.source == HostSourcePolicy::Bootstrap && !self.pins.contains_key(id) {
                continue;
            }
            let release = releases
                .iter()
                .find(|release| release.default_for_id && release.entry.plugin_id == *id)
                .with_context(|| {
                    format!("catalog_only requires an exact host selection for login method {id}")
                })?;
            if self.source == HostSourcePolicy::Bootstrap
                && release.host_bundle.is_none()
                && !self.required_bundles.contains(id)
            {
                // Existing signed OIDC contracts may predate host bundles.
                continue;
            }
            let host = host_spec(release)?;
            ensure!(
                release.entry.plugin_kind == PluginKind::AuthenticationProvider
                    && host
                        .ui
                        .as_ref()
                        .is_some_and(
                            |ui| ui.renderers.get(&PluginSlotId::LoginMethod) == Some(renderer)
                        ),
                "selected Plugin {id} does not implement the configured login method"
            );
        }
        if self.source != HostSourcePolicy::CatalogOnly {
            return Ok(());
        }
        if self.require_webauthn_storage {
            let mut claims = Vec::new();
            for release in releases
                .iter()
                .filter(|release| release.default_for_id && release.host_bundle.is_some())
            {
                let host = host_spec(release)?;
                if host
                    .native_capabilities
                    .iter()
                    .any(|capability| capability == "webauthn")
                {
                    claims.push((release, host));
                }
            }
            ensure!(
                claims.len() == 1,
                "catalog_only requires exactly one selected WebAuthn storage host, including for durable admin authentication"
            );
            let (release, host) = &claims[0];
            ensure!(
                release.entry.plugin_kind == PluginKind::AuthenticationProvider
                    && host.storage.is_some()
                    && host
                        .ui
                        .as_ref()
                        .is_some_and(|ui| ui.renderers.get(&PluginSlotId::AccountPanel)
                            == Some(&PluginRendererId::AccountPasskeysV1)),
                "selected WebAuthn host does not implement authentication storage"
            );
        }
        Ok(())
    }
}

fn host_spec(release: &CatalogHostRelease) -> Result<PluginHostSpec> {
    let bundle = release.host_bundle.as_ref().with_context(|| {
        format!(
            "selected Plugin {} requires a release-bound host bundle",
            release.entry.plugin_id
        )
    })?;
    PluginHostSpec::from_json(
        bundle
            .files
            .get("host.json")
            .context("selected Plugin is missing host.json")?
            .as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    fn document() -> serde_json::Value {
        serde_json::json!({
            "schema": CONFIG_SCHEMA,
            "source_policy": "catalog_only",
            "hosts": [{"plugin_id":"password", "plugin_version":"1.0.0",
                "artifact_digest": format!("sha256:{}", "a".repeat(64))}]
        })
    }

    fn parse(value: &serde_json::Value) -> Result<HostActivationPolicy> {
        HostActivationPolicy::from_bytes(&serde_json::to_vec(value)?)
    }

    #[test]
    fn configuration_is_closed_and_selections_are_exact() {
        let bootstrap = HostActivationPolicy::load(None).unwrap();
        assert_eq!(bootstrap.source, HostSourcePolicy::Bootstrap);
        assert!(!bootstrap.strict_activation());
        let valid = document();
        let selected = parse(&valid).unwrap();
        assert!(selected.strict_activation());
        assert!(selected.required_bundles.contains("password"));
        for (pointer, value) in [
            ("/schema", serde_json::json!("unknown")),
            ("/source_policy", serde_json::json!("latest")),
            ("/hosts/0/plugin_id", serde_json::json!("../password")),
            ("/hosts/0/plugin_version", serde_json::json!("^1.0")),
            ("/hosts/0/plugin_version", serde_json::json!("1.0.0-beta")),
            (
                "/hosts/0/plugin_version",
                serde_json::json!("1.0.0+mutable"),
            ),
            ("/hosts/0/artifact_digest", serde_json::json!("sha256:abc")),
            (
                "/hosts/0/artifact_digest",
                serde_json::json!(format!("sha256:{}", "A".repeat(64))),
            ),
        ] {
            let mut invalid = valid.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            assert!(parse(&invalid).is_err(), "accepted {pointer}");
        }
        let mut unknown = valid.clone();
        unknown["auto_upgrade"] = serde_json::json!(true);
        assert!(parse(&unknown).is_err());
        let mut unknown = valid.clone();
        unknown["hosts"][0]["latest"] = serde_json::json!(true);
        assert!(parse(&unknown).is_err());
        let mut duplicates = valid;
        let duplicate = duplicates["hosts"][0].clone();
        duplicates["hosts"].as_array_mut().unwrap().push(duplicate);
        assert!(
            parse(&duplicates)
                .unwrap_err()
                .to_string()
                .contains("duplicate")
        );
    }

    #[test]
    fn oidc_merge_cannot_override_an_explicit_host_selection() {
        let mut policy = parse(&document()).unwrap();
        let pin = policy.pins["password"].clone();
        policy.pin(pin.clone(), false).unwrap();
        assert!(policy.required_bundles.contains("password"));
        let mut conflicting = pin;
        conflicting.artifact_digest = format!("sha256:{}", "b".repeat(64));
        assert!(
            policy
                .pin(conflicting, false)
                .unwrap_err()
                .to_string()
                .contains("conflicting")
        );
    }

    #[test]
    fn configuration_requires_a_private_bounded_regular_file() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-host-policy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("policy.json");
        std::fs::write(&path, serde_json::to_vec(&document()).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            HostActivationPolicy::load(Some(&path))
                .unwrap()
                .strict_activation()
        );
        let link = root.join("link.json");
        symlink(&path, &link).unwrap();
        assert!(HostActivationPolicy::load(Some(&link)).is_err());
        assert!(HostActivationPolicy::load(Some(&root)).is_err());
        assert!(HostActivationPolicy::load(Some(Path::new("policy.json"))).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(HostActivationPolicy::load(Some(&path)).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_CONFIG_BYTES + 1)
            .unwrap();
        assert!(
            HostActivationPolicy::load(Some(&path))
                .unwrap_err()
                .to_string()
                .contains("too large")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
