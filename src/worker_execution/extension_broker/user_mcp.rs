//! Demand-triggered Phase 2 user MCP discovery, correlated `PendingAck`
//! state machine, and bounded sanitization (`docs/antigravity-user-mcp-r2-design.md`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::Semaphore;

use super::{
    Broker, CALL_TIMEOUT, Discovery, FACADE_TOOLS, Failure, MCP_PROTOCOL, Route, Server, Tool,
    refused, tool_name,
};
use crate::worker_execution::Client;
use crate::worker_execution::ledger::{Ledger, SettlementGuard};
use crate::worker_execution::relay::AckState;

/// Shared keeper/worker protocol constants (verified byte-identical to
/// `components/machine/src/execution_host/user_mcp.rs` by test; avoids touching
/// `components/wire/src/` and preserves `retained-worker-interface-compatible`).
pub const USER_MCP_INITIALIZATION_KEY: &str = "cowboyUserMcp";
pub const USER_MCP_LIST_METHOD: &str = "cowboy/listUserMcp";
pub const USER_MCP_START_METHOD: &str = "cowboy/startUserMcp";
pub const USER_MCP_STOP_METHOD: &str = "cowboy/stopUserMcp";
pub const USER_MCP_PROCESS_PREFIX: &str = "cowboy-worker-mcp-user-";

pub(super) const USER_MCP_SWEEP_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const USER_MCP_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);
const USER_MCP_SERVER_INIT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_USER_MCP_CONCURRENCY: usize = 4;
const MAX_USER_MCP_SERVERS: usize = 16;
const MAX_USER_MCP_TOOLS_PER_SERVER: usize = 64;
const MAX_USER_MCP_TOOLS_TOTAL: usize = 128;
const MAX_USER_MCP_INSTRUCTIONS_PER_SERVER: usize = 1024;
const MAX_USER_MCP_INSTRUCTIONS_TOTAL: usize = 4096;
/// Comfortably below the proxy's 14 MiB WebSocket `maxPayload`.
pub(super) const MAX_USER_MCP_RESULT_BYTES: usize = 4 * 1024 * 1024;
/// Bounded proxy acknowledgement deadline started AT PendingAck registration.
pub(super) const EXTENSION_ACK_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CLOSED_CONNECTIONS: usize = 256;

#[derive(Clone, Default)]
pub(super) struct UserDiscovery {
    pub tools: Vec<Tool>,
    pub instructions: Vec<String>,
}

pub(super) struct PendingAckEntry {
    pub call_id: String,
    pub connection_id: u64,
    pub ledger: Arc<Ledger>,
    pub guard: SettlementGuard,
    pub timer: tokio::task::AbortHandle,
}

#[derive(Default)]
pub(super) struct PendingAckState {
    closed_order: VecDeque<u64>,
    pub closed_set: HashSet<u64>,
    pub entries: HashMap<String, PendingAckEntry>,
}

impl PendingAckState {
    pub fn mark_connection_closed(&mut self, connection_id: u64) -> Vec<PendingAckEntry> {
        if self.closed_set.insert(connection_id) {
            self.closed_order.push_back(connection_id);
            while self.closed_order.len() > MAX_CLOSED_CONNECTIONS {
                if let Some(oldest) = self.closed_order.pop_front() {
                    self.closed_set.remove(&oldest);
                }
            }
        }
        let matching_keys: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.connection_id == connection_id)
            .map(|(k, _)| k.clone())
            .collect();
        let mut drained = Vec::with_capacity(matching_keys.len());
        for key in matching_keys {
            if let Some(entry) = self.entries.remove(&key) {
                drained.push(entry);
            }
        }
        drained
    }
}

