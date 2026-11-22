//! File ownership, disk reservation and health for the optional rollup.
use super::super::{
    Connection, OpenFlags, PAGE_BYTES, Policy, Result, atomic_json, private, private_read,
};
use super::{DAY_MS, Entry, RETENTION_MS, utc_day};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(super) const FILE: &str = "wait-cache-daily-v1.sqlite";
const APPLICATION_ID: i64 = 0x43574331;
const SCHEMA: i64 = 1;
const HEALTH: &str = "wait-cache-health-v1.json";
const CREATING: &str = "wait-cache-daily-v1.sqlite.creating";
const RECOVERY_FILES: [&str; 5] = [
    "wait-cache-daily-v1.sqlite-journal",
    "wait-cache-daily-v1.sqlite-wal",
    "wait-cache-daily-v1.sqlite-shm",
    CREATING,
    "wait-cache-daily-v1.sqlite.creating-journal",
];
const SCHEMA_SQL: &str = "
    CREATE TABLE IF NOT EXISTS seen (id TEXT PRIMARY KEY, day INTEGER NOT NULL) WITHOUT ROWID;
    CREATE INDEX IF NOT EXISTS seen_day ON seen(day);
    CREATE TABLE IF NOT EXISTS rollups (day INTEGER NOT NULL, dimensions TEXT NOT NULL, totals TEXT NOT NULL,
                                        PRIMARY KEY(day, dimensions)) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS parent_requests (
        clock TEXT NOT NULL, started_at_ms INTEGER NOT NULL, id TEXT NOT NULL, day INTEGER NOT NULL,
        dimensions TEXT NOT NULL, bucket TEXT NOT NULL, totals TEXT NOT NULL, gap_ms INTEGER,
        PRIMARY KEY(clock, started_at_ms, id)) WITHOUT ROWID;
    CREATE UNIQUE INDEX IF NOT EXISTS parent_request_id ON parent_requests(id);
    CREATE INDEX IF NOT EXISTS parent_request_time ON parent_requests(started_at_ms);
    CREATE TABLE IF NOT EXISTS reconciliation (day INTEGER PRIMARY KEY, snapshot_at_ms INTEGER NOT NULL, report TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS reconciliation_attempts (day INTEGER PRIMARY KEY, snapshot_at_ms INTEGER NOT NULL, report TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS capacity_floor (slot INTEGER PRIMARY KEY CHECK(slot=1), first_day INTEGER NOT NULL);
    DROP TABLE IF EXISTS parent_starts;";

#[derive(Serialize)]
pub(in crate::logs::storage) struct Space {
    pub data_bytes: u64,
    pub recovery_bytes: u64,
    pub reserved_bytes: u64,
    pub over_budget: bool,
    pub health: Option<Health>,
}

impl Space {
    pub fn read(directory: &Path, policy: &Policy, writing_rollup: bool) -> Self {
        // lstat only. An unreadable or foreign file cannot block primary logs.
        let bytes = |name| fs::symlink_metadata(directory.join(name)).map_or(0, |m| m.len());
        let data_bytes = bytes(FILE);
        let recovery_bytes = RECOVERY_FILES.into_iter().map(bytes).sum::<u64>();
        let reserved_bytes = if data_bytes != 0 || recovery_bytes != 0 || writing_rollup {
            reservation(policy)
        } else {
            0
        };
        Self {
            data_bytes,
            recovery_bytes,
            reserved_bytes,
            over_budget: data_bytes > data_limit(policy)
                || data_bytes.saturating_add(recovery_bytes) > reserved_bytes,
            health: Health::read(directory).ok(),
        }
    }

    pub fn bytes(&self) -> u64 {
        self.data_bytes.saturating_add(self.recovery_bytes)
    }

    pub fn primary_limit(&self, policy: &Policy) -> u64 {
        // For a manually oversized/corrupt file keep at least one primary
        // segment available and expose the overage in status.
        policy
            .max_bytes
            .saturating_sub(self.reserved_bytes.max(self.bytes()))
            .max(policy.segment_bytes)
    }
}

fn reservation(policy: &Policy) -> u64 {
    // Reserve the side DB, its rollback pages and journal header/creation
    // overhead together. The rest of max_bytes belongs to primary segments.
    (policy.max_bytes / 4).min(192 * 1024 * 1024)
}

pub(super) fn data_limit(policy: &Policy) -> u64 {
    reservation(policy) / 3 / PAGE_BYTES * PAGE_BYTES
}

#[derive(Serialize, Deserialize, Default)]
pub(in crate::logs::storage) struct Health {
    maintenance_failures: u64,
    failed_rollup_records: u64,
    last_failure_ms: Option<i64>,
    #[serde(default)]
    degraded_days_ms: Option<BTreeSet<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    degraded_through_ms: Option<i64>,
}

impl Health {
    fn path(directory: &Path) -> PathBuf {
        directory.join(HEALTH)
    }

    pub(super) fn read(directory: &Path) -> Result<Self> {
        let bytes = match private_read(&Self::path(directory), 4096) {
            Ok(bytes) => bytes,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(Self::default());
            }
            Err(error) => return Err(error),
        };
        let mut health: Self = serde_json::from_slice(&bytes)?;
        if health.degraded_days_ms.is_none() && health.failed_rollup_records != 0 {
            // Older files kept only the most recent failure time. Its day
            // bounds the earlier days whose coverage can no longer be proven.
            health.degraded_through_ms =
                Some(utc_day(health.last_failure_ms.ok_or_else(|| {
                    anyhow::anyhow!("wait/cache health is missing its failure time")
                })?));
            health.degraded_days_ms = Some(BTreeSet::new());
        }
        Ok(health)
    }

    fn covers_day(&self, day: i64) -> bool {
        self.degraded_through_ms.is_some_and(|last| day <= last)
            || self
                .degraded_days_ms
                .as_ref()
                .is_some_and(|days| days.contains(&day))
    }

    pub(super) fn degraded_days(&self, from: i64, to: i64) -> Vec<i64> {
        (from..to)
            .step_by(DAY_MS as usize)
            .filter(|day| self.covers_day(*day))
            .collect()
    }

    pub fn record_maintenance_failure(directory: &Path, now: i64) -> Result<()> {
        let mut health = Self::read(directory)?;
        health.maintenance_failures = health.maintenance_failures.saturating_add(1);
        health.save(directory, now, [])
    }

    pub fn record_failed_rollup(directory: &Path, now: i64, records: &[&Entry]) -> Result<()> {
        let mut health = Self::read(directory)?;
        health.failed_rollup_records = health
            .failed_rollup_records
            .saturating_add(records.len() as u64);
        health.save(
            directory,
            now,
            records.iter().map(|entry| super::event_day(entry, now)),
        )
    }

    pub(in crate::logs::storage) fn ensure_degraded_days(
        directory: &Path,
        now: i64,
        days: &[i64],
    ) -> Result<()> {
        if days.is_empty() {
            return Ok(());
        }
        let mut health = Self::read(directory)?;
        let missing: Vec<i64> = days
            .iter()
            .copied()
            .filter(|day| !health.covers_day(*day))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        health.save(directory, now, missing)
    }

    pub(super) fn record_capacity_loss(directory: &Path, now: i64, day: i64) -> Result<()> {
        Self::read(directory)?.save(directory, now, [day])
    }

    fn save(
        &mut self,
        directory: &Path,
        now: i64,
        days: impl IntoIterator<Item = i64>,
    ) -> Result<()> {
        let latest = self.last_failure_ms.map_or(now, |old| old.max(now));
        self.last_failure_ms = Some(latest);
        let cutoff = utc_day(latest).saturating_sub(RETENTION_MS);
        let affected = self.degraded_days_ms.get_or_insert_with(BTreeSet::new);
        affected.extend(days);
        if let Some(last) = affected.range(..cutoff).next_back().copied() {
            // Maintenance may still have historical rows when a day marker
            // ages out. Keep a conservative bound for those older queries.
            self.degraded_through_ms =
                Some(self.degraded_through_ms.map_or(last, |old| old.max(last)));
        }
        affected.retain(|day| *day >= cutoff);
        atomic_json(&Self::path(directory), self)
    }
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn create(directory: &Path, path: &Path, policy: &Policy) -> Result<()> {
    // Allocation and recovery share the primary store lock. Reuse one owned
    // staging path so repeated process death cannot accumulate staging files.
    let temp = directory.join(CREATING);
    for leftover in [temp.clone(), directory.join(RECOVERY_FILES[4])] {
        if exists(&leftover)? {
            private(&leftover, false)?;
            fs::remove_file(leftover)?;
        }
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temp)?;
    let mut db = connect(&temp, false)?;
    db.execute_batch(
        "PRAGMA page_size=4096; PRAGMA auto_vacuum=FULL;
                      PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;",
    )?;
    db.pragma_update(None, "max_page_count", data_limit(policy) / PAGE_BYTES)?;
    db.pragma_update(None, "application_id", APPLICATION_ID)?;
    upgrade(&mut db, 0)?;
    drop(db);
    fs::rename(temp, path)?;
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

fn connect(path: &Path, read_only: bool) -> Result<Connection> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    let db = Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NOFOLLOW)?;
    db.busy_timeout(Duration::from_millis(100))?;
    db.pragma_update(None, "trusted_schema", false)?;
    Ok(db)
}

fn path(directory: &Path) -> Result<Option<PathBuf>> {
    let path = directory.join(FILE);
    if !exists(&path)? {
        return Ok(None);
    }
    private(&path, false)?;
    for name in RECOVERY_FILES.iter().take(3) {
        let side = directory.join(name);
        if exists(&side)? {
            private(&side, false)?;
        }
    }
    Ok(Some(path))
}

pub(super) fn open(directory: &Path, read_only: bool) -> Result<Option<Connection>> {
    let Some(path) = path(directory)? else {
        return Ok(None);
    };
    let db = connect(&path, read_only)?;
    let application_id = db.pragma_query_value(None, "application_id", |r| r.get::<_, i64>(0))?;
    let schema = db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?;
    let journal = db.pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))?;
    let page_size = db.pragma_query_value(None, "page_size", |r| r.get::<_, u64>(0))?;
    anyhow::ensure!(
        application_id == APPLICATION_ID
            && (0..=SCHEMA).contains(&schema)
            && journal == "delete"
            && page_size == PAGE_BYTES,
        "unrecognized wait/cache rollup"
    );
    Ok(Some(db))
}

