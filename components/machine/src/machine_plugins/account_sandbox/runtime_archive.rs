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
    /// A file of the proxy component (`agy-proxy`), relative to its content
    /// root.
    Proxy(&'static str),
    /// A fixed executable script.
    Script(&'static str),
    /// The daemon's `cowboy-runtime.json` manifest.
    Manifest,
}

/// Every daemon path besides the component trees, which are copied as-is:
/// the launcher's at the root and the proxy's below [`PROXY_ROOT`].
const LAYOUT: &[(&str, Source)] = &[
    ("bin/agy", Source::VendorCli),
    ("bin/agy-proxy", Source::Script(PROXY_ENTRY)),
    ("bin/agy-hook", Source::Script(HOOK_ENTRY)),
    ("policy/hooks.json", Source::Proxy("app/hooks.json")),
    ("policy/settings.json", Source::Proxy("app/settings.json")),
    ("cowboy-runtime.json", Source::Manifest),
];
/// The daemon starts `login`, `logout` and `probe` units with this command.
const LAUNCHER: &str = "bin/agy-launch";
/// The proxy component's own entry point, inside its tree.
const PROXY: &str = "bin/agy-proxy";
const PROXY_ROOT: &str = "proxy";
/// `runtime` units start `bin/agy-proxy`: the proxy component's launcher.
const PROXY_ENTRY: &str = "#!/bin/sh\nset -eu\ncase \"$0\" in */*) cowboy_dir=${0%/*} ;; *) cowboy_dir=. ;; esac\nexec \"$cowboy_dir/../proxy/bin/agy-proxy\" \"$@\"\n";
/// `policy/hooks.json` runs `agy-hook` from `PATH`, which the proxy sets to
/// this runtime's `bin`; it runs the proxy's `hook.mjs` on the bundled Node.
const HOOK_ENTRY: &str = "#!/bin/sh\nset -eu\ncase \"$0\" in */*) cowboy_dir=${0%/*} ;; *) cowboy_dir=. ;; esac\nexec \"$cowboy_dir/../proxy/runtime/node\" \"$cowboy_dir/../proxy/app/hook.mjs\"\n";
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

/// The resolved inputs of one archive: component executables.
pub(in crate::machine_plugins) struct Components {
    pub vendor_cli: PathBuf,
    /// The `agy-launch` launcher (login, logout and probe units).
    pub launcher: PathBuf,
    /// The `agy-proxy` Node proxy (runtime units).
    pub proxy: PathBuf,
}

impl Components {
    /// Select the `provider_cli` executable and the two `provider_adapter`
    /// executables by command.
    pub(in crate::machine_plugins) fn select(
        payload: &cowboy_provider_sdk::PlatformPayload,
        resolve: impl Fn(&str) -> Result<PathBuf>,
    ) -> Result<Self> {
        let command = |kind: PrivateComponentKind, name: Option<&str>| {
            let mut matching = payload.private_components.iter().filter(|component| {
                component.kind == kind && name.is_none_or(|name| component.command == name)
            });
            match (matching.next(), matching.next()) {
                (Some(component), None) => resolve(&component.command),
                _ => bail!(
                    "the account sandbox needs exactly one {} component{}",
                    kind.as_str(),
                    name.map(|name| format!(" {name}")).unwrap_or_default()
                ),
            }
        };
        Ok(Self {
            vendor_cli: command(PrivateComponentKind::ProviderCli, None)?,
            launcher: command(PrivateComponentKind::ProviderAdapter, Some("agy-launch"))?,
            proxy: command(PrivateComponentKind::ProviderAdapter, Some("agy-proxy"))?,
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

/// The content root of the component whose executable is `command`, which
/// must be `expected` inside it.
fn content_root<'a>(command: &'a Path, expected: &str) -> Result<&'a Path> {
    let root = command
        .parent()
        .and_then(Path::parent)
        .context("component has no content root")?;
    ensure!(
        command.strip_prefix(root).ok() == Some(Path::new(expected)),
        "the component must provide {expected}"
    );
    Ok(root)
}

/// The launcher's tree at the root, the proxy's below [`PROXY_ROOT`], then
/// [`LAYOUT`], sorted by path.
fn entries(components: &Components) -> Result<Vec<Entry>> {
    let launcher = content_root(&components.launcher, LAUNCHER)?;
    let proxy = content_root(&components.proxy, PROXY)?;
    let mut entries = Vec::new();
    adapter_tree(launcher, launcher, "", &mut entries)?;
    adapter_tree(proxy, proxy, PROXY_ROOT, &mut entries)?;
    for (path, source) in LAYOUT {
        ensure!(
            !entries.iter().any(|entry| entry.path == *path),
            "a component already provides {path}"
        );
        let (content, executable) = match source {
            Source::VendorCli => (Content::File(components.vendor_cli.clone()), true),
            Source::Proxy(relative) => {
                let file = proxy.join(relative);
                ensure!(
                    fs::symlink_metadata(&file)
                        .with_context(|| format!("the proxy component lacks {relative}"))?
                        .is_file(),
                    "proxy file {relative} is not a regular file"
                );
                (Content::File(file), false)
            }
            Source::Script(script) => (Content::Bytes(script.as_bytes()), true),
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
fn adapter_tree(
    root: &Path,
    directory: &Path,
    prefix: &str,
    entries: &mut Vec<Entry>,
) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            adapter_tree(root, &path, prefix, entries)?;
        } else if kind.is_file() {
            let relative = path.strip_prefix(root)?;
            let relative = relative
                .to_str()
                .context("component file name is not UTF-8")?;
            let name = if prefix.is_empty() {
                relative.to_owned()
            } else {
                format!("{prefix}/{relative}")
            };
            entries.push(Entry {
                path: name,
                executable: entry.metadata()?.permissions().mode() & 0o111 != 0,
                content: Content::File(path),
            });
        } else {
            bail!("a component contains a link or special file");
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

    fn executable(path: &Path, content: &[u8]) {
        fs::write(path, content).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn fixture(root: &Path) -> Components {
        let cli = root.join("runtime/provider_cli-antigravity/content");
        let launcher = root.join("runtime/provider_adapter-antigravity/content");
        let proxy = root.join("runtime/provider_adapter-antigravity-proxy/content");
        for directory in [
            cli.clone(),
            launcher.join("bin"),
            launcher.join("share"),
            proxy.join("bin"),
            proxy.join("runtime"),
            proxy.join("app/node_modules/ws"),
        ] {
            fs::create_dir_all(directory).unwrap();
        }
        fs::write(cli.join("antigravity"), b"vendor").unwrap();
        executable(&launcher.join("bin/agy-launch"), b"#!/bin/sh\n");
        fs::write(launcher.join("share/hooks.json"), b"{}").unwrap();
        executable(&proxy.join("bin/agy-proxy"), b"#!/bin/sh\n# node wrapper\n");
        executable(&proxy.join("runtime/node"), b"node");
        fs::write(proxy.join("app/cowboy-launch.mjs"), b"// proxy").unwrap();
        fs::write(proxy.join("app/hook.mjs"), b"// hook").unwrap();
        fs::write(proxy.join("app/hooks.json"), b"{\"gate\":{}}").unwrap();
        fs::write(proxy.join("app/settings.json"), b"{\"permissions\":{}}").unwrap();
        fs::write(proxy.join("app/node_modules/ws/index.js"), b"ws").unwrap();
        fs::write(proxy.join("cowboy-runtime.json"), b"{\"schema_version\":1}").unwrap();
        Components {
            vendor_cli: cli.join("antigravity"),
            launcher: launcher.join("bin/agy-launch"),
            proxy: proxy.join("bin/agy-proxy"),
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
                "bin/agy-hook",
                "bin/agy-launch",
                "bin/agy-proxy",
                "cowboy-runtime.json",
                "policy/hooks.json",
                "policy/settings.json",
                "proxy/app/cowboy-launch.mjs",
                "proxy/app/hook.mjs",
                "proxy/app/hooks.json",
                "proxy/app/node_modules/ws/index.js",
                "proxy/app/settings.json",
                "proxy/bin/agy-proxy",
                "proxy/cowboy-runtime.json",
                "proxy/runtime/node",
                "share/hooks.json",
            ]
        );
        let entry = |name: &str| entries.iter().find(|(path, _, _)| path == name).unwrap();
        assert_eq!(entry("bin/agy").2, b"vendor");
        assert_eq!(entry("bin/agy").1, 0o555);
        assert_eq!(entry("bin/agy-proxy").1, 0o555);
        assert_eq!(entry("bin/agy-proxy").2, PROXY_ENTRY.as_bytes());
        assert_eq!(entry("bin/agy-hook").1, 0o555);
        assert_eq!(entry("proxy/runtime/node").1, 0o555);
        assert_eq!(entry("policy/hooks.json").2, b"{\"gate\":{}}");
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

    /// The trampolines resolve the proxy tree from their own location.
    #[test]
    fn entry_scripts_run_the_proxy_tree() {
        let root = tempfile::tempdir().unwrap();
        let unpacked = root.path().join("rt");
        fs::create_dir_all(unpacked.join("bin")).unwrap();
        fs::create_dir_all(unpacked.join("proxy/bin")).unwrap();
        fs::create_dir_all(unpacked.join("proxy/runtime")).unwrap();
        executable(&unpacked.join("bin/agy-proxy"), PROXY_ENTRY.as_bytes());
        executable(&unpacked.join("bin/agy-hook"), HOOK_ENTRY.as_bytes());
        executable(
            &unpacked.join("proxy/bin/agy-proxy"),
            b"#!/bin/sh\necho proxy \"$@\"\n",
        );
        executable(
            &unpacked.join("proxy/runtime/node"),
            b"#!/bin/sh\necho node \"$1\"\n",
        );
        let run = |command: &str| {
            let output = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(command)
                .env(
                    "PATH",
                    format!("{}:/usr/bin:/bin", unpacked.join("bin").display()),
                )
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap()
        };
        assert_eq!(run("agy-proxy a b"), "proxy a b\n");
        let hook = run("agy-hook");
        assert!(hook.starts_with("node "), "{hook}");
        assert!(
            hook.trim_end().ends_with("/bin/../proxy/app/hook.mjs"),
            "{hook}"
        );
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
    fn proxy_without_policy_or_with_links_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let components = fixture(root.path());
        let app = components
            .proxy
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("app");
        fs::remove_file(app.join("settings.json")).unwrap();
        assert!(build(&components, tempfile::tempdir().unwrap().path()).is_err());
        fs::write(app.join("settings.json"), b"{}").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", app.join("link")).unwrap();
        assert!(build(&components, tempfile::tempdir().unwrap().path()).is_err());
    }

    #[test]
    fn components_are_selected_by_command() {
        let payload: cowboy_provider_sdk::PlatformPayload = serde_json::from_value(serde_json::json!({
            "os": "linux",
            "architecture": "x86_64",
            "payload_digest": "",
            "launch_command": "agy-launch",
            "private_components": [
                {"kind": "provider_cli", "slot": "antigravity", "dependency": "cli", "command": "antigravity"},
                {"kind": "provider_adapter", "slot": "antigravity", "dependency": "launch", "command": "agy-launch"},
                {"kind": "provider_adapter", "slot": "antigravity-proxy", "dependency": "acp", "command": "agy-proxy"},
            ],
        }))
        .unwrap();
        let selected =
            Components::select(&payload, |command| Ok(PathBuf::from("/c").join(command))).unwrap();
        assert_eq!(selected.vendor_cli, Path::new("/c/antigravity"));
        assert_eq!(selected.launcher, Path::new("/c/agy-launch"));
        assert_eq!(selected.proxy, Path::new("/c/agy-proxy"));
        let mut without = payload.clone();
        without.private_components.pop();
        assert!(Components::select(&without, |command| Ok(PathBuf::from(command))).is_err());
    }
}
