//! Activate bundled host plugins into the server plugin directory.
//!
//! First-party host trees (`examples/authentication/*/host.json` and
//! `plugins/*/host.json`) are discovered at compile time and staged into
//! `plugins/live/<id>/generations/bundled/` on first boot. Catalog-signed
//! packages replace that generation later without changing core.

#![warn(clippy::pedantic)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};

use crate::plugin_dir::PluginDir;
use crate::plugin_host::{PluginHostSpec, PluginSlotId, PluginUsageSpec};
use crate::plugin_storage::{PluginNamespace, PluginStorage};
use crate::store::Store;

const BUNDLED_GENERATION: &str = "bundled";

struct BundledHostFile {
    path: &'static str,
    content: &'static str,
}

struct BundledHost {
    id: &'static str,
    files: &'static [BundledHostFile],
}

const BUNDLED_HOSTS: &[BundledHost] = include!(concat!(env!("OUT_DIR"), "/bundled_hosts.rs"));

#[derive(Debug, Clone, serde::Serialize)]
pub struct ActivatedHostPlugin {
    pub id: String,
    pub slots: Vec<&'static str>,
    pub storage: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<PluginUsageSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter_slot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_fields: Option<crate::plugin_host::PluginLoginFields>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rpc_argv: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visual: Option<crate::plugin_host::PluginVisualSpec>,
}

impl ActivatedHostPlugin {
    #[must_use]
    pub fn public_auth_surface(&self) -> bool {
        self.slots.iter().any(|slot| {
            *slot == PluginSlotId::LoginMethod.as_str()
                || *slot == PluginSlotId::AccountPanel.as_str()
        })
    }
}

pub struct PluginRuntime {
    pub hosts: Vec<ActivatedHostPlugin>,
    pub passkey: Option<PluginNamespace>,
}

impl PluginRuntime {
    #[must_use]
    pub fn usage_bindings(&self) -> Vec<PluginUsageSpec> {
        self.hosts
            .iter()
            .filter_map(|host| host.usage.clone())
            .collect()
    }

    /// Stage bundled host plugins and migrate any plugin-owned storage.
    ///
    /// # Errors
    /// Returns when a host spec is invalid. Storage failures are logged and
    /// skip that plugin rather than aborting the controller.
    pub async fn activate(
        storage: &PluginStorage,
        store: Option<&Store>,
        catalog: Option<&crate::plugin_catalog::PluginCatalog>,
    ) -> Result<Self> {
        let mut hosts = Vec::new();
        let mut passkey = None;
        for host in BUNDLED_HOSTS {
            let source = bundled_file(host, "host.json")
                .with_context(|| format!("bundled {} host.json", host.id))?;
            let spec = PluginHostSpec::from_json(source.as_bytes())
                .with_context(|| format!("bundled {} host spec", host.id))?;
            let generation = stage_bundled_generation(storage.plugin_dir(), host)?;
            let staged = PluginHostSpec::load_optional(&generation)?
                .with_context(|| format!("bundled {} host spec missing after stage", host.id))?;
            ensure!(
                staged == spec,
                "bundled {} host spec does not match staged files",
                host.id
            );
            activate_host(storage, store, host.id, &spec, &mut hosts, &mut passkey).await;
        }
        if let Some(catalog) = catalog {
            for (entry, bundle) in catalog.host_bundles() {
                if let Err(error) = storage.plugin_dir().install_host_files(
                    &entry.plugin_id,
                    &entry.plugin_version,
                    &bundle.package_digest,
                    &bundle.files,
                    true,
                ) {
                    tracing::error!(
                        %error,
                        plugin_id = entry.plugin_id.as_str(),
                        "installing catalog host bundle failed"
                    );
                    continue;
                }
                let Some(host_json) = bundle.files.get("host.json") else {
                    continue;
                };
                match PluginHostSpec::from_json(host_json.as_bytes()) {
                    Ok(spec) => {
                        activate_host(
                            storage,
                            store,
                            &entry.plugin_id,
                            &spec,
                            &mut hosts,
                            &mut passkey,
                        )
                        .await;
                    }
                    Err(error) => tracing::error!(
                        %error,
                        plugin_id = entry.plugin_id.as_str(),
                        "catalog host spec is invalid"
                    ),
                }
            }
        }
        Ok(Self { hosts, passkey })
    }
}

