//! Server-owned plugin directory.
//!
//! Every plugin generation, blob, catalog copy, and mutable plugin state lives
//! under `$COWBOY_DATA/plugins`. Transitional source host declarations may be
//! bundled by the Controller, but Catalog-only activation never stages them.
//! Machine hosts receive signed generations over the network.

#![warn(clippy::pedantic)]

use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context as _, Result, ensure};

use crate::plugin_host::validate_plugin_id;

const LIVE: &str = "live";
const CATALOG: &str = "catalog";
const BLOBS: &str = "blobs";
const TRUSTED: &str = "trusted-publishers";
const GENERATIONS: &str = "generations";
const STATE: &str = "state";
const CURRENT: &str = "current";
const CATALOG_AUTHORITY: &str = ".catalog-authority-v1";
#[cfg(feature = "full")]
const CATALOG_ONLY: &str = ".catalog-only-v1";
#[cfg(feature = "full")]
const CATALOG_ONLY_CONTENT: &[u8] = b"signed-catalog-only-v1\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDir {
    root: PathBuf,
}

impl PluginDir {
    /// Resolve the layout without creating directories or mutable state.
    #[must_use]
    pub(crate) fn inspect(data_dir: &Path) -> Self {
        Self {
            root: data_dir.join("plugins"),
        }
    }

    /// Create the canonical plugin directory layout under `data_dir`.
    ///
    /// # Errors
    /// Returns when directories cannot be created.
    pub fn open(data_dir: &Path) -> Result<Self> {
        let this = Self::inspect(data_dir);
        this.initialize()?;
        Ok(this)
    }

