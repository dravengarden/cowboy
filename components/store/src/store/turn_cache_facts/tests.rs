use std::collections::HashMap;

use serde_json::json;

use super::*;
use crate::agent_model::{Event, Status, TurnUsage};
use crate::model::{Envelope, SessionMeta, SessionOrigin};
use crate::provider_account::ProviderAccountId;
use crate::store::{ProductUser, Store};

fn sample_user(id: &str, username: &str) -> ProductUser {
    ProductUser {
        id: id.to_owned(),
        username: username.to_owned(),
        password_algo: "argon2id".to_owned(),
        password_hash: "test-hash".to_owned(),
        created_at_ms: 1_000,
        updated_at_ms: 1_000,
        disabled_at_ms: None,
    }
}

fn sample_meta(id: &str, owner: &str, provider: &str, account: &str) -> SessionMeta {
    SessionMeta {
        continued_from: None,
        handoff_resumed: false,
        forked_from: None,
        id: id.to_owned(),
        provider: provider.to_owned(),
        provider_version: "1.2.3".to_owned(),
        provider_generation_digest: "gen-digest".to_owned(),
        provider_auth_generation: None,
        provider_account_id: account.parse::<ProviderAccountId>().expect("account id"),
        provider_behavior: None,
        machine_id: "hawk".to_owned(),
        workspace_id: None,
        workspace_name: None,
        workspace_source_path: None,
        execution_binding: None,
        cwd: "/tmp/work".to_owned(),
        title: "Cache test".to_owned(),
        status: Status::Running,
        origin: SessionOrigin::Web,
        agent_session_id: None,
        system: false,
        paused: false,
        closing: false,
        context_used: 0,
        context_size: 0,
        usage: None,
        attention: crate::model::SessionAttention::default(),
        background_tasks: 0,
        supports_current_turn: false,
        wrap_up_steering: false,
        provider_update: None,
        provider_update_available: None,
        resource_wait: None,
        next_schedule_ms: None,
        owner_user_id: Some(owner.to_owned()),
        owner_username: Some("owner".to_owned()),
        trashed_at_ms: None,
    }
}

async fn sqlite_test_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::connect("sqlite::memory:", dir.path().join("artifacts"))
        .await
        .expect("connect sqlite");
    store.migrate().await.expect("migrate sqlite");
    (dir, store)
}

#[test]
fn normalize_turn_usage_enforces_disjoint_breakdown_and_preserves_unknowns() {
    let claude = TurnUsage {
        total_tokens: 105_000,
        input_tokens: 100_000,
        output_tokens: 5_000,
        cached_input_tokens: Some(80_000),
        uncached_input_tokens: Some(20_000),
        cache_write_tokens: Some(18_000),
        reasoning_tokens: None,
    };
    let norm = normalize_turn_usage(Some(&claude)).expect("valid claude usage");
    assert_eq!(norm.inclusive_input_tokens, 100_000);
    assert_eq!(norm.cached_read_tokens, Some(80_000));
    assert_eq!(norm.cache_write_tokens, Some(18_000));
    assert_eq!(norm.other_uncached_input_tokens, Some(2_000));
    assert_eq!(
        norm.cached_read_tokens.unwrap()
            + norm.cache_write_tokens.unwrap()
            + norm.other_uncached_input_tokens.unwrap(),
        norm.inclusive_input_tokens
    );

    let codex = TurnUsage {
        total_tokens: 52_000,
        input_tokens: 50_000,
        output_tokens: 2_000,
        cached_input_tokens: Some(40_000),
        uncached_input_tokens: Some(10_000),
        cache_write_tokens: None,
        reasoning_tokens: Some(500),
    };
    let norm = normalize_turn_usage(Some(&codex)).expect("valid codex usage");
    assert_eq!(norm.cached_read_tokens, Some(40_000));
    assert_eq!(norm.cache_write_tokens, None);
    assert_eq!(norm.other_uncached_input_tokens, None);
    assert_eq!(norm.reasoning_tokens, Some(500));

    let unknown = TurnUsage {
        total_tokens: 1_100,
        input_tokens: 1_000,
        output_tokens: 100,
        cached_input_tokens: None,
        uncached_input_tokens: None,
        cache_write_tokens: None,
        reasoning_tokens: None,
    };
    let norm = normalize_turn_usage(Some(&unknown)).expect("valid unknown cache usage");
    assert_eq!(norm.cached_read_tokens, None);
    assert_eq!(norm.cache_write_tokens, None);
    assert_eq!(norm.other_uncached_input_tokens, None);

    let bad = TurnUsage {
        total_tokens: 1_100,
        input_tokens: 1_000,
        output_tokens: 100,
        cached_input_tokens: Some(800),
        uncached_input_tokens: Some(200),
        cache_write_tokens: Some(250),
        reasoning_tokens: None,
    };
    assert_eq!(normalize_turn_usage(Some(&bad)), None);
}

