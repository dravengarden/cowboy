use super::data::{Coverage, Entry, MAX_SCAN, Metrics, Page, Query};
use anyhow::{Context as _, Result, ensure};
use base64::Engine as _;
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const APPLICATION_ID: i64 = 0x43424c47;
const MAX_SEGMENTS: usize = 64;
const PAGE_BYTES: u64 = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Policy {
    pub schema: u16,
    pub retain_seconds: u64,
    pub rotate_seconds: u64,
    pub segment_bytes: u64,
    pub max_bytes: u64,
    /// Retain sampled owned runtime spans locally instead of returning them
    /// through the Controller. This never suppresses business event delivery.
    pub forward_runtime: bool,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            schema: 1,
            retain_seconds: 7 * 86400,
            rotate_seconds: 86400,
            segment_bytes: 8 * 1024 * 1024,
            max_bytes: 128 * 1024 * 1024,
            forward_runtime: false,
        }
    }
}
impl Policy {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1
                && (60..=90 * 86400).contains(&self.retain_seconds)
                && (60..=86400).contains(&self.rotate_seconds)
                && self.rotate_seconds <= self.retain_seconds,
            "invalid log retention or rotation interval"
        );
        ensure!(
            (1024 * 1024..=64 * 1024 * 1024).contains(&self.segment_bytes)
                && (2 * self.segment_bytes..=4 * 1024 * 1024 * 1024).contains(&self.max_bytes),
            "invalid log storage capacity"
        );
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema: u16,
    id: String,
    policy: Policy,
    expired_records: u64,
    capacity_evicted_segments: u64,
    #[serde(default)]
    maintained_ms: i64,
}

pub(crate) fn private(path: &Path, directory: bool) -> Result<fs::Metadata> {
    let m = fs::symlink_metadata(path).context("log path unavailable")?;
    ensure!(
        !m.is_symlink()
            && (if directory { m.is_dir() } else { m.is_file() })
            && m.uid() == rustix::process::geteuid().as_raw()
            && m.mode() & 0o077 == 0
            && (directory || m.nlink() == 1),
        "log paths must be private, owned, and free of links"
    );
    Ok(m)
}

pub(crate) fn private_read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = private(path, false)?;
    ensure!(
        metadata.len() <= limit,
        "private log configuration exceeds limit"
    );
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let actual = file.metadata()?;
    ensure!(
        actual.ino() == metadata.ino() && actual.dev() == metadata.dev(),
        "log path changed during read"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "private log configuration exceeds limit"
    );
    Ok(bytes)
}

pub(crate) fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if path.try_exists()? {
        private(path, false)?;
    }
    let temporary = path.with_file_name(format!(".logs-{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(&serde_json::to_vec(value)?)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(path.parent().context("log file needs parent")?)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

struct Lock(File);
#[derive(Debug)]
pub(crate) struct Busy;
impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("log store busy")
    }
}
impl std::error::Error for Busy {}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

#[derive(Clone)]
pub(crate) struct SqliteStore {
    pub(crate) directory: PathBuf,
}
#[derive(Clone)]
struct Segment {
    path: PathBuf,
    created_ms: i64,
    bytes: u64,
}

