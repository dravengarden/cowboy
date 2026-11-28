//! Elastic account home capacity (design "Revision 7: elastic account
//! homes"). An account home is a sparse ext4 image whose fixed size was a
//! cliff: `agy` keeps every conversation of every recoverable session in it,
//! so a busy account filled it and its runtime died on `ENOSPC` mid-turn.
//!
//! A mounted home now grows online before it runs out. Once its free space
//! falls below `max(home_grow_free_bytes, size / 4)` (or an eighth of its
//! inodes remain) the image file doubles, up to `home_image_max_bytes`. The
//! file only ever grows (`fallocate` of its new last block, never a
//! truncate), then [`Host::reconcile_home`] verifies the mounted home, its
//! loop device and the image are one chain (by device and inode, through
//! descriptors held for the whole operation), refreshes the device's
//! capacity and resizes the filesystem to the whole device. Every check
//! reconciles first, so a grow interrupted after the file was lengthened is
//! finished by the next check whatever the pressure.
//!
//! The space the images may still claim is charged against the state root's
//! free-space reserve: a grow needs the host to keep
//! `free_space_reserve_bytes` plus every image's unallocated sparse length
//! (this one's new length included) plus registrations staging. All growth
//! runs under one daemon-wide lock, so two accounts cannot both spend the
//! same headroom; `remove_account` takes that lock before it unmounts.
//!
//! The 30 s sweep runs on its own thread without account locks, so a slow
//! resize never holds up fencing, stops or the unit sweep. A start (other
//! than logout) only grows synchronously when its home is below
//! `home_min_free_bytes`, and is refused with `insufficient_space` while it
//! stays there, so `agy` never starts into a full disk.
//!
//! [`Host::reconcile_home`]: crate::host::Host::reconcile_home

use std::collections::HashMap;
use std::os::fd::{AsFd as _, OwnedFd};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use rustix::fs::{AtFlags, FileType};

use crate::daemon::{ACCOUNTS_DIR, Daemon, guard};
use crate::host::FsUsage;
use crate::ids;
use crate::protocol::{Code, Mode, Outcome, fail};
use crate::safefs::{self, STRICT};
use crate::units::{GEMINI_DIR, GEMINI_STATE_DIR};

pub const IMAGE: &str = "home.img";
/// `agy`'s log directory in its state directory.
const AGY_LOG_DIR: &str = "log";
const MIB: u64 = 1024 * 1024;
/// Interval of [`Daemon::sweep_capacity`].
pub const CAPACITY_INTERVAL: Duration = Duration::from_secs(30);
/// Sweeps between two log prunes (hourly).
pub const LOG_PRUNE_EVERY: u64 = 120;

/// Serializes every home growth and the unmount of a home.
static CAPACITY: Mutex<()> = Mutex::new(());

/// The daemon-wide growth lock.
pub(crate) fn capacity_lock() -> MutexGuard<'static, ()> {
    CAPACITY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether a home with `usage` should grow now.
pub(crate) fn needs_growth(usage: FsUsage, grow_free_bytes: u64) -> bool {
    usage.free_bytes < grow_free_bytes.max(usage.size_bytes / 4)
        || usage.free_inodes < usage.inodes / 8
}

/// The length to give an image file of `file_len` bytes: doubled, at most
/// `max`, whole MiB; `None` when that is no growth.
pub(crate) fn growth_target(file_len: u64, max: u64) -> Option<u64> {
    let target = file_len.saturating_mul(2).min(max) / MIB * MIB;
    (target > file_len).then_some(target)
}

/// Why a home that needed to grow did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Held {
    AtMaximum,
    HostReserve { host_free: u64, required: u64 },
    Failed(String),
}

