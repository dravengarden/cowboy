//! Durable completion-notification lifecycle for opted-in Managed Calls
//! (Phase 2A of wait-cache observability).
//!
//! Coordinates dispatch fencing, startup reconciliation across Controller
//! restarts and native session rollover, and late Machine reconnect wakeups.
//! Live transcript event projection commits atomically inside the store's
//! `upsert_event_batch_with_anchors` transaction.

use super::super::AppState;
use crate::agent_model::{CALL_NOTICE_PREFIX, Status};
use crate::core::Hub;
use crate::store::{
    CallNoticeDelivery, DispatchClaim, LegacyAdoptionOutcome, NoticeDeliveryState,
    NoticeEchoEvidence, Store,
};

#[cfg(test)]
mod tests;

const RECOVERY_PAGE_SIZE: i64 = 200;
const MAX_CONFLICT_RETRIES: usize = 3;

/// Durable identity of one claimed `call-notice:` dispatch attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeDispatchToken {
    pub stamp: i64,
    pub command_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchGate {
    /// Proceed to hand the prompt to the worker, carrying the stamped claim
    /// token + command ID (when tracked in `managed_call_notices`) so
    /// non-delivery paths authenticate and release only this exact claim.
    Proceed(Option<NoticeDispatchToken>),
    /// Notice was already dispatched, accepted, consumed, or marked unknown.
    DropDuplicate,
    /// Durable state could not be updated; requeue without dropping.
    Requeue(String),
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn new_dispatch_command_id() -> String {
    format!("call-notice-cmd-{}", uuid::Uuid::new_v4())
}

/// Fence a `call-notice:` dispatch immediately before handing it to the worker.
/// Flushes any pending authoritative non-delivery release first, resolves any
/// prior ambiguous claim commit (Finding 5), then stamps `dispatch_started_at_ms`
/// and `dispatch_command_id` scoped to `(session_id, completion_id)`.
pub async fn before_notice_dispatch(
    state: &AppState,
    session_id: &str,
    cmid: Option<&str>,
) -> DispatchGate {
    before_notice_dispatch_in(&state.hub, state.store.as_ref(), session_id, cmid).await
}

async fn before_notice_dispatch_in(
    hub: &Hub,
    store: Option<&Store>,
    session_id: &str,
    cmid: Option<&str>,
) -> DispatchGate {
    let Some(completion_id) = cmid.filter(|id| id.starts_with(CALL_NOTICE_PREFIX)) else {
        return DispatchGate::Proceed(None);
    };
    let _commit_guard = hub.pending_commit_lock(session_id).await;
    if hub.is_notice_withdrawing(session_id, completion_id)
        || hub.is_result_relay_reserved(session_id, completion_id)
    {
        return DispatchGate::Requeue(format!(
            "{}completion notice settlement in progress",
            super::super::session_maintenance::WAIT_PREFIX
        ));
    }
    let Some(store) = store else {
        if hub
            .pending_result_consumption(session_id, completion_id)
            .is_some()
        {
            return DispatchGate::DropDuplicate;
        }
        return DispatchGate::Proceed(None);
    };
    if let Some(gate) =
        flush_pending_result_consumption_before_dispatch(hub, store, session_id, completion_id)
            .await
    {
        return gate;
    }
    let mut stamp = now_ms();
    if let Some((previous_stamp, previous_cmd)) =
        hub.take_pending_notice_release(session_id, completion_id)
    {
        if let Err(error) = store
            .clear_call_notice_dispatching(
                session_id,
                completion_id,
                Some(previous_stamp),
                Some(&previous_cmd),
                stamp,
            )
            .await
        {
            hub.restore_pending_notice_release(
                session_id,
                completion_id,
                previous_stamp,
                previous_cmd,
            );
            tracing::warn!(
                session_id,
                completion_id,
                %error,
                "deferring call completion notice dispatch while clearing prior non-delivery claim"
            );
            return DispatchGate::Requeue(format!(
                "{}durable completion notice store unavailable: {error}",
                super::super::session_maintenance::WAIT_PREFIX
            ));
        }
        stamp = stamp.max(previous_stamp.saturating_add(1));
    }
    let Some(_claim_guard) = hub.try_begin_notice_claim_guard(session_id, completion_id) else {
        return DispatchGate::Requeue(format!(
            "{}completion notice settlement in progress",
            super::super::session_maintenance::WAIT_PREFIX
        ));
    };
    // Round 2 #10: reuse any prior ambiguous `dispatch_command_id` directly in
    // `begin_call_notice_dispatch`, which is idempotent on `command_id` under
    // the SQL row lock (no unlocked SELECT readback racing an in-flight first tx).
    let (command_id, reusing_ambiguous) =
        match hub.peek_ambiguous_notice_claim(session_id, completion_id) {
            Some((ambiguous_cmd, _)) => (ambiguous_cmd, true),
            None => (new_dispatch_command_id(), false),
        };
    match store
        .begin_call_notice_dispatch(session_id, completion_id, &command_id, stamp)
        .await
    {
        Ok(DispatchClaim::Claimed(claimed)) => {
            finish_notice_claim_or_release(
                hub,
                store,
                session_id,
                completion_id,
                claimed,
                command_id,
            )
            .await
        }
        Ok(DispatchClaim::Untracked) => {
            if reusing_ambiguous {
                hub.clear_ambiguous_notice_claim(session_id, completion_id);
            }
            adopt_legacy_queued_notice_before_dispatch(
                hub,
                store,
                session_id,
                completion_id,
                command_id,
                stamp,
            )
            .await
        }
        Ok(DispatchClaim::AlreadyHandled(existing)) => {
            if reusing_ambiguous {
                hub.clear_ambiguous_notice_claim(session_id, completion_id);
            }
            tracing::debug!(
                session_id,
                completion_id,
                state = existing.code(),
                "skipping duplicate call completion notice dispatch"
            );
            DispatchGate::DropDuplicate
        }
        Err(error) => {
            hub.record_ambiguous_notice_claim(session_id, completion_id, command_id, stamp);
            tracing::warn!(
                session_id,
                completion_id,
                %error,
                "deferring call completion notice dispatch because durable store is unavailable"
            );
            DispatchGate::Requeue(format!(
                "{}durable completion notice store unavailable: {error}",
                super::super::session_maintenance::WAIT_PREFIX
            ))
        }
    }
}

/// Atomically transition a SQL `Claimed` result into an active in-memory claim,
/// or release the SQL claim and requeue if a `calls.result` relay reservation,
/// pending result consumption, or withdrawal claimed ownership concurrently.
async fn finish_notice_claim_or_release(
    hub: &Hub,
    store: &Store,
    session_id: &str,
    completion_id: &str,
    claimed: i64,
    command_id: String,
) -> DispatchGate {
    if hub.try_record_notice_claim(session_id, completion_id, claimed, command_id.clone()) {
        return DispatchGate::Proceed(Some(NoticeDispatchToken {
            stamp: claimed,
            command_id,
        }));
    }
    if let Err(error) = store
        .clear_call_notice_dispatching(
            session_id,
            completion_id,
            Some(claimed),
            Some(&command_id),
            now_ms(),
        )
        .await
    {
        hub.restore_pending_notice_release(session_id, completion_id, claimed, command_id);
        tracing::warn!(
            session_id,
            completion_id,
            %error,
            "deferred clearing raced completion notice claim after result settlement fence"
        );
    }
    DispatchGate::Requeue(format!(
        "{}completion notice settlement in progress",
        super::super::session_maintenance::WAIT_PREFIX
    ))
}

/// Adopt a verified pre-2A queued completion notice into `managed_call_notices`
/// before dispatch, checking Hub's in-memory transcript log first and then
/// running `adopt_legacy_call_notice_atomic` in one serialized SQL transaction
/// (Round 2 #11).
async fn adopt_legacy_queued_notice_before_dispatch(
    hub: &Hub,
    store: &Store,
    session_id: &str,
    completion_id: &str,
    command_id: String,
    stamp: i64,
) -> DispatchGate {
    if hub.has_echoed_cmid(session_id, completion_id) {
        return DispatchGate::DropDuplicate;
    }
    let Some(call_id) = completion_id.strip_prefix(CALL_NOTICE_PREFIX) else {
        return DispatchGate::Proceed(None);
    };
    let record = match store.managed_call(session_id, call_id).await {
        Ok(Some(record))
            if record.state.terminal() && super::wait_observation::wants_notice(&record) =>
        {
            record
        }
        Ok(_) => return DispatchGate::Proceed(None),
        Err(error) => {
            return DispatchGate::Requeue(format!(
                "{}durable completion notice lookup unavailable: {error}",
                super::super::session_maintenance::WAIT_PREFIX
            ));
        }
    };
    // Round 3 #12: re-check Hub's in-memory transcript log AFTER the async
    // ledger lookup so an echo that arrived while awaiting `managed_call`
    // vetoes adoption before writing an `enqueued` claim.
    if hub.has_echoed_cmid(session_id, completion_id) {
        return DispatchGate::DropDuplicate;
    }
    match store
        .adopt_legacy_call_notice_atomic(
            session_id,
            &record.call_id,
            completion_id,
            &command_id,
            stamp,
        )
        .await
    {
        Ok(LegacyAdoptionOutcome::Claimed(claimed)) => {
            // Round 3 #12: if an echo arrived in Hub while the SQL adoption
            // transaction ran (still buffered in the FIFO store writer), veto
            // handoff immediately and release the fresh dispatch stamp so the
            // buffered event batch promotes the row without a duplicate turn.
            if hub.has_echoed_cmid(session_id, completion_id) {
                let _ = store
                    .clear_call_notice_dispatching(
                        session_id,
                        completion_id,
                        Some(claimed),
                        Some(&command_id),
                        now_ms(),
                    )
                    .await;
                return DispatchGate::DropDuplicate;
            }
            finish_notice_claim_or_release(
                hub,
                store,
                session_id,
                completion_id,
                claimed,
                command_id,
            )
            .await
        }
        Ok(LegacyAdoptionOutcome::SettledFromEvidence) => DispatchGate::DropDuplicate,
        Ok(LegacyAdoptionOutcome::AlreadyExists) => {
            match store
                .begin_call_notice_dispatch(session_id, completion_id, &command_id, stamp)
                .await
            {
                Ok(DispatchClaim::Claimed(claimed)) => {
                    if hub.has_echoed_cmid(session_id, completion_id) {
                        let _ = store
                            .clear_call_notice_dispatching(
                                session_id,
                                completion_id,
                                Some(claimed),
                                Some(&command_id),
                                now_ms(),
                            )
                            .await;
                        return DispatchGate::DropDuplicate;
                    }
                    finish_notice_claim_or_release(
                        hub,
                        store,
                        session_id,
                        completion_id,
                        claimed,
                        command_id,
                    )
                    .await
                }
                Ok(DispatchClaim::AlreadyHandled(_)) => DispatchGate::DropDuplicate,
                Ok(DispatchClaim::Untracked) => DispatchGate::Proceed(None),
                Err(error) => {
                    hub.record_ambiguous_notice_claim(session_id, completion_id, command_id, stamp);
                    DispatchGate::Requeue(format!(
                        "{}durable completion notice store unavailable: {error}",
                        super::super::session_maintenance::WAIT_PREFIX
                    ))
                }
            }
        }
        Err(error) => {
            hub.record_ambiguous_notice_claim(session_id, completion_id, command_id, stamp);
            DispatchGate::Requeue(format!(
                "{}durable completion notice adoption unavailable: {error}",
                super::super::session_maintenance::WAIT_PREFIX
            ))
        }
    }
}

/// Release the exact dispatch stamp and command ID when handing the prompt to
/// the worker failed synchronously before leaving the Controller.
pub async fn after_notice_dispatch_error(
    state: &AppState,
    session_id: &str,
    cmid: Option<&str>,
    claim_token: Option<&NoticeDispatchToken>,
) {
    after_notice_dispatch_error_in(
        &state.hub,
        state.store.as_ref(),
        session_id,
        cmid,
        claim_token,
    )
    .await;
}

async fn after_notice_dispatch_error_in(
    hub: &Hub,
    store: Option<&Store>,
    session_id: &str,
    cmid: Option<&str>,
    claim_token: Option<&NoticeDispatchToken>,
) {
    let Some(completion_id) = cmid.filter(|id| id.starts_with(CALL_NOTICE_PREFIX)) else {
        return;
    };
    hub.clear_notice_claim(session_id, completion_id);
    let Some(store) = store else {
        return;
    };
    let Some(token) = claim_token else {
        return;
    };
    if let Err(error) = store
        .clear_call_notice_dispatching(
            session_id,
            completion_id,
            Some(token.stamp),
            Some(&token.command_id),
            now_ms(),
        )
        .await
    {
        // Retain the exact `(stamp, command_id)` so the next drain clears it before re-claiming.
        hub.restore_pending_notice_release(
            session_id,
            completion_id,
            token.stamp,
            token.command_id.clone(),
        );
        tracing::warn!(
            session_id,
            completion_id,
            %error,
            "failed to clear call completion notice dispatch stamp after send failure"
        );
    }
}

/// Filter out already-settled (`consumed` or `unknown`) `call-notice:` rows
/// from restored session queues before the Hub or dispatcher ever sees them,
/// and persist the pruned queue so a subsequent restart stays clean.
pub async fn prune_settled_restored_notices(
    store: &Store,
    restored: &mut [crate::core::RestoredSession],
) {
    for session in restored {
        let mut retained = Vec::with_capacity(session.queue.len());
        let mut changed = false;
        for msg in session.queue.drain(..) {
            let drop_settled = if let Some(cmid) = msg
                .cmid
                .as_deref()
                .filter(|id| id.starts_with(CALL_NOTICE_PREFIX))
            {
                matches!(
                    store
                        .call_notice_by_parent_completion_id(&session.meta.id, cmid)
                        .await,
                    Ok(Some(delivery))
                        if matches!(
                            delivery.state,
                            NoticeDeliveryState::Consumed | NoticeDeliveryState::Unknown
                        )
                )
            } else {
                false
            };
            if drop_settled {
                changed = true;
            } else {
                retained.push(msg);
            }
        }
        session.queue = retained;
        // Legacy Controllers could park a notice in drafts with its reserved
        // `call-notice:` cmid intact; strip it on restore so draft activation
        // submits it as an ordinary user prompt rather than dropping it.
        for draft in &mut session.drafts {
            if draft
                .cmid
                .as_deref()
                .is_some_and(crate::agent_model::is_reserved_call_notice_cmid)
            {
                draft.cmid = None;
                changed = true;
            }
        }
        if changed {
            let _ = store
                .update_pending(&session.meta.id, &session.queue, &session.drafts)
                .await;
        }
    }
}

/// Reconcile and recover unsettled completion notices after Controller start.
pub async fn recover(state: &AppState, store: &Store) {
    recover_in(&state.hub, store).await;
}

pub async fn recover_in(hub: &Hub, store: &Store) {
    // 1. Prune already-settled notices lingering in restored Hub queues before dispatch.
    //    Hold the per-session commit lock across delivery lookup and exact row_id
    //    removal so a concurrent `UndeliveredPrompt` reopen can never be deleted.
    for (parent, completion_id) in hub.queued_call_notices() {
        let _commit_guard = hub.pending_commit_lock(&parent).await;
        if hub.is_notice_withdrawing(&parent, &completion_id)
            || hub.is_notice_claiming(&parent, &completion_id)
        {
            continue;
        }
        let Some(row_id) = hub.queued_row_id_by_cmid(&parent, &completion_id) else {
            continue;
        };
        if let Ok(Some(delivery)) = store
            .call_notice_by_parent_completion_id(&parent, &completion_id)
            .await
            && matches!(
                delivery.state,
                NoticeDeliveryState::Consumed | NoticeDeliveryState::Unknown
            )
        {
            let _ = hub.remove_queued_by_id_if_cmid(&parent, &row_id, &completion_id);
        }
    }

    // 2. Keyset-page through all unsettled and promotable-unknown notices so
    //    older paused/held parents can never starve later notices.
    let mut cursor: Option<(i64, String)> = None;
    loop {
        let after = cursor.as_ref().map(|(ts, id)| (*ts, id.as_str()));
        let page = match store
            .unsettled_call_notices_page(after, RECOVERY_PAGE_SIZE)
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(%error, "managed call notice recovery unavailable");
                return;
            }
        };
        let page_len = page.len();
        if let Some(last) = page.last() {
            cursor = Some((last.created_at_ms, last.completion_id.clone()));
        }
        for notice in page {
            reconcile_one_notice(hub, store, notice).await;
        }
        if i64::try_from(page_len).unwrap_or(0) < RECOVERY_PAGE_SIZE {
            break;
        }
    }
}

