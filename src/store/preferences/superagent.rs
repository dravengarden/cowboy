//! Supervision audit uses the existing durable Agent API operation ledger.
//! There is no second task, queue, retry, or event store.
use crate::store::{StorageBackend, Store};
use anyhow::Result;
use serde_json::{Value, json};

impl Store {
    pub async fn superagent_owners(&self) -> Result<Vec<String>> {
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_scalar(
                    "SELECT owner_user_id FROM user_preferences WHERE scope='superagent'",
                )
                .fetch_all(&$db.pool)
                .await?
            };
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        })
    }

    /// Bounded, newest first; pending means unknown and must never be replayed.
    pub async fn superagent_activity(&self, owner: &str, before: i64) -> Result<Vec<Value>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_as::<_, (String, String, Option<String>, i64)>(
                "SELECT parent_session_id,operation_id,result,created_at_ms FROM agent_api_operations \
                 WHERE owner_user_id=$1 AND operation_id LIKE 'sa-%' AND created_at_ms<$2 \
                 ORDER BY created_at_ms DESC,operation_id DESC LIMIT 100")
                .bind(owner).bind(before).fetch_all(&$db.pool).await?
        }; }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        rows.into_iter()
            .map(|(session, operation, result, at)| {
                Ok(json!({
                    "session_id":session,"operation":operation,"created_at_ms":at,
                    "result":result.map(|v| serde_json::from_str::<Value>(&v)).transpose()?,
                }))
            })
            .collect()
    }
}
