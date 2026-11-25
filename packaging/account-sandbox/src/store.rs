//! The runtime store (`<state_root>/runtime/<sha256>/`): generation runtimes
//! received as bytes over the socket, verified by SHA-256, unpacked with the
//! strict ustar reader, checked against the root-held `agy` SHA-512
//! allow-list and kept root-owned and read-only. The store is bounded; a
//! runtime referenced by a live unit is never evicted.

use std::collections::HashSet;
use std::io::{self, Read, Write as _};
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
use std::path::Path;
use std::sync::RwLock;

use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;
use sha2::{Digest as _, Sha256, Sha512};

use crate::config::{Config, unit_safe};
use crate::protocol::{Code, Outcome, fail};
use crate::{safefs, tar};

pub const STORE_DIR: &str = "runtime";
/// The manifest every runtime archive carries at its root.
pub const MANIFEST: &str = "cowboy-runtime.json";
const MAX_MANIFEST_BYTES: u64 = 4096;
const MAX_ALLOW_LIST_BYTES: u64 = 64 * 1024;

/// `cowboy-runtime.json`: archive-relative paths of the vendor executable and
/// the two policy files the daemon binds read-only over the account home.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub agy: String,
    pub hooks: String,
    pub settings: String,
}

impl Manifest {
    fn validate(&self) -> Result<()> {
        for path in [&self.agy, &self.hooks, &self.settings] {
            ensure!(
                unit_safe(path)
                    && !path.starts_with('/')
                    && path
                        .split('/')
                        .all(|part| !part.is_empty() && part != "." && part != ".."),
                "unsafe manifest path {path:?}"
            );
        }
        Ok(())
    }
}

/// Open (creating) the store directory beneath the state root.
pub fn open_store(root: BorrowedFd<'_>) -> Result<OwnedFd> {
    safefs::ensure_dir(root, STORE_DIR, 0o711, crate::daemon::root_owner())
}

/// Remove leftovers of interrupted registrations.
pub fn sweep(store: BorrowedFd<'_>) -> Result<()> {
    for name in safefs::list(store)? {
        if name.starts_with(".incoming-") || name.starts_with(".unpack-") {
            safefs::remove_tree(store, &name)?;
        }
    }
    Ok(())
}

/// Digests currently in the store.
pub fn digests(store: BorrowedFd<'_>) -> Result<Vec<String>> {
    Ok(safefs::list(store)?
        .into_iter()
        .filter(|name| name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect())
}

/// Read and validate a stored runtime's manifest and re-check its `agy`
/// against the current allow-list.
pub fn load(store: BorrowedFd<'_>, sha256: &str, allow_list: &Path) -> Outcome<Manifest> {
    if !safefs::exists(store, sha256)? {
        return Err(fail(Code::NotFound, "runtime is not registered"));
    }
    let entry = safefs::open_dir(store, sha256, safefs::STRICT)?;
    verify_tree(entry.as_fd(), allow_list)
}

/// Whether runtime `sha256` is registered and passes verification.
fn intact(store: BorrowedFd<'_>, sha256: &str, allow_list: &Path) -> Outcome<bool> {
    if !safefs::exists(store, sha256)? {
        return Ok(false);
    }
    let entry = safefs::open_dir(store, sha256, safefs::STRICT)?;
    match verify_tree(entry.as_fd(), allow_list) {
        Ok(_) => Ok(true),
        Err(error) if error.code == Code::RuntimeRejected => Ok(false),
        Err(error) => Err(error),
    }
}

fn verify_tree(entry: BorrowedFd<'_>, allow_list: &Path) -> Outcome<Manifest> {
    let rejected = |message: String| fail(Code::RuntimeRejected, message);
    let bytes = safefs::read_file(entry, MANIFEST, MAX_MANIFEST_BYTES)
        .map_err(|error| rejected(format!("{error:#}")))?
        .ok_or_else(|| rejected(format!("{MANIFEST} is missing")))?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|error| rejected(format!("{MANIFEST}: {error}")))?;
    manifest
        .validate()
        .map_err(|error| rejected(format!("{error:#}")))?;
    for path in [&manifest.hooks, &manifest.settings] {
        safefs::open_regular(entry, path)
            .map_err(|error| rejected(format!("{error:#}")))?
            .ok_or_else(|| rejected(format!("{path} is missing")))?;
    }
    let (mut agy, mode) = safefs::open_regular(entry, &manifest.agy)
        .map_err(|error| rejected(format!("{error:#}")))?
        .ok_or_else(|| rejected("the agy executable is missing".to_owned()))?;
    if mode & 0o111 == 0 {
        return Err(rejected("the agy executable is not executable".to_owned()));
    }
    let mut hasher = Sha512::new();
    io::copy(&mut agy, &mut hasher).context("hash agy")?;
    let digest = hex::encode(hasher.finalize());
    if !read_allow_list(allow_list)?.contains(&digest) {
        return Err(rejected(format!(
            "agy SHA-512 {digest} is not allow-listed"
        )));
    }
    Ok(manifest)
}

