//! Health, version, metrics, observability ingestion, diagnostic logs and the
//! usage broadcast.

use super::*;

pub(super) async fn healthz(State(state): State<Arc<AppState>>) -> Response {
    if !state.runtime_health.is_healthy(state.store.is_some()) {
        return (StatusCode::SERVICE_UNAVAILABLE, "background task degraded").into_response();
    }
    if state
        .persistence_health
        .as_ref()
        .is_some_and(|health| !health.is_healthy())
    {
        (StatusCode::SERVICE_UNAVAILABLE, "persistence degraded").into_response()
    } else {
        "ok".into_response()
    }
}

/// A build identifier the SPA polls to detect a frontend rollout. `index.html`
/// references the content-hashed JS/CSS bundles, so hashing it keeps `/version`
/// aligned with the files currently exposed through the stable web-root path.
pub(super) async fn version(State(state): State<Arc<AppState>>) -> Response {
    let web_root = state
        .web_roots
        .observe()
        .unwrap_or_else(|| state.web_root.clone());
    match tokio::fs::read(web_root.join("index.html")).await {
        Ok(bytes) => Json(VersionResponse {
            version: content_hash(&bytes),
        })
        .into_response(),
        Err(error) => {
            tracing::warn!(%error, web_root = %state.web_root.display(), "reading web version failed");
            (StatusCode::NOT_FOUND, "UI not built").into_response()
        }
    }
}

/// Current and lifetime-high resident set of THIS process (not its agent or
/// usage-collector children). `/proc/self/status` reports KiB directly, avoiding
/// the incorrect 4 KiB page-size assumption `statm` would make on some hosts.
pub(super) fn daemon_memory() -> DaemonMemory {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .map_or_else(DaemonMemory::default, |status| {
            parse_proc_status_memory(&status)
        })
}

pub(super) fn parse_proc_status_memory(status: &str) -> DaemonMemory {
    fn kib_value(line: &str, key: &str) -> Option<u64> {
        line.strip_prefix(key)?
            .split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
            .map(|kib| kib.saturating_mul(1024))
    }

    let mut memory = DaemonMemory::default();
    for line in status.lines() {
        if let Some(bytes) = kib_value(line, "VmRSS:") {
            memory.rss_bytes = bytes;
        } else if let Some(bytes) = kib_value(line, "VmHWM:") {
            memory.rss_peak_bytes = bytes;
        }
    }
    memory
}

pub(super) async fn api_metrics(State(state): State<Arc<AppState>>) -> Response {
    let sessions_live = state.hub.session_list().len();
    let (db_bytes, events_rows, sessions_deleted) = match &state.store {
        Some(s) => s.observed_storage_metrics().await.unwrap_or((0, 0, 0)),
        None => (0, i64::try_from(state.hub.event_total()).unwrap_or(0), 0),
    };
    let runtime = state.runtime_router.stats();
    let code_cache = state.code_cache.metrics();
    let daemon_memory = daemon_memory();
    let cgroup_memory = crate::memory_observability::own_cgroup_memory().unwrap_or_default();
    let hub_memory = state.hub.memory_stats();
    Json(Metrics {
        db_bytes,
        events_rows,
        sessions_live,
        sessions_deleted,
        daemon_rss_bytes: daemon_memory.rss_bytes,
        daemon_rss_peak_bytes: daemon_memory.rss_peak_bytes,
        cgroup_memory_bytes: cgroup_memory.current_bytes,
        cgroup_memory_peak_bytes: cgroup_memory.peak_bytes,
        persistence_pending: state.persistence_health.as_ref().map_or(0, |h| h.pending()),
        persistence_pending_bytes: state
            .persistence_health
            .as_ref()
            .map_or(0, |h| h.pending_bytes()),
        persistence_dropped: state.persistence_health.as_ref().map_or(0, |h| h.dropped()),
        persistence_failed_batches: state
            .persistence_health
            .as_ref()
            .map_or(0, |h| h.failed_batches()),
        persistence_last_error: state
            .persistence_health
            .as_ref()
            .and_then(|h| h.last_error()),
        runtime_connected: state.runtime_router.has_connected_runtime(),
        runtime_workers: runtime.workers,
        runtime_busy_workers: runtime.busy_workers,
        runtime_draining_workers: runtime.draining_workers,
        runtime_handoff_workers: runtime.handoff_workers,
        runtime_pending_commands: runtime.pending_commands,
        code_cache_bytes: code_cache.bytes,
        code_cache_hits: code_cache.hits,
        code_cache_misses: code_cache.misses,
        code_cache_evictions: code_cache.evictions,
        observability_pending: state.observability.health().pending(),
        observability_pending_bytes: state.observability.health().pending_bytes(),
        observability_duplicate_batches: state.observability.health().duplicate_batches(),
        observability_failed_file_batches: state.observability.health().failed_file_batches(),
        observability_failed_incident_batches: state
            .observability
            .health()
            .failed_incident_batches(),
        observability_dropped_export_batches: state.observability.health().dropped_export_batches(),
        observability_accepted_batches: state.observability.health().accepted_batches(),
        observability_dropped_batches: state.observability.health().dropped_batches(),
        observability_failed_log_batches: state.observability.health().failed_log_batches(),
        observability_failed_metric_batches: state.observability.health().failed_metric_batches(),
        observability_failed_trace_batches: state.observability.health().failed_trace_batches(),
        observability_rejected_export_items: state.observability.health().rejected_export_items(),
        hub_session_count: hub_memory.session_count,
        hub_hot_log_bytes: hub_memory.hot_log_bytes,
        hub_broadcast_last_bytes: hub_memory.broadcast_last_bytes,
    })
    .into_response()
}

