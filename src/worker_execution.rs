//! Worker-owned endpoint for the execution protocol. The native client sees
//! one authenticated loopback socket; placement and retries are not model tools.

use std::collections::{BTreeMap, HashSet};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use futures::{SinkExt as _, StreamExt as _};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
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

pub(crate) const DESCRIPTOR_ENV: &str = "COWBOY_EXECUTION_DESCRIPTOR";
const MAX_PENDING: usize = 24;
const MAX_PENDING_BYTES: usize = 16 * 1024 * 1024;
// A validated binding is at most 16 KiB before JSON escaping. Bound the
// complete control envelope too, and reserve its worst-case aggregate so
// admitted bulk inputs cannot starve polling or result observation.
const MAX_CONTROL_BYTES: usize = 64 * 1024;
const CONTROL_RESERVE_BYTES: usize = MAX_PENDING * MAX_CONTROL_BYTES;
const RETRY: Duration = Duration::from_secs(30);

struct Pending {
    request: RuntimeRequest,
    sender: oneshot::Sender<MachineResponse>,
    sent: Option<Instant>,
    bytes: usize,
}

pub(crate) struct Client {
    session: String,
    epoch: String,
    binding: BindingV1,
    pending: Mutex<BTreeMap<String, Pending>>,
    notify: mpsc::UnboundedSender<()>,
    event_cursor: Mutex<Option<u64>>,
}

impl Client {
    pub(crate) fn new(
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
        })
    }

    pub(crate) fn frames(&self, reconnect: bool) -> Vec<Frame> {
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

    pub(crate) fn complete(&self, reply: RuntimeReply) {
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
            let _ = pending.sender.send(reply.response);
        }
    }

    async fn call(&self, command: Command) -> Result<Response> {
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
                    bytes,
                },
            );
        }
        let _ = self.notify.send(());
        let response = tokio::time::timeout(Duration::from_secs(180), receiver).await;
        self.pending.lock().remove(&id);
        match response {
            Ok(Ok(MachineResponse::Call { response })) => {
                if let Some(operation) = operation {
                    let failed = matches!(
                        &response,
                        Response::Refused { .. }
                            | Response::Operation {
                                outcome: Outcome::Unknown
                                    | Outcome::Missing
                                    | Outcome::ResultExpired
                            }
                    ) || matches!(&response, Response::Operation { outcome: Outcome::Completed { reply } } if reply.get("error").is_some());
                    if failed {
                        tracing::warn!(event_name = "cowboy.execution.call_failed", session = %self.session, environment_id = %self.binding.environment.id, %operation, reason = "native_or_keeper_refusal", duration_ms = started.elapsed().as_secs_f64() * 1000.0, "execution call failed");
                    } else {
                        tracing::info!(event_name = "cowboy.execution.call_observed", session = %self.session, environment_id = %self.binding.environment.id, %operation, duration_ms = started.elapsed().as_secs_f64() * 1000.0, "execution invocation observed");
                    }
                }
                Ok(response)
            }
            _ => {
                tracing::warn!(event_name = "cowboy.execution.response_timeout", session = %self.session, environment_id = %self.binding.environment.id, reason = "response_unavailable", duration_ms = started.elapsed().as_secs_f64() * 1000.0, "execution response unavailable");
                bail!(
                    "execution response unavailable; original operation was not replayed under a new identity"
                )
            }
        }
    }

    async fn invoke(&self, method: String, params: Value) -> Result<Value> {
        let invocation = Invocation {
            operation_id: format!("{:032x}", rand::random::<u128>()),
            method,
            params,
        };
        ensure!(invocation.validate(), "invalid execution request");
        let operation_id = invocation.operation_id.clone();
        let mut command = Command::Invoke {
            invocation,
            wait_ms: wire::MAX_WAIT_MS,
        };
        loop {
            match self.call(command).await? {
                Response::Operation {
                    outcome: Outcome::Completed { reply },
                } => return Ok(reply),
                Response::Operation {
                    outcome: Outcome::Pending,
                } => {
                    command = Command::Observe {
                        operation_id: operation_id.clone(),
                        wait_ms: wire::MAX_WAIT_MS,
                    };
                }
                _ => bail!(
                    "execution operation refused or its final result is unknown; do not replay"
                ),
            }
        }
    }
}

