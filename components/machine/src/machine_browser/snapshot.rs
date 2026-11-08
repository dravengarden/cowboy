//! Published profile snapshots: the Machine-wide staging budget, validation of
//! a staging tree as untrusted input, atomic publication, reconciliation of
//! orphans and abandoned staging, and the scoped removal reset and revocation
//! use. Nothing here follows a symlink or opens anything outside the account
//! directory it was given.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail, ensure};
use rustix::fs::{
    AtFlags, Dir, FileType, Mode, OFlags, fstat, fsync, openat, renameat, statat, unlinkat,
};

use super::settings::Settings;
use super::state_file::{
    self, ControlRecord, generation_name, parse_generation_name, snapshots_root, staging_root,
};

/// Entries one snapshot may hold.
pub const MAX_ENTRIES: u64 = 100_000;
/// Directory depth one snapshot may hold.
pub const MAX_DEPTH: u32 = 32;

const DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const REGULAR: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub max_bytes: u64,
    pub max_entries: u64,
    pub max_depth: u32,
}

impl Bounds {
    #[must_use]
    pub fn for_profile(profile_max_bytes: u64) -> Self {
        Self {
            max_bytes: profile_max_bytes,
            max_entries: MAX_ENTRIES,
            max_depth: MAX_DEPTH,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TreeStats {
    pub bytes: u64,
    pub entries: u64,
}

/// Why a snapshot was not admitted. Refused, never queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    AccountBusy,
    StagingBudget,
    DiskLowWatermark,
}

/// Machine-wide byte reservations of snapshot staging in flight: at most one
/// per account, bounded by `browser.staging_max_bytes` and by the Device disk
/// low watermark with the previous generation retained.
#[derive(Debug, Default)]
pub struct StagingLedger {
    inner: parking_lot::Mutex<BTreeMap<String, (u64, u64)>>,
}

/// A reserved staging generation; dropping it releases the bytes.
#[derive(Debug)]
pub struct StagingReservation {
    ledger: Arc<StagingLedger>,
    account: String,
    pub generation: u64,
    pub bytes: u64,
}

impl Drop for StagingReservation {
    fn drop(&mut self) {
        self.ledger.inner.lock().remove(&self.account);
    }
}

impl StagingLedger {
    /// Staging generations in flight per account.
    #[must_use]
    pub fn live(&self) -> BTreeMap<String, u64> {
        self.inner
            .lock()
            .iter()
            .map(|(account, (generation, _))| (account.clone(), *generation))
            .collect()
    }

    #[must_use]
    pub fn reserved_bytes(&self) -> u64 {
        self.inner.lock().values().map(|(_, bytes)| bytes).sum()
    }

    /// Admit one staging generation for `account`.
    ///
    /// # Errors
    /// Returns the refusal; nothing is reserved.
    pub fn admit(
        self: &Arc<Self>,
        account: &str,
        generation: u64,
        bytes: u64,
        settings: &Settings,
        free_bytes: Option<u64>,
    ) -> Result<StagingReservation, Refusal> {
        let mut inner = self.inner.lock();
        if inner.contains_key(account) {
            return Err(Refusal::AccountBusy);
        }
        let reserved: u64 = inner.values().map(|(_, bytes)| bytes).sum();
        let total = reserved.saturating_add(bytes);
        if total > settings.staging_max_bytes {
            return Err(Refusal::StagingBudget);
        }
        if let Some(free) = free_bytes
            && free.saturating_sub(total) < settings.disk_low_watermark
        {
            return Err(Refusal::DiskLowWatermark);
        }
        inner.insert(account.to_owned(), (generation, bytes));
        Ok(StagingReservation {
            ledger: Arc::clone(self),
            account: account.to_owned(),
            generation,
            bytes,
        })
    }
}

/// Free bytes on the filesystem holding `path`.
#[must_use]
pub fn free_bytes(path: &Path) -> Option<u64> {
    let stat = rustix::fs::statvfs(path).ok()?;
    Some(stat.f_bavail.saturating_mul(stat.f_frsize))
}

/// Create `staging/gen-<n>` for the launcher to fill; the directory is
/// private to the Machine user.
///
/// # Errors
/// Returns when the directory exists already or cannot be created.
pub fn create_staging(account_dir: &Path, generation: u64) -> Result<PathBuf> {
    let root = staging_root(account_dir);
    std::fs::create_dir_all(&root)?;
    for directory in [state_file::browser_root(account_dir), root.clone()] {
        crate::provider_account::prepare_private_directory(&directory)?;
    }
    let path = root.join(generation_name(generation));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .with_context(|| format!("creating {}", path.display()))?;
    Ok(path)
}

/// Validate a tree as untrusted input: regular files and directories only,
/// no links, bounded bytes, entries and depth; optionally fsync every file
/// and directory on the way.
///
/// # Errors
/// Returns when the tree violates the bounds or contains anything but
/// regular files and directories.
pub fn validate_tree(root: &Path, bounds: Bounds, sync: bool) -> Result<TreeStats> {
    let directory: File = rustix::fs::open(root, DIRECTORY, Mode::empty())
        .with_context(|| format!("opening {}", root.display()))?
        .into();
    let mut stats = TreeStats::default();
    walk(&directory, 0, bounds, sync, &mut stats)?;
    if sync {
        fsync(&directory)?;
    }
    Ok(stats)
}

fn walk(
    directory: &File,
    depth: u32,
    bounds: Bounds,
    sync: bool,
    stats: &mut TreeStats,
) -> Result<()> {
    ensure!(depth <= bounds.max_depth, "snapshot tree is too deep");
    for name in child_names(directory)? {
        stats.entries += 1;
        ensure!(
            stats.entries <= bounds.max_entries,
            "snapshot tree has too many entries"
        );
        let stat = statat(directory, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)?;
        match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => {
                let child: File =
                    openat(directory, name.as_str(), DIRECTORY, Mode::empty())?.into();
                walk(&child, depth + 1, bounds, sync, stats)?;
                if sync {
                    fsync(&child)?;
                }
            }
            FileType::RegularFile => {
                ensure!(stat.st_nlink == 1, "snapshot tree contains a hard link");
                stats.bytes = stats
                    .bytes
                    .saturating_add(u64::try_from(stat.st_size).unwrap_or(u64::MAX));
                ensure!(
                    stats.bytes <= bounds.max_bytes,
                    "snapshot tree exceeds its byte bound"
                );
                if sync {
                    let file: File =
                        openat(directory, name.as_str(), REGULAR, Mode::empty())?.into();
                    let opened = fstat(&file)?;
                    ensure!(
                        opened.st_ino == stat.st_ino && opened.st_dev == stat.st_dev,
                        "snapshot entry changed while it was validated"
                    );
                    fsync(&file)?;
                }
            }
            _ => bail!("snapshot tree contains something other than a regular file or directory"),
        }
    }
    Ok(())
}

/// A publication that failed, and how far it got.
#[derive(Debug)]
pub struct PublishFailure {
    /// `staging/gen-<n>` was already renamed to `snapshots/gen-<n>`: the
    /// staging path is gone and the generation must be reconciled against
    /// the durable control record (it is published exactly when that record
    /// names it).
    pub renamed: bool,
    pub error: anyhow::Error,
}

impl std::fmt::Display for PublishFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:#}", self.error)
    }
}

