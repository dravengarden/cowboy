//! The transient user unit of one browser instance and its cgroup tree.
//! The Machine starts it with the limits of the moment, observes it through
//! `systemctl show`, stops it through systemd, and freezes or thaws the
//! launcher's `browser/` sub-cgroup for coherent snapshots.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use tokio::process::Command;

pub const UNIT_PREFIX: &str = "cowboy-browser-";
pub const SLICE: &str = "cowboy-agents.slice";
/// `RuntimeDirectory=` parent below `$XDG_RUNTIME_DIR`; it holds only the two
/// sockets of each instance.
pub const RUNTIME_DIRECTORY_PARENT: &str = "cowboy-browser";
/// `TimeoutStopSec` of the unit: the launcher's graceful stop must fit in it.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(100);
/// Longest any single `systemctl` or `systemd-run` invocation may take; a
/// stalled user manager must never hold a stop (or the account queue behind
/// it) indefinitely.
pub const SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest the systemd phase of [`terminate`] (stop, wait, `SIGKILL`, wait)
/// may take before the `cgroup.kill` fallback runs.
const SYSTEMD_PHASE_TIMEOUT: Duration = Duration::from_secs(45);

/// Run one bounded subprocess: killed when `timeout` passes (or the future
/// is dropped), and reported as an error then.
///
/// # Errors
/// Returns when the program cannot be spawned or does not finish in time.
pub async fn run_bounded(command: &mut Command, timeout: Duration) -> Result<std::process::Output> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let child = command.spawn()?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(output) => Ok(output?),
        // Dropping the wait future drops the child, which kills it.
        Err(_) => bail!("timed out after {}s", timeout.as_secs_f32()),
    }
}

async fn systemctl(arguments: &[&str]) -> Result<std::process::Output> {
    run_bounded(Command::new("systemctl").args(arguments), SYSTEMCTL_TIMEOUT).await
}

#[must_use]
pub fn unit_name(instance_id: &str) -> String {
    format!("{UNIT_PREFIX}{instance_id}")
}

/// The instance id of a `cowboy-browser-<id>[.service]` unit name.
#[must_use]
pub fn instance_of_unit(unit: &str) -> Option<&str> {
    let name = unit.strip_suffix(".service").unwrap_or(unit);
    name.strip_prefix(UNIT_PREFIX).filter(|id| {
        !id.is_empty()
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    })
}

/// `RuntimeDirectory=` value of the unit.
#[must_use]
pub fn runtime_directory_property(instance_id: &str) -> String {
    format!("{RUNTIME_DIRECTORY_PARENT}/{instance_id}")
}

/// The unit's runtime directory on the host.
#[must_use]
pub fn runtime_directory(instance_id: &str) -> PathBuf {
    user_runtime_root()
        .join(RUNTIME_DIRECTORY_PARENT)
        .join(instance_id)
}

/// `$XDG_RUNTIME_DIR`, or systemd's default for this uid.
#[must_use]
pub fn user_runtime_root() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(format!("/run/user/{}", rustix::process::getuid().as_raw()))
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub memory_max: u64,
    pub memory_high: u64,
    pub cpu_quota_percent: u32,
    pub tasks_max: u32,
    pub max_runtime: Duration,
}

/// One unit start. `arguments` follow the launcher executable verbatim.
pub struct Launch<'a> {
    pub unit: &'a str,
    pub instance_id: &'a str,
    pub limits: Limits,
    pub launcher: &'a Path,
    pub arguments: Vec<OsString>,
}

