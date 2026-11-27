//! UTC rollups updated in the detail insert transaction, before either time or
//! capacity eviction can discard the detail. Side storage shares the store
//! budget and records failures independently of primary evidence.
use super::{Connection, PAGE_BYTES, Policy, Result, SqliteStore, params, query_deadline};
use crate::logs::data::Entry;
use crate::logs::wait_cache::{self as events, DAY_MS, LOSS, utc_day};
use rusqlite::{OptionalExtension as _, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

#[cfg(any(feature = "full", feature = "machine-host"))]
mod metrics;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use metrics::encode_wait_cache_otlp;

mod files;
mod parent_clock;
pub(super) use files::{Health, Space, attach};
pub(super) use parent_clock::{add_gap_fields, enrich, gap_projection};
const RETENTION_MS: i64 = 45 * DAY_MS;
pub(super) const MAX_APPEND_RECLAIMS: usize = (RETENTION_MS / DAY_MS) as usize + 1;
const MAX_GROUPS_PER_DAY: i64 = 512;
const MAX_REPORT_ROWS: usize = 6_000;
const MAX_REPORT_BYTES: usize = 6 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ReclaimOutcome {
    None,
    RetiredOlder,
    ReopenedActiveDay,
}

pub(super) fn tracks(entry: &Entry) -> bool {
    if entry.signal != "logs" {
        return false;
    }
    // Browser ingress admits custom event names but assigns its own service
    // identity. Only the producer that owns an event may contribute totals.
    let service = entry.service.as_str();
    match entry.event.as_str() {
        events::REQUEST => matches!(service, "cowboy-worker" | "cowboy-machine"),
        events::RECEIPT => service == "cowboy-worker",
        events::DISPATCH
        | events::SETTLED
        | events::DECISION
        | events::NOTICE
        | events::RECONCILIATION
        | "cowboy.context_rollover.history_read" => service == "cowboy-controller",
        events::COVERAGE => matches!(service, "cowboy-worker" | "cowboy-controller"),
        LOSS => matches!(
            service,
            "cowboy-worker" | "cowboy-machine" | "cowboy-controller"
        ),
        _ => false,
    }
}

pub(super) fn event_day(entry: &Entry, now: i64) -> i64 {
    utc_day(entry.timestamp_ms.min(now))
}

/// Returns false for an already recorded identity, even after its detail was
/// evicted or a subsequent segment was opened. No cross-host deduplication.
pub(super) fn insert(tx: &Transaction<'_>, entry: &Entry, now: i64) -> Result<bool> {
    let day = event_day(entry, now);
    if day < utc_day(now).saturating_sub(RETENTION_MS) {
        // Outside the diagnostic replay horizon; do not revive an expired
        // detailed attempt or create a partial historical cohort.
        return Ok(false);
    }
    if entry.event == events::RECONCILIATION {
        snapshot(tx, entry, day)?;
        return Ok(true);
    }
    if tx.execute(
        "INSERT OR IGNORE INTO wait_cache.seen VALUES (?1,?2)",
        params![entry.id, day],
    )? == 0
    {
        return Ok(false);
    }
    let dimensions = dimensions(entry);
    let totals = Totals::from_entry(entry);
    if let Some(clock) = parent_clock::RequestStart::of(entry, now) {
        clock.record(tx, entry, day, dimensions, totals)?;
    } else {
        add_totals(tx, day, &dimensions, &totals)?;
    }
    Ok(true)
}

const OVERFLOW: &str = r#"{"event":"group_overflow"}"#;

fn load_totals(tx: &Transaction<'_>, day: i64, key: &str) -> Result<Option<Totals>> {
    let raw: Option<String> = tx
        .query_row(
            "SELECT totals FROM wait_cache.rollups WHERE day=?1 AND dimensions=?2",
            params![day, key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(raw.map(|raw| serde_json::from_str(&raw)).transpose()?)
}

fn add_totals(
    tx: &Transaction<'_>,
    day: i64,
    dimensions: &Value,
    sample: &Totals,
) -> Result<String> {
    let mut key: String = serde_json::to_string(dimensions)?;
    let mut totals = if let Some(current) = load_totals(tx, day, &key)? {
        current
    } else {
        let groups: i64 = tx.query_row(
            "SELECT COUNT(*) FROM wait_cache.rollups WHERE day=?1",
            [day],
            |row| row.get(0),
        )?;
        if groups >= MAX_GROUPS_PER_DAY {
            key = OVERFLOW.to_owned();
            load_totals(tx, day, &key)?.unwrap_or_default()
        } else {
            Totals::default()
        }
    };
    totals.add(sample)?;
    save_totals(tx, day, &key, &totals)?;
    Ok(key)
}

fn remove_totals(tx: &Transaction<'_>, day: i64, key: &str, sample: &Totals) -> Result<()> {
    let mut totals =
        load_totals(tx, day, key)?.ok_or_else(|| anyhow::anyhow!("missing wait/cache cohort"))?;
    totals.remove(sample)?;
    if totals.records == 0 {
        tx.execute(
            "DELETE FROM wait_cache.rollups WHERE day=?1 AND dimensions=?2",
            params![day, key],
        )?;
    } else {
        save_totals(tx, day, key, &totals)?;
    }
    Ok(())
}

fn save_totals(tx: &Transaction<'_>, day: i64, key: &str, totals: &Totals) -> Result<()> {
    tx.execute(
        "INSERT INTO wait_cache.rollups VALUES (?1,?2,?3)
         ON CONFLICT(day, dimensions) DO UPDATE SET totals=excluded.totals",
        params![day, key, serde_json::to_string(totals)?],
    )?;
    Ok(())
}

fn snapshot(tx: &Transaction<'_>, entry: &Entry, day: i64) -> Result<()> {
    let fields = &entry.attributes;
    let snapshot = fields
        .get("snapshot_at_ms")
        .and_then(Value::as_i64)
        .unwrap_or(entry.observed_ms);
    let calls = fields.get("terminal_calls").and_then(Value::as_u64);
    let echoes = fields.get("durable_echoes").and_then(Value::as_u64);
    let counts = calls.zip(echoes).filter(|(calls, echoes)| echoes <= calls);
    let outcome = match (string(entry, "outcome"), counts) {
        ("observed", Some(_)) => "observed",
        ("truncated", Some(_)) => "truncated",
        _ => "unavailable",
    };
    let attempt = json!({"snapshot_at_ms": snapshot, "outcome": outcome});
    tx.execute(
        "INSERT INTO wait_cache.reconciliation_attempts VALUES (?1,?2,?3)
         ON CONFLICT(day) DO UPDATE SET snapshot_at_ms=excluded.snapshot_at_ms,report=excluded.report
         WHERE excluded.snapshot_at_ms >= reconciliation_attempts.snapshot_at_ms",
        params![day, snapshot, serde_json::to_string(&attempt)?],
    )?;
    let Some((calls, echoes)) = counts.filter(|_| outcome == "observed") else {
        return Ok(());
    };
    let report = json!({
        "from_ms": day,
        "to_ms": day + DAY_MS,
        "through_ms": snapshot.min(day + DAY_MS),
        "snapshot_at_ms": snapshot,
        "release": env!("CARGO_PKG_VERSION"),
        "outcome": outcome,
        "terminal_calls": calls,
        "durable_echoes": echoes,
        "echo_unobserved": calls - echoes,
    });
    tx.execute(
        "INSERT INTO wait_cache.reconciliation VALUES (?1,?2,?3)
         ON CONFLICT(day) DO UPDATE SET snapshot_at_ms=excluded.snapshot_at_ms,report=excluded.report
         WHERE excluded.snapshot_at_ms >= reconciliation.snapshot_at_ms",
        params![day, snapshot, serde_json::to_string(&report)?],
    )?;
    Ok(())
}

fn string<'a>(entry: &'a Entry, key: &str) -> &'a str {
    entry
        .attributes
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

fn allowed<'a>(value: &str, values: &'a [&str]) -> &'a str {
    values
        .iter()
        .copied()
        .find(|candidate| *candidate == value)
        .unwrap_or("unknown")
}

