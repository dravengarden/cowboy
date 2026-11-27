use super::*;
use crate::{agent_sink::AgentSink as _, provider, worker::RemoteSink};

pub(super) fn fixture_input(provider: &str) -> (WorkerArgs, PreparedLaunch) {
    let mut spec = provider::remote_generation(provider).unwrap();
    spec.package_remove_env_prefixes.extend([
        "OTEL_".into(),
        "BETA_TRACING_".into(),
        "ENABLE_BETA_TRACING_DETAILED".into(),
        "CLAUDE_CODE_ENABLE_TELEMETRY".into(),
    ]);
    spec.remove_env.extend([
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ]);
    spec.env
        .insert("CODEX_PATH".into(), "/fixture/codex".into());
    let args = WorkerArgs {
        mcp_plan: None,
        socket: "/fixture/broker".into(),
        session_id: "fixture-session".into(),
        provider: provider.into(),
        provider_version: "test".into(),
        provider_generation_digest: "fixture-signed-generation".into(),
        provider_auth_generation: None,
        provider_account_id: Default::default(),
        cwd: "/fixture/workspace".into(),
        resume: None,
        native_fork: None,
        system: false,
        generation: "test".into(),
        worker_epoch: None,
        fallback_for: None,
        execution_binding: None,
    };
    (
        args,
        PreparedLaunch {
            spec,
            sidecars: Vec::new(),
            execution_jsonrpc: false,
            mcp_bindings: false,
            codex_request_observation: false,
            preset_options: Default::default(),
        },
    )
}

fn client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
}

