//! Product-neutral External Intake HTTP API (`POST /api/v1/intake/dispatch`,
//! `GET /api/v1/intake/dispatches/{id}`).
//!
//! Authenticated strictly by a host-configured Bearer token file
//! (`--intake-token-file`), fail-closed when unconfigured, backed by a bounded
//! in-memory idempotency ledger, and capable of emitting structured turn-end
//! verdicts over an optional host-configured Unix socket.

use super::*;
use axum::http::HeaderMap;
use serde_json::{Value, json};
use sha2::Sha256;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::Duration;

const MAX_DISPATCHES: usize = 256;
const MIN_TOKEN_BYTES: usize = 32;
const DEFAULT_TIMEOUT_SECS: u64 = 240;

#[derive(Clone, Debug)]
pub struct Config {
    pub token_sha256: Option<[u8; 32]>,
    pub verdict_socket: Option<PathBuf>,
    pub provider: String,
    pub machine: String,
    pub workspace: PathBuf,
    pub public_url: Option<String>,
    pub timeout: Duration,
}

impl Config {
    pub fn from_serve_args(args: &crate::serve_args::ServeArgs) -> anyhow::Result<Self> {
        let token_sha256 = match &args.intake_token_file {
            Some(path) => {
                let raw = std::fs::read_to_string(path)
                    .with_context(|| format!("reading intake token file {}", path.display()))?;
                let trimmed = raw.trim();
                anyhow::ensure!(
                    trimmed.len() >= MIN_TOKEN_BYTES,
                    "intake token must be at least {MIN_TOKEN_BYTES} bytes"
                );
                Some(sha256_bytes(trimmed.as_bytes()))
            }
            None => None,
        };
        Ok(Self {
            token_sha256,
            verdict_socket: args.intake_verdict_socket.clone(),
            provider: args
                .intake_provider
                .clone()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "codex".to_owned()),
            machine: args
                .intake_machine
                .clone()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "local".to_owned()),
            workspace: args
                .intake_workspace
                .clone()
                .unwrap_or_else(|| args.workspace_root.clone()),
            public_url: args
                .intake_public_url
                .clone()
                .map(|u| u.trim_end_matches('/').to_owned())
                .filter(|u| u.starts_with("https://") || u.starts_with("http://")),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        })
    }

    pub fn enabled(&self) -> bool {
        self.token_sha256.is_some()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DispatchRequest {
    pub kind: String,
    pub episode_id: String,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub episode_key: Option<String>,
    pub source: String,
    #[serde(default)]
    pub machine: Option<String>,
    #[serde(default)]
    pub severity: Option<String>,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub target_url: Option<String>,
    #[serde(default)]
    pub evidence_ref: Option<serde_json::Value>,
    #[serde(default)]
    pub labels: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerdictPayload {
    pub version: u8,
    pub episode_id: String,
    pub dispatch_id: String,
    pub verdict: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources_checked: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions_taken: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_next_step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_url: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DispatchRecord {
    pub dispatch_id: String,
    pub idempotency_key: String,
    pub episode_id: String,
    pub kind: String,
    pub source: String,
    pub session_id: String,
    pub session_url: Option<String>,
    pub status: String,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub verdict: Option<VerdictPayload>,
    pub error: Option<String>,
}

#[derive(Default)]
struct IntakeLedger {
    order: VecDeque<String>,
    by_dispatch_id: HashMap<String, DispatchRecord>,
    by_idempotency: HashMap<String, String>,
}

impl IntakeLedger {
    fn get_by_idempotency(&self, key: &str) -> Option<DispatchRecord> {
        let id = self.by_idempotency.get(key)?;
        self.by_dispatch_id.get(id).cloned()
    }

    fn get(&self, id_or_episode: &str) -> Option<DispatchRecord> {
        if let Some(rec) = self.by_dispatch_id.get(id_or_episode) {
            return Some(rec.clone());
        }
        let mapped = self.by_idempotency.get(id_or_episode)?;
        self.by_dispatch_id.get(mapped).cloned()
    }

    fn insert(&mut self, record: DispatchRecord) {
        if self.order.len() >= MAX_DISPATCHES
            && let Some(oldest) = self.order.pop_front()
            && let Some(removed) = self.by_dispatch_id.remove(&oldest)
        {
            self.by_idempotency.remove(&removed.idempotency_key);
        }
        self.order.push_back(record.dispatch_id.clone());
        self.by_idempotency
            .insert(record.idempotency_key.clone(), record.dispatch_id.clone());
        self.by_dispatch_id
            .insert(record.dispatch_id.clone(), record);
    }

    fn complete(
        &mut self,
        dispatch_id: &str,
        status: &str,
        verdict: Option<VerdictPayload>,
        error: Option<String>,
    ) {
        if let Some(rec) = self.by_dispatch_id.get_mut(dispatch_id) {
            rec.status = status.to_owned();
            rec.completed_at = Some(chrono::Utc::now().to_rfc3339());
            rec.verdict = verdict;
            rec.error = error;
        }
    }
}

pub struct Service {
    state: Arc<AppState>,
    config: Config,
    ledger: parking_lot::Mutex<IntakeLedger>,
}

impl Service {
    pub fn new(state: Arc<AppState>, config: Config) -> Arc<Self> {
        Arc::new(Self {
            state,
            config,
            ledger: parking_lot::Mutex::new(IntakeLedger::default()),
        })
    }

    fn authorize(&self, headers: &HeaderMap) -> Result<(), (StatusCode, &'static str)> {
        let Some(expected) = self.config.token_sha256 else {
            return Err((StatusCode::SERVICE_UNAVAILABLE, "intake_not_configured"));
        };
        let Some(presented) = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|v| v.len() >= MIN_TOKEN_BYTES)
        else {
            return Err((StatusCode::UNAUTHORIZED, "unauthorized"));
        };
        let actual = sha256_bytes(presented.as_bytes());
        if !constant_time_eq_32(&expected, &actual) {
            return Err((StatusCode::UNAUTHORIZED, "unauthorized"));
        }
        Ok(())
    }
}

pub fn routes(service: Arc<Service>) -> Router {
    Router::new()
        .route(
            "/api/v1/intake/dispatch",
            post(dispatch).layer(DefaultBodyLimit::max(64 * 1024)),
        )
        .route("/api/v1/intake/dispatches/{id}", get(inspect))
        .with_state(service)
}

fn validate_request(req: &DispatchRequest) -> Result<(), &'static str> {
    if req.kind.trim().is_empty() || req.kind.len() > 128 {
        return Err("invalid_kind");
    }
    if req.episode_id.trim().is_empty() || req.episode_id.len() > 128 {
        return Err("invalid_episode_id");
    }
    if req.source.trim().is_empty() || req.source.len() > 128 {
        return Err("invalid_source");
    }
    if req.title.trim().is_empty() || req.title.len() > 256 {
        return Err("invalid_title");
    }
    if req.body.trim().is_empty() || req.body.len() > 8192 {
        return Err("invalid_body");
    }
    Ok(())
}

async fn dispatch(
    State(service): State<Arc<Service>>,
    headers: HeaderMap,
    Json(req): Json<DispatchRequest>,
) -> Response {
    if let Err((status, code)) = service.authorize(&headers) {
        return (status, Json(json!({ "error": code }))).into_response();
    }
    if let Err(code) = validate_request(&req) {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": code }))).into_response();
    }
    let idem_key = req
        .idempotency_key
        .as_deref()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(&req.episode_id)
        .to_owned();
    if let Some(existing) = service.ledger.lock().get_by_idempotency(&idem_key) {
        return (
            StatusCode::OK,
            Json(json!({ "schema": 1, "deduplicated": true, "dispatch": existing })),
        )
            .into_response();
    }

    let dispatch_id = format!("intk_{}", uuid::Uuid::new_v4().simple());
    let session_id = service.state.supervisor.reserve_session_id();
    let session_url = service
        .config
        .public_url
        .as_ref()
        .map(|base| format!("{base}/sessions/{session_id}"));
    let cwd = Some(service.config.workspace.to_string_lossy().into_owned());

    if let Err(err) = service.state.supervisor.new_session_on_with_id(
        &session_id,
        &service.config.provider,
        cwd,
        crate::core::SessionOrigin::Api,
        true,
        &service.config.machine,
        crate::supervisor::ProviderGeneration {
            account_id: None,
            version: "",
            digest: "",
            auth_generation: None,
            behavior: None,
        },
        None,
    ) {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "session_start_failed", "detail": err })),
        )
            .into_response();
    }

    let prompt = build_triage_prompt(&req);
    let cmid = format!("intake:{dispatch_id}");
    if let Err(err) = service.state.supervisor.send(
        &session_id,
        AgentCommand::Prompt(vec![ContentBlock::from(prompt)], Some(cmid), None),
    ) {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "prompt_dispatch_failed", "detail": err })),
        )
            .into_response();
    }

    let record = DispatchRecord {
        dispatch_id: dispatch_id.clone(),
        idempotency_key: idem_key,
        episode_id: req.episode_id.clone(),
        kind: req.kind.clone(),
        source: req.source.clone(),
        session_id: session_id.clone(),
        session_url: session_url.clone(),
        status: "running".to_owned(),
        created_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        verdict: None,
        error: None,
    };
    service.ledger.lock().insert(record.clone());

    let observer = Arc::clone(&service);
    let episode_id = req.episode_id;
    tokio::spawn(async move {
        observe_and_emit(observer, dispatch_id, episode_id, session_id, session_url).await;
    });

    (
        StatusCode::ACCEPTED,
        Json(json!({ "schema": 1, "deduplicated": false, "dispatch": record })),
    )
        .into_response()
}

