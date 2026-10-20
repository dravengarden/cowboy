//! Hashed asset fallback across a Web release.
//!
//! A Web release atomically retargets the web root link while this process
//! keeps running. A browser that still holds the previous shell can only
//! lazy-load its own content-hashed chunks, and those vanished from the new
//! root. Remember every root target this process has observed and let a
//! missing `/assets/` file be served from an earlier one.

use std::path::{Component, Path, PathBuf};

use parking_lot::Mutex;

/// Targets remembered after the current one. Releases are rare, so a small
/// window covers every shell a client could still be running.
const RETAINED_ROOTS: usize = 4;

pub(super) struct WebRoots {
    link: PathBuf,
    /// Canonical targets, newest first.
    observed: Mutex<Vec<PathBuf>>,
}

impl WebRoots {
    pub(super) fn new(link: PathBuf) -> Self {
        let roots = Self {
            link,
            observed: Mutex::new(Vec::new()),
        };
        roots.observe();
        roots
    }

    /// Record the target before serving any successful document or asset, not
    /// just on a cache miss. Otherwise an intermediate release can serve a
    /// shell without ever entering the fallback history.
    pub(super) fn observe(&self) -> Option<PathBuf> {
        let current = std::fs::canonicalize(&self.link).ok()?;
        let mut observed = self.observed.lock();
        if observed.first() != Some(&current) {
            observed.retain(|root| root != &current);
            observed.insert(0, current.clone());
            observed.truncate(RETAINED_ROOTS + 1);
        }
        Some(current)
    }

    /// Earlier targets that may still hold a chunk the current root lost.
    pub(super) fn previous(&self) -> Vec<PathBuf> {
        let current = self.observe();
        self.observed
            .lock()
            .iter()
            .filter(|root| current.as_ref() != Some(*root))
            .cloned()
            .collect()
    }
}

/// Read an `assets/...` file from the releases observed before the current one.
pub(super) async fn previous_release_asset(roots: &WebRoots, requested: &str) -> Option<Vec<u8>> {
    let relative = Path::new(requested);
    if !relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return None;
    }
    for root in roots.previous() {
        if let Ok(bytes) = tokio::fs::read(root.join(relative)).await {
            return Some(bytes);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_retargeted_root_keeps_serving_the_previous_release_assets() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("release-a");
        let second = dir.path().join("release-b");
        std::fs::create_dir_all(first.join("assets")).unwrap();
        std::fs::create_dir_all(second.join("assets")).unwrap();
        std::fs::write(first.join("assets/Old-abc.js"), b"old chunk").unwrap();
        std::fs::write(second.join("assets/New-def.js"), b"new chunk").unwrap();
        let link = dir.path().join("cowboy-web");
        std::os::unix::fs::symlink(&first, &link).unwrap();
        let roots = WebRoots::new(link.clone());
        assert!(roots.previous().is_empty());
        assert!(
            previous_release_asset(&roots, "assets/Old-abc.js")
                .await
                .is_none(),
            "the current root serves its own files"
        );

        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&second, &link).unwrap();
        assert_eq!(
            roots.previous(),
            vec![std::fs::canonicalize(&first).unwrap()]
        );
        assert_eq!(
            previous_release_asset(&roots, "assets/Old-abc.js")
                .await
                .as_deref(),
            Some(&b"old chunk"[..])
        );
        assert!(
            previous_release_asset(&roots, "assets/Missing-000.js")
                .await
                .is_none()
        );
        assert!(
            previous_release_asset(&roots, "../release-a/assets/Old-abc.js")
                .await
                .is_none(),
            "only plain relative paths leave the root"
        );
    }
}
