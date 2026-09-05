//! Host data and runtime sidecars bound to a signed Plugin package digest.
//!
//! The `.cowboy-plugin` payload stays the Plugin SDK contract. Host JSON and
//! collector programs travel beside it as `*.hostbundle.json` so a Catalog
//! copy cannot attach host behavior to a different signed package. UI is
//! selected only through the data-only renderer map in `host.json`.

#![warn(clippy::pedantic)]

use std::collections::BTreeMap;
#[cfg(any(feature = "full", test))]
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::plugin_host::{PluginHostSpec, validate_plugin_id};

pub const HOST_BUNDLE_SCHEMA: &str = cowboy_plugin_sdk::HOST_BUNDLE_SCHEMA;
#[cfg(feature = "full")]
pub const HOST_BUNDLE_SCHEMA_VERSION: u16 = cowboy_plugin_sdk::HOST_BUNDLE_SCHEMA_VERSION;
#[cfg(test)]
const HOST_BUNDLE_CONTENT_NAMESPACE: &str = "cowboy-plugin-hostbundle-content-v1";
const MAX_FILES: usize = 32;
const MAX_FILE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_HOST_BUNDLE_BYTES: usize = 40 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginHostBundle {
    pub schema: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub package_digest: String,
    pub files: BTreeMap<String, String>,
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
        let version = semver::Version::parse(&self.plugin_version)
            .context("host bundle plugin version is invalid")?;
        ensure!(
            version.pre.is_empty() && version.build.is_empty(),
            "host bundle plugin version must be exact stable SemVer"
        );
        ensure!(
            self.package_digest.starts_with("sha256:")
                && self.package_digest.len() == 71
                && self.package_digest[7..]
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
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
        let host = PluginHostSpec::from_json(self.files["host.json"].as_bytes())
            .context("host bundle host.json is invalid")?;
        host.validate_runtime_files(&self.files)?;
        Ok(())
    }

    #[must_use]
    #[cfg(test)]
    pub fn proof(&self) -> Vec<u8> {
        let mut proof = format!("{HOST_BUNDLE_CONTENT_NAMESPACE}\n").into_bytes();
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

    /// Semantic content address for the complete host sidecar. The outer
    /// Plugin release separately binds the exact serialized bundle bytes.
    #[must_use]
    #[cfg(test)]
    pub fn content_digest(&self) -> String {
        format!("{:x}", Sha256::digest(self.proof()))
    }

    /// # Errors
    /// Returns when the sidecar is missing, malformed, not bound by the outer
    /// release, or does not match the owning Plugin package.
    #[cfg(test)]
    pub fn load_for_package(
        path: &Path,
        plugin_id: &str,
        plugin_version: &str,
        package_digest: &str,
        expected_digest: Option<&str>,
    ) -> Result<Option<Self>> {
        Ok(Self::load_bytes_for_package(
            path,
            plugin_id,
            plugin_version,
            package_digest,
            expected_digest,
        )?
        .map(|(bundle, _)| bundle))
    }

    /// Load and retain the exact serialized bytes authenticated by the outer
    /// release. Re-serializing the parsed value would create a different wire
    /// artifact even when its semantic content is unchanged.
    ///
    /// # Errors
    /// Returns under the same conditions as [`Self::load_for_package`].
    #[cfg(any(feature = "full", test))]
    pub fn load_bytes_for_package(
        path: &Path,
        plugin_id: &str,
        plugin_version: &str,
        package_digest: &str,
        expected_digest: Option<&str>,
    ) -> Result<Option<(Self, Vec<u8>)>> {
        let Some(expected_digest) = expected_digest else {
            ensure!(
                !path.exists(),
                "plugin host bundle is not bound by the signed release"
            );
            return Ok(None);
        };
        ensure!(path.exists(), "signed release host bundle is missing");
        ensure!(
            expected_digest.starts_with("sha256:")
                && expected_digest.len() == 71
                && expected_digest[7..]
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
            "signed release host bundle digest is invalid"
        );
        let bytes = fs::read(path)
            .with_context(|| format!("reading plugin host bundle {}", path.display()))?;
        let bundle = Self::from_bytes_for_package(
            Some(&bytes),
            plugin_id,
            plugin_version,
            package_digest,
            Some(expected_digest),
        )?
        .context("signed release host bundle is missing")?;
        Ok(Some((bundle, bytes)))
    }

    /// Authenticate serialized host bytes against one exact Plugin release.
    ///
    /// # Errors
    /// Returns when presence, digest, semantic content, or identity disagree
    /// with the signed release.
    pub fn from_bytes_for_package(
        bytes: Option<&[u8]>,
        plugin_id: &str,
        plugin_version: &str,
        package_digest: &str,
        expected_digest: Option<&str>,
    ) -> Result<Option<Self>> {
        let Some(expected_digest) = expected_digest else {
            ensure!(
                bytes.is_none(),
                "plugin host bundle is not bound by the signed release"
            );
            return Ok(None);
        };
        let bytes = bytes.context("signed release host bundle is missing")?;
        ensure!(
            expected_digest.starts_with("sha256:")
                && expected_digest.len() == 71
                && expected_digest[7..]
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
            "signed release host bundle digest is invalid"
        );
        ensure!(
            bytes.len() <= MAX_HOST_BUNDLE_BYTES,
            "plugin host bundle is too large"
        );
        ensure!(
            format!("sha256:{:x}", Sha256::digest(bytes)) == expected_digest,
            "plugin host bundle digest does not match signed release"
        );
        let bundle: Self = serde_json::from_slice(bytes).context("parsing plugin host bundle")?;
        bundle.validate()?;
        ensure!(
            bundle.plugin_id == plugin_id
                && bundle.plugin_version == plugin_version
                && bundle.package_digest == package_digest,
            "host bundle does not match signed plugin"
        );
        Ok(Some(bundle))
    }

    /// Collect `host.json` and signed collector scripts from a plugin source directory.
    ///
    /// # Errors
    /// Returns when host.json is missing or a collector path is unsafe.
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
        reject_ui_files(&root.join("ui"))?;
        let collector_root = root.join("collector");
        if collector_root.is_dir() {
            collect_sidecar_files(&collector_root, "collector", &mut files)?;
        }
        let bundle = Self {
            schema: HOST_BUNDLE_SCHEMA.to_owned(),
            plugin_id: plugin_id.to_owned(),
            plugin_version: plugin_version.to_owned(),
            package_digest: package_digest.to_owned(),
            files,
        };
        bundle.validate()?;
        Ok(Some(bundle))
    }
}

