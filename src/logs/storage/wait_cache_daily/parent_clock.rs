//! Correlate parent starts in event-time order within the retained horizon.
//! The clock is private. Each insertion can change only its own gap and that
//! of its immediate successor, including when they belong to different days.
use super::{
    Connection, Entry, PAGE_BYTES, Policy, Result, SqliteStore, Totals, add_totals, events, files,
    params, query_deadline, remove_totals, string,
};
use rusqlite::{OptionalExtension as _, Transaction};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::path::Path;
use std::time::{Duration, Instant};

pub(super) const HORIZON_MS: i64 = 7 * events::DAY_MS;
const IDLE_MS: i64 = 300_000;

pub(super) struct RequestStart {
    clock: String,
    started_at_ms: i64,
}

impl RequestStart {
    pub(super) fn of(entry: &Entry, now: i64) -> Option<Self> {
        if entry.event != events::REQUEST || string(entry, "role") != "parent" {
            return None;
        }
        let provider = string(entry, "provider");
        let (domain, identity, time) = if string(entry, "producer") == "gateway" {
            let fingerprint = string(entry, "session_fingerprint");
            if entry.service != "cowboy-machine"
                || string(entry, "session_attribution") != "explicit"
                || fingerprint.len() != 32
                || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())
                || !matches!(
                    provider,
                    "claude-deepseek" | "codex-deepseek" | "deepseek-harness"
                )
            {
                return None;
            }
            ("gateway", fingerprint, "request_started_at_ms_inferred")
        } else if entry.service == "cowboy-worker" && !entry.session.is_empty() {
            ("native", entry.session.as_str(), "request_started_at_ms")
        } else {
            return None;
        };
        let started_at_ms = entry.attributes.get(time)?.as_i64()?;
        if !(now.saturating_sub(HORIZON_MS)..=now).contains(&started_at_ms) {
            return None;
        }
        let clock = format!(
            "{:x}",
            Sha256::digest(format!("{domain}\0{provider}\0{identity}"))
        );
        Some(Self {
            clock,
            started_at_ms,
        })
    }

    pub(super) fn record(
        self,
        tx: &Transaction<'_>,
        entry: &Entry,
        day: i64,
        mut dimensions: Value,
        mut totals: Totals,
    ) -> Result<()> {
        let previous: Option<i64> = tx
            .query_row(
                "SELECT started_at_ms FROM wait_cache.parent_requests
             WHERE clock=?1 AND (started_at_ms,id)<(?2,?3)
             ORDER BY started_at_ms DESC,id DESC LIMIT 1",
                params![self.clock, self.started_at_ms, entry.id],
                |row| row.get(0),
            )
            .optional()?;
        let gap = previous
            .map(|start| self.started_at_ms - start)
            .filter(|gap| *gap <= HORIZON_MS);
        self.correct_successor(tx, &entry.id)?;
        set_gap(&mut dimensions, &mut totals, gap);
        let bucket = add_totals(tx, day, &dimensions, &totals)?;
        tx.execute(
            "INSERT INTO wait_cache.parent_requests VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                self.clock,
                self.started_at_ms,
                entry.id,
                day,
                serde_json::to_string(&dimensions)?,
                bucket,
                serde_json::to_string(&totals)?,
                gap
            ],
        )?;
        Ok(())
    }

    fn correct_successor(&self, tx: &Transaction<'_>, id: &str) -> Result<()> {
        let successor: Option<Successor> = tx
            .query_row(
                "SELECT id, started_at_ms, day, dimensions, bucket, totals, gap_ms
             FROM wait_cache.parent_requests WHERE clock=?1 AND (started_at_ms,id)>(?2,?3)
             ORDER BY started_at_ms,id LIMIT 1",
                params![self.clock, self.started_at_ms, id],
                |row| {
                    Ok(Successor {
                        id: row.get(0)?,
                        started_at_ms: row.get(1)?,
                        day: row.get(2)?,
                        dimensions: row.get(3)?,
                        bucket: row.get(4)?,
                        totals: row.get(5)?,
                        gap: row.get(6)?,
                    })
                },
            )
            .optional()?;
        if let Some(successor) = successor {
            successor.correct(tx, self.started_at_ms)?;
        }
        Ok(())
    }
}

struct Successor {
    id: String,
    started_at_ms: i64,
    day: i64,
    dimensions: String,
    bucket: String,
    totals: String,
    gap: Option<i64>,
}

impl Successor {
    fn correct(self, tx: &Transaction<'_>, predecessor_ms: i64) -> Result<()> {
        let gap = Some(self.started_at_ms - predecessor_ms).filter(|gap| *gap <= HORIZON_MS);
        if self.gap == gap {
            return Ok(());
        }
        let mut dimensions = serde_json::from_str(&self.dimensions)?;
        let mut totals = serde_json::from_str(&self.totals)?;
        let changed_cohort = cohort(self.gap) != cohort(gap);
        let bucket = if changed_cohort {
            remove_totals(tx, self.day, &self.bucket, &totals)?;
            set_gap(&mut dimensions, &mut totals, gap);
            add_totals(tx, self.day, &dimensions, &totals)?
        } else {
            self.bucket
        };
        tx.execute(
            "UPDATE wait_cache.parent_requests SET dimensions=?2, bucket=?3, totals=?4, gap_ms=?5 WHERE id=?1",
            params![self.id, serde_json::to_string(&dimensions)?, bucket, serde_json::to_string(&totals)?, gap],
        )?;
        Ok(())
    }
}

