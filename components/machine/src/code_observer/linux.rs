//! Kernel-backed, non-projecting observation of a caller-pinned directory.
//! The caller still owns current grant/root admission. No pathname fallback.
//! Coverage is limited to inotify filesystem events, never content equality:
//! mmap modifications and remote network-filesystem events are not reported.
//! See https://man7.org/linux/man-pages/man7/inotify.7.html (limitations).
use super::{Gap, Journal};
use anyhow::{Context as _, Result, ensure};
use rustix::fs::{Mode, OFlags, ResolveFlags, inotify, openat2};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read as _,
    os::fd::AsRawFd as _,
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

mod scan;
#[cfg(test)]
mod tests;

const MAX_WATCHES: usize = 16_384;
const MAX_PATH_BYTES: usize = 8 * 1024 * 1024;
const MAX_EVENTS: usize = 4096;
const MASK: u32 = libc::IN_MODIFY
    | libc::IN_ATTRIB
    | libc::IN_CLOSE_WRITE
    | libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF
    | libc::IN_UNMOUNT;

struct Watch {
    paths: Vec<PathBuf>,
    directory: bool,
}

pub struct Source {
    fd: File,
    root: File,
    requested: PathBuf,
    mounts: [u8; 32],
    watches: BTreeMap<i32, Watch>,
    path_bytes: usize,
    invalid: bool,
}

fn mounts() -> Result<[u8; 32]> {
    let mut bytes = Vec::new();
    File::open("/proc/self/mountinfo")?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 1024 * 1024,
        "observer mount table exceeds budget"
    );
    Ok(Sha256::digest(bytes).into())
}

impl Source {
    /// `root` must be the handle retained by current Session root admission.
    /// This constructor cannot mint or renew a grant or root incarnation.
    pub fn new(root: File, requested: PathBuf, journal: &mut Journal) -> Result<Self> {
        journal.gap(Gap::Coverage);
        let fd =
            inotify::init(inotify::CreateFlags::NONBLOCK | inotify::CreateFlags::CLOEXEC)?.into();
        let mut source = Self {
            fd,
            root,
            requested,
            mounts: mounts()?,
            watches: BTreeMap::new(),
            path_bytes: 0,
            invalid: false,
        };
        source.current()?;
        let stamp = journal.begin_scan()?;
        source.scan()?;
        source.drain(journal)?;
        ensure!(!source.invalid, "observer coverage changed during scan");
        journal.finish_scan(stamp)?;
        Ok(source)
    }

    fn current(&self) -> Result<()> {
        let original = self.root.metadata()?;
        let current = std::fs::metadata(&self.requested)?;
        ensure!(
            original.is_dir()
                && current.is_dir()
                && original.dev() == current.dev()
                && original.ino() == current.ino()
                && mounts()? == self.mounts,
            "observer root or mount namespace changed"
        );
        Ok(())
    }

    fn open(&self, relative: &Path, directory: bool) -> Result<File> {
        let path = if relative.as_os_str().is_empty() {
            Path::new(".")
        } else {
            relative
        };
        let flags = if directory {
            OFlags::RDONLY | OFlags::DIRECTORY
        } else {
            OFlags::PATH
        };
        Ok(openat2(
            &self.root,
            path,
            flags | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
        )?
        .into())
    }

    fn watch(&mut self, file: &File, path: PathBuf, directory: bool) -> Result<()> {
        ensure!(
            self.watches.len() < MAX_WATCHES
                && self.path_bytes + path.as_os_str().len() <= MAX_PATH_BYTES,
            "observer watch budget exhausted"
        );
        // /proc/self/fd resolves only this live descriptor, never a repo link.
        let pinned = format!("/proc/self/fd/{}", file.as_raw_fd());
        let mask = MASK | libc::IN_MASK_ADD | if directory { libc::IN_ONLYDIR } else { 0 };
        let wd = inotify::add_watch(
            &self.fd,
            pinned,
            inotify::WatchFlags::from_bits_retain(mask),
        )?;
        let watch = self.watches.entry(wd).or_insert_with(|| Watch {
            paths: Vec::new(),
            directory,
        });
        ensure!(
            watch.directory == directory && watch.paths.len() < 64,
            "observer alias budget exhausted"
        );
        if !watch.paths.contains(&path) {
            self.path_bytes += path.as_os_str().len();
            watch.paths.push(path);
        }
        Ok(())
    }