impl Broker {
    /// Demand-triggered Phase 2 user MCP discovery. Never fails the session:
    /// returns whatever servers became ready before the 15 s wall budget,
    /// cleans up stragglers via `cowboy/stopUserMcp`, and logs/emits ONLY
    /// bounded closed reason codes.
    pub(super) async fn discover_user_mcp(
        &self,
        client: &Arc<Client>,
        phase1: &Discovery,
    ) -> UserDiscovery {
        let deadline = tokio::time::Instant::now() + self.phase2_timeout();
        let mut sweep = self.sweep_done.clone();
        let sweep_ok =
            match tokio::time::timeout_at(deadline, sweep.wait_for(Option::is_some)).await {
                Ok(Ok(val)) => (*val).unwrap_or(false),
                _ => false,
            };
        if !sweep_ok {
            tracing::warn!(
                reason = "orphan_sweep_timeout",
                "user MCP discovery skipped: orphan sweep incomplete"
            );
            return UserDiscovery {
                tools: Vec::new(),
                instructions: vec![
                    "User MCP servers unavailable in this session (orphan_sweep_timeout)."
                        .to_owned(),
                ],
            };
        }
        if !self.keeper_user_mcp.get().copied().unwrap_or(false) {
            return UserDiscovery::default();
        }
        let Some(ledger) = client.ledger().cloned() else {
            return UserDiscovery::default();
        };
        let (candidates, mut notes) = match self.fetch_user_mcp_candidates(client, deadline).await {
            Ok(parsed) => parsed,
            Err(early) => return early,
        };
        if candidates.is_empty() {
            return UserDiscovery {
                tools: Vec::new(),
                instructions: notes,
            };
        }
        let ready =
            launch_user_mcp_candidates(client, &ledger, candidates, deadline, &mut notes).await;
        self.assemble_user_discovery(phase1, ready, notes)
    }

