use super::*;

fn pending(store: &MachinePluginStore) -> (PathBuf, Profile) {
    let home = super::super::tests::ready(store, &super::super::tests::package());
    let (directory, mut profile) = store
        .read_native_profile_metadata("claude-code", 1)
        .unwrap();
    profile.identity = None;
    profile.leader_exited = false;
    profile.process_tree_exited = false;
    profile.login_succeeded = false;
    profile.process = Some(Reservation::new().unwrap());
    logout::persist(&directory, &profile).unwrap();
    assert!(home.exists());
    (directory, profile)
}

fn past_owner(process: &mut Reservation) {
    // Exercise the PID-reuse branch without requiring a privileged reboot.
    let mut wire = serde_json::to_value(&process.owner).unwrap();
    wire["start"] = serde_json::json!(wire["start"].as_u64().unwrap() + 1);
    process.owner = serde_json::from_value(wire).unwrap();
}

#[tokio::test]
async fn orphan_recovery_preserves_unknown_command_results_and_legacy_fences() {
    let temp = tempfile::tempdir().unwrap();
    let open = || {
        MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap()
    };
    let account = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    let store = open().for_account(&account).unwrap();
    let (directory, mut profile) = pending(&store);
    store
        .recover_native_auth_processes("claude-code", 1, "login-1", "logout-1")
        .await
        .unwrap();
    assert!(
        !store
            .read_native_profile_metadata("claude-code", 1)
            .unwrap()
            .1
            .process_tree_exited
    );
    assert!(
        store
            .recover_native_auth_processes("claude-code", 1, "other", "logout-1")
            .await
            .is_err()
    );
    past_owner(profile.process.as_mut().unwrap());
    logout::persist(&directory, &profile).unwrap();
    drop(store);
    let store = open().for_account(&account).unwrap();
    store
        .recover_native_auth_processes("claude-code", 1, "login-1", "logout-1")
        .await
        .unwrap();
    let (_, mut recovered) = store
        .read_native_profile_metadata("claude-code", 1)
        .unwrap();
    assert!(recovered.process_tree_exited);
    assert!(!recovered.leader_exited && !recovered.login_succeeded && recovered.identity.is_none());
    assert!(
        store
            .complete_native_profile("claude-code", 1, "login-1")
            .is_err()
    );
    let mut reservation = Reservation::new().unwrap();
    past_owner(&mut reservation);
    recovered.logout = Some(logout::Record {
        request: "logout-1".into(),
        succeeded: None,
        process_tree_exited: false,
        process: Some(reservation),
    });
    logout::persist(&directory, &recovered).unwrap();
    for _ in 0..2 {
        store
            .recover_native_auth_processes("claude-code", 1, "login-1", "logout-1")
            .await
            .unwrap();
        assert!(
            store
                .native_auth_trees_exited("claude-code", 1, "login-1", "logout-1")
                .unwrap()
        );
        assert!(matches!(
            store
                .prepare_native_logout("claude-code", 1, "login-1", "logout-1")
                .unwrap(),
            LogoutPreparation::Observed(None)
        ));
        assert!(
            store
                .cleanup_logged_out_native_profile("claude-code", 1, "login-1", "logout-1")
                .is_err()
        );
        assert!(directory.join("home").exists());
    }
    recovered.process = None; // Old journals never acquire new recovery authority.
    recovered.process_tree_exited = false;
    recovered.logout = None;
    logout::persist(&directory, &recovered).unwrap();
    store
        .recover_native_auth_processes("claude-code", 1, "login-1", "logout-1")
        .await
        .unwrap();
    assert!(
        !store
            .read_native_profile_metadata("claude-code", 1)
            .unwrap()
            .1
            .process_tree_exited
    );
}

