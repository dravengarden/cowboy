//! Compare the terminal Call ledger and the receiver's durable prompt echo.
//! The echo precedes native acceptance. A missing echo can reflect a queue
//! hold, transcript clear, or write-behind gap.
use super::{Record, Result, StorageBackend, Store};
use std::time::{Duration, Instant};

const SCAN_LIMIT: i64 = 10_000;
const EXECUTION_BUDGET: Duration = Duration::from_secs(2);

pub struct NoticeReconciliation {
    pub terminal_calls: u64,
    pub durable_echoes: u64,
    pub truncated: bool,
}

pub(super) fn notice_finished_at(record: &Record) -> Option<i64> {
    (record.state.terminal()
        && record
            .labels
            .get(crate::managed_calls::review::NOTIFY)
            .is_some_and(|value| value == "parent"))
    .then(|| record.finished_at_ms.unwrap_or(record.updated_at_ms))
}

macro_rules! query {
    ($echo_table:literal, $cmid:literal, $kind:literal, $update:literal) => {
        concat!(
            "SELECT COUNT(*), COUNT(*) FILTER (WHERE EXISTS (",
            "SELECT 1 FROM ",
            $echo_table,
            " WHERE e.session_id = c.parent_session_id ",
            "AND ",
            $cmid,
            " LIKE 'call-notice:%' AND ",
            $cmid,
            " = 'call-notice:' || c.call_id ",
            "AND ",
            $kind,
            " = 'update' AND ",
            $update,
            " = 'user_message_chunk'",
            ")) FROM (SELECT parent_session_id, call_id FROM managed_agent_calls ",
            "WHERE notice_parent AND notice_finished_at_ms IS NOT NULL ",
            "AND notice_finished_at_ms >= $1 AND notice_finished_at_ms < $2 ",
            "ORDER BY notice_finished_at_ms, call_id LIMIT $3) c"
        )
    };
}

const POSTGRES_QUERY: &str = query!(
    "events AS e",
    "(e.payload ->> 'cmid')",
    "(e.payload ->> 'kind')",
    "(e.payload #>> '{update,sessionUpdate}')"
);

pub(super) const SQLITE_QUERY: &str = query!(
    "events AS e INDEXED BY events_call_notice_echo_idx",
    "json_extract(e.payload, '$.cmid')",
    "json_extract(e.payload, '$.kind')",
    "json_extract(e.payload, '$.update.sessionUpdate')"
);

async fn sqlite_counts(
    pool: sqlx::SqlitePool,
    from_ms: i64,
    to_ms: i64,
    budget: Duration,
) -> Result<(i64, i64)> {
    let mut conn = tokio::time::timeout(Duration::from_millis(250), pool.acquire()).await??;
    let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
        .fetch_one(&mut *conn)
        .await?;
    sqlx::query("PRAGMA busy_timeout = 250")
        .execute(&mut *conn)
        .await?;
    let deadline = Instant::now() + budget;
    let Ok(mut handle) = conn.lock_handle().await else {
        conn.close_on_drop();
        anyhow::bail!("could not bound SQLite notice query");
    };
    handle.set_progress_handler(256, move || Instant::now() < deadline);
    drop(handle);
    let rows = sqlx::query_as(SQLITE_QUERY)
        .bind(from_ms)
        .bind(to_ms)
        .bind(SCAN_LIMIT + 1)
        .fetch_one(&mut *conn)
        .await;
    let Ok(mut handle) = conn.lock_handle().await else {
        conn.close_on_drop();
        anyhow::bail!("could not release SQLite notice deadline");
    };
    handle.remove_progress_handler();
    drop(handle);
    if sqlx::query(&format!("PRAGMA busy_timeout = {busy_timeout}"))
        .execute(&mut *conn)
        .await
        .is_err()
    {
        conn.close_on_drop();
    }
    Ok(rows?)
}

impl Store {
    pub async fn reconcile_call_notice_echoes(
        &self,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<NoticeReconciliation> {
        let (calls, echoes): (i64, i64) = match &self.backend {
            StorageBackend::Postgres(db) => {
                let mut tx = db.pool.begin().await?;
                sqlx::query("SET LOCAL statement_timeout = '2s'")
                    .execute(&mut *tx)
                    .await?;
                let rows = sqlx::query_as(POSTGRES_QUERY)
                    .bind(from_ms)
                    .bind(to_ms)
                    .bind(SCAN_LIMIT + 1)
                    .fetch_one(&mut *tx)
                    .await?;
                tx.commit().await?;
                rows
            }
            // The SQLx SQLite thread can continue after its caller is dropped.
            // Keep the deadline armed and restore the pooled connection even
            // if the outer diagnostic timeout drops this JoinHandle.
            StorageBackend::Sqlite(db) => {
                tokio::spawn(sqlite_counts(
                    db.pool.clone(),
                    from_ms,
                    to_ms,
                    EXECUTION_BUDGET,
                ))
                .await??
            }
        };
        Ok(NoticeReconciliation {
            terminal_calls: u64::try_from(calls)?,
            durable_echoes: u64::try_from(echoes)?,
            truncated: calls > SCAN_LIMIT,
        })
    }
}

#[cfg(test)]
pub(super) async fn assert_sqlite_plan_and_deadline(db: &super::SqliteStorage) {
    let plan: Vec<(i64, i64, i64, String)> =
        sqlx::query_as(&format!("EXPLAIN QUERY PLAN {SQLITE_QUERY}"))
            .bind(200)
            .bind(300)
            .bind(SCAN_LIMIT + 1)
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert!(
        plan.iter().any(|(_, _, _, detail)| {
            detail.contains("events_call_notice_echo_idx (session_id=? AND <expr>=?)")
        }),
        "{plan:?}"
    );
    sqlx::query(
        "WITH RECURSIVE i(n) AS (VALUES (1) UNION ALL SELECT n+1 FROM i WHERE n<500)
         INSERT INTO managed_agent_calls (call_id, parent_session_id, request_id, request_document,
             child_session_id, state, revision, document, document_sha256, notice_finished_at_ms, notice_parent)
         SELECT 'deadline-'||n, 'notice-parent', 'deadline-'||n, '{}', 'deadline-'||n,
             'failed', 1, '{}', '', 250, TRUE FROM i",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let error = sqlite_counts(db.pool.clone(), 200, 300, Duration::ZERO)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("interrupted"), "{error}");
    let counts = sqlite_counts(db.pool.clone(), 200, 300, EXECUTION_BUDGET)
        .await
        .unwrap();
    assert_eq!(counts, (502, 1));
}
