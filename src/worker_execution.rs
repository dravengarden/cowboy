//! Worker-owned endpoint for the execution protocol. The native client sees
//! one authenticated socket: loopback TCP, or for an account-sandboxed unit
//! the Unix socket the daemon bound into it. Placement and retries are not
//! model tools.

mod extension_broker;
mod ledger;
mod memory_bridge;
mod post_edit;
mod pre_auth;
mod relay;
pub use extension_broker::{
    Broker as ExtensionBroker, Plan as ExtensionPlan, ServerPlan, USER_MCP_INITIALIZATION_KEY,
    USER_MCP_LIST_METHOD, USER_MCP_PROCESS_PREFIX, USER_MCP_START_METHOD, USER_MCP_STOP_METHOD,
};
pub use ledger::{LEDGER_ENV, Ledger, MAX_PROCESSES};
pub use memory_bridge::MemoryBridge;
pub use post_edit::{PostEditBrokerSession, QueuedPostEdit, unix_socket_has_post_edit_headroom};
#[cfg(test)]
use pre_auth::classify_request_line;
use pre_auth::{PrefixedHandshakeStream, RequestLineRoute, peek_request_line};

use std::collections::BTreeMap;
use std::os::fd::OwnedFd;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use futures::StreamExt as _;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio_tungstenite::tungstenite::{
    Message,
    protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
};

use crate::execution_environment::BindingV1;
use crate::execution_protocol::{
    self as wire, Command, Invocation, Outcome, Response, RuntimeReply, RuntimeRequest, Scope,
};
use crate::machine_protocol::execution::{Refusal, Response as MachineResponse};
use crate::runtime_wire::Frame;

pub const DESCRIPTOR_ENV: &str = "COWBOY_EXECUTION_DESCRIPTOR";
const MAX_PENDING: usize = 24;
pub const MAX_PENDING_BYTES: usize = 16 * 1024 * 1024;
// A validated binding is at most 16 KiB before JSON escaping. Bound the
// complete control envelope too, and reserve its worst-case aggregate so
// admitted bulk inputs cannot starve polling or result observation.
const MAX_CONTROL_BYTES: usize = 64 * 1024;
const CONTROL_RESERVE_BYTES: usize = MAX_PENDING * MAX_CONTROL_BYTES;
const RETRY: Duration = Duration::from_secs(30);

pub struct Pending {
    request: RuntimeRequest,
    /// The response and how often the request was sent before it.
    sender: oneshot::Sender<(MachineResponse, u32)>,
    sent: Option<Instant>,
    sends: u32,
    pub bytes: usize,
}

pub struct Client {
    session: String,
    epoch: String,
    binding: BindingV1,
    pub pending: Mutex<BTreeMap<String, Pending>>,
    notify: mpsc::UnboundedSender<()>,
    event_cursor: Mutex<Option<u64>>,
    /// The trusted process ledger of an account-sandboxed session.
    ledger: OnceLock<Arc<Ledger>>,
    /// The session's memory relay, bridged for an account-sandboxed unit.
    memory: OnceLock<MemoryBridge>,
    /// The session's extension servers, run by the worker for the unit.
    extensions: OnceLock<Arc<ExtensionBroker>>,
    /// Relays whose reply the unit is still owed (design L1c).
    relays: Arc<crate::account_sandbox::RelayQuiet>,
    /// Unsequenced Antigravity post-edit diagnostic pull state (R4).
    post_edit: post_edit::State,
}

/// Wire calls at least this slow are logged individually.
const SLOW_CALL: Duration = Duration::from_secs(5);

impl Client {
    pub(crate) fn relays(&self) -> &Arc<crate::account_sandbox::RelayQuiet> {
        &self.relays
    }

    pub fn new(
        session: String,
        epoch: String,
        binding: BindingV1,
        notify: mpsc::UnboundedSender<()>,
    ) -> Arc<Self> {
        Arc::new(Self {
            session,
            epoch,
            binding,
            pending: Mutex::default(),
            notify,
            event_cursor: Mutex::new(None),
            ledger: OnceLock::new(),
            memory: OnceLock::new(),
            extensions: OnceLock::new(),
            relays: Arc::default(),
            post_edit: post_edit::State::default(),
        })
    }

    pub fn attach_post_edit_broker(&self, protocol: u16) -> (u64, mpsc::Receiver<QueuedPostEdit>) {
        self.post_edit.attach_broker(protocol)
    }

    pub fn detach_post_edit_broker(&self, conn_id: u64) {
        self.post_edit.detach_broker(conn_id);
    }

    pub fn set_active_turn_nonce(&self, turn: Option<(String, String)>) {
        self.post_edit.set_active_turn(turn);
    }

    pub fn prepare_post_edit_send(&self, item: &mut QueuedPostEdit, current_conn_id: u64) -> bool {
        self.post_edit
            .prepare_send(item, current_conn_id, &self.relays)
    }

    pub fn refuse_post_edit_local(&self, request_id: &str, reason: &str) {
        self.post_edit.refuse_local(request_id, reason);
    }

    pub fn complete_post_edit_reply(
        &self,
        reply: crate::post_edit_inspect::PostEditInspectReply,
        current_conn_id: u64,
    ) {
        let scope = crate::post_edit_inspect::PostEditScope::new(
            self.binding.id.clone(),
            self.binding.revision,
            self.binding.environment.incarnation.clone(),
        );
        self.post_edit.complete_reply(
            reply,
            &self.session,
            &self.epoch,
            &scope,
            current_conn_id,
            &self.relays,
        );
    }

    pub async fn inspect_post_edit(&self, unit_generation: u64, params: &Value) -> Value {
        let scope = crate::post_edit_inspect::PostEditScope::new(
            self.binding.id.clone(),
            self.binding.revision,
            self.binding.environment.incarnation.clone(),
        );
        self.post_edit
            .inspect(
                &self.session,
                &self.epoch,
                scope,
                self.binding.managed.is_some(),
                unit_generation,
                &self.relays,
                params,
            )
            .await
    }