pub(super) async fn api_observability_batch(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Json(mut batch): Json<TelemetryBatch>,
) -> Response {
    if batch
        .context
        .session_id
        .as_deref()
        .is_some_and(|session| !session_is_visible(&state.hub, &authenticated.principal, session))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    // The browser cannot forge a different Machine association for a session.
    batch.context.machine_id = batch
        .context
        .session_id
        .as_deref()
        .and_then(|session| state.hub.session_info(session))
        .map(|session| session.meta.machine_id);
    match state
        .observability
        .submit(&authenticated.principal.user_id, batch)
    {
        Ok(()) => (StatusCode::ACCEPTED, Json(SubmitReceipt { accepted: true })).into_response(),
        Err(message)
            if matches!(
                message,
                "observability queue full" | "observability unavailable"
            ) =>
        {
            (StatusCode::SERVICE_UNAVAILABLE, message).into_response()
        }
        Err(message) => (StatusCode::BAD_REQUEST, message).into_response(),
    }
}

pub(super) async fn api_observability_incidents(State(state): State<Arc<AppState>>) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "incident ledger unavailable",
        )
            .into_response();
    };
    match store.runtime_incidents(200).await {
        Ok(incidents) => Json(incidents).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) async fn api_otlp(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Path(signal): Path<crate::otlp::Signal>,
    Query(query): Query<OtlpQuery>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        != Some("application/x-protobuf")
        || headers.contains_key(axum::http::header::CONTENT_ENCODING)
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    match state.observability.submit_otlp(
        &authenticated.principal.user_id,
        signal,
        &query.batch_id,
        &body,
    ) {
        // All three standard Export*ServiceResponse messages encode a complete
        // success as empty protobuf. Acknowledgement means bounded admission,
        // not guaranteed persistence or delivery by the optional exporter.
        Ok(()) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/x-protobuf")],
            Vec::<u8>::new(),
        )
            .into_response(),
        Err("observability queue full" | "observability unavailable") => {
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
        Err(_) => StatusCode::BAD_REQUEST.into_response(),
    }
}

/// Fan every usage change out to all clients. A short settle window folds the
/// start and end of concurrent collections into fewer frames.
pub(super) async fn run_usage_broadcast(state: Arc<AppState>) {
    let mut changes = state.usage.subscribe();
    while changes.changed().await.is_ok() {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        changes.borrow_and_update();
        let bindings = state.usage.plugin_bindings();
        let snapshot = crate::usage::with_session_usage(
            state.usage.snapshot().await,
            &state.hub.session_list(),
            &state.provider_catalog,
            &bindings,
        );
        match serde_json::to_value(snapshot) {
            Ok(snapshot) => state.hub.broadcast_usage(snapshot),
            Err(error) => tracing::warn!(%error, "encoding usage broadcast"),
        }
    }
}

