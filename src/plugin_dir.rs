//! Server-owned plugin directory.
//!
//! Every plugin generation, blob, catalog copy, and mutable plugin state lives
//! under `$COWBOY_DATA/plugins`. The controller binary does not embed plugin
//! trees. Machine hosts receive a projection of this tree over the network.

#![warn(clippy::pedantic)]

use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};

use crate::plugin_host::validate_plugin_id;

const LIVE: &str = "live";
const CATALOG: &str = "catalog";
const BLOBS: &str = "blobs";
const TRUSTED: &str = "trusted-publishers";
const GENERATIONS: &str = "generations";
const STATE: &str = "state";
const CURRENT: &str = "current";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDir {
    root: PathBuf,
}

impl PluginDir {
    /// Create the canonical plugin directory layout under `data_dir`.
    ///
    /// # Errors
    /// Returns when directories cannot be created.
    pub fn open(data_dir: &Path) -> Result<Self> {
        let this = Self {
            root: data_dir.join("plugins"),
        };
        for dir in [
            this.blobs_dir(),
            this.catalog_dir(),
            this.live_root(),
            this.trusted_publishers_dir(),
        ] {
            fs::create_dir_all(&dir)
                .with_context(|| format!("creating plugin dir {}", dir.display()))?;
        }
        Ok(this)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn catalog_dir(&self) -> PathBuf {
        self.root.join(CATALOG)
    }

    #[must_use]
    pub fn blobs_dir(&self) -> PathBuf {
        self.root.join(BLOBS)
    }

    #[must_use]
    pub fn live_root(&self) -> PathBuf {
        self.root.join(LIVE)
    }

    #[must_use]
    pub fn trusted_publishers_dir(&self) -> PathBuf {
        self.root.join(TRUSTED)
    }

    #[must_use]
    pub fn legacy_catalog_dir(data_dir: &Path) -> PathBuf {
        data_dir.join("plugin-catalog")
    }

    /// # Errors
    /// Returns when the plugin id is invalid.
    pub fn plugin_live_dir(&self, plugin_id: &str) -> Result<PathBuf> {
        validate_plugin_id(plugin_id)?;
        Ok(self.live_root().join(plugin_id))
    }

    /// Create `live/<id>/state` with mode 0700.
    ///
    /// # Errors
    /// Returns when the id is invalid or the directory cannot be created.
    pub fn ensure_state_dir(&self, plugin_id: &str) -> Result<PathBuf> {
        let live = self.plugin_live_dir(plugin_id)?;
        fs::create_dir_all(&live)
            .with_context(|| format!("creating plugin live dir {}", live.display()))?;
        let state = live.join(STATE);
        fs::create_dir_all(&state)
            .with_context(|| format!("creating plugin state dir {}", state.display()))?;
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("locking plugin state dir {}", state.display()))?;
        Ok(state)
    }

    /// # Errors
    /// Returns when identity fields are invalid.
    pub fn generation_dir(&self, plugin_id: &str, version: &str, digest: &str) -> Result<PathBuf> {
        validate_plugin_id(plugin_id)?;
        validate_generation_version(version)?;
        let digest = normalize_digest(digest)?;
        Ok(self
            .plugin_live_dir(plugin_id)?
            .join(GENERATIONS)
            .join(format!("{version}-{digest}")))
    }

    /// # Errors
    /// Returns when the plugin id is invalid.
    pub fn current_link(&self, plugin_id: &str) -> Result<PathBuf> {
        Ok(self.plugin_live_dir(plugin_id)?.join(CURRENT))
    }

    /// Resolve the currently activated generation, if the link exists.
    ///
    /// # Errors
    /// Returns when the plugin id is invalid or the current link is dangling.
    pub fn current_generation(&self, plugin_id: &str) -> Result<Option<PathBuf>> {
        let link = self.current_link(plugin_id)?;
        if !link.exists() {
            return Ok(None);
        }
        let target = fs::canonicalize(&link)
            .with_context(|| format!("resolving plugin current {}", link.display()))?;
        ensure!(
            target.starts_with(self.plugin_live_dir(plugin_id)?.join(GENERATIONS)),
            "plugin current link escaped its generation root"
        );
        Ok(Some(target))
    }

