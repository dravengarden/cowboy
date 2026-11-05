
use super::{
    ProviderAuthExecutor, apply_workspace_inventory, failed_provider_auth_projection_ids,
    provider_auth_recovery_generations, provider_inventory_contract_matches,
    recoverable_provider_auth_failure, resolve_machine_workspace,
    resolve_scheduling_auth_generation, web_session_is_missing_machine,
};
use crate::core::{Hub, SessionOrigin, Status};
use std::collections::BTreeMap;

#[test]
fn provider_reload_reserves_lifecycle_until_resolution_finishes_or_is_cancelled() {
    let fences = super::PluginLifecycleFences::default();
    let key = ("machine".to_owned(), "codex".to_owned());
    let guard = super::ProviderReloadFence::acquire(&fences, key.clone()).unwrap();
    assert_eq!(
        fences.read().get(&key),
        Some(&super::PluginFenceState::Installing)
    );
    assert!(super::ProviderReloadFence::acquire(&fences, key.clone()).is_err());
    drop(guard);
    assert!(!fences.read().contains_key(&key));
    fences
        .write()
        .insert(key.clone(), super::PluginFenceState::Uninstalled);
    assert!(super::ProviderReloadFence::acquire(&fences, key.clone()).is_err());
    assert_eq!(
        fences.read().get(&key),
        Some(&super::PluginFenceState::Uninstalled)
    );
}

#[test]
fn provider_reload_accepts_versions_but_rejects_native_or_auth_contract_changes() {
    let source: cowboy_provider_sdk::StandardProviderSource =
        serde_json::from_str(include_str!("../../../../plugins/codex/provider.json")).unwrap();
    let previous = source.compile().unwrap();
    let mut next = previous.clone();
    next.version = "new-version".to_owned();
    assert!(super::provider_reload_contract_compatible(&previous, &next));
    next.authentication.projection_schema = "different-home".to_owned();
    assert!(!super::provider_reload_contract_compatible(
        &previous, &next
    ));
    next = previous.clone();
    next.compatibility.auth_contract_fingerprint = "different-auth".to_owned();
    assert!(!super::provider_reload_contract_compatible(
        &previous, &next
    ));
    // A later release may add a sign-in method and keep every earlier one.
    next = previous.clone();
    next.compatibility.auth_contract_fingerprint = "different-auth".to_owned();
    let mut added = next.authentication.methods[0].clone();
    added.id = "added-method".to_owned();
    next.authentication.methods.push(added);
    assert!(super::provider_reload_contract_compatible(&previous, &next));
    // Changing or removing a method the session may use is not additive.
    next.authentication.methods[0].label = "changed".to_owned();
    assert!(!super::provider_reload_contract_compatible(
        &previous, &next
    ));
    next.authentication.methods.remove(0);
    assert!(!super::provider_reload_contract_compatible(
        &previous, &next
    ));
    // Nor is any change outside the method list.
    next = previous.clone();
    next.compatibility.auth_contract_fingerprint = "different-auth".to_owned();
    next.authentication.required = !next.authentication.required;
    assert!(!super::provider_reload_contract_compatible(
        &previous, &next
    ));
    next = previous.clone();
    next.runtime.behavior.session = cowboy_provider_sdk::SessionBehavior::XaiSessionV1;
    assert!(!super::provider_reload_contract_compatible(
        &previous, &next
    ));
    next = previous.clone();
    next.id = "different-provider".to_owned();
    assert!(!super::provider_reload_contract_compatible(
        &previous, &next
    ));
}

