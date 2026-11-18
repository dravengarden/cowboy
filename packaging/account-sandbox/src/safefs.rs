//! Privileged traversal (amendment R3-2): every path inside an account tree
//! or the runtime store is opened with `openat2(RESOLVE_BENEATH |
//! RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV)` relative to
//! a directory descriptor, never by an absolute pathname. Names given to the
//! single-component helpers must not contain `/`.

use std::io::{Read as _, Write as _};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use anyhow::{Context as _, Result, bail, ensure};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags};
use rustix::io::Errno;

/// The resolution every account-tree open uses.
pub const STRICT: ResolveFlags = ResolveFlags::BENEATH
    .union(ResolveFlags::NO_SYMLINKS)
    .union(ResolveFlags::NO_MAGICLINKS)
    .union(ResolveFlags::NO_XDEV);

/// [`STRICT`] without `NO_XDEV`, used only to open a mount point itself (the
/// account home image); everything beneath it uses [`STRICT`] again.
pub const MOUNT_ROOT: ResolveFlags = ResolveFlags::BENEATH
    .union(ResolveFlags::NO_SYMLINKS)
    .union(ResolveFlags::NO_MAGICLINKS);

/// Open a directory below `dir`.
pub fn open_dir(dir: BorrowedFd<'_>, path: &str, resolve: ResolveFlags) -> Result<OwnedFd> {
    rustix::fs::openat2(
        dir,
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        resolve,
    )
    .with_context(|| format!("open directory {path}"))
}

/// Open the root directory of the tree as the anchor of later `openat2` calls.
pub fn open_root(path: &std::path::Path) -> Result<OwnedFd> {
    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .with_context(|| format!("open {}", path.display()))
}

fn check_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name != "."
            && name != ".."
            && !name.contains('/')
            && !name.contains('\0'),
        "invalid path component {name:?}"
    );
    Ok(())
}

/// Whether `path` exists below `dir`; a symlink or a mount crossing anywhere
/// on the way is an error, not "absent".
pub fn exists(dir: BorrowedFd<'_>, path: &str) -> Result<bool> {
    match rustix::fs::openat2(
        dir,
        path,
        OFlags::PATH | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        STRICT,
    ) {
        Ok(_) => Ok(true),
        Err(Errno::NOENT) => Ok(false),
        Err(error) => Err(error).with_context(|| format!("resolve {path}")),
    }
}

/// Create (if absent) and open directory `name` in `dir`, then enforce its
/// mode and owner. An existing non-directory or symlink is an error.
pub fn ensure_dir(
    dir: BorrowedFd<'_>,
    name: &str,
    mode: u32,
    owner: Option<(u32, u32)>,
) -> Result<OwnedFd> {
    check_name(name)?;
    match rustix::fs::mkdirat(dir, name, Mode::from_raw_mode(mode)) {
        // A new entry is durable before anything inside it is relied on.
        Ok(()) => rustix::fs::fsync(dir).context("fsync parent directory")?,
        Err(Errno::EXIST) => {}
        Err(error) => return Err(error).with_context(|| format!("mkdir {name}")),
    }
    let fd = open_dir(dir, name, STRICT)?;
    apply_owner_mode(&fd, mode, owner)?;
    Ok(fd)
}

fn apply_owner_mode(fd: &OwnedFd, mode: u32, owner: Option<(u32, u32)>) -> Result<()> {
    if let Some((uid, gid)) = owner {
        let current = rustix::fs::fstat(fd)?;
        if current.st_uid != uid || current.st_gid != gid {
            rustix::fs::fchown(
                fd,
                Some(rustix::fs::Uid::from_raw(uid)),
                Some(rustix::fs::Gid::from_raw(gid)),
            )
            .context("fchown")?;
        }
    }
    rustix::fs::fchmod(fd, Mode::from_raw_mode(mode)).context("fchmod")?;
    Ok(())
}

/// Enforce mode and owner of an already open descriptor.
pub fn set_owner_mode(fd: &OwnedFd, mode: u32, owner: Option<(u32, u32)>) -> Result<()> {
    apply_owner_mode(fd, mode, owner)
}

