//! Passkey credential rows in the passkey plugin namespace.

#![warn(clippy::pedantic)]

use anyhow::{Context as _, Result, ensure};

use crate::passkey::{ExternalPasskeyCeremonyRecord, UserPasskey};
use crate::plugin_storage::{NamespaceBackend, PluginNamespace, set_postgres_search_path};
use crate::store::Store;

const CORE_IMPORT: &str = "core_passkeys";

pub async fn list_user(ns: &PluginNamespace, user_id: &str) -> Result<Vec<UserPasskey>> {
    list_passkeys(ns, "user_passkeys", "user_id", user_id).await
}

pub async fn list_admin(ns: &PluginNamespace, account: &str) -> Result<Vec<UserPasskey>> {
    list_passkeys(ns, "admin_passkeys", "account", account).await
}

pub async fn count_user(ns: &PluginNamespace, user_id: &str) -> Result<u32> {
    count_passkeys(ns, "user_passkeys", "user_id", user_id).await
}

pub async fn count_admin(ns: &PluginNamespace, account: &str) -> Result<u32> {
    count_passkeys(ns, "admin_passkeys", "account", account).await
}

pub async fn insert_user(ns: &PluginNamespace, passkey: &UserPasskey) -> Result<()> {
    insert_passkey(ns, "user_passkeys", "user_id", passkey).await
}

pub async fn insert_admin(ns: &PluginNamespace, passkey: &UserPasskey) -> Result<()> {
    insert_passkey(ns, "admin_passkeys", "account", passkey).await
}

pub async fn delete_user(ns: &PluginNamespace, user_id: &str, passkey_id: &str) -> Result<u64> {
    delete_passkey(ns, "user_passkeys", "user_id", user_id, passkey_id).await
}

pub async fn delete_admin(ns: &PluginNamespace, account: &str, passkey_id: &str) -> Result<u64> {
    delete_passkey(ns, "admin_passkeys", "account", account, passkey_id).await
}

pub async fn update_user(
    ns: &PluginNamespace,
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
    ns: &PluginNamespace,
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
    ns: &PluginNamespace,
    ceremony: &ExternalPasskeyCeremonyRecord,
) -> Result<()> {
    match &ns.backend {
        NamespaceBackend::Postgres { pool, schema } => {
            let mut tx = pool.begin().await.context("begin plugin ceremony")?;
            set_postgres_search_path(&mut tx, schema).await?;
            sqlx::query("DELETE FROM external_passkey_ceremonies WHERE expires_at_ms <= $1")
                .bind(ceremony.expires_at_ms)
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
            sqlx::query("DELETE FROM external_passkey_ceremonies WHERE expires_at_ms <= ?1")
                .bind(ceremony.expires_at_ms)
                .execute(pool)
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
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

pub async fn ceremony(
    ns: &PluginNamespace,
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

/// Copy core passkey rows into the plugin namespace once.
///
/// # Errors
/// Returns when the plugin ledger or core export fails.
pub async fn import_from_core(ns: &PluginNamespace, store: &Store) -> Result<()> {
    if import_applied(ns).await? {
        return Ok(());
    }
    let snapshot = store.export_passkey_snapshot().await?;
    for passkey in &snapshot.user {
        insert_user(ns, passkey).await?;
    }
    for passkey in &snapshot.admin {
        insert_admin(ns, passkey).await?;
    }
    for row in &snapshot.ceremonies {
        upsert_ceremony(ns, row).await?;
    }
    mark_import(ns).await
}

async fn import_applied(ns: &PluginNamespace) -> Result<bool> {
    let count = match &ns.backend {
        NamespaceBackend::Postgres { pool, schema } => {
            let mut tx = pool.begin().await?;
            set_postgres_search_path(&mut tx, schema).await?;
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM _cowboy_import WHERE name = $1")
                    .bind(CORE_IMPORT)
                    .fetch_one(&mut *tx)
                    .await?;
            tx.commit().await.ok();
            count
        }
        NamespaceBackend::Sqlite(pool) => {
            sqlx::query_scalar("SELECT COUNT(*) FROM _cowboy_import WHERE name = ?1")
                .bind(CORE_IMPORT)
                .fetch_one(pool)
                .await?
        }
    };
    Ok(count > 0)
}

async fn mark_import(ns: &PluginNamespace) -> Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    match &ns.backend {
        NamespaceBackend::Postgres { pool, schema } => {
            let mut tx = pool.begin().await?;
            set_postgres_search_path(&mut tx, schema).await?;
            sqlx::query("INSERT INTO _cowboy_import (name, applied_at_ms) VALUES ($1, $2)")
                .bind(CORE_IMPORT)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        NamespaceBackend::Sqlite(pool) => {
            sqlx::query("INSERT INTO _cowboy_import (name, applied_at_ms) VALUES (?1, ?2)")
                .bind(CORE_IMPORT)
                .bind(now)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

async fn list_passkeys(
    ns: &PluginNamespace,
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
    ns: &PluginNamespace,
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
    ns: &PluginNamespace,
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
    ns: &PluginNamespace,
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
    ns: &PluginNamespace,
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
mod tests {
    use super::*;
    use crate::plugin_dir::PluginDir;
    use crate::plugin_host::PluginHostSpec;
    use crate::plugin_runtime::PluginRuntime;
    use crate::plugin_storage::PluginStorage;

    #[tokio::test]
    async fn sqlite_passkey_plugin_round_trip_and_import() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-plugin-passkeys-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::connect(
            &format!("sqlite://{}", root.join("core.sqlite3").display()),
            root.join("artifacts"),
        )
        .await
        .unwrap();
        store.migrate().await.unwrap();
        let user = crate::store::ProductUser {
            id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            username: "owner".to_owned(),
            password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
            password_hash: "hash".to_owned(),
            created_at_ms: 1,
            updated_at_ms: 1,
            disabled_at_ms: None,
        };
        store.insert_user(&user).await.unwrap();
        let core_passkey = UserPasskey {
            id: "pk1".to_owned(),
            user_id: user.id.clone(),
            credential_id: "cred-1".to_owned(),
            nickname: "laptop".to_owned(),
            passkey_json: "{\"ok\":true}".to_owned(),
            created_at_ms: 10,
            last_used_at_ms: None,
        };
        store.insert_user_passkey(&core_passkey).await.unwrap();

        let dir = PluginDir::open(&root).unwrap();
        let storage = PluginStorage::sqlite_files(dir);
        let spec = PluginHostSpec::from_json(
            include_str!("../examples/authentication/passkey/host.json").as_bytes(),
        )
        .unwrap();
        let ns = storage
            .migrate_plugin("passkey", spec.storage.as_ref().unwrap())
            .await
            .unwrap();
        import_from_core(&ns, &store).await.unwrap();
        import_from_core(&ns, &store).await.unwrap();
        let listed = list_user(&ns, &user.id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].nickname, "laptop");
        assert_eq!(count_user(&ns, &user.id).await.unwrap(), 1);

        let runtime_dir = PluginDir::open(&root.join("runtime")).unwrap();
        let runtime_storage = store.plugin_storage(runtime_dir);
        let runtime = PluginRuntime::activate(&runtime_storage, Some(&store), None)
            .await
            .unwrap();
        assert!(runtime.hosts.iter().any(|host| host.id == "password"));
        assert!(runtime.passkey.is_some());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
