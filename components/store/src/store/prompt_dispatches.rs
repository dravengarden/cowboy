//! Durable Controller-side prompt and steer dispatch origins (C-auth).
//!
//! Every prompt or steer handed off by the Controller records its computed
//! dispatch origin and initiating trigger keyed by `(session_id, cmid)` before
//! reaching a worker. Rows are immutable once written (`ON CONFLICT DO NOTHING`)
//! so retries or replays can never alter recorded provenance.

use std::collections::HashMap;

use anyhow::{Context as _, Result};

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use crate::prompt_origin::{DeliveryOrigin, PromptDispatchRecord};

/// Default per-session hot dispatch rows hydrated into Hub on startup.
pub const STARTUP_HYDRATE_PER_SESSION: i64 = 256;
/// Grace window before sweeping `prompt_dispatches` rows whose session has no
/// matching row in `sessions`, protecting brand-new sessions whose
/// `StoreWrite::InsertSession` is still in the write-behind queue.
pub const ORPHAN_GRACE_MS: i64 = 3_600_000;

macro_rules! dispatch {
    ($store:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match &$store.backend {
            StorageBackend::Postgres(db) => db.$method($($arg),*).await,
            StorageBackend::Sqlite(db) => db.$method($($arg),*).await,
        }
    };
}

fn decode_record(
    session_id: String,
    cmid: String,
    origin_json: &str,
    trigger_json: &str,
    dispatched_at_ms: i64,
) -> Result<PromptDispatchRecord> {
    let origin: DeliveryOrigin =
        serde_json::from_str(origin_json).context("decode prompt_dispatches.origin")?;
    let trigger: DeliveryOrigin =
        serde_json::from_str(trigger_json).context("decode prompt_dispatches.trigger_origin")?;
    Ok(PromptDispatchRecord {
        session_id,
        cmid,
        origin,
        trigger,
        dispatched_at_ms,
    })
}

impl Store {
    /// Record an immutable dispatch origin row before handing a prompt or steer
    /// to a worker. First write for `(session_id, cmid)` wins.
    pub async fn record_prompt_dispatch(&self, record: &PromptDispatchRecord) -> Result<()> {
        let _ = self.record_prompt_dispatch_inserted(record).await?;
        Ok(())
    }

    /// Record an immutable dispatch origin row and verify that any pre-existing
    /// row for `(session_id, cmid)` carries identical `(origin, trigger)`.
    /// Returns `Ok(true)` if inserted or identically matched, `Ok(false)` if an
    /// earlier dispatch already claimed this `(session_id, cmid)` under a
    /// different origin or trigger.
    pub async fn record_prompt_dispatch_if_compatible(
        &self,
        record: &PromptDispatchRecord,
    ) -> Result<bool> {
        if self.record_prompt_dispatch_inserted(record).await? {
            return Ok(true);
        }
        let existing = self
            .prompt_dispatch(&record.session_id, &record.cmid)
            .await?;
        Ok(existing.is_some_and(|row| row.origin == record.origin))
    }

    async fn record_prompt_dispatch_inserted(&self, record: &PromptDispatchRecord) -> Result<bool> {
        let origin =
            serde_json::to_string(&record.origin).context("encode prompt_dispatches.origin")?;
        let trigger = serde_json::to_string(&record.trigger)
            .context("encode prompt_dispatches.trigger_origin")?;
        let rows = dispatch!(
            self,
            record_prompt_dispatch_raw(
                &record.session_id,
                &record.cmid,
                &origin,
                &trigger,
                record.dispatched_at_ms,
            )
        )?;
        Ok(rows > 0)
    }

    /// Look up the recorded dispatch provenance for one `(session_id, cmid)`.
    pub async fn prompt_dispatch(
        &self,
        session_id: &str,
        cmid: &str,
    ) -> Result<Option<PromptDispatchRecord>> {
        let row = dispatch!(self, prompt_dispatch_raw(session_id, cmid))?;
        row.map(|(origin, trigger, dispatched_at_ms)| {
            decode_record(
                session_id.to_owned(),
                cmid.to_owned(),
                &origin,
                &trigger,
                dispatched_at_ms,
            )
        })
        .transpose()
    }

    /// Load up to `per_session_limit` newest dispatch records per active
    /// (non-deleted) session so the Hub starts warm after a Controller restart.
    pub async fn recent_prompt_dispatches(
        &self,
        per_session_limit: i64,
    ) -> Result<Vec<PromptDispatchRecord>> {
        let rows = dispatch!(self, recent_prompt_dispatches_raw(per_session_limit))?;
        let mut out = Vec::with_capacity(rows.len());
        for (session_id, cmid, origin, trigger, dispatched_at_ms) in rows {
            match decode_record(session_id, cmid, &origin, &trigger, dispatched_at_ms) {
                Ok(record) => out.push(record),
                Err(error) => {
                    tracing::warn!(%error, "skipping unreadable prompt_dispatches row on hydrate");
                }
            }
        }
        Ok(out)
    }

