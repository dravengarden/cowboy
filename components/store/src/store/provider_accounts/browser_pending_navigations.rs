//! Durable fence for Machine browser navigations whose terminal outcome is not
//! yet confirmed (across HTTP wait timeouts, Machine disconnects, and
//! Controller restarts).
use super::super::{StorageBackend, Store};
use crate::provider_account::ProviderAccountId;
use anyhow::Result;

/// Upper bound on how long an orphaned navigation row can fence a share when
/// its Machine never reconnects (10x the Machine's 30 s navigate timeout).
pub const PENDING_NAVIGATION_TTL_MS: i64 = 5 * 60 * 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingNavigationRecord {
    pub request_id: String,
    pub owner_user_id: String,
    pub browser_key: ProviderAccountId,
    pub machine_id: String,
    pub instance_id: String,
    pub incarnation: u64,
    pub observed_at_ms: i64,
    pub started_at_ms: i64,
}

type RawRow = (String, String, String, String, String, i64, i64, i64);

fn decode_row(row: RawRow) -> Option<PendingNavigationRecord> {
    let browser_key = ProviderAccountId::try_from(row.2).ok()?;
    let incarnation = u64::try_from(row.5).ok()?;
    Some(PendingNavigationRecord {
        request_id: row.0,
        owner_user_id: row.1,
        browser_key,
        machine_id: row.3,
        instance_id: row.4,
        incarnation,
        observed_at_ms: row.6,
        started_at_ms: row.7,
    })
}

impl Store {
    /// Persist a pending navigation before dispatching it to the Machine so the
    /// share fence survives disconnects and Controller restarts.
    pub async fn record_browser_pending_navigation(
        &self,
        record: &PendingNavigationRecord,
    ) -> Result<()> {
        let incarnation = i64::try_from(record.incarnation)
            .map_err(|_| anyhow::anyhow!("incarnation_overflow"))?;
        macro_rules! insert { ($db:expr) => {{
            sqlx::query(
                "INSERT INTO browser_pending_navigations                  (request_id, owner_user_id, browser_key, machine_id, instance_id, incarnation, observed_at_ms, started_at_ms)                  VALUES ($1, $2, $3, $4, $5, $6, $7, $8)                  ON CONFLICT (request_id) DO NOTHING",
            )
            .bind(&record.request_id)
            .bind(&record.owner_user_id)
            .bind(record.browser_key.as_str())
            .bind(&record.machine_id)
            .bind(&record.instance_id)
            .bind(incarnation)
            .bind(record.observed_at_ms)
            .bind(record.started_at_ms)
            .execute(&$db.pool)
            .await?;
        }}; }
        match &self.backend {
            StorageBackend::Postgres(db) => insert!(db),
            StorageBackend::Sqlite(db) => insert!(db),
        }
        Ok(())
    }

    pub async fn delete_browser_pending_navigation(&self, request_id: &str) -> Result<()> {
        macro_rules! delete {
            ($db:expr) => {{
                sqlx::query("DELETE FROM browser_pending_navigations WHERE request_id=$1")
                    .bind(request_id)
                    .execute(&$db.pool)
                    .await?;
            }};
        }
        match &self.backend {
            StorageBackend::Postgres(db) => delete!(db),
            StorageBackend::Sqlite(db) => delete!(db),
        }
        Ok(())
    }

    pub async fn delete_browser_pending_navigations(&self, request_ids: &[String]) -> Result<()> {
        if request_ids.is_empty() {
            return Ok(());
        }
        macro_rules! delete_batch {
            ($db:expr) => {{
                let mut tx = $db.pool.begin().await?;
                for request_id in request_ids {
                    sqlx::query("DELETE FROM browser_pending_navigations WHERE request_id=$1")
                        .bind(request_id)
                        .execute(&mut *tx)
                        .await?;
                }
                tx.commit().await?;
            }};
        }
        match &self.backend {
            StorageBackend::Postgres(db) => delete_batch!(db),
            StorageBackend::Sqlite(db) => delete_batch!(db),
        }
        Ok(())
    }

    pub async fn has_pending_browser_navigation_request(&self, request_id: &str) -> Result<bool> {
        macro_rules! check {
            ($db:expr) => {{
                let count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM browser_pending_navigations WHERE request_id=$1",
                )
                .bind(request_id)
                .fetch_one(&$db.pool)
                .await?;
                count > 0
            }};
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => check!(db),
            StorageBackend::Sqlite(db) => check!(db),
        })
    }

    pub async fn is_machine_revoked(&self, machine_id: &str) -> Result<bool> {
        macro_rules! check {
            ($db:expr, $revoked:literal) => {{
                let count: i64 = sqlx::query_scalar(concat!(
                    "SELECT COUNT(*) FROM machines WHERE id=$1 AND ",
                    $revoked,
                    " IS NOT NULL"
                ))
                .bind(machine_id)
                .fetch_one(&$db.pool)
                .await?;
                count > 0
            }};
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => check!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => check!(db, "revoked_at_ms"),
        })
    }

    pub async fn has_pending_browser_navigation(
        &self,
        owner: &str,
        browser_key: &ProviderAccountId,
        now_ms: i64,
    ) -> Result<bool> {
        let cutoff = now_ms.saturating_sub(PENDING_NAVIGATION_TTL_MS);
        macro_rules! check {
            ($db:expr, $revoked:literal) => {{
                let count: i64 = sqlx::query_scalar(concat!(
                    "SELECT COUNT(*) FROM browser_pending_navigations p ",
                    "WHERE p.owner_user_id=$1 AND p.browser_key=$2 AND p.started_at_ms > $3 ",
                    "AND NOT EXISTS (SELECT 1 FROM machines m WHERE m.id = p.machine_id AND m.",
                    $revoked,
                    " IS NOT NULL)"
                ))
                .bind(owner)
                .bind(browser_key.as_str())
                .bind(cutoff)
                .fetch_one(&$db.pool)
                .await?;
                count > 0
            }};
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => check!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => check!(db, "revoked_at_ms"),
        })
    }

    pub async fn pending_browser_navigations(
        &self,
        machine_id: Option<&str>,
    ) -> Result<Vec<PendingNavigationRecord>> {
        macro_rules! list { ($db:expr) => {{
            if let Some(machine_id) = machine_id {
                sqlx::query_as::<_, RawRow>(
                    "SELECT request_id, owner_user_id, browser_key, machine_id, instance_id,                             incarnation, observed_at_ms, started_at_ms                      FROM browser_pending_navigations                      WHERE machine_id=$1                      ORDER BY started_at_ms, request_id",
                )
                .bind(machine_id)
                .fetch_all(&$db.pool)
                .await?
            } else {
                sqlx::query_as::<_, RawRow>(
                    "SELECT request_id, owner_user_id, browser_key, machine_id, instance_id,                             incarnation, observed_at_ms, started_at_ms                      FROM browser_pending_navigations                      ORDER BY started_at_ms, request_id",
                )
                .fetch_all(&$db.pool)
                .await?
            }
        }}; }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => list!(db),
            StorageBackend::Sqlite(db) => list!(db),
        };
        Ok(rows.into_iter().filter_map(decode_row).collect())
    }
}