pub(super) fn selected_activity_filters(
    value: Option<&str>,
    allowed: &[&str],
) -> Result<Vec<String>, &'static str> {
    let Some(value) = value.filter(|value| !value.is_empty() && *value != "all") else {
        return Ok(Vec::new());
    };
    let mut selected = Vec::new();
    for item in value.split(',') {
        if item == "all" || !allowed.contains(&item) {
            return Err("invalid activity filter");
        }
        if !selected.iter().any(|existing| existing == item) {
            selected.push(item.to_owned());
        }
    }
    Ok(selected)
}

pub(super) fn parse_usage_activity_filter(
    query: &UsageActivityQuery,
    allowed_models: &[&str],
    allowed_agents: &[&str],
) -> Result<UsageActivityFilter, &'static str> {
    let now = now_ms();
    let (window, from_ms, to_ms) = match (query.from_ms, query.to_ms) {
        (Some(from_ms), Some(to_ms))
            if from_ms > 0
                && to_ms > from_ms
                && to_ms <= now.saturating_add(5 * 60_000)
                && to_ms.saturating_sub(from_ms) <= 30 * 86_400_000 =>
        {
            ("custom".to_owned(), from_ms, to_ms)
        }
        (Some(_), Some(_)) => return Err("invalid activity time range"),
        (None, None) => {
            let window = query.window.as_deref().unwrap_or("24h");
            let window_seconds = match window {
                "1h" => 3_600,
                "2h" => 2 * 3_600,
                "4h" => 4 * 3_600,
                "6h" => 6 * 3_600,
                "8h" => 8 * 3_600,
                "12h" => 12 * 3_600,
                "24h" => 24 * 3_600,
                "7d" => 7 * 86_400,
                "14d" => 14 * 86_400,
                "30d" => 30 * 86_400,
                _ => return Err("invalid activity window"),
            };
            (
                window.to_owned(),
                now.saturating_sub(i64::from(window_seconds) * 1_000),
                now,
            )
        }
        _ => return Err("activity time range requires both boundaries"),
    };
    Ok(UsageActivityFilter {
        window,
        from_ms,
        to_ms,
        models: selected_activity_filters(query.model.as_deref(), allowed_models)?,
        agents: selected_activity_filters(query.agent.as_deref(), allowed_agents)?,
    })
}

pub(super) fn new_reset_idempotency_key() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "cowboy-reset-{}-{}-{}",
        std::process::id(),
        now_ms(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    )
}

pub(super) fn reset_provider_supported(state: &AppState, provider: &str) -> bool {
    state
        .usage
        .reset_provider_ids()
        .iter()
        .any(|id| id == provider)
}

pub(super) fn selected_log_filters(
    value: Option<&str>,
    allowed: &[&str],
) -> Result<Vec<String>, &'static str> {
    match value {
        None | Some("") | Some("all") => Ok(Vec::new()),
        Some(value) => {
            let mut selected = Vec::new();
            for item in value.split(',') {
                if item == "all" || !allowed.contains(&item) {
                    return Err("invalid diagnostic log filter");
                }
                if !selected.iter().any(|existing| existing == item) {
                    selected.push(item.to_owned());
                }
            }
            Ok(selected)
        }
    }
}

pub(super) fn decode_log_cursor(
    value: Option<&str>,
) -> Result<Option<DiagnosticLogCursor>, &'static str> {
    let Some(value) = value else {
        return Ok(None);
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| "invalid diagnostic log cursor")?;
    let cursor: DiagnosticLogCursor =
        serde_json::from_slice(&bytes).map_err(|_| "invalid diagnostic log cursor")?;
    if cursor.occurred_at_ms <= 0 || cursor.id.is_empty() || cursor.id.len() > 512 {
        return Err("invalid diagnostic log cursor");
    }
    Ok(Some(cursor))
}

pub(super) fn encode_log_cursor(item: &crate::store::DiagnosticLogSummary) -> Option<String> {
    serde_json::to_vec(&DiagnosticLogCursor {
        occurred_at_ms: item.occurred_at_ms,
        id: item.id.clone(),
    })
    .ok()
    .map(|value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value))
}

