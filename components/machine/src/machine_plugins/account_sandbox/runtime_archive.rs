//! The runtime archive the account-sandbox daemon unpacks for one installed
//! Provider generation (`packaging/account-sandbox/README.md`, "Runtime
//! archives"). [`LAYOUT`] is the single mapping from the generation's
//! components to the daemon's fixed paths. The archive is deterministic, so
//! its SHA-256 identifies the generation's runtime; it is built and
//! registered once per generation digest and cached beside the generation.

use std::fs;
use std::io::{Read as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};
use cowboy_provider_sdk::PrivateComponentKind;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Where an archive entry comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// The vendor executable of the `provider_cli` component.
    VendorCli,
    /// A file of the `provider_adapter` (launcher) component, relative to
    /// its content root.
    Adapter(&'static str),
    /// The daemon's `cowboy-runtime.json` manifest.
    Manifest,
}

/// Every daemon path besides the adapter's own tree, which is copied as-is.
const LAYOUT: &[(&str, Source)] = &[
    ("bin/agy", Source::VendorCli),
    // The launcher stands in for the Node proxy until it exists (step 4).
    ("bin/agy-proxy", Source::Adapter("bin/agy-launch")),
    ("policy/hooks.json", Source::Adapter("share/hooks.json")),
    (
        "policy/settings.json",
        Source::Adapter("share/settings.json"),
    ),
    ("cowboy-runtime.json", Source::Manifest),
];
/// The daemon starts `login`, `logout` and `probe` units with this command.
const LAUNCHER: &str = "bin/agy-launch";
const MANIFEST: &str =
    r#"{"agy":"bin/agy","hooks":"policy/hooks.json","settings":"policy/settings.json"}"#;
/// The daemon's default `max_runtime_archive_bytes`.
const MAX_ARCHIVE_BYTES: u64 = 1 << 30;
const ARCHIVE_NAME: &str = "account-sandbox-runtime.tar";
const RECORD_NAME: &str = "account-sandbox-runtime.json";

/// A generation's runtime archive and the digest the daemon registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::machine_plugins) struct Archive {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    sha256: String,
    size: u64,
}

/// The archive of `generation` the daemon already registered, if its record
/// and bytes are still present and unchanged in size.
pub(in crate::machine_plugins) fn cached(generation: &Path) -> Result<Option<Archive>> {
    let bytes = match fs::read(generation.join(RECORD_NAME)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        result => result?,
    };
    let record: Record = serde_json::from_slice(&bytes).context("invalid runtime record")?;
    ensure!(record.schema == 1, "unsupported runtime record");
    let path = generation.join(ARCHIVE_NAME);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && metadata.len() == record.size => Ok(Some(Archive {
            path,
            sha256: record.sha256,
        })),
        _ => Ok(None),
    }
}

/// Record that the daemon registered `archive`.
pub(in crate::machine_plugins) fn record_registered(
    generation: &Path,
    archive: &Archive,
) -> Result<()> {
    let record = Record {
        schema: 1,
        sha256: archive.sha256.clone(),
        size: fs::metadata(&archive.path)?.len(),
    };
    super::super::atomic_write(
        &generation.join(RECORD_NAME),
        &serde_json::to_vec(&record)?,
        0o600,
    )
}

/// The resolved inputs of one archive: component executables by kind.
pub(in crate::machine_plugins) struct Components {
    pub vendor_cli: PathBuf,
    pub adapter: PathBuf,
}

impl Components {
    /// Select the `provider_cli` and `provider_adapter` executables.
    pub(in crate::machine_plugins) fn select(
        payload: &cowboy_provider_sdk::PlatformPayload,
        resolve: impl Fn(&str) -> Result<PathBuf>,
    ) -> Result<Self> {
        let command = |kind: PrivateComponentKind| {
            let mut matching = payload
                .private_components
                .iter()
                .filter(|component| component.kind == kind);
            match (matching.next(), matching.next()) {
                (Some(component), None) => resolve(&component.command),
                _ => bail!(
                    "the account sandbox needs exactly one {} component",
                    kind.as_str()
                ),
            }
        };
        Ok(Self {
            vendor_cli: command(PrivateComponentKind::ProviderCli)?,
            adapter: command(PrivateComponentKind::ProviderAdapter)?,
        })
    }
}

