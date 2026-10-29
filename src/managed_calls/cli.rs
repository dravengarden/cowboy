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
    /// When the call ends, Cowboy queues one notice into this session. End
    /// your turn instead of polling `cowboy call wait`.
    #[arg(long)]
    notify: bool,
}

/// The request contract, shown by `--help` so the always-on agent prompt need
/// not carry it. For a code review, `cowboy review` builds all of this.
pub const START_HELP: &str = r#"Request (JSON on stdin with --request-file -):
  {"schema":1,"request_id":"<new unique id>","purpose":"review|design_review|analysis|image_generate|image_edit",
   "instruction":"<self-contained task>","context":{"scope":"current-worktree"},
   "access":"read-only","conversation":{"mode":"fresh"}}

The command prints a JSON envelope with a call_id. With --notify, end your
turn: Cowboy posts a notice into this session when the call ends. Otherwise:
  cowboy call wait <call_id>     until the state is terminal
  cowboy call result <call_id>   the reply text
cowboy call capabilities lists whether calls are on, native capabilities and presets.
Image calls copy only context.files (up to 4 relative image paths). image_edit
requires input files or conversation:{"mode":"continue","child_session_id":"..."}.
Results include verified artifact paths on this execution target, without bytes."#;

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
pub struct Wiring {
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

pub fn wiring() -> Result<Wiring, &'static str> {
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

pub async fn exchange(context: Wiring, action: Action, timeout_ms: u64) -> Value {
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
    let mut request = match Request::parse(&bytes) {
        Ok(request) => request,
        Err(reason) => return emit(&error(&reason.to_string(), "not_submitted")),
    };
    if args.notify {
        request
            .labels
            .insert(super::review::NOTIFY.to_owned(), "parent".to_owned());
        if !super::valid_labels(&request.labels) {
            return emit(&error("invalid_labels", "not_submitted"));
        }
    }
    if args.check {
        return emit(&json!({"schema":1,"state":"validated","authorized":false,
            "provider":provider,"request_id":request.request_id,"request_digest":request.digest()}));
    }
    let context = match wiring() {
        Ok(context) => context,
        Err(code) => return emit(&error(code, "not_submitted")),
    };
    emit(
        &submit(
            &context,
            provider,
            &request,
            args.wait_ms,
            args.approval_wait_ms,
            args.preset,
        )
        .await,
    )
}

/// Submit one request, waiting up to `approval_wait_ms` for a person to allow
/// it while the session's policy refuses it, and resolve a lost response.
async fn submit(
    context: &Wiring,
    provider: &str,
    request: &Request,
    wait_ms: u64,
    approval_wait_ms: u64,
    preset: Option<String>,
) -> Value {
    let request_id = request.request_id.clone();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(approval_wait_ms);
    let mut announced = false;
    let mut value = loop {
        let value = exchange(
            context.clone(),
            Action::Start {
                provider: provider.into(),
                request: Box::new(request.clone()),
                wait_ms,
                preset: preset.clone(),
            },
            wait_ms,
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
        if let Some(observed) = observe(context, &request_id).await {
            return observed;
        }
        value["request_id"] = json!(request_id);
        value["next"] = json!({"argv":["cowboy","call","observe",request_id]});
    }
    value
}

/// A second-opinion code review from another agent, shaped for the agent that
/// asks: plain text in, one blocking command, only the findings out. Each
/// printed byte is read back into the caller's context, so nothing else is.
#[derive(Args)]
pub struct ReviewArgs {
    /// Short task title shown in Calls (no generated request ID).
    #[arg(long)]
    title: Option<String>,
    /// Why you chose this model/effort after reading call capabilities and quota.
    #[arg(long)]
    reason: Option<String>,
    /// Absolute Git worktree to review on the execution target. Defaults to
    /// the current Git worktree; the target validates registration.
    #[arg(long)]
    root: Option<std::path::PathBuf>,
    /// Stable task id (letters, digits, '-', '_', '.'). With --base, reuse the
    /// same complete review input.
    #[arg(long, requires = "base", value_parser = task_id)]
    task: Option<String>,
    /// Base commit/ref for --task; resolved to the immutable merge-base with HEAD.
    #[arg(long, requires = "task")]
    base: Option<String>,
    /// Deliberately request an independent review even if this input was reviewed.
    #[arg(long, requires = "task")]
    fresh: bool,
    /// Review design documents (specs, plans) instead of code.
    #[arg(long)]
    design: bool,
    /// The user asked for this review: if the session's reviews are off, ask
    /// them in Cowboy and wait for the answer instead of skipping.
    #[arg(long)]
    ask: bool,
    /// Reviewer agent; Auto lets the session's Tools choose.
    #[arg(long, default_value = "auto", value_parser = ["auto", "codex", "claude"])]
    agent: String,
    /// One of the reviewer's presets (model and reasoning) for this review.
    #[arg(long)]
    preset: Option<String>,
    /// Return once the review is admitted. Cowboy queues one notice into
    /// this session when it ends; end your turn, then read
    /// `cowboy call result <call_id>`. Use this when a shell command cannot
    /// wait for a whole review.
    #[arg(long)]
    notify: bool,
    /// Give up waiting after this many minutes; the review keeps running and
    /// its call id is printed.
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u64).range(1..=60))]
    timeout_min: u64,
}

