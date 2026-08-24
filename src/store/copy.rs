//! One-time, fail-closed durable-store copy operations.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::str::FromStr as _;
use std::time::Duration;

use anyhow::{Context as _, Result};
use futures::TryStreamExt as _;
use serde::Serialize;
use sqlx::postgres::PgConnection;
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqliteSynchronous};
use sqlx::{Connection as _, Row as _};

use super::sqlite::SqliteStorage;

const TABLES: &[&str] = &[
    "machines",
    "users",
    "sessions",
    "events",
    "settings",
    "scheduled_wakeups",
    "scheduled_provider_actions",
    "provider_action_logs",
    "machine_enrollment_tokens",
    "runtime_incidents",
    "provider_usage_events",
    "provider_usage_producers",
    "user_sessions",
    "user_api_tokens",
    "user_passkeys",
    "admin_passkeys",
];

#[derive(Debug, Serialize)]
pub(crate) struct StoreCopyReport {
    source_backend: &'static str,
    destination_backend: &'static str,
    tables: BTreeMap<String, u64>,
    total_rows: u64,
    destination_bytes: u64,
    foreign_key_check: &'static str,
    integrity_check: &'static str,
}

#[derive(Debug)]
struct PgColumn {
    name: String,
    data_type: String,
}

fn quote_identifier(identifier: &str) -> Result<String> {
    anyhow::ensure!(
        !identifier.is_empty()
            && identifier
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
        "unsafe SQL identifier {identifier:?}"
    );
    Ok(format!("\"{identifier}\""))
}

fn source_column_name<'a>(
    destination: &str,
    source: &'a BTreeMap<String, PgColumn>,
) -> Option<&'a str> {
    if let Some((name, _)) = source.get_key_value(destination) {
        return Some(name);
    }
    destination
        .strip_suffix("_ms")
        .and_then(|candidate| source.get_key_value(candidate))
        .map(|(name, _)| name.as_str())
}

fn postgres_projection(destination: &str, source: &BTreeMap<String, PgColumn>) -> Result<String> {
    let source_name = source_column_name(destination, source).with_context(|| {
        format!("SQLite column {destination:?} has no PostgreSQL source column")
    })?;
    let column = source
        .get(source_name)
        .context("resolved PostgreSQL source column disappeared")?;
    let quoted_source = quote_identifier(&column.name)?;
    let expression = if column.data_type == "timestamp with time zone"
        || column.data_type == "timestamp without time zone"
    {
        format!(
            "CASE WHEN {quoted_source} IS NULL THEN NULL ELSE round(extract(epoch FROM {quoted_source}) * 1000)::bigint END"
        )
    } else {
        quoted_source
    };
    Ok(format!(
        "{expression} AS {}",
        quote_identifier(destination)?
    ))
}

fn ensure_column_contract(
    table: &str,
    source: &BTreeMap<String, PgColumn>,
    destination: &[String],
) -> Result<()> {
    let mapped = destination
        .iter()
        .filter_map(|column| source_column_name(column, source))
        .collect::<BTreeSet<_>>();
    let source_names = source.keys().map(String::as_str).collect::<BTreeSet<_>>();
    anyhow::ensure!(
        mapped == source_names,
        "column set for {table} does not match the copy contract: PostgreSQL {source_names:?}, mapped by SQLite {mapped:?}"
    );
    Ok(())
}

fn sqlite_insert_sql(table: &str, columns: &[String]) -> Result<String> {
    let table = quote_identifier(table)?;
    let quoted = columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Result<Vec<_>>>()?;
    let extracted = columns
        .iter()
        .map(|column| format!("json_extract(?1, '$.\"{column}\"')"))
        .collect::<Vec<_>>();
    Ok(format!(
        "INSERT INTO {table} ({}) SELECT {}",
        quoted.join(", "),
        extracted.join(", ")
    ))
}

async fn postgres_tables(source: &mut PgConnection) -> Result<BTreeSet<String>> {
    let rows: Vec<String> = sqlx::query_scalar(
        r"SELECT table_name
          FROM information_schema.tables
          WHERE table_schema = 'public'
            AND table_type = 'BASE TABLE'
            AND table_name <> '_sqlx_migrations'
          ORDER BY table_name",
    )
    .fetch_all(source)
    .await
    .context("listing PostgreSQL Cowboy tables")?;
    Ok(rows.into_iter().collect())
}

async fn sqlite_tables(destination: &mut SqliteConnection) -> Result<BTreeSet<String>> {
    let rows: Vec<String> = sqlx::query_scalar(
        r"SELECT name
          FROM sqlite_schema
          WHERE type = 'table'
            AND name NOT LIKE 'sqlite_%'
            AND name <> '_sqlx_migrations'
          ORDER BY name",
    )
    .fetch_all(destination)
    .await
    .context("listing SQLite Cowboy tables")?;
    Ok(rows.into_iter().collect())
}

