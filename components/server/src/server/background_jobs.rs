//! Long-running Controller tasks: the store writer, prompt dispatcher,
//! lifecycle incidents, component convergence and generation retention.

use super::*;

/// Converge signed automatic components continuously instead of only at a
/// Machine's welcome. Every dispatch here is the same signed record, the same
/// Reconcile command and the same Machine-side verification a person would
/// trigger by hand; what this loop adds is that nobody has to.
pub(super) async fn run_component_convergence(
    state: Arc<AppState>,
    machine_snapshots: MachineSnapshots,
    mut shutdown: watch::Receiver<bool>,
) {
    let policy = crate::machine_convergence::ConvergencePolicy::default();
    let mut was_frozen = false;
    loop {
        tokio::select! {
            () = tokio::time::sleep(MACHINE_CONVERGENCE_INTERVAL) => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
        if *shutdown.borrow() {
            break;
        }
        match state.desired_machine_components.reload(now_ms()) {
            Ok(Some(generation)) => {
                tracing::info!(generation, "desired Machine component set reloaded");
            }
            Ok(None) => {}
            Err(error) => {
                // The last accepted set stays in force; convergence continues
                // against it rather than against a half-read file.
                tracing::error!(%error, "rejected the Machine component manifest");
            }
        }
        match state.desired_machine_plugins.reload(now_ms()) {
            Ok(Some(generation)) => {
                tracing::info!(generation, "Service-side Machine Plugin document reloaded");
                state.machine_snapshots.publish().await;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::error!(%error, "rejected the Machine Plugin document");
            }
        }
        let desired = state.desired_machine_components.current();
        if desired.is_empty() {
            continue;
        }
        let frozen = state.convergence_freeze.current();
        if frozen.is_some() != was_frozen {
            was_frozen = frozen.is_some();
            match &frozen {
                Some(record) => tracing::warn!(
                    actor = %record.actor,
                    reason = record.reason.as_deref().unwrap_or("unstated"),
                    "component convergence is frozen"
                ),
                None => tracing::info!("component convergence resumed"),
            }
        }
        let machines = match machine_snapshots.load().await {
            Ok(machines) => machines,
            Err(error) => {
                tracing::warn!(%error, "loading Machines for component convergence");
                continue;
            }
        };
        for summary in machines {
            if state.component_convergence.is_in_flight(&summary.id) {
                continue;
            }
            let attempts = state.component_convergence.attempts(&summary.id);
            let plan = crate::machine_convergence::plan_machine(
                &summary,
                &desired,
                &attempts,
                frozen.is_some(),
                now_ms(),
            );
            if summary.connected {
                // A converged component keeps no history. An offline Machine
                // reports nothing, so it never prunes: reconnecting must not
                // become a way to clear a blocked digest.
                state.component_convergence.retain(
                    &summary.id,
                    &plan.reported.iter().map(|entry| entry.id.clone()).collect(),
                );
            }
            if plan.dispatch.is_empty() {
                continue;
            }
            if state
                .machine_control
                .connection_age(&summary.id)
                .is_none_or(|age| age < MACHINE_CONVERGENCE_CONNECTION_GRACE)
            {
                continue;
            }
            if !state.component_convergence.begin(&summary.id) {
                continue;
            }
            let task_state = Arc::clone(&state);
            let snapshots = machine_snapshots.clone();
            tokio::spawn(async move {
                let machine_id = summary.id;
                let request_id = machine_request_id("converge");
                let components: Vec<crate::machine_protocol::ComponentId> = plan
                    .dispatch
                    .iter()
                    .map(|component| component.id.clone())
                    .collect();
                tracing::info!(
                    %machine_id,
                    %request_id,
                    components = components.len(),
                    "converging signed Machine components"
                );
                task_state.component_convergence.record_dispatch(
                    &machine_id,
                    &plan.dispatch,
                    policy,
                    now_ms(),
                );
                let outcome = task_state
                    .machine_control
                    .command_request_with_timeout(
                        &machine_id,
                        request_id.clone(),
                        crate::machine_protocol::MachineCommand::Reconcile {
                            request_id: request_id.clone(),
                            components: plan.dispatch.clone(),
                        },
                        MACHINE_COMPONENT_COMMAND_TIMEOUT,
                    )
                    .await;
                match outcome {
                    Ok(()) => {
                        task_state.component_convergence.record_outcome(
                            &machine_id,
                            &plan.dispatch,
                            None,
                        );
                        tracing::info!(
                            %machine_id,
                            %request_id,
                            "Machine accepted the component reconcile"
                        );
                    }
                    Err(error) => {
                        task_state.component_convergence.record_outcome(
                            &machine_id,
                            &plan.dispatch,
                            Some(&error),
                        );
                        tracing::warn!(
                            %machine_id,
                            %request_id,
                            %error,
                            "Machine component convergence failed"
                        );
                    }
                }
                task_state.component_convergence.finish(&machine_id);
                snapshots.publish().await;
            });
        }
    }
}

/// Drain write-behind intents in small batches. Streaming text/tool updates are
/// reduced to stable rows before persistence, and transient usage/session-info
/// frames only advance the durable sequence watermark.
pub(super) async fn run_store_writer(
    store: Store,
    mut rx: StoreReceiver,
    health: Arc<PersistenceHealth>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut reducer = EventReducer::default();
    let mut draining = *shutdown.borrow();
    if draining {
        rx.close();
    }
    loop {
        let first = tokio::select! {
            biased;
            changed = shutdown.changed(), if !draining => {
                if changed.is_err() || *shutdown.borrow() {
                    rx.close();
                    draining = true;
                }
                continue;
            }
            write = rx.recv() => write,
        };
        let Some(first) = first else { break };
        let mut bytes = crate::core::estimated_store_write_bytes(&first);
        let mut batch = vec![first];
        while batch.len() < 256 && bytes < crate::core::STORE_BATCH_MAX_BYTES {
            match tokio::time::timeout(std::time::Duration::from_millis(10), rx.recv()).await {
                Ok(Some(write)) => {
                    bytes = bytes.saturating_add(crate::core::estimated_store_write_bytes(&write));
                    batch.push(write);
                }
                Ok(None) | Err(_) => break,
            }
        }
        if !apply_store_batch(&store, &mut reducer, batch).await {
            health.mark_failed_batch();
        }
    }
    tracing::info!("store writer shutting down (channel closed)");
}

pub(super) async fn apply_store_batch(
    store: &Store,
    reducer: &mut EventReducer,
    writes: Vec<StoreWrite>,
) -> bool {
    let mut events: BTreeMap<(String, u64), Envelope> = BTreeMap::new();
    let mut highwaters: HashMap<String, u64> = HashMap::new();
    let mut anchors: Vec<crate::transcript_anchors::AnchorWrite> = Vec::new();
    let mut occurrences: Vec<crate::task_notifications::Occurrence> = Vec::new();
    let mut ok = true;
    for write in writes {
        match write {
            // Committed with the batch, so it never splits one.
            StoreWrite::RecordOccurrence(occurrence) => occurrences.push(occurrence),
            StoreWrite::AppendEvent(env) => {
                occurrences.extend(notifications::occurrence_of(&env));
                highwaters
                    .entry(env.session_id.clone())
                    .and_modify(|seq| *seq = (*seq).max(env.seq.saturating_add(1)))
                    .or_insert_with(|| env.seq.saturating_add(1));
                let anchor = match &env.event {
                    Event::Update { update } => crate::transcript_anchors::anchor_key(update),
                    _ => None,
                };
                // An anchor created earlier in this unflushed batch is not in
                // the table yet; the reducer may already have evicted it.
                let pending = anchor.as_deref().and_then(|key| {
                    anchors
                        .iter()
                        .find(|write| write.session_id == env.session_id && write.key == key)
                        .map(|write| write.seq)
                });
                let mut unresolved = None;
                if let (Some(key), Some(seq)) = (anchor.as_deref(), pending) {
                    reducer.adopt_anchor(&env.session_id, key, seq);
                } else if let Some(key) = anchor.as_deref()
                    && !reducer.knows_anchor(&env.session_id, key)
                {
                    match resolve_durable_anchor(store, &env.session_id, key).await {
                        Ok(Some(seq)) => reducer.adopt_anchor(&env.session_id, key, seq),
                        Ok(None) => anchors.push(crate::transcript_anchors::AnchorWrite {
                            session_id: env.session_id.clone(),
                            key: key.to_owned(),
                            seq: env.seq,
                        }),
                        Err(error) => {
                            // Never block the transcript on the index: the row
                            // lands at its own seq and clients merge by id. Its
                            // position is not an anchor; the next write of the
                            // key resolves the durable one again.
                            tracing::warn!(%error, session = %env.session_id, key, "transcript anchor lookup failed");
                            unresolved = Some((env.session_id.clone(), key.to_owned()));
                        }
                    }
                }
                if let Some(reduced) = reducer.reduce(env) {
                    events.insert((reduced.session_id.clone(), reduced.seq), reduced);
                }
                if let Some((session_id, key)) = unresolved {
                    reducer.forget_anchor(&session_id, &key);
                }
            }
            StoreWrite::ClearEvents { ref session_id, .. } => {
                ok &= flush_event_batch(
                    store,
                    reducer,
                    &mut events,
                    &mut highwaters,
                    &mut anchors,
                    &mut occurrences,
                )
                .await;
                reducer.clear_session(session_id);
                ok &= retry_store_write(store, &write).await;
            }
            other => {
                ok &= flush_event_batch(
                    store,
                    reducer,
                    &mut events,
                    &mut highwaters,
                    &mut anchors,
                    &mut occurrences,
                )
                .await;
                ok &= retry_store_write(store, &other).await;
            }
        }
    }
    ok &= flush_event_batch(
        store,
        reducer,
        &mut events,
        &mut highwaters,
        &mut anchors,
        &mut occurrences,
    )
    .await;
    ok
}

pub(super) async fn record_lifecycle_incidents(
    store: &Store,
    rows: &[Envelope],
) -> anyhow::Result<()> {
    for envelope in rows {
        if let Event::TurnEnd { stop_reason, .. } = &envelope.event
            && let Some(raw_summary) = stop_reason.strip_prefix("error: ")
        {
            let occurred_at_ms = now_ms();
            let classification = classify_session_error(raw_summary);
            let (summary, truncated) = bounded_incident_summary(raw_summary);
            let fingerprint = format!(
                "{:x}",
                Sha256::digest(format!("{classification}:{raw_summary}").as_bytes())
            );
            let incident_id = format!("turn-error:{}:{}", envelope.session_id, envelope.seq);
            store
                .upsert_runtime_incident(&crate::store::RuntimeIncidentWrite {
                    id: incident_id.clone(),
                    occurred_at_ms,
                    source: "controller".to_owned(),
                    classification: classification.to_owned(),
                    severity: session_error_severity(classification).to_owned(),
                    state: "failed".to_owned(),
                    summary: summary.clone(),
                    fingerprint,
                    session_id: Some(envelope.session_id.clone()),
                    client_id: None,
                    machine_id: None,
                    trace_id: None,
                    build: Some(env!("CARGO_PKG_VERSION").to_owned()),
                    evidence_start_ms: occurred_at_ms.saturating_sub(30_000),
                    evidence_end_ms: occurred_at_ms.saturating_add(30_000),
                    detail: serde_json::json!({
                        "source": "turn_failure",
                        "turn_end_seq": envelope.seq,
                        "stop_reason": summary,
                        "truncated": truncated,
                    }),
                })
                .await?;
            store
                .record_occurrence(
                    &crate::task_notifications::Occurrence::incident(
                        &envelope.session_id,
                        &incident_id,
                    ),
                    occurred_at_ms,
                )
                .await?;
            tracing::error!(
                incident_id,
                session_id = %envelope.session_id,
                classification,
                error = %raw_summary,
                "agent turn failed"
            );
        }
        let Event::Lifecycle { status, detail } = &envelope.event else {
            continue;
        };
        if let Some(recovery_outcome) = lifecycle_incident_recovery_outcome(*status) {
            let recovered = store
                .recover_runtime_incident(&envelope.session_id, now_ms(), recovery_outcome)
                .await?;
            if recovered > 0 {
                tracing::info!(
                    session_id = %envelope.session_id,
                    recovery_outcome,
                    "runtime incident recovered"
                );
            }
            continue;
        }
        if !matches!(status, Status::Crashed | Status::Interrupted) {
            continue;
        }
        let occurred_at_ms = now_ms();
        let classification = if *status == Status::Interrupted {
            classify_interruption_detail(detail.as_deref())
        } else {
            classify_crash_detail(detail.as_deref())
        };
        let raw_summary = detail.as_deref().unwrap_or(if *status == Status::Crashed {
            "Runtime crashed without a diagnostic detail"
        } else {
            "Runtime was interrupted"
        });
        let (summary, truncated) = bounded_incident_summary(raw_summary);
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(format!("{classification}:{raw_summary}").as_bytes())
        );
        let incident_id = format!("lifecycle:{}:{}", envelope.session_id, envelope.seq);
        store
            .upsert_runtime_incident(&crate::store::RuntimeIncidentWrite {
                id: incident_id.clone(),
                occurred_at_ms,
                source: "controller".to_owned(),
                classification: classification.to_owned(),
                severity: if *status == Status::Crashed && classification != "provider_refusal" {
                    "critical".to_owned()
                } else {
                    "warning".to_owned()
                },
                state: "active".to_owned(),
                summary: summary.clone(),
                fingerprint,
                session_id: Some(envelope.session_id.clone()),
                client_id: None,
                machine_id: None,
                trace_id: None,
                build: Some(env!("CARGO_PKG_VERSION").to_owned()),
                evidence_start_ms: occurred_at_ms.saturating_sub(30_000),
                evidence_end_ms: occurred_at_ms.saturating_add(30_000),
                detail: serde_json::json!({
                    "status": status,
                    "lifecycle_seq": envelope.seq,
                    "detail": summary,
                    "truncated": truncated,
                }),
            })
            .await?;
        // A crash has nobody watching it end; an interruption is recovered.
        if *status == Status::Crashed {
            store
                .record_occurrence(
                    &crate::task_notifications::Occurrence::incident(
                        &envelope.session_id,
                        &incident_id,
                    ),
                    occurred_at_ms,
                )
                .await?;
        }
        tracing::error!(
            incident_id,
            session_id = %envelope.session_id,
            classification,
            lifecycle_seq = envelope.seq,
            detail = summary,
            "runtime incident opened"
        );
    }
    Ok(())
}

