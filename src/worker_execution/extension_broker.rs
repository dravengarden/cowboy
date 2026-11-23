//! The session extensions of an account-sandboxed session (Antigravity),
//! brokered by the worker (docs/antigravity-real-accounts-design.md,
//! "Session extensions"). The worker starts Cowboy MCP and the folder's
//! bindings on the target from its own launch template, as a Claude or Codex
//! session runs them, discovers their tools, and answers the unit's two
//! requests: `cowboy/extensions` (the tools) and `cowboy/extensionCall` (one
//! `tools/call`, admitted only inside the active turn, drained before the
//! turn completes, and an uncertain effect when its outcome is lost).
//! Memory's tools go to the session's relay the same way, limited to the
//! signed tool list. The unit never starts, writes to or reads these
//! processes, nor sees their output.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};
use tokio::sync::{Mutex as AsyncMutex, watch};

use super::Client;
use super::ledger::{Ledger, SERVER_PREFIX};
use super::memory_bridge::MemoryBridge;

/// The unit's request listing the session's extension tools.
pub const EXTENSIONS_REQUEST: &str = "cowboy/extensions";
/// The unit's request running one extension tool.
pub const EXTENSION_CALL: &str = "cowboy/extensionCall";

const MCP_PROTOCOL: &str = "2025-06-18";
/// One deadline for all of discovery, required and optional alike.
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
    memory: Option<MemoryBridge>,
    memory_tools: Vec<String>,
    guidance: Vec<String>,
    discovery: watch::Receiver<Option<Discovery>>,
}