async fn postgres_columns(
    source: &mut PgConnection,
    table: &str,
) -> Result<BTreeMap<String, PgColumn>> {
    let rows = sqlx::query(
        r"SELECT column_name, data_type
          FROM information_schema.columns
          WHERE table_schema = 'public' AND table_name = $1
          ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(source)
    .await
    .with_context(|| format!("listing PostgreSQL columns for {table}"))?;
    rows.into_iter()
        .map(|row| {
            let name: String = row.try_get("column_name")?;
            let data_type: String = row.try_get("data_type")?;
            Ok((name.clone(), PgColumn { name, data_type }))
        })
        .collect::<std::result::Result<_, sqlx::Error>>()
        .with_context(|| format!("decoding PostgreSQL columns for {table}"))
}

async fn sqlite_columns(destination: &mut SqliteConnection, table: &str) -> Result<Vec<String>> {
    let sql = format!("PRAGMA table_info({})", quote_identifier(table)?);
    let rows = sqlx::query(&sql)
        .fetch_all(destination)
        .await
        .with_context(|| format!("listing SQLite columns for {table}"))?;
    rows.into_iter()
        .map(|row| row.try_get("name"))
        .collect::<std::result::Result<_, sqlx::Error>>()
        .with_context(|| format!("decoding SQLite columns for {table}"))
}

/// Copy a complete `PostgreSQL` Cowboy store into a newly created `SQLite` file.
///
/// The source is held in one repeatable-read, read-only transaction. The
/// destination is written in one transaction and is removed on every failure,
/// so a caller can never mistake a partial copy for a cutover candidate.
#[allow(clippy::too_many_lines)] // one fail-closed snapshot/copy/validation transaction
pub(crate) async fn postgres_to_sqlite(
    source_url: &str,
    destination_url: &str,
    artifact_dir: PathBuf,
) -> Result<StoreCopyReport> {
    anyhow::ensure!(
        source_url.starts_with("postgres://") || source_url.starts_with("postgresql://"),
        "store-copy source must be PostgreSQL"
    );
    anyhow::ensure!(
        destination_url.starts_with("sqlite:"),
        "store-copy destination must be SQLite"
    );

    let destination_options = SqliteConnectOptions::from_str(destination_url)
        .context("parsing SQLite destination URL")?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));
    let destination_path = destination_options.get_filename().to_owned();
    anyhow::ensure!(
        destination_path != std::path::Path::new(":memory:"),
        "store-copy requires a file-backed SQLite destination"
    );
    anyhow::ensure!(
        !destination_path.exists(),
        "refusing to overwrite SQLite destination {}",
        destination_path.display()
    );
    let parent = destination_path
        .parent()
        .context("SQLite destination has no parent directory")?;
    anyhow::ensure!(
        parent.is_dir(),
        "SQLite destination parent does not exist: {}",
        parent.display()
    );

    let copy_result = async {
        let sqlite_storage = SqliteStorage::connect(destination_url, artifact_dir).await?;
        sqlite_storage.migrate().await?;
        drop(sqlite_storage);

        let mut source = PgConnection::connect(source_url)
            .await
            .context("connecting to PostgreSQL source")?;
        let mut destination = SqliteConnection::connect_with(&destination_options)
            .await
            .context("connecting to SQLite destination")?;

        let expected: BTreeSet<String> = TABLES.iter().map(|table| (*table).to_owned()).collect();
        let source_tables = postgres_tables(&mut source).await?;
        anyhow::ensure!(
            source_tables == expected,
            "PostgreSQL table set does not match the copy contract: expected {expected:?}, found {source_tables:?}"
        );
        let destination_tables = sqlite_tables(&mut destination).await?;
        anyhow::ensure!(
            destination_tables == expected,
            "SQLite table set does not match the copy contract: expected {expected:?}, found {destination_tables:?}"
        );

        let mut source_transaction = source.begin().await.context("starting source snapshot")?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *source_transaction)
            .await
            .context("configuring source snapshot")?;
        let mut destination_transaction = destination
            .begin()
            .await
            .context("starting SQLite copy transaction")?;
        sqlx::query("DELETE FROM machines")
            .execute(&mut *destination_transaction)
            .await
            .context("removing SQLite bootstrap Machine row")?;

        let mut copied = BTreeMap::new();
        for table in TABLES {
            eprintln!("copying {table}...");
            let source_columns = postgres_columns(&mut source_transaction, table).await?;
            let destination_columns = sqlite_columns(&mut destination_transaction, table).await?;
            ensure_column_contract(table, &source_columns, &destination_columns)?;
            let projections = destination_columns
                .iter()
                .map(|column| postgres_projection(column, &source_columns))
                .collect::<Result<Vec<_>>>()?;
            let select_sql = format!(
                "SELECT row_to_json(projected)::text FROM (SELECT {} FROM {}) AS projected",
                projections.join(", "),
                quote_identifier(table)?
            );
            let insert_sql = sqlite_insert_sql(table, &destination_columns)?;
            let mut rows = sqlx::query_scalar::<_, String>(&select_sql)
                .fetch(&mut *source_transaction);
            let mut count = 0_u64;
            while let Some(row) = rows
                .try_next()
                .await
                .with_context(|| format!("reading PostgreSQL row from {table}"))?
            {
                sqlx::query(&insert_sql)
                    .bind(row)
                    .execute(&mut *destination_transaction)
                    .await
                    .with_context(|| format!("writing SQLite row to {table}"))?;
                count = count.saturating_add(1);
                if count.is_multiple_of(10_000) {
                    eprintln!("copied {table}: {count} rows");
                }
            }
            drop(rows);
            copied.insert((*table).to_owned(), count);
            eprintln!("copied {table}: {count} rows complete");
        }

        destination_transaction
            .commit()
            .await
            .context("committing SQLite copy")?;

        for (table, source_count) in &copied {
            let sql = format!("SELECT COUNT(*) FROM {}", quote_identifier(table)?);
            let destination_count: i64 = sqlx::query_scalar(&sql)
                .fetch_one(&mut destination)
                .await
                .with_context(|| format!("counting copied SQLite table {table}"))?;
            anyhow::ensure!(
                u64::try_from(destination_count).ok() == Some(*source_count),
                "row-count mismatch for {table}: source {source_count}, destination {destination_count}"
            );
        }

        let foreign_key_violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut destination)
            .await
            .context("checking SQLite foreign keys")?;
        anyhow::ensure!(
            foreign_key_violations.is_empty(),
            "SQLite foreign_key_check reported {} violation(s)",
            foreign_key_violations.len()
        );
        let integrity: String = sqlx::query_scalar("PRAGMA quick_check")
            .fetch_one(&mut destination)
            .await
            .context("checking SQLite database integrity")?;
        anyhow::ensure!(integrity == "ok", "SQLite quick_check failed: {integrity}");
        source_transaction
            .rollback()
            .await
            .context("closing PostgreSQL source snapshot")?;
        destination.close().await.context("closing SQLite destination")?;

        let destination_bytes = std::fs::metadata(&destination_path)
            .with_context(|| format!("reading {} metadata", destination_path.display()))?
            .len();
        let total_rows = copied.values().copied().sum();
        Ok(StoreCopyReport {
            source_backend: "postgresql",
            destination_backend: "sqlite",
            tables: copied,
            total_rows,
            destination_bytes,
            foreign_key_check: "ok",
            integrity_check: "ok",
        })
    }
    .await;

    if copy_result.is_err() {
        let _ = std::fs::remove_file(&destination_path);
        let mut wal = destination_path.as_os_str().to_owned();
        wal.push("-wal");
        let _ = std::fs::remove_file(PathBuf::from(wal));
        let mut shm = destination_path.as_os_str().to_owned();
        shm.push("-shm");
        let _ = std::fs::remove_file(PathBuf::from(shm));
    }
    copy_result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_columns_map_to_millisecond_destinations() {
        let source = BTreeMap::from([
            (
                "created_at".to_owned(),
                PgColumn {
                    name: "created_at".to_owned(),
                    data_type: "timestamp with time zone".to_owned(),
                },
            ),
            (
                "fire_at_ms".to_owned(),
                PgColumn {
                    name: "fire_at_ms".to_owned(),
                    data_type: "bigint".to_owned(),
                },
            ),
        ]);
        assert!(
            postgres_projection("created_at_ms", &source)
                .unwrap()
                .contains("extract(epoch FROM \"created_at\") * 1000")
        );
        assert_eq!(
            postgres_projection("fire_at_ms", &source).unwrap(),
            "\"fire_at_ms\" AS \"fire_at_ms\""
        );
        ensure_column_contract(
            "scheduled_wakeups",
            &source,
            &["created_at_ms".to_owned(), "fire_at_ms".to_owned()],
        )
        .unwrap();
    }

    #[test]
    fn column_contract_rejects_an_unmapped_source_column() {
        let source = BTreeMap::from([
            (
                "id".to_owned(),
                PgColumn {
                    name: "id".to_owned(),
                    data_type: "text".to_owned(),
                },
            ),
            (
                "legacy".to_owned(),
                PgColumn {
                    name: "legacy".to_owned(),
                    data_type: "text".to_owned(),
                },
            ),
        ]);
        let error = ensure_column_contract("sample", &source, &["id".to_owned()]).unwrap_err();
        assert!(error.to_string().contains("legacy"));
    }

    #[tokio::test]
    async fn sqlite_json_insert_preserves_scalars_objects_and_nulls() {
        let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE sample (id INTEGER, enabled INTEGER, payload TEXT CHECK(json_valid(payload)), optional TEXT)",
        )
        .execute(&mut connection)
        .await
        .unwrap();
        let columns = vec![
            "id".to_owned(),
            "enabled".to_owned(),
            "payload".to_owned(),
            "optional".to_owned(),
        ];
        sqlx::query(&sqlite_insert_sql("sample", &columns).unwrap())
            .bind(r#"{"id":7,"enabled":true,"payload":{"answer":42},"optional":null}"#)
            .execute(&mut connection)
            .await
            .unwrap();
        let row = sqlx::query("SELECT id, enabled, payload, optional FROM sample")
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(row.get::<i64, _>("id"), 7);
        assert_eq!(row.get::<i64, _>("enabled"), 1);
        assert_eq!(row.get::<String, _>("payload"), r#"{"answer":42}"#);
        assert_eq!(row.get::<Option<String>, _>("optional"), None);
    }
}