async fn reconcile_one_notice(hub: &Hub, store: &Store, mut notice: CallNoticeDelivery) {
    for _ in 0..MAX_CONFLICT_RETRIES {
        if hub.is_notice_withdrawing(&notice.parent_session_id, &notice.completion_id)
            || hub.is_notice_claiming(&notice.parent_session_id, &notice.completion_id)
        {
            return;
        }
        let now = now_ms();
        let evidence = match store
            .call_notice_echo_outcome(&notice.parent_session_id, &notice.completion_id)
            .await
        {
            Ok(evidence) => evidence,
            Err(error) => {
                tracing::warn!(
                    completion_id = %notice.completion_id,
                    %error,
                    "failed to inspect durable echo for unsettled call notice"
                );
                return;
            }
        };
        let conflicted = match evidence {
            NoticeEchoEvidence::Echoed {
                echo_seq,
                turn_completed: Some(true),
                ..
            } => {
                let _ = store
                    .mark_call_notice_consumed(
                        &notice.parent_session_id,
                        &notice.completion_id,
                        "turn_completed",
                        Some(echo_seq),
                        now,
                    )
                    .await;
                let _ = hub.remove_queued_by_cmid(&notice.parent_session_id, &notice.completion_id);
                false
            }
            NoticeEchoEvidence::Echoed {
                echo_seq,
                turn_completed: Some(false),
                ..
            } => {
                if notice.state == NoticeDeliveryState::Unknown {
                    let _ =
                        hub.remove_queued_by_cmid(&notice.parent_session_id, &notice.completion_id);
                    false
                } else {
                    transition_unknown_if(
                        hub,
                        store,
                        &notice,
                        "turn_not_completed",
                        Some(echo_seq),
                        now,
                        true,
                    )
                    .await
                }
            }
            NoticeEchoEvidence::Echoed {
                echo_seq,
                turn_completed: None,
                superseded_by_user_turn: true,
            } => {
                if notice.state == NoticeDeliveryState::Unknown {
                    let _ =
                        hub.remove_queued_by_cmid(&notice.parent_session_id, &notice.completion_id);
                    false
                } else {
                    transition_unknown_if(
                        hub,
                        store,
                        &notice,
                        "superseded_by_user_turn",
                        Some(echo_seq),
                        now,
                        true,
                    )
                    .await
                }
            }
            NoticeEchoEvidence::Echoed {
                echo_seq,
                turn_completed: None,
                superseded_by_user_turn: false,
            } => {
                if notice.state == NoticeDeliveryState::Unknown {
                    false
                } else if matches!(
                    hub.status(&notice.parent_session_id),
                    Some(Status::Busy | Status::Starting)
                ) {
                    transition_accepted_if(hub, store, &notice, echo_seq, now).await
                } else {
                    transition_unknown_if(
                        hub,
                        store,
                        &notice,
                        "turn_unconfirmed",
                        Some(echo_seq),
                        now,
                        true,
                    )
                    .await
                }
            }
            NoticeEchoEvidence::Absent => reconcile_absent_notice(hub, store, &notice, now).await,
        };
        if !conflicted {
            return;
        }
        match store
            .call_notice_by_parent_completion_id(&notice.parent_session_id, &notice.completion_id)
            .await
        {
            Ok(Some(refreshed)) if refreshed.state != NoticeDeliveryState::Consumed => {
                notice = refreshed;
            }
            _ => return,
        }
    }
}

