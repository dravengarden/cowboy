use super::{Containment, ContainmentIdentity, ProcessOwner};
use rustix::fs::{
    AtFlags, Dir, FileType, Mode, OFlags, ResolveFlags, fstat, fstatfs, open, openat, openat2,
    statat, unlinkat,
};
use std::{fs::File, io, io::Read as _, path::Path};

const DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

fn boot() -> io::Result<uuid::Uuid> {
    let id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .parse::<uuid::Uuid>()
        .map_err(io::Error::other)?;
    if id.is_nil() {
        return Err(io::Error::other("invalid kernel boot identity"));
    }
    Ok(id)
}

fn process(pid: u32) -> io::Result<(u64, bool)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    // comm may contain whitespace and parentheses. Fields after its final ')'
    // begin with state (3); starttime is field 22, not a wall-clock timestamp.
    let tail = stat
        .rsplit_once(')')
        .ok_or_else(|| io::Error::other("invalid process identity"))?
        .1;
    let fields: Vec<_> = tail.split_whitespace().collect();
    let start = fields
        .get(19)
        .ok_or_else(|| io::Error::other("missing process start"))?
        .parse()
        .map_err(io::Error::other)?;
    Ok((start, matches!(fields.first(), Some(&"Z" | &"X"))))
}

fn process_namespace() -> io::Result<u64> {
    use std::os::unix::fs::MetadataExt as _;
    // /proc may still expose a parent PID namespace after unshare. Refuse
    // numeric-PID observations unless procfs agrees with our execution view.
    let stat = std::fs::read_to_string("/proc/self/stat")?;
    if stat
        .split_whitespace()
        .next()
        .and_then(|id| id.parse::<u32>().ok())
        != Some(std::process::id())
    {
        return Err(io::Error::other(
            "process filesystem view differs from PID namespace",
        ));
    }
    Ok(std::fs::metadata("/proc/self/ns/pid")?.ino())
}

pub(super) fn owner() -> io::Result<ProcessOwner> {
    let pid = std::process::id();
    let pid_namespace = Some(process_namespace()?);
    Ok(ProcessOwner {
        boot: boot()?,
        pid,
        start: process(pid)?.0,
        pid_namespace,
    })
}

