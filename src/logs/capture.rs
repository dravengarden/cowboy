use super::data::{Entry, Level, now_ms, text};
use super::storage::{SqliteStore, atomic_json, private_read};
use anyhow::{Result, ensure};
use opentelemetry_proto::tonic::{
    common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value},
    logs::v1::LogRecord,
    resource::v1::Resource,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    Layer, filter::EnvFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

const MAX_PENDING_BYTES: usize = 8 * 1024 * 1024;
static CAPTURE: OnceLock<Arc<Capture>> = OnceLock::new();

/// Non-secret correlation supplied by the process owner, never environment
/// capabilities, prompts, command parameters or provider authentication.
#[derive(Clone, Default)]
pub struct Context {
    pub service: String,
    pub machine: String,
    pub session: String,
    pub environment: String,
    pub generation: String,
}
impl Context {
    pub fn new(service: &str) -> Self {
        Self {
            service: service.into(),
            ..Default::default()
        }
    }
    fn resource(&self, instance: &str) -> Resource {
        Resource {
            attributes: vec![
                text("service.name", &self.service),
                text("service.version", env!("CARGO_PKG_VERSION")),
                text("service.instance.id", instance),
                text("host.id", &self.machine),
                text("cowboy.generation", &self.generation),
                text(
                    "process.executable.path",
                    &std::env::current_exe()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                ),
            ],
            ..Default::default()
        }
    }
}

struct Pending {
    entry: Entry,
    bytes: usize,
}
struct Capture {
    normal: mpsc::SyncSender<Pending>,
    critical: mpsc::SyncSender<Pending>,
    context: parking_lot::RwLock<Context>,
    instance: String,
    pending: AtomicUsize,
    admitted: AtomicU64,
    dropped: AtomicU64,
    failures: AtomicU64,
    last_failure: parking_lot::Mutex<Option<String>>,
    written: AtomicU64,
    forward: AtomicBool,
    stopping: AtomicBool,
}
impl Capture {
    fn submit(&self, entry: Entry) {
        let bytes = entry.protobuf.len() + entry.body.len() + 2048;
        let important = entry.severity >= 13;
        let budget = if important {
            MAX_PENDING_BYTES
        } else {
            MAX_PENDING_BYTES - 1024 * 1024
        };
        let reserved = bytes <= super::data::MAX_RECORD_BYTES + 10240
            && !self.stopping.load(Ordering::Relaxed)
            && self
                .pending
                .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |v| {
                    v.checked_add(bytes).filter(|n| *n <= budget)
                })
                .is_ok();
        if !reserved {
            self.drop_record();
            return;
        }
        let tx = if important {
            &self.critical
        } else {
            &self.normal
        };
        if tx.try_send(Pending { entry, bytes }).is_ok() {
            self.admitted.fetch_add(1, Ordering::Relaxed);
        } else {
            self.pending.fetch_sub(bytes, Ordering::Relaxed);
            self.drop_record();
        }
    }
    fn drop_record(&self) {
        let count = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
        if count.is_power_of_two() {
            eprintln!("cowboy logs: local evidence queue full; dropped_records={count}");
        }
    }
    fn failed(&self, error: &anyhow::Error) {
        let kind = if let Some(rusqlite::Error::SqliteFailure(code, _)) =
            error.downcast_ref::<rusqlite::Error>()
        {
            format!("sqlite_{:?}", code.code)
        } else if let Some(error) = error.downcast_ref::<std::io::Error>() {
            format!("io_{:?}", error.kind())
        } else if error.is::<super::storage::Busy>() {
            "store_busy".into()
        } else if error.is::<serde_json::Error>() {
            "invalid_configuration".into()
        } else {
            "storage_integrity_or_policy".into()
        };
        *self.last_failure.lock() = Some(kind.clone());
        let count = self.failures.fetch_add(1, Ordering::Relaxed) + 1;
        if count.is_power_of_two() {
            eprintln!(
                "cowboy logs: local evidence write/maintenance failed; reason={kind}; failures={count}; inspect cowboy logs status"
            );
        }
    }
    fn entry(&self, name: &str, severity: i32, attributes: Vec<KeyValue>) -> Entry {
        let now = now_ms();
        let context = self.context.read();
        let mut attrs = vec![
            text("cowboy.session.id", &context.session),
            text("cowboy.execution.environment.id", &context.environment),
        ];
        for attribute in attributes {
            attrs.retain(|a| a.key != attribute.key);
            attrs.push(attribute);
        }
        Entry::log(
            context.resource(&self.instance),
            InstrumentationScope {
                name: "cowboy.runtime".into(),
                version: "1".into(),
                ..Default::default()
            },
            LogRecord {
                time_unix_nano: now.max(0) as u64 * 1_000_000,
                observed_time_unix_nano: now.max(0) as u64 * 1_000_000,
                severity_number: severity,
                event_name: name.into(),
                body: Some(AnyValue {
                    value: Some(any_value::Value::StringValue(name.into())),
                }),
                attributes: attrs,
                ..Default::default()
            },
            now,
        )
    }
}

