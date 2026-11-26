//! Durable completed-turn cursor index, Linked Fork reservation receipts,
//! and atomic handoff-cancellation commits.

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
    pub before_seq: u64,
    pub workspace_base: Option<String>,
    pub workspace_include_uncommitted: Option<bool>,
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
    /// Persist one completed-turn `(message_id -> before_seq)` cursor.
    pub async fn upsert_turn_completion(&self, completion: &TurnCompletion) -> Result<()> {
        dispatch!(self, upsert_turn_completion(completion))
    }

    /// Return the newest `(message_id, before_seq)` recorded for `session_id`
    /// in `context_generation`.
    pub async fn latest_turn_completion(
        &self,
        session_id: &str,
        context_generation: u64,
    ) -> Result<Option<(String, u64)>> {
        dispatch!(self, latest_turn_completion(session_id, context_generation))
    }

    /// Resolve `before_seq` for an exact `(session_id, context_generation, message_id)`.
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

    /// Cold-history fallback for historical native forks matching hot-path
    /// turn-final and current-generation semantics in one snapshot transaction.
    pub async fn find_message_turn_before_seq(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<Option<u64>> {
        dispatch!(self, find_message_turn_before_seq(session_id, message_id))
    }

    /// Look up a durable Linked Fork receipt by `(owner, source, operation)`.
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

    /// Atomically reserve `(owner, source, operation)` BEFORE creating the
    /// destination session. Returns `Existing(receipt)` on conflict.
    pub async fn reserve_duplicate_receipt(
        &self,
        receipt: &DuplicateReceipt,
    ) -> Result<ReserveOutcome> {
        dispatch!(self, reserve_duplicate_receipt(receipt))
    }

    /// Transition a reserved receipt to `"ready"` ONLY while it is still
    /// `"reserved"` and its destination session row is durably present.
    pub async fn complete_duplicate_receipt(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
        updated_at_ms: i64,
    ) -> Result<bool> {
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

    /// Mark a receipt `"deleted"` so retries fail explicitly instead of
    /// silently creating a fresh copy after deletion.
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

    /// Remove an uncommitted reservation after creation failed before the
    /// destination existed.
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

    /// Commit Cancel Switch atomically in one database transaction: clear the
    /// destination queue, write the cancelled receipt holding the returned
    /// prompt, and delete the destination's lineage edge.
    pub async fn commit_handoff_cancellation(
        &self,
        destination_session_id: &str,
        setting_key: &str,
        setting_value: &serde_json::Value,
    ) -> Result<()> {
        dispatch!(
            self,
            commit_handoff_cancellation(destination_session_id, setting_key, setting_value)
        )
    }
}

type PgReceiptRow = (
    String,
    String,
    i64,
    Option<String>,
    Option<bool>,
    String,
    i64,
    i64,
);

type SqliteReceiptRow = (
    String,
    String,
    i64,
    Option<String>,
    Option<i64>,
    String,
    i64,
    i64,
);

impl PostgresStorage {
    pub(super) async fn upsert_turn_completion(&self, c: &TurnCompletion) -> Result<()> {
        sqlx::query(
            "INSERT INTO session_turn_completions              (session_id, context_generation, message_id, before_seq, created_at_ms)              VALUES ($1, $2, $3, $4, $5)              ON CONFLICT (session_id, context_generation, message_id) DO UPDATE SET                before_seq = GREATEST(session_turn_completions.before_seq, EXCLUDED.before_seq),                created_at_ms = EXCLUDED.created_at_ms",
        )
        .bind(&c.session_id)
        .bind(to_i64_seq(c.context_generation)?)
        .bind(&c.message_id)
        .bind(to_i64_seq(c.before_seq)?)
        .bind(c.completed_at_ms)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPSERT turn completion for {}", c.session_id))?;
        Ok(())
    }

    pub(super) async fn latest_turn_completion(
        &self,
        session_id: &str,
        context_generation: u64,
    ) -> Result<Option<(String, u64)>> {
        let row: Option<(String, i64)> = sqlx::query_as(
            "SELECT message_id, before_seq FROM session_turn_completions              WHERE session_id = $1 AND context_generation = $2              ORDER BY before_seq DESC LIMIT 1",
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
            "SELECT before_seq FROM session_turn_completions              WHERE session_id = $1 AND context_generation = $2 AND message_id = $3",
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
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let sess: Option<(String, i64)> = sqlx::query_as(
            "SELECT status, next_seq FROM sessions WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await
        .with_context(|| format!("read PG session state for cold fork {session_id}"))?;
        let Some((status, next_seq)) = sess else {
            return Ok(None);
        };
        let boundary_floor: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(seq) FROM events WHERE session_id = $1                AND payload->>'kind' = 'update'                AND payload->'update'->>'sessionUpdate' IN ('context_cleared', 'cowboy_context_renewed')",
        )
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await
        .with_context(|| format!("scan PG context boundary for {session_id}"))?;
        let msg_seq: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(seq) FROM events              WHERE session_id = $1                AND ($3::bigint IS NULL OR seq > $3)                AND payload->>'kind' = 'update'                AND payload->'update'->>'sessionUpdate' = 'agent_message_chunk'                AND payload->'update'->>'messageId' = $2",
        )
        .bind(session_id)
        .bind(message_id)
        .bind(boundary_floor)
        .fetch_one(&mut *tx)
        .await
        .with_context(|| format!("scan PG message seq for {session_id}/{message_id}"))?;
        let Some(msg_seq) = msg_seq else {
            return Ok(None);
        };
        let boundary: Option<(i64, String)> = sqlx::query_as(
            "SELECT seq, payload->>'kind' FROM events              WHERE session_id = $1 AND seq > $2                AND (                  payload->>'kind' = 'turn_end'                  OR (payload->>'kind' = 'update'                      AND payload->'update'->>'sessionUpdate' = 'user_message_chunk'                      AND COALESCE(payload->'update'->'_meta'->>'cowboyDelivery', '') <> 'current_turn')                )              ORDER BY seq ASC LIMIT 1",
        )
        .bind(session_id)
        .bind(msg_seq)
        .fetch_optional(&mut *tx)
        .await
        .with_context(|| format!("scan PG turn boundary after {session_id}:{msg_seq}"))?;
        let upper = boundary.as_ref().map(|(seq, _)| *seq);
        let has_later_answer: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM events              WHERE session_id = $1 AND seq > $2 AND ($3::bigint IS NULL OR seq < $3)                AND payload->>'kind' = 'update'                AND payload->'update'->>'sessionUpdate' = 'agent_message_chunk'                AND payload->'update'->>'messageId' IS NOT NULL                AND payload->'update'->>'messageId' <> $4)",
        )
        .bind(session_id)
        .bind(msg_seq)
        .bind(upper)
        .bind(message_id)
        .fetch_one(&mut *tx)
        .await
        .with_context(|| format!("verify PG turn-final message for {session_id}/{message_id}"))?;
        tx.commit().await?;
        if has_later_answer {
            return Ok(None);
        }
        match boundary {
            Some((seq, kind)) => {
                let seq_u64 = u64::try_from(seq).ok();
                Ok(seq_u64.map(|s| {
                    if kind == "turn_end" {
                        s.saturating_add(1)
                    } else {
                        s
                    }
                }))
            }
            None if status == "busy" => Ok(None),
            None => Ok(u64::try_from(next_seq).ok()),
        }
    }

    pub(super) async fn duplicate_receipt(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
    ) -> Result<Option<DuplicateReceipt>> {
        let row: Option<PgReceiptRow> = sqlx::query_as(
            "SELECT fingerprint, destination_session_id, before_seq, workspace_base,                     workspace_include_uncommitted, state, created_at_ms, updated_at_ms              FROM session_duplicate_receipts              WHERE owner_user_id = $1 AND source_session_id = $2 AND operation_id = $3",
        )
        .bind(owner_user_id)
        .bind(source_session_id)
        .bind(operation_id)
        .fetch_optional(&self.pool)
        .await
        .context("SELECT duplicate receipt")?;
        Ok(row.map(
            |(
                fingerprint,
                destination_session_id,
                before_seq,
                workspace_base,
                workspace_include_uncommitted,
                state,
                created_at_ms,
                updated_at_ms,
            )| DuplicateReceipt {
                owner_user_id: owner_user_id.to_owned(),
                source_session_id: source_session_id.to_owned(),
                operation_id: operation_id.to_owned(),
                fingerprint,
                destination_session_id,
                before_seq: u64::try_from(before_seq).unwrap_or(0),
                workspace_base,
                workspace_include_uncommitted,
                state,
                created_at_ms,
                updated_at_ms,
            },
        ))
    }

    pub(super) async fn reserve_duplicate_receipt(
        &self,
        receipt: &DuplicateReceipt,
    ) -> Result<ReserveOutcome> {
        let done = sqlx::query(
            "INSERT INTO session_duplicate_receipts              (owner_user_id, source_session_id, operation_id, fingerprint,               destination_session_id, before_seq, workspace_base, workspace_include_uncommitted,               state, created_at_ms, updated_at_ms)              VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)              ON CONFLICT (owner_user_id, source_session_id, operation_id) DO NOTHING",
        )
        .bind(&receipt.owner_user_id)
        .bind(&receipt.source_session_id)
        .bind(&receipt.operation_id)
        .bind(&receipt.fingerprint)
        .bind(&receipt.destination_session_id)
        .bind(to_i64_seq(receipt.before_seq)?)
        .bind(receipt.workspace_base.as_deref())
        .bind(receipt.workspace_include_uncommitted)
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
    ) -> Result<bool> {
        let done = sqlx::query(
            "UPDATE session_duplicate_receipts SET state = 'ready', updated_at_ms = $4              WHERE owner_user_id = $1 AND source_session_id = $2 AND operation_id = $3                AND state = 'reserved'                AND EXISTS (SELECT 1 FROM sessions WHERE id = destination_session_id AND deleted_at IS NULL)",
        )
        .bind(owner_user_id)
        .bind(source_session_id)
        .bind(operation_id)
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await
        .context("UPDATE duplicate receipt ready")?;
        Ok(done.rows_affected() == 1)
    }

    pub(super) async fn mark_duplicate_receipt_deleted(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
        updated_at_ms: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE session_duplicate_receipts SET state = 'deleted', updated_at_ms = $4              WHERE owner_user_id = $1 AND source_session_id = $2 AND operation_id = $3",
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
            "DELETE FROM session_duplicate_receipts              WHERE owner_user_id = $1 AND source_session_id = $2 AND operation_id = $3                AND state = 'reserved'",
        )
        .bind(owner_user_id)
        .bind(source_session_id)
        .bind(operation_id)
        .execute(&self.pool)
        .await
        .context("DELETE duplicate receipt reservation")?;
        Ok(())
    }

    pub(super) async fn commit_handoff_cancellation(
        &self,
        destination_session_id: &str,
        setting_key: &str,
        setting_value: &serde_json::Value,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE sessions SET queue = '[]'::jsonb, updated_at = now() WHERE id = $1")
            .bind(destination_session_id)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!("clear PG queue for cancelled handoff {destination_session_id}")
            })?;
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, now())              ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(setting_key)
        .bind(setting_value)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("save PG cancelled handoff receipt {setting_key}"))?;
        sqlx::query("DELETE FROM session_lineage_edges WHERE child_session_id = $1")
            .bind(destination_session_id)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!("delete PG cancelled handoff lineage {destination_session_id}")
            })?;
        tx.commit().await?;
        Ok(())
    }
}

