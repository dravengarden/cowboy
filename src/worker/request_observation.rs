//! Private receiver for the two pinned native runtimes' request diagnostics.
//! No Provider traffic is proxied or rewritten. The native OTLP payload is
//! projected in memory and is never persisted, forwarded, or printed.
use super::WorkerArgs;
use crate::logs::{
    now_ms,
    wait_cache::{self, Fields, Tokens},
};
use crate::provider::{CODEX_REQUEST_OBSERVATION_ENV, LaunchSpec, PreparedLaunch};
use axum::{
    Router,
    extract::{Path, Request, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use parking_lot::Mutex;
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::sync::Semaphore;

mod antigravity;
mod claude;
mod codex;
mod launch;
mod metadata;
#[cfg(test)]
mod tests;
pub(super) use metadata::Observer;

const MAX_PENDING: usize = 2048;
const MAX_KEEP_MS: i64 = 10 * 60 * 1000;
const COALESCE_MS: i64 = 15_000;

struct Ingress {
    observations: Arc<Mutex<Observations>>,
    requests: Semaphore,
    codex_header: Option<String>,
}

pub(super) struct Collector {
    pub(super) observer: Observer,
    observations: Option<Arc<Mutex<Observations>>>,
    antigravity: Option<Arc<antigravity::Recorder>>,
    receive: Option<tokio::task::JoinHandle<()>>,
    flush: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Collector {
    fn drop(&mut self) {
        if let Some(receive) = &self.receive {
            receive.abort();
        }
        if let Some(flush) = &self.flush {
            flush.abort();
        }
        if let Some(observations) = &self.observations {
            observations.lock().flush(now_ms(), true);
        }
        if let Some(antigravity) = &self.antigravity {
            antigravity.emit_coverage("closed");
        }
    }
}

impl Collector {
    pub(super) async fn start(
        spec: &mut LaunchSpec,
        args: &WorkerArgs,
        prepared: &PreparedLaunch,
        epoch: &str,
    ) -> Option<Self> {
        let prefix_fingerprint = metadata::compute_prefix_fingerprint(args);
        if args.provider == "antigravity" {
            if args.provider_generation_digest.is_empty() {
                return None;
            }
            let (observer, facts) = Observer::new(args.resume.clone());
            let recorder = Arc::new(antigravity::Recorder::new(
                args,
                epoch,
                prefix_fingerprint,
                facts,
            ));
            recorder.emit_coverage("attached");
            return Some(Self {
                observer: observer.with_antigravity(Arc::clone(&recorder)),
                observations: None,
                antigravity: Some(recorder),
                receive: None,
                flush: None,
            });
        }
        let producer = match args.provider.as_str() {
            "claude-code" => Producer::Claude,
            "codex" => Producer::Codex,
            _ => return None,
        };
        if producer == Producer::Codex {
            spec.remove_env.push(CODEX_REQUEST_OBSERVATION_ENV);
        }
        let (observer, facts) = Observer::new(args.resume.clone());
        let state = Observations::new(producer, args, epoch, prefix_fingerprint, facts);
        let Some(listener) =
            launch::open_listener(spec, args, producer, prepared.codex_request_observation).await
        else {
            state.coverage("unavailable", 0, 0);
            return None;
        };
        let address = listener.local_addr().ok()?;
        let key = uuid::Uuid::new_v4().simple().to_string();
        let endpoint = format!("http://{address}");
        let (route, codex_header) = launch::configure(spec, producer, &endpoint, key);
        state.coverage(
            if state.attached {
                "attached"
            } else {
                "unconfirmed"
            },
            0,
            0,
        );
        let observations = Arc::new(Mutex::new(state));
        let app = Router::new()
            .route(&route, post(receive))
            .with_state(Arc::new(Ingress {
                observations: Arc::clone(&observations),
                requests: Semaphore::new(2),
                codex_header,
            }));
        let receive = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let record = Arc::clone(&observations);
        let flush = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(5));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                record.lock().flush(now_ms(), false);
            }
        });
        Some(Self {
            observer,
            observations: Some(observations),
            antigravity: None,
            receive: Some(receive),
            flush: Some(flush),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Producer {
    Claude,
    Codex,
}

impl Producer {
    fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude_otel",
            Self::Codex => "codex_otel",
        }
    }

    fn semantics(self) -> &'static str {
        match self {
            Self::Claude => "messages",
            Self::Codex => "responses",
        }
    }
}

