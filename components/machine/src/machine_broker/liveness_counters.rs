//! Cgroup v2 and pressure counters for heartbeat-gap attribution
//! (`liveness.rs`). Linux reads go through `openat2` beneath validated
//! cgroup2 and proc roots; other platforms report every value unavailable.

use std::time::Instant;

/// Logged in place of a value that could not be read.
pub(super) const UNAVAILABLE: i64 = -1;
#[cfg(target_os = "linux")]
const MAX_COUNTER_FILE: u64 = 4096;

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Psi {
    pub(super) some_us: u64,
    pub(super) full_us: Option<u64>,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Events {
    pub(super) high: u64,
    pub(super) max: u64,
    pub(super) oom: u64,
    pub(super) oom_kill: u64,
}

/// One read of a scope's counters. `None` is a field that could not be read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Sample {
    pub(super) cpu: Option<Psi>,
    pub(super) memory: Option<Psi>,
    pub(super) io: Option<Psi>,
    pub(super) events: Option<Events>,
    pub(super) events_local: Option<Events>,
    pub(super) current: Option<u64>,
    /// `u64::MAX` is `max` (no limit).
    pub(super) high: Option<u64>,
    pub(super) limit: Option<u64>,
    /// Device and inode of the cgroup directory read, so a cgroup recreated
    /// at the same path is never compared with its predecessor.
    pub(super) identity: Option<(u64, u64)>,
}

impl Sample {
    /// Whether any counter could be read (a removed cgroup reads nothing).
    pub(super) fn readable(&self) -> bool {
        self.cpu.is_some() || self.memory.is_some() || self.io.is_some() || self.current.is_some()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Delta {
    pub(super) cpu: (i64, i64),
    pub(super) memory: (i64, i64),
    pub(super) io: (i64, i64),
    pub(super) events: (i64, i64, i64, i64),
    pub(super) events_local: (i64, i64),
}

pub(super) fn saturate(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Counter growth from `before` to `after`; a field missing on either side is
/// [`UNAVAILABLE`].
pub(super) fn delta(before: &Sample, after: &Sample) -> Delta {
    let diff = |before: Option<u64>, after: Option<u64>| match (before, after) {
        (Some(before), Some(after)) => saturate(after.saturating_sub(before)),
        _ => UNAVAILABLE,
    };
    let psi = |before: Option<Psi>, after: Option<Psi>| {
        (
            diff(before.map(|p| p.some_us), after.map(|p| p.some_us)),
            diff(
                before.and_then(|p| p.full_us),
                after.and_then(|p| p.full_us),
            ),
        )
    };
    let event = |pick: fn(&Events) -> u64, local: bool| {
        let (before, after) = if local {
            (before.events_local, after.events_local)
        } else {
            (before.events, after.events)
        };
        diff(before.as_ref().map(pick), after.as_ref().map(pick))
    };
    Delta {
        cpu: psi(before.cpu, after.cpu),
        memory: psi(before.memory, after.memory),
        io: psi(before.io, after.io),
        events: (
            event(|e| e.high, false),
            event(|e| e.max, false),
            event(|e| e.oom, false),
            event(|e| e.oom_kill, false),
        ),
        events_local: (event(|e| e.high, true), event(|e| e.max, true)),
    }
}

/// The latest sample at or before `since`, else the oldest retained one, which
/// the record reports as partial coverage.
pub(super) fn baseline<T>(
    ring: &std::collections::VecDeque<(Instant, T)>,
    since: Instant,
) -> Option<&(Instant, T)> {
    ring.iter()
        .rev()
        .find(|(at, _)| *at <= since)
        .or_else(|| ring.front())
}

/// `some ... total=N` and optional `full ... total=N` lines.
// Read only by the Linux counters (and tests) on other platforms.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn parse_psi(text: &str) -> Option<Psi> {
    let total = |prefix: &str| {
        text.lines()
            .find(|line| line.starts_with(prefix))
            .and_then(|line| {
                line.split_whitespace()
                    .find_map(|field| field.strip_prefix("total="))
            })
            .and_then(|value| value.parse().ok())
    };
    Some(Psi {
        some_us: total("some ")?,
        full_us: total("full "),
    })
}

// Read only by the Linux counters (and tests) on other platforms.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn parse_events(text: &str) -> Option<Events> {
    let mut events = Events::default();
    let mut any = false;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let Ok(value) = value.trim().parse() else {
            continue;
        };
        let slot = match key {
            "high" => &mut events.high,
            "max" => &mut events.max,
            "oom" => &mut events.oom,
            "oom_kill" => &mut events.oom_kill,
            _ => continue,
        };
        *slot = value;
        any = true;
    }
    any.then_some(events)
}

