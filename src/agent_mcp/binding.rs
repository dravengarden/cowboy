//! Native startup is static; tool calls lazily open an authorized remote session.
//! A descriptor is metadata, never a substitute for the live parent gateway.
use std::collections::BTreeSet;
use std::future::Future;
use std::io::Read as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite, BufReader};
use tokio::sync::{Mutex, mpsc, watch};

use crate::managed_calls::protocol::Action;
use crate::mcp_binding::{NativeDescriptor, RelayAction, RelayRequest};

mod input;

const LIMIT: usize = 256 * 1024;
type Gateway =
    Arc<dyn Fn(RelayAction) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>> + Send + Sync>;

fn descriptor(path: &Path) -> Result<NativeDescriptor> {
    ensure!(path.is_absolute(), "MCP descriptor path must be absolute");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.mode() & 0o077 == 0
            && meta.uid() == rustix::process::geteuid().as_raw()
            && meta.len() <= 65536,
        "MCP descriptor must be a bounded private file"
    );
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65536, "MCP descriptor grew past limit");
    let descriptor: NativeDescriptor = serde_json::from_slice(&bytes)?;
    descriptor.validate()?;
    Ok(descriptor)
}

pub(super) async fn serve(path: &Path) -> Result<()> {
    let descriptor = descriptor(path)?;
    let binding = descriptor.binding.name.clone();
    let epoch = descriptor.epoch.clone();
    let gateway: Gateway = Arc::new(move |action| {
        let request = RelayRequest {
            binding: binding.clone(),
            epoch: epoch.clone(),
            action,
        };
        Box::pin(async move {
            request.validate()?;
            let wiring = crate::managed_calls::cli::wiring()
                .map_err(|_| anyhow::anyhow!("MCP gateway unavailable"))?;
            let response = crate::managed_calls::cli::exchange(
                wiring,
                Action::Mcp {
                    request: Box::new(request),
                },
                30_000,
            )
            .await;
            ensure!(
                response["schema"] == 1 && response["state"] == "ok",
                "MCP binding unavailable"
            );
            response
                .get("result")
                .cloned()
                .context("MCP gateway result missing")
        })
    });
    serve_io(
        input::stdin()?,
        tokio::io::stdout(),
        descriptor,
        gateway,
        true,
    )
    .await
}

#[derive(Default)]
struct InputState {
    ended: std::sync::atomic::AtomicBool,
    cancelled: parking_lot::Mutex<BTreeSet<String>>,
}

async fn read_input(
    input: impl AsyncRead + Unpin,
    messages: mpsc::Sender<Value>,
    state: Arc<InputState>,
    changed: watch::Sender<()>,
) {
    let mut input = BufReader::new(input);
    loop {
        let mut bytes = Vec::new();
        let Ok(count) = (&mut input)
            .take(LIMIT as u64 + 1)
            .read_until(b'\n', &mut bytes)
            .await
        else {
            break;
        };
        if count == 0 || count > LIMIT || !bytes.ends_with(b"\n") {
            break;
        }
        let Ok(message) = serde_json::from_slice::<Value>(&bytes) else {
            break;
        };
        if message["method"] == "notifications/cancelled" {
            let id = &message["params"]["requestId"];
            if valid_id(id) {
                let mut cancelled = state.cancelled.lock();
                if cancelled.len() >= 8 {
                    break;
                }
                cancelled.insert(id.to_string());
                changed.send_replace(());
            }
        } else if messages.try_send(message).is_err() {
            break;
        }
    }
    state
        .ended
        .store(true, std::sync::atomic::Ordering::Release);
    changed.send_replace(());
}

fn valid_id(id: &Value) -> bool {
    id.as_str().is_some_and(|id| id.len() <= 128) || id.is_i64() || id.is_u64()
}

struct Session {
    descriptor: NativeDescriptor,
    gateway: Gateway,
    remote: Option<String>,
    protocol: Option<String>,
}

impl Session {
    async fn request(&self, handle: &str, message: Value) -> Result<Value> {
        let value = (self.gateway)(RelayAction::Request {
            handle: handle.into(),
            message,
        })
        .await?;
        value
            .get("response")
            .cloned()
            .context("MCP response missing")
    }

    async fn call(&mut self, message: Value) -> Result<Value> {
        if self.remote.is_none() {
            let opened = (self.gateway)(RelayAction::Open).await?;
            let handle = opened["handle"]
                .as_str()
                .context("MCP handle missing")?
                .to_owned();
            RelayRequest {
                binding: self.descriptor.binding.name.clone(),
                epoch: self.descriptor.epoch.clone(),
                action: RelayAction::Close {
                    handle: handle.clone(),
                },
            }
            .validate()?;
            self.remote = Some(handle.clone());
            let initialized = self.request(&handle, json!({"jsonrpc":"2.0","id":"binding-initialize","method":"initialize","params":{"protocolVersion":self.protocol,"capabilities":{},"clientInfo":{"name":"cowboy-binding","version":"1"}}})).await?;
            ensure!(
                initialized.get("error").is_none()
                    && initialized["result"]["protocolVersion"]
                        .as_str()
                        .is_some_and(|version| super::PROTOCOL_VERSIONS.contains(&version)),
                "MCP initialization failed"
            );
            self.request(
                &handle,
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            )
            .await?;
        }
        let reply = self
            .request(self.remote.as_deref().unwrap(), message.clone())
            .await?;
        ensure!(
            reply["jsonrpc"] == "2.0" && reply["id"] == message["id"],
            "MCP response identity changed"
        );
        Ok(reply)
    }