    /// Batch-fetch dispatch origins for a slice of `cmid`s in one session.
    /// Used when scanning historical transcript pages for turn budget or
    /// superagent causal evidence.
    pub async fn prompt_dispatches_by_cmids(
        &self,
        session_id: &str,
        cmids: &[String],
    ) -> Result<HashMap<String, DeliveryOrigin>> {
        if cmids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = dispatch!(self, prompt_dispatches_by_cmids_raw(session_id, cmids))?;
        let mut map = HashMap::with_capacity(rows.len());
        for (cmid, origin_json) in rows {
            let origin: DeliveryOrigin =
                serde_json::from_str(&origin_json).context("decode prompt_dispatches.origin")?;
            map.insert(cmid, origin);
        }
        Ok(map)
    }

    /// Delete `prompt_dispatches` belonging to soft-deleted sessions about to
    /// be hard-purged. Scoped strictly to expired rows in `sessions` so a
    /// brand-new session whose `StoreWrite::InsertSession` has not flushed yet
    /// is never touched.
    pub(super) async fn purge_prompt_dispatches_for_expired_sessions(
        &self,
        retention_days: i64,
    ) -> Result<()> {
        dispatch!(
            self,
            purge_prompt_dispatches_for_expired_sessions_raw(retention_days)
        )
    }

    /// Sweep orphaned `prompt_dispatches` older than `grace_ms` whose session
    /// no longer exists (covering sessions purged between statements or failed
    /// `InsertSession` writes without touching freshly created unflushed rows).
    pub(super) async fn purge_orphaned_prompt_dispatches(&self, grace_ms: i64) -> Result<()> {
        let cutoff_ms = chrono::Utc::now()
            .timestamp_millis()
            .saturating_sub(grace_ms.max(0));
        dispatch!(self, purge_orphaned_prompt_dispatches_raw(cutoff_ms))
    }
}

macro_rules! shared_impl {
    ($backend:ty, $active_session:literal) => {
        impl $backend {
            async fn record_prompt_dispatch_raw(
                &self,
                session_id: &str,
                cmid: &str,
                origin: &str,
                trigger: &str,
                dispatched_at_ms: i64,
            ) -> Result<u64> {
                let res = sqlx::query(
                    "INSERT INTO prompt_dispatches \
                     (session_id, cmid, origin, trigger_origin, dispatched_at_ms) \
                     VALUES ($1, $2, $3, $4, $5) \
                     ON CONFLICT (session_id, cmid) DO NOTHING",
                )
                .bind(session_id)
                .bind(cmid)
                .bind(origin)
                .bind(trigger)
                .bind(dispatched_at_ms)
                .execute(&self.pool)
                .await
                .with_context(|| format!("INSERT prompt_dispatches ({session_id}, {cmid})"))?;
                Ok(res.rows_affected())
            }

            async fn prompt_dispatch_raw(
                &self,
                session_id: &str,
                cmid: &str,
            ) -> Result<Option<(String, String, i64)>> {
                sqlx::query_as(
                    "SELECT origin, trigger_origin, dispatched_at_ms \
                     FROM prompt_dispatches \
                     WHERE session_id = $1 AND cmid = $2",
                )
                .bind(session_id)
                .bind(cmid)
                .fetch_optional(&self.pool)
                .await
                .with_context(|| format!("SELECT prompt_dispatches ({session_id}, {cmid})"))
            }

            async fn purge_orphaned_prompt_dispatches_raw(&self, cutoff_ms: i64) -> Result<()> {
                sqlx::query(
                    "DELETE FROM prompt_dispatches \
                     WHERE dispatched_at_ms <= $1 \
                     AND NOT EXISTS (SELECT 1 FROM sessions s WHERE s.id = prompt_dispatches.session_id)",
                )
                .bind(cutoff_ms)
                .execute(&self.pool)
                .await
                .context("purge orphaned prompt_dispatches")?;
                Ok(())
            }

            async fn recent_prompt_dispatches_raw(
                &self,
                per_session_limit: i64,
            ) -> Result<Vec<(String, String, String, String, i64)>> {
                sqlx::query_as(concat!(
                    "WITH ranked AS ( ",
                    "SELECT pd.session_id, pd.cmid, pd.origin, pd.trigger_origin, ",
                    "pd.dispatched_at_ms, ",
                    "ROW_NUMBER() OVER ( ",
                    "PARTITION BY pd.session_id ",
                    "ORDER BY pd.dispatched_at_ms DESC, pd.cmid DESC ",
                    ") AS rn ",
                    "FROM prompt_dispatches pd ",
                    "JOIN sessions s ON s.id = pd.session_id ",
                    "WHERE ",
                    $active_session,
                    " ) ",
                    "SELECT session_id, cmid, origin, trigger_origin, dispatched_at_ms ",
                    "FROM ranked WHERE rn <= $1 ",
                    "ORDER BY session_id, dispatched_at_ms ASC, cmid ASC"
                ))
                .bind(per_session_limit)
                .fetch_all(&self.pool)
                .await
                .context("SELECT recent prompt_dispatches")
            }
        }
    };
}