async fn inspect(
    State(service): State<Arc<Service>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err((status, code)) = service.authorize(&headers) {
        return (status, Json(json!({ "error": code }))).into_response();
    }
    match service.ledger.lock().get(&id) {
        Some(record) => (
            StatusCode::OK,
            Json(json!({ "schema": 1, "dispatch": record })),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response(),
    }
}

async fn observe_and_emit(
    service: Arc<Service>,
    dispatch_id: String,
    episode_id: String,
    session_id: String,
    session_url: Option<String>,
) {
    let mut events = service.state.hub.subscribe();
    let deadline = tokio::time::Instant::now() + service.config.timeout;
    let outcome = loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(frame)) => match frame.outbound() {
                Outbound::Event { envelope }
                    if envelope.session_id == session_id
                        && matches!(envelope.event, crate::core::Event::TurnEnd { .. }) =>
                {
                    break Ok(extract_assistant_text(&service.state.hub, &session_id));
                }
                Outbound::SessionUpdated { session }
                    if session.id == session_id
                        && matches!(
                            session.status,
                            crate::core::Status::Crashed | crate::core::Status::Exited
                        ) =>
                {
                    break Err("session_terminated_before_turn_end".to_owned());
                }
                _ => {}
            },
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                break Err("hub_closed".to_owned());
            }
            Err(_) => break Err("intake_triage_timeout".to_owned()),
        }
    };

    let (status, verdict, err_msg) = match outcome {
        Ok(text) => {
            let v = parse_verdict_from_text(&text, &episode_id, &dispatch_id, session_url);
            ("completed", v, None)
        }
        Err(reason) => {
            let v = fallback_verdict(
                &episode_id,
                &dispatch_id,
                format!("Automated triage did not complete cleanly ({reason})."),
                session_url,
            );
            ("failed", v, Some(reason))
        }
    };

    service
        .ledger
        .lock()
        .complete(&dispatch_id, status, Some(verdict.clone()), err_msg);

    if let Some(socket) = &service.config.verdict_socket
        && let Err(error) = deliver_verdict_unix(socket, &verdict).await
    {
        tracing::warn!(%dispatch_id, %error, "external intake verdict socket delivery failed");
    }
}

