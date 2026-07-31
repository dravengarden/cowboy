//! Hawk-local, rebuildable content cache for saved Code files.
//!
//! Small files become content-addressed leaves when first opened. Metadata is
//! the fast invalidation key; SHA-256 is computed from bytes already being
//! read. Unsaved Zed buffers never enter this store.

use std::fs::File;
use std::io::{Read as _, Write as _};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension as _, params};
use sha2::{Digest as _, Sha256};

const MAX_CACHED_FILE_BYTES: u64 = 32 * 1024 * 1024;
const REVALIDATE_AFTER: Duration = Duration::from_secs(15);
const DIRECTORY_REVALIDATE_AFTER: Duration = Duration::from_secs(15);
const MAX_IDLE_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const HIGH_WATERMARK_PERCENT: u64 = 85;
const LOW_WATERMARK_PERCENT: u64 = 70;
const CACHE_SCHEMA_VERSION: i64 = 1;

#[derive(Debug)]
pub struct CachedFile {
    pub bytes: Vec<u8>,
    pub revision: String,
    pub size: u64,
}

#[derive(Debug)]
pub struct CachedDirectory {
    pub bytes: Vec<u8>,
    pub revision: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CodeCacheMetrics {
    pub bytes: u64,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

#[derive(Clone)]
pub struct CodeCache {
    inner: Arc<Inner>,
}

struct Inner {
    blobs: PathBuf,
    connection: Mutex<Connection>,
    eviction: Mutex<()>,
    quota_bytes: u64,
    bytes: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    temporary_sequence: AtomicU64,
}

#[derive(Debug)]
struct Leaf {
    fingerprint: Fingerprint,
    hash: String,
    validated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    dev: u64,
    ino: u64,
    size: u64,
    mtime_ns: i64,
    ctime_ns: i64,
}

impl CodeCache {
    pub fn open(root: PathBuf, quota_bytes: u64) -> Result<Self, String> {
        let blobs = root.join("blobs");
        std::fs::create_dir_all(&blobs).map_err(|error| error.to_string())?;
        remove_temporary_files(&blobs);
        let connection =
            Connection::open(root.join("index.sqlite3")).map_err(|error| error.to_string())?;
        let schema_version = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .map_err(|error| error.to_string())?;
        if schema_version != CACHE_SCHEMA_VERSION {
            connection
                .execute_batch(
                    "
                    DROP TABLE IF EXISTS directory_edges;
                    DROP TABLE IF EXISTS directory_nodes;
                    DROP TABLE IF EXISTS leaves;
                    DROP TABLE IF EXISTS blobs;
                    ",
                )
                .map_err(|error| error.to_string())?;
        }
        connection
            .execute_batch(
                "
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                PRAGMA foreign_keys = ON;
                CREATE TABLE IF NOT EXISTS blobs (
                    hash TEXT PRIMARY KEY,
                    size INTEGER NOT NULL,
                    last_access INTEGER NOT NULL,
                    hits INTEGER NOT NULL DEFAULT 1
                );
                CREATE TABLE IF NOT EXISTS leaves (
                    worktree TEXT NOT NULL,
                    path TEXT NOT NULL,
                    dev INTEGER NOT NULL,
                    ino INTEGER NOT NULL,
                    size INTEGER NOT NULL,
                    mtime_ns INTEGER NOT NULL,
                    ctime_ns INTEGER NOT NULL,
                    hash TEXT NOT NULL REFERENCES blobs(hash) ON DELETE CASCADE,
                    validated_at INTEGER NOT NULL,
                    last_access INTEGER NOT NULL,
                    hits INTEGER NOT NULL DEFAULT 1,
                    PRIMARY KEY (worktree, path)
                );
                CREATE INDEX IF NOT EXISTS leaves_hash ON leaves(hash);
                CREATE TABLE IF NOT EXISTS directory_nodes (
                    worktree TEXT NOT NULL,
                    path TEXT NOT NULL,
                    page_limit INTEGER NOT NULL,
                    hash TEXT NOT NULL REFERENCES blobs(hash) ON DELETE CASCADE,
                    merkle_hash TEXT NOT NULL,
                    revision TEXT NOT NULL,
                    validated_at INTEGER NOT NULL,
                    last_access INTEGER NOT NULL,
                    hits INTEGER NOT NULL DEFAULT 1,
                    PRIMARY KEY (worktree, path, page_limit)
                );
                CREATE INDEX IF NOT EXISTS directory_nodes_hash
                    ON directory_nodes(hash);
                CREATE TABLE IF NOT EXISTS directory_edges (
                    worktree TEXT NOT NULL,
                    parent_path TEXT NOT NULL,
                    page_limit INTEGER NOT NULL,
                    child_path TEXT NOT NULL,
                    child_kind TEXT NOT NULL,
                    PRIMARY KEY (worktree, parent_path, page_limit, child_path),
                    FOREIGN KEY (worktree, parent_path, page_limit)
                        REFERENCES directory_nodes(worktree, path, page_limit)
                        ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS directory_edges_child
                    ON directory_edges(worktree, child_path);
                CREATE INDEX IF NOT EXISTS blobs_eviction
                    ON blobs(hits, last_access);
                PRAGMA user_version = 1;
                ",
            )
            .map_err(|error| error.to_string())?;
        let bytes = total_bytes(&connection)?;
        let cache = Self {
            inner: Arc::new(Inner {
                blobs,
                connection: Mutex::new(connection),
                eviction: Mutex::new(()),
                quota_bytes,
                bytes: AtomicU64::new(bytes),
                hits: AtomicU64::new(0),
                misses: AtomicU64::new(0),
                evictions: AtomicU64::new(0),
                temporary_sequence: AtomicU64::new(0),
            }),
        };
        cache.reconcile()?;
        cache.evict_if_needed(None)?;
        Ok(cache)
    }

    pub fn metrics(&self) -> CodeCacheMetrics {
        CodeCacheMetrics {
            bytes: self.inner.bytes.load(Ordering::Relaxed),
            hits: self.inner.hits.load(Ordering::Relaxed),
            misses: self.inner.misses.load(Ordering::Relaxed),
            evictions: self.inner.evictions.load(Ordering::Relaxed),
        }
    }

    pub fn get_directory(
        &self,
        root: &Path,
        path: &str,
        page_limit: usize,
    ) -> Result<Option<CachedDirectory>, String> {
        if self.inner.quota_bytes == 0 {
            return Ok(None);
        }
        let worktree = root
            .canonicalize()
            .map_err(|error| format!("workspace unavailable: {error}"))?;
        let now = unix_seconds();
        let record = self
            .inner
            .connection
            .lock()
            .query_row(
                "SELECT hash, revision, validated_at FROM directory_nodes
                 WHERE worktree = ?1 AND path = ?2 AND page_limit = ?3",
                params![worktree.to_string_lossy(), path, page_limit],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((hash, revision, validated_at)) = record else {
            self.inner.misses.fetch_add(1, Ordering::Relaxed);
            return Ok(None);
        };
        if now.saturating_sub(validated_at)
            > i64::try_from(DIRECTORY_REVALIDATE_AFTER.as_secs()).unwrap_or(i64::MAX)
        {
            self.inner.misses.fetch_add(1, Ordering::Relaxed);
            return Ok(None);
        }
        let Ok(bytes) = std::fs::read(self.blob_path(&hash)) else {
            self.inner.misses.fetch_add(1, Ordering::Relaxed);
            return Ok(None);
        };
        if format!("{:x}", Sha256::digest(&bytes)) != hash {
            self.inner.misses.fetch_add(1, Ordering::Relaxed);
            return Ok(None);
        }
        let connection = self.inner.connection.lock();
        connection
            .execute(
                "UPDATE directory_nodes SET last_access = ?4, hits = hits + 1
                 WHERE worktree = ?1 AND path = ?2 AND page_limit = ?3",
                params![worktree.to_string_lossy(), path, page_limit, now],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE blobs SET last_access = ?2, hits = hits + 1 WHERE hash = ?1",
                params![hash, now],
            )
            .map_err(|error| error.to_string())?;
        self.inner.hits.fetch_add(1, Ordering::Relaxed);
        Ok(Some(CachedDirectory { bytes, revision }))
    }

    pub fn put_directory(
        &self,
        root: &Path,
        path: &str,
        page_limit: usize,
        revision: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        if self.inner.quota_bytes == 0 {
            return Ok(());
        }
        let worktree = root
            .canonicalize()
            .map_err(|error| format!("workspace unavailable: {error}"))?;
        let hash = format!("{:x}", Sha256::digest(bytes));
        self.publish_blob(&hash, bytes)?;
        let now = unix_seconds();
        let children = graph_children(bytes)?;
        let worktree_key = worktree.to_string_lossy();
        let mut connection = self.inner.connection.lock();
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO blobs(hash, size, last_access, hits)
                 VALUES (?1, ?2, ?3, 1)
                 ON CONFLICT(hash) DO UPDATE SET last_access = excluded.last_access",
                params![hash, bytes.len(), now],
            )
            .map_err(|error| error.to_string())?;
        let merkle_hash = node_merkle_hash(&transaction, &worktree_key, &hash, &children)?;
        transaction
            .execute(
                "INSERT INTO directory_nodes(
                    worktree, path, page_limit, hash, merkle_hash, revision,
                    validated_at, last_access, hits
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, 1)
                 ON CONFLICT(worktree, path, page_limit) DO UPDATE SET
                    hash = excluded.hash,
                    merkle_hash = excluded.merkle_hash,
                    revision = excluded.revision,
                    validated_at = excluded.validated_at,
                    last_access = excluded.last_access,
                    hits = directory_nodes.hits + 1",
                params![
                    worktree_key,
                    path,
                    page_limit,
                    hash,
                    merkle_hash,
                    revision,
                    now
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM directory_edges
                 WHERE worktree = ?1 AND parent_path = ?2 AND page_limit = ?3",
                params![worktree_key, path, page_limit],
            )
            .map_err(|error| error.to_string())?;
        for (child_path, child_kind) in &children {
            transaction
                .execute(
                    "INSERT INTO directory_edges(
                        worktree, parent_path, page_limit, child_path, child_kind
                     ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![worktree_key, path, page_limit, child_path, child_kind],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        self.inner
            .bytes
            .store(total_bytes(&connection)?, Ordering::Relaxed);
        drop(connection);
        self.refresh_ancestors(&worktree_key, path)?;
        self.evict_if_needed(Some(&hash))
    }

    /// Large files keep using the bounded direct page reader, represented by
    /// `None`; the cache never reads bytes the client did not request.
    pub fn get_or_load(&self, root: &Path, relative: &str) -> Result<Option<CachedFile>, String> {
        if self.inner.quota_bytes == 0 {
            return Ok(None);
        }
        let relative_path = safe_relative(relative)?;
        let worktree = root
            .canonicalize()
            .map_err(|error| format!("workspace unavailable: {error}"))?;
        let canonical_file = worktree
            .join(&relative_path)
            .canonicalize()
            .map_err(|_| "file not found".to_owned())?;
        if !canonical_file.starts_with(&worktree) || !canonical_file.is_file() {
            return Err("file not found".to_owned());
        }
        let metadata = canonical_file
            .metadata()
            .map_err(|error| error.to_string())?;
        if metadata.len() > MAX_CACHED_FILE_BYTES {
            return Ok(None);
        }
        let fingerprint = Fingerprint::from_metadata(&metadata);
        let worktree_key = worktree.to_string_lossy();
        let path_key = relative_path.to_string_lossy().replace('\\', "/");
        let now = unix_seconds();
        if let Some(leaf) = self.lookup(&worktree_key, &path_key)?
            && leaf.fingerprint == fingerprint
            && now.saturating_sub(leaf.validated_at)
                <= i64::try_from(REVALIDATE_AFTER.as_secs()).unwrap_or(i64::MAX)
            && let Ok(bytes) = std::fs::read(self.blob_path(&leaf.hash))
            && u64::try_from(bytes.len()).ok() == Some(fingerprint.size)
            && format!("{:x}", Sha256::digest(&bytes)) == leaf.hash
        {
            self.touch(&worktree_key, &path_key, &leaf.hash, now)?;
            self.inner.hits.fetch_add(1, Ordering::Relaxed);
            return Ok(Some(CachedFile {
                bytes,
                revision: leaf.hash,
                size: fingerprint.size,
            }));
        }

        self.inner.misses.fetch_add(1, Ordering::Relaxed);
        let before = Fingerprint::from_metadata(
            &canonical_file
                .metadata()
                .map_err(|error| error.to_string())?,
        );
        let mut bytes = Vec::with_capacity(usize::try_from(before.size).unwrap_or(0));
        File::open(&canonical_file)
            .and_then(|mut file| file.read_to_end(&mut bytes))
            .map_err(|error| error.to_string())?;
        let after = Fingerprint::from_metadata(
            &canonical_file
                .metadata()
                .map_err(|error| error.to_string())?,
        );
        if before != after || u64::try_from(bytes.len()).ok() != Some(after.size) {
            return Err("file snapshot changed".to_owned());
        }
        let hash = format!("{:x}", Sha256::digest(&bytes));
        self.publish_blob(&hash, &bytes)?;
        self.record_leaf(&worktree_key, &path_key, after, &hash, now)?;
        self.evict_if_needed(Some(&hash))?;
        Ok(Some(CachedFile {
            bytes,
            revision: hash,
            size: after.size,
        }))
    }

    fn lookup(&self, worktree: &str, path: &str) -> Result<Option<Leaf>, String> {
        self.inner
            .connection
            .lock()
            .query_row(
                "SELECT dev, ino, size, mtime_ns, ctime_ns, hash, validated_at
                 FROM leaves WHERE worktree = ?1 AND path = ?2",
                params![worktree, path],
                |row| {
                    Ok(Leaf {
                        fingerprint: Fingerprint {
                            dev: row.get(0)?,
                            ino: row.get(1)?,
                            size: row.get(2)?,
                            mtime_ns: row.get(3)?,
                            ctime_ns: row.get(4)?,
                        },
                        hash: row.get(5)?,
                        validated_at: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    fn touch(&self, worktree: &str, path: &str, hash: &str, now: i64) -> Result<(), String> {
        let connection = self.inner.connection.lock();
        connection
            .execute(
                "UPDATE leaves SET last_access = ?3, hits = hits + 1
                 WHERE worktree = ?1 AND path = ?2",
                params![worktree, path, now],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE blobs SET last_access = ?2, hits = hits + 1 WHERE hash = ?1",
                params![hash, now],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn record_leaf(
        &self,
        worktree: &str,
        path: &str,
        fingerprint: Fingerprint,
        hash: &str,
        now: i64,
    ) -> Result<(), String> {
        let mut connection = self.inner.connection.lock();
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO blobs(hash, size, last_access, hits)
                 VALUES (?1, ?2, ?3, 1)
                 ON CONFLICT(hash) DO UPDATE SET last_access = excluded.last_access",
                params![hash, fingerprint.size, now],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO leaves(
                    worktree, path, dev, ino, size, mtime_ns, ctime_ns, hash,
                    validated_at, last_access, hits
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, 1)
                 ON CONFLICT(worktree, path) DO UPDATE SET
                    dev = excluded.dev,
                    ino = excluded.ino,
                    size = excluded.size,
                    mtime_ns = excluded.mtime_ns,
                    ctime_ns = excluded.ctime_ns,
                    hash = excluded.hash,
                    validated_at = excluded.validated_at,
                    last_access = excluded.last_access,
                    hits = leaves.hits + 1",
                params![
                    worktree,
                    path,
                    fingerprint.dev,
                    fingerprint.ino,
                    fingerprint.size,
                    fingerprint.mtime_ns,
                    fingerprint.ctime_ns,
                    hash,
                    now
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        self.inner
            .bytes
            .store(total_bytes(&connection)?, Ordering::Relaxed);
        drop(connection);
        self.refresh_ancestors(worktree, path)?;
        Ok(())
    }

    fn publish_blob(&self, hash: &str, bytes: &[u8]) -> Result<(), String> {
        let path = self.blob_path(hash);
        if path.is_file() {
            return Ok(());
        }
        let parent = path
            .parent()
            .ok_or_else(|| "invalid blob path".to_owned())?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let sequence = self
            .inner
            .temporary_sequence
            .fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(".{hash}.{}.{}.tmp", std::process::id(), sequence));
        let mut file = File::create(&temporary).map_err(|error| error.to_string())?;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        match std::fs::rename(&temporary, &path) {
            Ok(()) => Ok(()),
            Err(_) if path.is_file() => {
                let _ = std::fs::remove_file(temporary);
                Ok(())
            }
            Err(error) => {
                let _ = std::fs::remove_file(temporary);
                Err(error.to_string())
            }
        }
    }

    fn blob_path(&self, hash: &str) -> PathBuf {
        self.inner.blobs.join(&hash[..2]).join(hash)
    }

    fn evict_if_needed(&self, protected_hash: Option<&str>) -> Result<(), String> {
        let _eviction = self.inner.eviction.lock();
        let high = self
            .inner
            .quota_bytes
            .saturating_mul(HIGH_WATERMARK_PERCENT)
            / 100;
        if self.inner.bytes.load(Ordering::Relaxed) <= high {
            return Ok(());
        }
        let low = self.inner.quota_bytes.saturating_mul(LOW_WATERMARK_PERCENT) / 100;
        while self.inner.bytes.load(Ordering::Relaxed) > low {
            let candidate = self
                .inner
                .connection
                .lock()
                .query_row(
                    "SELECT hash, size FROM blobs
                     WHERE (?1 IS NULL OR hash != ?1)
                     ORDER BY CASE WHEN hits <= 1 THEN 0 ELSE 1 END,
                              last_access ASC, size DESC
                     LIMIT 1",
                    [protected_hash],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            let Some((hash, size)) = candidate else {
                break;
            };
            self.remove_blob(&hash)?;
            self.inner.bytes.fetch_sub(size, Ordering::Relaxed);
            self.inner.evictions.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    fn reconcile(&self) -> Result<(), String> {
        let cutoff = unix_seconds()
            .saturating_sub(i64::try_from(MAX_IDLE_AGE.as_secs()).unwrap_or(i64::MAX));
        let hashes = {
            let connection = self.inner.connection.lock();
            let mut statement = connection
                .prepare("SELECT hash FROM blobs WHERE last_access < ?1")
                .map_err(|error| error.to_string())?;
            statement
                .query_map([cutoff], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        };
        for hash in hashes {
            self.delete_blob(&hash)?;
        }

        let indexed = {
            let connection = self.inner.connection.lock();
            let mut statement = connection
                .prepare("SELECT hash FROM blobs")
                .map_err(|error| error.to_string())?;
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?
                .collect::<Result<std::collections::HashSet<_>, _>>()
                .map_err(|error| error.to_string())?
        };
        for hash in &indexed {
            if !self.blob_path(hash).is_file() {
                self.remove_blob(hash)?;
            }
        }
        for path in blob_files(&self.inner.blobs) {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.len() == 64 && !indexed.contains(name) {
                let _ = std::fs::remove_file(path);
            }
        }
        self.inner.bytes.store(
            total_bytes(&self.inner.connection.lock())?,
            Ordering::Relaxed,
        );
        Ok(())
    }

    fn delete_blob(&self, hash: &str) -> Result<(), String> {
        self.remove_blob(hash)?;
        Ok(())
    }

    fn remove_blob(&self, hash: &str) -> Result<(), String> {
        let affected = {
            let connection = self.inner.connection.lock();
            let mut statement = connection
                .prepare(
                    "SELECT worktree, path FROM leaves WHERE hash = ?1
                     UNION
                     SELECT worktree, path FROM directory_nodes WHERE hash = ?1",
                )
                .map_err(|error| error.to_string())?;
            statement
                .query_map([hash], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        };
        self.inner
            .connection
            .lock()
            .execute("DELETE FROM blobs WHERE hash = ?1", [hash])
            .map_err(|error| error.to_string())?;
        let _ = std::fs::remove_file(self.blob_path(hash));
        for (worktree, path) in affected {
            self.refresh_ancestors(&worktree, &path)?;
        }
        Ok(())
    }

    fn refresh_ancestors(&self, worktree: &str, child_path: &str) -> Result<(), String> {
        let parents = {
            let connection = self.inner.connection.lock();
            let mut statement = connection
                .prepare(
                    "SELECT parent_path, page_limit FROM directory_edges
                     WHERE worktree = ?1 AND child_path = ?2",
                )
                .map_err(|error| error.to_string())?;
            statement
                .query_map(params![worktree, child_path], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        };
        for (parent_path, page_limit) in parents {
            let changed = {
                let connection = self.inner.connection.lock();
                let content_hash = connection
                    .query_row(
                        "SELECT hash FROM directory_nodes
                         WHERE worktree = ?1 AND path = ?2 AND page_limit = ?3",
                        params![worktree, parent_path, page_limit],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|error| error.to_string())?;
                let children = edge_children(&connection, worktree, &parent_path, page_limit)?;
                let next = node_merkle_hash(&connection, worktree, &content_hash, &children)?;
                connection
                    .execute(
                        "UPDATE directory_nodes SET merkle_hash = ?4
                         WHERE worktree = ?1 AND path = ?2 AND page_limit = ?3
                           AND merkle_hash != ?4",
                        params![worktree, parent_path, page_limit, next],
                    )
                    .map_err(|error| error.to_string())?
                    > 0
            };
            if changed {
                self.refresh_ancestors(worktree, &parent_path)?;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn directory_merkle_hash(&self, root: &Path, path: &str, page_limit: usize) -> String {
        let worktree = root.canonicalize().unwrap();
        self.inner
            .connection
            .lock()
            .query_row(
                "SELECT merkle_hash FROM directory_nodes
                 WHERE worktree = ?1 AND path = ?2 AND page_limit = ?3",
                params![worktree.to_string_lossy(), path, page_limit],
                |row| row.get(0),
            )
            .unwrap()
    }
}

impl Fingerprint {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            size: metadata.size(),
            mtime_ns: metadata.mtime().saturating_mul(1_000_000_000) + metadata.mtime_nsec(),
            ctime_ns: metadata.ctime().saturating_mul(1_000_000_000) + metadata.ctime_nsec(),
        }
    }
}

fn total_bytes(connection: &Connection) -> Result<u64, String> {
    connection
        .query_row("SELECT COALESCE(SUM(size), 0) FROM blobs", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())
}

fn graph_children(bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let mut children = value
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| {
            !entry
                .get("ignored")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        })
        .filter_map(|entry| {
            Some((
                entry.get("path")?.as_str()?.to_owned(),
                entry.get("kind")?.as_str()?.to_owned(),
            ))
        })
        .collect::<Vec<_>>();
    children.sort();
    Ok(children)
}

fn edge_children(
    connection: &Connection,
    worktree: &str,
    parent_path: &str,
    page_limit: usize,
) -> Result<Vec<(String, String)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT child_path, child_kind FROM directory_edges
             WHERE worktree = ?1 AND parent_path = ?2 AND page_limit = ?3
             ORDER BY child_path",
        )
        .map_err(|error| error.to_string())?;
    statement
        .query_map(params![worktree, parent_path, page_limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn node_merkle_hash(
    connection: &Connection,
    worktree: &str,
    content_hash: &str,
    children: &[(String, String)],
) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(b"cowboy-directory-v1");
    digest.update(content_hash.as_bytes());
    for (path, kind) in children {
        let child_hash = if kind == "directory" {
            connection
                .query_row(
                    "SELECT merkle_hash FROM directory_nodes
                     WHERE worktree = ?1 AND path = ?2
                     ORDER BY validated_at DESC LIMIT 1",
                    params![worktree, path],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?
        } else {
            connection
                .query_row(
                    "SELECT hash FROM leaves
                     WHERE worktree = ?1 AND path = ?2",
                    params![worktree, path],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?
        };
        digest.update([0]);
        digest.update(kind.as_bytes());
        digest.update([0]);
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(child_hash.as_deref().unwrap_or("unresolved").as_bytes());
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn safe_relative(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("invalid file path".to_owned());
    }
    Ok(path.to_path_buf())
}

fn remove_temporary_files(root: &Path) {
    let Ok(prefixes) = std::fs::read_dir(root) else {
        return;
    };
    for prefix in prefixes.flatten() {
        let Ok(entries) = std::fs::read_dir(prefix.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') && name.ends_with(".tmp") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn blob_files(root: &Path) -> Vec<PathBuf> {
    let Ok(prefixes) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    prefixes
        .flatten()
        .flat_map(|prefix| {
            std::fs::read_dir(prefix.path())
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
        })
        .filter(|path| path.is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("cowboy-code-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn caches_by_content_and_invalidates_changed_files() {
        let root = scratch("content");
        let worktree = root.join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join("main.rs"), "one\n").unwrap();
        let cache = CodeCache::open(root.join("cache"), 1024 * 1024).unwrap();
        let first = cache.get_or_load(&worktree, "main.rs").unwrap().unwrap();
        let second = cache.get_or_load(&worktree, "main.rs").unwrap().unwrap();
        assert_eq!(first.revision, second.revision);
        assert_eq!(cache.metrics().hits, 1);

        std::fs::write(worktree.join("main.rs"), "two-two\n").unwrap();
        let changed = cache.get_or_load(&worktree, "main.rs").unwrap().unwrap();
        assert_ne!(first.revision, changed.revision);
        assert_eq!(changed.bytes, b"two-two\n");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn evicts_probationary_content_at_the_high_watermark() {
        let root = scratch("eviction");
        let worktree = root.join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let cache = CodeCache::open(root.join("cache"), 100).unwrap();
        for index in 0..3 {
            let name = format!("{index}.txt");
            std::fs::write(worktree.join(&name), vec![b'a' + index; 40]).unwrap();
            cache.get_or_load(&worktree, &name).unwrap().unwrap();
        }
        assert!(cache.metrics().bytes <= 70);
        assert!(cache.metrics().evictions > 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn persists_lazy_directory_nodes() {
        let root = scratch("directory");
        let worktree = root.join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let cache_root = root.join("cache");
        let cache = CodeCache::open(cache_root.clone(), 1024 * 1024).unwrap();
        cache
            .put_directory(&worktree, "src", 200, "revision-1", br#"{"entries":[]}"#)
            .unwrap();
        drop(cache);

        let reopened = CodeCache::open(cache_root, 1024 * 1024).unwrap();
        let node = reopened
            .get_directory(&worktree, "src", 200)
            .unwrap()
            .unwrap();
        assert_eq!(node.revision, "revision-1");
        assert_eq!(node.bytes, br#"{"entries":[]}"#);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deduplicates_identical_file_content() {
        let root = scratch("deduplicate");
        let worktree = root.join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join("a.rs"), "same\n").unwrap();
        std::fs::write(worktree.join("b.rs"), "same\n").unwrap();
        let cache = CodeCache::open(root.join("cache"), 1024 * 1024).unwrap();
        cache.get_or_load(&worktree, "a.rs").unwrap().unwrap();
        cache.get_or_load(&worktree, "b.rs").unwrap().unwrap();
        assert_eq!(cache.metrics().bytes, 5);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_leaf_resolution_updates_ancestor_merkle_hash() {
        let root = scratch("merkle");
        let worktree = root.join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join("main.rs"), "fn main() {}\n").unwrap();
        let cache = CodeCache::open(root.join("cache"), 1024 * 1024).unwrap();
        cache
            .put_directory(
                &worktree,
                "",
                200,
                "tree-revision",
                br#"{"entries":[{"path":"main.rs","kind":"file","ignored":false}]}"#,
            )
            .unwrap();
        let unresolved = cache.directory_merkle_hash(&worktree, "", 200);
        cache.get_or_load(&worktree, "main.rs").unwrap().unwrap();
        let resolved = cache.directory_merkle_hash(&worktree, "", 200);
        assert_ne!(unresolved, resolved);
        std::fs::remove_dir_all(root).unwrap();
    }
}
