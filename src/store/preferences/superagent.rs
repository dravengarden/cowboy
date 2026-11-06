//! Supervision audit uses the existing durable Agent API operation ledger.
//! There is no second task, queue, retry, or event store.
use crate::store::{StorageBackend, Store};
use anyhow::Result;
use serde_json::{Value, json};

#[cfg(test)]
#[path = "superagent_tests.rs"]
mod tests;

impl Store {
    /// Project bindings only: routine MCP authorization never loads Markdown.
    pub async fn superagent_projects(&self, owner: &str) -> Result<Value> {
        let value: Option<Option<String>> = match &self.backend {
            StorageBackend::Postgres(db) => sqlx::query_scalar("SELECT (value::jsonb #> '{values,configuration,projects}')::text FROM user_preferences WHERE owner_user_id=$1 AND scope='superagent'")
                .bind(owner).fetch_optional(&db.pool).await?,
            StorageBackend::Sqlite(db) => sqlx::query_scalar("SELECT json_extract(value, '$.values.configuration.projects') FROM user_preferences WHERE owner_user_id=$1 AND scope='superagent'")
                .bind(owner).fetch_optional(&db.pool).await?,
        };
        match value {
            None => Ok(json!({})),
            Some(Some(value)) => Ok(serde_json::from_str(&value)?),
            Some(None) => anyhow::bail!("invalid superagent bindings"),
        }
    }
    /// Reopen only a proven zero-send reservation. Unknown and handed-off work
    /// are never eligible. The exact old receipt is the concurrency fence.
    pub async fn retry_unsent_superagent_operation(
        &self,
        owner: &str,
        parent: &str,
        operation: &str,
        previous: &Value,
        hash: &str,
    ) -> Result<bool> {
        anyhow::ensure!(
            operation.starts_with("sa-") && previous["status"] == "not_dispatched",
            "not an unsent supervision reservation"
        );
        let encoded = serde_json::to_string(previous)?;
        macro_rules! retry { ($db:expr) => {
            sqlx::query("UPDATE agent_api_operations SET result=NULL,request_hash=$5 WHERE owner_user_id=$1 AND parent_session_id=$2 AND operation_id=$3 AND result=$4")
                .bind(owner).bind(parent).bind(operation).bind(&encoded).bind(hash)
                .execute(&$db.pool).await?.rows_affected() == 1
        }; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => retry!(db),
            StorageBackend::Sqlite(db) => retry!(db),
        })
    }
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
    pub async fn superagent_activity(
        &self,
        owner: &str,
        before: &(i64, String, String),
    ) -> Result<Vec<Value>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_as::<_, (String, String, Option<String>, i64)>(
                "SELECT parent_session_id,operation_id,result,created_at_ms FROM agent_api_operations \
                 WHERE owner_user_id=$1 AND operation_id LIKE 'sa-%' AND (created_at_ms,operation_id,parent_session_id)<($2,$3,$4) \
                 ORDER BY created_at_ms DESC,operation_id DESC,parent_session_id DESC LIMIT 100")
                .bind(owner).bind(before.0).bind(&before.1).bind(&before.2).fetch_all(&$db.pool).await?
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
