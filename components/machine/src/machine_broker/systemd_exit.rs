//! Evidence for one observed transient unit incarnation, never a stop ACK.
use anyhow::{Context as _, Result, ensure};
use std::{collections::BTreeMap, path::Path, time::Duration};
use tokio::process::Command;

use crate::machine_plugins::{MachinePluginStore, SystemdWorker};

pub(super) use crate::runtime_wire::WORKER_LAUNCH_ENV as LAUNCH_ENV;

pub(super) async fn release_gate(
    store: &MachinePluginStore,
    session: &str,
    unit: &str,
    launch: uuid::Uuid,
    input: &mut tokio::process::ChildStdin,
) -> Result<()> {
    use tokio::io::AsyncWriteExt as _;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if capture(store, session, unit).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("worker launch identity was not captured")?;
    let receipt = store
        .account_worker_exit(session)?
        .context("worker launch missing")?;
    ensure!(
        receipt.launch == launch
            && !receipt.tree_exited
            && receipt
                .systemd
                .as_ref()
                .is_some_and(|binding| binding.unit == unit)
            && !store.account_session_stopped(session)?,
        "worker launch is no longer admitted"
    );
    input.write_all(format!("{launch}\n").as_bytes()).await?;
    Ok(())
}

fn fields(text: &str) -> Result<BTreeMap<&str, &str>> {
    let mut result = BTreeMap::new();
    for line in text.lines() {
        let (key, value) = line.split_once('=').context("invalid unit observation")?;
        ensure!(
            result.insert(key, value).is_none(),
            "duplicate unit property"
        );
    }
    Ok(result)
}

async fn show(unit: &str) -> Result<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(2),
        Command::new("systemctl")
            .kill_on_drop(true)
            .args([
                "--user",
                "show",
                unit,
                "--property=Id,LoadState,ActiveState,Job,InvocationID,ControlGroup,Environment",
            ])
            .output(),
    )
    .await??;
    ensure!(output.status.success(), "unit query failed");
    String::from_utf8(output.stdout).context("invalid unit encoding")
}

fn boot() -> Result<uuid::Uuid> {
    Ok(std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .parse()?)
}

fn cgroup_path(binding: &SystemdWorker) -> Result<std::path::PathBuf> {
    let path = Path::new(&binding.cgroup);
    ensure!(
        path.is_absolute()
            && path
                .components()
                .skip(1)
                .all(|part| matches!(part, std::path::Component::Normal(_)))
            && path.file_name().and_then(|name| name.to_str()) == Some(binding.unit.as_str()),
        "invalid worker cgroup scope"
    );
    Ok(Path::new("/sys/fs/cgroup").join(path.strip_prefix("/")?))
}

fn binding(text: &str, unit: &str, launch: uuid::Uuid, boot: uuid::Uuid) -> Result<SystemdWorker> {
    let fields = fields(text)?;
    ensure!(
        fields.get("Id") == Some(&unit) && fields.get("LoadState") == Some(&"loaded"),
        "unit identity unavailable"
    );
    let environment =
        shell_words::split(fields.get("Environment").context("missing environment")?)?;
    let expected = format!("{LAUNCH_ENV}={launch}");
    let markers: Vec<_> = environment
        .iter()
        .filter(|value| value.starts_with(&format!("{LAUNCH_ENV}=")))
        .collect();
    ensure!(
        markers.len() == 1 && markers[0] == &expected,
        "unit launch mismatch"
    );
    let invocation: uuid::Uuid = fields
        .get("InvocationID")
        .context("missing invocation")?
        .parse()?;
    ensure!(
        !invocation.is_nil() && !boot.is_nil(),
        "invalid unit incarnation"
    );
    let binding = SystemdWorker {
        unit: unit.into(),
        invocation,
        boot,
        cgroup: fields
            .get("ControlGroup")
            .context("missing cgroup")?
            .to_string(),
    };
    cgroup_path(&binding)?;
    Ok(binding)
}

