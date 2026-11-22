//! Primary evidence and optional wait/cache projection have separate commit
//! paths. A sidecar failure leaves attributed detail and primary evidence.
use super::{
    Connection, Entry, PAGE_BYTES, Policy, Result, Segment, SqliteStore, State,
    detail_segment_limit, ensure, params, transaction_chunks, wait_cache_daily,
};
use crate::logs::wait_cache::Fields;
use rusqlite::Statement;
use std::collections::BTreeSet;
use std::time::Instant;

const INSERT: &str =
    "INSERT OR IGNORE INTO entries VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)";

enum RollupWrite {
    Committed,
    Full,
}

impl SqliteStore {
    pub fn append(&self, entries: &[Entry], now: i64) -> Result<()> {
        ensure!(entries.len() <= 256 && now >= 0, "log batch exceeds bounds");
        for entry in entries {
            entry.validate()?;
        }
        let _lock = self.lock(false)?;
        let mut state = self.state()?;
        let chunks = transaction_chunks(entries, detail_segment_limit(&state.policy), |entry| {
            (entry.protobuf.len() + entry.body.len() + 2048) as u64
        });
        for chunk in chunks {
            self.append_chunk(chunk, &mut state, now)?;
        }
        Ok(())
    }

    fn append_chunk(&self, chunk: &[Entry], state: &mut State, now: i64) -> Result<()> {
        let (tracked, ordinary): (Vec<_>, Vec<_>) = chunk
            .iter()
            .partition(|entry| wait_cache_daily::tracks(entry));
        let segment = self.append_segment(chunk, state, now, !tracked.is_empty())?;
        if !ordinary.is_empty() {
            write_details(&mut writer(&segment, &state.policy)?, ordinary)?;
        }
        for tracked in tracked.chunks(wait_cache_daily::write_batch_size(&state.policy)) {
            if self
                .append_rollup(&segment, tracked, &state.policy, now)
                .is_err()
            {
                let _ =
                    wait_cache_daily::Health::record_failed_rollup(&self.directory, now, tracked);
                let days: BTreeSet<i64> = tracked
                    .iter()
                    .map(|entry| wait_cache_daily::event_day(entry, now))
                    .collect();
                let retained = tracked
                    .iter()
                    .map(|entry| with_rollup_status(entry, "failed"))
                    .collect::<Result<Vec<_>>>()?;
                write_fallback_details(&mut writer(&segment, &state.policy)?, &retained, &days)?;
            }
        }
        Ok(())
    }

    fn append_segment(
        &self,
        chunk: &[Entry],
        state: &mut State,
        now: i64,
        tracked: bool,
    ) -> Result<Segment> {
        let mut segments = self.segments()?;
        let detail_limit = detail_segment_limit(&state.policy);
        let estimate: u64 = chunk
            .iter()
            .map(|entry| (entry.protobuf.len() + entry.body.len() + 2048) as u64)
            .sum::<u64>()
            * 2
            + 32 * 1024;
        let space = wait_cache_daily::Space::read(&self.directory, &state.policy, tracked);
        let limit = space.primary_limit(&state.policy);
        let reserved_bytes = |segment: &Segment| {
            // Account for wait_cache_gaps archival growth during reclaim/maintenance
            // so append-triggered finalization stays within primary_limit, while
            // never undercounting an existing segment larger than a lowered cap.
            let headroom = (segment.bytes / 7).saturating_add(4 * PAGE_BYTES);
            segment
                .bytes
                .saturating_add(headroom)
                .min(state.policy.segment_bytes)
                .max(segment.bytes)
        };
        let mut retained: u64 = segments.iter().map(reserved_bytes).sum();
        let current = segments
            .last()
            .filter(|segment| {
                now >= segment.created_ms
                    && now - segment.created_ms < state.policy.rotate_seconds as i64 * 1000
                    && segment.bytes + estimate <= detail_limit
            })
            .cloned();
        let growth = if current.is_some() {
            estimate.saturating_add(estimate / 7)
        } else {
            state.policy.segment_bytes
        };
        let keep = usize::from(current.is_some());
        while segments.len() > keep
            && (retained.saturating_add(growth) > limit
                || segments.len() >= super::MAX_SEGMENTS + keep)
        {
            let oldest = segments.remove(0);
            self.remove(&oldest, now)?;
            retained = retained.saturating_sub(reserved_bytes(&oldest));
            state.capacity_evicted_segments += 1;
            self.save(state)?;
        }
        match current {
            Some(segment) => Ok(segment),
            None => self.create_segment(now, &state.policy),
        }
    }

