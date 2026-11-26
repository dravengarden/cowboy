//! Durable completed-turn cursor index and Linked Fork operation receipts.

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use super::to_i64_seq;
use crate::store::{PostgresStorage, SqliteStorage, StorageBackend, Store};

/// Exact `(message_id, before_seq)` pair persisted at `TurnEnd` so cold-history
/// forks and `latest_completed` resolve an exact cutoff even after hot-tail
/// trimming or Controller restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnCompletion {
    pub session_id: String,
    pub context_generation: u64,
    pub message_id: String,
    pub before_seq: u64,
    pub completed_at_ms: i64,
}

/// Durable idempotency receipt for one Linked Fork (`duplicate`) operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateReceipt {
    pub owner_user_id: String,
    pub source_session_id: String,
    pub operation_id: String,
    pub fingerprint: String,
    pub destination_session_id: String,
    pub created_at_ms: i64,
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
    pub async fn record_turn_completion(&self, row: &TurnCompletion) -> Result<()> {
        dispatch!(self, record_turn_completion(row))
    }

    pub async fn latest_turn_completion(
        &self,
        session_id: &str,
        context_generation: u64,
    ) -> Result<Option<(String, u64)>> {
        dispatch!(self, latest_turn_completion(session_id, context_generation))
    }

    pub async fn turn_completion_before_seq(
        &self,
        session_id: &str,
        context_generation: u64,
        message_id: &str,
    ) -> Result<Option<u64>> {
        dispatch!(
            self,
            turn_completion_before_seq(session_id, context_generation, message_id)
        )
    }

    pub async fn duplicate_receipt(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
    ) -> Result<Option<DuplicateReceipt>> {
        dispatch!(
            self,
            duplicate_receipt(owner_user_id, source_session_id, operation_id)
        )
    }

    pub async fn put_duplicate_receipt(&self, receipt: &DuplicateReceipt) -> Result<bool> {
        dispatch!(self, put_duplicate_receipt(receipt))
    }

    pub async fn delete_duplicate_receipt(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
    ) -> Result<()> {
        dispatch!(
            self,
            delete_duplicate_receipt(owner_user_id, source_session_id, operation_id)
        )
    }
}

impl PostgresStorage {
    pub(super) async fn record_turn_completion(&self, row: &TurnCompletion) -> Result<()> {
        sqlx::query(
            "INSERT INTO session_turn_completions \
             (session_id, context_generation, message_id, before_seq, completed_at_ms) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (session_id, context_generation, message_id) DO UPDATE SET \
               before_seq = GREATEST(session_turn_completions.before_seq, EXCLUDED.before_seq), \
               completed_at_ms = EXCLUDED.completed_at_ms",
        )
        .bind(&row.session_id)
        .bind(to_i64_seq(row.context_generation)?)
        .bind(&row.message_id)
        .bind(to_i64_seq(row.before_seq)?)
        .bind(row.completed_at_ms)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPSERT turn completion for {}", row.session_id))?;
        Ok(())
    }

    pub(super) async fn latest_turn_completion(
        &self,
        session_id: &str,
        context_generation: u64,
    ) -> Result<Option<(String, u64)>> {
        let row: Option<(String, i64)> = sqlx::query_as(
            "SELECT message_id, before_seq FROM session_turn_completions \
             WHERE session_id = $1 AND context_generation = $2 \
             ORDER BY before_seq DESC LIMIT 1",
        )
        .bind(session_id)
        .bind(to_i64_seq(context_generation)?)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("SELECT latest turn completion for {session_id}"))?;
        Ok(row.map(|(msg, seq)| (msg, u64::try_from(seq).unwrap_or(0))))
    }

    pub(super) async fn turn_completion_before_seq(
        &self,
        session_id: &str,
        context_generation: u64,
        message_id: &str,
    ) -> Result<Option<u64>> {
        let seq: Option<i64> = sqlx::query_scalar(
            "SELECT before_seq FROM session_turn_completions \
             WHERE session_id = $1 AND context_generation = $2 AND message_id = $3",
        )
        .bind(session_id)
        .bind(to_i64_seq(context_generation)?)
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("SELECT turn completion cursor for {session_id}"))?;
        Ok(seq.and_then(|value| u64::try_from(value).ok()))
    }

    pub(super) async fn duplicate_receipt(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
    ) -> Result<Option<DuplicateReceipt>> {
        let row: Option<(String, String, i64)> = sqlx::query_as(
            "SELECT fingerprint, destination_session_id, created_at_ms \
             FROM session_duplicate_receipts \
             WHERE owner_user_id = $1 AND source_session_id = $2 AND operation_id = $3",
        )
        .bind(owner_user_id)
        .bind(source_session_id)
        .bind(operation_id)
        .fetch_optional(&self.pool)
        .await
        .context("SELECT duplicate receipt")?;
        Ok(row.map(
            |(fingerprint, destination_session_id, created_at_ms)| DuplicateReceipt {
                owner_user_id: owner_user_id.to_owned(),
                source_session_id: source_session_id.to_owned(),
                operation_id: operation_id.to_owned(),
                fingerprint,
                destination_session_id,
                created_at_ms,
            },
        ))
    }

    pub(super) async fn put_duplicate_receipt(&self, receipt: &DuplicateReceipt) -> Result<bool> {
        let done = sqlx::query(
            "INSERT INTO session_duplicate_receipts \
             (owner_user_id, source_session_id, operation_id, fingerprint, destination_session_id, created_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (owner_user_id, source_session_id, operation_id) DO NOTHING",
        )
        .bind(&receipt.owner_user_id)
        .bind(&receipt.source_session_id)
        .bind(&receipt.operation_id)
        .bind(&receipt.fingerprint)
        .bind(&receipt.destination_session_id)
        .bind(receipt.created_at_ms)
        .execute(&self.pool)
        .await
        .context("INSERT duplicate receipt")?;
        Ok(done.rows_affected() > 0)
    }

    pub(super) async fn delete_duplicate_receipt(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "DELETE FROM session_duplicate_receipts \
             WHERE owner_user_id = $1 AND source_session_id = $2 AND operation_id = $3",
        )
        .bind(owner_user_id)
        .bind(source_session_id)
        .bind(operation_id)
        .execute(&self.pool)
        .await
        .context("DELETE duplicate receipt")?;
        Ok(())
    }
}

