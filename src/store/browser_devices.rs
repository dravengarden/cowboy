//! Device binding accompanies a credential; it never grants account authority.

use anyhow::Result;

use super::{StorageBackend, Store};

impl Store {
    pub(crate) async fn browser_device_key(
        &self,
        cookie: &str,
        token_hash: &str,
        now_ms: i64,
    ) -> Result<Option<String>> {
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_scalar(
                    "SELECT public_key FROM browser_device_bindings \
                     WHERE cookie_name = $1 AND token_hash = $2 AND expires_at_ms > $3",
                )
                .bind(cookie)
                .bind(token_hash)
                .bind(now_ms)
                .fetch_optional(&$db.pool)
                .await?
            };
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        })
    }

    pub(crate) async fn bind_browser_device(
        &self,
        cookie: &str,
        token_hash: &str,
        public_key: &str,
        now_ms: i64,
        expires_at_ms: i64,
    ) -> Result<()> {
        macro_rules! bind {
            ($db:expr) => {{
                let mut tx = $db.pool.begin().await?;
                sqlx::query("DELETE FROM browser_device_bindings WHERE expires_at_ms <= $1")
                    .bind(now_ms)
                    .execute(&mut *tx)
                    .await?;
                // A credential can never be rebound to a different device.
                sqlx::query(
                    "INSERT INTO browser_device_bindings \
                     (cookie_name, token_hash, public_key, expires_at_ms) \
                     VALUES ($1, $2, $3, $4)",
                )
                .bind(cookie)
                .bind(token_hash)
                .bind(public_key)
                .bind(expires_at_ms)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
            }};
        }
        match &self.backend {
            StorageBackend::Postgres(db) => bind!(db),
            StorageBackend::Sqlite(db) => bind!(db),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn binding_contract(store: &Store) {
        store.migrate().await.unwrap();
        store
            .bind_browser_device("cowboy_user", "token", "key-a", 100, 500)
            .await
            .unwrap();
        assert_eq!(
            store
                .browser_device_key("cowboy_user", "token", 499)
                .await
                .unwrap()
                .as_deref(),
            Some("key-a")
        );
        assert_eq!(
            store
                .browser_device_key("cowboy_admin", "token", 499)
                .await
                .unwrap(),
            None
        );
        assert!(
            store
                .bind_browser_device("cowboy_user", "token", "key-b", 200, 900)
                .await
                .is_err()
        );
        assert_eq!(
            store
                .browser_device_key("cowboy_user", "token", 499)
                .await
                .unwrap()
                .as_deref(),
            Some("key-a")
        );
        assert_eq!(
            store
                .browser_device_key("cowboy_user", "token", 500)
                .await
                .unwrap(),
            None
        );
        store
            .bind_browser_device("cowboy_user", "rotated-token", "key-a", 500, 900)
            .await
            .unwrap();
        assert_eq!(
            store
                .browser_device_key("cowboy_user", "token", 499)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn sqlite_binding_is_immutable_scoped_and_expires() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        binding_contract(&store).await;
    }

    #[tokio::test]
    #[ignore = "run with just test-postgres in a disposable database"]
    async fn postgres_binding_is_immutable_scoped_and_expires() {
        let root = tempfile::tempdir().unwrap();
        let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
        let store = Store::connect(&url, root.path().join("artifacts"))
            .await
            .unwrap();
        binding_contract(&store).await;
    }
}
