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
/// closing gets none; the Hub's call-scoped id absorbs a repeated notice.
pub(super) fn notify_parent(state: &super::AppState, record: &Record) {
    if !super::super::wait_observation::wants_notice(record) {
        return;
    }
    let parent = &record.placement.parent_session_id;
    if state
        .hub
        .session_info(parent)
        .is_none_or(|info| info.meta.closing)
    {
        let cmid = format!(
            "{}{}",
            crate::agent_model::CALL_NOTICE_PREFIX,
            record.call_id
        );
        crate::logs::wait_cache::record_notice_admission(parent, Some(&cmid), "parent_unavailable");
        return;
    }
    state
        .hub
        .submit_call_notice(parent, &record.call_id, super::call_notice(record));
}
