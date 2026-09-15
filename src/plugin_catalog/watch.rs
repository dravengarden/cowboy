//! Controller-owned observation of published Catalog inputs.
//!
//! Filesystem events and bounded metadata scans are hints, never publication,
//! signature or installation authority. Refresh uses the ordinary trusted reader.
//! Stop suppresses new attempts; an already-started refresh drains, not rolls back.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use notify::{EventKind, RecursiveMode, Watcher as _};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::PluginCatalog;

mod sources;

const SETTLE: Duration = Duration::from_millis(750);
const MAX_SETTLE: Duration = Duration::from_secs(3);
const RECHECK: Duration = Duration::from_secs(30);
const RETRY_BACKOFF: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(15),
    Duration::from_secs(60),
];

/// Drop requests stop; shutdown also joins the task before storage teardown.
/// Neither operation cancels an already-started Catalog/runtime refresh.
pub(crate) struct CatalogWatcher {
    watchers: Vec<notify::RecommendedWatcher>,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl Drop for CatalogWatcher {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        self.watchers.clear();
    }
}

impl CatalogWatcher {
    pub(crate) async fn shutdown(mut self) -> Result<()> {
        self.stop.send_replace(true);
        self.watchers.clear();
        if let Some(task) = self.task.take() {
            task.await.context("joining Plugin Catalog observer")?;
        }
        Ok(())
    }
}

struct Stop {
    owner: watch::Receiver<bool>,
    service: watch::Receiver<bool>,
}

impl Stop {
    fn requested(&self) -> bool {
        *self.owner.borrow()
            || self.owner.has_changed().is_err()
            || *self.service.borrow()
            || self.service.has_changed().is_err()
    }

    async fn cancelled(&mut self) {
        while !self.requested() {
            tokio::select! {
                _ = self.owner.changed() => {}
                _ = self.service.changed() => {}
            }
        }
    }
}

pub(crate) fn spawn(
    catalog: Arc<PluginCatalog>,
    storage: crate::plugin_storage::PluginStorage,
    providers: Arc<crate::provider_catalog::ProviderCatalog>,
    service: watch::Receiver<bool>,
) -> CatalogWatcher {
    let mut roots = catalog.roots();
    if let Some(legacy) = providers.legacy_catalog_root()
        && !roots.iter().any(|root| root == legacy)
    {
        roots.push(legacy.to_owned());
    }
    let (watchers, receiver) = match watch_roots(&roots) {
        Ok(watches) => watches,
        Err(error) => {
            tracing::warn!(%error, "Catalog notifications unavailable; using bounded source rechecks");
            let (_, receiver) = mpsc::channel(1);
            (Vec::new(), receiver)
        }
    };
    let (stop, owner) = watch::channel(false);
    let roots = Arc::new(roots);
    let task = tokio::spawn(run(
        receiver,
        Stop { owner, service },
        move || {
            let roots = Arc::clone(&roots);
            async move {
                tokio::task::spawn_blocking(move || sources::sample(&roots))
                    .await
                    .context("joining Catalog source scan")?
            }
        },
        move || {
            let catalog = Arc::clone(&catalog);
            let storage = storage.clone();
            let providers = Arc::clone(&providers);
            async move { adopt(&catalog, &storage, &providers).await }
        },
    ));
    CatalogWatcher {
        watchers,
        stop,
        task: Some(task),
    }
}