async fn reconcile_absent_notice(
    hub: &Hub,
    store: &Store,
    notice: &CallNoticeDelivery,
    now: i64,
) -> bool {
    if notice.state == NoticeDeliveryState::Unknown {
        return false;
    }
    if notice.state == NoticeDeliveryState::Accepted {
        return transition_unknown_if(
            hub,
            store,
            notice,
            "transcript_cleared",
            notice.echo_seq,
            now,
            false,
        )
        .await;
    }
    if let Some(stamp) = notice.dispatch_started_at_ms {
        if hub.is_notice_claiming(&notice.parent_session_id, &notice.completion_id)
            || hub.owns_notice_claim(
                &notice.parent_session_id,
                &notice.completion_id,
                stamp,
                notice.dispatch_command_id.as_deref(),
            )
        {
            return false;
        }
        // Handed to a worker before restart without a durable echo: fail
        // closed to `unknown` conditionally on the exact observed state so a
        // concurrently accepted echo is never overwritten.
        return transition_unknown_if(hub, store, notice, "dispatch_unconfirmed", None, now, true)
            .await;
    }
    if hub
        .session_info(&notice.parent_session_id)
        .is_none_or(|info| info.meta.closing)
    {
        return transition_unknown_if(hub, store, notice, "parent_unavailable", None, now, false)
            .await;
    }
    if hub.has_queued_cmid(&notice.parent_session_id, &notice.completion_id) {
        let _ = hub.wake_queued_call_notice(&notice.parent_session_id, &notice.completion_id);
    } else if let Ok(Some(record)) = store
        .managed_call(&notice.parent_session_id, &notice.call_id)
        .await
        && record.state.terminal()
    {
        super::runner::outcome::notify_parent_in(hub, Some(store), &record).await;
    }
    false
}

