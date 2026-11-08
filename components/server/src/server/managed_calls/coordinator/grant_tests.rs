use super::*;

fn authority(epoch: &str) -> Authority {
    let mut parent: crate::core::SessionMeta = serde_json::from_value(json!({
        "id":"parent", "provider":"claude-code", "machine_id":"ovh",
        "workspace_id":"cowboy", "cwd":"/runtime/session", "title":"MCP",
        "status":"running", "provider_version":"1.0.0",
        "provider_generation_digest":"generation"
    }))
    .unwrap();
    let binding = crate::execution_environment::fixture();
    let mut behavior = crate::provider::legacy_behavior("claude-code");
    behavior.execution = Some(cowboy_provider_sdk::ExecutionBehavior::JsonrpcV1 {
        executor_digests: [binding.decode().unwrap().environment.executor_digest].into(),
    });
    parent.provider_behavior = Some(behavior);
    parent.execution_binding = Some(binding);
    let worker = serde_json::from_value(json!({
        "session_id":parent.id,"worker_epoch":epoch,"generation":"build","state":"running",
        "launch":{"session_id":parent.id,"provider":parent.provider,
            "provider_version":parent.provider_version,"provider_generation_digest":parent.provider_generation_digest,
            "provider_behavior":parent.provider_behavior,"cwd":parent.cwd,"system":false,
            "generation":"build","execution_binding":parent.execution_binding}
    })).unwrap();
    Authority::for_worker("service-test", &parent, &worker).unwrap()
}

#[test]
fn late_install_ack_requires_the_original_authority_epoch_and_connection() {
    let entry = GrantEntry {
        grant: Grant {
            grant_id: "grant-test".into(),
            parent_session_id: "parent".into(),
            agent_api: true,
            agent_actions: true,
            mcp: true,
        },
        authority: authority("worker-one"),
        mcp_epoch: Some("policy-one".into()),
        installed_on: None,
        retry_after: None,
    };
    let current = (authority("worker-one"), Some("policy-one".into()));
    assert!(entry.accepts_installation(Some(&current), true, true));
    assert!(!entry.accepts_installation(Some(&current), false, true));
    assert!(!entry.accepts_installation(Some(&current), true, false));
    assert!(!entry.accepts_installation(None, true, true));
    assert!(!entry.accepts_installation(
        Some(&(authority("worker-two"), current.1.clone())),
        true,
        true
    ));
    assert!(!entry.accepts_installation(Some(&(current.0.clone(), None)), true, true));
    assert!(!entry.accepts_installation(
        Some(&(current.0, Some("policy-restored".into()))),
        true,
        true
    ));
}