pub(super) fn parse_diagnostic_log_filter(
    query: &DiagnosticLogsQuery,
) -> Result<crate::store::DiagnosticLogFilter, &'static str> {
    let now = now_ms();
    let (since_ms, until_ms) = match (query.from_ms, query.to_ms) {
        (Some(from_ms), Some(to_ms))
            if from_ms > 0
                && to_ms > from_ms
                && to_ms <= now.saturating_add(5 * 60_000)
                && to_ms.saturating_sub(from_ms) <= 365 * 86_400_000 =>
        {
            (from_ms, to_ms)
        }
        (Some(_), Some(_)) => return Err("invalid diagnostic log time range"),
        (None, None) => {
            let window_seconds = match query.window.as_deref().unwrap_or("7d") {
                "1h" => 3_600,
                "24h" => 86_400,
                "7d" => 7 * 86_400,
                "30d" => 30 * 86_400,
                _ => return Err("invalid diagnostic log window"),
            };
            (now.saturating_sub(i64::from(window_seconds) * 1_000), now)
        }
        _ => return Err("diagnostic log time range requires both boundaries"),
    };
    let session_ref = query
        .session
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if session_ref.as_ref().is_some_and(|value| {
        value.len() > 128
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')
            })
    }) {
        return Err("invalid diagnostic log session");
    }
    let cursor = decode_log_cursor(query.cursor.as_deref())?;
    Ok(crate::store::DiagnosticLogFilter {
        since_ms,
        until_ms,
        kinds: selected_log_filters(
            query.kind.as_deref(),
            &[
                "session_error",
                "provider_error",
                "cache_anomaly",
                "automation",
            ],
        )?,
        severities: selected_log_filters(
            query.severity.as_deref(),
            &["info", "warning", "error", "critical"],
        )?,
        states: selected_log_filters(
            query.state.as_deref(),
            &[
                "active",
                "recovered",
                "failed",
                "succeeded",
                "observed",
                "scheduled",
                "started",
                "retrying",
                "unknown",
                "cancelled",
            ],
        )?,
        agents: selected_log_filters(query.agent.as_deref(), &["codex", "claude"])?,
        session_ref,
        cursor_ms: cursor.as_ref().map(|value| value.occurred_at_ms),
        cursor_id: cursor.map(|value| value.id),
        limit: query.limit.unwrap_or(25).clamp(1, 100),
    })
}

