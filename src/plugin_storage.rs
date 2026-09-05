//! Plugin-owned storage on the controller database backend.
//!
//! `PostgreSQL`: one schema `plugin_<id>` per plugin, `search_path` locked, no
//! access to `public`. `SQLite`: one file under `plugins/live/<id>/state/`.
//! Core `SQLx` migrations are never used. A plugin migration failure rejects
//! the candidate runtime; an explicitly selected host must activate at startup.

#![warn(clippy::pedantic)]

use std::str::FromStr as _;
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use sha2::{Digest as _, Sha256};
use sqlx::postgres::PgPool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Executor as _, Row as _, SqlitePool};

use crate::plugin_dir::PluginDir;
use crate::plugin_host::{
    PluginSqlMigrations, PluginStorageSpec, postgres_schema_name, split_sql_statements,
    validate_plugin_id, validate_plugin_sql,
};

const MIGRATION_TABLE: &str = "_cowboy_plugin_migrations";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginStorageKind {
    Postgres,
    Sqlite,
}

#[derive(Clone)]
pub struct PluginStorage {
    kind: PluginStorageKind,
    postgres: Option<PgPool>,
    plugin_dir: PluginDir,
}

#[derive(Clone)]
pub struct PluginNamespace {
    plugin_id: String,
    pub(crate) backend: NamespaceBackend,
}

#[derive(Clone)]
pub(crate) enum NamespaceBackend {
    Postgres { pool: PgPool, schema: String },
    Sqlite(SqlitePool),
}

impl PluginStorage {
    #[must_use]
    pub fn sqlite_files(plugin_dir: PluginDir) -> Self {
        Self {
            kind: PluginStorageKind::Sqlite,
            postgres: None,
            plugin_dir,
        }
    }

    #[must_use]
    pub fn postgres(pool: PgPool, plugin_dir: PluginDir) -> Self {
        Self {
            kind: PluginStorageKind::Postgres,
            postgres: Some(pool),
            plugin_dir,
        }
    }

    #[must_use]
    pub fn kind(&self) -> PluginStorageKind {
        self.kind
    }

    #[must_use]
    pub fn plugin_dir(&self) -> &PluginDir {
        &self.plugin_dir
    }

    /// Apply the plugin's dialect-matching migrations.
    ///
    /// # Errors
    /// Returns when the plugin id, SQL, or backend apply fails.
    pub async fn migrate_plugin(
        &self,
        plugin_id: &str,
        storage: &PluginStorageSpec,
    ) -> Result<PluginNamespace> {
        validate_plugin_id(plugin_id)?;
        self.plugin_dir.ensure_state_dir(plugin_id)?;
        let namespace = match self.kind {
            PluginStorageKind::Postgres => {
                let pool = self
                    .postgres
                    .clone()
                    .context("plugin PostgreSQL pool is missing")?;
                migrate_postgres(&pool, plugin_id, &storage.postgres).await
            }
            PluginStorageKind::Sqlite => {
                migrate_sqlite(&self.plugin_dir, plugin_id, &storage.sqlite).await
            }
        }?;
        let applied = namespace
            .fetch_i64(&format!("SELECT COUNT(*) FROM {MIGRATION_TABLE}"))
            .await
            .context("plugin migration health check")?;
        ensure!(applied > 0, "plugin {plugin_id} has no applied migrations");
        Ok(namespace)
    }
}

impl PluginNamespace {
    #[must_use]
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    #[must_use]
    pub fn schema_name(&self) -> Option<&str> {
        match &self.backend {
            NamespaceBackend::Postgres { schema, .. } => Some(schema.as_str()),
            NamespaceBackend::Sqlite(_) => None,
        }
    }

    /// Execute a single guarded statement inside the plugin namespace.
    ///
    /// # Errors
    /// Returns when the statement is forbidden or the backend rejects it.
    #[allow(dead_code)]
    pub async fn execute(&self, sql: &str) -> Result<()> {
        let dialect = match self.backend {
            NamespaceBackend::Postgres { .. } => "postgres",
            NamespaceBackend::Sqlite(_) => "sqlite",
        };
        validate_plugin_sql(dialect, sql)?;
        let statements = split_sql_statements(sql)?;
        ensure!(
            statements.len() == 1,
            "plugin namespace execute is one statement"
        );
        match &self.backend {
            NamespaceBackend::Postgres { pool, schema } => {
                let mut transaction = pool.begin().await.context("begin plugin SQL")?;
                set_postgres_search_path(&mut transaction, schema).await?;
                sqlx::query(&statements[0])
                    .execute(&mut *transaction)
                    .await
                    .context("execute plugin PostgreSQL")?;
                transaction.commit().await.context("commit plugin SQL")?;
            }
            NamespaceBackend::Sqlite(pool) => {
                sqlx::query(&statements[0])
                    .execute(pool)
                    .await
                    .context("execute plugin SQLite")?;
            }
        }
        Ok(())
    }