/// Apply a conditional recovery transition to `accepted`. Returns `true` when
/// the row changed concurrently so the caller refreshes and reconciles again.
async fn transition_accepted_if(
    hub: &Hub,
    store: &Store,
    notice: &CallNoticeDelivery,
    echo_seq: u64,
    now: i64,
) -> bool {
    let _commit_guard = hub.pending_commit_lock(&notice.parent_session_id).await;
    if hub.is_notice_withdrawing(&notice.parent_session_id, &notice.completion_id)
        || hub.is_notice_claiming(&notice.parent_session_id, &notice.completion_id)
    {
        return false;
    }
    let row_id = hub.queued_row_id_by_cmid(&notice.parent_session_id, &notice.completion_id);
    match store
        .mark_call_notice_accepted_if(
            &notice.parent_session_id,
            &notice.completion_id,
            notice.state,
            notice.dispatch_started_at_ms,
            notice.echo_seq,
            echo_seq,
            now,
        )
        .await
    {
        Ok(true) => {
            if let Some(id) = row_id.as_deref() {
                let _ = hub.remove_queued_by_id_if_cmid(
                    &notice.parent_session_id,
                    id,
                    &notice.completion_id,
                );
            }
            false
        }
        Ok(false) => true,
        Err(error) => {
            tracing::warn!(
                completion_id = %notice.completion_id,
                %error,
                "conditional recovery acceptance failed"
            );
            false
        }
    }
}

