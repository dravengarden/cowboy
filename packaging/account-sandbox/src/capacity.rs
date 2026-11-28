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
//! free-space reserve through one budget ([`Daemon::budget`]): a new image,
//! a growth and a staging registration are each admitted and committed
//! under its guard, so none of them can spend headroom another committed.
//! Growth itself is serialized by a daemon-wide capacity lock, which
//! `remove_account` also needs before it unmounts a home.
//!
//! The 30 s sweep runs on its own thread without account locks, so a slow
//! resize never holds up fencing, stops or the unit sweep, and nothing that
//! holds an account lock ever waits for the capacity lock or does resize
//! I/O: a removal that finds the lock busy is refused and retried, and a
//! start never grows a home. A start (other than logout) into a home below
//! its minimum free space (`home_min_free_bytes`, at most an eighth of a
//! small home, at least one block) is refused with `insufficient_space` and
//! wakes the capacity thread, so `agy` never starts into a full disk.
//!
//! [`Host::reconcile_home`]: crate::host::Host::reconcile_home

use std::collections::HashMap;
use std::collections::HashSet;
use std::os::fd::{AsFd as _, OwnedFd};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use rustix::fs::{AtFlags, FileType};
use rustix::io::Errno;

use crate::daemon::{Daemon, guard};
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
/// Interval of [`Daemon::prune_logs`].
pub const LOG_PRUNE_INTERVAL: Duration = Duration::from_hours(1);

/// Entries of an account-controlled directory examined in one pass.
const MAX_ENTRIES_PER_PASS: usize = 4096;

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
    HostReserve { available: u64, growth: u64 },
    Failed(String),
}

impl std::fmt::Display for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AtMaximum => write!(f, "the image is at its maximum size"),
            Self::HostReserve { available, growth } => write!(
                f,
                "growing it by {growth} bytes needs that much free on the host beyond the reserve and what images and staging may still claim, and {available} are"
            ),
            Self::Failed(error) => write!(f, "growing it failed: {error}"),
        }
    }
}

