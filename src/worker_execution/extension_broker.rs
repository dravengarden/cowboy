//! The session extensions of an account-sandboxed session (Antigravity),
//! brokered by the worker (docs/antigravity-real-accounts-design.md,
//! "Session extensions", and docs/antigravity-user-mcp-r2-design.md).
//! The worker starts Cowboy MCP and the folder's bindings on the target from
//! its own launch template, as a Claude or Codex session runs them, discovers
//! their tools, and answers the unit's two requests: `cowboy/extensions` (the
//! tools) and `cowboy/extensionCall` (one `tools/call`, admitted only inside
//! the active turn, drained before the turn completes, and an uncertain effect
//! when its outcome is lost). Memory's tools go to the session's relay the
//! same way, limited to the signed tool list. Owner-authored `~/.claude.json`
//! user MCP servers run in Phase 2 when `ackProtocol >= 1` is advertised. The
//! unit never starts, writes to or reads these processes, nor sees their output.

mod user_mcp;

pub use user_mcp::{
    USER_MCP_INITIALIZATION_KEY, USER_MCP_LIST_METHOD, USER_MCP_PROCESS_PREFIX,
    USER_MCP_START_METHOD, USER_MCP_STOP_METHOD,
};

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine as _;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::{Mutex as AsyncMutex, watch};

use super::Client;
use super::ledger::{Ledger, SERVER_PREFIX, SettlementGuard};
use super::memory_bridge::MemoryBridge;
use super::relay::AckState;
use crate::execution_protocol::{Command, Response};
use user_mcp::{
    EXTENSION_ACK_TIMEOUT, KeeperSupport, PendingAckState, SweepState, USER_MCP_DISCOVERY_TIMEOUT,
    USER_MCP_SWEEP_TIMEOUT, UserDiscovery,
};

/// The unit's request listing the session's extension tools.
pub const EXTENSIONS_REQUEST: &str = "cowboy/extensions";
/// The unit's request running one extension tool.
pub const EXTENSION_CALL: &str = "cowboy/extensionCall";

const MCP_PROTOCOL: &str = "2025-06-18";
/// One deadline for all of Phase 1 discovery, required and optional alike.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(40);
/// Below the proxy connection's own request timer (185 s), so the worker
/// classifies a slow call before the unit gives up on it.
const CALL_TIMEOUT: Duration = Duration::from_secs(170);
const MAX_TOOLS: usize = 128;
const MAX_LINE: usize = 16 * 1024 * 1024;
const MAX_GUIDANCE: usize = 8;
/// The facade's own tools; an extension tool of the same name is aliased.
const FACADE_TOOLS: &[&str] = &[
    "ReadFile",
    "ListDirectory",
    "SearchFiles",
    "WriteFile",
    "EditFile",
    "RunCommand",
    "StopCommand",
];

/// One target MCP server the worker runs for the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerPlan {
    pub name: String,
    pub argv: Vec<String>,
    /// A folder binding the session requires (Claude and Codex refuse to
    /// start without one): its CLI must report the binding contract, and
    /// its failure refuses the session. Cowboy MCP is not required.
    pub required: bool,
}

/// The extension plan a sandboxed worker derived from its descriptor.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub servers: Vec<ServerPlan>,
    /// Memory's tools the signed configuration enables (empty: none).
    pub memory_tools: Vec<String>,
    pub memory_server: String,
    /// The installed managed-call guidance, unrendered.
    pub guidance: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Route {
    Server(usize),
    UserServer(usize),
    Memory,
}

#[derive(Clone)]
struct Tool {
    name: String,
    server: String,
    original: String,
    route: Route,
    description: Value,
    input_schema: Value,
}

#[derive(Clone, Default)]
struct Discovery {
    tools: Vec<Tool>,
    instructions: Vec<String>,
    /// Set when a required server failed: the session must not start.
    failed: Option<String>,
}

/// One stdio MCP server on the target, spoken to through the executor. One
/// exchange at a time; output is read only while a request waits for its
/// answer, so an idle server costs nothing.
struct Server {
    process: String,
    io: AsyncMutex<Io>,
    closed_hint: std::sync::atomic::AtomicBool,
}