fn extract_assistant_text(hub: &crate::core::Hub, session_id: &str) -> String {
    let Some((log, _)) = hub.snapshot(session_id) else {
        return String::new();
    };
    let mut out = String::new();
    for env in &log {
        if let crate::core::Event::Update { update } = &env.event
            && update["sessionUpdate"].as_str() == Some("agent_message_chunk")
            && let Some(text) = update.pointer("/content/text").and_then(Value::as_str)
        {
            out.push_str(text);
        }
    }
    out
}

pub fn build_triage_prompt(req: &DispatchRequest) -> String {
    let mut sections = vec![
        format!("[External Intake: {}]", req.kind),
        format!("Source: {}", req.source),
        format!("Episode ID: {}", req.episode_id),
        format!("Title: {}", req.title),
    ];
    if let Some(m) = &req.machine {
        sections.push(format!("Machine: {m}"));
    }
    if let Some(sev) = &req.severity {
        sections.push(format!("Severity: {sev}"));
    }
    if let Some(url) = &req.target_url {
        sections.push(format!("Target URL: {url}"));
    }
    sections.push(format!("\nSignal Body:\n{}", req.body));
    if let Some(ev) = &req.evidence_ref {
        sections.push(format!("\nEvidence Reference:\n{ev}"));
    }
    sections.push(
        "\nInstructions:\n\
         1. Inspect the referenced evidence using read-only diagnostics first.\n\
         2. Do not run destructive mutations without explicit human approval.\n\
         3. End your response with a single fenced ```json block matching:\n\
         {\"verdict\":\"resolved_by_ai|suppress_noise|escalate_human|needs_approval\",\"summary\":\"...\",\"sources_checked\":[],\"actions_taken\":[],\"recommended_next_step\":null}"
            .to_owned(),
    );
    sections.join("\n")
}