/// Publish `staging/gen-<n>` as the account's current snapshot generation:
/// validate and fsync the tree, rename it into `snapshots/`, fsync, update the
/// control record atomically, then prune every other generation. A failed
/// prune leaves the publication standing (the record names the generation);
/// reconciliation removes what remains.
///
/// # Errors
/// Returns the phase a failed step was in; `record` is unchanged then.
pub fn publish(
    account_dir: &Path,
    generation: u64,
    record: &mut ControlRecord,
    bounds: Bounds,
    now_ms: i64,
) -> Result<TreeStats, PublishFailure> {
    let staged = |error: anyhow::Error| PublishFailure {
        renamed: false,
        error,
    };
    let renamed = |error: anyhow::Error| PublishFailure {
        renamed: true,
        error,
    };
    let staging = staging_root(account_dir).join(generation_name(generation));
    let stats = validate_tree(&staging, bounds, true).map_err(staged)?;
    let snapshots = snapshots_root(account_dir);
    let opened = (|| -> Result<(File, File)> {
        let staging_parent: File =
            rustix::fs::open(staging_root(account_dir), DIRECTORY, Mode::empty())?.into();
        fsync(&staging_parent)?;
        std::fs::create_dir_all(&snapshots)?;
        crate::provider_account::prepare_private_directory(&snapshots)?;
        let snapshots_dir: File = rustix::fs::open(&snapshots, DIRECTORY, Mode::empty())?.into();
        Ok((staging_parent, snapshots_dir))
    })();
    let (staging_parent, snapshots_dir) = opened.map_err(staged)?;
    let name = generation_name(generation);
    renameat(
        &staging_parent,
        name.as_str(),
        &snapshots_dir,
        name.as_str(),
    )
    .with_context(|| format!("publishing snapshot {name}"))
    .map_err(staged)?;
    fsync(&snapshots_dir).map_err(|error| renamed(error.into()))?;
    let previous = (record.current_generation, record.current_generation_at_ms);
    record.current_generation = Some(generation);
    record.current_generation_at_ms = Some(now_ms);
    if let Err(error) = state_file::write_control(account_dir, record) {
        (record.current_generation, record.current_generation_at_ms) = previous;
        return Err(renamed(error));
    }
    if let Err(error) = prune(&snapshots_dir, Some(generation)) {
        tracing::warn!(%error, generation, "superseded browser snapshot not pruned; left for reconciliation");
    }
    Ok(stats)
}

