//! Controller side of the Agent API (`docs/agent-api.md`).
//!
//! An action arrives through the managed-call gateway after the coordinator
//! authorized the parent, so every operation here acts for exactly that
//! session and its owner. A read program runs in a fresh contained runtime
//! (Bubblewrap namespaces without network, a read-only system view, resource
//! limits and a wall-clock deadline); it reaches Cowboy only through the
//! line-delimited broker below, which validates every operation and bounds
//! every reply. There is no unsandboxed fallback.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::Semaphore;
use tokio::time::Instant;

use super::AppState;
use crate::agent_api::{
    API_VERSION, MAX_BROKER_BYTES, MAX_CALLS, MAX_LOG_BYTES, MAX_REPLY_BYTES, MAX_RESULT_BYTES,
};
use crate::core::{Envelope, Event, QueuedMessage, SessionInfo, SessionMeta};
use crate::managed_calls::protocol::Action;
use crate::managed_calls::service::Ledger;

const RUNNER: &str = include_str!("agent_api/runner.js");
mod actions;
pub(super) use actions::invoke;
/// Concurrent programs across the Controller; excess is refused, not queued.
static SLOTS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(2));
/// Raw hot-log events one `session.events` call may coalesce.
const EVENT_WINDOW: usize = 2000;

pub(super) struct Program {
    pub code: String,
    pub typescript: bool,
    pub module: bool,
    pub timeout_ms: u64,
}

fn error(code: &str) -> Value {
    json!({"schema":1,"state":"error","error":{"code":code}})
}

/// The compact status: no program, no runtime start.
pub(super) async fn status(state: &Arc<AppState>, meta: &SessionMeta, _ledger: &Ledger) -> Value {
    let Some(info) = state.hub.session_info(&meta.id) else {
        return error("not_found");
    };
    json!({
        "schema": 1,
        "state": "ok",
        "api": API_VERSION,
        "actions": state.machine_control.connection_protocol(&meta.machine_id)
            .is_some_and(|(_, protocol)| protocol >= crate::machine_protocol::AGENT_ACTION_PROTOCOL_VERSION),
        "session": session_view(&info),
        "calls": {"enabled": super::agent_tools::effective(state, meta).calls.enabled},
    })
}

pub(super) async fn read(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    ledger: &Ledger,
    program: Program,
) -> Value {
    let Ok(_slot) = SLOTS.try_acquire() else {
        return error("capacity");
    };
    let sandbox = match Sandbox::resolve() {
        Ok(sandbox) => sandbox,
        Err(code) => return error(code),
    };
    let host = SessionHost {
        state,
        meta,
        ledger,
    };
    sandbox.execute(&program, &host).await
}

// ---------------------------------------------------------------------------
// Operations

trait Host: Sync {
    fn call(
        &self,
        op: &str,
        args: Value,
        remaining: Duration,
    ) -> impl std::future::Future<Output = Result<Value, String>> + Send;
}

struct SessionHost<'a> {
    state: &'a Arc<AppState>,
    meta: &'a SessionMeta,
    ledger: &'a Ledger,
}

fn arg_str<'v>(args: &'v Value, key: &str) -> Result<&'v str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| crate::managed_calls::valid_id(value))
        .ok_or_else(|| "invalid_argument".to_owned())
}

fn arg_u64(args: &Value, key: &str, default: u64, max: u64) -> Result<u64, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_u64()
            .filter(|value| *value <= max)
            .ok_or_else(|| "invalid_argument".to_owned()),
    }
}

fn only_keys(args: &Value, allowed: &[&str]) -> Result<(), String> {
    match args {
        Value::Object(map) if map.keys().all(|key| allowed.contains(&key.as_str())) => Ok(()),
        _ => Err("invalid_argument".to_owned()),
    }
}

