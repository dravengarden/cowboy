//! Bounded read-only change hints, not signed identity or namespace continuity.
//! Inspect metadata only: never read artifacts, private state or credentials.

use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use anyhow::{Result, ensure};
use sha2::{Digest as _, Sha256};

const MAX_ENTRIES: usize = 8192;
const MAX_NAME_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Hint([u8; 32]);

#[derive(Default)]
struct Budget {
    entries: usize,
    names: usize,
}

pub(super) fn sample(roots: &[PathBuf]) -> Result<Hint> {
    let mut digest = Sha256::new();
    let mut budget = Budget::default();
    for root in roots {
        directory(root, false, &mut digest, &mut budget)?;
        directory(
            &root.join("trusted-publishers"),
            true,
            &mut digest,
            &mut budget,
        )?;
    }
    Ok(Hint(digest.finalize().into()))
}

fn directory(path: &Path, trust: bool, digest: &mut Sha256, budget: &mut Budget) -> Result<()> {
    let name = path.as_os_str().as_encoded_bytes();
    budget.names += name.len();
    ensure!(
        budget.names <= MAX_NAME_BYTES,
        "Catalog source name budget exceeded"
    );
    bytes(digest, name);
    metadata(path, digest)?;
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry?;
        budget.entries += 1;
        budget.names += entry.file_name().as_encoded_bytes().len();
        ensure!(
            budget.entries <= MAX_ENTRIES,
            "Catalog source entry budget exceeded"
        );
        ensure!(
            budget.names <= MAX_NAME_BYTES,
            "Catalog source name budget exceeded"
        );
        let name = entry.file_name();
        let name = name.as_encoded_bytes();
        if (trust && name.ends_with(b".pub"))
            || (!trust
                && [
                    b".cowboy-plugin".as_slice(),
                    b".cowboy-provider",
                    b".release.json",
                    b".hostbundle.json",
                ]
                .iter()
                .any(|suffix| name.ends_with(suffix)))
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    for path in paths {
        bytes(
            digest,
            path.file_name()
                .expect("directory entry has a name")
                .as_encoded_bytes(),
        );
        metadata(&path, digest)?;
    }
    Ok(())
}

fn bytes(digest: &mut Sha256, value: &[u8]) {
    digest.update(value.len().to_le_bytes());
    digest.update(value);
}

fn metadata(path: &Path, digest: &mut Sha256) -> Result<()> {
    // Include the link and its target so replacement of a configured symlink
    // is visible too. This performs no file open (including FIFOs/devices).
    for result in [fs::symlink_metadata(path), fs::metadata(path)] {
        let meta = match result {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                digest.update([0]);
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        digest.update([1]);
        digest.update(meta.dev().to_le_bytes());
        digest.update(meta.ino().to_le_bytes());
        digest.update(meta.mode().to_le_bytes());
        if !meta.is_dir() {
            // Ignore directory mtimes and access times: unrelated download and
            // reader activity must not rebuild all selected Plugin hosts.
            digest.update(meta.len().to_le_bytes());
            digest.update(meta.mtime().to_le_bytes());
            digest.update(meta.mtime_nsec().to_le_bytes());
            digest.update(meta.ctime().to_le_bytes());
            digest.update(meta.ctime_nsec().to_le_bytes());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