fn task_id(value: &str) -> Result<String, String> {
    if valid_id(value) {
        Ok(value.to_owned())
    } else {
        Err("use a stable id of up to 128 letters, digits, '-', '_' or '.'; describe the review in the focus text".into())
    }
}

fn review_request(args: &ReviewArgs, focus: &str) -> Result<Request, String> {
    let root = match &args.root {
        Some(root) => root.clone(),
        None => {
            let output = std::process::Command::new("git")
                .args(["rev-parse", "--show-toplevel"])
                .output()
                .map_err(|_| "worktree unavailable".to_owned())?;
            if !output.status.success() {
                return Err("not a Git worktree".into());
            }
            PathBuf::from(
                String::from_utf8(output.stdout)
                    .map_err(|_| "invalid worktree path".to_owned())?
                    .trim_end_matches('\n'),
            )
        }
    };
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    let mut request = json!({
        "schema": 1,
        "request_id": format!("review-{millis}-{}", std::process::id()),
        "labels": {
            "title": args.title.as_deref().unwrap_or(if args.design { "Design review" } else { "Code review" }),
            "selection_reason": args.reason.as_deref().unwrap_or("Session Tools selection"),
        },
        "purpose": if args.design { "design_review" } else { "review" },
        "instruction": super::review_instruction(args.design, focus, None),
        "context": {"scope": "current-worktree", "root": root},
        "access": "read-only",
        "conversation": {"mode": "fresh"},
    });
    if let (Some(task), Some(base)) = (&args.task, &args.base) {
        if base.starts_with('-') {
            return Err("invalid base".into());
        }
        let resolved = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["merge-base", "HEAD", base])
            .output()
            .map_err(|_| "base unavailable")?;
        let base = String::from_utf8(resolved.stdout).map_err(|_| "invalid base")?;
        let base = base.trim();
        if !resolved.status.success() || !super::review::valid_base(base) {
            return Err("base unavailable".into());
        }
        request["labels"][super::review::TASK] = json!(task);
        request["labels"][super::review::BASE] = json!(base);
        request["labels"][super::review::REUSE] = json!((!args.fresh).to_string());
        request["instruction"] = json!(super::review_instruction(args.design, focus, Some(base)));
        request["output"] = json!({"format":"json_schema","schema":super::review::output_schema()});
    }
    if args.notify {
        request["labels"][super::review::NOTIFY] = json!("parent");
    }
    Request::parse(request.to_string().as_bytes()).map_err(|reason| reason.to_string())
}