#[cfg(test)]
fn reject_ui_files(dir: &Path) -> Result<()> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            reject_ui_files(&entry.path())?;
        } else {
            anyhow::bail!(
                "plugin host UI code is forbidden: {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
fn collect_sidecar_files(
    dir: &Path,
    prefix: &str,
    files: &mut BTreeMap<String, String>,
) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .context("plugin collector file name is not UTF-8")?;
        let relative = format!("{prefix}/{name}");
        if entry.file_type()?.is_dir() {
            ensure!(
                name != "."
                    && name != ".."
                    && name.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                    }),
                "unsafe plugin host bundle path {relative}"
            );
            collect_sidecar_files(&entry.path(), &relative, files)?;
            continue;
        }
        if !matches!(
            Path::new(name)
                .extension()
                .and_then(std::ffi::OsStr::to_str),
            Some("js" | "json")
        ) {
            continue;
        }
        validate_bundle_path(&relative)?;
        files.insert(
            relative,
            fs::read_to_string(entry.path())
                .with_context(|| format!("reading {}", entry.path().display()))?,
        );
    }
    Ok(())
}

pub(crate) fn validate_bundle_path(path: &str) -> Result<()> {
    if path == "host.json" {
        return Ok(());
    }
    let Some(relative) = path.strip_prefix("collector/") else {
        anyhow::bail!("unsafe plugin host bundle path {path}");
    };
    ensure!(
        !relative.is_empty(),
        "unsafe plugin host bundle path {path}"
    );
    for part in relative.split('/') {
        ensure!(
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                }),
            "unsafe plugin host bundle path {path}"
        );
    }
    let extension = Path::new(relative)
        .extension()
        .and_then(std::ffi::OsStr::to_str);
    ensure!(
        extension.is_some_and(|extension| ["js", "json"].contains(&extension)),
        "unsupported collector file in plugin host bundle: {path}"
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
        };
        assert!(bundle.validate().is_err());
    }

    #[test]
    fn bundle_paths_and_content_addresses_are_closed() {
        let mut files = BTreeMap::from([
            ("host.json".to_owned(), r#"{"schema_version":1}"#.to_owned()),
            (
                "collector/nested/index.js".to_owned(),
                "export const collect = 1".to_owned(),
            ),
        ]);
        let mut bundle = PluginHostBundle {
            schema: HOST_BUNDLE_SCHEMA.to_owned(),
            plugin_id: "example".to_owned(),
            plugin_version: "1.0.0".to_owned(),
            package_digest: format!("sha256:{}", "a".repeat(64)),
            files: files.clone(),
        };
        bundle.validate().unwrap();
        let digest = bundle.content_digest();
        bundle.files.insert(
            "collector/nested/index.js".to_owned(),
            "export const collect = 2".to_owned(),
        );
        assert_ne!(bundle.content_digest(), digest);

        for unsafe_path in [
            "ui/./index.js",
            "ui/nested/../index.js",
            "ui/index.ts",
            "collector/run.sh",
            "ui/back\\slash.js",
        ] {
            files.insert(unsafe_path.to_owned(), "x".to_owned());
            bundle.files = files.clone();
            assert!(bundle.validate().is_err(), "accepted {unsafe_path}");
            files.remove(unsafe_path);
        }
    }

    #[test]
    fn bundle_rejects_invalid_host_contracts_and_missing_process_entries() {
        let bundle = |host: &str, files: BTreeMap<String, String>| PluginHostBundle {
            schema: HOST_BUNDLE_SCHEMA.to_owned(),
            plugin_id: "future-plugin".to_owned(),
            plugin_version: "1.0.0".to_owned(),
            package_digest: format!("sha256:{}", "a".repeat(64)),
            files: BTreeMap::from([("host.json".to_owned(), host.to_owned())])
                .into_iter()
                .chain(files)
                .collect(),
        };
        assert!(
            bundle(r#"{"schema_version":2}"#, BTreeMap::new())
                .validate()
                .is_err()
        );
        assert!(
            bundle(
                r#"{"schema_version":1,"rpc_argv":["@plugin-js","run","${PLUGIN_DIR}/collector/rpc.js"]}"#,
                BTreeMap::new(),
            )
            .validate()
            .is_err()
        );
        bundle(
            r#"{"schema_version":1,"rpc_argv":["@plugin-js","run","${PLUGIN_DIR}/collector/rpc.js"]}"#,
            BTreeMap::from([(
                "collector/rpc.js".to_owned(),
                "console.log('{}')".to_owned(),
            )]),
        )
        .validate()
        .expect("complete future host bundle");
    }

    #[test]
    fn from_plugin_dir_reads_data_only_google_example() {
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
        assert_eq!(bundle.files.len(), 1);
        assert!(bundle.files["host.json"].contains("login-oidc-v1"));
    }

    #[test]
    fn from_plugin_dir_reads_grok_collector_and_renderer_data() {
        let bundle = PluginHostBundle::from_plugin_dir(
            Path::new("plugins/grok"),
            "grok",
            "3.1.9",
            &format!("sha256:{}", "d".repeat(64)),
        )
        .unwrap()
        .unwrap();
        assert!(bundle.files.contains_key("host.json"));
        assert!(bundle.files.contains_key("collector/index.js"));
        assert!(bundle.files["collector/index.js"].contains("consume_reset"));
        assert!(bundle.files["host.json"].contains("provider-usage-v1"));
    }

    #[test]
    fn catalog_requires_exact_release_bound_host_bundle_bytes() {
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
        let bytes = serde_json::to_vec(&bundle).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let host_digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        assert!(
            PluginHostBundle::load_for_package(
                &path,
                "google",
                "1.0.0",
                &digest,
                Some(&format!("sha256:{}", "0".repeat(64))),
            )
            .is_err()
        );
        let loaded = PluginHostBundle::load_for_package(
            &path,
            "google",
            "1.0.0",
            &digest,
            Some(&host_digest),
        )
        .unwrap()
        .unwrap();
        assert_eq!(loaded.plugin_id, "google");
        assert!(
            PluginHostBundle::load_for_package(&path, "google", "1.0.0", &digest, None).is_err()
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
