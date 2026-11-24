//! Durable completion-notification delivery state for opted-in Managed Calls
//! (Phase 2A of wait-cache observability).
//!
//! Each opted-in terminal Call receives one immutable `completion_id`
//! (`call-notice:<call_id>`) atomically in the same transaction as its terminal
//! ledger transition, with explicit `enqueued` / `accepted` / `consumed` /
//! `unknown` delivery states recoverable across Controller restarts and native
//! session rollover.

use super::{PostgresStorage, Result, SqliteStorage, StorageBackend, Store};
use crate::agent_model::CALL_NOTICE_PREFIX;
use crate::model::QueuedMessage;
use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod projection;
pub(in crate::store) use projection::{
    clear_calls_on_transcript_clear_pg, clear_calls_on_transcript_clear_sqlite, project_events_pg,
    project_events_sqlite,
};

pub const INSERT_ENQUEUED: &str = "INSERT INTO managed_call_notices \
    (completion_id, call_id, parent_session_id, state, dispatch_started_at_ms, echo_seq, reason, created_at_ms, updated_at_ms) \
    VALUES ($1, $2, $3, 'enqueued', NULL, NULL, NULL, $4, $4) \
    ON CONFLICT (completion_id) DO NOTHING";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeDeliveryState {
    Enqueued,
    Accepted,
    Consumed,
    Unknown,
}

