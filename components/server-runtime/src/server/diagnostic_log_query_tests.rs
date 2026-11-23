use super::{
    DiagnosticLogsQuery, decode_log_cursor, encode_log_cursor, now_ms, parse_diagnostic_log_filter,
};

#[test]
fn diagnostic_log_filters_are_closed_and_bounded() {
    let filter = parse_diagnostic_log_filter(&DiagnosticLogsQuery {
        kind: Some("cache_anomaly,provider_error".to_owned()),
        severity: Some("warning,error".to_owned()),
        state: Some("observed,failed".to_owned()),
        agent: Some("claude,codex".to_owned()),
        window: Some("30d".to_owned()),
        from_ms: None,
        to_ms: None,
        session: Some("0123456789abcdef0123456789abcdef".to_owned()),
        limit: Some(5_000),
        cursor: None,
    })
    .expect("valid diagnostic filters");
    assert_eq!(filter.kinds, ["cache_anomaly", "provider_error"]);
    assert_eq!(filter.severities, ["warning", "error"]);
    assert_eq!(filter.states, ["observed", "failed"]);
    assert_eq!(filter.agents, ["claude", "codex"]);
    assert_eq!(filter.limit, 100);

    assert!(
        parse_diagnostic_log_filter(&DiagnosticLogsQuery {
            kind: Some("raw_prompt".to_owned()),
            ..DiagnosticLogsQuery::default()
        })
        .is_err()
    );
}

#[test]
fn diagnostic_log_time_ranges_require_two_bounded_boundaries() {
    let until_ms = now_ms();
    let filter = parse_diagnostic_log_filter(&DiagnosticLogsQuery {
        from_ms: Some(until_ms - 6 * 3_600_000),
        to_ms: Some(until_ms),
        ..DiagnosticLogsQuery::default()
    })
    .expect("valid explicit range");
    assert_eq!(filter.since_ms, until_ms - 6 * 3_600_000);
    assert_eq!(filter.until_ms, until_ms);
    assert!(
        parse_diagnostic_log_filter(&DiagnosticLogsQuery {
            from_ms: Some(until_ms - 3_600_000),
            ..DiagnosticLogsQuery::default()
        })
        .is_err()
    );
}

#[test]
fn diagnostic_log_cursor_round_trips_the_stable_sort_key() {
    let item = crate::store::DiagnosticLogSummary {
        id: "provider:hawk:producer-1:42".to_owned(),
        occurred_at_ms: 1_786_035_044_709,
        kind: "provider_error".to_owned(),
        severity: "error".to_owned(),
        state: "failed".to_owned(),
        title: "HTTP 400".to_owned(),
        summary: "request failed".to_owned(),
        session_ref: None,
        provider: Some("deepseek".to_owned()),
        agent: Some("claude".to_owned()),
        model: Some("deepseek-v4-flash".to_owned()),
        classification: Some("provider_http_error".to_owned()),
    };
    let encoded = encode_log_cursor(&item).expect("encode cursor");
    let decoded = decode_log_cursor(Some(&encoded))
        .expect("decode cursor")
        .expect("cursor present");
    assert_eq!(decoded.occurred_at_ms, item.occurred_at_ms);
    assert_eq!(decoded.id, item.id);
}
