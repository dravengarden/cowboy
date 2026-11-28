//! Elastic account home capacity (design R3-7, "Revision 7: elastic home
//! images"). An account home is a sparse ext4 image whose fixed size was a
//! cliff: `agy` keeps every conversation of every recoverable session in it,
//! so a busy account filled it and its runtime died on `ENOSPC` mid-turn.
//!
//! A mounted home now grows online before it runs out: once its free space
//! falls below `max(home_grow_free_bytes, size / 4)` (or an eighth of its
//! inodes remain) the image file doubles, its loop device's capacity is
//! refreshed and the mounted filesystem is resized. Growth stops at
//! `home_image_max_bytes` and never takes the state root's filesystem below
//! its free-space reserve, counting the whole new size as if written. A start
//! (other than logout) is refused with `insufficient_space` while its home
//! keeps less than `home_min_free_bytes` free after that attempt, so `agy`
//! never starts into a full disk. Starts check on the way in and the sweep
//! checks every mounted home while its units run.
//!
//! Growth is idempotent: an image file already longer than its filesystem (a
//! grow interrupted between `set_len` and `resize2fs`) is only resized.

use std::os::fd::{AsFd as _, OwnedFd};
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use rustix::fs::{AtFlags, FileType};

use crate::config::Config;
use crate::daemon::Daemon;
use crate::host::FsUsage;
use crate::ids;
use crate::protocol::{Code, Mode, Outcome, fail};
use crate::safefs::{self, STRICT};
use crate::units::{GEMINI_DIR, GEMINI_STATE_DIR};

const IMAGE: &str = "home.img";
/// `agy`'s log directory in its state directory.
const AGY_LOG_DIR: &str = "log";
const MIB: u64 = 1024 * 1024;

/// Whether a home with `usage` should grow now.
pub(crate) fn needs_growth(usage: FsUsage, config: &Config) -> bool {
    usage.free_bytes < config.home_grow_free_bytes.max(usage.size_bytes / 4)
        || usage.free_inodes < usage.inodes / 8
}

/// The length to give an image file of `file_len` bytes whose filesystem
/// reports `usage`, or `None` when it is at its maximum. ext4 reports its
/// size without metadata overhead (a few percent), so a filesystem below
/// seven eighths of its file is an interrupted grow to finish, not a reason
/// to double again.
pub(crate) fn growth_target(usage: FsUsage, file_len: u64, config: &Config) -> Option<u64> {
    if usage.size_bytes < file_len / 8 * 7 {
        return Some(file_len);
    }
    let target = file_len.saturating_mul(2).min(config.home_image_max_bytes) / MIB * MIB;
    (target > file_len).then_some(target)
}

/// Why a home that needed to grow did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Held {
    AtMaximum,
    HostReserve { host_free: u64, needed: u64 },
    Failed(String),
}

impl std::fmt::Display for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AtMaximum => write!(f, "the image is at its maximum size"),
            Self::HostReserve { host_free, needed } => write!(
                f,
                "growing it needs {needed} bytes of host space beyond the reserve and {host_free} are free"
            ),
            Self::Failed(error) => write!(f, "growing it failed: {error}"),
        }
    }
}

/// A home's usage after a capacity check, and why it did not grow when it
/// needed to.
pub(crate) struct Capacity {
    pub usage: FsUsage,
    pub held: Option<Held>,
}

impl Daemon {
    /// Grow the account's mounted home when it runs low; the caller holds
    /// the account lock. Only reading the usage can fail: a failed grow is
    /// reported in [`Capacity::held`].
    pub(crate) fn ensure_home_capacity(&self, dir: &OwnedFd, key: &str) -> Outcome<Capacity> {
        let paths = self.paths(key);
        let usage = self.host.filesystem_usage(&paths.home)?;
        if !needs_growth(usage, &self.config) {
            return Ok(Capacity { usage, held: None });
        }
        match self.grow_home(dir, key, usage) {
            Ok(Ok((before, after))) => {
                eprintln!(
                    "cowboy-account-sandbox: grew account {key} home image from {before} to {after} bytes"
                );
                let usage = self.host.filesystem_usage(&paths.home)?;
                let held = needs_growth(usage, &self.config).then(|| {
                    if after >= self.config.home_image_max_bytes {
                        Held::AtMaximum
                    } else {
                        Held::Failed("the filesystem did not take the new size".into())
                    }
                });
                Ok(Capacity { usage, held })
            }
            Ok(Err(held)) => Ok(Capacity {
                usage,
                held: Some(held),
            }),
            Err(error) => {
                eprintln!("cowboy-account-sandbox: growing account {key} home: {error:#}");
                Ok(Capacity {
                    usage,
                    held: Some(Held::Failed(format!("{error:#}"))),
                })
            }
        }
    }

