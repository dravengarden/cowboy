//! Contained JavaScript execution with a host-owned SDK and authorization boundary.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::time::Instant;
const MAX_CALLS: usize = 16;
const MAX_REPLY_BYTES: usize = 256 * 1024;
const MAX_BROKER_BYTES: usize = 1024 * 1024;
fn error(code: &str) -> Value {
    json!({"schema":1,"state":"error","error":{"code":code}})
}

/// Shared broker traffic budget for one CodeAct program execution.
/// Lets side-effecting reads (such as `calls.result`) reserve exact wire bytes
/// against both `MAX_REPLY_BYTES` and the remaining aggregate `MAX_BROKER_BYTES`
/// before committing durable consumption.
#[derive(Debug)]
pub struct ReplyBudget {
    remaining: AtomicUsize,
    closed: AtomicBool,
}

impl ReplyBudget {
    #[must_use]
    pub fn new(total_bytes: usize) -> Self {
        Self {
            remaining: AtomicUsize::new(total_bytes),
            closed: AtomicBool::new(false),
        }
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn charge(&self, bytes: usize) -> bool {
        if self.closed.load(Ordering::Acquire) {
            return false;
        }
        let mut cur = self.remaining.load(Ordering::Acquire);
        loop {
            if self.closed.load(Ordering::Acquire) {
                return false;
            }
            if cur < bytes {
                return false;
            }
            match self.remaining.compare_exchange_weak(
                cur,
                cur - bytes,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(actual) => cur = actual,
            }
        }
    }

    fn refund(&self, bytes: usize) {
        if bytes > 0 {
            self.remaining.fetch_add(bytes, Ordering::AcqRel);
        }
    }
}

/// Per-call handle into the program's shared [`ReplyBudget`].
pub struct CallBudget<'a> {
    budget: &'a ReplyBudget,
    reserved: AtomicUsize,
}

impl<'a> CallBudget<'a> {
    #[must_use]
    pub fn new(budget: &'a ReplyBudget) -> Self {
        Self {
            budget,
            reserved: AtomicUsize::new(0),
        }
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.budget.is_closed()
    }

    /// Reserve `wire_bytes` (including JSON wrapper + trailing newline) against
    /// both `MAX_REPLY_BYTES` and the remaining aggregate `MAX_BROKER_BYTES`.
    /// Dropping the returned guard without calling [`ReplyReservation::commit`]
    /// refunds the reserved bytes.
    pub fn reserve(&self, wire_bytes: usize) -> Result<ReplyReservation<'_>, &'static str> {
        if wire_bytes.saturating_sub(1) > MAX_REPLY_BYTES {
            return Err("reply_too_large");
        }
        if !self.budget.charge(wire_bytes) {
            return Err("budget_exceeded");
        }
        Ok(ReplyReservation {
            call: self,
            bytes: wire_bytes,
            committed: false,
        })
    }

    fn take_reserved(&self) -> usize {
        self.reserved.swap(0, Ordering::AcqRel)
    }
}

pub struct ReplyReservation<'a> {
    call: &'a CallBudget<'a>,
    bytes: usize,
    committed: bool,
}

impl ReplyReservation<'_> {
    pub fn commit(mut self) {
        self.call.reserved.fetch_add(self.bytes, Ordering::AcqRel);
        self.committed = true;
    }
}

impl Drop for ReplyReservation<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.call.budget.refund(self.bytes);
        }
    }
}

pub struct Sandbox {
    pub prlimit: PathBuf,
    pub bwrap: PathBuf,
    pub bun: PathBuf,
    pub runtime_directory: Option<PathBuf>,
    pub bun_loader: Option<PathBuf>,
}
pub trait Host: Sync {
    fn call(
        &self,
        op: &str,
        args: Value,
        remaining: Duration,
    ) -> impl std::future::Future<Output = Result<Value, String>> + Send;
    fn call_with_budget(
        &self,
        op: &str,
        args: Value,
        remaining: Duration,
        _budget: &CallBudget<'_>,
    ) -> impl std::future::Future<Output = Result<Value, String>> + Send {
        self.call(op, args, remaining)
    }
    /// Re-verify final authorization immediately before the broker writes an
    /// accepted reply to the sandbox, without committing side effects yet.
    /// Never invoked during post-exit cleanup after a budget refusal or protocol error.
    fn authorize_before_relay(
        &self,
        _op: &str,
        _args: &Value,
        _reply: &Value,
    ) -> impl std::future::Future<Output = Result<(), String>> + Send {
        std::future::ready(Ok(()))
    }

    /// Commit staged side effects (such as marking a completion notice consumed
    /// on `calls.result`) ONLY AFTER final authorization passed AND the reply
    /// bytes were successfully written and flushed to the sandbox.
    fn commit_after_relay(
        &self,
        _op: &str,
        _args: &Value,
        _reply: &Value,
    ) -> impl std::future::Future<Output = ()> + Send {
        std::future::ready(())
    }
    fn finish(&self, done: bool, frame: Value) -> Value;
    fn call_error(&self, op: &str, echo: &str, code: String) -> Value;
}