pub(super) fn classify_crash_detail(detail: Option<&str>) -> &'static str {
    if detail == Some(crate::core::MODEL_REFUSAL_DETAIL) {
        return "provider_refusal";
    }
    if detail.is_some_and(crate::provider_behavior::is_session_restore_timeout_error) {
        return "session_restore_timeout";
    }
    if detail.is_some_and(is_context_window_rejection) {
        return "provider_context_limit";
    }
    if detail.is_some_and(is_empty_stream_failure) {
        return "provider_empty_stream";
    }
    let detail = detail.unwrap_or_default().to_ascii_lowercase();
    if detail.contains("worker")
        && (detail.contains("before readiness")
            || detail.contains("did not become ready")
            || detail.contains("generation launch failed")
            || detail.contains("spawning worker")
            || detail.contains("transient worker unit"))
    {
        "worker_startup_failure"
    } else if detail.contains("oom")
        || detail.contains("out of memory")
        || detail.contains("signal: 9")
    {
        "resource_exhaustion"
    } else if detail.contains("protocol") || detail.contains("frame") || detail.contains("json-rpc")
    {
        "protocol_failure"
    } else if detail.contains("connection")
        || detail.contains("disconnected")
        || detail.contains("network error")
        || detail.contains("socket")
        || detail.contains("timed out")
    {
        "transport_failure"
    } else if detail.contains("exited")
        || detail.contains("exit status")
        || detail.contains("signal")
    {
        "process_exit"
    } else {
        "runtime_failure"
    }
}

