//! Core-owned Passkey persistence, using the existing namespace during migration.
//!
//! The legacy host selects a location, never credential SQL or a native driver.
//! This reader-compatible bridge preserves every applied migration and requires
//! an atomic first import before a typed store can be attached.

#![warn(clippy::pedantic)]

use anyhow::{Context as _, Result, ensure};
use sha2::{Digest as _, Sha256};

use crate::passkey::{ExternalPasskeyCeremonyRecord, UserPasskey};
use crate::plugin_storage::{NamespaceBackend, PluginNamespace, set_postgres_search_path};
use crate::store::Store;

const CORE_IMPORT: &str = "core_passkeys";
pub(crate) const STORAGE_CAPABILITY: &str = "webauthn";

/// Only the checked legacy handoff can construct this core storage port.
/// No generic SQL execution or Plugin capability dispatch is exposed.
#[derive(Clone)]
pub(crate) struct PasskeyStorage {
    backend: NamespaceBackend,
}

/// Shared by all Store clones, including clones created before startup binding.
/// Serialize initialization *before* any import, not only at the final pointer
/// swap, so competing initializers cannot copy credentials into two locations.
#[derive(Default)]
pub(crate) struct PasskeyBinding {
    storage: std::sync::OnceLock<PasskeyStorage>,
    initialization: tokio::sync::Mutex<()>,
}

impl PasskeyBinding {
    pub(crate) fn get(&self) -> Option<&PasskeyStorage> {
        self.storage.get()
    }

    pub(crate) async fn attach(&self, namespace: &PluginNamespace, store: &Store) -> Result<()> {
        let _initialization = self.initialization.lock().await;
        ensure!(
            self.storage.get().is_none(),
            "Passkey storage is already bound"
        );
        let storage = PasskeyStorage::open_legacy(namespace, store).await?;
        self.storage
            .set(storage)
            .map_err(|_| anyhow::anyhow!("Passkey storage is already bound"))
    }
}

// The exact historical 0001 SQL remains in its immutable signed package.
// Accepting a different schema requires a separate core migration/reader gate;
// a Plugin publication or pin alone cannot upgrade credential storage.
const LEGACY_POSTGRES_CHECKSUM: &str =
    "sha256:7e3b4ad9b0cbffa54fd4067ddd6e32a90906ec6f75f704722a2f28aca33c3424";
const LEGACY_SQLITE_CHECKSUM: &str =
    "sha256:d8b43ab7949eb0919d8429acca66c60bcd0be4cdb5b4ac198c41677fd1deb385";

pub(crate) fn validate_legacy_host(host: &crate::plugin_host::PluginHostSpec) -> Result<()> {
    if !host
        .native_capabilities
        .iter()
        .any(|name| name == STORAGE_CAPABILITY)
    {
        return Ok(());
    }
    let storage = host
        .storage
        .as_ref()
        .context("CoreSecurity requires Passkey storage")?;
    for (migrations, checksum) in [
        (&storage.postgres, LEGACY_POSTGRES_CHECKSUM),
        (&storage.sqlite, LEGACY_SQLITE_CHECKSUM),
    ] {
        ensure!(
            migrations.migrations.len() == 1
                && migrations.migrations[0].version == "0001"
                && format!(
                    "sha256:{:x}",
                    Sha256::digest(migrations.migrations[0].sql.as_bytes())
                ) == checksum,
            "CoreSecurity Passkey schema is frozen; a Plugin selection cannot migrate credentials"
        );
    }
    Ok(())
}

impl PasskeyStorage {
    /// Startup-only bridge, before accepting requests. Does not move the tables,
    /// switch host policy, or claim to fence a concurrently running old server.
    async fn open_legacy(namespace: &PluginNamespace, store: &Store) -> Result<Self> {
        let ledger_sql = "SELECT version, checksum FROM _cowboy_plugin_migrations";
        let (rows, checksum) = match &namespace.backend {
            NamespaceBackend::Postgres { pool, schema } => {
                let mut tx = pool.begin().await?;
                set_postgres_search_path(&mut tx, schema).await?;
                let rows = sqlx::query_as::<_, (String, String)>(ledger_sql)
                    .fetch_all(&mut *tx)
                    .await?;
                tx.commit().await?;
                (rows, LEGACY_POSTGRES_CHECKSUM)
            }
            NamespaceBackend::Sqlite(pool) => (
                sqlx::query_as::<_, (String, String)>(ledger_sql)
                    .fetch_all(pool)
                    .await?,
                LEGACY_SQLITE_CHECKSUM,
            ),
        };
        ensure!(
            rows.len() == 1 && rows[0].0 == "0001" && rows[0].1 == checksum,
            "CoreSecurity refuses an incompatible Passkey migration ledger"
        );
        let storage = Self {
            backend: namespace.backend.clone(),
        };
        import_from_core(&storage, store).await?;
        Ok(storage)
    }
}