    /// Fetch a single i64 from the plugin namespace. Test and health helper.
    ///
    /// # Errors
    /// Returns when the query fails or does not yield one integer.
    pub async fn fetch_i64(&self, sql: &str) -> Result<i64> {
        let dialect = match self.backend {
            NamespaceBackend::Postgres { .. } => "postgres",
            NamespaceBackend::Sqlite(_) => "sqlite",
        };
        validate_plugin_sql(dialect, sql)?;
        match &self.backend {
            NamespaceBackend::Postgres { pool, schema } => {
                let mut transaction = pool.begin().await.context("begin plugin fetch")?;
                set_postgres_search_path(&mut transaction, schema).await?;
                let value: i64 = sqlx::query_scalar(sql)
                    .fetch_one(&mut *transaction)
                    .await
                    .context("fetch plugin PostgreSQL")?;
                transaction.commit().await.context("commit plugin fetch")?;
                Ok(value)
            }
            NamespaceBackend::Sqlite(pool) => sqlx::query_scalar(sql)
                .fetch_one(pool)
                .await
                .context("fetch plugin SQLite"),
        }
    }
}

async fn migrate_postgres(
    pool: &PgPool,
    plugin_id: &str,
    migrations: &PluginSqlMigrations,
) -> Result<PluginNamespace> {
    migrations.validate("postgres")?;
    let schema = postgres_schema_name(plugin_id)?;
    sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
        .execute(pool)
        .await
        .with_context(|| format!("creating plugin schema {schema}"))?;
    let mut transaction = pool.begin().await.context("begin plugin migrations")?;
    set_postgres_search_path(&mut transaction, &schema).await?;
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {MIGRATION_TABLE} (
            version TEXT PRIMARY KEY,
            checksum TEXT NOT NULL,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )"
    ))
    .execute(&mut *transaction)
    .await
    .context("creating plugin migration ledger")?;
    let applied = sqlx::query(&format!(
        "SELECT version, checksum FROM {MIGRATION_TABLE} ORDER BY version"
    ))
    .fetch_all(&mut *transaction)
    .await
    .context("reading plugin migration ledger")?;
    let mut applied_map = std::collections::BTreeMap::new();
    for row in applied {
        let version: String = row.try_get("version")?;
        let checksum: String = row.try_get("checksum")?;
        applied_map.insert(version, checksum);
    }
    validate_applied_migrations(plugin_id, "PostgreSQL", migrations, &applied_map)?;
    for migration in &migrations.migrations {
        let checksum = sql_checksum(&migration.sql);
        if !applied_map.contains_key(&migration.version) {
            for statement in split_sql_statements(&migration.sql)? {
                sqlx::query(&statement)
                    .execute(&mut *transaction)
                    .await
                    .with_context(|| {
                        format!(
                            "applying plugin {plugin_id} PostgreSQL migration {}",
                            migration.version
                        )
                    })?;
            }
            sqlx::query(&format!(
                "INSERT INTO {MIGRATION_TABLE} (version, checksum) VALUES ($1, $2)"
            ))
            .bind(&migration.version)
            .bind(&checksum)
            .execute(&mut *transaction)
            .await
            .context("recording plugin PostgreSQL migration")?;
        }
    }
    transaction
        .commit()
        .await
        .context("commit plugin PostgreSQL migrations")?;
    Ok(PluginNamespace {
        plugin_id: plugin_id.to_owned(),
        backend: NamespaceBackend::Postgres {
            pool: pool.clone(),
            schema,
        },
    })
}

