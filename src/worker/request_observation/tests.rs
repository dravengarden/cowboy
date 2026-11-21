use super::*;
use crate::{agent_sink::AgentSink as _, provider, worker::RemoteSink};

fn input(provider: &str) -> (WorkerArgs, PreparedLaunch) {
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
    let (args, prepared) = input("claude-code");
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
    let (mut args, mut prepared) = input("codex");
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
            collector.observations.lock().attached,
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
