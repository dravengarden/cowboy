//! Warm restore of the Hub from durable state at Controller startup.

use super::*;

/// Warm the Hub from durable state before any client connects: settings,
/// Draft directories, workspace orders, session folders, every session with
/// its recent events, transcript anchors and subagent marks, continuations,
/// forks and artifacts. Persisted Busy turns stay guarded until their Machine
/// runtime has had one bounded reconnect window.
pub async fn restore_hub_from_store(store: &Store, hub: &Hub) -> anyhow::Result<()> {
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
    let mut restored: Vec<_> = loaded
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
    managed_calls::prune_settled_restored_notices(store, &mut restored).await;
    let restored_count = restored.len();
    // Load required lineage edges with retry before admitting requests or event
    // writes (fail-closed on persistent failure).
    let lineage_edges = load_required_lineage_edges(store).await?;
    hub.warm_lineage_edges(&lineage_edges);
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
    let dispatches = store
        .recent_prompt_dispatches(crate::store::prompt_dispatches::STARTUP_HYDRATE_PER_SESSION)
        .await
        .context("loading recent prompt dispatches")?;
    hub.hydrate_prompt_dispatches(dispatches);
    crate::session_context::restore_announcements(hub);
    hub.restore_continuations();
    hub.restore_forks();
    hub.restore_triggers();
    hub.warm_lineage_edges(&lineage_edges);
    for meta in hub.session_list() {
        if let Ok(ctx) = crate::session_context::read(hub, &meta.id) {
            for link in ctx.waiting {
                if link.before > 0 {
                    hub.freeze_lineage_prefix(&link.session_id, link.before);
                }
            }
        }
    }
    for (source_id, before_seq) in hub.staging_fork_freeze_floors() {
        hub.freeze_lineage_prefix(&source_id, before_seq);
    }
    if let Ok(reserved_floors) = store.load_reserved_duplicate_freeze_floors().await {
        for (source_id, before_seq) in reserved_floors {
            hub.freeze_lineage_prefix(&source_id, before_seq);
        }
    }
    hub.set_artifacts(store.artifacts());
    tracing::info!(restored = restored_count, "persistence wired",);
    Ok(())
}

async fn load_required_lineage_edges(
    store: &Store,
) -> anyhow::Result<Vec<crate::store::LineageEdge>> {
    let mut last_error = None;
    for attempt in 0..3 {
        match store.load_all_lineage_edges().await {
            Ok(edges) => return Ok(edges),
            Err(err) => {
                tracing::warn!(%err, attempt = attempt + 1, "retrying required lineage edge load");
                last_error = Some(err);
                tokio::time::sleep(std::time::Duration::from_millis(100 * (1 << attempt))).await;
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("required lineage edges unavailable"))
        .context("loading required lineage edges during startup recovery"))
}

/// What startup persistence produced: the Hub (warmed from the database when
/// one is configured), the store with its write-behind writer and purge
/// sweeper, and the admission fences restored before any of them run.
pub struct Persistence {
    pub hub: Hub,
    pub store: Option<Store>,
    pub health: Option<Arc<PersistenceHealth>>,
    pub writer_task: Option<tokio::task::JoinHandle<()>>,
    pub purge_task: Option<tokio::task::JoinHandle<()>>,
    pub session_id_floor: u64,
    pub plugin_lifecycle_fences: PluginLifecycleFences,
    pub telemetry_binding_fence: crate::telemetry_binding::LegacyFence,
    pub managed_export_activation: Option<crate::telemetry_plugin::background_policy::Activation>,
}

/// `--check-startup-admission`: whether the startup gates that depend on
/// durable session state would admit this Controller, checked read-only
/// beside the serving one (no vault or instance lock, migration, write or
/// listener). A component activator runs it for a candidate before stopping
/// anything, and for its rollback target to learn whether that one could
/// start. Prints one JSON report: `admissible`, `refused` (also the error,
/// so the exit status fails) or `unverified` when the answer cannot be
/// established read-only: the state could not be read (a missing database,
/// a schema older than this binary's migrations, a timeout), or only rows
/// startup settles on an admitted dataset stand in the way, which needs the
/// vault's wipe generations.
pub async fn check_startup_admission(args: &ServeArgs) -> anyhow::Result<()> {
    // Whether startup may settle rejected intake registrations is decided
    // before any read: without it only the plain live-session query runs,
    // which needs no table a pre-retirement schema may lack.
    let settlement = admission_settlement(
        crate::provider_service::recorded_admission(&args.data_dir),
        crate::service_identity::inspect(&args.data_dir),
    );
    let read = async {
        let Some(url) = args.database_url() else {
            return Ok(Vec::new());
        };
        let store = Store::connect_read_only(url, args.data_dir.join("artifacts"))
            .await
            .context("opening the database read-only")?;
        let rows = if matches!(settlement, Settlement::Never) {
            store
                .live_retired_default_sessions()
                .await
                .map(|ids| ids.into_iter().map(|id| (id, false)).collect())
        } else {
            store.live_retired_default_sessions_settleable().await
        };
        rows.context("reading live sessions on retired legacy default accounts")
    };
    let report = match tokio::time::timeout(std::time::Duration::from_secs(30), read).await {
        Ok(Ok(rows)) => admission_report(&settlement, rows),
        Ok(Err(error)) => AdmissionReport::unverified(format!("{error:#}")),
        Err(_) => AdmissionReport::unverified("timed out after 30s".to_owned()),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "dravengarden.cowboy.startup-admission-preflight/v1",
            "status": report.status,
            "retired_default_sessions": report.sessions,
            "unverified": report.unverified,
            "not_checked": [
                "provider_credentials",
                "machine_wipe_acknowledgements",
                "database_migrations",
                "configuration",
            ],
        }))?
    );
    match report.status {
        "refused" => provider_accounts::retirement::refuse_live_sessions(&report.sessions),
        _ => Ok(()),
    }
}