/// Apply a conditional recovery transition to `unknown`. Returns `true` when
/// the row changed concurrently so the caller refreshes and reconciles again.
async fn transition_unknown_if(
    hub: &Hub,
    store: &Store,
    notice: &CallNoticeDelivery,
    reason: &str,
    new_echo_seq: Option<u64>,
    now: i64,
    remove_queued_on_success: bool,
) -> bool {
    let _commit_guard = hub.pending_commit_lock(&notice.parent_session_id).await;
    if hub.is_notice_withdrawing(&notice.parent_session_id, &notice.completion_id)
        || hub.is_notice_claiming(&notice.parent_session_id, &notice.completion_id)
    {
        return false;
    }
    if reason == "dispatch_unconfirmed"
        && let Some(stamp) = notice.dispatch_started_at_ms
        && hub.owns_notice_claim(
            &notice.parent_session_id,
            &notice.completion_id,
            stamp,
            notice.dispatch_command_id.as_deref(),
        )
    {
        return false;
    }
    let row_id = remove_queued_on_success
        .then(|| hub.queued_row_id_by_cmid(&notice.parent_session_id, &notice.completion_id))
        .flatten();
    match store
        .mark_call_notice_unknown_if(
            &notice.parent_session_id,
            &notice.completion_id,
            notice.state,
            notice.dispatch_started_at_ms,
            notice.echo_seq,
            reason,
            new_echo_seq,
            now,
        )
        .await
    {
        Ok(true) => {
            if let Some(id) = row_id.as_deref() {
                let _ = hub.remove_queued_by_id_if_cmid(
                    &notice.parent_session_id,
                    id,
                    &notice.completion_id,
                );
            }
            false
        }
        Ok(false) => true,
        Err(error) => {
            tracing::warn!(
                completion_id = %notice.completion_id,
                %error,
                "conditional recovery transition failed"
            );
            false
        }
    }
}

