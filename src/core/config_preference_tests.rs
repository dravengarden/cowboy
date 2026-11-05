use super::*;

#[test]
fn prelaunch_configuration_keeps_durable_selections_separate_from_agent_snapshots() {
    let hub = Hub::new();
    hub.create_local_session(
        "s".into(),
        "codex".into(),
        "/tmp".into(),
        "Preview".into(),
        SessionOrigin::Web,
        false,
    );
    let meta = hub.session_info("s").unwrap().meta;
    let preview = serde_json::json!([{
        "id":"model", "category":"model", "currentValue":"gpt-6.1-sol",
        "options":[{"value":"gpt-6.1-sol"},{"value":"chosen"}]
    }]);
    hub.set_initial_config_options(&meta, preview.clone());
    hub.set_config_preference("s", "model".into(), serde_json::json!("chosen"))
        .unwrap();
    // Reloading the preview cannot erase the saved selection. No synthetic
    // options may masquerade as a worker response during native hydration.
    hub.set_initial_config_options(&meta, preview.clone());
    assert_eq!(
        hub.config_options("s").unwrap()[0]["currentValue"],
        "chosen"
    );
    assert_eq!(hub.config_preferences("s").unwrap()["model"], "chosen");
    assert_eq!(hub.persisted_config_options("s"), None);
    hub.set_config_options(
        "s",
        serde_json::json!([{
            "id":"model", "category":"model", "currentValue":"replacement",
            "options":[{"value":"replacement"}]
        }]),
    );
    hub.set_initial_config_options(&meta, preview);
    assert_eq!(
        hub.config_options("s").unwrap()[0]["currentValue"],
        "replacement"
    );
    assert_eq!(hub.config_preferences("s").unwrap()["model"], "replacement");
}

#[test]
fn new_codex_sessions_start_with_sol_6_1_medium_preferences() {
    let hub = Hub::new();
    hub.create_local_session(
        "codex-session".to_owned(),
        "codex".to_owned(),
        "/tmp".to_owned(),
        "test".to_owned(),
        SessionOrigin::Web,
        false,
    );

    assert_eq!(
        hub.config_preferences("codex-session"),
        Some(serde_json::json!({
            "model": "gpt-6.1-sol",
            "reasoning_effort": "medium",
        }))
    );
}

#[test]
fn new_grok_sessions_start_with_high_reasoning_and_full_access_without_pinning_a_model() {
    let hub = Hub::new();
    hub.create_local_session(
        "grok-session".to_owned(),
        "grok".to_owned(),
        "/tmp".to_owned(),
        "test".to_owned(),
        SessionOrigin::Web,
        false,
    );

    assert_eq!(
        hub.config_preferences("grok-session"),
        Some(serde_json::json!({
            "permission_mode": "always-approve",
            "reasoning_effort": "high",
        }))
    );
}

#[test]
fn new_claude_deepseek_sessions_start_with_flash_max_default_agent_preferences() {
    let hub = Hub::new();
    hub.create_local_session(
        "claude-deepseek-session".to_owned(),
        "claude-deepseek".to_owned(),
        "/tmp".to_owned(),
        "test".to_owned(),
        SessionOrigin::Web,
        false,
    );

    assert_eq!(
        hub.config_preferences("claude-deepseek-session"),
        Some(serde_json::json!({
            "model": "deepseek-flash[1m]",
            "deepseek_context": "830k",
            "deepseek_cache_protection": true,
            "effort": "max",
            "agent": "default",
        }))
    );
}

#[test]
fn new_codex_deepseek_sessions_start_with_flash_default_collaboration_max_preferences() {
    let hub = Hub::new();
    hub.create_local_session(
        "codex-deepseek-session".to_owned(),
        "codex-deepseek".to_owned(),
        "/tmp".to_owned(),
        "test".to_owned(),
        SessionOrigin::Web,
        false,
    );

    assert_eq!(
        hub.config_preferences("codex-deepseek-session"),
        Some(serde_json::json!({
            "model": "deepseek-flash",
            "deepseek_context": "680k",
            "deepseek_cache_protection": true,
            "collaboration_mode": "default",
            "reasoning_effort": "max",
        }))
    );
}

