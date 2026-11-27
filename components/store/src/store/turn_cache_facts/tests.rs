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

    let zero_write = TurnUsage {
        total_tokens: 1_100,
        input_tokens: 1_000,
        output_tokens: 100,
        cached_input_tokens: Some(800),
        uncached_input_tokens: Some(200),
        cache_write_tokens: Some(0),
        reasoning_tokens: None,
    };
    let claude_zero =
        normalize_turn_usage_for_provider("claude-code", Some(&zero_write)).expect("claude zero");
    assert_eq!(claude_zero.cache_write_tokens, Some(0));
    assert_eq!(claude_zero.other_uncached_input_tokens, Some(200));

    for provider in [
        "codex",
        "antigravity",
        "claude-deepseek",
        "codex-deepseek",
        "deepseek-harness",
        "grok",
    ] {
        let norm = normalize_turn_usage_for_provider(provider, Some(&zero_write))
            .expect("valid provider usage");
        assert_eq!(norm.cached_read_tokens, Some(800), "{provider}");
        assert_eq!(norm.cache_write_tokens, None, "{provider}");
        assert_eq!(norm.other_uncached_input_tokens, None, "{provider}");
    }
}

#[test]
fn extract_session_model_uses_exact_and_provider_declared_equivalences_only() {
    let fable_51_usage = json!({
        "raw": { "_cowboy": { "modelUsage": [{ "model": "claude-fable-5-1" }] } }
    });
    let fable_52_usage = json!({
        "raw": { "_cowboy": { "modelUsage": [{ "model": "claude-fable-5-2" }] } }
    });
    let multi_usage = json!({
        "raw": { "_cowboy": { "modelUsage": [
            { "model": "claude-fable-5-1", "totalTokens": 500_000 },
            { "model": "claude-fable-5-2", "totalTokens": 50_000 }
        ] } }
    });
    let fable_prefs = json!({ "model": "fable" });
    let fable_options = json!([{
        "id": "model",
        "currentValue": "fable",
        "_meta": {
            "cowboy.dev/configuration-equivalence": {
                "schema": 1,
                "value": "fable",
                "equivalent_values": ["claude-fable-5-2", "claude-fable-5-2[1m]"]
            }
        }
    }]);
    let dsh_options = json!([{
        "id": "model",
        "currentValue": "[\"deepseek-official\",\"deepseek-v4-flash\"]"
    }]);

    assert_eq!(
        extract_session_model(Some(&fable_52_usage), None, None).as_deref(),
        Some("claude-fable-5-2")
    );
    assert_eq!(
        extract_session_model(
            Some(&fable_52_usage),
            Some(&fable_prefs),
            Some(&fable_options)
        )
        .as_deref(),
        Some("claude-fable-5-2")
    );
    // R3-M3: Stale cumulative claude-fable-5-1 MUST NOT match alias "fable".
    assert_eq!(
        extract_session_model(
            Some(&fable_51_usage),
            Some(&fable_prefs),
            Some(&fable_options)
        ),
        None
    );
    assert_eq!(
        extract_session_model(Some(&fable_52_usage), Some(&fable_prefs), None),
        None
    );
    assert_eq!(
        extract_session_model(Some(&multi_usage), Some(&fable_prefs), Some(&fable_options))
            .as_deref(),
        Some("claude-fable-5-2")
    );
    assert_eq!(extract_session_model(Some(&multi_usage), None, None), None);
    assert_eq!(
        extract_session_model(None, None, Some(&dsh_options)).as_deref(),
        Some("deepseek-v4-flash")
    );
}

#[test]
fn normalize_stop_reason_maps_to_closed_nul_free_set() {
    assert_eq!(normalize_stop_reason("EndTurn"), "end_turn");
    assert_eq!(normalize_stop_reason("Cancelled"), "cancelled");
    assert_eq!(
        normalize_stop_reason("error: antigravity_backend_refusal:rate_limited: { ... }"),
        "error"
    );
    assert_eq!(normalize_stop_reason("weird\0custom\0reason"), "other");
    assert_eq!(normalize_stop_reason("   \0  "), "unknown");
}

