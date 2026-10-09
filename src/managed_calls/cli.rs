//! AI-facing bounded Unix client. It never spawns an unmanaged Provider CLI.

use std::io::Read as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, Subcommand};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;

use super::protocol::{Action, LocalRequest, valid_capability};
use super::{MAX_REQUEST_BYTES, Request, valid_id};

#[derive(Args)]
pub struct StartArgs {
    /// JSON request on this execution target; '-' reads bounded stdin.
    #[arg(long)]
    request_file: PathBuf,
    /// Validate input without contacting a gateway or authorizing execution.
    #[arg(long)]
    check: bool,
    /// Bounded initial observation; elapsed waiting never cancels the call.
    #[arg(long, default_value_t = 30_000, value_parser = clap::value_parser!(u64).range(0..=60_000))]
    wait_ms: u64,
}

#[derive(Args)]
pub struct CallArgs {
    #[command(subcommand)]
    command: CallCommand,
}

#[derive(Subcommand)]
enum CallCommand {
    /// Submit using the same contract as the Provider shortcuts.
    Start {
        #[arg(long, value_parser = ["codex", "claude-code"])]
        provider: String,
        #[command(flatten)]
        request: StartArgs,
    },
    /// Inspect the current parent-scoped launch capabilities.
    Capabilities,
    /// Observe an accepted call without submitting another prompt.
    Inspect { call_id: String },
    /// Wait for the same accepted call; a deadline does not cancel it.
    Wait {
        call_id: String,
        #[arg(long, default_value_t = 30_000, value_parser = clap::value_parser!(u64).range(0..=60_000))]
        timeout_ms: u64,
    },
    /// Read the final result of an accepted call.
    Result { call_id: String },
    /// Request cancellation; stopping is not confirmed termination.
    Cancel { call_id: String },
    /// Resolve a lost start response by the original request identity.
    Observe { request_id: String },
}

// Deliberately no Debug: both wiring and RPC frames contain private grants.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wiring {
    schema: u16,
    socket: PathBuf,
    capability: String,
}

fn error(code: &str, admission: &str) -> Value {
    json!({"schema":1,"state":"error","error":{"code":code,"admission":admission}})
}

fn emit(value: &Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string(value)?);
    if value.get("error").is_some() {
        anyhow::bail!("managed call refused; see JSON error code");
    }
    Ok(())
}

fn read_input(path: &Path) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    if path == Path::new("-") {
        std::io::stdin()
            .take(MAX_REQUEST_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "input_unreadable")?;
    } else {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|_| "input_unreadable")?;
        if !file.metadata().map_err(|_| "input_unreadable")?.is_file() {
            return Err("input_not_regular_file");
        }
        file.take(MAX_REQUEST_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "input_unreadable")?;
    }
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err("request_too_large");
    }
    Ok(bytes)
}

fn wiring() -> Result<Wiring, &'static str> {
    let path = std::env::var_os("COWBOY_CALL_CONTEXT").ok_or("context_unavailable")?;
    if !Path::new(&path).is_absolute() {
        return Err("invalid_context");
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| "context_unavailable")?;
    let metadata = file.metadata().map_err(|_| "invalid_context")?;
    if !metadata.is_file()
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.len() > 8192
    {
        return Err("invalid_context");
    }
    let mut bytes = Vec::new();
    file.take(8193)
        .read_to_end(&mut bytes)
        .map_err(|_| "invalid_context")?;
    if bytes.len() > 8192 {
        return Err("invalid_context");
    }
    let context: Wiring = serde_json::from_slice(&bytes).map_err(|_| "invalid_context")?;
    if context.schema != 1
        || !context.socket.is_absolute()
        || !valid_capability(&context.capability)
    {
        return Err("invalid_context");
    }
    Ok(context)
}

