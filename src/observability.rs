//! Bounded client observability ingestion and Victoria forwarding.
//!
//! Raw logs and metrics belong to Victoria. `PostgreSQL` stores only the Runtime
//! Incident Ledger, keeping high-volume evidence out of transcript history.

#![warn(clippy::pedantic)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};

use crate::store::{RuntimeIncidentWrite, Store};

const QUEUE_CAPACITY: usize = 256;
const MAX_ITEMS: usize = 200;
const MAX_MESSAGE_BYTES: usize = 4 * 1024;
const MAX_ATTRIBUTE_BYTES: usize = 16 * 1024;
const FORWARD_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(100),
    Duration::from_millis(250),
    Duration::from_millis(500),
];

#[derive(Debug, Clone, Deserialize)]
pub struct TelemetryBatch {
    pub batch_id: String,
    pub client: ClientIdentity,
    #[serde(default)]
    pub context: TelemetryContext,
    #[serde(default)]
    pub logs: Vec<ClientLog>,
    #[serde(default)]
    pub metrics: Vec<ClientMetric>,
    #[serde(default)]
    pub incidents: Vec<ClientIncident>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientIdentity {
    pub id: String,
    pub platform: String,
    pub app_version: String,
    #[serde(default)]
    pub surface: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[allow(clippy::struct_field_names)]
pub struct TelemetryContext {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientLog {
    pub occurred_at_ms: i64,
    pub level: String,
    pub event_name: String,
    pub message: String,
    #[serde(default)]
    pub attributes: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientMetric {
    pub occurred_at_ms: i64,
    pub name: String,
    pub value: f64,
    #[serde(default)]
    pub dimensions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientIncident {
    pub id: String,
    pub occurred_at_ms: i64,
    pub classification: String,
    pub severity: String,
    pub summary: String,
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub detail: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct SubmitReceipt {
    pub accepted: bool,
}

#[derive(Default)]
pub struct ObservabilityHealth {
    pending: AtomicUsize,
    accepted_batches: AtomicU64,
    dropped_batches: AtomicU64,
    failed_log_batches: AtomicU64,
    failed_metric_batches: AtomicU64,
}

impl ObservabilityHealth {
    pub fn pending(&self) -> usize {
        self.pending.load(Ordering::Relaxed)
    }

    pub fn accepted_batches(&self) -> u64 {
        self.accepted_batches.load(Ordering::Relaxed)
    }

    pub fn dropped_batches(&self) -> u64 {
        self.dropped_batches.load(Ordering::Relaxed)
    }

    pub fn failed_log_batches(&self) -> u64 {
        self.failed_log_batches.load(Ordering::Relaxed)
    }

    pub fn failed_metric_batches(&self) -> u64 {
        self.failed_metric_batches.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub struct Observability {
    tx: mpsc::Sender<TelemetryBatch>,
    health: Arc<ObservabilityHealth>,
}

impl Observability {
    pub fn start(store: Option<Store>, logs_url: String, metrics_url: String) -> Self {
        let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
        let health = Arc::new(ObservabilityHealth::default());
        tokio::spawn(run_writer(
            reqwest::Client::new(),
            store,
            logs_url,
            metrics_url,
            rx,
            Arc::clone(&health),
        ));
        Self { tx, health }
    }

    pub fn submit(&self, batch: TelemetryBatch) -> Result<(), &'static str> {
        validate_batch(&batch)?;
        self.health.pending.fetch_add(1, Ordering::Relaxed);
        if self.tx.try_send(batch).is_ok() {
            self.health.accepted_batches.fetch_add(1, Ordering::Relaxed);
            Ok(())
        } else {
            self.health.pending.fetch_sub(1, Ordering::Relaxed);
            self.health.dropped_batches.fetch_add(1, Ordering::Relaxed);
            Err("observability queue full")
        }
    }

    pub fn health(&self) -> &ObservabilityHealth {
        &self.health
    }
}

fn validate_batch(batch: &TelemetryBatch) -> Result<(), &'static str> {
    if !valid_token(&batch.batch_id, 128)
        || !valid_token(&batch.client.id, 128)
        || !valid_token(&batch.client.platform, 64)
        || batch.client.app_version.len() > 128
    {
        return Err("invalid batch identity");
    }
    let items = batch
        .logs
        .len()
        .saturating_add(batch.metrics.len())
        .saturating_add(batch.incidents.len());
    if items == 0 || items > MAX_ITEMS {
        return Err("batch must contain 1-200 items");
    }
    if batch.logs.iter().any(|entry| {
        !matches!(entry.level.as_str(), "debug" | "info" | "warn" | "error")
            || !valid_name(&entry.event_name)
            || entry.message.len() > MAX_MESSAGE_BYTES
            || serde_json::to_vec(&entry.attributes)
                .is_ok_and(|value| value.len() > MAX_ATTRIBUTE_BYTES)
    }) {
        return Err("invalid log entry");
    }
    if batch.metrics.iter().any(|metric| {
        !metric.value.is_finite()
            || !valid_name(&metric.name)
            || metric.dimensions.len() > 8
            || metric
                .dimensions
                .iter()
                .any(|(key, value)| !valid_name(key) || value.len() > 64)
    }) {
        return Err("invalid metric entry");
    }
    if batch.incidents.iter().any(|incident| {
        !valid_token(&incident.id, 128)
            || !valid_name(&incident.classification)
            || !matches!(incident.severity.as_str(), "warning" | "error" | "critical")
            || incident.summary.len() > MAX_MESSAGE_BYTES
            || serde_json::to_vec(&incident.detail)
                .is_ok_and(|value| value.len() > MAX_ATTRIBUTE_BYTES)
    }) {
        return Err("invalid incident entry");
    }
    Ok(())
}

fn valid_token(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
}

fn valid_name(value: &str) -> bool {
    valid_token(value, 96)
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && !sensitive_key(value)
}

fn sensitive_key(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    [
        "token",
        "secret",
        "password",
        "authorization",
        "cookie",
        "clipboard",
        "prompt",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn sanitize_attributes(value: &serde_json::Value) -> serde_json::Value {
    let Some(input) = value.as_object() else {
        return serde_json::json!({});
    };
    serde_json::Value::Object(
        input
            .iter()
            .filter(|(key, value)| {
                valid_name(key)
                    && matches!(
                        value,
                        serde_json::Value::Null
                            | serde_json::Value::Bool(_)
                            | serde_json::Value::Number(_)
                            | serde_json::Value::String(_)
                    )
            })
            .take(32)
            .map(|(key, value)| {
                let value = match value {
                    serde_json::Value::String(text) if text.len() > 512 => {
                        serde_json::Value::String(text.chars().take(512).collect())
                    }
                    other => other.clone(),
                };
                (key.clone(), value)
            })
            .collect(),
    )
}

async fn run_writer(
    client: reqwest::Client,
    store: Option<Store>,
    logs_url: String,
    metrics_url: String,
    mut rx: mpsc::Receiver<TelemetryBatch>,
    health: Arc<ObservabilityHealth>,
) {
    while let Some(batch) = rx.recv().await {
        let logs = logs_payload(&batch);
        if !logs.is_empty()
            && let Err(error) =
                post_with_retry(|| post_logs(&client, &logs_url, logs.clone())).await
        {
            health.failed_log_batches.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(%error, batch_id = %batch.batch_id, "forwarding client logs failed");
        }
        let metrics = metrics_payload(&batch);
        if !metrics.is_empty()
            && let Err(error) =
                post_with_retry(|| post_metrics(&client, &metrics_url, metrics.clone())).await
        {
            health.failed_metric_batches.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(%error, batch_id = %batch.batch_id, "forwarding client metrics failed");
        }
        if let Some(store) = store.as_ref() {
            for incident in &batch.incidents {
                if let Err(error) = persist_incident(store, &batch, incident).await {
                    tracing::error!(%error, incident_id = %incident.id, "persisting client runtime incident failed");
                }
            }
        }
        health.pending.fetch_sub(1, Ordering::Relaxed);
    }
}

fn logs_payload(batch: &TelemetryBatch) -> String {
    let mut output = String::new();
    for (index, entry) in batch.logs.iter().enumerate() {
        let timestamp = chrono::DateTime::from_timestamp_millis(entry.occurred_at_ms)
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339();
        let row = serde_json::json!({
            "timestamp": timestamp,
            "message": entry.message,
            "component": "cowboy-client",
            "level": entry.level,
            "event_name": entry.event_name,
            "event_id": format!("{}:log:{index}", batch.batch_id),
            "batch_id": batch.batch_id,
            "client_id": batch.client.id,
            "platform": batch.client.platform,
            "surface": batch.client.surface,
            "build": batch.client.app_version,
            "session_id": batch.context.session_id,
            "machine_id": batch.context.machine_id,
            "trace_id": batch.context.trace_id,
            "attributes": sanitize_attributes(&entry.attributes),
        });
        if let Ok(line) = serde_json::to_string(&row) {
            output.push_str(&line);
            output.push('\n');
        }
    }
    for (index, incident) in batch.incidents.iter().enumerate() {
        let timestamp = chrono::DateTime::from_timestamp_millis(incident.occurred_at_ms)
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339();
        let row = serde_json::json!({
            "timestamp": timestamp,
            "message": incident.summary,
            "component": "cowboy-client",
            "level": incident.severity,
            "event_name": "runtime_incident",
            "event_id": format!("{}:incident:{index}", batch.batch_id),
            "incident_id": incident.id,
            "classification": incident.classification,
            "batch_id": batch.batch_id,
            "client_id": batch.client.id,
            "platform": batch.client.platform,
            "surface": batch.client.surface,
            "build": batch.client.app_version,
            "session_id": batch.context.session_id,
            "machine_id": batch.context.machine_id,
            "trace_id": batch.context.trace_id,
            "attributes": sanitize_attributes(&incident.detail),
        });
        if let Ok(line) = serde_json::to_string(&row) {
            output.push_str(&line);
            output.push('\n');
        }
    }
    output
}

async fn post_with_retry<F, Fut>(mut operation: F) -> anyhow::Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let mut last_error = None;
    for (attempt, delay) in FORWARD_RETRY_DELAYS.iter().enumerate() {
        match operation().await {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 < FORWARD_RETRY_DELAYS.len() {
            sleep(*delay).await;
        }
    }
    Err(last_error.expect("retry loop always runs at least once"))
}

fn escape_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn metrics_payload(batch: &TelemetryBatch) -> String {
    let mut output = String::new();
    for metric in &batch.metrics {
        let mut labels = vec![
            ("platform", batch.client.platform.as_str()),
            ("build", batch.client.app_version.as_str()),
        ];
        if let Some(surface) = batch.client.surface.as_deref() {
            labels.push(("surface", surface));
        }
        let labels = labels
            .into_iter()
            .chain(
                metric
                    .dimensions
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            )
            .map(|(key, value)| format!("{key}=\"{}\"", escape_label(value)))
            .collect::<Vec<_>>()
            .join(",");
        let _ = writeln!(
            output,
            "cowboy_client_{}{{{labels}}} {} {}",
            metric.name, metric.value, metric.occurred_at_ms
        );
    }
    output
}

async fn post_logs(client: &reqwest::Client, base: &str, body: String) -> anyhow::Result<()> {
    client
        .post(format!(
            "{}/insert/jsonline?_stream_fields=component,platform&_time_field=timestamp&_msg_field=message",
            base.trim_end_matches('/')
        ))
        .header("content-type", "application/stream+json")
        .body(body)
        .send()
        .await
        .context("sending client logs to VictoriaLogs")?
        .error_for_status()
        .context("VictoriaLogs rejected client logs")?;
    Ok(())
}

async fn post_metrics(client: &reqwest::Client, base: &str, body: String) -> anyhow::Result<()> {
    client
        .post(format!(
            "{}/api/v1/import/prometheus",
            base.trim_end_matches('/')
        ))
        .header("content-type", "text/plain")
        .body(body)
        .send()
        .await
        .context("sending client metrics to VictoriaMetrics")?
        .error_for_status()
        .context("VictoriaMetrics rejected client metrics")?;
    Ok(())
}

async fn persist_incident(
    store: &Store,
    batch: &TelemetryBatch,
    incident: &ClientIncident,
) -> anyhow::Result<()> {
    let fingerprint = incident.fingerprint.clone().unwrap_or_else(|| {
        format!(
            "{:x}",
            Sha256::digest(format!("{}:{}", incident.classification, incident.summary).as_bytes())
        )
    });
    let detail = sanitize_attributes(&incident.detail);
    store
        .upsert_runtime_incident(&RuntimeIncidentWrite {
            id: incident.id.clone(),
            occurred_at_ms: incident.occurred_at_ms,
            source: "client".to_owned(),
            classification: incident.classification.clone(),
            severity: incident.severity.clone(),
            state: "active".to_owned(),
            summary: incident.summary.clone(),
            fingerprint,
            session_id: batch.context.session_id.clone(),
            client_id: Some(batch.client.id.clone()),
            machine_id: batch.context.machine_id.clone(),
            trace_id: batch.context.trace_id.clone(),
            build: Some(batch.client.app_version.clone()),
            evidence_start_ms: incident.occurred_at_ms.saturating_sub(30_000),
            evidence_end_ms: incident.occurred_at_ms.saturating_add(30_000),
            detail,
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch() -> TelemetryBatch {
        TelemetryBatch {
            batch_id: "batch-1".to_owned(),
            client: ClientIdentity {
                id: "client-1".to_owned(),
                platform: "ios-pwa".to_owned(),
                app_version: "cowboy-v1".to_owned(),
                surface: Some("mobile".to_owned()),
            },
            context: TelemetryContext::default(),
            logs: vec![ClientLog {
                occurred_at_ms: 1,
                level: "error".to_owned(),
                event_name: "render_error".to_owned(),
                message: "render failed".to_owned(),
                attributes: serde_json::json!({"view": "agent", "authorization": "secret"}),
            }],
            metrics: Vec::new(),
            incidents: Vec::new(),
        }
    }

    #[test]
    fn batch_validation_rejects_sensitive_dimensions() {
        let mut value = batch();
        value.metrics.push(ClientMetric {
            occurred_at_ms: 1,
            name: "render_ms".to_owned(),
            value: 2.0,
            dimensions: BTreeMap::from([("token".to_owned(), "oops".to_owned())]),
        });
        assert_eq!(validate_batch(&value), Err("invalid metric entry"));
    }

    #[test]
    fn log_payload_strips_sensitive_attributes() {
        let payload = logs_payload(&batch());
        assert!(payload.contains("render failed"));
        assert!(!payload.contains("authorization"));
        assert!(!payload.contains("secret"));
    }

    #[test]
    fn metric_payload_has_bounded_low_cardinality_labels() {
        let mut value = batch();
        value.metrics.push(ClientMetric {
            occurred_at_ms: 1_785_000_000_000,
            name: "websocket_connect_duration_ms".to_owned(),
            value: 12.5,
            dimensions: BTreeMap::from([("transport".to_owned(), "websocket".to_owned())]),
        });
        let payload = metrics_payload(&value);
        assert!(payload.starts_with("cowboy_client_websocket_connect_duration_ms{"));
        assert!(payload.contains("transport=\"websocket\""));
        assert!(!payload.contains("client-1"));
    }
}