fn upgrade(db: &mut Connection, schema: i64) -> Result<()> {
    if schema == 0 {
        let tx = db.transaction()?;
        tx.execute_batch(SCHEMA_SQL)?;
        tx.pragma_update(None, "user_version", SCHEMA)?;
        tx.commit()?;
    }
    Ok(())
}

pub(in crate::logs::storage) fn attach(
    db: &Connection,
    directory: &Path,
    policy: &Policy,
) -> Result<()> {
    let file = directory.join(FILE);
    let space = Space::read(directory, policy, true);
    anyhow::ensure!(!space.over_budget, "wait/cache rollup disk budget exceeded");
    let pages = data_limit(policy) / PAGE_BYTES;
    if !exists(&file)? {
        create(directory, &file, policy)?;
    }
    let mut side =
        open(directory, false)?.ok_or_else(|| anyhow::anyhow!("wait/cache rollup absent"))?;
    let schema = side.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?;
    side.pragma_update(None, "max_page_count", pages)?;
    upgrade(&mut side, schema)?;
    drop(side);
    let path = file
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("rollup path is not UTF-8"))?;
    db.execute("ATTACH DATABASE ?1 AS wait_cache", [path])?;
    let attached = Some(rusqlite::DatabaseName::Attached("wait_cache"));
    db.pragma_update(attached, "synchronous", "FULL")?;
    db.pragma_update(attached, "max_page_count", pages)?;
    Ok(())
}

pub(super) fn recover_for_maintenance(
    directory: &Path,
    started: std::time::Instant,
) -> Result<Option<Connection>> {
    let Some(mut db) = open(directory, false)? else {
        return Ok(None);
    };
    super::query_deadline(&db, started);
    let schema = db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?;
    upgrade(&mut db, schema)?;
    Ok(Some(db))
}
