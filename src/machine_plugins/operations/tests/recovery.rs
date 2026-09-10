use super::*;
use crate::machine_protocol::plugin_recovery::{
    InstallationEvidence, InstallationUnavailable, RecoveryBasis, RecoveryObservation,
    RecoveryUncertainty,
};

async fn installed() -> (
    tempfile::TempDir,
    MachinePluginStore,
    DesiredPlugin,
    UninstallStep,
) {
    let root = tempfile::tempdir().unwrap();
    let publisher =
        crate::machine_auth::MachineIdentity::load_or_create(&root.path().join("publisher"))
            .unwrap();
    let desired = crate::machine_plugins::tests::telemetry_release(&publisher, "1.0.0");
    let store = MachinePluginStore::new(
        &root.path().join("machine"),
        Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    store.install(&desired).await.unwrap();
    store.enable_installation_tracking().await.unwrap();
    let active = store.inventory().unwrap().remove(0);
    let mut request = step();
    request.schema = 2;
    request.installation_revision = active.installation_revision;
    request.generation_digest = active.generation_digest;
    request.contract_fingerprint = active.contract_fingerprint;
    (root, store, desired, request)
}

fn journal_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(journal_bytes(&path));
        } else {
            files.insert(path.clone(), fs::read(path).unwrap());
        }
    }
    files
}

async fn removed(store: &MachinePluginStore, step: &UninstallStep) {
    assert_eq!(
        receipt(
            store
                .uninstall_step(step, Some("service-a"), "machine-a", true, false)
                .await
                .result
        )
        .outcome,
        StepOutcome::Applied {}
    );
}

async fn inspect(store: &MachinePluginStore, step: &UninstallStep) -> RecoveryObservation {
    let before = journal_bytes(&store.operations.root);
    let observation = store
        .uninstall_recovery_observation(step, Some("service-a"), "machine-a")
        .await;
    assert!(observation.matches(step));
    assert_eq!(
        before,
        journal_bytes(&store.operations.root),
        "observation must not write or erase evidence"
    );
    observation
}

