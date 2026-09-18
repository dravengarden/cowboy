//! Connection-owned, coalesced credential observation. Never recurse into HOME.

use super::*;
use crate::machine_plugins::auth_watch::AuthWatchPlan;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use parking_lot::{Mutex, RwLock};

pub(super) struct AuthWatcher {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for AuthWatcher {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Watches {
    watcher: RecommendedWatcher,
    installed: BTreeSet<PathBuf>,
    plan: Arc<RwLock<AuthWatchPlan>>,
}

impl Watches {
    fn new(plan: AuthWatchPlan, changed: tokio::sync::mpsc::Sender<()>) -> anyhow::Result<Self> {
        let shared = Arc::new(RwLock::new(plan));
        let filter = Arc::clone(&shared);
        let watcher = notify::recommended_watcher(move |change: notify::Result<notify::Event>| {
            let relevant = match change {
                Ok(event) => {
                    !matches!(event.kind, EventKind::Access(_))
                        && (event.need_rescan() || filter.read().relevant(&event.paths))
                }
                Err(error) => {
                    tracing::warn!(%error, "Provider credential watch requires reconciliation");
                    true
                }
            };
            if relevant {
                // A full channel already represents every change: the consumer
                // reads current files, never replays stale credential contents.
                let _ = changed.try_send(());
            }
        })
        .context("creating Provider credential watcher")?;
        Ok(Self {
            watcher,
            installed: BTreeSet::new(),
            plan: shared,
        })
    }