impl Host for SessionHost<'_> {
    async fn call(&self, op: &str, args: Value, remaining: Duration) -> Result<Value, String> {
        match op {
            "session.info" => {
                only_keys(&args, &[])?;
                let info = self
                    .state
                    .hub
                    .session_info(&self.meta.id)
                    .ok_or("not_found")?;
                Ok(session_view(&info))
            }
            "session.queue" | "session.drafts" => {
                only_keys(&args, &[])?;
                let (queue, drafts) = self
                    .state
                    .hub
                    .queue_and_drafts(&self.meta.id)
                    .ok_or("not_found")?;
                let items = if op == "session.queue" { queue } else { drafts };
                Ok(Value::Array(items.iter().map(message_view).collect()))
            }
            "session.events" => {
                only_keys(&args, &["after", "before", "limit", "chars", "kinds"])?;
                let query = EventQuery::parse(&args)?;
                let (window, reached_start) = self
                    .state
                    .hub
                    .log_window(&self.meta.id, query.after, query.before, EVENT_WINDOW)
                    .ok_or("not_found")?;
                Ok(query.page(&window, reached_start))
            }
            "calls.capabilities" => {
                only_keys(&args, &[])?;
                self.calls(Action::Capabilities {}).await
            }
            "calls.inspect" => {
                only_keys(&args, &["call_id"])?;
                let call_id = arg_str(&args, "call_id")?.to_owned();
                self.calls(Action::Inspect { call_id }).await
            }
            "calls.result" => {
                only_keys(&args, &["call_id", "offset", "chars"])?;
                let call_id = arg_str(&args, "call_id")?.to_owned();
                let mut value = self.calls(Action::Result { call_id }).await?;
                text_window(&mut value["result"], "text", &args)?;
                Ok(value)
            }
            "calls.wait" => {
                only_keys(&args, &["call_id", "timeout_ms"])?;
                let call_id = arg_str(&args, "call_id")?.to_owned();
                let requested = arg_u64(
                    &args,
                    "timeout_ms",
                    30_000,
                    crate::managed_calls::protocol::MAX_WAIT_MS,
                )?;
                // Leave the program time to use what it waited for.
                let budget = remaining.saturating_sub(Duration::from_millis(1000));
                let timeout_ms = requested.min(u64::try_from(budget.as_millis()).unwrap_or(0));
                self.calls(Action::Wait {
                    call_id,
                    timeout_ms,
                })
                .await
            }
            "calls.observe" => {
                only_keys(&args, &["request_id"])?;
                let request_id = arg_str(&args, "request_id")?.to_owned();
                self.calls(Action::Observe { request_id }).await
            }
            "sessions.list" => {
                only_keys(&args, &["status", "limit"])?;
                let status = match args.get("status") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(status)) if status.len() <= 32 => Some(status.clone()),
                    Some(_) => return Err("invalid_argument".into()),
                };
                let limit = usize::try_from(arg_u64(&args, "limit", 50, 200)?).unwrap_or(200);
                let owner = self.meta.owner_user_id.clone();
                let rows = self
                    .state
                    .hub
                    .session_list_filtered(|row| row == owner.as_deref())
                    .into_iter()
                    .filter(|meta| !meta.system && !meta.closing)
                    .filter_map(|meta| self.state.hub.session_info(&meta.id))
                    .map(|info| session_view(&info))
                    .filter(|view| {
                        status
                            .as_deref()
                            .is_none_or(|status| view["status"] == status)
                    })
                    .take(limit)
                    .collect();
                Ok(Value::Array(rows))
            }
            "sessions.info" => {
                only_keys(&args, &["id"])?;
                let id = arg_str(&args, "id")?;
                let info = self
                    .state
                    .hub
                    .session_info(id)
                    .filter(|info| {
                        !info.meta.system && info.meta.owner_user_id == self.meta.owner_user_id
                    })
                    .ok_or("not_found")?;
                Ok(session_view(&info))
            }
            "documents.list" | "documents.read" => {
                let owner = self.meta.owner_user_id.as_deref().ok_or("unavailable")?;
                let store = self.state.store.as_ref().ok_or("unavailable")?;
                if op == "documents.list" {
                    only_keys(&args, &["after", "limit", "include_deleted"])?;
                    let after = args
                        .get("after")
                        .map(|_| arg_str(&args, "after"))
                        .transpose()?;
                    let deleted = optional_bool(&args, "include_deleted")?;
                    let limit =
                        usize::try_from(arg_u64(&args, "limit", 10_000, 200)?).unwrap_or(200);
                    let mut documents = store
                        .draft_documents(owner)
                        .await
                        .map_err(|_| "unavailable")?;
                    documents.sort_by(|a, b| a.id.cmp(&b.id));
                    Ok(Value::Array(
                        documents
                            .iter()
                            .filter(|document| deleted || !document.deleted)
                            .filter(|document| {
                                after.is_none_or(|after| document.id.as_str() > after)
                            })
                            .take(limit)
                            .map(|document| document_view(document, false))
                            .collect(),
                    ))
                } else {
                    only_keys(&args, &["id", "offset", "chars", "include_deleted"])?;
                    let id = arg_str(&args, "id")?;
                    let deleted = optional_bool(&args, "include_deleted")?;
                    let document = store
                        .draft_document(owner, id)
                        .await
                        .map_err(|_| "unavailable")?
                        .filter(|document| deleted || !document.deleted)
                        .ok_or("not_found")?;
                    let mut value = document_view(&document, true);
                    text_window(&mut value, "body", &args)?;
                    Ok(value)
                }
            }
            _ => Err("unknown_operation".into()),
        }
    }
}

