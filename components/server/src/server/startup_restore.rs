//! Warm restore of the Hub from durable state at Controller startup.

use super::*;

/// Warm the Hub from durable state before any client connects: settings,
/// Draft directories, workspace orders, session folders, every session with
/// its recent events, transcript anchors and subagent marks, continuations,
/// forks and artifacts. Persisted Busy turns stay guarded until their Machine
/// runtime has had one bounded reconnect window.
pub(super) async fn restore_hub_from_store(store: &Store, hub: &Hub) -> anyhow::Result<()> {
    let settings = store
        .load_settings()
        .await
        .context("loading persisted auth settings")?;
    hub.load_settings(settings);
    store
        .integrate_draft_folders()
        .await
        .context("integrating Draft directories")?;
    hub.restore_workspace_orders(store.load_workspace_orders().await?);
    let folders = store
        .load_session_folders()
        .await
        .context("loading session folders")?;
    hub.restore_session_folders(folders);
    // Warm restore — sessions + events come back exactly as the daemon
    // left them, so on a fresh process every WS client's first snapshot
    // is correct.
    let loaded = store.load_all().await.context("loading persisted state")?;
    let restored: Vec<_> = loaded
        .into_iter()
        .map(|ls| RestoredSession {
            meta: ls.meta,
            log: ls.events,
            event_count: ls.event_count,
            reached_start: ls.reached_start,
            next_seq: ls.next_seq,
            queue: ls.queue,
            drafts: ls.drafts,
            config_options: ls.config_options,
            config_preferences: ls.config_preferences,
            mobile_review_state: ls.mobile_review_state,
            folder_id: ls.folder_id,
            last_event_at_ms: ls.last_event_at_ms,
        })
        .collect();
    let restored_count = restored.len();
    // Adopt durable anchors before any reducer sees a new event, so a
    // restarted Controller never starts a second subagent/Call row.
    match store
        .recent_transcript_anchors(crate::transcript_anchors::HUB_WARM_ANCHORS)
        .await
    {
        Ok(anchors) => hub.warm_transcript_anchors(anchors),
        Err(error) => tracing::warn!(%error, "warming transcript anchors failed"),
    }
    match store
        .recent_subagent_snapshots(crate::transcript_anchors::WARM_SNAPSHOT_MARKS)
        .await
    {
        Ok(snapshots) => hub.warm_subagent_marks(snapshots),
        Err(error) => tracing::warn!(%error, "warming subagent marks failed"),
    }
    // Detached workers outlive this control-plane process. Keep
    // persisted Busy turns guarded until their Machine runtime has had
    // one bounded reconnect window to prove ownership; only then may a
    // missing worker become Interrupted.
    hub.restore_reconciling_runtime(restored);
    crate::session_context::restore_announcements(hub);
    hub.restore_continuations();
    hub.restore_forks();
    hub.set_artifacts(store.artifacts());
    tracing::info!(restored = restored_count, "persistence wired",);
    Ok(())
}

/// What startup persistence produced: the Hub (warmed from the database when
/// one is configured), the store with its write-behind writer and purge
/// sweeper, and the admission fences restored before any of them run.
pub(super) struct Persistence {
    pub(super) hub: Hub,
    pub(super) store: Option<Store>,
    pub(super) health: Option<Arc<PersistenceHealth>>,
    pub(super) writer_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) purge_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) session_id_floor: u64,
    pub(super) plugin_lifecycle_fences: PluginLifecycleFences,
    pub(super) telemetry_binding_fence: crate::telemetry_binding::LegacyFence,
    pub(super) managed_export_activation:
        Option<crate::telemetry_plugin::background_policy::Activation>,
}

