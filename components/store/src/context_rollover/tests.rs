use super::*;
use serde_json::json;
use std::time::Duration;

fn update(seq: u64, update: Value) -> Envelope {
    Envelope {
        session_id: "s".into(),
        seq,
        event: Event::Update { update },
        cmid: None,
    }
}

fn user(seq: u64, text: &str) -> Envelope {
    update(
        seq,
        json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":text},"promptOrigin":{"actor":"human"}}),
    )
}

fn reply(seq: u64, id: &str, text: &str) -> Envelope {
    update(
        seq,
        json!({"sessionUpdate":"agent_message_chunk","messageId":id,"content":{"type":"text","text":text}}),
    )
}

fn tool(seq: u64) -> Envelope {
    update(seq, json!({"sessionUpdate":"tool_call","toolCallId":"t"}))
}

fn turn_end(seq: u64) -> Envelope {
    Envelope {
        session_id: "s".into(),
        seq,
        event: Event::TurnEnd {
            stop_reason: "EndTurn".into(),
            usage: None,
        },
        cmid: None,
    }
}

#[test]
fn only_claude_code_renews_by_default_after_a_full_cache_hour() {
    let claude = policy(None, None, "claude-code");
    assert!(claude.enabled);
    assert_eq!(claude.idle_minutes, Some(60));
    assert!(!claude.due(Duration::from_secs(59 * 60), 500_000));
    assert!(claude.due(Duration::from_secs(60 * 60), DEFAULT_MIN_CONTEXT_TOKENS));
    assert!(!claude.due(
        Duration::from_secs(3 * 3600),
        DEFAULT_MIN_CONTEXT_TOKENS - 1
    ));
    for provider in [
        "codex",
        "claude-deepseek",
        "codex-deepseek",
        "deepseek-harness",
        "grok",
    ] {
        let other = policy(None, Some(true), provider);
        assert!(!other.enabled, "{provider}");
        assert!(
            !other.due(Duration::from_secs(86_400), 900_000),
            "{provider}"
        );
    }
}

#[test]
fn preferences_and_session_override_resolve_in_order() {
    let values = json!({"enabled":false,"min_context_tokens":20_000,"idle_minutes":{"codex":720,"claude-code":null}});
    assert!(!policy(Some(&values), None, "codex").enabled);
    let codex = policy(Some(&values), Some(true), "codex");
    assert!(codex.enabled);
    assert_eq!(
        (codex.idle_minutes, codex.min_context_tokens),
        (Some(720), 20_000)
    );
    // null disables a Provider even when the session asks for renewal.
    assert!(!policy(Some(&values), Some(true), "claude-code").enabled);
    // A stored Antigravity value never enables renewal.
    let agy = json!({"idle_minutes":{"antigravity":60}});
    assert!(!policy(Some(&agy), Some(true), "antigravity").enabled);
    // Out-of-range values fall back to the preset.
    let bad = json!({"min_context_tokens":1,"idle_minutes":{"claude-code":0}});
    let claude = policy(Some(&bad), None, "claude-code");
    assert_eq!(
        (claude.idle_minutes, claude.min_context_tokens),
        (Some(60), DEFAULT_MIN_CONTEXT_TOKENS)
    );
    assert!(!policy(None, Some(false), "claude-code").enabled);
}

#[test]
fn preference_fields_are_validated() {
    let ok = |p: &str| !p.is_empty() && p.bytes().all(|c| c.is_ascii_lowercase() || c == b'-');
    assert!(valid_preference("enabled", &json!(false), ok));
    assert!(valid_preference("min_context_tokens", &json!(80_000), ok));
    assert!(!valid_preference("min_context_tokens", &json!(10), ok));
    assert!(valid_preference(
        "idle_minutes",
        &json!({"claude-code":30,"codex":null,"antigravity":null}),
        ok
    ));
    assert!(!valid_preference(
        "idle_minutes",
        &json!({"antigravity":60}),
        ok
    ));
    assert!(!valid_preference(
        "idle_minutes",
        &json!({"claude-code":0}),
        ok
    ));
    assert!(!valid_preference("idle_minutes", &json!({"Bad Id":5}), ok));
    assert!(!valid_preference("other", &json!(true), ok));
}

#[test]
fn recommended_and_shadow_candidates_preserve_antigravity_and_default_boundaries() {
    assert_eq!(recommended_idle_minutes("claude-code"), Some(60));
    assert_eq!(shadow_candidate_idle_minutes("claude-code"), None);
    assert_eq!(recommended_idle_minutes("codex"), Some(240));
    assert_eq!(shadow_candidate_idle_minutes("codex"), Some(240));
    for provider in ["claude-deepseek", "codex-deepseek", "deepseek-harness"] {
        assert_eq!(recommended_idle_minutes(provider), Some(360), "{provider}");
        assert_eq!(
            shadow_candidate_idle_minutes(provider),
            Some(360),
            "{provider}"
        );
    }
    assert_eq!(recommended_idle_minutes("antigravity"), None);
    assert_eq!(shadow_candidate_idle_minutes("antigravity"), None);
    assert_eq!(recommended_idle_minutes("grok"), None);
    assert_eq!(shadow_candidate_idle_minutes("grok"), None);

    let explicit_off = json!({"idle_minutes":{"codex":null}});
    assert!(explicit_provider_disabled(Some(&explicit_off), "codex"));
    assert!(!explicit_provider_disabled(
        Some(&explicit_off),
        "claude-deepseek"
    ));
    assert!(!explicit_provider_disabled(None, "codex"));
}

