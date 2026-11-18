//! The crate's only `unsafe`: the Landlock syscalls rustix has no wrapper for
//! (`landlock_create_ruleset`, `landlock_restrict_self`). Everything else,
//! including `openat2`, goes through rustix's safe API.
#![allow(unsafe_code)]

use std::io;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};

/// `LANDLOCK_CREATE_RULESET_VERSION`: ask the kernel for its Landlock ABI.
const CREATE_RULESET_VERSION: libc::c_uint = 1;
/// `LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET` (ABI 6).
pub const SCOPE_ABSTRACT_UNIX_SOCKET: u64 = 1 << 0;
/// `LANDLOCK_SCOPE_SIGNAL` (ABI 6).
pub const SCOPE_SIGNAL: u64 = 1 << 1;

/// `struct landlock_ruleset_attr` as of ABI 6.
#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
    scoped: u64,
}

/// The running kernel's Landlock ABI version, or an error when Landlock is
/// unsupported or disabled.
pub fn landlock_abi() -> io::Result<u32> {
    // SAFETY: with the VERSION flag the kernel requires a null attribute
    // pointer and zero size, and reads no memory.
    let result = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<RulesetAttr>(),
            0_usize,
            CREATE_RULESET_VERSION,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    u32::try_from(result).map_err(|_| io::Error::other("Landlock ABI out of range"))
}

/// A ruleset that handles no filesystem or network access and only the given
/// IPC `scoped` bits.
pub fn scoped_ruleset(scoped: u64) -> io::Result<OwnedFd> {
    let attr = RulesetAttr {
        handled_access_fs: 0,
        handled_access_net: 0,
        scoped,
    };
    // SAFETY: `attr` is a live, fully initialised `landlock_ruleset_attr` of
    // exactly the size passed; flags are zero.
    let result = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &raw const attr,
            std::mem::size_of::<RulesetAttr>(),
            0_u32,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = i32::try_from(result).map_err(|_| io::Error::other("ruleset fd out of range"))?;
    // SAFETY: the kernel just returned this new descriptor; nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// `landlock_restrict_self(ruleset, 0)`; the caller must already have set
/// `PR_SET_NO_NEW_PRIVS`.
pub fn restrict_self(ruleset: &OwnedFd) -> io::Result<()> {
    // SAFETY: the descriptor is a live Landlock ruleset owned by the caller;
    // flags are zero and no memory is passed.
    let result =
        unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset.as_raw_fd(), 0_u32) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Take the single socket systemd passed by socket activation (`LISTEN_FDS`
/// = 1 for this process): descriptor 3.
pub fn take_listen_fd() -> io::Result<Option<OwnedFd>> {
    let for_us = std::env::var("LISTEN_PID")
        .ok()
        .and_then(|pid| pid.parse::<u32>().ok())
        == Some(std::process::id());
    if !for_us {
        return Ok(None);
    }
    match std::env::var("LISTEN_FDS").as_deref() {
        Ok("1") => {}
        _ => {
            return Err(io::Error::other(
                "expected exactly one socket-activated descriptor",
            ));
        }
    }
    // SAFETY: systemd hands this process descriptor 3 (`SD_LISTEN_FDS_START`)
    // and nothing in the process has claimed it; it is taken exactly once at
    // start-up.
    let fd = unsafe { OwnedFd::from_raw_fd(3) };
    rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::CLOEXEC)?;
    Ok(Some(fd))
}