pub(super) fn bounded_incident_summary(value: &str) -> (String, bool) {
    let mut end = value.len().min(4 * 1024);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    (value[..end].to_owned(), end < value.len())
}

pub(super) async fn retry_store_write(store: &Store, write: &StoreWrite) -> bool {
    let mut last_error = None;
    for attempt in 0..4 {
        match apply_store_write(store, write).await {
            Ok(()) => return true,
            Err(e) => {
                last_error = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(50 * (1 << attempt))).await;
            }
        }
    }
    if let Some(error) = last_error {
        tracing::error!(%error, ?write, "store writer exhausted intent retries");
    }
    false
}

pub(super) async fn apply_store_write(store: &Store, write: &StoreWrite) -> anyhow::Result<()> {
    match write {
        StoreWrite::InsertSession(meta) => store.insert_session(meta).await,
        StoreWrite::AppendEvent(_) => Ok(()),
        StoreWrite::UpdateStatus { session_id, status } => {
            store.update_status(session_id, *status).await
        }
        StoreWrite::RecordSessionError {
            id,
            session_id,
            occurred_at_ms,
            message,
        } => {
            let classification = classify_session_error(message);
            let fingerprint = format!(
                "{:x}",
                Sha256::digest(format!("{classification}:{message}").as_bytes())
            );
            store
                .upsert_runtime_incident(&crate::store::RuntimeIncidentWrite {
                    id: id.clone(),
                    occurred_at_ms: *occurred_at_ms,
                    source: "controller".to_owned(),
                    classification: classification.to_owned(),
                    severity: session_error_severity(classification).to_owned(),
                    state: "failed".to_owned(),
                    summary: message.clone(),
                    fingerprint,
                    session_id: Some(session_id.clone()),
                    client_id: None,
                    machine_id: None,
                    trace_id: None,
                    build: Some(env!("CARGO_PKG_VERSION").to_owned()),
                    evidence_start_ms: occurred_at_ms.saturating_sub(30_000),
                    evidence_end_ms: occurred_at_ms.saturating_add(30_000),
                    detail: serde_json::json!({ "source": "session_error" }),
                })
                .await?;
            store
                .record_occurrence(
                    &crate::task_notifications::Occurrence::incident(session_id, id),
                    now_ms(),
                )
                .await
        }
        StoreWrite::RecordOccurrence(occurrence) => {
            store.record_occurrence(occurrence, now_ms()).await
        }
        StoreWrite::UpdateTitle { session_id, title } => {
            store.update_title(session_id, title).await
        }
        StoreWrite::UpdateCwd {
            session_id,
            cwd,
            title,
        } => store.update_cwd(session_id, cwd, title.as_deref()).await,
        StoreWrite::ReloadProvider(meta) => store.reload_provider(meta).await,
        StoreWrite::SetAgentSessionId {
            session_id,
            agent_session_id,
        } => {
            store
                .update_agent_session_id(session_id, agent_session_id.as_deref())
                .await
        }
        StoreWrite::UpdateConfigOptions {
            session_id,
            options,
        } => store.update_config_options(session_id, options).await,
        StoreWrite::UpdateConfigPreferences {
            session_id,
            preferences,
        } => {
            store
                .update_config_preferences(session_id, preferences)
                .await
        }
        StoreWrite::ClearEvents {
            session_id,
            calls_before_ms,
        } => store.clear_events(session_id, *calls_before_ms).await,
        StoreWrite::DeleteSession(id) => store.delete_session(id).await,
        StoreWrite::UpdatePending {
            session_id,
            queue,
            drafts,
        } => store.update_pending(session_id, queue, drafts).await,
        StoreWrite::UpdateSessionOrder { order } => store.update_session_order(order).await,
        StoreWrite::UpdateWorkspaceOrder { owner, order } => {
            store.update_workspace_order(owner, order).await
        }
        StoreWrite::ReplaceSessionFolders {
            owner_user_id,
            folders,
        } => {
            store
                .replace_session_folders(owner_user_id.as_deref(), folders)
                .await
        }
        StoreWrite::UpdateSessionPlacement { placements } => {
            store.update_session_placement(placements).await
        }
        StoreWrite::SetSessionsTrashed {
            session_ids,
            trashed_at_ms,
        } => {
            store
                .set_sessions_trashed(session_ids, *trashed_at_ms)
                .await
        }
        StoreWrite::UpdateMobileReviewState { session_id, value } => {
            store.update_mobile_review_state(session_id, value).await
        }
        StoreWrite::UpsertWakeup {
            session_id,
            fire_at_ms,
            prompt,
        } => store.upsert_wakeup(session_id, *fire_at_ms, prompt).await,
        StoreWrite::DeleteWakeup { session_id } => store.delete_wakeup(session_id).await,
        StoreWrite::PutSetting { key, value } => store.put_setting(key, value).await,
    }
}