    pub fn frames(&self, reconnect: bool) -> Vec<Frame> {
        let mut pending = self.pending.lock();
        pending.retain(|_, pending| !pending.sender.is_closed());
        pending
            .values_mut()
            .filter_map(|pending| {
                if reconnect || pending.sent.is_none_or(|sent| sent.elapsed() >= RETRY) {
                    pending.sent = Some(Instant::now());
                    Some(Frame::ExecutionRequest {
                        request: Box::new(pending.request.clone()),
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// `frame` is about to be written: one send attempt of its request. A
    /// write that fails part-way still counts (it may have reached the
    /// broker); frames after it in the batch are never attempted.
    pub fn sending<'frame>(&self, frame: &'frame Frame) -> &'frame Frame {
        if let Frame::ExecutionRequest { request } = frame
            && let Some(pending) = self.pending.lock().get_mut(&request.request_id)
        {
            pending.sends = pending.sends.saturating_add(1);
        }
        frame
    }

    pub fn complete(&self, reply: RuntimeReply) {
        if reply.session_id != self.session
            || reply.worker_epoch != self.epoch
            || reply.scope != Scope::from_binding(&self.binding)
        {
            return;
        }
        if matches!(
            reply.response,
            MachineResponse::Refused {
                reason: Refusal::Unavailable | Refusal::Capacity
            }
        ) {
            // The same pending request will be forwarded again. Its invocation
            // retains its original effect identity, even when a reply was lost.
            if let Some(pending) = self.pending.lock().get_mut(&reply.request_id) {
                pending.sent = Some(Instant::now() - (RETRY - Duration::from_secs(1)));
                let notify = self.notify.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    let _ = notify.send(());
                });
            }
            return;
        }
        if let Some(pending) = self.pending.lock().remove(&reply.request_id) {
            let _ = pending.sender.send((reply.response, pending.sends));
        }
    }

    pub async fn call(&self, command: Command) -> Result<Response> {
        self.call_queued(command).await?
    }

    /// The outer error means the request was never queued for sending.
    async fn call_queued(&self, command: Command) -> Result<Result<Response>> {
        Ok(self
            .call_counted(command)
            .await?
            .map(|(response, _)| response))
    }

    /// `call_queued`, with how often the request was sent before its
    /// response arrived.
    async fn call_counted(&self, command: Command) -> Result<Result<(Response, u32)>> {
        let started = Instant::now();
        let operation = match &command {
            Command::Invoke { invocation, .. } => Some(invocation.operation_id.clone()),
            _ => None,
        };
        let id = format!("{:032x}", rand::random::<u128>());
        let request = RuntimeRequest {
            session_id: self.session.clone(),
            worker_epoch: self.epoch.clone(),
            request_id: id.clone(),
            binding: self.binding.clone(),
            command,
        };
        let bytes = serde_json::to_vec(&request)?.len();
        let invocation = matches!(request.command, Command::Invoke { .. });
        ensure!(
            bytes
                <= if invocation {
                    wire::MAX_FRAME_BYTES
                } else {
                    MAX_CONTROL_BYTES
                },
            "execution request exceeds transport limit"
        );
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = self.pending.lock();
            pending.retain(|_, pending| !pending.sender.is_closed());
            let (count_limit, byte_limit) = if invocation {
                (MAX_PENDING - 1, MAX_PENDING_BYTES - CONTROL_RESERVE_BYTES)
            } else {
                (MAX_PENDING, MAX_PENDING_BYTES)
            };
            ensure!(
                pending.len() < count_limit
                    && pending.values().map(|entry| entry.bytes).sum::<usize>() + bytes
                        <= byte_limit,
                "execution transport is at capacity"
            );
            pending.insert(
                id.clone(),
                Pending {
                    request,
                    sender,
                    sent: None,
                    sends: 0,
                    bytes,
                },
            );
        }
        let _ = self.notify.send(());
        let response = tokio::time::timeout(Duration::from_secs(180), receiver).await;
        self.pending.lock().remove(&id);
        match response {
            Ok(Ok((MachineResponse::Call { response }, sends))) => {
                if let Some(operation) = operation {
                    // A transport-level failure is a WARN; a tool that ran
                    // and reported its own error is ordinary agent traffic.
                    let failure = match &response {
                        Response::Refused { .. } => Some("refused"),
                        Response::Operation {
                            outcome: Outcome::Unknown,
                        } => Some("outcome_unknown"),
                        Response::Operation {
                            outcome: Outcome::Missing,
                        } => Some("outcome_missing"),
                        Response::Operation {
                            outcome: Outcome::ResultExpired,
                        } => Some("result_expired"),
                        _ => None,
                    };
                    let tool_error = matches!(&response, Response::Operation { outcome: Outcome::Completed { reply } } if reply.get("error").is_some());
                    if let Some(reason) = failure {
                        tracing::warn!(event_name = "cowboy.execution.call_failed", session = %self.session, environment_id = %self.binding.environment.id, %operation, reason, duration_ms = started.elapsed().as_secs_f64() * 1000.0, "execution call failed");
                    } else if tool_error {
                        tracing::debug!(event_name = "cowboy.execution.tool_error", session = %self.session, environment_id = %self.binding.environment.id, %operation, reason = "tool_error", duration_ms = started.elapsed().as_secs_f64() * 1000.0, "execution tool reported an error");
                    } else if started.elapsed() >= SLOW_CALL {
                        // Every completed tool call already records its total
                        // and backend time (tool_overhead); a wire call is
                        // kept only when slow, for tail diagnosis.
                        tracing::info!(event_name = "cowboy.execution.call_observed", session = %self.session, environment_id = %self.binding.environment.id, %operation, duration_ms = started.elapsed().as_secs_f64() * 1000.0, "slow execution invocation observed");
                    }
                }
                Ok(Ok((response, sends)))
            }
            _ => {
                tracing::warn!(event_name = "cowboy.execution.response_timeout", session = %self.session, environment_id = %self.binding.environment.id, reason = "response_unavailable", duration_ms = started.elapsed().as_secs_f64() * 1000.0, "execution response unavailable");
                Ok(Err(anyhow::anyhow!(
                    "execution response unavailable; original operation was not replayed under a new identity"
                )))
            }
        }
    }

    /// Read a remote managed child's current round from its execution target.
    /// The target keeper announces the constraint it enforces and where the
    /// Machine wrote the round; neither is taken from this worker's launch.
    pub async fn managed_round(
        &self,
        profile: cowboy_provider_sdk::ManagedRuntimeProfile,
    ) -> Result<crate::managed_calls::round::RoundMarker> {
        use base64::Engine as _;
        let Response::Ready { initialization, .. } = self.call(Command::Describe).await? else {
            bail!("execution environment unavailable");
        };
        let announced = initialization
            .get(wire::MANAGED_INITIALIZATION_KEY)
            .context("execution target does not enforce a managed profile")?;
        ensure!(
            announced.get("schema") == Some(&json!(1))
                && announced
                    .get("profile")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    == Some(profile),
            "execution target enforces a different managed profile"
        );
        let path = announced
            .get("roundPath")
            .and_then(Value::as_str)
            .filter(|path| Path::new(path).is_absolute())
            .context("execution target did not name its round")?;
        let url = url::Url::from_file_path(path)
            .map_err(|()| anyhow::anyhow!("invalid managed round path"))?;
        let reply = self
            .invoke("fs/readFile".to_owned(), json!({"path": url.as_str()}))
            .await?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(
            reply
                .pointer("/result/dataBase64")
                .and_then(Value::as_str)
                .context("managed round is unreadable")?,
        )?;
        ensure!(
            bytes.len() as u64 <= crate::managed_calls::round::MAX_ROUND_BYTES,
            "managed round exceeds its limit"
        );
        let round: crate::managed_calls::round::RoundMarker = serde_json::from_slice(&bytes)?;
        ensure!(round.schema == 1, "unsupported managed round");
        Ok(round)
    }

    pub async fn invoke(&self, method: String, params: Value) -> Result<Value> {
        self.invoke_classified(method, params).await.map_err(|_| {
            anyhow::anyhow!(
                "execution operation refused or its final result is unknown; do not replay"
            )
        })
    }

    /// One invocation; its error says whether the target may have admitted
    /// the effect (`true`: lost reply or unknown outcome) or certainly did
    /// not (`false`: refused before admission, or never sent).
    async fn invoke_classified(&self, method: String, params: Value) -> Result<Value, bool> {
        let started = std::time::Instant::now();
        let invocation = Invocation {
            // The suffix asks the keeper for the backend's own duration.
            operation_id: new_operation_id(),
            method,
            params,
        };
        let method = invocation.method.clone();
        if !invocation.validate() {
            return Err(false);
        }
        let operation_id = invocation.operation_id.clone();
        let mut command = Command::Invoke {
            invocation,
            wait_ms: wire::MAX_WAIT_MS,
        };
        let mut admitted = false;
        loop {
            // A request the transport refused to queue was never sent; any
            // later loss may follow admission.
            let queued = self.call_counted(command).await;
            let response = match queued {
                Ok(Ok((response, sends))) => {
                    // Sent more than once: an earlier send's reply was lost
                    // after the keeper may have admitted it, so a refusal of
                    // a later send proves nothing about the effect (F7).
                    admitted |= sends > 1;
                    response
                }
                Ok(Err(_)) => return Err(true),
                Err(_) => return Err(admitted),
            };
            match response {
                Response::Operation {
                    outcome: Outcome::Completed { mut reply },
                } => {
                    self.observe_tool_call(&method, &operation_id, started, &mut reply);
                    return Ok(reply);
                }
                Response::Operation {
                    outcome: Outcome::Pending,
                } => {
                    admitted = true;
                    command = Command::Observe {
                        operation_id: operation_id.clone(),
                        wait_ms: wire::MAX_WAIT_MS,
                    };
                }
                // The Machine resends a request itself when a bulk lane
                // fails, below this count: a refusal of an effectful request
                // may answer such a resend while an earlier send was admitted.
                // Fail closed: any refusal of an effectful request may answer
                // a copy the worker cannot see (a Machine resend) while
                // another copy was admitted.
                Response::Refused { .. } => {
                    return Err(admitted || ledger::effectful(&method));
                }
                _ => return Err(true),
            }
        }
    }
}

impl Client {
    /// One tool call's Cowboy overhead: its whole duration less the time the
    /// backend spent on it, which includes the command itself and any wait
    /// the caller requested (a long-polled read). Removes the keeper's
    /// duration member, so clients see the backend's reply unchanged.
    fn observe_tool_call(
        &self,
        method: &str,
        operation: &str,
        started: std::time::Instant,
        reply: &mut Value,
    ) {
        let total_ms = started.elapsed().as_secs_f64() * 1000.0;
        let executed_ms = reply
            .as_object_mut()
            .and_then(|object| object.remove(crate::execution_protocol::EXECUTED_MS))
            .and_then(|value| value.as_u64());
        match executed_ms {
            // `duration_ms` is the metric: `cowboy logs metrics` histograms it
            // per event, so this event's histogram is the Cowboy overhead. One
            // per call: the local store keeps it, the journal does not
            // (logs::EVIDENCE_ONLY).
            Some(executed_ms) => tracing::info!(
                target: "cowboy::evidence",
                event_name = "cowboy.execution.tool_overhead",
                session = %self.session,
                method,
                %operation,
                duration_ms = (total_ms - executed_ms as f64).max(0.0),
                total_ms,
                executed_ms,
                "execution tool call overhead"
            ),
            // A keeper that predates timing: only the whole duration is known.
            None => tracing::info!(
                target: "cowboy::evidence",
                event_name = "cowboy.execution.tool_untimed",
                session = %self.session,
                method,
                %operation,
                total_ms,
                "execution tool call without backend timing"
            ),
        }
    }
}

pub struct Endpoint {
    pub client: Arc<Client>,
    descriptor: Option<PathBuf>,
    listener: tokio::task::JoinHandle<()>,
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.listener.abort();
        if let Some(descriptor) = &self.descriptor {
            let _ = std::fs::remove_file(descriptor);
        }
    }
}