/// Ensure a regular file `name` exists in `dir` (created empty when absent)
/// with the given mode and owner. Opening never blocks on a FIFO.
pub fn ensure_file(
    dir: BorrowedFd<'_>,
    name: &str,
    mode: u32,
    owner: Option<(u32, u32)>,
) -> Result<()> {
    check_name(name)?;
    let fd = rustix::fs::openat2(
        dir,
        name,
        OFlags::RDONLY
            | OFlags::CREATE
            | OFlags::NONBLOCK
            | OFlags::NOCTTY
            | OFlags::CLOEXEC
            | OFlags::NOFOLLOW,
        Mode::from_raw_mode(mode),
        STRICT,
    )
    .with_context(|| format!("open {name}"))?;
    let stat = rustix::fs::fstat(&fd)?;
    ensure!(
        FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile,
        "{name} is not a regular file"
    );
    apply_owner_mode(&fd, mode, owner)
}

/// Open a regular file below `dir` for reading; `None` when absent. Returns
/// the file and its mode bits.
pub fn open_regular(dir: BorrowedFd<'_>, path: &str) -> Result<Option<(std::fs::File, u32)>> {
    let fd = match rustix::fs::openat2(
        dir,
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        STRICT,
    ) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("open {path}")),
    };
    let stat = rustix::fs::fstat(&fd)?;
    ensure!(
        FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile,
        "{path} is not a regular file"
    );
    Ok(Some((std::fs::File::from(fd), stat.st_mode & 0o7777)))
}

/// Read a regular file below `dir`, at most `max` bytes; `None` when absent.
pub fn read_file(dir: BorrowedFd<'_>, path: &str, max: u64) -> Result<Option<Vec<u8>>> {
    let Some((file, _)) = open_regular(dir, path)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= max, "{path} exceeds {max} bytes");
    Ok(Some(bytes))
}

/// Create a new file `name` in `dir` (fails if it exists).
pub fn create_new(dir: BorrowedFd<'_>, name: &str, mode: u32) -> Result<std::fs::File> {
    check_name(name)?;
    let fd = rustix::fs::openat2(
        dir,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(mode),
        STRICT,
    )
    .with_context(|| format!("create {name}"))?;
    rustix::fs::fchmod(&fd, Mode::from_raw_mode(mode))?;
    Ok(std::fs::File::from(fd))
}

/// Atomically replace `name` in `dir`: write a temporary file, fsync it,
/// rename it over `name` and fsync the directory.
pub fn write_atomic(dir: BorrowedFd<'_>, name: &str, bytes: &[u8], mode: u32) -> Result<()> {
    check_name(name)?;
    let temporary = format!(".{name}.tmp-{}", nonce(8)?);
    let result = (|| -> Result<()> {
        let mut file = create_new(dir, &temporary, mode)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        rustix::fs::renameat(dir, &temporary, dir, name)
            .with_context(|| format!("rename {name}"))?;
        rustix::fs::fsync(dir).context("fsync directory")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = rustix::fs::unlinkat(dir, &temporary, AtFlags::empty());
    }
    result
}

/// Remove the non-directory `name` from `dir`; `false` when it was absent.
pub fn remove_file(dir: BorrowedFd<'_>, name: &str) -> Result<bool> {
    check_name(name)?;
    match rustix::fs::unlinkat(dir, name, AtFlags::empty()) {
        Ok(()) => Ok(true),
        Err(Errno::NOENT) => Ok(false),
        Err(error) => Err(error).with_context(|| format!("unlink {name}")),
    }
}

/// Remove `name` (a file or a whole directory tree) from `dir` without ever
/// following a symlink or crossing a mount; `false` when it was absent. The
/// removal is durable (`dir` is synced, also when `name` was already absent,
/// so a replay after a crash makes an earlier removal durable).
pub fn remove_tree(dir: BorrowedFd<'_>, name: &str) -> Result<bool> {
    let removed = remove_tree_entry(dir, name)?;
    rustix::fs::fsync(dir).context("fsync directory")?;
    Ok(removed)
}

fn remove_tree_entry(dir: BorrowedFd<'_>, name: &str) -> Result<bool> {
    check_name(name)?;
    let stat = match rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => stat,
        Err(Errno::NOENT) => return Ok(false),
        Err(error) => return Err(error).with_context(|| format!("stat {name}")),
    };
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
        return remove_file(dir, name);
    }
    let child = open_dir(dir, name, STRICT)?;
    loop {
        let names = list(child.as_fd())?;
        if names.is_empty() {
            break;
        }
        for entry in names {
            remove_tree_entry(child.as_fd(), &entry)?;
        }
    }
    drop(child);
    rustix::fs::unlinkat(dir, name, AtFlags::REMOVEDIR).with_context(|| format!("rmdir {name}"))?;
    Ok(true)
}

