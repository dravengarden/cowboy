use super::*;
use crate::machine_protocol::ProviderUsageEvent;

pub(super) fn event() -> ProviderUsageEvent {
    ProviderUsageEvent {
        schema_version: 3,
        producer_id: "codex-deepseek".to_owned(),
        sequence: 1,
        occurred_at_ms: 1_786_000_000_000,
        account_fingerprint: "0123456789abcdef".to_owned(),
        provider: "deepseek".to_owned(),
        agent: "codex".to_owned(),
        model: "deepseek-v4-flash".to_owned(),
        model_family: "flash".to_owned(),
        resolved_model: Some("deepseek-v4-flash".to_owned()),
        model_revision: Some("fp-v4".to_owned()),
        request_role: "executor".to_owned(),
        status: 200,
        input_tokens: Some(10),
        output_tokens: Some(4),
        reasoning_tokens: Some(1),
        cache_hit_tokens: Some(7),
        cache_miss_tokens: Some(3),
        operation: "responses".to_owned(),
        protocol: "responses".to_owned(),
        client_protocol: "responses".to_owned(),
        upstream_protocol: "responses".to_owned(),
        translation_mode: "native".to_owned(),
        thinking_mode: "enabled".to_owned(),
        reasoning_effort: "high".to_owned(),
        session_fingerprint: Some("11111111111111111111111111111111".to_owned()),
        session_attribution: "response_lineage".to_owned(),
        traffic_source: "unattributed".to_owned(),
        static_prefix_fingerprint: Some("22222222222222222222222222222222".to_owned()),
        request_prefix_fingerprint: Some("33333333333333333333333333333333".to_owned()),
        gateway_build: Some("4444444444444444".to_owned()),
        gateway_boot_id: Some("5555555555555555".to_owned()),
        cache_observation: "derived".to_owned(),
        usage_observed: Some(true),
        completed: Some(true),
        streaming: Some(true),
        duration_ms: Some(42),
        request_bytes: Some(123),
        input_item_count: Some(2),
        tool_count: Some(1),
        system_block_count: Some(1),
        has_previous_response_id: Some(true),
        compatibility_fixes: Some(0),
        request_purpose: "interactive".to_owned(),
        cache_keepalive_outcome: "not_applicable".to_owned(),
        cache_keepalive_algorithm: None,
        cache_keepalive_attempt: Some(0),
        cache_keepalive_interval_ms: Some(0),
        cache_keepalive_source_age_ms: Some(0),
        source_request_prefix_fingerprint: None,
        search_requests: None,
        fetch_requests: None,
    }
}

#[test]
fn controller_accepts_generic_usage_producer_and_rejects_envelope_mismatch() {
    let mut candidate = event();
    candidate.producer_id = "future-gateway".to_owned();
    candidate.provider = "future-account".to_owned();
    candidate.agent = "future-agent".to_owned();
    candidate.model = "future/model-v1".to_owned();
    candidate.model_family = "standard".to_owned();
    candidate.resolved_model = Some("future/model-v1".to_owned());
    assert!(validate_provider_usage_event("future-gateway", &candidate).is_ok());
    assert!(validate_provider_usage_event("other-gateway", &candidate).is_err());
}

#[test]
fn activity_filter_validation_is_generic_and_bounded() {
    let dimensions = (0..crate::plugin_host::MAX_ACTIVITY_ENTRIES)
        .map(|index| format!("dimension-{index}"))
        .collect::<Vec<_>>();
    assert!(
        validate_provider_usage_activity_filter("future-account", 1_000, 61_000, &dimensions, &[],)
            .is_ok()
    );

    let mut oversized = dimensions.clone();
    oversized.push("one-too-many".to_owned());
    assert!(
        validate_provider_usage_activity_filter("future-account", 1_000, 61_000, &oversized, &[],)
            .is_err()
    );
    assert!(
        validate_provider_usage_activity_filter(
            "future-account",
            1_000,
            61_000,
            &["duplicate".to_owned(), "duplicate".to_owned()],
            &[],
        )
        .is_err()
    );
}

#[test]
fn controller_rejects_oversized_usage_metric() {
    let mut candidate = event();
    candidate.cache_hit_tokens = Some(crate::machine_protocol::PROVIDER_USAGE_MAX_TOKENS + 1);
    assert!(validate_provider_usage_event("codex-deepseek", &candidate).is_err());
}

#[test]
fn controller_accepts_unknown_role_for_existing_agent_lanes() {
    let mut candidate = event();
    candidate.request_role = "unknown".to_owned();
    assert!(validate_provider_usage_event("codex-deepseek", &candidate).is_ok());
}

#[test]
fn controller_rejects_inconsistent_usage_token_algebra() {
    let mut candidate = event();
    candidate.cache_miss_tokens = Some(4);
    assert!(validate_provider_usage_event("codex-deepseek", &candidate).is_err());

    let mut candidate = event();
    candidate.reasoning_tokens = Some(5);
    assert!(validate_provider_usage_event("codex-deepseek", &candidate).is_err());
}