fn dimensions(entry: &Entry) -> Value {
    let producer = allowed(
        string(entry, "producer"),
        &[
            "gateway",
            "claude_otel",
            "codex_otel",
            "antigravity_proxy",
            "native_wait",
            "call_reconciliation",
        ],
    );
    let release = string(entry, "producer_release");
    let release = if release.len() <= 48
        && release
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
    {
        release
    } else {
        "unknown"
    };
    let path = allowed(
        string(entry, "wait_path"),
        &[
            "managed_call_notify",
            "managed_call_observe",
            "native_subagent",
        ],
    );
    let outcome = events::outcome(
        entry
            .attributes
            .get("outcome")
            .or_else(|| entry.attributes.get("cowboy.outcome"))
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
    );
    json!({
        "event": entry.event,
        "provider": events::provider(string(entry, "provider")),
        "producer": producer,
        "producer_schema_version": entry.attributes.get("producer_schema_version").and_then(Value::as_u64).filter(|v| *v <= 32),
        "producer_release": release,
        "cowboy_release": env!("CARGO_PKG_VERSION"),
        "role": events::role(string(entry, "role")),
        "input_semantics": allowed(string(entry, "input_semantics"), &["messages", "responses", "normalized_total"]),
        "outcome": outcome,
        "wait_path": path,
        "gap_cohort": "unknown",
        "first_after_rollover": entry.attributes.get("is_first_after_rollover").and_then(Value::as_bool),
        "first_after_compact": entry.attributes.get("is_first_after_compact").and_then(Value::as_bool),
        "retention_tier": allowed(string(entry, "retention_tier"), &["5m", "1h", "30m", "12h", "24h", "in_memory"]),
        "shadow_outcome": events::outcome(string(entry, "shadow_outcome")),
    })
}

#[derive(Default, Deserialize, Serialize)]
struct Totals {
    records: u64,
    // (sum, number of records supplying the value), including explicit zero.
    values: BTreeMap<String, (u64, u64)>,
}

impl Totals {
    fn from_entry(entry: &Entry) -> Self {
        let mut sample = Self {
            records: 1,
            ..Self::default()
        };
        for key in [
            "input_tokens",
            "output_tokens",
            "uncached_input_tokens",
            "cache_read_input_tokens",
            "cache_write_input_tokens",
            "cache_write_5m_input_tokens",
            "cache_write_1h_input_tokens",
            "cache_write_unknown_ttl_input_tokens",
            "estimated_api_equiv_microusd",
            "wait_duration_ms",
            "local_dropped_records",
            "local_write_failures",
            "producer_dropped",
            "producer_unmatched",
            "is_first_after_rollover",
            "is_first_after_compact",
            "is_first_in_worker_epoch",
            "backend_step_count",
            "transient_retry_count",
            "parent_ended_turn",
            "wake_needed",
            "cowboy.bytes",
            "cowboy.read.depth",
        ] {
            let Some(value) = entry
                .attributes
                .get(key)
                .and_then(|v| v.as_u64().or_else(|| v.as_bool().map(u64::from)))
            else {
                continue;
            };
            sample.values.insert(key.to_owned(), (value, 1));
        }
        sample
    }

    fn add(&mut self, sample: &Self) -> Result<()> {
        self.records += sample.records;
        for (key, &(value, count)) in &sample.values {
            let (sum, samples) = self.values.entry(key.to_owned()).or_default();
            *sum = sum
                .checked_add(value)
                .ok_or_else(|| anyhow::anyhow!("wait/cache total overflow"))?;
            *samples += count;
        }
        Ok(())
    }

    fn remove(&mut self, sample: &Self) -> Result<()> {
        self.records = subtract(self.records, sample.records)?;
        for (key, &(value, count)) in &sample.values {
            let (sum, samples) = self
                .values
                .get_mut(key)
                .ok_or_else(|| anyhow::anyhow!("missing wait/cache sample"))?;
            *sum = subtract(*sum, value)?;
            *samples = subtract(*samples, count)?;
            if *samples == 0 {
                self.values.remove(key);
            }
        }
        Ok(())
    }
}