pub(crate) struct Endpoint {
    pub(crate) client: Arc<Client>,
    descriptor: PathBuf,
    listener: tokio::task::JoinHandle<()>,
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.listener.abort();
        let _ = std::fs::remove_file(&self.descriptor);
    }
}

impl Endpoint {
    pub(crate) async fn start(client: Arc<Client>, directory: &Path) -> Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let token = format!(
            "{:032x}{:032x}",
            rand::random::<u128>(),
            rand::random::<u128>()
        );
        let descriptor = directory.join(format!(".cowboy-execution-{}.json", client.epoch));
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&descriptor)?;
        serde_json::to_writer(
            &file,
            &json!({"schema": 1, "binding": client.binding,
            "endpoint": format!("ws://{address}"), "bearer_token": token}),
        )?;
        file.sync_all()?;
        let endpoint_client = Arc::clone(&client);
        let listener = tokio::spawn(async move {
            let connections = Arc::new(Semaphore::new(1));
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else { break; };
                        let Ok(permit) = Arc::clone(&connections).try_acquire_owned() else { continue; };
                        let client = Arc::clone(&endpoint_client);
                        let token = token.clone();
                        tasks.spawn(async move {
                            let _permit = permit;
                            let session = client.session.clone();
                            if serve(socket, &token, client).await.is_err() {
                                // Do not log frames, capabilities or arbitrary
                                // native error text from the private endpoint.
                                tracing::warn!(event_name = "cowboy.execution.disconnected", %session, reason = "endpoint_failed", "execution endpoint disconnected");
                            }
                        });
                    }
                    _ = tasks.join_next(), if !tasks.is_empty() => {}
                }
            }
        });
        Ok(Self {
            client,
            descriptor,
            listener,
        })
    }

    pub(crate) fn descriptor(&self) -> &Path {
        &self.descriptor
    }
}

#[expect(
    clippy::result_large_err,
    reason = "Tungstenite fixes the callback HTTP error response type"
)]
async fn serve(socket: TcpStream, token: &str, client: Arc<Client>) -> Result<()> {
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
    let mut calls = tokio::task::JoinSet::new();
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
                    Message::Ping(data) => { writer.send(Message::Pong(data)).await?; }
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
                            writer.send(Message::Text(json!({"id":id,"error":{"code":-32000,"message":"Execution request capacity reached; this request was not admitted"}}).to_string().into())).await?;
                            continue;
                        }
                        ids.insert(id.to_string());
                        let method = message["method"].as_str().context("execution method missing")?.to_owned();
                        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
                        let client = Arc::clone(&client);
                        calls.spawn(async move {
                            let reply = client.invoke(method, params).await.unwrap_or_else(|_| json!({"error": {"code": -32000, "message": "Execution unavailable or result unknown; no local fallback and no replay"}}));
                            (id, reply)
                        });
                    }
                    _ => bail!("unsupported execution frame"),
                }
            }
            result = calls.join_next(), if !calls.is_empty() => {
                let (id, mut reply) = result.context("execution task missing")??;
                ids.remove(&id.to_string());
                reply.as_object_mut().context("invalid execution result")?.insert("id".to_owned(), id);
                writer.send(Message::Text(reply.to_string().into())).await?;
            }
            result = &mut events => {
                match result? {
                    Response::Events { events: batch, through } => {
                        for event in batch {
                            writer.send(Message::Text(event.message.to_string().into())).await?;
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
                        writer.send(Message::Close(Some(CloseFrame {
                            code: CloseCode::Restart,
                            reason: "Execution event history expired; resume the original session".into(),
                        }))).await?;
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

fn valid_id(value: &Value) -> bool {
    value.is_i64()
        || value
            .as_str()
            .is_some_and(|value| !value.is_empty() && value.len() <= 128)
}

#[cfg(test)]
mod tests;
