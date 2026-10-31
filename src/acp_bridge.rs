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
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{
        Error as WsError, Message, client::IntoClientRequest as _, http::header::AUTHORIZATION,
    },
};

use crate::acp::STARTUP_PHASE_TIMEOUT;
use crate::cli::ServeAcpArgs;
use crate::core::{Envelope, Event, Inbound, Outbound, SessionMeta, Status};

#[path = "acp_bridge_subagents.rs"]
mod subagents;

const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);
const CONFIG_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
// The daemon's POST /api/sessions returns before the provider completes its ACP
// handshake. Zed decides whether to construct ConfigOptionsView from the
// session/new response; a later config_option_update can refresh an existing
// view, but cannot create one when the response contained no config options.
// Therefore this wait must outlive one provider startup phase. A cold Codex
// start has been observed taking 23s, well beyond the old 10s timeout.
const INITIAL_CONFIG_TIMEOUT: Duration = Duration::from_secs(STARTUP_PHASE_TIMEOUT.as_secs() + 5);
// Remote creation first prepares a Git workspace, then initializes the Provider
// and its session. Keep that bounded work separate from the config-options wait.
const REMOTE_CREATION_TIMEOUT: Duration = Duration::from_secs(
    crate::machine_control::WORKSPACE_ADAPTER_TIMEOUT.as_secs()
        + 2 * STARTUP_PHASE_TIMEOUT.as_secs()
        + INITIAL_CONFIG_TIMEOUT.as_secs(),
);
const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(30);
const APPROVAL_POLL_INTERVAL: Duration = Duration::from_millis(200);
// Deletion is acknowledged by the daemon's next session list, not by a command
// reply. Bound the wait so a refused or lost deletion cannot hold the request.
const DELETE_SESSION_TIMEOUT: Duration = Duration::from_secs(30);
const RECONNECT_INITIAL_DELAY: Duration = Duration::from_millis(100);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(5);
const HISTORY_PAGE_LIMIT: usize = 10_000;
const EVENT_CACHE_LIMIT: usize = 1_000;
// History pages fetched for one session after a reconnect. An outage longer
// than this is recovered by reloading the thread, not by an unbounded replay.
const CATCH_UP_PAGE_LIMIT: usize = 50;
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
    /// Authoritative: a prompt or provider-reported autonomous continuation is active.
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

/// `GET /api/sessions/{id}/bootstrap`. Frames stay untyped until applied so a
/// newer daemon's additional frame kinds never fail the hydration.
#[derive(Debug, Deserialize)]
struct SessionBootstrapResponse {
    messages: Vec<serde_json::Value>,
}

/// The transcript tail and config options of one session, as hydrated on
/// demand. The daemon WebSocket carries only the global session index.
#[derive(Default)]
struct SessionHydration {
    snapshot: Option<(Vec<Envelope>, bool)>,
    config_options: Option<Vec<SessionConfigOption>>,
}

impl SessionHydration {
    fn from_messages(messages: Vec<serde_json::Value>) -> Self {
        let mut hydration = Self::default();
        for message in messages {
            match serde_json::from_value::<Outbound>(message) {
                Ok(Outbound::Snapshot {
                    mut events,
                    reached_start,
                    ..
                }) => {
                    events.sort_unstable_by_key(|event| event.seq);
                    hydration.snapshot = Some((events, reached_start));
                }
                Ok(Outbound::ConfigOptions { options, .. }) => {
                    hydration.config_options = serde_json::from_value(options).ok();
                }
                Ok(_) | Err(_) => {}
            }
        }
        hydration
    }
}

#[derive(Default)]
struct CachedSession {
    meta: Option<SessionMeta>,
    events: Vec<Envelope>,
    reached_start: bool,
    attached: bool,
    replaying: bool,
    replay_buffer: Vec<Envelope>,
    /// Set while a reconnected session catches up: the last event sequence the
    /// client had received before the daemon connection was lost.
    resume_after: Option<Option<u64>>,
    last_seq: Option<u64>,
    last_detail: Option<String>,
    /// What the client was told about this session's subagents.
    subagents: subagents::SubagentProjection,
}

impl CachedSession {
    fn apply_snapshot(&mut self, events: Vec<Envelope>, reached_start: bool) {
        // Live events newer than the snapshot may already have been applied.
        self.last_seq = self.last_seq.max(events.last().map(|event| event.seq));
        if let Some(detail) = events.iter().rev().find_map(|envelope| {
            if let Event::Lifecycle { detail, .. } = &envelope.event {
                Some(detail.clone())
            } else {
                None
            }
        }) {
            self.last_detail = detail;
        }
        self.events = events;
        self.reached_start = reached_start;
    }
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
    delete_waiters: HashMap<String, Vec<oneshot::Sender<Result<(), String>>>>,
    /// The client declared `clientCapabilities.subagents`.
    subagent_client: bool,
}

/// What a reconnected, attached session missed, in transcript order.
struct CatchUp {
    events: Vec<Envelope>,
    settled: Vec<(oneshot::Sender<PromptOutcome>, PromptOutcome)>,
    config_options: Option<Vec<SessionConfigOption>>,
    status: Option<CowboyStatus>,
}

/// What the client must be told after the daemon published a new session list.
#[derive(Default)]
struct SessionListChange {
    /// Attached sessions whose title changed: `(session_id, title)`.
    retitled: Vec<(String, String)>,
    /// Prompts whose session no longer exists.
    orphaned: Vec<(oneshot::Sender<PromptOutcome>, PromptOutcome)>,
    deleted: Vec<oneshot::Sender<Result<(), String>>>,
}

impl SessionListChange {
    fn settle(self) -> Vec<(String, String)> {
        for (completion, outcome) in self.orphaned {
            let _ = completion.send(outcome);
        }
        for waiter in self.deleted {
            let _ = waiter.send(Ok(()));
        }
        self.retitled
    }
}

impl BridgeState {
    fn detach_session(&mut self, session_id: &str) {
        if let Some(cached) = self.sessions.get_mut(session_id) {
            cached.attached = false;
            cached.replaying = false;
            cached.resume_after = None;
            cached.replay_buffer.clear();
            cached.subagents = subagents::SubagentProjection::default();
        }
        self.config_waiters.remove(session_id);
    }

    /// The attached session whose subagent `child` the client was told about.
    fn subagent_parent(&self, child: &str) -> Option<String> {
        self.sessions
            .iter()
            .find(|(_, cached)| cached.attached && cached.subagents.knows(child))
            .map(|(session_id, _)| session_id.clone())
    }