pub async fn list_user(ns: &PasskeyStorage, user_id: &str) -> Result<Vec<UserPasskey>> {
    list_passkeys(ns, "user_passkeys", "user_id", user_id).await
}

pub async fn list_admin(ns: &PasskeyStorage, account: &str) -> Result<Vec<UserPasskey>> {
    list_passkeys(ns, "admin_passkeys", "account", account).await
}

pub async fn count_user(ns: &PasskeyStorage, user_id: &str) -> Result<u32> {
    count_passkeys(ns, "user_passkeys", "user_id", user_id).await
}

pub async fn count_admin(ns: &PasskeyStorage, account: &str) -> Result<u32> {
    count_passkeys(ns, "admin_passkeys", "account", account).await
}

pub async fn insert_user(ns: &PasskeyStorage, passkey: &UserPasskey) -> Result<()> {
    insert_passkey(ns, "user_passkeys", "user_id", passkey).await
}

pub async fn insert_admin(ns: &PasskeyStorage, passkey: &UserPasskey) -> Result<()> {
    insert_passkey(ns, "admin_passkeys", "account", passkey).await
}

pub async fn delete_user(ns: &PasskeyStorage, user_id: &str, passkey_id: &str) -> Result<u64> {
    delete_passkey(ns, "user_passkeys", "user_id", user_id, passkey_id).await
}

pub async fn delete_admin(ns: &PasskeyStorage, account: &str, passkey_id: &str) -> Result<u64> {
    delete_passkey(ns, "admin_passkeys", "account", account, passkey_id).await
}

pub async fn update_user(
    ns: &PasskeyStorage,
    user_id: &str,
    passkey_id: &str,
    passkey_json: &str,
    now_ms: i64,
) -> Result<()> {
    update_passkey(
        ns,
        "user_passkeys",
        "user_id",
        user_id,
        passkey_id,
        passkey_json,
        now_ms,
    )
    .await
}

pub async fn update_admin(
    ns: &PasskeyStorage,
    account: &str,
    passkey_id: &str,
    passkey_json: &str,
    now_ms: i64,
) -> Result<()> {
    update_passkey(
        ns,
        "admin_passkeys",
        "account",
        account,
        passkey_id,
        passkey_json,
        now_ms,
    )
    .await
}

pub async fn upsert_ceremony(
    ns: &PasskeyStorage,
    ceremony: &ExternalPasskeyCeremonyRecord,
) -> Result<()> {
    upsert_ceremony_at(ns, ceremony, chrono::Utc::now().timestamp_millis()).await
}

async fn upsert_ceremony_at(
    ns: &PasskeyStorage,
    ceremony: &ExternalPasskeyCeremonyRecord,
    now_ms: i64,
) -> Result<()> {
    match &ns.backend {
        NamespaceBackend::Postgres { pool, schema } => {
            let mut tx = pool.begin().await.context("begin plugin ceremony")?;
            set_postgres_search_path(&mut tx, schema).await?;
            sqlx::query("DELETE FROM external_passkey_ceremonies WHERE expires_at_ms <= $1")
                .bind(now_ms)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO external_passkey_ceremonies \
                 (transaction_hash, ceremony_json, expires_at_ms, created_at_ms) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (transaction_hash) DO UPDATE SET \
                 ceremony_json = EXCLUDED.ceremony_json, expires_at_ms = EXCLUDED.expires_at_ms",
            )
            .bind(&ceremony.transaction_hash)
            .bind(&ceremony.ceremony_json)
            .bind(ceremony.expires_at_ms)
            .bind(ceremony.created_at_ms)
            .execute(&mut *tx)
            .await?;
            tx.commit().await.context("commit plugin ceremony")?;
        }
        NamespaceBackend::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin core ceremony")?;
            sqlx::query("DELETE FROM external_passkey_ceremonies WHERE expires_at_ms <= ?1")
                .bind(now_ms)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO external_passkey_ceremonies \
                 (transaction_hash, ceremony_json, expires_at_ms, created_at_ms) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (transaction_hash) DO UPDATE SET \
                 ceremony_json = excluded.ceremony_json, expires_at_ms = excluded.expires_at_ms",
            )
            .bind(&ceremony.transaction_hash)
            .bind(&ceremony.ceremony_json)
            .bind(ceremony.expires_at_ms)
            .bind(ceremony.created_at_ms)
            .execute(&mut *tx)
            .await?;
            tx.commit().await.context("commit core ceremony")?;
        }
    }
    Ok(())
}

