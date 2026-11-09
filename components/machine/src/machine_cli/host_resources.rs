//! Host resource observation reported to the Controller. Linux reads `/proc`,
//! the state directory's filesystem and the Agent worker slice; other
//! platforms report nothing rather than a misleading partial value.

use crate::machine_protocol::HostResources;
use std::path::Path;

/// `(total, available, swap_total, swap_free)` in bytes from `/proc/meminfo`.
fn parse_meminfo(text: &str) -> Option<(u64, u64, u64, u64)> {
    let field = |name: &str| -> Option<u64> {
        text.lines().find_map(|line| {
            let rest = line.strip_prefix(name)?.strip_prefix(':')?;
            let kib = rest.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()?;
            kib.checked_mul(1024)
        })
    };
    Some((
        field("MemTotal")?,
        field("MemAvailable")?,
        field("SwapTotal")?,
        field("SwapFree")?,
    ))
}

/// One-minute load average in thousandths from `/proc/loadavg`.
fn parse_load_1m_milli(text: &str) -> Option<u64> {
    let value: f64 = text.split_whitespace().next()?.parse().ok()?;
    (value.is_finite() && value >= 0.0).then(|| (value * 1000.0).round() as u64)
}

/// Whole seconds since boot from `/proc/uptime`.
fn parse_uptime_seconds(text: &str) -> Option<u64> {
    let value: f64 = text.split_whitespace().next()?.parse().ok()?;
    (value.is_finite() && value >= 0.0).then_some(value as u64)
}

/// The `user@<uid>.service` root of this process's cgroup v2 path, under which
/// systemd-user places the Agent worker slice.
fn user_manager_root(cgroup: &str) -> Option<&str> {
    let path = cgroup.lines().find_map(|line| line.strip_prefix("0::"))?;
    let end = path.find(".service/").filter(|index| {
        path[..*index]
            .rsplit('/')
            .next()
            .is_some_and(|unit| unit.starts_with("user@"))
    })?;
    Some(&path[..end + ".service".len()])
}

fn agent_memory_bytes() -> Option<u64> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let root = user_manager_root(&cgroup)?;
    let path = Path::new("/sys/fs/cgroup")
        .join(root.trim_start_matches('/'))
        .join("cowboy.slice/cowboy-agents.slice/memory.current");
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The tighter finite memory.high/memory.max limit.
fn cgroup_limit(high: &str, max: &str) -> Option<u64> {
    [high, max]
        .into_iter()
        .filter_map(|value| value.trim().parse::<u64>().ok())
        .min()
}

/// Headroom of one memory scope and the limit it is measured below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Headroom {
    bytes: u64,
    bound: u64,
}

impl Headroom {
    fn tighter(self, other: Self) -> Self {
        if other.bytes < self.bytes {
            other
        } else {
            self
        }
    }
}

/// Estimated headroom below the tighter finite high/max limit, discounting
/// inactive file cache. Cache is an estimate, not a guarantee of cheap reclaim.
/// Subtract cache from usage before saturating: usage may already exceed high.
fn cgroup_headroom(high: &str, max: &str, current: &str, stat: &str) -> Option<u64> {
    let limit = cgroup_limit(high, max)?;
    let current = current.trim().parse::<u64>().ok()?;
    let inactive_file = stat
        .lines()
        .find_map(|line| line.strip_prefix("inactive_file "))
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    Some(limit.saturating_sub(current.saturating_sub(inactive_file)))
}

/// The smallest headroom from the Agent worker slice (or, without one, the
/// user manager) up to the cgroup root.
fn cgroup_memory_headroom() -> Option<Headroom> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let root = Path::new("/sys/fs/cgroup");
    let manager = root.join(user_manager_root(&cgroup)?.trim_start_matches('/'));
    let agents = manager.join("cowboy.slice/cowboy-agents.slice");
    let mut directory = if agents.is_dir() { agents } else { manager };
    let read = |directory: &Path, name: &str| std::fs::read_to_string(directory.join(name));
    let mut smallest: Option<Headroom> = None;
    while directory.starts_with(root) && directory != root {
        if let (Ok(high), Ok(max), Ok(current)) = (
            read(&directory, "memory.high"),
            read(&directory, "memory.max"),
            read(&directory, "memory.current"),
        ) && let (Some(bytes), Some(bound)) = (
            cgroup_headroom(
                &high,
                &max,
                &current,
                &read(&directory, "memory.stat").unwrap_or_default(),
            ),
            cgroup_limit(&high, &max),
        ) {
            let headroom = Headroom { bytes, bound };
            smallest = Some(smallest.map_or(headroom, |value| value.tighter(headroom)));
        }
        if !directory.pop() {
            break;
        }
    }
    smallest
}