fn bearer_token() -> String {
    format!(
        "{:032x}{:032x}",
        rand::random::<u128>(),
        rand::random::<u128>()
    )
}

const MAX_PRE_AUTH_SOCKETS: usize = 4;
const MAX_NON_MUTATION_PRE_AUTH_SOCKETS: usize = 3;
const MAX_PRE_AUTH_CLASSIFY_SOCKETS: usize = 2;
const PRE_AUTH_CLASSIFY_TIMEOUT: Duration = Duration::from_millis(100);
const PRE_AUTH_CLASSIFY_EVICT_AFTER: Duration = Duration::from_millis(50);
const PRE_AUTH_UPGRADE_TIMEOUT: Duration = Duration::from_millis(500);
const PRE_AUTH_STALL_EVICT_AFTER: Duration = Duration::from_millis(200);
const MAX_PRE_AUTH_HEADER_BYTES: usize = 4 * 1024;
const MAX_INIT_FRAME_BYTES: usize = 4 * 1024;
const MAX_INIT_WIRE_BYTES: usize = MAX_INIT_FRAME_BYTES + 128;
const INIT_FRAME_TIMEOUT: Duration = Duration::from_secs(1);
const DIAGNOSTICS_MAX_FRAME_BYTES: usize = 64 * 1024;
const DIAGNOSTICS_PATH: &str = "/diagnostics";