/// One line for an outcome that has no findings to show.
fn review_line(value: &Value) -> String {
    if value.pointer("/error/admission").and_then(Value::as_str) == Some("unknown")
        && let Some(request) = value
            .get("request_id")
            .and_then(Value::as_str)
            .filter(|id| valid_id(id))
    {
        return format!("review outcome unknown; inspect with: cowboy call observe {request}");
    }
    let code = value
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match code {
        // The session decides whether reviews run; an agent that chose to
        // ask needs only to know it should carry on without one.
        "calls_disabled" | "policy_denied" | "calls_declined" | "same_agent" => {
            format!("skipped: reviews are off for this session ({code})")
        }
        "concurrency_limit" | "call_limit" => format!("skipped: review budget reached ({code})"),
        _ => format!("review unavailable: {code}"),
    }
}

pub async fn review(args: ReviewArgs) -> anyhow::Result<()> {
    let mut focus = String::new();
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        std::io::stdin()
            .take(MAX_REQUEST_BYTES as u64 + 1)
            .read_to_string(&mut focus)
            .map_err(|_| anyhow::anyhow!("review unavailable: input_unreadable"))?;
    }
    let outcome = match wiring() {
        Ok(context) => run_review(&context, &args, &focus).await,
        Err(code) => Err(format!("review unavailable: {code}")),
    };
    match outcome {
        Ok(text) => {
            println!("{text}");
            Ok(())
        }
        Err(line) => {
            println!("{line}");
            anyhow::bail!("{line}")
        }
    }
}