/// Parse the allow-list: one lowercase hex SHA-512 per line; blank lines and
/// `#` comments are ignored; any other line invalidates the whole list.
pub fn read_allow_list(path: &Path) -> Result<HashSet<String>> {
    let metadata = std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    ensure!(
        metadata.len() <= MAX_ALLOW_LIST_BYTES,
        "allow-list is too large"
    );
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    parse_allow_list(&text)
}

pub fn parse_allow_list(text: &str) -> Result<HashSet<String>> {
    let mut digests = HashSet::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        crate::ids::validate_hex("allow-list entry", line, 128)?;
        digests.insert(line.to_owned());
    }
    Ok(digests)
}

/// Digests that must not be evicted, computed under the store's write lock.
pub type LiveDigests<'a> = &'a dyn Fn() -> Outcome<HashSet<String>>;

/// Receive exactly `size` bytes from `reader` and register them as runtime
/// `sha256`. Eviction and installation run under `lock`'s write side, with
/// `live` naming digests that must be kept. Returns whether the runtime was
/// already present.
pub fn register(
    store: BorrowedFd<'_>,
    config: &Config,
    sha256: &str,
    size: u64,
    reader: &mut impl Read,
    lock: &RwLock<()>,
    live: LiveDigests<'_>,
) -> Outcome<bool> {
    let nonce = safefs::nonce(8)?;
    let incoming = format!(".incoming-{nonce}");
    let unpacked = format!(".unpack-{nonce}");
    let result = receive_and_install(
        store, config, sha256, size, reader, lock, live, &incoming, &unpacked,
    );
    let _ = safefs::remove_tree(store, &incoming);
    let _ = safefs::remove_tree(store, &unpacked);
    result
}

#[allow(clippy::too_many_arguments)]
fn receive_and_install(
    store: BorrowedFd<'_>,
    config: &Config,
    sha256: &str,
    size: u64,
    reader: &mut impl Read,
    lock: &RwLock<()>,
    live: LiveDigests<'_>,
    incoming: &str,
    unpacked: &str,
) -> Outcome<bool> {
    let mut file = safefs::create_new(store, incoming, 0o600)?;
    let mut hasher = Sha256::new();
    let mut remaining = size;
    let mut buffer = vec![0_u8; 64 * 1024];
    while remaining > 0 {
        let want = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = reader.read(&mut buffer[..want]).context("read archive")?;
        if read == 0 {
            return Err(fail(
                Code::InvalidRequest,
                "archive shorter than its declared size",
            ));
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])?;
        remaining -= read as u64;
    }
    if hex::encode(hasher.finalize()) != sha256 {
        return Err(fail(Code::RuntimeRejected, "archive SHA-256 mismatch"));
    }
    if intact(store, sha256, &config.agy_allow_list)? {
        return Ok(true);
    }
    file.sync_all()?;
    let (mut archive, _) =
        safefs::open_regular(store, incoming)?.context("incoming archive vanished")?;
    drop(file);
    let target = safefs::ensure_dir(store, unpacked, 0o755, None)?;
    let limits = tar::Limits {
        max_entries: config.max_runtime_entries,
        max_total_bytes: config.max_runtime_archive_bytes,
    };
    tar::unpack(&mut archive, target.as_fd(), limits)
        .map_err(|error| fail(Code::RuntimeRejected, format!("{error:#}")))?;
    let manifest = verify_tree(target.as_fd(), &config.agy_allow_list)?;
    tar::sync_dirs(target.as_fd()).context("fsync runtime")?;
    let _guard = lock
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if intact(store, sha256, &config.agy_allow_list)? {
        return Ok(true);
    }
    if safefs::exists(store, sha256)? {
        // A damaged entry (a power loss before its tree was durable) is
        // replaced, unless a live unit still uses it.
        if live()?.contains(sha256) {
            return Err(fail(
                Code::RuntimeRejected,
                "the registered runtime is damaged and still in use",
            ));
        }
        safefs::remove_tree(store, sha256)?;
    }
    // Reuse identical immutable large binaries (`agy` and bundled `node`) from
    // an already-verified root-owned sibling under the store write lock. The
    // last runtime unlink reclaims the shared inode with no separate blob GC.
    // Pre-rename mismatches or link failures leave the unpacked tree untouched,
    // while a post-rename directory fsync failure MUST fail registration so a
    // non-durable replacement never evicts siblings or acknowledges success.
    deduplicate_from_siblings(store, target.as_fd(), &manifest)
        .context("deduplicate runtime binaries")?;
    drop(target);
    evict(store, config.max_runtimes.saturating_sub(1), &live()?)?;
    rustix::fs::renameat(store, unpacked, store, sha256).context("install runtime")?;
    rustix::fs::fsync(store).context("fsync store")?;
    Ok(false)
}

