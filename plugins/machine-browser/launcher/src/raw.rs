//! The crate's only `unsafe`: thin wrappers over the few syscalls rustix has
//! no safe form of (`fork`, `execve`, signal masks, `signalfd`, `_exit` and
//! bounding-set drops of capabilities newer than rustix knows). The launcher
//! is single-threaded and forks only from its own event loop, so the usual
//! fork hazards (locks held by other threads) do not arise, and a child runs
//! nothing but these wrappers and rustix calls before it execs or exits.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString};
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{FromRawFd as _, OwnedFd};

use rustix::process::Pid;
use rustix::thread::UnshareFlags;

pub enum Fork {
    Child,
    Parent(Pid),
}

/// `fork(2)`.
pub fn fork() -> io::Result<Fork> {
    // SAFETY: the launcher never starts a thread, so the child inherits a
    // consistent heap and no lock can be held by a thread that does not exist
    // in the child.
    let pid = unsafe { libc::fork() };
    match pid {
        -1 => Err(io::Error::last_os_error()),
        0 => Ok(Fork::Child),
        pid => Ok(Fork::Parent(
            Pid::from_raw(pid).expect("fork returned a positive pid"),
        )),
    }
}

/// `execve(2)`; returns only on failure.
pub fn execve(path: &CStr, argv: &[CString], envp: &[CString]) -> io::Error {
    let mut argv_pointers: Vec<*const libc::c_char> =
        argv.iter().map(|argument| argument.as_ptr()).collect();
    argv_pointers.push(std::ptr::null());
    let mut envp_pointers: Vec<*const libc::c_char> =
        envp.iter().map(|variable| variable.as_ptr()).collect();
    envp_pointers.push(std::ptr::null());
    // SAFETY: every pointer refers to a live, NUL-terminated `CString` owned by
    // the caller for the duration of the call, and both arrays end with NULL.
    unsafe {
        libc::execve(
            path.as_ptr(),
            argv_pointers.as_ptr(),
            envp_pointers.as_ptr(),
        )
    };
    io::Error::last_os_error()
}

/// Block `signals` for the calling thread (the only thread), so they queue
/// for a `signalfd` instead of running handlers.
pub fn block_signals(signals: &[i32]) -> io::Result<()> {
    let set = sigset(signals);
    // SAFETY: `set` is fully initialised; a null old-set pointer is permitted.
    let result =
        unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut()) };
    errno_result(result)
}

/// Clear the signal mask, before a child execs an executable that expects
/// the default disposition.
pub fn unblock_all_signals() -> io::Result<()> {
    let set = sigset(&[]);
    // SAFETY: `set` is an initialised empty set; a null old-set pointer is
    // permitted.
    let result =
        unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &raw const set, std::ptr::null_mut()) };
    errno_result(result)
}

/// A `signalfd` for `signals`, which must already be blocked.
pub fn signalfd(signals: &[i32], nonblocking: bool) -> io::Result<OwnedFd> {
    let set = sigset(signals);
    let mut flags = libc::SFD_CLOEXEC;
    if nonblocking {
        flags |= libc::SFD_NONBLOCK;
    }
    // SAFETY: `set` is initialised and `-1` asks for a new descriptor, which
    // nothing else references, so wrapping it in `OwnedFd` is sound.
    let fd = unsafe { libc::signalfd(-1, &raw const set, flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// `unshare(CLONE_NEWUSER | CLONE_NEWNS | CLONE_NEWPID | CLONE_NEWIPC)`: the
/// four namespaces of one instance. rustix marks every `unshare` unsafe
/// because `CLONE_FILES` and `CLONE_FS` would invalidate the process's view
/// of its descriptors; the namespace flags used here touch none of that.
pub fn unshare_instance_namespaces() -> io::Result<()> {
    // SAFETY: only namespace flags are passed (no `CLONE_FILES`, `CLONE_FS`
    // or `CLONE_VM`), and the process is single-threaded, so no thread can
    // observe a descriptor table or filesystem context it does not own.
    unsafe {
        rustix::thread::unshare_unsafe(
            UnshareFlags::NEWUSER
                | UnshareFlags::NEWNS
                | UnshareFlags::NEWPID
                | UnshareFlags::NEWIPC,
        )
    }
    .map_err(io::Error::from)
}

/// `prctl(PR_CAPBSET_DROP, capability)` for a capability number rustix's
/// `CapabilitySet` does not name.
pub fn drop_bounding_capability(capability: u32) -> io::Result<()> {
    // SAFETY: `PR_CAPBSET_DROP` takes one integer argument and touches no
    // memory; the unused arguments are zero as the manual requires.
    let result = unsafe {
        libc::prctl(
            libc::PR_CAPBSET_DROP,
            libc::c_ulong::from(capability),
            0,
            0,
            0,
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `_exit(2)`: leave a forked child without running the parent's `atexit`
/// handlers or flushing its buffers.
pub fn exit_now(code: i32) -> ! {
    // SAFETY: `_exit` never returns and has no preconditions.
    unsafe { libc::_exit(code) }
}

fn sigset(signals: &[i32]) -> libc::sigset_t {
    let mut set = MaybeUninit::<libc::sigset_t>::zeroed();
    // SAFETY: `sigemptyset` initialises the set and `sigaddset` only writes
    // into it; both are given a valid pointer to our own storage.
    unsafe {
        libc::sigemptyset(set.as_mut_ptr());
        for signal in signals {
            libc::sigaddset(set.as_mut_ptr(), *signal);
        }
        set.assume_init()
    }
}

fn errno_result(result: libc::c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result))
    }
}
