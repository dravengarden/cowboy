//! The host operations the daemon delegates to system tools: user management,
//! home images, mounts and systemd. A trait so the state machine is tested
//! unprivileged against a fake (`crate::fake`).

use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail, ensure};

use crate::config::Tools;

/// A unit's systemd state, `None` from [`Host::unit_status`] when unloaded
/// (`LoadState=not-found`), the only state that proves no invocation of it
/// exists or can start without a new submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitStatus {
    pub active_state: String,
    pub invocation_id: Option<String>,
    /// systemd holds a queued job for the unit (`Job=` is set): an inactive
    /// unit with a pending start job is about to run.
    pub job_pending: bool,
}

impl UnitStatus {
    /// Whether processes of the unit may still exist or are about to.
    /// `inactive` or `failed` alone is not proof: a queued job may still
    /// start it.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.job_pending || !matches!(self.active_state.as_str(), "inactive" | "failed")
    }
}

/// The `systemd-run --wait` process of one unit.
pub trait UnitChild: Send {
    /// Wait for the unit to finish; its exit status, `None` when killed.
    fn wait(&mut self) -> Result<Option<i32>>;
    /// End the `systemd-run` client itself (not the unit): once it is gone
    /// it can no longer submit the unit, so a later status is final.
    fn canceller(&self) -> Box<dyn Fn() + Send + Sync>;
}

/// The unit-side ends of the daemon-created pipes.
pub struct UnitStdio {
    pub stdin: OwnedFd,
    pub stdout: OwnedFd,
    pub stderr: OwnedFd,
}

pub trait Host: Send + Sync {
    /// Create the static system user (and its group) when absent; its ids.
    fn ensure_user(&self, user: &str) -> Result<(u32, u32)>;
    fn remove_user(&self, user: &str) -> Result<()>;
    /// `mkfs.ext4` the already sized sparse `image` with a fixed inode count
    /// and its root directory owned by `owner`.
    fn format_image(&self, image: &Path, inodes: u64, owner: (u32, u32)) -> Result<()>;
    fn mount_image(&self, image: &Path, target: &Path) -> Result<()>;
    fn unmount(&self, target: &Path) -> Result<()>;
    fn is_mounted(&self, target: &Path) -> Result<bool>;
    fn spawn_unit(&self, argv: &[String], stdio: UnitStdio) -> Result<Box<dyn UnitChild>>;
    fn unit_status(&self, unit: &str) -> Result<Option<UnitStatus>>;
    fn stop_unit(&self, unit: &str) -> Result<()>;
    /// Loaded units whose names match the glob `pattern`.
    fn list_units(&self, pattern: &str) -> Result<Vec<String>>;
    fn boot_id(&self) -> Result<String>;
    fn free_bytes(&self, path: &Path) -> Result<u64>;
}

/// The production host: system tools at configured absolute paths.
pub struct SystemHost {
    pub tools: Tools,
}

fn run(program: &Path, args: &[&str]) -> Result<std::process::Output> {
    let output = Command::new(program)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LANG", "C")
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("run {}", program.display()))?;
    Ok(output)
}

