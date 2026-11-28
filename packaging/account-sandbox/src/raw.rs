//! The crate's only `unsafe`: the Landlock syscalls rustix has no wrapper for
//! (`landlock_create_ruleset`, `landlock_restrict_self`) and the loop device
//! and ext4 ioctls of online home growth (`crate::capacity`). Everything
//! else, including `openat2`, goes through rustix's safe API.
#![allow(unsafe_code)]

use std::io;
use std::os::fd::{AsRawFd as _, BorrowedFd, FromRawFd as _, OwnedFd};

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

/// `struct loop_info64` (`linux/loop.h`), field names as in C.
#[repr(C)]
#[allow(clippy::struct_field_names)]
pub struct LoopInfo64 {
    /// `dev_t` of the filesystem holding the backing file.
    pub lo_device: u64,
    /// Inode of the backing file.
    pub lo_inode: u64,
    lo_rdevice: u64,
    pub lo_offset: u64,
    pub lo_sizelimit: u64,
    lo_number: u32,
    lo_encrypt_type: u32,
    lo_encrypt_key_size: u32,
    lo_flags: u32,
    lo_file_name: [u8; 64],
    lo_crypt_name: [u8; 64],
    lo_encrypt_key: [u8; 32],
    lo_init: [u64; 2],
}

const LOOP_GET_STATUS64: u32 = 0x4C05;
const LOOP_SET_CAPACITY: u32 = 0x4C07;
/// `_IOR(0x12, 114, size_t)`.
const BLKGETSIZE64: u32 = 0x8008_1272;
/// `_IOW('f', 16, __u64)`.
const EXT4_IOC_RESIZE_FS: u32 = 0x4008_6610;

/// The request number in the C library's `ioctl` parameter type (`int` on
/// musl, `unsigned long` on glibc), bit for bit.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::useless_conversion
)]
fn request(code: u32) -> libc::Ioctl {
    code as libc::Ioctl
}

fn check(result: libc::c_int) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// `LOOP_GET_STATUS64` of the loop device `device`.
pub fn loop_status(device: BorrowedFd<'_>) -> io::Result<LoopInfo64> {
    let mut info = std::mem::MaybeUninit::<LoopInfo64>::zeroed();
    // SAFETY: `info` is a writable, zero-initialised `loop_info64` the
    // kernel fills completely on success.
    check(unsafe {
        libc::ioctl(
            device.as_raw_fd(),
            request(LOOP_GET_STATUS64),
            info.as_mut_ptr(),
        )
    })?;
    // SAFETY: every field is plain data and the struct started zeroed.
    Ok(unsafe { info.assume_init() })
}

/// `LOOP_SET_CAPACITY`: the loop device takes its backing file's length.
pub fn loop_set_capacity(device: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: the request takes no argument.
    check(unsafe { libc::ioctl(device.as_raw_fd(), request(LOOP_SET_CAPACITY), 0) })
}

/// `BLKGETSIZE64`: the block device's size in bytes.
pub fn block_device_bytes(device: BorrowedFd<'_>) -> io::Result<u64> {
    let mut bytes = 0_u64;
    // SAFETY: the kernel writes one `u64` to the valid pointer.
    check(unsafe { libc::ioctl(device.as_raw_fd(), request(BLKGETSIZE64), &raw mut bytes) })?;
    Ok(bytes)
}

/// `EXT4_IOC_RESIZE_FS` on a descriptor of the mounted filesystem: grow it
/// online to `blocks` blocks. The kernel refuses to shrink and does nothing
/// for the current size.
pub fn ext4_resize(filesystem: BorrowedFd<'_>, blocks: u64) -> io::Result<()> {
    // SAFETY: the kernel reads one `u64` from the valid pointer.
    check(unsafe {
        libc::ioctl(
            filesystem.as_raw_fd(),
            request(EXT4_IOC_RESIZE_FS),
            &raw const blocks,
        )
    })
}