impl SessionHost<'_> {
    /// Call observations share the gateway's exact contract.
    async fn calls(&self, action: Action) -> Result<Value, String> {
        let mut value =
            super::managed_calls::dispatch_calls(self.state, self.meta, self.ledger, action).await;
        if value.get("state").and_then(Value::as_str) == Some("error")
            && value.get("call_id").is_none()
        {
            let code = value
                .pointer("/error/code")
                .and_then(Value::as_str)
                .unwrap_or("unavailable");
            return Err(code.to_owned());
        }
        if let Some(object) = value.as_object_mut() {
            // CLI-only fields: the program needs no argv or wire schema.
            object.remove("schema");
            object.remove("next");
        }
        Ok(value)
    }
}

fn session_view(info: &SessionInfo) -> Value {
    let meta = &info.meta;
    json!({
        "id": meta.id,
        "title": meta.title,
        "provider": meta.provider,
        "status": meta.status,
        "machine": meta.machine_id,
        "workspace": meta.workspace_name.as_ref().or(meta.workspace_id.as_ref()),
        "cwd": meta.cwd,
        "paused": meta.paused,
        "queued": info.queue_count,
        "drafts": info.drafts_count,
        "events": info.event_count,
        "background_tasks": meta.background_tasks,
        "context": (meta.context_size > 0)
            .then(|| json!({"used": meta.context_used, "size": meta.context_size})),
    })
}

fn message_view(message: &QueuedMessage) -> Value {
    let mut view = json!({
        "id": message.id,
        "text": message.text,
        "attachments": message.content.iter()
            .filter(|block| block.get("type").and_then(Value::as_str) != Some("text"))
            .count(),
    });
    if message.text.is_empty() {
        view["text"] = json!(content_text(&message.content));
    }
    if let Some(schedule) = &message.schedule {
        view["scheduled_at_ms"] = json!(schedule.fire_at_ms);
    }
    view
}

fn document_view(document: &crate::store::draft_documents::DraftDocument, body: bool) -> Value {
    let mut view = serde_json::to_value(document).unwrap_or(Value::Null);
    if let Some(object) = view.as_object_mut() {
        let attachments = object
            .remove("attachments")
            .and_then(|value| value.as_array().map(Vec::len))
            .unwrap_or(0);
        if body {
            object.insert("attachments".into(), json!(attachments));
        } else {
            object.remove("body");
        }
    }
    view
}

fn optional_bool(args: &Value, key: &str) -> Result<bool, String> {
    match args.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err("invalid_argument".into()),
    }
}

/// Unicode scalar offsets, never byte slices. Explicit windows let a program
/// read a large retained value without first exceeding the broker's budget.
fn text_window(value: &mut Value, field: &str, args: &Value) -> Result<(), String> {
    if args.get("offset").is_none() && args.get("chars").is_none() {
        return Ok(());
    }
    let offset = usize::try_from(arg_u64(args, "offset", 0, u64::from(u32::MAX))?)
        .map_err(|_| "invalid_argument")?;
    let count =
        usize::try_from(arg_u64(args, "chars", 1000, 4000)?).map_err(|_| "invalid_argument")?;
    if count == 0 {
        return Err("invalid_argument".into());
    }
    if let Some(text) = value[field].as_str() {
        let total = text.chars().count();
        let offset = offset.min(total);
        let body: String = text.chars().skip(offset).take(count).collect();
        let end = offset + body.chars().count();
        value[field] = json!(body);
        value["range"] = json!({"offset":offset,"next":(end<total).then_some(end),"total":total});
    }
    Ok(())
}

