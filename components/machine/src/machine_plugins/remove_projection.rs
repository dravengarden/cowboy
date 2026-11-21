//! Remove a Machine-owned credential projection, including read-only caches.
//! Directory permissions change through opened descriptors; symlinks are only
//! unlinked, and file permissions (including external hard links) never change.

use anyhow::{Result, ensure};
use rustix::fd::AsFd;
use rustix::fs::{AtFlags, Dir, Mode, OFlags, fchmod, fstat, open, openat, unlinkat};
use rustix::io::Errno;
use std::path::Path;

const DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

pub(super) fn remove(path: &Path) -> Result<()> {
    remove_checked(path, &|| Ok(()))
}

pub(super) fn remove_checked(path: &Path, checkpoint: &impl Fn() -> Result<()>) -> Result<()> {
    let parent = match open(
        path.parent()
            .ok_or_else(|| anyhow::anyhow!("projection parent missing"))?,
        DIRECTORY,
        Mode::empty(),
    ) {
        Ok(parent) => parent,
        Err(Errno::NOENT) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("projection name missing"))?;
    let device = fstat(&parent)?.st_dev;
    remove_entry(&parent, name, device, 0, checkpoint)
}

/// Remove `leaf` below every real `accounts/<account>/providers/<provider>`
/// directory of `base` whose account name `account` admits (only `provider`
/// when given). Traversal runs
/// from opened descriptors and never follows a symlink at any level; a
/// symlinked or non-directory level is skipped, a symlinked `leaf` only
/// unlinked. Enumeration failures propagate.
pub(super) fn remove_below_named_providers(
    base: &Path,
    account: &dyn Fn(&str) -> bool,
    leaf: &str,
    provider: Option<&str>,
) -> Result<()> {
    let base = match open(base, DIRECTORY, Mode::empty()) {
        Ok(base) => base,
        Err(Errno::NOENT) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let Some(accounts) = real_child(&base, "accounts")? else {
        return Ok(());
    };
    for name in entries(&accounts)? {
        if !name.to_str().is_ok_and(account) {
            continue;
        }
        let Some(root) = real_child(&accounts, name.as_c_str())? else {
            continue;
        };
        let Some(providers) = real_child(&root, "providers")? else {
            continue;
        };
        for name in entries(&providers)? {
            if provider.is_some_and(|only| name.to_bytes() != only.as_bytes()) {
                continue;
            }
            let Some(provider) = real_child(&providers, name.as_c_str())? else {
                continue;
            };
            let device = fstat(&provider)?.st_dev;
            remove_entry(&provider, leaf, device, 0, &|| Ok(()))?;
        }
    }
    Ok(())
}

/// A real child directory, or `None` when it is absent, a symlink or not a
/// directory.
fn real_child(
    parent: impl AsFd,
    name: impl rustix::path::Arg,
) -> Result<Option<rustix::fd::OwnedFd>> {
    match openat(parent, name, DIRECTORY, Mode::empty()) {
        Ok(directory) => Ok(Some(directory)),
        Err(Errno::NOENT | Errno::NOTDIR | Errno::LOOP) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// The names in a directory, excluding `.` and `..`.
fn entries(directory: impl AsFd) -> Result<Vec<std::ffi::CString>> {
    let mut names = Vec::new();
    for entry in Dir::read_from(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_bytes() != b"." && name.to_bytes() != b".." {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

fn remove_entry(
    parent: impl AsFd,
    name: impl rustix::path::Arg,
    device: rustix::fs::Dev,
    depth: usize,
    checkpoint: &impl Fn() -> Result<()>,
) -> Result<()> {
    checkpoint()?;
    ensure!(depth < 256, "projection nesting limit exceeded");
    let name = name.into_c_str()?;
    let name = name.as_ref();
    {
        let directory = match openat(&parent, name, DIRECTORY, Mode::empty()) {
            Ok(directory) => directory,
            Err(Errno::NOENT) => return Ok(()),
            Err(Errno::NOTDIR | Errno::LOOP) => {
                // A symlink is never opened or chmod'ed, even when dangling.
                unlinkat(parent, name, AtFlags::empty())?;
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        let metadata = fstat(&directory)?;
        ensure!(
            metadata.st_dev == device,
            "projection crosses a filesystem boundary"
        );
        let mode = Mode::from_raw_mode(metadata.st_mode);
        fchmod(&directory, mode | Mode::RWXU)?;
        for entry in Dir::read_from(&directory)? {
            let entry = entry?;
            let child = entry.file_name();
            if child.to_bytes() == b"." || child.to_bytes() == b".." {
                continue;
            }
            remove_entry(&directory, child, device, depth + 1, checkpoint)?;
        }
        checkpoint()?;
        unlinkat(parent, name, AtFlags::REMOVEDIR)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn removes_readonly_go_cache_without_touching_external_links() {
        let root = tempfile::tempdir().unwrap();
        let external = root.path().join("external");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("secret"), b"unread").unwrap();
        fs::set_permissions(external.join("secret"), fs::Permissions::from_mode(0o400)).unwrap();
        let projection = root.path().join("runtime");
        let cache = projection.join("home/go/pkg/mod/example@v1");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("source"), b"cache").unwrap();
        fs::hard_link(external.join("secret"), cache.join("hardlink")).unwrap();
        symlink(&external, cache.join("symlink")).unwrap();
        symlink("missing", cache.join("dangling")).unwrap();
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o555)).unwrap();
        remove(&projection).unwrap();
        assert!(!projection.exists());
        assert_eq!(fs::read(external.join("secret")).unwrap(), b"unread");
        assert_eq!(
            fs::metadata(external.join("secret"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o400
        );
        remove(&projection).unwrap();
    }

    #[test]
    fn root_symlink_is_unlinked_without_following_it() {
        let root = tempfile::tempdir().unwrap();
        let external = root.path().join("external");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("keep"), b"keep").unwrap();
        let projection = root.path().join("runtime");
        symlink(&external, &projection).unwrap();
        remove(&projection).unwrap();
        assert!(projection.symlink_metadata().is_err());
        assert!(external.join("keep").exists());
    }
}