fn subtract(total: u64, sample: u64) -> Result<u64> {
    total
        .checked_sub(sample)
        .ok_or_else(|| anyhow::anyhow!("invalid wait/cache cohort correction"))
}

fn retire_days(
    store: &SqliteStore,
    policy: &Policy,
    db: &mut Connection,
    now: i64,
    before: i64,
    capacity_floor: Option<i64>,
    started: Instant,
) -> Result<()> {
    let clock_before = before.max(now.saturating_sub(parent_clock::HORIZON_MS));
    parent_clock::finalize(store, policy, db, before, clock_before, started)?;
    let tx = db.transaction()?;
    tx.execute(
        "DELETE FROM parent_requests WHERE started_at_ms < ?1",
        [clock_before],
    )?;
    tx.execute(
        "DELETE FROM parent_requests WHERE id IN (SELECT id FROM seen WHERE day < ?1)",
        [before],
    )?;
    tx.execute("DELETE FROM seen WHERE day < ?1", [before])?;
    tx.execute("DELETE FROM rollups WHERE day < ?1", [before])?;
    tx.execute("DELETE FROM reconciliation WHERE day < ?1", [before])?;
    tx.execute(
        "DELETE FROM reconciliation_attempts WHERE day < ?1",
        [before],
    )?;
    if let Some(floor) = capacity_floor {
        tx.execute(
            "INSERT INTO capacity_floor VALUES (1,?1) ON CONFLICT(slot)
             DO UPDATE SET first_day = MAX(first_day,excluded.first_day)",
            [floor],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn retire_oldest(
    store: &SqliteStore,
    policy: &Policy,
    db: &mut Connection,
    now: i64,
    started: Instant,
    allow_reopen_active: bool,
) -> Result<ReclaimOutcome> {
    let oldest: Option<i64> = db.query_row(
        "SELECT MIN(day) FROM (
            SELECT MIN(day) AS day FROM seen UNION ALL
            SELECT MIN(day) FROM reconciliation UNION ALL
            SELECT MIN(day) FROM reconciliation_attempts
         )",
        [],
        |row| row.get(0),
    )?;
    let Some(day) = oldest else {
        return Ok(ReclaimOutcome::None);
    };
    let today = utc_day(now);
    let is_active = day >= today;
    if is_active && !allow_reopen_active {
        return Ok(ReclaimOutcome::None);
    }
    let before = day.saturating_add(DAY_MS);
    if is_active {
        // Reopen the active day for continued intake. Mark the lost prefix
        // durably before retiring its counts and their dedup keys together.
        Health::record_capacity_loss(&store.directory, now, day)?;
    }
    retire_days(
        store,
        policy,
        db,
        now,
        before,
        Some(before.min(today)),
        started,
    )?;
    Ok(if is_active {
        ReclaimOutcome::ReopenedActiveDay
    } else {
        ReclaimOutcome::RetiredOlder
    })
}

pub(super) fn reclaim_for_append(
    store: &SqliteStore,
    policy: &Policy,
    now: i64,
    started: Instant,
    allow_reopen_active: bool,
) -> Result<ReclaimOutcome> {
    let Some(mut db) = files::recover_for_maintenance(&store.directory, started)? else {
        return Ok(ReclaimOutcome::None);
    };
    retire_oldest(store, policy, &mut db, now, started, allow_reopen_active)
}

pub(super) fn write_batch_size(policy: &Policy) -> usize {
    // Clock rows can use overflow pages in addition to their indexes. Scale
    // transaction size with the side database so an empty small rollup can
    // accommodate a retry without swallowing an entire incoming batch.
    (files::data_limit(policy) / (4 * PAGE_BYTES)).clamp(1, 256) as usize
}

pub(super) fn maintain(store: &SqliteStore, policy: &Policy, now: i64) -> Result<()> {
    let started = Instant::now();
    let Some(mut db) = files::recover_for_maintenance(&store.directory, started)? else {
        return Ok(());
    };
    retire_days(
        store,
        policy,
        &mut db,
        now,
        utc_day(now).saturating_sub(RETENTION_MS),
        None,
        started,
    )?;
    let within_budget = |db: &Connection| -> Result<bool> {
        let pages = db.pragma_query_value(None, "page_count", |row| row.get::<_, u64>(0))?;
        Ok(pages <= files::data_limit(policy) / PAGE_BYTES)
    };
    // A reduced store budget can retire compact cohorts as a last resort.
    // Auto-vacuum returns their pages on commit without a second copy.
    let mut reopened_active = false;
    for _ in 0..=RETENTION_MS / DAY_MS {
        if within_budget(&db)? {
            return Ok(());
        }
        match retire_oldest(store, policy, &mut db, now, started, !reopened_active)? {
            ReclaimOutcome::RetiredOlder => {}
            ReclaimOutcome::ReopenedActiveDay => reopened_active = true,
            ReclaimOutcome::None => break,
        }
    }
    anyhow::ensure!(
        within_budget(&db)?,
        "wait/cache rollup disk budget exceeded"
    );
    Ok(())
}

pub(super) fn read_segment_incomplete_days(
    db: &Connection,
    from: Option<i64>,
    to: Option<i64>,
) -> Result<Vec<i64>> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='wait_cache_incomplete_days')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(Vec::new());
    }
    let mut days = Vec::new();
    match (from, to) {
        (Some(from), Some(to)) => {
            let mut stmt = db.prepare(
                "SELECT day FROM wait_cache_incomplete_days WHERE day>=?1 AND day<?2 ORDER BY day",
            )?;
            let mut rows = stmt.query(params![from, to])?;
            while let Some(row) = rows.next()? {
                days.push(row.get(0)?);
            }
        }
        _ => {
            let mut stmt = db.prepare("SELECT day FROM wait_cache_incomplete_days ORDER BY day")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                days.push(row.get(0)?);
            }
        }
    }
    Ok(days)
}

fn segment_degraded_days(store: &SqliteStore, from: i64, to: i64) -> Result<BTreeSet<i64>> {
    let started = Instant::now();
    let mut days = BTreeSet::new();
    for segment in store.segments()? {
        let db = SqliteStore::connection(&segment.path, false)?;
        query_deadline(&db, started);
        days.extend(read_segment_incomplete_days(&db, Some(from), Some(to))?);
    }
    Ok(days)
}

