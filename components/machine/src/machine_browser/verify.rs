//! Confinement verification from outside the instance. The launcher reports
//! what it built; the Machine reads the same facts through `/proc` and the
//! unit's cgroup and accepts readiness, navigation and adoption only when
//! they agree and nothing inside can listen, send datagrams or reach beyond
//! its aliases. Every pid is pinned to its start time so a reused pid can
//! never satisfy a check.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::launcher::{CONTROL_SOCKET, RFB_SOCKET, ReadyReport};
use super::unit;

/// A fact that did not hold. Carries a reason, never payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation(pub String);

impl std::fmt::Display for Violation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Violation {}

/// What the Machine expects of a running instance.
#[derive(Debug, Clone)]
pub struct Expectations<'a> {
    pub uid: u32,
    pub gid: u32,
    /// Extracted artifact root on the host.
    pub runtime_root: &'a Path,
    /// The unit's `RuntimeDirectory` on the host.
    pub runtime_dir: &'a Path,
    pub cgroup_dir: &'a Path,
    /// The unit's `MainPID`: the outer supervisor.
    pub main_pid: u32,
    pub profile_bytes: u64,
    pub instance_bytes: u64,
    pub policy_directory: &'a str,
    /// Enforce-mode labels; `None` only where no policy is expected (tests).
    pub bootstrap_label: Option<&'a str>,
    pub sealed_label: Option<&'a str>,
}

/// One verified process: host pid pinned to its start time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pin {
    pub pid: u32,
    pub start: u64,
}

/// Host-side identities of the instance's processes after verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub supervisor: Pin,
    pub init: Pin,
    pub display_server: Pin,
    pub window_manager: Pin,
    pub browser: Pin,
}

impl Verified {
    /// Whether every pinned process still exists with its start time.
    #[must_use]
    pub fn pins_hold(&self) -> bool {
        [
            self.supervisor,
            self.init,
            self.display_server,
            self.window_manager,
            self.browser,
        ]
        .iter()
        .all(|pin| start_time(pin.pid).is_ok_and(|start| start == pin.start))
    }
}

/// Full verification: process placement and identities, namespaces, uid and
/// gid maps, labels, capability sets, the browser's mount table, the socket
/// scan and the host runtime directory.
///
/// # Errors
/// Returns the first fact that did not hold.
pub fn verify(expect: &Expectations<'_>, report: &ReadyReport) -> Result<Verified, Violation> {
    let own = Namespaces::of(std::process::id())
        .map_err(|error| Violation(format!("own namespaces unreadable: {error}")))?;
    let placement = Placement::read(expect.cgroup_dir)?;
    let verified = placement.roles(expect, report)?;
    let instance = Namespaces::of(verified.init.pid)
        .map_err(|error| Violation(format!("init namespaces unreadable: {error}")))?;
    if instance.as_report() != report.namespaces {
        return Err(Violation(
            "launcher reported namespaces differ from the init's".into(),
        ));
    }
    if [&instance.user, &instance.mnt, &instance.pid, &instance.ipc]
        .iter()
        .any(|identity| own.all().contains(identity))
    {
        return Err(Violation(
            "instance shares a namespace with the Machine".into(),
        ));
    }
    for pin in [
        verified.display_server,
        verified.window_manager,
        verified.browser,
    ] {
        let namespaces = Namespaces::of(pin.pid).map_err(|error| {
            Violation(format!(
                "process {} namespaces unreadable: {error}",
                pin.pid
            ))
        })?;
        if namespaces != instance {
            return Err(Violation(format!(
                "process {} is outside the instance namespaces",
                pin.pid
            )));
        }
    }
    check_map(
        &format!("/proc/{}/uid_map", verified.init.pid),
        expect.uid,
        &report.uid_map,
        "uid_map",
    )?;
    check_map(
        &format!("/proc/{}/gid_map", verified.init.pid),
        expect.gid,
        &report.gid_map,
        "gid_map",
    )?;
    check_labels(expect, report, &placement, &verified)?;
    check_status(&placement, &verified)?;
    check_mounts(expect, verified.browser.pid)?;
    scan_sockets(expect.cgroup_dir)?;
    runtime_directory_clean(expect.runtime_dir)?;
    if !verified.pins_hold() {
        return Err(Violation(
            "a verified process changed during verification".into(),
        ));
    }
    Ok(verified)
}

