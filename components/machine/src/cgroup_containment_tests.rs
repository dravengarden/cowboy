#![allow(unused_imports)]
use crate::cgroup::containment::linux;
use crate::cgroup::containment::*;
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf};
use tokio::{io::AsyncReadExt as _, process::Command};

async fn detached() -> (Containment, tokio::process::Child) {
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            "setsid sh -c 'cat /proc/self/cgroup; exec sleep 60' & wait",
        ])
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true);
    let guard = Containment::prepare(&mut command).unwrap();
    let mut child = command.spawn().unwrap();
    let mut bytes = [0; 4096];
    let n = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        child.stdout.take().unwrap().read(&mut bytes),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        std::str::from_utf8(&bytes[..n]).unwrap().contains(
            guard
                .identity
                .directory
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
        )
    );
    (guard, child)
}

#[tokio::test]
#[ignore = "requires delegated writable cgroup v2"]
async fn native_auth_contains_detached_descendants_before_exec() {
    let (guard, mut child) = detached().await;
    let identity = guard.identity.clone();
    // Include a nested group left behind by a worker's own subprocess owner.
    std::fs::create_dir(identity.directory.join("nested")).unwrap();
    let (first, second) = tokio::join!(guard.finish(), identity.terminate());
    first.unwrap();
    second.unwrap();
    assert!(!child.wait().await.unwrap().success());
    assert!(!identity.directory.exists());
    identity.terminate().await.unwrap();
}

#[tokio::test]
#[ignore = "requires delegated writable cgroup v2"]
async fn removed_group_fences_delayed_spawn_and_replacement_identity() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("executed");
    let mut command = Command::new("touch");
    command.arg(&marker);
    let guard = Containment::prepare(&mut command).unwrap();
    let identity = guard.identity.clone();
    identity.terminate().await.unwrap();
    assert!(command.spawn().is_err());
    assert!(!marker.exists());
    std::fs::create_dir(&identity.directory).unwrap();
    assert!(identity.terminate().await.is_err());
    drop(guard); // A stale Drop must not kill or unlink the replacement either.
    assert!(identity.directory.exists());
    std::fs::remove_dir(&identity.directory).unwrap();
}

#[tokio::test]
#[ignore = "requires delegated writable cgroup v2"]
async fn changed_view_or_missing_ancestor_is_not_exit_evidence() {
    let (guard, mut child) = detached().await;
    let mut changed = guard.identity.clone();
    let name = changed.directory.file_name().unwrap().to_owned();
    changed.directory = changed
        .directory
        .parent()
        .unwrap()
        .join(format!("missing-{}", uuid::Uuid::new_v4()))
        .join(name);
    assert!(changed.terminate().await.is_err());
    changed = guard.identity.clone();
    changed.root_inode = Some(changed.root_inode.unwrap() + 1);
    assert!(changed.terminate().await.is_err());
    changed.root_inode = None; // An old journal cannot prove an absent view.
    assert!(changed.terminate().await.is_err());
    assert!(child.try_wait().unwrap().is_none());
    guard.finish().await.unwrap();
    child.wait().await.unwrap();
}

#[tokio::test]
#[ignore = "requires delegated writable cgroup v2 and private user/mount/cgroup namespaces"]
async fn containment_works_at_a_private_cgroup_namespace_root() {
    let temp = tempfile::tempdir().unwrap();
    let identity_file = temp.path().join("parent-view.json");
    let mut command = Command::new("unshare");
    command.args(["--user", "--map-current-user", "--keep-caps", "--mount", "--cgroup", "--pid", "--fork", "--mount-proc",
        "sh", "-ec", "mount --make-rprivate /; mount -t cgroup2 none /sys/fs/cgroup; exec \"$1\" --exact cgroup::containment::tests::namespace_root_fixture --ignored --nocapture", "root-fixture"])
        .arg(std::env::current_exe().unwrap())
        .env("COWBOY_CONTAINMENT_ROOT_FIXTURE", &identity_file)
        .kill_on_drop(true);
    let guard = Containment::prepare(&mut command).unwrap();
    std::fs::write(
        &identity_file,
        serde_json::to_vec(guard.identity()).unwrap(),
    )
    .unwrap();
    let status = command.status().await.unwrap();
    guard.finish().await.unwrap();
    assert!(status.success());
}

#[tokio::test]
#[ignore = "subprocess fixture for containment_works_at_a_private_cgroup_namespace_root"]
async fn namespace_root_fixture() {
    let Ok(identity_file) = std::env::var("COWBOY_CONTAINMENT_ROOT_FIXTURE") else {
        return;
    };
    assert_eq!(
        crate::cgroup::own_cgroup().as_deref(),
        Some(std::path::Path::new(crate::cgroup::CGROUP_ROOT))
    );
    // The parent-owned tree is still populated (we are in it), but its old
    // absolute path is invisible through this real, remounted cgroup view.
    let parent: ContainmentIdentity =
        serde_json::from_slice(&std::fs::read(identity_file).unwrap()).unwrap();
    assert!(parent.owner.exited().is_err());
    assert!(parent.terminate().await.is_err());
    let (guard, mut child) = detached().await;
    let identity = guard.identity.clone();
    guard.finish().await.unwrap();
    child.wait().await.unwrap();
    identity.terminate().await.unwrap();
    assert!(!identity.directory.exists());
}