fn installed_auth(generation: u64) -> crate::machine_protocol::PluginInventory {
    crate::machine_protocol::PluginInventory {
        plugin_id: "claude-code".to_owned(),
        plugin_version: "1.0.0".to_owned(),
        plugin_kind: cowboy_plugin_sdk::PluginKind::AgentProvider,
        generation_digest: format!("sha256:{}", "1".repeat(64)),
        installation_revision: None,
        contract_fingerprint: format!("sha256:{}", "2".repeat(64)),
        state: crate::machine_protocol::PluginInstallationState::Active,
        rollback_generation_digest: None,
        active_session_leases: 0,
        auth_generation: Some(generation),
        replica_state: crate::machine_protocol::ProviderReplicaState::Current,
        materialization_state: crate::machine_protocol::ProviderMaterializationState::Current,
        detail: None,
    }
}

#[test]
fn agent_inventory_reader_distinguishes_plugin_and_provider_contracts() {
    let mut installed = installed_auth(1);
    let inner = installed.contract_fingerprint.clone();
    let outer = format!("sha256:{}", "a".repeat(64));
    assert!(provider_inventory_contract_matches(
        &installed,
        &inner,
        Some(&outer)
    ));
    installed.contract_fingerprint = outer.clone();
    assert!(provider_inventory_contract_matches(
        &installed,
        &inner,
        Some(&outer)
    ));
    installed.installation_revision = Some(
        format!("installation-{}", "b".repeat(64))
            .try_into()
            .unwrap(),
    );
    assert!(provider_inventory_contract_matches(
        &installed,
        &inner,
        Some(&outer)
    ));
    assert!(!provider_inventory_contract_matches(
        &installed, &inner, None
    ));
    installed.contract_fingerprint = inner.clone();
    assert!(!provider_inventory_contract_matches(
        &installed,
        &inner,
        Some(&outer)
    ));
    assert!(provider_inventory_contract_matches(
        &installed, &inner, None
    ));
    installed.installation_revision = None;
    installed.contract_fingerprint = "untrusted".into();
    assert!(!provider_inventory_contract_matches(
        &installed,
        &inner,
        Some(&outer)
    ));
}

#[test]
fn authentication_candidate_is_bound_to_the_exact_executor_release() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let control = crate::machine_control::MachineControl::default();
    let executor = ProviderAuthExecutor {
        native_login: None,
        named_login: None,
        cancel_requested: false,
        owner_user_id: "owner".into(),
        machine_id: "hawk".to_owned(),
        account_id: Default::default(),
        connection: control.install(
            "hawk".into(),
            "fixture".into(),
            false,
            crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
            tx,
        ),
        provider_id: "claude-code".to_owned(),
        provider_version: "1.0.0".to_owned(),
        generation_digest: "sha256:release".to_owned(),
        auth_contract_fingerprint: "sha256:auth".to_owned(),
        auth_method: "api-key".to_owned(),
        expected_generation: 0,
        promotion_started: false,
        expires_at_ms: i64::MAX,
    };
    assert!(executor.accepts_candidate(
        "hawk",
        &Default::default(),
        "claude-code",
        "api-key",
        "1.0.0",
        "sha256:release",
        "sha256:auth",
    ));
    assert!(!executor.accepts_candidate(
        "hawk",
        &Default::default(),
        "claude-code",
        "api-key",
        "1.0.0",
        "sha256:other-release",
        "sha256:auth",
    ));
    let another_account = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    assert!(!executor.accepts_candidate(
        "hawk",
        &another_account,
        "claude-code",
        "api-key",
        "1.0.0",
        "sha256:release",
        "sha256:auth",
    ));
}

fn service_auth(
    generation: u64,
    state: crate::provider_service::ServiceAuthenticationState,
) -> crate::provider_service::ProviderAuthenticationStatus {
    crate::provider_service::ProviderAuthenticationStatus {
        provider_id: "claude-code".to_owned(),
        account_id: crate::provider_account::ProviderAccountId::default(),
        recipient_key: None,
        auth_generation: generation,
        authentication_state: state,
        distribution_state: crate::provider_service::ServiceDistributionState::Current,
        auth_contract_fingerprint: format!("sha256:{}", "3".repeat(64)),
        authentication_scope: "claude-code-auth-v1".to_owned(),
        portable_schema: "claude-code-auth-v1".to_owned(),
        projection_schema: "claude-code-home-v1".to_owned(),
        account_label: None,
        updated_at_ms: 1,
    }
}

