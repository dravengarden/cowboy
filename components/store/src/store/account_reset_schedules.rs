//! Owner-scheduled banked resets of named Provider accounts.
//!
//! One row per `(owner, account)`. The row is the one-shot authority to
//! spend the account's next reset: the timer deletes it, comparing the
//! idempotency key, before anything is sent, so a schedule fires at most once
//! even across a Controller restart. A replacement or cancellation changes or
//! removes the key, so a timer holding the earlier one claims nothing.

use anyhow::{Context as _, Result};

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};

/// One persisted named-account reset schedule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountResetSchedule {
    pub owner_id: String,
    pub account_id: String,
    pub fire_at_ms: i64,
    /// When the timer may next try it: the fire time, or later after pacing
    /// or an unsent preflight failure.
    pub next_attempt_at_ms: i64,
    /// Unsent preflight failures so far; pacing waits are not counted.
    pub attempt_count: i32,
    pub idempotency_key: String,
    /// The sign-in the owner scheduled it under.
    pub authority: String,
}

type Row = (String, String, i64, i64, i32, String, String);

fn decode(row: Row) -> AccountResetSchedule {
    let (owner_id, account_id, fire_at_ms, next_attempt_at_ms, attempt_count, key, authority) = row;
    AccountResetSchedule {
        owner_id,
        account_id,
        fire_at_ms,
        next_attempt_at_ms,
        attempt_count,
        idempotency_key: key,
        authority,
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
    /// Create or replace the owner's schedule of this account. A replacement
    /// gets the new key, so an earlier one can no longer be claimed.
    pub async fn upsert_account_reset_schedule(
        &self,
        owner: &str,
        account: &str,
        fire_at_ms: i64,
        idempotency_key: &str,
        authority: &str,
    ) -> Result<()> {
        dispatch!(
            self,
            upsert_account_reset_schedule_raw(
                owner,
                account,
                fire_at_ms,
                idempotency_key,
                authority
            )
        )
    }

    /// The owner's schedule of this account, if any.
    pub async fn account_reset_schedule(
        &self,
        owner: &str,
        account: &str,
    ) -> Result<Option<AccountResetSchedule>> {
        Ok(dispatch!(self, account_reset_schedule_raw(owner, account))?.map(decode))
    }

    /// The owner's schedules, for one usage listing.
    pub async fn account_reset_schedules(&self, owner: &str) -> Result<Vec<AccountResetSchedule>> {
        Ok(dispatch!(self, account_reset_schedules_raw(owner))?
            .into_iter()
            .map(decode)
            .collect())
    }

    /// Schedules whose next attempt is due, oldest first, at most `limit`.
    pub async fn due_account_reset_schedules(
        &self,
        now_ms: i64,
        limit: i64,
    ) -> Result<Vec<AccountResetSchedule>> {
        Ok(
            dispatch!(self, due_account_reset_schedules_raw(now_ms, limit))?
                .into_iter()
                .map(decode)
                .collect(),
        )
    }

    /// Remove exactly this schedule (its key): true only for the one caller
    /// that removed it. The timer claims a schedule this way before it sends
    /// anything.
    pub async fn claim_account_reset_schedule(
        &self,
        owner: &str,
        account: &str,
        idempotency_key: &str,
    ) -> Result<bool> {
        Ok(dispatch!(
            self,
            claim_account_reset_schedule_raw(owner, account, idempotency_key)
        )? == 1)
    }

    /// Move exactly this schedule's next attempt, counting an unsent
    /// preflight failure when `failed`. False when it was replaced or removed.
    pub async fn defer_account_reset_schedule(
        &self,
        owner: &str,
        account: &str,
        idempotency_key: &str,
        next_attempt_at_ms: i64,
        failed: bool,
    ) -> Result<bool> {
        Ok(dispatch!(
            self,
            defer_account_reset_schedule_raw(
                owner,
                account,
                idempotency_key,
                next_attempt_at_ms,
                i32::from(failed)
            )
        )? == 1)
    }

    /// Cancel the owner's schedule of this account; true when one existed.
    pub async fn delete_account_reset_schedule(&self, owner: &str, account: &str) -> Result<bool> {
        Ok(dispatch!(self, delete_account_reset_schedule_raw(owner, account))? == 1)
    }
}

const COLUMNS: &str = "owner_id, account_id, fire_at_ms, next_attempt_at_ms, attempt_count, \
                       idempotency_key, authority";

macro_rules! shared_impl {
    ($backend:ty) => {
        impl $backend {
            async fn upsert_account_reset_schedule_raw(
                &self,
                owner: &str,
                account: &str,
                fire_at_ms: i64,
                idempotency_key: &str,
                authority: &str,
            ) -> Result<()> {
                sqlx::query(
                    "INSERT INTO provider_account_reset_schedules \
                     (owner_id, account_id, fire_at_ms, next_attempt_at_ms, attempt_count, \
                      idempotency_key, authority) \
                     VALUES ($1, $2, $3, $3, 0, $4, $5) \
                     ON CONFLICT (owner_id, account_id) DO UPDATE SET \
                     fire_at_ms = excluded.fire_at_ms, \
                     next_attempt_at_ms = excluded.next_attempt_at_ms, attempt_count = 0, \
                     idempotency_key = excluded.idempotency_key, authority = excluded.authority",
                )
                .bind(owner)
                .bind(account)
                .bind(fire_at_ms)
                .bind(idempotency_key)
                .bind(authority)
                .execute(&self.pool)
                .await
                .context("UPSERT provider_account_reset_schedules")?;
                Ok(())
            }

            async fn account_reset_schedule_raw(
                &self,
                owner: &str,
                account: &str,
            ) -> Result<Option<Row>> {
                sqlx::query_as(&format!(
                    "SELECT {COLUMNS} FROM provider_account_reset_schedules \
                     WHERE owner_id = $1 AND account_id = $2"
                ))
                .bind(owner)
                .bind(account)
                .fetch_optional(&self.pool)
                .await
                .context("SELECT provider_account_reset_schedules")
            }

            async fn account_reset_schedules_raw(&self, owner: &str) -> Result<Vec<Row>> {
                sqlx::query_as(&format!(
                    "SELECT {COLUMNS} FROM provider_account_reset_schedules \
                     WHERE owner_id = $1 ORDER BY account_id"
                ))
                .bind(owner)
                .fetch_all(&self.pool)
                .await
                .context("SELECT owner provider_account_reset_schedules")
            }

            async fn due_account_reset_schedules_raw(
                &self,
                now_ms: i64,
                limit: i64,
            ) -> Result<Vec<Row>> {
                sqlx::query_as(&format!(
                    "SELECT {COLUMNS} FROM provider_account_reset_schedules \
                     WHERE next_attempt_at_ms <= $1 \
                     ORDER BY next_attempt_at_ms, owner_id, account_id LIMIT $2"
                ))
                .bind(now_ms)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
                .context("SELECT due provider_account_reset_schedules")
            }

            async fn claim_account_reset_schedule_raw(
                &self,
                owner: &str,
                account: &str,
                idempotency_key: &str,
            ) -> Result<u64> {
                Ok(sqlx::query(
                    "DELETE FROM provider_account_reset_schedules \
                     WHERE owner_id = $1 AND account_id = $2 AND idempotency_key = $3",
                )
                .bind(owner)
                .bind(account)
                .bind(idempotency_key)
                .execute(&self.pool)
                .await
                .context("claim provider_account_reset_schedules")?
                .rows_affected())
            }

            async fn defer_account_reset_schedule_raw(
                &self,
                owner: &str,
                account: &str,
                idempotency_key: &str,
                next_attempt_at_ms: i64,
                failed: i32,
            ) -> Result<u64> {
                Ok(sqlx::query(
                    "UPDATE provider_account_reset_schedules \
                     SET next_attempt_at_ms = $4, attempt_count = attempt_count + $5 \
                     WHERE owner_id = $1 AND account_id = $2 AND idempotency_key = $3",
                )
                .bind(owner)
                .bind(account)
                .bind(idempotency_key)
                .bind(next_attempt_at_ms)
                .bind(failed)
                .execute(&self.pool)
                .await
                .context("defer provider_account_reset_schedules")?
                .rows_affected())
            }

            async fn delete_account_reset_schedule_raw(
                &self,
                owner: &str,
                account: &str,
            ) -> Result<u64> {
                Ok(sqlx::query(
                    "DELETE FROM provider_account_reset_schedules \
                     WHERE owner_id = $1 AND account_id = $2",
                )
                .bind(owner)
                .bind(account)
                .execute(&self.pool)
                .await
                .context("DELETE provider_account_reset_schedules")?
                .rows_affected())
            }
        }
    };
}

shared_impl!(PostgresStorage);
shared_impl!(SqliteStorage);
