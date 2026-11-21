//! Notification intents, deliveries, subscriptions and acknowledgements
//! (docs/task-notifications.md).

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use crate::task_notifications::{Category, Occurrence};
use anyhow::{Context as _, Result};

macro_rules! dispatch {
    ($store:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match &$store.backend {
            StorageBackend::Postgres(db) => db.$method($($arg),*).await,
            StorageBackend::Sqlite(db) => db.$method($($arg),*).await,
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub dedupe_key: String,
    pub owner_user_id: String,
    pub session_id: String,
    pub category: Category,
    pub destination: String,
    pub marker: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub intent: Intent,
    pub endpoint_sha256: String,
    pub attempts: i64,
}

#[derive(Debug, Clone)]
pub struct StoredSubscription {
    pub endpoint_sha256: String,
    pub endpoint: String,
    pub owner_user_id: String,
    pub p256dh: String,
    pub auth: String,
    pub preferences: String,
}

type IntentRow = (String, String, String, String, String, String, i64);

fn intent(row: IntentRow) -> Option<Intent> {
    let (dedupe_key, owner_user_id, session_id, category, destination, marker, expires_at_ms) = row;
    Some(Intent {
        dedupe_key,
        owner_user_id,
        session_id,
        category: Category::parse(&category)?,
        destination,
        marker,
        expires_at_ms,
    })
}

pub(super) const INSERT_INTENT: &str = "INSERT INTO notification_intents(dedupe_key, owner_user_id, session_id, \
     category, destination, marker, due_at_ms, expires_at_ms, created_at_ms) \
     SELECT $1, s.owner_user_id, s.id, $2, $3, $4, $5, $6, $7 FROM sessions s \
     WHERE s.id = $8 AND s.owner_user_id IS NOT NULL AND s.system = FALSE \
     ON CONFLICT(dedupe_key) DO NOTHING";

impl Store {
    /// Record an occurrence once. A system or unowned session writes nothing.
    pub async fn record_occurrence(&self, occurrence: &Occurrence, now_ms: i64) -> Result<()> {
        dispatch!(self, record_occurrence(occurrence, now_ms))
    }

    pub async fn due_intents(&self, now_ms: i64, limit: i64) -> Result<Vec<Intent>> {
        dispatch!(self, due_intents(now_ms, limit))
    }

    pub async fn finish_intent(&self, dedupe_key: &str, state: &str) -> Result<()> {
        dispatch!(self, finish_intent(dedupe_key, state))
    }

    /// Freeze an intent's audience, even an empty one, in one transaction.
    pub async fn freeze_audience(
        &self,
        dedupe_key: &str,
        endpoints: &[String],
        now_ms: i64,
    ) -> Result<()> {
        dispatch!(self, freeze_audience(dedupe_key, endpoints, now_ms))
    }

    pub async fn due_deliveries(&self, now_ms: i64, limit: i64) -> Result<Vec<Delivery>> {
        dispatch!(self, due_deliveries(now_ms, limit))
    }

    pub async fn finish_delivery(
        &self,
        dedupe_key: &str,
        endpoint_sha256: &str,
        state: &str,
    ) -> Result<()> {
        dispatch!(self, finish_delivery(dedupe_key, endpoint_sha256, state))
    }

    pub async fn retry_delivery(
        &self,
        dedupe_key: &str,
        endpoint_sha256: &str,
        attempts: i64,
        next_attempt_at_ms: i64,
    ) -> Result<()> {
        dispatch!(
            self,
            retry_delivery(dedupe_key, endpoint_sha256, attempts, next_attempt_at_ms)
        )
    }

    pub async fn prune_notifications(&self, created_before_ms: i64) -> Result<()> {
        dispatch!(self, prune_notifications(created_before_ms))
    }

    /// Enroll an endpoint for an account, taking it over from any other.
    pub async fn upsert_web_push_subscription(
        &self,
        subscription: &StoredSubscription,
        enrolled_session_id: Option<&str>,
        now_ms: i64,
    ) -> Result<()> {
        dispatch!(
            self,
            upsert_web_push_subscription(subscription, enrolled_session_id, now_ms)
        )
    }

    pub async fn remove_web_push_subscription(&self, endpoint_sha256: &str) -> Result<()> {
        dispatch!(self, remove_web_push_subscription(endpoint_sha256))
    }

    /// Logout: what the ended cookie session enrolled stops being delivered.
    pub async fn remove_web_push_subscriptions_of_session(&self, session_id: &str) -> Result<()> {
        dispatch!(self, remove_web_push_subscriptions_of_session(session_id))
    }

    pub async fn web_push_subscriptions(&self, owner: &str) -> Result<Vec<StoredSubscription>> {
        dispatch!(self, web_push_subscriptions(owner))
    }

    pub async fn web_push_subscription(
        &self,
        endpoint_sha256: &str,
    ) -> Result<Option<StoredSubscription>> {
        dispatch!(self, web_push_subscription(endpoint_sha256))
    }

    pub async fn acknowledge_task(
        &self,
        owner: &str,
        session_id: &str,
        task_key: &str,
        marker: &str,
        now_ms: i64,
    ) -> Result<()> {
        dispatch!(
            self,
            acknowledge_task(owner, session_id, task_key, marker, now_ms)
        )
    }

    pub async fn task_acknowledged(
        &self,
        owner: &str,
        session_id: &str,
        task_key: &str,
        marker: &str,
    ) -> Result<bool> {
        dispatch!(self, task_acknowledged(owner, session_id, task_key, marker))
    }

    /// The account's acknowledgements in a session as `(task_key, marker)`.
    pub async fn task_acknowledgements(
        &self,
        owner: &str,
        session_id: &str,
    ) -> Result<Vec<(String, String)>> {
        dispatch!(self, task_acknowledgements(owner, session_id))
    }
}

type SubscriptionRow = (String, String, String, String, String, String);

fn subscription(row: SubscriptionRow) -> StoredSubscription {
    let (endpoint_sha256, endpoint, owner_user_id, p256dh, auth, preferences) = row;
    StoredSubscription {
        endpoint_sha256,
        endpoint,
        owner_user_id,
        p256dh,
        auth,
        preferences,
    }
}

const SUBSCRIPTION_COLUMNS: &str =
    "endpoint_sha256, endpoint, owner_user_id, p256dh, auth, preferences";

macro_rules! implementation {
    ($backend:ty) => {
        impl $backend {
            async fn record_occurrence(&self, occurrence: &Occurrence, now_ms: i64) -> Result<()> {
                let category = occurrence.category;
                sqlx::query(INSERT_INTENT)
                    .bind(&occurrence.dedupe_key)
                    .bind(category.code())
                    .bind(&occurrence.destination)
                    .bind(&occurrence.marker)
                    .bind(now_ms.saturating_add(category.delay_ms()))
                    .bind(now_ms.saturating_add(category.ttl_ms()))
                    .bind(now_ms)
                    .bind(&occurrence.session_id)
                    .execute(&self.pool)
                    .await
                    .context("INSERT notification intent")?;
                Ok(())
            }

            async fn due_intents(&self, now_ms: i64, limit: i64) -> Result<Vec<Intent>> {
                let rows: Vec<IntentRow> = sqlx::query_as(
                    "SELECT dedupe_key, owner_user_id, session_id, category, destination, marker, \
                     expires_at_ms FROM notification_intents \
                     WHERE state = 'pending' AND expanded_at_ms IS NULL AND due_at_ms <= $1 \
                     ORDER BY due_at_ms LIMIT $2",
                )
                .bind(now_ms)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
                .context("SELECT due notification intents")?;
                Ok(rows.into_iter().filter_map(intent).collect())
            }

            async fn finish_intent(&self, dedupe_key: &str, state: &str) -> Result<()> {
                sqlx::query("UPDATE notification_intents SET state = $1 WHERE dedupe_key = $2")
                    .bind(state)
                    .bind(dedupe_key)
                    .execute(&self.pool)
                    .await
                    .context("UPDATE notification intent state")?;
                Ok(())
            }

            async fn freeze_audience(
                &self,
                dedupe_key: &str,
                endpoints: &[String],
                now_ms: i64,
            ) -> Result<()> {
                let mut tx = self.pool.begin().await.context("begin audience freeze")?;
                for endpoint in endpoints {
                    sqlx::query(
                        "INSERT INTO notification_deliveries(dedupe_key, endpoint_sha256, next_attempt_at_ms) \
                         VALUES ($1, $2, $3) ON CONFLICT(dedupe_key, endpoint_sha256) DO NOTHING",
                    )
                    .bind(dedupe_key)
                    .bind(endpoint)
                    .bind(now_ms)
                    .execute(&mut *tx)
                    .await
                    .context("INSERT notification delivery")?;
                }
                sqlx::query("UPDATE notification_intents SET expanded_at_ms = $1 WHERE dedupe_key = $2")
                    .bind(now_ms)
                    .bind(dedupe_key)
                    .execute(&mut *tx)
                    .await
                    .context("UPDATE notification intent expansion")?;
                tx.commit().await.context("commit audience freeze")
            }

            async fn due_deliveries(&self, now_ms: i64, limit: i64) -> Result<Vec<Delivery>> {
                let rows: Vec<(String, String, String, String, String, String, i64, String, i64)> =
                    sqlx::query_as(
                        "SELECT i.dedupe_key, i.owner_user_id, i.session_id, i.category, i.destination, \
                         i.marker, i.expires_at_ms, d.endpoint_sha256, d.attempts \
                         FROM notification_deliveries d \
                         JOIN notification_intents i ON i.dedupe_key = d.dedupe_key \
                         WHERE d.state = 'pending' AND d.next_attempt_at_ms <= $1 \
                         ORDER BY d.next_attempt_at_ms LIMIT $2",
                    )
                    .bind(now_ms)
                    .bind(limit)
                    .fetch_all(&self.pool)
                    .await
                    .context("SELECT due notification deliveries")?;
                Ok(rows
                    .into_iter()
                    .filter_map(|(key, owner, session, category, destination, marker, expires, endpoint, attempts)| {
                        Some(Delivery {
                            intent: intent((key, owner, session, category, destination, marker, expires))?,
                            endpoint_sha256: endpoint,
                            attempts,
                        })
                    })
                    .collect())
            }

            async fn finish_delivery(
                &self,
                dedupe_key: &str,
                endpoint_sha256: &str,
                state: &str,
            ) -> Result<()> {
                sqlx::query(
                    "UPDATE notification_deliveries SET state = $1 \
                     WHERE dedupe_key = $2 AND endpoint_sha256 = $3",
                )
                .bind(state)
                .bind(dedupe_key)
                .bind(endpoint_sha256)
                .execute(&self.pool)
                .await
                .context("UPDATE notification delivery state")?;
                Ok(())
            }

            async fn retry_delivery(
                &self,
                dedupe_key: &str,
                endpoint_sha256: &str,
                attempts: i64,
                next_attempt_at_ms: i64,
            ) -> Result<()> {
                sqlx::query(
                    "UPDATE notification_deliveries SET attempts = $1, next_attempt_at_ms = $2 \
                     WHERE dedupe_key = $3 AND endpoint_sha256 = $4",
                )
                .bind(attempts)
                .bind(next_attempt_at_ms)
                .bind(dedupe_key)
                .bind(endpoint_sha256)
                .execute(&self.pool)
                .await
                .context("UPDATE notification delivery retry")?;
                Ok(())
            }

            async fn prune_notifications(&self, created_before_ms: i64) -> Result<()> {
                sqlx::query("DELETE FROM notification_intents WHERE created_at_ms < $1")
                    .bind(created_before_ms)
                    .execute(&self.pool)
                    .await
                    .context("DELETE old notification intents")?;
                Ok(())
            }

            async fn upsert_web_push_subscription(
                &self,
                subscription: &StoredSubscription,
                enrolled_session_id: Option<&str>,
                now_ms: i64,
            ) -> Result<()> {
                sqlx::query(
                    "INSERT INTO web_push_subscriptions(endpoint_sha256, endpoint, owner_user_id, \
                     enrolled_session_id, p256dh, auth, preferences, updated_at_ms) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                     ON CONFLICT(endpoint_sha256) DO UPDATE SET owner_user_id = EXCLUDED.owner_user_id, \
                     enrolled_session_id = EXCLUDED.enrolled_session_id, p256dh = EXCLUDED.p256dh, \
                     auth = EXCLUDED.auth, preferences = EXCLUDED.preferences, \
                     updated_at_ms = EXCLUDED.updated_at_ms",
                )
                .bind(&subscription.endpoint_sha256)
                .bind(&subscription.endpoint)
                .bind(&subscription.owner_user_id)
                .bind(enrolled_session_id)
                .bind(&subscription.p256dh)
                .bind(&subscription.auth)
                .bind(&subscription.preferences)
                .bind(now_ms)
                .execute(&self.pool)
                .await
                .context("UPSERT Web Push subscription")?;
                Ok(())
            }

            async fn remove_web_push_subscription(&self, endpoint_sha256: &str) -> Result<()> {
                sqlx::query("DELETE FROM web_push_subscriptions WHERE endpoint_sha256 = $1")
                    .bind(endpoint_sha256)
                    .execute(&self.pool)
                    .await
                    .context("DELETE Web Push subscription")?;
                Ok(())
            }

            async fn remove_web_push_subscriptions_of_session(&self, session_id: &str) -> Result<()> {
                sqlx::query("DELETE FROM web_push_subscriptions WHERE enrolled_session_id = $1")
                    .bind(session_id)
                    .execute(&self.pool)
                    .await
                    .context("DELETE Web Push subscriptions of a session")?;
                Ok(())
            }

            async fn web_push_subscriptions(&self, owner: &str) -> Result<Vec<StoredSubscription>> {
                let rows: Vec<SubscriptionRow> = sqlx::query_as(&format!(
                    "SELECT {SUBSCRIPTION_COLUMNS} FROM web_push_subscriptions WHERE owner_user_id = $1"
                ))
                .bind(owner)
                .fetch_all(&self.pool)
                .await
                .context("SELECT Web Push subscriptions")?;
                Ok(rows.into_iter().map(subscription).collect())
            }

            async fn web_push_subscription(
                &self,
                endpoint_sha256: &str,
            ) -> Result<Option<StoredSubscription>> {
                let row: Option<SubscriptionRow> = sqlx::query_as(&format!(
                    "SELECT {SUBSCRIPTION_COLUMNS} FROM web_push_subscriptions WHERE endpoint_sha256 = $1"
                ))
                .bind(endpoint_sha256)
                .fetch_optional(&self.pool)
                .await
                .context("SELECT Web Push subscription")?;
                Ok(row.map(subscription))
            }

            async fn acknowledge_task(
                &self,
                owner: &str,
                session_id: &str,
                task_key: &str,
                marker: &str,
                now_ms: i64,
            ) -> Result<()> {
                sqlx::query(
                    "INSERT INTO task_acknowledgements(owner_user_id, session_id, task_key, marker, \
                     acknowledged_at_ms) VALUES ($1, $2, $3, $4, $5) \
                     ON CONFLICT(owner_user_id, session_id, task_key, marker) DO NOTHING",
                )
                .bind(owner)
                .bind(session_id)
                .bind(task_key)
                .bind(marker)
                .bind(now_ms)
                .execute(&self.pool)
                .await
                .context("INSERT task acknowledgement")?;
                Ok(())
            }

            async fn task_acknowledged(
                &self,
                owner: &str,
                session_id: &str,
                task_key: &str,
                marker: &str,
            ) -> Result<bool> {
                sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM task_acknowledgements WHERE owner_user_id = $1 \
                     AND session_id = $2 AND task_key = $3 AND marker = $4)",
                )
                .bind(owner)
                .bind(session_id)
                .bind(task_key)
                .bind(marker)
                .fetch_one(&self.pool)
                .await
                .context("SELECT task acknowledgement")
            }

            async fn task_acknowledgements(
                &self,
                owner: &str,
                session_id: &str,
            ) -> Result<Vec<(String, String)>> {
                sqlx::query_as(
                    "SELECT task_key, marker FROM task_acknowledgements \
                     WHERE owner_user_id = $1 AND session_id = $2",
                )
                .bind(owner)
                .bind(session_id)
                .fetch_all(&self.pool)
                .await
                .context("SELECT task acknowledgements")
            }
        }
    };
}

implementation!(PostgresStorage);
implementation!(SqliteStorage);
