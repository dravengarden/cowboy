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

/// One cgroup's headroom: its limit (`memory.high`, else `memory.max`) minus
/// `memory.current`, plus the inactive file cache it can drop without
/// throttling. `None` when the cgroup is unlimited.
fn cgroup_headroom(high: &str, max: &str, current: &str, stat: &str) -> Option<u64> {
    let limit = [high, max]
        .into_iter()
        .find_map(|value| value.trim().parse::<u64>().ok())?;
    let current = current.trim().parse::<u64>().ok()?;
    let inactive_file = stat
        .lines()
        .find_map(|line| line.strip_prefix("inactive_file "))
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    Some(limit.saturating_sub(current).saturating_add(inactive_file))
}

/// The smallest headroom from the Agent worker slice (or, without one, the
/// user manager) up to the cgroup root.
pub(crate) fn memory_headroom_bytes() -> Option<u64> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let root = Path::new("/sys/fs/cgroup");
    let manager = root.join(user_manager_root(&cgroup)?.trim_start_matches('/'));
    let agents = manager.join("cowboy.slice/cowboy-agents.slice");
    let mut directory = if agents.is_dir() { agents } else { manager };
    let read = |directory: &Path, name: &str| std::fs::read_to_string(directory.join(name));
    let mut smallest: Option<u64> = None;
    while directory.starts_with(root) && directory != root {
        if let (Ok(high), Ok(max), Ok(current)) = (
            read(&directory, "memory.high"),
            read(&directory, "memory.max"),
            read(&directory, "memory.current"),
        ) && let Some(headroom) = cgroup_headroom(
            &high,
            &max,
            &current,
            &read(&directory, "memory.stat").unwrap_or_default(),
        ) {
            smallest = Some(smallest.map_or(headroom, |value| value.min(headroom)));
        }
        if !directory.pop() {
            break;
        }
    }
    smallest
}

/// Observe this host. `None` on platforms or hosts without the Linux sources.
pub(crate) fn sample(state_dir: &Path) -> Option<HostResources> {
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
