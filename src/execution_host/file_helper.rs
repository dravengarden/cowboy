//! Target-local file utilities, invoked through native process/start. They do
//! not introduce another RPC, filesystem authority, or interpreter dependency.
use anyhow::{Result, ensure};
use base64::Engine as _;
use clap::Subcommand;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Component, Path, PathBuf};

const TRANSCRIPT_LIMIT: usize = 8 * 1024 * 1024;
const READ_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Subcommand)]
pub enum Command {
    /// Verify an append and publish independent private cache/snapshot files.
    Snapshot {
        cache: PathBuf,
        delta: PathBuf,
        snapshot: PathBuf,
        base_hash: String,
        wanted: String,
    },
    /// Read a line range while retaining a verified whole-file conflict stamp.
    ReadRange {
        path: PathBuf,
        offset: usize,
        limit: usize,
    },
    /// Resolve symlinks while permitting nonexistent trailing components.
    Realpath { path: PathBuf },
    /// Compare-and-swap one file against the caller's buffered digest, then
    /// publish the spliced result atomically. The request is JSON, either
    /// inline as base64 or in a private payload file this call consumes.
    Apply {
        #[arg(long, conflicts_with = "request_file")]
        request: Option<String>,
        #[arg(long)]
        request_file: Option<PathBuf>,
    },
}

fn digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn open(path: &Path, no_follow: bool) -> Result<File> {
    ensure!(path.is_absolute(), "absolute file path required");
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | if no_follow { libc::O_NOFOLLOW } else { 0 })
        .open(path)?)
}

fn read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let source = open(path, true)?;
    ensure!(source.metadata()?.is_file(), "regular file required");
    let mut data = Vec::new();
    source.take((limit + 1) as u64).read_to_end(&mut data)?;
    ensure!(data.len() <= limit, "file exceeds limit");
    Ok(data)
}

fn write_new(path: &Path, data: &[u8]) -> Result<()> {
    ensure!(path.is_absolute(), "absolute file path required");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?
        .write_all(data)?;
    Ok(())
}

fn snapshot(cache: &Path, delta: &Path, snapshot: &Path, base: &str, wanted: &str) -> Result<i32> {
    ensure!(
        wanted.len() == 64 && wanted.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid digest"
    );
    let mut data = if base.is_empty() {
        Vec::new()
    } else {
        match read(cache, TRANSCRIPT_LIMIT) {
            Ok(data) if digest(&data) == base => data,
            _ => return Ok(75), // Explicit cache miss: no snapshot or hook started.
        }
    };
    data.extend(read(delta, TRANSCRIPT_LIMIT - data.len())?);
    ensure!(digest(&data) == wanted, "transcript digest mismatch");
    write_new(snapshot, &data)?;
    let temporary = snapshot.with_file_name(format!(
        "{}.cache",
        snapshot
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("snapshot name missing"))?
            .to_string_lossy()
    ));
    // Remove only a temporary file this invocation created, never a preexisting
    // file at an unexpected path. The caller owns cleanup after uncertainty.
    write_new(&temporary, &data)?;
    let result = std::fs::rename(&temporary, cache);
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result?;
    Ok(0)
}

fn stamp(value: &Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    (
        value.dev(),
        value.ino(),
        value.len(),
        value.mtime(),
        value.mtime_nsec(),
        value.ctime(),
        value.ctime_nsec(),
    )
}

