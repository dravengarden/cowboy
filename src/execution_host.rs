//! Detached target-owned execution keeper. The control connection is never the
//! owner of a native executor or its jobs. A lost keeper cannot be relaunched
//! under the same incarnation, and admitted effects cannot be retried as new.

pub mod file_helper;
mod ledger;

use crate::execution_protocol::{
    self as wire, Command, LaunchContract, Outcome, Refusal, Request, Response, Scope,
};
use anyhow::{Context as _, Result, ensure};
use ledger::{Admission, EventError, Ledger};
use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Command as ProcessCommand;
use tokio::sync::{Semaphore, mpsc, watch};

pub struct Args {
    pub contract: PathBuf,
    pub state_dir: PathBuf,
    pub socket: Option<PathBuf>,
}

/// Validate the private launch identity before initializing target-local logs.
///
/// # Errors
/// Invalid or oversized contracts fail without logging their contents.
pub fn log_context(path: &Path) -> Result<crate::logs::Context> {
    let contract: LaunchContract =
        serde_json::from_slice(&crate::logs::storage::private_read(path, 64 * 1024)?)?;
    validate_contract(&contract)?;
    let mut context = crate::logs::Context::new("cowboy-execution-host");
    context.session = contract.session_id;
    context.machine = contract.binding.environment.machine_id;
    context.environment = contract.binding.environment.id;
    context.generation = contract.executor.version;
    Ok(context)
}

struct Host {
    scope: Scope,
    call_context: Option<String>,
    capability_digest: [u8; 32],
    initialization: Value,
    cwd: PathBuf,
    inode: (u64, u64),
    ledger: Mutex<Ledger>,
    writer: mpsc::Sender<Value>,
    stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    stopped: watch::Receiver<bool>,
    close: tokio::sync::Notify,
}

async fn read_json<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Result<Value> {
    read_json_bounded(reader, wire::MAX_FRAME_BYTES).await
}

async fn read_json_bounded<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> Result<Value> {
    let mut bytes = Vec::new();
    let count = reader
        .take((limit + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .await?;
    ensure!(
        count > 0 && count <= limit && bytes.last() == Some(&b'\n'),
        "invalid execution frame"
    );
    serde_json::from_slice(&bytes).context("invalid execution JSON")
}

async fn write_json<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    value: &impl serde::Serialize,
) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() < wire::MAX_FRAME_BYTES,
        "execution response exceeds limit"
    );
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

fn validate_contract(contract: &LaunchContract) -> Result<()> {
    contract.binding.validate().map_err(anyhow::Error::msg)?;
    ensure!(
        contract.schema == wire::SCHEMA && wire::valid_operation_id(&contract.session_id),
        "invalid execution launch contract"
    );
    ensure!(
        contract.capability.len() == 64
            && contract.capability.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid execution capability"
    );
    ensure!(
        Path::new(&contract.executor.command).is_absolute()
            && contract.executor.sha256.len() == 64
            && contract
                .executor
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            && contract.binding.environment.executor_digest
                == format!("sha256:{}", contract.executor.sha256),
        "invalid executor identity"
    );
    ensure!(
        !contract.executor.version.is_empty() && contract.executor.version.len() <= 64,
        "invalid executor version"
    );
    ensure!(
        contract.environment.len() <= crate::execution_target_environment::MAX
            && contract.environment.iter().all(|(name, value)| {
                crate::execution_target_environment::allowed(name)
                    && value.len() <= 8192
                    && !value.contains('\0')
            }),
        "invalid target environment"
    );
    ensure!(
        contract.call_context.as_ref().is_none_or(|path| {
            Path::new(path).is_absolute() && path.len() <= 4096 && !path.contains('\0')
        }),
        "invalid call context"
    );
    Ok(())
}

impl Host {
    fn current_workspace(&self) -> bool {
        std::fs::metadata(&self.cwd)
            .is_ok_and(|metadata| (metadata.dev(), metadata.ino()) == self.inode)
    }

