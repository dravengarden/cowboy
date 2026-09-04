//! Host UI sidecar bound to a signed Plugin package digest.
//!
//! The `.cowboy-plugin` payload stays the Plugin SDK contract. Host JSON and
//! UI modules travel beside it as `*.hostbundle.json` so a Catalog copy cannot
//! attach UI to a different signed package.

#![warn(clippy::pedantic)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::machine_auth::{PLUGIN_HOSTBUNDLE_SIGNATURE_NAMESPACE, verify_namespaced};
use crate::plugin_host::validate_plugin_id;

pub const HOST_BUNDLE_SCHEMA: &str = "dravengarden.cowboy.plugin-hostbundle/v1";
const MAX_FILES: usize = 32;
const MAX_FILE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginHostBundle {
    pub schema: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub package_digest: String,
    pub files: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature: String,
}

impl PluginHostBundle {
    /// # Errors
    /// Returns when identity, digest, paths, or size limits are invalid.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == HOST_BUNDLE_SCHEMA,
            "unsupported plugin host bundle schema"
        );
        validate_plugin_id(&self.plugin_id)?;
        ensure!(
            !self.plugin_version.is_empty(),
            "host bundle plugin version is empty"
        );
        ensure!(
            self.package_digest.starts_with("sha256:") && self.package_digest.len() == 71,
            "host bundle package digest is invalid"
        );
        ensure!(
            !self.files.is_empty() && self.files.len() <= MAX_FILES,
            "host bundle file set is empty or too large"
        );
        for (path, content) in &self.files {
            validate_bundle_path(path)?;
            ensure!(
                !content.is_empty() && content.len() <= MAX_FILE_BYTES,
                "host bundle file {path} is empty or oversized"
            );
            ensure!(
                !content.contains('\0'),
                "host bundle file {path} contains NUL"
            );
        }
        ensure!(
            self.files.contains_key("host.json"),
            "host bundle is missing host.json"
        );
        Ok(())
    }

    #[must_use]
    pub fn proof(&self) -> Vec<u8> {
        let mut proof = format!("{PLUGIN_HOSTBUNDLE_SIGNATURE_NAMESPACE}\n").into_bytes();
        for field in [
            self.schema.as_str(),
            self.plugin_id.as_str(),
            self.plugin_version.as_str(),
            self.package_digest.as_str(),
        ] {
            proof.extend_from_slice(field.len().to_string().as_bytes());
            proof.push(b':');
            proof.extend_from_slice(field.as_bytes());
            proof.push(b'\n');
        }
        for (path, content) in &self.files {
            proof.extend_from_slice(path.len().to_string().as_bytes());
            proof.push(b':');
            proof.extend_from_slice(path.as_bytes());
            proof.push(b'\n');
            let digest = format!("{:x}", Sha256::digest(content.as_bytes()));
            proof.extend_from_slice(digest.as_bytes());
            proof.push(b'\n');
        }
        proof
    }

    /// # Errors
    /// Returns when the sidecar is malformed, unsigned, or does not match.
    pub fn load_for_package(
        path: &Path,
        plugin_id: &str,
        plugin_version: &str,
        package_digest: &str,
        public_key: &str,
    ) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let bundle: Self = serde_json::from_slice(
            &fs::read(path)
                .with_context(|| format!("reading plugin host bundle {}", path.display()))?,
        )
        .with_context(|| format!("parsing plugin host bundle {}", path.display()))?;
        bundle.validate()?;
        ensure!(
            bundle.plugin_id == plugin_id
                && bundle.plugin_version == plugin_version
                && bundle.package_digest == package_digest,
            "host bundle does not match signed plugin {}",
            path.display()
        );
        ensure!(!bundle.signature.is_empty(), "host bundle is unsigned");
        ensure!(
            verify_namespaced(
                public_key,
                PLUGIN_HOSTBUNDLE_SIGNATURE_NAMESPACE,
                &bundle.proof(),
                &bundle.signature,
            )?,
            "host bundle signature is invalid"
        );
        Ok(Some(bundle))
    }

    /// # Errors
    /// Returns when ssh-keygen cannot sign the host bundle proof.
    #[cfg(test)]
    pub fn sign(&mut self, identity: &crate::machine_auth::MachineIdentity) -> Result<()> {
        self.signature =
            identity.sign_namespaced(PLUGIN_HOSTBUNDLE_SIGNATURE_NAMESPACE, &self.proof())?;
        Ok(())
    }

    /// Collect `host.json` and `ui/**` from a plugin source directory.
    ///
    /// # Errors
    /// Returns when host.json is missing or a UI path is unsafe.
    #[cfg(test)]
    pub fn from_plugin_dir(
        root: &Path,
        plugin_id: &str,
        plugin_version: &str,
        package_digest: &str,
    ) -> Result<Option<Self>> {
        let host_path = root.join("host.json");
        if !host_path.exists() {
            return Ok(None);
        }
        let mut files = BTreeMap::new();
        files.insert(
            "host.json".to_owned(),
            fs::read_to_string(&host_path)
                .with_context(|| format!("reading {}", host_path.display()))?,
        );
        let ui_root = root.join("ui");
        if ui_root.is_dir() {
            collect_ui_files(&ui_root, "ui", &mut files)?;
        }
        let bundle = Self {
            schema: HOST_BUNDLE_SCHEMA.to_owned(),
            plugin_id: plugin_id.to_owned(),
            plugin_version: plugin_version.to_owned(),
            package_digest: package_digest.to_owned(),
            files,
            signature: String::new(),
        };
        bundle.validate()?;
        Ok(Some(bundle))
    }
}