/// Holds a bounded flush lifetime; SIGKILL can still lose queued records. The
/// fsynced SQLite transactions and writer health distinguish that from delivery.
pub struct Guard {
    capture: Arc<Capture>,
    job: Option<std::thread::JoinHandle<()>>,
    finished: mpsc::Receiver<()>,
    outcome_pending: bool,
}
impl Guard {
    /// Track the owning process result, including early returns or unwinding.
    #[must_use]
    pub fn track_outcome(mut self) -> Self {
        self.outcome_pending = true;
        self
    }

    /// Record only a closed error category; arbitrary error chains may include
    /// credentials, command arguments or provider response bodies.
    pub fn finish<T>(mut self, result: Result<T>) -> Result<T> {
        self.outcome_pending = false;
        if let Err(error) = &result {
            let reason = error
                .downcast_ref::<std::io::Error>()
                .map(|e| format!("io_{:?}", e.kind()))
                .unwrap_or_else(|| "returned_error".into());
            self.capture.submit(self.capture.entry(
                "cowboy.process.failed",
                17,
                vec![text("error.type", &reason)],
            ));
        }
        result
    }

    /// Enrollment may resolve the Machine identity after logging has started.
    pub fn set_machine(&self, machine: &str) -> Result<()> {
        ensure!(
            machine.len() <= 256 && !machine.chars().any(char::is_control),
            "invalid log Machine identity"
        );
        self.capture.context.write().machine = machine.into();
        Ok(())
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if self.outcome_pending {
            self.capture.submit(self.capture.entry(
                "cowboy.process.outcome_missing",
                13,
                vec![text(
                    "error.type",
                    if std::thread::panicking() {
                        "panic"
                    } else {
                        "early_return_or_cancellation"
                    },
                )],
            ));
        }
        self.capture
            .submit(self.capture.entry("cowboy.logs.writer.stopping", 9, vec![]));
        self.capture.stopping.store(true, Ordering::Release);
        if let Some(job) = self.job.take() {
            if self.finished.recv_timeout(Duration::from_secs(5)).is_ok() || job.is_finished() {
                let _ = job.join();
            } else {
                eprintln!(
                    "cowboy logs: shutdown flush deadline exceeded; queued evidence may be lost"
                );
                // A stalled filesystem must not hold the execution process
                // hostage. Its stale heartbeat remains evidence of uncertainty.
                drop(job);
            }
        }
    }
}

pub fn directory(default: &Path) -> PathBuf {
    std::env::var_os("COWBOY_LOGS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            default
                .canonicalize()
                .unwrap_or_else(|_| default.to_owned())
                .join("logs")
        })
}

pub(crate) fn init_stderr() {
    // EnvFilter belongs only to stderr. A user's RUST_LOG=off must not disable
    // the independently bounded failure-evidence lane.
    let fmt = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")));
    let _ = tracing_subscriber::registry()
        .with(fmt)
        .with(LogLayer)
        .try_init();
}

