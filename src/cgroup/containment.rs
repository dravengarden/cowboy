//! Fail-closed lifecycle ownership, not a sandbox against root or a same-user
//! process deliberately migrating itself. Persist the identity before spawn.
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf};

#[cfg(target_os = "linux")]
pub mod linux;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessOwner {
    pub boot: uuid::Uuid,
    pid: u32,
    pub start: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid_namespace: Option<u64>,
}

impl ProcessOwner {
    pub fn current() -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            linux::owner()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(io::Error::other("native process ownership unavailable"))
        }
    }

    pub fn exited(&self) -> io::Result<bool> {
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
pub struct ContainmentIdentity {
    pub owner: ProcessOwner,
    pub directory: PathBuf,
    pub device: u64,
    pub inode: u64,
    /// Bind absence observations to the original cgroupfs view. Legacy
    /// identities without it cannot establish same-boot path disappearance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_inode: Option<u64>,
}

impl ContainmentIdentity {
    pub fn owner(&self) -> &ProcessOwner {
        &self.owner
    }
    pub fn boot(&self) -> uuid::Uuid {
        self.owner.boot
    }

    /// Caller must own explicit stop authority for this exact launch. A missing
    /// group on the original boot is safe only because rmdir closes membership:
    /// no child can subsequently join through a previously opened descriptor.
    pub async fn terminate(&self) -> io::Result<()> {
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

pub struct Containment {
    pub identity: ContainmentIdentity,
    #[cfg(target_os = "linux")]
    directory: std::fs::File,
}

impl Containment {
    pub fn prepare(command: &mut tokio::process::Command) -> io::Result<Self> {
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

    pub fn identity(&self) -> &ContainmentIdentity {
        &self.identity
    }

    pub async fn finish(self) -> io::Result<()> {
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