    async fn fetch_user_mcp_candidates(
        &self,
        client: &Arc<Client>,
        deadline: tokio::time::Instant,
    ) -> Result<(Vec<(String, String)>, Vec<String>), UserDiscovery> {
        let listed = match tokio::time::timeout_at(
            deadline,
            client.invoke_classified(USER_MCP_LIST_METHOD.into(), json!({})),
        )
        .await
        {
            Ok(Ok(reply)) => reply,
            _ => {
                tracing::warn!(reason = "list_failed", "user MCP listing unavailable");
                return Err(UserDiscovery {
                    tools: Vec::new(),
                    instructions: vec![
                        "User MCP servers unavailable in this session (list_failed).".to_owned(),
                    ],
                });
            }
        };
        if let Some(error) = listed.get("error") {
            let code = closed_reason_from_error(error);
            tracing::warn!(reason = code, "user MCP listing declined");
            if code == "disabled" {
                return Err(UserDiscovery::default());
            }
            return Err(UserDiscovery {
                tools: Vec::new(),
                instructions: vec![format!(
                    "User MCP servers unavailable in this session ({code})."
                )],
            });
        }
        let mut notes = Vec::new();
        if let Some(omitted) = listed.pointer("/result/omitted").and_then(Value::as_array) {
            for entry in omitted.iter().take(16) {
                let Some(name) = entry.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let reason = closed_reason_from_str(
                    entry
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("invalid_config"),
                );
                if reason == "not_allowed" {
                    continue;
                }
                tracing::warn!(server = %safe_label(name), reason, "user MCP server omitted");
                notes.push(format!(
                    "User MCP server {} omitted ({reason}).",
                    safe_label(name)
                ));
            }
        }
        let reserved_names: HashSet<String> = self
            .phase1_server_names
            .iter()
            .chain(std::iter::once(&self.memory_server))
            .filter(|name| !name.is_empty())
            .map(|name| name.to_ascii_lowercase())
            .collect();
        let mut candidates = Vec::new();
        if let Some(servers) = listed.pointer("/result/servers").and_then(Value::as_array) {
            for entry in servers.iter().take(MAX_USER_MCP_SERVERS) {
                let Some(name) = entry
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|n| crate::config::valid_mcp_server_name(n))
                else {
                    continue;
                };
                let Some(fingerprint) = entry
                    .get("fingerprint")
                    .and_then(Value::as_str)
                    .filter(|fp| fp.len() == 64 && fp.bytes().all(|b| b.is_ascii_hexdigit()))
                else {
                    continue;
                };
                let lower = name.to_ascii_lowercase();
                if lower.starts_with("cowboy") || reserved_names.contains(&lower) {
                    tracing::warn!(
                        server = %name,
                        reason = "reserved_name_collision",
                        "user MCP server dropped due to reserved name collision"
                    );
                    notes.push(format!(
                        "User MCP server {name} omitted (reserved_name_collision)."
                    ));
                    continue;
                }
                candidates.push((name.to_owned(), fingerprint.to_owned()));
            }
        }
        Ok((candidates, notes))
    }

    fn assemble_user_discovery(
        &self,
        phase1: &Discovery,
        mut ready_servers: Vec<(String, Arc<Server>, String, Vec<Value>)>,
        notes: Vec<String>,
    ) -> UserDiscovery {
        ready_servers.sort_by(|a, b| a.0.cmp(&b.0));
        let mut taken: HashSet<String> = FACADE_TOOLS
            .iter()
            .map(|name| (*name).to_owned())
            .chain(phase1.tools.iter().map(|t| t.name.clone()))
            .collect();
        let mut user_discovery = UserDiscovery {
            tools: Vec::new(),
            instructions: Vec::new(),
        };
        let mut total_instr_chars: usize = 0;
        let mut stored_servers = self.user_servers.lock();
        for (name, server, instructions, tools) in ready_servers {
            let index = stored_servers.len();
            stored_servers.push(server);
            if !instructions.is_empty() && total_instr_chars < MAX_USER_MCP_INSTRUCTIONS_TOTAL {
                let remaining = MAX_USER_MCP_INSTRUCTIONS_TOTAL - total_instr_chars;
                let clipped: String = instructions
                    .chars()
                    .take(MAX_USER_MCP_INSTRUCTIONS_PER_SERVER.min(remaining))
                    .collect();
                if !clipped.is_empty() {
                    total_instr_chars += clipped.chars().count();
                    user_discovery
                        .instructions
                        .push(format!("{name}: {clipped}"));
                }
            }
            add_user_tools(
                &mut user_discovery.tools,
                &mut taken,
                &name,
                Route::UserServer(index),
                tools,
            );
        }
        drop(stored_servers);
        user_discovery.instructions.extend(notes);
        user_discovery
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn call_user_server_guarded(
        self: &Arc<Self>,
        client: &Arc<Client>,
        ledger: &Arc<Ledger>,
        tool: &Tool,
        index: usize,
        params: &Value,
        arguments: Value,
        connection_id: u64,
    ) -> (Value, Option<SettlementGuard>, AckState) {
        let ack_protocol = params
            .get("ackProtocol")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if ack_protocol < 1 {
            return (
                refused("proxy does not support extension call acknowledgement"),
                None,
                AckState::None,
            );
        }
        let admitted = match ledger.admit_call() {
            Ok(admitted) => admitted,
            Err(reason) => return (refused(reason), None, AckState::None),
        };
        let turn = Ledger::admitted_turn(&admitted).to_owned();
        let server = { self.user_servers.lock().get(index).cloned() };
        let Some(server) = server else {
            return (
                refused(&format!("{}: server unavailable", tool.server)),
                Some(ledger.guard_call(admitted)),
                AckState::None,
            );
        };
        if server.is_closed_hint() {
            return (
                refused(&format!("{}: the server has ended", tool.server)),
                Some(ledger.guard_call(admitted)),
                AckState::None,
            );
        }
        let call_id = format!("{:032x}", rand::random::<u128>());
        if let Err(reason) = ledger.record_pending_call(&call_id) {
            return (
                refused(reason),
                Some(ledger.guard_call(admitted)),
                AckState::None,
            );
        }
        let request = json!({"name": tool.original, "arguments": arguments});
        let outcome = tokio::time::timeout(
            CALL_TIMEOUT,
            server.request_bounded(
                client,
                ledger,
                Some(&turn),
                "tools/call",
                request,
                Some(MAX_USER_MCP_RESULT_BYTES),
            ),
        )
        .await
        .unwrap_or_else(|_| Err(Failure::MaybeSent("timed out".into())));

        match outcome {
            Ok(result) => {
                let guard = ledger.guard_call(admitted);
                let ack =
                    self.register_pending_ack(call_id, connection_id, Arc::clone(ledger), guard);
                (json!({"result": result}), None, ack)
            }
            Err(Failure::Refused(reason)) => {
                tracing::warn!(
                    server = %tool.server,
                    reason = "tool_error",
                    "user MCP tool returned error"
                );
                let guard = ledger.guard_call(admitted);
                let ack =
                    self.register_pending_ack(call_id, connection_id, Arc::clone(ledger), guard);
                (refused(&format!("{}: {reason}", tool.server)), None, ack)
            }
            Err(Failure::NotSent(reason)) => {
                let _ = ledger.clear_pending_call(&call_id);
                (
                    refused(&format!("{}: {reason}", tool.server)),
                    Some(ledger.guard_call(admitted)),
                    AckState::None,
                )
            }
            Err(Failure::MaybeSent(_)) => {
                tracing::warn!(
                    server = %tool.server,
                    reason = "call_timeout",
                    "user MCP tool outcome unknown"
                );
                ledger.convert_pending_calls_to_uncertain(&[call_id]);
                (
                    json!({"error": {
                        "code": -32000,
                        "message": "Extension call result unknown; no replay",
                        "data": {"outcome": "unknown"},
                    }}),
                    Some(ledger.guard_call(admitted)),
                    AckState::AlreadySettled,
                )
            }
        }
    }

    /// Mark a user MCP server closed immediately upon `process/closed` or
    /// `process/exited` so subsequent calls fail locally without writing a
    /// pending-call sentinel.
    pub fn mark_user_server_closed(&self, process_id: &str) {
        let servers = self.user_servers.lock();
        for server in servers.iter() {
            if server.process == process_id {
                server.mark_closed();
            }
        }
    }

    /// Stop all keeper-owned user MCP servers on worker teardown.
    pub async fn stop_user_servers(&self, client: &Arc<Client>) {
        {
            let servers = self.user_servers.lock();
            for server in servers.iter() {
                server.mark_closed();
            }
        }
        if !self.keeper_user_mcp.get().copied().unwrap_or(false) {
            return;
        }
        let _ = tokio::time::timeout(
            USER_MCP_SWEEP_TIMEOUT,
            client.invoke_classified(USER_MCP_STOP_METHOD.into(), json!({})),
        )
        .await;
    }

    /// Register a `PendingAck` for a completed user-MCP `tools/call` reply.
    /// Starts the `15 s` ack timer AT REGISTRATION (never after socket write),
    /// and settles immediately with `AckState::AlreadySettled` if the socket
    /// already closed before registration. Strict lock discipline: ALWAYS drop
    /// `pending_acks` lock BEFORE touching `ledger` or dropping `SettlementGuard`.
    pub(super) fn register_pending_ack(
        self: &Arc<Self>,
        call_id: String,
        connection_id: u64,
        ledger: Arc<Ledger>,
        guard: SettlementGuard,
    ) -> AckState {
        let already_closed = {
            let mut state = self.pending_acks.lock();
            if state.closed_set.contains(&connection_id) {
                true
            } else {
                let broker = Arc::clone(self);
                let timeout_id = call_id.clone();
                let ack_timeout = self.ack_timeout();
                let timer = tokio::spawn(async move {
                    tokio::time::sleep(ack_timeout).await;
                    broker.fail_pending_ack(&timeout_id);
                })
                .abort_handle();
                state.entries.insert(
                    call_id.clone(),
                    PendingAckEntry {
                        call_id: call_id.clone(),
                        connection_id,
                        ledger: Arc::clone(&ledger),
                        guard,
                        timer,
                    },
                );
                return AckState::Pending(call_id);
            }
        };
        if already_closed {
            // Outside `pending_acks` lock: convert sentinel once, then drop guard.
            ledger.convert_pending_calls_to_uncertain(&[call_id]);
            drop(guard);
            return AckState::AlreadySettled;
        }
        unreachable!()
    }

    /// Handle local `cowboy/extensionCallAck({ callId })`.
    pub fn acknowledge_call(&self, params: &Value) -> Value {
        let Some(call_id) = params.get("callId").and_then(Value::as_str).filter(|id| {
            id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        }) else {
            return json!({"result": {"acknowledged": false}});
        };
        let entry = self.pending_acks.lock().entries.remove(call_id);
        let Some(entry) = entry else {
            return json!({"result": {"acknowledged": false}});
        };
        entry.timer.abort();
        // Outside `pending_acks` lock: persist sentinel clear, THEN drop guard.
        let cleared = entry.ledger.clear_pending_call(&entry.call_id);
        drop(entry.guard);
        json!({"result": {"acknowledged": cleared}})
    }

    /// Atomically settle an unacknowledged user-MCP call as uncertain (called
    /// on timeout, `Relays::undelivered`/`drop`/`SendError`, or correlated
    /// `cowboy/uncertainEffect({ callId })`). Returns `true` if settled here.
    pub fn fail_pending_ack(&self, call_id: &str) -> bool {
        let entry = self.pending_acks.lock().entries.remove(call_id);
        let Some(entry) = entry else {
            return false;
        };
        entry.timer.abort();
        // Outside `pending_acks` lock: convert sentinel once, THEN drop guard.
        entry
            .ledger
            .convert_pending_calls_to_uncertain(&[entry.call_id]);
        drop(entry.guard);
        true
    }

    /// Called when a mutation WebSocket connection closes: records
    /// `connection_id` closed and converts all registered `PendingAck` entries
    /// for that connection in a single atomic persist before dropping guards.
    pub fn connection_closed(&self, connection_id: u64) {
        let drained = self
            .pending_acks
            .lock()
            .mark_connection_closed(connection_id);
        if drained.is_empty() {
            return;
        }
        let ledger = Arc::clone(&drained[0].ledger);
        let mut call_ids = Vec::with_capacity(drained.len());
        let mut guards = Vec::with_capacity(drained.len());
        for entry in drained {
            entry.timer.abort();
            call_ids.push(entry.call_id);
            guards.push(entry.guard);
        }
        // Outside `pending_acks` lock: batch convert, THEN drop all guards.
        ledger.convert_pending_calls_to_uncertain(&call_ids);
        drop(guards);
    }
}