/// Initialize the owned log directory and all-process tracing bridge.
///
/// # Errors
/// Invalid/private storage or a second initialization refuses startup rather
/// than pretending diagnostics were enabled. Subsequent failures are observable
/// and cannot block an execution/control socket.
pub fn init(directory: PathBuf, context: Context) -> Result<Guard> {
    ensure!(
        [
            &context.service,
            &context.machine,
            &context.session,
            &context.environment,
            &context.generation
        ]
        .iter()
        .all(|s| s.len() <= 256),
        "log identity exceeds bounds"
    );
    let store = SqliteStore::open(directory, true)?;
    let policy = store.policy()?;
    // Queries can own the lock for seconds. Defer routine startup cleanup to
    // the writer instead of making logging contention prevent execution.
    if let Err(error) = store.maintain_if_due(now_ms())
        && !error.is::<super::storage::Busy>()
    {
        return Err(error);
    }
    let (normal, rx) = mpsc::sync_channel(1024);
    let (critical, errors) = mpsc::sync_channel(256);
    let capture = Arc::new(Capture {
        normal,
        critical,
        context: parking_lot::RwLock::new(context),
        instance: uuid::Uuid::new_v4().simple().to_string(),
        pending: AtomicUsize::new(0),
        admitted: AtomicU64::new(0),
        dropped: AtomicU64::new(0),
        failures: AtomicU64::new(0),
        last_failure: parking_lot::Mutex::new(None),
        written: AtomicU64::new(0),
        forward: AtomicBool::new(policy.forward_runtime),
        stopping: AtomicBool::new(false),
    });
    ensure!(
        CAPTURE.set(Arc::clone(&capture)).is_ok(),
        "logs already initialized"
    );
    init_stderr();
    let writer = Arc::clone(&capture);
    let (done, finished) = mpsc::sync_channel(1);
    let job = std::thread::Builder::new()
        .name("cowboy-local-otel".into())
        .spawn(move || {
            run(store, writer, rx, errors);
            let _ = done.send(());
        })?;
    capture.submit(capture.entry("cowboy.logs.writer.started", 9, vec![]));
    Ok(Guard {
        capture,
        job: Some(job),
        finished,
        outcome_pending: false,
    })
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Health {
    schema: u16,
    instance: String,
    service: String,
    pid: u32,
    updated_ms: i64,
    stopped: bool,
    admitted: u64,
    written: u64,
    dropped: u64,
    failures: u64,
    #[serde(default)]
    last_failure: Option<String>,
    pending_bytes: usize,
}
fn run(
    store: SqliteStore,
    capture: Arc<Capture>,
    rx: mpsc::Receiver<Pending>,
    errors: mpsc::Receiver<Pending>,
) {
    let mut maintenance = Instant::now() - Duration::from_secs(30);
    let mut health_time = maintenance;
    let mut shutdown = None;
    loop {
        if capture.stopping.load(Ordering::Acquire) {
            shutdown.get_or_insert_with(Instant::now);
        }
        let first = errors
            .try_recv()
            .ok()
            .or_else(|| rx.recv_timeout(Duration::from_millis(100)).ok());
        if let Some(first) = first {
            let mut batch = vec![first.entry];
            let mut bytes = first.bytes;
            while batch.len() < 32 {
                let Some(next) = errors.try_recv().ok().or_else(|| rx.try_recv().ok()) else {
                    break;
                };
                bytes += next.bytes;
                batch.push(next.entry);
            }
            let started = Instant::now();
            let written = loop {
                match store.append(&batch, now_ms()) {
                    Ok(()) => break true,
                    Err(error)
                        if error.is::<super::storage::Busy>()
                            && started.elapsed() < Duration::from_secs(5) =>
                    {
                        // A lock failure proves no write began. Retain the batch
                        // during a query; never replay a partial database error.
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => {
                        capture.failed(&error);
                        break false;
                    }
                }
            };
            if written {
                capture
                    .written
                    .fetch_add(batch.len() as u64, Ordering::Relaxed);
            }
            capture.pending.fetch_sub(bytes, Ordering::Relaxed);
        }
        if maintenance.elapsed() >= Duration::from_secs(30) {
            if let Err(error) = store.maintain_if_due(now_ms())
                && !error.is::<super::storage::Busy>()
            {
                capture.failed(&error);
            }
            match store.policy() {
                Ok(policy) => {
                    capture
                        .forward
                        .store(policy.forward_runtime, Ordering::Relaxed);
                }
                Err(error) => capture.failed(&error),
            }
            maintenance = Instant::now();
        }
        let stop = shutdown.is_some_and(|s| {
            capture.pending.load(Ordering::Relaxed) == 0 || s.elapsed() >= Duration::from_secs(5)
        });
        if stop || health_time.elapsed() >= Duration::from_secs(5) {
            let health = Health {
                schema: 1,
                instance: capture.instance.clone(),
                service: capture.context.read().service.clone(),
                pid: std::process::id(),
                updated_ms: now_ms(),
                stopped: stop,
                admitted: capture.admitted.load(Ordering::Relaxed),
                written: capture.written.load(Ordering::Relaxed),
                dropped: capture.dropped.load(Ordering::Relaxed),
                failures: capture.failures.load(Ordering::Relaxed),
                last_failure: capture.last_failure.lock().clone(),
                pending_bytes: capture.pending.load(Ordering::Relaxed),
            };
            if let Err(error) = atomic_json(
                &store
                    .directory
                    .join(format!("writer-{}.json", capture.instance)),
                &health,
            ) {
                capture.failed(&error);
            }
            health_time = Instant::now();
        }
        if stop {
            break;
        }
    }
}
fn health_paths(directory: &Path, limit: usize) -> Result<(Vec<PathBuf>, bool)> {
    let mut paths = Vec::new();
    for (index, entry) in std::fs::read_dir(directory)?.enumerate() {
        if index >= 16384 {
            return Ok((paths, true));
        }
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(id) = name
            .strip_prefix("writer-")
            .and_then(|s| s.strip_suffix(".json"))
            && id.len() == 32
            && id.bytes().all(|b| b.is_ascii_hexdigit())
        {
            if paths.len() == limit {
                return Ok((paths, true));
            }
            paths.push(entry.path());
        }
    }
    Ok((paths, false))
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct HealthSummary {
    compacted_records: u64,
    oldest_ms: i64,
    newest_ms: i64,
    issues: Vec<String>,
}
#[derive(Serialize)]
pub(crate) struct HealthReport {
    pub(crate) writers: Vec<Health>,
    pub(crate) summary: HealthSummary,
    pub(crate) inventory_truncated: bool,
}
fn health_summary(directory: &Path) -> Result<HealthSummary> {
    let path = directory.join("writer-history.json");
    if !path.try_exists()? {
        return Ok(HealthSummary::default());
    }
    Ok(serde_json::from_slice(&private_read(&path, 8192)?)?)
}
pub(crate) fn read_health(directory: &Path) -> Result<HealthReport> {
    let (paths, inventory_truncated) = health_paths(directory, 4096)?;
    Ok(HealthReport {
        writers: paths
            .iter()
            .map(|path| Ok(serde_json::from_slice(&private_read(path, 8192)?)?))
            .collect::<Result<_>>()?,
        summary: health_summary(directory)?,
        inventory_truncated,
    })
}

impl Health {
    fn issues(&self, now: i64) -> Vec<String> {
        let mut issues = Vec::new();
        if self.dropped > 0 {
            issues.push("writer_reported_queue_loss".into());
        }
        if self.failures > 0 {
            issues.push("writer_reported_storage_failures".into());
        }
        if self.stopped && self.pending_bytes > 0 {
            issues.push("writer_shutdown_flush_incomplete".into());
        }
        if !self.stopped && now.saturating_sub(self.updated_ms) > 30_000 {
            issues.push("writer_health_stale".into());
        }
        issues
    }
}

pub(crate) fn health_issues(directory: &Path, from: i64) -> Result<Vec<String>> {
    let report = read_health(directory)?;
    let mut issues = Vec::new();
    if report.inventory_truncated {
        issues.push("writer_health_inventory_truncated".into());
    }
    if report.summary.compacted_records > 0 && report.summary.newest_ms >= from {
        issues.push("writer_history_compacted".into());
        issues.extend(report.summary.issues);
    }
    for h in report.writers.into_iter().filter(|h| h.updated_ms >= from) {
        for issue in h.issues(now_ms()) {
            if !issues.contains(&issue) {
                issues.push(issue);
            }
        }
    }
    Ok(issues)
}
pub(crate) fn cleanup_health(directory: &Path, cutoff: i64) -> Result<()> {
    let mut records = Vec::new();
    for path in health_paths(directory, 8192)?.0 {
        let record: Health = serde_json::from_slice(&private_read(&path, 8192)?)?;
        records.push((record, path));
    }
    records.sort_by_key(|r| r.0.updated_ms);
    let mut summary = health_summary(directory)?;
    if summary.newest_ms < cutoff {
        summary = HealthSummary::default();
    }
    let mut overflow = records.len().saturating_sub(4096);
    let now = now_ms();
    let mut removals = Vec::new();
    for (record, path) in records {
        if record.updated_ms < cutoff
            || (overflow > 0 && (record.stopped || now.saturating_sub(record.updated_ms) > 30_000))
        {
            if record.updated_ms >= cutoff {
                summary.oldest_ms = if summary.compacted_records == 0 {
                    record.updated_ms
                } else {
                    summary.oldest_ms.min(record.updated_ms)
                };
                summary.newest_ms = summary.newest_ms.max(record.updated_ms);
                summary.compacted_records = summary.compacted_records.saturating_add(1);
                for issue in record.issues(now) {
                    if !summary.issues.contains(&issue) {
                        summary.issues.push(issue);
                    }
                }
            }
            removals.push(path);
            overflow = overflow.saturating_sub(1);
        }
    }
    // Persist uncertainty before removing its detailed record. A crash can
    // conservatively count it twice, but cannot turn unknown loss into health.
    atomic_json(&directory.join("writer-history.json"), &summary)?;
    for path in removals {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

struct LogLayer;
impl<S: Subscriber> Layer<S> for LogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let Some(capture) = CAPTURE.get() else {
            return;
        };
        let meta = event.metadata();
        if !meta.target().starts_with("cowboy") || *meta.level() > tracing::Level::INFO {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        let name = fields
            .name
            .unwrap_or_else(|| format!("{}:{}", meta.target(), meta.line().unwrap_or(0)));
        fields
            .attributes
            .push(text("code.file.path", meta.file().unwrap_or("")));
        fields.attributes.push(text(
            "code.line.number",
            &meta.line().unwrap_or(0).to_string(),
        ));
        capture.submit(capture.entry(
            &name,
            Level::parse(meta.level().as_str()).number(),
            fields.attributes,
        ));
    }
}
#[derive(Default)]
struct Fields {
    name: Option<String>,
    attributes: Vec<KeyValue>,
}
fn field_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "session" | "session_id" => "cowboy.session.id",
        "environment_id" => "cowboy.execution.environment.id",
        "incarnation" => "cowboy.execution.incarnation",
        "machine" | "machine_id" => "cowboy.machine.id",
        "runtime_machine" => "cowboy.runtime.machine.id",
        "reason" => "error.type",
        "duration_ms" => "cowboy.duration_ms",
        "bytes" => "cowboy.bytes",
        "pending" => "cowboy.pending",
        "cursor" => "cowboy.cursor",
        "count" => "cowboy.count",
        "exit_code" => "process.exit.code",
        "error_code" => "rpc.jsonrpc.error_code",
        "retained_bytes" => "cowboy.retained_bytes",
        "retained_events" => "cowboy.retained_events",
        "operation" => "cowboy.operation",
        "generation" => "cowboy.generation",
        _ => return None,
    })
}
impl Fields {
    fn string(&mut self, field: &Field, value: &str) {
        if value.len() > 256 || value.chars().any(char::is_control) {
            return;
        }
        if field.name() == "event_name"
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            self.name = Some(value.into());
            return;
        }
        if let Some(name) = field_name(field.name()) {
            self.attributes.push(text(name, value));
        }
    }
}
impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.string(field, value);
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field_name(field.name()).is_some() {
            self.string(field, &format!("{value:?}"));
        }
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        if let Some(name) = field_name(field.name())
            && let Ok(value) = i64::try_from(value)
        {
            self.attributes.push(KeyValue {
                key: name.into(),
                value: Some(AnyValue {
                    value: Some(any_value::Value::IntValue(value)),
                }),
                ..Default::default()
            });
        }
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        if let Some(name) = field_name(field.name()) {
            self.attributes.push(KeyValue {
                key: name.into(),
                value: Some(AnyValue {
                    value: Some(any_value::Value::IntValue(value)),
                }),
                ..Default::default()
            });
        }
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        if let Some(name) = field_name(field.name())
            && value.is_finite()
        {
            self.attributes.push(KeyValue {
                key: name.into(),
                value: Some(AnyValue {
                    value: Some(any_value::Value::DoubleValue(value)),
                }),
                ..Default::default()
            });
        }
    }
}