impl Server {
    fn new(process: String) -> Self {
        Self {
            process,
            io: AsyncMutex::new(Io::default()),
            closed_hint: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn mark_closed(&self) {
        self.closed_hint
            .store(true, std::sync::atomic::Ordering::Release);
    }

    fn is_closed_hint(&self) -> bool {
        self.closed_hint.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[derive(Default)]
struct Io {
    after_seq: Option<u64>,
    buffer: Vec<u8>,
    next_id: u64,
    closed: bool,
}

/// A failed exchange.
enum Failure {
    /// Certainly not delivered: nothing can have happened.
    NotSent(String),
    /// The server answered with an error: a tool error, no unknown effect.
    Refused(String),
    /// Delivered, or possibly delivered, without an answer.
    MaybeSent(String),
}

impl Failure {
    fn text(self) -> String {
        match self {
            Self::NotSent(text) | Self::Refused(text) | Self::MaybeSent(text) => text,
        }
    }
}

pub struct Broker {
    servers: Vec<Arc<Server>>,
    phase1_server_names: Vec<String>,
    memory: Option<MemoryBridge>,
    memory_tools: Vec<String>,
    memory_server: String,
    guidance: Vec<String>,
    discovery: watch::Receiver<Option<Discovery>>,
    keeper_support: Mutex<KeeperSupport>,
    sweep_state: watch::Receiver<SweepState>,
    sweep_clean: AtomicBool,
    phase2_lock: AsyncMutex<()>,
    phase2_cache: Mutex<Option<UserDiscovery>>,
    stopping: Arc<AtomicBool>,
    user_servers: Mutex<Vec<Arc<Server>>>,
    pending_acks: Mutex<PendingAckState>,
    #[cfg(test)]
    ack_timeout_override: Option<Duration>,
    #[cfg(test)]
    phase2_timeout_override: Option<Duration>,
}

impl Broker {
    /// Start every planned server in the background; Phase 1 discovery completes
    /// independently and `cowboy/extensions` waits for it (bounded). Also runs
    /// an unconditional startup orphan sweep when the keeper advertises `cowboyUserMcp`.
    pub fn start(client: &Arc<Client>, plan: Plan, memory: Option<MemoryBridge>) -> Arc<Self> {
        Self::start_inner(client, plan, memory, None, None)
    }

    fn start_inner(
        client: &Arc<Client>,
        plan: Plan,
        memory: Option<MemoryBridge>,
        #[cfg_attr(not(test), allow(unused_variables))] ack_timeout_override: Option<Duration>,
        #[cfg_attr(not(test), allow(unused_variables))] phase2_timeout_override: Option<Duration>,
    ) -> Arc<Self> {
        let servers: Vec<Arc<Server>> = plan
            .servers
            .iter()
            .map(|_| {
                Arc::new(Server::new(format!(
                    "{SERVER_PREFIX}{:032x}",
                    rand::random::<u128>()
                )))
            })
            .collect();
        let memory = memory.filter(|_| !plan.memory_tools.is_empty());
        let phase1_server_names: Vec<String> =
            plan.servers.iter().map(|s| s.name.clone()).collect();
        let (sender, discovery) = watch::channel(None);
        let (sweep_tx, sweep_state) = watch::channel(SweepState::Pending);
        let broker = Arc::new(Self {
            servers,
            phase1_server_names,
            memory,
            memory_tools: plan.memory_tools.clone(),
            memory_server: plan.memory_server.clone(),
            guidance: plan.guidance.iter().take(MAX_GUIDANCE).cloned().collect(),
            discovery,
            keeper_support: Mutex::new(KeeperSupport::Unknown),
            sweep_state,
            sweep_clean: AtomicBool::new(false),
            phase2_lock: AsyncMutex::new(()),
            phase2_cache: Mutex::new(None),
            stopping: Arc::new(AtomicBool::new(false)),
            user_servers: Mutex::new(Vec::new()),
            pending_acks: Mutex::new(PendingAckState::default()),
            #[cfg(test)]
            ack_timeout_override,
            #[cfg(test)]
            phase2_timeout_override,
        });
        let client_p1 = Arc::clone(client);
        let discovering = Arc::clone(&broker);
        tokio::spawn(async move {
            let discovery =
                tokio::time::timeout(DISCOVERY_TIMEOUT, discovering.discover(&client_p1, &plan))
                    .await
                    .unwrap_or_else(|_| Discovery {
                        failed: plan
                            .servers
                            .iter()
                            .any(|server| server.required)
                            .then(|| "session extensions timed out".to_owned()),
                        ..Discovery::default()
                    });
            let _ = sender.send(Some(discovery));
        });

        let client_sweep = Arc::clone(client);
        let sweeping = Arc::clone(&broker);
        tokio::spawn(async move {
            if client_sweep.binding.managed.is_some() {
                *sweeping.keeper_support.lock() = KeeperSupport::Unsupported;
                sweeping.sweep_clean.store(true, Ordering::Release);
                let _ = sweep_tx.send(SweepState::Clean);
                return;
            }
            match tokio::time::timeout(USER_MCP_SWEEP_TIMEOUT, client_sweep.call(Command::Describe))
                .await
            {
                Ok(Ok(Response::Ready { initialization, .. })) => {
                    let supported = initialization
                        .get(USER_MCP_INITIALIZATION_KEY)
                        .and_then(|v| v.get("schema"))
                        .and_then(Value::as_u64)
                        == Some(1);
                    if !supported {
                        *sweeping.keeper_support.lock() = KeeperSupport::Unsupported;
                        sweeping.sweep_clean.store(true, Ordering::Release);
                        let _ = sweep_tx.send(SweepState::Clean);
                        return;
                    }
                    *sweeping.keeper_support.lock() = KeeperSupport::Supported;
                }
                _ => {
                    // Transient link/startup timeout: keep Unknown so Phase 2 retries
                    // on demand instead of permanently disabling user MCP.
                    tracing::warn!(
                        reason = "orphan_sweep_timeout",
                        "startup keeper user MCP probe incomplete; will retry on demand"
                    );
                    let _ = sweep_tx.send(SweepState::Incomplete);
                    return;
                }
            }
            let ok = match tokio::time::timeout(
                USER_MCP_SWEEP_TIMEOUT,
                client_sweep.invoke_classified(USER_MCP_STOP_METHOD.into(), json!({})),
            )
            .await
            {
                Ok(Ok(reply)) => reply.pointer("/result/failed").and_then(Value::as_u64) == Some(0),
                _ => false,
            };
            if ok {
                sweeping.sweep_clean.store(true, Ordering::Release);
                let _ = sweep_tx.send(SweepState::Clean);
            } else {
                tracing::warn!(
                    reason = "orphan_sweep_timeout",
                    "startup user MCP orphan sweep did not confirm clean"
                );
                let _ = sweep_tx.send(SweepState::Incomplete);
            }
        });
        broker
    }

    #[cfg(test)]
    pub(super) fn with_test_timeouts(
        client: &Arc<Client>,
        plan: Plan,
        ack_timeout: Option<Duration>,
        phase2_timeout: Option<Duration>,
    ) -> Arc<Self> {
        Self::start_inner(client, plan, None, ack_timeout, phase2_timeout)
    }

    fn phase2_timeout(&self) -> Duration {
        #[cfg(test)]
        if let Some(d) = self.phase2_timeout_override {
            return d;
        }
        USER_MCP_DISCOVERY_TIMEOUT
    }

    fn ack_timeout(&self) -> Duration {
        #[cfg(test)]
        if let Some(d) = self.ack_timeout_override {
            return d;
        }
        EXTENSION_ACK_TIMEOUT
    }

    async fn discover(&self, client: &Arc<Client>, plan: &Plan) -> Discovery {
        let servers = futures::future::join_all(
            self.servers
                .iter()
                .zip(&plan.servers)
                .map(|(server, plan)| server.initialize(client, plan)),
        );
        let memory = async {
            match &self.memory {
                Some(bridge) => Some(memory_tools(bridge).await),
                None => None,
            }
        };
        let (servers, memory) = tokio::join!(servers, memory);
        let mut discovery = Discovery::default();
        let mut taken: HashSet<String> =
            FACADE_TOOLS.iter().map(|name| (*name).to_owned()).collect();
        for (index, (result, plan)) in servers.into_iter().zip(&plan.servers).enumerate() {
            match result {
                Ok((instructions, tools)) => {
                    if !instructions.is_empty() {
                        discovery
                            .instructions
                            .push(format!("{}: {instructions}", plan.name));
                    }
                    add_tools(
                        &mut discovery,
                        &mut taken,
                        &plan.name,
                        Route::Server(index),
                        tools,
                    );
                }
                Err(failure) => {
                    tracing::warn!(server = %plan.name, reason = %failure, "session extension unavailable");
                    if plan.required {
                        discovery.failed = Some(format!("{} is unavailable: {failure}", plan.name));
                    } else {
                        discovery
                            .instructions
                            .push(format!("{} is unavailable in this session.", plan.name));
                    }
                }
            }
        }
        match memory {
            Some(Ok(tools)) => {
                let allowed = tools
                    .into_iter()
                    .filter(|tool| {
                        tool.get("name")
                            .and_then(Value::as_str)
                            .is_some_and(|name| {
                                self.memory_tools.iter().any(|allowed| allowed == name)
                            })
                    })
                    .collect();
                add_tools(
                    &mut discovery,
                    &mut taken,
                    &plan.memory_server,
                    Route::Memory,
                    allowed,
                );
            }
            Some(Err(failure)) => {
                tracing::warn!(reason = %failure, "memory tools unavailable");
                discovery.instructions.push(format!(
                    "{} is unavailable in this session.",
                    plan.memory_server
                ));
            }
            None => {}
        }
        discovery
    }

    /// The finished Phase 1 discovery (bounded wait), or why there is none.
    async fn discovered(&self) -> Result<Discovery, String> {
        let mut discovery = self.discovery.clone();
        let ready = tokio::time::timeout(
            DISCOVERY_TIMEOUT + Duration::from_secs(5),
            discovery.wait_for(Option::is_some),
        )
        .await;
        let Ok(Ok(found)) = ready else {
            return Err("session extensions are still starting".into());
        };
        let found = found.clone().unwrap_or_default();
        match &found.failed {
            Some(failed) => Err(failed.clone()),
            None => Ok(found),
        }
    }

    /// The tools, instructions and call guidance. Phase 1 publishes
    /// independently; Phase 2 runs only when `ackProtocol >= 1` is advertised.
    pub async fn list(&self, client: &Arc<Client>, params: &Value) -> Value {
        let found = match self.discovered().await {
            Ok(found) => found,
            Err(reason) => return refused(&reason),
        };
        let ack_protocol = params
            .get("ackProtocol")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let user_found = if ack_protocol >= 1 && client.binding.managed.is_none() {
            self.discover_user_mcp_cached(client, &found).await
        } else {
            UserDiscovery::default()
        };
        let tools: Vec<Value> = found
            .tools
            .iter()
            .chain(user_found.tools.iter())
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "server": tool.server,
                    "tool": tool.original,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                })
            })
            .collect();
        let instructions: Vec<&str> = found
            .instructions
            .iter()
            .chain(user_found.instructions.iter())
            .map(String::as_str)
            .collect();
        json!({"result": {
            "tools": tools,
            "instructions": instructions.join("\n\n"),
            "guidance": self.guidance,
        }})
    }

