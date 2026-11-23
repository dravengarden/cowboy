//! Live transcript event and transcript-clear projection into `managed_call_notices`.

use super::{Result, is_completed_turn_stop_reason};
use crate::agent_model::{CALL_NOTICE_PREFIX, Event, Status};
use crate::model::Envelope;

/// Archive old Calls on transcript clear while keeping visible any Call whose
/// opted-in completion notice is unconfirmed OR any active Call whose typed
/// `request_notify_parent` column opted into parent completion notices.
pub(in crate::store) async fn clear_calls_on_transcript_clear_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    session_id: &str,
    calls_before_ms: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE managed_call_notices \
         SET state = 'unknown', reason = 'transcript_cleared', updated_at_ms = GREATEST(updated_at_ms + 1, $2) \
         WHERE parent_session_id = $1 AND state = 'accepted'",
    )
    .bind(session_id)
    .bind(calls_before_ms)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE managed_agent_calls SET dismissed = TRUE \
         WHERE parent_session_id = $1 \
           AND (document::jsonb->>'created_at_ms')::bigint < $2 \
           AND NOT ( \
               state NOT IN ('completed', 'failed', 'cancelled') \
               AND request_notify_parent = TRUE \
           ) \
           AND NOT EXISTS ( \
               SELECT 1 FROM managed_call_notices n \
               WHERE n.call_id = managed_agent_calls.call_id \
                 AND n.state != 'consumed' \
           )",
    )
    .bind(session_id)
    .bind(calls_before_ms)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(in crate::store) async fn clear_calls_on_transcript_clear_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session_id: &str,
    calls_before_ms: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE managed_call_notices \
         SET state = 'unknown', reason = 'transcript_cleared', updated_at_ms = MAX(updated_at_ms + 1, ?2) \
         WHERE parent_session_id = ?1 AND state = 'accepted'",
    )
    .bind(session_id)
    .bind(calls_before_ms)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE managed_agent_calls SET dismissed = TRUE \
         WHERE parent_session_id = ?1 \
           AND json_extract(document, '$.created_at_ms') < ?2 \
           AND NOT ( \
               state NOT IN ('completed', 'failed', 'cancelled') \
               AND request_notify_parent = 1 \
           ) \
           AND NOT EXISTS ( \
               SELECT 1 FROM managed_call_notices n \
               WHERE n.call_id = managed_agent_calls.call_id \
                 AND n.state != 'consumed' \
           )",
    )
    .bind(session_id)
    .bind(calls_before_ms)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn event_relevant_for_notices(envelope: &Envelope) -> bool {
    match &envelope.event {
        Event::Update { update } => {
            update
                .get("sessionUpdate")
                .and_then(serde_json::Value::as_str)
                == Some("user_message_chunk")
        }
        Event::TurnEnd { .. }
        | Event::Lifecycle {
            status: Status::Crashed | Status::Interrupted | Status::Exited,
            ..
        } => true,
        _ => false,
    }
}