#[test]
fn web_creation_cannot_use_the_legacy_shared_local_workspace() {
    assert!(web_session_is_missing_machine("local", &SessionOrigin::Web));
    assert!(!web_session_is_missing_machine(
        "local",
        &SessionOrigin::Api
    ));
    assert!(!web_session_is_missing_machine("hawk", &SessionOrigin::Web));
}

#[test]
fn remote_workspace_id_is_resolved_before_session_persistence() {
    let workspaces = [crate::machine_protocol::MachineWorkspace {
        id: "cowboy".to_owned(),
        display_name: "Cowboy".to_owned(),
        canonical_path: "/srv/cowboy".to_owned(),
    }];
    assert_eq!(
        resolve_machine_workspace(&workspaces, Some("cowboy"))
            .map(|workspace| { (workspace.id.as_str(), workspace.canonical_path.as_str()) }),
        Ok(("cowboy", "/srv/cowboy"))
    );
    assert!(resolve_machine_workspace(&workspaces, Some("/srv/cowboy")).is_err());
    assert!(resolve_machine_workspace(&workspaces, Some("unknown")).is_err());
}

#[test]
fn session_scheduling_requires_the_exact_ready_service_auth_generation() {
    let installed = installed_auth(7);
    let ready = service_auth(
        7,
        crate::provider_service::ServiceAuthenticationState::Ready,
    );
    assert_eq!(
        resolve_scheduling_auth_generation(&installed, true, Some(&ready)),
        Ok(Some(7))
    );

    let stale = service_auth(
        8,
        crate::provider_service::ServiceAuthenticationState::Ready,
    );
    assert!(
        resolve_scheduling_auth_generation(&installed, true, Some(&stale))
            .unwrap_err()
            .contains("generation is not synchronized")
    );

    let signed_out = service_auth(
        7,
        crate::provider_service::ServiceAuthenticationState::SignedOut,
    );
    assert!(
        resolve_scheduling_auth_generation(&installed, true, Some(&signed_out))
            .unwrap_err()
            .contains("not ready")
    );
    assert_eq!(
        resolve_scheduling_auth_generation(&installed, false, None),
        Ok(None)
    );
}

#[test]
fn native_scheduling_uses_its_own_exact_generation_and_identity() {
    use super::provider_accounts::native_generation::resolve;
    let source: cowboy_provider_sdk::StandardProviderSource =
        serde_json::from_str(include_str!("../../../../plugins/claude-code/provider.json")).unwrap();
    let manifest = source.compile().unwrap();
    let mut installed = installed_auth(99);
    installed.plugin_id = manifest.id.clone();
    installed.plugin_version = manifest.version.clone();
    let profile = crate::provider_account::NativeProfileReceipt {
        account_id: "11111111-1111-1111-1111-111111111111"
            .to_owned()
            .try_into()
            .unwrap(),
        provider_id: manifest.id.clone(),
        request_id: "fixture".into(),
        generation: 7,
        method: "claude-subscription".into(),
        provider_version: manifest.version.clone(),
        generation_digest: installed.generation_digest.clone(),
        auth_contract_fingerprint: manifest.compatibility.auth_contract_fingerprint.clone(),
        identity: crate::provider_account::NativeIdentity {
            authority: "claude-ai".into(),
            subject: "personal-account".into(),
            tenant: Some("personal-org".into()),
        },
    };
    assert_eq!(resolve(&profile, &installed, &manifest, None), Ok(7));
    installed.auth_generation = None;
    assert_eq!(resolve(&profile, &installed, &manifest, None), Ok(7));
    for field in 0..9 {
        let mut bad = profile.clone();
        match field {
            0 => bad.account_id = crate::provider_account::ProviderAccountId::default(),
            1 => bad.provider_id = "codex".into(),
            2 => bad.provider_version = "other".into(),
            3 => bad.generation_digest = "other".into(),
            4 => bad.auth_contract_fingerprint = "other".into(),
            5 => bad.generation = 0,
            6 => bad.method = "api-key".into(),
            7 => bad.identity.authority = "other".into(),
            _ => bad.identity.tenant = None,
        }
        assert!(
            resolve(&bad, &installed, &manifest, None).is_err(),
            "field {field}"
        );
    }
    // A later release keeps the login only with identical signed
    // authentication, verified against the Catalog's signed-in release.
    let mut next = manifest.clone();
    next.version = "999.0.0".into();
    let mut upgraded = installed.clone();
    upgraded.plugin_version = next.version.clone();
    upgraded.generation_digest = "sha256:next".into();
    assert_eq!(resolve(&profile, &upgraded, &next, Some(&manifest)), Ok(7));
    assert!(resolve(&profile, &upgraded, &next, None).is_err());
    let mut renamed = manifest.clone();
    renamed.version = "0.0.1".into();
    assert!(resolve(&profile, &upgraded, &next, Some(&renamed)).is_err());
    let mut changed = next.clone();
    changed.authentication.methods.pop();
    assert!(resolve(&profile, &upgraded, &changed, Some(&manifest)).is_err());
    installed.plugin_version = "upgraded".into();
    assert!(resolve(&profile, &installed, &manifest, None).is_err());
}