/// No process of the cgroup subtree owns a listening TCP socket, any UDP
/// socket or an abstract Unix socket, in any network namespace found there.
///
/// # Errors
/// Returns the violation, or an unreadable observation as a violation.
pub fn scan_sockets(cgroup_dir: &Path) -> Result<(), Violation> {
    let pids: Vec<u32> = unit::procs_recursive(cgroup_dir)
        .map_err(|error| Violation(format!("cgroup unreadable: {error}")))?
        .into_iter()
        .map(|(_, pid)| pid)
        .collect();
    let mut owned: BTreeMap<u64, u32> = BTreeMap::new();
    let mut by_net: BTreeMap<String, u32> = BTreeMap::new();
    for &pid in &pids {
        let fds = match std::fs::read_dir(format!("/proc/{pid}/fd")) {
            Ok(fds) => fds,
            // A process that exited during the scan owns nothing.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(Violation(format!(
                    "process {pid} descriptors unreadable: {error}"
                )));
            }
        };
        for fd in fds.flatten() {
            if let Ok(target) = std::fs::read_link(fd.path())
                && let Some(inode) = target
                    .to_str()
                    .and_then(|target| target.strip_prefix("socket:["))
                    .and_then(|rest| rest.strip_suffix(']'))
                    .and_then(|inode| inode.parse::<u64>().ok())
            {
                owned.insert(inode, pid);
            }
        }
        if let Ok(net) = std::fs::read_link(format!("/proc/{pid}/ns/net")) {
            by_net
                .entry(net.to_string_lossy().into_owned())
                .or_insert(pid);
        }
    }
    if owned.is_empty() {
        return Ok(());
    }
    for pid in by_net.values() {
        for (table, listening_only) in [
            ("tcp", true),
            ("tcp6", true),
            ("udp", false),
            ("udp6", false),
        ] {
            let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/net/{table}")) else {
                continue;
            };
            for line in text.lines().skip(1) {
                let fields: Vec<&str> = line.split_whitespace().collect();
                let (Some(state), Some(inode)) = (fields.get(3), fields.get(9)) else {
                    continue;
                };
                let Ok(inode) = inode.parse::<u64>() else {
                    continue;
                };
                if let Some(owner) = owned.get(&inode)
                    && (!listening_only || *state == "0A")
                {
                    return Err(Violation(format!(
                        "process {owner} owns a {table} socket{}",
                        if listening_only {
                            " in LISTEN state"
                        } else {
                            ""
                        }
                    )));
                }
            }
        }
        if let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/net/unix")) {
            for line in text.lines().skip(1) {
                let fields: Vec<&str> = line.split_whitespace().collect();
                let (Some(inode), Some(path)) = (fields.get(6), fields.get(7)) else {
                    continue;
                };
                let Ok(inode) = inode.parse::<u64>() else {
                    continue;
                };
                if path.starts_with('@')
                    && let Some(owner) = owned.get(&inode)
                {
                    return Err(Violation(format!(
                        "process {owner} owns an abstract Unix socket"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// The host `RuntimeDirectory` holds exactly the control and RFB sockets.
///
/// # Errors
/// Returns the violation.
pub fn runtime_directory_clean(runtime_dir: &Path) -> Result<(), Violation> {
    use std::os::unix::fs::FileTypeExt as _;
    let mut names = BTreeSet::new();
    let entries = std::fs::read_dir(runtime_dir)
        .map_err(|error| Violation(format!("runtime directory unreadable: {error}")))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| Violation(format!("runtime directory unreadable: {error}")))?;
        let kind = entry
            .file_type()
            .map_err(|error| Violation(format!("runtime directory entry unreadable: {error}")))?;
        if !kind.is_socket() {
            return Err(Violation(
                "runtime directory holds something other than a socket".into(),
            ));
        }
        names.insert(entry.file_name().to_string_lossy().into_owned());
    }
    if names != BTreeSet::from([CONTROL_SOCKET.to_owned(), RFB_SOCKET.to_owned()]) {
        return Err(Violation(
            "runtime directory does not hold exactly the two sockets".into(),
        ));
    }
    Ok(())
}

/// Start time of a host process (`/proc/<pid>/stat` field 22).
///
/// # Errors
/// Returns when the process is gone or its stat is unreadable.
pub fn start_time(pid: u32) -> std::io::Result<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let tail = stat
        .rsplit_once(')')
        .ok_or_else(|| std::io::Error::other("invalid process stat"))?
        .1;
    tail.split_whitespace()
        .nth(19)
        .ok_or_else(|| std::io::Error::other("missing process start"))?
        .parse()
        .map_err(std::io::Error::other)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Namespaces {
    user: String,
    mnt: String,
    pid: String,
    ipc: String,
}

impl Namespaces {
    fn of(pid: u32) -> std::io::Result<Self> {
        let link = |name: &str| -> std::io::Result<String> {
            Ok(std::fs::read_link(format!("/proc/{pid}/ns/{name}"))?
                .to_string_lossy()
                .into_owned())
        };
        Ok(Self {
            user: link("user")?,
            mnt: link("mnt")?,
            pid: link("pid")?,
            ipc: link("ipc")?,
        })
    }

    fn all(&self) -> [&String; 4] {
        [&self.user, &self.mnt, &self.pid, &self.ipc]
    }

    fn as_report(&self) -> super::launcher::Namespaces {
        super::launcher::Namespaces {
            user: self.user.clone(),
            mnt: self.mnt.clone(),
            pid: self.pid.clone(),
            ipc: self.ipc.clone(),
        }
    }
}

/// `/proc/<pid>/status` facts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Status {
    ns_pids: Vec<u32>,
    no_new_privs: bool,
    /// Inheritable, permitted, effective, bounding, ambient.
    caps: [u64; 5],
}

fn status(pid: u32) -> std::io::Result<Status> {
    parse_status(&std::fs::read_to_string(format!("/proc/{pid}/status"))?)
}

fn parse_status(text: &str) -> std::io::Result<Status> {
    let mut status = Status::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let index = match key {
            "NSpid" => {
                status.ns_pids = value
                    .split_whitespace()
                    .map(|pid| pid.parse::<u32>().map_err(std::io::Error::other))
                    .collect::<Result<_, _>>()?;
                continue;
            }
            "NoNewPrivs" => {
                status.no_new_privs = value == "1";
                continue;
            }
            "CapInh" => 0,
            "CapPrm" => 1,
            "CapEff" => 2,
            "CapBnd" => 3,
            "CapAmb" => 4,
            _ => continue,
        };
        status.caps[index] = u64::from_str_radix(value, 16).map_err(std::io::Error::other)?;
    }
    if status.ns_pids.is_empty() {
        return Err(std::io::Error::other("process status lacks NSpid"));
    }
    Ok(status)
}

/// Processes of the unit's cgroup subtree, by sub-cgroup and namespace pid.
struct Placement {
    /// Sub-cgroup (relative, empty for the root) and status per host pid.
    processes: BTreeMap<u32, (PathBuf, Status)>,
}

impl Placement {
    fn read(cgroup_dir: &Path) -> Result<Self, Violation> {
        let mut processes = BTreeMap::new();
        for (sub, pid) in unit::procs_recursive(cgroup_dir)
            .map_err(|error| Violation(format!("cgroup unreadable: {error}")))?
        {
            let status = status(pid)
                .map_err(|error| Violation(format!("process {pid} status unreadable: {error}")))?;
            processes.insert(pid, (sub, status));
        }
        Ok(Self { processes })
    }

    /// Host pid of the process the instance knows as `ns_pid`, placed in
    /// `sub`. Nested sandboxes have longer `NSpid` lists; their second entry
    /// is still their pid in the instance namespace, so the match is exact.
    fn host_pid(&self, ns_pid: u32, sub: &str) -> Result<u32, Violation> {
        let mut found = None;
        for (pid, (placed, status)) in &self.processes {
            if status.ns_pids.len() >= 2 && status.ns_pids[1] == ns_pid {
                if placed != Path::new(sub) {
                    return Err(Violation(format!(
                        "process {pid} is placed in {:?} instead of {sub:?}",
                        placed.display()
                    )));
                }
                if found.replace(*pid).is_some() {
                    return Err(Violation(format!("namespace pid {ns_pid} is ambiguous")));
                }
            }
        }
        found.ok_or_else(|| Violation(format!("namespace pid {ns_pid} is not in the unit cgroup")))
    }

    fn roles(
        &self,
        expect: &Expectations<'_>,
        report: &ReadyReport,
    ) -> Result<Verified, Violation> {
        let pin = |pid: u32| -> Result<Pin, Violation> {
            Ok(Pin {
                pid,
                start: start_time(pid).map_err(|error| {
                    Violation(format!("process {pid} start unreadable: {error}"))
                })?,
            })
        };
        let mut supervisor = None;
        let mut init = None;
        for (pid, (sub, status)) in &self.processes {
            if !sub.as_os_str().is_empty() {
                continue;
            }
            match status.ns_pids.as_slice() {
                [_] if supervisor.replace(*pid).is_none() => {}
                [_, 1] if init.replace(*pid).is_none() => {}
                _ => {
                    return Err(Violation(format!(
                        "unexpected process {pid} in the unit's root cgroup"
                    )));
                }
            }
        }
        let supervisor = supervisor.ok_or_else(|| Violation("supervisor is missing".into()))?;
        if supervisor != expect.main_pid {
            return Err(Violation(
                "supervisor is not the unit's main process".into(),
            ));
        }
        let init = init.ok_or_else(|| Violation("namespace init is missing".into()))?;
        if report.pids.init != 1 {
            return Err(Violation(
                "launcher init is not pid 1 of its namespace".into(),
            ));
        }
        Ok(Verified {
            supervisor: pin(supervisor)?,
            init: pin(init)?,
            display_server: pin(self.host_pid(report.pids.display_server, "display")?)?,
            window_manager: pin(self.host_pid(report.pids.window_manager, "display")?)?,
            browser: pin(self.host_pid(report.pids.browser, "browser")?)?,
        })
    }
}

fn check_map(path: &str, id: u32, reported: &str, label: &str) -> Result<(), Violation> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| Violation(format!("{label} unreadable: {error}")))?;
    let normalized: Vec<&str> = text.split_whitespace().collect();
    let expected = [id.to_string(), id.to_string(), "1".to_owned()];
    if normalized != expected {
        return Err(Violation(format!("{label} is not the single self-mapping")));
    }
    if reported.split_whitespace().collect::<Vec<_>>() != expected {
        return Err(Violation(format!("launcher reported a different {label}")));
    }
    Ok(())
}