    /// One `tools/call` for the unit, inside its active turn.
    #[cfg(test)]
    pub async fn call(self: &Arc<Self>, client: &Arc<Client>, params: &Value) -> Value {
        self.call_guarded(client, params, 0).await.0
    }

    /// One `tools/call` returning a `SettlementGuard` (Phase 1 / Memory) or
    /// transferring ownership into `PendingAck` (`Route::UserServer`).
    pub(super) async fn call_guarded(
        self: &Arc<Self>,
        client: &Arc<Client>,
        params: &Value,
        connection_id: u64,
    ) -> (Value, Option<SettlementGuard>, AckState) {
        let Some(ledger) = client.ledger().cloned() else {
            return (refused("no execution ledger"), None, AckState::None);
        };
        let found = match self.discovery.borrow().clone() {
            Some(found) if found.failed.is_none() => found,
            Some(found) => {
                return (
                    refused(found.failed.as_deref().unwrap_or("unavailable")),
                    None,
                    AckState::None,
                );
            }
            None => {
                return (
                    refused("session extensions are still starting"),
                    None,
                    AckState::None,
                );
            }
        };
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let tool = found
            .tools
            .into_iter()
            .find(|tool| tool.name == name)
            .or_else(|| {
                self.phase2_cache
                    .lock()
                    .as_ref()
                    .and_then(|user| user.tools.iter().find(|tool| tool.name == name).cloned())
            });
        let Some(tool) = tool else {
            return (refused("unknown extension tool"), None, AckState::None);
        };
        let arguments = params
            .get("arguments")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));

        if let Route::UserServer(index) = tool.route {
            return self
                .call_user_server_guarded(
                    client,
                    &ledger,
                    &tool,
                    index,
                    params,
                    arguments,
                    connection_id,
                )
                .await;
        }

        let admitted = match ledger.admit_call() {
            Ok(admitted) => admitted,
            Err(reason) => return (refused(reason), None, AckState::None),
        };
        let turn = Ledger::admitted_turn(&admitted).to_owned();
        let request = json!({"name": tool.original, "arguments": arguments});
        let exchange = async {
            match tool.route {
                Route::Memory => {
                    if !self.memory_tools.contains(&tool.original) {
                        return Err(Failure::NotSent("memory tool not enabled".into()));
                    }
                    if !ledger.turn_open(&turn) {
                        return Err(Failure::NotSent(
                            "the turn ended before the call ran".into(),
                        ));
                    }
                    match &self.memory {
                        Some(bridge) => memory_call(bridge, &request).await,
                        None => Err(Failure::NotSent("memory unavailable".into())),
                    }
                }
                Route::Server(index) => match self.servers.get(index) {
                    Some(server) => {
                        server
                            .request(client, &ledger, Some(&turn), "tools/call", request.clone())
                            .await
                    }
                    None => Err(Failure::NotSent("server unavailable".into())),
                },
                Route::UserServer(_) => unreachable!("handled above"),
            }
        };
        let outcome = tokio::time::timeout(CALL_TIMEOUT, exchange)
            .await
            .unwrap_or_else(|_| Err(Failure::MaybeSent("timed out".into())));
        let reply = match outcome {
            Ok(result) => json!({"result": result}),
            Err(Failure::NotSent(reason) | Failure::Refused(reason)) => {
                refused(&format!("{}: {reason}", tool.server))
            }
            Err(Failure::MaybeSent(_)) => {
                ledger.mark_uncertain("tools/call");
                json!({"error": {
                    "code": -32000,
                    "message": "Extension call result unknown; no replay",
                    "data": {"outcome": "unknown"},
                }})
            }
        };
        let guard = ledger.guard_call(admitted);
        (reply, Some(guard), AckState::None)
    }
}