type ServerInitOutcome = (
    String,
    Result<(Arc<Server>, String, Vec<Value>), &'static str>,
);

async fn launch_user_mcp_candidates(
    client: &Arc<Client>,
    ledger: &Arc<Ledger>,
    candidates: Vec<(String, String)>,
    deadline: tokio::time::Instant,
    notes: &mut Vec<String>,
) -> Vec<(String, Arc<Server>, String, Vec<Value>)> {
    let sem = Arc::new(Semaphore::new(MAX_USER_MCP_CONCURRENCY));
    let in_flight: Arc<parking_lot::Mutex<HashMap<String, String>>> =
        Arc::new(parking_lot::Mutex::new(HashMap::new()));
    let mut join_set: tokio::task::JoinSet<ServerInitOutcome> = tokio::task::JoinSet::new();

    for (name, fingerprint) in candidates {
        let process_id = format!("{USER_MCP_PROCESS_PREFIX}{:032x}", rand::random::<u128>());
        in_flight.lock().insert(name.clone(), process_id.clone());
        let sem = Arc::clone(&sem);
        let client = Arc::clone(client);
        let ledger = Arc::clone(ledger);
        let in_flight_ref = Arc::clone(&in_flight);
        join_set.spawn(async move {
            let Ok(_permit) = tokio::time::timeout_at(deadline, sem.acquire_owned()).await else {
                return (name, Err("init_timeout"));
            };
            let Ok(_permit) = _permit else {
                return (name, Err("start_failed"));
            };
            let server_deadline =
                deadline.min(tokio::time::Instant::now() + USER_MCP_SERVER_INIT_TIMEOUT);
            let result = start_and_init_user_server(
                &client,
                &ledger,
                &name,
                &fingerprint,
                &process_id,
                server_deadline,
            )
            .await;
            in_flight_ref.lock().remove(&name);
            (name, result)
        });
    }

    let mut ready_servers = Vec::new();
    loop {
        match tokio::time::timeout_at(deadline, join_set.join_next()).await {
            Ok(Some(Ok((name, Ok((server, instructions, tools)))))) => {
                ready_servers.push((name, server, instructions, tools));
            }
            Ok(Some(Ok((name, Err(reason))))) => {
                tracing::warn!(server = %name, reason, "user MCP server unavailable");
                notes.push(format!("User MCP server {name} unavailable ({reason})."));
            }
            Ok(Some(Err(_))) => {}
            Ok(None) => break,
            Err(_) => {
                join_set.abort_all();
                let leftovers: Vec<(String, String)> = in_flight.lock().drain().collect();
                for (name, process_id) in leftovers {
                    tracing::warn!(
                        server = %name,
                        reason = "init_timeout",
                        "user MCP server timed out during discovery"
                    );
                    notes.push(format!(
                        "User MCP server {name} unavailable (init_timeout)."
                    ));
                    spawn_stop_user_process(Arc::clone(client), process_id);
                }
                break;
            }
        }
    }
    ready_servers
}