/// Ask one Machine to retire each active Provider's credential generations
/// that no session pins. A Machine without the adapter refuses the request;
/// that is expected until it is upgraded and only logged at debug level.
pub(super) async fn retire_auth_generations(
    state: &AppState,
    machine: &crate::machine_protocol::MachineSummary,
    referenced: &HashMap<(String, String), BTreeSet<u64>>,
) {
    let providers = machine.plugins.iter().filter(|plugin| {
        plugin.plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider
            && plugin.state == crate::machine_protocol::PluginInstallationState::Active
            && plugin.auth_generation.is_some()
    });
    for plugin in providers {
        let mut pinned = referenced
            .get(&(machine.id.clone(), plugin.plugin_id.clone()))
            .cloned()
            .unwrap_or_default();
        // A session registered after the reference snapshot is admitted on
        // the generation current in this Machine snapshot; keep it even if a
        // refresh advances the Machine before that session launches.
        pinned.extend(plugin.auth_generation);
        let request = crate::generation_retention::AuthRequest {
            provider_id: plugin.plugin_id.clone(),
            referenced: pinned,
        };
        if !request.is_valid() {
            tracing::warn!(machine = %machine.id, provider = %request.provider_id, "skipping invalid credential retention request");
            continue;
        }
        let Ok(payload) = serde_json::to_value(&request) else {
            continue;
        };
        match state
            .machine_control
            .adapter_request(
                &machine.id,
                crate::generation_retention::AUTH_ADAPTER,
                payload,
            )
            .await
            .and_then(|value| {
                serde_json::from_value::<crate::generation_retention::AuthOutcome>(value)
                    .map_err(|error| error.to_string())
            }) {
            Ok(outcome) if !outcome.retired.is_empty() => tracing::info!(
                machine = %machine.id,
                provider = %plugin.plugin_id,
                retired = outcome.retired.len(),
                retained = outcome.retained.len(),
                freed_bytes = outcome.freed_bytes,
                "retired unreferenced Provider credential generations"
            ),
            Ok(_) => {}
            Err(error) => tracing::debug!(
                machine = %machine.id,
                provider = %plugin.plugin_id,
                %error,
                "Provider credential generation retention unavailable"
            ),
        }
    }
}

