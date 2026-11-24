//! Worker-owned endpoint for the execution protocol. The native client sees
//! one authenticated socket: loopback TCP, or for an account-sandboxed unit
//! the Unix socket the daemon bound into it. Placement and retries are not
//! model tools.

mod extension_broker;
mod ledger;
mod memory_bridge;
mod relay;
pub use extension_broker::{Broker as ExtensionBroker, Plan as ExtensionPlan, ServerPlan};
pub use ledger::{LEDGER_ENV, Ledger, MAX_PROCESSES};
pub use memory_bridge::MemoryBridge;

use std::collections::{BTreeMap, HashSet};
use std::os::fd::OwnedFd;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use futures::{SinkExt as _, StreamExt as _};
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
}

/// Wire calls at least this slow are logged individually.
const SLOW_CALL: Duration = Duration::from_secs(5);

impl Client {
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
        })
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

/// One connection at a time: a second is closed while the first is served.
struct Admission {
    token: String,
    client: Arc<Client>,
    connections: Arc<Semaphore>,
    tasks: tokio::task::JoinSet<()>,
}

impl Admission {
    fn new(token: String, client: Arc<Client>) -> Self {
        Self {
            token,
            client,
            connections: Arc::new(Semaphore::new(1)),
            tasks: tokio::task::JoinSet::new(),
        }
    }

    fn admit<S>(&mut self, socket: S)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let Ok(permit) = Arc::clone(&self.connections).try_acquire_owned() else {
            return;
        };
        let client = Arc::clone(&self.client);
        let token = self.token.clone();
        self.tasks.spawn(async move {
            let _permit = permit;
            let session = client.session.clone();
            let relays = Arc::clone(&client.relays);
            if serve(socket, &token, client).await.is_err() {
                // Do not log frames, capabilities or arbitrary
                // native error text from the private endpoint.
                tracing::warn!(event_name = "cowboy.execution.disconnected", %session, reason = "endpoint_failed", "execution endpoint disconnected");
            }
            // Receipts belong to this socket: another one can never
            // acknowledge what this one wrote (design L1c).
            relays.socket_ended();
        });
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
                        admission.admit(socket);
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
    ) -> Result<(Self, crate::account_sandbox::ExecutionBridge)> {
        let token = bearer_token();
        let mut descriptor = json!({"schema": 1, "binding": client.binding,
            "endpoint": format!("unix:{}", crate::account_sandbox::UNIT_EXEC_SOCKET),
            "bearer_token": token});
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
            let mut listener: Option<UnixListener> = None;
            let mut open = true;
            loop {
                tokio::select! {
                    next = received.recv(), if open => match next {
                        Some(fd) => match unix_listener(fd) {
                            Ok(next) => listener = Some(next),
                            Err(error) => tracing::warn!(%error, "unusable execution socket from the account sandbox"),
                        },
                        None => open = false,
                    },
                    accepted = accept_unix(listener.as_ref()) => {
                        match accepted {
                            Ok(socket) => admission.admit(socket),
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

async fn accept_unix(listener: Option<&UnixListener>) -> std::io::Result<tokio::net::UnixStream> {
    match listener {
        Some(listener) => listener.accept().await.map(|(socket, _)| socket),
        None => std::future::pending().await,
    }
}

#[expect(
    clippy::result_large_err,
    reason = "Tungstenite fixes the callback HTTP error response type"
)]
async fn serve<S>(socket: S, token: &str, client: Arc<Client>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let expected = format!("Bearer {token}");
    let websocket = tokio_tungstenite::accept_hdr_async_with_config(
        socket,
        move |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
            if request
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                == Some(expected.as_str())
                && !request.headers().contains_key("origin")
            {
                Ok(response)
            } else {
                Err(tokio_tungstenite::tungstenite::http::Response::builder()
                    .status(401)
                    .body(Some("Unauthorized".to_owned()))
                    .expect("constant response"))
            }
        },
        Some(
            WebSocketConfig::default()
                .max_message_size(Some(wire::MAX_FRAME_BYTES))
                .max_frame_size(Some(wire::MAX_FRAME_BYTES)),
        ),
    );
    let mut socket = tokio::time::timeout(Duration::from_secs(10), websocket).await??;
    let first = tokio::time::timeout(Duration::from_secs(30), socket.next())
        .await?
        .context("execution initialization missing")??;
    let first: Value = serde_json::from_slice(&first.into_data())?;
    ensure!(
        first["method"] == "initialize" && valid_id(&first["id"]),
        "execution initialization required"
    );
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
    socket
        .send(Message::Text(
            json!({"id": first["id"], "result": initialization})
                .to_string()
                .into(),
        ))
        .await?;

    let (mut writer, mut reader) = socket.split();
    let mut relays = relay::Relays::new(Arc::clone(&client));
    let mut ids = HashSet::new();
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
                        ensure!(valid_id(&id) && !ids.contains(&id.to_string()), "invalid or duplicate execution request id");
                        if ids.len() >= 16 {
                            tracing::warn!(event_name = "cowboy.execution.request_rejected", session = %client.session, reason = "request_capacity", pending = ids.len(), "execution request not admitted");
                            // No effect was admitted. Limit this request, not
                            // the transport carrying other commands/results.
                            send_bounded(&mut writer, refusal(&id, "Execution request capacity reached; this request was not admitted")).await?;
                            continue;
                        }
                        ids.insert(id.to_string());
                        let method = message["method"].as_str().context("execution method missing")?.to_owned();
                        let params = native_params(&message);
                        // A unit being stopped sends only cleanup (L1c).
                        let Some(in_flight) = client.relays.try_enter(&method) else {
                            ids.remove(&id.to_string());
                            send_bounded(&mut writer, refusal(&id, "The session is ending; this request was not admitted")).await?;
                            continue;
                        };
                        relays.spawn(id, method, params, in_flight);
                    }
                    _ => bail!("unsupported execution frame"),
                }
            }
            done = relays.next(), if relays.pending() > 0 => {
                ids.remove(&done.id.to_string());
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
