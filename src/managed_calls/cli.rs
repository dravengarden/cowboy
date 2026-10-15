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

use super::protocol::{Action, LocalRequest, MAX_WAIT_MS, valid_capability};
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
    /// While the session's policy refuses this call, wait this long for a
    /// person to allow it in Cowboy, resubmitting the same request.
    #[arg(long, default_value_t = 100_000, value_parser = clap::value_parser!(u64).range(0..=3_600_000))]
    approval_wait_ms: u64,
    /// One of the called agent's presets (model and reasoning) for this
    /// call; `cowboy call capabilities` lists them. Default: the session's.
    #[arg(long)]
    preset: Option<String>,
}

/// How often a waiting request asks again; it also keeps the prompt shown.
const APPROVAL_POLL: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Args)]
pub struct CallArgs {
    #[command(subcommand)]
    command: CallCommand,
}

#[derive(Subcommand)]
enum CallCommand {
    /// Submit using the same contract as the Provider shortcuts.
    Start {
        #[arg(long, value_parser = ["codex", "claude-code", "auto"])]
        provider: String,
        #[command(flatten)]
        request: StartArgs,
    },
    /// Inspect the current parent-scoped launch capabilities.
    Capabilities,
    /// Observe an accepted call without submitting another prompt.
    Inspect { call_id: String },
    /// Wait for the same accepted call; a deadline does not cancel it.
    /// Waits longer than one gateway exchange are split into slices here.
    Wait {
        call_id: String,
        #[arg(long, visible_alias = "wait-ms", default_value_t = 30_000, value_parser = clap::value_parser!(u64).range(0..=3_600_000))]
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
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Wiring {
    schema: u16,
    socket: PathBuf,
    capability: String,
}

fn error(code: &str, admission: &str) -> Value {
    json!({"schema":1,"state":"error","error":{"code":code,"admission":admission}})
}

fn emit(value: &Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string(value)?);
    if refused(value) {
        anyhow::bail!("managed call refused; see JSON error code");
    }
    Ok(())
}

/// Call summaries always carry `error`; only a non-null code is a refusal.
fn refused(value: &Value) -> bool {
    value.get("error").is_some_and(|error| !error.is_null())
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

pub(crate) fn wiring() -> Result<Wiring, &'static str> {
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

pub(crate) async fn exchange(context: Wiring, action: Action, timeout_ms: u64) -> Value {
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
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_millis(args.approval_wait_ms);
    let mut announced = false;
    let mut value = loop {
        let value = exchange(
            context.clone(),
            Action::Start {
                provider: provider.into(),
                request: Box::new(request.clone()),
                wait_ms: args.wait_ms,
                preset: args.preset.clone(),
            },
            args.wait_ms,
        )
        .await;
        if value.get("approval").and_then(Value::as_str) != Some("pending")
            || std::time::Instant::now() + APPROVAL_POLL > deadline
        {
            break value;
        }
        if !announced {
            announced = true;
            eprintln!(
                "Agent calls are off for this Cowboy session; waiting for the user to allow this call in Cowboy..."
            );
        }
        tokio::time::sleep(APPROVAL_POLL).await;
    };
    if value.get("approval").and_then(Value::as_str) == Some("pending") {
        // Nobody answered in time; the same request may simply be run again.
        value["request_id"] = json!(request_id);
        value["hint"] = json!(
            "The user has not allowed this call yet. They can allow it from the prompt in Cowboy or turn on Agent calls in the session's Tools; rerun the same command afterwards."
        );
    }
    if value.pointer("/error/admission").and_then(Value::as_str) == Some("unknown") {
        // Observing never submits, so resolving a lost response here is
        // safe; only an unresolved outcome is left to the caller.
        if let Some(observed) = observe(&context, &request_id).await {
            return emit(&observed);
        }
        value["request_id"] = json!(request_id);
        value["next"] = json!({"argv":["cowboy","call","observe",request_id]});
    }
    emit(&value)
}

/// Resolve a start whose response was lost by its request identity.
async fn observe(context: &Wiring, request_id: &str) -> Option<Value> {
    for attempt in 0..4 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        let value = exchange(
            context.clone(),
            Action::Observe {
                request_id: request_id.to_owned(),
            },
            0,
        )
        .await;
        if value.get("call_id").and_then(Value::as_str).is_some() {
            return Some(value);
        }
    }
    None
}

/// Wait in gateway-sized slices until the call is terminal, the caller's
/// deadline passes, or an exchange fails.
async fn wait(context: Wiring, call_id: String, timeout_ms: u64) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let slice = u64::try_from(left.as_millis())
            .unwrap_or(u64::MAX)
            .min(MAX_WAIT_MS);
        let value = exchange(
            context.clone(),
            Action::Wait {
                call_id: call_id.clone(),
                timeout_ms: slice,
            },
            slice,
        )
        .await;
        let open = value
            .get("state")
            .and_then(Value::as_str)
            .is_some_and(|state| {
                !matches!(
                    state,
                    "completed" | "failed" | "cancelled" | "waiting_input" | "error"
                )
            });
        if refused(&value) || !open || std::time::Instant::now() >= deadline {
            return value;
        }
    }
}