pub(super) fn capacity_floor(tx: &Transaction<'_>) -> Result<Option<i64>> {
    Ok(tx
        .query_row(
            "SELECT first_day FROM wait_cache.capacity_floor WHERE slot=1",
            [],
            |row| row.get(0),
        )
        .optional()?)
}

impl SqliteStore {
    pub fn wait_cache_daily(&self, from_ms: i64, to_ms: i64) -> Result<Value> {
        anyhow::ensure!(
            from_ms >= 0 && from_ms < to_ms && to_ms - from_ms <= 46 * DAY_MS,
            "invalid wait/cache daily window"
        );
        let from = utc_day(from_ms);
        let to = utc_day(to_ms - 1)
            .checked_add(DAY_MS)
            .ok_or_else(|| anyhow::anyhow!("invalid wait/cache daily window"))?;
        let _lock = self.lock(false)?;
        let mut issues = super::super::capture::health_issues(&self.directory, from)?;
        let degraded_days = match (
            Health::read(&self.directory),
            segment_degraded_days(self, from, to),
        ) {
            (Ok(health), Ok(mut segment_days)) => {
                segment_days.extend(health.degraded_days(from, to));
                Some(segment_days.into_iter().collect::<Vec<_>>())
            }
            _ => {
                issues.push("wait_cache_rollup_health_unavailable".into());
                None
            }
        };
        if degraded_days.as_ref().is_some_and(|days| !days.is_empty()) {
            issues.push("wait_cache_rollup_degraded".into());
        }
        let report = read_report(&self.directory, from, to).unwrap_or_else(|_| {
            issues.push("wait_cache_rollup_unavailable".into());
            Report {
                truncated: true,
                ..Report::default()
            }
        });
        if report.capacity_floor.is_some_and(|floor| from < floor) {
            issues.push("wait_cache_rollup_capacity_evicted".into());
        }
        Ok(json!({
            "schema": "cowboy.wait_cache.daily/v1",
            "from_ms": from, "to_ms": to, "retention_days": RETENTION_MS / DAY_MS,
            "truncated": report.truncated, "groups": report.groups,
            "capacity_floor_ms": report.capacity_floor,
            "degraded_days_ms": degraded_days,
            "call_notice_echo_reconciliation": report.snapshots, "coverage_issues": issues,
        }))
    }
}

#[derive(Default)]
struct Report {
    groups: Vec<Value>,
    snapshots: Vec<Value>,
    truncated: bool,
    capacity_floor: Option<i64>,
}