/// Wake dormant (`Exited`) parents whose queue head is an unsettled
/// `call-notice:` targeting a Machine whose runtime just connected.
pub async fn wake_notices_for_machine(hub: &Hub, store: Option<&Store>, machine_id: &str) {
    for meta in hub.session_list() {
        if meta.status != Status::Exited {
            continue;
        }
        let target_matches = meta.machine_id == machine_id
            || meta
                .execution_binding
                .as_ref()
                .and_then(|binding| binding.decode().ok())
                .is_some_and(|binding| binding.environment.machine_id == machine_id);
        if !target_matches {
            continue;
        }
        let Some(completion_id) = hub.queue_head_call_notice(&meta.id) else {
            continue;
        };
        if hub.is_result_relay_reserved(&meta.id, &completion_id) {
            continue;
        }
        if let Some(call_id) = hub.pending_result_consumption(&meta.id, &completion_id) {
            if let Some(store) = store
                && store
                    .mark_call_notice_consumed_for_result_read(
                        &meta.id,
                        &call_id,
                        &completion_id,
                        now_ms(),
                    )
                    .await
                    .is_ok()
            {
                let _commit_guard = hub.pending_commit_lock(&meta.id).await;
                hub.clear_pending_result_consumption(&meta.id, &completion_id);
                if let Some(row_id) = hub.queued_row_id_by_cmid(&meta.id, &completion_id) {
                    let _ = hub.remove_queued_by_id_if_cmid(&meta.id, &row_id, &completion_id);
                }
            }
            continue;
        }
        let has_pending_release = hub.has_pending_notice_release(&meta.id, &completion_id)
            || hub.has_ambiguous_notice_claim(&meta.id, &completion_id);
        if let Some(store) = store
            && let Ok(Some(delivery)) = store
                .call_notice_by_parent_completion_id(&meta.id, &completion_id)
                .await
            && (delivery.state != NoticeDeliveryState::Enqueued
                || (delivery.dispatch_started_at_ms.is_some() && !has_pending_release))
        {
            continue;
        }
        let _ = hub.wake_queued_call_notice(&meta.id, &completion_id);
    }
}

/// Gateway `Action::Result` handler (keeps `coordinator.rs` within structure
/// budget). Direct Machine gateway `Action::Result` is intentionally
/// read-only (non-consuming): without a wire-level post-delivery receipt from
/// the local CLI caller, consuming the completion notice when buffering onto
/// the WebSocket channel could lose a notice on transport drop (Round 2 #3).
/// Callers reading via `cowboy.calls.result(...)` (`cowboy_read`) settle
/// `consumed("result_read")` after authorized sandbox write+flush.
pub async fn dispatch_result_action(
    state: &AppState,
    ledger: &crate::managed_calls::service::Ledger,
    call_id: &str,
) -> serde_json::Value {
    match ledger.inspect(call_id).await {
        Ok(Some(record)) if record.state.terminal() => {
            let mut value = super::runner::outcome::envelope_with_notice(state, &record).await;
            value["result"] = record.result.clone().unwrap_or(serde_json::Value::Null);
            value
        }
        Ok(Some(record)) => {
            let mut value = super::runner::outcome::envelope_with_notice(state, &record).await;
            value["state"] = serde_json::json!("error");
            value["error"] = serde_json::json!({
                "code": "not_ready",
                "admission": "accepted",
                "call_state": record.state.code(),
            });
            value
        }
        Ok(None) => serde_json::json!({
            "schema": 1,
            "state": "error",
            "error": { "code": "not_found", "admission": "not_submitted" },
        }),
        Err(_) => super::coordinator::ledger_unavailable(),
    }
}

