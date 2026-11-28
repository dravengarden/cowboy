//! A read-only [`Store`] for preflight checks that run beside the serving
//! Controller: no file, schema or row is created, migrated or written.

use std::str::FromStr as _;
use std::time::Duration;

use anyhow::{Context as _, Result};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};

impl Store {
    /// Open an existing database read-only. `SQLite` opens the file with
    /// `SQLITE_OPEN_READONLY` (a missing file is an error, never created);
    /// `PostgreSQL` sessions default every transaction to read-only. No
    /// migration runs, so queries see the schema the serving Controller left.
    pub async fn connect_read_only(url: &str, artifact_dir: std::path::PathBuf) -> Result<Self> {
        let artifacts = crate::artifacts::ArtifactStore::unopened(artifact_dir);
        let backend = if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            let options = PgConnectOptions::from_str(url)
                .context("parsing PostgreSQL URL")?
                .options([("default_transaction_read_only", "on")]);
            let pool = PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_secs(5))
                .connect_with(options)
                .await
                .context("connecting to PostgreSQL read-only")?;
            StorageBackend::Postgres(PostgresStorage { pool, artifacts })
        } else if url.starts_with("sqlite:") {
            let options = SqliteConnectOptions::from_str(url)
                .with_context(|| format!("parsing SQLite URL {url}"))?
                .read_only(true)
                .create_if_missing(false)
                .busy_timeout(Duration::from_secs(5));
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_secs(5))
                .connect_with(options)
                .await
                .with_context(|| format!("connecting to SQLite {url} read-only"))?;
            StorageBackend::Sqlite(SqliteStorage {
                pool,
                artifacts,
                database_path: None,
            })
        } else {
            anyhow::bail!("unsupported database URL scheme")
        };
        Ok(Self {
            backend,
            passkey_storage: std::sync::Arc::default(),
            observed_storage_metrics: std::sync::Arc::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_only_sqlite_store_never_creates_or_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cowboy.sqlite3");
        let url = format!("sqlite://{}", path.display());
        assert!(
            Store::connect_read_only(&url, dir.path().join("artifacts"))
                .await
                .is_err(),
            "a missing database must not be created"
        );
        assert!(!path.exists());
        let writer = Store::connect(&url, dir.path().join("artifacts"))
            .await
            .unwrap();
        writer.migrate().await.unwrap();
        let reader = Store::connect_read_only(&url, dir.path().join("unused-artifacts"))
            .await
            .unwrap();
        assert!(
            reader
                .retired_default_sessions_refusing_startup()
                .await
                .unwrap()
                .is_empty()
        );
        let StorageBackend::Sqlite(db) = &reader.backend else {
            panic!("sqlite")
        };
        assert!(
            sqlx::query("DELETE FROM sessions")
                .execute(&db.pool)
                .await
                .is_err(),
            "the read-only store must refuse writes"
        );
        assert!(!dir.path().join("unused-artifacts").exists());
    }
}