#[test]
fn controller_accepts_complete_cache_keepalive_and_rejects_missing_source_prefix() {
    let mut candidate = event();
    candidate.schema_version = 4;
    candidate.request_purpose = "cache_keepalive".to_owned();
    candidate.cache_keepalive_outcome = "hit".to_owned();
    candidate.cache_keepalive_algorithm = Some("adaptive-replay-v1".to_owned());
    candidate.cache_keepalive_attempt = Some(1);
    candidate.cache_keepalive_interval_ms = Some(19_800_000);
    candidate.cache_keepalive_source_age_ms = Some(19_801_000);
    candidate.source_request_prefix_fingerprint =
        Some("66666666666666666666666666666666".to_owned());
    candidate.session_attribution = "explicit".to_owned();
    candidate.traffic_source = "cowboy".to_owned();
    assert!(validate_provider_usage_event("codex-deepseek", &candidate).is_ok());

    candidate.source_request_prefix_fingerprint = None;
    assert!(validate_provider_usage_event("codex-deepseek", &candidate).is_err());
}

#[test]
fn cache_transition_classification_identifies_prefix_instability() {
    let previous = serde_json::json!({
        "agent": "claude",
        "operation": "messages",
        "model_family": "flash",
        "model_revision": "v4",
        "request_role": "executor",
        "upstream_protocol": "anthropic_messages",
        "translation_mode": "anthropic_compat",
        "thinking_mode": "enabled",
        "reasoning_effort": "high",
        "gateway_build": "build-1",
        "gateway_boot_id": "boot-1",
        "static_prefix_fingerprint": "prefix-1",
        "request_prefix_fingerprint": "request-1",
        "input_item_count": 20,
        "compatibility_fixes": 0,
    });
    let mut current = previous.clone();
    current["static_prefix_fingerprint"] = serde_json::json!("prefix-2");
    assert_eq!(
        cache_transition_cause(&current, &previous, false),
        "static_prefix_changed"
    );

    current = previous.clone();
    assert_eq!(
        cache_transition_cause(&current, &previous, false),
        "unexpected_exact_prefix_miss"
    );
    assert_eq!(
        cache_transition_cause(&current, &previous, true),
        "post_provider_error"
    );
}

#[test]
fn diagnostic_detail_ids_cannot_relabel_healthy_provider_rows() {
    let current = serde_json::json!({
        "schema_version": 3,
        "status": 200,
        "session_fingerprint": "session",
        "session_attribution": "explicit",
        "cache_observation": "explicit",
        "static_prefix_fingerprint": "prefix",
        "input_tokens": 10_000,
        "cache_hit_tokens": 0,
        "cache_miss_tokens": 10_000,
    });
    let previous = serde_json::json!({
        "status": 200,
        "static_prefix_fingerprint": "prefix",
        "input_tokens": 10_000,
        "cache_hit_tokens": 9_500,
        "cache_miss_tokens": 500,
    });
    assert!(!provider_detail_matches_kind(
        "provider_error",
        &current,
        &previous,
        Some(1_000),
    ));
    assert!(provider_detail_matches_kind(
        "cache_anomaly",
        &current,
        &previous,
        Some(1_000),
    ));
    assert!(!provider_detail_matches_kind(
        "cache_anomaly",
        &current,
        &previous,
        Some(31 * 60 * 1_000),
    ));
}

#[test]
fn diagnostic_titles_do_not_embed_vendor_product_names() {
    assert_eq!(diagnostic_http_title(400), "HTTP 400");
    assert_eq!(diagnostic_keepalive_title("hit"), "Cache protected");
    assert_eq!(diagnostic_keepalive_title("miss"), "Keepalive cache miss");
    assert_eq!(
        diagnostic_keepalive_title("unknown"),
        "Keepalive observation"
    );
    assert_eq!(diagnostic_title("xai"), "xAI");
    assert_eq!(diagnostic_title("openai"), "OpenAI");
    assert_eq!(diagnostic_title("cache_keepalive"), "Cache keepalive");
}

#[test]
fn provider_status_impact_separates_blockers_from_retryable_attempts() {
    for status in [400, 401, 402, 403, 404, 422] {
        assert!(provider_status_impact(status).1, "{status} should block");
    }
    for status in [408, 425, 429, 499, 500, 502, 503] {
        assert!(
            !provider_status_impact(status).1,
            "{status} should be retryable"
        );
    }
}

#[test]
fn provider_diagnostic_ids_preserve_colons_inside_producer_ids() {
    assert_eq!(
        parse_provider_diagnostic_id("cache:hawk:gateway:boot:42", "cache:"),
        Some(("hawk", "gateway:boot", 42))
    );
}