/// Run a retention pass every `plugins.generation_retention_interval`, and an
/// extra pass for one Device as soon as its available disk falls below the
/// watermark it declared, at most once per `plugins.urgent_retention_cooldown`.
/// Both settings are re-read live.
pub(super) async fn run_generation_retention(state: Arc<AppState>) {
    tokio::time::sleep(GENERATION_RETENTION_DELAY).await;
    let mut last_pass: Option<tokio::time::Instant> = None;
    let mut urgent_passes: HashMap<String, tokio::time::Instant> = HashMap::new();
    loop {
        let interval = state
            .service_config
            .get(&crate::config::schema::PLUGIN_GENERATION_RETENTION_INTERVAL);
        if last_pass.is_none_or(|at| at.elapsed() >= interval) {
            last_pass = Some(tokio::time::Instant::now());
            // Re-pin long-dormant sessions first so this pass can retire the
            // generations they no longer hold.
            dormant_pins::repin(&state).await;
            generation_retention_pass(&state, None).await;
            catalog_retention_pass(&state).await;
            adopt_shared_provider_authentication(&state).await;
        } else {
            let cooldown = state
                .service_config
                .get(&crate::config::schema::PLUGIN_URGENT_RETENTION_COOLDOWN);
            for (machine, observed) in state.machine_control.below_disk_watermark() {
                if urgent_passes
                    .get(&machine)
                    .is_some_and(|at| at.elapsed() < cooldown)
                {
                    continue;
                }
                tracing::warn!(
                    %machine,
                    disk_available_bytes = observed.resources.disk_available_bytes,
                    disk_low_watermark_bytes = ?observed.resources.disk_low_watermark_bytes,
                    "Device disk is below its low watermark; retiring Plugin generations now"
                );
                urgent_passes.insert(machine.clone(), tokio::time::Instant::now());
                generation_retention_pass(&state, Some(&machine)).await;
            }
        }
        tokio::time::sleep(URGENT_RETENTION_POLL).await;
    }
}

/// Ask each connected Machine (or only `only`) to retire Agent Provider
/// generations no recoverable session pins. The Machine keeps the active and
/// rollback generations itself and refuses while an installation is
/// unreconciled; the same request also prunes its unreferenced artifact cache.
/// Ask one Machine to retire generations left by Plugins it no longer has
/// installed, keeping those still pinned and any saga that may compensate.
pub(super) async fn retire_uninstalled_generations(
    state: &AppState,
    machine_id: &str,
    referenced: &HashMap<(String, String), BTreeSet<String>>,
    unfinished: &[(String, String)],
) {
    let request = crate::generation_retention::UninstalledRequest {
        referenced: referenced
            .iter()
            .filter(|((machine, _), _)| machine == machine_id)
            .map(|((_, plugin), digests)| (plugin.clone(), digests.clone()))
            .collect(),
        skip: unfinished
            .iter()
            .filter(|(machine, _)| machine == machine_id)
            .map(|(_, plugin)| plugin.clone())
            .collect(),
    };
    if !request.is_valid() {
        tracing::warn!(machine = %machine_id, "skipping invalid uninstalled generation retention request");
        return;
    }
    let Ok(payload) = serde_json::to_value(&request) else {
        return;
    };
    match state
        .machine_control
        .adapter_request(
            machine_id,
            crate::generation_retention::UNINSTALLED_ADAPTER,
            payload,
        )
        .await
        .and_then(|value| {
            serde_json::from_value::<Vec<crate::generation_retention::Outcome>>(value)
                .map_err(|error| error.to_string())
        }) {
        Ok(outcomes) => {
            for outcome in outcomes {
                tracing::info!(
                    machine = %machine_id,
                    plugin = %outcome.plugin_id,
                    retired = ?outcome.retired,
                    freed_bytes = outcome.freed_bytes,
                    "retired uninstalled Plugin generations"
                );
            }
        }
        // Machines released before this adapter refuse it; that is expected
        // until they upgrade, so it is not a warning.
        Err(error) => tracing::debug!(
            machine = %machine_id,
            %error,
            "uninstalled Plugin generation retention unavailable"
        ),
    }
}