#[tokio::test]
#[ignore = "requires delegated writable cgroup v2"]
async fn native_process_binding_is_durable_and_scoped_before_spawn() {
    let temp = tempfile::tempdir().unwrap();
    let store = MachinePluginStore::new(
        temp.path(),
        crate::machine_protocol::Platform::Linux,
        "x86_64".into(),
    )
    .unwrap()
    .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
    .unwrap();
    let (directory, _) = pending(&store);
    let mut command = tokio::process::Command::new("sleep");
    command.arg("60").kill_on_drop(true);
    let guard = crate::cgroup::Containment::prepare(&mut command).unwrap();
    assert!(
        store
            .bind_native_login_process("claude-code", 1, "other", guard.identity())
            .is_err()
    );
    store
        .bind_native_login_process("claude-code", 1, "login-1", guard.identity())
        .unwrap();
    assert!(
        store
            .bind_native_login_process("claude-code", 1, "login-1", guard.identity())
            .is_err()
    );
    let (_, profile) = store
        .read_native_profile_metadata("claude-code", 1)
        .unwrap();
    assert_eq!(
        profile.process.unwrap().containment.as_ref(),
        Some(guard.identity())
    );
    let mut child = command.spawn().unwrap();
    store
        .recover_native_auth_processes("claude-code", 1, "login-1", "logout-1")
        .await
        .unwrap();
    assert!(child.try_wait().unwrap().is_none());
    guard.finish().await.unwrap();
    child.wait().await.unwrap();
    assert!(directory.join("home").exists());
}

#[tokio::test]
#[ignore = "requires delegated writable cgroup v2"]
async fn crashed_native_owner_recovers_both_trees_without_success_or_deletion() {
    for phase in ["login", "logout"] {
        let temp = tempfile::tempdir().unwrap();
        let ready = temp.path().join("ready");
        let mut owner = tokio::process::Command::new(std::env::current_exe().unwrap());
        owner
            .args([
                "--exact",
                "machine_plugins::native_profile::processes::tests::native_crash_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("COWBOY_NATIVE_CRASH_TEST_ROOT", temp.path())
            .env("COWBOY_NATIVE_CRASH_TEST_PHASE", phase)
            .kill_on_drop(true);
        let mut owner = owner.spawn().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !ready.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let account = fs::read_to_string(ready).unwrap().try_into().unwrap();
        owner.kill().await.unwrap();
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap()
        .for_account(&account)
        .unwrap();
        store
            .recover_native_auth_processes("claude-code", 1, "login-1", "logout-1")
            .await
            .unwrap();
        store
            .recover_native_auth_processes("claude-code", 1, "login-1", "logout-1")
            .await
            .unwrap();
        let (directory, profile) = store
            .read_native_profile_for_cleanup("claude-code", 1)
            .unwrap();
        assert!(
            profile.process_tree_exited && !profile.login_succeeded && profile.identity.is_none()
        );
        if phase == "logout" {
            let record = profile.logout.unwrap();
            assert!(record.process_tree_exited && record.succeeded.is_none());
            assert!(matches!(
                store
                    .prepare_native_logout("claude-code", 1, "login-1", "logout-1")
                    .unwrap(),
                LogoutPreparation::Observed(None)
            ));
        }
        assert!(directory.join("home").exists());
        assert!(
            store
                .cleanup_logged_out_native_profile("claude-code", 1, "login-1", "logout-1")
                .is_err()
        );
    }
}

#[tokio::test]
#[ignore = "subprocess fixture for crashed_native_owner_recovers_both_trees_without_success_or_deletion"]
async fn native_crash_fixture() {
    use tokio::io::AsyncReadExt as _;
    let Ok(root) = std::env::var("COWBOY_NATIVE_CRASH_TEST_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    let store = MachinePluginStore::new(
        root,
        crate::machine_protocol::Platform::Linux,
        "x86_64".into(),
    )
    .unwrap()
    .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
    .unwrap();
    let (directory, mut profile) = pending(&store);
    let mut command = tokio::process::Command::new("sh");
    command
        .args([
            "-c",
            "setsid sh -c 'cat /proc/self/cgroup; exec sleep 60' & wait",
        ])
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true);
    let guard = crate::cgroup::Containment::prepare(&mut command).unwrap();
    if std::env::var("COWBOY_NATIVE_CRASH_TEST_PHASE").unwrap() == "logout" {
        profile.process_tree_exited = true;
        profile.logout = Some(logout::Record {
            request: "logout-1".into(),
            succeeded: None,
            process_tree_exited: false,
            process: Some(Reservation::new().unwrap()),
        });
        logout::persist(&directory, &profile).unwrap();
        store
            .bind_native_logout_process("claude-code", 1, "logout-1", guard.identity())
            .unwrap();
    } else {
        store
            .bind_native_login_process("claude-code", 1, "login-1", guard.identity())
            .unwrap();
    }
    let mut child = command.spawn().unwrap();
    let mut bytes = [0; 4096];
    assert!(child.stdout.take().unwrap().read(&mut bytes).await.unwrap() > 0);
    fs::write(root.join("ready"), store.account_id.as_str()).unwrap();
    std::future::pending::<()>().await;
}
