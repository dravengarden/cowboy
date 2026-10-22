use super::*;
use crate::machine_protocol::plugin_install::fixture;

fn desired(root: &Path) -> DesiredPlugin {
    let publisher = crate::machine_auth::MachineIdentity::load_or_create(root).unwrap();
    crate::machine_plugins::tests::telemetry_release(&publisher, "1.0.0")
}

fn bound(desired: &DesiredPlugin) -> InstallStep {
    InstallStep {
        plugin_id: desired.release.plugin_id.clone(),
        plugin_kind: desired.release.plugin_kind,
        plugin_version: desired.release.plugin_version.clone(),
        generation_digest: desired.release.artifact_digest.clone(),
        contract_fingerprint: desired.release.contract_fingerprint.clone(),
        envelope_digest: crate::machine_protocol::plugin_step::digest(
            &serde_json::to_vec(desired).unwrap(),
        ),
        ..fixture()
    }
}

fn found(
    observation: InstallObservation,
) -> crate::machine_protocol::plugin_install::InstallReceipt {
    let InstallLookup::Found { receipt } = observation.result else {
        panic!("{observation:?}");
    };
    *receipt
}

#[tokio::test]
async fn sealed_continuations_recheck_exact_lease_and_envelope_before_any_effect() {
    let root = tempfile::tempdir().unwrap();
    let store = MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).unwrap();
    store.enable_installation_tracking().await.unwrap();
    let desired = desired(&root.path().join("publisher"));
    let step = bound(&desired);
    let scope = PluginExecutionScope::new(Some(&step.service_id), &step.machine_id);
    let pending = store.operations.install_attempts.begin(&step).unwrap();
    let mut changed = step.clone();
    changed.operation_id = "mismatched-continuation-fixture".into();
    let wrong_lease = scope.installation(&changed).unwrap();
    let mut guard = InstallGuard::Durable(InstallAdmission {
        journal: &store.operations.install_attempts,
        pending: Box::new(pending),
        lease: &wrong_lease,
    });
    assert!(store.install_inner(&desired, &mut guard).await.is_err());
    let correct_lease = scope.installation(&step).unwrap();
    let InstallGuard::Durable(ref mut admission) = guard else {
        panic!();
    };
    admission.lease = &correct_lease;
    let mut changed = desired.clone();
    changed.package_base64.push('A');
    assert!(store.install_inner(&changed, &mut guard).await.is_err());
    assert!(
        store
            .install_inner(&desired, &mut InstallGuard::Legacy)
            .await
            .is_err()
    );
    assert!(!store.plugin_root(&step.plugin_id).exists());
    assert!(
        matches!(store.operations.install_attempts.query(&step), InstallLookup::Found { receipt } if receipt.outcome == (InstallOutcome::Pending { phase: InstallPhase::Prepared }))
    );
}

