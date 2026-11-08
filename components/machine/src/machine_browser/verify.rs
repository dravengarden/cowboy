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
    /// Artifact-relative managed policy file the policy bind must be backed by.
    pub policy_file: &'a str,
    /// Host paths that must be hidden behind an empty read-only tmpfs mask
    /// (the Machine state root, `/home`, `/run/user/<uid>`), as the kernel
    /// resolves them.
    pub masks: &'a [PathBuf],
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
    /// The namespace relay between the supervisor and the init.
    pub relay: Pin,
    pub init: Pin,
    pub display_server: Pin,
    pub window_manager: Pin,
    pub browser: Pin,
}

impl Verified {
    /// Nothing verified yet: every pin is the impossible pid 0.
    #[must_use]
    pub fn unpinned() -> Self {
        let none = Pin { pid: 0, start: 0 };
        Self {
            supervisor: none,
            relay: none,
            init: none,
            display_server: none,
            window_manager: none,
            browser: none,
        }
    }

    /// Whether every pinned process still exists with its start time.
    #[must_use]
    pub fn pins_hold(&self) -> bool {
        [
            self.supervisor,
            self.relay,
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
    let (verified, transient) = placement.roles(expect, report)?;
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
    let supervisor_pid_ns = std::fs::read_link(format!("/proc/{}/ns/pid", verified.supervisor.pid))
        .map_err(|error| Violation(format!("supervisor pid namespace unreadable: {error}")))?;
    let relay = Namespaces::of(verified.relay.pid)
        .map_err(|error| Violation(format!("relay namespaces unreadable: {error}")))?;
    let relay_children =
        std::fs::read_link(format!("/proc/{}/ns/pid_for_children", verified.relay.pid))
            .map_err(|error| Violation(format!("relay child pid namespace unreadable: {error}")))?;
    check_relay_namespaces(
        &relay,
        &relay_children.to_string_lossy(),
        &supervisor_pid_ns.to_string_lossy(),
        &instance,
    )?;
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
    check_labels(expect, report, &placement, &verified, &transient)?;
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
/// socket or a reachable (not connected) abstract Unix socket, in any network
/// namespace found there.
///
/// The scan fails closed: only a process confirmed gone (its `/proc` entry
/// vanished) may be skipped. Unreadable descriptors or namespace links of a
/// live process are a violation, and every network namespace's socket tables
/// must be read through some live member of it.
///
/// # Errors
/// Returns the violation, or an unreadable observation as a violation.
pub fn scan_sockets(cgroup_dir: &Path) -> Result<(), Violation> {
    let pids: Vec<u32> = unit::procs_recursive(cgroup_dir)
        .map_err(|error| Violation(format!("cgroup unreadable: {error}")))?
        .into_iter()
        .map(|(_, pid)| pid)
        .collect();
    scan_with(&HostProc, &pids)
}

/// Why a `/proc` observation is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Missing {
    /// The process is confirmed gone: it owns nothing any more.
    Exited,
    /// The process may be alive but its evidence could not be read.
    Unreadable(String),
}

/// The `/proc` facts the socket scan reads, so the scan's fail-closed rules
/// can be tested against synthetic process trees.
trait ProcView {
    /// Every descriptor target of `pid` (descriptors closed during the read
    /// are skipped).
    fn fd_targets(&self, pid: u32) -> Result<Vec<String>, Missing>;
    /// The `ns/net` identity of `pid`.
    fn net_namespace(&self, pid: u32) -> Result<String, Missing>;
    /// `/proc/<pid>/net/<table>`; `Ok(None)` when the process is alive and
    /// the kernel has no such table (IPv6 disabled).
    fn table(&self, pid: u32, table: &str) -> Result<Option<String>, Missing>;
}

struct HostProc;

impl HostProc {
    fn classify(pid: u32, error: &std::io::Error) -> Missing {
        let vanished = matches!(
            std::fs::symlink_metadata(format!("/proc/{pid}")),
            Err(ref gone) if gone.kind() == std::io::ErrorKind::NotFound
        );
        let exit_shaped = error.kind() == std::io::ErrorKind::NotFound
            || error.raw_os_error() == Some(rustix::io::Errno::SRCH.raw_os_error());
        if exit_shaped && vanished {
            Missing::Exited
        } else {
            Missing::Unreadable(error.to_string())
        }
    }
}

impl ProcView for HostProc {
    fn fd_targets(&self, pid: u32) -> Result<Vec<String>, Missing> {
        let entries = std::fs::read_dir(format!("/proc/{pid}/fd"))
            .map_err(|error| Self::classify(pid, &error))?;
        let mut targets = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| Self::classify(pid, &error))?;
            match std::fs::read_link(entry.path()) {
                Ok(target) => targets.push(target.to_string_lossy().into_owned()),
                // The descriptor was closed between the listing and the read.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if Self::classify(pid, &error) == Missing::Exited {
                        return Err(Missing::Exited);
                    }
                }
                Err(error) => return Err(Self::classify(pid, &error)),
            }
        }
        Ok(targets)
    }

    fn net_namespace(&self, pid: u32) -> Result<String, Missing> {
        std::fs::read_link(format!("/proc/{pid}/ns/net"))
            .map(|link| link.to_string_lossy().into_owned())
            .map_err(|error| Self::classify(pid, &error))
    }

    fn table(&self, pid: u32, table: &str) -> Result<Option<String>, Missing> {
        match std::fs::read_to_string(format!("/proc/{pid}/net/{table}")) {
            Ok(text) => Ok(Some(text)),
            Err(error) => match Self::classify(pid, &error) {
                Missing::Unreadable(_) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                missing => Err(missing),
            },
        }
    }
}

