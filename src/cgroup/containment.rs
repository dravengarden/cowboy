//! Fail-closed lifecycle ownership, not a sandbox against root or a same-user
//! process deliberately migrating itself. Persist the identity before spawn.
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(all(test, target_os = "linux"))]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessOwner {
    boot: uuid::Uuid,
    pid: u32,
    start: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pid_namespace: Option<u64>,
}

impl ProcessOwner {
    pub(crate) fn current() -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            linux::owner()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(io::Error::other("native process ownership unavailable"))
        }
    }

    pub(crate) fn exited(&self) -> io::Result<bool> {
        #[cfg(target_os = "linux")]
        {
            linux::owner_exited(self)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(io::Error::other("native process ownership unavailable"))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContainmentIdentity {
    owner: ProcessOwner,
    directory: PathBuf,
    device: u64,
    inode: u64,
    /// Bind absence observations to the original cgroupfs view. Legacy
    /// identities without it cannot establish same-boot path disappearance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_inode: Option<u64>,
}

impl ContainmentIdentity {
    pub(crate) fn owner(&self) -> &ProcessOwner {
        &self.owner
    }
    pub(crate) fn boot(&self) -> uuid::Uuid {
        self.owner.boot
    }

    /// Caller must own explicit stop authority for this exact launch. A missing
    /// group on the original boot is safe only because rmdir closes membership:
    /// no child can subsequently join through a previously opened descriptor.
    pub(crate) async fn terminate(&self) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            linux::terminate(self).await
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(io::Error::other("native process containment unavailable"))
        }
    }
}

pub(crate) struct Containment {
    identity: ContainmentIdentity,
    #[cfg(target_os = "linux")]
    directory: std::fs::File,
}

impl Containment {
    pub(crate) fn prepare(command: &mut tokio::process::Command) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            linux::prepare(command)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = command;
            Err(io::Error::other("native process containment unavailable"))
        }
    }

    pub(crate) fn identity(&self) -> &ContainmentIdentity {
        &self.identity
    }

    pub(crate) async fn finish(self) -> io::Result<()> {
        self.identity.terminate().await
    }
}

impl Drop for Containment {
    fn drop(&mut self) {
        // Only signal the opened original group. Aborting a future is not exit
        // evidence, and a replacement at this path must never receive a kill.
        #[cfg(target_os = "linux")]
        {
            let _ = linux::kill(&self.directory);
            let _ = linux::remove(&self.identity, &self.directory);
        }
    }
}