pub(super) async fn capture(store: &MachinePluginStore, session: &str, unit: &str) -> Result<()> {
    let receipt = store
        .account_worker_exit(session)?
        .context("worker launch missing")?;
    if receipt.tree_exited {
        return Ok(());
    }
    if let Some(binding) = receipt.systemd {
        // A prior atomic rename may have succeeded while directory fsync
        // failed. Re-persist before this observation can release a worker.
        return store.bind_account_worker_systemd(
            session,
            &receipt.account,
            receipt.launch,
            binding,
        );
    }
    let observed = binding(&show(unit).await?, unit, receipt.launch, boot()?)?;
    // Observe the actual directory before persisting its identity. Later
    // absence is evidence only for this previously existing cgroup.
    ensure!(
        std::fs::symlink_metadata(cgroup_path(&observed)?)?.is_dir(),
        "worker cgroup missing"
    );
    store.bind_account_worker_systemd(session, &receipt.account, receipt.launch, observed)
}

fn stopped(text: &str, expected: &SystemdWorker) -> Result<()> {
    let fields = fields(text)?;
    ensure!(
        fields.get("Id") == Some(&expected.unit.as_str()),
        "unit identity changed"
    );
    ensure!(
        matches!(fields.get("LoadState"), Some(&"loaded" | &"not-found")),
        "unit load unknown"
    );
    ensure!(
        matches!(fields.get("ActiveState"), Some(&"inactive" | &"failed"))
            && fields.get("Job") == Some(&""),
        "unit still active or queued"
    );
    let invocation = fields.get("InvocationID").context("missing invocation")?;
    ensure!(
        invocation.is_empty() || invocation.parse::<uuid::Uuid>()? == expected.invocation,
        "unit incarnation changed"
    );
    let cgroup = fields.get("ControlGroup").context("missing cgroup")?;
    ensure!(
        cgroup.is_empty() || *cgroup == expected.cgroup,
        "unit cgroup changed"
    );
    Ok(())
}

pub(super) async fn stop_allowed(
    store: &MachinePluginStore,
    session: &str,
    unit: &str,
) -> Result<bool> {
    store.reconcile_account_worker_boot(session)?;
    let Some(receipt) = store.account_worker_exit(session)? else {
        return Ok(true);
    };
    if receipt.tree_exited {
        return Ok(false);
    }
    if receipt.systemd.is_none() {
        capture(store, session, unit).await?;
    }
    if confirm(store, session).await.is_ok() {
        return Ok(false);
    }
    let receipt = store
        .account_worker_exit(session)?
        .context("worker launch missing")?;
    let expected = receipt.systemd.context("worker unit was not observed")?;
    ensure!(
        expected.unit == unit && expected.boot == boot()?,
        "worker unit scope changed"
    );
    let observed = binding(&show(unit).await?, unit, receipt.launch, expected.boot)?;
    ensure!(observed == expected, "worker unit incarnation changed");
    Ok(true)
}

fn empty(directory: &Path) -> Result<()> {
    match std::fs::symlink_metadata(directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        result => ensure!(result?.is_dir(), "invalid worker cgroup"),
    }
    let events = std::fs::read_to_string(directory.join("cgroup.events"))?;
    let populations: Vec<_> = events
        .lines()
        .filter_map(|line| line.strip_prefix("populated "))
        .collect();
    ensure!(populations == ["0"], "worker cgroup exit unconfirmed");
    Ok(())
}

fn collected(text: &str, unit: &str) -> Result<()> {
    let fields = fields(text)?;
    ensure!(
        fields.get("Id") == Some(&unit)
            && fields.get("LoadState") == Some(&"not-found")
            && fields.get("ActiveState") == Some(&"inactive")
            && ["Job", "InvocationID", "ControlGroup", "Environment"]
                .iter()
                .all(|key| fields.get(key) == Some(&"")),
        "legacy worker unit absence unconfirmed"
    );
    Ok(())
}

/// Default workers predate the account exit journal. A collected unit is an
/// observed legacy stop, never a substitute for a named worker's tree proof.
pub(super) async fn confirm_recycled(
    store: &MachinePluginStore,
    session: &str,
    unit: &str,
) -> Result<()> {
    store.reconcile_account_worker_boot(session)?;
    if store.account_worker_exit(session)?.is_some() {
        return confirm(store, session).await;
    }
    collected(&show(unit).await?, unit)?;
    ensure!(
        store.account_worker_exit(session)?.is_none(),
        "worker launch appeared during stop observation"
    );
    collected(&show(unit).await?, unit)
}

