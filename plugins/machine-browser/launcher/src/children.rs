//! Step 3 of the launcher contract: forking a sealed child. Between `fork`
//! and `execve` the child closes nothing by hand (every descriptor the
//! launcher opens is `CLOEXEC`; the three it needs are `dup2`ed over the
//! standard ones), changes into `/mnt/instance`, enters its sub-cgroup,
//! performs the immediate `AppArmor` transition to the sealed profile,
//! clears the bounding set while it still holds `CAP_SETPCAP`, clears every
//! capability set, sets `no_new_privs` and only then execs.

use std::ffi::CString;
use std::io::Read as _;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use rustix::pipe::{PipeFlags, pipe_with};
use rustix::process::{Pid, WaitOptions, waitpid};
use rustix::stdio::{dup2_stderr, dup2_stdin, dup2_stdout};
use rustix::thread::{
    CapabilitySet, CapabilitySets, clear_ambient_capability_set,
    remove_capability_from_bounding_set, set_capabilities, set_no_new_privs,
};

use crate::raw;

/// What a child runs and where.
pub struct Spec<'a> {
    pub name: &'a str,
    /// Absolute path inside the instance (the artifact's `/usr` or `/opt`).
    pub program: &'a str,
    pub arguments: &'a [String],
    pub environment: &'a [(String, String)],
    pub working_directory: &'a str,
    /// Sub-cgroup the child joins before it is sealed.
    pub cgroup: Option<&'a Path>,
    pub stdin: BorrowedFd<'a>,
    pub stdout: BorrowedFd<'a>,
    pub stderr: BorrowedFd<'a>,
}

/// How a child is sealed.
#[derive(Debug, Clone, Copy)]
pub struct Sealing<'a> {
    /// `AppArmor` profile to transition to; `None` skips the transition.
    pub profile: Option<&'a str>,
    /// `/proc/sys/kernel/cap_last_cap`, for capabilities rustix cannot name.
    pub last_capability: u32,
}

/// Fork and exec `spec`. Returns once the child has exec'd; a failure before
/// exec is reported back through a `CLOEXEC` pipe and the child is reaped.
pub fn spawn(spec: &Spec<'_>, sealing: Sealing<'_>) -> Result<Pid> {
    let program = CString::new(spec.program).context("program path")?;
    let mut argv = vec![program.clone()];
    for argument in spec.arguments {
        argv.push(CString::new(argument.as_str()).context("argument")?);
    }
    let mut envp = Vec::new();
    for (key, value) in spec.environment {
        envp.push(CString::new(format!("{key}={value}")).context("environment")?);
    }
    let (read_end, write_end) = pipe_with(PipeFlags::CLOEXEC).context("status pipe")?;
    match raw::fork().context("fork")? {
        raw::Fork::Child => {
            drop(read_end);
            let failure = match prepare(spec, sealing) {
                Ok(()) => raw::execve(&program, &argv, &envp).to_string(),
                Err(error) => error.to_string(),
            };
            let _ = rustix::io::write(&write_end, failure.as_bytes());
            raw::exit_now(127)
        }
        raw::Fork::Parent(pid) => {
            drop(write_end);
            let mut message = String::new();
            let mut pipe: std::fs::File = read_end.into();
            let _ = pipe.read_to_string(&mut message);
            if message.is_empty() {
                return Ok(pid);
            }
            let _ = waitpid(Some(pid), WaitOptions::empty());
            bail!("{} could not start: {message}", spec.name)
        }
    }
}

/// Everything between fork and exec, in the child.
fn prepare(spec: &Spec<'_>, sealing: Sealing<'_>) -> Result<()> {
    raw::unblock_all_signals().context("resetting the signal mask")?;
    dup2_stdin(spec.stdin).context("stdin")?;
    dup2_stdout(spec.stdout).context("stdout")?;
    dup2_stderr(spec.stderr).context("stderr")?;
    rustix::process::chdir(spec.working_directory)
        .with_context(|| format!("entering {}", spec.working_directory))?;
    if let Some(cgroup) = spec.cgroup {
        crate::cgroup::add_current_process(cgroup)
            .with_context(|| format!("joining cgroup {}", cgroup.display()))?;
    }
    if let Some(profile) = sealing.profile {
        change_profile(profile)?;
    }
    for capability in CapabilitySet::all().iter() {
        match remove_capability_from_bounding_set(capability) {
            Ok(()) | Err(rustix::io::Errno::INVAL) => {}
            Err(error) => return Err(error).context("clearing the bounding set"),
        }
    }
    let known_last = CapabilitySet::all().bits().ilog2();
    for capability in (known_last + 1)..=sealing.last_capability {
        raw::drop_bounding_capability(capability)
            .with_context(|| format!("dropping capability {capability} from the bounding set"))?;
    }
    clear_ambient_capability_set().context("clearing the ambient set")?;
    set_capabilities(
        None,
        CapabilitySets {
            effective: CapabilitySet::empty(),
            permitted: CapabilitySet::empty(),
            inheritable: CapabilitySet::empty(),
        },
    )
    .context("clearing the capability sets")?;
    set_no_new_privs(true).context("setting no_new_privs")?;
    Ok(())
}

/// The immediate `aa_change_profile` transition through procfs.
fn change_profile(profile: &str) -> Result<()> {
    let command = format!("changeprofile {profile}");
    let attempt = |path: &str| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new().write(true).open(path)?;
        std::io::Write::write_all(&mut file, command.as_bytes())
    };
    match attempt("/proc/self/attr/apparmor/current") {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            attempt("/proc/self/attr/current")
                .with_context(|| format!("AppArmor transition to {profile}"))
        }
        Err(error) => Err(error).with_context(|| format!("AppArmor transition to {profile}")),
    }
}

/// `/proc/sys/kernel/cap_last_cap`, or the highest capability rustix knows
/// when the file is unreadable.
#[must_use]
pub fn last_capability() -> u32 {
    std::fs::read_to_string("/proc/sys/kernel/cap_last_cap")
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(CapabilitySet::all().bits().ilog2())
}

/// Open `/dev/null` for a child's stdin.
pub fn dev_null() -> Result<OwnedFd> {
    rustix::fs::open(
        "/dev/null",
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .context("opening /dev/null")
}

/// Open (append) a child's log file.
pub fn log_file(path: &str) -> Result<OwnedFd> {
    rustix::fs::open(
        path,
        rustix::fs::OFlags::WRONLY
            | rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::APPEND
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::from_raw_mode(0o600),
    )
    .with_context(|| format!("opening log {path}"))
}

/// Last `limit` bytes of a child's log, for the launcher's own stderr when
/// a child dies unexpectedly.
#[must_use]
pub fn log_tail(path: &str, limit: usize) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let start = bytes.len().saturating_sub(limit);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

/// Borrow helper for `Spec` fields.
pub fn borrow(fd: &OwnedFd) -> BorrowedFd<'_> {
    fd.as_fd()
}