#[test]
fn only_failed_current_provider_projections_request_service_repair() {
    let mut failed = installed_auth(7);
    failed.materialization_state = crate::machine_protocol::ProviderMaterializationState::Failed;
    let mut refreshing = installed_auth(7);
    refreshing.plugin_id = "grok".to_owned();
    refreshing.materialization_state =
        crate::machine_protocol::ProviderMaterializationState::Applying;
    let mut absent = failed.clone();
    absent.plugin_id = "codex".to_owned();
    absent.replica_state = crate::machine_protocol::ProviderReplicaState::Absent;

    assert_eq!(
        failed_provider_auth_projection_ids(&[failed, refreshing, absent]),
        vec!["claude-code".to_owned()]
    );
}

#[test]
fn auth_refresh_recovery_requires_newer_ready_credentials_on_the_selected_machine() {
    let hub = Hub::new();
    hub.create_session(crate::core::SessionRegistration {
        id: "s".to_owned(),
        provider: "claude-code".to_owned(),
        provider_version: "1.0.0".to_owned(),
        provider_generation_digest: "sha256:provider".to_owned(),
        provider_account_id: crate::provider_account::ProviderAccountId::default(),
        provider_auth_generation: Some(7),
        provider_behavior: None,
        machine_id: "hawk".to_owned(),
        workspace_id: None,
        workspace_name: None,
        workspace_source_path: None,
        execution_binding: None,
        cwd: "/tmp".to_owned(),
        title: "test".to_owned(),
        origin: SessionOrigin::Web,
        system: false,
        owner_user_id: None,
        owner_username: None,
    });
    let session = hub.session_info("s").unwrap().meta;
    let ready = service_auth(
        8,
        crate::provider_service::ServiceAuthenticationState::Ready,
    );
    let synchronized = BTreeMap::from([("hawk".to_owned(), 8)]);

    hub.set_status("s", Status::Crashed, Some("unrelated failure".to_owned()));
    assert!(recoverable_provider_auth_failure(&hub, "s").is_none());
    hub.set_status(
        "s",
        Status::Crashed,
        Some(crate::provider::provider_auth_required_detail(
            "claude-code",
            true,
        )),
    );
    assert!(recoverable_provider_auth_failure(&hub, "s").is_some());
    hub.set_status("s", Status::Running, None);
    assert!(
        recoverable_provider_auth_failure(&hub, "s").is_some(),
        "an idle worker snapshot must not erase the unresolved auth failure"
    );
    hub.set_status("s", Status::Exited, None);
    assert!(
        recoverable_provider_auth_failure(&hub, "s").is_none(),
        "an explicitly exited session must remain stopped"
    );
    hub.set_status("s", Status::Running, None);
    hub.push(
        "s",
        crate::core::Event::TurnEnd {
            usage: None,
            stop_reason: "end_turn".to_owned(),
        },
    );
    assert!(
        recoverable_provider_auth_failure(&hub, "s").is_none(),
        "a clean turn completion resolves the prior auth failure"
    );

    assert_eq!(
        provider_auth_recovery_generations(
            &session,
            &ready,
            &synchronized,
            &ready.auth_contract_fingerprint,
        ),
        Some((7, 8))
    );
    let mut named_session = session.clone();
    named_session.provider_account_id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    let mut named_status = ready.clone();
    named_status.account_id = named_session.provider_account_id.clone();
    for (session, status) in [(&named_session, &ready), (&session, &named_status)] {
        assert_eq!(
            provider_auth_recovery_generations(
                session,
                status,
                &synchronized,
                &status.auth_contract_fingerprint,
            ),
            None,
            "a matching Provider and generation must not cross account identity"
        );
    }
    assert_eq!(
        provider_auth_recovery_generations(
            &session,
            &ready,
            &BTreeMap::new(),
            &ready.auth_contract_fingerprint,
        ),
        None
    );
    let stale_projection = BTreeMap::from([("hawk".to_owned(), 7)]);
    assert_eq!(
        provider_auth_recovery_generations(
            &session,
            &ready,
            &stale_projection,
            &ready.auth_contract_fingerprint,
        ),
        None
    );
    assert_eq!(
        provider_auth_recovery_generations(&session, &ready, &synchronized, "sha256:different",),
        None
    );
    let expired = service_auth(
        8,
        crate::provider_service::ServiceAuthenticationState::Expired,
    );
    assert_eq!(
        provider_auth_recovery_generations(
            &session,
            &expired,
            &synchronized,
            &expired.auth_contract_fingerprint,
        ),
        None
    );
}