fn label(pid: u32) -> Result<String, Violation> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/attr/current"))
        .map_err(|error| Violation(format!("process {pid} label unreadable: {error}")))?;
    Ok(text.trim_end_matches(['\n', '\0']).to_owned())
}

fn check_labels(
    expect: &Expectations<'_>,
    report: &ReadyReport,
    placement: &Placement,
    verified: &Verified,
) -> Result<(), Violation> {
    if let Some(bootstrap) = expect.bootstrap_label {
        for pin in [verified.supervisor, verified.init] {
            if label(pin.pid)? != bootstrap {
                return Err(Violation(format!(
                    "process {} is not under the bootstrap profile",
                    pin.pid
                )));
            }
        }
        if report.labels.init != bootstrap {
            return Err(Violation("launcher reported a different init label".into()));
        }
    }
    if let Some(sealed) = expect.sealed_label {
        for (pid, (sub, _)) in &placement.processes {
            if sub.as_os_str().is_empty() {
                continue;
            }
            if label(*pid)? != sealed {
                return Err(Violation(format!(
                    "process {pid} is not under the sealed profile"
                )));
            }
        }
        if report.labels.sealed.as_deref() != Some(sealed) {
            return Err(Violation(
                "launcher reported a different sealed label".into(),
            ));
        }
    }
    Ok(())
}