/// Retire Service Catalog releases nothing resolves any more; the keep set is
/// every identity a session, a Device or an install operation still names.
pub(super) async fn catalog_retention_pass(state: &AppState) {
    let keep_releases = state
        .service_config
        .get(&crate::config::schema::PLUGIN_CATALOG_KEEP_RELEASES);
    if keep_releases <= 0 {
        return;
    }
    let Some(store) = state.store.as_ref() else {
        return;
    };
    let mut keep = crate::plugin_catalog::CatalogKeepSet::default();
    for session in state.hub.session_list() {
        keep.insert(&session.provider_generation_digest);
    }
    match store.deleted_session_generations().await {
        Ok(deleted) => {
            for (_, _, digest) in deleted {
                keep.insert(&digest);
            }
        }
        Err(error) => {
            tracing::warn!(%error, "skipping Plugin Catalog retention");
            return;
        }
    }
    match store.plugin_install_generation_digests().await {
        Ok(digests) => {
            for digest in digests {
                keep.insert(&digest);
            }
        }
        Err(error) => {
            tracing::warn!(%error, "skipping Plugin Catalog retention");
            return;
        }
    }
    // Named logins are verified against their signed-in release when a
    // compatible later release uses them; never retire that manifest.
    match store.native_login_generation_digests().await {
        Ok(digests) => {
            for digest in digests {
                keep.insert(&digest);
            }
        }
        Err(error) => {
            tracing::warn!(%error, "skipping Plugin Catalog retention");
            return;
        }
    }
    let Ok(machines) = state.machine_snapshots.load().await else {
        return;
    };
    for machine in &machines {
        for plugin in &machine.plugins {
            keep.insert(&plugin.generation_digest);
            if let Some(digest) = &plugin.rollback_generation_digest {
                keep.insert(digest);
            }
        }
    }
    let referenced = keep.len();
    match state
        .plugin_catalog
        .retire_unreferenced(keep, keep_releases)
        .await
    {
        Ok(outcome) if outcome.retired.is_empty() && outcome.artifact_directories_removed == 0 => {}
        Ok(outcome) => {
            tracing::info!(
                retired = ?outcome.retired,
                retained = outcome.retained,
                referenced,
                artifact_directories_removed = outcome.artifact_directories_removed,
                freed_bytes = outcome.freed_bytes,
                "retired unreferenced Plugin Catalog releases"
            );
            if let Err(error) = state
                .plugin_catalog
                .refresh_with_runtime(&state.plugin_storage)
                .await
            {
                tracing::warn!(%error, "Plugin Catalog refresh after retention failed");
            } else if let Err(error) = state.provider_catalog.refresh_external() {
                tracing::warn!(%error, "Provider Catalog refresh after retention failed");
            }
        }
        Err(error) => tracing::warn!(%error, "Plugin Catalog retention failed"),
    }
}

pub(super) async fn generation_retention_pass(state: &AppState, only: Option<&str>) {
    let Ok(machines) = state.machine_snapshots.load().await else {
        return;
    };
    let mut referenced = referenced_generations(&state.hub.session_list());
    // Soft-deleted sessions stay recoverable until purged. Without that
    // list a pass could retire their generations, so skip it entirely.
    let Some(store) = state.store.as_ref() else {
        return;
    };
    match store.deleted_session_generations().await {
        Ok(deleted) => {
            for (machine_id, provider, digest) in deleted {
                referenced
                    .entry((machine_id, provider))
                    .or_default()
                    .insert(digest);
            }
        }
        Err(error) => {
            tracing::warn!(%error, "skipping Plugin generation retention");
            return;
        }
    }
    // A worker the Machine reports can still run a generation its session
    // record no longer pins: a cutover relaunched it on its declared
    // generation, or a surviving worker is reconciled only on reconnect.
    // Keep what every observed worker was launched with.
    for machine in &machines {
        if let Some(runtime) = state.runtime_router.runtime(&machine.id) {
            for (provider, digest) in runtime.worker_provider_generations() {
                referenced
                    .entry((machine.id.clone(), provider))
                    .or_default()
                    .insert(digest);
            }
        }
    }
    // Credential generations pinned by every unpurged session row joined with
    // the Hub (a session not yet persisted); an unreadable store skips
    // credential retention for this pass.
    let hub_auth = referenced_auth_generations(&state.hub.session_list());
    let auth_referenced = match store.session_auth_generations().await {
        Ok(stored) => {
            let mut referenced = hub_auth;
            for (machine_id, provider, generation) in stored {
                if let Ok(generation) = u64::try_from(generation) {
                    referenced
                        .entry((machine_id, provider))
                        .or_default()
                        .insert(generation);
                }
            }
            Some(referenced)
        }
        Err(error) => {
            tracing::warn!(%error, "skipping Provider credential generation retention");
            None
        }
    };
    // An unreadable uninstall ledger only skips uninstalled-Plugin cleanup.
    let unfinished_uninstalls = match store.unfinished_plugin_uninstalls().await {
        Ok(pairs) => Some(pairs),
        Err(error) => {
            tracing::warn!(%error, "skipping uninstalled Plugin generation retention");
            None
        }
    };
    for machine in machines {
        if only.is_some_and(|id| id != machine.id) {
            continue;
        }
        if !machine.connected || !machine.capabilities.hibernation {
            // Machines older than protocol 26 lack the retention adapter.
            continue;
        }
        let providers: Vec<String> = machine
            .plugins
            .iter()
            .filter(|plugin| {
                plugin.plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider
                    && plugin.state == crate::machine_protocol::PluginInstallationState::Active
            })
            .map(|plugin| plugin.plugin_id.clone())
            .collect();
        for plugin_id in providers {
            let request = crate::generation_retention::Request {
                referenced: referenced
                    .get(&(machine.id.clone(), plugin_id.clone()))
                    .cloned()
                    .unwrap_or_default(),
                plugin_id,
            };
            if !request.is_valid() {
                tracing::warn!(machine = %machine.id, plugin = %request.plugin_id, "skipping invalid generation retention request");
                continue;
            }
            let plugin_id = request.plugin_id.clone();
            let payload = match serde_json::to_value(&request) {
                Ok(payload) => payload,
                Err(_) => continue,
            };
            match state
                .machine_control
                .adapter_request(&machine.id, crate::generation_retention::ADAPTER, payload)
                .await
                .and_then(|value| {
                    serde_json::from_value::<crate::generation_retention::Outcome>(value)
                        .map_err(|error| error.to_string())
                }) {
                Ok(outcome) if !outcome.retired.is_empty() => tracing::info!(
                    machine = %machine.id,
                    plugin = %plugin_id,
                    retired = ?outcome.retired,
                    freed_bytes = outcome.freed_bytes,
                    "retired unreferenced Plugin generations"
                ),
                Ok(_) => {}
                Err(error) => tracing::warn!(
                    machine = %machine.id,
                    plugin = %plugin_id,
                    %error,
                    "Plugin generation retention failed"
                ),
            }
        }
        if let Some(auth_referenced) = &auth_referenced {
            retire_auth_generations(state, &machine, auth_referenced).await;
        }
        if let Some(unfinished) = &unfinished_uninstalls {
            retire_uninstalled_generations(state, &machine.id, &referenced, unfinished).await;
        }
    }
}