pub async fn ceremony(
    ns: &PasskeyStorage,
    transaction_hash: &str,
    now_ms: i64,
) -> Result<Option<ExternalPasskeyCeremonyRecord>> {
    match &ns.backend {
        NamespaceBackend::Postgres { pool, schema } => {
            let mut tx = pool.begin().await.context("begin plugin ceremony select")?;
            set_postgres_search_path(&mut tx, schema).await?;
            let row = sqlx::query_as::<_, (String, String, i64, i64)>(
                "SELECT transaction_hash, ceremony_json, expires_at_ms, created_at_ms \
                 FROM external_passkey_ceremonies \
                 WHERE transaction_hash = $1 AND expires_at_ms > $2",
            )
            .bind(transaction_hash)
            .bind(now_ms)
            .fetch_optional(&mut *tx)
            .await?;
            tx.commit().await.ok();
            Ok(row.map(into_ceremony))
        }
        NamespaceBackend::Sqlite(pool) => {
            let row = sqlx::query_as::<_, (String, String, i64, i64)>(
                "SELECT transaction_hash, ceremony_json, expires_at_ms, created_at_ms \
                 FROM external_passkey_ceremonies \
                 WHERE transaction_hash = ?1 AND expires_at_ms > ?2",
            )
            .bind(transaction_hash)
            .bind(now_ms)
            .fetch_optional(pool)
            .await?;
            Ok(row.map(into_ceremony))
        }
    }
}

// Shared SQL is deliberately instantiated against both typed SQLx backends.
// The caller owns the namespace write lock and commits only after the ledger.
macro_rules! import_snapshot {
    ($tx:ident, $store:ident) => {{
        let applied: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM _cowboy_import WHERE name = $1",
        )
        .bind(CORE_IMPORT)
        .fetch_one(&mut *$tx)
        .await?;
        if applied == 0 {
            let destination_rows: i64 = sqlx::query_scalar(
                "SELECT (SELECT COUNT(*) FROM user_passkeys) + \
                 (SELECT COUNT(*) FROM admin_passkeys) + \
                 (SELECT COUNT(*) FROM external_passkey_ceremonies)",
            )
            .fetch_one(&mut *$tx)
            .await?;
            ensure!(
                destination_rows == 0,
                "Passkey namespace has data without its import receipt; refusing automatic reconciliation"
            );
            let snapshot = $store.export_passkey_snapshot().await?;
            for (table, owner_column, passkeys) in [
                ("user_passkeys", "user_id", &snapshot.user),
                ("admin_passkeys", "account", &snapshot.admin),
            ] {
                for passkey in passkeys {
                    let sql = format!(
                        "INSERT INTO {table} (id, {owner_column}, credential_id, nickname, \
                         passkey_json, created_at_ms, last_used_at_ms) \
                         VALUES ($1, $2, $3, $4, $5, $6, $7)"
                    );
                    sqlx::query(&sql)
                        .bind(&passkey.id)
                        .bind(&passkey.user_id)
                        .bind(&passkey.credential_id)
                        .bind(&passkey.nickname)
                        .bind(&passkey.passkey_json)
                        .bind(passkey.created_at_ms)
                        .bind(passkey.last_used_at_ms)
                        .execute(&mut *$tx)
                        .await?;
                }
            }
            // Preserve every record exactly. Import is not ceremony GC, and
            // order must not make a later expiry delete an earlier live flow.
            for row in &snapshot.ceremonies {
                sqlx::query(
                    "INSERT INTO external_passkey_ceremonies \
                     (transaction_hash, ceremony_json, expires_at_ms, created_at_ms) \
                     VALUES ($1, $2, $3, $4)",
                )
                .bind(&row.transaction_hash)
                .bind(&row.ceremony_json)
                .bind(row.expires_at_ms)
                .bind(row.created_at_ms)
                .execute(&mut *$tx)
                .await?;
            }
            sqlx::query("INSERT INTO _cowboy_import (name, applied_at_ms) VALUES ($1, $2)")
                .bind(CORE_IMPORT)
                .bind(chrono::Utc::now().timestamp_millis())
                .execute(&mut *$tx)
                .await?;
        }
    }};
}