    fn append_rollup(
        &self,
        segment: &Segment,
        entries: &[&Entry],
        policy: &Policy,
        now: i64,
    ) -> Result<()> {
        let started = Instant::now();
        let mut reopened_active = false;
        for attempt in 0..=wait_cache_daily::MAX_APPEND_RECLAIMS {
            match self.try_append_rollup(segment, entries, policy, now)? {
                RollupWrite::Committed => return Ok(()),
                RollupWrite::Full => {
                    if attempt == wait_cache_daily::MAX_APPEND_RECLAIMS {
                        break;
                    }
                    match wait_cache_daily::reclaim_for_append(
                        self,
                        policy,
                        now,
                        started,
                        !reopened_active,
                    )? {
                        wait_cache_daily::ReclaimOutcome::RetiredOlder => {}
                        wait_cache_daily::ReclaimOutcome::ReopenedActiveDay => {
                            reopened_active = true;
                        }
                        wait_cache_daily::ReclaimOutcome::None => break,
                    }
                }
            }
        }
        anyhow::bail!("wait/cache rollup capacity exhausted")
    }

    fn try_append_rollup(
        &self,
        segment: &Segment,
        entries: &[&Entry],
        policy: &Policy,
        now: i64,
    ) -> Result<RollupWrite> {
        let mut db = writer(segment, policy)?;
        wait_cache_daily::attach(&db, &self.directory, policy)?;
        let tx = db.transaction()?;
        let capacity_floor = wait_cache_daily::capacity_floor(&tx)?;
        {
            let mut insert = tx.prepare_cached(INSERT)?;
            for entry in entries {
                let mut projected;
                let entry = if entry.attributes.is_empty() {
                    projected = (*entry).clone();
                    projected.project_attributes()?;
                    &projected
                } else {
                    entry
                };
                if capacity_floor.is_some_and(|floor| entry.timestamp_ms.min(now) < floor) {
                    write_entry(&mut insert, &with_rollup_status(entry, "capacity_evicted")?)?;
                    continue;
                }
                // Only rollup allocation is recoverable by dropping old
                // cohorts. A primary database write error takes its usual path.
                match wait_cache_daily::insert(&tx, entry, now) {
                    Ok(true) => write_entry(&mut insert, entry)?,
                    Ok(false) => {}
                    Err(error)
                        if error
                            .downcast_ref::<rusqlite::Error>()
                            .and_then(rusqlite::Error::sqlite_error_code)
                            == Some(rusqlite::ErrorCode::DiskFull) =>
                    {
                        return Ok(RollupWrite::Full);
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        tx.commit()?;
        Ok(RollupWrite::Committed)
    }
}

fn with_rollup_status(entry: &Entry, status: &str) -> Result<Entry> {
    let mut entry = entry.clone();
    entry.add_log_attributes(
        Fields::default()
            .string("wait_cache.rollup_status", status)
            .0,
    )?;
    Ok(entry)
}

fn writer(segment: &Segment, policy: &Policy) -> Result<Connection> {
    let db = SqliteStore::connection(&segment.path, true)?;
    db.pragma_update(
        None,
        "max_page_count",
        detail_segment_limit(policy) / PAGE_BYTES,
    )?;
    Ok(db)
}

fn write_fallback_details(
    db: &mut Connection,
    entries: &[Entry],
    incomplete_days: &BTreeSet<i64>,
) -> Result<()> {
    let tx = db.transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS wait_cache_incomplete_days (day INTEGER PRIMARY KEY) WITHOUT ROWID",
    )?;
    {
        let mut mark = tx
            .prepare_cached("INSERT OR IGNORE INTO wait_cache_incomplete_days (day) VALUES (?1)")?;
        for day in incomplete_days {
            mark.execute([day])?;
        }
        let mut insert = tx.prepare_cached(INSERT)?;
        for entry in entries {
            write_entry(&mut insert, entry)?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn write_details<'a>(
    db: &mut Connection,
    entries: impl IntoIterator<Item = &'a Entry>,
) -> Result<()> {
    let tx = db.transaction()?;
    {
        let mut insert = tx.prepare_cached(INSERT)?;
        for entry in entries {
            write_entry(&mut insert, entry)?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn write_entry(insert: &mut Statement<'_>, entry: &Entry) -> Result<()> {
    insert.execute(params![
        entry.id,
        entry.observed_ms,
        entry.timestamp_ms,
        entry.signal,
        entry.severity,
        entry.event,
        entry.service,
        entry.session,
        entry.machine,
        entry.environment,
        entry.trace_id,
        entry.duration_ms,
        entry.body,
        entry.protobuf,
    ])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::data::{Query, text};
    use crate::logs::wait_cache::{self as events, DAY_MS};
    use opentelemetry_proto::tonic::{
        common::v1::{AnyValue, InstrumentationScope, any_value},
        logs::v1::LogRecord,
        resource::v1::Resource,
    };
    use serde_json::json;

    fn parent_request(id: u32, start_ms: i64, session: &str, body_pad: usize) -> Entry {
        let mut entry = Entry::log(
            Resource {
                attributes: vec![text("service.name", "cowboy-worker")],
                ..Default::default()
            },
            InstrumentationScope::default(),
            LogRecord {
                event_name: events::REQUEST.to_owned(),
                time_unix_nano: u64::try_from(start_ms + 100).unwrap() * 1_000_000,
                body: Some(AnyValue {
                    value: Some(any_value::Value::StringValue("x".repeat(body_pad))),
                }),
                attributes: Fields::default()
                    .string("cowboy.session.id", session)
                    .string("provider", "codex")
                    .string("producer", "codex_otel")
                    .string("role", "parent")
                    .string("outcome", "usage")
                    .integer("request_started_at_ms", u64::try_from(start_ms).unwrap())
                    .integer("input_tokens", 100_u64)
                    .0,
                ..Default::default()
            },
            start_ms + 100,
        );
        entry.id = format!("{id:032x}");
        entry
    }

    #[test]
    fn failed_health_write_during_rollup_fallback_still_suppresses_closed_day_export() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        let day = 10 * DAY_MS;
        store
            .append(&[parent_request(1, day + 1_000, "s1", 16)], day + 2_000)
            .unwrap();
        // Corrupt the side DB so the next tracked batch takes the fallback path.
        std::fs::write(
            store.directory.join("wait-cache-daily-v1.sqlite"),
            b"corrupt",
        )
        .unwrap();
        store
            .append(&[parent_request(2, day + 5_000, "s1", 16)], day + 6_000)
            .unwrap();
        // Simulate a failed health-file replacement (e.g. ENOSPC leaving the
        // previous clean health file intact while the primary segment committed).
        super::super::atomic_json(
            &store.directory.join("wait-cache-health-v1.json"),
            &json!({
                "maintenance_failures": 0,
                "failed_rollup_records": 0,
                "last_failure_ms": null,
                "degraded_days_ms": []
            }),
        )
        .unwrap();
        let report = store.wait_cache_daily(day, day + DAY_MS).unwrap();
        assert_eq!(report["degraded_days_ms"], json!([day]));
        assert!(
            report["coverage_issues"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "wait_cache_rollup_degraded")
        );
        #[cfg(any(feature = "full", feature = "machine-host"))]
        assert!(store.wait_cache_otlp(day, day + 2 * DAY_MS, false).is_err());

        // When the primary segment ages out beyond the 45-day retention horizon
        // and is removed, its incomplete-day marker is still folded into
        // degraded_through_ms before unlink so historical exports stay suppressed.
        store.maintain(day + 60 * DAY_MS).unwrap();
        assert!(store.segments().unwrap().is_empty());
        let after_evict = store.wait_cache_daily(day, day + DAY_MS).unwrap();
        assert_eq!(after_evict["degraded_days_ms"], json!([day]));
        #[cfg(any(feature = "full", feature = "machine-host"))]
        assert!(
            store
                .wait_cache_otlp(day, day + 60 * DAY_MS, false)
                .is_err()
        );
    }

    #[test]
    fn lowered_segment_bytes_still_evicts_oversized_existing_segment() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        store
            .configure(Policy {
                retain_seconds: 14 * 86_400,
                rotate_seconds: 60,
                segment_bytes: 4 * 1024 * 1024,
                max_bytes: 16 * 1024 * 1024,
                forward_runtime: false,
                ..Policy::default()
            })
            .unwrap();
        let day = 10 * DAY_MS;
        // Write >1.5 MiB into a single 4 MiB segment.
        for chunk in 0..8_u32 {
            let batch: Vec<_> = (0..32_u32)
                .map(|j| {
                    let id = chunk * 32 + j + 1;
                    parent_request(id, day + i64::from(id) * 100, "big", 4_500)
                })
                .collect();
            store.append(&batch, day + 30_000).unwrap();
        }
        let first = store.segments().unwrap();
        assert_eq!(first.len(), 1);
        assert!(first[0].bytes > 1_500_000);
        // Lower segment_bytes to 1 MiB and max_bytes to 3 MiB (primary_limit
        // 2.25 MiB): counting the real >1.5 MiB file plus 1 MiB new segment
        // exceeds 2.25 MiB and evicts it, whereas capping at 1 MiB would not.
        store
            .configure(Policy {
                retain_seconds: 14 * 86_400,
                rotate_seconds: 60,
                segment_bytes: 1024 * 1024,
                max_bytes: 3 * 1024 * 1024,
                forward_runtime: false,
                ..Policy::default()
            })
            .unwrap();
        store
            .append(
                &[parent_request(500, day + 120_000, "small", 16)],
                day + 121_000,
            )
            .unwrap();
        let segments = store.segments().unwrap();
        assert_eq!(segments.len(), 1);
        assert_ne!(segments[0].path, first[0].path);
    }

    #[test]
    fn reclaim_retires_past_multiple_sparse_oldest_cohorts_to_admit_active_day() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        store
            .configure(Policy {
                retain_seconds: 30 * 86_400,
                rotate_seconds: 86_400,
                segment_bytes: 1024 * 1024,
                max_bytes: 2 * 1024 * 1024,
                forward_runtime: false,
                ..Policy::default()
            })
            .unwrap();
        let mut next_id = 1_u32;
        // Three sparse oldest cohorts (more than a 2-attempt cap).
        for offset in 0..3 {
            let day = (10 + offset) * DAY_MS;
            store
                .append(
                    &[parent_request(next_id, day + 1_000, "sparse", 16)],
                    day + 2_000,
                )
                .unwrap();
            next_id += 1;
        }
        // One dense cohort on day 13 filling the tiny rollup budget.
        let dense_day = 13 * DAY_MS;
        let mut batch = Vec::new();
        for i in 0..350 {
            batch.push(parent_request(
                next_id,
                dense_day + i * 1_000,
                &format!("dense-{i}"),
                16,
            ));
            next_id += 1;
            if batch.len() == 32 {
                store.append(&batch, dense_day + i * 1_000 + 500).unwrap();
                batch.clear();
            }
        }
        if !batch.is_empty() {
            store.append(&batch, dense_day + 400_000).unwrap();
        }
        // Day 14 intake must retire past the three sparse days and the dense day
        // without failing or marking day 14 degraded.
        let active_day = 14 * DAY_MS;
        let active = parent_request(next_id, active_day + 1_000, "active", 16);
        store.append(&[active], active_day + 2_000).unwrap();
        let report = store
            .wait_cache_daily(active_day, active_day + DAY_MS)
            .unwrap();
        assert_eq!(report["degraded_days_ms"], json!([]));
        assert_eq!(
            report["groups"]
                .as_array()
                .unwrap()
                .iter()
                .map(|g| g["totals"]["records"].as_u64().unwrap())
                .sum::<u64>(),
            1
        );
    }

    #[test]
    fn gap_archival_stays_within_segment_bytes_and_store_budget_on_near_cap_segment() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("logs"), true).unwrap();
        let policy = Policy {
            retain_seconds: 14 * 86_400,
            rotate_seconds: 86_400,
            segment_bytes: 1024 * 1024,
            max_bytes: 4 * 1024 * 1024,
            forward_runtime: false,
            ..Policy::default()
        };
        store.configure(policy.clone()).unwrap();
        let day = 10 * DAY_MS;
        let mut last_in_first = None;
        for i in 0..160_u32 {
            let start = day + i64::from(i) * 360_000;
            // ~5 KiB per record fills the first 1 MiB segment right up to its
            // detail admission limit and forces rotation into a second segment.
            let entry = parent_request(i + 1, start, "steady", 4_500);
            store
                .append(std::slice::from_ref(&entry), start + 500)
                .unwrap();
            if store.segments().unwrap().len() == 1 {
                last_in_first = Some(entry.id);
            } else {
                break;
            }
        }
        let target_id = last_in_first.expect("expected parent entries in first segment");
        assert!(store.segments().unwrap().len() >= 2);
        // Age past the 7-day clock horizon while retaining the 14-day detail.
        store.maintain(day + 9 * DAY_MS).unwrap();
        for segment in store.segments().unwrap() {
            assert!(
                segment.bytes <= policy.segment_bytes,
                "segment {} grew to {} > {}",
                segment.path.display(),
                segment.bytes,
                policy.segment_bytes
            );
        }
        let status = store.status().unwrap();
        assert!(status["retained_bytes"].as_u64().unwrap() <= policy.max_bytes);
        assert_eq!(
            status["wait_cache_rollup"]["health"]["maintenance_failures"],
            0
        );
        let query: Query = serde_json::from_value(json!({
            "from_ms": day,
            "to_ms": day + 2 * DAY_MS,
            "id": target_id,
            "limit": 1,
        }))
        .unwrap();
        let detail = store.query(&query).unwrap();
        assert_eq!(detail.items.len(), 1);
        assert_eq!(
            detail.items[0].attributes["gap_since_last_parent_req_ms"],
            360_000
        );
        assert_eq!(detail.items[0].attributes["is_first_after_idle"], true);
    }
}