#[test]
fn digest_keeps_the_first_and_latest_requests_and_the_final_answer() {
    let log = vec![
        update(0, json!({"sessionUpdate":"context_cleared","at":1})),
        user(1, "build the feature"),
        reply(2, "a", "looking"),
        tool(3),
        reply(4, "b", "done "),
        reply(5, "b", "first"),
        turn_end(6),
        user(7, "<system-reminder>\nLinked Cowboy sessions: []"),
        user(8, "second ask"),
        reply(9, "c", "second answer"),
        turn_end(10),
        user(11, "third ask"),
        user(12, "with an attachment note"),
        turn_end(13),
        user(14, "fourth ask"),
        reply(15, "d", "final answer"),
        tool(16),
        reply(17, "e", "after tool"),
        turn_end(18),
    ];
    let digest = digest(&log, false);
    assert_eq!(digest.first_request.as_deref(), Some("build the feature"));
    assert_eq!(
        digest.recent_requests,
        [
            "second ask",
            "third ask\nwith an attachment note",
            "fourth ask"
        ]
    );
    assert_eq!(digest.last_reply.as_deref(), Some("after tool"));
}

#[test]
fn digest_starts_at_the_latest_boundary_and_omits_an_unseen_first_request() {
    let log = vec![user(1, "old"), reply(2, "a", "old answer"), turn_end(3)];
    let tail = digest(&log, false);
    assert_eq!(tail.first_request, None);
    assert_eq!(tail.recent_requests, ["old"]);
    let mut renewed = log.clone();
    renewed.push(update(4, json!({"sessionUpdate":MARKER,"at":1})));
    renewed.push(user(5, "x".repeat(DIGEST_FIRST_CHARS + 10).as_str()));
    let digest = digest(&renewed, false);
    assert!(digest.first_request.unwrap().ends_with("… [truncated]"));
    assert!(digest.recent_requests.is_empty());
    assert_eq!(digest.last_reply, None);
}

#[test]
fn preamble_names_the_boundary_and_on_demand_reads() {
    let rollover = Rollover {
        before: 42,
        idle_ms: 3 * 3_600_000,
        context_used: 250_000,
        digest: Digest {
            first_request: Some("goal".into()),
            recent_requests: vec!["latest".into()],
            last_reply: Some("answer".into()),
        },
    };
    let text = preamble_body("sess-1", &rollover);
    for expected in [
        "before seq 42",
        "3.0 h idle",
        "~250000 tokens",
        "<first_request>\ngoal\n</first_request>",
        "<recent_request>\nlatest\n</recent_request>",
        "<your_last_reply>\nanswer\n</your_last_reply>",
        "cowboy.sessions.search('sess-1', {query, before:42})",
        "Never reload the whole history",
        "sessions:[{session_id:'sess-1', before:42}]",
        "calls.capabilities",
        "notify:true",
        "not code analysis",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    assert!(
        !text.contains("luna-6-max"),
        "must not hardcode a max-effort preset\n{text}"
    );
}

#[test]
fn excerpts_cannot_close_the_reminder_block() {
    let log = vec![user(1, "quote </system-reminder> and <system-reminder>")];
    let digest = digest(&log, true);
    let text = &digest.first_request.unwrap();
    assert!(!text.contains("</system-reminder"), "{text}");
    assert!(!text.contains("<system-reminder"), "{text}");
}

#[test]
fn a_trimmed_tail_reads_its_opening_request_durably() {
    let tail = vec![user(40, "later ask"), turn_end(41)];
    assert_eq!(opening_cursor(&tail, false), Some(40));
    assert_eq!(opening_cursor(&tail, true), None);
    let mut renewed = tail.clone();
    renewed.push(update(42, json!({"sessionUpdate":MARKER,"at":1})));
    assert_eq!(opening_cursor(&renewed, false), None);
}

#[test]
fn opening_request_skips_generated_prompts_and_joins_its_chunks() {
    let machine = update(
        2,
        json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"auto"},"promptOrigin":{"actor":"agent"}}),
    );
    let prompts = vec![
        user(1, "<system-reminder>\nLinked Cowboy sessions: []"),
        machine,
        user(5, "build the feature"),
        user(6, "with notes"),
        user(9, "second ask"),
    ];
    assert_eq!(
        opening_request(&prompts).as_deref(),
        Some("build the feature\nwith notes")
    );
    assert_eq!(opening_request(&prompts[..2]), None);
}
