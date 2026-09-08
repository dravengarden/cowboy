//! Bounded, backend-neutral client telemetry with private local evidence.
//!
//! Remote exporters are optional. The durable Runtime Incident Ledger remains
//! separate from diagnostic files and cannot wait behind remote delivery.

#![warn(clippy::pedantic)]

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use futures::future::BoxFuture;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::sync::{mpsc, watch};
use tokio::time::Duration;

use crate::store::{RuntimeIncidentWrite, Store};
use crate::telemetry_file::TelemetryFile;

const QUEUE_CAPACITY: usize = 64;
const MAX_BATCH_BYTES: usize = 256 * 1024;
const MAX_PENDING_BYTES: usize = 8 * 1024 * 1024;
const DEDUP_CAPACITY: usize = 2048;
const MAX_ITEMS: usize = 200;
const MAX_MESSAGE_BYTES: usize = 4 * 1024;
const MAX_ATTRIBUTE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClientIdentity {
    pub id: String,
    pub platform: String,
    pub app_version: String,
    #[serde(default)]
    pub surface: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[allow(clippy::struct_field_names)]
pub struct TelemetryContext {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClientLog {
    pub occurred_at_ms: i64,
    pub level: String,
    pub event_name: String,
    pub message: String,
    #[serde(default)]
    pub attributes: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClientMetric {
    pub occurred_at_ms: i64,
    pub name: String,
    pub value: f64,
    #[serde(default)]
    pub dimensions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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
    pending_bytes: AtomicUsize,
    accepted_batches: AtomicU64,
    duplicate_batches: AtomicU64,
    dropped_batches: AtomicU64,
    failed_file_batches: AtomicU64,
    failed_incident_batches: AtomicU64,
    dropped_export_batches: AtomicU64,
    failed_log_batches: AtomicU64,
    failed_metric_batches: AtomicU64,
    failed_trace_batches: AtomicU64,
    rejected_export_items: AtomicU64,
}

impl ObservabilityHealth {
    pub(crate) fn prometheus(&self) -> String {
        let mut output = format!(
            "# TYPE cowboy_observability_pending_bytes gauge\ncowboy_observability_pending_bytes {}\n",
            self.pending_bytes()
        );
        for (name, value) in [
            ("duplicate_batches", self.duplicate_batches()),
            ("failed_file_batches", self.failed_file_batches()),
            ("failed_incident_batches", self.failed_incident_batches()),
            ("dropped_export_batches", self.dropped_export_batches()),
            (
                "failed_trace_batches",
                self.failed_trace_batches.load(Ordering::Relaxed),
            ),
            (
                "rejected_export_items",
                self.rejected_export_items.load(Ordering::Relaxed),
            ),
        ] {
            let _ = writeln!(
                output,
                "# TYPE cowboy_observability_{name}_total counter\ncowboy_observability_{name}_total {value}"
            );
        }
        output
    }
    pub fn pending_bytes(&self) -> usize {
        self.pending_bytes.load(Ordering::Relaxed)
    }
    pub fn duplicate_batches(&self) -> u64 {
        self.duplicate_batches.load(Ordering::Relaxed)
    }
    pub fn failed_file_batches(&self) -> u64 {
        self.failed_file_batches.load(Ordering::Relaxed)
    }
    pub fn failed_incident_batches(&self) -> u64 {
        self.failed_incident_batches.load(Ordering::Relaxed)
    }
    pub fn dropped_export_batches(&self) -> u64 {
        self.dropped_export_batches.load(Ordering::Relaxed)
    }
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

    pub fn failed_trace_batches(&self) -> u64 {
        self.failed_trace_batches.load(Ordering::Relaxed)
    }
    pub fn rejected_export_items(&self) -> u64 {
        self.rejected_export_items.load(Ordering::Relaxed)
    }
}

pub(crate) struct ExportBatch {
    pub logs: String,
    pub metrics: String,
    pub otlp: Option<crate::otlp::Export>,
}

#[derive(Default)]
pub(crate) struct ExportReceipt {
    pub logs_delivered: bool,
    pub metrics_delivered: bool,
    pub traces_delivered: bool,
    pub rejected_items: u64,
}

pub(crate) type TelemetryExporter =
    Arc<dyn Fn(ExportBatch) -> BoxFuture<'static, ExportReceipt> + Send + Sync>;

struct PendingBatch {
    batch: PendingData,
    bytes: usize,
}

enum PendingData {
    Legacy(Box<TelemetryBatch>),
    Otlp {
        records: String,
        export: crate::otlp::Export,
    },
}

#[derive(Default)]
struct Admission {
    seen: BTreeMap<String, String>,
    order: VecDeque<String>,
    metrics: crate::otlp::Aggregator,
}

#[derive(Clone)]
pub struct Observability {
    tx: mpsc::Sender<PendingBatch>,
    health: Arc<ObservabilityHealth>,
    admission: Arc<Mutex<Admission>>,
    runtime: Arc<Mutex<crate::runtime_telemetry::RuntimeTelemetry>>,
    shutdown: watch::Sender<bool>,
    jobs: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl Observability {
    pub(crate) fn start(
        store: Option<Store>,
        file: TelemetryFile,
        exporter: Option<TelemetryExporter>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let health = Arc::new(ObservabilityHealth::default());
        let mut jobs = Vec::new();
        let export_tx = exporter.map(|exporter| {
            let (tx, rx) = mpsc::channel(16);
            jobs.push(tokio::spawn(run_exporter(
                exporter,
                rx,
                Arc::clone(&health),
            )));
            tx
        });
        let incident_tx = store.map(|store| {
            let (tx, rx) = mpsc::channel(16);
            jobs.push(tokio::spawn(run_incident_writer(
                store,
                rx,
                Arc::clone(&health),
            )));
            tx
        });
        jobs.push(tokio::spawn(run_writer(
            file,
            rx,
            export_tx,
            incident_tx,
            Arc::clone(&health),
            shutdown_rx,
        )));
        Self {
            tx,
            health,
            admission: Arc::default(),
            runtime: Arc::default(),
            shutdown,
            jobs: Arc::new(Mutex::new(jobs)),
        }
    }

    pub fn submit(&self, scope: &str, mut batch: TelemetryBatch) -> Result<(), &'static str> {
        validate_batch(&batch)?;
        let bytes = serde_json::to_vec(&batch).map_err(|_| "invalid telemetry batch")?;
        if bytes.len() > MAX_BATCH_BYTES {
            return Err("telemetry batch too large");
        }
        let key = scoped_identity(scope, &batch.client.id, &batch.batch_id);
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let mut admission = self.admission.lock();
        if let Some(previous) = admission.seen.get(&key) {
            if previous != &digest {
                return Err("batch identity reused with different content");
            }
            self.health
                .duplicate_batches
                .fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        if *self.shutdown.borrow() {
            return Err("observability unavailable");
        }
        sanitize_batch(scope, &mut batch);
        if self
            .health
            .pending_bytes
            .load(Ordering::Relaxed)
            .saturating_add(bytes.len())
            > MAX_PENDING_BYTES
        {
            self.health.dropped_batches.fetch_add(1, Ordering::Relaxed);
            return Err("observability queue full");
        }
        self.health.pending.fetch_add(1, Ordering::Relaxed);
        self.health
            .pending_bytes
            .fetch_add(bytes.len(), Ordering::Relaxed);
        if self
            .tx
            .try_send(PendingBatch {
                batch: PendingData::Legacy(Box::new(batch)),
                bytes: bytes.len(),
            })
            .is_ok()
        {
            if admission.order.len() == DEDUP_CAPACITY
                && let Some(oldest) = admission.order.pop_front()
            {
                admission.seen.remove(&oldest);
            }
            admission.order.push_back(key.clone());
            admission.seen.insert(key, digest);
            self.health.accepted_batches.fetch_add(1, Ordering::Relaxed);
            Ok(())
        } else {
            self.health.pending.fetch_sub(1, Ordering::Relaxed);
            self.health
                .pending_bytes
                .fetch_sub(bytes.len(), Ordering::Relaxed);
            self.health.dropped_batches.fetch_add(1, Ordering::Relaxed);
            Err("observability queue full")
        }
    }

    /// Authentication is supplied by the route, never by an OTLP resource.
    /// Reserve before aggregation: a rejected/retried request cannot increment
    /// a counter until its complete sanitized batch has entered the writer.
    pub(crate) fn submit_otlp(
        &self,
        principal: &str,
        signal: crate::otlp::Signal,
        batch_id: &str,
        body: &[u8],
    ) -> Result<(), &'static str> {
        if !valid_token(batch_id, 128) {
            return Err("invalid OTLP batch identity");
        }
        let mut request =
            crate::otlp::Request::decode(signal, body).map_err(|_| "invalid OTLP request")?;
        request
            .sanitize(principal)
            .map_err(|_| "invalid OTLP data")?;
        self.admit_otlp(principal, signal, batch_id, body, request)
    }

    pub(crate) fn begin_command_trace(text: &str) -> Option<crate::runtime_trace::SpanTimer> {
        // Deserialize only propagation metadata, never copy a prompt/content
        // object into the telemetry representation.
        #[derive(Deserialize)]
        struct Carrier {
            traceparent: Option<String>,
        }
        if !text.contains("\"traceparent\"") {
            return None;
        }
        let parent = serde_json::from_str::<Carrier>(text)
            .ok()
            .and_then(|v| v.traceparent)?;
        let context = crate::runtime_trace::TraceContext::from_browser(&parent)?;
        crate::runtime_trace::SpanTimer::start(
            &context,
            crate::runtime_trace::Stage::ControllerDispatch,
        )
    }

    pub(crate) fn finish_command_trace(
        &self,
        principal: &str,
        span: crate::runtime_trace::SpanTimer,
        accepted: bool,
    ) {
        if let Some(record) = span.finish(if accepted {
            crate::runtime_trace::Outcome::Ok
        } else {
            crate::runtime_trace::Outcome::Error
        }) {
            self.record_span(&crate::runtime_telemetry::owner_hash(principal), &record);
        }
    }

    pub(crate) fn bind_runtime_trace(
        &self,
        principal: &str,
        machine: &str,
        session: &str,
        cmid: &str,
        span: &crate::runtime_trace::SpanTimer,
    ) {
        self.runtime
            .lock()
            .bind(principal, machine, session, cmid, &span.record.context);
    }

    pub(crate) fn dispatch_trace(
        &self,
        session: &str,
        cmid: Option<&str>,
    ) -> Option<crate::runtime_trace::TraceCarrier> {
        let (owner, queue, carrier) = self.runtime.lock().dispatch(session, cmid?)?;
        if let Some(record) = queue {
            self.record_span(&owner, &record);
        }
        Some(carrier)
    }

    pub(crate) fn finish_delivery_trace(
        &self,
        carrier: &crate::runtime_trace::TraceCarrier,
        accepted: bool,
    ) {
        let result = self.runtime.lock().delivery(
            &carrier.context,
            if accepted {
                crate::runtime_trace::Outcome::Ok
            } else {
                crate::runtime_trace::Outcome::Error
            },
        );
        if let Some((owner, record)) = result {
            self.record_span(&owner, &record);
        }
    }

    pub(crate) fn record_runtime_spans(
        &self,
        machine: &str,
        session: &str,
        records: &[crate::runtime_trace::SpanRecord],
    ) {
        for record in records
            .iter()
            .take(crate::runtime_trace::MAX_SPANS_PER_FRAME)
        {
            let owner = self.runtime.lock().accept(machine, session, record);
            if let Some(owner) = owner {
                self.record_span(&owner, record);
            }
        }
    }

    fn record_span(&self, owner: &str, record: &crate::runtime_trace::SpanRecord) {
        let request = crate::otlp::runtime_span(owner, record);
        let export = request.export();
        let _ = self.admit_otlp(
            owner,
            crate::otlp::Signal::Traces,
            record.context.span_id(),
            export.protobuf.as_bytes(),
            request,
        );
    }

    fn admit_otlp(
        &self,
        principal: &str,
        signal: crate::otlp::Signal,
        batch_id: &str,
        body: &[u8],
        mut request: crate::otlp::Request,
    ) -> Result<(), &'static str> {
        let key = scoped_identity(principal, &format!("otlp:{signal:?}"), batch_id);
        let digest = format!("{:x}", Sha256::digest(body));
        let mut admission = self.admission.lock();
        if let Some(previous) = admission.seen.get(&key) {
            if previous != &digest {
                return Err("batch identity reused with different content");
            }
            self.health
                .duplicate_batches
                .fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        if *self.shutdown.borrow() {
            return Err("observability unavailable");
        }
        let permit = self.tx.try_reserve().map_err(|_| {
            self.health.dropped_batches.fetch_add(1, Ordering::Relaxed);
            "observability queue full"
        })?;
        let candidate = if signal == crate::otlp::Signal::Metrics {
            let mut candidate = admission.metrics.clone();
            candidate
                .aggregate(&mut request)
                .map_err(|_| "invalid OTLP metric streams")?;
            Some(candidate)
        } else {
            None
        };
        let records = request.records();
        let export = request.export();
        // Protobuf -> JSON can expand; account for actual retained bytes and
        // enforce an individual-record ceiling below the smallest file segment.
        let bytes = records.len().saturating_add(export.protobuf.len());
        if bytes > 1024 * 1024 || records.lines().any(|line| line.len() > 32 * 1024) {
            return Err("OTLP records too large");
        }
        if self.health.pending_bytes().saturating_add(bytes) > MAX_PENDING_BYTES {
            self.health.dropped_batches.fetch_add(1, Ordering::Relaxed);
            return Err("observability queue full");
        }
        if let Some(candidate) = candidate {
            admission.metrics = candidate;
        }
        if admission.order.len() == DEDUP_CAPACITY
            && let Some(oldest) = admission.order.pop_front()
        {
            admission.seen.remove(&oldest);
        }
        admission.order.push_back(key.clone());
        admission.seen.insert(key, digest);
        self.health.pending.fetch_add(1, Ordering::Relaxed);
        self.health
            .pending_bytes
            .fetch_add(bytes, Ordering::Relaxed);
        self.health.accepted_batches.fetch_add(1, Ordering::Relaxed);
        permit.send(PendingBatch {
            batch: PendingData::Otlp { records, export },
            bytes,
        });
        Ok(())
    }

    pub fn health(&self) -> &ObservabilityHealth {
        &self.health
    }

    pub(crate) async fn drain(&self) {
        let _ = self.shutdown.send(true);
        let mut jobs = std::mem::take(&mut *self.jobs.lock());
        if tokio::time::timeout(
            Duration::from_secs(10),
            futures::future::join_all(jobs.iter_mut()),
        )
        .await
        .is_err()
        {
            for job in jobs {
                job.abort();
            }
            tracing::warn!(
                pending = self.health.pending(),
                "telemetry shutdown deadline exceeded"
            );
        }
    }
}

fn scoped_identity(scope: &str, client: &str, id: &str) -> String {
    let mut hash = Sha256::new();
    for value in [scope, client, id] {
        hash.update(value.len().to_be_bytes());
        hash.update(value.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

fn validate_batch(batch: &TelemetryBatch) -> Result<(), &'static str> {
    if !valid_token(&batch.batch_id, 128)
        || !valid_token(&batch.client.id, 128)
        || !valid_token(&batch.client.platform, 64)
        || batch.client.app_version.len() > 128
        || batch
            .client
            .surface
            .as_deref()
            .is_some_and(|value| !valid_token(value, 32))
        || [
            &batch.context.session_id,
            &batch.context.machine_id,
            &batch.context.trace_id,
        ]
        .into_iter()
        .flatten()
        .any(|value| !valid_token(value, 128))
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
        entry.occurred_at_ms <= 0
            || !matches!(entry.level.as_str(), "debug" | "info" | "warn" | "error")
            || !valid_name(&entry.event_name)
            || entry.message.len() > MAX_MESSAGE_BYTES
            || serde_json::to_vec(&entry.attributes)
                .is_ok_and(|value| value.len() > MAX_ATTRIBUTE_BYTES)
    }) {
        return Err("invalid log entry");
    }
    if batch.metrics.iter().any(|metric| {
        metric.occurred_at_ms <= 0
            || !metric.value.is_finite()
            || !valid_metric_name(&metric.name)
            || metric.dimensions.len() > 8
            || metric.dimensions.iter().any(|(key, value)| {
                !valid_metric_name(key)
                    || sensitive_key(key)
                    || matches!(
                        key.as_str(),
                        "platform"
                            | "surface"
                            | "build"
                            | "__name__"
                            | "client_id"
                            | "session_id"
                            | "trace_id"
                            | "machine_id"
                            | "url"
                            | "path"
                    )
                    || value.len() > 64
            })
    }) {
        return Err("invalid metric entry");
    }
    if batch.incidents.iter().any(|incident| {
        incident.occurred_at_ms <= 0
            || !valid_token(&incident.id, 128)
            || !valid_name(&incident.classification)
            || !matches!(incident.severity.as_str(), "warning" | "error" | "critical")
            || incident.summary.len() > MAX_MESSAGE_BYTES
            || incident
                .fingerprint
                .as_deref()
                .is_some_and(|value| !valid_token(value, 128))
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
}

fn valid_metric_name(value: &str) -> bool {
    valid_name(value)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn truncate_utf8(value: &str, maximum: usize) -> &str {
    let mut end = value.len().min(maximum);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

pub(crate) fn redact_message(value: &str) -> String {
    static COOKIE: OnceLock<regex::Regex> = OnceLock::new();
    static CREDENTIAL: OnceLock<regex::Regex> = OnceLock::new();
    static BEARER: OnceLock<regex::Regex> = OnceLock::new();
    static URL: OnceLock<regex::Regex> = OnceLock::new();
    let credential = CREDENTIAL.get_or_init(|| regex::Regex::new(r#"(?i)([\"']?(?:authorization|proxy-authorization|cookie|set-cookie|password|secret|api[_-]?key|access[_-]?token|refresh[_-]?token|id[_-]?token|token|client[_-]?secret)[\"']?\s*[:=]\s*)(?:[\"'][^\"'\r\n]*[\"']|(?:Bearer|Basic)\s+[^\s,;]+|[^\s,;]+)"#).expect("literal credential regex"));
    let bearer = BEARER.get_or_init(|| {
        regex::Regex::new(r"(?i)\b(Bearer|Basic)\s+[^\s,;]+").expect("literal authorization regex")
    });
    let urls = URL
        .get_or_init(|| regex::Regex::new(r#"https?://[^\s\"'<>]+"#).expect("literal URL regex"));
    let cookie = COOKIE.get_or_init(|| {
        regex::Regex::new(r"(?im)\b(?:set-cookie|cookie)\s*[:=][^\r\n]*")
            .expect("literal cookie regex")
    });
    let redacted = cookie.replace_all(value, "cookie=[redacted]");
    let redacted = credential.replace_all(&redacted, "$1[redacted]");
    let redacted = bearer.replace_all(&redacted, "$1 [redacted]");
    let redacted = urls.replace_all(&redacted, |capture: &regex::Captures<'_>| {
        let Ok(mut url) = url::Url::parse(&capture[0]) else {
            return "[redacted-url]".to_owned();
        };
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        url.to_string()
    });
    truncate_utf8(&redacted, MAX_MESSAGE_BYTES).to_owned()
}

fn sanitize_batch(scope: &str, batch: &mut TelemetryBatch) {
    let platform = match batch.client.platform.as_str() {
        "ios" | "ios-pwa" => "ios",
        "macos" => "macos",
        "web" => "web",
        _ => "other",
    };
    platform.clone_into(&mut batch.client.platform);
    batch.client.surface = batch.client.surface.as_deref().map(|surface| {
        match surface {
            "mobile" => "mobile",
            "desktop" => "desktop",
            _ => "other",
        }
        .to_owned()
    });
    batch.client.app_version = redact_message(&batch.client.app_version);
    let received = chrono::Utc::now().timestamp_millis();
    let bounded_time = |time: i64| {
        time.clamp(
            received.saturating_sub(7 * 86_400_000),
            received.saturating_add(300_000),
        )
    };
    for entry in &mut batch.logs {
        entry.occurred_at_ms = bounded_time(entry.occurred_at_ms);
        entry.message = redact_message(&entry.message);
        entry.attributes = sanitize_attributes(&entry.attributes);
    }
    for metric in &mut batch.metrics {
        metric.occurred_at_ms = bounded_time(metric.occurred_at_ms);
        metric.dimensions = metric
            .dimensions
            .iter()
            .filter_map(|(key, value)| {
                metric_dimension(key, value).map(|(key, value)| (key.to_owned(), value.to_owned()))
            })
            .collect();
    }
    for incident in &mut batch.incidents {
        incident.id = scoped_identity(scope, &batch.client.id, &incident.id);
        incident.occurred_at_ms = bounded_time(incident.occurred_at_ms);
        incident.summary = redact_message(&incident.summary);
        incident.detail = sanitize_attributes(&incident.detail);
        // Grouping is derived from redacted content, not attacker-selected IDs.
        incident.fingerprint = None;
    }
    batch.batch_id = scoped_identity(scope, &batch.client.id, &batch.batch_id);
    batch.client.id = scoped_identity(scope, &batch.client.id, "client");
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

pub(crate) fn sanitize_attributes(value: &serde_json::Value) -> serde_json::Value {
    let Some(input) = value.as_object() else {
        return serde_json::json!({});
    };
    serde_json::Value::Object(
        input
            .iter()
            .filter(|(key, value)| {
                valid_name(key)
                    && !sensitive_key(key)
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
                    serde_json::Value::String(text) => serde_json::Value::String(
                        truncate_utf8(&redact_message(text), 512).to_owned(),
                    ),
                    other => other.clone(),
                };
                (key.clone(), value)
            })
            .collect(),
    )
}

async fn run_writer(
    file: TelemetryFile,
    mut rx: mpsc::Receiver<PendingBatch>,
    export_tx: Option<mpsc::Sender<ExportBatch>>,
    incident_tx: Option<mpsc::Sender<TelemetryBatch>>,
    health: Arc<ObservabilityHealth>,
    mut shutdown: watch::Receiver<bool>,
) {
    let file = Arc::new(Mutex::new(file));
    let mut closing = false;
    loop {
        let pending = tokio::select! {
            biased;
            _ = shutdown.changed(), if !closing => {
                closing = true;
                rx.close();
                continue;
            }
            pending = rx.recv() => pending,
        };
        let Some(PendingBatch { batch, bytes }) = pending else {
            break;
        };
        let (records, export) = match batch {
            PendingData::Otlp { records, export } => (
                records,
                ExportBatch {
                    logs: String::new(),
                    metrics: String::new(),
                    otlp: Some(export),
                },
            ),
            PendingData::Legacy(batch) => {
                let batch = *batch;
                if let Some(tx) = incident_tx.as_ref()
                    && !batch.incidents.is_empty()
                    && tx.try_send(batch.clone()).is_err()
                {
                    health
                        .failed_incident_batches
                        .fetch_add(1, Ordering::Relaxed);
                }
                let logs = logs_payload(&batch);
                let records = local_payload(&batch, &logs);
                // Incident-only submissions are the durable product ledger,
                // not a second diagnostic export. The client emits their OTel
                // log independently, including when traces are unsampled.
                let logs = if batch.logs.is_empty() && batch.metrics.is_empty() {
                    String::new()
                } else {
                    logs
                };
                (
                    records,
                    ExportBatch {
                        logs,
                        metrics: metrics_payload(&batch),
                        otlp: None,
                    },
                )
            }
        };
        let writer = Arc::clone(&file);
        let written = tokio::task::spawn_blocking(move || {
            writer
                .lock()
                .write(&records, chrono::Utc::now().timestamp_millis())
        })
        .await;
        if !matches!(written, Ok(Ok(()))) {
            health.failed_file_batches.fetch_add(1, Ordering::Relaxed);
            tracing::warn!("local telemetry write failed");
        }
        if let Some(tx) = export_tx.as_ref()
            && tx.try_send(export).is_err()
        {
            health
                .dropped_export_batches
                .fetch_add(1, Ordering::Relaxed);
        }
        health.pending.fetch_sub(1, Ordering::Relaxed);
        health.pending_bytes.fetch_sub(bytes, Ordering::Relaxed);
    }
}

async fn run_exporter(
    exporter: TelemetryExporter,
    mut rx: mpsc::Receiver<ExportBatch>,
    health: Arc<ObservabilityHealth>,
) {
    while let Some(batch) = rx.recv().await {
        if batch.logs.is_empty() && batch.metrics.is_empty() && batch.otlp.is_none() {
            continue;
        }
        let signal = batch.otlp.as_ref().map(|export| export.signal);
        let has_logs = !batch.logs.is_empty() || signal == Some(crate::otlp::Signal::Logs);
        let has_metrics = !batch.metrics.is_empty() || signal == Some(crate::otlp::Signal::Metrics);
        let has_traces = signal == Some(crate::otlp::Signal::Traces);
        let receipt = tokio::time::timeout(Duration::from_secs(15), exporter(batch))
            .await
            .unwrap_or_default();
        if has_logs && !receipt.logs_delivered {
            health.failed_log_batches.fetch_add(1, Ordering::Relaxed);
        }
        if has_metrics && !receipt.metrics_delivered {
            health.failed_metric_batches.fetch_add(1, Ordering::Relaxed);
        }
        if has_traces && !receipt.traces_delivered {
            health.failed_trace_batches.fetch_add(1, Ordering::Relaxed);
        }
        health
            .rejected_export_items
            .fetch_add(receipt.rejected_items, Ordering::Relaxed);
    }
}

async fn run_incident_writer(
    store: Store,
    mut rx: mpsc::Receiver<TelemetryBatch>,
    health: Arc<ObservabilityHealth>,
) {
    while let Some(batch) = rx.recv().await {
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            for incident in &batch.incidents {
                persist_incident(&store, &batch, incident).await?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            health
                .failed_incident_batches
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn local_payload(batch: &TelemetryBatch, logs: &str) -> String {
    let mut output = logs.to_owned();
    for (index, metric) in batch.metrics.iter().enumerate() {
        let row = serde_json::json!({
            "schema_version": 1, "kind": "metric", "occurred_at_ms": metric.occurred_at_ms,
            "event_id": format!("{}:metric:{index}", batch.batch_id), "batch_id": batch.batch_id,
            "client": batch.client, "context": batch.context, "name": metric.name,
            "value": metric.value, "dimensions": metric.dimensions,
        });
        let _ = writeln!(output, "{row}");
    }
    output
}

fn logs_payload(batch: &TelemetryBatch) -> String {
    let mut output = String::new();
    for (index, entry) in batch.logs.iter().enumerate() {
        let timestamp = chrono::DateTime::from_timestamp_millis(entry.occurred_at_ms)
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339();
        let row = serde_json::json!({
            "schema_version": 1, "kind": "log",
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
            "schema_version": 1, "kind": "incident",
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

fn escape_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn metrics_payload(batch: &TelemetryBatch) -> String {
    let mut output = String::new();
    for metric in &batch.metrics {
        let platform = match batch.client.platform.as_str() {
            "ios" | "ios-pwa" => "ios",
            "macos" => "macos",
            "web" => "web",
            _ => "other",
        };
        let mut labels = vec![("platform", platform)];
        if let Some(surface) = batch.client.surface.as_deref() {
            labels.push((
                "surface",
                match surface {
                    "mobile" => "mobile",
                    "desktop" => "desktop",
                    _ => "other",
                },
            ));
        }
        let labels = labels
            .into_iter()
            .chain(
                metric
                    .dimensions
                    .iter()
                    .filter_map(|(key, value)| metric_dimension(key, value)),
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

fn metric_dimension<'a>(key: &'a str, value: &'a str) -> Option<(&'a str, &'a str)> {
    // Finite instrument dimensions only. Arbitrary attributes remain in logs,
    // never remote time-series labels (including build/client/session IDs).
    let allowed = match key {
        "connection" => &["initial", "reconnect"][..],
        "transport" => &["websocket", "http"][..],
        "reason" => &[
            "initial",
            "online",
            "visibility",
            "retry",
            "heartbeat_timeout",
            "transport_error",
            "close",
            "manual",
        ][..],
        _ => return None,
    };
    Some((
        key,
        if allowed.contains(&value) {
            value
        } else {
            "other"
        },
    ))
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

    #[test]
    fn portable_metric_syntax_and_reserved_labels_are_enforced() {
        for name in ["bad.name", "bad-name", "bad:name"] {
            let mut value = batch();
            value.metrics.push(ClientMetric {
                occurred_at_ms: 1,
                name: name.into(),
                value: 1.0,
                dimensions: BTreeMap::new(),
            });
            assert!(validate_batch(&value).is_err());
        }
        for key in ["platform", "build", "session_id", "bad.name", "url"] {
            let mut value = batch();
            value.metrics.push(ClientMetric {
                occurred_at_ms: 1,
                name: "input_tokens".into(),
                value: 1.0,
                dimensions: BTreeMap::from([(key.into(), "private".into())]),
            });
            assert!(validate_batch(&value).is_err());
        }
        let mut value = batch();
        value.metrics.push(ClientMetric {
            occurred_at_ms: 1,
            name: "input_tokens".into(),
            value: 1.0,
            dimensions: BTreeMap::from([("reason".into(), "random-identity".into())]),
        });
        validate_batch(&value).unwrap();
        let payload = metrics_payload(&value);
        assert!(payload.contains("reason=\"other\""));
        assert!(!payload.contains("random-identity"));
        assert!(!payload.contains("build="));
    }

    #[test]
    fn server_redacts_credentials_and_namespaces_untrusted_identities() {
        let source = "Authorization: Bearer super-secret, password=\"two words\" https://user:pass@example.test/x?token=private#hash";
        let redacted = redact_message(source);
        for secret in ["super-secret", "two words", "user:", "private", "#hash"] {
            assert!(!redacted.contains(secret), "{redacted}");
        }
        assert!(redacted.contains("https://example.test/x"));
        assert!(redact_message(&"中".repeat(4096)).len() <= 4096);
        assert_ne!(
            scoped_identity("a", "b:c", "d"),
            scoped_identity("a:b", "c", "d")
        );
        let mut a = batch();
        a.incidents.push(ClientIncident {
            id: "forged".into(),
            occurred_at_ms: i64::MAX,
            classification: "client_window_error".into(),
            severity: "critical".into(),
            summary: source.into(),
            fingerprint: Some("forged".into()),
            detail: serde_json::json!({"url": "https://example.test/path?secret=value"}),
        });
        let mut b = a.clone();
        sanitize_batch("user-a", &mut a);
        sanitize_batch("user-b", &mut b);
        assert_ne!(a.batch_id, b.batch_id);
        assert_ne!(a.client.id, b.client.id);
        assert_ne!(a.incidents[0].id, b.incidents[0].id);
        assert!(a.incidents[0].fingerprint.is_none());
        assert!(a.incidents[0].occurred_at_ms <= chrono::Utc::now().timestamp_millis() + 300_000);
        assert!(!local_payload(&a, &logs_payload(&a)).contains("secret=value"));
    }

    fn local_file() -> (std::path::PathBuf, TelemetryFile) {
        let path = std::env::temp_dir().join(format!(
            "cowboy-observability-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let file = TelemetryFile::open(
            path.clone(),
            128 * 1024,
            3,
            chrono::Utc::now().timestamp_millis(),
        )
        .unwrap();
        (path, file)
    }

    #[tokio::test]
    async fn retries_are_idempotent_scoped_and_shutdown_drains_local_evidence() {
        let (path, file) = local_file();
        let writer = Observability::start(None, file, None);
        writer.submit("owner", batch()).unwrap();
        writer.submit("owner", batch()).unwrap();
        writer.submit("other-owner", batch()).unwrap();
        let mut changed = batch();
        changed.logs[0].message = "changed".into();
        assert_eq!(
            writer.submit("owner", changed),
            Err("batch identity reused with different content")
        );
        writer.drain().await;
        assert_eq!(writer.health().accepted_batches(), 2);
        assert_eq!(writer.health().duplicate_batches(), 1);
        assert_eq!(writer.health().pending_bytes(), 0);
        assert_eq!(writer.health().pending(), 0);
        let data = std::fs::read_to_string(path.join("telemetry.jsonl")).unwrap();
        assert_eq!(data.lines().count(), 2);
        assert!(!data.contains("authorization"));
        assert_eq!(
            writer.submit("new-owner", batch()),
            Err("observability unavailable")
        );
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn stuck_remote_does_not_block_files_or_durable_incidents() {
        let (path, file) = local_file();
        let store = Store::connect("sqlite::memory:", path.join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let exporter: TelemetryExporter = {
            let gate = Arc::clone(&gate);
            Arc::new(move |_| {
                let gate = Arc::clone(&gate);
                Box::pin(async move {
                    let _ = gate.acquire().await;
                    ExportReceipt::default()
                })
            })
        };
        let writer = Observability::start(Some(store.clone()), file, Some(exporter));
        for index in 0..40 {
            let mut value = batch();
            value.batch_id = format!("batch-{index}");
            if index == 0 {
                value.incidents.push(ClientIncident {
                    id: "crash".into(),
                    occurred_at_ms: 1,
                    classification: "client_window_error".into(),
                    severity: "critical".into(),
                    summary: "fixture crash".into(),
                    fingerprint: None,
                    detail: serde_json::Value::Null,
                });
            }
            writer.submit("owner", value).unwrap();
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            while writer.health().pending() != 0
                || store.runtime_incidents(10).await.unwrap().is_empty()
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(writer.health().dropped_export_batches() > 0);
        assert_eq!(writer.health().failed_file_batches(), 0);
        assert_eq!(writer.health().failed_incident_batches(), 0);
        assert!(
            std::fs::read_to_string(path.join("telemetry.jsonl"))
                .unwrap()
                .contains("fixture crash")
        );
        gate.close();
        writer.drain().await;
        assert!(writer.health().failed_log_batches() > 0);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn otlp_dedup_precedes_delta_aggregation_and_files_need_no_exporter() {
        let (path, file) = local_file();
        let writer = Observability::start(None, file, None);
        for (index, (signal, bytes)) in crate::otlp::client_fixtures().into_iter().enumerate() {
            let id = format!("fixture-{index}");
            writer.submit_otlp("a", signal, &id, &bytes).unwrap();
            writer.submit_otlp("a", signal, &id, &bytes).unwrap();
            writer.submit_otlp("b", signal, &id, &bytes).unwrap();
        }
        writer.drain().await;
        assert_eq!(writer.health().accepted_batches(), 8);
        assert_eq!(writer.health().duplicate_batches(), 4);
        assert_eq!(writer.health().pending_bytes(), 0);
        assert_eq!(writer.health().failed_file_batches(), 0);
        let data = std::fs::read_to_string(path.join("telemetry.jsonl")).unwrap();
        assert_eq!(data.lines().count(), 8);
        let points: Vec<serde_json::Value> = data
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .filter(|r| r["name"] == "cowboy.client.websocket.reconnects")
            .collect();
        assert_eq!(points.len(), 2);
        assert_eq!(points[1]["point"]["sum"]["asDouble"], 2.0);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn otlp_full_queue_does_not_consume_metric_delta_or_dedup_identity() {
        let (path, file) = local_file();
        let writer = Observability::start(None, file, None);
        for i in 0..QUEUE_CAPACITY {
            let mut b = batch();
            b.batch_id = format!("fill-{i}");
            writer.submit("a", b).unwrap();
        }
        let (signal, bytes) = crate::otlp::client_fixtures().pop().unwrap();
        assert_eq!(
            writer.submit_otlp("a", signal, "retry", &bytes),
            Err("observability queue full")
        );
        while writer.health().pending() > 0 {
            tokio::task::yield_now().await;
        }
        writer.submit_otlp("a", signal, "retry", &bytes).unwrap();
        writer.drain().await;
        let data = std::fs::read_to_string(path.join("telemetry.jsonl")).unwrap();
        let last: serde_json::Value = serde_json::from_str(data.lines().last().unwrap()).unwrap();
        assert_eq!(last["point"]["sum"]["asDouble"], 1.0);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn queue_and_identity_cache_have_hard_bounds() {
        let (path, file) = local_file();
        let writer = Observability::start(None, file, None);
        // No await: the single-threaded runtime cannot drain during admission.
        for index in 0..QUEUE_CAPACITY {
            let mut value = batch();
            value.batch_id = format!("b-{index}");
            writer.submit("owner", value).unwrap();
        }
        let mut extra = batch();
        extra.batch_id = "overflow".into();
        assert_eq!(
            writer.submit("owner", extra),
            Err("observability queue full")
        );
        assert_eq!(writer.health().pending(), QUEUE_CAPACITY);
        writer.drain().await;
        assert_eq!(writer.health().pending_bytes(), 0);
        assert!(writer.admission.lock().seen.len() <= DEDUP_CAPACITY);
        std::fs::remove_dir_all(path).unwrap();
    }
}