/// Extend the signed Agent and code lifecycle fixtures after their original
/// uninstall. Retained artifacts avoid extra servers and use real activation,
/// auth materialization (Agent) and installation tombstone CAS.
pub(in crate::machine_plugins) async fn assert_retained_lifecycle(
    store: &MachinePluginStore,
    desired: &DesiredPlugin,
) {
    let mut step = bound(desired);
    step.expected = store.install_target(&step.plugin_id).unwrap();
    assert!(matches!(step.expected, InstallTarget::Removed { .. }));
    let phases = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let recorded = phases.clone();
    store
        .operations
        .install_attempts
        .before_write_for_test(Box::new(move |receipt| {
            recorded.lock().push(receipt.outcome.clone());
            Ok(())
        }));
    let scope = PluginExecutionScope::new(Some(&step.service_id), &step.machine_id);
    let receipt = found(
        store
            .install_step(&step, desired, &scope.installation(&step).unwrap(), true)
            .await,
    );
    assert!(matches!(receipt.outcome, InstallOutcome::Applied { .. }));
    let phases = phases.lock().clone();
    assert_eq!(
        phases[0],
        InstallOutcome::Pending {
            phase: InstallPhase::Prepared
        }
    );
    assert_eq!(
        phases[1],
        InstallOutcome::Pending {
            phase: InstallPhase::Staging
        }
    );
    assert_eq!(
        phases[2],
        InstallOutcome::Pending {
            phase: InstallPhase::Activating
        }
    );
    if step.plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider {
        assert_eq!(
            phases[3],
            InstallOutcome::Pending {
                phase: InstallPhase::ProjectingAuthentication
            }
        );
    }
    let current = store.inventory_one(&step.plugin_id).unwrap().unwrap();
    let mut uninstall = crate::machine_protocol::plugin_step::fixture();
    uninstall.schema = 2;
    uninstall.operation_id = "uninstall-after-install-fixture-0001".into();
    uninstall.plugin_id = step.plugin_id.clone();
    uninstall.plugin_version = step.plugin_version.clone();
    uninstall.generation_digest = step.generation_digest.clone();
    uninstall.contract_fingerprint = step.contract_fingerprint.clone();
    uninstall.installation_revision = current.installation_revision;
    uninstall.expires_at_ms = step.expires_at_ms;
    let lease = scope.uninstall(&uninstall).unwrap();
    let removed = store
        .uninstall_step(
            &uninstall,
            Some(&step.service_id),
            &step.machine_id,
            true,
            UninstallAccess::Execute(&lease),
        )
        .await;
    assert!(
        matches!(removed.result, crate::machine_protocol::plugin_step::StepLookup::Found { receipt } if receipt.outcome == crate::machine_protocol::plugin_step::StepOutcome::Applied {})
    );
    assert!(store.inventory().unwrap().is_empty());
    assert_eq!(
        found(
            store
                .install_step(&step, desired, &scope.installation(&step).unwrap(), true)
                .await
        ),
        receipt
    );
    assert!(store.inventory().unwrap().is_empty()); // Duplicate must not reinstall after removal.
    assert_last_lease_boundary(store, desired).await;
}