async fn start_and_init_user_server(
    client: &Arc<Client>,
    ledger: &Ledger,
    name: &str,
    fingerprint: &str,
    process_id: &str,
    deadline: tokio::time::Instant,
) -> Result<(Arc<Server>, String, Vec<Value>), &'static str> {
    let start_reply = match tokio::time::timeout_at(
        deadline,
        client.invoke_classified(
            USER_MCP_START_METHOD.into(),
            json!({
                "name": name,
                "processId": process_id,
                "fingerprint": fingerprint,
            }),
        ),
    )
    .await
    {
        Ok(Ok(reply)) => reply,
        Ok(Err(true)) => {
            spawn_stop_user_process(Arc::clone(client), process_id.to_owned());
            return Err("start_failed");
        }
        Ok(Err(false)) => return Err("start_failed"),
        Err(_) => {
            spawn_stop_user_process(Arc::clone(client), process_id.to_owned());
            return Err("init_timeout");
        }
    };
    if let Some(error) = start_reply.get("error") {
        // Keeper returned a deterministic Outcome::Completed error: no process started.
        return Err(closed_reason_from_error(error));
    }
    if start_reply
        .pointer("/result/processId")
        .and_then(Value::as_str)
        != Some(process_id)
    {
        spawn_stop_user_process(Arc::clone(client), process_id.to_owned());
        return Err("start_failed");
    }

    let server = Arc::new(Server::new(process_id.to_owned()));
    let init = tokio::time::timeout_at(deadline, async {
        let ready = server
            .request_bounded(
                client,
                ledger,
                None,
                "initialize",
                json!({"protocolVersion": MCP_PROTOCOL, "capabilities": {},
                    "clientInfo": {"name": "cowboy-antigravity", "version": "1.0.0"}}),
                Some(MAX_USER_MCP_RESULT_BYTES),
            )
            .await
            .map_err(|_| "protocol_error")?;
        server
            .notify(client, "notifications/initialized")
            .await
            .map_err(|_| "protocol_error")?;
        let listed = server
            .request_bounded(
                client,
                ledger,
                None,
                "tools/list",
                json!({}),
                Some(MAX_USER_MCP_RESULT_BYTES),
            )
            .await
            .map_err(|_| "protocol_error")?;
        let instructions = ready
            .get("instructions")
            .and_then(Value::as_str)
            .map(|text| {
                text.chars()
                    .take(MAX_USER_MCP_INSTRUCTIONS_PER_SERVER)
                    .collect()
            })
            .unwrap_or_default();
        let tools = listed
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok::<_, &'static str>((instructions, tools))
    })
    .await;

    match init {
        Ok(Ok((instructions, tools))) => Ok((server, instructions, tools)),
        Ok(Err(reason)) => {
            server.mark_closed();
            spawn_stop_user_process(Arc::clone(client), process_id.to_owned());
            Err(reason)
        }
        Err(_) => {
            server.mark_closed();
            spawn_stop_user_process(Arc::clone(client), process_id.to_owned());
            Err("init_timeout")
        }
    }
}