/// Tables every network namespace is scanned through: `(name, listening
/// only, may be absent)`.
const SOCKET_TABLES: [(&str, bool, bool); 4] = [
    ("tcp", true, false),
    ("tcp6", true, true),
    ("udp", false, false),
    ("udp6", false, true),
];

fn scan_with(proc: &impl ProcView, pids: &[u32]) -> Result<(), Violation> {
    let mut owned: BTreeMap<u64, u32> = BTreeMap::new();
    let mut by_net: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for &pid in pids {
        let targets = match proc.fd_targets(pid) {
            Ok(targets) => targets,
            Err(Missing::Exited) => continue,
            Err(Missing::Unreadable(why)) => {
                return Err(Violation(format!(
                    "process {pid} descriptors unreadable: {why}"
                )));
            }
        };
        for target in targets {
            if let Some(inode) = target
                .strip_prefix("socket:[")
                .and_then(|rest| rest.strip_suffix(']'))
                .and_then(|inode| inode.parse::<u64>().ok())
            {
                owned.insert(inode, pid);
            }
        }
        match proc.net_namespace(pid) {
            Ok(net) => by_net.entry(net).or_default().push(pid),
            Err(Missing::Exited) => {}
            Err(Missing::Unreadable(why)) => {
                return Err(Violation(format!(
                    "process {pid} network namespace unreadable: {why}"
                )));
            }
        }
    }
    if owned.is_empty() {
        return Ok(());
    }
    for (net, members) in &by_net {
        for (table, listening_only, optional) in SOCKET_TABLES {
            let Some(text) = read_table(proc, net, members, table, optional)? else {
                continue;
            };
            check_inet_table(&text, table, listening_only, &owned)?;
        }
        if let Some(text) = read_table(proc, net, members, "unix", false)? {
            check_unix_table(&text, &owned)?;
        }
    }
    Ok(())
}

/// One namespace's table, read through the first live member that can; a
/// table no live member can read is a violation.
fn read_table(
    proc: &impl ProcView,
    net: &str,
    members: &[u32],
    table: &str,
    optional: bool,
) -> Result<Option<String>, Violation> {
    let mut last = None;
    let mut live = false;
    for &pid in members {
        match proc.table(pid, table) {
            Ok(Some(text)) => return Ok(Some(text)),
            Ok(None) if optional => return Ok(None),
            Ok(None) => {
                live = true;
                last = Some(format!("process {pid} has no {table} table"));
            }
            Err(Missing::Exited) => {}
            Err(Missing::Unreadable(why)) => {
                live = true;
                last = Some(format!("process {pid}: {why}"));
            }
        }
    }
    if live {
        return Err(Violation(format!(
            "{table} sockets of network namespace {net} unreadable ({})",
            last.unwrap_or_default()
        )));
    }
    // Every member exited during the scan: the namespace's processes, and
    // with them the sockets they owned, are gone.
    Ok(None)
}

