//! Durable submission receipts, not a second domain transaction manager.
use super::{StorageBackend, Store};
use anyhow::Result;
use serde_json::Value;

#[derive(Debug, PartialEq)]
pub(crate) enum Admission {
    New,
    Existing(Option<Value>),
    Conflict,
}

impl Store {
    pub(crate) async fn admit_agent_operation(
        &self,
        owner: &str,
        parent: &str,
        operation: &str,
        request_hash: &str,
    ) -> Result<Admission> {
        macro_rules! admit { ($db:expr) => {{
            let inserted = sqlx::query("INSERT INTO agent_api_operations (owner_user_id,parent_session_id,operation_id,request_hash,created_at_ms) VALUES ($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING")
                .bind(owner).bind(parent).bind(operation).bind(request_hash)
                .bind(chrono::Utc::now().timestamp_millis()).execute(&$db.pool).await?.rows_affected();
            if inserted == 1 { Admission::New } else {
                let (hash, result): (String, Option<String>) = sqlx::query_as("SELECT request_hash,result FROM agent_api_operations WHERE owner_user_id=$1 AND parent_session_id=$2 AND operation_id=$3")
                    .bind(owner).bind(parent).bind(operation).fetch_one(&$db.pool).await?;
                if hash == request_hash { Admission::Existing(result.map(|s| serde_json::from_str(&s)).transpose()?) }
                else { Admission::Conflict }
            }
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => admit!(db),
            StorageBackend::Sqlite(db) => admit!(db),
        })
    }

    pub(crate) async fn finish_agent_operation(
        &self,
        owner: &str,
        parent: &str,
        operation: &str,
        result: &Value,
    ) -> Result<()> {
        let encoded = serde_json::to_string(result)?;
        macro_rules! finish { ($db:expr) => {
            sqlx::query("UPDATE agent_api_operations SET result=$4 WHERE owner_user_id=$1 AND parent_session_id=$2 AND operation_id=$3 AND result IS NULL")
                .bind(owner).bind(parent).bind(operation).bind(&encoded).execute(&$db.pool).await?.rows_affected()
        }; }
        let affected = match &self.backend {
            StorageBackend::Postgres(db) => finish!(db),
            StorageBackend::Sqlite(db) => finish!(db),
        };
        anyhow::ensure!(affected == 1, "Agent operation was not pending");
        Ok(())
    }

    /// None: never observed. Some(None): admitted, outcome still uncertain.
    pub(crate) async fn agent_operation_receipt(
        &self,
        owner: &str,
        parent: &str,
        operation: &str,
    ) -> Result<Option<Option<Value>>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_scalar::<_, Option<String>>("SELECT result FROM agent_api_operations WHERE owner_user_id=$1 AND parent_session_id=$2 AND operation_id=$3")
                .bind(owner).bind(parent).bind(operation).fetch_optional(&$db.pool).await?
        }; }
        let result = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        result
            .map(|s| s.map(|s| serde_json::from_str(&s)).transpose())
            .transpose()
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn concurrent_submission_crash_and_receipt_are_owner_and_parent_scoped() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", directory.path().to_owned())
            .await
            .unwrap();
        store.migrate().await.unwrap();
        exercise(&store).await;
    }

    #[tokio::test]
    #[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
    async fn postgres_agent_operation_admission_and_receipts() {
        let directory = tempfile::tempdir().unwrap();
        let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
        let store = Store::connect(&url, directory.path().to_owned())
            .await
            .unwrap();
        store.migrate().await.unwrap();
        exercise(&store).await;
    }

    async fn exercise(store: &Store) {
        let (a, b) = tokio::join!(
            store.admit_agent_operation("owner", "parent", "op", "hash"),
            store.admit_agent_operation("owner", "parent", "op", "hash")
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert!(matches!(
            (&a, &b),
            (Admission::New, Admission::Existing(None))
                | (Admission::Existing(None), Admission::New)
        ));
        assert_eq!(
            store
                .admit_agent_operation("owner", "parent", "op", "changed")
                .await
                .unwrap(),
            Admission::Conflict
        );
        assert_eq!(
            store
                .agent_operation_receipt("foreign", "parent", "op")
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            store
                .agent_operation_receipt("owner", "other", "op")
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            store
                .agent_operation_receipt("owner", "parent", "op")
                .await
                .unwrap(),
            Some(None)
        );
        let receipt = json!({"status":"accepted"});
        store
            .finish_agent_operation("owner", "parent", "op", &receipt)
            .await
            .unwrap();
        assert_eq!(
            store
                .admit_agent_operation("owner", "parent", "op", "hash")
                .await
                .unwrap(),
            Admission::Existing(Some(receipt.clone()))
        );
        assert_eq!(
            store
                .agent_operation_receipt("owner", "parent", "op")
                .await
                .unwrap(),
            Some(Some(receipt))
        );
    }

    #[tokio::test]
    async fn reopened_store_keeps_pending_uncertain_and_completed_non_replayable() {
        let directory = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}?mode=rwc",
            directory.path().join("receipts.sqlite").display()
        );
        let store = Store::connect(&url, directory.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        for op in ["pending", "complete"] {
            assert_eq!(
                store
                    .admit_agent_operation("owner", "parent", op, "hash")
                    .await
                    .unwrap(),
                Admission::New
            );
        }
        store
            .finish_agent_operation(
                "owner",
                "parent",
                "complete",
                &json!({"status":"accepted","domain_operation":"domain-1"}),
            )
            .await
            .unwrap();
        if let StorageBackend::Sqlite(db) = &store.backend {
            db.pool.close().await;
        }
        drop(store);
        let store = Store::connect(&url, directory.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        assert_eq!(
            store
                .admit_agent_operation("owner", "parent", "pending", "hash")
                .await
                .unwrap(),
            Admission::Existing(None)
        );
        assert_eq!(
            store
                .agent_operation_receipt("owner", "parent", "pending")
                .await
                .unwrap(),
            Some(None)
        );
        assert!(
            matches!(store.admit_agent_operation("owner", "parent", "complete", "hash").await.unwrap(), Admission::Existing(Some(value)) if value["domain_operation"] == "domain-1")
        );
        assert_eq!(
            store
                .admit_agent_operation("owner", "parent", "pending", "changed")
                .await
                .unwrap(),
            Admission::Conflict
        );
    }
}
