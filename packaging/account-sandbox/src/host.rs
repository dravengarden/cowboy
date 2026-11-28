//! The host operations the daemon delegates to system tools: user management,
//! home images, mounts and systemd. A trait so the state machine is tested
//! unprivileged against a fake (`crate::fake`).

use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
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
    /// The unit's cgroup still holds a process (or could not be shown
    /// empty): a stop that timed out can leave a `failed` unit with a
    /// surviving descendant.
    pub cgroup_populated: bool,
}

impl UnitStatus {
    /// Whether processes of the unit may still exist or are about to.
    /// `inactive` or `failed` alone is not proof: a queued job may still
    /// start it, and its cgroup may still hold a process.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.job_pending
            || self.cgroup_populated
            || !matches!(self.active_state.as_str(), "inactive" | "failed")
    }
}

/// Whether the cgroup at `control_group` (systemd's `ControlGroup=`, relative
/// to the unified hierarchy) holds a process. An empty path or a removed
/// cgroup is empty; any other read failure counts as populated (fail closed).
fn cgroup_populated(control_group: &str) -> bool {
    if control_group.is_empty() {
        return false;
    }
    let path = PathBuf::from("/sys/fs/cgroup")
        .join(control_group.trim_start_matches('/'))
        .join("cgroup.events");
    match std::fs::read_to_string(&path) {
        Ok(events) => !events.lines().any(|line| line == "populated 0"),
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

/// Size and free space of a mounted filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsUsage {
    pub size_bytes: u64,
    /// Space an unprivileged writer can still use.
    pub free_bytes: u64,
    pub inodes: u64,
    pub free_inodes: u64,
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
    /// Size and free space of the filesystem `dir` is on.
    fn filesystem_usage(&self, dir: BorrowedFd<'_>) -> Result<FsUsage>;
    /// Verify that `home` is the root of the ext4 filesystem on the loop
    /// device whose backing file is `image` (same inode), then give the
    /// device the image's length and grow the mounted filesystem online to
    /// the whole device. Whether the filesystem was resized. Both objects
    /// stay pinned by their descriptors throughout, and nothing shrinks.
    fn reconcile_home(&self, image: &std::fs::File, home: BorrowedFd<'_>) -> Result<bool>;
}

/// The loop block device major number.
const LOOP_MAJOR: u32 = 7;
/// The root directory inode of an ext4 filesystem.
const EXT4_ROOT_INODE: u64 = 2;

/// Block count and block size of the ext4 superblock read from `device`
/// (through the page cache the mounted filesystem writes its superblock to).
fn ext4_geometry(device: BorrowedFd<'_>) -> Result<(u64, u64)> {
    let mut superblock = [0_u8; 1024];
    let read = rustix::io::pread(device, &mut superblock, 1024).context("read the superblock")?;
    ensure!(read == superblock.len(), "short superblock read");
    let u32_at = |offset: usize| {
        u32::from_le_bytes(
            superblock[offset..offset + 4]
                .try_into()
                .expect("four bytes"),
        )
    };
    ensure!(
        u16::from_le_bytes([superblock[0x38], superblock[0x39]]) == 0xEF53,
        "not an ext4 filesystem"
    );
    let log_block_size = u32_at(0x18);
    ensure!(log_block_size <= 6, "unsupported ext4 block size");
    let mut blocks = u64::from(u32_at(0x04));
    // INCOMPAT_64BIT: the block count's high half is in use.
    if u32_at(0x60) & 0x80 != 0 {
        blocks |= u64::from(u32_at(0x150)) << 32;
    }
    Ok((blocks, 1024_u64 << log_block_size))
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

/// A `systemd-run` client and a pidfd opened before any waiter could reap
/// it, so cancelling can never signal a recycled PID.
struct SystemdRun {
    child: std::process::Child,
    pidfd: Option<std::sync::Arc<OwnedFd>>,
}

impl UnitChild for SystemdRun {
    fn wait(&mut self) -> Result<Option<i32>> {
        Ok(self.child.wait()?.code())
    }

    fn canceller(&self) -> Box<dyn Fn() + Send + Sync> {
        let pidfd = self.pidfd.clone();
        Box::new(move || {
            // Without a pidfd the client is never signalled: the start stays
            // unconfirmed (fail closed) rather than risk another process.
            if let Some(pidfd) = &pidfd {
                let _ = rustix::process::pidfd_send_signal(pidfd, rustix::process::Signal::KILL);
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
        // Not yet waited for, so the pid still names this child.
        let pidfd = i32::try_from(child.id())
            .ok()
            .and_then(rustix::process::Pid::from_raw)
            .and_then(|pid| {
                rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()).ok()
            })
            .map(std::sync::Arc::new);
        Ok(Box::new(SystemdRun { child, pidfd }))
    }

    fn unit_status(&self, unit: &str) -> Result<Option<UnitStatus>> {
        let text = run_ok(
            &self.tools.systemctl,
            &[
                "show",
                "--property=LoadState,ActiveState,InvocationID,Job,ControlGroup",
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
            cgroup_populated: cgroup_populated(&field("ControlGroup")),
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

    fn filesystem_usage(&self, dir: BorrowedFd<'_>) -> Result<FsUsage> {
        let stat = rustix::fs::fstatvfs(dir).context("fstatvfs")?;
        Ok(FsUsage {
            size_bytes: stat.f_blocks.saturating_mul(stat.f_frsize),
            free_bytes: stat.f_bavail.saturating_mul(stat.f_frsize),
            inodes: stat.f_files,
            free_inodes: stat.f_favail,
        })
    }

    fn reconcile_home(&self, image: &std::fs::File, home: BorrowedFd<'_>) -> Result<bool> {
        let home_stat = rustix::fs::fstat(home).context("stat home")?;
        let image_stat = rustix::fs::fstat(image).context("stat home image")?;
        ensure!(
            home_stat.st_ino == EXT4_ROOT_INODE,
            "home is not the root of its filesystem"
        );
        let device = home_stat.st_dev;
        let (major, minor) = (rustix::fs::major(device), rustix::fs::minor(device));
        ensure!(major == LOOP_MAJOR, "home is not on a loop device");
        let path = loop_device_node(major, minor)?;
        let loop_device = rustix::fs::open(
            path.as_str(),
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW,
            rustix::fs::Mode::empty(),
        )
        .with_context(|| format!("open {path}"))?;
        let device_stat = rustix::fs::fstat(&loop_device)?;
        ensure!(
            rustix::fs::FileType::from_raw_mode(device_stat.st_mode)
                == rustix::fs::FileType::BlockDevice
                && device_stat.st_rdev == device,
            "{path} is not the device home is mounted from"
        );
        let status = crate::raw::loop_status(loop_device.as_fd())
            .with_context(|| format!("loop status of {path}"))?;
        ensure!(
            status.lo_inode == image_stat.st_ino && status.lo_device == image_stat.st_dev,
            "{path} is not backed by this home image"
        );
        ensure!(
            status.lo_offset == 0 && status.lo_sizelimit == 0,
            "{path} maps only part of its backing file"
        );
        let (blocks, block_size) = ext4_geometry(loop_device.as_fd())?;
        let filesystem_bytes = blocks.saturating_mul(block_size);
        let image_bytes = u64::try_from(image_stat.st_size).unwrap_or(0);
        // Never let the device shrink under the filesystem: refresh only
        // toward a longer image.
        ensure!(
            image_bytes >= filesystem_bytes,
            "the home image ({image_bytes} bytes) is shorter than its filesystem ({filesystem_bytes})"
        );
        if image_bytes > crate::raw::block_device_bytes(loop_device.as_fd())? {
            crate::raw::loop_set_capacity(loop_device.as_fd())
                .with_context(|| format!("refresh the capacity of {path}"))?;
        }
        let device_bytes = crate::raw::block_device_bytes(loop_device.as_fd())?;
        ensure!(
            device_bytes >= filesystem_bytes,
            "{path} ({device_bytes} bytes) is smaller than its filesystem ({filesystem_bytes})"
        );
        // A device left longer than its image (an extension rolled back)
        // must never carry the filesystem past the image.
        ensure!(
            device_bytes <= image_bytes,
            "{path} ({device_bytes} bytes) is longer than its image ({image_bytes})"
        );
        let device_blocks = device_bytes / block_size;
        if device_blocks == blocks {
            return Ok(false);
        }
        crate::raw::ext4_resize(home, device_blocks)
            .with_context(|| format!("grow the filesystem on {path} to {device_blocks} blocks"))?;
        Ok(true)
    }
}

/// The device node of block device `major:minor`, named by the kernel in
/// sysfs (a loop device's minor is not its index when partitions are
/// enabled); the caller verifies the node's device number after opening.
fn loop_device_node(major: u32, minor: u32) -> Result<String> {
    let uevent = std::fs::read_to_string(format!("/sys/dev/block/{major}:{minor}/uevent"))
        .with_context(|| format!("read the uevent of block device {major}:{minor}"))?;
    let name = uevent
        .lines()
        .find_map(|line| line.strip_prefix("DEVNAME="))
        .with_context(|| format!("block device {major}:{minor} has no name"))?;
    ensure!(
        name.starts_with("loop") && name.bytes().all(|b| b.is_ascii_alphanumeric()),
        "block device {major}:{minor} is named {name:?}, not a loop device"
    );
    Ok(format!("/dev/{name}"))
}

/// Whether `/var/run` is a directory of its own rather than a symlink.
#[must_use]
pub fn var_run_is_directory() -> bool {
    std::fs::symlink_metadata(PathBuf::from("/var/run")).is_ok_and(|metadata| metadata.is_dir())
}

#[cfg(test)]
mod tests {
    /// A mounted loop image grows online through the pinned descriptors.
    /// Needs root, loop devices, `mkfs.ext4` and `mount`/`umount` on `PATH`:
    /// `sudo <test binary> --ignored a_mounted_loop_image_grows_online`.
    #[test]
    #[ignore = "needs root, loop devices and e2fsprogs"]
    fn a_mounted_loop_image_grows_online() {
        use super::Host as _;
        use std::os::fd::AsFd as _;
        let which = |name: &str| {
            std::env::split_paths(&std::env::var_os("PATH").unwrap())
                .map(|dir| dir.join(name))
                .find(|path| path.is_file())
                .unwrap_or_else(|| panic!("{name} is not on PATH"))
        };
        let host = super::SystemHost {
            tools: crate::config::Tools {
                mkfs_ext4: which("mkfs.ext4"),
                mount: which("mount"),
                umount: which("umount"),
                ..crate::config::Tools::default()
            },
        };
        let temp = tempfile::tempdir().unwrap();
        let (image, home) = (temp.path().join("home.img"), temp.path().join("home"));
        std::fs::create_dir(&home).unwrap();
        let file = std::fs::File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&image)
            .unwrap();
        file.set_len(64 << 20).unwrap();
        host.format_image(&image, 4096, (0, 0)).unwrap();
        let other = temp.path().join("other.img");
        std::fs::write(&other, b"").unwrap();
        host.mount_image(&image, &home).unwrap();
        let result = std::panic::catch_unwind(|| {
            let root = crate::safefs::open_root(&home).unwrap();
            let before = host.filesystem_usage(root.as_fd()).unwrap();
            // Nothing to do at the current size.
            assert!(!host.reconcile_home(&file, root.as_fd()).unwrap());
            std::fs::write(home.join("data"), vec![7_u8; 40 << 20]).unwrap();
            rustix::fs::fallocate(
                &file,
                rustix::fs::FallocateFlags::empty(),
                (256 << 20) - 4096,
                4096,
            )
            .unwrap();
            // Another image's file is refused before anything changes.
            let other = std::fs::File::open(&other).unwrap();
            assert!(host.reconcile_home(&other, root.as_fd()).is_err());
            assert!(host.reconcile_home(&file, root.as_fd()).unwrap());
            let after = host.filesystem_usage(root.as_fd()).unwrap();
            assert!(
                after.size_bytes > before.size_bytes * 3,
                "{before:?} -> {after:?}"
            );
            assert!(after.inodes > before.inodes * 3, "{before:?} -> {after:?}");
            assert_eq!(std::fs::read(home.join("data")).unwrap().len(), 40 << 20);
            assert!(!host.reconcile_home(&file, root.as_fd()).unwrap());
            // A directory that is not a mount root is refused.
            std::fs::create_dir(home.join("sub")).unwrap();
            let sub = crate::safefs::open_root(&home.join("sub")).unwrap();
            assert!(host.reconcile_home(&file, sub.as_fd()).is_err());
        });
        host.unmount(&home).unwrap();
        result.unwrap();
    }

    #[test]
    fn a_missing_or_unset_cgroup_is_empty() {
        assert!(!super::cgroup_populated(""));
        assert!(!super::cgroup_populated(
            "/system.slice/cga-absent-0000.service"
        ));
    }
}
