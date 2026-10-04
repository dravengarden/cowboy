//! The same immutable reservation contract for both supported databases.
use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use anyhow::{Result, ensure};
use cardea_core::authorization::{Plan, Receipt};

#[derive(sqlx::FromRow)]
pub(crate) struct CardeaOperation {
    pub operation_id: String,
    pub request_digest: String,
    pub plan: String,
    pub claim_id: Option<String>,
    pub state: String,
    pub receipt: Option<String>,
}
impl CardeaOperation {
    pub(crate) fn plan(&self) -> Result<Plan> {
        let plan: Plan = cardea_core::authorization::decode(self.plan.as_bytes())
            .map_err(|_| anyhow::anyhow!("invalid saved plan"))?;
        ensure!(
            plan.operation_id == self.operation_id,
            "saved plan binding changed"
        );
        Ok(plan)
    }
}
macro_rules! dispatch {
    ($store:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match &$store.backend {
            StorageBackend::Postgres(db) => db.$method($($arg),*).await,
            StorageBackend::Sqlite(db) => db.$method($($arg),*).await,
        }
    };
}
impl Store {
    pub(crate) async fn cardea_device_allowed(&self, user: &str, device: &str) -> Result<bool> {
        dispatch!(self, cardea_device_allowed(user, device))
    }
    pub(crate) async fn cardea_operation(&self, id: &str) -> Result<Option<CardeaOperation>> {
        dispatch!(self, cardea_operation(id))
    }
    pub(crate) async fn cardea_request(&self, key: &str) -> Result<Option<CardeaOperation>> {
        dispatch!(self, cardea_request(key))
    }
    pub(crate) async fn prepare_cardea_operation(
        &self,
        key: &str,
        binding: &str,
        plan: &Plan,
    ) -> Result<()> {
        let document = serde_json::to_string(plan)?;
        ensure!(
            document.len() <= cardea_core::authorization::MAX_DOCUMENT_BYTES,
            "saved plan too large"
        );
        dispatch!(
            self,
            prepare_cardea_operation(key, binding, &plan.operation_id, &document)
        )
    }
    pub(crate) async fn claim_cardea_operation(
        &self,
        id: &str,
        claim: &str,
    ) -> Result<CardeaOperation> {
        dispatch!(self, claim_cardea_operation(id, claim))
    }
    pub(crate) async fn start_cardea_operation(&self, id: &str, claim: &str) -> Result<bool> {
        dispatch!(self, start_cardea_operation(id, claim))
    }
    pub(crate) async fn record_cardea_operation(&self, receipt: &Receipt) -> Result<()> {
        let document = serde_json::to_string(receipt)?;
        ensure!(document.len() <= 4096, "receipt too large");
        dispatch!(
            self,
            record_cardea_operation(&receipt.operation_id, &receipt.claim_id, &document)
        )
    }
}
macro_rules! implementation {
    ($db:ty, $revoked:literal) => {
        impl $db {
            async fn cardea_device_allowed(&self, user: &str, device: &str) -> Result<bool> {
                Ok(sqlx::query_scalar(concat!("SELECT NOT EXISTS (SELECT 1 FROM user_devices WHERE id = $1 AND (user_id <> $2 OR ", $revoked, " IS NOT NULL))")).bind(device).bind(user).fetch_one(&self.pool).await?)
            }
            async fn cardea_operation(&self, id: &str) -> Result<Option<CardeaOperation>> {
                Ok(sqlx::query_as("SELECT operation_id, request_digest, plan, claim_id, state, receipt FROM cardea_operations WHERE operation_id = $1").bind(id).fetch_optional(&self.pool).await?)
            }
            async fn cardea_request(&self, key: &str) -> Result<Option<CardeaOperation>> {
                Ok(sqlx::query_as("SELECT operation_id, request_digest, plan, claim_id, state, receipt FROM cardea_operations WHERE request_key = $1").bind(key).fetch_optional(&self.pool).await?)
            }
            async fn prepare_cardea_operation(&self, key: &str, binding: &str, id: &str, plan: &str) -> Result<()> {
                let mut transaction = self.pool.begin().await?;
                let admitted = sqlx::query("UPDATE cardea_operation_capacity SET retained = retained + 1 WHERE singleton = 1 AND retained < 4096").execute(&mut *transaction).await?.rows_affected();
                ensure!(admitted == 1, "operation capacity exhausted");
                sqlx::query("INSERT INTO cardea_operations(operation_id, request_key, request_digest, plan, state) VALUES ($1, $2, $3, $4, 'prepared')").bind(id).bind(key).bind(binding).bind(plan).execute(&mut *transaction).await?;
                transaction.commit().await?;
                Ok(())
            }
            async fn claim_cardea_operation(&self, id: &str, claim: &str) -> Result<CardeaOperation> {
                sqlx::query("UPDATE cardea_operations SET claim_id = $1 WHERE operation_id = $2 AND claim_id IS NULL AND state = 'prepared'").bind(claim).bind(id).execute(&self.pool).await?;
                let op = self.cardea_operation(id).await?.ok_or_else(|| anyhow::anyhow!("operation missing"))?;
                ensure!(op.claim_id.as_deref() == Some(claim), "different claim owns operation");
                Ok(op)
            }
            async fn start_cardea_operation(&self, id: &str, claim: &str) -> Result<bool> {
                Ok(sqlx::query("UPDATE cardea_operations SET state = 'executing' WHERE operation_id = $1 AND claim_id = $2 AND state = 'prepared'").bind(id).bind(claim).execute(&self.pool).await?.rows_affected() == 1)
            }
            async fn record_cardea_operation(&self, id: &str, claim: &str, document: &str) -> Result<()> {
                sqlx::query("UPDATE cardea_operations SET receipt = $1, state = 'completed' WHERE operation_id = $2 AND claim_id = $3 AND state = 'executing' AND receipt IS NULL").bind(document).bind(id).bind(claim).execute(&self.pool).await?;
                let saved = self.cardea_operation(id).await?.ok_or_else(|| anyhow::anyhow!("operation missing"))?;
                ensure!(saved.receipt.as_deref() == Some(document), "receipt conflict");
                Ok(())
            }
        }
    };
}
implementation!(PostgresStorage, "revoked_at");
implementation!(SqliteStorage, "revoked_at_ms");

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn plan() -> Plan {
        serde_json::from_value(json!({"schema":"dravengarden.cardea.operation/v1","operation_id":"A".repeat(43),"application_id":"cowboy-production","action":"plugin.install","catalog_digest":"fixture","policy_revision":"v1","subject_id":"draven","grant_id":"11111111-1111-4111-8111-111111111111","binding_key":"A".repeat(43),"resources":[{"kind":"machine-plugin","id":"test","scope":"cowboy","expected_revision":"v1"}],"input":{},"review":{"title":"Fixture","summary":"Storage fixture","facts":[]},"created_at":1,"review_until":2,"execute_until":3})).unwrap()
    }
    async fn contract(url: &str, root: &std::path::Path) {
        let store = Store::connect(url, root.join("artifacts")).await.unwrap();
        store.migrate().await.unwrap();
        device_contract(&store).await;
        let p = plan();
        store
            .prepare_cardea_operation("key", "binding", &p)
            .await
            .unwrap();
        assert!(
            store
                .prepare_cardea_operation("key", "different", &p)
                .await
                .is_err()
        );
        let saved = store.cardea_request("key").await.unwrap().unwrap();
        assert_eq!(saved.plan().unwrap(), p);
        assert_eq!(saved.request_digest, "binding");
        let a = store.clone();
        let b = store.clone();
        let id = p.operation_id.clone();
        let (one, two) = tokio::join!(
            a.claim_cardea_operation(&id, "claim-a"),
            b.claim_cardea_operation(&id, "claim-b")
        );
        assert_ne!(one.is_ok(), two.is_ok());
        let winner = one.ok().or_else(|| two.ok()).unwrap().claim_id.unwrap();
        let (one, two) = tokio::join!(
            a.start_cardea_operation(&id, &winner),
            b.start_cardea_operation(&id, &winner)
        );
        assert_ne!(one.unwrap(), two.unwrap());
        // Re-open the same durable database after a lost/cancelled response.
        let reopened = Store::connect(url, root.join("artifacts")).await.unwrap();
        let saved = reopened.cardea_operation(&id).await.unwrap().unwrap();
        assert_eq!(saved.state, "executing");
        assert_eq!(saved.claim_id.as_deref(), Some(winner.as_str()));
        assert!(!reopened.start_cardea_operation(&id, &winner).await.unwrap());
        let receipt = Receipt {
            operation_id: id.clone(),
            plan_digest: p.digest(),
            claim_id: winner,
            outcome: cardea_core::authorization::Status::Succeeded,
            finished_at: 10,
            summary: "Installed".into(),
        };
        reopened.record_cardea_operation(&receipt).await.unwrap();
        reopened.record_cardea_operation(&receipt).await.unwrap();
        let mut changed = receipt.clone();
        changed.summary = "Substituted".into();
        assert!(reopened.record_cardea_operation(&changed).await.is_err());
        assert_eq!(
            reopened
                .cardea_operation(&id)
                .await
                .unwrap()
                .unwrap()
                .receipt
                .as_deref(),
            Some(serde_json::to_string(&receipt).unwrap().as_str())
        );
        fill_capacity(&store).await;
        let mut next = p;
        next.operation_id = "Q".repeat(43);
        assert!(
            store
                .prepare_cardea_operation("next", "binding", &next)
                .await
                .is_err()
        );
        assert!(store.cardea_request("next").await.unwrap().is_none());
    }
    async fn device_contract(store: &Store) {
        let user = super::super::ProductUser {
            id: "c".repeat(32),
            username: "draven".into(),
            password_algo: "fixture".into(),
            password_hash: "unused".into(),
            created_at_ms: 1000,
            updated_at_ms: 1000,
            disabled_at_ms: None,
        };
        store.insert_user(&user).await.unwrap();
        let device = super::super::ProductDevice {
            id: "d".repeat(32),
            user_id: user.id.clone(),
            name: "Fixture".into(),
            public_key: "A".repeat(43),
            created_at_ms: 1000,
            last_used_at_ms: None,
            revoked_at_ms: None,
        };
        let refresh = super::super::ProductDeviceRefreshToken {
            token_hash: "test-only".into(),
            device_id: device.id.clone(),
            family_id: device.id.clone(),
            created_at_ms: 1000,
            expires_at_ms: 2000,
            used_at_ms: None,
            revoked_at_ms: None,
        };
        assert!(
            store
                .cardea_device_allowed(&user.id, &device.id)
                .await
                .unwrap()
        );
        store
            .admit_user_device(&device, &refresh, 8, true)
            .await
            .unwrap();
        assert!(
            store
                .cardea_device_allowed(&user.id, &device.id)
                .await
                .unwrap()
        );
        assert!(
            !store
                .cardea_device_allowed("another-user", &device.id)
                .await
                .unwrap()
        );
        store.revoke_user_device(&device.id, 1500).await.unwrap();
        assert!(
            !store
                .cardea_device_allowed(&user.id, &device.id)
                .await
                .unwrap()
        );
    }
    async fn fill_capacity(store: &Store) {
        let retained = match &store.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query_scalar::<_, i32>("SELECT retained FROM cardea_operation_capacity")
                    .fetch_one(&db.pool)
                    .await
                    .unwrap()
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query_scalar::<_, i32>("SELECT retained FROM cardea_operation_capacity")
                    .fetch_one(&db.pool)
                    .await
                    .unwrap()
            }
        };
        assert_eq!(
            retained, 1,
            "losing duplicate transactions must roll back capacity"
        );
        match &store.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query("UPDATE cardea_operation_capacity SET retained=4096")
                    .execute(&db.pool)
                    .await
                    .unwrap();
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query("UPDATE cardea_operation_capacity SET retained=4096")
                    .execute(&db.pool)
                    .await
                    .unwrap();
            }
        }
    }
    #[tokio::test]
    async fn sqlite_cardea_reservations_survive_restart_and_cannot_replay() {
        let root = tempfile::tempdir().unwrap();
        contract(
            &format!("sqlite://{}", root.path().join("store.sqlite").display()),
            root.path(),
        )
        .await;
    }
    #[tokio::test]
    #[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
    async fn postgres_cardea_reservations_survive_restart_and_cannot_replay() {
        let root = tempfile::tempdir().unwrap();
        let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database required");
        contract(&url, root.path()).await;
    }
}