/// Whether startup settles rejected intake registrations
/// (`retirement::admit_store`): only when the vault's recorded admission
/// matches this dataset and its current wipe generations.
enum Settlement {
    /// No recorded admission for this dataset: startup settles nothing.
    Never,
    /// Recorded for this dataset; the wipe generations need the vault.
    Unconfirmed(String),
}

fn admission_settlement(
    admitted: Option<crate::provider_service::AdmittedRetirement>,
    identity: anyhow::Result<String>,
) -> Settlement {
    let Some(admitted) = admitted else {
        return Settlement::Never;
    };
    match identity {
        Ok(id) if id != admitted.dataset => Settlement::Never,
        Ok(_) => Settlement::Unconfirmed(
            "startup settles these only if the vault's wipe generations still match its recorded admission".to_owned(),
        ),
        Err(error) => Settlement::Unconfirmed(format!("reading the Service identity: {error:#}")),
    }
}

struct AdmissionReport {
    status: &'static str,
    sessions: Vec<String>,
    unverified: Option<String>,
}

impl AdmissionReport {
    fn unverified(reason: String) -> Self {
        Self {
            status: "unverified",
            sessions: Vec::new(),
            unverified: Some(reason),
        }
    }
}

/// `rows`: each live retired-default session with whether startup's
/// settlement would take it. A session it would not take refuses startup
/// in every case; only settleable ones leave the answer unconfirmed.
fn admission_report(settlement: &Settlement, rows: Vec<(String, bool)>) -> AdmissionReport {
    let always: Vec<String> = rows
        .iter()
        .filter(|(_, settleable)| !settleable || matches!(settlement, Settlement::Never))
        .map(|(id, _)| id.clone())
        .collect();
    if !always.is_empty() {
        return AdmissionReport {
            status: "refused",
            sessions: always,
            unverified: None,
        };
    }
    match (rows.is_empty(), settlement) {
        (true, _) | (_, Settlement::Never) => AdmissionReport {
            status: "admissible",
            sessions: Vec::new(),
            unverified: None,
        },
        (false, Settlement::Unconfirmed(reason)) => AdmissionReport {
            sessions: rows.into_iter().map(|(id, _)| id).collect(),
            ..AdmissionReport::unverified(reason.clone())
        },
    }
}