/// What `cowboy review` prints: the findings, or one `skipped:` line, both
/// successes; any other outcome is one error line.
async fn run_review(context: &Wiring, args: &ReviewArgs, focus: &str) -> Result<String, String> {
    let request =
        review_request(args, focus).map_err(|code| format!("review unavailable: {code}"))?;
    let provider = match args.agent.as_str() {
        "claude" => "claude-code",
        other => other,
    };
    let approval_wait_ms = if args.ask { 600_000 } else { 0 };
    let started = submit(
        context,
        provider,
        &request,
        MAX_WAIT_MS.min(60_000),
        approval_wait_ms,
        args.preset.clone(),
    )
    .await;
    let Some(call_id) = started.get("call_id").and_then(Value::as_str) else {
        let line = review_line(&started);
        // Skipping is an answer, not a failure.
        return if line.starts_with("skipped") {
            Ok(line)
        } else {
            Err(line)
        };
    };
    let call_id = call_id.to_owned();
    let state = started.get("state").and_then(Value::as_str);
    if args.notify && state != Some("completed") {
        return Ok(format!(
            "review started: call {call_id}; Cowboy will post a notice in this session when it \
             ends. End your turn, then read: cowboy call result {call_id}"
        ));
    }
    let settled = if state == Some("completed") {
        started.clone()
    } else {
        wait(context.clone(), call_id.clone(), args.timeout_min * 60_000).await
    };
    match settled.get("state").and_then(Value::as_str) {
        Some("completed") => {}
        Some(state) if state != "error" => {
            let code = settled
                .pointer("/error/code")
                .and_then(Value::as_str)
                .filter(|code| valid_id(code));
            let detail = code.map(|code| format!(" ({code})")).unwrap_or_default();
            return Err(format!("review {state}{detail}; call {call_id}"));
        }
        _ => return Err(review_line(&settled)),
    }
    let result = exchange(
        context.clone(),
        Action::Result {
            call_id: call_id.clone(),
        },
        0,
    )
    .await;
    let text = result
        .pointer("/result/text")
        .and_then(Value::as_str)
        .ok_or_else(|| review_line(&result))?;
    let mut text = text.trim().to_owned();
    if text.is_empty() {
        return Err(format!("review incomplete: empty result; call {call_id}"));
    }
    if result.pointer("/result/truncated") == Some(&Value::Bool(true)) {
        text.push_str(&format!(
            "\nReview incomplete: retained output was truncated; call {call_id}."
        ));
    }
    Ok(text)
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
        | Action::AgentInvoke { .. }
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

    fn review_args(design: bool) -> ReviewArgs {
        ReviewArgs {
            task: None,
            base: None,
            fresh: false,
            title: None,
            reason: None,
            root: Some("/tasks/review".into()),
            design,
            ask: false,
            agent: "auto".into(),
            preset: None,
            notify: false,
            timeout_min: 20,
        }
    }

    #[test]
    fn a_review_request_is_valid_bounded_and_asks_for_findings_only() {
        let request = review_request(&review_args(false), "  \n").unwrap();
        assert_eq!(request.purpose, super::super::Purpose::Review);
        assert!(
            request
                .instruction
                .starts_with("Review this worktree's change")
        );
        assert_eq!(request.context.root.as_deref(), Some("/tasks/review"));
        assert!(request.instruction.ends_with("reply exactly: No findings."));
        assert!(valid_id(&request.request_id));
        let mut choice = review_args(false);
        choice.title = Some("Permission boundaries".into());
        choice.reason = Some("Fable model window constrained; Opus High is available".into());
        let selected = review_request(&choice, "").unwrap();
        assert_eq!(selected.labels["title"], "Permission boundaries");
        assert_eq!(
            selected.labels["selection_reason"],
            choice.reason.as_deref().unwrap()
        );
        choice.title = Some("x".repeat(257));
        assert!(review_request(&choice, "").is_err());
        let focused = review_request(&review_args(true), "Check the migration order").unwrap();
        assert_eq!(focused.purpose, super::super::Purpose::DesignReview);
        assert!(
            focused
                .instruction
                .contains("Focus: Check the migration order\n")
        );
        assert!(review_request(&review_args(false), &"x".repeat(200 * 1024)).is_err());
    }

    #[tokio::test]
    async fn a_review_submits_waits_and_prints_only_the_findings() {
        let (_dir, wiring, seen) = gateway(|action| match action["kind"].as_str() {
            Some("start") => json!({"schema":1,"state":"running","call_id":"call-1","error":null}),
            Some("wait") => json!({"schema":1,"state":"completed","call_id":"call-1","error":null}),
            Some("result") => {
                json!({"schema":1,"state":"completed","call_id":"call-1","error":null,
                "result":{"text":"  src/a.rs:3 - unchecked index - bound it\n","truncated":false}})
            }
            other => panic!("unexpected action {other:?}"),
        });
        let text = run_review(&wiring, &review_args(false), "").await.unwrap();
        assert_eq!(text, "src/a.rs:3 - unchecked index - bound it");
        let seen = seen.lock();
        assert_eq!(
            seen.iter()
                .map(|action| action["kind"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["start", "wait", "result"]
        );
        assert_eq!(seen[0]["provider"], "auto");
        assert_eq!(seen[0]["request"]["purpose"], "review");
    }

    #[tokio::test]
    async fn a_notified_review_returns_after_admission_without_waiting() {
        let (_dir, wiring, seen) =
            gateway(|_| json!({"schema":1,"state":"running","call_id":"call-1","error":null}));
        let mut args = review_args(false);
        args.notify = true;
        let text = run_review(&wiring, &args, "").await.unwrap();
        assert!(
            text.starts_with("review started: call call-1;")
                && text.ends_with("cowboy call result call-1"),
            "{text}"
        );
        let seen = seen.lock();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0]["request"]["labels"]["notify"], "parent");
    }

    #[tokio::test]
    async fn a_review_off_for_the_session_skips_at_once_without_asking() {
        let (_dir, wiring, seen) = gateway(|_| {
            json!({"schema":1,"state":"error","approval":"pending",
                "error":{"code":"calls_disabled","admission":"not_submitted"}})
        });
        let text = run_review(&wiring, &review_args(false), "").await.unwrap();
        assert!(text.starts_with("skipped: "), "{text}");
        // No approval wait: one exchange, no resubmission loop.
        assert_eq!(seen.lock().len(), 1);
    }

    #[test]
    fn a_review_the_session_does_not_allow_is_a_quiet_skip() {
        for code in [
            "calls_disabled",
            "policy_denied",
            "calls_declined",
            "same_agent",
        ] {
            assert!(review_line(&error(code, "not_submitted")).starts_with("skipped: "));
        }
        assert_eq!(
            review_line(&error("gateway_unavailable", "unknown")),
            "review unavailable: gateway_unavailable"
        );
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
