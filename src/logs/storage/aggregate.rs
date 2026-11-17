//! Metrics aggregated inside each SQLite segment.
//!
//! A per-record scan stopped at `MAX_SCAN` records: on OVH one per-call
//! overhead event made up 97% of the store, so a 7-day `metrics` request
//! covered only its newest 21 hours. Grouping in SQLite reads the indexed
//! columns once and returns one row per group, so the whole retained window
//! fits the same 5-second budget. Results equal the record scan's, newest
//! evidence first.

use super::super::analysis::{BUCKETS, series};
use super::super::data::{Coverage, Query, Series};
use super::{SqliteStore, query_deadline, query_step};
use anyhow::Result;
use rusqlite::{Connection, params};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// The scan filters without a page position; `?12` selects one evidence ID.
const FILTER: &str = "observed_ms>=?1 AND observed_ms<?2 AND (?3 IS NULL OR session=?3) AND (?4 IS NULL OR machine=?4) AND (?5 IS NULL OR environment=?5) AND (?6 IS NULL OR trace_id=?6) AND (?7 IS NULL OR event=?7) AND (?8 IS NULL OR service=?8) AND (?9 IS NULL OR signal=?9) AND severity>=?10 AND (?11 IS NULL OR instr(body,?11)>0 OR instr(event,?11)>0) AND (?12 IS NULL OR id=?12)";

/// Evidence IDs a group keeps, as the record scan keeps them.
const EVIDENCE: usize = 4;

fn totals_sql() -> String {
    // Cumulative counts at each boundary; their differences are the
    // explicit histogram's buckets, the remainder its +Inf bucket.
    let cumulative: String = BUCKETS
        .iter()
        .map(|upper| format!(",sum(duration_ms<={upper:?})"))
        .collect();
    format!(
        "SELECT service,signal,event,count(*),sum(severity>=17),sum(severity>=13 AND severity<17),count(duration_ms),total(duration_ms),max(duration_ms){cumulative} FROM entries WHERE {FILTER} GROUP BY service,signal,event ORDER BY max(observed_ms) DESC"
    )
}

fn evidence_sql() -> String {
    format!(
        "SELECT service,signal,event,id,failure,n,f FROM (SELECT service,signal,event,id,observed_ms,severity>=13 AS failure,row_number() OVER (PARTITION BY service,signal,event ORDER BY observed_ms DESC,id DESC) AS n,row_number() OVER (PARTITION BY service,signal,event,severity>=13 ORDER BY observed_ms DESC,id DESC) AS f FROM entries WHERE {FILTER}) WHERE n<={EVIDENCE} OR (failure AND f<={EVIDENCE}) ORDER BY observed_ms DESC,id DESC"
    )
}

macro_rules! filter_params {
    ($query:expr) => {
        params![
            $query.from_ms,
            $query.to_ms,
            $query.session,
            $query.machine,
            $query.environment,
            $query.trace_id,
            $query.event,
            $query.service,
            $query.signal,
            $query.min_severity,
            $query.contains,
            $query.id
        ]
    };
}

impl SqliteStore {
    pub(super) fn aggregate(
        &self,
        query: &Query,
        groups: &mut BTreeMap<String, Series>,
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
        coverage.issues = super::super::capture::health_issues(&self.directory, query.from_ms)?;
        let start = Instant::now();
        let (totals, evidence) = (totals_sql(), evidence_sql());
        for segment in segments.iter().rev() {
            if start.elapsed() >= Duration::from_secs(5) {
                coverage.truncated = true;
                coverage.issues.push("query_budget_exhausted".into());
                return Ok(coverage);
            }
            let db = Self::connection(&segment.path, false)?;
            query_deadline(&db, start);
            if !segment_totals(&db, &totals, query, groups, &mut coverage)?
                || !segment_evidence(&db, &evidence, query, groups, &mut coverage)?
            {
                return Ok(coverage);
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

/// Adds one segment's groups; false once the budget interrupted SQLite.
fn segment_totals(
    db: &Connection,
    sql: &str,
    query: &Query,
    groups: &mut BTreeMap<String, Series>,
    coverage: &mut Coverage,
) -> Result<bool> {
    let limits: rusqlite::Result<(Option<i64>, Option<i64>)> = db.query_row(
        "SELECT min(observed_ms),max(observed_ms) FROM entries",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    );
    let Some((lo, hi)) = query_step(limits, coverage)? else {
        return Ok(false);
    };
    if let Some(lo) = lo {
        coverage.oldest_observed_ms = Some(coverage.oldest_observed_ms.map_or(lo, |v| v.min(lo)));
    }
    if let Some(hi) = hi {
        coverage.newest_observed_ms = Some(coverage.newest_observed_ms.map_or(hi, |v| v.max(hi)));
    }
    let mut statement = db.prepare(sql)?;
    let rows = statement.query_map(filter_params!(query), |r| {
        let mut cumulative = Vec::with_capacity(BUCKETS.len());
        for index in 0..BUCKETS.len() {
            cumulative.push(r.get::<_, Option<i64>>(9 + index)?.unwrap_or(0));
        }
        Ok((
            (
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ),
            [
                r.get::<_, i64>(3)?,
                r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                r.get::<_, i64>(6)?,
            ],
            (r.get::<_, f64>(7)?, r.get::<_, Option<f64>>(8)?),
            cumulative,
        ))
    })?;
    for row in rows {
        let Some(((service, signal, event), counts, (sum, max), cumulative)) =
            query_step(row, coverage)?
        else {
            return Ok(false);
        };
        let [count, errors, warnings, durations] = counts.map(|v| u64::try_from(v).unwrap_or(0));
        let target = series(groups, &service, &signal, &event);
        target.count += count;
        target.errors += errors;
        target.warnings += warnings;
        target.duration_count += durations;
        target.duration_sum_ms += sum;
        if let Some(max) = max {
            target.duration_max_ms = target.duration_max_ms.max(max);
        }
        let mut below = 0;
        for (index, at) in cumulative.iter().enumerate() {
            let at = u64::try_from(*at).unwrap_or(0);
            target.duration_buckets[index] += at.saturating_sub(below);
            below = at;
        }
        target.duration_buckets[BUCKETS.len()] += durations.saturating_sub(below);
        coverage.scanned = coverage.scanned.saturating_add(count as usize);
    }
    Ok(true)
}

/// Adds the newest evidence IDs, as a newest-first record scan would.
fn segment_evidence(
    db: &Connection,
    sql: &str,
    query: &Query,
    groups: &mut BTreeMap<String, Series>,
    coverage: &mut Coverage,
) -> Result<bool> {
    let mut statement = db.prepare(sql)?;
    let rows = statement.query_map(filter_params!(query), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, bool>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, i64>(6)?,
        ))
    })?;
    for row in rows {
        let Some((service, signal, event, id, failure, n, f)) = query_step(row, coverage)? else {
            return Ok(false);
        };
        let target = series(groups, &service, &signal, &event);
        if n <= EVIDENCE as i64 && target.evidence_ids.len() < EVIDENCE {
            target.evidence_ids.push(id.clone());
        }
        if failure && f <= EVIDENCE as i64 && target.failure_evidence_ids.len() < EVIDENCE {
            target.failure_evidence_ids.push(id);
        }
    }
    Ok(true)
}