fn add_tools(
    discovery: &mut Discovery,
    taken: &mut HashSet<String>,
    server: &str,
    route: Route,
    tools: Vec<Value>,
) {
    for tool in tools.into_iter().take(MAX_TOOLS) {
        let Some(original) = tool
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| tool_name(name))
        else {
            continue;
        };
        let Some(name) = alias(taken, server, original) else {
            continue;
        };
        discovery.tools.push(Tool {
            name,
            server: server.to_owned(),
            original: original.to_owned(),
            route,
            description: tool.get("description").cloned().unwrap_or(Value::Null),
            input_schema: tool
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object"})),
        });
    }
}

fn alias(taken: &mut HashSet<String>, server: &str, original: &str) -> Option<String> {
    let base = format!("{server}_{original}");
    let candidates = std::iter::once(original.to_owned())
        .chain(std::iter::once(base.chars().take(64).collect()))
        .chain((2..100).map(|n| {
            let suffix = format!("_{n}");
            let head: String = base.chars().take(64 - suffix.len()).collect();
            format!("{head}{suffix}")
        }));
    candidates
        .into_iter()
        .find(|candidate| tool_name(candidate) && taken.insert(candidate.clone()))
}

fn tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn refused(message: &str) -> Value {
    json!({"error": {"code": -32000, "message": message}})
}

