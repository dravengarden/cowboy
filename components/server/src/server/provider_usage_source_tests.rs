use super::provider_usage_source_is_authorized;
use crate::machine_protocol::{
    PluginInstallationState, PluginInventory, ProviderMaterializationState, ProviderReplicaState,
};
use crate::plugin_host::{PluginHostSpec, PluginSlotId};
use crate::plugin_runtime::ActivatedHostPlugin;
use std::collections::BTreeSet;

fn activated(id: &str, source: &str) -> ActivatedHostPlugin {
    let spec = PluginHostSpec::from_json(source.as_bytes()).expect("host spec");
    ActivatedHostPlugin {
        id: id.to_owned(),
        plugin_version: Some("1.0.0".to_owned()),
        artifact_digest: Some("sha256:generation".to_owned()),
        generation: "test-generation".to_owned(),
        slots: spec
            .slots
            .iter()
            .map(|slot| PluginSlotId::as_str(*slot))
            .collect(),
        ui: spec.ui.clone(),
        usage: spec.usage,
        label: spec.label,
        adapter_slot: spec.adapter_slot,
        login_fields: spec.login_fields,
        rpc_argv: spec.rpc_argv,
        visual: spec.visual,
        native_capabilities: spec.native_capabilities,
    }
}

fn installed(id: &str, state: PluginInstallationState) -> PluginInventory {
    PluginInventory {
        plugin_id: id.to_owned(),
        plugin_version: "1.0.0".to_owned(),
        plugin_kind: cowboy_plugin_sdk::PluginKind::AgentProvider,
        generation_digest: "sha256:generation".to_owned(),
        installation_revision: None,
        contract_fingerprint: "sha256:contract".to_owned(),
        state,
        rollback_generation_digest: None,
        active_session_leases: 0,
        auth_generation: None,
        replica_state: ProviderReplicaState::Absent,
        materialization_state: ProviderMaterializationState::NotInstalled,
        detail: None,
    }
}

#[test]
fn signed_host_data_authorizes_a_future_usage_producer() {
    let plugins = [installed("future-runtime", PluginInstallationState::Active)];
    let runtime = crate::plugin_runtime::PluginRuntime::from_default_hosts(vec![
        activated(
            "future-runtime",
            r#"{"schema_version":1,"adapter_slot":"future-agent"}"#,
        ),
        activated(
            "future-usage",
            r#"{"schema_version":1,"usage":{"account":"future-account","activity":true,"activity_agents":[{"id":"future-agent","label":"Future Agent"}]}}"#,
        ),
    ]);
    assert!(provider_usage_source_is_authorized(
        &plugins,
        &runtime,
        &BTreeSet::new(),
        "future-runtime",
        "future-account",
        "future-agent",
    ));
    assert!(!provider_usage_source_is_authorized(
        &plugins,
        &runtime,
        &BTreeSet::new(),
        "future-runtime",
        "other-account",
        "future-agent",
    ));
    assert!(!provider_usage_source_is_authorized(
        &plugins,
        &runtime,
        &BTreeSet::new(),
        "future-runtime",
        "future-account",
        "other-agent",
    ));
}

#[test]
fn shared_gateway_reports_usage_for_its_consuming_agent() {
    let plugins = [installed(
        "claude-deepseek",
        PluginInstallationState::Active,
    )];
    let runtime = crate::plugin_runtime::PluginRuntime::from_default_hosts(vec![activated(
        "claude-deepseek",
        r#"{"schema_version":1,"adapter_slot":"claude","usage":{"account":"deepseek","activity":true,"activity_agents":[{"id":"claude","label":"Claude Code"},{"id":"deepseek-harness","label":"DeepSeek Harness"}]}}"#,
    )]);
    let shared = BTreeSet::from([("claude-deepseek".to_owned(), "deepseek-harness".to_owned())]);
    assert!(provider_usage_source_is_authorized(
        &plugins,
        &runtime,
        &shared,
        "claude-deepseek",
        "deepseek",
        "deepseek-harness",
    ));
    assert!(!provider_usage_source_is_authorized(
        &plugins,
        &runtime,
        &BTreeSet::new(),
        "claude-deepseek",
        "deepseek",
        "deepseek-harness",
    ));
    assert!(!provider_usage_source_is_authorized(
        &plugins,
        &runtime,
        &shared,
        "codex-deepseek",
        "deepseek",
        "deepseek-harness",
    ));
}

#[test]
fn inactive_plugin_cannot_publish_usage_events() {
    let plugins = [installed(
        "future-runtime",
        PluginInstallationState::Missing,
    )];
    let runtime = crate::plugin_runtime::PluginRuntime::from_default_hosts(vec![
        activated(
            "future-runtime",
            r#"{"schema_version":1,"adapter_slot":"future-agent"}"#,
        ),
        activated(
            "future-usage",
            r#"{"schema_version":1,"usage":{"account":"future-account","activity":true}}"#,
        ),
    ]);
    assert!(!provider_usage_source_is_authorized(
        &plugins,
        &runtime,
        &BTreeSet::new(),
        "future-runtime",
        "future-account",
        "future-agent",
    ));
}