async fn migrate_sqlite(
    plugin_dir: &PluginDir,
    plugin_id: &str,
    migrations: &PluginSqlMigrations,
) -> Result<PluginNamespace> {
    migrations.validate("sqlite")?;
    let path = plugin_dir.sqlite_path(plugin_id)?;
    let url = format!("sqlite://{}", path.display());
    let options = SqliteConnectOptions::from_str(&url)
        .with_context(|| format!("parsing plugin SQLite URL {url}"))?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await
        .with_context(|| format!("opening plugin SQLite {}", path.display()))?;
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {MIGRATION_TABLE} (
            version TEXT PRIMARY KEY,
            checksum TEXT NOT NULL,
            applied_at_ms INTEGER NOT NULL
        )"
    ))
    .execute(&pool)
    .await
    .context("creating plugin SQLite migration ledger")?;
    let applied = sqlx::query(&format!(
        "SELECT version, checksum FROM {MIGRATION_TABLE} ORDER BY version"
    ))
    .fetch_all(&pool)
    .await
    .context("reading plugin SQLite migration ledger")?;
    let mut applied_map = std::collections::BTreeMap::new();
    for row in applied {
        let version: String = row.try_get("version")?;
        let checksum: String = row.try_get("checksum")?;
        applied_map.insert(version, checksum);
    }
    validate_applied_migrations(plugin_id, "SQLite", migrations, &applied_map)?;
    for migration in &migrations.migrations {
        let checksum = sql_checksum(&migration.sql);
        if !applied_map.contains_key(&migration.version) {
            let mut transaction = pool
                .begin()
                .await
                .context("begin plugin SQLite migration")?;
            for statement in split_sql_statements(&migration.sql)? {
                sqlx::query(&statement)
                    .execute(&mut *transaction)
                    .await
                    .with_context(|| {
                        format!(
                            "applying plugin {plugin_id} SQLite migration {}",
                            migration.version
                        )
                    })?;
            }
            sqlx::query(&format!(
                "INSERT INTO {MIGRATION_TABLE} (version, checksum, applied_at_ms) VALUES (?1, ?2, ?3)"
            ))
            .bind(&migration.version)
            .bind(&checksum)
            .bind(chrono::Utc::now().timestamp_millis())
            .execute(&mut *transaction)
            .await
            .context("recording plugin SQLite migration")?;
            transaction
                .commit()
                .await
                .context("commit plugin SQLite migration")?;
        }
    }
    Ok(PluginNamespace {
        plugin_id: plugin_id.to_owned(),
        backend: NamespaceBackend::Sqlite(pool),
    })
}

fn sql_checksum(sql: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(sql.as_bytes()))
}

fn validate_applied_migrations(
    plugin_id: &str,
    dialect: &str,
    migrations: &PluginSqlMigrations,
    applied: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    for (version, existing_checksum) in applied {
        let migration = migrations
            .migrations
            .iter()
            .find(|migration| migration.version == *version)
            .with_context(|| {
                format!(
                    "plugin {plugin_id} {dialect} migration {version} is absent from the signed manifest"
                )
            })?;
        ensure!(
            existing_checksum == &sql_checksum(&migration.sql),
            "plugin {plugin_id} {dialect} migration {version} checksum mismatch"
        );
    }
    Ok(())
}