impl Broker {
    /// Start every planned server in the background; discovery completes on
    /// its own and `cowboy/extensions` waits for it (bounded).
    pub fn start(client: &Arc<Client>, plan: Plan, memory: Option<MemoryBridge>) -> Arc<Self> {
        let servers: Vec<Arc<Server>> = plan
            .servers
            .iter()
            .map(|_| {
                Arc::new(Server {
                    process: format!("{SERVER_PREFIX}{:032x}", rand::random::<u128>()),
                    io: AsyncMutex::new(Io::default()),
                })
            })
            .collect();
        let memory = memory.filter(|_| !plan.memory_tools.is_empty());
        let (sender, receiver) = watch::channel(None);
        let broker = Arc::new(Self {
            servers,
            memory,
            memory_tools: plan.memory_tools.clone(),
            guidance: plan.guidance.iter().take(MAX_GUIDANCE).cloned().collect(),
            discovery: receiver,
        });
        let client = Arc::clone(client);
        let discovering = Arc::clone(&broker);
        tokio::spawn(async move {
            let discovery =
                tokio::time::timeout(DISCOVERY_TIMEOUT, discovering.discover(&client, &plan))
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
        broker
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
        let mut taken: std::collections::HashSet<String> =
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

    /// The finished discovery (bounded wait), or why there is none.
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

    /// The tools, instructions and call guidance, once discovery finished.
    pub async fn list(&self) -> Value {
        let found = match self.discovered().await {
            Ok(found) => found,
            Err(reason) => return refused(&reason),
        };
        json!({"result": {
            "tools": found.tools.iter().map(|tool| json!({
                "name": tool.name,
                "server": tool.server,
                "tool": tool.original,
                "description": tool.description,
                "inputSchema": tool.input_schema,
            })).collect::<Vec<_>>(),
            "instructions": found.instructions.join("\n\n"),
            "guidance": self.guidance,
        }})
    }

    /// One `tools/call` for the unit, inside its active turn.
    pub async fn call(&self, client: &Arc<Client>, params: &Value) -> Value {
        let Some(ledger) = client.ledger().cloned() else {
            return refused("no execution ledger");
        };
        // A session whose required binding failed has no extension tools.
        let found = match self.discovery.borrow().clone() {
            Some(found) if found.failed.is_none() => found,
            Some(found) => return refused(found.failed.as_deref().unwrap_or("unavailable")),
            None => return refused("session extensions are still starting"),
        };
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(tool) = found.tools.into_iter().find(|tool| tool.name == name) else {
            return refused("unknown extension tool");
        };
        let arguments = params
            .get("arguments")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let admitted = match ledger.admit_call() {
            Ok(admitted) => admitted,
            Err(reason) => return refused(reason),
        };
        let turn = Ledger::admitted_turn(&admitted).to_owned();
        let request = json!({"name": tool.original, "arguments": arguments});
        let exchange = async {
            match tool.route {
                Route::Memory => {
                    // The signed list decides again at dispatch.
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
            }
        };
        let outcome = tokio::time::timeout(CALL_TIMEOUT, exchange)
            .await
            .unwrap_or_else(|_| Err(Failure::MaybeSent("timed out".into())));
        // The outcome is decided before the turn may complete.
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
        ledger.settle_call(&admitted);
        reply
    }
}

/// Add `tools` of `server` under unique aliases within the gate's names.
fn add_tools(
    discovery: &mut Discovery,
    taken: &mut std::collections::HashSet<String>,
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

/// The tool's own name, else `<server>_<name>`, else a numbered alias, all
/// within 64 characters; deterministic for the same discovery order.
fn alias(
    taken: &mut std::collections::HashSet<String>,
    server: &str,
    original: &str,
) -> Option<String> {
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

/// Start a target process the worker owns, recorded before it is sent.
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
    /// Start the server, initialize it and list its tools.
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

    /// One request and its answer; other messages the server sends are
    /// skipped (it has no client capabilities to ask for). With `turn`, the
    /// request is written only while that turn is still active.
    async fn request(
        &self,
        client: &Arc<Client>,
        ledger: &Ledger,
        turn: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, Failure> {
        let mut io = self.io.lock().await;
        if io.closed {
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
            if let Some(answer) = take_answer(&mut io.buffer, id) {
                return answer.map_err(Failure::Refused);
            }
            if io.buffer.len() > MAX_LINE {
                io.closed = true;
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
                // Output the executor no longer holds would splice two
                // replies; only the exit's own number may be skipped.
                let expected = io.after_seq.map_or(1, |after| after + 1);
                if seq != expected && !(exited && seq == expected + 1) {
                    io.closed = true;
                    return Err(Failure::MaybeSent("server output was lost".into()));
                }
                io.after_seq = Some(seq);
                if chunk.get("stream").and_then(Value::as_str) == Some("stdout") {
                    let Some(bytes) = chunk.get("chunk").and_then(Value::as_str).and_then(|text| {
                        base64::engine::general_purpose::STANDARD.decode(text).ok()
                    }) else {
                        io.closed = true;
                        return Err(Failure::MaybeSent("undecodable server output".into()));
                    };
                    io.buffer.extend_from_slice(&bytes);
                }
            }
            if result.get("closed") == Some(&Value::Bool(true)) && chunks.is_empty() {
                io.closed = true;
                return Err(Failure::MaybeSent("the server exited".into()));
            }
        }
    }

    /// One `process/read` at `after`; a read the transport never sent is
    /// retried from the same cursor, an error answer ends the exchange.
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
                output.extend_from_slice(&bytes);
            }
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

/// The answer to request `id` among the complete lines in `buffer` (other
/// lines are consumed and skipped); a line is decoded only once complete.
fn take_answer(buffer: &mut Vec<u8>, id: u64) -> Option<Result<Value, String>> {
    while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
        let line: Vec<u8> = buffer.drain(..=newline).collect();
        let Ok(message) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        // A response carries our id and a result or an error; a request
        // from the server has a method and is not an answer.
        if message.get("method").is_some() || message.get("id") != Some(&json!(id)) {
            continue;
        }
        if let Some(result) = message.get("result") {
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

/// Memory's tool list from the relay's MCP endpoint, after the handshake.
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
    fn tool_names_stay_in_the_gate_namespace_and_aliases_are_unique() {
        assert!(tool_name("cowboy_read") && tool_name("suger-query"));
        assert!(
            !tool_name("") && !tool_name("a b") && !tool_name("x.y") && !tool_name(&"a".repeat(65))
        );
        let mut taken = std::collections::HashSet::new();
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
}