macro_rules! project_events_fn {
    (
        $fn_name:ident,
        $user_fn:ident,
        $turn_end_fn:ident,
        $db:ty,
        $e_cmid:literal,
        $e_kind:literal,
        $e_update:literal,
        $e_delivery:literal,
        $p_cmid:literal,
        $p_kind:literal,
        $p_update:literal,
        $p_delivery:literal,
        $max_fn:literal,
        $is_pg:expr
    ) => {
        async fn $user_fn(
            tx: &mut sqlx::Transaction<'_, $db>,
            envelope: &Envelope,
            update: &serde_json::Value,
            seq: i64,
            now_ms: i64,
        ) -> Result<()> {
            // Steering injected into the current turn shares the active turn
            // boundary and must not supersede an accepted notice.
            let is_current_turn = update
                .pointer("/_meta/cowboyDelivery")
                .and_then(serde_json::Value::as_str)
                == Some("current_turn");
            if is_current_turn {
                return Ok(());
            }
            if let Some(cmid) = envelope
                .cmid
                .as_deref()
                .filter(|id| id.starts_with(CALL_NOTICE_PREFIX))
            {
                let supersede_sql = concat!(
                    "UPDATE managed_call_notices \
                     SET state = 'unknown', reason = 'superseded_by_user_turn', \
                         updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $4) \
                     WHERE parent_session_id = $1 AND state = 'accepted' \
                       AND echo_seq IS NOT NULL AND echo_seq < $2 \
                       AND completion_id != $3"
                );
                sqlx::query(supersede_sql)
                    .bind(&envelope.session_id)
                    .bind(seq)
                    .bind(cmid)
                    .bind(now_ms)
                    .execute(&mut **tx)
                    .await?;
                // Only accept `enqueued` notices or a late surviving echo for a
                // `dispatch_unconfirmed` row; never resurrect cancelled,
                // superseded, withdrawn, or cleared notices.
                let accept_sql = concat!(
                    "UPDATE managed_call_notices \
                     SET state = 'accepted', echo_seq = COALESCE(echo_seq, $3), \
                         reason = NULL, updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $4) \
                     WHERE parent_session_id = $1 AND completion_id = $2 \
                       AND (state = 'enqueued' OR (state = 'unknown' AND reason = 'dispatch_unconfirmed'))"
                );
                sqlx::query(accept_sql)
                    .bind(&envelope.session_id)
                    .bind(cmid)
                    .bind(seq)
                    .bind(now_ms)
                    .execute(&mut **tx)
                    .await?;
            } else if envelope.cmid.is_some() {
                let sql = concat!(
                    "UPDATE managed_call_notices \
                     SET state = 'unknown', reason = 'superseded_by_user_turn', \
                         updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $3) \
                     WHERE parent_session_id = $1 AND state = 'accepted' \
                       AND echo_seq IS NOT NULL AND echo_seq < $2"
                );
                sqlx::query(sql)
                    .bind(&envelope.session_id)
                    .bind(seq)
                    .bind(now_ms)
                    .execute(&mut **tx)
                    .await?;
            } else {
                // Contiguous untagged multi-block continuation chunks (such as a
                // rollover preamble followed by the notice text) belong to the
                // same turn and must not supersede their own notice.
                let sql = concat!(
                    "UPDATE managed_call_notices \
                     SET state = 'unknown', reason = 'superseded_by_user_turn', \
                         updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $3) \
                     WHERE parent_session_id = $1 AND state = 'accepted' \
                       AND echo_seq IS NOT NULL AND echo_seq < $2 \
                       AND EXISTS ( \
                           SELECT 1 FROM events AS p \
                           WHERE p.session_id = $1 \
                             AND p.seq > managed_call_notices.echo_seq \
                             AND p.seq < $2 \
                             AND NOT ( \
                                 ", $p_kind, " = 'update' \
                                 AND ", $p_update, " = 'user_message_chunk' \
                                 AND (", $p_cmid, " IS NULL OR ", $p_cmid, " = managed_call_notices.completion_id) \
                                 AND COALESCE(", $p_delivery, ", '') != 'current_turn' \
                             ) \
                       )"
                );
                sqlx::query(sql)
                    .bind(&envelope.session_id)
                    .bind(seq)
                    .bind(now_ms)
                    .execute(&mut **tx)
                    .await?;
            }
            Ok(())
        }

        async fn $turn_end_fn(
            tx: &mut sqlx::Transaction<'_, $db>,
            session_id: &str,
            seq: i64,
            stop_reason: &str,
            now_ms: i64,
        ) -> Result<()> {
            let completed = is_completed_turn_stop_reason(stop_reason);
            let (next_state, reason) = if completed {
                ("consumed", "turn_completed")
            } else {
                ("unknown", "turn_not_completed")
            };
            let settle_sql = concat!(
                "UPDATE managed_call_notices \
                 SET state = $3, reason = $4, updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $5) \
                 WHERE parent_session_id = $1 AND state = 'accepted' \
                   AND echo_seq IS NOT NULL AND echo_seq < $2"
            );
            sqlx::query(settle_sql)
                .bind(session_id)
                .bind(seq)
                .bind(next_state)
                .bind(reason)
                .bind(now_ms)
                .execute(&mut **tx)
                .await?;
            if !completed {
                return Ok(());
            }
            let candidates: Vec<(String, i64)> = sqlx::query_as(
                "SELECT completion_id, echo_seq FROM managed_call_notices \
                 WHERE parent_session_id = $1 AND state = 'unknown' \
                   AND reason IN ('worker_stopped', 'turn_unconfirmed') \
                   AND echo_seq IS NOT NULL AND echo_seq < $2",
            )
            .bind(session_id)
            .bind(seq)
            .fetch_all(&mut **tx)
            .await?;
            // Unified turn-boundary check shared with call_notice_echo_outcome:
            // ignore current_turn steering, repeated same-notice echoes, and
            // contiguous untagged continuation chunks.
            let check_sql = concat!(
                "SELECT EXISTS(SELECT 1 FROM events AS e WHERE e.session_id = $1 \
                 AND e.seq > $2 AND e.seq < $3 AND (",
                $e_kind,
                " = 'turn_end' OR (",
                $e_kind,
                " = 'update' AND ",
                $e_update,
                " = 'user_message_chunk' AND COALESCE(",
                $e_delivery,
                ", '') != 'current_turn' AND COALESCE(",
                $e_cmid,
                ", '') != $4 AND (",
                $e_cmid,
                " IS NOT NULL OR EXISTS ( \
                    SELECT 1 FROM events AS p WHERE p.session_id = e.session_id \
                    AND p.seq > $2 AND p.seq < e.seq AND NOT ( \
                        ", $p_kind, " = 'update' AND ", $p_update, " = 'user_message_chunk' \
                        AND (", $p_cmid, " IS NULL OR ", $p_cmid, " = $4) \
                        AND COALESCE(", $p_delivery, ", '') != 'current_turn' \
                    ))))))"
            );
            let promote_sql = concat!(
                "UPDATE managed_call_notices \
                 SET state = 'consumed', reason = 'turn_completed', \
                     updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $3) \
                 WHERE parent_session_id = $1 AND completion_id = $2 AND state != 'consumed'"
            );
            for (completion_id, echo_seq) in candidates {
                let (intervening,): (bool,) = sqlx::query_as(check_sql)
                    .bind(session_id)
                    .bind(echo_seq)
                    .bind(seq)
                    .bind(&completion_id)
                    .fetch_one(&mut **tx)
                    .await?;
                if !intervening {
                    sqlx::query(promote_sql)
                        .bind(session_id)
                        .bind(&completion_id)
                        .bind(now_ms)
                        .execute(&mut **tx)
                        .await?;
                }
            }
            Ok(())
        }

        pub(in crate::store) async fn $fn_name(
            tx: &mut sqlx::Transaction<'_, $db>,
            events: &[Envelope],
            now_ms: i64,
        ) -> Result<()> {
            let mut relevant: Vec<&Envelope> = events
                .iter()
                .filter(|envelope| event_relevant_for_notices(envelope))
                .collect();
            if relevant.is_empty() {
                return Ok(());
            }
            relevant.sort_by_key(|envelope| envelope.seq);
            if $is_pg {
                // Serialize with `adopt_legacy_call_notice_atomic` per parent
                // session so a concurrent legacy adoption cannot read `events`
                // before this batch's echoes become visible and insert a fresh
                // `enqueued` row after projection already ran.
                let sessions: std::collections::BTreeSet<&str> = relevant
                    .iter()
                    .map(|envelope| envelope.session_id.as_str())
                    .collect();
                for session_id in sessions {
                    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                        .bind(format!("call-notice-adopt:{session_id}"))
                        .execute(&mut **tx)
                        .await?;
                }
            }
            let stop_sql = concat!(
                "UPDATE managed_call_notices \
                 SET state = 'unknown', reason = 'worker_stopped', \
                     updated_at_ms = ", $max_fn, "(updated_at_ms + 1, $3) \
                 WHERE parent_session_id = $1 AND state = 'accepted' \
                   AND echo_seq IS NOT NULL AND echo_seq < $2"
            );
            for envelope in relevant {
                let seq = i64::try_from(envelope.seq)?;
                match &envelope.event {
                    Event::Update { update } => {
                        $user_fn(tx, envelope, update, seq, now_ms).await?;
                    }
                    Event::TurnEnd { stop_reason, .. } => {
                        $turn_end_fn(tx, &envelope.session_id, seq, stop_reason, now_ms).await?;
                    }
                    Event::Lifecycle {
                        status: Status::Crashed | Status::Interrupted | Status::Exited,
                        ..
                    } => {
                        sqlx::query(stop_sql)
                            .bind(&envelope.session_id)
                            .bind(seq)
                            .bind(now_ms)
                            .execute(&mut **tx)
                            .await?;
                    }
                    _ => {}
                }
            }
            Ok(())
        }
    };
}

