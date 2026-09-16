//! A separate test process owns PATH changes; parallel tests never inherit them.
use super::*;
use std::os::unix::fs::PermissionsExt as _;
use tokio::io::{AsyncBufReadExt as _, BufReader};

#[test]
fn broad_process_selectors_are_not_owned_groups() {
    for id in [0, 1, u32::MAX, i32::MAX as u32 + 1] {
        assert!(owned_group_id(id).is_none());
    }
    assert_eq!(owned_group_id(2).unwrap().as_raw_pid(), 2);
}

fn tool(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
        .unwrap()
}

#[tokio::test]
async fn group_cleanup_needs_no_path_helper() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("helper-executed");
    let helper = root.path().join("kill");
    std::fs::write(
        &helper,
        format!(
            "#!{}\nprintf wrong > '{}'\nexit 0\n",
            tool("sh").display(),
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    for path in [Path::new("/nonexistent-cowboy-test-path"), root.path()] {
        let output = tokio::time::timeout(
            Duration::from_secs(15),
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "plugin_process::process_group_tests::isolated_group_cleanup",
                    "--nocapture",
                ])
                .env("COWBOY_GROUP_TEST_SHELL", tool("sh"))
                .env("COWBOY_GROUP_TEST_SLEEP", tool("sleep"))
                .env("PATH", path)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!marker.exists(), "cleanup executed an ambient helper");
    }
}

#[tokio::test]
async fn isolated_group_cleanup() {
    let Some(shell) = std::env::var_os("COWBOY_GROUP_TEST_SHELL") else {
        return;
    };
    let sleep = std::env::var_os("COWBOY_GROUP_TEST_SLEEP").unwrap();
    let mut unrelated = Command::new(&sleep)
        .arg("60")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut command = Command::new(shell);
    command
        .args([
            "-c",
            "\"$1\" 60 </dev/null >/dev/null 2>&1 & printf '%s\\n' \"$!\"; wait",
            "fixture",
        ])
        .arg(sleep)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command.as_std_mut().process_group(0);
    let mut child = command.spawn().unwrap();
    let pid = child.id().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let descendant: u32 = line.trim().parse().unwrap();
    // Deliberately exercise the fallback with no writable cgroup, even on a
    // delegated test host. The leader remains unreaped until after signaling.
    drop(PluginProcessGroup {
        process_id: pid,
        cgroup: None,
    });
    let stopped = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    // Always clean up the exact fixture group, including when testing old code.
    if stopped.is_err() {
        let _ = rustix::process::kill_process_group(
            owned_group_id(pid).unwrap(),
            rustix::process::Signal::KILL,
        );
        let _ = child.wait().await;
    }
    let isolated = unrelated.try_wait().unwrap().is_none();
    unrelated.kill().await.unwrap();
    unrelated.wait().await.unwrap();
    assert!(stopped.is_ok(), "group leader survived cleanup");
    assert!(isolated, "cleanup affected an unrelated child");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            // Orphan reaping is PID 1's responsibility; a zombie is terminated,
            // not a running descendant. This is not a durable recovery claim.
            match std::fs::read_to_string(format!("/proc/{descendant}/status")) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Ok(status)
                    if status
                        .lines()
                        .any(|line| line.starts_with("State:") && line.contains("Z (zombie)")) =>
                {
                    break;
                }
                _ => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("descendant survived cleanup");
}
