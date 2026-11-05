use super::provider_auth_sync_required_before_install;
use crate::machine_protocol::{
    PluginInstallationState, PluginInventory, ProviderMaterializationState, ProviderReplicaState,
};
use crate::provider_service::{
    ProviderAuthenticationStatus, ServiceAuthenticationState, ServiceDistributionState,
};

fn authentication(auth_generation: u64) -> ProviderAuthenticationStatus {
    ProviderAuthenticationStatus {
        provider_id: "codex".to_owned(),
        account_id: crate::provider_account::ProviderAccountId::default(),
        recipient_key: None,
        auth_generation,
        authentication_state: ServiceAuthenticationState::Ready,
        distribution_state: ServiceDistributionState::Failed,
        auth_contract_fingerprint: "sha256:new-auth-contract".to_owned(),
        authentication_scope: "codex-auth-v1".to_owned(),
        portable_schema: "codex-auth-v1".to_owned(),
        projection_schema: "codex-home-v1".to_owned(),
        account_label: None,
        updated_at_ms: 1,
    }
}

fn installed(auth_generation: Option<u64>, replica_state: ProviderReplicaState) -> PluginInventory {
    PluginInventory {
        plugin_id: "codex".to_owned(),
        plugin_version: "1.1.1".to_owned(),
        plugin_kind: cowboy_plugin_sdk::PluginKind::AgentProvider,
        generation_digest: "sha256:old-provider".to_owned(),
        installation_revision: None,
        contract_fingerprint: "sha256:old-contract".to_owned(),
        state: PluginInstallationState::Active,
        rollback_generation_digest: None,
        active_session_leases: 0,
        auth_generation,
        replica_state,
        materialization_state: ProviderMaterializationState::Failed,
        detail: None,
    }
}

#[test]
fn current_sealed_replica_allows_an_incompatible_provider_upgrade() {
    let authentication = authentication(2);
    let installed = installed(Some(2), ProviderReplicaState::Current);

    assert!(!provider_auth_sync_required_before_install(
        Some(&authentication),
        Some(&installed),
    ));
}

#[test]
fn missing_or_stale_replica_still_requires_pre_install_sync() {
    let authentication = authentication(2);

    assert!(provider_auth_sync_required_before_install(
        Some(&authentication),
        None,
    ));
    assert!(provider_auth_sync_required_before_install(
        Some(&authentication),
        Some(&installed(Some(1), ProviderReplicaState::Current)),
    ));
    assert!(provider_auth_sync_required_before_install(
        Some(&authentication),
        Some(&installed(Some(2), ProviderReplicaState::Failed)),
    ));
}

#[test]
fn signed_out_provider_does_not_require_auth_sync() {
    assert!(!provider_auth_sync_required_before_install(None, None));
}