shared_impl!(PostgresStorage, "s.deleted_at IS NULL");
shared_impl!(SqliteStorage, "s.deleted_at_ms IS NULL");

impl PostgresStorage {
    async fn purge_prompt_dispatches_for_expired_sessions_raw(
        &self,
        retention_days: i64,
    ) -> Result<()> {
        sqlx::query(
            "DELETE FROM prompt_dispatches WHERE session_id IN ( \
             SELECT id FROM sessions WHERE deleted_at IS NOT NULL \
             AND COALESCE(purge_after_at, deleted_at + make_interval(days => $1::int)) < now() \
             AND NOT EXISTS (SELECT 1 FROM managed_agent_calls AS call \
                 WHERE (call.parent_session_id = sessions.id OR call.child_session_id = sessions.id) \
                 AND call.state NOT IN ('completed', 'failed', 'cancelled')) \
             AND NOT EXISTS (SELECT 1 FROM plugin_uninstall_operations AS operation \
                 WHERE operation.machine_id = sessions.machine_id AND operation.plugin_id = sessions.provider \
                 AND operation.phase NOT IN ('completed', 'compensated', 'aborted')))",
        )
        .bind(i32::try_from(retention_days).unwrap_or(3))
        .execute(&self.pool)
        .await
        .context("purge prompt_dispatches for expired sessions")?;
        Ok(())
    }

    async fn prompt_dispatches_by_cmids_raw(
        &self,
        session_id: &str,
        cmids: &[String],
    ) -> Result<Vec<(String, String)>> {
        sqlx::query_as(
            "SELECT cmid, origin FROM prompt_dispatches \
             WHERE session_id = $1 AND cmid = ANY($2)",
        )
        .bind(session_id)
        .bind(cmids)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("SELECT prompt_dispatches batch ({session_id})"))
    }
}

impl SqliteStorage {
    async fn purge_prompt_dispatches_for_expired_sessions_raw(
        &self,
        retention_days: i64,
    ) -> Result<()> {
        let retention_ms = retention_days.saturating_mul(86_400_000);
        sqlx::query(
            "DELETE FROM prompt_dispatches WHERE session_id IN ( \
             SELECT id FROM sessions WHERE deleted_at_ms IS NOT NULL \
             AND COALESCE(purge_after_at_ms, deleted_at_ms + ?1) <= ?2 \
             AND NOT EXISTS (SELECT 1 FROM managed_agent_calls AS call \
                 WHERE (call.parent_session_id = sessions.id OR call.child_session_id = sessions.id) \
                 AND call.state NOT IN ('completed', 'failed', 'cancelled')) \
             AND NOT EXISTS (SELECT 1 FROM plugin_uninstall_operations AS operation \
                 WHERE operation.machine_id = sessions.machine_id AND operation.plugin_id = sessions.provider \
                 AND operation.phase NOT IN ('completed', 'compensated', 'aborted')))",
        )
        .bind(retention_ms)
        .bind(chrono::Utc::now().timestamp_millis())
        .execute(&self.pool)
        .await
        .context("purge SQLite prompt_dispatches for expired sessions")?;
        Ok(())
    }

    async fn prompt_dispatches_by_cmids_raw(
        &self,
        session_id: &str,
        cmids: &[String],
    ) -> Result<Vec<(String, String)>> {
        let mut out = Vec::new();
        for chunk in cmids.chunks(200) {
            let placeholders = (0..chunk.len())
                .map(|idx| format!("?{}", idx + 2))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT cmid, origin FROM prompt_dispatches \
                 WHERE session_id = ?1 AND cmid IN ({placeholders})"
            );
            let mut query = sqlx::query_as::<_, (String, String)>(&sql).bind(session_id);
            for cmid in chunk {
                query = query.bind(cmid);
            }
            let mut rows = query
                .fetch_all(&self.pool)
                .await
                .with_context(|| format!("SELECT SQLite prompt_dispatches batch ({session_id})"))?;
            out.append(&mut rows);
        }
        Ok(out)
    }
}
