//! Shared memory for ACP Providers whose packages have no memory adapter.
//!
//! The worker is the one place every such Provider passes through, so memory
//! lives here for all of them: tools through ACP `mcpServers`, recall as a
//! leading prompt block, and learning from the session's own updates. Everything
//! reaches the backend through the Machine relay named by the session
//! descriptor; this module needs no credential or network path of its own.
//!
//! Memory is never required. Nothing here returns an error to the turn, a turn
//! waits for memory only for a bounded time, and after a failed exchange turns
//! stop waiting until delivery succeeds again. Completed turns stay in a private
//! outbox during an outage. Provider-native memory is left as it is.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, Instant};

use agent_client_protocol::schema::v1::{
    ContentBlock, HttpHeader, McpServer, McpServerHttp, SessionUpdate, ToolCallStatus,
};
use anyhow::{Context as _, Result, ensure};
use parking_lot::Mutex;
use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

const EXCHANGE: Duration = Duration::from_secs(4);
/// The longest memory may hold a user turn.
const BEGIN_BUDGET: Duration = Duration::from_secs(5);
/// After a failed exchange, turns skip recall for this long.
const BREAKER: Duration = Duration::from_secs(30);
const DELIVERY_INTERVAL: Duration = Duration::from_secs(10);
const MAX_OUTBOX: usize = 512;
const MAX_EVENTS: usize = 200;
const MAX_SEGMENT_BYTES: usize = 200_000;
const MAX_EVENT_CHARS: usize = 12_000;
const CONTEXT_BUDGET: u32 = 6000;
const UNAVAILABLE: &str =
    "[Matrix memory unavailable for this turn. Continue normally and verify facts directly.]";

/// The Provider packages this worker serves. Standard Claude and Codex keep
/// their signed adapters; another Provider has no backend client identity.
fn bridged(provider_id: &str) -> bool {
    matches!(
        provider_id,
        "grok" | "deepseek-harness" | "claude-deepseek" | "codex-deepseek"
    )
}

static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)-----BEGIN [^-]*PRIVATE KEY-----.*?-----END [^-]*PRIVATE KEY-----")
        .expect("private key pattern")
});
static CREDENTIAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)bearer\s+[a-z0-9._~+/-]{12,}|(?:sk-|gh[pousr]_|github_pat_)[a-z0-9_-]{16,}|(?:api[_-]?key|access[_-]?token|password|secret)\s*[:=]\s*["']?[^\s"',;]{6,}"#,
    )
    .expect("credential pattern")
});

fn public_text(value: &str) -> String {
    let value = PRIVATE_KEY.replace_all(value, "[redacted private key]");
    CREDENTIAL
        .replace_all(&value, "[redacted credential]")
        .chars()
        .take(MAX_EVENT_CHARS)
        .collect()
}

#[derive(Default)]
struct Segment {
    turn: String,
    events: Vec<Value>,
    bytes: usize,
    /// The agent's message so far: it arrives as chunks and is one event.
    assistant: String,
}

impl Segment {
    fn add(&mut self, role: &str, text: &str) {
        let text = public_text(text);
        if text.trim().is_empty() || text.contains("[Matrix memory") {
            return;
        }
        if self.events.len() >= MAX_EVENTS || self.bytes + text.len() > MAX_SEGMENT_BYTES {
            return;
        }
        self.bytes += text.len();
        let id = format!("e{}", self.events.len());
        self.events
            .push(json!({"id": id, "role": role, "text": text}));
    }

    fn close_message(&mut self) {
        let message = std::mem::take(&mut self.assistant);
        self.add("assistant", &message);
    }
}

pub struct AcpMemory {
    endpoint: String,
    token: String,
    server: String,
    tools: bool,
    recall: bool,
    observe: bool,
    outbox: PathBuf,
    client: reqwest::Client,
    segment: Mutex<Segment>,
    unavailable_until: Mutex<Option<Instant>>,
    delivery: tokio::sync::Mutex<()>,
}

fn new_turn() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir()
            && metadata.mode() & 0o077 == 0
            && metadata.uid() == rustix::process::geteuid().as_raw(),
        "memory outbox is not private"
    );
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let partial = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let published = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&partial)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&partial, path)?;
        let parent = path.parent().context("outbox entry has no directory")?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if published.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    published
}