pub async fn call(args: CallArgs) -> anyhow::Result<()> {
    if let CallCommand::Wait {
        call_id,
        timeout_ms,
    } = args.command
    {
        if !valid_id(&call_id) {
            return emit(&error("invalid_identity", "not_submitted"));
        }
        return match wiring() {
            Ok(context) => emit(&wait(context, call_id, timeout_ms).await),
            Err(code) => emit(&error(code, "not_submitted")),
        };
    }
    let (action, wait_ms) = match args.command {
        CallCommand::Start { provider, request } => return start(&provider, request).await,
        CallCommand::Capabilities => (Action::Capabilities {}, 0),
        CallCommand::Inspect { call_id } => (Action::Inspect { call_id }, 0),
        CallCommand::Wait { .. } => unreachable!("handled above"),
        CallCommand::Result { call_id } => (Action::Result { call_id }, 0),
        CallCommand::Cancel { call_id } => (Action::Cancel { call_id }, 0),
        CallCommand::Observe { request_id } => (Action::Observe { request_id }, 0),
    };
    let id = match &action {
        Action::Capabilities {}
        | Action::Start { .. }
        | Action::AgentStatus {}
        | Action::AgentRead { .. } => None,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake gateway answering each exchange from `reply` and recording the
    /// actions it received.
    fn gateway(
        reply: impl Fn(&Value) -> Value + Send + Sync + 'static,
    ) -> (
        tempfile::TempDir,
        Wiring,
        std::sync::Arc<parking_lot::Mutex<Vec<Value>>>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("gateway.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let log = std::sync::Arc::clone(&seen);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let frame: Value = serde_json::from_str(&line).unwrap();
                log.lock().push(frame["action"].clone());
                let mut bytes = serde_json::to_vec(&reply(&frame["action"])).unwrap();
                bytes.push(b'\n');
                reader.get_mut().write_all(&bytes).await.unwrap();
            }
        });
        let wiring = Wiring {
            schema: 1,
            socket,
            capability: "a".repeat(64),
        };
        (dir, wiring, seen)
    }

    #[tokio::test]
    async fn a_long_wait_is_split_into_gateway_sized_slices() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let (_dir, wiring, seen) = gateway(move |_| {
            let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let state = if n < 2 { "running" } else { "completed" };
            json!({"schema":1,"state":state,"call_id":"call-1","error":null})
        });
        let value = wait(wiring, "call-1".into(), 600_000).await;
        assert_eq!(value["state"], "completed");
        let seen = seen.lock();
        assert_eq!(seen.len(), 3);
        assert!(
            seen.iter().all(|action| action["kind"] == "wait"
                && action["timeout_ms"].as_u64() == Some(MAX_WAIT_MS))
        );
    }

    #[tokio::test]
    async fn a_wait_returns_when_the_call_needs_input_or_is_refused() {
        let (_dir, wiring, seen) = gateway(
            |_| json!({"schema":1,"state":"waiting_input","call_id":"call-1","error":null}),
        );
        assert_eq!(
            wait(wiring, "call-1".into(), 600_000).await["state"],
            "waiting_input"
        );
        assert_eq!(seen.lock().len(), 1);
        let (_dir, wiring, seen) = gateway(|_| error("not_found", "not_submitted"));
        assert!(refused(&wait(wiring, "call-1".into(), 600_000).await));
        assert_eq!(seen.lock().len(), 1);
    }

    #[tokio::test]
    async fn a_lost_start_response_resolves_by_observing_its_request() {
        let (_dir, wiring, seen) = gateway(|action| {
            assert_eq!(action["kind"], "observe", "observing must never resubmit");
            json!({"schema":1,"state":"running","call_id":"call-1","error":null})
        });
        let observed = observe(&wiring, "request-1").await.unwrap();
        assert_eq!(observed["call_id"], "call-1");
        assert_eq!(seen.lock()[0]["request_id"], "request-1");
        let (_dir, wiring, _) = gateway(|_| error("not_found", "not_submitted"));
        assert!(observe(&wiring, "request-1").await.is_none());
    }

    #[test]
    fn only_a_present_error_code_is_a_refusal() {
        assert!(refused(&error("grant_revoked", "not_submitted")));
        assert!(refused(
            &json!({"state":"failed","error":{"code":"timeout"}})
        ));
        assert!(!refused(&json!({"state":"completed","error":null})));
        assert!(!refused(&json!({"state":"running"})));
    }
}