/// Build the archive of `components` into `generation` and return it.
pub(in crate::machine_plugins) fn build(
    components: &Components,
    generation: &Path,
) -> Result<Archive> {
    let entries = entries(components)?;
    let temporary = generation.join(format!(".{ARCHIVE_NAME}.{}", std::process::id()));
    let result = write(&entries, &temporary).and_then(|sha256| {
        let path = generation.join(ARCHIVE_NAME);
        fs::rename(&temporary, &path)?;
        Ok(Archive { path, sha256 })
    });
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// One archive entry: its path, executable bit and content.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    path: String,
    executable: bool,
    content: Content,
}

#[derive(Debug, PartialEq, Eq)]
enum Content {
    File(PathBuf),
    Bytes(&'static [u8]),
}

/// The adapter's tree as-is, then [`LAYOUT`], sorted by path.
fn entries(components: &Components) -> Result<Vec<Entry>> {
    let bin = components
        .adapter
        .parent()
        .context("launcher has no directory")?;
    let root = bin.parent().context("launcher has no content root")?;
    ensure!(
        components.adapter.strip_prefix(root).ok() == Some(Path::new(LAUNCHER)),
        "the launcher component must provide {LAUNCHER}"
    );
    let mut entries = Vec::new();
    adapter_tree(root, root, &mut entries)?;
    for (path, source) in LAYOUT {
        ensure!(
            !entries.iter().any(|entry| entry.path == *path),
            "the launcher component already provides {path}"
        );
        let (content, executable) = match source {
            Source::VendorCli => (Content::File(components.vendor_cli.clone()), true),
            Source::Adapter(relative) => {
                let file = root.join(relative);
                ensure!(
                    fs::symlink_metadata(&file)
                        .with_context(|| format!("the launcher component lacks {relative}"))?
                        .is_file(),
                    "launcher file {relative} is not a regular file"
                );
                let executable = fs::metadata(&file)?.permissions().mode() & 0o111 != 0;
                (Content::File(file), executable)
            }
            Source::Manifest => (Content::Bytes(MANIFEST.as_bytes()), false),
        };
        entries.push(Entry {
            path: (*path).to_owned(),
            executable,
            content,
        });
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

/// Every regular file under `directory`; links and special files refuse.
fn adapter_tree(root: &Path, directory: &Path, entries: &mut Vec<Entry>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            adapter_tree(root, &path, entries)?;
        } else if kind.is_file() {
            let relative = path.strip_prefix(root)?;
            let name = relative
                .to_str()
                .context("launcher file name is not UTF-8")?
                .to_owned();
            entries.push(Entry {
                path: name,
                executable: entry.metadata()?.permissions().mode() & 0o111 != 0,
                content: Content::File(path),
            });
        } else {
            bail!("launcher component contains a link or special file");
        }
    }
    Ok(())
}

/// Write a deterministic ustar archive and return its SHA-256.
fn write(entries: &[Entry], destination: &Path) -> Result<String> {
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .with_context(|| format!("creating {}", destination.display()))?;
    let mut writer = Hashing {
        inner: std::io::BufWriter::new(file),
        hasher: Sha256::new(),
        written: 0,
    };
    for entry in entries {
        let mut header = tar::Header::new_ustar();
        header.set_path(&entry.path)?;
        header.set_mode(if entry.executable { 0o555 } else { 0o444 });
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_entry_type(tar::EntryType::Regular);
        match &entry.content {
            Content::Bytes(bytes) => {
                header.set_size(bytes.len() as u64);
                header.set_cksum();
                writer.write_all(header.as_bytes())?;
                writer.write_all(bytes)?;
                pad(&mut writer, bytes.len() as u64)?;
            }
            Content::File(path) => {
                let mut source =
                    fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
                let size = source.metadata()?.len();
                header.set_size(size);
                header.set_cksum();
                writer.write_all(header.as_bytes())?;
                let copied = std::io::copy(&mut (&mut source).take(size), &mut writer)?;
                ensure!(copied == size, "{} changed while archiving", path.display());
                pad(&mut writer, size)?;
            }
        }
        ensure!(
            writer.written <= MAX_ARCHIVE_BYTES,
            "runtime archive exceeds {MAX_ARCHIVE_BYTES} bytes"
        );
    }
    writer.write_all(&[0; 1024])?;
    writer.inner.flush()?;
    writer.inner.get_ref().sync_all()?;
    Ok(format!("{:x}", writer.hasher.finalize()))
}

fn pad(writer: &mut impl std::io::Write, size: u64) -> Result<()> {
    let remainder = (size % 512) as usize;
    if remainder != 0 {
        writer.write_all(&[0; 512][..512 - remainder])?;
    }
    Ok(())
}

struct Hashing<W> {
    inner: W,
    hasher: Sha256,
    written: u64,
}

impl<W: std::io::Write> std::io::Write for Hashing<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(bytes)?;
        self.hasher.update(&bytes[..written]);
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path) -> Components {
        let cli = root.join("runtime/provider_cli-antigravity/content");
        let adapter = root.join("runtime/provider_adapter-antigravity/content");
        for directory in [cli.clone(), adapter.join("bin"), adapter.join("share")] {
            fs::create_dir_all(directory).unwrap();
        }
        fs::write(cli.join("antigravity"), b"vendor").unwrap();
        fs::write(adapter.join("bin/agy-launch"), b"#!/bin/sh\n").unwrap();
        fs::set_permissions(
            adapter.join("bin/agy-launch"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        fs::write(adapter.join("share/hooks.json"), b"{}").unwrap();
        fs::write(adapter.join("share/settings.json"), b"{\"permissions\":{}}").unwrap();
        Components {
            vendor_cli: cli.join("antigravity"),
            adapter: adapter.join("bin/agy-launch"),
        }
    }

    /// Parse the archive back: path, mode and content of every entry.
    fn read(path: &Path) -> Vec<(String, u32, Vec<u8>)> {
        let mut archive = tar::Archive::new(fs::File::open(path).unwrap());
        archive
            .entries()
            .unwrap()
            .map(|entry| {
                let mut entry = entry.unwrap();
                let header = entry.header().clone();
                assert_eq!(&header.as_bytes()[257..263], b"ustar\0");
                let mut content = Vec::new();
                entry.read_to_end(&mut content).unwrap();
                (
                    header.path().unwrap().display().to_string(),
                    header.mode().unwrap(),
                    content,
                )
            })
            .collect()
    }

    #[test]
    fn archive_maps_components_to_the_daemon_layout_deterministically() {
        let root = tempfile::tempdir().unwrap();
        let components = fixture(root.path());
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let archive = build(&components, first.path()).unwrap();
        let again = build(&components, second.path()).unwrap();
        assert_eq!(archive.sha256, again.sha256);
        assert_eq!(
            fs::read(&archive.path).unwrap(),
            fs::read(&again.path).unwrap()
        );
        let entries = read(&archive.path);
        let names: Vec<&str> = entries.iter().map(|(name, _, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "bin/agy",
                "bin/agy-launch",
                "bin/agy-proxy",
                "cowboy-runtime.json",
                "policy/hooks.json",
                "policy/settings.json",
                "share/hooks.json",
                "share/settings.json",
            ]
        );
        let entry = |name: &str| entries.iter().find(|(path, _, _)| path == name).unwrap();
        assert_eq!(entry("bin/agy").2, b"vendor");
        assert_eq!(entry("bin/agy").1, 0o555);
        assert_eq!(entry("bin/agy-proxy").2, b"#!/bin/sh\n");
        assert_eq!(entry("policy/settings.json").1, 0o444);
        let manifest: serde_json::Value =
            serde_json::from_slice(&entry("cowboy-runtime.json").2).unwrap();
        assert_eq!(
            manifest,
            serde_json::json!({
                "agy": "bin/agy",
                "hooks": "policy/hooks.json",
                "settings": "policy/settings.json",
            })
        );
        let digest = format!("{:x}", Sha256::digest(fs::read(&archive.path).unwrap()));
        assert_eq!(digest, archive.sha256);
    }

    #[test]
    fn registration_is_cached_per_generation() {
        let root = tempfile::tempdir().unwrap();
        let components = fixture(root.path());
        let generation = tempfile::tempdir().unwrap();
        assert!(cached(generation.path()).unwrap().is_none());
        let archive = build(&components, generation.path()).unwrap();
        assert!(cached(generation.path()).unwrap().is_none());
        record_registered(generation.path(), &archive).unwrap();
        assert_eq!(cached(generation.path()).unwrap(), Some(archive.clone()));
        fs::write(&archive.path, b"truncated").unwrap();
        assert!(cached(generation.path()).unwrap().is_none());
    }

    #[test]
    fn launcher_without_policy_or_with_links_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let components = fixture(root.path());
        let share = components
            .adapter
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("share");
        fs::remove_file(share.join("settings.json")).unwrap();
        assert!(build(&components, tempfile::tempdir().unwrap().path()).is_err());
        fs::write(share.join("settings.json"), b"{}").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", share.join("link")).unwrap();
        assert!(build(&components, tempfile::tempdir().unwrap().path()).is_err());
    }
}