/// Drain the Hub→dispatcher channel: each [`DispatchReq`] is a queued prompt the
/// Hub decided is ready to send. We forward it to the session's agent. On
/// success, derive the auto-title from the first prompt (a no-op after the
/// first); on failure, clear the in-flight guard (so the queue can keep
/// draining) and surface the error to every client.
pub(super) async fn run_dispatcher(
    state: Arc<AppState>,
    mut rx: mpsc::Receiver<DispatchReq>,
    mut shutdown: watch::Receiver<bool>,
) {
    // A slow cold target must not hold up prompts for other sessions. Hub
    // admission still permits only one outstanding dispatch per session.
    let mut dispatches = tokio::task::JoinSet::new();
    loop {
        let req = tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { rx.close(); }
                rx.recv().await
            }
            result = dispatches.join_next(), if !dispatches.is_empty() => {
                if let Some(Err(error)) = result { tracing::error!(%error, "dispatch task failed"); }
                continue;
            }
            req = rx.recv(), if dispatches.len() < 32 => req,
        };
        let Some(req) = req else { break };
        dispatches.spawn(dispatch_one(Arc::clone(&state), req));
    }
    while let Some(result) = dispatches.join_next().await {
        if let Err(error) = result {
            tracing::error!(%error, "dispatch task failed while draining");
        }
    }
    tracing::info!("dispatcher shutting down (channel closed)");
}

pub(super) async fn dispatch_preflight<T>(
    shutdown: &mut watch::Receiver<bool>,
    preparation: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::select! {
        biased;
        _ = shutdown.wait_for(|stopping| *stopping) => {
            Err("Controller disconnected for shutdown; prompt remains queued".into())
        }
        result = preparation => result,
    }
}

pub(super) async fn dispatch_one(state: Arc<AppState>, req: DispatchReq) {
    if req.ownership.supervision().is_some() {
        superagent::dispatch(state, req).await;
        return;
    }
    let hub = &state.hub;
    let supervisor = &state.supervisor;
    let plugin_lifecycle_fences = &state.plugin_lifecycle_fences;
    let observability = &state.observability;
    let DispatchReq {
        ownership: _ownership,
        session_id,
        text,
        content,
        cmid,
        explicit,
    } = req;
    let retain = |session_id: String, text, content, cmid, error: &str| {
        let session = session_id.clone();
        retain_failed_dispatch(hub, session_id, text, content, cmid, error);
        if explicit {
            retain_explicit_intent(hub, &session, error);
        }
    };
    // A user send retries a preparation that failed permanently; the
    // preflight below then holds the prompt until the target is ready.
    if explicit {
        execution::sessions::retry_blocked(&state, &session_id);
    }
    let mut shutdown = state.shutdown.clone();
    let recovery = match dispatch_preflight(
        &mut shutdown,
        execution::recovery::before_dispatch(&state, &session_id),
    )
    .await
    {
        Ok(recovery) => recovery,
        Err(error) => {
            retain(session_id, text, content, cmid, &error);
            return;
        }
    };
    // A failed or interrupted Web workspace preparation left this session on
    // its source checkout; prepare it again before any worker can start.
    // Completion drains the queue; an explicit intent would re-drain it on
    // every maintenance tick while the Machine is still preparing.
    if let Some(wait) = workspace_preparation::resume(&state, &session_id) {
        retain_failed_dispatch(hub, session_id, text, content, cmid, &wait);
        return;
    }
    // Links wait for a real message: a preamble would break a slash command.
    if !crate::session_context::is_command(&text)
        && let Err(error) =
            session_context::claim_carrier(&state, &session_id, cmid.as_deref()).await
    {
        retain(session_id, text, content, cmid, &error);
        return;
    }
    let plugin_key = provider_fence_key_for_session(hub, &session_id);
    let dispatch_fence = plugin_lifecycle_fences.read();
    if plugin_key
        .as_ref()
        .and_then(|key| dispatch_fence.get(key))
        .is_some_and(|state| *state != PluginFenceState::Installing)
    {
        hub.requeue_prompt(&session_id, text, content, cmid);
        return;
    }
    let Some(blocks) = build_prompt_blocks(&text, &content) else {
        tracing::warn!(session = %session_id, "queued prompt had no content; dropping");
        hub.clear_in_flight(&session_id);
        return;
    };
    let title = first_prompt_title(&text, &content);
    let trace = observability.dispatch_trace(&session_id, cmid.as_deref());
    let sent = supervisor.send_traced_after_execution_recovery(
        &session_id,
        AgentCommand::Prompt(blocks, cmid.clone(), None),
        trace.clone(),
        recovery,
    );
    match sent {
        Ok(()) => {
            // The message reached the agent: the latest in-place agent change
            // is now part of the conversation, so Settings loses its Back.
            config_revert::clear(hub, &session_id);
            // Successful sends synchronously enter the runtime pending set.
            // Failed sends retain dispatch ownership until requeue completes.
            drop(_ownership);
            session_maintenance::clear_wait(hub, &session_id);
            if let Some(t) = title {
                hub.auto_title(&session_id, t);
            }
        }
        Err(e) => {
            if let Some(trace) = &trace {
                observability.finish_delivery_trace(trace, false);
            }
            tracing::warn!(session = %session_id, error = %e, "queued dispatch failed");
            retain(session_id, text, content, cmid, &e);
        }
    }
}