project_events_fn!(
    project_events_pg,
    project_user_chunk_pg,
    project_turn_end_pg,
    sqlx::Postgres,
    "(e.payload ->> 'cmid')",
    "(e.payload ->> 'kind')",
    "(e.payload #>> '{update,sessionUpdate}')",
    "(e.payload #>> '{update,_meta,cowboyDelivery}')",
    "(p.payload ->> 'cmid')",
    "(p.payload ->> 'kind')",
    "(p.payload #>> '{update,sessionUpdate}')",
    "(p.payload #>> '{update,_meta,cowboyDelivery}')",
    "GREATEST",
    true
);

project_events_fn!(
    project_events_sqlite,
    project_user_chunk_sqlite,
    project_turn_end_sqlite,
    sqlx::Sqlite,
    "json_extract(e.payload, '$.cmid')",
    "json_extract(e.payload, '$.kind')",
    "json_extract(e.payload, '$.update.sessionUpdate')",
    "json_extract(e.payload, '$.update._meta.cowboyDelivery')",
    "json_extract(p.payload, '$.cmid')",
    "json_extract(p.payload, '$.kind')",
    "json_extract(p.payload, '$.update.sessionUpdate')",
    "json_extract(p.payload, '$.update._meta.cowboyDelivery')",
    "MAX",
    false
);