    async fn request(&self, request: Request) -> Response {
        let digest: [u8; 32] = Sha256::digest(request.capability.as_bytes()).into();
        if request.schema != wire::SCHEMA
            || request.scope != self.scope
            || digest != self.capability_digest
        {
            return Response::Refused {
                reason: Refusal::Unauthorized,
            };
        }
        let result = match request.command {
            Command::Shutdown => {
                self.ledger.lock().lose();
                if let Some(stop) = self.stop.lock().take() {
                    let _ = stop.send(());
                }
                let mut stopped = self.stopped.clone();
                let finished = tokio::time::timeout(Duration::from_secs(15), async {
                    while !*stopped.borrow_and_update() {
                        stopped
                            .changed()
                            .await
                            .map_err(|_| Refusal::EnvironmentLost)?;
                    }
                    Ok::<_, Refusal>(())
                })
                .await;
                if matches!(finished, Ok(Ok(()))) {
                    Ok(Response::Closed)
                } else {
                    Err(Refusal::EnvironmentLost)
                }
            }
            Command::Describe => {
                if self.ledger.lock().lost {
                    Err(Refusal::EnvironmentLost)
                } else if !self.current_workspace() {
                    Err(Refusal::WorkspaceChanged)
                } else {
                    Ok(Response::Ready {
                        scope: self.scope.clone(),
                        initialization: self.initialization.clone(),
                        event_cursor: self.ledger.lock().event_cursor(),
                    })
                }
            }
            Command::Invoke {
                invocation,
                wait_ms,
            } => {
                if wait_ms > wire::MAX_WAIT_MS {
                    Err(Refusal::InvalidRequest)
                } else if !self.current_workspace() {
                    Err(Refusal::WorkspaceChanged)
                } else {
                    let admission = {
                        // Reserve before admitting so a full local queue proves
                        // non-admission. Once admitted, the writer owns the call.
                        let mut ledger = self.ledger.lock();
                        if ledger.observe(&invocation.operation_id).is_some() || ledger.lost {
                            ledger.admit(&invocation)
                        } else {
                            match self.writer.try_reserve() {
                                Ok(permit) => {
                                    let result = ledger.admit(&invocation);
                                    if let Ok(Admission::New { id, .. }) = &result {
                                        // The ledger retains the admitted params;
                                        // only the native frame carries context.
                                        let params = wire::with_call_context(
                                            &invocation.method,
                                            &invocation.params,
                                            self.call_context.as_deref(),
                                        );
                                        permit.send(json!({"id": id, "method": invocation.method, "params": params}));
                                    }
                                    result
                                }
                                Err(_) => Err(Refusal::Capacity),
                            }
                        }
                    };
                    match admission {
                        Ok(Admission::Existing(receiver) | Admission::New { receiver, .. }) => {
                            Ok(Response::Operation {
                                outcome: wait_outcome(receiver, wait_ms).await,
                            })
                        }
                        Err(reason) => Err(reason),
                    }
                }
            }
            Command::Observe {
                operation_id,
                wait_ms,
            } => {
                if wait_ms > wire::MAX_WAIT_MS || !wire::valid_operation_id(&operation_id) {
                    Err(Refusal::InvalidRequest)
                } else {
                    let receiver = self.ledger.lock().observe(&operation_id);
                    Ok(Response::Operation {
                        outcome: match receiver {
                            Some(receiver) => wait_outcome(receiver, wait_ms).await,
                            None => Outcome::Missing,
                        },
                    })
                }
            }
            Command::Events { after, wait_ms } => {
                if wait_ms > wire::MAX_WAIT_MS {
                    Err(Refusal::InvalidRequest)
                } else {
                    self.events(after, wait_ms).await
                }
            }
        };
        result.unwrap_or_else(|reason| Response::Refused { reason })
    }

    async fn events(&self, after: u64, wait_ms: u64) -> Result<Response, Refusal> {
        let mut changed = self.ledger.lock().event_changed.subscribe();
        let (events, through) = self.ledger.lock().events(after)?;
        if events.is_empty() && self.ledger.lock().lost {
            return Err(Refusal::EnvironmentLost);
        }
        if events.is_empty() && wait_ms > 0 && !self.ledger.lock().lost {
            let _ = tokio::time::timeout(Duration::from_millis(wait_ms), changed.changed()).await;
            let (events, through) = self.ledger.lock().events(after)?;
            return Ok(Response::Events { events, through });
        }
        Ok(Response::Events { events, through })
    }
}