#[test]
fn selecting_a_config_value_updates_the_shared_snapshot() {
    let hub = Hub::new();
    hub.create_local_session(
        "codex-session".to_owned(),
        "codex".to_owned(),
        "/tmp".to_owned(),
        "test".to_owned(),
        SessionOrigin::Web,
        false,
    );
    hub.set_config_options(
        "codex-session",
        serde_json::json!([{
            "id": "model",
            "currentValue": "gpt-5.6-sol",
            "options": [{"value": "gpt-5.6-sol"}, {"value": "gpt-5.6-luna"}],
        }]),
    );

    hub.set_config_preference(
        "codex-session",
        "model".to_owned(),
        serde_json::json!("gpt-5.6-luna"),
    )
    .expect("valid config preference");

    assert_eq!(
        hub.config_preferences("codex-session")
            .and_then(|value| value.get("model").cloned()),
        Some(serde_json::json!("gpt-5.6-luna"))
    );
    assert_eq!(
        hub.config_options("codex-session")
            .and_then(|value| value[0].get("currentValue").cloned()),
        Some(serde_json::json!("gpt-5.6-luna"))
    );
}

#[test]
fn a_preset_does_not_flip_back_on_the_agents_answer_to_its_first_option() {
    let hub = Hub::new();
    hub.create_local_session(
        "s".to_owned(),
        "claude-code".to_owned(),
        "/tmp".to_owned(),
        "test".to_owned(),
        SessionOrigin::Web,
        false,
    );
    let snapshot = |model: &str, effort: &str| {
        serde_json::json!([
            {"id": "model", "currentValue": model,
             "options": [{"value": "opus"}, {"value": "sonnet"}]},
            {"id": "effort", "currentValue": effort,
             "options": [{"value": "default"}, {"value": "high"}]},
        ])
    };
    let current = |id: usize| {
        hub.config_options("s")
            .and_then(|options| options[id].get("currentValue").cloned())
    };
    hub.set_config_options("s", snapshot("opus", "default"));
    // A preset sends both options before the agent answers either.
    for (id, value) in [("model", "sonnet"), ("effort", "high")] {
        hub.set_config_preference("s", id.to_owned(), serde_json::json!(value))
            .expect("preference");
    }
    // The answer to the model change still carries the old effort.
    hub.set_config_options("s", snapshot("sonnet", "default"));
    assert_eq!(current(1), Some(serde_json::json!("high")));
    hub.set_config_options("s", snapshot("sonnet", "high"));
    assert_eq!(current(1), Some(serde_json::json!("high")));
    // Settled: a later agent-side change is authoritative again.
    hub.set_config_options("s", snapshot("sonnet", "default"));
    assert_eq!(current(1), Some(serde_json::json!("default")));
    assert_eq!(current(0), Some(serde_json::json!("sonnet")));
}

#[test]
fn rapid_preset_taps_never_show_a_stale_intermediate_selection() {
    let hub = Hub::new();
    hub.create_local_session(
        "s".to_owned(),
        "claude-code".to_owned(),
        "/tmp".to_owned(),
        "test".to_owned(),
        SessionOrigin::Web,
        false,
    );
    let snapshot = |model: &str, effort: &str| {
        serde_json::json!([
            {"id": "model", "category": "model", "currentValue": model,
             "options": [{"value": "opus"}, {"value": "sonnet"}]},
            {"id": "effort", "category": "thought_level", "currentValue": effort,
             "options": [{"value": "default"}, {"value": "high"}]},
        ])
    };
    let shown = || {
        let options = hub.config_options("s").expect("options");
        (
            options[0]["currentValue"].clone(),
            options[1]["currentValue"].clone(),
        )
    };
    let target = (serde_json::json!("opus"), serde_json::json!("default"));
    hub.set_config_options("s", snapshot("opus", "default"));
    // Tap Sonnet·High, then immediately back to Opus·Default.
    for (id, value) in [
        ("model", "sonnet"),
        ("effort", "high"),
        ("model", "opus"),
        ("effort", "default"),
    ] {
        hub.set_config_preference("s", id.to_owned(), serde_json::json!(value))
            .expect("preference");
    }
    // The agent answers each command in order. The first answer matches
    // the final effort by coincidence; it must not settle it alone.
    for (model, effort) in [
        ("sonnet", "default"),
        ("sonnet", "high"),
        ("opus", "high"),
        ("opus", "default"),
    ] {
        hub.set_config_options("s", snapshot(model, effort));
        assert_eq!(shown(), target, "after agent answer ({model}, {effort})");
    }
    hub.set_config_options("s", snapshot("opus", "high"));
    assert_eq!(shown().1, serde_json::json!("high"), "settled values yield");
}