#[derive(Deserialize)]
struct RawModelVerdict {
    verdict: String,
    summary: String,
    #[serde(default)]
    sources_checked: Vec<String>,
    #[serde(default)]
    actions_taken: Vec<String>,
    #[serde(default)]
    recommended_next_step: Option<String>,
}

pub fn parse_verdict_from_text(
    text: &str,
    episode_id: &str,
    dispatch_id: &str,
    session_url: Option<String>,
) -> VerdictPayload {
    if let Some(json_block) = extract_last_json_fence(text)
        && let Ok(parsed) = serde_json::from_str::<RawModelVerdict>(json_block)
        && matches!(
            parsed.verdict.as_str(),
            "resolved_by_ai" | "suppress_noise" | "escalate_human" | "needs_approval"
        )
        && !parsed.summary.trim().is_empty()
    {
        return VerdictPayload {
            version: 1,
            episode_id: episode_id.to_owned(),
            dispatch_id: dispatch_id.to_owned(),
            verdict: parsed.verdict,
            summary: parsed.summary.chars().take(2000).collect(),
            sources_checked: parsed.sources_checked,
            actions_taken: parsed.actions_taken,
            recommended_next_step: parsed.recommended_next_step,
            session_url,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
    }
    let trimmed = text.trim();
    let summary = if trimmed.is_empty() {
        "Triage agent finished without a structured verdict; escalating to operator.".to_owned()
    } else {
        trimmed.chars().take(1500).collect()
    };
    fallback_verdict(episode_id, dispatch_id, summary, session_url)
}

fn fallback_verdict(
    episode_id: &str,
    dispatch_id: &str,
    summary: String,
    session_url: Option<String>,
) -> VerdictPayload {
    VerdictPayload {
        version: 1,
        episode_id: episode_id.to_owned(),
        dispatch_id: dispatch_id.to_owned(),
        verdict: "escalate_human".to_owned(),
        summary,
        sources_checked: Vec::new(),
        actions_taken: Vec::new(),
        recommended_next_step: Some("Review triage session transcript.".to_owned()),
        session_url,
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn extract_last_json_fence(text: &str) -> Option<&str> {
    let start = text.rfind("```json")?;
    let after = &text[start + "```json".len()..];
    let body = after
        .strip_prefix('\n')
        .or_else(|| after.strip_prefix("\r\n"))?;
    let end = body.find("```")?;
    Some(body[..end].trim())
}

#[cfg(unix)]
async fn deliver_verdict_unix(
    path: &std::path::Path,
    verdict: &VerdictPayload,
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt as _;
    let mut stream = tokio::net::UnixStream::connect(path).await?;
    let line = format!("{}\n", serde_json::json!({ "verdict": verdict }));
    stream.write_all(line.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(not(unix))]
async fn deliver_verdict_unix(
    _path: &std::path::Path,
    _verdict: &VerdictPayload,
) -> anyhow::Result<()> {
    Ok(())
}

fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

fn constant_time_eq_32(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0_u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_structured_verdict_fence_and_falls_back_safely() {
        let reply = "Investigated disk usage.\n```json\n{\"verdict\":\"resolved_by_ai\",\"summary\":\"Cleared stale cache.\",\"sources_checked\":[\"journald\"],\"actions_taken\":[\"vacuumed logs\"]}\n```\n";
        let v = parse_verdict_from_text(
            reply,
            "ep-1",
            "intk_1",
            Some("https://cowboy.example/sessions/s1".into()),
        );
        assert_eq!(v.verdict, "resolved_by_ai");
        assert_eq!(v.summary, "Cleared stale cache.");
        assert_eq!(v.sources_checked, vec!["journald"]);
        assert_eq!(
            v.session_url.as_deref(),
            Some("https://cowboy.example/sessions/s1")
        );

        let unstructured =
            parse_verdict_from_text("Could not determine cause", "ep-2", "intk_2", None);
        assert_eq!(unstructured.verdict, "escalate_human");
        assert_eq!(unstructured.summary, "Could not determine cause");
    }

    #[test]
    fn ledger_deduplicates_and_bounds_capacity() {
        let mut ledger = IntakeLedger::default();
        for i in 0..=MAX_DISPATCHES {
            ledger.insert(DispatchRecord {
                dispatch_id: format!("d-{i}"),
                idempotency_key: format!("k-{i}"),
                episode_id: format!("e-{i}"),
                kind: "triage".into(),
                source: "probe".into(),
                session_id: format!("s-{i}"),
                session_url: None,
                status: "running".into(),
                created_at: "2026-11-28T00:00:00Z".into(),
                completed_at: None,
                verdict: None,
                error: None,
            });
        }
        assert_eq!(ledger.by_dispatch_id.len(), MAX_DISPATCHES);
        assert!(ledger.get("d-0").is_none());
        assert!(ledger.get("k-256").is_some());
    }

    #[test]
    fn constant_time_token_comparison_checks_all_bytes() {
        let a = sha256_bytes(b"token-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let b = sha256_bytes(b"token-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let c = sha256_bytes(b"token-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        assert!(constant_time_eq_32(&a, &b));
        assert!(!constant_time_eq_32(&a, &c));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn delivers_verdict_json_line_over_unix_socket() {
        use tokio::io::AsyncBufReadExt as _;
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("verdict.sock");
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        let verdict = fallback_verdict("ep-9", "intk_9", "Needs operator".into(), None);
        let send_task = tokio::spawn({
            let sock = sock.clone();
            let verdict = verdict.clone();
            async move { deliver_verdict_unix(&sock, &verdict).await }
        });
        let (stream, _) = listener.accept().await.unwrap();
        let mut line = String::new();
        tokio::io::BufReader::new(stream)
            .read_line(&mut line)
            .await
            .unwrap();
        send_task.await.unwrap().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["verdict"]["episode_id"], "ep-9");
        assert_eq!(parsed["verdict"]["verdict"], "escalate_human");
    }
}