// Read only by the Linux counters (and tests) on other platforms.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn parse_limit(text: &str) -> Option<u64> {
    match text.trim() {
        "max" => Some(u64::MAX),
        value => value.parse().ok(),
    }
}

/// A cgroup v2 path as the kernel reports it (`/a/b.slice/c.service`),
/// relative to the mount. Refuses the root, empty segments, `.` and `..`.
pub(super) fn relative_cgroup(control_group: &str) -> Option<String> {
    let relative = control_group.strip_prefix('/')?;
    if relative.is_empty()
        || relative.len() > 4096
        || relative
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return None;
    }
    Some(relative.to_owned())
}

/// Every strict ancestor of a relative cgroup path below the root, nearest
/// first.
pub(super) fn ancestors(relative: &str) -> Vec<String> {
    let mut chain = Vec::new();
    let mut path = relative;
    while let Some((parent, _)) = path.rsplit_once('/') {
        chain.push(parent.to_owned());
        path = parent;
    }
    chain
}

/// The cgroup v2 entry (`0::/path`) of a `/proc/<pid>/cgroup` file.
// Read only by the Linux counters (and tests) on other platforms.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn unified_cgroup(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("0::"))
        .map(str::to_owned)
}

pub(super) use sys::{
    CgroupDir, PeerHandle, broker_cgroup, open_cgroup, open_peer, peer_member, read_cgroup,
    read_host, read_path,
};

#[cfg(target_os = "linux")]
mod sys {
    use super::{
        MAX_COUNTER_FILE, Sample, parse_events, parse_limit, parse_psi, relative_cgroup,
        unified_cgroup,
    };
    use rustix::fd::{AsFd, OwnedFd};
    use rustix::fs::{Mode, OFlags, ResolveFlags, fstatfs, open, openat2};
    use std::io::{self, Read as _};

