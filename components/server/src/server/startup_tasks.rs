//! Self-contained Controller startup steps that `serve` sequences: the
//! Catalog preflight, local telemetry sinks, persisted timer restoration and
//! the long-running scheduler, reconciliation and dispatcher tasks.

use super::*;

/// `--check-plugin-catalog`: report the Catalog releases this Controller can
/// read without activating any host.
pub(super) fn print_plugin_catalog_preflight(args: &ServeArgs) -> anyhow::Result<()> {
    let catalog = crate::plugin_catalog::PluginCatalog::inspect(
        &args.data_dir,
        args.plugin_catalog_dir.clone(),
    )?;
    let released = catalog
        .entries()
        .into_iter()
        .filter(|entry| {
            matches!(
                entry.release_state,
                crate::plugin_catalog::PluginReleaseState::Ready
            )
        })
        .map(|entry| {
            serde_json::json!({
                "plugin_id": entry.plugin_id,
                "plugin_version": entry.plugin_version,
                "artifact_digest": entry.artifact_digest,
            })
        })
        .collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "dravengarden.cowboy.catalog-reader-preflight/v1",
            "status": "readable",
            "supported_release_schema": cowboy_plugin_sdk::RELEASE_SCHEMA_VERSION,
            "supported_provider_sdk_version": cowboy_provider_sdk::PROVIDER_SDK_VERSION,
            "releases": released,
            "not_checked": ["runtime_artifact_bytes", "host_activation", "database_migrations", "real_login"],
        }))?
    );
    Ok(())
}

/// Records the allocator and async runtime configuration this process uses.
pub(super) fn log_runtime_configuration() {
    tracing::info!(
        compiled =
            crate::memory_observability::compiled_malloc_conf().unwrap_or("jemalloc defaults"),
        runtime_override = crate::memory_observability::runtime_malloc_conf_override()
            .as_deref()
            .unwrap_or("none"),
        "jemalloc configuration"
    );
    tracing::info!(
        tokio_workers = std::env::var("COWBOY_TOKIO_WORKERS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(4),
        "controller tokio runtime"
    );
}

/// Reset credits belong to provider accounts, not sessions. Restore one
/// shared timer per provider and keep them independent from session queues.
pub(super) async fn restore_provider_reset_schedules(usage: &UsageService, store: Option<&Store>) {
    let Some(store) = store else {
        return;
    };
    for provider in &usage.reset_provider_ids() {
        match store.load_provider_reset(provider).await {
            Ok(Some(action)) => {
                usage
                    .set_reset_schedule(
                        provider.as_str(),
                        Some(crate::usage::ResetSchedule {
                            fire_at_ms: action.fire_at_ms,
                        }),
                    )
                    .await;
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, %provider, "loading scheduled provider reset"),
        }
    }
}

/// Opens the configured local telemetry backends.
pub(super) fn local_evidence_sinks(
    args: &ServeArgs,
    log_directory: PathBuf,
) -> anyhow::Result<Vec<Box<dyn crate::logs::EvidenceSink>>> {
    let mut local_sinks: Vec<Box<dyn crate::logs::EvidenceSink>> = Vec::new();
    if matches!(
        args.telemetry_local_backend,
        crate::logs::LocalBackend::Sqlite | crate::logs::LocalBackend::Both
    ) {
        local_sinks.push(Box::new(crate::logs::SqliteStore::open(
            log_directory,
            false,
        )?));
    }
    if matches!(
        args.telemetry_local_backend,
        crate::logs::LocalBackend::Jsonl | crate::logs::LocalBackend::Both
    ) {
        local_sinks.push(Box::new(
            crate::telemetry_file::TelemetryFile::open(
                args.telemetry_dir
                    .clone()
                    .unwrap_or_else(|| crate::telemetry_file::default_directory(&args.data_dir)),
                args.telemetry_segment_bytes,
                args.telemetry_retained_files,
                chrono::Utc::now().timestamp_millis(),
            )
            .context("opening local telemetry")?
            .with_retention_seconds(args.telemetry_retain_seconds)?,
        ));
    }
    Ok(local_sinks)
}

/// Honor agent `ScheduleWakeup`s: fires a wake-prompt (via the same dispatch
/// path) at the scheduled time. Without this, an ACP-driven agent's scheduled
/// self-checks never fire and get consumed by the next user turn instead.
pub(super) async fn start_scheduler(
    hub: &Hub,
    store: Option<&Store>,
    runtime_health: &Arc<RuntimeHealth>,
) -> tokio::task::JoinHandle<()> {
    let (sched_tx, sched_rx) = mpsc::channel::<crate::scheduler::ScheduleCmd>(1_024);
    hub.set_scheduler_tx(sched_tx);
    runtime_health.set_scheduler(true);
    let scheduler_health = Arc::clone(runtime_health);
    let scheduler_hub = hub.clone();
    let scheduler_task = tokio::spawn(async move {
        crate::scheduler::run_scheduler(scheduler_hub, sched_rx).await;
        scheduler_health.set_scheduler(false);
        tracing::error!("scheduler exited unexpectedly");
    });
    // Re-arm wakeups that were pending across this restart; any already overdue
    // fire immediately (catch-up for the downtime).
    if let Some(store) = store {
        match store.load_wakeups().await {
            Ok(ws) => {
                let n = ws.len();
                for (sid, fire_at_ms, prompt) in ws {
                    hub.rearm_wakeup(&sid, fire_at_ms, prompt);
                }
                if n > 0 {
                    tracing::info!(rearmed = n, "re-armed persisted scheduled wakeups");
                }
            }
            Err(e) => tracing::warn!(error = %e, "loading scheduled wakeups (skipping re-arm)"),
        }
    }
    // Re-arm user-scheduled DRAFTS across the restart. These persist inside the
    // restored sessions' drafts jsonb (not a separate table), so re-arm scans the
    // now-restored in-memory sessions. An overdue one fires immediately (catch-up).
    hub.rearm_scheduled_drafts();
    scheduler_task
}

/// Machine WebSockets can only reconnect after Axum starts listening. Keep
/// this timer independent of the request task: real worker snapshots remove
/// their sessions from the reconciliation set, while the remainder become
/// genuine interruptions after the Machine's maximum reconnect delay plus
/// a bounded snapshot-settlement window. Interrupted turns stay stopped
/// until the user submits work.
pub(super) fn spawn_runtime_reconciliation(
    hub: Hub,
    mut shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::select! {
            () = tokio::time::sleep(RUNTIME_RECONCILIATION_GRACE) => {
                let interrupted = hub.finalize_runtime_reconciliation();
                if !interrupted.is_empty() {
                    tracing::warn!(
                        count = interrupted.len(),
                        grace_seconds = RUNTIME_RECONCILIATION_GRACE.as_secs(),
                        "runtime reconciliation grace expired without detached owners"
                    );
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    tracing::info!("runtime reconciliation cancelled by shutdown");
                }
            }
        }
    })
}