/// Prove the previous named launch's tree exit before a replacement launch
/// (generation cutover, its fallback, or a reload) on the same stable unit.
/// Its launching broker's reaper normally records this, but a Machine host
/// restart loses that reaper and an in-process replacement can win the race
/// with it, after which the journal refuses every relaunch and the session is
/// left without a worker. The caller has observed the stable unit collected;
/// this rechecks the exact recorded incarnation and its empty cgroup, so a live
/// or unobserved previous worker still refuses the launch (one owner).
pub(super) async fn confirm_replaced(store: &MachinePluginStore, session: &str) -> Result<()> {
    const ATTEMPTS: u32 = 20;
    for attempt in 1..=ATTEMPTS {
        store.reconcile_account_worker_boot(session)?;
        match store.account_worker_exit(session)? {
            Some(receipt) if !receipt.tree_exited => {}
            _ => return Ok(()),
        }
        match confirm(store, session).await {
            Ok(()) => return Ok(()),
            Err(error) if attempt == ATTEMPTS => return Err(error),
            Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
    unreachable!("the final attempt returns")
}

pub(super) async fn confirm(store: &MachinePluginStore, session: &str) -> Result<()> {
    store.reconcile_account_worker_boot(session)?;
    let receipt = store
        .account_worker_exit(session)?
        .context("worker launch missing")?;
    if receipt.tree_exited {
        return Ok(());
    }
    let binding = receipt.systemd.context("worker unit was not observed")?;
    ensure!(binding.boot == boot()?, "worker boot changed");
    stopped(&show(&binding.unit).await?, &binding)?;
    ensure!(
        Path::new("/sys/fs/cgroup/cgroup.controllers").is_file(),
        "cgroup v2 unavailable"
    );
    empty(&cgroup_path(&binding)?)?;
    // Reject a replacement or queued restart appearing during the filesystem
    // observation. The launch journal prevents Cowboy replacing this worker.
    stopped(&show(&binding.unit).await?, &binding)?;
    store.record_account_worker_exit(session, &receipt.account, receipt.launch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collected_legacy_unit_requires_positive_absence() {
        let text = "Id=worker.service\nLoadState=not-found\nActiveState=inactive\nJob=\nInvocationID=\nControlGroup=\nEnvironment=\n";
        assert!(collected(text, "worker.service").is_ok());
        assert!(collected(text, "another.service").is_err());
        for (from, to) in [
            ("LoadState=not-found", "LoadState=loaded"),
            ("LoadState=not-found", "LoadState=error"),
            ("ActiveState=inactive", "ActiveState=activating"),
            ("Job=", "Job=1 start"),
            ("InvocationID=", "InvocationID=another"),
            ("ControlGroup=", "ControlGroup=/still-owned"),
            ("Environment=", "Environment=launch=other"),
        ] {
            assert!(collected(&text.replace(from, to), "worker.service").is_err());
        }
        assert!(collected(&text.replace("Job=\n", ""), "worker.service").is_err());
        assert!(collected(&format!("{text}Job=\n"), "worker.service").is_err());
    }

    #[tokio::test]
    #[ignore = "requires user systemd and cgroup v2"]
    async fn systemd_short_worker_is_bound_before_gate_release() {
        let temp = tempfile::tempdir().unwrap();
        let account = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        for admitted in [true, false] {
            let store = MachinePluginStore::new(
                temp.path(),
                crate::machine_protocol::Platform::Linux,
                "x86_64".into(),
            )
            .unwrap();
            let session = format!("short-{admitted}");
            let marker = temp.path().join(&session);
            let unit = format!("cowboy-gate-test-{}.service", uuid::Uuid::new_v4());
            let mut command = Command::new("systemd-run");
            command.args(["--user", "--quiet", "--wait", "--collect", "--pipe", "--service-type=exec",
                "--property=KillMode=control-group", "--property=RuntimeMaxSec=10s",
                &format!("--unit={unit}"), &format!("--setenv={LAUNCH_ENV}"),
                "sh", "-c", "IFS= read -r token && test \"$token\" = \"$COWBOY_ACCOUNT_WORKER_LAUNCH\" && : > \"$1\"", "sh"])
                .arg(&marker).stdin(std::process::Stdio::piped());
            let (mut child, launch) =
                super::super::account_worker::spawn(&mut command, &store, &session, &account)
                    .unwrap();
            let mut input = child.stdin.take().unwrap();
            assert!(!marker.exists());
            if admitted {
                release_gate(&store, &session, &unit, launch.unwrap(), &mut input)
                    .await
                    .unwrap();
                assert!(
                    store
                        .account_worker_exit(&session)
                        .unwrap()
                        .unwrap()
                        .systemd
                        .is_some()
                );
            }
            drop(input);
            let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(status.success(), admitted);
            assert_eq!(marker.exists(), admitted);
            drop(store);
            if admitted {
                let reopened = MachinePluginStore::new(
                    temp.path(),
                    crate::machine_protocol::Platform::Linux,
                    "x86_64".into(),
                )
                .unwrap();
                confirm(&reopened, &session).await.unwrap();
                assert!(
                    reopened
                        .account_worker_exit(&session)
                        .unwrap()
                        .unwrap()
                        .tree_exited
                );
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires user systemd and cgroup v2"]
    async fn systemd_exit_observes_real_tree_after_store_restart() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let account = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let unit = format!("cowboy-exit-proof-test-{}.service", uuid::Uuid::new_v4());
        let mut command = Command::new("systemd-run");
        command.args([
            "--user",
            "--quiet",
            "--wait",
            "--collect",
            "--service-type=exec",
            "--property=KillMode=control-group",
            "--property=TimeoutStopSec=1s",
            "--property=RuntimeMaxSec=15s",
            &format!("--unit={unit}"),
            &format!("--setenv={LAUNCH_ENV}"),
            "sh",
            "-c",
            "setsid sleep 60 & wait",
        ]);
        let (mut child, launch) =
            super::super::account_worker::spawn(&mut command, &store, "s", &account).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if capture(&store, "s", &unit).await.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(confirm(&store, "s").await.is_err());
        let saved = store.account_worker_exit("s").unwrap().unwrap();
        let bound = saved.systemd.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let processes =
                    std::fs::read_to_string(cgroup_path(&bound).unwrap().join("cgroup.procs"))
                        .unwrap();
                if processes.lines().count() >= 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(stop_allowed(&store, "s", &unit).await.unwrap());
        assert!(
            stop_allowed(&store, "s", "different.service")
                .await
                .is_err()
        );
        // Rebinding the same launch cannot replace the observed invocation.
        let mut changed = bound.clone();
        changed.invocation = uuid::Uuid::new_v4();
        assert!(
            store
                .bind_account_worker_systemd("s", &account, launch.unwrap(), changed)
                .is_err()
        );
        drop(store);
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        assert_eq!(
            store.account_worker_exit("s").unwrap().unwrap().systemd,
            Some(bound)
        );
        let status = Command::new("systemctl")
            .args(["--user", "stop", &unit])
            .status()
            .await
            .unwrap();
        assert!(status.success());
        let _ = child.wait().await.unwrap();
        confirm(&store, "s").await.unwrap();
        assert!(store.account_worker_exit("s").unwrap().unwrap().tree_exited);
        confirm(&store, "s").await.unwrap();
        assert!(!stop_allowed(&store, "s", &unit).await.unwrap());
    }

    /// 2026-10-31 OVH `--workers` rollout: a named worker launched by the
    /// previous Machine host was stopped by the cutover, but only that host's
    /// lost reaper would have recorded its tree exit, so the replacement and
    /// its fallback were both refused and the session was left without one.
    #[tokio::test]
    #[ignore = "requires user systemd and cgroup v2"]
    async fn replacement_proves_a_collected_named_unit_whose_reaper_was_lost() {
        let temp = tempfile::tempdir().unwrap();
        let open = || {
            MachinePluginStore::new(
                temp.path(),
                crate::machine_protocol::Platform::Linux,
                "x86_64".into(),
            )
            .unwrap()
        };
        let account: crate::provider_account::ProviderAccountId =
            uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let unit = format!("cowboy-handoff-test-{}.service", uuid::Uuid::new_v4());
        let store = open();
        let mut command = Command::new("systemd-run");
        command.args([
            "--user",
            "--quiet",
            "--collect",
            "--service-type=exec",
            "--property=KillMode=control-group",
            "--property=TimeoutStopSec=1s",
            "--property=RuntimeMaxSec=30s",
            &format!("--unit={unit}"),
            &format!("--setenv={LAUNCH_ENV}"),
            "sleep",
            "60",
        ]);
        let (mut child, _) =
            super::super::account_worker::spawn(&mut command, &store, "s", &account).unwrap();
        assert!(child.wait().await.unwrap().success());
        tokio::time::timeout(Duration::from_secs(5), async {
            while capture(&store, "s", &unit).await.is_err() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        // The launching host (and its reaper) is gone; a new one owns the store.
        drop(store);
        let store = open();
        // A live previous incarnation is never proven exited: one owner.
        assert!(confirm_replaced(&store, "s").await.is_err());
        assert!(store.begin_account_worker("s", &account).is_err());
        let status = Command::new("systemctl")
            .args(["--user", "stop", &unit])
            .status()
            .await
            .unwrap();
        assert!(status.success());
        tokio::time::timeout(Duration::from_secs(10), async {
            while collected(&show(&unit).await.unwrap(), &unit).is_err() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            store
                .begin_account_worker("s", &account)
                .unwrap_err()
                .to_string()
                .contains("previous worker tree exit unconfirmed")
        );
        confirm_replaced(&store, "s").await.unwrap();
        assert!(store.account_worker_exit("s").unwrap().unwrap().tree_exited);
        store.begin_account_worker("s", &account).unwrap();
    }

    fn fixture() -> (SystemdWorker, String, uuid::Uuid) {
        let launch = uuid::Uuid::new_v4();
        let binding = SystemdWorker {
            unit: "cowboy-proof.service".into(),
            invocation: uuid::Uuid::new_v4(),
            boot: uuid::Uuid::new_v4(),
            cgroup: "/user.slice/cowboy-proof.service".into(),
        };
        let text = format!(
            "Id={}\nLoadState=loaded\nActiveState=inactive\nJob=\nInvocationID={}\nControlGroup={}\nEnvironment={LAUNCH_ENV}={launch}\n",
            binding.unit,
            binding.invocation.simple(),
            binding.cgroup
        );
        (binding, text, launch)
    }

    #[test]
    fn systemd_exit_rejects_wrong_launch_replacement_and_queued_job() {
        let (expected, text, launch) = fixture();
        assert_eq!(
            binding(&text, &expected.unit, launch, expected.boot).unwrap(),
            expected
        );
        assert!(binding(&text, &expected.unit, uuid::Uuid::new_v4(), expected.boot).is_err());
        stopped(&text, &expected).unwrap();
        for changed in [
            text.replace("inactive", "active"),
            text.replace("Job=", "Job=42"),
            text.replace(
                &expected.invocation.simple().to_string(),
                &uuid::Uuid::new_v4().simple().to_string(),
            ),
            text.replace("LoadState=loaded", "LoadState=error"),
            format!("{text}Job=\n"),
        ] {
            assert!(stopped(&changed, &expected).is_err());
        }
        let collected = format!(
            "Id={}\nLoadState=not-found\nActiveState=inactive\nJob=\nInvocationID=\nControlGroup=\n",
            expected.unit
        );
        stopped(&collected, &expected).unwrap();
    }

    #[test]
    fn systemd_exit_requires_empty_tree_and_scoped_path() {
        let temp = tempfile::tempdir().unwrap();
        assert!(empty(temp.path()).is_err());
        for value in ["populated 1\n", "", "populated 0\npopulated 1\n"] {
            std::fs::write(temp.path().join("cgroup.events"), value).unwrap();
            assert!(empty(temp.path()).is_err());
        }
        std::fs::write(temp.path().join("cgroup.events"), "populated 0\n").unwrap();
        empty(temp.path()).unwrap();
        empty(&temp.path().join("collected")).unwrap();
        let (mut binding, _, _) = fixture();
        for value in [
            "/",
            "/user.slice/../cowboy-proof.service",
            "/other.service",
            "relative",
        ] {
            binding.cgroup = value.into();
            assert!(cgroup_path(&binding).is_err());
        }
    }
}