/// Both the host and its tightest ancestor constrain a cold launch. Hosts
/// without a finite cgroup still have a physical memory admission boundary.
pub fn memory_headroom_bytes() -> Option<u64> {
    let (total, available, _, _) = parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)?;
    let host = Headroom {
        bytes: available,
        bound: total,
    };
    let tightest = cgroup_memory_headroom().map_or(host, |cgroup| host.tighter(cgroup));
    Some(admission_headroom(tightest, memory_stalled()))
}

/// A stall counts as scarcity only while the tightest scope has less than
/// this fraction of its limit left.
const STALL_SCARCITY_DIVISOR: u64 = 4;

/// This is an admission estimate, not MemAvailable: while tasks stall because
/// the scope is nearly full there is no budget for another worker, and
/// reporting zero lets the Controller reclaim an idle worker before admission
/// retries. A stall with ample headroom has another cause, such as one
/// execution unit or background service at its own memory.high, refaulting
/// file cache or swap-in; hibernating idle sessions cannot relieve it.
fn admission_headroom(tightest: Headroom, stalled: bool) -> u64 {
    if stalled && tightest.bytes < tightest.bound / STALL_SCARCITY_DIVISOR {
        0
    } else {
        tightest.bytes
    }
}

fn full_pressure(text: &str) -> Option<f64> {
    let full = text.lines().find(|line| line.starts_with("full "))?;
    let value = full
        .split_whitespace()
        .find_map(|part| part.strip_prefix("avg10="))?
        .parse::<f64>()
        .ok()?;
    value.is_finite().then_some(value)
}

/// Whether a scope sits at its own finite `memory.high`, where the kernel
/// throttles that scope alone. `memory.current` hovers just below the limit
/// while it is reclaimed, so within 2% counts.
fn at_own_high(high: &str, current: &str) -> bool {
    match (high.trim().parse::<u64>(), current.trim().parse::<u64>()) {
        (Ok(high), Ok(current)) => current.saturating_mul(50) >= high.saturating_mul(49),
        _ => false,
    }
}

/// One execution unit below the Agent worker slice.
#[derive(Debug, Clone, Copy)]
struct UnitPressure {
    stalled: bool,
    at_own_high: bool,
}

/// Whether an observed stall means the Device is short of memory. Host and
/// ancestor PSI include every descendant, so one runaway unit throttled at
/// its own `memory.high` stalls them too: on Hawk on 2026-11-05 a 25 GiB
/// script held at its unit's 24 GiB limit kept host PSI "full" near 70 % and
/// refused every worker launch while other sessions had ample memory. Such a
/// unit cannot take memory from its neighbours, so its stall is local unless
/// a shared ancestor is at its own limit or another unit stalls as well.
fn stall_is_scarcity(stalled: bool, ancestor_at_high: bool, units: &[UnitPressure]) -> bool {
    if !stalled || ancestor_at_high {
        return stalled;
    }
    let throttled = units.iter().any(|unit| unit.stalled && unit.at_own_high);
    let shared = units.iter().any(|unit| unit.stalled && !unit.at_own_high);
    !throttled || shared
}

/// Pause cold allocations while memory stalls consume at least 1% of the
/// last ten seconds. Existing work continues; the launch queue stays bounded.
pub fn memory_stalled() -> bool {
    let read = |path: &Path| std::fs::read_to_string(path).unwrap_or_default();
    let stalled = |directory: &Path| {
        full_pressure(&read(&directory.join("memory.pressure"))).is_some_and(|full| full >= 1.0)
    };
    let mut any =
        full_pressure(&read(Path::new("/proc/pressure/memory"))).is_some_and(|full| full >= 1.0);
    let cgroup = read(Path::new("/proc/self/cgroup"));
    let Some(manager) = user_manager_root(&cgroup) else {
        return any;
    };
    let root = Path::new("/sys/fs/cgroup");
    let manager = root.join(manager.trim_start_matches('/'));
    let agents = manager.join("cowboy.slice/cowboy-agents.slice");
    let units: Vec<UnitPressure> = std::fs::read_dir(&agents)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .map(|unit| UnitPressure {
            stalled: stalled(&unit),
            at_own_high: at_own_high(
                &read(&unit.join("memory.high")),
                &read(&unit.join("memory.current")),
            ),
        })
        .collect();
    let mut ancestor_at_high = false;
    let mut directory = if agents.is_dir() { agents } else { manager };
    while directory.starts_with(root) && directory != root {
        any |= stalled(&directory);
        ancestor_at_high |= at_own_high(
            &read(&directory.join("memory.high")),
            &read(&directory.join("memory.current")),
        );
        if !directory.pop() {
            break;
        }
    }
    stall_is_scarcity(any, ancestor_at_high, &units)
}