/// Phase 2: when --database-url is supplied, hook in the persistent store.
/// Migrations run on every start (sqlx tracks applied versions, so it's
/// idempotent); the in-memory Hub is then warmed from the DB before WS
/// clients can connect. Without a database URL the daemon falls back to
/// pure in-memory mode — same behaviour as before, useful for dev or for
/// running on a host without durable storage configured.
pub(super) async fn open_persistence(
    args: &ServeArgs,
    core_security: Option<&crate::core_security::Config>,
    plugin_dir: &crate::plugin_dir::PluginDir,
    service_id: &str,
    managed_export_policy: Option<
        Arc<crate::telemetry_plugin::background_policy::BackgroundPolicy>,
    >,
    runtime_health: &Arc<RuntimeHealth>,
    store_shutdown_rx: &watch::Receiver<bool>,
) -> anyhow::Result<Persistence> {
    if let Some(url) = args.database_url() {
        let store = Store::connect(url, args.data_dir.join("artifacts"))
            .await
            .context("connecting database")?;
        store.migrate().await.context("running migrations")?;
        store
            .initialize_core_security(core_security, plugin_dir)
            .await
            .context("initializing core security authority")?;
        // Restore durable admission fences before any background writer,
        // sweeper, Plugin host or runtime can race the startup scan.
        let plugin_lifecycle_fences = plugin_uninstall::recover_fences(Some(&store), service_id)
            .await
            .context("restoring Plugin lifecycle fences")?;
        let telemetry_binding_fence =
            crate::telemetry_binding::LegacyFence::recover(Some(&store), service_id)
                .await
                .context("restoring Service telemetry binding fence")?;
        let managed_export_activation = match managed_export_policy {
            Some(policy) => Some(policy.activate(&store).await),
            None => None,
        };
        let session_id_floor = store
            .next_session_number()
            .await
            .context("seeding session id counter")?;
        let health = Arc::new(PersistenceHealth::default());
        let (sink, rx) = StoreSink::channel(STORE_QUEUE_CAPACITY, Arc::clone(&health));
        let hub = Hub::with_store(Some(sink));
        restore_hub_from_store(&store, &hub).await?;
        // Background DB writer: dequeues StoreWrite intents and applies them.
        // Errors are logged but don't bring the daemon down — the in-memory
        // state remains authoritative for the current process.
        runtime_health.set_store_writer(true);
        let writer_health = Arc::clone(runtime_health);
        let writer_store = store.clone();
        let writer_persistence_health = Arc::clone(&health);
        let writer_shutdown = store_shutdown_rx.clone();
        let writer_task = tokio::spawn(async move {
            run_store_writer(writer_store, rx, writer_persistence_health, writer_shutdown).await;
            writer_health.set_store_writer(false);
        });
        // Background sweeper: hard-delete sessions soft-deleted past the
        // retention window, reclaiming their event storage.
        runtime_health.set_purge_sweeper(true);
        let purge_health = Arc::clone(runtime_health);
        let purge_store = store.clone();
        let purge_task = tokio::spawn(async move {
            run_purge_sweeper(purge_store).await;
            purge_health.set_purge_sweeper(false);
            tracing::error!("purge sweeper exited unexpectedly");
        });
        Ok(Persistence {
            hub,
            store: Some(store),
            health: Some(health),
            writer_task: Some(writer_task),
            purge_task: Some(purge_task),
            session_id_floor,
            plugin_lifecycle_fences,
            telemetry_binding_fence,
            managed_export_activation,
        })
    } else {
        tracing::info!("no --database-url: running in-memory only");
        let plugin_lifecycle_fences = plugin_uninstall::recover_fences(None, service_id).await?;
        let telemetry_binding_fence =
            crate::telemetry_binding::LegacyFence::recover(None, service_id).await?;
        let managed_export_activation = None;
        Ok(Persistence {
            hub: Hub::new(),
            store: None,
            health: None,
            writer_task: None,
            purge_task: None,
            session_id_floor: 1,
            plugin_lifecycle_fences,
            telemetry_binding_fence,
            managed_export_activation,
        })
    }
}