fn workspace_url(client: &Client) -> Result<String, String> {
    reqwest::Url::from_file_path(&client.binding.workspace.cwd)
        .map(|url| url.to_string())
        .map_err(|()| "workspace path".to_owned())
}

fn launch(process: &str, argv: &[String], cwd: &str, stdin: bool) -> Value {
    json!({
        "processId": process,
        "argv": argv,
        "cwd": cwd,
        "env": {},
        "tty": false,
        "pipeStdin": stdin,
        "arg0": null,
        "envPolicy": {"inherit": "all", "ignoreDefaultExcludes": true,
            "exclude": [], "set": {}, "includeOnly": []},
    })
}

async fn start(
    client: &Arc<Client>,
    ledger: &Ledger,
    process: &str,
    params: Value,
) -> Result<(), String> {
    ledger.record_server(process)?;
    let started = client.invoke("process/start".into(), params).await;
    let ok = started.as_ref().is_ok_and(|reply| {
        reply.pointer("/result/processId").and_then(Value::as_str) == Some(process)
    });
    ledger.server_started(process, ok);
    if ok {
        Ok(())
    } else {
        Err("the target refused the process".into())
    }
}

impl Server {
    async fn initialize(
        &self,
        client: &Arc<Client>,
        plan: &ServerPlan,
    ) -> Result<(String, Vec<Value>), String> {
        let ledger = client.ledger().cloned().ok_or("no execution ledger")?;
        let cwd = workspace_url(client)?;
        if plan.required {
            binding_capabilities(client, &ledger, &cwd, plan).await?;
        }
        start(
            client,
            &ledger,
            &self.process,
            launch(&self.process, &plan.argv, &cwd, true),
        )
        .await?;
        let ready = self
            .request(
                client,
                &ledger,
                None,
                "initialize",
                json!({"protocolVersion": MCP_PROTOCOL, "capabilities": {},
                    "clientInfo": {"name": "cowboy-antigravity", "version": "1.0.0"}}),
            )
            .await
            .map_err(Failure::text)?;
        self.notify(client, "notifications/initialized")
            .await
            .map_err(Failure::text)?;
        let listed = self
            .request(client, &ledger, None, "tools/list", json!({}))
            .await
            .map_err(Failure::text)?;
        let instructions = ready
            .get("instructions")
            .and_then(Value::as_str)
            .map(|text| text.chars().take(8192).collect())
            .unwrap_or_default();
        let tools = listed
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok((instructions, tools))
    }

