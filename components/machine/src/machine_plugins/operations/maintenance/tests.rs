use super::*;

fn fixture(root: &Path) -> UninstallStep {
    fixture_with(root, true)
}

/// `intent`: whether the failed forward step began its slot intent.
fn fixture_with(root: &Path, intent: bool) -> UninstallStep {
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
            if intent {
                journal.installations.begin(
                    &step.plugin_id,
                    Some(&step.generation_digest),
                    None,
                    installations::Effect::Uninstall,
                    Some(step.request_digest()?),
                )?;
            }
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

/// Offline completion runs the live uninstall's refusal check: a named
/// account's private credential copy (or any named projection) refuses it and
/// is never deleted here; an empty private-copy directory goes with the
/// default scope's copies. A linked named Provider directory refuses it.
#[test]
fn offline_completion_refuses_named_credentials_and_removes_only_empty_copies() {
    let root = tempfile::tempdir().unwrap();
    let step = fixture(root.path());
    let account = root
        .path()
        .join("provider-auth/accounts/11111111-1111-4111-8111-111111111111/providers");
    let copy = account.join("victoria/private-homes/0123/home");
    fs::create_dir_all(&copy).unwrap();
    fs::write(copy.join("auth.json"), b"fixture").unwrap();
    let default_copy = root
        .path()
        .join("provider-auth/providers/victoria/private-homes/4567");
    fs::create_dir_all(&default_copy).unwrap();
    assert!(call(root.path(), &step, false).is_err());
    assert!(call(root.path(), &step, true).is_err());
    assert!(
        copy.join("auth.json").is_file(),
        "named copies are not ours"
    );
    fs::remove_dir_all(account.join("victoria/private-homes/0123")).unwrap();
    for leaf in ["runtime", "native/generations/1"] {
        fs::create_dir_all(account.join("victoria").join(leaf)).unwrap();
        assert!(call(root.path(), &step, true).is_err(), "{leaf}");
        fs::remove_dir_all(
            account
                .join("victoria")
                .join(leaf.split('/').next().unwrap()),
        )
        .unwrap();
    }
    let outside = root.path().join("outside");
    fs::create_dir_all(outside.join("private-homes/kept")).unwrap();
    let other = root
        .path()
        .join("provider-auth/accounts/22222222-2222-4222-8222-222222222222/providers");
    fs::create_dir_all(&other).unwrap();
    symlink(&outside, other.join("victoria")).unwrap();
    assert!(call(root.path(), &step, true).is_err());
    assert!(outside.join("private-homes/kept").exists());
    fs::remove_file(other.join("victoria")).unwrap();
    assert_eq!(call(root.path(), &step, true).unwrap()["applied"], true);
    assert!(!account.join("victoria/private-homes").exists());
    assert!(!default_copy.exists());
}

/// Roll forward: an uncertain uninstall that never removed its active link
/// (or lost that removal) is finished, link first, and recorded applied with
/// the matching tombstone; an interrupted receipt is accepted too.
#[test]
fn offline_completion_rolls_a_present_active_link_forward() {
    let root = tempfile::tempdir().unwrap();
    let step = fixture(root.path());
    let generation = digest_generation_name(&step.generation_digest).unwrap();
    let plugin = root.path().join("plugins/victoria");
    fs::create_dir_all(plugin.join("generations").join(&generation)).unwrap();
    symlink(format!("generations/{generation}"), plugin.join("active")).unwrap();
    {
        let journal = Journal::open(root.path()).unwrap();
        let mut state = journal.state.lock();
        let key = step.key().unwrap();
        let receipt = state.receipts.get_mut(&key).unwrap();
        receipt.outcome = StepOutcome::Unknown {
            reason: StepUncertainty::Interrupted,
        };
        journal.persist(&key, receipt).unwrap();
    }
    let preview = call(root.path(), &step, false).unwrap();
    assert_eq!(preview["removes_active_link"], true);
    assert!(plugin.join("active").is_symlink());
    assert_eq!(call(root.path(), &step, true).unwrap()["applied"], true);
    assert!(plugin.join("active").symlink_metadata().is_err());
    assert!(plugin.join("generations").join(&generation).is_dir());
    assert!(
        !root
            .path()
            .join("provider-auth/providers/victoria/runtime")
            .exists()
    );
    let reopened = Journal::open(root.path()).unwrap();
    assert!(
        matches!(reopened.query(&step), StepLookup::Found { receipt } if receipt.outcome == StepOutcome::Applied {})
    );
    reopened.ensure_unfenced(&step.plugin_id).unwrap();
}

/// Another generation as the active link is never removed.
#[test]
fn offline_completion_refuses_a_different_active_generation() {
    let root = tempfile::tempdir().unwrap();
    let step = fixture(root.path());
    let other = digest_generation_name(&digest(b"other release")).unwrap();
    let plugin = root.path().join("plugins/victoria");
    fs::create_dir_all(plugin.join("generations").join(&other)).unwrap();
    symlink(format!("generations/{other}"), plugin.join("active")).unwrap();
    assert!(call(root.path(), &step, true).is_err());
    assert!(plugin.join("active").is_symlink());
    super::super::super::remove_projection::remove(
        &root.path().join("provider-auth/providers/victoria/runtime"),
    )
    .unwrap();
}

/// A linked accounts root is refused, never read as absent: no completion
/// record is written while copies behind it may remain reachable.
#[test]
fn offline_completion_refuses_a_linked_accounts_root() {
    let root = tempfile::tempdir().unwrap();
    let step = fixture(root.path());
    let outside = root.path().join("outside");
    let copy =
        outside.join("11111111-1111-4111-8111-111111111111/providers/victoria/private-homes/0123");
    fs::create_dir_all(&copy).unwrap();
    symlink(&outside, root.path().join("provider-auth/accounts")).unwrap();
    assert!(call(root.path(), &step, true).is_err());
    assert!(copy.exists());
    let journal = Journal::open(root.path()).unwrap();
    assert!(
        matches!(journal.query(&step), StepLookup::Found { receipt } if matches!(receipt.outcome, StepOutcome::Unknown { .. }))
    );
    drop(journal);
    fs::remove_file(root.path().join("provider-auth/accounts")).unwrap();
    assert_eq!(call(root.path(), &step, true).unwrap()["applied"], true);
}

/// The forward step records its uncertain receipt before it begins the slot
/// intent; a crash between the two leaves the original stable installation.
/// Roll-forward accepts exactly that and writes the matching tombstone.
#[test]
fn offline_completion_rolls_forward_a_step_interrupted_before_its_intent() {
    let root = tempfile::tempdir().unwrap();
    let step = fixture_with(root.path(), false);
    {
        let journal = Journal::open(root.path()).unwrap();
        assert!(journal.ensure_unfenced(&step.plugin_id).is_err());
        assert!(
            journal
                .installations
                .revision(&step.plugin_id, &step.generation_digest)
                .is_ok(),
            "the original installation is still stable"
        );
    }
    assert_eq!(call(root.path(), &step, true).unwrap()["applied"], true);
    let reopened = Journal::open(root.path()).unwrap();
    reopened.ensure_unfenced(&step.plugin_id).unwrap();
    assert!(
        reopened
            .installations
            .revision(&step.plugin_id, &step.generation_digest)
            .is_err(),
        "the slot now holds the removal tombstone"
    );
    reopened
        .installations
        .maintenance_removal(&step, false)
        .unwrap();
}

/// An outstanding login candidate home, default scope or named, refuses
/// offline completion as it refuses the live uninstall.
#[test]
fn offline_completion_refuses_outstanding_login_candidates() {
    let root = tempfile::tempdir().unwrap();
    let step = fixture(root.path());
    for candidates in [
        "provider-auth/providers/victoria/candidates",
        "provider-auth/accounts/11111111-1111-4111-8111-111111111111/providers/victoria/candidates",
    ] {
        let home = root.path().join(candidates).join("request-1/home");
        fs::create_dir_all(&home).unwrap();
        assert!(call(root.path(), &step, true).is_err(), "{candidates}");
        fs::remove_dir_all(root.path().join(candidates).join("request-1")).unwrap();
    }
    assert_eq!(call(root.path(), &step, true).unwrap()["applied"], true);
}
