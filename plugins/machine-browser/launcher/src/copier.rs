//! The no-follow, bounded profile copier used for restoring a snapshot into
//! the profile tmpfs and for copying the profile into a staging generation.
//! Every entry is opened descriptor-relative with `O_NOFOLLOW`; only regular
//! files and directories are copied; caches and singleton files are skipped
//! by name; holes are preserved; the copy stops at the byte, entry and depth
//! bounds. The tree it reads is treated as untrusted input.

use std::fs::File;
use std::os::fd::AsFd;

use anyhow::{Context as _, Result, bail, ensure};
use rustix::fs::{
    AtFlags, Dir, FileType, Mode, OFlags, SeekFrom, copy_file_range, fstat, fsync, ftruncate,
    mkdirat, openat, seek, statat,
};
use rustix::io::Errno;

/// Entries one copy may hold; matches the Machine's validation bound.
pub const MAX_ENTRIES: u64 = 100_000;
/// Directory depth one copy may hold; matches the Machine's validation bound.
pub const MAX_DEPTH: u32 = 32;

const DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const SOURCE_FILE: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOCTTY);
const TARGET_FILE: OFlags = OFlags::WRONLY
    .union(OFlags::CREATE)
    .union(OFlags::EXCL)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const CHUNK: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// Logical bytes of regular files.
    pub bytes: u64,
    pub entries: u64,
    pub depth: u32,
}

impl Bounds {
    #[must_use]
    pub fn for_profile(profile_bytes: u64) -> Self {
        Self {
            bytes: profile_bytes,
            entries: MAX_ENTRIES,
            depth: MAX_DEPTH,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Logical bytes of the copied regular files.
    pub bytes: u64,
    /// Copied files and directories.
    pub entries: u64,
}

/// Names skipped at any depth. Directories: caches Chromium rebuilds and
/// in-flight blob storage. Files: the process singleton and `Local State`
/// lock, which only describe the dead incarnation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exclusions {
    directories: &'static [&'static str],
    files: &'static [&'static str],
}

impl Exclusions {
    #[must_use]
    pub const fn chromium() -> Self {
        Self {
            directories: &[
                "Cache",
                "Code Cache",
                "GPUCache",
                "ShaderCache",
                "GrShaderCache",
                "DawnCache",
                "DawnGraphiteCache",
                "DawnWebGPUCache",
                "GraphiteDawnCache",
                "blob_storage",
                "Crashpad",
            ],
            files: &[
                "SingletonLock",
                "SingletonSocket",
                "SingletonCookie",
                "lockfile",
            ],
        }
    }

    #[cfg(test)]
    pub const fn none() -> Self {
        Self {
            directories: &[],
            files: &[],
        }
    }

    #[must_use]
    pub fn skips_directory(&self, name: &str) -> bool {
        self.directories.contains(&name)
    }

    #[must_use]
    pub fn skips_file(&self, name: &str) -> bool {
        self.files.contains(&name)
    }
}

/// Copy the tree below `source` into the (empty) directory `target`.
pub fn copy_tree(
    source: &File,
    target: &File,
    bounds: Bounds,
    exclusions: &Exclusions,
    sync: bool,
) -> Result<Stats> {
    let mut stats = Stats::default();
    copy_directory(source, target, 0, bounds, exclusions, sync, &mut stats)?;
    if sync {
        fsync(target).context("syncing the copy root")?;
    }
    Ok(stats)
}