/// Restore a prompt that left the durable queue but never reached its runtime.
///
/// A remote machine can disconnect between queue drain and dispatch (notably
/// while the controller is restarting). Keeping the original `cmid` makes the
/// restore idempotent and lets every client reconcile the same queued item.
pub(super) fn retain_failed_dispatch(
    hub: &Hub,
    session_id: String,
    text: String,
    content: Vec<serde_json::Value>,
    cmid: Option<String>,
    error: &str,
) {
    hub.requeue_prompt(&session_id, text, content, cmid);
    if let Some(reason) = error.strip_prefix(session_maintenance::WAIT_PREFIX) {
        session_maintenance::wait(hub, &session_id, reason);
        return;
    }
    // A Controller restart authenticates the Machine WebSocket before the
    // ACP runtime handshake finishes. The prompt is durable; do not toast a
    // transient "not connected" as a user-facing send failure.
    if is_transient_machine_disconnect(error) {
        tracing::warn!(
            session = %session_id,
            error = %error,
            "queued dispatch deferred until Machine runtime is ready"
        );
        return;
    }
    // A refusal that is not a wait ends an earlier retry: the prompt stays
    // queued for explicit intervention instead of failing every two seconds.
    session_maintenance::clear_wait(hub, &session_id);
    // Restore-timeout holds already explain the queue and the next step.
    // Prefixing "send failed:" makes a protective refusal look like a
    // transport crash.
    let message = if crate::provider_behavior::is_session_restore_timeout_error(error) {
        error.to_owned()
    } else {
        format!("send failed: {error}")
    };
    hub.broadcast_error(Some(session_id), message);
}

pub(super) fn recoverable_provider_auth_failure(
    hub: &Hub,
    session_id: &str,
) -> Option<((Status, u64), String)> {
    let revision @ (status, _) = hub.status_revision(session_id)?;
    if !matches!(status, Status::Crashed | Status::Running) {
        return None;
    }
    let detail = hub.unresolved_crash_detail(session_id)?;
    crate::provider_behavior::is_provider_auth_required_error(&detail).then_some((revision, detail))
}

pub(super) fn recover_failed_provider_sessions(
    state: &AppState,
    statuses: &[crate::provider_service::ProviderAuthenticationStatus],
    replicas: &BTreeMap<String, ProviderDistributionOutcome>,
) -> Vec<String> {
    let mut recovered = Vec::new();
    for session in state.hub.session_list() {
        let Some((status_revision, crash_detail)) =
            recoverable_provider_auth_failure(&state.hub, &session.id)
        else {
            continue;
        };
        let Some(status) = statuses.iter().find(|status| {
            status.provider_id == session.provider
                && status.account_id == session.provider_account_id
        }) else {
            continue;
        };
        let Some(outcome) = replicas.get(&session.provider) else {
            continue;
        };
        let Some(package) = state.provider_catalog.package(
            &session.provider,
            &session.provider_version,
            &session.provider_generation_digest,
        ) else {
            continue;
        };
        let Ok(fingerprint) = crate::provider_account::portable_contract::fingerprint(&package)
        else {
            continue;
        };
        let Some((expected_generation, next_generation)) = provider_auth_recovery_generations(
            &session,
            status,
            &outcome.synchronized_generations,
            &fingerprint,
        ) else {
            continue;
        };
        match state.hub.begin_provider_auth_recovery(
            &session.id,
            status_revision,
            &crash_detail,
            expected_generation,
            next_generation,
        ) {
            Ok(true) => {
                if let Err(error) = state.supervisor.reload_session(&session.id, false) {
                    tracing::warn!(
                        session = %session.id,
                        provider = %session.provider,
                        %error,
                        "reloading session after Provider reauthorization"
                    );
                    state
                        .hub
                        .set_status(&session.id, Status::Crashed, Some(crash_detail.clone()));
                }
                recovered.push(session.id);
            }
            Ok(false) => {}
            Err(error) => tracing::warn!(
                session = %session.id,
                provider = %session.provider,
                %error,
                "reserving refreshed Provider authentication recovery"
            ),
        }
    }
    if !recovered.is_empty() {
        tracing::info!(sessions = ?recovered, "recovered failed sessions with refreshed Provider authentication");
    }
    recovered
}

pub(super) fn is_context_window_rejection(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    detail.contains("maximum context length")
        || detail.contains("prompt is too long")
        || (detail.contains("context window")
            && (detail.contains("exceed")
                || detail.contains("full")
                || detail.contains("limit reached")))
}

pub(super) fn is_empty_stream_failure(detail: &str) -> bool {
    detail
        .to_ascii_lowercase()
        .contains("stream ended without receiving any events")
}

pub(super) fn classify_session_error(detail: &str) -> &'static str {
    match classify_crash_detail(Some(detail)) {
        "runtime_failure" => "session_command_error",
        classification => classification,
    }
}

pub(super) fn session_error_severity(classification: &str) -> &'static str {
    match classification {
        "worker_startup_failure" | "process_exit" | "resource_exhaustion" | "runtime_failure" => {
            "critical"
        }
        _ => "error",
    }
}

pub(super) fn classify_interruption_detail(detail: Option<&str>) -> &'static str {
    let detail = detail.unwrap_or_default().to_ascii_lowercase();
    if detail.contains("deploy")
        || detail.contains("shutdown")
        || detail.contains("controller restart")
    {
        "expected_interruption"
    } else {
        "runtime_interruption"
    }
}