fn check_status(placement: &Placement, verified: &Verified) -> Result<(), Violation> {
    let main = [
        verified.display_server.pid,
        verified.window_manager.pid,
        verified.browser.pid,
    ];
    for (pid, (sub, status)) in &placement.processes {
        if sub.as_os_str().is_empty() {
            continue;
        }
        if !status.no_new_privs {
            return Err(Violation(format!("process {pid} lacks no_new_privs")));
        }
        // Nested sandboxes legitimately hold capabilities inside their own
        // user namespace; the three main processes must hold none anywhere.
        if main.contains(pid) && status.caps.iter().any(|set| *set != 0) {
            return Err(Violation(format!("process {pid} holds capabilities")));
        }
        if status.ns_pids.len() < 2 {
            return Err(Violation(format!(
                "process {pid} is outside the instance pid namespace"
            )));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Mount {
    mount_point: String,
    root: String,
    device: String,
    options: BTreeSet<String>,
    fstype: String,
    super_options: BTreeSet<String>,
}

fn unescape(field: &str) -> String {
    let mut output = String::with_capacity(field.len());
    let mut chars = field.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let code: String = chars.by_ref().take(3).collect();
            if let Ok(byte) = u8::from_str_radix(&code, 8) {
                output.push(byte as char);
                continue;
            }
            output.push('\\');
            output.push_str(&code);
            continue;
        }
        output.push(ch);
    }
    output
}

fn parse_mountinfo(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let left: Vec<&str> = left.split_whitespace().collect();
            let right: Vec<&str> = right.split_whitespace().collect();
            Some(Mount {
                device: (*left.get(2)?).to_owned(),
                root: unescape(left.get(3)?),
                mount_point: unescape(left.get(4)?),
                options: left.get(5)?.split(',').map(ToOwned::to_owned).collect(),
                fstype: (*right.first()?).to_owned(),
                super_options: right
                    .get(2)
                    .map(|options| options.split(',').map(ToOwned::to_owned).collect())
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// Backing `(device, root)` a host path would show in a bind mount's
/// mountinfo entry: the device of the mount holding it and the path within
/// that mount's root.
fn host_backing(path: &Path) -> Result<(String, String), Violation> {
    let text = std::fs::read_to_string("/proc/self/mountinfo")
        .map_err(|error| Violation(format!("host mountinfo unreadable: {error}")))?;
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| Violation(format!("{} unreadable: {error}", path.display())))?;
    let mounts = parse_mountinfo(&text);
    let holder = mounts
        .iter()
        .filter(|mount| canonical.starts_with(&mount.mount_point))
        .max_by_key(|mount| mount.mount_point.len())
        .ok_or_else(|| Violation("host mount of the runtime is unknown".into()))?;
    let remainder = canonical
        .strip_prefix(&holder.mount_point)
        .map_err(|_| Violation("host mount resolution failed".into()))?;
    let mut root = PathBuf::from(&holder.root);
    root.push(remainder);
    Ok((
        holder.device.clone(),
        root.to_string_lossy().trim_end_matches('/').to_owned(),
    ))
}

fn tmpfs_size_matches(mount: &Mount, bytes: u64) -> bool {
    mount.super_options.iter().any(|option| {
        option
            .strip_prefix("size=")
            .and_then(|size| size.strip_suffix('k'))
            .and_then(|size| size.parse::<u64>().ok())
            .is_some_and(|kib| kib.saturating_mul(1024).abs_diff(bytes) < 4096)
    })
}

fn check_mounts(expect: &Expectations<'_>, browser: u32) -> Result<(), Violation> {
    let text = std::fs::read_to_string(format!("/proc/{browser}/mountinfo"))
        .map_err(|error| Violation(format!("browser mountinfo unreadable: {error}")))?;
    let mounts = parse_mountinfo(&text);
    let find = |point: &str| mounts.iter().rfind(|mount| mount.mount_point == point);
    for (point, subdir) in [("/usr", "usr"), ("/opt", "opt")] {
        let mount = find(point)
            .ok_or_else(|| Violation(format!("{point} is not a mount in the instance")))?;
        let (device, root) = host_backing(&expect.runtime_root.join(subdir))?;
        if mount.device != device || mount.root.trim_end_matches('/') != root {
            return Err(Violation(format!(
                "{point} is not backed by the installed generation"
            )));
        }
        for flag in ["ro", "nosuid", "nodev"] {
            if !mount.options.contains(flag) {
                return Err(Violation(format!("{point} lacks the {flag} flag")));
            }
        }
    }
    let runtime_device = host_backing(expect.runtime_root)?.0;
    for (point, size) in [
        ("/mnt/profile", Some(expect.profile_bytes)),
        ("/mnt/instance", Some(expect.instance_bytes)),
        ("/dev/shm", Some(expect.profile_bytes)),
        ("/tmp", None),
        ("/etc", None),
        ("/home", None),
    ] {
        let mount = find(point)
            .ok_or_else(|| Violation(format!("{point} is not a mount in the instance")))?;
        if mount.fstype != "tmpfs" {
            return Err(Violation(format!("{point} is not a tmpfs")));
        }
        if let Some(bytes) = size
            && !tmpfs_size_matches(mount, bytes)
        {
            return Err(Violation(format!(
                "{point} does not have the expected size"
            )));
        }
    }
    let run_user = format!("/run/user/{}", expect.uid);
    let masked = find(&run_user).ok_or_else(|| Violation(format!("{run_user} is not masked")))?;
    if masked.fstype != "tmpfs" {
        return Err(Violation(format!("{run_user} is not masked with a tmpfs")));
    }
    let sockets =
        find("/mnt/sockets").ok_or_else(|| Violation("/mnt/sockets is not a mount".into()))?;
    let (device, root) = host_backing(expect.runtime_dir)?;
    if sockets.device != device || sockets.root.trim_end_matches('/') != root {
        return Err(Violation(
            "/mnt/sockets is not the unit's runtime directory".into(),
        ));
    }
    if !sockets.options.contains("ro") {
        return Err(Violation("/mnt/sockets is writable".into()));
    }
    for absent in ["/mnt/snapshot", "/mnt/publish"] {
        if find(absent).is_some() {
            return Err(Violation(format!("{absent} is still mounted")));
        }
    }
    let policy_prefix = format!("{}/", expect.policy_directory.trim_end_matches('/'));
    let policy = mounts
        .iter()
        .find(|mount| {
            mount.mount_point.starts_with(&policy_prefix)
                || mount.mount_point == expect.policy_directory
        })
        .ok_or_else(|| Violation("browser policy is not bound".into()))?;
    if policy.device != runtime_device {
        return Err(Violation(
            "browser policy is not bound from the artifact".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parsing_reads_nspid_no_new_privs_and_capability_sets() {
        let status = parse_status(
            "Name:\tchrome\nNSpid:\t4242\t17\t3\nCapInh:\t0000000000000000\nCapPrm:\t0000000000000000\nCapEff:\t0000000000000000\nCapBnd:\t0000000000000000\nCapAmb:\t0000000000000000\nNoNewPrivs:\t1\n",
        )
        .unwrap();
        assert_eq!(status.ns_pids, vec![4242, 17, 3]);
        assert!(status.no_new_privs);
        assert_eq!(status.caps, [0; 5]);
        let privileged =
            parse_status("NSpid:\t1\nCapBnd:\t000001ffffffffff\nNoNewPrivs:\t0\n").unwrap();
        assert!(!privileged.no_new_privs);
        assert_ne!(privileged.caps[3], 0);
        assert!(parse_status("Name:\tx\n").is_err());
    }

    #[test]
    fn mountinfo_parsing_unescapes_paths_and_reads_sizes() {
        let mounts = parse_mountinfo(
            "66 1 259:13 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw\n\
             70 66 0:40 / /mnt/profile rw,nosuid,nodev,relatime - tmpfs tmpfs rw,size=262144k,uid=1000\n\
             71 66 259:13 /state/plugins/a/content/usr /usr ro,nosuid,nodev,relatime - ext4 /dev/nvme0n1p2 rw\n\
             72 66 0:41 / /mnt/a\\040b rw - tmpfs tmpfs rw\n",
        );
        assert_eq!(mounts.len(), 4);
        assert_eq!(mounts[1].mount_point, "/mnt/profile");
        assert!(tmpfs_size_matches(&mounts[1], 256 << 20));
        assert!(!tmpfs_size_matches(&mounts[1], 128 << 20));
        assert_eq!(mounts[2].root, "/state/plugins/a/content/usr");
        assert!(mounts[2].options.contains("ro"));
        assert_eq!(mounts[3].mount_point, "/mnt/a b");
    }

    #[test]
    fn the_machine_itself_is_a_valid_subject_for_the_proc_readers() {
        let pid = std::process::id();
        let own = Namespaces::of(pid).unwrap();
        assert!(own.user.starts_with("user:["));
        assert!(start_time(pid).unwrap() > 0);
        let status = status(pid).unwrap();
        assert_eq!(status.ns_pids.last(), Some(&pid));
        let pin = Pin {
            pid,
            start: start_time(pid).unwrap(),
        };
        assert!(start_time(pin.pid).is_ok_and(|start| start == pin.start));
        assert!(
            host_backing(Path::new("/proc/self/exe")).is_ok()
                || host_backing(Path::new("/")).is_ok()
        );
    }

    #[test]
    fn runtime_directory_must_hold_exactly_the_two_sockets() {
        let temp = tempfile::tempdir().unwrap();
        assert!(runtime_directory_clean(temp.path()).is_err());
        let _control =
            std::os::unix::net::UnixListener::bind(temp.path().join(CONTROL_SOCKET)).unwrap();
        let _rfb = std::os::unix::net::UnixListener::bind(temp.path().join(RFB_SOCKET)).unwrap();
        runtime_directory_clean(temp.path()).unwrap();
        std::fs::write(temp.path().join("extra"), b"x").unwrap();
        assert!(runtime_directory_clean(temp.path()).is_err());
    }
}