fn copy_directory(
    source: &File,
    target: &File,
    depth: u32,
    bounds: Bounds,
    exclusions: &Exclusions,
    sync: bool,
    stats: &mut Stats,
) -> Result<()> {
    ensure!(depth <= bounds.depth, "copy tree is too deep");
    for name in child_names(source)? {
        let stat = statat(source, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
            .with_context(|| format!("inspecting {name}"))?;
        match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => {
                if exclusions.skips_directory(&name) {
                    continue;
                }
                count_entry(stats, bounds)?;
                let child_source: File = openat(source, name.as_str(), DIRECTORY, Mode::empty())
                    .with_context(|| format!("opening directory {name}"))?
                    .into();
                let opened = fstat(&child_source)?;
                ensure!(
                    opened.st_ino == stat.st_ino && opened.st_dev == stat.st_dev,
                    "directory {name} changed while it was copied"
                );
                mkdirat(target, name.as_str(), Mode::from_raw_mode(0o700))
                    .with_context(|| format!("creating directory {name}"))?;
                let child_target: File = openat(target, name.as_str(), DIRECTORY, Mode::empty())
                    .with_context(|| format!("opening created directory {name}"))?
                    .into();
                copy_directory(
                    &child_source,
                    &child_target,
                    depth + 1,
                    bounds,
                    exclusions,
                    sync,
                    stats,
                )?;
                if sync {
                    fsync(&child_target).with_context(|| format!("syncing directory {name}"))?;
                }
            }
            FileType::RegularFile => {
                if exclusions.skips_file(&name) {
                    continue;
                }
                count_entry(stats, bounds)?;
                let size = u64::try_from(stat.st_size).unwrap_or(u64::MAX);
                stats.bytes = stats.bytes.saturating_add(size);
                ensure!(
                    stats.bytes <= bounds.bytes,
                    "copy exceeds its byte bound at {name}"
                );
                let file_source: File = openat(source, name.as_str(), SOURCE_FILE, Mode::empty())
                    .with_context(|| format!("opening {name}"))?
                    .into();
                let opened = fstat(&file_source)?;
                ensure!(
                    opened.st_ino == stat.st_ino
                        && opened.st_dev == stat.st_dev
                        && FileType::from_raw_mode(opened.st_mode) == FileType::RegularFile,
                    "file {name} changed while it was copied"
                );
                let file_target: File = openat(
                    target,
                    name.as_str(),
                    TARGET_FILE,
                    Mode::from_raw_mode(0o600),
                )
                .with_context(|| format!("creating {name}"))?
                .into();
                copy_contents(&file_source, &file_target, size)
                    .with_context(|| format!("copying {name}"))?;
                if sync {
                    fsync(&file_target).with_context(|| format!("syncing {name}"))?;
                }
            }
            // Symlinks (including Chromium's singleton links), sockets, FIFOs
            // and device nodes are never copied.
            _ => {}
        }
    }
    Ok(())
}

fn count_entry(stats: &mut Stats, bounds: Bounds) -> Result<()> {
    stats.entries += 1;
    ensure!(
        stats.entries <= bounds.entries,
        "copy exceeds its entry bound"
    );
    Ok(())
}

/// Copy `size` logical bytes, skipping holes. The source is re-measured as
/// it is read so a file that grows after `statat` cannot exceed the bound by
/// more than its reported size: anything past `size` is not copied.
fn copy_contents(source: &File, target: &File, size: u64) -> Result<()> {
    let mut position = 0_u64;
    let mut buffer = vec![0_u8; CHUNK];
    while position < size {
        let data_start = match seek(source, SeekFrom::Data(position)) {
            Ok(offset) => offset,
            Err(Errno::NXIO) => break,
            Err(Errno::INVAL | Errno::NOTSUP) => {
                // No hole support on this file system: copy linearly.
                copy_range(source, target, position, size, &mut buffer)?;
                break;
            }
            Err(error) => return Err(error).context("seeking data"),
        };
        if data_start >= size {
            break;
        }
        let hole_start = match seek(source, SeekFrom::Hole(data_start)) {
            Ok(offset) => offset.min(size),
            Err(Errno::NXIO | Errno::INVAL | Errno::NOTSUP) => size,
            Err(error) => return Err(error).context("seeking hole"),
        };
        if hole_start <= data_start {
            // No hole support on this file system: copy linearly to the end.
            copy_range(source, target, data_start, size, &mut buffer)?;
            break;
        }
        copy_range(source, target, data_start, hole_start, &mut buffer)?;
        position = hole_start;
    }
    ftruncate(target, size).context("setting the copied length")?;
    Ok(())
}