fn spawn_stop_user_process(client: Arc<Client>, process_id: String) {
    tokio::spawn(async move {
        let _ = tokio::time::timeout(
            USER_MCP_SWEEP_TIMEOUT,
            client.invoke_classified(
                USER_MCP_STOP_METHOD.into(),
                json!({"processId": process_id}),
            ),
        )
        .await;
    });
}

fn safe_label(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "server".to_owned()
    } else {
        cleaned
    }
}

fn closed_reason_from_error(error: &Value) -> &'static str {
    let code = error
        .pointer("/data/code")
        .and_then(Value::as_str)
        .unwrap_or("");
    closed_reason_from_str(code)
}

fn closed_reason_from_str(code: &str) -> &'static str {
    match code {
        "user_mcp_disabled" | "disabled" => "disabled",
        "user_mcp_config_drift" | "config_drift" => "config_drift",
        "user_mcp_not_allowed" | "not_allowed" => "not_allowed",
        "unsupported_transport" => "unsupported_transport",
        "unresolved_variable" => "unresolved_variable",
        "reserved_name_collision" => "reserved_name_collision",
        "server_limit" | "user_mcp_capacity" => "server_limit",
        "user_mcp_stopped" => "stopped",
        "user_mcp_manager_gate_required" | "user_mcp_unsupported" => "disabled",
        "init_timeout" => "init_timeout",
        "process_exited" => "process_exited",
        "protocol_error" => "protocol_error",
        "result_too_large" => "result_too_large",
        "call_timeout" => "call_timeout",
        "orphan_sweep_timeout" => "orphan_sweep_timeout",
        _ => "invalid_config",
    }
}