    pub(crate) fn initialize(&self) -> Result<()> {
        for dir in [
            self.blobs_dir(),
            self.catalog_dir(),
            self.live_root(),
            self.trusted_publishers_dir(),
        ] {
            fs::create_dir_all(&dir)
                .with_context(|| format!("creating plugin dir {}", dir.display()))?;
        }
        Ok(())
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

    /// Permanently record that this Plugin ID has crossed the signed Catalog
    /// authority boundary. Removing or temporarily hiding Catalog files must
    /// never reactivate source-bundled behavior for that ID.
    ///
    /// # Errors
    /// Returns when the marker cannot be created or an existing path is not
    /// the exact regular marker written by Cowboy.
    pub fn record_catalog_authority(&self, plugin_id: &str) -> Result<()> {
        use std::io::Write as _;

        let live = self.plugin_live_dir(plugin_id)?;
        fs::create_dir_all(&live)
            .with_context(|| format!("creating plugin live dir {}", live.display()))?;
        let marker = live.join(CATALOG_AUTHORITY);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
        {
            Ok(mut file) => {
                file.write_all(b"signed-catalog-authority-v1\n")?;
                file.sync_all()?;
                fs::set_permissions(&marker, fs::Permissions::from_mode(0o444))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&marker)?;
                ensure!(
                    metadata.is_file() && !metadata.file_type().is_symlink(),
                    "plugin Catalog authority marker is not a regular file"
                );
                ensure!(
                    fs::read(&marker)? == b"signed-catalog-authority-v1\n",
                    "plugin Catalog authority marker is invalid"
                );
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    /// # Errors
    /// Returns when an existing marker is malformed or unsafe.
    pub fn has_catalog_authority(&self, plugin_id: &str) -> Result<bool> {
        let marker = self.plugin_live_dir(plugin_id)?.join(CATALOG_AUTHORITY);
        let metadata = match fs::symlink_metadata(&marker) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "plugin Catalog authority marker is not a regular file"
        );
        ensure!(
            fs::read(marker)? == b"signed-catalog-authority-v1\n",
            "plugin Catalog authority marker is invalid"
        );
        Ok(true)
    }

    /// # Errors
    /// Rejects unsafe or corrupt markers; a completed Catalog-only cutover is
    /// never silently undone by missing startup configuration.
    #[cfg(feature = "full")]
    pub fn catalog_only_required(&self) -> Result<bool> {
        use std::io::Read as _;
        use std::os::unix::fs::OpenOptionsExt as _;

        let file = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.root.join(CATALOG_ONLY))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error).context("opening Catalog-only authority marker"),
        };
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.len() == CATALOG_ONLY_CONTENT.len() as u64,
            "invalid Catalog-only authority marker"
        );
        let mut content = Vec::new();
        file.take(CATALOG_ONLY_CONTENT.len() as u64 + 1)
            .read_to_end(&mut content)?;
        ensure!(
            content == CATALOG_ONLY_CONTENT,
            "invalid Catalog-only authority marker"
        );
        Ok(true)
    }

    /// # Errors
    /// Returns if the one-way cutover receipt cannot be durably recorded.
    #[cfg(feature = "full")]
    pub fn record_catalog_only(&self) -> Result<()> {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;

        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o400)
            .open(self.root.join(CATALOG_ONLY))
        {
            Ok(mut file) => {
                file.write_all(CATALOG_ONLY_CONTENT)?;
                file.sync_all()?;
                fs::File::open(&self.root)?.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                ensure!(
                    self.catalog_only_required()?,
                    "Catalog-only authority marker disappeared"
                );
            }
            Err(error) => return Err(error).context("recording Catalog-only authority"),
        }
        Ok(())
    }

    /// Resolve the currently activated generation, if the link exists.
    ///
    /// # Errors
    /// Returns when the plugin id is invalid or the current link is dangling.
    #[cfg(test)]
    pub fn current_generation(&self, plugin_id: &str) -> Result<Option<PathBuf>> {
        let link = self.current_link(plugin_id)?;
        match fs::symlink_metadata(&link) {
            Ok(metadata) => ensure!(
                metadata.file_type().is_symlink(),
                "plugin current is not a symlink"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        let target = fs::canonicalize(&link)
            .with_context(|| format!("resolving plugin current {}", link.display()))?;
        ensure!(
            target.starts_with(self.plugin_live_dir(plugin_id)?.join(GENERATIONS)),
            "plugin current link escaped its generation root"
        );
        Ok(Some(target))
    }

    /// Write host data/runtime files into a content-addressed generation and optionally
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
        for relative in files.keys() {
            crate::plugin_host_bundle::validate_bundle_path(relative)?;
        }
        let generations = generation
            .parent()
            .context("plugin generation has no parent")?;
        fs::create_dir_all(generations).with_context(|| {
            format!("creating plugin generation root {}", generations.display())
        })?;
        match generation.symlink_metadata() {
            Ok(_) => verify_generation_files(&generation, files)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let staging = create_staging_generation(generations)?;
                let staged = (|| -> Result<()> {
                    for (relative, content) in files {
                        let path = staging.join(relative);
                        if let Some(parent) = path.parent() {
                            fs::create_dir_all(parent)?;
                        }
                        fs::write(&path, content)
                            .with_context(|| format!("writing plugin file {}", path.display()))?;
                    }
                    verify_generation_files(&staging, files)
                })();
                if let Err(error) = staged {
                    let _ = fs::remove_dir_all(&staging);
                    return Err(error);
                }
                if let Err(error) = fs::rename(&staging, &generation) {
                    let _ = fs::remove_dir_all(&staging);
                    match generation.symlink_metadata() {
                        Ok(_) => verify_generation_files(&generation, files)?,
                        Err(inspect_error)
                            if inspect_error.kind() == std::io::ErrorKind::NotFound =>
                        {
                            return Err(error).with_context(|| {
                                format!("installing plugin generation {}", generation.display())
                            });
                        }
                        Err(inspect_error) => {
                            return Err(inspect_error).with_context(|| {
                                format!("checking raced plugin generation {}", generation.display())
                            });
                        }
                    }
                }
            }
            Err(error) => return Err(error.into()),
        }
        lock_generation(&generation)?;
        if make_current {
            self.activate_host_generation(plugin_id, &generation)?;
        }
        Ok(generation)
    }

    /// Atomically point one host plugin at an already staged immutable
    /// generation.
    ///
    /// # Errors
    /// Returns when the generation is outside this plugin's generation root or
    /// the activation link cannot be replaced.
    pub fn activate_host_generation(&self, plugin_id: &str, generation: &Path) -> Result<()> {
        let live = self.plugin_live_dir(plugin_id)?;
        let generations = live.join(GENERATIONS);
        let metadata = generation
            .symlink_metadata()
            .with_context(|| format!("reading plugin host generation {}", generation.display()))?;
        ensure!(
            generation.parent() == Some(generations.as_path())
                && metadata.is_dir()
                && !metadata.file_type().is_symlink(),
            "plugin host generation is outside its generation root"
        );
        let name = generation
            .file_name()
            .context("plugin host generation has no directory name")?;
        let target = Path::new(GENERATIONS).join(name);
        let current = self.current_link(plugin_id)?;
        let pending = live.join(".current-next");
        if pending.symlink_metadata().is_ok() {
            fs::remove_file(&pending)
                .with_context(|| format!("removing stale plugin link {}", pending.display()))?;
        }
        symlink(&target, &pending).with_context(|| {
            format!(
                "linking pending plugin current {} -> {}",
                pending.display(),
                target.display()
            )
        })?;
        fs::rename(&pending, &current).with_context(|| {
            format!(
                "activating plugin current {} -> {}",
                current.display(),
                target.display()
            )
        })?;
        Ok(())
    }

    /// `SQLite` file owned by this plugin. Isolated from the controller store.
    ///
    /// # Errors
    /// Returns when the id is invalid or the state directory cannot be created.
    pub fn sqlite_path(&self, plugin_id: &str) -> Result<PathBuf> {
        Ok(self.ensure_state_dir(plugin_id)?.join("db.sqlite"))
    }
}