/// Copy the stopped legacy core source exactly once, with all rows and the
/// existing receipt committed atomically. A missing receipt on a nonempty
/// destination is ambiguous (including a partial import by an old Controller)
/// and must be reconciled explicitly; never guess or overwrite current data.
async fn import_from_core(ns: &PasskeyStorage, store: &Store) -> Result<()> {
    match &ns.backend {
        NamespaceBackend::Postgres { pool, schema } => {
            let mut tx = pool.begin().await.context("begin core Passkey import")?;
            set_postgres_search_path(&mut tx, schema).await?;
            sqlx::query(
                "LOCK TABLE _cowboy_import, user_passkeys, admin_passkeys, \
                 external_passkey_ceremonies IN SHARE ROW EXCLUSIVE MODE",
            )
            .execute(&mut *tx)
            .await?;
            import_snapshot!(tx, store);
            tx.commit().await.context("commit core Passkey import")?;
        }
        NamespaceBackend::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin core Passkey import")?;
            // Acquire SQLite's writer reservation before reading the receipt.
            // This updates no rows, including when the ledger is empty.
            sqlx::query("UPDATE _cowboy_import SET applied_at_ms = applied_at_ms WHERE 0 = 1")
                .execute(&mut *tx)
                .await?;
            import_snapshot!(tx, store);
            tx.commit().await.context("commit core Passkey import")?;
        }
    }
    Ok(())
}

async fn list_passkeys(
    ns: &PasskeyStorage,
    table: &str,
    owner_column: &str,
    owner: &str,
) -> Result<Vec<UserPasskey>> {
    validate_ident(table)?;
    validate_ident(owner_column)?;
    match &ns.backend {
        NamespaceBackend::Postgres { pool, schema } => {
            let mut tx = pool.begin().await?;
            set_postgres_search_path(&mut tx, schema).await?;
            let sql = format!(
                "SELECT id, {owner_column} AS user_id, credential_id, nickname, passkey_json, \
                 created_at_ms, last_used_at_ms FROM {table} WHERE {owner_column} = $1 \
                 ORDER BY created_at_ms"
            );
            let rows = sqlx::query_as::<_, PasskeyRow>(&sql)
                .bind(owner)
                .fetch_all(&mut *tx)
                .await?;
            tx.commit().await.ok();
            Ok(rows.into_iter().map(PasskeyRow::into_passkey).collect())
        }
        NamespaceBackend::Sqlite(pool) => {
            let sql = format!(
                "SELECT id, {owner_column} AS user_id, credential_id, nickname, passkey_json, \
                 created_at_ms, last_used_at_ms FROM {table} WHERE {owner_column} = ?1 \
                 ORDER BY created_at_ms"
            );
            let rows = sqlx::query_as::<_, PasskeyRow>(&sql)
                .bind(owner)
                .fetch_all(pool)
                .await?;
            Ok(rows.into_iter().map(PasskeyRow::into_passkey).collect())
        }
    }
}

async fn count_passkeys(
    ns: &PasskeyStorage,
    table: &str,
    owner_column: &str,
    owner: &str,
) -> Result<u32> {
    validate_ident(table)?;
    validate_ident(owner_column)?;
    let count: i64 = match &ns.backend {
        NamespaceBackend::Postgres { pool, schema } => {
            let mut tx = pool.begin().await?;
            set_postgres_search_path(&mut tx, schema).await?;
            let sql = format!("SELECT COUNT(*) FROM {table} WHERE {owner_column} = $1");
            let count = sqlx::query_scalar(&sql)
                .bind(owner)
                .fetch_one(&mut *tx)
                .await?;
            tx.commit().await.ok();
            count
        }
        NamespaceBackend::Sqlite(pool) => {
            let sql = format!("SELECT COUNT(*) FROM {table} WHERE {owner_column} = ?1");
            sqlx::query_scalar(&sql).bind(owner).fetch_one(pool).await?
        }
    };
    Ok(u32::try_from(count.max(0)).unwrap_or(0))
}