impl NoticeDeliveryState {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Enqueued => "enqueued",
            Self::Accepted => "accepted",
            Self::Consumed => "consumed",
            Self::Unknown => "unknown",
        }
    }

    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "enqueued" => Ok(Self::Enqueued),
            "accepted" => Ok(Self::Accepted),
            "consumed" => Ok(Self::Consumed),
            "unknown" => Ok(Self::Unknown),
            other => anyhow::bail!("unknown managed_call_notices state {other:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallNoticeDelivery {
    pub completion_id: String,
    pub call_id: String,
    pub parent_session_id: String,
    pub state: NoticeDeliveryState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatch_started_at_ms: Option<i64>,
    /// Internal dispatch-attempt command ID bound when `begin_call_notice_dispatch`
    /// claims this notice. Used to authenticate `UndeliveredPrompt` receipts and
    /// recover ownership after a lost SQL commit response.
    #[serde(skip)]
    pub dispatch_command_id: Option<String>,
    /// Internal recipient worker epoch observed for this dispatch attempt.
    #[serde(skip)]
    pub dispatch_worker_epoch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub echo_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Internal deduplication receipt for the last committed non-delivery requeue
    /// (`worker:<epoch>:<seq>`, `teardown:<cmd>`, `recovery-abort:<cmd>`). Never
    /// serialized in product API responses.
    #[serde(skip)]
    pub last_undelivered_key: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Exact identity and idempotency receipt for releasing a notice's active
/// dispatch claim atomically with a `sessions.queue` update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeClaimRelease {
    pub completion_id: String,
    pub expected_stamp: Option<i64>,
    pub expected_command_id: Option<String>,
    pub expected_worker_epoch: Option<String>,
    pub require_worker_epoch_match: bool,
    pub receipt_key: String,
    /// When `true`, do not release an active claim; instead verify inside the
    /// same transaction that the notice is STILL unclaimed `enqueued` with
    /// `last_undelivered_key == receipt_key` and no transcript echo before
    /// repairing `sessions.queue` (Round 3 #13).
    #[serde(default)]
    pub reconcile_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyAdoptionOutcome {
    /// Atomically inserted as `enqueued` AND claimed with this stamp and command ID.
    Claimed(i64),
    /// Atomically inserted in the non-dispatchable state implied by transcript evidence.
    SettledFromEvidence,
    /// A row already existed (concurrent adoption/creation); caller should re-run dispatch claim.
    AlreadyExists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchClaim {
    /// No `managed_call_notices` row exists for this `(parent, completion_id)`
    /// (legacy pre-2A call, wrong parent, or non-store test).
    Untracked,
    /// Atomically claimed an `enqueued` notice with this strictly monotonic
    /// `dispatch_started_at_ms` stamp.
    Claimed(i64),
    /// Notice is already dispatching, accepted, consumed, or unknown.
    AlreadyHandled(NoticeDeliveryState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeEchoEvidence {
    Absent,
    Echoed {
        echo_seq: u64,
        /// `Some(true)` for a completed `TurnEnd`, `Some(false)` for a cancelled
        /// or failed `TurnEnd`, and `None` when no `TurnEnd` followed the echo yet.
        turn_completed: Option<bool>,
        superseded_by_user_turn: bool,
    },
}

#[must_use]
pub fn completion_id_for_call(call_id: &str) -> String {
    format!("{CALL_NOTICE_PREFIX}{call_id}")
}

#[must_use]
pub fn is_completed_turn_stop_reason(stop_reason: &str) -> bool {
    matches!(
        stop_reason,
        "EndTurn"
            | "MaxTokens"
            | "MaxTurnRequests"
            | "end_turn"
            | "max_tokens"
            | "max_turn_requests"
    )
}

#[derive(sqlx::FromRow)]
struct NoticeRow {
    completion_id: String,
    call_id: String,
    parent_session_id: String,
    state: String,
    dispatch_started_at_ms: Option<i64>,
    dispatch_command_id: Option<String>,
    dispatch_worker_epoch: Option<String>,
    echo_seq: Option<i64>,
    reason: Option<String>,
    last_undelivered_key: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl NoticeRow {
    fn decode(self) -> Result<CallNoticeDelivery> {
        let echo_seq = self.echo_seq.map(u64::try_from).transpose()?;
        Ok(CallNoticeDelivery {
            completion_id: self.completion_id,
            call_id: self.call_id,
            parent_session_id: self.parent_session_id,
            state: NoticeDeliveryState::parse(&self.state)?,
            dispatch_started_at_ms: self.dispatch_started_at_ms,
            dispatch_command_id: self.dispatch_command_id,
            dispatch_worker_epoch: self.dispatch_worker_epoch,
            echo_seq,
            reason: self.reason,
            last_undelivered_key: self.last_undelivered_key,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
        })
    }
}

const SELECT_COLS: &str = "completion_id, call_id, parent_session_id, state, \
    dispatch_started_at_ms, dispatch_command_id, dispatch_worker_epoch, \
    echo_seq, reason, last_undelivered_key, created_at_ms, updated_at_ms";

macro_rules! dispatch {
    ($store:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match &$store.backend {
            StorageBackend::Postgres(db) => db.$method($($arg),*).await,
            StorageBackend::Sqlite(db) => db.$method($($arg),*).await,
        }
    };
}

impl Store {
    pub async fn call_notice_delivery(
        &self,
        parent: &str,
        call_id: &str,
    ) -> Result<Option<CallNoticeDelivery>> {
        dispatch!(self, call_notice_delivery(parent, call_id))
    }

    pub async fn call_notice_by_parent_completion_id(
        &self,
        parent: &str,
        completion_id: &str,
    ) -> Result<Option<CallNoticeDelivery>> {
        dispatch!(
            self,
            call_notice_by_parent_completion_id(parent, completion_id)
        )
    }

    /// Atomically inspect durable transcript echo evidence and insert a
    /// pre-2A queued completion notice directly in the implied delivery state
    /// inside one serialized transaction (taking the per-session advisory lock
    /// on PostgreSQL and `BEGIN IMMEDIATE` on SQLite so concurrent transcript
    /// echo projection cannot interleave between reading `events` and
    /// inserting `managed_call_notices`).
    pub async fn adopt_legacy_call_notice_atomic(
        &self,
        parent: &str,
        call_id: &str,
        completion_id: &str,
        command_id: &str,
        now_ms: i64,
    ) -> Result<LegacyAdoptionOutcome> {
        ensure!(
            completion_id.starts_with(CALL_NOTICE_PREFIX),
            "invalid call notice completion_id"
        );
        dispatch!(
            self,
            adopt_legacy_call_notice_atomic(parent, call_id, completion_id, command_id, now_ms)
        )
    }

    pub async fn call_notice_deliveries_for_parent(
        &self,
        parent: &str,
        call_ids: &[String],
    ) -> Result<HashMap<String, CallNoticeDelivery>> {
        if call_ids.is_empty() {
            return Ok(HashMap::new());
        }
        dispatch!(self, call_notice_deliveries_for_parent(parent, call_ids))
    }

    pub async fn unsettled_call_notices_page(
        &self,
        after: Option<(i64, &str)>,
        limit: i64,
    ) -> Result<Vec<CallNoticeDelivery>> {
        ensure!(
            limit > 0 && limit <= 1000,
            "invalid unsettled notice page limit"
        );
        dispatch!(self, unsettled_call_notices_page(after, limit))
    }

    pub async fn begin_call_notice_dispatch(
        &self,
        parent: &str,
        completion_id: &str,
        command_id: &str,
        now_ms: i64,
    ) -> Result<DispatchClaim> {
        ensure!(
            completion_id.starts_with(CALL_NOTICE_PREFIX),
            "invalid call notice completion_id"
        );
        dispatch!(
            self,
            begin_call_notice_dispatch(parent, completion_id, command_id, now_ms)
        )
    }

    /// Record the recipient worker epoch for an active dispatch attempt when
    /// not yet recorded, so delayed non-delivery receipts from that exact
    /// worker remain verifiable even after a replacement worker is adopted.
    pub async fn record_call_notice_dispatch_worker_epoch(
        &self,
        parent: &str,
        completion_id: &str,
        command_id: &str,
        worker_epoch: &str,
    ) -> Result<bool> {
        dispatch!(
            self,
            record_call_notice_dispatch_worker_epoch(
                parent,
                completion_id,
                command_id,
                worker_epoch
            )
        )
    }

    pub async fn clear_call_notice_dispatching(
        &self,
        parent: &str,
        completion_id: &str,
        expected_stamp: Option<i64>,
        expected_command_id: Option<&str>,
        now_ms: i64,
    ) -> Result<bool> {
        dispatch!(
            self,
            clear_call_notice_dispatching(
                parent,
                completion_id,
                expected_stamp,
                expected_command_id,
                now_ms
            )
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn mark_call_notice_accepted_if(
        &self,
        parent: &str,
        completion_id: &str,
        expected_state: NoticeDeliveryState,
        expected_dispatch_started_at_ms: Option<i64>,
        expected_echo_seq: Option<u64>,
        echo_seq: u64,
        now_ms: i64,
    ) -> Result<bool> {
        let expected_echo_seq = expected_echo_seq.map(i64::try_from).transpose()?;
        dispatch!(
            self,
            mark_call_notice_accepted_if(
                parent,
                completion_id,
                expected_state.code(),
                expected_dispatch_started_at_ms,
                expected_echo_seq,
                i64::try_from(echo_seq)?,
                now_ms,
            )
        )
    }

    pub async fn mark_call_notice_accepted(
        &self,
        parent: &str,
        completion_id: &str,
        echo_seq: u64,
        now_ms: i64,
    ) -> Result<bool> {
        dispatch!(
            self,
            mark_call_notice_accepted(parent, completion_id, i64::try_from(echo_seq)?, now_ms)
        )
    }

    pub async fn mark_call_notice_consumed(
        &self,
        parent: &str,
        completion_id: &str,
        reason: &str,
        echo_seq: Option<u64>,
        now_ms: i64,
    ) -> Result<bool> {
        let echo_seq = echo_seq.map(i64::try_from).transpose()?;
        dispatch!(
            self,
            mark_call_notice_consumed(parent, completion_id, reason, echo_seq, now_ms)
        )
    }

    /// Durably record `consumed("result_read")`, creating the row if the Call
    /// predates Phase 2A so its legacy queued notice can never replay after a
    /// restart.
    pub async fn mark_call_notice_consumed_for_result_read(
        &self,
        parent: &str,
        call_id: &str,
        completion_id: &str,
        now_ms: i64,
    ) -> Result<()> {
        ensure!(
            completion_id.starts_with(CALL_NOTICE_PREFIX),
            "invalid call notice completion_id"
        );
        dispatch!(
            self,
            mark_call_notice_consumed_for_result_read(parent, call_id, completion_id, now_ms)
        )
    }

    pub async fn mark_call_notice_unknown(
        &self,
        parent: &str,
        completion_id: &str,
        reason: &str,
        echo_seq: Option<u64>,
        now_ms: i64,
    ) -> Result<bool> {
        let echo_seq = echo_seq.map(i64::try_from).transpose()?;
        dispatch!(
            self,
            mark_call_notice_unknown(parent, completion_id, reason, echo_seq, now_ms)
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn mark_call_notice_unknown_if(
        &self,
        parent: &str,
        completion_id: &str,
        expected_state: NoticeDeliveryState,
        expected_dispatch_started_at_ms: Option<i64>,
        expected_echo_seq: Option<u64>,
        reason: &str,
        new_echo_seq: Option<u64>,
        now_ms: i64,
    ) -> Result<bool> {
        let expected_echo_seq = expected_echo_seq.map(i64::try_from).transpose()?;
        let new_echo_seq = new_echo_seq.map(i64::try_from).transpose()?;
        dispatch!(
            self,
            mark_call_notice_unknown_if(
                parent,
                completion_id,
                expected_state.code(),
                expected_dispatch_started_at_ms,
                expected_echo_seq,
                reason,
                new_echo_seq,
                now_ms,
            )
        )
    }

    pub async fn call_notice_echo_outcome(
        &self,
        parent: &str,
        completion_id: &str,
    ) -> Result<NoticeEchoEvidence> {
        dispatch!(self, call_notice_echo_outcome(parent, completion_id))
    }

    /// Persist a session's queue + drafts and atomically commit any notice
    /// withdrawals/parkings (`settled_notices`) and exact non-delivery claim
    /// releases (`released_claims`) in the same SQL transaction.
    pub async fn update_pending_with_notice_ops(
        &self,
        session_id: &str,
        queue: &[QueuedMessage],
        drafts: &[QueuedMessage],
        settled_notices: &[(String, String)],
        released_claims: &[NoticeClaimRelease],
    ) -> Result<()> {
        self.update_pending_atomic(
            session_id,
            queue,
            drafts,
            settled_notices,
            released_claims,
            None,
        )
        .await
    }

    pub async fn update_pending_atomic(
        &self,
        session_id: &str,
        queue: &[QueuedMessage],
        drafts: &[QueuedMessage],
        settled_notices: &[(String, String)],
        released_claims: &[NoticeClaimRelease],
        recovery_cmids: Option<&[String]>,
    ) -> Result<()> {
        if settled_notices.is_empty() && released_claims.is_empty() && recovery_cmids.is_none() {
            return self.update_pending(session_id, queue, drafts).await;
        }
        let now = chrono::Utc::now().timestamp_millis();
        dispatch!(
            self,
            update_pending_with_notice_ops(
                session_id,
                queue,
                drafts,
                settled_notices,
                released_claims,
                recovery_cmids,
                false,
                now,
            )
        )
        .map(|_| ())
    }

    /// Atomically release/reopen a notice claim AND update the session's
    /// pending queue ONLY IF at least one conditional claim release matched in
    /// SQL. Returns `Ok(true)` when applied and `Ok(false)` when the receipt
    /// was a duplicate or the notice was already settled/reclaimed (leaving
    /// `sessions.queue` untouched).
    pub async fn update_pending_requiring_claim_release(
        &self,
        session_id: &str,
        queue: &[QueuedMessage],
        drafts: &[QueuedMessage],
        released_claims: &[NoticeClaimRelease],
    ) -> Result<bool> {
        if released_claims.is_empty() {
            return Ok(false);
        }
        let now = chrono::Utc::now().timestamp_millis();
        dispatch!(
            self,
            update_pending_with_notice_ops(
                session_id,
                queue,
                drafts,
                &[],
                released_claims,
                None,
                true,
                now,
            )
        )
    }
}

macro_rules! notice_impl {
    (
        $backend:ty,
        $db:ty,
        $echo_cmid:literal,
        $echo_kind:literal,
        $echo_update:literal,
        $echo_delivery:literal,
        $p_cmid:literal,
        $p_kind:literal,
        $p_update:literal,
        $p_delivery:literal,
        $stop_reason:literal,
        $echo_table:literal,
        $null_eq:literal,
        $max_fn:literal,
        $update_session_pending_sql:literal,
        $upsert_setting_sql:literal,
        $bind_now_for_pending:expr,
        $begin_immediate:literal,
        $for_update:literal,
        $is_pg:expr
    ) => {
        impl $backend {
            async fn call_notice_delivery(
                &self,
                parent: &str,
                call_id: &str,
            ) -> Result<Option<CallNoticeDelivery>> {
                let sql = format!(
                    "SELECT {SELECT_COLS} FROM managed_call_notices \
                     WHERE parent_session_id = $1 AND call_id = $2"
                );
                sqlx::query_as::<_, NoticeRow>(&sql)
                    .bind(parent)
                    .bind(call_id)
                    .fetch_optional(&self.pool)
                    .await?
                    .map(NoticeRow::decode)
                    .transpose()
            }

            async fn call_notice_by_parent_completion_id(
                &self,
                parent: &str,
                completion_id: &str,
            ) -> Result<Option<CallNoticeDelivery>> {
                let sql = format!(
                    "SELECT {SELECT_COLS} FROM managed_call_notices \
                     WHERE parent_session_id = $1 AND completion_id = $2"
                );
                sqlx::query_as::<_, NoticeRow>(&sql)
                    .bind(parent)
                    .bind(completion_id)
                    .fetch_optional(&self.pool)
                    .await?
                    .map(NoticeRow::decode)
                    .transpose()
            }

            async fn call_notice_deliveries_for_parent(
                &self,
                parent: &str,
                call_ids: &[String],
            ) -> Result<HashMap<String, CallNoticeDelivery>> {
                let mut result = HashMap::new();
                for chunk in call_ids.chunks(200) {
                    let placeholders = (0..chunk.len())
                        .map(|idx| format!("${}", idx + 2))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let sql = format!(
                        "SELECT {SELECT_COLS} FROM managed_call_notices \
                         WHERE parent_session_id = $1 AND call_id IN ({placeholders})"
                    );
                    let mut query = sqlx::query_as::<_, NoticeRow>(&sql).bind(parent);
                    for call_id in chunk {
                        query = query.bind(call_id);
                    }
                    for row in query.fetch_all(&self.pool).await? {
                        let delivery = row.decode()?;
                        result.insert(delivery.call_id.clone(), delivery);
                    }
                }
                Ok(result)
            }

            async fn unsettled_call_notices_page(
                &self,
                after: Option<(i64, &str)>,
                limit: i64,
            ) -> Result<Vec<CallNoticeDelivery>> {
                let rows = if let Some((after_ms, after_id)) = after {
                    let sql = format!(
                        "SELECT {SELECT_COLS} FROM managed_call_notices \
                         WHERE (state IN ('enqueued', 'accepted') \
                            OR (state = 'unknown' AND reason IN ('worker_stopped', 'turn_unconfirmed') AND echo_seq IS NOT NULL)) \
                           AND (created_at_ms > $1 OR (created_at_ms = $1 AND completion_id > $2)) \
                         ORDER BY created_at_ms ASC, completion_id ASC LIMIT $3"
                    );
                    sqlx::query_as::<_, NoticeRow>(&sql)
                        .bind(after_ms)
                        .bind(after_id)
                        .bind(limit)
                        .fetch_all(&self.pool)
                        .await?
                } else {
                    let sql = format!(
                        "SELECT {SELECT_COLS} FROM managed_call_notices \
                         WHERE (state IN ('enqueued', 'accepted') \
                            OR (state = 'unknown' AND reason IN ('worker_stopped', 'turn_unconfirmed') AND echo_seq IS NOT NULL)) \
                         ORDER BY created_at_ms ASC, completion_id ASC LIMIT $1"
                    );
                    sqlx::query_as::<_, NoticeRow>(&sql)
                        .bind(limit)
                        .fetch_all(&self.pool)
                        .await?
                };
                rows.into_iter().map(NoticeRow::decode).collect()
            }

            async fn adopt_legacy_call_notice_atomic(
                &self,
                parent: &str,
                call_id: &str,
                completion_id: &str,
                command_id: &str,
                now_ms: i64,
            ) -> Result<LegacyAdoptionOutcome> {
                let mut tx = self.pool.begin_with($begin_immediate).await?;
                if $is_pg {
                    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                        .bind(format!("call-notice-adopt:{parent}"))
                        .execute(&mut *tx)
                        .await?;
                }
                let evidence =
                    Self::call_notice_echo_outcome_in_tx(&mut tx, parent, completion_id).await?;
                let (state, dispatch_stamp, dispatch_cmd, echo_seq, reason, outcome) =
                    match evidence {
                        NoticeEchoEvidence::Echoed {
                            echo_seq,
                            turn_completed: Some(true),
                            ..
                        } => (
                            "consumed",
                            None,
                            None,
                            Some(i64::try_from(echo_seq)?),
                            Some("turn_completed"),
                            LegacyAdoptionOutcome::SettledFromEvidence,
                        ),
                        NoticeEchoEvidence::Echoed {
                            echo_seq,
                            turn_completed: Some(false),
                            ..
                        } => (
                            "unknown",
                            None,
                            None,
                            Some(i64::try_from(echo_seq)?),
                            Some("turn_not_completed"),
                            LegacyAdoptionOutcome::SettledFromEvidence,
                        ),
                        NoticeEchoEvidence::Echoed {
                            echo_seq,
                            turn_completed: None,
                            superseded_by_user_turn: true,
                        } => (
                            "unknown",
                            None,
                            None,
                            Some(i64::try_from(echo_seq)?),
                            Some("superseded_by_user_turn"),
                            LegacyAdoptionOutcome::SettledFromEvidence,
                        ),
                        NoticeEchoEvidence::Echoed { echo_seq, .. } => (
                            "accepted",
                            None,
                            None,
                            Some(i64::try_from(echo_seq)?),
                            None,
                            LegacyAdoptionOutcome::SettledFromEvidence,
                        ),
                        NoticeEchoEvidence::Absent => (
                            "enqueued",
                            Some(now_ms),
                            Some(command_id),
                            None,
                            None,
                            LegacyAdoptionOutcome::Claimed(now_ms),
                        ),
                    };
                let res = sqlx::query(
                    "INSERT INTO managed_call_notices \
                     (completion_id, call_id, parent_session_id, state, dispatch_started_at_ms, \
                      dispatch_command_id, dispatch_worker_epoch, echo_seq, reason, created_at_ms, updated_at_ms) \
                     VALUES ($1, $2, $3, $4, $5, $6, NULL, $7, $8, $9, $9) \
                     ON CONFLICT (completion_id) DO NOTHING",
                )
                .bind(completion_id)
                .bind(call_id)
                .bind(parent)
                .bind(state)
                .bind(dispatch_stamp)
                .bind(dispatch_cmd)
                .bind(echo_seq)
                .bind(reason)
                .bind(now_ms)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                Ok(if res.rows_affected() == 1 {
                    outcome
                } else {
                    LegacyAdoptionOutcome::AlreadyExists
                })
            }

            async fn begin_call_notice_dispatch(
                &self,
                parent: &str,
                completion_id: &str,
                command_id: &str,
                now_ms: i64,
            ) -> Result<DispatchClaim> {
                let mut tx = self.pool.begin_with($begin_immediate).await?;
                // Lock the exact notice row BEFORE deciding whether to claim,
                // idempotently reuse the same `command_id`, or drop (Round 3 #11).
                // On PostgreSQL `FOR UPDATE` blocks behind any concurrent
                // transcript settlement transaction so a retry never reads a
                // stale pre-settlement `enqueued` tuple.
                let lock_sql = concat!(
                    "SELECT state, dispatch_started_at_ms, dispatch_command_id \
                     FROM managed_call_notices \
                     WHERE parent_session_id = $1 AND completion_id = $2",
                    $for_update
                );
                let locked: Option<(String, Option<i64>, Option<String>)> = sqlx::query_as(lock_sql)
                    .bind(parent)
                    .bind(completion_id)
                    .fetch_optional(&mut *tx)
                    .await?;
                let Some((state, existing_stamp, existing_cmd)) = locked else {
                    tx.commit().await?;
                    return Ok(DispatchClaim::Untracked);
                };
                if state == "enqueued" && existing_stamp.is_none() {
                    // Strictly monotonic claim token derived from updated_at_ms so
                    // a delayed async release from an earlier attempt can never
                    // match a subsequent claim even within the same millisecond.
                    let claim_sql = concat!(
                        "UPDATE managed_call_notices \
                         SET dispatch_started_at_ms = ", $max_fn, "(updated_at_ms + 1, $3), \
                             dispatch_command_id = $4, \
                             dispatch_worker_epoch = NULL, \
                             updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $3) \
                         WHERE parent_session_id = $1 AND completion_id = $2 \
                           AND state = 'enqueued' AND dispatch_started_at_ms IS NULL \
                         RETURNING dispatch_started_at_ms"
                    );
                    let claimed: Option<(i64,)> = sqlx::query_as(claim_sql)
                        .bind(parent)
                        .bind(completion_id)
                        .bind(now_ms)
                        .bind(command_id)
                        .fetch_optional(&mut *tx)
                        .await?;
                    tx.commit().await?;
                    if let Some((stamp,)) = claimed {
                        return Ok(DispatchClaim::Claimed(stamp));
                    }
                    return Ok(DispatchClaim::AlreadyHandled(
                        NoticeDeliveryState::parse(&state)?,
                    ));
                }
                tx.commit().await?;
                if state == "enqueued"
                    && let Some(stamp) = existing_stamp
                    && existing_cmd.as_deref() == Some(command_id)
                {
                    return Ok(DispatchClaim::Claimed(stamp));
                }
                Ok(DispatchClaim::AlreadyHandled(
                    NoticeDeliveryState::parse(&state)?,
                ))
            }

            async fn record_call_notice_dispatch_worker_epoch(
                &self,
                parent: &str,
                completion_id: &str,
                command_id: &str,
                worker_epoch: &str,
            ) -> Result<bool> {
                let res = sqlx::query(
                    "UPDATE managed_call_notices \
                     SET dispatch_worker_epoch = $4 \
                     WHERE parent_session_id = $1 AND completion_id = $2 \
                       AND dispatch_command_id = $3 \
                       AND dispatch_worker_epoch IS NULL \
                       AND (state = 'enqueued' OR (state = 'unknown' AND reason = 'dispatch_unconfirmed'))",
                )
                .bind(parent)
                .bind(completion_id)
                .bind(command_id)
                .bind(worker_epoch)
                .execute(&self.pool)
                .await?;
                Ok(res.rows_affected() == 1)
            }

            async fn clear_call_notice_dispatching(
                &self,
                parent: &str,
                completion_id: &str,
                expected_stamp: Option<i64>,
                expected_command_id: Option<&str>,
                now_ms: i64,
            ) -> Result<bool> {
                let sql = concat!(
                    "UPDATE managed_call_notices \
                     SET dispatch_started_at_ms = NULL, \
                         dispatch_command_id = NULL, \
                         dispatch_worker_epoch = NULL, \
                         updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $5) \
                     WHERE parent_session_id = $1 AND completion_id = $2 \
                       AND state = 'enqueued' \
                       AND ($3 IS NULL OR dispatch_started_at_ms = $3) \
                       AND ($4 IS NULL OR dispatch_command_id = $4)"
                );
                let res = sqlx::query(sql)
                    .bind(parent)
                    .bind(completion_id)
                    .bind(expected_stamp)
                    .bind(expected_command_id)
                    .bind(now_ms)
                    .execute(&self.pool)
                    .await?;
                Ok(res.rows_affected() == 1)
            }

            async fn mark_call_notice_accepted(
                &self,
                parent: &str,
                completion_id: &str,
                echo_seq: i64,
                now_ms: i64,
            ) -> Result<bool> {
                let sql = concat!(
                    "UPDATE managed_call_notices \
                     SET state = 'accepted', echo_seq = COALESCE(echo_seq, $3), \
                         reason = NULL, updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $4) \
                     WHERE parent_session_id = $1 AND completion_id = $2 \
                       AND (state = 'enqueued' OR (state = 'unknown' AND reason = 'dispatch_unconfirmed'))"
                );
                let res = sqlx::query(sql)
                    .bind(parent)
                    .bind(completion_id)
                    .bind(echo_seq)
                    .bind(now_ms)
                    .execute(&self.pool)
                    .await?;
                Ok(res.rows_affected() == 1)
            }

            #[allow(clippy::too_many_arguments)]
            async fn mark_call_notice_accepted_if(
                &self,
                parent: &str,
                completion_id: &str,
                expected_state: &str,
                expected_dispatch_started_at_ms: Option<i64>,
                expected_echo_seq: Option<i64>,
                echo_seq: i64,
                now_ms: i64,
            ) -> Result<bool> {
                let sql = concat!(
                    "UPDATE managed_call_notices \
                     SET state = 'accepted', echo_seq = COALESCE(echo_seq, $6), \
                         reason = NULL, updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $7) \
                     WHERE parent_session_id = $1 AND completion_id = $2 \
                       AND state = $3 \
                       AND (state = 'enqueued' OR (state = 'unknown' AND reason = 'dispatch_unconfirmed')) \
                       AND dispatch_started_at_ms ", $null_eq, " $4 \
                       AND echo_seq ", $null_eq, " $5"
                );
                let res = sqlx::query(sql)
                    .bind(parent)
                    .bind(completion_id)
                    .bind(expected_state)
                    .bind(expected_dispatch_started_at_ms)
                    .bind(expected_echo_seq)
                    .bind(echo_seq)
                    .bind(now_ms)
                    .execute(&self.pool)
                    .await?;
                Ok(res.rows_affected() == 1)
            }

            async fn mark_call_notice_consumed(
                &self,
                parent: &str,
                completion_id: &str,
                reason: &str,
                echo_seq: Option<i64>,
                now_ms: i64,
            ) -> Result<bool> {
                let sql = concat!(
                    "UPDATE managed_call_notices \
                     SET state = 'consumed', echo_seq = COALESCE(echo_seq, $3), \
                         reason = $4, updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $5) \
                     WHERE parent_session_id = $1 AND completion_id = $2 AND state != 'consumed'"
                );
                let res = sqlx::query(sql)
                    .bind(parent)
                    .bind(completion_id)
                    .bind(echo_seq)
                    .bind(reason)
                    .bind(now_ms)
                    .execute(&self.pool)
                    .await?;
                Ok(res.rows_affected() == 1)
            }

            async fn mark_call_notice_consumed_for_result_read(
                &self,
                parent: &str,
                call_id: &str,
                completion_id: &str,
                now_ms: i64,
            ) -> Result<()> {
                let sql = concat!(
                    "INSERT INTO managed_call_notices \
                     (completion_id, call_id, parent_session_id, state, dispatch_started_at_ms, echo_seq, reason, created_at_ms, updated_at_ms) \
                     VALUES ($2, $3, $1, 'consumed', NULL, NULL, 'result_read', $4, $4) \
                     ON CONFLICT (completion_id) DO UPDATE \
                     SET state = 'consumed', reason = 'result_read', \
                         updated_at_ms = ", $max_fn, "(managed_call_notices.updated_at_ms + 1, EXCLUDED.updated_at_ms) \
                     WHERE managed_call_notices.parent_session_id = EXCLUDED.parent_session_id \
                       AND managed_call_notices.state != 'consumed'"
                );
                sqlx::query(sql)
                    .bind(parent)
                    .bind(completion_id)
                    .bind(call_id)
                    .bind(now_ms)
                    .execute(&self.pool)
                    .await?;
                Ok(())
            }

            async fn mark_call_notice_unknown(
                &self,
                parent: &str,
                completion_id: &str,
                reason: &str,
                echo_seq: Option<i64>,
                now_ms: i64,
            ) -> Result<bool> {
                let sql = concat!(
                    "UPDATE managed_call_notices \
                     SET state = 'unknown', echo_seq = COALESCE(echo_seq, $3), \
                         reason = $4, updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $5) \
                     WHERE parent_session_id = $1 AND completion_id = $2 AND state IN ('enqueued', 'accepted')"
                );
                let res = sqlx::query(sql)
                    .bind(parent)
                    .bind(completion_id)
                    .bind(echo_seq)
                    .bind(reason)
                    .bind(now_ms)
                    .execute(&self.pool)
                    .await?;
                Ok(res.rows_affected() == 1)
            }

            #[allow(clippy::too_many_arguments)]
            async fn mark_call_notice_unknown_if(
                &self,
                parent: &str,
                completion_id: &str,
                expected_state: &str,
                expected_dispatch_started_at_ms: Option<i64>,
                expected_echo_seq: Option<i64>,
                reason: &str,
                new_echo_seq: Option<i64>,
                now_ms: i64,
            ) -> Result<bool> {
                let sql = concat!(
                    "UPDATE managed_call_notices \
                     SET state = 'unknown', echo_seq = COALESCE(echo_seq, $6), \
                         reason = $7, updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $8) \
                     WHERE parent_session_id = $1 AND completion_id = $2 \
                       AND state = $3 \
                       AND dispatch_started_at_ms ", $null_eq, " $4 \
                       AND echo_seq ", $null_eq, " $5"
                );
                let res = sqlx::query(sql)
                    .bind(parent)
                    .bind(completion_id)
                    .bind(expected_state)
                    .bind(expected_dispatch_started_at_ms)
                    .bind(expected_echo_seq)
                    .bind(new_echo_seq)
                    .bind(reason)
                    .bind(now_ms)
                    .execute(&self.pool)
                    .await?;
                Ok(res.rows_affected() == 1)
            }

            async fn call_notice_echo_outcome(
                &self,
                parent: &str,
                completion_id: &str,
            ) -> Result<NoticeEchoEvidence> {
                let mut tx = self.pool.begin().await?;
                let outcome =
                    Self::call_notice_echo_outcome_in_tx(&mut tx, parent, completion_id).await?;
                tx.commit().await?;
                Ok(outcome)
            }

            async fn call_notice_echo_outcome_in_tx(
                tx: &mut sqlx::Transaction<'_, $db>,
                parent: &str,
                completion_id: &str,
            ) -> Result<NoticeEchoEvidence> {
                let echo_sql = concat!(
                    "SELECT MIN(e.seq) FROM ",
                    $echo_table,
                    " WHERE e.session_id = $1 AND ",
                    $echo_cmid,
                    " LIKE 'call-notice:%' AND ",
                    $echo_cmid,
                    " = $2 AND ",
                    $echo_kind,
                    " = 'update' AND ",
                    $echo_update,
                    " = 'user_message_chunk' AND COALESCE(",
                    $echo_delivery,
                    ", '') != 'current_turn'"
                );
                let echo_seq: Option<i64> = sqlx::query_scalar(echo_sql)
                    .bind(parent)
                    .bind(completion_id)
                    .fetch_one(&mut **tx)
                    .await?;
                let Some(echo_seq) = echo_seq else {
                    return Ok(NoticeEchoEvidence::Absent);
                };
                // Unified turn-boundary predicate shared with project_events:
                // exempt current_turn steering, repeated same-notice echoes,
                // and contiguous untagged continuation chunks.
                let next_sql = concat!(
                    "SELECT ",
                    $echo_kind,
                    ", ",
                    $stop_reason,
                    " FROM events AS e WHERE e.session_id = $1 AND e.seq > $2 AND (",
                    $echo_kind,
                    " = 'turn_end' OR (",
                    $echo_kind,
                    " = 'update' AND ",
                    $echo_update,
                    " = 'user_message_chunk' AND COALESCE(",
                    $echo_delivery,
                    ", '') != 'current_turn' AND COALESCE(",
                    $echo_cmid,
                    ", '') != $3 AND (",
                    $echo_cmid,
                    " IS NOT NULL OR EXISTS ( \
                        SELECT 1 FROM events AS p WHERE p.session_id = e.session_id \
                        AND p.seq > $2 AND p.seq < e.seq AND NOT ( \
                            ", $p_kind, " = 'update' AND ", $p_update, " = 'user_message_chunk' \
                            AND (", $p_cmid, " IS NULL OR ", $p_cmid, " = $3) \
                            AND COALESCE(", $p_delivery, ", '') != 'current_turn' \
                        ))))) ORDER BY e.seq ASC LIMIT 1"
                );
                let next: Option<(String, Option<String>)> = sqlx::query_as(next_sql)
                    .bind(parent)
                    .bind(echo_seq)
                    .bind(completion_id)
                    .fetch_optional(&mut **tx)
                    .await?;
                let echo_seq = u64::try_from(echo_seq)?;
                Ok(match next {
                    Some((kind, stop_reason)) if kind == "turn_end" => NoticeEchoEvidence::Echoed {
                        echo_seq,
                        turn_completed: Some(
                            stop_reason
                                .as_deref()
                                .is_some_and(is_completed_turn_stop_reason),
                        ),
                        superseded_by_user_turn: false,
                    },
                    Some(_) => NoticeEchoEvidence::Echoed {
                        echo_seq,
                        turn_completed: None,
                        superseded_by_user_turn: true,
                    },
                    None => NoticeEchoEvidence::Echoed {
                        echo_seq,
                        turn_completed: None,
                        superseded_by_user_turn: false,
                    },
                })
            }

            #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
            async fn update_pending_with_notice_ops(
                &self,
                session_id: &str,
                queue: &[QueuedMessage],
                drafts: &[QueuedMessage],
                settled_notices: &[(String, String)],
                released_claims: &[NoticeClaimRelease],
                recovery_cmids: Option<&[String]>,
                require_claim_release: bool,
                now_ms: i64,
            ) -> Result<bool> {
                let mut queue_json = serde_json::to_value(queue).context("serialize queue")?;
                let mut drafts_json = serde_json::to_value(drafts).context("serialize drafts")?;
                crate::store::strip_nul(&mut queue_json);
                crate::store::strip_nul(&mut drafts_json);
                let mut tx = self.pool.begin_with($begin_immediate).await?;
                let release_sql = concat!(
                    "UPDATE managed_call_notices \
                     SET state = 'enqueued', reason = NULL, \
                         dispatch_started_at_ms = NULL, \
                         dispatch_command_id = NULL, \
                         dispatch_worker_epoch = NULL, \
                         last_undelivered_key = $7, \
                         updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $8) \
                     WHERE parent_session_id = $1 AND completion_id = $2 \
                       AND (state = 'enqueued' OR (state = 'unknown' AND reason = 'dispatch_unconfirmed')) \
                       AND echo_seq IS NULL \
                       AND dispatch_started_at_ms IS NOT NULL \
                       AND ($3 IS NULL OR dispatch_started_at_ms = $3) \
                       AND ($4 IS NULL OR dispatch_command_id = $4) \
                       AND ($5 = 0 OR dispatch_worker_epoch = $6) \
                       AND (last_undelivered_key IS NULL OR last_undelivered_key != $7) \
                       AND NOT EXISTS ( \
                           SELECT 1 FROM ", $echo_table, " \
                           WHERE e.session_id = $1 \
                             AND ", $echo_kind, " = 'update' \
                             AND ", $echo_update, " = 'user_message_chunk' \
                             AND ", $echo_cmid, " LIKE 'call-notice:%' \
                             AND ", $echo_cmid, " = $2 \
                       )"
                );
                let reconcile_sql = concat!(
                    "SELECT 1 FROM managed_call_notices \
                     WHERE parent_session_id = $1 AND completion_id = $2 \
                       AND state = 'enqueued' \
                       AND dispatch_started_at_ms IS NULL \
                       AND echo_seq IS NULL \
                       AND last_undelivered_key = $3 \
                       AND NOT EXISTS ( \
                           SELECT 1 FROM ", $echo_table, " \
                           WHERE e.session_id = $1 \
                             AND ", $echo_kind, " = 'update' \
                             AND ", $echo_update, " = 'user_message_chunk' \
                             AND ", $echo_cmid, " LIKE 'call-notice:%' \
                             AND ", $echo_cmid, " = $2 \
                       )",
                    $for_update
                );
                let mut released_any = false;
                for release in released_claims {
                    if release.reconcile_only {
                        let still_reconcilable: Option<(i32,)> = sqlx::query_as(reconcile_sql)
                            .bind(session_id)
                            .bind(&release.completion_id)
                            .bind(&release.receipt_key)
                            .fetch_optional(&mut *tx)
                            .await?;
                        if still_reconcilable.is_some() {
                            released_any = true;
                        }
                    } else {
                        let require_epoch: i64 = i64::from(release.require_worker_epoch_match);
                        let res = sqlx::query(release_sql)
                            .bind(session_id)
                            .bind(&release.completion_id)
                            .bind(release.expected_stamp)
                            .bind(release.expected_command_id.as_deref())
                            .bind(require_epoch)
                            .bind(release.expected_worker_epoch.as_deref())
                            .bind(&release.receipt_key)
                            .bind(now_ms)
                            .execute(&mut *tx)
                            .await?;
                        if res.rows_affected() > 0 {
                            released_any = true;
                        }
                    }
                }
                if require_claim_release && !released_any {
                    tx.commit().await?;
                    return Ok(false);
                }
                let query = sqlx::query($update_session_pending_sql)
                    .bind(&queue_json)
                    .bind(&drafts_json);
                let query = if $bind_now_for_pending {
                    query.bind(now_ms).bind(session_id)
                } else {
                    query.bind(session_id)
                };
                query
                    .execute(&mut *tx)
                    .await
                    .with_context(|| format!("UPDATE session pending {session_id}"))?;
                let settle_sql = concat!(
                    "UPDATE managed_call_notices \
                     SET state = 'unknown', reason = $3, \
                         dispatch_started_at_ms = NULL, \
                         dispatch_command_id = NULL, \
                         dispatch_worker_epoch = NULL, \
                         updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $4) \
                     WHERE parent_session_id = $1 AND completion_id = $2 \
                       AND (state = 'enqueued' OR (state = 'unknown' AND reason = 'dispatch_unconfirmed'))"
                );
                for (completion_id, reason) in settled_notices {
                    sqlx::query(settle_sql)
                        .bind(session_id)
                        .bind(completion_id)
                        .bind(reason)
                        .bind(now_ms)
                        .execute(&mut *tx)
                        .await?;
                }
                if let Some(cmids) = recovery_cmids {
                    let key = format!(
                        "{}{session_id}",
                        crate::model::settings_keys::SESSION_RECOVERY_CMIDS_PREFIX
                    );
                    let mut value = serde_json::json!(cmids);
                    crate::store::strip_nul(&mut value);
                    let setting_query = sqlx::query($upsert_setting_sql).bind(&key).bind(&value);
                    let setting_query = if $bind_now_for_pending {
                        setting_query.bind(now_ms)
                    } else {
                        setting_query
                    };
                    setting_query
                        .execute(&mut *tx)
                        .await
                        .with_context(|| format!("UPSERT recovery cmids setting {key}"))?;
                }
                tx.commit().await?;
                Ok(true)
            }
        }
    };
}

notice_impl!(
    PostgresStorage,
    sqlx::Postgres,
    "(e.payload ->> 'cmid')",
    "(e.payload ->> 'kind')",
    "(e.payload #>> '{update,sessionUpdate}')",
    "(e.payload #>> '{update,_meta,cowboyDelivery}')",
    "(p.payload ->> 'cmid')",
    "(p.payload ->> 'kind')",
    "(p.payload #>> '{update,sessionUpdate}')",
    "(p.payload #>> '{update,_meta,cowboyDelivery}')",
    "(e.payload ->> 'stop_reason')",
    "events AS e",
    "IS NOT DISTINCT FROM",
    "GREATEST",
    "UPDATE sessions SET queue = $1, drafts = $2, updated_at = now() WHERE id = $3",
    "INSERT INTO settings(key, value, updated_at) VALUES ($1, $2, now()) ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()",
    false,
    "BEGIN",
    " FOR UPDATE",
    true
);

notice_impl!(
    SqliteStorage,
    sqlx::Sqlite,
    "json_extract(e.payload, '$.cmid')",
    "json_extract(e.payload, '$.kind')",
    "json_extract(e.payload, '$.update.sessionUpdate')",
    "json_extract(e.payload, '$.update._meta.cowboyDelivery')",
    "json_extract(p.payload, '$.cmid')",
    "json_extract(p.payload, '$.kind')",
    "json_extract(p.payload, '$.update.sessionUpdate')",
    "json_extract(p.payload, '$.update._meta.cowboyDelivery')",
    "json_extract(e.payload, '$.stop_reason')",
    "events AS e INDEXED BY events_call_notice_echo_idx",
    "IS",
    "MAX",
    "UPDATE sessions SET queue = ?1, drafts = ?2, updated_at_ms = ?3 WHERE id = ?4",
    "INSERT INTO settings(key, value, updated_at_ms) VALUES (?1, ?2, ?3) ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at_ms = excluded.updated_at_ms",
    true,
    "BEGIN IMMEDIATE",
    "",
    false
);

#[cfg(test)]
mod tests;