/// Starts notification delivery with the Controller's shutdown signal.
pub(super) fn spawn_notifications(state: &Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(notifications::run(
        Arc::clone(state),
        state.shutdown.clone(),
    ))
}

/// Starts the prompt dispatcher that drains the Hub's ready queue into
/// agents, and records its liveness in runtime health.
pub(super) fn spawn_dispatcher(
    state: &Arc<AppState>,
    dispatch_rx: mpsc::Receiver<DispatchReq>,
) -> tokio::task::JoinHandle<()> {
    state.runtime_health.set_dispatcher(true);
    let dispatcher_health = Arc::clone(&state.runtime_health);
    let dispatcher_state = Arc::clone(state);
    let dispatcher_shutdown = state.shutdown.clone();
    let dispatcher_exit_state = dispatcher_shutdown.clone();
    tokio::spawn(async move {
        run_dispatcher(dispatcher_state, dispatch_rx, dispatcher_shutdown).await;
        dispatcher_health.set_dispatcher(false);
        if *dispatcher_exit_state.borrow() {
            tracing::info!("dispatcher stopped after shutdown");
        } else {
            tracing::error!("dispatcher exited while Cowboy was still serving");
        }
    })
}

/// The background tasks `serve` drains after Axum stops and the Catalog
/// observer has shut down. Persistence closes last: the dispatcher returns
/// undelivered prompts first, telemetry drains, then the write-behind store
/// flushes.
pub(super) struct ControllerTasks {
    pub(super) scheduler_task: tokio::task::JoinHandle<()>,
    pub(super) reset_task: tokio::task::JoinHandle<()>,
    pub(super) runtime_reconciliation_task: tokio::task::JoinHandle<()>,
    pub(super) web_push_task: tokio::task::JoinHandle<()>,
    pub(super) dispatcher_task: tokio::task::JoinHandle<()>,
    pub(super) purge_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) machine_presence_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) telemetry_shutdown: Observability,
    pub(super) store_shutdown_tx: watch::Sender<bool>,
    pub(super) writer_task: Option<tokio::task::JoinHandle<()>>,
}

impl ControllerTasks {
    pub(super) async fn shutdown(self) {
        let Self {
            scheduler_task,
            reset_task,
            runtime_reconciliation_task,
            web_push_task,
            mut dispatcher_task,
            purge_task,
            machine_presence_task,
            telemetry_shutdown,
            store_shutdown_tx,
            writer_task,
        } = self;
        scheduler_task.abort();
        reset_task.abort();
        runtime_reconciliation_task.abort();
        web_push_task.abort();
        match tokio::time::timeout(std::time::Duration::from_secs(5), &mut dispatcher_task).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!(%error, "dispatcher task failed during shutdown"),
            Err(_) => {
                tracing::warn!(
                    "dispatcher still returning undelivered prompts; keeping persistence alive"
                );
                if let Err(error) = dispatcher_task.await {
                    tracing::error!(%error, "dispatcher failed while finishing shutdown retention");
                }
            }
        }
        if let Some(task) = purge_task {
            task.abort();
        }
        if let Some(task) = machine_presence_task {
            task.abort();
        }
        telemetry_shutdown.drain().await;
        let _ = store_shutdown_tx.send(true);
        if let Some(task) = writer_task {
            match tokio::time::timeout(std::time::Duration::from_secs(10), task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::error!(%error, "store writer task failed during shutdown")
                }
                Err(_) => tracing::error!("store writer did not drain within shutdown deadline"),
            }
        }
    }
}