    const CGROUP2_SUPER_MAGIC: i64 = 0x6367_7270;
    const PROC_SUPER_MAGIC: i64 = 0x9fa0;
    const DIRECTORY: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);
    const BENEATH: ResolveFlags = ResolveFlags::BENEATH
        .union(ResolveFlags::NO_SYMLINKS)
        .union(ResolveFlags::NO_MAGICLINKS)
        .union(ResolveFlags::NO_XDEV);

    /// A `/proc/<pid>` directory handle opened while the connection is being
    /// admitted. Reads through it fail once that process exits, so a reused
    /// PID can never be described by it.
    pub(in super::super) type PeerHandle = OwnedFd;
    /// A pinned cgroup directory: a recreated unit has a new directory, and
    /// reads through a removed one fail rather than describe the replacement.
    pub(in super::super) type CgroupDir = OwnedFd;

    fn root(path: &str, magic: i64) -> io::Result<OwnedFd> {
        let fd = open(path, DIRECTORY, Mode::empty())?;
        #[allow(clippy::unnecessary_cast)]
        let found = fstatfs(&fd)?.f_type as i64;
        if found != magic {
            return Err(io::Error::other("unexpected filesystem"));
        }
        Ok(fd)
    }

    pub(in super::super) fn read_at<Fd: AsFd>(directory: Fd, relative: &str) -> io::Result<String> {
        let fd = openat2(
            directory,
            relative,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            BENEATH,
        )?;
        let mut text = String::new();
        std::fs::File::from(fd)
            .take(MAX_COUNTER_FILE)
            .read_to_string(&mut text)?;
        Ok(text)
    }

    fn open_dir_at<Fd: AsFd>(directory: Fd, relative: &str) -> io::Result<OwnedFd> {
        Ok(openat2(
            directory,
            relative,
            DIRECTORY,
            Mode::empty(),
            BENEATH,
        )?)
    }

    pub(in super::super) fn open_peer(pid: u32) -> Option<PeerHandle> {
        let proc = root("/proc", PROC_SUPER_MAGIC).ok()?;
        open_dir_at(&proc, &pid.to_string()).ok()
    }

    /// Whether the pinned peer still lives in `control_group` or below it.
    pub(in super::super) fn peer_member(
        peer: &PeerHandle,
        control_group: &str,
    ) -> Result<(), &'static str> {
        let text = read_at(peer, "cgroup").map_err(|_| "peer_exited")?;
        let path = unified_cgroup(&text).ok_or("peer_cgroup_unknown")?;
        if path == control_group || path.starts_with(&format!("{control_group}/")) {
            Ok(())
        } else {
            Err("peer_outside_unit")
        }
    }

    pub(in super::super) fn open_cgroup(control_group: &str) -> io::Result<CgroupDir> {
        let relative =
            relative_cgroup(control_group).ok_or_else(|| io::Error::other("invalid cgroup"))?;
        open_dir_at(root("/sys/fs/cgroup", CGROUP2_SUPER_MAGIC)?, &relative)
    }

    /// The broker's own cgroup, read through the validated proc root.
    pub(in super::super) fn broker_cgroup() -> Option<String> {
        let proc = root("/proc", PROC_SUPER_MAGIC).ok()?;
        let text = read_at(&proc, &format!("{}/cgroup", std::process::id())).ok()?;
        relative_cgroup(&unified_cgroup(&text)?)
    }

    pub(in super::super) fn counters<Fd: AsFd + Copy>(directory: Fd) -> Sample {
        let file = |name: &str| read_at(directory, name).ok();
        Sample {
            cpu: file("cpu.pressure").as_deref().and_then(parse_psi),
            memory: file("memory.pressure").as_deref().and_then(parse_psi),
            io: file("io.pressure").as_deref().and_then(parse_psi),
            events: file("memory.events").as_deref().and_then(parse_events),
            events_local: file("memory.events.local")
                .as_deref()
                .and_then(parse_events),
            current: file("memory.current").as_deref().and_then(parse_limit),
            high: file("memory.high").as_deref().and_then(parse_limit),
            limit: file("memory.max").as_deref().and_then(parse_limit),
            identity: rustix::fs::fstat(directory)
                .ok()
                .map(|stat| (stat.st_dev, stat.st_ino)),
        }
    }

    pub(in super::super) fn read_cgroup(directory: &CgroupDir) -> Sample {
        counters(directory)
    }

    /// Counters of a shared ancestor by relative path.
    pub(in super::super) fn read_path(relative: &str) -> Sample {
        root("/sys/fs/cgroup", CGROUP2_SUPER_MAGIC)
            .and_then(|root| open_dir_at(root, relative))
            .map(|directory| counters(&directory))
            .unwrap_or_default()
    }

    pub(in super::super) fn read_host() -> Sample {
        let Ok(proc) = root("/proc", PROC_SUPER_MAGIC) else {
            return Sample::default();
        };
        let file = |name: &str| read_at(&proc, &format!("pressure/{name}")).ok();
        Sample {
            cpu: file("cpu").as_deref().and_then(parse_psi),
            memory: file("memory").as_deref().and_then(parse_psi),
            io: file("io").as_deref().and_then(parse_psi),
            ..Sample::default()
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod sys {
    use super::Sample;
    use std::io;

    pub(in super::super) type PeerHandle = std::os::fd::OwnedFd;
    pub(in super::super) type CgroupDir = std::os::fd::OwnedFd;

    pub(in super::super) fn open_peer(_pid: u32) -> Option<PeerHandle> {
        None
    }

    pub(in super::super) fn peer_member(
        _peer: &PeerHandle,
        _control_group: &str,
    ) -> Result<(), &'static str> {
        Err("unsupported_platform")
    }

    pub(in super::super) fn open_cgroup(_control_group: &str) -> io::Result<CgroupDir> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    pub(in super::super) fn broker_cgroup() -> Option<String> {
        None
    }

    pub(in super::super) fn read_cgroup(_directory: &CgroupDir) -> Sample {
        Sample::default()
    }

    pub(in super::super) fn read_path(_relative: &str) -> Sample {
        Sample::default()
    }

    pub(in super::super) fn read_host() -> Sample {
        Sample::default()
    }
}

#[cfg(all(test, target_os = "linux"))]
pub(super) use sys::{counters, read_at};