    async fn write(&self, client: &Arc<Client>, message: &Value) -> Result<(), Failure> {
        let mut line =
            serde_json::to_vec(message).map_err(|_| Failure::NotSent("encode".into()))?;
        line.push(b'\n');
        let chunk = base64::engine::general_purpose::STANDARD.encode(line);
        let write_id = format!("{:032x}", rand::random::<u128>());
        // A new write is an effect: refused while effects cannot be recorded
        // durably (retries of one already sent keep going).
        if client.ledger().is_some_and(|ledger| ledger.unwritable()) {
            return Err(Failure::NotSent(
                "the execution ledger cannot be written".into(),
            ));
        }
        for _ in 0..50 {
            let reply = client
                .invoke_classified(
                    "process/write".into(),
                    json!({"processId": self.process, "chunk": chunk, "writeId": write_id}),
                )
                .await
                .map_err(|maybe| {
                    if maybe {
                        Failure::MaybeSent("write reply lost".into())
                    } else {
                        Failure::NotSent("write refused".into())
                    }
                })?;
            match reply.pointer("/result/status").and_then(Value::as_str) {
                Some("accepted") => return Ok(()),
                Some("starting") => tokio::time::sleep(Duration::from_millis(100)).await,
                _ => return Err(Failure::NotSent("the server refused input".into())),
            }
        }
        Err(Failure::NotSent("the server never started".into()))
    }

    async fn notify(&self, client: &Arc<Client>, method: &str) -> Result<(), Failure> {
        let _io = self.io.lock().await;
        self.write(
            client,
            &json!({"jsonrpc": "2.0", "method": method, "params": {}}),
        )
        .await
    }