fn content_text(content: &[Value]) -> String {
    content
        .iter()
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Transcript projection: chunks coalesce into messages, tool updates into
// their call, thoughts are omitted.

const EVENT_KINDS: &[&str] = &[
    "user",
    "agent",
    "tool",
    "plan",
    "permission",
    "turn_end",
    "lifecycle",
];

struct EventQuery {
    after: Option<u64>,
    before: Option<u64>,
    limit: usize,
    chars: usize,
    kinds: Vec<String>,
}

impl EventQuery {
    fn parse(args: &Value) -> Result<Self, String> {
        let optional = |key: &str| match args.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_u64()
                .map(Some)
                .ok_or_else(|| "invalid_argument".to_owned()),
        };
        let kinds = match args.get("kinds") {
            None | Some(Value::Null) => EVENT_KINDS
                .iter()
                .filter(|kind| **kind != "lifecycle")
                .map(|kind| (*kind).to_owned())
                .collect(),
            Some(Value::Array(kinds)) => kinds
                .iter()
                .map(|kind| {
                    kind.as_str()
                        .filter(|kind| EVENT_KINDS.contains(kind))
                        .map(str::to_owned)
                        .ok_or_else(|| "invalid_argument".to_owned())
                })
                .collect::<Result<_, _>>()?,
            Some(_) => return Err("invalid_argument".into()),
        };
        Ok(Self {
            after: optional("after")?,
            before: optional("before")?,
            limit: usize::try_from(arg_u64(args, "limit", 20, 100)?).unwrap_or(100),
            chars: usize::try_from(arg_u64(args, "chars", 400, 4000)?).unwrap_or(4000),
            kinds,
        })
    }

    fn page(&self, window: &[Envelope], reached_start: bool) -> Value {
        let mut items: Vec<Item> = coalesce(window)
            .into_iter()
            .filter(|item| self.kinds.iter().any(|kind| kind == item.kind))
            .collect();
        let more = if self.after.is_some() {
            let more = items.len() > self.limit || window.len() == EVENT_WINDOW;
            items.truncate(self.limit);
            more
        } else {
            let more = items.len() > self.limit || !reached_start;
            items.drain(..items.len().saturating_sub(self.limit));
            more
        };
        json!({
            "items": items.iter().map(|item| item.view(self.chars)).collect::<Vec<_>>(),
            "reached_start": reached_start && !more,
            "more": more,
        })
    }
}

struct Item {
    seq: u64,
    to: u64,
    kind: &'static str,
    text: String,
    tool: Option<Map<String, Value>>,
}

impl Item {
    fn view(&self, chars: usize) -> Value {
        let mut view = json!({"seq": self.seq, "to": self.to, "kind": self.kind});
        if !self.text.is_empty() {
            let mut text: String = self.text.chars().take(chars).collect();
            if text.len() < self.text.len() {
                view["truncated"] = json!(true);
            } else {
                text = self.text.clone();
            }
            view["text"] = json!(text);
        }
        if let Some(tool) = &self.tool {
            view["tool"] = Value::Object(tool.clone());
        }
        view
    }
}

fn block_text(block: &Value) -> String {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => block
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        Some(other) => format!("[{other}]"),
        None => String::new(),
    }
}