pub(crate) async fn set_postgres_search_path(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    schema: &str,
) -> Result<()> {
    ensure!(
        schema.starts_with("plugin_")
            && schema
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
        "refusing to set search_path to {schema}"
    );
    transaction
        .execute(format!("SET LOCAL search_path TO {schema}, pg_temp").as_str())
        .await
        .context("setting plugin search_path")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_host::{PluginMigration, PluginSqlMigrations, PluginStorageSpec};
    use crate::store::Store;

    fn notes_storage() -> PluginStorageSpec {
        let sql = "CREATE TABLE notes (id TEXT PRIMARY KEY, body TEXT NOT NULL);";
        PluginStorageSpec {
            postgres: PluginSqlMigrations {
                migrations: vec![PluginMigration {
                    version: "0001".to_owned(),
                    sql: sql.to_owned(),
                }],
            },
            sqlite: PluginSqlMigrations {
                migrations: vec![PluginMigration {
                    version: "0001".to_owned(),
                    sql: sql.to_owned(),
                }],
            },
        }
    }

    fn notes_storage_v2() -> PluginStorageSpec {
        let mut storage = notes_storage();
        let migration = PluginMigration {
            version: "0002".to_owned(),
            sql: "ALTER TABLE notes ADD COLUMN category TEXT;".to_owned(),
        };
        storage.postgres.migrations.push(migration.clone());
        storage.sqlite.migrations.push(migration);
        storage
    }

    #[tokio::test]
    async fn sqlite_plugin_storage_is_isolated_from_core() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-plugin-sqlite-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let core = Store::connect(
            &format!("sqlite://{}", root.join("core.sqlite3").display()),
            root.join("artifacts"),
        )
        .await
        .unwrap();
        core.migrate().await.unwrap();
        let dir = PluginDir::open(&root).unwrap();
        let storage = core.plugin_storage(dir);
        assert_eq!(storage.kind(), PluginStorageKind::Sqlite);
        let namespace = storage
            .migrate_plugin("usage-demo", &notes_storage())
            .await
            .unwrap();
        assert_eq!(namespace.plugin_id(), "usage-demo");
        assert!(
            storage
                .plugin_dir()
                .sqlite_path("usage-demo")
                .unwrap()
                .exists()
        );
        namespace
            .execute("INSERT INTO notes (id, body) VALUES ('n1', 'hello')")
            .await
            .unwrap();
        assert_eq!(
            namespace
                .fetch_i64("SELECT COUNT(*) FROM notes")
                .await
                .unwrap(),
            1
        );
        assert!(
            namespace
                .execute("INSERT INTO sessions (id) VALUES ('x')")
                .await
                .is_err()
        );
        assert!(
            crate::plugin_host::validate_plugin_sql(
                "sqlite",
                "ATTACH DATABASE 'core.sqlite3' AS core"
            )
            .is_err()
        );
        storage
            .migrate_plugin("usage-demo", &notes_storage_v2())
            .await
            .expect("upgrade plugin storage");
        let downgrade = storage.migrate_plugin("usage-demo", &notes_storage()).await;
        assert!(downgrade.is_err());
        let downgrade = downgrade.err().expect("downgrade error");
        assert!(
            downgrade
                .to_string()
                .contains("absent from the signed manifest")
        );
        drop(core);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    #[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
    async fn postgres_plugin_schema_cannot_read_public() {
        let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
            .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
        let root = std::env::temp_dir().join(format!(
            "cowboy-plugin-pg-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let core = Store::connect(&url, root.join("artifacts")).await.unwrap();
        core.migrate().await.unwrap();
        sqlx::query("CREATE TABLE IF NOT EXISTS public.cowboy_plugin_secret (id INT PRIMARY KEY)")
            .execute(core.postgres_pool().expect("postgres pool"))
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO public.cowboy_plugin_secret (id) VALUES (7) ON CONFLICT DO NOTHING",
        )
        .execute(core.postgres_pool().expect("postgres pool"))
        .await
        .unwrap();
        let dir = PluginDir::open(&root).unwrap();
        let storage = core.plugin_storage(dir);
        assert_eq!(storage.kind(), PluginStorageKind::Postgres);
        let namespace = storage
            .migrate_plugin("usage-demo", &notes_storage())
            .await
            .unwrap();
        assert_eq!(namespace.schema_name(), Some("plugin_usage_demo"));
        namespace
            .execute("INSERT INTO notes (id, body) VALUES ('n1', 'hello')")
            .await
            .unwrap();
        assert_eq!(
            namespace
                .fetch_i64("SELECT COUNT(*) FROM notes")
                .await
                .unwrap(),
            1
        );
        assert!(
            namespace
                .fetch_i64("SELECT COUNT(*) FROM cowboy_plugin_secret")
                .await
                .is_err()
        );
        assert!(
            crate::plugin_host::validate_plugin_sql(
                "postgres",
                "SELECT COUNT(*) FROM public.cowboy_plugin_secret"
            )
            .is_err()
        );
        storage
            .migrate_plugin("usage-demo", &notes_storage_v2())
            .await
            .expect("upgrade plugin storage");
        let downgrade = storage.migrate_plugin("usage-demo", &notes_storage()).await;
        assert!(
            downgrade
                .err()
                .expect("downgrade must fail")
                .to_string()
                .contains("absent from the signed manifest")
        );
        let mut broken = notes_storage_v2();
        broken.postgres.migrations.push(PluginMigration {
            version: "0003".to_owned(),
            sql: "CREATE TABLE rollback_probe (id TEXT PRIMARY KEY); \
                  INSERT INTO no_such_table (id) VALUES ('fail');"
                .to_owned(),
        });
        assert!(storage.migrate_plugin("usage-demo", &broken).await.is_err());
        let rolled_back: bool =
            sqlx::query_scalar("SELECT to_regclass('plugin_usage_demo.rollback_probe') IS NULL")
                .fetch_one(core.postgres_pool().unwrap())
                .await
                .unwrap();
        assert!(
            rolled_back,
            "failed migration left a partially applied schema"
        );
        assert_eq!(
            namespace
                .fetch_i64("SELECT COUNT(*) FROM _cowboy_plugin_migrations")
                .await
                .unwrap(),
            2
        );
        let core_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cowboy_plugin_secret")
            .fetch_one(core.postgres_pool().unwrap())
            .await
            .unwrap();
        assert_eq!(
            core_count, 1,
            "plugin search_path leaked into the core pool"
        );
        drop(core);
        let _ = std::fs::remove_dir_all(root);
    }
}