impl SqliteStore {
    pub(crate) fn open(directory: PathBuf, create: bool) -> Result<Self> {
        ensure!(
            directory.is_absolute() && directory.file_name().is_some(),
            "log directory must be an absolute child directory"
        );
        if create && !directory.try_exists()? {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .context("creating private log directory")?;
        }
        private(&directory, true)?;
        let store = Self { directory };
        if create {
            let _lock = store.lock(true)?;
            if !store.directory.join("store.json").try_exists()? {
                store.save(&State {
                    schema: 1,
                    id: uuid::Uuid::new_v4().simple().to_string(),
                    policy: Policy::default(),
                    expired_records: 0,
                    capacity_evicted_segments: 0,
                    maintained_ms: 0,
                })?;
            }
        }
        store.state()?;
        Ok(store)
    }
    fn lock(&self, create: bool) -> Result<Lock> {
        private(&self.directory, true)?;
        let path = self.directory.join(".logs.lock");
        if path.try_exists()? {
            private(&path, false)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(create)
            .create(create)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)?;
        let expected = private(&path, false)?;
        let actual = file.metadata()?;
        ensure!(
            actual.ino() == expected.ino() && actual.dev() == expected.dev(),
            "log lock changed"
        );
        let start = Instant::now();
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Lock(file)),
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        && start.elapsed() < Duration::from_millis(500) =>
                {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Err(Busy.into()),
                Err(_) => anyhow::bail!("log store lock unavailable"),
            }
        }
    }
    fn state(&self) -> Result<State> {
        let state: State = serde_json::from_slice(&private_read(
            &self.directory.join("store.json"),
            16 * 1024,
        )?)
        .context("invalid log store configuration")?;
        ensure!(
            state.schema == 1 && state.id.len() == 32,
            "unsupported log store"
        );
        state.policy.validate()?;
        Ok(state)
    }
    fn save(&self, state: &State) -> Result<()> {
        atomic_json(&self.directory.join("store.json"), state)
    }
    pub(crate) fn policy(&self) -> Result<Policy> {
        Ok(self.state()?.policy)
    }
    pub(crate) fn configure(&self, policy: Policy) -> Result<()> {
        policy.validate()?;
        let _lock = self.lock(false)?;
        let mut state = self.state()?;
        state.policy = policy;
        self.save(&state)
    }
    fn segments(&self) -> Result<Vec<Segment>> {
        let mut segments = Vec::new();
        for (index, entry) in fs::read_dir(&self.directory)?.enumerate() {
            ensure!(index < 8192, "log directory inventory exceeds bounds");
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(id) = name
                .strip_prefix("otel-")
                .and_then(|s| s.strip_suffix(".sqlite"))
            else {
                continue;
            };
            let Some((stamp, nonce)) = id.split_once('-') else {
                continue;
            };
            if stamp.len() != 16
                || nonce.len() != 16
                || !stamp.bytes().all(|b| b.is_ascii_digit())
                || !nonce.bytes().all(|b| b.is_ascii_hexdigit())
            {
                continue;
            }
            let m = private(&entry.path(), false)?;
            segments.push(Segment {
                path: entry.path(),
                created_ms: stamp.parse()?,
                bytes: m.len(),
            });
            ensure!(
                segments.len() <= MAX_SEGMENTS + 1,
                "too many log segments; maintenance required"
            );
        }
        segments.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(segments)
    }
    fn connection(path: &Path, write: bool) -> Result<Connection> {
        private(path, false)?;
        for suffix in ["-journal", "-wal", "-shm"] {
            let side = PathBuf::from(format!("{}{suffix}", path.display()));
            if side.try_exists()? {
                private(&side, false)?;
            }
        }
        let flags = if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let connection = Connection::open_with_flags(
            path,
            flags | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.busy_timeout(Duration::from_millis(250))?;
        connection.pragma_update(None, "trusted_schema", false)?;
        ensure!(
            connection.pragma_query_value(None, "application_id", |r| r.get::<_, i64>(0))?
                == APPLICATION_ID
                && connection.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?
                    == 1,
            "unrecognized log database; preserved without modification"
        );
        Ok(connection)
    }
    fn create_segment(&self, now: i64, policy: &Policy) -> Result<Segment> {
        let path = self.directory.join(format!(
            "otel-{now:016}-{:016x}.sqlite",
            rand::random::<u64>()
        ));
        let temporary = path.with_extension("creating");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)?;
        let connection = Connection::open_with_flags(
            &temporary,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.execute_batch("PRAGMA page_size=4096; PRAGMA auto_vacuum=FULL; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA trusted_schema=OFF;
            CREATE TABLE entries(id TEXT PRIMARY KEY, observed_ms INTEGER NOT NULL, timestamp_ms INTEGER NOT NULL, signal TEXT NOT NULL, severity INTEGER NOT NULL, event TEXT NOT NULL, service TEXT NOT NULL, session TEXT NOT NULL, machine TEXT NOT NULL, environment TEXT NOT NULL, trace_id TEXT NOT NULL, duration_ms REAL, body TEXT NOT NULL, protobuf BLOB NOT NULL);
            CREATE INDEX entries_time ON entries(observed_ms,id);
            CREATE INDEX entries_session ON entries(session,observed_ms);
            CREATE INDEX entries_trace ON entries(trace_id,observed_ms);")?;
        connection.pragma_update(None, "application_id", APPLICATION_ID)?;
        connection.pragma_update(None, "user_version", 1)?;
        connection.pragma_update(None, "max_page_count", policy.segment_bytes / PAGE_BYTES)?;
        drop(connection);
        fs::rename(&temporary, &path)?;
        File::open(&self.directory)?.sync_all()?;
        Ok(Segment {
            bytes: private(&path, false)?.len(),
            path,
            created_ms: now,
        })
    }
    pub(crate) fn append(&self, entries: &[Entry], now: i64) -> Result<()> {
        ensure!(entries.len() <= 256 && now >= 0, "log batch exceeds bounds");
        for e in entries {
            e.validate()?;
        }
        let _lock = self.lock(false)?;
        let mut state = self.state()?;
        for chunk in entries.chunks(4) {
            let mut segments = self.segments()?;
            let estimate: u64 = chunk
                .iter()
                .map(|e| (e.protobuf.len() + e.body.len() + 2048) as u64)
                .sum::<u64>()
                * 2
                + 32 * 1024;
            let current = segments.last().filter(|s| {
                now >= s.created_ms
                    && now - s.created_ms < state.policy.rotate_seconds as i64 * 1000
                    && s.bytes + estimate <= state.policy.segment_bytes
            });
            let segment = if let Some(segment) = current {
                segment.clone()
            } else {
                // Free oldest capacity before allocating; current and temporary
                // SQLite journals have a separately bounded one-segment allowance.
                while !segments.is_empty()
                    && (segments.iter().map(|s| s.bytes).sum::<u64>() + state.policy.segment_bytes
                        > state.policy.max_bytes
                        || segments.len() >= MAX_SEGMENTS)
                {
                    self.remove(&segments.remove(0))?;
                    state.capacity_evicted_segments += 1;
                    self.save(&state)?;
                }
                self.create_segment(now, &state.policy)?
            };
            let mut db = Self::connection(&segment.path, true)?;
            db.pragma_update(
                None,
                "max_page_count",
                state.policy.segment_bytes / PAGE_BYTES,
            )?;
            let tx = db.transaction()?;
            {
                let mut insert = tx.prepare_cached("INSERT OR IGNORE INTO entries VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)")?;
                for e in chunk {
                    insert.execute(params![
                        e.id,
                        e.observed_ms,
                        e.timestamp_ms,
                        e.signal,
                        e.severity,
                        e.event,
                        e.service,
                        e.session,
                        e.machine,
                        e.environment,
                        e.trace_id,
                        e.duration_ms,
                        e.body,
                        e.protobuf
                    ])?;
                }
            }
            tx.commit()?;
        }
        Ok(())
    }
    fn remove(&self, segment: &Segment) -> Result<()> {
        // Verify ownership and the application marker before every unlink.
        // The store lock excludes all Cowboy readers/writers; connections are
        // closed between batches, so rotation never unlinks an active database.
        drop(Self::connection(&segment.path, true)?);
        for suffix in ["-journal", "-wal", "-shm", ""] {
            let path = PathBuf::from(format!("{}{suffix}", segment.path.display()));
            if path.try_exists()? {
                private(&path, false)?;
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }
    fn prune(&self, state: &mut State, now: i64) -> Result<()> {
        let cutoff = now.saturating_sub(state.policy.retain_seconds as i64 * 1000);
        let before = (state.expired_records, state.capacity_evicted_segments);
        let mut segments = self.segments()?;
        for segment in &segments {
            let db = Self::connection(&segment.path, true)?;
            // Time-to-live is exact per observed record, not a whole-day grace.
            let removed = db.execute("DELETE FROM entries WHERE observed_ms < ?1", [cutoff])?;
            state.expired_records += removed as u64;
            let empty: bool =
                db.query_row("SELECT NOT EXISTS(SELECT 1 FROM entries)", [], |r| r.get(0))?;
            drop(db);
            if empty {
                self.remove(segment)?;
            }
        }
        segments = self.segments()?;
        while !segments.is_empty()
            && (segments.iter().map(|s| s.bytes).sum::<u64>() > state.policy.max_bytes
                || segments.len() > MAX_SEGMENTS)
        {
            self.remove(&segments.remove(0))?;
            state.capacity_evicted_segments += 1;
        }
        if before != (state.expired_records, state.capacity_evicted_segments) {
            self.save(state)?;
        }
        Ok(())
    }
    pub(crate) fn maintain(&self, now: i64) -> Result<()> {
        self.maintenance(now, false)
    }
    pub(crate) fn maintain_if_due(&self, now: i64) -> Result<()> {
        self.maintenance(now, true)
    }
    fn maintenance(&self, now: i64, if_due: bool) -> Result<()> {
        let _lock = self.lock(false)?;
        let mut state = self.state()?;
        if if_due && now >= state.maintained_ms && now - state.maintained_ms < 30_000 {
            return Ok(());
        }
        // Creation happens under this same lock. An interrupted initializer
        // cannot leave a recognized half-schema database blocking all writers.
        for entry in fs::read_dir(&self.directory)?.take(8192) {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let base = name.strip_suffix("-journal").unwrap_or(&name);
            if let Some(id) = base
                .strip_prefix("otel-")
                .and_then(|s| s.strip_suffix(".creating"))
                && let Some((stamp, nonce)) = id.split_once('-')
                && stamp.len() == 16
                && stamp.bytes().all(|b| b.is_ascii_digit())
                && nonce.len() == 16
                && nonce.bytes().all(|b| b.is_ascii_hexdigit())
            {
                private(&entry.path(), false)?;
                fs::remove_file(entry.path())?;
            }
            if let Some(id) = name
                .strip_prefix(".logs-")
                .and_then(|s| s.strip_suffix(".tmp"))
                && id.len() == 32
                && id.bytes().all(|b| b.is_ascii_hexdigit())
            {
                let meta = private(&entry.path(), false)?;
                if meta
                    .modified()?
                    .elapsed()
                    .is_ok_and(|v| v > Duration::from_secs(3600))
                {
                    fs::remove_file(entry.path())?;
                }
            }
        }
        self.prune(&mut state, now)?;
        super::capture::cleanup_health(
            &self.directory,
            now - state.policy.retain_seconds as i64 * 1000,
        )?;
        state.maintained_ms = now;
        self.save(&state)
    }
    pub(crate) fn status(&self) -> Result<serde_json::Value> {
        let _lock = self.lock(false)?;
        let state = self.state()?;
        let segments = self.segments()?;
        Ok(
            serde_json::json!({"schema":"cowboy.logs.status/v1","store_id":state.id,"directory":self.directory,"policy":state.policy,"retained_bytes":segments.iter().map(|s|s.bytes).sum::<u64>(),"segments":segments.len(),"expired_records":state.expired_records,"capacity_evicted_segments":state.capacity_evicted_segments,"maintained_ms":state.maintained_ms,"maintenance_interval_seconds":30,"writers":super::capture::read_health(&self.directory)?}),
        )
    }
    pub(crate) fn query(&self, query: &Query) -> Result<Page> {
        query.validate()?;
        let _lock = self.lock(false)?;
        let state = self.state()?;
        let (query, position) = decode_cursor(query, &state.id)?;
        let mut items = Vec::new();
        let mut coverage =
            self.scan(&query, position.as_ref(), true, query.limit + 1, |entry| {
                items.push(entry);
                items.sort_by(|a, b| (b.observed_ms, &b.id).cmp(&(a.observed_ms, &a.id)));
                items.truncate(query.limit + 1);
            })?;
        let more = items.len() > query.limit;
        items.truncate(query.limit);
        let next_cursor = if more {
            items
                .last()
                .map(|e| encode_cursor(&query, &state.id, e))
                .transpose()?
        } else {
            None
        };
        if !query.include_protobuf {
            for e in &mut items {
                e.protobuf.clear();
            }
        }
        coverage.truncated |= more;
        Ok(Page {
            schema: "cowboy.logs.page/v1".into(),
            items,
            next_cursor,
            coverage,
        })
    }
    pub(crate) fn metrics(&self, query: &Query) -> Result<Metrics> {
        query.validate()?;
        ensure!(query.after.is_none(), "metrics do not accept a page cursor");
        let _lock = self.lock(false)?;
        let mut groups = std::collections::BTreeMap::new();
        let coverage = self.scan(query, None, false, MAX_SCAN, |e| {
            super::analysis::accumulate(&mut groups, &e)
        })?;
        Ok(Metrics {
            schema: "cowboy.logs.metrics/v1".into(),
            from_ms: query.from_ms,
            to_ms: query.to_ms,
            groups,
            coverage,
        })
    }
    fn scan(
        &self,
        query: &Query,
        position: Option<&(i64, String)>,
        blobs: bool,
        per_segment: usize,
        mut accept: impl FnMut(Entry),
    ) -> Result<Coverage> {
        let state = self.state()?;
        let segments = self.segments()?;
        let mut coverage = Coverage {
            segments: segments.len(),
            retained_bytes: segments.iter().map(|s| s.bytes).sum(),
            expired_records: state.expired_records,
            capacity_evicted_segments: state.capacity_evicted_segments,
            ..Default::default()
        };
        coverage.issues = super::capture::health_issues(&self.directory, query.from_ms)?;
        let start = Instant::now();
        let mut bytes = 0usize;
        for segment in segments.iter().rev() {
            let db = Self::connection(&segment.path, false)?;
            let (lo, hi): (Option<i64>, Option<i64>) = db.query_row(
                "SELECT min(observed_ms),max(observed_ms) FROM entries",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if let Some(lo) = lo {
                coverage.oldest_observed_ms =
                    Some(coverage.oldest_observed_ms.map_or(lo, |v| v.min(lo)));
            }
            if let Some(hi) = hi {
                coverage.newest_observed_ms =
                    Some(coverage.newest_observed_ms.map_or(hi, |v| v.max(hi)));
            }
            let projection = if blobs { "body,protobuf" } else { "'',x''" };
            let sql = format!(
                "SELECT id,observed_ms,timestamp_ms,signal,severity,event,service,session,machine,environment,trace_id,duration_ms,{projection} FROM entries WHERE observed_ms>=?1 AND observed_ms<?2 AND (?3 IS NULL OR session=?3) AND (?4 IS NULL OR machine=?4) AND (?5 IS NULL OR environment=?5) AND (?6 IS NULL OR trace_id=?6) AND (?7 IS NULL OR event=?7) AND (?8 IS NULL OR service=?8) AND (?9 IS NULL OR signal=?9) AND severity>=?10 AND (?11 IS NULL OR instr(body,?11)>0 OR instr(event,?11)>0) AND (?12 IS NULL OR (observed_ms,id)<(?12,?13)) AND (?15 IS NULL OR id=?15) ORDER BY observed_ms DESC,id DESC LIMIT ?14"
            );
            let mut statement = db.prepare(&sql)?;
            let rows = statement.query_map(
                params![
                    query.from_ms,
                    query.to_ms,
                    query.session,
                    query.machine,
                    query.environment,
                    query.trace_id,
                    query.event,
                    query.service,
                    query.signal,
                    query.min_severity,
                    query.contains,
                    position.map(|p| p.0),
                    position.map(|p| &p.1),
                    per_segment as i64 + 1,
                    query.id
                ],
                |r| {
                    Ok(Entry {
                        id: r.get(0)?,
                        observed_ms: r.get(1)?,
                        timestamp_ms: r.get(2)?,
                        signal: r.get(3)?,
                        severity: r.get(4)?,
                        event: r.get(5)?,
                        service: r.get(6)?,
                        session: r.get(7)?,
                        machine: r.get(8)?,
                        environment: r.get(9)?,
                        trace_id: r.get(10)?,
                        duration_ms: r.get(11)?,
                        body: r.get(12)?,
                        protobuf: r.get(13)?,
                        attributes: Default::default(),
                        metric: None,
                    })
                },
            )?;
            for (index, row) in rows.enumerate() {
                if index >= per_segment {
                    coverage.truncated = true;
                    break;
                }
                let mut row = row?;
                if blobs {
                    row.project_attributes()?;
                }
                bytes += row.protobuf.len() + row.body.len();
                if coverage.scanned >= MAX_SCAN
                    || bytes > 16 * 1024 * 1024
                    || start.elapsed() > Duration::from_secs(5)
                {
                    coverage.truncated = true;
                    coverage.issues.push("query_budget_exhausted".into());
                    return Ok(coverage);
                }
                coverage.scanned += 1;
                accept(row);
            }
        }
        if coverage
            .oldest_observed_ms
            .is_none_or(|v| query.from_ms < v)
        {
            coverage
                .issues
                .push("requested_window_precedes_retained_evidence".into());
        }
        Ok(coverage)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    store: String,
    filter: String,
    from: i64,
    to: i64,
    at: i64,
    id: String,
}
fn filter_hash(query: &Query) -> Result<String> {
    let mut query = query.clone();
    query.from_ms = 0;
    query.to_ms = 0;
    query.after = None;
    query.limit = 1;
    query.include_protobuf = false;
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(&query)?)))
}
fn encode_cursor(query: &Query, store: &str, e: &Entry) -> Result<String> {
    Ok(
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Cursor {
            store: store.into(),
            filter: filter_hash(query)?,
            from: query.from_ms,
            to: query.to_ms,
            at: e.observed_ms,
            id: e.id.clone(),
        })?),
    )
}
fn decode_cursor(query: &Query, store: &str) -> Result<(Query, Option<(i64, String)>)> {
    let mut query = query.clone();
    let Some(after) = query.after.take() else {
        return Ok((query, None));
    };
    let cursor: Cursor =
        serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(after)?)?;
    ensure!(
        cursor.store == store && cursor.filter == filter_hash(&query)? && cursor.id.len() == 32,
        "cursor belongs to another store or query"
    );
    query.from_ms = cursor.from;
    query.to_ms = cursor.to;
    query.validate()?;
    Ok((query, Some((cursor.at, cursor.id))))
}

#[cfg(feature = "full")]
impl super::EvidenceSink for SqliteStore {
    fn write(&mut self, records: &str, now: i64) -> Result<()> {
        ensure!(
            records.len() <= 1024 * 1024,
            "telemetry batch exceeds bounds"
        );
        let mut entries = Vec::new();
        for line in records.lines() {
            ensure!(
                line.len() <= super::data::MAX_RECORD_BYTES,
                "telemetry record exceeds bounds"
            );
            entries.push(Entry::legacy(&serde_json::from_str(line)?, now)?);
        }
        self.append(&entries, now)
    }
    fn maintain(&mut self, now: i64) -> Result<()> {
        SqliteStore::maintain_if_due(self, now)
    }
}
