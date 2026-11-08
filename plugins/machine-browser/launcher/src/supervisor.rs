//! Step 9 of the launcher contract. The process systemd starts stays the
//! unit's `MainPID`, forwards `SIGTERM` to the namespace init and exits with
//! its status. Between them sits one more relay: the process that unshares
//! the namespaces cannot itself become PID 1 of the new pid namespace, only
//! its next child can, so it forks the init and relays in the same way.
//!
//! The control socket is created here, in the host namespaces, before any
//! isolation step, so a failure at any stage (including `unshare` itself)
//! can still be reported to the Machine as `sandbox`, `host` or `internal`.

use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::fs::FileTypeExt as _;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use rustix::process::{
    Pid, Signal, WaitOptions, WaitStatus, getgid, getpid, getuid, kill_process,
    set_parent_process_death_signal, waitpid,
};

use crate::args::Args;
use crate::control::FailureKind;
use crate::{cgroup, init, raw};

pub const CONTROL_SOCKET: &str = "control.sock";
const SIGNALS: &[i32] = &[libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGCHLD];

/// Run one instance to completion; returns the exit status of the unit.
pub fn run(args: Args) -> u8 {
    let supervisor_pid = getpid().as_raw_nonzero().get();
    let cgroup_dir = match cgroup::unit_dir(args.cgroup_dir.as_deref()) {
        Ok(dir) => dir,
        Err(error) => {
            log("supervisor", &format!("{error:#}"));
            return FailureKind::Host.exit_code();
        }
    };
    let listener = match bind_control_socket(&args.control_dir) {
        Ok(listener) => listener,
        Err(error) => {
            log("supervisor", &format!("{error:#}"));
            return FailureKind::Host.exit_code();
        }
    };
    // Blocked before the fork so no signal can reach either process before
    // its signalfd exists; children restore the default mask before exec.
    if let Err(error) = raw::block_signals(SIGNALS) {
        log("supervisor", &format!("blocking signals: {error}"));
        return FailureKind::Internal.exit_code();
    }
    match raw::fork() {
        Err(error) => {
            log(
                "supervisor",
                &format!("forking the namespace relay: {error}"),
            );
            FailureKind::Internal.exit_code()
        }
        Ok(raw::Fork::Child) => namespace_relay(args, cgroup_dir, listener, supervisor_pid),
        Ok(raw::Fork::Parent(relay_pid)) => {
            drop(listener);
            relay(relay_pid, "supervisor")
        }
    }
}

/// `control.sock` in the unit's runtime directory, mode 0600.
fn bind_control_socket(directory: &Path) -> Result<UnixListener> {
    ensure!(
        std::fs::symlink_metadata(directory).is_ok_and(|metadata| metadata.is_dir()),
        "control directory {} is not a directory",
        directory.display()
    );
    let path = directory.join(CONTROL_SOCKET);
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        ensure!(
            metadata.file_type().is_socket(),
            "{} exists and is not a socket",
            path.display()
        );
        std::fs::remove_file(&path)
            .with_context(|| format!("removing stale {}", path.display()))?;
    }
    let previous = rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o077));
    let bound = UnixListener::bind(&path);
    rustix::process::umask(previous);
    let listener = bound.with_context(|| format!("binding {}", path.display()))?;
    listener
        .set_nonblocking(true)
        .context("control socket non-blocking")?;
    Ok(listener)
}

/// The process that creates the namespaces, writes the self-only maps,
/// forks the init and relays signals to it.
fn namespace_relay(
    args: Args,
    cgroup_dir: PathBuf,
    listener: UnixListener,
    supervisor_pid: i32,
) -> u8 {
    let prepared = (|| -> Result<(), (FailureKind, String)> {
        set_parent_process_death_signal(Some(Signal::KILL)).map_err(|error| {
            (
                FailureKind::Internal,
                format!("parent death signal: {error}"),
            )
        })?;
        // Read the ids before the unshare: inside the new, still unmapped user
        // namespace they read as the overflow id.
        let uid = getuid().as_raw();
        let gid = getgid().as_raw();
        raw::unshare_instance_namespaces().map_err(|error| {
            (
                FailureKind::Sandbox,
                format!("creating user, mount, pid and ipc namespaces: {error}"),
            )
        })?;
        write_self_mapping(uid, gid).map_err(|error| {
            (
                FailureKind::Sandbox,
                format!("writing the self-only id maps: {error}"),
            )
        })
    })();
    if let Err((kind, message)) = prepared {
        log("namespace relay", &message);
        init::deliver_fatal(&listener, kind, &message);
        return kind.exit_code();
    }
    match raw::fork() {
        Err(error) => {
            let message = format!("forking the init: {error}");
            log("namespace relay", &message);
            init::deliver_fatal(&listener, FailureKind::Internal, &message);
            FailureKind::Internal.exit_code()
        }
        Ok(raw::Fork::Child) => init::main(init::Context {
            args,
            cgroup_dir,
            listener,
            supervisor_pid,
        }),
        Ok(raw::Fork::Parent(init_pid)) => {
            drop(listener);
            relay(init_pid, "namespace relay")
        }
    }
}

/// `setgroups` denied, then one line mapping the Machine user's gid and uid
/// (as read in the parent namespace) to themselves; nothing inside is root
/// in any sense.
fn write_self_mapping(uid: u32, gid: u32) -> io::Result<()> {
    std::fs::write("/proc/self/setgroups", "deny")?;
    std::fs::write("/proc/self/gid_map", format!("{gid} {gid} 1"))?;
    std::fs::write("/proc/self/uid_map", format!("{uid} {uid} 1"))?;
    Ok(())
}

/// Forward termination signals to `child` and exit with its status.
fn relay(child: Pid, who: &str) -> u8 {
    let signals = match raw::signalfd(SIGNALS, false) {
        Ok(fd) => fd,
        Err(error) => {
            log(who, &format!("signalfd: {error}"));
            let _ = kill_process(child, Signal::KILL);
            return FailureKind::Internal.exit_code();
        }
    };
    loop {
        match read_signal(&signals) {
            Ok(libc::SIGCHLD) => match waitpid(Some(child), WaitOptions::NOHANG) {
                Ok(Some((_, status))) => return exit_code(status),
                Ok(None) => {}
                Err(error) => {
                    log(who, &format!("waiting for the child: {error}"));
                    return FailureKind::Internal.exit_code();
                }
            },
            Ok(_) => {
                let _ = kill_process(child, Signal::TERM);
            }
            Err(error) => {
                log(who, &format!("reading signals: {error}"));
                let _ = kill_process(child, Signal::KILL);
                return FailureKind::Internal.exit_code();
            }
        }
    }
}

/// One `signalfd_siginfo`, whose first field is the signal number.
pub fn read_signal(fd: &OwnedFd) -> io::Result<i32> {
    let mut buffer = [0_u8; 128];
    loop {
        match rustix::io::read(fd, &mut buffer) {
            Ok(read) if read >= 4 => {
                let number = u32::from_ne_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
                return i32::try_from(number)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "signal number"));
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short signalfd read",
                ));
            }
            Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

/// The unit's exit status for a child's wait status: its exit code, or
/// 128 plus the terminating signal.
#[must_use]
pub fn exit_code(status: WaitStatus) -> u8 {
    if let Some(code) = status.exit_status() {
        return u8::try_from(code & 0xff).unwrap_or(1);
    }
    status
        .terminating_signal()
        .and_then(|signal| u8::try_from(128 + signal).ok())
        .unwrap_or(1)
}

pub fn log(who: &str, message: &str) {
    eprintln!("cowboy-browser-launcher[{who}]: {message}");
}