async fn run<P, A>(
    mut receiver: mpsc::Receiver<()>,
    mut stop: Stop,
    mut probe: impl FnMut() -> P,
    mut refresh: impl FnMut() -> A,
) where
    P: Future<Output = Result<sources::Hint>>,
    A: Future<Output = Result<usize>>,
{
    let mut poll = tokio::time::interval(RECHECK);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut hints_open = true;
    let mut accepted = None;
    loop {
        // The first interval tick reconciles the startup/watch registration gap.
        tokio::select! {
            biased;
            _ = stop.cancelled() => return,
            hint = receiver.recv(), if hints_open => {
                if hint.is_none() {
                    hints_open = false;
                    continue;
                }
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => return,
                    () = settle(&mut receiver) => {}
                }
            }
            _ = poll.tick() => {}
        }
        if stop.requested() {
            return;
        }
        let hint = match probe().await {
            Ok(hint) => hint,
            Err(error) => {
                tracing::warn!(%error, "Catalog source scan failed; retrying at the next observation");
                continue;
            }
        };
        if stop.requested() {
            return;
        }
        // An unchanged hint avoids rebuilding every Plugin host on every timer.
        // No hint substitutes for the exact signature-checked reader below.
        if accepted == Some(hint) {
            continue;
        }
        let mut backoff = RETRY_BACKOFF.iter();
        loop {
            if stop.requested() {
                return;
            }
            // Deliberately not selected against cancellation: staging/migration
            // may have begun. Shutdown joins this attempt before closing storage.
            match refresh().await {
                Ok(count) => {
                    accepted = Some(hint);
                    tracing::info!(
                        external_releases = count,
                        "refreshed published Catalog projections"
                    );
                    break;
                }
                Err(error) => {
                    if stop.requested() {
                        return;
                    }
                    let Some(delay) = backoff.next() else {
                        tracing::error!(%error, "Catalog refresh/projection failed; later source rechecks remain active");
                        break;
                    };
                    tracing::warn!(%error, ?delay, "Catalog refresh/projection failed; retrying");
                    tokio::select! {
                        biased;
                        _ = stop.cancelled() => return,
                        () = tokio::time::sleep(*delay) => {}
                    }
                }
            }
        }
    }
}

fn watch_roots(roots: &[PathBuf]) -> Result<(Vec<notify::RecommendedWatcher>, mpsc::Receiver<()>)> {
    let (sender, receiver) = mpsc::channel(1);
    let mut watchers = Vec::new();
    for root in roots {
        // Trust files are direct children of this separate, nonrecursive watch.
        // Absent/replaced paths and backend loss are covered by source rechecks.
        for path in [root.clone(), root.join("trusted-publishers")] {
            if !path.is_dir() {
                continue;
            }
            let sender = sender.clone();
            let mut watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    if wakes_reader(&event) {
                        let _ = sender.try_send(());
                    }
                })
                .context("creating Plugin Catalog notifications")?;
            watcher
                .watch(&path, RecursiveMode::NonRecursive)
                .with_context(|| format!("watching Plugin Catalog {}", path.display()))?;
            watchers.push(watcher);
        }
    }
    Ok((watchers, receiver))
}

fn wakes_reader(event: &notify::Result<notify::Event>) -> bool {
    match event {
        Err(_) => true, // Backend errors/overflow require a bounded recheck.
        Ok(event) => {
            event.need_rescan()
                || matches!(
                    event.kind,
                    EventKind::Any
                        | EventKind::Other
                        | EventKind::Create(_)
                        | EventKind::Modify(_)
                        | EventKind::Remove(_)
                )
        }
    }
}

async fn settle(receiver: &mut mpsc::Receiver<()>) {
    let deadline = tokio::time::Instant::now() + MAX_SETTLE;
    loop {
        let quiet = (tokio::time::Instant::now() + SETTLE).min(deadline);
        match tokio::time::timeout_at(quiet, receiver.recv()).await {
            Ok(Some(())) if tokio::time::Instant::now() < deadline => {}
            _ => return,
        }
    }
}

async fn adopt(
    catalog: &PluginCatalog,
    storage: &crate::plugin_storage::PluginStorage,
    providers: &crate::provider_catalog::ProviderCatalog,
) -> Result<usize> {
    let count = catalog
        .refresh_with_runtime(storage)
        .await
        .context("refreshing verified Plugin Catalog/runtime")?;
    // The Plugin snapshot has committed at this point. A legacy Provider
    // projection failure is not a rollback or an atomic two-Catalog snapshot.
    providers
        .refresh_external()
        .context("refreshing Provider projection after Catalog commit")?;
    Ok(count)
}

impl PluginCatalog {
    pub(crate) fn roots(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }
}

#[cfg(test)]
mod tests;
