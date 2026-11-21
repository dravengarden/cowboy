//! Owner-scoped reads of the reset action log. Service collector lanes are
//! Service-wide; a named account's lane (`account-<id>`) is visible only to
//! its owner. The filter runs in SQL, before the row limit, so another
//! owner's rows can never displace or reveal the caller's.
use super::{ProviderActionLog, StorageBackend, Store};
use anyhow::{Context as _, Result};

type Row = (
    i64,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
);

const COLUMNS: &str = "SELECT id, provider, action, trigger, status, phase, message, credit_id, \
     idempotency_suffix, created_at_ms FROM provider_action_logs";

fn into_log(row: Row) -> ProviderActionLog {
    ProviderActionLog {
        id: row.0,
        provider: row.1,
        action: row.2,
        trigger: row.3,
        status: row.4,
        phase: row.5,
        message: row.6,
        credit_id: row.7,
        idempotency_suffix: row.8,
        created_at_ms: row.9,
    }
}

impl super::sqlite::SqliteStorage {
    /// The newest Service-lane rows only, for the Service-wide diagnostics
    /// view; named-account lanes belong to their owners' usage logs.
    pub(super) async fn service_action_logs(&self) -> Result<Vec<ProviderActionLog>> {
        let rows: Vec<Row> = sqlx::query_as(&format!(
            "{COLUMNS} WHERE provider NOT LIKE 'account-%' \
             ORDER BY created_at_ms DESC, id DESC LIMIT 200"
        ))
        .fetch_all(&self.pool)
        .await
        .context("list Service provider action logs")?;
        Ok(rows.into_iter().map(into_log).collect())
    }
}

impl Store {
    /// The newest `limit` rows of Service lanes and of `lanes` (the caller's
    /// own named-account lanes).
    pub async fn visible_provider_action_logs(
        &self,
        limit: i64,
        lanes: &[String],
    ) -> Result<Vec<ProviderActionLog>> {
        let lanes = serde_json::to_string(lanes)?;
        let limit = limit.clamp(1, 200);
        let rows: Vec<Row> = match &self.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query_as(&format!(
                    "{COLUMNS} WHERE provider NOT LIKE 'account-%' \
                     OR provider IN (SELECT jsonb_array_elements_text($1::jsonb)) \
                     ORDER BY created_at_ms DESC, id DESC LIMIT $2"
                ))
                .bind(&lanes)
                .bind(limit)
                .fetch_all(&db.pool)
                .await
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query_as(&format!(
                    "{COLUMNS} WHERE provider NOT LIKE 'account-%' \
                     OR provider IN (SELECT value FROM json_each(?1)) \
                     ORDER BY created_at_ms DESC, id DESC LIMIT ?2"
                ))
                .bind(&lanes)
                .bind(limit)
                .fetch_all(&db.pool)
                .await
            }
        }
        .context("list visible provider action logs")?;
        Ok(rows.into_iter().map(into_log).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn another_owners_rows_never_fill_or_reveal_the_limit() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let mine = "account-11111111-1111-4111-8111-111111111111".to_owned();
        let theirs = "account-22222222-2222-4222-8222-222222222222";
        let append = |lane: &'static str, at: i64| {
            let store = store.clone();
            async move {
                store
                    .append_provider_action_log(
                        lane,
                        "manual",
                        "succeeded",
                        "p",
                        "m",
                        None,
                        None,
                        at,
                    )
                    .await
                    .unwrap();
            }
        };
        append("anthropic", 1).await;
        store
            .append_provider_action_log(&mine, "manual", "succeeded", "p", "m", None, None, 2)
            .await
            .unwrap();
        for at in 3..10 {
            append(theirs, at).await;
        }
        let visible = store
            .visible_provider_action_logs(2, std::slice::from_ref(&mine))
            .await
            .unwrap();
        let lanes: Vec<_> = visible.iter().map(|log| log.provider.as_str()).collect();
        assert_eq!(lanes, [mine.as_str(), "anthropic"]);
        let stranger = store.visible_provider_action_logs(10, &[]).await.unwrap();
        assert_eq!(stranger.len(), 1);
        assert_eq!(stranger[0].provider, "anthropic");
    }
}