/// Stage a terminal Call result for broker relay: validate windowing, attach
/// the current `notice_delivery`, and reserve exact wire bytes against `budget`
/// WITHOUT committing durable notice consumption yet. Consumption runs in
/// [`commit_windowed_result`] ONLY AFTER final authorization and sandbox write
/// succeed (Finding 4).
pub async fn prepare_windowed_result<F>(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    ledger: &crate::managed_calls::service::Ledger,
    call_id: &str,
    budget: &cowboy_codeact_runtime::CallBudget<'_>,
    window: F,
) -> Result<serde_json::Value, String>
where
    F: FnOnce(&mut serde_json::Value) -> Result<(), String>,
{
    if super::super::superagent::restricted(state, meta).await? {
        return Err("superagent_scope_denied".to_owned());
    }
    let (value, reservation) =
        prepare_windowed_result_in(state.store.as_ref(), ledger, call_id, Some(budget), window)
            .await?;
    if let Some(reservation) = reservation {
        reservation.commit();
    }
    Ok(value)
}

/// Validate and window a terminal Call result before durably marking its
/// completion notice consumed, so a refused read (such as
/// `text_window_required`) never withdraws the notice and a failed durable
/// receipt never acknowledges consumption.
pub async fn read_windowed_result<F>(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    ledger: &crate::managed_calls::service::Ledger,
    call_id: &str,
    budget: Option<&cowboy_codeact_runtime::CallBudget<'_>>,
    window: F,
) -> Result<serde_json::Value, String>
where
    F: FnOnce(&mut serde_json::Value) -> Result<(), String>,
{
    if super::super::superagent::restricted(state, meta).await? {
        return Err("superagent_scope_denied".to_owned());
    }
    read_windowed_result_in(
        &state.hub,
        state.store.as_ref(),
        ledger,
        call_id,
        budget,
        window,
    )
    .await
}

pub async fn read_windowed_result_in<F>(
    hub: &Hub,
    store: Option<&Store>,
    ledger: &crate::managed_calls::service::Ledger,
    call_id: &str,
    budget: Option<&cowboy_codeact_runtime::CallBudget<'_>>,
    window: F,
) -> Result<serde_json::Value, String>
where
    F: FnOnce(&mut serde_json::Value) -> Result<(), String>,
{
    let (mut value, reservation) =
        prepare_windowed_result_in(store, ledger, call_id, budget, window).await?;
    if value.get("state").and_then(serde_json::Value::as_str) == Some("error") {
        return Ok(value);
    }
    if budget.is_some_and(cowboy_codeact_runtime::CallBudget::is_closed) {
        return Err("budget_exceeded".to_owned());
    }
    let record = match ledger.inspect(call_id).await {
        Ok(Some(record)) if record.state.terminal() => record,
        Ok(Some(_)) => return Ok(value),
        Ok(None) => return Err("not_found".to_owned()),
        Err(_) => return Err("unavailable".to_owned()),
    };
    commit_windowed_result_in(hub, store, &record).await?;
    if super::wait_observation::wants_notice(&record)
        && let Some(store) = store
        && let Ok(Some(delivery)) = store
            .call_notice_delivery(&record.placement.parent_session_id, &record.call_id)
            .await
    {
        value["notice_delivery"] = serde_json::json!(delivery);
    }
    if let Some(reservation) = reservation {
        reservation.commit();
    }
    Ok(value)
}

async fn prepare_windowed_result_in<'a, F>(
    store: Option<&Store>,
    ledger: &crate::managed_calls::service::Ledger,
    call_id: &str,
    budget: Option<&'a cowboy_codeact_runtime::CallBudget<'a>>,
    window: F,
) -> Result<
    (
        serde_json::Value,
        Option<cowboy_codeact_runtime::ReplyReservation<'a>>,
    ),
    String,
