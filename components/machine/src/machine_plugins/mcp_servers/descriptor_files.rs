//! Content-addressed native descriptor files shared by every connection.
//!
//! A worker keeps the launch plan it started with across Controller restarts,
//! so the same profile and descriptor must resolve to the same path when a new
//! Controller connection prepares it again. The file is still private metadata:
//! it lives in a private directory, is replaced atomically and is removed when
//! its last receipt from any connection is released.
use std::collections::HashMap;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Weak};

use anyhow::{Context as _, Result, ensure};
use parking_lot::Mutex;
use sha2::{Digest as _, Sha256};

use crate::mcp_binding::{NativeDescriptor, ProcessProfile};

static FILES: LazyLock<Mutex<HashMap<PathBuf, Weak<DescriptorFile>>>> =
    LazyLock::new(Default::default);

pub(super) struct DescriptorFile {
    path: PathBuf,
}

impl DescriptorFile {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Reuse the live file for this exact content or write it at its stable
    /// path. `scope` separates independent stores in one process.
    pub(super) fn acquire(
        scope: &Path,
        profile: &ProcessProfile,
        descriptor: &NativeDescriptor,
    ) -> Result<Arc<Self>> {
        let bytes = serde_json::to_vec(descriptor)?;
        ensure!(bytes.len() <= 65536, "MCP descriptor too large");
        let mut identity = Sha256::new();
        for part in [
            scope.as_os_str().as_encoded_bytes(),
            &serde_json::to_vec(profile)?,
            &bytes,
        ] {
            identity.update((part.len() as u64).to_be_bytes());
            identity.update(part);
        }
        let directory = private_directory()?;
        let digest = format!("{:x}", identity.finalize());
        let path = directory.join(format!("cowboy-mcp-binding-{}.json", &digest[..32]));
        let mut files = FILES.lock();
        if let Some(live) = files.get(&path).and_then(Weak::upgrade) {
            return Ok(live);
        }
        let mut file = tempfile::Builder::new()
            .prefix(".cowboy-mcp-pending-")
            .tempfile_in(&directory)?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(&path).map_err(|error| error.error)?;
        let live = Arc::new(Self { path: path.clone() });
        files.insert(path, Arc::downgrade(&live));
        Ok(live)
    }
}

impl Drop for DescriptorFile {
    fn drop(&mut self) {
        let mut files = FILES.lock();
        // A newer owner of the same content keeps the file. Never upgrade here:
        // dropping that temporary could re-enter this lock.
        if files
            .get(&self.path)
            .is_some_and(|live| live.strong_count() > 0)
        {
            return;
        }
        files.remove(&self.path);
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A per-user `0700` directory below the process temporary directory, where
/// launched bridges already read descriptors. A foreign or widened directory
/// is refused rather than written into.
fn private_directory() -> Result<PathBuf> {
    let uid = rustix::process::geteuid().as_raw();
    let directory = std::env::temp_dir().join(format!("cowboy-mcp-descriptors-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error).context("MCP descriptor directory"),
    }
    let meta = std::fs::symlink_metadata(&directory)?;
    ensure!(
        meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0,
        "MCP descriptor directory is not private"
    );
    Ok(directory)
}
