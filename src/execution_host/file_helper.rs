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
}