#[cfg(test)]
fn collect_ui_files(dir: &Path, prefix: &str, files: &mut BTreeMap<String, String>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().context("plugin UI file name is not UTF-8")?;
        let relative = format!("{prefix}/{name}");
        validate_bundle_path(&relative)?;
        if entry.file_type()?.is_dir() {
            collect_ui_files(&entry.path(), &relative, files)?;
            continue;
        }
        if !(name.ends_with(".js") || name.ends_with(".css") || name.ends_with(".json")) {
            continue;
        }
        files.insert(
            relative,
            fs::read_to_string(entry.path())
                .with_context(|| format!("reading {}", entry.path().display()))?,
        );
    }
    Ok(())
}

fn validate_bundle_path(path: &str) -> Result<()> {
    ensure!(
        path == "host.json"
            || path.starts_with("ui/")
                && !path.contains("//")
                && !path.split('/').any(|part| part.is_empty() || part == ".."),
        "unsafe plugin host bundle path {path}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_rejects_path_escape_and_core_mismatch() {
        let mut files = BTreeMap::new();
        files.insert("host.json".to_owned(), r#"{"schema_version":1}"#.to_owned());
        files.insert("../evil.js".to_owned(), "alert(1)".to_owned());
        let bundle = PluginHostBundle {
            schema: HOST_BUNDLE_SCHEMA.to_owned(),
            plugin_id: "google".to_owned(),
            plugin_version: "1.0.0".to_owned(),
            package_digest: format!("sha256:{}", "a".repeat(64)),
            files,
            signature: String::new(),
        };
        assert!(bundle.validate().is_err());
    }

    #[test]
    fn from_plugin_dir_reads_google_example() {
        let root = Path::new("examples/authentication/google");
        let bundle = PluginHostBundle::from_plugin_dir(
            root,
            "google",
            "1.0.0",
            &format!("sha256:{}", "b".repeat(64)),
        )
        .unwrap()
        .unwrap();
        assert!(bundle.files.contains_key("host.json"));
        assert!(bundle.files.contains_key("ui/index.js"));
        assert!(bundle.files["ui/index.js"].contains("oidc"));
    }

    #[test]
    fn from_plugin_dir_reads_grok_usage_ui() {
        let bundle = PluginHostBundle::from_plugin_dir(
            Path::new("plugins/grok"),
            "grok",
            "3.1.7",
            &format!("sha256:{}", "d".repeat(64)),
        )
        .unwrap()
        .unwrap();
        assert!(bundle.files.contains_key("host.json"));
        assert!(bundle.files.contains_key("ui/index.js"));
        assert!(bundle.files["ui/index.js"].contains("provider.usage"));
        assert!(bundle.files["host.json"].contains("provider.usage"));
    }

    #[test]
    fn catalog_rejects_unsigned_host_bundle() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-hostbundle-unsigned-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let digest = format!("sha256:{}", "c".repeat(64));
        let bundle = PluginHostBundle::from_plugin_dir(
            Path::new("examples/authentication/google"),
            "google",
            "1.0.0",
            &digest,
        )
        .unwrap()
        .unwrap();
        let path = root.join("google.hostbundle.json");
        std::fs::write(&path, serde_json::to_vec(&bundle).unwrap()).unwrap();
        let identity =
            crate::machine_auth::MachineIdentity::load_or_create(&root.join("identity")).unwrap();
        assert!(
            PluginHostBundle::load_for_package(
                &path,
                "google",
                "1.0.0",
                &digest,
                identity.public_key(),
            )
            .is_err()
        );
        let mut signed = bundle;
        signed.sign(&identity).unwrap();
        std::fs::write(&path, serde_json::to_vec(&signed).unwrap()).unwrap();
        let loaded = PluginHostBundle::load_for_package(
            &path,
            "google",
            "1.0.0",
            &digest,
            identity.public_key(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(loaded.plugin_id, "google");
        let _ = std::fs::remove_dir_all(root);
    }
}