fn create_staging_generation(generations: &Path) -> Result<PathBuf> {
    static NEXT_STAGING: AtomicU64 = AtomicU64::new(1);
    loop {
        let sequence = NEXT_STAGING.fetch_add(1, Ordering::Relaxed);
        let staging = generations.join(format!(".staging-{}-{sequence}", std::process::id()));
        match fs::create_dir(&staging) {
            Ok(()) => return Ok(staging),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("creating plugin staging dir {}", staging.display()));
            }
        }
    }
}

fn verify_generation_files(
    generation: &Path,
    expected: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    let metadata = generation
        .symlink_metadata()
        .with_context(|| format!("reading plugin generation {}", generation.display()))?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "plugin generation must be a real directory"
    );
    let mut actual = std::collections::BTreeMap::new();
    collect_generation_files(generation, generation, &mut actual)?;
    ensure!(
        actual == *expected,
        "plugin generation contents do not match their content address"
    );
    Ok(())
}

fn collect_generation_files(
    root: &Path,
    directory: &Path,
    files: &mut std::collections::BTreeMap<String, String>,
) -> Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("reading plugin generation {}", directory.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "plugin generation contains a symlink"
        );
        if metadata.is_dir() {
            collect_generation_files(root, &path, files)?;
            continue;
        }
        ensure!(
            metadata.is_file(),
            "plugin generation contains a special file"
        );
        let relative = path
            .strip_prefix(root)
            .context("plugin generation file escaped its root")?
            .to_string_lossy()
            .replace('\\', "/");
        crate::plugin_host_bundle::validate_bundle_path(&relative)?;
        let content = fs::read_to_string(&path)
            .with_context(|| format!("reading plugin generation file {}", path.display()))?;
        ensure!(
            files.insert(relative, content).is_none(),
            "plugin generation contains duplicate files"
        );
    }
    Ok(())
}