#[test]
fn component_only_inventory_preserves_workspaces_until_an_explicit_reload() {
    let workspace = |id: &str| crate::machine_protocol::MachineWorkspace {
        id: id.to_owned(),
        display_name: id.to_owned(),
        canonical_path: format!("/srv/{id}"),
    };
    let identity = |id: &str| crate::machine_protocol::WorkspaceRootIdentity {
        workspace_id: id.to_owned(),
        incarnation: format!("incarnation-{id}"),
    };
    let mut workspaces = vec![workspace("old")];
    let mut identities = vec![identity("old")];
    let mut revision = Some("revision-old".to_owned());
    apply_workspace_inventory(
        &mut workspaces,
        &mut identities,
        &mut revision,
        None,
        None,
        None,
    );
    assert_eq!(workspaces, vec![workspace("old")]);
    assert_eq!(identities, vec![identity("old")]);
    assert_eq!(revision.as_deref(), Some("revision-old"));

    apply_workspace_inventory(
        &mut workspaces,
        &mut identities,
        &mut revision,
        Some(vec![workspace("new")]),
        Some(vec![identity("new")]),
        Some("revision-new".to_owned()),
    );
    assert_eq!(workspaces, vec![workspace("new")]);
    assert_eq!(identities, vec![identity("new")]);
    assert_eq!(revision.as_deref(), Some("revision-new"));

    // An older Machine's workspace observation drops stale identities
    // rather than letting them outlive the roots they described.
    apply_workspace_inventory(
        &mut workspaces,
        &mut identities,
        &mut revision,
        Some(vec![workspace("legacy")]),
        None,
        None,
    );
    assert_eq!(workspaces, vec![workspace("legacy")]);
    assert!(identities.is_empty());
    assert!(revision.is_none());
}