async fn receive(
    State(ingress): State<Arc<Ingress>>,
    Path(signal): Path<String>,
    request: Request,
) -> (StatusCode, axum::Json<Value>) {
    let response = axum::Json(serde_json::json!({}));
    if !matches!(signal.as_str(), "logs" | "traces") {
        return (StatusCode::NOT_FOUND, response);
    }
    if let Some(key) = &ingress.codex_header
        && (signal != "logs" || !header_matches(request.headers(), key))
    {
        return (StatusCode::UNAUTHORIZED, response);
    }
    let Ok(_permit) = ingress.requests.try_acquire() else {
        ingress.observations.lock().dropped += 1;
        return (StatusCode::TOO_MANY_REQUESTS, response);
    };
    let bytes = tokio::time::timeout(
        Duration::from_secs(2),
        axum::body::to_bytes(request.into_body(), 1024 * 1024),
    )
    .await;
    let Ok(Ok(bytes)) = bytes else {
        ingress.observations.lock().dropped += 1;
        return (StatusCode::PAYLOAD_TOO_LARGE, response);
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        ingress.observations.lock().dropped += 1;
        return (StatusCode::BAD_REQUEST, response);
    };
    ingress
        .observations
        .lock()
        .ingest(&signal, &value, now_ms());
    (StatusCode::OK, response)
}

fn header_matches(headers: &HeaderMap, expected: &str) -> bool {
    let header_values = headers.get_all(launch::CODEX_HEADER);
    let mut values = header_values.iter();
    let Some(value) = values.next() else {
        return false;
    };
    let value = value.as_bytes();
    values.next().is_none()
        && value.len() == expected.len()
        && value
            .iter()
            .zip(expected.bytes())
            .fold(0, |diff, (left, right)| diff | (left ^ right))
            == 0
}

#[derive(Clone)]
struct Attempt {
    id: String,
    thread: String,
    turn: Option<String>,
    role: &'static str,
    model: Option<String>,
    native_release: Option<String>,
    start_ms: i64,
    end_ms: i64,
    start_source: &'static str,
    outcome: &'static str,
    tokens: Tokens,
    cost: Option<u64>,
    status: Option<u64>,
    overage: Option<bool>,
    retention: &'static str,
    usage_source: &'static str,
}

struct Observations {
    producer: Producer,
    attached: bool,
    session: String,
    epoch: String,
    release: String,
    prefix_fingerprint: String,
    facts: metadata::Facts,
    managed: bool,
    fresh: bool,
    first_in_epoch: bool,
    renewed_at: Option<i64>,
    compacted: VecDeque<i64>,
    compact_coverage: bool,
    turns: HashMap<String, u64>,
    seen: VecDeque<String>,
    claude: claude::Pending,
    codex: codex::Pending,
    dropped: u64,
    unmatched: u64,
}

impl Observations {
    fn new(
        producer: Producer,
        args: &WorkerArgs,
        epoch: &str,
        prefix_fingerprint: String,
        facts: metadata::Facts,
    ) -> Self {
        Self {
            producer,
            attached: producer == Producer::Claude,
            session: args.session_id.clone(),
            epoch: epoch.into(),
            release: args.provider_version.clone(),
            prefix_fingerprint,
            facts,
            managed: args
                .execution_binding
                .as_ref()
                .and_then(crate::execution_environment::ExecutionBinding::managed)
                .is_some(),
            fresh: args.resume.is_none() && args.native_fork.is_none(),
            first_in_epoch: true,
            renewed_at: None,
            compacted: VecDeque::new(),
            compact_coverage: true,
            turns: HashMap::new(),
            seen: VecDeque::new(),
            claude: claude::Pending::default(),
            codex: codex::Pending::default(),
            dropped: 0,
            unmatched: 0,
        }
    }