fn check_inet_table(
    text: &str,
    table: &str,
    listening_only: bool,
    owned: &BTreeMap<u64, u32>,
) -> Result<(), Violation> {
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
    Ok(())
}

fn check_unix_table(text: &str, owned: &BTreeMap<u64, u32>) -> Result<(), Violation> {
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let (Some(state), Some(inode), Some(path)) = (fields.get(5), fields.get(6), fields.get(7))
        else {
            continue;
        };
        let Ok(inode) = inode.parse::<u64>() else {
            continue;
        };
        // Chromium's SO_PASSCRED socketpairs are autobound to an abstract
        // name on first send; they are connected (state 03) and refuse every
        // third party. Only an unconnected or listening abstract socket is
        // reachable from outside.
        if path.starts_with('@')
            && *state != "03"
            && let Some(owner) = owned.get(&inode)
        {
            return Err(Violation(format!(
                "process {owner} owns a reachable abstract Unix socket"
            )));
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
    /// Parent pid as the host sees it.
    ppid: u32,
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
            "PPid" => {
                status.ppid = value.parse().map_err(std::io::Error::other)?;
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
            let status = match status(pid) {
                Ok(status) => status,
                // A process that exited after the cgroup listing (a finished
                // copier or helper) is no longer part of the tree.
                Err(_) if std::fs::symlink_metadata(format!("/proc/{pid}")).is_err() => continue,
                Err(error) => {
                    return Err(Violation(format!(
                        "process {pid} status unreadable: {error}"
                    )));
                }
            };
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

    /// The launcher's root-cgroup processes, by identity rather than count:
    /// the supervisor is the unit's `MainPID` in the host pid namespace; the
    /// namespace relay is its only child there, also with a single-entry
    /// `NSpid`; the init is the relay's child and pid 1 of the instance.
    /// Anything else in the root cgroup must be a child of the init inside
    /// the instance pid namespace (the snapshot copier, or a child in the
    /// instant before it moves itself into its sub-cgroup).
    fn root_roles(&self, main_pid: u32) -> Result<RootRoles, Violation> {
        let root: BTreeMap<u32, &Status> = self
            .processes
            .iter()
            .filter(|(_, (sub, _))| sub.as_os_str().is_empty())
            .map(|(pid, (_, status))| (*pid, status))
            .collect();
        let supervisor = root
            .get(&main_pid)
            .ok_or_else(|| Violation("supervisor is not the unit's main process".into()))?;
        if supervisor.ns_pids.len() != 1 {
            return Err(Violation(
                "supervisor is not in the host pid namespace".into(),
            ));
        }
        let only = |what: &str, matches: Vec<u32>| -> Result<u32, Violation> {
            match matches.as_slice() {
                [pid] => Ok(*pid),
                [] => Err(Violation(format!("{what} is missing"))),
                _ => Err(Violation(format!("{what} is ambiguous"))),
            }
        };
        let relay = only(
            "namespace relay",
            root.iter()
                .filter(|(pid, status)| {
                    **pid != main_pid && status.ns_pids.len() == 1 && status.ppid == main_pid
                })
                .map(|(pid, _)| *pid)
                .collect(),
        )?;
        let init = only(
            "namespace init",
            root.iter()
                .filter(|(_, status)| {
                    status.ns_pids.len() == 2 && status.ns_pids[1] == 1 && status.ppid == relay
                })
                .map(|(pid, _)| *pid)
                .collect(),
        )?;
        let mut transient = Vec::new();
        for (pid, status) in &root {
            if [main_pid, relay, init].contains(pid) {
                continue;
            }
            if status.ns_pids.len() == 2 && status.ns_pids[1] != 1 && status.ppid == init {
                transient.push(*pid);
                continue;
            }
            return Err(Violation(format!(
                "unexpected process {pid} in the unit's root cgroup"
            )));
        }
        Ok(RootRoles {
            supervisor: main_pid,
            relay,
            init,
            transient,
        })
    }

    fn roles(
        &self,
        expect: &Expectations<'_>,
        report: &ReadyReport,
    ) -> Result<(Verified, Vec<u32>), Violation> {
        let pin = |pid: u32| -> Result<Pin, Violation> {
            Ok(Pin {
                pid,
                start: start_time(pid).map_err(|error| {
                    Violation(format!("process {pid} start unreadable: {error}"))
                })?,
            })
        };
        let roots = self.root_roles(expect.main_pid)?;
        if report.pids.init != 1 {
            return Err(Violation(
                "launcher init is not pid 1 of its namespace".into(),
            ));
        }
        Ok((
            Verified {
                supervisor: pin(roots.supervisor)?,
                relay: pin(roots.relay)?,
                init: pin(roots.init)?,
                display_server: pin(self.host_pid(report.pids.display_server, "display")?)?,
                window_manager: pin(self.host_pid(report.pids.window_manager, "display")?)?,
                browser: pin(self.host_pid(report.pids.browser, "browser")?)?,
            },
            roots.transient,
        ))
    }
}

/// Host pids of the launcher's own processes in the unit's root cgroup.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RootRoles {
    supervisor: u32,
    relay: u32,
    init: u32,
    /// Children of the init still in the root cgroup.
    transient: Vec<u32>,
}

/// The relay unshared the user, mount and IPC namespaces the init lives in
/// and stays in its parent's pid namespace, while its children's pid
/// namespace is the instance's.
fn check_relay_namespaces(
    relay: &Namespaces,
    relay_children_pid: &str,
    supervisor_pid: &str,
    instance: &Namespaces,
) -> Result<(), Violation> {
    if relay.user != instance.user || relay.mnt != instance.mnt || relay.ipc != instance.ipc {
        return Err(Violation(
            "namespace relay is outside the instance's user, mount or IPC namespace".into(),
        ));
    }
    if relay.pid != supervisor_pid {
        return Err(Violation(
            "namespace relay left the supervisor's pid namespace".into(),
        ));
    }
    if relay_children_pid != instance.pid {
        return Err(Violation(
            "namespace relay's children are outside the instance pid namespace".into(),
        ));
    }
    Ok(())
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
    transient: &[u32],
) -> Result<(), Violation> {
    if let Some(bootstrap) = expect.bootstrap_label {
        for pid in [
            verified.supervisor.pid,
            verified.relay.pid,
            verified.init.pid,
        ] {
            if label(pid)? != bootstrap {
                return Err(Violation(format!(
                    "process {pid} is not under the bootstrap profile"
                )));
            }
        }
        for &pid in transient {
            match label(pid) {
                Ok(found) if found == bootstrap => {}
                // A copier that finished during verification is gone.
                Err(_) if std::fs::symlink_metadata(format!("/proc/{pid}")).is_err() => {}
                Ok(_) | Err(_) => {
                    return Err(Violation(format!(
                        "process {pid} is not under the bootstrap profile"
                    )));
                }
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
    for (point, size) in [
        ("/mnt/profile", Some(expect.profile_bytes)),
        ("/mnt/instance", Some(expect.instance_bytes)),
        ("/dev/shm", Some(expect.profile_bytes)),
        ("/tmp", None),
        ("/etc", None),
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
    check_masks(&mounts, expect.masks)?;
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
    let backing = host_backing(&expect.runtime_root.join(expect.policy_file))?;
    check_policy_bind(&mounts, expect.policy_directory, &backing)
}

/// Bytes of the empty mask tmpfs the launcher mounts.
const MASK_BYTES: u64 = 4096;

/// An empty read-only mask: `tmpfs`, `ro,nosuid,nodev,noexec`, 4 KiB.
fn is_mask(mount: &Mount) -> bool {
    mount.fstype == "tmpfs"
        && ["ro", "nosuid", "nodev", "noexec"]
            .iter()
            .all(|flag| mount.options.contains(*flag))
        && tmpfs_size_matches(mount, MASK_BYTES)
}

/// Every sensitive path is covered by a mask: the topmost mount holding it
/// is a mask, and nothing but masks is mounted below it.
fn check_masks(mounts: &[Mount], paths: &[PathBuf]) -> Result<(), Violation> {
    for path in paths {
        let covering = mounts
            .iter()
            .enumerate()
            .filter(|(_, mount)| path.starts_with(&mount.mount_point))
            .max_by_key(|(index, mount)| {
                (Path::new(&mount.mount_point).components().count(), *index)
            })
            .map(|(_, mount)| mount)
            .ok_or_else(|| Violation(format!("{} is not under any mount", path.display())))?;
        if !is_mask(covering) {
            return Err(Violation(format!(
                "{} is not hidden behind a read-only mask",
                path.display()
            )));
        }
        if let Some(exposed) = mounts.iter().find(|mount| {
            let point = Path::new(&mount.mount_point);
            point != path && point.starts_with(path) && !is_mask(mount)
        }) {
            return Err(Violation(format!(
                "{} re-exposes a mount below the masked {}",
                exposed.mount_point,
                path.display()
            )));
        }
    }
    Ok(())
}

/// The managed policy is bound read-only at `<policy-directory>/policy.json`
/// from exactly the artifact's policy file (`backing` is its host device and
/// path within that device's mount root).
fn check_policy_bind(
    mounts: &[Mount],
    policy_directory: &str,
    backing: &(String, String),
) -> Result<(), Violation> {
    let point = format!("{}/policy.json", policy_directory.trim_end_matches('/'));
    let policy = mounts
        .iter()
        .rfind(|mount| mount.mount_point == point)
        .ok_or_else(|| Violation("browser policy is not bound".into()))?;
    if policy.device != backing.0 || policy.root.trim_end_matches('/') != backing.1 {
        return Err(Violation(
            "browser policy is not bound from the artifact's policy file".into(),
        ));
    }
    if !policy.options.contains("ro") {
        return Err(Violation("browser policy bind is writable".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parsing_reads_nspid_no_new_privs_and_capability_sets() {
        let status = parse_status(
            "Name:\tchrome\nPPid:\t4241\nNSpid:\t4242\t17\t3\nCapInh:\t0000000000000000\nCapPrm:\t0000000000000000\nCapEff:\t0000000000000000\nCapBnd:\t0000000000000000\nCapAmb:\t0000000000000000\nNoNewPrivs:\t1\n",
        )
        .unwrap();
        assert_eq!(status.ns_pids, vec![4242, 17, 3]);
        assert_eq!(status.ppid, 4241);
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

    fn process(sub: &str, ns_pids: &[u32], ppid: u32) -> (PathBuf, Status) {
        (
            PathBuf::from(sub),
            Status {
                ns_pids: ns_pids.to_vec(),
                ppid,
                no_new_privs: !sub.is_empty(),
                caps: [0; 5],
            },
        )
    }

    /// The launcher's normal tree: supervisor (MainPID) → relay → init in
    /// the root cgroup, the display and browser below.
    fn launcher_tree() -> Placement {
        Placement {
            processes: BTreeMap::from([
                (100, process("", &[100], 1)),
                (101, process("", &[101], 100)),
                (102, process("", &[102, 1], 101)),
                (110, process("display", &[110, 7], 102)),
                (111, process("display", &[111, 9], 102)),
                (120, process("browser", &[120, 12], 102)),
            ]),
        }
    }

    #[test]
    fn root_roles_accept_the_three_process_supervision_tree() {
        let tree = launcher_tree();
        assert_eq!(
            tree.root_roles(100).unwrap(),
            RootRoles {
                supervisor: 100,
                relay: 101,
                init: 102,
                transient: vec![],
            }
        );
        // The snapshot copier is the init's child in the root cgroup.
        let mut copying = launcher_tree();
        copying.processes.insert(130, process("", &[130, 20], 102));
        assert_eq!(copying.root_roles(100).unwrap().transient, vec![130]);
        assert_eq!(tree.host_pid(12, "browser").unwrap(), 120);
    }

    #[test]
    fn root_roles_reject_strays_and_broken_ancestry() {
        // Not the unit's MainPID.
        assert!(launcher_tree().root_roles(101).is_err());
        // A second host-namespace process in the root cgroup.
        let mut stray = launcher_tree();
        stray.processes.insert(140, process("", &[140], 1));
        assert!(stray.root_roles(100).is_err());
        // Two relays.
        let mut doubled = launcher_tree();
        doubled.processes.insert(141, process("", &[141], 100));
        assert!(doubled.root_roles(100).is_err());
        // The init's parent is not the relay.
        let mut orphan = launcher_tree();
        orphan.processes.insert(102, process("", &[102, 1], 100));
        assert!(orphan.root_roles(100).is_err());
        // An instance process in the root cgroup that is not the init's child.
        let mut foreign = launcher_tree();
        foreign.processes.insert(150, process("", &[150, 30], 120));
        assert!(foreign.root_roles(100).is_err());
        // No relay at all (the old two-process shape).
        let mut flat = launcher_tree();
        flat.processes.remove(&101);
        flat.processes.insert(102, process("", &[102, 1], 100));
        assert!(flat.root_roles(100).is_err());
    }

    #[test]
    fn relay_namespaces_must_be_the_instance_s_except_its_own_pid_namespace() {
        let instance = Namespaces {
            user: "user:[2]".into(),
            mnt: "mnt:[3]".into(),
            pid: "pid:[4]".into(),
            ipc: "ipc:[5]".into(),
        };
        let relay = Namespaces {
            pid: "pid:[1]".into(),
            ..instance.clone()
        };
        check_relay_namespaces(&relay, "pid:[4]", "pid:[1]", &instance).unwrap();
        assert!(check_relay_namespaces(&relay, "pid:[1]", "pid:[1]", &instance).is_err());
        assert!(check_relay_namespaces(&relay, "pid:[4]", "pid:[9]", &instance).is_err());
        let host_user = Namespaces {
            user: "user:[1]".into(),
            ..relay.clone()
        };
        assert!(check_relay_namespaces(&host_user, "pid:[4]", "pid:[1]", &instance).is_err());
    }

    #[derive(Default)]
    struct FakeProc {
        fds: BTreeMap<u32, Result<Vec<String>, Missing>>,
        nets: BTreeMap<u32, Result<String, Missing>>,
        tables: BTreeMap<(u32, &'static str), Result<Option<String>, Missing>>,
    }

    impl ProcView for FakeProc {
        fn fd_targets(&self, pid: u32) -> Result<Vec<String>, Missing> {
            self.fds.get(&pid).cloned().unwrap_or(Err(Missing::Exited))
        }
        fn net_namespace(&self, pid: u32) -> Result<String, Missing> {
            self.nets.get(&pid).cloned().unwrap_or(Err(Missing::Exited))
        }
        fn table(&self, pid: u32, table: &str) -> Result<Option<String>, Missing> {
            self.tables
                .iter()
                .find(|((owner, name), _)| *owner == pid && *name == table)
                .map_or(Err(Missing::Exited), |(_, text)| text.clone())
        }
    }

    const TCP_HEADER: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n";

    fn quiet_namespace(proc: &mut FakeProc, pid: u32) {
        for table in ["tcp", "tcp6", "udp", "udp6"] {
            proc.tables
                .insert((pid, table), Ok(Some(TCP_HEADER.to_owned())));
        }
        proc.tables.insert(
            (pid, "unix"),
            Ok(Some(
                "Num RefCount Protocol Flags Type St Inode Path\n".to_owned(),
            )),
        );
    }

    fn one_socket_tree() -> FakeProc {
        let mut proc = FakeProc::default();
        proc.fds
            .insert(1, Ok(vec!["socket:[77]".into(), "/dev/null".into()]));
        proc.nets.insert(1, Ok("net:[9]".into()));
        proc.fds.insert(2, Ok(vec![]));
        proc.nets.insert(2, Ok("net:[9]".into()));
        quiet_namespace(&mut proc, 1);
        quiet_namespace(&mut proc, 2);
        proc
    }

    #[test]
    fn socket_scan_reads_every_namespace_and_detects_listeners() {
        scan_with(&one_socket_tree(), &[1, 2]).unwrap();
        let mut listening = one_socket_tree();
        listening.tables.insert(
            (1, "tcp"),
            Ok(Some(format!(
                "{TCP_HEADER}   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 77\n"
            ))),
        );
        assert!(scan_with(&listening, &[1, 2]).is_err());
        // IPv6 tables may be absent; IPv4 ones may not.
        let mut no_v6 = one_socket_tree();
        no_v6.tables.insert((1, "tcp6"), Ok(None));
        scan_with(&no_v6, &[1, 2]).unwrap();
        let mut no_v4 = one_socket_tree();
        no_v4.tables.insert((1, "tcp"), Ok(None));
        no_v4.tables.insert((2, "tcp"), Ok(None));
        assert!(scan_with(&no_v4, &[1, 2]).is_err());
    }

    #[test]
    fn socket_scan_fails_closed_on_missing_evidence_of_live_processes() {
        // A process that exited is skipped.
        let mut exited = one_socket_tree();
        exited.fds.insert(3, Err(Missing::Exited));
        scan_with(&exited, &[1, 2, 3]).unwrap();
        // Unreadable descriptors or namespace of a live process are not.
        let mut hidden = one_socket_tree();
        hidden
            .fds
            .insert(3, Err(Missing::Unreadable("EACCES".into())));
        assert!(scan_with(&hidden, &[1, 2, 3]).is_err());
        let mut no_net = one_socket_tree();
        no_net
            .nets
            .insert(2, Err(Missing::Unreadable("EACCES".into())));
        assert!(scan_with(&no_net, &[1, 2]).is_err());
        // A table unreadable through one member is read through another.
        let mut retry = one_socket_tree();
        retry
            .tables
            .insert((1, "udp"), Err(Missing::Unreadable("EACCES".into())));
        scan_with(&retry, &[1, 2]).unwrap();
        retry
            .tables
            .insert((2, "udp"), Err(Missing::Unreadable("EACCES".into())));
        assert!(scan_with(&retry, &[1, 2]).is_err());
        // Members that exited are skipped; the remaining one must answer.
        let mut gone = one_socket_tree();
        gone.tables.insert((1, "unix"), Err(Missing::Exited));
        scan_with(&gone, &[1, 2]).unwrap();
    }

    fn mask_line(id: u32, point: &str) -> String {
        format!(
            "{id} 1 0:{id} / {point} ro,nosuid,nodev,noexec,relatime - tmpfs tmpfs ro,size=4k,mode=755\n"
        )
    }

    #[test]
    fn sensitive_paths_must_be_covered_by_read_only_masks() {
        let mut text = String::from("1 0 259:1 / / rw,relatime - ext4 /dev/root rw\n");
        text.push_str(&mask_line(20, "/home"));
        text.push_str(&mask_line(21, "/run/user/1000"));
        text.push_str(&mask_line(22, "/srv/state"));
        let mounts = parse_mountinfo(&text);
        let paths = [
            PathBuf::from("/home"),
            PathBuf::from("/run/user/1000"),
            PathBuf::from("/srv/state"),
            // A state root below /home is covered by the /home mask.
            PathBuf::from("/home/me/.local/state/cowboy"),
        ];
        check_masks(&mounts, &paths).unwrap();
        assert!(check_masks(&mounts, &[PathBuf::from("/srv/other")]).is_err());
        // A writable tmpfs is not a mask.
        let writable = parse_mountinfo(&format!(
            "{text}30 1 0:30 / /srv/state rw,nosuid,nodev - tmpfs tmpfs rw,size=4k\n"
        ));
        assert!(check_masks(&writable, &paths).is_err());
        // A bind below a mask re-exposes host data.
        let exposed = parse_mountinfo(&format!(
            "{text}31 22 259:1 /srv/state/keys /srv/state/keys rw - ext4 /dev/root rw\n"
        ));
        assert!(check_masks(&exposed, &paths).is_err());
    }

    #[test]
    fn the_policy_bind_must_be_the_artifact_s_exact_file() {
        let mounts = parse_mountinfo(
            "40 1 259:13 /state/plugins/b/content/usr/share/cowboy-browser/policy.json /etc/opt/chrome/policies/managed/policy.json ro,nosuid,nodev - ext4 /dev/nvme0n1p2 rw\n",
        );
        let backing = (
            "259:13".to_owned(),
            "/state/plugins/b/content/usr/share/cowboy-browser/policy.json".to_owned(),
        );
        check_policy_bind(&mounts, "/etc/opt/chrome/policies/managed", &backing).unwrap();
        check_policy_bind(&mounts, "/etc/opt/chrome/policies/managed/", &backing).unwrap();
        let other = (
            "259:13".to_owned(),
            "/state/plugins/b/content/usr/share/cowboy-browser/other.json".to_owned(),
        );
        assert!(check_policy_bind(&mounts, "/etc/opt/chrome/policies/managed", &other).is_err());
        assert!(check_policy_bind(&mounts, "/etc/chromium/policies/managed", &backing).is_err());
    }
}