async fn activate_host(
    storage: &PluginStorage,
    store: Option<&Store>,
    plugin_id: &str,
    spec: &PluginHostSpec,
    hosts: &mut Vec<ActivatedHostPlugin>,
    passkey: &mut Option<PluginNamespace>,
) {
    let mut activated = ActivatedHostPlugin {
        id: plugin_id.to_owned(),
        slots: spec
            .slots
            .iter()
            .map(|slot| PluginSlotId::as_str(*slot))
            .collect(),
        storage: spec.storage.is_some(),
        usage_account: spec.usage.as_ref().map(|usage| usage.account.clone()),
        usage: spec.usage.clone(),
        label: spec.label.clone(),
        adapter_slot: spec.adapter_slot.clone(),
        login_fields: spec.login_fields.clone(),
        rpc_argv: spec.rpc_argv.clone(),
        visual: spec.visual.clone(),
    };
    if let Some(storage_spec) = &spec.storage {
        match storage.migrate_plugin(plugin_id, storage_spec).await {
            Ok(namespace) => {
                tracing::debug!(
                    plugin_id = namespace.plugin_id(),
                    schema = namespace.schema_name(),
                    "plugin storage ready"
                );
                if plugin_id == "passkey" {
                    if let Some(store) = store
                        && let Err(error) =
                            crate::plugin_passkeys::import_from_core(&namespace, store).await
                    {
                        tracing::error!(%error, plugin_id, "passkey plugin import from core failed");
                    }
                    *passkey = Some(namespace);
                }
            }
            Err(error) => {
                tracing::error!(%error, plugin_id, "plugin storage migrate failed");
                activated.storage = false;
            }
        }
    }
    hosts.retain(|host| host.id != plugin_id);
    hosts.push(activated);
}

fn bundled_file<'a>(host: &'a BundledHost, path: &str) -> Result<&'a str> {
    host.files
        .iter()
        .find(|file| file.path == path)
        .map(|file| file.content)
        .with_context(|| format!("bundled {} is missing {path}", host.id))
}

fn stage_bundled_generation(dir: &PluginDir, host: &BundledHost) -> Result<PathBuf> {
    let generation = dir
        .plugin_live_dir(host.id)?
        .join("generations")
        .join(BUNDLED_GENERATION);
    for file in host.files {
        ensure!(
            file.path == "host.json"
                || (file.path.starts_with("ui/") && !file.path.split('/').any(|part| part == "..")),
            "refusing to stage bundled plugin path {}",
            file.path
        );
        let path = generation.join(file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "creating bundled plugin generation {}",
                    generation.display()
                )
            })?;
        }
        fs::write(&path, file.content)
            .with_context(|| format!("writing {} {}", host.id, file.path))?;
    }
    let current = dir.current_link(host.id)?;
    if !current.exists() {
        if let Some(parent) = current.parent() {
            fs::create_dir_all(parent)?;
        }
        let target = Path::new("generations").join(BUNDLED_GENERATION);
        symlink(&target, &current).with_context(|| {
            format!(
                "linking plugin current {} -> {}",
                current.display(),
                target.display()
            )
        })?;
    }
    Ok(generation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_dir::PluginDir;

    fn bundled(id: &str) -> &'static BundledHost {
        BUNDLED_HOSTS
            .iter()
            .find(|host| host.id == id)
            .unwrap_or_else(|| panic!("bundled host {id} is missing"))
    }

    #[test]
    fn public_auth_hosts_exclude_usage_plugins() {
        let login = ActivatedHostPlugin {
            id: "password".to_owned(),
            slots: vec!["login.method"],
            storage: false,
            usage_account: None,
            usage: None,
            label: Some("Password".to_owned()),
            adapter_slot: None,
            login_fields: None,
            rpc_argv: Vec::new(),
            visual: None,
        };
        let grok = ActivatedHostPlugin {
            id: "grok".to_owned(),
            slots: vec!["provider.usage"],
            storage: false,
            usage_account: Some("xai".to_owned()),
            usage: None,
            label: None,
            adapter_slot: None,
            login_fields: None,
            rpc_argv: Vec::new(),
            visual: None,
        };
        assert!(login.public_auth_surface());
        assert!(!grok.public_auth_surface());
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
            "shared oidc-login.js is not a plugin"
        );
    }

    #[test]
    fn bundled_host_specs_validate() {
        for host in BUNDLED_HOSTS {
            let spec =
                PluginHostSpec::from_json(bundled_file(host, "host.json").unwrap().as_bytes())
                    .unwrap_or_else(|error| panic!("{}: {error}", host.id));
            if spec.ui.is_some() {
                assert_eq!(
                    spec.ui.as_ref().map(|ui| ui.entry.as_str()),
                    Some("ui/index.js"),
                    "{}",
                    host.id
                );
                assert!(
                    bundled_file(host, "ui/index.js")
                        .unwrap()
                        .contains("__COWBOY_PLUGIN_HOST"),
                    "{}",
                    host.id
                );
            } else {
                assert!(
                    spec.loopback_origin.is_some(),
                    "{} metadata-only host needs loopback_origin",
                    host.id
                );
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
        assert!(
            bundled_file(bundled("google"), "ui/index.js")
                .unwrap()
                .contains("oidc")
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
        stage_bundled_generation(&dir, password).unwrap();
        stage_bundled_generation(&dir, password).unwrap();
        let current = dir.current_generation("password").unwrap().unwrap();
        assert!(current.join("host.json").is_file());
        assert!(current.join("ui").join("index.js").is_file());
        assert!(
            std::fs::read_to_string(current.join("ui").join("index.js"))
                .unwrap()
                .contains("__COWBOY_PLUGIN_HOST")
        );
        let google = bundled("google");
        stage_bundled_generation(&dir, google).unwrap();
        assert!(
            dir.current_generation("google")
                .unwrap()
                .unwrap()
                .join("ui")
                .join("index.js")
                .is_file()
        );
        let _ = fs::remove_dir_all(root);
    }
}