fn run_ok(program: &Path, args: &[&str]) -> Result<String> {
    let output = run(program, args)?;
    if !output.status.success() {
        bail!(
            "{} {} failed ({}): {}",
            program.display(),
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn path_arg(path: &Path) -> Result<&str> {
    path.to_str().context("non-UTF-8 path")
}

/// Look `name` up in a `passwd`/`group` style file: its third field.
fn lookup_id(file: &str, name: &str) -> Result<Option<u32>> {
    let text = std::fs::read_to_string(file).with_context(|| format!("read {file}"))?;
    for line in text.lines() {
        let mut fields = line.split(':');
        if fields.next() == Some(name) {
            let id = fields.nth(1).context("malformed entry")?;
            return Ok(Some(id.parse().context("malformed id")?));
        }
    }
    Ok(None)
}

struct SystemdRun(std::process::Child);

impl UnitChild for SystemdRun {
    fn wait(&mut self) -> Result<Option<i32>> {
        Ok(self.0.wait()?.code())
    }

    fn canceller(&self) -> Box<dyn Fn() + Send + Sync> {
        let pid = i32::try_from(self.0.id())
            .ok()
            .and_then(rustix::process::Pid::from_raw);
        Box::new(move || {
            if let Some(pid) = pid {
                // The client has not been reaped (its waiter owns it), so
                // the pid still names it.
                let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
            }
        })
    }
}

impl Host for SystemHost {
    fn ensure_user(&self, user: &str) -> Result<(u32, u32)> {
        if lookup_id("/etc/passwd", user)?.is_none() {
            run_ok(
                &self.tools.useradd,
                &[
                    "--system",
                    "--no-create-home",
                    "--home-dir",
                    "/nonexistent",
                    "--shell",
                    "/usr/sbin/nologin",
                    "--user-group",
                    user,
                ],
            )?;
        }
        let uid = lookup_id("/etc/passwd", user)?.context("user missing after useradd")?;
        let gid = lookup_id("/etc/group", user)?.context("user group missing")?;
        ensure!(uid != 0 && gid != 0, "account user resolves to root");
        Ok((uid, gid))
    }

    fn remove_user(&self, user: &str) -> Result<()> {
        if lookup_id("/etc/passwd", user)?.is_some() {
            run_ok(&self.tools.userdel, &[user])?;
        }
        Ok(())
    }

    fn format_image(&self, image: &Path, inodes: u64, (uid, gid): (u32, u32)) -> Result<()> {
        let inodes = inodes.to_string();
        let root_owner = format!("root_owner={uid}:{gid}");
        run_ok(
            &self.tools.mkfs_ext4,
            &[
                "-q",
                "-F",
                "-N",
                &inodes,
                "-m",
                "0",
                "-E",
                &root_owner,
                path_arg(image)?,
            ],
        )?;
        Ok(())
    }

    fn mount_image(&self, image: &Path, target: &Path) -> Result<()> {
        run_ok(
            &self.tools.mount,
            &[
                "-t",
                "ext4",
                "-o",
                "loop,nosuid,nodev,noatime",
                "--",
                path_arg(image)?,
                path_arg(target)?,
            ],
        )?;
        Ok(())
    }

    fn unmount(&self, target: &Path) -> Result<()> {
        run_ok(&self.tools.umount, &["--", path_arg(target)?])?;
        Ok(())
    }

    fn is_mounted(&self, target: &Path) -> Result<bool> {
        let mountinfo = std::fs::read_to_string("/proc/self/mountinfo")?;
        let target = path_arg(target)?;
        Ok(mountinfo
            .lines()
            .any(|line| line.split(' ').nth(4) == Some(target)))
    }

    fn spawn_unit(&self, argv: &[String], stdio: UnitStdio) -> Result<Box<dyn UnitChild>> {
        let child = Command::new(&argv[0])
            .args(&argv[1..])
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .stdin(Stdio::from(stdio.stdin))
            .stdout(Stdio::from(stdio.stdout))
            .stderr(Stdio::from(stdio.stderr))
            .spawn()
            .context("spawn systemd-run")?;
        Ok(Box::new(SystemdRun(child)))
    }

    fn unit_status(&self, unit: &str) -> Result<Option<UnitStatus>> {
        let text = run_ok(
            &self.tools.systemctl,
            &[
                "show",
                "--property=LoadState,ActiveState,InvocationID,Job",
                "--",
                unit,
            ],
        )?;
        let field = |name: &str| {
            text.lines()
                .find_map(|line| {
                    line.strip_prefix(name)
                        .and_then(|rest| rest.strip_prefix('='))
                })
                .unwrap_or_default()
                .to_owned()
        };
        if field("LoadState") == "not-found" {
            return Ok(None);
        }
        let invocation = field("InvocationID");
        let job = field("Job");
        Ok(Some(UnitStatus {
            active_state: field("ActiveState"),
            invocation_id: (!invocation.is_empty()).then_some(invocation),
            job_pending: !job.is_empty() && job != "0",
        }))
    }

    fn stop_unit(&self, unit: &str) -> Result<()> {
        run_ok(&self.tools.systemctl, &["stop", "--", unit])?;
        Ok(())
    }

    fn list_units(&self, pattern: &str) -> Result<Vec<String>> {
        let text = run_ok(
            &self.tools.systemctl,
            &[
                "list-units",
                "--all",
                "--plain",
                "--no-legend",
                "--no-pager",
                "--",
                pattern,
            ],
        )?;
        Ok(text
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .map(str::to_owned)
            .collect())
    }

    fn boot_id(&self) -> Result<String> {
        Ok(std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
            .trim()
            .to_owned())
    }

    fn free_bytes(&self, path: &Path) -> Result<u64> {
        let stat =
            rustix::fs::statvfs(path).with_context(|| format!("statvfs {}", path.display()))?;
        Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
    }
}

/// Whether `/var/run` is a directory of its own rather than a symlink.
#[must_use]
pub fn var_run_is_directory() -> bool {
    std::fs::symlink_metadata(PathBuf::from("/var/run")).is_ok_and(|metadata| metadata.is_dir())
}