/// Bytes the image files under `accounts` may still allocate: their length
/// beyond their allocated blocks. Only a confirmed absence counts as
/// nothing; any other failure to account for an image is an error.
pub(crate) fn unallocated_image_bytes(accounts: &OwnedFd) -> anyhow::Result<u64> {
    let mut total = 0_u64;
    for key in safefs::list(accounts.as_fd())? {
        if ids::validate_key(&key).is_err() {
            continue;
        }
        let dir = match rustix::fs::openat2(
            accounts.as_fd(),
            key.as_str(),
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOFOLLOW,
            rustix::fs::Mode::empty(),
            STRICT,
        ) {
            Ok(dir) => dir,
            Err(Errno::NOENT) => continue,
            Err(error) => return Err(error).with_context(|| format!("open account {key}")),
        };
        let stat = match rustix::fs::statat(&dir, IMAGE, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(Errno::NOENT) => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("stat the home image of {key}"));
            }
        };
        anyhow::ensure!(
            FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile,
            "the home image of {key} is not a regular file"
        );
        {
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
    /// The daemon-wide growth lock, waiting for it. Only the capacity sweep,
    /// which holds no account lock, waits.
    pub(crate) fn capacity_lock(&self) -> MutexGuard<'_, ()> {
        self.capacity.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The growth lock when it is free now; for callers holding an account
    /// lock, which must never wait for a resize.
    pub(crate) fn try_capacity_lock(&self) -> Option<MutexGuard<'_, ()>> {
        match self.capacity.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    /// The configured ceiling, never below the initial size.
    fn max_image_bytes(&self) -> u64 {
        self.config
            .home_image_max_bytes
            .max(self.config.home_image_bytes)
    }

    /// Holding the growth lock (`_capacity`): finish any interrupted grow of
    /// the mounted `home` of the account directory `dir`, then grow it when
    /// it runs low; why it did not grow when it needed to.
    pub(crate) fn check_home(
        &self,
        _capacity: &MutexGuard<'_, ()>,
        dir: &OwnedFd,
        home: &OwnedFd,
        key: &str,
    ) -> Option<Held> {
        match self.grow_home(dir, home) {
            Ok(Ok(Some((before, after)))) => {
                eprintln!(
                    "cowboy-account-sandbox: grew account {key} home image from {before} to {after} bytes"
                );
                None
            }
            Ok(Ok(None)) => None,
            Ok(Err(held)) => Some(held),
            Err(error) => Some(Held::Failed(format!("{error:#}"))),
        }
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
        let growth = target - file_len;
        let staging = self.budget();
        let available = self
            .available_space(&staging)
            .map_err(|failure| anyhow::anyhow!("{failure}"))?;
        if available < growth {
            return Ok(Err(Held::HostReserve { available, growth }));
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
        drop(staging);
        image.sync_all().context("sync home image")?;
        self.host.reconcile_home(&image, home.as_fd())?;
        Ok(Ok(Some((file_len, target))))
    }

    /// The free space a home of `size_bytes` must keep for a start: the
    /// configured minimum, at most an eighth of a small home, and never less
    /// than one block.
    fn min_free_bytes(&self, size_bytes: u64) -> u64 {
        self.config
            .home_min_free_bytes
            .min(size_bytes / 8)
            .max(4096)
    }

    /// Refuse a unit start into a home that keeps less than its minimum free
    /// space, and wake the capacity thread to grow it. A start never grows a
    /// home itself: it holds the account lock, which resize I/O must never
    /// delay. Logout is never refused: it must always be able to sign the
    /// account out.
    pub(crate) fn admit_home_capacity(&self, home: &OwnedFd, mode: Mode) -> Outcome<()> {
        if mode == Mode::Logout {
            return Ok(());
        }
        let usage = self.host.filesystem_usage(home.as_fd())?;
        if needs_growth(usage, self.config.home_grow_free_bytes) {
            self.wake_capacity();
        }
        if usage.free_bytes >= self.min_free_bytes(usage.size_bytes) && usage.free_inodes > 0 {
            return Ok(());
        }
        Err(fail(
            Code::InsufficientSpace,
            format!(
                "account home storage is exhausted: {} of {} bytes and {} inodes free; it grows within seconds unless it is at its maximum size or the host lacks space, so retry shortly",
                usage.free_bytes, usage.size_bytes, usage.free_inodes
            ),
        ))
    }

    /// Check every mounted account home (on the capacity thread, without
    /// account locks). Each home is checked holding the growth lock from
    /// before it is opened until it is closed, so a removal never finds it
    /// pinned.
    pub fn sweep_capacity(&self) {
        static REPORTED: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
        let Ok(accounts) = self.accounts() else {
            return;
        };
        let Ok(keys) = safefs::list(accounts.as_fd()) else {
            return;
        };
        let mut seen = HashSet::new();
        for key in keys {
            if ids::validate_key(&key).is_err() {
                continue;
            }
            let checked = {
                let capacity = self.capacity_lock();
                if !self
                    .host
                    .is_mounted(&self.paths(&key).home)
                    .unwrap_or(false)
                {
                    continue;
                }
                (|| -> Outcome<Option<Held>> {
                    let dir = safefs::open_dir(accounts.as_fd(), &key, STRICT)?;
                    let home = safefs::open_dir(dir.as_fd(), "home", safefs::MOUNT_ROOT)?;
                    Ok(self.check_home(&capacity, &dir, &home, &key))
                })()
            };
            seen.insert(key.clone());
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
        }
        // Forget accounts that were removed or unmounted.
        if let Some(reported) = guard(&REPORTED).as_mut() {
            reported.retain(|key, _| seen.contains(key));
        }
    }

    /// Remove old `agy` logs of every account that runs no unit (on the log
    /// thread, apart from capacity checks). An account whose lock is busy is
    /// skipped until the next pass.
    pub fn prune_logs(&self) {
        if self.config.agy_log_retention_days == 0 {
            return;
        }
        let Ok(accounts) = self.accounts() else {
            return;
        };
        for key in safefs::list(accounts.as_fd()).unwrap_or_default() {
            if ids::validate_key(&key).is_ok()
                && self
                    .host
                    .is_mounted(&self.paths(&key).home)
                    .unwrap_or(false)
            {
                self.prune_account_logs(&key);
            }
        }
    }

    /// Remove the account's old `agy` logs while it runs no unit (under its
    /// lock, so none starts meanwhile): a live `agy` may still reopen an old
    /// log by name.
    fn prune_account_logs(&self, key: &str) {
        let retention = Duration::from_hours(self.config.agy_log_retention_days * 24);
        let Some(result) = self.try_with_account(key, |dir, meta| {
            if !meta.units.is_empty() || self.any_live_unit(key)? {
                return Ok((0, 0));
            }
            let home = self.open_home(dir, key)?;
            Ok(prune_agy_logs(&home, retention, SystemTime::now())?)
        }) else {
            return;
        };
        match result {
            Ok((0, _)) => {}
            Ok((files, bytes)) => eprintln!(
                "cowboy-account-sandbox: removed {files} agy log files ({bytes} bytes) of account {key}"
            ),
            Err(error) if error.code == Code::NotFound => {}
            Err(error) => eprintln!("cowboy-account-sandbox: pruning account {key} logs: {error}"),
        }
    }

    /// Whether systemd holds a live unit of the account, recorded or not.
    fn any_live_unit(&self, key: &str) -> Outcome<bool> {
        for unit in self
            .host
            .list_units(&format!("{}-*", ids::user_name(key)))?
        {
            if self
                .host
                .unit_status(&unit)?
                .is_some_and(|status| status.is_live())
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Remove the regular files in the home's `agy` log directory last modified
/// before `now - retention`, examining at most a bounded number of entries
/// per pass; the files and bytes removed. Everything is opened beneath the
/// home without following a symlink, so an account cannot point the root
/// daemon at another file.
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
    for name in safefs::list_bounded(dir.as_fd(), MAX_ENTRIES_PER_PASS)? {
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