impl Sandbox {
    pub async fn execute(
        &self,
        source: &str,
        typescript: bool,
        timeout_ms: u64,
        runner: &str,
        host: &impl Host,
    ) -> Value {
        if let Some(root) = &self.runtime_directory {
            let valid = std::fs::canonicalize(root)
                .ok()
                .zip(std::fs::canonicalize(&self.bun).ok())
                .is_some_and(|(root, bun)| {
                    root.is_dir() && root.parent().is_some() && bun.starts_with(root)
                });
            if !valid {
                return error("runtime_unavailable");
            }
        }
        if let Some(loader) = &self.bun_loader {
            let valid = self
                .runtime_directory
                .as_ref()
                .and_then(|root| std::fs::canonicalize(root).ok())
                .zip(std::fs::canonicalize(loader).ok())
                .is_some_and(|(root, loader)| loader.is_file() && loader.starts_with(root));
            if !valid {
                return error("runtime_unavailable");
            }
        }
        let Ok(directory) = private_directory() else {
            return error("runtime_unavailable");
        };
        let result = self
            .run(&directory.0, source, typescript, timeout_ms, runner, host)
            .await;
        drop(directory);
        result
    }
    fn runtime_path(&self, path: &Path) -> PathBuf {
        self.runtime_directory
            .as_ref()
            .and_then(|root| std::fs::canonicalize(root).ok())
            .zip(std::fs::canonicalize(path).ok())
            .and_then(|(root, path)| {
                path.strip_prefix(root)
                    .ok()
                    .map(|relative| Path::new("/runtime").join(relative))
            })
            .unwrap_or_else(|| path.to_owned())
    }