struct PreAuthSlot {
    started_at: Instant,
    abort: Option<tokio::task::AbortHandle>,
    /// True when admitted into the reserved 4th reconnect slot while the
    /// mutation slot is vacant; `/diagnostics` is rejected on this ticket (M5).
    mutation_only: bool,
}

struct ClassifySlot {
    started_at: Instant,
    abort: Option<tokio::task::AbortHandle>,
}

#[derive(Default)]
struct AdmissionState {
    next_id: u64,
    pre_auth: std::collections::HashMap<u64, PreAuthSlot>,
    classifying: std::collections::HashMap<u64, ClassifySlot>,
}

struct PreAuthCleanup {
    state: Arc<Mutex<AdmissionState>>,
    id: u64,
}

impl Drop for PreAuthCleanup {
    fn drop(&mut self) {
        let mut guard = self.state.lock();
        guard.pre_auth.remove(&self.id);
        guard.classifying.remove(&self.id);
    }
}

/// Drops an authenticated mutation socket permit and notifies `RelayQuiet`
/// (`socket_ended`) exclusively for mutation sockets — never for `/diagnostics`
/// or unauthenticated/pre-auth sockets.
struct MutationSocketGuard {
    relays: Arc<crate::account_sandbox::RelayQuiet>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl Drop for MutationSocketGuard {
    fn drop(&mut self) {
        self.relays.socket_ended();
    }
}

/// Separate single-slot admission for `/` (mutation) and `/diagnostics`
/// (post-edit pull), with a bounded pre-auth pool that evicts a stalled
/// pre-auth socket when the mutation slot is vacant so reconnect / `cowboy/drained`
/// can never be starved (R3-H8).
struct Admission {
    token: String,
    client: Arc<Client>,
    mutation_slot: Arc<Semaphore>,
    diagnostics_slot: Arc<Semaphore>,
    state: Arc<Mutex<AdmissionState>>,
    tasks: tokio::task::JoinSet<()>,
}

impl Admission {
    fn new(token: String, client: Arc<Client>) -> Self {
        Self {
            token,
            client,
            mutation_slot: Arc::new(Semaphore::new(1)),
            diagnostics_slot: Arc::new(Semaphore::new(1)),
            state: Arc::new(Mutex::new(AdmissionState::default())),
            tasks: tokio::task::JoinSet::new(),
        }
    }

    fn admit<S>(&mut self, socket: S, unit_generation: u64)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let mut guard = self.state.lock();
        let mutation_vacant = self.mutation_slot.available_permits() > 0;
        // R3-M4: When all 3 general pre-auth slots are occupied and mutation is
        // vacant, classify the HTTP request line in a short bounded stage BEFORE
        // assigning the 4th mutation-only ticket so a stalled /diagnostics
        // handshake can never occupy the mutation reserve.
        let classify_first = guard.pre_auth.len() >= MAX_NON_MUTATION_PRE_AUTH_SOCKETS;
        if classify_first {
            if !mutation_vacant {
                return;
            }
            if guard.classifying.len() >= MAX_PRE_AUTH_CLASSIFY_SOCKETS {
                if let Some((&evict_id, _)) = guard
                    .classifying
                    .iter()
                    .filter(|(_, slot)| slot.started_at.elapsed() >= PRE_AUTH_CLASSIFY_EVICT_AFTER)
                    .min_by_key(|(_, slot)| slot.started_at)
                {
                    if let Some(evicted) = guard.classifying.remove(&evict_id)
                        && let Some(abort) = evicted.abort
                    {
                        abort.abort();
                    }
                } else {
                    return;
                }
            }
        }
        guard.next_id = guard.next_id.saturating_add(1);
        let id = guard.next_id;
        let state = Arc::clone(&self.state);
        let client = Arc::clone(&self.client);
        let token = self.token.clone();
        let mutation_slot = Arc::clone(&self.mutation_slot);
        let diagnostics_slot = Arc::clone(&self.diagnostics_slot);
        let cleanup = PreAuthCleanup {
            state: Arc::clone(&state),
            id,
        };
        let abort = self.tasks.spawn(async move {
            let _cleanup = cleanup;
            let session = client.session.clone();
            let upgraded = if classify_first {
                classify_then_promote_mutation_reserve(
                    socket,
                    &token,
                    Arc::clone(&client),
                    unit_generation,
                    id,
                    Arc::clone(&state),
                    mutation_slot,
                    diagnostics_slot,
                )
                .await
            } else {
                upgrade_and_classify(
                    socket,
                    None,
                    &token,
                    Arc::clone(&client),
                    unit_generation,
                    id,
                    Arc::clone(&state),
                    mutation_slot,
                    diagnostics_slot,
                )
                .await
            };
            match upgraded {
                Ok(UpgradedConnection::Mutation {
                    websocket,
                    initialized,
                    _guard,
                }) => {
                    if serve_mutation(websocket, initialized, client).await.is_err() {
                        tracing::warn!(event_name = "cowboy.execution.disconnected", %session, reason = "endpoint_failed", "execution endpoint disconnected");
                    }
                    drop(_guard);
                }
                Ok(UpgradedConnection::Diagnostics {
                    websocket,
                    initialized,
                    _permit,
                }) => {
                    let _ =
                        serve_diagnostics(websocket, initialized, client, unit_generation).await;
                    drop(_permit);
                }
                Err(_) => {}
            }
        });
        if classify_first {
            guard.classifying.insert(
                id,
                ClassifySlot {
                    started_at: Instant::now(),
                    abort: Some(abort),
                },
            );
        } else {
            guard.pre_auth.insert(
                id,
                PreAuthSlot {
                    started_at: Instant::now(),
                    abort: Some(abort),
                    mutation_only: false,
                },
            );
        }
    }