    /// Lengthen the image file and resize the mounted filesystem: the old
    /// and new lengths, or why it was held.
    fn grow_home(
        &self,
        dir: &OwnedFd,
        key: &str,
        usage: FsUsage,
    ) -> anyhow::Result<Result<(u64, u64), Held>> {
        let file = safefs::open_regular_rw(dir.as_fd(), IMAGE)?;
        let file_len = file.metadata().context("stat home image")?.len();
        let Some(target) = growth_target(usage, file_len, &self.config) else {
            return Ok(Err(Held::AtMaximum));
        };
        let needed = self
            .config
            .free_space_reserve_bytes
            .saturating_add(target - file_len);
        let host_free = self.host.free_bytes(&self.config.state_root)?;
        if host_free < needed {
            return Ok(Err(Held::HostReserve { host_free, needed }));
        }
        if target > file_len {
            file.set_len(target).context("lengthen home image")?;
            file.sync_all().context("sync home image")?;
        }
        drop(file);
        let paths = self.paths(key);
        self.host.grow_mounted_image(&paths.image, &paths.home)?;
        Ok(Ok((file_len, target)))
    }

    /// Refuse a unit start into a home that keeps less than the configured
    /// minimum free after growing. Logout still starts: it frees nothing but
    /// must always be able to sign the account out.
    pub(crate) fn admit_home_capacity(&self, dir: &OwnedFd, key: &str, mode: Mode) -> Outcome<()> {
        let Capacity { usage, held } = self.ensure_home_capacity(dir, key)?;
        if mode == Mode::Logout
            || (usage.free_bytes >= self.config.home_min_free_bytes && usage.free_inodes > 0)
        {
            return Ok(());
        }
        let reason = held.map_or_else(String::new, |held| format!("; {held}"));
        Err(fail(
            Code::InsufficientSpace,
            format!(
                "account home storage is exhausted: {} of {} bytes and {} inodes free{reason}",
                usage.free_bytes, usage.size_bytes, usage.free_inodes
            ),
        ))
    }

    /// Check every mounted account home and, when `prune_logs`, remove its
    /// `agy` log files older than the retention.
    pub fn sweep_capacity(&self, prune_logs: bool) {
        let Ok(accounts) = self.accounts() else {
            return;
        };
        let Ok(keys) = safefs::list(accounts.as_fd()) else {
            return;
        };
        for key in keys {
            if ids::validate_key(&key).is_err()
                || !self
                    .host
                    .is_mounted(&self.paths(&key).home)
                    .unwrap_or(false)
            {
                continue;
            }
            let result = self.with_account(&key, |dir, _meta| {
                let Capacity { held, .. } = self.ensure_home_capacity(dir, &key)?;
                if let Some(held @ (Held::AtMaximum | Held::HostReserve { .. })) = held {
                    eprintln!("cowboy-account-sandbox: account {key} home is low on space; {held}");
                }
                if prune_logs && self.config.agy_log_retention_days > 0 {
                    let home = self.open_home(dir, &key)?;
                    let retention =
                        Duration::from_hours(self.config.agy_log_retention_days * 24);
                    let (files, bytes) = prune_agy_logs(&home, retention, SystemTime::now())?;
                    if files > 0 {
                        eprintln!(
                            "cowboy-account-sandbox: removed {files} agy log files ({bytes} bytes) of account {key}"
                        );
                    }
                }
                Ok(())
            });
            if let Err(error) = result
                && error.code != Code::NotFound
            {
                eprintln!("cowboy-account-sandbox: checking account {key} home: {error}");
            }
        }
    }
}

/// Remove the regular files in the home's `agy` log directory last modified
/// before `now - retention`; the files and bytes removed. Everything is
/// opened beneath the home without following a symlink, so an account cannot
/// point the root daemon at another file.
pub(crate) fn prune_agy_logs(
    home: &OwnedFd,
    retention: Duration,
    now: SystemTime,
) -> anyhow::Result<(usize, u64)> {
    let mut dir = home.try_clone()?;
    for name in [GEMINI_DIR, GEMINI_STATE_DIR, AGY_LOG_DIR] {
        if !safefs::exists(dir.as_fd(), name)? {
            return Ok((0, 0));
        }
        dir = safefs::open_dir(dir.as_fd(), name, STRICT)?;
    }
    let cutoff = now
        .checked_sub(retention)
        .and_then(|cutoff| cutoff.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |cutoff| cutoff.as_secs());
    let (mut files, mut bytes) = (0, 0);
    for name in safefs::list(dir.as_fd())? {
        let Ok(stat) = rustix::fs::statat(dir.as_fd(), name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
        else {
            continue;
        };
        let modified = u64::try_from(stat.st_mtime).unwrap_or(0);
        if FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile
            && modified < cutoff
            && safefs::remove_file(dir.as_fd(), &name)?
        {
            files += 1;
            bytes += u64::try_from(stat.st_size).unwrap_or(0);
        }
    }
    Ok((files, bytes))
}