    /// Drain to the kernel's EAGAIN boundary. Structural ambiguity deliberately
    /// requires a new bounded scan; it cannot yield a successful empty page.
    pub fn drain(&mut self, journal: &mut Journal) -> Result<usize> {
        if self.current().is_err() {
            self.invalid = true;
            journal.gap(Gap::RootChanged);
        }
        ensure!(!self.invalid, "observer source requires reconciliation");
        let mut buffer = [0_u8; 64 * 1024];
        let mut events = 0;
        loop {
            let bytes = match self.fd.read(&mut buffer) {
                Ok(0) => {
                    self.invalid = true;
                    journal.gap(Gap::Disconnected);
                    anyhow::bail!("observer event source ended");
                }
                Ok(n) => n,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    self.invalid = true;
                    journal.gap(Gap::Coverage);
                    return Err(error.into());
                }
            };
            let mut offset = 0;
            while offset < bytes {
                if events == MAX_EVENTS {
                    self.invalid = true;
                    journal.gap(Gap::Budget);
                    anyhow::bail!("observer event burst exceeds budget");
                }
                let result = self.event(&buffer[offset..bytes], journal);
                match result {
                    Ok(length) => offset += length,
                    Err(error) => {
                        self.invalid = true;
                        journal.gap(Gap::Coverage);
                        return Err(error);
                    }
                }
                events += 1;
                ensure!(
                    !self.invalid,
                    "observer event coverage requires reconciliation"
                );
            }
        }
        if self.current().is_err() {
            self.invalid = true;
            journal.gap(Gap::RootChanged);
        }
        ensure!(!self.invalid, "observer root changed while draining");
        Ok(events)
    }

    fn event(&mut self, bytes: &[u8], journal: &mut Journal) -> Result<usize> {
        ensure!(bytes.len() >= 16, "truncated observer event");
        let wd = i32::from_ne_bytes(bytes[..4].try_into()?);
        let mask = u32::from_ne_bytes(bytes[4..8].try_into()?);
        let length = usize::try_from(u32::from_ne_bytes(bytes[12..16].try_into()?))?;
        ensure!(length <= bytes.len() - 16, "truncated observer event name");
        if mask & libc::IN_Q_OVERFLOW != 0 {
            self.invalid = true;
            journal.gap(Gap::QueueOverflow);
            return Ok(16 + length);
        }
        if mask & (libc::IN_IGNORED | libc::IN_UNMOUNT | libc::IN_MOVE_SELF | libc::IN_DELETE_SELF)
            != 0
        {
            self.invalid = true;
            journal.gap(Gap::Coverage);
            return Ok(16 + length);
        }
        let watch = self.watches.get(&wd).context("unknown observer watch")?;
        let raw = bytes[16..16 + length]
            .split(|b| *b == 0)
            .next()
            .unwrap_or_default();
        let name = std::str::from_utf8(raw).context("observer name is not UTF-8")?;
        let paths = watch
            .paths
            .iter()
            .map(|path| {
                let path = if watch.directory && !name.is_empty() {
                    path.join(name)
                } else {
                    path.clone()
                };
                path.into_os_string()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("observer path is not UTF-8"))
            })
            .collect::<Result<Vec<_>>>()?;
        journal.record(paths)?;
        if mask & (libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_MOVED_TO) != 0
        {
            self.invalid = true;
            journal.gap(Gap::Coverage);
        }
        Ok(16 + length)
    }
}