    /// Adopt the daemon's authoritative session list. A session that left the
    /// list, or that the daemon is already closing, was deleted: its cached
    /// state is released and everything still waiting on it is settled.
    fn apply_session_list(&mut self, sessions: Vec<SessionMeta>) -> SessionListChange {
        let mut change = SessionListChange::default();
        let live_ids = sessions
            .iter()
            .filter(|meta| !meta.closing)
            .map(|meta| meta.id.clone())
            .collect::<HashSet<_>>();
        for meta in sessions {
            let cached = self.sessions.entry(meta.id.clone()).or_default();
            if cached.attached
                && cached
                    .meta
                    .as_ref()
                    .is_some_and(|previous| previous.title != meta.title)
            {
                change.retitled.push((meta.id.clone(), meta.title.clone()));
            }
            cached.meta = Some(meta);
        }
        let gone = self
            .sessions
            .keys()
            .filter(|id| !live_ids.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        for session_id in gone {
            // A session created over HTTP is attached before the daemon lists
            // it; only a session the daemon had listed can have been deleted.
            let listed = self
                .sessions
                .get(&session_id)
                .is_some_and(|cached| cached.meta.is_some());
            let requested = self.delete_waiters.remove(&session_id);
            if !listed && requested.is_none() {
                continue;
            }
            let outcome = || {
                if requested.is_some() {
                    PromptOutcome::Stopped(StopReason::Cancelled)
                } else {
                    PromptOutcome::Failed("this session was deleted in Cowboy".to_owned())
                }
            };
            for pending in self.pending_prompts.remove(&session_id).unwrap_or_default() {
                change.orphaned.push((pending.completion, outcome()));
            }
            change.deleted.extend(requested.unwrap_or_default());
            self.sessions.remove(&session_id);
            self.config_options.remove(&session_id);
            self.config_waiters.remove(&session_id);
        }
        self.sessions
            .retain(|_, cached| cached.meta.is_some() || cached.attached);
        change
    }

    /// Settle a prompt the daemon dispatched whose turn ended while no live
    /// event could report it (the daemon connection was down).
    fn settle_idle_prompt(
        &mut self,
        session_id: &str,
    ) -> Option<(oneshot::Sender<PromptOutcome>, PromptOutcome)> {
        let status = self.sessions.get(session_id)?.meta.as_ref()?.status;
        let reason = idle_stop_reason(status)?;
        let prompts = self.pending_prompts.get_mut(session_id)?;
        let index = prompts.iter().position(|prompt| prompt.started)?;
        let pending = prompts.remove(index)?;
        Some((pending.completion, PromptOutcome::Stopped(reason)))
    }

    /// Close a reconnect catch-up: adopt the hydrated transcript tail and
    /// return what the client missed. Hydrated events after `resume_after` come
    /// first, then the live events buffered while the hydration ran. `None`
    /// when the session was detached or reloaded by the client meanwhile.
    fn complete_catch_up(
        &mut self,
        session_id: &str,
        hydration: SessionHydration,
    ) -> Option<CatchUp> {
        let cached = self.sessions.get_mut(session_id)?;
        let resume_after = cached.resume_after.take()?;
        cached.replaying = false;
        let buffered = std::mem::take(&mut cached.replay_buffer);
        let live = buffered
            .iter()
            .map(|envelope| envelope.seq)
            .collect::<HashSet<_>>();
        let mut hydrated = Vec::new();
        if let Some((events, reached_start)) = hydration.snapshot {
            hydrated.extend(
                events
                    .iter()
                    .filter(|envelope| {
                        resume_after.is_none_or(|seq| envelope.seq > seq)
                            && !live.contains(&envelope.seq)
                    })
                    .cloned(),
            );
            cached.apply_snapshot(events, reached_start);
            cached.events.retain(|event| !live.contains(&event.seq));
            cached.events.extend(buffered.iter().cloned());
            cached.events.sort_unstable_by_key(|event| event.seq);
        }
        let attached = cached.attached;
        let status = cached
            .meta
            .as_ref()
            .map(|meta| Bridge::status_from_cached(meta, Some(cached)));
        // Live events were correlated on arrival; hydrated ones were not.
        let mut settled = Vec::new();
        for envelope in &hydrated {
            settled.extend(self.correlate_prompt(envelope).0);
        }
        settled.extend(self.settle_idle_prompt(session_id));
        if let Some(options) = &hydration.config_options {
            self.config_options
                .insert(session_id.to_owned(), options.clone());
        }
        let mut events = hydrated;
        events.extend(buffered);
        events.sort_unstable_by_key(|event| event.seq);
        // A request answered on another surface during the outage must not
        // reappear as a prompt nobody is waiting for.
        let unresolved = unresolved_permission_requests(&events)
            .into_iter()
            .map(|envelope| envelope.seq)
            .collect::<HashSet<_>>();
        events.retain(|envelope| {
            !matches!(envelope.event, Event::PermissionRequest { .. })
                || unresolved.contains(&envelope.seq)
        });
        Some(CatchUp {
            events,
            settled,
            config_options: hydration.config_options.filter(|_| attached),
            status,
        })
    }

    /// Record the live echo of a prompt this bridge submitted: the daemon has
    /// dispatched it. Returns whether its cancellation was requested earlier.
    fn mark_dispatched(&mut self, envelope: &Envelope) -> bool {
        let Some(cmid) = envelope.cmid.as_deref() else {
            return false;
        };
        self.pending_prompts
            .get_mut(&envelope.session_id)
            .and_then(|prompts| prompts.iter_mut().find(|prompt| prompt.cmid == cmid))
            .is_some_and(|prompt| {
                prompt.started = true;
                prompt.cancel_requested
            })
    }

    /// Correlate one transcript event with the prompts this bridge submitted.
    /// Returns the prompt it settles and whether a cancellation requested
    /// before dispatch must now be routed to the active turn.
    fn correlate_prompt(
        &mut self,
        envelope: &Envelope,
    ) -> (
        Option<(oneshot::Sender<PromptOutcome>, PromptOutcome)>,
        bool,
    ) {
        let mut prompt_outcome = None;
        let cancel_active = self.mark_dispatched(envelope);
        let Some(prompts) = self.pending_prompts.get_mut(&envelope.session_id) else {
            return (None, false);
        };
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
        if let Event::TurnEnd { stop_reason, .. } = &envelope.event {
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
            if let Some(reason) = idle_stop_reason(*status)
                && let Some(index) = prompts.iter().position(|prompt| prompt.started)
            {
                let pending = prompts.remove(index).expect("index came from queue");
                prompt_outcome = Some((pending.completion, PromptOutcome::Stopped(reason)));
            }
        }
        (prompt_outcome, cancel_active)
    }
}

/// How a dispatched prompt ended when the session is observed in `status`
/// without its `TurnEnd`: `None` while the turn can still be running.
fn idle_stop_reason(status: Status) -> Option<StopReason> {
    match status {
        Status::Running => Some(StopReason::EndTurn),
        Status::Exited | Status::Crashed | Status::Interrupted => Some(StopReason::Cancelled),
        Status::Starting | Status::Busy => None,
    }
}

/// Permission requests still awaiting an answer at the end of `events`, in
/// transcript order. This is the Web client's `pendingPermissions` rule:
/// besides its `PermissionResolved`, a request has ended when its id is asked
/// again (ids restart with every agent process), when its tool call reached a
/// terminal status, when the turn ended or when the worker stopped.
fn unresolved_permission_requests(events: &[Envelope]) -> Vec<&Envelope> {
    fn asked(envelope: &Envelope) -> Option<(&str, Option<&str>)> {
        match &envelope.event {
            Event::PermissionRequest {
                request_id,
                tool_call,
                ..
            } => Some((
                request_id,
                tool_call
                    .get("toolCallId")
                    .and_then(serde_json::Value::as_str),
            )),
            _ => None,
        }
    }
    let mut open: Vec<&Envelope> = Vec::new();
    for envelope in events {
        match &envelope.event {
            Event::PermissionRequest { request_id, .. }
            | Event::PermissionResolved { request_id, .. } => {
                open.retain(|candidate| {
                    asked(candidate).is_none_or(|(id, _)| id != request_id.as_str())
                });
                if matches!(envelope.event, Event::PermissionRequest { .. }) {
                    open.push(envelope);
                }
            }
            Event::TurnEnd { .. } => open.clear(),
            Event::Lifecycle { status, .. } => {
                if matches!(
                    status,
                    Status::Crashed | Status::Interrupted | Status::Exited
                ) {
                    open.clear();
                }
            }
            Event::Update { update } => {
                let field = |name: &str| update.get(name).and_then(serde_json::Value::as_str);
                if field("sessionUpdate") == Some("tool_call_update")
                    && matches!(field("status"), Some("completed" | "failed"))
                    && let Some(finished) = field("toolCallId")
                {
                    open.retain(|candidate| {
                        asked(candidate).is_none_or(|(_, tool)| tool != Some(finished))
                    });
                }
            }
        }
    }
    open
}

#[derive(Clone)]
struct Bridge {
    provider: Arc<str>,
    machine: Arc<str>,
    runtime_machine: Option<Arc<str>>,
    workspace: Option<Arc<str>>,
    base_url: Url,
    authentication: crate::client_auth_client::ClientAuthentication,
    http: reqwest::Client,
    state: Arc<Mutex<BridgeState>>,
    connected_notify: Arc<Notify>,
    command_tx: mpsc::UnboundedSender<Inbound>,
    next_cmid: Arc<AtomicU64>,
}

pub async fn serve(args: ServeAcpArgs) -> anyhow::Result<()> {
    if let Some(runtime) = args.runtime_machine.as_deref()
        && (runtime.trim().is_empty()
            || runtime == "local"
            || args.machine == "local"
            || runtime == args.machine)
    {
        bail!("--runtime-machine requires distinct enrolled runtime and Project Machines");
    }
    if args.provider.is_empty()
        || args.provider.len() > 128
        || matches!(args.provider.as_str(), "." | "..")
        || !args.provider.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
    {
        bail!("invalid Provider id {:?}", args.provider);
    }

    let base_url = crate::client_auth_client::normalized_base_url(&args.daemon_url)?;
    let authentication = crate::client_auth_client::ClientAuthentication::new(
        base_url.clone(),
        args.token.as_deref(),
        args.auth_state_dir,
        args.device_name,
    )?;
    let state = Arc::new(Mutex::new(BridgeState::default()));

    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let bridge = Bridge {
        provider: Arc::from(args.provider),
        machine: Arc::from(args.machine),
        runtime_machine: args.runtime_machine.map(Arc::from),
        workspace: args.workspace.map(Arc::from),
        base_url,
        authentication,
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("building ACP HTTP client")?,
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
                initialize_bridge.state.lock().subagent_client =
                    request.client_capabilities.subagents.is_some();
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
            async move |request: ListSessionsRequest, responder, cx: ConnectionTo<Client>| {
                let bridge = list_bridge.clone();
                cx.spawn(async move {
                    match bridge.ready_sessions(request.cwd.as_ref()).await {
                        Ok(sessions) => responder.respond(ListSessionsResponse::new(sessions))?,
                        Err(error) => responder.respond_with_error(
                            agent_client_protocol::util::internal_error(error),
                        )?,
                    }
                    Ok(())
                })?;
                Ok(())
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
                                match bridge.wait_for_initial_config_options(&session_id).await {
                                    Ok(options) => options,
                                    Err(error) => {
                                        bridge.state.lock().detach_session(&session_id);
                                        responder.respond_with_error(
                                            agent_client_protocol::util::internal_error(error),
                                        )?;
                                        return Ok(());
                                    }
                                };
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
                    if let Err(error) = bridge
                        .validate_ready_session(&session_id, Some(&request.cwd))
                        .await
                    {
                        responder.respond_with_error(
                            agent_client_protocol::util::internal_error(error),
                        )?;
                        return Ok(());
                    }
                    let was_attached = bridge.attach(&session_id, true);
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
                            // The client has no thread for a load it was
                            // refused; do not keep streaming updates into it.
                            if !was_attached {
                                bridge.state.lock().detach_session(&session_id);
                            }
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
                // Releasing a thread must succeed even when Cowboy already
                // deleted its session; only another provider's session is
                // refused.
                if close_bridge.knows_session(&session_id)
                    && let Err(error) = close_bridge.validate_session(&session_id, None)
                {
                    return responder
                        .respond_with_error(agent_client_protocol::util::internal_error(error));
                }
                close_bridge.detach(&session_id);
                responder.respond(CloseSessionResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: DeleteSessionRequest, responder, cx: ConnectionTo<Client>| {
                let bridge = delete_bridge.clone();
                let session_id = request.session_id.0.to_string();
                if !bridge.knows_session(&session_id) {
                    // Already deleted in Cowboy: the requested end state holds.
                    bridge.detach(&session_id);
                    return responder.respond(DeleteSessionResponse::new());
                }
                if let Err(error) = bridge.validate_session(&session_id, None) {
                    return responder
                        .respond_with_error(agent_client_protocol::util::internal_error(error));
                }
                let deleted = bridge.register_delete_waiter(&session_id);
                bridge.send_command(Inbound::DeleteSession {
                    session_id: session_id.clone(),
                })?;
                cx.spawn(async move {
                    match tokio::time::timeout(DELETE_SESSION_TIMEOUT, deleted).await {
                        Ok(Ok(Ok(()))) => responder.respond(DeleteSessionResponse::new())?,
                        Ok(Ok(Err(error))) => responder.respond_with_error(
                            agent_client_protocol::util::internal_error(error),
                        )?,
                        Ok(Err(_)) | Err(_) => {
                            bridge.state.lock().delete_waiters.remove(&session_id);
                            responder.respond_with_error(
                                agent_client_protocol::util::internal_error(
                                    "timed out waiting for Cowboy to delete the session",
                                ),
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
            async move |request: PromptRequest, responder, cx: ConnectionTo<Client>| {
                let bridge = prompt_bridge.clone();
                let session_id = request.session_id.0.to_string();
                if let Err(error) = bridge.validate_session(&session_id, None) {
                    return responder
                        .respond_with_error(agent_client_protocol::util::internal_error(error));
                }
                let content = request
                    .prompt
                    .into_iter()
                    .filter_map(|block| serde_json::to_value(block).ok())
                    .collect::<Vec<_>>();
                if content.is_empty() {
                    // The daemon drops an empty submission without an event,
                    // which would leave this request pending forever.
                    return responder.respond_with_error(
                        agent_client_protocol::Error::invalid_params()
                            .data(serde_json::json!("prompt has no content blocks")),
                    );
                }
                let (cmid, completion) = bridge.register_prompt(&session_id);
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
                let session_id = notification.session_id.0.as_ref();
                // A subagent client stops one subagent by cancelling its
                // child session; Cowboy addresses it through its parent.
                let parent = cancel_bridge.state.lock().subagent_parent(session_id);
                if let Some(parent) = parent {
                    cancel_bridge.send_command(Inbound::CancelSubagent {
                        session_id: parent,
                        subagent_session_id: session_id.to_owned(),
                    })?;
                    return Ok(());
                }
                cancel_bridge.cancel_prompts(session_id);
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
        format!("Cowboy · {}", crate::provider::display_name(&self.provider))
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

    async fn ready_sessions(&self, cwd: Option<&PathBuf>) -> Result<Vec<SessionInfo>, String> {
        self.wait_for_daemon().await?;
        Ok(self.list_sessions(cwd))
    }

    async fn validate_ready_session(
        &self,
        session_id: &str,
        cwd: Option<&PathBuf>,
    ) -> Result<(), String> {
        self.wait_for_daemon().await?;
        self.validate_session(session_id, cwd)
    }

    fn list_sessions(&self, cwd: Option<&PathBuf>) -> Vec<SessionInfo> {
        let state = self.state.lock();
        let mut sessions = state
            .sessions
            .values()
            .filter_map(|cached| cached.meta.as_ref())
            .filter(|meta| meta.provider == self.provider.as_ref())
            .filter(|meta| {
                if self.machine.as_ref() == "local" {
                    cwd.is_none_or(|cwd| std::path::Path::new(&meta.cwd) == cwd)
                } else {
                    self.matches_remote_workspace(meta)
                }
            })
            .map(|meta| {
                let client_cwd = if self.machine.as_ref() != "local" {
                    cwd.cloned().unwrap_or_else(|| PathBuf::from(&meta.cwd))
                } else {
                    PathBuf::from(&meta.cwd)
                };
                SessionInfo::new(meta.id.clone(), client_cwd)
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

    fn matches_remote_workspace(&self, meta: &SessionMeta) -> bool {
        if let Some(runtime) = self.runtime_machine.as_deref() {
            return meta.machine_id == runtime
                && meta.execution_binding.as_ref().is_some_and(|binding| {
                    let ready = binding
                        .for_runtime(runtime, &meta.cwd)
                        .is_ok_and(|binding| {
                            binding.environment.machine_id == self.machine.as_ref()
                                && self.workspace.as_deref() == Some(binding.workspace.id.as_str())
                        });
                    ready
                        || binding.preparation().is_some_and(|preparation| {
                            preparation.runtime.machine_id == runtime
                                && preparation.runtime.cwd == meta.cwd
                                && preparation.machine_id == self.machine.as_ref()
                                && self.workspace.as_deref()
                                    == Some(preparation.workspace_id.as_str())
                        })
                });
        }
        meta.machine_id == self.machine.as_ref()
            && self.workspace.is_some()
            && meta.workspace_id.as_deref() == self.workspace.as_deref()
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
        if self.machine.as_ref() != "local" {
            if !self.matches_remote_workspace(meta) {
                return Err(format!(
                    "session {session_id:?} does not belong to the selected Machine and workspace"
                ));
            }
            return Ok(());
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

    fn knows_session(&self, session_id: &str) -> bool {
        self.state
            .lock()
            .sessions
            .get(session_id)
            .is_some_and(|cached| cached.meta.is_some())
    }

    fn register_delete_waiter(&self, session_id: &str) -> oneshot::Receiver<Result<(), String>> {
        let (tx, rx) = oneshot::channel();
        self.state
            .lock()
            .delete_waiters
            .entry(session_id.to_owned())
            .or_default()
            .push(tx);
        rx
    }

    /// Returns whether the client already held the session.
    fn attach(&self, session_id: &str, replaying: bool) -> bool {
        let mut state = self.state.lock();
        let cached = state.sessions.entry(session_id.to_owned()).or_default();
        let was_attached = std::mem::replace(&mut cached.attached, true);
        cached.replaying = replaying;
        cached.resume_after = None;
        cached.replay_buffer.clear();
        // A loaded thread is rebuilt from its replay, subagents included.
        cached.subagents = subagents::SubagentProjection::default();
        was_attached
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
                    return Ok(());
                }
                // The first connection waits for a person to approve this
                // computer in a browser. The ACP client hides stderr, so the
                // request is the only place the link can reach them; answer
                // at once instead of a timeout that names no cause.
                if let Some(pending) = self.authentication.pending_approval() {
                    return Err(approval_required_message(&pending));
                }
                let _ = tokio::time::timeout(APPROVAL_POLL_INTERVAL, notified).await;
            }
        };
        tokio::time::timeout(DAEMON_READY_TIMEOUT, wait)
            .await
            .map_err(|_| "timed out waiting for the cowboy daemon to reconnect".to_owned())?
    }

    async fn create_session(&self, cwd: PathBuf) -> Result<String, String> {
        let requested_workspace = if self.machine.as_ref() == "local" {
            if self.workspace.is_some() {
                return Err("--workspace requires a non-local --machine".to_owned());
            }
            serde_json::json!(cwd)
        } else {
            let workspace = self
                .workspace
                .as_deref()
                .filter(|workspace| !workspace.trim().is_empty())
                .ok_or(
                    "remote session creation requires --workspace with a registered workspace ID",
                )?;
            serde_json::json!(workspace)
        };
        self.wait_for_daemon().await?;
        let path = if self.runtime_machine.is_some() {
            "api/execution-sessions"
        } else {
            "api/sessions"
        };
        let url = self
            .base_url
            .join(path)
            .map_err(|error| error.to_string())?;
        let target = url_request_target(&url);
        let mut body = serde_json::json!({
            "provider": self.provider.as_ref(),
            "machine_id": self.machine.as_ref(),
            "cwd": requested_workspace,
            "origin": "api",
            "system": false
        });
        if let Some(runtime) = self.runtime_machine.as_deref() {
            body.as_object_mut()
                .expect("creation body is an object")
                .remove("system");
            body["runtime_machine_id"] = serde_json::json!(runtime);
        }
        let mut rejected_access = None;
        let mut attempts = 0_u8;
        let response = loop {
            attempts = attempts.saturating_add(1);
            let authorization = self
                .authentication
                .authorize(&reqwest::Method::POST, &target, rejected_access.as_deref())
                .await
                .map_err(|error| error.to_string())?;
            let request = authorization
                .apply_reqwest(self.http.post(url.clone()).json(&body))
                .map_err(|error| error.to_string())?;
            let response = request.send().await.map_err(|error| error.to_string())?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED && attempts == 1 {
                rejected_access = Some(authorization.bearer.unwrap_or_default());
                continue;
            }
            break response;
        };
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
    ) -> Result<Option<Vec<SessionConfigOption>>, String> {
        if self.machine.as_ref() != "local" {
            return tokio::time::timeout(REMOTE_CREATION_TIMEOUT, async {
                let mut ready_since = None;
                loop {
                    {
                        let state = self.state.lock();
                        if let Some(cached) = state.sessions.get(session_id) {
                            let status =
                                cached.meta.as_ref().map(|meta| meta.status).or_else(|| {
                                    cached.events.iter().rev().find_map(|envelope| {
                                        if let Event::Lifecycle { status, .. } = &envelope.event {
                                            Some(*status)
                                        } else {
                                            None
                                        }
                                    })
                                });
                            match status {
                                Some(Status::Crashed | Status::Exited | Status::Interrupted) => {
                                    return Err(cached.last_detail.clone().unwrap_or_else(|| {
                                        "remote Machine session preparation failed".to_owned()
                                    }));
                                }
                                Some(Status::Running | Status::Busy) => {
                                    if let Some(options) = state.config_options.get(session_id) {
                                        return Ok(Some(options.clone()));
                                    }
                                    let since =
                                        ready_since.get_or_insert_with(tokio::time::Instant::now);
                                    if since.elapsed() >= INITIAL_CONFIG_TIMEOUT {
                                        return Ok(None);
                                    }
                                }
                                Some(Status::Starting) | None => ready_since = None,
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .map_err(|_| "timed out waiting for remote Machine session readiness".to_owned())?;
        }
        let receiver = {
            let mut state = self.state.lock();
            if let Some(options) = state.config_options.get(session_id) {
                return Ok(Some(options.clone()));
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
            Ok(Ok(options)) => Ok(Some(options)),
            Ok(Err(_)) => Ok(None),
            Err(_) => {
                // The broadcast may have been published while the daemon
                // connection was re-established; ask for the current value.
                if let Ok(hydration) = self.fetch_hydration(session_id).await
                    && let Some(options) = hydration.config_options
                {
                    self.state
                        .lock()
                        .config_options
                        .insert(session_id.to_owned(), options.clone());
                    return Ok(Some(options));
                }
                tracing::warn!(
                    session = %session_id,
                    timeout_s = INITIAL_CONFIG_TIMEOUT.as_secs(),
                    "initial config options did not arrive before session/new response"
                );
                Ok(None)
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
        let history = self.load_history(session_id).await;
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
        for envelope in &history {
            self.forward_replay_envelope(cx, envelope)?;
        }
        let status = self.status(session_id);
        // A turn that is still waiting for an answer keeps waiting after the
        // thread is reopened; without the request the client could only watch.
        if status.as_ref().is_some_and(|status| status.turn_running) {
            for envelope in unresolved_permission_requests(&history) {
                self.forward_envelope(cx, envelope, None)
                    .map_err(|error| error.to_string())?;
            }
        }
        if let Some(status) = status {
            send_status(cx, status).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Authorized `GET` returning JSON, retrying once with a fresh access token.
    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        url: Url,
        what: &str,
    ) -> Result<T, String> {
        let target = url_request_target(&url);
        let mut rejected_access = None;
        let mut attempts = 0_u8;
        let response = loop {
            attempts = attempts.saturating_add(1);
            let authorization = self
                .authentication
                .authorize(&reqwest::Method::GET, &target, rejected_access.as_deref())
                .await
                .map_err(|error| error.to_string())?;
            let request = authorization
                .apply_reqwest(self.http.get(url.clone()))
                .map_err(|error| error.to_string())?;
            let response = request.send().await.map_err(|error| error.to_string())?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED && attempts == 1 {
                rejected_access = Some(authorization.bearer.unwrap_or_default());
                continue;
            }
            break response;
        };
        if !response.status().is_success() {
            let status = response.status();
            return Err(format!("{what} request failed with {status}"));
        }
        response
            .json::<T>()
            .await
            .map_err(|error| error.to_string())
    }

    /// Fetch one session's transcript tail and config options. The daemon
    /// WebSocket is negotiated lazily and carries neither.
    async fn fetch_hydration(&self, session_id: &str) -> Result<SessionHydration, String> {
        let url = self
            .base_url
            .join(&format!("api/sessions/{session_id}/bootstrap"))
            .map_err(|error| error.to_string())?;
        let response = self
            .get_json::<SessionBootstrapResponse>(url, "session bootstrap")
            .await?;
        Ok(SessionHydration::from_messages(response.messages))
    }

    /// Hydrate the session, then page back to the start of its transcript.
    async fn load_history(&self, session_id: &str) -> Result<Vec<Envelope>, String> {
        let hydration = self.fetch_hydration(session_id).await?;
        {
            let mut state = self.state.lock();
            if let Some(options) = hydration.config_options {
                state.config_options.insert(session_id.to_owned(), options);
            }
            if let Some((events, reached_start)) = hydration.snapshot {
                state
                    .sessions
                    .get_mut(session_id)
                    .ok_or_else(|| format!("session {session_id:?} disappeared during replay"))?
                    .apply_snapshot(events, reached_start);
            }
        }
        self.fetch_history(session_id).await
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
            let mut url = self
                .base_url
                .join(&format!("api/history/{session_id}"))
                .map_err(|error| error.to_string())?;
            url.query_pairs_mut()
                .append_pair("before_seq", &before_seq.to_string());
            let page = self.get_json::<HistoryResponse>(url, "history").await?;
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
        events.sort_unstable_by_key(|event| event.seq);
        events.dedup_by_key(|event| event.seq);
        Ok(events)
    }

    fn forward_replay_envelope(
        &self,
        cx: &ConnectionTo<Client>,
        envelope: &Envelope,
    ) -> Result<(), String> {
        if let Event::Update { update } = &envelope.event {
            for notification in self.client_updates(&envelope.session_id, update) {
                cx.send_notification(notification)
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    /// The ACP notifications for one transcript update. A `cowboy_subagent`
    /// snapshot is projected for this client (see [`subagents`]); an update
    /// with no ACP form is dropped.
    fn client_updates(
        &self,
        session_id: &str,
        update: &serde_json::Value,
    ) -> Vec<SessionNotification> {
        if update
            .get("sessionUpdate")
            .and_then(serde_json::Value::as_str)
            == Some(subagents::SUBAGENT_UPDATE)
        {
            let mut state = self.state.lock();
            let standard = state.subagent_client;
            return state
                .sessions
                .get_mut(session_id)
                .map(|cached| cached.subagents.project(session_id, update, standard))
                .unwrap_or_default();
        }
        serde_json::from_value::<SessionUpdate>(update.clone())
            .map(|update| vec![SessionNotification::new(session_id.to_owned(), update)])
            .unwrap_or_default()
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
                Err(error) if daemon_auth_rejected(&error) => {
                    return Err(agent_client_protocol::util::internal_error(
                        error.to_string(),
                    ));
                }
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
        let target = url_request_target(&ws_url);
        let mut rejected_access = None;
        let mut attempts = 0_u8;
        let mut socket = loop {
            attempts = attempts.saturating_add(1);
            let authorization = self
                .authentication
                .authorize(&reqwest::Method::GET, &target, rejected_access.as_deref())
                .await?;
            let mut request = ws_url.as_str().into_client_request().with_context(|| {
                format!("building cowboy daemon WebSocket request for {ws_url}")
            })?;
            if let Some(bearer) = &authorization.bearer {
                request.headers_mut().insert(
                    AUTHORIZATION,
                    format!("Bearer {bearer}")
                        .parse()
                        .context("invalid Cowboy authorization header")?,
                );
            }
            for (name, value) in &authorization.proof_headers {
                request.headers_mut().insert(
                    tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(name.as_bytes())
                        .context("invalid Cowboy proof header name")?,
                    value.parse().context("invalid Cowboy proof header value")?,
                );
            }
            match connect_async(request).await {
                Ok((socket, _)) => break socket,
                Err(error) if websocket_auth_rejected(&error) && attempts == 1 => {
                    rejected_access = Some(authorization.bearer.unwrap_or_default());
                }
                Err(error) if websocket_auth_rejected(&error) => {
                    return Err(anyhow!(error).context(daemon_auth_rejection_message(
                        "cowboy daemon rejected this computer login",
                    )));
                }
                Err(error) => {
                    return Err(
                        anyhow!(error).context(format!("connecting to cowboy daemon at {ws_url}"))
                    );
                }
            }
        };
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
        let mut catch_ups = self
            .begin_catch_up()
            .into_iter()
            .map(|(session_id, resume_after)| async move {
                let missed = self.fetch_missed(&session_id, resume_after).await;
                (session_id, missed)
            })
            .collect::<futures::stream::FuturesUnordered<_>>();

        loop {
            tokio::select! {
                Some((session_id, missed)) = catch_ups.next(), if !catch_ups.is_empty() => {
                    if let Err(error) = self.finish_catch_up(&session_id, missed, cx) {
                        tracing::warn!(%error, "failed to forward a reconnect catch-up to ACP client");
                    }
                }
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
            Outbound::Sessions { sessions } => self.apply_sessions(sessions, cx)?,
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
                tracing::warn!(?session_id, %message, "cowboy daemon reported a command error");
                if let Some(session_id) = session_id {
                    self.fail_refused(&session_id, &message);
                }
            }
            // The addressed refusal of a prompt this bridge submitted.
            Outbound::CommandResult {
                session_id,
                cmid,
                message,
                ..
            } => self.fail_prompt(&session_id, &cmid, message),
            Outbound::BootstrapComplete
            | Outbound::AuthSession { .. }
            | Outbound::ClientCapacity { .. }
            | Outbound::Machines { .. }
            | Outbound::Ping
            | Outbound::ConnectionProbe { .. }
            | Outbound::SyncPatch { .. }
            | Outbound::Usage { .. }
            | Outbound::AccountUsageChanged { .. }
            | Outbound::CallApproval { .. }
            | Outbound::Settings { .. } => {}
        }
        Ok(())
    }

    fn apply_sessions(
        &self,
        sessions: Vec<SessionMeta>,
        cx: &ConnectionTo<Client>,
    ) -> Result<(), agent_client_protocol::Error> {
        let change = self.state.lock().apply_session_list(sessions);
        for (session_id, title) in change.settle() {
            // Cowboy titles a session from its first prompt and lets the user
            // rename it; the open thread follows instead of keeping a stale name.
            cx.send_notification(SessionNotification::new(
                session_id,
                SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title(title)),
            ))?;
        }
        Ok(())
    }

    fn apply_snapshot(&self, session_id: String, events: Vec<Envelope>, reached_start: bool) {
        let mut events = events;
        events.sort_unstable_by_key(|event| event.seq);
        self.state
            .lock()
            .sessions
            .entry(session_id)
            .or_default()
            .apply_snapshot(events, reached_start);
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

    fn handle_live_envelope(
        &self,
        envelope: &Envelope,
        cx: &ConnectionTo<Client>,
    ) -> Result<(), agent_client_protocol::Error> {
        let session_id = envelope.session_id.clone();
        let (forward, status, prompt_outcome, cancel_active) = {
            let mut state = self.state.lock();
            let cached = state.sessions.entry(session_id.clone()).or_default();
            // Hydration and the live stream overlap; a sequence is applied once.
            // Only the live copy carries the prompt echo, so that still counts.
            if cached.last_seq.is_some_and(|last| envelope.seq <= last)
                && cached.events.iter().any(|seen| seen.seq == envelope.seq)
            {
                let cancel_active = state.mark_dispatched(envelope);
                drop(state);
                if cancel_active {
                    self.send_command(Inbound::Cancel { session_id })?;
                }
                return Ok(());
            }
            cached.last_seq = Some(
                cached
                    .last_seq
                    .map_or(envelope.seq, |last| last.max(envelope.seq)),
            );
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
            let forward = cached.attached && !cached.replaying;
            let status = cached
                .meta
                .as_ref()
                .map(|meta| Self::status_from_cached(meta, Some(cached)));
            let (prompt_outcome, cancel_active) = state.correlate_prompt(envelope);
            (forward, status, prompt_outcome, cancel_active)
        };

        if let Some((completion, outcome)) = prompt_outcome {
            let _ = completion.send(outcome);
        }
        if cancel_active {
            self.send_command(Inbound::Cancel {
                session_id: session_id.clone(),
            })?;
        }
        if !forward {
            return Ok(());
        }
        self.forward_envelope(cx, envelope, status)
    }

    /// Publish one transcript event to the ACP client.
    fn forward_envelope(
        &self,
        cx: &ConnectionTo<Client>,
        envelope: &Envelope,
        status: Option<CowboyStatus>,
    ) -> Result<(), agent_client_protocol::Error> {
        match &envelope.event {
            Event::Update { update } => {
                for notification in self.client_updates(&envelope.session_id, update) {
                    cx.send_notification(notification)?;
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

    /// Mark every attached session as catching up and return the ones to
    /// hydrate. Live events are buffered until [`Self::finish_catch_up`].
    fn begin_catch_up(&self) -> Vec<(String, Option<u64>)> {
        let mut state = self.state.lock();
        state
            .sessions
            .iter_mut()
            // A session the client is loading replays its own history. A
            // catch-up cut short by another disconnect resumes from its
            // original position and keeps what it buffered.
            .filter(|(_, cached)| {
                cached.attached
                    && cached.meta.is_some()
                    && (!cached.replaying || cached.resume_after.is_some())
            })
            .map(|(session_id, cached)| {
                cached.replaying = true;
                let resume_after = *cached.resume_after.get_or_insert(cached.last_seq);
                (session_id.clone(), resume_after)
            })
            .collect()
    }

    /// Hydrate a reconnected session and, when the outage outlasted the
    /// transcript tail, page back to the last event the client received.
    async fn fetch_missed(
        &self,
        session_id: &str,
        resume_after: Option<u64>,
    ) -> Result<SessionHydration, String> {
        let mut hydration = self.fetch_hydration(session_id).await?;
        let (Some(resume_after), Some((events, reached_start))) =
            (resume_after, hydration.snapshot.as_mut())
        else {
            return Ok(hydration);
        };
        let mut before_seq = events.first().map_or(0, |event| event.seq);
        let mut pages = 0usize;
        while !*reached_start && before_seq > resume_after.saturating_add(1) {
            pages += 1;
            if pages > CATCH_UP_PAGE_LIMIT {
                tracing::warn!(session = %session_id, "reconnect gap exceeds the catch-up limit");
                break;
            }
            let mut url = self
                .base_url
                .join(&format!("api/history/{session_id}"))
                .map_err(|error| error.to_string())?;
            url.query_pairs_mut()
                .append_pair("before_seq", &before_seq.to_string());
            let page = self.get_json::<HistoryResponse>(url, "history").await?;
            *reached_start = page.reached_start;
            events.extend(page.events);
            match page.next_before_seq {
                Some(next) if next < before_seq => before_seq = next,
                _ => break,
            }
        }
        events.sort_unstable_by_key(|event| event.seq);
        events.dedup_by_key(|event| event.seq);
        Ok(hydration)
    }

    /// Deliver what an attached session missed while the daemon connection was
    /// down: the hydrated events after the last one the client received, then
    /// the live events buffered meanwhile. Runs on the daemon loop so it can
    /// never interleave with the live stream.
    fn finish_catch_up(
        &self,
        session_id: &str,
        hydration: Result<SessionHydration, String>,
        cx: &ConnectionTo<Client>,
    ) -> Result<(), agent_client_protocol::Error> {
        let hydration = hydration.unwrap_or_else(|error| {
            tracing::warn!(session = %session_id, %error, "could not catch up a reconnected session");
            SessionHydration::default()
        });
        let Some(catch_up) = self.state.lock().complete_catch_up(session_id, hydration) else {
            return Ok(());
        };
        for (completion, outcome) in catch_up.settled {
            let _ = completion.send(outcome);
        }
        for envelope in &catch_up.events {
            self.forward_envelope(cx, envelope, None)?;
        }
        if let Some(options) = catch_up.config_options {
            cx.send_notification(SessionNotification::new(
                session_id.to_owned(),
                SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(options)),
            ))?;
        }
        if let Some(status) = catch_up.status {
            send_status(cx, status)?;
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
        let mut tool_call = tool_call.clone();
        let standard = self.state.lock().subagent_client;
        // A subagent's request is asked in its child session, or named in the
        // title for a client without subagents. The answer still goes to the
        // parent session, which owns the request.
        let asking_session = subagents::subagent_request(&mut tool_call, standard)
            .unwrap_or_else(|| session_id.to_owned());
        let request = serde_json::from_value::<RequestPermissionRequest>(serde_json::json!({
            "sessionId": asking_session,
            "toolCall": tool_call,
            "options": options
        }));
        let Ok(request) = request else {
            tracing::warn!(%request_id, "cannot decode permission request for ACP client");
            return Ok(());
        };
        let bridge = self.clone();
        let permission_session = session_id.to_owned();
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

    /// A session-scoped daemon error is broadcast to every surface and names no
    /// command. It can only be this bridge's refusal for a deletion in flight or
    /// for a prompt the daemon has not dispatched yet; a turn that is already
    /// running ends through its own `TurnEnd`, never through another surface's
    /// failed command.
    fn fail_refused(&self, session_id: &str, error: &str) {
        let (deletions, pending) = {
            let mut state = self.state.lock();
            let deletions = state.delete_waiters.remove(session_id).unwrap_or_default();
            let pending = if deletions.is_empty() {
                state
                    .pending_prompts
                    .get_mut(session_id)
                    .and_then(|prompts| {
                        let index = prompts.iter().position(|prompt| !prompt.started)?;
                        prompts.remove(index)
                    })
            } else {
                None
            };
            (deletions, pending)
        };
        for waiter in deletions {
            let _ = waiter.send(Err(error.to_owned()));
        }
        if let Some(pending) = pending {
            let _ = pending
                .completion
                .send(PromptOutcome::Failed(error.to_owned()));
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
            // A newer daemon may add frame kinds; skipping one must not turn
            // every connection attempt into a bootstrap failure.
            let Ok(outbound) = serde_json::from_str::<Outbound>(&text) else {
                tracing::debug!("ignoring an unknown cowboy daemon bootstrap frame");
                continue;
            };
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
            // No client is reachable from here; title changes reach it with the
            // next live list, deletions settle now.
            state.apply_session_list(sessions).settle();
        }
        // The connection is negotiated lazily; a daemon that still sends the
        // complete bootstrap is accepted and its frames are applied.
        Outbound::Snapshot {
            session_id,
            mut events,
            reached_start,
        } => {
            events.sort_unstable_by_key(|event| event.seq);
            state
                .sessions
                .entry(session_id)
                .or_default()
                .apply_snapshot(events, reached_start);
        }
        Outbound::ConfigOptions {
            session_id,
            options,
        } => {
            if let Ok(options) = serde_json::from_value(options) {
                state.config_options.insert(session_id, options);
            }
        }
        Outbound::Event { .. }
        | Outbound::BootstrapComplete
        | Outbound::AuthSession { .. }
        | Outbound::ClientCapacity { .. }
        | Outbound::Machines { .. }
        | Outbound::Ping
        | Outbound::ConnectionProbe { .. }
        | Outbound::SyncPatch { .. }
        | Outbound::Settings { .. }
        | Outbound::Usage { .. }
        | Outbound::AccountUsageChanged { .. }
        | Outbound::CallApproval { .. }
        | Outbound::CommandResult { .. }
        | Outbound::Error { .. } => {}
    }
}

fn approval_required_message(pending: &crate::client_auth_client::PendingApproval) -> String {
    format!(
        "Cowboy is waiting for you to approve this computer. Open {} in a browser, check that \
         the client fingerprint is {}, approve it, then try again.",
        pending.url, pending.fingerprint
    )
}

fn is_fatal_auth_status(status: u16) -> bool {
    status == 401 || status == 403
}

fn websocket_handshake_status(error: &WsError) -> Option<u16> {
    match error {
        WsError::Http(response) => Some(response.status().as_u16()),
        _ => None,
    }
}

fn websocket_auth_rejected(error: &WsError) -> bool {
    websocket_handshake_status(error).is_some_and(is_fatal_auth_status)
}

fn daemon_auth_rejection_message(context: &str) -> String {
    format!("{context}. Run `cowboy login <Cowboy URL>` and try again.")
}

fn daemon_auth_rejected(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<WsError>()
            .is_some_and(websocket_auth_rejected)
            || cause.to_string().contains("rejected this computer login")
            // A person refused this computer. Retrying would file a new
            // approval request every few seconds for them to refuse again.
            || cause.to_string().contains("device authorization was denied")
    })
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
    // Only the global session index: with the complete bootstrap every
    // connection downloaded every session's transcript tail and had to finish
    // within BOOTSTRAP_TIMEOUT. One session is hydrated when it is loaded.
    url.query_pairs_mut().append_pair("bootstrap", "lazy");
    Ok(url)
}

fn url_request_target(url: &Url) -> String {
    url.query().map_or_else(
        || url.path().to_owned(),
        |query| format!("{}?{query}", url.path()),
    )
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

    #[tokio::test]
    async fn new_session_sends_explicit_machine_and_preserves_remote_rejection() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        use axum::{Json, Router, http::StatusCode, routing::post};
        let (requests, mut received) = mpsc::unbounded_channel();
        let native_requests = requests.clone();
        let app = Router::new()
            .route(
                "/api/sessions",
                post(move |Json(body): Json<serde_json::Value>| {
                    let requests = requests.clone();
                    async move {
                        requests.send(body.clone()).unwrap();
                        if body["machine_id"] != "local" {
                            let workspaces = [crate::machine_protocol::MachineWorkspace {
                                id: "matrix".to_owned(),
                                display_name: "Matrix".to_owned(),
                                canonical_path: "/home/ubuntu/matrix".to_owned(),
                            }];
                            let Some(requested) = body["cwd"]
                                .as_str()
                                .filter(|value| !value.trim().is_empty())
                            else {
                                return (
                                    StatusCode::BAD_REQUEST,
                                    "remote session requires a trusted workspace id".to_owned(),
                                );
                            };
                            if !workspaces.iter().any(|workspace| workspace.id == requested) {
                                return (
                                    StatusCode::BAD_REQUEST,
                                    format!("unknown trusted workspace {requested:?}"),
                                );
                            }
                            if body["provider"] == "async-fixture" {
                                return (
                                    StatusCode::CREATED,
                                    r#"{"session_id":"async-created"}"#.to_owned(),
                                );
                            }
                        }
                        (
                            StatusCode::SERVICE_UNAVAILABLE,
                            "Machine ovh is offline".to_owned(),
                        )
                    }
                }),
            )
            .route(
                "/api/execution-sessions",
                post(move |Json(body): Json<serde_json::Value>| {
                    let native_requests = native_requests.clone();
                    async move {
                        native_requests.send(body).unwrap();
                        (
                            StatusCode::SERVICE_UNAVAILABLE,
                            "native Remote environment is offline",
                        )
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (command_tx, _command_rx) = mpsc::unbounded_channel();
        let mut bridge = Bridge {
            provider: Arc::from("grok"),
            machine: Arc::from("ovh"),
            runtime_machine: None,
            workspace: None,
            authentication: crate::client_auth_client::ClientAuthentication::fixture(
                base_url.clone(),
            ),
            base_url,
            http: reqwest::Client::new(),
            state: Arc::new(Mutex::new(BridgeState::default())),
            connected_notify: Arc::new(Notify::new()),
            command_tx,
            next_cmid: Arc::new(AtomicU64::new(1)),
        };
        bridge.state.lock().connected = true;
        assert!(
            bridge
                .create_session(PathBuf::from("/client/project"))
                .await
                .unwrap_err()
                .contains("requires --workspace")
        );
        assert!(received.try_recv().is_err());
        for machine in ["ovh", "local"] {
            bridge.machine = Arc::from(machine);
            bridge.workspace = (machine != "local").then(|| Arc::from("matrix"));
            let error = bridge
                .create_session(PathBuf::from("/workspace"))
                .await
                .unwrap_err();
            assert!(error.contains("Machine ovh is offline"));
            let request = received.recv().await.unwrap();
            assert_eq!(request["machine_id"], machine);
            assert_eq!(request["provider"], "grok");
            assert_eq!(
                request["cwd"],
                if machine == "local" {
                    "/workspace"
                } else {
                    "matrix"
                }
            );
            assert!(
                received.try_recv().is_err(),
                "must not fall back to another Machine"
            );
        }
        bridge.machine = Arc::from("ovh");
        bridge.workspace = Some(Arc::from("/home/ubuntu/matrix"));
        assert!(
            bridge
                .create_session(PathBuf::from("/client/project"))
                .await
                .unwrap_err()
                .contains("unknown trusted workspace")
        );
        assert_eq!(received.recv().await.unwrap()["cwd"], "/home/ubuntu/matrix");
        assert!(received.try_recv().is_err());

        bridge.workspace = Some(Arc::from("matrix"));
        let mut remote = fixture_session_meta();
        remote.provider = "grok".to_owned();
        remote.machine_id = "ovh".to_owned();
        remote.workspace_id = Some("matrix".to_owned());
        remote.cwd = "/home/ubuntu/.local/state/cowboy-machine/worktrees/session".to_owned();
        for (id, machine, workspace, provider) in [
            ("remote", "ovh", "matrix", "grok"),
            ("other-machine", "hawk", "matrix", "grok"),
            ("other-workspace", "ovh", "columbus", "grok"),
            ("other-provider", "ovh", "matrix", "codex"),
        ] {
            let mut meta = remote.clone();
            meta.id = id.to_owned();
            meta.machine_id = machine.to_owned();
            meta.workspace_id = Some(workspace.to_owned());
            meta.provider = provider.to_owned();
            bridge.state.lock().sessions.insert(
                id.to_owned(),
                CachedSession {
                    meta: Some(meta),
                    ..Default::default()
                },
            );
        }
        let client_cwd = PathBuf::from("/client/project");
        let listed = bridge.list_sessions(Some(&client_cwd));
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id.0.as_ref(), "remote");
        assert_eq!(listed[0].cwd, client_cwd);
        assert!(bridge.validate_session("remote", Some(&client_cwd)).is_ok());
        for id in ["other-machine", "other-workspace", "other-provider"] {
            assert!(bridge.validate_session(id, Some(&client_cwd)).is_err());
        }

        // A fresh stdio client can list/load immediately after initialize,
        // before the independently connected WebSocket has its bootstrap.
        {
            let mut cold = bridge.clone();
            cold.state = Arc::new(Mutex::new(BridgeState::default()));
            cold.connected_notify = Arc::new(Notify::new());
            let listing = cold.ready_sessions(Some(&client_cwd));
            let loading = cold.validate_ready_session("remote", Some(&client_cwd));
            tokio::pin!(listing, loading);
            assert!(
                tokio::time::timeout(Duration::from_millis(10), &mut listing)
                    .await
                    .is_err()
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(10), &mut loading)
                    .await
                    .is_err()
            );
            cold.state.lock().sessions = bridge
                .state
                .lock()
                .sessions
                .iter()
                .map(|(id, cached)| {
                    (
                        id.clone(),
                        CachedSession {
                            meta: cached.meta.clone(),
                            ..Default::default()
                        },
                    )
                })
                .collect();
            cold.set_connected(true);
            assert_eq!(listing.await.unwrap()[0].session_id.0.as_ref(), "remote");
            loading.await.unwrap();
            assert!(
                cold.validate_ready_session("missing", Some(&client_cwd))
                    .await
                    .is_err()
            );
            assert!(
                cold.validate_ready_session("other-machine", Some(&client_cwd))
                    .await
                    .is_err()
            );
        }

        bridge.provider = Arc::from("async-fixture");
        let created = bridge.create_session(client_cwd.clone()).await.unwrap();
        assert_eq!(created, "async-created");
        assert_eq!(received.recv().await.unwrap()["cwd"], "matrix");
        let mut preparing = remote.clone();
        preparing.id = created.clone();
        preparing.status = Status::Starting;
        bridge.state.lock().sessions.insert(
            created.clone(),
            CachedSession {
                meta: Some(preparing),
                ..Default::default()
            },
        );
        let asynchronous = bridge.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let mut state = asynchronous.state.lock();
            let cached = state.sessions.get_mut("async-created").unwrap();
            cached.meta.as_mut().unwrap().status = Status::Crashed;
            cached.last_detail =
                Some("Machine ovh disconnected during workspace preparation".to_owned());
        });
        let error = tokio::time::timeout(
            Duration::from_secs(1),
            bridge.wait_for_initial_config_options(&created),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(error.contains("disconnected during workspace preparation"));
        {
            let mut state = bridge.state.lock();
            state
                .sessions
                .get_mut(&created)
                .unwrap()
                .meta
                .as_mut()
                .unwrap()
                .status = Status::Running;
        }
        let delayed_options = bridge.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            delayed_options
                .state
                .lock()
                .config_options
                .insert("async-created".to_owned(), Vec::new());
        });
        assert_eq!(
            bridge
                .wait_for_initial_config_options(&created)
                .await
                .unwrap(),
            Some(Vec::new())
        );
        bridge.provider = Arc::from("grok");
        bridge.machine = Arc::from("local");
        bridge.workspace = None;
        assert!(bridge.list_sessions(Some(&client_cwd)).is_empty());
        assert!(
            bridge
                .validate_session("remote", Some(&client_cwd))
                .is_err()
        );
        bridge.machine = Arc::from("hawk");
        bridge.runtime_machine = Some(Arc::from("ovh"));
        bridge.workspace = Some(Arc::from("cowboy"));
        let error = bridge.create_session(client_cwd.clone()).await.unwrap_err();
        assert!(error.contains("native Remote environment is offline"));
        let request = received.recv().await.unwrap();
        assert_eq!(request["machine_id"], "hawk");
        assert_eq!(request["runtime_machine_id"], "ovh");
        assert_eq!(request["cwd"], "cowboy");
        assert!(request.get("system").is_none());
        assert!(
            received.try_recv().is_err(),
            "must not fall back to local creation"
        );

        let mut native = remote;
        native.cwd = "/runtime/session".to_owned();
        native.workspace_id = Some("cowboy".to_owned());
        native.execution_binding = Some(crate::execution_environment::fixture());
        assert!(bridge.matches_remote_workspace(&native));
        for pointer in [
            "/environment/machine_id",
            "/runtime/machine_id",
            "/workspace/id",
        ] {
            let mut other = native.clone();
            let mut record = other.execution_binding.as_ref().unwrap().record().clone();
            *record.pointer_mut(pointer).unwrap() = serde_json::json!("other");
            other.execution_binding = Some(
                crate::execution_environment::ExecutionBinding::from_record(record),
            );
            assert!(!bridge.matches_remote_workspace(&other), "{pointer}");
        }
        native.execution_binding = None;
        assert!(
            !bridge.matches_remote_workspace(&native),
            "unbound sessions are not Remote sessions"
        );
        let ready = crate::execution_environment::fixture().decode().unwrap();
        let pending = crate::execution_environment::PreparationV1 {
            schema: 1,
            phase: "preparing".to_owned(),
            runtime: ready.runtime,
            machine_id: ready.environment.machine_id,
            workspace_id: ready.workspace.id,
            source_path: ready.workspace.source_path,
            executor_digest: ready.environment.executor_digest,
        };
        let pending = serde_json::to_value(pending).unwrap();
        native.execution_binding = Some(
            crate::execution_environment::ExecutionBinding::from_record(pending.clone()),
        );
        assert!(bridge.matches_remote_workspace(&native));
        assert!(native.execution_binding.as_ref().unwrap().decode().is_err());
        for pointer in [
            "/runtime/machine_id",
            "/runtime/cwd",
            "/machine_id",
            "/workspace_id",
            "/phase",
        ] {
            let mut other = native.clone();
            let mut record = pending.clone();
            *record.pointer_mut(pointer).unwrap() = serde_json::json!("other");
            other.execution_binding = Some(
                crate::execution_environment::ExecutionBinding::from_record(record),
            );
            assert!(!bridge.matches_remote_workspace(&other), "{pointer}");
        }
        server.abort();
    }

    #[test]
    fn initial_config_wait_outlives_provider_handshake() {
        assert!(INITIAL_CONFIG_TIMEOUT > STARTUP_PHASE_TIMEOUT);
        assert!(REMOTE_CREATION_TIMEOUT > crate::machine_control::WORKSPACE_ADAPTER_TIMEOUT);
    }

    #[test]
    fn stop_reasons_accept_hub_debug_and_wire_names() {
        assert_eq!(parse_stop_reason("EndTurn"), Some(StopReason::EndTurn));
        assert_eq!(parse_stop_reason("max_tokens"), Some(StopReason::MaxTokens));
        assert_eq!(parse_stop_reason("Cancelled"), Some(StopReason::Cancelled));
        assert_eq!(parse_stop_reason("error: adapter died"), None);
    }

    #[test]
    fn auth_rejection_statuses_are_fatal() {
        assert!(is_fatal_auth_status(401));
        assert!(is_fatal_auth_status(403));
        assert!(!is_fatal_auth_status(502));
        assert!(!is_fatal_auth_status(101));
        let error = anyhow!(daemon_auth_rejection_message(
            "cowboy daemon rejected this computer login"
        ));
        assert!(daemon_auth_rejected(&error));
        assert!(!daemon_auth_rejected(&anyhow!("connection refused")));
        assert!(daemon_auth_rejected(&anyhow!(
            "Cowboy device authorization was denied"
        )));
        assert!(!daemon_auth_rejected(&anyhow!(
            "Cowboy device authorization timed out"
        )));
    }

    #[test]
    fn websocket_url_preserves_prefix_and_switches_scheme() {
        let base =
            crate::client_auth_client::normalized_base_url("https://example.test/cowboy").unwrap();
        assert_eq!(
            websocket_url(&base).unwrap().as_str(),
            "wss://example.test/cowboy/ws?bootstrap=lazy"
        );
    }

    fn fixture_session_meta() -> SessionMeta {
        SessionMeta {
            continued_from: None,
            forked_from: None,
            id: "sess-1".to_owned(),
            provider: "codex".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "local".to_owned(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: "/tmp".to_owned(),
            title: "test".to_owned(),
            status: Status::Busy,
            origin: crate::core::SessionOrigin::Api,
            agent_session_id: None,
            paused: false,
            closing: false,
            system: false,
            context_used: 0,
            context_size: 0,
            usage: None,
            background_tasks: 0,
            supports_current_turn: false,
            provider_update: None,
            provider_update_available: None,
            resource_wait: None,
            next_schedule_ms: None,
            owner_user_id: None,
            owner_username: None,
            trashed_at_ms: None,
        }
    }

    #[test]
    fn status_never_claims_background_idle() {
        let meta = fixture_session_meta();
        let status = Bridge::status_from_cached(&meta, None);
        assert!(status.turn_running);
        assert_eq!(status.background_running, None);
    }

    fn fixture_bridge() -> (Bridge, mpsc::UnboundedReceiver<Inbound>) {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let base_url = Url::parse("http://127.0.0.1:9/").unwrap();
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let bridge = Bridge {
            provider: Arc::from("codex"),
            machine: Arc::from("local"),
            runtime_machine: None,
            workspace: None,
            authentication: crate::client_auth_client::ClientAuthentication::fixture(
                base_url.clone(),
            ),
            base_url,
            http: reqwest::Client::new(),
            state: Arc::new(Mutex::new(BridgeState::default())),
            connected_notify: Arc::new(Notify::new()),
            command_tx,
            next_cmid: Arc::new(AtomicU64::new(1)),
        };
        (bridge, command_rx)
    }

    fn envelope(seq: u64, event: Event) -> Envelope {
        Envelope {
            session_id: "sess-1".to_owned(),
            seq,
            cmid: None,
            event,
        }
    }

    fn text_update(text: &str) -> Event {
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": text}
            }),
        }
    }

    fn permission_request(request_id: &str) -> Event {
        Event::PermissionRequest {
            request_id: request_id.to_owned(),
            tool_call: serde_json::json!({"toolCallId": request_id}),
            options: serde_json::json!([]),
        }
    }

    fn listed(bridge: &Bridge, attached: bool) {
        let mut state = bridge.state.lock();
        state
            .apply_session_list(vec![fixture_session_meta()])
            .settle();
        state.sessions.get_mut("sess-1").unwrap().attached = attached;
    }

    fn start_prompt(bridge: &Bridge) -> (String, oneshot::Receiver<PromptOutcome>) {
        let (cmid, completion) = bridge.register_prompt("sess-1");
        let mut echo = envelope(1, text_update("prompt"));
        echo.cmid = Some(cmid.clone());
        assert!(bridge.state.lock().correlate_prompt(&echo).0.is_none());
        (cmid, completion)
    }

    #[tokio::test]
    async fn a_pending_browser_approval_answers_requests_with_its_link() {
        let (bridge, _commands) = fixture_bridge();
        let waiting = bridge.clone();
        let listing = tokio::spawn(async move { waiting.ready_sessions(None).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!listing.is_finished(), "a connecting daemon is waited for");

        bridge.authentication.set_pending_approval(Some(
            crate::client_auth_client::PendingApproval {
                url: "https://cowboy.example/auth/device#request".to_owned(),
                fingerprint: "SHA256:fixture".to_owned(),
            },
        ));
        let error = tokio::time::timeout(Duration::from_secs(2), listing)
            .await
            .expect("the approval link is reported without the readiness timeout")
            .unwrap()
            .unwrap_err();
        assert!(error.contains("https://cowboy.example/auth/device#request"));
        assert!(error.contains("SHA256:fixture"));

        // Once approved and connected the same request succeeds.
        bridge.authentication.set_pending_approval(None);
        bridge.set_connected(true);
        assert!(bridge.ready_sessions(None).await.unwrap().is_empty());
    }

    #[test]
    fn daemon_connection_negotiates_the_lazy_bootstrap() {
        let base = crate::client_auth_client::normalized_base_url("http://127.0.0.1:3333").unwrap();
        let url = websocket_url(&base).unwrap();
        assert_eq!(url.as_str(), "ws://127.0.0.1:3333/ws?bootstrap=lazy");
        assert_eq!(url_request_target(&url), "/ws?bootstrap=lazy");
    }

    /// A daemon that serves only the lazy protocol: the global session index
    /// on the WebSocket and one session's transcript over HTTP.
    #[tokio::test]
    async fn lazy_daemon_connection_hydrates_a_session_on_load_and_after_a_reconnect() {
        use axum::extract::ws::{Message as AxumMessage, WebSocketUpgrade};
        use axum::extract::{Path, Query};
        use axum::{Json, Router, routing::get};

        let (bridge, _commands) = fixture_bridge();
        let (history_requests, mut history_seen) = mpsc::unbounded_channel::<u64>();
        let app = Router::new()
            .route(
                "/ws",
                get(
                    |Query(query): Query<HashMap<String, String>>, ws: WebSocketUpgrade| async move {
                        assert_eq!(query.get("bootstrap").map(String::as_str), Some("lazy"));
                        ws.on_upgrade(|mut socket| async move {
                            for frame in [
                                serde_json::to_string(&Outbound::Sessions {
                                    sessions: vec![fixture_session_meta()],
                                })
                                .unwrap(),
                                serde_json::json!({"type": "a_frame_from_a_newer_daemon"})
                                    .to_string(),
                                serde_json::to_string(&Outbound::BootstrapComplete).unwrap(),
                            ] {
                                socket.send(AxumMessage::Text(frame.into())).await.unwrap();
                            }
                            // Hold the socket open until the bridge drops it.
                            while socket.recv().await.is_some() {}
                        })
                    },
                ),
            )
            .route(
                "/api/sessions/{id}/bootstrap",
                get(|Path(id): Path<String>| async move {
                    Json(serde_json::json!({"messages": [
                        Outbound::Snapshot {
                            session_id: id.clone(),
                            events: vec![envelope(5, text_update("e")), envelope(6, text_update("f"))],
                            reached_start: false,
                        },
                        Outbound::ConfigOptions { session_id: id, options: serde_json::json!([]) },
                        {"type": "a_frame_from_a_newer_daemon"},
                    ]}))
                }),
            )
            .route(
                "/api/history/{id}",
                get(move |Query(query): Query<HashMap<String, String>>| {
                    let history_requests = history_requests.clone();
                    async move {
                        let before_seq = query["before_seq"].parse::<u64>().unwrap();
                        history_requests.send(before_seq).unwrap();
                        let first = before_seq - 2;
                        Json(serde_json::json!({
                            "events": [
                                envelope(first, text_update("older")),
                                envelope(first + 1, text_update("older")),
                            ],
                            "next_before_seq": first,
                            "reached_start": first == 1,
                        }))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let bridge = Bridge {
            authentication: crate::client_auth_client::ClientAuthentication::fixture(
                base_url.clone(),
            ),
            base_url,
            ..bridge
        };

        // The unknown frame must not fail the bootstrap.
        let socket = bridge.connect_daemon().await.unwrap();
        assert!(bridge.knows_session("sess-1"));
        assert!(
            bridge.state.lock().sessions["sess-1"].events.is_empty(),
            "the connection carries no transcript"
        );

        bridge.attach("sess-1", true);
        let history = bridge.load_history("sess-1").await.unwrap();
        assert_eq!(
            history.iter().map(|event| event.seq).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6]
        );
        assert_eq!(bridge.config_options("sess-1"), Some(Vec::new()));
        assert_eq!(history_seen.recv().await, Some(5));
        assert_eq!(history_seen.recv().await, Some(3));
        assert!(history_seen.try_recv().is_err());

        // After a reconnect only the gap behind the transcript tail is paged.
        let missed = bridge.fetch_missed("sess-1", Some(3)).await.unwrap();
        let (events, _) = missed.snapshot.unwrap();
        assert_eq!(
            events.iter().map(|event| event.seq).collect::<Vec<_>>(),
            [3, 4, 5, 6]
        );
        assert_eq!(history_seen.recv().await, Some(5));
        assert!(history_seen.try_recv().is_err());
        let missed = bridge.fetch_missed("sess-1", Some(5)).await.unwrap();
        assert_eq!(missed.snapshot.unwrap().0.len(), 2);
        assert!(
            history_seen.try_recv().is_err(),
            "the tail already covers the gap"
        );

        drop(socket);
        server.abort();
    }

    #[test]
    fn hydration_reads_known_frames_and_skips_the_rest() {
        let hydration = SessionHydration::from_messages(vec![
            serde_json::json!({"type": "a_frame_from_a_newer_daemon", "value": 1}),
            serde_json::to_value(Outbound::Snapshot {
                session_id: "sess-1".to_owned(),
                events: vec![envelope(8, text_update("b")), envelope(7, text_update("a"))],
                reached_start: false,
            })
            .unwrap(),
            serde_json::to_value(Outbound::ConfigOptions {
                session_id: "sess-1".to_owned(),
                options: serde_json::json!([]),
            })
            .unwrap(),
        ]);
        let (events, reached_start) = hydration.snapshot.unwrap();
        assert_eq!(
            events.iter().map(|event| event.seq).collect::<Vec<_>>(),
            [7, 8]
        );
        assert!(!reached_start);
        assert_eq!(hydration.config_options, Some(Vec::new()));
    }

    #[tokio::test]
    async fn session_deleted_in_cowboy_releases_its_state_and_fails_its_prompts() {
        let (bridge, _commands) = fixture_bridge();
        listed(&bridge, true);
        let (_cmid, completion) = bridge.register_prompt("sess-1");
        bridge
            .state
            .lock()
            .config_options
            .insert("sess-1".to_owned(), Vec::new());

        let change = bridge.state.lock().apply_session_list(Vec::new());
        assert!(change.settle().is_empty());

        assert!(matches!(
            completion.await.unwrap(),
            PromptOutcome::Failed(error) if error.contains("deleted in Cowboy")
        ));
        {
            let state = bridge.state.lock();
            assert!(state.sessions.is_empty());
            assert!(state.config_options.is_empty());
            assert!(state.pending_prompts.is_empty());
        }
        assert!(!bridge.knows_session("sess-1"));
    }

    #[tokio::test]
    async fn requested_deletion_is_acknowledged_by_the_session_list() {
        let (bridge, _commands) = fixture_bridge();
        listed(&bridge, true);
        let deleted = bridge.register_delete_waiter("sess-1");
        let (_cmid, completion) = bridge.register_prompt("sess-1");

        // A list that still contains the session acknowledges nothing.
        bridge
            .state
            .lock()
            .apply_session_list(vec![fixture_session_meta()])
            .settle();
        assert!(bridge.state.lock().delete_waiters.contains_key("sess-1"));

        // An accepted deletion that still waits for its execution environment
        // is published as `closing`.
        let mut closing = fixture_session_meta();
        closing.closing = true;
        bridge
            .state
            .lock()
            .apply_session_list(vec![closing])
            .settle();
        deleted.await.unwrap().unwrap();
        assert!(matches!(
            completion.await.unwrap(),
            PromptOutcome::Stopped(StopReason::Cancelled)
        ));
        assert!(bridge.list_sessions(None).is_empty());
        assert!(bridge.validate_session("sess-1", None).is_err());
    }

    #[tokio::test]
    async fn refused_deletion_reports_the_daemon_error() {
        let (bridge, _commands) = fixture_bridge();
        listed(&bridge, true);
        let deleted = bridge.register_delete_waiter("sess-1");
        let (_cmid, mut completion) = bridge.register_prompt("sess-1");

        bridge.fail_refused("sess-1", "command failed: stop is unconfirmed");

        assert_eq!(
            deleted.await.unwrap().unwrap_err(),
            "command failed: stop is unconfirmed"
        );
        assert!(
            completion.try_recv().is_err(),
            "the prompt is not the refused command"
        );
        assert!(bridge.knows_session("sess-1"));
    }

    #[test]
    fn session_list_keeps_a_created_session_until_the_daemon_lists_it() {
        let (bridge, _commands) = fixture_bridge();
        bridge.attach("created", false);
        bridge
            .state
            .lock()
            .apply_session_list(vec![fixture_session_meta()])
            .settle();
        assert!(bridge.state.lock().sessions.contains_key("created"));
    }

    #[test]
    fn retitled_session_is_reported_only_while_attached() {
        let (bridge, _commands) = fixture_bridge();
        listed(&bridge, false);
        let mut renamed = fixture_session_meta();
        renamed.title = "Fix the bridge".to_owned();
        let change = bridge
            .state
            .lock()
            .apply_session_list(vec![renamed.clone()]);
        assert!(change.settle().is_empty());

        bridge.attach("sess-1", false);
        let change = bridge
            .state
            .lock()
            .apply_session_list(vec![renamed.clone()]);
        assert!(
            change.settle().is_empty(),
            "an unchanged title is not republished"
        );
        renamed.title = "Fix the Zed bridge".to_owned();
        let change = bridge.state.lock().apply_session_list(vec![renamed]);
        assert_eq!(
            change.settle(),
            [("sess-1".to_owned(), "Fix the Zed bridge".to_owned())]
        );
    }

    #[tokio::test]
    async fn another_surface_error_never_fails_a_running_turn() {
        let (bridge, _commands) = fixture_bridge();
        listed(&bridge, true);
        let (_running, mut running) = start_prompt(&bridge);
        let (_queued, queued) = bridge.register_prompt("sess-1");

        bridge.fail_refused("sess-1", "command failed: unknown permission request");

        assert!(running.try_recv().is_err());
        assert!(matches!(
            queued.await.unwrap(),
            PromptOutcome::Failed(error) if error.contains("unknown permission request")
        ));
        // With nothing left undispatched the running turn still stays pending.
        bridge.fail_refused("sess-1", "command failed: again");
        assert!(running.try_recv().is_err());
    }

    #[tokio::test]
    async fn addressed_refusal_fails_exactly_the_named_prompt() {
        let (bridge, _commands) = fixture_bridge();
        listed(&bridge, true);
        let (_first, mut first) = bridge.register_prompt("sess-1");
        let (second_cmid, second) = bridge.register_prompt("sess-1");

        bridge.fail_prompt("sess-1", "another-device-cmid", "ignored".to_owned());
        bridge.fail_prompt(
            "sess-1",
            &second_cmid,
            "this session no longer exists".to_owned(),
        );

        assert!(first.try_recv().is_err());
        assert!(matches!(
            second.await.unwrap(),
            PromptOutcome::Failed(error) if error == "this session no longer exists"
        ));
    }

    #[test]
    fn unresolved_permission_requests_follow_the_web_pending_rule() {
        let resolved = |request_id: &str| Event::PermissionResolved {
            request_id: request_id.to_owned(),
            option_id: Some("allow".to_owned()),
        };
        let tool_update = |tool: &str, status: &str| Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": tool,
                "status": status
            }),
        };
        let lifecycle = |status: Status| Event::Lifecycle {
            status,
            detail: None,
        };
        let seqs = |events: &[Envelope]| {
            unresolved_permission_requests(events)
                .into_iter()
                .map(|envelope| envelope.seq)
                .collect::<Vec<_>>()
        };
        let mut events = vec![
            envelope(1, permission_request("r-1")),
            envelope(2, resolved("r-1")),
            envelope(3, permission_request("r-2")),
            envelope(4, text_update("still waiting")),
            envelope(5, lifecycle(Status::Busy)),
        ];
        assert_eq!(seqs(&events), [3]);

        // The agent restarted: its request ids start over, and the earlier
        // asker of a reused id is gone.
        events.push(envelope(6, permission_request("r-2")));
        events.push(envelope(7, permission_request("r-3")));
        assert_eq!(seqs(&events), [6, 7]);

        // A tool call that ran is no longer asking, even without its
        // resolution row; one that is still in progress is.
        events.push(envelope(8, tool_update("r-3", "in_progress")));
        assert_eq!(seqs(&events), [6, 7]);
        events.push(envelope(9, tool_update("r-3", "completed")));
        assert_eq!(seqs(&events), [6]);

        events.push(envelope(
            10,
            Event::TurnEnd {
                usage: None,
                stop_reason: "Cancelled".to_owned(),
            },
        ));
        assert!(seqs(&events).is_empty());
        events.push(envelope(11, permission_request("r-4")));
        events.push(envelope(12, lifecycle(Status::Running)));
        assert_eq!(seqs(&events), [11]);
        events.push(envelope(13, lifecycle(Status::Interrupted)));
        assert!(seqs(&events).is_empty());
    }

    #[tokio::test]
    async fn reconnect_catch_up_delivers_missed_events_once_and_settles_the_turn() {
        let (bridge, _commands) = fixture_bridge();
        listed(&bridge, true);
        let (_cmid, completion) = start_prompt(&bridge);
        {
            let mut state = bridge.state.lock();
            let cached = state.sessions.get_mut("sess-1").unwrap();
            cached.events = vec![
                envelope(1, text_update("prompt")),
                envelope(2, text_update("a")),
            ];
            cached.last_seq = Some(2);
        }

        // The connection drops; the turn finishes while it is down.
        assert_eq!(bridge.begin_catch_up(), [("sess-1".to_owned(), Some(2))]);
        // A second drop before the hydration returns resumes the same gap.
        assert_eq!(bridge.begin_catch_up(), [("sess-1".to_owned(), Some(2))]);
        // A live event that overlaps the hydration is buffered, not forwarded.
        {
            let mut state = bridge.state.lock();
            let cached = state.sessions.get_mut("sess-1").unwrap();
            assert!(cached.replaying);
            cached.replay_buffer.push(envelope(6, text_update("live")));
            cached.events.push(envelope(6, text_update("live")));
            cached.last_seq = Some(6);
            cached.meta.as_mut().unwrap().status = Status::Running;
        }
        let hydration = SessionHydration {
            snapshot: Some((
                vec![
                    envelope(2, text_update("a")),
                    envelope(3, text_update("b")),
                    envelope(4, permission_request("answered-on-the-phone")),
                    envelope(
                        5,
                        Event::PermissionResolved {
                            request_id: "answered-on-the-phone".to_owned(),
                            option_id: Some("allow".to_owned()),
                        },
                    ),
                    envelope(6, text_update("live")),
                ],
                false,
            )),
            config_options: Some(Vec::new()),
        };

        let catch_up = bridge
            .state
            .lock()
            .complete_catch_up("sess-1", hydration)
            .unwrap();

        assert_eq!(
            catch_up
                .events
                .iter()
                .map(|event| event.seq)
                .collect::<Vec<_>>(),
            [3, 5, 6],
            "already delivered, answered and duplicated events are not replayed"
        );
        assert_eq!(catch_up.config_options, Some(Vec::new()));
        assert_eq!(catch_up.status.unwrap().state, "running");
        assert_eq!(catch_up.settled.len(), 1);
        for (completion, outcome) in catch_up.settled {
            let _ = completion.send(outcome);
        }
        assert!(matches!(
            completion.await.unwrap(),
            PromptOutcome::Stopped(StopReason::EndTurn)
        ));
        {
            let state = bridge.state.lock();
            let cached = state.sessions.get("sess-1").unwrap();
            assert!(!cached.replaying);
            assert_eq!(cached.last_seq, Some(6));
            assert_eq!(
                cached
                    .events
                    .iter()
                    .map(|event| event.seq)
                    .collect::<Vec<_>>(),
                [2, 3, 4, 5, 6]
            );
        }
        assert_eq!(
            bridge.begin_catch_up(),
            [("sess-1".to_owned(), Some(6))],
            "a later reconnect catches up from the new position"
        );
    }

    #[tokio::test]
    async fn catch_up_reports_the_missed_turn_end_and_keeps_a_running_turn() {
        let (bridge, _commands) = fixture_bridge();
        listed(&bridge, true);
        let (_cmid, completion) = start_prompt(&bridge);
        bridge.begin_catch_up();
        let hydration = SessionHydration {
            snapshot: Some((
                vec![envelope(
                    9,
                    Event::TurnEnd {
                        usage: None,
                        stop_reason: "max_tokens".to_owned(),
                    },
                )],
                true,
            )),
            config_options: None,
        };
        let catch_up = bridge
            .state
            .lock()
            .complete_catch_up("sess-1", hydration)
            .unwrap();
        for (completion, outcome) in catch_up.settled {
            let _ = completion.send(outcome);
        }
        assert!(matches!(
            completion.await.unwrap(),
            PromptOutcome::Stopped(StopReason::MaxTokens)
        ));

        // The session is still busy with a turn this bridge dispatched: a
        // failed hydration must not end it.
        let (_cmid, mut running) = start_prompt(&bridge);
        bridge.begin_catch_up();
        let catch_up = bridge
            .state
            .lock()
            .complete_catch_up("sess-1", SessionHydration::default())
            .unwrap();
        assert!(catch_up.settled.is_empty());
        assert!(running.try_recv().is_err());
    }

    #[test]
    fn catch_up_yields_to_a_detach_or_a_client_reload() {
        let (bridge, _commands) = fixture_bridge();
        listed(&bridge, true);
        bridge.begin_catch_up();
        bridge.attach("sess-1", true);
        assert!(
            bridge
                .state
                .lock()
                .complete_catch_up("sess-1", SessionHydration::default())
                .is_none()
        );
        assert!(
            bridge.state.lock().sessions["sess-1"].replaying,
            "the reload still owns the replay"
        );
        assert!(bridge.begin_catch_up().is_empty());

        bridge.state.lock().detach_session("sess-1");
        assert!(bridge.begin_catch_up().is_empty());
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

    fn subagent_snapshot(revision: u64, state: &str) -> serde_json::Value {
        serde_json::json!({
            "sessionUpdate": "cowboy_subagent",
            "sessionId": "child",
            "revision": revision,
            "title": "Explore",
            "state": state,
            "cancellable": state == "running",
            "activity": [],
        })
    }

    #[test]
    fn subagent_snapshots_follow_the_client_capability_and_a_reload() {
        let (bridge, _commands) = fixture_bridge();
        listed(&bridge, true);
        let kinds = |bridge: &Bridge, revision| {
            bridge
                .client_updates("sess-1", &subagent_snapshot(revision, "running"))
                .into_iter()
                .map(|notification| {
                    serde_json::to_value(notification).unwrap()["update"]["sessionUpdate"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect::<Vec<_>>()
        };
        // A client without subagents sees a tool call, once.
        assert_eq!(kinds(&bridge, 1), ["tool_call"]);
        assert!(kinds(&bridge, 1).is_empty());
        // A reloaded thread starts over: the replay announces it again.
        bridge.attach("sess-1", true);
        assert_eq!(kinds(&bridge, 1), ["tool_call"]);

        bridge.state.lock().subagent_client = true;
        bridge.attach("sess-1", true);
        assert_eq!(kinds(&bridge, 2), ["subagent_update"]);
        // Ordinary updates are unchanged; an unknown one is dropped.
        assert_eq!(bridge.client_updates("sess-1", &serde_json::json!({
            "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hi"},
        })).len(), 1);
        assert!(
            bridge
                .client_updates(
                    "sess-1",
                    &serde_json::json!({ "sessionUpdate": "cowboy_unknown" })
                )
                .is_empty()
        );
    }

    #[test]
    fn cancelling_an_announced_child_session_stops_that_subagent() {
        let (bridge, _commands) = fixture_bridge();
        listed(&bridge, true);
        bridge.state.lock().subagent_client = true;
        assert!(bridge.state.lock().subagent_parent("child").is_none());
        bridge.client_updates("sess-1", &subagent_snapshot(1, "running"));
        assert_eq!(
            bridge.state.lock().subagent_parent("child").as_deref(),
            Some("sess-1")
        );
        // The detached thread no longer owns its subagents.
        bridge.state.lock().detach_session("sess-1");
        assert!(bridge.state.lock().subagent_parent("child").is_none());
    }
}
