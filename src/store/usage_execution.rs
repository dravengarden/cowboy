//! Account usage placement has its own durable owner, separate from auth settings.

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use anyhow::Result;
use std::collections::BTreeMap;

impl Store {
    pub async fn usage_execution_machines(&self) -> Result<BTreeMap<String, String>> {
        match &self.backend {
            StorageBackend::Postgres(db) => db.usage_execution_machines().await,
            StorageBackend::Sqlite(db) => db.usage_execution_machines().await,
        }
    }

    pub async fn set_usage_execution_machine(
        &self,
        account: &str,
        machine: Option<&str>,
    ) -> Result<()> {
        match &self.backend {
            StorageBackend::Postgres(db) => db.set_usage_execution_machine(account, machine).await,
            StorageBackend::Sqlite(db) => db.set_usage_execution_machine(account, machine).await,
        }
    }
}

macro_rules! implementation {
    ($storage:ty) => {
        impl $storage {
            async fn usage_execution_machines(&self) -> Result<BTreeMap<String, String>> {
                let rows: Vec<(String, String)> = sqlx::query_as("SELECT account, machine_id FROM usage_execution_machines")
                    .fetch_all(&self.pool).await?;
                Ok(rows.into_iter().collect())
            }

            async fn set_usage_execution_machine(&self, account: &str, machine: Option<&str>) -> Result<()> {
                if let Some(machine) = machine {
                    sqlx::query("INSERT INTO usage_execution_machines (account, machine_id) VALUES ($1, $2) ON CONFLICT (account) DO UPDATE SET machine_id = EXCLUDED.machine_id")
                        .bind(account).bind(machine).execute(&self.pool).await?;
                } else {
                    sqlx::query("DELETE FROM usage_execution_machines WHERE account = $1")
                        .bind(account).execute(&self.pool).await?;
                }
                Ok(())
            }
        }
    };
}

implementation!(PostgresStorage);
implementation!(SqliteStorage);