    async fn close(&mut self) {
        if let Some(handle) = self.remote.take() {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                (self.gateway)(RelayAction::Close { handle }),
            )
            .await;
        }
    }
}

async fn serve_io(
    input: impl AsyncRead + Unpin + Send + 'static,
    output: impl AsyncWrite + Unpin,
    descriptor: NativeDescriptor,
    gateway: Gateway,
    signals: bool,
) -> Result<()> {
    descriptor.validate()?;
    let state = Arc::new(InputState::default());
    let (notify, mut changed) = watch::channel(());
    let mut stopped = notify.subscribe();
    let (sender, mut receiver) = mpsc::channel(8);
    let signal = if signals {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let mut hup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?;
        let state = Arc::clone(&state);
        let notify = notify.clone();
        Some(tokio::spawn(async move {
            tokio::select! { _ = term.recv() => {}, _ = interrupt.recv() => {}, _ = hup.recv() => {} }
            state
                .ended
                .store(true, std::sync::atomic::Ordering::Release);
            notify.send_replace(());
        }))
    } else {
        None
    };
    let reader = tokio::spawn(read_input(input, sender, Arc::clone(&state), notify));
    let output = Mutex::new(output);
    let mut session = Session {
        descriptor,
        gateway,
        remote: None,
        protocol: None,
    };
    let result = tokio::select! {
        biased;
        () = ended(&state, &mut stopped) => Ok(()),
        result = run(&mut session, &output, &state, &mut changed, &mut receiver) => result,
    };
    if let Some(signal) = signal {
        signal.abort();
    }
    reader.abort();
    session.close().await;
    result
}

async fn ended(state: &InputState, changed: &mut watch::Receiver<()>) {
    while !state.ended.load(std::sync::atomic::Ordering::Acquire) {
        if changed.changed().await.is_err() {
            return;
        }
    }
}

async fn run(
    session: &mut Session,
    output: &Mutex<impl AsyncWrite + Unpin>,
    state: &InputState,
    changed: &mut watch::Receiver<()>,
    receiver: &mut mpsc::Receiver<Value>,
) -> Result<()> {
    loop {
        let message = tokio::select! { biased; _ = changed.changed() => {
            if state.ended.load(std::sync::atomic::Ordering::Acquire) { return Ok(()); }
            continue;
        }, message = receiver.recv() => match message { Some(message) => message, None => return Ok(()) } };
        let id = message.get("id").cloned();
        if message["jsonrpc"] != "2.0" || id.as_ref().is_some_and(|id| !valid_id(id)) {
            super::write_reply(
                output,
                &super::rpc_error(Value::Null, -32600, "invalid request"),
            )
            .await?;
            continue;
        }
        let method = message["method"].as_str().unwrap_or("");
        if method == "notifications/initialized" && id.is_none() {
            continue;
        }
        let Some(id) = id else {
            continue;
        };
        if state.cancelled.lock().remove(&id.to_string()) {
            continue;
        }
        let reply = match method {
            "initialize" if session.protocol.is_none() => {
                let requested = message["params"]["protocolVersion"].as_str().unwrap_or("");
                let version = if super::PROTOCOL_VERSIONS.contains(&requested) {
                    requested
                } else {
                    super::PROTOCOL_VERSIONS[0]
                };
                session.protocol = Some(version.into());
                json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":version,"capabilities":{"tools":{}},"serverInfo":{"name":session.descriptor.binding.server.plugin_id,"version":session.descriptor.server_version}}})
            }
            "tools/list" if session.protocol.is_some() => {
                json!({"jsonrpc":"2.0","id":id,"result":{"tools":session.descriptor.tools}})
            }
            "ping" => json!({"jsonrpc":"2.0","id":id,"result":{}}),
            "tools/call"
                if session.protocol.is_some()
                    && message["params"]["name"].as_str().is_some_and(|name| {
                        session
                            .descriptor
                            .binding
                            .tools
                            .iter()
                            .any(|allowed| allowed == name)
                    }) =>
            {
                let outcome = {
                    let call = session.call(message.clone());
                    tokio::pin!(call);
                    loop {
                        if state.ended.load(std::sync::atomic::Ordering::Acquire)
                            || state.cancelled.lock().remove(&id.to_string())
                        {
                            return Ok(());
                        }
                        tokio::select! { biased; _ = changed.changed() => {}, result = &mut call => break result }
                    }
                };
                match outcome {
                    Ok(reply) => reply,
                    Err(_) => {
                        super::write_reply(
                            output,
                            &super::rpc_error(id, -32000, "MCP binding unavailable"),
                        )
                        .await?;
                        return Ok(());
                    }
                }
            }
            _ => super::rpc_error(id, -32601, "method or tool not admitted"),
        };
        super::write_reply(output, &reply).await?;
    }
}

#[cfg(test)]
mod tests;