/// Keep evidence off the turn: a stalled disk must not hold the ACP runtime.
fn store(outbox: PathBuf, payload: &Value) -> Result<()> {
    ensure!(
        queued(&outbox)?.len() < MAX_OUTBOX,
        "memory delivery queue is full"
    );
    let bytes = serde_json::to_vec(payload)?;
    let name = format!("{:x}.json", Sha256::digest(&bytes));
    write_private(&outbox.join(name), &bytes)
}

fn queued(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            paths.push(path);
        }
    }
    Ok(paths)
}

impl AcpMemory {
    /// This session's memory, from its descriptor. `None` for every reason a
    /// session runs without memory, including every fault.
    pub fn open(provider_id: &str) -> Option<Arc<Self>> {
        if !bridged(provider_id) {
            return None;
        }
        let path = std::env::var_os("COWBOY_SESSION_EXTENSIONS")?;
        if path == "disabled" {
            return None;
        }
        Self::from_descriptor(Path::new(&path))
            .inspect_err(|error| {
                tracing::warn!(error = %format!("{error:#}"), "session runs without memory");
            })
            .ok()
            .flatten()
    }

    fn from_descriptor(path: &Path) -> Result<Option<Arc<Self>>> {
        let descriptor: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(descriptor["schema"] == 1, "unsupported descriptor schema");
        let memory = &descriptor["memory"];
        if memory.is_null() || memory["protocol"] != "matrix_http_v1" {
            return Ok(None);
        }
        let relay = &memory["relay"];
        if relay.is_null() {
            return Ok(None);
        }
        let text = |value: &Value, name: &str| {
            value
                .as_str()
                .map(str::to_owned)
                .with_context(|| format!("descriptor has no {name}"))
        };
        let endpoint = text(&relay["endpoint"], "relay endpoint")?;
        ensure!(
            endpoint.starts_with("http://127.0.0.1:"),
            "memory relay is not on this Machine"
        );
        let outbox = PathBuf::from(text(&relay["state_dir"], "relay outbox")?);
        ensure!(outbox.is_absolute(), "memory outbox is not absolute");
        private_directory(&outbox)?;
        // Building a client without a provider panics; this is idempotent.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let memory = Arc::new(Self {
            endpoint,
            token: text(&relay["token"], "relay token")?,
            server: text(&memory["plugin"]["id"], "memory plugin")?,
            tools: memory["enable_tools"] == true,
            recall: memory["enable_recall"] == true,
            observe: memory["observe"] == true,
            outbox,
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(EXCHANGE)
                .build()?,
            segment: Mutex::new(Segment {
                turn: new_turn(),
                ..Segment::default()
            }),
            unavailable_until: Mutex::new(None),
            delivery: tokio::sync::Mutex::new(()),
        });
        if memory.observe {
            tokio::spawn(deliver_periodically(Arc::downgrade(&memory)));
        }
        Ok(Some(memory))
    }

    /// The MCP servers to declare when opening a thread.
    #[must_use]
    pub fn mcp_servers(&self, http_supported: bool) -> Vec<McpServer> {
        if !self.tools || !http_supported {
            return Vec::new();
        }
        vec![McpServer::Http(
            McpServerHttp::new(self.server.clone(), format!("{}/mcp", self.endpoint)).headers(
                vec![HttpHeader::new(
                    "Authorization",
                    format!("Bearer {}", self.token),
                )],
            ),
        )]
    }

    fn known_down(&self) -> bool {
        self.unavailable_until
            .lock()
            .is_some_and(|until| Instant::now() < until)
    }

    /// The relay supplies the session's identity; this binding is replaced.
    async fn exchange(&self, action: &str, payload: &Value) -> Result<Value> {
        let sent = self
            .client
            .post(format!("{}/v1/{action}", self.endpoint))
            .bearer_auth(&self.token)
            .json(&json!({
                "binding": {"project": "relay", "machine": "relay", "session": "relay"},
                "payload": payload,
            }))
            .send()
            .await;
        let outcome = match sent {
            Ok(response) if response.status().is_success() => {
                response.json::<Value>().await.map_err(anyhow::Error::from)
            }
            Ok(response) if undeliverable(response.status().as_u16()) => {
                // The service answered: it is up, this request can never pass.
                *self.unavailable_until.lock() = None;
                return Err(Refused.into());
            }
            Ok(response) => Err(anyhow::anyhow!("memory answered {}", response.status())),
            Err(error) => Err(error.into()),
        };
        *self.unavailable_until.lock() = outcome.is_err().then(|| Instant::now() + BREAKER);
        outcome
    }

    /// `blocks` with this turn's memory first. Always returns within budget,
    /// and a failure leaves the turn as the user wrote it plus a short notice.
    pub async fn prepare(self: &Arc<Self>, blocks: Vec<ContentBlock>) -> Vec<ContentBlock> {
        let prompt = blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if prompt.trim().is_empty() || prompt.trim_start().starts_with('/') {
            return blocks;
        }
        let notice = self.recall.then(|| UNAVAILABLE.to_owned());
        let context = match tokio::time::timeout(BEGIN_BUDGET, self.begin(&prompt)).await {
            Ok(context) => context,
            // Whatever stalled, the next turns must not wait for it again.
            Err(_) => {
                *self.unavailable_until.lock() = Some(Instant::now() + BREAKER);
                notice
            }
        };
        let Some(context) = context.filter(|text| !text.trim().is_empty()) else {
            return blocks;
        };
        let mut prepared = Vec::with_capacity(blocks.len() + 1);
        prepared.push(ContentBlock::from(context));
        prepared.extend(blocks);
        prepared
    }

    async fn begin(self: &Arc<Self>, prompt: &str) -> Option<String> {
        self.finish();
        let evidence = {
            let mut segment = self.segment.lock();
            if self.observe {
                segment.add("user", prompt);
            }
            (!segment.events.is_empty()).then(|| {
                json!({
                    "turn": format!("{}-user", segment.turn),
                    "events": segment.events,
                    "learn": false,
                })
            })
        };
        if !self.recall && !self.tools {
            return None;
        }
        let notice = self.recall.then(|| UNAVAILABLE.to_owned());
        if self.known_down() {
            if let Some(evidence) = evidence {
                self.enqueue(evidence);
            }
            return notice;
        }
        let receipt = match &evidence {
            Some(evidence) => match self.exchange("observe", evidence).await {
                Ok(receipt) => Some(receipt),
                Err(_) => {
                    self.enqueue(evidence.clone());
                    return notice;
                }
            },
            None => None,
        };
        let mut context = String::new();
        if self.recall {
            let query = json!({"query": public_text(prompt), "budget": CONTEXT_BUDGET});
            let found = self.exchange("context", &query).await.ok()?;
            context.push_str(found["text"].as_str()?);
        }
        if self.tools
            && let Some(events) = receipt
                .as_ref()
                .and_then(|receipt| receipt["events"].as_array())
        {
            let cited = events
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            context.push_str(&format!(
                "\nCurrent user evidence for memory_put: {cited}. Cite an exact supporting excerpt; automatic learning also runs after the turn.\n"
            ));
        }
        Some(context)
    }

    /// Learn from one session update of the running turn.
    pub fn observe_update(&self, update: &SessionUpdate) {
        if !self.observe {
            return;
        }
        let mut segment = self.segment.lock();
        match update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                if let ContentBlock::Text(text) = &chunk.content
                    && segment.assistant.len() < MAX_SEGMENT_BYTES
                {
                    segment.assistant.push_str(&text.text);
                }
            }
            SessionUpdate::ToolCallUpdate(update) => {
                if matches!(
                    update.fields.status,
                    Some(ToolCallStatus::Completed | ToolCallStatus::Failed)
                ) {
                    segment.close_message();
                    let event = json!({
                        "tool": update.fields.title,
                        "status": update.fields.status,
                        "output": update.fields.raw_output,
                    });
                    segment.add("tool", &event.to_string());
                }
            }
            _ => {}
        }
    }

    /// The turn ended: keep its segment for delivery. Never waits for either.
    pub fn finish(self: &Arc<Self>) {
        let payload = {
            let mut segment = self.segment.lock();
            segment.close_message();
            let events = std::mem::take(&mut segment.events);
            let turn = std::mem::replace(&mut segment.turn, new_turn());
            segment.bytes = 0;
            (!events.is_empty()).then(|| json!({"turn": turn, "events": events}))
        };
        let memory = Arc::clone(self);
        tokio::spawn(async move {
            if let Some(payload) = payload {
                memory.store(payload).await;
            }
            memory.deliver().await;
        });
    }

    fn enqueue(self: &Arc<Self>, payload: Value) {
        let memory = Arc::clone(self);
        tokio::spawn(async move { memory.store(payload).await });
    }

    async fn store(&self, payload: Value) {
        let outbox = self.outbox.clone();
        let stored = tokio::task::spawn_blocking(move || store(outbox, &payload)).await;
        match stored {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(error = %format!("{error:#}"), "memory evidence was not kept");
            }
            Err(error) => tracing::warn!(%error, "memory evidence was not kept"),
        }
    }

    /// One delivery pass per process. Delivery also probes: its success closes
    /// the breaker.
    async fn deliver(&self) {
        let Ok(_delivery) = self.delivery.try_lock() else {
            return;
        };
        let outbox = self.outbox.clone();
        let Ok(Ok(paths)) = tokio::task::spawn_blocking(move || queued(&outbox)).await else {
            return;
        };
        for path in paths.into_iter().take(8) {
            let source = path.clone();
            let Ok(read) = tokio::task::spawn_blocking(move || std::fs::read(source)).await else {
                return;
            };
            let payload = read
                .map_err(anyhow::Error::from)
                .and_then(|bytes| Ok(serde_json::from_slice::<Value>(&bytes)?));
            let remove = match payload {
                Err(_) => true,
                Ok(payload) => match self.exchange("observe", &payload).await {
                    Ok(_) => true,
                    Err(error) if error.is::<Refused>() => true,
                    Err(_) => return,
                },
            };
            if remove {
                tokio::task::spawn_blocking(move || {
                    let _ = std::fs::remove_file(path);
                });
            }
        }
    }
}