#[tokio::test]
async fn recovery_reads_exact_tombstone_after_reader_reopen_and_rejects_same_release_aba() {
    let (root, store, desired, request) = installed().await;
    removed(&store, &request).await;
    let observation = inspect(&store, &request).await;
    assert!(matches!(
        observation.basis(&request),
        RecoveryBasis::MatchingRemoval { .. }
    ));
    drop(store);
    let reader = MachinePluginStore::new(
        &root.path().join("machine"),
        Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    assert_eq!(inspect(&reader, &request).await, observation);
    assert!(
        reader.install(&desired).await.is_err(),
        "query must not enable the reader's writer"
    );
    reader.enable_installation_tracking().await.unwrap();
    let later = reader.install(&desired).await.unwrap();
    assert_ne!(later.installation_revision, request.installation_revision);
    let replaced = inspect(&reader, &request).await;
    assert_eq!(
        replaced.basis(&request),
        RecoveryBasis::InstallationChanged {}
    );
    let RecoveryObservation::Observed { snapshot } = replaced else {
        panic!()
    };
    assert_eq!(
        snapshot.receipt.unwrap().outcome,
        StepOutcome::Applied {},
        "the old receipt remains historical truth"
    );
    let mut later_request = request.clone();
    later_request.operation_id.push('2');
    later_request.installation_revision = later.installation_revision;
    removed(&reader, &later_request).await;
    assert_eq!(
        inspect(&reader, &request).await.basis(&request),
        RecoveryBasis::InstallationChanged {},
        "absence after a DIFFERENT uninstall is not the original tombstone"
    );
    assert!(matches!(
        inspect(&reader, &later_request).await.basis(&later_request),
        RecoveryBasis::MatchingRemoval { .. }
    ));
}

#[tokio::test]
async fn recovery_never_infers_forward_completion_from_a_committed_slot() {
    let (_root, store, _desired, request) = installed().await;
    assert_eq!(
        inspect(&store, &request).await.basis(&request),
        RecoveryBasis::Unknown {
            reason: RecoveryUncertainty::MissingForwardReceipt
        }
    );
    removed(&store, &request).await;
    // Crash window: the slot tombstone reached disk but the final parent
    // receipt did not. Keep both facts without upgrading Unknown to Applied.
    let key = request.key().unwrap();
    {
        let mut state = store.operations.state.lock();
        let receipt = state.receipts.get_mut(&key).unwrap();
        receipt.outcome = StepOutcome::Unknown {
            reason: StepUncertainty::Interrupted,
        };
        store.operations.persist(&key, receipt).unwrap();
    }
    assert_eq!(
        inspect(&store, &request).await.basis(&request),
        RecoveryBasis::Unknown {
            reason: RecoveryUncertainty::ForwardOutcome
        }
    );
    assert!(
        store
            .operations
            .ensure_unfenced(&request.plugin_id)
            .is_err()
    );
}

#[tokio::test]
async fn recovery_observes_pending_slots_and_other_unknown_steps_without_clearing_them() {
    let (root, store, desired, request) = installed().await;
    removed(&store, &request).await;
    let mut other = request.clone();
    other.operation_id.push('2');
    store
        .operations
        .execute(&other, || true, || bail!("fixture crash"));
    assert_eq!(
        inspect(&store, &request).await.basis(&request),
        RecoveryBasis::SlotFenced {}
    );
    assert!(store.install(&desired).await.is_err());
    drop(store);
    let reader = MachinePluginStore::new(
        &root.path().join("machine"),
        Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    assert_eq!(
        inspect(&reader, &request).await.basis(&request),
        RecoveryBasis::SlotFenced {}
    );

    let (root, store, desired, request) = installed().await;
    removed(&store, &request).await;
    store.install(&desired).await.unwrap();
    let pending = store
        .operations
        .installations
        .begin(
            &request.plugin_id,
            Some(&request.generation_digest),
            Some(&request.generation_digest),
            installations::Effect::Install,
            None,
        )
        .unwrap();
    drop(pending); // A durable Unknown does not carry completion authority.
    assert_eq!(
        inspect(&store, &request).await.basis(&request),
        RecoveryBasis::Unknown {
            reason: RecoveryUncertainty::InstallationPending
        }
    );
    drop(store);
    let reader = MachinePluginStore::new(
        &root.path().join("machine"),
        Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    assert_eq!(
        inspect(&reader, &request).await.basis(&request),
        RecoveryBasis::Unknown {
            reason: RecoveryUncertainty::InstallationPending
        }
    );
}

#[tokio::test]
async fn recovery_does_not_treat_invalid_or_changed_active_paths_as_removed() {
    let (_root, store, _desired, request) = installed().await;
    removed(&store, &request).await;
    let active = store.plugin_root(&request.plugin_id).join("active");
    fs::write(&active, b"not a symlink").unwrap();
    let mismatched = |value: RecoveryObservation| {
        assert_eq!(
            value.basis(&request),
            RecoveryBasis::Unknown {
                reason: RecoveryUncertainty::InstallationUnavailable
            }
        );
        let RecoveryObservation::Observed { snapshot } = value else {
            panic!()
        };
        assert_eq!(
            snapshot.installation,
            InstallationEvidence::Unavailable {
                reason: InstallationUnavailable::ActiveLinkMismatch
            }
        );
    };
    mismatched(inspect(&store, &request).await);
    fs::remove_file(&active).unwrap();
    for target in [
        "/outside/generation".to_owned(),
        "generations/../../outside".to_owned(),
        format!("generations/{}", "f".repeat(64)), // dangling
        format!("generations/{}", &request.generation_digest[7..]), // real, but contradicts tombstone
    ] {
        std::os::unix::fs::symlink(target, &active).unwrap();
        mismatched(inspect(&store, &request).await);
        fs::remove_file(&active).unwrap();
    }
    assert!(matches!(
        inspect(&store, &request).await.basis(&request),
        RecoveryBasis::MatchingRemoval { .. }
    ));
}

#[tokio::test]
async fn recovery_rejects_wrong_owner_conflicting_identity_and_poisoned_storage() {
    let (_root, store, _desired, request) = installed().await;
    removed(&store, &request).await;
    for (service, machine) in [
        (None, "machine-a"),
        (Some("other-service"), "machine-a"),
        (Some("service-a"), "other-machine"),
    ] {
        assert_eq!(
            store
                .uninstall_recovery_observation(&request, service, machine)
                .await,
            RecoveryObservation::Unavailable {
                reason: StepUnavailable::WrongOwner
            }
        );
    }
    let mut changed = request.clone();
    changed.plan_digest = digest(b"other approved actor");
    assert_eq!(
        inspect(&store, &changed).await,
        RecoveryObservation::Unavailable {
            reason: StepUnavailable::IdentityConflict
        }
    );
    changed.plugin_id = "../../private".into();
    assert_eq!(
        store
            .uninstall_recovery_observation(&changed, Some("service-a"), "machine-a")
            .await,
        RecoveryObservation::Unavailable {
            reason: StepUnavailable::InvalidRequest
        }
    );
    store.operations.state.lock().poisoned = true;
    assert_eq!(
        inspect(&store, &request).await,
        RecoveryObservation::Unavailable {
            reason: StepUnavailable::Storage
        }
    );
}

#[tokio::test]
async fn recovery_is_serialized_with_lifecycle_and_does_not_adopt_untracked_state() {
    let root = tempfile::tempdir().unwrap();
    let store = MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).unwrap();
    let request = step();
    let guard = store.lifecycle.lock().await;
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            inspect(&store, &request)
        )
        .await
        .is_err()
    );
    drop(guard);
    let RecoveryObservation::Observed { snapshot } = inspect(&store, &request).await else {
        panic!()
    };
    assert_eq!(snapshot.installation, InstallationEvidence::Untracked {});
    assert!(snapshot.receipt.is_none());
    assert!(
        !root
            .path()
            .join("plugin-operations/installations-v1")
            .exists()
    );
}
