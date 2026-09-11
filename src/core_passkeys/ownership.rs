//! Core-only creation/adoption. Historical Plugin ledger bytes never advance.

use super::{CORE_IMPORT, LEGACY_POSTGRES_CHECKSUM, LEGACY_SQLITE_CHECKSUM, PasskeyStorage};
use crate::core_security::{Authority, Phase, Source};
use crate::plugin_storage::{NamespaceBackend, set_postgres_search_path};
use anyhow::{Result, ensure};

const OWNER_TABLE: &str = "_cowboy_core_security_owner";
const OWNER_DDL: &str = "CREATE TABLE IF NOT EXISTS _cowboy_core_security_owner (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), authority TEXT NOT NULL)";
const OWNER_SELECT: &str = "SELECT authority FROM _cowboy_core_security_owner WHERE singleton = 1";

fn baseline(postgres: bool) -> &'static str {
    // The final source-file newline is not part of the historical SQL string.
    if postgres {
        include_str!("baseline-postgres.sql").trim_end_matches('\n')
    } else {
        include_str!("baseline-sqlite.sql").trim_end_matches('\n')
    }
}

macro_rules! finish_namespace {
    ($tx:ident, $authority:ident, $phase:ident, $tables:ident, $postgres:expr) => {{
        let fresh = $authority.passkeys.source == Source::Fresh && $phase == Phase::Prepared;
        if fresh && $tables.is_empty() {
            let (ledger, insert, checksum) = if $postgres {
                ("CREATE TABLE _cowboy_plugin_migrations (version TEXT PRIMARY KEY, checksum TEXT NOT NULL, applied_at TIMESTAMPTZ NOT NULL DEFAULT now())",
                 "INSERT INTO _cowboy_plugin_migrations (version, checksum) VALUES ('0001', $1)", LEGACY_POSTGRES_CHECKSUM)
            } else {
                ("CREATE TABLE _cowboy_plugin_migrations (version TEXT PRIMARY KEY, checksum TEXT NOT NULL, applied_at_ms INTEGER NOT NULL)",
                 "INSERT INTO _cowboy_plugin_migrations (version, checksum, applied_at_ms) VALUES ('0001', $1, CAST(strftime('%s', 'now') AS INTEGER) * 1000)", LEGACY_SQLITE_CHECKSUM)
            };
            sqlx::query(ledger).execute(&mut *$tx).await?;
            sqlx::raw_sql(baseline($postgres)).execute(&mut *$tx).await?;
            sqlx::query(insert).bind(checksum).execute(&mut *$tx).await?;
            sqlx::query("INSERT INTO _cowboy_import (name, applied_at_ms) VALUES ($1, $2)")
                .bind(CORE_IMPORT).bind(chrono::Utc::now().timestamp_millis()).execute(&mut *$tx).await?;
        } else {
            // Never use a missing receipt to import stale core data. A prepared
            // fresh retry is either wholly empty or already has our exact token.
            let rows: Vec<(String, String)> = sqlx::query_as("SELECT version, checksum FROM _cowboy_plugin_migrations")
                .fetch_all(&mut *$tx).await?;
            let checksum = if $postgres { LEGACY_POSTGRES_CHECKSUM } else { LEGACY_SQLITE_CHECKSUM };
            ensure!(rows == vec![("0001".to_owned(), checksum.to_owned())], "incompatible core Passkey migration ledger");
            let imported: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _cowboy_import WHERE name = $1")
                .bind(CORE_IMPORT).fetch_one(&mut *$tx).await?;
            ensure!(imported == 1, "CoreSecurity requires the existing complete import receipt");
            ensure!(!fresh || $tables.iter().any(|name| name == OWNER_TABLE),
                "prepared fresh namespace contains unowned state; refusing reconciliation");
        }
        let owns = $tables.iter().any(|name| name == OWNER_TABLE);
        let encoded = serde_json::to_string($authority)?;
        if $phase == Phase::Prepared && !owns {
            sqlx::query(OWNER_DDL).execute(&mut *$tx).await?;
            sqlx::query("INSERT INTO _cowboy_core_security_owner (singleton, authority) VALUES (1, $1)")
                .bind(&encoded).execute(&mut *$tx).await?;
        }
        let found: Option<String> = sqlx::query_scalar(OWNER_SELECT).fetch_optional(&mut *$tx).await?;
        ensure!(found.as_deref() == Some(encoded.as_str()),
            "CoreSecurity namespace ownership token is missing or conflicts with core authority");
    }};
}

