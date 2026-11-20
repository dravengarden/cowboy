//! The session extensions of an account-sandboxed session (Antigravity),
//! brokered by the worker (docs/antigravity-real-accounts-design.md,
//! "Session extensions"). The worker starts Cowboy MCP and the folder's
//! bindings on the target from its own launch template, as a Claude or Codex
//! session runs them, discovers their tools, and answers the unit's two
//! requests: `cowboy/extensions` (the tools) and `cowboy/extensionCall` (one
//! `tools/call`, admitted only inside the active turn, drained before the
//! turn completes, and an uncertain effect when its reply is lost). Memory's
//! tools go to the session's relay the same way, limited to the signed tool
//! list. The unit never starts, writes to or reads these processes.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};
use tokio::sync::{Mutex as AsyncMutex, watch};

use super::Client;
use super::memory_bridge::MemoryBridge;

/// The unit's request listing the session's extension tools.
pub const EXTENSIONS_REQUEST: &str = "cowboy/extensions";
/// The unit's request running one extension tool.
pub const EXTENSION_CALL: &str = "cowboy/extensionCall";

const MCP_PROTOCOL: &str = "2025-06-18";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(45);
const CALL_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_TOOLS: usize = 128;
const MAX_LINE: usize = 16 * 1024 * 1024;
/// The facade's own tools; an extension tool of the same name is prefixed.
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
    /// start without one); Cowboy MCP is not.
    pub required: bool,
}

/// The extension plan a sandboxed worker derived from its frozen descriptor.
#[derive(Debug, Clone)]
pub struct Plan {
    pub servers: Vec<ServerPlan>,
    /// Memory's tools the signed configuration enables (empty: none).
    pub memory_tools: Vec<String>,
    pub memory_server: String,
}

#[derive(Clone)]
struct Tool {
    name: String,
    server: String,
    original: String,
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
    name: String,
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

/// A failed exchange: whether the request may have reached the server.
enum Failure {
    NotSent(String),
    MaybeSent(String),
}

pub struct Broker {
    servers: Vec<Arc<Server>>,
    memory: Option<(MemoryBridge, String)>,
    memory_tools: Vec<String>,
    discovery: watch::Receiver<Option<Discovery>>,
}

impl Broker {
    /// Start every planned server in the background; discovery completes on
    /// its own and `cowboy/extensions` waits for it (bounded).
    pub fn start(client: &Arc<Client>, plan: Plan, memory: Option<MemoryBridge>) -> Arc<Self> {
        let servers: Vec<(Arc<Server>, ServerPlan)> = plan
            .servers
            .into_iter()
            .map(|server| {
                (
                    Arc::new(Server {
                        name: server.name.clone(),
                        process: format!("cowboy-worker-mcp-{:032x}", rand::random::<u128>()),
                        io: AsyncMutex::new(Io::default()),
                    }),
                    server,
                )
            })
            .collect();
        let memory = memory
            .filter(|_| !plan.memory_tools.is_empty())
            .map(|bridge| (bridge, plan.memory_server.clone()));
        let (sender, receiver) = watch::channel(None);
        let broker = Arc::new(Self {
            servers: servers
                .iter()
                .map(|(server, _)| Arc::clone(server))
                .collect(),
            memory,
            memory_tools: plan.memory_tools,
            discovery: receiver,
        });
        let client = Arc::clone(client);
        let discovering = Arc::clone(&broker);
        tokio::spawn(async move {
            let discovery = discovering.discover(&client, servers).await;
            let _ = sender.send(Some(discovery));
        });
        broker
    }