    async fn request(
        &self,
        client: &Arc<Client>,
        ledger: &Ledger,
        turn: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, Failure> {
        self.request_bounded(client, ledger, turn, method, params, None)
            .await
    }

    async fn request_bounded(
        &self,
        client: &Arc<Client>,
        ledger: &Ledger,
        turn: Option<&str>,
        method: &str,
        params: Value,
        max_result_bytes: Option<usize>,
    ) -> Result<Value, Failure> {
        let mut io = self.io.lock().await;
        if io.closed || self.is_closed_hint() {
            io.closed = true;
            return Err(Failure::NotSent("the server has ended".into()));
        }
        if turn.is_some_and(|turn| !ledger.turn_open(turn)) {
            return Err(Failure::NotSent(
                "the turn ended before the call ran".into(),
            ));
        }
        io.next_id += 1;
        let id = io.next_id;
        self.write(
            client,
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )
        .await?;
        loop {
            if let Some(answer) = take_answer_bounded(&mut io.buffer, id, max_result_bytes) {
                return answer.map_err(Failure::Refused);
            }
            if io.buffer.len() > MAX_LINE {
                io.closed = true;
                self.mark_closed();
                return Err(Failure::MaybeSent("the server's line is too long".into()));
            }
            let result = self.read(client, io.after_seq).await?;
            let chunks = result
                .get("chunks")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let exited = result.get("exited") == Some(&Value::Bool(true));
            for chunk in &chunks {
                let Some(seq) = chunk.get("seq").and_then(Value::as_u64) else {
                    continue;
                };
                if io.after_seq.is_some_and(|after| seq <= after) {
                    continue;
                }
                let expected = io.after_seq.map_or(1, |after| after + 1);
                if seq != expected && !(exited && seq == expected + 1) {
                    io.closed = true;
                    self.mark_closed();
                    return Err(Failure::MaybeSent("server output was lost".into()));
                }
                io.after_seq = Some(seq);
                if chunk.get("stream").and_then(Value::as_str) == Some("stdout") {
                    let Some(bytes) = chunk.get("chunk").and_then(Value::as_str).and_then(|text| {
                        base64::engine::general_purpose::STANDARD.decode(text).ok()
                    }) else {
                        io.closed = true;
                        self.mark_closed();
                        return Err(Failure::MaybeSent("undecodable server output".into()));
                    };
                    io.buffer.extend_from_slice(&bytes);
                }
            }
            if result.get("closed") == Some(&Value::Bool(true)) && chunks.is_empty() {
                io.closed = true;
                self.mark_closed();
                return Err(Failure::MaybeSent("the server exited".into()));
            }
        }
    }

    async fn read(&self, client: &Arc<Client>, after: Option<u64>) -> Result<Value, Failure> {
        for attempt in 0..5u64 {
            match client
                .invoke_classified(
                    "process/read".into(),
                    json!({"processId": self.process, "afterSeq": after,
                        "maxBytes": 1_048_576, "waitMs": 1000}),
                )
                .await
            {
                Ok(reply) => {
                    return reply
                        .get("result")
                        .cloned()
                        .ok_or_else(|| Failure::MaybeSent("the target refused the read".into()));
                }
                Err(false) => {
                    tokio::time::sleep(Duration::from_millis(200 * (attempt + 1))).await;
                }
                Err(true) => return Err(Failure::MaybeSent("read reply lost".into())),
            }
        }
        Err(Failure::MaybeSent("the target kept refusing reads".into()))
    }
}

/// A required binding's CLI must report the binding contract Claude's and
/// Codex's runtimes check (`mcp --binding-capabilities`).
async fn binding_capabilities(
    client: &Arc<Client>,
    ledger: &Ledger,
    cwd: &str,
    plan: &ServerPlan,
) -> Result<(), String> {
    let command = plan.argv.first().ok_or("binding without a command")?;
    let process = format!("{SERVER_PREFIX}{:032x}", rand::random::<u128>());
    let argv = vec![
        command.clone(),
        "mcp".into(),
        "--binding-capabilities".into(),
    ];
    start(
        client,
        ledger,
        &process,
        launch(&process, &argv, cwd, false),
    )
    .await?;
    let mut output = Vec::new();
    let mut after = None;
    for _ in 0..30 {
        let reply = client
            .invoke(
                "process/read".into(),
                json!({"processId": process, "afterSeq": after, "maxBytes": 8192, "waitMs": 1000}),
            )
            .await
            .map_err(|error| error.to_string())?;
        let result = reply.get("result").cloned().unwrap_or(Value::Null);
        for chunk in result
            .get("chunks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            after = chunk.get("seq").and_then(Value::as_u64).or(after);
            if chunk.get("stream").and_then(Value::as_str) == Some("stdout")
                && let Some(bytes) = chunk
                    .get("chunk")
                    .and_then(Value::as_str)
                    .and_then(|text| base64::engine::general_purpose::STANDARD.decode(text).ok())
            {
                if output.len().saturating_add(bytes.len()) > 4096 {
                    return Err("binding capability probe failed".into());
                }
                output.extend_from_slice(&bytes);
            }
        }
        if result.get("closed") == Some(&Value::Bool(true)) {
            ledger.closed(&process);
        }
        if result.get("exited") == Some(&Value::Bool(true)) {
            ledger.exited(&process);
            if result.get("exitCode").and_then(Value::as_i64) != Some(0) || output.len() > 4096 {
                return Err("binding capability probe failed".into());
            }
            let report: Value = serde_json::from_slice(&output)
                .map_err(|_| "invalid binding capability report".to_owned())?;
            let expected = json!({"schema": 1, "descriptor_schema": 1, "relay_schema": 1,
                "grant_mcp": true, "transport": "stdio_tool_only_v1"});
            return if report == expected {
                Ok(())
            } else {
                Err("incompatible binding capabilities".into())
            };
        }
    }
    Err("binding capability probe timed out".into())
}

fn take_answer_bounded(
    buffer: &mut Vec<u8>,
    id: u64,
    max_result_bytes: Option<usize>,
) -> Option<Result<Value, String>> {
    while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
        let line: Vec<u8> = buffer.drain(..=newline).collect();
        let line_len = line.len();
        let Ok(message) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if message.get("method").is_some() || message.get("id") != Some(&json!(id)) {
            continue;
        }
        if let Some(result) = message.get("result") {
            if max_result_bytes.is_some_and(|limit| line_len > limit) {
                tracing::warn!(
                    reason = "result_too_large",
                    bytes = line_len,
                    "user MCP tool result exceeded size bound"
                );
                return Some(Ok(json!({
                    "content": [{"type": "text", "text": "user MCP tool result exceeded 4 MiB limit"}],
                    "isError": true,
                })));
            }
            return Some(Ok(result.clone()));
        }
        if let Some(error) = message.get("error") {
            let text = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("MCP error");
            return Some(Err(text.chars().take(500).collect()));
        }
    }
    None
}