async fn insert_passkey(
    ns: &PasskeyStorage,
    table: &str,
    owner_column: &str,
    passkey: &UserPasskey,
) -> Result<()> {
    validate_ident(table)?;
    validate_ident(owner_column)?;
    match &ns.backend {
        NamespaceBackend::Postgres { pool, schema } => {
            let mut tx = pool.begin().await?;
            set_postgres_search_path(&mut tx, schema).await?;
            let sql = format!(
                "INSERT INTO {table} (id, {owner_column}, credential_id, nickname, passkey_json, \
                 created_at_ms, last_used_at_ms) VALUES ($1, $2, $3, $4, $5, $6, $7)"
            );
            sqlx::query(&sql)
                .bind(&passkey.id)
                .bind(&passkey.user_id)
                .bind(&passkey.credential_id)
                .bind(&passkey.nickname)
                .bind(&passkey.passkey_json)
                .bind(passkey.created_at_ms)
                .bind(passkey.last_used_at_ms)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        NamespaceBackend::Sqlite(pool) => {
            let sql = format!(
                "INSERT INTO {table} (id, {owner_column}, credential_id, nickname, passkey_json, \
                 created_at_ms, last_used_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
            );
            sqlx::query(&sql)
                .bind(&passkey.id)
                .bind(&passkey.user_id)
                .bind(&passkey.credential_id)
                .bind(&passkey.nickname)
                .bind(&passkey.passkey_json)
                .bind(passkey.created_at_ms)
                .bind(passkey.last_used_at_ms)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

async fn delete_passkey(
    ns: &PasskeyStorage,
    table: &str,
    owner_column: &str,
    owner: &str,
    passkey_id: &str,
) -> Result<u64> {
    validate_ident(table)?;
    validate_ident(owner_column)?;
    match &ns.backend {
        NamespaceBackend::Postgres { pool, schema } => {
            let mut tx = pool.begin().await?;
            set_postgres_search_path(&mut tx, schema).await?;
            let sql = format!("DELETE FROM {table} WHERE id = $1 AND {owner_column} = $2");
            let result = sqlx::query(&sql)
                .bind(passkey_id)
                .bind(owner)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok(result.rows_affected())
        }
        NamespaceBackend::Sqlite(pool) => {
            let sql = format!("DELETE FROM {table} WHERE id = ?1 AND {owner_column} = ?2");
            let result = sqlx::query(&sql)
                .bind(passkey_id)
                .bind(owner)
                .execute(pool)
                .await?;
            Ok(result.rows_affected())
        }
    }
}

async fn update_passkey(
    ns: &PasskeyStorage,
    table: &str,
    owner_column: &str,
    owner: &str,
    passkey_id: &str,
    passkey_json: &str,
    now_ms: i64,
) -> Result<()> {
    validate_ident(table)?;
    validate_ident(owner_column)?;
    let affected = match &ns.backend {
        NamespaceBackend::Postgres { pool, schema } => {
            let mut tx = pool.begin().await?;
            set_postgres_search_path(&mut tx, schema).await?;
            let sql = format!(
                "UPDATE {table} SET passkey_json = $3, last_used_at_ms = $4 \
                 WHERE id = $1 AND {owner_column} = $2"
            );
            let result = sqlx::query(&sql)
                .bind(passkey_id)
                .bind(owner)
                .bind(passkey_json)
                .bind(now_ms)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            result.rows_affected()
        }
        NamespaceBackend::Sqlite(pool) => {
            let sql = format!(
                "UPDATE {table} SET passkey_json = ?3, last_used_at_ms = ?4 \
                 WHERE id = ?1 AND {owner_column} = ?2"
            );
            sqlx::query(&sql)
                .bind(passkey_id)
                .bind(owner)
                .bind(passkey_json)
                .bind(now_ms)
                .execute(pool)
                .await?
                .rows_affected()
        }
    };
    ensure!(affected == 1, "passkey not found");
    Ok(())
}

fn validate_ident(value: &str) -> Result<()> {
    ensure!(
        matches!(
            value,
            "user_passkeys" | "admin_passkeys" | "user_id" | "account"
        ),
        "refusing plugin SQL identifier {value}"
    );
    Ok(())
}

fn into_ceremony(
    (transaction_hash, ceremony_json, expires_at_ms, created_at_ms): (String, String, i64, i64),
) -> ExternalPasskeyCeremonyRecord {
    ExternalPasskeyCeremonyRecord {
        transaction_hash,
        ceremony_json,
        expires_at_ms,
        created_at_ms,
    }
}

#[derive(sqlx::FromRow)]
struct PasskeyRow {
    id: String,
    user_id: String,
    credential_id: String,
    nickname: String,
    passkey_json: String,
    created_at_ms: i64,
    last_used_at_ms: Option<i64>,
}

impl PasskeyRow {
    fn into_passkey(self) -> UserPasskey {
        UserPasskey {
            id: self.id,
            user_id: self.user_id,
            credential_id: self.credential_id,
            nickname: self.nickname,
            passkey_json: self.passkey_json,
            created_at_ms: self.created_at_ms,
            last_used_at_ms: self.last_used_at_ms,
        }
    }
}

#[cfg(test)]
mod tests;