    fn sync_facts(&mut self) {
        if self.fresh {
            self.renewed_at = self.facts.first_prompt.get().copied().flatten();
        }
        let mut dropped = 0;
        for _ in 0..metadata::COMPACTIONS {
            let Ok(at) = self.facts.compacted.try_recv() else {
                break;
            };
            if self.compacted.len() == metadata::COMPACTIONS {
                self.compacted.pop_front();
                dropped += 1;
            }
            self.compacted.push_back(at);
        }
        dropped += self.facts.dropped.swap(0, Ordering::Relaxed);
        if dropped != 0 {
            self.dropped += dropped;
            // The missing boundary could have fallen before any subsequent
            // attempt. Preserve positive evidence, and omit negative claims.
            self.compact_coverage = false;
        }
    }

    fn ingest(&mut self, signal: &str, value: &Value, now: i64) {
        if !self.attached {
            // A signed Codex adapter can preserve an existing exporter by
            // declining the binding. Report attachment on authenticated ingress.
            self.attached = true;
            self.coverage("attached", 0, 0);
        }
        let (resources, scopes, records) = if signal == "logs" {
            ("resourceLogs", "scopeLogs", "logRecords")
        } else {
            ("resourceSpans", "scopeSpans", "spans")
        };
        let records = array(&value[resources])
            .flat_map(|resource| array(&resource[scopes]))
            .flat_map(|scope| array(&scope[records]));
        for record in records {
            self.dropped += match self.producer {
                Producer::Claude => self.claude.ingest(signal, record, now),
                Producer::Codex if signal == "logs" => self.codex.ingest(record, now),
                Producer::Codex => 0,
            };
        }
    }

    fn flush(&mut self, now: i64, force: bool) {
        self.sync_facts();
        let mut observed = match self.producer {
            Producer::Claude => self.claude.drain(now, force, &mut self.unmatched),
            Producer::Codex => self.codex.drain(
                now,
                force,
                self.facts.native_id.borrow().clone().as_deref(),
                &mut self.unmatched,
            ),
        };
        observed.sort_by_key(|attempt| attempt.start_ms);
        for attempt in observed {
            self.record(attempt);
        }
        if self.dropped != 0 || self.unmatched != 0 || force {
            let dropped = std::mem::take(&mut self.dropped);
            let unmatched = std::mem::take(&mut self.unmatched);
            let outcome = match (force, self.attached) {
                (true, true) => "closed",
                (true, false) => "unavailable",
                (false, _) => "partial",
            };
            self.coverage(outcome, dropped, unmatched);
        }
    }