    async fn reap(&mut self) {
        if self.tasks.join_next().await.is_none() {
            std::future::pending::<()>().await;
        }
    }
}

impl Endpoint {
    pub async fn start(client: Arc<Client>, directory: &Path) -> Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let token = bearer_token();
        let descriptor = directory.join(format!(".cowboy-execution-{}.json", client.epoch));
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&descriptor)?;
        let mut descriptor_value = json!({"schema": 1, "binding": client.binding,
            "endpoint": format!("ws://{address}"), "bearer_token": token});
        if client.binding.managed.is_none() {
            descriptor_value["diagnostics"] = json!({"schema": 1, "path": DIAGNOSTICS_PATH});
        }
        // The Machine's file-buffer settings; absent on older Machines.
        if let Some(policy) = std::env::var_os(crate::config::file_buffers::POLICY_ENV)
            .and_then(|path| path.into_string().ok())
            .filter(|path| Path::new(path).is_absolute())
        {
            descriptor_value["file_buffers"] = json!(policy);
        }
        serde_json::to_writer(&file, &descriptor_value)?;
        file.sync_all()?;
        let mut admission = Admission::new(token, Arc::clone(&client));
        let listener = tokio::spawn(async move {
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else { break; };
                        let unit_generation = admission.client.relays.drain_generation();
                        admission.admit(socket, unit_generation);
                    }
                    () = admission.reap() => {}
                }
            }
        });
        Ok(Self {
            client,
            descriptor: Some(descriptor),
            listener,
        })
    }

    /// The endpoint of an account-sandboxed unit (design R3-11): no loopback
    /// listener and no descriptor file. The returned bridge carries the
    /// descriptor the daemon hands the unit, naming the socket bound into it,
    /// and takes the listening socket each started unit returns; the newest
    /// one is served with the same token, one-connection limit and frame
    /// bounds as the loopback endpoint.
    pub fn start_unix(
        client: Arc<Client>,
        extensions: Option<Value>,
        managed_session_id: Option<&str>,
    ) -> Result<(Self, crate::account_sandbox::ExecutionBridge)> {
        let token = bearer_token();
        let mut descriptor = json!({"schema": 1, "binding": client.binding,
            "endpoint": format!("unix:{}", crate::account_sandbox::UNIT_EXEC_SOCKET),
            "bearer_token": token});
        if client.binding.managed.is_none() {
            descriptor["diagnostics"] = json!({"schema": 1, "path": DIAGNOSTICS_PATH});
        } else if let Some(native_id) = managed_session_id.filter(|id| !id.trim().is_empty()) {
            descriptor["managed_session_id"] = json!(native_id);
        }
        // What the session's extensions offer the unit; never a relay
        // address, token or host path (`extensions::unit_summary`).
        if let Some(extensions) = extensions {
            descriptor["extensions"] = extensions;
        }
        let descriptor = serde_json::to_vec(&descriptor)?;
        let relays = Arc::clone(&client.relays);
        let weak = Arc::downgrade(&client);
        relays.set_recorder(Box::new(move |effects| {
            if let Some(client) = weak.upgrade() {
                client.unread(effects);
            }
        }));
        let (listeners, mut received) = mpsc::unbounded_channel::<OwnedFd>();
        let mut admission = Admission::new(token, Arc::clone(&client));
        let listener = tokio::spawn(async move {
            let mut listener: Option<(UnixListener, u64)> = None;
            let mut open = true;
            loop {
                tokio::select! {
                    next = received.recv(), if open => match next {
                        Some(fd) => match unix_listener(fd) {
                            Ok(next) => {
                                let unit_generation = admission.client.relays.drain_generation();
                                listener = Some((next, unit_generation));
                            }
                            Err(error) => tracing::warn!(%error, "unusable execution socket from the account sandbox"),
                        },
                        None => open = false,
                    },
                    accepted = accept_unix(listener.as_ref()) => {
                        match accepted {
                            Ok((socket, unit_generation)) => admission.admit(socket, unit_generation),
                            Err(error) => {
                                tracing::warn!(%error, "execution socket accept failed");
                                tokio::time::sleep(Duration::from_millis(100)).await;
                            }
                        }
                    }
                    () = admission.reap() => {}
                }
            }
        });
        Ok((
            Self {
                client,
                descriptor: None,
                listener,
            },
            crate::account_sandbox::ExecutionBridge {
                descriptor,
                listeners,
                relays,
            },
        ))
    }

    /// The loopback descriptor file; `None` for a sandboxed unit's endpoint.
    #[must_use]
    pub fn descriptor(&self) -> Option<&Path> {
        self.descriptor.as_deref()
    }
}

fn unix_listener(fd: OwnedFd) -> Result<UnixListener> {
    let listener = std::os::unix::net::UnixListener::from(fd);
    listener.set_nonblocking(true)?;
    Ok(UnixListener::from_std(listener)?)
}

async fn accept_unix(
    listener: Option<&(UnixListener, u64)>,
) -> std::io::Result<(tokio::net::UnixStream, u64)> {
    match listener {
        Some((listener, unit_generation)) => listener
            .accept()
            .await
            .map(|(socket, _)| (socket, *unit_generation)),
        None => std::future::pending().await,
    }
}

enum UpgradedConnection<S> {
    Mutation {
        websocket: tokio_tungstenite::WebSocketStream<PrefixedHandshakeStream<S>>,
        initialized: Arc<std::sync::atomic::AtomicBool>,
        _guard: MutationSocketGuard,
    },
    Diagnostics {
        websocket: tokio_tungstenite::WebSocketStream<PrefixedHandshakeStream<S>>,
        initialized: Arc<std::sync::atomic::AtomicBool>,
        _permit: tokio::sync::OwnedSemaphorePermit,
    },
}

