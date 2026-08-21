//! Build-time allocator policy and service-cgroup memory observability.

use std::path::PathBuf;

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// The controller's Nix release supplies this while compiling jemalloc. Keeping
/// the exact value in the Cowboy binary lets startup logs prove which policy an
/// immutable release was built with without adding a `mallctl` wrapper (and its
/// additional supply-chain surface) to the long-lived daemon.
#[must_use]
pub(crate) const fn compiled_malloc_conf() -> Option<&'static str> {
    option_env!("JEMALLOC_SYS_WITH_MALLOC_CONF")
}

/// `MALLOC_CONF` is applied by jemalloc after its compiled-in policy and can
/// therefore intentionally override it at launch. Surface a non-empty override
/// beside the compiled configuration so operators never diagnose the wrong
/// effective policy.
#[must_use]
pub(crate) fn runtime_malloc_conf_override() -> Option<String> {
    std::env::var("MALLOC_CONF")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Memory charged to Cowboy's complete service cgroup, including short-lived
/// usage collectors and any delegated child cgroups. This complements the
/// daemon-only RSS metric when diagnosing service-level startup peaks.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CgroupMemory {
    pub current_bytes: u64,
    pub peak_bytes: u64,
}

/// Current and lifetime-high cgroup-v2 memory, when Cowboy's own cgroup can be
/// resolved. Older kernels without `memory.peak` fall back to the current value.
#[must_use]
pub(crate) fn own_cgroup_memory() -> Option<CgroupMemory> {
    let raw = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let relative = raw.lines().find_map(|line| line.strip_prefix("0::"))?;
    let directory = PathBuf::from(CGROUP_ROOT).join(relative.trim_start_matches('/'));
    if !directory.is_dir() {
        return None;
    }
    let current_bytes = read_memory_counter(&directory.join("memory.current"))?;
    let peak_bytes = read_memory_counter(&directory.join("memory.peak")).unwrap_or(current_bytes);
    Some(CgroupMemory {
        current_bytes,
        peak_bytes,
    })
}

fn read_memory_counter(path: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}