    fn record(&mut self, attempt: Attempt) {
        if self.seen.contains(&attempt.id) {
            return;
        }
        if self.seen.len() >= MAX_PENDING * 2 {
            self.seen.pop_front();
        }
        self.seen.push_back(attempt.id.clone());
        let index = attempt.turn.as_ref().map(|turn| {
            if self.turns.len() >= MAX_PENDING && !self.turns.contains_key(turn) {
                self.turns.clear();
                self.dropped += 1;
            }
            let index = self.turns.entry(turn.clone()).or_default();
            *index += 1;
            *index
        });
        let parent = attempt.role == "parent";
        let (first_rollover, first_compact, first_in_epoch) = if parent {
            let rollover = self
                .renewed_at
                .is_some_and(|start| start <= attempt.start_ms);
            if rollover {
                self.renewed_at = None;
            }
            self.fresh = false;
            // Consume only boundaries before this start, independent of turn
            // index or idle cohort.
            let mut compact = false;
            while self
                .compacted
                .front()
                .is_some_and(|at| *at <= attempt.start_ms)
            {
                self.compacted.pop_front();
                compact = true;
            }
            let first_epoch = self.first_in_epoch;
            self.first_in_epoch = false;
            (
                Some(rollover),
                (compact || self.compact_coverage).then_some(compact),
                Some(first_epoch),
            )
        } else {
            (None, None, None)
        };
        let role = if parent && self.managed {
            "managed_child"
        } else {
            attempt.role
        };
        let mut fields = self
            .common()
            .string("role", role)
            .string("native_session_id", &attempt.thread)
            .string("outcome", attempt.outcome)
            .string("start_source", attempt.start_source)
            .string("usage_source", attempt.usage_source)
            .string("retention_tier", attempt.retention)
            .integer(
                "request_started_at_ms",
                u64::try_from(attempt.start_ms).ok(),
            )
            .integer(
                "cowboy.duration_ms",
                u64::try_from(attempt.end_ms.saturating_sub(attempt.start_ms)).ok(),
            )
            .integer("request_index_in_turn", index)
            .boolean("is_first_after_rollover", first_rollover)
            .boolean("is_first_after_compact", first_compact)
            .boolean("is_first_in_worker_epoch", first_in_epoch)
            .boolean("is_using_overage", attempt.overage)
            .integer("http.response.status_code", attempt.status)
            .integer("estimated_api_equiv_microusd", attempt.cost);
        for (key, value) in [
            ("model", attempt.model.as_deref()),
            ("native_runtime_release", attempt.native_release.as_deref()),
            ("native_turn_id", attempt.turn.as_deref()),
        ] {
            if let Some(value) = value {
                fields = fields.string(key, value);
            }
        }
        wait_cache::record(
            wait_cache::REQUEST,
            &[&self.session, &self.epoch, &attempt.id],
            Some(attempt.start_ms),
            attempt.tokens.fields(fields, self.producer.semantics()),
        );
    }

    fn common(&self) -> Fields {
        Fields::default()
            .string("cowboy.session.id", &self.session)
            .string(
                "provider",
                match self.producer {
                    Producer::Claude => "claude-code",
                    Producer::Codex => "codex",
                },
            )
            .string("producer", self.producer.name())
            .string("producer_release", &self.release)
            .string("prefix_fingerprint", &self.prefix_fingerprint)
            .integer("producer_schema_version", 1_u64)
    }

    fn coverage(&self, outcome: &str, dropped: u64, unmatched: u64) {
        wait_cache::record(
            wait_cache::COVERAGE,
            &[],
            None,
            self.common()
                .string("outcome", outcome)
                .integer("producer_dropped", dropped)
                .integer("producer_unmatched", unmatched),
        );
    }
}

fn array(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

fn integer(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

fn millis(nanos: &Value) -> Option<i64> {
    i64::try_from(integer(nanos)? / 1_000_000).ok()
}

fn bounded(value: Option<&str>) -> Option<String> {
    let value = value?;
    (!value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:.@".contains(&b)))
    .then(|| value.to_owned())
}

struct Attrs<'a>(&'a Value);
impl<'a> Attrs<'a> {
    fn of(value: &'a Value) -> Self {
        Self(&value["attributes"])
    }
    fn field(&self, key: &str, ty: &str) -> Option<&'a Value> {
        array(self.0)
            .find(|attribute| attribute["key"].as_str() == Some(key))?
            .get("value")?
            .get(ty)
    }
    fn text(&self, key: &str) -> Option<&'a str> {
        self.field(key, "stringValue")?.as_str()
    }
    fn count(&self, key: &str) -> Option<u64> {
        self.field(key, "intValue")
            .and_then(integer)
            .or_else(|| self.text(key)?.parse().ok())
    }
    fn boolean(&self, key: &str) -> Option<bool> {
        self.field(key, "boolValue")
            .and_then(Value::as_bool)
            .or_else(|| self.text(key)?.parse().ok())
    }
}