/// Relink exact byte-and-mode matches of the large runtime binaries from any
/// already-installed sibling runtime under the store write lock.
fn deduplicate_from_siblings(
    store: BorrowedFd<'_>,
    unpacked: BorrowedFd<'_>,
    manifest: &Manifest,
) -> Result<()> {
    let Ok(siblings) = digests(store) else {
        return Ok(());
    };
    if siblings.is_empty() {
        return Ok(());
    }
    let mut candidates = vec![manifest.agy.as_str()];
    if manifest.agy != "proxy/runtime/node" {
        candidates.push("proxy/runtime/node");
    }
    for rel_path in candidates {
        deduplicate_path(store, unpacked, &siblings, rel_path)?;
    }
    Ok(())
}

fn sha256_file(file: &mut std::fs::File) -> Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    io::copy(file, &mut hasher).context("hash runtime file")?;
    Ok(hasher.finalize().into())
}

fn open_parent_dir<'a>(
    root: BorrowedFd<'_>,
    rel_path: &'a str,
) -> Result<(Option<OwnedFd>, &'a str)> {
    match rel_path.rsplit_once('/') {
        Some((parent, name)) => Ok((Some(safefs::open_dir(root, parent, safefs::STRICT)?), name)),
        None => Ok((None, rel_path)),
    }
}

/// Create a fresh hardlink to `sibling_name` under `target_parent`, retrying
/// `EEXIST` with a fresh name and never unlinking a colliding pre-existing entry.
fn link_unique_temp(
    sibling_parent: BorrowedFd<'_>,
    sibling_name: &str,
    target_parent: BorrowedFd<'_>,
) -> Result<String> {
    for _ in 0..8 {
        let temp = format!(".dedup-{}", safefs::nonce(8)?);
        match rustix::fs::linkat(
            sibling_parent,
            sibling_name,
            target_parent,
            &temp,
            rustix::fs::AtFlags::empty(),
        ) {
            Ok(()) => return Ok(temp),
            Err(rustix::io::Errno::EXIST) => {}
            Err(error) => return Err(error).context("hardlink sibling runtime file"),
        }
    }
    anyhow::bail!("could not allocate unique deduplication link name")
}