impl std::fmt::Display for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AtMaximum => write!(f, "the image is at its maximum size"),
            Self::HostReserve {
                host_free,
                required,
            } => write!(
                f,
                "growing it needs {required} bytes free on the host (reserve, space the images may still claim and the growth) and {host_free} are free"
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

/// Bytes the image files under `accounts` may still allocate: their length
/// beyond their allocated blocks.
fn unallocated_image_bytes(accounts: &OwnedFd) -> anyhow::Result<u64> {
    let mut total = 0_u64;
    for key in safefs::list(accounts.as_fd())? {
        if ids::validate_key(&key).is_err() {
            continue;
        }
        let Ok(dir) = safefs::open_dir(accounts.as_fd(), &key, STRICT) else {
            continue;
        };
        let Ok(stat) = rustix::fs::statat(dir.as_fd(), IMAGE, AtFlags::SYMLINK_NOFOLLOW) else {
            continue;
        };
        if FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile {
            let length = u64::try_from(stat.st_size).unwrap_or(0);
            let allocated = u64::try_from(stat.st_blocks)
                .unwrap_or(0)
                .saturating_mul(512);
            total = total.saturating_add(length.saturating_sub(allocated));
        }
    }
    Ok(total)
}

impl Daemon {
    /// The configured ceiling, never below the initial size.
    fn max_image_bytes(&self) -> u64 {
        self.config
            .home_image_max_bytes
            .max(self.config.home_image_bytes)
    }

    /// Under the growth lock: finish any interrupted grow of the mounted
    /// `home` of the account directory `dir`, then grow it when it runs low.
    /// Only reading its usage can fail; a failed or held grow is reported
    /// in [`Capacity::held`].
    pub(crate) fn check_home(&self, dir: &OwnedFd, home: &OwnedFd, key: &str) -> Outcome<Capacity> {
        let _capacity = capacity_lock();
        let held = match self.grow_home(dir, home) {
            Ok(Ok(Some((before, after)))) => {
                eprintln!(
                    "cowboy-account-sandbox: grew account {key} home image from {before} to {after} bytes"
                );
                None
            }
            Ok(Ok(None)) => None,
            Ok(Err(held)) => Some(held),
            Err(error) => Some(Held::Failed(format!("{error:#}"))),
        };
        let usage = self.host.filesystem_usage(home.as_fd())?;
        Ok(Capacity { usage, held })
    }

    /// Reconcile, then lengthen and reconcile again when the home runs low:
    /// the old and new lengths when it grew, or why it was held.
    fn grow_home(
        &self,
        dir: &OwnedFd,
        home: &OwnedFd,
    ) -> anyhow::Result<Result<Option<(u64, u64)>, Held>> {
        let image = safefs::open_regular_rw(dir.as_fd(), IMAGE)?;
        self.host.reconcile_home(&image, home.as_fd())?;
        let usage = self.host.filesystem_usage(home.as_fd())?;
        if !needs_growth(usage, self.config.home_grow_free_bytes) {
            return Ok(Ok(None));
        }
        let file_len = image.metadata().context("stat home image")?.len();
        let Some(target) = growth_target(file_len, self.max_image_bytes()) else {
            return Ok(Err(Held::AtMaximum));
        };
        let accounts = safefs::open_dir(self.root.as_fd(), ACCOUNTS_DIR, STRICT)?;
        let required = self
            .config
            .free_space_reserve_bytes
            .saturating_add(unallocated_image_bytes(&accounts)?)
            .saturating_add(target - file_len)
            .saturating_add(self.staging_bytes());
        let host_free = self.host.free_bytes(&self.config.state_root)?;
        if host_free < required {
            return Ok(Err(Held::HostReserve {
                host_free,
                required,
            }));
        }
        // Allocating the new last block lengthens the file and never
        // shortens one that something else lengthened meanwhile.
        rustix::fs::fallocate(
            &image,
            rustix::fs::FallocateFlags::empty(),
            target - 4096,
            4096,
        )
        .context("lengthen home image")?;
        image.sync_all().context("sync home image")?;
        self.host.reconcile_home(&image, home.as_fd())?;
        Ok(Ok(Some((file_len, target))))
    }

    /// Refuse a unit start into a home that keeps less than the configured
    /// minimum free, growing it first when it is that low. Logout is never
    /// refused or delayed: it must always be able to sign the account out.
    pub(crate) fn admit_home_capacity(
        &self,
        dir: &OwnedFd,
        home: &OwnedFd,
        key: &str,
        mode: Mode,
    ) -> Outcome<()> {
        if mode == Mode::Logout {
            return Ok(());
        }
        let roomy = |usage: FsUsage| {
            usage.free_bytes >= self.config.home_min_free_bytes && usage.free_inodes > 0
        };
        if roomy(self.host.filesystem_usage(home.as_fd())?) {
            return Ok(());
        }
        let Capacity { usage, held } = self.check_home(dir, home, key)?;
        if roomy(usage) {
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

    /// Check every mounted account home (on the capacity thread, without
    /// account locks) and, when `prune_logs`, remove old `agy` logs of each
    /// account that runs no unit.
    pub fn sweep_capacity(&self, prune_logs: bool) {
        static REPORTED: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
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
            let checked = (|| -> Outcome<Option<Held>> {
                let dir = safefs::open_dir(accounts.as_fd(), &key, STRICT)?;
                let home = safefs::open_dir(dir.as_fd(), "home", safefs::MOUNT_ROOT)?;
                Ok(self.check_home(&dir, &home, &key)?.held)
            })();
            // Report a held home once per reason, not every sweep.
            let report = match &checked {
                Ok(None) => None,
                Ok(Some(held)) => Some(format!("home is low on space; {held}")),
                Err(error) => Some(format!("checking home: {error}")),
            };
            {
                let mut reported = guard(&REPORTED);
                let reported = reported.get_or_insert_with(HashMap::new);
                match report {
                    Some(report) if reported.get(&key) != Some(&report) => {
                        eprintln!("cowboy-account-sandbox: account {key} {report}");
                        reported.insert(key.clone(), report);
                    }
                    Some(_) => {}
                    None => {
                        reported.remove(&key);
                    }
                }
            }
            if prune_logs && self.config.agy_log_retention_days > 0 {
                self.prune_account_logs(&key);
            }
        }
    }

    /// Remove the account's old `agy` logs while it runs no unit (under its
    /// lock, so none starts meanwhile): a live `agy` may still reopen an old
    /// log by name.
    fn prune_account_logs(&self, key: &str) {
        let retention = Duration::from_hours(self.config.agy_log_retention_days * 24);
        let result = self.with_account(key, |dir, meta| {
            if !meta.units.is_empty() {
                return Ok((0, 0));
            }
            let home = self.open_home(dir, key)?;
            Ok(prune_agy_logs(&home, retention, SystemTime::now())?)
        });
        match result {
            Ok((0, _)) => {}
            Ok((files, bytes)) => eprintln!(
                "cowboy-account-sandbox: removed {files} agy log files ({bytes} bytes) of account {key}"
            ),
            Err(error) if error.code == Code::NotFound => {}
            Err(error) => eprintln!("cowboy-account-sandbox: pruning account {key} logs: {error}"),
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