/// Observe this host. `None` on platforms or hosts without the Linux sources.
pub fn sample(state_dir: &Path) -> Option<HostResources> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let (memory_total_bytes, memory_available_bytes, swap_total_bytes, swap_free_bytes) =
        parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)?;
    let disk = rustix::fs::statvfs(state_dir).ok()?;
    let block = disk.f_frsize;
    Some(HostResources {
        memory_total_bytes,
        memory_available_bytes,
        swap_total_bytes,
        swap_free_bytes,
        load_1m_milli: std::fs::read_to_string("/proc/loadavg")
            .ok()
            .and_then(|text| parse_load_1m_milli(&text))
            .unwrap_or(0),
        cpu_count: std::thread::available_parallelism()
            .map_or(1, |count| u32::try_from(count.get()).unwrap_or(u32::MAX)),
        disk_total_bytes: disk.f_blocks.saturating_mul(block),
        disk_available_bytes: disk.f_bavail.saturating_mul(block),
        agent_memory_bytes: agent_memory_bytes(),
        memory_headroom_bytes: memory_headroom_bytes(),
        uptime_seconds: std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|text| parse_uptime_seconds(&text))
            .unwrap_or(0),
        // The caller attaches the configured watermark.
        disk_low_watermark_bytes: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo_reports_bytes_including_swap() {
        let text = "MemTotal:       11951492 kB\nMemFree:          430000 kB\n\
                    MemAvailable:    7616704 kB\nSwapTotal:      20971516 kB\n\
                    SwapFree:       20971000 kB\n";
        assert_eq!(
            parse_meminfo(text),
            Some((
                11_951_492 * 1024,
                7_616_704 * 1024,
                20_971_516 * 1024,
                20_971_000 * 1024
            ))
        );
        assert_eq!(parse_meminfo("MemTotal: 1 kB\n"), None);
    }

    #[test]
    fn load_and_uptime_parse_their_first_field() {
        assert_eq!(
            parse_load_1m_milli("0.40 0.39 0.35 1/679 1350631\n"),
            Some(400)
        );
        assert_eq!(parse_load_1m_milli("nan 0 0"), None);
        assert_eq!(
            parse_uptime_seconds("454473.12 2700000.00\n"),
            Some(454_473)
        );
    }

    #[test]
    fn pressure_uses_full_stalls_not_busy_cpu_or_partial_stalls() {
        assert_eq!(
            full_pressure("some avg10=99.00\nfull avg10=0.15 avg60=2.00"),
            Some(0.15)
        );
        assert_eq!(full_pressure("full avg10=1.25 avg60=0.00"), Some(1.25));
        assert_eq!(full_pressure("full avg10=NaN"), None);
        assert_eq!(full_pressure(""), None);
    }

    #[test]
    fn cgroup_headroom_uses_the_tighter_limit_and_counts_dropable_cache() {
        let stat = "anon 7000\nfile 1300\ninactive_file 580\nactive_file 720\n";
        // memory.high wins over memory.max; inactive file cache is headroom.
        assert_eq!(
            cgroup_headroom("10200\n", "11274\n", "10106\n", stat),
            Some(674)
        );
        // Without memory.high the hard limit applies.
        assert_eq!(
            cgroup_headroom("max\n", "11274\n", "10106\n", stat),
            Some(1748)
        );
        // An unlimited cgroup contributes nothing; usage past a limit is zero.
        assert_eq!(cgroup_headroom("max\n", "max\n", "10106\n", stat), None);
        assert_eq!(cgroup_headroom("100\n", "max\n", "500\n", ""), Some(0));
        // Breaching high must not erase the overage before cache is credited.
        assert_eq!(
            cgroup_headroom("1000", "1200", "1100", "inactive_file 200"),
            Some(100)
        );
        assert_eq!(
            cgroup_headroom("1000", "1200", "1300", "inactive_file 200"),
            Some(0)
        );
        // A finite hard limit can be tighter than high.
        assert_eq!(cgroup_headroom("1200", "1000", "900", ""), Some(100));
        assert_eq!(
            cgroup_headroom("1000", "1200", "100", "inactive_file 200"),
            Some(1000)
        );
    }

    /// The OVH user slice that refused a Machine browser: a 768 MiB browser
    /// reserve is admitted beside an in-flight cold worker, while the old
    /// 2 GiB `MemoryMax` weight was not. Active file cache is not credited.
    #[test]
    fn observed_user_slice_admits_a_browser_reserve() {
        use crate::launch_ledger::Ledger;
        let stat = "anon 5500000000\ninactive_file 2370000000\nactive_file 1500000000\n";
        let bytes =
            cgroup_headroom("10200000000\n", "11270000000\n", "9590000000\n", stat).unwrap();
        assert_eq!(bytes, 2_980_000_000);
        let headroom = admission_headroom(
            Headroom {
                bytes,
                bound: 10_200_000_000,
            },
            false,
        );
        let worker = 512 << 20;
        assert!(Ledger::fits(headroom, worker, 768 << 20));
        assert!(!Ledger::fits(headroom, worker, 2 << 30));
    }

    #[test]
    fn a_stall_zeroes_headroom_only_when_the_tightest_scope_is_short() {
        const GIB: u64 = 1 << 30;
        // Hawk on 2026-10-29: the agent slice had 18 GiB of its 48 GiB left
        // while one session refaulted its own cache.
        let ample = Headroom {
            bytes: 18 * GIB,
            bound: 48 * GIB,
        };
        assert_eq!(admission_headroom(ample, true), 18 * GIB);
        // A slice at its memory.high that only credits inactive cache.
        let short = Headroom {
            bytes: 2 * GIB,
            bound: 48 * GIB,
        };
        assert_eq!(admission_headroom(short, true), 0);
        assert_eq!(admission_headroom(short, false), 2 * GIB);
        // Without a finite cgroup the host's MemAvailable and MemTotal apply.
        let host = Headroom {
            bytes: 2 * GIB,
            bound: 12 * GIB,
        };
        assert_eq!(admission_headroom(host, true), 0);
        assert_eq!(
            admission_headroom(host.tighter(ample), true),
            0,
            "the tighter scope decides"
        );
    }

    #[test]
    fn a_unit_throttled_at_its_own_high_does_not_starve_the_device() {
        let runaway = UnitPressure {
            stalled: true,
            at_own_high: true,
        };
        let idle = UnitPressure {
            stalled: false,
            at_own_high: false,
        };
        let starved = UnitPressure {
            stalled: true,
            at_own_high: false,
        };
        // Hawk on 2026-11-05: host PSI came only from the runaway unit.
        assert!(!stall_is_scarcity(true, false, &[runaway, idle]));
        // Another stalled unit or a shared ancestor at its limit is scarcity.
        assert!(stall_is_scarcity(true, false, &[runaway, starved]));
        assert!(stall_is_scarcity(true, true, &[runaway, idle]));
        // A stall no unit explains keeps the previous behaviour.
        assert!(stall_is_scarcity(true, false, &[idle]));
        assert!(stall_is_scarcity(true, false, &[]));
        assert!(!stall_is_scarcity(false, true, &[starved]));
        assert!(at_own_high("25769803776\n", "25700000000\n"));
        assert!(!at_own_high("25769803776\n", "20000000000\n"));
        assert!(!at_own_high("max\n", "25700000000\n"));
    }

    #[test]
    fn worker_slice_lives_under_the_user_manager() {
        let cgroup = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/\
                      cowboy-machine-svc-4e4d.service\n";
        assert_eq!(
            user_manager_root(cgroup),
            Some("/user.slice/user-1000.slice/user@1000.service")
        );
        assert_eq!(user_manager_root("0::/system.slice/cowboy.service\n"), None);
    }

    #[test]
    fn sample_observes_this_linux_host() {
        if cfg!(target_os = "linux") {
            let resources = sample(Path::new("/")).expect("linux host resources");
            assert!(resources.memory_total_bytes > 0);
            assert!(resources.memory_available_bytes <= resources.memory_total_bytes);
            assert!(resources.disk_total_bytes >= resources.disk_available_bytes);
            assert!(resources.cpu_count >= 1);
        }
    }
}