impl SqliteStorage {
    pub(super) async fn upsert_turn_completion(&self, c: &TurnCompletion) -> Result<()> {
        sqlx::query(
            "INSERT INTO session_turn_completions              (session_id, context_generation, message_id, before_seq, created_at_ms)              VALUES (?1, ?2, ?3, ?4, ?5)              ON CONFLICT (session_id, context_generation, message_id) DO UPDATE SET                before_seq = max(session_turn_completions.before_seq, excluded.before_seq),                created_at_ms = excluded.created_at_ms",
        )
        .bind(&c.session_id)
        .bind(to_i64_seq(c.context_generation)?)
        .bind(&c.message_id)
        .bind(to_i64_seq(c.before_seq)?)
        .bind(c.completed_at_ms)
        .execute(&self.pool)
        .await
        .with_context(|| format!("UPSERT SQLite turn completion for {}", c.session_id))?;
        Ok(())
    }

    pub(super) async fn latest_turn_completion(
        &self,
        session_id: &str,
        context_generation: u64,
    ) -> Result<Option<(String, u64)>> {
        let row: Option<(String, i64)> = sqlx::query_as(
            "SELECT message_id, before_seq FROM session_turn_completions              WHERE session_id = ?1 AND context_generation = ?2              ORDER BY before_seq DESC LIMIT 1",
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
            "SELECT before_seq FROM session_turn_completions              WHERE session_id = ?1 AND context_generation = ?2 AND message_id = ?3",
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
        let mut tx = self.pool.begin().await?;
        let sess: Option<(String, i64)> = sqlx::query_as(
            "SELECT status, next_seq FROM sessions WHERE id = ?1 AND deleted_at_ms IS NULL",
        )
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await
        .with_context(|| format!("read SQLite session state for cold fork {session_id}"))?;
        let Some((status, next_seq)) = sess else {
            return Ok(None);
        };
        let boundary_floor: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(seq) FROM events WHERE session_id = ?1                AND json_extract(payload, '$.kind') = 'update'                AND json_extract(payload, '$.update.sessionUpdate') IN ('context_cleared', 'cowboy_context_renewed')",
        )
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await
        .with_context(|| format!("scan SQLite context boundary for {session_id}"))?;
        let msg_seq: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(seq) FROM events              WHERE session_id = ?1                AND (?3 IS NULL OR seq > ?3)                AND json_extract(payload, '$.kind') = 'update'                AND json_extract(payload, '$.update.sessionUpdate') = 'agent_message_chunk'                AND json_extract(payload, '$.update.messageId') = ?2",
        )
        .bind(session_id)
        .bind(message_id)
        .bind(boundary_floor)
        .fetch_one(&mut *tx)
        .await
        .with_context(|| format!("scan SQLite message seq for {session_id}/{message_id}"))?;
        let Some(msg_seq) = msg_seq else {
            return Ok(None);
        };
        let boundary: Option<(i64, String)> = sqlx::query_as(
            "SELECT seq, json_extract(payload, '$.kind') FROM events              WHERE session_id = ?1 AND seq > ?2                AND (                  json_extract(payload, '$.kind') = 'turn_end'                  OR (json_extract(payload, '$.kind') = 'update'                      AND json_extract(payload, '$.update.sessionUpdate') = 'user_message_chunk'                      AND COALESCE(json_extract(payload, '$.update._meta.cowboyDelivery'), '') <> 'current_turn')                )              ORDER BY seq ASC LIMIT 1",
        )
        .bind(session_id)
        .bind(msg_seq)
        .fetch_optional(&mut *tx)
        .await
        .with_context(|| format!("scan SQLite turn boundary after {session_id}:{msg_seq}"))?;
        let upper = boundary.as_ref().map(|(seq, _)| *seq);
        let has_later_answer: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM events              WHERE session_id = ?1 AND seq > ?2 AND (?3 IS NULL OR seq < ?3)                AND json_extract(payload, '$.kind') = 'update'                AND json_extract(payload, '$.update.sessionUpdate') = 'agent_message_chunk'                AND json_extract(payload, '$.update.messageId') IS NOT NULL                AND json_extract(payload, '$.update.messageId') <> ?4)",
        )
        .bind(session_id)
        .bind(msg_seq)
        .bind(upper)
        .bind(message_id)
        .fetch_one(&mut *tx)
        .await
        .with_context(|| format!("verify SQLite turn-final message for {session_id}/{message_id}"))?;
        tx.commit().await?;
        if has_later_answer != 0 {
            return Ok(None);
        }
        match boundary {
            Some((seq, kind)) => {
                let seq_u64 = u64::try_from(seq).ok();
                Ok(seq_u64.map(|s| {
                    if kind == "turn_end" {
                        s.saturating_add(1)
                    } else {
                        s
                    }
                }))
            }
            None if status == "busy" => Ok(None),
            None => Ok(u64::try_from(next_seq).ok()),
        }
    }

    pub(super) async fn duplicate_receipt(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
    ) -> Result<Option<DuplicateReceipt>> {
        let row: Option<SqliteReceiptRow> = sqlx::query_as(
            "SELECT fingerprint, destination_session_id, before_seq, workspace_base,                     workspace_include_uncommitted, state, created_at_ms, updated_at_ms              FROM session_duplicate_receipts              WHERE owner_user_id = ?1 AND source_session_id = ?2 AND operation_id = ?3",
        )
        .bind(owner_user_id)
        .bind(source_session_id)
        .bind(operation_id)
        .fetch_optional(&self.pool)
        .await
        .context("SELECT SQLite duplicate receipt")?;
        Ok(row.map(
            |(
                fingerprint,
                destination_session_id,
                before_seq,
                workspace_base,
                workspace_include_uncommitted,
                state,
                created_at_ms,
                updated_at_ms,
            )| DuplicateReceipt {
                owner_user_id: owner_user_id.to_owned(),
                source_session_id: source_session_id.to_owned(),
                operation_id: operation_id.to_owned(),
                fingerprint,
                destination_session_id,
                before_seq: u64::try_from(before_seq).unwrap_or(0),
                workspace_base,
                workspace_include_uncommitted: workspace_include_uncommitted.map(|v| v != 0),
                state,
                created_at_ms,
                updated_at_ms,
            },
        ))
    }

    pub(super) async fn reserve_duplicate_receipt(
        &self,
        receipt: &DuplicateReceipt,
    ) -> Result<ReserveOutcome> {
        let done = sqlx::query(
            "INSERT INTO session_duplicate_receipts              (owner_user_id, source_session_id, operation_id, fingerprint,               destination_session_id, before_seq, workspace_base, workspace_include_uncommitted,               state, created_at_ms, updated_at_ms)              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)              ON CONFLICT (owner_user_id, source_session_id, operation_id) DO NOTHING",
        )
        .bind(&receipt.owner_user_id)
        .bind(&receipt.source_session_id)
        .bind(&receipt.operation_id)
        .bind(&receipt.fingerprint)
        .bind(&receipt.destination_session_id)
        .bind(to_i64_seq(receipt.before_seq)?)
        .bind(receipt.workspace_base.as_deref())
        .bind(receipt.workspace_include_uncommitted.map(i64::from))
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
    ) -> Result<bool> {
        let done = sqlx::query(
            "UPDATE session_duplicate_receipts SET state = 'ready', updated_at_ms = ?4              WHERE owner_user_id = ?1 AND source_session_id = ?2 AND operation_id = ?3                AND state = 'reserved'                AND EXISTS (SELECT 1 FROM sessions WHERE id = destination_session_id AND deleted_at_ms IS NULL)",
        )
        .bind(owner_user_id)
        .bind(source_session_id)
        .bind(operation_id)
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await
        .context("UPDATE SQLite duplicate receipt ready")?;
        Ok(done.rows_affected() == 1)
    }

    pub(super) async fn mark_duplicate_receipt_deleted(
        &self,
        owner_user_id: &str,
        source_session_id: &str,
        operation_id: &str,
        updated_at_ms: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE session_duplicate_receipts SET state = 'deleted', updated_at_ms = ?4              WHERE owner_user_id = ?1 AND source_session_id = ?2 AND operation_id = ?3",
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
            "DELETE FROM session_duplicate_receipts              WHERE owner_user_id = ?1 AND source_session_id = ?2 AND operation_id = ?3                AND state = 'reserved'",
        )
        .bind(owner_user_id)
        .bind(source_session_id)
        .bind(operation_id)
        .execute(&self.pool)
        .await
        .context("DELETE SQLite duplicate receipt reservation")?;
        Ok(())
    }

    pub(super) async fn commit_handoff_cancellation(
        &self,
        destination_session_id: &str,
        setting_key: &str,
        setting_value: &serde_json::Value,
    ) -> Result<()> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let value_json =
            serde_json::to_string(setting_value).context("serialize cancelled setting")?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("UPDATE sessions SET queue = '[]', updated_at_ms = ?2 WHERE id = ?1")
            .bind(destination_session_id)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!("clear SQLite queue for cancelled handoff {destination_session_id}")
            })?;
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at_ms) VALUES (?1, ?2, ?3)              ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(setting_key)
        .bind(&value_json)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("save SQLite cancelled handoff receipt {setting_key}"))?;
        sqlx::query("DELETE FROM session_lineage_edges WHERE child_session_id = ?1")
            .bind(destination_session_id)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!("delete SQLite cancelled handoff lineage {destination_session_id}")
            })?;
        tx.commit().await?;
        Ok(())
    }
}