impl PasskeyStorage {
    pub(crate) async fn validate_adoption(backend: &NamespaceBackend) -> Result<()> {
        Self::validate_ledger(backend).await?;
        let sql = "SELECT COUNT(*) FROM _cowboy_import WHERE name = $1";
        let (count, owned): (i64, bool) = match backend {
            NamespaceBackend::Postgres { pool, schema } => {
                let mut tx = pool.begin().await?;
                set_postgres_search_path(&mut tx, schema).await?;
                let count = sqlx::query_scalar(sql)
                    .bind(CORE_IMPORT)
                    .fetch_one(&mut *tx)
                    .await?;
                let owned = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_schema = $1 AND table_name = $2)")
                    .bind(schema).bind(OWNER_TABLE).fetch_one(&mut *tx).await?;
                tx.commit().await?;
                (count, owned)
            }
            NamespaceBackend::Sqlite(pool) => {
                let count = sqlx::query_scalar(sql)
                    .bind(CORE_IMPORT)
                    .fetch_one(pool)
                    .await?;
                let owned = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = $1)",
                )
                .bind(OWNER_TABLE)
                .fetch_one(pool)
                .await?;
                (count, owned)
            }
        };
        ensure!(
            count == 1,
            "CoreSecurity adoption requires the existing complete import receipt"
        );
        ensure!(
            !owned,
            "namespace is already core-owned but its database authority is missing"
        );
        Ok(())
    }

    pub(crate) async fn open_core(
        backend: NamespaceBackend,
        authority: &Authority,
        phase: Phase,
        checkpoint: &impl Fn(crate::store::HandoffPoint) -> Result<()>,
    ) -> Result<Self> {
        match &backend {
            NamespaceBackend::Postgres { pool, schema } => {
                let mut tx = pool.begin().await?;
                set_postgres_search_path(&mut tx, schema).await?;
                sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 4962901))")
                    .bind(schema)
                    .execute(&mut *tx)
                    .await?;
                let tables: Vec<String> = sqlx::query_scalar(
                    "SELECT table_name FROM information_schema.tables WHERE table_schema = $1",
                )
                .bind(schema)
                .fetch_all(&mut *tx)
                .await?;
                finish_namespace!(tx, authority, phase, tables, true);
                checkpoint(crate::store::HandoffPoint::NamespacePrepared)?;
                tx.commit().await?;
            }
            NamespaceBackend::Sqlite(pool) => {
                // Reserve the single writer before reading schema or receipts.
                let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
                let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
                    .fetch_all(&mut *tx).await?;
                finish_namespace!(tx, authority, phase, tables, false);
                checkpoint(crate::store::HandoffPoint::NamespacePrepared)?;
                tx.commit().await?;
            }
        }
        Ok(Self { backend })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest as _, Sha256};

    #[test]
    fn core_baselines_are_byte_identical_to_the_immutable_legacy_sql() {
        let host = crate::plugin_host::PluginHostSpec::from_json(include_bytes!(
            "../../examples/authentication/passkey/host.json"
        ));
        // Resolved below from the repository root, not a runtime Plugin package.
        assert!(host.is_ok());
        let storage = host.unwrap().storage.unwrap();
        for (postgres, sql, checksum) in [
            (
                true,
                storage.postgres.migrations[0].sql.as_str(),
                LEGACY_POSTGRES_CHECKSUM,
            ),
            (
                false,
                storage.sqlite.migrations[0].sql.as_str(),
                LEGACY_SQLITE_CHECKSUM,
            ),
        ] {
            assert_eq!(baseline(postgres), sql);
            assert_eq!(
                format!("sha256:{:x}", Sha256::digest(baseline(postgres).as_bytes())),
                checksum
            );
        }
    }
}
