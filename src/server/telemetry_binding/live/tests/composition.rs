//! Resolution must retain an observation lease, not repeatedly reconstruct it
//! from a currently matching tuple. All identities and packages are disposable.
use super::*;
use crate::composition::telemetry::ResolvedBinding;
use crate::machine_control::CommandFailure;
use crate::machine_protocol::PluginInstallationState;

pub(super) fn disruptions(installed: &PluginInventory) -> Vec<Vec<PluginInventory>> {
    let mut result = vec![vec![], vec![installed.clone(), installed.clone()]];
    for field in [
        "state",
        "kind",
        "version",
        "digest",
        "contract",
        "incarnation",
        "auth",
    ] {
        let mut plugin = installed.clone();
        match field {
            "state" => plugin.state = PluginInstallationState::Installing,
            "kind" => plugin.plugin_kind = cowboy_plugin_sdk::PluginKind::AgentProvider,
            "version" => plugin.plugin_version = "7.0.0".into(),
            "digest" => plugin.generation_digest = format!("sha256:{}", "a".repeat(64)),
            "contract" => plugin.contract_fingerprint = format!("sha256:{}", "b".repeat(64)),
            "incarnation" => plugin.installation_revision = None,
            "auth" => plugin.auth_generation = Some(1),
            _ => unreachable!(),
        }
        result.push(vec![plugin]);
    }
    result
}

#[tokio::test]
async fn resolved_binding_does_not_revive_after_observed_inventory_aba() {
    let f = Fixture::new(true, false).await;
    let intent = f.select();
    for changed in disruptions(&f.installed) {
        let effects = f.bind(&intent).unwrap();
        f.publish(changed);
        f.publish(vec![f.installed.clone()]);
        // No current() between observations: even a matching tuple cannot
        // erase the connection owner's observation of the discontinuity.
        assert!(!effects.current());
        assert!(
            effects
                .dispatch(&intent.machine_step().unwrap())
                .await
                .is_err()
        );
    }
    assert_eq!(f.sends.load(Ordering::Relaxed), 0);
    assert_eq!(f.queries.load(Ordering::Relaxed), 0);
    assert!(
        f.bind(&intent).unwrap().current(),
        "fresh resolution remains possible"
    );
}

