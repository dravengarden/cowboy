//! Tool-only stdio sessions. One actor owns I/O and the process group; dropping
//! a request or its live connection closes the session rather than replaying it.
use std::io::Write as _;
use std::os::fd::AsRawFd as _;
use std::os::unix::{
    fs::{MetadataExt as _, OpenOptionsExt as _},
    process::CommandExt as _,
};
use std::process::Stdio;

use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Semaphore, mpsc, oneshot};

use super::super::{MachinePluginStore, McpInvocation};
use crate::plugin_process::PluginProcessGroup;

const MAX_FRAME: usize = 1024 * 1024;
static PROCESSES: Semaphore = Semaphore::const_new(8);

type Reply = oneshot::Sender<Result<Value>>;
struct Request {
    message: Value,
    reply: Reply,
    deadline: tokio::time::Instant,
}

pub struct McpProcess {
    requests: mpsc::Sender<Request>,
}

impl McpProcess {
    pub(super) fn closed(&self) -> bool {
        self.requests.is_closed()
    }

    pub async fn request(&self, message: Value) -> Result<Value> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .try_send(Request {
                message,
                reply,
                deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(30),
            })
            .map_err(|_| anyhow::anyhow!("MCP session busy or closed"))?;
        answer.await.context("MCP session ended")?
    }
}

impl MachinePluginStore {
    pub async fn open_mcp(&self, invocation: McpInvocation) -> Result<McpProcess> {
        invocation.check()?;
        let permit = PROCESSES.try_acquire().context("MCP process capacity")?;
        let lifecycle = tokio::select! {
            biased;
            () = invocation.ended() => anyhow::bail!("MCP authorization ended"),
            guard = self.lifecycle.lock() => guard,
        };
        let prepared = self.prepare_mcp_binding(&invocation.profile().binding)?;
        let executable = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&prepared.server.executable)?;
        let before = stamp(&executable.metadata()?);
        // Reverify under the lifecycle fence after opening the executable; the
        // child executes this descriptor, never a subsequently replaced path.
        self.prepare_mcp_binding(&invocation.profile().binding)?;
        ensure!(
            before == stamp(&executable.metadata()?)
                && before == stamp(&std::fs::metadata(&prepared.server.executable)?),
            "MCP executable changed during launch"
        );
        let mut profile = tempfile::NamedTempFile::new()?;
        profile.write_all(&serde_json::to_vec(invocation.profile())?)?;
        let profile_reader = std::fs::File::open(profile.path())?;
        // Parent descriptors pin the opened inode without unsafe pre_exec hooks.
        let program = format!("/proc/{}/fd/{}", std::process::id(), executable.as_raw_fd());
        let profile_path = format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            profile_reader.as_raw_fd()
        );
        let mut command = Command::new(program);
        command
            .args(&prepared.server.contract.arguments)
            .env_clear()
            .env("COWBOY_MCP_PROFILE", profile_path)
            .current_dir(
                prepared
                    .server
                    .executable
                    .parent()
                    .context("MCP executable has no directory")?,
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command.as_std_mut().process_group(0);
        invocation.check()?;
        let mut child = command.spawn().context("starting signed MCP server")?;
        let group = PluginProcessGroup::new(child.id().context("MCP child has no pid")?);
        let input = child.stdin.take().context("MCP stdin missing")?;
        let output = BufReader::new(child.stdout.take().context("MCP stdout missing")?);
        drop(lifecycle);
        let (requests, receiver) = mpsc::channel(1);
        tokio::spawn(async move {
            let _permit = permit;
            let _descriptors = (executable, profile_reader);
            run(invocation, child, group, input, output, receiver).await;
        });
        Ok(McpProcess { requests })
    }
}

fn stamp(metadata: &std::fs::Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}

async fn run(
    invocation: McpInvocation,
    mut child: Child,
    group: PluginProcessGroup,
    mut input: ChildStdin,
    mut output: BufReader<ChildStdout>,
    mut requests: mpsc::Receiver<Request>,
) {
    loop {
        let request = tokio::select! {
            biased;
            () = invocation.ended() => break,
            request = requests.recv() => match request { Some(request) => request, None => break },
        };
        let Request {
            message,
            mut reply,
            deadline,
        } = request;
        let result = tokio::select! {
            biased;
            () = invocation.ended() => break,
            () = reply.closed() => break,
            () = tokio::time::sleep_until(deadline) => Err(anyhow::anyhow!("MCP request timed out")),
            result = exchange(&mut input, &mut output, &message, &invocation.profile().binding.tools) => result,
        };
        if invocation.check().is_err() {
            break;
        }
        let failed = result.is_err();
        if failed {
            requests.close();
        }
        let _ = reply.send(result);
        if failed {
            break;
        }
    }
    requests.close();
    drop(group);
    let _ = child.wait().await;
}

async fn exchange(
    input: &mut ChildStdin,
    output: &mut BufReader<ChildStdout>,
    message: &Value,
    tools: &[String],
) -> Result<Value> {
    let method = message["method"].as_str().context("missing MCP method")?;
    ensure!(message["jsonrpc"] == "2.0", "invalid MCP version");
    ensure!(
        matches!(
            method,
            "initialize" | "notifications/initialized" | "ping" | "tools/list" | "tools/call"
        ),
        "unsupported MCP method"
    );
    if method == "tools/call" {
        ensure!(
            message["params"]["name"]
                .as_str()
                .is_some_and(|name| tools.iter().any(|tool| tool == name)),
            "MCP tool not admitted"
        );
    }
    let id = &message["id"];
    ensure!(
        method == "notifications/initialized" || id.is_string() || id.is_i64() || id.is_u64(),
        "invalid MCP request id"
    );
    let bytes = serde_json::to_vec(message)?;
    ensure!(bytes.len() < MAX_FRAME, "MCP input too large");
    input.write_all(&bytes).await?;
    input.write_all(b"\n").await?;
    input.flush().await?;
    if method == "notifications/initialized" {
        return Ok(Value::Null);
    }
    let mut frame = Vec::new();
    let count = output
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut frame)
        .await?;
    ensure!(
        count <= MAX_FRAME && frame.last() == Some(&b'\n'),
        "MCP output frame invalid or too large"
    );
    let mut response: Value = serde_json::from_slice(&frame)?;
    ensure!(
        response["jsonrpc"] == "2.0"
            && response["id"] == *id
            && response.get("method").is_none()
            && (response.get("result").is_some() != response.get("error").is_some()),
        "unexpected MCP response"
    );
    if let Some(result) = response.get_mut("result") {
        let result = result.as_object_mut().context("invalid MCP result")?;
        if method == "tools/list" {
            result
                .get_mut("tools")
                .and_then(Value::as_array_mut)
                .context("invalid MCP tools list")?
                .retain(|tool| {
                    tool["name"]
                        .as_str()
                        .is_some_and(|name| tools.iter().any(|allowed| allowed == name))
                });
        }
        if method == "initialize" {
            result.insert("capabilities".into(), json!({"tools":{}}));
        }
    }
    Ok(response)
}
