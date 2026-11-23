use super::superseding_plugin;
use crate::machine_protocol::{
    ComponentId, ComponentKind, PluginInstallationState, PluginInventory,
};

fn plugin(id: &str, state: PluginInstallationState) -> PluginInventory {
    PluginInventory {
        plugin_id: id.to_owned(),
        plugin_version: "3.1.28".to_owned(),
        plugin_kind: cowboy_plugin_sdk::PluginKind::AgentProvider,
        generation_digest: "sha256:aa".to_owned(),
        installation_revision: None,
        contract_fingerprint: "sha256:bb".to_owned(),
        state,
        rollback_generation_digest: None,
        active_session_leases: 0,
        auth_generation: None,
        replica_state: crate::machine_protocol::ProviderReplicaState::Current,
        materialization_state: crate::machine_protocol::ProviderMaterializationState::Current,
        detail: None,
    }
}

fn component(kind: ComponentKind, slot: &str) -> ComponentId {
    ComponentId {
        kind,
        slot: slot.to_owned(),
    }
}

#[test]
fn independent_draft_routes_require_product_identity_and_operator_writes() {
    use super::{RouteAuth, classify_route};
    use axum::http::Method;
    assert_eq!(
        classify_route(&Method::GET, "/api/drafts"),
        RouteAuth::Product
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/drafts/doc/history"),
        RouteAuth::Product
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/drafts/doc/history/index"),
        RouteAuth::Product
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/drafts/doc/history/7"),
        RouteAuth::Product
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/drafts/mutations"),
        RouteAuth::ProductOperator
    );
    assert_eq!(
        classify_route(&Method::DELETE, "/api/sessions/session/draft-copies/cmid"),
        RouteAuth::ProductSessionMutate
    );
}

#[test]
fn an_installed_plugin_supersedes_the_legacy_binary_in_its_slot() {
    let plugins = [plugin("claude-code", PluginInstallationState::Active)];
    assert_eq!(
        superseding_plugin(&component(ComponentKind::ProviderCli, "claude"), &plugins),
        Some("claude-code".to_owned())
    );
    assert_eq!(
        superseding_plugin(
            &component(ComponentKind::ProviderAdapter, "claude"),
            &plugins
        ),
        Some("claude-code".to_owned())
    );
}

#[test]
fn another_providers_plugin_never_supersedes_this_slot() {
    let plugins = [plugin("codex", PluginInstallationState::Active)];
    assert_eq!(
        superseding_plugin(&component(ComponentKind::ProviderCli, "claude"), &plugins),
        None
    );
}

#[test]
fn only_an_active_installation_supersedes() {
    for state in [
        PluginInstallationState::Missing,
        PluginInstallationState::Installing,
        PluginInstallationState::Failed,
        PluginInstallationState::Incompatible,
        PluginInstallationState::Uninstalling,
    ] {
        let plugins = [plugin("claude-code", state)];
        assert_eq!(
            superseding_plugin(&component(ComponentKind::ProviderCli, "claude"), &plugins),
            None,
            "a Plugin that is not active does not run anybody's session"
        );
    }
}

#[test]
fn a_machine_owned_component_is_never_superseded() {
    let plugins = [plugin("claude-code", PluginInstallationState::Active)];
    for kind in [
        ComponentKind::MachineHost,
        ComponentKind::AcpRuntime,
        ComponentKind::ZedServer,
    ] {
        assert_eq!(
            superseding_plugin(&component(kind, "claude"), &plugins),
            None
        );
    }
}