enum SlotKind {
    Mutation(MutationSocketGuard),
    Diagnostics(tokio::sync::OwnedSemaphorePermit),
}

#[allow(clippy::too_many_arguments)]
async fn classify_then_promote_mutation_reserve<S>(
    mut socket: S,
    token: &str,
    client: Arc<Client>,
    unit_generation: u64,
    id: u64,
    admission_state: Arc<Mutex<AdmissionState>>,
    mutation_slot: Arc<Semaphore>,
    diagnostics_slot: Arc<Semaphore>,
) -> Result<UpgradedConnection<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt as _;
    let peeked =
        tokio::time::timeout(PRE_AUTH_CLASSIFY_TIMEOUT, peek_request_line(&mut socket)).await??;
    match peeked.1 {
        RequestLineRoute::Diagnostics => {
            let _ = socket
                .write_all(
                    b"HTTP/1.1 503 Busy\r\nContent-Length: 4\r\nConnection: close\r\n\r\nBusy",
                )
                .await;
            bail!("diagnostics rejected before mutation reserve admission");
        }
        RequestLineRoute::Invalid => {
            let _ = socket
                .write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 11\r\nConnection: close\r\n\r\nBad Request",
                )
                .await;
            bail!("malformed request line rejected before mutation reserve admission");
        }
        RequestLineRoute::Mutation => {}
    }
    // Promote classified mutation candidate into the 4th reserved pre-auth ticket.
    {
        let mut guard = admission_state.lock();
        let Some(slot) = guard.classifying.remove(&id) else {
            bail!("classify slot evicted");
        };
        if mutation_slot.available_permits() == 0 {
            bail!("mutation slot no longer vacant");
        }
        if guard.pre_auth.len() >= MAX_PRE_AUTH_SOCKETS {
            if let Some((&evict_id, _)) = guard
                .pre_auth
                .iter()
                .filter(|(_, s)| s.started_at.elapsed() >= PRE_AUTH_STALL_EVICT_AFTER)
                .min_by_key(|(_, s)| s.started_at)
            {
                if let Some(evicted) = guard.pre_auth.remove(&evict_id)
                    && let Some(abort) = evicted.abort
                {
                    abort.abort();
                }
            } else {
                bail!("pre-auth pool full");
            }
        }
        guard.pre_auth.insert(
            id,
            PreAuthSlot {
                started_at: Instant::now(),
                abort: slot.abort,
                mutation_only: true,
            },
        );
    }
    upgrade_and_classify(
        socket,
        Some(peeked),
        token,
        client,
        unit_generation,
        id,
        admission_state,
        mutation_slot,
        diagnostics_slot,
    )
    .await
}

#[expect(
    clippy::result_large_err,
    reason = "Tungstenite fixes the callback HTTP error response type"
)]
#[allow(clippy::too_many_arguments)]
async fn upgrade_and_classify<S>(
    mut socket: S,
    pre_peeked: Option<(Vec<u8>, RequestLineRoute)>,
    token: &str,
    client: Arc<Client>,
    unit_generation: u64,
    pre_auth_id: u64,
    admission_state: Arc<Mutex<AdmissionState>>,
    mutation_slot: Arc<Semaphore>,
    diagnostics_slot: Arc<Semaphore>,
) -> Result<UpgradedConnection<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt as _;
    let deadline = tokio::time::Instant::now() + PRE_AUTH_UPGRADE_TIMEOUT;
    let (prefix, route) = match pre_peeked {
        Some(peeked) => peeked,
        None => tokio::time::timeout_at(deadline, peek_request_line(&mut socket)).await??,
    };
    if route == RequestLineRoute::Invalid {
        let _ = socket
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 11\r\nConnection: close\r\n\r\nBad Request",
            )
            .await;
        bail!("malformed execution HTTP request line");
    }
    let peeked_diagnostics = route == RequestLineRoute::Diagnostics;
    let remaining_header = MAX_PRE_AUTH_HEADER_BYTES.saturating_sub(prefix.len());
    let upgraded = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let initialized = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let prefixed = PrefixedHandshakeStream {
        inner: socket,
        prefix,
        prefix_pos: 0,
        pre_auth_remaining: remaining_header,
        init_remaining: MAX_INIT_WIRE_BYTES,
        upgraded: Arc::clone(&upgraded),
        initialized: Arc::clone(&initialized),
    };
    let max_frame = if peeked_diagnostics {
        DIAGNOSTICS_MAX_FRAME_BYTES
    } else {
        wire::MAX_FRAME_BYTES
    };
    let expected = format!("Bearer {token}");
    let acquired: Arc<Mutex<Option<SlotKind>>> = Arc::new(Mutex::new(None));
    let acquired_cb = Arc::clone(&acquired);
    let upgraded_cb = Arc::clone(&upgraded);
    let client_cb = Arc::clone(&client);
    let websocket = tokio_tungstenite::accept_hdr_async_with_config(
        prefixed,
        move |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
            let auth_ok = request
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| {
                    post_edit::constant_time_eq(value.as_bytes(), expected.as_bytes())
                });
            if !auth_ok || request.headers().contains_key("origin") {
                return Err(tokio_tungstenite::tungstenite::http::Response::builder()
                    .status(401)
                    .body(Some("Unauthorized".to_owned()))
                    .expect("constant response"));
            }
            let path = request.uri().path();
            let mut adm = admission_state.lock();
            let Some(slot) = adm.pre_auth.remove(&pre_auth_id) else {
                return Err(tokio_tungstenite::tungstenite::http::Response::builder()
                    .status(503)
                    .body(Some("Evicted".to_owned()))
                    .expect("constant response"));
            };
            if path == DIAGNOSTICS_PATH {
                if slot.mutation_only {
                    return Err(tokio_tungstenite::tungstenite::http::Response::builder()
                        .status(503)
                        .body(Some("Reserved".to_owned()))
                        .expect("constant response"));
                }
                let drain = client_cb.relays.drain_state();
                if !peeked_diagnostics || client_cb.binding.managed.is_some() {
                    return Err(tokio_tungstenite::tungstenite::http::Response::builder()
                        .status(403)
                        .body(Some("Forbidden".to_owned()))
                        .expect("constant response"));
                }
                if drain.draining || drain.generation != unit_generation {
                    return Err(tokio_tungstenite::tungstenite::http::Response::builder()
                        .status(503)
                        .body(Some("Draining".to_owned()))
                        .expect("constant response"));
                }
                let Ok(permit) = Arc::clone(&diagnostics_slot).try_acquire_owned() else {
                    return Err(tokio_tungstenite::tungstenite::http::Response::builder()
                        .status(503)
                        .body(Some("Busy".to_owned()))
                        .expect("constant response"));
                };
                *acquired_cb.lock() = Some(SlotKind::Diagnostics(permit));
            } else if !peeked_diagnostics {
                let Ok(permit) = Arc::clone(&mutation_slot).try_acquire_owned() else {
                    return Err(tokio_tungstenite::tungstenite::http::Response::builder()
                        .status(503)
                        .body(Some("Busy".to_owned()))
                        .expect("constant response"));
                };
                *acquired_cb.lock() = Some(SlotKind::Mutation(MutationSocketGuard {
                    relays: Arc::clone(&client_cb.relays),
                    _permit: permit,
                }));
            } else {
                return Err(tokio_tungstenite::tungstenite::http::Response::builder()
                    .status(400)
                    .body(Some("Bad Request".to_owned()))
                    .expect("constant response"));
            }
            upgraded_cb.store(true, std::sync::atomic::Ordering::Release);
            Ok(response)
        },
        Some(
            WebSocketConfig::default()
                .max_message_size(Some(max_frame))
                .max_frame_size(Some(max_frame)),
        ),
    );
    let websocket = tokio::time::timeout_at(deadline, websocket).await??;
    match acquired.lock().take() {
        Some(SlotKind::Mutation(guard)) => Ok(UpgradedConnection::Mutation {
            websocket,
            initialized,
            _guard: guard,
        }),
        Some(SlotKind::Diagnostics(permit)) => Ok(UpgradedConnection::Diagnostics {
            websocket,
            initialized,
            _permit: permit,
        }),
        None => bail!("execution websocket upgraded without a slot"),
    }
}

