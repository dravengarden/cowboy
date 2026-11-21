//! One durable-journal event per finished review, so routing and reviewer
//! cost and outcome can be analysed later (docs/review-routing.md). The ledger
//! keeps the full record; this carries counts and identifiers only, never
//! instructions, findings text or paths.

use crate::managed_calls::lifecycle::{Record, State};

pub(super) fn record(record: &Record) {
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
    let findings = structured
        .and_then(|structured| structured["findings"].as_array())
        .map(Vec::as_slice)
        .unwrap_or_default();
    let severity = |level: &str| findings.iter().filter(|f| f["severity"] == level).count() as u64;
    let configuration = record.configuration.as_ref();
    let option = |key: &str| {
        configuration
            .and_then(|values| values.get(key))
            .map_or("", String::as_str)
    };
    let duration_ms = match (record.started_at_ms, record.finished_at_ms) {
        (Some(started), Some(finished)) => (finished - started).max(0),
        _ => 0,
    };
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
        verdict = structured.and_then(|s| s["verdict"].as_str()).unwrap_or(""),
        review_complete = structured.and_then(|s| s["review_complete"].as_bool()).unwrap_or(false),
        findings = findings.len() as u64,
        critical = severity("critical"),
        high = severity("high"),
        medium = severity("medium"),
        low = severity("low"),
        completed = record.state == State::Completed,
        "review finished"
    );
}