/// Phase 2: when --database-url is supplied, hook in the persistent store.
/// Migrations run on every start (sqlx tracks applied versions, so it's
/// idempotent); the in-memory Hub is then warmed from the DB before WS
/// clients can connect. Without a database URL the daemon falls back to
/// pure in-memory mode — same behaviour as before, useful for dev or for
/// running on a host without durable storage configured.
pub async fn open_persistence(
    args: &ServeArgs,
    core_security: Option<&crate::core_security::Config>,
    plugin_dir: &crate::plugin_dir::PluginDir,
    (service_id, provider_auth): (&str, &crate::provider_service::ProviderAuthService),
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
        // Before retiring account rows, validate the credentials and sessions.
        // A previously admitted dataset can settle rejected intake rows here.
        provider_accounts::retirement::admit_store(provider_auth, &store, service_id)
            .await
            .context("admitting the legacy card retirement")?;
        // After both credential gates, before anything is served: never from
        // a CLI that may share a database with a Controller from before the
        // retirement.
        store
            .retire_legacy_card_rows()
            .await
            .context("removing retired legacy card rows")?;
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
        let writer_acks = hub.store_ack_sink();
        let writer_floors = hub.snapshot_freeze_floors();
        let writer_task = tokio::spawn(async move {
            run_store_writer_with_floors(
                writer_store,
                rx,
                writer_persistence_health,
                writer_shutdown,
                Some(writer_acks),
                writer_floors,
            )
            .await;
            writer_health.set_store_writer(false);
        });
        // Observed subagent transcripts have their own bounded writer, so
        // they can never delay the session timeline's persistence.
        crate::subagent_transcripts::spawn_writer(store.clone());
        let backfill_store = store.clone();
        let backfill_shutdown = store_shutdown_rx.clone();
        tokio::spawn(async move {
            run_turn_cache_backfill(backfill_store, backfill_shutdown).await;
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

#[cfg(test)]
mod admission_report_tests {
    use super::{Settlement, admission_report, admission_settlement};
    use crate::provider_service::AdmittedRetirement;

    fn admitted(dataset: &str) -> Option<AdmittedRetirement> {
        Some(AdmittedRetirement {
            dataset: dataset.to_owned(),
            wipes: std::collections::BTreeMap::new(),
        })
    }

    fn report(settlement: &Settlement, rows: &[(&str, bool)]) -> (&'static str, Vec<String>) {
        let rows = rows
            .iter()
            .map(|(id, settleable)| ((*id).to_owned(), *settleable))
            .collect();
        let report = admission_report(settlement, rows);
        (report.status, report.sessions)
    }

    #[test]
    fn settlement_needs_a_recorded_admission_of_this_dataset() {
        let ok = || anyhow::Ok("svc".to_owned());
        assert!(matches!(
            admission_settlement(None, ok()),
            Settlement::Never
        ));
        assert!(matches!(
            admission_settlement(admitted("other"), ok()),
            Settlement::Never
        ));
        assert!(matches!(
            admission_settlement(admitted("svc"), ok()),
            Settlement::Unconfirmed(_)
        ));
        assert!(matches!(
            admission_settlement(admitted("svc"), Err(anyhow::anyhow!("symlinked"))),
            Settlement::Unconfirmed(_)
        ));
    }

    #[test]
    fn only_settlement_independent_answers_are_definitive() {
        let never = Settlement::Never;
        let unconfirmed = Settlement::Unconfirmed("wipes".to_owned());
        assert_eq!(report(&unconfirmed, &[]), ("admissible", vec![]));
        assert_eq!(
            report(&unconfirmed, &[("intake", true), ("live", false)]),
            ("refused", vec!["live".to_owned()])
        );
        assert_eq!(
            report(&unconfirmed, &[("intake", true)]),
            ("unverified", vec!["intake".to_owned()])
        );
        assert_eq!(
            report(&never, &[("intake", true)]),
            ("refused", vec!["intake".to_owned()])
        );
        assert_eq!(report(&never, &[]), ("admissible", vec![]));
    }
}