use relay::{BrokerConnectionGuard, MutationInflightIds, next_mutation_connection_id};

async fn serve_mutation<S>(
    mut socket: tokio_tungstenite::WebSocketStream<S>,
    initialized: Arc<std::sync::atomic::AtomicBool>,
    client: Arc<Client>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let first = tokio::time::timeout(INIT_FRAME_TIMEOUT, socket.next())
        .await?
        .context("execution initialization missing")??;
    let first_bytes = first.into_data();
    ensure!(
        first_bytes.len() <= MAX_INIT_FRAME_BYTES,
        "execution initialization frame exceeded 4 KiB limit"
    );
    let first: Value = serde_json::from_slice(&first_bytes)?;
    ensure!(
        first["method"] == "initialize" && valid_id(&first["id"]),
        "execution initialization required"
    );
    initialized.store(true, std::sync::atomic::Ordering::Release);
    let Response::Ready {
        scope,
        initialization,
        event_cursor,
    } = client.call(Command::Describe).await?
    else {
        bail!("execution environment unavailable");
    };
    ensure!(
        scope == Scope::from_binding(&client.binding),
        "execution identity mismatch"
    );
    let resume = first.pointer("/params/resumeSessionId");
    ensure!(
        resume
            .is_none_or(|value| value.is_null() || Some(value) == initialization.get("sessionId")),
        "execution session identity mismatch"
    );
    let init_frame = json!({"id": first["id"], "result": initialization}).to_string();
    send_bounded(&mut socket, Message::Text(init_frame.into())).await?;

    let (mut writer, mut reader) = socket.split();
    let connection_id = next_mutation_connection_id();
    let mut relays = relay::Relays::new(Arc::clone(&client), connection_id);
    // Declared AFTER `relays` so `_broker_conn` drops FIRST when `serve_mutation`
    // exits, marking `connection_id` closed and settling registered `PendingAck`s
    // before `Relays::drop` drains still-in-flight tasks.
    let _broker_conn = BrokerConnectionGuard {
        client: Arc::clone(&client),
        connection_id,
    };
    let mut inflight_ids = MutationInflightIds::new();
    let mut cursor = *client.event_cursor.lock().get_or_insert(event_cursor);
    tracing::info!(event_name = "cowboy.execution.connected", session = %client.session, environment_id = %client.binding.environment.id, cursor, "execution endpoint connected");
    let mut events = Box::pin(client.call(Command::Events {
        after: cursor,
        wait_ms: wire::MAX_WAIT_MS,
    }));
    loop {
        tokio::select! {
            message = reader.next() => {
                let Some(message) = message else { break; };
                match message? {
                    Message::Close(_) => break,
                    Message::Ping(data) => { send_bounded(&mut writer, Message::Pong(data)).await?; }
                    Message::Pong(_) => {}
                    Message::Text(text) => {
                        let message: Value = serde_json::from_str(&text)?;
                        if message["method"] == "initialized" && message.get("id").is_none() { continue; }
                        let id = message.get("id").context("execution request id missing")?.clone();
                        let method = message["method"].as_str().context("execution method missing")?.to_owned();
                        if !inflight_ids.try_admit(&id, &method)? {
                            let reason = if MutationInflightIds::is_ack(&method) {
                                "ack_capacity"
                            } else if MutationInflightIds::is_control(&method) {
                                "control_capacity"
                            } else {
                                "request_capacity"
                            };
                            tracing::warn!(event_name = "cowboy.execution.request_rejected", session = %client.session, reason, pending = inflight_ids.pending_for(&method), "execution request not admitted");
                            send_bounded(&mut writer, refusal(&id, "Execution request capacity reached; this request was not admitted")).await?;
                            continue;
                        }
                        let params = native_params(&message);
                        // A unit being stopped sends only cleanup (L1c).
                        let Some(in_flight) = client.relays.try_enter(&method) else {
                            inflight_ids.remove(&id);
                            send_bounded(&mut writer, refusal(&id, "The session is ending; this request was not admitted")).await?;
                            continue;
                        };
                        relays.spawn(id, method, params, in_flight);
                    }
                    _ => bail!("unsupported execution frame"),
                }
            }
            done = relays.next(), if relays.pending() > 0 => {
                inflight_ids.remove(&done.id);
                if let Err(error) = send_bounded(&mut writer, Message::Text(done.frame()?.into())).await {
                    relays.undelivered(&done);
                    return Err(error);
                }
                relays.delivered(&done);
            }
            result = &mut events => {
                match result? {
                    Response::Events { events: batch, through } => {
                        for event in batch {
                            client.observe_event(&event.message);
                            // The worker's own servers' output never reaches the unit.
                            if ledger::worker_event(&event.message) {
                                continue;
                            }
                            send_bounded(&mut writer, Message::Text(event.message.to_string().into())).await?;
                        }
                        cursor = through;
                        *client.event_cursor.lock() = Some(cursor);
                    }
                    Response::Refused { reason: wire::Refusal::CursorExpired } => {
                        // Legacy keepers evict output. Native Codex resumes
                        // the same executor session and recovers its processes
                        // with process/read. Reusing the expired cursor poisons
                        // every reconnect, including unrelated future calls.
                        *client.event_cursor.lock() = None;
                        tracing::warn!(event_name = "cowboy.execution.cursor_expired", session = %client.session, environment_id = %client.binding.environment.id, cursor, reason = "cursor_expired", "resuming native execution after an event gap");
                        let _ = send_bounded(&mut writer, Message::Close(Some(CloseFrame {
                            code: CloseCode::Restart,
                            reason: "Execution event history expired; resume the original session".into(),
                        }))).await;
                        return Ok(());
                    }
                    _ => bail!("execution events unavailable"),
                }
                events = Box::pin(client.call(Command::Events { after: cursor, wait_ms: wire::MAX_WAIT_MS }));
            }
        }
    }
    Ok(())
}