    /// Write host/UI files into a content-addressed generation and optionally
    /// point `current` at it.
    ///
    /// # Errors
    /// Returns when identity, paths, or filesystem writes fail.
    pub fn install_host_files(
        &self,
        plugin_id: &str,
        version: &str,
        digest: &str,
        files: &std::collections::BTreeMap<String, String>,
        make_current: bool,
    ) -> Result<PathBuf> {
        let generation = self.generation_dir(plugin_id, version, digest)?;
        fs::create_dir_all(&generation)
            .with_context(|| format!("creating plugin generation {}", generation.display()))?;
        for (relative, content) in files {
            ensure!(
                relative == "host.json"
                    || relative.starts_with("ui/") && !relative.split('/').any(|part| part == ".."),
                "refusing to install plugin path {relative}"
            );
            let path = generation.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, content)
                .with_context(|| format!("writing plugin file {}", path.display()))?;
        }
        if make_current {
            let current = self.current_link(plugin_id)?;
            if current.exists() || current.symlink_metadata().is_ok() {
                fs::remove_file(&current)
                    .with_context(|| format!("replacing plugin current {}", current.display()))?;
            }
            let target = PathBuf::from(GENERATIONS).join(format!(
                "{version}-{}",
                digest.strip_prefix("sha256:").unwrap_or(digest)
            ));
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

    /// SQLite file owned by this plugin. Isolated from the controller store.
    ///
    /// # Errors
    /// Returns when the id is invalid or the state directory cannot be created.
    pub fn sqlite_path(&self, plugin_id: &str) -> Result<PathBuf> {
        Ok(self.ensure_state_dir(plugin_id)?.join("db.sqlite"))
    }
}

fn validate_generation_version(version: &str) -> Result<()> {
    let parsed = semver::Version::parse(version).context("plugin generation version")?;
    ensure!(
        parsed.pre.is_empty() && parsed.build.is_empty(),
        "plugin generation version must be exact stable SemVer"
    );
    Ok(())
}

fn normalize_digest(digest: &str) -> Result<String> {
    let value = digest.strip_prefix("sha256:").unwrap_or(digest);
    ensure!(
        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "plugin generation digest must be sha256 hex"
    );
    Ok(value.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_canonical_layout() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-plugin-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        let dir = PluginDir::open(&root).unwrap();
        assert!(dir.catalog_dir().is_dir());
        assert!(dir.blobs_dir().is_dir());
        assert!(dir.live_root().is_dir());
        let state = dir.ensure_state_dir("passkey").unwrap();
        assert_eq!(state, root.join("plugins/live/passkey/state"));
        assert_eq!(
            fs::metadata(&state).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(dir.trusted_publishers_dir().is_dir());
        let generation = dir
            .generation_dir(
                "passkey",
                "1.0.0",
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            )
            .unwrap();
        assert!(generation.ends_with(
            "plugins/live/passkey/generations/1.0.0-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(dir.ensure_state_dir("../etc").is_err());
        let mut files = std::collections::BTreeMap::new();
        files.insert(
            "host.json".to_owned(),
            r#"{"schema_version":1,"slots":["login.method"]}"#.to_owned(),
        );
        files.insert(
            "ui/index.js".to_owned(),
            "export default function X(){}".to_owned(),
        );
        let generation = dir
            .install_host_files(
                "google",
                "1.0.0",
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                &files,
                true,
            )
            .unwrap();
        assert!(generation.join("ui").join("index.js").is_file());
        assert!(dir.current_generation("google").unwrap().is_some());
        let _ = fs::remove_dir_all(root);
    }
}
