//! Stdio ACP server face backed by the already-running cowboy daemon.
//!
//! This module is intentionally a transport bridge, not a second runtime: the
//! daemon remains the sole owner of the Hub, persistence, agent subprocesses,
//! prompt serialization, and global event ordering. Zed launches one bridge
//! process per configured provider and the bridge talks to the daemon over its
//! public HTTP/WebSocket API.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, CloseSessionRequest, CloseSessionResponse,
    ConfigOptionUpdate, DeleteSessionRequest, DeleteSessionResponse, Implementation,
    InitializeRequest, InitializeResponse, ListSessionsRequest, ListSessionsResponse,
    LoadSessionRequest, LoadSessionResponse, NewSessionRequest, NewSessionResponse,
    PromptCapabilities, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, SessionCapabilities, SessionCloseCapabilities, SessionConfigOption,
    SessionDeleteCapabilities, SessionInfo, SessionInfoUpdate, SessionListCapabilities,
    SessionNotification, SessionUpdate, SetSessionConfigOptionRequest,
    SetSessionConfigOptionResponse, StopReason,
};
use agent_client_protocol::{
    Agent, Client, ConnectionTo, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, Stdio,
};
use anyhow::{Context as _, anyhow, bail};
use futures::{SinkExt as _, StreamExt as _};
use parking_lot::Mutex;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

use crate::acp::STARTUP_PHASE_TIMEOUT;
use crate::cli::ServeAcpArgs;
use crate::core::{Envelope, Event, Inbound, Outbound, SessionMeta, Status};

const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);
const CONFIG_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
// The daemon's POST /api/sessions returns before the provider completes its ACP
// handshake. Zed decides whether to construct ConfigOptionsView from the
// session/new response; a later config_option_update can refresh an existing
// view, but cannot create one when the response contained no config options.
// Therefore this wait must outlive one provider startup phase. A cold Codex
// start has been observed taking 23s, well beyond the old 10s timeout.
const INITIAL_CONFIG_TIMEOUT: Duration = Duration::from_secs(STARTUP_PHASE_TIMEOUT.as_secs() + 5);
const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(30);
const RECONNECT_INITIAL_DELAY: Duration = Duration::from_millis(100);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(5);
const HISTORY_PAGE_LIMIT: usize = 10_000;
const EVENT_CACHE_LIMIT: usize = 1_000;
const STATUS_EXTENSION_VERSION: u32 = 1;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, Clone, Serialize, Deserialize, JsonRpcRequest)]
#[request(method = "_cowboy/session/status", response = CowboyStatus)]
#[serde(rename_all = "camelCase")]
struct CowboyStatusRequest {
    session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonRpcResponse)]
#[serde(rename_all = "camelCase")]
struct CowboyStatus {
    session_id: String,
    provider: String,
    state: String,
    /// Authoritative: true exactly while the Hub has an in-flight prompt turn.
    turn_running: bool,
    /// The provider subprocess/ACP session is currently usable or starting.
    agent_alive: bool,
    /// Monotonic Hub event sequence that produced this snapshot, when known.
    seq: Option<u64>,
    detail: Option<String>,
    /// `None` means the selected provider adapter does not yet expose a complete
    /// background-resource snapshot. Never turn an unknown into a false idle.
    background_running: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonRpcNotification)]
#[notification(method = "_cowboy/session/status_changed")]
struct CowboyStatusNotification {
    #[serde(flatten)]
    status: CowboyStatus,
}

