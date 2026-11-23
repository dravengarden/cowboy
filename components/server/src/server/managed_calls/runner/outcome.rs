//! One durable-journal event per finished review, so routing and reviewer
//! cost and outcome can be analysed later (docs/review-routing.md). The ledger
//! keeps the full record; this carries counts and identifiers only, never
//! instructions, findings text or paths.

use crate::managed_calls::lifecycle::{Record, State};

pub(in crate::server::managed_calls) fn record(record: &Record) {
    if !record.purpose.is_review() || !record.state.terminal() {
        return;
    }
    let result = record.result.as_ref();
    let usage = result.map(|result| &result["usage"]);
    let tokens = |key: &str| {
        usage
            .and_then(|usage| usage[key].as_u64())
            .unwrap_or_default()
    };
    let structured = result.map(|result| &result["structured"]);
    let findings = structured.and_then(|structured| structured["findings"].as_array());
    let severity = |level: &str| {
        findings.map(|all| all.iter().filter(|f| f["severity"] == level).count() as u64)
    };
    // Only the standard verdicts: a custom schema may put prose there.
    let verdict = structured
        .and_then(|s| s["verdict"].as_str())
        .map(|verdict| match verdict {
            "approve" | "needs-attention" | "incomplete" => verdict,
            _ => "other",
        });
    let configuration = record.configuration.as_ref();
    let option = |key: &str| {
        configuration
            .and_then(|values| values.get(key))
            .map_or("", String::as_str)
    };
    let duration_ms = record
        .started_at_ms
        .zip(record.finished_at_ms)
        .map(|(started, finished)| (finished - started).max(0));
    tracing::info!(
        event_name = "cowboy.review.finished",
        call_id = %record.call_id,
        parent_session_id = %record.placement.parent_session_id,
        provider = %record.provider,
        purpose = ?record.purpose,
        state = ?record.state,
        error = record.error.as_ref().map_or("", |error| error.code.as_str()),
        preset = record.preset.as_deref().unwrap_or(""),
        model = option("model"),
        effort = if option("reasoning_effort").is_empty() { option("effort") } else { option("reasoning_effort") },
        risk = record
            .labels
            .get(crate::managed_calls::review::RISK)
            .map_or("undeclared", String::as_str),
        reused = result.and_then(|r| r.pointer("/reused_from/call_id")).is_some(),
        total_tokens = tokens("total_tokens"),
        input_tokens = tokens("input_tokens"),
        cached_input_tokens = tokens("cached_input_tokens"),
        uncached_input_tokens = tokens("uncached_input_tokens"),
        output_tokens = tokens("output_tokens"),
        reasoning_tokens = tokens("reasoning_tokens"),
        duration_ms,
        verdict,
        review_complete = structured.and_then(|s| s["review_complete"].as_bool()),
        findings = findings.map(|all| all.len() as u64),
        critical = severity("critical"),
        high = severity("high"),
        medium = severity("medium"),
        low = severity("low"),
        completed = record.state == State::Completed,
        "review finished"
    );
}

/// Queue the parent's opted-in completion notice. A parent that is gone or
/// closing gets none; the durable completion ledger and call-scoped id absorb
/// repeated submissions across restarts and native session rollover.
pub(in crate::server::managed_calls) async fn notify_parent(
    state: &super::AppState,
    record: &Record,
) {
    notify_parent_in(&state.hub, state.store.as_ref(), record).await;
}

pub(in crate::server::managed_calls) async fn notify_parent_in(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    record: &Record,
) {
    if !super::super::wait_observation::wants_notice(record) {
        return;
    }
    let parent = &record.placement.parent_session_id;
    let cmid = crate::store::completion_id_for_call(&record.call_id);
    // Finding 12: serialize delivery-state check and Hub enqueue with result
    // consumption and non-delivery requeues under `pending_commit_lock`.
    let _commit_guard = hub.pending_commit_lock(parent).await;
    if let Some(store) = store
        && let Ok(Some(delivery)) = store.call_notice_delivery(parent, &record.call_id).await
        && (delivery.state != crate::store::NoticeDeliveryState::Enqueued
            || delivery.dispatch_started_at_ms.is_some())
    {
        return;
    }
    if hub
        .session_info(parent)
        .is_none_or(|info| info.meta.closing)
    {
        crate::logs::wait_cache::record_notice_admission(parent, Some(&cmid), "parent_unavailable");
        if let Some(store) = store {
            let now = chrono::Utc::now().timestamp_millis();
            let _ = store
                .mark_call_notice_unknown(parent, &cmid, "parent_unavailable", None, now)
                .await;
        }
        return;
    }
    hub.submit_call_notice(parent, &record.call_id, super::call_notice(record));
}

/// Reading a terminal Call result confirms consumption durably before
/// withdrawing any still-queued untouched completion notice.
pub(in crate::server::managed_calls) async fn on_result_read_in(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    record: &Record,
) -> anyhow::Result<()> {
    if !record.state.terminal() || !super::super::wait_observation::wants_notice(record) {
        return Ok(());
    }
    let parent = &record.placement.parent_session_id;
    let completion_id = crate::store::completion_id_for_call(&record.call_id);
    // Finding 12: serialize durable consumption and exact queue removal with
    // non-delivery requeues and runner admission under `pending_commit_lock`.
    let _commit_guard = hub.pending_commit_lock(parent).await;
    if let Some(store) = store {
        let now = chrono::Utc::now().timestamp_millis();
        store
            .mark_call_notice_consumed_for_result_read(parent, &record.call_id, &completion_id, now)
            .await?;
    }
    hub.clear_all_notice_claims(parent, &completion_id);
    let _ = hub.remove_queued_by_cmid(parent, &completion_id);
    Ok(())
}

/// Build the Call envelope and attach durable completion-delivery state when
/// the Call opted into `notify: "parent"`.
pub(in crate::server::managed_calls) async fn envelope_with_notice(
    state: &super::AppState,
    record: &Record,
) -> serde_json::Value {
    let mut value = super::super::coordinator::envelope(record);
    if super::super::wait_observation::wants_notice(record)
        && let Some(store) = &state.store
        && let Ok(Some(delivery)) = store
            .call_notice_delivery(&record.placement.parent_session_id, &record.call_id)
            .await
    {
        value["notice_delivery"] = serde_json::json!(delivery);
    }
    value
}

/// The call outcome code of a child turn's stop reason.
pub(in crate::server::managed_calls) fn stop_code(stop_reason: &str) -> &'static str {
    match stop_reason.to_ascii_lowercase().replace('_', "").as_str() {
        "endturn" | "done" => "completed",
        "cancelled" | "canceled" => "cancelled",
        "refusal" => "refusal",
        "maxtokens" => "max_tokens",
        "maxturnrequests" => "max_turn_requests",
        _ => "provider_error",
    }
}