fn lock_generation(generation: &Path) -> Result<()> {
    for entry in fs::read_dir(generation)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "plugin generation contains a symlink"
        );
        if metadata.is_dir() {
            lock_generation(&path)?;
        } else {
            ensure!(
                metadata.is_file(),
                "plugin generation contains a special file"
            );
            fs::set_permissions(&path, fs::Permissions::from_mode(0o444))?;
        }
    }
    fs::set_permissions(generation, fs::Permissions::from_mode(0o555))?;
    Ok(())
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
    #[cfg(feature = "full")]
    fn catalog_only_receipt_is_durable_idempotent_and_fails_closed() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-catalog-only-marker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dir = PluginDir::open(&root).unwrap();
        assert!(!dir.catalog_only_required().unwrap());
        dir.record_catalog_only().unwrap();
        dir.record_catalog_only().unwrap();
        assert!(
            PluginDir::open(&root)
                .unwrap()
                .catalog_only_required()
                .unwrap()
        );
        let marker = dir.root().join(CATALOG_ONLY);
        fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&marker, b"invalid").unwrap();
        assert!(dir.catalog_only_required().is_err());
        assert!(dir.record_catalog_only().is_err());
        fs::remove_file(&marker).unwrap();
        let target = root.join("target");
        fs::write(&target, CATALOG_ONLY_CONTENT).unwrap();
        symlink(&target, &marker).unwrap();
        assert!(dir.catalog_only_required().is_err());
        assert!(dir.record_catalog_only().is_err());
        fs::remove_dir_all(root).unwrap();
    }

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
        assert!(!dir.has_catalog_authority("google").unwrap());
        dir.record_catalog_authority("google").unwrap();
        dir.record_catalog_authority("google").unwrap();
        assert!(dir.has_catalog_authority("google").unwrap());
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
        files.insert("host.json".to_owned(), r#"{"schema_version":1}"#.to_owned());
        files.insert(
            "collector/index.js".to_owned(),
            "export function collect(){}".to_owned(),
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
        assert!(generation.join("collector").join("index.js").is_file());
        assert_eq!(
            fs::metadata(generation.join("collector/index.js"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o444
        );
        assert_eq!(
            fs::metadata(&generation).unwrap().permissions().mode() & 0o777,
            0o555
        );
        assert!(dir.current_generation("google").unwrap().is_some());
        let replacement = dir
            .install_host_files(
                "google",
                "1.0.1",
                "1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                &files,
                false,
            )
            .unwrap();
        dir.activate_host_generation("google", &replacement)
            .unwrap();
        assert_eq!(
            dir.current_generation("google").unwrap().unwrap(),
            fs::canonicalize(&replacement).unwrap()
        );
        assert!(dir.activate_host_generation("google", &root).is_err());
        files.insert("collector/../escape.js".to_owned(), "x".to_owned());
        assert!(
            dir.install_host_files(
                "google",
                "1.0.2",
                "2123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                &files,
                false,
            )
            .is_err()
        );
        unlock_tree(&root);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn content_addressed_generation_rejects_changed_or_extra_files() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-plugin-dir-integrity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dir = PluginDir::open(&root).unwrap();
        let files = std::collections::BTreeMap::from([
            ("host.json".to_owned(), r#"{"schema_version":1}"#.to_owned()),
            (
                "collector/index.js".to_owned(),
                "export function collect(){}".to_owned(),
            ),
        ]);
        let digest = "3123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let generation = dir
            .install_host_files("google", "1.0.0", digest, &files, false)
            .unwrap();
        let entry = generation.join("collector/index.js");
        fs::set_permissions(&entry, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(&entry, "tampered").unwrap();
        assert!(
            dir.install_host_files("google", "1.0.0", digest, &files, false)
                .is_err()
        );

        let extra_digest = "4123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let extra_generation = dir
            .install_host_files("google", "1.0.0", extra_digest, &files, false)
            .unwrap();
        fs::set_permissions(&extra_generation, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(extra_generation.join("extra.json"), "{}").unwrap();
        assert!(
            dir.install_host_files("google", "1.0.0", extra_digest, &files, false)
                .is_err()
        );

        unlock_tree(&root);
        fs::remove_dir_all(root).unwrap();
    }
}