/// Authentication and rate answers may pass later; other refusals are about
/// the request itself, and retrying one would block the queue behind it.
fn undeliverable(status: u16) -> bool {
    (400..500).contains(&status) && !matches!(status, 401 | 408 | 429)
}

#[derive(Debug)]
struct Refused;

impl std::fmt::Display for Refused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("memory refused the request")
    }
}

impl std::error::Error for Refused {}

async fn deliver_periodically(memory: Weak<AcpMemory>) {
    let mut interval = tokio::time::interval(DELIVERY_INTERVAL);
    interval.tick().await;
    loop {
        interval.tick().await;
        let Some(memory) = memory.upgrade() else {
            return;
        };
        memory.deliver().await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

    use agent_client_protocol::schema::v1::{
        ContentChunk, ToolCallId, ToolCallUpdate, ToolCallUpdateFields,
    };

    use super::*;

    struct Service {
        endpoint: String,
        status: AtomicU16,
        stalled: AtomicBool,
        requests: Mutex<Vec<(String, Value)>>,
    }

    /// A minimal backend: one request per connection, read until its length.
    async fn service() -> Arc<Service> {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let service = Arc::new(Service {
            endpoint: format!("http://{}", listener.local_addr().unwrap()),
            status: AtomicU16::new(200),
            stalled: AtomicBool::new(false),
            requests: Mutex::default(),
        });
        let shared = Arc::clone(&service);
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let service = Arc::clone(&shared);
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let mut chunk = [0u8; 8192];
                    let (head, body) = loop {
                        let read = stream.read(&mut chunk).await.unwrap();
                        bytes.extend_from_slice(&chunk[..read]);
                        let text = String::from_utf8_lossy(&bytes).into_owned();
                        if let Some((head, body)) = text.split_once("\r\n\r\n") {
                            let length = head
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().unwrap())
                                })
                                .unwrap_or(0);
                            if body.len() >= length {
                                break (head.to_owned(), body.to_owned());
                            }
                        }
                    };
                    let path = head.split_whitespace().nth(1).unwrap().to_owned();
                    service
                        .requests
                        .lock()
                        .push((path.clone(), serde_json::from_str(&body).unwrap()));
                    if service.stalled.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    }
                    let status = service.status.load(Ordering::SeqCst);
                    let reply = if path.ends_with("context") {
                        json!({"text": "[Matrix memory]\nremembered"})
                    } else {
                        json!({"events": ["observation:e0"]})
                    }
                    .to_string();
                    let frame = format!(
                        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                        reply.len()
                    );
                    let _ = stream.write_all(frame.as_bytes()).await;
                });
            }
        });
        service
    }

    fn descriptor(root: &Path, endpoint: &str, tools: bool, recall: bool) -> PathBuf {
        let path = root.join("session.json");
        std::fs::write(
            &path,
            json!({"schema": 1, "memory": {
                "plugin": {"id": "matrix"},
                "protocol": "matrix_http_v1",
                "enable_tools": tools,
                "enable_recall": recall,
                "observe": true,
                "relay": {
                    "endpoint": endpoint,
                    "token": "cbm1.s.proof",
                    "state_dir": root.join("outbox"),
                },
            }})
            .to_string(),
        )
        .unwrap();
        path
    }

    fn open(root: &Path, endpoint: &str) -> Arc<AcpMemory> {
        AcpMemory::from_descriptor(&descriptor(root, endpoint, true, true))
            .unwrap()
            .unwrap()
    }

    fn text(block: &ContentBlock) -> &str {
        match block {
            ContentBlock::Text(text) => &text.text,
            _ => panic!("expected text"),
        }
    }

    fn outbox(root: &Path) -> usize {
        queued(&root.join("outbox")).unwrap().len()
    }

    /// Delivery runs in the background; wait for the outbox to drain.
    async fn settle(root: &Path) {
        for _ in 0..400 {
            let empty = outbox(root) == 0;
            let quiet =
                tokio::time::timeout(Duration::from_millis(5), std::future::pending::<()>());
            let _ = quiet.await;
            if empty && outbox(root) == 0 {
                return;
            }
        }
        panic!("the outbox did not drain");
    }

    #[test]
    fn only_packages_without_an_adapter_are_bridged() {
        for provider in [
            "grok",
            "deepseek-harness",
            "claude-deepseek",
            "codex-deepseek",
        ] {
            assert!(bridged(provider));
        }
        for provider in ["claude-code", "codex", "gemini"] {
            assert!(!bridged(provider));
            assert!(AcpMemory::open(provider).is_none());
        }
    }

    #[test]
    fn descriptors_without_a_relay_give_no_memory_and_faults_give_no_failure() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("session.json");
        for (body, memory) in [
            (json!({"schema": 1, "memory": null}), false),
            (
                json!({"schema": 1, "memory": {"protocol": "matrix_http_v1"}}),
                false,
            ),
            (json!({"schema": 1, "memory": {"protocol": "other"}}), false),
        ] {
            std::fs::write(&path, body.to_string()).unwrap();
            assert_eq!(AcpMemory::from_descriptor(&path).unwrap().is_some(), memory);
        }
        let foreign = json!({"schema": 1, "memory": {
            "protocol": "matrix_http_v1",
            "plugin": {"id": "matrix"},
            "relay": {"endpoint": "http://example.com", "token": "t", "state_dir": "/tmp/x"},
        }});
        std::fs::write(&path, foreign.to_string()).unwrap();
        assert!(AcpMemory::from_descriptor(&path).is_err());
    }

    #[tokio::test]
    async fn a_turn_carries_recall_and_its_observation_is_delivered() {
        let root = tempfile::tempdir().unwrap();
        let service = service().await;
        let memory = open(root.path(), &service.endpoint);
        let prepared = memory
            .prepare(vec![ContentBlock::from("what did we decide?")])
            .await;
        assert_eq!(prepared.len(), 2);
        assert!(text(&prepared[0]).contains("remembered"));
        assert!(text(&prepared[0]).contains("observation:e0"));
        assert_eq!(text(&prepared[1]), "what did we decide?");

        for chunk in ["the ", "answer"] {
            memory.observe_update(&SessionUpdate::AgentMessageChunk(ContentChunk::new(
                ContentBlock::from(chunk),
            )));
        }
        memory.observe_update(&SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            ToolCallId::new("t1"),
            ToolCallUpdateFields::new()
                .title("Read file".to_owned())
                .status(ToolCallStatus::Completed),
        )));
        memory.finish();
        let delivered = || {
            service.requests.lock().iter().any(|(_, body)| {
                body["payload"]["events"]
                    .as_array()
                    .is_some_and(|events| events.len() == 3)
            })
        };
        for _ in 0..400 {
            if delivered() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        settle(root.path()).await;
        let requests = service.requests.lock();
        let last = &requests.last().unwrap().1["payload"];
        assert_eq!(last["events"][1]["role"], "assistant");
        assert_eq!(last["events"][1]["text"], "the answer");
        assert_eq!(last["events"][2]["role"], "tool");
    }

    #[tokio::test]
    async fn an_outage_keeps_the_turn_and_its_evidence_and_recovers() {
        let root = tempfile::tempdir().unwrap();
        let service = service().await;
        let memory = open(root.path(), &service.endpoint);
        service.status.store(503, Ordering::SeqCst);
        let prepared = memory.prepare(vec![ContentBlock::from("one")]).await;
        assert!(text(&prepared[0]).contains("unavailable"));
        assert_eq!(text(&prepared[1]), "one");
        assert!(memory.known_down());
        // A known outage asks nothing and keeps the evidence.
        let asked = service.requests.lock().len();
        memory.prepare(vec![ContentBlock::from("two")]).await;
        assert_eq!(service.requests.lock().len(), asked);
        for _ in 0..200 {
            if outbox(root.path()) >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(outbox(root.path()) >= 2);
        // Delivery probes, closes the breaker and drains the queue.
        service.status.store(200, Ordering::SeqCst);
        memory.deliver().await;
        assert!(!memory.known_down());
        settle(root.path()).await;
    }

    #[test]
    fn only_refusals_about_the_request_are_dropped() {
        for status in [400, 403, 404, 413] {
            assert!(undeliverable(status), "{status}");
        }
        for status in [200, 401, 408, 429, 500, 503] {
            assert!(!undeliverable(status), "{status}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_service_holds_a_turn_for_a_bounded_time() {
        let root = tempfile::tempdir().unwrap();
        let service = service().await;
        let memory = open(root.path(), &service.endpoint);
        service.stalled.store(true, Ordering::SeqCst);
        let prepared = tokio::time::timeout(
            BEGIN_BUDGET + Duration::from_secs(1),
            memory.prepare(vec![ContentBlock::from("hello")]),
        )
        .await
        .expect("bounded");
        assert_eq!(text(prepared.last().unwrap()), "hello");
        assert!(memory.known_down());
    }

    #[tokio::test]
    async fn a_dead_relay_and_an_unwritable_outbox_are_survivable() {
        let root = tempfile::tempdir().unwrap();
        let memory = open(root.path(), "http://127.0.0.1:9");
        std::fs::remove_dir_all(root.path().join("outbox")).unwrap();
        let prepared = memory.prepare(vec![ContentBlock::from("hello")]).await;
        assert_eq!(text(prepared.last().unwrap()), "hello");
        memory.finish();
        memory.deliver().await;
    }

    #[tokio::test]
    async fn commands_and_empty_prompts_are_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let service = service().await;
        let memory = open(root.path(), &service.endpoint);
        for prompt in ["/compact", "   "] {
            assert_eq!(
                memory.prepare(vec![ContentBlock::from(prompt)]).await.len(),
                1
            );
        }
        assert!(service.requests.lock().is_empty());
    }

    #[tokio::test]
    async fn tools_follow_the_switch_and_the_agent_capability() {
        let root = tempfile::tempdir().unwrap();
        let on =
            AcpMemory::from_descriptor(&descriptor(root.path(), "http://127.0.0.1:1", true, false))
                .unwrap()
                .unwrap();
        assert_eq!(on.mcp_servers(true).len(), 1);
        assert!(on.mcp_servers(false).is_empty());
        let off =
            AcpMemory::from_descriptor(&descriptor(root.path(), "http://127.0.0.1:1", false, true))
                .unwrap()
                .unwrap();
        assert!(off.mcp_servers(true).is_empty());
    }

    #[test]
    fn credentials_never_leave_the_session() {
        assert_eq!(public_text("api_key=abcdef123456"), "[redacted credential]");
        assert!(!public_text("Bearer abcdefghijklmnop").contains("abcdefghijklmnop"));
        let mut segment = Segment::default();
        segment.add("assistant", "[Matrix memory]\nrecalled");
        assert!(segment.events.is_empty());
    }
}