pub(super) async fn api_diagnostic_logs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<DiagnosticLogsQuery>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "diagnostic logs unavailable",
        )
            .into_response();
    };
    let filter = match parse_diagnostic_log_filter(&query) {
        Ok(filter) => filter,
        Err(message) => return (StatusCode::BAD_REQUEST, message).into_response(),
    };
    match store.diagnostic_logs(&filter).await {
        Ok(mut items) => {
            let has_more = items.len() > usize::try_from(filter.limit).unwrap_or(100);
            items.truncate(usize::try_from(filter.limit).unwrap_or(100));
            let next_cursor = has_more && !items.is_empty();
            Json(DiagnosticLogPage {
                next_cursor: next_cursor
                    .then(|| items.last().and_then(encode_log_cursor))
                    .flatten(),
                items,
            })
            .into_response()
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) async fn api_diagnostic_log_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "diagnostic logs unavailable",
        )
            .into_response();
    };
    if id.is_empty() || id.len() > 512 {
        return (StatusCode::BAD_REQUEST, "invalid diagnostic log id").into_response();
    }
    match store.diagnostic_log_detail(&id).await {
        Ok(Some(detail)) => Json(detail).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) async fn prometheus_metrics(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !crate::product_auth::metrics_scrape_allowed(peer, &headers) {
        tracing::info!(reason = "forwarded", "metrics_rejected");
        return StatusCode::NOT_FOUND.into_response();
    }
    let sessions_live = state.hub.session_list().len();
    let (db_bytes, events_rows, sessions_deleted) = match &state.store {
        Some(store) => store.observed_storage_metrics().await.unwrap_or((0, 0, 0)),
        None => (0, i64::try_from(state.hub.event_total()).unwrap_or(0), 0),
    };
    let health = state.persistence_health.as_ref();
    let runtime = state.runtime_router.stats();
    let runtime_connected = state.runtime_router.has_connected_runtime();
    let daemon_memory = daemon_memory();
    let cgroup_memory = crate::memory_observability::own_cgroup_memory().unwrap_or_default();
    let hub_memory = state.hub.memory_stats();
    let body = format!(
        "# TYPE cowboy_up gauge\ncowboy_up {}\n# TYPE cowboy_database_bytes gauge\ncowboy_database_bytes {db_bytes}\n# TYPE cowboy_events_rows gauge\ncowboy_events_rows {events_rows}\n# TYPE cowboy_sessions gauge\ncowboy_sessions{{state=\"live\"}} {sessions_live}\ncowboy_sessions{{state=\"deleted\"}} {sessions_deleted}\n# TYPE cowboy_daemon_rss_bytes gauge\ncowboy_daemon_rss_bytes {}\n# TYPE cowboy_daemon_rss_peak_bytes gauge\ncowboy_daemon_rss_peak_bytes {}\n# TYPE cowboy_cgroup_memory_bytes gauge\ncowboy_cgroup_memory_bytes {}\n# TYPE cowboy_cgroup_memory_peak_bytes gauge\ncowboy_cgroup_memory_peak_bytes {}\n# TYPE cowboy_persistence_pending gauge\ncowboy_persistence_pending {}\n# TYPE cowboy_persistence_pending_bytes gauge\ncowboy_persistence_pending_bytes {}\n# TYPE cowboy_hub_hot_log_bytes gauge\ncowboy_hub_hot_log_bytes {}\n# TYPE cowboy_hub_broadcast_last_bytes gauge\ncowboy_hub_broadcast_last_bytes {}\n# TYPE cowboy_persistence_dropped_total counter\ncowboy_persistence_dropped_total {}\n# TYPE cowboy_persistence_failed_batches_total counter\ncowboy_persistence_failed_batches_total {}\n# TYPE cowboy_persistence_healthy gauge\ncowboy_persistence_healthy {}\n# TYPE cowboy_runtime_connected gauge\ncowboy_runtime_connected {}\n# TYPE cowboy_runtime_workers gauge\ncowboy_runtime_workers {}\n# TYPE cowboy_runtime_busy_workers gauge\ncowboy_runtime_busy_workers {}\n# TYPE cowboy_runtime_draining_workers gauge\ncowboy_runtime_draining_workers {}\n# TYPE cowboy_runtime_handoff_workers gauge\ncowboy_runtime_handoff_workers {}\n# TYPE cowboy_runtime_pending_commands gauge\ncowboy_runtime_pending_commands {}\n# TYPE cowboy_observability_pending gauge\ncowboy_observability_pending {}\n# TYPE cowboy_observability_accepted_batches_total counter\ncowboy_observability_accepted_batches_total {}\n# TYPE cowboy_observability_dropped_batches_total counter\ncowboy_observability_dropped_batches_total {}\n# TYPE cowboy_observability_failed_log_batches_total counter\ncowboy_observability_failed_log_batches_total {}\n# TYPE cowboy_observability_failed_metric_batches_total counter\ncowboy_observability_failed_metric_batches_total {}\n",
        u8::from(state.runtime_health.is_healthy(state.store.is_some()) && runtime_connected),
        daemon_memory.rss_bytes,
        daemon_memory.rss_peak_bytes,
        cgroup_memory.current_bytes,
        cgroup_memory.peak_bytes,
        health.map_or(0, |h| h.pending()),
        health.map_or(0, |h| h.pending_bytes()),
        hub_memory.hot_log_bytes,
        hub_memory.broadcast_last_bytes,
        health.map_or(0, |h| h.dropped()),
        health.map_or(0, |h| h.failed_batches()),
        u8::from(health.is_none_or(|h| h.is_healthy())),
        u8::from(runtime_connected),
        runtime.workers,
        runtime.busy_workers,
        runtime.draining_workers,
        runtime.handoff_workers,
        runtime.pending_commands,
        state.observability.health().pending(),
        state.observability.health().accepted_batches(),
        state.observability.health().dropped_batches(),
        state.observability.health().failed_log_batches(),
        state.observability.health().failed_metric_batches(),
    );
    let body = body
        + &state.observability.health().prometheus()
        + &crate::transcript_anchors::METRICS.prometheus()
        + &notifications::COUNTERS.prometheus();
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}