#[cfg(any(feature = "full", feature = "machine-host"))]
pub(crate) fn forward_runtime() -> bool {
    CAPTURE
        .get()
        .is_none_or(|capture| capture.forward.load(Ordering::Relaxed))
}

#[cfg(any(feature = "full", feature = "machine-host"))]
pub(crate) fn runtime_spans(session: &str, records: &[crate::runtime_trace::SpanRecord]) {
    use opentelemetry_proto::tonic::trace::v1::{Span, Status};
    let Some(capture) = CAPTURE.get() else {
        return;
    };
    for record in records.iter().filter(|r| r.valid()) {
        let span = Span {
            trace_id: super::data::unhex(record.context.trace_id()),
            span_id: super::data::unhex(record.context.span_id()),
            parent_span_id: super::data::unhex(&record.parent_span_id),
            name: record.stage.name().into(),
            start_time_unix_nano: record.started_ns,
            end_time_unix_nano: record.started_ns.saturating_add(record.duration_ns),
            attributes: vec![text("cowboy.session.id", session)],
            status: Some(Status {
                code: if record.outcome == crate::runtime_trace::Outcome::Error {
                    2
                } else {
                    1
                },
                message: String::new(),
            }),
            ..Default::default()
        };
        capture.submit(Entry::span(
            capture.context.read().resource(&capture.instance),
            InstrumentationScope {
                name: "cowboy.runtime".into(),
                ..Default::default()
            },
            span,
            now_ms(),
        ));
    }
}