fn set_gap(dimensions: &mut Value, totals: &mut Totals, gap: Option<i64>) {
    dimensions["gap_cohort"] = cohort(gap).into();
    if let Some(gap) = gap {
        totals
            .values
            .insert("is_first_after_idle".into(), (u64::from(gap >= IDLE_MS), 1));
    } else {
        totals.values.remove("is_first_after_idle");
    }
}

fn cohort(gap: Option<i64>) -> &'static str {
    match gap {
        Some(0..=299_999) => "under_5m",
        Some(300_000..=1_799_999) => "5m_to_30m",
        Some(1_800_000..=3_599_999) => "30m_to_1h",
        Some(3_600_000..=43_199_999) => "1h_to_12h",
        Some(43_200_000..) => "12h_or_more",
        _ => "unknown",
    }
}

/// Copy finalized gaps into the segments that still own those records before
/// dropping the live clock rows. The store lock keeps segment inventory and
/// corrections stable. A partial copy can be retried from the same clock.
pub(super) fn finalize(
    store: &SqliteStore,
    policy: &Policy,
    rollup: &Connection,
    day_before: i64,
    clock_before: i64,
    started: Instant,
) -> Result<()> {
    let pending: bool = rollup.query_row(
        "SELECT EXISTS(SELECT 1 FROM parent_requests WHERE started_at_ms < ?2 AND gap_ms IS NOT NULL)
         OR EXISTS(SELECT 1 FROM seen s CROSS JOIN parent_requests p ON p.id=s.id
                   WHERE s.day < ?1 AND p.gap_ms IS NOT NULL)",
        params![day_before, clock_before],
        |row| row.get(0),
    )?;
    if !pending {
        return Ok(());
    }
    let path = rollup
        .path()
        .ok_or_else(|| anyhow::anyhow!("missing wait/cache rollup path"))?;
    let max_pages = policy.segment_bytes / PAGE_BYTES;
    for segment in store.segments()? {
        anyhow::ensure!(
            started.elapsed() < Duration::from_secs(5),
            "idle gap finalization exceeded its budget"
        );
        let mut detail = SqliteStore::connection(&segment.path, true)?;
        detail.pragma_update(None, "max_page_count", max_pages)?;
        query_deadline(&detail, started);
        detail.execute("ATTACH DATABASE ?1 AS wait_cache", [path])?;
        finalize_segment(&mut detail, day_before, clock_before)?;
    }
    Ok(())
}

const RETIRED_GAPS: &str = "
    SELECT e.id, gaps.gap_ms FROM (
        SELECT id, gap_ms FROM wait_cache.parent_requests WHERE started_at_ms < ?2
        UNION ALL
        SELECT p.id, p.gap_ms FROM wait_cache.seen s
        CROSS JOIN wait_cache.parent_requests p ON p.id = s.id
        WHERE s.day < ?1 AND p.started_at_ms >= ?2
    ) AS gaps
    CROSS JOIN entries e ON e.id = gaps.id WHERE gaps.gap_ms IS NOT NULL";

fn finalize_segment(db: &mut Connection, day_before: i64, clock_before: i64) -> Result<()> {
    let bounds = params![day_before, clock_before];
    let has_rows: bool =
        db.query_row(&format!("SELECT EXISTS({RETIRED_GAPS})"), bounds, |row| {
            row.get(0)
        })?;
    if has_rows {
        let tx = db.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS wait_cache_gaps (
                id TEXT PRIMARY KEY REFERENCES entries(id) ON DELETE CASCADE,
                gap_ms INTEGER NOT NULL
             ) WITHOUT ROWID",
        )?;
        tx.execute(
            &format!(
                "INSERT INTO wait_cache_gaps {RETIRED_GAPS}
                 ON CONFLICT(id) DO UPDATE SET gap_ms=excluded.gap_ms
                 WHERE gap_ms != excluded.gap_ms"
            ),
            bounds,
        )?;
        tx.commit()?;
    }
    Ok(())
}

pub(in crate::logs::storage) fn gap_projection(db: &Connection) -> Result<&'static str> {
    let has_archive: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='wait_cache_gaps')",
        [],
        |row| row.get(0),
    )?;
    Ok(if has_archive {
        "(SELECT gap_ms FROM wait_cache_gaps WHERE id=entries.id)"
    } else {
        "NULL"
    })
}

pub(in crate::logs::storage) fn add_gap_fields(entry: &mut Entry, gap: i64) -> Result<()> {
    entry.add_log_attributes(
        events::Fields::default()
            .integer("gap_since_last_parent_req_ms", u64::try_from(gap).ok())
            .boolean("is_first_after_idle", gap >= IDLE_MS)
            .0,
    )
}

/// Derive the exported log fields from the latest correction while retaining
/// the original local evidence. This also works across rotated detail segments.
pub(in crate::logs::storage) fn enrich(directory: &Path, items: &mut [Entry]) -> Result<()> {
    let parents = items.iter_mut().filter(|entry| {
        entry.event == events::REQUEST && super::tracks(entry) && string(entry, "role") == "parent"
    });
    let mut parents = parents.peekable();
    if parents.peek().is_none() {
        return Ok(());
    }
    let Some(db) = files::open(directory, true)? else {
        return Ok(());
    };
    query_deadline(&db, Instant::now());
    let mut stmt = db.prepare("SELECT gap_ms FROM parent_requests WHERE id=?1")?;
    for entry in parents {
        let gap: Option<Option<i64>> = stmt.query_row([&entry.id], |row| row.get(0)).optional()?;
        if let Some(gap) = gap.flatten() {
            add_gap_fields(entry, gap)?;
        }
    }
    Ok(())
}