#[cfg(test)]
fn take_answer(buffer: &mut Vec<u8>, id: u64) -> Option<Result<Value, String>> {
    take_answer_bounded(buffer, id, None)
}

async fn memory_tools(bridge: &MemoryBridge) -> Result<Vec<Value>, String> {
    bridge
        .mcp(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": MCP_PROTOCOL, "capabilities": {},
                "clientInfo": {"name": "cowboy-antigravity", "version": "1.0.0"}}}))
        .await
        .map_err(|(_, text)| text)?;
    bridge
        .notify(&json!({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}))
        .await?;
    let listed = bridge
        .mcp(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}))
        .await
        .map_err(|(_, text)| text)?;
    Ok(listed
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

async fn memory_call(bridge: &MemoryBridge, request: &Value) -> Result<Value, Failure> {
    bridge
        .mcp(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": request}))
        .await
        .map_err(|(refused, text)| {
            if refused {
                Failure::Refused(text)
            } else {
                Failure::MaybeSent(text)
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_matched_by_id_and_whole_lines_only() {
        let mut buffer = br#"{"jsonrpc":"2.0","id":7,"method":"ping"}
{"jsonrpc":"2.0","id":6,"result":{"stale":true}}
not json
{"jsonrpc":"2.0","id":7,"result":{"ok":"caf"#
            .to_vec();
        assert!(take_answer(&mut buffer, 7).is_none());
        buffer.extend_from_slice("é\"}}\n".as_bytes());
        assert_eq!(take_answer(&mut buffer, 7), Some(Ok(json!({"ok": "café"}))));
        let mut error = br#"{"jsonrpc":"2.0","id":8,"error":{"message":"nope"}}
"#
        .to_vec();
        assert_eq!(take_answer(&mut error, 8), Some(Err("nope".into())));
    }

    #[test]
    fn oversized_user_mcp_result_returns_bounded_tool_error_and_drains_line() {
        let big = "x".repeat(2048);
        let mut buffer = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{big}\"}}]}}}}\n{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"ok\":true}}}}\n"
        )
        .into_bytes();
        let res = take_answer_bounded(&mut buffer, 1, Some(512))
            .unwrap()
            .unwrap();
        assert_eq!(res["isError"], true);
        assert_eq!(
            take_answer_bounded(&mut buffer, 2, Some(512)),
            Some(Ok(json!({"ok": true})))
        );
    }

    #[test]
    fn tool_names_stay_in_the_gate_namespace_and_aliases_are_unique() {
        assert!(tool_name("cowboy_read") && tool_name("suger-query"));
        assert!(
            !tool_name("") && !tool_name("a b") && !tool_name("x.y") && !tool_name(&"a".repeat(65))
        );
        let mut taken = HashSet::new();
        assert_eq!(alias(&mut taken, "s", "read").as_deref(), Some("read"));
        assert_eq!(alias(&mut taken, "s", "read").as_deref(), Some("s_read"));
        assert_eq!(alias(&mut taken, "s", "read").as_deref(), Some("s_read_2"));
        let long = "a".repeat(64);
        assert_eq!(
            alias(&mut taken, "s", &long).as_deref(),
            Some(long.as_str())
        );
        let second = alias(&mut taken, "s", &long).unwrap();
        assert!(second.len() <= 64 && second != long);
    }

    #[test]
    fn user_tool_alias_always_enforces_mandatory_server_prefix() {
        let mut taken = HashSet::new();
        assert_eq!(
            user_mcp::user_tool_alias(&mut taken, "chrome-devtools", "screenshot").as_deref(),
            Some("chrome-devtools_screenshot")
        );
        assert_eq!(
            user_mcp::user_tool_alias(&mut taken, "chrome-devtools", "screenshot").as_deref(),
            Some("chrome-devtools_screenshot_2")
        );
        assert_eq!(
            user_mcp::user_tool_alias(&mut taken, "my.server", "run").as_deref(),
            Some("my_server_run")
        );
    }
}
