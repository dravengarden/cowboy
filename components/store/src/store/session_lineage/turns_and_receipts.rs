//! Durable completed-turn cursor index and Linked Fork reservation receipts.

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

/// Durable reservation & idempotency receipt for one Linked Fork (`duplicate`).
/// Lifecycle: `"reserved"` -> `"ready"` -> `"deleted"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateReceipt {
    pub owner_user_id: String,
    pub source_session_id: String,
    pub operation_id: String,
    pub fingerprint: String,
    pub destination_session_id: String,
    pub state: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Atomic reservation outcome for a Linked Fork operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveOutcome {
    Reserved,
    Existing(DuplicateReceipt),
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

    /// Cold-history fallback: scan durable `events` for `message_id` and return
    /// the exclusive sequence cursor right after its completed turn.
    pub async fn find_message_turn_before_seq(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<Option<u64>> {
        dispatch!(self, find_message_turn_before_seq(session_id, message_id))
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

    pub async fn reserve_duplicate_receipt(
        &self,
        receipt: &DuplicateReceipt,
    ) -> Result<ReserveOutcome> {
        dispatch!(self, reserve_duplicate_receipt(receipt))
    }

    pub async fn complete_duplicate_receipt(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
        updated_at_ms: i64,
    ) -> Result<()> {
        dispatch!(
            self,
            complete_duplicate_receipt(
                owner_user_id,
                source_session_id,
                operation_id,
                updated_at_ms
            )
        )
    }

    pub async fn mark_duplicate_receipt_deleted(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
        updated_at_ms: i64,
    ) -> Result<()> {
        dispatch!(
            self,
            mark_duplicate_receipt_deleted(
                owner_user_id,
                source_session_id,
                operation_id,
                updated_at_ms,
            )
        )
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

    pub(super) async fn load_max_turn_completion_floors(&self) -> Result<Vec<(String, u64)>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT session_id, MAX(before_seq) FROM session_turn_completions GROUP BY session_id",
        )
        .fetch_all(&self.pool)
        .await
        .context("SELECT max turn completion floors")?;
        Ok(rows
            .into_iter()
            .filter_map(|(sid, seq)| u64::try_from(seq).ok().map(|s| (sid, s)))
            .collect())
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

    pub(super) async fn find_message_turn_before_seq(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<Option<u64>> {
        let msg_seq: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(seq) FROM events \
             WHERE session_id = $1 \
               AND payload->>'kind' = 'update' \
               AND payload->'update'->>'sessionUpdate' = 'agent_message_chunk' \
               AND payload->'update'->>'messageId' = $2",
        )
        .bind(session_id)
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("scan PG message seq for {session_id}/{message_id}"))?
        .flatten();
        let Some(msg_seq) = msg_seq else {
            return Ok(None);
        };
        let boundary: Option<(i64, String)> = sqlx::query_as(
            "SELECT seq, payload->>'kind' FROM events \
             WHERE session_id = $1 AND seq > $2 \
               AND ( \
                 payload->>'kind' = 'turn_end' \
                 OR (payload->>'kind' = 'update' \
                     AND payload->'update'->>'sessionUpdate' = 'user_message_chunk' \
                     AND COALESCE(payload->'update'->'_meta'->>'cowboyDelivery', '') <> 'current_turn') \
               ) \
             ORDER BY seq ASC LIMIT 1",
        )
        .bind(session_id)
        .bind(msg_seq)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("scan PG turn boundary after {session_id}:{msg_seq}"))?;
        Ok(boundary.and_then(|(seq, kind)| {
            let seq_u64 = u64::try_from(seq).ok()?;
            Some(if kind == "turn_end" {
                seq_u64.saturating_add(1)
            } else {
                seq_u64
            })
        }))
    }

    pub(super) async fn duplicate_receipt(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
    ) -> Result<Option<DuplicateReceipt>> {
        let row: Option<(String, String, String, i64, i64)> = sqlx::query_as(
            "SELECT fingerprint, destination_session_id, state, created_at_ms, updated_at_ms \
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
            |(fingerprint, destination_session_id, state, created_at_ms, updated_at_ms)| {
                DuplicateReceipt {
                    owner_user_id: owner_user_id.to_owned(),
                    source_session_id: source_session_id.to_owned(),
                    operation_id: operation_id.to_owned(),
                    fingerprint,
                    destination_session_id,
                    state,
                    created_at_ms,
                    updated_at_ms,
                }
            },
        ))
    }

    pub(super) async fn reserve_duplicate_receipt(
        &self,
        receipt: &DuplicateReceipt,
    ) -> Result<ReserveOutcome> {
        let done = sqlx::query(
            "INSERT INTO session_duplicate_receipts \
             (owner_user_id, source_session_id, operation_id, fingerprint, \
              destination_session_id, state, created_at_ms, updated_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (owner_user_id, source_session_id, operation_id) DO NOTHING",
        )
        .bind(&receipt.owner_user_id)
        .bind(&receipt.source_session_id)
        .bind(&receipt.operation_id)
        .bind(&receipt.fingerprint)
        .bind(&receipt.destination_session_id)
        .bind(&receipt.state)
        .bind(receipt.created_at_ms)
        .bind(receipt.updated_at_ms)
        .execute(&self.pool)
        .await
        .context("INSERT duplicate receipt reservation")?;
        if done.rows_affected() > 0 {
            return Ok(ReserveOutcome::Reserved);
        }
        let existing = self
            .duplicate_receipt(
                &receipt.owner_user_id,
                &receipt.source_session_id,
                &receipt.operation_id,
            )
            .await?
            .context("duplicate receipt vanished immediately after conflict")?;
        Ok(ReserveOutcome::Existing(existing))
    }

    pub(super) async fn complete_duplicate_receipt(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
        updated_at_ms: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE session_duplicate_receipts SET state = 'ready', updated_at_ms = $4 \
             WHERE owner_user_id = $1 AND source_session_id = $2 AND operation_id = $3 \
               AND state <> 'deleted'",
        )
        .bind(owner_user_id)
        .bind(source_session_id)
        .bind(operation_id)
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await
        .context("UPDATE duplicate receipt ready")?;
        Ok(())
    }

    pub(super) async fn mark_duplicate_receipt_deleted(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
        updated_at_ms: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE session_duplicate_receipts SET state = 'deleted', updated_at_ms = $4 \
             WHERE owner_user_id = $1 AND source_session_id = $2 AND operation_id = $3",
        )
        .bind(owner_user_id)
        .bind(source_session_id)
        .bind(operation_id)
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await
        .context("UPDATE duplicate receipt deleted")?;
        Ok(())
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

    pub(super) async fn load_max_turn_completion_floors(&self) -> Result<Vec<(String, u64)>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT session_id, MAX(before_seq) FROM session_turn_completions GROUP BY session_id",
        )
        .fetch_all(&self.pool)
        .await
        .context("SELECT SQLite max turn completion floors")?;
        Ok(rows
            .into_iter()
            .filter_map(|(sid, seq)| u64::try_from(seq).ok().map(|s| (sid, s)))
            .collect())
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

    pub(super) async fn find_message_turn_before_seq(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<Option<u64>> {
        let msg_seq: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(seq) FROM events \
             WHERE session_id = ?1 \
               AND json_extract(payload, '$.kind') = 'update' \
               AND json_extract(payload, '$.update.sessionUpdate') = 'agent_message_chunk' \
               AND json_extract(payload, '$.update.messageId') = ?2",
        )
        .bind(session_id)
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("scan SQLite message seq for {session_id}/{message_id}"))?
        .flatten();
        let Some(msg_seq) = msg_seq else {
            return Ok(None);
        };
        let boundary: Option<(i64, String)> = sqlx::query_as(
            "SELECT seq, json_extract(payload, '$.kind') FROM events \
             WHERE session_id = ?1 AND seq > ?2 \
               AND ( \
                 json_extract(payload, '$.kind') = 'turn_end' \
                 OR (json_extract(payload, '$.kind') = 'update' \
                     AND json_extract(payload, '$.update.sessionUpdate') = 'user_message_chunk' \
                     AND COALESCE(json_extract(payload, '$.update._meta.cowboyDelivery'), '') <> 'current_turn') \
               ) \
             ORDER BY seq ASC LIMIT 1",
        )
        .bind(session_id)
        .bind(msg_seq)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("scan SQLite turn boundary after {session_id}:{msg_seq}"))?;
        Ok(boundary.and_then(|(seq, kind)| {
            let seq_u64 = u64::try_from(seq).ok()?;
            Some(if kind == "turn_end" {
                seq_u64.saturating_add(1)
            } else {
                seq_u64
            })
        }))
    }

    pub(super) async fn duplicate_receipt(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
    ) -> Result<Option<DuplicateReceipt>> {
        let row: Option<(String, String, String, i64, i64)> = sqlx::query_as(
            "SELECT fingerprint, destination_session_id, state, created_at_ms, updated_at_ms \
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
            |(fingerprint, destination_session_id, state, created_at_ms, updated_at_ms)| {
                DuplicateReceipt {
                    owner_user_id: owner_user_id.to_owned(),
                    source_session_id: source_session_id.to_owned(),
                    operation_id: operation_id.to_owned(),
                    fingerprint,
                    destination_session_id,
                    state,
                    created_at_ms,
                    updated_at_ms,
                }
            },
        ))
    }

    pub(super) async fn reserve_duplicate_receipt(
        &self,
        receipt: &DuplicateReceipt,
    ) -> Result<ReserveOutcome> {
        let done = sqlx::query(
            "INSERT INTO session_duplicate_receipts \
             (owner_user_id, source_session_id, operation_id, fingerprint, \
              destination_session_id, state, created_at_ms, updated_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT (owner_user_id, source_session_id, operation_id) DO NOTHING",
        )
        .bind(&receipt.owner_user_id)
        .bind(&receipt.source_session_id)
        .bind(&receipt.operation_id)
        .bind(&receipt.fingerprint)
        .bind(&receipt.destination_session_id)
        .bind(&receipt.state)
        .bind(receipt.created_at_ms)
        .bind(receipt.updated_at_ms)
        .execute(&self.pool)
        .await
        .context("INSERT SQLite duplicate receipt reservation")?;
        if done.rows_affected() > 0 {
            return Ok(ReserveOutcome::Reserved);
        }
        let existing = self
            .duplicate_receipt(
                &receipt.owner_user_id,
                &receipt.source_session_id,
                &receipt.operation_id,
            )
            .await?
            .context("SQLite duplicate receipt vanished immediately after conflict")?;
        Ok(ReserveOutcome::Existing(existing))
    }

    pub(super) async fn complete_duplicate_receipt(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
        updated_at_ms: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE session_duplicate_receipts SET state = 'ready', updated_at_ms = ?4 \
             WHERE owner_user_id = ?1 AND source_session_id = ?2 AND operation_id = ?3 \
               AND state <> 'deleted'",
        )
        .bind(owner_user_id)
        .bind(source_session_id)
        .bind(operation_id)
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await
        .context("UPDATE SQLite duplicate receipt ready")?;
        Ok(())
    }

    pub(super) async fn mark_duplicate_receipt_deleted(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
        updated_at_ms: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE session_duplicate_receipts SET state = 'deleted', updated_at_ms = ?4 \
             WHERE owner_user_id = ?1 AND source_session_id = ?2 AND operation_id = ?3",
        )
        .bind(owner_user_id)
        .bind(source_session_id)
        .bind(operation_id)
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await
        .context("UPDATE SQLite duplicate receipt deleted")?;
        Ok(())
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