async fn assert_last_lease_boundary(store: &MachinePluginStore, desired: &DesiredPlugin) {
    let mut step = bound(desired);
    step.operation_id = "last-install-boundary-fixture-0001".into();
    step.expected = store.install_target(&step.plugin_id).unwrap();
    let phase = if step.plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider {
        InstallPhase::ProjectingAuthentication
    } else {
        InstallPhase::Activating
    };
    let scope = PluginExecutionScope::new(Some(&step.service_id), &step.machine_id);
    let lease = scope.installation(&step).unwrap();
    let scope = parking_lot::Mutex::new(Some(scope));
    store
        .operations
        .install_attempts
        .before_write_for_test(Box::new(move |receipt| {
            if receipt.outcome == (InstallOutcome::Pending { phase }) {
                scope.lock().take();
            }
            Ok(())
        }));
    let result = found(store.install_step(&step, desired, &lease, true).await);
    assert_eq!(
        result.outcome,
        InstallOutcome::Unknown {
            phase,
            reason: InstallUncertainty::AuthorizationEnded
        }
    );
    assert!(store.operations.ensure_unfenced(&step.plugin_id).is_err());
    if phase == InstallPhase::ProjectingAuthentication {
        // Activation happened, but the old approval may not start credential
        // projection or compensate it. Keep the installation incarnation pending.
        let active = store.recovery_active_digest(&step.plugin_id).unwrap();
        assert_eq!(active.as_deref(), Some(step.generation_digest.as_str()));
        assert!(
            store
                .operations
                .installations
                .revision(&step.plugin_id, &step.generation_digest)
                .is_err()
        );
        assert!(
            !store
                .auth_provider_root(&step.plugin_id)
                .join("materialized/current")
                .exists()
        );
    } else {
        assert!(
            store
                .recovery_active_digest(&step.plugin_id)
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn write_failure_at_each_phase_blocks_the_next_real_effect_and_keeps_evidence() {
    for boundary in ["prepared", "staging", "activating", "applied"] {
        let root = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).unwrap();
        store.enable_installation_tracking().await.unwrap();
        let desired = desired(&root.path().join("publisher"));
        let step = bound(&desired);
        store
            .operations
            .install_attempts
            .before_write_for_test(Box::new(move |receipt| {
                ensure!(
                    !matches!(
                        (boundary, &receipt.outcome),
                        (
                            "prepared",
                            InstallOutcome::Pending {
                                phase: InstallPhase::Prepared
                            }
                        ) | (
                            "staging",
                            InstallOutcome::Pending {
                                phase: InstallPhase::Staging
                            }
                        ) | (
                            "activating",
                            InstallOutcome::Pending {
                                phase: InstallPhase::Activating
                            }
                        ) | ("applied", InstallOutcome::Applied { .. })
                    ),
                    "fixture durability failure"
                );
                Ok(())
            }));
        let scope = PluginExecutionScope::new(Some(&step.service_id), &step.machine_id);
        let result = store
            .install_step(&step, &desired, &scope.installation(&step).unwrap(), true)
            .await;
        assert_eq!(
            result.result,
            unavailable(InstallUnavailable::Storage),
            "{boundary}"
        );
        let active = store.recovery_active_digest(&step.plugin_id).unwrap();
        assert_eq!(active.is_some(), boundary == "applied", "{boundary}");
        assert_eq!(
            store.plugin_root(&step.plugin_id).exists(),
            matches!(boundary, "activating" | "applied"),
            "{boundary}"
        );
        assert!(store.operations.ensure_unfenced(&step.plugin_id).is_err());
        drop(store);
        let reader =
            MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).unwrap();
        if boundary != "prepared" {
            assert!(reader.operations.ensure_unfenced(&step.plugin_id).is_err());
            assert!(
                matches!(reader.operations.install_attempts.query(&step), InstallLookup::Found { receipt } if matches!(receipt.outcome, InstallOutcome::Pending { .. }))
            );
        }
    }
}

#[tokio::test]
async fn connection_loss_during_a_staging_flush_cannot_activate_or_renew() {
    let root = tempfile::tempdir().unwrap();
    let store = MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).unwrap();
    store.enable_installation_tracking().await.unwrap();
    let desired = desired(&root.path().join("publisher"));
    let step = bound(&desired);
    let scope = PluginExecutionScope::new(Some(&step.service_id), &step.machine_id);
    let lease = scope.installation(&step).unwrap();
    let retained_scope = parking_lot::Mutex::new(Some(scope));
    store
        .operations
        .install_attempts
        .before_write_for_test(Box::new(move |receipt| {
            if receipt.outcome
                == (InstallOutcome::Pending {
                    phase: InstallPhase::Activating,
                })
            {
                retained_scope.lock().take();
            }
            Ok(())
        }));
    let receipt = found(store.install_step(&step, &desired, &lease, true).await);
    assert_eq!(
        receipt.outcome,
        InstallOutcome::Unknown {
            phase: InstallPhase::Activating,
            reason: InstallUncertainty::AuthorizationEnded
        }
    );
    assert!(
        store
            .recovery_active_digest(&step.plugin_id)
            .unwrap()
            .is_none()
    );
    let new_scope = PluginExecutionScope::new(Some(&step.service_id), &step.machine_id);
    assert_eq!(
        found(
            store
                .install_step(
                    &step,
                    &desired,
                    &new_scope.installation(&step).unwrap(),
                    true
                )
                .await
        ),
        receipt
    );
    assert!(
        store
            .recovery_active_digest(&step.plugin_id)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn missing_namespace_or_slot_cannot_be_adopted_from_current_links() {
    for namespace in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).unwrap();
        store.enable_installation_tracking().await.unwrap();
        let desired = desired(&root.path().join("publisher"));
        let step = bound(&desired);
        let scope = PluginExecutionScope::new(Some(&step.service_id), &step.machine_id);
        assert!(matches!(
            found(
                store
                    .install_step(&step, &desired, &scope.installation(&step).unwrap(), true)
                    .await
            )
            .outcome,
            InstallOutcome::Applied { .. }
        ));
        drop(store);
        let slots = root.path().join("plugin-operations/installations-v1");
        let from = if namespace {
            slots
        } else {
            slots.join("victoria.json")
        };
        fs::rename(from, root.path().join("retained-authority")).unwrap();
        assert!(MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).is_err());
        assert!(root.path().join("plugins/victoria/active").exists());
    }
}