fn copy_range(source: &File, target: &File, start: u64, end: u64, buffer: &mut [u8]) -> Result<()> {
    let mut offset_in = start;
    let mut offset_out = start;
    while offset_in < end {
        let remaining = usize::try_from(end - offset_in).unwrap_or(usize::MAX);
        match copy_file_range(
            source,
            Some(&mut offset_in),
            target,
            Some(&mut offset_out),
            remaining,
        ) {
            Ok(0) => break,
            Ok(_) => continue,
            Err(Errno::XDEV | Errno::INVAL | Errno::NOSYS | Errno::OPNOTSUPP | Errno::TXTBSY) => {}
            Err(error) => return Err(error).context("copy_file_range"),
        }
        let want = remaining.min(buffer.len());
        let read =
            rustix::io::pread(source, &mut buffer[..want], offset_in).context("reading source")?;
        if read == 0 {
            break;
        }
        let mut written = 0;
        while written < read {
            written +=
                rustix::io::pwrite(target, &buffer[written..read], offset_out + written as u64)
                    .context("writing target")?;
        }
        offset_in += read as u64;
        offset_out += read as u64;
    }
    Ok(())
}

fn child_names(directory: &File) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in Dir::read_from(directory.as_fd()).context("reading directory")? {
        let entry = entry.context("reading directory entry")?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}

/// Open a directory for copying (no symlink following).
pub fn open_directory(path: &str) -> Result<File> {
    let fd = rustix::fs::open(path, DIRECTORY, Mode::empty())
        .with_context(|| format!("opening directory {path}"))?;
    Ok(fd.into())
}