#[allow(clippy::too_many_lines)]
async fn assert_projection_contract(store: &Store) {
    let owner_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let owner_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    store
        .insert_user(&sample_user(owner_a, "alice"))
        .await
        .expect("owner-a");
    store
        .insert_user(&sample_user(owner_b, "bob"))
        .await
        .expect("owner-b");
    let acct_a = "11111111-1111-1111-1111-111111111111";
    let acct_b = "22222222-2222-2222-2222-222222222222";

    let s1 = sample_meta("sess-100", owner_a, "claude-code", acct_a);
    let s2 = sample_meta("sess-200", owner_b, "claude-code", acct_b);
    let s_unobs = sample_meta("sess-300", owner_b, "codex", acct_b);
    store.insert_session(&s1).await.expect("insert s1");
    store.insert_session(&s2).await.expect("insert s2");
    store
        .insert_session(&s_unobs)
        .await
        .expect("insert s_unobs");
    store
        .update_config_preferences("sess-100", &json!({"model": "claude-opus-4-6"}))
        .await
        .expect("prefs s1");

    let batch = vec![
        Envelope {
            session_id: "sess-100".into(),
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
            session_id: "sess-100".into(),
            seq: 2,
            event: Event::TurnEnd {
                // Embedded NUL in stop_reason must never break PG or SQLite writes.
                stop_reason: "EndTurn\0".into(),
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
            session_id: "sess-100".into(),
            seq: 3,
            event: Event::Update {
                update: json!({ "sessionUpdate": "context_rolled_over" }),
            },
            cmid: None,
        },
        Envelope {
            session_id: "sess-100".into(),
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
            session_id: "sess-100".into(),
            seq: 5,
            event: Event::TurnEnd {
                stop_reason: "custom\0reason".into(),
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
            session_id: "sess-200".into(),
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
        // Eligible turn with NO usage reported: totals must stay None, not 0.
        Envelope {
            session_id: "sess-300".into(),
            seq: 1,
            event: Event::TurnEnd {
                stop_reason: "EndTurn".into(),
                usage: None,
            },
            cmid: None,
        },
    ];
    let highwaters = HashMap::from([
        ("sess-100".into(), 6),
        ("sess-200".into(), 2),
        ("sess-300".into(), 2),
    ]);
    store
        .upsert_event_batch(&batch, &highwaters)
        .await
        .expect("upsert batch");

    let now = chrono::Utc::now().timestamp_millis();
    let acct_metrics = store
        .account_cache_metrics(
            owner_a,
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
    assert_eq!(acct_metrics.totals.inclusive_input_tokens, Some(110_000));
    assert_eq!(
        acct_metrics.totals.observed_cache_input_tokens,
        Some(100_000)
    );
    assert_eq!(acct_metrics.totals.cached_read_tokens, Some(80_000));
    assert_eq!(acct_metrics.totals.cache_write_tokens, Some(18_000));
    assert_eq!(acct_metrics.totals.uncached_input_tokens, Some(20_000));
    assert_eq!(acct_metrics.totals.other_uncached_input_tokens, Some(2_000));
    assert!((acct_metrics.totals.cached_input_share.unwrap() - 0.8).abs() < 1e-9);
    assert_eq!(acct_metrics.by_model[0].model, "claude-opus-4-6");
    assert!(acct_metrics.coverage.oldest_fact_ms.is_some());
    assert!(acct_metrics.coverage.newest_fact_ms.is_some());

    // Unreported usage totals stay None rather than 0.
    let unobs_metrics = store
        .session_cache_metrics("sess-300", Some(owner_b), "codex", acct_b, 10)
        .await
        .expect("unobserved session");
    assert_eq!(unobs_metrics.totals.eligible_turns, 1);
    assert_eq!(unobs_metrics.totals.observed_usage_turns, 0);
    assert_eq!(unobs_metrics.totals.inclusive_input_tokens, None);
    assert_eq!(unobs_metrics.totals.output_tokens, None);

    // Owner isolation: owner_a querying acct_b sees zero rows and NO timestamps from owner_b.
    let cross = store
        .account_cache_metrics(
            owner_a,
            "claude-code",
            acct_b,
            now - 86_400_000,
            now + 60_000,
            "24h",
        )
        .await
        .expect("cross owner");
    assert_eq!(cross.totals.total_turns, 0);
    assert_eq!(cross.coverage.oldest_fact_ms, None);
    assert_eq!(cross.coverage.newest_fact_ms, None);

    // Owner-scoped session query cannot see another owner's retained facts.
    let wrong_owner_session = store
        .session_cache_metrics("sess-100", Some(owner_b), "claude-code", acct_a, 10)
        .await
        .expect("wrong owner session");
    assert_eq!(wrong_owner_session.totals.total_turns, 0);
    assert!(wrong_owner_session.recent_turns.is_empty());

    // Backfill must NOT overwrite live-projected attribution (e.g. model on sess-100)
    // and must count only newly inserted rows.
    store
        .begin_turn_cache_reconcile()
        .await
        .expect("begin reconcile");
    let p1 = store
        .backfill_turn_cache_facts_batch(50, DEFAULT_RETENTION_DAYS)
        .await
        .expect("reconcile batch");
    assert!(p1.completed);
    assert_eq!(p1.rows_projected, 0, "all live rows already projected");

    // Clear preserves facts AND live model attribution survives.
    store
        .clear_events("sess-100", now + 10)
        .await
        .expect("clear sess-100");
    let after_clear = store
        .session_cache_metrics("sess-100", Some(owner_a), "claude-code", acct_a, 1)
        .await
        .expect("session metrics after clear");
    assert_eq!(after_clear.totals.total_turns, 2);
    assert_eq!(after_clear.recent_turns.len(), 1, "honors indexed LIMIT");
    assert!(after_clear.recent_turns[0].renewed);
    assert_eq!(after_clear.recent_turns[0].stop_reason, "other");
    assert_eq!(after_clear.by_model[0].model, "claude-opus-4-6");

    // Soft-delete + purge sess-200: its retained turn_cache_facts must still
    // reserve sess-200 in next_session_number() so IDs are never reused across owners.
    store.delete_session("sess-200").await.expect("delete s2");
    assert_eq!(store.purge_deleted(0).await.expect("purge s2"), 1);
    let floor = store.next_session_number().await.expect("floor");
    assert!(
        floor > 300,
        "next_session_number must stay above retained turn_cache_facts IDs (got {floor})"
    );
    let after_purge = store
        .account_cache_metrics(
            owner_b,
            "claude-code",
            acct_b,
            now - 86_400_000,
            now + 60_000,
            "24h",
        )
        .await
        .expect("account b after purge");
    assert_eq!(after_purge.totals.total_turns, 1);

    // Default account isolation across providers.
    let s_ds = sample_meta("sess-400", owner_a, "claude-deepseek", "default");
    let s_cx = sample_meta("sess-401", owner_a, "codex", "default");
    store.insert_session(&s_ds).await.expect("insert s_ds");
    store.insert_session(&s_cx).await.expect("insert s_cx");
    store
        .upsert_event_batch(
            &[
                Envelope {
                    session_id: "sess-400".into(),
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
                    session_id: "sess-401".into(),
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
            &HashMap::from([("sess-400".into(), 2), ("sess-401".into(), 2)]),
        )
        .await
        .expect("upsert default sessions");
    let ds_default = store
        .account_cache_metrics(
            owner_a,
            "claude-deepseek",
            "default",
            now - 86_400_000,
            now + 60_000,
            "24h",
        )
        .await
        .expect("ds default");
    assert_eq!(ds_default.totals.total_turns, 1);
    assert_eq!(ds_default.totals.inclusive_input_tokens, Some(1_000));
    assert_eq!(ds_default.totals.cached_read_tokens, Some(600));

    assert_eq!(
        store
            .prune_turn_cache_facts(DEFAULT_RETENTION_DAYS)
            .await
            .expect("prune"),
        0
    );
}

#[tokio::test]
async fn sqlite_projection_survives_clear_isolates_accounts_and_backfills() {
    let (_dir, store) = sqlite_test_store().await;
    assert_projection_contract(&store).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_projection_survives_clear_isolates_accounts_and_backfills() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
        .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .expect("connect postgres");
    store.migrate().await.expect("migrate postgres");
    assert_projection_contract(&store).await;
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn sqlite_pre_delete_and_startup_reconcile_preserve_unprojected_turns() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("reconcile.sqlite3");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .expect("connect sqlite");
    store.migrate().await.expect("migrate sqlite");

    let owner = "cccccccccccccccccccccccccccccccc";
    let acct = "33333333-3333-3333-3333-333333333333";
    store
        .insert_user(&sample_user(owner, "carol"))
        .await
        .expect("owner");
    store
        .insert_session(&sample_meta("sess-500", owner, "claude-code", acct))
        .await
        .expect("s500");
    store
        .insert_session(&sample_meta("sess-501", owner, "claude-code", acct))
        .await
        .expect("s501");
    store
        .insert_session(&sample_meta("sess-502", owner, "claude-code", acct))
        .await
        .expect("s502");
    store
        .update_config_preferences("sess-502", &json!({"model": "claude-opus-4-6"}))
        .await
        .expect("prefs s502");

    // Live-project sess-502 seq=1 so we can verify startup reconcile does NOT
    // overwrite live attribution when it scans missing keys.
    store
        .upsert_event_batch(
            &[Envelope {
                session_id: "sess-502".into(),
                seq: 1,
                event: Event::TurnEnd {
                    stop_reason: "EndTurn".into(),
                    usage: Some(TurnUsage {
                        total_tokens: 1_100,
                        input_tokens: 1_000,
                        output_tokens: 100,
                        cached_input_tokens: Some(800),
                        uncached_input_tokens: Some(200),
                        cache_write_tokens: None,
                        reasoning_tokens: None,
                    }),
                },
                cmid: None,
            }],
            &HashMap::from([("sess-502".into(), 2)]),
        )
        .await
        .expect("live s502");

    // Complete backfill cursor past all current sessions.
    let done = store
        .backfill_turn_cache_facts_batch(50, DEFAULT_RETENTION_DAYS)
        .await
        .expect("initial backfill");
    assert!(done.completed);

    // Insert raw TurnEnd events directly (simulating pre-upgrade / rollback
    // unprojected rows) behind the completed backfill cursor:
    // - sess-500: will be cleared before backfill runs (H1 clear path)
    // - sess-501: will be deleted + purged before backfill runs (H1 purge path)
    // - sess-502 seq=2: sits behind cursor, recovered by begin_turn_cache_reconcile (M1)
    let raw = sqlx::SqlitePool::connect(&url).await.expect("raw sqlite");
    let now = chrono::Utc::now().timestamp_millis();
    let payload = serde_json::to_string(&Event::TurnEnd {
        stop_reason: "EndTurn".into(),
        usage: Some(TurnUsage {
            total_tokens: 2_200,
            input_tokens: 2_000,
            output_tokens: 200,
            cached_input_tokens: Some(1_500),
            uncached_input_tokens: Some(500),
            cache_write_tokens: None,
            reasoning_tokens: None,
        }),
    })
    .expect("serialize");
    for (sid, seq) in [
        ("sess-500", 1_i64),
        ("sess-501", 1_i64),
        ("sess-502", 2_i64),
    ] {
        sqlx::query("INSERT INTO events (session_id, seq, payload, ts_ms) VALUES (?, ?, ?, ?)")
            .bind(sid)
            .bind(seq)
            .bind(&payload)
            .bind(now)
            .execute(&raw)
            .await
            .expect("raw event insert");
    }
    raw.close().await;

    // H1a: Clear sess-500 BEFORE any backfill -> unprojected turn must survive.
    store
        .clear_events("sess-500", now + 10)
        .await
        .expect("clear s500");
    let m500 = store
        .session_cache_metrics("sess-500", Some(owner), "claude-code", acct, 10)
        .await
        .expect("m500");
    assert_eq!(m500.totals.total_turns, 1);
    assert_eq!(m500.totals.cached_read_tokens, Some(1_500));
    assert_eq!(
        m500.by_model[0].model, "unknown",
        "pre-clear backfill must not fabricate historical model"
    );

    // H1b: Delete + purge sess-501 BEFORE any backfill -> unprojected turn must survive.
    store.delete_session("sess-501").await.expect("delete s501");
    assert_eq!(store.purge_deleted(0).await.expect("purge s501"), 1);
    let m501 = store
        .session_cache_metrics("sess-501", Some(owner), "claude-code", acct, 10)
        .await
        .expect("m501");
    assert_eq!(m501.totals.total_turns, 1);
    assert_eq!(m501.totals.cached_read_tokens, Some(1_500));

    // M1: Startup reconcile resets cursor and projects sess-502 seq=2 without
    // overwriting live seq=1 attribution (claude-opus-4-6).
    store
        .begin_turn_cache_reconcile()
        .await
        .expect("begin reconcile");
    let rec = store
        .backfill_turn_cache_facts_batch(50, DEFAULT_RETENTION_DAYS)
        .await
        .expect("reconcile");
    assert!(rec.completed);
    assert_eq!(rec.rows_projected, 1, "only missing seq=2 should insert");
    let m502 = store
        .session_cache_metrics("sess-502", Some(owner), "claude-code", acct, 10)
        .await
        .expect("m502");
    assert_eq!(m502.totals.total_turns, 2);
    assert!(
        m502.by_model.iter().any(|m| m.model == "claude-opus-4-6"),
        "live attribution on seq=1 must survive reconcile"
    );
    assert!(
        m502.by_model.iter().any(|m| m.model == "unknown"),
        "reconciled historical seq=2 must leave model unknown"
    );
}

#[tokio::test]
async fn sqlite_independent_cache_write_and_large_token_sums_never_overflow() {
    let (_dir, store) = sqlite_test_store().await;
    let owner = "dddddddddddddddddddddddddddddddd";
    let acct = "44444444-4444-4444-4444-444444444444";
    store
        .insert_user(&sample_user(owner, "dave"))
        .await
        .expect("owner");
    store
        .insert_session(&sample_meta("sess-600", owner, "claude-code", acct))
        .await
        .expect("s600");
    store
        .insert_session(&sample_meta("sess-601", owner, "claude-code", acct))
        .await
        .expect("s601");

    // R2-M5: Turn reporting cache_write_tokens WITHOUT cached_read_tokens must
    // still report observed_cache_write_turns and cache_write_tokens cleanly
    // while keeping other_uncached_input_tokens None (no mismatched 3-way bar).
    store
        .upsert_event_batch(
            &[Envelope {
                session_id: "sess-600".into(),
                seq: 1,
                event: Event::TurnEnd {
                    stop_reason: "EndTurn".into(),
                    usage: Some(TurnUsage {
                        total_tokens: 1_100,
                        input_tokens: 1_000,
                        output_tokens: 100,
                        cached_input_tokens: None,
                        uncached_input_tokens: None,
                        cache_write_tokens: Some(400),
                        reasoning_tokens: None,
                    }),
                },
                cmid: None,
            }],
            &HashMap::from([("sess-600".into(), 2)]),
        )
        .await
        .expect("write-only turn");
    let m600 = store
        .session_cache_metrics("sess-600", Some(owner), "claude-code", acct, 10)
        .await
        .expect("m600");
    assert_eq!(m600.totals.observed_cache_turns, 0);
    assert_eq!(m600.totals.observed_cache_write_turns, 1);
    assert_eq!(m600.totals.cache_write_tokens, Some(400));
    assert_eq!(m600.totals.other_uncached_input_tokens, None);
    assert!(!m600.totals.totals_saturated);

    // R2-L3: 1,030 turns at MAX_SAFE_TOKENS (2^53 - 1) would overflow SQLite
    // 64-bit SUM (> 2^63 - 1). Two-limb accumulation must succeed without 500,
    // compute exact cached_input_share, and set totals_saturated = true.
    let per_turn_input = MAX_SAFE_TOKENS - 1_000;
    let per_turn_cached = per_turn_input / 2;
    let per_turn_uncached = per_turn_input - per_turn_cached;
    let mut envelopes = Vec::with_capacity(1_030);
    for seq in 1..=1_030_u64 {
        envelopes.push(Envelope {
            session_id: "sess-601".into(),
            seq,
            event: Event::TurnEnd {
                stop_reason: "EndTurn".into(),
                usage: Some(TurnUsage {
                    total_tokens: per_turn_input + 1_000,
                    input_tokens: per_turn_input,
                    output_tokens: 1_000,
                    cached_input_tokens: Some(per_turn_cached),
                    uncached_input_tokens: Some(per_turn_uncached),
                    cache_write_tokens: None,
                    reasoning_tokens: None,
                }),
            },
            cmid: None,
        });
    }
    store
        .upsert_event_batch(&envelopes, &HashMap::from([("sess-601".into(), 1_031)]))
        .await
        .expect("1030 huge turns");
    let m601 = store
        .session_cache_metrics("sess-601", Some(owner), "claude-code", acct, 5)
        .await
        .expect("m601 no overflow");
    assert_eq!(m601.totals.total_turns, 1_030);
    assert!(m601.totals.totals_saturated);
    assert_eq!(m601.totals.inclusive_input_tokens, Some(MAX_SAFE_TOKENS));
    assert!((m601.totals.cached_input_share.unwrap() - 0.5).abs() < 1e-9);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn sqlite_reused_session_id_incarnations_and_pre_clear_rollover_stay_isolated() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("incarnations.sqlite3");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .expect("connect sqlite");
    store.migrate().await.expect("migrate sqlite");

    let owner = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    let acct = "55555555-5555-5555-5555-555555555555";
    store
        .insert_user(&sample_user(owner, "erin"))
        .await
        .expect("owner");
    store
        .insert_session(&sample_meta("sess-700", owner, "claude-code", acct))
        .await
        .expect("s700-1");
    store
        .update_config_preferences("sess-700", &json!({"model": "claude-opus-4-6"}))
        .await
        .expect("prefs 1");
    store
        .upsert_event_batch(
            &[Envelope {
                session_id: "sess-700".into(),
                seq: 1,
                event: Event::TurnEnd {
                    stop_reason: "EndTurn".into(),
                    usage: Some(TurnUsage {
                        total_tokens: 1_100,
                        input_tokens: 1_000,
                        output_tokens: 100,
                        cached_input_tokens: Some(800),
                        uncached_input_tokens: Some(200),
                        cache_write_tokens: None,
                        reasoning_tokens: None,
                    }),
                },
                cmid: None,
            }],
            &HashMap::from([("sess-700".into(), 2)]),
        )
        .await
        .expect("turn inc1");
    store.delete_session("sess-700").await.expect("delete inc1");
    assert_eq!(store.purge_deleted(0).await.expect("purge inc1"), 1);

    // Rolled-back predecessor reuses sess-700 at a strictly newer created_at_ms (T2)
    store
        .insert_session(&sample_meta("sess-700", owner, "claude-code", acct))
        .await
        .expect("s700-2");
    let raw = sqlx::SqlitePool::connect(&url).await.expect("raw sqlite");
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("UPDATE sessions SET created_at_ms = created_at_ms + 5000 WHERE id = 'sess-700'")
        .execute(&raw)
        .await
        .expect("bump created_at_ms");
    let raw_turn1 = serde_json::to_string(&Event::TurnEnd {
        stop_reason: "EndTurn".into(),
        usage: Some(TurnUsage {
            total_tokens: 2_200,
            input_tokens: 2_000,
            output_tokens: 200,
            cached_input_tokens: Some(1_000),
            uncached_input_tokens: Some(1_000),
            cache_write_tokens: None,
            reasoning_tokens: None,
        }),
    })
    .expect("ser");
    sqlx::query(
        "INSERT INTO events (session_id, seq, payload, ts_ms) VALUES ('sess-700', 1, ?, ?)",
    )
    .bind(&raw_turn1)
    .bind(now - 2_000)
    .execute(&raw)
    .await
    .expect("raw inc2 turn1");
    raw.close().await;

    store
        .begin_turn_cache_reconcile()
        .await
        .expect("begin reconcile");
    let progress = store
        .backfill_turn_cache_facts_batch(50, DEFAULT_RETENTION_DAYS)
        .await
        .expect("reconcile inc2");
    assert!(progress.completed);
    assert_eq!(progress.rows_projected, 1);

    // R3-L2: Pre-clear rollover followed by context_cleared before TurnEnd -> renewed=false
    store
        .upsert_event_batch(
            &[
                Envelope {
                    session_id: "sess-700".into(),
                    seq: 2,
                    event: Event::Update {
                        update: json!({ "sessionUpdate": "context_rolled_over" }),
                    },
                    cmid: None,
                },
                Envelope {
                    session_id: "sess-700".into(),
                    seq: 3,
                    event: Event::Update {
                        update: json!({ "sessionUpdate": "context_cleared" }),
                    },
                    cmid: None,
                },
                Envelope {
                    session_id: "sess-700".into(),
                    seq: 4,
                    event: Event::Update {
                        update: json!({
                            "sessionUpdate": "user_message_chunk",
                            "promptOrigin": { "actor": "human" },
                            "content": { "type": "text", "text": "fresh after clear" }
                        }),
                    },
                    cmid: None,
                },
                Envelope {
                    session_id: "sess-700".into(),
                    seq: 5,
                    event: Event::TurnEnd {
                        stop_reason: "EndTurn".into(),
                        usage: Some(TurnUsage {
                            total_tokens: 3_300,
                            input_tokens: 3_000,
                            output_tokens: 300,
                            cached_input_tokens: Some(2_700),
                            uncached_input_tokens: Some(300),
                            cache_write_tokens: None,
                            reasoning_tokens: None,
                        }),
                    },
                    cmid: None,
                },
            ],
            &HashMap::from([("sess-700".into(), 6)]),
        )
        .await
        .expect("post-clear turn");

    let live_sess = store
        .session_cache_metrics("sess-700", Some(owner), "claude-code", acct, 10)
        .await
        .expect("live sess");
    assert_eq!(live_sess.totals.total_turns, 2);
    assert_eq!(live_sess.totals.renewed_turns, 0);
    assert_eq!(live_sess.totals.inclusive_input_tokens, Some(5_000));
    assert!(
        live_sess
            .by_model
            .iter()
            .all(|m| m.model != "claude-opus-4-6")
    );
    assert_eq!(live_sess.recent_turns[0].turn_end_seq, 5);
    assert!(!live_sess.recent_turns[0].renewed);
    assert_eq!(live_sess.recent_turns[0].idle_gap_ms, None);

    let acct_metrics = store
        .account_cache_metrics(
            owner,
            "claude-code",
            acct,
            now - 86_400_000,
            now + 60_000,
            "24h",
        )
        .await
        .expect("acct metrics");
    assert_eq!(acct_metrics.totals.total_turns, 3);
    assert_eq!(acct_metrics.totals.inclusive_input_tokens, Some(6_000));
}