/// Remove every `snapshots/` entry except the record's current generation and
/// every `staging/` entry not named in `live_staging`, using the control
/// record as the only authority. Returns the removed entry names.
///
/// # Errors
/// Returns when a removal fails part-way.
pub fn reconcile(
    account_dir: &Path,
    record: &ControlRecord,
    live_staging: &BTreeSet<u64>,
) -> Result<Vec<String>> {
    let mut removed = reconcile_published(account_dir, record)?;
    if let Some(staging) = open_optional(&staging_root(account_dir))? {
        for name in child_names(&staging)? {
            let keep = parse_generation_name(&name)
                .is_some_and(|generation| live_staging.contains(&generation));
            if !keep {
                remove_entry(&staging, &name, 0)?;
                removed.push(format!("staging/{name}"));
            }
        }
    }
    Ok(removed)
}

/// Remove every `snapshots/` entry except the record's current generation,
/// leaving `staging/` untouched: the reconciliation that stays safe while a
/// copier the Machine does not account for may still write staging.
///
/// # Errors
/// Returns when a removal fails part-way.
pub fn reconcile_published(account_dir: &Path, record: &ControlRecord) -> Result<Vec<String>> {
    match open_optional(&snapshots_root(account_dir))? {
        Some(snapshots) => prune(&snapshots, record.current_generation),
        None => Ok(Vec::new()),
    }
}