fn read_report(directory: &Path, from: i64, to: i64) -> Result<Report> {
    let mut report = Report::default();
    let Some(db) = files::open(directory, true)? else {
        return Ok(report);
    };
    query_deadline(&db, Instant::now());
    report.capacity_floor = db
        .query_row(
            "SELECT first_day FROM capacity_floor WHERE slot=1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    report.truncated = report.capacity_floor.is_some_and(|floor| from < floor);
    let mut stmt = db.prepare(
        "SELECT day,dimensions,totals FROM rollups WHERE day>=?1 AND day<?2 ORDER BY day DESC,dimensions LIMIT ?3"
    )?;
    let mut results = stmt.query(params![from, to, MAX_REPORT_ROWS + 1])?;
    let mut bytes = 0;
    while let Some(row) = results.next()? {
        let day: i64 = row.get(0)?;
        let cohort = json!({
            "from_ms": day, "to_ms": day + DAY_MS,
            "dimensions": serde_json::from_str::<Value>(&row.get::<_, String>(1)?)?,
            "totals": serde_json::from_str::<Value>(&row.get::<_, String>(2)?)?,
        });
        bytes += serde_json::to_vec(&cohort)?.len() + 1;
        if bytes > MAX_REPORT_BYTES || report.groups.len() == MAX_REPORT_ROWS {
            report.truncated = true;
            break;
        }
        report.truncated |= cohort["dimensions"]["event"] == "group_overflow";
        report.groups.push(cohort);
    }
    report.snapshots = read_snapshots(&db, from, to)?;
    Ok(report)
}

fn read_snapshots(db: &rusqlite::Connection, from: i64, to: i64) -> Result<Vec<Value>> {
    let mut stmt = db.prepare(
        "SELECT days.day, s.report, a.report FROM (
             SELECT day FROM reconciliation WHERE day>=?1 AND day<?2
             UNION SELECT day FROM reconciliation_attempts WHERE day>=?1 AND day<?2
         ) days LEFT JOIN reconciliation s ON s.day=days.day
                LEFT JOIN reconciliation_attempts a ON a.day=days.day ORDER BY days.day DESC",
    )?;
    let mut rows = stmt.query(params![from, to])?;
    let mut snapshots = Vec::new();
    while let Some(row) = rows.next()? {
        let day: i64 = row.get(0)?;
        let parse = |column| -> Result<Option<Value>> {
            Ok(row
                .get::<_, Option<String>>(column)?
                .map(|s| serde_json::from_str::<Value>(&s))
                .transpose()?)
        };
        let saved = parse(1)?;
        let latest = parse(2)?.or_else(|| {
            saved.as_ref().map(|s| {
                json!({
                    "snapshot_at_ms": s["snapshot_at_ms"], "outcome": s["outcome"],
                })
            })
        });
        let mut summary: Value = saved
            .filter(|s| s["outcome"] == "observed")
            .unwrap_or_else(|| json!({"from_ms": day, "to_ms": day + DAY_MS}));
        summary["latest_attempt"] = json!(latest);
        snapshots.push(summary);
    }
    Ok(snapshots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::data::{Query, text};
    use opentelemetry_proto::tonic::{
        common::v1::InstrumentationScope, logs::v1::LogRecord, resource::v1::Resource,
    };

    fn entry(service: &str, name: &str, id: u8, time: i64, fields: events::Fields) -> Entry {
        let mut entry = Entry::log(
            Resource {
                attributes: vec![text("service.name", service)],
                ..Default::default()
            },
            InstrumentationScope::default(),
            LogRecord {
                event_name: name.to_owned(),
                time_unix_nano: u64::try_from(time).unwrap() * 1_000_000,
                attributes: fields.0,
                ..Default::default()
            },
            time + 30,
        );
        entry.id = format!("{id:032x}");
        entry
    }

    fn attempt(id: u8, time: i64, role: &str) -> Entry {
        entry(
            "cowboy-worker",
            events::REQUEST,
            id,
            time,
            events::Fields::default()
                .string("cowboy.session.id", "private-session")
                .string("provider", "codex")
                .string("producer", "codex_otel")
                .string("role", role)
                .string("outcome", "usage")
                .integer("request_started_at_ms", u64::try_from(time).unwrap())
                .integer("input_tokens", 100_u64)
                .boolean("is_first_after_compact", true)
                .boolean("is_first_after_rollover", true),
        )
    }

    #[test]
    fn clock_uses_parent_starts_and_daily_rollups_survive_eviction_and_replay() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        store
            .configure(super::super::Policy {
                retain_seconds: 60,
                rotate_seconds: 60,
                ..Default::default()
            })
            .unwrap();
        let day = 10 * DAY_MS;
        let start = day + 1_000;
        let mut first = attempt(1, start, "parent");
        first.observed_ms = start + 50_000;
        // First completion may be far later than its recorded start.
        store
            .append(std::slice::from_ref(&first), start + 50_000)
            .unwrap();
        let keepalive = attempt(2, start + 220_000, "keepalive");
        store.append(&[keepalive], start + 230_000).unwrap();
        let last = attempt(3, start + 310_000, "parent");
        store
            .append(&[first, last.clone()], start + 315_000)
            .unwrap();
        let detail = store
            .query(
                &serde_json::from_value::<Query>(
                    json!({"from_ms": day,"to_ms": day + DAY_MS,"limit": 100,"id": last.id}),
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(detail.items.len(), 1);
        let attrs = &detail.items[0].attributes;
        assert_eq!(attrs["gap_since_last_parent_req_ms"], 310_000);
        assert_eq!(attrs["is_first_after_idle"], true);
        assert_eq!(attrs["is_first_after_compact"], true);
        assert_eq!(attrs["is_first_after_rollover"], true);

        store.maintain(day + DAY_MS).unwrap();
        assert!(
            store
                .query(
                    &serde_json::from_value::<Query>(
                        json!({"from_ms": day,"to_ms": day + DAY_MS,"limit": 100})
                    )
                    .unwrap()
                )
                .unwrap()
                .items
                .is_empty()
        );
        // Deduplication persists after the original segment is deleted.
        store.append(&[last], day + DAY_MS + 12_000).unwrap();
        let daily = store.wait_cache_daily(day, day + DAY_MS).unwrap();
        let groups = daily["groups"].as_array().unwrap();
        assert_eq!(
            groups
                .iter()
                .map(|g| g["totals"]["records"].as_u64().unwrap())
                .sum::<u64>(),
            3
        );
        assert!(!daily.to_string().contains("private-session"));
        assert!(!daily["truncated"].as_bool().unwrap());
    }

    #[test]
    fn a_client_event_with_a_reserved_name_stays_out_of_provider_rollups() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        let day = 10 * DAY_MS;
        let browser = entry(
            "cowboy-web",
            events::REQUEST,
            2,
            day + 1_000,
            events::Fields::default()
                .string("service.name", "cowboy-worker")
                .string("provider", "codex")
                .string("producer", "codex_otel")
                .string("role", "parent")
                .string("outcome", "usage")
                .integer("input_tokens", 20_000_u64),
        );
        store
            .append(&[attempt(1, day + 900, "parent"), browser], day + 1_500)
            .unwrap();
        let report = store.wait_cache_daily(day, day + DAY_MS).unwrap();
        let groups = report["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["totals"]["records"], 1);
        assert_eq!(groups[0]["totals"]["values"]["input_tokens"][0], 100);
        let query = serde_json::from_value(json!({
            "from_ms": day, "to_ms": day + DAY_MS, "limit": 10, "event": events::REQUEST,
        }))
        .unwrap();
        assert_eq!(store.query(&query).unwrap().items.len(), 2);
    }

    #[test]
    fn daily_window_rejects_an_unrepresentable_upper_day_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        assert!(store.wait_cache_daily(i64::MAX - 1, i64::MAX).is_err());
    }

    #[test]
    fn reconciliation_is_a_revisable_utc_snapshot_instead_of_event_counts() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        let day = 11 * DAY_MS;
        let snapshot = |id, age, echoes| {
            entry(
                "cowboy-controller",
                events::RECONCILIATION,
                id,
                day,
                events::Fields::default()
                    .string("outcome", "observed")
                    .integer("snapshot_at_ms", u64::try_from(day + age).unwrap())
                    .integer("terminal_calls", 10_u64)
                    .integer("durable_echoes", echoes),
            )
        };
        store
            .append(
                &[snapshot(1, 1200, 8_u64), snapshot(2, 1400, 9_u64)],
                day + 1500,
            )
            .unwrap();
        store
            .append(&[snapshot(3, 1000, 6_u64)], day + 1600)
            .unwrap();
        let unavailable = entry(
            "cowboy-controller",
            events::RECONCILIATION,
            4,
            day,
            events::Fields::default()
                .string("outcome", "unavailable")
                .integer("snapshot_at_ms", u64::try_from(day + 1700).unwrap()),
        );
        store.append(&[unavailable], day + 1800).unwrap();
        let report = store.wait_cache_daily(day, day + DAY_MS).unwrap();
        let values = report["call_notice_echo_reconciliation"]
            .as_array()
            .unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["durable_echoes"], 9);
        assert_eq!(values[0]["through_ms"], day + 1400);
        assert_eq!(values[0]["latest_attempt"]["outcome"], "unavailable");
        assert_eq!(values[0]["latest_attempt"]["snapshot_at_ms"], day + 1700);
    }

    #[test]
    fn truncated_reconciliation_keeps_complete_counts_and_has_no_coverage_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        let day = 11 * DAY_MS;
        let snapshot = |id, cohort, at, outcome, calls| {
            entry(
                "cowboy-controller",
                events::RECONCILIATION,
                id,
                cohort,
                events::Fields::default()
                    .string("outcome", outcome)
                    .integer("snapshot_at_ms", u64::try_from(at).unwrap())
                    .integer("terminal_calls", calls)
                    .integer("durable_echoes", 8_u64),
            )
        };
        let next = day + DAY_MS;
        store
            .append(
                &[
                    snapshot(1, day, day + 1_000, "observed", 10_u64),
                    snapshot(2, day, next + 100, "truncated", 10_001_u64),
                    snapshot(3, next, next + 200, "truncated", 10_001_u64),
                ],
                next + 250,
            )
            .unwrap();
        let report = store.wait_cache_daily(day, next + DAY_MS).unwrap();
        let new = &report["call_notice_echo_reconciliation"][0];
        assert_eq!(new["from_ms"], next);
        assert_eq!(new["latest_attempt"]["outcome"], "truncated");
        assert!(new.get("through_ms").is_none());
        assert!(new.get("terminal_calls").is_none());
        let old = &report["call_notice_echo_reconciliation"][1];
        assert_eq!(old["terminal_calls"], 10);
        assert_eq!(old["through_ms"], day + 1_000);
        assert_eq!(old["latest_attempt"]["outcome"], "truncated");
        assert_eq!(old["latest_attempt"]["snapshot_at_ms"], next + 100);
    }

    #[test]
    fn corrupt_rollup_degrades_coverage_without_blocking_maintenance() {
        use std::os::unix::fs::OpenOptionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        let day = 10 * DAY_MS;
        let file = store.directory.join(files::FILE);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file)
            .unwrap();
        std::fs::write(&file, b"damaged observation database").unwrap();
        // This is the startup maintenance path used by logs::init.
        store.maintain_if_due(day).unwrap();
        let status = store.status().unwrap();
        assert_eq!(
            status["wait_cache_rollup"]["health"]["maintenance_failures"],
            1
        );
        let daily = store.wait_cache_daily(day, day + DAY_MS).unwrap();
        assert_eq!(daily["truncated"], true);
        assert!(
            daily["coverage_issues"]
                .to_string()
                .contains("wait_cache_rollup_unavailable")
        );
        assert_eq!(
            std::fs::read(file).unwrap(),
            b"damaged observation database"
        );
    }

    #[test]
    fn reduced_budget_retires_old_cohorts_and_rejects_rollup_replays() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        let day = 10 * DAY_MS;
        store
            .append(&[attempt(1, day + 100, "parent")], day + 200)
            .unwrap();
        let mut db = files::open(&store.directory, false).unwrap().unwrap();
        let tx = db.transaction().unwrap();
        for id in 0..2200 {
            let id = format!("fixture-{id:0120}");
            tx.execute("INSERT INTO seen VALUES (?1,?2)", params![id, day])
                .unwrap();
        }
        tx.commit().unwrap();
        drop(db);
        let policy = Policy {
            max_bytes: 2 * 1024 * 1024,
            segment_bytes: 1024 * 1024,
            ..Policy::default()
        };
        assert!(Space::read(&store.directory, &policy, false).over_budget);
        store.configure(policy.clone()).unwrap();
        store.maintain(day + DAY_MS + 300).unwrap();
        assert!(!Space::read(&store.directory, &policy, false).over_budget);
        let report = store.wait_cache_daily(day, day + DAY_MS).unwrap();
        assert_eq!(report["capacity_floor_ms"], day + DAY_MS);
        assert_eq!(report["truncated"], true);
        assert_eq!(report["groups"], json!([]));

        let late = attempt(2, day + 150, "parent");
        store.append(&[late], day + DAY_MS + 400).unwrap();
        let replayed = store.wait_cache_daily(day, day + DAY_MS).unwrap();
        assert_eq!(replayed["groups"], json!([]));
        store
            .append(
                &[attempt(3, day + DAY_MS + 500, "parent")],
                day + DAY_MS + 600,
            )
            .unwrap();
        let resumed = store
            .wait_cache_daily(day + DAY_MS, day + 2 * DAY_MS)
            .unwrap();
        assert_eq!(resumed["groups"][0]["totals"]["records"], 1);
        assert_eq!(resumed["truncated"], false);
    }

    #[test]
    fn sqlite_full_retires_closed_days_and_steady_ingestion_continues() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        let policy = Policy {
            max_bytes: 2 * 1024 * 1024,
            segment_bytes: 1024 * 1024,
            ..Policy::default()
        };
        store.configure(policy.clone()).unwrap();
        let start = 10 * DAY_MS;
        for index in 0..8 {
            let day = start + index * DAY_MS;
            ingest_parent_batch(&store, day, usize::try_from(index).unwrap() * 12, 12);
            let active = store.wait_cache_daily(day, day + DAY_MS).unwrap();
            assert_eq!(active["degraded_days_ms"], json!([]));
            assert_eq!(records_in(&active), 12);
        }
        let first = store.wait_cache_daily(start, start + DAY_MS).unwrap();
        assert_eq!(first["truncated"], true);
        assert!(first["capacity_floor_ms"].as_i64().unwrap() > start);
        let status = store.status().unwrap();
        assert_eq!(
            status["wait_cache_rollup"]["health"]["failed_rollup_records"],
            0
        );
        assert!(status["retained_bytes"].as_u64().unwrap() <= policy.max_bytes);
        assert!(!Space::read(&store.directory, &policy, false).over_budget);
    }

    #[test]
    fn sqlite_full_reopens_a_busy_active_day_with_explicit_loss_coverage() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        let policy = Policy {
            max_bytes: 2 * 1024 * 1024,
            segment_bytes: 1024 * 1024,
            ..Policy::default()
        };
        store.configure(policy.clone()).unwrap();
        let day = 10 * DAY_MS;
        for index in 0..16 {
            ingest_parent_batch(&store, day, index * 32, 32);
        }
        let report = store.wait_cache_daily(day, day + DAY_MS).unwrap();
        assert_eq!(report["degraded_days_ms"], json!([day]));
        assert!(records_in(&report) > 0);
        let side = files::open(&store.directory, true).unwrap().unwrap();
        let last: bool = side
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM seen WHERE id=?1)",
                [format!("{:032x}", 511)],
                |row| row.get(0),
            )
            .unwrap();
        assert!(last, "the final batch should still commit to the rollup");
        assert_eq!(
            store.status().unwrap()["wait_cache_rollup"]["health"]["failed_rollup_records"],
            0
        );
        assert!(!Space::read(&store.directory, &policy, false).over_budget);
    }

    fn ingest_parent_batch(store: &SqliteStore, day: i64, first: usize, len: usize) {
        let entries: Vec<_> = (first..first + len)
            .map(|index| {
                let mut row = attempt(0, day + i64::try_from(index).unwrap() * 100, "parent");
                row.id = format!("{index:032x}");
                row
            })
            .collect();
        store
            .append(&entries, entries.last().unwrap().observed_ms + 100)
            .unwrap();
    }

    fn records_in(report: &Value) -> u64 {
        report["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["totals"]["records"].as_u64().unwrap())
            .sum()
    }

    #[test]
    fn rollup_transaction_failure_preserves_warnings_and_marks_tracked_detail() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        let day = 10 * DAY_MS;
        store
            .append(&[attempt(1, day + 1_000, "parent")], day + 1_100)
            .unwrap();
        files::open(&store.directory, false).unwrap().unwrap().execute_batch(
            "CREATE TRIGGER fail_rollup BEFORE INSERT ON seen BEGIN SELECT RAISE(ABORT,'fixture'); END;",
        ).unwrap();
        let mut warning = entry(
            "cowboy-worker",
            "fixture.warning",
            2,
            day + 1_500,
            events::Fields::default(),
        );
        warning.severity = 13;
        let tracked = attempt(3, day + 1_600, "parent");
        store
            .append(&[tracked.clone(), warning.clone()], day + 1_700)
            .unwrap();
        let query =
            serde_json::from_value(json!({"from_ms": day, "to_ms": day+DAY_MS, "limit":10}))
                .unwrap();
        let detail = store.query(&query).unwrap();
        assert_eq!(
            detail
                .items
                .iter()
                .find(|e| e.id == warning.id)
                .unwrap()
                .severity,
            13
        );
        assert_eq!(
            detail
                .items
                .iter()
                .find(|e| e.id == tracked.id)
                .unwrap()
                .attributes["wait_cache.rollup_status"],
            "failed"
        );
        let daily = store.wait_cache_daily(day, day + DAY_MS).unwrap();
        assert_eq!(daily["groups"][0]["totals"]["records"], 1);
        assert!(
            daily["coverage_issues"]
                .to_string()
                .contains("wait_cache_rollup_degraded")
        );
    }

    #[test]
    fn degradation_tracks_affected_days_and_suppresses_only_incomplete_cohorts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs");
        let store = SqliteStore::open(path.clone(), true).unwrap();
        let first = 10 * DAY_MS;
        let second = first + DAY_MS;
        let healthy = second + DAY_MS;
        for (id, day) in [(1, first), (2, second)] {
            store
                .append(&[attempt(id, day + 100, "keepalive")], day + 200)
                .unwrap();
        }
        let delayed = attempt(4, first + 300, "keepalive");
        let current = attempt(5, second + 300, "keepalive");
        let side = files::open(&store.directory, false).unwrap().unwrap();
        side.execute_batch(&format!(
            "CREATE TRIGGER fail_rollup BEFORE INSERT ON seen
             WHEN NEW.id IN ('{}','{}') BEGIN SELECT RAISE(ABORT,'fixture'); END;",
            delayed.id, current.id
        ))
        .unwrap();
        drop(side);
        store.append(&[delayed], second + 400).unwrap();
        let first_status = store.wait_cache_daily(first, healthy).unwrap();
        assert_eq!(first_status["degraded_days_ms"], json!([first]));

        store.append(&[current], healthy + 400).unwrap();
        store
            .append(&[attempt(3, healthy + 500, "keepalive")], healthy + 600)
            .unwrap();
        let reopened = SqliteStore::open(path, false).unwrap();
        let summary = reopened.wait_cache_daily(first, healthy + DAY_MS).unwrap();
        assert_eq!(summary["degraded_days_ms"], json!([first, second]));
        assert_eq!(summary["truncated"], false);
        assert_eq!(records_in(&summary), 3);
        #[cfg(any(feature = "full", feature = "machine-host"))]
        {
            for day in [first, second] {
                let error = reopened
                    .wait_cache_otlp(day, healthy + DAY_MS, true)
                    .unwrap_err();
                assert!(error.to_string().contains("incomplete rollup coverage"));
            }
            assert!(
                !reopened
                    .wait_cache_otlp(healthy, healthy + DAY_MS, true)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn detail_and_rollup_reservation_share_the_configured_budget() {
        use std::os::unix::fs::OpenOptionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        let policy = super::super::Policy {
            segment_bytes: 1024 * 1024,
            max_bytes: 2 * 1024 * 1024,
            rotate_seconds: 60,
            ..Default::default()
        };
        store.configure(policy.clone()).unwrap();
        let day = 10 * DAY_MS;
        for id in 1..6 {
            let now = day + i64::from(id) * 61_000;
            store
                .append(&[attempt(id, now - 50, "parent")], now)
                .unwrap();
            let mut details = Vec::new();
            for i in 0..100 {
                let mut row = entry(
                    "cowboy-worker",
                    "fixture.detail",
                    0,
                    now,
                    events::Fields::default(),
                );
                row.id = format!("{:032x}", 1024 + usize::from(id) * 100 + i);
                row.body = "x".repeat(8000);
                details.push(row);
            }
            store.append(&details, now).unwrap();
        }
        let journal = store.directory.join(format!("{}-journal", files::FILE));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(journal)
            .unwrap()
            .set_len(4096)
            .unwrap();
        let status = store.status().unwrap();
        let bytes = status["retained_bytes"].as_u64().unwrap();
        let space = &status["wait_cache_rollup"];
        let detail: u64 = store.segments().unwrap().iter().map(|s| s.bytes).sum();
        assert!(status["capacity_evicted_segments"].as_u64().unwrap() > 0);
        assert_eq!(space["recovery_bytes"], 4096);
        assert_eq!(bytes, detail + space["data_bytes"].as_u64().unwrap() + 4096);
        assert!(detail + space["reserved_bytes"].as_u64().unwrap() <= policy.max_bytes);
    }

    #[test]
    fn late_parent_start_corrects_the_successor_cohort_and_detail() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        let day = 10 * DAY_MS;
        let initial = attempt(1, day, "parent");
        let sixth = attempt(2, day + 360_000, "parent");
        store
            .append(&[initial, sixth.clone()], day + 361_000)
            .unwrap();
        assert!(
            store
                .wait_cache_daily(day, day + DAY_MS)
                .unwrap()
                .to_string()
                .contains("5m_to_30m")
        );
        store
            .append(&[attempt(3, day + 180_000, "parent")], day + 362_000)
            .unwrap();
        let query = serde_json::from_value(
            json!({"from_ms":day, "to_ms":day+DAY_MS, "id":sixth.id, "limit":1}),
        )
        .unwrap();
        let detail = store.query(&query).unwrap();
        assert_eq!(
            detail.items[0].attributes["gap_since_last_parent_req_ms"],
            180_000
        );
        assert_eq!(detail.items[0].attributes["is_first_after_idle"], false);
        let groups = store.wait_cache_daily(day, day + DAY_MS).unwrap();
        assert!(!groups.to_string().contains("5m_to_30m"));
        let corrected = groups["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["dimensions"]["gap_cohort"] == "under_5m")
            .unwrap();
        assert_eq!(corrected["totals"]["records"], 2);
        assert_eq!(
            corrected["totals"]["values"]["input_tokens"],
            json!([200, 2])
        );
    }

    #[test]
    fn corrected_idle_fields_survive_clock_pruning_and_export_with_retained_detail() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        store
            .configure(Policy {
                retain_seconds: 14 * 86_400,
                rotate_seconds: 60,
                ..Policy::default()
            })
            .unwrap();
        let day = 10 * DAY_MS;
        let successor = attempt(2, day + 360_000, "parent");
        store
            .append(
                &[attempt(1, day, "parent"), successor.clone()],
                day + 361_000,
            )
            .unwrap();
        let mut late = attempt(3, day + 180_000, "parent");
        late.observed_ms = day + 430_000;
        store.append(&[late], day + 431_000).unwrap();
        assert_eq!(store.segments().unwrap().len(), 2);
        let query: Query = serde_json::from_value(json!({
            "from_ms":day, "to_ms":day + DAY_MS, "id":successor.id, "limit":1, "include_protobuf":true
        })).unwrap();
        assert_eq!(
            store.query(&query).unwrap().items[0].attributes["gap_since_last_parent_req_ms"],
            180_000
        );
        store.maintain(day + 8 * DAY_MS).unwrap();
        let side = files::open(&store.directory, true).unwrap().unwrap();
        let live: i64 = side
            .query_row("SELECT COUNT(*) FROM parent_requests", [], |row| row.get(0))
            .unwrap();
        assert_eq!(live, 0);
        let mut detail = store.query(&query).unwrap().items.remove(0);
        assert_eq!(detail.attributes["gap_since_last_parent_req_ms"], 180_000);
        assert_eq!(detail.attributes["is_first_after_idle"], false);
        detail.attributes.clear();
        detail.project_attributes().unwrap();
        assert_eq!(detail.attributes["gap_since_last_parent_req_ms"], 180_000);
        assert_eq!(detail.attributes["is_first_after_idle"], false);
    }

    #[test]
    fn history_outcome_and_all_produced_shadow_decisions_have_finite_cohorts() {
        let day = 10 * DAY_MS;
        let history = entry(
            "cowboy-controller",
            "cowboy.context_rollover.history_read",
            1,
            day,
            events::Fields::default().string("cowboy.outcome", "ok"),
        );
        assert_eq!(dimensions(&history)["outcome"], "ok");
        for outcome in [
            "renewed",
            "disabled",
            "notify_held",
            "would_pass_preflight",
            "cache_warm",
        ] {
            let decision = entry(
                "cowboy-controller",
                events::DECISION,
                2,
                day,
                events::Fields::default().string("shadow_outcome", outcome),
            );
            assert_eq!(dimensions(&decision)["shadow_outcome"], outcome);
        }
    }

    #[test]
    fn only_explicit_gateway_parents_share_an_inferred_start_clock() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        let day = 10 * DAY_MS;
        let request = |id, provider, attribution, role, start| {
            entry(
                "cowboy-machine",
                events::REQUEST,
                id,
                day + start + 100,
                events::Fields::default()
                    .string("producer", "gateway")
                    .string("provider", provider)
                    .string("role", role)
                    .string("session_attribution", attribution)
                    .string("session_fingerprint", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                    .integer(
                        "request_started_at_ms_inferred",
                        u64::try_from(day + start).unwrap(),
                    ),
            )
        };
        let first = request(1, "codex-deepseek", "explicit", "parent", 1000);
        let other = request(2, "claude-deepseek", "explicit", "parent", 100_000);
        let lineage = request(3, "codex-deepseek", "response_lineage", "parent", 200_000);
        let keepalive = request(4, "codex-deepseek", "explicit", "keepalive", 210_000);
        let last = request(5, "codex-deepseek", "explicit", "parent", 320_000);
        store
            .append(
                &[first, other, lineage, keepalive, last.clone()],
                day + 321_000,
            )
            .unwrap();
        let query = serde_json::from_value(
            json!({"from_ms":day, "to_ms":day+DAY_MS, "id":last.id, "limit":1}),
        )
        .unwrap();
        let detail = store.query(&query).unwrap();
        assert_eq!(
            detail.items[0].attributes["gap_since_last_parent_req_ms"],
            319_000
        );
        assert_eq!(detail.items[0].attributes["is_first_after_idle"], true);
        let report = store.wait_cache_daily(day, day + DAY_MS).unwrap();
        assert!(
            !report
                .to_string()
                .contains("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
    }
}