    async fn discover(
        &self,
        client: &Arc<Client>,
        servers: Vec<(Arc<Server>, ServerPlan)>,
    ) -> Discovery {
        let started = futures::future::join_all(servers.iter().map(|(server, plan)| async move {
            let result =
                tokio::time::timeout(DISCOVERY_TIMEOUT, server.initialize(client, plan)).await;
            (server, plan, result)
        }))
        .await;
        let mut discovery = Discovery::default();
        let mut taken: std::collections::HashSet<String> =
            FACADE_TOOLS.iter().map(|name| (*name).to_owned()).collect();
        let mut add = |discovery: &mut Discovery, server: &str, tools: Vec<Value>| {
            for tool in tools.into_iter().take(MAX_TOOLS) {
                let Some(original) = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| tool_name(name))
                else {
                    continue;
                };
                let mut name = original.to_owned();
                if taken.contains(&name) {
                    name = format!("{server}_{original}");
                }
                if name.len() > 64 || !tool_name(&name) || !taken.insert(name.clone()) {
                    continue;
                }
                discovery.tools.push(Tool {
                    name,
                    server: server.to_owned(),
                    original: original.to_owned(),
                    description: tool.get("description").cloned().unwrap_or(Value::Null),
                    input_schema: tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object"})),
                });
            }
        };
        for (server, plan, result) in started {
            let failure = match result {
                Ok(Ok((instructions, tools))) => {
                    if !instructions.is_empty() {
                        discovery
                            .instructions
                            .push(format!("{}: {instructions}", server.name));
                    }
                    add(&mut discovery, &server.name, tools);
                    continue;
                }
                Ok(Err(error)) => error,
                Err(_) => "timed out".to_owned(),
            };
            tracing::warn!(server = %server.name, reason = %failure, "session extension unavailable");
            if plan.required {
                discovery.failed = Some(format!("{} is unavailable: {failure}", server.name));
            } else {
                discovery
                    .instructions
                    .push(format!("{} is unavailable in this session.", server.name));
            }
        }
        if let Some((bridge, server)) = &self.memory {
            match tokio::time::timeout(DISCOVERY_TIMEOUT, memory_tools(bridge)).await {
                Ok(Ok(tools)) => {
                    let allowed: Vec<Value> = tools
                        .into_iter()
                        .filter(|tool| {
                            tool.get("name")
                                .and_then(Value::as_str)
                                .is_some_and(|name| {
                                    self.memory_tools.iter().any(|allowed| allowed == name)
                                })
                        })
                        .collect();
                    add(&mut discovery, server, allowed);
                }
                _ => discovery
                    .instructions
                    .push(format!("{server} is unavailable in this session.")),
            }
        }
        discovery
    }

    /// The tools and instructions, once discovery finished (bounded).
    pub async fn list(&self) -> Value {
        let mut discovery = self.discovery.clone();
        let ready = tokio::time::timeout(
            DISCOVERY_TIMEOUT + Duration::from_secs(5),
            discovery.wait_for(Option::is_some),
        )
        .await;
        let Ok(Ok(found)) = ready else {
            return refused("session extensions are still starting");
        };
        let found = found.clone().unwrap_or_default();
        if let Some(failed) = found.failed {
            return refused(&failed);
        }
        json!({"result": {
            "tools": found.tools.iter().map(|tool| json!({
                "name": tool.name,
                "server": tool.server,
                "tool": tool.original,
                "description": tool.description,
                "inputSchema": tool.input_schema,
            })).collect::<Vec<_>>(),
            "instructions": found.instructions.join("\n\n"),
        }})
    }

