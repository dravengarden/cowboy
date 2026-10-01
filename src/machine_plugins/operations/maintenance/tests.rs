use super::*;

fn fixture(root: &Path) -> UninstallStep {
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    for path in [
        "plugins",
        "plugins/victoria",
        "provider-auth",
        "provider-auth/providers",
        "provider-auth/providers/victoria",
        "provider-auth/providers/victoria/runtime",
    ] {
        fs::DirBuilder::new()
            .mode(0o700)
            .create(root.join(path))
            .unwrap();
    }
    let runtime = root.join("provider-auth/providers/victoria/runtime");
    // install_agent_plugin uses create_dir_all here under a normal 0022 umask.
    fs::set_permissions(
        root.join("plugins/victoria"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::write(runtime.join("cached"), b"fixture").unwrap();
    fs::set_permissions(runtime, fs::Permissions::from_mode(0o555)).unwrap();
    let journal = Journal::open(root).unwrap();
    let mut step = super::super::tests::step();
    journal
        .installations
        .enable(&[(step.plugin_id.clone(), step.generation_digest.clone())])
        .unwrap();
    step.schema = 2;
    step.installation_revision = journal
        .installations
        .revision(&step.plugin_id, &step.generation_digest)
        .unwrap();
    let scope = lease::PluginExecutionScope::new(Some(&step.service_id), &step.machine_id);
    let lease = scope.uninstall(&step).unwrap();
    let result = journal.execute(
        &step,
        &lease,
        || true,
        || {
            journal.installations.begin(
                &step.plugin_id,
                Some(&step.generation_digest),
                None,
                installations::Effect::Uninstall,
                Some(step.request_digest()?),
            )?;
            bail!("fixture cleanup failure")
        },
    );
    assert!(
        matches!(result, StepLookup::Found { receipt } if matches!(receipt.outcome, StepOutcome::Unknown { .. }))
    );
    step
}

fn call(root: &Path, step: &UninstallStep, apply: bool) -> Result<serde_json::Value> {
    complete_absent_uninstall(
        root,
        &step.service_id,
        &step.machine_id,
        &step.operation_id,
        apply.then(|| step.request_digest().unwrap()).as_deref(),
    )
}

#[test]
fn explicit_offline_recovery_preserves_failure_and_finishes_both_journals() {
    let root = tempfile::tempdir().unwrap();
    let step = fixture(root.path());
    assert_eq!(call(root.path(), &step, false).unwrap()["applied"], false);
    assert!(!root.path().join("plugin-maintenance").exists());
    let result = call(root.path(), &step, true).unwrap();
    assert_eq!(result["applied"], true);
    assert!(
        !root
            .path()
            .join("provider-auth/providers/victoria/runtime")
            .exists()
    );
    let audit: serde_json::Value = serde_json::from_slice(
        &fs::read(
            root.path()
                .join("plugin-maintenance")
                .join(format!("{}.json", result["audit_id"].as_str().unwrap())),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(audit["original_receipt"]["outcome"]["state"], "unknown");
    assert_eq!(audit["completed"], true);
    let reopened = Journal::open(root.path()).unwrap();
    assert!(
        matches!(reopened.query(&step), StepLookup::Found { receipt } if receipt.outcome == StepOutcome::Applied {})
    );
    reopened.ensure_unfenced(&step.plugin_id).unwrap();
}

#[test]
fn live_owner_wrong_digest_and_reactivated_slot_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let step = fixture(root.path());
    let owner = Journal::open(root.path()).unwrap();
    assert!(call(root.path(), &step, true).is_err());
    drop(owner);
    assert!(
        complete_absent_uninstall(
            root.path(),
            &step.service_id,
            &step.machine_id,
            &step.operation_id,
            Some(&digest(b"different"))
        )
        .is_err()
    );
    symlink("missing", root.path().join("plugins/victoria/active")).unwrap();
    assert!(call(root.path(), &step, true).is_err());
    assert!(!root.path().join("plugin-maintenance").exists());
    assert!(
        root.path()
            .join("provider-auth/providers/victoria/runtime/cached")
            .exists()
    );
    super::super::super::remove_projection::remove(
        &root.path().join("provider-auth/providers/victoria/runtime"),
    )
    .unwrap();
}

#[test]
fn stable_matching_tombstone_after_interrupted_repair_is_safe_to_finish() {
    let root = tempfile::tempdir().unwrap();
    let step = fixture(root.path());
    let journal = Journal::open(root.path()).unwrap();
    journal
        .installations
        .maintenance_removal(&step, true)
        .unwrap();
    drop(journal);
    assert_eq!(call(root.path(), &step, true).unwrap()["applied"], true);
}

#[test]
fn writable_packages_and_public_credential_directories_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let step = fixture(root.path());
    let package = root.path().join("plugins/victoria");
    fs::set_permissions(&package, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(call(root.path(), &step, true).is_err());
    fs::set_permissions(&package, fs::Permissions::from_mode(0o755)).unwrap();
    let auth = root.path().join("provider-auth/providers/victoria");
    fs::set_permissions(&auth, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(call(root.path(), &step, true).is_err());
    assert!(!root.path().join("plugin-maintenance").exists());
    super::super::super::remove_projection::remove(&auth.join("runtime")).unwrap();
}
