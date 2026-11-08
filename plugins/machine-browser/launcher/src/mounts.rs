//! Step 2 of the launcher contract: the instance's private mount table.
//! Every mount happens inside the namespace init while it still holds its
//! user-namespace capabilities; nothing here is reachable after the children
//! are sealed. Sources that must survive masking are opened as descriptors
//! first and bound through `open_tree`/`move_mount`.

use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use rustix::fs::{CWD, Mode, OFlags};
use rustix::mount::{
    MountFlags, MountPropagationFlags, MoveMountFlags, OpenTreeFlags, UnmountFlags, mount,
    mount_bind, mount_change, mount_remount, move_mount, open_tree, unmount,
};

use crate::procinfo::{self, MountInfo};

/// Files of the host's `/etc` the scaffold exposes; each is bound only when
/// the host has it.
pub const HOST_ETC_FILES: &[&str] = &["hosts", "localtime", "passwd", "group", "machine-id"];

/// Host facts gathered before anything is masked.
pub struct HostFacts {
    pub etc_files: Vec<(&'static str, OwnedFd)>,
    pub ssl: Option<OwnedFd>,
    pub resolv_conf: String,
    pub timezone: String,
}

/// Open the host `/etc` entries the scaffold binds and read the rest.
pub fn open_host_facts() -> HostFacts {
    let open_path = |path: &str| -> Option<OwnedFd> {
        rustix::fs::open(path, OFlags::PATH | OFlags::CLOEXEC, Mode::empty()).ok()
    };
    let mut etc_files = Vec::new();
    for name in HOST_ETC_FILES {
        let path = format!("/etc/{name}");
        if std::fs::metadata(&path).is_ok_and(|metadata| metadata.is_file())
            && let Some(fd) = open_path(&path)
        {
            etc_files.push((*name, fd));
        }
    }
    let ssl = std::fs::metadata("/etc/ssl")
        .ok()
        .filter(std::fs::Metadata::is_dir)
        .and_then(|_| open_path("/etc/ssl"));
    let resolv_conf =
        procinfo::resolv_conf(&std::fs::read_to_string("/etc/resolv.conf").unwrap_or_default());
    let localtime_link = std::fs::read_link("/etc/localtime").ok();
    let timezone = procinfo::timezone(
        std::env::var("TZ").ok().as_deref(),
        localtime_link.as_deref(),
        std::fs::read_to_string("/etc/timezone").ok().as_deref(),
        std::fs::metadata("/etc/localtime").is_ok(),
    );
    HostFacts {
        etc_files,
        ssl,
        resolv_conf,
        timezone,
    }
}

/// Make every mount in the namespace private so nothing propagates back.
pub fn make_private() -> Result<()> {
    mount_change(
        "/",
        MountPropagationFlags::PRIVATE | MountPropagationFlags::REC,
    )
    .context("making / recursively private")
}

/// A fresh tmpfs of `bytes` at `target`.
pub fn tmpfs(target: &str, bytes: u64, mode: u32, extra: MountFlags) -> Result<()> {
    let data =
        std::ffi::CString::new(format!("size={bytes},mode=0{mode:o}")).context("tmpfs options")?;
    mount(
        "tmpfs",
        target,
        "tmpfs",
        MountFlags::NOSUID | MountFlags::NODEV | extra,
        Some(data.as_c_str()),
    )
    .with_context(|| format!("mounting tmpfs on {target}"))
}

/// An empty, read-only tmpfs over `target`, hiding whatever was there. A
/// missing target has nothing to hide and is skipped (`Ok(false)`).
pub fn mask(target: &str) -> Result<bool> {
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => anyhow::bail!("mask target {target} is not a directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).with_context(|| format!("inspecting {target}")),
    }
    tmpfs(target, 4096, 0o755, MountFlags::RDONLY | MountFlags::NOEXEC)?;
    Ok(true)
}

/// Bind `source` at `target`; `readonly` adds `ro,nosuid,nodev` through the
/// remount a bind needs for its flags to take effect.
pub fn bind(source: &str, target: &str, readonly: bool) -> Result<()> {
    mount_bind(source, target).with_context(|| format!("binding {source} at {target}"))?;
    if readonly {
        remount_readonly(target)?;
    }
    Ok(())
}

/// Bind the object `fd` refers to at `target`, through a detached mount
/// (`open_tree` + `move_mount`), so the source needs no reachable pathname.
pub fn bind_fd(fd: BorrowedFd<'_>, target: &str, readonly: bool) -> Result<()> {
    let tree = open_tree(
        fd,
        "",
        OpenTreeFlags::OPEN_TREE_CLONE
            | OpenTreeFlags::OPEN_TREE_CLOEXEC
            | OpenTreeFlags::AT_EMPTY_PATH,
    )
    .with_context(|| format!("cloning the mount for {target}"))?;
    move_mount(
        tree.as_fd(),
        "",
        CWD,
        target,
        MoveMountFlags::MOVE_MOUNT_F_EMPTY_PATH,
    )
    .with_context(|| format!("attaching the mount at {target}"))?;
    drop(tree);
    if readonly {
        remount_readonly(target)?;
    }
    Ok(())
}

/// Remount the bind at `target` read-only, `nosuid`, `nodev`, keeping the
/// flags the kernel locks on an unprivileged remount (`noexec`, atime).
pub fn remount_readonly(target: &str) -> Result<()> {
    let flags = MountFlags::BIND
        | MountFlags::RDONLY
        | MountFlags::NOSUID
        | MountFlags::NODEV
        | locked_flags(target)?;
    mount_remount(target, flags, "").with_context(|| format!("remounting {target} read-only"))
}

/// Remount a filesystem the launcher mounted itself (a tmpfs) read-only.
pub fn remount_filesystem_readonly(target: &str) -> Result<()> {
    let flags = MountFlags::RDONLY | MountFlags::NOSUID | MountFlags::NODEV | locked_flags(target)?;
    mount_remount(target, flags, "").with_context(|| format!("remounting {target} read-only"))
}

/// `umount2(MNT_DETACH)`.
pub fn unmount_detached(target: &str) -> Result<()> {
    unmount(target, UnmountFlags::DETACH).with_context(|| format!("unmounting {target}"))
}

/// Mount a fresh `proc` for the new pid namespace.
pub fn proc() -> Result<()> {
    mount(
        "proc",
        "/proc",
        "proc",
        MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOEXEC,
        None,
    )
    .context("mounting proc")
}

/// The current mountinfo entry for `target` (the topmost mount there).
pub fn entry(target: &str) -> Result<MountInfo> {
    let table = std::fs::read_to_string("/proc/self/mountinfo").context("reading mountinfo")?;
    procinfo::parse_mountinfo(&table)
        .into_iter()
        .rfind(|entry| entry.target == target)
        .with_context(|| format!("{target} is not a mount point"))
}

fn locked_flags(target: &str) -> Result<MountFlags> {
    let entry = entry(target)?;
    let mut flags = MountFlags::empty();
    for option in entry.vfs_options.split(',') {
        flags |= match option {
            "nosuid" => MountFlags::NOSUID,
            "nodev" => MountFlags::NODEV,
            "noexec" => MountFlags::NOEXEC,
            "noatime" => MountFlags::NOATIME,
            "nodiratime" => MountFlags::NODIRATIME,
            "relatime" => MountFlags::RELATIME,
            "strictatime" => MountFlags::STRICTATIME,
            _ => MountFlags::empty(),
        };
    }
    Ok(flags)
}

/// Build the `/etc` scaffold: a private tmpfs holding bind mounts of the
/// host's `hosts`, `localtime`, `passwd`, `group`, `machine-id` and `ssl/`
/// where they exist, a generated `resolv.conf`, the artifact's own
/// `nsswitch.conf`, and the managed policy file bound at the browser's
/// policy directory. The tmpfs ends read-only.
pub fn build_etc(
    facts: &HostFacts,
    nsswitch_source: &str,
    policy_source: &str,
    policy_directory: &Path,
) -> Result<()> {
    let nsswitch = std::fs::read_to_string(nsswitch_source)
        .with_context(|| format!("reading {nsswitch_source}"))?;
    ensure!(
        nsswitch
            .lines()
            .any(|line| line.trim_start().starts_with("hosts:")),
        "{nsswitch_source} has no hosts database"
    );
    ensure!(
        std::fs::metadata(policy_source).is_ok_and(|metadata| metadata.is_file()),
        "policy file {policy_source} is missing from the artifact"
    );
    tmpfs("/etc", 1 << 20, 0o755, MountFlags::NOEXEC)?;
    for (name, fd) in &facts.etc_files {
        let target = format!("/etc/{name}");
        File::create(&target).with_context(|| format!("creating {target}"))?;
        bind_fd(fd.as_fd(), &target, true)?;
    }
    if let Some(ssl) = &facts.ssl {
        std::fs::create_dir("/etc/ssl").context("creating /etc/ssl")?;
        bind_fd(ssl.as_fd(), "/etc/ssl", true)?;
    }
    std::fs::write("/etc/resolv.conf", &facts.resolv_conf).context("writing /etc/resolv.conf")?;
    std::fs::write("/etc/nsswitch.conf", nsswitch).context("writing /etc/nsswitch.conf")?;
    let directory = policy_directory
        .to_str()
        .context("policy directory is not UTF-8")?;
    std::fs::create_dir_all(directory).with_context(|| format!("creating {directory}"))?;
    let policy_target = format!("{directory}/policy.json");
    File::create(&policy_target).with_context(|| format!("creating {policy_target}"))?;
    bind(policy_source, &policy_target, true)?;
    remount_filesystem_readonly("/etc")
}

/// Mount table entries for `targets`, as the init sees them.
pub fn report(targets: &[String]) -> Result<Vec<serde_json::Value>> {
    let table = std::fs::read_to_string("/proc/self/mountinfo").context("reading mountinfo")?;
    let mut entries: Vec<MountInfo> = procinfo::parse_mountinfo(&table)
        .into_iter()
        .filter(|entry| targets.contains(&entry.target))
        .collect();
    entries.sort_by_key(|entry| entry.mount_id);
    Ok(entries
        .iter()
        .map(|entry| {
            serde_json::json!({
                "target": entry.target,
                "source": entry.source_display(),
                "fstype": entry.fstype,
                "options": entry.options_display(),
            })
        })
        .collect())
}