impl SqliteStorage {
    pub(super) async fn record_turn_completion(&self, row: &TurnCompletion) -> Result<()> {
        sqlx::query(
            "INSERT INTO session_turn_completions \
             (session_id, context_generation, message_id, before_seq, completed_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (session_id, context_generation, message_id) DO UPDATE SET \
               before_seq = max(session_turn_completions.before_seq, excluded.before_seq), \
               completed_at_ms = excluded.completed_at_ms",
        )
        .bind(&row.session_id)
        .bind(to_i64_seq(row.context_generation)?)
        .bind(&row.message_id)
        .bind(to_i64_seq(row.before_seq)?)
        .bind(row.completed_at_ms)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPSERT SQLite turn completion for {}", row.session_id))?;
        Ok(())
    }

    pub(super) async fn latest_turn_completion(
        &self,
        session_id: &str,
        context_generation: u64,
    ) -> Result<Option<(String, u64)>> {
        let row: Option<(String, i64)> = sqlx::query_as(
            "SELECT message_id, before_seq FROM session_turn_completions \
             WHERE session_id = ?1 AND context_generation = ?2 \
             ORDER BY before_seq DESC LIMIT 1",
        )
        .bind(session_id)
        .bind(to_i64_seq(context_generation)?)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite latest turn completion for {session_id}"))?;
        Ok(row.map(|(msg, seq)| (msg, u64::try_from(seq).unwrap_or(0))))
    }

    pub(super) async fn turn_completion_before_seq(
        &self,
        session_id: &str,
        context_generation: u64,
        message_id: &str,
    ) -> Result<Option<u64>> {
        let seq: Option<i64> = sqlx::query_scalar(
            "SELECT before_seq FROM session_turn_completions \
             WHERE session_id = ?1 AND context_generation = ?2 AND message_id = ?3",
        )
        .bind(session_id)
        .bind(to_i64_seq(context_generation)?)
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("SELECT SQLite turn completion cursor for {session_id}"))?;
        Ok(seq.and_then(|value| u64::try_from(value).ok()))
    }

    pub(super) async fn duplicate_receipt(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
    ) -> Result<Option<DuplicateReceipt>> {
        let row: Option<(String, String, i64)> = sqlx::query_as(
            "SELECT fingerprint, destination_session_id, created_at_ms \
             FROM session_duplicate_receipts \
             WHERE owner_user_id = ?1 AND source_session_id = ?2 AND operation_id = ?3",
        )
        .bind(owner_user_id)
        .bind(source_session_id)
        .bind(operation_id)
        .fetch_optional(&self.pool)
        .await
        .context("SELECT SQLite duplicate receipt")?;
        Ok(row.map(
            |(fingerprint, destination_session_id, created_at_ms)| DuplicateReceipt {
                owner_user_id: owner_user_id.to_owned(),
                source_session_id: source_session_id.to_owned(),
                operation_id: operation_id.to_owned(),
                fingerprint,
                destination_session_id,
                created_at_ms,
            },
        ))
    }

    pub(super) async fn put_duplicate_receipt(&self, receipt: &DuplicateReceipt) -> Result<bool> {
        let done = sqlx::query(
            "INSERT INTO session_duplicate_receipts \
             (owner_user_id, source_session_id, operation_id, fingerprint, destination_session_id, created_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT (owner_user_id, source_session_id, operation_id) DO NOTHING",
        )
        .bind(&receipt.owner_user_id)
        .bind(&receipt.source_session_id)
        .bind(&receipt.operation_id)
        .bind(&receipt.fingerprint)
        .bind(&receipt.destination_session_id)
        .bind(receipt.created_at_ms)
        .execute(&self.pool)
        .await
        .context("INSERT SQLite duplicate receipt")?;
        Ok(done.rows_affected() > 0)
    }

    pub(super) async fn delete_duplicate_receipt(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "DELETE FROM session_duplicate_receipts \
             WHERE owner_user_id = ?1 AND source_session_id = ?2 AND operation_id = ?3",
        )
        .bind(owner_user_id)
        .bind(source_session_id)
        .bind(operation_id)
        .execute(&self.pool)
        .await
        .context("DELETE SQLite duplicate receipt")?;
        Ok(())
    }
}