async fn connect(socket: &Path) -> std::io::Result<UnixStream> {
    let stream = UnixStream::connect(socket).await?;
    if stream.peer_cred()?.uid() != rustix::process::geteuid().as_raw() {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    Ok(stream)
}

async fn exchange(context: Wiring, action: Action, timeout_ms: u64) -> Value {
    // No frame byte leaves this process before a verified connection exists,
    // so a connect failure proves non-admission and the caller may resubmit.
    let Ok(Ok(mut stream)) =
        tokio::time::timeout(Duration::from_secs(5), connect(&context.socket)).await
    else {
        return error("gateway_unavailable", "not_submitted");
    };
    // Once the write begins an error cannot prove non-admission. The caller
    // must observe its original id; this client never retries the start effect.
    let result = tokio::time::timeout(Duration::from_millis(timeout_ms + 5000), async {
        let frame = LocalRequest {
            schema: 1,
            capability: context.capability,
            action,
        };
        let mut bytes = serde_json::to_vec(&frame)?;
        bytes.push(b'\n');
        stream.write_all(&bytes).await?;
        let mut reader = BufReader::new(stream);
        let mut reply = Vec::new();
        loop {
            let chunk = reader.fill_buf().await?;
            if chunk.is_empty() {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            let end = chunk.iter().position(|b| *b == b'\n');
            let count = end.map_or(chunk.len(), |index| index + 1);
            if reply.len() + count > 4 * 1024 * 1024 {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            reply.extend_from_slice(&chunk[..count]);
            reader.consume(count);
            if end.is_some() {
                break;
            }
        }
        let value: Value = serde_json::from_slice(&reply)?;
        if value.get("schema").and_then(Value::as_u64) != Some(1) {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        Ok::<_, std::io::Error>(value)
    })
    .await;
    match result {
        Ok(Ok(value)) => value,
        _ => error("gateway_unavailable", "unknown"),
    }
}

pub async fn start(provider: &str, args: StartArgs) -> anyhow::Result<()> {
    let bytes = match read_input(&args.request_file) {
        Ok(bytes) => bytes,
        Err(code) => return emit(&error(code, "not_submitted")),
    };
    let request = match Request::parse(&bytes) {
        Ok(request) => request,
        Err(reason) => return emit(&error(&reason.to_string(), "not_submitted")),
    };
    if args.check {
        return emit(&json!({"schema":1,"state":"validated","authorized":false,
            "provider":provider,"request_id":request.request_id,"request_digest":request.digest()}));
    }
    let context = match wiring() {
        Ok(context) => context,
        Err(code) => return emit(&error(code, "not_submitted")),
    };
    let request_id = request.request_id.clone();
    let mut value = exchange(
        context,
        Action::Start {
            provider: provider.into(),
            request,
            wait_ms: args.wait_ms,
        },
        args.wait_ms,
    )
    .await;
    if value.pointer("/error/admission").and_then(Value::as_str) == Some("unknown") {
        value["request_id"] = json!(request_id);
        value["next"] = json!({"argv":["cowboy","call","observe",request_id]});
    }
    emit(&value)
}

pub async fn call(args: CallArgs) -> anyhow::Result<()> {
    let (action, wait_ms) = match args.command {
        CallCommand::Start { provider, request } => return start(&provider, request).await,
        CallCommand::Capabilities => (Action::Capabilities {}, 0),
        CallCommand::Inspect { call_id } => (Action::Inspect { call_id }, 0),
        CallCommand::Wait {
            call_id,
            timeout_ms,
        } => (
            Action::Wait {
                call_id,
                timeout_ms,
            },
            timeout_ms,
        ),
        CallCommand::Result { call_id } => (Action::Result { call_id }, 0),
        CallCommand::Cancel { call_id } => (Action::Cancel { call_id }, 0),
        CallCommand::Observe { request_id } => (Action::Observe { request_id }, 0),
    };
    let id = match &action {
        Action::Capabilities {} | Action::Start { .. } => None,
        Action::Observe { request_id } => Some(request_id),
        Action::Inspect { call_id }
        | Action::Wait { call_id, .. }
        | Action::Result { call_id }
        | Action::Cancel { call_id } => Some(call_id),
    };
    if id.is_some_and(|id| !valid_id(id)) {
        return emit(&error("invalid_identity", "not_submitted"));
    }
    let context = match wiring() {
        Ok(context) => context,
        Err(code) => return emit(&error(code, "not_submitted")),
    };
    emit(&exchange(context, action, wait_ms).await)
}