/// Delete the account's whole `browser/` tree (snapshots and staging) while
/// the control record beside it stays in place.
///
/// # Errors
/// Returns when a removal fails part-way; the caller retries from its fence.
pub fn remove_browser_dir(account_dir: &Path) -> Result<()> {
    let account: File = rustix::fs::open(account_dir, DIRECTORY, Mode::empty())
        .with_context(|| format!("opening {}", account_dir.display()))?
        .into();
    match statat(&account, state_file::BROWSER_DIR, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => remove_entry(&account, state_file::BROWSER_DIR, 0),
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Remove one abandoned `staging/gen-<n>` with the scoped, no-follow removal;
/// an absent generation is already removed.
///
/// # Errors
/// Returns when a removal fails part-way; the caller keeps the generation's
/// staging reservation and retries.
pub fn remove_staging(account_dir: &Path, generation: u64) -> Result<()> {
    let Some(staging) = open_optional(&staging_root(account_dir))? else {
        return Ok(());
    };
    let name = generation_name(generation);
    match statat(&staging, name.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => remove_entry(&staging, &name, 0),
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// The published generation directory, when the record names one that exists.
#[must_use]
pub fn current_snapshot(account_dir: &Path, record: &ControlRecord) -> Option<PathBuf> {
    let generation = record.current_generation?;
    let path = snapshots_root(account_dir).join(generation_name(generation));
    std::fs::symlink_metadata(&path)
        .ok()
        .filter(std::fs::Metadata::is_dir)
        .map(|_| path)
}

fn open_optional(path: &Path) -> Result<Option<File>> {
    match rustix::fs::open(path, DIRECTORY, Mode::empty()) {
        Ok(fd) => Ok(Some(fd.into())),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("opening {}", path.display())),
    }
}

fn child_names(directory: &File) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in Dir::read_from(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}

/// Remove every child of `snapshots` except `keep`.
fn prune(snapshots: &File, keep: Option<u64>) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for name in child_names(snapshots)? {
        if parse_generation_name(&name).is_some_and(|generation| Some(generation) == keep) {
            continue;
        }
        remove_entry(snapshots, &name, 0)?;
        removed.push(format!("snapshots/{name}"));
    }
    Ok(removed)
}

/// Remove `name` below `parent` without following anything: directories are
/// descended through their own descriptors, everything else is unlinked as
/// the entry it is.
fn remove_entry(parent: &File, name: &str, depth: u32) -> Result<()> {
    ensure!(depth <= MAX_DEPTH + 1, "removal tree is too deep");
    let stat = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
    if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
        let directory: File = openat(parent, name, DIRECTORY, Mode::empty())?.into();
        for child in child_names(&directory)? {
            remove_entry(&directory, &child, depth + 1)?;
        }
        unlinkat(parent, name, AtFlags::REMOVEDIR)?;
    } else {
        unlinkat(parent, name, AtFlags::empty())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn settings() -> Settings {
        Settings::from_config(None)
    }

    #[test]
    fn staging_budget_admits_one_per_account_under_the_budget_and_watermark() {
        let ledger = Arc::new(StagingLedger::default());
        let settings = settings();
        let first = ledger
            .admit("a", 1, 256 << 20, &settings, Some(100 << 30))
            .unwrap();
        assert_eq!(
            ledger
                .admit("a", 2, 1, &settings, Some(100 << 30))
                .unwrap_err(),
            Refusal::AccountBusy
        );
        assert_eq!(
            ledger
                .admit("b", 1, 1 << 30, &settings, Some(100 << 30))
                .unwrap_err(),
            Refusal::StagingBudget
        );
        assert_eq!(
            ledger
                .admit(
                    "b",
                    1,
                    256 << 20,
                    &settings,
                    Some(settings.disk_low_watermark + (400 << 20))
                )
                .unwrap_err(),
            Refusal::DiskLowWatermark
        );
        let second = ledger
            .admit("b", 1, 256 << 20, &settings, Some(100 << 30))
            .unwrap();
        assert_eq!(ledger.reserved_bytes(), 512 << 20);
        assert_eq!(ledger.live().get("b"), Some(&1));
        drop(first);
        drop(second);
        assert_eq!(ledger.reserved_bytes(), 0);
    }

    #[test]
    fn validation_rejects_links_devices_and_bounds_and_publication_is_atomic() {
        let temp = tempfile::tempdir().unwrap();
        let account = temp.path();
        let staging = create_staging(account, 1).unwrap();
        std::fs::create_dir(staging.join("Default")).unwrap();
        std::fs::write(staging.join("Default/Cookies"), vec![1u8; 4096]).unwrap();
        std::fs::write(staging.join("Local State"), b"{}").unwrap();
        let bounds = Bounds::for_profile(1 << 20);
        let stats = validate_tree(&staging, bounds, false).unwrap();
        assert_eq!(
            stats,
            TreeStats {
                bytes: 4098,
                entries: 3
            }
        );

        symlink("/etc/passwd", staging.join("escape")).unwrap();
        assert!(validate_tree(&staging, bounds, false).is_err());
        std::fs::remove_file(staging.join("escape")).unwrap();
        std::fs::hard_link(staging.join("Local State"), staging.join("link")).unwrap();
        assert!(validate_tree(&staging, bounds, false).is_err());
        std::fs::remove_file(staging.join("link")).unwrap();
        assert!(
            validate_tree(
                &staging,
                Bounds {
                    max_bytes: 100,
                    ..bounds
                },
                false
            )
            .is_err()
        );
        assert!(
            validate_tree(
                &staging,
                Bounds {
                    max_entries: 2,
                    ..bounds
                },
                false
            )
            .is_err()
        );

        let mut record = ControlRecord::default();
        let stats = publish(account, 1, &mut record, bounds, 42).unwrap();
        assert_eq!(stats.entries, 3);
        assert_eq!(record.current_generation, Some(1));
        assert!(
            snapshots_root(account)
                .join("gen-1/Default/Cookies")
                .is_file()
        );
        assert!(!staging.exists());
        assert_eq!(state_file::read_control(account).unwrap(), record);
        assert_eq!(
            current_snapshot(account, &record).unwrap(),
            snapshots_root(account).join("gen-1")
        );

        // A second generation replaces the first; an orphan and abandoned
        // staging are reconciled against the record alone.
        let staging = create_staging(account, 2).unwrap();
        std::fs::write(staging.join("Local State"), b"{}").unwrap();
        publish(account, 2, &mut record, bounds, 43).unwrap();
        assert!(!snapshots_root(account).join("gen-1").exists());
        std::fs::create_dir_all(snapshots_root(account).join("gen-9/deep")).unwrap();
        std::fs::create_dir_all(staging_root(account).join("gen-3")).unwrap();
        std::fs::create_dir_all(staging_root(account).join("gen-4")).unwrap();
        symlink("/", staging_root(account).join("gen-4/root")).unwrap();
        let removed = reconcile(account, &record, &BTreeSet::from([3])).unwrap();
        assert_eq!(removed, vec!["snapshots/gen-9", "staging/gen-4"]);
        assert!(staging_root(account).join("gen-3").is_dir());
        assert!(snapshots_root(account).join("gen-2").is_dir());

        // One abandoned generation is removed without following its links.
        symlink(account, staging_root(account).join("gen-3/escape")).unwrap();
        remove_staging(account, 3).unwrap();
        assert!(!staging_root(account).join("gen-3").exists());
        assert!(account.join(state_file::CONTROL_FILE).is_file());
        remove_staging(account, 3).unwrap();

        remove_browser_dir(account).unwrap();
        assert!(!state_file::browser_root(account).exists());
        assert!(account.join(state_file::CONTROL_FILE).is_file());
        remove_browser_dir(account).unwrap();
    }

    #[test]
    fn a_failure_after_the_rename_reports_its_phase_and_reconciles_away() {
        use std::os::unix::fs::PermissionsExt as _;
        if rustix::process::geteuid().is_root() {
            return; // Permissions cannot make the record unwritable.
        }
        let temp = tempfile::tempdir().unwrap();
        let account = temp.path();
        let bounds = Bounds::for_profile(1 << 20);
        let mut record = ControlRecord::default();
        create_staging(account, 1).unwrap();
        publish(account, 1, &mut record, bounds, 1).unwrap();
        let staging = create_staging(account, 2).unwrap();
        std::fs::write(staging.join("Local State"), b"{}").unwrap();
        // The record beside `browser/` cannot be replaced any more.
        std::fs::set_permissions(account, std::fs::Permissions::from_mode(0o500)).unwrap();
        let failure = publish(account, 2, &mut record, bounds, 2).unwrap_err();
        std::fs::set_permissions(account, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(failure.renamed, "{failure}");
        assert_eq!(record.current_generation, Some(1));
        assert!(!staging.exists());
        assert!(snapshots_root(account).join("gen-2").is_dir());
        // The durable record is the authority: the renamed generation is an
        // orphan and goes, the published one stays.
        let durable = state_file::read_control(account).unwrap();
        assert_eq!(durable.current_generation, Some(1));
        let removed = reconcile(account, &durable, &BTreeSet::new()).unwrap();
        assert_eq!(removed, vec!["snapshots/gen-2"]);
        assert!(snapshots_root(account).join("gen-1").is_dir());
        // A failure before the rename leaves the staging in place.
        let mut record = durable;
        let failure = publish(account, 3, &mut record, bounds, 3).unwrap_err();
        assert!(!failure.renamed);
    }
}