pub(super) fn owner_exited(owner: &ProcessOwner) -> io::Result<bool> {
    if owner.boot.is_nil() || owner.pid == 0 || owner.start == 0 {
        return Err(io::Error::other("invalid containment owner"));
    }
    if boot()? != owner.boot {
        return Ok(true);
    }
    if owner.pid_namespace != Some(process_namespace()?) {
        return Err(io::Error::other(
            "process ownership namespace changed or unknown",
        ));
    }
    match process(owner.pid) {
        Ok((start, dead)) => Ok(start != owner.start || dead),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

fn checked_path(identity: &ContainmentIdentity) -> io::Result<&Path> {
    let relative = identity
        .directory
        .strip_prefix(crate::cgroup::CGROUP_ROOT)
        .map_err(io::Error::other)?;
    let name = relative
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("agent-contained-"));
    if identity.owner.boot.is_nil()
        || identity.owner.pid == 0
        || identity.owner.start == 0
        || identity.device == 0
        || identity.inode == 0
        || !relative
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
        || !name.is_some_and(|name| {
            uuid::Uuid::parse_str(name).is_ok_and(|id| !id.is_nil() && id.to_string() == name)
        })
    {
        return Err(io::Error::other("invalid containment identity"));
    }
    Ok(relative)
}

fn open_root() -> io::Result<File> {
    let root = open(crate::cgroup::CGROUP_ROOT, DIRECTORY, Mode::empty())?;
    if fstatfs(&root)?.f_type != libc::CGROUP2_SUPER_MAGIC {
        return Err(io::Error::other("containment is not cgroup v2"));
    }
    Ok(root.into())
}

fn open_beneath(root: &File, relative: &Path) -> io::Result<File> {
    // A leaf directly under the cgroup namespace root has an empty parent.
    if relative.as_os_str().is_empty() {
        return root.try_clone();
    }
    let fd = openat2(
        root,
        relative,
        DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
    )?;
    if fstatfs(&fd)?.f_type != libc::CGROUP2_SUPER_MAGIC {
        return Err(io::Error::other("containment is not cgroup v2"));
    }
    Ok(fd.into())
}

fn recovery_parent(identity: &ContainmentIdentity) -> io::Result<File> {
    let relative = checked_path(identity)?;
    let root = open_root()?;
    let stat = fstat(&root)?;
    if stat.st_dev != identity.device || identity.root_inode != Some(stat.st_ino) {
        return Err(io::Error::other(
            "containment filesystem view changed or unknown",
        ));
    }
    // Missing ancestors are not an exit proof: the original view may no longer
    // be exposed. Only absence of the leaf beneath this verified view counts.
    open_beneath(
        &root,
        relative
            .parent()
            .ok_or_else(|| io::Error::other("missing cgroup parent"))?,
    )
}

fn matches(identity: &ContainmentIdentity, directory: &File) -> io::Result<()> {
    let stat = fstat(directory)?;
    if stat.st_dev != identity.device || stat.st_ino != identity.inode {
        return Err(io::Error::other("containment incarnation changed"));
    }
    Ok(())
}

fn member(directory: &File, name: &str, flags: OFlags) -> io::Result<File> {
    Ok(openat(
        directory,
        name,
        flags | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?
    .into())
}

pub(super) fn kill(directory: &File) -> io::Result<()> {
    use io::Write as _;
    member(directory, "cgroup.kill", OFlags::WRONLY)?.write_all(b"1")
}

fn empty(directory: &File) -> io::Result<bool> {
    let mut events = String::new();
    member(directory, "cgroup.events", OFlags::RDONLY)?
        .take(4096)
        .read_to_string(&mut events)?;
    match events
        .lines()
        .find_map(|line| line.strip_prefix("populated "))
    {
        Some("0") => Ok(true),
        Some("1") => Ok(false),
        _ => Err(io::Error::other("invalid cgroup population observation")),
    }
}

fn removed(directory: &File) -> io::Result<bool> {
    // kernfs retains st_nlink on an open, removed cgroup. Its mandatory
    // membership file cannot be unlinked independently; observing ENOENT
    // through the original directory FD proves that the kernel removed it.
    match member(directory, "cgroup.procs", OFlags::RDONLY) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

pub(super) fn prepare(command: &mut tokio::process::Command) -> io::Result<Containment> {
    let directory = crate::cgroup::agent_dir(&format!("contained-{}", uuid::Uuid::new_v4()))
        .ok_or_else(|| io::Error::other("cgroup v2 unavailable"))?;
    let owner = owner()?;
    let root = open_root()?;
    let root_stat = fstat(&root)?;
    std::fs::create_dir(&directory)?;
    let opened = open_beneath(
        &root,
        directory
            .strip_prefix(crate::cgroup::CGROUP_ROOT)
            .map_err(io::Error::other)?,
    )?;
    let stat = fstat(&opened)?;
    let guard = Containment {
        identity: ContainmentIdentity {
            owner,
            directory,
            device: stat.st_dev,
            inode: stat.st_ino,
            root_inode: Some(root_stat.st_ino),
        },
        directory: opened,
    };
    let membership = member(&guard.directory, "cgroup.procs", OFlags::WRONLY)?;
    let _kill = member(&guard.directory, "cgroup.kill", OFlags::WRONLY)?;
    if !empty(&guard.directory)? {
        return Err(io::Error::other("new containment is populated"));
    }
    attach_before_exec(command, membership);
    Ok(guard)
}

/// Success requires unlinking the exact original group. Checking populated=0
/// alone races a child waiting to enter pre_exec with an already opened FD.
pub(super) fn remove(identity: &ContainmentIdentity, directory: &File) -> io::Result<()> {
    let relative = checked_path(identity)?;
    if removed(directory)? {
        return Ok(());
    }
    remove_empty_children(directory, 0, &mut 4096)?;
    let parent = recovery_parent(identity)?;
    let name = relative
        .file_name()
        .ok_or_else(|| io::Error::other("missing cgroup name"))?;
    let current = open_beneath(&parent, Path::new(name))?;
    matches(identity, &current)?;
    unlinkat(parent, name, AtFlags::REMOVEDIR)?;
    if !removed(directory)? {
        return Err(io::Error::other("containment removal unconfirmed"));
    }
    Ok(())
}

fn remove_empty_children(directory: &File, depth: u32, budget: &mut usize) -> io::Result<()> {
    if depth > 32 || *budget == 0 {
        return Err(io::Error::other(
            "containment subtree exceeds cleanup bound",
        ));
    }
    *budget -= 1;
    for entry in Dir::read_from(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_bytes() == b"."
            || name.to_bytes() == b".."
            || entry.file_type() != FileType::Directory
        {
            continue;
        }
        let child: File = openat(directory, name, DIRECTORY, Mode::empty())?.into();
        remove_empty_children(&child, depth + 1, budget)?;
        let original = fstat(&child)?;
        let current = statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)?;
        if original.st_ino != current.st_ino || original.st_dev != current.st_dev {
            return Err(io::Error::other("containment subtree changed"));
        }
        unlinkat(directory, name, AtFlags::REMOVEDIR)?;
        if !removed(&child)? {
            return Err(io::Error::other("containment subtree removal unconfirmed"));
        }
    }
    Ok(())
}

pub(super) async fn terminate(identity: &ContainmentIdentity) -> io::Result<()> {
    let relative = checked_path(identity)?;
    if boot()? != identity.owner.boot {
        return Ok(()); // Old-boot processes cannot survive; never touch the new path.
    }
    let parent = recovery_parent(identity)?;
    let name = relative
        .file_name()
        .ok_or_else(|| io::Error::other("missing cgroup name"))?;
    let directory = match open_beneath(&parent, Path::new(name)) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    matches(identity, &directory)?;
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if removed(&directory)? {
                return Ok(());
            }
            kill(&directory)?;
            if empty(&directory)? {
                match remove(identity, &directory) {
                    Ok(()) => return Ok(()),
                    Err(error)
                        if error.raw_os_error() == Some(rustix::io::Errno::BUSY.raw_os_error()) => {
                    }
                    Err(error) => return Err(error),
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| io::Error::other("process tree exit unconfirmed"))?;
    // Another reconciliation can remove this same group while we read or kill
    // it. Confirm disappearance through the original FD, never a new path.
    match result {
        Err(_) if removed(&directory)? => Ok(()),
        result => result,
    }
}

#[allow(unsafe_code)]
pub(super) fn attach_before_exec(command: &mut tokio::process::Command, membership: File) {
    // SAFETY: only a write syscall and stack-only error conversion after fork.
    // A removed cgroup rejects this write; provider code cannot escape recovery.
    unsafe {
        command.pre_exec(move || match rustix::io::write(&membership, b"0") {
            Ok(1) => Ok(()),
            Ok(_) => Err(io::Error::from_raw_os_error(5)),
            Err(error) => Err(error.into()),
        });
    }
}