/// Refuse a copy whose root is not a plain directory.
pub fn ensure_directory(file: &File, what: &str) -> Result<()> {
    let stat = fstat(file)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
        bail!("{what} is not a directory");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek as _, SeekFrom as IoSeek, Write as _};
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;
    use std::path::Path;

    fn open(path: &Path) -> File {
        open_directory(path.to_str().unwrap()).unwrap()
    }

    fn names(root: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                found.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
                if entry.file_type().unwrap().is_dir() {
                    pending.push(path);
                }
            }
        }
        found.sort();
        found
    }

    #[test]
    fn copies_files_and_directories_only_and_skips_chromium_caches() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("profile");
        std::fs::create_dir_all(source.join("Default/Cache")).unwrap();
        std::fs::create_dir_all(source.join("Default/Code Cache/js")).unwrap();
        std::fs::create_dir_all(source.join("Default/IndexedDB")).unwrap();
        std::fs::create_dir_all(source.join("GrShaderCache")).unwrap();
        std::fs::write(source.join("Local State"), b"{}").unwrap();
        std::fs::write(source.join("lockfile"), b"x").unwrap();
        std::fs::write(source.join("Default/Cookies"), vec![7_u8; 3000]).unwrap();
        std::fs::write(source.join("Default/Cookies-journal"), b"j").unwrap();
        std::fs::write(source.join("Default/IndexedDB/MANIFEST-000001"), b"m").unwrap();
        std::fs::write(source.join("Default/IndexedDB/000003.ldb"), b"ldb").unwrap();
        std::fs::write(source.join("Default/Cache/data_0"), b"cache").unwrap();
        symlink("/etc/passwd", source.join("SingletonLock")).unwrap();
        symlink("Default", source.join("escape-dir")).unwrap();
        UnixListener::bind(source.join("SingletonSocket")).unwrap();
        rustix::fs::mknodat(
            rustix::fs::CWD,
            source.join("fifo").to_str().unwrap(),
            FileType::Fifo,
            Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        // A sparse file: 1 MiB hole, then data.
        let mut sparse = File::create(source.join("Default/sparse")).unwrap();
        sparse.seek(IoSeek::Start(1 << 20)).unwrap();
        sparse.write_all(b"tail").unwrap();
        drop(sparse);

        let target = temp.path().join("gen-1");
        std::fs::create_dir(&target).unwrap();
        let stats = copy_tree(
            &open(&source),
            &open(&target),
            Bounds::for_profile(8 << 20),
            &Exclusions::chromium(),
            true,
        )
        .unwrap();
        assert_eq!(
            names(&target),
            vec![
                "Default",
                "Default/Cookies",
                "Default/Cookies-journal",
                "Default/IndexedDB",
                "Default/IndexedDB/000003.ldb",
                "Default/IndexedDB/MANIFEST-000001",
                "Default/sparse",
                "Local State",
            ]
        );
        assert_eq!(stats.entries, 8);
        assert_eq!(stats.bytes, 2 + 3000 + 1 + 1 + 3 + (1 << 20) + 4);
        let copied = std::fs::read(target.join("Default/sparse")).unwrap();
        assert_eq!(copied.len(), (1 << 20) + 4);
        assert_eq!(&copied[1 << 20..], b"tail");
        assert!(copied[..1 << 20].iter().all(|byte| *byte == 0));
        assert_eq!(
            std::fs::read(target.join("Default/Cookies")).unwrap(),
            vec![7_u8; 3000]
        );
        assert!(std::fs::symlink_metadata(target.join("SingletonLock")).is_err());
    }

    #[test]
    fn bounds_stop_the_copy() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("src");
        std::fs::create_dir_all(source.join("a/b/c")).unwrap();
        std::fs::write(source.join("a/b/c/file"), vec![1_u8; 4096]).unwrap();
        std::fs::write(source.join("other"), vec![1_u8; 4096]).unwrap();
        let fresh = |name: &str| {
            let target = temp.path().join(name);
            std::fs::create_dir(&target).unwrap();
            open(&target)
        };
        let bounds = Bounds::for_profile(1 << 20);
        assert!(
            copy_tree(
                &open(&source),
                &fresh("bytes"),
                Bounds {
                    bytes: 5000,
                    ..bounds
                },
                &Exclusions::none(),
                false
            )
            .is_err()
        );
        assert!(
            copy_tree(
                &open(&source),
                &fresh("entries"),
                Bounds {
                    entries: 2,
                    ..bounds
                },
                &Exclusions::none(),
                false
            )
            .is_err()
        );
        assert!(
            copy_tree(
                &open(&source),
                &fresh("depth"),
                Bounds { depth: 1, ..bounds },
                &Exclusions::none(),
                false
            )
            .is_err()
        );
        let stats = copy_tree(
            &open(&source),
            &fresh("ok"),
            bounds,
            &Exclusions::none(),
            false,
        )
        .unwrap();
        assert_eq!(stats.entries, 5);
        assert_eq!(stats.bytes, 8192);
    }

    #[test]
    fn exclusion_names_match_the_documented_list() {
        let exclusions = Exclusions::chromium();
        for directory in [
            "Cache",
            "Code Cache",
            "GPUCache",
            "ShaderCache",
            "GrShaderCache",
            "DawnCache",
            "blob_storage",
            "Crashpad",
        ] {
            assert!(exclusions.skips_directory(directory), "{directory}");
        }
        for file in [
            "SingletonLock",
            "SingletonSocket",
            "SingletonCookie",
            "lockfile",
        ] {
            assert!(exclusions.skips_file(file), "{file}");
        }
        assert!(!exclusions.skips_directory("IndexedDB"));
        assert!(!exclusions.skips_file("Cookies-journal"));
        assert!(!exclusions.skips_file("MANIFEST-000001"));
        assert!(!exclusions.skips_directory("Local Storage"));
    }
}