/// The `systemd-run` argument vector for `launch`, without the program name.
#[must_use]
pub fn systemd_run_arguments(launch: &Launch<'_>) -> Vec<OsString> {
    let limits = launch.limits;
    let mut arguments: Vec<OsString> = [
        "--user".to_owned(),
        "--quiet".to_owned(),
        "--collect".to_owned(),
        "--service-type=exec".to_owned(),
        format!("--unit={}", launch.unit),
        format!("--slice={SLICE}"),
        "--property=Delegate=yes".to_owned(),
        "--property=KillMode=control-group".to_owned(),
        "--property=Restart=no".to_owned(),
        format!("--property=MemoryMax={}", limits.memory_max),
        format!("--property=MemoryHigh={}", limits.memory_high),
        "--property=MemorySwapMax=0".to_owned(),
        format!("--property=CPUQuota={}%", limits.cpu_quota_percent),
        format!("--property=TasksMax={}", limits.tasks_max),
        "--property=OOMScoreAdjust=1000".to_owned(),
        "--property=LimitCORE=0".to_owned(),
        format!("--property=RuntimeMaxSec={}", limits.max_runtime.as_secs()),
        format!("--property=TimeoutStopSec={}", STOP_TIMEOUT.as_secs()),
        format!(
            "--property=RuntimeDirectory={}",
            runtime_directory_property(launch.instance_id)
        ),
        "--property=RuntimeDirectoryMode=0700".to_owned(),
        "--property=RuntimeDirectoryPreserve=no".to_owned(),
        // A unit starts from the user manager's environment; the launcher
        // clears it for every child, and needs nothing from it itself.
        "--property=UnsetEnvironment=HOME USER LOGNAME SHELL PATH".to_owned(),
        "--".to_owned(),
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    arguments.push(launch.launcher.as_os_str().to_owned());
    arguments.extend(launch.arguments.iter().cloned());
    arguments
}

/// Start the unit. Returns once systemd has started it (no `--wait`).
///
/// # Errors
/// Returns when a unit of that name still exists or `systemd-run` fails.
pub async fn start(launch: &Launch<'_>) -> Result<()> {
    let existing = show(launch.unit).await?;
    ensure!(
        existing.load_state == "not-found",
        "browser unit {} already exists (LoadState={}, ActiveState={})",
        launch.unit,
        existing.load_state,
        existing.active_state
    );
    let output = run_bounded(
        Command::new("systemd-run").args(systemd_run_arguments(launch)),
        SYSTEMCTL_TIMEOUT,
    )
    .await
    .context("starting systemd-run for the browser unit")?;
    ensure!(
        output.status.success(),
        "systemd-run exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitStatus {
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub main_pid: u32,
    pub invocation_id: String,
    pub control_group: String,
    pub result: String,
    /// The unit's `MemoryMax` (its admitted budget); `None` when unlimited
    /// or not reported.
    pub memory_max: Option<u64>,
}

impl UnitStatus {
    /// Host path of the unit's cgroup, when systemd reports one.
    #[must_use]
    pub fn cgroup_dir(&self) -> Option<PathBuf> {
        let relative = self.control_group.strip_prefix('/')?;
        (!relative.is_empty()).then(|| Path::new(crate::cgroup::CGROUP_ROOT).join(relative))
    }

    #[must_use]
    pub fn gone(&self) -> bool {
        self.load_state == "not-found"
    }

    #[must_use]
    pub fn live(&self) -> bool {
        matches!(
            self.active_state.as_str(),
            "active" | "activating" | "reloading"
        )
    }
}

/// `systemctl --user show` of one unit. An unloaded unit reports
/// `LoadState=not-found`; a user bus failure is an error, never "gone".
///
/// # Errors
/// Returns when `systemctl` cannot be run or answers unexpectedly.
pub async fn show(unit: &str) -> Result<UnitStatus> {
    let output = systemctl(&[
        "--user",
        "show",
        unit,
        "--property=LoadState,ActiveState,SubState,MainPID,InvocationID,ControlGroup,Result,MemoryMax",
    ])
    .await
    .with_context(|| format!("querying browser unit {unit}"))?;
    ensure!(
        output.status.success(),
        "systemctl show {unit} exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    parse_show(&String::from_utf8_lossy(&output.stdout))
}

fn parse_show(text: &str) -> Result<UnitStatus> {
    let fields: BTreeMap<&str, &str> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    let field = |key: &str| fields.get(key).map(|value| value.trim().to_owned());
    Ok(UnitStatus {
        load_state: field("LoadState").context("systemctl show omitted LoadState")?,
        active_state: field("ActiveState").unwrap_or_default(),
        sub_state: field("SubState").unwrap_or_default(),
        main_pid: field("MainPID")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
        invocation_id: field("InvocationID").unwrap_or_default(),
        control_group: field("ControlGroup").unwrap_or_default(),
        result: field("Result").unwrap_or_default(),
        // `infinity` (unlimited) does not parse and reads as `None`.
        memory_max: field("MemoryMax").and_then(|value| value.parse().ok()),
    })
}

/// Wait until the unit is active with a main process, or fails.
///
/// # Errors
/// Returns when the unit fails, disappears or does not become active in time.
pub async fn wait_running(unit: &str, timeout: Duration) -> Result<UnitStatus> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let status = show(unit).await?;
        if status.active_state == "active"
            && status.main_pid > 0
            && !status.control_group.is_empty()
        {
            return Ok(status);
        }
        if status.gone()
            || matches!(
                status.active_state.as_str(),
                "failed" | "inactive" | "deactivating"
            )
        {
            bail!(
                "browser unit {unit} ended before readiness (ActiveState={}, Result={})",
                status.active_state,
                status.result
            );
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "browser unit {unit} did not become active in time"
        );
        tokio::time::sleep(POLL).await;
    }
}

/// `systemctl --user stop --no-block`: the stop job is enqueued and the
/// caller waits for collection itself, so no call blocks on the unit's own
/// stop timeout. A unit that is already gone is not an error.
///
/// # Errors
/// Returns when systemd refuses the stop for another reason.
pub async fn stop(unit: &str) -> Result<()> {
    let output = systemctl(&["--user", "stop", "--no-block", unit])
        .await
        .with_context(|| format!("stopping browser unit {unit}"))?;
    if output.status.success() || show(unit).await?.gone() {
        return Ok(());
    }
    bail!(
        "systemctl stop {unit} exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

/// `SIGKILL` the whole unit through systemd.
///
/// # Errors
/// Returns when systemd refuses the signal and the unit still exists.
pub async fn kill(unit: &str) -> Result<()> {
    let output = systemctl(&["--user", "kill", "--signal=SIGKILL", unit])
        .await
        .with_context(|| format!("killing browser unit {unit}"))?;
    if output.status.success() || show(unit).await?.gone() {
        return Ok(());
    }
    bail!(
        "systemctl kill {unit} exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

/// Wait until systemd has collected the unit (`LoadState=not-found`).
///
/// # Errors
/// Returns when `systemctl` fails; `Ok(false)` when the unit is still there
/// after `timeout`.
pub async fn wait_gone(unit: &str, timeout: Duration) -> Result<bool> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if show(unit).await?.gone() {
            return Ok(true);
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Stop the unit, escalating to `SIGKILL` when it outlives its own stop
/// timeout, and wait for collection.
///
/// # Errors
/// Returns when systemd cannot be driven; `Ok(false)` when the unit is still
/// present after escalation.
pub async fn stop_and_wait(unit: &str) -> Result<bool> {
    stop(unit).await?;
    if wait_gone(unit, STOP_TIMEOUT + Duration::from_secs(5)).await? {
        return Ok(true);
    }
    tracing::warn!(%unit, "browser unit outlived its stop timeout; escalating to SIGKILL");
    kill(unit).await?;
    wait_gone(unit, Duration::from_secs(10)).await
}

/// Whether `status` shows that the invocation a caller stopped is gone: the
/// unit was collected, or the name now belongs to another invocation (a
/// unit name is reused across starts; the old invocation must have ended for
/// a new one to start, and the new one must never be stopped in its place).
#[must_use]
pub fn invocation_gone(status: &UnitStatus, invocation_id: Option<&str>) -> bool {
    if status.gone() {
        return true;
    }
    invocation_id.is_some_and(|expected| {
        !expected.is_empty() && !status.invocation_id.is_empty() && status.invocation_id != expected
    })
}

/// Stop the unit and confirm that none of its processes remain: systemd's
/// stop with its own `SIGKILL` escalation first (bounded as a whole), then
/// `cgroup.kill` on the unit's cgroup and a wait for it to empty. `true`
/// only when the exit is verified (the unit was collected, its name belongs
/// to another invocation, or its cgroup is gone or unpopulated); a caller
/// that gets `false` must keep treating the instance as running.
///
/// `invocation_id`, when known, binds the termination to that invocation:
/// a unit of the same name with another invocation ID is never stopped or
/// killed, and counts as the original having exited.
pub async fn terminate(unit: &str, cgroup_dir: Option<&Path>, invocation_id: Option<&str>) -> bool {
    match show(unit).await {
        Ok(status) if invocation_gone(&status, invocation_id) => return true,
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, %unit, "browser unit unobservable before its stop"),
    }
    match tokio::time::timeout(SYSTEMD_PHASE_TIMEOUT, stop_and_wait(unit)).await {
        Ok(Ok(true)) => return true,
        Ok(Ok(false)) => tracing::error!(%unit, "browser unit is still present after SIGKILL"),
        Ok(Err(error)) => tracing::warn!(%error, %unit, "browser unit stop failed"),
        Err(_) => tracing::error!(%unit, "systemd did not stop the browser unit in time"),
    }
    // The retained cgroup path is preferred: it needs no answer from systemd.
    let observed = show(unit).await.ok();
    if observed
        .as_ref()
        .is_some_and(|status| invocation_gone(status, invocation_id) && !status.gone())
    {
        return true;
    }
    let cgroup = cgroup_dir
        .map(Path::to_path_buf)
        .or_else(|| observed.and_then(|status| status.cgroup_dir()));
    let Some(cgroup) = cgroup else {
        tracing::error!(%unit, "browser unit cgroup unknown; exit unconfirmed");
        return false;
    };
    if let Err(error) = kill_cgroup(&cgroup) {
        tracing::warn!(%error, %unit, "browser cgroup.kill failed");
    }
    match wait_cgroup_empty(&cgroup, CGROUP_KILL_WAIT).await {
        Ok(true) => true,
        Ok(false) => {
            tracing::error!(%unit, "browser cgroup still populated after cgroup.kill");
            false
        }
        Err(error) => {
            tracing::error!(%error, %unit, "browser cgroup unreadable; exit unconfirmed");
            false
        }
    }
}

/// How long `terminate` waits for a killed cgroup to empty.
const CGROUP_KILL_WAIT: Duration = Duration::from_secs(10);

/// `SIGKILL` every process of the cgroup subtree through `cgroup.kill`. A
/// missing cgroup has nothing left to kill.
///
/// # Errors
/// Returns when the kill file cannot be written.
pub fn kill_cgroup(dir: &Path) -> Result<()> {
    match std::fs::write(dir.join("cgroup.kill"), "1") {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("writing {}", dir.join("cgroup.kill").display()))
        }
    }
}

/// Whether the cgroup holds no process any more: it was removed, or
/// `cgroup.events` reports `populated 0`.
///
/// # Errors
/// Returns when the cgroup exists but cannot be read.
pub fn cgroup_empty(dir: &Path) -> Result<bool> {
    match std::fs::read_to_string(dir.join("cgroup.events")) {
        Ok(events) => Ok(events.lines().any(|line| line.trim() == "populated 0")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error).with_context(|| format!("reading {}", dir.display())),
    }
}

/// Wait until [`cgroup_empty`] holds.
///
/// # Errors
/// Returns when the cgroup cannot be read; `Ok(false)` on timeout.
pub async fn wait_cgroup_empty(dir: &Path, timeout: Duration) -> Result<bool> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if cgroup_empty(dir)? {
            return Ok(true);
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Every loaded `cowboy-browser-*` unit of the user manager.
///
/// # Errors
/// Returns when `systemctl` cannot list units.
pub async fn list_units() -> Result<Vec<String>> {
    let pattern = format!("{UNIT_PREFIX}*.service");
    let output = systemctl(&[
        "--user",
        "list-units",
        "--all",
        "--plain",
        "--no-legend",
        "--no-pager",
        &pattern,
    ])
    .await
    .context("listing browser units")?;
    ensure!(
        output.status.success(),
        "systemctl list-units exited {}",
        output.status
    );
    Ok(parse_unit_list(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_unit_list(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| instance_of_unit(name).is_some())
        .map(|name| name.trim_end_matches(".service").to_owned())
        .collect()
}

/// Every process of the cgroup subtree, with the sub-cgroup it belongs to
/// relative to `dir` (empty for the root).
///
/// # Errors
/// Returns when the tree cannot be read.
pub fn procs_recursive(dir: &Path) -> Result<Vec<(PathBuf, u32)>> {
    let mut pids = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let path = dir.join(&relative);
        let procs = std::fs::read_to_string(path.join("cgroup.procs"))
            .with_context(|| format!("reading {}", path.join("cgroup.procs").display()))?;
        for pid in procs.split_whitespace() {
            pids.push((relative.clone(), pid.parse().context("invalid cgroup pid")?));
        }
        for entry in std::fs::read_dir(&path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(relative.join(entry.file_name()));
            }
        }
    }
    pids.sort();
    Ok(pids)
}

/// # Errors
/// Returns when the cgroup cannot be read.
pub fn populated(dir: &Path) -> Result<bool> {
    let events = std::fs::read_to_string(dir.join("cgroup.events"))?;
    Ok(events.lines().any(|line| line.trim() == "populated 1"))
}

/// Freeze (`true`) or thaw (`false`) a cgroup. Thawing a missing cgroup is
/// not an error: the unit already ended.
///
/// # Errors
/// Returns when the freezer file cannot be written.
pub fn set_frozen(dir: &Path, frozen: bool) -> Result<()> {
    match std::fs::write(dir.join("cgroup.freeze"), if frozen { "1" } else { "0" }) {
        Ok(()) => Ok(()),
        Err(error) if !frozen && error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("writing {}", dir.join("cgroup.freeze").display()))
        }
    }
}

/// # Errors
/// Returns when the cgroup cannot be read.
pub fn is_frozen(dir: &Path) -> Result<bool> {
    let events = std::fs::read_to_string(dir.join("cgroup.events"))?;
    Ok(events.lines().any(|line| line.trim() == "frozen 1"))
}

/// Wait until `cgroup.events` reports `frozen 1`.
///
/// # Errors
/// Returns when the cgroup cannot be read; `Ok(false)` on timeout.
pub async fn wait_frozen(dir: &Path, timeout: Duration) -> Result<bool> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if is_frozen(dir)? {
            return Ok(true);
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgroup_exit_is_confirmed_only_by_an_empty_or_removed_cgroup() {
        let temp = tempfile::tempdir().unwrap();
        let cgroup = temp.path().join("cowboy-browser-a.service");
        std::fs::create_dir(&cgroup).unwrap();
        std::fs::write(cgroup.join("cgroup.events"), "populated 1\nfrozen 0\n").unwrap();
        assert!(!cgroup_empty(&cgroup).unwrap());
        kill_cgroup(&cgroup).unwrap();
        assert_eq!(
            std::fs::read_to_string(cgroup.join("cgroup.kill")).unwrap(),
            "1"
        );
        std::fs::write(cgroup.join("cgroup.events"), "populated 0\nfrozen 0\n").unwrap();
        assert!(cgroup_empty(&cgroup).unwrap());
        let removed = temp.path().join("gone.service");
        assert!(cgroup_empty(&removed).unwrap());
        kill_cgroup(&removed).unwrap();
    }

    #[tokio::test]
    async fn a_stalled_subprocess_is_killed_at_its_bound() {
        let started = std::time::Instant::now();
        let mut command = Command::new("sleep");
        command.arg("30");
        let error = run_bounded(&mut command, Duration::from_millis(200))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5));
        let mut command = Command::new("sh");
        command.args(["-c", "echo ok"]);
        let output = run_bounded(&mut command, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ok");
    }

    #[test]
    fn termination_is_bound_to_the_original_invocation() {
        let status = |load: &str, invocation: &str| UnitStatus {
            load_state: load.into(),
            active_state: "active".into(),
            invocation_id: invocation.into(),
            ..UnitStatus::default()
        };
        // Collected: gone whatever the invocation.
        assert!(invocation_gone(&status("not-found", ""), Some("a")));
        // The same invocation still runs.
        assert!(!invocation_gone(&status("loaded", "a"), Some("a")));
        // A replacement reuses the name: the original is gone and the
        // replacement must not be stopped in its place.
        assert!(invocation_gone(&status("loaded", "b"), Some("a")));
        // Without a known invocation (or an observed one) nothing is assumed.
        assert!(!invocation_gone(&status("loaded", "b"), None));
        assert!(!invocation_gone(&status("loaded", ""), Some("a")));
    }

    #[test]
    fn unit_names_round_trip_and_reject_foreign_units() {
        assert_eq!(unit_name("inst-1"), "cowboy-browser-inst-1");
        assert_eq!(
            instance_of_unit("cowboy-browser-inst-1.service"),
            Some("inst-1")
        );
        assert_eq!(instance_of_unit("cowboy-browser-inst_1"), Some("inst_1"));
        assert_eq!(instance_of_unit("cowboy-browser-"), None);
        assert_eq!(instance_of_unit("cowboy-worker-abc.service"), None);
        assert_eq!(instance_of_unit("cowboy-browser-a/b"), None);
        assert_eq!(
            parse_unit_list(
                "cowboy-browser-a.service loaded active running\nother.service loaded\n"
            ),
            vec!["cowboy-browser-a"]
        );
    }

    #[test]
    fn show_output_parses_and_a_missing_unit_is_gone() {
        let status = parse_show(
            "LoadState=loaded\nActiveState=active\nSubState=running\nMainPID=4242\nInvocationID=abc\nControlGroup=/user.slice/x/cowboy-browser-a.service\nResult=success\nMemoryMax=1363148800\n",
        )
        .unwrap();
        assert_eq!(status.main_pid, 4242);
        assert_eq!(status.memory_max, Some(1300 << 20));
        assert!(status.live());
        assert_eq!(
            status.cgroup_dir().unwrap(),
            Path::new("/sys/fs/cgroup/user.slice/x/cowboy-browser-a.service")
        );
        let gone = parse_show("LoadState=not-found\nActiveState=inactive\nMainPID=0\n").unwrap();
        assert!(gone.gone());
        assert!(gone.cgroup_dir().is_none());
        assert_eq!(gone.memory_max, None);
        let unlimited = parse_show("LoadState=loaded\nMemoryMax=infinity\n").unwrap();
        assert_eq!(unlimited.memory_max, None);
        assert!(parse_show("ActiveState=active\n").is_err());
    }

    #[test]
    fn systemd_run_arguments_carry_every_limit_and_the_runtime_directory() {
        let launch = Launch {
            unit: "cowboy-browser-inst-1",
            instance_id: "inst-1",
            limits: Limits {
                memory_max: 2 << 30,
                memory_high: 1536 << 20,
                cpu_quota_percent: 200,
                tasks_max: 512,
                max_runtime: Duration::from_secs(1800),
            },
            launcher: Path::new("/state/plugins/x/usr/libexec/cowboy-browser-launcher"),
            arguments: vec!["run".into(), "--display".into(), ":7".into()],
        };
        let arguments: Vec<String> = systemd_run_arguments(&launch)
            .into_iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        for expected in [
            "--property=MemoryMax=2147483648",
            "--property=MemorySwapMax=0",
            "--property=CPUQuota=200%",
            "--property=TasksMax=512",
            "--property=RuntimeMaxSec=1800",
            "--property=RuntimeDirectory=cowboy-browser/inst-1",
            "--property=RuntimeDirectoryMode=0700",
            "--property=Delegate=yes",
            "--property=OOMScoreAdjust=1000",
            "--slice=cowboy-agents.slice",
        ] {
            assert!(
                arguments.iter().any(|argument| argument == expected),
                "{expected}"
            );
        }
        let separator = arguments
            .iter()
            .position(|argument| argument == "--")
            .unwrap();
        assert_eq!(
            &arguments[separator + 1..],
            [
                "/state/plugins/x/usr/libexec/cowboy-browser-launcher",
                "run",
                "--display",
                ":7"
            ]
        );
        assert_eq!(
            runtime_directory_property("inst-1"),
            "cowboy-browser/inst-1"
        );
        assert!(runtime_directory("inst-1").ends_with("cowboy-browser/inst-1"));
    }
}