    fn command(&self, directory: &Path, entry: &str) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.prlimit);
        command
            .args([
                "--cpu=4",
                "--data=536870912",
                "--nofile=64",
                "--fsize=1048576",
                "--core=0",
                "--",
            ])
            .arg(&self.bwrap)
            .args([
                "--unshare-all",
                "--die-with-parent",
                "--new-session",
                "--cap-drop",
                "ALL",
                "--clearenv",
                "--setenv",
                "HOME",
                "/tmp",
                "--setenv",
                "BUN_RUNTIME_TRANSPILER_CACHE_PATH",
                "0",
            ]);
        if self.bun_loader.is_none() {
            for path in ["/nix/store", "/usr", "/lib", "/lib64"] {
                command.args(["--ro-bind-try", path, path]);
            }
        }
        if let Some(root) = &self.runtime_directory {
            command.arg("--ro-bind").arg(root).arg("/runtime");
        }
        command.arg("--ro-bind").arg(directory).args([
            "/sandbox",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            // tmpfs pages are host memory outside every process limit:
            // /dev stays read-only and /tmp holds at most 1 MiB.
            "--remount-ro",
            "/dev",
            "--size",
            "1048576",
            "--tmpfs",
            "/tmp",
            "--chdir",
            "/sandbox",
            "--",
        ]);
        if let Some(loader) = &self.bun_loader {
            command
                .arg(self.runtime_path(loader))
                .args(["--inhibit-cache", "--library-path"])
                .arg(
                    self.runtime_path(loader)
                        .parent()
                        .expect("validated loader")
                        .join("lib"),
                );
        }
        command
            .arg(self.runtime_path(&self.bun))
            .args(["--no-install", "--smol", "run", "/sandbox/runner.js"])
            .arg(entry)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }
    async fn broker(
        stdout: tokio::process::ChildStdout,
        mut stdin: tokio::process::ChildStdin,
        deadline: Instant,
        host: &impl Host,
    ) -> Option<Value> {
        let mut reader = BufReader::new(stdout);
        let mut calls = 0usize;
        let budget = ReplyBudget::new(MAX_BROKER_BYTES);
        use futures::{StreamExt as _, stream::FuturesUnordered};
        let mut pending = FuturesUnordered::new();
        let mut line = Vec::new();
        let outcome = loop {
            let remaining_bytes = MAX_REPLY_BYTES + 1 - line.len();
            if remaining_bytes == 0 {
                break Some(error("budget_exceeded"));
            }
            let mut limited = (&mut reader).take(remaining_bytes as u64);
            tokio::select! {
                reply = pending.next(), if !pending.is_empty() => {
                    let (id, op, args, reply, reserved): (u64, String, Value, Value, usize) =
                        reply.expect("nonempty pending calls");
                    let mut bytes = serde_json::to_vec(&reply).unwrap_or_default();
                    let oversized = bytes.len() > MAX_REPLY_BYTES;
                    if oversized {
                        bytes = serde_json::to_vec(&json!({"id":id,"error":{"code":"reply_too_large"}})).unwrap_or_default();
                    }
                    let mut held = reserved;
                    let mut authorized_ok = false;
                    if !oversized && reply.get("ok").is_some() {
                        let needed = bytes.len() + 1;
                        if needed > held {
                            if !budget.charge(needed - held) {
                                budget.close();
                                break Some(error("budget_exceeded"));
                            }
                            held = needed;
                        }
                        match host.authorize_before_relay(&op, &args, &reply).await {
                            Ok(()) => {
                                authorized_ok = true;
                            }
                            Err(code) => {
                                let echo = args.to_string();
                                let err = json!({"id":id,"error":host.call_error(&op, &echo, code)});
                                bytes = serde_json::to_vec(&err).unwrap_or_default();
                            }
                        }
                    }
                    bytes.push(b'\n');
                    if bytes.len() <= held {
                        budget.refund(held - bytes.len());
                    } else if !budget.charge(bytes.len() - held) {
                        budget.close();
                        break Some(error("budget_exceeded"));
                    }
                    if stdin.write_all(&bytes).await.is_err() || stdin.flush().await.is_err() {
                        budget.close();
                        break None;
                    }
                    if authorized_ok {
                        host.commit_after_relay(&op, &args, &reply).await;
                    }
                }
                read = limited.read_until(b'\n', &mut line), if pending.len() < 4 => {
                    match read {
                        Ok(0) | Err(_) => break None,
                        Ok(_) if line.len() > MAX_REPLY_BYTES => {
                            budget.close();
                            break Some(error("budget_exceeded"));
                        }
                        Ok(_) if !line.ends_with(b"\n") => break None,
                        Ok(_) => {
                            if !budget.charge(line.len()) {
                                budget.close();
                                break Some(error("budget_exceeded"));
                            }
                        }
                    }
                    let Ok(frame) = serde_json::from_slice::<Value>(&line) else { break Some(error("runtime_protocol")); };
                    line.clear();
                    if let Some(done) = frame.get("done").and_then(Value::as_bool) { break Some(host.finish(done, frame)); }
                    let (Some(id), Some(op)) = (frame.get("id").and_then(Value::as_u64),frame.get("op").and_then(Value::as_str)) else { break Some(error("runtime_protocol")); };
                    calls += 1;
                    let permitted = calls <= MAX_CALLS;
                    let op = op.to_owned();
                    let args = frame.get("args").cloned().unwrap_or_else(||json!({}));
                    let budget_ref = &budget;
                    pending.push(async move {
                        if !permitted { return (id, op, args, json!({"id":id,"error":{"code":"call_limit"}}), 0); }
                        let echo = args.to_string();
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        let call_budget = CallBudget::new(budget_ref);
                        let reply = match host.call_with_budget(&op, args.clone(), remaining, &call_budget).await {
                            Ok(value) => json!({"id":id,"ok":value}),
                            Err(code) => json!({"id":id,"error":host.call_error(&op,&echo,code)}),
                        };
                        (id, op, args, reply, call_budget.take_reserved())
                    });
                }
            }
        };
        // Stop relaying further replies before draining in-flight calls so
        // side-effecting reads cannot reserve leftover budget and commit after
        // the broker stopped relaying.
        budget.close();
        // Finish admitted calls before returning; the enclosing deadline still bounds cleanup.
        while pending.next().await.is_some() {}
        outcome
    }
    async fn run(
        &self,
        directory: &Path,
        source: &str,
        typescript: bool,
        timeout_ms: u64,
        runner: &str,
        host: &impl Host,
    ) -> Value {
        let extension = if typescript { "ts" } else { "js" };
        let entry = format!("/sandbox/program.{extension}");
        if std::fs::write(directory.join("runner.js"), runner).is_err()
            || std::fs::write(directory.join(format!("program.{extension}")), source).is_err()
        {
            return error("runtime_unavailable");
        }
        let mut command = self.command(directory, &entry);
        let Ok(mut child) = command.spawn() else {
            return error("runtime_unavailable");
        };
        let (Some(stdin), Some(stdout), Some(mut stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return error("runtime_unavailable");
        };
        let diagnostics = tokio::spawn(async move {
            let mut bytes = Vec::new();
            let _ = (&mut stderr).take(8192).read_to_end(&mut bytes).await;
            // Keep draining so the runtime never blocks on a full pipe.
            let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
            bytes
        });
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let outcome =
            tokio::time::timeout_at(deadline, Self::broker(stdout, stdin, deadline, host)).await;
        let _ = child.start_kill();
        let _ = child.wait().await;
        match outcome {
            Err(_) => error("timeout"),
            Ok(Some(value)) => value,
            Ok(None) => {
                let stderr = diagnostics.await.unwrap_or_default();
                let mut value = error("runtime_failed");
                let text = String::from_utf8_lossy(&stderr);
                if !text.trim().is_empty() {
                    value["error"]["message"] = json!(text.chars().take(1000).collect::<String>());
                }
                value
            }
        }
    }
}
struct PrivateDirectory(PathBuf);
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn private_directory() -> std::io::Result<PrivateDirectory> {
    use std::os::unix::fs::DirBuilderExt as _;
    let mut entropy = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut entropy);
    let name: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
    let directory = std::env::temp_dir().join(format!("cowboy-agent-{name}"));
    std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
    Ok(PrivateDirectory(directory))
}