fn coalesce(window: &[Envelope]) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut tools: HashMap<String, usize> = HashMap::new();
    for envelope in window {
        let seq = envelope.seq;
        let (kind, text, tool): (&'static str, String, Option<Map<String, Value>>) = match &envelope
            .event
        {
            Event::Update { update } => match update.get("sessionUpdate").and_then(Value::as_str) {
                Some("user_message_chunk") => ("user", block_text(&update["content"]), None),
                Some("agent_message_chunk") => ("agent", block_text(&update["content"]), None),
                Some(tag @ ("tool_call" | "tool_call_update")) => {
                    let id = update
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let mut fields = Map::new();
                    for (from, to) in [("title", "title"), ("status", "status"), ("kind", "kind")] {
                        if let Some(value) = update.get(from).filter(|v| !v.is_null()) {
                            fields.insert(to.into(), value.clone());
                        }
                    }
                    if let Some(index) = tools.get(&id).copied() {
                        let item = &mut items[index];
                        item.to = seq;
                        if let Some(tool) = &mut item.tool {
                            tool.extend(fields);
                        }
                        continue;
                    }
                    if tag == "tool_call_update" && id.is_empty() {
                        continue;
                    }
                    fields.insert("id".into(), json!(id));
                    tools.insert(id, items.len());
                    ("tool", String::new(), Some(fields))
                }
                Some("plan") => {
                    let text = update
                        .get("entries")
                        .and_then(Value::as_array)
                        .map(|entries| {
                            entries
                                .iter()
                                .map(|entry| {
                                    format!(
                                        "[{}] {}",
                                        entry["status"].as_str().unwrap_or("?"),
                                        entry["content"].as_str().unwrap_or_default()
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    ("plan", text, None)
                }
                _ => continue,
            },
            Event::PermissionRequest { tool_call, .. } => (
                "permission",
                tool_call["title"].as_str().unwrap_or_default().to_owned(),
                None,
            ),
            Event::Lifecycle { status, detail } => (
                "lifecycle",
                format!(
                    "{}{}",
                    serde_json::to_value(status)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default(),
                    detail
                        .as_ref()
                        .map(|d| format!(": {d}"))
                        .unwrap_or_default()
                ),
                None,
            ),
            Event::TurnEnd { stop_reason } => ("turn_end", stop_reason.clone(), None),
            Event::PermissionResolved { .. } => continue,
        };
        if matches!(kind, "user" | "agent")
            && let Some(last) = items.last_mut()
            && last.kind == kind
        {
            last.text.push_str(&text);
            last.to = seq;
            continue;
        }
        items.push(Item {
            seq,
            to: seq,
            kind,
            text,
            tool,
        });
    }
    items
}

// ---------------------------------------------------------------------------
// Contained runtime

struct Sandbox {
    prlimit: PathBuf,
    bwrap: PathBuf,
    bun: PathBuf,
}

/// A runtime override, then the path the Nix build pinned, then PATH.
fn executable(variable: &str, pinned: Option<&str>, name: &str) -> Option<PathBuf> {
    let found = std::env::var_os(variable)
        .map(PathBuf::from)
        .or_else(|| pinned.map(PathBuf::from))
        .or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH")?)
                .map(|directory| directory.join(name))
                .find(|path| path.is_file())
        })?;
    std::fs::canonicalize(found).ok()
}

impl Sandbox {
    fn resolve() -> Result<Self, &'static str> {
        Ok(Self {
            prlimit: executable(
                "COWBOY_AGENT_SANDBOX_PRLIMIT",
                option_env!("COWBOY_AGENT_SANDBOX_PRLIMIT"),
                "prlimit",
            )
            .ok_or("runtime_unavailable")?,
            bwrap: executable(
                "COWBOY_AGENT_SANDBOX_BWRAP",
                option_env!("COWBOY_AGENT_SANDBOX_BWRAP"),
                "bwrap",
            )
            .ok_or("runtime_unavailable")?,
            bun: executable(
                "COWBOY_AGENT_SANDBOX_BUN",
                option_env!("COWBOY_AGENT_SANDBOX_BUN"),
                "bun",
            )
            .ok_or("runtime_unavailable")?,
        })
    }

    async fn execute(&self, program: &Program, host: &impl Host) -> Value {
        let Ok(directory) = private_directory() else {
            return error("runtime_unavailable");
        };
        let result = self.run(&directory, program, host).await;
        let _ = std::fs::remove_dir_all(&directory);
        result
    }

    async fn run(&self, directory: &Path, program: &Program, host: &impl Host) -> Value {
        let extension = if program.typescript { "ts" } else { "js" };
        let source = if program.module {
            program.code.clone()
        } else {
            format!(
                "export default async function ({{ cowboy }}) {{\n{}\n}}\n",
                program.code
            )
        };
        let entry = format!("/sandbox/program.{extension}");
        if std::fs::write(directory.join("runner.js"), RUNNER).is_err()
            || std::fs::write(directory.join(format!("program.{extension}")), source).is_err()
        {
            return error("runtime_unavailable");
        }
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
                "--ro-bind-try",
                "/nix/store",
                "/nix/store",
                "--ro-bind-try",
                "/usr",
                "/usr",
                "--ro-bind-try",
                "/lib",
                "/lib",
                "--ro-bind-try",
                "/lib64",
                "/lib64",
                "--ro-bind",
            ])
            .arg(directory)
            .args([
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
            ])
            .arg(&self.bun)
            .args(["--no-install", "--smol", "run", "/sandbox/runner.js"])
            .arg(&entry)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let Ok(mut child) = command.spawn() else {
            return error("runtime_unavailable");
        };
        let (Some(mut stdin), Some(stdout), Some(mut stderr)) =
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
        let deadline = Instant::now() + Duration::from_millis(program.timeout_ms);
        let broker = async {
            let mut reader = BufReader::new(stdout);
            let mut calls = 0usize;
            let mut traffic = 0usize;
            loop {
                let mut line = Vec::new();
                let read = (&mut reader)
                    .take(MAX_REPLY_BYTES as u64 + 1)
                    .read_until(b'\n', &mut line)
                    .await;
                match read {
                    Ok(0) | Err(_) => return None,
                    Ok(count) if count > MAX_REPLY_BYTES => return Some(error("budget_exceeded")),
                    // A truncated final line: the runtime died mid-frame.
                    Ok(_) if !line.ends_with(b"\n") => return None,
                    Ok(count) => traffic += count,
                }
                let Ok(frame) = serde_json::from_slice::<Value>(&line) else {
                    return Some(error("runtime_protocol"));
                };
                if let Some(done) = frame.get("done").and_then(Value::as_bool) {
                    return Some(finish(done, frame));
                }
                let (Some(id), Some(op)) = (
                    frame.get("id").and_then(Value::as_u64),
                    frame.get("op").and_then(Value::as_str),
                ) else {
                    return Some(error("runtime_protocol"));
                };
                calls += 1;
                let reply = if calls > MAX_CALLS {
                    json!({"id": id, "error": {"code": "call_limit"}})
                } else {
                    let args = frame.get("args").cloned().unwrap_or_else(|| json!({}));
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    match host.call(op, args, remaining).await {
                        Ok(value) => json!({"id": id, "ok": value}),
                        Err(code) => json!({"id": id, "error": {"code": code}}),
                    }
                };
                let mut bytes = serde_json::to_vec(&reply).unwrap_or_default();
                if bytes.len() > MAX_REPLY_BYTES {
                    bytes = serde_json::to_vec(
                        &json!({"id": id, "error": {"code": "reply_too_large"}}),
                    )
                    .unwrap_or_default();
                }
                bytes.push(b'\n');
                traffic += bytes.len();
                if traffic > MAX_BROKER_BYTES {
                    return Some(error("budget_exceeded"));
                }
                if stdin.write_all(&bytes).await.is_err() {
                    return None;
                }
            }
        };
        let outcome = tokio::time::timeout_at(deadline, broker).await;
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

fn finish(done: bool, mut frame: Value) -> Value {
    let logs = frame
        .get("logs")
        .and_then(Value::as_str)
        .filter(|logs| !logs.is_empty())
        .map(|logs| logs.chars().take(MAX_LOG_BYTES).collect::<String>());
    let mut value = if done {
        let result = frame.get_mut("value").map_or(Value::Null, Value::take);
        let size = serde_json::to_vec(&result).map_or(usize::MAX, |bytes| bytes.len());
        if size > MAX_RESULT_BYTES {
            json!({"schema":1,"state":"error","error":{"code":"result_too_large","bytes":size,
                "limit":MAX_RESULT_BYTES}})
        } else {
            json!({"schema":1,"state":"ok","value":result})
        }
    } else {
        // Only a bounded code and message cross back; a thrown object is
        // program-controlled and must not bypass the result budget.
        let failure = frame.get("error");
        let code = failure
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str)
            .filter(|code| {
                !code.is_empty()
                    && code.len() <= 64
                    && code
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            })
            .unwrap_or("program_error");
        let mut error = json!({"code": code});
        if let Some(message) = failure
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
        {
            error["message"] = json!(message.chars().take(2000).collect::<String>());
        }
        json!({"schema":1,"state":"error","error":error})
    };
    if let Some(logs) = logs {
        value["logs"] = json!(logs);
    }
    value
}

fn private_directory() -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt as _;
    let mut entropy = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut entropy);
    let name: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
    let directory = std::env::temp_dir().join(format!("cowboy-agent-{name}"));
    std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
    Ok(directory)
}

#[cfg(test)]
mod tests;