fn range_with(
    path: &Path,
    offset: usize,
    limit: usize,
    after_read: impl FnOnce(),
) -> Result<Value> {
    ensure!(
        (1..=10_000_000).contains(&offset) && (1..=10_000).contains(&limit),
        "invalid range"
    );
    let mut source = open(path, false)?;
    let before = source.metadata()?;
    ensure!(
        before.is_file() && before.len() <= READ_LIMIT as u64,
        "regular bounded file required"
    );
    let mut data = Vec::new();
    (&mut source)
        .take((READ_LIMIT + 1) as u64)
        .read_to_end(&mut data)?;
    after_read();
    let after = source.metadata()?;
    ensure!(
        data.len() as u64 == before.len()
            && data.len() <= READ_LIMIT
            && stamp(&before) == stamp(&after)
            && stamp(&after) == stamp(&std::fs::metadata(path)?),
        "file changed while reading"
    );
    if data.starts_with(b"\x89PNG\r\n\x1a\n")
        || data.starts_with(b"\xff\xd8\xff")
        || data.starts_with(b"GIF")
        || data.starts_with(b"%PDF")
        || (data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP"))
    {
        return Ok(json!({"fallback": true}));
    }
    let text = String::from_utf8_lossy(&data);
    let lines: Vec<_> = text.split('\n').collect();
    let selected: Vec<_> = lines.iter().skip(offset - 1).take(limit).copied().collect();
    let content = selected.join("\n");
    if content.len() > 32768 {
        return Ok(json!({"fallback": true}));
    }
    Ok(
        json!({"schema": 1, "sha256": digest(&data), "size": data.len(), "totalLines": lines.len(),
        "startLine": offset, "numLines": selected.len(),
        "dataBase64": base64::engine::general_purpose::STANDARD.encode(content.as_bytes())}),
    )
}

fn realpath(path: &Path) -> Result<PathBuf> {
    ensure!(path.is_absolute(), "absolute file path required");
    let mut resolved = PathBuf::from("/");
    let mut pending: std::collections::VecDeque<_> = path
        .components()
        .map(|c| c.as_os_str().to_owned())
        .collect();
    let mut links = 0;
    while let Some(part) = pending.pop_front() {
        match Path::new(&part).components().next() {
            Some(Component::RootDir) => resolved = PathBuf::from("/"),
            Some(Component::ParentDir) => {
                resolved.pop();
            }
            Some(Component::Normal(_)) => {
                resolved.push(&part);
                match std::fs::symlink_metadata(&resolved) {
                    Ok(m) if m.file_type().is_symlink() => {
                        links += 1;
                        ensure!(links <= 40, "too many symlinks");
                        let target = std::fs::read_link(&resolved)?;
                        resolved.pop();
                        for component in target.components().rev() {
                            pending.push_front(component.as_os_str().to_owned());
                        }
                    }
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            _ => {}
        }
    }
    Ok(resolved)
}

/// One replacement of the base bytes `[offset, offset + delete)`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Splice {
    offset: usize,
    delete: usize,
    /// Base64 bytes inserted at `offset`.
    insert: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ApplyRequest {
    schema: u16,
    path: PathBuf,
    /// The digest of the caller's buffered copy, or `absent` for a new file.
    base: String,
    /// The digest of the base with the splices applied.
    expect: String,
    splices: Vec<Splice>,
    /// Remove this helper's abandoned temporary files older than this from
    /// the target directory and the payload directory.
    stale_temp_seconds: u64,
    payload_directory: Option<PathBuf>,
}

const ABSENT: &str = "absent";
const TEMP_PREFIX: &str = ".cowboy-buffer-";
const TEMP_SUFFIX: &str = ".tmp";
const PAYLOAD_PREFIX: &str = "payload-";
const PAYLOAD_SUFFIX: &str = ".json";
/// A temporary file younger than this may belong to a running apply.
const MIN_STALE_TEMP_SECONDS: u64 = 600;
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
const PAYLOAD_LIMIT: usize = READ_LIMIT * 2 + 65536;

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The target's bytes and the metadata they were read under, or `None` when
/// it does not exist. A file that changes while it is read is an error.
fn current(path: &Path) -> Result<Option<(Vec<u8>, Metadata)>> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
        Ok(metadata) => {
            ensure!(metadata.is_file(), "regular file required");
            let mut source = open(path, true)?;
            let before = source.metadata()?;
            let mut data = Vec::new();
            (&mut source)
                .take((READ_LIMIT + 1) as u64)
                .read_to_end(&mut data)?;
            ensure!(data.len() <= READ_LIMIT, "file exceeds limit");
            ensure!(
                data.len() as u64 == before.len() && stamp(&before) == stamp(&source.metadata()?),
                "file changed while reading"
            );
            Ok(Some((data, before)))
        }
    }
}

fn splice(base: &[u8], splices: &[Splice]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(base.len());
    let mut cursor = 0;
    for item in splices {
        let end = item
            .offset
            .checked_add(item.delete)
            .filter(|end| item.offset >= cursor && *end <= base.len())
            .ok_or_else(|| {
                anyhow::anyhow!("splices must be ordered, disjoint and inside the base")
            })?;
        out.extend_from_slice(&base[cursor..item.offset]);
        out.extend(base64::engine::general_purpose::STANDARD.decode(&item.insert)?);
        ensure!(out.len() <= READ_LIMIT, "result exceeds limit");
        cursor = end;
    }
    out.extend_from_slice(&base[cursor..]);
    ensure!(out.len() <= READ_LIMIT, "result exceeds limit");
    Ok(out)
}

/// An exclusive advisory lock on the parent directory serializes Cowboy
/// writers across sessions. Writers that ignore it are caught by the
/// just-in-time check before publishing instead.
fn lock_directory(directory: &Path) -> Result<File> {
    let handle = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY)
        .open(directory)?;
    let started = std::time::Instant::now();
    loop {
        let error = match rustix::fs::flock(
            &handle,
            rustix::fs::FlockOperation::NonBlockingLockExclusive,
        ) {
            Ok(()) => return Ok(handle),
            Err(error) => error,
        };
        ensure!(
            error == rustix::io::Errno::WOULDBLOCK && started.elapsed() < LOCK_WAIT,
            "directory lock unavailable: {error}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Remove only regular files named exactly as this helper names them.
fn sweep(directory: &Path, prefix: &str, suffix: &str, older_than: u64) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let Some(cutoff) = std::time::SystemTime::now().checked_sub(std::time::Duration::from_secs(
        older_than.max(MIN_STALE_TEMP_SECONDS),
    )) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(middle) = name
            .to_str()
            .and_then(|name| name.strip_prefix(prefix))
            .and_then(|rest| rest.strip_suffix(suffix))
        else {
            continue;
        };
        if middle.len() != 32 || !middle.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        if let Ok(metadata) = std::fs::symlink_metadata(entry.path())
            && metadata.is_file()
            && metadata.modified().is_ok_and(|modified| modified < cutoff)
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn outcome(status: &str, digest: &str, size: usize) -> Value {
    json!({"schema": 1, "status": status, "sha256": digest, "size": size})
}

fn held(path: &Path) -> Result<Value> {
    let now = current(path)?;
    let size = now.as_ref().map_or(0, |(data, _)| data.len());
    let digest = now.map_or_else(|| ABSENT.to_owned(), |(data, _)| digest(&data));
    Ok(outcome("conflict", &digest, size))
}

/// Compare-and-swap one file against the caller's buffered digest under a
/// directory lock, publish it by rename, and read back what was published.
fn apply_with(request: &ApplyRequest, before_publish: impl FnOnce()) -> Result<Value> {
    ensure!(request.schema == 1, "unsupported apply schema");
    ensure!(
        (request.base == ABSENT || valid_digest(&request.base)) && valid_digest(&request.expect),
        "invalid digest"
    );
    ensure!(request.path.is_absolute(), "absolute file path required");
    if let Some(directory) = &request.payload_directory {
        ensure!(
            directory.is_absolute(),
            "absolute payload directory required"
        );
        sweep(
            directory,
            PAYLOAD_PREFIX,
            PAYLOAD_SUFFIX,
            request.stale_temp_seconds,
        );
    }
    // A symlink is written through, as a native write is: its target changes.
    let path = realpath(&request.path)?;
    let directory = path
        .parent()
        .filter(|_| path.file_name().is_some())
        .ok_or_else(|| anyhow::anyhow!("file path has no parent"))?
        .to_path_buf();
    // As natively, writing a file creates its missing parent directories.
    std::fs::create_dir_all(&directory)?;
    let _lock = lock_directory(&directory)?;
    sweep(
        &directory,
        TEMP_PREFIX,
        TEMP_SUFFIX,
        request.stale_temp_seconds,
    );
    let held_now = current(&path)?;
    let held_digest = held_now
        .as_ref()
        .map_or_else(|| ABSENT.to_owned(), |(data, _)| digest(data));
    // A retried apply whose first attempt published finds its own result.
    if held_digest == request.expect {
        let size = held_now.as_ref().map_or(0, |(data, _)| data.len());
        return Ok(outcome("applied", &held_digest, size));
    }
    if held_digest != request.base {
        let size = held_now.as_ref().map_or(0, |(data, _)| data.len());
        return Ok(outcome("conflict", &held_digest, size));
    }
    let base = held_now
        .as_ref()
        .map_or(&[][..], |(data, _)| data.as_slice());
    let result = splice(base, &request.splices)?;
    ensure!(
        digest(&result) == request.expect,
        "splices do not produce the expected digest"
    );
    let metadata = held_now.as_ref().map(|(_, metadata)| metadata);
    let (uid, gid) = (
        rustix::process::getuid().as_raw(),
        rustix::process::getgid().as_raw(),
    );
    let foreign = metadata.is_some_and(|m| m.uid() != uid || m.gid() != gid);
    let temporary = directory.join(format!(
        "{TEMP_PREFIX}{:032x}{TEMP_SUFFIX}",
        rand::random::<u128>()
    ));
    let prepared = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            // A new file gets the process umask, as a native create does.
            .mode(if metadata.is_some() { 0o600 } else { 0o666 })
            .open(&temporary)?;
        file.write_all(&result)?;
        if let Some(metadata) = metadata {
            if foreign {
                std::os::unix::fs::fchown(&file, Some(metadata.uid()), Some(metadata.gid()))?;
            }
            file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(
                metadata.mode() & 0o7777,
            ))?;
        }
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = prepared {
        let _ = std::fs::remove_file(&temporary);
        // An owner this user cannot reproduce must not change: the file is
        // rewritten in place instead, as the executor's own write does.
        ensure!(foreign, "{error}");
        return in_place(&path, &result, metadata, &request.expect);
    }
    before_publish();
    // Just in time: the target must still be exactly what was compared.
    let unchanged = match (std::fs::symlink_metadata(&path), metadata) {
        (Err(error), None) => error.kind() == std::io::ErrorKind::NotFound,
        (Ok(now), Some(then)) => stamp(&now) == stamp(then),
        _ => false,
    };
    if !unchanged {
        let _ = std::fs::remove_file(&temporary);
        return held(&path);
    }
    let published = if metadata.is_none() {
        // A link never replaces a file another writer created meanwhile.
        match std::fs::hard_link(&temporary, &path) {
            Ok(()) => std::fs::remove_file(&temporary).map_err(anyhow::Error::from),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_file(&temporary);
                return held(&path);
            }
            // Filesystems without hard links fall back to the checked rename.
            Err(_) => std::fs::rename(&temporary, &path).map_err(anyhow::Error::from),
        }
    } else {
        std::fs::rename(&temporary, &path).map_err(anyhow::Error::from)
    };
    if let Err(error) = published {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    File::open(&directory)?.sync_all()?;
    verify(&path, &request.expect, result.len())
}

fn in_place(path: &Path, data: &[u8], metadata: Option<&Metadata>, expect: &str) -> Result<Value> {
    let metadata =
        metadata.ok_or_else(|| anyhow::anyhow!("an in-place write needs an existing file"))?;
    let mut file = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    if stamp(&file.metadata()?) != stamp(metadata) {
        return held(path);
    }
    file.set_len(0)?;
    file.write_all(data)?;
    file.sync_all()?;
    verify(path, expect, data.len())
}

/// Read back what was published: the digest the caller confirms on its side.
fn verify(path: &Path, expect: &str, size: usize) -> Result<Value> {
    let published = digest(&read(path, READ_LIMIT)?);
    ensure!(
        published == expect,
        "published file does not match the expected digest"
    );
    Ok(outcome("applied", &published, size))
}

fn apply_request(inline: Option<String>, payload: Option<PathBuf>) -> Result<ApplyRequest> {
    let raw = match (inline, payload) {
        (Some(inline), None) => base64::engine::general_purpose::STANDARD.decode(inline)?,
        (None, Some(payload)) => {
            // A private one-shot file the caller wrote; consumed here.
            let data = read(&payload, PAYLOAD_LIMIT);
            let _ = std::fs::remove_file(&payload);
            data?
        }
        _ => anyhow::bail!("exactly one of an inline request or a request file"),
    };
    Ok(serde_json::from_slice(&raw)?)
}

/// Execute without logging filenames or file contents on failure.
pub fn run(command: Command) -> Result<i32> {
    match command {
        Command::Snapshot {
            cache,
            delta,
            snapshot: path,
            base_hash,
            wanted,
        } => snapshot(&cache, &delta, &path, &base_hash, &wanted),
        Command::ReadRange {
            path,
            offset,
            limit,
        } => {
            let value = range_with(&path, offset, limit, || {}).unwrap_or_else(
                |_| json!({"error": "Target range read failed or file changed; read it again."}),
            );
            println!("{value}");
            Ok(0)
        }
        Command::Realpath { path } => {
            print!(
                "{}",
                realpath(&path)?
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("path is not UTF-8"))?
            );
            Ok(0)
        }
        Command::Apply {
            request,
            request_file,
        } => {
            // Every failure leaves the target either unchanged or exactly at
            // the expected digest; the caller re-reads to tell which.
            let value = apply_request(request, request_file)
                .and_then(|request| apply_with(&request, || {}))
                .unwrap_or_else(
                    |error| json!({"schema": 1, "status": "error", "error": error.to_string()}),
                );
            println!("{value}");
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    #[test]
    fn snapshots_verify_base_and_preserve_old_copies() {
        let dir = tempfile::tempdir().unwrap();
        let p = |name| dir.path().join(name);
        std::fs::write(p("delta"), b"first").unwrap();
        assert_eq!(
            snapshot(&p("cache"), &p("delta"), &p("one"), "", &digest(b"first")).unwrap(),
            0
        );
        std::fs::write(p("delta"), b" next").unwrap();
        assert_eq!(
            snapshot(
                &p("cache"),
                &p("delta"),
                &p("two"),
                &digest(b"first"),
                &digest(b"first next")
            )
            .unwrap(),
            0
        );
        assert_eq!(std::fs::read(p("one")).unwrap(), b"first");
        assert_eq!(std::fs::read(p("two")).unwrap(), b"first next");
        assert_eq!(
            std::fs::metadata(p("cache")).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            snapshot(
                &p("cache"),
                &p("delta"),
                &p("miss"),
                &digest(b"first"),
                &digest(b"first next")
            )
            .unwrap(),
            75
        );
        assert!(!p("miss").exists());
        assert!(snapshot(&p("cache"), &p("delta"), &p("bad"), "", &digest(b"wrong")).is_err());
        assert!(!p("bad").exists());
        symlink(p("delta"), p("link")).unwrap();
        assert!(
            snapshot(
                &p("cache"),
                &p("link"),
                &p("link-copy"),
                "",
                &digest(b" next")
            )
            .is_err()
        );
    }

    #[test]
    fn range_rejects_a_racing_write_and_resolves_missing_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        std::fs::write(&path, b"before\n").unwrap();
        assert!(range_with(&path, 1, 1, || std::fs::write(&path, b"after\n").unwrap()).is_err());
        symlink(dir.path(), dir.path().join("link")).unwrap();
        assert_eq!(
            realpath(&dir.path().join("link/missing/../tail")).unwrap(),
            dir.path().join("tail")
        );
    }

    fn request(path: &Path, base: &[u8], result: &[u8], splices: Vec<Splice>) -> ApplyRequest {
        ApplyRequest {
            schema: 1,
            path: path.to_path_buf(),
            base: digest(base),
            expect: digest(result),
            splices,
            stale_temp_seconds: 3600,
            payload_directory: None,
        }
    }

    fn insert(offset: usize, delete: usize, text: &[u8]) -> Splice {
        Splice {
            offset,
            delete,
            insert: base64::engine::general_purpose::STANDARD.encode(text),
        }
    }

    fn temps(directory: &Path) -> usize {
        std::fs::read_dir(directory)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX))
            .count()
    }

    #[test]
    fn apply_swaps_only_the_buffered_base_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        std::fs::write(&path, b"hello world\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o751)).unwrap();
        let edit = request(
            &path,
            b"hello world\n",
            b"hello there world\n",
            vec![insert(6, 0, b"there ")],
        );
        let value = apply_with(&edit, || {}).unwrap();
        assert_eq!(value["status"], "applied");
        assert_eq!(value["sha256"], digest(b"hello there world\n"));
        assert_eq!(std::fs::read(&path).unwrap(), b"hello there world\n");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o751
        );
        // A lost reply's retry finds its own result and changes nothing.
        let inode = std::fs::metadata(&path).unwrap().ino();
        assert_eq!(apply_with(&edit, || {}).unwrap()["status"], "applied");
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
        // A stale buffer is refused with the digest the target holds.
        let stale = request(&path, b"hello world\n", b"bye", vec![insert(0, 12, b"bye")]);
        let value = apply_with(&stale, || {}).unwrap();
        assert_eq!(value["status"], "conflict");
        assert_eq!(value["sha256"], digest(b"hello there world\n"));
        assert_eq!(std::fs::read(&path).unwrap(), b"hello there world\n");
        assert_eq!(temps(dir.path()), 0);
    }

    #[test]
    fn apply_refuses_a_write_racing_the_publish() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        std::fs::write(&path, b"one").unwrap();
        let edit = request(&path, b"one", b"two", vec![insert(0, 3, b"two")]);
        let value = apply_with(&edit, || std::fs::write(&path, b"other").unwrap()).unwrap();
        assert_eq!(value["status"], "conflict");
        assert_eq!(value["sha256"], digest(b"other"));
        assert_eq!(std::fs::read(&path).unwrap(), b"other");
        assert_eq!(temps(dir.path()), 0);
        // A file created while a new one was being prepared is kept.
        let fresh = dir.path().join("fresh");
        let mut create = request(&fresh, b"", b"mine", vec![insert(0, 0, b"mine")]);
        create.base = ABSENT.to_owned();
        let value = apply_with(&create, || std::fs::write(&fresh, b"theirs").unwrap()).unwrap();
        assert_eq!(value["status"], "conflict");
        assert_eq!(std::fs::read(&fresh).unwrap(), b"theirs");
        assert_eq!(temps(dir.path()), 0);
    }

    #[test]
    fn apply_creates_parents_and_writes_through_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/new.rs");
        let mut create = request(
            &nested,
            b"",
            b"fn main() {}\n",
            vec![insert(0, 0, b"fn main() {}\n")],
        );
        create.base = ABSENT.to_owned();
        assert_eq!(apply_with(&create, || {}).unwrap()["status"], "applied");
        assert_eq!(std::fs::read(&nested).unwrap(), b"fn main() {}\n");
        // Creating over an existing file with an absent base is a conflict.
        let mut again = request(&nested, b"", b"x", vec![insert(0, 0, b"x")]);
        again.base = ABSENT.to_owned();
        let value = apply_with(&again, || {}).unwrap();
        assert_eq!(value["status"], "conflict");
        let target = dir.path().join("target");
        std::fs::write(&target, b"abc").unwrap();
        let link = dir.path().join("link");
        symlink(&target, &link).unwrap();
        let edit = request(&link, b"abc", b"abXc", vec![insert(2, 0, b"X")]);
        assert_eq!(apply_with(&edit, || {}).unwrap()["status"], "applied");
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"abXc");
    }

    #[test]
    fn apply_rejects_bad_requests_without_touching_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        std::fs::write(&path, b"abcdef").unwrap();
        // Splices that do not produce the promised digest.
        let wrong = request(&path, b"abcdef", b"zzz", vec![insert(0, 1, b"Q")]);
        assert!(apply_with(&wrong, || {}).is_err());
        // Overlapping and out-of-range splices.
        let overlap = request(
            &path,
            b"abcdef",
            b"x",
            vec![insert(2, 2, b""), insert(1, 0, b"")],
        );
        assert!(apply_with(&overlap, || {}).is_err());
        let outside = request(&path, b"abcdef", b"x", vec![insert(5, 9, b"")]);
        assert!(apply_with(&outside, || {}).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"abcdef");
        // A directory or a FIFO is never a buffered file.
        let directory = dir.path().join("dir");
        std::fs::create_dir(&directory).unwrap();
        assert!(apply_with(&request(&directory, b"", b"x", vec![]), || {}).is_err());
        let fifo = dir.path().join("fifo");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        assert!(apply_with(&request(&fifo, b"", b"x", vec![]), || {}).is_err());
        assert_eq!(temps(dir.path()), 0);
        let mut relative = request(&path, b"abcdef", b"abcdef", vec![]);
        relative.path = PathBuf::from("file");
        assert!(apply_with(&relative, || {}).is_err());
    }

    #[test]
    fn apply_sweeps_only_its_own_stale_files() {
        let dir = tempfile::tempdir().unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        let stale = dir
            .path()
            .join(format!("{TEMP_PREFIX}{}{TEMP_SUFFIX}", "a".repeat(32)));
        let young = dir
            .path()
            .join(format!("{TEMP_PREFIX}{}{TEMP_SUFFIX}", "b".repeat(32)));
        let foreign = dir
            .path()
            .join(format!("{TEMP_PREFIX}notours{TEMP_SUFFIX}"));
        let payloads = dir.path().join("payloads");
        std::fs::create_dir(&payloads).unwrap();
        let payload = payloads.join(format!(
            "{PAYLOAD_PREFIX}{}{PAYLOAD_SUFFIX}",
            "c".repeat(32)
        ));
        for file in [&stale, &young, &foreign, &payload] {
            std::fs::write(file, b"x").unwrap();
        }
        for file in [&stale, &foreign, &payload] {
            File::options()
                .write(true)
                .open(file)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        let path = dir.path().join("file");
        let mut create = request(&path, b"", b"x", vec![insert(0, 0, b"x")]);
        create.base = ABSENT.to_owned();
        create.payload_directory = Some(payloads.clone());
        assert_eq!(apply_with(&create, || {}).unwrap()["status"], "applied");
        assert!(!stale.exists() && !payload.exists());
        assert!(young.exists() && foreign.exists());
    }

    #[test]
    fn apply_requests_come_inline_or_from_a_consumed_payload() {
        let dir = tempfile::tempdir().unwrap();
        let json = br#"{"schema":1,"path":"/x","base":"absent","expect":"0000000000000000000000000000000000000000000000000000000000000000","splices":[],"staleTempSeconds":60}"#;
        let inline = base64::engine::general_purpose::STANDARD.encode(json);
        assert_eq!(
            apply_request(Some(inline), None)
                .unwrap()
                .stale_temp_seconds,
            60
        );
        let file = dir.path().join("payload");
        std::fs::write(&file, json).unwrap();
        assert!(apply_request(None, Some(file.clone())).is_ok());
        assert!(!file.exists());
        assert!(apply_request(None, None).is_err());
        let unknown =
            base64::engine::general_purpose::STANDARD.encode(br#"{"schema":1,"extra":1}"#);
        assert!(apply_request(Some(unknown), None).is_err());
    }
}