#[tokio::test]
async fn claude_routes_both_standard_signals_to_one_private_listener() {
    let (args, prepared) = fixture_input("claude-code");
    let mut spec = prepared.spec.clone();
    let _collector = Collector::start(&mut spec, &args, &prepared, "test")
        .await
        .unwrap();
    let base = &spec.env["BETA_TRACING_ENDPOINT"];
    let client = client();
    for (signal, endpoint) in [
        ("logs", &spec.env["OTEL_EXPORTER_OTLP_LOGS_ENDPOINT"]),
        ("traces", &spec.env["OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"]),
    ] {
        assert_eq!(endpoint, &format!("{base}/v1/{signal}"));
        let response = client.post(endpoint).body("{}").send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn codex_uses_a_signed_adapter_binding_without_exporter_overrides_or_argv_secrets() {
    let (mut args, mut prepared) = fixture_input("codex");
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join(".codex");
    std::fs::create_dir(&project).unwrap();
    let explicit_policy = "[otel]\nexporter = \"none\"\n";
    std::fs::write(project.join("config.toml"), explicit_policy).unwrap();
    args.cwd = root.path().into();
    let mut spec = prepared.spec.clone();
    spec.args
        .extend(["-c".into(), "approval_policy=never".into()]);
    let argv = spec.args.clone();
    assert!(
        Collector::start(&mut spec, &args, &prepared, "test")
            .await
            .is_none()
    );
    assert!(!spec.env.contains_key(CODEX_REQUEST_OBSERVATION_ENV));

    prepared.codex_request_observation = true;
    let collector = Collector::start(&mut spec, &args, &prepared, "test")
        .await
        .unwrap();
    assert_eq!(spec.args, argv);
    assert!(spec.env.keys().all(|key| !key.starts_with("OTEL_")));
    assert_eq!(
        std::fs::read_to_string(project.join("config.toml")).unwrap(),
        explicit_policy
    );
    let binding = serde_json::from_str::<Value>(&spec.env[CODEX_REQUEST_OBSERVATION_ENV]).unwrap();
    let key = binding["header_value"].as_str().unwrap();
    let endpoint = binding["logs_endpoint"].as_str().unwrap();
    assert!(!endpoint.contains(key));
    let header = binding["header_name"].as_str().unwrap();
    let client = client();
    let post = || client.post(endpoint).body("{}");
    for (request, expected) in [
        (post(), StatusCode::UNAUTHORIZED),
        (post().header(header, "wrong"), StatusCode::UNAUTHORIZED),
        (
            post().header(header, key).header(header, "wrong"),
            StatusCode::UNAUTHORIZED,
        ),
        (post().header(header, key), StatusCode::OK),
    ] {
        assert_eq!(request.send().await.unwrap().status(), expected);
        assert_eq!(
            collector.observations.as_ref().unwrap().lock().attached,
            expected == StatusCode::OK
        );
    }
}

#[test]
fn fork_callback_updates_attribution_and_keeps_its_existing_wire_event() {
    let (mut shared, _notify) = crate::worker::shared();
    let (observer, facts) = Observer::new(Some("source".into()));
    Arc::get_mut(&mut shared).unwrap().request_observer = Some(observer);
    let sink = RemoteSink {
        shared: Arc::clone(&shared),
    };
    sink.set_forked_agent_session_id("sess-1", "fork".into());
    assert_eq!(facts.native_id.borrow().as_deref(), Some("fork"));
    assert!(matches!(
        shared.outbox.lock().get(&1),
        Some(crate::runtime_wire::RuntimeEvent::AgentSessionId {
            agent_session_id,
            materialized: true,
        }) if agent_session_id == "fork"
    ));
}

#[tokio::test]
async fn antigravity_collector_attaches_without_opening_listener_or_mutating_env() {
    let (args, prepared) = fixture_input("antigravity");
    let mut spec = prepared.spec.clone();
    let original_env = spec.env.clone();
    let collector = Collector::start(&mut spec, &args, &prepared, "epoch-1")
        .await
        .expect("signed antigravity worker attaches proxy diagnostic recorder");
    assert_eq!(spec.env, original_env);
    assert!(collector.observations.is_none());
    assert!(collector.antigravity.is_some());
    collector.observer.prompt_started(1_700_000_000_000);
    collector.observer.observe_config_options(
        &serde_json::json!([{"id": "model", "currentValue": "gemini-4-neon"}]),
    );
    let meta = serde_json::json!({
        "cowboy": {
            "antigravity_backend": { "class": "ok", "successes": 3, "transient": 0 }
        }
    });
    collector.observer.record_prompt_observation(
        "EndTurn",
        Some(crate::agent_model::TurnUsage {
            total_tokens: 1000,
            input_tokens: 900,
            output_tokens: 100,
            cached_input_tokens: Some(700),
            uncached_input_tokens: Some(200),
            cache_write_tokens: None,
            reasoning_tokens: Some(40),
        }),
        meta.as_object(),
    );
}

#[test]
fn launch_prefix_fingerprint_is_deterministic_and_sensitive_to_matrix_and_structural_changes() {
    let (mut a, prep_a) = fixture_input("codex");
    let mut b = a.clone();
    let mut spec_b = prep_a.spec.clone();
    a.mcp_plan = Some(Box::new(crate::mcp_binding::NativeLaunchPlan {
        schema: 1,
        command: "/bin/cowboy".into(),
        bindings: vec![
            crate::mcp_binding::NativeLaunchBinding {
                name: "zeta".into(),
                descriptor: "/tmp/z".into(),
            },
            crate::mcp_binding::NativeLaunchBinding {
                name: "alpha".into(),
                descriptor: "/tmp/a".into(),
            },
        ],
    }));
    b.mcp_plan = Some(Box::new(crate::mcp_binding::NativeLaunchPlan {
        schema: 1,
        command: "/bin/cowboy".into(),
        bindings: vec![
            crate::mcp_binding::NativeLaunchBinding {
                name: "alpha".into(),
                descriptor: "/tmp/other-descriptor-path".into(),
            },
            crate::mcp_binding::NativeLaunchBinding {
                name: "zeta".into(),
                descriptor: "/tmp/other-path-2".into(),
            },
        ],
    }));
    let fp_a = metadata::compute_launch_prefix_fingerprint(&a, &prep_a.spec);
    let fp_b = metadata::compute_launch_prefix_fingerprint(&b, &spec_b);
    assert_eq!(fp_a.len(), 16);
    assert!(fp_a.bytes().all(|ch| ch.is_ascii_hexdigit()));
    assert_eq!(fp_a, fp_b);

    // Matrix memory enrollment changes the launch signature without hashing secret values
    spec_b
        .env
        .insert("COWBOY_MEMORY_DESCRIPTOR".into(), "/tmp/mem.json".into());
    let fp_mem = metadata::compute_launch_prefix_fingerprint(&b, &spec_b);
    assert_ne!(fp_a, fp_mem);

    b.provider_version = "3.6.99".into();
    assert_ne!(
        fp_mem,
        metadata::compute_launch_prefix_fingerprint(&b, &spec_b)
    );
}

#[tokio::test]
async fn codex_listener_declines_when_node_debug_is_set() {
    let (args, mut prepared) = fixture_input("codex");
    prepared.codex_request_observation = true;
    let mut spec = prepared.spec.clone();
    spec.env.insert("NODE_DEBUG".into(), "child_process".into());
    assert!(
        Collector::start(&mut spec, &args, &prepared, "test")
            .await
            .is_none()
    );
    assert!(!spec.env.contains_key(CODEX_REQUEST_OBSERVATION_ENV));
}

#[test]
fn launch_prefix_fingerprint_detects_session_extensions_memory_and_call_context() {
    let dir = tempfile::tempdir().unwrap();
    let ext_no_mem = dir.path().join("ext-no-mem.json");
    let ext_with_mem = dir.path().join("ext-with-mem.json");
    std::fs::write(&ext_no_mem, r#"{"schema":1,"memory":null}"#).unwrap();
    std::fs::write(
        &ext_with_mem,
        r#"{"schema":1,"memory":{"server":"matrix"}}"#,
    )
    .unwrap();

    let (args, prep) = fixture_input("codex");
    let mut s1 = prep.spec.clone();
    s1.env.insert(
        "COWBOY_SESSION_EXTENSIONS".into(),
        ext_no_mem.display().to_string(),
    );
    s1.remove_env.push("COWBOY_CALL_CONTEXT");
    let mut s2 = s1.clone();
    s2.env.insert(
        "COWBOY_SESSION_EXTENSIONS".into(),
        ext_with_mem.display().to_string(),
    );
    let fp1 = metadata::compute_launch_prefix_fingerprint(&args, &s1);
    let fp2 = metadata::compute_launch_prefix_fingerprint(&args, &s2);
    assert_ne!(fp1, fp2);

    let mut s3 = s2.clone();
    s3.env
        .insert("COWBOY_CALL_CONTEXT".into(), "/tmp/call-ctx.json".into());
    assert_ne!(fp2, metadata::compute_launch_prefix_fingerprint(&args, &s3));

    // When COWBOY_SESSION_EXTENSIONS explicitly disables memory, legacy COWBOY_MATRIX_CONFIG does not override it.
    let mut s1_legacy = s1.clone();
    s1_legacy.env.insert(
        "COWBOY_MATRIX_CONFIG".into(),
        "/tmp/legacy-matrix.json".into(),
    );
    assert_eq!(
        fp1,
        metadata::compute_launch_prefix_fingerprint(&args, &s1_legacy)
    );
    assert_ne!(
        fp2,
        metadata::compute_launch_prefix_fingerprint(&args, &s1_legacy)
    );
}

#[test]
fn codex_earliest_pending_start_subtracts_queued_start_duration() {
    let mut pending = codex::Pending::default();
    let rec = serde_json::json!({
        "timeUnixNano": "200000000000",
        "body": { "stringValue": "codex.api_request" },
        "attributes": [
            { "key": "event.name", "value": { "stringValue": "codex.api_request" } },
            { "key": "endpoint", "value": { "stringValue": "/responses" } },
            { "key": "conversation.id", "value": { "stringValue": "t1" } },
            { "key": "duration_ms", "value": { "intValue": "150000" } },
            { "key": "http.response.status_code", "value": { "intValue": "200" } }
        ]
    });
    assert_eq!(pending.ingest(&rec, 200_000), 0);
    // 200_000ms - 150_000ms duration = 50_000ms actual start
    assert_eq!(pending.earliest_pending_start_ms(), Some(50_000));
}

#[test]
fn epoch_ordering_stays_unknown_when_loss_precedes_first_or_earlier_request_is_pending() {
    let (args, prep) = fixture_input("claude-code");
    let (_obs, facts) = Observer::new(None);
    let mut state = Observations::new(
        Producer::Claude,
        &args,
        "ep-loss",
        metadata::compute_launch_prefix_fingerprint(&args, &prep.spec),
        facts,
    );
    state.dropped = 1;
    state.flush(1_000, false);
    assert!(state.epoch_order_unproven);
    assert!(!state.epoch_first_claimed);

    let (_obs2, facts2) = Observer::new(None);
    let mut state2 = Observations::new(
        Producer::Claude,
        &args,
        "ep-pending",
        metadata::compute_launch_prefix_fingerprint(&args, &prep.spec),
        facts2,
    );
    let later = Attempt {
        id: "late-1".into(),
        thread: "t1".into(),
        role: "parent",
        outcome: "usage",
        start_source: "span",
        usage_source: "log",
        retention: "unknown",
        start_ms: 200,
        end_ms: 250,
        tokens: Tokens::default(),
        overage: None,
        status: Some(200),
        cost: None,
        model: None,
        native_release: None,
        turn: None,
    };
    state2.record(later, Some(100));
    assert!(!state2.epoch_first_claimed);
    assert!(state2.epoch_order_unproven);
}