#[test]
fn authoritative_options_replace_a_retired_persisted_value() {
    let health = Arc::new(PersistenceHealth::default());
    let (sink, mut rx) = StoreSink::channel(16, health);
    let hub = Hub::with_store(Some(sink));
    hub.create_local_session(
        "codex-session".to_owned(),
        "codex".to_owned(),
        "/tmp".to_owned(),
        "test".to_owned(),
        SessionOrigin::Web,
        false,
    );
    hub.set_config_preference(
        "codex-session",
        "model".to_owned(),
        serde_json::json!("gpt-5.3-codex-spark"),
    )
    .expect("model preference");
    hub.set_config_preference(
        "codex-session",
        "reasoning_effort".to_owned(),
        serde_json::json!("max"),
    )
    .expect("reasoning preference");
    while rx.try_recv().is_ok() {}

    hub.set_config_options(
        "codex-session",
        serde_json::json!([
            {
                "id": "model",
                "currentValue": "gpt-5.3-codex-spark",
                "options": [{"value": "gpt-5.3-codex-spark"}],
            },
            {
                "id": "reasoning_effort",
                "currentValue": "low",
                "options": [
                    {"value": "low"},
                    {"value": "medium"},
                    {"value": "high"},
                    {"value": "xhigh"},
                ],
            },
        ]),
    );

    assert_eq!(
        hub.config_preferences("codex-session"),
        Some(serde_json::json!({
            "model": "gpt-5.3-codex-spark",
            "reasoning_effort": "low",
        }))
    );
    assert_eq!(
        hub.config_options("codex-session")
            .and_then(|value| value[1].get("currentValue").cloned()),
        Some(serde_json::json!("low"))
    );
    assert!(matches!(
        rx.try_recv(),
        Ok(StoreWrite::UpdateConfigOptions { .. })
    ));
    assert!(matches!(
        rx.try_recv(),
        Ok(StoreWrite::UpdateConfigPreferences { preferences, .. })
            if preferences["reasoning_effort"] == serde_json::json!("low")
    ));
}

#[test]
fn previous_model_snapshot_does_not_retire_a_pending_presets_reasoning() {
    let mut preferences = serde_json::json!({"model": "astra", "reasoning_effort": "max"});
    let mut options = serde_json::json!([
        {"id": "model", "category": "model", "currentValue": "spark",
         "options": [{"value": "spark"}, {"value": "astra"}]},
        {"id": "reasoning_effort", "category": "thought_level", "currentValue": "low",
         "options": [{"value": "low"}, {"value": "medium"}]}
    ]);
    assert!(!reconcile_config_preferences(
        Some(&options),
        &mut preferences
    ));
    assert_eq!(preferences["reasoning_effort"], "max");
    options[0]["currentValue"] = serde_json::json!("astra");
    options[1]["options"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"value": "max"}));
    assert!(!reconcile_config_preferences(
        Some(&options),
        &mut preferences
    ));
    assert_eq!(preferences["reasoning_effort"], "max");
    // Once the selected model itself retires the effort, normal repair
    // still applies; a genuinely removed value never survives indefinitely.
    options[1]["options"].as_array_mut().unwrap().pop();
    assert!(reconcile_config_preferences(
        Some(&options),
        &mut preferences
    ));
    assert_eq!(preferences["reasoning_effort"], "low");
}

#[test]
fn deepseek_context_option_is_projected_after_the_model() {
    let hub = Hub::new();
    hub.create_local_session(
        "deepseek-session".to_owned(),
        "codex-deepseek".to_owned(),
        "/tmp".to_owned(),
        "test".to_owned(),
        SessionOrigin::Web,
        false,
    );
    hub.set_config_options(
        "deepseek-session",
        serde_json::json!([
            {"id": "model", "currentValue": "deepseek-v4-flash", "options": []},
            {"id": "reasoning_effort", "currentValue": "max", "options": []},
        ]),
    );

    let options = hub.config_options("deepseek-session").unwrap();
    assert_eq!(options[0]["id"], "model");
    assert_eq!(options[1]["id"], "deepseek_context");
    assert_eq!(options[1]["currentValue"], "680k");
    assert_eq!(options[2]["id"], "deepseek_cache_protection");
    assert_eq!(options[2]["currentValue"], true);
    assert_eq!(options[3]["id"], "reasoning_effort");
}
