//! Best-effort cgroup-v2 containment for agent subprocesses.
//!
//! WHY: the agent (e.g. Claude Code) can `setsid`-detach shell commands into
//! their OWN sessions — an unbounded `until <cond>; do sleep N; done` poll loop
//! (the confirmed wedge trigger) keeps running long after its tool "completed".
//! Killing the agent process does NOT reap such a child: a process GROUP can't
//! catch it (setsid creates a new pgid) and walking `/proc` to SIGKILL arbitrary
//! detached sessions would overstep cowboy's boundary (it might hit something it
//! doesn't own). A cgroup CAN: membership is inherited by every descendant and
//! `setsid` does not escape it. So each agent runs in its own leaf cgroup, and
//! tearing the agent down writes `cgroup.kill` to SIGKILL the WHOLE subtree at
//! once — agent + every detached poll loop.
//!
//! Everything here is FAIL-OPEN: any error (no `Delegate=yes` on the unit, a
//! cgroup-v1 host, EACCES, the cgroup-v2 "no internal processes" rule) logs a
//! warning and degrades to None / a no-op. The agent still runs, just without
//! subtree reaping — so this is safe to ship before the unit gains `Delegate`,
//! and a misconfigured host loses the leak fix but nothing else.

use std::path::{Path, PathBuf};

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// The deterministic leaf-cgroup path for a session, or None if cowboy's own
/// cgroup can't be resolved (cgroup-v1 host / parse failure). Pure path math —
/// does NOT create anything — so a reader (e.g. the proc-watcher) can find an
/// agent's cgroup the same way [`create`] laid it out, without threading the
/// `PathBuf` through every caller.
#[must_use]
pub fn agent_dir(session_id: &str) -> Option<PathBuf> {
    Some(own_cgroup()?.join(format!("agent-{session_id}")))
}

/// Create a leaf cgroup for one agent under cowboy's own (delegated) cgroup and
/// return its absolute path, or None on any failure (fail-open).
///
/// No controllers are enabled on the leaf — it exists purely for grouping +
/// `cgroup.kill`, which keeps cowboy clear of the cgroup-v2 "no internal
/// processes" rule (cowboy's own process may stay in the parent).
#[must_use]
pub fn create(session_id: &str) -> Option<PathBuf> {
    let dir = agent_dir(session_id)?;
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(error = %e, dir = %dir.display(), "cgroup: create failed — agent runs uncontained");
        return None;
    }
    Some(dir)
}

/// Move a freshly-spawned agent PID — and thus every process it later forks —
/// into the agent's cgroup. Fail-open. Call this right after spawn, before the
/// agent has had a chance to fork its own children, so the whole future subtree
/// inherits membership.
pub fn add_pid(dir: &Path, pid: u32) {
    if let Err(e) = std::fs::write(dir.join("cgroup.procs"), pid.to_string()) {
        tracing::warn!(error = %e, pid, "cgroup: add_pid failed — agent runs uncontained");
    }
}

/// SIGKILL the whole agent subtree — the agent plus all descendants, including
/// `setsid`-detached ones — and remove the now-empty leaf. Idempotent / fail-open
/// — safe even if the agent already exited. Called from `agent_main` teardown so
/// a leaked child (e.g. an unbounded `until …; do sleep; done` poll loop) can't
/// outlive the agent.
pub fn kill_and_remove(dir: &Path) {
    write_kill(dir);
    // `cgroup.kill` is synchronous in killing, but the kernel may take a beat to
    // reap the zombies before the dir is removable. A few short retries cover it;
    // a leftover EMPTY dir is harmless (cleaned on the next teardown or restart).
    for _ in 0..20 {
        match std::fs::remove_dir(dir) {
            Ok(()) => return,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(25)),
        }
    }
    tracing::warn!(dir = %dir.display(), "cgroup: leaf not removable after kill (left empty)");
}

#[derive(Debug, Clone)]
pub struct Stats {
    pub memory_bytes: u64,
    pub pids: u64,
    pub cpu_usage_usec: u64,
}

/// Current resource use for one delegated agent subtree.
#[must_use]
pub fn stats(session_id: &str) -> Option<Stats> {
    let dir = agent_dir(session_id)?;
    let memory_bytes = read_number(&dir.join("memory.current"))?;
    let pids = read_number(&dir.join("pids.current"))?;
    let cpu = std::fs::read_to_string(dir.join("cpu.stat")).ok()?;
    let cpu_usage_usec = cpu
        .lines()
        .find_map(|line| line.strip_prefix("usage_usec ")?.parse::<u64>().ok())?;
    Some(Stats {
        memory_bytes,
        pids,
        cpu_usage_usec,
    })
}

fn read_number(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn write_kill(dir: &Path) {
    if let Err(e) = std::fs::write(dir.join("cgroup.kill"), "1") {
        // ENOENT just means it was already torn down — only warn on real errors.
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(error = %e, dir = %dir.display(), "cgroup: kill failed");
        }
    }
}

/// cowboy's own cgroup-v2 directory, parsed from `/proc/self/cgroup`
/// (`0::/<path>`). None if the host is cgroup-v1 / the parse fails / the dir is
/// absent — all of which fall through to running uncontained.
fn own_cgroup() -> Option<PathBuf> {
    let raw = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    // A cgroup-v2 host has exactly one line: `0::/system.slice/cowboy.service`.
    let rel = raw.lines().find_map(|l| l.strip_prefix("0::"))?;
    let dir = PathBuf::from(CGROUP_ROOT).join(rel.trim_start_matches('/'));
    dir.is_dir().then_some(dir)
}