#[tokio::test]
async fn signed_install_reinstall_duplicate_and_reader_reopen_keep_exact_incarnations() {
    let root = tempfile::tempdir().unwrap();
    let store = MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).unwrap();
    store.enable_installation_tracking().await.unwrap();
    let desired = desired(&root.path().join("publisher"));
    let step = bound(&desired);
    let scope = PluginExecutionScope::new(Some(&step.service_id), &step.machine_id);
    let first = found(
        store
            .install_step(&step, &desired, &scope.installation(&step).unwrap(), true)
            .await,
    );
    let InstallOutcome::Applied {
        revision: first_revision,
    } = &first.outcome
    else {
        panic!("{first:?}");
    };
    assert!(store.install(&desired).await.is_err());
    let mut second_step = step.clone();
    second_step.operation_id = "installation-fixture-0002".into();
    second_step.expected = InstallTarget::Installed {
        revision: first_revision.clone(),
        generation_digest: desired.release.artifact_digest.clone(),
    };
    let second = found(
        store
            .install_step(
                &second_step,
                &desired,
                &scope.installation(&second_step).unwrap(),
                true,
            )
            .await,
    );
    assert_ne!(first.outcome, second.outcome);
    assert_eq!(
        found(
            store
                .install_step(&step, &desired, &scope.installation(&step).unwrap(), true)
                .await
        ),
        first
    );
    assert_eq!(
        store.install_target("victoria").unwrap().revision(),
        match &second.outcome {
            InstallOutcome::Applied { revision } => Some(revision),
            _ => panic!(),
        }
    );
    drop(store);
    let reader = MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).unwrap();
    let observed = reader
        .query_installation_step(&step, Some(&step.service_id), &step.machine_id, false)
        .await;
    assert!(!observed.admission_enabled);
    assert_eq!(found(observed), first);
    assert!(reader.install(&desired).await.is_err());
}

#[tokio::test]
async fn ended_authority_reader_only_and_changed_envelopes_never_create_attempts() {
    for case in ["disconnect", "expire", "reader", "envelope", "wrong-target"] {
        let root = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).unwrap();
        store.enable_installation_tracking().await.unwrap();
        let mut desired = desired(&root.path().join("publisher"));
        let mut step = bound(&desired);
        if case == "wrong-target" {
            step.expected = InstallTarget::Removed {
                revision:
                    crate::machine_protocol::installation_revision::InstallationRevision::fresh()
                        .unwrap(),
            };
        }
        let scope = PluginExecutionScope::new(Some(&step.service_id), &step.machine_id);
        let lease = scope.installation(&step).unwrap();
        if case == "expire" {
            lease.expire_for_test();
        }
        if case == "envelope" {
            desired.package_base64.push('A');
        }
        if case == "disconnect" {
            drop(scope);
        }
        let observation = store
            .install_step(&step, &desired, &lease, case != "reader")
            .await;
        assert!(
            matches!(observation.result, InstallLookup::Unavailable { .. }),
            "{case}: {observation:?}"
        );
        assert!(
            !root
                .path()
                .join("plugin-operations/install-attempts-v1")
                .exists(),
            "{case}"
        );
        assert!(store.inventory().unwrap().is_empty());
    }
}