>
where
    F: FnOnce(&mut serde_json::Value) -> Result<(), String>,
{
    match ledger.inspect(call_id).await {
        Ok(Some(record)) if record.state.terminal() => {
            let mut result = record.result.clone().unwrap_or(serde_json::Value::Null);
            window(&mut result)?;
            let mut value = super::coordinator::envelope(&record);
            value["result"] = result;
            if let Some(object) = value.as_object_mut() {
                object.remove("schema");
                object.remove("next");
            }
            if super::wait_observation::wants_notice(&record)
                && let Some(store) = store
                && let Ok(Some(delivery)) = store
                    .call_notice_delivery(&record.placement.parent_session_id, &record.call_id)
                    .await
            {
                value["notice_delivery"] = serde_json::json!(delivery);
            }
            let wrapped = serde_json::json!({ "id": u64::MAX, "ok": &value });
            let wire_bytes = serde_json::to_vec(&wrapped)
                .map(|bytes| bytes.len().saturating_add(1))
                .unwrap_or(usize::MAX);
            let reservation = if let Some(budget) = budget {
                Some(budget.reserve(wire_bytes).map_err(str::to_owned)?)
            } else {
                if wire_bytes.saturating_sub(1) > crate::agent_api::MAX_REPLY_BYTES {
                    return Err("reply_too_large".to_owned());
                }
                None
            };
            Ok((value, reservation))
        }
        Ok(Some(record)) => {
            let mut value = super::coordinator::envelope(&record);
            if super::wait_observation::wants_notice(&record)
                && let Some(store) = store
                && let Ok(Some(delivery)) = store
                    .call_notice_delivery(&record.placement.parent_session_id, &record.call_id)
                    .await
            {
                value["notice_delivery"] = serde_json::json!(delivery);
            }
            value["state"] = serde_json::json!("error");
            value["error"] = serde_json::json!({
                "code": "not_ready",
                "admission": "accepted",
                "call_state": record.state.code(),
            });
            if let Some(object) = value.as_object_mut() {
                object.remove("schema");
                object.remove("next");
            }
            Ok((value, None))
        }
        Ok(None) => Err("not_found".to_owned()),
        Err(_) => Err("unavailable".to_owned()),
    }
}

async fn flush_pending_result_consumption_before_dispatch(
    hub: &Hub,
    store: &Store,
    session_id: &str,
    completion_id: &str,
) -> Option<DispatchGate> {
    let call_id = hub.pending_result_consumption(session_id, completion_id)?;
    match store
        .mark_call_notice_consumed_for_result_read(session_id, &call_id, completion_id, now_ms())
        .await
    {
        Ok(()) => {
            hub.clear_pending_result_consumption(session_id, completion_id);
            Some(DispatchGate::DropDuplicate)
        }
        Err(error) => {
            tracing::warn!(
                session_id,
                completion_id,
                %error,
                "deferring call completion notice while flushing deferred result_read consumption"
            );
            Some(DispatchGate::Requeue(format!(
                "{}durable completion notice store unavailable: {error}",
                super::super::session_maintenance::WAIT_PREFIX
            )))
        }
    }
}

/// Spawn bounded post-relay `result_read` settlement retries tracked on `Hub` so
/// settlement survives beyond the CodeAct broker deadline and drains cleanly on
/// shutdown (Round 3 #5).
pub fn spawn_commit_windowed_result(
    state: &std::sync::Arc<AppState>,
    parent_session_id: &str,
    call_id: &str,
    completion_id: &str,
) -> tokio::sync::oneshot::Receiver<Result<(), ()>> {
    let hub = state.hub.clone();
    let store = state.store.clone();
    let parent_owned = parent_session_id.to_owned();
    let call_id_owned = call_id.to_owned();
    let completion_owned = completion_id.to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    state.hub.spawn_teardown_requeue(async move {
        let mut last_err = None;
        for (attempt, delay_ms) in [0_u64, 50, 100, 200].into_iter().enumerate() {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            let settled = if let Some(store) = store.as_ref() {
                match store.managed_call(&parent_owned, &call_id_owned).await {
                    Ok(Some(record)) if record.state.terminal() => {
                        commit_windowed_result_in(&hub, Some(store), &record).await
                    }
                    Ok(Some(_) | None) => Ok(()),
                    Err(err) => Err(err.to_string()),
                }
            } else {
                Ok(())
            };
            match settled {
                Ok(()) => {
                    hub.clear_pending_result_consumption(&parent_owned, &completion_owned);
                    let _ = tx.send(Ok(()));
                    return;
                }
                Err(err) => last_err = Some(err),
            }
        }
        tracing::error!(
            parent_session_id = %parent_owned,
            call_id = %call_id_owned,
            error = ?last_err,
            "failed to durably mark completion notice consumed after relaying calls.result; fenced in Hub for retry before dispatch"
        );
        let _ = tx.send(Err(()));
    });
    rx
}

async fn commit_windowed_result_in(
    hub: &Hub,
    store: Option<&Store>,
    record: &crate::managed_calls::lifecycle::Record,
) -> Result<(), String> {
    super::runner::outcome::on_result_read_in(hub, store, record)
        .await
        .map_err(|_| "unavailable".to_owned())
}