#[test]
fn extract_session_model_prefers_usage_then_preferences_then_options() {
    let last_usage = json!({
        "raw": { "_cowboy": { "modelUsage": [{ "model": "claude-opus-4-6" }] } }
    });
    let prefs = json!({ "model": "opus" });
    let options = json!([{
        "id": "model",
        "currentValue": "[\"deepseek-official\",\"deepseek-v4-flash\"]"
    }]);
    assert_eq!(
        extract_session_model(Some(&last_usage), Some(&prefs), Some(&options)).as_deref(),
        Some("claude-opus-4-6")
    );
    assert_eq!(
        extract_session_model(None, Some(&prefs), Some(&options)).as_deref(),
        Some("opus")
    );
    assert_eq!(
        extract_session_model(None, None, Some(&options)).as_deref(),
        Some("deepseek-v4-flash")
    );
}

#[test]
fn normalize_stop_reason_bounds_verbose_errors() {
    assert_eq!(normalize_stop_reason("EndTurn"), "end_turn");
    assert_eq!(normalize_stop_reason("Cancelled"), "cancelled");
    assert_eq!(
        normalize_stop_reason("error: antigravity_backend_refusal:rate_limited: { ... }"),
        "error"
    );
}

#[tokio::test]
async fn sqlite_projection_survives_clear_isolates_accounts_and_backfills() {
    let (_dir, store) = sqlite_test_store().await;
    store
        .insert_user(&sample_user("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "alice"))
        .await
        .expect("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    store
        .insert_user(&sample_user("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "bob"))
        .await
        .expect("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    let acct_a = "11111111-1111-1111-1111-111111111111";
    let acct_b = "22222222-2222-2222-2222-222222222222";

    let s1 = sample_meta(
        "sess-1",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "claude-code",
        acct_a,
    );
    let s2 = sample_meta(
        "sess-2",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "claude-code",
        acct_b,
    );
    store.insert_session(&s1).await.expect("insert s1");
    store.insert_session(&s2).await.expect("insert s2");
    store
        .update_config_preferences("sess-1", &json!({"model": "claude-opus-4-6"}))
        .await
        .expect("prefs s1");

    let batch = vec![
        Envelope {
            session_id: "sess-1".into(),
            seq: 1,
            event: Event::Update {
                update: json!({
                    "sessionUpdate": "user_message_chunk",
                    "promptOrigin": { "actor": "human" },
                    "content": { "type": "text", "text": "hello" }
                }),
            },
            cmid: None,
        },
        Envelope {
            session_id: "sess-1".into(),
            seq: 2,
            event: Event::TurnEnd {
                stop_reason: "EndTurn".into(),
                usage: Some(TurnUsage {
                    total_tokens: 105_000,
                    input_tokens: 100_000,
                    output_tokens: 5_000,
                    cached_input_tokens: Some(80_000),
                    uncached_input_tokens: Some(20_000),
                    cache_write_tokens: Some(18_000),
                    reasoning_tokens: None,
                }),
            },
            cmid: None,
        },
        Envelope {
            session_id: "sess-1".into(),
            seq: 3,
            event: Event::Update {
                update: json!({ "sessionUpdate": "context_rolled_over" }),
            },
            cmid: None,
        },
        Envelope {
            session_id: "sess-1".into(),
            seq: 4,
            event: Event::Update {
                update: json!({
                    "sessionUpdate": "user_message_chunk",
                    "promptOrigin": { "actor": "human" },
                    "content": { "type": "text", "text": "continue" }
                }),
            },
            cmid: None,
        },
        Envelope {
            session_id: "sess-1".into(),
            seq: 5,
            event: Event::TurnEnd {
                stop_reason: "EndTurn".into(),
                usage: Some(TurnUsage {
                    total_tokens: 11_000,
                    input_tokens: 10_000,
                    output_tokens: 1_000,
                    cached_input_tokens: None,
                    uncached_input_tokens: None,
                    cache_write_tokens: None,
                    reasoning_tokens: None,
                }),
            },
            cmid: None,
        },
        Envelope {
            session_id: "sess-2".into(),
            seq: 1,
            event: Event::TurnEnd {
                stop_reason: "EndTurn".into(),
                usage: Some(TurnUsage {
                    total_tokens: 51_000,
                    input_tokens: 50_000,
                    output_tokens: 1_000,
                    cached_input_tokens: Some(25_000),
                    uncached_input_tokens: Some(25_000),
                    cache_write_tokens: None,
                    reasoning_tokens: None,
                }),
            },
            cmid: None,
        },
    ];
    let highwaters = HashMap::from([("sess-1".into(), 6), ("sess-2".into(), 2)]);
    store
        .upsert_event_batch(&batch, &highwaters)
        .await
        .expect("upsert batch");

    let now = chrono::Utc::now().timestamp_millis();
    let acct_metrics = store
        .account_cache_metrics(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "claude-code",
            acct_a,
            now - 86_400_000,
            now + 60_000,
            "24h",
        )
        .await
        .expect("account metrics");
    assert_eq!(acct_metrics.totals.total_turns, 2);
    assert_eq!(acct_metrics.totals.eligible_turns, 2);
    assert_eq!(acct_metrics.totals.observed_usage_turns, 2);
    assert_eq!(acct_metrics.totals.observed_cache_turns, 1);
    assert_eq!(acct_metrics.totals.renewed_turns, 1);
    assert_eq!(acct_metrics.totals.inclusive_input_tokens, 110_000);
    assert_eq!(acct_metrics.totals.observed_cache_input_tokens, 100_000);
    assert_eq!(acct_metrics.totals.cached_read_tokens, Some(80_000));
    assert_eq!(acct_metrics.totals.cache_write_tokens, Some(18_000));
    assert_eq!(acct_metrics.totals.uncached_input_tokens, Some(20_000));
    assert_eq!(acct_metrics.totals.other_uncached_input_tokens, Some(2_000));
    assert!((acct_metrics.totals.cached_input_share.unwrap() - 0.8).abs() < 1e-9);
    assert_eq!(acct_metrics.by_model[0].model, "claude-opus-4-6");

    let cross = store
        .account_cache_metrics(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "claude-code",
            acct_b,
            now - 86_400_000,
            now + 60_000,
            "24h",
        )
        .await
        .expect("cross owner");
    assert_eq!(cross.totals.total_turns, 0);

    // Transcript Clear removes events, while turn_cache_facts survives.
    store
        .clear_events("sess-1", now + 10)
        .await
        .expect("clear sess-1");
    let after_clear = store
        .session_cache_metrics("sess-1", "claude-code", acct_a, 10)
        .await
        .expect("session metrics after clear");
    assert_eq!(after_clear.totals.total_turns, 2);
    assert_eq!(after_clear.recent_turns.len(), 2);
    assert!(after_clear.recent_turns[0].renewed);
    assert_eq!(after_clear.recent_turns[0].prompt_origin, "human");
    assert_eq!(after_clear.recent_turns[1].cached_read_tokens, Some(80_000));

    let p1 = store
        .backfill_turn_cache_facts_batch(10, DEFAULT_RETENTION_DAYS)
        .await
        .expect("backfill");
    assert!(p1.completed);
    let acct_b_metrics = store
        .account_cache_metrics(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "claude-code",
            acct_b,
            now - 86_400_000,
            now + 60_000,
            "24h",
        )
        .await
        .expect("account b metrics");
    assert!(acct_b_metrics.coverage.backfill_complete);
    assert_eq!(acct_b_metrics.totals.total_turns, 1);

    // Default account isolation: same owner with provider_account_id='default' on two
    // providers must never mix across providers.
    let s_ds = sample_meta(
        "sess-ds",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "claude-deepseek",
        "default",
    );
    let s_cx = sample_meta(
        "sess-cx",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "codex",
        "default",
    );
    store.insert_session(&s_ds).await.expect("insert s_ds");
    store.insert_session(&s_cx).await.expect("insert s_cx");
    store
        .upsert_event_batch(
            &[
                Envelope {
                    session_id: "sess-ds".into(),
                    seq: 1,
                    event: Event::TurnEnd {
                        stop_reason: "EndTurn".into(),
                        usage: Some(TurnUsage {
                            total_tokens: 1_200,
                            input_tokens: 1_000,
                            output_tokens: 200,
                            cached_input_tokens: Some(600),
                            uncached_input_tokens: Some(400),
                            cache_write_tokens: None,
                            reasoning_tokens: None,
                        }),
                    },
                    cmid: None,
                },
                Envelope {
                    session_id: "sess-cx".into(),
                    seq: 1,
                    event: Event::TurnEnd {
                        stop_reason: "EndTurn".into(),
                        usage: Some(TurnUsage {
                            total_tokens: 2_200,
                            input_tokens: 2_000,
                            output_tokens: 200,
                            cached_input_tokens: Some(1_800),
                            uncached_input_tokens: Some(200),
                            cache_write_tokens: None,
                            reasoning_tokens: None,
                        }),
                    },
                    cmid: None,
                },
            ],
            &HashMap::from([("sess-ds".into(), 2), ("sess-cx".into(), 2)]),
        )
        .await
        .expect("upsert default sessions");
    let ds_default = store
        .account_cache_metrics(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "claude-deepseek",
            "default",
            now - 86_400_000,
            now + 60_000,
            "24h",
        )
        .await
        .expect("ds default");
    assert_eq!(ds_default.totals.total_turns, 1);
    assert_eq!(ds_default.totals.inclusive_input_tokens, 1_000);
    assert_eq!(ds_default.totals.cached_read_tokens, Some(600));

    assert_eq!(
        store
            .prune_turn_cache_facts(DEFAULT_RETENTION_DAYS)
            .await
            .expect("prune"),
        0
    );
}