/// Entry names of the directory `dir` (without `.` and `..`).
pub fn list(dir: BorrowedFd<'_>) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in rustix::fs::Dir::read_from(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        match std::str::from_utf8(name) {
            Ok(name) => names.push(name.to_owned()),
            Err(_) => bail!("non-UTF-8 directory entry"),
        }
    }
    names.sort();
    Ok(names)
}

/// `bytes` random bytes as lowercase hex.
pub fn nonce(bytes: usize) -> Result<String> {
    let mut buffer = vec![0_u8; bytes];
    let mut filled = 0;
    while filled < bytes {
        filled +=
            rustix::rand::getrandom(&mut buffer[filled..], rustix::rand::GetRandomFlags::empty())?;
    }
    Ok(hex::encode(buffer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt as _, symlink};

    #[test]
    fn symlink_attacks_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"token").unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        symlink(outside.path(), root.join("link")).unwrap();
        symlink(outside.path().join("secret"), root.join("file-link")).unwrap();
        let fd = open_root(&root).unwrap();
        assert!(read_file(fd.as_fd(), "link/secret", 100).is_err());
        assert!(read_file(fd.as_fd(), "file-link", 100).is_err());
        assert!(read_file(fd.as_fd(), "../root/x", 100).is_err());
        assert!(open_dir(fd.as_fd(), "link", STRICT).is_err());
        assert!(exists(fd.as_fd(), "link/secret").is_err());
        assert!(ensure_dir(fd.as_fd(), "link", 0o700, None).is_err());
        assert!(ensure_file(fd.as_fd(), "file-link", 0o600, None).is_err());
        // Removing a symlink removes only the link, never its target.
        assert!(remove_tree(fd.as_fd(), "link").unwrap());
        assert!(remove_file(fd.as_fd(), "file-link").unwrap());
        assert_eq!(
            std::fs::read(outside.path().join("secret")).unwrap(),
            b"token"
        );
    }

    #[test]
    fn absolute_paths_escape_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let fd = open_root(temp.path()).unwrap();
        assert!(read_file(fd.as_fd(), "/etc/passwd", 1 << 20).is_err());
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temporary() {
        let temp = tempfile::tempdir().unwrap();
        let fd = open_root(temp.path()).unwrap();
        write_atomic(fd.as_fd(), "meta.json", b"one", 0o600).unwrap();
        write_atomic(fd.as_fd(), "meta.json", b"two", 0o600).unwrap();
        assert_eq!(
            read_file(fd.as_fd(), "meta.json", 10).unwrap().unwrap(),
            b"two"
        );
        assert_eq!(list(fd.as_fd()).unwrap(), ["meta.json"]);
        let mode = std::fs::metadata(temp.path().join("meta.json"))
            .unwrap()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(read_file(fd.as_fd(), "meta.json", 2).is_err());
    }

    #[test]
    fn remove_tree_deletes_nested_content() {
        let temp = tempfile::tempdir().unwrap();
        let fd = open_root(temp.path()).unwrap();
        let a = ensure_dir(fd.as_fd(), "a", 0o711, None).unwrap();
        let b = ensure_dir(a.as_fd(), "b", 0o700, None).unwrap();
        ensure_file(b.as_fd(), "c", 0o600, None).unwrap();
        assert!(exists(fd.as_fd(), "a/b/c").unwrap());
        assert!(remove_tree(fd.as_fd(), "a").unwrap());
        assert!(!exists(fd.as_fd(), "a").unwrap());
        assert!(!remove_tree(fd.as_fd(), "a").unwrap());
    }
}