    /// One `tools/call` for the unit, inside its active turn.
    pub async fn call(&self, client: &Arc<Client>, params: &Value) -> Value {
        let Some(ledger) = client.ledger().cloned() else {
            return refused("no execution ledger");
        };
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let tool = self
            .discovery
            .borrow()
            .as_ref()
            .and_then(|found| found.tools.iter().find(|tool| tool.name == name).cloned());
        let Some(tool) = tool else {
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
        let request = json!({"name": tool.original, "arguments": arguments});
        let outcome = if self
            .memory
            .as_ref()
            .is_some_and(|(_, server)| *server == tool.server)
        {
            let (bridge, _) = self.memory.as_ref().expect("memory server");
            tokio::time::timeout(CALL_TIMEOUT, memory_call(bridge, &request))
                .await
                .unwrap_or_else(|_| Err(Failure::MaybeSent("timed out".into())))
        } else if let Some(server) = self
            .servers
            .iter()
            .find(|server| server.name == tool.server)
        {
            tokio::time::timeout(CALL_TIMEOUT, server.request(client, "tools/call", request))
                .await
                .unwrap_or_else(|_| Err(Failure::MaybeSent("timed out".into())))
        } else {
            Err(Failure::NotSent("server unavailable".into()))
        };
        ledger.settled(&admitted, None);
        match outcome {
            Ok(result) => json!({"result": result}),
            Err(Failure::NotSent(reason)) => refused(&format!("{}: {reason}", tool.server)),
            Err(Failure::MaybeSent(_)) => {
                ledger.mark_uncertain("tools/call");
                json!({"error": {
                    "code": -32000,
                    "message": "Extension call result unknown; no replay",
                    "data": {"outcome": "unknown"},
                }})
            }
        }
    }
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

impl Server {
    /// Start the server, initialize it and list its tools.
    async fn initialize(
        &self,
        client: &Arc<Client>,
        plan: &ServerPlan,
    ) -> Result<(String, Vec<Value>), String> {
        let cwd = reqwest::Url::from_file_path(&client.binding.workspace.cwd)
            .map_err(|()| "workspace path".to_owned())?;
        let ledger = client.ledger().cloned().ok_or("no execution ledger")?;
        ledger.record_server(&self.process)?;
        let started = client
            .invoke(
                "process/start".into(),
                json!({
                    "processId": self.process,
                    "argv": plan.argv,
                    "cwd": cwd.as_str(),
                    "env": {},
                    "tty": false,
                    "pipeStdin": true,
                    "arg0": null,
                    "envPolicy": {"inherit": "all", "ignoreDefaultExcludes": true,
                        "exclude": [], "set": {}, "includeOnly": []},
                }),
            )
            .await
            .map_err(|error| error.to_string())?;
        if started.pointer("/result/processId").and_then(Value::as_str) != Some(&self.process) {
            ledger.exited(&self.process);
            return Err("the target refused the server".into());
        }
        let ready = self
            .request(
                client,
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
            .request(client, "tools/list", json!({}))
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
    /// skipped (it has no client capabilities to ask for).
    async fn request(
        &self,
        client: &Arc<Client>,
        method: &str,
        params: Value,
    ) -> Result<Value, Failure> {
        let mut io = self.io.lock().await;
        if io.closed {
            return Err(Failure::NotSent("the server has ended".into()));
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
                return answer.map_err(Failure::MaybeSent);
            }
            if io.buffer.len() > MAX_LINE {
                io.closed = true;
                return Err(Failure::MaybeSent("the server's line is too long".into()));
            }
            let reply = client
                .invoke(
                    "process/read".into(),
                    json!({"processId": self.process, "afterSeq": io.after_seq,
                        "maxBytes": 1_048_576, "waitMs": 1000}),
                )
                .await
                .map_err(|_| Failure::MaybeSent("read failed".into()))?;
            let result = reply.get("result").cloned().unwrap_or(Value::Null);
            let chunks = result
                .get("chunks")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for chunk in &chunks {
                let Some(seq) = chunk.get("seq").and_then(Value::as_u64) else {
                    continue;
                };
                if io.after_seq.is_some_and(|after| seq <= after) {
                    continue;
                }
                // Output the executor no longer holds would splice two
                // replies: the server is unusable from here on.
                let expected = io.after_seq.map_or(1, |after| after + 1);
                let is_output = chunk.get("stream").and_then(Value::as_str) == Some("stdout");
                if seq != expected && is_output {
                    io.closed = true;
                    return Err(Failure::MaybeSent("server output was lost".into()));
                }
                io.after_seq = Some(seq);
                if is_output
                    && let Some(bytes) =
                        chunk.get("chunk").and_then(Value::as_str).and_then(|text| {
                            base64::engine::general_purpose::STANDARD.decode(text).ok()
                        })
                {
                    io.buffer.extend_from_slice(&bytes);
                }
            }
            if result.get("closed") == Some(&Value::Bool(true)) && chunks.is_empty() {
                io.closed = true;
                return Err(Failure::MaybeSent("the server exited".into()));
            }
        }
    }
}

impl Failure {
    fn text(self) -> String {
        match self {
            Self::NotSent(text) | Self::MaybeSent(text) => text,
        }
    }
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

/// Memory's tool list from the relay's MCP endpoint (stateless JSON).
async fn memory_tools(bridge: &MemoryBridge) -> Result<Vec<Value>, String> {
    let init = bridge
        .mcp(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": MCP_PROTOCOL, "capabilities": {},
                "clientInfo": {"name": "cowboy-antigravity", "version": "1.0.0"}}}))
        .await?;
    let _ = init;
    let listed = bridge
        .mcp(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}))
        .await?;
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
        .map_err(Failure::MaybeSent)
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
    fn tool_names_stay_in_the_gate_namespace() {
        assert!(tool_name("cowboy_read") && tool_name("suger-query"));
        assert!(
            !tool_name("") && !tool_name("a b") && !tool_name("x.y") && !tool_name(&"a".repeat(65))
        );
    }
}