/// Add user-MCP server tools with a mandatory `<server>_` prefix, bounded to
/// `MAX_USER_MCP_TOOLS_PER_SERVER` (64) per server and `MAX_USER_MCP_TOOLS_TOTAL` (128) overall.
pub(super) fn add_user_tools(
    out: &mut Vec<Tool>,
    taken: &mut HashSet<String>,
    server: &str,
    route: Route,
    tools: Vec<Value>,
) {
    let mut added_for_server = 0;
    for tool in tools {
        if added_for_server >= MAX_USER_MCP_TOOLS_PER_SERVER
            || out.len() >= MAX_USER_MCP_TOOLS_TOTAL
        {
            break;
        }
        let Some(original) = tool
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| tool_name(name))
        else {
            continue;
        };
        let Some(name) = user_tool_alias(taken, server, original) else {
            continue;
        };
        out.push(Tool {
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
        added_for_server += 1;
    }
}

/// Mandatory `<server>_<tool>` prefix (sanitizing `.` to `_` so names match
/// `^[A-Za-z0-9_-]{1,64}$`), never returning a bare unprefixed tool name.
pub(super) fn user_tool_alias(
    taken: &mut HashSet<String>,
    server: &str,
    original: &str,
) -> Option<String> {
    let safe_server: String = server
        .chars()
        .map(|c| if c == '.' { '_' } else { c })
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(32)
        .collect();
    if safe_server.is_empty() {
        return None;
    }
    let base = format!("{safe_server}_{original}");
    let candidates =
        std::iter::once(base.chars().take(64).collect::<String>()).chain((2..100).map(|n| {
            let suffix = format!("_{n}");
            let head: String = base.chars().take(64 - suffix.len()).collect();
            format!("{head}{suffix}")
        }));
    candidates
        .into_iter()
        .find(|candidate| tool_name(candidate) && taken.insert(candidate.clone()))
}
