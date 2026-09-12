//! One bounded atomic binding ledger, identical on `PostgreSQL` and `SQLite`.

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use crate::telemetry_binding::{
    Ledger, MAX_BYTES,
    writer::{Change, Updated},
};
use anyhow::{Result, ensure};
use sha2::Digest as _;

#[derive(sqlx::FromRow)]
struct Record {
    document: String,
    document_sha256: String,
}

impl Record {
    fn decode(self, service: &str) -> Result<Ledger> {
        ensure!(
            self.document.len() <= MAX_BYTES && self.document_sha256 == checksum(&self.document),
            "invalid Service binding checksum or capacity"
        );
        Ledger::decode(&self.document, service)
    }
}

fn checksum(document: &str) -> String {
    format!("{:x}", sha2::Sha256::digest(document.as_bytes()))
}

impl Store {
    pub(crate) async fn telemetry_binding_ledger(&self, service: &str) -> Result<Option<Ledger>> {
        match &self.backend {
            StorageBackend::Postgres(db) => db.telemetry_binding_ledger(service).await,
            StorageBackend::Sqlite(db) => db.telemetry_binding_ledger(service).await,
        }
    }

    // Only the staged finite coordinator uses this. No production writer
    // endpoint exists; reader activation never inserts a row or adopts policy.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn change_telemetry_binding(
        &self,
        change: &Change<'_>,
        within_budget: &(dyn Fn() -> bool + Sync),
    ) -> Result<Updated> {
        match &self.backend {
            StorageBackend::Postgres(db) => {
                db.change_telemetry_binding(change, within_budget).await
            }
            StorageBackend::Sqlite(db) => db.change_telemetry_binding(change, within_budget).await,
        }
    }
}

macro_rules! journal {
    ($backend:ty, $durability:literal, $lock:literal) => {
        impl $backend {
            async fn telemetry_binding_ledger(&self, service: &str) -> Result<Option<Ledger>> {
                sqlx::query_as::<_, Record>("SELECT document, document_sha256 FROM telemetry_binding_journal WHERE slot = 'telemetry'")
                    .fetch_optional(&self.pool).await?.map(|record| record.decode(service)).transpose()
            }

            #[cfg_attr(not(test), allow(dead_code))]
            async fn change_telemetry_binding(&self, change: &Change<'_>, within_budget: &(dyn Fn() -> bool + Sync)) -> Result<Updated> {
                let mut tx = self.pool.begin().await?;
                sqlx::query($durability).execute(&mut *tx).await?;
                // In particular, reserve SQLite's writer before SELECT. A WAL
                // snapshot upgrade is not repaired by busy_timeout.
                sqlx::query($lock).execute(&mut *tx).await?;
                ensure!(within_budget() && change.within_budget(), "binding admission ended while waiting for storage");
                let mut ledger = sqlx::query_as::<_, Record>("SELECT document, document_sha256 FROM telemetry_binding_journal WHERE slot = 'telemetry'")
                    .fetch_optional(&mut *tx).await?.map(|record| record.decode(change.service())).transpose()?;
                let updated = crate::telemetry_binding::writer::apply(&mut ledger, change)?;
                let document = ledger.ok_or_else(|| anyhow::anyhow!("missing updated binding journal"))?.encode(change.service())?;
                sqlx::query("INSERT INTO telemetry_binding_journal (slot, document, document_sha256) VALUES ('telemetry', $1, $2) ON CONFLICT (slot) DO UPDATE SET document = excluded.document, document_sha256 = excluded.document_sha256")
                    .bind(&document).bind(checksum(&document)).execute(&mut *tx).await?;
                ensure!(within_budget() && change.within_budget(), "binding admission ended before commit");
                tx.commit().await?;
                Ok(updated)
            }
        }
    };
}

journal!(
    PostgresStorage,
    "SET LOCAL synchronous_commit = on",
    "LOCK TABLE telemetry_binding_journal IN SHARE ROW EXCLUSIVE MODE"
);
journal!(
    SqliteStorage,
    "SELECT 1",
    "UPDATE telemetry_binding_journal SET slot = slot WHERE 0"
);

#[cfg(test)]
mod tests;