    fn update(&mut self, plan: AuthWatchPlan) -> anyhow::Result<()> {
        let desired: BTreeSet<_> = plan
            .directories
            .iter()
            .filter(|path| path.is_dir())
            .cloned()
            .collect();
        *self.plan.write() = plan;
        for path in self.installed.difference(&desired) {
            // A removed directory has already lost its kernel watch.
            let _ = self.watcher.unwatch(path);
        }
        // Re-arm even an unchanged pathname: its directory inode may have been
        // deleted and recreated while events were coalesced.
        for path in &desired {
            self.watcher
                .watch(path, RecursiveMode::NonRecursive)
                .with_context(|| format!("watching credential directory {}", path.display()))?;
        }
        self.installed = desired;
        Ok(())
    }
}

pub(super) async fn start(
    providers: Arc<MachinePluginStore>,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
) -> anyhow::Result<AuthWatcher> {
    let store = Arc::clone(&providers);
    let plan = tokio::task::spawn_blocking(move || store.auth_watch_plan()).await??;
    let (changed, mut changes) = tokio::sync::mpsc::channel(1);
    let mut watches = Watches::new(plan.clone(), changed)?;
    watches.update(plan)?;
    tracing::info!(
        directories = watches.installed.len(),
        "Provider credential watches installed"
    );
    let watches = Arc::new(Mutex::new(watches));
    let task = tokio::spawn(async move {
        loop {
            let store = Arc::clone(&providers);
            let output = events.clone();
            let owned = Arc::clone(&watches);
            let result = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                if output.is_closed() {
                    return Ok(());
                }
                // Install newly discovered paths before reading credentials.
                // This closes the generation-creation and atomic-rename gap.
                owned.lock().update(store.auth_watch_plan()?)?;
                Ok(())
            })
            .await;
            match result {
                Ok(Ok(())) => publish_provider_auth_observations(&providers, &events, false).await,
                other => {
                    tracing::warn!(error = ?other, "Provider credential reconciliation failed")
                }
            }
            tokio::select! {
                () = events.closed() => break,
                change = changes.recv() => if change.is_none() { break; },
                // Recover from watch overflow or a directory recreated between
                // notifications. This is a bounded metadata scan, not a HOME walk.
                () = tokio::time::sleep(Duration::from_secs(60)) => {}
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            // At most one queued marker; ongoing writes cannot starve the scan.
            let _ = changes.try_recv();
        }
    });
    Ok(AuthWatcher { task })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn refresh_after_upgrade_advertises_matching_generation_before_candidate() {
        let installed = |version: &str, digest: &str| {
            serde_json::from_value::<crate::machine_protocol::PluginInventory>(serde_json::json!({
                "plugin_id": "claude-code", "plugin_version": version,
                "generation_digest": digest, "contract_fingerprint": "fixture",
                "state": "active", "auth_generation": 8, "replica_state": "current",
                "materialization_state": "applying"
            }))
            .unwrap()
        };
        let mut controller_inventory = vec![installed("old", "old-digest")];
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        super::super::publish_auth_refresh_events(
            &tx,
            vec![installed("new", "new-digest")],
            vec![MachineEvent::ProviderAuthRefreshCandidate {
                request_id: "refresh-fixture".into(),
                provider_id: "claude-code".into(),
                expected_generation: 8,
                provider_version: "new".into(),
                generation_digest: "new-digest".into(),
                auth_contract_fingerprint: "fixture".into(),
                portable_schema: "fixture".into(),
                auth_method: "fixture".into(),
                bundle: Default::default(),
            }],
        );
        let mut accepted = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                MachineEvent::PluginInventory { plugins, .. } => controller_inventory = plugins,
                MachineEvent::ProviderAuthRefreshCandidate {
                    provider_version,
                    generation_digest,
                    expected_generation,
                    ..
                } => {
                    let current = &mut controller_inventory[0];
                    assert_eq!(current.plugin_version, provider_version);
                    assert_eq!(current.generation_digest, generation_digest);
                    assert_eq!(current.auth_generation, Some(expected_generation));
                    // Model an immediate successful reconciliation. A trailing
                    // Applying inventory must not overwrite this terminal state.
                    current.materialization_state = ProviderMaterializationState::Current;
                    accepted = true;
                }
                _ => panic!("unexpected refresh event"),
            }
        }
        assert!(accepted);
        assert_eq!(
            controller_inventory[0].materialization_state,
            ProviderMaterializationState::Current
        );
    }

    #[tokio::test]
    async fn authority_apply_publishes_inventory_even_without_a_refresh_candidate() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(
            MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).unwrap(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        publish_provider_auth_observations(&store, &tx, false).await;
        assert!(rx.try_recv().is_err());
        publish_provider_auth_observations(&store, &tx, true).await;
        assert!(matches!(
            rx.try_recv().unwrap(),
            MachineEvent::PluginInventory { .. }
        ));
    }

    #[tokio::test]
    async fn native_atomic_replacement_is_observed_without_watching_runtime_trees() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("runtime/3/home/.claude");
        fs::create_dir_all(config.join("projects/session/deep")).unwrap();
        fs::create_dir_all(config.join("plugins/cache/deep")).unwrap();
        let credential = config.join(".credentials.json");
        let canonical = root.path().join("canonical.json");
        fs::write(&canonical, b"before").unwrap();
        std::os::unix::fs::symlink(&canonical, &credential).unwrap();
        let mut plan = AuthWatchPlan::default();
        plan.file(root.path(), credential.clone());
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let mut watcher = Watches::new(plan.clone(), tx).unwrap();
        watcher.update(plan.clone()).unwrap();
        assert_eq!(watcher.installed.len(), 5);
        for index in 0..100 {
            fs::write(
                config.join(format!("projects/session/deep/{index}.jsonl")),
                b"noise",
            )
            .unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), rx.recv())
                .await
                .is_err()
        );
        let temporary = config.join(".credentials.tmp");
        fs::write(&temporary, b"rotated").unwrap();
        fs::rename(temporary, &credential).unwrap();
        tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fs::read(&credential).unwrap(), b"rotated");
        assert_eq!(fs::read(&canonical).unwrap(), b"before");
        fs::remove_dir_all(&config).unwrap();
        fs::create_dir_all(&config).unwrap();
        watcher.update(plan).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = rx.try_recv();
        fs::write(&credential, b"after-recreation").unwrap();
        tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn dropping_connection_owner_aborts_observation_task() {
        let (started, ready) = tokio::sync::oneshot::channel();
        let (released, done) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _release_on_abort = released;
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        let owner = AuthWatcher { task };
        ready.await.unwrap();
        drop(owner);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), done)
                .await
                .unwrap()
                .is_err()
        );
    }
}