#[tokio::test]
#[ignore = "requires delegated writable cgroup v2"]
async fn crashed_owner_recovers_only_its_journaled_worker() {
    let temp = tempfile::tempdir().unwrap();
    let ready = temp.path().join("ready");
    let mut owner = Command::new(std::env::current_exe().unwrap());
    owner
        .args([
            "--exact",
            "cgroup::containment::tests::crash_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("COWBOY_CONTAINMENT_TEST_ROOT", temp.path())
        .kill_on_drop(true);
    let mut owner = owner.spawn().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !ready.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let identity: ContainmentIdentity =
        serde_json::from_slice(&std::fs::read(&ready).unwrap()).unwrap();
    assert!(!identity.owner.exited().unwrap());
    let (other, mut other_child) = detached().await;
    owner.kill().await.unwrap(); // SIGKILL skips Containment::drop and reaper.
    assert!(identity.owner.exited().unwrap());
    assert!(
        std::fs::read_to_string(identity.directory.join("cgroup.events"))
            .unwrap()
            .contains("populated 1")
    );
    let store = crate::machine_plugins::MachinePluginStore::new(
        temp.path(),
        crate::machine_protocol::Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    assert!(
        !store
            .account_worker_exit("crash")
            .unwrap()
            .unwrap()
            .tree_exited
    );
    store.terminate_account_worker_tree("crash").await.unwrap();
    store.terminate_account_worker_tree("crash").await.unwrap();
    assert!(
        store
            .account_worker_exit("crash")
            .unwrap()
            .unwrap()
            .tree_exited
    );
    assert!(!identity.directory.exists());
    assert!(other_child.try_wait().unwrap().is_none());
    other.finish().await.unwrap();
    other_child.wait().await.unwrap();
}

#[tokio::test]
#[ignore = "subprocess fixture for crashed_owner_recovers_only_its_journaled_worker"]
async fn crash_fixture() {
    let Ok(root) = std::env::var("COWBOY_CONTAINMENT_TEST_ROOT") else {
        return;
    };
    let root = std::path::Path::new(&root);
    let store = crate::machine_plugins::MachinePluginStore::new(
        root,
        crate::machine_protocol::Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            "setsid sh -c 'cat /proc/self/cgroup; exec sleep 60' & wait",
        ])
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true);
    let guard = Containment::prepare(&mut command).unwrap();
    let launch: crate::runtime_wire::StartSession = serde_json::from_value(serde_json::json!({
        "session_id":"crash", "provider":"claude-code", "provider_account_id":uuid::Uuid::new_v4().to_string(),
        "provider_version":"fixture", "provider_generation_digest":"digest", "provider_auth_generation":1,
        "cwd":"/fixture", "generation":"worker"
    })).unwrap();
    store
        .begin_contained_account_worker(&launch, Some(guard.identity()))
        .unwrap();
    let mut child = command.spawn().unwrap();
    let mut bytes = [0; 4096];
    let n = child.stdout.take().unwrap().read(&mut bytes).await.unwrap();
    assert!(n > 0);
    std::fs::write(
        root.join("ready"),
        serde_json::to_vec(guard.identity()).unwrap(),
    )
    .unwrap();
    std::future::pending::<()>().await;
}

#[tokio::test]
async fn failed_pre_exec_membership_never_runs_provider_code() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("executed");
    let mut command = Command::new("touch");
    command.arg(&marker);
    linux::attach_before_exec(
        &mut command,
        std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap(),
    );
    assert!(command.spawn().is_err());
    assert!(!marker.exists());
}

#[tokio::test]
async fn invalid_identity_cannot_authorize_a_kill_or_absence() {
    let temp = tempfile::tempdir().unwrap();
    let mut identity = ContainmentIdentity {
        owner: ProcessOwner::current().unwrap(),
        directory: temp
            .path()
            .join(format!("agent-contained-{}", uuid::Uuid::new_v4())),
        device: 1,
        inode: 1,
        root_inode: None,
    };
    assert!(identity.terminate().await.is_err());
    identity.directory = PathBuf::from(
        "/sys/fs/cgroup/../../../tmp/agent-contained-00000000-0000-0000-0000-000000000000",
    );
    assert!(identity.terminate().await.is_err());
    identity.owner.boot = uuid::Uuid::nil();
    assert!(identity.owner.exited().is_err());
}

#[test]
fn process_owner_checks_start_time_and_boot_not_only_pid() {
    let owner = ProcessOwner::current().unwrap();
    assert!(!owner.exited().unwrap());
    let mut foreign = owner.clone();
    foreign.pid_namespace = None;
    assert!(foreign.exited().is_err());
    foreign.pid_namespace = Some(owner.pid_namespace.unwrap() + 1);
    assert!(foreign.exited().is_err());
    let mut reused = owner.clone();
    reused.start += 1;
    assert!(reused.exited().unwrap());
    reused = owner;
    reused.boot = uuid::Uuid::new_v4();
    assert!(reused.exited().unwrap());
    reused.start = 0;
    assert!(reused.exited().is_err());
}
