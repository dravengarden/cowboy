//! Unattended adoption of published Catalog bytes.
//!
//! Publication is already unauthenticated and signature-anchored: a publisher
//! hard-links immutable artifacts, then links the release envelope last as the
//! Catalog commit marker, and [`load_catalog_root`] verifies every package
//! against the trusted publisher keyring before it grants any identity. A
//! concurrent reader therefore sees either the old Catalog or one complete
//! immutable release.
//!
//! The Controller consequently never needed an operator session to *notice* a
//! release — only to be told the directory changed. This watcher supplies that
//! signal so a published version stops waiting for a human to open the admin
//! surface. It grants no authority of its own: it re-runs exactly the refresh
//! that startup and the admin endpoint already run.
//!
//! [`load_catalog_root`]: super::load_catalog_root

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use notify::{EventKind, RecursiveMode, Watcher as _};

use super::PluginCatalog;

/// Collapse one publication burst — artifacts, package, host bundle, envelope —
/// into a single refresh instead of re-staging the whole Catalog per file.
const SETTLE: Duration = Duration::from_millis(750);

/// A refresh can fail for reasons unrelated to the write that woke us: host
/// staging, a required migration, a transient storage error. Retry on a bounded
/// backoff so a published release is not stranded until the next unrelated
/// directory write. Failure never clears the visible snapshot.
const RETRY_BACKOFF: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(15),
    Duration::from_secs(60),
];

/// Dropping this stops the watch. Held by the server for its whole lifetime.
pub(crate) struct CatalogWatcher {
    _watchers: Vec<notify::RecommendedWatcher>,
}

/// Watch every Catalog root and re-run the ordinary refresh when it changes.
///
/// # Errors
/// Returns when a root cannot be watched. The caller may continue without
/// unattended adoption; the admin refresh endpoint and restart still work.
pub(crate) fn spawn(
    catalog: Arc<PluginCatalog>,
    storage: crate::plugin_storage::PluginStorage,
    providers: Arc<crate::provider_catalog::ProviderCatalog>,
) -> Result<CatalogWatcher> {
    let (watchers, mut receiver) = watch_roots(&catalog.roots())?;

    tokio::spawn(async move {
        while receiver.recv().await.is_some() {
            settle(&mut receiver).await;
            let mut backoff = RETRY_BACKOFF.iter();
            loop {
                match adopt(&catalog, &storage, &providers).await {
                    Ok(count) => {
                        tracing::info!(external_releases = count, "adopted the published Catalog");
                        break;
                    }
                    Err(error) => {
                        let Some(delay) = backoff.next() else {
                            tracing::error!(
                                %error,
                                "keeping the previous Catalog; adoption needs an explicit refresh"
                            );
                            break;
                        };
                        tracing::warn!(%error, ?delay, "Catalog adoption failed; retrying");
                        tokio::time::sleep(*delay).await;
                    }
                }
            }
        }
    });

    Ok(CatalogWatcher {
        _watchers: watchers,
    })
}

/// Wire every existing root to one wake-up channel.
///
/// An absent root is skipped rather than rejected: the legacy compatibility
/// directory is frequently missing, and publication creates the canonical one
/// on demand.
fn watch_roots(
    roots: &[PathBuf],
) -> Result<(
    Vec<notify::RecommendedWatcher>,
    tokio::sync::mpsc::Receiver<()>,
)> {
    // Depth 1 is deliberate: a burst only has to wake the loop once, and the
    // loop always reloads every root. A dropped tick therefore loses nothing,
    // because a full channel already means "reload pending".
    let (sender, receiver) = tokio::sync::mpsc::channel::<()>(1);
    let mut watchers = Vec::new();
    for root in roots {
        if !root.is_dir() {
            tracing::debug!(root = %root.display(), "Plugin Catalog root is absent; not watched");
            continue;
        }
        let sender = sender.clone();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else { return };
                if !matches!(
                    event.kind,
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                ) {
                    return;
                }
                let _ = sender.try_send(());
            })
            .context("creating the Plugin Catalog watcher")?;
        watcher
            .watch(root, RecursiveMode::NonRecursive)
            .with_context(|| format!("watching Plugin Catalog {}", root.display()))?;
        tracing::info!(root = %root.display(), "watching the Plugin Catalog for published releases");
        watchers.push(watcher);
    }
    if watchers.is_empty() {
        tracing::warn!("no Plugin Catalog root is watchable; releases need an explicit refresh");
    }
    Ok((watchers, receiver))
}

/// Absorb the rest of a publication burst before reloading once.
async fn settle(receiver: &mut tokio::sync::mpsc::Receiver<()>) {
    loop {
        match tokio::time::timeout(SETTLE, receiver.recv()).await {
            // Another write arrived inside the window: keep waiting.
            Ok(Some(())) => continue,
            // Timed out, or the sender is gone and the queue is drained.
            Ok(None) | Err(_) => return,
        }
    }
}

/// The same reload startup and the admin endpoint perform, in the same order.
async fn adopt(
    catalog: &PluginCatalog,
    storage: &crate::plugin_storage::PluginStorage,
    providers: &crate::provider_catalog::ProviderCatalog,
) -> Result<usize> {
    let count = catalog.refresh_with_runtime(storage).await?;
    providers.refresh_external()?;
    Ok(count)
}

impl PluginCatalog {
    /// The ordered Catalog roots this Catalog reads, canonical root first.
    pub(crate) fn roots(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Publication writes several files per release. The loop must observe the
    /// burst, not one reload per byte.
    #[tokio::test(start_paused = true)]
    async fn a_publication_burst_settles_into_one_reload() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<()>(1);
        for _ in 0..5 {
            let _ = sender.try_send(());
        }
        assert!(receiver.recv().await.is_some(), "the burst wakes the loop");
        settle(&mut receiver).await;
        // Everything the burst queued was absorbed by the settle window, so a
        // second reload has nothing left to consume.
        assert!(
            tokio::time::timeout(SETTLE, receiver.recv()).await.is_err(),
            "the settled burst must not schedule a second reload"
        );
        drop(sender);
    }

    /// A closed channel ends the watch instead of spinning.
    #[tokio::test(start_paused = true)]
    async fn settling_returns_when_every_watcher_is_gone() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<()>(1);
        drop(sender);
        settle(&mut receiver).await;
        assert!(receiver.recv().await.is_none());
    }

    #[test]
    fn an_absent_root_is_skipped_without_failing_the_watch() {
        let root = tempfile::tempdir().unwrap();
        let present = root.path().join("catalog");
        fs::create_dir_all(&present).unwrap();
        let (watchers, _receiver) =
            watch_roots(&[present, root.path().join("legacy-that-never-existed")]).unwrap();
        assert_eq!(watchers.len(), 1, "only the existing root is watched");
    }

    /// The real notify backend must wake the loop when a release lands. This
    /// exercises the actual filesystem path, not a simulated event.
    #[tokio::test]
    async fn a_published_file_wakes_the_loop() {
        let root = tempfile::tempdir().unwrap();
        let catalog = root.path().join("catalog");
        fs::create_dir_all(&catalog).unwrap();
        let (_watchers, mut receiver) = watch_roots(std::slice::from_ref(&catalog)).unwrap();

        // The envelope is the commit marker publication links last.
        fs::write(catalog.join("fixture-1.0.0.release.json"), b"{}").unwrap();

        tokio::time::timeout(Duration::from_secs(10), receiver.recv())
            .await
            .expect("a Catalog write must wake the loop")
            .expect("the watcher keeps the channel open");
    }
}