async fn serve_diagnostics<S>(
    socket: tokio_tungstenite::WebSocketStream<S>,
    initialized: Arc<std::sync::atomic::AtomicBool>,
    client: Arc<Client>,
    unit_generation: u64,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut drain = client.relays.subscribe_drain();
    let is_fenced = |state: &crate::account_sandbox::DrainState| {
        state.draining || state.generation != unit_generation
    };
    if client.binding.managed.is_some() || is_fenced(&drain.borrow()) {
        return Ok(());
    }
    tokio::select! {
        biased;
        _ = drain.wait_for(is_fenced) => Ok(()),
        res = serve_diagnostics_inner(socket, initialized, client, unit_generation) => res,
    }
}

async fn serve_diagnostics_inner<S>(
    mut socket: tokio_tungstenite::WebSocketStream<S>,
    initialized: Arc<std::sync::atomic::AtomicBool>,
    client: Arc<Client>,
    unit_generation: u64,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let first = tokio::time::timeout(INIT_FRAME_TIMEOUT, socket.next())
        .await?
        .context("diagnostics initialization missing")??;
    let first_bytes = first.into_data();
    ensure!(
        first_bytes.len() <= MAX_INIT_FRAME_BYTES,
        "diagnostics initialization frame exceeded 4 KiB limit"
    );
    let first: Value = serde_json::from_slice(&first_bytes)?;
    ensure!(
        first["method"] == "initialize" && valid_id(&first["id"]),
        "diagnostics initialization required"
    );
    initialized.store(true, std::sync::atomic::Ordering::Release);
    let init_frame = json!({"id": first["id"], "result": {"schema": 1}}).to_string();
    send_bounded(&mut socket, Message::Text(init_frame.into())).await?;

    while let Some(message) = socket.next().await {
        match message? {
            Message::Close(_) => break,
            Message::Ping(data) => {
                send_bounded(&mut socket, Message::Pong(data)).await?;
            }
            Message::Pong(_) => {}
            Message::Text(text) => {
                let message: Value = serde_json::from_str(&text)?;
                if message["method"] == "initialized" && message.get("id").is_none() {
                    continue;
                }
                let id = message
                    .get("id")
                    .context("diagnostics request id missing")?
                    .clone();
                ensure!(valid_id(&id), "invalid diagnostics request id");
                let method = message["method"]
                    .as_str()
                    .context("diagnostics method missing")?;
                let reply = if method == "diagnostics/post_edit" {
                    let params = native_params(&message);
                    let result = tokio::select! {
                        biased;
                        peer = socket.next() => {
                            let _ = peer;
                            return Ok(());
                        }
                        result = client.inspect_post_edit(unit_generation, &params) => result,
                    };
                    json!({"id": id, "result": result})
                } else {
                    json!({"id": id, "error": {"code": -32601, "message": "method not found"}})
                };
                send_bounded(&mut socket, Message::Text(reply.to_string().into())).await?;
            }
            _ => bail!("unsupported diagnostics frame"),
        }
    }
    Ok(())
}

/// Bound writes to a backpressured execution endpoint so a wedged peer cannot
/// hold turn completion guards indefinitely (Finding 14).
const ENDPOINT_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

async fn send_bounded<S>(writer: &mut S, message: Message) -> Result<()>
where
    S: futures::SinkExt<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    tokio::time::timeout(ENDPOINT_WRITE_TIMEOUT, writer.send(message))
        .await
        .map_err(|_| anyhow::anyhow!("execution endpoint write timed out"))?
        .map_err(Into::into)
}

/// A refusal of one native request that admitted no effect.
fn refusal(id: &Value, message: &str) -> Message {
    Message::Text(
        json!({"id": id, "error": {"code": -32000, "message": message}})
            .to_string()
            .into(),
    )
}

/// A native request's params as the wire contract carries them: native sends
/// parameterless requests such as environment/info with `"params": null`.
fn native_params(message: &Value) -> Value {
    match message.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(params) => params.clone(),
    }
}

/// A fresh keeper operation id; the suffix asks the keeper for the backend's
/// own duration.
fn new_operation_id() -> String {
    format!(
        "{:032x}{}",
        rand::random::<u128>(),
        crate::execution_protocol::TIMED_SUFFIX
    )
}

fn valid_id(value: &Value) -> bool {
    value.is_i64()
        || value
            .as_str()
            .is_some_and(|value| !value.is_empty() && value.len() <= 128)
}

#[cfg(test)]
mod tests;
