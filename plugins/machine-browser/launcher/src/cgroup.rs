//! The unit's cgroup (`Delegate=yes`) and the two sub-cgroups the launcher
//! creates in it: `display/` for the display server, window manager and
//! their helpers, `browser/` for every Chromium process, so the Machine can
//! freeze the browser alone and the launcher can tell when a graceful stop
//! has drained it.

use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use rustix::process::{Pid, Signal, kill_process};

pub const ROOT: &str = "/sys/fs/cgroup";

/// The unit's cgroup directory: the explicit `--cgroup-dir`, or the cgroup
/// `/proc/self/cgroup` places the launcher in.
pub fn unit_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    let dir = if let Some(path) = explicit {
        path.to_path_buf()
    } else {
        let content =
            std::fs::read_to_string("/proc/self/cgroup").context("reading /proc/self/cgroup")?;
        let relative = crate::procinfo::cgroup_path(&content)
            .context("/proc/self/cgroup has no unified hierarchy entry")?;
        Path::new(ROOT).join(relative.trim_start_matches('/'))
    };
    ensure!(
        dir.join("cgroup.procs").is_file(),
        "{} is not a cgroup directory",
        dir.display()
    );
    Ok(dir)
}

/// Create (or reuse) the sub-cgroup `name` below `unit`.
pub fn ensure_child(unit: &Path, name: &str) -> Result<PathBuf> {
    let dir = unit.join(name);
    match std::fs::create_dir(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error).with_context(|| format!("creating cgroup {}", dir.display()));
        }
    }
    ensure!(
        dir.join("cgroup.procs").is_file(),
        "{} is not a cgroup directory",
        dir.display()
    );
    Ok(dir)
}

/// Direct member pids of `dir`.
pub fn procs(dir: &Path) -> Result<Vec<i32>> {
    let content = std::fs::read_to_string(dir.join("cgroup.procs"))
        .with_context(|| format!("reading {}/cgroup.procs", dir.display()))?;
    Ok(content
        .split_whitespace()
        .filter_map(|pid| pid.parse().ok())
        .collect())
}

/// Whether `dir` and every descendant cgroup hold no process.
pub fn is_empty(dir: &Path) -> Result<bool> {
    let events = std::fs::read_to_string(dir.join("cgroup.events"))
        .with_context(|| format!("reading {}/cgroup.events", dir.display()))?;
    Ok(events.lines().any(|line| line.trim() == "populated 0"))
}

/// `SIGKILL` everything in `dir` and below: through `cgroup.kill` when the
/// kernel offers it, otherwise by signalling each listed pid.
pub fn kill_all(dir: &Path) -> Result<()> {
    let kill = dir.join("cgroup.kill");
    if kill.is_file() {
        return std::fs::write(&kill, "1").with_context(|| format!("writing {}", kill.display()));
    }
    for pid in procs(dir)? {
        if let Some(pid) = Pid::from_raw(pid) {
            let _ = kill_process(pid, Signal::KILL);
        }
    }
    Ok(())
}

/// Move the calling process into `dir`. Used by a forked child before it
/// seals itself.
pub fn add_current_process(dir: &Path) -> io::Result<()> {
    let pid = rustix::process::getpid().as_raw_nonzero().get();
    std::fs::write(dir.join("cgroup.procs"), pid.to_string())
}