#[derive(Debug, Deserialize)]
struct NewSessionHttpResponse {
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct HistoryResponse {
    events: Vec<Envelope>,
    next_before_seq: Option<u64>,
    reached_start: bool,
}

#[derive(Default)]
struct CachedSession {
    meta: Option<SessionMeta>,
    events: Vec<Envelope>,
    reached_start: bool,
    attached: bool,
    replaying: bool,
    replay_buffer: Vec<Envelope>,
    last_seq: Option<u64>,
    last_detail: Option<String>,
}

struct PendingPrompt {
    cmid: String,
    started: bool,
    cancel_requested: bool,
    completion: oneshot::Sender<PromptOutcome>,
}

enum PromptOutcome {
    Stopped(StopReason),
    Failed(String),
}

enum ConnectedExit {
    Disconnected(Option<Inbound>),
    CommandsClosed,
}

#[derive(Default)]
struct BridgeState {
    connected: bool,
    sessions: HashMap<String, CachedSession>,
    pending_prompts: HashMap<String, VecDeque<PendingPrompt>>,
    config_options: HashMap<String, Vec<SessionConfigOption>>,
    config_waiters: HashMap<String, Vec<oneshot::Sender<Vec<SessionConfigOption>>>>,
}

impl BridgeState {
    fn detach_session(&mut self, session_id: &str) {
        if let Some(cached) = self.sessions.get_mut(session_id) {
            cached.attached = false;
            cached.replaying = false;
            cached.replay_buffer.clear();
        }
        self.config_waiters.remove(session_id);
    }
}

#[derive(Clone)]
struct Bridge {
    provider: Arc<str>,
    base_url: Url,
    http: reqwest::Client,
    state: Arc<Mutex<BridgeState>>,
    connected_notify: Arc<Notify>,
    command_tx: mpsc::UnboundedSender<Inbound>,
    next_cmid: Arc<AtomicU64>,
}

pub async fn serve(args: ServeAcpArgs) -> anyhow::Result<()> {
    if crate::provider::lookup(&args.provider).is_none() {
        bail!("unknown provider {:?}", args.provider);
    }

    let base_url = normalized_base_url(&args.daemon_url)?;
    let state = Arc::new(Mutex::new(BridgeState::default()));

    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let bridge = Bridge {
        provider: Arc::from(args.provider),
        base_url,
        http: reqwest::Client::new(),
        state,
        connected_notify: Arc::new(Notify::new()),
        command_tx,
        next_cmid: Arc::new(AtomicU64::new(1)),
    };

    tracing::info!(
        provider = %bridge.provider,
        daemon = %bridge.base_url,
        "cowboy ACP bridge ready; daemon connection runs independently"
    );

    run_acp_server(bridge, command_rx)
        .await
        .map_err(|error| anyhow!("ACP bridge: {error}"))
}

#[allow(clippy::too_many_lines)] // Declarative ACP handler registration is clearest in one chain.
async fn run_acp_server(
    bridge: Bridge,
    command_rx: mpsc::UnboundedReceiver<Inbound>,
) -> Result<(), agent_client_protocol::Error> {
    let initialize_bridge = bridge.clone();
    let list_bridge = bridge.clone();
    let new_bridge = bridge.clone();
    let load_bridge = bridge.clone();
    let close_bridge = bridge.clone();
    let delete_bridge = bridge.clone();
    let prompt_bridge = bridge.clone();
    let cancel_bridge = bridge.clone();
    let config_bridge = bridge.clone();
    let status_bridge = bridge.clone();
    let main_bridge = bridge;

    Agent
        .builder()
        .name("cowboy-serve-acp")
        .on_receive_request(
            async move |request: InitializeRequest, responder, _cx: ConnectionTo<Client>| {
                let capabilities = AgentCapabilities::new()
                    .load_session(true)
                    .prompt_capabilities(
                        PromptCapabilities::new().image(true).embedded_context(true),
                    )
                    .session_capabilities(
                        SessionCapabilities::new()
                            .list(SessionListCapabilities::new())
                            .delete(SessionDeleteCapabilities::new())
                            .close(SessionCloseCapabilities::new()),
                    )
                    .meta(initialize_bridge.capability_meta());
                let info = Implementation::new("cowboy", env!("CARGO_PKG_VERSION"))
                    .title(initialize_bridge.display_name());
                responder.respond(
                    InitializeResponse::new(request.protocol_version)
                        .agent_capabilities(capabilities)
                        .agent_info(info)
                        .meta(initialize_bridge.capability_meta()),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: ListSessionsRequest, responder, _cx: ConnectionTo<Client>| {
                responder.respond(ListSessionsResponse::new(
                    list_bridge.list_sessions(request.cwd.as_ref()),
                ))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, cx: ConnectionTo<Client>| {
                let bridge = new_bridge.clone();
                cx.spawn(async move {
                    Bridge::warn_unsupported_context(
                        &request.additional_directories,
                        request.mcp_servers.len(),
                    );
                    match bridge.create_session(request.cwd).await {
                        Ok(session_id) => {
                            bridge.attach(&session_id, false);
                            let config_options =
                                bridge.wait_for_initial_config_options(&session_id).await;
                            responder.respond(
                                NewSessionResponse::new(session_id.clone())
                                    .config_options(config_options)
                                    .meta(bridge.session_meta(&session_id)),
                            )?;
                        }
                        Err(error) => {
                            responder.respond_with_error(
                                agent_client_protocol::util::internal_error(error),
                            )?;
                        }
                    }
                    Ok(())
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: LoadSessionRequest, responder, cx: ConnectionTo<Client>| {
                let bridge = load_bridge.clone();
                let replay_cx = cx.clone();
                cx.spawn(async move {
                    let session_id = request.session_id.0.to_string();
                    Bridge::warn_unsupported_context(
                        &request.additional_directories,
                        request.mcp_servers.len(),
                    );
                    if let Err(error) = bridge.validate_session(&session_id, Some(&request.cwd)) {
                        responder.respond_with_error(
                            agent_client_protocol::util::internal_error(error),
                        )?;
                        return Ok(());
                    }
                    bridge.attach(&session_id, true);
                    bridge.send_command(Inbound::OpenSession {
                        session_id: session_id.clone(),
                    })?;
                    match bridge.replay_session(&session_id, &replay_cx).await {
                        Ok(()) => {
                            responder.respond(
                                LoadSessionResponse::new()
                                    .config_options(bridge.config_options(&session_id))
                                    .meta(bridge.session_meta(&session_id)),
                            )?;
                        }
                        Err(error) => {
                            responder.respond_with_error(
                                agent_client_protocol::util::internal_error(error),
                            )?;
                        }
                    }
                    Ok(())
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: CloseSessionRequest, responder, _cx: ConnectionTo<Client>| {
                let session_id = request.session_id.0.to_string();
                if let Err(error) = close_bridge.validate_session(&session_id, None) {
                    return responder
                        .respond_with_error(agent_client_protocol::util::internal_error(error));
                }
                close_bridge.detach(&session_id);
                responder.respond(CloseSessionResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: DeleteSessionRequest, responder, _cx: ConnectionTo<Client>| {
                let session_id = request.session_id.0.to_string();
                if let Err(error) = delete_bridge.validate_session(&session_id, None) {
                    return responder
                        .respond_with_error(agent_client_protocol::util::internal_error(error));
                }
                delete_bridge.send_command(Inbound::DeleteSession { session_id })?;
                responder.respond(DeleteSessionResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, cx: ConnectionTo<Client>| {
                let bridge = prompt_bridge.clone();
                let session_id = request.session_id.0.to_string();
                if let Err(error) = bridge.validate_session(&session_id, None) {
                    return responder
                        .respond_with_error(agent_client_protocol::util::internal_error(error));
                }
                let (cmid, completion) = bridge.register_prompt(&session_id);
                let content = request
                    .prompt
                    .into_iter()
                    .filter_map(|block| serde_json::to_value(block).ok())
                    .collect();
                if let Err(error) = bridge.send_command(Inbound::Submit {
                    session_id: session_id.clone(),
                    text: String::new(),
                    content,
                    cmid: Some(cmid.clone()),
                    force: false,
                    front: false,
                }) {
                    bridge.fail_prompt(&session_id, &cmid, error.to_string());
                }
                let cancellation = responder.cancellation();
                cx.spawn(async move {
                    tokio::select! {
                        outcome = completion => match outcome {
                            Ok(PromptOutcome::Stopped(reason)) => {
                                responder.respond(PromptResponse::new(reason))?;
                            }
                            Ok(PromptOutcome::Failed(error)) => {
                                responder.respond_with_error(
                                    agent_client_protocol::util::internal_error(error),
                                )?;
                            }
                            Err(_) => {
                                responder.respond_with_error(
                                    agent_client_protocol::util::internal_error(
                                        "cowboy daemon disconnected before the turn finished",
                                    ),
                                )?;
                            }
                        },
                        () = cancellation.cancelled() => {
                            bridge.cancel_prompts(&session_id);
                            responder.respond_with_error(
                                agent_client_protocol::Error::request_cancelled(),
                            )?;
                        }
                    };
                    Ok(())
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |notification: CancelNotification, _cx: ConnectionTo<Client>| {
                cancel_bridge.cancel_prompts(notification.session_id.0.as_ref());
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: SetSessionConfigOptionRequest,
                        responder,
                        cx: ConnectionTo<Client>| {
                let bridge = config_bridge.clone();
                let session_id = request.session_id.0.to_string();
                if let Err(error) = bridge.validate_session(&session_id, None) {
                    return responder
                        .respond_with_error(agent_client_protocol::util::internal_error(error));
                }
                let value = request
                    .value
                    .as_value_id()
                    .map(|value| serde_json::Value::String(value.0.to_string()))
                    .or_else(|| request.value.as_bool().map(serde_json::Value::Bool))
                    .unwrap_or(serde_json::Value::Null);
                let waiter = bridge.register_config_waiter(&session_id);
                bridge.send_command(Inbound::SetConfigOption {
                    session_id: session_id.clone(),
                    config_id: request.config_id.0.to_string(),
                    value,
                })?;
                cx.spawn(async move {
                    let options = tokio::time::timeout(CONFIG_RESPONSE_TIMEOUT, waiter)
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .or_else(|| bridge.config_options(&session_id))
                        .unwrap_or_default();
                    responder.respond(SetSessionConfigOptionResponse::new(options))?;
                    Ok(())
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: CowboyStatusRequest, responder, _cx: ConnectionTo<Client>| {
                match status_bridge.status(&request.session_id) {
                    Some(status) => responder.respond(status),
                    None => {
                        responder.respond_with_error(agent_client_protocol::util::internal_error(
                            "unknown session or provider mismatch",
                        ))
                    }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(Stdio::new(), async move |cx: ConnectionTo<Client>| {
            main_bridge.run_daemon(command_rx, cx).await
        })
        .await
}

impl Bridge {
    fn display_name(&self) -> String {
        match self.provider.as_ref() {
            "codex" => "Cowboy · Codex",
            "claude-code" => "Cowboy · Claude Code",
            "gemini" => "Cowboy · Gemini",
            provider => provider,
        }
        .to_owned()
    }

    fn capability_meta(&self) -> serde_json::Map<String, serde_json::Value> {
        serde_json::json!({
            "cowboy.dev": {
                "version": STATUS_EXTENSION_VERSION,
                "sessionStatus": true,
                "provider": self.provider.as_ref(),
                "backgroundActivity": false
            }
        })
        .as_object()
        .cloned()
        .unwrap_or_default()
    }

    fn warn_unsupported_context(additional_directories: &[PathBuf], mcp_servers: usize) {
        if !additional_directories.is_empty() {
            tracing::warn!(
                count = additional_directories.len(),
                "TODO(acp-bridge): additionalDirectories are not yet forwarded to an existing cowboy session"
            );
        }
        if mcp_servers > 0 {
            tracing::warn!(
                count = mcp_servers,
                "TODO(acp-bridge): client-provided MCP servers are not yet forwarded through the daemon"
            );
        }
    }

    fn list_sessions(&self, cwd: Option<&PathBuf>) -> Vec<SessionInfo> {
        let state = self.state.lock();
        let mut sessions = state
            .sessions
            .values()
            .filter_map(|cached| cached.meta.as_ref())
            .filter(|meta| meta.provider == self.provider.as_ref())
            .filter(|meta| cwd.is_none_or(|cwd| std::path::Path::new(&meta.cwd) == cwd))
            .map(|meta| {
                SessionInfo::new(meta.id.clone(), PathBuf::from(&meta.cwd))
                    .title(meta.title.clone())
                    .meta(status_meta(&Self::status_from_cached(
                        meta,
                        state.sessions.get(&meta.id),
                    )))
            })
            .collect::<Vec<_>>();
        sessions.sort_unstable_by(|a, b| a.session_id.0.cmp(&b.session_id.0));
        sessions
    }

    fn validate_session(&self, session_id: &str, cwd: Option<&PathBuf>) -> Result<(), String> {
        let state = self.state.lock();
        let meta = state
            .sessions
            .get(session_id)
            .and_then(|cached| cached.meta.as_ref())
            .ok_or_else(|| format!("unknown cowboy session {session_id:?}"))?;
        if meta.provider != self.provider.as_ref() {
            return Err(format!(
                "session {session_id:?} belongs to provider {:?}, not {:?}",
                meta.provider, self.provider
            ));
        }
        if let Some(cwd) = cwd
            && std::path::Path::new(&meta.cwd) != cwd
        {
            return Err(format!(
                "session {session_id:?} cwd mismatch: stored {}, requested {}",
                meta.cwd,
                cwd.display()
            ));
        }
        Ok(())
    }

    fn attach(&self, session_id: &str, replaying: bool) {
        let mut state = self.state.lock();
        let cached = state.sessions.entry(session_id.to_owned()).or_default();
        cached.attached = true;
        cached.replaying = replaying;
        cached.replay_buffer.clear();
    }

    fn detach(&self, session_id: &str) {
        // ACP requires close to cancel work owned by this client. Route the
        // cancellation before detaching so a prompt submitted by this bridge
        // cannot continue invisibly after Zed releases its thread entity.
        self.cancel_prompts(session_id);

        self.state.lock().detach_session(session_id);
    }

    async fn wait_for_daemon(&self) -> Result<(), String> {
        let wait = async {
            loop {
                let notified = self.connected_notify.notified();
                if self.state.lock().connected {
                    return;
                }
                notified.await;
            }
        };
        tokio::time::timeout(DAEMON_READY_TIMEOUT, wait)
            .await
            .map_err(|_| "timed out waiting for the cowboy daemon to reconnect".to_owned())
    }

    async fn create_session(&self, cwd: PathBuf) -> Result<String, String> {
        self.wait_for_daemon().await?;
        let url = self
            .base_url
            .join("api/sessions")
            .map_err(|error| error.to_string())?;
        let response = self
            .http
            .post(url)
            .json(&serde_json::json!({
                "provider": self.provider.as_ref(),
                "cwd": cwd,
                "origin": "api",
                "system": false
            }))
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("cowboy session creation failed ({status}): {body}"));
        }
        response
            .json::<NewSessionHttpResponse>()
            .await
            .map(|response| response.session_id)
            .map_err(|error| error.to_string())
    }

    fn send_command(&self, command: Inbound) -> Result<(), agent_client_protocol::Error> {
        self.command_tx.send(command).map_err(|_| {
            agent_client_protocol::util::internal_error("cowboy daemon connection is closed")
        })
    }

    fn register_prompt(&self, session_id: &str) -> (String, oneshot::Receiver<PromptOutcome>) {
        let n = self.next_cmid.fetch_add(1, Ordering::Relaxed);
        let cmid = format!("acp-{}-{n}", std::process::id());
        let (tx, rx) = oneshot::channel();
        self.state
            .lock()
            .pending_prompts
            .entry(session_id.to_owned())
            .or_default()
            .push_back(PendingPrompt {
                cmid: cmid.clone(),
                started: false,
                cancel_requested: false,
                completion: tx,
            });
        (cmid, rx)
    }

    fn fail_prompt(&self, session_id: &str, cmid: &str, error: String) {
        let pending = {
            let mut state = self.state.lock();
            let Some(queue) = state.pending_prompts.get_mut(session_id) else {
                return;
            };
            let Some(index) = queue.iter().position(|prompt| prompt.cmid == cmid) else {
                return;
            };
            queue.remove(index)
        };
        if let Some(pending) = pending {
            let _ = pending.completion.send(PromptOutcome::Failed(error));
        }
    }

    fn cancel_prompts(&self, session_id: &str) {
        let (queued_cmids, active) = {
            let mut state = self.state.lock();
            let Some(prompts) = state.pending_prompts.get_mut(session_id) else {
                return;
            };
            let mut queued_cmids = Vec::new();
            let active = prompts.iter().any(|prompt| prompt.started);
            for prompt in prompts {
                prompt.cancel_requested = true;
                if !prompt.started {
                    queued_cmids.push(prompt.cmid.clone());
                }
            }
            (queued_cmids, active)
        };

        for cmid in queued_cmids {
            let _ = self.send_command(Inbound::CancelSubmitted {
                session_id: session_id.to_owned(),
                cmid,
            });
        }
        if active {
            let _ = self.send_command(Inbound::Cancel {
                session_id: session_id.to_owned(),
            });
        }
    }

    fn register_config_waiter(
        &self,
        session_id: &str,
    ) -> oneshot::Receiver<Vec<SessionConfigOption>> {
        let (tx, rx) = oneshot::channel();
        self.state
            .lock()
            .config_waiters
            .entry(session_id.to_owned())
            .or_default()
            .push(tx);
        rx
    }

    async fn wait_for_initial_config_options(
        &self,
        session_id: &str,
    ) -> Option<Vec<SessionConfigOption>> {
        let receiver = {
            let mut state = self.state.lock();
            if let Some(options) = state.config_options.get(session_id) {
                return Some(options.clone());
            }
            let (tx, rx) = oneshot::channel();
            state
                .config_waiters
                .entry(session_id.to_owned())
                .or_default()
                .push(tx);
            rx
        };
        match tokio::time::timeout(INITIAL_CONFIG_TIMEOUT, receiver).await {
            Ok(Ok(options)) => Some(options),
            Ok(Err(_)) => None,
            Err(_) => {
                tracing::warn!(
                    session = %session_id,
                    timeout_s = INITIAL_CONFIG_TIMEOUT.as_secs(),
                    "initial config options did not arrive before session/new response"
                );
                None
            }
        }
    }

    fn config_options(&self, session_id: &str) -> Option<Vec<SessionConfigOption>> {
        self.state.lock().config_options.get(session_id).cloned()
    }

    fn status(&self, session_id: &str) -> Option<CowboyStatus> {
        let state = self.state.lock();
        let cached = state.sessions.get(session_id)?;
        let meta = cached.meta.as_ref()?;
        (meta.provider == self.provider.as_ref())
            .then(|| Self::status_from_cached(meta, Some(cached)))
    }

    fn status_from_cached(meta: &SessionMeta, cached: Option<&CachedSession>) -> CowboyStatus {
        CowboyStatus {
            session_id: meta.id.clone(),
            provider: meta.provider.clone(),
            state: status_name(meta.status).to_owned(),
            turn_running: meta.status == Status::Busy,
            agent_alive: matches!(
                meta.status,
                Status::Starting | Status::Running | Status::Busy
            ),
            seq: cached.and_then(|cached| cached.last_seq),
            detail: cached.and_then(|cached| cached.last_detail.clone()),
            // TODO(codex-background-activity): codex app-server knows child
            // threads and background terminals, but codex-acp 1.1.2 does not
            // expose a complete snapshot. Preserve unknown instead of lying.
            background_running: None,
        }
    }

    fn session_meta(&self, session_id: &str) -> serde_json::Map<String, serde_json::Value> {
        self.status(session_id).map_or_else(
            || {
                serde_json::json!({
                    "cowboy": {
                        "version": STATUS_EXTENSION_VERSION,
                        "provider": self.provider.as_ref()
                    }
                })
                .as_object()
                .cloned()
                .unwrap_or_default()
            },
            |status| status_meta(&status),
        )
    }

    async fn replay_session(
        &self,
        session_id: &str,
        cx: &ConnectionTo<Client>,
    ) -> Result<(), String> {
        let history = self.fetch_history(session_id).await;
        let buffered = {
            let mut state = self.state.lock();
            let cached = state
                .sessions
                .get_mut(session_id)
                .ok_or_else(|| format!("session {session_id:?} disappeared during replay"))?;
            cached.replaying = false;
            std::mem::take(&mut cached.replay_buffer)
        };

        let mut history = match history {
            Ok(history) => history,
            Err(error) => {
                // The load response will fail, but do not silently discard live
                // events that arrived while the historical HTTP request was in
                // flight. They remain observable if the client retries load.
                let mut state = self.state.lock();
                if let Some(cached) = state.sessions.get_mut(session_id) {
                    cached.events.extend(buffered);
                    cached.events.sort_unstable_by_key(|event| event.seq);
                    cached.events.dedup_by_key(|event| event.seq);
                }
                return Err(error);
            }
        };
        history.extend(buffered);
        history.sort_unstable_by_key(|event| event.seq);
        history.dedup_by_key(|event| event.seq);
        for envelope in history {
            Self::forward_replay_envelope(cx, &envelope)?;
        }
        if let Some(status) = self.status(session_id) {
            send_status(cx, status).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    async fn fetch_history(&self, session_id: &str) -> Result<Vec<Envelope>, String> {
        let (mut events, mut reached_start) = {
            let state = self.state.lock();
            let cached = state
                .sessions
                .get(session_id)
                .ok_or_else(|| format!("unknown session {session_id:?}"))?;
            (cached.events.clone(), cached.reached_start)
        };
        events.sort_unstable_by_key(|event| event.seq);
        events.dedup_by_key(|event| event.seq);
        let mut before_seq = events.first().map_or(0, |event| event.seq);
        let mut pages = 0usize;
        while !reached_start && before_seq > 0 {
            pages += 1;
            if pages > HISTORY_PAGE_LIMIT {
                return Err("history pagination exceeded safety limit".to_owned());
            }
            let url = self
                .base_url
                .join(&format!("api/history/{session_id}"))
                .map_err(|error| error.to_string())?;
            let response = self
                .http
                .get(url)
                .query(&[("before_seq", before_seq)])
                .send()
                .await
                .map_err(|error| error.to_string())?;
            if !response.status().is_success() {
                return Err(format!("history request failed with {}", response.status()));
            }
            let page = response
                .json::<HistoryResponse>()
                .await
                .map_err(|error| error.to_string())?;
            reached_start = page.reached_start;
            let next = page.next_before_seq;
            events.extend(page.events);
            match next {
                Some(next) if next < before_seq => before_seq = next,
                Some(_) if !reached_start => {
                    return Err("history cursor did not advance".to_owned());
                }
                _ => break,
            }
        }
        Ok(events)
    }

    fn forward_replay_envelope(
        cx: &ConnectionTo<Client>,
        envelope: &Envelope,
    ) -> Result<(), String> {
        if let Event::Update { update } = &envelope.event
            && let Ok(update) = serde_json::from_value::<SessionUpdate>(update.clone())
        {
            cx.send_notification(SessionNotification::new(
                envelope.session_id.clone(),
                update,
            ))
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    async fn run_daemon(
        &self,
        mut command_rx: mpsc::UnboundedReceiver<Inbound>,
        cx: ConnectionTo<Client>,
    ) -> Result<(), agent_client_protocol::Error> {
        let mut reconnect_delay = RECONNECT_INITIAL_DELAY;
        let mut retry_command = None;
        let mut connected_once = false;

        loop {
            let socket = match self.connect_daemon().await {
                Ok(socket) => socket,
                Err(error) => {
                    tracing::warn!(
                        %error,
                        retry_ms = reconnect_delay.as_millis(),
                        "cowboy daemon unavailable; ACP bridge will retry"
                    );
                    tokio::time::sleep(reconnect_delay).await;
                    reconnect_delay = reconnect_delay.saturating_mul(2).min(RECONNECT_MAX_DELAY);
                    continue;
                }
            };

            self.set_connected(true);
            self.publish_attached_state(&cx);
            if connected_once {
                tracing::info!(daemon = %self.base_url, "cowboy daemon WebSocket reconnected");
            } else {
                tracing::info!(daemon = %self.base_url, "cowboy daemon WebSocket connected");
                connected_once = true;
            }
            reconnect_delay = RECONNECT_INITIAL_DELAY;

            match self
                .drive_connected(socket, &mut command_rx, &cx, &mut retry_command)
                .await
            {
                ConnectedExit::CommandsClosed => return Ok(()),
                ConnectedExit::Disconnected(retry) => retry_command = retry,
            }

            self.set_connected(false);
            self.publish_reconnecting_state(&cx);
            tracing::warn!(
                retry_ms = reconnect_delay.as_millis(),
                "cowboy daemon WebSocket disconnected; keeping ACP server alive"
            );
            tokio::time::sleep(reconnect_delay).await;
            reconnect_delay = reconnect_delay.saturating_mul(2).min(RECONNECT_MAX_DELAY);
        }
    }

    async fn connect_daemon(&self) -> anyhow::Result<Socket> {
        let ws_url = websocket_url(&self.base_url)?;
        let (mut socket, _) = connect_async(ws_url.as_str())
            .await
            .with_context(|| format!("connecting to cowboy daemon at {ws_url}"))?;
        wait_for_bootstrap(&mut socket, &self.state).await?;
        Ok(socket)
    }

    async fn drive_connected(
        &self,
        socket: Socket,
        command_rx: &mut mpsc::UnboundedReceiver<Inbound>,
        cx: &ConnectionTo<Client>,
        retry_command: &mut Option<Inbound>,
    ) -> ConnectedExit {
        let (mut sink, mut stream) = socket.split();

        for session_id in self.attached_session_ids() {
            let command = Inbound::OpenSession { session_id };
            if send_daemon_command(&mut sink, &command).await.is_err() {
                return ConnectedExit::Disconnected(None);
            }
        }
        if let Some(command) = retry_command.take()
            && self.should_send_command(&command)
            && send_daemon_command(&mut sink, &command).await.is_err()
        {
            return ConnectedExit::Disconnected(Some(command));
        }

        loop {
            tokio::select! {
                command = command_rx.recv() => {
                    let Some(command) = command else {
                        return ConnectedExit::CommandsClosed;
                    };
                    if self.should_send_command(&command)
                        && send_daemon_command(&mut sink, &command).await.is_err()
                    {
                        return ConnectedExit::Disconnected(Some(command));
                    }
                }
                message = stream.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            match serde_json::from_str::<Outbound>(&text) {
                                Ok(outbound) => {
                                    if let Err(error) = self.handle_outbound(outbound, cx) {
                                        tracing::warn!(%error, "failed to forward daemon update to ACP client");
                                    }
                                }
                                Err(error) => tracing::warn!(%error, "invalid cowboy daemon WebSocket message"),
                            }
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            if sink.send(Message::Pong(payload)).await.is_err() {
                                return ConnectedExit::Disconnected(None);
                            }
                        }
                        Some(Ok(Message::Close(_)) | Err(_)) | None => {
                            return ConnectedExit::Disconnected(None);
                        }
                        Some(Ok(Message::Binary(_) | Message::Pong(_) | Message::Frame(_))) => {}
                    }
                }
            }
        }
    }

    fn set_connected(&self, connected: bool) {
        self.state.lock().connected = connected;
        if connected {
            self.connected_notify.notify_waiters();
        }
    }

    fn attached_session_ids(&self) -> Vec<String> {
        self.state
            .lock()
            .sessions
            .iter()
            .filter(|(_, cached)| cached.attached && cached.meta.is_some())
            .map(|(session_id, _)| session_id.clone())
            .collect()
    }

    fn should_send_command(&self, command: &Inbound) -> bool {
        let pending = |session_id: &str, cmid: &str| {
            self.state
                .lock()
                .pending_prompts
                .get(session_id)
                .is_some_and(|prompts| prompts.iter().any(|prompt| prompt.cmid == cmid))
        };
        match command {
            Inbound::Submit {
                session_id,
                cmid: Some(cmid),
                ..
            }
            | Inbound::CancelSubmitted {
                session_id, cmid, ..
            } => pending(session_id, cmid),
            _ => true,
        }
    }

    fn publish_attached_state(&self, cx: &ConnectionTo<Client>) {
        let (statuses, config_options) = {
            let state = self.state.lock();
            let statuses = state
                .sessions
                .values()
                .filter(|cached| cached.attached)
                .filter_map(|cached| {
                    cached
                        .meta
                        .as_ref()
                        .map(|meta| Self::status_from_cached(meta, Some(cached)))
                })
                .collect::<Vec<_>>();
            let config_options = state
                .sessions
                .iter()
                .filter(|(_, cached)| cached.attached)
                .filter_map(|(session_id, _)| {
                    state
                        .config_options
                        .get(session_id)
                        .cloned()
                        .map(|options| (session_id.clone(), options))
                })
                .collect::<Vec<_>>();
            (statuses, config_options)
        };
        for status in statuses {
            if let Err(error) = send_status(cx, status) {
                tracing::debug!(%error, "ACP client closed while publishing reconnected status");
                return;
            }
        }
        for (session_id, options) in config_options {
            if let Err(error) = cx.send_notification(SessionNotification::new(
                session_id,
                SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(options)),
            )) {
                tracing::debug!(%error, "ACP client closed while restoring config options");
                return;
            }
        }
    }

    fn publish_reconnecting_state(&self, cx: &ConnectionTo<Client>) {
        let statuses = {
            let state = self.state.lock();
            state
                .sessions
                .values()
                .filter(|cached| cached.attached)
                .filter_map(|cached| {
                    let meta = cached.meta.as_ref()?;
                    let mut status = Self::status_from_cached(meta, Some(cached));
                    "reconnecting".clone_into(&mut status.state);
                    status.agent_alive = false;
                    status.detail = Some("cowboy daemon disconnected; reconnecting".to_owned());
                    Some(status)
                })
                .collect::<Vec<_>>()
        };
        for status in statuses {
            if send_status(cx, status).is_err() {
                return;
            }
        }
    }

    fn handle_outbound(
        &self,
        outbound: Outbound,
        cx: &ConnectionTo<Client>,
    ) -> Result<(), agent_client_protocol::Error> {
        match outbound {
            Outbound::Sessions { sessions } => self.apply_sessions(sessions),
            Outbound::Snapshot {
                session_id,
                events,
                reached_start,
            } => self.apply_snapshot(session_id, events, reached_start),
            Outbound::Event { envelope } => self.handle_live_envelope(&envelope, cx)?,
            Outbound::ConfigOptions {
                session_id,
                options,
            } => self.apply_config_options(&session_id, options, cx)?,
            Outbound::Error {
                session_id,
                message,
            } => {
                tracing::warn!(?session_id, %message, "cowboy daemon rejected ACP bridge command");
                if let Some(session_id) = session_id {
                    self.fail_oldest_prompt(&session_id, message);
                }
            }
            Outbound::BootstrapComplete
            | Outbound::Ping
            | Outbound::SyncPatch { .. }
            | Outbound::Settings { .. }
            | Outbound::Skills { .. }
            | Outbound::JudgeResult { .. }
            | Outbound::JudgeHistory { .. } => {}
        }
        Ok(())
    }

    fn apply_sessions(&self, sessions: Vec<SessionMeta>) {
        let mut state = self.state.lock();
        let live_ids = sessions
            .iter()
            .map(|meta| meta.id.clone())
            .collect::<HashSet<_>>();
        for (id, cached) in &mut state.sessions {
            if !live_ids.contains(id) {
                cached.meta = None;
            }
        }
        for meta in sessions {
            let id = meta.id.clone();
            state.sessions.entry(id).or_default().meta = Some(meta);
        }
        state.sessions.retain(|_, cached| {
            cached.meta.is_some() || cached.attached || !cached.events.is_empty()
        });
    }

    fn apply_snapshot(&self, session_id: String, mut events: Vec<Envelope>, reached_start: bool) {
        events.sort_unstable_by_key(|event| event.seq);
        let mut state = self.state.lock();
        let cached = state.sessions.entry(session_id).or_default();
        cached.last_seq = events.last().map(|event| event.seq).or(cached.last_seq);
        cached.events = events;
        cached.reached_start = reached_start;
    }

    fn apply_config_options(
        &self,
        session_id: &str,
        options: serde_json::Value,
        cx: &ConnectionTo<Client>,
    ) -> Result<(), agent_client_protocol::Error> {
        let Ok(options) = serde_json::from_value::<Vec<SessionConfigOption>>(options) else {
            tracing::warn!(session = %session_id, "invalid config options from cowboy daemon");
            return Ok(());
        };
        let (waiters, attached) = {
            let mut state = self.state.lock();
            state
                .config_options
                .insert(session_id.to_owned(), options.clone());
            let attached = state
                .sessions
                .get(session_id)
                .is_some_and(|session| session.attached);
            (
                state.config_waiters.remove(session_id).unwrap_or_default(),
                attached,
            )
        };
        for waiter in waiters {
            let _ = waiter.send(options.clone());
        }
        if attached {
            cx.send_notification(SessionNotification::new(
                session_id.to_owned(),
                SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(options)),
            ))?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)] // One lock computes correlation and forwarding actions atomically.
    fn handle_live_envelope(
        &self,
        envelope: &Envelope,
        cx: &ConnectionTo<Client>,
    ) -> Result<(), agent_client_protocol::Error> {
        let session_id = envelope.session_id.clone();
        let (attached, replaying, status, prompt_outcome, cancel_active) = {
            let mut state = self.state.lock();
            let cached = state.sessions.entry(session_id.clone()).or_default();
            cached.last_seq = Some(envelope.seq);
            if let Event::Lifecycle { status, detail } = &envelope.event {
                if let Some(meta) = cached.meta.as_mut() {
                    meta.status = *status;
                }
                cached.last_detail.clone_from(detail);
            }
            cached.events.push(envelope.clone());
            if cached.events.len() > EVENT_CACHE_LIMIT {
                let excess = cached.events.len() - EVENT_CACHE_LIMIT;
                cached.events.drain(..excess);
                cached.reached_start = false;
            }
            if cached.replaying {
                cached.replay_buffer.push(envelope.clone());
            }
            let attached = cached.attached;
            let replaying = cached.replaying;
            let status = cached
                .meta
                .as_ref()
                .map(|meta| Self::status_from_cached(meta, Some(cached)));

            let mut prompt_outcome = None;
            let mut cancel_active = false;
            if let Some(prompts) = state.pending_prompts.get_mut(&session_id) {
                if let Some(cmid) = envelope.cmid.as_deref()
                    && let Some(prompt) = prompts.iter_mut().find(|prompt| prompt.cmid == cmid)
                {
                    prompt.started = true;
                    cancel_active = prompt.cancel_requested;
                }
                if let Event::Update { update } = &envelope.event
                    && update
                        .get("sessionUpdate")
                        .and_then(serde_json::Value::as_str)
                        == Some("cowboy_prompt_cancelled")
                    && let Some(cmid) = update.get("cmid").and_then(serde_json::Value::as_str)
                    && let Some(index) = prompts.iter().position(|prompt| prompt.cmid == cmid)
                {
                    let pending = prompts.remove(index).expect("index came from queue");
                    prompt_outcome = Some((
                        pending.completion,
                        PromptOutcome::Stopped(StopReason::Cancelled),
                    ));
                }
                if let Event::TurnEnd { stop_reason } = &envelope.event {
                    if let Some(index) = prompts.iter().position(|prompt| prompt.started) {
                        let pending = prompts.remove(index).expect("index came from queue");
                        let outcome = parse_stop_reason(stop_reason).map_or_else(
                            || PromptOutcome::Failed(stop_reason.clone()),
                            PromptOutcome::Stopped,
                        );
                        prompt_outcome = Some((pending.completion, outcome));
                    }
                } else if let Event::Lifecycle { status, .. } = &envelope.event {
                    // A daemon restart can terminate the downstream turn after its
                    // last streamed update but before persisting TurnEnd. Keep the
                    // ACP request alive across the socket outage, then close it
                    // normally once the revived session reports idle. Returning an
                    // internal error at disconnect time leaves a permanent red error
                    // in Zed even though the bridge reconnects successfully.
                    let settled = match status {
                        Status::Running => Some(StopReason::EndTurn),
                        Status::Exited | Status::Crashed | Status::Interrupted => {
                            Some(StopReason::Cancelled)
                        }
                        Status::Starting | Status::Busy => None,
                    };
                    if let Some(reason) = settled
                        && let Some(index) = prompts.iter().position(|prompt| prompt.started)
                    {
                        let pending = prompts.remove(index).expect("index came from queue");
                        prompt_outcome = Some((pending.completion, PromptOutcome::Stopped(reason)));
                    }
                }
            }
            (attached, replaying, status, prompt_outcome, cancel_active)
        };

        if let Some((completion, outcome)) = prompt_outcome {
            let _ = completion.send(outcome);
        }
        if cancel_active {
            self.send_command(Inbound::Cancel {
                session_id: session_id.clone(),
            })?;
        }
        if !attached || replaying {
            return Ok(());
        }

        match &envelope.event {
            Event::Update { update } => {
                if let Ok(update) = serde_json::from_value::<SessionUpdate>(update.clone()) {
                    cx.send_notification(SessionNotification::new(session_id, update))?;
                }
            }
            Event::PermissionRequest {
                request_id,
                tool_call,
                options,
            } => self.forward_permission_request(
                cx,
                &envelope.session_id,
                request_id.clone(),
                tool_call,
                options,
            )?,
            Event::Lifecycle { .. } => {
                if let Some(status) = status {
                    send_status(cx, status)?;
                }
            }
            Event::PermissionResolved { .. } | Event::TurnEnd { .. } => {}
        }
        Ok(())
    }

    fn forward_permission_request(
        &self,
        cx: &ConnectionTo<Client>,
        session_id: &str,
        request_id: String,
        tool_call: &serde_json::Value,
        options: &serde_json::Value,
    ) -> Result<(), agent_client_protocol::Error> {
        let request = serde_json::from_value::<RequestPermissionRequest>(serde_json::json!({
            "sessionId": session_id,
            "toolCall": tool_call,
            "options": options
        }));
        let Ok(request) = request else {
            tracing::warn!(%request_id, "cannot decode permission request for ACP client");
            return Ok(());
        };
        let bridge = self.clone();
        let permission_session = request.session_id.0.to_string();
        let permission_cx = cx.clone();
        cx.spawn(async move {
            match permission_cx.send_request(request).block_task().await {
                Ok(response) => {
                    let option_id = match response.outcome {
                        RequestPermissionOutcome::Selected(selected) => {
                            Some(selected.option_id.0.to_string())
                        }
                        _ => None,
                    };
                    let _ = bridge.send_command(Inbound::Permission {
                        session_id: permission_session,
                        request_id,
                        option_id,
                    });
                }
                Err(error) => tracing::warn!(%error, "ACP client permission request failed"),
            }
            Ok(())
        })?;
        Ok(())
    }

    fn fail_oldest_prompt(&self, session_id: &str, error: String) {
        let pending = self
            .state
            .lock()
            .pending_prompts
            .get_mut(session_id)
            .and_then(VecDeque::pop_front);
        if let Some(pending) = pending {
            let _ = pending.completion.send(PromptOutcome::Failed(error));
        }
    }
}

async fn send_daemon_command(
    sink: &mut futures::stream::SplitSink<Socket, Message>,
    command: &Inbound,
) -> anyhow::Result<()> {
    let text = serde_json::to_string(command).context("encoding cowboy daemon command")?;
    sink.send(Message::Text(text.into()))
        .await
        .context("sending cowboy daemon command")
}

async fn wait_for_bootstrap(
    socket: &mut Socket,
    state: &Arc<Mutex<BridgeState>>,
) -> anyhow::Result<()> {
    tokio::time::timeout(BOOTSTRAP_TIMEOUT, async {
        loop {
            let message = socket
                .next()
                .await
                .ok_or_else(|| anyhow!("cowboy daemon closed during bootstrap"))??;
            let Message::Text(text) = message else {
                continue;
            };
            let outbound: Outbound = serde_json::from_str(&text)?;
            let complete = matches!(outbound, Outbound::BootstrapComplete);
            apply_bootstrap_outbound(state, outbound);
            if complete {
                return Ok::<_, anyhow::Error>(());
            }
        }
    })
    .await
    .context("timed out waiting for cowboy WebSocket bootstrap")??;
    Ok(())
}

fn apply_bootstrap_outbound(state: &Arc<Mutex<BridgeState>>, outbound: Outbound) {
    let mut state = state.lock();
    match outbound {
        Outbound::Sessions { sessions } => {
            let live_ids = sessions
                .iter()
                .map(|meta| meta.id.clone())
                .collect::<HashSet<_>>();
            for (session_id, cached) in &mut state.sessions {
                if !live_ids.contains(session_id) {
                    cached.meta = None;
                }
            }
            for meta in sessions {
                let id = meta.id.clone();
                state.sessions.entry(id).or_default().meta = Some(meta);
            }
            state.sessions.retain(|_, cached| {
                cached.meta.is_some() || cached.attached || !cached.events.is_empty()
            });
        }
        Outbound::Snapshot {
            session_id,
            mut events,
            reached_start,
        } => {
            events.sort_unstable_by_key(|event| event.seq);
            let cached = state.sessions.entry(session_id).or_default();
            cached.last_seq = events.last().map(|event| event.seq);
            cached.events = events;
            cached.reached_start = reached_start;
        }
        Outbound::ConfigOptions {
            session_id,
            options,
        } => {
            if let Ok(options) = serde_json::from_value(options) {
                state.config_options.insert(session_id, options);
            }
        }
        Outbound::Event { envelope } => {
            let cached = state
                .sessions
                .entry(envelope.session_id.clone())
                .or_default();
            cached.last_seq = Some(envelope.seq);
            cached.events.push(envelope);
        }
        Outbound::BootstrapComplete
        | Outbound::Ping
        | Outbound::SyncPatch { .. }
        | Outbound::Settings { .. }
        | Outbound::Skills { .. }
        | Outbound::JudgeResult { .. }
        | Outbound::JudgeHistory { .. }
        | Outbound::Error { .. } => {}
    }
}

fn normalized_base_url(input: &str) -> anyhow::Result<Url> {
    let mut url = Url::parse(input).with_context(|| format!("invalid daemon URL {input:?}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("daemon URL must use http or https");
    }
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

fn websocket_url(base: &Url) -> anyhow::Result<Url> {
    let mut url = base.join("ws")?;
    let scheme = if base.scheme() == "https" {
        "wss"
    } else {
        "ws"
    };
    url.set_scheme(scheme)
        .map_err(|()| anyhow!("cannot convert daemon URL to WebSocket URL"))?;
    Ok(url)
}

fn parse_stop_reason(value: &str) -> Option<StopReason> {
    match value.trim().to_ascii_lowercase().as_str() {
        "endturn" | "end_turn" => Some(StopReason::EndTurn),
        "maxtokens" | "max_tokens" => Some(StopReason::MaxTokens),
        "maxturnrequests" | "max_turn_requests" => Some(StopReason::MaxTurnRequests),
        "refusal" => Some(StopReason::Refusal),
        "cancelled" | "canceled" => Some(StopReason::Cancelled),
        _ => None,
    }
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Starting => "starting",
        Status::Running => "running",
        Status::Busy => "busy",
        Status::Exited => "exited",
        Status::Crashed => "crashed",
        Status::Interrupted => "interrupted",
    }
}

fn status_meta(status: &CowboyStatus) -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "cowboy": {
            "version": STATUS_EXTENSION_VERSION,
            "status": status
        }
    })
    .as_object()
    .cloned()
    .unwrap_or_default()
}

fn send_status(
    cx: &ConnectionTo<Client>,
    status: CowboyStatus,
) -> Result<(), agent_client_protocol::Error> {
    cx.send_notification(SessionNotification::new(
        status.session_id.clone(),
        SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().meta(status_meta(&status))),
    ))?;
    cx.send_notification(CowboyStatusNotification { status })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_config_wait_outlives_provider_handshake() {
        assert!(INITIAL_CONFIG_TIMEOUT > STARTUP_PHASE_TIMEOUT);
    }

    #[test]
    fn stop_reasons_accept_hub_debug_and_wire_names() {
        assert_eq!(parse_stop_reason("EndTurn"), Some(StopReason::EndTurn));
        assert_eq!(parse_stop_reason("max_tokens"), Some(StopReason::MaxTokens));
        assert_eq!(parse_stop_reason("Cancelled"), Some(StopReason::Cancelled));
        assert_eq!(parse_stop_reason("error: adapter died"), None);
    }

    #[test]
    fn websocket_url_preserves_prefix_and_switches_scheme() {
        let base = normalized_base_url("https://example.test/cowboy").unwrap();
        assert_eq!(
            websocket_url(&base).unwrap().as_str(),
            "wss://example.test/cowboy/ws"
        );
    }

    #[test]
    fn status_never_claims_background_idle() {
        let meta = SessionMeta {
            id: "sess-1".to_owned(),
            provider: "codex".to_owned(),
            machine_id: "local".to_owned(),
            cwd: "/tmp".to_owned(),
            title: "test".to_owned(),
            status: Status::Busy,
            origin: crate::core::SessionOrigin::Api,
            agent_session_id: None,
            auto_resume: None,
            awaiting_user: false,
            done: false,
            judging: false,
            paused: false,
            system: false,
            context_used: 0,
            context_size: 0,
            usage: None,
            next_schedule_ms: None,
        };
        let status = Bridge::status_from_cached(&meta, None);
        assert!(status.turn_running);
        assert_eq!(status.background_running, None);
    }

    #[test]
    fn close_detaches_session_and_discards_replay_state() {
        let mut state = BridgeState::default();
        state.sessions.insert(
            "sess-1".to_owned(),
            CachedSession {
                attached: true,
                replaying: true,
                replay_buffer: vec![Envelope {
                    session_id: "sess-1".to_owned(),
                    seq: 1,
                    cmid: None,
                    event: Event::Lifecycle {
                        status: Status::Running,
                        detail: None,
                    },
                }],
                ..CachedSession::default()
            },
        );

        state.detach_session("sess-1");

        let cached = state.sessions.get("sess-1").unwrap();
        assert!(!cached.attached);
        assert!(!cached.replaying);
        assert!(cached.replay_buffer.is_empty());
    }
}