fn deduplicate_path(
    store: BorrowedFd<'_>,
    unpacked: BorrowedFd<'_>,
    siblings: &[String],
    rel_path: &str,
) -> Result<()> {
    let Ok(Some((mut target_file, target_mode))) = safefs::open_regular(unpacked, rel_path) else {
        return Ok(());
    };
    let Ok(target_stat) = rustix::fs::fstat(&target_file) else {
        return Ok(());
    };
    let expected_uid = rustix::process::geteuid().as_raw();
    if target_stat.st_size == 0 || target_stat.st_uid != expected_uid {
        return Ok(());
    }
    let Ok(target_digest) = sha256_file(&mut target_file) else {
        return Ok(());
    };
    drop(target_file);

    for sibling_digest in siblings {
        let Ok(sibling_dir) = safefs::open_dir(store, sibling_digest, safefs::STRICT) else {
            continue;
        };
        let Ok(Some((mut sibling_file, sibling_mode))) =
            safefs::open_regular(sibling_dir.as_fd(), rel_path)
        else {
            continue;
        };
        let Ok(sibling_stat) = rustix::fs::fstat(&sibling_file) else {
            continue;
        };
        if sibling_stat.st_uid != expected_uid
            || sibling_stat.st_size != target_stat.st_size
            || (sibling_mode & 0o7777) != (target_mode & 0o7777)
            || (sibling_stat.st_dev, sibling_stat.st_ino)
                == (target_stat.st_dev, target_stat.st_ino)
        {
            continue;
        }
        let Ok(candidate_digest) = sha256_file(&mut sibling_file) else {
            continue;
        };
        if candidate_digest != target_digest {
            continue;
        }
        let Ok((target_parent_owned, file_name)) = open_parent_dir(unpacked, rel_path) else {
            return Ok(());
        };
        let Ok((sibling_parent_owned, sibling_name)) =
            open_parent_dir(sibling_dir.as_fd(), rel_path)
        else {
            continue;
        };
        let target_parent = target_parent_owned
            .as_ref()
            .map_or(unpacked, |fd| fd.as_fd());
        let sibling_parent = sibling_parent_owned
            .as_ref()
            .map_or(sibling_dir.as_fd(), |fd| fd.as_fd());
        let Ok(temp) = link_unique_temp(sibling_parent, sibling_name, target_parent) else {
            continue;
        };
        // Only unlink `temp` because `linkat` created it; disarm once `renameat`
        // moves it into `file_name`.
        let renamed = (|| -> Result<()> {
            let linked_stat =
                rustix::fs::statat(target_parent, &temp, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
                    .context("stat linked runtime file")?;
            ensure!(
                (linked_stat.st_dev, linked_stat.st_ino)
                    == (sibling_stat.st_dev, sibling_stat.st_ino),
                "sibling runtime file inode changed during link"
            );
            rustix::fs::renameat(target_parent, &temp, target_parent, file_name)
                .context("replace unpacked file with sibling hardlink")?;
            Ok(())
        })();
        if renamed.is_err() {
            let _ = rustix::fs::unlinkat(target_parent, &temp, rustix::fs::AtFlags::empty());
            continue;
        }
        // `renameat` replaced the unpacked entry; fsync must succeed before
        // registration can evict siblings or report durability.
        rustix::fs::fsync(target_parent).context("fsync deduplicated directory")?;
        return Ok(());
    }
    Ok(())
}

/// Evict the oldest runtimes not in `live` until at most `keep` remain.
fn evict(store: BorrowedFd<'_>, keep: usize, live: &HashSet<String>) -> Outcome<()> {
    let mut present: Vec<(i64, String)> = Vec::new();
    for digest in digests(store)? {
        let stat = rustix::fs::statat(
            store,
            digest.as_str(),
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        )
        .context("stat runtime")?;
        present.push((stat.st_mtime, digest));
    }
    present.sort();
    let mut count = present.len();
    for (_, digest) in present {
        if count <= keep {
            break;
        }
        if !live.contains(&digest) {
            safefs::remove_tree(store, &digest)?;
            count -= 1;
        }
    }
    if count > keep {
        return Err(fail(
            Code::Limit,
            "runtime store is full of runtimes in use",
        ));
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub const AGY: &[u8] = b"#!/bin/sh\necho agy\n";

    pub fn agy_sha512() -> String {
        hex::encode(Sha512::digest(AGY))
    }

    pub fn runtime_archive(agy: &[u8]) -> (Vec<u8>, String) {
        let manifest =
            br#"{"agy":"bin/agy","hooks":"policy/hooks.json","settings":"policy/settings.json"}"#;
        let bytes = tar::build(&[
            ("cowboy-runtime.json", b'0', manifest, 0o644),
            ("bin/agy", b'0', agy, 0o755),
            ("bin/agy-proxy", b'0', b"proxy", 0o755),
            ("policy/hooks.json", b'0', b"{}", 0o644),
            ("policy/settings.json", b'0', b"{}", 0o644),
        ]);
        let sha = hex::encode(Sha256::digest(&bytes));
        (bytes, sha)
    }

    pub fn test_config(temp: &Path) -> Config {
        let allow = temp.join("agy-sha512.allow");
        std::fs::write(&allow, format!("# pinned agy\n{}\n", agy_sha512())).unwrap();
        Config {
            state_root: temp.join("state"),
            run_dir: temp.join("run"),
            agy_allow_list: allow,
            max_runtimes: 2,
            ..Config::default()
        }
    }

    fn store_in(temp: &Path) -> OwnedFd {
        std::fs::create_dir_all(temp.join("state")).unwrap();
        let root = safefs::open_root(&temp.join("state")).unwrap();
        open_store(root.as_fd()).unwrap()
    }

    #[test]
    fn allow_list_parsing_is_strict() {
        let digest = agy_sha512();
        assert!(
            parse_allow_list(&format!("\n# c\n{digest}\n"))
                .unwrap()
                .contains(&digest)
        );
        assert!(parse_allow_list(&digest[..127]).is_err());
        assert!(parse_allow_list(&digest.to_uppercase()).is_err());
        assert!(parse_allow_list("").unwrap().is_empty());
    }

    #[test]
    fn registration_verifies_digest_and_allow_list() {
        let temp = tempfile::tempdir().unwrap();
        let config = test_config(temp.path());
        let store = store_in(temp.path());
        let (bytes, sha) = runtime_archive(AGY);
        let lock = RwLock::new(());
        let none = || Ok(HashSet::new());
        let live: LiveDigests<'_> = &none;
        let wrong = "0".repeat(64);
        let mismatch = register(
            store.as_fd(),
            &config,
            &wrong,
            bytes.len() as u64,
            &mut &bytes[..],
            &lock,
            live,
        );
        assert_eq!(mismatch.unwrap_err().code, Code::RuntimeRejected);
        assert!(
            !register(
                store.as_fd(),
                &config,
                &sha,
                bytes.len() as u64,
                &mut &bytes[..],
                &lock,
                live
            )
            .unwrap()
        );
        assert!(
            register(
                store.as_fd(),
                &config,
                &sha,
                bytes.len() as u64,
                &mut &bytes[..],
                &lock,
                live
            )
            .unwrap()
        );
        let manifest = load(store.as_fd(), &sha, &config.agy_allow_list).unwrap();
        assert_eq!(manifest.agy, "bin/agy");
        let (other, other_sha) = runtime_archive(b"not the pinned agy");
        let rejected = register(
            store.as_fd(),
            &config,
            &other_sha,
            other.len() as u64,
            &mut &other[..],
            &lock,
            live,
        );
        assert_eq!(rejected.unwrap_err().code, Code::RuntimeRejected);
        assert_eq!(digests(store.as_fd()).unwrap(), std::slice::from_ref(&sha));
        assert_eq!(safefs::list(store.as_fd()).unwrap(), [sha]);
        let short = register(
            store.as_fd(),
            &config,
            &other_sha,
            other.len() as u64 + 1,
            &mut &other[..],
            &lock,
            live,
        );
        assert_eq!(short.unwrap_err().code, Code::InvalidRequest);
    }

    #[test]
    fn eviction_keeps_live_runtimes() {
        let temp = tempfile::tempdir().unwrap();
        let config = test_config(temp.path());
        let store = store_in(temp.path());
        let lock = RwLock::new(());
        let mut shas = Vec::new();
        for extra in ["a", "b"] {
            let manifest = br#"{"agy":"bin/agy","hooks":"h","settings":"s"}"#;
            let bytes = tar::build(&[
                ("cowboy-runtime.json", b'0', manifest, 0o644),
                ("bin/agy", b'0', AGY, 0o755),
                ("h", b'0', b"{}", 0o644),
                ("s", b'0', b"{}", 0o644),
                (extra, b'0', b"", 0o644),
            ]);
            let sha = hex::encode(Sha256::digest(&bytes));
            register(
                store.as_fd(),
                &config,
                &sha,
                bytes.len() as u64,
                &mut &bytes[..],
                &lock,
                &|| Ok(HashSet::new()),
            )
            .unwrap();
            shas.push(sha);
        }
        let all: HashSet<String> = shas.iter().cloned().collect();
        let (bytes, sha) = runtime_archive(AGY);
        let full = register(
            store.as_fd(),
            &config,
            &sha,
            bytes.len() as u64,
            &mut &bytes[..],
            &lock,
            &|| Ok(all.clone()),
        );
        assert_eq!(full.unwrap_err().code, Code::Limit);
        let one: HashSet<String> = [shas[1].clone()].into();
        register(
            store.as_fd(),
            &config,
            &sha,
            bytes.len() as u64,
            &mut &bytes[..],
            &lock,
            &|| Ok(one.clone()),
        )
        .unwrap();
        let mut expected = vec![shas[1].clone(), sha];
        expected.sort();
        assert_eq!(digests(store.as_fd()).unwrap(), expected);
    }
    #[test]
    fn sibling_deduplication_hardlinks_identical_binaries_and_survives_eviction() {
        use std::os::unix::fs::MetadataExt as _;
        let temp = tempfile::tempdir().unwrap();
        let config = test_config(temp.path());
        let store = store_in(temp.path());
        let lock = RwLock::new(());
        let none = || Ok(HashSet::new());
        let live: LiveDigests<'_> = &none;

        let manifest =
            br#"{"agy":"bin/agy","hooks":"policy/hooks.json","settings":"policy/settings.json"}"#;
        let build_rt = |proxy_script: &[u8], node_mode: u32| {
            let bytes = tar::build(&[
                ("cowboy-runtime.json", b'0', manifest, 0o644),
                ("bin/agy", b'0', AGY, 0o755),
                ("bin/agy-proxy", b'0', proxy_script, 0o755),
                (
                    "proxy/runtime/node",
                    b'0',
                    b"identical-node-binary",
                    node_mode,
                ),
                ("policy/hooks.json", b'0', b"{}", 0o644),
                ("policy/settings.json", b'0', b"{}", 0o644),
            ]);
            let sha = hex::encode(Sha256::digest(&bytes));
            (bytes, sha)
        };

        let (rt1, sha1) = build_rt(b"proxy-v1", 0o755);
        let (rt2, sha2) = build_rt(b"proxy-v2", 0o755);
        assert!(
            !register(
                store.as_fd(),
                &config,
                &sha1,
                rt1.len() as u64,
                &mut &rt1[..],
                &lock,
                live
            )
            .unwrap()
        );
        assert!(
            !register(
                store.as_fd(),
                &config,
                &sha2,
                rt2.len() as u64,
                &mut &rt2[..],
                &lock,
                live
            )
            .unwrap()
        );

        let st_root = temp.path().join("state/runtime");
        let agy1 = std::fs::symlink_metadata(st_root.join(&sha1).join("bin/agy")).unwrap();
        let agy2 = std::fs::symlink_metadata(st_root.join(&sha2).join("bin/agy")).unwrap();
        assert_eq!((agy1.dev(), agy1.ino()), (agy2.dev(), agy2.ino()));
        assert_eq!(agy1.nlink(), 2);

        let node1 =
            std::fs::symlink_metadata(st_root.join(&sha1).join("proxy/runtime/node")).unwrap();
        let node2 =
            std::fs::symlink_metadata(st_root.join(&sha2).join("proxy/runtime/node")).unwrap();
        assert_eq!((node1.dev(), node1.ino()), (node2.dev(), node2.ino()));
        assert_eq!(node1.nlink(), 2);

        // Changed file stays distinct.
        let p1 = std::fs::symlink_metadata(st_root.join(&sha1).join("bin/agy-proxy")).unwrap();
        let p2 = std::fs::symlink_metadata(st_root.join(&sha2).join("bin/agy-proxy")).unwrap();
        assert_ne!(p1.ino(), p2.ino());

        // Different normalized mode on node (0o444 vs 0o555) does NOT share inode; evicting sha1 keeps sha2 intact.
        let (rt3, sha3) = build_rt(b"proxy-v3", 0o644);
        // Make sha1 oldest so eviction removes sha1 (max_runtimes = 2).
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let keep_sha2 = || Ok(HashSet::from([sha2.clone()]));
        assert!(
            !register(
                store.as_fd(),
                &config,
                &sha3,
                rt3.len() as u64,
                &mut &rt3[..],
                &lock,
                &keep_sha2
            )
            .unwrap()
        );
        assert!(!st_root.join(&sha1).exists());

        // sha2 survives eviction intact and shares agy with sha3, while node mode differs so stays separate.
        assert_eq!(
            load(store.as_fd(), &sha2, &config.agy_allow_list)
                .unwrap()
                .agy,
            "bin/agy"
        );
        let agy3 = std::fs::symlink_metadata(st_root.join(&sha3).join("bin/agy")).unwrap();
        assert_eq!((agy2.dev(), agy2.ino()), (agy3.dev(), agy3.ino()));
        let node3 =
            std::fs::symlink_metadata(st_root.join(&sha3).join("proxy/runtime/node")).unwrap();
        assert_ne!(node2.ino(), node3.ino());
    }
}