#[tokio::test]
async fn resolved_binding_preserves_independent_slots_and_observation_metadata() {
    let f = Fixture::new(true, false).await;
    let intent = f.select();
    let effects = f.bind(&intent).unwrap();
    let mut other = f.installed.clone();
    other.plugin_id = "independent-plugin".into();
    let mut same = f.installed.clone();
    same.active_session_leases = 23;
    same.detail = Some("fixture observation".into());
    same.rollback_generation_digest = Some(format!("sha256:{}", "c".repeat(64)));
    same.replica_state = crate::machine_protocol::ProviderReplicaState::Pending;
    same.materialization_state = crate::machine_protocol::ProviderMaterializationState::Applying;
    f.publish(vec![other.clone(), same.clone()]);
    assert!(effects.current());
    other.state = PluginInstallationState::Uninstalling;
    f.publish(vec![same, other]);
    assert!(effects.current());
    f.publish(vec![f.installed.clone()]);
    assert!(effects.current());
    let observation = effects
        .dispatch(&intent.machine_step().unwrap())
        .await
        .unwrap();
    assert!(
        matches!(observation, BindingObservation::Observed { snapshot }
        if matches!(snapshot.receipt.as_ref().unwrap().outcome, BindingOutcome::Applied { .. }))
    );
    assert_eq!(f.sends.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn resolved_binding_rejects_matching_inventory_with_unverified_release_claims() {
    let f = Fixture::new(true, false).await;
    for field in ["id", "version", "digest", "fingerprint"] {
        let mut claimed = f.installed.clone();
        match field {
            "id" => claimed.plugin_id = "unknown-release".into(),
            "version" => claimed.plugin_version = "77.0.0".into(),
            "digest" => claimed.generation_digest = format!("sha256:{}", "d".repeat(64)),
            "fingerprint" => claimed.contract_fingerprint = format!("sha256:{}", "e".repeat(64)),
            _ => unreachable!(),
        }
        f.publish(vec![claimed.clone()]);
        assert!(f.bind(&Fixture::selection(&claimed)).is_err(), "{field}");
    }
    f.publish(vec![f.installed.clone(), f.installed.clone()]);
    assert!(
        f.bind(&f.select()).is_err(),
        "matching duplicate is not unique"
    );
    assert_eq!(f.sends.load(Ordering::Relaxed), 0);
    assert_eq!(f.queries.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn resolved_binding_retains_catalog_acceptance_rules_without_reviving_a_failed_resolution() {
    let f = Fixture::new(true, false).await;
    let effects = f.bind(&f.select()).unwrap();
    let storage = crate::plugin_storage::PluginStorage::sqlite_files(
        crate::plugin_dir::PluginDir::open(f.root.path()).unwrap(),
    );
    let marker = f.root.path().join("catalog/victoria.release.json");
    let release = fs::read(&marker).unwrap();
    fs::write(&marker, b"invalid fixture candidate").unwrap();
    assert!(f.catalog.refresh_with_runtime(&storage).await.is_err());
    assert!(
        effects.current(),
        "failed refresh retains the accepted Catalog"
    );
    fs::remove_file(&marker).unwrap();
    assert_eq!(f.catalog.refresh_with_runtime(&storage).await.unwrap(), 0);
    assert!(!effects.current());
    assert!(f.bind(&f.select()).is_err());
    fs::write(&marker, release).unwrap();
    assert_eq!(f.catalog.refresh_with_runtime(&storage).await.unwrap(), 1);
    assert!(!effects.current(), "a failed resolution is terminal");
    assert!(f.bind(&f.select()).unwrap().current());
    assert_eq!(f.sends.load(Ordering::Relaxed), 0);
}

pub(super) async fn remove_and_restore_release(f: &Fixture) {
    let storage = crate::plugin_storage::PluginStorage::sqlite_files(
        crate::plugin_dir::PluginDir::open(f.root.path()).unwrap(),
    );
    let marker = f.root.path().join("catalog/victoria.release.json");
    let release = fs::read(&marker).unwrap();
    fs::remove_file(&marker).unwrap();
    assert_eq!(f.catalog.refresh_with_runtime(&storage).await.unwrap(), 0);
    fs::write(&marker, release).unwrap();
    assert_eq!(f.catalog.refresh_with_runtime(&storage).await.unwrap(), 1);
}

#[tokio::test]
async fn resolved_binding_does_not_revive_after_catalog_aba_between_checks() {
    let f = Fixture::new(true, false).await;
    let intent = f.select();
    let effects = f.bind(&intent).unwrap();
    remove_and_restore_release(&f).await;
    // No current() call while the release was absent. The Catalog owner must
    // remember the accepted discontinuity, even when the bytes are identical.
    assert!(!effects.current());
    assert!(!effects.authorized(&intent).await);
    assert!(
        effects
            .dispatch(&intent.machine_step().unwrap())
            .await
            .is_err()
    );
    assert_eq!(f.sends.load(Ordering::Relaxed), 0);
    assert_eq!(f.queries.load(Ordering::Relaxed), 0);
    assert!(f.bind(&intent).unwrap().current());
}

#[tokio::test]
async fn resolved_binding_fences_the_last_enqueue_check_without_losing_read_only_observation() {
    let f = Fixture::new(true, false).await;
    let step = f.select().machine_step().unwrap();
    let resolved = ResolvedBinding::resolve(&f.catalog, &f.control, step.clone()).unwrap();
    let target = step.after().unwrap().selection.unwrap();
    let lease = f
        .control
        .lease_telemetry_installation(&f.connection, &target)
        .unwrap();
    assert!(resolved.current(&f.catalog, &f.control));
    f.publish(vec![]);
    f.publish(vec![f.installed.clone()]);
    // Exercise enqueue itself, not merely the resolver's earlier current().
    assert_eq!(
        f.control
            .commit_telemetry_binding(&f.connection, &step, Some(&lease))
            .await
            .unwrap_err()
            .certainty,
        CommandFailure::NotSent
    );
    assert_eq!(
        resolved
            .dispatch(&f.catalog, &f.control)
            .await
            .unwrap_err()
            .certainty,
        CommandFailure::NotSent
    );
    let observed = f
        .control
        .telemetry_binding_observation(resolved.connection(), resolved.step())
        .await
        .unwrap();
    assert!(observed.matches(&step));
    assert_eq!(f.sends.load(Ordering::Relaxed), 0);
    assert_eq!(f.queries.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn resolved_binding_owns_the_exact_input_and_never_mints_writer_authority() {
    let f = Fixture::new(true, false).await;
    let intent = f.select();
    for field in ["operation", "service", "machine", "expiry", "purpose"] {
        let effects = f.bind(&intent).unwrap();
        let mut changed = intent.clone();
        match field {
            "operation" => changed.operation_id = "different-operation".into(),
            "service" => changed.service_id = "another-service".into(),
            "machine" => changed.machine_id = "another-machine".into(),
            "expiry" => changed.expires_at_ms += 1,
            "purpose" => {
                changed.change = BindingChange::Revoke {
                    policy_epoch: "1".to_owned().try_into().unwrap(),
                }
            }
            _ => unreachable!(),
        }
        assert!(!effects.authorized(&changed).await, "{field}");
        assert!(!effects.authorized(&intent).await);
        assert!(
            effects
                .dispatch(&changed.machine_step().unwrap())
                .await
                .is_err()
        );
    }
    let effects = LiveEffects::bind(
        f.control.clone(),
        f.catalog.clone(),
        f.fences.clone(),
        &intent,
    )
    .unwrap();
    assert!(
        effects.current(),
        "resolution itself is not write admission"
    );
    assert!(!effects.authorized(&intent).await);
    assert_eq!(f.sends.load(Ordering::Relaxed), 0);
}