async fn wait_outcome(mut receiver: watch::Receiver<Outcome>, wait_ms: u64) -> Outcome {
    if *receiver.borrow() == Outcome::Pending && wait_ms > 0 {
        let _ = tokio::time::timeout(Duration::from_millis(wait_ms), receiver.changed()).await;
    }
    receiver.borrow().clone()
}

async fn connection(host: Arc<Host>, stream: UnixStream) -> Result<()> {
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    // A connection may issue successive bounded requests; waiting and polling
    // is transport work and never enters the model conversation.
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(30), read_json(&mut read)).await??;
        let request: Request =
            serde_json::from_value(frame).context("invalid execution request")?;
        let response = host.request(request).await;
        tokio::time::timeout(Duration::from_secs(10), write_json(&mut write, &response)).await??;
        if response == Response::Closed {
            host.close.notify_one();
            return Ok(());
        }
    }
}

/// Start exactly one target executor. The Machine must supply an owned pinned
/// component; using another Plugin's installed private CLI is not a launch path.
pub async fn run(args: Args) -> Result<()> {
    let contract_metadata = std::fs::symlink_metadata(&args.contract)?;
    ensure!(
        contract_metadata.is_file() && contract_metadata.mode() & 0o077 == 0,
        "execution contract must be a private regular file"
    );
    ensure!(
        contract_metadata.len() <= 64 * 1024,
        "execution contract exceeds limit"
    );
    let contract: LaunchContract = serde_json::from_slice(&std::fs::read(&args.contract)?)?;
    validate_contract(&contract)?;
    tracing::info!(event_name = "cowboy.execution.starting", session = %contract.session_id, environment_id = %contract.binding.environment.id, incarnation = %contract.binding.environment.incarnation, "starting owned execution environment");
    ensure!(
        args.state_dir.is_absolute(),
        "execution state path must be absolute"
    );
    let metadata = std::fs::symlink_metadata(&args.state_dir)?;
    ensure!(
        metadata.is_dir()
            && metadata.mode() & 0o077 == 0
            && metadata.uid() == contract_metadata.uid(),
        "execution state directory must be private"
    );
    let executor = std::fs::File::open(&contract.executor.command)?;
    let digest = tokio::task::spawn_blocking(move || {
        let mut file = executor;
        let mut hash = Sha256::new();
        std::io::copy(&mut file, &mut hash).map(|_| format!("{:x}", hash.finalize()))
    })
    .await??;
    ensure!(
        digest == contract.executor.sha256,
        "executor digest differs"
    );
    let cwd = PathBuf::from(&contract.binding.workspace.cwd);
    let metadata = std::fs::metadata(&cwd)?;
    ensure!(metadata.is_dir(), "execution workspace unavailable");
    // An existing marker is never treated as a stale PID/socket to remove.
    // Lost keepers require explicit new incarnation admission by the Machine.
    let marker = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(args.state_dir.join("started.json"))
        .context("execution incarnation has already started")?;
    serde_json::to_writer(&marker, &Scope::from_binding(&contract.binding))?;
    marker.sync_all()?;
    std::fs::File::open(&args.state_dir)?.sync_all()?;
    let socket = args
        .socket
        .unwrap_or_else(|| args.state_dir.join("keeper.sock"));
    ensure!(
        socket.is_absolute()
            && socket.file_name().is_some_and(|name| name == "keeper.sock")
            && socket
                .parent()
                .is_some_and(|parent| std::fs::canonicalize(parent).ok()
                    == std::fs::canonicalize(&args.state_dir).ok()),
        "control socket must belong to the private execution directory"
    );
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    let private_home = args.state_dir.join("executor-home");
    std::fs::create_dir(&private_home)?;
    std::fs::set_permissions(&private_home, std::fs::Permissions::from_mode(0o700))?;
    let mut child = ProcessCommand::new(&contract.executor.command)
        .args(["exec-server", "--listen", "stdio"])
        .current_dir(&cwd)
        .env_clear()
        .envs(&contract.environment)
        .env("CODEX_HOME", &private_home)
        .env("COWBOY_EXECUTION_FILE_HELPER", std::env::current_exe()?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("starting owned executor")?;
    let mut input = child.stdin.take().context("executor input unavailable")?;
    let mut native_errors = child
        .stderr
        .take()
        .context("executor diagnostics unavailable")?;
    let stderr_reader = tokio::spawn(async move {
        // Native stderr is unstructured and may contain command data. Drain it
        // without blocking the executor, retaining only bounded metadata.
        let mut buffer = [0_u8; 4096];
        let mut bytes = 0_u64;
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                result = native_errors.read(&mut buffer) => match result {
                    Ok(0) | Err(_) => break,
                    Ok(count) => { bytes = bytes.saturating_add(count as u64); }
                },
                _ = interval.tick(), if bytes > 0 => {
                    tracing::warn!(event_name = "cowboy.execution.native_stderr", reason = "native_diagnostic_output", bytes, "native executor wrote diagnostics; content excluded");
                    bytes = 0;
                }
            }
        }
        if bytes > 0 {
            tracing::warn!(
                event_name = "cowboy.execution.native_stderr",
                reason = "native_diagnostic_output",
                bytes,
                "native executor wrote diagnostics before exit"
            );
        }
    });
    let mut output = BufReader::new(child.stdout.take().context("executor output unavailable")?);
    let initialization = tokio::time::timeout(Duration::from_secs(15), async {
        write_json(&mut input, &json!({"id": 0, "method": "initialize", "params": {"clientName": "cowboy-machine-executor"}})).await?;
        let reply = read_json(&mut output).await?;
        ensure!(reply.get("id") == Some(&json!(0)) && reply.get("error").is_none(), "executor initialization refused");
        let result = reply.get("result").context("executor initialization missing")?.clone();
        ensure!(result.pointer("/environmentInfo/executorVersion").and_then(Value::as_str) == Some(contract.executor.version.as_str()), "executor protocol version differs");
        let actual_cwd = result.pointer("/environmentInfo/cwd").and_then(Value::as_str).and_then(|path| url::Url::parse(path).ok()).and_then(|path| path.to_file_path().ok());
        ensure!(actual_cwd.as_ref() == Some(&cwd), "executor working directory differs");
        write_json(&mut input, &json!({"method": "initialized", "params": {}})).await?;
        Ok::<_, anyhow::Error>(result)
    }).await??;
    let (writer, mut requests) = mpsc::channel(64);
    tracing::info!(
        event_name = "cowboy.execution.ready",
        "native executor initialized"
    );
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let (finished, finished_rx) = watch::channel(false);
    let host = Arc::new(Host {
        scope: Scope::from_binding(&contract.binding),
        call_context: contract.call_context.clone(),
        capability_digest: Sha256::digest(contract.capability.as_bytes()).into(),
        initialization,
        cwd,
        inode: (metadata.dev(), metadata.ino()),
        ledger: Mutex::new(Ledger::new()),
        writer,
        stop: Mutex::new(Some(stop)),
        stopped: finished_rx,
        close: tokio::sync::Notify::new(),
    });
    let (messages, mut native_messages) = mpsc::channel(4);
    let reader = tokio::spawn(async move {
        loop {
            let message = read_json_bounded(&mut output, 32 * 1024 * 1024).await;
            let ended = message.is_err();
            if messages.send(message).await.is_err() || ended {
                break;
            }
        }
    });
    let backend_host = Arc::clone(&host);
    let backend = tokio::spawn(async move {
        // Poll both directions independently. A full executor output pipe must
        // not prevent us from draining it while a large input frame is being
        // written. Bounded admission owns the queued effects; backpressure is
        // not evidence that the executor died and grants no replay authority.
        let mut native_writer = Box::pin(async {
            while let Some(request) = requests.recv().await {
                write_json(&mut input, &request).await?;
            }
            Ok::<_, anyhow::Error>(())
        });
        let mut event_space = backend_host.ledger.lock().event_space.subscribe();
        let mut pending_event = None;
        let mut pressure_started: Option<std::time::Instant> = None;
        loop {
            if let Some(message) = pending_event.take() {
                match backend_host.ledger.lock().push_event(message) {
                    Ok(()) => {
                        if let Some(started) = pressure_started.take() {
                            tracing::info!(
                                event_name = "cowboy.execution.backpressure_cleared",
                                duration_ms = started.elapsed().as_secs_f64() * 1000.0,
                                "execution event consumer caught up"
                            );
                        }
                    }
                    Err(EventError::Full(message)) => {
                        if pressure_started.is_none() {
                            pressure_started = Some(std::time::Instant::now());
                            tracing::warn!(
                                event_name = "cowboy.execution.backpressure",
                                reason = "unacknowledged_event_capacity",
                                "execution event consumer is behind"
                            );
                        }
                        pending_event = Some(message);
                    }
                    Err(EventError::Invalid) => {
                        tracing::warn!(
                            event_name = "cowboy.execution.backend_stopped",
                            reason = "native_event_limit",
                            "execution backend stopped"
                        );
                        break;
                    }
                }
            }
            tokio::select! {
                _ = &mut native_writer => {
                    tracing::warn!(event_name = "cowboy.execution.backend_stopped", reason = "native_input_closed", "execution backend stopped");
                    break;
                }
                message = native_messages.recv(), if pending_event.is_none() => {
                    let Some(Ok(mut message)) = message else {
                        tracing::warn!(event_name = "cowboy.execution.backend_stopped", reason = "native_stream_closed", "execution backend stopped");
                        break;
                    };
                    let mut ledger = backend_host.ledger.lock();
                    let result = if let Some(id) = message.get("id").and_then(Value::as_u64) {
                        if message.get("method").is_some() || message.get("result").is_some() == message.get("error").is_some() { break; }
                        message.as_object_mut().expect("response object").remove("id");
                        if serde_json::to_vec(&message).map_or(true, |bytes| bytes.len() > wire::MAX_FRAME_BYTES - 4096) {
                            message = json!({"error":{"code":-32000,"message":"Execution finished, but its reply exceeds the retained transport limit; do not repeat a write or command. Use a bounded read to inspect the result."}});
                        }
                        ledger.finish(id, message)
                    } else if message.get("method").and_then(Value::as_str).is_some() {
                        pending_event = Some(message);
                        Ok(())
                    } else { Err(()) };
                    if result.is_err() { break; }
                }
                _ = event_space.changed(), if pending_event.is_some() => {},
                status = child.wait() => {
                    tracing::warn!(event_name = "cowboy.execution.native_exited", exit_code = status.ok().and_then(|s| s.code()).unwrap_or(-1), "native executor exited");
                    break;
                },
                _ = &mut stopped => {
                    tracing::info!(event_name = "cowboy.execution.shutdown_requested", "execution shutdown requested");
                    break;
                },
            }
        }
        backend_host.ledger.lock().lose();
        // EOF gives the executor a chance to reap its owned process groups.
        // The Machine service additionally owns the complete cgroup.
        drop(native_writer);
        drop(input);
        if tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .is_err()
        {
            let _ = child.kill().await;
        }
        reader.abort();
        let _ = reader.await;
        stderr_reader.abort();
        let _ = stderr_reader.await;
        let _ = finished.send(true);
    });
    let connections = Arc::new(Semaphore::new(32));
    let mut shutdown = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        tokio::select! {
            _ = shutdown.recv() => break,
            _ = host.close.notified() => break,
            _ = tokio::signal::ctrl_c() => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let Ok(permit) = Arc::clone(&connections).try_acquire_owned() else { drop(stream); continue; };
                let host = Arc::clone(&host);
                tokio::spawn(async move {
                    let _permit = permit;
                    let _ = connection(host, stream).await;
                });
            }
        }
    }
    host.ledger.lock().lose();
    if let Some(stop) = host.stop.lock().take() {
        let _ = stop.send(());
    }
    let _ = backend.await;
    tracing::info!(
        event_name = "cowboy.execution.stopped",
        "execution environment stopped"
    );
    Ok(())
}
