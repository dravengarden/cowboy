//! Durable fence for identity-bound Machine browser navigations whose
//! terminal outcome is not yet confirmed (across HTTP wait timeouts, Machine
//! disconnects, and Controller restarts).
use super::super::{StorageBackend, Store};
use crate::provider_account::ProviderAccountId;
use anyhow::Result;

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
    /// Persist a pending identity-bound navigation before dispatching it to
    /// the Machine so the share fence survives disconnects and Controller
    /// restarts.
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

    pub async fn has_pending_browser_navigation(
        &self,
        owner: &str,
        browser_key: &ProviderAccountId,
    ) -> Result<bool> {
        macro_rules! check { ($db:expr) => {{
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM browser_pending_navigations                  WHERE owner_user_id=$1 AND browser_key=$2",
            )
            .bind(owner)
            .bind(browser_key.as_str())
            .fetch_one(&$db.pool)
            .await?;
            count > 0
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => check!(db),
            StorageBackend::Sqlite(db) => check!(db),
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