#[cfg(feature = "machine-host")]
pub(crate) fn provider_usage(event: &crate::machine_protocol::ProviderUsageEvent) {
    let Some(capture) = CAPTURE.get() else {
        return;
    };
    // Typed, validated accounting metadata only. No URL, headers, request,
    // response, account identity or prefix material crosses this projection.
    let mut attributes = vec![
        text("cowboy.provider.name", &event.provider),
        text("cowboy.provider.model", &event.model),
        text("cowboy.provider.operation", &event.operation),
        text("cowboy.provider.request_purpose", &event.request_purpose),
        text(
            "cowboy.provider.cache_observation",
            &event.cache_observation,
        ),
    ];
    for (key, value) in [
        ("http.response.status_code", Some(u64::from(event.status))),
        ("cowboy.duration_ms", event.duration_ms),
        ("cowboy.provider.input_tokens", event.input_tokens),
        ("cowboy.provider.output_tokens", event.output_tokens),
        ("cowboy.provider.cache_hit_tokens", event.cache_hit_tokens),
        ("cowboy.provider.cache_miss_tokens", event.cache_miss_tokens),
        ("cowboy.request.bytes", event.request_bytes),
    ] {
        if let Some(value) = value.and_then(|v| i64::try_from(v).ok()) {
            attributes.push(KeyValue {
                key: key.into(),
                value: Some(AnyValue {
                    value: Some(any_value::Value::IntValue(value)),
                }),
                ..Default::default()
            });
        }
    }
    let failed = event.status >= 400 || event.completed == Some(false);
    if failed {
        attributes.push(text(
            "error.type",
            if event.status >= 400 {
                "provider_http_error"
            } else {
                "provider_stream_incomplete"
            },
        ));
    }
    capture.submit(capture.entry(
        "cowboy.provider.request_observed",
        if failed { 17 } else { 9 },
        attributes,
    ));
}
