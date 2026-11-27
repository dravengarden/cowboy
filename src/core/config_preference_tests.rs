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
fn new_codex_sessions_start_with_sol_6_2_medium_preferences() {
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
            "model": "gpt-6.2-sol",
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
            "deepseek_cache_protection": false,
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
            "deepseek_cache_protection": false,
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
    assert_eq!(options[2]["currentValue"], false);
    assert_eq!(options[3]["id"], "reasoning_effort");
}

#[test]
fn dormant_choices_are_display_only_and_end_with_the_next_agent_snapshot() {
    let hub = Hub::new();
    hub.create_local_session(
        "dormant".to_owned(),
        "codex".to_owned(),
        "/tmp".to_owned(),
        "test".to_owned(),
        SessionOrigin::Web,
        false,
    );
    let reported = serde_json::json!([
        {"id": "model", "category": "model", "currentValue": "gpt-6.1-sol",
         "options": [{"value": "gpt-6.1-sol", "name": "6.1 Sol"}]},
        {"id": "mode", "currentValue": "ask",
         "options": [{"group": "Profiles", "options": [{"value": "ask", "name": "Ask"}]}]},
    ]);
    hub.set_config_options("dormant", reported.clone());
    let repin = |hub: &Hub, digest: &str| {
        hub.inner
            .sessions
            .lock()
            .get_mut("dormant")
            .unwrap()
            .meta
            .provider_generation_digest = digest.to_owned();
        hub.session_info("dormant").unwrap().meta
    };
    let choices = || -> std::collections::BTreeMap<String, Vec<serde_json::Value>> {
        [
            (
                "model".to_owned(),
                vec![
                    serde_json::json!({"value": "gpt-6.1-sol", "name": "duplicate"}),
                    serde_json::json!({"value": "gpt-6.2-sol", "name": "6.2 Sol"}),
                ],
            ),
            (
                "mode".to_owned(),
                vec![serde_json::json!({"value": "bypass", "name": "Bypass"})],
            ),
        ]
        .into()
    };
    let shown = |hub: &Hub| {
        hub.config_options("dormant").unwrap()[0]["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|choice| choice["value"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };

    // A session with a worker reports its own options.
    hub.set_status("dormant", Status::Running, None);
    let live = hub.session_info("dormant").unwrap().meta;
    assert!(!hub.set_dormant_config_choices(&live, &reported, choices()));

    hub.set_status("dormant", Status::Exited, None);
    // The pinned generation reported this snapshot: nothing to add.
    let current = hub.session_info("dormant").unwrap().meta;
    assert_eq!(hub.config_reported_by_current(&current), Some(true));
    assert!(!hub.set_dormant_config_choices(&current, &reported, choices()));

    // Moved to another generation while exited.
    let meta = repin(&hub, "sha256:next");
    assert_eq!(hub.config_reported_by_current(&meta), Some(false));
    let mut moved = meta.clone();
    moved.provider_generation_digest = "sha256:other".to_owned();
    assert!(!hub.set_dormant_config_choices(&moved, &reported, choices()));
    // Choices derived from a snapshot a worker has since replaced.
    assert!(!hub.set_dormant_config_choices(&meta, &serde_json::json!([]), choices()));

    assert!(hub.set_dormant_config_choices(&meta, &reported, choices()));
    assert_eq!(shown(&hub), ["gpt-6.1-sol", "gpt-6.2-sol"]);
    // Never persisted, so never replayed as an agent capability.
    assert_eq!(hub.persisted_config_options("dormant").unwrap(), reported);
    // Grouped choices keep the agent's grouping.
    assert_eq!(
        hub.config_options("dormant").unwrap()[1]["options"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(!hub.set_dormant_config_choices(&meta, &reported, choices()));
    // An empty lookup replaces the previous one.
    assert!(hub.set_dormant_config_choices(&meta, &reported, Default::default()));
    assert_eq!(shown(&hub), ["gpt-6.1-sol"]);
    assert!(hub.set_dormant_config_choices(&meta, &reported, choices()));

    // Selecting one is an ordinary preference; the stored snapshot keeps only
    // what the agent reported, with the selection as its current value.
    hub.set_config_preference(
        "dormant",
        "model".to_owned(),
        serde_json::json!("gpt-6.2-sol"),
    )
    .unwrap();
    let stored = hub.persisted_config_options("dormant").unwrap();
    assert_eq!(stored[0]["currentValue"], "gpt-6.2-sol");
    assert_eq!(stored[0]["options"].as_array().unwrap().len(), 1);
    assert_eq!(
        hub.config_preferences("dormant").unwrap()["model"],
        "gpt-6.2-sol"
    );

    // Hidden while a worker runs, and gone after its first snapshot.
    hub.set_status("dormant", Status::Starting, None);
    assert_eq!(shown(&hub), ["gpt-6.1-sol"]);
    hub.set_config_options("dormant", reported.clone());
    hub.set_status("dormant", Status::Exited, None);
    assert_eq!(shown(&hub), ["gpt-6.1-sol"]);
    let current = hub.session_info("dormant").unwrap().meta;
    assert_eq!(hub.config_reported_by_current(&current), Some(true));

    // A report replayed for an exited session may come from a past worker of
    // an earlier generation: its producer is unknown, not the pinned one.
    let meta = repin(&hub, "sha256:later");
    hub.set_config_options("dormant", reported);
    assert_eq!(hub.config_reported_by_current(&meta), None);
}
