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
        uptime_seconds: std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|text| parse_uptime_seconds(&text))
            .unwrap_or(0),
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
